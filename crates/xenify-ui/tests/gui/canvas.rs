//! The canvas's right-click menu, text boxes and showing slices.

use super::common::*;
use egui::{Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_ui::testing::FakeLog;

fn click(h: &mut Harness, at: egui::Pos2, button: PointerButton) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

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

fn screen(h: &Harness, x: f64, y: f64) -> egui::Pos2 {
    h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(x, y))
}

/// A point on the demo's (locked) background, where nothing can be picked.
fn empty_spot(h: &Harness) -> egui::Pos2 {
    screen(h, 12.0, 12.0)
}

/// The menu item `label` nearest `at` (layers can have the same names).
fn menu_item(h: &Harness, label: &str, at: egui::Pos2) -> egui::Pos2 {
    h.get_all_by_label(label)
        .map(|n| n.rect().center())
        .min_by(|a, b| (*a - at).length().total_cmp(&(*b - at).length()))
        .expect("menu item")
}

#[test]
fn right_clicking_empty_canvas_creates_a_square_there() {
    let mut h = harness(FakeLog::default());
    h.state_mut().select_only(None);
    let before = ids(&h).len();
    let at = empty_spot(&h);
    click(&mut h, at, PointerButton::Secondary);
    let item = menu_item(&h, "Square", at);
    click(&mut h, item, PointerButton::Primary);
    assert_eq!(ids(&h).len(), before + 1, "status: {}", h.state().status);
    let new = *ids(&h).last().unwrap();
    let b = xenify_ui::testing::world_box(h.state(), new);
    assert!((b.width() - b.height()).abs() < 1e-6, "a square: {b:?}");
    let c = h.state().viewport.comp_to_screen(b.center());
    assert!((c - at).length() < 2.0, "centred on the click: {c:?} vs {at:?}");
}

#[test]
fn right_clicking_a_selection_groups_it() {
    let mut h = harness(FakeLog::default());
    let (a, b) = (ids(&h)[1], ids(&h)[2]);
    h.state_mut().select_only(Some(a));
    h.state_mut().toggle_selection(b);
    h.run_steps(1);
    let at = h.state().viewport.comp_to_screen(xenify_ui::testing::world_box(h.state(), b).center());
    click(&mut h, at, PointerButton::Secondary);
    // Still both selected: the click was on part of the selection.
    assert_eq!(h.state().selected_layers().len(), 2);
    h.get_by_label("Mask ⏵");
    h.get_by_label("Boolean ⏵");
    h.get_by_label("Effects ⏵");
    h.get_by_label_contains("Group").click();
    h.run_steps(3);
    let comp = h.state().comp();
    let group = comp.layers.iter().find(|l| matches!(l.kind, LayerKind::Group)).expect("a new group");
    for id in [a, b] {
        assert_eq!(comp.layers.iter().find(|l| l.id == id).unwrap().meta.parent, Some(group.id));
    }
}

#[test]
fn dragging_with_the_text_tool_makes_a_text_box() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h).len();
    h.key_press(Key::T);
    h.run_steps(2);
    let (from, to) = (screen(&h, 100.0, 100.0), screen(&h, 500.0, 300.0));
    drag(&mut h, from, to);
    assert_eq!(ids(&h).len(), before + 1);
    let layer = h.state().comp().layers.last().unwrap().clone();
    let LayerKind::Text(t) = &layer.kind else { panic!("a text layer") };
    let f = t.frame.expect("a text box");
    assert!((f.rect.width() - 400.0).abs() < 2.0 && (f.rect.height() - 200.0).abs() < 2.0, "{:?}", f.rect);
    assert!(f.wrap && !f.clip);
    // Its box can be resized from the Properties panel.
    h.get_by_label("Text box");
    h.get_by_label("Wrap lines");
    // Scroll Properties down to the box settings.
    let wrap = h.get_by_label("Wrap lines").rect().center();
    h.hover_at(wrap - egui::vec2(0.0, 200.0));
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(10);
    let at = h.get_by_label("Hide overflow").rect().center();
    click(&mut h, at, PointerButton::Primary);
    let LayerKind::Text(t) = &h.state().project().layer(layer.id).unwrap().kind else { unreachable!() };
    assert!(t.frame.unwrap().clip);
}

#[test]
fn unticking_show_slices_hides_them_even_in_the_slice_tool() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::SHIFT, Key::C);
    h.run_steps(2);
    assert!(h.state().prefs.show_slices, "picking the Slice tool shows slices");
    h.state_mut().prefs.show_slices = false;
    h.run_steps(2);
    assert!(!h.state().prefs.show_slices, "and unticking hides them again");
}

#[test]
fn the_empty_canvas_menu_unlocks_all_and_toggles_snapping() {
    let mut h = harness(FakeLog::default());
    h.state_mut().select_only(None);
    let first = h.state().comp().layers[0].clone();
    let mut meta = first.meta.clone();
    meta.locked = true;
    h.state_mut().exec("Lock", xenify_ops::Command::SetLayerMeta { layer: first.id, meta });
    h.run_steps(2);
    let at = empty_spot(&h);
    click(&mut h, at, PointerButton::Secondary);
    let item = menu_item(&h, "Unlock All", at);
    click(&mut h, item, PointerButton::Primary);
    assert!(h.state().comp().layers.iter().all(|l| !l.meta.locked));
    // One undo locks them again.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert!(h.state().comp().layers.iter().any(|l| l.meta.locked));

    let snap = h.state().prefs.snap_grid;
    click(&mut h, at, PointerButton::Secondary);
    h.get_by_label("Show ⏵").hover();
    h.run_steps(3);
    h.get_by_label("Snap to Grid").click();
    h.run_steps(2);
    assert_eq!(h.state().prefs.snap_grid, !snap);
}

#[test]
fn the_toolbar_has_crop() {
    // (Free Transform is part of the Select tool: its box is always there.)
    let mut h = harness(FakeLog::default());
    h.state_mut().select_only(None);
    h.get_by_label("Crop").click();
    h.run_steps(2);
    assert_eq!(h.state().tool, xenify_ui::testing::Tool::Crop);
}
