//! File ▸ Settings: general preferences, keyboard shortcuts and the AI setup.

use crate::app::XenifyApp;
use crate::keymap::{Action, Category, Chord};
use crate::widgets::ACCENT;
use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};

/// eframe storage key for [`Prefs`].
pub const PREFS_KEY: &str = "prefs_v1";

/// General preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Seconds between autosaves of unsaved work.
    pub autosave_secs: u64,
    /// Files kept in File ▸ Open Recent.
    pub recent_max: usize,
    /// Imported images larger than this (longest side, pixels) are scaled down.
    pub max_image_side: u32,
    /// Imported 3D models with more triangles are simplified to this many
    /// (0 = keep every triangle).
    pub model_triangles: usize,
    /// Open the files that were open last time.
    pub reopen_tabs: bool,
    pub show_rulers: bool,
    pub show_grid: bool,
    pub show_guides: bool,
    /// Draw slices on the canvas even when the Slice tool isn't in use.
    pub show_slices: bool,
    /// Snap to the frame and other objects' edges and centres.
    pub snap_objects: bool,
    pub snap_guides: bool,
    pub snap_grid: bool,
    /// Also snap this far (pixels) from the frame's and other objects' edges; 0 = off.
    pub snap_margin: f64,
    /// Interface size (1.0 = normal).
    pub ui_scale: f32,
    /// Brighter text and stronger outlines.
    pub high_contrast: bool,
    /// No sliding or fading in the interface, and still previews in the
    /// galleries (the document's own animation still plays).
    pub reduce_motion: bool,
    /// Keyframe every change at the playhead (Auto Keying), remembered
    /// between sessions.
    pub auto_key: bool,
    /// Open the New dialog when Xenify Vector starts.
    pub show_start: bool,
    /// Play the project's sound while it plays.
    pub play_sound: bool,
    /// Emoji used lately (the Emoji panel's Recent), newest first.
    pub recent_emoji: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            autosave_secs: 60,
            recent_max: 10,
            max_image_side: 4096,
            model_triangles: 30_000,
            reopen_tabs: true,
            show_rulers: true,
            show_grid: false,
            show_guides: true,
            show_slices: false,
            snap_objects: true,
            snap_guides: true,
            snap_grid: false,
            snap_margin: 0.0,
            ui_scale: 1.0,
            high_contrast: false,
            reduce_motion: false,
            auto_key: true,
            show_start: true,
            play_sound: true,
            recent_emoji: Vec::new(),
        }
    }
}

impl Prefs {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage.and_then(|s| eframe::get_value(s, PREFS_KEY)).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    General,
    Shortcuts,
    Ai,
}

/// The Settings window's state.
#[derive(Default)]
pub struct SettingsUi {
    pub is_open: bool,
    tab: Tab,
    filter: String,
    /// Waiting for a key to bind to this action.
    pub recording: Option<Action>,
    /// A recorded chord that's already used: (action, chord, actions using it).
    clash: Option<(Action, Chord, Vec<Action>)>,
    /// Closing with unsaved AI settings: asking to save or discard them.
    confirm_close: bool,
}

impl SettingsUi {
    pub fn open(&mut self, tab: Tab) {
        self.is_open = true;
        self.tab = tab;
    }
}

pub fn window(app: &mut XenifyApp, ctx: &egui::Context) {
    if !app.settings.is_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Settings")
        .open(&mut open)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(620.0)
        .default_height(560.0)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [(Tab::General, "General"), (Tab::Shortcuts, "Shortcuts"), (Tab::Ai, "AI")] {
                    ui.selectable_value(&mut app.settings.tab, tab, label);
                }
            });
            if app.settings.confirm_close {
                egui::Frame::new().fill(crate::widgets::RECORD.gamma_multiply(0.25)).inner_margin(6.0).show(ui, |ui| {
                    ui.label(RichText::new("Your AI settings (or the key you pasted) aren't saved yet.").strong());
                    ui.horizontal(|ui| {
                        if ui.button("Save and Close").clicked() {
                            crate::assistant::save_ai_settings(app);
                            app.settings.confirm_close = false;
                            app.settings.is_open = false;
                        }
                        if ui.button("Discard").clicked() {
                            crate::assistant::discard_ai_settings(app);
                            app.settings.confirm_close = false;
                            app.settings.is_open = false;
                        }
                        if ui.button("Keep Editing").clicked() {
                            app.settings.confirm_close = false;
                        }
                    });
                });
            }
            ui.separator();
            match app.settings.tab {
                Tab::General => general(app, ui),
                Tab::Shortcuts => shortcuts(app, ui),
                Tab::Ai => {
                    egui::ScrollArea::vertical().show(ui, |ui| crate::assistant::ai_settings_ui(app, ui));
                }
            }
        });
    // Closing with unsaved AI settings asks first.
    if !open && crate::assistant::ai_settings_unsaved(app) {
        app.settings.confirm_close = true;
        app.settings.tab = Tab::Ai;
        open = true;
    }
    if !open || !app.settings.is_open {
        app.settings.is_open = false;
        app.settings.recording = None;
        app.settings.clash = None;
        app.assistant.settings_open = false;
    }
}

fn general(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let p = &mut app.prefs;
    egui::Grid::new("prefs").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        ui.label("Autosave every");
        ui.add(egui::DragValue::new(&mut p.autosave_secs).range(10..=3600).suffix(" s"))
            .on_hover_text("Unsaved work is kept safe in the background, and offered back after a crash");
        ui.end_row();
        ui.label("Recent files");
        ui.add(egui::DragValue::new(&mut p.recent_max).range(1..=50));
        ui.end_row();
        ui.label("Largest imported image");
        ui.add(egui::DragValue::new(&mut p.max_image_side).range(256..=16384).suffix(" px"))
            .on_hover_text("Bigger images are scaled down when imported or pasted (longest side)");
        ui.end_row();
        ui.label("3D model detail");
        let label = |n: usize| match n {
            0 => "Full (every triangle)".to_string(),
            n => format!("Up to {}k triangles", n / 1000),
        };
        egui::ComboBox::from_id_salt("model-detail")
            .selected_text(label(p.model_triangles))
            .show_ui(ui, |ui| {
                for n in [10_000, 30_000, 100_000, 300_000, 0] {
                    ui.selectable_value(&mut p.model_triangles, n, label(n));
                }
            })
            .response
            .on_hover_text(
                "Denser models are simplified on import (smoother playback). More detail looks finer but plays slower",
            );
        ui.end_row();
        ui.label("Interface size");
        let mut pct = (p.ui_scale * 100.0).round();
        // Applied when the slider is let go (resizing under the pointer mid-drag jumps).
        let key = egui::Id::new("ui-scale-draft");
        if let Some(d) = ui.data(|d| d.get_temp::<f32>(key)) {
            pct = d;
        }
        let r = ui
            .add(egui::Slider::new(&mut pct, 75.0..=200.0).step_by(5.0).suffix("%"))
            .on_hover_text("Makes all text and controls bigger or smaller (Ctrl + / Ctrl − zoom the canvas instead)");
        if r.dragged() {
            ui.data_mut(|d| d.insert_temp(key, pct));
        } else {
            ui.data_mut(|d| d.remove::<f32>(key));
            if r.drag_stopped() || r.changed() {
                p.ui_scale = pct / 100.0;
            }
        }
        ui.end_row();
        ui.label("Contrast");
        ui.checkbox(&mut p.high_contrast, "High contrast (brighter text, stronger outlines)");
        ui.end_row();
        ui.label("Motion");
        ui.checkbox(&mut p.reduce_motion, "Reduce motion").on_hover_text(
            "No sliding or fading menus and panels, no smooth scrolling, and still pictures instead of moving \
             previews in the Effects and Transitions galleries. Your document's animation still plays.",
        );
        ui.end_row();
        ui.label("On start");
        ui.checkbox(&mut p.reopen_tabs, "Reopen the files that were open last time");
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.show_start, "Open the New dialog when Xenify Vector starts");
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.play_sound, "Play sound while playing and presenting");
        ui.end_row();
        ui.label("Animation");
        ui.checkbox(&mut app.auto_key, "Keyframe every change (Auto Keying)").on_hover_text(
            "On: any change at the playhead adds a keyframe, so everything can animate without clicking \
             each property's ⏱. Off: changes apply to the whole clip unless a property is already animated. \
             Also the Auto Keying button in the timeline",
        );
        ui.end_row();
    });
    ui.add_space(8.0);
    if ui.button("Restore defaults").clicked() {
        app.prefs = Prefs::default();
    }
}

fn shortcuts(app: &mut XenifyApp, ui: &mut egui::Ui) {
    record(app, ui);
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(egui::TextEdit::singleline(&mut app.settings.filter).desired_width(220.0).hint_text("action or key"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Reset all").on_hover_text("Back to the standard shortcuts").clicked() {
                app.keymap.reset_all();
            }
        });
    });
    if let Some((action, chord, others)) = app.settings.clash.clone() {
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let names: Vec<String> = others.iter().map(|a| a.label()).collect();
            ui.colored_label(
                Color32::from_rgb(240, 170, 80),
                format!("{} is already used by {}.", chord.text(), names.join(", ")),
            );
            ui.horizontal(|ui| {
                if ui.button(format!("Use it for {} instead", action.label())).clicked() {
                    for other in &others {
                        let kept: Vec<Chord> =
                            app.keymap.chords(*other).iter().copied().filter(|c| *c != chord).collect();
                        app.keymap.set(*other, kept);
                    }
                    add_chord(app, action, chord);
                    app.settings.clash = None;
                }
                if ui.button("Cancel").clicked() {
                    app.settings.clash = None;
                }
            });
        });
    }
    ui.add_space(4.0);
    let filter = app.settings.filter.to_lowercase();
    let all = Action::all();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for cat in Category::ALL {
            let rows: Vec<Action> = all
                .iter()
                .copied()
                .filter(|a| a.category() == cat)
                .filter(|a| {
                    filter.is_empty()
                        || a.label().to_lowercase().contains(&filter)
                        || app.keymap.all_text(*a).to_lowercase().contains(&filter)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.add_space(6.0);
            ui.label(RichText::new(cat.label()).strong().color(ACCENT));
            egui::Grid::new(("keys", cat.label())).num_columns(3).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for a in rows {
                    shortcut_row(app, ui, a);
                    ui.end_row();
                }
            });
        }
    });
}

fn shortcut_row(app: &mut XenifyApp, ui: &mut egui::Ui, a: Action) {
    ui.label(a.label());
    ui.horizontal(|ui| {
        if a.fixed() {
            ui.label(RichText::new(app.keymap.all_text(a)).monospace())
                .on_hover_text("Set by the system's clipboard keys, so it can't be changed here");
            return;
        }
        let chords = app.keymap.chords(a).to_vec();
        for chord in &chords {
            // The shortcut, then its own small remove button (a stray click
            // on the shortcut itself changes nothing).
            egui::Frame::new().fill(ui.visuals().widgets.inactive.bg_fill).corner_radius(3.0).inner_margin(2.0).show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.label(RichText::new(chord.text()).monospace());
                    let x = egui::Button::new("×").frame(false).small();
                    if ui.add(x).on_hover_text("Remove this shortcut").clicked() {
                        let kept: Vec<Chord> = chords.iter().copied().filter(|c| c != chord).collect();
                        app.keymap.set(a, kept);
                    }
                },
            );
        }
        let recording = app.settings.recording == Some(a);
        let label = if recording { "press keys… (Esc cancels)" } else { "+" };
        if ui.selectable_label(recording, label).on_hover_text("Add a shortcut").clicked() {
            app.settings.recording = if recording { None } else { Some(a) };
            app.settings.clash = None;
        }
    });
    if app.keymap.is_default(a) {
        ui.label("");
    } else if ui.small_button("Reset").on_hover_text("Back to the standard shortcut").clicked() {
        app.keymap.reset(a);
    }
}

/// While recording, the next key press (with its modifiers) becomes a shortcut.
fn record(app: &mut XenifyApp, ui: &egui::Ui) {
    let Some(action) = app.settings.recording else { return };
    let pressed = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
            _ => None,
        })
    });
    let Some((key, mods)) = pressed else { return };
    app.settings.recording = None;
    if key == egui::Key::Escape && mods.is_none() {
        return;
    }
    let mods = egui::Modifiers { command: mods.command || mods.ctrl, ctrl: false, mac_cmd: false, ..mods };
    let chord = Chord::new(mods, key);
    let others = app.keymap.conflicts_with(action, chord);
    if others.is_empty() {
        add_chord(app, action, chord);
    } else {
        app.settings.clash = Some((action, chord, others));
    }
}

fn add_chord(app: &mut XenifyApp, action: Action, chord: Chord) {
    let mut chords = app.keymap.chords(action).to_vec();
    if !chords.contains(&chord) {
        chords.push(chord);
    }
    app.keymap.set(action, chords);
}

/// Applies the interface size and contrast preferences (cheap when unchanged).
pub fn apply_display(ctx: &egui::Context, prefs: &Prefs) {
    // Ctrl + / − / 0 zoom the canvas, not the whole interface.
    if ctx.options(|o| o.zoom_with_keyboard) {
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
    }
    let scale = prefs.ui_scale.clamp(0.5, 3.0);
    if (ctx.zoom_factor() - scale).abs() > 1e-3 {
        ctx.set_zoom_factor(scale);
    }
    let key = egui::Id::new("display-applied");
    let wanted = (prefs.high_contrast, prefs.reduce_motion);
    if ctx.data(|d| d.get_temp::<(bool, bool)>(key)) == Some(wanted) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, wanted));
    let tokens = if prefs.high_contrast { crate::theme::Tokens::HIGH_CONTRAST } else { crate::theme::Tokens::DARK };
    crate::theme::apply(ctx, tokens);
    crate::theme::set_reduce_motion(ctx, prefs.reduce_motion);
}
