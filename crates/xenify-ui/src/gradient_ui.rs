//! The colour panel's paint editor for a shape's fill or stroke: solid,
//! linear or radial; a gradient bar with draggable colour stops (the picker
//! edits the chosen one); and where the gradient starts and ends.

use crate::app::XenifyApp;
use crate::color_picker;
use egui::{Color32, Sense, Stroke, StrokeKind, pos2, vec2};
use xenify_doc::{Color, Gradient, GradientKind, GradientStop, Id, Layer, PropPath, ShapeField, ShapeItem, Value};
use xenify_ops::Command;

/// The paint editor for the selected shape's fill (or stroke). Returns true
/// when it shows a gradient (so the caller doesn't also show a solid picker).
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, stroke: bool) -> bool {
    let what = if stroke { "stroke" } else { "fill" };
    let found = app.selected.and_then(|id| {
        let layer = app.project().layer(id).ok()?.clone();
        let index = layer.shapes().iter().position(|s| if stroke { s.stroke.is_some() } else { s.fill.is_some() })?;
        Some((layer, index))
    });
    let Some((layer, index)) = found else {
        ui.weak(format!(
            "Select a shape with a {what} to give it a gradient, or drag across one with the Gradient tool (G)."
        ));
        return false;
    };
    let shape = layer.shapes()[index].clone();
    editor(app, ui, &layer, index, &shape, stroke, false);
    let Ok(layer) = app.project().layer(layer.id) else { return false };
    let Some(shape) = layer.shapes().get(index) else { return false };
    if stroke {
        shape.stroke.as_ref().is_some_and(|s| s.gradient.is_some())
    } else {
        shape.fill.as_ref().is_some_and(|f| f.gradient.is_some())
    }
}

/// A new gradient from `base`: the colour, then a darker shade.
pub fn default_stops(base: Color) -> Vec<GradientStop> {
    vec![
        GradientStop { offset: 0.0, color: base },
        GradientStop { offset: 1.0, color: Color::rgba(base.r * 0.35, base.g * 0.35, base.b * 0.35, base.a) },
    ]
}

/// Solid / linear / radial, the stops and the ends. `compact` (Properties)
/// shows the chosen stop's colour as a swatch that opens the picker.
pub fn editor(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    layer: &Layer,
    index: usize,
    shape: &ShapeItem,
    stroke: bool,
    compact: bool,
) {
    let (base_colour, existing) = if stroke {
        let s = shape.stroke.as_ref().unwrap();
        (&s.color, s.gradient.as_ref())
    } else {
        let f = shape.fill.as_ref().unwrap();
        (&f.color, f.gradient.as_ref())
    };
    let base = base_colour.value_at(app.time);
    let bounds = kurbo::Shape::bounding_box(&xenify_anim::geometry_path(&shape.geometry, app.time));
    // Edits the gradient's value at the playhead (keyframed if it's animated).
    let current = existing.map(|g| g.value_at(app.time));
    let mut gradient = current.clone();
    let mut toggled = false;

    ui.push_id(("grad", stroke), |ui| {
        ui.horizontal(|ui| {
            ui.label(if stroke { "Stroke paint" } else { "Fill paint" });
            let now = gradient.as_ref().map(|g| g.kind);
            let mut want = now;
            ui.selectable_value(&mut want, None, "Solid").on_hover_text("One colour");
            ui.selectable_value(&mut want, Some(GradientKind::Linear), "Linear")
                .on_hover_text("Colours blend along a line from the start to the end");
            ui.selectable_value(&mut want, Some(GradientKind::Radial), "Radial")
                .on_hover_text("Colours blend outwards in circles from the centre");
            if want != now {
                match (&mut gradient, want) {
                    (_, None) => {
                        toggled = true;
                        gradient = None;
                    }
                    (Some(g), Some(kind)) => *g = Gradient::across(bounds, kind, g.angle_deg(), g.stops.clone()),
                    (None, Some(kind)) => {
                        toggled = true;
                        gradient = Some(Gradient::across(bounds, kind, 90.0, default_stops(base)));
                    }
                }
            }
        });
        let Some(g) = &mut gradient else { return };

        // The stops: the bar, then the chosen stop's position and colour.
        let sel_id = ui.id().with(("stop", layer.id, index));
        let mut sel: usize = ui.data(|d| d.get_temp(sel_id)).unwrap_or(0);
        sel = sel.min(g.stops.len() - 1);
        stop_bar(ui, g, &mut sel);
        let mut picked = color_picker::Picked::default();
        ui.horizontal(|ui| {
            ui.label("Stop");
            if compact {
                picked = color_picker::button(ui, ("stop", stroke), &mut g.stops[sel].color);
            }
            let mut pct = g.stops[sel].offset * 100.0;
            if ui
                .add(egui::DragValue::new(&mut pct).range(0.0..=100.0).speed(0.5).max_decimals(1).suffix("%"))
                .on_hover_text("Where this colour sits along the gradient")
                .changed()
            {
                g.stops[sel].offset = pct / 100.0;
            }
            if ui
                .add_enabled(g.stops.len() > 2, egui::Button::new("Remove").small())
                .on_hover_text("Remove this colour (or drag its marker down off the bar)")
                .clicked()
            {
                g.stops.remove(sel);
                sel = sel.min(g.stops.len() - 1);
            }
            ui.separator();
            if ui.small_button("Reverse").on_hover_text("Swap the ends").clicked() {
                for st in &mut g.stops {
                    st.offset = 1.0 - st.offset;
                }
            }
            if ui.small_button("Fade out").on_hover_text("The colour fading to transparent").clicked() {
                g.stops = vec![
                    GradientStop { offset: 0.0, color: base },
                    GradientStop { offset: 1.0, color: base.with_alpha(0.0) },
                ];
                sel = 0;
            }
            if ui
                .small_button("Shine")
                .on_hover_text(
                    "A radial highlight lit from the top left, so the shape looks rounded (like a ball or a button)",
                )
                .clicked()
            {
                let light = xenify_doc::Animatable::lerp(&base, &Color::rgba(1.0, 1.0, 1.0, base.a), 0.75);
                let dark = Color::rgba(base.r * 0.3, base.g * 0.3, base.b * 0.3, base.a);
                let c = kurbo::Vec2::new(bounds.x0 + bounds.width() * 0.35, bounds.y0 + bounds.height() * 0.3);
                let r = bounds.width().max(bounds.height()) * 0.8;
                *g = Gradient {
                    kind: GradientKind::Radial,
                    start: c,
                    end: c + kurbo::Vec2::new(r, 0.0),
                    stops: vec![
                        GradientStop { offset: 0.0, color: light },
                        GradientStop { offset: 0.35, color: base },
                        GradientStop { offset: 1.0, color: dark },
                    ],
                };
                sel = 0;
            }
        });
        if !compact {
            picked = color_picker::picker(ui, ("stop", stroke), &mut g.stops[sel].color);
        }
        if picked.released {
            app.paint.remember(g.stops[sel].color);
        }
        ui.data_mut(|d| d.insert_temp(sel_id, sel));

        ui.add_space(4.0);
        ends(ui, g, bounds);
        ui.weak("Drag the start and end on the canvas with the Gradient tool (G) or the Select tool.");
    });

    if toggled {
        let mut new = shape.clone();
        if stroke {
            new.stroke.as_mut().unwrap().gradient = gradient.map(Into::into);
        } else {
            new.fill.as_mut().unwrap().gradient = gradient.map(Into::into);
        }
        app.exec("Toggle gradient", Command::SetShape { layer: layer.id, index, shape: new });
    } else if let Some(g) = gradient.filter(|g| Some(g) != current.as_ref()) {
        edit(app, layer.id, index, stroke, g);
    }
}

/// Commands that change a shape's fill (or stroke) gradient to `new` at the
/// playhead. Only the tracks that changed (start, end, colours) are edited,
/// so each keeps its own keyframes. Returns the commands and the edited
/// tracks.
pub fn edit_commands(
    app: &XenifyApp,
    layer: Id,
    index: usize,
    stroke: bool,
    new: &Gradient,
) -> (Vec<Command>, Vec<PropPath>) {
    let (project, t) = (app.project(), app.time);
    let paint = project.layer(layer).ok().and_then(|l| {
        let s = l.shapes().get(index)?;
        if stroke { s.stroke.as_ref()?.gradient.clone() } else { s.fill.as_ref()?.gradient.clone() }
    });
    let Some(paint) = paint else { return (Vec::new(), Vec::new()) };
    let old = paint.value_at(t);
    let mut cmds = Vec::new();
    if new.kind != old.kind {
        // Linear ↔ radial isn't animated: it changes the shape itself.
        let Ok(l) = project.layer(layer) else { return (Vec::new(), Vec::new()) };
        let mut shape = l.shapes()[index].clone();
        let g = if stroke {
            shape.stroke.as_mut().and_then(|s| s.gradient.as_mut())
        } else {
            shape.fill.as_mut().and_then(|f| f.gradient.as_mut())
        };
        if let Some(g) = g {
            g.kind = new.kind;
        }
        cmds.push(Command::SetShape { layer, index, shape });
    }
    let [start, end, colours] = ShapeField::gradient_tracks(stroke);
    let changes = [
        (start, old.start != new.start, Value::Vec2(new.start)),
        (end, old.end != new.end, Value::Vec2(new.end)),
        (colours, old.stops != new.stops, Value::Stops(new.stops.clone())),
    ];
    let mut paths = Vec::new();
    for (field, changed, value) in changes {
        let path = PropPath::shape(index, field);
        if changed && let Ok(c) = Command::edit_value(project, layer, path, t, value, app.auto_key) {
            cmds.push(c);
            paths.push(path);
        }
    }
    (cmds, paths)
}

/// Changes a shape's fill (or stroke) gradient to `new` at the playhead (see
/// [`edit_commands`]) as one undo step.
pub fn edit(app: &mut XenifyApp, layer: Id, index: usize, stroke: bool, new: Gradient) {
    app.settle_ai();
    let (cmds, paths) = edit_commands(app, layer, index, stroke, &new);
    if cmds.is_empty() {
        return;
    }
    let was_animated =
        app.project().layer(layer).is_ok_and(|l| paths.iter().any(|p| l.property(*p).is_ok_and(|p| p.is_animated())));
    let label = if stroke { "Change stroke gradient" } else { "Change gradient" };
    if app.exec_merged(label, Command::batch(cmds)) && !was_animated && !app.auto_key && !paths.is_empty() {
        app.not_animated_hint(layer, paths[0]);
    }
}

/// Start and end points (centre and radius for radial), the angle, and a
/// reset to the shape's box.
fn ends(ui: &mut egui::Ui, g: &mut Gradient, bounds: kurbo::Rect) {
    let radial = g.kind == GradientKind::Radial;
    let xy = |ui: &mut egui::Ui, label: &str, p: &mut kurbo::Vec2| -> bool {
        ui.label(label).on_hover_text("In the layer's own coordinates (px)");
        let rx = ui.add(egui::DragValue::new(&mut p.x).speed(1.0).max_decimals(1).prefix("x "));
        let ry = ui.add(egui::DragValue::new(&mut p.y).speed(1.0).max_decimals(1).prefix("y "));
        rx.changed() || ry.changed()
    };
    ui.horizontal(|ui| {
        let before = g.start;
        if xy(ui, if radial { "Centre" } else { "Start" }, &mut g.start) && radial {
            // Moving the centre carries the radius handle along.
            g.end += g.start - before;
        }
    });
    if radial {
        ui.horizontal(|ui| {
            let mut len = (g.end - g.start).hypot();
            ui.label("Radius");
            if ui.add(egui::DragValue::new(&mut len).speed(1.0).range(1.0..=100_000.0).max_decimals(1)).changed() {
                let dir = (g.end - g.start).normalize();
                let dir = if dir.is_finite() { dir } else { kurbo::Vec2::new(1.0, 0.0) };
                g.end = g.start + dir * len;
            }
        });
    } else {
        ui.horizontal(|ui| {
            xy(ui, "End", &mut g.end);
        });
        ui.horizontal(|ui| {
            ui.label("Angle");
            let mut angle = g.angle_deg();
            let r =
                ui.add(egui::DragValue::new(&mut angle).range(-180.0..=180.0).suffix("°").speed(1.0).max_decimals(0));
            if user_changed(&r) {
                // Turn around the middle, keeping the length.
                let mid = (g.start + g.end) / 2.0;
                let half = (g.end - g.start).hypot() / 2.0;
                let a = angle.to_radians();
                let dir = kurbo::Vec2::new(a.cos(), a.sin());
                g.start = mid - dir * half;
                g.end = mid + dir * half;
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.small_button("Fit to shape").on_hover_text("Span the shape's box again").clicked() {
            *g = Gradient::across(bounds, g.kind, if radial { 0.0 } else { g.angle_deg() }, g.stops.clone());
        }
    });
}

/// The gradient bar with a marker per stop: click a marker to choose it, drag
/// it to move it, drag it down off the bar to remove it; click the bar to add
/// a colour there.
fn stop_bar(ui: &mut egui::Ui, g: &mut Gradient, sel: &mut usize) {
    let width = ui.available_width().clamp(160.0, 320.0);
    let (bar, bar_resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click());
    color_picker::paint_track(ui.painter(), bar, false, |t| g.color_at(t));
    let bar_resp = bar_resp.on_hover_text("Click to add a colour here");
    let (lane, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    let x_of = |offset: f64| bar.left() + offset.clamp(0.0, 1.0) as f32 * bar.width();
    let offset_at = |x: f32| ((x - bar.left()) / bar.width()).clamp(0.0, 1.0) as f64;

    let mut remove = None;
    for i in 0..g.stops.len() {
        let x = x_of(g.stops[i].offset);
        let hit = egui::Rect::from_center_size(pos2(x, lane.center().y), vec2(12.0, lane.height()));
        let resp = ui.interact(hit, ui.id().with(("stop-marker", i)), Sense::click_and_drag());
        if resp.is_pointer_button_down_on() || resp.clicked() {
            *sel = i;
        }
        // Keyboard: Tab to a stop, arrows move it (Shift: further), Delete removes it.
        if resp.has_focus() {
            *sel = i;
            let filter = egui::EventFilter { horizontal_arrows: true, ..Default::default() };
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, filter));
            let (step, delete) = ui.input(|inp| {
                let big = if inp.modifiers.shift { 0.10 } else { 0.01 };
                let step = if inp.key_pressed(egui::Key::ArrowLeft) {
                    -big
                } else if inp.key_pressed(egui::Key::ArrowRight) {
                    big
                } else {
                    0.0
                };
                (step, inp.key_pressed(egui::Key::Delete) || inp.key_pressed(egui::Key::Backspace))
            });
            if step != 0.0 {
                g.stops[i].offset = (g.stops[i].offset + step).clamp(0.0, 1.0);
            }
            if delete && g.stops.len() > 2 {
                remove = Some(i);
            }
        }
        let mut off_bar = false;
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            off_bar = g.stops.len() > 2 && p.y > lane.bottom() + 24.0;
            g.stops[i].offset = offset_at(p.x);
        }
        if resp.drag_stopped()
            && g.stops.len() > 2
            && resp.interact_pointer_pos().is_some_and(|p| p.y > lane.bottom() + 24.0)
        {
            remove = Some(i);
        }
        let x = x_of(g.stops[i].offset);
        let chosen = *sel == i;
        let edge = if chosen { Color32::WHITE } else { Color32::from_gray(110) };
        let alpha = if off_bar { 90 } else { 255 };
        let tip = pos2(x, lane.top());
        let body = egui::Rect::from_min_size(pos2(x - 5.0, lane.top() + 4.0), vec2(10.0, 10.0));
        ui.painter().add(egui::Shape::convex_polygon(
            vec![tip, pos2(x + 5.0, lane.top() + 4.0), pos2(x - 5.0, lane.top() + 4.0)],
            edge.gamma_multiply(alpha as f32 / 255.0),
            Stroke::NONE,
        ));
        color_picker::checker(ui.painter(), body);
        ui.painter().rect_filled(
            body,
            1.0,
            crate::colors::to_color32(g.stops[i].color).gamma_multiply(alpha as f32 / 255.0),
        );
        ui.painter().rect_stroke(body, 1.0, Stroke::new(if chosen { 2.0 } else { 1.0 }, edge), StrokeKind::Outside);
        if chosen {
            ui.painter().line_segment([pos2(x, bar.top()), pos2(x, bar.bottom())], Stroke::new(1.0, Color32::WHITE));
        }
        resp.on_hover_text(format!("{:.0}% — drag to move, drag down to remove", g.stops[i].offset * 100.0))
            .widget_info(|| {
                egui::WidgetInfo::slider(true, (g.stops[i].offset * 100.0).round(), format!("Colour stop {}", i + 1))
            });
    }
    if let Some(i) = remove {
        g.stops.remove(i);
        *sel = (*sel).min(g.stops.len() - 1);
    }
    if bar_resp.clicked()
        && let Some(p) = bar_resp.interact_pointer_pos()
    {
        let t = offset_at(p.x);
        g.stops.push(GradientStop { offset: t, color: g.color_at(t) });
        *sel = g.stops.len() - 1;
    }
}

/// A change the user made (not a slider re-snapping a value that moved
/// underneath it, e.g. an animated gradient at a new time).
fn user_changed(r: &egui::Response) -> bool {
    r.changed() && (r.dragged() || r.has_focus() || r.is_pointer_button_down_on() || r.clicked())
}
