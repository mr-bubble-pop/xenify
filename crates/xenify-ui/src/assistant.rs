//! The AI assistant panel: chat, background requests to OpenRouter, and
//! accept/reject of the assistant's changes (one undo step per turn).

use crate::app::XenifyApp;
use crate::widgets::{ACCENT, RECORD};
use egui::{Color32, RichText};
use xenify_ai::{
    Agent, AiError, AiSettings, BRAIN_PRESETS, ChatItem, ModelInfo, ModelPreset, Next, OpenRouter, Purpose,
    REVIEWER_PRESETS, ToolHost,
};
use xenify_anim::SceneText;
use xenify_doc::{Id, Project};
use xenify_ops::Command;
use xenify_render::Offscreen;
use serde_json::Value as Json;
use std::sync::mpsc;

const SUGGESTIONS: &[&str] = &[
    "A 5 second logo intro for “Nova Labs” with a geometric icon",
    "A lower-third title: “Jane Doe — Product Designer”",
    "A looping loading animation with three bouncing dots",
    "Make everything in this scene feel more playful",
];

type Reply = Result<Json, AiError>;

/// Canned model replies for tests: requests are recorded instead of sent,
/// and answered in order.
#[derive(Default)]
pub struct Script {
    pub replies: std::collections::VecDeque<Json>,
    pub sent: Vec<Json>,
}

pub struct Assistant {
    agent: Agent,
    input: String,
    inflight: Option<(Purpose, mpsc::Receiver<Reply>)>,
    /// Text of the reply being streamed in right now.
    streaming: std::sync::Arc<std::sync::Mutex<String>>,
    /// The assistant changed the project and the user hasn't accepted/rejected yet.
    pub reviewing: bool,
    gpu: Option<Offscreen>,
    /// The AI settings draft is being edited (Settings ▸ AI).
    pub settings_open: bool,
    draft: AiSettings,
    models: Option<Result<Vec<ModelInfo>, String>>,
    models_rx: Option<mpsc::Receiver<Result<Vec<ModelInfo>, String>>>,
    model_filter: String,
    only_zdr: bool,
    scroll_to_bottom: bool,
    /// When the current request started (for the progress readout).
    started: Option<std::time::Instant>,
    /// Decoded review thumbnails, by transcript index.
    thumbs: std::collections::HashMap<usize, egui::TextureHandle>,
    /// Tests: answer from this instead of calling OpenRouter.
    pub script: Option<std::sync::Arc<std::sync::Mutex<Script>>>,
    /// Session cost when the current request started (its own cost is the difference).
    turn_cost_start: f64,
    /// The project before the current request, to list what it changed.
    review_base: Option<xenify_doc::Project>,
    /// Layers the request changed, cached by editor revision: (id, name, new?).
    changed_cache: Option<(u64, Vec<ChangedLayer>)>,
    /// Whether an API key was found (checked when needed, not every frame).
    has_key: Option<bool>,
    /// Put the cursor in the input box next frame (after picking a suggestion).
    focus_input: bool,
}

impl Default for Assistant {
    fn default() -> Self {
        let settings = AiSettings::load();
        Self {
            draft: settings.clone(),
            agent: Agent::new(settings),
            input: String::new(),
            inflight: None,
            streaming: Default::default(),
            reviewing: false,
            gpu: None,
            settings_open: false,
            models: None,
            models_rx: None,
            model_filter: String::new(),
            only_zdr: true,
            scroll_to_bottom: false,
            started: None,
            thumbs: Default::default(),
            script: None,
            turn_cost_start: 0.0,
            review_base: None,
            changed_cache: None,
            has_key: None,
            focus_input: false,
        }
    }
}

/// A layer the request under review changed: id, name, and whether it's new.
pub type ChangedLayer = (Id, String, bool);

/// Whether the assistant can run: an OpenRouter key is set (or a test script
/// answers). When not, opens Settings › AI and says why.
pub fn ai_ready(app: &mut XenifyApp) -> bool {
    if app.assistant.key_ready() {
        return true;
    }
    app.settings.open(crate::settings::Tab::Ai);
    app.fail("The assistant needs an OpenRouter key: paste one in Settings › AI (it's stored in your system keyring).");
    false
}

/// The layers the request under review changed: (id, name, new?). Cached.
pub fn changed_layers(app: &mut XenifyApp) -> Vec<ChangedLayer> {
    if !app.assistant.reviewing {
        return Vec::new();
    }
    let rev = app.editor.revision();
    if let Some((r, list)) = &app.assistant.changed_cache
        && *r == rev
    {
        return list.clone();
    }
    let Some(base) = &app.assistant.review_base else { return Vec::new() };
    let mut list = Vec::new();
    for comp in &app.project().compositions {
        for layer in &comp.layers {
            match base.layer(layer.id) {
                Ok(old) if old == layer => {}
                Ok(_) => list.push((layer.id, layer.meta.name.clone(), false)),
                Err(_) => list.push((layer.id, layer.meta.name.clone(), true)),
            }
        }
    }
    app.assistant.changed_cache = Some((rev, list.clone()));
    list
}

/// Gives the agent access to the editor.
struct Host<'a> {
    app: &'a mut XenifyApp,
    gpu: &'a mut Option<Offscreen>,
    label: String,
}

impl ToolHost for Host<'_> {
    fn project(&self) -> &Project {
        self.app.editor.project()
    }
    fn comp(&self) -> Id {
        self.app.comp
    }
    fn set_comp(&mut self, comp: Id) -> Result<(), String> {
        self.app.project().comp(comp).map_err(|e| e.to_string())?;
        self.app.open_comp(comp);
        Ok(())
    }
    fn time(&self) -> f64 {
        self.app.time
    }
    fn execute(&mut self, label: &str, cmd: Command) -> Result<(), String> {
        // All changes of one assistant turn form a single undo step.
        if !self.app.editor.in_group() {
            self.app.editor.begin_group(&self.label);
        }
        self.app.editor.execute(label, cmd).map_err(|e| e.to_string())
    }
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect {
        self.app.viewport.measure_text(text)
    }
    fn trace_image(&mut self, image: &[u8], colors: u8) -> Result<Vec<xenify_doc::Layer>, String> {
        let s = &self.app.comp().settings;
        let opts = xenify_export::trace::TraceOptions { colors, ..Default::default() };
        xenify_export::trace::trace_to_layers(image, &opts, self.app.project().next_id, s.width, s.height)
    }
    fn animated_emoji(&mut self, key: &str) -> Result<(xenify_export::animated_emoji::AnimatedEmoji, Vec<u8>), String> {
        // Blocks while downloading (once; then it's cached).
        use xenify_export::animated_emoji as ae;
        let list = ae::index()?;
        let e = ae::find(&list, key).ok_or_else(|| format!("no animated emoji matches {key:?}"))?.clone();
        let data = ae::lottie(&e)?;
        Ok((e, data))
    }
    fn giphy_gif(&mut self, query: &str) -> Result<(xenify_export::giphy::Gif, Vec<u8>), String> {
        // Blocks while searching and downloading.
        let panel = &mut self.app.giphy_panel;
        let key = panel
            .key
            .clone()
            .filter(|k| !k.is_empty())
            .or_else(xenify_export::giphy::saved_key)
            .ok_or("there's no GIPHY key yet: ask the user to add theirs in the GIFs panel (Layer › New › GIF)")?;
        let backend = panel.backend;
        let g =
            (backend.search)(&key, query, 1, 0)?.into_iter().next().ok_or_else(|| format!("no GIFs for {query:?}"))?;
        let data = (backend.get)(&g.full)?;
        Ok((g, data))
    }
    fn import_svg(&mut self, svg: &[u8]) -> Result<Vec<xenify_doc::Layer>, String> {
        let s = &self.app.comp().settings;
        xenify_export::svg::import_svg(svg, self.app.project().next_id, s.width, s.height)
    }
    fn render_png(&mut self, time: f64) -> Result<Vec<u8>, String> {
        if self.gpu.is_none() {
            *self.gpu = Some(Offscreen::new().map_err(|e| e.to_string())?);
        }
        let (project, comp) = (self.app.project().clone(), self.app.comp);
        self.gpu.as_mut().unwrap().preview_png(&project, comp, time, 768).map_err(|e| e.to_string())
    }
}

impl Assistant {
    pub fn image_model(&self) -> &str {
        &self.agent.settings.image_model
    }

    /// Starts generating an image of `subject` in the background.
    /// Paints `subject` in the background. `for_tracing` steers the model to
    /// flat artwork on white, which traces into clean shapes.
    pub fn start_image(
        &self,
        subject: &str,
        for_tracing: bool,
        ctx: &egui::Context,
    ) -> Result<mpsc::Receiver<Result<Vec<u8>, String>>, String> {
        let transport = OpenRouter::new(&self.agent.settings).map_err(|e| e.to_string())?;
        let prompt = if for_tracing { xenify_ai::images::vector_friendly_prompt(subject) } else { subject.to_string() };
        let body = xenify_ai::images::image_request(&self.agent.settings.image_model, &prompt);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = transport
                .chat(&body)
                .map_err(|e| e.to_string())
                .and_then(|resp| xenify_ai::images::image_from_response(&resp).map_err(|e| e.to_string()));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        Ok(rx)
    }

    pub fn busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// An API key is set (checked once, again after the settings change).
    pub fn key_ready(&mut self) -> bool {
        if self.script.is_some() {
            return true;
        }
        *self.has_key.get_or_insert_with(|| AiSettings::default().resolve_api_key().is_some())
    }

    /// Tests: behave as if there were no key.
    pub(crate) fn pretend_no_key(&mut self) {
        self.has_key = Some(false);
    }

    /// Forget the cached key check (after the key was saved or removed).
    pub fn recheck_key(&mut self) {
        self.has_key = None;
    }

    /// What the request so far has cost.
    fn turn_cost(&self) -> f64 {
        (self.agent.usage.cost - self.turn_cost_start).max(0.0)
    }

    /// Runs `f` with the agent and a tool host, then acts on the agent's `Next`.
    fn drive(app: &mut XenifyApp, ctx: &egui::Context, f: impl FnOnce(&mut Agent, &mut dyn ToolHost) -> Next) {
        let mut me = std::mem::take(&mut app.assistant);
        let label = me
            .agent
            .transcript
            .iter()
            .rev()
            .find_map(|i| if let ChatItem::User(t) = i { Some(t.clone()) } else { None })
            .map_or("AI changes".into(), |t| format!("AI: {}", truncate(&t, 40)));
        let next = {
            let mut host = Host { app, gpu: &mut me.gpu, label };
            f(&mut me.agent, &mut host)
        };
        me.dispatch(next, app, ctx);
        me.scroll_to_bottom = true;
        app.assistant = me;
    }

    fn dispatch(&mut self, next: Next, app: &mut XenifyApp, ctx: &egui::Context) {
        if let (Next::Request { body, purpose }, Some(script)) = (&next, &self.script) {
            let mut script = script.lock().unwrap();
            script.sent.push(body.clone());
            let reply = script.replies.pop_front().ok_or_else(|| AiError::Network("no scripted reply left".into()));
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(reply);
            self.inflight = Some((purpose.clone(), rx));
            ctx.request_repaint();
            return;
        }
        match next {
            Next::Request { body, purpose } => match OpenRouter::new(&self.agent.settings) {
                Ok(transport) => {
                    let (tx, rx) = mpsc::channel();
                    let ctx = ctx.clone();
                    // The planner's replies stream in so the user sees it think;
                    // frame reviews and images arrive whole.
                    let stream = matches!(purpose, Purpose::Brain).then(|| self.streaming.clone());
                    self.streaming.lock().unwrap().clear();
                    std::thread::Builder::new()
                        .name("xenify-ai".into())
                        .spawn(move || {
                            let reply = match stream {
                                Some(buf) => transport.chat_stream(&body, &mut |t| {
                                    buf.lock().unwrap().push_str(t);
                                    ctx.request_repaint();
                                }),
                                None => transport.chat(&body),
                            };
                            let _ = tx.send(reply);
                            ctx.request_repaint();
                        })
                        .expect("spawn AI thread");
                    self.inflight = Some((purpose, rx));
                }
                Err(e) => {
                    self.agent.stop();
                    self.agent.transcript.push(ChatItem::Error(e.to_string()));
                    self.finish_turn(app);
                }
            },
            Next::WaitForUser => {}
            Next::Idle => self.finish_turn(app),
        }
    }

    fn finish_turn(&mut self, app: &mut XenifyApp) {
        let changed = self.agent.take_changed();
        if app.editor.in_group() {
            if changed {
                self.reviewing = true;
                app.status = "The assistant made changes — Accept or Reject them in the Assistant panel.".into();
            } else {
                app.editor.end_group();
            }
        }
        app.fix_selection();
    }

    /// Checks for replies from OpenRouter (call every frame).
    pub fn poll(app: &mut XenifyApp, ctx: &egui::Context) {
        let Some((purpose, rx)) = &app.assistant.inflight else { return };
        let reply = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(AiError::Network("request thread crashed".into())),
        };
        let purpose = purpose.clone();
        app.assistant.inflight = None;
        app.assistant.streaming.lock().unwrap().clear();
        Self::drive(app, ctx, |agent, host| agent.handle_response(purpose, reply, host));
    }

    fn send(app: &mut XenifyApp, ctx: &egui::Context, text: String) {
        if text.trim().is_empty() || app.assistant.busy() || !ai_ready(app) {
            return;
        }
        // Starting a new request implicitly accepts the previous changes.
        if app.assistant.reviewing && !app.assistant.agent.is_waiting_for_user() {
            Self::accept(app);
        }
        if !app.assistant.reviewing {
            // What this request changes is measured from here.
            app.assistant.review_base = Some(app.project().clone());
            app.assistant.changed_cache = None;
            app.assistant.turn_cost_start = app.assistant.agent.usage.cost;
        }
        app.assistant.input.clear();
        if !app.assistant.agent.is_waiting_for_user() {
            app.assistant.started = Some(std::time::Instant::now());
        }
        Self::drive(app, ctx, |agent, host| agent.send(text.trim(), host));
    }

    /// Sends `text` on the user's behalf (AI menu, New dialog), showing the
    /// assistant panel. `fresh` starts a new chat first. Returns false, with
    /// a status message, when the assistant is still busy. `steps` raises the
    /// step limit for this request (big builds).
    pub fn request(app: &mut XenifyApp, ctx: &egui::Context, text: String, fresh: bool, steps: Option<usize>) -> bool {
        if app.assistant.busy() {
            app.status = "The assistant is still working — wait for it or press Stop first.".into();
            return false;
        }
        if !ai_ready(app) {
            return false;
        }
        crate::dock::reveal(app, crate::dock::Panel::Assistant);
        if fresh {
            if app.assistant.reviewing {
                Self::accept(app);
            }
            app.assistant.agent.reset();
            app.assistant.thumbs.clear();
        }
        if let Some(n) = steps {
            app.assistant.agent.allow_steps_next_turn(n);
        }
        Self::send(app, ctx, text);
        true
    }

    pub fn accept(app: &mut XenifyApp) {
        if app.editor.in_group() {
            app.editor.end_group();
        }
        app.assistant.reviewing = false;
        app.status = "Accepted the assistant's changes (Ctrl+Z still undoes them).".into();
    }

    fn reject(app: &mut XenifyApp) {
        app.editor.cancel_group();
        app.assistant.reviewing = false;
        app.assistant.agent.note_for_next_turn("The user rejected and undid your previous changes.");
        app.assistant.agent.transcript.push(ChatItem::Error("Changes rejected and undone.".into()));
        app.fix_selection();
        app.status = "Rejected the assistant's changes.".into();
    }

    fn stop(app: &mut XenifyApp) {
        app.assistant.inflight = None;
        app.assistant.agent.stop();
        let mut me = std::mem::take(&mut app.assistant);
        me.finish_turn(app);
        app.assistant = me;
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    header(app, ui);
    ui.separator();

    if app.assistant.reviewing {
        let changed = changed_layers(app);
        let cost = app.assistant.turn_cost();
        egui::Frame::group(ui.style()).fill(ACCENT.gamma_multiply(0.15)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Review the assistant's changes").strong());
                if ui.button("✔ Accept").clicked() {
                    Assistant::accept(app);
                }
                if ui.button("Reject").on_hover_text("Undo everything from this request").clicked() {
                    Assistant::reject(app);
                }
            });
            // What changed (marked "AI" in Layers too) and what it cost.
            let names =
                |new: bool| -> Vec<String> { changed.iter().filter(|c| c.2 == new).map(|c| c.1.clone()).collect() };
            let list = |v: Vec<String>| {
                let more = v.len().saturating_sub(6);
                let mut s = v.into_iter().take(6).collect::<Vec<_>>().join(", ");
                if more > 0 {
                    s += &format!(" and {more} more");
                }
                s
            };
            let (edited, added) = (names(false), names(true));
            if !added.is_empty() {
                ui.weak(format!("New: {}", list(added)));
            }
            if !edited.is_empty() {
                ui.weak(format!("Changed: {}", list(edited)));
            }
            ui.weak(format!("This request cost ${cost:.4}"));
        });
    }

    // Input at the bottom; transcript fills the rest.
    let input_h = 86.0;
    let transcript_h = (ui.available_height() - input_h).max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("chat")
        .max_height(transcript_h)
        .min_scrolled_height(transcript_h)
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            transcript(app, ui, &ctx);
            // The reply as it streams in (replaced by the final message when done).
            let partial = app.assistant.streaming.lock().unwrap().clone();
            if app.assistant.busy() && !partial.trim().is_empty() {
                ui.add_space(4.0);
                ui.add(egui::Label::new(strip_markdown(partial.trim())).wrap());
            }
        });

    ui.separator();
    let busy = app.assistant.busy();
    let waiting = app.assistant.agent.is_waiting_for_user();
    let hint = if waiting { "Type your answer…" } else { "Describe what to make or change… (Enter to send)" };
    let r = ui.add_enabled(
        !busy,
        egui::TextEdit::multiline(&mut app.assistant.input)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    );
    if std::mem::take(&mut app.assistant.focus_input) {
        r.request_focus();
    }
    let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    ui.horizontal(|ui| {
        if busy {
            ui.spinner();
            let what = match &app.assistant.inflight {
                Some((Purpose::Review { time, .. }, _)) => format!("Looking at the frame at {time:.1}s…"),
                _ if !app.assistant.streaming.lock().unwrap().trim().is_empty() => "Writing…".into(),
                _ => "Thinking…".into(),
            };
            let secs = app.assistant.started.map_or(0, |t| t.elapsed().as_secs());
            let cost = app.assistant.turn_cost();
            ui.weak(format!("{what}  {secs}s · step {} · ${cost:.4}", app.assistant.agent.steps() + 1));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            if ui
                .button("Stop")
                .on_hover_text("Stop now and keep what's done so far — then Accept or Reject it")
                .clicked()
            {
                Assistant::stop(app);
            }
        } else if (ui.button("Send").clicked() || enter) && !app.assistant.input.trim().is_empty() {
            let text = app.assistant.input.trim_end_matches('\n').to_string();
            Assistant::send(app, &ctx, text);
        }
    });
}

fn header(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let s = &app.assistant.agent.settings;
        ui.label(RichText::new(short_model(&s.brain_model)).strong()).on_hover_text(format!(
            "Brain: {}\nReviewer: {}",
            s.brain_model,
            if s.reviewer_model.is_empty() { "(none)" } else { &s.reviewer_model }
        ));
        let u = app.assistant.agent.usage;
        ui.weak(format!("${:.4}", u.cost)).on_hover_text(format!(
            "Spent this session: ${:.4} ({} input + {} output tokens). Each request's cost shows while it runs \
             and when you review it",
            u.cost, u.prompt_tokens, u.completion_tokens
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("⚙").on_hover_text("AI settings").clicked() {
                app.settings.open(crate::settings::Tab::Ai);
            }
            if ui.add_enabled(!app.assistant.busy(), egui::Button::new("New chat")).clicked() {
                if app.assistant.reviewing {
                    Assistant::accept(app);
                }
                app.assistant.agent.reset();
                app.assistant.thumbs.clear();
            }
        });
    });
}

fn transcript(app: &mut XenifyApp, ui: &mut egui::Ui, ctx: &egui::Context) {
    if app.assistant.agent.transcript.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("What should we make?").heading());
        ui.weak("I can build animations from a description, change what's on the canvas, and check my work by looking at frames. Every change can be accepted or undone.");
        ui.add_space(8.0);
        if !app.assistant.key_ready() {
            ui.colored_label(RECORD, "No OpenRouter key yet: the assistant can't run without one.");
            if ui.button("Add a key…").on_hover_text("Settings › AI (stored in your system keyring)").clicked() {
                app.settings.open(crate::settings::Tab::Ai);
            }
            ui.add_space(8.0);
        }
        ui.weak("Try one (it goes in the box below to edit first):");
        let mut chosen = None;
        for s in SUGGESTIONS {
            if ui.add(egui::Button::new(RichText::new(*s).small()).wrap()).clicked() {
                chosen = Some(s.to_string());
            }
        }
        if let Some(s) = chosen {
            app.assistant.input = s;
            app.assistant.focus_input = true;
        }
        let _ = ctx;
        return;
    }
    let mut answer = None;
    let mut jump_to = None;
    let busy = app.assistant.busy();
    let items = app.assistant.agent.transcript.clone();
    for (index, item) in items.iter().enumerate() {
        match item {
            ChatItem::User(t) => {
                ui.add_space(6.0);
                egui::Frame::new().fill(ACCENT.gamma_multiply(0.25)).corner_radius(6.0).inner_margin(6.0).show(
                    ui,
                    |ui| {
                        ui.add(egui::Label::new(t).wrap());
                    },
                );
            }
            ChatItem::Assistant(t) => {
                ui.add_space(4.0);
                ui.add(egui::Label::new(strip_markdown(t)).wrap());
            }
            ChatItem::Activity { text, ok } => {
                let color = if *ok { Color32::from_rgb(110, 190, 120) } else { RECORD };
                ui.horizontal_wrapped(|ui| {
                    if text.starts_with("Measured") {
                        tag(ui, "measured", Color32::from_rgb(90, 170, 255))
                            .on_hover_text("Exact numbers from the document — reliable.");
                    } else {
                        ui.label(RichText::new("•").color(color));
                    }
                    ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
                });
            }
            ChatItem::Review { time, png, text } => {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        tag(ui, "reviewer", Color32::from_rgb(230, 170, 60)).on_hover_text(
                            "A vision model's impression of this frame. Good for look & feel; \
                             it can misjudge exact positions — measurements win.",
                        );
                        ui.weak(format!("looked at {time:.2}s"));
                    });
                    if let Some(tex) = thumb(&mut app.assistant.thumbs, ui.ctx(), index, png) {
                        let w = ui.available_width().min(260.0);
                        let size = tex.size_vec2() * (w / tex.size_vec2().x);
                        let r = ui
                            .add(egui::Image::new((tex.id(), size)).sense(egui::Sense::click()))
                            .on_hover_text("This is exactly what the reviewer saw. Click to jump to this time.");
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("What the reviewer saw at {time:.2}s: jump there"),
                            )
                        });
                        if r.clicked() {
                            jump_to = Some(*time);
                        }
                    }
                    ui.add(egui::Label::new(RichText::new(strip_markdown(text)).weak()).wrap());
                });
            }
            ChatItem::Question { text, options, answered } => {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).strong()).wrap());
                    if !answered {
                        for o in options {
                            if ui.add_enabled(!busy, egui::Button::new(o.as_str()).wrap()).clicked() {
                                answer = Some(o.clone());
                            }
                        }
                        if !options.is_empty() {
                            ui.weak("…or type your own answer below.");
                        }
                    }
                });
            }
            ChatItem::Generated { image, text } => {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    tag(ui, "generated", Color32::from_rgb(170, 120, 255)).on_hover_text(
                        "The picture the image model drew — it was traced into vector shapes on the canvas.",
                    );
                    if let Some(tex) = thumb(&mut app.assistant.thumbs, ui.ctx(), index, image) {
                        let w = ui.available_width().min(260.0);
                        let size = tex.size_vec2() * (w / tex.size_vec2().x);
                        ui.add(egui::Image::new((tex.id(), size)));
                    }
                    ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
                });
            }
            ChatItem::Error(e) => {
                ui.add(egui::Label::new(RichText::new(e).color(RECORD)).wrap());
            }
        }
    }
    if let Some(a) = answer {
        Assistant::send(app, ctx, a);
    }
    if let Some(t) = jump_to {
        app.playing = false;
        app.set_time(t);
    }
}

/// A small coloured label.
fn tag(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.25))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)))
        .response
}

/// Decodes (once) and returns the thumbnail texture for a review.
fn thumb(
    cache: &mut std::collections::HashMap<usize, egui::TextureHandle>,
    ctx: &egui::Context,
    index: usize,
    png_bytes: &[u8],
) -> Option<egui::TextureHandle> {
    if let Some(t) = cache.get(&index) {
        return Some(t.clone());
    }
    // Any common format (PNG, JPEG, WebP), shrunk for the chat.
    let img = image::load_from_memory(png_bytes).ok()?.thumbnail(520, 520).to_rgba8();
    let (w, h) = img.dimensions();
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
    let tex = ctx.load_texture(format!("review-{index}"), image, egui::TextureOptions::LINEAR);
    cache.insert(index, tex.clone());
    Some(tex)
}

/// Settings ▸ AI: the OpenRouter key and models (edits a draft; Save applies it).
pub fn ai_settings_ui(app: &mut XenifyApp, ui: &mut egui::Ui) {
    if !app.assistant.settings_open {
        // Opened from the Settings window: start from the live settings.
        app.assistant.draft = app.assistant.agent.settings.clone();
        app.assistant.settings_open = true;
    }
    // Collect the model list from the background fetch.
    if let Some(rx) = &app.assistant.models_rx
        && let Ok(r) = rx.try_recv()
    {
        app.assistant.models = Some(r);
        app.assistant.models_rx = None;
    }
    let mut save = false;
    {
        let a = &mut app.assistant;
        ui.horizontal(|ui| {
            ui.label("OpenRouter key:");
            ui.label(RichText::new(a.draft.api_key_source()).strong());
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut a.draft.api_key)
                    .password(true)
                    .hint_text("paste your OpenRouter API key (openrouter.ai/keys)")
                    .desired_width(360.0),
            );
            if AiSettings::has_stored_api_key()
                && ui.button("Forget key").on_hover_text("Remove your saved key from this computer").clicked()
            {
                a.recheck_key();
                app.status = match AiSettings::forget_api_key() {
                    Ok(()) => "Removed the saved OpenRouter key.".into(),
                    Err(e) => format!("Couldn't remove the key: {e}"),
                };
            }
        });
        ui.weak(
            "Your key is saved in the system keyring (never in project or settings files) and only sent to OpenRouter.",
        );
        ui.separator();
        model_picker(ui, "Brain (plans & edits — needs tool calling)", &mut a.draft.brain_model, BRAIN_PRESETS, false);
        model_picker(
            ui,
            "Reviewer (looks at frames — needs vision)",
            &mut a.draft.reviewer_model,
            REVIEWER_PRESETS,
            true,
        );
        model_picker(
            ui,
            "Illustrations (image output, traced to vectors)",
            &mut a.draft.image_model,
            xenify_ai::images::IMAGE_PRESETS,
            false,
        );
        ui.horizontal(|ui| {
            ui.label("Creativity");
            ui.add(egui::Slider::new(&mut a.draft.temperature, 0.0..=1.2).fixed_decimals(1));
            ui.label("Max steps");
            ui.add(egui::DragValue::new(&mut a.draft.max_steps).range(5..=200));
            ui.label("Frame checks");
            ui.add(egui::DragValue::new(&mut a.draft.max_reviews).range(0..=10)).on_hover_text(
                "How many frames the reviewer may look at per request. Each check adds a round trip (time), \
                 so fewer = faster.",
            );
        });
        ui.checkbox(&mut a.draft.propose_concepts, "Offer 2–3 concepts before building something new")
            .on_hover_text("For briefs like \"make an intro for my podcast\", the assistant first suggests a few directions to pick from. Edits are always done directly.");
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Browse OpenRouter models");
            if a.models_rx.is_some() {
                ui.spinner();
            } else if ui.button(if a.models.is_some() { "Refresh" } else { "Load list" }).clicked() {
                let (tx, rx) = mpsc::channel();
                let base = a.draft.base_url.clone();
                let ctx2 = ui.ctx().clone();
                std::thread::spawn(move || {
                    let _ = tx.send(xenify_ai::fetch_models(&base).map_err(|e| e.to_string()));
                    ctx2.request_repaint();
                });
                a.models_rx = Some(rx);
            }
        });
        match &a.models {
            None => {
                ui.weak("Load the live list to search every model, with prices per million tokens.");
            }
            Some(Err(e)) => {
                ui.colored_label(RECORD, e);
            }
            Some(Ok(models)) => {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut a.model_filter).hint_text("search, e.g. qwen").desired_width(200.0));
                    ui.checkbox(&mut a.only_zdr, "ZDR only").on_hover_text(
                        "Zero Data Retention: providers that don't store your prompts. Needed if your OpenRouter account has the ZDR guardrail on.",
                    );
                });
                let filter = a.model_filter.to_lowercase();
                let rows: Vec<&ModelInfo> = models
                    .iter()
                    .filter(|m| m.tools || m.vision)
                    .filter(|m| !a.only_zdr || m.zdr)
                    .filter(|m| {
                        filter.is_empty()
                            || m.id.to_lowercase().contains(&filter)
                            || m.name.to_lowercase().contains(&filter)
                    })
                    .collect();
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    egui::Grid::new("models").striped(true).num_columns(5).show(ui, |ui| {
                        ui.weak("model");
                        ui.weak("$ in / out");
                        ui.weak("features");
                        ui.weak("");
                        ui.weak("");
                        ui.end_row();
                        for m in rows.iter().take(300) {
                            ui.label(&m.id).on_hover_text(format!("{} · {}k context", m.name, m.context / 1000));
                            ui.label(format!("{:.2} / {:.2}", m.prompt_price, m.completion_price));
                            let mut f = Vec::new();
                            if m.tools {
                                f.push("tools");
                            }
                            if m.vision {
                                f.push("vision");
                            }
                            if m.zdr {
                                f.push("ZDR");
                            }
                            ui.weak(f.join(" · "));
                            if ui.add_enabled(m.tools, egui::Button::new("Brain").small()).clicked() {
                                a.draft.brain_model = m.id.clone();
                            }
                            if ui.add_enabled(m.vision, egui::Button::new("Reviewer").small()).clicked() {
                                a.draft.reviewer_model = m.id.clone();
                            }
                            ui.end_row();
                        }
                    });
                });
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                save = true;
            }
            if ui.button("Reset to defaults").clicked() {
                let key = std::mem::take(&mut a.draft.api_key);
                a.draft = AiSettings { api_key: key, ..Default::default() };
            }
        });
    }
    if save {
        save_ai_settings(app);
    }
}

/// Whether Settings › AI has changes (or a typed key) not saved yet.
pub fn ai_settings_unsaved(app: &XenifyApp) -> bool {
    app.assistant.draft != app.assistant.agent.settings
}

/// Forgets unsaved Settings › AI changes (and a typed, unsaved key).
pub fn discard_ai_settings(app: &mut XenifyApp) {
    app.assistant.draft = app.assistant.agent.settings.clone();
}

/// Saves Settings › AI: the key to the keyring, the rest to the settings file.
pub fn save_ai_settings(app: &mut XenifyApp) {
    {
        let a = &mut app.assistant;
        // A newly typed key goes to the keyring, never into the settings file.
        let typed = std::mem::take(&mut a.draft.api_key);
        let key_note = if typed.trim().is_empty() {
            String::new()
        } else {
            a.recheck_key();
            match AiSettings::store_api_key(&typed) {
                Ok(loc) => format!(" Key saved in the {}.", loc.describe()),
                Err(e) => format!(" Couldn't save the key: {e}."),
            }
        };
        a.agent.settings = a.draft.clone();
        match a.draft.save() {
            Ok(()) => app.status = format!("AI settings saved.{key_note}"),
            Err(e) => app.fail(format!("Couldn't save AI settings: {e}")),
        }
    }
}

fn model_picker(ui: &mut egui::Ui, label: &str, value: &mut String, presets: &[ModelPreset], allow_none: bool) {
    ui.label(label);
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(label).selected_text("Presets").width(90.0).show_ui(ui, |ui| {
            for p in presets {
                if ui.selectable_label(*value == p.id, format!("{}  —  {}", p.id, p.note)).clicked() {
                    *value = p.id.to_string();
                }
            }
            if allow_none && ui.selectable_label(value.is_empty(), "None (don't look at frames)").clicked() {
                value.clear();
            }
        });
        ui.add(egui::TextEdit::singleline(value).desired_width(360.0).hint_text("any OpenRouter model id"));
    });
}

fn short_model(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// egui has no markdown renderer; drop the most common markers.
fn strip_markdown(s: &str) -> String {
    s.replace("**", "")
        .replace("__", "")
        .lines()
        .map(|l| l.trim_start_matches("### ").trim_start_matches("## ").trim_start_matches("# "))
        .collect::<Vec<_>>()
        .join("\n")
}
