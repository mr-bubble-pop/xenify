//! AI ▸ New From a Description, New Slide and Modify This Slide, with a
//! scripted model (nothing is sent to OpenRouter).

use super::common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::FakeLog;
use serde_json::{Value, json};

fn tool_calls(calls: &[(&str, &str, Value)]) -> Value {
    let calls: Vec<Value> = calls
        .iter()
        .map(|(id, name, args)| {
            json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}})
        })
        .collect();
    json!({"choices": [{"message": {"role": "assistant", "content": "", "tool_calls": calls}}]})
}

fn reply(text: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": text}}]})
}

fn text_layer(name: &str, y: f64) -> Value {
    json!({"type": "text", "name": name, "text": name, "position": [960, y], "font_size": 80})
}

/// The user's message in the first request the assistant sent.
fn first_request(script: &std::sync::Arc<std::sync::Mutex<xenify_ui::testing::Script>>) -> String {
    let s = script.lock().unwrap();
    let msgs = s.sent.first().expect("a request was sent")["messages"].as_array().unwrap().clone();
    msgs.iter().filter(|m| m["role"] == "user").map(|m| m["content"].as_str().unwrap_or("").to_string()).collect()
}

fn open_ai_menu_item(h: &mut Harness, label: &str) {
    h.get_by_label("AI").click();
    h.run_steps(2);
    h.get_by_label_contains(label).click();
    h.run_steps(3);
}

#[test]
fn a_described_slide_deck_is_built_in_a_new_tab() {
    let mut h = harness(FakeLog::default());
    let script = xenify_ui::testing::script_ai(
        h.state_mut(),
        vec![
            tool_calls(&[
                ("c1", "add_layer", text_layer("Garden Plan", 540.0)),
                ("c2", "add_slide", json!({"name": "The problem", "transition": "fade"})),
                ("c3", "add_layer", text_layer("No green space", 200.0)),
            ]),
            reply("Here's a two-slide deck."),
        ],
    );
    open_ai_menu_item(&mut h, "New From a Description");
    // The description box has the cursor.
    h.event(egui::Event::Text("A pitch for a community garden".into()));
    h.run_steps(1);
    h.get_by_label("Slide deck").click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(8);

    // (The untouched starting document is replaced rather than kept in a tab.)
    let p = h.state().project();
    assert_eq!(p.comp(p.slides[0].comp).unwrap().layers.len(), 1, "a fresh document");
    assert_eq!(p.slides.len(), 2);
    assert_eq!(p.comp(p.slides[0].comp).unwrap().layers[0].meta.name, "Garden Plan");
    assert_eq!(p.comp(p.slides[1].comp).unwrap().settings.name, "The problem");
    assert_eq!(p.slides[1].transition_in.kind, xenify_doc::TransitionKind::Fade);
    let asked = first_request(&script);
    assert!(asked.contains("slide deck") && asked.contains("community garden"), "{asked}");
    assert!(xenify_ui::testing::panel_open(h.state(), "Assistant"));

    // One reviewable change: rejecting would undo it all; accepting keeps it.
    xenify_ui::testing::accept_ai(h.state_mut());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert!(h.state().project().slides.is_empty(), "one undo removes the whole build");
}

#[test]
fn ai_new_slide_adds_a_slide_and_asks_for_its_content() {
    let mut h = harness(FakeLog::default());
    let script = xenify_ui::testing::script_ai(
        h.state_mut(),
        vec![tool_calls(&[("c1", "add_layer", text_layer("Pricing", 200.0))]), reply("Added a pricing slide.")],
    );
    let first = h.state().comp;
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::M);
    h.run_steps(2);
    h.event(egui::Event::Text("Pricing: three tiers".into()));
    h.get_by_label("Create Slide").click();
    h.run_steps(8);
    let p = h.state().project();
    assert_eq!(p.slides.len(), 2);
    assert_eq!(p.slides[0].comp, first);
    assert_eq!(h.state().comp, p.slides[1].comp, "the new slide is open");
    assert_eq!(h.state().comp().layers.len(), 1);
    assert_eq!(h.state().comp().layers[0].meta.name, "Pricing");
    let asked = first_request(&script);
    assert!(asked.contains("slide 2") && asked.contains("three tiers"), "{asked}");
}

#[test]
fn modify_this_slide_sends_the_change_to_the_assistant() {
    let mut h = harness(FakeLog::default());
    let script = xenify_ui::testing::script_ai(h.state_mut(), vec![reply("Nothing to change.")]);
    let before = h.state().project().clone();
    open_ai_menu_item(&mut h, "Modify This Slide");
    h.event(egui::Event::Text("Make the title bigger".into()));
    h.get_by_label("Apply").click();
    h.run_steps(6);
    let asked = first_request(&script);
    assert!(asked.contains("Make the title bigger") && asked.contains("Only change what's asked"), "{asked}");
    assert_eq!(*h.state().project(), before, "no slide was added");
}

#[test]
fn without_a_key_nothing_is_added_and_settings_open() {
    let mut h = harness(FakeLog::default());
    xenify_ui::testing::pretend_no_ai_key(h.state_mut());
    let slides = h.state().project().slides.len();
    let comps = h.state().project().compositions.len();
    open_ai_menu_item(&mut h, "New Slide");
    h.event(egui::Event::Text("A title slide".into()));
    h.run_steps(1);
    h.get_by_label("Create Slide").click();
    h.run_steps(3);
    assert_eq!(h.state().project().slides.len(), slides);
    assert_eq!(h.state().project().compositions.len(), comps, "no empty slide left behind");
    assert!(h.state().error.as_deref().is_some_and(|e| e.contains("OpenRouter key")), "{:?}", h.state().error);
    assert!(h.state().settings.is_open, "Settings › AI opened");
}

#[test]
fn a_suggestion_goes_in_the_box_to_edit_and_review_lists_the_changes() {
    let mut h = harness(FakeLog::default());
    let _script = xenify_ui::testing::script_ai(
        h.state_mut(),
        vec![tool_calls(&[("c1", "add_layer", text_layer("Hello", 540.0))]), reply("Added a title.")],
    );
    xenify_ui::testing::show_panel(h.state_mut(), "Assistant");
    h.run_steps(3);
    let before = ids(&h).len();
    h.get_by_label_contains("lower-third").click();
    h.run_steps(3);
    assert_eq!(ids(&h).len(), before, "not sent yet");
    let input = h.get_by(|n| n.role() == egui::accesskit::Role::MultilineTextInput);
    assert!(input.value().unwrap_or_default().contains("lower-third"));
    h.get_by_label("Send").click();
    for _ in 0..50 {
        h.run_steps(2);
    }
    assert_eq!(ids(&h).len(), before + 1);
    h.get_by_label_contains("New: Hello");
    h.get_by_label_contains("This request cost $");
}
