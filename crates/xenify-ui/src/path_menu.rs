//! Layer › Path: whole-shape point tools for the selected layers (simplify,
//! subdivide, convert to path); per-point tools live on Edit Points'
//! right-click menu.

use crate::app::{XenifyApp, Tool};
use xenify_doc::{Geometry, PropPath, ShapeField, Value, bezier};
use xenify_ops::Command;

/// Applies `f` to every path shape of the selected layers (one undo step).
fn each_path(app: &mut XenifyApp, label: &str, f: impl Fn(&kurbo::BezPath) -> kurbo::BezPath) -> (usize, usize) {
    let (mut before, mut after) = (0, 0);
    let mut cmds = Vec::new();
    for id in app.selected_layers() {
        let Ok(layer) = app.project().layer(id).cloned() else { continue };
        for (i, s) in layer.shapes().iter().enumerate() {
            let Geometry::Path { path } = &s.geometry else { continue };
            let p = path.value_at(app.time);
            let new = f(&p);
            before += bezier::point_count(&p);
            after += bezier::point_count(&new);
            if new != p {
                let prop = PropPath::shape(i, ShapeField::Path);
                if let Ok(c) = Command::edit_value(app.project(), id, prop, app.time, Value::Path(new), app.auto_key) {
                    cmds.push(c);
                }
            }
        }
    }
    if !cmds.is_empty() {
        app.exec(label, Command::batch(cmds));
    }
    (before, after)
}

pub fn simplify_selected(app: &mut XenifyApp) {
    // About one screen pixel.
    let tol = 1.0 / app.viewport.zoom().max(0.05);
    let (before, after) = each_path(app, "Simplify", |p| bezier::simplify(p, tol));
    app.status = if before == 0 {
        "Select shapes with paths to simplify (Convert to Path first for rectangles and ellipses).".into()
    } else {
        format!("Simplified: {before} → {after} points.")
    };
}

pub fn subdivide_selected(app: &mut XenifyApp) {
    let (before, after) = each_path(app, "Subdivide", |p| bezier::subdivide(p, &[]));
    app.status = if before == 0 {
        "Select shapes with paths to subdivide (Convert to Path first for rectangles and ellipses).".into()
    } else {
        format!("Subdivided: {before} → {after} points.")
    };
}

pub fn convert_selected(app: &mut XenifyApp) {
    let cmds: Vec<Command> =
        app.selected_layers().into_iter().filter_map(|id| crate::viewport::convert_to_path_command(app, id)).collect();
    if cmds.is_empty() {
        app.status = "Nothing to convert: select rectangles or ellipses.".into();
    } else if app.exec("Convert to path", Command::batch(cmds)) {
        app.set_tool(Tool::Node);
        app.status = "Converted to paths: drag their points with Edit Points (A).".into();
    }
}

/// Whether any selected layer has a shape (so path tools apply).
pub fn has_shapes(app: &XenifyApp) -> bool {
    app.selected_layers().iter().any(|id| app.project().layer(*id).is_ok_and(|l| !l.shapes().is_empty()))
}

/// Path › Outline Stroke: the selected shapes' strokes become filled shapes.
pub fn outline_selected(app: &mut XenifyApp) {
    let (comp, layers) = (app.comp().id, app.selected_layers());
    match xenify_ops::outline::outline_strokes(app.project(), comp, &layers, app.time) {
        Ok(cmd) => {
            if app.exec("Outline stroke", cmd) {
                app.status = "Outlined: the strokes are filled shapes now.".into();
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

/// Path › Offset Path: adds an Offset Path modifier to the selected layers.
pub fn offset_selected(app: &mut XenifyApp) {
    let layers: Vec<xenify_doc::Layer> =
        app.selected_layers().iter().filter_map(|id| app.project().layer(*id).ok().cloned()).collect();
    crate::modifiers_ui::add(app, &layers, crate::modifiers_ui::Preset::OffsetPath);
}
