//! Properties › Animation for animated images (GIF, WebP, APNG): speed,
//! looping and where it starts.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{ImagePlayback, Layer, LayerKind};
use xenify_ops::Command;

pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let LayerKind::Image { asset, playback } = layer.kind else { return };
    let Some(a) = app.project().asset(asset) else { return };
    let Some(frames) = a.frames.as_ref().map(Vec::len) else { return };
    let length = a.animation_length();
    let mut p = playback;
    let mut begin = false;
    egui::CollapsingHeader::new(RichText::new("Animation").strong()).default_open(true).show(ui, |ui| {
        ui.weak(format!("An animated image: {frames} frames, {length:.2} s a pass."));
        egui::Grid::new(("image-anim", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            ui.label("Speed").on_hover_text("1× as made; 2× twice as fast; 0 stops on the start frame");
            let r = ui.add(egui::DragValue::new(&mut p.speed).range(0.0..=16.0).speed(0.01).suffix("×"));
            begin |= r.drag_started();
            ui.end_row();
            ui.label("Starts At").on_hover_text("How far into the animation it is when the layer comes in");
            let r = ui.add(egui::DragValue::new(&mut p.offset).range(0.0..=length.max(0.0)).speed(0.01).suffix(" s"));
            begin |= r.drag_started();
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut p.looped, "Loop Animation")
                .on_hover_text("Play over and over (else once, then hold the last frame)");
            ui.end_row();
        });
        if ui.button("Reset Playback").clicked() {
            p = ImagePlayback::default();
        }
    });
    if begin {
        app.new_gesture();
    }
    if p != playback {
        app.exec("Animated image playback", Command::SetImagePlayback { layer: layer.id, playback: p });
    }
}
