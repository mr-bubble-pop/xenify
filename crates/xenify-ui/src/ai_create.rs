//! AI ▸ …: describe a new animation or slide deck, a new slide, or a change
//! to this slide, and the assistant builds it (as one reviewable change).

use crate::app::XenifyApp;
use crate::assistant::Assistant;
use xenify_ai::requests;

/// What a new document described to the assistant should become.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocKind {
    Animation,
    Deck,
}

/// AI ▸ New Slide / Modify This Slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlideTask {
    New,
    Modify,
}

pub struct AiPrompt {
    pub task: SlideTask,
    pub text: String,
}

/// Starts building a new document the assistant was asked for (the new tab is open).
pub fn start_new_document(app: &mut XenifyApp, ctx: &egui::Context, kind: DocKind, description: &str) {
    let (w, h) = (app.comp().settings.width, app.comp().settings.height);
    let (text, steps) = match kind {
        DocKind::Animation => (requests::new_animation(description, w, h), None),
        DocKind::Deck => (requests::new_deck(description, w, h), Some(requests::DECK_STEPS)),
    };
    if Assistant::request(app, ctx, text, true, steps) {
        app.status = match kind {
            DocKind::Animation => "The assistant is building your animation…".into(),
            DocKind::Deck => "The assistant is building your slide deck…".into(),
        };
    }
}

fn run(app: &mut XenifyApp, ctx: &egui::Context, task: SlideTask, text: &str) {
    if app.assistant.busy() {
        app.status = "The assistant is still working — wait for it or press Stop first.".into();
        return;
    }
    // No key: say so before adding a slide that would stay empty.
    if !crate::assistant::ai_ready(app) {
        return;
    }
    if task == SlideTask::New {
        crate::slides_panel::add_after(app, None);
    }
    let slide = app.project().slide_index(app.comp);
    let text = match task {
        SlideTask::New => requests::new_slide(text, slide, app.comp().settings.height),
        SlideTask::Modify => requests::modify_slide(text, slide),
    };
    Assistant::request(app, ctx, text, false, None);
}

/// The dialog's title.
pub fn title_of(d: &AiPrompt) -> &'static str {
    match d.task {
        SlideTask::New => "New slide with AI",
        SlideTask::Modify => "Modify this slide with AI",
    }
}

/// Draws the AI ▸ New Slide / Modify This Slide dialog; returns true when it should close.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, d: &mut AiPrompt) -> bool {
    let (hint, button) = match d.task {
        SlideTask::New => (
            "What should the slide say or show? e.g. “Three benefits of solar power, with an icon for each”",
            "Create Slide",
        ),
        SlideTask::Modify => {
            ("What should change? e.g. “Make the title bigger and have the bullets fly in one by one”", "Apply")
        }
    };
    ui.add(egui::TextEdit::multiline(&mut d.text).hint_text(hint).desired_rows(4).desired_width(420.0)).request_focus();
    ui.weak("The assistant builds it as one change you can review, accept or undo. Ctrl+Enter to send.");
    ui.separator();
    let mut close = false;
    ui.horizontal(|ui| {
        let ready = !d.text.trim().is_empty();
        let send = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
        if ui.add_enabled(ready, crate::widgets::primary(button)).clicked() || (ready && send) {
            let ctx = ui.ctx().clone();
            run(app, &ctx, d.task, &d.text);
            close = true;
        }
        close |= ui.button("Cancel").clicked();
    });
    close
}
