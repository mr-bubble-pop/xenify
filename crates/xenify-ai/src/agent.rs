use crate::tools::{self, ToolOutcome};
use crate::{AiError, AiSettings};
use base64::Engine;
use serde_json::{Value as Json, json};
use std::collections::VecDeque;
use xenify_anim::SceneText;
use xenify_doc::{Id, Project};
use xenify_ops::Command;

/// What the agent needs from the application to run tools.
pub trait ToolHost {
    fn project(&self) -> &Project;
    /// The composition being edited.
    fn comp(&self) -> Id;
    /// Switches which composition is edited (e.g. to work inside a precomp).
    fn set_comp(&mut self, _comp: Id) -> Result<(), String> {
        Err("switching compositions isn't available here".into())
    }
    /// The editor's current time.
    fn time(&self) -> f64;
    /// Applies a command (as part of the AI's undo group).
    fn execute(&mut self, label: &str, cmd: Command) -> Result<(), String>;
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect;
    /// Renders the composition at `time` as a PNG (a reviewer-friendly size).
    fn render_png(&mut self, time: f64) -> Result<Vec<u8>, String>;
    /// Converts SVG markup into path layers (ids from the project's next id),
    /// centred in the composition. Hosts without SVG support return an error.
    fn import_svg(&mut self, _svg: &[u8]) -> Result<Vec<xenify_doc::Layer>, String> {
        Err("SVG import isn't available here".into())
    }
    /// Traces an encoded image (PNG/JPEG/WebP) into path layers, like
    /// [`Self::import_svg`]. `colors` is 1 (posterised) … 8 (detailed).
    fn trace_image(&mut self, _image: &[u8], _colors: u8) -> Result<Vec<xenify_doc::Layer>, String> {
        Err("image tracing isn't available here".into())
    }
    /// An animated emoji (by glyph or name) and its Lottie file, from the
    /// download cache or the network. Hosts without network access return
    /// an error.
    /// The first GIPHY result for `query` and the GIF's bytes, with the
    /// user's own key. Hosts without one return an error.
    fn giphy_gif(&mut self, _query: &str) -> Result<(xenify_export::giphy::Gif, Vec<u8>), String> {
        Err("GIPHY isn't available here".into())
    }
    fn animated_emoji(
        &mut self,
        _key: &str,
    ) -> Result<(xenify_export::animated_emoji::AnimatedEmoji, Vec<u8>), String> {
        Err("animated emoji aren't available here".into())
    }
    /// Whether this host edits pixels (raster layers): the raster tools are offered then.
    fn paints_pixels(&self) -> bool {
        false
    }
    /// A pixel-editing call of the host's (`raster_begin`, `raster_fill`, `raster_select`, …).
    fn raster(&mut self, _call: &str, _args: &Json) -> Result<Json, String> {
        Err("raster layers aren't available here".into())
    }
}

/// Which model a request is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Purpose {
    Brain,
    Review {
        call_id: String,
        time: f64,
    },
    /// Generating an illustration for a `generate_illustration` call.
    Image {
        call_id: String,
        args: Json,
    },
    /// A mask for a `select_pixels` call with shape "ai" (`args`: what `raster_select_mask` needs, but the mask).
    Mask {
        call_id: String,
        args: Json,
    },
    /// Repainting a picture for a `change_picture` call (`args`: what `raster_ai_apply` needs, but the picture).
    EditPicture {
        call_id: String,
        args: Json,
    },
}

/// What the caller should do next.
#[derive(Debug)]
pub enum Next {
    /// Send this chat-completions body to OpenRouter and call
    /// [`Agent::handle_response`] with the result.
    Request { body: Json, purpose: Purpose },
    /// The agent asked a question; wait for [`Agent::send`].
    WaitForUser,
    /// The turn is finished.
    Idle,
}

/// Something to show in the chat panel.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatItem {
    User(String),
    Assistant(String),
    /// A tool the agent used (e.g. "Added rect “Ball”").
    Activity {
        text: String,
        ok: bool,
    },
    Question {
        text: String,
        options: Vec<String>,
        answered: bool,
    },
    /// The reviewer looked at a rendered frame (PNG) and said `text`.
    Review {
        time: f64,
        png: Vec<u8>,
        text: String,
    },
    /// An image model drew this picture (then it was traced into vectors).
    Generated {
        image: Vec<u8>,
        text: String,
    },
    Error(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// USD, as reported by OpenRouter.
    pub cost: f64,
}

struct ToolCall {
    id: String,
    name: String,
    args: String,
}

pub struct Agent {
    pub settings: AiSettings,
    messages: Vec<Json>,
    queue: VecDeque<ToolCall>,
    waiting_for: Option<String>,
    steps: usize,
    pub transcript: Vec<ChatItem>,
    pub usage: Usage,
    /// Whether any command was applied since [`Agent::take_changed`].
    changed: bool,
    /// Frame reviews used this turn.
    reviews: usize,
    /// The frame currently being reviewed (shown in the chat with the verdict).
    review_png: Option<Vec<u8>>,
    /// Context to prepend to the next user message (e.g. "your changes were rejected").
    pending_note: Option<String>,
    /// A bigger step budget for the next request only (e.g. a whole deck).
    next_budget: Option<usize>,
    /// The step budget of the current request.
    budget: usize,
    /// Failing calls this turn, keyed by tool + arguments (loop detection).
    failures: std::collections::HashMap<String, usize>,
    /// Offer the raster tools (the host paints pixels: [`ToolHost::paints_pixels`]).
    pub raster_tools: bool,
}

/// Messages at the end of the conversation always sent whole.
const RECENT_MESSAGES: usize = 8;
/// Older tool results (and scene snapshots in older requests) longer than
/// this are shortened: the model rarely needs them again, and resending
/// them every step makes each step slower and dearer.
const OLD_RESULT_CHARS: usize = 1200;

/// The conversation as sent: recent messages whole, older bulky tool results
/// and scene snapshots cut short. Once a message is old it stays trimmed the
/// same way, so the start of the conversation doesn't change between steps
/// (providers' prompt caching keeps working).
fn trimmed_history(messages: &[Json]) -> Vec<Json> {
    let keep_from = messages.len().saturating_sub(RECENT_MESSAGES);
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if i >= keep_from {
                return m.clone();
            }
            let role = m["role"].as_str().unwrap_or("");
            // Old tool calls: they've run; their bulky arguments needn't go again.
            if role == "assistant" && m["tool_calls"].is_array() {
                let mut m = m.clone();
                for call in m["tool_calls"].as_array_mut().into_iter().flatten() {
                    let args = &mut call["function"]["arguments"];
                    if args.as_str().is_some_and(|a| a.len() > OLD_RESULT_CHARS / 2) {
                        *args = Json::String(r#"{"note":"arguments omitted (already done)"}"#.into());
                    }
                }
                return m;
            }
            let Some(content) = m["content"].as_str() else { return m.clone() };
            if content.len() <= OLD_RESULT_CHARS || !(role == "tool" || role == "user") {
                return m.clone();
            }
            let short = match role {
                // An old request: keep what was asked, drop its scene snapshot.
                "user" => match content.split_once("<current_scene>") {
                    Some((ask, _)) => format!("{}(scene snapshot omitted: it has changed since)", ask),
                    None => return m.clone(),
                },
                _ => {
                    let mut cut = 400;
                    while !content.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    format!(
                        "{}… [older result shortened — call get_scene if you need the current state]",
                        &content[..cut]
                    )
                }
            };
            let mut m = m.clone();
            m["content"] = Json::String(short);
            m
        })
        .collect()
}

/// Identical failing calls allowed before the agent is told to change tack,
/// and before the turn is stopped.
const REPEAT_WARN: usize = 2;
const REPEAT_STOP: usize = 4;

impl Agent {
    pub fn new(settings: AiSettings) -> Self {
        let budget = settings.max_steps;
        Self {
            settings,
            messages: Vec::new(),
            queue: VecDeque::new(),
            waiting_for: None,
            steps: 0,
            transcript: Vec::new(),
            usage: Usage::default(),
            changed: false,
            next_budget: None,
            budget,
            failures: Default::default(),
            pending_note: None,
            reviews: 0,
            review_png: None,
            raster_tools: false,
        }
    }

    pub fn is_waiting_for_user(&self) -> bool {
        self.waiting_for.is_some()
    }

    /// Returns and clears whether the agent changed the project.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Tells the model something at the start of the next turn.
    pub fn note_for_next_turn(&mut self, note: &str) {
        self.pending_note = Some(note.to_string());
    }

    /// Forgets the conversation (keeps settings and total cost).
    pub fn reset(&mut self) {
        self.messages.clear();
        self.queue.clear();
        self.waiting_for = None;
        self.transcript.clear();
    }

    /// The user typed something: a new request, or the answer to a question.
    pub fn send(&mut self, text: &str, host: &mut dyn ToolHost) -> Next {
        self.transcript.push(ChatItem::User(text.to_string()));
        if let Some(call_id) = self.waiting_for.take() {
            for item in self.transcript.iter_mut().rev() {
                if let ChatItem::Question { answered, .. } = item {
                    *answered = true;
                    break;
                }
            }
            self.tool_result(&call_id, json!({"user_answer": text}));
            return self.process_queue(host);
        }
        self.steps = 0;
        self.budget = self.next_budget.take().unwrap_or(self.settings.max_steps).max(self.settings.max_steps);
        self.reviews = 0;
        self.failures.clear();
        // Give the model the current state with every request: the user may
        // have edited things by hand since the last turn.
        let (project, comp, time) = (host.project().clone(), host.comp(), host.time());
        let scene = tools::scene_summary(&project, comp, time, &mut tools::HostMeasure(host));
        let note = self.pending_note.take().map_or(String::new(), |n| format!("({n})\n\n"));
        self.messages.push(json!({
            "role": "user",
            "content": format!("{note}{text}\n\n<current_scene>\n{scene}\n</current_scene>"),
        }));
        self.brain_request()
    }

    /// Lets the next request take up to `steps` model round trips (big
    /// builds such as a whole slide deck); later requests use the setting.
    pub fn allow_steps_next_turn(&mut self, steps: usize) {
        self.next_budget = Some(steps);
    }

    /// Model round trips so far this turn.
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// Abandons the current turn, keeping the conversation valid.
    pub fn stop(&mut self) {
        if let Some(id) = self.waiting_for.take() {
            self.tool_result(&id, json!({"error": "cancelled by the user"}));
        }
        while let Some(call) = self.queue.pop_front() {
            self.tool_result(&call.id, json!({"error": "cancelled by the user"}));
        }
        self.transcript.push(ChatItem::Activity {
            text: "Stopped — anything already changed is kept for you to Accept or Reject.".into(),
            ok: true,
        });
    }

    /// Feeds back the result of a [`Next::Request`].
    pub fn handle_response(
        &mut self,
        purpose: Purpose,
        result: Result<Json, AiError>,
        host: &mut dyn ToolHost,
    ) -> Next {
        let resp = match result {
            Ok(r) => r,
            Err(e) => {
                return match purpose {
                    Purpose::Review { call_id, .. } => {
                        self.tool_result(&call_id, json!({"error": format!("frame review failed: {e}")}));
                        self.transcript
                            .push(ChatItem::Activity { text: format!("Couldn't review frame: {e}"), ok: false });
                        self.process_queue(host)
                    }
                    Purpose::Image { call_id, .. }
                    | Purpose::EditPicture { call_id, .. }
                    | Purpose::Mask { call_id, .. } => {
                        self.tool_result(&call_id, json!({"error": format!("image generation failed: {e}")}));
                        self.transcript
                            .push(ChatItem::Activity { text: format!("Couldn't generate the image: {e}"), ok: false });
                        self.process_queue(host)
                    }
                    Purpose::Brain => {
                        self.stop_quietly();
                        self.transcript.push(ChatItem::Error(e.to_string()));
                        Next::Idle
                    }
                };
            }
        };
        self.add_usage(&resp);
        if let Purpose::Mask { call_id, args } = &purpose {
            let result = crate::images::image_from_response(&resp).map_err(|e| e.to_string()).and_then(|bytes| {
                let mut a = args.clone();
                a["base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
                host.raster("raster_select_mask", &a)
            });
            match result {
                Ok(bounds) => {
                    let found = !bounds.is_null();
                    self.transcript.push(ChatItem::Activity {
                        text: if found {
                            "Selected it with the image model".into()
                        } else {
                            "The image model found nothing to select".into()
                        },
                        ok: found,
                    });
                    self.tool_result(call_id, json!({"bounds_in_layer_pixels": bounds}));
                }
                Err(e) => {
                    self.transcript.push(ChatItem::Activity { text: format!("Couldn't select it: {e}"), ok: false });
                    self.tool_result(call_id, json!({"error": e}));
                }
            }
            return self.process_queue(host);
        }
        if let Purpose::EditPicture { call_id, args } = &purpose {
            let said = crate::images::reply_text(&resp);
            let result = crate::images::image_from_response(&resp).map_err(|e| e.to_string()).and_then(|bytes| {
                let mut a = args.clone();
                a["base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
                let mut counting = CountingHost { inner: host, applied: false };
                counting.raster("raster_ai_apply", &a)?;
                self.changed |= counting.applied;
                Ok(bytes)
            });
            match result {
                Ok(bytes) => {
                    let model = &self.settings.image_model;
                    let text = if said.is_empty() {
                        format!("Changed with {model}.")
                    } else {
                        format!("Changed with {model}: {said}")
                    };
                    self.transcript.push(ChatItem::Generated { image: bytes, text });
                    let result = if said.is_empty() {
                        json!({"ok": true, "note": "The image model didn't say what it changed; look at the frame (look_at_frame) if you need to check the result."})
                    } else {
                        json!({"ok": true, "the_image_model_said": said})
                    };
                    self.tool_result(call_id, result);
                }
                Err(e) => {
                    self.transcript
                        .push(ChatItem::Activity { text: format!("Couldn't change the picture: {e}"), ok: false });
                    self.tool_result(call_id, json!({"error": e}));
                }
            }
            return self.process_queue(host);
        }
        if let Purpose::Image { call_id, args } = &purpose {
            let result = crate::images::image_from_response(&resp).map_err(|e| e.to_string()).and_then(|bytes| {
                let colors = args["colors"].as_u64().unwrap_or(5).clamp(1, 8) as u8;
                let layers = host.trace_image(&bytes, colors)?;
                let mut counting = CountingHost { inner: host, applied: false };
                let out = tools::add_artwork(&mut counting, layers, args, "Illustration");
                self.changed |= counting.applied;
                Ok((bytes, out?))
            });
            match result {
                Ok((bytes, (value, n))) => {
                    self.transcript.push(ChatItem::Generated {
                        image: bytes,
                        text: format!(
                            "Generated with {} and traced into {n} vector shapes.",
                            self.settings.image_model
                        ),
                    });
                    self.tool_result(call_id, value);
                }
                Err(e) => {
                    self.transcript.push(ChatItem::Activity { text: format!("Illustration failed: {e}"), ok: false });
                    self.tool_result(call_id, json!({"error": e}));
                }
            }
            return self.process_queue(host);
        }
        let message = resp["choices"][0]["message"].clone();
        if message.is_null() {
            let err = format!("model returned no message: {}", truncate(&resp.to_string(), 200));
            return match purpose {
                Purpose::Review { call_id, .. }
                | Purpose::Image { call_id, .. }
                | Purpose::EditPicture { call_id, .. }
                | Purpose::Mask { call_id, .. } => {
                    self.tool_result(&call_id, json!({"error": err}));
                    self.process_queue(host)
                }
                Purpose::Brain => {
                    self.transcript.push(ChatItem::Error(err));
                    Next::Idle
                }
            };
        }
        let content = message["content"].as_str().unwrap_or("").trim().to_string();

        match purpose {
            Purpose::Image { .. } | Purpose::EditPicture { .. } | Purpose::Mask { .. } => unreachable!("handled above"),
            Purpose::Review { call_id, time } => {
                self.transcript.push(ChatItem::Review {
                    time,
                    png: self.review_png.take().unwrap_or_default(),
                    text: content.clone(),
                });
                self.tool_result(&call_id, json!({"observation": content}));
                self.process_queue(host)
            }
            Purpose::Brain => {
                let calls: Vec<ToolCall> = message["tool_calls"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|c| ToolCall {
                                id: c["id"].as_str().unwrap_or_default().to_string(),
                                name: c["function"]["name"].as_str().unwrap_or_default().to_string(),
                                args: c["function"]["arguments"].as_str().unwrap_or("{}").to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut stored = json!({"role": "assistant", "content": content});
                if let Some(tc) = message.get("tool_calls").filter(|t| t.as_array().is_some_and(|a| !a.is_empty())) {
                    stored["tool_calls"] = tc.clone();
                }
                self.messages.push(stored);
                if !content.is_empty() {
                    self.transcript.push(ChatItem::Assistant(content));
                }
                if calls.is_empty() {
                    return Next::Idle;
                }
                self.queue.extend(calls);
                self.process_queue(host)
            }
        }
    }

    fn stop_quietly(&mut self) {
        while let Some(call) = self.queue.pop_front() {
            self.tool_result(&call.id, json!({"error": "cancelled"}));
        }
    }

    fn process_queue(&mut self, host: &mut dyn ToolHost) -> Next {
        while let Some(call) = self.queue.pop_front() {
            let args: Json = if call.args.trim().is_empty() {
                json!({})
            } else {
                match serde_json::from_str(&call.args) {
                    Ok(a) => a,
                    Err(e) => {
                        self.tool_result(&call.id, json!({"error": format!("arguments are not valid JSON: {e}")}));
                        continue;
                    }
                }
            };
            let mut counting = CountingHost { inner: host, applied: false };
            let outcome = tools::run(&call.name, &args, &mut counting);
            self.changed |= counting.applied;
            match outcome {
                Ok(ToolOutcome::Done { result, activity }) => {
                    if let Some(text) = activity {
                        self.transcript.push(ChatItem::Activity { text, ok: true });
                    }
                    self.tool_result(&call.id, result);
                }
                Ok(ToolOutcome::AskUser { question, options }) => {
                    self.transcript.push(ChatItem::Question { text: question, options, answered: false });
                    self.waiting_for = Some(call.id);
                    return Next::WaitForUser;
                }
                Ok(ToolOutcome::Illustrate { prompt, args }) => {
                    let body = crate::images::image_request(
                        &self.settings.image_model,
                        &crate::images::vector_friendly_prompt(&prompt),
                    );
                    return Next::Request { body, purpose: Purpose::Image { call_id: call.id, args } };
                }
                Ok(ToolOutcome::EditPicture { change, image, mask, refs, args }) => {
                    let body = crate::images::edit_request(
                        &self.settings.image_model,
                        &change,
                        &image,
                        mask.as_deref(),
                        &refs,
                    );
                    return Next::Request { body, purpose: Purpose::EditPicture { call_id: call.id, args } };
                }
                Ok(ToolOutcome::SelectWithAi { what, image, args }) => {
                    let body = crate::images::mask_request(&self.settings.image_model, &what, &image);
                    return Next::Request { body, purpose: Purpose::Mask { call_id: call.id, args } };
                }
                Ok(ToolOutcome::Review { .. }) if self.reviews >= self.settings.max_reviews => {
                    self.tool_result(
                        &call.id,
                        json!({"error": format!(
                            "Frame-check budget for this request is used up ({}). Rely on bounds and \
                             layout_warnings instead, and finish.",
                            self.settings.max_reviews
                        )}),
                    );
                }
                Ok(ToolOutcome::Review { time, question }) => match host.render_png(time) {
                    Ok(png) => {
                        self.reviews += 1;
                        self.review_png = Some(png.clone());
                        let size = host
                            .project()
                            .comp(host.comp())
                            .map_or((1920, 1080), |c| (c.settings.width, c.settings.height));
                        return self.review_request(call.id, time, &question, &png, size);
                    }
                    Err(e) => self.tool_result(&call.id, json!({"error": format!("render failed: {e}")})),
                },
                Err(e) => {
                    let n = {
                        let n = self.failures.entry(format!("{}:{}", call.name, call.args)).or_insert(0);
                        *n += 1;
                        *n
                    };
                    self.transcript.push(ChatItem::Activity { text: format!("{} failed: {e}", call.name), ok: false });
                    if n >= REPEAT_STOP {
                        self.tool_result(&call.id, json!({"error": e}));
                        self.stop_quietly();
                        self.transcript.push(ChatItem::Error(format!(
                            "Stopped: the assistant kept repeating a failing {} call.",
                            call.name
                        )));
                        return Next::Idle;
                    }
                    let hint = if n >= REPEAT_WARN {
                        " You already tried exactly this and it failed. Do not repeat it: change the arguments \
                         (check the tool schema and value types) or skip this step."
                    } else {
                        ""
                    };
                    self.tool_result(&call.id, json!({"error": format!("{e}.{hint}")}));
                }
            }
        }
        self.steps += 1;
        if self.steps >= self.budget {
            self.transcript.push(ChatItem::Error(format!(
                "Stopped after {} steps (safety limit). Ask me to continue if needed.",
                self.steps
            )));
            return Next::Idle;
        }
        self.brain_request()
    }

    fn tool_result(&mut self, call_id: &str, content: Json) {
        self.messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": content.to_string()}));
    }

    fn brain_request(&self) -> Next {
        let mut tools = tools::definitions(!self.settings.reviewer_model.trim().is_empty());
        if self.raster_tools {
            tools.extend(crate::raster_tools::definitions());
        }
        let mut messages = vec![json!({"role": "system", "content": crate::prompt::system(&self.settings)})];
        messages.extend(trimmed_history(&self.messages));
        Next::Request {
            body: json!({
                "model": self.settings.brain_model,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto",
                "temperature": self.settings.temperature,
                "usage": {"include": true},
            }),
            purpose: Purpose::Brain,
        }
    }

    fn review_request(&self, call_id: String, time: f64, question: &str, png: &[u8], comp_size: (u32, u32)) -> Next {
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        let (w, h) = comp_size;
        Next::Request {
            body: json!({
                "model": self.settings.reviewer_model,
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "text", "text": format!(
                            "You are reviewing a frame (t = {time:.2}s) of a motion-graphics animation for its designer. \
                             The image is a scaled-down render of a {w}×{h} composition. Describe positions and sizes \
                             as percentages of the frame (e.g. \"centred at x≈50%, y≈85%, about 30% wide\"), never in \
                             image pixels. Answer concisely and concretely. Be strict and critical: \
                             explicitly report ANY elements that overlap or touch, text that is clipped, too small or \
                             low-contrast, anything too close to the frame edge, and awkward composition. Do not say \
                             it looks fine unless you checked each of these.\n\nQuestion: {question}")},
                        {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{data}")}}
                    ]
                }],
                "temperature": 0.2,
                "usage": {"include": true},
            }),
            purpose: Purpose::Review { call_id, time },
        }
    }

    fn add_usage(&mut self, resp: &Json) {
        let u = &resp["usage"];
        self.usage.prompt_tokens += u["prompt_tokens"].as_u64().unwrap_or(0);
        self.usage.completion_tokens += u["completion_tokens"].as_u64().unwrap_or(0);
        self.usage.cost += u["cost"].as_f64().unwrap_or(0.0);
    }
}

/// Wraps a host to notice whether a tool actually changed the project.
struct CountingHost<'a> {
    inner: &'a mut dyn ToolHost,
    applied: bool,
}

impl ToolHost for CountingHost<'_> {
    fn project(&self) -> &Project {
        self.inner.project()
    }
    fn comp(&self) -> Id {
        self.inner.comp()
    }
    fn set_comp(&mut self, comp: Id) -> Result<(), String> {
        self.inner.set_comp(comp)
    }
    fn time(&self) -> f64 {
        self.inner.time()
    }
    fn execute(&mut self, label: &str, cmd: Command) -> Result<(), String> {
        self.inner.execute(label, cmd)?;
        self.applied = true;
        Ok(())
    }
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect {
        self.inner.text_bounds(text)
    }
    fn render_png(&mut self, time: f64) -> Result<Vec<u8>, String> {
        self.inner.render_png(time)
    }
    fn import_svg(&mut self, svg: &[u8]) -> Result<Vec<xenify_doc::Layer>, String> {
        self.inner.import_svg(svg)
    }
    fn trace_image(&mut self, image: &[u8], colors: u8) -> Result<Vec<xenify_doc::Layer>, String> {
        self.inner.trace_image(image, colors)
    }
    fn animated_emoji(&mut self, key: &str) -> Result<(xenify_export::animated_emoji::AnimatedEmoji, Vec<u8>), String> {
        self.inner.animated_emoji(key)
    }
    fn giphy_gif(&mut self, query: &str) -> Result<(xenify_export::giphy::Gif, Vec<u8>), String> {
        self.inner.giphy_gif(query)
    }
    fn paints_pixels(&self) -> bool {
        self.inner.paints_pixels()
    }
    fn raster(&mut self, call: &str, args: &Json) -> Result<Json, String> {
        let out = self.inner.raster(call, args)?;
        // Edits that save pixels (the info, inputs and selection calls don't change the drawing).
        if matches!(call, "add_raster_layer" | "raster_end" | "raster_fill" | "raster_clear" | "raster_ai_apply") {
            self.applied = true;
        }
        Ok(out)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}
