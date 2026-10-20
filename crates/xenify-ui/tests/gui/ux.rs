//! UX follow-ups: small behaviours that make the app feel right.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::{PropPath, Value};
use xenify_ui::testing::FakeLog;

#[test]
fn canvas_settings_warn_about_cut_keyframes_and_take_enter() {
    let mut h = harness(FakeLog::default());
    let last = h
        .state()
        .comp()
        .layers
        .iter()
        .flat_map(|l| l.property_paths().into_iter().flat_map(move |p| l.keyframe_times(p).unwrap()))
        .fold(0.0f64, f64::max);
    assert!(last > 0.5, "the demo has keyframes");
    xenify_ui::testing::open_comp_settings(h.state_mut(), last / 2.0);
    h.run_steps(10);
    assert!(h.query_by_label_contains("cuts off").is_some());
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(!xenify_ui::testing::dialog_open(h.state()), "Enter is OK");
    assert!((h.state().comp().settings.duration - last / 2.0).abs() < 1e-9);
}

#[test]
fn the_isnt_animated_hint_shows_once_per_property() {
    let mut h = harness(FakeLog::default());
    let layer = h
        .state()
        .comp()
        .layers
        .iter()
        .find(|l| l.property(PropPath::Opacity).is_ok_and(|p| !p.is_animated()))
        .expect("a layer with still opacity")
        .id;
    h.state_mut().auto_key = false;
    h.state_mut().edit_value(layer, PropPath::Opacity, Value::Scalar(0.5));
    assert!(h.state().status.contains("isn't animated"), "{}", h.state().status);
    h.state_mut().status.clear();
    h.state_mut().edit_value(layer, PropPath::Opacity, Value::Scalar(0.4));
    assert!(!h.state().status.contains("isn't animated"), "said once is enough");
}

#[test]
fn blend_sits_by_opacity_and_skew_is_folded_under_more() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1600.0, 1400.0));
    let id = ids(&h)[2];
    h.state_mut().select_only(Some(id));
    h.run_steps(3);
    assert!(h.query_by_label("Skew").is_none(), "folded away");
    h.get_by_label("More").click();
    h.run_steps(2);
    assert!(h.query_by_label("Skew").is_some());
    h.get_by_label("Normal ⏷").click();
    h.run_steps(2);
    h.get_by_label("Multiply").click();
    h.run_steps(2);
    assert_eq!(h.state().project().layer(id).unwrap().meta.blend, xenify_doc::BlendMode::Multiply);
}

#[test]
fn hovering_outlines_what_a_click_would_pick() {
    use xenify_doc::{Color, Id, Layer, ShapeItem, Vec2};
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1800.0, 1100.0));
    h.run_steps(3);
    // Two squares in a group, well clear of everything else.
    let comp = h.state().comp;
    let a = Layer::shape(Id(900), "A", vec![ShapeItem::rect(Vec2::new(80.0, 80.0), Color::WHITE)])
        .with_position(Vec2::new(1000.0, 560.0));
    let b = Layer::shape(Id(901), "B", vec![ShapeItem::rect(Vec2::new(80.0, 80.0), Color::WHITE)])
        .with_position(Vec2::new(1110.0, 560.0));
    for layer in [a, b] {
        h.state_mut().exec("add", xenify_ops::Command::AddLayer { comp, index: None, layer });
    }
    h.state_mut().select_only(Some(Id(900)));
    h.state_mut().toggle_selection(Id(901));
    h.state_mut().group_selection();
    let group = h.state().selected.expect("the group");
    h.state_mut().select_only(None);
    h.run_steps(2);
    let at = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(1000.0, 560.0));
    h.hover_at(at);
    h.run_steps(2);
    assert_eq!(h.state().viewport.hover_pick.get(), Some(group), "the whole group, as a click would");
    h.input_mut().modifiers = egui::Modifiers::COMMAND;
    h.hover_at(at + egui::vec2(1.0, 0.0));
    h.run_steps(2);
    assert_eq!(h.state().viewport.hover_pick.get(), Some(Id(900)), "Ctrl reaches inside");
}

#[test]
fn dragging_shows_live_numbers_by_the_pointer() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1800.0, 1100.0));
    h.run_steps(3);
    let ball = h.state().comp().layers.iter().find(|l| l.meta.name == "Ball").unwrap().clone();
    let p = ball.transform.position.value_at(h.state().time);
    let from = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(p.x, p.y));
    let to = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(p.x + 100.0, p.y));
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    let label = xenify_ui::testing::live_numbers(h.state()).expect("a label while moving");
    assert!(label.starts_with("Δx +100"), "{label}");
    h.drop_at(to);
    h.run_steps(2);
    assert!(xenify_ui::testing::live_numbers(h.state()).is_none(), "gone after the drag");
}

#[test]
fn shift_keeps_a_move_to_one_axis() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1800.0, 1100.0));
    h.run_steps(3);
    let ball = h.state().comp().layers.iter().find(|l| l.meta.name == "Ball").unwrap().id;
    let p0 = h.state().project().layer(ball).unwrap().transform.position.value_at(h.state().time);
    let from = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(p0.x, p0.y));
    let to = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(p0.x + 120.0, p0.y + 40.0));
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        // (Shift at the start of a drag box-selects; press it once moving.)
        if i == 3 {
            h.input_mut().modifiers = egui::Modifiers::SHIFT | egui::Modifiers::COMMAND;
        }
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(2);
    h.input_mut().modifiers = egui::Modifiers::NONE;
    let p1 = h.state().project().layer(ball).unwrap().transform.position.value_at(h.state().time);
    assert!((p1.y - p0.y).abs() < 1e-6, "no vertical move: {p0:?} → {p1:?}");
    assert!((p1.x - p0.x - 120.0).abs() < 1.0, "the horizontal part, unsnapped: {p0:?} → {p1:?}");
}

#[test]
fn a_diagram_size_makes_a_diagram_canvas() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1600.0, 1100.0));
    xenify_ui::testing::open_launch_dialog(h.state_mut());
    h.run_steps(10);
    // The size tab (Kind has a "Diagram" radio button too).
    h.get_by(|n| n.label().as_deref() == Some("Diagram") && n.role() != egui::accesskit::Role::RadioButton).click();
    h.run_steps(2);
    h.get_by_label("Whiteboard").click();
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(4);
    let s = &h.state().comp().settings;
    assert_eq!((s.width, s.height), (2400, 1600));
    assert_eq!(s.kind, xenify_doc::CompKind::Diagram);
}

#[test]
fn dragging_a_connector_moves_its_loose_ends() {
    use xenify_doc::{ConnectorEnd, Vec2};
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1800.0, 1100.0));
    h.run_steps(3);
    let comp = h.state().comp;
    let (cmd, id) = xenify_ops::diagram::add_connector(
        h.state().project(),
        comp,
        ConnectorEnd::free(Vec2::new(700.0, 520.0)),
        ConnectorEnd::free(Vec2::new(1000.0, 520.0)),
    )
    .unwrap();
    h.state_mut().exec("add", cmd);
    h.state_mut().select_only(None);
    h.run_steps(2);
    let from = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(850.0, 520.0));
    let to = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(850.0, 600.0));
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(2);
    let c = h.state().project().layer(id).unwrap().meta.connector.clone().unwrap();
    assert!((c.from.point.y - 600.0).abs() < 2.0 && (c.to.point.y - 600.0).abs() < 2.0, "{:?} {:?}", c.from, c.to);
    assert!((c.from.point.x - 700.0).abs() < 2.0, "moved straight down");
    assert_eq!(h.state().selected, Some(id));
    assert!(xenify_ui::testing::live_numbers(h.state()).is_none());
}

#[test]
fn drawn_stars_stay_live() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1800.0, 1400.0));
    h.run_steps(3);
    h.state_mut().set_tool(xenify_ui::testing::Tool::Polygon);
    h.state_mut().tool_options.figure = xenify_ops::draw::Figure::Star;
    let a = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(900.0, 450.0));
    let b = h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(1100.0, 650.0));
    h.drag_at(a);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(a + (b - a) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(b);
    h.run_steps(3);
    let id = h.state().selected.expect("the new star");
    let layer = h.state().project().layer(id).unwrap().clone();
    let xenify_doc::LayerKind::Shape { shapes } = &layer.kind else { panic!() };
    assert!(
        matches!(
            shapes[0].geometry,
            xenify_doc::Geometry::Figure { kind: xenify_doc::primitives::FigureKind::Star, .. }
        ),
        "{:?}",
        shapes[0].geometry
    );
    // (The tool options have a "Points" too.)
    assert!(h.get_all_by_label("Points").count() >= 2, "its points are a property");
}
