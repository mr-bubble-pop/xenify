//! Animation: easing between keyframes, ready-made animations (fade, pop,
//! bounce…), wiggle, expressions, parenting, and what the graph editor
//! draws (a property's keyframes and its value over time).

use serde_json::{Value as Json, json};
use xenify_doc::{Easing, Expression, Id, PropPath, Value, Wiggle};
use xenify_ops::Command;
use xenify_ops::presets::Preset;

use crate::{Session, err, f, id_arg, json_value, prop_path, value_json};

fn path_arg(args: &Json) -> Result<PropPath, String> {
    let name = args.get("prop").and_then(Json::as_str).unwrap_or("");
    prop_path(name).ok_or_else(|| format!("no property {name}"))
}

/// An easing by its preset name, or as stored (`{"bezier": …}`).
pub(crate) fn easing_arg(v: &Json) -> Result<Easing, String> {
    if let Some(name) = v.as_str() {
        return Easing::PRESETS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, e)| *e)
            .ok_or_else(|| format!("no easing {name}"));
    }
    serde_json::from_value(v.clone()).map_err(err)
}

fn easing_name(e: Easing) -> String {
    Easing::PRESETS.iter().find(|(_, p)| *p == e).map_or_else(
        || match e {
            Easing::Bezier { .. } => "custom".into(),
            Easing::Steps { .. } => "steps".into(),
            Easing::Spring { .. } => "spring".into(),
            _ => "custom".into(),
        },
        |(n, _)| n.to_string(),
    )
}

/// A value as numbers (for the graph): scalars one, positions two, colours four.
fn dims(v: &Value) -> Vec<f64> {
    match v {
        Value::Scalar(s) => vec![*s],
        Value::Vec2(p) => vec![p.x, p.y],
        Value::Color(c) => vec![c.r, c.g, c.b, c.a],
        _ => Vec::new(),
    }
}

impl Session {
    pub(crate) fn anim_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "easings" => {
                Ok(json!(Easing::PRESETS.iter().map(|(n, e)| json!({ "name": n, "easing": e })).collect::<Vec<_>>()))
            }
            "set_easing" => self.set_easing(args),
            "ease_sample" => easing_arg(args.get("easing").unwrap_or(&Json::Null)).map(|e| {
                let n = args.get("n").and_then(Json::as_u64).unwrap_or(200).clamp(2, 2000);
                json!((0..=n).map(|i| e.apply(i as f64 / n as f64)).collect::<Vec<_>>())
            }),
            "key_easings" => self.key_easings(args),
            "anim_presets" => {
                Ok(json!(Preset::DEFAULTS.iter().map(|(n, p)| json!({ "name": n, "preset": p })).collect::<Vec<_>>()))
            }
            "apply_preset" => self.apply_preset(args),
            "set_wiggle" => self.set_wiggle(args),
            "set_expression" => self.set_expression(args),
            "expr_preview" => self.expr_preview(args),
            "set_parent" => self.set_parent(args),
            "graph" => self.graph(args),
            "set_key" => self.set_key(args),
            "physics" => self.physics(args),
            "orbit" => self.orbit(args),
            "joint_chain" => {
                let cmd = xenify_ops::rig::make_joint_chain(self.project(), self.comp, &self.selection, self.time)
                    .map_err(err);
                cmd.and_then(|c| self.exec("Joint chain", c)).map(|_| Json::Null)
            }
            "unlink_joints" => {
                let cmd = xenify_ops::rig::unlink(self.project(), &self.selection).map_err(err);
                cmd.and_then(|c| self.exec("Unlink joints", c)).map(|_| Json::Null)
            }
            "ik_move" => self.ik_move(args),
            "bones" | "pose_turn" | "move_pivot" | "attach" | "follow_path" => self.rig_call(name, args),
            "text_anim_presets" => Ok(json!(xenify_doc::text_anim::PRESETS)),
            "add_text_anim" => {
                let name = args.get("name").and_then(Json::as_str).unwrap_or("").to_string();
                let r = id_arg(args, "id").and_then(|id| {
                    xenify_ops::text_anim::add_preset(self.project(), id, &name, self.time).map_err(err)
                });
                r.and_then(|c| self.exec(&format!("Text animation: {name}"), c)).map(|_| Json::Null)
            }
            "set_text_anims" => self.set_text_anims(args),
            _ => return None,
        })
    }

    /// Easing from the keyframe at `time` to the next, on one property or (`prop` missing) every animated one of the layer there.
    fn set_easing(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let time = f(args, "time").ok_or("missing time")?;
        let easing = easing_arg(args.get("easing").ok_or("missing easing")?)?;
        let paths: Vec<PropPath> = match args.get("prop") {
            Some(_) => vec![path_arg(args)?],
            None => {
                let l = self.project().layer(id).map_err(err)?;
                l.property_paths()
                    .into_iter()
                    .filter(|p| l.keyframe_times(*p).is_ok_and(|t| t.iter().any(|k| (k - time).abs() < 1e-6)))
                    .collect()
            }
        };
        let cmds: Vec<Command> =
            paths.into_iter().map(|path| Command::SetEasing { layer: id, path, time, easing }).collect();
        let key = format!("easing:{}:{time}", id.0);
        if args.get("merge").and_then(Json::as_bool).unwrap_or(false) {
            self.editor.execute_merged("Set easing", key, Command::batch(cmds)).map_err(err)?;
        } else {
            self.exec_all("Set easing", cmds)?;
        }
        Ok(Json::Null)
    }

    /// Each keyframe of a property with its easing (name and as stored).
    fn key_easings(&self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let path = path_arg(args)?;
        let l = self.project().layer(id).map_err(err)?;
        let prop = l.property(path).map_err(err)?;
        Ok(json!(
            prop.keyframe_times()
                .iter()
                .map(|t| {
                    let e = prop.easing_at(*t).unwrap_or_default();
                    json!({ "time": t, "name": easing_name(e), "easing": e })
                })
                .collect::<Vec<_>>()
        ))
    }

    /// A ready-made animation on the selected layers, from the playhead; `stagger` seconds between layers.
    fn apply_preset(&mut self, args: &Json) -> Result<Json, String> {
        let preset: Preset = match args.get("preset") {
            Some(v) if v.is_object() => serde_json::from_value(v.clone()).map_err(err)?,
            _ => {
                let name = args.get("name").and_then(Json::as_str).unwrap_or("");
                Preset::DEFAULTS
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, p)| *p)
                    .ok_or_else(|| format!("no animation {name}"))?
            }
        };
        // On `ids` (else the selection), starting at `time` (else the playhead).
        let ids: Vec<Id> = match args.get("ids").and_then(Json::as_array) {
            Some(a) => a.iter().filter_map(Json::as_u64).map(Id).collect(),
            None => self.selection.clone(),
        };
        if ids.is_empty() {
            return Err("Select the layers to animate first.".into());
        }
        let stagger = f(args, "stagger").unwrap_or(0.0);
        let at = f(args, "time").unwrap_or(self.time).max(0.0);
        let cmd = xenify_ops::presets::apply_many(self.project(), &ids, preset, at, stagger).map_err(err)?;
        let label = args.get("name").and_then(Json::as_str).unwrap_or("Animate").to_string();
        self.exec(&label, cmd)?;
        Ok(Json::Null)
    }

    /// Wiggle (noise on position, rotation and scale): `wiggle` as stored, or null for none.
    fn set_wiggle(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let wiggle: Option<Wiggle> = match args.get("wiggle") {
            None | Some(Json::Null) => None,
            Some(v) => Some(serde_json::from_value(v.clone()).map_err(err)?),
        };
        let cmd = Command::SetWiggle { layer: id, wiggle };
        if args.get("merge").and_then(Json::as_bool).unwrap_or(false) {
            self.editor.execute_merged("Wiggle", format!("wiggle:{}", id.0), cmd).map_err(err)?;
        } else {
            self.exec("Wiggle", cmd)?;
        }
        Ok(Json::Null)
    }

    /// The expression on property `prop` (empty `source` removes it).
    fn set_expression(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let path = path_arg(args)?;
        let source = args.get("source").and_then(Json::as_str).unwrap_or("").to_string();
        let mut meta = self.project().layer(id).map_err(err)?.meta.clone();
        meta.expressions.retain(|e| e.path != path);
        if !source.trim().is_empty() {
            meta.expressions.push(Expression { path, source });
        }
        self.editor
            .execute_merged("Expression", format!("expr:{}:{path}", id.0), Command::SetLayerMeta { layer: id, meta })
            .map_err(err)?;
        Ok(Json::Null)
    }

    /// What an expression gives now, or what's wrong with it.
    fn expr_preview(&self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let path = path_arg(args)?;
        let source = args.get("source").and_then(Json::as_str).unwrap_or("");
        let c = self.project().comp(self.comp).map_err(err)?;
        Ok(match xenify_anim::expr::preview(c, id, source, path, self.time) {
            Ok(v) => json!({ "ok": value_json(&v) }),
            Err(e) => json!({ "error": e }),
        })
    }

    /// Parents layer `id` to `parent` (null: none), keeping where it is on the canvas.
    fn set_parent(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let parent = args.get("parent").and_then(Json::as_u64).map(Id);
        let mut meta = self.project().layer(id).map_err(err)?.meta.clone();
        if parent == Some(id) {
            return Err("A layer can't be its own parent.".into());
        }
        meta.parent = parent;
        self.exec("Set parent", Command::SetLayerMeta { layer: id, meta })?;
        Ok(Json::Null)
    }

    /// The graph editor's picture of one property: its keyframes (time,
    /// value, easing) and its value sampled every frame from 0 to the end.
    fn graph(&self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let path = path_arg(args)?;
        let l = self.project().layer(id).map_err(err)?;
        let prop = l.property(path).map_err(err)?;
        let s = self.project().comp(self.comp).map_err(err)?.settings.clone();
        let step = 1.0 / s.fps.max(1.0);
        let n = ((s.duration / step).ceil() as usize).clamp(1, 4000);
        let samples: Vec<Json> = (0..=n)
            .map(|i| {
                let t = (i as f64 * step).min(s.duration);
                let mut row = vec![t];
                row.extend(l.value_at(path, t).map(|v| dims(&v)).unwrap_or_default());
                json!(row)
            })
            .collect();
        let keys: Vec<Json> = prop
            .keyframe_times()
            .iter()
            .map(|t| {
                let v = l.value_at(path, *t).map(|v| dims(&v)).unwrap_or_default();
                json!({ "time": t, "value": v, "easing": prop.easing_at(*t).unwrap_or_default() })
            })
            .collect();
        Ok(json!({ "keys": keys, "samples": samples }))
    }

    /// Bakes physics for the selected layers from the playhead (keyframes to the end).
    fn physics(&mut self, args: &Json) -> Result<Json, String> {
        let mut st = xenify_ops::physics::PhysicsSettings::default();
        let b = |k: &str, d: bool| args.get(k).and_then(Json::as_bool).unwrap_or(d);
        st.gravity = f(args, "gravity").unwrap_or(st.gravity);
        st.bounce = f(args, "bounce").unwrap_or(st.bounce).clamp(0.0, 1.0);
        st.friction = f(args, "friction").unwrap_or(st.friction).clamp(0.0, 1.0);
        st.squish = f(args, "squish").unwrap_or(st.squish).clamp(0.0, 1.0);
        st.floor = b("floor", st.floor);
        st.walls = b("walls", st.walls);
        st.ceiling = b("ceiling", st.ceiling);
        st.obstacles = b("obstacles", st.obstacles);
        st.squish_moving = b("squish_moving", st.squish_moving);
        st.squish_hit = b("squish_hit", st.squish_hit);
        if self.selection.is_empty() {
            return Err("Select the layers that should fall and bounce.".into());
        }
        let (p, comp, sel, time) = (self.project().clone(), self.comp, self.selection.clone(), self.time);
        let cmd = xenify_ops::physics::simulate(&p, comp, &sel, time, &st, &mut self.text).map_err(err)?;
        self.exec("Physics", cmd)?;
        Ok(Json::Null)
    }

    /// The other selected layers circle the first one clicked, from the playhead.
    fn orbit(&mut self, args: &Json) -> Result<Json, String> {
        let mut st = xenify_ops::physics::OrbitSettings::default();
        st.speed = f(args, "speed").unwrap_or(st.speed);
        st.flatten = f(args, "flatten").unwrap_or(st.flatten).clamp(0.05, 1.0);
        st.clockwise = args.get("clockwise").and_then(Json::as_bool).unwrap_or(true);
        st.kepler = args.get("kepler").and_then(Json::as_bool).unwrap_or(false);
        let [center, ref orbiters @ ..] = self.selection[..] else {
            return Err("Select the centre first, then what goes round it (Shift+click).".into());
        };
        if orbiters.is_empty() {
            return Err("Select the centre first, then what goes round it (Shift+click).".into());
        }
        let cmd =
            xenify_ops::physics::orbit(self.project(), self.comp, center, orbiters, self.time, &st).map_err(err)?;
        self.exec("Orbit", cmd)?;
        Ok(Json::Null)
    }

    /// The Bones tool: where each layer turns, posing (turning about the pivot), moving a joint, attaching.
    fn rig_call(&mut self, name: &str, args: &Json) -> Result<Json, String> {
        match name {
            "bones" => {
                let c = self.project().comp(self.comp).map_err(err)?;
                let list = xenify_ops::rig::bones(c, self.time);
                Ok(json!(
                    list.iter()
                        .map(
                            |(id, at, parent)| json!({ "id": id.0, "at": [at.x, at.y], "parent": parent.map(|p| p.0), "rotation": c.layer(*id).map_or(0.0, |l| l.transform.rotation.value_at(self.time)) })
                        )
                        .collect::<Vec<_>>()
                ))
            }
            "pose_turn" => {
                // The Bones tool: the layer turns about its pivot (`rotation` in degrees); one drag, one undo step.
                let id = id_arg(args, "id")?;
                let deg = f(args, "rotation").ok_or("missing rotation")?;
                let cmd = Command::edit_value(
                    self.project(),
                    id,
                    xenify_doc::PropPath::Rotation,
                    self.time,
                    xenify_doc::Value::Scalar(deg),
                    self.auto_key,
                )
                .map_err(err)?;
                let key = format!("pose:{}", f(args, "drag").unwrap_or(0.0));
                self.editor.execute_merged("Pose", key, cmd).map_err(err)?;
                Ok(Json::Null)
            }
            "move_pivot" => {
                let id = id_arg(args, "id")?;
                let to = args
                    .get("to")
                    .and_then(|v| Some(kurbo::Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
                    .ok_or("missing to")?;
                let cmd = xenify_ops::rig::move_pivot(self.project(), self.comp, id, to, self.time).map_err(err)?;
                let key = format!("pivot:{}", f(args, "drag").unwrap_or(0.0));
                self.editor.execute_merged("Move the joint", key, cmd).map_err(err)?;
                Ok(Json::Null)
            }
            "attach" => {
                // `parent` null: no longer attached to anything.
                let id = id_arg(args, "id")?;
                let parent = args.get("parent").and_then(Json::as_u64).map(xenify_doc::Id);
                let cmds =
                    xenify_ops::rig::place_under(self.project(), self.comp, id, parent, self.time).map_err(err)?;
                self.exec(if parent.is_some() { "Attach" } else { "Detach" }, Command::batch(cmds))?;
                Ok(Json::Null)
            }
            "follow_path" => {
                // The selected layer goes along `path` from the playhead for `seconds`.
                let id = id_arg(args, "id")?;
                let path = id_arg(args, "path")?;
                let seconds = f(args, "seconds").unwrap_or(2.0);
                let turn = args.get("turn").and_then(Json::as_bool).unwrap_or(false);
                let reverse = args.get("reverse").and_then(Json::as_bool).unwrap_or(false);
                let cmd = xenify_ops::rig::follow_path(
                    self.project(),
                    self.comp,
                    id,
                    path,
                    self.time,
                    seconds,
                    turn,
                    reverse,
                )
                .map_err(err)?;
                self.exec("Follow a path", cmd)?;
                Ok(Json::Null)
            }
            _ => Err(format!("no call {name}")),
        }
    }

    /// Dragging the end of a joint chain: the chain bends to reach `to` (composition space).
    fn ik_move(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let to = args
            .get("to")
            .and_then(|v| Some(kurbo::Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
            .ok_or("missing to")?;
        // `chain`: how many joints above it bend (0: all), instead of its own joint chain.
        let chain = args.get("chain").and_then(Json::as_u64).map(|n| n as u32);
        let cmds =
            xenify_ops::rig::ik_commands_with(self.project(), self.comp, id, to, self.time, self.auto_key, chain)
                .map_err(err)?;
        if !cmds.is_empty() {
            let key = format!("ik:{}", f(args, "drag").unwrap_or(0.0));
            self.editor.execute_merged("Pose", key, Command::batch(cmds)).map_err(err)?;
        }
        Ok(Json::Null)
    }

    fn set_text_anims(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let animators: Vec<xenify_doc::TextAnimator> =
            serde_json::from_value(args.get("animators").cloned().unwrap_or(json!([]))).map_err(err)?;
        let cmd = Command::SetTextAnimators { layer: id, animators };
        self.editor.execute_merged("Text animation", format!("textanim:{}", id.0), cmd).map_err(err)?;
        Ok(Json::Null)
    }

    /// Sets the value of the keyframe at `time` (keeping its easing).
    fn set_key(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let path = path_arg(args)?;
        let time = f(args, "time").ok_or("missing time")?;
        let l = self.project().layer(id).map_err(err)?;
        let like = l.value_at(path, time).map_err(err)?;
        let value = json_value(args.get("value").unwrap_or(&Json::Null), &like).ok_or("bad value")?;
        let easing = l.property(path).map_err(err)?.easing_at(time);
        let key = format!("key:{}:{path}:{time}", id.0);
        self.editor
            .execute_merged("Move keyframe", key, Command::SetKeyframe { layer: id, path, time, value, easing })
            .map_err(err)?;
        Ok(Json::Null)
    }
}
