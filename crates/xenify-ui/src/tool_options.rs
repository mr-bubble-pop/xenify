//! Settings for the current tool, shown at the end of the control bar.

use crate::app::{XenifyApp, Tool};
use xenify_ops::draw::SculptMode;

/// `[` / `]`: smaller / bigger brush (brush, pencil or sculpt).
pub fn resize_brush(app: &mut XenifyApp, bigger: bool) {
    let f = if bigger { 1.25 } else { 0.8 };
    let o = &mut app.tool_options;
    match app.tool {
        Tool::Brush => o.brush_size = (o.brush_size * f).clamp(1.0, 500.0),
        Tool::Pencil => o.pencil_width = (o.pencil_width * f).clamp(0.5, 200.0),
        Tool::Sculpt => o.sculpt_radius = (o.sculpt_radius * f).clamp(5.0, 600.0),
        _ => {}
    }
}

/// The polygon tool's shape (polygon, star, gear, heart…) and its settings,
/// in the control bar and the click-for-size dialog.
pub fn figure_controls(ui: &mut egui::Ui, o: &mut crate::app::ToolOptions) {
    use xenify_ops::draw::Figure;
    let params = o.figure_params();
    egui::ComboBox::from_id_salt("polygon-figure").selected_text(o.figure_name()).show_ui(ui, |ui| {
        for f in Figure::ALL {
            let picture = |b| xenify_ops::draw::figure_path(f, &params, b);
            if crate::widgets::shape_row(ui, o.figure == f, f.name(), picture).clicked() {
                o.figure = f;
            }
        }
    });
    match o.figure {
        Figure::Polygon | Figure::Star => {
            ui.label(if o.figure == Figure::Star { "Points" } else { "Sides" });
            ui.add(egui::DragValue::new(&mut o.polygon_sides).range(3..=64).speed(0.1));
            if o.figure == Figure::Star {
                ui.label("Inner");
                ui.add(egui::Slider::new(&mut o.star_inner, 0.1..=0.95).fixed_decimals(2))
                    .on_hover_text("How far in the points' valleys reach (fraction of the size)");
            }
        }
        Figure::Gear => {
            ui.label("Teeth");
            ui.add(egui::DragValue::new(&mut o.gear_teeth).range(3..=200).speed(0.1));
            ui.label("Depth");
            ui.add(egui::Slider::new(&mut o.gear_depth, 0.05..=0.5).fixed_decimals(2))
                .on_hover_text("How deep the teeth are (fraction of the radius)");
            ui.label("Hole");
            ui.add(egui::Slider::new(&mut o.gear_hole, 0.0..=0.7).fixed_decimals(2))
                .on_hover_text("Size of the hole in the middle (fraction of the radius; 0 = none)");
        }
        Figure::Heart | Figure::Arrow | Figure::Bubble => {}
    }
}

/// The current tool's name and options, laid out in the caller's row.
pub fn bar(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(app.tool.name()).strong());
    let o = &mut app.tool_options;
    match app.tool {
        Tool::Polygon => figure_controls(ui, o),
        Tool::Connector => crate::diagram_ui::tool_options(app, ui),
        Tool::Rect => {
            ui.label("Corners");
            ui.add(egui::DragValue::new(&mut o.corner_radius).range(0.0..=10_000.0).speed(0.5).suffix(" px"))
                .on_hover_text(
                    "Corner radius of new rectangles (0 = sharp). Round them later by dragging the \
                     circles inside their corners with Edit Points (A)",
                );
        }
        Tool::Pencil | Tool::Brush => {
            if app.tool == Tool::Pencil {
                ui.label("Width");
                ui.add(egui::DragValue::new(&mut o.pencil_width).range(0.5..=200.0).speed(0.2).suffix(" px"));
            } else {
                ui.label("Size");
                ui.add(egui::DragValue::new(&mut o.brush_size).range(1.0..=500.0).speed(0.3).suffix(" px"));
            }
            ui.label("Smoothing");
            ui.add(egui::Slider::new(&mut o.smoothing, 1.0..=10.0).fixed_decimals(0))
                .on_hover_text("Higher: smoother lines with fewer points. Lower: follows your hand closely");
            ui.checkbox(&mut o.recognize, "Recognise shapes")
                .on_hover_text("Lines, circles, ellipses, rectangles and triangles become clean shapes (Alt: skip)");
        }
        Tool::Sculpt => {
            for (mode, label, tip) in [
                (SculptMode::Push, "Push", "Drag points along with the brush, like a finger"),
                (SculptMode::Attract, "Attract", "Magnet: pull points towards the brush (Alt: repel)"),
                (SculptMode::Repel, "Repel", "Push points away from the brush (Alt: attract)"),
            ] {
                ui.selectable_value(&mut o.sculpt_mode, mode, label).on_hover_text(tip);
            }
            ui.label("Size");
            ui.add(egui::DragValue::new(&mut o.sculpt_radius).range(5.0..=600.0).speed(0.5).suffix(" px"));
            ui.label("Strength");
            ui.add(egui::Slider::new(&mut o.sculpt_strength, 0.05..=1.0).fixed_decimals(2));
        }
        Tool::Select | Tool::FreeTransform | Tool::Perspective => {
            let selected = app.selected_layers();
            let distorted =
                selected.iter().any(|id| app.project().layer(*id).is_ok_and(|l| l.transform.distort.is_some()));
            if selected.is_empty() {
                ui.weak("Click to select, drag to move · Shift+click adds · drag on empty canvas to box-select");
            }
            if (distorted || app.tool == Tool::Perspective)
                && ui
                    .button("Remove perspective")
                    .on_hover_text("Clears the corner pin of the selected layers")
                    .clicked()
            {
                crate::viewport::clear_distort(app);
            }
            if app.tool != Tool::Perspective
                && app.viewport.pivot_moved(app)
                && ui.button("Re-centre").on_hover_text("Put the centre back in the middle of the selection").clicked()
            {
                crate::viewport::recentre(app);
            }
        }
        Tool::Crop | Tool::Slice => crate::viewport::crop_options(app, ui),
        Tool::Mask => crate::viewport::mask_options(app, ui),
        Tool::Zoom | Tool::Hand => {
            if ui.button("Fit").clicked() {
                app.viewport.request_fit();
            }
            if ui.button("100%").clicked() {
                app.viewport.set_zoom_100();
            }
        }
        Tool::Gradient => {
            use crate::app::PaintSlot;
            use xenify_doc::GradientKind;
            let mut slot = app.paint.active;
            ui.selectable_value(&mut slot, PaintSlot::Fill, "Fill").on_hover_text("Set the fill's gradient");
            ui.selectable_value(&mut slot, PaintSlot::Stroke, "Stroke")
                .on_hover_text("Set the stroke's (outline's) gradient");
            app.paint.active = slot;
            ui.separator();
            let o = &mut app.tool_options;
            ui.selectable_value(&mut o.gradient_kind, GradientKind::Linear, "Linear")
                .on_hover_text("Drag from where it starts to where it ends");
            ui.selectable_value(&mut o.gradient_kind, GradientKind::Radial, "Radial")
                .on_hover_text("Drag from the centre out to the edge");
            ui.separator();
            ui.weak("Drag across a shape · drag the ends to adjust · colours in the Colour panel");
        }
        Tool::Eyedropper => {
            ui.weak("Click a shape to take its fill and stroke. The colour panel's dropper takes just one colour.");
        }
        _ => {}
    }
}
