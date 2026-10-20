//! SVG export: one frame, or a whole composition as an animated SVG.
//!
//! Every frame is built as a tree of [`El`]s whose `key`s identify the same
//! element across frames. A still is one tree written out. An animation
//! merges the trees of all frames: attributes that change become CSS
//! `@keyframes` (SMIL `<animate>` for geometry CSS can't animate, such as
//! `d`), and elements that come and go get a `visibility` animation.
//!
//! Top-level layers whose transform and opacity are plain keyframes (no
//! parent, wiggle, expression or perspective) keep their keyframes and
//! easing: the transform is split into nested groups (position, rotation,
//! scale, anchor) animated with the keyframes' own `cubic-bezier()` /
//! `steps()` timing functions. Everything else is sampled once per frame.

use xenify_anim::{Scene, SceneContent, SceneLayer, SceneText, TextMeasure};
use xenify_doc::{
    Animatable, Color, Easing, FillRule, GradientKind, Id, Layer, LineCap, LineJoin, MatteMode, Project, PropPath,
    Property, TextAlign, Vec2,
};
use kurbo::Affine;
use std::collections::HashMap;
use std::fmt::Write;

/// Options for [`export_animated_svg`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimatedSvgOptions {
    /// Repeat forever; otherwise play once and hold the last frame.
    pub looping: bool,
}

impl Default for AnimatedSvgOptions {
    fn default() -> Self {
        Self { looping: true }
    }
}

/// Writes one frame (an evaluated scene) as a standalone SVG document.
pub fn export_svg(scene: &Scene, measure: &mut dyn TextMeasure) -> String {
    let (defs, body) = Builder::new(scene, measure, &HashMap::new(), false).build();
    let mut w = Writer::default();
    for el in &defs {
        w.still(el, 2);
    }
    let defs = std::mem::take(&mut w.out);
    for el in &body {
        w.still(el, 1);
    }
    document(scene.width, scene.height, "", &defs, &w.out, &image_uris(std::slice::from_ref(scene)))
}

/// Writes composition `comp` of `project` as an animated SVG that plays in
/// browsers (CSS animations, plus SMIL for path shapes and gradient geometry).
pub fn export_animated_svg(
    project: &Project,
    comp: Id,
    measure: &mut dyn TextMeasure,
    options: AnimatedSvgOptions,
) -> Result<String, String> {
    let c = project.comp(comp).map_err(|e| e.to_string())?;
    let (fps, duration) = (c.settings.fps, c.settings.duration);
    let frames = c.settings.frame_count().max(1);
    if fps <= 0.0 || duration <= 0.0 {
        return Err("the composition has no duration".into());
    }
    let times: Vec<f64> = (0..frames).map(|f| c.settings.frame_time(f)).collect();
    let scenes: Vec<Scene> = times.iter().map(|&t| xenify_anim::evaluate_project(project, comp, t)).collect();
    let native = native_layers(&c.layers, &scenes, &times, duration);
    let has_camera = c.layers.iter().any(|l| matches!(l.kind, xenify_doc::LayerKind::Camera(_)));

    let mut defs = Vec::new();
    let mut body = Vec::new();
    for scene in &scenes {
        let mut builder = Builder::new(scene, measure, &native, true);
        builder.camera = has_camera;
        let (d, b) = builder.build();
        defs.push(d);
        body.push(b);
    }
    let timing = Timing { times, duration, looping: options.looping };
    let mut w = Writer::default();
    for el in merge(&defs.iter().map(|d| d.as_slice()).collect::<Vec<_>>()) {
        w.animated(&el, &timing, 2, None);
    }
    let defs = std::mem::take(&mut w.out);
    for el in merge(&body.iter().map(|b| b.as_slice()).collect::<Vec<_>>()) {
        w.animated(&el, &timing, 1, None);
    }
    let first = &scenes[0];
    Ok(document(first.width, first.height, &w.css, &defs, &w.out, &image_uris(&scenes)))
}

/// Data URIs of every image drawn in `scenes`, by content hash.
fn image_uris(scenes: &[Scene]) -> HashMap<u64, String> {
    fn add(img: &xenify_anim::SceneImage, out: &mut HashMap<u64, String>) {
        out.entry(img.hash).or_insert_with(|| {
            use base64::Engine;
            // A video frame goes in as a picture of itself.
            let (mime, bytes) = crate::image::scene_file(img).unwrap_or_default();
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            format!("data:{mime};base64,{data}")
        });
    }
    fn walk(layers: &[SceneLayer], out: &mut HashMap<u64, String>) {
        for l in layers {
            match &l.content {
                SceneContent::Image(img) => add(img, out),
                SceneContent::Shapes(items) => {
                    for p in items.iter().filter_map(|i| i.pattern.as_ref()) {
                        add(&p.image, out);
                    }
                }
                SceneContent::Precomp { layers, .. } | SceneContent::Group(layers) => walk(layers, out),
                _ => {}
            }
            if let Some(src) = l.matte.as_ref().and_then(|m| m.source.as_ref()) {
                walk(std::slice::from_ref(src), out);
            }
        }
    }
    let mut out = HashMap::new();
    for s in scenes {
        walk(&s.layers, &mut out);
    }
    out
}

/// A stand-in for image `hash`'s data URI, swapped in when the document is
/// finished (so animations don't carry megabytes of base64 per frame).
fn image_token(hash: u64) -> String {
    format!("\u{3}{hash:016x}\u{3}")
}

fn document(w: u32, h: u32, css: &str, defs: &str, body: &str, images: &HashMap<u64, String>) -> String {
    // Blend modes are CSS classes (the style attribute carries animations).
    let mut css = css.to_string();
    for b in xenify_doc::BlendMode::ALL.iter().filter(|b| !b.is_normal()) {
        if body.contains(&format!("class=\"blend-{}\"", b.css())) {
            css.push_str(&format!("    .blend-{0} {{ mix-blend-mode: {0} }}\n", b.css()));
        }
    }
    let style = if css.is_empty() { String::new() } else { format!("  <style>\n{css}  </style>\n") };
    let doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n{style}  <defs>\n{defs}  </defs>\n{body}</svg>\n"
    );
    let mut doc = resolve_refs(&doc);
    for (hash, uri) in images {
        doc = doc.replace(&image_token(*hash), uri);
    }
    doc
}

/// A colour without alpha: `#rrggbb`, or `rgb(r, g, b)` in animations (so
/// frames can be interpolated and compressed).
fn hex(c: Color, animated: bool) -> String {
    let [r, g, b, _] = c.to_rgba8();
    if animated { format!("rgb({r}, {g}, {b})") } else { format!("#{r:02x}{g:02x}{b:02x}") }
}

/// Path data with coordinates rounded like [`num`].
fn path_d(p: &kurbo::BezPath) -> String {
    use kurbo::PathEl;
    let pt = |p: kurbo::Point| format!("{},{}", num(p.x), num(p.y));
    let mut d = String::new();
    for el in p.elements() {
        if !d.is_empty() {
            d.push(' ');
        }
        let _ = match *el {
            PathEl::MoveTo(a) => write!(d, "M{}", pt(a)),
            PathEl::LineTo(a) => write!(d, "L{}", pt(a)),
            PathEl::QuadTo(a, b) => write!(d, "Q{} {}", pt(a), pt(b)),
            PathEl::CurveTo(a, b, c) => write!(d, "C{} {} {}", pt(a), pt(b), pt(c)),
            PathEl::ClosePath => write!(d, "Z"),
        };
    }
    d
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A number rounded to 4 decimals, without trailing zeros.
fn num(v: f64) -> String {
    let r = (v * 10_000.0).round() / 10_000.0;
    format!("{}", if r == 0.0 { 0.0 } else { r })
}

// ---------------------------------------------------------------- element tree

/// An SVG element. `key` identifies it across frames.
#[derive(Clone, Debug, PartialEq)]
struct El {
    key: String,
    tag: &'static str,
    attrs: Vec<(&'static str, Attr)>,
    children: Vec<El>,
    /// Escaped character content.
    text: Option<String>,
}

impl El {
    fn new(key: impl Into<String>, tag: &'static str) -> Self {
        Self { key: key.into(), tag, attrs: Vec::new(), children: Vec::new(), text: None }
    }
    fn attr(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.attrs.push((name, Attr::Value(value.into())));
        self
    }
    fn track(mut self, name: &'static str, track: Track) -> Self {
        self.attrs.push((name, Attr::Track(track)));
        self
    }
    fn child(mut self, el: El) -> Self {
        self.children.push(el);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Attr {
    /// This frame's value.
    Value(String),
    /// Animated from document keyframes (identical in every frame).
    Track(Track),
}

/// A reference to the element with key `key` (`id="…"` or `url(#…)`); ids
/// are assigned when the document is finished (see [`resolve_refs`]).
fn id_of(key: &str) -> String {
    format!("\u{1}{key}\u{2}")
}

/// Replaces the key tokens left by [`id_of`] with short ids (`g1`, `m1`, `c1`
/// by kind, numbered in order of appearance).
fn resolve_refs(doc: &str) -> String {
    let mut ids: HashMap<&str, String> = HashMap::new();
    let mut counts: HashMap<char, usize> = HashMap::new();
    let mut out = String::with_capacity(doc.len());
    let mut rest = doc;
    while let Some(start) = rest.find('\u{1}') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('\u{2}').map_or(rest.len(), |e| start + e);
        let key = &rest[start + 1..end];
        let id = ids.entry(key).or_insert_with(|| {
            let kind = key.chars().next().unwrap_or('x');
            let n = counts.entry(kind).or_default();
            *n += 1;
            format!("{kind}{n}")
        });
        out.push_str(id);
        rest = &rest[(end + 1).min(rest.len())..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- building one frame

struct Builder<'a> {
    scene: &'a Scene,
    /// Building one frame of an animation: colours are written as `rgb()`
    /// (so they interpolate) and invisible layers are kept.
    animated: bool,
    measure: &'a mut dyn TextMeasure,
    native: &'a HashMap<Id, NativeLayer>,
    defs: Vec<El>,
    /// Put the layers in a root `<g>` carrying the camera's view (an
    /// animation keeps it on every frame, so it can move).
    camera: bool,
}

impl<'a> Builder<'a> {
    fn new(
        scene: &'a Scene,
        measure: &'a mut dyn TextMeasure,
        native: &'a HashMap<Id, NativeLayer>,
        animated: bool,
    ) -> Self {
        let camera = scene.camera != kurbo::Affine::IDENTITY;
        Self { scene, animated, measure, native, defs: Vec::new(), camera }
    }

    /// `(defs, body)`.
    fn build(mut self) -> (Vec<El>, Vec<El>) {
        let (w, h) = (self.scene.width, self.scene.height);
        let mut body = vec![
            El::new("bg", "rect")
                .attr("width", w.to_string())
                .attr("height", h.to_string())
                .attr("fill", hex(self.scene.background, self.animated)),
        ];
        if self.camera {
            let mut layers = Vec::new();
            self.layers(&mut layers, &self.scene.layers, "", None, true);
            let m = self.scene.camera.as_coeffs();
            let m =
                format!("matrix({} {} {} {} {} {})", num(m[0]), num(m[1]), num(m[2]), num(m[3]), num(m[4]), num(m[5]));
            let mut view = El::new("camera", "g").attr("transform", m);
            view.children = layers;
            body.push(view);
        } else {
            self.layers(&mut body, &self.scene.layers, "", None, true);
        }
        (self.defs, body)
    }

    /// Layers in paint order, with their track mattes as `<mask>`s.
    fn layers(&mut self, out: &mut Vec<El>, layers: &[SceneLayer], prefix: &str, force: Option<Color>, top: bool) {
        for layer in layers {
            if layer.opacity <= 0.0 && !self.animated {
                continue;
            }
            // Copies made by modifiers share the layer's id.
            let key = match layer.copy {
                0 => format!("{prefix}{}", layer.id.0),
                n => format!("{prefix}{}~{n}", layer.id.0),
            };
            if layer.masks.is_empty() {
                self.matted(out, layer, &key, force, top);
            } else {
                let mut inner = Vec::new();
                self.matted(&mut inner, layer, &key, force, top);
                out.push(self.masked(layer, &key, inner));
            }
        }
    }

    /// Wraps `content` in `<mask>`s built from the layer's masks. Add and
    /// subtract paint white / black in one mask; each intersect nests a
    /// further mask; difference is approximated as add.
    fn masked(&mut self, layer: &SceneLayer, key: &str, content: Vec<El>) -> El {
        use xenify_doc::MaskMode;
        let (w, h) = (self.scene.width as f64, self.scene.height as f64);
        let big = kurbo::Rect::new(-w, -h, 2.0 * w, 2.0 * h);
        // Split into stages: a run of add/subtract masks, then one per intersect.
        let mut stages: Vec<Vec<&xenify_anim::SceneMask>> = vec![Vec::new()];
        for m in &layer.masks {
            if m.mode == MaskMode::Intersect && !stages.last().unwrap().is_empty() {
                stages.push(Vec::new());
            }
            stages.last_mut().unwrap().push(m);
        }
        let mut group = El::new(format!("{key}:mk"), "g");
        group.children = content;
        for (si, stage) in stages.into_iter().enumerate() {
            let mkey = format!("k:{key}/{si}");
            let mut mask = El::new(mkey.clone(), "mask")
                .attr("id", id_of(&mkey))
                .attr("maskUnits", "userSpaceOnUse")
                .attr("x", num(big.x0))
                .attr("y", num(big.y0))
                .attr("width", num(big.width()))
                .attr("height", num(big.height()));
            if stage.first().is_some_and(|m| m.mode == MaskMode::Subtract) {
                mask.children.push(
                    El::new(format!("{mkey}/all"), "rect")
                        .attr("x", num(big.x0))
                        .attr("y", num(big.y0))
                        .attr("width", num(big.width()))
                        .attr("height", num(big.height()))
                        .attr("fill", "#ffffff"),
                );
            }
            for (i, m) in stage.iter().enumerate() {
                let colour = if m.mode == MaskMode::Subtract { "#000000" } else { "#ffffff" };
                let mut d = path_d(&m.path);
                if m.inverted {
                    d = format!("{} {d}", path_d(&kurbo::Shape::to_path(&big, 0.1)));
                }
                let mut el = El::new(format!("{mkey}/{i}"), "path")
                    .attr("d", d)
                    .attr("fill", colour)
                    .attr("fill-rule", if m.inverted { "evenodd" } else { "nonzero" });
                if m.opacity < 1.0 {
                    el = el.attr("fill-opacity", format!("{:.3}", m.opacity));
                }
                if m.feather > 0.0 {
                    let fkey = format!("f:{mkey}/{i}");
                    self.defs.push(
                        El::new(fkey.clone(), "filter")
                            .attr("id", id_of(&fkey))
                            .attr("filterUnits", "userSpaceOnUse")
                            .attr("x", num(big.x0))
                            .attr("y", num(big.y0))
                            .attr("width", num(big.width()))
                            .attr("height", num(big.height()))
                            .child(
                                El::new(format!("{fkey}/b"), "feGaussianBlur")
                                    .attr("stdDeviation", num(m.feather / 2.0)),
                            ),
                    );
                    el = el.attr("filter", format!("url(#{})", id_of(&fkey)));
                }
                mask.children.push(el);
            }
            self.defs.push(mask);
            group = El::new(format!("{key}:mk{si}"), "g").attr("mask", format!("url(#{})", id_of(&mkey))).child(group);
        }
        group
    }

    /// One layer with its track matte (as a `<mask>`), if it has one.
    fn matted(&mut self, out: &mut Vec<El>, layer: &SceneLayer, key: &str, force: Option<Color>, top: bool) {
        let (w, h) = (self.scene.width, self.scene.height);
        let Some(matte) = &layer.matte else {
            self.layer(out, layer, key, force, top);
            return;
        };
        // Luminance masks: white shows, black hides. Alpha mattes paint the
        // matte white (or black on white when inverted); luma inverted is
        // approximated the same way.
        let mask_key = format!("m:{key}");
        let mut mask = El::new(mask_key.clone(), "mask")
            .attr("id", id_of(&mask_key))
            .attr("maskUnits", "userSpaceOnUse")
            .attr("x", "0")
            .attr("y", "0")
            .attr("width", w.to_string())
            .attr("height", h.to_string());
        let matte_force = match matte.mode {
            MatteMode::Alpha => Some(Color::WHITE),
            MatteMode::Luma => None,
            MatteMode::AlphaInverted | MatteMode::LumaInverted => {
                mask.children.push(
                    El::new(format!("{mask_key}/bg"), "rect")
                        .attr("width", w.to_string())
                        .attr("height", h.to_string())
                        .attr("fill", "#ffffff"),
                );
                Some(Color::BLACK)
            }
        };
        if let Some(source) = &matte.source {
            self.layer(&mut mask.children, source, &format!("{mask_key}/{}", source.id.0), matte_force, false);
        }
        self.defs.push(mask);
        let mut group = El::new(format!("{key}:m"), "g").attr("mask", format!("url(#{})", id_of(&mask_key)));
        self.layer(&mut group.children, layer, key, force, top);
        out.push(group);
    }

    /// One layer as a `<g>`; `force` paints everything in one colour (for masks).
    fn layer(&mut self, out: &mut Vec<El>, layer: &SceneLayer, key: &str, force: Option<Color>, top: bool) {
        if layer.opacity <= 0.0 && !self.animated {
            return;
        }
        let animated = self.animated;
        let native = self.native.get(&layer.id).filter(|_| top && force.is_none() && layer.copy == 0);
        let recolor = |c: Color| force.map_or(c, |f| Color { a: c.a, ..f });
        let hex = |c: Color| hex(c, animated);
        let matrix = |m: Affine| {
            let m = m.as_coeffs();
            format!("matrix({} {} {} {} {} {})", num(m[0]), num(m[1]), num(m[2]), num(m[3]), num(m[4]), num(m[5]))
        };
        let opacity =
            |el: El| if layer.opacity < 1.0 { el.attr("opacity", format!("{:.3}", layer.opacity)) } else { el };
        if let SceneContent::Precomp { width, height, layers } = &layer.content {
            // Children are already in scene space; clip them to the precomp's frame.
            let clip_key = format!("c:{key}");
            self.defs.push(
                El::new(clip_key.clone(), "clipPath").attr("id", id_of(&clip_key)).child(
                    El::new(format!("{clip_key}/r"), "rect")
                        .attr("width", num(*width))
                        .attr("height", num(*height))
                        .attr("transform", matrix(layer.transform)),
                ),
            );
            let mut group = opacity(El::new(key, "g").attr("clip-path", format!("url(#{})", id_of(&clip_key))));
            self.layers(&mut group.children, layers, &format!("{key}/"), force, false);
            let group = self.effects(layer, key, group);
            out.push(group);
            return;
        }
        // A group drawn as one: its members, with the group's opacity and effects.
        if let SceneContent::Group(layers) = &layer.content {
            let mut group = opacity(El::new(key, "g"));
            self.layers(&mut group.children, layers, &format!("{key}/"), force, false);
            let group = self.effects(layer, key, group);
            out.push(group);
            return;
        }

        // The content goes into `inner`: the layer group itself, or the
        // innermost of the nested transform groups of a keyframed layer.
        let mut content = Vec::new();
        match &layer.content {
            SceneContent::Shapes(items) => {
                for (i, item) in items.iter().enumerate() {
                    let item_key = format!("{key}/{i}");
                    let mut el = El::new(item_key.clone(), "path").attr("d", path_d(&item.path));
                    match (&item.gradient, item.fill) {
                        _ if item.pattern.is_some() && force.is_none() => {
                            let url = self.pattern_def(&format!("t:{item_key}"), item.pattern.as_ref().unwrap());
                            el = el.attr("fill", url);
                        }
                        (Some(g), _) => {
                            let url = self.gradient_def(&format!("g:{item_key}"), g, &recolor);
                            el = el.attr("fill", url);
                        }
                        (None, Some(c)) => {
                            let c = recolor(c);
                            el = el.attr("fill", hex(c));
                            if c.a < 1.0 {
                                el = el.attr("fill-opacity", format!("{:.3}", c.a));
                            }
                        }
                        (None, None) => el = el.attr("fill", "none"),
                    }
                    if item.fill_rule == FillRule::EvenOdd {
                        el = el.attr("fill-rule", "evenodd");
                    }
                    // Styled strokes are filled shapes: write them as their own paths.
                    let marks = item.stroke.as_ref().and_then(|s| s.marks.as_ref().map(|m| (s.color, m)));
                    if let Some(s) = item.stroke.as_ref().filter(|s| s.marks.is_none()) {
                        let join = match s.join {
                            LineJoin::Miter => "miter",
                            LineJoin::Round => "round",
                            LineJoin::Bevel => "bevel",
                        };
                        let cap = match s.cap {
                            LineCap::Butt => "butt",
                            LineCap::Round => "round",
                            LineCap::Square => "square",
                        };
                        let paint = match &s.gradient {
                            Some(g) => self.gradient_def(&format!("g:{item_key}/s"), g, &recolor),
                            None => hex(recolor(s.color)),
                        };
                        el = el
                            .attr("stroke", paint)
                            .attr("stroke-width", num(s.width))
                            .attr("stroke-linejoin", join)
                            .attr("stroke-linecap", cap);
                        if s.color.a < 1.0 && s.gradient.is_none() {
                            el = el.attr("stroke-opacity", format!("{:.3}", s.color.a));
                        }
                        if !s.dashes.is_empty() {
                            let d: Vec<String> = s.dashes.iter().map(|v| num(*v)).collect();
                            el = el.attr("stroke-dasharray", d.join(" "));
                            if s.dash_offset != 0.0 {
                                el = el.attr("stroke-dashoffset", num(s.dash_offset));
                            }
                        }
                    }
                    if item.fill.is_some() || item.gradient.is_some() || marks.is_none() {
                        content.push(el);
                    }
                    if let Some((color, marks)) = marks {
                        let c = recolor(color);
                        for (k, m) in marks.iter().enumerate() {
                            content.push(
                                El::new(format!("{item_key}/k{k}"), "path")
                                    .attr("d", path_d(&m.path))
                                    .attr("fill", hex(c))
                                    .attr("fill-opacity", format!("{:.3}", c.a * m.alpha)),
                            );
                        }
                    }
                }
            }
            SceneContent::Precomp { .. } | SceneContent::Group(_) => unreachable!("handled above"),
            SceneContent::Image(img) => {
                let (w, h) = (img.width.to_string(), img.height.to_string());
                match force {
                    // In a mask the image stands for its box.
                    Some(c) => content.push(
                        El::new(format!("{key}/img"), "rect").attr("width", w).attr("height", h).attr("fill", hex(c)),
                    ),
                    None => {
                        let def_key = self.image_def(img);
                        let mut el = El::new(format!("{key}/img"), "use").attr("href", format!("#{}", id_of(&def_key)));
                        if !img.adjust.is_empty() {
                            // Adjust Colours: hue, saturation, then contrast and brightness.
                            let fkey = format!("f:{key}/adj");
                            let mut filter = El::new(fkey.clone(), "filter")
                                .attr("id", id_of(&fkey))
                                .attr("color-interpolation-filters", "sRGB");
                            for (i, [b, k, sat, hue]) in img.adjust.iter().enumerate() {
                                let k = (1.0 + k).max(0.0);
                                let intercept = 0.5 - 0.5 * k + b;
                                let func = |ch: &'static str| {
                                    El::new(format!("{fkey}/{i}{ch}"), ch)
                                        .attr("type", "linear")
                                        .attr("slope", num(k))
                                        .attr("intercept", num(intercept))
                                };
                                filter = filter
                                    .child(
                                        El::new(format!("{fkey}/{i}h"), "feColorMatrix")
                                            .attr("type", "hueRotate")
                                            .attr("values", num(*hue)),
                                    )
                                    .child(
                                        El::new(format!("{fkey}/{i}s"), "feColorMatrix")
                                            .attr("type", "saturate")
                                            .attr("values", num((1.0 + sat).max(0.0))),
                                    )
                                    .child(
                                        El::new(format!("{fkey}/{i}t"), "feComponentTransfer")
                                            .child(func("feFuncR"))
                                            .child(func("feFuncG"))
                                            .child(func("feFuncB")),
                                    );
                            }
                            self.defs.push(filter);
                            el = el.attr("filter", format!("url(#{})", id_of(&fkey)));
                        }
                        content.push(el);
                    }
                }
            }
            SceneContent::Text(t) => {
                let t = SceneText { color: recolor(t.color), ..t.clone() };
                match &t.path {
                    Some(tp) => {
                        let def_key = format!("p:{key}");
                        self.defs.push(
                            El::new(def_key.clone(), "path").attr("id", id_of(&def_key)).attr("d", path_d(&tp.path)),
                        );
                        content.push(text_on_path(&t, key, &def_key, tp.offset, animated));
                    }
                    None if t.is_animated() && self.measure.glyph_outlines(&t).is_some() => {
                        // Letters animating: each glyph is its own outline (fonts can't do this).
                        let glyphs = self.measure.glyph_outlines(&t).unwrap_or_default();
                        let mut g = El::new(format!("{key}/glyphs"), "g");
                        for (i, (path, opacity)) in glyphs.iter().enumerate() {
                            g.children.push(
                                El::new(format!("{key}/g{i}"), "path")
                                    .attr("d", path_d(path))
                                    .attr("fill", hex(t.color))
                                    .attr("fill-opacity", format!("{:.3}", t.color.a * opacity)),
                            );
                        }
                        content.push(g);
                    }
                    None => {
                        let lines = text_elements(&t, key, self.measure, animated);
                        match t.frame().filter(|f| f.clip) {
                            // Hidden overflow: clip the lines to the box.
                            Some(f) => {
                                let clip_key = format!("tc:{key}");
                                self.defs.push(
                                    El::new(clip_key.clone(), "clipPath").attr("id", id_of(&clip_key)).child(
                                        El::new(format!("{clip_key}/r"), "rect")
                                            .attr("x", num(f.rect.x0))
                                            .attr("y", num(f.rect.y0))
                                            .attr("width", num(f.rect.width()))
                                            .attr("height", num(f.rect.height())),
                                    ),
                                );
                                let mut g = El::new(format!("{key}/tc"), "g")
                                    .attr("clip-path", format!("url(#{})", id_of(&clip_key)));
                                g.children = lines;
                                content.push(g);
                            }
                            None => content.extend(lines),
                        }
                    }
                }
            }
        }

        let Some(native) = native else {
            let mut group = opacity(El::new(key, "g").attr("transform", matrix(layer.transform)));
            group.children = content;
            let group = self.effects(layer, key, group);
            out.push(group);
            return;
        };
        // position → rotation → scale → -anchor, outermost first.
        let mut inner = El::new(format!("{key}/a"), "g").track("transform", native.anchor.clone());
        inner.children = content;
        let scale = El::new(format!("{key}/s"), "g").track("transform", native.scale.clone()).child(inner);
        let rotation = El::new(format!("{key}/r"), "g").track("transform", native.rotation.clone()).child(scale);
        let group = El::new(key, "g")
            .track("transform", native.position.clone())
            .track("opacity", native.opacity.clone())
            .child(rotation);
        let group = self.effects(layer, key, group);
        out.push(group);
    }

    /// The image's `<image>` in `<defs>` (once per picture); returns its key.
    fn image_def(&mut self, img: &xenify_anim::SceneImage) -> String {
        let def_key = format!("i:{:016x}", img.hash);
        if !self.defs.iter().any(|d| d.key == def_key) {
            self.defs.push(
                El::new(def_key.clone(), "image")
                    .attr("id", id_of(&def_key))
                    .attr("width", img.width.to_string())
                    .attr("height", img.height.to_string())
                    .attr("preserveAspectRatio", "none")
                    .attr("href", image_token(img.hash)),
            );
        }
        def_key
    }

    /// A pattern fill in `<defs>`; returns the `url(#…)` paint.
    fn pattern_def(&mut self, key: &str, p: &xenify_anim::ScenePattern) -> String {
        let image = self.image_def(&p.image);
        let m = p.transform.as_coeffs();
        let matrix =
            format!("matrix({} {} {} {} {} {})", num(m[0]), num(m[1]), num(m[2]), num(m[3]), num(m[4]), num(m[5]));
        self.defs.push(
            El::new(key.to_string(), "pattern")
                .attr("id", id_of(key))
                .attr("patternUnits", "userSpaceOnUse")
                .attr("width", p.image.width.to_string())
                .attr("height", p.image.height.to_string())
                .attr("patternTransform", matrix)
                .child(El::new(format!("{key}/u"), "use").attr("href", format!("#{}", id_of(&image)))),
        );
        format!("url(#{})", id_of(key))
    }

    /// A gradient in `<defs>` (in the layer's own coordinates); returns the
    /// `url(#…)` paint referring to it.
    fn gradient_def(&mut self, key: &str, g: &xenify_doc::Gradient, recolor: &dyn Fn(Color) -> Color) -> String {
        let animated = self.animated;
        let sorted = g.sorted_stops();
        let stops = sorted.iter().enumerate().map(|(s, stop)| {
            El::new(format!("{key}/{s}"), "stop")
                .attr("offset", format!("{:.4}", stop.offset))
                .attr("stop-color", hex(recolor(stop.color), animated))
                .attr("stop-opacity", format!("{:.3}", stop.color.a))
        });
        // userSpaceOnUse = this layer's coordinate system (inside the <g>).
        let grad = match g.kind {
            GradientKind::Linear => El::new(key.to_string(), "linearGradient")
                .attr("id", id_of(key))
                .attr("gradientUnits", "userSpaceOnUse")
                .attr("x1", num(g.start.x))
                .attr("y1", num(g.start.y))
                .attr("x2", num(g.end.x))
                .attr("y2", num(g.end.y)),
            GradientKind::Radial => El::new(key.to_string(), "radialGradient")
                .attr("id", id_of(key))
                .attr("gradientUnits", "userSpaceOnUse")
                .attr("cx", num(g.start.x))
                .attr("cy", num(g.start.y))
                .attr("r", num((g.end - g.start).hypot())),
        };
        self.defs.push(stops.fold(grad, El::child));
        format!("url(#{})", id_of(key))
    }

    /// Wraps a layer's group for its effects and blend mode: an SVG filter
    /// (shadows, glows, blur) and a `mix-blend-mode` class. The wrapper has no
    /// transform, so effect sizes are composition pixels like in Xenify Vector.
    fn effects(&mut self, layer: &SceneLayer, key: &str, group: El) -> El {
        if layer.effects.is_empty() && layer.blend.is_normal() {
            return group;
        }
        let mut wrap = El::new(format!("{key}:fx"), "g");
        if !layer.blend.is_normal() {
            wrap = wrap.attr("class", format!("blend-{}", layer.blend.css()));
        }
        if !layer.effects.is_empty() {
            let (w, h) = (self.scene.width as f64, self.scene.height as f64);
            let fkey = format!("f:{key}");
            let mut filter = El::new(fkey.clone(), "filter")
                .attr("id", id_of(&fkey))
                .attr("filterUnits", "userSpaceOnUse")
                .attr("x", num(-w))
                .attr("y", num(-h))
                .attr("width", num(3.0 * w))
                .attr("height", num(3.0 * h))
                .attr("color-interpolation-filters", "sRGB");
            let mut merge = Vec::new();
            let mut n = 0;
            let mut fe = |tag: &'static str| {
                n += 1;
                El::new(format!("{fkey}/{n}"), tag)
            };
            let mut content = "SourceGraphic".to_string();
            for (i, e) in layer.effects.iter().enumerate() {
                let (color, offset, radius) = match *e {
                    xenify_doc::Fx::DropShadow { color, offset, blur } => (color, offset, blur),
                    xenify_doc::Fx::Glow { color, radius } => (color, kurbo::Vec2::ZERO, radius),
                    xenify_doc::Fx::Blur { radius } => {
                        filter = filter.child(
                            fe("feGaussianBlur")
                                .attr("in", "SourceGraphic")
                                .attr("stdDeviation", num(radius / 2.0))
                                .attr("result", "content"),
                        );
                        content = "content".into();
                        continue;
                    }
                    xenify_doc::Fx::AdjustColors { .. } => continue,
                };
                let r = format!("s{i}");
                filter = filter
                    .child(fe("feGaussianBlur").attr("in", "SourceAlpha").attr("stdDeviation", num(radius / 2.0)))
                    .child(
                        fe("feOffset")
                            .attr("dx", num(offset.x))
                            .attr("dy", num(offset.y))
                            .attr("result", format!("{r}o")),
                    )
                    .child(
                        fe("feFlood")
                            .attr("flood-color", hex(color, false))
                            .attr("flood-opacity", format!("{:.3}", color.a)),
                    )
                    .child(
                        fe("feComposite").attr("in2", format!("{r}o")).attr("operator", "in").attr("result", r.clone()),
                    );
                merge.push(r);
            }
            merge.push(content);
            let mut m = fe("feMerge");
            for (k, r) in merge.into_iter().enumerate() {
                m = m.child(El::new(format!("{fkey}/m{k}"), "feMergeNode").attr("in", r));
            }
            self.defs.push(filter.child(m));
            wrap = wrap.attr("filter", format!("url(#{})", id_of(&fkey)));
        }
        wrap.child(group)
    }
}

/// Text as one `<text>` per laid-out line (wrapped as in Xenify Vector), placed with
/// the measured lines so alignment matches.
fn text_elements(t: &SceneText, key: &str, measure: &mut dyn TextMeasure, animated: bool) -> Vec<El> {
    measure
        .text_lines(t)
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.text.trim().is_empty())
        .map(|(i, line)| {
            let (anchor, x) = match t.align {
                TextAlign::Left => ("start", line.x0),
                TextAlign::Center => ("middle", (line.x0 + line.x1) / 2.0),
                TextAlign::Right => ("end", line.x1),
            };
            // The line's text is part of the key: an element's content can't animate.
            let mut el = El::new(format!("{key}/t{i}:{}", line.text), "text")
                .attr("x", num(x))
                .attr("y", num(line.baseline))
                .attr("font-family", esc(&t.font_family))
                .attr("font-size", num(t.font_size))
                .attr("fill", hex(t.color, animated))
                .attr("fill-opacity", format!("{:.3}", t.color.a))
                .attr("text-anchor", anchor);
            el = spacing(el, t);
            el.text = Some(esc(line.text.trim_end()));
            el
        })
        .collect()
}

fn spacing(mut el: El, t: &SceneText) -> El {
    if t.letter_spacing != 0.0 {
        el = el.attr("letter-spacing", num(t.letter_spacing));
    }
    if t.word_spacing != 0.0 {
        el = el.attr("word-spacing", num(t.word_spacing));
    }
    el
}

/// Text along the path `def_key` (an SVG `<textPath>`, so it stays text).
fn text_on_path(t: &SceneText, key: &str, def_key: &str, offset: f64, animated: bool) -> El {
    let (anchor, start) = match t.align {
        TextAlign::Left => ("start", num(offset)),
        TextAlign::Center => ("middle", format!("calc(50% + {}px)", num(offset))),
        TextAlign::Right => ("end", format!("calc(100% - {}px)", num(offset))),
    };
    // Plain percentages where the offset is zero (calc() isn't supported everywhere).
    let start = match (t.align, offset == 0.0) {
        (TextAlign::Center, true) => "50%".into(),
        (TextAlign::Right, true) => "100%".into(),
        _ => start,
    };
    let baseline = match t.valign {
        xenify_doc::TextVAlign::Baseline => "auto",
        xenify_doc::TextVAlign::Top => "hanging",
        xenify_doc::TextVAlign::Middle => "middle",
        xenify_doc::TextVAlign::Bottom => "text-after-edge",
    };
    let line = t.text.replace('\n', " ");
    let mut on_path = El::new(format!("{key}/tp:{line}"), "textPath")
        .attr("href", format!("#{}", id_of(def_key)))
        .attr("startOffset", start);
    on_path.text = Some(esc(&line));
    let el = El::new(format!("{key}/t:{line}"), "text")
        .attr("font-family", esc(&t.font_family))
        .attr("font-size", num(t.font_size))
        .attr("fill", hex(t.color, animated))
        .attr("fill-opacity", format!("{:.3}", t.color.a))
        .attr("text-anchor", anchor)
        .attr("dominant-baseline", baseline);
    spacing(el, t).child(on_path)
}

// ---------------------------------------------------------------- keyframed layers

/// A property animated from its document keyframes.
#[derive(Clone, Debug, PartialEq)]
struct Track {
    kind: TrackKind,
    /// `(time, value, easing to the next key)`; one entry for a static value.
    keys: Vec<(f64, [f64; 2], Easing)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TrackKind {
    Translate,
    /// Translate by minus the value (the anchor point).
    NegTranslate,
    Rotate,
    Scale,
    Opacity,
}

impl Track {
    fn of<T: Animatable>(
        kind: TrackKind,
        p: &Property<T>,
        duration: f64,
        get: impl Fn(&T) -> [f64; 2],
    ) -> Option<Self> {
        let keys: Vec<_> = match p {
            Property::Static(v) => vec![(0.0, get(v), Easing::Linear)],
            Property::Animated(keys) => keys.iter().map(|k| (k.time, get(&k.value), k.easing)).collect(),
        };
        // CSS keyframes live inside the animation's duration.
        keys.iter().all(|k| (-1e-9..=duration + 1e-9).contains(&k.0)).then_some(Self { kind, keys })
    }

    /// The value as an SVG attribute (`css` false) or a CSS property value.
    fn format(&self, v: [f64; 2], css: bool) -> String {
        let (x, y) = (num(v[0]), num(v[1]));
        let (nx, ny) = (num(-v[0]), num(-v[1]));
        match (self.kind, css) {
            (TrackKind::Translate, false) => format!("translate({x} {y})"),
            (TrackKind::Translate, true) => format!("translate({x}px, {y}px)"),
            (TrackKind::NegTranslate, false) => format!("translate({nx} {ny})"),
            (TrackKind::NegTranslate, true) => format!("translate({nx}px, {ny}px)"),
            (TrackKind::Rotate, false) => format!("rotate({x})"),
            (TrackKind::Rotate, true) => format!("rotate({x}deg)"),
            (TrackKind::Scale, false) => format!("scale({x} {y})"),
            (TrackKind::Scale, true) => format!("scale({x}, {y})"),
            (TrackKind::Opacity, _) => format!("{:.3}", v[0]),
        }
    }
}

/// The keyframes of a layer whose transform and opacity export as-is.
#[derive(Clone, Debug, PartialEq)]
struct NativeLayer {
    position: Track,
    rotation: Track,
    scale: Track,
    anchor: Track,
    opacity: Track,
}

/// Top-level layers whose transform and opacity can be written as their own
/// keyframes: checked against the evaluated scene at every frame, so anything
/// the scene does differently (parents, expressions, wiggle, rigs) falls back
/// to per-frame sampling.
fn native_layers(layers: &[Layer], scenes: &[Scene], times: &[f64], duration: f64) -> HashMap<Id, NativeLayer> {
    let mut out = HashMap::new();
    for layer in layers {
        let tr = &layer.transform;
        let simple = layer.meta.parent.is_none()
            && tr.wiggle.is_none()
            && tr.distort.is_none()
            && matches!(tr.skew, Property::Static(v) if v == 0.0)
            && !layer.meta.expressions.iter().any(|e| {
                matches!(
                    e.path,
                    PropPath::Anchor | PropPath::Position | PropPath::Scale | PropPath::Rotation | PropPath::Opacity
                )
            });
        if !simple {
            continue;
        }
        let v2 = |v: &Vec2| [v.x, v.y];
        let Some(native) = (|| {
            Some(NativeLayer {
                position: Track::of(TrackKind::Translate, &tr.position, duration, v2)?,
                rotation: Track::of(TrackKind::Rotate, &tr.rotation, duration, |r| [*r, 0.0])?,
                scale: Track::of(TrackKind::Scale, &tr.scale, duration, v2)?,
                anchor: Track::of(TrackKind::NegTranslate, &tr.anchor, duration, v2)?,
                opacity: Track::of(TrackKind::Opacity, &layer.opacity, duration, |o| [*o, 0.0])?,
            })
        })() else {
            continue;
        };
        let matches = scenes.iter().zip(times).all(|(scene, &t)| {
            // Top-level layers only (not inside precomps or used only as mattes).
            let Some(sl) = scene.layers.iter().find(|l| l.id == layer.id) else { return true };
            if sl.distort.is_some() || matches!(sl.content, SceneContent::Precomp { .. }) {
                return false;
            }
            let (p, s, a) = (tr.position.value_at(t), tr.scale.value_at(t), tr.anchor.value_at(t));
            let m = Affine::translate(p)
                * Affine::rotate(tr.rotation.value_at(t).to_radians())
                * Affine::scale_non_uniform(s.x, s.y)
                * Affine::translate(-a);
            let close = m.as_coeffs().iter().zip(sl.transform.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-6);
            close && (layer.opacity.value_at(t) - sl.opacity).abs() < 1e-6
        });
        if matches {
            out.insert(layer.id, native);
        }
    }
    out
}

// ---------------------------------------------------------------- merging frames

/// One element across all frames.
struct Merged {
    tag: &'static str,
    text: Option<String>,
    /// Per frame: whether the element exists.
    present: Vec<bool>,
    /// Per attribute, per frame (`None` where absent).
    attrs: Vec<(&'static str, Vec<Option<Attr>>)>,
    children: Vec<Merged>,
}

/// Merges one list of sibling elements per frame into the union of their
/// elements, in paint order.
fn merge(frames: &[&[El]]) -> Vec<Merged> {
    let mut order: Vec<&str> = Vec::new();
    for list in frames {
        let mut after: Option<usize> = None;
        for el in *list {
            match order.iter().position(|k| *k == el.key) {
                Some(i) => after = Some(i),
                None => {
                    let at = after.map_or(0, |i| i + 1);
                    order.insert(at, &el.key);
                    after = Some(at);
                }
            }
        }
    }
    let by_key: Vec<HashMap<&str, &El>> =
        frames.iter().map(|list| list.iter().map(|e| (e.key.as_str(), e)).collect()).collect();
    order
        .into_iter()
        .map(|key| {
            let found: Vec<Option<&El>> = by_key.iter().map(|m| m.get(key).copied()).collect();
            let first = found.iter().flatten().next().expect("every key comes from some frame");
            let mut names: Vec<&'static str> = Vec::new();
            for el in found.iter().flatten() {
                for (name, _) in &el.attrs {
                    if !names.contains(name) {
                        names.push(name);
                    }
                }
            }
            let attrs = names
                .into_iter()
                .map(|name| {
                    let per_frame = found
                        .iter()
                        .map(|e| e.and_then(|e| e.attrs.iter().find(|(n, _)| *n == name)).map(|(_, v)| v.clone()))
                        .collect();
                    (name, per_frame)
                })
                .collect();
            let empty: &[El] = &[];
            let children: Vec<&[El]> = found.iter().map(|e| e.map_or(empty, |e| e.children.as_slice())).collect();
            Merged {
                tag: first.tag,
                text: first.text.clone(),
                present: found.iter().map(Option::is_some).collect(),
                attrs,
                children: merge(&children),
            }
        })
        .collect()
}

/// An attribute's value when an element that has it in other frames doesn't
/// write it.
fn default_value(name: &str) -> Option<&'static str> {
    Some(match name {
        "opacity" | "fill-opacity" | "stroke-opacity" | "stop-opacity" => "1",
        "stroke" => "none",
        "stroke-width" => "1",
        "fill-rule" => "nonzero",
        "stroke-linejoin" => "miter",
        "stroke-linecap" => "butt",
        "transform" => "matrix(1 0 0 1 0 0)",
        _ => return None,
    })
}

/// Properties animated with CSS; everything else uses SMIL `<animate>`.
fn css_animatable(name: &str) -> bool {
    matches!(
        name,
        "transform"
            | "opacity"
            | "fill"
            | "fill-opacity"
            | "stroke"
            | "stroke-opacity"
            | "stroke-width"
            | "stop-color"
            | "stop-opacity"
            | "visibility"
    )
}

// ---------------------------------------------------------------- writing

struct Timing {
    /// Time of each frame, seconds.
    times: Vec<f64>,
    duration: f64,
    looping: bool,
}

impl Timing {
    fn pct(&self, t: f64) -> String {
        format!("{}%", num((t / self.duration * 100.0).clamp(0.0, 100.0)))
    }

    /// The `animation` shorthand for keyframes `name`.
    fn css_animation(&self, name: &str, timing: &str) -> String {
        let repeat = if self.looping { "infinite" } else { "1 forwards" };
        format!("{name} {}s {timing} {repeat}", num(self.duration))
    }
}

#[derive(Default)]
struct Writer {
    out: String,
    css: String,
    animations: usize,
}

impl Writer {
    fn open(&mut self, tag: &str, attrs: &str, depth: usize) {
        let _ = write!(self.out, "{}<{tag}{attrs}", "  ".repeat(depth));
    }

    fn close(&mut self, tag: &str, children: impl FnOnce(&mut Self), text: Option<&str>, depth: usize, empty: bool) {
        if empty && text.is_none() {
            self.out.push_str("/>\n");
            return;
        }
        self.out.push('>');
        if let Some(t) = text {
            self.out.push_str(t);
        } else {
            self.out.push('\n');
        }
        children(self);
        if text.is_none() {
            self.out.push_str(&"  ".repeat(depth));
        }
        let _ = writeln!(self.out, "</{tag}>");
    }

    /// A single frame's element.
    fn still(&mut self, el: &El, depth: usize) {
        let mut attrs = String::new();
        for (name, value) in &el.attrs {
            let v = match value {
                Attr::Value(v) => v.clone(),
                Attr::Track(t) => {
                    let v = t.keys.first().map_or([0.0; 2], |k| k.1);
                    t.format(v, false)
                }
            };
            // Tracks of an unchanging identity transform are left out.
            if matches!(value, Attr::Track(_)) && is_identity(name, &v) {
                continue;
            }
            let _ = write!(attrs, r#" {name}="{v}""#);
        }
        self.open(el.tag, &attrs, depth);
        let children = |w: &mut Self| {
            for c in &el.children {
                w.still(c, depth + 1);
            }
        };
        self.close(el.tag, children, el.text.as_deref(), depth, el.children.is_empty());
    }

    /// An element across all frames, with its animations.
    /// `shown`: the frames in which the parent is visible (`None` at the top).
    fn animated(&mut self, el: &Merged, timing: &Timing, depth: usize, shown: Option<&[bool]>) {
        // Visible where both it and its parent are. A child only needs its own
        // animation if it's missing while the parent shows (and then must
        // also hide wherever the parent does: `visible` would override it).
        let visible: Vec<bool> = match shown {
            Some(parent) => el.present.iter().zip(parent).map(|(a, b)| *a && *b).collect(),
            None => el.present.clone(),
        };
        let own_gaps = shown.map_or(el.present.iter().any(|p| !p), |parent| visible.as_slice() != parent);
        let mut attrs = String::new();
        let mut css = Vec::new();
        let mut smil = Vec::new();
        for (name, per_frame) in &el.attrs {
            if let Some(Attr::Track(t)) = per_frame.iter().flatten().next() {
                let first = t.format(t.keys[0].1, false);
                if t.keys.len() > 1 {
                    css.push(self.track_keyframes(t, name, timing));
                } else if is_identity(name, &first) {
                    continue;
                }
                let _ = write!(attrs, r#" {name}="{first}""#);
                continue;
            }
            let values = fill_gaps(name, per_frame, &el.present);
            let _ = write!(attrs, r#" {name}="{}""#, values[0]);
            if values.iter().all(|v| *v == values[0]) {
                continue;
            }
            let kept = compress(&values, matches!(*name, "fill" | "stroke" | "stop-color"));
            if css_animatable(name) {
                css.push(self.sampled_keyframes(name, &values, &kept, timing, "linear"));
            } else {
                smil.push(smil_animate(name, &values, &kept, timing));
            }
        }
        if own_gaps {
            let vis: Vec<String> = visible.iter().map(|p| if *p { "visible" } else { "hidden" }.to_string()).collect();
            let kept = compress(&vis, false);
            css.push(self.sampled_keyframes("visibility", &vis, &kept, timing, "step-end"));
            let _ = write!(attrs, r#" visibility="{}""#, vis[0]);
        }
        if !css.is_empty() {
            let _ = write!(attrs, r#" style="animation: {}""#, css.join(", "));
        }
        self.open(el.tag, &attrs, depth);
        let children = |w: &mut Self| {
            for s in &smil {
                let _ = writeln!(w.out, "{}{s}", "  ".repeat(depth + 1));
            }
            for c in &el.children {
                w.animated(c, timing, depth + 1, Some(&visible));
            }
        };
        let empty = el.children.is_empty() && smil.is_empty();
        self.close(el.tag, children, el.text.as_deref(), depth, empty);
    }

    fn next_name(&mut self) -> String {
        self.animations += 1;
        format!("k{}", self.animations)
    }

    /// `@keyframes` from a document track, with each key's easing as its
    /// timing function. Returns the `animation` value.
    fn track_keyframes(&mut self, t: &Track, name: &str, timing: &Timing) -> String {
        let anim = self.next_name();
        let _ = writeln!(self.css, "    @keyframes {anim} {{");
        let first = t.keys[0];
        let last = t.keys[t.keys.len() - 1];
        if first.0 > 1e-9 {
            let _ = writeln!(self.css, "      0% {{ {name}: {} }}", t.format(first.1, true));
        }
        for (time, v, easing) in &t.keys {
            let _ = writeln!(
                self.css,
                "      {} {{ {name}: {}; animation-timing-function: {} }}",
                timing.pct(*time),
                t.format(*v, true),
                easing.css()
            );
        }
        if last.0 < timing.duration - 1e-9 {
            let _ = writeln!(self.css, "      100% {{ {name}: {} }}", t.format(last.1, true));
        }
        self.css.push_str("    }\n");
        timing.css_animation(&anim, "linear")
    }

    /// `@keyframes` from per-frame values (only the `kept` frames).
    fn sampled_keyframes(
        &mut self,
        name: &str,
        values: &[String],
        kept: &[usize],
        timing: &Timing,
        function: &str,
    ) -> String {
        let anim = self.next_name();
        let _ = writeln!(self.css, "    @keyframes {anim} {{");
        for &i in kept {
            let _ = writeln!(
                self.css,
                "      {} {{ {name}: {} }}",
                timing.pct(timing.times[i]),
                css_value(name, &values[i])
            );
        }
        // Hold the last frame to the end.
        let last = *kept.last().expect("compress keeps the last frame");
        if timing.times[last] < timing.duration - 1e-9 {
            let _ = writeln!(self.css, "      100% {{ {name}: {} }}", css_value(name, &values[last]));
        }
        self.css.push_str("    }\n");
        timing.css_animation(&anim, function)
    }
}

/// Transform tracks that do nothing are left out of the markup.
fn is_identity(name: &str, v: &str) -> bool {
    name == "transform" && matches!(v, "translate(0 0)" | "rotate(0)" | "scale(1 1)")
}

/// An SVG attribute value as CSS.
fn css_value(name: &str, v: &str) -> String {
    if name == "transform" {
        // CSS functions need commas: matrix(1 0 0 1 5 5) → matrix(1, 0, 0, 1, 5, 5).
        v.split(' ').collect::<Vec<_>>().join(", ")
    } else {
        v.to_string()
    }
}

/// A SMIL `<animate>` for an attribute CSS can't animate (`d`, gradient geometry).
fn smil_animate(name: &str, values: &[String], kept: &[usize], timing: &Timing) -> String {
    let mut times: Vec<String> = kept.iter().map(|&i| num(timing.times[i] / timing.duration)).collect();
    let mut vals: Vec<&str> = kept.iter().map(|&i| values[i].as_str()).collect();
    // keyTimes must end at 1: hold the last value to the end.
    let last = *kept.last().expect("compress keeps the last frame");
    if timing.times[last] < timing.duration - 1e-9 {
        times.push("1".into());
        vals.push(&values[last]);
    }
    let repeat = if timing.looping { r#"repeatCount="indefinite""# } else { r#"fill="freeze""# };
    format!(
        r#"<animate attributeName="{name}" dur="{}s" {repeat} calcMode="linear" keyTimes="{}" values="{}"/>"#,
        num(timing.duration),
        times.join(";"),
        vals.join(";")
    )
}

/// Per-frame values of one attribute with the gaps filled: the attribute's
/// default where the element exists without it, the nearest value where the
/// element doesn't exist.
fn fill_gaps(name: &str, per_frame: &[Option<Attr>], present: &[bool]) -> Vec<String> {
    let mut values: Vec<Option<String>> = per_frame
        .iter()
        .zip(present)
        .map(|(v, &p)| match v {
            Some(Attr::Value(v)) => Some(v.clone()),
            Some(Attr::Track(_)) => None,
            None if p => default_value(name).map(str::to_string),
            None => None,
        })
        .collect();
    let first = values.iter().flatten().next().cloned().unwrap_or_default();
    let mut last = first;
    for v in &mut values {
        match v {
            Some(v) => last = v.clone(),
            None => *v = Some(last.clone()),
        }
    }
    values.into_iter().map(Option::unwrap_or_default).collect()
}

/// A value split into its numbers and the text around them, so values with
/// the same shape can be interpolated.
fn numbers(v: &str) -> (String, Vec<f64>) {
    if v.starts_with('#') {
        // A hex colour: not interpolated here (only exact repeats are dropped).
        return (v.to_string(), Vec::new());
    }
    let mut shape = String::new();
    let mut nums = Vec::new();
    let mut token = String::new();
    let flush = |token: &mut String, shape: &mut String, nums: &mut Vec<f64>| {
        if token.is_empty() {
            return;
        }
        match token.parse::<f64>() {
            Ok(n) => {
                nums.push(n);
                shape.push('#');
            }
            Err(_) => shape.push_str(token),
        }
        token.clear();
    };
    for c in v.chars() {
        if c == '-' {
            flush(&mut token, &mut shape, &mut nums);
            token.push(c);
        } else if c.is_ascii_digit() || c == '.' {
            token.push(c);
        } else {
            flush(&mut token, &mut shape, &mut nums);
            shape.push(c);
        }
    }
    flush(&mut token, &mut shape, &mut nums);
    (shape, nums)
}

/// The frames to keep as keyframes: a frame is dropped when linear
/// interpolation between its kept neighbours reproduces it.
/// `colour`: the numbers are 0–255 channels, rounded; allow ±1.
fn compress(values: &[String], colour: bool) -> Vec<usize> {
    let parsed: Vec<(String, Vec<f64>)> = values.iter().map(|v| numbers(v)).collect();
    let fits = |a: usize, b: usize| {
        (a + 1..b).all(|i| {
            if values[i] == values[a] && values[i] == values[b] {
                return true;
            }
            let (pa, pb, pi) = (&parsed[a], &parsed[b], &parsed[i]);
            if pa.0 != pi.0 || pb.0 != pi.0 || pi.1.is_empty() {
                return false;
            }
            let u = (i - a) as f64 / (b - a) as f64;
            pi.1.iter().zip(&pa.1).zip(&pb.1).all(|((x, xa), xb)| {
                let want = xa + (xb - xa) * u;
                (x - want).abs() <= if colour { 1.0 } else { 1e-3 * want.abs().max(1.0) }
            })
        })
    };
    let mut kept = vec![0];
    let mut a = 0;
    let mut b = 1;
    while b < values.len() {
        if b + 1 < values.len() && fits(a, b + 1) {
            b += 1;
            continue;
        }
        kept.push(b);
        a = b;
        b += 1;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Keyframe, ShapeItem};

    fn project_with(layer: Layer) -> (Project, Id) {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let c = project.comp_mut(comp).unwrap();
        c.settings.duration = 2.0;
        c.settings.fps = 10.0;
        c.layers = vec![layer];
        (project, comp)
    }

    fn keyed<T: Clone>(keys: &[(f64, T, Easing)]) -> Property<T> {
        Property::Animated(
            keys.iter()
                .map(|(time, value, easing)| Keyframe { time: *time, value: value.clone(), easing: *easing })
                .collect(),
        )
    }

    fn parses(svg: &str) {
        assert!(usvg::Tree::from_str(svg, &usvg::Options::default()).is_ok(), "{svg}");
    }

    #[test]
    fn array_copies_get_their_own_elements() {
        use xenify_doc::{Modifier, ModifierKind};
        let mut dot = Layer::shape(Id(1), "dot", vec![ShapeItem::ellipse(Vec2::new(20.0, 20.0), Color::WHITE)]);
        dot.transform.position =
            keyed(&[(0.0, Vec2::new(100.0, 100.0), Easing::Linear), (2.0, Vec2::new(300.0, 100.0), Easing::Linear)]);
        dot.meta.modifiers.push(Modifier::new(ModifierKind::array(3, Vec2::new(0.0, 40.0), Vec2::ZERO)));
        let (project, comp) = project_with(dot);
        let svg =
            export_animated_svg(&project, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
        let ids: Vec<&str> = svg.split(" id=\"").skip(1).filter_map(|s| s.split('"').next()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "no id is used twice: {svg}");
        assert_eq!(svg.matches("<path").count(), 3, "{svg}");
        assert_eq!(svg.matches("animation: k").count(), 3, "each copy moves on its own: {svg}");
        parses(&svg);
        let scene = xenify_anim::evaluate_project(&project, comp, 1.0);
        let still = export_svg(&scene, &mut xenify_anim::ApproxText);
        assert!(still.contains("matrix(1 0 0 1 200 180)"), "the last copy, 80 below: {still}");
    }

    #[test]
    fn a_boiling_outline_animates() {
        use xenify_doc::{Modifier, ModifierKind};
        let mut blob = Layer::shape(Id(1), "blob", vec![ShapeItem::ellipse(Vec2::new(80.0, 80.0), Color::WHITE)])
            .with_position(Vec2::new(200.0, 200.0));
        blob.meta.modifiers.push(Modifier::new(ModifierKind::Jitter {
            amount: 6.0.into(),
            detail: 8.0.into(),
            seed: keyed(&[(0.0, 0.0, Easing::Linear), (2.0, 10.0, Easing::Linear)]),
            smooth: true,
        }));
        blob.meta.modifiers.push(Modifier::new(ModifierKind::ZigZag {
            amount: 4.0.into(),
            ridges: keyed(&[(0.0, 1.0, Easing::Linear), (2.0, 4.0, Easing::Linear)]),
            smooth: false,
        }));
        let (project, comp) = project_with(blob);
        let svg =
            export_animated_svg(&project, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
        assert!(svg.contains("<animate"), "the outline changes: {}", &svg[..svg.len().min(3000)]);
        parses(&svg);
    }

    #[test]
    fn dashes_and_gradient_strokes_round_trip() {
        use xenify_doc::{Gradient, GradientKind, GradientStop, Stroke};
        let mut path = xenify_doc::BezPath::new();
        path.move_to((10.0, 10.0));
        path.line_to((200.0, 10.0));
        let mut s = ShapeItem::path(path, None, None);
        let g = Gradient {
            kind: GradientKind::Linear,
            start: Vec2::new(10.0, 0.0),
            end: Vec2::new(200.0, 0.0),
            stops: vec![
                GradientStop { offset: 0.0, color: Color::rgb(1.0, 0.0, 0.0) },
                GradientStop { offset: 1.0, color: Color::rgb(0.0, 0.0, 1.0) },
            ],
        };
        s.stroke = Some(Stroke {
            width: 6.0.into(),
            dashes: vec![12.0, 6.0],
            dash_offset: 3.0.into(),
            gradient: Some(g.into()),
            ..Default::default()
        });
        let (project, comp) = project_with(Layer::shape(Id(1), "line", vec![s]));
        let scene = xenify_anim::evaluate_project(&project, comp, 0.0);
        let svg = export_svg(&scene, &mut xenify_anim::ApproxText);
        assert!(svg.contains(r#"stroke-dasharray="12 6""#) && svg.contains(r#"stroke-dashoffset="3""#), "{svg}");
        assert!(svg.contains(r#"stroke="url(#"#) && svg.contains("<linearGradient"), "{svg}");
        parses(&svg);
        // And back in.
        let layers = crate::svg::import_svg(svg.as_bytes(), 10, 1920, 1080).unwrap();
        let st = layers.iter().flat_map(|l| l.shapes()).find_map(|s| s.stroke.clone()).expect("a stroke");
        // Import scales the drawing to fit, dashes included: same proportions as the width.
        let w = st.width.value_at(0.0);
        assert!(
            (st.dashes[0] / w - 2.0).abs() < 1e-6 && (st.dashes[1] / w - 1.0).abs() < 1e-6,
            "{:?} / {w}",
            st.dashes
        );
        assert!((st.dash_offset.value_at(0.0) / w - 0.5).abs() < 1e-6);
        assert!(st.gradient.is_some(), "the gradient survives import");
        // Lottie: a gradient stroke with dashes.
        let json = crate::lottie::export_lottie(&project, comp, &mut xenify_render::TextEngine::new()).unwrap();
        let text = json.to_string();
        assert!(text.contains(r#""ty":"gs""#) && text.contains(r#""n":"d""#) && text.contains(r#""n":"o""#), "{text}");
    }

    #[test]
    fn pattern_fills_export_and_import() {
        use xenify_doc::{Fill, Pattern};
        let png = xenify_render::RgbaImage { width: 2, height: 2, pixels: vec![255; 16] }.to_png();
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let asset = xenify_doc::ImageAsset::new(project.alloc_id(), "tile", "image/png", 2, 2, png);
        let mut shape = ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE);
        shape.fill = Some(Fill {
            pattern: Some(Pattern { scale: 8.0, rotation: 30.0, ..Pattern::new(asset.id) }),
            ..Fill::solid(Color::WHITE)
        });
        let id = project.alloc_id();
        project.assets.push(asset);
        project
            .comp_mut(comp)
            .unwrap()
            .layers
            .push(Layer::shape(id, "p", vec![shape]).with_position(Vec2::new(400.0, 300.0)));
        let scene = xenify_anim::evaluate_project(&project, comp, 0.0);
        let svg = export_svg(&scene, &mut xenify_anim::ApproxText);
        assert!(
            svg.contains("<pattern") && svg.contains("patternTransform") && svg.contains("data:image/png;base64"),
            "{svg}"
        );
        parses(&svg);
        // Back in: the pattern keeps its picture, size and angle.
        let import = crate::svg::import_svg_document(svg.as_bytes(), 10, 1920, 1080, Default::default()).unwrap();
        let p = import
            .layers
            .iter()
            .flat_map(|l| l.shapes())
            .find_map(|s| s.fill.as_ref().and_then(|f| f.pattern))
            .expect("a pattern fill");
        assert!(import.assets.iter().any(|a| a.id == p.asset));
        assert!((p.rotation - 30.0).abs() < 0.5, "{p:?}");
        assert!(p.scale > 1.0, "{p:?}");
    }

    #[test]
    fn masks_become_svg_masks_and_animate() {
        use xenify_doc::{Mask, MaskMode, kurbo::Rect};
        let mut photo = Layer::shape(Id(1), "photo", vec![ShapeItem::rect(Vec2::new(200.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 300.0));
        let mut crop = Mask::rect("Crop", Rect::new(-100.0, -50.0, 0.0, 50.0));
        crop.path = keyed(&[
            (0.0, xenify_doc::rect_path(Rect::new(-100.0, -50.0, 0.0, 50.0)), Easing::Linear),
            (1.0, xenify_doc::rect_path(Rect::new(-100.0, -50.0, 100.0, 50.0)), Easing::Linear),
        ]);
        photo.meta.masks = vec![
            crop,
            Mask {
                mode: MaskMode::Subtract,
                feather: 4.0.into(),
                ..Mask::ellipse("hole", Rect::new(-20.0, -20.0, 0.0, 0.0))
            },
            Mask { mode: MaskMode::Intersect, inverted: true, ..Mask::rect("x", Rect::new(-5.0, -5.0, 5.0, 5.0)) },
        ];
        let (project, comp) = project_with(photo);
        let svg =
            export_animated_svg(&project, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
        assert_eq!(svg.matches("<mask ").count(), 2, "add/subtract in one, the intersect nested: {svg}");
        assert!(svg.contains(r##"fill="#000000""##) && svg.contains("feGaussianBlur") && svg.contains("evenodd"));
        assert!(svg.contains(r#"<animate attributeName="d""#), "the crop animates: {svg}");
        parses(&svg);
    }

    #[test]
    fn keyframed_layers_keep_their_easing() {
        let mut dot = Layer::shape(Id(1), "dot", vec![ShapeItem::ellipse(Vec2::new(40.0, 40.0), Color::WHITE)]);
        dot.transform.position = keyed(&[
            (0.0, Vec2::new(100.0, 100.0), Easing::EASE_OUT),
            (1.0, Vec2::new(300.0, 100.0), Easing::Steps { count: 3, jump: xenify_doc::StepJump::End }),
            (2.0, Vec2::new(300.0, 400.0), Easing::Linear),
        ]);
        dot.opacity = keyed(&[(0.5, 1.0, Easing::Hold), (1.5, 0.25, Easing::Linear)]);
        let (project, comp) = project_with(dot);
        let svg =
            export_animated_svg(&project, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
        assert!(
            svg.contains(
                "0% { transform: translate(100px, 100px); animation-timing-function: cubic-bezier(0, 0, 0.58, 1) }"
            ),
            "{svg}"
        );
        assert!(
            svg.contains("50% { transform: translate(300px, 100px); animation-timing-function: steps(3, jump-end) }"),
            "{svg}"
        );
        assert!(svg.contains("25% { opacity: 1.000; animation-timing-function: step-end }"), "{svg}");
        assert!(svg.contains("100% { opacity: 0.250 }"), "{svg}");
        assert!(svg.contains("s linear infinite"));
        // The unanimated parts of the transform aren't written.
        assert!(!svg.contains("rotate(") && !svg.contains("scale("), "{svg}");
        parses(&svg);
    }

    #[test]
    fn parented_layers_are_sampled() {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let c = project.comp_mut(comp).unwrap();
        c.settings.duration = 1.0;
        c.settings.fps = 4.0;
        let mut parent = Layer::new(Id(1), "parent", xenify_doc::LayerKind::Null);
        parent.transform.rotation = keyed(&[(0.0, 0.0, Easing::Linear), (1.0, 90.0, Easing::Linear)]);
        let mut child = Layer::shape(Id(2), "box", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::WHITE)]);
        child.meta.parent = Some(Id(1));
        child.transform.position = Vec2::new(50.0, 0.0).into();
        c.layers = vec![parent, child];
        let svg =
            export_animated_svg(&project, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions { looping: false })
                .unwrap();
        assert!(svg.contains("{ transform: matrix("), "{svg}");
        assert!(svg.contains("1 forwards"), "{svg}");
        parses(&svg);
    }

    #[test]
    fn shapes_and_layers_that_come_and_go() {
        let mut shape = ShapeItem::path(kurbo::BezPath::new(), Some(Color::WHITE), None);
        let tri = |h: f64| {
            let mut p = kurbo::BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((10.0, h));
            p.line_to((20.0, 0.0));
            p.close_path();
            p
        };
        shape.geometry = xenify_doc::Geometry::Path {
            path: keyed(&[(0.0, tri(0.0), Easing::Linear), (2.0, tri(40.0), Easing::Linear)]),
        };
        let mut layer = Layer::shape(Id(1), "tri", vec![shape]);
        layer.meta.in_point = 0.5;
        let (project, comp) = project_with(layer);
        let svg =
            export_animated_svg(&project, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
        assert!(svg.contains(r#"<animate attributeName="d""#), "{svg}");
        assert!(svg.contains("visibility: hidden") && svg.contains("step-end infinite"), "{svg}");
        parses(&svg);
    }

    #[test]
    fn compress_drops_linear_frames_only() {
        let v: Vec<String> = [0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 5.0].iter().map(|x| format!("translate({x} 0)")).collect();
        assert_eq!(compress(&v, false), vec![0, 3, 5, 6]);
        let hex: Vec<String> = ["#000000", "#000000", "#ffffff", "#ffffff"].iter().map(|s| s.to_string()).collect();
        assert_eq!(compress(&hex, true), vec![0, 1, 2, 3]);
        let rgb: Vec<String> = [0, 10, 21, 30].iter().map(|c| format!("rgb({c}, 0, 0)")).collect();
        assert_eq!(compress(&rgb, true), vec![0, 3]);
    }

    #[test]
    fn box_text_exports_its_wrapped_lines_clipped_to_the_box() {
        let mut t = xenify_doc::TextLayer::new("one two three four five six", "Inter", 40.0, Color::WHITE);
        let mut f = xenify_doc::TextFrame::new(kurbo::Rect::new(0.0, 0.0, 160.0, 100.0));
        f.clip = true;
        t.frame = Some(f);
        let (project, comp) = project_with(Layer::new(Id(1), "t", xenify_doc::LayerKind::Text(t)));
        let scene = xenify_anim::evaluate_project(&project, comp, 0.0);
        let svg = export_svg(&scene, &mut xenify_render::TextEngine::new());
        assert!(svg.matches("<text").count() >= 3, "one <text> per wrapped line: {svg}");
        assert!(svg.contains("<clipPath") && svg.contains(r#"width="160""#), "{svg}");
        parses(&svg);
    }

    #[test]
    fn refs_get_short_ids() {
        let doc = format!("<mask id=\"{}\"/><g mask=\"url(#{})\"/>", id_of("m:5"), id_of("m:5"));
        assert_eq!(resolve_refs(&doc), r#"<mask id="m1"/><g mask="url(#m1)"/>"#);
    }
}
