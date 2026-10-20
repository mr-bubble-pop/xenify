//! Text tool (T): click for point text, drag out a text box; drag a box's
//! edges or corners to resize it (the text re-wraps).

use super::{HANDLE, Viewport, add_text_layer};
use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke};
use xenify_anim::Scene;
use xenify_doc::{Id, LayerKind, TextFrame};
use xenify_ops::Command;
use kurbo::{Affine, Point};

/// Which sides of a box a grip moves: (left, top, right, bottom).
type Sides = (bool, bool, bool, bool);

#[derive(Clone)]
pub(super) struct TextDrag {
    /// The text layer whose box is resized; `None` while drawing a new box.
    layer: Option<Id>,
    sides: Sides,
    /// The box at the start (layer space; composition space for a new box).
    rect: kurbo::Rect,
    /// Where the drag began, in the same space.
    start: Point,
    /// Composition → that space.
    inv: Affine,
    /// The box now.
    now: kurbo::Rect,
}

/// The eight grips of a box: corners and edge middles.
fn grips(r: kurbo::Rect) -> [(Point, Sides); 8] {
    let (cx, cy) = (r.center().x, r.center().y);
    [
        (Point::new(r.x0, r.y0), (true, true, false, false)),
        (Point::new(cx, r.y0), (false, true, false, false)),
        (Point::new(r.x1, r.y0), (false, true, true, false)),
        (Point::new(r.x1, cy), (false, false, true, false)),
        (Point::new(r.x1, r.y1), (false, false, true, true)),
        (Point::new(cx, r.y1), (false, false, false, true)),
        (Point::new(r.x0, r.y1), (true, false, false, true)),
        (Point::new(r.x0, cy), (true, false, false, false)),
    ]
}

fn cursor_for(sides: Sides) -> egui::CursorIcon {
    use egui::CursorIcon::*;
    match sides {
        (true, true, _, _) | (_, _, true, true) => ResizeNwSe,
        (_, true, true, _) | (true, _, _, true) => ResizeNeSw,
        (true, ..) | (_, _, true, _) => ResizeHorizontal,
        _ => ResizeVertical,
    }
}

impl Viewport {
    /// The selected text layer's box and its layer → composition transform.
    pub(super) fn selected_text_box(&self, app: &XenifyApp, scene: &Scene) -> Option<(Id, TextFrame, Affine)> {
        let id = app.selected.filter(|_| app.selected_layers().len() == 1)?;
        let LayerKind::Text(t) = &app.project().layer(id).ok()?.kind else { return None };
        let frame = t.frame.filter(|_| t.path.is_none())?;
        Some((id, frame, scene.layer(id)?.transform))
    }

    /// The grip of the selected text box under `pos` (screen).
    fn text_grip_at(
        &self,
        app: &XenifyApp,
        rect: Rect,
        scene: &Scene,
        pos: Pos2,
    ) -> Option<(Id, TextFrame, Affine, Sides)> {
        let (id, frame, t) = self.selected_text_box(app, scene)?;
        grips(frame.rect)
            .into_iter()
            .find(|(p, _)| self.to_screen(rect, t * *p).distance(pos) <= HANDLE + 2.0)
            .map(|(_, sides)| (id, frame, t, sides))
    }

    pub(super) fn text_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp_pos: Point,
        scene: &Scene,
    ) {
        if self.text_drag.is_none() && resp.hovered() {
            let icon = self.text_grip_at(app, rect, scene, pos).map_or(egui::CursorIcon::Text, |g| cursor_for(g.3));
            resp.ctx.set_cursor_icon(icon);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let start = self.to_comp(rect, pos);
            self.text_drag = Some(match self.text_grip_at(app, rect, scene, pos) {
                Some((id, frame, t, sides)) => {
                    app.new_gesture();
                    let inv = t.inverse();
                    TextDrag { layer: Some(id), sides, rect: frame.rect, start: inv * start, inv, now: frame.rect }
                }
                None => {
                    let r = kurbo::Rect::from_points(start, start);
                    TextDrag {
                        layer: None,
                        sides: (false, false, true, true),
                        rect: r,
                        start,
                        inv: Affine::IDENTITY,
                        now: r,
                    }
                }
            });
        } else if resp.clicked() {
            // Clicking text edits it; clicking elsewhere starts point text.
            let hit = xenify_anim::hit_test(scene, comp_pos, 4.0 / self.zoom, &mut *self.text.borrow_mut());
            match hit.filter(|id| app.project().layer(*id).is_ok_and(|l| matches!(l.kind, LayerKind::Text(_)))) {
                Some(id) => super::in_place::start(app, id),
                None => add_text_layer(app, comp_pos.to_vec2()),
            }
        }
        let Some(mut d) = self.text_drag.clone() else { return };
        if resp.dragged() {
            let p = d.inv * comp_pos;
            let (dx, dy) = (p.x - d.start.x, p.y - d.start.y);
            let mut r = d.rect;
            if d.layer.is_none() {
                r = kurbo::Rect::from_points(d.start, p);
            } else {
                let (l, t, rt, b) = d.sides;
                if l {
                    r.x0 += dx;
                }
                if rt {
                    r.x1 += dx;
                }
                if t {
                    r.y0 += dy;
                }
                if b {
                    r.y1 += dy;
                }
                r = r.abs();
                r.x1 = r.x1.max(r.x0 + 4.0);
                r.y1 = r.y1.max(r.y0 + 4.0);
                if let Some(id) = d.layer
                    && let Ok(LayerKind::Text(t)) = app.project().layer(id).map(|l| &l.kind)
                    && let Some(f) = t.frame
                {
                    app.exec_merged(
                        "Resize text box",
                        Command::SetTextFrame { layer: id, frame: Some(TextFrame { rect: r, ..f }) },
                    );
                }
            }
            d.now = r;
            self.text_drag = Some(d.clone());
        }
        if resp.drag_stopped() {
            self.text_drag = None;
            if d.layer.is_none() {
                let r = d.now;
                // A tiny drag is a click.
                if r.width() * self.zoom < 6.0 || r.height() * self.zoom < 6.0 {
                    add_text_layer(app, d.start.to_vec2());
                } else {
                    add_text_box(app, r);
                }
            }
        }
    }

    /// The selected text layer's box (dashed; with grips in the Text tool)
    /// and a box being drawn.
    pub(super) fn draw_text_box(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect, scene: &Scene) {
        let outline = |corners: [Point; 4], t: Affine| {
            let pts: Vec<Pos2> = corners.iter().map(|c| self.to_screen(rect, t * *c)).collect();
            for i in 0..4 {
                painter.extend(egui::Shape::dashed_line(
                    &[pts[i], pts[(i + 1) % 4]],
                    Stroke::new(1.0, ACCENT),
                    5.0,
                    3.0,
                ));
            }
        };
        let corners = |r: kurbo::Rect| {
            [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
        };
        if let Some(d) = self.text_drag.as_ref().filter(|d| d.layer.is_none()) {
            outline(corners(d.now), Affine::IDENTITY);
            return;
        }
        if !matches!(app.tool, Tool::Text | Tool::Select) {
            return;
        }
        let Some((_, frame, t)) = self.selected_text_box(app, scene) else { return };
        outline(corners(frame.rect), t);
        if app.tool == Tool::Text {
            for (p, _) in grips(frame.rect) {
                let c = self.to_screen(rect, t * p);
                let r = Rect::from_center_size(c, egui::vec2(HANDLE, HANDLE));
                painter.rect_filled(r, 1.0, Color32::WHITE);
                painter.rect_stroke(r, 1.0, Stroke::new(1.0, ACCENT), egui::StrokeKind::Middle);
            }
        }
    }
}

/// A new text layer wrapping inside `r` (composition space), ready to type into.
pub fn add_text_box(app: &mut XenifyApp, r: kurbo::Rect) {
    super::add_text(app, r.origin().to_vec2(), Some(r.size()));
}
