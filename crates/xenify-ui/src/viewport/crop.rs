//! Crop (C): resize the canvas, or clip the selected layers, to a rectangle.
//! Slice (Shift+C): named regions of the canvas that export as separate files.

use super::{HANDLE, Viewport};
use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2 as EVec2};
use xenify_doc::Slice;
use xenify_ops::Command;
use kurbo::Point;

/// Which part of a crop/slice rectangle is being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    /// Moving the whole rectangle.
    Body,
    /// Corners/edges: which sides move (left, top, right, bottom).
    Sides { l: bool, t: bool, r: bool, b: bool },
    /// Drawing a new rectangle from this corner.
    New(Point),
}

#[derive(Clone)]
pub(super) struct RectDrag {
    grip: Grip,
    start: Point,
    rect: kurbo::Rect,
}

/// Crop tool state: the rectangle being adjusted (composition space) and an
/// optional fixed aspect ratio.
#[derive(Clone, Default)]
pub(super) struct CropState {
    pub rect: Option<kurbo::Rect>,
    pub aspect: Option<f64>,
    pub drag: Option<RectDrag>,
}

/// Slice tool state: the selected slice and any drag in progress.
#[derive(Clone, Default)]
pub(super) struct SliceState {
    pub selected: Option<usize>,
    pub drag: Option<RectDrag>,
}

const ASPECTS: [(&str, Option<f64>); 6] = [
    ("Free", None),
    ("1:1", Some(1.0)),
    ("4:3", Some(4.0 / 3.0)),
    ("16:9", Some(16.0 / 9.0)),
    ("9:16", Some(9.0 / 16.0)),
    ("4:5", Some(0.8)),
];

const SIZES: [(&str, u32, u32); 6] = [
    ("1920×1080 (HD)", 1920, 1080),
    ("3840×2160 (4K)", 3840, 2160),
    ("1080×1080 (square post)", 1080, 1080),
    ("1080×1350 (portrait post)", 1080, 1350),
    ("1080×1920 (story / reel)", 1080, 1920),
    ("1280×720", 1280, 720),
];

/// Starts cropping: the canvas (`selection` false), or the selected layers.
pub fn start_crop(app: &mut XenifyApp, selection: bool) {
    if !selection {
        app.select_only(None);
    } else if app.selected_layers().is_empty() {
        app.status = "Select the layers or group to crop first.".into();
        return;
    }
    app.set_tool(Tool::Crop);
    app.viewport.crop = CropState::default();
    app.status = if selection {
        "Drag the edges to crop the selection, then Apply (Enter). Esc cancels.".into()
    } else {
        "Drag the edges to crop the canvas (or out to enlarge it), then Apply (Enter). Esc cancels.".into()
    };
}

impl Viewport {
    /// The crop rectangle, starting as the selection's box or the whole frame.
    fn crop_rect(&mut self, app: &XenifyApp, scene: &xenify_anim::Scene) -> kurbo::Rect {
        if let Some(r) = self.crop.rect {
            return r;
        }
        let mut text = self.text.borrow_mut();
        let selection = app
            .selected_layers()
            .into_iter()
            .filter_map(|id| xenify_anim::world_box(app.comp(), scene, id, &mut *text))
            .reduce(|a, b| a.union(b));
        drop(text);
        let r = selection.unwrap_or(kurbo::Rect::new(0.0, 0.0, scene.width as f64, scene.height as f64));
        self.crop.rect = Some(r);
        r
    }

    fn grip_at(&self, rect: Rect, r: kurbo::Rect, pos: Pos2) -> Option<Grip> {
        let s = Rect::from_min_max(self.to_screen(rect, r.origin()), self.to_screen(rect, Point::new(r.x1, r.y1)));
        let tol = HANDLE + 3.0;
        let near_x = |x: f32| (pos.x - x).abs() <= tol;
        let near_y = |y: f32| (pos.y - y).abs() <= tol;
        let in_x = pos.x >= s.left() - tol && pos.x <= s.right() + tol;
        let in_y = pos.y >= s.top() - tol && pos.y <= s.bottom() + tol;
        let (l, t, rr, b) =
            (near_x(s.left()) && in_y, near_y(s.top()) && in_x, near_x(s.right()) && in_y, near_y(s.bottom()) && in_x);
        if l || t || rr || b {
            return Some(Grip::Sides { l, t, r: rr, b });
        }
        s.contains(pos).then_some(Grip::Body)
    }

    fn grip_cursor(grip: Option<Grip>) -> egui::CursorIcon {
        use egui::CursorIcon as C;
        match grip {
            Some(Grip::Body) => C::Move,
            Some(Grip::Sides { l, t, r, b }) => match (l || r, t || b) {
                (true, true) => {
                    if (l && t) || (r && b) {
                        C::ResizeNwSe
                    } else {
                        C::ResizeNeSw
                    }
                }
                (true, false) => C::ResizeHorizontal,
                _ => C::ResizeVertical,
            },
            _ => C::Crosshair,
        }
    }

    /// Applies a drag to a rectangle, keeping `aspect` if set.
    fn drag_rect(d: &RectDrag, now: Point, aspect: Option<f64>, square: bool) -> kurbo::Rect {
        let delta = now - d.start;
        let mut r = d.rect;
        match d.grip {
            Grip::Body => r = r + delta,
            Grip::Sides { l, t, r: right, b } => {
                if l {
                    r.x0 += delta.x;
                }
                if right {
                    r.x1 += delta.x;
                }
                if t {
                    r.y0 += delta.y;
                }
                if b {
                    r.y1 += delta.y;
                }
                if let Some(a) = aspect {
                    // Height follows width (or width follows height for top/bottom edges).
                    if l || right {
                        let h = r.width().abs() / a;
                        if t { r.y0 = r.y1 - h } else { r.y1 = r.y0 + h }
                    } else {
                        let w = r.height().abs() * a;
                        r.x1 = r.x0 + w;
                    }
                }
                r = r.abs();
            }
            Grip::New(from) => {
                let mut to = now;
                let aspect = aspect.or(square.then_some(1.0));
                if let Some(a) = aspect {
                    let d = to - from;
                    let w = d.x.abs().max(d.y.abs() * a);
                    to = from + kurbo::Vec2::new(w.copysign(d.x), (w / a).copysign(d.y));
                }
                r = kurbo::Rect::from_points(from, to);
            }
        }
        r
    }

    pub(super) fn crop_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp_pos: Point,
        scene: &xenify_anim::Scene,
    ) {
        let r = self.crop_rect(app, scene);
        if self.crop.drag.is_none() && resp.hovered() {
            resp.ctx.set_cursor_icon(Self::grip_cursor(self.grip_at(rect, r, pos)));
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let start = self.to_comp(rect, pos);
            let grip = self.grip_at(rect, r, pos).unwrap_or(Grip::New(start));
            self.crop.drag = Some(RectDrag { grip, start, rect: r });
        }
        if let Some(d) = self.crop.drag.clone() {
            let square = resp.ctx.input(|i| i.modifiers.shift);
            self.crop.rect = Some(Self::drag_rect(&d, comp_pos, self.crop.aspect, square));
            if resp.drag_stopped() {
                self.crop.drag = None;
            }
        }
        if resp.double_clicked() && self.grip_at(rect, r, pos) == Some(Grip::Body) {
            self.commit_crop(app);
        }
    }

    /// Enter: crops the canvas (nothing selected) or clips the selected layers.
    pub(super) fn commit_crop(&mut self, app: &mut XenifyApp) {
        let Some(r) = self.crop.rect.take() else { return };
        // Cropping is a one-off: back to selecting afterwards.
        app.tool = Tool::Select;
        let layers = app.selected_layers();
        if layers.is_empty() {
            match xenify_ops::crop::crop_comp(app.project(), app.comp, r) {
                Ok(cmd) => {
                    if app.exec("Crop canvas", cmd) {
                        let s = &app.comp().settings;
                        app.status = format!("Canvas is now {}×{}.", s.width, s.height);
                        self.request_fit();
                    }
                }
                Err(e) => app.fail(e.to_string()),
            }
        } else {
            match xenify_ops::crop::crop_layers(app.project(), app.comp, &layers, r, app.time, app.auto_key) {
                Ok(cmd) => {
                    if app.exec("Crop layers", cmd) {
                        app.status =
                            "Cropped with a mask: crop again to adjust it, reshape it with the Mask tool (U), \
                                      or remove it in Properties › Masks."
                                .into();
                    }
                }
                Err(e) => app.fail(e.to_string()),
            }
        }
    }

    pub(super) fn draw_crop(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let Some(r) = self.crop.rect else { return };
        let s = Rect::from_min_max(self.to_screen(rect, r.origin()), self.to_screen(rect, Point::new(r.x1, r.y1)));
        // Dim everything outside.
        let shade = Color32::from_black_alpha(140);
        for band in [
            Rect::from_min_max(rect.min, Pos2::new(rect.max.x, s.top())),
            Rect::from_min_max(Pos2::new(rect.min.x, s.bottom()), rect.max),
            Rect::from_min_max(Pos2::new(rect.min.x, s.top()), Pos2::new(s.left(), s.bottom())),
            Rect::from_min_max(Pos2::new(s.right(), s.top()), Pos2::new(rect.max.x, s.bottom())),
        ] {
            painter.rect_filled(band.intersect(rect), 0.0, shade);
        }
        painter.rect_stroke(s, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Middle);
        // Rule-of-thirds guides.
        let thin = Stroke::new(1.0, Color32::from_white_alpha(70));
        for k in 1..3 {
            let f = k as f32 / 3.0;
            painter.line_segment(
                [Pos2::new(s.left() + s.width() * f, s.top()), Pos2::new(s.left() + s.width() * f, s.bottom())],
                thin,
            );
            painter.line_segment(
                [Pos2::new(s.left(), s.top() + s.height() * f), Pos2::new(s.right(), s.top() + s.height() * f)],
                thin,
            );
        }
        handles(painter, s);
        let target = if app.selected_layers().is_empty() { "canvas" } else { "selected layers" };
        painter.text(
            s.left_top() + EVec2::new(0.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{:.0} × {:.0}  ·  Enter crops the {target}", r.width(), r.height()),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    pub(super) fn slice_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp_pos: Point,
    ) {
        let slices = app.comp().settings.slices.clone();
        let grip_of = |vp: &Viewport, i: usize| vp.grip_at(rect, slices[i].rect, pos);
        if self.slice.drag.is_none() && resp.hovered() {
            let g = self.slice.selected.filter(|i| *i < slices.len()).and_then(|i| grip_of(self, i));
            let g = g.or_else(|| {
                (0..slices.len()).rev().find_map(|i| (grip_of(self, i) == Some(Grip::Body)).then_some(Grip::Body))
            });
            resp.ctx.set_cursor_icon(Self::grip_cursor(g));
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let start = self.to_comp(rect, pos);
            // The selected slice's handles first, then any slice's body, else a new slice.
            let on_selected =
                self.slice.selected.filter(|i| *i < slices.len()).and_then(|i| Some((i, grip_of(self, i)?)));
            let hit = on_selected
                .or_else(|| (0..slices.len()).rev().find(|i| slices[*i].rect.contains(start)).map(|i| (i, Grip::Body)));
            match hit {
                Some((i, grip)) => {
                    app.new_gesture();
                    self.slice.selected = Some(i);
                    self.slice.drag = Some(RectDrag { grip, start, rect: slices[i].rect });
                }
                None => {
                    self.slice.selected = None;
                    self.slice.drag = Some(RectDrag { grip: Grip::New(start), start, rect: kurbo::Rect::ZERO });
                }
            }
        } else if resp.clicked() {
            self.slice.selected = (0..slices.len()).rev().find(|i| slices[*i].rect.contains(comp_pos));
        }
        let Some(d) = self.slice.drag.clone() else { return };
        let square = resp.ctx.input(|i| i.modifiers.shift);
        let r = Self::drag_rect(&d, comp_pos, None, square).round();
        match (d.grip, self.slice.selected) {
            (Grip::New(_), _) => {
                if resp.drag_stopped() {
                    self.slice.drag = None;
                    if r.width() >= 2.0 && r.height() >= 2.0 {
                        let mut settings = app.comp().settings.clone();
                        let n = settings.slices.len() + 1;
                        settings.slices.push(Slice { name: format!("Slice {n}"), rect: r });
                        self.slice.selected = Some(settings.slices.len() - 1);
                        app.exec("Add slice", Command::SetCompSettings { comp: app.comp, settings });
                    }
                } else {
                    self.slice_preview = Some(r);
                }
            }
            (_, Some(i)) if i < slices.len() => {
                let mut settings = app.comp().settings.clone();
                settings.slices[i].rect = r;
                app.exec_merged("Edit slice", Command::SetCompSettings { comp: app.comp, settings });
                if resp.drag_stopped() {
                    self.slice.drag = None;
                }
            }
            _ => self.slice.drag = None,
        }
        if self.slice.drag.is_none() {
            self.slice_preview = None;
        }
    }

    pub(super) fn draw_slices(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let active = app.tool == Tool::Slice;
        for (i, s) in app.comp().settings.slices.iter().enumerate() {
            let r = Rect::from_min_max(
                self.to_screen(rect, s.rect.origin()),
                self.to_screen(rect, Point::new(s.rect.x1, s.rect.y1)),
            );
            let selected = active && self.slice.selected == Some(i);
            let color = if selected { ACCENT } else { Color32::from_rgb(255, 140, 60) };
            let alpha = if active { 255 } else { 90 };
            let color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
            painter.rect_stroke(r, 0.0, Stroke::new(if selected { 2.0 } else { 1.0 }, color), StrokeKind::Middle);
            if active {
                let label = format!("{}  {:.0}×{:.0}", s.name, s.rect.width(), s.rect.height());
                let galley = painter.layout_no_wrap(label, egui::FontId::proportional(11.0), Color32::WHITE);
                let bg = Rect::from_min_size(r.left_top(), galley.size() + EVec2::new(8.0, 4.0));
                painter.rect_filled(bg, 0.0, color);
                painter.galley(bg.min + EVec2::new(4.0, 2.0), galley, Color32::WHITE);
                if selected {
                    handles(painter, r);
                }
            }
        }
        if let Some(p) = self.slice_preview {
            let r = Rect::from_min_max(self.to_screen(rect, p.origin()), self.to_screen(rect, Point::new(p.x1, p.y1)));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), StrokeKind::Middle);
        }
    }

    /// Delete: removes the selected slice. Returns whether it did.
    pub fn selected_slice(&self) -> Option<usize> {
        self.slice.selected
    }

    pub fn select_slice(&mut self, i: Option<usize>) {
        self.slice.selected = i;
    }

    pub fn delete_selected_slice(&mut self, app: &mut XenifyApp) -> bool {
        let Some(i) = self.slice.selected.take() else { return false };
        let mut settings = app.comp().settings.clone();
        if i >= settings.slices.len() {
            return false;
        }
        settings.slices.remove(i);
        app.exec("Delete slice", Command::SetCompSettings { comp: app.comp, settings })
    }
}

fn handles(painter: &egui::Painter, s: Rect) {
    for p in [
        s.left_top(),
        s.center_top(),
        s.right_top(),
        s.right_center(),
        s.right_bottom(),
        s.center_bottom(),
        s.left_bottom(),
        s.left_center(),
    ] {
        painter.rect(
            Rect::from_center_size(p, EVec2::splat(HANDLE)),
            0.0,
            Color32::WHITE,
            Stroke::new(1.0, ACCENT),
            StrokeKind::Middle,
        );
    }
}

/// Crop / slice settings in the control bar.
pub fn crop_options(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let mut vp = std::mem::take(&mut app.viewport);
    if app.tool == Tool::Crop {
        let target = if app.selected_layers().is_empty() { "Canvas" } else { "Selected layers" };
        ui.label(format!("Crops: {target}")).on_hover_text("Select layers to clip them instead of resizing the canvas");
        let mut current = vp.crop.aspect;
        egui::ComboBox::from_id_salt("crop-aspect")
            .selected_text(ASPECTS.iter().find(|a| a.1 == current).map_or("Custom", |a| a.0))
            .width(64.0)
            .show_ui(ui, |ui| {
                for (label, a) in ASPECTS {
                    ui.selectable_value(&mut current, a, label);
                }
            });
        if current != vp.crop.aspect {
            vp.crop.aspect = current;
            if let (Some(a), Some(r)) = (current, vp.crop.rect) {
                let c = r.center();
                let w = r.width().min(r.height() * a);
                vp.crop.rect = Some(kurbo::Rect::from_center_size(c, (w, w / a)));
            }
        }
        if let Some(r) = vp.crop.rect {
            let (mut w, mut h) = (r.width(), r.height());
            ui.label("W");
            let cw = ui.add(egui::DragValue::new(&mut w).range(1.0..=16384.0).speed(1.0)).changed();
            ui.label("H");
            let ch = ui.add(egui::DragValue::new(&mut h).range(1.0..=16384.0).speed(1.0)).changed();
            if cw || ch {
                vp.crop.rect = Some(kurbo::Rect::new(r.x0, r.y0, r.x0 + w.round(), r.y0 + h.round()));
            }
        }
        ui.menu_button("Sizes", |ui| {
            for (label, w, h) in SIZES {
                if ui.button(label).clicked() {
                    let c = vp.crop.rect.map_or(kurbo::Point::ZERO, |r| r.center());
                    vp.crop.rect = Some(kurbo::Rect::from_center_size(c, (w as f64, h as f64)));
                    vp.crop.aspect = Some(w as f64 / h as f64);
                    ui.close();
                }
            }
        });
        if ui.button(egui::RichText::new("Apply (Enter)").strong()).clicked() {
            vp.commit_crop(app);
        }
        if ui.button("Reset").on_hover_text("Back to the whole frame / selection").clicked() {
            vp.crop = CropState::default();
        }
        if ui.button("Cancel (Esc)").clicked() {
            vp.crop = CropState::default();
            app.tool = Tool::Select;
        }
    } else {
        let n = app.comp().settings.slices.len();
        let sel = vp.slice.selected.filter(|i| *i < n);
        if let Some(i) = sel {
            let mut name = app.comp().settings.slices[i].name.clone();
            ui.label("Name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0)).changed() {
                let mut settings = app.comp().settings.clone();
                settings.slices[i].name = name;
                app.exec_merged("Rename slice", Command::SetCompSettings { comp: app.comp, settings });
            }
            if ui.button("Delete").clicked() {
                vp.delete_selected_slice(app);
            }
        } else {
            ui.weak(format!("{n} slice{} — drag to add one", if n == 1 { "" } else { "s" }));
        }
        if ui.add_enabled(n > 0, egui::Button::new("Export Slices…")).clicked() {
            app.dialog = Some(crate::dialogs::Dialog::export_slices(app));
        }
    }
    app.viewport = vp;
}
