//! What screen readers see of the canvas: a canvas node holding one node per
//! object (top-level layers and whole groups, in the Layers panel's order),
//! each named by kind and layer name, with its box on screen and whether
//! it's selected. A screen reader's "click" on one selects it.
//!
//! Only built while a screen reader (or a test) asks for the tree.

use crate::app::XenifyApp;
use egui::accesskit;
use xenify_anim::Scene;
use xenify_doc::{Id, LayerKind};
use std::collections::HashMap;

/// A plain name for a layer's kind ("Shape", "Text"…).
pub fn kind_name(kind: &LayerKind) -> &'static str {
    match kind {
        LayerKind::Shape { .. } => "Shape",
        LayerKind::Text(_) => "Text",
        LayerKind::Null => "Null",
        LayerKind::Group => "Group",
        LayerKind::Precomp { .. } => "Symbol",
        LayerKind::Solid(_) => "3D object",
        LayerKind::Image { .. } => "Image",
        LayerKind::Camera(_) => "Camera",
        LayerKind::Particles(_) => "Particles",
        LayerKind::Liquid(_) => "Water",
        LayerKind::Video(_) => "Video",
        LayerKind::Audio(_) => "Sound",
    }
}

/// How a layer is read out: its kind, its name, and a text layer's words.
pub fn describe(layer: &xenify_doc::Layer) -> String {
    let kind = kind_name(&layer.kind);
    let mut out = if layer.meta.name.is_empty() { kind.to_string() } else { format!("{kind} {}", layer.meta.name) };
    if let LayerKind::Text(t) = &layer.kind
        && !t.text.is_empty()
        && t.text != layer.meta.name
    {
        out.push_str(&format!(": {}", t.text));
    }
    if !layer.meta.visible {
        out.push_str(" (hidden)");
    }
    if layer.meta.locked {
        out.push_str(" (locked)");
    }
    out
}

/// The node id of the object `layer` on the canvas.
pub fn object_id(layer: Id) -> egui::Id {
    egui::Id::new(("canvas-object", layer.0))
}

/// Describes the canvas `canvas` (already allocated) and the objects on it.
/// `boxes` gives each scene layer's box in composition space; `to_screen`
/// maps composition points to the screen.
pub fn canvas(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    canvas: &egui::Response,
    scene: &Scene,
    boxes: impl Fn(&xenify_anim::SceneLayer) -> Option<kurbo::Rect>,
    to_screen: impl Fn(kurbo::Point) -> egui::Pos2,
) {
    let comp_name = app.comp().settings.name.clone();
    let on = ui
        .ctx()
        .accesskit_node_builder(canvas.id, |n| {
            n.set_role(accesskit::Role::Canvas);
            n.set_label(format!("Canvas: {comp_name}"));
        })
        .is_some();
    if !on {
        return;
    }
    // Each object's box: its own, or a group's members' together.
    let mut bounds: HashMap<Id, kurbo::Rect> = HashMap::new();
    for l in &scene.layers {
        let Some(b) = boxes(l) else { continue };
        let id = xenify_anim::outermost_group(app.comp(), l.id);
        bounds.entry(id).and_modify(|r| *r = r.union(b)).or_insert(b);
    }
    let comp = app.comp();
    let top: Vec<(Id, String)> = comp
        .layers
        .iter()
        .rev()
        .filter(|l| xenify_anim::outermost_group(comp, l.id) == l.id)
        .map(|l| (l.id, describe(l)))
        .collect();
    let mut select = None;
    for (id, label) in top {
        let nid = object_id(id);
        let rect = bounds.get(&id).map(|b| {
            egui::Rect::from_two_pos(to_screen(kurbo::Point::new(b.x0, b.y0)), to_screen(kurbo::Point::new(b.x1, b.y1)))
        });
        let at = rect.map_or(canvas.rect.min, |r| r.min);
        // A child Ui is how egui makes a node a child of another one.
        let _ = ui.new_child(
            egui::UiBuilder::new()
                .id(nid)
                .accessibility_parent(canvas.id)
                .max_rect(egui::Rect::from_min_size(at, egui::Vec2::ZERO)),
        );
        let selected = app.is_selected(id);
        ui.ctx().accesskit_node_builder(nid, |n| {
            n.set_role(accesskit::Role::GraphicsObject);
            n.set_label(label);
            n.set_selected(selected);
            n.add_action(accesskit::Action::Click);
            n.add_action(accesskit::Action::Focus);
            if let Some(r) = rect {
                n.set_bounds(accesskit::Rect {
                    x0: r.min.x as f64,
                    y0: r.min.y as f64,
                    x1: r.max.x as f64,
                    y1: r.max.y as f64,
                });
            }
        });
        let asked = ui.input(|i| {
            i.has_accesskit_action_request(nid, accesskit::Action::Click)
                || i.has_accesskit_action_request(nid, accesskit::Action::Focus)
        });
        if asked {
            select = Some(id);
        }
    }
    if let Some(id) = select {
        let name = app.project().layer(id).map(describe).unwrap_or_default();
        app.select_only(Some(id));
        app.status = format!("Selected {name}");
    }
}
