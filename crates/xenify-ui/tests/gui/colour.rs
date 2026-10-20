//! The colour panel's colour models and gradient editor, and the Gradient tool.

use super::common::*;
use egui::Key;
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::FakeLog;

fn drag(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) {
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * (i as f32 / 8.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(2);
}

fn fill_gradient(h: &Harness, id: xenify_doc::Id) -> Option<xenify_doc::Gradient> {
    let layer = h.state().project().layer(id).ok()?;
    Some(layer.shapes().first()?.fill.as_ref()?.gradient.as_ref()?.value_at(h.state().time))
}

#[test]
fn the_colour_panel_shows_colour_models_and_no_stroke_width() {
    let mut h = harness(FakeLog::default());
    h.run_steps(2);
    // A tab behind Properties by default.
    xenify_ui::testing::show_panel(h.state_mut(), "Colour");
    h.run_steps(2);
    for model in ["RGB", "HSV", "HSL", "CMYK", "Grey"] {
        h.get_by_label(model);
    }
    assert!(h.query_by_label("Stroke width").is_none(), "stroke width lives in the control bar");
    h.get_by_label("CMYK").click();
    h.run_steps(2);
    for ink in ["Cyan", "Magenta", "Yellow", "Black", "Alpha"] {
        h.get_by_label(ink);
    }
}

#[test]
fn the_gradient_tool_drags_out_a_gradient_and_its_ends_move() {
    let mut h = harness(FakeLog::default());
    let square = h.state().comp().layers.iter().find(|l| l.meta.name == "Square").expect("demo layer").id;
    assert!(fill_gradient(&h, square).is_none());
    let b = xenify_ui::testing::world_box(h.state(), square);
    let at = |h: &Harness, fx: f64, fy: f64| {
        h.state().viewport.comp_to_screen(kurbo::Point::new(b.x0 + b.width() * fx, b.y0 + b.height() * fy))
    };
    let (from, to, lower) = (at(&h, 0.2, 0.5), at(&h, 0.8, 0.5), at(&h, 0.8, 0.9));
    h.key_press(Key::G);
    h.run_steps(2);
    drag(&mut h, from, to);
    let g = fill_gradient(&h, square).expect("a gradient");
    assert_eq!(g.kind, xenify_doc::GradientKind::Linear);
    assert_eq!(g.stops.len(), 2);
    let d = g.end - g.start;
    assert!(d.x > 0.0 && d.y.abs() < d.x * 0.05, "left to right: {d:?}");
    // The colour panel (and Properties) edit its stops.
    assert!(h.query_all_by_label("Colour stop 2").count() >= 1);

    // Dragging the end handle moves only the end.
    drag(&mut h, to, lower);
    let moved = fill_gradient(&h, square).unwrap();
    assert!((moved.start - g.start).hypot() < 1e-6, "start stays");
    assert!(moved.end.y > g.end.y + 1.0, "end moved down: {:?} → {:?}", g.end, moved.end);
    assert_eq!(moved.stops, g.stops, "colours kept");
}

#[test]
fn a_gradient_moves_on_its_own_track_without_keying_its_colours() {
    use xenify_doc::{PropPath, ShapeField};
    let mut h = harness(FakeLog::default());
    let square = h.state().comp().layers.iter().find(|l| l.meta.name == "Square").expect("demo layer").id;
    let b = xenify_ui::testing::world_box(h.state(), square);
    let at = |h: &Harness, fx: f64, fy: f64| {
        h.state().viewport.comp_to_screen(kurbo::Point::new(b.x0 + b.width() * fx, b.y0 + b.height() * fy))
    };
    let (from, to) = (at(&h, 0.2, 0.5), at(&h, 0.8, 0.5));
    h.key_press(Key::G);
    h.run_steps(2);
    drag(&mut h, from, to);
    let first = fill_gradient(&h, square).expect("a gradient");

    // At 1 s, with Auto Keying, drag the end handle (where the moving layer has it now).
    h.state_mut().auto_key = true;
    h.state_mut().set_time(1.0);
    h.run_steps(2);
    let to_comp = xenify_ui::testing::layer_to_comp(h.state(), square);
    let screen = |h: &Harness, p: kurbo::Vec2| h.state().viewport.comp_to_screen(to_comp * p.to_point());
    let (grab, drop) = (screen(&h, first.end), screen(&h, first.end + kurbo::Vec2::new(0.0, 30.0)));
    drag(&mut h, grab, drop);

    let layer = h.state().project().layer(square).unwrap().clone();
    let times = |f| layer.keyframe_times(PropPath::shape(0, f)).unwrap();
    assert_eq!(times(ShapeField::GradientEnd), vec![0.0, 1.0], "the end is keyed where it was and where it moved");
    assert!(times(ShapeField::GradientStart).is_empty(), "the start stays unanimated");
    assert!(times(ShapeField::GradientColors).is_empty(), "the colours stay unanimated");
    let moved = fill_gradient(&h, square).unwrap();
    assert!((moved.end - first.end).hypot() > 5.0 && moved.start == first.start, "{moved:?}");
    h.state_mut().set_time(0.0);
    h.run_steps(1);
    assert_eq!(fill_gradient(&h, square).unwrap(), first, "unchanged at the start");
    h.state_mut().set_time(0.5);
    h.run_steps(1);
    let mid = fill_gradient(&h, square).unwrap();
    let half = first.end.lerp(moved.end, 0.5);
    assert!((mid.end - half).hypot() < (moved.end - first.end).hypot() * 0.3, "moves in between: {mid:?}");

    // Properties lists the three tracks.
    h.state_mut().select_only(Some(square));
    h.run_steps(2);
    for label in ["Gradient Start", "Gradient End", "Gradient Colours"] {
        assert!(h.query_all_by_label(label).count() >= 1, "{label}");
    }
}

#[test]
fn clicking_the_active_swatch_again_opens_its_picker() {
    let mut h = harness(FakeLog::default());
    let fill = |h: &Harness| {
        h.get_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Fill"))
            .next()
            .unwrap()
            .rect()
            .center()
    };
    // Fill is active at the start: one click opens its picker.
    let at = fill(&h);
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        let button = egui::PointerButton::Primary;
        h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    h.get_by_label("Choose the fill colour");
}
