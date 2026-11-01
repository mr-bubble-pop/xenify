//! 3D. Insert › 3D model reads a model (STL, OBJ, glTF/GLB, 3MF) once,
//! draws it turned however you like (`model_scene`) and places that as a
//! picture, keeping the model with it so it can be turned again. The 3D
//! layers of older canvases (solid shapes, extrusions, models) still draw.

use std::sync::Arc;

use base64::Engine as _;
use kurbo::Affine;
use serde_json::{Value as Json, json};
use xenify_doc::{Color, Id, Layer, LayerKind, ModelSource, Project, Property, Solid, SolidShape, Vec2};
use xenify_ops::Command;

use crate::{Session, err, f, id_arg, parse_color};

/// A model read for Insert › 3D model, before it's placed.
pub(crate) struct Staged {
    name: String,
    data: Arc<[u8]>,
    mesh: Box<xenify_doc::Mesh>,
    /// The picture it was placed as, when it's being turned again.
    asset: Option<Id>,
}

/// How a model is turned (degrees).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pose {
    pub rot_x: f64,
    pub rot_y: f64,
    pub rot_z: f64,
}

impl Pose {
    pub fn from_json(v: &Json) -> Self {
        Self { rot_x: f(v, "rot_x").unwrap_or(0.0), rot_y: f(v, "rot_y").unwrap_or(0.0), rot_z: f(v, "rot_z").unwrap_or(0.0) }
    }
}

/// Its colour where the model has none of its own.
const MODEL_COLOR: &str = "#b9c6d8";

impl Session {
    pub(crate) fn model_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "solid_shapes" => Ok(json!(SolidShape::primitives().iter().map(|s| s.label()).collect::<Vec<_>>())),
            "add_solid" => self.add_solid(args),
            "extrude" => self.extrude(),
            "import_model" => self.import_model(args),
            "set_solid_json" => self.set_solid_json(args),
            "turn_3d" => self.orbit_drag(args),
            "solid_info" => id_arg(args, "id").and_then(|id| self.solid_info(id)),
            "solid_op" => self.solid_op(args),
            "model_stage" => self.model_stage(args),
            "model_place" => self.model_place(args),
            _ => return None,
        })
    }

    fn centre(&self) -> Vec2 {
        self.comp_settings().map_or(Vec2::ZERO, |s| Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0))
    }

    fn add_solid(&mut self, args: &Json) -> Result<Json, String> {
        let label = args.get("shape").and_then(Json::as_str).unwrap_or("Cube");
        let shape = SolidShape::primitives()
            .into_iter()
            .find(|s| s.label() == label)
            .ok_or_else(|| format!("no 3D shape {label}"))?;
        let s = self.comp_settings().ok_or("no canvas")?;
        let size = (s.width.min(s.height) as f64 / 3.0).round();
        let depth = if matches!(shape, SolidShape::Cylinder | SolidShape::Cone) { size * 1.2 } else { size };
        let color =
            args.get("color").and_then(parse_color).unwrap_or(Color::from_hex("#118ab2").unwrap_or(Color::WHITE));
        let id = self.next_id();
        let mut solid = Solid::new(shape, size, depth, color);
        solid.rot_x = 20.0.into();
        solid.rot_y = (-30.0).into();
        let layer =
            Layer::new(id, self.unique_name(label), LayerKind::Solid(Box::new(solid))).with_position(self.centre());
        self.add_layer(&format!("Add {}", label.to_lowercase()), layer)
    }

    /// The selected shape layer becomes a 3D object with depth.
    fn extrude(&mut self) -> Result<Json, String> {
        let id = *self.selection.first().ok_or("Select a shape to extrude.")?;
        let layer = self.project().layer(id).map_err(err)?.clone();
        let mut path = kurbo::BezPath::new();
        let mut fill_rule = xenify_doc::FillRule::NonZero;
        let mut color = None;
        for s in layer.shapes() {
            path.extend(xenify_anim::geometry_path(&s.geometry, self.time).iter());
            fill_rule = s.fill_rule;
            color = color
                .or(s.fill.as_ref().map(|f| f.color.value_at(self.time)))
                .or(s.stroke.as_ref().map(|s| s.color.value_at(self.time)));
        }
        if path.elements().is_empty() {
            return Err("Only shapes can be extruded.".into());
        }
        let b = kurbo::Shape::bounding_box(&path);
        let depth = (b.width().max(b.height()) * 0.3).round().max(1.0);
        let mut solid = Solid::new(SolidShape::Extrude { path, fill_rule }, 0.0, depth, color.unwrap_or(Color::WHITE));
        solid.rot_x = 15.0.into();
        solid.rot_y = (-25.0).into();
        let mut new = layer.clone();
        new.kind = LayerKind::Solid(Box::new(solid));
        let index = self.project().comp(self.comp).ok().and_then(|c| c.layer_index(id));
        self.exec(
            "Extrude",
            Command::batch(vec![
                Command::RemoveLayer { layer: id },
                Command::AddLayer { comp: self.comp, index, layer: new },
            ]),
        )?;
        self.selection = vec![id];
        Ok(Json::Null)
    }

    /// A model file (base64) as a 3D layer in the middle of the canvas; answers what was read.
    fn import_model(&mut self, args: &Json) -> Result<Json, String> {
        let name = args.get("name").and_then(Json::as_str).unwrap_or("model.stl").to_string();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing file")?)
            .map_err(err)?;
        let mut opts = xenify_export::model3d::ModelOptions::default();
        if let Some(n) = f(args, "max_triangles") {
            opts.max_triangles = n.clamp(500.0, 1_000_000.0) as usize;
        }
        let model =
            xenify_export::model3d::import_model(&name, &bytes, opts, &|_| None).map_err(|e| format!("{name}: {e}"))?;
        let s = self.comp_settings().ok_or("no canvas")?;
        let (w, h) = (s.width as f64, s.height as f64);
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s).to_string();
        let mut solid = Solid::new(
            SolidShape::Mesh(Box::new(model.mesh.clone())),
            (w.min(h) * 0.5).round(),
            0.0,
            Color::from_hex("#b9c6d8").unwrap_or(Color::WHITE),
        );
        solid.rot_x = 20.0.into();
        solid.rot_y = (-30.0).into();
        let id = self.next_id();
        let layer = Layer::new(id, self.unique_name(&stem), LayerKind::Solid(Box::new(solid)))
            .with_position(Vec2::new(w / 2.0, h / 2.0));
        self.add_layer(&format!("Import {stem}"), layer)?;
        Ok(json!({ "id": id.0, "summary": model.summary() }))
    }

    /// A 3D layer's settings as stored (perspective, its shape's own settings, part colours).
    fn set_solid_json(&mut self, args: &Json) -> Result<Json, String> {
        let layer = id_arg(args, "id")?;
        let solid: Solid = serde_json::from_value(args.get("solid").cloned().ok_or("missing solid")?).map_err(err)?;
        self.editor
            .execute_merged(
                "Change 3D",
                format!("solid:{}", layer.0),
                Command::SetSolid { layer, solid: Box::new(solid) },
            )
            .map_err(err)?;
        Ok(Json::Null)
    }

    fn solid_of(&self, id: xenify_doc::Id) -> Result<Solid, String> {
        match &self.project().layer(id).map_err(err)?.kind {
            LayerKind::Solid(s) => Ok((**s).clone()),
            _ => Err("not a 3D layer".into()),
        }
    }

    /// A 3D layer's shape and, for a model, its name, faces, colours and parts (without the mesh itself).
    fn solid_info(&self, id: xenify_doc::Id) -> Result<Json, String> {
        let s = self.solid_of(id)?;
        let mut out = json!({ "shape": s.shape.label(), "perspective": s.perspective });
        match &s.shape {
            SolidShape::Torus { thickness } => out["thickness"] = json!(thickness),
            SolidShape::Mesh(m) => {
                out["model"] = json!({
                    "name": m.name, "faces": m.polygon_count(), "has_colors": m.has_colors(), "own_colors": m.own_colors,
                    "parts": m.parts.iter().enumerate().map(|(i, p)| {
                        let motion = s.parts.get(i);
                        json!({ "name": if p.name.is_empty() { format!("Part {}", i + 1) } else { p.name.clone() }, "visible": motion.is_none_or(|m| m.visible), "moved": motion.is_some() })
                    }).collect::<Vec<_>>(),
                    "any_moved": !s.parts.is_empty(),
                });
            }
            _ => {}
        }
        Ok(out)
    }

    /// Changes to a 3D layer: `shape` (a primitive's name), `thickness` (torus), `own_colors` (a model),
    /// `part` + `visible` / `move`, `reset_parts`, `explode` (parts fly apart over a second from the playhead).
    fn solid_op(&mut self, args: &Json) -> Result<Json, String> {
        use xenify_doc::{Keyframe, PartMotion, Property};
        let id = id_arg(args, "id")?;
        let mut s = self.solid_of(id)?;
        let mut label = "Change 3D";
        if let Some(name) = args.get("shape").and_then(Json::as_str) {
            s.shape = SolidShape::primitives().into_iter().find(|p| p.label() == name).ok_or("no such shape")?;
            label = "Change 3D shape";
        }
        if let Some(t) = f(args, "thickness") {
            s.shape = SolidShape::Torus { thickness: t.clamp(0.05, 0.5) };
            label = "Change thickness";
        }
        if let Some(own) = args.get("own_colors").and_then(Json::as_bool)
            && let SolidShape::Mesh(m) = &mut s.shape
        {
            m.own_colors = own;
            label = "Model colours";
        }
        if let Some(i) = args.get("part").and_then(Json::as_u64).map(|i| i as usize) {
            if s.parts.len() <= i {
                s.parts.resize(i + 1, PartMotion::default());
            }
            if let Some(v) = args.get("visible").and_then(Json::as_bool) {
                s.parts[i].visible = v;
                label = "Show part";
            } else {
                label = "Move part";
            }
        }
        if args.get("reset_parts").and_then(Json::as_bool).unwrap_or(false) {
            s.parts.clear();
            label = "Reset parts";
        }
        if args.get("explode").and_then(Json::as_bool).unwrap_or(false) {
            let SolidShape::Mesh(mesh) = &s.shape else { return Err("only a model has parts".into()) };
            let mesh = mesh.clone();
            let size = s.size.value_at(self.time);
            let (t0, t1) = (self.time, self.time + 1.0);
            let centre = mesh.bounds().map_or([0.0; 3], |(lo, hi)| [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0));
            s.parts.resize(mesh.parts.len().max(s.parts.len()), PartMotion::default());
            let key = |time: f64, value| Keyframe { time, value, easing: xenify_doc::Easing::EASE_IN_OUT };
            for (i, part) in mesh.parts.iter().enumerate() {
                let Some((lo, hi)) = part.bounds() else { continue };
                let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0 - centre[k]);
                let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                // Parts at the very middle go straight up.
                let dir = if len < 1e-6 { [0.0, -1.0, 0.0] } else { c.map(|v| v / len) };
                let push = size * 0.35;
                let m = &mut s.parts[i];
                let (o0, d0) = (m.offset.value_at(t0), m.depth.value_at(t0));
                m.offset = Property::from_keyframes(vec![
                    Keyframe { time: t0, value: o0, easing: xenify_doc::Easing::EASE_IN_OUT },
                    Keyframe {
                        time: t1,
                        value: o0 + Vec2::new(dir[0] * push, dir[1] * push),
                        easing: xenify_doc::Easing::EASE_IN_OUT,
                    },
                ])
                .unwrap_or(m.offset.clone());
                m.depth =
                    Property::from_keyframes(vec![key(t0, d0), key(t1, d0 + dir[2] * push)]).unwrap_or(m.depth.clone());
            }
            label = "Explode parts";
        }
        let merge = args.get("thickness").is_some();
        let cmd = Command::SetSolid { layer: id, solid: Box::new(s) };
        if merge {
            self.editor.execute_merged(label, format!("solid:{}", id.0), cmd).map_err(err)?;
        } else {
            self.exec(label, cmd)?;
        }
        Ok(Json::Null)
    }

    /// The orbit tool: turns 3D layer `id` by `dx`, `dy` degrees (one undo step per `drag`).
    fn orbit_drag(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let l = self.project().layer(id).map_err(err)?;
        if !matches!(l.kind, LayerKind::Solid(_)) {
            return Err("The orbit tool turns 3D layers.".into());
        }
        let key = format!("orbit:{}", f(args, "drag").unwrap_or(0.0));
        let mut cmds = Vec::new();
        for (field, d) in [
            (xenify_doc::SolidField::RotY, f(args, "dx").unwrap_or(0.0)),
            (xenify_doc::SolidField::RotX, -f(args, "dy").unwrap_or(0.0)),
        ] {
            let path = xenify_doc::PropPath::Solid(field);
            let now = match l.value_at(path, self.time).map_err(err)? {
                xenify_doc::Value::Scalar(v) => v,
                _ => 0.0,
            };
            cmds.push(
                Command::edit_value(
                    self.project(),
                    id,
                    path,
                    self.time,
                    xenify_doc::Value::Scalar(now + d),
                    self.auto_key,
                )
                .map_err(err)?,
            );
        }
        self.editor.execute_merged("Turn", key, Command::batch(cmds)).map_err(err)?;
        Ok(Json::Null)
    }
}

impl Session {
    /// Reads a model file to turn: the file (`name`, `base64`), or the model picture layer `id` was drawn from.
    /// Answers what was read and how it's turned now.
    fn model_stage(&mut self, args: &Json) -> Result<Json, String> {
        let mut side = 0;
        let (name, data, asset, pose) = if let Some(id) = args.get("id").and_then(Json::as_u64) {
            let a = self.model_asset(Id(id))?;
            let m = a.model.as_deref().ok_or("This picture wasn't made from a 3D model.")?;
            side = a.width.max(a.height);
            (m.name.clone(), m.data.clone(), Some(a.id), Pose { rot_x: m.rot_x, rot_y: m.rot_y, rot_z: m.rot_z })
        } else {
            let name = args.get("name").and_then(Json::as_str).unwrap_or("model.stl").to_string();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(args.get("base64").and_then(Json::as_str).ok_or("missing file")?)
                .map_err(err)?;
            (name, Arc::from(bytes), None, Pose { rot_x: 20.0, rot_y: -30.0, rot_z: 0.0 })
        };
        let model = xenify_export::model3d::import_model(&name, &data, Default::default(), &|_| None)
            .map_err(|e| format!("{name}: {e}"))?;
        let summary = model.summary();
        self.model_stage = Some(Staged { name, data, mesh: Box::new(model.mesh), asset });
        // `side`: how big the picture is now (its longer side), when turning one again.
        Ok(json!({ "summary": summary, "rot_x": pose.rot_x, "rot_y": pose.rot_y, "rot_z": pose.rot_z, "side": side }))
    }

    /// The picture image layer `id` shows.
    fn model_asset(&self, id: Id) -> Result<&xenify_doc::ImageAsset, String> {
        match self.project().layer(id).map_err(err)?.kind {
            LayerKind::Image { asset, .. } => self.project().asset(asset).ok_or_else(|| "the picture is missing".into()),
            _ => Err("Select a picture made from a 3D model.".into()),
        }
    }

    /// The model being turned, drawn turned by `pose` on a see-through picture whose longer side is `size`
    /// pixels, cropped to the model.
    pub fn model_scene(&mut self, pose: Pose, size: u32) -> Option<xenify_anim::Scene> {
        let st = self.model_stage.as_ref()?;
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let c = p.comp_mut(comp).ok()?;
        c.settings.width = 1000;
        c.settings.height = 1000;
        c.settings.background = Color::rgba(0.0, 0.0, 0.0, 0.0);
        let mut solid = Solid::new(
            SolidShape::Mesh(st.mesh.clone()),
            600.0,
            0.0,
            Color::from_hex(MODEL_COLOR).unwrap_or(Color::WHITE),
        );
        solid.rot_x = pose.rot_x.into();
        solid.rot_y = pose.rot_y.into();
        solid.rot_z = pose.rot_z.into();
        c.layers.push(Layer::new(Id(1_000_000), "Model", LayerKind::Solid(Box::new(solid))).with_position(Vec2::new(500.0, 500.0)));
        let scene = xenify_anim::evaluate_project(&p, comp, 0.0);
        let mut layer = scene.layers.first()?.clone();
        let b = layer.transform.transform_rect_bbox(layer.bounds(&mut self.text)?);
        if b.width() < 1e-6 || b.height() < 1e-6 {
            return None;
        }
        let k = size.max(16) as f64 / b.width().max(b.height());
        let (w, h) = (((b.width() * k).ceil() as u32).max(1), ((b.height() * k).ceil() as u32).max(1));
        layer.transform = Affine::translate((w as f64 / 2.0, h as f64 / 2.0))
            * Affine::scale(k)
            * Affine::translate(-b.center().to_vec2())
            * layer.transform;
        Some(xenify_anim::Scene { width: w, height: h, background: Color::rgba(0.0, 0.0, 0.0, 0.0), layers: vec![layer], ..scene })
    }

    /// Places the model being turned as the picture drawn of it (`base64` PNG, `width`, `height`, turned by
    /// `pose`): a new picture layer, or (turned again) the picture it came from, changed in place.
    fn model_place(&mut self, args: &Json) -> Result<Json, String> {
        let st = self.model_stage.as_ref().ok_or("No 3D model is open.")?;
        let pose = Pose::from_json(args.get("pose").unwrap_or(&Json::Null));
        let model = ModelSource { name: st.name.clone(), data: st.data.clone(), rot_x: pose.rot_x, rot_y: pose.rot_y, rot_z: pose.rot_z };
        let stem = st.name.rsplit_once('.').map_or(st.name.as_str(), |(s, _)| s).to_string();
        let png = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing picture")?)
            .map_err(err)?;
        let (w, h) = (f(args, "width").unwrap_or(0.0) as u32, f(args, "height").unwrap_or(0.0) as u32);
        if w == 0 || h == 0 {
            return Err("the model's picture came out empty".into());
        }
        let turning = st.asset;
        let mut asset = xenify_doc::ImageAsset::new(Id(0), format!("{stem}.png"), "image/png", w, h, png);
        asset.model = Some(Box::new(model));
        if let Some(old) = turning.and_then(|a| self.project().asset(a).cloned()) {
            // Turned again: the same picture, so every layer showing it keeps its place, size and animation;
            // the middle stays where it was.
            asset.id = old.id;
            asset.name = old.name.clone();
            let mut cmds = vec![Command::SetAsset { asset }];
            let showing: Vec<Id> = self
                .project()
                .compositions
                .iter()
                .flat_map(|c| &c.layers)
                .filter(|l| matches!(l.kind, LayerKind::Image { asset: x, .. } if x == old.id))
                .filter(|l| matches!(l.transform.anchor, Property::Static(a) if (a - Vec2::new(old.width as f64 / 2.0, old.height as f64 / 2.0)).hypot() < 0.5))
                .map(|l| l.id)
                .collect();
            for id in showing {
                let mut l = self.project().layer(id).map_err(err)?.clone();
                l.transform.anchor = Vec2::new(w as f64 / 2.0, h as f64 / 2.0).into();
                // About as big on the canvas as before.
                let k = old.width.max(old.height) as f64 / w.max(h) as f64;
                if let Property::Static(s) = l.transform.scale {
                    l.transform.scale = Property::Static(s * k);
                }
                cmds.push(Command::ReplaceLayer { layer: Box::new(l) });
            }
            self.exec_all("Turn the 3D model", cmds)?;
            self.model_stage = None;
            return Ok(Json::Null);
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&asset.data);
        let model = asset.model.take().map(|m| *m);
        let id = self.add_picture_with(
            &json!({ "base64": b64, "name": format!("{stem}.png"), "mime": "image/png", "width": w, "height": h }),
            model,
        )?;
        self.model_stage = None;
        Ok(id)
    }
}

