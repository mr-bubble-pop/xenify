//! Fixes from the UX review: deleting a multi-selection, the pen's keys and
//! double-click, arrow-key nudging, undo steps.

use super::common::*;
use egui::{Key, Modifiers, PointerButton};
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
}

fn spot(h: &Harness, x: f32, y: f32) -> egui::Pos2 {
    h.state().viewport.screen_rect().left_top() + egui::vec2(x, y)
}

fn path_points(h: &Harness, id: xenify_doc::Id) -> usize {
    match &h.state().project().layer(id).unwrap().shapes()[0].geometry {
        xenify_doc::Geometry::Path { path } => xenify_doc::bezier::point_count(&path.value_at(0.0)),
        _ => 0,
    }
}

#[test]
fn delete_removes_every_selected_layer_in_one_step() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h);
    h.state_mut().select_only(Some(before[0]));
    h.state_mut().toggle_selection(before[1]);
    h.state_mut().toggle_selection(before[2]);
    h.key_press(Key::Delete);
    h.run_steps(2);
    assert_eq!(ids(&h), vec![before[3]]);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(ids(&h), before);
}

#[test]
fn the_pen_finishes_on_double_click_and_backspace_takes_back_a_point() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h).len();
    h.key_press(Key::P);
    h.run_steps(2);
    for x in [200.0, 300.0, 400.0] {
        let at = spot(&h, x, 150.0);
        click(&mut h, at);
        h.run_steps(20); // not a double-click
    }
    // Backspace takes back the last point, not the layer below.
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), before, "nothing deleted");
    // A double-click adds its point and finishes.
    let end = spot(&h, 350.0, 250.0);
    click(&mut h, end);
    click(&mut h, end);
    h.run_steps(3);
    assert_eq!(ids(&h).len(), before + 1, "one new path");
    let path = *ids(&h).last().unwrap();
    assert_eq!(path_points(&h, path), 3, "200, 300 and the double-clicked point");
    assert_eq!(h.state().tool, Tool::Pen);
}

#[test]
fn escape_keeps_the_pen_path_drawn_so_far() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h).len();
    h.key_press(Key::P);
    h.run_steps(2);
    for x in [200.0, 320.0] {
        let at = spot(&h, x, 150.0);
        click(&mut h, at);
        h.run_steps(20);
    }
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), before + 1, "kept as an open line");
    assert_eq!(path_points(&h, *ids(&h).last().unwrap()), 2);
}

#[test]
fn arrow_keys_nudge_the_selection_and_step_frames_without_one() {
    let mut h = harness(FakeLog::default());
    h.state_mut().auto_key = false;
    let id = ids(&h)[2];
    h.state_mut().select_only(Some(id));
    let pos = |h: &Harness| h.state().project().layer(id).unwrap().transform.position.value_at(0.0);
    let start = pos(&h);
    h.key_press(Key::ArrowRight);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(pos(&h) - start, xenify_doc::Vec2::new(1.0, 10.0));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(pos(&h), start, "the nudges were one undo step");
    // Nothing selected: frames.
    h.key_press(Key::Escape);
    h.run_steps(2);
    let t = h.state().time;
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert!(h.state().time > t);
    h.key_press(Key::Period);
    h.run_steps(2);
    assert!(h.state().time > t + 1.5 / h.state().fps());
}

#[test]
fn a_new_click_starts_a_new_undo_step() {
    let mut h = harness(FakeLog::default());
    h.state_mut().auto_key = false;
    let id = ids(&h)[2];
    h.state_mut().select_only(Some(id));
    let pos = |h: &Harness| h.state().project().layer(id).unwrap().transform.position.value_at(0.0);
    let start = pos(&h);
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    // A click somewhere (e.g. into another field) ends that step.
    click(&mut h, egui::pos2(5.0, 995.0));
    h.run_steps(2);
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(pos(&h) - start, xenify_doc::Vec2::new(1.0, 0.0), "only the second nudge undone");
}

#[test]
fn properties_edit_every_selected_layer_keeping_their_spacing() {
    let mut h = harness(FakeLog::default());
    h.state_mut().auto_key = false;
    let all = ids(&h);
    let (a, b) = (all[1], all[2]);
    h.state_mut().select_only(Some(a));
    h.state_mut().toggle_selection(b);
    h.run_steps(3);
    egui_kittest::kittest::Queryable::get_by_label_contains(&h, "change on all 2");
    let x = |h: &Harness, id| h.state().project().layer(id).unwrap().transform.position.value_at(0.0).x;
    let (xa, xb) = (x(&h, a), x(&h, b));
    let pos = xenify_doc::PropPath::Position;
    assert!(h.state().shared_value_mixed(a, pos), "different positions show as mixed");
    // As the Position field does: primary moves to +25, the other by the same.
    let mut v = h.state().project().layer(a).unwrap().transform.position.value_at(0.0);
    v.x += 25.0;
    h.state_mut().edit_value_all(a, pos, xenify_doc::Value::Vec2(v));
    assert_eq!((x(&h, a) - xa, x(&h, b) - xb), (25.0, 25.0));
    // Equal values are set, not shifted.
    let op = xenify_doc::PropPath::Opacity;
    h.state_mut().edit_value_all(a, op, xenify_doc::Value::Scalar(0.5));
    let opacity = |h: &Harness, id: xenify_doc::Id| match h.state().project().layer(id).unwrap().value_at(op, 0.0) {
        Ok(xenify_doc::Value::Scalar(v)) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!((opacity(&h, a), opacity(&h, b)), (0.5, 0.5));
}

#[test]
fn errors_stay_in_red_until_dismissed() {
    let mut h = harness(FakeLog::default());
    h.state_mut().fail("Couldn't write the file");
    for _ in 0..3 {
        h.run_steps(300); // well past the 8 s messages last
    }
    egui_kittest::kittest::Queryable::get_by_label(&h, "Couldn't write the file");
    egui_kittest::kittest::Queryable::get_by_label(&h, "Dismiss").click();
    h.run_steps(2);
    assert!(h.state().error.is_none());
    assert!(egui_kittest::kittest::Queryable::query_by_label(&h, "Couldn't write the file").is_none());
}
