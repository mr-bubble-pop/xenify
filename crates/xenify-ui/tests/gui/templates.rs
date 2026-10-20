//! Slide templates: new slides from layouts, Apply Theme, saved layouts,
//! theme roles, and a templated deck from the New dialog.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::{LayerKind, Role};
use xenify_ui::testing::FakeLog;

fn slide_menu(h: &mut Harness, sub: &str) {
    h.get_by_label("Slide").click();
    h.run_steps(2);
    h.get_by_label(&format!("{sub} ⏵")).hover();
    h.run_steps(3);
}

fn subtitle_colour(h: &Harness) -> xenify_doc::Color {
    let l = h.state().comp().layers.iter().find(|l| l.meta.role == Some(Role::Subtitle)).expect("a subtitle");
    let LayerKind::Text(t) = &l.kind else { panic!() };
    t.color.value_at(0.0)
}

#[test]
fn layouts_themes_and_saved_layouts() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1600.0, 1000.0));
    slide_menu(&mut h, "New Slide From Layout");
    h.get_by_label("Big Stat").click();
    h.run_steps(3);
    let p = h.state().project();
    assert_eq!(p.slides.len(), 2, "the canvas became slide 1, the stat slide 2");
    assert!(h.state().comp().layers.iter().any(|l| l.meta.name == "Stat"));
    assert!(xenify_ui::testing::panel_open(h.state(), "Slides"));

    // Apply Theme › Retro recolours it; one undo restores it.
    let before = subtitle_colour(&h);
    slide_menu(&mut h, "Apply Theme");
    h.get_by_label("Retro").click();
    h.run_steps(3);
    let retro = xenify_doc::templates::theme("Retro").unwrap();
    assert_eq!(h.state().project().theme.as_deref(), Some("Retro"));
    assert_eq!(subtitle_colour(&h), retro.subtitle);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(subtitle_colour(&h), before);

    // Save Slide as Layout, then make a slide from it.
    let dir = std::env::temp_dir().join(format!("xenify-gui-layouts-{}", std::process::id()));
    xenify_ui::testing::set_layouts_dir(h.state_mut(), dir.clone());
    slide_menu(&mut h, "Save Slide as Layout");
    let field = h.get_by(|n| n.role() == egui::accesskit::Role::TextInput);
    field.focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by(|n| n.role() == egui::accesskit::Role::TextInput).type_text("My Stat");
    h.run_steps(1);
    h.get_by_label("Save Layout").click();
    h.run_steps(2);
    assert!(dir.join("My Stat.xenify").exists(), "{}", h.state().status);
    slide_menu(&mut h, "New Slide From Layout");
    h.get_by_label("My Stat").click();
    h.run_steps(3);
    assert_eq!(h.state().project().slides.len(), 3);
    assert!(h.state().comp().layers.iter().any(|l| l.meta.name == "Stat"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_new_dialog_makes_a_themed_starter_deck() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1600.0, 1100.0));
    xenify_ui::testing::open_launch_dialog(h.state_mut());
    h.run_steps(10);
    h.get_by_label("Slides").click();
    h.run_steps(3);
    h.get_all_by_label("Bold Gradient").next().unwrap().click();
    h.run_steps(3);
    h.get_by_label("Create").click();
    h.run_steps(5);
    let p = h.state().project();
    assert_eq!(p.slides.len(), 5);
    assert_eq!(p.theme.as_deref(), Some("Bold Gradient"));
    assert!(h.state().status.contains("Bold Gradient"), "{}", h.state().status);
}

#[test]
fn the_showcase_opens_from_the_launch_column_and_help() {
    let mut h = harness(FakeLog::default());
    h.set_size(egui::vec2(1600.0, 1100.0));
    assert!(h.state().project().slides.is_empty(), "tests start from the plain demo");
    xenify_ui::testing::open_launch_dialog(h.state_mut());
    h.run_steps(10);
    h.get_by_label("Showcase").click();
    h.run_steps(4);
    assert_eq!(h.state().project().slides.len(), 11);
    assert!(!xenify_ui::testing::dialog_open(h.state()));
    // Help › Showcase opens it again (a new tab, as this one is open).
    h.get_by_label("Help").click();
    h.run_steps(2);
    h.get_by_label("Showcase").click();
    h.run_steps(4);
    assert_eq!(h.state().project().slides.len(), 11);
}
