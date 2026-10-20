//! Making sounds: Layer › New Sound (sound effects, and the music bed
//! dialog), and changing a generated sound's recipe in Properties
//! (re-generated in place, one undo step).

use crate::app::XenifyApp;
use egui::RichText;
use xenify_audio::synth::{Mood, MusicRecipe, Recipe, Sfx, SfxRecipe};
use xenify_doc::{Id, Layer, LayerKind, MediaAsset};
use xenify_ops::Command;

/// Layer › New Sound.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    for kind in Sfx::ALL {
        if ui.button(kind.label()).on_hover_text("A sound effect made by the program, at the playhead").clicked() {
            add(app, Recipe::Sfx(SfxRecipe::of(kind)));
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Music Bed…").on_hover_text("Background music: tempo, key, mood and instruments").clicked() {
        app.dialog = Some(crate::dialogs::Dialog::MusicBed(Box::new(MusicState::new())));
        ui.close();
    }
}

/// Adds a generated sound as an audio layer at the playhead.
pub fn add(app: &mut XenifyApp, recipe: Recipe) -> Option<Id> {
    let media = xenify_audio::synth::media_asset(&recipe, Id(app.project().next_id));
    let layer = Id(media.id.0 + 1);
    let name = recipe.name();
    match xenify_ops::media::add_clip(app.project(), app.comp, media, layer, app.time) {
        Ok(cmd) => {
            if app.exec(&format!("Add {name}"), cmd) {
                app.select_only(Some(layer));
                app.status =
                    format!("Added the “{name}” sound at the playhead. Change it in Properties › Audio › Sound.");
                return Some(layer);
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
    None
}

/// The Music Bed dialog.
pub struct MusicState {
    pub recipe: MusicRecipe,
    player: Option<xenify_platform::media::Player>,
}

impl MusicState {
    pub fn new() -> Self {
        Self { recipe: MusicRecipe::default(), player: None }
    }
}

/// The Music Bed dialog's body; returns true when done.
pub fn music_dialog(app: &mut XenifyApp, ui: &mut egui::Ui, state: &mut MusicState) -> bool {
    ui.label(
        "Background music made by the program: pick a feel, then add it to this canvas or the whole presentation.",
    );
    music_editor(ui, "music-dialog", &mut state.recipe);
    ui.separator();
    let mut close = false;
    ui.horizontal(|ui| {
        let playing = state.player.is_some();
        if ui.button(if playing { "Stop" } else { "▶ Preview" }).clicked() {
            state.player = if playing {
                None
            } else {
                xenify_platform::media::Player::start(Recipe::Music(state.recipe.clone()).render())
            };
            if !playing && state.player.is_none() {
                app.status = "No sound device found to preview on.".into();
            }
        }
        if ui.add(crate::widgets::primary("Add to Timeline")).clicked() {
            add(app, Recipe::Music(state.recipe.clone()));
            close = true;
        }
        if ui.button("Use as Soundtrack").on_hover_text("Plays across the whole presentation").clicked() {
            let media =
                xenify_audio::synth::media_asset(&Recipe::Music(state.recipe.clone()), Id(app.project().next_id));
            let looped = state.recipe.looped;
            let mut cmd = xenify_ops::media::add_to_soundtrack(app.project(), media, 0.0);
            if looped
                && let Command::Batch { commands } = &mut cmd
                && let Some(Command::SetSoundtrack { soundtrack }) = commands.last_mut()
                && let Some(c) = soundtrack.clips.last_mut()
            {
                c.audio.looped = true;
            }
            app.exec("Music soundtrack", cmd);
            close = true;
        }
        close |= ui.button("Cancel").clicked();
    });
    if close {
        state.player = None;
    }
    close
}

fn music_editor(ui: &mut egui::Ui, id: &str, m: &mut MusicRecipe) -> bool {
    let mut changed = false;
    egui::Grid::new(id).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Mood");
        ui.horizontal(|ui| {
            for mood in Mood::ALL {
                changed |= ui.selectable_value(&mut m.mood, mood, mood.label()).changed();
            }
        });
        ui.end_row();
        ui.label("Tempo");
        changed |= ui.add(egui::DragValue::new(&mut m.tempo).range(40.0..=220.0).speed(0.5).suffix(" bpm")).changed();
        ui.end_row();
        ui.label("Key");
        const KEYS: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
        egui::ComboBox::from_id_salt((id, "key")).selected_text(KEYS[m.key.rem_euclid(12) as usize]).show_ui(
            ui,
            |ui| {
                for (i, k) in KEYS.iter().enumerate() {
                    changed |= ui.selectable_value(&mut m.key, i as i32, *k).changed();
                }
            },
        );
        ui.end_row();
        ui.label("Bars");
        let mut bars = m.bars as f64;
        if ui.add(egui::DragValue::new(&mut bars).range(1.0..=64.0).speed(0.1)).changed() {
            m.bars = bars.round() as u32;
            changed = true;
        }
        ui.end_row();
        ui.label("Instruments");
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut m.pad, "Pad").changed();
            changed |= ui.checkbox(&mut m.arp, "Arpeggio").changed();
            changed |= ui.checkbox(&mut m.bass, "Bass").changed();
            changed |= ui.checkbox(&mut m.kick, "Kick").changed();
            changed |= ui.checkbox(&mut m.hat, "Hi-hat").changed();
        });
        ui.end_row();
        ui.label("");
        changed |= ui.checkbox(&mut m.looped, "Loops without a seam").changed();
        ui.end_row();
        ui.label("Volume");
        let mut v = m.volume * 100.0;
        if ui.add(egui::DragValue::new(&mut v).range(0.0..=100.0).speed(0.5).suffix(" %")).changed() {
            m.volume = v / 100.0;
            changed = true;
        }
        ui.end_row();
        ui.label("Variation");
        let mut seed = m.seed as f64;
        if ui.add(egui::DragValue::new(&mut seed).range(0.0..=9999.0).speed(0.2)).changed() {
            m.seed = seed.round() as u64;
            changed = true;
        }
        ui.end_row();
    });
    changed
}

fn sfx_editor(ui: &mut egui::Ui, id: &str, s: &mut SfxRecipe) -> bool {
    let mut changed = false;
    egui::Grid::new(id).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Sound");
        egui::ComboBox::from_id_salt((id, "kind")).selected_text(s.kind.label()).show_ui(ui, |ui| {
            for k in Sfx::ALL {
                if ui.selectable_value(&mut s.kind, k, k.label()).changed() {
                    s.length = k.length();
                    changed = true;
                }
            }
        });
        ui.end_row();
        let mut row = |ui: &mut egui::Ui,
                       label: &str,
                       tip: &str,
                       v: &mut f64,
                       range: std::ops::RangeInclusive<f64>,
                       speed: f64,
                       suffix: &str| {
            ui.label(label).on_hover_text(tip);
            changed |= ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix)).changed();
            ui.end_row();
        };
        row(ui, "Pitch", "Semitones up or down", &mut s.pitch, -24.0..=24.0, 0.1, " st");
        row(ui, "Length", "Seconds", &mut s.length, 0.02..=20.0, 0.01, " s");
        let mut bright = s.brightness * 100.0;
        row(ui, "Brightness", "Dull to bright", &mut bright, 0.0..=100.0, 0.5, " %");
        s.brightness = bright / 100.0;
        let mut decay = s.decay * 100.0;
        row(ui, "Ring", "How long it rings on", &mut decay, 10.0..=400.0, 1.0, " %");
        s.decay = decay / 100.0;
        let mut var = s.variation * 100.0;
        row(ui, "Variation", "How much the seed changes it", &mut var, 0.0..=100.0, 0.5, " %");
        s.variation = var / 100.0;
        let mut seed = s.seed as f64;
        row(ui, "Seed", "A different take", &mut seed, 0.0..=9999.0, 0.2, "");
        s.seed = seed.round() as u64;
        let mut vol = s.volume * 100.0;
        row(ui, "Volume", "How loud it's made", &mut vol, 0.0..=100.0, 0.5, " %");
        s.volume = vol / 100.0;
    });
    changed
}

/// Properties › Audio › Sound: the recipe of a generated clip, and a way to
/// make it again with changes.
pub fn recipe_editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, media: &MediaAsset) {
    let Some(recipe) = xenify_audio::synth::recipe_of(media) else { return };
    let key = egui::Id::new(("recipe-edit", layer.id));
    let mut edited: Recipe = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| recipe.clone());
    ui.label(RichText::new("Sound").strong()).on_hover_text("Made by the program: change it and make it again");
    let changed = match &mut edited {
        Recipe::Sfx(s) => sfx_editor(ui, &format!("sfx-{}", layer.id.0), s),
        Recipe::Music(m) => music_editor(ui, &format!("music-{}", layer.id.0), m),
    };
    if changed {
        ui.ctx().data_mut(|d| d.insert_temp(key, edited.clone()));
    }
    let dirty = edited != recipe;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(dirty, crate::widgets::primary("Make It Again"))
            .on_hover_text("Replace the sound with the new settings")
            .clicked()
        {
            regenerate(app, layer, &edited);
            ui.ctx().data_mut(|d| d.remove::<Recipe>(key));
        }
        if ui.button("▶ Preview").clicked() {
            app.sound.preview = xenify_platform::media::Player::start(edited.render());
            if app.sound.preview.is_none() {
                app.status = "No sound device found to preview on.".into();
            }
        }
    });
}

/// Swaps `layer`'s sound for `recipe`'s (a new clip; the layer keeps its place and settings).
fn regenerate(app: &mut XenifyApp, layer: &Layer, recipe: &Recipe) {
    let LayerKind::Audio(a) = &layer.kind else { return };
    let media = xenify_audio::synth::media_asset(recipe, Id(app.project().next_id));
    let mut audio = a.clone();
    audio.asset = media.id;
    let mut meta = layer.meta.clone();
    let end = meta.in_point + xenify_doc::media::play_length(media.duration, audio.source_in, audio.speed);
    if meta.out_point.is_some() {
        meta.out_point = Some(end);
    }
    let cmd = Command::batch(vec![
        Command::AddMedia { media },
        Command::SetAudio { layer: layer.id, audio },
        Command::SetLayerMeta { layer: layer.id, meta },
    ]);
    app.exec("Make the sound again", cmd);
}
