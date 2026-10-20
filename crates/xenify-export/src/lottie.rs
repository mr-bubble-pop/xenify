//! Lottie (Bodymovin JSON) export, plus a one-file HTML player page.
//!
//! Keyframes, easing, parenting, trim paths, gradients, track mattes and
//! precomps map directly onto Lottie. Text is exported as glyph outlines (players
//! can't be relied on to have the fonts), with animated font size as a scale.

use xenify_anim::{SceneText, geometry_path};
use xenify_doc::{
    Animatable, Color, Composition, Easing, FillRule, Geometry, Gradient, GradientKind, GradientPaint, GradientStop,
    Id, Keyframe, Layer, LayerKind, LineCap, LineJoin, MatteMode, Project, Property, ShapeItem, Vec2,
};
use xenify_render::TextEngine;
use kurbo::{Affine, BezPath, PathEl, Point};
use serde_json::{Value as Json, json};
use std::collections::HashSet;

/// Precomps nested deeper than this (or cyclic) are exported empty.
const MAX_DEPTH: usize = 8;

/// The Lottie JSON for composition `comp`.
pub fn export_lottie(project: &Project, comp: Id, text: &mut TextEngine) -> Result<Json, String> {
    // Lottie has no labels: they go out as text layers.
    let mut labelled;
    let project = if project.compositions.iter().any(|c| c.layers.iter().any(|l| l.meta.label.is_some())) {
        labelled = project.clone();
        xenify_anim::diagram::labels_as_layers(&mut labelled);
        &labelled
    } else {
        project
    };
    let c = project.comp(comp).map_err(|e| e.to_string())?;
    let mut ex =
        Exporter { project, fps: c.settings.fps, text, assets: Vec::new(), done: HashSet::new(), adjust: Vec::new() };
    let layers = ex.layers(c, 0);
    let s = &c.settings;
    Ok(json!({
        "v": "5.7.0",
        "fr": s.fps,
        "ip": 0,
        "op": s.duration * s.fps,
        "w": s.width,
        "h": s.height,
        "nm": s.name,
        "ddd": 0,
        "assets": ex.assets,
        "layers": layers,
        "meta": {"g": "Xenify Vector"}
    }))
}

/// A self-contained page that plays `lottie` with lottie-web (loaded from a CDN).
pub fn export_html(lottie: &Json, title: &str, background: Color) -> String {
    let data = lottie.to_string().replace("</", "<\\/");
    let (w, h) = (lottie["w"].as_f64().unwrap_or(1920.0), lottie["h"].as_f64().unwrap_or(1080.0));
    let title = title.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  html, body {{ margin: 0; height: 100%; background: #111; }}
  #anim {{ width: min(100vw, 100vh * {ratio}); aspect-ratio: {w} / {h}; margin: auto; position: absolute; inset: 0; background: {bg}; }}
</style>
</head>
<body>
<div id="anim"></div>
<script src="https://cdn.jsdelivr.net/npm/lottie-web@5.12.2/build/player/lottie.min.js"></script>
<script>
  const data = {data};
  window.anim = lottie.loadAnimation({{ container: document.getElementById('anim'), renderer: 'svg', loop: true, autoplay: !location.hash, animationData: data }});
  // #frame=N shows a single frame (useful for previews and tests).
  const m = location.hash.match(/frame=([\d.]+)/);
  if (m) window.anim.addEventListener('DOMLoaded', () => window.anim.goToAndStop(parseFloat(m[1]), true));
</script>
</body>
</html>
"#,
        ratio = w / h,
        bg = css(background),
    )
}

fn css(c: Color) -> String {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgba({}, {}, {}, {:.3})", b(c.r), b(c.g), b(c.b), c.a)
}

struct Exporter<'a> {
    project: &'a Project,
    fps: f64,
    text: &'a mut TextEngine,
    assets: Vec<Json>,
    /// Precomp assets already exported.
    done: HashSet<Id>,
    /// Adjust Colours settings of the layer being written (and its groups),
    /// baked into its colours: Lottie players have no such effect.
    adjust: Vec<[f64; 4]>,
}

/// `c` through Adjust Colours settings, in order.
fn adjusted(adjust: &[[f64; 4]], c: Color) -> Color {
    adjust.iter().fold(c, |c, [b, k, s, h]| xenify_doc::adjust_color(c, *b, *k, *s, *h))
}

impl Exporter<'_> {
    /// Lottie layers of `comp`, top-most first.
    fn layers(&mut self, comp: &Composition, depth: usize) -> Vec<Json> {
        // Lottie can't run expressions: bake them into per-frame keys.
        let keyed = xenify_anim::expr::keyframed(comp);
        let comp = &*keyed;
        let ind = |id: Id| comp.layer_index(id).map_or(0, |i| i as u64 + 1);
        let matte_sources: HashSet<Id> = comp.layers.iter().filter_map(|l| l.meta.matte.map(|m| m.layer)).collect();
        let mut extra_ind = comp.layers.len() as u64 + 1;
        let mut bottom_up = Vec::new();
        for layer in &comp.layers {
            let Some(mut json) = self.layer(comp, layer, ind(layer.id), depth) else { continue };
            if matte_sources.contains(&layer.id) {
                // Drawn only through the layers it masks; kept (hidden) so parenting still works.
                json["hd"] = json!(true);
                bottom_up.push(json);
                continue;
            }
            // Groups/nulls draw nothing; their members carry the (inherited) matte.
            let drawable = !matches!(layer.kind, LayerKind::Null | LayerKind::Group);
            let matte = effective_matte(comp, layer).filter(|_| drawable);
            if let Some((mode, _)) = matte {
                json["tt"] = json!(match mode {
                    MatteMode::Alpha => 1,
                    MatteMode::AlphaInverted => 2,
                    MatteMode::Luma => 3,
                    MatteMode::LumaInverted => 4,
                });
            }
            bottom_up.push(json);
            // Lottie mattes are the layer directly above, flagged `td`.
            if let Some((_, source)) = matte
                && let Some(mut m) = self.layer(comp, source, extra_ind, depth)
            {
                extra_ind += 1;
                m["td"] = json!(1);
                m["hd"] = json!(false);
                m["nm"] = json!(format!("{} (matte)", source.meta.name));
                bottom_up.push(m);
            }
        }
        if comp.layers.iter().any(|l| matches!(l.kind, LayerKind::Camera(_))) {
            // Seen through a camera: every top-level layer hangs off a null
            // that moves the whole picture the way the camera sees it.
            for json in &mut bottom_up {
                if json.get("parent").is_none() {
                    json["parent"] = json!(extra_ind);
                }
            }
            bottom_up.push(self.camera_null(comp, extra_ind));
        }
        bottom_up.reverse();
        bottom_up
    }

    /// A null keyed every frame with the composition's camera view.
    fn camera_null(&self, comp: &Composition, ind: u64) -> Json {
        let s = &comp.settings;
        let frames = (s.duration * self.fps).ceil() as u64;
        let key = |i: u64, v: Vec<f64>| json!({"t": i, "s": v, "o": {"x": 0.0, "y": 0.0}, "i": {"x": 1.0, "y": 1.0}});
        let (mut p, mut r, mut sc) = (Vec::new(), Vec::new(), Vec::new());
        let mut last_turn: Option<f64> = None;
        for i in 0..=frames {
            let [a, b, c, d, e, f] = xenify_anim::camera_view(comp, i as f64 / self.fps).as_coeffs();
            let sx = a.hypot(b);
            let sy = if sx > 0.0 { (a * d - b * c) / sx } else { 0.0 };
            let mut turn = b.atan2(a).to_degrees();
            // Keep turning the same way rather than jumping at ±180°.
            if let Some(prev) = last_turn {
                turn += 360.0 * ((prev - turn) / 360.0).round();
            }
            last_turn = Some(turn);
            p.push(key(i, vec![e, f, 0.0]));
            r.push(key(i, vec![turn]));
            sc.push(key(i, vec![sx * 100.0, sy * 100.0, 100.0]));
        }
        json!({
            "ddd": 0, "ind": ind, "ty": 3, "nm": "Camera view", "sr": 1, "ao": 0,
            "ip": 0, "op": s.duration * self.fps, "st": 0, "bm": 0,
            "ks": {
                "a": {"a": 0, "k": [0, 0, 0]},
                "p": {"a": 1, "k": p},
                "r": {"a": 1, "k": r},
                "s": {"a": 1, "k": sc},
                "o": {"a": 0, "k": 100},
            },
        })
    }

    fn layer(&mut self, comp: &Composition, layer: &Layer, ind: u64, depth: usize) -> Option<Json> {
        let s = &comp.settings;
        // Its own colour adjustments, then its groups' (inner to outer).
        self.adjust.clear();
        let mut at = Some(layer);
        let mut guard = 0;
        while let Some(l) = at {
            for e in l.meta.effects.iter().filter(|e| e.enabled) {
                if let xenify_doc::Fx::AdjustColors { brightness, contrast, saturation, hue } = e.kind.at(0.0) {
                    self.adjust.push([brightness, contrast, saturation, hue]);
                }
            }
            at = l.meta.parent.and_then(|p| comp.layer(p)).filter(|p| matches!(p.kind, LayerKind::Group));
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        use xenify_doc::ModifierKind as K;
        let shape_layer = matches!(layer.kind, LayerKind::Shape { .. });
        let on = || layer.meta.modifiers.iter().filter(|m| m.enabled);
        // Blends and arrays along a path depend on other layers: written out
        // from what's drawn. Outline changes: written out from the shapes.
        // Connectors too: their line comes from the shapes they join.
        let from_scene = shape_layer
            && (layer.meta.connector.is_some()
                || on().any(|m| matches!(m.kind, K::Blend { .. } | K::PathArray { .. } | K::Shatter { .. })));
        let baked = from_scene || shape_layer && on().any(|m| !m.kind.is_array());
        let mut shift = None;
        let (ty, content) = match &layer.kind {
            // Particles: baked from what's drawn, frame by frame.
            LayerKind::Particles(_) | LayerKind::Liquid(_) => {
                let (ty, content, moved) = self.drawn_as_scene(comp, layer);
                shift = moved;
                (ty, content)
            }
            LayerKind::Shape { .. } if from_scene => {
                let (ty, content, moved) = self.drawn_as_scene(comp, layer);
                shift = moved;
                (ty, content)
            }
            LayerKind::Shape { shapes } if baked => {
                let (ty, content, moved) = self.baked(layer, shapes, s);
                shift = moved;
                (ty, content)
            }
            LayerKind::Shape { shapes } => (
                4,
                json!({"shapes": shapes
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(i, sh)| self.shape(sh, layer.id.0.wrapping_mul(1000) + i as u64))
                    .collect::<Vec<_>>()}),
            ),
            LayerKind::Text(t) if t.animators.iter().any(|a| a.enabled) => {
                let (ty, content, moved) = self.animated_text(comp, layer);
                shift = moved;
                (ty, content)
            }
            LayerKind::Text(t) if t.path.is_some() => (4, json!({"shapes": [self.text_on_path_group(comp, layer, t)]})),
            LayerKind::Text(t) => (4, json!({"shapes": [self.text_group(t)]})),
            // Lottie can't play video or sound: they keep their place (as nulls) only.
            LayerKind::Null | LayerKind::Group | LayerKind::Camera(_) | LayerKind::Video(_) | LayerKind::Audio(_) => {
                (3, json!({}))
            }
            LayerKind::Solid(solid) => {
                let animated = [&solid.size, &solid.depth, &solid.rot_x, &solid.rot_y, &solid.rot_z]
                    .iter()
                    .any(|p| p.is_animated())
                    || solid.color.is_animated();
                if !animated {
                    (4, json!({"shapes": solid_faces(solid, 0.0)}))
                } else {
                    // Which faces show changes as it turns, so each frame is its own
                    // one-frame layer inside a precomp.
                    let ref_id = format!("solid_{}", layer.id.0);
                    let frames = (s.duration * self.fps).ceil() as u64;
                    let layers: Vec<Json> = (0..frames)
                        .rev()
                        .map(|f| {
                            json!({
                                "ddd": 0, "ind": f + 1, "ty": 4, "nm": format!("frame {f}"), "sr": 1,
                                "ks": identity_ks(), "ao": 0, "ip": f, "op": f + 1, "st": 0, "bm": 0,
                                "shapes": solid_faces(solid, f as f64 / self.fps),
                            })
                        })
                        .collect();
                    self.assets.push(json!({"id": ref_id, "nm": layer.meta.name, "fr": self.fps, "layers": layers}));
                    (0, json!({"refId": ref_id, "w": s.width, "h": s.height, "st": 0}))
                }
            }
            LayerKind::Image { asset, .. } => {
                let a = self.project.asset(*asset)?;
                let ref_id = format!("image_{}", a.id.0);
                if self.done.insert(a.id) {
                    self.assets.push(json!({
                        "id": ref_id, "nm": a.name, "w": a.width, "h": a.height, "u": "", "p": a.data_uri(), "e": 1,
                    }));
                }
                (2, json!({"refId": ref_id}))
            }
            LayerKind::Precomp { comp: child, start, looped } => {
                let c = self.project.comp(*child).ok()?;
                let ref_id = format!("comp_{}", child.0);
                if depth < MAX_DEPTH && self.done.insert(*child) {
                    let layers = self.layers(c, depth + 1);
                    self.assets
                        .push(json!({"id": ref_id, "nm": c.settings.name, "fr": c.settings.fps, "layers": layers}));
                }
                let mut content =
                    json!({"refId": ref_id, "w": c.settings.width, "h": c.settings.height, "st": start * self.fps});
                if *looped && c.settings.duration > 0.0 {
                    // Time remapping (seconds of the symbol) as a sawtooth: 0 → end, again and again.
                    let (d, frame) = (c.settings.duration, 1.0 / self.fps);
                    let mut keys = Vec::new();
                    let mut k = 0.0;
                    while *start + k * d < s.duration && k < 1000.0 {
                        let t = (*start + k * d) * self.fps;
                        keys.push(
                            json!({"t": t, "s": [0.0], "o": {"x": [0.0], "y": [0.0]}, "i": {"x": [1.0], "y": [1.0]}}),
                        );
                        keys.push(json!({"t": t + (d - frame) * self.fps, "s": [d - frame], "h": 1}));
                        k += 1.0;
                    }
                    content["st"] = json!(0);
                    content["tm"] = json!({"a": 1, "k": keys});
                }
                (0, content)
            }
        };
        let t = &layer.transform;
        let mut out = json!({
            "ddd": 0,
            "ind": ind,
            "ty": ty,
            "nm": layer.meta.name,
            "sr": 1,
            "ks": {
                "a": self.prop(&t.anchor, |v| vec![v.x, v.y, 0.0]),
                "p": self.prop(&t.position, |v| vec![v.x, v.y, 0.0]),
                "s": self.prop(&t.scale, |v| vec![v.x * 100.0, v.y * 100.0, 100.0]),
                "r": self.prop(&t.rotation, |v| vec![*v]),
                "o": self.prop(&layer.opacity, |v| vec![v * 100.0]),
                // Lottie skews the other way round, about a mirrored axis.
                "sk": self.prop(&t.skew, |v| vec![-v]),
                "sa": self.prop(&t.skew_axis, |v| vec![-v]),
            },
            "ao": 0,
            "ip": layer.meta.in_point * self.fps,
            "op": layer.meta.out_point.unwrap_or(s.duration).min(s.duration) * self.fps,
            "st": 0,
            "bm": xenify_doc::BlendMode::ALL.iter().position(|b| *b == layer.meta.blend).unwrap_or(0),
        });
        for (k, v) in content.as_object().unwrap() {
            out[k] = v.clone();
        }
        if let Some(shift) = shift {
            let mut anchor = t.anchor.clone();
            map_prop(&mut anchor, |v| *v + shift);
            out["ks"]["a"] = self.prop(&anchor, |v| vec![v.x, v.y, 0.0]);
        }
        // Arrays are Lottie repeaters, each repeating the groups made so far.
        // (Only shape content can hold one: other kinds of layer lose their
        // copies, and text keeps its outline.)
        if ty == 4 && !baked {
            for m in layer.meta.modifiers.iter().filter(|m| m.enabled && m.kind.is_array()) {
                if matches!(m.kind, K::PathArray { .. } | K::Shatter { .. }) {
                    continue;
                }
                let mut items = out["shapes"].as_array().cloned().unwrap_or_default();
                items.push(self.repeater(&m.kind));
                items.push(identity_tr(None));
                out["shapes"] = json!([{"ty": "gr", "nm": m.kind.label(), "it": items}]);
            }
        }
        // Lottie has no wiggle: bake it into a keyframe per frame.
        if t.wiggle.is_some() {
            let frames = (s.duration * self.fps).ceil() as u64;
            let sample = |f: &dyn Fn(f64) -> Vec<f64>| {
                let k: Vec<Json> = (0..=frames)
                    .map(|i| {
                        json!({"t": i, "s": f(i as f64 / self.fps), "o": {"x": 0.0, "y": 0.0}, "i": {"x": 1.0, "y": 1.0}})
                    })
                    .collect();
                json!({"a": 1, "k": k})
            };
            out["ks"]["p"] = sample(&|time| {
                let p = t.position_at(time);
                vec![p.x, p.y, 0.0]
            });
            out["ks"]["r"] = sample(&|time| vec![t.rotation_at(time)]);
            out["ks"]["s"] = sample(&|time| {
                let s = t.scale_at(time);
                vec![s.x * 100.0, s.y * 100.0, 100.0]
            });
        }
        let effects = self.effects(layer);
        if !effects.is_empty() {
            out["ef"] = json!(effects);
        }
        let masks = self.masks(layer);
        if !masks.is_empty() {
            out["hasMask"] = json!(true);
            out["masksProperties"] = json!(masks);
        }
        if let Some(p) = layer.meta.parent.and_then(|p| comp.layer_index(p)) {
            out["parent"] = json!(p + 1);
        }
        if !layer.meta.visible {
            out["hd"] = json!(true);
        }
        Some(out)
    }

    /// A shape layer whose modifiers change its outline: Lottie has no such
    /// modifiers that play everywhere, so the outlines (every copy's) are
    /// written as they come out. When they change over time, each frame is
    /// its own one-frame layer inside a precomp.
    fn baked(&mut self, layer: &Layer, shapes: &[ShapeItem], s: &xenify_doc::CompSettings) -> (u8, Json, Option<Vec2>) {
        let animated = layer.property_paths().into_iter().any(|p| {
            matches!(p, xenify_doc::PropPath::Modifier { .. }) && layer.property(p).is_ok_and(|q| q.is_animated())
        }) || shapes.iter().any(|s| animated_geometry(&s.geometry));
        if !animated {
            return (4, json!({"shapes": self.baked_shapes(layer, shapes, 0.0, Vec2::ZERO)}), None);
        }
        let ref_id = format!("modified_{}", layer.id.0);
        let frames = (s.duration * self.fps).ceil() as u64;
        // Precomps are clipped to their size: move everything drawn (over all
        // frames) into it, and the layer's transform origin with it.
        let mut bounds: Option<kurbo::Rect> = None;
        for f in 0..frames {
            let time = f as f64 / self.fps;
            for plan in xenify_doc::plan(&layer.meta.modifiers, time) {
                for sh in shapes {
                    let path = xenify_anim::geometry_path(&sh.geometry, time);
                    let b = (plan.transform * xenify_anim::deform::deform_copy(&plan, &path, Affine::IDENTITY))
                        .control_box();
                    let pad = sh.stroke.as_ref().map_or(0.0, |st| st.width.value_at(time)) * 2.0 + 2.0;
                    let b = b.inflate(pad, pad);
                    bounds = Some(bounds.map_or(b, |a| a.union(b)));
                }
            }
        }
        let b = bounds.unwrap_or_default();
        let shift = Vec2::new(-b.x0.floor(), -b.y0.floor());
        let layers: Vec<Json> = (0..frames)
            .rev()
            .map(|f| {
                json!({
                    "ddd": 0, "ind": f + 1, "ty": 4, "nm": format!("frame {f}"), "sr": 1,
                    "ks": identity_ks(), "ao": 0, "ip": f, "op": f + 1, "st": 0, "bm": 0,
                    "shapes": self.baked_shapes(layer, shapes, f as f64 / self.fps, shift),
                })
            })
            .collect();
        self.assets.push(json!({"id": ref_id, "nm": layer.meta.name, "fr": self.fps, "layers": layers}));
        let (w, h) = ((b.width() + 2.0).ceil(), (b.height() + 2.0).ceil());
        (0, json!({"refId": ref_id, "w": w, "h": h, "st": 0}), Some(shift))
    }

    /// A shape layer whose drawing depends on other layers (a blend, an array
    /// along a path): everything drawn as it, taken from the evaluated
    /// composition, frame by frame when anything involved moves.
    fn drawn_as_scene(&mut self, comp: &Composition, layer: &Layer) -> (u8, Json, Option<Vec2>) {
        let mut ids: Vec<Id> = vec![layer.id];
        ids.extend(layer.meta.modifiers.iter().filter_map(|m| m.kind.target()));
        if let Some(c) = &layer.meta.connector {
            ids.extend(c.attached());
        }
        // Flowing connectors and particles move every frame on their own.
        let flowing = layer.meta.connector.as_ref().is_some_and(|c| c.flow.is_some())
            || matches!(layer.kind, LayerKind::Particles(_) | LayerKind::Liquid(_));
        let animated = flowing
            || ids.iter().filter_map(|id| comp.layer(*id)).any(|l| {
                l.transform.wiggle.is_some()
                    || l.meta.parent.is_some()
                    || l.property_paths().into_iter().any(|p| l.property(p).is_ok_and(|q| q.is_animated()))
            });
        let s = &comp.settings;
        let frames = if animated { (s.duration * self.fps).ceil() as u64 } else { 1 };
        let drawings: Vec<Vec<Json>> =
            (0..frames).map(|f| self.scene_groups(comp, layer, f as f64 / self.fps)).collect();
        if !animated {
            return (4, json!({"shapes": drawings.into_iter().next().unwrap_or_default()}), None);
        }
        let ref_id = format!("modified_{}", layer.id.0);
        // Everything drawn, over all frames, moved into the precomp's frame.
        let mut bounds: Option<kurbo::Rect> = None;
        for f in 0..frames {
            let time = f as f64 / self.fps;
            let scene = xenify_anim::evaluate_project(self.project, comp.id, time);
            let to_layer = xenify_anim::world_transform(comp, layer.id, time).inverse();
            for d in xenify_anim::drawn(&scene.layers, layer.id) {
                for item in xenify_anim::placed_items(&d, to_layer) {
                    let pad = item.stroke.as_ref().map_or(0.0, |st| st.width) + 2.0;
                    let b = kurbo::Shape::bounding_box(&item.path).inflate(pad, pad);
                    bounds = Some(bounds.map_or(b, |a| a.union(b)));
                }
            }
        }
        let b = bounds.unwrap_or_default();
        let shift = Vec2::new(-b.x0.floor(), -b.y0.floor());
        let layers: Vec<Json> = (0..frames)
            .rev()
            .map(|f| {
                let mut shapes = drawings[f as usize].clone();
                // Shift the frame's groups into the precomp.
                shapes.push(json!({"ty": "tr", "p": {"a": 0, "k": [shift.x, shift.y]}, "a": {"a": 0, "k": [0, 0]},
                    "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}}));
                json!({
                    "ddd": 0, "ind": f + 1, "ty": 4, "nm": format!("frame {f}"), "sr": 1,
                    "ks": identity_ks(), "ao": 0, "ip": f, "op": f + 1, "st": 0, "bm": 0,
                    "shapes": [{"ty": "gr", "it": shapes}],
                })
            })
            .collect();
        self.assets.push(json!({"id": ref_id, "nm": layer.meta.name, "fr": self.fps, "layers": layers}));
        let (w, h) = ((b.width() + 2.0).ceil(), (b.height() + 2.0).ceil());
        (0, json!({"refId": ref_id, "w": w, "h": h, "st": 0}), Some(shift))
    }

    /// Text with animators: Lottie can't move glyphs one by one, so every
    /// frame is its own one-frame layer of glyph outlines (each with its
    /// opacity) inside a precomp.
    fn animated_text(&mut self, comp: &Composition, layer: &Layer) -> (u8, Json, Option<Vec2>) {
        let s = &comp.settings;
        let frames = (s.duration * self.fps).ceil().max(1.0) as u64;
        let adj = self.adjust.clone();
        let glyphs: Vec<Vec<(BezPath, f64, Color)>> = (0..frames)
            .map(|f| {
                let scene = xenify_anim::evaluate_project(self.project, comp.id, f as f64 / self.fps);
                match scene.layer(layer.id).map(|l| &l.content) {
                    Some(xenify_anim::SceneContent::Text(st)) => self
                        .text
                        .glyph_outlines(st)
                        .into_iter()
                        .filter(|g| g.1 >= 0.004)
                        .map(|(p, o)| (p, o, adjusted(&adj, st.color)))
                        .collect(),
                    _ => Vec::new(),
                }
            })
            .collect();
        let b = glyphs
            .iter()
            .flatten()
            .map(|g| kurbo::Shape::bounding_box(&g.0))
            .reduce(|a, b| a.union(b))
            .unwrap_or_default()
            .inflate(2.0, 2.0);
        let shift = Vec2::new(-b.x0.floor(), -b.y0.floor());
        let ref_id = format!("text_{}", layer.id.0);
        let layers: Vec<Json> = (0..frames)
            .rev()
            .map(|f| {
                let mut shapes: Vec<Json> = glyphs[f as usize]
                    .iter()
                    .rev()
                    .map(|(path, opacity, c)| {
                        let mut it: Vec<Json> =
                            contours(path).into_iter().map(|k| json!({"ty": "sh", "ks": {"a": 0, "k": k}})).collect();
                        it.push(json!({"ty": "fl", "r": 1, "c": {"a": 0, "k": [c.r, c.g, c.b, 1.0]},
                            "o": {"a": 0, "k": c.a * opacity * 100.0}}));
                        it.push(identity_tr(None));
                        json!({"ty": "gr", "it": it})
                    })
                    .collect();
                shapes.push(json!({"ty": "tr", "p": {"a": 0, "k": [shift.x, shift.y]}, "a": {"a": 0, "k": [0, 0]},
                    "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}}));
                json!({
                    "ddd": 0, "ind": f + 1, "ty": 4, "nm": format!("frame {f}"), "sr": 1,
                    "ks": identity_ks(), "ao": 0, "ip": f, "op": f + 1, "st": 0, "bm": 0,
                    "shapes": [{"ty": "gr", "it": shapes}],
                })
            })
            .collect();
        self.assets.push(json!({"id": ref_id, "nm": layer.meta.name, "fr": self.fps, "layers": layers}));
        let (w, h) = ((b.width() + 2.0).ceil(), (b.height() + 2.0).ceil());
        (0, json!({"refId": ref_id, "w": w, "h": h, "st": 0}), Some(shift))
    }

    /// Everything drawn as `layer` at `time`, in its own space, one group per
    /// copy (top-most first).
    fn scene_groups(&mut self, comp: &Composition, layer: &Layer, time: f64) -> Vec<Json> {
        let scene = xenify_anim::evaluate_project(self.project, comp.id, time);
        let w = xenify_anim::world_transform(comp, layer.id, time);
        if w.determinant().abs() < 1e-12 {
            return Vec::new();
        }
        let own = layer.opacity.value_at(time).clamp(0.0, 1.0);
        let mut groups = Vec::new();
        for (c, d) in xenify_anim::drawn(&scene.layers, layer.id).iter().enumerate().rev() {
            let mut placed = xenify_anim::placed_items(d, w.inverse());
            // Shards (Shatter): Lottie can't mask a group, so the outlines are cut instead.
            let cuts: Vec<kurbo::BezPath> = d
                .masks
                .iter()
                .filter(|m| m.mode == xenify_doc::MaskMode::Intersect && !m.inverted)
                .map(|m| w.inverse() * m.path.clone())
                .collect();
            if !cuts.is_empty() {
                for item in &mut placed {
                    if item.fill.is_some() || item.gradient.is_some() {
                        for cut in &cuts {
                            item.path = xenify_anim::deform::intersect(&item.path, cut).unwrap_or_default();
                        }
                    }
                }
            }
            let mut items: Vec<Json> = placed
                .iter()
                .enumerate()
                .rev()
                .map(|(i, item)| {
                    let shape = xenify_anim::shape_from_item(item, "Shape");
                    self.shape(&shape, layer.id.0.wrapping_mul(1000) + i as u64)
                })
                .collect();
            let mut tr = identity_tr(None);
            let o = if own > 0.0 { (d.opacity / own).min(1.0) } else { 0.0 };
            tr["o"] = json!({"a": 0, "k": o * 100.0});
            items.push(tr);
            groups.push(json!({"ty": "gr", "nm": format!("Copy {}", c + 1), "it": items}));
        }
        groups
    }

    /// One Lottie group per copy (top-most first), outlines as they are at `time`.
    /// Everything is moved by `shift`.
    fn baked_shapes(&mut self, layer: &Layer, shapes: &[ShapeItem], time: f64, shift: Vec2) -> Vec<Json> {
        let plans = xenify_doc::plan(&layer.meta.modifiers, time);
        let mut groups = Vec::new();
        for (c, plan) in plans.iter().enumerate().rev() {
            let t = Affine::translate(shift) * plan.transform;
            let k = t.determinant().abs().sqrt();
            let mut items: Vec<Json> = shapes
                .iter()
                .enumerate()
                .rev()
                .map(|(i, s)| {
                    let path = xenify_anim::geometry_path(&s.geometry, time);
                    let path = t * xenify_anim::deform::deform_copy(plan, &path, Affine::IDENTITY);
                    let mut s = ShapeItem { geometry: Geometry::Path { path: path.into() }, ..s.clone() };
                    let place = |g: &mut xenify_doc::GradientPaint| {
                        map_prop(&mut g.start, |v| (t * v.to_point()).to_vec2());
                        map_prop(&mut g.end, |v| (t * v.to_point()).to_vec2());
                    };
                    if let Some(g) = s.fill.as_mut().and_then(|f| f.gradient.as_mut()) {
                        place(g);
                    }
                    if let Some(st) = s.stroke.as_mut() {
                        map_prop(&mut st.width, |w| w * k);
                        if let Some(g) = st.gradient.as_mut() {
                            place(g);
                        }
                    }
                    self.shape(&s, layer.id.0.wrapping_mul(1000) + i as u64)
                })
                .collect();
            let mut tr = identity_tr(None);
            tr["o"] = json!({"a": 0, "k": plan.opacity * 100.0});
            items.push(tr);
            groups.push(json!({"ty": "gr", "nm": format!("Copy {}", c + 1), "it": items}));
        }
        groups
    }

    /// A Lottie repeater for an array modifier.
    fn repeater(&self, kind: &xenify_doc::ModifierKind) -> Json {
        if let xenify_doc::ModifierKind::Mirror { origin, rotation, .. } = kind {
            // Reflecting across a line at θ is turning by −2θ, then flipping upside down.
            return json!({
                "ty": "rp", "nm": kind.label(), "m": 1,
                "c": {"a": 0, "k": 2},
                "o": {"a": 0, "k": 0},
                "tr": {
                    "ty": "tr",
                    "a": self.prop(origin, |v| vec![v.x, v.y]),
                    "p": {"a": 0, "k": [0, 0]},
                    "r": self.prop(rotation, |v| vec![-2.0 * v]),
                    "s": {"a": 0, "k": [100, -100]},
                    "so": {"a": 0, "k": 100},
                    "eo": {"a": 0, "k": 100},
                },
            });
        }
        let xenify_doc::ModifierKind::Array { count, offset, rotation, scale, origin, end_opacity } = kind else {
            return json!({});
        };
        json!({
            "ty": "rp", "nm": kind.label(), "m": 1,
            "c": self.prop(count, |c| vec![c.round().clamp(1.0, xenify_doc::MAX_COPIES as f64)]),
            "o": {"a": 0, "k": 0},
            "tr": {
                "ty": "tr",
                "a": self.prop(origin, |v| vec![v.x, v.y]),
                "p": self.prop(offset, |v| vec![v.x, v.y]),
                "r": self.prop(rotation, |v| vec![*v]),
                "s": self.prop(scale, |v| vec![v.x * 100.0, v.y * 100.0]),
                "so": {"a": 0, "k": 100},
                "eo": self.prop(end_opacity, |v| vec![v.clamp(0.0, 1.0) * 100.0]),
            },
        })
    }

    /// One shape item as a Lottie group: geometry, trim, stroke, fill, transform.
    fn shape(&mut self, shape: &ShapeItem, seed: u64) -> Json {
        let adj = self.adjust.clone();
        let mut items = Vec::new();
        // Pen styles / rounded caps: Lottie can't draw them, so a static stroke is
        // exported as its generated shapes (the same ones Xenify Vector renders).
        let styled_marks = shape.stroke.as_ref().and_then(|s| {
            let styled = !s.style.is_solid() || (s.cap == LineCap::Square && s.cap_roundness > 0.0);
            let animated = animated_geometry(&shape.geometry)
                || s.width.is_animated()
                || shape
                    .trim
                    .as_ref()
                    .is_some_and(|t| t.start.is_animated() || t.end.is_animated() || t.offset.is_animated());
            if !styled || animated {
                return None;
            }
            let mut path = geometry_path(&shape.geometry, 0.0);
            if let Some(t) = &shape.trim {
                path =
                    xenify_anim::trim_path(&path, t.start.value_at(0.0), t.end.value_at(0.0), t.offset.value_at(0.0));
            }
            let st = xenify_anim::SceneStroke::solid(s.color.value_at(0.0), s.width.value_at(0.0), s.join, s.cap);
            let marks = xenify_anim::stroke_style::marks(&path, &st, &s.style, s.cap_roundness, seed)?;
            Some((s, marks))
        });
        match &shape.geometry {
            // Trimmed rects/ellipses become paths so the trim starts where Xenify Vector's does.
            g @ (Geometry::Rect { .. } | Geometry::Ellipse { .. }) if shape.trim.is_some() && !animated_geometry(g) => {
                for c in contours(&geometry_path(g, 0.0)) {
                    items.push(json!({"ty": "sh", "ks": {"a": 0, "k": c}}));
                }
            }
            Geometry::Rect { position, size, corner_radius } => items.push(json!({
                "ty": "rc", "d": 1,
                "p": self.prop(position, |v| vec![v.x, v.y]),
                "s": self.prop(size, |v| vec![v.x, v.y]),
                "r": self.prop(corner_radius, |v| vec![*v]),
            })),
            Geometry::Ellipse { position, size } => items.push(json!({
                "ty": "el", "d": 1,
                "p": self.prop(position, |v| vec![v.x, v.y]),
                "s": self.prop(size, |v| vec![v.x, v.y]),
            })),
            Geometry::Path { path } => {
                let n = contours(&path.value_at(0.0)).len();
                for k in 0..n {
                    items.push(json!({"ty": "sh", "ks": self.path_prop(path, k)}));
                }
            }
            // Live polygons, stars and gears: their outline, keyed where they change.
            g @ Geometry::Figure { .. } => {
                if let Geometry::Path { path } = xenify_doc::primitives::to_path_geometry(g) {
                    let n = contours(&path.value_at(0.0)).len();
                    for k in 0..n {
                        items.push(json!({"ty": "sh", "ks": self.path_prop(&path, k)}));
                    }
                }
            }
        }
        if let Some(t) = &shape.trim {
            items.push(json!({
                "ty": "tm", "m": 1,
                "s": self.prop(&t.start, |v| vec![v * 100.0]),
                "e": self.prop(&t.end, |v| vec![v * 100.0]),
                "o": self.prop(&t.offset, |v| vec![v * 360.0]),
            }));
        }
        if let Some(s) = shape.stroke.as_ref().filter(|_| styled_marks.is_none()) {
            let mut st = match &s.gradient {
                // A gradient stroke ("gs") is a gradient fill with line settings.
                Some(g) => {
                    let mut gs = self.gradient_fill(g, 1);
                    gs["ty"] = json!("gs");
                    gs.as_object_mut().unwrap().remove("r");
                    gs
                }
                None => json!({
                    "ty": "st",
                    "c": self.prop(&s.color, |c| {
                    let c = adjusted(&adj, *c);
                    vec![c.r, c.g, c.b, 1.0]
                }),
                    "o": self.prop(&s.color, |c| vec![c.a * 100.0]),
                }),
            };
            st["w"] = self.prop(&s.width, |v| vec![*v]);
            st["lc"] = json!(match s.cap {
                LineCap::Butt => 1,
                LineCap::Round => 2,
                LineCap::Square => 3,
            });
            st["lj"] = json!(match s.join {
                LineJoin::Miter => 1,
                LineJoin::Round => 2,
                LineJoin::Bevel => 3,
            });
            st["ml"] = json!(4);
            if !s.dashes.is_empty() {
                // Lottie alternates named dash ("d") and gap ("g") entries, then the offset.
                let mut d: Vec<Json> = s
                    .dashes
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let n = if i % 2 == 0 { "d" } else { "g" };
                        json!({"n": n, "nm": n, "v": {"a": 0, "k": v}})
                    })
                    .collect();
                if s.dashes.len() % 2 == 1 {
                    // An odd list repeats (SVG rule), making it even.
                    let again: Vec<Json> = s
                        .dashes
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let n = if (i + s.dashes.len()) % 2 == 0 { "d" } else { "g" };
                            json!({"n": n, "nm": n, "v": {"a": 0, "k": v}})
                        })
                        .collect();
                    d.extend(again);
                }
                d.push(json!({"n": "o", "nm": "offset", "v": self.prop(&s.dash_offset, |v| vec![*v])}));
                st["d"] = json!(d);
            }
            items.push(st);
        }
        let rule = match shape.fill_rule {
            FillRule::NonZero => 1,
            FillRule::EvenOdd => 2,
        };
        if let Some(f) = &shape.fill {
            match &f.gradient {
                Some(g) => items.push(self.gradient_fill(g, rule)),
                None => items.push(json!({
                    "ty": "fl", "r": rule,
                    "c": self.prop(&f.color, |c| {
                    let c = adjusted(&adj, *c);
                    vec![c.r, c.g, c.b, 1.0]
                }),
                    "o": self.prop(&f.color, |c| vec![c.a * 100.0]),
                })),
            }
        }
        items.push(identity_tr(None));
        let body = json!({"ty": "gr", "nm": shape.name, "it": items});
        let Some((s, marks)) = styled_marks else { return body };
        // Marks first (drawn on top), each group with its own fill.
        let mut marks_it: Vec<Json> = marks
            .iter()
            .map(|m| {
                let mut it: Vec<Json> =
                    contours(&m.path).into_iter().map(|c| json!({"ty": "sh", "ks": {"a": 0, "k": c}})).collect();
                it.push(json!({
                    "ty": "fl", "r": 1,
                    "c": self.prop(&s.color, |c| {
                    let c = adjusted(&adj, *c);
                    vec![c.r, c.g, c.b, 1.0]
                }),
                    "o": self.prop(&s.color, |c| vec![c.a * m.alpha * 100.0]),
                }));
                it.push(identity_tr(None));
                json!({"ty": "gr", "it": it})
            })
            .collect();
        marks_it.push(identity_tr(None));
        json!({"ty": "gr", "nm": shape.name, "it": [
            {"ty": "gr", "nm": "Stroke", "it": marks_it},
            body,
            identity_tr(None),
        ]})
    }

    fn gradient_fill(&mut self, g: &GradientPaint, rule: u8) -> Json {
        let adj = self.adjust.clone();
        // Lottie needs the same stops in every keyframe: resample all at the union of offsets.
        let mut offsets: Vec<f64> = match &g.stops {
            Property::Static(s) => s.iter().map(|s| s.offset).collect(),
            Property::Animated(keys) => keys.iter().flat_map(|k| k.value.iter().map(|s| s.offset)).collect(),
        };
        offsets.sort_by(f64::total_cmp);
        offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let flat = |stops: &Vec<GradientStop>| {
            let g = Gradient { kind: g.kind, start: Default::default(), end: Default::default(), stops: stops.clone() };
            let colors = offsets.iter().flat_map(|&o| {
                let c = adjusted(&adj, g.color_at(o));
                [o, c.r, c.g, c.b]
            });
            let alphas = offsets.iter().flat_map(|&o| [o, g.color_at(o).a]);
            colors.chain(alphas).collect::<Vec<f64>>()
        };
        json!({
            "ty": "gf", "r": rule,
            "t": if g.kind == GradientKind::Linear { 1 } else { 2 },
            "s": self.prop(&g.start, |v| vec![v.x, v.y]),
            "e": self.prop(&g.end, |v| vec![v.x, v.y]),
            "g": {"p": offsets.len(), "k": self.prop(&g.stops, flat)},
            "o": {"a": 0, "k": 100},
            "h": {"a": 0, "k": 0},
            "a": {"a": 0, "k": 0},
        })
    }

    /// Text as a group of glyph outlines filled with the text colour.
    /// Text that follows a path, as outlines laid along it (as it is at 0 s:
    /// Lottie has no text on a path of its own).
    fn text_on_path_group(&mut self, comp: &Composition, layer: &Layer, t: &xenify_doc::TextLayer) -> Json {
        let adj = self.adjust.clone();
        let scene = xenify_anim::evaluate_project(self.project, comp.id, 0.0);
        let outline = match scene.layer(layer.id).map(|l| &l.content) {
            Some(xenify_anim::SceneContent::Text(st)) => self.text.outline(st),
            _ => kurbo::BezPath::new(),
        };
        let mut items: Vec<Json> =
            contours(&outline).into_iter().map(|c| json!({"ty": "sh", "ks": {"a": 0, "k": c}})).collect();
        items.push(json!({
            "ty": "fl", "r": 1,
            "c": self.prop(&t.color, |c| {
                let c = adjusted(&adj, *c);
                vec![c.r, c.g, c.b, 1.0]
            }),
            "o": self.prop(&t.color, |c| vec![c.a * 100.0]),
        }));
        items.push(identity_tr(None));
        json!({"ty": "gr", "nm": "Text", "it": items})
    }

    /// Shadows, glows and blur as Lottie effects (the Drop Shadow and
    /// Gaussian Blur that lottie-web draws). A glow is a shadow with no offset.
    fn effects(&self, layer: &Layer) -> Vec<Json> {
        use xenify_doc::EffectKind as K;
        let num = |nm: &str, v: Json| json!({"ty": 0, "nm": nm, "v": v});
        let mut out = Vec::new();
        for e in layer.meta.effects.iter().filter(|e| e.enabled) {
            match &e.kind {
                K::DropShadow { color, offset, blur } => {
                    out.push(self.shadow(color, Some(offset), blur, num));
                }
                K::Glow { color, radius } => out.push(self.shadow(color, None, radius, num)),
                K::Blur { radius } => out.push(json!({
                    "ty": 29, "nm": "Gaussian Blur", "en": 1,
                    "ef": [
                        // lottie-web: σ = blurriness × 0.3; ours: σ = radius / 2.
                        num("Blurriness", self.prop(radius, |r| vec![r / 2.0 / 0.3])),
                        json!({"ty": 7, "nm": "Blur Dimensions", "v": {"a": 0, "k": 1}}),
                        json!({"ty": 7, "nm": "Repeat Edge Pixels", "v": {"a": 0, "k": 0}}),
                    ],
                })),
                K::AdjustColors { .. } => {}
            }
        }
        out
    }

    fn shadow(
        &self,
        color: &Property<Color>,
        offset: Option<&Property<Vec2>>,
        blur: &Property<f64>,
        num: impl Fn(&str, Json) -> Json,
    ) -> Json {
        let zero = Property::Static(Vec2::ZERO);
        let offset = offset.unwrap_or(&zero);
        json!({
            "ty": 25, "nm": "Drop Shadow", "en": 1,
            "ef": [
                json!({"ty": 2, "nm": "Shadow Color", "v": self.prop(color, |c| vec![c.r, c.g, c.b, 1.0])}),
                num("Opacity", self.prop(color, |c| vec![c.a * 255.0])),
                // lottie-web: angle = direction − 90°, clockwise from the right.
                num("Direction", self.prop(offset, |o| vec![o.y.atan2(o.x).to_degrees() + 90.0])),
                num("Distance", self.prop(offset, |o| vec![o.hypot()])),
                // lottie-web: σ = softness / 4.
                num("Softness", self.prop(blur, |b| vec![b / 2.0 * 4.0])),
            ],
        })
    }

    fn text_group(&mut self, t: &xenify_doc::TextLayer) -> Json {
        let adj = self.adjust.clone();
        let base = t.font_size.value_at(0.0).max(0.1);
        let scene_text = SceneText {
            align: t.align,
            valign: t.valign,
            letter_spacing: t.letter_spacing,
            word_spacing: t.word_spacing,
            line_height: t.line_height,
            // Box text is outlined as wrapped at the first frame's size.
            frame: t.frame,
            ..SceneText::new(t.text.clone(), t.font_family.clone(), base, t.color.value_at(0.0))
        };
        let outline = self.text.outline(&scene_text);
        let mut items: Vec<Json> =
            contours(&outline).into_iter().map(|c| json!({"ty": "sh", "ks": {"a": 0, "k": c}})).collect();
        items.push(json!({
            "ty": "fl", "r": 1,
            "c": self.prop(&t.color, |c| {
                let c = adjusted(&adj, *c);
                vec![c.r, c.g, c.b, 1.0]
            }),
            "o": self.prop(&t.color, |c| vec![c.a * 100.0]),
        }));
        // Layout scales linearly with size around the layer origin.
        let scale = self.prop(&t.font_size, |s| vec![s / base * 100.0, s / base * 100.0]);
        items.push(identity_tr(Some(scale)));
        json!({"ty": "gr", "nm": "Text", "it": items})
    }

    /// A property as Lottie `{a, k}`; `map` gives the value's components.
    fn prop<T: Animatable>(&self, p: &Property<T>, map: impl Fn(&T) -> Vec<f64>) -> Json {
        match p {
            Property::Static(v) => {
                let v = map(v);
                json!({"a": 0, "k": if v.len() == 1 { json!(v[0]) } else { json!(v) }})
            }
            Property::Animated(keys) => {
                json!({"a": 1, "k": self.keys(keys, |v| json!(map(v)))})
            }
        }
    }

    /// The layer's masks (Lottie has no mask feather: edges stay sharp).
    fn masks(&self, layer: &Layer) -> Vec<Json> {
        use xenify_doc::MaskMode;
        let mut out = Vec::new();
        for m in layer.meta.masks.iter().filter(|m| m.enabled) {
            let mode = match m.mode {
                MaskMode::Add => "a",
                MaskMode::Subtract => "s",
                MaskMode::Intersect => "i",
                MaskMode::Difference => "f",
            };
            // One Lottie mask per contour (a Xenify Vector mask may have several).
            let n = contours(&m.path.value_at(0.0)).len().max(1);
            for k in 0..n {
                out.push(json!({
                    "nm": m.name,
                    "mode": mode,
                    "inv": m.inverted,
                    "pt": self.path_prop(&m.path, k),
                    "o": self.prop(&m.opacity, |v| vec![v * 100.0]),
                    "x": {"a": 0, "k": 0},
                }));
            }
        }
        out
    }

    /// Contour `k` of an (animated) path.
    fn path_prop(&self, p: &Property<BezPath>, k: usize) -> Json {
        let contour = |path: &BezPath| contours(path).into_iter().nth(k).unwrap_or(json!(null));
        match p {
            Property::Static(v) => json!({"a": 0, "k": contour(v)}),
            Property::Animated(keys) => json!({"a": 1, "k": self.keys(keys, |v| json!([contour(v)]))}),
        }
    }

    /// Lottie keyframes. Lottie has no step easing, so a `steps()` segment
    /// becomes a run of hold keyframes, one per step.
    fn keys<T: Animatable>(&self, keys: &[Keyframe<T>], value: impl Fn(&T) -> Json) -> Vec<Json> {
        let mut out = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let mut j = json!({"t": k.time * self.fps, "s": value(&k.value)});
            let Some(next) = keys.get(i + 1) else {
                out.push(j);
                break;
            };
            match k.easing {
                Easing::Hold => j["h"] = json!(1),
                Easing::Linear => {
                    j["o"] = json!({"x": 0.0, "y": 0.0});
                    j["i"] = json!({"x": 1.0, "y": 1.0});
                }
                Easing::Bezier { x1, y1, x2, y2 } => {
                    j["o"] = json!({"x": x1.clamp(0.0, 1.0), "y": y1});
                    j["i"] = json!({"x": x2.clamp(0.0, 1.0), "y": y2});
                }
                Easing::Steps { count, .. } => {
                    // Hold each step's value from its start; the value is sampled mid-step.
                    let n = count.max(1);
                    let span = next.time - k.time;
                    for s in 0..n {
                        let u = s as f64 / n as f64;
                        let v = T::lerp(&k.value, &next.value, k.easing.apply(u + 0.5 / n as f64));
                        out.push(json!({"t": (k.time + u * span) * self.fps, "s": value(&v), "h": 1}));
                    }
                    continue;
                }
                Easing::Spring { .. } | Easing::Stops { .. } => {
                    // No such curve in Lottie: linear pieces, a few per frame's worth.
                    let span = next.time - k.time;
                    let n = ((span * self.fps).ceil() as usize).clamp(8, 60);
                    for s in 0..n {
                        let u = s as f64 / n as f64;
                        let v = T::lerp(&k.value, &next.value, k.easing.apply(u));
                        out.push(json!({"t": (k.time + u * span) * self.fps, "s": value(&v),
                                        "o": {"x": 0.0, "y": 0.0}, "i": {"x": 1.0, "y": 1.0}}));
                    }
                    continue;
                }
            }
            out.push(j);
        }
        out
    }
}

fn animated_geometry(g: &Geometry) -> bool {
    match g {
        Geometry::Rect { position, size, corner_radius } => {
            position.is_animated() || size.is_animated() || corner_radius.is_animated()
        }
        Geometry::Ellipse { position, size } => position.is_animated() || size.is_animated(),
        Geometry::Path { path } => path.is_animated(),
        Geometry::Figure { position, size, count, inner, hole, .. } => {
            position.is_animated() || size.is_animated() || [count, inner, hole].iter().any(|p| p.is_animated())
        }
    }
}

/// Changes every value of `p` (the static one or each keyframe's).
fn map_prop<T: Clone>(p: &mut Property<T>, f: impl Fn(&T) -> T) {
    match p {
        Property::Static(v) => *v = f(v),
        Property::Animated(keys) => keys.iter_mut().for_each(|k| k.value = f(&k.value)),
    }
}

/// A group transform that does nothing (optionally with a scale property).
fn identity_tr(scale: Option<Json>) -> Json {
    json!({
        "ty": "tr",
        "p": {"a": 0, "k": [0, 0]},
        "a": {"a": 0, "k": [0, 0]},
        "s": scale.unwrap_or(json!({"a": 0, "k": [100, 100]})),
        "r": {"a": 0, "k": 0},
        "o": {"a": 0, "k": 100},
    })
}

/// The track matte for `layer`: its own, else the nearest parent's (groups).
fn effective_matte<'c>(comp: &'c Composition, layer: &'c Layer) -> Option<(MatteMode, &'c Layer)> {
    let mut current = Some(layer);
    for _ in 0..64 {
        let l = current?;
        if let Some(m) = l.meta.matte {
            return Some((m.mode, comp.layer(m.layer)?));
        }
        current = l.meta.parent.and_then(|p| comp.layer(p));
    }
    None
}

/// Splits a path into Lottie contours: `{v, i, o, c}` with tangents relative to vertices.
pub(crate) fn contours(path: &BezPath) -> Vec<Json> {
    struct C {
        v: Vec<Point>,
        i: Vec<Vec2>,
        o: Vec<Vec2>,
        closed: bool,
    }
    fn finish(c: C, out: &mut Vec<Json>) {
        let mut c = c;
        // A closing segment that returns exactly to the start duplicates the first vertex.
        if c.closed && c.v.len() > 1 && (c.v[c.v.len() - 1] - c.v[0]).hypot() < 1e-6 {
            c.v.pop();
            c.i[0] = c.i.pop().unwrap();
            c.o.pop();
        }
        if c.v.is_empty() {
            return;
        }
        let pts = |v: &[Point]| v.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>();
        let vecs = |v: &[Vec2]| v.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>();
        out.push(json!({"i": vecs(&c.i), "o": vecs(&c.o), "v": pts(&c.v), "c": c.closed}));
    }
    let mut out = Vec::new();
    let mut cur: Option<C> = None;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if let Some(c) = cur.take().filter(|c| c.v.len() > 1) {
                    finish(c, &mut out);
                }
                cur = Some(C { v: vec![p], i: vec![Vec2::ZERO], o: vec![Vec2::ZERO], closed: false });
            }
            PathEl::LineTo(p) => {
                let c = cur.get_or_insert_with(|| C { v: vec![], i: vec![], o: vec![], closed: false });
                c.v.push(p);
                c.i.push(Vec2::ZERO);
                c.o.push(Vec2::ZERO);
            }
            PathEl::QuadTo(q, p) => {
                let Some(c) = cur.as_mut() else { continue };
                let p0 = *c.v.last().unwrap();
                let (c1, c2) = (p0 + (q - p0) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0));
                *c.o.last_mut().unwrap() = c1 - p0;
                c.v.push(p);
                c.i.push(c2 - p);
                c.o.push(Vec2::ZERO);
            }
            PathEl::CurveTo(c1, c2, p) => {
                let Some(c) = cur.as_mut() else { continue };
                let p0 = *c.v.last().unwrap();
                *c.o.last_mut().unwrap() = c1 - p0;
                c.v.push(p);
                c.i.push(c2 - p);
                c.o.push(Vec2::ZERO);
            }
            PathEl::ClosePath => {
                if let Some(mut c) = cur.take() {
                    c.closed = true;
                    let start = c.v.first().copied();
                    finish(c, &mut out);
                    // Drawing may continue from the start point without a MoveTo; a lone
                    // point is replaced by the next MoveTo or dropped at the end.
                    cur = start.map(|p| C { v: vec![p], i: vec![Vec2::ZERO], o: vec![Vec2::ZERO], closed: false });
                }
            }
        }
    }
    if let Some(c) = cur.take()
        && c.v.len() > 1
    {
        finish(c, &mut out);
    }
    out
}

/// A 3D solid's shaded faces at `time` as Lottie shape groups (back to front).
fn solid_faces(solid: &xenify_doc::Solid, time: f64) -> Vec<Json> {
    xenify_anim::solid_items(solid, time)
        .iter()
        .rev()
        .map(|item| {
            let c = item.fill.unwrap_or(xenify_doc::Color::WHITE);
            let mut it: Vec<Json> =
                contours(&item.path).into_iter().map(|k| json!({"ty": "sh", "ks": {"a": 0, "k": k}})).collect();
            it.push(json!({"ty": "st", "c": {"a": 0, "k": [c.r, c.g, c.b, 1.0]}, "o": {"a": 0, "k": c.a * 100.0},
                "w": {"a": 0, "k": 0.8}, "lc": 2, "lj": 2}));
            it.push(json!({"ty": "fl", "r": if item.fill_rule == xenify_doc::FillRule::EvenOdd { 2 } else { 1 },
                "c": {"a": 0, "k": [c.r, c.g, c.b, 1.0]}, "o": {"a": 0, "k": c.a * 100.0}}));
            it.push(json!({"ty": "tr", "p": {"a": 0, "k": [0, 0]}, "a": {"a": 0, "k": [0, 0]},
                "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}}));
            json!({"ty": "gr", "it": it})
        })
        .collect()
}

fn identity_ks() -> Json {
    json!({"a": {"a": 0, "k": [0, 0, 0]}, "p": {"a": 0, "k": [0, 0, 0]}, "s": {"a": 0, "k": [100, 100, 100]},
        "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contours_convert_curves_and_drop_the_closing_duplicate() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((10.0, 0.0), (20.0, 10.0), (20.0, 20.0));
        p.line_to((0.0, 0.0));
        p.close_path();
        p.move_to((50.0, 50.0));
        p.line_to((60.0, 50.0));
        let c = contours(&p);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0]["v"], json!([[0.0, 0.0], [20.0, 20.0]]));
        assert_eq!(c[0]["o"][0], json!([10.0, 0.0]));
        assert_eq!(c[0]["i"][1], json!([0.0, -10.0]));
        assert_eq!(c[0]["c"], json!(true));
        assert_eq!(c[1]["c"], json!(false));
    }

    #[test]
    fn effects_adjusted_colours_and_text_on_a_path() {
        use xenify_doc::{Effect, EffectKind, TextLayer, TextPath};
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut red =
            Layer::shape(Id(1), "red", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::rgb(1.0, 0.0, 0.0))]);
        red.meta.effects = vec![
            Effect::new(EffectKind::DropShadow {
                color: Color::rgba(0.0, 0.0, 0.0, 0.5).into(),
                offset: Vec2::new(0.0, 10.0).into(),
                blur: 6.0.into(),
            }),
            Effect::new(EffectKind::Blur { radius: 3.0.into() }),
            // Fully desaturated: the red fill is exported grey.
            Effect::new(EffectKind::AdjustColors {
                brightness: 0.0.into(),
                contrast: 0.0.into(),
                saturation: (-1.0).into(),
                hue: 0.0.into(),
            }),
        ];
        let circle = Layer::shape(Id(2), "arc", vec![ShapeItem::ellipse(Vec2::new(300.0, 300.0), Color::WHITE)])
            .with_position(Vec2::new(500.0, 500.0));
        let mut t = TextLayer::new("Round and round", "Inter", 30.0, Color::WHITE);
        t.path = Some(TextPath { layer: Id(2), offset: 0.0, flip: false });
        let text = Layer::new(Id(3), "text", LayerKind::Text(t));
        p.comp_mut(comp).unwrap().layers = vec![red, circle, text];
        let json = export_lottie(&p, comp, &mut TextEngine::new()).unwrap();
        let layers = json["layers"].as_array().unwrap();
        let red = layers.iter().find(|l| l["nm"] == "red").unwrap();
        let ef = red["ef"].as_array().unwrap();
        assert_eq!((ef[0]["ty"].clone(), ef[1]["ty"].clone()), (json!(25), json!(29)));
        assert_eq!(ef[0]["ef"][2]["v"]["k"], json!(180.0), "straight down");
        assert_eq!(ef[0]["ef"][3]["v"]["k"], json!(10.0));
        let fill = red.to_string();
        let grey = fill.contains(r#""c":{"a":0,"k":[0.2126,0.2126,0.2126,1.0]}"#);
        assert!(grey, "red baked to grey: {fill}");
        // The text follows the circle: its outline spans the circle, not a straight line.
        let text = layers.iter().find(|l| l["nm"] == "text").unwrap();
        let shapes = text["shapes"][0]["it"].as_array().unwrap();
        let ys: Vec<f64> = shapes
            .iter()
            .filter(|s| s["ty"] == "sh")
            .flat_map(|s| {
                s["ks"]["k"]["v"].as_array().unwrap().iter().map(|v| v[1].as_f64().unwrap()).collect::<Vec<_>>()
            })
            .collect();
        let spread = ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread > 100.0, "glyphs curve around the circle: vertical spread {spread}");
    }

    #[test]
    fn arrays_become_nested_repeaters() {
        use xenify_doc::{Modifier, ModifierKind};
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(1), "dot", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        l.meta.modifiers = vec![
            Modifier::new(ModifierKind::array(3, Vec2::new(20.0, 0.0), Vec2::ZERO)),
            Modifier::new(ModifierKind::radial_array(4, Vec2::new(0.0, 50.0))),
        ];
        p.comp_mut(comp).unwrap().layers.push(l);
        let json = export_lottie(&p, comp, &mut TextEngine::new()).unwrap();
        let outer = &json["layers"][0]["shapes"][0]["it"];
        assert_eq!(outer[1]["ty"], "rp");
        assert_eq!((outer[1]["c"]["k"].clone(), outer[1]["tr"]["r"]["k"].clone()), (json!(4.0), json!(90.0)));
        assert_eq!(outer[1]["tr"]["a"]["k"], json!([0.0, 50.0]));
        let inner = &outer[0]["it"];
        assert_eq!((inner[1]["ty"].clone(), inner[1]["tr"]["p"]["k"].clone()), (json!("rp"), json!([20.0, 0.0])));
        assert_eq!(inner[0]["ty"], "gr", "the shape itself, innermost");
    }

    #[test]
    fn outline_changes_are_written_out_and_animated_ones_frame_by_frame() {
        use xenify_doc::{Modifier, ModifierKind};
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(1), "star", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE)]);
        l.meta.modifiers = vec![
            Modifier::new(ModifierKind::PuckerBloat { strength: (-0.3).into() }),
            Modifier::new(ModifierKind::array(2, Vec2::new(150.0, 0.0), Vec2::ZERO)),
        ];
        p.comp_mut(comp).unwrap().layers.push(l.clone());
        let json = export_lottie(&p, comp, &mut TextEngine::new()).unwrap();
        let groups = json["layers"][0]["shapes"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "one group per copy");
        assert!(!json.to_string().contains(r#""ty":"rp""#), "copies are written out, not repeated");

        // Jitter with a moving seed: a precomp of one-frame layers, sized to
        // what's drawn (it clips), with the layer's origin moved to match.
        l.meta.modifiers = vec![Modifier::new(ModifierKind::Jitter {
            amount: 5.0.into(),
            detail: 5.0.into(),
            seed: Property::Animated(vec![
                Keyframe { time: 0.0, value: 0.0, easing: Easing::Linear },
                Keyframe { time: 1.0, value: 3.0, easing: Easing::Linear },
            ]),
            smooth: false,
        })];
        p.comp_mut(comp).unwrap().layers = vec![l];
        let json = export_lottie(&p, comp, &mut TextEngine::new()).unwrap();
        let layer = &json["layers"][0];
        assert_eq!(layer["ty"], 0);
        let w = layer["w"].as_f64().unwrap();
        assert!((110.0..130.0).contains(&w), "{w}");
        let a = layer["ks"]["a"]["k"].as_array().unwrap();
        assert!(a[0].as_f64().unwrap() > 50.0, "the rect's middle, moved into the frame: {a:?}");
    }

    #[test]
    fn masks_become_lottie_masks() {
        use xenify_doc::{Mask, MaskMode, kurbo::Rect};
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(1), "photo", vec![ShapeItem::rect(Vec2::new(200.0, 100.0), Color::WHITE)]);
        l.meta.masks = vec![
            Mask::rect("Crop", Rect::new(-100.0, -50.0, 0.0, 50.0)),
            Mask {
                mode: MaskMode::Subtract,
                inverted: true,
                opacity: 0.5.into(),
                ..Mask::ellipse("o", Rect::new(0.0, 0.0, 10.0, 10.0))
            },
        ];
        p.comp_mut(comp).unwrap().layers.push(l);
        let json = export_lottie(&p, comp, &mut TextEngine::new()).unwrap();
        let layer = &json["layers"][0];
        assert_eq!(layer["hasMask"], true);
        let masks = layer["masksProperties"].as_array().unwrap();
        assert_eq!(masks.len(), 2);
        assert_eq!(masks[0]["mode"], "a");
        assert_eq!(masks[0]["pt"]["k"]["v"].as_array().unwrap().len(), 4, "{}", masks[0]);
        assert_eq!(
            (masks[1]["mode"].clone(), masks[1]["inv"].clone(), masks[1]["o"]["k"].clone()),
            (json!("s"), json!(true), json!(50.0))
        );
    }

    #[test]
    fn exports_keyframes_easing_mattes_and_precomps() {
        use xenify_doc::{Matte, demo_project};
        let mut p = demo_project();
        let main = p.compositions[0].id;
        // Matte the square with the ball, and nest a copy of the comp.
        let ids: Vec<Id> = p.compositions[0].layers.iter().map(|l| l.id).collect();
        p.comp_mut(main).unwrap().layers[1].meta.matte = Some(Matte { layer: ids[2], mode: MatteMode::Alpha });
        let inner = Composition { id: Id(900), ..p.compositions[0].clone() };
        let mut inner = inner;
        inner.layers.truncate(1);
        for l in &mut inner.layers {
            l.id = Id(l.id.0 + 1000);
        }
        p.compositions.push(inner.clone());
        p.comp_mut(main).unwrap().layers.push(Layer::precomp(Id(901), "Nested", &inner, Vec2::new(10.0, 10.0)));

        let j = export_lottie(&p, main, &mut TextEngine::new()).unwrap();
        let layers = j["layers"].as_array().unwrap();
        assert_eq!(layers[0]["ty"], 0, "precomp on top");
        assert_eq!(j["assets"][0]["id"], "comp_900");
        // The matte copy sits directly above the matted layer.
        let m = layers.iter().position(|l| l["td"] == 1).unwrap();
        assert_eq!(layers[m + 1]["tt"], 1);
        // Easing: the demo's rotation uses easy ease.
        let square = &layers[m + 1];
        let r = &square["ks"]["r"]["k"][0];
        assert_eq!(r["o"]["x"], json!(0.333));
        assert_eq!(r["t"], json!(0.0));
        // Text becomes outlines.
        let title = layers.iter().find(|l| l["nm"] == "Title").unwrap();
        assert!(title["shapes"][0]["it"].as_array().unwrap().len() > 3);
        assert!(export_html(&j, "Demo", Color::BLACK).contains("lottie.loadAnimation"));
    }
}
