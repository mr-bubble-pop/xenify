//! Symbols: reusable drawings with their own layers and timeline. Every
//! placed copy shows the symbol, so editing it changes them all. A symbol is
//! a composition that isn't the document or a slide.

use serde_json::{Value as Json, json};
use xenify_doc::{CompSettings, Composition, Id, Layer, LayerKind, Project, Vec2};
use xenify_ops::Command;

use crate::{Session, err, f, id_arg};

fn is_symbol(p: &Project, id: Id) -> bool {
    !xenify_ops::slides::slides_or_main(p).iter().any(|s| s.comp == id)
}

fn next_name(p: &Project) -> String {
    (1..)
        .map(|n| format!("Symbol {n}"))
        .find(|name| !p.compositions.iter().any(|c| &c.settings.name == name))
        .unwrap_or_default()
}

impl Session {
    pub(crate) fn comps_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "symbols" => Ok(self.symbols()),
            // A diagram (flowchart, network…) as a symbol to place on canvases, opened with the diagram tools.
            "new_diagram_symbol" => (|| -> Result<Json, String> {
                let name = (1..)
                    .map(|n| format!("Diagram {n}"))
                    .find(|name| !self.project().compositions.iter().any(|c| &c.settings.name == name))
                    .unwrap();
                let (cmd, id) = xenify_ops::diagram::new_diagram(self.project(), &name, 1600, 1000);
                self.exec("New diagram", cmd)?;
                self.edit_symbol(id)?;
                Ok(json!({ "id": id.0, "name": name }))
            })(),
            "edit_symbol" => id_arg(args, "id").and_then(|id| self.edit_symbol(id)),
            "finish_symbol" => self.finish_symbol(),
            "new_symbol" => self.new_symbol(),
            "place_symbol" => id_arg(args, "id").and_then(|id| self.place_symbol(id)),
            "delete_symbol" => id_arg(args, "id").and_then(|id| self.delete_symbol(id)),
            "rename_symbol" => self.rename_symbol(args),
            "set_symbol_timing" => self.set_symbol_timing(args),
            "add_camera" => {
                let id = Id(self.project().next_id);
                xenify_ops::camera::add_camera(
                    self.project(),
                    self.comp,
                    id,
                    self.selection
                        .first()
                        .copied()
                        .filter(|_| args.get("follow").and_then(Json::as_bool).unwrap_or(false)),
                )
                .map_err(err)
                .and_then(|cmd| self.exec("Add camera", cmd))
                .map(|_| {
                    self.selection = vec![id];
                    json!(id.0)
                })
            }
            "add_group" => {
                let id = self.next_id();
                let n = self
                    .project()
                    .comp(self.comp)
                    .map_or(0, |c| c.layers.iter().filter(|l| matches!(l.kind, LayerKind::Group)).count())
                    + 1;
                let (w, h) = self.comp_settings().map_or((0.0, 0.0), |s| (s.width as f64, s.height as f64));
                let layer =
                    Layer::new(id, format!("Group {n}"), LayerKind::Group).with_position(Vec2::new(w / 2.0, h / 2.0));
                self.add_layer("New group", layer)
            }
            // Water that rises to fill the canvas, or the selected layer's shape (a title, a logo…).
            "add_water" => (|| -> Result<Json, String> {
                let id = Id(self.project().next_id);
                let cmd = match args.get("fill").and_then(Json::as_str) {
                    Some("layer") => {
                        let target = self.selection.first().copied().ok_or("select the layer to fill first")?;
                        let scene = self.scene();
                        let c = self.project().comp(self.comp).map_err(err)?.clone();
                        let bounds = xenify_anim::world_box(&c, &scene, target, &mut self.text)
                            .ok_or("that layer isn't visible now, so there's nothing to fill")?;
                        xenify_ops::liquid::fill_layer(self.project(), self.comp, id, target, bounds, self.time)
                    }
                    _ => xenify_ops::liquid::fill_screen(self.project(), self.comp, id, self.time),
                }
                .map_err(err)?;
                self.exec("Add water", cmd)?;
                self.selection = vec![id];
                Ok(json!(id.0))
            })(),
            // A camera: whether it's the one used, making it so, and its shake.
            "camera_info" => (|| -> Result<Json, String> {
                let id = id_arg(args, "id")?;
                let active = self
                    .project()
                    .comp(self.comp)
                    .ok()
                    .and_then(|c| xenify_doc::active_camera(&c.layers, self.time))
                    .map(|l| l.id);
                Ok(json!({ "active": active == Some(id) }))
            })(),
            // Every camera's frame on the canvas (what it sees), for drawing and grabbing.
            "camera_frames" => (|| -> Result<Json, String> {
                let c = self.project().comp(self.comp).map_err(err)?;
                let active = xenify_doc::active_camera(&c.layers, self.time).map(|l| l.id);
                Ok(json!(c.layers.iter().filter(|l| matches!(l.kind, LayerKind::Camera(_))).filter_map(|l| {
                    let f = xenify_anim::camera_frame(c, l.id, self.time)?;
                    Some(json!({ "id": l.id.0, "name": l.meta.name, "active": active == Some(l.id), "visible": l.meta.visible, "locked": l.meta.locked,
                        "corners": f.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>() }))
                }).collect::<Vec<_>>()))
            })(),
            "camera_make_active" => (|| -> Result<Json, String> {
                let id = id_arg(args, "id")?;
                match xenify_ops::camera::make_active(self.project(), id).map_err(err)? {
                    Some(cmd) => self.exec("Make camera active", cmd).map(|_| json!(true)),
                    None => Ok(json!(false)),
                }
            })(),
            "camera_shake" => (|| -> Result<Json, String> {
                let id = id_arg(args, "id")?;
                let shake = match args.get("shake") {
                    Some(Json::Bool(true)) => Some(xenify_doc::CameraShake { start: self.time, ..Default::default() }),
                    Some(Json::Bool(false)) | Some(Json::Null) | None => None,
                    Some(v) => Some(serde_json::from_value(v.clone()).map_err(err)?),
                };
                let on = shake.is_some();
                let cmd = xenify_ops::camera::set_shake(self.project(), id, shake).map_err(err)?;
                let key = format!("shake:{}", id.0);
                self.editor
                    .execute_merged(if on { "Camera shake" } else { "Remove camera shake" }, key, cmd)
                    .map_err(err)?;
                Ok(Json::Null)
            })(),
            "look_through" => {
                if let Some(on) = args.get("on").and_then(Json::as_bool) {
                    self.look_through = on;
                }
                let has = self
                    .project()
                    .comp(self.comp)
                    .is_ok_and(|c| c.layers.iter().any(|l| matches!(l.kind, LayerKind::Camera(_))));
                Ok(json!({ "on": self.look_through, "has_camera": has }))
            }
            _ => return None,
        })
    }

    /// Every symbol: id, name, how many copies are placed, and whether it's the one being edited.
    fn symbols(&self) -> Json {
        let p = self.project();
        json!({
            "editing": is_symbol(p, self.comp).then(|| p.comp(self.comp).map(|c| c.settings.name.clone()).ok()).flatten(),
            "list": p.compositions.iter().filter(|c| is_symbol(p, c.id)).map(|c| {
                let uses = p.compositions.iter().flat_map(|o| &o.layers)
                    .filter(|l| matches!(l.kind, LayerKind::Precomp { comp, .. } if comp == c.id)).count();
                json!({ "id": c.id.0, "name": c.settings.name, "uses": uses, "open": c.id == self.comp })
            }).collect::<Vec<_>>(),
        })
    }

    /// Opens composition `id` (a symbol, or a slide) in the canvas.
    pub(crate) fn open_comp(&mut self, id: Id) -> Result<(), String> {
        let c = self.project().comp(id).map_err(err)?;
        let end = c.settings.duration;
        self.comp = id;
        self.selection.clear();
        self.editing = None;
        self.time = self.time.min(end);
        Ok(())
    }

    fn edit_symbol(&mut self, id: Id) -> Result<Json, String> {
        if !is_symbol(self.project(), id) {
            return Err("That isn't a symbol.".into());
        }
        if !is_symbol(self.project(), self.comp) {
            self.symbol_return = Some(self.comp);
        }
        self.open_comp(id)?;
        Ok(self.state())
    }

    /// Back from a symbol to the canvas (or slide) it was opened from.
    fn finish_symbol(&mut self) -> Result<Json, String> {
        let back = self
            .symbol_return
            .take()
            .filter(|id| self.project().comp(*id).is_ok())
            .or_else(|| xenify_ops::slides::slides_or_main(self.project()).first().map(|s| s.comp))
            .ok_or("nowhere to go back to")?;
        self.open_comp(back)?;
        Ok(self.state())
    }

    /// A new symbol from the selected layers (they're replaced by one copy of it), or an empty one.
    fn new_symbol(&mut self) -> Result<Json, String> {
        let name = next_name(self.project());
        if !self.selection.is_empty() {
            let (cmd, _, layer) =
                xenify_ops::precomp::precompose(self.project(), self.comp, &self.selection, &name).map_err(err)?;
            self.exec("Convert to symbol", cmd)?;
            self.selection = vec![layer];
            return Ok(json!(layer.0));
        }
        let id = Id(self.project().next_id);
        let s = self.project().comp(self.comp).map_err(err)?.settings.clone();
        let settings = CompSettings { name, slices: Vec::new(), pauses: Vec::new(), ..s };
        self.exec(
            "New symbol",
            Command::AddComposition { index: None, comp: Composition { id, settings, layers: Vec::new() } },
        )?;
        self.edit_symbol(id)?;
        Ok(json!(id.0))
    }

    /// A copy of symbol `id` in the middle of this canvas.
    fn place_symbol(&mut self, id: Id) -> Result<Json, String> {
        if id == self.comp {
            return Err("A symbol can't contain itself.".into());
        }
        let child = self.project().comp(id).map_err(err)?.clone();
        let s = self.project().comp(self.comp).map_err(err)?.settings.clone();
        let layer_id = Id(self.project().next_id);
        let layer = Layer::precomp(
            layer_id,
            child.settings.name.clone(),
            &child,
            Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0),
        );
        self.exec("Place symbol", Command::AddLayer { comp: self.comp, index: None, layer })?;
        self.selection = vec![layer_id];
        Ok(json!(layer_id.0))
    }

    /// Deletes symbol `id` and every copy of it.
    fn delete_symbol(&mut self, id: Id) -> Result<Json, String> {
        if !is_symbol(self.project(), id) {
            return Err("That isn't a symbol.".into());
        }
        if self.comp == id {
            self.finish_symbol()?;
        }
        let mut cmds: Vec<Command> = self
            .project()
            .compositions
            .iter()
            .flat_map(|c| &c.layers)
            .filter(|l| matches!(l.kind, LayerKind::Precomp { comp, .. } if comp == id))
            .map(|l| Command::RemoveLayer { layer: l.id })
            .collect();
        cmds.push(Command::RemoveComposition { comp: id });
        self.selection.clear();
        self.exec("Delete symbol", Command::batch(cmds))?;
        Ok(Json::Null)
    }

    fn rename_symbol(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("").trim().to_string();
        if name.is_empty() {
            return Ok(Json::Null);
        }
        let mut settings = self.project().comp(id).map_err(err)?.settings.clone();
        settings.name = name;
        self.exec("Rename symbol", Command::SetCompSettings { comp: id, settings })?;
        Ok(Json::Null)
    }

    /// A placed copy's start time on this canvas and whether it loops.
    fn set_symbol_timing(&mut self, args: &Json) -> Result<Json, String> {
        let layer = id_arg(args, "id")?;
        let mut cmds = Vec::new();
        if let Some(start) = f(args, "start") {
            cmds.push(Command::SetPrecompStart { layer, start });
        }
        if let Some(looped) = args.get("loop").and_then(Json::as_bool) {
            cmds.push(Command::SetPrecompLoop { layer, looped });
        }
        self.exec_all("Symbol timing", cmds)?;
        Ok(Json::Null)
    }
}
