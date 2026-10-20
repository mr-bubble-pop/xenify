//! Easing curve editor: a CSS-style cubic-bézier box with two draggable handles.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use xenify_doc::{Easing, StepJump};

use crate::widgets::ACCENT;

/// Vertical range shown, so handles can overshoot (`y` outside 0..1) for bouncy curves.
const Y_MIN: f64 = -0.5;
const Y_MAX: f64 = 1.5;

/// The result of one frame of the editor.
pub struct CurveEdit {
    /// The edited easing, if a handle moved or a preset was picked.
    pub changed: Option<Easing>,
    /// A new drag began (start a new undo gesture).
    pub began: bool,
}

/// Draws the editor for `easing`. Hold/linear start from a straight curve.
pub fn easing_editor(ui: &mut egui::Ui, id: egui::Id, easing: Easing) -> CurveEdit {
    let (mut x1, mut y1, mut x2, mut y2) = match easing {
        Easing::Bezier { x1, y1, x2, y2 } => (x1, y1, x2, y2),
        _ => (1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0),
    };
    let mut out = CurveEdit { changed: None, began: false };
    let side = ui.available_width().clamp(160.0, 260.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(side, side * 1.25), Sense::hover());
    let pad = 10.0;
    let area = rect.shrink(pad);
    let to_screen = |x: f64, y: f64| {
        Pos2::new(
            area.left() + x as f32 * area.width(),
            area.bottom() - ((y - Y_MIN) / (Y_MAX - Y_MIN)) as f32 * area.height(),
        )
    };
    let from_screen = |p: Pos2| {
        (
            ((p.x - area.left()) / area.width()).clamp(0.0, 1.0) as f64,
            (Y_MIN + (area.bottom() - p.y) as f64 / area.height() as f64 * (Y_MAX - Y_MIN)).clamp(Y_MIN, Y_MAX),
        )
    };

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(24));
    let unit = Rect::from_two_pos(to_screen(0.0, 0.0), to_screen(1.0, 1.0));
    painter.rect_filled(unit, 0.0, Color32::from_gray(32));
    let grid = Stroke::new(1.0, Color32::from_gray(44));
    for i in 1..4 {
        let f = i as f64 / 4.0;
        painter.line_segment([to_screen(f, 0.0), to_screen(f, 1.0)], grid);
        painter.line_segment([to_screen(0.0, f), to_screen(1.0, f)], grid);
    }
    painter.line_segment([to_screen(0.0, 0.0), to_screen(1.0, 1.0)], Stroke::new(1.0, Color32::from_gray(60)));
    painter.text(
        to_screen(0.0, 1.0) + Vec2::new(3.0, 3.0),
        egui::Align2::LEFT_TOP,
        "value",
        egui::FontId::proportional(10.0),
        Color32::from_gray(110),
    );
    painter.text(
        to_screen(1.0, 0.0) - Vec2::new(3.0, 3.0),
        egui::Align2::RIGHT_BOTTOM,
        "time",
        egui::FontId::proportional(10.0),
        Color32::from_gray(110),
    );

    // Springs and linear() stops: show the real curve, edited below.
    if !easing.is_simple() && !matches!(easing, Easing::Steps { .. }) {
        let pts: Vec<Pos2> = (0..=120)
            .map(|i| {
                let u = i as f64 / 120.0;
                to_screen(u, easing.apply(u).clamp(Y_MIN, Y_MAX))
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(2.5, ACCENT)));
        ui.label(egui::RichText::new(short_css(&easing.css())).monospace().size(11.0).weak());
        ui.horizontal_wrapped(|ui| {
            if let Some(preset) = preset_picker(ui, Some(easing)) {
                out.began = true;
                out.changed = Some(preset);
            }
        });
        if let Some(e) = spring_editor(ui, easing) {
            out.changed = Some(e);
        }
        return out;
    }

    // Handles (drawn and dragged before the curve so the curve shows the new shape).
    let r = 7.0;
    for (i, (hx, hy)) in [(&mut x1, &mut y1), (&mut x2, &mut y2)].into_iter().enumerate() {
        let pos = to_screen(*hx, *hy);
        let resp = ui.interact(Rect::from_center_size(pos, Vec2::splat(r * 3.0)), id.with(i), Sense::drag());
        let (vx, vy) = (*hx, *hy);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Slider,
                true,
                format!("Curve handle {}: time {vx:.2}, value {vy:.2}", i + 1),
            )
        });
        if resp.drag_started() {
            out.began = true;
        }
        // Keyboard: Tab to a handle, arrows move it (Shift: further).
        if resp.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resp.id,
                    egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() },
                )
            });
            let (dx, dy) = ui.input(|inp| {
                let s = if inp.modifiers.shift { 0.1 } else { 0.01 };
                let k = |key| if inp.key_pressed(key) { s } else { 0.0 };
                (k(egui::Key::ArrowRight) - k(egui::Key::ArrowLeft), k(egui::Key::ArrowUp) - k(egui::Key::ArrowDown))
            });
            if dx != 0.0 || dy != 0.0 {
                out.began = true;
                *hx = ((*hx + dx) * 100.0).round() / 100.0;
                *hx = hx.clamp(0.0, 1.0);
                *hy = ((*hy + dy) * 100.0).round() / 100.0;
                *hy = hy.clamp(Y_MIN, Y_MAX);
                out.changed = Some(Easing::Bezier { x1: 0.0, y1: 0.0, x2: 0.0, y2: 0.0 });
            }
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (nx, ny) = from_screen(p);
            // Snap to 0 and 1 (and the grid) when close, which makes clean curves easy.
            let snap = |v: f64| {
                let g = (v * 20.0).round() / 20.0;
                if (v - g).abs() < 0.012 { g } else { v }
            };
            (*hx, *hy) = (snap(nx), snap(ny));
            out.changed = Some(Easing::Bezier { x1: 0.0, y1: 0.0, x2: 0.0, y2: 0.0 });
        }
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    let (p0, p3) = (to_screen(0.0, 0.0), to_screen(1.0, 1.0));
    let (p1, p2) = (to_screen(x1, y1), to_screen(x2, y2));
    let arm = Stroke::new(1.5, Color32::from_gray(150));
    painter.line_segment([p0, p1], arm);
    painter.line_segment([p3, p2], arm);
    let curve = egui::epaint::CubicBezierShape::from_points_stroke(
        [p0, p1, p2, p3],
        false,
        Color32::TRANSPARENT,
        Stroke::new(2.5, ACCENT),
    );
    painter.add(curve);
    for p in [p0, p3] {
        painter.circle_filled(p, 3.0, Color32::from_gray(200));
    }
    for p in [p1, p2] {
        painter.circle(p, r * 0.75, Color32::WHITE, Stroke::new(1.5, ACCENT));
    }

    if out.changed.is_some() {
        out.changed = Some(Easing::Bezier { x1, y1, x2, y2 });
    }
    ui.label(
        egui::RichText::new(format!("cubic-bezier({x1:.2}, {y1:.2}, {x2:.2}, {y2:.2})")).monospace().size(11.0).weak(),
    );
    ui.horizontal_wrapped(|ui| {
        if let Some(preset) = preset_picker(ui, Some(easing)) {
            out.began = true;
            out.changed = Some(preset);
        }
    });
    if let Some(steps) = steps_editor(ui, id, easing) {
        out.began |= !matches!(easing, Easing::Steps { .. });
        out.changed = Some(steps);
    }
    ui.horizontal(|ui| {
        if ui.selectable_label(false, "spring").on_hover_text("A physical spring: overshoots and settles").clicked() {
            out.began = true;
            out.changed = Easing::preset("spring");
        }
    });
    out
}

/// A long `linear(…)` shortened for display.
fn short_css(css: &str) -> String {
    if css.len() > 60 { format!("{}…)", &css[..57]) } else { css.to_string() }
}

/// Stiffness and damping for a spring easing.
fn spring_editor(ui: &mut egui::Ui, easing: Easing) -> Option<Easing> {
    let Easing::Spring { mut stiffness, mut damping } = easing else { return None };
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Stiffness");
        changed |= ui.add(egui::DragValue::new(&mut stiffness).range(10.0..=1000.0).speed(1.0)).changed();
        ui.label("Damping");
        changed |= ui.add(egui::DragValue::new(&mut damping).range(0.5..=100.0).speed(0.1)).changed();
    })
    .response
    .on_hover_text(
        "Stiffer springs wobble faster; less damping wobbles more (around 2×√stiffness there's no overshoot)",
    );
    changed.then_some(Easing::Spring { stiffness, damping })
}

/// CSS `steps(count, jump)`: a toggle, then the count and jump position.
fn steps_editor(ui: &mut egui::Ui, id: egui::Id, easing: Easing) -> Option<Easing> {
    let mut changed = None;
    ui.horizontal(|ui| {
        let on = matches!(easing, Easing::Steps { .. });
        if ui.selectable_label(on, "steps").on_hover_text("Jump between values in equal steps (CSS steps())").clicked()
            && !on
        {
            changed = Some(Easing::Steps { count: 4, jump: StepJump::End });
        }
        let Easing::Steps { mut count, mut jump } = easing else { return };
        let mut edited = ui.add(egui::DragValue::new(&mut count).range(1..=100).suffix(" steps")).changed();
        egui::ComboBox::from_id_salt(id.with("jump")).selected_text(jump.css()).show_ui(ui, |ui| {
            for j in StepJump::ALL {
                edited |= ui.selectable_value(&mut jump, j, j.css()).changed();
            }
        });
        if edited {
            changed = Some(Easing::Steps { count, jump });
        }
    });
    changed
}

/// A preset's display name: `ease_in_out_back` → "ease in out back".
pub fn preset_label(name: &str) -> String {
    name.replace('_', " ")
}

/// The easing presets as selectable labels: the common ones inline, the rest
/// under "More". Returns the preset picked this frame.
pub fn preset_picker(ui: &mut egui::Ui, current: Option<Easing>) -> Option<Easing> {
    let (common, more) = Easing::PRESETS.split_at(Easing::COMMON_PRESETS);
    let mut picked = None;
    for (name, preset) in common {
        if ui.selectable_label(current == Some(*preset), preset_label(name)).clicked() {
            picked = Some(*preset);
        }
    }
    let in_more = more.iter().any(|(_, p)| current == Some(*p));
    ui.menu_button(if in_more { "More ●" } else { "More" }, |ui| {
        for (name, preset) in more {
            if ui.selectable_label(current == Some(*preset), preset_label(name)).clicked() {
                picked = Some(*preset);
                ui.close();
            }
        }
    });
    picked
}
