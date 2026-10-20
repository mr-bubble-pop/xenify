//! Shapes as points: rectangles stay live until a point changes, corners
//! round from the canvas, paths that are still rectangles keep their
//! controls, and the extra shapes (gear, heart…).

use super::common::*;
use egui::{Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
use xenify_doc::primitives::{self, Primitive};
use xenify_doc::{Geometry, Id};
use xenify_ui::testing::{FakeLog, Tool};

fn click(h: &mut Harness, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
}

/// Drags with `modifiers` held throughout.
fn drag_with(h: &mut Harness, from: egui::Pos2, to: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(from);
    h.run_steps(1);
    let button = |pressed, pos| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
    h.event_modifiers(button(true, from), modifiers);
    h.run_steps(1);
    for i in 1..=8 {
        let p = from + (to - from) * (i as f32 / 8.0);
        h.event_modifiers(egui::Event::PointerMoved(p), modifiers);
        h.run_steps(1);
    }
    h.event_modifiers(button(false, to), modifiers);
    h.run_steps(3);
}

/// Draws a 200 × 200 shape with the current tool by clicking (the size dialog).
fn new_shape(h: &mut Harness) -> Id {
    let spot = h.state().viewport.screen_rect().left_top() + egui::vec2(300.0, 200.0);
    click(h, spot);
    h.get_by_label("Create").click();
    h.run_steps(3);
    *ids(h).last().unwrap()
}

fn geometry(h: &Harness, id: Id) -> Geometry {
    h.state().project().layer(id).unwrap().shapes()[0].geometry.clone()
}

/// Screen position of the rounding circle inside `corner` (0 = top-left, clockwise).
fn corner_circle(h: &Harness, id: Id, corner: usize) -> egui::Pos2 {
    let b = xenify_ui::testing::world_box(h.state(), id);
    let (p, d) = match corner {
        0 => ((b.x0, b.y0), egui::vec2(1.0, 1.0)),
        1 => ((b.x1, b.y0), egui::vec2(-1.0, 1.0)),
        2 => ((b.x1, b.y1), egui::vec2(-1.0, -1.0)),
        _ => ((b.x0, b.y1), egui::vec2(1.0, -1.0)),
    };
    // Sharp corners: 16 screen points in along the diagonal.
    h.state().viewport.comp_to_screen(p.into()) + d.normalized() * 16.0
}

#[test]
fn corner_circles_round_every_corner_or_just_one_with_alt() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::R);
    h.run_steps(2);
    let rect = new_shape(&mut h);
    h.key_press(Key::A);
    h.run_steps(2);
    h.state_mut().select_only(Some(rect));
    h.run_steps(2);

    // Every corner: the rectangle stays a live rectangle with a radius.
    let from = corner_circle(&h, rect, 0);
    drag_with(&mut h, from, from + egui::vec2(15.0, 15.0), Modifiers::NONE);
    let Geometry::Rect { corner_radius, .. } = geometry(&h, rect) else { panic!("still a rectangle") };
    let r = corner_radius.value_at(0.0);
    assert!(r > 5.0, "rounded: {r}");

    // Alt: just the bottom-right one. It becomes a path that is still a rectangle.
    let r_at = |h: &Harness| match geometry(h, rect) {
        Geometry::Path { path } => primitives::recognize(&path.value_at(0.0)),
        _ => None,
    };
    let b = xenify_ui::testing::world_box(h.state(), rect);
    let z = h.state().viewport.comp_to_screen((b.x1, b.y1).into());
    // A rounded corner's circle sits at the corner's centre, r in on both axes.
    let zoom = (h.state().viewport.comp_to_screen((1.0, 0.0).into()).x
        - h.state().viewport.comp_to_screen((0.0, 0.0).into()).x) as f64;
    let from = z - egui::vec2((r * zoom) as f32, (r * zoom) as f32);
    drag_with(&mut h, from, from - egui::vec2(12.0, 12.0), Modifiers::ALT);
    match r_at(&h) {
        Some(Primitive::Rect { radii, .. }) => {
            assert!((radii[0] - r).abs() < 0.5 && (radii[1] - r).abs() < 0.5, "{radii:?}");
            assert!(radii[2] > r + 5.0, "only the bottom-right grew: {radii:?}");
        }
        other => panic!("a path that is still a rectangle: {other:?}"),
    }
    h.get_by_label("Still a rectangle");
    // One undo step puts it back to the live rectangle.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert!(matches!(geometry(&h, rect), Geometry::Rect { .. }));
}

#[test]
fn moving_a_point_makes_a_path_and_keeps_the_animation() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::R);
    h.run_steps(2);
    let rect = new_shape(&mut h);
    // Animate its size.
    let path = xenify_doc::PropPath::shape(0, xenify_doc::ShapeField::Size);
    let key = |t: f64, w: f64| xenify_ops::Command::SetKeyframe {
        layer: rect,
        path,
        time: t,
        value: xenify_doc::Value::Vec2(xenify_doc::Vec2::new(w, 200.0)),
        easing: None,
    };
    h.state_mut().exec("keys", xenify_ops::Command::batch(vec![key(0.0, 200.0), key(1.0, 400.0)]));
    h.key_press(Key::A);
    h.run_steps(2);
    h.state_mut().select_only(Some(rect));
    h.run_steps(2);
    let b = xenify_ui::testing::world_box(h.state(), rect);
    let corner = h.state().viewport.comp_to_screen((b.x0, b.y0).into());
    drag_with(&mut h, corner, corner - egui::vec2(20.0, 0.0), Modifiers::NONE);
    let Geometry::Path { path } = geometry(&h, rect) else { panic!("a path now") };
    assert_eq!(path.keyframes().len(), 2, "the size animation became path keyframes");
    let w1 = xenify_doc::kurbo::Shape::bounding_box(&path.value_at(1.0)).width();
    assert!(w1 > 390.0, "still grows: {w1}");
}

#[test]
fn star_corners_round_together() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::Y);
    h.run_steps(2);
    h.state_mut().tool_options.figure = xenify_ops::draw::Figure::Star;
    h.state_mut().tool_options.polygon_sides = 5;
    let star = new_shape(&mut h);
    let points = |h: &Harness| match geometry(h, star) {
        Geometry::Path { path } => xenify_doc::bezier::point_count(&path.value_at(0.0)),
        g => xenify_doc::primitives::live_path(&g, 0.0).map_or(0, |p| xenify_doc::bezier::point_count(&p)),
    };
    assert!(matches!(geometry(&h, star), Geometry::Figure { .. }), "drawn live");
    assert_eq!(points(&h), 10);
    h.key_press(Key::A);
    h.run_steps(2);
    h.state_mut().select_only(Some(star));
    h.run_steps(2);
    // The top point's circle: straight down from it.
    let b = xenify_ui::testing::world_box(h.state(), star);
    let top = h.state().viewport.comp_to_screen((b.center().x, b.y0).into());
    drag_with(&mut h, top + egui::vec2(0.0, 16.0), top + egui::vec2(0.0, 30.0), Modifiers::NONE);
    assert_eq!(points(&h), 20, "every point became a rounded pair");
}

#[test]
fn the_shapes_list_has_gears_hearts_and_rounded_rectangles() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::Y);
    h.run_steps(2);
    h.state_mut().tool_options.figure = xenify_ops::draw::Figure::Gear;
    let gear = new_shape(&mut h);
    let l = h.state().project().layer(gear).unwrap();
    assert!(l.meta.name.starts_with("Gear"), "{}", l.meta.name);
    let outline = xenify_doc::primitives::live_path(&l.shapes()[0].geometry, 0.0).expect("a live gear");
    let subpaths = outline.elements().iter().filter(|e| matches!(e, xenify_doc::kurbo::PathEl::MoveTo(_))).count();
    assert_eq!(subpaths, 2, "teeth and a hole");

    h.state_mut().tool_options.figure = xenify_ops::draw::Figure::Heart;
    let heart = new_shape(&mut h);
    assert!(h.state().project().layer(heart).unwrap().meta.name.starts_with("Heart"));

    h.key_press(Key::R);
    h.run_steps(2);
    h.state_mut().tool_options.corner_radius = 24.0;
    let rounded = new_shape(&mut h);
    let Geometry::Rect { corner_radius, .. } = geometry(&h, rounded) else { panic!() };
    assert_eq!(corner_radius.value_at(0.0), 24.0);
    assert_eq!(h.state().tool, Tool::Rect);
}

#[test]
fn a_rectangles_corners_can_be_set_one_by_one() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::R);
    h.run_steps(2);
    let rect = new_shape(&mut h);
    h.key_press(Key::V);
    h.run_steps(2);
    h.state_mut().select_only(Some(rect));
    // Tall enough for Properties to show the shape's rows without scrolling.
    h.set_size(egui::vec2(1600.0, 2600.0));
    h.run_steps(3);
    // "Each corner": top-left, top-right, bottom-right, bottom-left.
    let row = h.get_by_label("Each corner").rect();
    let mut fields: Vec<egui::Rect> = h
        .get_all_by(|n| n.role() == egui::accesskit::Role::SpinButton)
        .map(|n| n.rect())
        .filter(|r| (r.center().y - row.center().y).abs() < 4.0)
        .collect();
    fields.sort_by(|a, b| a.left().total_cmp(&b.left()));
    assert_eq!(fields.len(), 4);
    let field = fields[1].center();
    // Dragging a number field right raises it.
    drag_with(&mut h, field, field + egui::vec2(40.0, 0.0), Modifiers::NONE);
    match geometry(&h, rect) {
        Geometry::Path { path } => match primitives::recognize(&path.value_at(0.0)) {
            Some(Primitive::Rect { radii, .. }) => {
                assert!(radii[1] > 5.0 && radii[0] == 0.0 && radii[2] == 0.0 && radii[3] == 0.0, "{radii:?}")
            }
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_select_tool_rounds_a_rectangles_corners_too() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::R);
    h.run_steps(2);
    let rect = new_shape(&mut h);
    h.key_press(Key::V);
    h.run_steps(2);
    h.state_mut().select_only(Some(rect));
    h.run_steps(2);
    let before = h.state().project().layer(rect).unwrap().transform.position.value_at(0.0);
    let from = corner_circle(&h, rect, 1);
    drag_with(&mut h, from, from + egui::vec2(-15.0, 15.0), Modifiers::NONE);
    let Geometry::Rect { corner_radius, .. } = geometry(&h, rect) else { panic!("still a rectangle") };
    assert!(corner_radius.value_at(0.0) > 5.0, "rounded");
    let after = h.state().project().layer(rect).unwrap().transform.position.value_at(0.0);
    assert_eq!(before, after, "the drag rounded corners instead of moving the layer");
    assert_eq!(h.state().tool, Tool::Select);
}
