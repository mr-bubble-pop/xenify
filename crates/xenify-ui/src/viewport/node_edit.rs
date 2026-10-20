//! Edit Points (A): select and move the points, handles and edges of paths,
//! with Blender-style point tools on the right-click menu (subdivide,
//! delete, break, join, smooth/corner, simplify).
//!
//! Every shape shows its points: a rectangle or ellipse becomes a path only
//! when one of its points is actually changed (its animation is kept). Paths
//! that are still rectangles keep their size and corner controls (see
//! `xenify_doc::primitives`).
//! Click a point to select it (Shift adds), drag a box to select several,
//! drag any selected point to move them all, drag an edge to move both its
//! ends, double-click a point to make it smooth / sharp, double-click an
//! edge to add a point. The circles inside corners round them: every corner,
//! or with Alt just that one.

use super::{HANDLE, Viewport, path_tools::draw_bezpath};
use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2 as EVec2};
use xenify_anim::world_transform;
use xenify_doc::bezier;
use xenify_doc::primitives::{self, Primitive};
use xenify_doc::{Geometry, Id, LayerKind, PropPath, ShapeField, Value};
use kurbo::{Affine, BezPath, Point, Vec2};

/// How far inside a sharp corner its rounding circle sits (screen points).
const CORNER_INSET: f64 = 16.0;
/// Shapes with more sharp corners than this get no rounding circles.
const MAX_CORNER_WIDGETS: usize = 48;

/// What a drag with Edit Points is moving.
#[derive(Clone, Debug)]
pub(super) enum NodeDrag {
    /// The selected points (per shape), from where they were.
    Points { layer: Id, start: Point, paths: Vec<(usize, BezPath, Vec<usize>)>, inv: Affine },
    /// One curve handle.
    Handle { layer: Id, shape: usize, index: usize, incoming: bool, inv: Affine },
    /// A selection box (screen space).
    Box { from: Pos2 },
    /// A corner's rounding circle: `start` is how far along its inward
    /// direction the drag began (layer space).
    Corner { layer: Id, widget: CornerWidget, start: f64, inv: Affine, one: bool, path: BezPath },
}

/// A rounding circle inside a corner (layer space).
#[derive(Clone, Debug)]
pub(super) struct CornerWidget {
    shape: usize,
    /// The corner itself.
    corner: Point,
    /// Unit direction into the shape, along which the circle moves.
    dir: Vec2,
    kind: CornerKind,
}

#[derive(Clone, Copy, Debug)]
enum CornerKind {
    /// A live rectangle (one radius for all corners): corner 0..4.
    Live { index: usize, rect: kurbo::Rect, radius: f64 },
    /// A path that is a rectangle: corner 0..4 (top-left clockwise).
    Rect { index: usize, rect: kurbo::Rect, radii: [f64; 4] },
    /// A sharp point of any path; `cos_half` is the cosine of half its angle.
    Point { index: usize, cos_half: f64 },
}

impl CornerWidget {
    /// This corner's radius (0 for a sharp point).
    fn radius(&self) -> f64 {
        match self.kind {
            CornerKind::Live { radius, .. } => radius,
            CornerKind::Rect { index, radii, .. } => radii[index],
            CornerKind::Point { .. } => 0.0,
        }
    }

    /// How far from the corner the circle sits for its radius, at least `min`.
    fn distance(&self, min: f64) -> f64 {
        let d = match self.kind {
            CornerKind::Point { .. } => 0.0,
            _ => self.radius() * std::f64::consts::SQRT_2,
        };
        d.max(min)
    }
}

/// Selected layer id, its layer→comp transform, and `(shape index, path)` pairs.
type EditablePaths = (Id, Affine, Vec<(usize, BezPath)>);

impl Viewport {
    pub(super) fn editable_paths(app: &XenifyApp) -> Option<EditablePaths> {
        let id = app.selected?;
        let layer = app.project().layer(id).ok()?;
        let LayerKind::Shape { shapes } = &layer.kind else { return None };
        let paths: Vec<(usize, BezPath)> = shapes
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.geometry {
                Geometry::Path { path } => Some((i, path.value_at(app.time))),
                // Rectangles and ellipses show the points they'd have as paths.
                live => primitives::live_path(live, app.time).map(|p| (i, p)),
            })
            .collect();
        Some((id, world_transform(app.comp(), id, app.time), paths))
    }

    /// Shape `shape`'s outline now (a rectangle's or ellipse's as a path).
    fn current_path(app: &XenifyApp, layer: Id, shape: usize) -> Option<BezPath> {
        let l = app.project().layer(layer).ok()?;
        match &l.shapes().get(shape)?.geometry {
            Geometry::Path { path } => Some(path.value_at(app.time)),
            live => primitives::live_path(live, app.time),
        }
    }

    fn set_path(app: &mut XenifyApp, layer: Id, shape: usize, new: BezPath) {
        crate::shape_smart::set_shape_path(app, layer, shape, new);
    }

    /// The rounding circles of the selected layer's corners: the four of a
    /// rectangle (live or a path), or each sharp point of other paths (in
    /// Edit Points only: the Select tool shows rectangles' alone).
    pub(super) fn corner_widgets(&self, app: &XenifyApp, rect: Rect) -> Vec<CornerWidget> {
        let Some(id) = app.selected else { return Vec::new() };
        let select = app.tool == crate::app::Tool::Select;
        if select && !app.extra_selected.is_empty() {
            return Vec::new();
        }
        let Ok(layer) = app.project().layer(id) else { return Vec::new() };
        let LayerKind::Shape { shapes } = &layer.kind else { return Vec::new() };
        let world = world_transform(app.comp(), id, app.time);
        // Screen points per layer unit.
        let scale = self.zoom * world.determinant().abs().sqrt();
        if scale <= 1e-9 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (i, item) in shapes.iter().enumerate() {
            let t = app.time;
            let rect_kind = match &item.geometry {
                Geometry::Rect { position, size, corner_radius } => {
                    let r = kurbo::Rect::from_center_size(position.value_at(t).to_point(), size.value_at(t).to_size())
                        .abs();
                    Some((r, None, corner_radius.value_at(t).max(0.0)))
                }
                Geometry::Path { path } => match primitives::recognize(&path.value_at(t)) {
                    Some(Primitive::Rect { rect, radii }) => Some((rect, Some(radii), 0.0)),
                    Some(Primitive::Ellipse { .. }) => continue,
                    None => None,
                },
                Geometry::Ellipse { .. } => continue,
                // Its points get circles like a path's (below).
                Geometry::Figure { .. } => None,
            };
            if let Some((r, radii, radius)) = rect_kind {
                // Too small on screen for circles inside the corners.
                if r.width().min(r.height()) * scale < CORNER_INSET * 3.0 {
                    continue;
                }
                let corners = [
                    (Point::new(r.x0, r.y0), Vec2::new(1.0, 1.0)),
                    (Point::new(r.x1, r.y0), Vec2::new(-1.0, 1.0)),
                    (Point::new(r.x1, r.y1), Vec2::new(-1.0, -1.0)),
                    (Point::new(r.x0, r.y1), Vec2::new(1.0, -1.0)),
                ];
                for (index, (corner, dir)) in corners.into_iter().enumerate() {
                    let kind = match radii {
                        Some(radii) => CornerKind::Rect { index, rect: r, radii },
                        None => CornerKind::Live { index, rect: r, radius },
                    };
                    out.push(CornerWidget { shape: i, corner, dir: dir.normalize(), kind });
                }
                continue;
            }
            // Paths, and live figures (rounding turns them into paths).
            let path = match &item.geometry {
                Geometry::Path { path } => path.value_at(t),
                g @ Geometry::Figure { .. } => match primitives::live_path(g, t) {
                    Some(p) => p,
                    None => continue,
                },
                _ => continue,
            };
            if select {
                continue;
            }
            let mut corners = Vec::new();
            let mut flat = 0;
            for sp in bezier::to_nodes(&path) {
                let n = sp.nodes.len();
                for (k, node) in sp.nodes.iter().enumerate() {
                    let index = flat + k;
                    let inner = sp.closed || (k > 0 && k + 1 < n);
                    if !inner || n < 3 || node.handle_in.is_some() || node.handle_out.is_some() {
                        continue;
                    }
                    let prev = sp.nodes[(k + n - 1) % n];
                    let next = sp.nodes[(k + 1) % n];
                    let a = prev.handle_out.unwrap_or(prev.pos) - node.pos;
                    let b = next.handle_in.unwrap_or(next.pos) - node.pos;
                    if a.hypot() * scale < CORNER_INSET * 1.5 || b.hypot() * scale < CORNER_INSET * 1.5 {
                        continue;
                    }
                    let (a, b) = (a.normalize(), b.normalize());
                    let cos = a.dot(b).clamp(-1.0, 1.0);
                    // Nearly straight, or a spike: nothing to round.
                    if cos < -0.996 || cos > 0.996 {
                        continue;
                    }
                    let cos_half = ((1.0 + cos) / 2.0).sqrt();
                    corners.push(CornerWidget {
                        shape: i,
                        corner: node.pos,
                        dir: (a + b).normalize(),
                        kind: CornerKind::Point { index, cos_half },
                    });
                }
                flat += n;
            }
            if corners.len() <= MAX_CORNER_WIDGETS {
                out.extend(corners);
            }
        }
        let _ = rect;
        out
    }

    /// Where a corner's circle is drawn (screen).
    fn corner_widget_pos(&self, app: &XenifyApp, rect: Rect, w: &CornerWidget) -> Option<Pos2> {
        let id = app.selected?;
        let world = world_transform(app.comp(), id, app.time);
        let scale = self.zoom * world.determinant().abs().sqrt();
        let p = w.corner + w.dir * w.distance(CORNER_INSET / scale.max(1e-9));
        Some(self.to_screen(rect, world * p))
    }

    fn corner_hit(&self, app: &XenifyApp, rect: Rect, p: Pos2) -> Option<CornerWidget> {
        self.corner_widgets(app, rect)
            .into_iter()
            .find(|w| self.corner_widget_pos(app, rect, w).is_some_and(|c| (c - p).length() <= HANDLE))
    }

    /// A drag on the corner circle under `origin`, if there is one.
    fn corner_drag_at(&self, app: &XenifyApp, rect: Rect, origin: Pos2, one: bool) -> Option<NodeDrag> {
        let widget = self.corner_hit(app, rect, origin)?;
        let (layer, world, paths) = Self::editable_paths(app)?;
        let inv = world.inverse();
        let start = (inv * self.to_comp(rect, origin) - widget.corner).dot(widget.dir);
        let path = paths.iter().find(|(s, _)| *s == widget.shape).map(|(_, p)| p.clone()).unwrap_or_default();
        Some(NodeDrag::Corner { layer, widget, start, inv, one, path })
    }

    /// Select tool: whether a drag is starting on a rectangle's corner circle
    /// (or one is under way), so the drag rounds corners.
    pub(super) fn on_corner_circle(&self, app: &XenifyApp, rect: Rect, resp: &egui::Response) -> bool {
        if matches!(self.node_drag, Some(NodeDrag::Corner { .. })) {
            return true;
        }
        let press = resp.ctx.input(|i| i.pointer.press_origin());
        resp.drag_started_by(egui::PointerButton::Primary)
            && press.is_some_and(|p| self.corner_hit(app, rect, p).is_some())
    }

    /// Select tool: rounds corners by dragging their circles.
    pub(super) fn corner_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp: Point,
    ) {
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let alt = resp.ctx.input(|i| i.modifiers.alt);
            self.node_drag = self.corner_drag_at(app, rect, origin, alt);
            if self.node_drag.is_some() {
                app.new_gesture();
            }
        }
        if resp.dragged() {
            self.node_drag_update(app, rect, resp, comp);
        }
        if resp.drag_stopped() {
            self.node_drag = None;
        }
    }

    /// Selected points of the selected layer: (shape, point index).
    fn points_of(&self, app: &XenifyApp) -> Vec<(usize, usize)> {
        if self.nodes_layer != app.selected {
            return Vec::new();
        }
        self.selected_nodes.clone()
    }

    fn set_points(&mut self, app: &XenifyApp, points: Vec<(usize, usize)>) {
        self.nodes_layer = app.selected;
        self.selected_nodes = points;
    }

    /// What's under screen point `p`: a handle, a point, or an edge.
    fn node_hit(&self, app: &XenifyApp, rect: Rect, p: Pos2) -> Option<Hit> {
        let (_, world, paths) = Self::editable_paths(app)?;
        let near = |q: Point| (self.to_screen(rect, world * q) - p).length() <= HANDLE;
        for (shape, path) in &paths {
            for (i, a) in bezier::anchors(path).iter().enumerate() {
                if a.handle_in.is_some_and(|(_, h)| near(h)) {
                    return Some(Hit::Handle(*shape, i, true));
                }
                if a.handle_out.is_some_and(|(_, h)| near(h)) {
                    return Some(Hit::Handle(*shape, i, false));
                }
            }
        }
        for (shape, path) in &paths {
            for (i, a) in bezier::anchors(path).iter().enumerate() {
                if near(a.pos) {
                    return Some(Hit::Point(*shape, i));
                }
            }
        }
        // An edge: the two points it joins.
        let local = world.inverse() * self.to_comp(rect, p);
        let tol = (HANDLE as f64) / self.zoom / world.determinant().abs().sqrt().max(1e-9);
        for (shape, path) in &paths {
            let Some(h) = bezier::nearest(path, local) else { continue };
            if h.distance > tol {
                continue;
            }
            let subs = bezier::to_nodes(path);
            let before: usize = subs[..h.subpath].iter().map(|s| s.nodes.len()).sum();
            let n = subs[h.subpath].nodes.len();
            let (a, b) = (h.segment, (h.segment + 1) % n);
            return Some(Hit::Edge(*shape, before + a, before + b));
        }
        None
    }

    pub(super) fn node_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp: Point,
        scene: &xenify_anim::Scene,
    ) {
        let shift = resp.ctx.input(|i| i.modifiers.shift);
        if resp.double_clicked() {
            self.node_double_click(app, rect, pos, comp);
            return;
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let alt = resp.ctx.input(|i| i.modifiers.alt);
            self.node_drag = self.start_node_drag(app, rect, origin, shift, alt);
            if matches!(self.node_drag, Some(NodeDrag::Corner { .. })) {
                app.new_gesture();
            }
            if matches!(self.node_drag, Some(NodeDrag::Points { .. } | NodeDrag::Handle { .. })) {
                app.new_gesture();
                // Snap to the frame and other layers.
                if let Some(id) = app.selected {
                    let (_, snap) = self.snap_setup(app, scene, &[id]);
                    self.node_snap = Some(snap);
                }
            }
        } else if resp.clicked() {
            self.node_click(app, rect, pos, comp, scene, shift);
        }
        if resp.secondary_clicked() && self.points_of(app).is_empty() {
            // Right-click on a point selects it for the menu.
            if let Some(Hit::Point(s, i)) = self.node_hit(app, rect, pos) {
                self.set_points(app, vec![(s, i)]);
            }
        }

        if resp.dragged() {
            self.node_drag_update(app, rect, resp, comp);
        }
        if resp.drag_stopped() {
            match self.node_drag.take() {
                Some(NodeDrag::Box { from }) => self.box_select(app, rect, Rect::from_two_pos(from, pos), shift),
                // Rounding a point makes two: the old indices are stale.
                Some(NodeDrag::Corner { widget: CornerWidget { kind: CornerKind::Point { .. }, .. }, .. }) => {
                    self.set_points(app, Vec::new())
                }
                _ => {}
            }
            self.node_drag = None;
            self.node_snap = None;
            self.guides.clear();
        }
    }

    fn start_node_drag(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        origin: Pos2,
        shift: bool,
        alt: bool,
    ) -> Option<NodeDrag> {
        let (layer, world, paths) = Self::editable_paths(app)?;
        let inv = world.inverse();
        let start = inv * self.to_comp(rect, origin);
        let hit = self.node_hit(app, rect, origin);
        if !matches!(hit, Some(Hit::Handle(..) | Hit::Point(..)))
            && let Some(drag) = self.corner_drag_at(app, rect, origin, alt)
        {
            return Some(drag);
        }
        let mut points = self.points_of(app);
        match hit {
            Some(Hit::Handle(shape, index, incoming)) => {
                return Some(NodeDrag::Handle { layer, shape, index, incoming, inv });
            }
            Some(Hit::Point(s, i)) => {
                if !points.contains(&(s, i)) {
                    if !shift {
                        points.clear();
                    }
                    points.push((s, i));
                }
            }
            Some(Hit::Edge(s, a, b)) => {
                if !shift {
                    points.clear();
                }
                for p in [(s, a), (s, b)] {
                    if !points.contains(&p) {
                        points.push(p);
                    }
                }
            }
            None => return Some(NodeDrag::Box { from: origin }),
        }
        self.set_points(app, points.clone());
        let groups = paths
            .into_iter()
            .filter_map(|(shape, path)| {
                let idx: Vec<usize> = points.iter().filter(|(s, _)| *s == shape).map(|(_, i)| *i).collect();
                (!idx.is_empty()).then_some((shape, path, idx))
            })
            .collect();
        Some(NodeDrag::Points { layer, start, paths: groups, inv })
    }

    fn node_drag_update(&mut self, app: &mut XenifyApp, rect: Rect, resp: &egui::Response, comp: Point) {
        self.guides.clear();
        let comp = match (&self.node_snap, resp.ctx.input(|i| i.modifiers.command)) {
            (Some(snap), false) => {
                let snap = snap.clone();
                self.snap_point(comp, &snap)
            }
            _ => comp,
        };
        match self.node_drag.clone() {
            Some(NodeDrag::Points { layer, start, paths, inv }) => {
                let delta = inv * comp - start;
                for (shape, path, idx) in paths {
                    let new = bezier::move_nodes(&path, &idx, delta);
                    Self::set_path(app, layer, shape, new);
                }
            }
            Some(NodeDrag::Handle { layer, shape, index, incoming, inv }) => {
                if let Some(current) = Self::current_path(app, layer, shape) {
                    // Alt breaks the handle pair (a sharp corner with curves).
                    let mirror = !resp.ctx.input(|i| i.modifiers.alt);
                    let new = bezier::move_handle(&current, index, incoming, inv * comp, mirror);
                    Self::set_path(app, layer, shape, new);
                }
            }
            Some(NodeDrag::Corner { layer, widget, start, inv, one, path }) => {
                let along = (inv * comp - widget.corner).dot(widget.dir) - start;
                let selected = self.points_of(app);
                self.round_corner(app, layer, &widget, along, one, &path, &selected);
            }
            Some(NodeDrag::Box { .. }) | None => {}
        }
        let _ = rect;
    }

    fn box_select(&mut self, app: &XenifyApp, rect: Rect, area: Rect, add: bool) {
        let Some((_, world, paths)) = Self::editable_paths(app) else { return };
        let mut points = if add { self.points_of(app) } else { Vec::new() };
        for (shape, path) in &paths {
            for (i, a) in bezier::anchors(path).iter().enumerate() {
                if area.contains(self.to_screen(rect, world * a.pos)) && !points.contains(&(*shape, i)) {
                    points.push((*shape, i));
                }
            }
        }
        self.set_points(app, points);
    }

    fn node_click(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        pos: Pos2,
        comp: Point,
        scene: &xenify_anim::Scene,
        shift: bool,
    ) {
        match self.node_hit(app, rect, pos) {
            Some(Hit::Point(s, i)) => {
                let mut points = if shift { self.points_of(app) } else { Vec::new() };
                if let Some(k) = points.iter().position(|p| *p == (s, i)) {
                    points.remove(k);
                } else {
                    points.push((s, i));
                }
                self.set_points(app, points);
            }
            Some(Hit::Edge(s, a, b)) => {
                let mut points = if shift { self.points_of(app) } else { Vec::new() };
                points.extend([(s, a), (s, b)]);
                self.set_points(app, points);
            }
            Some(Hit::Handle(..)) => {}
            None => {
                // Another layer (or nothing): select it; shapes become editable paths.
                let hit = xenify_anim::hit_test(scene, comp, 4.0 / self.zoom, &mut *self.text.borrow_mut());
                app.select_only(hit);
                app.selected_keys.clear();
                self.set_points(app, Vec::new());
                if let Some(id) = hit {
                    self.make_editable(app, id);
                }
            }
        }
    }

    /// Explains what can't have its points edited. (Rectangles and ellipses
    /// become paths only once a point actually changes.)
    fn make_editable(&mut self, app: &mut XenifyApp, id: Id) {
        let Ok(layer) = app.project().layer(id) else { return };
        match &layer.kind {
            LayerKind::Shape { .. } => {}
            LayerKind::Text(_) => {
                app.status = "Text has no points to edit — use the Select tool to move or size it.".into();
            }
            _ => {
                app.status = "Only shapes have points to edit.".into();
            }
        }
    }

    /// Double-click: on a point toggles smooth/corner; on an edge adds a point.
    fn node_double_click(&mut self, app: &mut XenifyApp, rect: Rect, pos: Pos2, comp: Point) {
        let Some((layer, world, paths)) = Self::editable_paths(app) else { return };
        match self.node_hit(app, rect, pos) {
            Some(Hit::Point(shape, i)) => {
                if let Some(p) = Self::current_path(app, layer, shape)
                    && let Some(new) = bezier::toggle_smooth(&p, i)
                {
                    app.new_gesture();
                    Self::set_path(app, layer, shape, new);
                    self.set_points(app, vec![(shape, i)]);
                }
            }
            _ => {
                let local = world.inverse() * comp;
                let tol = (HANDLE as f64 + 2.0) / self.zoom / world.determinant().abs().sqrt().max(1e-9);
                let best = paths
                    .iter()
                    .filter_map(|(shape, path)| bezier::nearest(path, local).map(|h| (*shape, path, h)))
                    .min_by(|a, b| a.2.distance.total_cmp(&b.2.distance));
                if let Some((shape, path, hit)) = best
                    && hit.distance <= tol
                    && let Some((new, index)) = bezier::insert_node(path, hit)
                {
                    app.new_gesture();
                    Self::set_path(app, layer, shape, new);
                    self.set_points(app, vec![(shape, index)]);
                }
            }
        }
    }

    /// Applies `f` to each shape's path with that shape's selected points.
    fn edit_selected(
        &mut self,
        app: &mut XenifyApp,
        label: &str,
        f: impl Fn(&BezPath, &[usize]) -> Option<BezPath>,
    ) -> bool {
        let Some((layer, _, paths)) = Self::editable_paths(app) else { return false };
        let points = self.points_of(app);
        let mut changed = false;
        app.new_gesture();
        for (shape, path) in paths {
            let idx: Vec<usize> = points.iter().filter(|(s, _)| *s == shape).map(|(_, i)| *i).collect();
            if let Some(new) = f(&path, &idx).filter(|n| *n != path) {
                Self::set_path(app, layer, shape, new);
                changed = true;
            }
        }
        if changed {
            app.status = label.to_string();
        }
        changed
    }

    /// Arrow keys in Edit Points: moves the selected points by `d` (canvas
    /// pixels). False when no points are selected.
    pub fn nudge_points(&mut self, app: &mut XenifyApp, d: Vec2) -> bool {
        let points = self.points_of(app);
        let Some((layer, world, paths)) = Self::editable_paths(app).filter(|_| !points.is_empty()) else {
            return false;
        };
        let inv = world.inverse();
        let local = inv * d.to_point() - inv * Point::ZERO;
        for (shape, path) in paths {
            let idx: Vec<usize> = points.iter().filter(|(s, _)| *s == shape).map(|(_, i)| *i).collect();
            if !idx.is_empty() {
                Self::set_path(app, layer, shape, bezier::move_nodes(&path, &idx, local));
            }
        }
        true
    }

    /// Delete key: removes the selected points.
    pub fn delete_selected_node(&mut self, app: &mut XenifyApp) -> bool {
        if self.points_of(app).is_empty() {
            return false;
        }
        let done = self
            .edit_selected(app, "Deleted the points", |p, idx| (!idx.is_empty()).then(|| bezier::delete_nodes(p, idx)));
        self.set_points(app, Vec::new());
        done
    }

    /// The point tool's right-click menu. Shown whether or not the pointer is
    /// over the canvas (it moves onto the menu to use it).
    pub(super) fn node_context_menu(&mut self, app: &mut XenifyApp, resp: &egui::Response) {
        if app.tool == crate::app::Tool::Node {
            resp.context_menu(|ui| self.node_menu(app, ui));
        }
    }

    /// The right-click menu of point tools.
    fn node_menu(&mut self, app: &mut XenifyApp, ui: &mut egui::Ui) {
        let Some((_, _, paths)) = Self::editable_paths(app) else {
            ui.weak("Select a shape to edit its points");
            return;
        };
        if paths.is_empty() {
            ui.weak("This layer has no points to edit");
            return;
        }
        let points = self.points_of(app);
        let n = points.len();
        ui.label(
            egui::RichText::new(match n {
                0 => "No points selected (tools act on the whole path)".to_string(),
                1 => "1 point selected".to_string(),
                n => format!("{n} points selected"),
            })
            .weak(),
        );
        if ui
            .button("Subdivide")
            .on_hover_text("Add a point halfway along each selected edge (or every edge)")
            .clicked()
        {
            self.edit_selected(app, "Subdivided", |p, idx| Some(bezier::subdivide(p, idx)));
            ui.close();
        }
        if ui.add_enabled(n > 0, egui::Button::new("Delete Points").shortcut_text("Del")).clicked() {
            self.delete_selected_node(app);
            ui.close();
        }
        if ui
            .add_enabled(n == 1, egui::Button::new("Break Here"))
            .on_hover_text("Split the outline at this point (a closed shape opens; a line becomes two)")
            .clicked()
        {
            self.edit_selected(app, "Broke the path", |p, idx| idx.first().and_then(|i| bezier::break_at(p, *i)));
            ui.close();
        }
        let same_shape = n == 2 && points[0].0 == points[1].0;
        if ui
            .add_enabled(same_shape, egui::Button::new("Join Ends"))
            .on_hover_text("Connect two loose ends with a straight edge (select both ends)")
            .clicked()
        {
            let ok = self.edit_selected(app, "Joined", |p, idx| match idx {
                [a, b] => bezier::join(p, *a, *b),
                _ => None,
            });
            if !ok {
                app.status = "Select the two loose ends of open lines to join them.".into();
            }
            ui.close();
        }
        if ui
            .add_enabled(n > 0, egui::Button::new("Smooth / Sharp"))
            .on_hover_text("Toggle curve handles on the selected points")
            .clicked()
        {
            self.edit_selected(app, "Toggled smooth points", |p, idx| {
                let mut out = p.clone();
                for i in idx {
                    out = bezier::toggle_smooth(&out, *i).unwrap_or(out);
                }
                Some(out)
            });
            ui.close();
        }
        // Blender-style tools.
        let one_end = match points.as_slice() {
            [(shape, i)] => paths.iter().find(|(s, _)| s == shape).and_then(|(_, p)| {
                let ends = bezier::open_ends(p);
                let anchors = bezier::anchors(p);
                let pos = anchors.get(*i)?.pos;
                ends.iter().any(|(_, _, e)| *e == pos).then_some((*shape, *i, p.clone(), anchors))
            }),
            _ => None,
        };
        if ui
            .add_enabled(one_end.is_some(), egui::Button::new("Extrude"))
            .on_hover_text("Add a new point beyond this end of the line, ready to drag")
            .on_disabled_hover_text("Select one end point of an open line")
            .clicked()
            && let Some((shape, i, p, anchors)) = one_end
        {
            // Continue the line's direction.
            let here = anchors[i].pos;
            let neighbour = if i == 0 { anchors.get(1) } else { anchors.get(i - 1) }.map_or(here, |a| a.pos);
            let dir = (here - neighbour).normalize();
            let dir = if dir.is_finite() { dir } else { kurbo::Vec2::new(1.0, 0.0) };
            if let Some((new, j)) = bezier::extrude(&p, i, dir * (40.0 / self.zoom.max(0.05)))
                && let Some((layer, _, _)) = Self::editable_paths(app)
            {
                app.new_gesture();
                Self::set_path(app, layer, shape, new);
                self.set_points(app, vec![(shape, j)]);
                app.status = "Extruded: drag the new point into place.".into();
            }
            ui.close();
        }
        let size_id = egui::Id::new("bevel-size");
        let mut size: f64 = ui.data(|d| d.get_temp(size_id)).unwrap_or(10.0);
        ui.horizontal(|ui| {
            let bevel =
                ui.button("Bevel").on_hover_text("Cut the selected corners (or every corner) off with a straight edge");
            let round = ui.button("Round").on_hover_text("Round the selected corners (or every corner) with a curve");
            ui.add(egui::DragValue::new(&mut size).range(0.5..=1000.0).speed(0.5).suffix(" px"))
                .on_hover_text("How far along each edge the cut starts");
            if bevel.clicked() || round.clicked() {
                let is_round = round.clicked();
                self.edit_selected(app, if is_round { "Rounded corners" } else { "Bevelled corners" }, |p, idx| {
                    Some(bezier::bevel(p, idx, size, is_round))
                });
                self.set_points(app, Vec::new());
                ui.close();
            }
        });
        ui.data_mut(|d| d.insert_temp(size_id, size));
        if ui
            .button("Merge by Distance")
            .on_hover_text("Join neighbouring points that almost touch (the selected ones, or all)")
            .clicked()
        {
            let before: usize = paths.iter().map(|(_, p)| bezier::point_count(p)).sum();
            let dist = 3.0 / self.zoom.max(0.05);
            self.edit_selected(app, "Merged points", |p, idx| Some(bezier::merge_by_distance(p, idx, dist)));
            let after: usize =
                Self::editable_paths(app).map_or(0, |(_, _, ps)| ps.iter().map(|(_, p)| bezier::point_count(p)).sum());
            app.status = format!("Merged {} points.", before.saturating_sub(after));
            self.set_points(app, Vec::new());
            ui.close();
        }
        ui.separator();
        if ui.button("Select All Points").clicked() {
            let all: Vec<(usize, usize)> =
                paths.iter().flat_map(|(s, p)| (0..bezier::point_count(p)).map(move |i| (*s, i))).collect();
            self.set_points(app, all);
            ui.close();
        }
        if ui.button("Simplify Path").on_hover_text("Fewer points, same look").clicked() {
            let before: usize = paths.iter().map(|(_, p)| bezier::point_count(p)).sum();
            let tol = 1.0 / self.zoom.max(0.05);
            self.edit_selected(app, "Simplified", |p, _| Some(bezier::simplify(p, tol)));
            let after: usize =
                Self::editable_paths(app).map_or(0, |(_, _, ps)| ps.iter().map(|(_, p)| bezier::point_count(p)).sum());
            app.status = format!("Simplified: {before} → {after} points.");
            self.set_points(app, Vec::new());
            ui.close();
        }
    }

    /// Rounds corners by dragging `widget`'s circle `along` (layer units)
    /// from where the drag began: every corner of the shape (or its selected
    /// points), or with `one` just this one. `path` is the shape's outline
    /// when the drag began.
    #[allow(clippy::too_many_arguments)]
    fn round_corner(
        &mut self,
        app: &mut XenifyApp,
        layer: Id,
        widget: &CornerWidget,
        along: f64,
        one: bool,
        path: &BezPath,
        selected: &[(usize, usize)],
    ) {
        let shape = widget.shape;
        // A radius moves its circle √2 as far along the diagonal.
        let r = |r0: f64, rect: kurbo::Rect| {
            (r0 + along / std::f64::consts::SQRT_2).clamp(0.0, rect.width().min(rect.height()) / 2.0)
        };
        match widget.kind {
            CornerKind::Live { index, rect, radius } => {
                let r = r(radius, rect);
                if one {
                    let mut radii = [radius; 4];
                    radii[index] = r;
                    Self::set_path(app, layer, shape, primitives::rect_path(rect, radii));
                } else {
                    app.edit_value(layer, PropPath::shape(shape, ShapeField::CornerRadius), Value::Scalar(r));
                }
                app.status =
                    format!("Corner radius {r:.1} px{}", if one { "" } else { " (every corner; Alt: just this one)" });
            }
            CornerKind::Rect { index, rect, radii } => {
                let r = r(radii[index], rect);
                let mut new = if one { radii } else { [r; 4] };
                new[index] = r;
                Self::set_path(app, layer, shape, primitives::rect_path(rect, new));
                app.status =
                    format!("Corner radius {r:.1} px{}", if one { "" } else { " (every corner; Alt: just this one)" });
            }
            CornerKind::Point { index, cos_half } => {
                // How far along each edge the curve starts, for the circle
                // whose centre is `along` into the corner.
                let cut = (along * cos_half).max(0.0);
                let mine: Vec<usize> = selected.iter().filter(|(s, _)| *s == shape).map(|(_, i)| *i).collect();
                let which: Vec<usize> = if one {
                    vec![index]
                } else if mine.contains(&index) {
                    mine
                } else {
                    // Every sharp point with a circle of its own.
                    self.corner_widgets(app, Rect::NOTHING)
                        .into_iter()
                        .filter_map(|w| match w.kind {
                            CornerKind::Point { index, .. } if w.shape == shape => Some(index),
                            _ => None,
                        })
                        .collect()
                };
                let new = if cut > 1e-6 { bezier::bevel(path, &which, cut, true) } else { path.clone() };
                Self::set_path(app, layer, shape, new);
                app.status = format!(
                    "Rounding {} corner{} ({cut:.1} px){}",
                    which.len(),
                    if which.len() == 1 { "" } else { "s" },
                    if one { "" } else { " · Alt: just this one" }
                );
            }
        }
    }

    pub(super) fn draw_nodes(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let Some((_, world, paths)) = Self::editable_paths(app) else { return };
        let stroke = Stroke::new(1.0, ACCENT);
        let map = |p: Point| self.to_screen(rect, world * p);
        let hover = painter.ctx().pointer_hover_pos();
        let selected = self.points_of(app);
        for (shape, path) in &paths {
            draw_bezpath(painter, path, map, stroke);
            for (i, a) in bezier::anchors(path).iter().enumerate() {
                let ap = map(a.pos);
                for h in [a.handle_in, a.handle_out].into_iter().flatten() {
                    let hp = map(h.1);
                    painter.line_segment([ap, hp], Stroke::new(1.0, ACCENT.gamma_multiply(0.7)));
                    let on = hover.is_some_and(|m| (m - hp).length() <= HANDLE);
                    painter.circle(hp, 3.5, if on { ACCENT } else { Color32::WHITE }, stroke);
                }
                let on = selected.contains(&(*shape, i)) || hover.is_some_and(|m| (m - ap).length() <= HANDLE);
                painter.rect(
                    Rect::from_center_size(ap, EVec2::splat(HANDLE)),
                    0.0,
                    if on { ACCENT } else { Color32::WHITE },
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        self.draw_corner_widgets(app, painter, rect);
        if let (Some(NodeDrag::Box { from }), Some(to)) = (&self.node_drag, hover) {
            let r = Rect::from_two_pos(*from, to);
            painter.rect(r, 0.0, ACCENT.gamma_multiply(0.12), Stroke::new(1.0, ACCENT), StrokeKind::Middle);
        }
    }

    /// Corner rounding circles (not while moving points or transforming).
    pub(super) fn draw_corner_widgets(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let hover = painter.ctx().pointer_hover_pos();
        let busy =
            matches!(self.node_drag, Some(NodeDrag::Points { .. } | NodeDrag::Handle { .. } | NodeDrag::Box { .. }))
                || self.xdrag.is_some()
                || self.drag.is_some();
        if !busy {
            let dragging = match &self.node_drag {
                Some(NodeDrag::Corner { widget, .. }) => Some(widget.corner),
                _ => None,
            };
            for w in self.corner_widgets(app, rect) {
                let Some(c) = self.corner_widget_pos(app, rect, &w) else { continue };
                let on = dragging == Some(w.corner) || hover.is_some_and(|m| (m - c).length() <= HANDLE);
                painter.circle(c, 4.5, if on { ACCENT } else { Color32::WHITE }, Stroke::new(1.5, ACCENT));
                if on && dragging.is_none() {
                    let galley = painter.layout_no_wrap(
                        "Drag to round every corner · Alt: just this one".into(),
                        egui::FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                    let at = c + EVec2::new(12.0, -12.0 - galley.size().y);
                    let bg = Rect::from_min_size(at, galley.size()).expand(4.0);
                    painter.rect_filled(bg, 4.0, Color32::from_black_alpha(200));
                    painter.galley(at, galley, Color32::WHITE);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Hit {
    Handle(usize, usize, bool),
    Point(usize, usize),
    Edge(usize, usize, usize),
}
