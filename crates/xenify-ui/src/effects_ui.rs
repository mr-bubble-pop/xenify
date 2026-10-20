//! The Effects section of Properties and Layer ▸ Effects: drop shadows,
//! glows, blur, colour adjustment and blend modes.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{BlendMode, Effect, EffectKind, Id, Layer, LayerKind, LayerMeta, PropPath};
use xenify_ops::Command;

/// Whether effects can apply to a layer (groups take them as a whole).
pub fn takes_effects(l: &Layer) -> bool {
    !matches!(l.kind, LayerKind::Null)
}

pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if !takes_effects(layer) {
        return;
    }
    let n = layer.meta.effects.len();
    let title = if n == 0 { "Effects".to_string() } else { format!("Effects ({n})") };
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(("fx", layer.id))
        .default_open(n > 0)
        .show(ui, |ui| editor(app, ui, layer, None));
}

/// The blend mode and effects of `layer` (or only its effect `only`): on/off,
/// settings, order, add and remove.
pub fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, only: Option<usize>) {
    // (The blend mode is in Transform, beside Opacity.)
    let mut meta = layer.meta.clone();
    let mut remove = None;
    let mut move_up = None;
    let n = meta.effects.len();
    for (i, e) in meta.effects.iter_mut().enumerate() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        if only.is_none() {
            ui.separator();
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut e.enabled, RichText::new(e.kind.label()).strong()).on_hover_text("On / off");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                // Moving one down is moving the next one up.
                if only.is_none() && i + 1 < n && ui.small_button("⏷").on_hover_text("Move down").clicked() {
                    move_up = Some(i + 1);
                }
                if only.is_none() && i > 0 && ui.small_button("⏶").on_hover_text("Move up").clicked() {
                    move_up = Some(i);
                }
            });
        });
        // Each setting is a property row: ⏱ animates it. The settings window
        // shows them while the effect is off too, to set it up first.
        if e.enabled || only.is_some() {
            egui::Grid::new(("fx-props", layer.id, i)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                for &field in e.kind.fields() {
                    let path = PropPath::Effect { index: i, field };
                    crate::panels::prop_row_named(app, ui, layer, path, &crate::widgets::prop_field_label(path));
                }
            });
        }
    }
    if let Some(i) = remove {
        meta.effects.remove(i);
    }
    if let Some(i) = move_up {
        meta.effects.swap(i, i - 1);
    }
    if only.is_none() {
        ui.separator();
        ui.menu_button("+ Add effect", |ui| {
            for kind in EffectKind::presets() {
                if ui.button(kind.label()).clicked() {
                    meta.effects.push(Effect::new(kind));
                    ui.close();
                }
            }
        });
    }
    // Property rows edit the project directly; only structural
    // changes (on/off, order, add/remove, blend) go through here.
    let fresh = app.project().layer(layer.id).map(|l| l.meta.clone()).unwrap_or_else(|_| layer.meta.clone());
    if meta != layer.meta && fresh == layer.meta {
        app.exec("Effects", Command::SetLayerMeta { layer: layer.id, meta });
    }
}

/// Which layer (and effect) the effects window shows.
#[derive(Clone, Copy)]
struct Open {
    layer: Id,
    only: Option<usize>,
    /// Where it opens: left of the pointer when asked.
    at: Option<egui::Pos2>,
}

fn open_id() -> egui::Id {
    egui::Id::new("fx-window-open")
}

/// Opens the effects window on `layer`'s effects (or only effect `only`).
pub fn open_window(ctx: &egui::Context, layer: Id, only: Option<usize>) {
    let at = ctx.pointer_latest_pos().map(|p| p - egui::vec2(340.0, 20.0));
    ctx.data_mut(|d| d.insert_temp(open_id(), Open { layer, only, at }));
}

/// The effects window (double-click an effect or the fx badge in Layers).
pub fn window(app: &mut XenifyApp, ctx: &egui::Context) {
    let Some(o) = ctx.data(|d| d.get_temp::<Open>(open_id())) else { return };
    let Some(layer) = app.project().layer(o.layer).ok().cloned() else {
        ctx.data_mut(|d| d.remove::<Open>(open_id()));
        return;
    };
    let only = o.only.filter(|i| *i < layer.meta.effects.len());
    let title = match only {
        Some(i) => format!("{} — {}", layer.meta.effects[i].kind.label(), layer.meta.name),
        None => format!("Effects — {}", layer.meta.name),
    };
    let mut open = true;
    let mut show_all = false;
    egui::Window::new(title)
        .id(egui::Id::new("fx-window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .default_pos(o.at.unwrap_or(egui::pos2(200.0, 120.0)))
        .show(ctx, |ui| {
            editor(app, ui, &layer, only);
            if only.is_some() {
                ui.separator();
                show_all = ui.button("All effects of this layer…").clicked();
            }
        });
    if !open {
        ctx.data_mut(|d| d.remove::<Open>(open_id()));
    } else if show_all {
        open_window(ctx, layer.id, None);
    }
}

/// Turns effect `index` of `layer` on or off.
pub fn toggle(app: &mut XenifyApp, layer: &Layer, index: usize) {
    let mut meta = layer.meta.clone();
    let Some(e) = meta.effects.get_mut(index) else { return };
    e.enabled = !e.enabled;
    let label = if e.enabled { "Turn effect on" } else { "Turn effect off" };
    app.exec(label, Command::SetLayerMeta { layer: layer.id, meta });
}

/// Removes effect `index` of `layer`.
pub fn remove(app: &mut XenifyApp, layer: &Layer, index: usize) {
    let mut meta = layer.meta.clone();
    if index < meta.effects.len() {
        meta.effects.remove(index);
        app.exec("Remove effect", Command::SetLayerMeta { layer: layer.id, meta });
    }
}

/// Layer ▸ Effects: add to or clear the selected layers' effects, set their blend.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let targets: Vec<Layer> = app
        .selected_layers()
        .iter()
        .filter_map(|id| app.project().layer(*id).ok().cloned())
        .filter(takes_effects)
        .collect();
    ui.add_enabled_ui(!targets.is_empty(), |ui| {
        ui.menu_button("Effects", |ui| {
            for kind in EffectKind::presets() {
                if ui.button(format!("Add {}", kind.label())).clicked() {
                    edit_all(app, &targets, "Add effect", |m| m.effects.push(Effect::new(kind.clone())));
                    ui.close();
                }
            }
            ui.menu_button("Blend Mode", |ui| {
                let current = targets.first().map(|l| l.meta.blend);
                for b in BlendMode::ALL {
                    if ui.selectable_label(current == Some(b), b.label()).clicked() {
                        edit_all(app, &targets, "Blend mode", |m| m.blend = b);
                        ui.close();
                    }
                }
            });
            ui.separator();
            let any = targets.iter().any(|l| !l.meta.effects.is_empty() || !l.meta.blend.is_normal());
            if ui.add_enabled(any, egui::Button::new("Clear Effects")).clicked() {
                edit_all(app, &targets, "Clear effects", |m| {
                    m.effects.clear();
                    m.blend = BlendMode::Normal;
                });
                ui.close();
            }
        })
        .response
        .on_disabled_hover_text("Select layers or groups first");
    });
}

fn edit_all(app: &mut XenifyApp, layers: &[Layer], label: &str, f: impl Fn(&mut LayerMeta)) {
    let cmds = layers
        .iter()
        .map(|l| {
            let mut meta = l.meta.clone();
            f(&mut meta);
            Command::SetLayerMeta { layer: l.id, meta }
        })
        .collect();
    app.exec(label, Command::batch(cmds));
}
