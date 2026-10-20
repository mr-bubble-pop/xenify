//! The modifier handle: with the Select tool and one layer selected, the
//! origin of its first Array or Twist (the point copies turn around, or the
//! twist's centre) shows as a diamond on the canvas and can be dragged.

use super::{Drag, HANDLE, Viewport};
use crate::app::{XenifyApp, Tool};
use egui::{Color32, Pos2, Rect, Stroke};
use xenify_doc::{Id, ModifierField, PropPath, Value};
use kurbo::{Affine, Point};

impl Viewport {
    /// The selected layer's first enabled array: (layer, modifier index,
    /// origin in layer space, layer → composition transform).
    fn array_target(&self, app: &XenifyApp) -> Option<(Id, usize, Point, Affine)> {
        if app.tool != Tool::Select || !app.extra_selected.is_empty() {
            return None;
        }
        let id = app.selected?;
        let layer = app.project().layer(id).ok()?;
        let (index, origin) = layer
            .meta
            .modifiers
            .iter()
            .enumerate()
            .find_map(|(i, m)| Some((i, m.kind.origin().filter(|_| m.enabled)?)))?;
        let to_comp = xenify_anim::world_transform(app.comp(), id, app.time);
        Some((id, index, origin.value_at(app.time).to_point(), to_comp))
    }

    pub(super) fn array_handle_at(&self, app: &XenifyApp, rect: Rect, pos: Pos2) -> Option<Drag> {
        let (layer, index, origin, to_comp) = self.array_target(app)?;
        let near = (self.to_screen(rect, to_comp * origin) - pos).length() <= HANDLE + 3.0;
        (near && to_comp.determinant().abs() > 1e-12).then(|| Drag::ArrayOrigin {
            layer,
            index,
            inv: to_comp.inverse(),
        })
    }

    pub(super) fn draw_array_handle(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect) {
        let Some((layer, index, origin, to_comp)) = self.array_target(app) else { return };
        let c = self.to_screen(rect, to_comp * origin);
        // A mirror's axis: a long dashed line through the handle.
        let axis = app.project().layer(layer).ok().and_then(|l| match &l.meta.modifiers[index].kind {
            xenify_doc::ModifierKind::Mirror { rotation, .. } => Some(rotation.value_at(app.time)),
            _ => None,
        });
        if let Some(deg) = axis {
            let dir = to_comp * (origin + kurbo::Vec2::from_angle(deg.to_radians()) * 100.0) - to_comp * origin;
            if dir.hypot() > 1e-9 {
                let far = dir.normalize() * 4000.0;
                let (a, b) = (to_comp * origin - far, to_comp * origin + far);
                let (a, b) = (self.to_screen(rect, a), self.to_screen(rect, b));
                let dashes = egui::Shape::dashed_line(&[a, b], Stroke::new(1.0, crate::widgets::ACCENT), 6.0, 4.0);
                painter.with_clip_rect(rect).extend(dashes);
            }
        }
        let r = HANDLE * 0.9;
        let diamond =
            vec![c - egui::vec2(0.0, r), c + egui::vec2(r, 0.0), c + egui::vec2(0.0, r), c - egui::vec2(r, 0.0)];
        painter.add(egui::Shape::convex_polygon(
            diamond.clone(),
            crate::widgets::ACCENT,
            Stroke::new(3.0, Color32::from_black_alpha(140)),
        ));
        painter.add(egui::Shape::closed_line(diamond, Stroke::new(1.5, Color32::WHITE)));
    }
}

/// Moves array `index` of `layer`'s origin to `p` (layer space).
pub(super) fn move_array_origin(app: &mut XenifyApp, layer: Id, index: usize, p: Point) {
    let path = PropPath::Modifier { index, field: ModifierField::Origin };
    app.edit_value(layer, path, Value::Vec2(p.to_vec2()));
}
