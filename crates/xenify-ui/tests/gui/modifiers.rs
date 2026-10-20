//! Layer › Modifiers (arrays of copies), the Modifiers section of
//! Properties, and Duplicate's Transform Again.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::{ModifierKind, PropPath, Value, Vec2};
use xenify_ui::testing::FakeLog;

fn copies(h: &Harness, id: xenify_doc::Id) -> usize {
    let scene = xenify_anim::evaluate_project(h.state().project(), h.state().comp().id, h.state().time);
    scene.layers.iter().filter(|l| l.id == id).count()
}

#[test]
fn add_a_radial_array_from_the_layer_menu_then_apply_it() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(Some(square));
    h.run_steps(1);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Modifiers ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Add Radial Array").click();
    h.run_steps(2);
    let meta = |h: &Harness| h.state().project().layer(square).unwrap().meta.clone();
    let [m] = meta(&h).modifiers.try_into().expect("one modifier");
    let ModifierKind::Array { rotation, .. } = &m.kind else { panic!() };
    assert_eq!(rotation.value_at(0.0), 45.0, "eight copies around a centre");
    assert_eq!(copies(&h, square), 8);
    h.get_by_label("Modifiers (1)"); // the Properties section shows it
    h.get_by_label("Count");

    // Apply (scrolling Properties down to it): seven new layers, and the
    // square has no modifier left.
    let before = ids(&h).len();
    h.run_steps(10); // let the section finish opening
    // (Over the top of Properties: the timeline is below it.)
    h.hover_at(h.get_by_label("Name").rect().center() + egui::vec2(0.0, 60.0));
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -800.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(10);
    let apply = h.get_by_label("Apply").rect();
    assert!(apply.right() < 1600.0, "the row fits the panel: {apply:?}");
    let at = apply.center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(1);
    assert_eq!(ids(&h).len(), before + 7);
    assert!(meta(&h).modifiers.is_empty());
    assert_eq!(copies(&h, square), 1);

    // One undo puts the array back.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!((ids(&h).len(), meta(&h).modifiers.len()), (before, 1));
}

#[test]
fn duplicate_repeats_the_move_made_to_the_last_copy() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(Some(square));
    h.run_steps(1);
    let pos = |h: &Harness, id| match h.state().project().layer(id).unwrap().value_at(PropPath::Position, 0.0) {
        Ok(Value::Vec2(v)) => v,
        v => panic!("{v:?}"),
    };
    let start = pos(&h, square);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(2);
    let first = h.state().selected.unwrap();
    assert_ne!(first, square);
    // Move and turn the copy…
    for (path, value) in
        [(PropPath::Position, Value::Vec2(start + Vec2::new(60.0, 0.0))), (PropPath::Rotation, Value::Scalar(15.0))]
    {
        let cmd = xenify_ops::Command::edit_value(h.state().project(), first, path, 0.0, value, false).unwrap();
        h.state_mut().editor.execute("move", cmd).unwrap();
    }
    h.run_steps(1);
    // …and each Duplicate does the same again.
    for n in 2..=3 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
        h.run_steps(2);
        let copy = h.state().selected.unwrap();
        let p = pos(&h, copy);
        let rot = h.state().project().layer(copy).unwrap().transform.rotation.value_at(0.0);
        assert!((rot - 15.0 * n as f64).abs() < 1e-6, "copy {n} turned {rot}");
        assert!((p - start).hypot() > 60.0 * (n as f64 - 1.0), "copy {n} moved further: {p:?}");
    }
    // One undo removes the last copy (duplicate and move are one step).
    let n = ids(&h).len();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), n - 1);
}

#[test]
fn the_array_origin_is_a_handle_on_the_canvas() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(Some(square));
    let mut meta = h.state().project().layer(square).unwrap().meta.clone();
    meta.modifiers.push(xenify_doc::Modifier::new(ModifierKind::radial_array(6, Vec2::new(0.0, 150.0))));
    h.state_mut().editor.execute("array", xenify_ops::Command::SetLayerMeta { layer: square, meta }).unwrap();
    h.run_steps(2);
    let to_comp = xenify_ui::testing::layer_to_comp(h.state(), square);
    let screen = |h: &Harness, p: Vec2| h.state().viewport.comp_to_screen(to_comp * p.to_point());
    let (from, to) = (screen(&h, Vec2::new(0.0, 150.0)), screen(&h, Vec2::new(0.0, 250.0)));
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * (i as f32 / 8.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(2);
    let layer = h.state().project().layer(square).unwrap().clone();
    let ModifierKind::Array { origin, .. } = &layer.meta.modifiers[0].kind else { panic!() };
    let o = origin.value_at(0.0);
    assert!((o - Vec2::new(0.0, 250.0)).hypot() < 2.0, "the centre moved down: {o:?}");
    assert_eq!(
        h.state().project().layer(square).unwrap().transform,
        {
            let before = xenify_doc::demo_project();
            before.compositions[0].layers[2].transform.clone()
        },
        "the layer itself didn't move"
    );
}

#[test]
fn outline_changes_from_the_menu_and_outline_stroke() {
    let mut h = harness(FakeLog::default());
    let by_name = |h: &Harness, n: &str| h.state().comp().layers.iter().find(|l| l.meta.name == n).unwrap().id;
    let square = by_name(&h, "Square");
    h.state_mut().select_only(Some(square));
    h.run_steps(1);
    let outline = |h: &Harness| {
        let scene = xenify_anim::evaluate_project(h.state().project(), h.state().comp().id, 0.0);
        let xenify_anim::SceneContent::Shapes(items) = &scene.layer(square).unwrap().content else { panic!() };
        items[0].path.clone()
    };
    let plain = outline(&h);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Modifiers ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Add Wave").click();
    h.run_steps(2);
    assert_ne!(outline(&h), plain, "the square's edges wave");
    h.get_by_label("Smooth (wave)");
    let [m] = h.state().project().layer(square).unwrap().meta.modifiers.clone().try_into().unwrap();
    assert!(matches!(m.kind, ModifierKind::ZigZag { smooth: true, .. }));
    // Applying writes the wave into the square's own outline.
    let waved = outline(&h);
    let cmd = xenify_ops::modifiers::apply_first(h.state().project(), h.state().comp().id, square, 0.0).unwrap();
    h.state_mut().editor.execute("apply", cmd).unwrap();
    assert!(h.state().project().layer(square).unwrap().meta.modifiers.is_empty());
    assert_eq!(outline(&h), waved);

    // Path › Outline Stroke on the ring.
    let ring = by_name(&h, "Ring");
    h.state_mut().select_only(Some(ring));
    h.run_steps(1);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Path ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Outline Stroke").click();
    h.run_steps(2);
    let shapes = h.state().project().layer(ring).unwrap().shapes().to_vec();
    assert!(
        shapes.iter().all(|s| s.stroke.is_none()) && shapes.iter().any(|s| s.fill.is_some()),
        "{}",
        h.state().status
    );
}

#[test]
fn blend_and_array_along_a_path_between_two_selected_layers() {
    let mut h = harness(FakeLog::default());
    let by_name = |h: &Harness, n: &str| h.state().comp().layers.iter().find(|l| l.meta.name == n).unwrap().id;
    let (square, ball, ring) = (by_name(&h, "Square"), by_name(&h, "Ball"), by_name(&h, "Ring"));
    let count = |h: &Harness, id| {
        let scene = xenify_anim::evaluate_project(h.state().project(), h.state().comp().id, 0.0);
        scene.layers.iter().filter(|l| l.id == id).count()
    };
    let add = |h: &mut Harness, label: &str| {
        h.get_by_label("Layer").click();
        h.run_steps(2);
        h.get_by_label("Modifiers ⏵").hover();
        h.run_steps(2);
        h.get_by_label(label).click();
        h.run_steps(2);
    };
    // The square, then Shift+click the ball: the square blends into the ball.
    h.state_mut().select_only(Some(square));
    h.state_mut().toggle_selection(ball);
    h.run_steps(1);
    add(&mut h, "Add Blend");
    let m = h.state().project().layer(square).unwrap().meta.modifiers.clone();
    assert!(matches!(m.as_slice(), [x] if matches!(x.kind, ModifierKind::Blend { target, .. } if target == ball)));
    assert_eq!(count(&h, square), 5, "itself and four steps");
    assert!(h.state().project().layer(ball).unwrap().meta.modifiers.is_empty());

    // The ball, then the ring: balls around the ring.
    h.state_mut().select_only(Some(ball));
    h.state_mut().toggle_selection(ring);
    h.run_steps(1);
    add(&mut h, "Add Array Along Path");
    assert_eq!(count(&h, ball), 8);
    // Properties names the path it follows.
    h.state_mut().select_only(Some(ball));
    h.run_steps(3);
    h.get_by_label("Turn with the path");
}

#[test]
fn shatter_out_breaks_the_layer_from_the_playhead() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(Some(square));
    h.state_mut().time = 0.5;
    h.run_steps(1);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Modifiers ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Add Shatter Out").click();
    h.run_steps(2);
    let meta = |h: &Harness| h.state().project().layer(square).unwrap().meta.clone();
    let [m] = meta(&h).modifiers.try_into().expect("one modifier");
    let ModifierKind::Shatter { progress, count, .. } = &m.kind else { panic!("a shatter") };
    assert_eq!(progress.value_at(0.5), 0.0, "whole at the playhead");
    assert!(progress.value_at(1.5) > 0.0);
    assert_eq!(copies(&h, square), 1);
    h.state_mut().time = 1.5;
    h.run_steps(1);
    assert_eq!(copies(&h, square), count.value_at(0.0) as usize, "in pieces a second later");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(meta(&h).modifiers.is_empty());
}
