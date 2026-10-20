//! The Pens panel: every pen style drawn as a sample stroke, your own saved
//! pens, and the current pen's settings. Clicking a pen uses it for the
//! selected strokes (or for new strokes).

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{Color, PenKind, PenStyle, ShapeItem};
use std::cell::RefCell;

/// Size of a preview tile (points).
const TILE: egui::Vec2 = egui::vec2(120.0, 40.0);

thread_local! {
    static GPU: RefCell<Option<Result<xenify_render::Offscreen, String>>> = const { RefCell::new(None) };
}

/// A texture showing `pen` drawing a wavy line, cached per pen settings.
fn preview(ctx: &egui::Context, pen: &PenStyle) -> Option<egui::TextureHandle> {
    let key = egui::Id::new(("pen-preview", format!("{pen:?}")));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    let (w, h) = ((TILE.x * 2.0) as u32, (TILE.y * 2.0) as u32);
    let mut path = kurbo::BezPath::new();
    path.move_to((14.0, h as f64 * 0.65));
    path.curve_to(
        (w as f64 * 0.3, -(h as f64) * 0.2),
        (w as f64 * 0.6, h as f64 * 1.2),
        (w as f64 - 14.0, h as f64 * 0.35),
    );
    let mut shape = ShapeItem::path(path, None, Some((Color::rgb(0.92, 0.94, 0.97), 7.0)));
    if let Some(s) = shape.stroke.as_mut() {
        s.style = pen.clone();
        s.cap = xenify_doc::LineCap::Round;
    }
    let mut comp = xenify_doc::Project::new().compositions.remove(0);
    comp.settings.width = w;
    comp.settings.height = h;
    comp.settings.background = Color::rgba(0.0, 0.0, 0.0, 0.0);
    comp.layers.push(xenify_doc::Layer::shape(xenify_doc::Id(1), "pen", vec![shape]));
    let scene = xenify_anim::evaluate(&comp, 0.0);
    let img = GPU.with_borrow_mut(|g| {
        let gpu = g.get_or_insert_with(|| xenify_render::Offscreen::new().map_err(|e| e.to_string()));
        gpu.as_mut().ok()?.render(&scene, w, h).ok()
    })?;
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img.pixels);
    let tex = ctx.load_texture(format!("pen-{}", key.value()), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}

/// One pen tile; returns true when clicked.
fn tile(ui: &mut egui::Ui, pen: &PenStyle, label: &str, current: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(TILE + egui::vec2(0.0, 16.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, current, label));
    let visuals = ui.style().interact_selectable(&resp, current);
    ui.painter().rect_filled(rect, 4.0, if current { visuals.bg_fill } else { egui::Color32::from_gray(28) });
    if current || resp.hovered() {
        ui.painter().rect_stroke(rect, 4.0, visuals.bg_stroke, egui::StrokeKind::Inside);
    }
    let img_rect = egui::Rect::from_min_size(rect.min, TILE);
    match preview(ui.ctx(), pen) {
        Some(t) => {
            ui.painter().image(
                t.id(),
                img_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            ui.painter().text(
                img_rect.center(),
                egui::Align2::CENTER_CENTER,
                "—",
                egui::FontId::proportional(14.0),
                egui::Color32::GRAY,
            );
        }
    }
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.5),
        visuals.text_color(),
    );
    resp
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let current = crate::control_bar::current_pen(app);
    let target = if app.selected_layers().is_empty() { "new strokes" } else { "the selected strokes" };
    ui.weak(format!("Click a pen to use it for {target}."));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let mut chosen: Option<PenStyle> = None;
        ui.horizontal_wrapped(|ui| {
            for kind in PenKind::ALL {
                let preset = PenStyle { seed: current.seed, ..PenStyle::preset(kind) };
                let on = current.kind == kind && current.name.is_empty();
                if tile(ui, &preset, kind.label(), on).on_hover_text(kind.label()).clicked() {
                    chosen = Some(preset);
                }
            }
        });
        if !app.my_pens.is_empty() {
            ui.separator();
            ui.label(RichText::new("My pens").strong());
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (i, p) in app.my_pens.iter().enumerate() {
                    let r = tile(ui, p, &p.name, current == *p)
                        .on_hover_text(format!("{} — right-click to delete", p.kind.label()));
                    if r.clicked() {
                        chosen = Some(p.clone());
                    }
                    r.context_menu(|ui| {
                        if ui.button("Delete this pen").clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                }
            });
            if let Some(i) = remove {
                let pen = app.my_pens.remove(i);
                app.status = format!("Deleted the pen “{}” (Restore brings it back).", pen.name);
                app.deleted_pen = Some((i, pen));
            }
        }
        if let Some((i, pen)) = app.deleted_pen.clone()
            && ui.button(format!("Restore “{}”", pen.name)).on_hover_text("Undo deleting this pen").clicked()
        {
            let at = i.min(app.my_pens.len());
            app.my_pens.insert(at, pen);
            app.deleted_pen = None;
        }
        if let Some(p) = chosen {
            crate::control_bar::apply_pen(app, p, false);
        }
        ui.separator();
        // The current pen's settings; changes apply straight away.
        let mut pen = crate::control_bar::current_pen(app);
        let before = pen.clone();
        let mut begin = false;
        crate::control_bar::pen_settings(app, ui, &mut pen, &mut begin);
        if pen != before {
            if begin {
                app.new_gesture();
            }
            crate::control_bar::apply_pen(app, pen, true);
        }
        ui.weak("Name it and press Save to keep it as your own pen (it appears above, with a preview).");
    });
}
