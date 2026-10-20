//! Tools and menus from the usability round: zoom out, exact shape sizes,
//! point editing, the knife, menus that follow the pointer, slices.

use super::common::*;
use egui::{Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
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

fn named(h: &Harness, name: &str) -> xenify_doc::Id {
    h.state().comp().layers.iter().find(|l| l.meta.name == name).expect("demo layer").id
}

fn centre_of(h: &Harness, id: xenify_doc::Id) -> egui::Pos2 {
    h.state().viewport.comp_to_screen(xenify_ui::testing::world_box(h.state(), id).center())
}

fn path_len(h: &Harness, id: xenify_doc::Id) -> Option<usize> {
    let layer = h.state().project().layer(id).ok()?;
    match &layer.shapes().first()?.geometry {
        xenify_doc::Geometry::Path { path, .. } => Some(path.value_at(0.0).elements().len()),
        _ => None,
    }
}

#[test]
fn the_zoom_tool_zooms_out_on_right_click() {
    let mut h = harness(FakeLog::default());
    h.key_press(Key::Z);
    h.run_steps(2);
    let at = h.state().viewport.screen_rect().center();
    let z0 = h.state().viewport.zoom();
    click(&mut h, at, PointerButton::Primary);
    let z1 = h.state().viewport.zoom();
    assert!(z1 > z0 * 1.1, "click zooms in: {z0} → {z1}");
    click(&mut h, at, PointerButton::Secondary);
    assert!(h.state().viewport.zoom() < z1 / 1.1, "right-click zooms out");
}

#[test]
fn clicking_with_a_shape_tool_asks_for_its_size() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h).len();
    h.key_press(Key::R);
    h.run_steps(2);
    let at = h.state().viewport.screen_rect().center();
    let target = h.state().viewport.screen_to_comp(at).unwrap();
    click(&mut h, at, PointerButton::Primary);
    h.get_by_label("New rectangle");
    h.get_by_label("Create").click();
    h.run_steps(3);
    assert_eq!(ids(&h).len(), before + 1);
    let new = *ids(&h).last().unwrap();
    let b = xenify_ui::testing::world_box(h.state(), new);
    assert!((b.width() - 200.0).abs() < 1e-6 && (b.height() - 200.0).abs() < 1e-6, "{b:?}");
    assert!((b.center() - target).hypot() < 1.0, "centred on the click");
}

#[test]
fn the_point_tool_makes_a_shape_editable_and_subdivides_it() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    assert_eq!(path_len(&h, square), None, "starts as an ellipse");
    h.key_press(Key::A);
    h.run_steps(2);
    let at = centre_of(&h, square);
    click(&mut h, at, PointerButton::Primary);
    assert_eq!(path_len(&h, square), None, "its points show, but it stays an ellipse until one changes");
    let n = 5; // an ellipse's four points and the close
    // Right-click away from its points: the tools act on the whole outline.
    let b = xenify_ui::testing::world_box(h.state(), square);
    let empty =
        h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(b.x1, b.center().y)) + egui::vec2(40.0, 0.0);
    click(&mut h, empty, PointerButton::Secondary);
    h.get_by_label("Subdivide").click();
    h.run_steps(3);
    assert!(path_len(&h, square).unwrap() > n, "a point added on every edge");
}

#[test]
fn the_knife_cuts_the_selected_shape_in_two() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    let before = ids(&h).len();
    let at = centre_of(&h, square);
    click(&mut h, at, PointerButton::Primary);
    assert_eq!(h.state().selected_layers(), vec![square]);
    h.key_press(Key::C);
    h.run_steps(2);
    let b = xenify_ui::testing::world_box(h.state(), square);
    let vp = &h.state().viewport;
    let from = vp.comp_to_screen(xenify_doc::kurbo::Point::new(b.x0 - 30.0, b.center().y));
    let to = vp.comp_to_screen(xenify_doc::kurbo::Point::new(b.x1 + 30.0, b.center().y));
    drag(&mut h, from, to);
    assert_eq!(ids(&h).len(), before + 1, "two halves where there was one");
}

#[test]
fn an_open_menu_follows_the_pointer_to_the_next_menu() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("File").click();
    h.run_steps(2);
    assert!(h.query_by_label("Undo").is_none() || h.query_all_by_label("Undo").count() == 0);
    h.get_by_label("Edit").hover();
    h.run_steps(3);
    h.get_by_label_contains("Duplicate");
}

#[test]
fn slices_can_be_shown_from_the_view_menu_and_listed_in_a_panel() {
    let mut h = harness(FakeLog::default());
    assert!(!h.state().prefs.show_slices);
    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Show Slices").click();
    h.run_steps(2);
    assert!(h.state().prefs.show_slices);
    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Panels ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Slices").click();
    h.run_steps(3);
    assert!(xenify_ui::testing::panel_open(h.state(), "Slices"));
    h.get_by_label("+ Draw slice");
}

#[test]
fn double_clicking_the_fill_swatch_opens_a_colour_picker() {
    let mut h = harness(FakeLog::default());
    assert!(h.query_by_label("Choose the fill colour").is_none());
    let at = h
        .get_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Fill"))
        .next()
        .expect("the fill swatch")
        .rect()
        .center();
    for _ in 0..2 {
        click(&mut h, at, PointerButton::Primary);
    }
    // Two quick clicks: egui sees a double-click on the second.
    h.get_by_label("Choose the fill colour");
}

fn click_with(h: &mut Harness, at: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event_modifiers(
            egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers },
            modifiers,
        );
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn shift_click_adds_to_the_selection() {
    let mut h = harness(FakeLog::default());
    let (ball, square) = (ids(&h)[1], ids(&h)[2]);
    let (a, b) = (centre_of(&h, ball), centre_of(&h, square));
    click(&mut h, a, PointerButton::Primary);
    click_with(&mut h, b, Modifiers::SHIFT);
    let mut sel = h.state().selected_layers();
    sel.sort();
    let mut want = vec![ball, square];
    want.sort();
    assert_eq!(sel, want);
}

#[test]
fn dragging_on_empty_canvas_selects_everything_in_the_box() {
    let mut h = harness(FakeLog::default());
    let (ball, square) = (ids(&h)[1], ids(&h)[2]);
    let r = xenify_ui::testing::world_box(h.state(), ball).union(xenify_ui::testing::world_box(h.state(), square));
    let vp = &h.state().viewport;
    let from = vp.comp_to_screen(xenify_doc::kurbo::Point::new(r.x0 - 20.0, r.y0 - 20.0));
    let to = vp.comp_to_screen(xenify_doc::kurbo::Point::new(r.x1 + 20.0, r.y1 + 20.0));
    drag(&mut h, from, to);
    let sel = h.state().selected_layers();
    assert!(sel.contains(&ball) && sel.contains(&square), "{sel:?}");
}

#[test]
fn a_selection_becomes_a_symbol_that_can_be_edited_and_left() {
    let mut h = harness(FakeLog::default());
    let square = named(&h, "Square");
    let before = xenify_ui::testing::world_box(h.state(), square);
    let at = centre_of(&h, square);
    click(&mut h, at, PointerButton::Primary);
    let main = h.state().comp;
    h.get_by_label("Canvas").click();
    h.run_steps(2);
    h.get_by_label_contains("Convert to Symbol").click();
    h.run_steps(3);
    let copy = h.state().selected_layers()[0];
    assert_ne!(copy, square, "the square now lives in the symbol; a copy stands in its place");
    let b = xenify_ui::testing::world_box(h.state(), copy);
    assert!(
        (b.width() - before.width()).abs() < 1.0 && (b.center() - before.center()).hypot() < 1.0,
        "the copy's box is the square's: {b:?} vs {before:?}"
    );
    // Double-click the copy (where the square was): the symbol opens, with a way back.
    for _ in 0..2 {
        click(&mut h, at, PointerButton::Primary);
    }
    assert_ne!(h.state().comp, main);
    h.get_by_label_contains("Editing symbol");
    h.get_by_label("Done").click();
    h.run_steps(3);
    assert_eq!(h.state().comp, main);
}

#[test]
fn the_toolbar_names_its_tools_for_screen_readers() {
    let h = harness(FakeLog::default());
    for name in ["Select", "Edit points", "Knife", "Zoom", "Hand"] {
        h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(name));
    }
}

#[test]
fn bevel_rounds_off_the_corners_of_a_path() {
    let mut h = harness(FakeLog::default());
    // A plain (sharp-cornered) rectangle.
    h.key_press(Key::R);
    h.run_steps(2);
    let spot = h.state().viewport.screen_rect().left_top() + egui::vec2(300.0, 150.0);
    click(&mut h, spot, PointerButton::Primary);
    h.get_by_label("Create").click();
    h.run_steps(3);
    let rect = *ids(&h).last().unwrap();
    h.key_press(Key::A);
    h.run_steps(2);
    let at = centre_of(&h, rect);
    click(&mut h, at, PointerButton::Primary);
    assert_eq!(path_len(&h, rect), None, "still a live rectangle");
    let b = xenify_ui::testing::world_box(h.state(), rect);
    let empty =
        h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(b.x1, b.center().y)) + egui::vec2(60.0, 0.0);
    click(&mut h, empty, PointerButton::Secondary);
    h.get_by_label("Bevel").click();
    h.run_steps(3);
    assert!(path_len(&h, rect).unwrap() >= 9, "each corner became two points: {:?}", path_len(&h, rect));
}

#[test]
fn moving_the_transform_origin_of_a_moving_layer_keeps_it_in_place() {
    let mut h = harness(FakeLog::default());
    let ball = named(&h, "Ball");
    let animated = |h: &Harness| h.state().project().layer(ball).unwrap().transform.position.is_animated();
    assert!(animated(&h), "the demo ball bounces");
    let boxes = |h: &mut Harness| {
        [0.0, 0.7, 1.9].map(|t| {
            h.state_mut().set_time(t);
            xenify_ui::testing::world_box(h.state(), ball)
        })
    };
    let before = boxes(&mut h);
    h.state_mut().set_time(0.7);
    let corner = xenify_doc::kurbo::Point::new(before[1].x0, before[1].y0);
    assert!(xenify_ui::testing::set_origin(h.state_mut(), ball, corner));
    let after = boxes(&mut h);
    for (a, b) in before.iter().zip(&after) {
        assert!((a.x0 - b.x0).abs() < 1e-6 && (a.y0 - b.y0).abs() < 1e-6, "didn't move: {a:?} vs {b:?}");
    }
    assert!(animated(&h), "still animated");
    let l = h.state().project().layer(ball).unwrap();
    let origin =
        xenify_anim::world_transform(h.state().comp(), ball, 0.7) * l.transform.anchor.value_at(0.7).to_point();
    assert!((origin - corner).hypot() < 1e-6, "the origin is at the corner now");
}
