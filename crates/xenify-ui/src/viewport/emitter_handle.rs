//! Particle emitters on the canvas: every emitter shows its source shape
//! and a small marker (click it to select the layer, even before any
//! particles fly); the selected one also shows an arrow for its initial
//! direction and speed, which can be dragged (Shift: 15° steps).

use super::{Drag, HANDLE, Viewport};
use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke};
use xenify_doc::{Emitter, EmitterField, EmitterShape, Id, LayerKind, PropPath, Value};
use kurbo::{Affine, Point, Shape, Vec2};

/// Composition pixels of arrow per pixel-per-second of speed.
const ARROW_SCALE: f64 = 0.2;

/// Where an emitter sits and faces: origin (composition space) and the
/// layer's turn in degrees.
fn frame(app: &XenifyApp, id: Id) -> (Point, f64, Affine) {
    let world = xenify_anim::world_transform(app.comp(), id, app.time);
    let v = world * Point::new(1.0, 0.0) - world * Point::ZERO;
    (world * Point::ZERO, v.y.atan2(v.x).to_degrees(), world)
}

fn emitter(app: &XenifyApp, id: Id) -> Option<Emitter> {
    match &app.project().layer(id).ok()?.kind {
        LayerKind::Particles(e) => Some((**e).clone()),
        _ => None,
    }
}

/// The arrow's tip (composition space) for emitter `e` of layer `id`.
fn tip(app: &XenifyApp, id: Id, e: &Emitter) -> (Point, Point) {
    let (origin, turn, _) = frame(app, id);
    let dir = (e.direction.value_at(app.time) + turn).to_radians();
    let len = (e.speed.value_at(app.time) * ARROW_SCALE).max(1.0);
    (origin, origin + Vec2::new(dir.cos(), dir.sin()) * len)
}

/// The particle layer whose marker is under `at`, if any.
pub fn hit(app: &XenifyApp, at: Point, tolerance: f64) -> Option<Id> {
    app.comp()
        .layers
        .iter()
        .rev()
        .filter(|l| matches!(l.kind, LayerKind::Particles(_)) && l.meta.visible && !l.meta.locked)
        .find(|l| frame(app, l.id).0.distance(at) <= tolerance)
        .map(|l| l.id)
}

impl Viewport {
    fn selected_emitter(&self, app: &XenifyApp) -> Option<(Id, Emitter)> {
        if app.tool != Tool::Select || !app.extra_selected.is_empty() {
            return None;
        }
        let id = app.selected?;
        Some((id, emitter(app, id)?))
    }

    /// The direction arrow's tip under `pos`: start dragging it.
    pub(super) fn emitter_handle_at(&self, app: &XenifyApp, rect: Rect, pos: Pos2) -> Option<Drag> {
        let (id, e) = self.selected_emitter(app)?;
        let (origin, tip) = tip(app, id, &e);
        let near = (self.to_screen(rect, tip) - pos).length() <= HANDLE + 4.0;
        let (_, turn, _) = frame(app, id);
        near.then_some(Drag::EmitterArrow { layer: id, origin, turn })
    }

    pub(super) fn draw_emitters(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect);
        let selected = self.selected_emitter(app).map(|(id, _)| id);
        for l in app.comp().layers.iter().filter(|l| l.meta.visible) {
            let LayerKind::Particles(e) = &l.kind else { continue };
            let (origin, _, world) = frame(app, l.id);
            let on = Some(l.id) == selected || app.is_selected(l.id);
            let colour = if on { ACCENT } else { Color32::from_rgb(255, 170, 60) };
            // The source shape, dashed.
            let outline = match &e.shape {
                EmitterShape::Point => None,
                EmitterShape::Line { length } => {
                    let mut p = kurbo::BezPath::new();
                    p.move_to((-length / 2.0, 0.0));
                    p.line_to((length / 2.0, 0.0));
                    Some(p)
                }
                EmitterShape::Rect { size } => {
                    Some(kurbo::Rect::from_center_size(Point::ZERO, size.to_size()).to_path(0.1))
                }
                EmitterShape::Ellipse { size } => Some(kurbo::Ellipse::new(Point::ZERO, *size / 2.0, 0.0).to_path(0.1)),
                EmitterShape::Layer { .. } => None,
            };
            if let Some(path) = outline {
                let mut pts = Vec::new();
                kurbo::flatten(world * path, 0.5, |el| match el {
                    kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => pts.push(self.to_screen(rect, p)),
                    kurbo::PathEl::ClosePath => {
                        if let Some(first) = pts.first().copied() {
                            pts.push(first);
                        }
                    }
                    _ => {}
                });
                painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, colour.gamma_multiply(0.8)), 6.0, 4.0));
            }
            // The marker: a small sunburst.
            let c = self.to_screen(rect, origin);
            painter.circle(c, 4.0, colour, Stroke::new(1.5, Color32::from_black_alpha(160)));
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let d = egui::vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 6.0, c + d * 10.0], Stroke::new(1.2, colour));
            }
            if Some(l.id) != selected {
                continue;
            }
            let (from, to) = tip(app, l.id, e);
            let (a, b) = (self.to_screen(rect, from), self.to_screen(rect, to));
            let shaft = Stroke::new(2.0, ACCENT);
            painter.line_segment([a, b], Stroke::new(4.0, Color32::from_black_alpha(120)));
            painter.line_segment([a, b], shaft);
            let dir = (b - a).normalized();
            if dir.is_finite() {
                let side = egui::vec2(-dir.y, dir.x);
                let head = vec![b + dir * 4.0, b - dir * 10.0 + side * 6.0, b - dir * 10.0 - side * 6.0];
                painter.add(egui::Shape::convex_polygon(head, ACCENT, Stroke::new(1.0, Color32::WHITE)));
            }
            // The spread: two faint lines either side.
            let spread = e.spread.value_at(app.time).min(360.0);
            if spread > 0.5 && spread < 359.5 {
                let len = (b - a).length();
                let base = (b - a).angle();
                for s in [-0.5, 0.5] {
                    let ang = base + (spread * s).to_radians() as f32;
                    let end = a + egui::vec2(ang.cos(), ang.sin()) * len * 0.8;
                    painter.line_segment([a, end], Stroke::new(1.0, ACCENT.gamma_multiply(0.5)));
                }
            }
            painter.text(
                b + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.0} px/s · {:.0}°", e.speed.value_at(app.time), e.direction.value_at(app.time)),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }
}

impl Viewport {
    fn selected_liquid(&self, app: &XenifyApp) -> Option<(Id, xenify_doc::LiquidFill)> {
        if app.tool != Tool::Select || !app.extra_selected.is_empty() {
            return None;
        }
        let id = app.selected?;
        match &app.project().layer(id).ok()?.kind {
            LayerKind::Liquid(w) => Some((id, (**w).clone())),
            _ => None,
        }
    }

    /// The water level handle under `pos`: start dragging it.
    pub(super) fn liquid_handle_at(&self, app: &XenifyApp, rect: Rect, pos: Pos2) -> Option<Drag> {
        let (id, w) = self.selected_liquid(app)?;
        let world = xenify_anim::world_transform(app.comp(), id, app.time);
        let at = world * xenify_anim::liquid::surface_middle(&w, app.time);
        let near = (self.to_screen(rect, at) - pos).length() <= HANDLE + 4.0;
        (near && world.determinant().abs() > 1e-12).then(|| Drag::LiquidLevel { layer: id, inv: world.inverse() })
    }

    /// The selected water's box and its level handle.
    pub(super) fn draw_liquid(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let Some((id, w)) = self.selected_liquid(app) else { return };
        let world = xenify_anim::world_transform(app.comp(), id, app.time);
        let bx = world * xenify_anim::liquid::container(&w).to_path(0.1);
        let mut pts = Vec::new();
        kurbo::flatten(bx, 0.5, |el| match el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => pts.push(self.to_screen(rect, p)),
            kurbo::PathEl::ClosePath => {
                if let Some(first) = pts.first().copied() {
                    pts.push(first);
                }
            }
            _ => {}
        });
        let painter = painter.with_clip_rect(rect);
        painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT.gamma_multiply(0.8)), 6.0, 4.0));
        let at = self.to_screen(rect, world * xenify_anim::liquid::surface_middle(&w, app.time));
        painter.circle(at, HANDLE * 0.8, ACCENT, Stroke::new(1.5, Color32::WHITE));
        painter.text(
            at + egui::vec2(10.0, -10.0),
            egui::Align2::LEFT_BOTTOM,
            format!("Level {:.0}%", w.level.value_at(app.time) * 100.0),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }
}

/// Dragging the water's surface sets its level.
pub(super) fn drag_level(app: &mut XenifyApp, layer: Id, inv: Affine, to: Point) {
    let Ok(l) = app.project().layer(layer) else { return };
    let LayerKind::Liquid(w) = &l.kind else { return };
    let level = xenify_anim::liquid::level_for(w, inv * to);
    app.edit_value(
        layer,
        PropPath::Liquid(xenify_doc::LiquidField::Level),
        Value::Scalar((level * 1000.0).round() / 1000.0),
    );
}

/// Dragging the arrow: its angle is the direction, its length the speed.
pub(super) fn drag_arrow(app: &mut XenifyApp, layer: Id, origin: Point, turn: f64, to: Point, snap: bool) {
    let v = to - origin;
    if v.hypot() < 1.0 {
        return;
    }
    let mut deg = v.y.atan2(v.x).to_degrees() - turn;
    if snap {
        deg = (deg / 15.0).round() * 15.0;
    }
    // Keep it near the old value so keyframes don't spin the long way round.
    if let Some(e) = emitter(app, layer) {
        let old = e.direction.value_at(app.time);
        deg += 360.0 * ((old - deg) / 360.0).round();
    }
    app.edit_value(layer, PropPath::Emitter(EmitterField::Direction), Value::Scalar(deg));
    app.edit_value(layer, PropPath::Emitter(EmitterField::Speed), Value::Scalar((v.hypot() / ARROW_SCALE).round()));
}
