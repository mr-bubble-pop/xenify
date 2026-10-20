//! Mask tool (U): draw rectangle or ellipse masks on the selected layer, and
//! reshape them: drag a rectangle's corners and edges like a crop, other
//! masks' points, or the inside to move a mask. Changes key the mask path
//! when it's animated or auto-key is on.

use super::{HANDLE, Viewport};
use crate::app::XenifyApp;
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2 as EVec2};
use xenify_doc::{Id, Mask, MaskField, MaskMode, PropPath, Value, as_rect, rect_path};
use kurbo::{Affine, BezPath, PathEl, Point, Shape};

/// The outline colour of masks on the canvas (After Effects yellow).
pub fn mask_colour() -> Color32 {
    Color32::from_rgb(255, 214, 64)
}

/// Which shape a new mask gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskShape {
    #[default]
    Rectangle,
    Ellipse,
}

#[derive(Clone)]
enum MaskDrag {
    /// Drawing a new mask between two layer-space points.
    New { from: Point, to: Point },
    /// Resizing a rectangle mask: which sides move.
    Sides { mask: usize, rect: kurbo::Rect, l: bool, t: bool, r: bool, b: bool, start: Point },
    /// Moving one point (element index) of a mask's path.
    Point { mask: usize, path: BezPath, el: usize, start: Point },
    /// Moving the whole mask.
    Body { mask: usize, path: BezPath, start: Point },
}

#[derive(Clone, Default)]
pub(super) struct MaskState {
    drag: Option<MaskDrag>,
    /// The mask last drawn or touched (tool options edit it).
    pub active: Option<usize>,
    pub shape: MaskShape,
    pub new_mode: MaskMode,
}

/// The layer the mask tool works on and its layer → composition transform.
fn target(app: &XenifyApp) -> Option<(Id, Affine)> {
    let id = app.selected?;
    Some((id, xenify_anim::world_transform(app.comp(), id, app.time)))
}

/// A path's on-curve points: (element index, point).
fn points(path: &BezPath) -> Vec<(usize, Point)> {
    path.elements()
        .iter()
        .enumerate()
        .filter_map(|(i, el)| match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some((i, p)),
            PathEl::ClosePath => None,
        })
        .collect()
}

/// `path` with the point of element `el` moved by `d`, taking its curve
/// handles along.
fn move_point(path: &BezPath, el: usize, d: kurbo::Vec2) -> BezPath {
    let mut els: Vec<PathEl> = path.elements().to_vec();
    let n = els.len();
    match &mut els[el] {
        PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) => *p += d,
        PathEl::CurveTo(_, c2, p) => {
            *p += d;
            *c2 += d;
        }
        PathEl::ClosePath => {}
    }
    // The handle leaving this point.
    if let Some(PathEl::CurveTo(c1, _, _)) = els.get_mut(el + 1) {
        *c1 += d;
    }
    // A closed path's start point is also where its last curve ends.
    let start = path.elements()[el].end_point().unwrap_or_default();
    if matches!(els[el], PathEl::MoveTo(_))
        && matches!(els.last(), Some(PathEl::ClosePath))
        && n >= 2
        && let PathEl::CurveTo(_, c2, p) = &mut els[n - 2]
        && p.distance(start) < 1e-9
    {
        *p += d;
        *c2 += d;
    }
    BezPath::from_vec(els)
}

impl Viewport {
    /// Makes mask `i` of the selected layer the one the tool options edit.
    pub fn set_active_mask(&mut self, i: Option<usize>) {
        self.mask.active = i;
    }

    pub(super) fn mask_tool(&mut self, app: &mut XenifyApp, rect: Rect, resp: &egui::Response, pos: Pos2) {
        let Some((id, to_comp)) = target(app) else {
            if resp.clicked() || resp.drag_started() {
                app.status = "Select the layer to mask first (or use the Select tool).".into();
            }
            return;
        };
        let to_local = to_comp.inverse();
        let Ok(layer) = app.project().layer(id).cloned() else { return };
        let local_of = |vp: &Viewport, p: Pos2| to_local * vp.to_comp(rect, p);

        if resp.drag_started_by(egui::PointerButton::Primary) {
            let press = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let start = local_of(self, press);
            self.mask.drag = Some(
                self.mask_grip(app, &layer, to_comp, rect, press).unwrap_or(MaskDrag::New { from: start, to: start }),
            );
            app.new_gesture();
        }
        if !resp.dragged() && resp.hovered() {
            let cursor = match self.mask_grip(app, &layer, to_comp, rect, pos) {
                Some(MaskDrag::Body { .. }) => egui::CursorIcon::Move,
                Some(_) => egui::CursorIcon::Grab,
                None => egui::CursorIcon::Crosshair,
            };
            resp.ctx.set_cursor_icon(cursor);
        }
        let Some(drag) = self.mask.drag.clone() else { return };
        let now = local_of(self, pos);
        let shift = resp.ctx.input(|i| i.modifiers.shift);
        let set_path = |app: &mut XenifyApp, mask: usize, path: BezPath| {
            app.edit_value(id, PropPath::Mask { index: mask, field: MaskField::Path }, Value::Path(path));
        };
        match drag {
            MaskDrag::New { from, .. } => {
                let mut to = now;
                if shift {
                    let d = to - from;
                    let s = d.x.abs().max(d.y.abs());
                    to = from + kurbo::Vec2::new(s.copysign(d.x), s.copysign(d.y));
                }
                self.mask.drag = Some(MaskDrag::New { from, to });
                if resp.drag_stopped() {
                    self.mask.drag = None;
                    let r = kurbo::Rect::from_points(from, to);
                    if r.width() * to_comp.determinant().abs().sqrt() < 3.0 || r.height() < 1e-9 {
                        return;
                    }
                    let n = layer.meta.masks.len() + 1;
                    let mut m = match self.mask.shape {
                        MaskShape::Rectangle => Mask::rect(format!("Mask {n}"), r),
                        MaskShape::Ellipse => Mask::ellipse(format!("Mask {n}"), r),
                    };
                    m.mode = self.mask.new_mode;
                    let mut meta = layer.meta.clone();
                    meta.masks.push(m);
                    if app.exec("Add mask", xenify_ops::Command::SetLayerMeta { layer: id, meta }) {
                        self.mask.active = Some(n - 1);
                        app.status = "Added a mask. Drag its corners or points to reshape it; Properties › Masks \
                                      has feather, invert and mode."
                            .into();
                    }
                }
            }
            MaskDrag::Sides { mask, rect: r0, l, t, r, b, start } => {
                let d = now - start;
                let mut nr = r0;
                if l {
                    nr.x0 += d.x;
                }
                if r {
                    nr.x1 += d.x;
                }
                if t {
                    nr.y0 += d.y;
                }
                if b {
                    nr.y1 += d.y;
                }
                set_path(app, mask, rect_path(nr.abs()));
                self.mask.active = Some(mask);
            }
            MaskDrag::Point { mask, path, el, start } => {
                set_path(app, mask, move_point(&path, el, now - start));
                self.mask.active = Some(mask);
            }
            MaskDrag::Body { mask, path, start } => {
                set_path(app, mask, Affine::translate(now - start) * path);
                self.mask.active = Some(mask);
            }
        }
        if resp.drag_stopped() {
            self.mask.drag = None;
        }
    }

    /// What pressing at screen `pos` would grab on the layer's masks.
    fn mask_grip(
        &self,
        app: &XenifyApp,
        layer: &xenify_doc::Layer,
        to_comp: Affine,
        rect: Rect,
        pos: Pos2,
    ) -> Option<MaskDrag> {
        let t = app.time;
        let screen = |p: Point| self.to_screen(rect, to_comp * p);
        let near = |p: Point| screen(p).distance(pos) <= HANDLE + 2.0;
        let local = to_comp.inverse() * self.to_comp(rect, pos);
        // Handles first (the active mask's win), then insides (topmost first).
        let mut order: Vec<usize> = (0..layer.meta.masks.len()).rev().collect();
        if let Some(a) = self.mask.active.filter(|a| *a < layer.meta.masks.len()) {
            order.retain(|i| *i != a);
            order.insert(0, a);
        }
        for &i in &order {
            let path = layer.meta.masks[i].path.value_at(t);
            if let Some(r) = as_rect(&path) {
                let (mx, my) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
                let grips = [
                    (Point::new(r.x0, r.y0), (true, true, false, false)),
                    (Point::new(mx, r.y0), (false, true, false, false)),
                    (Point::new(r.x1, r.y0), (false, true, true, false)),
                    (Point::new(r.x1, my), (false, false, true, false)),
                    (Point::new(r.x1, r.y1), (false, false, true, true)),
                    (Point::new(mx, r.y1), (false, false, false, true)),
                    (Point::new(r.x0, r.y1), (true, false, false, true)),
                    (Point::new(r.x0, my), (true, false, false, false)),
                ];
                if let Some((_, (l, tp, rr, b))) = grips.iter().find(|(p, _)| near(*p)) {
                    return Some(MaskDrag::Sides { mask: i, rect: r, l: *l, t: *tp, r: *rr, b: *b, start: local });
                }
            } else if let Some((el, _)) = points(&path).into_iter().find(|(_, p)| near(*p)) {
                return Some(MaskDrag::Point { mask: i, path, el, start: local });
            }
        }
        for &i in &order {
            let path = layer.meta.masks[i].path.value_at(t);
            if path.winding(local) != 0 {
                return Some(MaskDrag::Body { mask: i, path, start: local });
            }
        }
        None
    }

    /// Mask outlines and handles on the selected layer.
    pub(super) fn draw_masks(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let Some((id, to_comp)) = target(app) else { return };
        let Ok(layer) = app.project().layer(id) else { return };
        let screen = |p: Point| self.to_screen(rect, to_comp * p);
        let outline = |path: &BezPath| -> Vec<Vec<Pos2>> {
            let mut lines: Vec<Vec<Pos2>> = Vec::new();
            kurbo::flatten(to_comp * path.clone(), 0.25 / self.zoom.max(1e-3), |el| match el {
                PathEl::MoveTo(p) => lines.push(vec![self.to_screen(rect, p)]),
                PathEl::LineTo(p) => {
                    if let Some(l) = lines.last_mut() {
                        l.push(self.to_screen(rect, p));
                    }
                }
                PathEl::ClosePath => {
                    if let Some(l) = lines.last_mut()
                        && let Some(f) = l.first().copied()
                    {
                        l.push(f);
                    }
                }
                _ => {}
            });
            lines
        };
        for (i, m) in layer.meta.masks.iter().enumerate() {
            let active = self.mask.active == Some(i);
            let colour = if m.enabled { mask_colour() } else { mask_colour().gamma_multiply(0.4) };
            let path = m.path.value_at(app.time);
            for line in outline(&path) {
                painter.add(egui::Shape::line(line, Stroke::new(if active { 2.0 } else { 1.2 }, colour)));
            }
            let grips: Vec<Point> = match as_rect(&path) {
                Some(r) => {
                    let (mx, my) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
                    vec![
                        Point::new(r.x0, r.y0),
                        Point::new(mx, r.y0),
                        Point::new(r.x1, r.y0),
                        Point::new(r.x1, my),
                        Point::new(r.x1, r.y1),
                        Point::new(mx, r.y1),
                        Point::new(r.x0, r.y1),
                        Point::new(r.x0, my),
                    ]
                }
                None => points(&path).into_iter().map(|(_, p)| p).collect(),
            };
            for p in grips {
                let fill = if active { colour } else { Color32::from_gray(30) };
                painter.rect(
                    Rect::from_center_size(screen(p), EVec2::splat(HANDLE - 1.0)),
                    0.0,
                    fill,
                    Stroke::new(1.0, colour),
                    StrokeKind::Middle,
                );
            }
        }
        if let Some(MaskDrag::New { from, to }) = &self.mask.drag {
            let r = kurbo::Rect::from_points(*from, *to);
            let path = match self.mask.shape {
                MaskShape::Rectangle => rect_path(r),
                MaskShape::Ellipse => kurbo::Ellipse::from_rect(r).to_path(0.1),
            };
            for line in outline(&path) {
                painter.add(egui::Shape::line(line, Stroke::new(1.5, mask_colour())));
            }
        }
    }
}

/// Mask tool settings in the control bar.
pub fn mask_options(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let mut vp = std::mem::take(&mut app.viewport);
    ui.label("Shape");
    ui.selectable_value(&mut vp.mask.shape, MaskShape::Rectangle, "□ Rectangle");
    ui.selectable_value(&mut vp.mask.shape, MaskShape::Ellipse, "◯ Ellipse");
    ui.label("New masks");
    egui::ComboBox::from_id_salt("mask-new-mode").selected_text(vp.mask.new_mode.label()).width(80.0).show_ui(
        ui,
        |ui| {
            for m in MaskMode::ALL {
                ui.selectable_value(&mut vp.mask.new_mode, m, m.label());
            }
        },
    );
    ui.separator();
    match app.selected.and_then(|id| app.project().layer(id).ok()).cloned() {
        None => {
            ui.weak("Select a layer, then drag on it to draw a mask");
        }
        Some(layer) if layer.meta.masks.is_empty() => {
            ui.weak("Drag on the canvas to draw a mask on the selected layer");
        }
        Some(layer) => {
            let n = layer.meta.masks.len();
            let i = vp.mask.active.filter(|i| *i < n).unwrap_or(n - 1);
            let mut meta = layer.meta.clone();
            let m = &mut meta.masks[i];
            ui.label(format!("{}:", if m.name.is_empty() { format!("Mask {}", i + 1) } else { m.name.clone() }));
            egui::ComboBox::from_id_salt("mask-mode").selected_text(m.mode.label()).width(80.0).show_ui(ui, |ui| {
                for mode in MaskMode::ALL {
                    ui.selectable_value(&mut m.mode, mode, mode.label());
                }
            });
            ui.checkbox(&mut m.inverted, "Invert");
            let mut delete = false;
            if ui.button("Delete mask").clicked() {
                delete = true;
            }
            if delete {
                meta.masks.remove(i);
                vp.mask.active = None;
            }
            if meta != layer.meta {
                app.exec(
                    if delete { "Delete mask" } else { "Mask settings" },
                    xenify_ops::Command::SetLayerMeta { layer: layer.id, meta },
                );
            }
        }
    }
    app.viewport = vp;
}
