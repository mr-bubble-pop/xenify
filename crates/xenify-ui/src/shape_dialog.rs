//! Clicking (without dragging) with a shape tool: type the new shape's exact
//! size (in the document's units), and for polygons the number of sides.

use crate::app::{XenifyApp, Tool};
use kurbo::{Point, Vec2};

pub struct ShapeSize {
    tool: Tool,
    /// Where it goes (its centre), composition pixels.
    at: Point,
    /// Size in the document's units.
    w: f64,
    h: f64,
    keep_square: bool,
}

impl ShapeSize {
    pub fn new(app: &XenifyApp, at: Point) -> Self {
        let u = app.comp().settings.units;
        let side = u.from_px(200.0);
        Self { tool: app.tool, at, w: side, h: side, keep_square: false }
    }
}

/// The dialog's title ("New rounded rectangle", "New star"…).
pub fn title_of(d: &ShapeSize, app: &XenifyApp) -> String {
    title(d.tool, app)
}

fn title(tool: Tool, app: &XenifyApp) -> String {
    match tool {
        Tool::Rect if app.tool_options.corner_radius > 0.0 => "New rounded rectangle".into(),
        Tool::Rect => "New rectangle".into(),
        Tool::Ellipse => "New ellipse".into(),
        Tool::Line => "New line".into(),
        _ => format!("New {}", app.tool_options.figure_name().to_lowercase()),
    }
}

/// Draws the dialog; returns true when it should close.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, d: &mut ShapeSize) -> bool {
    let u = app.comp().settings.units;
    let suffix = format!(" {}", u.unit.suffix());
    let line = d.tool == Tool::Line;
    ui.weak(if line {
        "Starts where you clicked. (Drag instead of clicking to draw it.)"
    } else {
        "Centred where you clicked. (Drag instead of clicking to draw it.)"
    });
    ui.add_space(4.0);
    egui::Grid::new("shape-size").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(if line { "Across" } else { "Width" });
        let ratio = if d.h.abs() > 1e-9 { d.w / d.h } else { 1.0 };
        let rw = ui.add(egui::DragValue::new(&mut d.w).speed(u.from_px(1.0)).suffix(&suffix).range(0.0..=f64::MAX));
        ui.end_row();
        ui.label(if line { "Down" } else { "Height" });
        let rh = ui.add(egui::DragValue::new(&mut d.h).speed(u.from_px(1.0)).suffix(&suffix).range(0.0..=f64::MAX));
        ui.end_row();
        if d.keep_square && rw.changed() {
            d.h = d.w / ratio;
        } else if d.keep_square && rh.changed() {
            d.w = d.h * ratio;
        }
        if !line {
            ui.label("");
            ui.checkbox(&mut d.keep_square, "Keep proportions");
            ui.end_row();
        }
        if d.tool == Tool::Polygon {
            ui.label("Shape");
            ui.horizontal(|ui| crate::tool_options::figure_controls(ui, &mut app.tool_options));
            ui.end_row();
        }
        if d.tool == Tool::Rect {
            ui.label("Corners");
            ui.add(
                egui::DragValue::new(&mut app.tool_options.corner_radius)
                    .range(0.0..=f64::MAX)
                    .speed(0.5)
                    .suffix(" px"),
            );
            ui.end_row();
        }
    });
    ui.separator();
    let mut close = false;
    ui.horizontal(|ui| {
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.add(crate::widgets::primary("Create")).clicked() || enter {
            let size = Vec2::new(u.to_px(d.w), u.to_px(d.h));
            if line {
                crate::viewport::create_line(app, d.at.to_vec2() + size / 2.0, size);
            } else if size.x > 0.0 && size.y > 0.0 {
                crate::viewport::create_shape(app, d.tool, d.at.to_vec2(), size);
            }
            close = true;
        }
        close |= ui.button("Cancel").clicked();
    });
    close
}
