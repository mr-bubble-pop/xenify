//! Video and audio layers in Properties: the clip (length, size, where its
//! file is), playback (start in the clip, speed, loop, mute, volume ⏱,
//! fades), Fit / Fill, and Trim to Clip.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{Layer, LayerKind, MediaSource, PropPath};

/// Properties › Video / Audio.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let (asset, title) = match &layer.kind {
        LayerKind::Video(v) => (v.asset, "Video"),
        LayerKind::Audio(a) => (a.asset, "Audio"),
        _ => return,
    };
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, |ui| {
        let Some(media) = app.project().media(asset).cloned() else {
            ui.colored_label(egui::Color32::LIGHT_RED, "The clip is missing from the project.");
            return;
        };
        let mut facts = vec![format!("{:.1} s", media.duration)];
        if media.is_video() {
            facts.push(format!("{}×{}", media.width, media.height));
            facts.push(format!("{} fps", (media.fps * 100.0).round() / 100.0));
            facts.push(if media.has_audio { "with sound".into() } else { "no sound".into() });
        }
        ui.label(format!("{} · {}", media.name, facts.join(" · ")));
        match &media.source {
            MediaSource::Linked { path } => {
                ui.weak(format!("Linked: {path}"))
                    .on_hover_text("The project refers to this file; keep it where it is");
                if ui
                    .button("Keep Inside the Project File")
                    .on_hover_text("The clip travels with the project (the file gets bigger)")
                    .clicked()
                {
                    match std::fs::read(path) {
                        Ok(bytes) => {
                            if let Ok(cmd) = xenify_ops::media::set_source(
                                app.project(),
                                asset,
                                MediaSource::Embedded { data: bytes.into() },
                            ) {
                                app.exec("Embed clip", cmd);
                            }
                        }
                        Err(e) => app.fail(format!("Couldn't read {path}: {e}")),
                    }
                }
            }
            MediaSource::Embedded { data } => {
                ui.weak(format!("Inside the project file ({:.1} MB)", data.len() as f64 / 1e6));
            }
        }
        crate::sound_gen_ui::recipe_editor(app, ui, layer, &media);
        ui.add_space(4.0);
        playback(app, ui, layer);
        egui::Grid::new(("media-volume", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            crate::panels::prop_row(app, ui, layer, PropPath::Volume);
        });
        ui.horizontal(|ui| {
            if matches!(layer.kind, LayerKind::Video(_)) {
                for (label, fill, tip) in [
                    ("Fit Frame", false, "The whole picture inside the frame"),
                    ("Fill Frame", true, "Cover the frame (edges may be cut off)"),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked()
                        && let Ok(cmd) = xenify_ops::media::fit(app.project(), layer.id, app.time, fill, app.auto_key)
                    {
                        app.exec(label, cmd);
                    }
                }
            }
            if ui.button("Trim to Clip").on_hover_text("Make the layer as long as the clip plays").clicked()
                && let Ok(cmd) = xenify_ops::media::fit_length(app.project(), layer.id)
            {
                app.exec("Trim to clip", cmd);
            }
        });
        ui.weak("Crop: Layer › Crop Selection…  Move and scale: the Select tool.");
    });
}

/// Where in the clip it starts, speed, loop, mute (and fades for audio).
fn playback(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let mut begin = false;
    match &layer.kind {
        LayerKind::Video(v) => {
            let mut b = (**v).clone();
            begin |= common(ui, layer, &mut b.source_in, &mut b.speed, &mut b.looped, &mut b.muted);
            if begin {
                app.new_gesture();
            }
            if b != **v
                && let Ok(cmd) = xenify_ops::media::edit_video(app.project(), layer.id, |x| *x = b)
            {
                app.exec_merged("Change video", cmd);
            }
        }
        LayerKind::Audio(a) => {
            let mut b = (**a).clone();
            begin |= common(ui, layer, &mut b.source_in, &mut b.speed, &mut b.looped, &mut b.muted);
            egui::Grid::new(("audio-fades", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
                ui.label("Fade in");
                begin |= ui
                    .add(egui::DragValue::new(&mut b.fade_in).range(0.0..=60.0).speed(0.01).suffix(" s"))
                    .drag_started();
                ui.end_row();
                ui.label("Fade out");
                begin |= ui
                    .add(egui::DragValue::new(&mut b.fade_out).range(0.0..=60.0).speed(0.01).suffix(" s"))
                    .drag_started();
                ui.end_row();
            });
            if begin {
                app.new_gesture();
            }
            if b != **a
                && let Ok(cmd) = xenify_ops::media::edit_audio(app.project(), layer.id, |x| *x = b)
            {
                app.exec_merged("Change audio", cmd);
            }
        }
        _ => {}
    }
}

fn common(
    ui: &mut egui::Ui,
    layer: &Layer,
    source_in: &mut f64,
    speed: &mut f64,
    looped: &mut bool,
    muted: &mut bool,
) -> bool {
    let mut begin = false;
    egui::Grid::new(("media-playback", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label("Starts at").on_hover_text("Where in the clip it begins (trim the start)");
        begin |= ui.add(egui::DragValue::new(source_in).range(0.0..=86_400.0).speed(0.01).suffix(" s")).drag_started();
        ui.end_row();
        ui.label("Speed").on_hover_text("100% as recorded; 50% slow motion; 200% twice as fast");
        let mut pct = *speed * 100.0;
        let r = ui.add(egui::DragValue::new(&mut pct).range(5.0..=1000.0).speed(1.0).suffix(" %"));
        begin |= r.drag_started();
        *speed = pct / 100.0;
        ui.end_row();
    });
    ui.horizontal(|ui| {
        ui.checkbox(looped, "Loop Clip").on_hover_text("Start again from the beginning when it ends");
        ui.checkbox(muted, "Mute");
    });
    begin
}
