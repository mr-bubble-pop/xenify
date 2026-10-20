//! Diagram settings: the Connector, Label and Arrowheads sections of the
//! Properties panel, and the Connector tool's options (the defaults for new
//! connectors, stored with the canvas).

use crate::app::XenifyApp;
use crate::color_picker;
use egui::RichText;
use xenify_doc::{
    DiagramSettings, Flow, FlowKind, JumpStyle, Layer, LayerKind, Marker, MarkerKind, NodeLabel, Port, PortRef,
    PortSide, RouteStyle, TextAlign,
};
use xenify_ops::Command;

fn marker_combo(ui: &mut egui::Ui, id: &str, m: &mut Marker) {
    egui::ComboBox::from_id_salt(id).selected_text(m.kind.label()).width(96.0).show_ui(ui, |ui| {
        for k in MarkerKind::ALL {
            ui.selectable_value(&mut m.kind, k, k.label());
        }
    });
}

fn route_combo(ui: &mut egui::Ui, id: &str, r: &mut RouteStyle) {
    egui::ComboBox::from_id_salt(id).selected_text(r.label()).width(80.0).show_ui(ui, |ui| {
        for s in RouteStyle::ALL {
            ui.selectable_value(r, s, s.label()).on_hover_text(match s {
                RouteStyle::Orthogonal => "Horizontal and vertical runs with right-angle bends, around shapes",
                RouteStyle::Straight => "A direct line",
                RouteStyle::Curved => "A smooth curve that still bends around shapes",
            });
        }
    });
}

fn jump_combo(ui: &mut egui::Ui, id: &str, j: &mut JumpStyle) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(j.label())
        .width(70.0)
        .show_ui(ui, |ui| {
            for s in JumpStyle::ALL {
                ui.selectable_value(j, s, s.label());
            }
        })
        .response
        .on_hover_text("How a line shows it crosses another without joining it");
}

/// Called when a diagram canvas is opened: the diagram tools come first.
pub fn entered(app: &mut XenifyApp) {
    if !matches!(app.tool, crate::app::Tool::Select | crate::app::Tool::Connector) {
        app.set_tool(crate::app::Tool::Select);
    }
    crate::dock::reveal(app, crate::dock::Panel::Library);
}

/// The Connector tool's options: defaults for new connectors on this canvas.
pub fn tool_options(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let comp = app.comp;
    let old = app.comp().settings.clone();
    let mut d = old.diagram.clone().unwrap_or_default();
    ui.label("Route");
    route_combo(ui, "tool-route", &mut d.route);
    ui.label("Jumps");
    jump_combo(ui, "tool-jump", &mut d.jump);
    ui.label("Arrow");
    marker_combo(ui, "tool-arrow", &mut d.end_marker);
    ui.label("Width");
    ui.add(egui::DragValue::new(&mut d.line_width).range(0.25..=50.0).speed(0.1).suffix(" px"));
    let mut c = d.line_color;
    if color_picker::button(ui, "tool-line-color", &mut c).changed {
        d.line_color = c;
    }
    ui.separator();
    ui.weak("Drag from a shape (or one of its ports) to another · Ctrl: don't glue");
    if Some(&d) != old.diagram.as_ref() && (old.diagram.is_some() || d != DiagramSettings::default()) {
        let settings = xenify_doc::CompSettings { diagram: Some(d), ..old };
        app.exec_merged("Connector defaults", Command::SetCompSettings { comp, settings });
    }
}

/// Properties: the connector's routing, the label and arrowheads.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if let Some(c) = &layer.meta.connector {
        let mut meta = layer.meta.clone();
        let mut new = (**c).clone();
        let mut reset = false;
        // Where the line ends now, for ends that are let go.
        let route = xenify_anim::connector_routes(app.project(), app.comp, app.time)
            .into_iter()
            .find(|r| r.id == layer.id)
            .map(|r| r.points);
        let ends = route.as_ref().map(|p| (p[0], p[p.len() - 1]));
        egui::CollapsingHeader::new(RichText::new("Connector").strong())
            .id_salt(("connector", layer.id))
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new(("connector-grid", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
                    ui.label("Route");
                    route_combo(ui, "route", &mut new.route);
                    ui.end_row();
                    if new.route == RouteStyle::Orthogonal {
                        ui.label("Corners");
                        ui.add(
                            egui::DragValue::new(&mut new.corner_radius).range(0.0..=200.0).speed(0.25).suffix(" px"),
                        )
                        .on_hover_text("Rounds the bends (0 = sharp)");
                        ui.end_row();
                    }
                    ui.label("Jumps");
                    ui.horizontal(|ui| {
                        jump_combo(ui, "jump", &mut new.jump);
                        if new.jump != JumpStyle::None {
                            ui.add(egui::DragValue::new(&mut new.jump_size).range(1.0..=50.0).speed(0.1).suffix(" px"))
                                .on_hover_text("Half the width of a jump");
                        }
                    });
                    ui.end_row();
                    ui.label("Avoid");
                    ui.checkbox(&mut new.avoid, "Go around shapes")
                        .on_hover_text("Find a way around shapes in between (off: straight through them)");
                    ui.end_row();
                    ui.label("Spacing");
                    ui.add(egui::DragValue::new(&mut new.margin).range(0.0..=200.0).speed(0.25).suffix(" px"))
                        .on_hover_text("Space kept around shapes, and how far the line leaves a shape before turning");
                    ui.end_row();
                    for (label, end, now) in
                        [("Start", &mut new.from, ends.map(|e| e.0)), ("End", &mut new.to, ends.map(|e| e.1))]
                    {
                        ui.label(label);
                        ui.horizontal(|ui| match end.attach {
                            Some(a) => {
                                let name = app
                                    .project()
                                    .layer(a.layer)
                                    .map_or_else(|_| "(deleted)".to_string(), |l| l.meta.name.clone());
                                let at = match a.port {
                                    PortRef::Auto => "best side".to_string(),
                                    PortRef::Port(i) => format!("port {}", i + 1),
                                };
                                ui.label(format!("{name}, {at}"));
                                if matches!(a.port, PortRef::Port(_))
                                    && ui
                                        .small_button("Any side")
                                        .on_hover_text("Let the route pick the side")
                                        .clicked()
                                {
                                    end.attach = Some(xenify_doc::Attach { port: PortRef::Auto, ..a });
                                }
                                if ui
                                    .small_button("Detach")
                                    .on_hover_text("Unglue this end; it stays where it is")
                                    .clicked()
                                {
                                    end.attach = None;
                                    if let Some(p) = now {
                                        end.point = p.to_vec2();
                                    }
                                }
                            }
                            None => {
                                ui.weak("not glued");
                            }
                        });
                        ui.end_row();
                    }
                    ui.label("Flow");
                    ui.horizontal(|ui| {
                        let mut on = new.flow.is_some();
                        if ui.checkbox(&mut on, "").on_hover_text("Marks moving along the line, start to end").changed()
                        {
                            new.flow = on.then_some(Flow { speed: 60.0, kind: FlowKind::Dashes });
                        }
                        if let Some(f) = &mut new.flow {
                            egui::ComboBox::from_id_salt("flow-kind")
                                .selected_text(f.kind.label())
                                .width(70.0)
                                .show_ui(ui, |ui| {
                                    for k in FlowKind::ALL {
                                        ui.selectable_value(&mut f.kind, k, k.label());
                                    }
                                });
                            ui.add(
                                egui::DragValue::new(&mut f.speed).range(-2000.0..=2000.0).speed(1.0).suffix(" px/s"),
                            )
                            .on_hover_text("How fast (negative: backwards)");
                        }
                    });
                    ui.end_row();
                    if meta.label.is_some() {
                        ui.label("Label at");
                        ui.add(egui::Slider::new(&mut new.label_at, 0.0..=1.0).fixed_decimals(2))
                            .on_hover_text("Where the label sits along the line");
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Swap ends").on_hover_text("Run the line the other way").clicked() {
                        std::mem::swap(&mut new.from, &mut new.to);
                    }
                    if !new.waypoints.is_empty()
                        && ui.button("Reset route").on_hover_text("Forget dragged bends; route automatically").clicked()
                    {
                        reset = true;
                    }
                });
            });
        if reset {
            new.waypoints.clear();
        }
        if new != **c {
            meta.connector = Some(Box::new(new));
            app.exec_merged("Connector settings", Command::SetLayerMeta { layer: layer.id, meta });
            return;
        }
    }
    arrowheads(app, ui, layer);
    label(app, ui, layer);
    ports(app, ui, layer);
}

/// Where connectors glue to a shape: the middle of each side unless set.
fn ports(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if layer.meta.connector.is_some()
        || !matches!(layer.kind, LayerKind::Shape { .. } | LayerKind::Precomp { .. } | LayerKind::Image { .. })
    {
        return;
    }
    let custom = !layer.meta.ports.is_empty();
    let mut ports: Vec<Port> = layer.meta.ports().to_vec();
    let mut changed = false;
    egui::CollapsingHeader::new(RichText::new(if custom { "Ports (custom)" } else { "Ports" }).strong())
        .id_salt(("ports", layer.id))
        .default_open(false)
        .show(ui, |ui| {
            ui.weak("Where connectors glue: across and down the shape's box (0–100%), and which way lines leave.");
            let mut remove = None;
            egui::Grid::new(("ports-grid", layer.id)).num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
                for (i, p) in ports.iter_mut().enumerate() {
                    ui.label(format!("{}", i + 1));
                    let mut x = p.pos.x * 100.0;
                    let mut y = p.pos.y * 100.0;
                    ui.horizontal(|ui| {
                        changed |=
                            ui.add(egui::DragValue::new(&mut x).range(0.0..=100.0).speed(0.5).suffix(" %")).changed();
                        changed |=
                            ui.add(egui::DragValue::new(&mut y).range(0.0..=100.0).speed(0.5).suffix(" %")).changed();
                    });
                    p.pos = xenify_doc::Vec2::new(x / 100.0, y / 100.0);
                    egui::ComboBox::from_id_salt(("port-side", i))
                        .selected_text(side_label(p.side))
                        .width(60.0)
                        .show_ui(ui, |ui| {
                            for s in [PortSide::Top, PortSide::Right, PortSide::Bottom, PortSide::Left] {
                                changed |= ui.selectable_value(&mut p.side, s, side_label(s)).changed();
                            }
                        });
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = remove {
                ports.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button("Add port").on_hover_text("A new port in the middle of the top edge").clicked() {
                    ports.push(Port::new(0.5, 0.0, PortSide::Top));
                    changed = true;
                }
                if custom && ui.button("Reset").on_hover_text("Back to the middle of each side").clicked() {
                    ports.clear();
                    changed = true;
                }
            });
        });
    if changed {
        // The default four are stored as "none set".
        let ports = if ports.as_slice() == xenify_doc::DEFAULT_PORTS.as_slice() { Vec::new() } else { ports };
        if let Ok(cmd) = xenify_ops::diagram::set_ports(app.project(), layer.id, ports) {
            app.exec_merged("Ports", cmd);
        }
    }
}

fn side_label(s: PortSide) -> &'static str {
    match s {
        PortSide::Top => "Up",
        PortSide::Right => "Right",
        PortSide::Bottom => "Down",
        PortSide::Left => "Left",
    }
}

/// Start and end marks for strokes of open lines.
fn arrowheads(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let LayerKind::Shape { shapes } = &layer.kind else { return };
    let open = |s: &xenify_doc::ShapeItem| match &s.geometry {
        xenify_doc::Geometry::Path { path } => {
            !path.value_at(app.time).elements().iter().any(|e| matches!(e, kurbo::PathEl::ClosePath))
        }
        _ => false,
    };
    let lines: Vec<usize> =
        shapes.iter().enumerate().filter(|(_, s)| s.stroke.is_some() && open(s)).map(|(i, _)| i).collect();
    if lines.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(RichText::new("Arrowheads").strong())
        .id_salt(("arrowheads", layer.id))
        .default_open(layer.meta.connector.is_some())
        .show(ui, |ui| {
            for index in lines {
                let Some(stroke) = &shapes[index].stroke else { continue };
                let mut s = stroke.clone();
                ui.push_id(("arrow", index), |ui| {
                    if shapes.len() > 1 {
                        ui.weak(&shapes[index].name);
                    }
                    egui::Grid::new("arrow-grid").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                        for (label, m) in [("Start", &mut s.start_marker), ("End", &mut s.end_marker)] {
                            ui.label(label);
                            marker_combo(ui, label, m);
                            if !m.is_none() {
                                ui.add(egui::DragValue::new(&mut m.size).range(0.2..=6.0).speed(0.02).prefix("× "))
                                    .on_hover_text("Size, relative to the line's width");
                            }
                            ui.end_row();
                        }
                    });
                });
                if &s != stroke {
                    let mut shape = shapes[index].clone();
                    shape.stroke = Some(s);
                    app.exec_merged("Arrowheads", Command::SetShape { layer: layer.id, index, shape });
                    return;
                }
            }
        });
}

/// Text shown inside a shape or along a connector.
fn label(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if !matches!(layer.kind, LayerKind::Shape { .. }) {
        return;
    }
    let old = layer.meta.label.clone();
    let mut l = old.clone().unwrap_or_else(|| NodeLabel::new(""));
    // Double-clicking a connector or diagram shape asks for the label.
    let focus = std::mem::take(&mut app.focus_text_edit);
    egui::CollapsingHeader::new(RichText::new("Label").strong())
        .id_salt(("label", layer.id))
        .default_open(old.is_some() || app.project().comp(app.comp).is_ok_and(|c| !c.settings.kind.is_animation()))
        .open(focus.then_some(true))
        .show(ui, |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut l.text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("Text inside the shape (double-click the shape on a diagram)"),
            );
            if focus {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                ui.label("Size");
                ui.add(egui::DragValue::new(&mut l.font_size).range(4.0..=400.0).speed(0.2).suffix(" px"));
                color_picker::button(ui, ("label-color", layer.id), &mut l.color);
                for (a, name) in [(TextAlign::Left, "Left"), (TextAlign::Center, "Centre"), (TextAlign::Right, "Right")]
                {
                    ui.selectable_value(&mut l.align, a, name);
                }
            });
            if layer.meta.connector.is_some() {
                // The box that keeps the line from running through the text.
                ui.horizontal(|ui| {
                    let diagram = app.project().comp(app.comp).is_ok_and(|c| !c.settings.kind.is_animation());
                    let canvas = app.comp().settings.background;
                    let mut own = l.background.is_some();
                    let tip = if diagram {
                        "Its own colour behind the label (else the canvas colour)"
                    } else {
                        "A box behind the label, which then sits on the line (else it sits beside it)"
                    };
                    if ui.checkbox(&mut own, "Box Behind").on_hover_text(tip).changed() {
                        l.background = own.then_some(canvas);
                    }
                    if let Some(bg) = &mut l.background {
                        color_picker::button(ui, ("label-bg", layer.id), bg);
                    }
                });
            }
        });
    let new = (!l.text.is_empty()).then_some(l);
    if new != old {
        let mut meta = layer.meta.clone();
        meta.label = new;
        app.exec_merged("Label", Command::SetLayerMeta { layer: layer.id, meta });
    }
}
