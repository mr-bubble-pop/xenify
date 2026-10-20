//! View › Navigation (Ctrl+Shift+N): the document's headings as a tree on
//! the left. Click one to go there; the heading the caret is under is
//! highlighted; levels fold; the box at the top filters by text.

use crate::app::WriteApp;
use crate::icons::{self, Icon};
use xenify_wp_doc::{NodeId, Pos, Selection};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Nav {
    pub show: bool,
    pub filter: String,
    /// Headings whose sub-headings are folded away.
    collapsed: HashSet<NodeId>,
    /// Paragraph order in the document, for the document version `order_rev`.
    order: HashMap<NodeId, usize>,
    order_rev: Option<u64>,
}

impl Nav {
    pub fn new(show: bool) -> Self {
        Self { show, ..Default::default() }
    }

    /// Each paragraph's place in the document (worked out again after edits).
    fn order(&mut self, doc: &xenify_wp_doc::Document) -> &HashMap<NodeId, usize> {
        if self.order_rev != Some(doc.revision) {
            self.order = doc.paragraphs().enumerate().map(|(i, p)| (p.id, i)).collect();
            self.order_rev = Some(doc.revision);
        }
        &self.order
    }
}

/// The headings: (paragraph, level, text).
fn headings(app: &WriteApp) -> Vec<(NodeId, u8, String)> {
    xenify_wp_layout::toc_entries(&app.editor.doc, 9)
}

/// The heading the caret is in or under.
fn current(app: &mut WriteApp, heads: &[(NodeId, u8, String)]) -> Option<NodeId> {
    let caret = app.editor.sel.focus.para;
    let order = app.nav.order(&app.editor.doc);
    let at = *order.get(&caret)?;
    heads.iter().rev().find(|(id, _, _)| order.get(id).is_some_and(|&i| i <= at)).map(|(id, _, _)| *id)
}

/// Moves the caret to the start of `heading` and scrolls it to the top.
pub fn go_to(app: &mut WriteApp, heading: NodeId) {
    app.editor.sel = Selection::caret(Pos::new(heading, 0));
    app.view.reveal_at_top();
}

pub fn panel(app: &mut WriteApp, ui: &mut egui::Ui) {
    if !app.nav.show {
        return;
    }
    egui::Panel::left("navigation").resizable(true).default_size(240.0).size_range(180.0..=420.0).show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(icons::image(ui, Icon::Contents, icons::MENU));
            ui.strong("Navigation");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Close, "Close", "Close navigation (Ctrl+Shift+N)").clicked() {
                    app.nav.show = false;
                }
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut app.nav.filter).hint_text("Find a heading").desired_width(f32::INFINITY),
        );
        ui.separator();
        let heads = headings(app);
        if heads.is_empty() {
            ui.weak("No headings yet. Use a Heading style, or type # and a space at the start of a line.");
            return;
        }
        let here = current(app, &heads);
        let filter = app.nav.filter.trim().to_lowercase();
        let mut go = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("nav-tree").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // Below a folded heading, everything deeper is hidden.
            let mut hide_below: Option<u8> = None;
            for (i, (id, level, text)) in heads.iter().enumerate() {
                if filter.is_empty() {
                    if let Some(l) = hide_below {
                        if *level > l {
                            continue;
                        }
                        hide_below = None;
                    }
                } else if !text.to_lowercase().contains(&filter) {
                    continue;
                }
                let has_children = heads.get(i + 1).is_some_and(|(_, next, _)| next > level);
                let folded = app.nav.collapsed.contains(id);
                ui.horizontal(|ui| {
                    let indent = if filter.is_empty() { (*level as f32 - 1.0) * 14.0 } else { 0.0 };
                    ui.add_space(indent);
                    if has_children && filter.is_empty() {
                        let icon = if folded { Icon::Folded } else { Icon::More };
                        let tip = if folded {
                            "Show the headings under this one"
                        } else {
                            "Fold the headings under this one"
                        };
                        let img = icons::image(ui, icon, 12.0);
                        if ui.add(egui::Button::image(img).frame(false)).on_hover_text(tip).clicked() {
                            if folded {
                                app.nav.collapsed.remove(id);
                            } else {
                                app.nav.collapsed.insert(*id);
                            }
                        }
                    } else {
                        ui.add_space(14.0);
                    }
                    let label = egui::RichText::new(text.trim());
                    let label = if *level == 1 { label.strong() } else { label };
                    let r = ui.add(egui::Button::selectable(here == Some(*id), label).truncate());
                    if r.clicked() {
                        go = Some(*id);
                    }
                });
                if folded && filter.is_empty() {
                    hide_below = Some(*level);
                }
            }
        });
        if let Some(id) = go {
            go_to(app, id);
        }
    });
}
