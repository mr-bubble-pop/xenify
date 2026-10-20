//! The AI panel: review, rewrite, continue writing and chat, through
//! OpenRouter (the user's own key; the model is configurable).

use crate::app::WriteApp;
use egui::{Color32, RichText};
use xenify_openrouter::{AiSettings, BRAIN_PRESETS};
use xenify_wp_ai::{Focus, Message, Piece, Proposal, Suggestion, Voice};
use xenify_wp_doc::{NodeId, Pos, Selection, ops};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

pub const AI_COLOR: Color32 = Color32::from_rgb(0xe8, 0x71, 0x0a);

/// Sends a chat-completions body and returns the reply (tests plug in a fake).
pub type Transport = Arc<dyn Fn(&Value, &mut dyn FnMut(&str)) -> Result<Value, String> + Send + Sync>;

fn openrouter(settings: AiSettings) -> Transport {
    Arc::new(move |body: &Value, on_text: &mut dyn FnMut(&str)| {
        let client = xenify_openrouter::OpenRouter::new(&settings).map_err(|e| e.to_string())?;
        if body["stream"].as_bool() == Some(true) {
            let mut body = body.clone();
            body.as_object_mut().map(|o| o.remove("stream"));
            client.chat_stream(&body, on_text).map_err(|e| e.to_string())
        } else {
            client.chat(body).map_err(|e| e.to_string())
        }
    })
}

#[derive(Clone, Debug)]
enum JobKind {
    Review,
    Rewrite { paras: Vec<(NodeId, Range<usize>, String)>, instruction: String },
    Continue { at: Pos },
    Chat,
}

struct Job {
    kind: JobKind,
    rx: Receiver<Result<Value, String>>,
    streamed: Arc<Mutex<String>>,
}

/// A rewrite waiting to be accepted: per paragraph, the old and new text.
pub struct RewritePreview {
    pub parts: Vec<(NodeId, Range<usize>, String, String)>,
    pub instruction: String,
}

/// What to do once the user agrees to send text to the AI service.
type ConsentAction = Box<dyn FnOnce(&mut WriteApp) + Send>;
pub struct Assistant {
    pub settings: AiSettings,
    pub transport: Option<Transport>,
    key_input: String,
    /// The user agreed to send text to the AI service.
    pub consent: bool,
    asking_consent: Option<ConsentAction>,
    job: Option<Job>,
    pub suggestions: Vec<Suggestion>,
    pub rewrite: Option<RewritePreview>,
    /// Text written by "Continue", waiting to be inserted at `Pos`.
    pub ghost: Option<(Pos, String)>,
    pub chat: Vec<Message>,
    pub proposals: Vec<Proposal>,
    chat_input: String,
    hint: String,
    custom: String,
    pub focus: BTreeSet<u8>,
    pub status: Option<(String, bool)>,
}

impl Default for Assistant {
    fn default() -> Self {
        Self {
            settings: AiSettings::load(),
            transport: None,
            key_input: String::new(),
            consent: false,
            asking_consent: None,
            job: None,
            suggestions: Vec::new(),
            rewrite: None,
            ghost: None,
            chat: Vec::new(),
            proposals: Vec::new(),
            chat_input: String::new(),
            hint: String::new(),
            custom: String::new(),
            focus: [0u8, 1, 2, 3].into_iter().collect(),
            status: None,
        }
    }
}

fn focus_list(set: &BTreeSet<u8>) -> Vec<Focus> {
    Focus::ALL.iter().enumerate().filter(|(i, _)| set.contains(&(*i as u8))).map(|(_, f)| *f).collect()
}

impl Assistant {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn model(&self) -> String {
        self.settings.writer_model().to_string()
    }

    fn start(&mut self, ctx: &egui::Context, kind: JobKind, body: Value) {
        let transport = self.transport.clone().unwrap_or_else(|| openrouter(self.settings.clone()));
        let (tx, rx) = channel();
        let streamed = Arc::new(Mutex::new(String::new()));
        let s2 = streamed.clone();
        let ctx2 = ctx.clone();
        std::thread::Builder::new()
            .name("xenify-write-ai".into())
            .spawn(move || {
                let r = transport(&body, &mut |t| {
                    s2.lock().unwrap_or_else(|e| e.into_inner()).push_str(t);
                    ctx2.request_repaint();
                });
                let _ = tx.send(r);
                ctx2.request_repaint();
            })
            .expect("thread");
        self.status = Some(("Working…".into(), false));
        self.job = Some(Job { kind, rx, streamed });
    }
}

/// Runs `f` now if the user agreed to AI use, else asks first.
fn with_consent(app: &mut WriteApp, f: impl FnOnce(&mut WriteApp) + Send + 'static) {
    if app.assistant.consent {
        f(app);
    } else {
        app.assistant.asking_consent = Some(Box::new(f));
    }
}

pub fn review(app: &mut WriteApp, ctx: &egui::Context, whole: bool) {
    let ctx = ctx.clone();
    with_consent(app, move |app| {
        let paras: Vec<NodeId> = if whole || app.editor.sel.is_collapsed() {
            app.editor.doc.paragraphs().map(|p| p.id).collect()
        } else {
            ops::selected_paragraphs(&app.editor)
        };
        let body = xenify_wp_ai::review_request(
            &app.assistant.model(),
            &app.editor.doc,
            &paras,
            &focus_list(&app.assistant.focus),
            &Voice::of(&app.editor.doc),
        );
        app.assistant.suggestions.clear();
        app.assistant.start(&ctx, JobKind::Review, body);
    });
}

pub fn rewrite(app: &mut WriteApp, ctx: &egui::Context, instruction: String) {
    let ctx = ctx.clone();
    with_consent(app, move |app| {
        let ed = &app.editor;
        let (a, b) = ed.doc.ordered(ed.sel);
        let paras: Vec<(NodeId, Range<usize>, String)> = ops::selected_paragraphs(ed)
            .into_iter()
            .filter_map(|id| {
                let p = ed.doc.para(id)?;
                let s = if id == a.para { a.offset } else { 0 };
                let e = if id == b.para { b.offset } else { p.text.len() };
                // Nothing selected: the caret's whole paragraph.
                let (s, e) = if a == b { (0, p.text.len()) } else { (s, e) };
                (s < e).then(|| (id, s..e, p.text[s..e].to_string()))
            })
            .collect();
        if paras.is_empty() {
            app.assistant.status = Some(("Select some text to rewrite.".into(), true));
            return;
        }
        let text = paras.iter().map(|p| p.2.as_str()).collect::<Vec<_>>().join("\n");
        let body =
            xenify_wp_ai::rewrite_request(&app.assistant.model(), &text, &instruction, &Voice::of(&app.editor.doc));
        app.assistant.rewrite = None;
        app.assistant.start(&ctx, JobKind::Rewrite { paras, instruction }, body);
    });
}

pub fn continue_writing(app: &mut WriteApp, ctx: &egui::Context) {
    let ctx = ctx.clone();
    with_consent(app, move |app| {
        let at = app.editor.sel.focus;
        // The text up to the caret.
        let mut before = String::new();
        for p in app.editor.doc.paragraphs() {
            if p.id == at.para {
                before.push_str(&p.text[..at.offset.min(p.text.len())]);
                break;
            }
            before.push_str(&p.text);
            before.push_str("\n\n");
        }
        let mut body = xenify_wp_ai::continue_request(
            &app.assistant.model(),
            &before,
            &app.assistant.hint,
            &Voice::of(&app.editor.doc),
        );
        body["stream"] = Value::Bool(true);
        app.assistant.ghost = Some((at, String::new()));
        app.assistant.start(&ctx, JobKind::Continue { at }, body);
    });
}

fn send_chat(app: &mut WriteApp, ctx: &egui::Context) {
    let text = std::mem::take(&mut app.assistant.chat_input);
    if text.trim().is_empty() {
        return;
    }
    let ctx = ctx.clone();
    with_consent(app, move |app| {
        app.assistant.chat.push(Message { role: "user".into(), text });
        let sel = app.editor.doc.text_in(app.editor.sel);
        let body = xenify_wp_ai::chat_request(
            &app.assistant.model(),
            &app.editor.doc,
            &sel,
            true,
            &app.assistant.chat,
            &Voice::of(&app.editor.doc),
        );
        app.assistant.start(&ctx, JobKind::Chat, body);
    });
}

/// Picks up finished requests (call every frame).
pub fn poll(app: &mut WriteApp) {
    let Some(job) = &app.assistant.job else { return };
    if let JobKind::Continue { at } = job.kind {
        let so_far = job.streamed.lock().unwrap_or_else(|e| e.into_inner()).clone();
        app.assistant.ghost = Some((at, so_far));
    }
    let result = match job.rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(_) => Err("the request stopped unexpectedly".into()),
    };
    let job = app.assistant.job.take().expect("checked");
    let a = &mut app.assistant;
    a.status = None;
    let resp = match result {
        Ok(r) => r,
        Err(e) => {
            a.status = Some((e, true));
            if matches!(job.kind, JobKind::Continue { .. }) {
                a.ghost = None;
            }
            return;
        }
    };
    match job.kind {
        JobKind::Review => {
            let (found, dropped) = xenify_wp_ai::parse_review(&resp, &app.editor.doc);
            let n = found.len();
            app.assistant.suggestions = found;
            app.assistant.status = Some((
                match (n, dropped) {
                    (0, 0) => "No problems found.".into(),
                    (n, 0) => format!("{n} suggestions."),
                    (n, d) => format!("{n} suggestions ({d} couldn't be matched to the text and were left out)."),
                },
                false,
            ));
        }
        JobKind::Rewrite { paras, instruction } => {
            let new = xenify_wp_ai::reply_text(&resp);
            let pieces: Vec<&str> = new.split("\n\n").map(str::trim).filter(|s| !s.is_empty()).collect();
            let parts = if pieces.len() == paras.len() {
                paras.into_iter().zip(pieces).map(|((id, r, old), n)| (id, r, old, n.to_string())).collect()
            } else {
                // Different paragraph count: offer it all as one replacement of the first part.
                let mut v: Vec<_> = paras.into_iter().map(|(id, r, old)| (id, r, old, String::new())).collect();
                if let Some(first) = v.first_mut() {
                    first.3 = new.clone();
                }
                v
            };
            app.assistant.rewrite = Some(RewritePreview { parts, instruction });
        }
        JobKind::Continue { at } => {
            let text = xenify_wp_ai::reply_text(&resp);
            let text =
                if text.is_empty() { job.streamed.lock().unwrap_or_else(|e| e.into_inner()).clone() } else { text };
            app.assistant.ghost = Some((at, text));
        }
        JobKind::Chat => {
            let (text, props) = xenify_wp_ai::parse_chat(&resp);
            let shown = if text.is_empty() && !props.is_empty() { "Here's a suggestion:".to_string() } else { text };
            app.assistant.chat.push(Message { role: "assistant".into(), text: shown });
            app.assistant.proposals.extend(props);
        }
    }
}

/// Accepts the whole rewrite as one undo step per paragraph.
pub fn accept_rewrite(app: &mut WriteApp) {
    let Some(pre) = app.assistant.rewrite.take() else { return };
    app.editor.seal();
    for (id, range, old, new) in pre.parts.iter().rev() {
        if new.is_empty() && pre.parts.len() > 1 {
            // Merged into the first part: remove this one.
            let _ = ops::apply_edits(&mut app.editor, *id, &[(range.clone(), String::new())], "Rewrite");
            continue;
        }
        let edits: Vec<(Range<usize>, String)> = xenify_wp_ai::edits(old, new)
            .into_iter()
            .map(|(r, t)| (r.start + range.start..r.end + range.start, t))
            .collect();
        let _ = ops::apply_edits(&mut app.editor, *id, &edits, "Rewrite");
    }
}

fn accept_suggestion(app: &mut WriteApp, i: usize) {
    let s = app.assistant.suggestions.remove(i);
    if let Some(r) = &s.replacement
        && app.editor.doc.para(s.para).is_some_and(|p| p.text.get(s.range.clone()).is_some())
    {
        let _ = ops::replace_text(&mut app.editor, s.para, s.range.clone(), r, "AI suggestion");
        // Suggestions overlapping the changed text no longer apply; later
        // ones in that paragraph move with the text.
        app.assistant
            .suggestions
            .retain(|o| o.para != s.para || o.range.end <= s.range.start || o.range.start >= s.range.end);
        let delta = r.len() as isize - s.range.len() as isize;
        for o in &mut app.assistant.suggestions {
            if o.para == s.para && o.range.start >= s.range.end {
                o.range = (o.range.start as isize + delta) as usize..(o.range.end as isize + delta) as usize;
            }
        }
    }
}

fn apply_proposal(app: &mut WriteApp, p: &Proposal) -> Result<(), String> {
    if p.quote.is_empty() {
        app.editor.seal();
        ops::insert_text(&mut app.editor, &p.replacement.replace("\n\n", "\n"), None).map_err(|e| e.to_string())
    } else {
        let (para, range) =
            xenify_wp_ai::locate(&app.editor.doc, &p.quote).ok_or("that text isn't in the document any more")?;
        ops::replace_text(&mut app.editor, para, range, &p.replacement, "AI edit").map_err(|e| e.to_string())
    }
}

fn diff_label(ui: &mut egui::Ui, old: &str, new: &str) {
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::proportional(13.0);
    for piece in xenify_wp_ai::word_diff(old, new) {
        let (text, fmt) = match &piece {
            Piece::Same(t) => {
                (t, egui::TextFormat { font_id: font.clone(), color: ui.visuals().text_color(), ..Default::default() })
            }
            Piece::Removed(t) => (
                t,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: Color32::from_rgb(0xe5, 0x73, 0x73),
                    strikethrough: egui::Stroke::new(1.0, Color32::from_rgb(0xe5, 0x73, 0x73)),
                    ..Default::default()
                },
            ),
            Piece::Added(t) => (
                t,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: Color32::from_rgb(0x66, 0xbb, 0x6a),
                    ..Default::default()
                },
            ),
        };
        job.append(text, 0.0, fmt);
    }
    job.wrap.max_width = ui.available_width();
    ui.label(job);
}

/// The AI side panel.
/// Settings › AI assistant: the key and the model.
pub fn settings(app: &mut WriteApp, ui: &mut egui::Ui) {
    use crate::settings::group;
    let has_key = app.assistant.settings.resolve_api_key().is_some() || app.assistant.transport.is_some();
    group(ui, "Your OpenRouter key", |ui| {
        if has_key {
            ui.label(format!("A key is set up ({}).", app.assistant.settings.api_key_source()));
            if AiSettings::has_stored_api_key() && ui.button("Forget the saved key").clicked() {
                let _ = AiSettings::forget_api_key();
            }
        } else {
            ui.label("The assistant uses OpenRouter. Get a key at openrouter.ai/keys and paste it here.");
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.assistant.key_input)
                    .password(true)
                    .hint_text(if has_key { "Replace with a new key" } else { "sk-or-…" })
                    .desired_width(240.0),
            );
            if ui.add_enabled(!app.assistant.key_input.trim().is_empty(), egui::Button::new("Save key")).clicked() {
                match AiSettings::store_api_key(app.assistant.key_input.trim()) {
                    Ok(loc) => app.assistant.status = Some((format!("Key saved ({}).", loc.describe()), false)),
                    Err(e) => app.assistant.status = Some((e, true)),
                }
                app.assistant.key_input.clear();
            }
        });
        if let Some((msg, err)) = &app.assistant.status {
            let t = RichText::new(msg);
            ui.label(if *err { t.color(ui.visuals().error_fg_color) } else { t });
        }
    });
    group(ui, "Model", |ui| {
        let mut model = app.assistant.settings.writer_model().to_string();
        egui::ComboBox::from_id_salt("ai-model").selected_text(&model).width(300.0).show_ui(ui, |ui| {
            for p in BRAIN_PRESETS {
                ui.selectable_value(&mut model, p.id.to_string(), format!("{}  ({})", p.id, p.note));
            }
        });
        egui::CollapsingHeader::new("Advanced").id_salt("ai-advanced").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Another OpenRouter model:");
                ui.text_edit_singleline(&mut model);
            });
        });
        if model != app.assistant.settings.writer_model() {
            app.assistant.settings.writer_model = model;
            let _ = app.assistant.settings.save();
        }
    });
    group(ui, "Your text", |ui| {
        ui.weak("Only the text you ask about is sent, and only to providers that don't keep a copy of it.");
    });
}

/// Who the document is for and its tone (saved in the document; the AI
/// writes and reviews for them).
pub fn audience(app: &mut WriteApp, ui: &mut egui::Ui) {
    egui::Grid::new("ai-audience").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("For");
        if ui
            .add(
                egui::TextEdit::singleline(&mut app.editor.doc.meta.audience)
                    .hint_text("e.g. new customers")
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            app.editor.doc.revision += 1;
        }
        ui.end_row();
        ui.label("Tone");
        if ui
            .add(
                egui::TextEdit::singleline(&mut app.editor.doc.meta.tone)
                    .hint_text("e.g. friendly, confident")
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            app.editor.doc.revision += 1;
        }
        ui.end_row();
    });
}

/// The AI tools: review, rewrite, write, and a chat about the document.
pub fn tools(app: &mut WriteApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let has_key = app.assistant.settings.resolve_api_key().is_some() || app.assistant.transport.is_some();
    if !has_key {
        ui.label("The AI assistant needs an OpenRouter key.");
        if ui.button("Set up the AI assistant…").clicked() {
            app.open_settings(crate::settings::Section::Assistant);
        }
        return;
    }
    if let Some((msg, err)) = &app.assistant.status {
        let t = RichText::new(msg);
        ui.label(if *err { t.color(ui.visuals().error_fg_color) } else { t });
    }
    let busy = app.assistant.busy();
    if busy {
        ui.spinner();
    }
    ui.separator();

    // Review: what it looks at follows the kind of document and its audience.
    ui.label(RichText::new("Review").strong());
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy && has_key, egui::Button::new("Review selection")).clicked() {
            review(app, &ctx, false);
        }
        if ui.add_enabled(!busy && has_key, egui::Button::new("Review document")).clicked() {
            review(app, &ctx, true);
        }
    });
    let mut accept = None;
    let mut dismiss = None;
    egui::ScrollArea::vertical().id_salt("ai-suggestions").max_height(220.0).show(ui, |ui| {
        for (i, s) in app.assistant.suggestions.iter().enumerate() {
            let text = app.editor.doc.para(s.para).and_then(|p| p.text.get(s.range.clone())).unwrap_or("").to_string();
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(format!("{} · {}", s.category, s.severity)).small().color(AI_COLOR));
                match &s.replacement {
                    Some(r) => diff_label(ui, &text, r),
                    None => {
                        ui.label(RichText::new(&text).italics());
                    }
                }
                ui.label(&s.explanation);
                ui.horizontal(|ui| {
                    if s.replacement.is_some() && ui.button("Accept").clicked() {
                        accept = Some(i);
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = Some(i);
                    }
                    if ui.button("Show").clicked() {
                        app.editor.sel =
                            Selection { anchor: Pos::new(s.para, s.range.start), focus: Pos::new(s.para, s.range.end) };
                        app.view.reveal();
                    }
                });
            });
        }
    });
    if let Some(i) = accept {
        accept_suggestion(app, i);
    }
    if let Some(i) = dismiss {
        app.assistant.suggestions.remove(i);
    }
    ui.separator();

    // Rewrite.
    ui.label(RichText::new("Rewrite the selection").strong());
    ui.horizontal_wrapped(|ui| {
        for (name, instruction) in xenify_wp_ai::REWRITES {
            if ui.add_enabled(!busy && has_key, egui::Button::new(name)).clicked() {
                rewrite(app, &ctx, instruction.to_string());
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.assistant.custom).hint_text("Or say how…").desired_width(180.0));
        if ui
            .add_enabled(!busy && has_key && !app.assistant.custom.trim().is_empty(), egui::Button::new("Rewrite"))
            .clicked()
        {
            let i = app.assistant.custom.clone();
            rewrite(app, &ctx, i);
        }
    });
    if let Some(pre) = &app.assistant.rewrite {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            for (_, _, old, new) in &pre.parts {
                if !new.is_empty() {
                    diff_label(ui, old, new);
                }
            }
        });
        let instruction = pre.instruction.clone();
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Accept").strong()).clicked() {
                accept_rewrite(app);
            }
            if ui.add_enabled(!busy, egui::Button::new("Try again")).clicked() {
                rewrite(app, &ctx, instruction);
            }
            if ui.button("Discard").clicked() {
                app.assistant.rewrite = None;
            }
        });
    }
    ui.separator();

    // Continue writing.
    ui.label(RichText::new("Write").strong());
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.assistant.hint)
                .hint_text("What to write (optional)")
                .desired_width(180.0),
        );
        if ui
            .add_enabled(!busy && has_key, egui::Button::new("Continue"))
            .on_hover_text("Write from the caret (Ctrl+Shift+Enter)")
            .clicked()
        {
            continue_writing(app, &ctx);
        }
    });
    if let Some((at, text)) = app.assistant.ghost.clone() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new(&text).color(Color32::from_gray(160)).italics());
        });
        if !busy {
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Insert text").strong()).clicked() && !text.is_empty() {
                    app.editor.sel = Selection::caret(app.editor.doc.clamp(at));
                    app.editor.seal();
                    let _ = ops::insert_text(&mut app.editor, &text.replace("\n\n", "\n"), None);
                    app.assistant.ghost = None;
                }
                if ui.button("Discard").clicked() {
                    app.assistant.ghost = None;
                }
            });
        }
    }
    ui.separator();

    // Chat.
    let _ = egui::CollapsingHeader::new("Ask about the document").id_salt("ai-chat").show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("chat").max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
            for m in &app.assistant.chat {
                let who = if m.role == "user" { "You" } else { "Assistant" };
                ui.label(RichText::new(who).small().strong());
                ui.label(&m.text);
            }
            let mut apply = None;
            let mut drop_p = None;
            for (i, p) in app.assistant.proposals.iter().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(&p.reason).small().color(AI_COLOR));
                    if p.quote.is_empty() {
                        ui.label(RichText::new(format!("Insert: {}", p.replacement)).italics());
                    } else {
                        diff_label(ui, &p.quote, &p.replacement);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            apply = Some(i);
                        }
                        if ui.button("Dismiss").clicked() {
                            drop_p = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = apply {
                let p = app.assistant.proposals.remove(i);
                if let Err(e) = apply_proposal(app, &p) {
                    app.assistant.status = Some((e, true));
                }
            }
            if let Some(i) = drop_p {
                app.assistant.proposals.remove(i);
            }
        });
        let r = ui.add(
            egui::TextEdit::multiline(&mut app.assistant.chat_input)
                .hint_text("Ask about the document… (Enter to send)")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        ui.horizontal(|ui| {
            if (ui.add_enabled(!busy && has_key, egui::Button::new("Send")).clicked() || enter && !busy && has_key)
                && !app.assistant.chat_input.trim().is_empty()
            {
                let t = app.assistant.chat_input.trim_end_matches('\n').to_string();
                app.assistant.chat_input = t;
                send_chat(app, &ctx);
            }
            if ui.button("Clear chat").clicked() {
                app.assistant.chat.clear();
                app.assistant.proposals.clear();
            }
        });
    });
}

/// The one-time notice before any text is sent.
pub fn consent_dialog(app: &mut WriteApp, ctx: &egui::Context) {
    if app.assistant.asking_consent.is_none() {
        return;
    }
    let mut decision = None;
    egui::Modal::new(egui::Id::new("ai-consent")).show(ctx, |ui| {
        ui.heading("Send text to the AI service?");
        ui.label("The AI features send the text you choose (and the document when reviewing or chatting) to OpenRouter and the model provider you picked, using your API key.");
        ui.label("With your account's zero-data-retention setting, providers don't keep it. Nothing is sent until you use an AI feature.");
        use crate::dialog::Kind;
        decision = crate::dialog::buttons(ui, &[("Continue", Kind::Main), ("Cancel", Kind::Cancel)]).map(|i| i == 0);
    });
    match decision {
        Some(true) => {
            app.assistant.consent = true;
            if let Some(f) = app.assistant.asking_consent.take() {
                f(app);
            }
        }
        Some(false) => app.assistant.asking_consent = None,
        None => {}
    }
}

/// The ranges AI review suggestions cover (for the orange underline).
pub fn marked(app: &WriteApp) -> Vec<(NodeId, Range<usize>)> {
    app.assistant.suggestions.iter().map(|s| (s.para, s.range.clone())).collect()
}
