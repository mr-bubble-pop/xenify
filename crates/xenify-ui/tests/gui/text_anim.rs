//! Text animators: Properties › Text › Animators and the Layers panel rows.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_ui::testing::FakeLog;

#[test]
fn add_a_text_animation_edit_it_and_undo() {
    let mut h = harness(FakeLog::default());
    let text = h.state().comp().layers.iter().find(|l| matches!(l.kind, LayerKind::Text(_))).unwrap().id;
    h.state_mut().select_only(Some(text));
    h.run_steps(3);
    let animators = |h: &Harness| match &h.state().project().layer(text).unwrap().kind {
        LayerKind::Text(t) => t.animators.clone(),
        _ => unreachable!(),
    };
    h.get_by_label("+ Add Animation").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("+ Add Animation").click();
    h.run_steps(3);
    h.get_by_label("Pop by Letter").click();
    h.run_steps(3);
    assert_eq!(animators(&h).len(), 1);
    assert_eq!(animators(&h)[0].name, "Pop by Letter");
    // It gets a row under the layer.
    h.get_by_label("Aa Pop by Letter");
    // Its settings: turn it into an "out" animation.
    h.get_by_label("Animate out").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Animate out").click();
    h.run_steps(3);
    assert!(animators(&h)[0].out);
    // Undo both steps.
    for _ in 0..2 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    }
    assert!(animators(&h).is_empty());
}
