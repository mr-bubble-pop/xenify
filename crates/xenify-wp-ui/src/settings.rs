//! File › Settings: everything about how Xenify Write behaves for you, in
//! one place. Sections down the left; everyday options shown, the rest
//! folded under "Advanced".

use crate::app::WriteApp;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    General,
    Appearance,
    Editing,
    Saving,
    Proofing,
    Assistant,
}

impl Section {
    const ALL: [Section; 6] = [
        Section::General,
        Section::Appearance,
        Section::Editing,
        Section::Saving,
        Section::Proofing,
        Section::Assistant,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Editing => "Typing",
            Section::Saving => "Saving",
            Section::Proofing => "Spelling and grammar",
            Section::Assistant => "AI assistant",
        }
    }
}

/// The settings window's state: open, and on which section.
#[derive(Default)]
pub struct SettingsState {
    pub open: bool,
    pub section: Section,
}

/// A titled group with room around it.
pub fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(4.0);
    ui.indent(title, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        add(ui);
    });
}

pub fn window(app: &mut WriteApp, ctx: &egui::Context) {
    if !app.settings.open {
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
        ui.set_width(660.0);
        ui.set_height(440.0);
        ui.horizontal(|ui| {
            ui.heading("Settings");
        });
        ui.separator();
        ui.horizontal_top(|ui| {
            // Sections.
            ui.allocate_ui_with_layout(
                egui::vec2(170.0, 370.0),
                egui::Layout::top_down_justified(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
                    for s in Section::ALL {
                        if ui.selectable_label(app.settings.section == s, s.name()).clicked() {
                            app.settings.section = s;
                        }
                    }
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().id_salt("settings-body").max_height(370.0).show(ui, |ui| {
                    ui.set_width(450.0);
                    match app.settings.section {
                        Section::General => general(app, ui),
                        Section::Appearance => appearance(app, ui),
                        Section::Editing => typing(app, ui),
                        Section::Saving => saving(app, ui),
                        Section::Proofing => crate::proofing::settings(app, ui),
                        Section::Assistant => crate::assistant::settings(app, ui),
                    }
                });
            });
        });
        ui.separator();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                app.settings.open = false;
            }
            ui.weak("Changes apply straight away.");
        });
    });
    if modal.should_close() {
        app.settings.open = false;
    }
}

fn general(app: &mut WriteApp, ui: &mut egui::Ui) {
    group(ui, "You", |ui| {
        ui.horizontal(|ui| {
            ui.label("Your name");
            ui.add(egui::TextEdit::singleline(&mut app.user_name).desired_width(220.0));
        });
        ui.weak("Shown on your comments, tracked changes and change-log entries.");
    });
    group(ui, "Recent documents", |ui| {
        ui.label(format!("{} in File › Open recent", app.prefs.recent.len()));
        if ui.add_enabled(!app.prefs.recent.is_empty(), egui::Button::new("Clear the list")).clicked() {
            app.prefs.recent.clear();
        }
    });
}

fn appearance(app: &mut WriteApp, ui: &mut egui::Ui) {
    group(ui, "Theme", |ui| {
        let now = app.appearance();
        ui.horizontal(|ui| {
            for a in crate::theme::Appearance::ALL {
                if ui.radio(now == a, a.name()).clicked() {
                    app.set_appearance(ui.ctx(), a);
                }
            }
        });
        ui.weak("Pages stay white; this changes everything around them.");
    });
    group(ui, "Accessibility", |ui| {
        let d = &mut app.prefs.display;
        ui.horizontal(|ui| {
            ui.label("Interface size");
            let mut pct = (d.ui_scale * 100.0).round();
            // Applied when the slider is let go (resizing under the pointer mid-drag jumps).
            let key = egui::Id::new("write-ui-scale-draft");
            if let Some(v) = ui.data(|m| m.get_temp::<f32>(key)) {
                pct = v;
            }
            let r = ui
                .add(egui::Slider::new(&mut pct, 75.0..=200.0).step_by(5.0).suffix("%"))
                .on_hover_text("Makes menus, panels and dialogs bigger or smaller (the page has its own zoom)");
            if r.dragged() {
                ui.data_mut(|m| m.insert_temp(key, pct));
            } else {
                ui.data_mut(|m| m.remove::<f32>(key));
                if r.drag_stopped() || r.changed() {
                    d.ui_scale = pct / 100.0;
                }
            }
        });
        ui.checkbox(&mut d.high_contrast, "High contrast (black and white, stronger outlines)");
        ui.checkbox(&mut d.reduce_motion, "Reduce motion (steady caret, no sliding or fading)");
        ui.weak("Screen readers (Orca, NVDA, VoiceOver) can read the document and follow the caret.");
    });
    group(ui, "Show", |ui| {
        let mut toolbar = !app.prefs.hide_toolbar;
        if ui.checkbox(&mut toolbar, "Toolbar").changed() {
            app.prefs.hide_toolbar = !toolbar;
        }
        let mut ruler = !app.prefs.hide_ruler;
        if ui.checkbox(&mut ruler, "Ruler").changed() {
            app.prefs.hide_ruler = !ruler;
        }
    });
    group(ui, "Dialogs", |ui| {
        let mut own = !app.prefs.dialogs_inside;
        if ui.checkbox(&mut own, "Open dialogs in windows of their own").changed() {
            app.prefs.dialogs_inside = !own;
        }
        ui.weak("Off: dialogs appear inside the Xenify Write window. Questions that need an answer first always do.");
    });
    group(ui, "Measurements", |ui| {
        ui.horizontal(|ui| {
            ui.label("Show lengths in");
            for u in crate::page_setup::Unit::ALL {
                ui.radio_value(&mut app.prefs.unit, u, u.name());
            }
        });
    });
}

fn typing(app: &mut WriteApp, ui: &mut egui::Ui) {
    group(ui, "Format as you type", |ui| {
        let a = &mut app.view.autoformat;
        ui.checkbox(&mut a.bullets, "Typing * then a space starts a bulleted list");
        ui.checkbox(&mut a.numbers, "Typing 1. then a space starts a numbered list");
        ui.checkbox(&mut a.checklist, "Typing [ ] then a space starts a checklist");
        ui.checkbox(&mut a.headings, "Typing # then a space makes a heading (## for level 2…)");
        ui.checkbox(&mut a.quote, "Typing > then a space makes a quote");
        ui.weak("Ctrl+Z straight after undoes the formatting and keeps what you typed.");
    });
    group(ui, "Autocorrect", |ui| autocorrect(&mut app.view.autocorrect, ui));
}

/// The autocorrect switches and its editable list.
fn autocorrect(a: &mut xenify_wp_doc::autocorrect::Autocorrect, ui: &mut egui::Ui) {
    ui.checkbox(&mut a.smart_quotes, "Curly quotes: \" and ' become “ ” and ‘ ’");
    ui.checkbox(&mut a.capitalize_sentences, "Start each sentence with a capital letter");
    ui.checkbox(&mut a.emoticons, "Emoticons become emoji: :) → 🙂, ;) → 😉, <3 → ❤️")
        .on_hover_text("Typed on their own, then a space. Search \":)\" in Symbols & emoji to see them all.");
    ui.checkbox(&mut a.replace, "Replace what I type using this list:");
    ui.add_enabled_ui(a.replace, |ui| {
        let mut remove = None;
        egui::Grid::new("autocorrect-list").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.weak("Typed");
            ui.weak("Becomes");
            ui.end_row();
            for (i, (from, to)) in a.entries.iter_mut().enumerate() {
                ui.add(egui::TextEdit::singleline(from).desired_width(110.0));
                ui.add(egui::TextEdit::singleline(to).desired_width(110.0));
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            a.entries.remove(i);
        }
        ui.horizontal(|ui| {
            if ui.button("Add").clicked() {
                a.entries.push((String::new(), String::new()));
            }
            if ui.button("Reset to the starting list").clicked() {
                a.entries = xenify_wp_doc::autocorrect::default_entries();
            }
        });
    });
    ui.weak("A correction happens when you finish the word (space or punctuation). Ctrl+Z straight after puts back what you typed.");
}

fn saving(app: &mut WriteApp, ui: &mut egui::Ui) {
    group(ui, "Change log", |ui| {
        ui.checkbox(&mut app.prefs.ask_change_note, "Ask what changed each time I save");
        ui.weak("Adds “who changed what, when” to the document (File › Record a change).");
    });
    group(ui, "If Xenify Write closes unexpectedly", |ui| {
        ui.label(format!(
            "Unsaved work is kept every {} seconds, apart from your file, and offered back next time.",
            crate::recovery::AUTOSAVE_SECS
        ));
        if let Some(dir) = crate::recovery::root() {
            ui.weak(format!("Kept in {}", dir.display()));
        }
    });
}
