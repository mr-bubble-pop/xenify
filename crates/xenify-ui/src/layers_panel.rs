//! The Layers panel: one row per layer (top of the list = drawn on top), with
//! visibility, lock, a thumbnail and the name. Drag rows to restack them (or
//! into a group), double-click a name or press F2 to rename. A layer's
//! effects are listed under it: 👁 turns one on or off, double-clicking it
//! opens its settings.

use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use xenify_anim::{Scene, SceneContent, SceneLayer};
use xenify_doc::{Id, Layer, LayerKind, LayerMeta};
use xenify_ops::Command;
use xenify_ops::order::Placement;

const ROW_H: f32 = 30.0;
const FX_ROW_H: f32 = 22.0;
const THUMB: f32 = 26.0;

/// The layer being renamed and its name so far.
#[derive(Clone)]
struct Rename(Id, String);

fn rename_id() -> egui::Id {
    egui::Id::new("layer-rename")
}

/// Starts renaming `layer` (F2 / double-click).
pub fn start_rename(ctx: &egui::Context, layer: &Layer) {
    ctx.data_mut(|d| d.insert_temp(rename_id(), Rename(layer.id, layer.meta.name.clone())));
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    // The tab already says "Layers": just the add button.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("+ Add", |ui| crate::panels::add_layer_buttons(app, ui)).response.on_hover_text("Add layer");
        });
    });
    ui.separator();
    let comp = app.comp().clone();
    let scene = xenify_anim::evaluate_project(app.project(), comp.id, app.time);
    let mut drop: Option<(Id, Placement)> = None;
    let in_group =
        |l: &Layer| l.meta.parent.and_then(|p| comp.layer(p)).is_some_and(|p| matches!(p.kind, LayerKind::Group));
    // The rows top to bottom, for Shift-click ranges.
    let mut order = Vec::new();
    for layer in comp.layers.iter().rev().filter(|l| !in_group(l)) {
        visible_rows(ui, &comp, layer, &mut order);
    }
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("layer-row-order"), order.clone()));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for layer in comp.layers.iter().rev() {
            if !in_group(layer) {
                tree(app, ui, &comp, &scene, layer, 0, &mut drop);
            }
        }
        if comp.layers.is_empty() {
            ui.weak("No layers yet. Draw with the rectangle, ellipse or pen tool, or drop an image here.");
        }
        // Below the last row: dropping here sends layers to the bottom.
        if let Some(&last) = order.last() {
            let (rect, _) = ui
                .allocate_exact_size(Vec2::new(ui.available_width(), ROW_H.max(ui.available_height())), Sense::hover());
            if let Some(dragged) = egui::DragAndDrop::payload::<Id>(ui.ctx()).map(|p| *p)
                && ui.ctx().pointer_latest_pos().is_some_and(|p| rect.contains(p))
            {
                ui.painter().hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0, ACCENT));
                if ui.input(|i| i.pointer.any_released()) {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    // Below the bottom-most top-level layer.
                    let bottom = comp.layers.iter().find(|l| !in_group(l)).map_or(last, |l| l.id);
                    if dragged != bottom {
                        drop = Some((dragged, Placement::Below(bottom)));
                    }
                }
            }
        }
    });
    crate::effects_ui::window(app, ui.ctx());
    if let Some((dragged, placement)) = drop {
        let moving = if app.is_selected(dragged) { app.selected_layers() } else { vec![dragged] };
        match xenify_ops::order::move_layers(app.project(), comp.id, &moving, placement) {
            Ok(Some(cmd)) => {
                app.exec("Move layers", cmd);
            }
            Ok(None) => {}
            Err(e) => app.fail(e.to_string()),
        }
    }
}

fn tree(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    comp: &xenify_doc::Composition,
    scene: &Scene,
    layer: &Layer,
    depth: usize,
    drop: &mut Option<(Id, Placement)>,
) {
    row(app, ui, comp, scene, layer, depth, drop);
    effect_rows(app, ui, layer, depth);
    crate::text_anim_ui::layer_rows(app, ui, layer, depth);
    if matches!(layer.kind, LayerKind::Group) && group_open(ui, layer.id) {
        for child in comp.layers.iter().rev().filter(|l| l.meta.parent == Some(layer.id)) {
            tree(app, ui, comp, scene, child, depth + 1, drop);
        }
    }
}

/// Appends the rows `layer` shows (itself, then open groups' members).
fn visible_rows(ui: &egui::Ui, comp: &xenify_doc::Composition, layer: &Layer, out: &mut Vec<Id>) {
    out.push(layer.id);
    if matches!(layer.kind, LayerKind::Group) && group_open(ui, layer.id) {
        for child in comp.layers.iter().rev().filter(|l| l.meta.parent == Some(layer.id)) {
            visible_rows(ui, comp, child, out);
        }
    }
}

fn has_fx(layer: &Layer) -> bool {
    !layer.meta.effects.is_empty() || !layer.meta.blend.is_normal()
}

fn fx_open_id(id: Id) -> egui::Id {
    egui::Id::new(("fx-open", id))
}

/// Whether a layer's effects are listed under it (folded until opened).
fn fx_open(ui: &egui::Ui, id: Id) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(fx_open_id(id))).unwrap_or(false)
}

/// The layer's blend mode and effects, one small row each under the layer.
fn effect_rows(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, depth: usize) {
    if !has_fx(layer) || !fx_open(ui, layer.id) {
        return;
    }
    // Lined up under the layer's thumbnail.
    let indent = 4.0 + depth as f32 * 16.0 + 18.0 + 48.0;
    if !layer.meta.blend.is_normal() {
        let r = effect_row(ui, layer, indent, None, &format!("Blend: {}", layer.meta.blend.label()), true);
        if r.double_clicked() || r.triple_clicked() {
            crate::effects_ui::open_window(ui.ctx(), layer.id, None);
        }
        r.context_menu(|ui| {
            if ui.button("Effects…").clicked() {
                crate::effects_ui::open_window(ui.ctx(), layer.id, None);
                ui.close();
            }
            if ui.button("Normal Blend").clicked() {
                let meta = LayerMeta { blend: xenify_doc::BlendMode::Normal, ..layer.meta.clone() };
                app.exec("Blend mode", Command::SetLayerMeta { layer: layer.id, meta });
                ui.close();
            }
        });
    }
    for (i, e) in layer.meta.effects.iter().enumerate() {
        let r = effect_row(ui, layer, indent, Some(i), e.kind.label(), e.enabled);
        let eye = Rect::from_min_size(Pos2::new(r.rect.left() + indent, r.rect.top()), Vec2::new(20.0, FX_ROW_H));
        let eye = ui
            .interact(eye, egui::Id::new(("fx-eye", layer.id, i)), Sense::click())
            .on_hover_text(if e.enabled { "Turn this effect off" } else { "Turn this effect on" });
        eye.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, e.enabled, format!("{} on", e.kind.label()))
        });
        if eye.clicked() {
            crate::effects_ui::toggle(app, layer, i);
        } else if r.double_clicked() || r.triple_clicked() {
            crate::effects_ui::open_window(ui.ctx(), layer.id, Some(i));
        } else if r.clicked() {
            app.select_only(Some(layer.id));
        }
        r.context_menu(|ui| {
            if ui.button("Settings…").clicked() {
                crate::effects_ui::open_window(ui.ctx(), layer.id, Some(i));
                ui.close();
            }
            if ui.button(if e.enabled { "Turn Off" } else { "Turn On" }).clicked() {
                crate::effects_ui::toggle(app, layer, i);
                ui.close();
            }
            if ui.button("Remove").clicked() {
                crate::effects_ui::remove(app, layer, i);
                ui.close();
            }
        });
    }
}

/// One effect's row: 👁 (for effects), "fx" and its name, dimmed when off.
fn effect_row(
    ui: &mut egui::Ui,
    layer: &Layer,
    indent: f32,
    index: Option<usize>,
    label: &str,
    on: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), FX_ROW_H), Sense::hover());
    let r = ui.interact(rect, egui::Id::new(("fx-row", layer.id, index)), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("fx {label}")));
    let painter = ui.painter_at(rect);
    if r.hovered() {
        painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let dim = crate::theme::Tokens::get(ui.ctx()).text_disabled;
    let text = if on { ui.visuals().text_color() } else { dim };
    let y = rect.center().y;
    if index.is_some() {
        painter.text(
            Pos2::new(rect.left() + indent + 10.0, y),
            egui::Align2::CENTER_CENTER,
            "👁",
            egui::FontId::proportional(12.0),
            text,
        );
    }
    painter.text(
        Pos2::new(rect.left() + indent + 24.0, y),
        egui::Align2::LEFT_CENTER,
        "fx",
        egui::FontId::proportional(11.0),
        if on { ACCENT } else { dim },
    );
    painter.text(
        Pos2::new(rect.left() + indent + 42.0, y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        text,
    );
    r.on_hover_text("Double-click to change its settings")
}

fn group_open(ui: &egui::Ui, id: Id) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new(("group-open", id)))).unwrap_or(true)
}

#[allow(clippy::too_many_arguments)]
fn row(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    comp: &xenify_doc::Composition,
    scene: &Scene,
    layer: &Layer,
    depth: usize,
    drop: &mut Option<(Id, Placement)>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    // The row itself: select, drag, right-click. Registered first so the
    // buttons drawn on top of it take their own clicks.
    let r = ui.interact(rect, egui::Id::new(("layer-row", layer.id)), Sense::click_and_drag());
    let selected = app.is_selected(layer.id);
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &layer.meta.name));
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 3.0, ACCENT.gamma_multiply(0.28));
    } else if r.hovered() {
        painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }

    let mut x = rect.left() + 4.0 + depth as f32 * 16.0;
    let cell = |x: f32, w: f32| Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, ROW_H));
    let child = |ui: &mut egui::Ui, r: Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
        // A stable id per cell, so a rename box appearing in one row doesn't
        // shift the ids of the buttons after it (egui flags that in red).
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("layer-cell", layer.id, r.min.x.round() as i32))
                .max_rect(r)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            add,
        );
    };

    // Small icon buttons with ids of their own (from the layer id), so rows
    // appearing or changing above them can't make two share an id.
    let icon = |ui: &mut egui::Ui, r: Rect, key: &str, text: &str, color: Color32, tip: &str| {
        let resp = ui.interact(r, egui::Id::new((key, layer.id)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
        let color = if resp.hovered() { ui.visuals().strong_text_color() } else { color };
        painter.text(r.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.0), color);
        resp.on_hover_text(tip).clicked()
    };

    // Expand / collapse (groups).
    if matches!(layer.kind, LayerKind::Group) {
        let open = group_open(ui, layer.id);
        let arrow = if open { "⏷" } else { "⏵" };
        if icon(
            ui,
            cell(x, 18.0),
            "layer-open",
            arrow,
            crate::theme::Tokens::get(ui.ctx()).text_dim,
            "Show / hide what's inside",
        ) {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("group-open", layer.id)), !open));
        }
    }
    x += 18.0;
    let eye =
        if layer.meta.visible { ui.visuals().text_color() } else { crate::theme::Tokens::get(ui.ctx()).text_disabled };
    if icon(ui, cell(x, 22.0), "layer-eye", "👁", eye, "Show / hide") {
        let meta = LayerMeta { visible: !layer.meta.visible, ..layer.meta.clone() };
        app.exec("Toggle visibility", Command::SetLayerMeta { layer: layer.id, meta });
    }
    // An open padlock only while hovering the row; a locked one always.
    let row_hovered = ui.rect_contains_pointer(rect);
    let (lock, color) = if layer.meta.locked {
        ("🔒", ACCENT)
    } else if row_hovered {
        ("🔓", crate::theme::Tokens::get(ui.ctx()).text_disabled)
    } else {
        ("", Color32::TRANSPARENT)
    };
    if icon(ui, cell(x + 22.0, 22.0), "layer-lock", lock, color, "Lock / unlock") {
        let meta = LayerMeta { locked: !layer.meta.locked, ..layer.meta.clone() };
        app.exec("Toggle lock", Command::SetLayerMeta { layer: layer.id, meta });
    }
    x += 48.0;
    let thumb = Rect::from_center_size(Pos2::new(x + THUMB / 2.0, rect.center().y), Vec2::splat(THUMB));
    thumbnail(app, ui, &painter, thumb, layer, scene, scene.layer(layer.id));
    x += THUMB + 8.0;

    // Name (or the rename box).
    let is_matte = comp.layers.iter().any(|l| l.meta.matte.is_some_and(|m| m.layer == layer.id));
    let name_rect = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(rect.right() - 22.0, rect.bottom()));
    let renaming = ui.ctx().data(|d| d.get_temp::<Rename>(rename_id())).filter(|r| r.0 == layer.id);
    if let Some(Rename(id, mut text)) = renaming {
        let mut done = None;
        child(ui, name_rect, &mut |ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(name_rect.width()));
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            } else if edit.lost_focus() {
                done = Some(true);
            }
        });
        match done {
            None => {
                ui.ctx().data_mut(|d| d.insert_temp(rename_id(), Rename(id, text)));
            }
            Some(commit) => {
                ui.ctx().data_mut(|d| d.remove::<Rename>(rename_id()));
                let name = text.trim();
                if commit && !name.is_empty() && name != layer.meta.name {
                    let meta = LayerMeta { name: name.to_string(), ..layer.meta.clone() };
                    app.exec("Rename layer", Command::SetLayerMeta { layer: id, meta });
                }
            }
        }
    } else {
        let mut name = RichText::new(&layer.meta.name);
        if !layer.meta.visible {
            name = name.color(crate::theme::Tokens::get(ui.ctx()).text_disabled);
        }
        if layer.meta.matte.is_some() && !is_matte {
            name = RichText::new(format!("↳ {}", layer.meta.name));
        }
        // Changed by the assistant's request under review: in the accent colour.
        let by_ai = app.assistant.reviewing && crate::assistant::changed_layers(app).iter().any(|c| c.0 == layer.id);
        let color = if by_ai {
            ACCENT
        } else if layer.meta.visible {
            ui.visuals().text_color()
        } else {
            crate::theme::Tokens::get(ui.ctx()).text_disabled
        };
        let has_fx = has_fx(layer);
        // Long names end in "…" before the badges.
        let badges = if is_matte { 38.0 } else { 0.0 }
            + if has_fx { 32.0 } else { 0.0 }
            + if layer.meta.masks.is_empty() { 0.0 } else { 16.0 }
            + if matches!(layer.kind, LayerKind::Precomp { .. }) { 22.0 } else { 0.0 };
        let mut job =
            egui::text::LayoutJob::simple_singleline(name.text().to_string(), egui::FontId::proportional(13.5), color);
        job.wrap = egui::text::TextWrapping {
            max_width: (name_rect.width() - badges).max(20.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(Pos2::new(name_rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
        if !layer.meta.masks.is_empty() {
            // Masked: a dashed-oval badge, left of the fx badge.
            let x = if is_matte { rect.right() - 44.0 } else { rect.right() - 6.0 } - if has_fx { 32.0 } else { 0.0 };
            let c = Pos2::new(x - 6.0, rect.center().y);
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(12.0, 10.0)),
                1.0,
                egui::Stroke::new(1.0, crate::viewport::mask_colour()),
                egui::StrokeKind::Middle,
            );
            painter.circle_stroke(c, 3.0, egui::Stroke::new(1.0, crate::viewport::mask_colour()));
        }
        if has_fx {
            let x = if is_matte { rect.right() - 44.0 } else { rect.right() - 6.0 };
            let open = fx_open(ui, layer.id);
            painter.text(
                Pos2::new(x, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                if open { "fx ⏷" } else { "fx ⏵" },
                egui::FontId::proportional(11.0),
                ACCENT,
            );
            // Click: show or fold the effects under the layer; double-click: edit them.
            let badge = Rect::from_min_max(Pos2::new(x - 30.0, rect.top()), Pos2::new(x + 2.0, rect.bottom()));
            let b = ui.interact(badge, egui::Id::new(("fx-badge", layer.id)), Sense::click());
            b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Show or hide effects"));
            let b = b.on_hover_text("Click to show or hide the effects · double-click to edit them");
            if b.double_clicked() || b.triple_clicked() {
                crate::effects_ui::open_window(ui.ctx(), layer.id, None);
            } else if b.clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(fx_open_id(layer.id), !open));
            }
        }
        if let LayerKind::Precomp { comp: symbol, .. } = layer.kind {
            let x = rect.right() - 6.0 - if is_matte { 38.0 } else { 0.0 } - if has_fx { 32.0 } else { 0.0 };
            let at = Rect::from_center_size(Pos2::new(x - 9.0, rect.center().y), Vec2::new(18.0, ROW_H));
            if icon(ui, at, "symbol-edit", "✏", ACCENT, "Edit this symbol (every copy updates)") {
                crate::symbols::edit(app, symbol);
            }
        }
        if is_matte {
            painter.text(
                Pos2::new(rect.right() - 6.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                "mask",
                egui::FontId::proportional(10.5),
                crate::theme::Tokens::get(ui.ctx()).text_dim,
            );
        }
    }

    // Clicks on the row.
    if r.clicked() {
        let m = ui.input(|i| i.modifiers);
        if m.command {
            // Ctrl-click: add or remove this one.
            app.toggle_selection(layer.id);
        } else if m.shift
            && let Some(anchor) = app.selected
        {
            // Shift-click: everything from the last clicked row to this one.
            let order: Vec<Id> = ui.ctx().data(|d| d.get_temp(egui::Id::new("layer-row-order"))).unwrap_or_default();
            match (order.iter().position(|i| *i == anchor), order.iter().position(|i| *i == layer.id)) {
                (Some(a), Some(b)) => {
                    let range: Vec<Id> =
                        if a <= b { order[a..=b].to_vec() } else { order[b..=a].iter().rev().copied().collect() };
                    app.select_only(Some(anchor));
                    for id in range.into_iter().filter(|id| *id != anchor) {
                        app.toggle_selection(id);
                    }
                    // The anchor stays where the range started.
                }
                _ => app.toggle_selection(layer.id),
            }
        } else {
            app.select_only(Some(layer.id));
        }
    }
    if r.double_clicked() || r.triple_clicked() {
        match layer.kind {
            LayerKind::Precomp { comp, .. }
                if thumb.expand(4.0).contains(r.interact_pointer_pos().unwrap_or_default()) =>
            {
                crate::symbols::edit(app, comp)
            }
            _ => start_rename(ui.ctx(), layer),
        }
    }
    let hover = match layer.kind {
        LayerKind::Precomp { .. } => {
            "A symbol: double-click the picture to edit it (every copy updates), the name to rename this copy"
        }
        _ if is_matte => "Used as a mask for other layers, so it isn't drawn itself",
        _ if layer.meta.matte.is_some() => "Masked by another layer",
        _ => "Drag to restack · double-click to rename",
    };
    let by_ai = app.assistant.reviewing && crate::assistant::changed_layers(app).iter().any(|c| c.0 == layer.id);
    let hover = if by_ai {
        format!("Changed by the assistant (Accept or Reject in its panel)\n{hover}")
    } else {
        hover.to_string()
    };
    let fx: Vec<&str> = layer.meta.effects.iter().filter(|e| e.enabled).map(|e| e.kind.label()).collect();
    let hover = if fx.is_empty() && layer.meta.blend.is_normal() {
        hover
    } else {
        let blend = (!layer.meta.blend.is_normal()).then(|| format!("{} blend", layer.meta.blend.label()));
        format!("{hover}\nfx: {}", fx.iter().map(|s| s.to_string()).chain(blend).collect::<Vec<_>>().join(", "))
    };
    let hover = match layer.meta.masks.len() {
        0 => hover,
        1 => format!("{hover}\n1 mask"),
        n => format!("{hover}\n{n} masks"),
    };
    let r = r.on_hover_text(hover);

    // Dragging rows: drop above / below another, or into a group.
    if r.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), layer.id);
    }
    if r.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if let Some(pointer) = ui.ctx().pointer_latest_pos().filter(|p| rect.contains(*p))
        && let Some(dragged) = egui::DragAndDrop::payload::<Id>(ui.ctx()).map(|p| *p)
        && dragged != layer.id
    {
        let t = (pointer.y - rect.top()) / rect.height();
        let placement = if matches!(layer.kind, LayerKind::Group) && (0.3..0.7).contains(&t) {
            Placement::IntoGroup(layer.id)
        } else if t < 0.5 {
            Placement::Above(layer.id)
        } else {
            Placement::Below(layer.id)
        };
        let line = Stroke::new(2.0, ACCENT);
        match placement {
            Placement::IntoGroup(_) => {
                painter.rect_stroke(rect.shrink(1.0), 3.0, line, egui::StrokeKind::Inside);
            }
            Placement::Above(_) => {
                painter.hline(rect.x_range(), rect.top() + 1.0, line);
            }
            _ => {
                painter.hline(rect.x_range(), rect.bottom() - 1.0, line);
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            *drop = Some((dragged, placement));
        }
    }

    context_menu(app, &r, layer);
}

/// The same menu as right-clicking the layer on the canvas.
fn context_menu(app: &mut XenifyApp, r: &egui::Response, layer: &Layer) {
    r.context_menu(|ui| {
        if !app.is_selected(layer.id) {
            app.select_only(Some(layer.id));
        }
        ui.set_min_width(200.0);
        crate::canvas_menu::selection_items(app, ui);
    });
}

/// A small picture of the layer: its shapes (outlined in their colours), its
/// image, or a symbol for layers without their own content.
fn thumbnail(
    app: &XenifyApp,
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    layer: &Layer,
    scene: &Scene,
    sl: Option<&SceneLayer>,
) {
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let symbol = |s: &str| {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, s, egui::FontId::proportional(15.0), Color32::GRAY);
    };
    // Groups and nested canvases show what they contain.
    if matches!(layer.kind, LayerKind::Group | LayerKind::Precomp { .. }) {
        let mut items = Vec::new();
        match sl.map(|sl| &sl.content) {
            Some(SceneContent::Group(_) | SceneContent::Precomp { .. }) => collect_items(sl.unwrap(), &mut items),
            _ => {
                for id in xenify_anim::descendants(app.comp(), layer.id) {
                    if let Some(child) = scene.layer(id) {
                        collect_items(child, &mut items);
                    }
                }
            }
        }
        if !draw_items(painter, rect.shrink(3.0), &items) {
            symbol(if matches!(layer.kind, LayerKind::Group) { "▣" } else { "⊞" });
        }
        return;
    }
    let Some(sl) = sl else {
        symbol(match layer.kind {
            LayerKind::Null => "□",
            LayerKind::Camera(_) => "📷",
            LayerKind::Particles(_) => "✨",
            LayerKind::Liquid(_) => "≈",
            LayerKind::Audio(_) => "♪",
            _ => "·",
        });
        return;
    };
    match &sl.content {
        SceneContent::Shapes(items) => {
            // The layer's own drawing, unrotated, filling the tile.
            let items: Vec<_> = items.iter().map(|i| (i, kurbo::Affine::IDENTITY)).collect();
            draw_items(painter, rect.shrink(3.0), &items);
        }
        SceneContent::Text(t) => {
            let [r, g, b, _] = t.color.to_rgba8();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "T",
                egui::FontId::proportional(17.0),
                Color32::from_rgb(r, g, b),
            );
        }
        SceneContent::Image(img) => match image_texture(ui.ctx(), img) {
            Some(tex) => {
                let size = tex.size_vec2();
                let k = (rect.width() / size.x).min(rect.height() / size.y);
                let r = Rect::from_center_size(rect.center(), size * k);
                painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => symbol("🖼"),
        },
        SceneContent::Precomp { .. } => symbol("⊞"),
        SceneContent::Group(_) => symbol("▣"),
    }
}

/// Every shape drawn by `sl` (and, for groups and nested canvases, by its
/// layers), with its layer → scene transform.
fn collect_items<'a>(sl: &'a SceneLayer, out: &mut Vec<(&'a xenify_anim::DrawItem, kurbo::Affine)>) {
    match &sl.content {
        SceneContent::Shapes(items) => out.extend(items.iter().map(|i| (i, sl.transform))),
        SceneContent::Group(layers) | SceneContent::Precomp { layers, .. } => {
            for l in layers {
                collect_items(l, out);
            }
        }
        _ => {}
    }
}

/// Outlines of `items` scaled to fit `inner`; false when there's nothing to draw.
fn draw_items(painter: &egui::Painter, inner: Rect, items: &[(&xenify_anim::DrawItem, kurbo::Affine)]) -> bool {
    use kurbo::Shape as _;
    let b = items
        .iter()
        .map(|(i, t)| (*t * i.path.clone()).bounding_box())
        .reduce(|a, b| a.union(b))
        .filter(|b| b.width() > 0.0 || b.height() > 0.0);
    let Some(b) = b else { return false };
    let k = (inner.width() as f64 / b.width().max(1e-6)).min(inner.height() as f64 / b.height().max(1e-6));
    let map = |p: kurbo::Point| {
        Pos2::new(
            inner.center().x + ((p.x - b.center().x) * k) as f32,
            inner.center().y + ((p.y - b.center().y) * k) as f32,
        )
    };
    for (item, t) in items {
        let colour = item.fill.or(item.stroke.as_ref().map(|s| s.color)).unwrap_or(xenify_doc::Color::WHITE);
        let colour = item.gradient.as_ref().and_then(|g| g.sorted_stops().first().map(|s| s.color)).unwrap_or(colour);
        let [r, g, bl, _] = colour.to_rgba8();
        let stroke = Stroke::new(1.4, Color32::from_rgb(r, g, bl));
        let mut pts = Vec::new();
        kurbo::flatten((*t * item.path.clone()).iter(), 0.5 / k.max(1e-6), |el| match el {
            kurbo::PathEl::MoveTo(p) => {
                if pts.len() > 1 {
                    painter.add(egui::Shape::line(std::mem::take(&mut pts), stroke));
                }
                pts.clear();
                pts.push(map(p));
            }
            kurbo::PathEl::LineTo(p) => pts.push(map(p)),
            kurbo::PathEl::ClosePath => {
                if let Some(first) = pts.first().copied() {
                    pts.push(first);
                }
            }
            _ => {}
        });
        if pts.len() > 1 {
            painter.add(egui::Shape::line(pts, stroke));
        }
    }
    true
}

/// A small texture of an embedded image, made once per image.
fn image_texture(ctx: &egui::Context, img: &xenify_anim::SceneImage) -> Option<egui::TextureHandle> {
    let key = egui::Id::new(("layer-thumb", img.hash));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    let small = image::load_from_memory(&img.data).ok()?.thumbnail(64, 64).to_rgba8();
    let (w, h) = small.dimensions();
    let tex = ctx.load_texture(
        format!("layer-thumb-{:x}", img.hash),
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], small.as_raw()),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}
