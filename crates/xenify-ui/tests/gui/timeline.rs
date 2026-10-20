//! The timeline: stopwatches on property rows, dragging keyframes in time.

use super::common::*;
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

fn click(h: &mut Harness, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        let button = egui::PointerButton::Primary;
        h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn square(h: &Harness) -> xenify_doc::Layer {
    h.state().comp().layers.iter().find(|l| l.meta.name == "Square").expect("demo layer").clone()
}

fn key_times(layer: &xenify_doc::Layer) -> Vec<f64> {
    let mut t: Vec<f64> = layer.property_paths().into_iter().flat_map(|p| layer.keyframe_times(p).unwrap()).collect();
    t.sort_by(f64::total_cmp);
    t.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    t
}

#[test]
fn keyframe_dots_on_a_collapsed_layer_drag_in_time() {
    let mut h = harness(FakeLog::default());
    let layer = square(&h);
    let before = key_times(&layer);
    let last = *before.last().expect("the demo square is animated");
    let tl = &h.state().timeline;
    let from = tl.summary_point(layer.id, last).expect("dot drawn");
    let dx = tl.ruler_point(last - 0.5).unwrap().x - tl.ruler_point(last).unwrap().x;
    drag(&mut h, from, from + egui::vec2(dx, 0.0));
    let after = key_times(&square(&h));
    assert_eq!(after.len(), before.len());
    assert!((after.last().unwrap() - (last - 0.5)).abs() < 0.05, "{before:?} → {after:?}");
    assert!(before.iter().rev().skip(1).all(|t| after.iter().any(|a| (a - t).abs() < 1e-6)), "the others stay");
}

#[test]
fn the_stopwatch_on_a_timeline_row_starts_and_stops_animating() {
    let mut h = harness(FakeLog::default());
    let layer = square(&h);
    let tl = &h.state().timeline;
    let dot = tl.summary_point(layer.id, key_times(&layer)[0]).expect("dot drawn");
    // The name column sits left of the tracks: expand arrow, then (on property rows) the stopwatch.
    let row_left = tl.ruler_point(0.0).unwrap().x - 12.0 - 220.0;
    click(&mut h, egui::pos2(row_left + 10.0, dot.y));
    // Expanded, every property is listed; find a still one and its row.
    let paths = layer.property_paths();
    let (row, path) =
        paths.iter().enumerate().find(|(_, p)| !layer.property(**p).unwrap().is_animated()).expect("a still property");
    h.state_mut().set_time(1.0);
    h.run_steps(2);
    let watch = egui::pos2(row_left + 26.0, dot.y + 22.0 * (row + 1) as f32);
    click(&mut h, watch);
    h.run_steps(2);
    assert_eq!(square(&h).keyframe_times(*path).unwrap(), vec![1.0], "{path} keyed at the playhead");
    click(&mut h, watch);
    h.run_steps(2);
    assert!(!square(&h).property(*path).unwrap().is_animated(), "{path} still again");
}

#[test]
fn a_layers_stopwatch_tracks_every_property_and_auto_keying_is_on() {
    let mut h = harness(FakeLog::default());
    assert!(h.state().auto_key, "every change keyframes by default");
    let animated = |h: &Harness| {
        let l = square(h);
        let n = l.property_paths().into_iter().filter(|p| l.property(*p).unwrap().is_animated()).count();
        (n, l.property_paths().len())
    };
    let (before, total) = animated(&h);
    assert!(before < total);
    let watch = egui_kittest::kittest::Queryable::get_by_label(&h, "Track all of Square").rect().center();
    click(&mut h, watch);
    assert_eq!(animated(&h), (total, total), "everything tracked");
    click(&mut h, watch);
    assert_eq!(animated(&h).0, 0, "and all of it stopped");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(animated(&h), (total, total), "one undo step each");
}
