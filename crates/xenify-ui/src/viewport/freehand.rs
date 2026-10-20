//! Pencil and Brush (freehand strokes, optionally recognised as clean
//! shapes) and the Sculpt brush (push / attract / repel path points).

use super::{Viewport, add_layer};
use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke};
use xenify_doc::{Color, Geometry, Id, LayerKind, PropPath, ShapeField, ShapeItem, Value, Vec2};
use xenify_ops::Command;
use xenify_ops::draw::{Recognized, SculptMode};
use kurbo::{Affine, BezPath, Point};

impl Viewport {
    pub(super) fn freehand_tool(&mut self, app: &mut XenifyApp, rect: Rect, resp: &egui::Response, comp_pos: Point) {
        if resp.hovered() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            self.stroke = vec![origin.map_or(comp_pos, |p| self.to_comp(rect, p))];
        }
        if resp.dragged()
            && !self.stroke.is_empty()
            && self.stroke.last().is_none_or(|p| p.distance(comp_pos) * self.zoom >= 1.0)
        {
            self.stroke.push(comp_pos);
        }
        if resp.drag_stopped() && !self.stroke.is_empty() {
            let points = std::mem::take(&mut self.stroke);
            let recognize = app.tool_options.recognize && !resp.ctx.input(|i| i.modifiers.alt);
            finish_stroke(app, &points, recognize);
        }
    }

    pub(super) fn draw_freehand(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        if self.stroke.len() < 2 {
            return;
        }
        let pts: Vec<Pos2> = self.stroke.iter().map(|p| self.to_screen(rect, *p)).collect();
        let color = preview_color(app);
        let width = if app.tool == Tool::Brush {
            (app.tool_options.brush_size * self.zoom) as f32
        } else {
            ((app.tool_options.pencil_width * self.zoom) as f32).max(1.0)
        };
        painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
    }

    pub(super) fn sculpt_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        comp_pos: Point,
        scene: &xenify_anim::Scene,
    ) {
        if resp.hovered() || resp.dragged() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let start = origin.map_or(comp_pos, |p| self.to_comp(rect, p));
            if app.selected_layers().is_empty() {
                let hit = xenify_anim::hit_test(scene, start, 4.0 / self.zoom, &mut *self.text.borrow_mut());
                app.select_only(hit);
            }
            app.new_gesture();
            // Rectangles and ellipses become paths so their points can move.
            let convert: Vec<Command> =
                sculpt_layers(app).into_iter().filter_map(|id| super::convert_to_path_command(app, id)).collect();
            if !convert.is_empty() {
                app.exec_merged("Sculpt", Command::batch(convert));
            }
            if sculpt_layers(app).is_empty() {
                app.status = "Sculpt works on shapes: select one (or start the stroke on it).".into();
            }
            self.sculpt_last = Some(start);
        }
        let Some(last) = self.sculpt_last else { return };
        if resp.dragged() {
            let o = &app.tool_options;
            let mut mode = o.sculpt_mode;
            if resp.ctx.input(|i| i.modifiers.alt) {
                mode = match mode {
                    SculptMode::Attract => SculptMode::Repel,
                    SculptMode::Repel => SculptMode::Attract,
                    m => m,
                };
            }
            let radius = o.sculpt_radius / self.zoom;
            let dt = resp.ctx.input(|i| i.stable_dt).clamp(0.001, 0.05) as f64;
            // Pixels per second at full strength: a third of the brush.
            let amount = o.sculpt_strength * radius * 2.0 * dt;
            let delta = comp_pos - last;
            if mode != SculptMode::Push || delta.hypot() > 0.0 {
                sculpt_step(app, mode, comp_pos, radius, delta, amount);
            }
            self.sculpt_last = Some(comp_pos);
            if mode != SculptMode::Push {
                // Attract/repel keep working while the brush is held still.
                resp.ctx.request_repaint();
            }
        }
        if resp.drag_stopped() {
            self.sculpt_last = None;
        }
    }

    /// The Brush and Pencil's size as a ring around the pointer.
    pub(super) fn draw_brush_ring(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect, hover: Option<Pos2>) {
        let Some(p) = hover.filter(|p| rect.contains(*p)) else { return };
        let width = if app.tool == Tool::Brush { app.tool_options.brush_size } else { app.tool_options.pencil_width };
        let r = ((width * self.zoom) as f32 / 2.0).max(1.5);
        painter.circle_stroke(p, r, Stroke::new(2.0, Color32::from_black_alpha(120)));
        painter.circle_stroke(p, r, Stroke::new(1.0, Color32::WHITE));
    }

    pub(super) fn draw_sculpt_brush(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect, hover: Option<Pos2>) {
        let Some(p) = hover.filter(|p| rect.contains(*p)) else { return };
        let r = app.tool_options.sculpt_radius as f32;
        painter.circle_stroke(p, r, Stroke::new(2.0, Color32::from_black_alpha(120)));
        painter.circle_stroke(p, r, Stroke::new(1.0, Color32::WHITE));
        let inner = match app.tool_options.sculpt_mode {
            SculptMode::Push => 0.0,
            _ => r * 0.5,
        };
        if inner > 0.0 {
            painter.circle_stroke(p, inner, Stroke::new(1.0, Color32::from_white_alpha(90)));
        }
        painter.circle_filled(p, 2.0, ACCENT);
    }
}

/// Shape layers in the selection (groups expanded).
fn sculpt_layers(app: &XenifyApp) -> Vec<Id> {
    let mut out = Vec::new();
    for id in app.selected_layers() {
        for l in std::iter::once(id).chain(xenify_anim::descendants(app.comp(), id)) {
            if app.project().layer(l).is_ok_and(|l| matches!(l.kind, LayerKind::Shape { .. }) && !l.meta.locked)
                && !out.contains(&l)
            {
                out.push(l);
            }
        }
    }
    out
}

fn sculpt_step(app: &mut XenifyApp, mode: SculptMode, center: Point, radius: f64, delta: kurbo::Vec2, amount: f64) {
    let t = app.time;
    let mut cmds = Vec::new();
    for id in sculpt_layers(app) {
        let w = xenify_anim::world_transform(app.comp(), id, t);
        if w.determinant().abs() < 1e-12 {
            continue;
        }
        let inv = w.inverse();
        let scale = w.determinant().abs().sqrt();
        let c = inv.as_coeffs();
        let local_delta = kurbo::Vec2::new(c[0] * delta.x + c[2] * delta.y, c[1] * delta.x + c[3] * delta.y);
        let Ok(layer) = app.project().layer(id) else { continue };
        for (index, shape) in layer.shapes().iter().enumerate() {
            let Geometry::Path { path } = &shape.geometry else { continue };
            let before = path.value_at(t);
            let local_center = inv * center;
            // Skip shapes the brush doesn't reach.
            let b = kurbo::Shape::bounding_box(&before).inflate(radius / scale, radius / scale);
            if !b.contains(local_center) {
                continue;
            }
            let after =
                xenify_ops::draw::sculpt(&before, mode, local_center, radius / scale, local_delta, amount / scale);
            if after != before
                && let Ok(cmd) = Command::edit_value(
                    app.project(),
                    id,
                    PropPath::shape(index, ShapeField::Path),
                    t,
                    Value::Path(after),
                    app.auto_key,
                )
            {
                cmds.push(cmd);
            }
        }
    }
    if !cmds.is_empty() {
        app.exec_merged("Sculpt", Command::batch(cmds));
    }
}

fn preview_color(app: &XenifyApp) -> Color32 {
    let c = match app.tool {
        Tool::Brush => app.paint.fill.or(app.paint.stroke),
        _ => app.paint.stroke.or(app.paint.fill),
    };
    crate::colors::to_color32(c.unwrap_or(Color::WHITE))
}

/// A stroked (unfilled) path in the current stroke colour (or fill, if there's no stroke).
fn line_shape(app: &XenifyApp, path: BezPath, width: f64) -> ShapeItem {
    let color = app.paint.stroke.or(app.paint.fill).unwrap_or(Color::WHITE);
    let mut s = ShapeItem::path(path, None, Some((color, width)));
    if let Some(st) = &mut s.stroke {
        st.cap = xenify_doc::LineCap::Round;
        st.join = xenify_doc::LineJoin::Round;
    }
    s
}

/// A shape with the current fill and stroke (a white outline if both are off).
fn painted(app: &XenifyApp, shape: ShapeItem) -> ShapeItem {
    let mut s = app.paint.apply_to(shape);
    if s.fill.is_none() && s.stroke.is_none() {
        s = ShapeItem { fill: Some(xenify_doc::Fill::solid(Color::WHITE)), ..s };
    }
    s
}

/// Adds a layer centred on `path`'s box, with the path re-centred on the layer.
fn add_centred(app: &mut XenifyApp, name: &str, path: BezPath, make: impl FnOnce(&XenifyApp, BezPath) -> ShapeItem) {
    let c = kurbo::Shape::bounding_box(&path).center().to_vec2();
    let shape = make(app, Affine::translate(-c) * path);
    add_layer(app, name, vec![shape], c);
}

fn finish_stroke(app: &mut XenifyApp, points: &[Point], recognize: bool) {
    let tool = app.tool;
    if points.len() < 2 {
        return;
    }
    let brush = tool == Tool::Brush;
    let width = if brush { app.tool_options.brush_size } else { app.tool_options.pencil_width };
    let mut result = if recognize {
        xenify_ops::draw::recognize(points)
    } else {
        Recognized::Free { path: xenify_ops::draw::smooth_stroke(points, false), closed: false }
    };
    // Smoother strokes have fewer points (Smoothing in the tool options).
    if let Recognized::Free { path, .. } = &mut result {
        let b = kurbo::Shape::bounding_box(&*path);
        let accuracy =
            (b.width().max(b.height()) * 0.004).clamp(0.3, 2.5) * app.tool_options.smoothing.clamp(1.0, 10.0);
        *path = xenify_doc::bezier::simplify(path, accuracy);
    }
    match result {
        Recognized::Line(a, b) => {
            if a.distance(b) < 1.0 {
                return;
            }
            let mut p = BezPath::new();
            p.move_to(a);
            p.line_to(b);
            add_centred(app, "Line", p, |app, p| line_shape(app, p, width));
        }
        Recognized::Ellipse { center, radii, angle } => {
            let name = if (radii.x - radii.y).abs() < 1e-9 { "Circle" } else { "Ellipse" };
            let shape = painted(app, ShapeItem::ellipse(Vec2::new(radii.x * 2.0, radii.y * 2.0), Color::WHITE));
            add_rotated(app, name, shape, center, angle);
        }
        Recognized::Rect { center, size, angle } => {
            let name = if (size.x - size.y).abs() < 0.08 * size.x.max(size.y) { "Square" } else { "Rectangle" };
            let size = if name == "Square" { Vec2::new(size.x.max(size.y), size.x.max(size.y)) } else { size };
            let shape = painted(app, ShapeItem::rect(size, Color::WHITE));
            add_rotated(app, name, shape, center, angle);
        }
        Recognized::Polygon(corners) => {
            let name = if corners.len() == 3 { "Triangle" } else { "Polygon" };
            let path = xenify_ops::draw::closed_polyline(&corners);
            add_centred(app, name, path, |app, p| painted(app, ShapeItem::path(p, None, None)));
        }
        Recognized::Free { path, closed } => {
            if brush {
                // The brush paints the stroke's outline as a filled shape.
                let outline = kurbo::stroke(
                    path.iter(),
                    &kurbo::Stroke::new(width).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round),
                    &kurbo::StrokeOpts::default(),
                    0.1,
                );
                let outline = xenify_doc::bezier::simplify(&outline, 0.3);
                let color = app.paint.fill.or(app.paint.stroke).unwrap_or(Color::WHITE);
                add_centred(app, "Brush stroke", outline, |_, p| ShapeItem::path(p, Some(color), None));
            } else {
                let mut p = path;
                if closed {
                    p.close_path();
                }
                add_centred(app, "Pencil", p, |app, p| line_shape(app, p, width));
            }
        }
    }
    // Keep drawing with the same tool.
    app.tool = tool;
}

fn add_rotated(app: &mut XenifyApp, name: &str, shape: ShapeItem, center: Point, angle: f64) {
    super::add_layer_rotated(app, name, vec![shape], center.to_vec2(), angle.to_degrees());
}
