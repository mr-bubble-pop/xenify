//! Connectors on the canvas.
//!
//! - Connector tool (F): drag from a shape to another. Over a port (the
//!   small circles on the shape under the pointer) the end glues to that
//!   port; elsewhere on the shape it glues to the whole shape and the route
//!   picks the best side. Ctrl leaves the end unglued.
//! - Select tool, with a connector selected: drag an end to glue it
//!   somewhere else, or a middle segment of an elbow line to move it (the
//!   bends are then kept as waypoints; Properties › Reset route forgets them).

use super::{Drag, HANDLE, Viewport};
use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Stroke};
use xenify_anim::Scene;
use xenify_doc::{ConnectorEnd, Id, LayerKind, PortRef, RouteStyle, Vec2};
use kurbo::Point;

/// How close (screen points) the pointer must be to a port to use it.
const PORT_REACH: f32 = 9.0;

/// Quick-connect arrows: where each is (screen) and which way it points.
type Arrows = [(Pos2, Vec2); 4];

/// What a connector end would glue to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Glue {
    pub end: ConnectorEnd,
    /// The shape's box (composition space), to highlight.
    pub shape: Option<kurbo::Rect>,
}

impl Viewport {
    /// Layers connectors can glue to: visible, drawn, not connectors.
    fn glue_targets(app: &XenifyApp, scene: &Scene, exclude: Option<Id>) -> Vec<Id> {
        app.comp()
            .layers
            .iter()
            .filter(|l| l.meta.visible && l.meta.connector.is_none() && Some(l.id) != exclude)
            .filter(|l| {
                !matches!(l.kind, LayerKind::Null)
                    && (scene.layer(l.id).is_some() || matches!(l.kind, LayerKind::Group))
            })
            .map(|l| l.id)
            .collect()
    }

    /// What an end dropped at `comp` would glue to: a port within reach,
    /// else the (outermost group of the) shape under the pointer, else
    /// nothing. `free` (Ctrl) never glues.
    pub(super) fn glue_at(
        &self,
        app: &XenifyApp,
        rect: Rect,
        scene: &Scene,
        comp: Point,
        exclude: Option<Id>,
        free: bool,
    ) -> Glue {
        let loose = Glue { end: ConnectorEnd::free(comp.to_vec2()), shape: None };
        if free {
            return loose;
        }
        let c = app.comp();
        let targets = Self::glue_targets(app, scene, exclude);
        let screen = self.to_screen(rect, comp);
        let mut measure = self.text.borrow_mut();
        // Ports first: they can sit just outside their shape.
        let mut best: Option<(f32, Id, usize, Point)> = None;
        for id in &targets {
            for (i, (p, _)) in xenify_anim::diagram::port_points(c, scene, *id).into_iter().enumerate() {
                let d = (self.to_screen(rect, p) - screen).length();
                if d <= PORT_REACH && best.is_none_or(|b| d < b.0) {
                    best = Some((d, *id, i, p));
                }
            }
        }
        if let Some((_, id, i, p)) = best {
            let shape = xenify_anim::world_box(c, scene, id, &mut *measure);
            return Glue { end: ConnectorEnd::glued(id, PortRef::Port(i as u32), p.to_vec2()), shape };
        }
        let hit = xenify_anim::hit_test(scene, comp, 3.0 / self.zoom, &mut *measure)
            .map(|id| xenify_anim::outermost_group(c, id))
            .filter(|id| targets.contains(id));
        match hit {
            Some(id) => Glue {
                end: ConnectorEnd::glued(id, PortRef::Auto, comp.to_vec2()),
                shape: xenify_anim::world_box(c, scene, id, &mut *measure),
            },
            None => loose,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn connector_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp: Point,
        scene: &Scene,
        ctrl: bool,
    ) {
        if resp.hovered() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.clicked() {
            let hit = xenify_anim::hit_test(scene, comp, 3.0 / self.zoom, &mut *self.text.borrow_mut());
            app.select_only(hit.map(|id| xenify_anim::outermost_group(app.comp(), id)));
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let start = self.to_comp(rect, pos);
            let from = self.glue_at(app, rect, scene, start, None, ctrl).end;
            self.drag = Some(Drag::Connect { from, start });
        }
        if resp.drag_stopped() {
            if let Some(Drag::Connect { from, start }) = self.drag.clone() {
                let to = self.glue_at(app, rect, scene, comp, None, ctrl).end;
                let moved = (comp - start).hypot() * self.zoom > 6.0;
                let same = from.attach.is_some()
                    && from.attach.map(|a| a.layer) == to.attach.map(|a| a.layer)
                    && (from.attach.map(|a| a.port) == Some(PortRef::Auto) || from.attach == to.attach);
                if moved && !same {
                    match xenify_ops::diagram::add_connector(app.project(), app.comp, from, to) {
                        Ok((cmd, id)) => {
                            if app.exec("Add connector", cmd) {
                                app.select_only(Some(id));
                            }
                        }
                        Err(e) => app.fail(e.to_string()),
                    }
                }
            }
            self.drag = None;
        }
    }

    /// The selected connector (Select tool) and its route: (layer, points).
    fn selected_route(&self, app: &XenifyApp) -> Option<(Id, RouteStyle, Vec<Point>)> {
        if app.tool != Tool::Select || !app.extra_selected.is_empty() {
            return None;
        }
        let id = app.selected?;
        let style = app.project().layer(id).ok()?.meta.connector.as_ref()?.route;
        let r = xenify_anim::connector_routes(app.project(), app.comp, app.time).into_iter().find(|r| r.id == id)?;
        Some((id, style, r.points))
    }

    /// A drag of the selected connector's end or middle segment, when `pos`
    /// (screen) is on one.
    pub(super) fn connector_handle_at(&self, app: &XenifyApp, rect: Rect, pos: Pos2) -> Option<Drag> {
        let (layer, style, points) = self.selected_route(app)?;
        let near = |p: Point| (self.to_screen(rect, p) - pos).length() <= HANDLE + 3.0;
        let n = points.len();
        if near(points[n - 1]) {
            return Some(Drag::ConnectorEnd { layer, to: true });
        }
        if near(points[0]) {
            return Some(Drag::ConnectorEnd { layer, to: false });
        }
        if style == RouteStyle::Orthogonal {
            for k in 1..n.saturating_sub(2) {
                if near(points[k].midpoint(points[k + 1])) {
                    return Some(Drag::ConnectorSegment { layer, index: k, points, start: self.to_comp(rect, pos) });
                }
            }
        }
        None
    }

    /// Starts dragging the selected connector's handle under the press, if
    /// there is one.
    pub(super) fn start_connector_handle(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
    ) -> bool {
        let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        match self.connector_handle_at(app, rect, pos) {
            Some(d) => {
                app.new_gesture();
                self.drag = Some(d);
                true
            }
            None => false,
        }
    }

    /// Dragging a connector handle (the Select tool).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn connector_handle_drag(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        comp: Point,
        scene: &Scene,
        ctrl: bool,
    ) {
        match self.drag.clone() {
            Some(Drag::ConnectorEnd { layer, to }) if resp.dragged() => {
                let glue = self.glue_at(app, rect, scene, comp, Some(layer), ctrl);
                let cmd = xenify_ops::diagram::edit_connector(app.project(), layer, |c| {
                    if to {
                        c.to = glue.end;
                    } else {
                        c.from = glue.end;
                    }
                });
                if let Ok(cmd) = cmd {
                    app.exec_merged("Move connector end", cmd);
                }
            }
            Some(Drag::ConnectorSegment { layer, index, points, start }) if resp.dragged() => {
                let d = comp - start;
                let (a, b) = (points[index], points[index + 1]);
                let horizontal = (a.y - b.y).abs() < 1e-6;
                let shift = if horizontal { kurbo::Vec2::new(0.0, d.y) } else { kurbo::Vec2::new(d.x, 0.0) };
                let mut moved = points.clone();
                moved[index] += shift;
                moved[index + 1] += shift;
                let n = moved.len();
                let waypoints: Vec<Vec2> = (1..n - 2).map(|k| moved[k].midpoint(moved[k + 1]).to_vec2()).collect();
                if let Ok(cmd) = xenify_ops::diagram::edit_connector(app.project(), layer, |c| c.waypoints = waypoints)
                {
                    app.exec_merged("Move connector segment", cmd);
                }
            }
            _ => {}
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
    }

    /// Ports of the shape under the pointer and the glue highlight (Connector
    /// tool, or while dragging a connector end); the selected connector's
    /// handles (Select tool).
    pub(super) fn draw_connectors(
        &self,
        app: &XenifyApp,
        painter: &egui::Painter,
        rect: Rect,
        scene: &Scene,
        hover: Option<Pos2>,
    ) {
        let ctrl = painter.ctx().input(|i| i.modifiers.command);
        let dragging_end = match &self.drag {
            Some(Drag::ConnectorEnd { layer, .. }) => Some(*layer),
            _ => None,
        };
        if (app.tool == Tool::Connector || dragging_end.is_some())
            && let Some(h) = hover.filter(|h| rect.contains(*h))
        {
            let comp = self.to_comp(rect, h);
            let glue = self.glue_at(app, rect, scene, comp, dragging_end, ctrl);
            if let Some(a) = glue.end.attach {
                self.draw_ports(app, painter, rect, scene, a.layer, Some(a.port));
            }
            if let Some(b) = glue.shape {
                let r =
                    Rect::from_two_pos(self.to_screen(rect, b.origin()), self.to_screen(rect, Point::new(b.x1, b.y1)));
                painter.rect_stroke(r.expand(2.0), 2.0, Stroke::new(1.5, ACCENT), egui::StrokeKind::Middle);
            }
            if let Some(Drag::Connect { from, .. }) = &self.drag {
                let a = self.to_screen(rect, from.point.to_point());
                let b = self.to_screen(rect, glue.end.point.to_point());
                painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, ACCENT), 6.0, 4.0));
                if let Some(f) = from.attach {
                    self.draw_ports(app, painter, rect, scene, f.layer, Some(f.port));
                }
            }
        }
        self.draw_quick_arrows(app, painter, rect, scene, hover);
        if let Some((_, style, points)) = self.selected_route(app) {
            let n = points.len();
            let sel = app.selected.and_then(|id| app.project().layer(id).ok()).and_then(|l| l.meta.connector.clone());
            for (p, glued) in [
                (points[0], sel.as_ref().is_some_and(|c| c.from.attach.is_some())),
                (points[n - 1], sel.as_ref().is_some_and(|c| c.to.attach.is_some())),
            ] {
                let c = self.to_screen(rect, p);
                let fill = if glued { ACCENT } else { Color32::WHITE };
                painter.circle(c, HANDLE * 0.8, fill, Stroke::new(1.5, Color32::from_black_alpha(160)));
            }
            if style == RouteStyle::Orthogonal {
                for k in 1..n.saturating_sub(2) {
                    let c = self.to_screen(rect, points[k].midpoint(points[k + 1]));
                    let r = Rect::from_center_size(c, egui::vec2(HANDLE * 1.3, HANDLE * 1.3));
                    painter.rect(r, 1.0, Color32::WHITE, Stroke::new(1.5, ACCENT), egui::StrokeKind::Middle);
                }
            }
        }
    }

    /// Quick connect (diagram canvases, Select tool): the selected shape's
    /// four arrows, as (screen point, direction) pairs.
    fn quick_arrows(&self, app: &XenifyApp, rect: Rect, scene: &Scene) -> Option<(Id, kurbo::Rect, Arrows)> {
        if app.tool != Tool::Select || app.comp().settings.kind.is_animation() || !app.extra_selected.is_empty() {
            return None;
        }
        if self.drag.is_some() {
            return None;
        }
        let id = app.selected?;
        let layer = app.project().layer(id).ok()?;
        if layer.meta.connector.is_some()
            || layer.meta.locked
            || !matches!(layer.kind, LayerKind::Shape { .. } | LayerKind::Precomp { .. })
        {
            return None;
        }
        let b = xenify_anim::world_box(app.comp(), scene, id, &mut *self.text.borrow_mut())?;
        let (a, c) = (self.to_screen(rect, b.origin()), self.to_screen(rect, Point::new(b.x1, b.y1)));
        let r = Rect::from_two_pos(a, c);
        const OUT: f32 = 22.0;
        Some((
            id,
            b,
            [
                (r.center_top() - egui::vec2(0.0, OUT), Vec2::new(0.0, -1.0)),
                (r.right_center() + egui::vec2(OUT, 0.0), Vec2::new(1.0, 0.0)),
                (r.center_bottom() + egui::vec2(0.0, OUT), Vec2::new(0.0, 1.0)),
                (r.left_center() - egui::vec2(OUT, 0.0), Vec2::new(-1.0, 0.0)),
            ],
        ))
    }

    /// A click on a quick-connect arrow: a copy of the shape that way, joined
    /// to it by a connector. Returns whether the click was used.
    pub(super) fn quick_connect(&mut self, app: &mut XenifyApp, rect: Rect, pos: Pos2, scene: &Scene) -> bool {
        let Some((id, b, arrows)) = self.quick_arrows(app, rect, scene) else { return false };
        let Some(&(_, dir)) = arrows.iter().find(|(p, _)| (*p - pos).length() <= 11.0) else { return false };
        let Ok(original) = app.project().layer(id).cloned() else { return false };
        let gap = 80.0;
        let step = Vec2::new(dir.x * (b.width() + gap), dir.y * (b.height() + gap));
        let next = app.project().next_id;
        let (copy_id, conn_id) = (Id(next), Id(next + 1));
        let mut copy = original.clone();
        copy.id = copy_id;
        let t = app.time;
        let pos0 = copy.transform.position.value_at(t);
        copy.transform.position = (pos0 + step).into();
        let d = xenify_ops::diagram::settings_of(app.project(), app.comp);
        let conn = xenify_ops::diagram::connector_layer(
            conn_id,
            ConnectorEnd::glued(id, PortRef::Auto, b.center().to_vec2()),
            ConnectorEnd::glued(copy_id, PortRef::Auto, (b.center() + step).to_vec2()),
            &d,
        );
        let comp = app.comp;
        let cmd = xenify_ops::Command::batch(vec![
            xenify_ops::Command::AddLayer { comp, index: None, layer: copy },
            xenify_ops::Command::AddLayer { comp, index: None, layer: conn },
        ]);
        if app.exec("Quick connect", cmd) {
            app.select_only(Some(copy_id));
        }
        true
    }

    fn draw_quick_arrows(
        &self,
        app: &XenifyApp,
        painter: &egui::Painter,
        rect: Rect,
        scene: &Scene,
        hover: Option<Pos2>,
    ) {
        let Some((_, _, arrows)) = self.quick_arrows(app, rect, scene) else { return };
        for (p, d) in arrows {
            let on = hover.is_some_and(|h| (h - p).length() <= 11.0);
            let (u, n) = (egui::vec2(d.x as f32, d.y as f32), egui::vec2(-d.y as f32, d.x as f32));
            let s = if on { 9.0 } else { 7.0 };
            let tri = vec![p + u * s, p - u * s * 0.6 + n * s * 0.8, p - u * s * 0.6 - n * s * 0.8];
            let fill = if on { ACCENT } else { ACCENT.gamma_multiply(0.55) };
            painter.add(egui::Shape::convex_polygon(tri, fill, Stroke::new(1.0, Color32::WHITE)));
        }
    }

    fn draw_ports(
        &self,
        app: &XenifyApp,
        painter: &egui::Painter,
        rect: Rect,
        scene: &Scene,
        id: Id,
        active: Option<PortRef>,
    ) {
        for (i, (p, _)) in xenify_anim::diagram::port_points(app.comp(), scene, id).into_iter().enumerate() {
            let c = self.to_screen(rect, p);
            let on = active == Some(PortRef::Port(i as u32));
            let r = if on { 5.5 } else { 4.0 };
            painter.circle(c, r, if on { ACCENT } else { Color32::WHITE }, Stroke::new(1.5, ACCENT));
        }
    }
}
