//! The Slices panel: the named export regions of this canvas — rename,
//! resize, delete, and export them all.

use crate::app::{XenifyApp, Tool};
use egui::RichText;
use xenify_ops::Command;

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let n = app.comp().settings.slices.len();
    ui.horizontal(|ui| {
        if ui.button("+ Draw slice").on_hover_text("Slice tool (Shift+C): drag out a region on the canvas").clicked() {
            app.set_tool(Tool::Slice);
        }
        if ui.add_enabled(n > 0, egui::Button::new("Export Slices…")).clicked() {
            app.dialog = Some(crate::dialogs::Dialog::export_slices(app));
        }
        let mut show = app.prefs.show_slices;
        if ui.checkbox(&mut show, "Show on canvas").changed() {
            app.prefs.show_slices = show;
        }
    });
    ui.separator();
    if n == 0 {
        ui.weak("No slices yet. Slices are named parts of the canvas that export as their own files (icons, banners, social crops…).");
        return;
    }
    let u = app.comp().settings.units;
    let mut settings = app.comp().settings.clone();
    let mut remove = None;
    let selected = app.viewport.selected_slice();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("slices").num_columns(4).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new("Name").weak());
            ui.label(RichText::new("Position").weak());
            ui.label(RichText::new("Size").weak());
            ui.label("");
            ui.end_row();
            for (i, s) in settings.slices.iter_mut().enumerate() {
                let r = ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(120.0));
                if r.gained_focus() || r.clicked() {
                    app.viewport.select_slice(Some(i));
                }
                if selected == Some(i) {
                    r.highlight();
                }
                let (mut x, mut y) = (u.from_px(s.rect.x0), u.from_px(s.rect.y0));
                let (mut w, mut h) = (u.from_px(s.rect.width()), u.from_px(s.rect.height()));
                let speed = u.from_px(1.0);
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut x).speed(speed).max_decimals(u.decimals()));
                    ui.add(egui::DragValue::new(&mut y).speed(speed).max_decimals(u.decimals()));
                });
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut w).speed(speed).range(0.0..=f64::MAX).max_decimals(u.decimals()));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut h).speed(speed).range(0.0..=f64::MAX).max_decimals(u.decimals()));
                });
                let r =
                    kurbo::Rect::from_origin_size((u.to_px(x), u.to_px(y)), (u.to_px(w).max(1.0), u.to_px(h).max(1.0)));
                if (r.x0 - s.rect.x0).abs() > 1e-9
                    || (r.y0 - s.rect.y0).abs() > 1e-9
                    || (r.width() - s.rect.width()).abs() > 1e-9
                    || (r.height() - s.rect.height()).abs() > 1e-9
                {
                    s.rect = r;
                }
                if ui.small_button("Delete").on_hover_text("Delete this slice").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    });
    if let Some(i) = remove {
        settings.slices.remove(i);
        app.viewport.select_slice(None);
    }
    if settings != app.comp().settings {
        let label = if remove.is_some() { "Delete slice" } else { "Edit slice" };
        app.exec_merged(label, Command::SetCompSettings { comp: app.comp, settings });
    }
    ui.add_space(4.0);
    ui.weak(format!("Units: {} (Canvas › Canvas Settings to change)", u.unit.label()));
}
