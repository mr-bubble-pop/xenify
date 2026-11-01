//! Diagrams: the shape library (flowchart, network, cloud, BPMN), labels
//! inside shapes, and connectors glued between shapes that route
//! themselves around others as the shapes move.

use serde_json::{Value as Json, json};
use xenify_doc::{ConnectorEnd, Id, PortRef, Vec2};
use xenify_ops::Command;
use xenify_ops::library::{Category, Role};

use crate::{Session, err, id_arg};

fn point(v: Option<&Json>) -> Option<Vec2> {
    let v = v?;
    Some(Vec2::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))
}

impl Session {
    pub(crate) fn diagram_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "library" => Ok(library()),
            "add_library_shape" => self.add_library_shape(args),
            "connect" => self.connect(args),
            "set_relation" => (|| -> Result<Json, String> {
                let id = id_arg(args, "id")?;
                let r = args.get("relation").and_then(Json::as_str).ok_or("missing relation")?.to_string();
                self.restyle_relation(id, &r).map(|_| Json::Null)
            })(),
            "relations" => Ok(json!(xenify_ops::library::UML_RELATIONS)),
            "connector_ends" => {
                let id = id_arg(args, "id");
                id.map(|id| {
                    let r = xenify_anim::connector_routes(self.project(), self.comp, self.time)
                        .into_iter()
                        .find(|r| r.id == id);
                    match r.and_then(|r| Some((*r.points.first()?, *r.points.last()?))) {
                        Some((a, b)) => json!([[a.x, a.y], [b.x, b.y]]),
                        None => Json::Null,
                    }
                })
            }
            "reglue" => self.reglue(args),
            // The selected connector's whole route (its bends) and how it's routed, for dragging its segments.
            "connector_route" => id_arg(args, "id").map(|id| {
                let r = xenify_anim::connector_routes(self.project(), self.comp, self.time)
                    .into_iter()
                    .find(|r| r.id == id);
                let style = self.project().layer(id).ok().and_then(|l| l.meta.connector.as_ref().map(|c| c.route));
                match r {
                    Some(r) => {
                        json!({ "points": r.points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(), "route": style })
                    }
                    None => Json::Null,
                }
            }),
            // Where connectors glue to layer `id`: its ports on the canvas.
            "ports" => id_arg(args, "id").map(|id| {
                let scene = self.scene();
                self.project().comp(self.comp).map_or(json!([]), |c| {
                    json!(
                        xenify_anim::diagram::port_points(c, &scene, id)
                            .into_iter()
                            .map(|(p, _)| [p.x, p.y])
                            .collect::<Vec<_>>()
                    )
                })
            }),
            // The spot on a shape's edge near a point (for gluing a connector end there): {at, spot} or null.
            "edge_spot" => id_arg(args, "id").map(|id| {
                let scene = self.scene();
                let p = kurbo::Point::new(
                    args.get("x").and_then(Json::as_f64).unwrap_or(0.0),
                    args.get("y").and_then(Json::as_f64).unwrap_or(0.0),
                );
                let tol = args.get("tolerance").and_then(Json::as_f64).unwrap_or(8.0);
                self.project()
                    .comp(self.comp)
                    .ok()
                    .and_then(|c| xenify_anim::diagram::edge_spot(c, &scene, id, p, tol))
                    .map_or(Json::Null, |(q, f)| json!({ "at": [q.x, q.y], "spot": [f.x, f.y] }))
            }),
            "set_waypoints" => {
                let id = match id_arg(args, "id") {
                    Ok(id) => id,
                    Err(e) => return Some(Err(e)),
                };
                let w: Vec<Vec2> = args
                    .get("waypoints")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(|p| point(Some(p))).collect())
                    .unwrap_or_default();
                let n = args.get("drag").and_then(Json::as_u64).unwrap_or(0);
                xenify_ops::diagram::edit_connector(self.project(), id, |c| c.waypoints = w)
                    .map_err(err)
                    .and_then(|cmd| {
                        self.editor
                            .execute_merged("Move connector segment", format!("waypoints:{}:{n}", id.0), cmd)
                            .map_err(err)
                    })
                    .map(|_| Json::Null)
            }
            "quick_connect" => self.quick_connect(args),
            "set_label" => {
                let text = args.get("text").and_then(Json::as_str).unwrap_or("").to_string();
                // Typing merges into one undo step per label (per editing session: `edit`).
                let edit = args.get("edit").and_then(Json::as_u64).unwrap_or(0);
                id_arg(args, "id")
                    .and_then(|id| {
                        xenify_ops::diagram::set_label_text(self.project(), id, &text).map(|c| (id, c)).map_err(err)
                    })
                    .and_then(|(id, cmd)| {
                        self.editor.execute_merged("Label", format!("label:{}:{edit}", id.0), cmd).map_err(err)
                    })
                    .map(|_| Json::Null)
            }
            _ => return None,
        })
    }

    /// A quick-connect arrow: a copy of shape `id` that way (`dir`: [1, 0], [0, -1]…), joined to it by a connector.
    fn quick_connect(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let dir = point(args.get("dir")).ok_or("missing dir")?;
        let scene = self.scene();
        let c = self.project().comp(self.comp).map_err(err)?.clone();
        let b = xenify_anim::world_box(&c, &scene, id, &mut self.text).ok_or("not on screen now")?;
        let original = self.project().layer(id).map_err(err)?.clone();
        let gap = 80.0;
        let step = Vec2::new(dir.x * (b.width() + gap), dir.y * (b.height() + gap));
        let next = self.project().next_id;
        let (copy_id, conn_id) = (Id(next), Id(next + 1));
        let mut copy = original;
        copy.id = copy_id;
        let pos0 = copy.transform.position.value_at(self.time);
        copy.transform.position = (pos0 + step).into();
        let d = xenify_ops::diagram::settings_of(self.project(), self.comp);
        let conn = xenify_ops::diagram::connector_layer(
            conn_id,
            ConnectorEnd::glued(id, PortRef::Auto, b.center().to_vec2()),
            ConnectorEnd::glued(copy_id, PortRef::Auto, (b.center() + step).to_vec2()),
            &d,
        );
        let comp = self.comp;
        self.exec(
            "Quick connect",
            Command::batch(vec![
                Command::AddLayer { comp, index: None, layer: copy },
                Command::AddLayer { comp, index: None, layer: conn },
            ]),
        )?;
        self.selection = vec![copy_id];
        Ok(json!(copy_id.0))
    }

    /// A library shape (`id` like "flowchart.decision") at `at` (else the middle), labelled with its name.
    fn add_library_shape(&mut self, args: &Json) -> Result<Json, String> {
        let shape = args.get("shape").and_then(Json::as_str).ok_or("missing shape")?;
        // Where it was dropped; else to the right of the selected shape; else the middle.
        let beside = || -> Option<Vec2> {
            let sel = *self.selection.first()?;
            let size = xenify_ops::library::find(shape)?.size;
            let scene = self.scene();
            let c = self.project().comp(self.comp).ok()?.clone();
            let b = xenify_anim::world_box(&c, &scene, sel, &mut xenify_anim::ApproxText)?;
            Some(Vec2::new(b.x1 + 80.0 + size.0 / 2.0, b.center().y))
        };
        let at = match point(args.get("at")) {
            Some(p) => p,
            None => beside().unwrap_or_else(|| {
                self.comp_settings().map_or(Vec2::ZERO, |s| Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0))
            }),
        };
        let label = args.get("label").and_then(Json::as_str);
        let (cmd, id) = xenify_ops::library::insert(self.project(), self.comp, shape, at, None, label).map_err(err)?;
        self.editor.begin_group("Add shape");
        let r = self.exec("Add shape", cmd);
        // Dropped in a pool or lane: it joins it.
        if r.is_ok()
            && let Ok(Some(join)) = xenify_ops::diagram::adopt(self.project(), self.comp, &[id], self.time)
        {
            let _ = self.exec("Add shape", join);
        }
        self.editor.end_group();
        r?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    /// A connector from `a` to `b` (composition space), glued to the layers `from` and `to` when given.
    fn connect(&mut self, args: &Json) -> Result<Json, String> {
        let a = point(args.get("a")).ok_or("missing a")?;
        let b = point(args.get("b")).ok_or("missing b")?;
        // Glued to a layer (at port `<end>_port`, else the best side), or loose.
        let end = |k: &str, p: Vec2| match args.get(k).and_then(Json::as_u64) {
            Some(l) => ConnectorEnd::glued(Id(l), port_arg(args.get(format!("{k}_port"))), p),
            None => ConnectorEnd::free(p),
        };
        let (cmd, id) =
            xenify_ops::diagram::add_connector(self.project(), self.comp, end("from", a), end("to", b)).map_err(err)?;
        let relation = args.get("relation").and_then(Json::as_str).filter(|r| !r.is_empty());
        self.editor.begin_group("Connect");
        let done = self.exec("Connect", cmd).and_then(|_| match relation {
            Some(r) => self.restyle_relation(id, r),
            None => Ok(()),
        });
        self.editor.end_group();
        done?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    /// A connector drawn as a UML relationship (inheritance, dependency…): its arrowheads and dashes.
    fn restyle_relation(&mut self, id: Id, relation: &str) -> Result<(), String> {
        let l = self.project().layer(id).map_err(err)?;
        let mut shape = l.shapes().first().cloned().ok_or("that isn't a line")?;
        let stroke = shape.stroke.as_mut().ok_or("that line has no stroke")?;
        if !xenify_ops::library::style_uml_relation(stroke, relation) {
            return Err(format!("no relationship called {relation}"));
        }
        self.exec("Relationship", Command::SetShape { layer: id, index: 0, shape })
    }
}

impl Session {
    /// Moves a connector's `end` ("from" or "to") to `at`, glued to layer `layer` when given (else loose there).
    fn reglue(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let at = point(args.get("at")).ok_or("missing at")?;
        let to_end = args.get("end").and_then(Json::as_str) == Some("to");
        let end = match args.get("layer").and_then(Json::as_u64).filter(|l| *l != id.0) {
            Some(l) => ConnectorEnd::glued(Id(l), port_arg(args.get("port")), at),
            None => ConnectorEnd::free(at),
        };
        let cmd =
            xenify_ops::diagram::edit_connector(self.project(), id, |c| if to_end { c.to = end } else { c.from = end })
                .map_err(err)?;
        self.exec("Move connector end", cmd)?;
        Ok(Json::Null)
    }
}

/// A port number, a spot on the edge ({"spot": [x, y]}, fractions of the shape's box), or the best side when none.
fn port_arg(v: Option<&Json>) -> PortRef {
    if let Some([x, y]) = v
        .and_then(|v| v.get("spot"))
        .and_then(Json::as_array)
        .and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]))
    {
        return PortRef::Spot { x: x.clamp(0.0, 1.0), y: y.clamp(0.0, 1.0) };
    }
    v.and_then(Json::as_u64).map_or(PortRef::Auto, |i| PortRef::Port(i as u32))
}

/// Every library shape with a small picture of it (SVG path data per part, at 40×40).
fn library() -> Json {
    json!(Category::ALL
        .iter()
        .map(|c| json!({
            "category": c.label(),
            "shapes": xenify_ops::library::all().iter().filter(|s| s.category == *c).map(|s| {
                let k = 36.0 / s.size.0.max(s.size.1);
                let shown: Vec<_> = s.parts(s.size.0 * k, s.size.1 * k).into_iter().filter(|p| p.role != Role::Hit).collect();
                let b = shown.iter().map(|p| kurbo::Shape::bounding_box(&p.path)).reduce(|a, b| a.union(b)).unwrap_or_default();
                let parts: Vec<Json> = shown.iter().map(|p| {
                    json!({ "d": p.path.to_svg(), "role": match p.role { Role::Body => "body", Role::Line => "line", Role::Solid => "solid", Role::Hit => "hit" } })
                }).collect();
                json!({ "id": s.id, "name": s.name, "box": [b.x0, b.y0, b.width(), b.height()], "parts": parts })
            }).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>())
}
