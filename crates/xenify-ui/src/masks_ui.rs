//! Masks in Properties and Layer ▸ Mask: add rectangle / ellipse masks
//! (a crop), turn a shape into a mask, and each mask's mode, invert,
//! feather and opacity (the last two keyframeable, like the path).

use crate::app::{XenifyApp, Tool};
use egui::RichText;
use xenify_doc::{Id, Layer, LayerKind, Mask, MaskField, MaskMode, PropPath};
use xenify_ops::Command;
use kurbo::Rect;

/// Whether masks can apply to a layer (groups take them as a whole).
pub fn takes_masks(l: &Layer) -> bool {
    !matches!(l.kind, LayerKind::Null)
}

/// The layer's own box in its own coordinates (for a mask covering it).
fn local_box(app: &XenifyApp, id: Id) -> Option<Rect> {
    let comp = app.comp();
    let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
    let mut m = app.viewport.measurer();
    let world = xenify_anim::world_transform(comp, id, app.time);
    let l = scene.layer(id);
    match l.and_then(|l| l.bounds(&mut m)) {
        Some(b) if l.is_some_and(|l| !matches!(l.content, xenify_anim::SceneContent::Group(_))) => Some(b),
        _ => {
            // Groups: their members' box, brought into the group's space.
            let b = xenify_anim::world_box(comp, &scene, id, &mut m)?;
            Some(world.inverse().transform_rect_bbox(b))
        }
    }
}

/// Adds a mask covering the whole of each selected layer, ready to drag in
/// (a crop), and switches to the Mask tool.
pub fn add_mask(app: &mut XenifyApp, ellipse: bool) {
    let mut cmds = Vec::new();
    let mut last = None;
    for id in app.selected_layers() {
        let Ok(layer) = app.project().layer(id).cloned() else { continue };
        if !takes_masks(&layer) {
            continue;
        }
        let Some(b) = local_box(app, id) else { continue };
        let n = layer.meta.masks.len() + 1;
        let m = if ellipse { Mask::ellipse(format!("Mask {n}"), b) } else { Mask::rect(format!("Mask {n}"), b) };
        let mut meta = layer.meta.clone();
        meta.masks.push(m);
        last = Some(n - 1);
        cmds.push(Command::SetLayerMeta { layer: id, meta });
    }
    if cmds.is_empty() {
        app.status = "Select the layers to mask first.".into();
        return;
    }
    if app.exec(if ellipse { "Add ellipse mask" } else { "Add rectangle mask" }, Command::batch(cmds)) {
        app.set_tool(Tool::Mask);
        app.viewport.set_active_mask(last);
        app.status = "Added a mask around the layer: drag its edges or points in to crop.".into();
    }
}

/// Layer ▸ Mask ▸ Use Shape as Mask: with two layers selected, the upper
/// one (a shape) becomes a mask on the other and is removed.
pub fn shape_to_mask(app: &mut XenifyApp) {
    let sel = app.selected_layers();
    let comp = app.comp();
    let [a, b] = sel[..] else {
        app.status = "Select two layers: the shape to use (on top) and the layer to mask.".into();
        return;
    };
    let (ia, ib) = (comp.layer_index(a).unwrap_or(0), comp.layer_index(b).unwrap_or(0));
    let (shape_id, target) = if ia > ib { (a, b) } else { (b, a) };
    let Ok(shape) = app.project().layer(shape_id).cloned() else { return };
    if shape.shapes().is_empty() {
        app.status = "The upper layer has to be a shape to use as a mask.".into();
        return;
    }
    let t = app.time;
    let to_comp = xenify_anim::world_transform(comp, shape_id, t);
    let to_local = xenify_anim::world_transform(comp, target, t).inverse();
    let mut path = kurbo::BezPath::new();
    for s in shape.shapes() {
        path.extend(xenify_anim::geometry_path(&s.geometry, t).iter());
    }
    let path = (to_local * to_comp) * path;
    let Ok(layer) = app.project().layer(target).cloned() else { return };
    let mut meta = layer.meta.clone();
    meta.masks.push(Mask::new(shape.meta.name.clone(), path));
    let cmd =
        Command::batch(vec![Command::SetLayerMeta { layer: target, meta }, Command::RemoveLayer { layer: shape_id }]);
    if app.exec("Use shape as mask", cmd) {
        app.select_only(Some(target));
        app.status = format!("“{}” is now a mask on “{}”.", shape.meta.name, layer.meta.name);
    }
}

/// Layer ▸ Mask.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let any = app.selected_layers().iter().any(|id| app.project().layer(*id).is_ok_and(takes_masks));
    ui.add_enabled_ui(any, |ui| {
        ui.menu_button("Mask", |ui| {
            if ui.button("Add Rectangle Mask").on_hover_text("Crop: drag the mask's edges in").clicked() {
                add_mask(app, false);
                ui.close();
            }
            if ui.button("Add Ellipse Mask").clicked() {
                add_mask(app, true);
                ui.close();
            }
            if ui
                .add_enabled(app.selected_layers().len() == 2, egui::Button::new("Use Shape as Mask"))
                .on_hover_text("Select a shape on top of a layer: the shape becomes that layer's mask")
                .clicked()
            {
                shape_to_mask(app);
                ui.close();
            }
            if ui.button("Draw Masks (Mask Tool)").clicked() {
                app.set_tool(Tool::Mask);
                ui.close();
            }
            ui.separator();
            let has =
                app.selected_layers().iter().any(|id| app.project().layer(*id).is_ok_and(|l| !l.meta.masks.is_empty()));
            if ui.add_enabled(has, egui::Button::new("Remove Masks")).clicked() {
                let cmds = app
                    .selected_layers()
                    .into_iter()
                    .filter_map(|id| app.project().layer(id).ok().cloned())
                    .filter(|l| !l.meta.masks.is_empty())
                    .map(|l| {
                        let mut meta = l.meta.clone();
                        meta.masks.clear();
                        Command::SetLayerMeta { layer: l.id, meta }
                    })
                    .collect();
                app.exec("Remove masks", Command::batch(cmds));
                ui.close();
            }
        })
        .response
        .on_disabled_hover_text("Select a layer first");
    });
}

/// The Masks section of Properties.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if !takes_masks(layer) {
        return;
    }
    let n = layer.meta.masks.len();
    let title = if n == 0 { "Masks".to_string() } else { format!("Masks ({n})") };
    egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("masks", layer.id)).default_open(n > 0).show(
        ui,
        |ui| {
            let mut meta = layer.meta.clone();
            let mut remove = None;
            for (i, m) in meta.masks.iter_mut().enumerate() {
                ui.push_id(("mask", i), |ui| {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut m.enabled, "").on_hover_text("On / off");
                        ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(110.0));
                        egui::ComboBox::from_id_salt("mode").selected_text(m.mode.label()).width(84.0).show_ui(
                            ui,
                            |ui| {
                                for mode in MaskMode::ALL {
                                    ui.selectable_value(&mut m.mode, mode, mode.label());
                                }
                            },
                        );
                        ui.checkbox(&mut m.inverted, "Invert");
                        if ui.small_button("Remove").on_hover_text("Remove this mask from the layer").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                meta.masks.remove(i);
            }
            if meta != layer.meta {
                // Typing a name merges into one undo step; other changes are their own.
                let label = if remove.is_some() { "Delete mask" } else { "Mask settings" };
                app.exec_merged(label, Command::SetLayerMeta { layer: layer.id, meta });
                return;
            }
            if n > 0 {
                egui::Grid::new(("mask-props", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                    for index in 0..n {
                        let name = layer
                            .meta
                            .masks
                            .get(index)
                            .map_or_else(|| format!("Mask {}", index + 1), |m| m.name.clone());
                        ui.label(RichText::new(name).strong());
                        ui.end_row();
                        for field in MaskField::ALL {
                            let path = PropPath::Mask { index, field };
                            crate::panels::prop_row_named(
                                app,
                                ui,
                                layer,
                                path,
                                &crate::widgets::prop_field_label(path),
                            );
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.button("+ Rectangle").on_hover_text("A mask around the layer: drag its edges to crop").clicked() {
                    app.select_only(Some(layer.id));
                    add_mask(app, false);
                }
                if ui.button("+ Ellipse").clicked() {
                    app.select_only(Some(layer.id));
                    add_mask(app, true);
                }
                if ui.button("Draw…").on_hover_text("Mask tool (U)").clicked() {
                    app.set_tool(Tool::Mask);
                }
            });
        },
    );
}
