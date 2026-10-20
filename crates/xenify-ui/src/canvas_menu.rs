//! The canvas's right-click menu: what can be done with the selection
//! (group, mask, combine, align, arrange, effects…), and new objects created
//! where the click was.

use crate::app::{XenifyApp, Tool};
use crate::keymap::Action;
use crate::menus::item;
use xenify_doc::{LayerKind, Vec2};
use xenify_ops::Command;
use xenify_ops::draw::Figure;

/// Shows the menu on a right-click of the canvas `response` (except in tools
/// with their own right-click: points and zoom).
pub fn show(app: &mut XenifyApp, response: &egui::Response) {
    if matches!(app.tool, Tool::Node | Tool::Zoom) {
        return;
    }
    response.context_menu(|ui| {
        ui.set_min_width(200.0);
        if app.selected_layers().is_empty() {
            create_items(app, ui);
            ui.separator();
            item(app, ui, Action::Paste);
            item(app, ui, Action::SelectAll);
            ui.separator();
            canvas_items(app, ui);
        } else {
            selection_items(app, ui);
            ui.separator();
            ui.menu_button("Create Here", |ui| create_items(app, ui));
        }
    });
}

/// Where the menu was opened, in composition space (else the canvas centre).
fn menu_point(app: &XenifyApp) -> Vec2 {
    app.viewport.menu_at().map(|p| p.to_vec2()).unwrap_or_else(|| {
        let s = &app.comp().settings;
        Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0)
    })
}

/// New shapes and text at the click, in the current colours.
fn create_items(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let at = menu_point(app);
    let s = &app.comp().settings;
    let side = (s.width.min(s.height) as f64 / 4.0).round().max(8.0);
    let (square, wide) = (Vec2::new(side, side), Vec2::new(side * 1.5, side));
    let tool = app.tool;
    let shape = |app: &mut XenifyApp, kind: Tool, size: Vec2, sides: Option<(usize, Figure)>| {
        let saved = app.tool_options.clone();
        if let Some((n, figure)) = sides {
            (app.tool_options.polygon_sides, app.tool_options.figure) = (n, figure);
        }
        crate::viewport::create_shape(app, kind, at, size);
        app.tool_options = saved;
        // Stay in the tool the menu was opened from.
        app.tool = tool;
    };
    ui.label(egui::RichText::new("Create").weak());
    for (label, kind, size, sides) in [
        ("Square", Tool::Rect, square, None),
        ("Rectangle", Tool::Rect, wide, None),
        ("Circle", Tool::Ellipse, square, None),
        ("Ellipse", Tool::Ellipse, wide, None),
        ("Triangle", Tool::Polygon, square, Some((3, Figure::Polygon))),
        ("Hexagon", Tool::Polygon, square, Some((6, Figure::Polygon))),
        ("Star", Tool::Polygon, square, Some((5, Figure::Star))),
        ("Gear", Tool::Polygon, square, Some((5, Figure::Gear))),
        ("Heart", Tool::Polygon, square, Some((5, Figure::Heart))),
        ("Arrow", Tool::Polygon, wide, Some((5, Figure::Arrow))),
        ("Speech Bubble", Tool::Polygon, wide, Some((5, Figure::Bubble))),
    ] {
        if ui.button(label).clicked() {
            shape(app, kind, size, sides);
            ui.close();
        }
    }
    if ui.button("Line").clicked() {
        crate::viewport::create_line(app, at, Vec2::new(side, 0.0));
        app.tool = tool;
        ui.close();
    }
    if ui.button("Text").on_hover_text("Point text: one line that grows as you type").clicked() {
        crate::viewport::add_text_layer(app, at);
        ui.close();
    }
    if ui.button("Text Box").on_hover_text("Text that wraps inside a box you can resize").clicked() {
        let size = kurbo::Size::new(side * 2.0, side);
        crate::viewport::add_text(app, at - Vec2::new(size.width, size.height) / 2.0, Some(size));
        ui.close();
    }
    if ui.button("Empty Group").clicked() {
        let id = xenify_doc::Id(app.project().next_id);
        let n = app.comp().layers.iter().filter(|l| matches!(l.kind, LayerKind::Group)).count() + 1;
        let layer = xenify_doc::Layer::new(id, format!("Group {n}"), LayerKind::Group).with_position(at);
        if app.exec("New group", Command::AddLayer { comp: app.comp, index: None, layer }) {
            app.select_only(Some(id));
        }
        ui.close();
    }
    ui.menu_button("Effect Here", |ui| crate::effects_gallery::here_menu(app, ui, at));
    item(app, ui, Action::Import);
}

/// The whole canvas: view toggles and settings.
fn canvas_items(app: &mut XenifyApp, ui: &mut egui::Ui) {
    item(app, ui, Action::ZoomFit);
    ui.menu_button("Show", |ui| {
        ui.checkbox(&mut app.prefs.show_rulers, "Rulers");
        ui.checkbox(&mut app.prefs.show_grid, "Grid");
        ui.checkbox(&mut app.prefs.show_guides, "Guides");
        ui.checkbox(&mut app.prefs.show_slices, "Slices");
        ui.separator();
        ui.checkbox(&mut app.prefs.snap_objects, "Snap to Objects");
        ui.checkbox(&mut app.prefs.snap_guides, "Snap to Guides");
        ui.checkbox(&mut app.prefs.snap_grid, "Snap to Grid");
    });
    let locked: Vec<xenify_doc::Id> = app.comp().layers.iter().filter(|l| l.meta.locked).map(|l| l.id).collect();
    if !locked.is_empty()
        && ui
            .button("Unlock All")
            .on_hover_text(format!(
                "{} locked layer(s) can't be clicked on the canvas; this unlocks them",
                locked.len()
            ))
            .clicked()
    {
        set_meta(app, &locked, "Unlock all", |m| m.locked = false);
        ui.close();
    }
    if ui.button("Canvas Settings…").clicked() {
        app.dialog = Some(crate::dialogs::Dialog::comp_settings(app));
        ui.close();
    }
}

/// Everything that acts on the selected layers.
pub(crate) fn selection_items(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let layers = app.selected_layers();
    let n = layers.len();
    let first = app.selected.and_then(|id| app.project().layer(id).ok()).cloned();
    let heading = match (&first, n) {
        (Some(l), 1) => l.meta.name.clone(),
        _ => format!("{n} layers"),
    };
    ui.label(egui::RichText::new(heading).weak());
    for a in [Action::Cut, Action::Copy, Action::Paste, Action::Duplicate, Action::Delete] {
        item(app, ui, a);
    }
    ui.separator();
    item(app, ui, Action::Group).on_hover_text("Put the selected layers in a group");
    if crate::actions::enabled(app, Action::Ungroup) {
        item(app, ui, Action::Ungroup);
    }
    item(app, ui, Action::Precompose).on_hover_text("Reuse the selection as a symbol with its own timeline");
    ui.menu_button("Mask", |ui| {
        item(app, ui, Action::ClipMask).on_hover_text("The top-most selected shape clips the others");
        item(app, ui, Action::ReleaseClipMask);
        ui.separator();
        crate::masks_ui::menu(app, ui);
    });
    ui.add_enabled_ui(n > 1, |ui| {
        ui.menu_button("Boolean", |ui| crate::control_bar::boolean(app, ui, &layers))
            .response
            .on_disabled_hover_text("Select 2 or more shapes (Shift+click)");
    });
    ui.menu_button("Align", |ui| crate::control_bar::align(app, ui, &layers));
    ui.menu_button("Arrange", |ui| {
        for a in [Action::ToFront, Action::Forward, Action::Backward, Action::ToBack] {
            item(app, ui, a);
        }
    });
    ui.menu_button("Transform", |ui| {
        for a in [Action::RotateCw, Action::RotateCcw, Action::Rotate180, Action::FlipH, Action::FlipV] {
            item(app, ui, a);
        }
        ui.separator();
        item(app, ui, Action::PositionSize);
    });
    if crate::path_menu::has_shapes(app) {
        crate::menus::path_menu(app, ui);
    }
    if let Some(l) = first.as_ref().filter(|_| n == 1) {
        text_items(app, ui, l);
    }
    crate::effects_ui::menu(app, ui);
    crate::modifiers_ui::menu(app, ui);
    ui.separator();
    if let Some(l) = first.as_ref().filter(|_| n == 1) {
        if let LayerKind::Image { .. } = l.kind
            && ui.button("Trace Image to Vectors…").clicked()
        {
            app.dialog = Some(crate::dialogs::Dialog::TraceLayer { layer: l.id, opts: Default::default() });
            ui.close();
        }
        if let LayerKind::Precomp { comp, .. } = l.kind
            && ui.button("Edit Symbol").clicked()
        {
            crate::symbols::edit(app, comp);
            ui.close();
        }
    }
    if ui.button("Crop Selection…").on_hover_text("Drag the edges in to crop, then Apply").clicked() {
        crate::viewport::start_crop(app, true);
        ui.close();
    }
    let all_hidden = layers.iter().all(|id| app.project().layer(*id).is_ok_and(|l| !l.meta.visible));
    let all_locked = layers.iter().all(|id| app.project().layer(*id).is_ok_and(|l| l.meta.locked));
    if ui.button(if all_hidden { "Show" } else { "Hide" }).clicked() {
        set_meta(app, &layers, if all_hidden { "Show" } else { "Hide" }, |m| m.visible = all_hidden);
        ui.close();
    }
    if ui
        .button(if all_locked { "Unlock" } else { "Lock" })
        .on_hover_text("Locked layers can't be selected or moved on the canvas")
        .clicked()
    {
        set_meta(app, &layers, if all_locked { "Unlock" } else { "Lock" }, |m| m.locked = !all_locked);
        if !all_locked {
            app.select_only(None);
        }
        ui.close();
    }
    if n == 1 {
        item(app, ui, Action::RenameLayer);
    }
}

/// Edit text, and switch between point text and a text box.
fn text_items(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &xenify_doc::Layer) {
    let LayerKind::Text(t) = &layer.kind else { return };
    ui.menu_button("Text", |ui| {
        if ui.button("Edit Text").clicked() {
            crate::viewport::start_in_place(app, layer.id);
            ui.close();
        }
        if t.path.is_none() {
            let label = if t.frame.is_some() { "Make Point Text" } else { "Make Text Box" };
            if ui.button(label).clicked() {
                let frame = match t.frame {
                    Some(_) => None,
                    None => {
                        let b = app.viewport.layer_text_bounds(app, layer.id);
                        Some(xenify_doc::TextFrame::new(
                            b.unwrap_or(kurbo::Rect::new(0.0, 0.0, 400.0, 100.0)).inflate(2.0, 2.0),
                        ))
                    }
                };
                app.exec("Text box", Command::SetTextFrame { layer: layer.id, frame });
                if frame.is_some() {
                    app.set_tool(Tool::Text);
                }
                ui.close();
            }
        }
    });
}

fn set_meta(app: &mut XenifyApp, layers: &[xenify_doc::Id], label: &str, f: impl Fn(&mut xenify_doc::LayerMeta)) {
    let cmds = layers
        .iter()
        .filter_map(|id| app.project().layer(*id).ok())
        .map(|l| {
            let mut meta = l.meta.clone();
            f(&mut meta);
            Command::SetLayerMeta { layer: l.id, meta }
        })
        .collect();
    app.exec(label, Command::batch(cmds));
}
