//! Menu bar behaviour.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::FakeLog;

fn text_input<'a>(h: &'a Harness, value: &'a str) -> egui_kittest::Node<'a> {
    h.get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some(value))
}

fn ctrl() -> &'static str {
    if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }
}

#[test]
fn menu_shortcuts_follow_the_keymap() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Edit").click();
    h.run_steps(2);
    h.get_by_label(&format!("Duplicate {}+D", ctrl()));

    // Rebind Duplicate: the menu shows the new keys.
    let dup = xenify_ui::testing::action_by_id("edit.duplicate");
    h.state_mut().keymap.set(dup, vec![xenify_ui::testing::chord("Ctrl+Shift+D")]);
    h.run_steps(2);
    h.get_by_label(&format!("Duplicate {}+Shift+D", ctrl()));
    assert!(h.query_by_label(&format!("Duplicate {}+D", ctrl())).is_none());
}

#[test]
fn view_panels_closes_and_reopens_a_panel() {
    let mut h = harness(FakeLog::default());
    let open = |h: &Harness| xenify_ui::testing::panel_open(h.state(), "Graph Editor");
    assert!(open(&h));
    for expected in [false, true] {
        h.get_by_label("View").click();
        h.run_steps(2);
        h.get_by_label("Panels ⏵").hover();
        h.run_steps(2);
        h.get_by_label("Graph Editor").click();
        h.run_steps(3);
        assert_eq!(open(&h), expected);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
    }
}

#[test]
fn export_dialog_takes_the_format_from_the_name() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::E);
    h.run_steps(3);
    h.get_by_value("MP4 video (H.264)");
    text_input(&h, "untitled.mp4").focus();
    h.run_steps(1);
    h.key_press(egui::Key::End);
    for _ in 0..3 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(1);
    text_input(&h, "untitled.").type_text("gif");
    h.run_steps(3);
    h.get_by_value("GIF animation");
}

#[test]
fn print_sends_the_frame_to_the_chosen_printer() {
    let log = FakeLog::default();
    log.with(|s| s.printers = vec!["Office".into(), "PDF".into()]);
    let mut h = harness(log.clone());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    h.run_steps(2);
    h.get_by_value("Office").click();
    h.run_steps(2);
    h.get_by_label("PDF").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Print").click();
    h.run_steps(2);
    let printed = log.with(|s| s.printed.clone());
    if printed.is_empty() && h.state().status.contains("GPU") {
        eprintln!("skipping: {}", h.state().status);
        return;
    }
    assert_eq!(printed.len(), 1, "{}", h.state().status);
    let (path, opts) = &printed[0];
    assert_eq!(opts.printer.as_deref(), Some("PDF"));
    assert!(std::fs::read(path).unwrap().starts_with(b"\x89PNG"));
}

#[test]
fn print_explains_when_there_is_no_printer() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    h.run_steps(2);
    h.get_by_label_contains("Can't print from here");
}

#[test]
fn exports_run_in_the_background_and_offer_the_folder() {
    let dir = std::env::temp_dir().join(format!("xenify-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness(FakeLog::default());
    h.state_mut().last_dir = Some(dir.clone());
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::E);
    h.run_steps(3);
    text_input(&h, "untitled.mp4").focus();
    h.run_steps(1);
    h.key_press(egui::Key::End);
    for _ in 0..3 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(1);
    text_input(&h, "untitled.").type_text("svg");
    h.run_steps(3);
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Export")).click();
    h.run_steps(2);
    for _ in 0..500 {
        if h.state().export.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let file = dir.join("untitled.svg");
    assert!(file.exists(), "{}", h.state().status);
    assert!(h.state().status.starts_with("Exported"), "{}", h.state().status);
    assert_eq!(h.state().last_export.as_deref(), Some(file.as_path()));
    h.get_by_label("Show in folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dialogs_have_a_title_bar_that_closes_them() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::E);
    h.run_steps(3);
    assert!(h.state().dialog.is_some());
    // The title is in the bar, beside its close button.
    // The topmost "Export" is the title (the other is the button at the bottom).
    let title =
        h.get_all_by_label("Export").map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("a title");
    let close = h
        .get_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("×"))
        .map(|n| n.rect())
        .find(|r| (r.center().y - title.center().y).abs() < 6.0)
        .expect("a × in the title bar");
    h.hover_at(close.center());
    h.run_steps(1);
    for pressed in [true, false] {
        let button = egui::PointerButton::Primary;
        h.event(egui::Event::PointerButton { pos: close.center(), button, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
}

#[test]
fn credits_list_every_asset_from_the_sources_record() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Help").click();
    h.run_steps(2);
    h.get_by_label("Credits & Licences").click();
    // The modal centres itself over a few frames once it knows its size.
    h.run_steps(10);
    let all = xenify_export::sources::bundled();
    for e in &all {
        // Licence files are shown inside the entry that uses them.
        if all.iter().any(|o| o.license_file.is_some() && o.license_file == e.path) {
            continue;
        }
        assert!(h.query_by_label(&e.name).is_some(), "{} is in the credits", e.name);
    }
    h.get_all_by_label("Licence text").next().unwrap().click();
    h.run_steps(10);
    assert!(h.query_by_label_contains("SIL Open Font License").is_some());
}

#[test]
fn the_export_dialog_offers_4k_and_custom_sizes() {
    let mut h = harness(FakeLog::default());
    xenify_ui::testing::open_export(h.state_mut());
    h.run_steps(10);
    assert_eq!(xenify_ui::testing::export_size(h.state()), Some(None), "the canvas size to start with");
    h.get_by_label("4K UHD").click();
    h.run_steps(2);
    let s = xenify_ui::testing::export_size(h.state()).unwrap().unwrap();
    assert!(s == (3840, 2160) || s == (2160, 3840), "{s:?}");
    h.get_by_label("Custom").click();
    h.run_steps(2);
    assert_eq!(xenify_ui::testing::export_size(h.state()), Some(Some(s)), "custom starts from the preset");
    h.get_by_label("Canvas Size").click();
    h.run_steps(2);
    assert_eq!(xenify_ui::testing::export_size(h.state()), Some(None));
}
