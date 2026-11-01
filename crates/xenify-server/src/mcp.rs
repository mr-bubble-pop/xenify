//! MCP tool servers an admin adds for everyone (by address: Streamable
//! HTTP), which Calc's Ask can use. The apps send their JSON-RPC messages
//! here and the server passes them on, with any key the tool server needs
//! (kept here, never sent to the apps).
//!
//! - Using them needs the "ai.tools" permission. Only the messages Calc
//!   sends go through (starting, listing and calling tools, ping), and
//!   each person has their own session with each tool server.
//! - Unless the admin lets a tool server's tools change things, only the
//!   tools that say they only read are listed and can be called.

use crate::{Server, admin_power, db, err, failed, signed_in, unavailable};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/mcp", get(list))
        .route("/v1/mcp/{id}", post(pass_on))
        .route("/v1/admin/mcp", get(admin_list).post(admin_save))
        .route("/v1/admin/mcp/{id}", axum::routing::delete(admin_delete))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tool {
    pub id: u32,
    pub name: String,
    pub url: String,
    /// Sent with every message (a key, say): kept here only.
    pub headers: HashMap<String, String>,
    /// Tools that change things may be called too (otherwise only those
    /// that say they only read).
    pub changes: bool,
}

/// The messages the apps send a tool server.
const METHODS: &[&str] = &["initialize", "notifications/initialized", "ping", "tools/list", "tools/call"];

/// Whether a tool (from `tools/list`) says it only reads.
fn reads_only(t: &Value) -> bool {
    t["annotations"]["readOnlyHint"].as_bool() == Some(true)
}

/// A `tools/list` answer with only the tools that may be called.
pub fn only_allowed(mut answer: Value, changes: bool) -> Value {
    if !changes && let Some(list) = answer["result"]["tools"].as_array_mut() {
        list.retain(reads_only);
    }
    answer
}

async fn tools(c: &deadpool_postgres::Client) -> Vec<Tool> {
    db::setting(c, "mcp").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// Sessions the tool servers gave, by person and tool id (kept while the
/// server runs).
static SESSIONS: Mutex<Option<HashMap<(i64, u32), String>>> = Mutex::new(None);

/// The tools each tool server listed, by tool id, and whether each said it
/// only reads, for checking calls.
static READ_ONLY: Mutex<Option<HashMap<u32, HashMap<String, bool>>>> = Mutex::new(None);

fn forget(id: u32) {
    if let Ok(mut m) = SESSIONS.lock() {
        m.get_or_insert_with(HashMap::new).retain(|k, _| k.1 != id);
    }
    if let Ok(mut m) = READ_ONLY.lock() {
        m.get_or_insert_with(HashMap::new).remove(&id);
    }
}

async fn list(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if let Ok(c) = s.pool().get().await
        && !crate::perms::can(&c, me.id, "ai.tools").await
    {
        return Json(json!([])).into_response();
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    Json(tools(&c).await.iter().map(|t| json!({ "id": t.id, "name": t.name })).collect::<Vec<_>>()).into_response()
}

/// Sends one JSON-RPC message to tool `t` in `user`'s session: its answer
/// (blocking).
fn send(t: &Tool, user: i64, body: &Value) -> Result<Value, String> {
    let key = (user, t.id);
    let session = SESSIONS.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(&key).cloned()));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req =
        agent.post(&t.url).header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream");
    for (k, v) in &t.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(sid) = &session {
        req = req.header("Mcp-Session-Id", sid.as_str());
    }
    let mut r = req.send(body.to_string()).map_err(|e| format!("couldn't reach {}: {e}", t.name))?;
    if let Some(sid) = r.headers().get("mcp-session-id").and_then(|v| v.to_str().ok())
        && let Ok(mut m) = SESSIONS.lock()
    {
        m.get_or_insert_with(HashMap::new).insert(key, sid.to_string());
    }
    let status = r.status().as_u16();
    let ctype = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mut text = String::new();
    r.body_mut().as_reader().take(16 << 20).read_to_string(&mut text).map_err(|e| e.to_string())?;
    if status == 202 {
        return Ok(json!({}));
    }
    if status >= 400 {
        return Err(format!("{} answered {status}", t.name));
    }
    let trimmed = text.trim_start();
    if ctype.contains("event-stream") || trimmed.starts_with("event:") || trimmed.starts_with("data:") {
        return Ok(text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
            .find(|v| v.get("id") == body.get("id"))
            .unwrap_or(Value::Null));
    }
    serde_json::from_str::<Value>(&text).map_err(|_| format!("{} sent something unexpected", t.name))
}

/// Keeps which tools say they only read, from a `tools/list` answer.
fn note_read_only(id: u32, answer: &Value) {
    let Some(list) = answer["result"]["tools"].as_array() else { return };
    if let Ok(mut m) = READ_ONLY.lock() {
        let known = m.get_or_insert_with(HashMap::new).entry(id).or_default();
        for t in list {
            if let Some(name) = t["name"].as_str() {
                known.insert(name.to_string(), reads_only(t));
            }
        }
    }
}

/// Whether tool `name` of tool server `id` said it only reads; None when
/// it hasn't been listed.
fn said_read_only(id: u32, name: &str) -> Option<bool> {
    READ_ONLY.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(&id).and_then(|n| n.get(name).copied())))
}

/// An error answer to a JSON-RPC message, as the tool servers send them.
fn refusal(body: &Value, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": body.get("id").cloned().unwrap_or(Value::Null), "error": { "code": -32601, "message": message } }))
        .into_response()
}

/// One JSON-RPC message to tool server `id`: its answer.
async fn pass_on(State(s): State<Server>, h: HeaderMap, Path(id): Path<u32>, Json(body): Json<Value>) -> Response {
    let me = match crate::perms::need(&s, &h, "ai.tools").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let method = body["method"].as_str().unwrap_or_default().to_string();
    if !METHODS.contains(&method.as_str()) {
        return refusal(&body, "the server passes on only starting, listing and calling tools");
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Some(t) = tools(&c).await.into_iter().find(|t| t.id == id) else {
        return err(StatusCode::NOT_FOUND, "missing", "no such tool");
    };
    drop(c);
    // Only the fields JSON-RPC has go on.
    let mut msg = json!({ "jsonrpc": "2.0", "method": method });
    for k in ["id", "params"] {
        if let Some(v) = body.get(k) {
            msg[k] = v.clone();
        }
    }
    let user = me.id;
    let got = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        if method == "tools/call" && !t.changes {
            let name = msg["params"]["name"].as_str().unwrap_or_default().to_string();
            if said_read_only(t.id, &name).is_none() {
                // Not listed yet (the list may be older than this server's run): ask.
                let list = send(&t, user, &json!({ "jsonrpc": "2.0", "id": "xenify-check", "method": "tools/list", "params": {} }))?;
                note_read_only(t.id, &list);
            }
            if said_read_only(t.id, &name) != Some(true) {
                return Ok(json!({ "jsonrpc": "2.0", "id": msg.get("id"), "error": { "code": -32001,
                    "message": format!("{name} may change things, and this server's admin allows only tools that read") } }));
            }
        }
        let answer = send(&t, user, &msg)?;
        if method == "tools/list" {
            note_read_only(t.id, &answer);
            return Ok(only_allowed(answer, t.changes));
        }
        Ok(answer)
    })
    .await;
    match got {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, "tool", &e),
        Err(_) => failed(),
    }
}

async fn admin_list(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.data").await {
        return e;
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    Json(tools(&c).await.iter().map(|t| json!({ "id": t.id, "name": t.name, "url": t.url, "headers": t.headers.keys().collect::<Vec<_>>(), "changes": t.changes })).collect::<Vec<_>>()).into_response()
}

#[derive(Deserialize)]
struct Save {
    #[serde(default)]
    id: Option<u32>,
    name: String,
    url: String,
    /// New header values (empty: keep the one there, unless the address
    /// moves to another host).
    #[serde(default)]
    headers: HashMap<String, String>,
    /// Tools that change things may be called.
    #[serde(default)]
    changes: bool,
}

async fn admin_save(State(s): State<Server>, h: HeaderMap, Json(b): Json<Save>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.data").await {
        return e;
    }
    if b.name.trim().is_empty() || !(b.url.starts_with("http://") || b.url.starts_with("https://")) {
        return err(
            StatusCode::BAD_REQUEST,
            "bad",
            "a tool needs a name and an address starting with http:// or https://",
        );
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let mut list = tools(&c).await;
    let id = b.id.unwrap_or_else(|| list.iter().map(|t| t.id).max().unwrap_or(0) + 1);
    let old = list.iter().find(|t| t.id == id);
    let url = b.url.trim().to_string();
    // Kept keys go only to the host they were given for.
    let moved = old.is_some_and(|t| crate::ai::origin(&t.url) != crate::ai::origin(&url));
    let kept = old.filter(|_| !moved).map(|t| t.headers.clone()).unwrap_or_default();
    let mut headers = kept;
    for (k, v) in b.headers {
        if !v.is_empty() {
            headers.insert(k, v);
        }
    }
    // Keys that were there and now aren't: to be typed again.
    let cleared: Vec<String> =
        old.map(|t| t.headers.keys().filter(|k| !headers.contains_key(*k)).cloned().collect()).unwrap_or_default();
    list.retain(|t| t.id != id);
    list.push(Tool { id, name: b.name.trim().to_string(), url, headers, changes: b.changes });
    forget(id);
    match db::set_setting(&c, "mcp", &serde_json::to_string(&list).unwrap_or_default()).await {
        Ok(()) => Json(json!({ "id": id, "cleared": cleared })).into_response(),
        Err(_) => failed(),
    }
}

async fn admin_delete(State(s): State<Server>, h: HeaderMap, Path(id): Path<u32>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.data").await {
        return e;
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let mut list = tools(&c).await;
    list.retain(|t| t.id != id);
    forget(id);
    match db::set_setting(&c, "mcp", &serde_json::to_string(&list).unwrap_or_default()).await {
        Ok(()) => Json(json!({})).into_response(),
        Err(_) => failed(),
    }
}
