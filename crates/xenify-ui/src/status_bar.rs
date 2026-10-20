//! The status bar along the bottom: the latest message for a few seconds,
//! otherwise what the current tool does; the selection on the right.

use crate::app::{XenifyApp, Tool};
use egui::{RichText, accesskit};

/// How long a message stays before the tool hint comes back.
const SHOW_SECS: f64 = 8.0;

/// Makes a label a live region: screen readers say it when it changes
/// (selection, saving, exporting…) without the user going there.
fn announce(ui: &egui::Ui, id: egui::Id, live: accesskit::Live) {
    ui.ctx().accesskit_node_builder(id, |n| n.set_live(live));
}

pub fn bar(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let now = ui.input(|i| i.time);
    let key = egui::Id::new("status-seen");
    let last_key = egui::Id::new("status-last");
    // When did the current message first appear?
    let (seen, since) = ui.ctx().data(|d| d.get_temp::<(String, f64)>(key)).unwrap_or_default();
    let since = if seen == app.status {
        since
    } else {
        ui.ctx().data_mut(|d| d.insert_temp(key, (app.status.clone(), now)));
        now
    };
    let fresh = !app.status.is_empty() && now - since < SHOW_SECS;
    if fresh {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(SHOW_SECS - (now - since) + 0.1));
    } else if !app.status.is_empty() {
        // Shown long enough: kept for the hint's tooltip, and cleared so the
        // same message said again shows again.
        let old = std::mem::take(&mut app.status);
        ui.ctx().data_mut(|d| d.insert_temp(last_key, old));
    }
    let last: String = ui.ctx().data(|d| d.get_temp(last_key)).unwrap_or_default();
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        if !app.pending_models.is_empty() {
            ui.spinner();
            let names: Vec<&str> = app.pending_models.iter().map(|p| p.name.as_str()).collect();
            ui.label(RichText::new(format!("Importing {}…", names.join(", "))).color(t.text))
                .on_hover_text("3D models are read and simplified in the background");
            ui.separator();
        }
        if let Some(error) = app.error.clone() {
            // Errors stay until dismissed.
            let red = t.error;
            ui.label(RichText::new("⚠").color(red));
            let r = ui.label(RichText::new(&error).color(red)).on_hover_text(&error);
            announce(ui, r.id, accesskit::Live::Assertive);
            if ui.small_button("Dismiss").clicked() {
                app.error = None;
            }
            ui.separator();
        }
        if fresh && app.error.as_deref() != Some(app.status.as_str()) {
            ui.label(RichText::new("ℹ").color(t.accent));
            let r = ui.label(RichText::new(&app.status).color(t.text_strong));
            announce(ui, r.id, accesskit::Live::Polite);
            // Just exported: open the folder it went to.
            if app.status.starts_with("Exported")
                && let Some(path) = app.last_export.clone()
                && ui.small_button("Show in folder").clicked()
                && let Err(e) = app.platform.shell.reveal(&path)
            {
                app.fail(format!("{e}. The file is at {}", path.display()));
            }
        } else if !fresh {
            ui.label(RichText::new(tool_hint(app.tool)).color(t.text_dim)).on_hover_text(if last.is_empty() {
                ""
            } else {
                last.as_str()
            });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n = app.selected_layers().len();
            let sel = match n {
                0 => "Nothing selected".to_string(),
                1 => app
                    .selected
                    .and_then(|id| app.project().layer(id).ok())
                    .map_or("1 layer".into(), |l| format!("Selected: {}", l.meta.name)),
                n => format!("{n} layers selected"),
            };
            ui.label(RichText::new(sel).color(t.text_dim));
        });
    });
}

/// The current tool's name and what it does (the tooltip's first part).
fn tool_hint(tool: Tool) -> String {
    let tip = Tool::ALL.iter().find(|t| t.0 == tool).map_or("", |t| t.2);
    // "Rectangle (R) — drag to draw, …" → "Rectangle: drag to draw, …"
    let tip = tip.split_once(" — ").map_or(tip.to_string(), |(name, rest)| {
        let name = name.split(" (").next().unwrap_or(name);
        format!("{name}: {rest}")
    });
    tip.replace("  ", " ")
}
