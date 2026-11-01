//! The AI assistant: the same agent and tools as the egui app. The agent
//! never does network I/O: it hands back request bodies, the screens send
//! them to OpenRouter with the person's own key and pass the replies back.
//! All the changes of one turn are one undo step, to Accept or Reject.

use serde_json::{Value as Json, json};
use xenify_ai::{Agent, AiError, AiSettings, ChatItem, Next, Purpose, ToolHost};
use xenify_anim::SceneText;
use xenify_doc::{Id, Project};
use xenify_ops::Command;

use crate::{Session, err};

/// The agent, the requests out at the screens (by number), and whether a turn's changes wait for Accept/Reject.
#[derive(Default)]
pub struct Assistant {
    agent: Option<Agent>,
    pending: Vec<(u64, Purpose)>,
    next: u64,
    pub reviewing: bool,
    /// The canvas before this turn's changes (to show which layers they touched).
    before: Option<Project>,
}

/// Draws a scene at a size as PNG bytes (the screens' renderer), for the reviewer model.
pub type PngRenderer<'a> = &'a mut dyn FnMut(&xenify_anim::Scene, u32, u32) -> Result<Vec<u8>, String>;

struct Host<'a, 'b> {
    s: &'a mut Session,
    png: Option<PngRenderer<'b>>,
}

impl Host<'_, '_> {
    /// The turn's changes are one undo step, to Accept or Reject.
    fn open_group(&mut self) {
        if !self.s.editor.in_group() {
            self.s.assistant.before = Some(self.s.editor.project().clone());
            self.s.editor.begin_group("Assistant");
        }
    }
}

impl ToolHost for Host<'_, '_> {
    fn project(&self) -> &Project {
        self.s.editor.project()
    }
    fn comp(&self) -> Id {
        self.s.comp
    }
    fn set_comp(&mut self, comp: Id) -> Result<(), String> {
        self.s.open_comp(comp)
    }
    fn time(&self) -> f64 {
        self.s.time
    }
    fn execute(&mut self, label: &str, cmd: Command) -> Result<(), String> {
        self.open_group();
        self.s.editor.execute(label, cmd).map_err(err)
    }
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect {
        self.s.text.layout(text).bounds
    }
    fn render_png(&mut self, time: f64) -> Result<Vec<u8>, String> {
        let png = self.png.as_mut().ok_or("looking at frames isn't available here")?;
        let scene = xenify_anim::evaluate_project(self.s.editor.project(), self.s.comp, time);
        // A size the reviewer reads well: the longer side about 1024 pixels.
        let k = 1024.0 / scene.width.max(scene.height).max(1) as f64;
        let (w, h) =
            (((scene.width as f64 * k).round() as u32).max(1), ((scene.height as f64 * k).round() as u32).max(1));
        png(&scene, w, h)
    }
    fn import_svg(&mut self, svg: &[u8]) -> Result<Vec<xenify_doc::Layer>, String> {
        let (w, h) = self.s.comp_settings().map_or((1080, 1080), |c| (c.width, c.height));
        xenify_export::svg::import_svg(svg, self.s.project().next_id, w, h)
    }
    fn trace_image(&mut self, image: &[u8], colors: u8) -> Result<Vec<xenify_doc::Layer>, String> {
        let (w, h) = self.s.comp_settings().map_or((1080, 1080), |c| (c.width, c.height));
        let opts = xenify_export::trace::TraceOptions { colors, ..Default::default() };
        xenify_export::trace::trace_to_layers(image, &opts, self.s.project().next_id, w, h)
    }
    fn paints_pixels(&self) -> bool {
        true
    }
    fn raster(&mut self, call: &str, args: &Json) -> Result<Json, String> {
        match call {
            // What the canvas shows, on the layer's pixel grid (fill on all layers), drawn by the screens' renderer.
            "raster_sample" => {
                let id = crate::id_arg(args, "id")?;
                let (mut scene, m, w, h) = self.s.raster_sample_view(id).ok_or("not a raster layer")?;
                let png = self.png.as_mut().ok_or("looking at the canvas isn't available here")?;
                scene.camera = m * scene.camera;
                scene.width = w;
                scene.height = h;
                let img = image::load_from_memory(&png(&scene, w, h)?).map_err(|e| e.to_string())?.into_rgba8();
                if img.dimensions() != (w, h) {
                    return Err("this layer is too big to look at all at once".into());
                }
                self.s.raster_set_sample(id, img.into_raw());
                Ok(Json::Null)
            }
            // A layer on its own as a PNG (base64), e.g. a reference picture for change_picture.
            "layer_png" => {
                let id = crate::id_arg(args, "id")?;
                let scene = self.s.layer_preview(id, 768).ok_or("that layer shows nothing")?;
                let png = self.png.as_mut().ok_or("drawing layers isn't available here")?;
                Ok(json!(b64(&png(&scene, 768, 768)?)))
            }
            _ => {
                self.open_group();
                self.s.raster_call(call, args).unwrap_or_else(|| Err(format!("no pixel call {call}")))
            }
        }
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn chat_json(item: &ChatItem) -> Json {
    match item {
        ChatItem::User(t) => json!({ "kind": "user", "text": t }),
        ChatItem::Assistant(t) => json!({ "kind": "assistant", "text": t }),
        ChatItem::Activity { text, ok } => json!({ "kind": "activity", "text": text, "ok": ok }),
        ChatItem::Question { text, options, answered } => {
            json!({ "kind": "question", "text": text, "options": options, "answered": answered })
        }
        ChatItem::Review { time, text, png } => {
            json!({ "kind": "review", "time": time, "text": text, "image": b64(png) })
        }
        ChatItem::Generated { text, image } => json!({ "kind": "generated", "text": text, "image": b64(image) }),
        ChatItem::Error(t) => json!({ "kind": "error", "text": t }),
    }
}

impl Session {
    pub(crate) fn ai_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        self.ai_call_with(name, args, None)
    }

    /// The assistant's calls, with a renderer for frame reviews when the screens have one.
    pub fn ai_call_with(
        &mut self,
        name: &str,
        args: &Json,
        png: Option<PngRenderer<'_>>,
    ) -> Option<Result<Json, String>> {
        Some(match name {
            "ai_state" => Ok(self.ai_state(Vec::new())),
            // AI › New illustration: the request for the image model, and the picture from its reply.
            "image_request" => {
                let subject = args.get("prompt").and_then(Json::as_str).unwrap_or("").trim();
                let prompt = if args.get("for_tracing").and_then(Json::as_bool).unwrap_or(true) {
                    xenify_ai::images::vector_friendly_prompt(subject)
                } else {
                    subject.to_string()
                };
                let model = args
                    .get("model")
                    .and_then(Json::as_str)
                    .filter(|m| !m.is_empty())
                    .map_or_else(|| AiSettings::default().image_model, str::to_string);
                Ok(
                    json!({ "body": xenify_ai::images::image_request(&model, &prompt), "model": model, "base_url": AiSettings::default().base_url }),
                )
            }
            // AI › Change this picture: the request for an image model that edits pictures.
            "image_edit_request" => (|| {
                let model = args
                    .get("model")
                    .and_then(Json::as_str)
                    .filter(|m| !m.is_empty())
                    .map_or_else(|| AiSettings::default().image_model, str::to_string);
                let url = |v: &Json| {
                    v.as_str().map(|b| {
                        if b.starts_with("data:") { b.to_string() } else { format!("data:image/png;base64,{b}") }
                    })
                };
                let image = url(&args["image"]).ok_or("missing picture")?;
                let mask = url(&args["mask"]);
                let refs: Vec<(String, String)> = args["references"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|r| Some((url(&r["image"])?, r["note"].as_str().unwrap_or("").to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                let change = args["change"].as_str().unwrap_or("").trim();
                Ok(
                    json!({ "body": xenify_ai::images::edit_request(&model, change, &image, mask.as_deref(), &refs), "model": model, "base_url": AiSettings::default().base_url }),
                )
            })(),
            "image_mask_request" => (|| {
                let model = args
                    .get("model")
                    .and_then(Json::as_str)
                    .filter(|m| !m.is_empty())
                    .map_or_else(|| AiSettings::default().image_model, str::to_string);
                let image = args["image"]
                    .as_str()
                    .map(|b| if b.starts_with("data:") { b.to_string() } else { format!("data:image/png;base64,{b}") })
                    .ok_or("missing picture")?;
                let what = args["what"].as_str().unwrap_or("").trim();
                if what.is_empty() {
                    return Err("Say what to select.".to_string());
                }
                Ok(
                    json!({ "body": xenify_ai::images::mask_request(&model, what, &image), "model": model, "base_url": AiSettings::default().base_url }),
                )
            })(),
            "image_reply_text" => Ok(json!(xenify_ai::images::reply_text(args.get("response").unwrap_or(&Json::Null)))),
            "image_from_response" => {
                xenify_ai::images::image_from_response(args.get("response").unwrap_or(&Json::Null))
                    .map(|b| json!(b64(&b)))
                    .map_err(|e| e.to_string())
            }
            "ai_send" => self.ai_send(args, png),
            "ai_reply" => self.ai_reply(args, png),
            "ai_stop" => {
                self.assistant.pending.clear();
                if let Some(a) = self.assistant.agent.as_mut() {
                    a.stop();
                }
                self.ai_finish();
                Ok(self.ai_state(Vec::new()))
            }
            "ai_accept" => {
                if self.editor.in_group() {
                    self.editor.end_group();
                }
                self.assistant.reviewing = false;
                Ok(self.ai_state(Vec::new()))
            }
            "ai_reject" => {
                self.editor.cancel_group();
                self.assistant.reviewing = false;
                if let Some(a) = self.assistant.agent.as_mut() {
                    a.note_for_next_turn("The user rejected and undid your previous changes.");
                    a.transcript.push(ChatItem::Error("Changes rejected and undone.".into()));
                }
                self.selection.retain(|id| self.editor.project().layer(*id).is_ok());
                Ok(self.ai_state(Vec::new()))
            }
            "ai_clear" => {
                self.assistant = Assistant::default();
                Ok(self.ai_state(Vec::new()))
            }
            "ai_models" => Ok(json!({
                "brain": xenify_ai::BRAIN_PRESETS.iter().map(|m| json!({ "id": m.id, "note": m.note })).collect::<Vec<_>>(),
                "reviewer": xenify_ai::REVIEWER_PRESETS.iter().map(|m| json!({ "id": m.id, "note": m.note })).collect::<Vec<_>>(),
                "image": xenify_ai::images::IMAGE_PRESETS.iter().map(|m| json!({ "id": m.id, "note": m.note })).collect::<Vec<_>>(),
                "default": AiSettings::default().brain_model,
                "default_reviewer": AiSettings::default().reviewer_model,
                "default_image": AiSettings::default().image_model,
                "defaults": {
                    "temperature": AiSettings::default().temperature, "max_steps": AiSettings::default().max_steps,
                    "max_reviews": AiSettings::default().max_reviews, "concepts": AiSettings::default().propose_concepts,
                },
            })),
            _ => return None,
        })
    }

    /// The chat so far, whether it's working, waiting for an answer or for Accept/Reject, and requests for the screens to send.
    fn ai_state(&self, requests: Vec<Json>) -> Json {
        let a = self.assistant.agent.as_ref();
        json!({
            "transcript": a.map(|a| a.transcript.iter().map(chat_json).collect::<Vec<_>>()).unwrap_or_default(),
            "working": !self.assistant.pending.is_empty(),
            "asking": a.is_some_and(|a| a.is_waiting_for_user()),
            "reviewing": self.assistant.reviewing,
            // The layers this turn added or changed (while there's something to keep or undo).
            "changed": self.assistant.before.as_ref().filter(|_| self.assistant.reviewing || !self.assistant.pending.is_empty()).map(|b| {
                self.project().comp(self.comp).map(|c| c.layers.iter().filter(|l| !b.layer(l.id).is_ok_and(|old| old == *l)).map(|l| l.id.0).collect::<Vec<_>>()).unwrap_or_default()
            }).unwrap_or_default(),
            "cost": a.map_or(0.0, |a| a.usage.cost),
            "requests": requests,
            "base_url": a.map_or_else(|| AiSettings::default().base_url, |a| a.settings.base_url.clone()),
        })
    }

    fn ai_next(&mut self, next: Next) -> Vec<Json> {
        match next {
            Next::Request { body, purpose } => {
                let id = self.assistant.next;
                self.assistant.next += 1;
                self.assistant.pending.push((id, purpose));
                vec![json!({ "id": id, "body": body })]
            }
            Next::WaitForUser => Vec::new(),
            Next::Idle => {
                self.ai_finish();
                Vec::new()
            }
        }
    }

    /// The turn ended: changes wait for Accept or Reject; none, the group closes.
    fn ai_finish(&mut self) {
        let changed = self.assistant.agent.as_mut().is_some_and(|a| a.take_changed());
        if self.editor.in_group() {
            if changed {
                self.assistant.reviewing = true;
            } else {
                self.editor.end_group();
            }
        }
        self.selection.retain(|id| self.editor.project().layer(*id).is_ok());
    }

    /// A message from the person (or an answer to the agent's question). `models`: brain model id.
    fn ai_send(&mut self, args: &Json, png: Option<PngRenderer<'_>>) -> Result<Json, String> {
        let mut text = args.get("text").and_then(Json::as_str).unwrap_or("").trim().to_string();
        // The AI menu's tasks: a new animation or deck (in a new tab), a new slide, a change to this slide.
        let task = args.get("task").and_then(Json::as_str).unwrap_or("");
        let mut steps = None;
        if !task.is_empty() && !text.is_empty() {
            let (w, h) = self.comp_settings().map_or((1920, 1080), |s| (s.width, s.height));
            let slide = self.project().slide_index(self.comp);
            text = match task {
                "new_animation" => xenify_ai::requests::new_animation(&text, w, h),
                "new_deck" => {
                    steps = Some(xenify_ai::requests::DECK_STEPS);
                    xenify_ai::requests::new_deck(&text, w, h)
                }
                "new_slide" => {
                    let here =
                        xenify_ops::slides::slides_or_main(self.project()).iter().position(|s| s.comp == self.comp);
                    let (cmd, comp) = xenify_ops::slides::add_slide(self.project(), here).map_err(err)?;
                    self.exec("New slide", cmd)?;
                    self.open_comp(comp)?;
                    xenify_ai::requests::new_slide(&text, self.project().slide_index(self.comp), h)
                }
                "modify_slide" => xenify_ai::requests::modify_slide(&text, slide),
                _ => text,
            };
            // A new document starts the conversation afresh.
            if task.starts_with("new_") && task != "new_slide" {
                self.assistant.agent = None;
            }
        }
        if text.is_empty() {
            return Ok(self.ai_state(Vec::new()));
        }
        if self.assistant.reviewing {
            // Sending again keeps the last turn's changes.
            if self.editor.in_group() {
                self.editor.end_group();
            }
            self.assistant.reviewing = false;
        }
        let mut settings = AiSettings::default();
        if let Some(m) = args.get("model").and_then(Json::as_str).filter(|m| !m.is_empty()) {
            settings.brain_model = m.to_string();
        }
        if let Some(m) = args.get("reviewer").and_then(Json::as_str) {
            settings.reviewer_model = m.to_string();
        }
        if let Some(m) = args.get("image_model").and_then(Json::as_str).filter(|m| !m.is_empty()) {
            settings.image_model = m.to_string();
        }
        if let Some(t) = args.get("temperature").and_then(Json::as_f64) {
            settings.temperature = t.clamp(0.0, 2.0) as f32;
        }
        if let Some(n) = args.get("max_steps").and_then(Json::as_u64) {
            settings.max_steps = n.clamp(1, 500) as usize;
        }
        if let Some(n) = args.get("max_reviews").and_then(Json::as_u64) {
            settings.max_reviews = n.min(50) as usize;
        }
        if let Some(b) = args.get("concepts").and_then(Json::as_bool) {
            settings.propose_concepts = b;
        }
        // Frame reviews need a renderer (the screens lend theirs).
        if png.is_none() {
            settings.reviewer_model.clear();
        }
        let mut agent = self.assistant.agent.take().unwrap_or_else(|| Agent::new(settings.clone()));
        agent.settings.brain_model = settings.brain_model.clone();
        agent.settings.reviewer_model = settings.reviewer_model.clone();
        agent.settings.image_model = settings.image_model.clone();
        agent.settings.temperature = settings.temperature;
        agent.settings.max_steps = settings.max_steps;
        agent.settings.max_reviews = settings.max_reviews;
        agent.settings.propose_concepts = settings.propose_concepts;
        if let Some(n) = steps {
            agent.allow_steps_next_turn(n);
        }
        agent.raster_tools = true;
        let next = agent.send(&text, &mut Host { s: self, png });
        self.assistant.agent = Some(agent);
        let reqs = self.ai_next(next);
        Ok(self.ai_state(reqs))
    }

    /// OpenRouter's reply to request `id`: `response` (JSON), or `error`.
    fn ai_reply(&mut self, args: &Json, png: Option<PngRenderer<'_>>) -> Result<Json, String> {
        let id = args.get("id").and_then(Json::as_u64).ok_or("missing id")?;
        let Some(i) = self.assistant.pending.iter().position(|(r, _)| *r == id) else {
            return Ok(self.ai_state(Vec::new()));
        };
        let (_, purpose) = self.assistant.pending.remove(i);
        let result = match (args.get("response"), args.get("error").and_then(Json::as_str)) {
            (_, Some(e)) => Err(AiError::Network(e.to_string())),
            (Some(r), None) => match r.get("error") {
                Some(e) => Err(AiError::Api {
                    status: 400,
                    message: e.get("message").and_then(Json::as_str).unwrap_or("error").to_string(),
                }),
                None => Ok(r.clone()),
            },
            (None, None) => Err(AiError::BadResponse("empty reply".into())),
        };
        let Some(mut agent) = self.assistant.agent.take() else { return Ok(self.ai_state(Vec::new())) };
        let next = agent.handle_response(purpose, result, &mut Host { s: self, png });
        self.assistant.agent = Some(agent);
        let reqs = self.ai_next(next);
        Ok(self.ai_state(reqs))
    }
}
