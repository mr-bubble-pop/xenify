//! Slide › Soundtrack…: sound that plays across the whole presentation
//! (music, usually), each clip starting at a point in the deck, and whether
//! it keeps playing while presenting waits at a pause.

use crate::app::XenifyApp;
use crate::file_browser::{FileBrowser, Mode};
use egui::RichText;
use xenify_doc::Id;

/// Sound files the soundtrack takes.
const SOUND_EXTENSIONS: &[&str] =
    &["wav", "mp3", "ogg", "oga", "opus", "flac", "m4a", "aac", "mp4", "webm", "mov", "mkv"];

/// The dialog's state: the file browser while choosing a file.
pub struct State {
    pub browser: Option<FileBrowser>,
}

impl State {
    pub fn new() -> Self {
        Self { browser: None }
    }
}

/// The dialog's body. Returns true when it's done.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, state: &mut State) -> bool {
    if let Some(browser) = &mut state.browser {
        ui.label("Choose a sound file (or a video: its sound is used).");
        match browser.ui(ui, "Add") {
            Some(Ok(path)) => {
                app.last_dir = Some(browser.dir().to_path_buf());
                let name = path.file_stem().map_or("Sound".into(), |s| s.to_string_lossy().into_owned());
                match crate::import::media_asset(&path, &name, Id(app.project().next_id)) {
                    Ok(media) if media.has_audio || !media.is_video() => {
                        let mut media = media;
                        media.kind = xenify_doc::MediaKind::Audio;
                        let cmd = xenify_ops::media::add_to_soundtrack(app.project(), media, 0.0);
                        app.exec("Add to soundtrack", cmd);
                    }
                    Ok(_) => app.fail(format!("{name} has no sound")),
                    Err(e) => app.fail(e),
                }
                state.browser = None;
            }
            Some(Err(())) => state.browser = None,
            None => {}
        }
        return false;
    }
    let deck_length = xenify_anim::deck::Deck::new(app.project()).duration();
    ui.label(format!("Plays across the whole presentation ({deck_length:.1} s), under each slide's own sound."));
    let mut soundtrack = app.project().soundtrack.clone();
    let before = soundtrack.clone();
    let mut remove = None;
    let mut begin = false;
    if soundtrack.clips.is_empty() {
        ui.weak("No soundtrack yet.");
    }
    egui::Grid::new("soundtrack").num_columns(6).spacing([8.0, 4.0]).show(ui, |ui| {
        for (i, clip) in soundtrack.clips.iter_mut().enumerate() {
            let name = app.project().media(clip.audio.asset).map_or("(missing)".into(), |m| m.name.clone());
            ui.label(RichText::new(name).strong());
            ui.label("from");
            begin |= ui
                .add(egui::DragValue::new(&mut clip.start).range(0.0..=36_000.0).speed(0.05).suffix(" s"))
                .drag_started();
            let mut vol = clip.audio.volume.value_at(0.0) * 100.0;
            let r = ui.add(egui::DragValue::new(&mut vol).range(0.0..=400.0).speed(0.5).prefix("volume ").suffix(" %"));
            begin |= r.drag_started();
            if r.changed() {
                clip.audio.volume = (vol / 100.0).into();
            }
            ui.checkbox(&mut clip.audio.looped, "Loop");
            if ui.button("Remove").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        soundtrack.clips.remove(i);
    }
    ui.checkbox(&mut soundtrack.keep_playing, "Keep playing while presenting waits at a pause")
        .on_hover_text("Off: the soundtrack holds with the slides");
    if begin {
        app.new_gesture();
    }
    if soundtrack != before {
        let cmd = xenify_ops::media::edit_soundtrack(app.project(), |s| *s = soundtrack);
        app.exec_merged("Change soundtrack", cmd);
    }
    ui.separator();
    let mut close = false;
    ui.horizontal(|ui| {
        if ui.add(crate::widgets::primary("Add Sound File…")).clicked() {
            state.browser =
                Some(FileBrowser::new(Mode::Open, &crate::dialogs::Dialog::start_dir(app), SOUND_EXTENSIONS));
        }
        close |= ui.button("Done").clicked();
    });
    close
}
