//! Layer ▸ Effects and the Effects section of Properties.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::{BlendMode, EffectKind};
use xenify_ui::testing::FakeLog;

#[test]
fn add_a_shadow_and_a_blend_mode_from_the_layer_menu() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(Some(square));
    h.run_steps(1);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Effects ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Add Drop shadow").click();
    h.run_steps(2);
    let meta = |h: &Harness| h.state().project().layer(square).unwrap().meta.clone();
    assert!(matches!(meta(&h).effects.as_slice(), [e] if matches!(e.kind, EffectKind::DropShadow { .. })));
    h.get_by_label("Effects (1)"); // the Properties section shows it

    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Effects ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Blend Mode ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Screen").click();
    h.run_steps(2);
    assert_eq!(meta(&h).blend, BlendMode::Screen);

    // One undo each.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(meta(&h).effects.is_empty() && meta(&h).blend == BlendMode::Normal);
}

#[test]
fn effects_are_listed_under_their_layer_and_toggled_and_edited_there() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    let mut meta = h.state().project().layer(square).unwrap().meta.clone();
    meta.effects.push(xenify_doc::Effect::new(EffectKind::presets()[0].clone()));
    h.state_mut().editor.execute("fx", xenify_ops::Command::SetLayerMeta { layer: square, meta }).unwrap();
    h.run_steps(2);
    let enabled = |h: &Harness| h.state().project().layer(square).unwrap().meta.effects[0].enabled;

    // Effect rows start folded: the fx badge opens them.
    assert!(h.query_by_label("fx Drop shadow").is_none());
    h.get_by_label("Show or hide effects").click();
    h.run_steps(2);
    // The effect has its own row under the layer; its 👁 turns it off and on.
    let row = h.get_by_label("fx Drop shadow").rect();
    let eye = row.left_center() + egui::vec2(4.0 + 18.0 + 48.0 + 10.0, 0.0);
    h.hover_at(eye);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: eye,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(!enabled(&h), "the eye turned it off");

    // Double-clicking the row opens its settings, with its on/off switch.
    let at = h.get_by_label("fx Drop shadow").rect().center();
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
    }
    h.run_steps(3);
    h.get_by_label("All effects of this layer…").click();
    h.run_steps(3);
    // The whole list: in Properties and in the window.
    assert_eq!(h.get_all_by_label("+ Add effect").count(), 2);
}
