//! Size and corner controls for shapes that are rectangles or ellipses:
//! live ones, and paths that still are one (recognised from their points),
//! so a rectangle keeps its controls after its points were edited.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::primitives::{self, Primitive};
use xenify_doc::{Geometry, Id, Layer, PropPath, ShapeField, ShapeItem, Value};
use xenify_ops::Command;
use kurbo::BezPath;

/// Sets shape `shape`'s outline, first turning a rectangle or ellipse into a
/// path (keeping its animation) in the same undo step.
pub fn set_shape_path(app: &mut XenifyApp, layer: Id, shape: usize, new: BezPath) {
    let live = app
        .project()
        .layer(layer)
        .ok()
        .and_then(|l| l.shapes().get(shape).filter(|s| !matches!(s.geometry, Geometry::Path { .. })).cloned());
    if let Some(item) = live {
        let baked = ShapeItem { geometry: primitives::to_path_geometry(&item.geometry), ..item };
        if !app.exec_merged("Edit points", Command::SetShape { layer, index: shape, shape: baked }) {
            return;
        }
    }
    app.edit_value(layer, PropPath::shape(shape, ShapeField::Path), Value::Path(new));
}

/// Starts a new undo step when a field is grabbed or focused.
fn gesture(app: &mut XenifyApp, r: &egui::Response) {
    if r.drag_started() || r.gained_focus() {
        app.new_gesture();
    }
}

fn length(ui: &mut egui::Ui, v: &mut f64, tip: &str) -> egui::Response {
    ui.add(egui::DragValue::new(v).range(0.0..=100_000.0).speed(0.5).max_decimals(1).suffix(" px")).on_hover_text(tip)
}

/// The "Each corner" row: top-left clockwise. Returns the new radii when one
/// changed. It comes first in the same grid for live rectangles and for
/// paths that are rectangles, so a field being dragged stays the same widget
/// when the first change turns a live rectangle into a path.
fn each_corner(app: &mut XenifyApp, ui: &mut egui::Ui, radii: [f64; 4]) -> Option<[f64; 4]> {
    let mut out = None;
    ui.label("Each corner").on_hover_text(
        "Round corners separately (a live rectangle becomes a path; it keeps these controls while it's a rectangle)",
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (i, name) in ["top-left", "top-right", "bottom-right", "bottom-left"].into_iter().enumerate() {
            let mut v = radii[i];
            let r = length(ui, &mut v, &format!("Radius of the {name} corner"));
            gesture(app, &r);
            if r.changed() {
                let mut new = radii;
                new[i] = v;
                out = Some(new);
            }
        }
    });
    ui.end_row();
    out
}

/// One radius for every corner; returns it when changed.
fn all_corners(app: &mut XenifyApp, ui: &mut egui::Ui, radii: [f64; 4]) -> Option<[f64; 4]> {
    let same = radii.iter().all(|r| (r - radii[0]).abs() < 1e-6);
    let mut all = radii.iter().copied().fold(0.0, f64::max);
    ui.label("Corners");
    let r = ui.add(egui::DragValue::new(&mut all).range(0.0..=100_000.0).speed(0.5).max_decimals(1).suffix(if same {
        " px"
    } else {
        " px (mixed)"
    }));
    gesture(app, &r);
    let changed = r.on_hover_text("Rounds every corner by this radius").changed();
    ui.end_row();
    changed.then_some([all; 4])
}

/// Width and height fields; returns the new size when one changed.
fn size(app: &mut XenifyApp, ui: &mut egui::Ui, rect: kurbo::Rect) -> Option<kurbo::Rect> {
    let (mut w, mut h) = (rect.width(), rect.height());
    let mut changed = false;
    ui.label("Size");
    ui.horizontal(|ui| {
        for (v, tip) in [(&mut w, "Width"), (&mut h, "Height")] {
            let r = length(ui, v, tip);
            gesture(app, &r);
            changed |= r.changed();
        }
    });
    ui.end_row();
    changed.then(|| kurbo::Rect::from_center_size(rect.center(), (w.max(0.01), h.max(0.01))))
}

/// Under a shape's own rows in Properties: per-corner radii for live
/// rectangles, and size and corners for paths that are rectangles or ellipses.
pub fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, index: usize, shape: &ShapeItem) {
    let t = app.time;
    let grid = ("smart-shape", layer.id, index);
    match &shape.geometry {
        // Live figures edit through their own rows (count, inner radius, hole).
        Geometry::Figure { .. } => {}
        Geometry::Rect { position, size: sz, corner_radius } => {
            let rect = kurbo::Rect::from_center_size(position.value_at(t).to_point(), sz.value_at(t).to_size()).abs();
            let r = corner_radius.value_at(t);
            let mut new = None;
            egui::Grid::new(grid).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
                new = each_corner(app, ui, [r; 4]);
            });
            if let Some(radii) = new {
                set_shape_path(app, layer.id, index, primitives::rect_path(rect, radii));
            }
        }
        Geometry::Path { path } => {
            let Some(found) = primitives::recognize(&path.value_at(t)) else { return };
            let mut new = None;
            egui::Grid::new(grid).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
                if let Primitive::Rect { rect, radii } = found {
                    // Both rows every frame (so widget ids don't shift).
                    let each = each_corner(app, ui, radii);
                    let all = all_corners(app, ui, radii);
                    if let Some(radii) = each.or(all) {
                        new = Some(Primitive::Rect { rect, radii: primitives::clamp_radii(rect, radii) });
                    }
                }
                if let Some(rect) = size(app, ui, found.rect()) {
                    new = Some(found.with_rect(rect));
                }
            });
            let what = match found {
                Primitive::Rect { .. } => "Still a rectangle",
                Primitive::Ellipse { .. } => "Still an ellipse",
            };
            ui.label(RichText::new(what).weak()).on_hover_text(
                "Its points still make this shape, so its size and corners can be set here. Move a point \
                 off the shape and these go away (the points stay editable).",
            );
            if let Some(p) = new {
                set_shape_path(app, layer.id, index, p.to_path());
            }
        }
        Geometry::Ellipse { .. } => {}
    }
}
