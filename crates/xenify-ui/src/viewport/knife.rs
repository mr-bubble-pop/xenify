//! Knife (C): drag a line right across shapes to cut them in two. Cuts the
//! selected shapes, or any shape the line crosses when nothing is selected.

use super::Viewport;
use crate::app::XenifyApp;
use egui::{Color32, Pos2, Rect, Stroke};
use kurbo::Point;

impl Viewport {
    pub(super) fn knife_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        comp: Point,
        shift: bool,
    ) {
        if resp.hovered() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).map_or(comp, |p| self.to_comp(rect, p));
            self.knife = Some((origin, origin));
        }
        if let Some((a, _)) = self.knife {
            // Shift keeps the cut horizontal, vertical or at 45°.
            let mut b = comp;
            if shift {
                let d = b - a;
                let angle = (d.y.atan2(d.x) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
                b = a + kurbo::Vec2::new(angle.cos(), angle.sin()) * d.hypot();
            }
            self.knife = Some((a, b));
        }
        if resp.drag_stopped()
            && let Some((a, b)) = self.knife.take()
        {
            let selected = app.selected_layers();
            let targets = if selected.is_empty() { app.comp().layers.iter().map(|l| l.id).collect() } else { selected };
            match xenify_ops::pathfinder::knife(app.project(), app.comp, &targets, a, b, app.time) {
                Ok((cmd, ids)) => {
                    if app.exec("Knife", cmd) {
                        let pieces = ids.len();
                        app.select_only(ids.first().copied());
                        app.extra_selected = ids.into_iter().skip(1).collect();
                        app.keep_extras = true;
                        app.status = format!("Cut into {pieces} pieces.");
                    }
                }
                Err(e) => app.fail(e.to_string()),
            }
        }
    }

    pub(super) fn draw_knife(&self, painter: &egui::Painter, rect: Rect) {
        let Some((a, b)) = self.knife else { return };
        let (a, b): (Pos2, Pos2) = (self.to_screen(rect, a), self.to_screen(rect, b));
        painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(160)));
        let dash = egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, Color32::from_rgb(255, 90, 90)), 6.0, 4.0);
        painter.extend(dash);
    }
}
