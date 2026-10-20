//! File ▸ Settings: recording shortcuts, clashes.

use super::common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::{FakeLog, action_by_id, chord};

#[test]
fn ctrl_comma_opens_settings_with_its_tabs() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    h.run_steps(2);
    h.get_by_label("Shortcuts").click();
    h.run_steps(2);
    h.get_by_label("Reset all");
    // The Settings tab (the menu bar also has an AI menu, drawn first).
    h.get_all_by_label("AI").last().unwrap().click();
    h.run_steps(2);
    h.get_by_label("Save");
}

#[test]
fn recording_a_shortcut_binds_it_without_running_anything() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h);
    let dup = action_by_id("edit.duplicate");
    xenify_ui::testing::open_settings(h.state_mut(), "shortcuts");
    h.state_mut().settings.recording = Some(dup);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::K);
    h.run_steps(2);
    assert!(h.state().keymap.chords(dup).contains(&chord("Ctrl+Shift+K")));
    assert_eq!(ids(&h), before, "the recorded keys don't also do something");

    // The new shortcut works.
    h.state_mut().settings.is_open = false;
    let first = before[0];
    h.state_mut().select_only(Some(first));
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::K);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), before.len() + 1);
}

#[test]
fn a_clash_asks_before_taking_the_keys() {
    let mut h = harness(FakeLog::default());
    let (group, dup) = (action_by_id("object.group"), action_by_id("edit.duplicate"));
    xenify_ui::testing::open_settings(h.state_mut(), "shortcuts");
    h.state_mut().settings.recording = Some(group);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run_steps(2);
    let ctrl_d = chord("Ctrl+D");
    assert!(!h.state().keymap.chords(group).contains(&ctrl_d), "not taken yet");
    h.get_by_label_contains("is already used by Duplicate");
    h.get_by_label("Use it for Group instead").click();
    h.run_steps(2);
    assert!(h.state().keymap.chords(group).contains(&ctrl_d));
    assert!(!h.state().keymap.chords(dup).contains(&ctrl_d));
}

#[test]
fn settings_only_count_as_unsaved_after_an_ai_change() {
    let mut h = harness(FakeLog::default());
    xenify_ui::testing::open_settings(h.state_mut(), "ai");
    h.run_steps(3);
    assert!(!xenify_ui::testing::ai_settings_unsaved(h.state()), "opening and looking changes nothing");
}

/// A look at the interface theme, normal and high contrast: `SHOT_DIR=dir
/// cargo test -p xenify-ui --test gui settings::theme_screenshot -- --ignored`
/// writes PNGs.
#[test]
#[ignore]
fn theme_screenshot() {
    let Some(dir) = std::env::var_os("SHOT_DIR").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness(FakeLog::default());
    h.run_steps(4);
    h.render().unwrap().save(dir.join("theme.png")).unwrap();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    h.run_steps(4);
    h.render().unwrap().save(dir.join("theme-settings.png")).unwrap();
    h.state_mut().prefs.high_contrast = true;
    h.run_steps(4);
    h.render().unwrap().save(dir.join("theme-high-contrast.png")).unwrap();
}
