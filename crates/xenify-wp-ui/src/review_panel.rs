//! The Review pane: tracked changes (accept or reject each), and comment
//! threads as cards next to the text they're about.

use crate::app::WriteApp;
use crate::icons::{self, Icon};
use xenify_wp_doc::review::{self, ChangeKind};
use xenify_wp_doc::{NodeId, Selection};
use std::collections::HashMap;

#[derive(Default)]
pub struct ReviewState {
    /// The pane is open.
    pub show: bool,
    show_resolved: bool,
    /// A comment being written: the text it's on, and the words so far.
    pub new_comment: Option<(Selection, String)>,
    /// Replies being written, by thread.
    replies: HashMap<NodeId, String>,
    /// The thread whose text is being edited.
    editing: Option<(NodeId, String)>,
    /// Give the document the keyboard back next frame.
    refocus: bool,
    /// How tracked changes are shown.
    pub markup: review::Markup,
    /// The document as shown with no markup / original: (revision, view, it).
    pub shown: Option<(u64, review::Markup, xenify_wp_doc::Editor)>,
}

impl ReviewState {
    /// The read-only copy to show instead of the document (not for All markup).
    pub fn shown_doc(&mut self, doc: &xenify_wp_doc::Document) -> Option<&mut xenify_wp_doc::Editor> {
        if self.markup == review::Markup::All {
            self.shown = None;
            return None;
        }
        let fresh = matches!(&self.shown, Some((rev, m, _)) if *rev == doc.revision && *m == self.markup);
        if !fresh {
            self.shown =
                Some((doc.revision, self.markup, xenify_wp_doc::Editor::new(review::view_as(doc, self.markup))));
        }
        self.shown.as_mut().map(|(_, _, e)| e)
    }
}

/// The All markup / No markup / Original choice.
pub fn markup_choice(ui: &mut egui::Ui, markup: &mut review::Markup) {
    for m in review::Markup::ALL {
        let tip = match m {
            review::Markup::All => "Show insertions and deletions (you can edit)",
            review::Markup::Final => "Show the document as if every change were accepted (read only)",
            review::Markup::Original => "Show the document as it was before the changes (read only)",
        };
        ui.radio_value(markup, m, m.name()).on_hover_text(tip);
    }
}

impl ReviewState {
    /// Review › New comment: starts a comment on the selection (or the word at the caret).
    pub fn start_comment(&mut self, ed: &xenify_wp_doc::Editor) {
        let sel = if ed.sel.is_collapsed() { xenify_wp_doc::ops::word_at(ed, ed.sel.focus) } else { ed.sel };
        self.show = true;
        self.new_comment = Some((sel, String::new()));
    }
}

/// "Today 14:03" (local time) from an ISO date (or the date as it is).
fn short_date(iso: &str) -> String {
    if let Some(ms) = crate::recovery::parse_iso(iso) {
        return crate::recovery::when(ms);
    }
    short_date_raw(iso)
}

fn short_date_raw(iso: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (d, t) = iso.split_once('T').unwrap_or((iso, ""));
    let parts: Vec<&str> = d.split('-').collect();
    match (parts.as_slice(), t.get(..5)) {
        ([_, m, day], time) => {
            let month = m.parse::<usize>().ok().and_then(|m| MONTHS.get(m.wrapping_sub(1))).copied().unwrap_or(m);
            let day = day.trim_start_matches('0');
            match time {
                Some(t) => format!("{t}, {day} {month}"),
                None => format!("{day} {month}"),
            }
        }
        _ => iso.to_string(),
    }
}

fn author_chip(ui: &mut egui::Ui, name: &str, date: &str) {
    ui.horizontal(|ui| {
        let c = xenify_wp_layout::author_color(name).0;
        ui.label(egui::RichText::new(name).strong().color(egui::Color32::from_rgb(c[0], c[1], c[2])));
        if !date.is_empty() {
            ui.weak(short_date(date));
        }
    });
}

pub fn panel(app: &mut WriteApp, ui: &mut egui::Ui) {
    if std::mem::take(&mut app.review.refocus) {
        ui.memory_mut(|m| m.request_focus(app.view.id));
    }
    let mut tracking = app.editor.doc.meta.track_changes;
    if ui
        .checkbox(&mut tracking, "Track changes")
        .on_hover_text("Ctrl+Shift+E: edits are marked (with your name) until someone accepts or rejects them")
        .changed()
    {
        let _ = review::set_tracking(&mut app.editor, tracking);
    }
    ui.horizontal(|ui| {
        ui.label("Show");
        markup_choice(ui, &mut app.review.markup);
    });
    ui.separator();
    changes_list(app, ui);
    ui.separator();
    comments(app, ui);
}

fn changes_list(app: &mut WriteApp, ui: &mut egui::Ui) {
    let all = review::changes(&app.editor.doc);
    egui::CollapsingHeader::new(format!("Changes ({})", all.len())).id_salt("review-changes").default_open(true).show(
        ui,
        |ui| {
            if all.is_empty() {
                ui.weak("No tracked changes.");
                return;
            }
            ui.horizontal(|ui| {
                if ui.button("Accept all").clicked() {
                    let _ = review::decide(&mut app.editor, true, true);
                }
                if ui.button("Reject all").clicked() {
                    let _ = review::decide(&mut app.editor, false, true);
                }
            });
            egui::ScrollArea::vertical().id_salt("review-changes-list").max_height(220.0).show(ui, |ui| {
                for c in all {
                    ui.horizontal(|ui| {
                        let (verb, colour) = match c.change.kind {
                            ChangeKind::Inserted => ("Added", egui::Color32::from_rgb(0x2e, 0x7d, 0x32)),
                            ChangeKind::Deleted => ("Deleted", egui::Color32::from_rgb(0xb4, 0x23, 0x18)),
                        };
                        let snippet: String = c.text.chars().take(28).collect();
                        let more = if c.text.chars().count() > 28 { "…" } else { "" };
                        let r = ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(if c.is_break {
                                        match c.change.kind {
                                            ChangeKind::Inserted => "New paragraph".to_string(),
                                            ChangeKind::Deleted => "Joined paragraphs".to_string(),
                                        }
                                    } else {
                                        format!("{verb} “{snippet}{more}”")
                                    })
                                    .color(colour),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text(format!("{} · {}", c.change.author, short_date(&c.change.date)));
                        let sel = Selection {
                            anchor: xenify_wp_doc::Pos::new(c.para, c.range.start),
                            focus: xenify_wp_doc::Pos::new(c.para, c.range.end),
                        };
                        if r.clicked() {
                            app.editor.sel = sel;
                            app.view.reveal();
                        }
                        if icons::button(ui, Icon::Check, "Accept", "Accept this change").clicked() {
                            app.editor.sel = sel;
                            let _ = review::decide(&mut app.editor, true, false);
                        }
                        if icons::button(ui, Icon::Close, "Reject", "Reject this change").clicked() {
                            app.editor.sel = sel;
                            let _ = review::decide(&mut app.editor, false, false);
                        }
                    });
                }
            });
        },
    );
}

fn comments(app: &mut WriteApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Comments").strong());
        if icons::button(ui, Icon::CommentAdd, "Add comment", "New comment on the selection (Ctrl+Alt+M)").clicked() {
            app.review.start_comment(&app.editor);
        }
        ui.checkbox(&mut app.review.show_resolved, "Show resolved");
    });
    // A comment being written.
    if let Some((sel, text)) = &mut app.review.new_comment {
        let quoted = {
            let d = &app.editor.doc;
            let (a, b) = d.ordered(*sel);
            d.para(a.para).map_or(String::new(), |p| {
                let end = if b.para == a.para { b.offset } else { p.text.len() };
                p.text[a.offset.min(end)..end].chars().take(40).collect()
            })
        };
        let mut post = false;
        let mut cancel = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.weak(format!("On “{quoted}”"));
            let r = ui.add(
                egui::TextEdit::multiline(text)
                    .hint_text("Write a comment")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            r.request_focus();
            ui.horizontal(|ui| {
                post = ui.add_enabled(!text.trim().is_empty(), egui::Button::new("Comment")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if post {
            let (sel, text) = app.review.new_comment.take().expect("open");
            app.editor.sel = sel;
            let _ = review::add_comment(&mut app.editor, &text);
            app.review.refocus = true;
        } else if cancel {
            app.review.new_comment = None;
            app.review.refocus = true;
        }
    }
    let anchors = review::anchors(&app.editor.doc);
    let threads: Vec<_> = anchors
        .iter()
        .filter_map(|(id, a, b)| app.editor.doc.meta.comments.iter().find(|c| c.id == *id).map(|c| (c.clone(), *a, *b)))
        .filter(|(c, _, _)| app.review.show_resolved || !c.resolved)
        .collect();
    if threads.is_empty() && app.review.new_comment.is_none() {
        ui.weak("No comments. Select some text and choose Review › New comment (Ctrl+Alt+M).");
        return;
    }
    let caret = app.editor.sel.focus;
    let scale = app.view.scale();
    let origin = app.view.doc_origin;
    let mut actions: Vec<(NodeId, &'static str, String)> = Vec::new();
    for (c, a, b) in threads {
        // Line the card up with its text (below the one before, never on top of it).
        if let Some(r) = app.view.layout.caret(a, false) {
            let y = origin.y + r.y0 as f32 * scale;
            let gap = y - ui.cursor().top();
            if gap > 0.0 {
                ui.add_space(gap);
            }
        }
        let here = {
            let d = &app.editor.doc;
            let order: Vec<NodeId> = d.paragraphs().map(|p| p.id).collect();
            let k = |p: xenify_wp_doc::Pos| (order.iter().position(|x| *x == p.para), p.offset);
            k(a) <= k(caret) && k(caret) <= k(b)
        };
        let frame = egui::Frame::group(ui.style()).fill(if here {
            ui.visuals().selection.bg_fill.gamma_multiply(0.35)
        } else {
            ui.visuals().faint_bg_color
        });
        let r = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                author_chip(ui, &c.author, &c.date);
                match &mut app.review.editing {
                    Some((id, text)) if *id == c.id => {
                        ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
                                actions.push((c.id, "edit", text.clone()));
                            }
                            if ui.button("Cancel").clicked() {
                                actions.push((c.id, "stop-edit", String::new()));
                            }
                        });
                    }
                    _ => {
                        ui.label(&c.text);
                    }
                }
                for r in &c.replies {
                    ui.indent(("reply", c.id.0, &r.date), |ui| {
                        author_chip(ui, &r.author, &r.date);
                        ui.label(&r.text);
                    });
                }
                if c.resolved {
                    ui.weak("Resolved");
                }
                // The thread at the caret opens for a reply.
                if here {
                    let draft = app.review.replies.entry(c.id).or_default();
                    ui.add(egui::TextEdit::singleline(draft).hint_text("Reply").desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!draft.trim().is_empty(), egui::Button::new("Reply")).clicked() {
                            actions.push((c.id, "reply", draft.clone()));
                        }
                        let resolve = if c.resolved { "Reopen" } else { "Resolve" };
                        if ui.button(resolve).clicked() {
                            actions.push((c.id, "resolve", String::new()));
                        }
                        ui.menu_button("…", |ui| {
                            if icons::item(ui, Icon::Edit, "Edit", "").clicked() {
                                actions.push((c.id, "start-edit", c.text.clone()));
                            }
                            if icons::item(ui, Icon::Delete, "Delete thread", "").clicked() {
                                actions.push((c.id, "delete", String::new()));
                            }
                        });
                    });
                }
            })
            .response;
        if r.interact(egui::Sense::click()).clicked() {
            app.editor.sel = Selection { anchor: a, focus: b };
            app.view.reveal();
        }
        ui.add_space(4.0);
    }
    for (id, what, text) in actions {
        let ed = &mut app.editor;
        let _ = match what {
            "reply" => {
                app.review.replies.remove(&id);
                review::reply(ed, id, &text)
            }
            "resolve" => {
                let resolved = ed.doc.meta.comments.iter().find(|c| c.id == id).is_some_and(|c| c.resolved);
                review::set_resolved(ed, id, !resolved)
            }
            "delete" => review::delete_comment(ed, id),
            "start-edit" => {
                app.review.editing = Some((id, text));
                Ok(())
            }
            "stop-edit" => {
                app.review.editing = None;
                Ok(())
            }
            "edit" => {
                app.review.editing = None;
                review::edit_comment(ed, id, &text)
            }
            _ => Ok(()),
        };
    }
}
