//! Rulers, guides, grid, snapping and exact sizes.

use super::common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
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

#[test]
fn double_clicking_a_rectangle_edits_its_points() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    let at = h.state().viewport.comp_to_screen(xenify_ui::testing::world_box(h.state(), square).center());
    h.hover_at(at);
    h.run_steps(1);
    // Click to select, then double-click soon after (egui calls that a triple click).
    for pressed in [true, false, true, false, true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert_eq!(h.state().tool, xenify_ui::testing::Tool::Node, "every shape is points");
    let live = |h: &Harness| {
        let l = h.state().project().layer(square).unwrap();
        !matches!(l.shapes()[0].geometry, xenify_doc::Geometry::Path { .. })
    };
    assert!(live(&h), "looking at its points doesn't change it");
    // Ctrl+T still types exact values.
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.state_mut().select_only(Some(square));
    h.key_press_modifiers(Modifiers::COMMAND, Key::T);
    h.run_steps(3);
    h.get_by_label("Position & size");
}

#[test]
fn shortcuts_toggle_rulers_and_grid() {
    let mut h = harness(FakeLog::default());
    assert!(h.state().prefs.show_rulers && !h.state().prefs.show_grid);
    h.key_press_modifiers(Modifiers::COMMAND, Key::R);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Quote);
    h.run_steps(2);
    assert!(!h.state().prefs.show_rulers && h.state().prefs.show_grid);
}

#[test]
fn guides_come_out_of_the_rulers_and_go_back() {
    let mut h = harness(FakeLog::default());
    let canvas = h.state().viewport.screen_rect();
    // From the top ruler down into the canvas: a horizontal guide there.
    let from = egui::pos2(canvas.center().x, canvas.top() + 8.0);
    let to = egui::pos2(canvas.center().x, canvas.center().y);
    let expected = h.state().viewport.screen_to_comp(to).unwrap().y.round();
    drag(&mut h, from, to);
    let guides = h.state().comp().settings.guides.clone();
    assert_eq!(guides.len(), 1);
    assert!(!guides[0].vertical);
    assert!((guides[0].pos - expected).abs() <= 1.0, "{} vs {expected}", guides[0].pos);
    // Dragging it back onto the ruler removes it (one undo step each way).
    h.state_mut().set_tool_select();
    drag(&mut h, to, from);
    assert!(h.state().comp().settings.guides.is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().comp().settings.guides.len(), 1);
}

#[test]
fn moving_objects_snap_to_guides_with_a_margin() {
    let mut h = harness(FakeLog::default());
    // A vertical guide at x = 400, snapping 20 px away from it.
    let mut settings = h.state().comp().settings.clone();
    settings.guides.push(xenify_doc::GuideLine { vertical: true, pos: 400.0 });
    let comp = h.state().comp;
    h.state_mut().exec("guide", xenify_ops::Command::SetCompSettings { comp, settings });
    h.state_mut().prefs.snap_margin = 20.0;
    h.state_mut().prefs.snap_objects = false; // only the guide
    h.run_steps(1);
    let square = ids(&h)[2];
    let left = |h: &Harness| xenify_ui::testing::world_box(h.state(), square).x0;
    // Drag the square so its left edge lands 3 px short of 420 (guide + margin).
    let b = xenify_ui::testing::world_box(h.state(), square);
    let grab = h.state().viewport.comp_to_screen(b.center());
    let dx = (417.0 - b.x0) * h.state().viewport.zoom();
    drag(&mut h, grab, grab + egui::vec2(dx as f32, 0.0));
    assert!((left(&h) - 420.0).abs() < 1e-6, "snapped to guide + margin: {}", left(&h));
}

#[test]
fn ctrl_t_sets_an_exact_width() {
    let mut h = harness(FakeLog::default());
    let square = ids(&h)[2];
    h.state_mut().select_only(Some(square));
    let b = xenify_ui::testing::world_box(h.state(), square);
    h.key_press_modifiers(Modifiers::COMMAND, Key::T);
    h.run_steps(3);
    h.get_by_label("Position & size");
    // Width: drag the number sideways (1 px per point) from 80 to 300.
    let width = format!("{:.1} px", b.width());
    let field = h.get_all_by_value(&width).next().expect("width field").rect().center(); // width comes before height
    let to = field + egui::vec2((300.0 - b.width()) as f32, 0.0);
    h.drag_at(field);
    h.run_steps(1);
    for i in 1..=10 {
        h.hover_at(field + (to - field) * (i as f32 / 10.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(2);
    h.get_by_value("300.0 px");
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let after = xenify_ui::testing::world_box(h.state(), square);
    assert!((after.width() - 300.0).abs() < 1e-6, "{after:?}");
    assert!((after.x0 - b.x0).abs() < 1e-6, "the left edge stays");
}
