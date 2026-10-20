//! Gradient (G): drag across shapes to set where the gradient of their fill
//! (or their stroke, when the stroke swatch is active) starts and ends. A
//! solid paint becomes a gradient from its colour; an existing gradient keeps
//! its colours. The ends of the selected shape's gradient can be dragged.

use super::{Drag, HANDLE, Viewport};
use crate::app::{XenifyApp, PaintSlot};
use egui::{Pos2, Rect};
use xenify_anim::Scene;
use xenify_doc::{Gradient, GradientKind, Id, ShapeItem};
use xenify_ops::Command;
use kurbo::Point;

impl Viewport {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn gradient_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp: Point,
        scene: &Scene,
        shift: bool,
    ) {
        if resp.hovered() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.clicked() {
            self.pick_layer(app, scene, comp);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let from = self.to_comp(rect, pos);
            app.new_gesture();
            if let Some(drag) = self.gradient_handle_at(app, rect, scene, pos) {
                self.drag = Some(drag);
                return;
            }
            if app.selected_layers().is_empty() {
                self.pick_layer(app, scene, from);
            }
            self.drag = Some(Drag::GradientLine { from });
        }
        if resp.dragged() {
            match self.drag.clone() {
                Some(Drag::Gradient { layer, index, stroke, end, inv }) => {
                    move_gradient_end(app, layer, index, stroke, end, inv * comp);
                }
                Some(Drag::GradientLine { from }) => {
                    let mut to = comp;
                    if shift {
                        let d = to - from;
                        let step = 15f64.to_radians();
                        let a = (d.y.atan2(d.x) / step).round() * step;
                        to = from + kurbo::Vec2::new(a.cos(), a.sin()) * d.hypot();
                    }
                    if (to - from).hypot() * self.zoom > 2.0 {
                        apply_line(app, scene, from, to);
                    }
                }
                _ => {}
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
    }

    /// Selects the layer under `comp` (or nothing, on empty canvas).
    fn pick_layer(&self, app: &mut XenifyApp, scene: &Scene, comp: Point) {
        let hit = xenify_anim::hit_test(scene, comp, 3.0 / self.zoom, &mut *self.text.borrow_mut());
        let usable = hit.filter(|id| app.project().layer(*id).is_ok_and(|l| l.meta.visible && !l.meta.locked));
        app.select_only(usable);
    }

    /// A drag of the selected shape's gradient start or end, when `pos`
    /// (screen) is on one.
    pub(super) fn gradient_handle_at(&self, app: &XenifyApp, rect: Rect, scene: &Scene, pos: Pos2) -> Option<Drag> {
        let (layer, index, stroke, g, to_comp) = self.gradient_target(app, scene)?;
        let near = |p: Point| (self.to_screen(rect, to_comp * p) - pos).length() <= HANDLE + 3.0;
        let end = near(g.end.to_point());
        (end || near(g.start.to_point())).then(|| Drag::Gradient { layer, index, stroke, end, inv: to_comp.inverse() })
    }
}

/// Moves a gradient's start or end to `p` (layer space). Moving a radial
/// gradient's centre carries its radius along.
pub(super) fn move_gradient_end(app: &mut XenifyApp, layer: Id, index: usize, stroke: bool, end: bool, p: Point) {
    let paint = app.project().layer(layer).ok().and_then(|l| {
        let s = l.shapes().get(index)?;
        if stroke { s.stroke.as_ref()?.gradient.clone() } else { s.fill.as_ref()?.gradient.clone() }
    });
    let Some(mut g) = paint.map(|g| g.value_at(app.time)) else { return };
    let p = p.to_vec2();
    if end {
        g.end = p;
    } else {
        if g.kind == GradientKind::Radial {
            g.end += p - g.start;
        }
        g.start = p;
    }
    crate::gradient_ui::edit(app, layer, index, stroke, g);
}

/// Gives every selected shape's fill (or stroke) a gradient from `from` to
/// `to` (composition space), in the Gradient tool's kind.
fn apply_line(app: &mut XenifyApp, scene: &Scene, from: Point, to: Point) {
    let stroke = app.paint.active == PaintSlot::Stroke;
    let kind = app.tool_options.gradient_kind;
    let mut cmds = Vec::new();
    let mut shapes = 0;
    for id in crate::colors::targets(app) {
        let Ok(layer) = app.project().layer(id) else { continue };
        let Some(to_comp) = scene.layer(id).map(|l| l.transform) else { continue };
        if layer.meta.locked || layer.shapes().is_empty() {
            continue;
        }
        shapes += layer.shapes().len();
        let inv = to_comp.inverse();
        let (start, end) = ((inv * from).to_vec2(), (inv * to).to_vec2());
        for (index, shape) in layer.shapes().iter().enumerate() {
            cmds.extend(shape_command(app, id, index, shape, stroke, kind, start, end));
        }
    }
    if !cmds.is_empty() {
        app.exec_merged("Gradient", Command::batch(cmds));
    } else if shapes > 0 {
        app.status =
            format!("The selected shapes have no {} — give them one first.", if stroke { "stroke" } else { "fill" });
    } else {
        app.status = "Select a shape, or drag across one, to give it a gradient.".into();
    }
}

#[allow(clippy::too_many_arguments)]
fn shape_command(
    app: &XenifyApp,
    layer: Id,
    index: usize,
    shape: &ShapeItem,
    stroke: bool,
    kind: GradientKind,
    start: kurbo::Vec2,
    end: kurbo::Vec2,
) -> Option<Command> {
    let (color, existing) = if stroke {
        let s = shape.stroke.as_ref()?;
        (&s.color, s.gradient.as_ref())
    } else {
        let f = shape.fill.as_ref()?;
        (&f.color, f.gradient.as_ref())
    };
    match existing {
        Some(g) => {
            let g = Gradient { kind, start, end, ..g.value_at(app.time) };
            let (cmds, _) = crate::gradient_ui::edit_commands(app, layer, index, stroke, &g);
            (!cmds.is_empty()).then(|| Command::batch(cmds))
        }
        None => {
            let stops = crate::gradient_ui::default_stops(color.value_at(app.time));
            let g = Some(Gradient { kind, start, end, stops }.into());
            let mut new = shape.clone();
            if stroke {
                new.stroke.as_mut()?.gradient = g;
            } else {
                new.fill.as_mut()?.gradient = g;
            }
            Some(Command::SetShape { layer, index, shape: new })
        }
    }
}
