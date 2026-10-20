//! What screen readers get: the page as a text field whose lines, caret and
//! selection follow the document.

use super::*;
use egui::accesskit::Role;
use egui_kittest::kittest::NodeT;

#[test]
fn screen_readers_read_the_document_and_follow_the_caret() {
    let mut h = harness();
    type_text(&mut h, "Hello world\nSecond line");
    let doc = h.get_by_role_and_label(Role::MultilineTextInput, "Document");
    let node = doc.accesskit_node();
    assert!(node.supports_text_ranges());
    assert_eq!(node.document_range().text(), "Hello world\nSecond line\n");
    // The caret is at the end of "Second line".
    let focus = node.text_selection_focus().expect("caret");
    assert_eq!(focus.to_global_usv_index(), "Hello world\nSecond line".chars().count());
    assert_eq!(focus.to_line_index(), 1);

    // Moving the caret moves what the screen reader reads.
    press(&mut h, Modifiers::NONE, Key::ArrowUp);
    press(&mut h, Modifiers::NONE, Key::Home);
    press(&mut h, Modifiers::SHIFT, Key::End);
    let doc = h.get_by_role_and_label(Role::MultilineTextInput, "Document");
    let node = doc.accesskit_node();
    assert_eq!(node.text_selection().expect("selection").text(), "Hello world");
}

#[test]
fn long_paragraphs_read_line_by_line() {
    let mut h = harness();
    let words = "accessible ".repeat(40);
    type_text(&mut h, words.trim_end());
    let doc = h.get_by_role_and_label(Role::MultilineTextInput, "Document");
    let node = doc.accesskit_node();
    // Wrapped onto several lines, and every word is there once.
    let runs = node.children().filter(|c| c.role() == Role::TextRun).count();
    assert!(runs >= 3, "{runs} lines");
    assert_eq!(node.document_range().text().trim_end(), words.trim_end());
    let first = node.line_range_from_index(1).expect("second line");
    assert!(first.start().is_word_start());
}

#[test]
fn accessibility_settings_change_the_interface() {
    let mut h = harness();
    h.state_mut().open_settings(xenify_wp_ui::settings::Section::Appearance);
    h.run_steps(2);
    h.get_by_label("Reduce motion (steady caret, no sliding or fading)").click();
    h.run_steps(2);
    h.get_by_label("High contrast (black and white, stronger outlines)").click();
    h.run_steps(2);
    let ctx = h.ctx.clone();
    assert_eq!(ctx.global_style().animation_time, 0.0);
    let v = ctx.global_style().visuals.clone();
    let expected = if v.dark_mode { egui::Color32::BLACK } else { egui::Color32::WHITE };
    assert_eq!(v.panel_fill, expected);
    assert!(v.widgets.inactive.bg_stroke.width >= 1.5);
}
