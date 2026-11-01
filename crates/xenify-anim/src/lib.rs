//! Evaluates a [`Composition`] at a point in time into a [`Scene`]: a flat list
//! of layers with resolved world transforms, opacities and geometry, ready for
//! a renderer. Pure and deterministic, so the viewport, the exporter and the
//! AI's preview frames all agree exactly.

pub mod deck;
pub mod deform;
pub mod diagram;
pub mod expr;
pub mod liquid;
pub mod media;
pub mod morph;
pub mod particles;
pub mod shatter;
pub mod stroke_style;
pub mod transition;

use kurbo::{Affine, BezPath, Rect, Shape};
use std::collections::HashMap;
use std::collections::HashSet;
use xenify_doc::{
    Color, Composition, Geometry, Homography, Id, Layer, LayerKind, LineCap, LineJoin, MatteMode, ShapeItem,
};

mod solid;
mod solid_bsp;
pub use solid::{solid_bounds, solid_items};

/// Tolerance used when flattening ellipses/rounded rects into Béziers.
const PATH_TOLERANCE: f64 = 0.05;

/// Parent chains deeper than this are treated as cycles and cut off.
const MAX_PARENT_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    /// Paint order, bottom-most first.
    pub layers: Vec<SceneLayer>,
    /// Composition → what the viewer sees: the active camera's view
    /// (identity without a camera). Layers stay in composition space, so
    /// editing isn't affected; renderers and exporters apply this at the root.
    pub camera: Affine,
    /// The moment it shows (seconds, composition time).
    pub time: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneLayer {
    pub id: Id,
    /// Layer space → composition space.
    pub transform: Affine,
    /// Group opacity, `0.0..=1.0`.
    pub opacity: f64,
    pub content: SceneContent,
    /// Track matte masking this layer (its own, or its group's).
    pub matte: Option<Box<SceneMatte>>,
    /// Perspective still to apply to text content, in layer space (shape
    /// paths already have it baked in).
    pub distort: Option<xenify_doc::Homography>,
    /// Shadows, glows and blur to draw (colour adjustments are already
    /// applied to the content).
    pub effects: Vec<xenify_doc::Fx>,
    pub blend: xenify_doc::BlendMode,
    /// The layer's masks, in order (applied after everything else).
    pub masks: Vec<SceneMask>,
    /// Which of the layer's copies this is (0 = the layer itself; see
    /// [`xenify_doc::Modifier`]). Copies share the layer's id.
    pub copy: u32,
}

/// A layer mask at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneMask {
    /// The outline in scene space (the layer's transform applied).
    pub path: BezPath,
    pub mode: xenify_doc::MaskMode,
    pub inverted: bool,
    /// Edge softness in composition pixels.
    pub feather: f64,
    pub opacity: f64,
}

/// `layer`'s enabled masks at `time`, placed by `transform` (layer → scene)
/// and, for perspective layers, `distort` (in layer space).
fn scene_masks(layer: &Layer, transform: Affine, distort: Option<xenify_doc::Homography>, time: f64) -> Vec<SceneMask> {
    let scale = transform.determinant().abs().sqrt();
    layer
        .meta
        .masks
        .iter()
        .filter(|m| m.enabled)
        .map(|m| {
            let local = m.path.value_at(time);
            let local = distort.map_or(local.clone(), |h| h.map_path(&local));
            SceneMask {
                path: transform * local,
                mode: m.mode,
                inverted: m.inverted,
                feather: m.feather.value_at(time).max(0.0) * scale,
                opacity: m.opacity.value_at(time).clamp(0.0, 1.0),
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneMatte {
    pub mode: MatteMode,
    /// The matte layer's content; `None` when it's outside its in/out range
    /// (masks everything, or nothing when inverted).
    pub source: Option<SceneLayer>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SceneContent {
    Shapes(Vec<DrawItem>),
    Text(SceneText),
    /// A nested composition, clipped to `0,0 → width,height` in layer space.
    /// Its layers' transforms are already in *this* scene's space.
    Precomp {
        width: f64,
        height: f64,
        layers: Vec<SceneLayer>,
    },
    /// A raster image covering `0,0 → width,height` in layer space.
    Image(SceneImage),
    /// A group drawn as one picture (it has effects, a blend mode or less
    /// than full opacity). Its layers are in scene space; the group's own
    /// transform is the identity.
    Group(Vec<SceneLayer>),
}

/// An embedded image, as drawn: shares the project's bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneImage {
    pub asset: Id,
    /// Content hash of `data` (render caches key on it).
    pub hash: u64,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// The encoded file (PNG, JPEG, …).
    pub data: std::sync::Arc<[u8]>,
    /// Adjust Colours effects to apply to the pixels, in order:
    /// `[brightness, contrast, saturation, hue]`.
    pub adjust: Vec<[f64; 4]>,
    /// A frame of a video instead of `data`: the file and the moment (seconds).
    pub video: Option<(media::MediaRef, f64)>,
    /// Which frame of an animated image (GIF, WebP, APNG) in `data`.
    pub frame: Option<u32>,
}

impl SceneImage {
    pub fn of(a: &xenify_doc::ImageAsset) -> Self {
        Self {
            asset: a.id,
            hash: a.hash,
            mime: a.mime.clone(),
            width: a.width,
            height: a.height,
            data: a.data.clone(),
            adjust: Vec::new(),
            video: None,
            frame: None,
        }
    }

    pub fn rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f64, self.height as f64)
    }
}

/// One filled and/or stroked path, in layer space.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawItem {
    pub path: BezPath,
    pub fill_rule: xenify_doc::FillRule,
    /// Overrides the solid `fill` colour when set.
    pub gradient: Option<xenify_doc::Gradient>,
    pub fill: Option<Color>,
    pub stroke: Option<SceneStroke>,
    /// Overrides the gradient and colour when set: a repeating image.
    pub pattern: Option<ScenePattern>,
}

/// A pattern fill: `image` repeated, placed by `transform` (image pixels →
/// the item's coordinates).
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePattern {
    pub image: SceneImage,
    pub transform: Affine,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneStroke {
    pub color: Color,
    pub width: f64,
    pub join: LineJoin,
    pub cap: LineCap,
    /// For pen styles and rounded caps: the stroke as filled shapes (drawn in
    /// `color` × each mark's alpha) instead of a native stroke.
    pub marks: Option<Vec<stroke_style::StrokeMark>>,
    /// Dash and gap lengths (empty: solid) and where the pattern starts.
    pub dashes: Vec<f64>,
    pub dash_offset: f64,
    /// Paints the line instead of `color`.
    pub gradient: Option<xenify_doc::Gradient>,
}

impl SceneStroke {
    /// A plain solid line.
    pub fn solid(color: Color, width: f64, join: LineJoin, cap: LineCap) -> Self {
        Self { color, width, join, cap, marks: None, dashes: Vec::new(), dash_offset: 0.0, gradient: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneText {
    pub text: String,
    pub font_family: String,
    pub font_size: f64,
    pub color: Color,
    pub align: xenify_doc::TextAlign,
    pub valign: xenify_doc::TextVAlign,
    /// Extra pixels between letters / words; line distance as a multiple of
    /// the size (0: the font's own).
    pub letter_spacing: f64,
    pub word_spacing: f64,
    pub line_height: f64,
    /// Set along this outline (layer space) instead of in lines.
    pub path: Option<TextOnPath>,
    /// Box text: wrapped and aligned inside this box (ignored on a path).
    pub frame: Option<xenify_doc::TextFrame>,
    /// Letters, words or lines animating in, and the layer's time for them.
    pub animators: Vec<xenify_doc::TextAnimator>,
    pub time: f64,
}

impl SceneText {
    /// Plain text: no extra spacing, not on a path.
    pub fn new(text: impl Into<String>, font_family: impl Into<String>, font_size: f64, color: Color) -> Self {
        Self {
            text: text.into(),
            font_family: font_family.into(),
            font_size,
            color,
            align: Default::default(),
            valign: Default::default(),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            line_height: 0.0,
            path: None,
            frame: None,
            animators: Vec::new(),
            time: 0.0,
        }
    }

    /// Whether any animator is on (the glyphs don't all sit at rest).
    pub fn is_animated(&self) -> bool {
        self.animators.iter().any(|a| a.enabled)
    }

    /// The text box, unless the text follows a path.
    pub fn frame(&self) -> Option<xenify_doc::TextFrame> {
        self.frame.filter(|_| self.path.is_none())
    }
}

/// The outline text follows, in the text layer's own space.
#[derive(Clone, Debug, PartialEq)]
pub struct TextOnPath {
    pub path: BezPath,
    /// Pixels along the outline before the text (see [`xenify_doc::TextPath`]).
    pub offset: f64,
}

/// Measures text; implemented by the renderer (which owns the fonts), so this
/// crate stays font-free.
pub trait TextMeasure {
    /// Bounds of `text` in layer space, honouring its alignment (e.g. centred
    /// text is centred on the layer origin). Box text: its box (and any text
    /// spilling out of it).
    fn text_bounds(&mut self, text: &SceneText) -> Rect;

    /// Each glyph's outline (layer space) with its opacity, as text
    /// animators have it (hidden ones with opacity 0); `None` when this
    /// measure has no fonts.
    fn glyph_outlines(&mut self, _text: &SceneText) -> Option<Vec<(BezPath, f64)>> {
        None
    }

    /// The lines as laid out (after wrapping), in layer space.
    fn text_lines(&mut self, text: &SceneText) -> Vec<TextLine> {
        let b = self.text_bounds(text);
        let lines: Vec<&str> = text.text.split('\n').collect();
        let line_h = b.height() / lines.len().max(1) as f64;
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| TextLine {
                text: l.to_string(),
                x0: b.x0,
                x1: b.x1,
                // About 80% down each line (Inter's ascent : descent ratio).
                baseline: b.y0 + line_h * (i as f64 + 0.8),
            })
            .collect()
    }
}

/// One laid-out line of text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub text: String,
    /// Left and right ends of the line's ink advance.
    pub x0: f64,
    pub x1: f64,
    pub baseline: f64,
}

/// How far to move text laid out with its origin at the left end of the first
/// baseline so that `align`/`valign` put the right point on the origin.
pub fn align_offset(bounds: Rect, align: xenify_doc::TextAlign, valign: xenify_doc::TextVAlign) -> kurbo::Vec2 {
    use xenify_doc::{TextAlign, TextVAlign};
    let x = match align {
        TextAlign::Left => 0.0,
        TextAlign::Center => -bounds.center().x,
        TextAlign::Right => -bounds.x1,
    };
    let y = match valign {
        TextVAlign::Baseline => 0.0,
        TextVAlign::Top => -bounds.y0,
        TextVAlign::Middle => -bounds.center().y,
        TextVAlign::Bottom => -bounds.y1,
    };
    kurbo::Vec2::new(x, y)
}

/// Rough text metrics for when no font engine is available (tests, headless tools).
pub struct ApproxText;

impl TextMeasure for ApproxText {
    fn text_bounds(&mut self, text: &SceneText) -> Rect {
        let s = text.font_size;
        let lines = text.text.split('\n').collect::<Vec<_>>();
        let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
        let r = Rect::new(0.0, -0.8 * s, 0.55 * s * longest, (lines.len() as f64 - 0.8) * 1.2 * s + 0.25 * s);
        match text.frame() {
            Some(f) => f.rect,
            None => r + align_offset(r, text.align, text.valign),
        }
    }
}

impl Scene {
    /// A drawn layer (also inside a group drawn as one), or a layer used
    /// only as a matte, by id.
    pub fn layer(&self, id: Id) -> Option<&SceneLayer> {
        fn find(layers: &[SceneLayer], id: Id) -> Option<&SceneLayer> {
            layers.iter().find_map(|l| {
                if l.id == id {
                    return Some(l);
                }
                if let Some(src) = l.matte.as_ref().and_then(|m| m.source.as_ref())
                    && src.id == id
                {
                    return Some(src);
                }
                match &l.content {
                    SceneContent::Group(inner) => find(inner, id),
                    _ => None,
                }
            })
        }
        find(&self.layers, id)
    }
}

impl SceneLayer {
    /// Bounding box of the layer's content in layer space.
    pub fn bounds(&self, measure: &mut dyn TextMeasure) -> Option<Rect> {
        match &self.content {
            SceneContent::Text(t) => {
                let b = measure.text_bounds(t);
                Some(match self.distort {
                    Some(h) => xenify_doc::Quad::of_rect(b)
                        .map(|p| h * p)
                        .points()
                        .iter()
                        .fold(Rect::from_points(h * b.origin(), h * b.origin()), |r, p| r.union_pt(*p)),
                    None => b,
                })
            }
            // A symbol's box is what it draws (within its frame), not its whole page.
            SceneContent::Precomp { width, height, layers } if self.distort.is_none() => {
                let frame = Rect::new(0.0, 0.0, *width, *height);
                if self.transform.determinant().abs() < 1e-12 {
                    return Some(frame);
                }
                // Its layers are in scene space: bring their boxes into this layer's.
                let to_local = self.transform.inverse();
                let content = layers
                    .iter()
                    .filter_map(|l| {
                        Some(to_local.transform_rect_bbox(l.transform.transform_rect_bbox(l.bounds(measure)?)))
                    })
                    .reduce(|a, b| a.union(b))
                    .map(|c| c.intersect(frame))
                    .filter(|c| c.width() > 0.0 && c.height() > 0.0);
                Some(content.unwrap_or(frame))
            }
            SceneContent::Shapes(_) | SceneContent::Precomp { .. } | SceneContent::Image(_) => self.local_bounds(),
            // Members are in scene space (the group's transform is the identity).
            SceneContent::Group(layers) => layers
                .iter()
                .filter_map(|l| Some(l.transform.transform_rect_bbox(l.bounds(measure)?)))
                .reduce(|a, b| a.union(b)),
        }
    }

    /// Bounding box of the layer's shape geometry in layer space (`None` for text;
    /// use [`Self::bounds`] for that).
    pub fn local_bounds(&self) -> Option<Rect> {
        let items = match &self.content {
            SceneContent::Shapes(items) => items,
            SceneContent::Precomp { .. } | SceneContent::Image(_) => {
                let r = match &self.content {
                    SceneContent::Image(img) => img.rect(),
                    SceneContent::Precomp { width, height, .. } => Rect::new(0.0, 0.0, *width, *height),
                    _ => unreachable!(),
                };
                let h = self.distort.unwrap_or_default();
                let pts = xenify_doc::Quad::of_rect(r).map(|p| h * p).points();
                return Some(pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p)));
            }
            SceneContent::Text(_) => return None,
            SceneContent::Group(layers) => {
                return layers
                    .iter()
                    .filter_map(|l| Some(l.transform.transform_rect_bbox(l.local_bounds()?)))
                    .reduce(|a, b| a.union(b));
            }
        };
        items
            .iter()
            .map(|i| {
                let b = i.path.bounding_box();
                match &i.stroke {
                    Some(s) => match &s.marks {
                        // Styled strokes (glow, spray) can reach beyond half the width.
                        Some(m) => m.iter().fold(b, |r, mark| r.union(mark.path.bounding_box())),
                        None => b.inflate(s.width / 2.0, s.width / 2.0),
                    },
                    None => b,
                }
            })
            .reduce(|a, b| a.union(b))
    }
}

/// The top-most layer whose geometry contains `point` (composition space).
/// `tolerance` widens strokes, in composition pixels.
pub fn hit_test(scene: &Scene, point: kurbo::Point, tolerance: f64, measure: &mut dyn TextMeasure) -> Option<Id> {
    hit_in(&scene.layers, point, tolerance, measure)
}

/// Like [`hit_test`], passing over the layers `skip` says to (the Bones
/// tool: what's under a joint being dropped, other than the part itself).
pub fn hit_test_skipping(
    scene: &Scene,
    point: kurbo::Point,
    tolerance: f64,
    measure: &mut dyn TextMeasure,
    skip: &dyn Fn(Id) -> bool,
) -> Option<Id> {
    fn find(
        layers: &[SceneLayer],
        point: kurbo::Point,
        tolerance: f64,
        measure: &mut dyn TextMeasure,
        skip: &dyn Fn(Id) -> bool,
    ) -> Option<Id> {
        for l in layers.iter().rev() {
            if !hits(l, point, tolerance, measure) {
                continue;
            }
            if let SceneContent::Group(inner) = &l.content
                && let Some(id) = find(inner, point, tolerance, measure, skip)
            {
                return Some(id);
            }
            if !skip(l.id) && !matches!(l.content, SceneContent::Group(_)) {
                return Some(l.id);
            }
        }
        None
    }
    find(&scene.layers, point, tolerance, measure, skip)
}

/// The top-most layer hit among `layers`; inside a group drawn as one, the
/// member that was hit (callers pick the group when they want it).
fn hit_in(layers: &[SceneLayer], point: kurbo::Point, tolerance: f64, measure: &mut dyn TextMeasure) -> Option<Id> {
    let l = layers.iter().rev().find(|l| hits(l, point, tolerance, measure))?;
    match &l.content {
        SceneContent::Group(inner) => hit_in(inner, point, tolerance, measure).or(Some(l.id)),
        _ => Some(l.id),
    }
}

/// Whether `point` hits the visible part of `layer` (respecting its matte
/// and masks).
fn hits(layer: &SceneLayer, point: kurbo::Point, tolerance: f64, measure: &mut dyn TextMeasure) -> bool {
    masks_show(&layer.masks, point)
        && layer_contains(layer, point, tolerance, measure)
        && layer.matte.as_ref().is_none_or(|m| {
            let inside = m.source.as_ref().is_some_and(|s| layer_contains(s, point, 0.0, measure));
            inside != m.mode.inverted()
        })
}

/// Whether `masks` let the layer show at `point` (scene space; feathered
/// edges count as their outline).
pub fn masks_show(masks: &[SceneMask], point: kurbo::Point) -> bool {
    use xenify_doc::MaskMode;
    let Some(first) = masks.first() else { return true };
    let mut shown = matches!(first.mode, MaskMode::Subtract | MaskMode::Intersect);
    for m in masks.iter().filter(|m| m.opacity > 0.0) {
        let inside = (m.path.winding(point) != 0) != m.inverted;
        match m.mode {
            MaskMode::Add => shown |= inside,
            MaskMode::Subtract => shown &= !inside,
            MaskMode::Intersect => shown &= inside,
            MaskMode::Difference => shown ^= inside,
        }
    }
    shown
}

/// The box (scene space) the masks can let anything show in, when they
/// limit it (`None`: no limit).
pub fn masks_box(masks: &[SceneMask]) -> Option<Rect> {
    use xenify_doc::MaskMode;
    let mut limit: Option<Rect> = None;
    for (i, m) in masks.iter().enumerate() {
        let b = m.path.bounding_box().inflate(m.feather, m.feather);
        limit = match (m.mode, m.inverted) {
            (MaskMode::Add, false) if i == 0 => Some(b),
            (MaskMode::Add, false) => limit.map(|l| l.union(b)),
            (MaskMode::Intersect, false) => Some(limit.map_or(b, |l| l.intersect(b))),
            (MaskMode::Subtract, _) | (MaskMode::Intersect, true) => limit,
            _ => None,
        };
    }
    limit
}

/// Whether `point` (composition space) is on the layer's ink: inside a fill,
/// on a stroke (widened by `tolerance`), or inside a text box.
pub fn layer_contains(layer: &SceneLayer, point: kurbo::Point, tolerance: f64, measure: &mut dyn TextMeasure) -> bool {
    if layer.transform.determinant().abs() < 1e-12 {
        return false;
    }
    let inv = layer.transform.inverse();
    let local = inv * point;
    // Tolerance is in comp pixels; convert to (approximately) layer units.
    let scale = layer.transform.determinant().abs().sqrt().max(1e-9);
    let tol = tolerance / scale;
    let items = match &layer.content {
        SceneContent::Text(t) => {
            let local = match layer.distort.and_then(|h| h.inverse()) {
                Some(inv) => inv * local,
                None => local,
            };
            return measure.text_bounds(t).inflate(tol, tol).contains(local);
        }
        SceneContent::Precomp { width, height, layers } => {
            let local = layer.distort.and_then(|h| h.inverse()).map_or(local, |inv| inv * local);
            return Rect::new(0.0, 0.0, *width, *height).contains(local)
                && layers.iter().any(|l| hits(l, point, tolerance, measure));
        }
        SceneContent::Image(img) => {
            let local = layer.distort.and_then(|h| h.inverse()).map_or(local, |inv| inv * local);
            return img.rect().inflate(tol, tol).contains(local);
        }
        SceneContent::Group(layers) => return layers.iter().any(|l| hits(l, point, tolerance, measure)),
        SceneContent::Shapes(items) => items,
    };
    items.iter().any(|item| {
        let winding = item.path.winding(local);
        let inside = match item.fill_rule {
            xenify_doc::FillRule::NonZero => winding != 0,
            xenify_doc::FillRule::EvenOdd => winding % 2 != 0,
        };
        if item.fill.is_some() && inside {
            return true;
        }
        let half = item.stroke.as_ref().map_or(0.0, |s| s.width / 2.0) + tol;
        (item.stroke.is_some() || item.fill.is_some() && tol > 0.0)
            && item.path.segments().any(|seg| {
                use kurbo::ParamCurveNearest;
                seg.nearest(local, 0.1).distance_sq <= half * half
            })
    })
}

/// Whether two layers' drawn content overlaps (not just their bounding
/// boxes): samples a grid over the boxes' intersection and checks both layers'
/// ink. A hollow ring doesn't "overlap" text sitting inside it.
pub fn ink_overlap(a: &SceneLayer, b: &SceneLayer, measure: &mut dyn TextMeasure) -> bool {
    let (Some(ba), Some(bb)) = (a.bounds(measure), b.bounds(measure)) else { return false };
    let i = a.transform.transform_rect_bbox(ba).intersect(b.transform.transform_rect_bbox(bb));
    if i.width() <= 0.5 || i.height() <= 0.5 {
        return false;
    }
    const N: usize = 12;
    (0..N).any(|yi| {
        (0..N).any(|xi| {
            let p = kurbo::Point::new(
                i.x0 + (xi as f64 + 0.5) / N as f64 * i.width(),
                i.y0 + (yi as f64 + 0.5) / N as f64 * i.height(),
            );
            layer_contains(a, p, 0.0, measure) && layer_contains(b, p, 0.0, measure)
        })
    })
}

/// All layers under `id` (children, grandchildren, …), in stacking order.
pub fn descendants(comp: &Composition, id: Id) -> Vec<Id> {
    let mut out = Vec::new();
    let mut frontier = vec![id];
    while let Some(p) = frontier.pop() {
        for l in comp.layers.iter().filter(|l| l.meta.parent == Some(p)) {
            if !out.contains(&l.id) && l.id != id {
                out.push(l.id);
                frontier.push(l.id);
            }
        }
    }
    out.sort_by_key(|i| comp.layer_index(*i));
    out
}

/// A layer's on-screen box at the scene's time; for groups/nulls, the union
/// of everything under them.
pub fn world_box(comp: &Composition, scene: &Scene, id: Id, measure: &mut dyn TextMeasure) -> Option<Rect> {
    let own = |id: Id, measure: &mut dyn TextMeasure| {
        let l = scene.layer(id)?;
        let b = l.transform.transform_rect_bbox(l.bounds(measure)?);
        // A cropped layer's box is what the crop leaves.
        Some(masks_box(&l.masks).map_or(b, |m| b.intersect(m)))
    };
    match comp.layer(id).map(|l| &l.kind) {
        Some(LayerKind::Group) | Some(LayerKind::Null) => {
            descendants(comp, id).into_iter().filter_map(|d| own(d, measure)).reduce(|a, b| a.union(b))
        }
        Some(LayerKind::Camera(_)) => {
            let corners = camera_frame(comp, id, scene.time)?;
            Some(corners.iter().fold(Rect::from_points(corners[0], corners[0]), |r, p| r.union_pt(*p)))
        }
        _ => own(id, measure),
    }
}

/// The outermost group containing `id` (or `id` itself): what a plain click selects.
pub fn outermost_group(comp: &Composition, id: Id) -> Id {
    let mut best = id;
    let mut cursor = comp.layer(id).and_then(|l| l.meta.parent);
    let mut guard = 0;
    while let Some(p) = cursor {
        let Some(l) = comp.layer(p) else { break };
        // A folder (a layer of the Layers panel) holds shapes; it is never picked by a click.
        if l.meta.folder {
            break;
        }
        if matches!(l.kind, LayerKind::Group) {
            best = p;
        }
        cursor = l.meta.parent;
        guard += 1;
        if guard > comp.layers.len() {
            break;
        }
    }
    best
}

/// Nested precomps deeper than this (or referencing themselves) render empty.
const MAX_PRECOMP_DEPTH: usize = 8;

/// Evaluates `comp` at `time` seconds. Precomp layers render empty; use
/// [`evaluate_project`] when the composition may contain them.
pub fn evaluate(comp: &Composition, time: f64) -> Scene {
    scene_of(comp, time, None)
}

/// Evaluates composition `comp` of `project`, including nested compositions.
pub fn evaluate_project(project: &xenify_doc::Project, comp: Id, time: f64) -> Scene {
    match project.comp(comp) {
        Ok(c) => scene_of(c, time, Some(project)),
        Err(_) => {
            Scene { width: 0, height: 0, background: Color::BLACK, layers: Vec::new(), camera: Affine::IDENTITY, time }
        }
    }
}

fn scene_of(comp: &Composition, time: f64, project: Option<&xenify_doc::Project>) -> Scene {
    let ctx = Ctx { project, depth: 0 };
    let layers = comp_layers(comp, time, Affine::IDENTITY, ctx);
    Scene {
        width: comp.settings.width,
        height: comp.settings.height,
        background: comp.settings.background,
        layers,
        camera: camera_view(comp, time),
        time,
    }
}

/// Composition → view transform of `comp`'s active camera at `time`
/// (identity when it has none).
pub fn camera_view(comp: &Composition, time: f64) -> Affine {
    xenify_doc::active_camera(&comp.layers, time).and_then(|cam| view_of(comp, cam.id, time)).unwrap_or_default()
}

/// Composition → view transform of camera layer `camera` (active or not).
pub fn view_of(comp: &Composition, camera: Id, time: f64) -> Option<Affine> {
    let LayerKind::Camera(c) = &comp.layer(camera)?.kind else { return None };
    let baked = expr::bake(comp, time);
    let world = WorldTransforms::new(&baked, time).get(camera);
    Some(c.view(world, comp.settings.width as f64, comp.settings.height as f64, time))
}

/// The corners (clockwise from top left, composition space) of what camera
/// layer `camera` sees at `time`.
pub fn camera_frame(comp: &Composition, camera: Id, time: f64) -> Option<[kurbo::Point; 4]> {
    let inv = view_of(comp, camera, time)?.inverse();
    let (w, h) = (comp.settings.width as f64, comp.settings.height as f64);
    Some([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| inv * kurbo::Point::new(x, y)))
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    project: Option<&'a xenify_doc::Project>,
    depth: usize,
}

/// The drawn layers of `comp` at `time`, with `outer` applied to every transform.
fn comp_layers(comp: &Composition, time: f64, outer: Affine, ctx: Ctx) -> Vec<SceneLayer> {
    // Expression-driven properties become plain values for this frame.
    let baked = expr::bake(comp, time);
    let comp = &*baked;
    let world = WorldTransforms::new(comp, time);
    let mut layers = flat_layers(comp, &world, time, outer, ctx);
    diagram::resolve_connectors(comp, &mut layers, outer, time);
    group_containers(comp, layers, &world, outer, time)
}

/// Where the connectors of composition `comp` run at `time` (composition
/// space), in stacking order.
pub fn connector_routes(project: &xenify_doc::Project, comp: Id, time: f64) -> Vec<diagram::Routed> {
    let Ok(c) = project.comp(comp) else { return Vec::new() };
    if !c.layers.iter().any(|l| l.meta.connector.is_some()) {
        return Vec::new();
    }
    let baked = expr::bake(c, time);
    let c = &*baked;
    let world = WorldTransforms::new(c, time);
    let layers = flat_layers(c, &world, time, Affine::IDENTITY, Ctx { project: Some(project), depth: 0 });
    diagram::route_all(c, &layers, Affine::IDENTITY)
}

/// Every layer drawn on its own (with copies and blends), before connectors
/// are routed and groups gathered.
fn flat_layers(comp: &Composition, world: &WorldTransforms, time: f64, outer: Affine, ctx: Ctx) -> Vec<SceneLayer> {
    // Layers used as a matte are only drawn through the layers they mask.
    let matte_sources: HashSet<Id> = comp.layers.iter().filter_map(|l| l.meta.matte.map(|m| m.layer)).collect();
    comp.layers
        .iter()
        .filter(|l| l.is_active_at(time) && !matte_sources.contains(&l.id))
        .flat_map(|layer| {
            let Some(mut drawn) = scene_layer(layer, world, outer, time, ctx) else { return Vec::new() };
            drawn.matte = effective_matte(comp, layer, world, outer, time, ctx).map(Box::new);
            let to_comp = outer * world.get(layer.id);
            let plans = layer_plan(layer, world, time, content_box(&drawn, to_comp));
            let mut out = with_copies(drawn.clone(), plans, to_comp);
            for m in layer.meta.modifiers.iter().filter(|m| m.enabled) {
                let xenify_doc::ModifierKind::Blend { target, count } = &m.kind else { continue };
                let Some(target) = world.by_id.get(target).filter(|b| b.id != layer.id) else { continue };
                let Some(b) = scene_layer(target, world, outer, time, ctx) else { continue };
                // Towards the target as it's drawn (its first copy, e.g. placed on a path).
                let mut b_plans = layer_plan(target, world, time, None);
                b_plans.truncate(1);
                let Some(b) = with_copies(b, b_plans, outer * world.get(target.id)).into_iter().next() else {
                    continue;
                };
                let steps = count.value_at(time).round().clamp(0.0, xenify_doc::MAX_COPIES as f64) as usize;
                let first = out.len() as u32;
                out.extend(blend_steps(&drawn, &b, steps, first));
            }
            out
        })
        .collect()
}

/// Gathers the members of groups that have effects, a blend mode or less
/// than full opacity into one [`SceneContent::Group`] each (inner groups
/// first), so those apply to the group as a whole.
fn group_containers(
    comp: &Composition,
    mut layers: Vec<SceneLayer>,
    world: &WorldTransforms,
    outer: Affine,
    time: f64,
) -> Vec<SceneLayer> {
    let depth = |id: Id| {
        let mut d = 0;
        let mut p = comp.layer(id).and_then(|l| l.meta.parent);
        while let Some(pp) = p {
            d += 1;
            if d > MAX_PARENT_DEPTH {
                break;
            }
            p = comp.layer(pp).and_then(|l| l.meta.parent);
        }
        d
    };
    let mut groups: Vec<&Layer> = comp
        .layers
        .iter()
        .filter(|g| matches!(g.kind, LayerKind::Group) && g.is_active_at(time))
        .filter(|g| {
            !g.meta.effects.iter().all(|e| !e.enabled)
                || !g.meta.blend.is_normal()
                || g.opacity.value_at(time) < 1.0
                || g.meta.masks.iter().any(|m| m.enabled)
                || g.meta.modifiers.iter().any(|m| m.enabled)
        })
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(depth(g.id)));
    for g in groups {
        let members: HashSet<Id> = descendants(comp, g.id).into_iter().collect();
        let Some(first) = layers.iter().position(|l| members.contains(&l.id)) else { continue };
        let (inside, outside): (Vec<SceneLayer>, Vec<SceneLayer>) =
            layers.into_iter().partition(|l| members.contains(&l.id));
        let mut container = SceneLayer {
            id: g.id,
            transform: Affine::IDENTITY,
            opacity: g.opacity.value_at(time).clamp(0.0, 1.0),
            content: SceneContent::Group(inside),
            matte: None,
            distort: None,
            effects: Vec::new(),
            blend: g.meta.blend,
            masks: scene_masks(g, outer * world.get(g.id), None, time),
            copy: 0,
        };
        for e in g.meta.effects.iter().filter(|e| e.enabled) {
            match e.kind.at(time) {
                xenify_doc::Fx::AdjustColors { brightness, contrast, saturation, hue } => {
                    let f = |c: Color| xenify_doc::adjust_color(c, brightness, contrast, saturation, hue);
                    recolor(&mut container.content, &f, [brightness, contrast, saturation, hue]);
                }
                k => container.effects.push(k),
            }
        }
        layers = outside;
        let at = first.min(layers.len());
        let to_comp = outer * world.get(g.id);
        let plans = layer_plan(g, world, time, content_box(&container, to_comp));
        layers.splice(at..at, with_copies(container, plans, to_comp));
    }
    layers
}

/// What `drawn` covers, in the space `to_comp` maps from (its layer's).
fn content_box(drawn: &SceneLayer, to_comp: Affine) -> Option<Rect> {
    if to_comp.determinant().abs() < 1e-12 {
        return None;
    }
    let b = drawn.bounds(&mut ApproxText)?;
    Some(to_comp.inverse().transform_rect_bbox(drawn.transform.transform_rect_bbox(b)))
}

/// The copies `layer`'s modifiers make at `time` (see [`xenify_doc::plan`]),
/// with arrays along a path placed on their path and shatters cut from
/// `content` (the layer's box, its own space).
fn layer_plan(layer: &Layer, world: &WorldTransforms, time: f64, content: Option<Rect>) -> Vec<xenify_doc::CopyPlan> {
    xenify_doc::plan_with(&layer.meta.modifiers, time, &|_, kind| match kind {
        xenify_doc::ModifierKind::PathArray { path, count, start, end, align } => {
            let n = count.value_at(time).round().clamp(1.0, xenify_doc::MAX_COPIES as f64) as usize;
            path_copies(layer, *path, n, (start.value_at(time), end.value_at(time)), *align, world, time)
                .into_iter()
                .map(|(m, o)| (m, o, None))
                .collect()
        }
        xenify_doc::ModifierKind::Shatter { count, origin, force, amount, rotation, gravity, spin, progress, seed } => {
            let Some(b) = content else { return Vec::new() };
            let v = |p: &xenify_doc::Property<f64>| p.value_at(time);
            shatter::copies(
                b,
                &shatter::Flight {
                    pieces: v(count).round().clamp(1.0, 400.0) as usize,
                    origin: origin.value_at(time).to_point(),
                    force: v(force),
                    push: v(amount),
                    direction: v(rotation),
                    gravity: v(gravity),
                    spin: v(spin),
                    progress: v(progress),
                    seed: v(seed).round().max(0.0) as u64,
                },
            )
        }
        _ => Vec::new(),
    })
}

/// What `layer`'s modifiers make at `time`: each copy's layer-space
/// transform, opacity and outline changes (the layer itself first).
pub fn modifier_plan(comp: &Composition, layer: Id, time: f64) -> Vec<xenify_doc::CopyPlan> {
    let Some(l) = comp.layer(layer) else { return Vec::new() };
    modifiers_plan(comp, l, &l.meta.modifiers, time)
}

/// [`modifier_plan`] for some of `layer`'s modifiers only.
pub fn modifiers_plan(
    comp: &Composition,
    layer: &Layer,
    modifiers: &[xenify_doc::Modifier],
    time: f64,
) -> Vec<xenify_doc::CopyPlan> {
    let world = WorldTransforms::new(comp, time);
    let mut l = layer.clone();
    l.meta.modifiers = modifiers.to_vec();
    layer_plan(&l, &world, time, None)
}

/// The drawings a layer's Blend modifiers add at `time`, in composition
/// space (for applying them).
pub fn blend_drawings(comp: &Composition, layer: Id, time: f64) -> Vec<SceneLayer> {
    let scene = evaluate(comp, time);
    let own = comp.layer(layer).map_or(0, |l| modifier_plan(comp, l.id, time).len());
    drawn(&scene.layers, layer).into_iter().skip(own.max(1)).collect()
}

/// Everything drawn as layer `id` (it and its copies), also inside groups
/// drawn as one.
pub fn drawn(layers: &[SceneLayer], id: Id) -> Vec<SceneLayer> {
    let mut out = Vec::new();
    for l in layers {
        match &l.content {
            SceneContent::Group(inner) if l.id != id => out.extend(drawn(inner, id)),
            _ if l.id == id => out.push(l.clone()),
            _ => {}
        }
    }
    out
}

/// Copies of `layer` spread along the first outline of shape layer `path`
/// (composition space), between the fractions `range` of its length; each
/// copy's transform origin sits on the line, turned with it when `align`.
fn path_copies(
    layer: &Layer,
    path: Id,
    n: usize,
    range: (f64, f64),
    align: bool,
    world: &WorldTransforms,
    time: f64,
) -> Vec<(Affine, f64)> {
    use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv};
    let Some(p) = world.by_id.get(&path).filter(|p| p.id != layer.id) else { return Vec::new() };
    let LayerKind::Shape { shapes } = &p.kind else { return Vec::new() };
    let Some(shape) = shapes.first() else { return Vec::new() };
    let line = world.get(path) * geometry_path(&shape.geometry, time);
    let segs: Vec<kurbo::PathSeg> = line.segments().collect();
    let lens: Vec<f64> = segs.iter().map(|s| s.arclen(0.05)).collect();
    let total: f64 = lens.iter().sum();
    let w = world.get(layer.id);
    if segs.is_empty() || total <= 0.0 || w.determinant().abs() < 1e-12 {
        return Vec::new();
    }
    let closed = (segs[0].start() - segs[segs.len() - 1].end()).hypot() < 1e-6;
    let (a, b) = (range.0.clamp(0.0, 1.0), range.1.clamp(0.0, 1.0));
    // Round a whole closed loop, the last copy would sit on the first.
    let span = if closed && (b - a).abs() >= 1.0 - 1e-9 { n as f64 } else { (n.max(2) - 1) as f64 };
    let anchor = w * layer.transform.anchor.value_at(time).to_point();
    let inverse = w.inverse();
    (0..n)
        .map(|i| {
            let mut d = total * (a + (b - a) * i as f64 / span);
            let mut place = (segs[segs.len() - 1].end(), 0.0);
            for (s, l) in segs.iter().zip(&lens) {
                if d <= *l {
                    let t = if *l > 0.0 { s.inv_arclen(d, 0.05) } else { 0.0 };
                    let tan = match s {
                        kurbo::PathSeg::Line(l) => l.p1 - l.p0,
                        kurbo::PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
                        kurbo::PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
                    };
                    place = (s.eval(t), tan.y.atan2(tan.x));
                    break;
                }
                d -= l;
            }
            let turn = if align { place.1 } else { 0.0 };
            let x = Affine::translate(place.0.to_vec2()) * Affine::rotate(turn) * Affine::translate(-anchor.to_vec2());
            (inverse * x * w, 1.0)
        })
        .collect()
}

/// `layer` as its modifiers draw it: every copy. `world` is the layer →
/// scene transform the modifiers' layer-space steps are relative to.
fn with_copies(layer: SceneLayer, plans: Vec<xenify_doc::CopyPlan>, world: Affine) -> Vec<SceneLayer> {
    let plain = plans.len() == 1
        && plans[0].deforms.is_empty()
        && plans[0].transform == Affine::IDENTITY
        && plans[0].clip.is_none();
    if plain || plans.is_empty() || world.determinant().abs() < 1e-12 {
        return vec![layer];
    }
    let inverse = world.inverse();
    plans
        .into_iter()
        .enumerate()
        .map(|(i, plan)| {
            let mut c = layer.clone();
            if !plan.deforms.is_empty() {
                deform_layer(&mut c, &plan, inverse);
            }
            // A shard: only its part of the layer shows (moving with it).
            if let Some(clip) = &plan.clip {
                c.masks.push(SceneMask {
                    path: world * clip.clone(),
                    mode: xenify_doc::MaskMode::Intersect,
                    inverted: false,
                    feather: 0.0,
                    opacity: 1.0,
                });
            }
            if plan.transform != Affine::IDENTITY {
                move_layer(&mut c, world * plan.transform * inverse, false);
            }
            c.copy = i as u32;
            c.opacity *= plan.opacity;
            c
        })
        .collect()
}

/// A drawn item as a (still) shape: its outline and paint as they are.
pub fn shape_from_item(item: &DrawItem, name: &str) -> ShapeItem {
    let paint = |g: &xenify_doc::Gradient| xenify_doc::GradientPaint {
        kind: g.kind,
        start: g.start.into(),
        end: g.end.into(),
        stops: g.stops.clone().into(),
    };
    ShapeItem {
        name: name.to_string(),
        geometry: Geometry::Path { path: item.path.clone().into() },
        fill_rule: item.fill_rule,
        fill: item.fill.map(|c| xenify_doc::Fill {
            color: c.into(),
            gradient: item.gradient.as_ref().map(paint),
            pattern: None,
        }),
        stroke: item.stroke.as_ref().map(|s| xenify_doc::Stroke {
            color: s.color.into(),
            width: s.width.into(),
            join: s.join,
            cap: s.cap,
            dashes: s.dashes.clone(),
            dash_offset: s.dash_offset.into(),
            gradient: s.gradient.as_ref().map(paint),
            ..Default::default()
        }),
        trim: None,
    }
}

/// `layer`'s shapes in scene space (its transform applied), then moved by `to`.
pub fn placed_items(layer: &SceneLayer, to: Affine) -> Vec<DrawItem> {
    let SceneContent::Shapes(items) = &layer.content else { return Vec::new() };
    let t = to * layer.transform;
    let k = t.determinant().abs().sqrt();
    let place = |g: &mut xenify_doc::Gradient| {
        g.start = (t * g.start.to_point()).to_vec2();
        g.end = (t * g.end.to_point()).to_vec2();
    };
    items
        .iter()
        .map(|item| {
            let mut item = item.clone();
            item.path = t * &item.path;
            if let Some(g) = &mut item.gradient {
                place(g);
            }
            if let Some(s) = &mut item.stroke {
                s.width *= k;
                s.marks = None;
                if let Some(g) = &mut s.gradient {
                    place(g);
                }
            }
            item
        })
        .collect()
}

/// The `steps` drawings between shape layers `a` and `b` (a Blend), as copies
/// of `a` numbered from `first`, in scene space.
fn blend_steps(a: &SceneLayer, b: &SceneLayer, steps: usize, first: u32) -> Vec<SceneLayer> {
    let (ia, ib) = (placed_items(a, Affine::IDENTITY), placed_items(b, Affine::IDENTITY));
    if ia.is_empty() || ib.is_empty() {
        return Vec::new();
    }
    (1..=steps)
        .map(|k| {
            let t = k as f64 / (steps + 1) as f64;
            let items = ia.iter().zip(&ib).map(|(x, y)| morph::lerp_items(x, y, t)).collect();
            SceneLayer {
                id: a.id,
                transform: Affine::IDENTITY,
                opacity: a.opacity + (b.opacity - a.opacity) * t,
                content: SceneContent::Shapes(items),
                matte: a.matte.clone(),
                distort: None,
                effects: a.effects.clone(),
                blend: a.blend,
                masks: Vec::new(),
                copy: first + k as u32 - 1,
            }
        })
        .collect()
}

/// Changes the outlines a drawn layer holds by `plan`'s deforms, which work
/// in the modifier's layer space (`to_space`: scene → that space). Text and
/// images keep their shape.
fn deform_layer(l: &mut SceneLayer, plan: &xenify_doc::CopyPlan, to_space: Affine) {
    let local = to_space * l.transform;
    match &mut l.content {
        SceneContent::Shapes(items) => {
            for item in items {
                item.path = deform::deform_copy(plan, &item.path, local);
                if let Some(marks) = item.stroke.as_mut().and_then(|s| s.marks.as_mut()) {
                    for m in marks {
                        m.path = deform::deform_copy(plan, &m.path, local);
                    }
                }
            }
        }
        SceneContent::Group(inner) | SceneContent::Precomp { layers: inner, .. } => {
            for c in inner {
                deform_layer(c, plan, to_space);
            }
        }
        SceneContent::Text(_) | SceneContent::Image(_) => {}
    }
}

/// Moves a drawn layer and everything it holds by `m` (scene space). Its
/// track matte, being another layer, stays put unless `with_matte` (for the
/// contents of a nested composition).
fn move_layer(l: &mut SceneLayer, m: Affine, with_matte: bool) {
    let scale = m.determinant().abs().sqrt();
    for mask in &mut l.masks {
        mask.path = m * &mask.path;
        mask.feather *= scale;
    }
    if with_matte && let Some(src) = l.matte.as_mut().and_then(|mt| mt.source.as_mut()) {
        move_layer(src, m, true);
    }
    match &mut l.content {
        // Members are in scene space; the group's own transform stays the identity.
        SceneContent::Group(inner) => {
            for c in inner {
                move_layer(c, m, with_matte);
            }
            return;
        }
        SceneContent::Precomp { layers, .. } => {
            for c in layers {
                move_layer(c, m, true);
            }
        }
        _ => {}
    }
    l.transform = m * l.transform;
}

/// A layer's own drawable content (no matte); `None` for nulls and groups.
fn scene_layer(layer: &Layer, world: &WorldTransforms, outer: Affine, time: f64, ctx: Ctx) -> Option<SceneLayer> {
    let transform = outer * world.get(layer.id);
    let content = match &layer.kind {
        LayerKind::Shape { shapes } => {
            let seed = layer.id.0.wrapping_mul(1000);
            let mut items: Vec<DrawItem> =
                shapes.iter().enumerate().map(|(i, s)| draw_item(s, time, seed + i as u64, ctx.project)).collect();
            // Connectors get theirs once routed.
            if layer.meta.connector.is_none() {
                diagram::add_markers(&mut items, shapes, seed);
            }
            SceneContent::Shapes(items)
        }
        LayerKind::Text(t) => SceneContent::Text(SceneText {
            text: t.text.clone(),
            font_family: t.font_family.clone(),
            font_size: t.font_size.value_at(time).max(0.0),
            color: t.color.value_at(time),
            align: t.align,
            valign: t.valign,
            letter_spacing: t.letter_spacing,
            word_spacing: t.word_spacing,
            line_height: t.line_height,
            path: t.path.and_then(|tp| text_path(layer, &tp, world, time)),
            frame: t.frame,
            animators: t.animators.clone(),
            time: time - layer.meta.in_point,
        }),
        LayerKind::Precomp { comp, start, looped } => {
            let child = ctx.project.and_then(|p| p.comp(*comp).ok());
            let (width, height) = child.map_or((0.0, 0.0), |c| (c.settings.width as f64, c.settings.height as f64));
            let layers = match child {
                Some(c) if ctx.depth < MAX_PRECOMP_DEPTH => {
                    let mut local = time - start;
                    if *looped && local >= 0.0 && c.settings.duration > 0.0 {
                        local = local.rem_euclid(c.settings.duration);
                    }
                    let inside = local >= 0.0 && local < c.settings.duration;
                    let ctx = Ctx { depth: ctx.depth + 1, ..ctx };
                    if inside { comp_layers(c, local, transform * camera_view(c, local), ctx) } else { Vec::new() }
                }
                _ => Vec::new(),
            };
            SceneContent::Precomp { width, height, layers }
        }
        LayerKind::Solid(solid) => SceneContent::Shapes(solid::solid_items(solid, time)),
        LayerKind::Image { asset, playback } => {
            let a = ctx.project.and_then(|p| p.asset(*asset))?;
            let mut img = SceneImage::of(a);
            if let Some(n) = a.frame_at(playback.time(time - layer.meta.in_point), playback.looped) {
                // One picture per frame (caches key on it).
                img.frame = Some(n as u32);
                img.hash ^= (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            }
            SceneContent::Image(img)
        }
        LayerKind::Null | LayerKind::Group | LayerKind::Camera(_) => return None,
        // Drawn in composition space (particles fly free of the emitter once born).
        LayerKind::Particles(e) => SceneContent::Shapes(particles::particle_items(world.comp, layer, e, time)),
        LayerKind::Liquid(w) => SceneContent::Shapes(liquid::liquid_items(w, time)),
        LayerKind::Video(v) => {
            let a = ctx.project.and_then(|p| p.media(v.asset))?;
            let t =
                xenify_doc::media::source_time(v.source_in, v.speed, v.looped, a.duration, time - layer.meta.in_point)?;
            // One picture per frame of the file (caches key on it).
            let n = media::frame_index(a.fps, t);
            SceneContent::Image(SceneImage {
                asset: a.id,
                hash: a.hash ^ (n.wrapping_add(1)).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                mime: "image/x-video-frame".into(),
                width: a.width,
                height: a.height,
                data: std::sync::Arc::from(Vec::new()),
                adjust: Vec::new(),
                video: Some((media::MediaRef::of(a), if a.fps > 0.0 { n as f64 / a.fps } else { t })),
                frame: None,
            })
        }
        LayerKind::Audio(_) => return None,
    };
    let mut out = SceneLayer {
        id: layer.id,
        transform,
        opacity: layer.opacity.value_at(time).clamp(0.0, 1.0),
        content,
        matte: None,
        distort: None,
        effects: Vec::new(),
        blend: layer.meta.blend,
        masks: Vec::new(),
        copy: 0,
    };
    for e in layer.meta.effects.iter().filter(|e| e.enabled) {
        match e.kind.at(time) {
            xenify_doc::Fx::AdjustColors { brightness, contrast, saturation, hue } => {
                let f = |c: Color| xenify_doc::adjust_color(c, brightness, contrast, saturation, hue);
                recolor(&mut out.content, &f, [brightness, contrast, saturation, hue]);
            }
            k => out.effects.push(k),
        }
    }
    let distort = chain_distort(layer, world, outer, time);
    if let Some(g) = distort {
        apply_distort(&mut out, g);
    }
    out.masks = scene_masks(layer, transform, distort, time);
    if matches!(layer.kind, LayerKind::Particles(_)) {
        out.transform = outer;
    }
    if let Some(label) = &layer.meta.label
        && layer.meta.connector.is_none()
        && matches!(out.content, SceneContent::Shapes(_))
    {
        out = diagram::label_node(out, label);
    }
    Some(out)
}

/// Applies `f` to every colour in `content` (images are left as they are).
/// (Images get `adjust` to apply to their pixels when drawn.)
fn recolor(content: &mut SceneContent, f: &dyn Fn(Color) -> Color, adjust: [f64; 4]) {
    match content {
        SceneContent::Shapes(items) => {
            for item in items {
                item.fill = item.fill.map(f);
                if let Some(s) = &mut item.stroke {
                    s.color = f(s.color);
                }
                if let Some(g) = &mut item.gradient {
                    for stop in &mut g.stops {
                        stop.color = f(stop.color);
                    }
                }
            }
        }
        SceneContent::Text(t) => t.color = f(t.color),
        SceneContent::Precomp { layers, .. } | SceneContent::Group(layers) => {
            for l in layers {
                recolor(&mut l.content, f, adjust);
            }
        }
        SceneContent::Image(img) => img.adjust.push(adjust),
    }
}

/// The outline text layer `text` follows, mapped into its own space.
fn text_path(text: &Layer, tp: &xenify_doc::TextPath, world: &WorldTransforms, time: f64) -> Option<TextOnPath> {
    let src = world.by_id.get(&tp.layer)?;
    let LayerKind::Shape { shapes } = &src.kind else { return None };
    let shape = shapes.first()?;
    let path = geometry_path(&shape.geometry, time);
    let to_text = world.get(text.id).inverse() * world.get(src.id);
    let mut path = to_text * path;
    if tp.flip {
        path = reversed(&path);
    }
    Some(TextOnPath { path, offset: tp.offset })
}

/// `path` drawn the other way round.
fn reversed(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let segs: Vec<kurbo::PathSeg> = path.segments().collect();
    for (i, seg) in segs.iter().rev().enumerate() {
        let seg = seg.reverse();
        if i == 0 {
            out.move_to(kurbo::ParamCurve::start(&seg));
        }
        match seg {
            kurbo::PathSeg::Line(l) => out.line_to(l.p1),
            kurbo::PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
            kurbo::PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
        }
    }
    out
}

/// The perspective acting on `layer` in composition (scene) space: its own
/// corner pin and those of its parents/groups, innermost first.
fn chain_distort(layer: &Layer, world: &WorldTransforms, outer: Affine, time: f64) -> Option<Homography> {
    let mut total: Option<Homography> = None;
    let mut current = Some(layer);
    for _ in 0..MAX_PARENT_DEPTH {
        let Some(l) = current else { break };
        if let Some(d) = &l.transform.distort {
            let h = d.homography_at(time);
            if !h.is_identity() {
                let w = Homography::from(outer * world.get(l.id));
                if let Some(w_inv) = w.inverse() {
                    let g = w * h * w_inv;
                    total = Some(total.map_or(g, |t| g * t));
                }
            }
        }
        current = l.meta.parent.and_then(|p| world.by_id.get(&p).copied());
    }
    total
}

/// Applies scene-space perspective `g` to a layer's content.
fn apply_distort(layer: &mut SceneLayer, g: Homography) {
    let w = Homography::from(layer.transform);
    let Some(w_inv) = w.inverse() else { return };
    // The same map, expressed in the layer's own space.
    let m = w_inv * g * w;
    match &mut layer.content {
        SceneContent::Shapes(items) => {
            for item in items {
                item.path = m.map_path(&item.path);
                if let Some(marks) = item.stroke.as_mut().and_then(|s| s.marks.as_mut()) {
                    for mark in marks {
                        mark.path = m.map_path(&mark.path);
                    }
                }
                if let Some(gr) = &mut item.gradient {
                    gr.start = (m * gr.start.to_point()).to_vec2();
                    gr.end = (m * gr.end.to_point()).to_vec2();
                }
            }
        }
        // Drawn with the perspective applied at render time.
        SceneContent::Text(_) | SceneContent::Image(_) => layer.distort = Some(layer.distort.map_or(m, |d| m * d)),
        SceneContent::Group(_) => {}
        SceneContent::Precomp { layers, .. } => {
            // Kept so the frame clip can follow the perspective.
            layer.distort = Some(layer.distort.map_or(m, |d| m * d));
            for child in layers {
                apply_distort(child, g);
                if let Some(m) = &mut child.matte
                    && let Some(src) = &mut m.source
                {
                    apply_distort(src, g);
                }
            }
        }
    }
}

/// The matte for `layer`: its own, else the nearest group/parent's.
fn effective_matte(
    comp: &Composition,
    layer: &Layer,
    world: &WorldTransforms,
    outer: Affine,
    time: f64,
    ctx: Ctx,
) -> Option<SceneMatte> {
    let mut current = Some(layer);
    for _ in 0..MAX_PARENT_DEPTH {
        let l = current?;
        if let Some(m) = l.meta.matte {
            let source = comp.layer(m.layer)?;
            let active = time >= source.meta.in_point && source.meta.out_point.is_none_or(|out| time < out);
            let source = if active { scene_layer(source, world, outer, time, ctx) } else { None };
            return Some(SceneMatte { mode: m.mode, source });
        }
        current = l.meta.parent.and_then(|p| comp.layer(p));
    }
    None
}

/// Layer → composition transform of one layer, including its parents.
pub fn world_transform(comp: &Composition, layer: Id, time: f64) -> Affine {
    WorldTransforms::new(comp, time).get(layer)
}

/// Resolves parented transforms, memoised per evaluation.
struct WorldTransforms<'a> {
    comp: &'a Composition,
    by_id: HashMap<Id, &'a Layer>,
    time: f64,
    cache: std::cell::RefCell<HashMap<Id, Affine>>,
}

impl<'a> WorldTransforms<'a> {
    fn new(comp: &'a Composition, time: f64) -> Self {
        Self { comp, by_id: comp.layers.iter().map(|l| (l.id, l)).collect(), time, cache: Default::default() }
    }

    fn get(&self, id: Id) -> Affine {
        self.resolve(id, 0)
    }

    fn resolve(&self, id: Id, depth: usize) -> Affine {
        if let Some(a) = self.cache.borrow().get(&id) {
            return *a;
        }
        let Some(layer) = self.by_id.get(&id) else {
            return Affine::IDENTITY;
        };
        let local = layer.transform.affine_at(self.time);
        let world = match layer.meta.parent {
            Some(parent) if parent != id && depth < MAX_PARENT_DEPTH && self.by_id.contains_key(&parent) => {
                self.resolve(parent, depth + 1) * local
            }
            _ => local,
        };
        self.cache.borrow_mut().insert(id, world);
        world
    }
}

fn abs(v: kurbo::Vec2) -> kurbo::Vec2 {
    kurbo::Vec2::new(v.x.abs(), v.y.abs())
}

/// Accuracy for arc-length computations when trimming paths.
const ARCLEN_ACCURACY: f64 = 1e-3;

/// Keeps only the part of each subpath between `start` and `end` (fractions of
/// its length, 0..1), shifted by `offset` (0..1 wraps once). Closed subpaths
/// wrap around their start seamlessly, like After Effects' "Trim Paths".
pub fn trim_path(path: &BezPath, start: f64, end: f64, offset: f64) -> BezPath {
    use kurbo::{ParamCurve, ParamCurveArclen, PathSeg};
    let (s, e) = {
        let (a, b) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
        if a <= b { (a, b) } else { (b, a) }
    };
    let span = e - s;
    let mut out = BezPath::new();
    if span <= 1e-9 {
        return out;
    }
    // Split into subpaths of segments.
    let mut subpaths: Vec<(Vec<PathSeg>, bool)> = Vec::new();
    // Rebuild subpaths from the elements (`segments()` loses the boundaries).
    let mut current: Vec<PathSeg> = Vec::new();
    let mut closed = false;
    let mut last = kurbo::Point::ZERO;
    let mut first = kurbo::Point::ZERO;
    for el in path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                if !current.is_empty() {
                    subpaths.push((std::mem::take(&mut current), closed));
                }
                closed = false;
                first = p;
                last = p;
            }
            kurbo::PathEl::LineTo(p) => {
                current.push(PathSeg::Line(kurbo::Line::new(last, p)));
                last = p;
            }
            kurbo::PathEl::QuadTo(a, p) => {
                current.push(PathSeg::Quad(kurbo::QuadBez::new(last, a, p)));
                last = p;
            }
            kurbo::PathEl::CurveTo(a, b, p) => {
                current.push(PathSeg::Cubic(kurbo::CubicBez::new(last, a, b, p)));
                last = p;
            }
            kurbo::PathEl::ClosePath => {
                if (last - first).hypot() > 1e-9 {
                    current.push(PathSeg::Line(kurbo::Line::new(last, first)));
                }
                closed = true;
                last = first;
            }
        }
    }
    if !current.is_empty() {
        subpaths.push((current, closed));
    }

    for (segs, closed) in subpaths {
        let lens: Vec<f64> = segs.iter().map(|s| s.arclen(ARCLEN_ACCURACY)).collect();
        let total: f64 = lens.iter().sum();
        if total <= 1e-9 {
            continue;
        }
        if span >= 1.0 - 1e-9 {
            // Nothing trimmed.
            out.move_to(segs[0].start());
            for seg in &segs {
                out.push(seg.as_path_el());
            }
            if closed {
                out.close_path();
            }
            continue;
        }
        let a = (s + offset).rem_euclid(1.0);
        let b = a + span;
        // Appends the part of the outline between lengths `from`..`to`.
        let emit = |out: &mut BezPath, from: f64, to: f64, continue_path: bool| {
            let mut acc = 0.0;
            let mut started = continue_path;
            for (seg, len) in segs.iter().zip(&lens) {
                let (s0, s1) = (acc, acc + len);
                acc = s1;
                if s1 <= from || s0 >= to || *len <= 0.0 {
                    continue;
                }
                let t0 = if from > s0 { seg.inv_arclen(from - s0, ARCLEN_ACCURACY) } else { 0.0 };
                let t1 = if to < s1 { seg.inv_arclen(to - s0, ARCLEN_ACCURACY) } else { 1.0 };
                let piece = seg.subsegment(t0..t1);
                if !started {
                    out.move_to(piece.start());
                    started = true;
                }
                out.push(piece.as_path_el());
            }
        };
        if b <= 1.0 {
            emit(&mut out, a * total, b * total, false);
        } else if closed {
            // Wrap across the start point as one continuous piece.
            emit(&mut out, a * total, total, false);
            emit(&mut out, 0.0, (b - 1.0) * total, true);
        } else {
            emit(&mut out, a * total, total, false);
            emit(&mut out, 0.0, (b - 1.0) * total, false);
        }
    }
    out
}

/// The outline of `geometry` at `time`, in layer space.
pub fn geometry_path(geometry: &Geometry, time: f64) -> BezPath {
    match geometry {
        Geometry::Rect { position, size, corner_radius } => {
            let size = abs(size.value_at(time));
            let rect = Rect::from_center_size(position.value_at(time).to_point(), size.to_size());
            let radius = corner_radius.value_at(time).clamp(0.0, size.x.min(size.y) / 2.0);
            if radius > 0.0 {
                rect.to_rounded_rect(radius).to_path(PATH_TOLERANCE)
            } else {
                rect.to_path(PATH_TOLERANCE)
            }
        }
        Geometry::Ellipse { position, size } => {
            let size = abs(size.value_at(time));
            let rect = Rect::from_center_size(position.value_at(time).to_point(), size.to_size());
            kurbo::Ellipse::from_rect(rect).to_path(PATH_TOLERANCE)
        }
        Geometry::Path { path } => path.value_at(time),
        Geometry::Figure { .. } => xenify_doc::primitives::live_path(geometry, time).unwrap_or_default(),
    }
}

/// One shape as drawn at `time` (trimmed, with its stroke's pen marks).
pub fn draw_item(shape: &ShapeItem, time: f64, seed: u64, project: Option<&xenify_doc::Project>) -> DrawItem {
    let pattern = shape.fill.as_ref().and_then(|f| f.pattern).and_then(|p| {
        let a = project?.asset(p.asset)?;
        Some(ScenePattern { image: SceneImage::of(a), transform: p.affine() })
    });
    let mut path = geometry_path(&shape.geometry, time);
    if let Some(t) = &shape.trim {
        path = trim_path(&path, t.start.value_at(time), t.end.value_at(time), t.offset.value_at(time));
    }
    DrawItem {
        fill_rule: shape.fill_rule,
        gradient: shape.fill.as_ref().and_then(|f| f.gradient.as_ref()).map(|g| g.value_at(time)),
        fill: shape.fill.as_ref().map(|f| f.color.value_at(time)),
        pattern,
        stroke: shape.stroke.as_ref().map(|s| {
            let mut st = SceneStroke {
                dashes: s.dashes.iter().map(|d| d.max(0.0)).collect(),
                dash_offset: s.dash_offset.value_at(time),
                gradient: s.gradient.as_ref().map(|g| g.value_at(time)),
                ..SceneStroke::solid(s.color.value_at(time), s.width.value_at(time).max(0.0), s.join, s.cap)
            };
            if st.dashes.iter().sum::<f64>() <= 0.0 {
                st.dashes.clear();
            }
            st.marks = stroke_style::marks(&path, &st, &s.style, s.cap_roundness, seed);
            st
        }),
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;
    use xenify_doc::{Easing, Keyframe, LayerMeta, Project, Property, ShapeItem, Vec2};

    #[test]
    fn text_on_a_path_follows_the_shape_wherever_it_is() {
        let circle = Layer::shape(Id(1), "circle", vec![ShapeItem::ellipse(Vec2::new(200.0, 200.0), Color::WHITE)])
            .with_position(Vec2::new(500.0, 400.0));
        let mut t = xenify_doc::TextLayer::new("Round", "Inter", 30.0, Color::WHITE);
        t.path = Some(xenify_doc::TextPath { layer: Id(1), offset: 10.0, flip: true });
        let text = Layer::new(Id(2), "text", LayerKind::Text(t)).with_position(Vec2::new(100.0, 100.0));
        let scene = evaluate(&comp_with(vec![circle, text]), 0.0);
        let sl = scene.layer(Id(2)).unwrap();
        let SceneContent::Text(st) = &sl.content else { panic!() };
        let tp = st.path.as_ref().expect("resolved path");
        assert_eq!(tp.offset, 10.0);
        // In the text layer's space, mapped back to the composition: around (500, 400), radius 100.
        for el in tp.path.elements() {
            if let kurbo::PathEl::MoveTo(p) = el {
                let world = sl.transform * *p;
                assert!(((world - Point::new(500.0, 400.0)).hypot() - 100.0).abs() < 1e-6, "{world:?}");
            }
        }
    }

    #[test]
    fn groups_with_opacity_or_effects_are_drawn_as_one() {
        let mut group = Layer::new(Id(1), "group", LayerKind::Group).with_position(Vec2::new(100.0, 0.0));
        group.opacity = 0.5.into();
        let mut a = Layer::shape(Id(2), "a", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::WHITE)]);
        a.meta.parent = Some(Id(1));
        let mut b = Layer::shape(Id(3), "b", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::WHITE)])
            .with_position(Vec2::new(10.0, 0.0));
        b.meta.parent = Some(Id(1));
        let other = Layer::shape(Id(4), "other", vec![ShapeItem::rect(Vec2::new(5.0, 5.0), Color::WHITE)]);
        let scene = evaluate(&comp_with(vec![a, b, group, other]), 0.0);
        let ids: Vec<Id> = scene.layers.iter().map(|l| l.id).collect();
        assert_eq!(ids, [Id(1), Id(4)], "the group replaces its members, in their place");
        let g = &scene.layers[0];
        assert_eq!(g.opacity, 0.5);
        let SceneContent::Group(members) = &g.content else { panic!() };
        assert_eq!(members.iter().map(|l| l.id).collect::<Vec<_>>(), [Id(2), Id(3)]);
        // Members are still found and hit.
        assert!(scene.layer(Id(3)).is_some());
        assert_eq!(hit_test(&scene, Point::new(113.0, 0.0), 0.0, &mut ApproxText), Some(Id(3)));
        assert_eq!(g.bounds(&mut ApproxText), Some(kurbo::Rect::new(90.0, -10.0, 120.0, 10.0)));
        // A plain group stays flat (members drawn one by one).
        let mut plain = comp_with(vec![]);
        let mut g2 = Layer::new(Id(1), "g", LayerKind::Group);
        g2.opacity = 1.0.into();
        let mut m = Layer::shape(Id(2), "m", vec![ShapeItem::rect(Vec2::new(5.0, 5.0), Color::WHITE)]);
        m.meta.parent = Some(Id(1));
        plain.layers = vec![m, g2];
        assert_eq!(evaluate(&plain, 0.0).layers.iter().map(|l| l.id).collect::<Vec<_>>(), [Id(2)]);
    }

    #[test]
    fn array_modifiers_add_copies_of_layers_and_groups() {
        use xenify_doc::{Modifier, ModifierKind};
        let mut row = Layer::shape(Id(1), "row", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::WHITE)])
            .with_position(Vec2::new(100.0, 100.0));
        row.transform.rotation = 90.0.into();
        let mut arr = ModifierKind::array(3, Vec2::new(30.0, 0.0), Vec2::ZERO);
        if let ModifierKind::Array { end_opacity, .. } = &mut arr {
            *end_opacity = 0.0.into();
        }
        row.meta.modifiers.push(Modifier::new(arr));
        let other = Layer::shape(Id(2), "other", vec![ShapeItem::rect(Vec2::new(5.0, 5.0), Color::WHITE)]);
        let scene = evaluate(&comp_with(vec![row, other]), 0.0);
        let got: Vec<(Id, u32, f64)> = scene.layers.iter().map(|l| (l.id, l.copy, l.opacity)).collect();
        assert_eq!(got, [(Id(1), 0, 1.0), (Id(1), 1, 0.5), (Id(1), 2, 0.0), (Id(2), 0, 1.0)]);
        // Steps are in layer space, so the row turns with the layer: it runs down.
        let at = |l: &SceneLayer| l.transform * Point::ZERO;
        assert!((at(&scene.layers[2]) - Point::new(100.0, 160.0)).hypot() < 1e-9, "{:?}", at(&scene.layers[2]));
        // Copies are hit as the layer; the layer itself is what's looked up.
        assert_eq!(hit_test(&scene, Point::new(100.0, 130.0), 0.0, &mut ApproxText), Some(Id(1)));
        assert_eq!(scene.layer(Id(1)).map(|l| l.copy), Some(0));

        // A group repeats as a whole, members, masks and all.
        let mut group = Layer::new(Id(1), "group", LayerKind::Group).with_position(Vec2::new(100.0, 0.0));
        group.meta.modifiers.push(Modifier::new(ModifierKind::radial_array(4, Vec2::new(0.0, 50.0))));
        let mut a = Layer::shape(Id(2), "a", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::WHITE)]);
        a.meta.parent = Some(Id(1));
        a.meta.masks.push(xenify_doc::Mask::new("m", xenify_doc::rect_path(kurbo::Rect::new(-5.0, -5.0, 5.0, 5.0))));
        let scene = evaluate(&comp_with(vec![a, group]), 0.0);
        assert_eq!(scene.layers.len(), 4);
        let SceneContent::Group(members) = &scene.layers[2].content else { panic!() };
        assert_eq!(scene.layers[2].transform, Affine::IDENTITY);
        assert!((at(&members[0]) - Point::new(100.0, 100.0)).hypot() < 1e-9, "half a turn about (100, 50)");
        let mask_box = members[0].masks[0].path.bounding_box();
        assert!((mask_box.center() - Point::new(100.0, 100.0)).hypot() < 1e-9, "the mask moves with it");
    }

    #[test]
    fn arrays_along_a_path_blends_and_mirrors() {
        use xenify_doc::{Modifier, ModifierKind};
        // A line from (100, 100) to (400, 100), and a dot placed along it.
        let mut line = Layer::shape(
            Id(1),
            "line",
            vec![ShapeItem::path(
                kurbo::Line::new((0.0, 0.0), (300.0, 0.0)).to_path(0.1),
                None,
                Some((Color::WHITE, 2.0)),
            )],
        )
        .with_position(Vec2::new(100.0, 100.0));
        line.meta.visible = false;
        let mut dot = Layer::shape(Id(2), "dot", vec![ShapeItem::rect(Vec2::new(10.0, 4.0), Color::WHITE)])
            .with_position(Vec2::new(900.0, 900.0));
        dot.meta.modifiers.push(Modifier::new(ModifierKind::PathArray {
            path: Id(1),
            count: 4.0.into(),
            start: 0.0.into(),
            end: 1.0.into(),
            align: true,
        }));
        let scene = evaluate(&comp_with(vec![line.clone(), dot.clone()]), 0.0);
        let at: Vec<Point> = scene.layers.iter().filter(|l| l.id == Id(2)).map(|l| l.transform * Point::ZERO).collect();
        let want = [100.0, 200.0, 300.0, 400.0].map(|x| Point::new(x, 100.0));
        assert!(at.iter().zip(want).all(|(p, q)| (*p - q).hypot() < 1e-6), "on the line, ends included: {at:?}");

        // Blend: three steps between a small square and a big one further on.
        let a = Layer::shape(Id(3), "a", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::rgb(1.0, 0.0, 0.0))]);
        let b = Layer::shape(Id(4), "b", vec![ShapeItem::rect(Vec2::new(50.0, 50.0), Color::rgb(0.0, 0.0, 1.0))])
            .with_position(Vec2::new(400.0, 0.0));
        let mut a2 = a.clone();
        a2.meta.modifiers.push(Modifier::new(ModifierKind::Blend { target: Id(4), count: 3.0.into() }));
        let scene = evaluate(&comp_with(vec![a2, b]), 0.0);
        let steps: Vec<&SceneLayer> = scene.layers.iter().filter(|l| l.id == Id(3) && l.copy > 0).collect();
        assert_eq!(steps.len(), 3);
        let SceneContent::Shapes(items) = &steps[1].content else { panic!() };
        let bb = items[0].path.bounding_box();
        assert!((bb.center().x - 200.0).abs() < 1e-6 && (bb.width() - 30.0).abs() < 1e-6, "half way: {bb:?}");
        let c = items[0].fill.unwrap();
        assert!((c.r - 0.5).abs() < 1e-9 && (c.b - 0.5).abs() < 1e-9);

        // Mirror across the layer's own upright axis at x = 20 (layer space).
        let mut m = a.clone();
        m.meta.modifiers.push(Modifier::new(ModifierKind::Mirror {
            origin: Vec2::new(20.0, 0.0).into(),
            rotation: 90.0.into(),
            merge: false,
        }));
        let scene = evaluate(&comp_with(vec![m]), 0.0);
        assert_eq!(scene.layers.len(), 2);
        assert!((scene.layers[1].transform * Point::ZERO - Point::new(40.0, 0.0)).hypot() < 1e-9);
    }

    #[test]
    fn image_layers_have_their_pixel_box() {
        let mut p = Project::new();
        let asset = xenify_doc::ImageAsset::new(p.alloc_id(), "img", "image/png", 200, 100, vec![1, 2, 3]);
        let id = p.alloc_id();
        let mut layer = Layer::image(id, "Img", &asset, Vec2::new(500.0, 400.0));
        layer.transform.rotation = 90.0.into();
        p.assets.push(asset);
        let comp = p.compositions[0].id;
        p.comp_mut(comp).unwrap().layers.push(layer);

        let scene = evaluate_project(&p, comp, 0.0);
        let l = scene.layer(id).unwrap();
        assert!(matches!(&l.content, SceneContent::Image(img) if img.width == 200 && *img.data == [1, 2, 3]));
        assert_eq!(l.local_bounds(), Some(Rect::new(0.0, 0.0, 200.0, 100.0)));
        // Rotated 90° about its centre: 100 wide, 200 tall around (500, 400).
        let m = &mut ApproxText;
        assert_eq!(hit_test(&scene, Point::new(500.0, 490.0), 0.0, m), Some(id));
        assert_eq!(hit_test(&scene, Point::new(590.0, 400.0), 0.0, m), None);
        // Without the project (no asset table) nothing is drawn.
        assert!(evaluate(&p.compositions[0], 0.0).layers.is_empty());
    }

    fn comp_with(layers: Vec<Layer>) -> Composition {
        let mut p = Project::new();
        let mut c = p.compositions.remove(0);
        c.layers = layers;
        c
    }

    fn rect_layer(id: u64, pos: Vec2) -> Layer {
        Layer::shape(Id(id), "R", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]).with_position(pos)
    }

    #[test]
    fn animated_position_moves_geometry() {
        let mut l = rect_layer(1, Vec2::ZERO);
        l.transform.position = Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: Vec2::new(0.0, 0.0), easing: Easing::Linear },
            Keyframe { time: 2.0, value: Vec2::new(200.0, 100.0), easing: Easing::Linear },
        ])
        .unwrap();
        let scene = evaluate(&comp_with(vec![l]), 1.0);
        let layer = &scene.layers[0];
        let centre = layer.transform * layer.local_bounds().unwrap().center();
        assert!((centre - Point::new(100.0, 50.0)).hypot() < 1e-9);
    }

    #[test]
    fn in_out_points_and_visibility() {
        let mut a = rect_layer(1, Vec2::ZERO);
        a.meta = LayerMeta { in_point: 1.0, out_point: Some(2.0), ..LayerMeta::named("a") };
        let mut b = rect_layer(2, Vec2::ZERO);
        b.meta.visible = false;
        let c = comp_with(vec![a, b]);
        assert!(evaluate(&c, 0.5).layers.is_empty());
        assert_eq!(evaluate(&c, 1.5).layers.len(), 1);
        assert!(evaluate(&c, 2.0).layers.is_empty());
    }

    #[test]
    fn parenting_composes_and_nulls_are_not_drawn() {
        let parent = Layer::new(Id(1), "Null", LayerKind::Null).with_position(Vec2::new(100.0, 0.0));
        let mut child = rect_layer(2, Vec2::new(0.0, 50.0));
        child.meta.parent = Some(Id(1));
        let scene = evaluate(&comp_with(vec![parent, child]), 0.0);
        assert_eq!(scene.layers.len(), 1);
        let p = scene.layers[0].transform * Point::ZERO;
        assert_eq!(p, Point::new(100.0, 50.0));
    }

    #[test]
    fn parent_cycles_terminate() {
        let mut a = rect_layer(1, Vec2::new(1.0, 0.0));
        a.meta.parent = Some(Id(2));
        let mut b = rect_layer(2, Vec2::new(1.0, 0.0));
        b.meta.parent = Some(Id(1));
        let scene = evaluate(&comp_with(vec![a, b]), 0.0);
        assert_eq!(scene.layers.len(), 2);
    }

    #[test]
    fn rect_geometry_and_opacity_clamp() {
        let mut l = rect_layer(1, Vec2::ZERO);
        l.opacity = 3.0.into();
        let scene = evaluate(&comp_with(vec![l]), 0.0);
        assert_eq!(scene.layers[0].opacity, 1.0);
        let b = scene.layers[0].local_bounds().unwrap();
        assert_eq!(b, Rect::new(-5.0, -5.0, 5.0, 5.0));
    }

    #[test]
    fn hit_testing_picks_topmost() {
        let bottom = rect_layer(1, Vec2::new(50.0, 50.0));
        let mut top = rect_layer(2, Vec2::new(55.0, 50.0));
        top.transform.scale = Vec2::new(2.0, 2.0).into();
        let scene = evaluate(&comp_with(vec![bottom, top]), 0.0);
        // Inside both → the top one.
        assert_eq!(hit_test(&scene, Point::new(50.0, 50.0), 0.0, &mut ApproxText), Some(Id(2)));
        // Only inside the (scaled) top one.
        assert_eq!(hit_test(&scene, Point::new(64.0, 50.0), 0.0, &mut ApproxText), Some(Id(2)));
        assert_eq!(hit_test(&scene, Point::new(200.0, 200.0), 0.0, &mut ApproxText), None);
        // Just outside, but within tolerance.
        assert_eq!(hit_test(&scene, Point::new(66.0, 50.0), 2.0, &mut ApproxText), Some(Id(2)));
    }

    #[test]
    fn text_layers_hit_test_by_measured_bounds() {
        let text =
            Layer::new(Id(1), "T", LayerKind::Text(xenify_doc::TextLayer::new("Hello", "Inter", 100.0, Color::WHITE)))
                .with_position(Vec2::new(100.0, 200.0));
        let scene = evaluate(&comp_with(vec![text]), 0.0);
        // Just above the baseline, inside the first glyph.
        assert_eq!(hit_test(&scene, Point::new(110.0, 180.0), 0.0, &mut ApproxText), Some(Id(1)));
        assert_eq!(hit_test(&scene, Point::new(90.0, 180.0), 0.0, &mut ApproxText), None);
        assert!(scene.layers[0].bounds(&mut ApproxText).is_some());
    }

    #[test]
    fn ink_overlap_ignores_empty_space_in_hollow_shapes() {
        let mut ring = ShapeItem::ellipse(Vec2::new(400.0, 400.0), Color::WHITE);
        ring.fill = None;
        ring.stroke = Some(xenify_doc::Stroke {
            cap_roundness: 0.0,
            style: Default::default(),
            color: Color::WHITE.into(),
            width: 10.0.into(),
            join: LineJoin::Round,
            cap: LineCap::Round,
            ..Default::default()
        });
        let ring = Layer::shape(Id(1), "ring", vec![ring]).with_position(Vec2::new(500.0, 500.0));
        let inside = rect_layer(2, Vec2::new(500.0, 500.0));
        let on_edge = rect_layer(3, Vec2::new(700.0, 500.0));
        let scene = evaluate(&comp_with(vec![ring, inside, on_edge]), 0.0);
        let [r, a, b] = [&scene.layers[0], &scene.layers[1], &scene.layers[2]];
        assert!(!ink_overlap(r, a, &mut ApproxText), "text-sized box in the middle of a ring doesn't touch it");
        assert!(ink_overlap(r, b, &mut ApproxText), "box on the ring's line does");
    }

    fn length(p: &BezPath) -> f64 {
        use kurbo::ParamCurveArclen;
        p.segments().map(|s| s.arclen(1e-6)).sum()
    }

    #[test]
    fn precomps_offset_time_and_survive_cycles() {
        let mut p = Project::new();
        let main = p.compositions[0].id;
        let inner = Composition { id: Id(50), settings: Default::default(), layers: vec![] };
        let mut dot = Layer::shape(Id(51), "dot", vec![ShapeItem::ellipse(Vec2::new(10.0, 10.0), Color::WHITE)]);
        dot.opacity = Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: 0.0, easing: Easing::Linear },
            Keyframe { time: 1.0, value: 1.0, easing: Easing::Linear },
        ])
        .unwrap();
        p.compositions.push(Composition { layers: vec![dot], ..inner.clone() });
        let mut pre = Layer::precomp(Id(52), "pre", &inner, Vec2::new(960.0, 540.0));
        pre.kind = LayerKind::Precomp { comp: Id(50), start: 1.0, looped: false };
        // The inner comp also contains itself: must not recurse forever.
        let own = Layer::precomp(Id(53), "self", &inner, Vec2::ZERO);
        p.compositions[1].layers.push(own);
        p.comp_mut(main).unwrap().layers.push(pre);
        let scene = evaluate_project(&p, main, 1.5);
        let SceneContent::Precomp { layers, width, .. } = &scene.layers[0].content else { panic!() };
        assert_eq!(*width, 1920.0);
        assert!((layers[0].opacity - 0.5).abs() < 1e-9, "local time 0.5 s");
        // Before its start the precomp is empty.
        let early = evaluate_project(&p, main, 0.5);
        assert!(matches!(&early.layers[0].content, SceneContent::Precomp { layers, .. } if layers.is_empty()));
    }

    #[test]
    fn expressions_drive_properties_and_reach_children() {
        let mut c = Project::new().compositions.remove(0);
        let mut parent = Layer::new(Id(1), "p", LayerKind::Null).with_position(Vec2::new(100.0, 100.0));
        parent.meta.expressions.push(xenify_doc::Expression {
            path: xenify_doc::PropPath::Position,
            source: "value + [time * 10, 0]".into(),
        });
        let mut child = Layer::shape(Id(2), "c", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        child.meta.parent = Some(Id(1));
        // A broken expression is ignored (the keyframed value is used).
        child
            .meta
            .expressions
            .push(xenify_doc::Expression { path: xenify_doc::PropPath::Rotation, source: "oops(".into() });
        c.layers = vec![parent, child];
        let scene = evaluate(&c, 2.0);
        let origin = scene.layers[0].transform * Point::ORIGIN;
        assert_eq!(origin, Point::new(120.0, 100.0));
    }

    #[test]
    fn mattes_hide_their_source_and_apply_through_groups() {
        use xenify_doc::{Matte, MatteMode};
        let mut c = Project::new().compositions.remove(0);
        let circle = Layer::shape(Id(1), "matte", vec![ShapeItem::ellipse(Vec2::new(100.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(100.0, 100.0));
        let mut group = Layer::new(Id(2), "group", LayerKind::Group);
        group.meta.matte = Some(Matte { layer: Id(1), mode: MatteMode::Alpha });
        let mut big = Layer::shape(Id(3), "big", vec![ShapeItem::rect(Vec2::new(400.0, 400.0), Color::WHITE)]);
        big.meta.parent = Some(Id(2));
        c.layers = vec![circle, group, big];
        let scene = evaluate(&c, 0.0);
        assert_eq!(scene.layers.iter().map(|l| l.id).collect::<Vec<_>>(), vec![Id(3)], "matte source not drawn");
        let m = scene.layers[0].matte.as_ref().expect("inherited from the group");
        assert_eq!(m.source.as_ref().unwrap().id, Id(1));
        // Hit-testing respects the matte.
        assert_eq!(hit_test(&scene, Point::new(100.0, 100.0), 0.0, &mut ApproxText), Some(Id(3)));
        assert_eq!(hit_test(&scene, Point::new(180.0, 20.0), 0.0, &mut ApproxText), None);
    }

    #[test]
    fn trim_paths_by_length_with_wrap_around() {
        let sq = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1); // length 400
        let half = trim_path(&sq, 0.0, 0.5, 0.0);
        assert!((length(&half) - 200.0).abs() < 0.01, "{}", length(&half));
        assert!(trim_path(&sq, 0.3, 0.3, 0.0).elements().is_empty());
        // Full range keeps the whole closed shape.
        let full = trim_path(&sq, 0.0, 1.0, 0.25);
        assert!((length(&full) - 400.0).abs() < 0.01);
        assert!(matches!(full.elements().last(), Some(kurbo::PathEl::ClosePath)));
        // Offset past the end on a closed path: one continuous piece across the seam.
        let wrap = trim_path(&sq, 0.0, 0.5, 0.75);
        assert!((length(&wrap) - 200.0).abs() < 0.01);
        let moves = wrap.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
        assert_eq!(moves, 1);
        // Start > end is treated as swapped.
        assert!((length(&trim_path(&sq, 0.75, 0.25, 0.0)) - 200.0).abs() < 0.01);
    }

    #[test]
    fn stroke_inflates_bounds() {
        let mut shape = ShapeItem::ellipse(Vec2::new(20.0, 20.0), Color::WHITE);
        shape.stroke = Some(xenify_doc::Stroke {
            cap_roundness: 0.0,
            style: Default::default(),
            color: Color::BLACK.into(),
            width: 4.0.into(),
            join: LineJoin::Round,
            cap: LineCap::Round,
            ..Default::default()
        });
        let scene = evaluate(&comp_with(vec![Layer::shape(Id(1), "E", vec![shape])]), 0.0);
        let b = scene.layers[0].local_bounds().unwrap();
        assert!((b.width() - 24.0).abs() < 1e-6);
    }
}

#[cfg(test)]
mod mask_tests {
    use super::*;
    use xenify_doc::{Mask, MaskMode, Project, ShapeItem, Vec2};

    #[test]
    fn masks_limit_clicks_and_boxes() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(50), "r", vec![ShapeItem::rect(Vec2::new(200.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 300.0));
        // Crop to the left half, with a hole.
        l.meta.masks = vec![
            Mask::rect("crop", Rect::new(-100.0, -50.0, 0.0, 50.0)),
            Mask { mode: MaskMode::Subtract, ..Mask::rect("hole", Rect::new(-60.0, -10.0, -40.0, 10.0)) },
        ];
        p.compositions[0].layers.push(l);
        let scene = evaluate_project(&p, comp, 0.0);
        let hit = |x, y| hit_test(&scene, kurbo::Point::new(x, y), 0.0, &mut ApproxText);
        assert_eq!(hit(220.0, 300.0), Some(Id(50)));
        assert_eq!(hit(350.0, 300.0), None, "cropped away");
        assert_eq!(hit(250.0, 300.0), None, "in the hole");
        let b = world_box(&p.compositions[0], &scene, Id(50), &mut ApproxText).unwrap();
        assert_eq!(b, Rect::new(200.0, 250.0, 300.0, 350.0));
    }
}

#[cfg(test)]
mod effect_anim_tests {
    use super::*;
    use xenify_doc::{Effect, EffectField, EffectKind, Fx, Keyframe, Project, PropPath, ShapeItem, Vec2};

    #[test]
    fn effect_settings_animate() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(7), "glow", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        l.meta.effects = vec![Effect::new(EffectKind::Glow { color: Color::WHITE.into(), radius: 0.0.into() })];
        let path = PropPath::Effect { index: 0, field: EffectField::Radius };
        assert!(l.property_paths().contains(&path));
        let keys = xenify_doc::Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: 0.0, easing: Default::default() },
            Keyframe { time: 1.0, value: 20.0, easing: xenify_doc::Easing::Linear },
        ])
        .unwrap();
        l.replace_property(path, xenify_doc::AnyProperty::Scalar(keys)).unwrap();
        p.compositions[0].layers.push(l);
        let radius = |t| match &evaluate_project(&p, comp, t).layer(Id(7)).unwrap().effects[0] {
            Fx::Glow { radius, .. } => *radius,
            other => panic!("{other:?}"),
        };
        assert_eq!(radius(0.0), 0.0);
        assert!((radius(0.5) - 10.0).abs() < 1e-9);
        assert_eq!(path.to_string(), "effects.0.radius");
        assert_eq!("effects.0.radius".parse::<PropPath>(), Ok(path));
    }
}
