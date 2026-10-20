//! Slides and presenting.

use super::common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::FakeLog;

fn slides(h: &Harness) -> Vec<xenify_doc::Id> {
    h.state().project().slides.iter().map(|s| s.comp).collect()
}

#[test]
fn ctrl_m_adds_a_slide_and_shows_the_slides_panel() {
    let mut h = harness(FakeLog::default());
    let first = h.state().comp;
    assert!(!xenify_ui::testing::panel_open(h.state(), "Slides"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    h.run_steps(3);
    let list = slides(&h);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], first, "the existing composition became slide 1");
    assert_eq!(h.state().comp, list[1], "the new slide is open");
    assert!(xenify_ui::testing::panel_open(h.state(), "Slides"));

    // Clicking slide 1 in the panel opens it.
    h.get_by_label_contains("Slide 1").click();
    h.run_steps(2);
    assert_eq!(h.state().comp, first);

    // Undo takes the slide away again.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert!(h.state().project().slides.is_empty());
}

#[test]
fn pause_markers_toggle_at_the_playhead() {
    let mut h = harness(FakeLog::default());
    h.state_mut().set_time(1.0);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    h.run_steps(2);
    assert_eq!(h.state().comp().settings.pauses, vec![1.0]);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    h.run_steps(2);
    assert!(h.state().comp().settings.pauses.is_empty());
}

#[test]
fn presenting_pauses_advances_and_ends_on_the_last_slide_shown() {
    let mut h = harness(FakeLog::default());
    let first = h.state().comp;
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    h.run_steps(2);
    let second = h.state().comp;
    xenify_ui::testing::present_in_this_window(h.state_mut());
    // Back on slide 1, with a pause 1 s in.
    h.get_by_label_contains("Slide 1").click();
    h.run_steps(2);
    h.state_mut().set_time(1.0);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    h.run_steps(1);

    h.key_press(Key::F5);
    h.run_steps(2);
    assert!(h.state().presenter.is_some(), "presenting");
    assert_eq!(xenify_ui::testing::presenting_waits_at(h.state()), Some(1.0));
    // → skips to the pause; → plays on; → skips ahead. Slide 1 has a pause,
    // so it doesn't wait again at its end: it carries on into slide 2.
    for _ in 0..3 {
        h.key_press(Key::ArrowRight);
        h.run_steps(1);
    }
    h.run_steps(2);
    assert_eq!(xenify_ui::testing::presenting_slide(h.state()), Some(1), "no second stop at the end of slide 1");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().presenter.is_none());
    assert_eq!(h.state().comp, second, "the editor shows the slide that was on screen");
    let _ = first;
}

#[test]
fn deleting_a_slide_from_the_menu() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    h.run_steps(2);
    assert_eq!(slides(&h).len(), 3);
    h.get_by_label("Slide").click();
    h.run_steps(2);
    h.get_by_label_contains("Delete Slide").click();
    h.run_steps(2);
    assert_eq!(slides(&h).len(), 2);
    assert!(slides(&h).contains(&h.state().comp), "a remaining slide is open");
}

#[test]
fn the_presenter_view_shows_notes_the_next_slide_and_a_timer() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    h.run_steps(2);
    {
        let app = h.state_mut();
        let cmd = xenify_ops::slides::edit_slide(app.project(), 0, |s| s.notes = "Welcome everyone".into()).unwrap();
        app.exec("notes", cmd);
    }
    h.get_by_label("Slide").click();
    h.run_steps(2);
    h.get_by_label("Rehearse With Presenter View").click();
    h.run_steps(3);
    assert!(h.state().presenter.is_some());
    h.get_by_label("Welcome everyone");
    h.get_by_label_contains("Slide 1 of 2");
    h.get_by_label("0:00");
    // Next ▶ in the view moves on (to the end of slide 1, then slide 2).
    for _ in 0..2 {
        h.get_by_label("Next ▶").click();
        h.run_steps(2);
    }
    assert_eq!(xenify_ui::testing::presenting_slide(h.state()), Some(1));
    h.get_by_label_contains("No notes for this slide");
    // V shows the audience view; Esc ends.
    h.key_press(Key::V);
    h.run_steps(2);
    assert!(h.query_by_label("Next ▶").is_none());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().presenter.is_none());
}

#[test]
fn right_clicking_the_ruler_adds_a_pause_where_you_clicked() {
    let mut h = harness(FakeLog::default());
    h.state_mut().set_time(0.5);
    h.run_steps(2);
    let at = h.state().timeline.ruler_point(2.0).expect("timeline drawn");
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    h.get_by_label("Add pause at 2s 00f").click();
    h.run_steps(2);
    assert_eq!(h.state().comp().settings.pauses, vec![2.0], "at the click, not the playhead (0.5 s)");
    assert_eq!(h.state().time, 0.5, "the playhead stays put");
    // Right-clicking the marker offers to remove it.
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    h.get_by_label("Remove pause at 2s 00f").click();
    h.run_steps(2);
    assert!(h.state().comp().settings.pauses.is_empty());
}

#[test]
fn pick_a_transition_from_the_previews_and_set_it_up() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::M);
    h.run_steps(3);
    // The new slide (2) is open; its settings are in the Slides panel.
    // The first picker is how it comes in (the second, how it goes out).
    h.get_all_by_label("Cut ⏷").next().unwrap().scroll_to_me();
    h.run_steps(2);
    h.get_all_by_label("Cut ⏷").next().unwrap().click();
    h.run_steps(4);
    h.get_by_label("Wipe").click();
    h.run_steps(3);
    let tr = |h: &Harness| h.state().project().slides[1].transition_in;
    assert_eq!(tr(&h).kind, xenify_doc::TransitionKind::Wipe);
    // Its own settings show: direction and soft edge.
    h.get_by_label("Up").click();
    h.run_steps(2);
    assert_eq!(tr(&h).direction, xenify_doc::Direction::Up);
    assert!(h.query_by_label("Soft edge").is_some());

    // Slide › Transition switches it to Smart Morph, which lists its matches.
    h.get_by_label("Slide").click();
    h.run_steps(2);
    h.get_by_label("Transition ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Smart Morph").click();
    h.run_steps(3);
    assert_eq!(tr(&h).kind, xenify_doc::TransitionKind::SmartMorph);
    assert!(h.query_by_label_contains("Match (").is_some());

    // Undo, twice (the menu, then the direction and kind edits merged or not).
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(tr(&h).kind, xenify_doc::TransitionKind::Wipe);
}
