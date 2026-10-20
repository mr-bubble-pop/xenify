//! Typing on the canvas: new text, editing existing text, one undo step per
//! session, Esc and clicking away to finish, emptied text going away.

use super::common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_ui::testing::FakeLog;

fn big(h: &mut Harness) {
    h.set_size(egui::vec2(1800.0, 1100.0));
    h.run_steps(3);
}

fn screen(h: &Harness, x: f64, y: f64) -> egui::Pos2 {
    h.state().viewport.comp_to_screen(xenify_doc::kurbo::Point::new(x, y))
}

fn click(h: &mut Harness, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn double_click(h: &mut Harness, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
    }
    h.run_steps(3);
}

fn text_of(h: &Harness, id: xenify_doc::Id) -> Option<String> {
    match &h.state().project().layer(id).ok()?.kind {
        LayerKind::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

fn type_in_field(h: &mut Harness, s: &str) {
    h.get_by(|n| n.role() == Role::MultilineTextInput && n.is_focused()).type_text(s);
    h.run_steps(2);
}

#[test]
fn clicking_with_the_text_tool_types_straight_onto_the_canvas() {
    let mut h = harness(FakeLog::default());
    big(&mut h);
    h.state_mut().set_tool(xenify_ui::testing::Tool::Text);
    let before = ids(&h).len();
    let at = screen(&h, 900.0, 600.0);
    click(&mut h, at);
    assert_eq!(ids(&h).len(), before + 1, "a new text layer");
    let id = h.state().selected.unwrap();
    assert!(h.state().in_place.is_some(), "typing in place");
    // The placeholder is selected: typing replaces it.
    type_in_field(&mut h, "Hello");
    assert_eq!(text_of(&h, id).as_deref(), Some("Hello"));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().in_place.is_none(), "Esc finishes");
    assert_eq!(h.state().tool, xenify_ui::testing::Tool::Text, "the Text tool stays on");
}

#[test]
fn double_clicking_text_edits_it_in_one_undo_step() {
    let mut h = harness(FakeLog::default());
    big(&mut h);
    let title = h.state().comp().layers.iter().find(|l| matches!(l.kind, LayerKind::Text(_))).unwrap().clone();
    let original = text_of(&h, title.id).unwrap();
    let b = xenify_ui::testing::world_box(h.state(), title.id);
    let at = screen(&h, b.center().x, b.center().y);
    double_click(&mut h, at);
    let edit = h.state().in_place.clone().expect("editing in place");
    assert_eq!(edit.layer, title.id);
    // No "Text" placeholder here: the cursor goes to the end.
    h.key_press(egui::Key::End);
    h.run_steps(1);
    type_in_field(&mut h, "!!");
    type_in_field(&mut h, "?");
    assert_eq!(text_of(&h, title.id), Some(format!("{original}!!?")));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(text_of(&h, title.id), Some(original), "one undo takes the whole session back");
}

#[test]
fn clicking_away_finishes_and_emptied_text_goes_away() {
    let mut h = harness(FakeLog::default());
    big(&mut h);
    h.state_mut().set_tool(xenify_ui::testing::Tool::Text);
    let at = screen(&h, 900.0, 600.0);
    click(&mut h, at);
    let id = h.state().selected.unwrap();
    // Delete the placeholder (it's selected), then click elsewhere.
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
    h.state_mut().set_tool(xenify_ui::testing::Tool::Select);
    let at = screen(&h, 50.0, 50.0);
    click(&mut h, at);
    assert!(h.state().in_place.is_none(), "clicking away finishes");
    assert!(h.state().project().layer(id).is_err(), "empty text isn't kept");
}

/// A look at typing on the canvas: `SHOT_DIR=dir cargo test -p xenify-ui
/// --test gui text_in_place::screenshot -- --ignored` writes PNGs.
#[test]
#[ignore]
fn screenshot() {
    let Some(dir) = std::env::var_os("SHOT_DIR").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness(FakeLog::default());
    big(&mut h);
    let title = h.state().comp().layers.iter().find(|l| matches!(l.kind, LayerKind::Text(_))).unwrap().id;
    let b = xenify_ui::testing::world_box(h.state(), title);
    let at = screen(&h, b.center().x, b.center().y);
    double_click(&mut h, at);
    h.key_press(egui::Key::End);
    type_in_field(&mut h, " world");
    h.run_steps(4);
    let img = h.render().expect("a picture");
    img.save(dir.join("text-in-place.png")).unwrap();
    // A new box text with wrapping.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().set_tool(xenify_ui::testing::Tool::Text);
    let (a, bb) = (screen(&h, 150.0, 450.0), screen(&h, 500.0, 650.0));
    h.drag_at(a);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(a + (bb - a) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(bb);
    h.run_steps(3);
    type_in_field(&mut h, "A box of text that wraps onto more lines as you type");
    h.run_steps(4);
    h.render().unwrap().save(dir.join("text-box-in-place.png")).unwrap();
}

#[test]
fn double_clicking_a_diagram_shape_types_its_label() {
    let mut h = harness(FakeLog::default());
    big(&mut h);
    // A diagram canvas with one process box.
    let comp = h.state().comp;
    let mut settings = h.state().comp().settings.clone();
    xenify_ops::diagram::set_kind(&mut settings, xenify_doc::CompKind::Diagram);
    h.state_mut().exec("diagram", xenify_ops::Command::SetCompSettings { comp, settings });
    let (cmd, id) = xenify_ops::library::insert(
        h.state().project(),
        comp,
        "flowchart.process",
        xenify_doc::Vec2::new(1000.0, 300.0),
        Some(xenify_doc::Vec2::new(240.0, 120.0)),
        Some(""),
    )
    .unwrap();
    h.state_mut().exec("add", cmd);
    h.state_mut().select_only(None);
    h.run_steps(2);
    let at = screen(&h, 1000.0, 300.0);
    double_click(&mut h, at);
    let edit = h.state().in_place.clone().expect("typing the label");
    assert!(edit.label && edit.layer == id);
    type_in_field(&mut h, "Go");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    let label = h.state().project().layer(id).unwrap().meta.label.clone().map(|l| l.text);
    assert_eq!(label.as_deref(), Some("Go"));
    assert!(h.state().in_place.is_none());
}
