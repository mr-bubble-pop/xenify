//! Accessibility: what screen readers get from the canvas, and picking
//! objects without a mouse.

use super::common::*;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use xenify_ui::testing::FakeLog;

fn selected_name(h: &Harness) -> Option<String> {
    let id = h.state().selected?;
    Some(h.state().project().layer(id).ok()?.meta.name.clone())
}

#[test]
fn the_canvas_lists_its_objects_for_screen_readers() {
    let mut h = harness(FakeLog::default());
    h.state_mut().select_only(None);
    h.run_steps(2);
    let canvas = h.get_by_role(Role::Canvas);
    let objects: Vec<String> = canvas
        .accesskit_node()
        .children()
        .filter(|c| c.role() == Role::GraphicsObject)
        .filter_map(|c| c.label())
        .collect();
    // Top first, as in the Layers panel; named by kind and name.
    assert_eq!(objects.len(), 4, "{objects:?}");
    assert!(objects[0].starts_with("Text Title"), "{objects:?}");
    assert!(objects.iter().any(|o| o == "Shape Ball"), "{objects:?}");

    // A screen reader's click on one selects it.
    h.get_by_label("Shape Ball").click_accesskit();
    h.run_steps(2);
    assert_eq!(selected_name(&h).as_deref(), Some("Ball"));
    let ball = h.get_by_label("Shape Ball");
    assert_eq!(ball.accesskit_node().is_selected(), Some(true));
    // It has a box on screen (for magnifiers and touch exploration).
    assert!(ball.rect().width() > 1.0 && ball.rect().height() > 1.0);
}

#[test]
fn alt_brackets_step_through_objects() {
    let mut h = harness(FakeLog::default());
    h.state_mut().select_only(None);
    h.run_steps(1);
    let step = |h: &mut Harness, key| {
        h.key_press_modifiers(Modifiers::ALT, key);
        h.run_steps(2);
    };
    step(&mut h, Key::CloseBracket);
    assert_eq!(selected_name(&h).as_deref(), Some("Title"));
    assert!(h.state().status.starts_with("Selected Title (1 of 4)"), "{}", h.state().status);
    step(&mut h, Key::CloseBracket);
    assert_eq!(selected_name(&h).as_deref(), Some("Ball"));
    step(&mut h, Key::OpenBracket);
    step(&mut h, Key::OpenBracket);
    // Wraps round to the bottom-most.
    assert_eq!(selected_name(&h).as_deref(), Some("Ring"));
}

#[test]
fn reduce_motion_and_contrast_settings() {
    let mut h = harness(FakeLog::default());
    xenify_ui::testing::open_settings(h.state_mut(), "general");
    h.run_steps(2);
    h.get_by_label("Reduce motion").click();
    h.run_steps(2);
    assert!(h.state().prefs.reduce_motion);
    assert_eq!(h.ctx.global_style().animation_time, 0.0);
    h.get_by_label("Reduce motion").click();
    h.run_steps(2);
    assert!(h.ctx.global_style().animation_time > 0.0);
}

#[test]
fn status_messages_are_announced() {
    let mut h = harness(FakeLog::default());
    h.state_mut().select_only(None);
    h.key_press_modifiers(Modifiers::ALT, Key::CloseBracket);
    h.run_steps(2);
    let msg = h.get_by_label_contains("Selected Title");
    assert_eq!(msg.accesskit_node().live(), egui::accesskit::Live::Polite);
}
