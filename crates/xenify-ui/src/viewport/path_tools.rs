//! Pen tool (click for corners, drag for curves) and node editing (drag
//! anchors and handles of path shapes).

use super::{HANDLE, Viewport, add_layer};
use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2 as EVec2};
use xenify_doc::bezier::{self, PenPoint};
use xenify_doc::{Color, Geometry, Id, PropPath, ShapeField, ShapeItem, Value};
use kurbo::{Affine, BezPath, PathEl, Point};

/// An open path being continued with the pen.
#[derive(Clone, Copy, Debug)]
pub(super) struct PenContinue {
    layer: Id,
    shape: usize,
    subpath: usize,
    from_start: bool,
    /// The end being continued from, and the other end (to close on), in comp space.
    start: Point,
    other: Point,
    /// Composition → layer space.
    to_layer: Affine,
}

/// Draws a Bézier path with egui shapes, mapping each point through `map`.
pub(super) fn draw_bezpath(painter: &egui::Painter, path: &BezPath, map: impl Fn(Point) -> Pos2, stroke: Stroke) {
    let mut start = None;
    let mut last = Pos2::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                last = map(p);
                start = Some(last);
            }
            PathEl::LineTo(p) => {
                let p = map(p);
                painter.line_segment([last, p], stroke);
                last = p;
            }
            PathEl::QuadTo(a, b) => {
                let (a, b) = (map(a), map(b));
                painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
                    [last, a, b],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
                last = b;
            }
            PathEl::CurveTo(a, b, c) => {
                let (a, b, c) = (map(a), map(b), map(c));
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [last, a, b, c],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
                last = c;
            }
            PathEl::ClosePath => {
                if let Some(s) = start {
                    painter.line_segment([last, s], stroke);
                    last = s;
                }
            }
        }
    }
}

impl Viewport {
    pub(super) fn pen_tool(&mut self, app: &mut XenifyApp, rect: Rect, resp: &egui::Response, pos: Pos2, comp: Point) {
        let enter = resp.ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        // Closing target: the first point, or the far end of a path being continued.
        let near_first = |vp: &Self, p: Pos2| match vp.pen_continue {
            Some(c) => c.other != c.start && (vp.to_screen(rect, c.other) - p).length() < HANDLE * 1.5,
            None => vp.pen.len() >= 3 && (vp.to_screen(rect, vp.pen[0].pos) - p).length() < HANDLE * 1.5,
        };

        // Starting on a loose end of the selected path continues that path.
        let starting = resp.clicked() || resp.drag_started_by(egui::PointerButton::Primary);
        if starting && self.pen.is_empty() && self.pen_continue.is_none() {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            if let Some(c) = self.open_end_at(app, rect, origin) {
                self.pen_continue = Some(c);
                app.status =
                    "Continuing the path — click to add points, click its other end to close, Enter to finish.".into();
                return;
            }
        }

        // Double-click finishes an open path: its first click added the last
        // point, and the second lands on it. (egui reports the second as a
        // click too, and after a recent click as a triple.)
        let on_last = self.pen.last().is_some_and(|p| (self.to_screen(rect, p.pos) - pos).length() < HANDLE * 1.5);
        if (resp.double_clicked() || resp.triple_clicked()) && on_last {
            self.finish_pen(app, false);
            return;
        }
        // Click: corner point (or close the path on the first point).
        if resp.clicked() {
            if near_first(self, pos) {
                self.finish_pen(app, true);
            } else {
                self.pen.push(PenPoint { pos: comp, handle: kurbo::Vec2::ZERO });
            }
            return;
        }
        // Drag: smooth point, dragging out its handle.
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            self.pen_drag = Some((self.to_comp(rect, origin), near_first(self, origin)));
        }
        if let Some((anchor, closes)) = self.pen_drag
            && resp.drag_stopped()
        {
            self.pen_drag = None;
            let handle = comp - anchor;
            if closes {
                // Dragging on the first point closes with a curve through it.
                self.pen[0].handle = handle;
                self.finish_pen(app, true);
            } else {
                self.pen.push(PenPoint { pos: anchor, handle });
            }
            return;
        }
        if enter {
            self.finish_pen(app, false);
        }
    }

    /// Esc with the pen: keeps what was drawn as an open path (nothing is
    /// thrown away). Returns whether a path was in progress.
    pub fn end_pen(&mut self, app: &mut XenifyApp) -> bool {
        if self.pen.is_empty() && self.pen_continue.is_none() {
            return false;
        }
        self.finish_pen(app, false);
        true
    }

    /// Whether a path is being drawn with the pen.
    pub fn pen_in_progress(&self) -> bool {
        !self.pen.is_empty() || self.pen_continue.is_some()
    }

    /// Backspace / Delete with the pen: takes back the last point.
    pub fn pen_remove_last(&mut self) -> bool {
        if self.pen.pop().is_some() {
            return true;
        }
        // Nothing drawn yet while continuing a path: stop continuing it.
        self.pen_continue.take().is_some()
    }

    /// A loose end of the selected layer's open paths under screen point `p`.
    fn open_end_at(&self, app: &XenifyApp, rect: Rect, p: Pos2) -> Option<PenContinue> {
        let (layer, world, paths) = Self::editable_paths(app)?;
        for (shape, path) in &paths {
            let ends = bezier::open_ends(path);
            for &(subpath, from_start, q) in &ends {
                if (self.to_screen(rect, world * q) - p).length() < HANDLE * 1.5 {
                    let other = ends.iter().find(|e| e.0 == subpath && e.1 != from_start).map_or(q, |e| e.2);
                    return Some(PenContinue {
                        layer,
                        shape: *shape,
                        subpath,
                        from_start,
                        start: world * q,
                        other: world * other,
                        to_layer: world.inverse(),
                    });
                }
            }
        }
        None
    }

    /// Loose ends of the selected path, drawn while the pen is ready (so you
    /// can see where a path can be continued).
    pub(super) fn draw_open_ends(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        if !self.pen.is_empty() || self.pen_continue.is_some() {
            return;
        }
        let Some((_, world, paths)) = Self::editable_paths(app) else { return };
        for (_, path) in &paths {
            for (_, _, q) in bezier::open_ends(path) {
                painter.circle(
                    self.to_screen(rect, world * q),
                    HANDLE * 0.8,
                    Color32::from_black_alpha(160),
                    Stroke::new(1.5, ACCENT),
                );
            }
        }
    }

    pub(super) fn finish_pen(&mut self, app: &mut XenifyApp, closed: bool) {
        let points = std::mem::take(&mut self.pen);
        self.pen_drag = None;
        if let Some(c) = self.pen_continue.take() {
            if points.is_empty() && !closed {
                return;
            }
            // Into the layer's space (handles are directions: linear part only).
            let lin = Affine::new({
                let m = c.to_layer.as_coeffs();
                [m[0], m[1], m[2], m[3], 0.0, 0.0]
            });
            let local: Vec<PenPoint> = points
                .iter()
                .map(|p| PenPoint { pos: c.to_layer * p.pos, handle: (lin * p.handle.to_point()).to_vec2() })
                .collect();
            let path_prop = PropPath::shape(c.shape, ShapeField::Path);
            let Ok(Value::Path(path)) = app.project().layer(c.layer).and_then(|l| l.value_at(path_prop, app.time))
            else {
                return;
            };
            if let Some(new) = bezier::extend_subpath(&path, c.subpath, c.from_start, &local, closed) {
                app.new_gesture();
                app.edit_value(c.layer, path_prop, Value::Path(new));
                app.status = if closed { "Closed the path.".into() } else { "Continued the path.".into() };
            }
            return;
        }
        if points.len() < 2 {
            return;
        }
        let comp_path = bezier::pen_path(&points, closed);
        // Put the layer's origin at the path's centre so it rotates nicely.
        let c = kurbo::Shape::bounding_box(&comp_path).center().to_vec2();
        let path = Affine::translate(-c) * comp_path;
        // The colour panel's fill (closed paths only) and stroke; open paths always get a stroke.
        let fill = if closed { app.paint.fill } else { None };
        let stroke = app.paint.stroke.or(if closed && fill.is_some() { None } else { Some(Color::WHITE) });
        let shape = ShapeItem::path(path, fill, stroke.map(|c| (c, app.paint.stroke_width.max(0.5))));
        add_layer(app, "Path", vec![shape], c);
        // Like Illustrator, the pen stays active (so the new path can be continued).
        app.tool = crate::app::Tool::Pen;
    }

    pub(super) fn draw_pen(&self, painter: &egui::Painter, rect: Rect) {
        if self.pen.is_empty() && self.pen_drag.is_none() && self.pen_continue.is_none() {
            return;
        }
        let stroke = Stroke::new(1.5, ACCENT);
        let hover = painter.ctx().pointer_hover_pos();
        let mut points = self.pen.clone();
        // A continued path grows from its loose end.
        if let Some(c) = self.pen_continue {
            points.insert(0, PenPoint { pos: c.start, handle: kurbo::Vec2::ZERO });
            painter.circle(
                self.to_screen(rect, c.other),
                HANDLE * 0.8,
                Color32::from_black_alpha(160),
                Stroke::new(1.5, ACCENT),
            );
        }
        // Show the point being dragged out, or a rubber band to the pointer.
        if let (Some((anchor, _)), Some(h)) = (self.pen_drag, hover) {
            let handle = self.to_comp(rect, h) - anchor;
            points.push(PenPoint { pos: anchor, handle });
            let a = self.to_screen(rect, anchor);
            for end in [anchor + handle, anchor - handle] {
                let e = self.to_screen(rect, end);
                painter.line_segment([a, e], Stroke::new(1.0, ACCENT.gamma_multiply(0.7)));
                painter.circle(e, 3.0, Color32::WHITE, Stroke::new(1.0, ACCENT));
            }
        } else if let Some(h) = hover {
            points.push(PenPoint { pos: self.to_comp(rect, h), handle: kurbo::Vec2::ZERO });
        }
        draw_bezpath(painter, &bezier::pen_path(&points, false), |p| self.to_screen(rect, p), stroke);
        for (i, p) in self.pen.iter().enumerate() {
            let s = self.to_screen(rect, p.pos);
            let fill = if i == 0 { ACCENT } else { Color32::WHITE };
            painter.rect(Rect::from_center_size(s, EVec2::splat(HANDLE)), 1.0, fill, stroke, StrokeKind::Middle);
        }
    }
}

/// Replaces every rectangle/ellipse in the layer with an equivalent editable
/// path (as it looks at `time`). Returns `None` if there is nothing to convert.
pub fn convert_to_path_command(app: &XenifyApp, layer: Id) -> Option<xenify_ops::Command> {
    use xenify_ops::Command;
    let l = app.project().layer(layer).ok()?;
    let mut cmds = Vec::new();
    for (index, shape) in l.shapes().iter().enumerate() {
        if matches!(shape.geometry, Geometry::Path { .. }) {
            continue;
        }
        // Keeps its animation: a path keyframe wherever it had one.
        let new = ShapeItem { geometry: xenify_doc::primitives::to_path_geometry(&shape.geometry), ..shape.clone() };
        cmds.push(Command::SetShape { layer, index, shape: new });
    }
    (!cmds.is_empty()).then(|| Command::batch(cmds))
}
