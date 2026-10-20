//! Symbols: reusable pieces of animation (like SVG's `<symbol>` / `<use>`).
//! Each symbol has its own layers and timeline; every placed copy shows it,
//! so editing the symbol updates them all. Internally a symbol is a nested
//! composition that isn't a slide (or the document itself).

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{Id, LayerKind, Project};
use xenify_ops::Command;

/// Whether composition `id` is a symbol (not the document or a slide).
pub fn is_symbol(project: &Project, id: Id) -> bool {
    !xenify_ops::slides::slides_or_main(project).iter().any(|s| s.comp == id)
}

/// The project's symbols: (id, name, how many copies are placed).
pub fn list(project: &Project) -> Vec<(Id, String, usize)> {
    project
        .compositions
        .iter()
        .filter(|c| is_symbol(project, c.id))
        .map(|c| {
            let uses = project
                .compositions
                .iter()
                .flat_map(|o| &o.layers)
                .filter(|l| matches!(l.kind, LayerKind::Precomp { comp, .. } if comp == c.id))
                .count();
            (c.id, c.settings.name.clone(), uses)
        })
        .collect()
}

/// A name like "Symbol 3" not used yet.
pub fn next_name(project: &Project) -> String {
    (1..)
        .map(|n| format!("Symbol {n}"))
        .find(|name| !project.compositions.iter().any(|c| &c.settings.name == name))
        .unwrap()
}

/// Opens symbol `id` for editing, remembering where to come back to.
pub fn edit(app: &mut XenifyApp, id: Id) {
    if !is_symbol(app.project(), app.comp) {
        app.symbol_return = Some(app.comp);
    }
    app.open_comp(id);
    if xenify_ops::diagram::is_diagram(app.project(), id) {
        crate::diagram_ui::entered(app);
    }
}

/// Adds an empty diagram symbol and opens it with the diagram tools.
pub fn new_diagram(app: &mut XenifyApp) {
    let name = (1..)
        .map(|n| format!("Diagram {n}"))
        .find(|name| !app.project().compositions.iter().any(|c| &c.settings.name == name))
        .unwrap();
    let (cmd, id) = xenify_ops::diagram::new_diagram(app.project(), &name, 1600, 1000);
    if app.exec("New diagram", cmd) {
        edit(app, id);
        app.status = format!("Editing “{name}” — place it on other canvases from the Symbols panel");
    }
}

/// Back from editing a symbol to the slide or document it was opened from.
pub fn finish(app: &mut XenifyApp) {
    let back = app
        .symbol_return
        .take()
        .filter(|id| app.project().comp(*id).is_ok())
        .or_else(|| xenify_ops::slides::slides_or_main(app.project()).first().map(|s| s.comp));
    if let Some(id) = back {
        app.open_comp(id);
    }
}

/// The strip over the canvas while a symbol is being edited.
pub fn editing_banner(app: &mut XenifyApp, ui: &mut egui::Ui) {
    if !is_symbol(app.project(), app.comp) {
        return;
    }
    let name = app.comp().settings.name.clone();
    egui::Frame::new().fill(crate::widgets::ACCENT.gamma_multiply(0.25)).inner_margin(egui::vec2(8.0, 4.0)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                let what = if app.comp().settings.kind.is_animation() { "symbol" } else { "diagram" };
                ui.label(RichText::new(format!("Editing {what} “{name}”")).strong());
                ui.weak("— changes update every copy");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Done")
                        .on_hover_text("Back to where you were (Esc when nothing is selected)")
                        .clicked()
                    {
                        finish(app);
                    }
                });
            });
        },
    );
}

/// The Symbols panel.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let has_selection = !app.selected_layers().is_empty();
        if ui
            .add_enabled(
                has_selection,
                egui::Button::new("Convert to Symbol")
                    .shortcut_text(app.keymap.text(crate::keymap::Action::Precompose)),
            )
            .on_hover_text("Turn the selected layers into a symbol, leaving one copy in their place")
            .on_disabled_hover_text("Select the layers to turn into a symbol")
            .clicked()
        {
            app.precompose_selection();
        }
        if ui.button("+ New Symbol").on_hover_text("An empty symbol, opened for editing").clicked() {
            app.new_comp();
        }
        if ui
            .button("+ New Diagram")
            .on_hover_text("A flowchart, network or workflow diagram, opened with the diagram tools")
            .clicked()
        {
            new_diagram(app);
        }
    });
    ui.separator();
    let symbols = list(app.project());
    if symbols.is_empty() {
        ui.weak(
            "No symbols yet. A symbol is a reusable piece of animation — a logo, an icon, a character. \
             Place it as many times as you like; edit it once and every copy updates.",
        );
        return;
    }
    // Symbols that can be placed here without putting one inside itself.
    let blocked = xenify_ops::precomp::containing(app.project(), app.comp);
    let mut rename: Option<(Id, String)> = None;
    let mut delete = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("symbols").num_columns(3).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
            for (id, name, uses) in &symbols {
                let mut text = name.clone();
                let r = ui.add_sized([130.0, 20.0], egui::TextEdit::singleline(&mut text));
                if r.lost_focus() && text.trim() != name && !text.trim().is_empty() {
                    rename = Some((*id, text.trim().to_string()));
                }
                ui.weak(match uses {
                    0 => "not placed".to_string(),
                    1 => "1 copy".to_string(),
                    n => format!("{n} copies"),
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!blocked.contains(id), egui::Button::new("Place"))
                        .on_hover_text("Add a copy to this canvas")
                        .on_disabled_hover_text("A symbol can't be placed inside itself")
                        .clicked()
                    {
                        app.add_comp_layer(*id);
                    }
                    if ui.add_enabled(*id != app.comp, egui::Button::new("Edit")).clicked() {
                        edit(app, *id);
                    }
                    if ui
                        .add_enabled(*uses == 0, egui::Button::new("Delete").small())
                        .on_hover_text("Delete this symbol")
                        .on_disabled_hover_text("Delete its copies first")
                        .clicked()
                    {
                        delete = Some(*id);
                    }
                });
                ui.end_row();
            }
        });
    });
    if let Some((id, name)) = rename
        && let Ok(c) = app.project().comp(id)
    {
        let settings = xenify_doc::CompSettings { name, ..c.settings.clone() };
        app.exec("Rename symbol", Command::SetCompSettings { comp: id, settings });
    }
    if let Some(id) = delete {
        if app.comp == id {
            finish(app);
        }
        app.exec("Delete symbol", Command::RemoveComposition { comp: id });
    }
}

/// Properties of a placed symbol: when it starts, whether it loops, and a
/// way in to edit it.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &xenify_doc::Layer) {
    let LayerKind::Precomp { comp, start, looped } = layer.kind else { return };
    let (name, length) =
        app.project().comp(comp).map_or((String::from("?"), 0.0), |c| (c.settings.name.clone(), c.settings.duration));
    egui::CollapsingHeader::new(RichText::new("Symbol").strong()).default_open(true).show(ui, |ui| {
        ui.weak(format!("Shows “{name}” ({length:.2} s)."));
        egui::Grid::new(("symbol", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            ui.label("Starts At").on_hover_text("When the symbol's own timeline begins, on this canvas's timeline");
            let mut s = start;
            let r = ui.add(egui::DragValue::new(&mut s).speed(0.01).suffix(" s"));
            if r.drag_started() {
                app.new_gesture();
            }
            if r.changed() && s != start {
                app.exec("Symbol start", Command::SetPrecompStart { layer: layer.id, start: s });
            }
            ui.end_row();
            ui.label("");
            let mut on = looped;
            if ui
                .checkbox(&mut on, "Loop Symbol")
                .on_hover_text("Play the symbol over and over (else it shows once and then disappears)")
                .changed()
            {
                app.exec(
                    if on { "Loop symbol" } else { "Stop looping symbol" },
                    Command::SetPrecompLoop { layer: layer.id, looped: on },
                );
            }
            ui.end_row();
        });
        if ui.button("Edit Symbol").on_hover_text("Open its layers and timeline (or double-click it)").clicked() {
            edit(app, comp);
        }
    });
}
