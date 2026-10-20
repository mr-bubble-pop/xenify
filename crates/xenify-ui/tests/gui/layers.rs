//! The Layers panel: renaming, dragging rows, thumbnails.

use super::common::*;
use egui::Key;
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::FakeLog;

fn names(h: &Harness) -> Vec<String> {
    h.state().comp().layers.iter().rev().map(|l| l.meta.name.clone()).collect()
}

#[test]
fn f2_renames_the_selected_layer() {
    let mut h = harness(FakeLog::default());
    let id = ids(&h)[0];
    h.state_mut().select_only(Some(id));
    h.key_press(Key::F2);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    // The rename box has the keyboard focus (Properties shows the name too).
    let field = h.get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.is_focused());
    assert_eq!(field.value().as_deref(), Some("Ring"));
    field.type_text("Halo");
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(names(&h).iter().any(|n| n.contains("Halo")), "{:?}", names(&h));
    assert!(names(&h).iter().all(|n| n != "Ring"));
}

#[test]
fn dragging_a_row_restacks_it() {
    let mut h = harness(FakeLog::default());
    // Top of the list first (drawn on top).
    assert_eq!(names(&h), ["Title", "Ball", "Square", "Ring"]);
    let from = h.get_by_label("Ring").rect().center();
    let to = h.get_by_label("Ball").rect();
    let target = egui::pos2(to.center().x, to.top() + 3.0); // upper edge: above Ball
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (target - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(target);
    h.run_steps(3);
    assert_eq!(names(&h), ["Title", "Ring", "Ball", "Square"]);
}

#[test]
fn double_clicking_a_row_renames_it_without_widget_id_clashes() {
    let mut h = harness(FakeLog::default());
    // A group, so rows have arrows, eyes and locks at different depths.
    let all = ids(&h);
    h.state_mut().select_only(Some(all[1]));
    h.state_mut().toggle_selection(all[2]);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::G);
    h.run_steps(3);
    for name in names(&h) {
        let Some(row) = h.query_all_by_label(&name).next().map(|n| n.rect()) else { continue };
        let at = egui::pos2(row.left() + 110.0, row.center().y);
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..2 {
            for pressed in [true, false] {
                let button = egui::PointerButton::Primary;
                h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
        }
        h.run_steps(3);
        // egui marks clashing widget ids with a red box and a 🔥 note.
        let clash = h.output().shapes.iter().any(|s| format!("{:?}", s.shape).contains('🔥'));
        assert!(!clash, "double-clicking {name} made two widgets share an id");
        h.key_press(Key::Escape);
        h.run_steps(2);
    }
}

/// Clicks layer row `name` with `modifiers`.
fn click_row(h: &mut Harness, name: &str, modifiers: egui::Modifiers) {
    let at = h
        .get_all_by(|n| n.label().as_deref() == Some(name) && n.role() != egui::accesskit::Role::TextInput)
        .map(|n| n.rect().center())
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .expect("row");
    h.hover_at(at);
    h.run_steps(1);
    h.input_mut().modifiers = modifiers;
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run_steps(1);
    }
    h.input_mut().modifiers = egui::Modifiers::NONE;
    h.run_steps(1);
}

#[test]
fn shift_selects_a_range_and_ctrl_toggles() {
    let mut h = harness(FakeLog::default());
    // Top-level rows, top to bottom.
    let comp = h.state().comp().clone();
    let rows: Vec<(xenify_doc::Id, String)> =
        comp.layers.iter().rev().filter(|l| l.meta.parent.is_none()).map(|l| (l.id, l.meta.name.clone())).collect();
    assert!(rows.len() >= 3, "{rows:?}");
    let unique = |n: &str| rows.iter().filter(|(_, m)| m == n).count() == 1;
    assert!(unique(&rows[0].1) && unique(&rows[2].1));
    click_row(&mut h, &rows[0].1, egui::Modifiers::NONE);
    click_row(&mut h, &rows[2].1, egui::Modifiers::SHIFT);
    let mut sel = h.state().selected_layers();
    sel.sort();
    let mut want: Vec<_> = rows[..3].iter().map(|r| r.0).collect();
    want.sort();
    assert_eq!(sel, want, "a range of three");
    click_row(&mut h, &rows[1].1, egui::Modifiers::COMMAND);
    assert!(!h.state().selected_layers().contains(&rows[1].0), "Ctrl-click toggles it off");
    assert_eq!(h.state().selected_layers().len(), 2);
}
