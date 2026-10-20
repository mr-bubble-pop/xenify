//! Several open files as tabs, and File ▸ New.

use super::common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_platform::desktop::ClipContent;
use xenify_ui::testing::FakeLog;

fn new_blank(h: &mut Harness) {
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(3);
}

fn tab_count(h: &Harness) -> usize {
    h.state().tab_info().len()
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xenify-tabs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn each_tab_keeps_its_own_document_selection_and_time() {
    let mut h = harness(FakeLog::default());
    // Edit the demo so it isn't replaced by the new document.
    let demo_layer = ids(&h)[1];
    h.state_mut().select_only(Some(demo_layer));
    h.state_mut().set_time(1.5);
    h.key_press(Key::Delete);
    h.run_steps(2);
    let demo_count = ids(&h).len();

    new_blank(&mut h);
    assert_eq!(tab_count(&h), 2);
    assert!(ids(&h).is_empty(), "the new document is empty");
    assert_eq!(h.state().time, 0.0);
    assert_eq!(h.state().comp().settings.width, 1920);

    h.key_press_modifiers(Modifiers::COMMAND, Key::Tab);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), demo_count, "back on the demo");
    assert_eq!(h.state().time, 1.5);
    // Undo history belongs to its document.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), demo_count + 1);
}

#[test]
fn an_untouched_document_is_replaced_not_kept() {
    let mut h = harness(FakeLog::default());
    new_blank(&mut h);
    assert_eq!(tab_count(&h), 1, "the untouched demo made way for the new document");
}

#[test]
fn closing_an_unsaved_tab_asks_first() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Delete);
    h.run_steps(2);
    new_blank(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Tab);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(2);
    h.get_by_label_contains("Save changes to");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(tab_count(&h), 2, "cancel keeps it");
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(2);
    h.get_by_label("Don't Save").click();
    h.run_steps(2);
    assert_eq!(tab_count(&h), 1);
    assert!(ids(&h).is_empty(), "the blank document is left");
}

#[test]
fn opening_a_file_twice_switches_to_its_tab_and_reveal_uses_the_file_manager() {
    let log = FakeLog::default();
    let mut h = harness(log.clone());
    let path = temp_file("twice.xenify");
    h.state_mut().save_to(path.clone());
    h.run_steps(1);
    new_blank(&mut h);
    // Make the blank one non-pristine so opening doesn't replace it.
    h.state_mut().editor.mark_unsaved();
    assert_eq!(tab_count(&h), 2);
    h.state_mut().open(path.clone());
    h.run_steps(2);
    assert_eq!(tab_count(&h), 2, "no second copy");
    assert_eq!(h.state().file.as_ref(), Some(&path));

    h.get_by_label("twice").click_secondary();
    h.run_steps(2);
    h.get_by_label("Reveal in File Manager").click();
    h.run_steps(2);
    assert_eq!(log.with(|s| s.revealed.clone()), vec![path]);
}

#[test]
fn new_from_the_clipboard_sizes_the_canvas_to_the_image() {
    let log = FakeLog::default();
    log.with(|s| s.clipboard = Some(ClipContent::Image { width: 30, height: 20, rgba: vec![128; 2400] }));
    let mut h = harness(log);
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(2);
    h.get_by_label("From clipboard (30×20)");
    h.get_by_label("Create").click();
    h.run_steps(3);
    let s = &h.state().comp().settings;
    assert_eq!((s.width, s.height), (30, 20));
    let layers = &h.state().comp().layers;
    assert_eq!(layers.len(), 1);
    assert!(matches!(layers[0].kind, LayerKind::Image { .. }));
    assert_eq!(layers[0].transform.position.value_at(0.0), xenify_doc::Vec2::new(15.0, 10.0));
    assert!(!h.state().editor.is_dirty(), "a fresh document, nothing to save yet");
}

#[test]
fn files_named_on_the_command_line_open() {
    let mut h = harness(FakeLog::default());
    let path = temp_file("from-cli.xenify");
    let mut p = xenify_doc::Project::new();
    p.compositions[0].settings.name = "From the command line".into();
    std::fs::write(&path, p.to_json()).unwrap();
    let png = temp_file("pic.png");
    std::fs::write(&png, xenify_render::RgbaImage { width: 4, height: 4, pixels: vec![255; 64] }.to_png()).unwrap();
    h.state_mut().open_from_command_line(vec![path.clone(), png]);
    h.run_steps(2);
    let tabs = h.state().tab_info();
    assert_eq!(tabs.len(), 2, "the project (replacing the untouched demo) and a new document for the picture");
    assert_eq!(h.state().comp().layers.len(), 1, "the picture was imported");
    assert!(!h.state().editor.is_dirty(), "a fresh import isn't an unsaved change");
    h.state_mut().cycle_tab(false);
    h.run_steps(2);
    assert_eq!(h.state().file.as_deref(), Some(path.as_path()));
    assert_eq!(h.state().comp().settings.name, "From the command line");
}

#[test]
fn a_slides_size_in_the_new_dialog_makes_a_presentation() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(3);
    h.get_by_label("Slides").click();
    h.run_steps(2);
    h.get_by_label_contains("Slide 16:9").click();
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(3);
    assert_eq!(tab_count(&h), 1, "it replaces the untouched demo");
    assert_eq!(h.state().project().slides.len(), 1, "a one-slide deck");
    assert!(xenify_ui::testing::panel_open(h.state(), "Slides"));
    assert!(h.state().dialog.is_none());
}

#[test]
fn the_new_dialog_shows_paper_sizes_to_scale_and_turns_them() {
    let mut h = harness(FakeLog::default());
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(3);
    h.get_by_label("Print").click();
    h.run_steps(2);
    h.get_by_label("A3").click();
    h.run_steps(2);
    let size = |h: &Harness| xenify_ui::testing::new_dialog_size(h.state()).expect("the New dialog is open");
    assert_eq!(size(&h), (3508, 4961), "A3 at 300 ppi");
    h.get_by_label("Landscape").click();
    h.run_steps(2);
    assert_eq!(size(&h), (4961, 3508));
    h.get_by_label("For screens (96 ppi)").click();
    h.run_steps(2);
    assert_eq!(size(&h), (1587, 1123));
    // The preview names it, beside the sizes it's compared with.
    h.get_by_label("Posters and drawings: two A4s side by side");
    h.get_by_label("1587 × 1123 at 96 ppi");
}

#[test]
fn the_launch_dialog_offers_recent_files_and_the_example() {
    let mut h = harness(FakeLog::default());
    h.state_mut().session.recent = vec![std::path::PathBuf::from("/tmp/nowhere/poster.xenify")];
    xenify_ui::testing::open_launch_dialog(h.state_mut());
    h.run_steps(4);
    h.get_by_label("Open a File…");
    h.get_by_label("poster"); // the recent file, by name
    // The example is what's open behind the dialog: choosing it just closes it.
    h.get_by_label("Bouncing ball").click();
    h.run_steps(3);
    assert!(!xenify_ui::testing::dialog_open(h.state()));
    assert_eq!(tab_count(&h), 1);
    // File › New doesn't show the column.
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(3);
    assert!(h.query_by_label("Bouncing ball").is_none());
}
