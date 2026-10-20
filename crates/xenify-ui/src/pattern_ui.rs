//! Pattern fills in Properties: fill a shape with a repeating picture (one
//! already in the project, or a file), then size, turn and shift the tiles.

use crate::app::XenifyApp;
use xenify_doc::{Id, Layer, Pattern, ShapeItem};
use xenify_ops::Command;

/// The Pattern row under a shape's fill.
pub fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, index: usize, shape: &ShapeItem) {
    let Some(fill) = &shape.fill else { return };
    let current = fill.pattern;
    let mut set: Option<Option<Pattern>> = None;
    ui.horizontal(|ui| {
        ui.label("Pattern").on_hover_text("Fill with a repeating picture (like SVG patterns)");
        let name = current.and_then(|p| app.project().asset(p.asset)).map_or("None".to_string(), |a| a.name.clone());
        egui::ComboBox::from_id_salt(("pattern", layer.id, index)).selected_text(name).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "None").clicked() {
                set = Some(None);
            }
            let assets: Vec<(Id, String)> = app.project().assets.iter().map(|a| (a.id, a.name.clone())).collect();
            for (id, name) in assets {
                if ui.selectable_label(current.is_some_and(|p| p.asset == id), name).clicked() {
                    set = Some(Some(current.map_or(Pattern::new(id), |p| Pattern { asset: id, ..p })));
                }
            }
            ui.separator();
            if ui.button("From a file…").clicked() {
                app.dialog = Some(crate::dialogs::Dialog::pattern_image(app, layer.id, index));
            }
        });
    });
    if let Some(mut p) = current {
        let before = p;
        ui.horizontal(|ui| {
            ui.label("Tiles");
            let r1 = ui.add(egui::DragValue::new(&mut p.scale).range(0.01..=50.0).speed(0.01).prefix("× "));
            let r2 = ui.add(egui::DragValue::new(&mut p.rotation).speed(0.5).suffix("°"));
            let r3 = ui.add(egui::DragValue::new(&mut p.offset.x).speed(0.5).prefix("x "));
            let r4 = ui.add(egui::DragValue::new(&mut p.offset.y).speed(0.5).prefix("y "));
            if [&r1, &r2, &r3, &r4].iter().any(|r| r.drag_started()) {
                app.new_gesture();
            }
        });
        if p != before {
            let mut new = shape.clone();
            new.fill.as_mut().unwrap().pattern = Some(p);
            app.exec_merged("Pattern", Command::SetShape { layer: layer.id, index, shape: new });
        }
    }
    if let Some(p) = set {
        let mut new = shape.clone();
        new.fill.as_mut().unwrap().pattern = p;
        app.exec("Pattern", Command::SetShape { layer: layer.id, index, shape: new });
    }
}

/// Uses the picture in `bytes` as shape `index` of `layer`'s pattern (adding
/// it to the project, or reusing an identical one). One undo step.
pub fn set_from_file(app: &mut XenifyApp, layer: Id, index: usize, bytes: &[u8], name: &str) {
    let img = match xenify_export::image::prepare(bytes, app.prefs.max_image_side) {
        Ok(img) => img,
        Err(e) => {
            app.fail(format!("{name}: {e}"));
            return;
        }
    };
    let mut project = app.project().clone();
    let (add, asset) =
        xenify_ops::assets::add_image(&mut project, name, img.mime, img.width, img.height, img.data, None);
    let Some(mut shape) = app.project().layer(layer).ok().and_then(|l| l.shapes().get(index).cloned()) else { return };
    let Some(fill) = shape.fill.as_mut() else {
        app.status = "Turn the shape's fill on to give it a pattern.".into();
        return;
    };
    fill.pattern = Some(fill.pattern.map_or(Pattern::new(asset), |p| Pattern { asset, ..p }));
    let cmds: Vec<Command> = add.into_iter().chain([Command::SetShape { layer, index, shape }]).collect();
    if app.exec("Pattern from file", Command::batch(cmds)) {
        app.status = format!("Filled with a {name} pattern. Size and turn the tiles under Pattern.");
    }
}
