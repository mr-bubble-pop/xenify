//! Liquid layers: Layer › New › Water and the level handle.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
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

fn water(h: &Harness) -> Option<(xenify_doc::Id, xenify_doc::LiquidFill)> {
    h.state().comp().layers.iter().find_map(|l| match &l.kind {
        LayerKind::Liquid(w) => Some((l.id, (**w).clone())),
        _ => None,
    })
}

#[test]
fn fill_the_screen_drag_the_level_and_undo() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Water ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Fill the Screen").click();
    h.run_steps(3);
    let (id, _) = water(&h).expect("a water layer");
    assert!(h.state().is_selected(id));
    assert!(h.query_by_label("Wave trains").is_some(), "the Properties section");

    // Drag the surface to 3/4 of the way up.
    let t = h.state().time;
    let (_, w) = water(&h).unwrap();
    let world = xenify_anim::world_transform(h.state().comp(), id, t);
    let from = world * xenify_anim::liquid::surface_middle(&w, t);
    let height = h.state().comp().settings.height as f64;
    let to = kurbo::Point::new(from.x, height * 0.25);
    let (a, b) = (h.state().viewport.comp_to_screen(from), h.state().viewport.comp_to_screen(to));
    drag(&mut h, a, b);
    let level = water(&h).unwrap().1.level.value_at(h.state().time);
    assert!((level - 0.75).abs() < 0.02, "level {level}");

    for _ in 0..2 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    }
    assert!(water(&h).is_none());
}
