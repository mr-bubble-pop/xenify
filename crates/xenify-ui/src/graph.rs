//! Graph editor: a property's value over time as curves, After Effects style.
//! Drag keyframes to change their value and time, and drag the bézier handles
//! of a segment to shape its easing (they are the easing's cubic-bézier
//! control points, drawn in value space).

use crate::app::XenifyApp;
use crate::widgets::{ACCENT, prop_label};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use xenify_doc::{AnyProperty, Easing, Id, PropPath, Value};
use xenify_ops::Command;

const KEY_R: f32 = 5.0;
const HANDLE_R: f32 = 4.0;
/// Curve colours per dimension (x/red, y/green), or the accent for single values.
const DIM_COLORS: [Color32; 2] = [Color32::from_rgb(235, 110, 100), Color32::from_rgb(110, 205, 120)];

#[derive(Default)]
pub struct GraphState {
    /// Property picked in the dropdown (else the selected keyframes' property).
    chosen: Option<(Id, PropPath)>,
    drag: Option<GraphDrag>,
}

#[derive(Clone, Copy)]
enum GraphDrag {
    /// A keyframe's value in dimension `dim`; `time` follows the drag.
    Key { time: f64, dim: usize },
    /// The out (`out == true`) or in handle of the segment starting at `time`.
    Handle { time: f64, out: bool, dim: usize },
}

/// One keyframe as plain numbers.
struct Key {
    time: f64,
    value: Vec<f64>,
    easing: Easing,
}

fn keys_of(prop: &AnyProperty) -> Option<Vec<Key>> {
    Some(match prop {
        AnyProperty::Scalar(p) => {
            p.keyframes().iter().map(|k| Key { time: k.time, value: vec![k.value], easing: k.easing }).collect()
        }
        AnyProperty::Vec2(p) => p
            .keyframes()
            .iter()
            .map(|k| Key { time: k.time, value: vec![k.value.x, k.value.y], easing: k.easing })
            .collect(),
        _ => return None,
    })
}

fn dims(v: &Value) -> Vec<f64> {
    match v {
        Value::Scalar(x) => vec![*x],
        Value::Vec2(p) => vec![p.x, p.y],
        _ => vec![],
    }
}

fn with_dim(v: &Value, dim: usize, x: f64) -> Value {
    match v {
        Value::Vec2(p) if dim == 0 => Value::Vec2(xenify_doc::Vec2::new(x, p.y)),
        Value::Vec2(p) => Value::Vec2(xenify_doc::Vec2::new(p.x, x)),
        _ => Value::Scalar(x),
    }
}

/// The easing's control points (x1, y1, x2, y2); linear is a straight bézier.
fn controls(e: Easing) -> Option<[f64; 4]> {
    match e {
        Easing::Linear => Some([1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]),
        Easing::Bezier { x1, y1, x2, y2 } => Some([x1, y1, x2, y2]),
        Easing::Hold | Easing::Steps { .. } | Easing::Spring { .. } | Easing::Stops { .. } => None,
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    // Which property: the chosen one, else the selected keyframes', else the
    // selected layer's first animated number/position property.
    let layer_id = app.selected;
    let mut target = app.graph.chosen.filter(|(l, _)| Some(*l) == layer_id);
    if target.is_none() {
        target = app.selected_keys.first().filter(|k| Some(k.layer) == layer_id).map(|k| (k.layer, k.path));
    }
    let Some(layer) = layer_id.and_then(|id| app.project().layer(id).ok()).cloned() else {
        ui.weak("Select a layer to see its animation curves.");
        return;
    };
    let graphable: Vec<PropPath> = layer
        .property_paths()
        .into_iter()
        .filter(|p| layer.property(*p).is_ok_and(|a| a.is_animated() && keys_of(&a).is_some()))
        .collect();
    let path = target.map(|t| t.1).filter(|p| graphable.contains(p)).or_else(|| graphable.first().copied());

    ui.horizontal(|ui| {
        ui.label(RichText::new(&layer.meta.name).strong());
        let shown = path.map_or("—".to_string(), prop_label);
        egui::ComboBox::from_id_salt("graph-prop").selected_text(shown).show_ui(ui, |ui| {
            for p in &graphable {
                if ui.selectable_label(Some(*p) == path, prop_label(*p)).clicked() {
                    app.graph.chosen = Some((layer.id, *p));
                }
            }
        });
        ui.weak("Drag keyframes (value/time) or the handles (easing). Shift: value only.");
    });
    let Some(path) = path else {
        ui.weak("This layer has no animated number or position properties. Add keyframes to see their curves.");
        return;
    };
    let Ok(prop) = layer.property(path) else { return };
    let Some(keys) = keys_of(&prop) else { return };
    let (mul, suffix, _) = crate::widgets::prop_display_in(path, app.comp().settings.units);
    let settings = app.comp().settings.clone();
    let (duration, fps) = (settings.duration, settings.fps);
    let n_dims = keys.first().map_or(1, |k| k.value.len());

    // Sample the curve for the value range.
    let (rect, bg) = ui.allocate_exact_size(ui.available_size().max(Vec2::new(200.0, 120.0)), Sense::click_and_drag());
    bg.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, format!("Graph of {path}")));
    let plot = Rect::from_min_max(rect.min + Vec2::new(52.0, 8.0), rect.max - Vec2::new(10.0, 20.0));
    let samples = (plot.width() as usize / 2).clamp(32, 1200);
    let curve: Vec<(f64, Vec<f64>)> = (0..=samples)
        .map(|i| {
            let t = duration * i as f64 / samples as f64;
            (t, layer.value_at(path, t).map(|v| dims(&v)).unwrap_or_default())
        })
        .collect();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, v) in &curve {
        for x in v {
            lo = lo.min(*x * mul);
            hi = hi.max(*x * mul);
        }
    }
    for k in &keys {
        for x in &k.value {
            lo = lo.min(*x * mul);
            hi = hi.max(*x * mul);
        }
    }
    if !lo.is_finite() {
        return;
    }
    let span = (hi - lo).max(1.0);
    let (lo, hi) = (lo - span * 0.12, hi + span * 0.12);
    let to_x = |t: f64| plot.left() + (t / duration) as f32 * plot.width();
    let to_y = |v: f64| plot.bottom() - ((v * mul - lo) / (hi - lo)) as f32 * plot.height();
    let from_x = |x: f32| ((x - plot.left()) / plot.width()) as f64 * duration;
    let from_y = |y: f32| (lo + ((plot.bottom() - y) / plot.height()) as f64 * (hi - lo)) / mul;
    let pt = |t: f64, v: f64| Pos2::new(to_x(t), to_y(v));

    let painter = ui.painter_at(rect);
    painter.rect_filled(plot, 2.0, Color32::from_gray(24));
    // Value grid with labels.
    let step = nice_step((hi - lo) / 5.0);
    let mut g = (lo / step).ceil() * step;
    while g <= hi {
        let y = plot.bottom() - ((g - lo) / (hi - lo)) as f32 * plot.height();
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, Color32::from_gray(38)),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{}{suffix}", trim_num(g)),
            egui::FontId::monospace(10.0),
            Color32::from_gray(120),
        );
        g += step;
    }
    // Seconds along the bottom.
    for s in 0..=duration.floor() as i64 {
        let x = to_x(s as f64);
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, Color32::from_gray(32)),
        );
        painter.text(
            Pos2::new(x, plot.bottom() + 3.0),
            egui::Align2::CENTER_TOP,
            format!("{s}s"),
            egui::FontId::monospace(10.0),
            Color32::from_gray(120),
        );
    }
    let color_of = |d: usize| if n_dims == 1 { ACCENT } else { DIM_COLORS[d % 2] };
    // Curves.
    for d in 0..n_dims {
        let color = color_of(d);
        let pts: Vec<Pos2> = curve.iter().filter_map(|(t, v)| v.get(d).map(|x| pt(*t, *x))).collect();
        painter.add(egui::Shape::line(pts, Stroke::new(2.0, color)));
    }
    // Playhead.
    let px = to_x(app.time);
    painter.line_segment([Pos2::new(px, plot.top()), Pos2::new(px, plot.bottom())], Stroke::new(1.0, ACCENT));

    // Handles and keyframes (interactive: handles win over keys).
    // Hit targets, in priority order (handles first).
    let mut targets: Vec<(Pos2, f32, GraphDrag)> = Vec::new();
    for (i, w) in keys.windows(2).enumerate() {
        let (a, b) = (&w[0], &w[1]);
        let Some([x1, y1, x2, y2]) = controls(a.easing) else { continue };
        for d in 0..n_dims {
            let dv = b.value[d] - a.value[d];
            if dv.abs() < 1e-9 {
                continue;
            }
            let color = color_of(d);
            let dt = b.time - a.time;
            let h_out = pt(a.time + x1 * dt, a.value[d] + y1 * dv);
            let h_in = pt(a.time + x2 * dt, a.value[d] + y2 * dv);
            let (ka, kb) = (pt(a.time, a.value[d]), pt(b.time, b.value[d]));
            let s = Stroke::new(1.0, color.gamma_multiply(0.7));
            painter.line_segment([ka, h_out], s);
            painter.line_segment([kb, h_in], s);
            for (h, out) in [(h_out, true), (h_in, false)] {
                painter.circle(h, HANDLE_R, Color32::from_gray(30), Stroke::new(1.5, color));
                targets.push((h, HANDLE_R, GraphDrag::Handle { time: keys[i].time, out, dim: d }));
            }
        }
    }
    for k in &keys {
        for d in 0..n_dims {
            let p = pt(k.time, k.value[d]);
            let selected = app.selected_keys.iter().any(|s| s.is(layer.id, path, k.time));
            let fill = if selected { Color32::WHITE } else { Color32::from_rgb(250, 200, 60) };
            let r = KEY_R;
            painter.add(egui::Shape::convex_polygon(
                vec![p + Vec2::new(0.0, -r), p + Vec2::new(r, 0.0), p + Vec2::new(0.0, r), p + Vec2::new(-r, 0.0)],
                fill,
                Stroke::new(1.0, Color32::from_black_alpha(180)),
            ));
            targets.push((p, KEY_R, GraphDrag::Key { time: k.time, dim: d }));
        }
    }
    let hit = |q: Option<Pos2>| {
        let q = q?;
        targets.iter().find(|(p, r, _)| (q - *p).length() <= r + 3.0).map(|t| t.2)
    };
    // Drags start where the button went down (egui reports them a few px later).
    let origin = ui.input(|i| i.pointer.press_origin());
    let hovered = if bg.drag_started() { hit(origin) } else { hit(ui.input(|i| i.pointer.hover_pos())) };
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // Interaction.
    if bg.drag_started() {
        app.graph.drag = hovered;
        if let Some(GraphDrag::Key { time, .. }) = hovered {
            app.selected_keys = vec![crate::app::KeySel { layer: layer.id, path, time }];
        }
        if hovered.is_some() {
            app.new_gesture();
        }
    }
    if bg.clicked() {
        match hovered {
            Some(GraphDrag::Key { time, .. }) => {
                app.selected_keys = vec![crate::app::KeySel { layer: layer.id, path, time }];
            }
            _ => {
                if let Some(p) = bg.interact_pointer_pos() {
                    app.playing = false;
                    app.set_time(from_x(p.x).clamp(0.0, duration));
                }
            }
        }
    }
    if bg.dragged()
        && let (Some(drag), Some(p)) = (app.graph.drag, bg.interact_pointer_pos())
    {
        match drag {
            GraphDrag::Key { time, dim } => {
                let value_only = ui.input(|i| i.modifiers.shift);
                let new_time = if value_only { time } else { ((from_x(p.x) * fps).round() / fps).clamp(0.0, duration) };
                let Ok(current) = layer.value_at(path, time) else { return };
                let value = with_dim(&current, dim, from_y(p.y));
                let mut cmds = Vec::new();
                let taken = keys.iter().any(|k| (k.time - new_time).abs() < 1e-6 && (k.time - time).abs() > 1e-6);
                let t = if (new_time - time).abs() > 1e-6 && !taken {
                    cmds.push(Command::MoveKeyframe { layer: layer.id, path, from: time, to: new_time });
                    new_time
                } else {
                    time
                };
                cmds.push(Command::SetKeyframe { layer: layer.id, path, time: t, value, easing: None });
                if app.exec_merged("Edit keyframe", Command::batch(cmds)) {
                    app.graph.drag = Some(GraphDrag::Key { time: t, dim });
                    app.selected_keys = vec![crate::app::KeySel { layer: layer.id, path, time: t }];
                }
            }
            GraphDrag::Handle { time, out, dim } => {
                let Some(i) = keys.iter().position(|k| (k.time - time).abs() < 1e-6) else { return };
                let (a, b) = (&keys[i], &keys[(i + 1).min(keys.len() - 1)]);
                let (dt, dv) = (b.time - a.time, b.value[dim] - a.value[dim]);
                let Some([mut x1, mut y1, mut x2, mut y2]) = controls(a.easing) else { return };
                if dt <= 0.0 || dv.abs() < 1e-9 {
                    return;
                }
                let x = ((from_x(p.x) - a.time) / dt).clamp(0.0, 1.0);
                let y = (from_y(p.y) - a.value[dim]) / dv;
                if out {
                    (x1, y1) = (x, y);
                } else {
                    (x2, y2) = (x, y);
                }
                let easing = Easing::Bezier { x1, y1, x2, y2 };
                app.exec_merged("Edit easing", Command::SetEasing { layer: layer.id, path, time, easing });
            }
        }
    }
    if bg.drag_stopped() {
        app.graph.drag = None;
    }
}

/// A round grid step (1, 2 or 5 × 10ⁿ) near `raw`.
fn nice_step(raw: f64) -> f64 {
    let p = 10f64.powf(raw.max(1e-9).log10().floor());
    let m = raw / p;
    p * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_steps_are_round() {
        assert_eq!(nice_step(0.9), 1.0);
        assert_eq!(nice_step(23.0), 20.0);
        assert_eq!(nice_step(40.0), 50.0);
        assert_eq!(trim_num(12.50), "12.5");
        assert_eq!(controls(Easing::Linear).unwrap()[0], 1.0 / 3.0);
    }
}
