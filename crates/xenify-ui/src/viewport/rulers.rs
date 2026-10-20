//! Rulers along the canvas edges, guides dragged out of them, and the grid.

use super::Viewport;
use crate::app::{XenifyApp, Tool};
use egui::{Color32, CursorIcon, Painter, Pos2, Rect, Sense, Stroke, Vec2};
use xenify_doc::GuideLine;
use kurbo::Point;

/// Ruler thickness (screen points).
pub const RULER: f32 = 18.0;
/// How close (screen points) the pointer must be to grab a guide.
const GRAB: f32 = 4.0;

pub fn guide_colour() -> Color32 {
    Color32::from_rgb(40, 200, 255)
}

/// A guide being dragged: `index` is `None` for a new one from a ruler.
#[derive(Clone, Copy, Debug)]
pub struct GuideDrag {
    index: Option<usize>,
    vertical: bool,
}

impl Viewport {
    /// The grid and guides, drawn over the picture.
    pub(super) fn draw_grid_and_guides(&self, app: &XenifyApp, painter: &Painter, rect: Rect) {
        let s = &app.comp().settings;
        if app.prefs.show_grid {
            let step = s.grid_spacing();
            if step * self.zoom >= 6.0 {
                let stroke = Stroke::new(1.0, Color32::from_white_alpha(22));
                let (w, h) = (s.width as f64, s.height as f64);
                let top = self.to_screen(rect, Point::new(0.0, 0.0));
                let bottom = self.to_screen(rect, Point::new(w, h));
                let mut x = step;
                while x < w {
                    let sx = self.to_screen(rect, Point::new(x, 0.0)).x;
                    painter.vline(sx, top.y..=bottom.y, stroke);
                    x += step;
                }
                let mut y = step;
                while y < h {
                    let sy = self.to_screen(rect, Point::new(0.0, y)).y;
                    painter.hline(top.x..=bottom.x, sy, stroke);
                    y += step;
                }
            }
        }
        if app.prefs.show_guides {
            let stroke = Stroke::new(1.0, guide_colour());
            for (i, g) in s.guides.iter().enumerate() {
                // The one being dragged is drawn where the pointer is.
                if self.guide_drag.is_some_and(|d| d.index == Some(i)) {
                    continue;
                }
                self.draw_guide(painter, rect, g, stroke);
            }
        }
    }

    fn draw_guide(&self, painter: &Painter, rect: Rect, g: &GuideLine, stroke: Stroke) {
        if g.vertical {
            let x = self.to_screen(rect, Point::new(g.pos, 0.0)).x;
            painter.vline(x, rect.y_range(), stroke);
        } else {
            let y = self.to_screen(rect, Point::new(0.0, g.pos)).y;
            painter.hline(rect.x_range(), y, stroke);
        }
    }

    /// The guide under screen point `p`, if any.
    fn guide_at(&self, app: &XenifyApp, rect: Rect, p: Pos2) -> Option<(usize, bool)> {
        if !app.prefs.show_guides {
            return None;
        }
        app.comp().settings.guides.iter().enumerate().find_map(|(i, g)| {
            let near = if g.vertical {
                (self.to_screen(rect, Point::new(g.pos, 0.0)).x - p.x).abs() <= GRAB
            } else {
                (self.to_screen(rect, Point::new(0.0, g.pos)).y - p.y).abs() <= GRAB
            };
            near.then_some((i, g.vertical))
        })
    }

    /// Rulers, and dragging guides out of them or around. Returns true while
    /// a guide is being dragged (the tools then leave the pointer alone).
    pub(super) fn rulers(&mut self, app: &mut XenifyApp, ui: &egui::Ui, rect: Rect, canvas: &egui::Response) -> bool {
        let show = app.prefs.show_rulers;
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), RULER));
        let left = Rect::from_min_size(rect.min, Vec2::new(RULER, rect.height()));
        let pointer = ui.input(|i| i.pointer.latest_pos());

        // Start: from a ruler (new guide) or on an existing guide.
        if show {
            for (strip, vertical, id) in [(top, false, "ruler-top"), (left, true, "ruler-left")] {
                let r = ui.interact(strip, ui.id().with(id), Sense::drag());
                let name = if vertical {
                    "Left ruler: drag right to add a guide"
                } else {
                    "Top ruler: drag down to add a guide"
                };
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, name));
                if r.hovered() {
                    ui.ctx().set_cursor_icon(if vertical {
                        CursorIcon::ResizeHorizontal
                    } else {
                        CursorIcon::ResizeVertical
                    });
                }
                if r.drag_started() {
                    self.guide_drag = Some(GuideDrag { index: None, vertical });
                    app.prefs.show_guides = true;
                }
            }
        }
        let can_grab = matches!(app.tool, Tool::Select | Tool::Node | Tool::Hand);
        if self.guide_drag.is_none() && can_grab {
            if let Some(p) = canvas.hover_pos()
                && let Some((_, vertical)) = self.guide_at(app, rect, p)
            {
                ui.ctx().set_cursor_icon(if vertical {
                    CursorIcon::ResizeHorizontal
                } else {
                    CursorIcon::ResizeVertical
                });
            }
            // A drag starts once the pointer has moved: check where it was pressed.
            if canvas.drag_started()
                && let Some(p) = ui.input(|i| i.pointer.press_origin())
                && let Some((i, vertical)) = self.guide_at(app, rect, p)
            {
                self.guide_drag = Some(GuideDrag { index: Some(i), vertical });
            }
        }

        // Dragging: show it; on release, place it (or remove it when dropped on a ruler / outside).
        if let Some(d) = self.guide_drag {
            let Some(p) = pointer else { return true };
            let c = self.to_comp(rect, p);
            let pos = self.round_guide(app, if d.vertical { c.x } else { c.y });
            let stroke = Stroke::new(1.0, guide_colour());
            let painter = ui.painter_at(rect);
            self.draw_guide(&painter, rect, &GuideLine { vertical: d.vertical, pos }, stroke);
            let units = app.comp().settings.units;
            painter.text(
                p + Vec2::new(12.0, 12.0),
                egui::Align2::LEFT_TOP,
                units.format(pos),
                egui::FontId::proportional(12.0),
                guide_colour(),
            );
            if !ui.input(|i| i.pointer.primary_down()) {
                self.guide_drag = None;
                let over_ruler = show && (top.contains(p) || left.contains(p));
                let mut settings = app.comp().settings.clone();
                let keep = rect.contains(p) && !over_ruler;
                match (d.index, keep) {
                    (Some(i), true) => settings.guides[i].pos = pos,
                    (Some(i), false) => {
                        settings.guides.remove(i);
                    }
                    (None, true) => settings.guides.push(GuideLine { vertical: d.vertical, pos }),
                    (None, false) => return true,
                }
                let label = if keep { "Move guide" } else { "Remove guide" };
                app.exec(label, xenify_ops::Command::SetCompSettings { comp: app.comp, settings });
            }
            return true;
        }
        false
    }

    /// Guide positions land on whole pixels (or the unit's precision).
    fn round_guide(&self, app: &XenifyApp, v: f64) -> f64 {
        let units = app.comp().settings.units;
        let step = if units.unit == xenify_doc::Unit::Px {
            1.0
        } else {
            units.to_px(10f64.powi(-(units.decimals() as i32 - 1)))
        };
        (v / step).round() * step
    }

    /// The rulers, drawn last (over everything else on the canvas).
    pub(super) fn draw_rulers(&self, app: &XenifyApp, painter: &Painter, rect: Rect, pointer: Option<Pos2>) {
        if !app.prefs.show_rulers {
            return;
        }
        let units = app.comp().settings.units;
        let bg = Color32::from_rgb(30, 30, 34);
        let ink = Color32::from_gray(150);
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), RULER));
        let left = Rect::from_min_size(rect.min, Vec2::new(RULER, rect.height()));
        painter.rect_filled(top, 0.0, bg);
        painter.rect_filled(left, 0.0, bg);
        // A label step of 1, 2 or 5 × 10ⁿ units, at least ~60 points apart.
        let px_per_unit = units.px_per_unit() * self.zoom;
        let raw = 60.0 / px_per_unit.max(1e-9);
        let pow = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * pow).find(|s| *s >= raw).unwrap_or(10.0 * pow);
        let minor = step / if (step / pow).round() as i64 == 2 { 4.0 } else { 5.0 };
        let font = egui::FontId::proportional(9.5);
        let label = |v: f64| {
            let d = if step >= 1.0 { 0 } else { (-step.log10().floor()) as usize };
            format!("{v:.d$}")
        };
        // Horizontal.
        let origin = self.to_screen(rect, Point::ZERO);
        let first = ((rect.left() - origin.x) as f64 / px_per_unit / minor).floor() as i64;
        let last = ((rect.right() - origin.x) as f64 / px_per_unit / minor).ceil() as i64;
        for k in first..=last {
            let v = k as f64 * minor;
            let x = origin.x + (v * px_per_unit) as f32;
            if x < rect.left() + RULER {
                continue;
            }
            let major = (v / step).round() * step;
            let is_major = (v - major).abs() < minor * 0.01;
            let len = if is_major { RULER * 0.6 } else { RULER * 0.25 };
            painter.vline(x, (top.bottom() - len)..=top.bottom(), Stroke::new(1.0, ink));
            if is_major {
                painter.text(Pos2::new(x + 2.0, top.top() + 1.0), egui::Align2::LEFT_TOP, label(v), font.clone(), ink);
            }
        }
        // Vertical.
        let first = ((rect.top() - origin.y) as f64 / px_per_unit / minor).floor() as i64;
        let last = ((rect.bottom() - origin.y) as f64 / px_per_unit / minor).ceil() as i64;
        for k in first..=last {
            let v = k as f64 * minor;
            let y = origin.y + (v * px_per_unit) as f32;
            if y < rect.top() + RULER {
                continue;
            }
            let major = (v / step).round() * step;
            let is_major = (v - major).abs() < minor * 0.01;
            let len = if is_major { RULER * 0.6 } else { RULER * 0.25 };
            painter.hline((left.right() - len)..=left.right(), y, Stroke::new(1.0, ink));
            if is_major {
                // Written up the ruler, like on paper.
                let galley = painter.layout_no_wrap(label(v), font.clone(), ink);
                let at = Pos2::new(left.left() + 2.0, y - 2.0);
                painter.add(egui::epaint::TextShape::new(at, galley, ink).with_angle(-std::f32::consts::FRAC_PI_2));
            }
        }
        // Where the pointer is.
        if let Some(p) = pointer.filter(|p| rect.contains(*p)) {
            let mark = Stroke::new(1.0, guide_colour());
            painter.vline(p.x, top.y_range(), mark);
            painter.hline(left.x_range(), p.y, mark);
        }
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::splat(RULER)), 0.0, bg);
        painter.text(
            rect.min + Vec2::splat(RULER / 2.0),
            egui::Align2::CENTER_CENTER,
            units.unit.suffix(),
            egui::FontId::proportional(8.5),
            ink,
        );
    }
}
