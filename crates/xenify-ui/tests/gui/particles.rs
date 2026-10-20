//! Particle layers: Layer › New › Particles, Properties › Particles and the
//! direction arrow on the canvas.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::{Emitter, EndLook, LayerKind, Project};
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

fn emitter(h: &Harness) -> Option<(xenify_doc::Id, Emitter)> {
    h.state().comp().layers.iter().find_map(|l| match &l.kind {
        LayerKind::Particles(e) => Some((l.id, (**e).clone())),
        _ => None,
    })
}

#[test]
fn add_confetti_aim_it_change_it_and_undo() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Particles ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Confetti Burst").click();
    h.run_steps(3);
    let (id, e) = emitter(&h).expect("a particle layer");
    assert!(h.state().is_selected(id));
    assert_eq!(e.direction.value_at(0.0), -90.0);

    // Drag the arrow's tip to the right: it now points right, a bit faster.
    let time = h.state().time;
    let origin = xenify_anim::world_transform(h.state().comp(), id, time) * kurbo::Point::ZERO;
    let tip = origin + kurbo::Vec2::new(0.0, -e.speed.value_at(0.0) * 0.2);
    let to = origin + kurbo::Vec2::new(250.0, 0.0);
    let (a, b) = (h.state().viewport.comp_to_screen(tip), h.state().viewport.comp_to_screen(to));
    drag(&mut h, a, b);
    let (_, e2) = emitter(&h).unwrap();
    assert!(e2.direction.value_at(time).abs() < 2.0, "points right: {}", e2.direction.value_at(time));
    assert!((e2.speed.value_at(time) - 1250.0).abs() < 30.0, "speed from the length: {}", e2.speed.value_at(time));

    // Properties: finish by settling on the floor.
    h.get_by_label("Settle on the floor").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Settle on the floor").click();
    h.run_steps(3);
    assert!(matches!(emitter(&h).unwrap().1.end, EndLook::Settle { .. }));

    // Saved with the project.
    let back = Project::from_json(&h.state().project().to_json()).unwrap();
    assert_eq!(back.layer(id).unwrap().kind, h.state().project().layer(id).unwrap().kind);

    // Undo: the ending, the aim (one step), then the layer.
    for _ in 0..3 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    }
    assert!(emitter(&h).is_none());
}

#[test]
fn big_blast_adds_a_group_of_parts_in_one_undo() {
    let mut h = harness(FakeLog::default());
    let before = h.state().comp().layers.len();
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Particles ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Big Blast").scroll_to_me();
    h.run_steps(2);
    h.get_by_label("Big Blast").click();
    h.run_steps(3);
    let comp = h.state().comp();
    let group = comp.layers.iter().find(|l| l.meta.name == "Big Blast").expect("a Big Blast group");
    assert!(h.state().is_selected(group.id));
    for part in ["Flash", "Shockwave", "Fireball", "Stem", "Mushroom Cap", "Dust", "Debris"] {
        assert!(comp.layers.iter().any(|l| l.meta.name == part && l.meta.parent == Some(group.id)), "{part}");
    }
    assert!(xenify_doc::active_camera(&comp.layers, h.state().time).is_some(), "a shaking camera");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().comp().layers.len(), before);
}

#[test]
fn the_effect_gallery_adds_effects_and_uses_the_selection() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(None);
    h.run_steps(1);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New Effect…").click();
    h.run_steps(10);
    // Without a selection, effects on a layer can't be chosen.
    h.get_by_label("Shatter Out").click();
    h.run_steps(2);
    assert!(h.state().project().layer(square).unwrap().meta.modifiers.is_empty());
    h.get_by_label("Snow").click();
    h.run_steps(3);
    assert!(emitter(&h).is_some(), "snow added");
    assert!(h.query_by_label("Big Blast").is_none(), "the gallery closed");

    // With the square selected, Shatter Out breaks it.
    h.state_mut().select_only(Some(square));
    h.run_steps(1);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New Effect…").click();
    h.run_steps(10); // the dialog settles in the middle
    h.get_by_label("Shatter Out").scroll_to_me();
    h.run_steps(4);
    h.get_by_label("Shatter Out").click();
    h.run_steps(3);
    let meta = h.state().project().layer(square).unwrap().meta.clone();
    assert!(matches!(meta.modifiers.as_slice(), [m] if matches!(m.kind, xenify_doc::ModifierKind::Shatter { .. })));
}
