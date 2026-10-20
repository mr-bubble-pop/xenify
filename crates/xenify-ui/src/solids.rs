//! 3D objects: adding primitives, extruding shapes, the properties editor
//! and the orbit tool.

use crate::app::{XenifyApp, Tool};
use xenify_doc::{Color, Id, Layer, LayerKind, PropPath, Solid, SolidField, SolidShape, Value, Vec2};
use xenify_ops::Command;

/// Layer ▸ 3D.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    for shape in SolidShape::primitives() {
        if ui.button(format!("New {}", shape.label())).clicked() {
            add_solid(app, shape);
            ui.close();
        }
    }
    if ui
        .button("Import 3D Model…")
        .on_hover_text("STL, OBJ, glTF / GLB or 3MF (export one of these from CAD programs)")
        .clicked()
    {
        app.dialog = Some(crate::dialogs::Dialog::import(app));
        ui.close();
    }
    ui.separator();
    let can_extrude = app.selected.and_then(|id| app.project().layer(id).ok()).is_some_and(|l| !l.shapes().is_empty());
    if ui
        .add_enabled(can_extrude, egui::Button::new("Extrude Selected Shape"))
        .on_hover_text("Turns the selected shape layer into a 3D object with depth")
        .clicked()
    {
        extrude_selection(app);
        ui.close();
    }
}

/// Adds a primitive in the middle of the frame, in the current fill colour.
pub fn add_solid(app: &mut XenifyApp, shape: SolidShape) {
    let s = &app.comp().settings;
    let centre = Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0);
    let size = (s.width.min(s.height) as f64 / 3.0).round();
    let color = app.paint.fill.or(app.paint.stroke).unwrap_or(Color::from_hex("#118ab2").unwrap_or(Color::WHITE));
    let depth = if matches!(shape, SolidShape::Cylinder | SolidShape::Cone) { size * 1.2 } else { size };
    let id = Id(app.project().next_id);
    let name = shape.label().to_string();
    let layer = Layer::new(id, name.clone(), LayerKind::Solid(Box::new(Solid::new(shape, size, depth, color))))
        .with_position(centre);
    if app.exec(&format!("Add {name}"), Command::AddLayer { comp: app.comp, index: None, layer }) {
        app.select_only(Some(id));
        app.set_tool(Tool::Orbit);
        app.status = format!("Added a 3D {name}. Drag it to turn it; rotation X/Y/Z are keyframeable in Properties.");
    }
}

/// Replaces the selected shape layer with an extruded 3D version of its outline.
pub fn extrude_selection(app: &mut XenifyApp) {
    let Some(layer) = app.selected.and_then(|id| app.project().layer(id).ok()).cloned() else { return };
    let t = app.time;
    let mut path = kurbo::BezPath::new();
    let mut fill_rule = xenify_doc::FillRule::NonZero;
    let mut color = None;
    for s in layer.shapes() {
        path.extend(xenify_anim::geometry_path(&s.geometry, t).iter());
        fill_rule = s.fill_rule;
        color =
            color.or(s.fill.as_ref().map(|f| f.color.value_at(t))).or(s.stroke.as_ref().map(|s| s.color.value_at(t)));
    }
    if path.elements().is_empty() {
        app.status = "Nothing to extrude.".into();
        return;
    }
    let b = kurbo::Shape::bounding_box(&path);
    let depth = (b.width().max(b.height()) * 0.3).round().max(1.0);
    let mut solid = Solid::new(SolidShape::Extrude { path, fill_rule }, 0.0, depth, color.unwrap_or(Color::WHITE));
    solid.rot_x = 15.0.into();
    solid.rot_y = (-25.0).into();
    let mut new = layer.clone();
    new.kind = LayerKind::Solid(Box::new(solid));
    let index = app.comp().layer_index(layer.id);
    let cmd = Command::batch(vec![
        Command::RemoveLayer { layer: layer.id },
        Command::AddLayer { comp: app.comp, index, layer: new },
    ]);
    if app.exec("Extrude", cmd) {
        app.select_only(Some(layer.id));
        app.set_tool(Tool::Orbit);
        app.status = "Extruded into 3D. Drag with the orbit tool (O) to turn it.".into();
    }
}

/// The 3D section of the properties panel (shape and perspective; the
/// animatable values are ordinary property rows).
pub fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, solid: &Solid) {
    if let SolidShape::Mesh(mesh) = &solid.shape {
        model_info(app, ui, layer, solid, mesh);
        if mesh.parts.len() > 1 {
            parts_editor(app, ui, layer, solid, mesh);
        }
    } else {
        shape_picker(app, ui, layer, solid);
    }
    let mut persp = solid.perspective;
    ui.horizontal(|ui| {
        ui.label("Perspective");
        let r = ui.add(egui::DragValue::new(&mut persp).range(0.0..=1.0).speed(0.005).fixed_decimals(2));
        if r.drag_started() {
            app.new_gesture();
        }
        if r.changed() {
            app.exec_merged(
                "Change perspective",
                Command::SetSolid { layer: layer.id, solid: Box::new(Solid { perspective: persp, ..solid.clone() }) },
            );
        }
    });
    if let SolidShape::Torus { thickness } = solid.shape {
        let mut th = thickness;
        ui.horizontal(|ui| {
            ui.label("Thickness");
            let r = ui.add(egui::DragValue::new(&mut th).range(0.05..=0.5).speed(0.002).fixed_decimals(2));
            if r.drag_started() {
                app.new_gesture();
            }
            if r.changed() {
                let solid = Solid { shape: SolidShape::Torus { thickness: th }, ..solid.clone() };
                app.exec_merged("Change thickness", Command::SetSolid { layer: layer.id, solid: Box::new(solid) });
            }
        });
    }
}

/// An imported model's name, size and colour choice.
fn model_info(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, solid: &Solid, mesh: &xenify_doc::Mesh) {
    let parts = match mesh.parts.len() {
        1 => String::new(),
        n => format!(", {n} parts"),
    };
    ui.label(format!("Model: {}", mesh.name))
        .on_hover_text(format!("{} faces{parts}. Size sets its largest dimension in pixels.", mesh.polygon_count()));
    if mesh.has_colors() {
        let mut own = mesh.own_colors;
        if ui
            .checkbox(&mut own, "Use the model's colours")
            .on_hover_text("Off: every part takes the Colour property (which can be animated)")
            .changed()
        {
            let shape = SolidShape::Mesh(Box::new(xenify_doc::Mesh { own_colors: own, ..mesh.clone() }));
            app.exec(
                "Model colours",
                Command::SetSolid { layer: layer.id, solid: Box::new(Solid { shape, ..solid.clone() }) },
            );
        }
    }
}

/// A multi-part model's parts: show/hide each, and move or turn it on its
/// own (keyframeable), or explode them all apart.
fn parts_editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, solid: &Solid, mesh: &xenify_doc::Mesh) {
    use xenify_doc::{PartField, PartMotion};
    egui::CollapsingHeader::new(format!("Parts ({})", mesh.parts.len())).id_salt(("parts", layer.id)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .button("Explode")
                .on_hover_text(
                    "Animates every part outward from the middle over the next second (keyframes from the playhead)",
                )
                .clicked()
            {
                explode(app, layer, solid, mesh);
            }
            if !solid.parts.is_empty()
                && ui.button("Reset parts").on_hover_text("Put every part back where it was").clicked()
            {
                let solid = Solid { parts: Vec::new(), ..solid.clone() };
                app.exec("Reset parts", Command::SetSolid { layer: layer.id, solid: Box::new(solid) });
            }
        });
        for (i, part) in mesh.parts.iter().enumerate() {
            ui.push_id(("part", i), |ui| {
                let motion = solid.parts.get(i);
                let name = if part.name.is_empty() { format!("Part {}", i + 1) } else { part.name.clone() };
                ui.horizontal(|ui| {
                    let mut visible = motion.is_none_or(|m| m.visible);
                    if ui.checkbox(&mut visible, name).on_hover_text("Show / hide this part").changed() {
                        let mut s = solid.clone();
                        if s.parts.len() <= i {
                            s.parts.resize(i + 1, PartMotion::default());
                        }
                        s.parts[i].visible = visible;
                        app.exec("Show part", Command::SetSolid { layer: layer.id, solid: Box::new(s) });
                    }
                    if motion.is_none()
                        && ui.small_button("Move…").on_hover_text("Move or turn this part on its own").clicked()
                    {
                        let mut s = solid.clone();
                        s.parts.resize(i + 1, PartMotion::default());
                        app.exec("Move part", Command::SetSolid { layer: layer.id, solid: Box::new(s) });
                    }
                });
                if motion.is_some() {
                    egui::Grid::new(("part-props", layer.id, i)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                        for field in PartField::ALL {
                            crate::panels::prop_row(
                                app,
                                ui,
                                layer,
                                xenify_doc::PropPath::SolidPart { index: i, field },
                            );
                        }
                    });
                }
            });
        }
    });
}

/// Keyframes every part from where it is now to pushed out from the model's
/// middle, one second later (an exploded view).
fn explode(app: &mut XenifyApp, layer: &Layer, solid: &Solid, mesh: &xenify_doc::Mesh) {
    use xenify_doc::{Keyframe, PartMotion, Property};
    let size = solid.size.value_at(app.time);
    let (t0, t1) = (app.time, app.time + 1.0);
    let centre = mesh.bounds().map_or([0.0; 3], |(lo, hi)| [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0));
    let mut s = solid.clone();
    s.parts.resize(mesh.parts.len().max(s.parts.len()), PartMotion::default());
    for (i, part) in mesh.parts.iter().enumerate() {
        let Some((lo, hi)) = part.bounds() else { continue };
        let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0 - centre[k]);
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        // Parts at the very middle go straight up.
        let dir = if len < 1e-6 { [0.0, -1.0, 0.0] } else { c.map(|v| v / len) };
        let push = size * 0.35;
        let m = &mut s.parts[i];
        let (o0, d0) = (m.offset.value_at(t0), m.depth.value_at(t0));
        fn key<T>(time: f64, value: T) -> Keyframe<T> {
            Keyframe { time, value, easing: xenify_doc::Easing::EASE_IN_OUT }
        }
        m.offset = Property::from_keyframes(vec![
            key(t0, o0),
            key(t1, o0 + xenify_doc::Vec2::new(dir[0] * push, dir[1] * push)),
        ])
        .unwrap_or(m.offset.clone());
        m.depth = Property::from_keyframes(vec![key(t0, d0), key(t1, d0 + dir[2] * push)]).unwrap_or(m.depth.clone());
    }
    if app.exec("Explode parts", Command::SetSolid { layer: layer.id, solid: Box::new(s) }) {
        app.status = "Exploded the parts over one second from the playhead — play to see it; Undo removes it.".into();
    }
}

fn shape_picker(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, solid: &Solid) {
    ui.horizontal(|ui| {
        ui.label("Shape");
        let mut chosen = None;
        egui::ComboBox::from_id_salt(("solid-shape", layer.id)).selected_text(solid.shape.label()).show_ui(ui, |ui| {
            for shape in SolidShape::primitives() {
                if ui.selectable_label(shape.label() == solid.shape.label(), shape.label()).clicked() {
                    chosen = Some(shape);
                }
            }
        });
        if let Some(shape) = chosen
            && shape.label() != solid.shape.label()
        {
            app.exec(
                "Change 3D shape",
                Command::SetSolid { layer: layer.id, solid: Box::new(Solid { shape, ..solid.clone() }) },
            );
        }
    });
}

impl crate::viewport::Viewport {
    /// Orbit tool: dragging turns the selected (or clicked) 3D object.
    pub(crate) fn orbit_drag(app: &mut XenifyApp, id: Id, start: (f64, f64), delta: kurbo::Vec2, shift: bool) {
        let (mut dx, mut dy) = (delta.x, delta.y);
        if shift {
            if dx.abs() > dy.abs() { dy = 0.0 } else { dx = 0.0 }
        }
        // Like grabbing the front face: drag down to see more of the top, right to see the left side.
        let y = start.1 - dx * 0.5;
        let x = start.0 + dy * 0.5;
        let cmds: Vec<Command> = [(SolidField::RotX, x), (SolidField::RotY, y)]
            .into_iter()
            .filter_map(|(f, v)| {
                Command::edit_value(app.project(), id, PropPath::Solid(f), app.time, Value::Scalar(v), app.auto_key)
                    .ok()
            })
            .collect();
        app.exec_merged("Turn 3D object", Command::batch(cmds));
        app.status = format!("X {x:.0}°  Y {y:.0}°");
    }
}
