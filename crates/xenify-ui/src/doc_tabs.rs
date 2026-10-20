//! The strip of open files under the menu bar, like a browser's tabs:
//! click to switch, × or middle-click to close, drag to reorder, right-click
//! for more.

use crate::app::XenifyApp;
use crate::documents::{DocId, others, to_the_right};
use crate::session::{Pending, guard};
use crate::widgets::ACCENT;
use egui::{RichText, Sense};

pub fn strip(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let tabs = app.tab_info();
    let ids: Vec<DocId> = tabs.iter().map(|t| t.id).collect();
    let drag_id = egui::Id::new("doc-tab-drag");
    let mut rects = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for t in &tabs {
            let fill = if t.active { ui.visuals().widgets.active.weak_bg_fill } else { ui.visuals().extreme_bg_color };
            let frame = egui::Frame::new().fill(fill).corner_radius(4).inner_margin(egui::Margin::symmetric(8, 3));
            let inner = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut name = RichText::new(&t.name);
                    if t.active {
                        name = name.color(ui.visuals().strong_text_color());
                    }
                    ui.add(egui::Label::new(name).selectable(false).sense(Sense::click()));
                    if t.dirty {
                        ui.label(RichText::new("•").color(ACCENT)).on_hover_text("Unsaved changes");
                    }
                    let close = ui.add(
                        egui::Button::new(RichText::new("×").color(crate::theme::Tokens::get(ui.ctx()).text_dim))
                            .frame(false)
                            .small(),
                    );
                    close.clicked()
                })
                .inner
            });
            let closed = inner.inner;
            let r = ui.interact(inner.response.rect, egui::Id::new(("doc-tab", t.id.0)), Sense::click_and_drag());
            let tab_name =
                if t.dirty { format!("{} tab (unsaved changes)", t.name) } else { format!("{} tab", t.name) };
            r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, t.active, &tab_name));
            rects.push((t.id, inner.response.rect));
            let path = t.path.as_ref().map_or("Not saved yet".to_string(), |p| p.display().to_string());
            let r = r.on_hover_text(path);
            if closed || r.middle_clicked() {
                guard(app, Pending::Close(vec![t.id]));
            } else if r.clicked() {
                app.activate(t.id);
            }
            if r.drag_started() {
                ui.data_mut(|d| d.insert_temp(drag_id, t.id));
            }
            r.context_menu(|ui| {
                let unsaved = "Save it first: it isn't a file yet";
                let reveal = ui
                    .add_enabled(t.path.is_some(), egui::Button::new("Reveal in File Manager"))
                    .on_disabled_hover_text(unsaved);
                if reveal.clicked()
                    && let Some(p) = &t.path
                {
                    // Where the file is on this computer (differs inside a dev container).
                    let host = xenify_platform::desktop::local_to_host(p, crate::platform::shared_folders())
                        .unwrap_or_else(|| p.clone());
                    if let Err(e) = app.platform.shell.reveal(p) {
                        ui.ctx().copy_text(host.display().to_string());
                        app.status = format!("{e}. The file is at {} (copied to the clipboard).", host.display());
                    }
                    ui.close();
                }
                let copy =
                    ui.add_enabled(t.path.is_some(), egui::Button::new("Copy Path")).on_disabled_hover_text(unsaved);
                if copy.clicked()
                    && let Some(p) = &t.path
                {
                    let host = xenify_platform::desktop::local_to_host(p, crate::platform::shared_folders())
                        .unwrap_or_else(|| p.clone());
                    ui.ctx().copy_text(host.display().to_string());
                    ui.close();
                }
                ui.separator();
                if ui.button("Close").clicked() {
                    guard(app, Pending::Close(vec![t.id]));
                    ui.close();
                }
                let rest = others(&ids, t.id);
                let only = "This is the only open file";
                if ui
                    .add_enabled(!rest.is_empty(), egui::Button::new("Close Others"))
                    .on_disabled_hover_text(only)
                    .clicked()
                {
                    guard(app, Pending::Close(rest));
                    ui.close();
                }
                let right = to_the_right(&ids, t.id);
                let right_btn = egui::Button::new("Close to the Right");
                if ui.add_enabled(!right.is_empty(), right_btn).on_disabled_hover_text("No tabs to the right").clicked()
                {
                    guard(app, Pending::Close(right));
                    ui.close();
                }
                let saved: Vec<DocId> = tabs.iter().filter(|x| !x.dirty).map(|x| x.id).collect();
                if ui.add_enabled(!saved.is_empty(), egui::Button::new("Close Saved")).clicked() {
                    guard(app, Pending::Close(saved));
                    ui.close();
                }
            });
        }
        let tip = app.keymap.tip(crate::keymap::Action::New, "New document");
        if ui.add(egui::Button::new("+").frame(false)).on_hover_text(tip).clicked() {
            app.dialog = Some(crate::dialogs::Dialog::new_document(app));
        }
    });
    // Dropping a dragged tab: it goes where the pointer is.
    if let Some(dragged) = ui.data(|d| d.get_temp::<DocId>(drag_id))
        && !ui.input(|i| i.pointer.primary_down())
    {
        ui.data_mut(|d| d.remove::<DocId>(drag_id));
        if let Some(x) = ui.input(|i| i.pointer.latest_pos()).map(|p| p.x) {
            let to = rects.iter().filter(|(id, r)| *id != dragged && r.center().x < x).count();
            app.move_tab(dragged, to);
        }
    }
}
