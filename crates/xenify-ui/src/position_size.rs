//! Object ▸ Position & Size (Ctrl+T, or double-click a shape): exact
//! position, size and rotation of the selection, in the document's units.

use crate::app::XenifyApp;
use xenify_doc::Id;
use kurbo::{Affine, Rect};

pub struct PositionSize {
    layers: Vec<Id>,
    /// The selection's box and rotation when the dialog opened (pixels, degrees).
    start: Rect,
    start_rotation: f64,
    /// What's typed, in the document's units (and degrees).
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rotation: f64,
    keep_ratio: bool,
}

impl PositionSize {
    pub fn new(app: &mut XenifyApp) -> Option<Self> {
        let layers = app.selected_layers();
        let comp = app.comp().clone();
        let scene = xenify_anim::evaluate_project(app.project(), comp.id, app.time);
        let start = {
            let mut m = app.viewport.measurer();
            layers
                .iter()
                .filter_map(|id| xenify_anim::world_box(&comp, &scene, *id, &mut m))
                .reduce(|a, b| a.union(b))?
        };
        let start_rotation = match layers.as_slice() {
            [one] => app.project().layer(*one).ok()?.transform.rotation.value_at(app.time),
            _ => 0.0,
        };
        let u = comp.settings.units;
        Some(Self {
            layers,
            start,
            start_rotation,
            x: u.from_px(start.x0),
            y: u.from_px(start.y0),
            w: u.from_px(start.width()),
            h: u.from_px(start.height()),
            rotation: start_rotation,
            keep_ratio: false,
        })
    }
}

/// The transform taking the box `from` to `to` and turning it by `turn`
/// degrees around the new box's centre.
pub fn box_transform(from: Rect, to: Rect, turn: f64) -> Affine {
    let sx = if from.width() > 1e-9 { to.width() / from.width() } else { 1.0 };
    let sy = if from.height() > 1e-9 { to.height() / from.height() } else { 1.0 };
    let c = to.center().to_vec2();
    Affine::translate(c)
        * Affine::rotate(turn.to_radians())
        * Affine::translate(-c)
        * Affine::translate(to.origin().to_vec2())
        * Affine::scale_non_uniform(sx, sy)
        * Affine::translate(-from.origin().to_vec2())
}

/// Draws the dialog; returns true when it should close.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, d: &mut PositionSize) -> bool {
    let u = app.comp().settings.units;
    let suffix = format!(" {}", u.unit.suffix());
    ui.weak(if d.layers.len() == 1 {
        "The object's bounding box, from the frame's top-left corner."
    } else {
        "The bounding box around the selected objects, from the frame's top-left corner."
    });
    ui.add_space(4.0);
    let speed = u.from_px(1.0);
    let decimals = u.decimals();
    let ratio = if d.h.abs() > 1e-12 { d.w / d.h } else { 1.0 };
    egui::Grid::new("position-size").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("X");
        ui.add(egui::DragValue::new(&mut d.x).speed(speed).fixed_decimals(decimals).suffix(&suffix));
        ui.label("Y");
        ui.add(egui::DragValue::new(&mut d.y).speed(speed).fixed_decimals(decimals).suffix(&suffix));
        ui.end_row();
        ui.label("Width");
        let w = ui.add(
            egui::DragValue::new(&mut d.w).speed(speed).range(0.0..=f64::MAX).fixed_decimals(decimals).suffix(&suffix),
        );
        ui.label("Height");
        let h = ui.add(
            egui::DragValue::new(&mut d.h).speed(speed).range(0.0..=f64::MAX).fixed_decimals(decimals).suffix(&suffix),
        );
        if d.keep_ratio && w.changed() {
            d.h = d.w / ratio;
        } else if d.keep_ratio && h.changed() {
            d.w = d.h * ratio;
        }
        ui.end_row();
        ui.label("Rotation");
        ui.add(egui::DragValue::new(&mut d.rotation).speed(0.5).fixed_decimals(1).suffix("°"));
        ui.label("");
        ui.checkbox(&mut d.keep_ratio, "Keep proportions");
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.weak(format!("Units: {} at {} pixels per inch (Canvas › Canvas Settings to change).", u.unit.label(), u.dpi));
    ui.separator();
    let mut close = false;
    ui.horizontal(|ui| {
        if ui.add(crate::widgets::primary("Apply")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let to =
                Rect::from_origin_size((u.to_px(d.x), u.to_px(d.y)), (u.to_px(d.w).max(1e-6), u.to_px(d.h).max(1e-6)));
            apply(app, d, to);
            close = true;
        }
        close |= ui.button("Cancel").clicked();
    });
    close
}

fn apply(app: &mut XenifyApp, d: &PositionSize, to: Rect) {
    let x = box_transform(d.start, to, d.rotation - d.start_rotation);
    if x.as_coeffs().iter().zip(Affine::IDENTITY.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-12) {
        return;
    }
    let (comp, time, auto_key) = (app.comp, app.time, app.auto_key);
    let mut cmds = Vec::new();
    for id in &d.layers {
        let snap = xenify_ops::transform::Snapshot::new(app.project(), comp, *id, time, Rect::ZERO);
        match snap.and_then(|s| s.affine_commands(app.project(), x, time, auto_key)) {
            Ok(c) => cmds.extend(c),
            Err(e) => {
                app.fail(e.to_string());
                return;
            }
        }
    }
    app.exec("Position & size", xenify_ops::Command::batch(cmds));
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn boxes_map_onto_boxes() {
        let from = Rect::new(10.0, 10.0, 30.0, 20.0);
        let to = Rect::new(100.0, 50.0, 140.0, 80.0);
        let x = box_transform(from, to, 0.0);
        assert!((x * Point::new(10.0, 10.0) - Point::new(100.0, 50.0)).hypot() < 1e-9);
        assert!((x * Point::new(30.0, 20.0) - Point::new(140.0, 80.0)).hypot() < 1e-9);
        // Turning keeps the centre in place.
        let t = box_transform(from, from, 90.0);
        assert!((t * from.center() - from.center()).hypot() < 1e-9);
    }
}
