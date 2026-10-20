//! The thin control bar under the menu: Boolean, Align, Stroke and Physics
//! for the current selection, then the current tool's options. The groups always sit in the same places and
//! open as compact dropdowns; buttons grey out when they don't apply.
//!
//! Stroke settings apply to the selected layers' strokes, or — with nothing
//! selected — to new strokes you draw.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{Id, LayerKind, LineCap, LineJoin, PenKind, PenStyle, PropPath, ShapeField, ShapeItem, Value};
use xenify_ops::Command;

pub const MY_PENS_KEY: &str = "my_pens";

pub fn bar(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let layers = app.selected_layers();
        let many = layers.len() > 1;

        ui.add_enabled_ui(many, |ui| {
            ui.menu_button("Boolean ⏷", |ui| boolean(app, ui, &layers))
                .response
                .on_hover_text("Combine 2+ shapes: union, difference, intersection, exclusion")
                .on_disabled_hover_text("Select 2 or more shapes (Shift+click)");
        });
        ui.add_enabled_ui(!layers.is_empty(), |ui| {
            ui.menu_button("Align ⏷", |ui| align(app, ui, &layers))
                .response
                .on_hover_text("Align and distribute the selection")
                .on_disabled_hover_text("Select layers to align");
        });
        ui.separator();
        stroke_controls(app, ui, &layers);
        ui.separator();
        ui.menu_button("Physics ⏷", |ui| physics(app, ui, &layers))
            .response
            .on_hover_text("Joint chains (IK), gravity & collisions with squish, and orbits");
        ui.separator();
        crate::tool_options::bar(app, ui);
    });
}

pub(crate) fn boolean(app: &mut XenifyApp, ui: &mut egui::Ui, layers: &[Id]) {
    use xenify_ops::pathfinder::PathOp;
    ui.set_min_width(220.0);
    for (op, name) in PathOp::ALL {
        let tip = match op {
            PathOp::Unite => "Merge into one shape",
            PathOp::MinusFront => "Cut the shapes above out of the bottom one",
            PathOp::Intersect => "Keep only where they all overlap",
            PathOp::Exclude => "Remove where they overlap",
        };
        if ui.button(name).on_hover_text(tip).clicked() {
            let (project, comp, time) = (app.project().clone(), app.comp, app.time);
            match xenify_ops::pathfinder::combine(&project, comp, layers, op, time) {
                Ok((cmd, id)) => {
                    if app.exec(name, cmd) {
                        app.select_only(Some(id));
                    }
                }
                Err(e) => app.fail(e.to_string()),
            }
            ui.close();
        }
    }
}

pub(crate) fn align(app: &mut XenifyApp, ui: &mut egui::Ui, layers: &[Id]) {
    use xenify_ops::arrange::{self, AlignEdge, AlignTo, Axis};
    let many = layers.len() > 1;
    let id = egui::Id::new("align-to-comp");
    let mut to_comp = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false) || !many;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(many, |ui| ui.radio_value(&mut to_comp, false, "To selection"));
        ui.radio_value(&mut to_comp, true, "To canvas");
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, to_comp));
    let to = if to_comp { AlignTo::Composition } else { AlignTo::Selection };
    let (project, comp, time, auto_key) = (app.project().clone(), app.comp, app.time, app.auto_key);
    let mut action = None;
    {
        let mut measure = app.viewport.measurer();
        egui::Grid::new("align-grid").num_columns(3).show(ui, |ui| {
            for (i, (edge, label)) in [
                (AlignEdge::Left, "⇤ Left"),
                (AlignEdge::HCenter, "↔ Centre"),
                (AlignEdge::Right, "Right ⇥"),
                (AlignEdge::Top, "↑ Top"),
                (AlignEdge::VCenter, "↕ Middle"),
                (AlignEdge::Bottom, "↓ Bottom"),
            ]
            .into_iter()
            .enumerate()
            {
                if ui.button(label).clicked() {
                    action = Some(arrange::align(&project, comp, layers, edge, to, time, auto_key, &mut measure));
                }
                if i == 2 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        ui.add_enabled_ui(layers.len() >= 3, |ui| {
            ui.horizontal(|ui| {
                ui.label("Distribute");
                for (axis, label) in [(Axis::Horizontal, "Horizontally"), (Axis::Vertical, "Vertically")] {
                    if ui.button(label).on_disabled_hover_text("Select 3 or more layers").clicked() {
                        action = Some(arrange::distribute(&project, comp, layers, axis, time, auto_key, &mut measure));
                    }
                }
            });
        });
    }
    match action {
        Some(Ok(Some(cmd))) => {
            app.exec("Arrange", cmd);
        }
        Some(Ok(None)) => app.status = "Already aligned.".into(),
        Some(Err(e)) => app.fail(e.to_string()),
        None => {}
    }
}

/// (layer, shape index) of every stroked shape in the selection.
fn stroked(app: &XenifyApp, layers: &[Id]) -> Vec<(Id, usize, ShapeItem)> {
    let mut out = Vec::new();
    for id in layers {
        let Ok(l) = app.project().layer(*id) else { continue };
        let members = std::iter::once(*id).chain(xenify_anim::descendants(app.comp(), *id));
        for m in members {
            let Ok(l) = app.project().layer(m) else { continue };
            if let LayerKind::Shape { shapes } = &l.kind {
                for (i, s) in shapes.iter().enumerate() {
                    if s.stroke.is_some() {
                        out.push((m, i, s.clone()));
                    }
                }
            }
        }
        let _ = l;
    }
    out
}

/// The stroke settings shown: the first selected stroke's, else the defaults for new strokes.
struct StrokeState {
    width: f64,
    cap: LineCap,
    roundness: f64,
    join: LineJoin,
    pen: PenStyle,
}

fn stroke_controls(app: &mut XenifyApp, ui: &mut egui::Ui, layers: &[Id]) {
    let targets = stroked(app, layers);
    let editing_selection = !targets.is_empty();
    let t = app.time;
    let mut st = match targets.first().and_then(|(_, _, s)| s.stroke.clone()) {
        Some(s) => StrokeState {
            width: s.width.value_at(t),
            cap: s.cap,
            roundness: s.cap_roundness,
            join: s.join,
            pen: s.style,
        },
        None => StrokeState {
            width: app.paint.stroke_width,
            cap: app.paint.cap,
            roundness: app.paint.cap_roundness,
            join: app.paint.join,
            pen: app.paint.pen.clone(),
        },
    };
    let before = (st.width, st.cap, st.roundness, st.join, st.pen.clone());
    let mut begin = false;

    ui.label(RichText::new("Stroke").weak()).on_hover_text(if editing_selection {
        "Editing the selected strokes"
    } else if !layers.is_empty() {
        "The selection has no strokes — add one with the Stroke checkbox in Properties. These settings are for new strokes."
    } else {
        "Settings for new strokes (select shapes to edit theirs)"
    });
    // The Pencil and Brush have their own width (in their tool options).
    if editing_selection || !matches!(app.tool, crate::app::Tool::Pencil | crate::app::Tool::Brush) {
        let r = ui.add(egui::DragValue::new(&mut st.width).range(0.0..=500.0).speed(0.2).suffix(" px").max_decimals(1));
        begin |= r.drag_started() || r.gained_focus();
        r.on_hover_text("Stroke width");
    }

    // End caps, including a rounded square with adjustable roundness.
    let cap_label = match (st.cap, st.roundness > 0.0) {
        (LineCap::Butt, _) => "Flat".to_string(),
        (LineCap::Round, _) => "Round".to_string(),
        (LineCap::Square, false) => "Square".to_string(),
        (LineCap::Square, true) => format!("Rounded {:.0}%", st.roundness * 100.0),
    };
    ui.menu_button(format!("Cap: {cap_label} ⏷"), |ui| {
        ui.set_min_width(220.0);
        ui.label(RichText::new("End caps").strong());
        for (cap, r, label) in [
            (LineCap::Butt, 0.0, "Flat"),
            (LineCap::Square, 0.0, "Square (extends past the end)"),
            (LineCap::Round, 0.0, "Round"),
            (LineCap::Square, 0.4, "Rounded rectangle"),
            (LineCap::Square, 0.8, "Almost round"),
        ] {
            let on = st.cap == cap && (st.roundness - r).abs() < 1e-6;
            if ui.selectable_label(on, label).clicked() {
                (st.cap, st.roundness) = (cap, r);
                begin = true;
            }
        }
        ui.add_enabled_ui(st.cap == LineCap::Square, |ui| {
            let mut pct = st.roundness * 100.0;
            let r = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").text("roundness"));
            begin |= r.drag_started();
            if r.changed() {
                st.roundness = pct / 100.0;
            }
        });
        ui.separator();
        ui.label(RichText::new("Corners").strong());
        ui.horizontal(|ui| {
            for (j, label) in [(LineJoin::Miter, "Sharp"), (LineJoin::Round, "Round"), (LineJoin::Bevel, "Bevel")] {
                if ui.selectable_label(st.join == j, label).clicked() {
                    st.join = j;
                    begin = true;
                }
            }
        });
    });

    // Pen type and its settings.
    ui.menu_button(format!("Pen: {} ⏷", pen_name(&st.pen)), |ui| {
        ui.set_min_width(260.0);
        pen_menu(app, ui, &mut st.pen, &mut begin);
    });

    let after = (st.width, st.cap, st.roundness, st.join, st.pen.clone());
    if after == before {
        return;
    }
    if begin {
        app.new_gesture();
    }
    if editing_selection {
        let width_changed = (st.width - before.0).abs() > 1e-9;
        let mut cmds = Vec::new();
        for (layer, index, shape) in &targets {
            let mut s = shape.clone();
            if let Some(k) = s.stroke.as_mut() {
                k.cap = st.cap;
                k.cap_roundness = st.roundness;
                k.join = st.join;
                k.style = st.pen.clone();
            }
            cmds.push(Command::SetShape { layer: *layer, index: *index, shape: s });
            if width_changed {
                let path = PropPath::shape(*index, ShapeField::StrokeWidth);
                // Honours animation / auto-key like any property edit.
                if let Ok(c) =
                    Command::edit_value(app.project(), *layer, path, t, Value::Scalar(st.width), app.auto_key)
                {
                    cmds.push(c);
                }
            }
        }
        app.exec_merged("Stroke style", Command::batch(cmds));
    }
    // Also remembered for new strokes.
    let p = &mut app.paint;
    (p.stroke_width, p.cap, p.cap_roundness, p.join, p.pen) = (st.width, st.cap, st.roundness, st.join, st.pen);
}

/// Uses `pen` for the selected strokes (or new strokes when none are
/// selected). One undo step.
pub fn apply_pen(app: &mut XenifyApp, pen: PenStyle, merged: bool) {
    let layers = app.selected_layers();
    let targets = stroked(app, &layers);
    if !targets.is_empty() {
        let cmds = targets
            .iter()
            .map(|(layer, index, shape)| {
                let mut s = shape.clone();
                if let Some(k) = s.stroke.as_mut() {
                    k.style = pen.clone();
                }
                Command::SetShape { layer: *layer, index: *index, shape: s }
            })
            .collect();
        if merged {
            app.exec_merged("Pen settings", Command::batch(cmds));
        } else {
            app.exec("Pen", Command::batch(cmds));
            app.status = format!("Pen: {} on the selected strokes.", pen_name(&pen));
        }
    } else if !merged {
        app.status = format!("Pen: {} for new strokes (select shapes to change theirs).", pen_name(&pen));
    }
    app.paint.pen = pen;
}

/// The pen the selection uses (or new strokes will use).
pub fn current_pen(app: &XenifyApp) -> PenStyle {
    let layers = app.selected_layers();
    stroked(app, &layers)
        .first()
        .and_then(|(_, _, s)| s.stroke.as_ref().map(|k| k.style.clone()))
        .unwrap_or_else(|| app.paint.pen.clone())
}

pub fn pen_settings(app: &mut XenifyApp, ui: &mut egui::Ui, pen: &mut PenStyle, begin: &mut bool) {
    pen_menu_settings(app, ui, pen, begin);
}

pub fn pen_name(p: &PenStyle) -> String {
    if p.name.is_empty() { p.kind.label().to_string() } else { p.name.clone() }
}

/// The pen list, the current pen's settings, and saved custom pens.
fn pen_menu(app: &mut XenifyApp, ui: &mut egui::Ui, pen: &mut PenStyle, begin: &mut bool) {
    ui.label(RichText::new("Pens").strong());
    egui::Grid::new("pen-grid").num_columns(3).spacing([4.0, 2.0]).show(ui, |ui| {
        for (i, kind) in PenKind::ALL.into_iter().enumerate() {
            if ui.selectable_label(pen.kind == kind && pen.name.is_empty(), kind.label()).clicked() {
                *pen = PenStyle { seed: pen.seed, ..PenStyle::preset(kind) };
                *begin = true;
            }
            if i % 3 == 2 {
                ui.end_row();
            }
        }
    });
    if !app.my_pens.is_empty() {
        ui.separator();
        ui.label(RichText::new("My pens").strong());
        let mut remove = None;
        for (i, p) in app.my_pens.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.selectable_label(*pen == *p, &p.name).on_hover_text(p.kind.label()).clicked() {
                    *pen = p.clone();
                    *begin = true;
                }
                if ui.small_button("Delete").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            app.my_pens.remove(i);
        }
    }
    ui.separator();
    pen_menu_settings(app, ui, pen, begin);
}

/// The current pen's sliders, and saving it as a named pen.
fn pen_menu_settings(app: &mut XenifyApp, ui: &mut egui::Ui, pen: &mut PenStyle, begin: &mut bool) {
    ui.label(RichText::new(format!("{} settings", pen.kind.label())).strong());
    let uses = pen.kind.uses();
    let mut slider =
        |ui: &mut egui::Ui, on: bool, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, tip: &str| {
            if !on {
                return;
            }
            let r = ui.add(egui::Slider::new(v, range).text(label)).on_hover_text(tip);
            *begin |= r.drag_started();
        };
    let mut p = pen.clone();
    slider(ui, uses.taper, "taper start", &mut p.taper_start, 0.0..=1.0, "How much the start thins out");
    slider(ui, uses.taper, "taper end", &mut p.taper_end, 0.0..=1.0, "How much the end thins out");
    slider(
        ui,
        uses.thinning,
        "thinning",
        &mut p.thinning,
        0.0..=1.0,
        "Thinnest part (calligraphy hairlines, brush pressure)",
    );
    slider(ui, uses.angle, "nib angle", &mut p.angle, -90.0..=90.0, "Angle of the flat nib, in degrees");
    slider(ui, uses.roughness, "roughness", &mut p.roughness, 0.0..=1.0, "Wobble and unevenness");
    slider(ui, uses.grain, "grain", &mut p.grain, 0.0..=1.0, "Texture density: more = fuller coverage");
    slider(ui, uses.dash, "dash", &mut p.dash, 0.0..=10.0, "Dash length, in stroke widths");
    slider(ui, uses.dash, "gap", &mut p.gap, 0.1..=10.0, "Gap length, in stroke widths");
    slider(ui, true, "flow", &mut p.flow, 0.05..=1.0, "Opacity of the marks");
    if uses.count {
        let r = ui
            .add(egui::Slider::new(&mut p.count, 1..=12).text("count"))
            .on_hover_text("Bristles, passes, layers or lines");
        *begin |= r.drag_started();
    }
    ui.horizontal(|ui| {
        if ui.button("Shuffle").on_hover_text("A different random pattern").clicked() {
            p.seed = p.seed.wrapping_add(1);
            *begin = true;
        }
        if ui.button("Reset").on_hover_text("Back to this pen's standard settings").clicked() {
            p = PenStyle::preset(p.kind);
            *begin = true;
        }
    });
    if p != *pen {
        if p.name == pen.name && !pen.name.is_empty() {
            // Tweaking a saved pen makes it a new, unsaved variation.
            p.name.clear();
        }
        *pen = p;
    }
    // Save the current settings as a named custom pen.
    let id = egui::Id::new("new-pen-name");
    let mut name: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut name).hint_text("Name").desired_width(120.0));
        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Save as my pen")).clicked() {
            let saved = PenStyle { name: name.trim().to_string(), ..pen.clone() };
            app.my_pens.retain(|p| p.name != saved.name);
            app.my_pens.push(saved.clone());
            *pen = saved;
            *begin = true;
            name.clear();
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, name));
}

fn physics(app: &mut XenifyApp, ui: &mut egui::Ui, _layers: &[Id]) {
    ui.set_min_width(300.0);
    // Chains and orbits follow the order you clicked the layers in.
    let ordered = app.selection_in_click_order();
    let layers = ordered.as_slice();
    let (project, comp, time) = (app.project().clone(), app.comp, app.time);
    ui.label(RichText::new("Joint chain (IK)").strong());
    ui.weak(
        "Pose like a puppet: dragging the tip bends its parent joints. Put each joint's anchor point where it pivots.",
    );
    if ui
        .add_enabled(layers.len() >= 2, egui::Button::new("Make joint chain from selection"))
        .on_hover_text("Select from the root to the tip (e.g. upper arm, forearm, hand), in that order")
        .on_disabled_hover_text("Select 2+ layers, root first")
        .clicked()
    {
        match xenify_ops::rig::make_joint_chain(&project, comp, layers, time) {
            Ok(cmd) => {
                if app.exec("Make joint chain", cmd) {
                    // The tip is what you drag to pose.
                    app.select_only(layers.last().copied());
                    app.status = "Joint chain made — drag the selected tip to bend the chain.".into();
                }
            }
            Err(e) => app.fail(e.to_string()),
        }
        ui.close();
    }
    if let [one] = layers
        && let Ok(l) = project.layer(*one)
        && l.meta.parent.is_some()
    {
        let mut n = l.meta.rig.ik_chain;
        ui.horizontal(|ui| {
            ui.label("This layer bends");
            ui.add(egui::DragValue::new(&mut n).range(0..=12));
            ui.label("parent joints when dragged");
        });
        if n != l.meta.rig.ik_chain {
            let mut meta = l.meta.clone();
            meta.rig.ik_chain = n;
            app.exec("IK chain length", Command::SetLayerMeta { layer: l.id, meta });
        }
    }
    ui.separator();
    simulation(app, ui, layers);
    ui.separator();
    orbit(app, ui, layers);
    ui.separator();
    let rigged = layers.iter().any(|id| project.layer(*id).is_ok_and(|l| !l.meta.rig.is_off()));
    if ui
        .add_enabled(rigged, egui::Button::new("Unlink selection"))
        .on_hover_text("Remove the IK chain (parenting stays)")
        .clicked()
    {
        if let Ok(cmd) = xenify_ops::rig::unlink(&project, layers) {
            app.exec("Unlink rig", cmd);
        }
        ui.close();
    }
}

fn simulation(app: &mut XenifyApp, ui: &mut egui::Ui, layers: &[Id]) {
    ui.label(RichText::new("Gravity & collisions").strong());
    ui.weak("Simulates the selected layers from the playhead to the end and writes the motion as keyframes.");
    let p = &mut app.physics;
    egui::Grid::new("physics-grid").num_columns(2).show(ui, |ui| {
        ui.label("Gravity");
        ui.add(egui::Slider::new(&mut p.gravity, 0.0..=8000.0).suffix(" px/s²"))
            .on_hover_text("0 = floating (for collisions only)");
        ui.end_row();
        ui.label("Bounciness");
        ui.add(egui::Slider::new(&mut p.bounce, 0.0..=1.0));
        ui.end_row();
        ui.label("Friction");
        ui.add(egui::Slider::new(&mut p.friction, 0.0..=1.0));
        ui.end_row();
        ui.label("Collide with");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut p.floor, "floor");
            ui.checkbox(&mut p.walls, "sides");
            ui.checkbox(&mut p.ceiling, "top");
            ui.checkbox(&mut p.obstacles, "other layers")
                .on_hover_text("Other visible layers are obstacles (they can be animated)");
        });
        ui.end_row();
        ui.label("Squish");
        ui.add(egui::Slider::new(&mut p.squish, 0.0..=1.0))
            .on_hover_text("How much impacts squash, at the point of contact");
        ui.end_row();
        ui.label("Who squishes");
        ui.horizontal(|ui| {
            ui.checkbox(&mut p.squish_moving, "the moving layers");
            ui.checkbox(&mut p.squish_hit, "what they hit").on_hover_text("Obstacles dent where they're hit");
        });
        ui.end_row();
    });
    let settings = p.clone();
    if ui
        .add_enabled(!layers.is_empty(), egui::Button::new("Simulate selection"))
        .on_hover_text(
            "Replaces their motion from the playhead on. Layers already moving keep their speed (throw them!).",
        )
        .on_disabled_hover_text("Select the layers that should fall and bounce")
        .clicked()
    {
        let (project, comp, time) = (app.project().clone(), app.comp, app.time);
        let result = {
            let mut measure = app.viewport.measurer();
            xenify_ops::physics::simulate(&project, comp, layers, time, &settings, &mut measure)
        };
        match result {
            Ok(cmd) => {
                if app.exec("Simulate physics", cmd) {
                    app.status = "Simulated — press Space to play. Ctrl+Z undoes it.".into();
                }
            }
            Err(e) => app.fail(e.to_string()),
        }
        ui.close();
    }
}

fn orbit(app: &mut XenifyApp, ui: &mut egui::Ui, layers: &[Id]) {
    ui.label(RichText::new("Orbit").strong());
    ui.weak("Click the centre first, then the layers that circle it (the centre can move).");
    let o = &mut app.orbit;
    egui::Grid::new("orbit-grid").num_columns(2).show(ui, |ui| {
        ui.label("Speed");
        ui.add(egui::Slider::new(&mut o.speed, 0.02..=3.0).suffix(" turns/s").logarithmic(true));
        ui.end_row();
        ui.label("Flatten");
        ui.add(egui::Slider::new(&mut o.flatten, 0.1..=1.0))
            .on_hover_text("Squash the circle into an ellipse, like a tilted 3D orbit");
        ui.end_row();
        ui.label("");
        ui.horizontal(|ui| {
            ui.checkbox(&mut o.clockwise, "clockwise");
            ui.checkbox(&mut o.kepler, "outer ones slower").on_hover_text("Like planets: farther orbits take longer");
        });
        ui.end_row();
    });
    let settings = o.clone();
    if ui
        .add_enabled(layers.len() >= 2, egui::Button::new("Orbit around the first selected"))
        .on_disabled_hover_text("Select the centre, then Shift+click the orbiting layers")
        .clicked()
    {
        let project = app.project().clone();
        match xenify_ops::physics::orbit(&project, app.comp, layers[0], &layers[1..], app.time, &settings) {
            Ok(cmd) => {
                if app.exec("Orbit", cmd) {
                    app.status = "Orbiting — press Space to play.".into();
                }
            }
            Err(e) => app.fail(e.to_string()),
        }
        ui.close();
    }
}
