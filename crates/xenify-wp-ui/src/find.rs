//! Edit › Find (Ctrl+F), Replace (Ctrl+H) and Filter lines: a bar along
//! the bottom of the window, as in a web browser. Every match is
//! highlighted; Enter goes to the next, Shift+Enter to the one before; Esc
//! closes the bar. Searches can be regular expressions, and replacements
//! can use their groups ($1).

use crate::app::WriteApp;
use crate::icons::{self, Icon};
use xenify_wp_doc::{NodeId, Pos, Selection, ops};
use std::ops::Range;

#[derive(Default)]
pub struct FindState {
    pub open: bool,
    /// Show the replace row too.
    pub replace: bool,
    pub query: String,
    pub with: String,
    pub case: bool,
    pub whole: bool,
    /// The search is a regular expression.
    pub regex: bool,
    /// Show the filter row (remove or keep the paragraphs that match).
    pub filter: bool,
    /// Why the regular expression isn't valid.
    pub error: Option<String>,
    /// The match shown (index into the matches).
    pub current: usize,
    /// Put the keyboard in the find box next frame.
    pub focus: bool,
    /// Matches, and the document version and search they were found for.
    found: Vec<(NodeId, Range<usize>)>,
    key: Option<(u64, String, bool, bool, bool)>,
}

impl FindState {
    /// Forgets the matches (another document came to the front).
    pub fn forget(&mut self) {
        self.found.clear();
        self.key = None;
        self.current = 0;
    }

    /// Opens the bar (with the replace row when `replace`), searching for the selection when it's short.
    pub fn show(&mut self, replace: bool, selected: Option<String>) {
        self.open = true;
        self.replace |= replace;
        self.filter = false;
        self.focus = true;
        if let Some(t) = selected.filter(|t| !t.is_empty() && t.len() < 80 && !t.contains('\n')) {
            self.query = t;
        }
    }

    /// The matches for the current search (found again when anything changed).
    pub fn matches(&mut self, doc: &xenify_wp_doc::Document) -> &[(NodeId, Range<usize>)] {
        let key = (doc.revision, self.query.clone(), self.case, self.whole, self.regex);
        if self.key.as_ref() != Some(&key) {
            self.error = None;
            self.found = if !self.open {
                Vec::new()
            } else if self.regex {
                ops::find_regex(doc, &self.query, self.case, self.whole).unwrap_or_else(|e| {
                    self.error = Some(e);
                    Vec::new()
                })
            } else {
                ops::find_all(doc, &self.query, self.case, self.whole)
            };
            self.key = Some(key);
            self.current = self.current.min(self.found.len().saturating_sub(1));
        }
        &self.found
    }
}

/// Selects match `i` and scrolls to it.
fn go(app: &mut WriteApp, i: usize) {
    let found = app.find.matches(&app.editor.doc).to_vec();
    let Some((para, r)) = found.get(i).cloned() else { return };
    app.find.current = i;
    app.editor.sel = Selection { anchor: Pos::new(para, r.start), focus: Pos::new(para, r.end) };
    app.view.reveal();
}

/// The first match at or after the caret (going forward) or before it.
fn from_caret(app: &mut WriteApp, forward: bool) -> Option<usize> {
    let order: Vec<NodeId> = app.editor.doc.paragraphs().map(|p| p.id).collect();
    let key = |p: NodeId, o: usize| (order.iter().position(|x| *x == p).unwrap_or(0), o);
    let (a, b) = app.editor.doc.ordered(app.editor.sel);
    let found = app.find.matches(&app.editor.doc).to_vec();
    if found.is_empty() {
        return None;
    }
    if forward {
        let here = key(b.para, b.offset);
        Some(found.iter().position(|(p, r)| key(*p, r.start) >= here).unwrap_or(0))
    } else {
        let here = key(a.para, a.offset);
        Some(found.iter().rposition(|(p, r)| key(*p, r.end) <= here).unwrap_or(found.len() - 1))
    }
}

pub fn next(app: &mut WriteApp, forward: bool) {
    let n = app.find.matches(&app.editor.doc).len();
    if n == 0 {
        return;
    }
    // From the match shown when it's selected; otherwise from the caret.
    let cur = app.find.current;
    let selected = app.find.found.get(cur).is_some_and(|(p, r)| {
        let (a, b) = app.editor.doc.ordered(app.editor.sel);
        a == Pos::new(*p, r.start) && b == Pos::new(*p, r.end)
    });
    let i = if selected {
        if forward { (cur + 1) % n } else { (cur + n - 1) % n }
    } else {
        from_caret(app, forward).unwrap_or(0)
    };
    go(app, i);
}

/// Replaces the match shown (when it's selected), then goes to the next.
fn replace_one(app: &mut WriteApp) {
    let found = app.find.matches(&app.editor.doc).to_vec();
    let cur = app.find.current;
    if let Some((para, r)) = found.get(cur).cloned() {
        let (a, b) = app.editor.doc.ordered(app.editor.sel);
        if a == Pos::new(para, r.start) && b == Pos::new(para, r.end) {
            let one = [(para, r)];
            let with = replacements(app, &one);
            let _ = ops::replace_each(&mut app.editor, &one, &with);
            app.editor.sel = Selection::caret(Pos::new(para, a.offset + with[0].len()));
        }
    }
    next(app, true);
}

/// What each match becomes (the groups filled in, for a regular expression).
fn replacements(app: &WriteApp, found: &[(NodeId, Range<usize>)]) -> Vec<String> {
    let f = &app.find;
    if f.regex {
        ops::regex_replacements(&app.editor.doc, &f.query, f.case, f.whole, &f.with, found)
            .unwrap_or_else(|_| found.iter().map(|_| f.with.clone()).collect())
    } else {
        found.iter().map(|_| f.with.clone()).collect()
    }
}

/// An on/off button for a search option (lit while on).
fn option(ui: &mut egui::Ui, on: &mut bool, label: &str, tip: &str) {
    let text = egui::RichText::new(label).monospace();
    if ui.add(egui::Button::selectable(*on, text)).on_hover_text(tip).clicked() {
        *on = !*on;
    }
}

pub fn bar(app: &mut WriteApp, ui: &mut egui::Ui) {
    let count = app.find.matches(&app.editor.doc).len();
    let mut close = false;
    ui.add_space(4.0);
    let field = 260.0;
    ui.horizontal(|ui| {
        ui.add(icons::image(ui, Icon::Find, icons::MENU));
        let r =
            ui.add(egui::TextEdit::singleline(&mut app.find.query).hint_text("Find in document").desired_width(field));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Find"));
        if std::mem::take(&mut app.find.focus) {
            r.request_focus();
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter {
            let back = ui.input(|i| i.modifiers.shift);
            next(app, !back);
            r.request_focus();
        }
        if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if icons::button_enabled(ui, count > 0, Icon::SortUp, "Previous match", "Previous (Shift+Enter)").clicked() {
            next(app, false);
        }
        if icons::button_enabled(ui, count > 0, Icon::SortDown, "Next match", "Next (Enter)").clicked() {
            next(app, true);
        }
        option(ui, &mut app.find.case, "Aa", "Match case");
        option(ui, &mut app.find.whole, "ab", "Whole words only");
        option(ui, &mut app.find.regex, ".*", "Regular expression (e.g. \\d+ for numbers, ^ for the start of a line)");
        if let Some(e) = &app.find.error {
            ui.colored_label(ui.visuals().error_fg_color, format!("Pattern: {e}"));
        } else {
            let label = match (count, app.find.query.is_empty()) {
                (_, true) => String::new(),
                (0, false) => "No matches".into(),
                (n, false) => {
                    let shown = app.find.found.get(app.find.current).is_some_and(|(p, r)| {
                        let (a, b) = app.editor.doc.ordered(app.editor.sel);
                        a == Pos::new(*p, r.start) && b == Pos::new(*p, r.end)
                    });
                    if shown { format!("{} of {n}", app.find.current + 1) } else { format!("{n} found") }
                }
            };
            ui.weak(label);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, Icon::Close, "Close find", "Close (Esc)").clicked() {
                close = true;
            }
            ui.toggle_value(&mut app.find.filter, "Filter lines")
                .on_hover_text("Remove the paragraphs that contain a match, or keep only those");
            ui.toggle_value(&mut app.find.replace, "Replace");
        });
    });
    if app.find.replace {
        ui.horizontal(|ui| {
            ui.add(icons::image(ui, Icon::Replace, icons::MENU));
            let r =
                ui.add(egui::TextEdit::singleline(&mut app.find.with).hint_text("Replace with").desired_width(field));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Replace with"));
            if ui.add_enabled(count > 0, egui::Button::new("Replace")).clicked() {
                replace_one(app);
            }
            if ui.add_enabled(count > 0, egui::Button::new("Replace all")).clicked() {
                let found = app.find.matches(&app.editor.doc).to_vec();
                let with = replacements(app, &found);
                if let Ok(n) = ops::replace_each(&mut app.editor, &found, &with) {
                    app.message = Some((format!("Replaced {n} {}", if n == 1 { "match" } else { "matches" }), false));
                }
            }
            if app.find.regex {
                ui.weak("$1, $2… put in what the groups in ( ) matched");
            }
        });
    }
    if app.find.filter {
        ui.horizontal(|ui| {
            ui.add(icons::image(ui, Icon::Rows, icons::MENU));
            ui.label("Paragraphs with a match:");
            let found = app.find.matches(&app.editor.doc).to_vec();
            let has = app.find.error.is_none() && !app.find.query.is_empty();
            for (keep, label, tip) in [
                (false, "Remove them", "Delete every paragraph that contains a match"),
                (true, "Keep only them", "Delete every paragraph that doesn't contain a match"),
            ] {
                if ui.add_enabled(has, egui::Button::new(label)).on_hover_text(tip).clicked() {
                    match ops::filter_paragraphs(&mut app.editor, &found, keep) {
                        Ok(n) => {
                            let what = if n == 1 { "paragraph" } else { "paragraphs" };
                            app.message = Some((format!("Removed {n} {what} (Ctrl+Z brings them back)"), false));
                        }
                        Err(e) => app.message = Some((format!("Couldn't filter ({e})"), true)),
                    }
                    app.view.reveal();
                }
            }
            ui.weak("Tables, headers and notes are left as they are.");
        });
    }
    ui.add_space(2.0);
    if close {
        app.find.open = false;
        app.view.find_marks.clear();
        app.focus_document(ui.ctx());
    }
}
