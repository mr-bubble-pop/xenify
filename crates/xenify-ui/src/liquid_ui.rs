//! Liquid layers in the UI: Layer › New › Water (fill the screen, or fill
//! the selected layer, e.g. a title) and Properties › Water. The level can
//! also be dragged on the canvas (`viewport/emitter_handle.rs`).

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{Id, Layer, LayerKind, LiquidField, PropPath};

/// Layer › New › Water.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    if ui.button("Fill the Screen").on_hover_text("Water rises until it fills the canvas").clicked() {
        let id = Id(app.project().next_id);
        match xenify_ops::liquid::fill_screen(app.project(), app.comp, id, app.time) {
            Ok(cmd) => {
                if app.exec("Water fills the screen", cmd) {
                    app.select_only(Some(id));
                    app.status = "Water rises from the playhead. Keyframe Level to make it rise or drain.".into();
                }
            }
            Err(e) => app.fail(e.to_string()),
        }
        ui.close();
    }
    let target = app.selected.filter(|id| {
        app.project().layer(*id).is_ok_and(|l| !matches!(l.kind, LayerKind::Liquid(_) | LayerKind::Camera(_)))
    });
    if ui
        .add_enabled(target.is_some(), egui::Button::new("Fill the Selected Layer"))
        .on_hover_text("Water rises inside the selected layer's shape (a title, a logo…)")
        .on_disabled_hover_text("Select the layer to fill first")
        .clicked()
    {
        fill_selected(app, target.unwrap());
        ui.close();
    }
}

fn fill_selected(app: &mut XenifyApp, target: Id) {
    let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
    let bounds = xenify_anim::world_box(app.comp(), &scene, target, &mut app.viewport.measurer());
    let Some(bounds) = bounds else {
        app.fail("That layer isn't visible now, so there's nothing to fill.");
        return;
    };
    let id = Id(app.project().next_id);
    match xenify_ops::liquid::fill_layer(app.project(), app.comp, id, target, bounds, app.time) {
        Ok(cmd) => {
            if app.exec("Fill with water", cmd) {
                app.select_only(Some(id));
                app.status = "The layer is now the water's mask (Set Mask). Keyframe Level to fill or drain it.".into();
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

/// Properties › Water.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let LayerKind::Liquid(w) = &layer.kind else { return };
    let w = (**w).clone();
    let mut b = w.clone();
    let mut begin = false;
    egui::CollapsingHeader::new(RichText::new("Water").strong()).default_open(true).show(ui, |ui| {
        ui.weak("Drag the level handle on the canvas, or keyframe Level to fill and drain.");
        egui::Grid::new(("liquid", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            for (f, label) in [
                (LiquidField::Level, "Level (0–1)"),
                (LiquidField::Tilt, "Tilt (°)"),
                (LiquidField::WaveHeight, "Wave Height (px)"),
                (LiquidField::Color, "Colour"),
            ] {
                crate::panels::prop_row_named(app, ui, layer, PropPath::Liquid(f), label);
            }
        });
        egui::Grid::new(("liquid-2", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            let num = |ui: &mut egui::Ui, label: &str, tip: &str, v: &mut f64, range, speed: f64, suffix: &str| {
                ui.label(label).on_hover_text(tip);
                let r = ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix));
                ui.end_row();
                r.drag_started()
            };
            begin |= num(ui, "Width", "The container's width", &mut b.size.x, 1.0..=100_000.0, 1.0, " px");
            begin |= num(ui, "Height", "The container's height", &mut b.size.y, 1.0..=100_000.0, 1.0, " px");
            begin |= num(ui, "Rises towards", "-90° = up", &mut b.direction, -360.0..=360.0, 1.0, "°");
            begin |= num(ui, "Slosh", "Rocking back and forth", &mut b.slosh, 0.0..=45.0, 0.1, "°");
            begin |= num(ui, "Slosh speed", "Rocks per second", &mut b.slosh_speed, 0.0..=10.0, 0.01, " /s");
            let mut waves = b.waves as f64;
            begin |= num(ui, "Wave trains", "Layers of waves (0: flat)", &mut waves, 0.0..=6.0, 0.05, "");
            b.waves = waves.round() as u32;
            begin |= num(ui, "Wave length", "Crest to crest", &mut b.wave_length, 1.0..=100_000.0, 1.0, " px");
            begin |= num(ui, "Wave speed", "Wavelengths per second", &mut b.wave_speed, -10.0..=10.0, 0.01, " /s");
            let mut bubbles = b.bubbles as f64;
            begin |= num(ui, "Bubbles", "How many rise at once", &mut bubbles, 0.0..=500.0, 0.2, "");
            b.bubbles = bubbles.round() as u32;
            begin |= num(ui, "Foam", "A bright line on the surface (0: none)", &mut b.foam, 0.0..=100.0, 0.1, " px");
            ui.label("Deeper colour").on_hover_text("A gradient down to this colour");
            ui.horizontal(|ui| {
                let mut on = b.deep_color.is_some();
                ui.checkbox(&mut on, "");
                if on {
                    let mut c = b.deep_color.unwrap_or(xenify_doc::Color::rgba(0.02, 0.2, 0.45, 0.95));
                    crate::color_picker::button(ui, ("deep", layer.id.0), &mut c);
                    b.deep_color = Some(c);
                } else {
                    b.deep_color = None;
                }
            });
            ui.end_row();
        });
    });
    if begin {
        app.new_gesture();
    }
    if b != w
        && let Ok(cmd) = xenify_ops::liquid::edit(app.project(), layer.id, |x| *x = b)
    {
        app.exec_merged("Change water", cmd);
    }
}
