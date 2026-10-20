//! The Shapes panel: the diagram library (flowchart, network, cloud &
//! software, BPMN). Click a shape to add it (next to the selected shape, or
//! in the middle of the view), or drag it onto the canvas.

use crate::app::XenifyApp;
use egui::{Color32, Pos2, Rect, Sense, Stroke, vec2};
use xenify_doc::Vec2;
use xenify_ops::library::{self, Category, LibShape, Role};

/// What's being dragged out of the panel.
#[derive(Clone, Copy, Debug)]
pub struct LibPayload(pub &'static str);

const TILE: f32 = 58.0;

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let search_id = egui::Id::new("shape-library-search");
    let mut search: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search shapes").desired_width(f32::INFINITY));
    });
    ui.data_mut(|d| d.insert_temp(search_id, search.clone()));
    let needle = search.trim().to_lowercase();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for cat in Category::ALL {
            let shapes: Vec<&LibShape> = library::all()
                .iter()
                .filter(|s| s.category == cat)
                .filter(|s| needle.is_empty() || s.name.to_lowercase().contains(&needle) || s.id.contains(&needle))
                .collect();
            if shapes.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(cat.label()).id_salt(("lib", cat.label())).default_open(true).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for s in shapes {
                        tile(app, ui, s);
                    }
                });
            });
        }
        ui.add_space(6.0);
        ui.weak("All shapes are Xenify Vector's own drawings (MIT licence), free to use anywhere.");
    });
}

fn tile(app: &mut XenifyApp, ui: &mut egui::Ui, s: &'static LibShape) {
    let (rect, resp) = ui.allocate_exact_size(vec2(TILE, TILE), Sense::click_and_drag());
    let visuals = ui.style().interact(&resp);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, visuals.bg_fill);
    }
    paint(ui.painter(), rect.shrink(8.0), s, visuals.fg_stroke.color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s.name));
    let resp = resp.on_hover_text(format!("{} — click to add, or drag onto the canvas", s.name));
    resp.dnd_set_drag_payload(LibPayload(s.id));
    if resp.clicked() {
        let at = place_for(app, s);
        add(app, s.id, at);
    }
}

/// Draws `s` as outlines, fitted into `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, s: &LibShape, color: Color32) {
    let (w, h) = s.size;
    let k = (rect.width() as f64 / w).min(rect.height() as f64 / h);
    let c = rect.center();
    let map = |p: kurbo::Point| Pos2::new(c.x + (p.x * k) as f32, c.y + (p.y * k) as f32);
    for part in s.parts(w, h) {
        if part.role == Role::Hit {
            continue;
        }
        let mut lines: Vec<(Vec<Pos2>, bool)> = Vec::new();
        kurbo::flatten(&part.path, 0.5 / k.max(1e-3), |el| match el {
            kurbo::PathEl::MoveTo(p) => lines.push((vec![map(p)], false)),
            kurbo::PathEl::LineTo(p) => {
                if let Some(l) = lines.last_mut() {
                    l.0.push(map(p));
                }
            }
            kurbo::PathEl::ClosePath => {
                if let Some(l) = lines.last_mut() {
                    l.1 = true;
                }
            }
            _ => {}
        });
        for (points, closed) in lines {
            if part.role == Role::Solid {
                painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
            } else if closed {
                painter.add(egui::Shape::closed_line(points, Stroke::new(1.2, color)));
            } else {
                painter.add(egui::Shape::line(points, Stroke::new(1.2, color)));
            }
        }
    }
}

/// Where a clicked shape goes: to the right of the selected shape, else the
/// middle of the view.
fn place_for(app: &XenifyApp, s: &LibShape) -> Vec2 {
    if let Some(id) = app.selected {
        let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
        if let Some(b) = xenify_anim::world_box(app.comp(), &scene, id, &mut xenify_anim::ApproxText) {
            return Vec2::new(b.x1 + 80.0 + s.size.0 / 2.0, b.center().y);
        }
    }
    let v = &app.viewport;
    v.screen_to_comp(v.screen_rect().center()).map_or_else(
        || Vec2::new(app.comp().settings.width as f64 / 2.0, app.comp().settings.height as f64 / 2.0),
        |p| p.to_vec2(),
    )
}

/// Adds library shape `id` centred at `at` and selects it.
pub fn add(app: &mut XenifyApp, id: &str, at: Vec2) {
    match library::insert(app.project(), app.comp, id, at, None, None) {
        Ok((cmd, layer)) => {
            if app.exec("Add shape", cmd) {
                app.select_only(Some(layer));
                // Dropped in a pool or lane: it joins it.
                if let Ok(Some(join)) = xenify_ops::diagram::adopt(app.project(), app.comp, &[layer], app.time) {
                    app.exec_merged("Add shape", join);
                }
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}
