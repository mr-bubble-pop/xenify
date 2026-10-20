//! The Writing tab: what kind of document this is (and who it's for),
//! checking it one thing at a time (Check, Previous, Next), and the AI
//! assistant's review, rewrite and write tools.

use crate::app::WriteApp;
use egui::RichText;
use xenify_wp_check::{Category, Issue};
use xenify_wp_doc::{Pos, Selection, ops};

/// The issues in document order (paragraph, then place in it).
fn in_order(app: &WriteApp) -> Vec<Issue> {
    let order: std::collections::HashMap<_, _> =
        app.editor.doc.paragraphs().enumerate().map(|(i, p)| (p.id, i)).collect();
    let mut v = app.proofing.issues.clone();
    v.sort_by_key(|i| (order.get(&i.para).copied().unwrap_or(usize::MAX), i.range.start));
    v
}

/// Goes to issue `i`: selected in the text and scrolled to.
fn show_issue(app: &mut WriteApp, i: Issue) {
    app.editor.sel = Selection { anchor: Pos::new(i.para, i.range.start), focus: Pos::new(i.para, i.range.end) };
    app.view.reveal();
    app.proofing.panel.current = Some(i);
}

/// The next (or previous) issue after the caret, wrapping round.
fn step(app: &mut WriteApp, forward: bool) {
    let list = in_order(app);
    if list.is_empty() {
        app.proofing.panel.current = None;
        return;
    }
    let order: std::collections::HashMap<_, _> =
        app.editor.doc.paragraphs().enumerate().map(|(i, p)| (p.id, i)).collect();
    let key = |p: Pos| (order.get(&p.para).copied().unwrap_or(0), p.offset);
    let (a, b) = app.editor.doc.ordered(app.editor.sel);
    let pick = if forward {
        let from = key(b);
        list.iter()
            .find(|i| (order.get(&i.para).copied().unwrap_or(0), i.range.start) >= from && !same(app, i))
            .or(list.first())
    } else {
        let from = key(a);
        list.iter().rev().find(|i| (order.get(&i.para).copied().unwrap_or(0), i.range.start) < from).or(list.last())
    };
    if let Some(i) = pick.cloned() {
        show_issue(app, i);
    }
}

/// Whether `i` is the issue shown now (Next moves past it).
fn same(app: &WriteApp, i: &Issue) -> bool {
    app.proofing.panel.current.as_ref().is_some_and(|c| c.para == i.para && c.range == i.range)
}

pub fn panel(app: &mut WriteApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().id_salt("writing").auto_shrink([false, false]).show(ui, |ui| {
        // Three sections, each folds away.
        egui::CollapsingHeader::new(RichText::new("This document").strong()).id_salt("w-doc").default_open(true).show(
            ui,
            |ui| {
                crate::proofing::document_type(app, ui);
                crate::assistant::audience(app, ui);
            },
        );
        egui::CollapsingHeader::new(RichText::new("Spelling and grammar").strong())
            .id_salt("w-check")
            .default_open(true)
            .show(ui, |ui| check(app, ui));
        egui::CollapsingHeader::new(RichText::new("Improve with AI").strong())
            .id_salt("w-ai")
            .default_open(true)
            .show(ui, |ui| crate::assistant::tools(app, ui));
    });
}

/// Check spelling and grammar: a count, Check / Previous / Next, and the one
/// thing found at a time, with what to do about it.
fn check(app: &mut WriteApp, ui: &mut egui::Ui) {
    if !app.proofing.settings.enabled {
        ui.label("Checking is turned off.");
        if ui.button("Turn it on").clicked() {
            app.proofing.settings.enabled = true;
            app.proofing.recheck();
        }
        return;
    }
    let issues = &app.proofing.issues;
    if issues.is_empty() {
        ui.label("Nothing to look at. ✓");
    } else {
        // How many of each kind, in the kind's colour.
        ui.horizontal_wrapped(|ui| {
            for c in Category::ALL {
                let n = issues.iter().filter(|i| i.category == c).count();
                if n > 0 {
                    ui.label(RichText::new(format!("{} {n}", c.name())).color(crate::proofing::color(c)));
                }
            }
        });
    }
    let any = !app.proofing.issues.is_empty();
    ui.horizontal(|ui| {
        if ui.add_enabled(any, egui::Button::new("◀ Previous")).on_hover_text("The one before the caret").clicked() {
            step(app, false);
        }
        if ui
            .add_enabled(any, egui::Button::new(RichText::new("Check").strong()))
            .on_hover_text("From the start")
            .clicked()
            && let Some(i) = in_order(app).first().cloned()
        {
            show_issue(app, i);
        }
        if ui.add_enabled(any, egui::Button::new("Next ▶")).on_hover_text("The next one after the caret").clicked() {
            step(app, true);
        }
    });
    // The one being looked at (gone when fixed or ignored).
    let current = app
        .proofing
        .panel
        .current
        .clone()
        .filter(|c| app.proofing.issues.iter().any(|i| i.para == c.para && i.range == c.range));
    if let Some(issue) = current {
        card(app, ui, &issue);
    }
    if ui.small_button("Settings…").clicked() {
        app.open_settings(crate::settings::Section::Proofing);
    }
}

/// What was found, its suggestions, and Ignore / Add to dictionary / Why.
fn card(app: &mut WriteApp, ui: &mut egui::Ui, issue: &Issue) {
    let text = app.editor.doc.para(issue.para).map(|p| p.text.clone()).unwrap_or_default();
    let word = text.get(issue.range.clone()).unwrap_or("").to_string();
    let mut done = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let kind = match issue.feature {
            Some(f) => format!("{} · {}", issue.category.name(), f.name()),
            None => issue.category.name().to_string(),
        };
        ui.label(RichText::new(kind).small().color(crate::proofing::color(issue.category)));
        ui.label(RichText::new(&word).strong());
        ui.label(&issue.message);
        if !issue.suggestions.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for s in issue.suggestions.iter().take(6) {
                    let label = if s.is_empty() { "Remove it".to_string() } else { s.clone() };
                    if ui.button(RichText::new(label).strong()).clicked() {
                        let _ = ops::replace_text(&mut app.editor, issue.para, issue.range.clone(), s, "Correction");
                        done = true;
                    }
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Ignore").clicked() {
                app.editor.doc.meta.ignore_issues.push(issue.ignore_key(&text));
                app.editor.doc.revision += 1;
                done = true;
            }
            if issue.category == Category::Spelling && ui.button("Add to dictionary").clicked() {
                app.proofing.add_word(&word);
                done = true;
            }
            if ui.button("Why?").clicked() {
                app.proofing.why = Some(issue.clone());
            }
        });
    });
    // Fixed or ignored: on to the next one.
    if done {
        app.proofing.panel.current = None;
        step(app, true);
    }
}
