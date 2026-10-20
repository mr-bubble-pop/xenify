//! Free Transform (Q) and Perspective (W): a box around the selection with
//! scale, rotate and corner-pin handles, working around a movable centre.

use super::{HANDLE, Viewport};
use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke, Vec2 as EVec2};
use xenify_doc::{Homography, Id, LayerKind, PropPath, Quad};
use xenify_ops::Command;
use xenify_ops::transform::{Snapshot, rotate_about, scale_about};
use kurbo::{Affine, Point, Vec2};

/// Distance outside a corner (screen points) where dragging rotates.
const ROTATE_ZONE: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Move,
    /// Corner index (top-left, top-right, bottom-right, bottom-left).
    Corner(usize),
    /// Edge index: top, right, bottom, left.
    Edge(usize),
    Rotate,
    Pivot,
}

/// What a free-transform box acts on right now.
pub(super) struct Target {
    snapshots: Vec<Snapshot>,
    /// The box in composition space (a quad once perspective is applied).
    quad: Quad,
    /// Direction of the box's x axis (radians).
    angle: f64,
}

#[derive(Clone)]
pub(super) struct XDrag {
    part: Part,
    snapshots: Vec<Snapshot>,
    quad: Quad,
    angle: f64,
    pivot: Point,
    start: Point,
}

/// A custom centre for the current selection (reset when the selection changes).
#[derive(Clone, Default)]
pub(super) struct Pivot {
    pub for_layers: Vec<Id>,
    pub at: Option<Point>,
    /// Alt is held: with the Select tool the centre pivot is only grabbed
    /// then (or once moved), so dragging the middle of a shape moves it.
    pub alt: std::cell::Cell<bool>,
}

impl Viewport {
    /// The selection turns around a centre the user moved (for one layer:
    /// its transform origin is away from the middle of its box).
    pub fn pivot_moved(&self, app: &XenifyApp) -> bool {
        let selected = app.selected_layers();
        if self.pivot.at.is_some() && self.pivot.for_layers == selected {
            return true;
        }
        selected.len() == 1
            && box_centre(app, selected[0]).is_some_and(|c| (c - origin_in_comp(app, selected[0])).hypot() > 0.5)
    }

    pub fn reset_pivot(&mut self) {
        self.pivot = Pivot::default();
    }

    /// The layers to transform: the selection, minus locked layers and layers
    /// whose parent is also selected (they move with it).
    fn xform_target(&self, app: &XenifyApp, scene: &xenify_anim::Scene) -> Option<Target> {
        let t = app.time;
        let ids: Vec<Id> = app
            .selected_layers()
            .into_iter()
            // Connectors have no box to scale or turn: they're moved by their ends.
            .filter(|id| app.project().layer(*id).is_ok_and(|l| !l.meta.locked && l.meta.connector.is_none()))
            .filter(|id| app.project().layer(*id).ok().and_then(|l| l.meta.parent).is_none_or(|p| !app.is_selected(p)))
            .collect();
        if ids.is_empty() {
            return None;
        }
        let mut text = self.text.borrow_mut();
        let mut snapshots = Vec::new();
        for id in &ids {
            let layer = app.project().layer(*id).ok()?;
            let world = xenify_anim::world_transform(app.comp(), *id, t);
            // The box the corner pin maps from: the pin's own source, else the content box.
            let bounds = match (&layer.transform.distort, &layer.kind) {
                (Some(d), _) => d.source,
                (None, LayerKind::Group | LayerKind::Null) => {
                    let b = xenify_anim::world_box(app.comp(), scene, *id, &mut *text)?;
                    let inv = world.inverse();
                    let pts = Quad::of_rect(b).map(|p| inv * p).points();
                    pts.iter().fold(kurbo::Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
                }
                (None, _) => scene.layer(*id)?.bounds(&mut *text)?,
            };
            snapshots.push(Snapshot::new(app.project(), app.comp, *id, t, bounds).ok()?);
        }
        let (quad, angle) = if snapshots.len() == 1 {
            let s = &snapshots[0];
            let [a, b, ..] = s.world().as_coeffs();
            (s.corners(t), b.atan2(a))
        } else {
            let pts: Vec<Point> = snapshots.iter().flat_map(|s| s.corners(t).points()).collect();
            let r = pts.iter().fold(kurbo::Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
            (Quad::of_rect(r), 0.0)
        };
        Some(Target { snapshots, quad, angle })
    }

    fn pivot_for(&self, app: &XenifyApp, quad: &Quad) -> Point {
        let selected = app.selected_layers();
        match self.pivot.at {
            Some(p) if self.pivot.for_layers == selected => p,
            // One layer turns around its own transform origin.
            _ if selected.len() == 1 && app.tool != Tool::Perspective => origin_in_comp(app, selected[0]),
            _ => {
                let c = quad.0.iter().fold(Vec2::ZERO, |s, p| s + *p) / 4.0;
                c.to_point()
            }
        }
    }

    fn part_at(&self, app: &XenifyApp, rect: Rect, target: &Target, pos: Pos2) -> Option<Part> {
        let s = target.quad.points().map(|p| self.to_screen(rect, p));
        let near = |q: Pos2| (q - pos).length() <= HANDLE + 3.0;
        let pivot = self.to_screen(rect, self.pivot_for(app, &target.quad));
        let perspective = app.tool == Tool::Perspective;
        let pivot_live = app.tool != Tool::Select
            || self.pivot.alt.get()
            || (self.pivot.at.is_some() && self.pivot.for_layers == app.selected_layers());
        if !perspective && pivot_live && near(pivot) {
            return Some(Part::Pivot);
        }
        if let Some(i) = (0..4).find(|i| near(s[*i])) {
            return Some(Part::Corner(i));
        }
        if !perspective {
            if let Some(i) = (0..4).find(|i| near(s[*i] + (s[(i + 1) % 4] - s[*i]) / 2.0)) {
                return Some(Part::Edge(i));
            }
            if (0..4).any(|i| (s[i] - pos).length() <= HANDLE + ROTATE_ZONE) && !inside(&s, pos) {
                return Some(Part::Rotate);
            }
        }
        inside(&s, pos).then_some(Part::Move)
    }

    /// Select tool: whether the pointer is pressing (or hovering) one of the
    /// selection's transform handles — then the drag is a transform.
    pub(super) fn on_transform_handles(
        &self,
        app: &XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        scene: &xenify_anim::Scene,
    ) -> bool {
        self.pivot.alt.set(resp.ctx.input(|i| i.modifiers.alt));
        let Some(t) = self.xform_target(app, scene) else { return false };
        let press = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        let pressed = resp.drag_started() || resp.double_clicked();
        let at = if pressed { press } else { pos };
        // On a selection that's small on screen the handles cover its middle:
        // pressing inside it then moves it (handles still work from outside).
        let s = t.quad.points().map(|p| self.to_screen(rect, p));
        let screen = egui::Rect::from_points(&s);
        if app.tool == Tool::Select
            && screen.width().min(screen.height()) < HANDLE * 6.0
            && screen.shrink(1.0).contains(at)
        {
            return false;
        }
        let part = self.part_at(app, rect, &t, at);
        let handle = matches!(part, Some(Part::Corner(_) | Part::Edge(_) | Part::Rotate | Part::Pivot));
        handle && (pressed || resp.hovered())
    }

    pub(super) fn xform_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp_pos: Point,
        scene: &xenify_anim::Scene,
    ) {
        let mods = resp.ctx.input(|i| i.modifiers);
        let target = self.xform_target(app, scene);
        // Cursor feedback.
        if self.xdrag.is_none()
            && let Some(t) = &target
            && resp.hovered()
        {
            let icon = match self.part_at(app, rect, t, pos) {
                Some(Part::Move) => egui::CursorIcon::Move,
                Some(Part::Corner(_)) if app.tool == Tool::Perspective => egui::CursorIcon::Crosshair,
                Some(Part::Corner(i)) => {
                    if i % 2 == 0 {
                        egui::CursorIcon::ResizeNwSe
                    } else {
                        egui::CursorIcon::ResizeNeSw
                    }
                }
                Some(Part::Edge(i)) => {
                    if i % 2 == 0 {
                        egui::CursorIcon::ResizeVertical
                    } else {
                        egui::CursorIcon::ResizeHorizontal
                    }
                }
                Some(Part::Rotate) => egui::CursorIcon::Alias,
                Some(Part::Pivot) => egui::CursorIcon::Grab,
                None => egui::CursorIcon::Default,
            };
            resp.ctx.set_cursor_icon(icon);
        }

        if resp.double_clicked()
            && let Some(t) = &target
            && self.part_at(app, rect, t, pos) == Some(Part::Pivot)
        {
            self.reset_pivot();
            return;
        }

        if resp.drag_started_by(egui::PointerButton::Primary) {
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let start = self.to_comp(rect, pos);
            let part = target.as_ref().and_then(|t| self.part_at(app, rect, t, pos));
            match (target, part) {
                (Some(t), Some(part)) => {
                    app.new_gesture();
                    let pivot = self.pivot_for(app, &t.quad);
                    self.xdrag =
                        Some(XDrag { part, snapshots: t.snapshots, quad: t.quad, angle: t.angle, pivot, start });
                }
                _ => {
                    // Clicking elsewhere selects what's there (then drag it next time).
                    let hit = xenify_anim::hit_test(scene, start, 4.0 / self.zoom, &mut *self.text.borrow_mut())
                        .map(|h| xenify_anim::outermost_group(app.comp(), h));
                    app.select_only(hit);
                }
            }
        } else if resp.clicked() {
            let on_box = target.as_ref().and_then(|t| self.part_at(app, rect, t, pos)).is_some();
            if !on_box {
                let hit = xenify_anim::hit_test(scene, comp_pos, 4.0 / self.zoom, &mut *self.text.borrow_mut())
                    .map(|h| xenify_anim::outermost_group(app.comp(), h));
                if mods.shift {
                    if let Some(h) = hit {
                        app.toggle_selection(h);
                    }
                } else {
                    app.select_only(hit);
                }
            }
        }

        if resp.dragged()
            && let Some(d) = self.xdrag.clone()
        {
            self.xform_drag(app, &d, comp_pos, mods);
        }
        if resp.drag_stopped() {
            if let Some(d) = self.xdrag.take()
                && d.part == Part::Move
                && let Some(p) = self.pivot.at
                && self.pivot.for_layers == app.selected_layers()
            {
                // A custom centre travels with the selection.
                self.pivot.at = Some(p + (comp_pos - d.start));
            }
            if let Some(p) = self.pending_pivot.take() {
                // One layer: move its transform origin (nothing on screen moves).
                // Several: a centre for this selection only.
                let selected = app.selected_layers();
                if !(selected.len() == 1 && set_origin(app, selected[0], p)) {
                    self.pivot = Pivot { for_layers: selected, at: Some(p), ..Default::default() };
                }
            }
        }
    }

    fn xform_drag(&mut self, app: &mut XenifyApp, d: &XDrag, now: Point, mods: egui::Modifiers) {
        let (cos, sin) = (d.angle.cos(), d.angle.sin());
        // Composition vector → box axes.
        let to_box = |v: Vec2| Vec2::new(v.x * cos + v.y * sin, -v.x * sin + v.y * cos);
        let from_box = |v: Vec2| Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
        let corners = d.quad.points();
        let perspective_tool = app.tool == Tool::Perspective;
        let affine = |x: Affine, app: &mut XenifyApp, label: &str| {
            let mut cmds = Vec::new();
            for s in &d.snapshots {
                match s.affine_commands(app.project(), x, app.time, app.auto_key) {
                    Ok(c) => cmds.extend(c),
                    Err(e) => app.fail(e.to_string()),
                }
            }
            app.exec_merged(label, Command::batch(cmds));
        };
        match d.part {
            Part::Pivot => {
                self.pending_pivot = Some(now);
            }
            Part::Move => {
                let mut delta = now - d.start;
                if mods.shift {
                    // Constrain to the dominant axis.
                    if delta.x.abs() > delta.y.abs() { delta.y = 0.0 } else { delta.x = 0.0 }
                }
                affine(Affine::translate(delta), app, "Move");
            }
            Part::Rotate => {
                let a0 = (d.start - d.pivot).atan2();
                let a1 = (now - d.pivot).atan2();
                let mut deg = (a1 - a0).to_degrees();
                if mods.shift {
                    deg = (deg / 15.0).round() * 15.0;
                }
                affine(rotate_about(d.pivot, deg), app, "Rotate");
                app.status = format!("Rotate {deg:.1}°");
            }
            // Distorting corners is the Perspective tool's job only.
            Part::Corner(i) if perspective_tool => {
                let delta = now - d.start;
                let mut q = d.quad;
                if mods.command {
                    // Free distort: just this corner.
                    q.0[i] += delta;
                } else {
                    // Perspective: the neighbour on the edge we're sliding along moves the other way.
                    let b = to_box(delta);
                    let (along, neighbour) = if b.x.abs() >= b.y.abs() {
                        (from_box(Vec2::new(b.x, 0.0)), [1, 0, 3, 2][i])
                    } else {
                        (from_box(Vec2::new(0.0, b.y)), [3, 2, 1, 0][i])
                    };
                    q.0[i] += along;
                    q.0[neighbour] -= along;
                }
                if let Some(g) = Homography::from_quads(&d.quad, &q) {
                    let cmds: Vec<Command> =
                        d.snapshots.iter().filter_map(|s| s.distort_command(g, app.time, app.auto_key)).collect();
                    app.exec_merged("Perspective", Command::batch(cmds));
                }
            }
            Part::Corner(i) | Part::Edge(i) => {
                let edge = matches!(d.part, Part::Edge(_));
                // Alt: scale from the opposite corner/edge instead of the centre.
                let pivot = if mods.alt {
                    if edge {
                        let (a, b) = (corners[(i + 2) % 4], corners[(i + 3) % 4]);
                        a.midpoint(b)
                    } else {
                        corners[(i + 2) % 4]
                    }
                } else {
                    d.pivot
                };
                let u0 = to_box(d.start - pivot);
                let u1 = to_box(now - pivot);
                let ratio = |a: f64, b: f64| if a.abs() < 1e-6 { 1.0 } else { b / a };
                let (mut sx, mut sy) = (ratio(u0.x, u1.x), ratio(u0.y, u1.y));
                if edge {
                    if i % 2 == 0 {
                        sx = 1.0;
                    } else {
                        sy = 1.0;
                    }
                } else {
                    // Corners keep proportions (the Perspective tool distorts).
                    let s = (u1.hypot() / u0.hypot().max(1e-6)).copysign(u0.dot(u1));
                    (sx, sy) = (s, s);
                }
                affine(scale_about(pivot, d.angle, sx, sy), app, "Scale");
                app.status = format!("Scale {:.0}% × {:.0}%", sx * 100.0, sy * 100.0);
            }
        }
    }

    pub(super) fn draw_xform(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect, scene: &xenify_anim::Scene) {
        // No handles over text being typed into.
        if app.in_place.is_some() {
            return;
        }
        let Some(t) = self.xform_target(app, scene) else { return };
        let stroke = Stroke::new(1.5, ACCENT);
        let s = t.quad.points().map(|p| self.to_screen(rect, p));
        painter.add(egui::Shape::closed_line(s.to_vec(), stroke));
        let perspective = app.tool == Tool::Perspective;
        for (i, c) in s.iter().enumerate() {
            if perspective {
                painter.circle(*c, HANDLE * 0.75, Color32::WHITE, stroke);
            } else {
                painter.rect(
                    Rect::from_center_size(*c, EVec2::splat(HANDLE)),
                    1.0,
                    Color32::WHITE,
                    stroke,
                    egui::StrokeKind::Middle,
                );
                let mid = *c + (s[(i + 1) % 4] - *c) / 2.0;
                painter.rect(
                    Rect::from_center_size(mid, EVec2::splat(HANDLE * 0.8)),
                    0.0,
                    Color32::WHITE,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
        }
        if !perspective {
            let p = self.pending_pivot.unwrap_or_else(|| self.pivot_for(app, &t.quad));
            let p = self.to_screen(rect, p);
            painter.circle_stroke(p, 5.0, Stroke::new(1.5, Color32::WHITE));
            painter.circle_stroke(p, 5.0, Stroke::new(1.0, ACCENT));
            painter.line_segment([p - EVec2::X * 9.0, p + EVec2::X * 9.0], stroke);
            painter.line_segment([p - EVec2::Y * 9.0, p + EVec2::Y * 9.0], stroke);
        }
    }
}

/// Whether `p` is inside the (convex) screen quad.
fn inside(q: &[Pos2; 4], p: Pos2) -> bool {
    let sign = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
    let s: Vec<f32> = (0..4).map(|i| sign(q[i], q[(i + 1) % 4])).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// Removes the corner pin from the selected layers.
pub fn clear_distort(app: &mut XenifyApp) {
    let cmds: Vec<Command> = app
        .selected_layers()
        .into_iter()
        .filter(|id| app.project().layer(*id).is_ok_and(|l| l.transform.distort.is_some()))
        .map(|layer| Command::SetDistort { layer, distort: None })
        .collect();
    if cmds.is_empty() {
        app.status = "Nothing selected has perspective.".into();
    } else {
        app.exec("Remove perspective", Command::batch(cmds));
    }
}

/// Where a layer's transform origin is, in composition space.
fn origin_in_comp(app: &XenifyApp, id: Id) -> Point {
    let t = app.time;
    let Ok(layer) = app.project().layer(id) else { return Point::ZERO };
    xenify_anim::world_transform(app.comp(), id, t) * layer.transform.anchor.value_at(t).to_point()
}

/// The middle of a layer's box, in composition space.
fn box_centre(app: &XenifyApp, id: Id) -> Option<Point> {
    let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
    xenify_anim::world_box(app.comp(), &scene, id, &mut xenify_anim::ApproxText).map(|b| b.center())
}

/// Moves a layer's transform origin to `p` (composition space) without moving
/// the layer: the position changes to match. Animated origins and positions
/// shift as a whole (every keyframe), which keeps the layer in place at every
/// time as long as its rotation, scale and skew aren't animated. Returns false
/// (changing nothing) when they are.
pub fn set_origin(app: &mut XenifyApp, id: Id, p: Point) -> bool {
    let t = app.time;
    let Ok(layer) = app.project().layer(id).cloned() else { return false };
    let tr = &layer.transform;
    if tr.rotation.is_animated()
        || tr.scale.is_animated()
        || tr.skew.is_animated()
        || tr.skew_axis.is_animated()
        || tr.distort.is_some()
    {
        app.status = "This layer's rotation, scale or skew is animated, so the centre is moved for now only.".into();
        return false;
    }
    let world = xenify_anim::world_transform(app.comp(), id, t);
    let parent = layer.meta.parent.map_or(Affine::IDENTITY, |p| xenify_anim::world_transform(app.comp(), p, t));
    // How far the origin moves (layer space) and the position with it (parent space).
    let d_anchor = (world.inverse() * p).to_vec2() - tr.anchor.value_at(t);
    let d_position = (parent.inverse() * p).to_vec2() - tr.position.value_at(t);
    let shifted = |prop: &xenify_doc::Property<xenify_doc::Vec2>, d: xenify_doc::Vec2| match prop {
        xenify_doc::Property::Static(v) => xenify_doc::Property::Static(*v + d),
        xenify_doc::Property::Animated(keys) => xenify_doc::Property::Animated(
            keys.iter().map(|k| xenify_doc::Keyframe { value: k.value + d, ..k.clone() }).collect(),
        ),
    };
    let cmds = vec![
        Command::SetProperty {
            layer: id,
            path: PropPath::Anchor,
            property: xenify_doc::AnyProperty::Vec2(shifted(&tr.anchor, d_anchor)),
        },
        Command::SetProperty {
            layer: id,
            path: PropPath::Position,
            property: xenify_doc::AnyProperty::Vec2(shifted(&tr.position, d_position)),
        },
    ];
    app.exec("Move transform origin", Command::batch(cmds));
    true
}

/// "Re-centre": the transform origin back to the middle of the layer (or
/// the selection's centre back to the middle).
pub fn recentre(app: &mut XenifyApp) {
    let selected = app.selected_layers();
    if selected.len() == 1
        && let Some(c) = box_centre(app, selected[0])
    {
        set_origin(app, selected[0], c);
    }
    app.viewport.reset_pivot();
}
