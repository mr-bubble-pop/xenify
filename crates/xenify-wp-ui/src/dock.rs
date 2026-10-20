//! The side dock: one area beside the page holding the panels as tabs —
//! Writing (checking and the AI assistant), Styles, Comments, Dictionary
//! and Symbols. The tabs are icons (named on hover and above the panel), so
//! they always fit. One-off tasks (mail merge, editing a style) are dialogs.

use crate::app::WriteApp;
use crate::icons::{self, Icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// This document's kind, checking it, and the AI assistant.
    Writing,
    Styles,
    Comments,
    Dictionary,
    Symbols,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Writing, Tab::Styles, Tab::Comments, Tab::Dictionary, Tab::Symbols];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Writing => "Writing",
            Tab::Styles => "Styles",
            Tab::Comments => "Comments",
            Tab::Dictionary => "Dictionary",
            Tab::Symbols => "Symbols & emoji",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::Writing => Icon::Spelling,
            Tab::Styles => Icon::Styles,
            Tab::Comments => Icon::Comments,
            Tab::Dictionary => Icon::Sources,
            Tab::Symbols => Icon::Emoji,
        }
    }

    fn about(self) -> &'static str {
        match self {
            Tab::Writing => "What kind of document this is, checking it, and the AI assistant",
            Tab::Styles => "Paragraph styles: click one to use it",
            Tab::Comments => "Comments and tracked changes",
            Tab::Dictionary => "Meanings and other words",
            Tab::Symbols => "Symbols and emoji to put in the text (Ctrl+.)",
        }
    }
}

/// Which tab is in front, and what was open last frame (a tab just opened
/// comes to the front).
#[derive(Default)]
pub struct Dock {
    pub active: Option<Tab>,
    was_open: Vec<Tab>,
}

/// Whether `tab` is open.
pub fn is_open(app: &WriteApp, tab: Tab) -> bool {
    match tab {
        Tab::Writing => app.show_proofing || app.show_ai,
        Tab::Styles => app.show_styles,
        Tab::Comments => app.review.show,
        Tab::Dictionary => app.dictionary.show,
        Tab::Symbols => app.symbols.open,
    }
}

pub fn set_open(app: &mut WriteApp, tab: Tab, open: bool) {
    match tab {
        Tab::Writing => {
            app.show_proofing = open;
            app.show_ai = open;
        }
        Tab::Styles => app.show_styles = open,
        Tab::Comments => app.review.show = open,
        Tab::Dictionary => app.dictionary.show = open,
        Tab::Symbols => app.symbols.open = open,
    }
}

/// Opens `tab` in front.
pub fn open(app: &mut WriteApp, tab: Tab) {
    set_open(app, tab, true);
    app.dock.active = Some(tab);
}

pub fn show(app: &mut WriteApp, ui: &mut egui::Ui) {
    if std::mem::take(&mut app.dictionary.bring_to_front) {
        app.dictionary.show = true;
        app.dock.active = Some(Tab::Dictionary);
        app.dock.was_open.retain(|t| *t != Tab::Dictionary);
        app.dock.was_open.push(Tab::Dictionary);
    }
    let open: Vec<Tab> = Tab::ALL.into_iter().filter(|t| is_open(app, *t)).collect();
    // A tab opened since last frame comes to the front.
    if let Some(new) = open.iter().find(|t| !app.dock.was_open.contains(t)) {
        app.dock.active = Some(*new);
    }
    app.dock.was_open = open.clone();
    if open.is_empty() {
        return;
    }
    let active = match app.dock.active.filter(|a| open.contains(a)) {
        Some(a) => a,
        None => open[0],
    };
    app.dock.active = Some(active);
    let panel = if app.prefs.dock_left { egui::Panel::left("dock") } else { egui::Panel::right("dock") };
    panel.resizable(true).default_size(320.0).size_range(260.0..=520.0).show(ui, |ui| {
        // Scroll bars beside the content, always shown (not floating over it).
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Icons only, so every open tab fits; the name shows on hover.
            for t in &open {
                let img = icons::image(ui, t.icon(), icons::MENU);
                let r = ui
                    .add(egui::Button::image(img).selected(*t == active).frame_when_inactive(*t == active))
                    .on_hover_text(format!("{} — {}", t.name(), t.about()));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t.name()));
                if r.clicked() {
                    app.dock.active = Some(*t);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Close, "Close tab", &format!("Close {}", active.name())).clicked() {
                    set_open(app, active, false);
                }
                let side = if app.prefs.dock_left { "Move to the right" } else { "Move to the left" };
                if icons::button(ui, Icon::Panel, "Move panel", side).clicked() {
                    app.prefs.dock_left = !app.prefs.dock_left;
                }
            });
        });
        ui.separator();
        ui.label(egui::RichText::new(active.name()).strong().size(15.0));
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            match active {
                Tab::Writing => crate::writing::panel(app, ui),
                Tab::Styles => crate::styles_panel::show(app, ui),
                Tab::Comments => crate::review_panel::panel(app, ui),
                Tab::Dictionary => crate::dictionary::panel(app, ui),
                Tab::Symbols => crate::symbols::panel(app, ui),
            }
        });
    });
}
