//! Layer › New Effect…: every ready-made effect as a card with a moving
//! preview (drawn offscreen from a little scene of its own). Clicking one
//! adds it at the playhead; effects that work on a layer use the selection.

use crate::app::XenifyApp;
use egui::{Color32, RichText};
use xenify_doc::Id;
use xenify_ops::effects;
use std::collections::HashMap;

const OPEN: &str = "effects-gallery";
/// Preview size (points) and frames in its loop.
const CARD: egui::Vec2 = egui::vec2(176.0, 99.0);
const FRAMES: usize = 12;
/// New preview frames drawn per UI frame (keeps the window responsive).
const PER_FRAME: usize = 6;

/// The gallery's renderer and preview scenes (made when first needed).
#[derive(Default)]
pub struct Previews {
    gpu: Option<Result<xenify_render::Offscreen, String>>,
    scenes: HashMap<&'static str, Option<(xenify_doc::Project, Id)>>,
}

pub fn open(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(OPEN), true));
}

/// When preview frame `f` is taken (seconds).
fn frame_time(f: usize) -> f64 {
    0.3 + f as f64 * 3.3 / FRAMES as f64
}

/// Preview frame `f` of effect `name`, if it's ready (drawing it when the
/// budget allows).
fn frame(
    ctx: &egui::Context,
    previews: &mut Previews,
    name: &'static str,
    f: usize,
    budget: &mut usize,
) -> Option<egui::TextureHandle> {
    let key = egui::Id::new(("effect-preview", name, f));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    let scene = previews
        .scenes
        .entry(name)
        .or_insert_with(|| effects::preview(name))
        .as_ref()
        .map(|(p, comp)| xenify_anim::evaluate_project(p, *comp, frame_time(f)))?;
    let (w, h) = ((CARD.x * 2.0) as u32, (CARD.y * 2.0) as u32);
    let gpu = previews.gpu.get_or_insert_with(|| xenify_render::Offscreen::new().map_err(|e| e.to_string()));
    let img = gpu.as_mut().ok()?.render(&scene, w, h).ok()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img.pixels);
    let tex = ctx.load_texture(format!("effect-{name}-{f}"), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}

/// Shows the gallery while it's open (call every frame).
pub fn window(app: &mut XenifyApp, ctx: &egui::Context) {
    let id = egui::Id::new(OPEN);
    if !ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let mut picked = None;
    let target = app.selected.and_then(|s| app.project().layer(s).ok()).map(|l| l.meta.name.clone());
    let (_, closed) = crate::widgets::modal(ctx, OPEN, "Effects", 820.0, |ui| {
        ui.label(match &target {
            Some(name) => format!("Click one to add it at the playhead. Effects on a layer use “{name}”."),
            None => {
                "Click one to add it at the playhead. Select a layer first for the ones that work on a layer.".into()
            }
        });
        ui.add_space(6.0);
        let mut budget = PER_FRAME;
        let now = ui.input(|i| i.time);
        // Reduce motion: one still frame from the middle of each effect.
        let still = crate::theme::reduce_motion(ui.ctx());
        let f = if still { FRAMES / 2 } else { ((now * FRAMES as f64 / 3.3) as usize) % FRAMES };
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            for (section, names) in effects::CATALOG {
                ui.label(RichText::new(*section).strong());
                ui.horizontal_wrapped(|ui| {
                    for &name in *names {
                        let usable = !effects::needs_target(name) || target.is_some();
                        let (rect, resp) = ui.allocate_exact_size(CARD + egui::vec2(0.0, 20.0), egui::Sense::click());
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, usable, name));
                        let img_rect = egui::Rect::from_min_size(rect.min, CARD);
                        let p = ui.painter();
                        p.rect_filled(rect, 4.0, Color32::from_gray(26));
                        // The playing frame, else the first one, else a placeholder.
                        let ctx = ui.ctx().clone();
                        let shown = frame(&ctx, &mut app.effect_previews, name, f, &mut budget)
                            .or_else(|| frame(&ctx, &mut app.effect_previews, name, 0, &mut budget));
                        match shown {
                            Some(t) => {
                                let tint = if usable { Color32::WHITE } else { Color32::from_gray(90) };
                                p.image(
                                    t.id(),
                                    img_rect,
                                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                    tint,
                                );
                            }
                            None => {
                                p.text(
                                    img_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "…",
                                    egui::FontId::proportional(18.0),
                                    Color32::GRAY,
                                );
                            }
                        }
                        if resp.hovered() && usable {
                            p.rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(2.0, crate::widgets::ACCENT),
                                egui::StrokeKind::Inside,
                            );
                        }
                        p.text(
                            egui::pos2(rect.min.x + 6.0, rect.max.y - 10.0),
                            egui::Align2::LEFT_CENTER,
                            name,
                            egui::FontId::proportional(12.5),
                            if usable { ui.visuals().text_color() } else { Color32::from_gray(110) },
                        );
                        let tip = if usable {
                            effects::tip(name).to_string()
                        } else {
                            format!("{} (select the layer first)", effects::tip(name))
                        };
                        if resp.on_hover_text(tip).clicked() && usable {
                            picked = Some(name);
                        }
                    }
                });
                ui.add_space(6.0);
            }
        });
        // Keep the previews moving (and finish drawing them).
        if !still || budget == 0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
    });
    if let Some(name) = picked {
        add(app, name);
    }
    if closed || picked.is_some() {
        ctx.data_mut(|d| d.remove::<bool>(id));
    }
}

/// The selected layer's box: on the canvas, and in its own space.
fn target_boxes(app: &XenifyApp, id: Id) -> (Option<kurbo::Rect>, Option<kurbo::Rect>) {
    let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
    let world = xenify_anim::world_box(app.comp(), &scene, id, &mut app.viewport.measurer());
    let to_layer = xenify_anim::world_transform(app.comp(), id, app.time);
    let local =
        world.filter(|_| to_layer.determinant().abs() > 1e-12).map(|b| to_layer.inverse().transform_rect_bbox(b));
    (world, local)
}

/// Adds effect `name` at the playhead (on the selection where it needs a layer).
pub fn add(app: &mut XenifyApp, name: &str) {
    add_at(app, name, None);
}

/// [`add`], placed at `at` (composition space) instead of its usual spot.
pub fn add_at(app: &mut XenifyApp, name: &str, at: Option<kurbo::Vec2>) {
    let target = app.selected.filter(|_| effects::needs_target(name));
    let (target_box, target_local_box) = target.map_or((None, None), |t| target_boxes(app, t));
    let placement = effects::Placement { at, time: app.time, target, target_box, target_local_box };
    let first = Id(app.project().next_id);
    match effects::add_effect(app.project(), app.comp, first, name, placement) {
        Ok((cmd, select)) => {
            if app.exec(&format!("Add {name}"), cmd) {
                app.select_only(Some(select));
                app.status = format!("Added {name}. Its settings are in Properties; press Space to watch it.");
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

/// The canvas menu's Effect Here: the catalogue by section, placed at `at`.
pub fn here_menu(app: &mut XenifyApp, ui: &mut egui::Ui, at: kurbo::Vec2) {
    for (section, names) in effects::CATALOG {
        ui.menu_button(*section, |ui| {
            for &name in *names {
                let usable = !effects::needs_target(name) || app.selected.is_some();
                if ui
                    .add_enabled(usable, egui::Button::new(name))
                    .on_hover_text(effects::tip(name))
                    .on_disabled_hover_text("Select the layer first")
                    .clicked()
                {
                    add_at(app, name, Some(at));
                    ui.close();
                }
            }
        });
    }
    ui.separator();
    if ui.button("Effect Gallery…").clicked() {
        open(ui.ctx());
        ui.close();
    }
}
