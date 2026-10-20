//! Text layout (parley) and glyph drawing (Vello).
//!
//! Inter is bundled and always available, so projects render identically on
//! every machine; installed system fonts can be used by name, falling back to
//! Inter when missing.

use xenify_anim::{SceneText, TextMeasure};
use kurbo::{Affine, Rect};
use parley::{FontContext, FontFamily, LayoutContext, StyleProperty};
use std::collections::HashMap;
use std::sync::Arc;
use vello::Glyph;
use vello::peniko::{Blob, FontData};

/// The bundled default font family.
pub const DEFAULT_FAMILY: &str = "Inter";

/// The bundled Inter font file (SIL Open Font License), e.g. for UI fallback.
pub const INTER_FONT: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");

/// Layout cache size before it is cleared (animated text sizes create many entries).
const CACHE_LIMIT: usize = 512;

/// Glyphs of one font run, positioned relative to the layer origin.
pub struct GlyphRun {
    pub font: FontData,
    pub font_size: f32,
    pub coords: Vec<i16>,
    pub glyphs: Vec<Glyph>,
    /// Each glyph's advance width (for placing it along a path).
    pub advances: Vec<f32>,
    /// Byte offset in the text of the character each glyph draws (text animators).
    pub clusters: Vec<usize>,
    /// The line the run is on.
    pub line: usize,
}

/// A laid-out piece of text in layer space: point text is placed by its
/// alignment around the origin, box text inside its box.
pub struct TextLayout {
    pub runs: Vec<GlyphRun>,
    /// The lines' box (for box text, the box and any text spilling out).
    pub bounds: Rect,
    pub lines: Vec<xenify_anim::TextLine>,
    /// Box text that hides what spills out: the box.
    pub clip: Option<Rect>,
}

type Key = (String, String, u64, xenify_doc::TextAlign, xenify_doc::TextVAlign, [u64; 3], Option<[u64; 5]>);

pub struct TextEngine {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    cache: HashMap<Key, Arc<TextLayout>>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> Self {
        let mut fonts = FontContext::new();
        fonts.collection.register_fonts(Blob::new(Arc::new(INTER_FONT)), None);
        Self { fonts, layouts: LayoutContext::new(), cache: HashMap::new() }
    }

    /// Lays out `text` (cached).
    pub fn layout(&mut self, text: &SceneText) -> Arc<TextLayout> {
        let spacing = [text.letter_spacing.to_bits(), text.word_spacing.to_bits(), text.line_height.to_bits()];
        let frame = text.frame().map(|f| {
            let r = f.rect;
            [r.x0.to_bits(), r.y0.to_bits(), r.x1.to_bits(), r.y1.to_bits(), f.wrap as u64 | (f.clip as u64) << 1]
        });
        let key = (
            text.text.clone(),
            text.font_family.clone(),
            text.font_size.to_bits(),
            text.align,
            text.valign,
            spacing,
            frame,
        );
        if let Some(l) = self.cache.get(&key) {
            return l.clone();
        }
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        let layout = Arc::new(self.build(text));
        self.cache.insert(key, layout.clone());
        layout
    }

    fn build(&mut self, text: &SceneText) -> TextLayout {
        let size = text.font_size.max(0.1) as f32;
        let family = text.font_family.trim();
        // CSS-style stack: the requested family, then the bundled default.
        let stack = if family.is_empty() || family == DEFAULT_FAMILY {
            format!("\"{DEFAULT_FAMILY}\", sans-serif")
        } else {
            format!("\"{}\", \"{DEFAULT_FAMILY}\", sans-serif", family.replace('"', ""))
        };
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, &text.text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::Source(stack.into())));
        builder.push_default(StyleProperty::FontSize(size));
        if text.letter_spacing != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(text.letter_spacing as f32));
        }
        if text.word_spacing != 0.0 {
            builder.push_default(StyleProperty::WordSpacing(text.word_spacing as f32));
        }
        if text.line_height > 0.0 {
            builder
                .push_default(StyleProperty::LineHeight(parley::LineHeight::FontSizeRelative(text.line_height as f32)));
        }
        let mut layout = builder.build(&text.text);
        let frame = text.frame();
        let wrap = frame.filter(|f| f.wrap).map(|f| f.rect.width().max(1.0) as f32);
        layout.break_all_lines(wrap);
        // Align lines of multi-line text relative to each other (and to the
        // box's width when wrapping).
        let line_align = match text.align {
            xenify_doc::TextAlign::Left => parley::Alignment::Left,
            xenify_doc::TextAlign::Center => parley::Alignment::Center,
            xenify_doc::TextAlign::Right => parley::Alignment::Right,
        };
        layout.align(line_align, parley::AlignmentOptions::default());

        let first_baseline = layout.lines().next().map_or(size * 0.8, |l| l.metrics().baseline) as f64;
        // The lines' box with the origin at the left end of the first baseline.
        let width = wrap.map_or(layout.width(), |w| w.max(layout.width())) as f64;
        let lines_box = Rect::new(0.0, -first_baseline, width, layout.height() as f64 - first_baseline);
        let offset = match frame {
            // Point text: put the requested point (e.g. the centre) on the origin.
            None => xenify_anim::align_offset(lines_box, text.align, text.valign),
            // Box text: align the lines' box inside the box.
            Some(f) => {
                let r = f.rect;
                let x = match text.align {
                    xenify_doc::TextAlign::Left => r.x0,
                    xenify_doc::TextAlign::Center => r.center().x - lines_box.width() / 2.0,
                    xenify_doc::TextAlign::Right => r.x1 - lines_box.width(),
                };
                let y = match text.valign {
                    xenify_doc::TextVAlign::Top | xenify_doc::TextVAlign::Baseline => r.y0,
                    xenify_doc::TextVAlign::Middle => r.center().y - lines_box.height() / 2.0,
                    xenify_doc::TextVAlign::Bottom => r.y1 - lines_box.height(),
                };
                kurbo::Vec2::new(x, y + first_baseline)
            }
        };
        let (dx, dy) = (offset.x as f32, offset.y as f32);
        let mut runs = Vec::new();
        let mut lines = Vec::new();
        for (line_index, line) in layout.lines().enumerate() {
            let m = line.metrics();
            let range = line.text_range();
            lines.push(xenify_anim::TextLine {
                // (Empty text still has one line, whose range runs past the end.)
                text: text.text.get(range).unwrap_or("").trim_end_matches(['\n', '\r']).to_string(),
                x0: (m.offset + dx) as f64,
                x1: (m.offset + m.advance - m.trailing_whitespace + dx) as f64,
                baseline: m.baseline as f64 - first_baseline + dy as f64,
            });
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let (glyphs, advances): (Vec<Glyph>, Vec<f32>) = run
                    .positioned_glyphs()
                    .map(|g| (Glyph { id: g.id, x: g.x + dx, y: g.y - first_baseline as f32 + dy }, g.advance))
                    .unzip();
                let r = run.run();
                let clusters = cluster_offsets(&run, &glyphs);
                runs.push(GlyphRun {
                    font: r.font().clone(),
                    font_size: r.font_size(),
                    coords: r.normalized_coords().to_vec(),
                    glyphs,
                    advances,
                    clusters,
                    line: line_index,
                });
            }
        }
        let placed = lines_box + offset;
        let (bounds, clip) = match frame {
            None => (placed, None),
            Some(f) if f.clip => (f.rect, Some(f.rect)),
            Some(f) => (f.rect.union(placed), None),
        };
        TextLayout { runs, bounds, lines, clip }
    }

    /// Draws `text` into `scene` with layer-to-target transform `transform`.
    pub fn draw(&mut self, scene: &mut vello::Scene, text: &SceneText, transform: Affine) {
        let layout = self.layout(text);
        let brush = crate::to_peniko(text.color);
        let placed = text.path.as_ref().map(|p| place_on_path(&layout, p, text.align));
        if text.is_animated() {
            self.draw_animated(scene, text, &layout, placed.as_deref(), transform);
            return;
        }
        if let Some(clip) = layout.clip {
            scene.push_layer(vello::peniko::Fill::NonZero, vello::peniko::Mix::Normal, 1.0, transform, &clip);
        }
        for (r, run) in layout.runs.iter().enumerate() {
            let draw = |scene: &mut vello::Scene, t: Affine, glyphs: &mut dyn Iterator<Item = Glyph>| {
                scene
                    .draw_glyphs(&run.font)
                    .font_size(run.font_size)
                    .transform(t)
                    .normalized_coords(&run.coords)
                    .brush(brush)
                    .draw(vello::peniko::Fill::NonZero, glyphs);
            };
            match &placed {
                None => draw(scene, transform, &mut run.glyphs.iter().copied()),
                // Along a path every glyph has its own position and angle.
                Some(placed) => {
                    for (g, at) in run.glyphs.iter().zip(&placed[r]) {
                        if let Some(at) = at {
                            draw(scene, transform * *at, &mut std::iter::once(*g));
                        }
                    }
                }
            }
        }
        if layout.clip.is_some() {
            scene.pop_layer();
        }
    }

    /// Draws each glyph on its own, as the text's animators have it now.
    fn draw_animated(
        &mut self,
        scene: &mut vello::Scene,
        text: &SceneText,
        layout: &TextLayout,
        placed: Option<&[Vec<Option<Affine>>]>,
        transform: Affine,
    ) {
        let looks = glyph_looks(text, layout);
        if let Some(clip) = layout.clip {
            scene.push_layer(vello::peniko::Fill::NonZero, vello::peniko::Mix::Normal, 1.0, transform, &clip);
        }
        let on_screen = transform.as_coeffs()[0].hypot(transform.as_coeffs()[1]);
        for (r, run) in layout.runs.iter().enumerate() {
            for (i, g) in run.glyphs.iter().enumerate() {
                let look = looks[r][i];
                if look.opacity < 0.004 {
                    continue;
                }
                let at = match placed {
                    Some(p) => match p[r][i] {
                        Some(at) => at,
                        None => continue,
                    },
                    None => Affine::IDENTITY,
                };
                let t = transform * at * look.transform;
                let copies = if look.blur * on_screen > 0.5 {
                    crate::blur_samples(look.blur, look.blur * on_screen)
                } else {
                    vec![(kurbo::Vec2::ZERO, 1.0)]
                };
                for (offset, weight) in copies {
                    let c = text.color;
                    let alpha = (c.a * look.opacity * weight as f64) as f32;
                    let brush = crate::to_peniko(xenify_doc::Color { a: 1.0, ..c }).multiply_alpha(alpha);
                    scene
                        .draw_glyphs(&run.font)
                        .font_size(run.font_size)
                        .transform(t * Affine::translate(offset))
                        .normalized_coords(&run.coords)
                        .brush(brush)
                        .draw(vello::peniko::Fill::NonZero, std::iter::once(*g));
                }
            }
        }
        if layout.clip.is_some() {
            scene.pop_layer();
        }
    }

    /// The box around the text (along its path, if it has one), in layer space.
    fn bounds_of(&mut self, text: &SceneText) -> Rect {
        let layout = self.layout(text);
        let Some(path) = &text.path else { return layout.bounds };
        let placed = place_on_path(&layout, path, text.align);
        let (top, bottom) = (layout.bounds.y0, layout.bounds.y1);
        let mut out: Option<Rect> = None;
        for (run, placed) in layout.runs.iter().zip(&placed) {
            for ((g, adv), at) in run.glyphs.iter().zip(&run.advances).zip(placed) {
                let Some(at) = at else { continue };
                let cell = Rect::new(g.x as f64, top, (g.x + adv) as f64, bottom);
                let b = at.transform_rect_bbox(cell);
                out = Some(out.map_or(b, |o| o.union(b)));
            }
        }
        out.unwrap_or(Rect::ZERO)
    }

    /// The text as vector outlines in layer space (exactly what [`Self::draw`]
    /// fills), for exports that can't embed fonts.
    pub fn outline(&mut self, text: &SceneText) -> kurbo::BezPath {
        let mut out = kurbo::BezPath::new();
        for (path, opacity) in self.glyph_outlines(text) {
            if opacity >= 0.004 {
                out.extend(path);
            }
        }
        out
    }

    /// Each glyph's outline in layer space with its opacity (text animators
    /// move and fade glyphs one by one), hidden ones included (opacity 0).
    pub fn glyph_outlines(&mut self, text: &SceneText) -> Vec<(kurbo::BezPath, f64)> {
        use skrifa::MetadataProvider;
        let layout = self.layout(text);
        let looks = text.is_animated().then(|| glyph_looks(text, &layout));
        let placed = text.path.as_ref().map(|p| place_on_path(&layout, p, text.align));
        let mut out = Vec::new();
        let mut pen = Pen { path: kurbo::BezPath::new(), dx: 0.0, dy: 0.0 };
        for (r, run) in layout.runs.iter().enumerate() {
            let Ok(font) = skrifa::FontRef::from_index(run.font.data.as_ref(), run.font.index) else { continue };
            let outlines = font.outline_glyphs();
            let coords: Vec<skrifa::instance::NormalizedCoord> =
                run.coords.iter().map(|c| skrifa::raw::types::F2Dot14::from_bits(*c)).collect();
            let location = skrifa::instance::LocationRef::new(&coords);
            for (i, g) in run.glyphs.iter().enumerate() {
                // Outlines can't be cut: a hidden-overflow box keeps the glyphs
                // whose middle is inside it.
                if let Some(clip) = layout.clip {
                    let mid = kurbo::Point::new(
                        (g.x + run.advances[i] / 2.0) as f64,
                        g.y as f64 - run.font_size as f64 * 0.3,
                    );
                    if !clip.contains(mid) {
                        continue;
                    }
                }
                let Some(glyph) = outlines.get(skrifa::GlyphId::new(g.id)) else { continue };
                let at = match &placed {
                    Some(p) => match p[r][i] {
                        Some(at) => at,
                        None => continue,
                    },
                    None => Affine::IDENTITY,
                };
                (pen.dx, pen.dy) = (g.x as f64, g.y as f64);
                pen.path = kurbo::BezPath::new();
                let settings =
                    skrifa::outline::DrawSettings::unhinted(skrifa::instance::Size::new(run.font_size), location);
                let _ = glyph.draw(settings, &mut pen);
                let look = looks.as_ref().map_or(xenify_doc::text_anim::GlyphLook::REST, |l| l[r][i]);
                out.push((at * look.transform * std::mem::take(&mut pen.path), look.opacity));
            }
        }
        out
    }

    /// Names of all font families available (bundled + system), sorted.
    pub fn families(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self.fonts.collection.family_names().map(str::to_string).collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }
}

impl TextMeasure for TextEngine {
    fn text_bounds(&mut self, text: &SceneText) -> Rect {
        self.bounds_of(text)
    }

    fn text_lines(&mut self, text: &SceneText) -> Vec<xenify_anim::TextLine> {
        self.layout(text).lines.clone()
    }

    fn glyph_outlines(&mut self, text: &SceneText) -> Option<Vec<(kurbo::BezPath, f64)>> {
        Some(TextEngine::glyph_outlines(self, text))
    }
}

/// The byte offset of the character behind each of `glyphs` (a line's piece
/// of `run`): parley hides where the piece starts in the run, so it's found
/// by matching glyph ids.
fn cluster_offsets(run: &parley::GlyphRun<'_, ()>, glyphs: &[Glyph]) -> Vec<usize> {
    let all: Vec<(u32, usize)> = run
        .run()
        .visual_clusters()
        .flat_map(|c| {
            let start = c.text_range().start;
            c.glyphs().map(move |g| (g.id, start))
        })
        .collect();
    let ids: Vec<u32> = glyphs.iter().map(|g| g.id).collect();
    let skip = (0..=all.len().saturating_sub(ids.len()))
        .find(|&k| all[k..].iter().zip(&ids).all(|(a, b)| a.0 == *b))
        .unwrap_or(0);
    (0..ids.len()).map(|i| all.get(skip + i).map_or(0, |a| a.1)).collect()
}

/// How every glyph of `layout` looks under `text`'s animators, by run.
fn glyph_looks(text: &SceneText, layout: &TextLayout) -> Vec<Vec<xenify_doc::text_anim::GlyphLook>> {
    use xenify_doc::text_anim::{UnitCounts, glyph_look, place_at, places};
    let places = places(&text.text);
    let letters = places.iter().filter(|p| p.1.is_some()).count();
    let words = places.iter().filter_map(|p| p.2).max().map_or(0, |w| w + 1);
    let counts = UnitCounts { letters, words, lines: layout.lines.len().max(1) };
    let middle = (letters.max(1) - 1) as f64 / 2.0;
    layout
        .runs
        .iter()
        .map(|run| {
            run.glyphs
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    let place = place_at(&places, run.clusters[i], run.line);
                    let adv = run.advances.get(i).copied().unwrap_or(0.0) as f64;
                    let centre = kurbo::Point::new(g.x as f64 + adv / 2.0, g.y as f64 - run.font_size as f64 * 0.35);
                    let spread = place.letter.map_or(0.0, |l| l as f64 - middle);
                    glyph_look(&text.animators, place, counts, centre, spread, text.time)
                })
                .collect()
        })
        .collect()
}

/// Points along a path by distance, from a flattened copy of it.
struct ArcTable {
    points: Vec<kurbo::Point>,
    /// Distance from the start to each point.
    dist: Vec<f64>,
    closed: bool,
}

impl ArcTable {
    fn new(path: &kurbo::BezPath) -> Option<Self> {
        let (mut points, mut closed) = (Vec::new(), false);
        kurbo::flatten(path.iter(), 0.1, |el| match el {
            kurbo::PathEl::MoveTo(p) if points.is_empty() => points.push(p),
            kurbo::PathEl::LineTo(p) => points.push(p),
            kurbo::PathEl::ClosePath => {
                closed = true;
                if let Some(&first) = points.first() {
                    points.push(first);
                }
            }
            _ => {}
        });
        if points.len() < 2 {
            return None;
        }
        let mut dist = vec![0.0];
        for w in points.windows(2) {
            dist.push(dist.last().unwrap() + (w[1] - w[0]).hypot());
        }
        (*dist.last().unwrap() > 1e-9).then_some(Self { points, dist, closed })
    }

    fn length(&self) -> f64 {
        *self.dist.last().unwrap()
    }

    /// The point and direction (radians) `s` along the path; closed paths
    /// wrap around, open ones end.
    fn at(&self, s: f64) -> Option<(kurbo::Point, f64)> {
        let len = self.length();
        let s = if self.closed { s.rem_euclid(len) } else { s };
        if !(0.0..=len).contains(&s) {
            return None;
        }
        let i = self.dist.partition_point(|d| *d <= s).clamp(1, self.points.len() - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let span = (self.dist[i] - self.dist[i - 1]).max(1e-12);
        let t = (s - self.dist[i - 1]) / span;
        Some((a.lerp(b, t), (b - a).atan2()))
    }
}

/// Where each glyph goes when the text follows `path`: per run, per glyph,
/// the transform from its straight-line position (or `None` when it falls
/// off the end of an open path). Glyphs are centred on the path at their
/// middle and turned to its direction; their height above the baseline
/// becomes their distance from the path.
fn place_on_path(
    layout: &TextLayout,
    path: &xenify_anim::TextOnPath,
    align: xenify_doc::TextAlign,
) -> Vec<Vec<Option<Affine>>> {
    let Some(table) = ArcTable::new(&path.path) else {
        return layout.runs.iter().map(|r| vec![None; r.glyphs.len()]).collect();
    };
    let len = table.length();
    let anchor = match align {
        xenify_doc::TextAlign::Left => path.offset,
        xenify_doc::TextAlign::Center => len / 2.0 + path.offset,
        xenify_doc::TextAlign::Right => len - path.offset,
    };
    layout
        .runs
        .iter()
        .map(|run| {
            run.glyphs
                .iter()
                .zip(&run.advances)
                .map(|(g, adv)| {
                    let mid = g.x as f64 + *adv as f64 / 2.0;
                    let (p, angle) = table.at(anchor + mid)?;
                    Some(Affine::translate(p.to_vec2()) * Affine::rotate(angle) * Affine::translate((-mid, 0.0)))
                })
                .collect()
        })
        .collect()
}

/// Collects glyph outlines (font space is y-up) into a y-down path at an offset.
struct Pen {
    path: kurbo::BezPath,
    dx: f64,
    dy: f64,
}

impl Pen {
    fn p(&self, x: f32, y: f32) -> kurbo::Point {
        kurbo::Point::new(self.dx + x as f64, self.dy - y as f64)
    }
}

impl skrifa::outline::OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.path.move_to(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.path.line_to(p);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.p(cx0, cy0), self.p(x, y));
        self.path.quad_to(c, p);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (a, b, p) = (self.p(cx0, cy0), self.p(cx1, cy1), self.p(x, y));
        self.path.curve_to(a, b, p);
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}
