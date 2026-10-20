//! View › Split: two panes, side by side or one above the other, each
//! showing an open document (or both the same one, at different places).
//! The pane you click into is the one the menus and checking work on.

use crate::app::WriteApp;
use crate::view::DocView;
use egui::{Rect, UiBuilder};

pub struct Split {
    /// The tab shown in the pane that isn't being worked in.
    pub other: u64,
    /// The pane being worked in is the left (or top) one.
    pub active_first: bool,
    /// One above the other instead of side by side.
    pub stacked: bool,
    /// The second view when both panes show the same document.
    mirror: DocView,
}

impl WriteApp {
    /// View › Split: a second pane showing this document too.
    pub fn open_split(&mut self, stacked: bool) {
        let mut mirror = DocView::new(self.view.engine.fonts.clone());
        mirror.zoom = self.view.zoom;
        self.split = Some(Split { other: self.tabs.active, active_first: true, stacked, mirror });
    }

    pub fn close_split(&mut self) {
        self.split = None;
    }
}

/// Splits `area` in two: (first, second).
fn halves(area: Rect, stacked: bool) -> (Rect, Rect) {
    let gap = 3.0;
    if stacked {
        let mid = area.center().y;
        (
            Rect::from_min_max(area.min, egui::pos2(area.max.x, mid - gap)),
            Rect::from_min_max(egui::pos2(area.min.x, mid + gap), area.max),
        )
    } else {
        let mid = area.center().x;
        (
            Rect::from_min_max(area.min, egui::pos2(mid - gap, area.max.y)),
            Rect::from_min_max(egui::pos2(mid + gap, area.min.y), area.max),
        )
    }
}

/// The page area: one pane, or two. `main` draws the pane being worked in.
pub fn show(
    app: &mut WriteApp,
    ui: &mut egui::Ui,
    frame: &eframe::Frame,
    main: impl FnOnce(&mut WriteApp, &mut egui::Ui),
) {
    let Some(split) = app.split.as_ref() else {
        main(app, ui);
        return;
    };
    // The other document may have been closed: then it shows this one.
    let other_key = if app.tabs.order.contains(&split.other) { split.other } else { app.tabs.active };
    let area = ui.available_rect_before_wrap();
    let (first, second) = halves(area, split.stacked);
    let (mine, theirs) = if split.active_first { (first, second) } else { (second, first) };
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    let mid = if split.stacked {
        [egui::pos2(area.min.x, area.center().y), egui::pos2(area.max.x, area.center().y)]
    } else {
        [egui::pos2(area.center().x, area.min.y), egui::pos2(area.center().x, area.max.y)]
    };
    ui.painter().line_segment(mid, line);
    ui.scope_builder(UiBuilder::new().max_rect(mine), |ui| {
        ui.set_clip_rect(mine);
        main(app, ui);
    });
    // The other pane: a header to pick its document or close the split, then the page.
    let mut pick = None;
    let mut close = false;
    let mut focus_other = false;
    ui.scope_builder(UiBuilder::new().max_rect(theirs), |ui| {
        ui.set_clip_rect(theirs);
        let header = Rect::from_min_size(theirs.min, egui::vec2(theirs.width(), ui.spacing().interact_size.y + 6.0));
        ui.painter().rect_filled(header, 0.0, ui.visuals().panel_fill);
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let names: Vec<(u64, String)> = app
                .tabs
                .order
                .iter()
                .filter_map(|&k| app.tabs.doc(app, k).map(|(_, p)| (k, crate::tabs::name(p))))
                .collect();
            let shown = names.iter().find(|(k, _)| *k == other_key).map_or(String::new(), |(_, n)| n.clone());
            egui::ComboBox::from_id_salt("split-doc").selected_text(shown).show_ui(ui, |ui| {
                for (k, n) in &names {
                    if ui.selectable_label(*k == other_key, n).clicked() {
                        pick = Some(*k);
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text("Close the split (one pane)").clicked() {
                    close = true;
                }
            });
        });
        let rest = ui.available_rect_before_wrap();
        ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| {
            let s = app.split.as_mut().expect("checked");
            if other_key == app.tabs.active {
                // The same document: a second view of it.
                // Read-only until clicked into (then it's the pane worked in).
                s.mirror.read_only = true;
                s.mirror.ui(ui, &mut app.editor, Some(frame), &app.proofing.issues, &[]);
                focus_other = s.mirror.response.as_ref().is_some_and(|r| r.clicked() || r.drag_started());
            } else if let Some(t) = app.tabs.others.iter_mut().find(|t| t.key == other_key) {
                t.view.read_only = true;
                t.view.ui(ui, &mut t.editor, Some(frame), &[], &[]);
                focus_other = t.view.response.as_ref().is_some_and(|r| r.clicked() || r.drag_started());
            }
        });
    });
    if close {
        app.close_split();
        return;
    }
    if let Some(k) = pick {
        if let Some(s) = app.split.as_mut() {
            s.other = k;
        }
        return;
    }
    // Clicking into the other pane makes it the one worked in.
    if focus_other {
        let was = app.tabs.active;
        if other_key != was {
            app.switch_to(other_key);
        }
        if let Some(s) = app.split.as_mut() {
            s.other = was;
            s.active_first = !s.active_first;
            if other_key == was {
                std::mem::swap(&mut app.view, &mut s.mirror);
            }
        }
        app.refocus = true;
    }
}
