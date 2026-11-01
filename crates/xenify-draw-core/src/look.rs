//! How layers look: the whole layer as JSON for the Layer panel (fill and
//! stroke settings, effects, masks, modifiers, blend mode, mask layer),
//! every animatable property by its path, and the calls that add effects,
//! modifiers, masks, gradients and patterns.

use kurbo::{Rect, Vec2};
use serde_json::{Value as Json, json};
use xenify_doc::{
    Color, Effect, EffectKind, GradientKind, GradientPaint, GradientStop, Id, Layer, LayerKind, LayerMeta, Mask, Matte,
    MatteMode, PenKind, PenStyle, ShapeItem,
};
use xenify_ops::Command;
use xenify_ops::modifiers::Preset;

use crate::{Session, err, f, id_arg, value_json};

const PRESETS: [Preset; 17] = [
    Preset::Row,
    Preset::Radial,
    Preset::Grid,
    Preset::Mirror,
    Preset::PathArray,
    Preset::Jitter,
    Preset::ZigZag,
    Preset::Wave,
    Preset::PuckerBloat,
    Preset::Twist,
    Preset::Smooth,
    Preset::Subdivide,
    Preset::Simplify,
    Preset::OffsetPath,
    Preset::Blend,
    Preset::ShatterOut,
    Preset::ShatterIn,
];

fn effect_kind(name: &str) -> Option<EffectKind> {
    Some(match name {
        "drop_shadow" => EffectKind::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.45).into(),
            offset: Vec2::new(8.0, 10.0).into(),
            blur: 12.0.into(),
        },
        "glow" => {
            EffectKind::Glow { color: Color::from_hex("#FDE68A").unwrap_or(Color::WHITE).into(), radius: 18.0.into() }
        }
        "blur" => EffectKind::Blur { radius: 6.0.into() },
        "adjust_colors" => EffectKind::AdjustColors {
            brightness: 0.0.into(),
            contrast: 0.0.into(),
            saturation: 0.0.into(),
            hue: 0.0.into(),
        },
        _ => return None,
    })
}

impl Session {
    pub(crate) fn look_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "layer_full" => id_arg(args, "id").and_then(|id| self.layer_full(id)),
            "set_meta_json" => self.set_meta_json(args),
            "set_shape_json" => self.set_shape_json(args),
            "add_effect" => self.add_effect(args),
            "modifier_presets" => Ok(json!(PRESETS
                .iter()
                .map(|p| json!({ "key": format!("{p:?}"), "label": p.label(), "tip": p.tip(), "pair": p.needs_target() }))
                .collect::<Vec<_>>())),
            "add_modifier" => self.add_modifier(args),
            "apply_modifier" => self.apply_modifier(args),
            "add_mask" => self.add_mask(args.get("ellipse").and_then(Json::as_bool).unwrap_or(false)),
            "shape_to_mask" => self.shape_to_mask(),
            "set_matte" => self.set_matte(args),
            "set_gradient" => self.set_gradient(args),
            "gradient_op" => self.gradient_op(args),
            "pens" => Ok(json!(PenKind::ALL.iter().map(|k| json!({ "kind": k, "label": k.label() })).collect::<Vec<_>>())),
            "pen_preset" => serde_json::from_value::<PenKind>(args.get("kind").cloned().unwrap_or(json!("solid")))
                .map_err(err)
                .and_then(|k| serde_json::to_value(PenStyle::preset(k)).map_err(err)),
            "add_shape_item" => self.add_shape_item(args),
            "set_pen" => self.set_pen(args),
            "set_pattern" => self.set_pattern(args),
            // The pictures this canvas already has (a pattern can reuse one).
            "pictures" => Ok(json!(self.project().assets.iter().map(|a| json!({ "id": a.id.0, "name": a.name, "width": a.width, "height": a.height })).collect::<Vec<_>>())),
            "gradient_drag" => self.gradient_drag(args),
            // Where the selected layer's gradient starts and ends, on the canvas (to drag them).
            "gradient_ends" => Ok((|| {
                let stroke = args.get("stroke").and_then(Json::as_bool).unwrap_or(false);
                let id = *self.selection.first()?;
                let l = self.project().layer(id).ok()?;
                let c = self.project().comp(self.comp).ok()?;
                let m = xenify_anim::world_transform(c, id, self.time);
                let g = l.shapes().iter().find_map(|s| if stroke { s.stroke.as_ref()?.gradient.clone() } else { s.fill.as_ref()?.gradient.clone() })?;
                let (a, b) = (m * g.start.value_at(self.time).to_point(), m * g.end.value_at(self.time).to_point());
                Some(json!({ "a": [a.x, a.y], "b": [b.x, b.y], "radial": matches!(g.kind, GradientKind::Radial) }))
            })().unwrap_or(Json::Null)),
            "effect_gallery" => Ok(json!(xenify_ops::effects::CATALOG
                .iter()
                .map(|(section, names)| json!({
                    "section": section,
                    "effects": names.iter().map(|n| json!({ "name": n, "tip": xenify_ops::effects::tip(n), "needs_layer": xenify_ops::effects::needs_target(n) })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>())),
            "add_gallery_effect" => self.add_gallery_effect(args),
            _ => return None,
        })
    }

    /// A layer as stored (its settings, shapes, effects, masks…), every
    /// property now (by path: value, animated, keyframe times), its box in
    /// its own space and the other layers it could use as a mask.
    fn layer_full(&mut self, id: Id) -> Result<Json, String> {
        let l = self.project().layer(id).map_err(err)?.clone();
        let props: Vec<Json> = l
            .property_paths()
            .into_iter()
            .filter_map(|path| {
                let prop = l.property(path).ok()?;
                let v = l.value_at(path, self.time).ok()?;
                let value = match &v {
                    xenify_doc::Value::Stops(s) => serde_json::to_value(s).ok()?,
                    other => value_json(other),
                };
                if value.is_null() {
                    return None;
                }
                Some(json!({ "path": path.to_string(), "value": value, "animated": prop.is_animated(), "keys": prop.keyframe_times() }))
            })
            .collect();
        let others: Vec<Json> = self
            .project()
            .comp(self.comp)
            .map(|c| {
                c.layers.iter().filter(|o| o.id != id).map(|o| json!({ "id": o.id.0, "name": o.meta.name })).collect()
            })
            .unwrap_or_default();
        let bounds = self.local_box(id).map(|b| json!([b.x0, b.y0, b.x1, b.y1]));
        Ok(json!({ "layer": serde_json::to_value(&l).map_err(err)?, "props": props, "others": others, "box": bounds }))
    }

    /// The layer's content box in its own space (a group's around its members).
    fn local_box(&mut self, id: Id) -> Option<Rect> {
        let scene = self.scene();
        if let Some(b) = scene.layer(id).and_then(|sl| sl.bounds(&mut self.text)) {
            return Some(b);
        }
        let c = self.project().comp(self.comp).ok()?.clone();
        let world = xenify_anim::world_box(&c, &scene, id, &mut self.text)?;
        let inv = xenify_anim::world_transform(&c, id, self.time).inverse();
        let pts = [(world.x0, world.y0), (world.x1, world.y0), (world.x1, world.y1), (world.x0, world.y1)]
            .map(|p| inv * kurbo::Point::from(p));
        Some(pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p)))
    }

    fn set_meta_json(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let meta: LayerMeta = serde_json::from_value(args.get("meta").cloned().ok_or("missing meta")?).map_err(err)?;
        let label = args.get("label").and_then(Json::as_str).unwrap_or("Change layer").to_string();
        // Typing in a field or dragging merges into one step.
        match args.get("merge").and_then(Json::as_str) {
            Some(k) => self
                .editor
                .execute_merged(label, format!("meta:{}:{k}", id.0), Command::SetLayerMeta { layer: id, meta })
                .map_err(err)?,
            None => self.exec(&label, Command::SetLayerMeta { layer: id, meta })?,
        }
        Ok(Json::Null)
    }

    fn set_shape_json(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        let shape: ShapeItem =
            serde_json::from_value(args.get("shape").cloned().ok_or("missing shape")?).map_err(err)?;
        let label = args.get("label").and_then(Json::as_str).unwrap_or("Change shape").to_string();
        match args.get("merge").and_then(Json::as_str) {
            Some(k) => self
                .editor
                .execute_merged(
                    label,
                    format!("shape:{}:{index}:{k}", id.0),
                    Command::SetShape { layer: id, index, shape },
                )
                .map_err(err)?,
            None => self.exec(&label, Command::SetShape { layer: id, index, shape })?,
        }
        Ok(Json::Null)
    }

    /// The selected layers (or `ids`), each with its meta, for a change to all.
    fn targets(&self, args: &Json) -> Vec<(Id, LayerMeta)> {
        let ids: Vec<Id> = match args.get("ids").and_then(Json::as_array) {
            Some(a) => a.iter().filter_map(Json::as_u64).map(Id).collect(),
            None => self.selection.clone(),
        };
        ids.into_iter().filter_map(|id| Some((id, self.project().layer(id).ok()?.meta.clone()))).collect()
    }

    fn add_effect(&mut self, args: &Json) -> Result<Json, String> {
        let name = args.get("kind").and_then(Json::as_str).unwrap_or("");
        let kind = effect_kind(name).ok_or_else(|| format!("no effect {name}"))?;
        let cmds: Vec<Command> = self
            .targets(args)
            .into_iter()
            .map(|(id, mut meta)| {
                meta.effects.push(Effect::new(kind.clone()));
                Command::SetLayerMeta { layer: id, meta }
            })
            .collect();
        if cmds.is_empty() {
            return Err("Select the layers to add an effect to.".into());
        }
        self.exec_all(&format!("Add {}", kind.label().to_lowercase()), cmds)?;
        Ok(Json::Null)
    }

    /// A modifier preset on the selected layers; the pair ones (blend, array
    /// along a path) take the first selected layer and use the second.
    fn add_modifier(&mut self, args: &Json) -> Result<Json, String> {
        let key = args.get("preset").and_then(Json::as_str).unwrap_or("");
        let preset = *PRESETS.iter().find(|p| format!("{p:?}") == key).ok_or_else(|| format!("no modifier {key}"))?;
        let sel = self.selection.clone();
        let (layers, target) = if preset.needs_target() {
            match sel.as_slice() {
                [a, b, ..] => (vec![*a], Some(*b)),
                _ => {
                    return Err(format!(
                        "{} needs two layers: select this one, then the other (Shift+click).",
                        preset.label()
                    ));
                }
            }
        } else {
            (sel, None)
        };
        let mut cmds = Vec::new();
        for id in layers {
            let Ok(mut meta) = self.project().layer(id).map(|l| l.meta.clone()) else { continue };
            let b = self.local_box(id).unwrap_or(Rect::new(0.0, 0.0, 100.0, 100.0));
            meta.modifiers.extend(preset.modifiers(b, target, self.time));
            cmds.push(Command::SetLayerMeta { layer: id, meta });
        }
        if cmds.is_empty() {
            return Err("Select the layers to add it to.".into());
        }
        self.exec_all(preset.label(), cmds)?;
        Ok(Json::Null)
    }

    fn apply_modifier(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let cmd = xenify_ops::modifiers::apply_first(self.project(), self.comp, id, self.time).map_err(err)?;
        self.exec("Apply modifier", cmd)?;
        Ok(Json::Null)
    }

    fn add_mask(&mut self, ellipse: bool) -> Result<Json, String> {
        let mut cmds = Vec::new();
        for (id, mut meta) in self.targets(&Json::Null) {
            if self.project().layer(id).is_ok_and(|l| matches!(l.kind, LayerKind::Null)) {
                continue;
            }
            let Some(b) = self.local_box(id) else { continue };
            let n = meta.masks.len() + 1;
            meta.masks.push(if ellipse {
                Mask::ellipse(format!("Mask {n}"), b)
            } else {
                Mask::rect(format!("Mask {n}"), b)
            });
            cmds.push(Command::SetLayerMeta { layer: id, meta });
        }
        if cmds.is_empty() {
            return Err("Select the layers to mask first.".into());
        }
        self.exec_all(if ellipse { "Add ellipse mask" } else { "Add rectangle mask" }, cmds)?;
        Ok(Json::Null)
    }

    /// With two layers selected, the upper one (a shape) becomes a mask on the other.
    fn shape_to_mask(&mut self) -> Result<Json, String> {
        let [a, b] = self.selection[..] else {
            return Err("Select two layers: the shape to use (on top) and the layer to mask.".into());
        };
        let c = self.project().comp(self.comp).map_err(err)?.clone();
        let (ia, ib) = (c.layer_index(a).unwrap_or(0), c.layer_index(b).unwrap_or(0));
        let (shape_id, target) = if ia > ib { (a, b) } else { (b, a) };
        let shape = c.layer(shape_id).cloned().ok_or("no layer")?;
        if shape.shapes().is_empty() {
            return Err("The upper layer has to be a shape to use as a mask.".into());
        }
        let to_comp = xenify_anim::world_transform(&c, shape_id, self.time);
        let to_local = xenify_anim::world_transform(&c, target, self.time).inverse();
        let mut path = kurbo::BezPath::new();
        for s in shape.shapes() {
            path.extend(xenify_anim::geometry_path(&s.geometry, self.time).iter());
        }
        let path = (to_local * to_comp) * path;
        let mut meta = c.layer(target).ok_or("no layer")?.meta.clone();
        meta.masks.push(Mask::new(shape.meta.name.clone(), path));
        self.exec(
            "Use shape as mask",
            Command::batch(vec![
                Command::SetLayerMeta { layer: target, meta },
                Command::RemoveLayer { layer: shape_id },
            ]),
        )?;
        self.selection = vec![target];
        Ok(Json::Null)
    }

    /// Shows the layer only inside (or outside) another: `layer` null for none.
    fn set_matte(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let mut meta = self.project().layer(id).map_err(err)?.meta.clone();
        let mode: MatteMode =
            serde_json::from_value(args.get("mode").cloned().unwrap_or(json!("alpha"))).map_err(err)?;
        meta.matte = args.get("layer").and_then(Json::as_u64).map(|l| Matte { layer: Id(l), mode });
        self.exec("Set mask layer", Command::SetLayerMeta { layer: id, meta })?;
        Ok(Json::Null)
    }

    /// A shape's fill or stroke paint: `solid`, `linear` or `radial` (made
    /// from its colour, across its box).
    fn set_gradient(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        let stroke = args.get("stroke").and_then(Json::as_bool).unwrap_or(false);
        let kind = match args.get("kind").and_then(Json::as_str) {
            Some("linear") => Some(GradientKind::Linear),
            Some("radial") => Some(GradientKind::Radial),
            _ => None,
        };
        let l = self.project().layer(id).map_err(err)?;
        let mut shape = l.shapes().get(index).cloned().ok_or("no such shape")?;
        let b = xenify_anim::geometry_path(&shape.geometry, self.time);
        let b = kurbo::Shape::bounding_box(&b);
        let base = if stroke {
            shape.stroke.as_ref().map(|s| s.color.value_at(self.time))
        } else {
            shape.fill.as_ref().map(|s| s.color.value_at(self.time))
        }
        .ok_or(if stroke { "Turn the stroke on first." } else { "Turn the fill on first." })?;
        let make = |k: GradientKind| {
            let (start, end) = match k {
                GradientKind::Linear => (Vec2::new(b.x0, b.center().y), Vec2::new(b.x1, b.center().y)),
                GradientKind::Radial => (b.center().to_vec2(), Vec2::new(b.x1, b.center().y)),
            };
            let stops = vec![
                GradientStop { offset: 0.0, color: base },
                GradientStop { offset: 1.0, color: Color::rgba(base.r * 0.35, base.g * 0.35, base.b * 0.35, base.a) },
            ];
            GradientPaint { kind: k, start: start.into(), end: end.into(), stops: stops.into() }
        };
        let slot = if stroke {
            shape.stroke.as_mut().map(|s| &mut s.gradient)
        } else {
            shape.fill.as_mut().map(|s| &mut s.gradient)
        };
        let Some(slot) = slot else { return Ok(Json::Null) };
        *slot = match (kind, slot.take()) {
            (None, _) => None,
            (Some(k), Some(mut g)) => {
                g.kind = k;
                Some(g)
            }
            (Some(k), None) => Some(make(k)),
        };
        self.exec("Change paint", Command::SetShape { layer: id, index, shape })?;
        Ok(Json::Null)
    }

    /// The gradient editor's tools on shape `index`'s fill (or `stroke`) gradient: `reverse`, `fade` (the colour
    /// fading out), `shine` (a highlight, as if rounded), `fit` (span the shape again), `angle` (degrees, linear) and
    /// `radius` (radial). One undo step; animated ends and colours get keys (as typing them would).
    fn gradient_op(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        let stroke = args.get("stroke").and_then(Json::as_bool).unwrap_or(false);
        let op = args.get("op").and_then(Json::as_str).unwrap_or("");
        let value = f(args, "value");
        let l = self.project().layer(id).map_err(err)?;
        let shape = l.shapes().get(index).cloned().ok_or("no such shape")?;
        let paint = if stroke {
            shape.stroke.as_ref().and_then(|s| s.gradient.clone())
        } else {
            shape.fill.as_ref().and_then(|s| s.gradient.clone())
        }
        .ok_or("no gradient")?;
        let base = if stroke {
            shape.stroke.as_ref().map(|s| s.color.value_at(self.time))
        } else {
            shape.fill.as_ref().map(|s| s.color.value_at(self.time))
        }
        .unwrap_or(Color::BLACK);
        let bounds = kurbo::Shape::bounding_box(&xenify_anim::geometry_path(&shape.geometry, self.time));
        let mut g = paint.value_at(self.time);
        match op {
            "reverse" => {
                for st in &mut g.stops {
                    st.offset = 1.0 - st.offset;
                }
            }
            "fade" => {
                let c = g.sorted_stops().first().map_or(base, |s| s.color);
                g.stops = vec![
                    GradientStop { offset: 0.0, color: c },
                    GradientStop { offset: 1.0, color: Color::rgba(c.r, c.g, c.b, 0.0) },
                ];
            }
            "shine" => {
                let light = Color::rgba(
                    base.r + (1.0 - base.r) * 0.75,
                    base.g + (1.0 - base.g) * 0.75,
                    base.b + (1.0 - base.b) * 0.75,
                    base.a,
                );
                let dark = Color::rgba(base.r * 0.3, base.g * 0.3, base.b * 0.3, base.a);
                let c = Vec2::new(bounds.x0 + bounds.width() * 0.35, bounds.y0 + bounds.height() * 0.3);
                let r = bounds.width().max(bounds.height()) * 0.8;
                g = xenify_doc::Gradient {
                    kind: GradientKind::Radial,
                    start: c,
                    end: c + Vec2::new(r, 0.0),
                    stops: vec![
                        GradientStop { offset: 0.0, color: light },
                        GradientStop { offset: 0.35, color: base },
                        GradientStop { offset: 1.0, color: dark },
                    ],
                };
            }
            "fit" => {
                let angle = if g.kind == GradientKind::Radial { 0.0 } else { g.angle_deg() };
                g = xenify_doc::Gradient::across(bounds, g.kind, angle, g.stops.clone());
            }
            "angle" => {
                // Turned about its middle, the same length.
                let mid = (g.start + g.end) / 2.0;
                let half = (g.end - g.start).hypot() / 2.0;
                let a = value.unwrap_or(0.0).to_radians();
                let dir = Vec2::new(a.cos(), a.sin());
                g.start = mid - dir * half;
                g.end = mid + dir * half;
            }
            "radius" => {
                let dir = (g.end - g.start).normalize();
                let dir = if dir.is_finite() { dir } else { Vec2::new(1.0, 0.0) };
                g.end = g.start + dir * value.unwrap_or(1.0).max(1.0);
            }
            _ => return Err(format!("no gradient tool {op}")),
        }
        let mut cmds = Vec::new();
        if g.kind != paint.kind {
            let mut new = shape.clone();
            let slot = if stroke {
                new.stroke.as_mut().map(|s| &mut s.gradient)
            } else {
                new.fill.as_mut().map(|s| &mut s.gradient)
            };
            if let Some(slot) = slot {
                *slot = Some(GradientPaint {
                    kind: g.kind,
                    start: g.start.into(),
                    end: g.end.into(),
                    stops: g.stops.clone().into(),
                });
            }
            cmds.push(Command::SetShape { layer: id, index, shape: new });
        } else {
            let pre =
                if stroke { format!("shapes.{index}.stroke_gradient_") } else { format!("shapes.{index}.gradient_") };
            for (name, v) in [
                ("start", xenify_doc::Value::Vec2(g.start)),
                ("end", xenify_doc::Value::Vec2(g.end)),
                ("colors", xenify_doc::Value::Stops(g.stops.clone())),
            ] {
                let path = crate::prop_path(&format!("{pre}{name}")).ok_or("no such gradient")?;
                cmds.push(Command::edit_value(self.project(), id, path, self.time, v, self.auto_key).map_err(err)?);
            }
        }
        // Typing an angle or radius merges; each button press is its own step.
        if value.is_some() {
            self.editor
                .execute_merged("Gradient", format!("gradient-op:{}:{op}", id.0), Command::batch(cmds))
                .map_err(err)?;
        } else {
            self.exec("Gradient", Command::batch(cmds))?;
        }
        Ok(Json::Null)
    }

    /// An effect from the gallery (new layers), on the selected layer for
    /// those that need one, at `at` (composition space) when given.
    fn add_gallery_effect(&mut self, args: &Json) -> Result<Json, String> {
        let name = args.get("name").and_then(Json::as_str).ok_or("missing name")?.to_string();
        let target = self.selection.first().copied();
        if xenify_ops::effects::needs_target(&name) && target.is_none() {
            return Err(format!("Select the layer for {name} first."));
        }
        let (target_box, target_local_box) = match target {
            Some(t) => {
                let scene = self.scene();
                let c = self.project().comp(self.comp).map_err(err)?.clone();
                (xenify_anim::world_box(&c, &scene, t, &mut self.text), self.local_box(t))
            }
            None => (None, None),
        };
        let at = args.get("at").and_then(|v| Some(Vec2::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)));
        let placement = xenify_ops::effects::Placement { at, time: self.time, target, target_box, target_local_box };
        let first = Id(self.project().next_id);
        let (cmd, select) =
            xenify_ops::effects::add_effect(self.project(), self.comp, first, &name, placement).map_err(err)?;
        self.exec(&format!("Add {name}"), cmd)?;
        self.selection = vec![select];
        Ok(json!(select.0))
    }

    /// The gradient tool: the selected shapes' fill (or `stroke`) gradient
    /// runs from `a` to `b` (composition space); a solid paint becomes a
    /// linear gradient. Steps of one drag (`drag`) are one undo step.
    fn gradient_drag(&mut self, args: &Json) -> Result<Json, String> {
        let pt = |k: &str| -> Option<kurbo::Point> {
            let v = args.get(k)?;
            Some(kurbo::Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))
        };
        let (a, b) = (pt("a").ok_or("missing a")?, pt("b").ok_or("missing b")?);
        let stroke = args.get("stroke").and_then(Json::as_bool).unwrap_or(false);
        let key = format!("gradient:{}", f(args, "drag").unwrap_or(0.0));
        let mut cmds = Vec::new();
        for id in self.selection.clone() {
            let Ok(l) = self.project().layer(id).cloned() else { continue };
            let Ok(c) = self.project().comp(self.comp) else { continue };
            let inv = xenify_anim::world_transform(c, id, self.time).inverse();
            let (la, lb) = ((inv * a).to_vec2(), (inv * b).to_vec2());
            for (index, item) in l.shapes().iter().enumerate() {
                let mut shape = item.clone();
                let base = if stroke {
                    shape.stroke.as_ref().map(|s| s.color.value_at(self.time))
                } else {
                    shape.fill.as_ref().map(|s| s.color.value_at(self.time))
                };
                let Some(base) = base else { continue };
                let slot = if stroke {
                    shape.stroke.as_mut().map(|s| &mut s.gradient)
                } else {
                    shape.fill.as_mut().map(|s| &mut s.gradient)
                };
                let Some(slot) = slot else { continue };
                match slot {
                    Some(g) => {
                        g.start = la.into();
                        g.end = lb.into();
                    }
                    None => {
                        let stops = vec![
                            GradientStop { offset: 0.0, color: base },
                            GradientStop {
                                offset: 1.0,
                                color: Color::rgba(base.r * 0.35, base.g * 0.35, base.b * 0.35, base.a),
                            },
                        ];
                        *slot = Some(GradientPaint {
                            kind: GradientKind::Linear,
                            start: la.into(),
                            end: lb.into(),
                            stops: stops.into(),
                        });
                    }
                }
                cmds.push(Command::SetShape { layer: id, index, shape });
            }
        }
        if cmds.is_empty() {
            return Err("Select shapes with a fill to give them a gradient.".into());
        }
        self.editor.execute_merged("Gradient", key, Command::batch(cmds)).map_err(err)?;
        Ok(Json::Null)
    }

    /// A picture (base64 bytes, its size measured by the screens) repeated as a shape's fill.
    fn set_pattern(&mut self, args: &Json) -> Result<Json, String> {
        use base64::Engine as _;
        let id = id_arg(args, "id")?;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        let data = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing picture")?)
            .map_err(err)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Pattern");
        let mime = args.get("mime").and_then(Json::as_str).unwrap_or("image/png");
        let (w, h) = (f(args, "width").unwrap_or(0.0), f(args, "height").unwrap_or(0.0));
        if w < 1.0 || h < 1.0 {
            return Err("couldn't read the picture's size".into());
        }
        let mut p = self.project().clone();
        let (add, asset) = xenify_ops::assets::add_image(&mut p, name, mime, w as u32, h as u32, data, None);
        let l = self.project().layer(id).map_err(err)?;
        let mut shape = l.shapes().get(index).cloned().ok_or("no such shape")?;
        // About four tiles across the shape.
        let b = kurbo::Shape::bounding_box(&xenify_anim::geometry_path(&shape.geometry, self.time));
        let mut pattern = xenify_doc::Pattern::new(asset);
        pattern.scale = (b.width() / 4.0 / w).clamp(0.01, 10.0);
        pattern.offset = Vec2::new(b.x0, b.y0);
        match &mut shape.fill {
            Some(fl) => fl.pattern = Some(pattern),
            None => {
                shape.fill =
                    Some(xenify_doc::Fill { color: Color::WHITE.into(), gradient: None, pattern: Some(pattern) })
            }
        }
        let mut cmds: Vec<Command> = add.into_iter().collect();
        cmds.push(Command::SetShape { layer: id, index, shape });
        self.exec_all("Picture fill", cmds)?;
        Ok(Json::Null)
    }

    /// Puts pen `style` (as stored; null for a plain line) on every stroke of the selected layers.
    fn set_pen(&mut self, args: &Json) -> Result<Json, String> {
        let style: Option<PenStyle> = match args.get("style") {
            None | Some(Json::Null) => None,
            Some(v) => Some(serde_json::from_value(v.clone()).map_err(err)?),
        };
        let mut cmds = Vec::new();
        for (id, _) in self.targets(args) {
            let Ok(l) = self.project().layer(id) else { continue };
            for (index, item) in l.shapes().iter().enumerate() {
                let Some(st) = &item.stroke else { continue };
                let mut shape = item.clone();
                if let Some(s) = &mut shape.stroke {
                    s.style = style.clone().unwrap_or_default();
                }
                if st.style != shape.stroke.as_ref().map(|s| s.style.clone()).unwrap_or_default() {
                    cmds.push(Command::SetShape { layer: id, index, shape });
                }
            }
        }
        if cmds.is_empty() {
            return Err("Select shapes with a stroke to use the pen on.".into());
        }
        self.exec_all("Pen", cmds)?;
        Ok(Json::Null)
    }

    /// Another shape in the same layer (a copy of the last one, or a
    /// rectangle), so one layer can hold several outlines.
    fn add_shape_item(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let l: &Layer = self.project().layer(id).map_err(err)?;
        let item = l.shapes().last().cloned().unwrap_or_else(|| ShapeItem::rect(Vec2::new(100.0, 100.0), Color::BLACK));
        let index = l.shapes().len();
        self.exec("Add shape", Command::AddShape { layer: id, index: Some(index), shape: item })?;
        Ok(json!(index))
    }
}
