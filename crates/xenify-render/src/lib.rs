//! Draws [`xenify_anim::Scene`]s with Vello.
//!
//! [`encode`] turns an evaluated scene into Vello draw commands; the same
//! encoding is used by the editor viewport (rendered into a texture that egui
//! displays) and by [`Offscreen`] for export, so the preview and the exported
//! video are pixel-identical.

use kurbo::{Affine, Rect};
use peniko::{BlendMode, Compose, Fill, Mix};
use xenify_anim::{Scene, SceneContent, SceneLayer, SceneStroke};
use xenify_doc::{Color, LineCap, LineJoin, MatteMode};

#[cfg(feature = "hybrid")]
pub mod hybrid;
pub mod images;
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
mod offscreen;
mod painter;
mod text;
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub use offscreen::{Offscreen, RgbaImage};
pub use painter::{Glyph, Painter};
pub use peniko;
pub use text::{DEFAULT_FAMILY, INTER_FONT, TextEngine, TextLayout};

#[cfg(feature = "gpu")]
pub use vello;

/// Texture format Vello renders into.
#[cfg(feature = "gpu")]
pub const TARGET_FORMAT: vello::wgpu::TextureFormat = vello::wgpu::TextureFormat::Rgba8Unorm;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("GPU error: {0}")]
    Gpu(String),
}

/// Appends `scene` to `out`, mapping composition space through `view`.
/// Draws the composition background as a rectangle, so whatever is outside the
/// composition (the pasteboard) stays the render target's base colour.
pub fn encode(scene: &Scene, out: &mut impl Painter, view: Affine, text: &mut TextEngine) {
    encode_seen(scene, out, view, scene.camera, text);
}

/// Like [`encode`], but seen through `camera` instead of the scene's own
/// (the editor passes the identity to show the composition itself).
pub fn encode_seen(scene: &Scene, out: &mut impl Painter, view: Affine, camera: Affine, text: &mut TextEngine) {
    let frame = Rect::new(0.0, 0.0, scene.width as f64, scene.height as f64);
    out.fill(Fill::NonZero, view, to_peniko(scene.background), None, &frame);
    // Everything is clipped to the composition frame, like in After Effects.
    out.push_clip_layer(Fill::NonZero, view, &frame);
    // Seen through the camera: layers are in composition space, and what
    // the frame shows of that space is the camera's view.
    let seen = if camera == Affine::IDENTITY { frame } else { camera.inverse().transform_rect_bbox(frame) };
    draw_layers(out, &scene.layers, view * camera, &seen, text);
    out.pop_layer();
}

/// Draws a moment of a presentation into a `width`×`height` target: each
/// slide scaled to fit (letterboxed on black), looking as its transition
/// has it (moved, clipped, in pieces, blurred, washed with a colour).
pub fn encode_deck(
    project: &xenify_doc::Project,
    frame: &xenify_anim::deck::DeckFrame,
    out: &mut impl Painter,
    width: u32,
    height: u32,
    text: &mut TextEngine,
) {
    encode_deck_on(project, frame, out, width, height, Color::BLACK, text);
}

/// [`encode_deck`] with bars of `letterbox` colour where the slide doesn't fill the target.
pub fn encode_deck_on(
    project: &xenify_doc::Project,
    frame: &xenify_anim::deck::DeckFrame,
    out: &mut impl Painter,
    width: u32,
    height: u32,
    letterbox: Color,
    text: &mut TextEngine,
) {
    let target = Rect::new(0.0, 0.0, width as f64, height as f64);
    out.fill(Fill::NonZero, Affine::IDENTITY, to_peniko(letterbox), None, &target);
    out.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &target);
    match (frame.morph, &frame.over) {
        (Some(p), Some(over)) => {
            let scene = xenify_anim::transition::morph_scene(
                project,
                (frame.under.comp, frame.under.time),
                (over.comp, over.time),
                p,
            );
            draw_shot(out, &scene, &xenify_anim::deck::Look::PLAIN, target, text);
        }
        _ => {
            let shots: Vec<&xenify_anim::deck::Shot> = if frame.swap {
                frame.over.iter().chain(std::iter::once(&frame.under)).collect()
            } else {
                std::iter::once(&frame.under).chain(frame.over.iter()).collect()
            };
            for shot in shots {
                if shot.look.opacity <= 0.0 {
                    continue;
                }
                let scene = xenify_anim::evaluate_project(project, shot.comp, shot.time);
                draw_shot(out, &scene, &shot.look, target, text);
            }
        }
    }
    out.pop_layer();
}

/// Offsets (fraction of `radius`) and weights for soft edges and blur.
const SOFT_SAMPLES: [(f64, f64, f32); 8] = [
    (0.0, 0.0, 0.2),
    (0.5, 0.0, 0.1),
    (-0.5, 0.0, 0.1),
    (0.0, 0.5, 0.1),
    (0.0, -0.5, 0.1),
    (0.7, 0.7, 0.1),
    (-0.7, -0.7, 0.15),
    (0.7, -0.7, 0.15),
];

/// One slide as its transition has it, fitted into `target`.
fn draw_shot<P: Painter>(
    out: &mut P,
    scene: &Scene,
    look: &xenify_anim::deck::Look,
    target: Rect,
    text: &mut TextEngine,
) {
    if scene.width == 0 || scene.height == 0 || look.opacity <= 0.0 {
        return;
    }
    let (sw, sh) = (scene.width as f64, scene.height as f64);
    let k = (target.width() / sw).min(target.height() / sh);
    let size = kurbo::Vec2::new(sw * k, sh * k);
    let place = Affine::translate(target.center().to_vec2() - size / 2.0) * Affine::scale(k);
    let unit = Affine::scale_non_uniform(sw, sh);
    let to_target = place * unit;
    let view_of = |m: Affine| place * unit * m * unit.inverse();
    let faded = look.opacity < 1.0;
    if faded {
        out.push_layer(Fill::NonZero, Mix::Normal, look.opacity as f32, Affine::IDENTITY, &target);
    }
    // Several weighted copies, summed: a soft clip edge or a blur.
    let spread = if look.clip.is_some() && look.pieces.is_empty() { look.feather } else { 0.0 }.max(look.blur);
    let samples: &[(f64, f64, f32)] = if spread > 1e-4 { &SOFT_SAMPLES } else { &[(0.0, 0.0, 1.0)] };
    if samples.len() > 1 {
        out.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &target);
    }
    for &(dx, dy, weight) in samples {
        let shift = kurbo::Vec2::new(dx, dy) * spread * size.x;
        if samples.len() > 1 {
            out.push_layer(
                Fill::NonZero,
                BlendMode::new(Mix::Normal, Compose::Plus),
                weight,
                Affine::IDENTITY,
                &target,
            );
        }
        // A soft edge moves the clip; a blur moves the picture.
        let (clip_shift, picture_shift) =
            if look.blur > 0.0 { (kurbo::Vec2::ZERO, shift) } else { (shift, kurbo::Vec2::ZERO) };
        if look.pieces.is_empty() {
            if let Some(clip) = &look.clip {
                out.push_clip_layer(Fill::NonZero, Affine::translate(clip_shift) * to_target, clip);
            }
            encode(scene, out, Affine::translate(picture_shift) * view_of(look.transform), text);
            if look.clip.is_some() {
                out.pop_layer();
            }
        } else {
            for piece in &look.pieces {
                if piece.opacity <= 0.0 {
                    continue;
                }
                out.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    piece.opacity as f32,
                    to_target * piece.transform,
                    &piece.clip,
                );
                encode(scene, out, view_of(piece.transform), text);
                out.pop_layer();
            }
        }
        if samples.len() > 1 {
            out.pop_layer();
        }
    }
    if samples.len() > 1 {
        out.pop_layer();
    }
    if let Some((c, strength)) = look.wash {
        let c = Color { a: c.a * strength.clamp(0.0, 1.0), ..c };
        out.fill(Fill::NonZero, to_target * look.transform, to_peniko(c), None, &Rect::new(0.0, 0.0, 1.0, 1.0));
    }
    for &(centre, radius, alpha) in &look.specks {
        let c = Color { a: look.speck_color.a * alpha, ..look.speck_color };
        let at = to_target * centre;
        out.fill(Fill::NonZero, Affine::IDENTITY, to_peniko(c), None, &kurbo::Circle::new(at, radius * size.y));
    }
    if faded {
        out.pop_layer();
    }
}

/// Draws layers in paint order, applying their track mattes.
fn draw_layers<P: Painter>(out: &mut P, layers: &[SceneLayer], view: Affine, frame: &Rect, text: &mut TextEngine) {
    for layer in layers {
        if layer.opacity <= 0.0 {
            continue;
        }
        if layer.masks.is_empty() {
            draw_matted(out, layer, view, frame, text);
        } else {
            // Draw it on its own, then keep only what the masks let through.
            out.push_layer(Fill::NonZero, Mix::Normal, 1.0, view, frame);
            draw_matted(out, layer, view, frame, text);
            apply_masks(out, &layer.masks, view, frame);
            out.pop_layer();
        }
    }
}

/// Cuts what's been drawn in the current layer down to `masks` (combined in
/// order like After Effects: add, subtract, intersect, difference).
fn apply_masks<P: Painter>(out: &mut P, masks: &[xenify_anim::SceneMask], view: Affine, frame: &Rect) {
    use xenify_doc::MaskMode;
    out.push_layer(Fill::NonZero, BlendMode::new(Mix::Normal, Compose::DestIn), 1.0, view, frame);
    // Masks that take away start from everything.
    if masks.first().is_some_and(|m| matches!(m.mode, MaskMode::Subtract | MaskMode::Intersect)) {
        out.fill(Fill::NonZero, view, peniko::Color::WHITE, None, frame);
    }
    let scale = view.as_coeffs()[0].hypot(view.as_coeffs()[1]);
    // Far beyond anything visible: what an inverted mask covers.
    let everything =
        frame.inflate(frame.width().max(frame.height()) * 4.0 + 1e4, frame.width().max(frame.height()) * 4.0 + 1e4);
    for m in masks {
        let compose = match m.mode {
            MaskMode::Add => Compose::SrcOver,
            MaskMode::Subtract => Compose::DestOut,
            MaskMode::Intersect => Compose::DestIn,
            MaskMode::Difference => Compose::Xor,
        };
        out.push_layer(Fill::NonZero, BlendMode::new(Mix::Normal, compose), 1.0, view, &everything);
        let brush = peniko::Color::new([1.0, 1.0, 1.0, m.opacity as f32]);
        let (shape, rule) = if m.inverted {
            let mut p = kurbo::Shape::to_path(&everything, 0.1);
            p.extend(m.path.iter());
            (p, Fill::EvenOdd)
        } else {
            (m.path.clone(), Fill::NonZero)
        };
        blurred(out, &everything, view, m.feather, scale, |out, v| out.fill(rule, v, brush, None, &shape));
        out.pop_layer();
    }
    out.pop_layer();
}

/// Draws one layer with its track matte, if it has one.
fn draw_matted<P: Painter>(out: &mut P, layer: &SceneLayer, view: Affine, frame: &Rect, text: &mut TextEngine) {
    let Some(matte) = &layer.matte else {
        draw_layer(out, layer, view, frame, text);
        return;
    };
    // Isolate the layer, then cut it with the matte.
    out.push_layer(Fill::NonZero, Mix::Normal, 1.0, view, frame);
    draw_layer(out, layer, view, frame, text);
    match matte.mode {
        MatteMode::Alpha | MatteMode::AlphaInverted => {
            let compose = if matte.mode == MatteMode::Alpha { Compose::DestIn } else { Compose::DestOut };
            out.push_layer(Fill::NonZero, BlendMode::new(Mix::Normal, compose), 1.0, view, frame);
            if let Some(source) = &matte.source {
                draw_layer(out, source, view, frame, text);
            }
            out.pop_layer();
        }
        MatteMode::Luma | MatteMode::LumaInverted => {
            out.push_luminance_mask_layer(Fill::NonZero, 1.0, view, frame);
            if matte.mode == MatteMode::LumaInverted {
                // 1 − luma: white, minus the matte via a difference blend.
                out.fill(Fill::NonZero, view, peniko::Color::WHITE, None, frame);
                out.push_layer(Fill::NonZero, Mix::Difference, 1.0, view, frame);
            }
            if let Some(source) = &matte.source {
                draw_layer(out, source, view, frame, text);
            }
            if matte.mode == MatteMode::LumaInverted {
                out.pop_layer();
            }
            out.pop_layer();
        }
    }
    out.pop_layer();
}

/// Draws one layer with its opacity, blend mode and effects (ignoring its matte).
fn draw_layer<P: Painter>(out: &mut P, layer: &SceneLayer, view: Affine, frame: &Rect, text: &mut TextEngine) {
    if layer.opacity <= 0.0 {
        return;
    }
    let blend = !layer.blend.is_normal();
    let group = layer.opacity < 1.0 || blend;
    if group {
        let mode = match layer.blend {
            xenify_doc::BlendMode::Add => BlendMode::new(Mix::Normal, Compose::Plus),
            b => BlendMode::new(to_mix(b), Compose::SrcOver),
        };
        out.push_layer(Fill::NonZero, mode, layer.opacity as f32, view, frame);
    }
    // Shadows and glows go behind the layer.
    let scale = view.as_coeffs()[0].hypot(view.as_coeffs()[1]);
    for e in &layer.effects {
        let (color, offset, radius) = match *e {
            xenify_doc::Fx::DropShadow { color, offset, blur } => (color, offset, blur),
            xenify_doc::Fx::Glow { color, radius } => (color, kurbo::Vec2::ZERO, radius),
            _ => continue,
        };
        if color.a <= 0.0 {
            continue;
        }
        let area = blur_area(layer, frame, radius, offset, text);
        out.push_layer(Fill::NonZero, Mix::Normal, color.a as f32, view, &area);
        let solid = Color { a: 1.0, ..color };
        blurred(out, &area, view, radius, scale, |out, v| {
            silhouette(out, layer, v * Affine::translate(offset), solid, text);
        });
        out.pop_layer();
    }
    let blur = layer
        .effects
        .iter()
        .filter_map(|e| match e {
            xenify_doc::Fx::Blur { radius } => Some(*radius),
            _ => None,
        })
        .fold(0.0, f64::max);
    let area = if blur > 0.0 { blur_area(layer, frame, blur, kurbo::Vec2::ZERO, text) } else { *frame };
    blurred(out, &area, view, blur, scale, |out, v| draw_content(out, layer, v, frame, text));
    if group {
        out.pop_layer();
    }
}

/// Where a blur of the layer (moved by `offset`) can reach, within the frame.
/// Blurs push a layer per copy, so clipping them to this rather than the
/// whole frame keeps a canvas with many shadows within the GPU's buffers
/// (past those it drew a black frame).
fn blur_area(layer: &SceneLayer, frame: &Rect, radius: f64, offset: kurbo::Vec2, text: &mut TextEngine) -> Rect {
    let Some(b) = layer.bounds(text) else { return *frame };
    let b = layer.transform.transform_rect_bbox(b) + offset;
    let a = b.inflate(radius * 1.3 + 2.0, radius * 1.3 + 2.0).intersect(*frame);
    if a.width() > 0.0 && a.height() > 0.0 { a } else { Rect::ZERO }
}

/// Draws `draw` blurred by `radius` (composition pixels): copies at offsets
/// spread like a Gaussian, added together at equal weights.
fn blurred<P: Painter>(
    out: &mut P,
    frame: &Rect,
    view: Affine,
    radius: f64,
    scale: f64,
    mut draw: impl FnMut(&mut P, Affine),
) {
    // Below half a screen pixel a blur can't be seen.
    if radius * scale < 0.5 {
        draw(out, view);
        return;
    }
    let samples = blur_samples(radius, radius * scale);
    let add = BlendMode::new(Mix::Normal, Compose::Plus);
    out.push_layer(Fill::NonZero, Mix::Normal, 1.0, view, frame);
    for (off, w) in samples {
        out.push_layer(Fill::NonZero, add, w, view, frame);
        draw(out, view * Affine::translate(off));
        out.pop_layer();
    }
    out.pop_layer();
}

/// Offsets and weights for a blur of `radius` (σ = radius / 2): copies
/// spread evenly over a disc (a golden-angle spiral) and weighted by the
/// Gaussian, so the outermost ones fade out instead of showing as lumps.
/// Wider blurs on screen get more copies. The weights add up to 1.
pub fn blur_samples(radius: f64, on_screen: f64) -> Vec<(kurbo::Vec2, f32)> {
    let n = (12.0 + on_screen * 2.5).clamp(16.0, 96.0) as usize;
    let sigma = radius / 2.0;
    let reach = 2.5 * sigma;
    let raw: Vec<(kurbo::Vec2, f64)> = (0..n)
        .map(|i| {
            let r = reach * ((i as f64 + 0.5) / n as f64).sqrt();
            let a = i as f64 * 2.399_963_229_728_653;
            (kurbo::Vec2::new(r * a.cos(), r * a.sin()), (-r * r / (2.0 * sigma * sigma)).exp())
        })
        .collect();
    let total: f64 = raw.iter().map(|s| s.1).sum();
    raw.into_iter().map(|(o, w)| (o, (w / total) as f32)).collect()
}

/// The layer's shape filled with one colour (for shadows and glows).
fn silhouette<P: Painter>(out: &mut P, layer: &SceneLayer, view: Affine, color: Color, text: &mut TextEngine) {
    let transform = view * layer.transform;
    let brush = to_peniko(color);
    match &layer.content {
        SceneContent::Shapes(items) => {
            for item in items {
                if item.fill.is_some() || item.gradient.is_some() {
                    let rule = match item.fill_rule {
                        xenify_doc::FillRule::NonZero => Fill::NonZero,
                        xenify_doc::FillRule::EvenOdd => Fill::EvenOdd,
                    };
                    out.fill(rule, transform, brush, None, &item.path);
                }
                if let Some(stroke) = item.stroke.as_ref().filter(|s| s.width > 0.0) {
                    match &stroke.marks {
                        Some(marks) => {
                            for m in marks {
                                out.fill(Fill::NonZero, transform, brush, None, &m.path);
                            }
                        }
                        None => out.stroke(&to_kurbo_stroke(stroke), transform, brush, None, &item.path),
                    }
                }
            }
        }
        SceneContent::Text(t) => {
            let t = xenify_anim::SceneText { color, ..t.clone() };
            match layer.distort {
                Some(h) => out.fill(Fill::NonZero, transform, brush, None, &h.map_path(&text.outline(&t))),
                None => text.draw(out, &t, transform),
            }
        }
        SceneContent::Image(img) => {
            let r = kurbo::Shape::to_path(&img.rect(), 0.1);
            let r = layer.distort.map_or(r.clone(), |h| h.map_path(&r));
            out.fill(Fill::NonZero, transform, brush, None, &r);
        }
        SceneContent::Precomp { layers, .. } | SceneContent::Group(layers) => {
            for l in layers {
                silhouette(out, l, view, color, text);
            }
        }
    }
}

/// The layer's own content.
fn draw_content<P: Painter>(out: &mut P, layer: &SceneLayer, view: Affine, frame: &Rect, text: &mut TextEngine) {
    let transform = view * layer.transform;
    match &layer.content {
        SceneContent::Shapes(items) => {
            for item in items {
                if let Some(fill) = item.fill {
                    let rule = match item.fill_rule {
                        xenify_doc::FillRule::NonZero => Fill::NonZero,
                        xenify_doc::FillRule::EvenOdd => Fill::EvenOdd,
                    };
                    let tiled = item.pattern.as_ref().and_then(|p| Some((images::decoded(&p.image)?, p)));
                    match (&item.gradient, tiled) {
                        (_, Some((data, p))) => {
                            // Stretch the decoded pixels to the recorded size, then place the tiles.
                            let fit = Affine::scale_non_uniform(
                                p.image.width as f64 / data.width.max(1) as f64,
                                p.image.height as f64 / data.height.max(1) as f64,
                            );
                            let brush = peniko::ImageBrush::new(data).with_extend(peniko::Extend::Repeat);
                            out.fill(rule, transform, &brush, Some(p.transform * fit), &item.path)
                        }
                        (Some(g), None) => out.fill(rule, transform, &to_peniko_gradient(g), None, &item.path),
                        (None, None) => out.fill(rule, transform, to_peniko(fill), None, &item.path),
                    }
                }
                if let Some(stroke) = &item.stroke
                    && stroke.width > 0.0
                {
                    match &stroke.marks {
                        Some(marks) => {
                            for m in marks {
                                let c = Color { a: stroke.color.a * m.alpha, ..stroke.color };
                                out.fill(Fill::NonZero, transform, to_peniko(c), None, &m.path);
                            }
                        }
                        None => match &stroke.gradient {
                            Some(g) => out.stroke(
                                &to_kurbo_stroke(stroke),
                                transform,
                                &to_peniko_gradient(g),
                                None,
                                &item.path,
                            ),
                            None => out.stroke(
                                &to_kurbo_stroke(stroke),
                                transform,
                                to_peniko(stroke.color),
                                None,
                                &item.path,
                            ),
                        },
                    }
                }
            }
        }
        SceneContent::Text(t) => match layer.distort {
            // Perspective text is drawn as mapped glyph outlines.
            Some(h) => {
                let path = h.map_path(&text.outline(t));
                out.fill(Fill::NonZero, transform, to_peniko(t.color), None, &path);
            }
            None => text.draw(out, t, transform),
        },
        SceneContent::Image(img) => {
            let clip = layer.distort.map(|h| h.map_path(&kurbo::Shape::to_path(&img.rect(), 0.1)));
            images::draw(out, img, transform, clip.as_ref());
        }
        SceneContent::Precomp { width, height, layers } => {
            let own = Rect::new(0.0, 0.0, *width, *height);
            let clip = layer.distort.unwrap_or_default().map_path(&kurbo::Shape::to_path(&own, 0.1));
            out.push_clip_layer(Fill::NonZero, transform, &clip);
            draw_layers(out, layers, view, frame, text);
            out.pop_layer();
        }
        SceneContent::Group(layers) => draw_layers(out, layers, view, frame, text),
    }
}

fn to_mix(b: xenify_doc::BlendMode) -> Mix {
    use xenify_doc::BlendMode as B;
    match b {
        B::Normal => Mix::Normal,
        B::Multiply => Mix::Multiply,
        B::Screen => Mix::Screen,
        B::Overlay => Mix::Overlay,
        B::Darken => Mix::Darken,
        B::Lighten => Mix::Lighten,
        B::ColorDodge => Mix::ColorDodge,
        B::ColorBurn => Mix::ColorBurn,
        B::HardLight => Mix::HardLight,
        B::SoftLight => Mix::SoftLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
        B::Add => Mix::Normal, // composed with Plus instead
    }
}

pub fn to_peniko(c: Color) -> peniko::Color {
    peniko::Color::new([c.r as f32, c.g as f32, c.b as f32, c.a as f32])
}

pub fn to_peniko_gradient(g: &xenify_doc::Gradient) -> peniko::Gradient {
    let stops: Vec<(f32, peniko::Color)> =
        g.sorted_stops().iter().map(|s| (s.offset as f32, to_peniko(s.color))).collect();
    let base = match g.kind {
        xenify_doc::GradientKind::Linear => peniko::Gradient::new_linear(g.start.to_point(), g.end.to_point()),
        xenify_doc::GradientKind::Radial => {
            peniko::Gradient::new_radial(g.start.to_point(), (g.end - g.start).hypot().max(0.01) as f32)
        }
    };
    base.with_stops(stops.as_slice())
}

fn to_kurbo_stroke(s: &SceneStroke) -> kurbo::Stroke {
    let join = match s.join {
        LineJoin::Miter => kurbo::Join::Miter,
        LineJoin::Round => kurbo::Join::Round,
        LineJoin::Bevel => kurbo::Join::Bevel,
    };
    let cap = match s.cap {
        LineCap::Butt => kurbo::Cap::Butt,
        LineCap::Round => kurbo::Cap::Round,
        LineCap::Square => kurbo::Cap::Square,
    };
    let st = kurbo::Stroke::new(s.width).with_join(join).with_caps(cap);
    if s.dashes.is_empty() { st } else { st.with_dashes(s.dash_offset, s.dashes.iter().copied()) }
}

#[cfg(feature = "gpu")]
/// Creates a texture Vello can render into and egui/wgpu can sample or copy from.
pub fn create_target(device: &vello::wgpu::Device, width: u32, height: u32) -> vello::wgpu::Texture {
    use vello::wgpu;
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("xenify render target"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[cfg(feature = "gpu")]
/// Vello renderer options used everywhere (area AA only: fastest to compile
/// and good quality for motion graphics).
pub fn renderer_options() -> vello::RendererOptions {
    vello::RendererOptions { antialiasing_support: vello::AaSupport::area_only(), ..Default::default() }
}

#[cfg(feature = "gpu")]
pub fn render_params(width: u32, height: u32, base: Color) -> vello::RenderParams {
    vello::RenderParams { base_color: to_peniko(base), width, height, antialiasing_method: vello::AaConfig::Area }
}
