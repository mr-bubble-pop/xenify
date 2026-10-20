//! Diagrams: the Connector tool, glued ends that follow their shapes, and
//! dragging a connector's end onto another shape.

use super::common::*;
use egui::Key;
use egui_kittest::kittest::Queryable;
use xenify_doc::{Color, Id, Layer, PortRef, ShapeItem, Vec2};
use xenify_ops::Command;
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

/// The app with an empty canvas (the demo's layers removed).
fn empty() -> Harness {
    let mut h = harness(FakeLog::default());
    let app = h.state_mut();
    let cmds = app.comp().layers.iter().rev().map(|l| Command::RemoveLayer { layer: l.id }).collect();
    assert!(app.exec("clear", Command::batch(cmds)));
    h.run_steps(2);
    h
}

/// Adds a box (on top of everything) centred at `at`; returns its id.
fn add_box(h: &mut Harness, name: &str, at: Vec2) -> Id {
    let app = h.state_mut();
    let id = Id(app.project().next_id);
    let comp = app.comp;
    let layer = Layer::shape(id, name, vec![ShapeItem::rect(Vec2::new(120.0, 80.0), Color::WHITE)]).with_position(at);
    assert!(app.exec("box", Command::AddLayer { comp, index: None, layer }));
    h.run_steps(2);
    id
}

fn screen(h: &Harness, p: Vec2) -> egui::Pos2 {
    h.state().viewport.comp_to_screen(p.to_point())
}

fn connectors(h: &Harness) -> Vec<(Id, xenify_doc::Connector)> {
    h.state().comp().layers.iter().filter_map(|l| Some((l.id, (**l.meta.connector.as_ref()?).clone()))).collect()
}

fn route_end(h: &Harness, id: Id) -> kurbo::Point {
    let app = h.state();
    let r = xenify_anim::connector_routes(app.project(), app.comp, app.time).into_iter().find(|r| r.id == id).unwrap();
    *r.points.last().unwrap()
}

#[test]
fn connector_tool_glues_a_line_between_two_shapes_that_follows_them() {
    let mut h = empty();
    let a = add_box(&mut h, "A", Vec2::new(300.0, 200.0));
    let b = add_box(&mut h, "B", Vec2::new(900.0, 200.0));
    h.key_press(Key::F);
    h.run_steps(2);
    assert_eq!(h.state().tool, xenify_ui::testing::Tool::Connector);
    let (from, to) = (screen(&h, Vec2::new(300.0, 200.0)), screen(&h, Vec2::new(900.0, 200.0)));
    drag(&mut h, from, to);
    let cs = connectors(&h);
    assert_eq!(cs.len(), 1, "one connector drawn");
    let (id, c) = &cs[0];
    assert_eq!(c.from.attach.map(|x| (x.layer, x.port)), Some((a, PortRef::Auto)));
    assert_eq!(c.to.attach.map(|x| (x.layer, x.port)), Some((b, PortRef::Auto)));
    assert_eq!(h.state().selected, Some(*id), "the new connector is selected");
    assert_eq!(route_end(&h, *id), kurbo::Point::new(840.0, 200.0));

    // Move B down with the Select tool: the line follows.
    h.key_press(Key::V);
    h.run_steps(2);
    let (p, q) = (screen(&h, Vec2::new(900.0, 200.0)), screen(&h, Vec2::new(900.0, 500.0)));
    drag(&mut h, p, q);
    let end = route_end(&h, *id);
    assert!((end.y - 200.0).abs() > 100.0, "the end moved with B: {end:?}");
}

#[test]
fn dragging_a_connector_end_onto_another_shape_reglues_it() {
    let mut h = empty();
    let a = add_box(&mut h, "A", Vec2::new(300.0, 200.0));
    let _b = add_box(&mut h, "B", Vec2::new(900.0, 200.0));
    let c = add_box(&mut h, "C", Vec2::new(900.0, 600.0));
    h.key_press(Key::F);
    h.run_steps(2);
    let (from, to) = (screen(&h, Vec2::new(300.0, 200.0)), screen(&h, Vec2::new(900.0, 200.0)));
    drag(&mut h, from, to);
    let (id, _) = connectors(&h)[0].clone();
    h.key_press(Key::V);
    h.run_steps(2);
    // The end handle sits where the line meets B.
    let end = route_end(&h, id);
    let (p, q) = (h.state().viewport.comp_to_screen(end), screen(&h, Vec2::new(900.0, 600.0)));
    drag(&mut h, p, q);
    let (_, conn) = connectors(&h)[0].clone();
    assert_eq!(conn.to.attach.map(|x| x.layer), Some(c));
    assert_eq!(conn.from.attach.map(|x| x.layer), Some(a));
    // One undo step for the whole drag.
    h.state_mut().undo();
    h.run_steps(1);
    assert_ne!(connectors(&h)[0].1.to.attach.map(|x| x.layer), Some(c));
}

#[test]
fn deleting_a_glued_shape_leaves_the_line_in_place() {
    let mut h = empty();
    add_box(&mut h, "A", Vec2::new(300.0, 200.0));
    let b = add_box(&mut h, "B", Vec2::new(900.0, 200.0));
    h.key_press(Key::F);
    h.run_steps(2);
    let (from, to) = (screen(&h, Vec2::new(300.0, 200.0)), screen(&h, Vec2::new(900.0, 200.0)));
    drag(&mut h, from, to);
    let app = h.state_mut();
    assert!(app.exec("delete", Command::RemoveLayer { layer: b }));
    let (_, c) = connectors(&h)[0].clone();
    assert!(c.to.attach.is_none());
    assert_eq!(c.to.point, Vec2::new(900.0, 200.0));
}

#[test]
fn shapes_panel_adds_library_shapes_with_labels() {
    let mut h = empty();
    xenify_ui::testing::show_panel(h.state_mut(), "Shapes");
    h.run_steps(3);
    h.get_by_label("Decision").click();
    h.run_steps(3);
    let app = h.state();
    let layer = app.comp().layers.last().expect("a shape was added");
    assert_eq!(layer.meta.name, "Decision");
    assert_eq!(layer.meta.label.as_ref().map(|l| l.text.as_str()), Some("Decision"));
    assert_eq!(app.selected, Some(layer.id));
    // The next one goes to the right of the selected shape.
    let first = layer.id;
    h.get_by_label("Process").click();
    h.run_steps(3);
    let app = h.state();
    let x = |id| app.project().layer(id).unwrap().transform.position.value_at(0.0).x;
    let second = app.comp().layers.last().unwrap().id;
    assert!(x(second) > x(first) + 100.0);
}

#[test]
fn a_diagram_symbol_opens_with_the_diagram_tools() {
    let mut h = harness(FakeLog::default());
    let main = h.state().comp;
    h.get_by_label("Canvas").click();
    h.run_steps(2);
    h.get_by_label("New Diagram Symbol").click();
    h.run_steps(3);
    let app = h.state();
    assert_ne!(app.comp, main, "the new diagram is open");
    assert_eq!(app.comp().settings.kind, xenify_doc::CompKind::Diagram);
    assert!(xenify_ui::testing::panel_open(app, "Shapes"), "the Shapes panel comes up");
    // Diagram toolbar: the Connector tool is there, the Sculpt tool isn't.
    assert!(h.query_by_label("Connector").is_some());
    assert!(h.query_by_label("Sculpt").is_none());
    // Place it back on the main canvas; double-clicking the copy opens it again.
    let diagram = h.state().comp;
    h.state_mut().open_comp(main);
    h.run_steps(2);
    h.state_mut().add_comp_layer(diagram);
    h.run_steps(2);
    assert!(
        h.state()
            .comp()
            .layers
            .iter()
            .any(|l| matches!(l.kind, xenify_doc::LayerKind::Precomp { comp, .. } if comp == diagram))
    );
}

#[test]
fn quick_connect_adds_a_joined_copy_next_to_the_shape() {
    let mut h = empty();
    let app = h.state_mut();
    let comp = app.comp;
    let mut settings = app.comp().settings.clone();
    xenify_ops::diagram::set_kind(&mut settings, xenify_doc::CompKind::Diagram);
    assert!(app.exec("diagram", Command::SetCompSettings { comp, settings }));
    let a = add_box(&mut h, "A", Vec2::new(300.0, 300.0));
    h.state_mut().select_only(Some(a));
    h.key_press(Key::V);
    h.run_steps(2);
    // The right-hand arrow sits just outside the box's right edge.
    let edge = screen(&h, Vec2::new(360.0, 300.0));
    let arrow = edge + egui::vec2(22.0, 0.0);
    h.hover_at(arrow);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: arrow,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    let layers = &h.state().comp().layers;
    assert_eq!(layers.len(), 3, "a copy and a connector");
    let copy = &layers[1];
    assert!(copy.transform.position.value_at(0.0).x > 400.0, "the copy is to the right");
    let conn = layers[2].meta.connector.as_ref().expect("joined");
    assert_eq!(conn.from.attach.map(|x| x.layer), Some(a));
    assert_eq!(conn.to.attach.map(|x| x.layer), Some(copy.id));
    assert_eq!(h.state().selected, Some(copy.id));
}
