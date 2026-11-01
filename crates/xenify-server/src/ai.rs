//! AI through programs people install themselves: Ollama (the recommended
//! one for text), LocalAI, or any OpenAI-compatible service such as
//! OpenRouter; pictures and clips through ComfyUI (`comfy`).
//!
//! - Apps send AI requests here with their session (`/v1/ai/…`); the server
//!   passes them to the runner with its key, so apps need none. Answers
//!   stream straight through.
//! - Admins pick the runner and the default models, search Hugging Face and
//!   Civitai, and install models: the server downloads the file into the
//!   runner's models folder (resuming, checking its SHA-256), writes the
//!   runner's model settings and asks it to load them. Only SafeTensors and
//!   GGUF files are installed: older pickle checkpoints can run code.

use crate::{Server, admin_power, db, err, failed, signed_in, unavailable};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/ai", get(info))
        .route("/v1/ai/models", get(models))
        .route("/v1/gifs", get(gifs))
        .route("/v1/ai/chat/completions", post(|s, h, b| forward(s, h, b, "chat/completions", "text")))
        // Calc's Ask: the "Working with data" model, thinking as the admin set it.
        .route("/v1/ai/data/chat/completions", post(|s, h, b| forward(s, h, b, "chat/completions", "data")))
        .route("/v1/ai/images/generations", post(|s, h, b| forward(s, h, b, "images/generations", "image")))
        // LocalAI's video endpoint sits outside /v1.
        .route("/v1/ai/video", post(|s, h, b| forward(s, h, b, "/video", "video")))
        .route("/v1/admin/ai", get(admin_settings).post(change_settings))
        .route("/v1/admin/ai/status", get(status))
        .route("/v1/admin/ai/comfy", get(crate::comfy::admin_status))
        .route("/v1/admin/ai/search", get(search))
        .route("/v1/admin/ai/files", get(files))
        .route("/v1/admin/ai/install", post(install))
        .route("/v1/admin/ai/jobs", get(jobs))
        .route("/v1/admin/ai/jobs/{id}", axum::routing::delete(cancel))
        .route("/v1/admin/ai/styles", get(style_parts).post(make_style))
        // Requests carry pictures (a canvas to change, a picture to look at).
        .layer(axum::extract::DefaultBodyLimit::max(64 << 20))
}

/// Which kind of runner. One this version doesn't know (set by a newer
/// server, before going back to this one) reads as `Openai`, the general
/// OpenAI-style kind, instead of making all the AI settings unreadable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Runner {
    Localai,
    /// The recommended one for text (docs/ai-model-tests.md).
    #[default]
    Ollama,
    /// Any other OpenAI-compatible service (OpenRouter, LM Studio, vLLM).
    Openai,
}

impl<'de> Deserialize<'de> for Runner {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "localai" => Runner::Localai,
            "ollama" => Runner::Ollama,
            _ => Runner::Openai,
        })
    }
}

/// Models offered to people, by kind.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Offered {
    /// Writing and questions.
    pub text: Vec<String>,
    /// Working with data (Calc's Ask): models that think and call tools.
    pub data: Vec<String>,
    /// Looking at pictures.
    pub vision: Vec<String>,
    /// Making pictures.
    pub image: Vec<String>,
    /// Making video.
    pub video: Vec<String>,
    /// Picture styles: picture models made from another with a LoRA
    /// (`make_style`), also in `image`.
    pub lora: Vec<String>,
}

/// The AI settings, kept as JSON in the `settings` table under "ai".
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub runner: Runner,
    /// The runner's OpenAI-style address, e.g. http://localhost:8080/v1
    /// (Ollama: http://localhost:11434/v1).
    pub url: String,
    pub key: String,
    /// Where the runner keeps its models (LocalAI's models folder), when it
    /// runs on this computer.
    pub models_dir: String,
    /// ComfyUI's address, for pictures and clips (e.g. http://localhost:8188).
    pub comfy_url: String,
    /// The default for each kind (people can pick another of `offered`).
    pub text_model: String,
    /// Working with data (optional: Calc uses the writing model without one).
    pub data_model: String,
    /// How long the data model thinks: "quick", "normal" (empty) or "thorough".
    pub data_thinking: String,
    pub vision_model: String,
    pub image_model: String,
    pub video_model: String,
    /// The models people can choose from, by kind.
    pub offered: Offered,
    pub hf_token: String,
    pub civitai_token: String,
    /// GIPHY's API key: with one, the apps offer GIFs (searched through
    /// here, so people need no key of their own).
    pub giphy_key: String,
    /// Show Civitai models marked as adult content.
    pub adult: bool,
    /// For tests: other addresses for Hugging Face and Civitai.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hf_base: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub civitai_base: String,
    /// How much AI each person may use (AI › Limits).
    pub limits: Limits,
}

/// Limits on what people may ask the AI for; 0 means no limit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Pictures one person may have made at the same time.
    pub pictures_at_once: u32,
    /// Clips one person may have made at the same time.
    pub clips_at_once: u32,
    /// Pictures and clips made at the same time on the whole server.
    pub server_at_once: u32,
    /// Pictures and clips one person may ask for in an hour.
    pub media_per_hour: u32,
    /// Writing and questions one person may ask for in an hour.
    pub text_per_hour: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { pictures_at_once: 2, clips_at_once: 1, server_at_once: 4, media_per_hour: 60, text_per_hour: 1000 }
    }
}

impl Settings {
    fn hf(&self) -> &str {
        if self.hf_base.is_empty() { "https://huggingface.co" } else { &self.hf_base }
    }
    fn civitai(&self) -> &str {
        if self.civitai_base.is_empty() { "https://civitai.com" } else { &self.civitai_base }
    }
    fn base(&self) -> String {
        self.url.trim_end_matches('/').to_string()
    }
    /// OpenRouter has its own addresses for pictures (`images`) and video
    /// (`videos`, a job to wait for), and lists those models separately.
    fn openrouter(&self) -> bool {
        self.runner == Runner::Openai && self.url.contains("openrouter.ai")
    }
}

pub(crate) async fn settings(c: &deadpool_postgres::Client) -> Settings {
    settings_to_change(c).await.unwrap_or_default()
}

/// The settings to change and save again: refused when what's kept can't be
/// read (saved by a newer server), so saving can't replace it with defaults.
async fn settings_to_change(c: &deadpool_postgres::Client) -> Result<Settings, Response> {
    let Some(v) = db::setting(c, "ai").await.ok().flatten() else { return Ok(Settings::default()) };
    serde_json::from_str(&v).map_err(|e| {
        eprintln!("xenify-server: the AI settings can't be read ({e}): using none until they're fixed or the server is updated");
        err(
            StatusCode::CONFLICT,
            "newer_settings",
            "The AI settings were saved by a newer version of the server, so this one won't change them. Update the server.",
        )
    })
}

/// One answer from the default writing model to OpenAI-style `messages`,
/// for the server's own use (Chat's questions about people's messages);
/// None when there's no AI.
pub(crate) async fn complete(c: &deadpool_postgres::Client, messages: Vec<Value>) -> Option<Result<String, String>> {
    let a = settings(c).await;
    if a.url.is_empty() || a.text_model.is_empty() {
        return None;
    }
    let (url, key) = (format!("{}/chat/completions", a.base()), a.key.clone());
    let req = json!({ "model": a.text_model, "messages": messages, "stream": false, "temperature": 0.2 });
    let got = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut r = agent(Some(Duration::from_secs(300))).post(&url).header("Content-Type", "application/json");
        if !key.is_empty() {
            r = r.header("Authorization", &format!("Bearer {key}"));
        }
        let mut resp = r.send(req.to_string()).map_err(|e| format!("the AI runner didn't answer: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().with_config().limit(8 << 20).read_to_string().map_err(|e| e.to_string())?;
        let v: Value =
            serde_json::from_str(&text).map_err(|_| "the AI runner sent something unexpected".to_string())?;
        if status >= 400 {
            let why = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("it refused");
            return Err(format!("the AI runner said no: {why}"));
        }
        Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string())
    })
    .await;
    Some(got.unwrap_or_else(|e| Err(e.to_string())))
}

fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(timeout).http_status_as_error(false).build().into()
}

/// A blocking GET returning JSON, on a worker thread.
async fn get_json(url: String, token: Option<String>) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        let mut req = agent(Some(Duration::from_secs(30))).get(&url).header("Accept", "application/json");
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        let mut r = req.call().map_err(|e| format!("can't reach {url}: {e}"))?;
        let status = r.status().as_u16();
        let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(format!("{url} answered {status}"));
        }
        serde_json::from_str(&text).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- For the apps ---------------------------------------------------------

#[derive(Deserialize)]
struct GifQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    offset: u32,
}

/// GIPHY search (or what's trending, without `q`) with the server's key:
/// GIPHY's own answer, which the apps read. 404 when there's no key.
async fn gifs(State(s): State<Server>, h: HeaderMap, Query(q): Query<GifQuery>) -> Response {
    if let Err(e) = signed_in(&s, &h).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let key = settings(&c).await.giphy_key;
    if key.trim().is_empty() {
        return err(StatusCode::NOT_FOUND, "no_gifs", "this server has no GIPHY key: its admin can add one on the AI page");
    }
    let query = q.q.trim().to_string();
    let base = if query.is_empty() {
        "https://api.giphy.com/v1/gifs/trending?".to_string()
    } else {
        format!("https://api.giphy.com/v1/gifs/search?q={}&", enc(&query))
    };
    let url = format!("{base}api_key={}&limit=24&offset={}&rating=g&bundle=messaging_non_clips", enc(key.trim()), q.offset.min(4999));
    let got = tokio::task::spawn_blocking(move || {
        let mut r = agent(Some(Duration::from_secs(20))).get(&url).call().map_err(|_| "GIPHY didn't answer".to_string())?;
        let status = r.status().as_u16();
        let text = r.body_mut().with_config().limit(4 << 20).read_to_string().map_err(|e| e.to_string())?;
        Ok::<_, String>((status, text))
    })
    .await;
    match got {
        Ok(Ok((status, text))) => {
            let code = if status == 200 { StatusCode::OK } else { StatusCode::BAD_GATEWAY };
            (code, [(header::CONTENT_TYPE, "application/json")], text).into_response()
        }
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, "giphy", &e),
        Err(e) => err(StatusCode::BAD_GATEWAY, "giphy", &e.to_string()),
    }
}

/// What AI the apps can use here (no secrets).
async fn info(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = signed_in(&s, &h).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    // The defaults are always among what's offered.
    let with = |mut list: Vec<String>, default: &str| {
        if !default.is_empty() && !list.iter().any(|m| m == default) {
            list.insert(0, default.to_string());
        }
        list
    };
    Json(json!({
        "available": !a.url.is_empty() || !a.comfy_url.trim().is_empty(),
        // GIFs (GIPHY) through /v1/gifs: not AI, but set up on the same page.
        "gifs": !a.giphy_key.trim().is_empty(),
        "text_model": a.text_model,
        "data_model": a.data_model,
        "data_thinking": if a.data_thinking.is_empty() { "normal" } else { &a.data_thinking },
        "vision_model": a.vision_model,
        "image_model": a.image_model,
        "video_model": a.video_model,
        "offered": {
            "text": with(a.offered.text.clone(), &a.text_model),
            "data": with(a.offered.data.clone(), &a.data_model),
            "vision": with(a.offered.vision.clone(), &a.vision_model),
            "image": with(a.offered.image.clone(), &a.image_model),
            "video": with(a.offered.video.clone(), &a.video_model),
            "lora": a.offered.lora,
        },
    }))
    .into_response()
}

async fn models(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    if a.url.is_empty() {
        return no_ai();
    }
    match get_json(format!("{}/models", a.base()), Some(a.key.clone())).await {
        Ok(v) => Json(v).into_response(),
        // The runner's address is for those who look after AI.
        Err(e) if me.admin || crate::perms::can(&c, me.id, "admin.ai").await => {
            err(StatusCode::BAD_GATEWAY, "runner", &format!("the AI runner didn't answer: {e}"))
        }
        Err(_) => err(StatusCode::BAD_GATEWAY, "runner", "the AI runner didn't answer"),
    }
}

/// Asks the runner for pictures or video and returns its answer with each
/// file's bytes (`b64_json`) in place of a link to it.
fn made_files(url: &str, key: &str, req: &Value, root: &str) -> Result<(u16, Value), String> {
    let mut r = agent(Some(Duration::from_secs(1800))).post(url).header("Content-Type", "application/json");
    if !key.is_empty() {
        r = r.header("Authorization", &format!("Bearer {key}"));
    }
    let mut resp = r.send(req.to_string()).map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(512 << 20).read_to_string().map_err(|e| e.to_string())?;
    let mut v: Value = serde_json::from_str(&text).map_err(|_| "the runner sent something unexpected".to_string())?;
    if let Some(items) = v.get_mut("data").and_then(Value::as_array_mut) {
        for item in items {
            if item.get("b64_json").and_then(Value::as_str).is_some() {
                continue;
            }
            let Some(link) = item.get("url").and_then(Value::as_str).map(String::from) else { continue };
            // Only files on the runner itself.
            let file = if link.starts_with('/') { format!("{root}{link}") } else { link.clone() };
            // The runner's own address, not one that merely starts the same
            // ("http://runner:8080.example.org/…").
            let base = root.trim_end_matches('/');
            if !file.strip_prefix(base).is_some_and(|rest| rest.starts_with('/')) {
                continue;
            }
            let mut g = agent(Some(Duration::from_secs(600))).get(&file);
            if !key.is_empty() {
                g = g.header("Authorization", &format!("Bearer {key}"));
            }
            let bytes = g
                .call()
                .map_err(|e| e.to_string())?
                .body_mut()
                .with_config()
                .limit(512 << 20)
                .read_to_vec()
                .map_err(|e| e.to_string())?;
            item["b64_json"] = json!(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes));
            if let Some(o) = item.as_object_mut() {
                o.remove("url");
            }
        }
    }
    Ok((status, v))
}

/// An OpenAI-style picture request as OpenRouter's `images` takes it: the
/// size as a shape (its models don't all take pixels), base64 answers
/// always.
fn openrouter_picture(req: &mut Value) {
    let Some(o) = req.as_object_mut() else { return };
    o.remove("response_format");
    let Some(size) = o.remove("size") else { return };
    let wh = size.as_str().and_then(|t| t.split_once('x')).and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
    if let Some((w, h)) = wh.filter(|&(w, h)| w > 0 && h > 0) {
        let gcd = |mut a: u32, mut b: u32| {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        };
        let g = gcd(w, h);
        o.insert("aspect_ratio".into(), json!(format!("{}:{}", w / g, h / g)));
    }
}

/// Runs blocking work (a call to the runner) on a worker thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Makes a clip with OpenRouter: sends the job, waits for it (up to half an
/// hour) and answers as LocalAI's video does, the file as `b64_json`. The
/// waiting stops when the person goes away (the request is dropped).
async fn openrouter_video(base: String, key: String, req: Value) -> Result<Value, String> {
    let auth = format!("Bearer {key}");
    let body = json!({ "model": req["model"], "prompt": req["prompt"] });
    let (b, a) = (base.clone(), auth.clone());
    let job: Value = blocking(move || {
        let mut r = agent(Some(Duration::from_secs(60)))
            .post(&format!("{b}/videos"))
            .header("Content-Type", "application/json")
            .header("Authorization", &a)
            .send(body.to_string())
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
            .map_err(|_| "it sent something unexpected".to_string())
    })
    .await?;
    let id = job["id"].as_str().ok_or_else(|| job["error"]["message"].as_str().unwrap_or("no job").to_string())?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) {
        return Err("it sent something unexpected".into());
    }
    let id = id.to_string();
    let started = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (b, a, i) = (base.clone(), auth.clone(), id.clone());
        let v: Value = blocking(move || {
            let mut g = agent(Some(Duration::from_secs(60)))
                .get(&format!("{b}/videos/{i}"))
                .header("Authorization", &a)
                .call()
                .map_err(|e| e.to_string())?;
            serde_json::from_str(&g.body_mut().read_to_string().map_err(|e| e.to_string())?)
                .map_err(|_| "it sent something unexpected".to_string())
        })
        .await?;
        match v["status"].as_str() {
            Some("completed") => break,
            Some("failed") => {
                let e = &v["error"];
                return Err(e.as_str().or(e["message"].as_str()).unwrap_or("it failed").to_string());
            }
            _ if started.elapsed() > Duration::from_secs(1800) => return Err("it took longer than half an hour".into()),
            _ => {}
        }
    }
    let bytes = blocking(move || {
        agent(Some(Duration::from_secs(600)))
            .get(&format!("{base}/videos/{id}/content"))
            .header("Authorization", &auth)
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(512 << 20)
            .read_to_vec()
            .map_err(|e| e.to_string())
    })
    .await?;
    Ok(json!({ "data": [{ "b64_json": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes) }] }))
}

// ---- What people may ask for ------------------------------------------------

/// The models people may ask for with a request of `kind`: those offered,
/// and the defaults. Writing requests may also name a model that looks at
/// pictures, or a picture model (Draw changes pictures through a chat).
pub fn offered_for(a: &Settings, kind: &str) -> Vec<String> {
    let o = &a.offered;
    let list: Vec<&String> = match kind {
        "image" => o.image.iter().chain(&o.lora).chain([&a.image_model]).collect(),
        "video" => o.video.iter().chain([&a.video_model]).collect(),
        "data" => o.data.iter().chain(&o.text).chain([&a.data_model, &a.text_model]).collect(),
        _ => o
            .text
            .iter()
            .chain(&o.vision)
            .chain(&o.image)
            .chain(&o.lora)
            .chain([&a.text_model, &a.vision_model, &a.image_model])
            .collect(),
    };
    let mut out: Vec<String> = Vec::new();
    for m in list {
        if !m.is_empty() && !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// Whether every picture (or other file) in these messages is in the
/// request itself (`data:`), not a link the runner would go and fetch.
fn only_inline_files(v: &Value) -> bool {
    match v {
        Value::Array(list) => list.iter().all(only_inline_files),
        Value::Object(o) => o.iter().all(|(k, x)| {
            if k.ends_with("_url") {
                let url = x.as_str().or_else(|| x.get("url").and_then(Value::as_str));
                return match url {
                    Some(u) => u.trim_start().get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")),
                    None => x.is_null(),
                };
            }
            only_inline_files(x)
        }),
        _ => true,
    }
}

/// A request rebuilt from only the fields the apps use (so nobody can pick
/// the provider, add plugins or spend more than a request should), with
/// numbers kept sensible.
pub fn clean_request(req: &Value, kind: &str) -> Result<Value, String> {
    let Some(o) = req.as_object() else { return Err("that isn't a request".into()) };
    let keep: &[&str] = match kind {
        "image" => &["model", "prompt", "size", "n", "response_format", "quality"],
        "video" => &["model", "prompt", "size"],
        _ => &[
            "model",
            "messages",
            "temperature",
            "top_p",
            "max_tokens",
            "max_completion_tokens",
            "stream",
            "tools",
            "tool_choice",
            "response_format",
            "modalities",
            "stop",
            "seed",
        ],
    };
    let mut out = serde_json::Map::new();
    for k in keep {
        if let Some(v) = o.get(*k).filter(|v| !v.is_null()) {
            out.insert(k.to_string(), v.clone());
        }
    }
    match out.get("model") {
        None => {
            out.insert("model".into(), json!(""));
        }
        Some(Value::String(_)) => {}
        Some(_) => return Err("the model is named by its name".into()),
    }
    let clamp = |out: &mut serde_json::Map<String, Value>, k: &str, lo: f64, hi: f64, whole: bool| {
        match out.get(k).and_then(Value::as_f64) {
            Some(x) if whole => {
                out.insert(k.into(), json!(x.clamp(lo, hi) as u64));
            }
            Some(x) => {
                out.insert(k.into(), json!(x.clamp(lo, hi)));
            }
            None => {
                out.remove(k);
            }
        }
    };
    match kind {
        "image" | "video" => {
            if !out.get("prompt").is_none_or(Value::is_string) {
                return Err("say what to make in words".into());
            }
            if kind == "image" {
                clamp(&mut out, "n", 1.0, 4.0, true);
            }
        }
        _ => {
            let Some(messages) = out.get("messages").filter(|m| m.is_array()) else {
                return Err("a request needs its messages".into());
            };
            if !only_inline_files(messages) {
                return Err("pictures go in the request itself, not as links".into());
            }
            clamp(&mut out, "temperature", 0.0, 2.0, false);
            clamp(&mut out, "top_p", 0.0, 1.0, false);
            clamp(&mut out, "max_tokens", 1.0, 32768.0, true);
            clamp(&mut out, "max_completion_tokens", 1.0, 32768.0, true);
            if !out.get("stream").is_none_or(Value::is_boolean) {
                out.remove("stream");
            }
            // OpenRouter's "say what it cost", and nothing else under it.
            if let Some(b) = o.get("usage").and_then(|u| u.get("include")).and_then(Value::as_bool) {
                out.insert("usage".into(), json!({ "include": b }));
            }
        }
    }
    Ok(Value::Object(out))
}

/// What's being made right now and what each person asked for lately, for
/// the limits (kept while the server runs).
#[derive(Default)]
struct Busy {
    /// Pictures ("image") and clips ("video") being made, by person.
    making: HashMap<(i64, &'static str), u32>,
    /// All of them, on the whole server.
    total: u32,
    /// When each person asked: (person, pictures or clips?) → times.
    asked: HashMap<(i64, bool), std::collections::VecDeque<std::time::Instant>>,
}

static BUSY: std::sync::LazyLock<Mutex<Busy>> = std::sync::LazyLock::new(Mutex::default);

/// A picture or clip being made: its place is given back when this goes
/// (done, failed, or the person went away).
pub struct Slot {
    user: i64,
    kind: Option<&'static str>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let Some(k) = self.kind else { return };
        let mut b = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = b.making.get_mut(&(self.user, k)) {
            *n = n.saturating_sub(1);
        }
        b.total = b.total.saturating_sub(1);
    }
}

/// Room for one more request of `kind` from `user`, within the limits; or
/// why not.
pub fn take_slot(l: &Limits, user: i64, kind: &str) -> Result<Slot, &'static str> {
    let media = kind == "image" || kind == "video";
    let mut b = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let per_hour = if media { l.media_per_hour } else { l.text_per_hour };
    let asked = {
        let q = b.asked.entry((user, media)).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(3600)) {
            q.pop_front();
        }
        q.len() as u32
    };
    if per_hour > 0 && asked >= per_hour {
        return Err(if media {
            "you've asked for a lot of pictures and clips this hour: try again later"
        } else {
            "you've asked the AI a lot this hour: try again later"
        });
    }
    let kind = match kind {
        "image" => Some("image"),
        "video" => Some("video"),
        _ => None,
    };
    if let Some(k) = kind {
        let mine = b.making.get(&(user, k)).copied().unwrap_or(0);
        let most = if k == "video" { l.clips_at_once } else { l.pictures_at_once };
        if most > 0 && mine >= most {
            return Err(if k == "video" {
                "a clip of yours is still being made: wait for it to finish"
            } else {
                "your other pictures are still being made: wait for one to finish"
            });
        }
        if l.server_at_once > 0 && b.total >= l.server_at_once {
            return Err("the server is busy making other pictures and clips: try again in a minute");
        }
        *b.making.entry((user, k)).or_default() += 1;
        b.total += 1;
    }
    b.asked.entry((user, media)).or_default().push_back(now);
    Ok(Slot { user, kind })
}

fn no_ai() -> Response {
    err(StatusCode::SERVICE_UNAVAILABLE, "no_ai", "this server has no AI set up: an admin can add it under AI")
}

/// Passes a request to the runner, filling in the default model, and
/// streams the answer back. Only the models offered and the fields the apps
/// use go through, within the limits (AI › Limits).
async fn forward(
    State(s): State<Server>,
    h: HeaderMap,
    body: Bytes,
    path: &'static str,
    kind: &'static str,
) -> Response {
    let perm = match kind {
        "image" => "ai.pictures",
        "video" => "ai.video",
        _ => "ai.text",
    };
    let me = match crate::perms::need(&s, &h, perm).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    drop(c);
    let Ok(raw) = serde_json::from_slice::<Value>(&body) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "that isn't JSON");
    };
    let mut req = match clean_request(&raw, kind) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    let named = req["model"].as_str().unwrap_or_default().to_string();
    if named.is_empty() {
        let default = match kind {
            "image" => &a.image_model,
            "video" => &a.video_model,
            "data" if !a.data_model.is_empty() => &a.data_model,
            _ => &a.text_model,
        };
        req["model"] = json!(default);
    } else if !offered_for(&a, kind).contains(&named) {
        return err(StatusCode::BAD_REQUEST, "model_not_offered", "that model isn't offered here: pick one of the server's");
    }
    let body = Bytes::from(req.to_string());
    // Pictures and clips from a model ComfyUI makes.
    let comfy = if kind == "image" || kind == "video" { crate::comfy::chosen(&a, &body, kind) } else { None };
    if comfy.is_none() && a.url.is_empty() {
        return no_ai();
    }
    let slot = match take_slot(&a.limits, me.id, kind) {
        Ok(slot) => slot,
        Err(why) => return err(StatusCode::TOO_MANY_REQUESTS, "busy", why),
    };
    if let Some(r) = comfy {
        // Going away stops it on ComfyUI too, and gives the place back.
        let out = crate::comfy::make(&a, r, &body).await;
        drop(slot);
        return out;
    }
    // Working with data: thinking, asked for the way this runner understands.
    if kind == "data" {
        think_how(&mut req, &a);
    }
    let key = a.key.clone();
    if a.openrouter() && kind == "video" {
        let out = match openrouter_video(a.base(), key, req).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => err(StatusCode::BAD_GATEWAY, "runner", &format!("OpenRouter didn't make the clip: {e}")),
        };
        drop(slot);
        return out;
    }
    let path = if a.openrouter() && kind == "image" {
        openrouter_picture(&mut req);
        "images"
    } else {
        path
    };
    let url = match path.strip_prefix('/') {
        Some(root) => format!("{}/{root}", a.base().trim_end_matches("/v1")),
        None => format!("{}/{path}", a.base()),
    };
    // Pictures and video: the runner may answer with a link to the file on
    // itself, which the apps can't reach; the file is fetched and sent.
    if kind == "image" || kind == "video" {
        let root = a.base().trim_end_matches("/v1").to_string();
        // The runner can't be stopped part way: the place stays taken
        // until it's done, even if the person goes away.
        let got = tokio::task::spawn_blocking(move || {
            let _slot = slot;
            made_files(&url, &key, &req, &root)
        })
        .await;
        return match got {
            Ok(Ok((status, v))) => {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), Json(v)).into_response()
            }
            Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, "runner", &format!("the AI runner didn't answer: {e}")),
            Err(_) => failed(),
        };
    }
    drop(slot);
    let (head_tx, head_rx) = tokio::sync::oneshot::channel::<Result<(u16, String), String>>();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(32);
    tokio::task::spawn_blocking(move || {
        let mut r = agent(Some(Duration::from_secs(600))).post(&url).header("Content-Type", "application/json");
        if !key.is_empty() {
            r = r.header("Authorization", &format!("Bearer {key}"));
        }
        let resp = match r.send(req.to_string()) {
            Ok(resp) => resp,
            Err(e) => {
                let _ = head_tx.send(Err(e.to_string()));
                return;
            }
        };
        let ctype =
            resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("application/json").to_string();
        let _ = head_tx.send(Ok((resp.status().as_u16(), ctype.clone())));
        let mut reader = resp.into_body().into_reader();
        // Working with data: the thinking goes apart from the answer.
        if kind == "data" {
            let mut all = Vec::new();
            let mut split = ThinkSplit::default();
            let mut line = Vec::new();
            let mut buf = vec![0u8; 16 * 1024];
            let stream = ctype.contains("event-stream");
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        return;
                    }
                };
                if !stream {
                    all.extend_from_slice(&buf[..n]);
                    continue;
                }
                let mut out = Vec::new();
                for &b in &buf[..n] {
                    line.push(b);
                    if b == b'\n' {
                        out.extend(split.line(&String::from_utf8_lossy(&line)).into_bytes());
                        line.clear();
                    }
                }
                if !out.is_empty() && tx.blocking_send(Ok(Bytes::from(out))).is_err() {
                    return;
                }
            }
            let rest =
                if stream { split.line(&String::from_utf8_lossy(&line)).into_bytes() } else { split.whole(&all) };
            let _ = tx.blocking_send(Ok(Bytes::from(rest)));
            return;
        }
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(Ok(Bytes::copy_from_slice(&buf[..n]))).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    break;
                }
            }
        }
    });
    let (status, ctype) = match head_rx.await {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return err(StatusCode::BAD_GATEWAY, "runner", &format!("the AI runner didn't answer: {e}")),
        Err(_) => return failed(),
    };
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|x| (x, rx)) });
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (status, [(header::CONTENT_TYPE, ctype)], Body::from_stream(stream)).into_response()
}

/// Asks the data model to think, the way its runner understands: an
/// OpenAI-style `reasoning_effort` (OpenRouter: `reasoning.effort`; Ollama
/// reads it too). LocalAI's thinking models (Qwen3) think by themselves.
fn think_how(req: &mut Value, a: &Settings) {
    let effort = match a.data_thinking.as_str() {
        "quick" => "low",
        "thorough" => "high",
        _ => "medium",
    };
    if req.get("reasoning_effort").is_some() || req.get("reasoning").is_some() {
        return;
    }
    match a.runner {
        Runner::Openai if a.openrouter() => req["reasoning"] = json!({ "effort": effort }),
        Runner::Openai | Runner::Ollama => req["reasoning_effort"] = json!(effort),
        Runner::Localai => {}
    }
}

/// Separates a model's thinking from its answer: thinking written into the
/// answer between `<think>` tags (Qwen3 and others through LocalAI) goes to
/// `reasoning`, as do `reasoning_content` (DeepSeek style) and OpenRouter's
/// `reasoning`, so the apps read one field.
#[derive(Default)]
pub struct ThinkSplit {
    inside: bool,
    carry: String,
}

impl ThinkSplit {
    /// Text of the answer: (thinking, answer).
    pub fn push(&mut self, text: &str) -> (String, String) {
        let mut s = std::mem::take(&mut self.carry) + text;
        let (mut think, mut answer) = (String::new(), String::new());
        loop {
            let tag = if self.inside { "</think>" } else { "<think>" };
            if let Some(at) = s.find(tag) {
                let before = &s[..at];
                if self.inside {
                    think.push_str(before)
                } else {
                    answer.push_str(before)
                }
                self.inside = !self.inside;
                s = s[at + tag.len()..].to_string();
                continue;
            }
            // A tag cut in two by the stream: keep its start for next time.
            let keep = (1..tag.len())
                .rev()
                .find(|k| s.len() >= *k && s.is_char_boundary(s.len() - k) && tag.starts_with(&s[s.len() - k..]))
                .unwrap_or(0);
            let (body, rest) = s.split_at(s.len() - keep);
            if self.inside {
                think.push_str(body)
            } else {
                answer.push_str(body)
            }
            self.carry = rest.to_string();
            break;
        }
        (think, answer)
    }

    fn part(&mut self, m: &mut Value) {
        let mut thinking = String::new();
        for k in ["reasoning_content", "reasoning"] {
            if let Some(t) = m.get(k).and_then(Value::as_str) {
                thinking.push_str(t);
            }
        }
        if let Some(text) = m.get("content").and_then(Value::as_str).map(str::to_string) {
            let (t, a) = self.push(&text);
            thinking.push_str(&t);
            m["content"] = json!(a);
        }
        if let Some(o) = m.as_object_mut() {
            o.remove("reasoning_content");
            if thinking.is_empty() {
                o.remove("reasoning");
            } else {
                o.insert("reasoning".into(), json!(thinking));
            }
        }
    }

    /// One line of a streamed answer (`data: {…}`), changed.
    pub fn line(&mut self, line: &str) -> String {
        let Some(json_text) = line.trim_end().strip_prefix("data:").map(str::trim) else { return line.to_string() };
        let Ok(mut v) = serde_json::from_str::<Value>(json_text) else { return line.to_string() };
        if let Some(choices) = v.get_mut("choices").and_then(Value::as_array_mut) {
            for c in choices {
                if let Some(d) = c.get_mut("delta") {
                    self.part(d);
                }
            }
        }
        format!("data: {v}\n")
    }

    /// A whole (not streamed) answer, changed.
    pub fn whole(&mut self, body: &[u8]) -> Vec<u8> {
        let Ok(mut v) = serde_json::from_slice::<Value>(body) else { return body.to_vec() };
        if let Some(choices) = v.get_mut("choices").and_then(Value::as_array_mut) {
            for c in choices {
                if let Some(m) = c.get_mut("message") {
                    *self = ThinkSplit::default();
                    self.part(m);
                }
            }
        }
        v.to_string().into_bytes()
    }
}

#[cfg(test)]
mod think_tests {
    use super::{Runner, Settings, ThinkSplit, think_how};
    use serde_json::json;

    #[test]
    fn each_runner_is_asked_to_think_its_way() {
        let ask = |runner, url: &str, thinking: &str| {
            let a = Settings { runner, url: url.into(), data_thinking: thinking.into(), ..Default::default() };
            let mut req = json!({ "model": "m" });
            think_how(&mut req, &a);
            req
        };
        assert_eq!(
            ask(Runner::Openai, "https://openrouter.ai/api/v1", "quick")["reasoning"],
            json!({ "effort": "low" })
        );
        assert_eq!(ask(Runner::Openai, "http://lmstudio:1234/v1", "")["reasoning_effort"], "medium");
        assert_eq!(ask(Runner::Ollama, "http://localhost:11434/v1", "thorough")["reasoning_effort"], "high");
        assert_eq!(ask(Runner::Localai, "http://localhost:8080/v1", "thorough"), json!({ "model": "m" }));
    }

    #[test]
    fn thinking_goes_apart_even_when_cut() {
        let mut t = ThinkSplit::default();
        let mut think = String::new();
        let mut answer = String::new();
        for chunk in ["<thi", "nk>Sum the ", "column</th", "ink>\n\nProfit is ", "<", "b>12</b>"] {
            let (a, b) = t.push(chunk);
            think.push_str(&a);
            answer.push_str(&b);
        }
        let (a, b) = t.push("");
        think.push_str(&a);
        answer.push_str(&b);
        assert_eq!(think, "Sum the column");
        assert_eq!(answer, "\n\nProfit is <b>12</b>");
        let mut t = ThinkSplit::default();
        let out = t.line(r#"data: {"choices":[{"delta":{"reasoning_content":"hmm","content":"Hi"}}]}"#);
        assert!(out.contains(r#""reasoning":"hmm""#) && !out.contains("reasoning_content"), "{out}");
        let whole =
            String::from_utf8(t.whole(br#"{"choices":[{"message":{"content":"<think>a</think>b"}}]}"#)).unwrap();
        assert!(whole.contains(r#""content":"b""#) && whole.contains(r#""reasoning":"a""#), "{whole}");
    }
}

// ---- For admins ---------------------------------------------------------------

fn masked(a: &Settings) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or_default();
    for k in ["key", "hf_token", "civitai_token", "giphy_key"] {
        let set = v[k].as_str().is_some_and(|s| !s.trim().is_empty());
        v[k] = json!("");
        v[format!("{k}_set")] = json!(set);
    }
    v
}

async fn admin_settings(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    Json(masked(&settings(&c).await)).into_response()
}

/// Where an address goes: its scheme, host and port ("https://openrouter.ai").
pub fn origin(url: &str) -> String {
    let u = url.trim();
    let (scheme, rest) = u.split_once("://").unwrap_or(("http", u));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    format!("{}://{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
}

/// Whether `dir` is inside the server's data folder `data`.
fn inside(dir: &str, data: &std::path::Path) -> bool {
    let p = std::path::Path::new(dir);
    if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return false;
    }
    let data = data.canonicalize().unwrap_or_else(|_| data.to_path_buf());
    // The deepest part that's there, so links can't lead out.
    let mut have = p.to_path_buf();
    let mut rest = Vec::new();
    while !have.exists() {
        let Some(name) = have.file_name().map(|n| n.to_os_string()) else { return false };
        rest.push(name);
        if !have.pop() {
            return false;
        }
    }
    let Ok(mut real) = have.canonicalize() else { return false };
    real.extend(rest.iter().rev());
    real.starts_with(&data)
}

/// Changes the settings given; secrets left empty stay as they were, unless
/// the address they're sent to changes (then they're cleared, so a new
/// address can't be used to read them).
async fn change_settings(State(s): State<Server>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    let me = match admin_power(&s, &h, "admin.ai").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let before = match settings_to_change(&c).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let mut cur = serde_json::to_value(&before).unwrap_or_default();
    let Some(obj) = v.as_object() else { return err(StatusCode::BAD_REQUEST, "bad_request", "expected settings") };
    let given = |k: &str| obj.get(k).and_then(Value::as_str).is_some_and(|x| !x.is_empty());
    for (k, val) in obj {
        if ["key", "hf_token", "civitai_token", "giphy_key"].contains(&k.as_str()) && val.as_str().is_some_and(str::is_empty) {
            continue;
        }
        cur[k] = val.clone();
    }
    let Ok(mut new) = serde_json::from_value::<Settings>(cur) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "some settings aren't right");
    };
    if !new.url.is_empty() && !(new.url.starts_with("http://") || new.url.starts_with("https://")) {
        return err(StatusCode::BAD_REQUEST, "bad_url", "the runner's address starts with http:// or https://");
    }
    let mut cleared = Vec::new();
    if origin(&before.url) != origin(&new.url) && !given("key") && !new.key.is_empty() {
        new.key.clear();
        cleared.push("key");
    }
    if origin(before.hf()) != origin(new.hf()) && !given("hf_token") && !new.hf_token.is_empty() {
        new.hf_token.clear();
        cleared.push("hf_token");
    }
    if origin(before.civitai()) != origin(new.civitai()) && !given("civitai_token") && !new.civitai_token.is_empty() {
        new.civitai_token.clear();
        cleared.push("civitai_token");
    }
    // Models are written into the models folder: anywhere only for full
    // admins, inside the server's data folder for others.
    if new.models_dir != before.models_dir && !new.models_dir.is_empty() && !me.admin && !inside(&new.models_dir, &s.data) {
        return err(
            StatusCode::FORBIDDEN,
            "models_dir",
            &format!("only full admins can put the models folder outside the server's data folder ({})", s.data.display()),
        );
    }
    match db::set_setting(&c, "ai", &serde_json::to_string(&new).unwrap()).await {
        Ok(()) => {
            let mut out = masked(&new);
            out["cleared"] = json!(cleared);
            Json(out).into_response()
        }
        Err(_) => failed(),
    }
}

/// Whether the runner answers, and the models it has.
async fn status(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    if a.url.is_empty() {
        return Json(json!({ "set_up": false })).into_response();
    }
    // OpenRouter lists only models that write unless asked for all; its
    // picture models (for `images`) are listed apart.
    let all = if a.openrouter() { "?output_modalities=all" } else { "" };
    match get_json(format!("{}/models{all}", a.base()), Some(a.key.clone())).await {
        Ok(v) => {
            let pictures: Vec<Value> = if a.openrouter() {
                get_json(format!("{}/images/models", a.base()), Some(a.key.clone()))
                    .await
                    .ok()
                    .and_then(|p| p["data"].as_array().map(|d| d.iter().filter_map(|m| m.get("id").cloned()).collect()))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let list: Vec<Value> = v["data"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut m| {
                    // Only those `images` makes; the others answer chats.
                    if !pictures.is_empty() && !pictures.contains(&m["id"]) && let Some(o) = m["architecture"]["output_modalities"].as_array_mut() {
                        o.retain(|x| x != "image");
                    }
                    m
                })
                .filter(|m| kinds_of(m).is_none_or(|k| !k.is_empty()))
                .collect();
            let ids: Vec<Value> = list.iter().filter_map(|m| m.get("id").cloned()).collect();
            let kinds: serde_json::Map<String, Value> = list
                .iter()
                .filter_map(|m| Some((m["id"].as_str()?.to_string(), json!(kinds_of(m)?))))
                .collect();
            Json(json!({ "set_up": true, "reachable": true, "models": ids, "kinds": kinds })).into_response()
        }
        Err(e) => Json(json!({ "set_up": true, "reachable": false, "error": e })).into_response(),
    }
}

/// What a model in the runner's list can be offered for, when the list says
/// (OpenRouter's does: what each model takes in, gives back and supports).
/// None when it doesn't, so every kind offers it. Pictures and video from
/// a text prompt; clips made from another clip (editing, upscaling) aren't
/// offered.
fn kinds_of(m: &Value) -> Option<Vec<&'static str>> {
    let has = |v: &Value, x: &str| v.as_array().is_some_and(|a| a.iter().any(|y| y == x));
    let arch = m.get("architecture")?;
    let (input, output) = (&arch["input_modalities"], &arch["output_modalities"]);
    if !input.is_array() || !output.is_array() {
        return None;
    }
    let mut kinds = Vec::new();
    if has(input, "text") && has(output, "text") {
        kinds.push("text");
        if has(&m["supported_parameters"], "tools") {
            kinds.push("data");
        }
        if has(input, "image") {
            kinds.push("vision");
        }
    }
    if has(input, "text") && has(output, "image") {
        kinds.push("image");
    }
    if has(input, "text") && has(output, "video") && (has(input, "image") || !has(input, "video")) {
        kinds.push("video");
    }
    Some(kinds)
}

/// One model found on Hugging Face or Civitai.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub source: String,
    /// Hugging Face: "owner/repo"; Civitai: the model number.
    pub id: String,
    pub name: String,
    /// "text", "image" or "lora".
    pub kind: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub gated: bool,
    #[serde(default)]
    pub adult: bool,
    #[serde(default)]
    pub base_model: String,
    #[serde(default)]
    pub downloads: i64,
}

/// A file that can be installed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelFile {
    pub name: String,
    pub size: i64,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    /// SafeTensors or GGUF (pickle checkpoints are never installed).
    pub safe: bool,
    /// Civitai: the version this file belongs to.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub base_model: String,
    /// A model split into numbered files ("…-00001-of-00003.gguf"): the
    /// others, downloaded along with this first one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more: Vec<ModelFile>,
}

/// "x-00002-of-00003.gguf" → Some(("x", 2, 3)).
fn split_part(name: &str) -> Option<(&str, u32, u32)> {
    let stem = name.strip_suffix(".gguf")?;
    let (head, of) = stem.rsplit_once("-of-")?;
    let (base, n) = head.rsplit_once('-')?;
    Some((base, n.parse().ok()?, of.parse().ok()?))
}

/// Puts split models' parts under their first file.
fn join_parts(files: Vec<ModelFile>) -> Vec<ModelFile> {
    let (parts, mut out): (Vec<_>, Vec<_>) =
        files.into_iter().partition(|f| split_part(&f.name).is_some_and(|p| p.1 > 1));
    for f in &mut out {
        if let Some((base, 1, of)) = split_part(&f.name) {
            let mut more: Vec<ModelFile> = parts
                .iter()
                .filter(|p| split_part(&p.name).is_some_and(|q| q.0 == base && q.2 == of))
                .cloned()
                .collect();
            more.sort_by_key(|p| split_part(&p.name).map(|q| q.1));
            f.size += more.iter().map(|p| p.size).sum::<i64>();
            f.more = more;
        }
    }
    out
}

fn safe_file(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".gguf") || n.ends_with(".safetensors")
}

#[derive(Deserialize)]
struct SearchQuery {
    source: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    kind: String,
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn search(State(s): State<Server>, h: HeaderMap, Query(q): Query<SearchQuery>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    let kind = if q.kind.is_empty() { "text".to_string() } else { q.kind.clone() };
    // Models for working with data are text models (ones that think and call tools).
    let kind = if kind == "data" { "text".to_string() } else { kind };
    let hits = match q.source.as_str() {
        "hf" => search_hf(&a, &q.q, &kind).await,
        "civitai" => search_civitai(&a, &q.q, &kind).await,
        _ => return err(StatusCode::BAD_REQUEST, "bad_source", "look on hf or civitai"),
    };
    match hits {
        Ok(h) => Json(h).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "search", &e),
    }
}

async fn search_hf(a: &Settings, q: &str, kind: &str) -> Result<Vec<Hit>, String> {
    let filter = match kind {
        "text" | "vision" => "gguf",
        "video" => "diffusers&pipeline_tag=text-to-video",
        _ => "diffusers&pipeline_tag=text-to-image",
    };
    let url = format!("{}/api/models?search={}&filter={filter}&sort=downloads&limit=20&full=true", a.hf(), enc(q));
    let v = get_json(url, Some(a.hf_token.clone())).await?;
    Ok(v.as_array()
        .map(|list| {
            list.iter()
                .map(|m| {
                    let tags: Vec<&str> =
                        m["tags"].as_array().map(|t| t.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                    Hit {
                        source: "hf".into(),
                        id: m["id"].as_str().unwrap_or_default().into(),
                        name: m["id"].as_str().unwrap_or_default().into(),
                        kind: kind.into(),
                        license: tags.iter().find_map(|t| t.strip_prefix("license:")).unwrap_or_default().into(),
                        gated: m["gated"].as_bool().unwrap_or(false) || m["gated"].is_string(),
                        adult: false,
                        base_model: String::new(),
                        downloads: m["downloads"].as_i64().unwrap_or(0),
                    }
                })
                .collect()
        })
        .unwrap_or_default())
}

fn civitai_kind(t: &str) -> &'static str {
    if t.eq_ignore_ascii_case("lora") || t.eq_ignore_ascii_case("locon") { "lora" } else { "image" }
}

fn civitai_license(m: &Value) -> String {
    let commercial = m["allowCommercialUse"]
        .as_array()
        .map(|a| !a.is_empty())
        .or_else(|| m["allowCommercialUse"].as_str().map(|s| s != "None"));
    match commercial {
        Some(true) => "Civitai: commercial use allowed".into(),
        Some(false) => "Civitai: no commercial use".into(),
        None => "see Civitai".into(),
    }
}

async fn search_civitai(a: &Settings, q: &str, kind: &str) -> Result<Vec<Hit>, String> {
    let types = if kind == "lora" { "LORA" } else { "Checkpoint" };
    let mut url = format!("{}/api/v1/models?limit=20&types={types}&query={}", a.civitai(), enc(q));
    if !a.adult {
        url.push_str("&nsfw=false");
    }
    let v = get_json(url, Some(a.civitai_token.clone())).await?;
    Ok(v["items"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|m| a.adult || !m["nsfw"].as_bool().unwrap_or(false))
                .map(|m| Hit {
                    source: "civitai".into(),
                    id: m["id"].as_i64().map(|i| i.to_string()).unwrap_or_default(),
                    name: m["name"].as_str().unwrap_or_default().into(),
                    kind: civitai_kind(m["type"].as_str().unwrap_or_default()).into(),
                    license: civitai_license(m),
                    gated: false,
                    adult: m["nsfw"].as_bool().unwrap_or(false),
                    base_model: m["modelVersions"][0]["baseModel"].as_str().unwrap_or_default().into(),
                    downloads: m["stats"]["downloadCount"].as_i64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Deserialize)]
struct FilesQuery {
    source: String,
    id: String,
}

async fn files(State(s): State<Server>, h: HeaderMap, Query(q): Query<FilesQuery>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    match files_of(&a, &q.source, &q.id).await {
        Ok(f) => Json(f).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "files", &e),
    }
}

async fn files_of(a: &Settings, source: &str, id: &str) -> Result<Vec<ModelFile>, String> {
    if source == "hf" {
        if !id.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c))) {
            return Err("that isn't a Hugging Face model name".into());
        }
        let v = get_json(format!("{}/api/models/{id}?blobs=true", a.hf()), Some(a.hf_token.clone())).await?;
        return Ok(v["siblings"]
            .as_array()
            .map(|s| {
                s.iter()
                    .filter_map(|f| {
                        let name = f["rfilename"].as_str()?;
                        let n = name.to_lowercase();
                        (n.ends_with(".gguf")
                            || n.ends_with(".safetensors")
                            || n.ends_with(".ckpt")
                            || n.ends_with(".bin"))
                        .then(|| ModelFile {
                            name: name.into(),
                            size: f["size"].as_i64().unwrap_or(0),
                            url: format!("{}/{id}/resolve/main/{name}", a.hf()),
                            sha256: f["lfs"]["sha256"].as_str().unwrap_or_default().into(),
                            safe: safe_file(name),
                            version: String::new(),
                            base_model: String::new(),
                            more: Vec::new(),
                        })
                    })
                    .collect()
            })
            .map(join_parts)
            .unwrap_or_default());
    }
    if source == "civitai" {
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return Err("that isn't a Civitai model number".into());
        }
        let v = get_json(format!("{}/api/v1/models/{id}", a.civitai()), Some(a.civitai_token.clone())).await?;
        let mut out = Vec::new();
        for ver in v["modelVersions"].as_array().into_iter().flatten() {
            for f in ver["files"].as_array().into_iter().flatten() {
                let name = f["name"].as_str().unwrap_or_default();
                let format = f["metadata"]["format"].as_str().unwrap_or_default();
                out.push(ModelFile {
                    name: name.into(),
                    size: (f["sizeKB"].as_f64().unwrap_or(0.0) * 1024.0) as i64,
                    url: f["downloadUrl"].as_str().unwrap_or_default().into(),
                    sha256: f["hashes"]["SHA256"].as_str().unwrap_or_default().to_lowercase(),
                    safe: safe_file(name) && (format.is_empty() || format == "SafeTensor" || format == "GGUF"),
                    version: ver["name"].as_str().unwrap_or_default().into(),
                    base_model: ver["baseModel"].as_str().unwrap_or_default().into(),
                    more: Vec::new(),
                });
            }
        }
        return Ok(out);
    }
    Err("look on hf or civitai".into())
}

// ---- Installing ------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    pub total: i64,
    pub done: i64,
    /// "downloading", "checking", "loading", "installed", "failed", "stopped".
    pub state: String,
    #[serde(default)]
    pub message: String,
    #[serde(skip)]
    pub stop: Arc<std::sync::atomic::AtomicBool>,
}

/// Installs going on (kept in memory: a restart forgets finished ones, and
/// an unfinished download carries on from its `.part` file next time).
#[derive(Default)]
pub struct Jobs {
    list: Mutex<HashMap<String, Job>>,
}

impl Jobs {
    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) {
        if let Some(j) = self.list.lock().unwrap().get_mut(id) {
            f(j);
        }
    }
}

#[derive(Deserialize)]
struct Install {
    source: String,
    id: String,
    /// The file to install; empty for a whole Hugging Face diffusers model.
    #[serde(default)]
    file: String,
    /// What the model is called in the runner (and in the apps).
    #[serde(default)]
    name: String,
    /// "text", "image" or "lora".
    kind: String,
}

/// A name the runner (and the file system) accepts.
fn model_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c.to_ascii_lowercase() } else { '-' })
        .collect();
    n.trim_matches(|c| c == '-' || c == '.').chars().take(64).collect()
}

/// The diffusers pipeline for a Civitai base model.
fn pipeline(base: &str) -> &'static str {
    let b = base.to_lowercase();
    if b.contains("flux") {
        "FluxPipeline"
    } else if b.contains("xl") || b.contains("pony") || b.contains("illustrious") || b.contains("noob") {
        "StableDiffusionXLPipeline"
    } else {
        "StableDiffusionPipeline"
    }
}

/// LocalAI's settings for a model whose file is `file` in its models folder.
pub fn localai_config(name: &str, file: &str, kind: &str, base_model: &str) -> String {
    match kind {
        "image" => format!(
            "name: {name}\nbackend: diffusers\nf16: true\nparameters:\n  model: {file}\ndiffusers:\n  pipeline_type: {}\n  cuda: true\n",
            pipeline(base_model)
        ),
        _ => format!("name: {name}\nbackend: llama-cpp\ncontext_size: 8192\nparameters:\n  model: {file}\n"),
    }
}

/// What picture styles can be made from: the picture models whose settings
/// are in the models folder, and the LoRA files installed (`loras/`).
async fn style_parts(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    let dir = std::path::Path::new(&a.models_dir);
    let mut bases: Vec<String> = a
        .offered
        .image
        .iter()
        .chain(std::iter::once(&a.image_model))
        .filter(|m| !m.is_empty() && !a.offered.lora.contains(m) && dir.join(format!("{m}.yaml")).is_file())
        .cloned()
        .collect();
    bases.sort();
    bases.dedup();
    let mut loras: Vec<String> = std::fs::read_dir(dir.join("loras"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".safetensors"))
        .collect();
    loras.sort();
    Json(json!({ "bases": bases, "loras": loras })).into_response()
}

#[derive(Deserialize)]
pub struct Style {
    pub name: String,
    /// The picture model it's made from.
    pub base: String,
    /// A file in the models folder's `loras/`.
    pub lora: String,
    /// How strongly the LoRA shows, 0.1–2 (1 = as made).
    #[serde(default = "one")]
    pub strength: f32,
}

fn one() -> f32 {
    1.0
}

/// A picture model's settings with a LoRA added, under another name.
/// LocalAI's diffusers runner takes LoRAs from a model's settings
/// (`lora_adapters`, `lora_scales`), not per request.
pub fn style_config(base_yaml: &str, name: &str, lora: &str, strength: f32) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in base_yaml.lines() {
        // The base's own LoRAs (a style made from a style) are replaced.
        if line.starts_with("lora_adapters:") || line.starts_with("lora_scales:") {
            skip = true;
            continue;
        }
        if skip && line.starts_with("  - ") {
            continue;
        }
        skip = false;
        if line.starts_with("name:") {
            out.push_str(&format!("name: {name}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!("lora_adapters:\n  - loras/{lora}\nlora_scales:\n  - {strength}\n"));
    out
}

/// Makes a picture style (a picture model with a LoRA) and offers it.
async fn make_style(State(s): State<Server>, h: HeaderMap, Json(st): Json<Style>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let mut a = match settings_to_change(&c).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if a.models_dir.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "no_models_dir",
            "say where the runner keeps its models (its models folder) first",
        );
    }
    let name = model_name(&st.name);
    let plain = |f: &str| !f.is_empty() && !f.contains(['/', '\\']) && !f.starts_with('.');
    if name.is_empty() || !plain(&st.base) || !plain(&st.lora) || !st.lora.ends_with(".safetensors") {
        return err(StatusCode::BAD_REQUEST, "bad_request", "give the style a name, a picture model and a LoRA");
    }
    if !(0.1..=2.0).contains(&st.strength) {
        return err(StatusCode::BAD_REQUEST, "bad_strength", "the strength goes from 0.1 to 2");
    }
    let dir = std::path::PathBuf::from(&a.models_dir);
    let Ok(base) = std::fs::read_to_string(dir.join(format!("{}.yaml", st.base))) else {
        return err(StatusCode::NOT_FOUND, "no_base", "that picture model's settings aren't in the models folder");
    };
    if !dir.join("loras").join(&st.lora).is_file() {
        return err(StatusCode::NOT_FOUND, "no_lora", "that LoRA isn't installed");
    }
    if name == st.base || dir.join(format!("{name}.yaml")).is_file() && !a.offered.lora.contains(&name) {
        return err(StatusCode::CONFLICT, "taken", "a model already has that name");
    }
    let yaml = dir.join(format!("{name}.yaml"));
    if std::fs::write(&yaml, style_config(&base, &name, &st.lora, st.strength)).is_err() {
        return failed();
    }
    for list in [&mut a.offered.lora, &mut a.offered.image] {
        if !list.contains(&name) {
            list.push(name.clone());
        }
    }
    if db::set_setting(&c, "ai", &serde_json::to_string(&a).unwrap()).await.is_err() {
        return failed();
    }
    let (base_url, key) = (a.base(), a.key.clone());
    let (apply_name, apply_yaml) = (name.clone(), yaml.clone());
    tokio::task::spawn_blocking(move || {
        let body = json!({ "config_url": format!("file://{}", apply_yaml.display()), "name": apply_name });
        let mut req =
            agent(Some(Duration::from_secs(30))).post(&format!("{}/models/apply", base_url.trim_end_matches("/v1")));
        if !key.is_empty() {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let _ = req.header("Content-Type", "application/json").send(body.to_string());
    });
    Json(masked(&a)).into_response()
}

async fn install(State(s): State<Server>, h: HeaderMap, Json(i): Json<Install>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = settings(&c).await;
    drop(c);
    // A whole diffusers model from Hugging Face (pictures, video): LocalAI
    // fetches it by name; we only write its settings.
    if i.file.is_empty() && i.source == "hf" && (i.kind == "image" || i.kind == "video") {
        return install_repo(&s, &a, &i).await;
    }
    let files = match files_of(&a, &i.source, &i.id).await {
        Ok(f) => f,
        Err(e) => return err(StatusCode::BAD_GATEWAY, "files", &e),
    };
    let Some(file) = files.into_iter().find(|f| f.name == i.file) else {
        return err(StatusCode::NOT_FOUND, "no_file", "that file isn't in this model");
    };
    if !file.safe {
        return err(
            StatusCode::BAD_REQUEST,
            "unsafe",
            "only SafeTensors and GGUF files are installed: older checkpoint files can run programs on this computer",
        );
    }
    let name =
        model_name(if i.name.is_empty() { file.name.rsplit_once('.').map_or(&file.name, |x| x.0) } else { &i.name });
    let job = Job {
        id: crate::random(9),
        name: name.clone(),
        file: file.name.clone(),
        kind: i.kind.clone(),
        total: file.size,
        done: 0,
        state: "downloading".into(),
        message: String::new(),
        stop: Arc::default(),
    };
    // Ollama fetches Hugging Face GGUF models itself.
    if a.runner == Runner::Ollama {
        if i.source != "hf" || i.kind != "text" {
            return err(
                StatusCode::BAD_REQUEST,
                "runner",
                "Ollama runs text models from Hugging Face; pictures need LocalAI",
            );
        }
        let quant = file.name.rsplit_once('.').map_or("", |x| x.0).rsplit(['-', '.']).next().unwrap_or("").to_string();
        let model = format!("hf.co/{}:{quant}", i.id);
        s.jobs.list.lock().unwrap().insert(job.id.clone(), job.clone());
        let (jobs, id, base) = (s.jobs.clone(), job.id.clone(), a.base().trim_end_matches("/v1").to_string());
        tokio::task::spawn_blocking(move || ollama_pull(&jobs, &id, &base, &model));
        return Json(job).into_response();
    }
    if a.models_dir.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "no_models_dir",
            "say where the runner keeps its models (its models folder) first",
        );
    }
    let dir = std::path::PathBuf::from(&a.models_dir);
    if !dir.is_dir() {
        return err(
            StatusCode::BAD_REQUEST,
            "no_models_dir",
            "the runner's models folder isn't there on this computer",
        );
    }
    s.jobs.list.lock().unwrap().insert(job.id.clone(), job.clone());
    let token = match i.source.as_str() {
        "hf" => a.hf_token.clone(),
        _ => a.civitai_token.clone(),
    };
    let (jobs, id) = (s.jobs.clone(), job.id.clone());
    let base = a.base();
    let key = a.key.clone();
    tokio::task::spawn_blocking(move || {
        let r = std::iter::once(&file)
            .chain(file.more.iter())
            .try_fold((None, 0), |(first, offset), f| {
                let path = download(&jobs, &id, f, &token, &dir, offset)?;
                Ok::<_, String>((first.or(Some(path)), offset + f.size))
            })
            .map(|(p, _)| p.expect("at least one file"))
            .and_then(|path| {
                jobs.update(&id, |j| j.state = "loading".into());
                let local = path.file_name().unwrap().to_string_lossy().to_string();
                let sub = if i.kind == "lora" { format!("loras/{local}") } else { local };
                if i.kind == "lora" {
                    // LoRAs sit beside the models; pictures name them per request.
                    return Ok("installed".to_string());
                }
                let yaml = dir.join(format!("{name}.yaml"));
                std::fs::write(&yaml, localai_config(&name, &sub, &i.kind, &file.base_model))
                    .map_err(|e| e.to_string())?;
                // Ask LocalAI to load it now (otherwise it does on its next start).
                let body = json!({ "config_url": format!("file://{}", yaml.display()), "name": name });
                let mut req = agent(Some(Duration::from_secs(30)))
                    .post(&format!("{}/models/apply", base.trim_end_matches("/v1")));
                if !key.is_empty() {
                    req = req.header("Authorization", &format!("Bearer {key}"));
                }
                let loaded = req
                    .header("Content-Type", "application/json")
                    .send(body.to_string())
                    .is_ok_and(|r| r.status().as_u16() < 300);
                Ok(if loaded { "installed".into() } else { "installed: restart LocalAI if it doesn't list it".into() })
            });
        match r {
            Ok(msg) => jobs.update(&id, |j| {
                j.state = "installed".into();
                j.message = msg;
            }),
            Err(e) => jobs.update(&id, |j| {
                if j.state != "stopped" {
                    j.state = "failed".into();
                }
                j.message = e;
            }),
        }
    });
    Json(job).into_response()
}

async fn install_repo(s: &Server, a: &Settings, i: &Install) -> Response {
    if !i.id.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c))) {
        return err(StatusCode::BAD_REQUEST, "bad_id", "that isn't a Hugging Face model name");
    }
    if a.models_dir.is_empty() || !std::path::Path::new(&a.models_dir).is_dir() {
        return err(
            StatusCode::BAD_REQUEST,
            "no_models_dir",
            "say where the runner keeps its models (its models folder) first",
        );
    }
    let info = match get_json(format!("{}/api/models/{}", a.hf(), i.id), Some(a.hf_token.clone())).await {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_GATEWAY, "files", &e),
    };
    let pipeline = info["tags"]
        .as_array()
        .and_then(|t| t.iter().filter_map(|x| x.as_str()).find_map(|x| x.strip_prefix("diffusers:")))
        .unwrap_or(if i.kind == "video" { "DiffusionPipeline" } else { "StableDiffusionXLPipeline" })
        .to_string();
    if !pipeline.chars().all(|c| c.is_ascii_alphanumeric()) {
        return err(StatusCode::BAD_GATEWAY, "files", "Hugging Face gave a pipeline name that can't be used");
    }
    let name = model_name(if i.name.is_empty() { i.id.rsplit('/').next().unwrap_or(&i.id) } else { &i.name });
    let yaml = std::path::Path::new(&a.models_dir).join(format!("{name}.yaml"));
    let config = format!(
        "name: {name}\nbackend: diffusers\nf16: true\nparameters:\n  model: {}\ndiffusers:\n  pipeline_type: {pipeline}\n  cuda: true\n",
        i.id
    );
    if std::fs::write(&yaml, config).is_err() {
        return failed();
    }
    let job = Job {
        id: crate::random(9),
        name: name.clone(),
        file: i.id.clone(),
        kind: i.kind.clone(),
        total: 0,
        done: 0,
        state: "installed".into(),
        message: "LocalAI downloads it the first time it's used".into(),
        stop: Arc::default(),
    };
    s.jobs.list.lock().unwrap().insert(job.id.clone(), job.clone());
    let (base, key) = (a.base(), a.key.clone());
    tokio::task::spawn_blocking(move || {
        let body = json!({ "config_url": format!("file://{}", yaml.display()), "name": name });
        let mut req =
            agent(Some(Duration::from_secs(30))).post(&format!("{}/models/apply", base.trim_end_matches("/v1")));
        if !key.is_empty() {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let _ = req.header("Content-Type", "application/json").send(body.to_string());
    });
    Json(job).into_response()
}

/// Downloads `file` into `dir` (LoRAs into `dir/loras`), carrying on from a
/// `.part` file, and checks its SHA-256. Returns where it is.
fn download(
    jobs: &Jobs,
    id: &str,
    file: &ModelFile,
    token: &str,
    dir: &std::path::Path,
    // Bytes of earlier parts, for the progress shown.
    offset: i64,
) -> Result<std::path::PathBuf, String> {
    use sha2::{Digest, Sha256};
    let kind = jobs.list.lock().unwrap().get(id).map(|j| j.kind.clone()).unwrap_or_default();
    let dir = if kind == "lora" { dir.join("loras") } else { dir.to_path_buf() };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let local = file.name.rsplit('/').next().unwrap_or(&file.name).to_string();
    if local.starts_with('.') || local.contains("..") || !safe_file(&local) {
        return Err("that file name can't be used".into());
    }
    let path = dir.join(&local);
    let part = dir.join(format!("{local}.part"));
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let mut req = agent(None).get(&file.url);
    if !token.is_empty() {
        req = req.header("Authorization", &format!("Bearer {token}"));
    }
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| format!("can't download: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err("the site wants you signed in for this model: add your key under AI".into());
    }
    if status != 200 && status != 206 {
        return Err(format!("the download was refused ({status})"));
    }
    let resumed = status == 206;
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(true)
        .truncate(!resumed)
        .open(&part)
        .map_err(|e| e.to_string())?;
    let mut done = if resumed { have as i64 } else { 0 };
    let stop = jobs.list.lock().unwrap().get(id).map(|j| j.stop.clone()).unwrap_or_default();
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            jobs.update(id, |j| j.state = "stopped".into());
            return Err("stopped".into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("the download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as i64;
        jobs.update(id, |j| j.done = offset + done);
    }
    drop(out);
    if !file.sha256.is_empty() {
        jobs.update(id, |j| j.state = "checking".into());
        let mut h = Sha256::new();
        let mut f = std::fs::File::open(&part).map_err(|e| e.to_string())?;
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != file.sha256.to_lowercase() {
            let _ = std::fs::remove_file(&part);
            return Err("the downloaded file didn't match its checksum, so it was thrown away".into());
        }
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn ollama_pull(jobs: &Jobs, id: &str, base: &str, model: &str) {
    let r = agent(None)
        .post(&format!("{base}/api/pull"))
        .header("Content-Type", "application/json")
        .send(json!({ "model": model, "stream": true }).to_string());
    let resp = match r {
        Ok(r) if r.status().as_u16() == 200 => r,
        Ok(r) => {
            return jobs.update(id, |j| {
                j.state = "failed".into();
                j.message = format!("Ollama said {}", r.status().as_u16());
            });
        }
        Err(e) => {
            return jobs.update(id, |j| {
                j.state = "failed".into();
                j.message = format!("can't reach Ollama: {e}");
            });
        }
    };
    let reader = std::io::BufReader::new(resp.into_body().into_reader());
    use std::io::BufRead;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(e) = v["error"].as_str() {
            return jobs.update(id, |j| {
                j.state = "failed".into();
                j.message = e.into();
            });
        }
        jobs.update(id, |j| {
            if let (Some(t), Some(c)) = (v["total"].as_i64(), v["completed"].as_i64()) {
                j.total = t;
                j.done = c;
            }
            j.message = v["status"].as_str().unwrap_or_default().into();
        });
    }
    jobs.update(id, |j| {
        if j.state != "failed" {
            j.state = "installed".into();
        }
    });
}

async fn jobs(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let list: Vec<Job> = s.jobs.list.lock().unwrap().values().cloned().collect();
    Json(list).into_response()
}

async fn cancel(State(s): State<Server>, h: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let found = s
        .jobs
        .list
        .lock()
        .unwrap()
        .get(&id)
        .map(|j| j.stop.store(true, std::sync::atomic::Ordering::Relaxed))
        .is_some();
    if found {
        StatusCode::NO_CONTENT.into_response()
    } else {
        err(StatusCode::NOT_FOUND, "no_job", "there's no such download")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_settings_from_a_newer_server_still_read() {
        let a: Settings = serde_json::from_str(r#"{"runner":"vllm_cluster","url":"http://x/v1","text_model":"m","new":1}"#).unwrap();
        assert_eq!((a.runner, a.url.as_str(), a.text_model.as_str()), (Runner::Openai, "http://x/v1", "m"));
        for r in [Runner::Localai, Runner::Ollama, Runner::Openai] {
            assert_eq!(serde_json::from_value::<Runner>(serde_json::to_value(r).unwrap()).unwrap(), r);
        }
    }

    #[test]
    fn only_the_fields_the_apps_use_go_on() {
        let req = json!({
            "model": "qwen", "messages": [{ "role": "user", "content": "hi" }], "stream": true, "temperature": 9,
            "max_tokens": 10_000_000, "provider": { "data_collection": "allow" }, "models": ["x/expensive"],
            "plugins": [{ "id": "web" }], "reasoning": { "effort": "high" }, "usage": { "include": true, "extra": 1 },
            "tools": [], "tool_choice": "auto",
        });
        let out = clean_request(&req, "text").unwrap();
        for gone in ["provider", "models", "plugins", "reasoning"] {
            assert!(out.get(gone).is_none(), "{gone}: {out}");
        }
        assert_eq!((out["temperature"].clone(), out["max_tokens"].clone()), (json!(2.0), json!(32768)));
        assert_eq!((out["stream"].clone(), out["usage"].clone()), (json!(true), json!({ "include": true })));
        assert_eq!(out["tool_choice"], "auto");
        // Pictures: in the request, never a link the runner would fetch.
        let pic = |url: &str| json!({ "messages": [{ "role": "user", "content": [{ "type": "image_url", "image_url": { "url": url } }] }] });
        assert!(clean_request(&pic("data:image/png;base64,AAAA"), "text").is_ok());
        assert!(clean_request(&pic("http://169.254.169.254/latest/meta-data"), "text").is_err());
        assert!(clean_request(&json!({ "messages": [{ "role": "user", "content": [{ "type": "video_url", "video_url": "file:///etc/passwd" }] }] }), "text").is_err());
        assert!(clean_request(&json!({ "model": "x" }), "text").is_err(), "messages are needed");
        // Pictures: a few at a time, and only their own fields.
        let out = clean_request(&json!({ "prompt": "a cat", "n": 50, "size": "1024x1024", "provider": {} }), "image").unwrap();
        assert_eq!(out, json!({ "model": "", "prompt": "a cat", "n": 4, "size": "1024x1024" }));
        let out = clean_request(&json!({ "prompt": "a boat", "start_image": "http://x/y.png" }), "video").unwrap();
        assert_eq!(out, json!({ "model": "", "prompt": "a boat" }));
    }

    #[test]
    fn only_offered_models() {
        let a = Settings {
            text_model: "qwen".into(),
            vision_model: "eyes".into(),
            image_model: "flux".into(),
            offered: Offered { text: vec!["qwen-big".into()], lora: vec!["ink".into()], ..Default::default() },
            ..Default::default()
        };
        let text = offered_for(&a, "text");
        for m in ["qwen", "qwen-big", "eyes", "flux", "ink"] {
            assert!(text.contains(&m.to_string()), "{m}");
        }
        assert_eq!(offered_for(&a, "image"), ["ink", "flux"]);
        assert!(offered_for(&a, "video").is_empty());
        assert_eq!(offered_for(&a, "data"), ["qwen-big", "qwen"]);
    }

    #[test]
    fn limits_hold_and_places_come_back() {
        let l = Limits { pictures_at_once: 2, clips_at_once: 1, server_at_once: 0, media_per_hour: 5, text_per_hour: 2 };
        let user = -4242;
        let a = take_slot(&l, user, "image").unwrap();
        let b = take_slot(&l, user, "image").unwrap();
        assert!(take_slot(&l, user, "image").is_err(), "two pictures at a time");
        let clip = take_slot(&l, user, "video").unwrap();
        assert!(take_slot(&l, user, "video").is_err(), "one clip at a time");
        drop(clip);
        drop(a);
        let c = take_slot(&l, user, "image").unwrap();
        let d = take_slot(&l, user, "video").unwrap();
        drop((b, c, d));
        assert!(take_slot(&l, user, "image").is_err(), "five an hour");
        assert!(take_slot(&l, user, "text").is_ok() && take_slot(&l, user, "text").is_ok());
        assert!(take_slot(&l, user, "text").is_err(), "two questions an hour");
        assert!(take_slot(&l, user - 1, "text").is_ok(), "each person has their own");
    }

    #[test]
    fn secrets_follow_their_host() {
        assert_eq!(origin("https://OpenRouter.ai/api/v1"), "https://openrouter.ai");
        assert_eq!(origin("http://localhost:8080/v1"), origin("http://localhost:8080/v1/"));
        assert_ne!(origin("http://localhost:8080/v1"), origin("http://localhost:8081/v1"));
        assert_ne!(origin("https://openrouter.ai/api"), origin("https://openrouter.ai.evil.example/api"));
        assert_eq!(origin("https://user:pw@hf.co/x"), "https://hf.co");
    }

    #[test]
    fn models_folder_inside_the_data_folder() {
        let data = std::env::temp_dir().join(format!("xenify-data-{}", std::process::id()));
        std::fs::create_dir_all(data.join("models")).unwrap();
        let p = |x: &str| data.join(x).to_string_lossy().to_string();
        assert!(inside(&p("models"), &data));
        assert!(inside(&p("models/new"), &data));
        assert!(!inside(&p("models/../../etc"), &data));
        assert!(!inside("/etc", &data));
        assert!(!inside("models", &data), "relative");
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink("/etc", data.join("out"));
            assert!(!inside(&p("out/x"), &data), "a link leading out");
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn openrouter_pictures_by_shape() {
        let mut r = json!({ "prompt": "a cat", "size": "1024x1024", "n": 1, "response_format": "b64_json" });
        openrouter_picture(&mut r);
        assert_eq!(r, json!({ "prompt": "a cat", "n": 1, "aspect_ratio": "1:1" }));
        let mut r = json!({ "size": "1920x1080" });
        openrouter_picture(&mut r);
        assert_eq!(r["aspect_ratio"], "16:9");
    }

    #[test]
    fn kinds_from_the_model_list() {
        let m = |i: &[&str], o: &[&str], p: &[&str]| {
            json!({ "id": "x", "architecture": { "input_modalities": i, "output_modalities": o }, "supported_parameters": p })
        };
        assert_eq!(kinds_of(&m(&["text"], &["text"], &[])), Some(vec!["text"]));
        assert_eq!(kinds_of(&m(&["text", "image"], &["text"], &["tools"])), Some(vec!["text", "data", "vision"]));
        assert_eq!(kinds_of(&m(&["text"], &["image"], &[])), Some(vec!["image"]));
        assert_eq!(kinds_of(&m(&["text", "image"], &["video"], &[])), Some(vec!["video"]));
        assert_eq!(kinds_of(&m(&["text", "video"], &["video"], &[])), Some(vec![]));
        assert_eq!(kinds_of(&m(&["text"], &["embeddings"], &[])), Some(vec![]));
        assert_eq!(kinds_of(&json!({ "id": "llama3" })), None);
    }

    fn f(name: &str, size: i64) -> ModelFile {
        ModelFile {
            name: name.into(),
            size,
            url: String::new(),
            sha256: String::new(),
            safe: true,
            version: String::new(),
            base_model: String::new(),
            more: Vec::new(),
        }
    }

    #[test]
    fn split_models_install_as_one() {
        let out =
            join_parts(vec![f("big-00002-of-00002.gguf", 5), f("big-00001-of-00002.gguf", 10), f("small.gguf", 3)]);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].name.as_str(), out[0].size, out[0].more.len()), ("big-00001-of-00002.gguf", 15, 1));
        assert_eq!(out[0].more[0].name, "big-00002-of-00002.gguf");
        assert!(out[1].more.is_empty());
    }
}
