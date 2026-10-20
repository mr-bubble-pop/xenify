//! Dictionary and thesaurus: right-click a word for synonyms (put back in
//! the word's form: "happier" → "gladder") or to look it up; the Dictionary
//! tab shows definitions, examples, pronunciation, synonyms, opposites and
//! broader/narrower words, and puts a clicked word in place of the target.
//! Open English WordNet, all local (see `xenify_wp_check::lexicon`).

use crate::app::WriteApp;
use crate::icons::{self, Icon};
use egui::RichText;
use xenify_wp_check::lexicon::{self, Inflection, Lookup, PartOfSpeech};
use xenify_wp_doc::{NodeId, Pos, ops};
use std::ops::Range;

/// The word in the document that a clicked synonym replaces.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub para: NodeId,
    pub range: Range<usize>,
    pub text: String,
}

pub struct Dictionary {
    /// The Dictionary tab is open.
    pub show: bool,
    /// Set when "Look up" asks for the tab; the dock brings it to the front
    /// and clears this.
    pub bring_to_front: bool,
    /// Look up each word as it's selected in the text.
    pub follow: bool,
    query: String,
    looked_up: String,
    result: Lookup,
    target: Option<Target>,
    back: Vec<String>,
}

impl Dictionary {
    /// A dictionary tab that looks up selected words when `follow`.
    pub fn following(follow: bool) -> Self {
        Self { follow, ..Default::default() }
    }
}

impl Default for Dictionary {
    fn default() -> Self {
        lexicon::preload();
        Dictionary {
            show: false,
            bring_to_front: false,
            follow: false,
            query: String::new(),
            looked_up: String::new(),
            result: Lookup::default(),
            target: None,
            back: Vec::new(),
        }
    }
}

impl Dictionary {
    fn look_up(&mut self, word: &str) {
        let word = word.trim();
        if word.is_empty() || word == self.looked_up {
            return;
        }
        if !self.looked_up.is_empty() {
            self.back.push(std::mem::take(&mut self.looked_up));
        }
        self.query = word.to_string();
        self.looked_up = word.to_string();
        self.result = lexicon::lexicon().lookup(word);
    }
}

/// The word (or selected phrase) at `pos`: the selection when it's within
/// one paragraph and holds `pos`, else the word around `pos`.
pub fn target_at(app: &WriteApp, pos: Pos) -> Option<Target> {
    let doc = &app.editor.doc;
    let text = &doc.para(pos.para)?.text;
    let (a, b) = doc.ordered(app.editor.sel);
    let range =
        if a.para == b.para && a.para == pos.para && a.offset < b.offset && (a.offset..=b.offset).contains(&pos.offset)
        {
            let r = a.offset..b.offset;
            let s = text.get(r.clone())?;
            // Trim spaces and punctuation around a selected phrase.
            let lead = s.len() - s.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
            let trail = s.len() - s.trim_end_matches(|c: char| !c.is_alphanumeric()).len();
            (r.start + lead)..(r.end - trail).max(r.start + lead)
        } else {
            lexicon::word_at(text, pos.offset)?
        };
    let word = text.get(range.clone())?;
    if word.is_empty() || word.len() > 60 || word.contains('\u{FFFC}') {
        return None;
    }
    Some(Target { para: pos.para, range, text: word.to_string() })
}

/// True when `t` still says what it said.
fn still_there(app: &WriteApp, t: &Target) -> bool {
    app.editor.doc.para(t.para).and_then(|p| p.text.get(t.range.clone())).is_some_and(|s| s == t.text)
}

fn replace(app: &mut WriteApp, t: &Target, with: &str) {
    if still_there(app, t) {
        let _ = ops::replace_text(&mut app.editor, t.para, t.range.clone(), with, "Synonym");
    }
}

/// Opens the Dictionary tab on the word at `pos`.
pub fn open_at(app: &mut WriteApp, pos: Pos) {
    let target = target_at(app, pos);
    if let Some(t) = &target {
        app.dictionary.look_up(&t.text.clone());
    }
    app.dictionary.target = target;
    app.dictionary.show = true;
    app.dictionary.bring_to_front = true;
}

/// Opens the Dictionary tab on the word at the caret (Tools › Dictionary,
/// Shift+F7).
pub fn open_at_caret(app: &mut WriteApp) {
    open_at(app, app.editor.sel.focus);
}

/// The right-click menu's items for the word at `pos`: a Synonyms submenu
/// and "Look up". Adds nothing away from a word.
pub fn context_items(app: &mut WriteApp, ui: &mut egui::Ui, pos: Pos) {
    let Some(target) = target_at(app, pos) else { return };
    let short: String = target.text.chars().take(24).collect();
    match lexicon::try_lexicon() {
        None => {
            icons::item_enabled(ui, false, Icon::Replace, "Synonyms (loading the dictionary…)", "");
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        Some(lx) => {
            let groups = lx.synonyms(&target.text);
            // Nothing shown when there are none (a greyed line is just noise).
            if !groups.is_empty() {
                icons::submenu(ui, Icon::Replace, "Synonyms", |ui| {
                    ui.set_max_width(280.0);
                    for (pos, definition, words) in groups.iter().take(5) {
                        ui.label(RichText::new(format!("{} {}", abbrev(*pos), clip(definition, 48))).small().weak());
                        for w in words.iter().take(6) {
                            if ui.button(w).clicked() {
                                replace(app, &target, w);
                                ui.close();
                            }
                        }
                    }
                    ui.separator();
                    if icons::item(ui, Icon::Sources, "More in the dictionary…", "").clicked() {
                        open_at(app, pos);
                        ui.close();
                    }
                });
            }
        }
    }
    if icons::item(ui, Icon::Sources, &format!("Look up “{short}”"), "").clicked() {
        open_at(app, pos);
        ui.close();
    }
    ui.separator();
}

fn abbrev(pos: PartOfSpeech) -> &'static str {
    match pos {
        PartOfSpeech::Noun => "n.",
        PartOfSpeech::Verb => "v.",
        PartOfSpeech::Adjective => "adj.",
        PartOfSpeech::Adverb => "adv.",
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n).collect();
    format!("{}…", cut.trim_end())
}

/// What a clicked word does.
enum Click {
    LookUp(String),
    Replace(String),
}

/// The Dictionary tab (the dock draws its title).
pub fn panel(app: &mut WriteApp, ui: &mut egui::Ui) {
    let mut click = None;
    // Following the selection: a word selected in the text is looked up.
    if app.dictionary.follow && !app.editor.sel.is_collapsed() {
        let (a, b) = app.editor.doc.ordered(app.editor.sel);
        let selected = app.editor.doc.text_in(app.editor.sel);
        if a.para == b.para
            && let Some(t) = target_at(app, a).filter(|t| t.text == selected.trim())
            && app.dictionary.target.as_ref() != Some(&t)
        {
            app.dictionary.look_up(&t.text.clone());
            app.dictionary.target = Some(t);
        }
    }
    ui.horizontal(|ui| {
        let back = !app.dictionary.back.is_empty();
        if icons::button_enabled(ui, back, Icon::Undo, "Back", "Back to the previous word").clicked()
            && let Some(prev) = app.dictionary.back.pop()
        {
            app.dictionary.query = prev.clone();
            app.dictionary.looked_up = prev.clone();
            app.dictionary.result = lexicon::lexicon().lookup(&prev);
        }
        let r = ui.add(
            egui::TextEdit::singleline(&mut app.dictionary.query)
                .hint_text("Look up a word…")
                .desired_width(ui.available_width() - 34.0),
        );
        let go = icons::button(ui, Icon::Find, "Look up", "Look up the word (Enter)").clicked();
        if go || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            click = Some(Click::LookUp(app.dictionary.query.clone()));
        }
    });
    let at_caret = target_at(app, app.editor.sel.focus);
    ui.horizontal_wrapped(|ui| {
        if let Some(t) = at_caret.as_ref().filter(|t| Some(*t) != app.dictionary.target.as_ref())
            && ui.small_button(format!("Look up “{}” at the cursor", clip(&t.text, 20))).clicked()
        {
            app.dictionary.target = Some(t.clone());
            click = Some(Click::LookUp(t.text.clone()));
        }
    });
    ui.checkbox(&mut app.dictionary.follow, "Look up words as I select them");
    let target = app.dictionary.target.clone().filter(|t| still_there(app, t));
    if let Some(t) = &target {
        ui.weak(format!("Click a word to put it in place of “{}”; right-click for more.", clip(&t.text, 24)));
    }
    ui.separator();

    egui::ScrollArea::vertical().id_salt("dictionary").auto_shrink([false, false]).show(ui, |ui| {
        if lexicon::try_lexicon().is_none() {
            ui.weak("Loading the dictionary…");
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let looked_up = app.dictionary.looked_up.clone();
        if looked_up.is_empty() {
            ui.weak("Right-click a word in the document and choose Look up, or type one above.");
        } else if app.dictionary.result.is_empty() {
            ui.label(format!("“{looked_up}” isn't in the dictionary."));
        }
        let lx = lexicon::lexicon();
        for found in &app.dictionary.result.entries {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&found.word).heading().strong());
                if !found.pronunciation.is_empty() {
                    ui.label(RichText::new(format!("/{}/", found.pronunciation)).weak());
                }
                ui.label(RichText::new(found.pos.name()).italics());
            });
            if found.inflection != Inflection::Base && found.word.to_lowercase() != looked_up.to_lowercase() {
                ui.weak(format!("“{looked_up}” is a form of “{}”.", found.word));
            }
            // A word clicked replaces the target in the target's form.
            let in_form = |w: &str| match &target {
                Some(t) => lexicon::match_case(&t.text, &lx.inflect(w, found.pos, found.inflection)),
                None => w.to_string(),
            };
            for (n, m) in found.meanings.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{}.", n + 1)).strong());
                    ui.label(&m.definition);
                });
                for e in &m.examples {
                    ui.label(RichText::new(format!("“{e}”")).italics().weak());
                }
                let rows: [(&str, &Vec<String>, bool); 5] = [
                    ("Synonyms", &m.synonyms, true),
                    ("Similar", &m.similar, true),
                    ("Opposites", &m.antonyms, true),
                    ("A kind of", &m.broader, false),
                    ("Kinds", &m.narrower, false),
                ];
                for (label, words, replaces) in rows {
                    if words.is_empty() {
                        continue;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("{label}:")).small().weak());
                        for w in words {
                            if let Some(c) =
                                chip(ui, w, replaces.then(|| target.as_ref().map(|_| in_form(w))).flatten())
                            {
                                click = Some(c);
                            }
                        }
                    });
                }
                ui.add_space(4.0);
            }
            ui.separator();
        }
        ui.label(RichText::new(lexicon::ATTRIBUTION).small().weak());
    });

    match click {
        Some(Click::LookUp(w)) => app.dictionary.look_up(&w),
        Some(Click::Replace(w)) => {
            if let Some(t) = &target {
                replace(app, t, &w);
                // The replacement becomes the target, so another click swaps it again.
                app.dictionary.target =
                    Some(Target { para: t.para, range: t.range.start..t.range.start + w.len(), text: w });
            }
        }
        None => {}
    }
}

/// A word as a small button: a click replaces the target with `replace`
/// when there's one (else looks it up); right-click for the rest.
fn chip(ui: &mut egui::Ui, word: &str, replace: Option<String>) -> Option<Click> {
    let mut out = None;
    let r = ui.small_button(word);
    let r = match &replace {
        Some(w) => r.on_hover_text(format!("Put “{w}” in")),
        None => r.on_hover_text(format!("Look up “{word}”")),
    };
    if r.clicked() {
        out = Some(match &replace {
            Some(w) => Click::Replace(w.clone()),
            None => Click::LookUp(word.to_string()),
        });
    }
    r.context_menu(|ui| {
        if let Some(w) = &replace
            && ui.button(format!("Put “{w}” in")).clicked()
        {
            out = Some(Click::Replace(w.clone()));
            ui.close();
        }
        if ui.button(format!("Look up “{word}”")).clicked() {
            out = Some(Click::LookUp(word.to_string()));
            ui.close();
        }
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(word.to_string());
            ui.close();
        }
    });
    out
}
