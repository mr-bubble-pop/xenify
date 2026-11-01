//! Handles on the canvas for generated layers: a particle emitter's marker
//! (click it to select the layer, even before particles fly) and its arrow
//! (angle = direction, length = speed), and water's level.

use kurbo::{Affine, Point, Vec2};
use serde_json::{Value as Json, json};
use xenify_doc::{EmitterField, Id, LayerKind, LiquidField, PropPath, Value};
use xenify_ops::Command;

use crate::{Session, err, f, id_arg};

/// Composition pixels of arrow per pixel-per-second of speed (as in the egui app).
const ARROW_SCALE: f64 = 0.2;

fn pt(v: Option<&Json>) -> Option<Point> {
    let v = v?;
    Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))
}

impl Session {
    /// The selected layer's first enabled Array, Twist or Mirror: where its origin is on the canvas,
    /// and a mirror's axis (degrees), to drag it.
    fn modifier_handle(&self) -> Json {
        let Some(&id) = self.selection.first().filter(|_| self.selection.len() == 1) else { return Json::Null };
        let (Ok(l), Ok(c)) = (self.project().layer(id), self.project().comp(self.comp)) else { return Json::Null };
        let Some((index, origin)) =
            l.meta.modifiers.iter().enumerate().find_map(|(i, m)| Some((i, m.kind.origin().filter(|_| m.enabled)?)))
        else {
            return Json::Null;
        };
        let m = xenify_anim::world_transform(c, id, self.time);
        let at = m * origin.value_at(self.time).to_point();
        let axis = match &l.meta.modifiers[index].kind {
            xenify_doc::ModifierKind::Mirror { rotation, .. } => {
                let o = origin.value_at(self.time).to_point();
                let d = m * (o + kurbo::Vec2::from_angle(rotation.value_at(self.time).to_radians()) * 100.0) - at;
                Some(d.y.atan2(d.x).to_degrees())
            }
            _ => None,
        };
        json!({ "index": index, "at": [at.x, at.y], "axis": axis })
    }

    /// Drags that origin to `to` (on the canvas).
    fn drag_modifier_origin(&mut self, args: &Json) -> Result<Json, String> {
        let id = crate::id_arg(args, "id")?;
        let index = args.get("index").and_then(Json::as_u64).ok_or("missing index")? as usize;
        let to = args
            .get("to")
            .and_then(Json::as_array)
            .and_then(|a| Some(kurbo::Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?)))
            .ok_or("missing to")?;
        let c = self.project().comp(self.comp).map_err(crate::err)?;
        let inv = xenify_anim::world_transform(c, id, self.time).inverse();
        let p = inv * to;
        let path = xenify_doc::PropPath::Modifier { index, field: xenify_doc::ModifierField::Origin };
        let cmd = Command::edit_value(
            self.project(),
            id,
            path,
            self.time,
            xenify_doc::Value::Vec2(p.to_vec2()),
            self.auto_key,
        )
        .map_err(crate::err)?;
        let n = args.get("drag").and_then(Json::as_u64).unwrap_or(0);
        self.editor.execute_merged("Move the origin", format!("modorigin:{}:{n}", id.0), cmd).map_err(crate::err)?;
        Ok(Json::Null)
    }

    pub(crate) fn handles_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "emitters" => Ok(self.emitters()),
            "modifier_handle" => Ok(self.modifier_handle()),
            "drag_modifier_origin" => self.drag_modifier_origin(args),
            "generator_handle" => id_arg(args, "id").map(|id| self.generator_handle(id)),
            "drag_emitter" => self.drag_emitter(args),
            "drag_level" => self.drag_level(args),
            _ => return None,
        })
    }

    /// Where layer `id` sits and faces: its origin (composition space), its turn (degrees), and its transform.
    fn frame_of(&self, id: Id) -> (Point, f64, Affine) {
        let world =
            self.project().comp(self.comp).map_or(Affine::IDENTITY, |c| xenify_anim::world_transform(c, id, self.time));
        let v = world * Point::new(1.0, 0.0) - world * Point::ZERO;
        (world * Point::ZERO, v.y.atan2(v.x).to_degrees(), world)
    }

    /// Every visible particle layer's marker.
    fn emitters(&self) -> Json {
        let Ok(c) = self.project().comp(self.comp) else { return json!([]) };
        json!(
            c.layers
                .iter()
                .filter(|l| matches!(l.kind, LayerKind::Particles(_)) && l.meta.visible && !l.meta.locked)
                .map(|l| {
                    let o = self.frame_of(l.id).0;
                    json!({ "id": l.id.0, "at": [o.x, o.y] })
                })
                .collect::<Vec<_>>()
        )
    }

    /// The selected generated layer's handle: an emitter's arrow, or water's level and box.
    fn generator_handle(&self, id: Id) -> Json {
        let Ok(l) = self.project().layer(id) else { return Json::Null };
        let (origin, turn, world) = self.frame_of(id);
        match &l.kind {
            LayerKind::Particles(e) => {
                let dir = (e.direction.value_at(self.time) + turn).to_radians();
                let len = (e.speed.value_at(self.time) * ARROW_SCALE).max(1.0);
                let tip = origin + Vec2::new(dir.cos(), dir.sin()) * len;
                json!({ "kind": "emitter", "origin": [origin.x, origin.y], "tip": [tip.x, tip.y] })
            }
            LayerKind::Liquid(w) => {
                let at = world * xenify_anim::liquid::surface_middle(w, self.time);
                let mut pts = Vec::new();
                kurbo::flatten(world * kurbo::Shape::to_path(&xenify_anim::liquid::container(w), 0.1), 0.5, |el| {
                    if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
                        pts.push([p.x, p.y]);
                    }
                });
                json!({ "kind": "water", "at": [at.x, at.y], "level": w.level.value_at(self.time), "box": pts })
            }
            _ => Json::Null,
        }
    }

    /// Dragging an emitter's arrow tip to `to`: its angle is the direction, its length the speed (`snap`: 15° steps).
    fn drag_emitter(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let to = pt(args.get("to")).ok_or("missing to")?;
        let (origin, turn, _) = self.frame_of(id);
        let LayerKind::Particles(e) = &self.project().layer(id).map_err(err)?.kind else {
            return Err("not particles".into());
        };
        let v = to - origin;
        if v.hypot() < 1.0 {
            return Ok(Json::Null);
        }
        let mut deg = v.y.atan2(v.x).to_degrees() - turn;
        if args.get("snap").and_then(Json::as_bool).unwrap_or(false) {
            deg = (deg / 15.0).round() * 15.0;
        }
        // Near the old value, so keyframes don't spin the long way round.
        let old = e.direction.value_at(self.time);
        deg += 360.0 * ((old - deg) / 360.0).round();
        let key = format!("emitter:{}", f(args, "drag").unwrap_or(0.0));
        let cmds = vec![
            Command::edit_value(
                self.project(),
                id,
                PropPath::Emitter(EmitterField::Direction),
                self.time,
                Value::Scalar(deg),
                self.auto_key,
            )
            .map_err(err)?,
            Command::edit_value(
                self.project(),
                id,
                PropPath::Emitter(EmitterField::Speed),
                self.time,
                Value::Scalar((v.hypot() / ARROW_SCALE).round()),
                self.auto_key,
            )
            .map_err(err)?,
        ];
        self.editor.execute_merged("Aim particles", key, Command::batch(cmds)).map_err(err)?;
        Ok(Json::Null)
    }

    /// Dragging water's surface to `to` sets its level.
    fn drag_level(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let to = pt(args.get("to")).ok_or("missing to")?;
        let (_, _, world) = self.frame_of(id);
        let LayerKind::Liquid(w) = &self.project().layer(id).map_err(err)?.kind else { return Err("not water".into()) };
        let level = xenify_anim::liquid::level_for(w, world.inverse() * to);
        let key = format!("level:{}", f(args, "drag").unwrap_or(0.0));
        let cmd = Command::edit_value(
            self.project(),
            id,
            PropPath::Liquid(LiquidField::Level),
            self.time,
            Value::Scalar((level * 1000.0).round() / 1000.0),
            self.auto_key,
        )
        .map_err(err)?;
        self.editor.execute_merged("Water level", key, cmd).map_err(err)?;
        Ok(Json::Null)
    }
}
