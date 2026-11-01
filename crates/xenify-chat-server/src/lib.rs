//! The Xenify Chat server, part of the Xenify server (which has the
//! accounts and sessions; chat uses the same tables).
//!
//! - Direct chats, groups and channels in PostgreSQL. Message numbers come
//!   from a row lock, so order holds across servers.
//! - Servers are interchangeable: each listens for PostgreSQL notifications
//!   (LISTEN/NOTIFY) and pushes to the people connected to it. Put several
//!   behind a load balancer.
//! - Message search (PostgreSQL's text search), each person's settings and
//!   reminders, and deleting messages older than the server's or a chat's
//!   limit ([`Server::expire`], run by the Xenify server).

mod ask;
pub mod calls;
pub mod db;
mod things;

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::Pool;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use xenify_chat_proto::{
    AppMode, BOOK, BUSY, CALENDAR, CONTACTS, Kind, MEETING, Presence, Role, SHARED_CAL, ToApp, ToServer, User,
};

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// postgres://user:password@host/db
    pub database: String,
    /// Let app webhooks point at this computer or its network (for tests;
    /// otherwise only public https:// addresses, so nobody can use the
    /// server to reach machines behind it).
    pub local_webhooks: bool,
}

/// One app's connection, as the rest of the server reaches it.
#[derive(Clone)]
struct Conn {
    tx: mpsc::Sender<ToApp>,
    /// It isn't keeping up: it closes.
    full: Arc<tokio::sync::Notify>,
    /// Check the session still works (someone signed out or was disabled).
    check: Arc<tokio::sync::Notify>,
}

type Online = HashMap<i64, Vec<Conn>>;

/// What may wait for one connection before it counts as not keeping up
/// (and is closed: the app connects again and catches up).
const QUEUE: usize = 1024;
/// Messages one person's apps may send this server: this many at once,
/// then this many a second. Far more than anyone chatting sends.
const BURST: f64 = 600.0;
const PER_SECOND: f64 = 60.0;
/// How often an open connection checks its session still works.
pub(crate) const RECHECK: std::time::Duration = std::time::Duration::from_secs(120);

/// A token bucket: whether one more message may go now.
fn take_token(bucket: &mut (f64, std::time::Instant)) -> bool {
    let now = std::time::Instant::now();
    bucket.0 = (bucket.0 + now.duration_since(bucket.1).as_secs_f64() * PER_SECOND).min(BURST);
    bucket.1 = now;
    if bucket.0 >= 1.0 {
        bucket.0 -= 1.0;
        true
    } else {
        false
    }
}

#[derive(Clone)]
pub struct Server {
    pool: Pool,
    config: Config,
    /// People connected to this server: user id → their connections.
    online: Arc<Mutex<Online>>,
    /// This server's name in the presence table (new each start).
    id: Arc<str>,
    /// Calls going on through this server.
    calls: Arc<Mutex<calls::Calls>>,
    /// How many days the server keeps messages (None: for ever); the Xenify
    /// server sets it from Admin › Network › Chat.
    keep_days: Arc<Mutex<Option<i64>>>,
    /// The server's AI, for questions about people's messages (`Ask`).
    assistant: Arc<Mutex<Option<Arc<dyn Assistant>>>>,
    /// How many messages each person connected here may still send now.
    buckets: Arc<Mutex<HashMap<i64, (f64, std::time::Instant)>>>,
}

/// The server's AI, as Chat uses it: whether someone may ask it, and an
/// answer to a conversation (OpenAI-style `messages`). Chat gives it only
/// messages the person asking may read.
pub trait Assistant: Send + Sync {
    fn ready(&self, user: i64) -> futures_util::future::BoxFuture<'_, bool>;
    fn answer(
        &self,
        user: i64,
        messages: Vec<serde_json::Value>,
    ) -> futures_util::future::BoxFuture<'static, Result<String, String>>;
}

pub fn pool(database: &str) -> Result<Pool, String> {
    let cfg: tokio_postgres::Config = database.parse().map_err(|e| format!("database address: {e}"))?;
    let mgr = deadpool_postgres::Manager::new(cfg, tokio_postgres::NoTls);
    Pool::builder(mgr).max_size(32).build().map_err(|e| e.to_string())
}

/// Creates the tables (safe to run every start).
pub async fn migrate(pool: &Pool) -> Result<(), String> {
    let c = pool.get().await.map_err(|e| e.to_string())?;
    // Several servers starting at once: one creates, the others wait.
    c.execute("SELECT pg_advisory_lock(7420426)", &[]).await.map_err(|e| e.to_string())?;
    let r = match c.batch_execute(db::SCHEMA).await {
        Ok(()) => match c.batch_execute(things::SCHEMA).await {
            Ok(()) => c.batch_execute(db::SEARCH_SCHEMA).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        },
        Err(e) => Err(e.to_string()),
    };
    let _ = c.execute("SELECT pg_advisory_unlock(7420426)", &[]).await;
    r
}

impl Server {
    pub async fn start(config: Config) -> Result<Server, String> {
        let pool = pool(&config.database)?;
        migrate(&pool).await?;
        let s = Server {
            pool,
            config,
            online: Arc::default(),
            id: random(12).into(),
            calls: Arc::default(),
            keep_days: Arc::default(),
            assistant: Arc::default(),
            buckets: Arc::default(),
        };
        s.clone().listen().await?;
        // Keep our connections looking fresh to the other servers.
        let beat = s.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                every.tick().await;
                if let Ok(c) = beat.pool.get().await {
                    let _ = db::presence_heartbeat(&c, &beat.id).await;
                }
            }
        });
        Ok(s)
    }

    /// Chat's routes, without signing in or making accounts: the Xenify
    /// server has those.
    pub fn chat_routes(&self) -> Router {
        Router::new()
            .route("/v1/ws", get(ws))
            .route("/v1/call/{id}/media", get(calls::media))
            // For apps (bots, alerts, feeds): post and update messages over plain HTTP.
            .route("/v1/post", post(app_post))
            .route("/v1/edit", post(app_edit))
            // Managing your apps from a script (with a key from Settings › Apps).
            .route(
                "/v1/apps",
                get(|State(s): State<Server>, h: HeaderMap| api_call(s, h, Ok(ToServer::ListApps))).post(
                    |State(s): State<Server>, h: HeaderMap, Json(v): Json<serde_json::Value>| {
                        api_call(s, h, as_msg("create_app", v))
                    },
                ),
            )
            .route(
                "/v1/apps/update",
                post(|State(s): State<Server>, h: HeaderMap, Json(v): Json<serde_json::Value>| {
                    api_call(s, h, as_msg("update_app", v))
                }),
            )
            .route(
                "/v1/apps/key",
                post(|State(s): State<Server>, h: HeaderMap, Json(v): Json<serde_json::Value>| {
                    api_call(s, h, as_msg("new_app_key", v))
                }),
            )
            .route(
                "/v1/apps/delete",
                post(|State(s): State<Server>, h: HeaderMap, Json(v): Json<serde_json::Value>| {
                    api_call(s, h, as_msg("delete_app", v))
                }),
            )
            .with_state(self.clone())
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// How many days the server keeps messages (None: for ever).
    pub fn set_keep_days(&self, days: Option<i64>) {
        *self.keep_days.lock().unwrap() = days.filter(|d| *d > 0);
    }

    pub fn keep_days(&self) -> Option<i64> {
        *self.keep_days.lock().unwrap()
    }

    /// The server's AI, for people's questions about their messages.
    pub fn set_assistant(&self, a: Arc<dyn Assistant>) {
        *self.assistant.lock().unwrap() = Some(a);
    }

    fn assistant(&self) -> Option<Arc<dyn Assistant>> {
        self.assistant.lock().unwrap().clone()
    }

    /// Deletes messages past their limit (see [`db::expire`]) and tells
    /// everyone in those conversations. Returns them, each with the time
    /// before which everything went, so files sent then can go too.
    pub async fn expire(&self) -> Result<Vec<(i64, i64)>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let gone = db::expire(&c, self.keep_days()).await.map_err(|e| e.to_string())?;
        drop(c);
        for (conv, _) in &gone {
            self.notify(format!("c:{conv}")).await;
        }
        Ok(gone)
    }

    /// Reminders now due: shown on their owners' devices, and handed back
    /// (for those with no device connected anywhere, say).
    pub async fn due_reminders(&self) -> Result<Vec<(i64, xenify_chat_proto::Reminder)>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let due = db::due_reminders(&c).await.map_err(|e| e.to_string())?;
        drop(c);
        for (user, r) in &due {
            self.notify(format!("rd:{user}:{}", r.key)).await;
        }
        Ok(due)
    }

    /// Whether `user` has an app connected to any server.
    pub async fn connected(&self, user: i64) -> bool {
        let Ok(c) = self.pool.get().await else { return false };
        matches!(db::presence_of(&c, &[user]).await.as_deref(), Ok([p]) if p.state != Presence::Offline)
    }

    /// Hears what every server did, and tells the people connected here.
    async fn listen(self) -> Result<(), String> {
        let cfg: tokio_postgres::Config =
            self.config.database.parse().map_err(|e: tokio_postgres::Error| e.to_string())?;
        let (client, mut conn) = cfg.connect(tokio_postgres::NoTls).await.map_err(|e| e.to_string())?;
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            let mut stream = futures_util::stream::poll_fn(move |cx| conn.poll_message(cx));
            while let Some(Ok(msg)) = stream.next().await {
                if let tokio_postgres::AsyncMessage::Notification(n) = msg {
                    let _ = tx.send(n.payload().to_string());
                }
            }
        });
        client.batch_execute(&format!("LISTEN {}", db::CHANNEL)).await.map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            let _keep = client;
            while let Some(payload) = rx.recv().await {
                let _ = self.fan_out(&payload).await;
            }
        });
        Ok(())
    }

    /// What happened, as every server hears it:
    /// - `m:conv:id`: a message (new, edited or deleted);
    /// - `c:conv`: the conversation changed (members, title…);
    /// - `p:conv:user`: one person's view of it changed (their settings);
    /// - `x:conv:user`: someone is no longer in it;
    /// - `t:conv:user`: someone is typing;
    /// - `r:conv:user:seq`: someone read up to `seq`;
    /// - `o:user`: someone came online, went offline or changed status;
    /// - `s:user`: someone's settings changed;
    /// - `rd:user:key`, `rg:user:key`: a reminder is due, or done with;
    /// - `k:user`: someone's sessions were deleted or they were disabled
    ///   (see [`Server::sessions_changed`]).
    async fn fan_out(&self, payload: &str) -> Result<(), String> {
        // A reminder's key may have colons in it.
        let parts: Vec<&str> = if payload.starts_with("rd:") || payload.starts_with("rg:") {
            payload.splitn(3, ':').collect()
        } else {
            payload.split(':').collect()
        };
        let num = |i: usize| parts.get(i).and_then(|p| p.parse::<i64>().ok()).unwrap_or(0);
        let conv = num(1);
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let here = |ids: Vec<i64>| -> Vec<i64> {
            let online = self.online.lock().unwrap();
            ids.into_iter().filter(|u| online.contains_key(u)).collect()
        };
        match parts.first().copied() {
            Some("m") => {
                let members = here(db::member_ids(&c, conv).await.map_err(|e| e.to_string())?);
                if let Some(msg) = db::message(&c, num(2)).await.map_err(|e| e.to_string())? {
                    self.tell(&members, ToApp::Message { msg });
                }
            }
            Some("c") => {
                let online = here(db::member_ids(&c, conv).await.map_err(|e| e.to_string())?);
                for (u, conv) in db::conversation_for(&c, conv, &online).await.map_err(|e| e.to_string())? {
                    self.tell(&[u], ToApp::Conversation { conv });
                }
            }
            Some("p") => {
                if let Some(conv) = db::conversation(&c, conv, num(2)).await.map_err(|e| e.to_string())? {
                    self.tell(&[num(2)], ToApp::Conversation { conv });
                }
            }
            Some("x") => {
                self.tell(&[num(2)], ToApp::Removed { conv });
                // Out of the conversation: out of its call.
                self.out_of_call(num(2), conv).await;
            }
            // Someone's sessions changed (signed out, disabled): their
            // connections, and their apps', check theirs still work.
            Some("k") => {
                let who = num(1);
                let mut ids = vec![who];
                if let Ok(rows) = c.query("SELECT user_id FROM apps WHERE owner = $1", &[&who]).await {
                    ids.extend(rows.iter().map(|r| r.get::<_, i64>(0)));
                }
                self.recheck(&ids);
            }
            Some("t") => {
                let who = num(2);
                if let Some(user) = db::user(&c, who).await.map_err(|e| e.to_string())? {
                    let others: Vec<i64> = here(db::member_ids(&c, conv).await.map_err(|e| e.to_string())?)
                        .into_iter()
                        .filter(|u| *u != who)
                        .collect();
                    self.tell(&others, ToApp::Typing { conv, user });
                }
            }
            // Your other devices stop showing it as unread.
            Some("r") => self.tell(&[num(2)], ToApp::Read { conv, seq: num(3) }),
            // A booking made, answered or called off.
            Some("b") => {
                if let Some((to, mine, theirs)) = things::changed(&c, num(1)).await {
                    let booker = mine.user.id;
                    self.tell(&here(vec![booker]), ToApp::Booked { booking: mine, tag: String::new() });
                    let others: Vec<i64> = here(to).into_iter().filter(|u| *u != booker).collect();
                    self.tell(&others, ToApp::Booked { booking: theirs, tag: String::new() });
                }
            }
            Some("s") => {
                let who = num(1);
                if !here(vec![who]).is_empty() {
                    let settings = db::settings(&c, who).await.map_err(|e| e.to_string())?;
                    self.tell(&[who], ToApp::Settings { settings });
                }
            }
            Some("rd") => {
                let key = parts.get(2).copied().unwrap_or("");
                if !here(vec![num(1)]).is_empty()
                    && let Some(reminder) = db::reminder(&c, num(1), key).await.map_err(|e| e.to_string())?
                {
                    self.tell(&[num(1)], ToApp::Reminder { reminder });
                }
            }
            Some("rg") => self.tell(&[num(1)], ToApp::ReminderGone { key: parts.get(2).unwrap_or(&"").to_string() }),
            Some("o") => {
                let who = num(1);
                let list = db::presence_of(&c, &[who]).await.map_err(|e| e.to_string())?;
                let to = here(db::contacts(&c, who).await.map_err(|e| e.to_string())?);
                self.tell(&to, ToApp::Presence { list });
            }
            _ => {}
        }
        Ok(())
    }

    /// Call after deleting someone's sessions or disabling them (on any
    /// server): their open connections, their apps' and their calls' check
    /// their sessions at once, and close if they no longer work. (Open
    /// connections check every few minutes anyway.)
    pub async fn sessions_changed(&self, user: i64) {
        self.notify(format!("k:{user}")).await;
    }

    /// Asks these people's connections here to check their sessions.
    fn recheck(&self, users: &[i64]) {
        {
            let online = self.online.lock().unwrap();
            for u in users {
                for conn in online.get(u).into_iter().flatten() {
                    conn.check.notify_one();
                }
            }
        }
        self.recheck_media(users);
    }

    fn tell(&self, users: &[i64], m: ToApp) {
        let mut online = self.online.lock().unwrap();
        for u in users {
            if let Some(list) = online.get_mut(u) {
                list.retain(|conn| match conn.tx.try_send(m.clone()) {
                    Ok(()) => true,
                    // Not keeping up: it closes (and catches up when it's back).
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        conn.full.notify_one();
                        false
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                });
            }
        }
    }

    pub(crate) async fn notify(&self, payload: String) {
        if let Ok(c) = self.pool.get().await {
            let _ = c.execute("SELECT pg_notify($1, $2)", &[&db::CHANNEL, &payload]).await;
        }
    }
}

fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, Sha256::digest(token.as_bytes()))
}

fn random(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("random");
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, b)
}

fn hash_password(password: &str) -> String {
    use argon2::password_hash::PasswordHasher;
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("random");
    argon2::Argon2::default().hash_password_with_salt(password.as_bytes(), &salt).expect("hash").to_string()
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// A display name: trimmed, single spaces, at most 64 characters.
pub fn clean_name(n: &str) -> String {
    n.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(64).collect()
}

/// User names: 2–32 letters, digits, dots, dashes or underscores.
pub fn valid_username(n: &str) -> bool {
    (2..=32).contains(&n.chars().count()) && n.chars().all(|c| c.is_alphanumeric() || "._-".contains(c))
}

/// The account behind an `Authorization: Bearer` key.
async fn bearer(s: &Server, headers: &HeaderMap) -> Result<User, Box<Response>> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let Ok(c) = s.pool.get().await else {
        return Err(Box::new(err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable")));
    };
    db::session_user(&c, &hash_token(token))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| Box::new(err(StatusCode::UNAUTHORIZED, "unknown key: make one in Chat, Settings › Apps")))
}

fn status_of(code: &str) -> StatusCode {
    match code {
        "not_member" | "not_allowed" | "not_yours" | "not_owner" => StatusCode::FORBIDDEN,
        "no_conversation" | "no_message" => StatusCode::NOT_FOUND,
        "failed" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    }
}

#[derive(serde::Deserialize)]
struct AppPost {
    /// The conversation's number, or…
    #[serde(default)]
    conv: Option<i64>,
    /// …the title of a channel or group the app is in.
    #[serde(default)]
    channel: Option<String>,
    text: String,
    /// Sending again with the same id doesn't post twice.
    #[serde(default)]
    client_id: String,
}

/// `POST /v1/post {"channel": "Build alerts", "text": "…"}` → `{"conv", "seq", "id"}`.
async fn app_post(State(s): State<Server>, headers: HeaderMap, Json(p): Json<AppPost>) -> Response {
    let me = match bearer(&s, &headers).await {
        Ok(u) => u,
        Err(r) => return *r,
    };
    let Ok(mut c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let conv = match (p.conv, p.channel.as_deref().map(str::trim)) {
        (Some(id), _) => id,
        (None, Some(title)) if !title.is_empty() => match c
            .query_opt(
                "SELECT cv.id FROM conversations cv JOIN members m ON m.conv = cv.id AND m.user_id = $1
                 WHERE lower(cv.title) = lower($2) ORDER BY cv.id LIMIT 1",
                &[&me.id, &title],
            )
            .await
        {
            Ok(Some(r)) => r.get(0),
            Ok(None) => return err(StatusCode::NOT_FOUND, "this app isn't in a chat or channel with that name"),
            Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "database error"),
        },
        _ => return err(StatusCode::BAD_REQUEST, "say where: \"conv\" (a number) or \"channel\" (a name)"),
    };
    match db::may_post(&c, conv, me.id).await {
        Ok(Some(true)) => {}
        Ok(Some(false)) => {
            return err(StatusCode::FORBIDDEN, "only the channel's owners, and apps they added, post here");
        }
        Ok(None) => return err(StatusCode::FORBIDDEN, "this app isn't in that conversation"),
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "database error"),
    }
    if p.text.trim().is_empty() || p.text.len() > 64 * 1024 {
        return err(StatusCode::BAD_REQUEST, "messages are 1 to 65536 bytes");
    }
    let id = match db::send(&mut c, conv, me.id, &p.client_id, &p.text).await {
        Ok(id) => id,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't post"),
    };
    let seq = db::message(&c, id).await.ok().flatten().map(|m| m.seq).unwrap_or(0);
    Json(serde_json::json!({ "conv": conv, "seq": seq, "id": id })).into_response()
}

#[derive(serde::Deserialize)]
struct AppEdit {
    conv: i64,
    seq: i64,
    text: String,
}

/// `POST /v1/edit {"conv", "seq", "text"}`: replaces the app's own message
/// (send it again and again to stream an answer as it's written).
async fn app_edit(State(s): State<Server>, headers: HeaderMap, Json(p): Json<AppEdit>) -> Response {
    let me = match bearer(&s, &headers).await {
        Ok(u) => u,
        Err(r) => return *r,
    };
    match handle(&s, &me, ToServer::Edit { conv: p.conv, seq: p.seq, text: p.text }).await {
        Some(ToApp::Error { code, message }) => err(status_of(&code), &message),
        _ => Json(serde_json::json!({ "ok": true })).into_response(),
    }
}

/// A new key for `user` (shown once; only its hash is kept).
pub async fn issue_token(pool: &Pool, user: i64, device: &str) -> Result<String, String> {
    let token = random(32);
    let c = pool.get().await.map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO sessions (token_hash, user_id, device) VALUES ($1, $2, $3)",
        &[&hash_token(&token), &user, &device],
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(token)
}

async fn owns(c: &deadpool_postgres::Client, me: &User, app: i64) -> bool {
    matches!(db::app(c, app).await, Ok(Some((a, _))) if a.owner == me.id)
}

/// A body for `ToServer` variant `tag`.
fn as_msg(tag: &str, mut v: serde_json::Value) -> Result<ToServer, String> {
    if let Some(o) = v.as_object_mut() {
        o.insert("t".into(), tag.into());
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// Runs `msg` for the key's owner over HTTP, answering with JSON.
async fn api_call(s: Server, headers: HeaderMap, msg: Result<ToServer, String>) -> Response {
    let me = match bearer(&s, &headers).await {
        Ok(u) => u,
        Err(r) => return *r,
    };
    let msg = match msg {
        Ok(m) => m,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    match handle(&s, &me, msg).await {
        Some(ToApp::Error { code, message }) => err(status_of(&code), &message),
        Some(m) => Json(m).into_response(),
        None => Json(serde_json::json!({ "ok": true })).into_response(),
    }
}

/// Checks a webhook address: https:// (http:// too when local webhooks are
/// allowed), and, unless allowed, nowhere on this computer or a private
/// network. Empty is fine (no webhook).
pub async fn check_webhook(url: &str, local_ok: bool) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    let u = webhook_url(url, local_ok)?;
    if !local_ok {
        pinned_addrs(&u, false).await?;
    }
    Ok(u.to_string())
}

fn webhook_url(url: &str, local_ok: bool) -> Result<url::Url, String> {
    let u = url::Url::parse(url.trim())
        .map_err(|_| "a webhook is a web address, like https://example.org/xenify".to_string())?;
    match u.scheme() {
        "https" => {}
        "http" if local_ok => {}
        _ => return Err("webhooks must use https://".into()),
    }
    if u.host_str().is_none() {
        return Err("that web address has no host".into());
    }
    Ok(u)
}

/// Looks the host up once; unless `local_ok`, every address must be public.
async fn pinned_addrs(u: &url::Url, local_ok: bool) -> Result<Vec<std::net::SocketAddr>, String> {
    let host = u.host_str().unwrap_or_default().to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port))
        .await
        .map_err(|_| format!("{host} can't be found"))?
        .take(16)
        .collect();
    if addrs.is_empty() || (!local_ok && addrs.iter().any(|a| !public(a.ip()))) {
        return Err(format!("{host} is on a private network; webhooks must be public addresses"));
    }
    Ok(addrs)
}

/// A web address checked and looked up once: requests through
/// [`Pinned::agent`] go to exactly the addresses that were checked (the
/// name isn't looked up again, so it can't be pointed somewhere else in
/// between), while TLS and the Host header still use the name.
#[derive(Clone, Debug)]
pub struct Pinned {
    pub url: String,
    pub addrs: Vec<std::net::SocketAddr>,
}

/// Checks `url` as [`check_webhook`] does and pins where it goes.
pub async fn pin_webhook(url: &str, local_ok: bool) -> Result<Pinned, String> {
    let u = webhook_url(url, local_ok)?;
    let addrs = pinned_addrs(&u, local_ok).await?;
    Ok(Pinned { url: u.to_string(), addrs })
}

#[derive(Debug)]
struct PinnedResolver {
    host: String,
    addrs: Vec<std::net::SocketAddr>,
}

impl ureq::unversioned::resolver::Resolver for PinnedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        // Only the address that was checked.
        if uri.host().map(|h| h.trim_matches(['[', ']'])) != Some(self.host.as_str()) {
            return Err(ureq::Error::HostNotFound);
        }
        let mut out = self.empty();
        for a in self.addrs.iter().take(16) {
            out.push(*a);
        }
        if out.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(out)
    }
}

impl Pinned {
    /// An agent that connects only to the pinned addresses: no redirects,
    /// no proxy (a proxy would look the name up again).
    pub fn agent(&self, timeout: std::time::Duration) -> ureq::Agent {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .build();
        let host = url::Url::parse(&self.url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.trim_matches(['[', ']']).to_string()))
            .unwrap_or_default();
        ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            PinnedResolver { host, addrs: self.addrs.clone() },
        )
    }
}

/// An address on the public internet (not this computer, a private network,
/// link-local or shared address space).
fn public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_documentation()
                || v.is_multicast()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (18..20).contains(&o[1])))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let seg = v.segments();
            // NAT64: an IPv4 address inside.
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let o = v.octets();
                return public(IpAddr::V4(std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15])));
            }
            let first = seg[0];
            !(v.is_loopback()
                || v.is_multicast() || v.is_unspecified() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// HMAC-SHA256 of `data` with `secret`, as hex.
pub fn sign(secret: &str, data: &str) -> String {
    use hmac::{KeyInit, Mac};
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("any key length works");
    m.update(data.as_bytes());
    m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

impl Server {
    /// Sends message `id` to the webhooks of apps it's for: in a 1:1 chat
    /// with a personal app; in a group or channel when a shared app is
    /// @mentioned. An answer with {"text": …} is posted as the app's reply.
    /// (Messages from apps aren't sent on, so two bots can't loop.)
    fn deliver(&self, conv: i64, id: i64) {
        let s = self.clone();
        tokio::spawn(async move {
            let Ok(c) = s.pool.get().await else { return };
            let Ok(Some(msg)) = db::message(&c, id).await else { return };
            let Ok(Some((kind, _))) = db::conv_info(&c, conv).await else { return };
            let Ok(apps) = db::webhook_apps(&c, conv, msg.sender.id).await else { return };
            drop(c);
            let text = msg.text.to_lowercase();
            for (app, secret) in apps {
                let wanted = if kind == Kind::Direct {
                    app.mode != AppMode::Shared
                } else {
                    app.mode != AppMode::Personal && text.contains(&format!("@{}", app.user.username))
                };
                if !wanted {
                    continue;
                }
                // Checked again (the address could have moved since it was
                // saved), and sent to exactly the address checked.
                let Ok(pinned) = pin_webhook(&app.webhook, s.config.local_webhooks).await else { continue };
                let body = serde_json::json!({
                    "event": "message",
                    "app": app.user.username,
                    "conv": conv,
                    "kind": kind,
                    "message": msg,
                })
                .to_string();
                let at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
                    .to_string();
                let signature = sign(&secret, &format!("{at}.{body}"));
                let reply = tokio::task::spawn_blocking(move || -> Option<String> {
                    let agent = pinned.agent(std::time::Duration::from_secs(10));
                    let mut r = agent
                        .post(&pinned.url)
                        .header("Content-Type", "application/json")
                        .header("X-Xenify-Timestamp", &at)
                        .header("X-Xenify-Signature", format!("sha256={signature}"))
                        .send(body)
                        .ok()?;
                    if r.status().as_u16() != 200 {
                        return None;
                    }
                    let text = r.body_mut().with_config().limit(256 * 1024).read_to_string().ok()?;
                    serde_json::from_str::<serde_json::Value>(&text).ok()?.get("text")?.as_str().map(str::to_string)
                })
                .await
                .ok()
                .flatten();
                if let Some(t) = reply.filter(|t| !t.trim().is_empty() && t.len() <= 64 * 1024)
                    && let Ok(mut c) = s.pool.get().await
                {
                    let _ = db::send(&mut c, conv, app.user.id, "", &t).await;
                }
            }
        });
    }
}

/// The subprotocol browsers sign in with: they can't send an Authorization
/// header with a WebSocket, so they offer `["xenify-chat", <token>]` and
/// the server picks the first.
pub(crate) const BROWSER_PROTOCOL: &str = "xenify-chat";

async fn ws(State(s): State<Server>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    let offered: Vec<String> = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').map(|p| p.trim().to_string()).collect())
        .unwrap_or_default();
    let from_browser = offered.first().map(String::as_str) == Some(BROWSER_PROTOCOL);
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| from_browser.then(|| offered.get(1).cloned()).flatten())
        .unwrap_or_default();
    let token = token.as_str();
    let Ok(c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let hash = hash_token(token);
    let Some(user) = db::session_user(&c, &hash).await.ok().flatten() else {
        return err(StatusCode::UNAUTHORIZED, "sign in again");
    };
    drop(c);
    let upgrade = if from_browser { upgrade.protocols([BROWSER_PROTOCOL]) } else { upgrade };
    upgrade.max_message_size(256 * 1024).on_upgrade(move |socket| connection(s, user, hash, socket))
}

/// Whether a session still works; yes when the database can't say (a
/// moment's trouble there shouldn't close everyone's chats).
pub(crate) async fn still_signed_in(s: &Server, token_hash: &str) -> bool {
    let Ok(c) = s.pool.get().await else { return true };
    db::session_alive(&c, token_hash).await.unwrap_or(true)
}

/// One connected app.
async fn connection(s: Server, user: User, token_hash: String, socket: WebSocket) {
    let (mut out, mut inp) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ToApp>(QUEUE);
    let conn = Conn { tx: tx.clone(), full: Arc::default(), check: Arc::default() };
    s.online.lock().unwrap().entry(user.id).or_default().push(conn.clone());
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let text = serde_json::to_string(&m).expect("serialises");
            if out.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    // You're around now; and here's who else is.
    if let Ok(c) = s.pool.get().await {
        let settings = db::settings(&c, user.id).await.unwrap_or_else(|_| serde_json::json!({}));
        let _ = tx.send(welcome(&s, &c, user.clone(), settings).await).await;
        // Reminders still showing on your other devices.
        for reminder in db::open_reminders(&c, user.id).await.unwrap_or_default() {
            let _ = tx.send(ToApp::Reminder { reminder }).await;
        }
        let _ = db::presence_change(&c, &s.id, user.id, 1).await;
        if let Ok(ids) = db::contacts(&c, user.id).await
            && let Ok(list) = db::presence_of(&c, &ids).await
        {
            let _ = tx.send(ToApp::Presence { list }).await;
        }
    }
    s.notify(format!("o:{}", user.id)).await;
    // Calls already going in your conversations.
    for call in s.calls_for(user.id).await {
        let _ = tx.send(ToApp::Call { call }).await;
    }
    let mut every = tokio::time::interval_at(tokio::time::Instant::now() + RECHECK, RECHECK);
    loop {
        let frame = tokio::select! {
            f = inp.next() => f,
            _ = conn.full.notified() => break,
            _ = conn.check.notified() => {
                if !still_signed_in(&s, &token_hash).await {
                    break;
                }
                continue;
            }
            _ = every.tick() => {
                if !still_signed_in(&s, &token_hash).await {
                    break;
                }
                continue;
            }
        };
        let Some(Ok(frame)) = frame else { break };
        let WsMessage::Text(text) = frame else {
            if matches!(frame, WsMessage::Close(_)) {
                break;
            }
            continue;
        };
        let allowed = take_token(
            s.buckets.lock().unwrap().entry(user.id).or_insert((BURST, std::time::Instant::now())),
        );
        let reply = if !allowed {
            Some(error("slow_down", "that's a lot at once: wait a moment and try again"))
        } else {
            match serde_json::from_str::<ToServer>(&text) {
                // Following a channel checks you're still signed in first.
                Ok(ToServer::Subscribe { .. }) if !still_signed_in(&s, &token_hash).await => break,
                Ok(m) => handle(&s, &user, m).await,
                Err(e) => Some(ToApp::Error { code: "bad_request".into(), message: e.to_string() }),
            }
        };
        // Not reading what it's sent: it closes.
        if let Some(r) = reply
            && tx.try_send(r).is_err()
        {
            break;
        }
    }
    let last = {
        let mut online = s.online.lock().unwrap();
        let mut last = false;
        if let Some(list) = online.get_mut(&user.id) {
            list.retain(|x| !x.tx.same_channel(&tx));
            if list.is_empty() {
                online.remove(&user.id);
                s.buckets.lock().unwrap().remove(&user.id);
                last = true;
            }
        }
        last
    };
    writer.abort();
    // Their app went away: they're out of their calls.
    if last {
        s.gone(&user).await;
    }
    if let Ok(c) = s.pool.get().await {
        let _ = db::presence_change(&c, &s.id, user.id, -1).await;
    }
    s.notify(format!("o:{}", user.id)).await;
}

/// Whether `user` may do `perm` (e.g. "chat.create_channel"): the host
/// server's roles decide (Admin › Roles). On its own, without them, a chat
/// server lets everyone do everything but moderate.
pub(crate) async fn may(c: &deadpool_postgres::Client, user: i64, perm: &str) -> bool {
    match c.query_one("SELECT xenify_can($1::BIGINT, $2::TEXT)", &[&user, &perm]).await {
        Ok(r) => r.get(0),
        Err(e) if e.code() == Some(&tokio_postgres::error::SqlState::UNDEFINED_FUNCTION) => perm != "chat.moderate",
        Err(_) => false,
    }
}

fn denied(what: &str) -> ToApp {
    error("not_allowed", &format!("your server's admins haven't let you {what}"))
}

pub(crate) fn error(code: &str, message: &str) -> ToApp {
    ToApp::Error { code: code.into(), message: message.into() }
}

pub(crate) fn failed(e: impl std::fmt::Display) -> ToApp {
    error("server", &format!("something went wrong on the server ({e})"))
}

async fn welcome(s: &Server, c: &deadpool_postgres::Client, user: User, settings: serde_json::Value) -> ToApp {
    let mut perms = Vec::new();
    for p in xenify_chat_proto::PERMS {
        if may(c, user.id, p).await {
            perms.push(p.to_string());
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let ai = match s.assistant() {
        Some(a) => a.ready(user.id).await,
        None => false,
    };
    let threads = threads_on(c).await;
    ToApp::Welcome {
        user,
        version: xenify_chat_proto::VERSION,
        settings,
        keep_days: s.keep_days(),
        perms,
        now,
        threads,
        ai,
    }
}

/// Whether people reply in threads (Admin › Network › Chat; on unless an
/// admin turned it off, and on for a chat server on its own).
pub(crate) async fn threads_on(c: &deadpool_postgres::Client) -> bool {
    !matches!(
        c.query_opt("SELECT value FROM settings WHERE key = 'chat_threads'", &[]).await,
        Ok(Some(r)) if r.get::<_, &str>(0) == "off"
    )
}

fn no_answer(q: String, why: &str, tag: String) -> ToApp {
    ToApp::Answer { q, text: String::new(), sources: Vec::new(), error: why.into(), tag }
}

fn not_member() -> ToApp {
    error("not_member", "you're not in that conversation")
}

/// Looks people up by user name; the first unknown one is an error.
async fn user_ids(c: &deadpool_postgres::Client, names: &[String]) -> Result<Vec<i64>, Box<ToApp>> {
    let mut ids = Vec::new();
    for n in names {
        match db::user_id(c, n).await {
            Ok(Some(id)) => ids.push(id),
            Ok(None) => return Err(Box::new(error("no_user", &format!("there's no one called {n} here")))),
            Err(e) => return Err(Box::new(failed(e))),
        }
    }
    Ok(ids)
}

async fn handle(s: &Server, me: &User, m: ToServer) -> Option<ToApp> {
    let mut c = match s.pool.get().await {
        Ok(c) => c,
        Err(e) => return Some(failed(e)),
    };
    // The caller's role in a conversation, or the not-a-member answer.
    macro_rules! role {
        ($conv:expr) => {
            match db::role_of(&c, $conv, me.id).await {
                Ok(Some(r)) => r,
                Ok(None) => return Some(not_member()),
                Err(e) => return Some(failed(e)),
            }
        };
    }
    macro_rules! info {
        ($conv:expr) => {
            match db::conv_info(&c, $conv).await {
                Ok(Some(i)) => i,
                Ok(None) => return Some(error("no_conversation", "that conversation doesn't exist")),
                Err(e) => return Some(failed(e)),
            }
        };
    }
    // The conversation as the caller sees it, as the answer.
    macro_rules! view {
        ($conv:expr) => {
            match db::conversation(&c, $conv, me.id).await {
                Ok(Some(conv)) => Some(ToApp::Conversation { conv }),
                Ok(None) => Some(not_member()),
                Err(e) => Some(failed(e)),
            }
        };
    }
    match m {
        ToServer::Ping => Some(ToApp::Pong),
        ToServer::List => Some(match db::conversations(&c, me.id).await {
            Ok(list) => ToApp::Conversations { list },
            Err(e) => failed(e),
        }),
        ToServer::OpenDirect { user } => {
            let other = match user_ids(&c, &[user]).await {
                Ok(ids) => ids[0],
                Err(e) => return Some(*e),
            };
            if let Ok(Some((app, _))) = db::app(&c, other).await
                && app.mode == AppMode::Shared
            {
                return Some(error("not_allowed", "this app only answers in groups and channels"));
            }
            let conv = match db::open_direct(&mut c, me.id, other).await {
                Ok(id) => id,
                Err(e) => return Some(failed(e)),
            };
            s.notify(format!("c:{conv}")).await;
            view!(conv)
        }
        ToServer::CreateGroup { title, members, purpose } => {
            if !["", MEETING, CALENDAR, CONTACTS].contains(&purpose.as_str()) {
                return Some(error("bad_request", "a group is a chat, a meeting, a calendar or an address book"));
            }
            if purpose == CALENDAR || purpose == CONTACTS {
                match db::owned_with_purpose(&c, me.id, &purpose).await {
                    Ok(Some(conv)) => return view!(conv),
                    Ok(None) => {}
                    Err(e) => return Some(failed(e)),
                }
            }
            let ids = match user_ids(&c, &members).await {
                Ok(ids) => ids,
                Err(e) => return Some(*e),
            };
            let ids = if purpose == CALENDAR || purpose == CONTACTS { Vec::new() } else { ids };
            let conv = match db::create(&mut c, Kind::Group, title.trim(), false, me.id, &ids, &purpose).await {
                Ok(id) => id,
                Err(e) => return Some(failed(e)),
            };
            s.notify(format!("c:{conv}")).await;
            view!(conv)
        }
        ToServer::CreateChannel { title, open, purpose } => {
            if title.trim().is_empty() {
                return Some(error("bad_request", "a channel needs a name"));
            }
            if !["", BUSY, BOOK, SHARED_CAL].contains(&purpose.as_str()) {
                return Some(error("bad_request", "a channel is for posts, busy times, an address book or a calendar"));
            }
            // Busy times and address books are the apps' own, not channels people see.
            if purpose.is_empty() {
                if !may(&c, me.id, "chat.create_channel").await {
                    return Some(denied("make channels"));
                }
                if open && !may(&c, me.id, "chat.create_open_channel").await {
                    return Some(denied("make channels anyone can follow"));
                }
            }
            if purpose == BUSY {
                match db::owned_with_purpose(&c, me.id, BUSY).await {
                    Ok(Some(conv)) => return view!(conv),
                    Ok(None) => {}
                    Err(e) => return Some(failed(e)),
                }
            }
            let conv = match db::create(&mut c, Kind::Channel, title.trim(), open, me.id, &[], &purpose).await {
                Ok(id) => id,
                Err(e) => return Some(failed(e)),
            };
            view!(conv)
        }
        ToServer::SetOpen { conv, open } => {
            if role!(conv) != Role::Owner {
                return Some(error("not_owner", "only the channel's owners can change that"));
            }
            if open
                && db::purpose_of(&c, conv).await.is_ok_and(|p| p.is_empty())
                && !may(&c, me.id, "chat.create_open_channel").await
            {
                return Some(denied("make channels anyone can follow"));
            }
            if let Err(e) = c
                .execute("UPDATE conversations SET open = $2 WHERE id = $1 AND kind = 'channel'", &[&conv, &open])
                .await
            {
                return Some(failed(e));
            }
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::SetRole { conv, user, owner } => {
            if role!(conv) != Role::Owner {
                return Some(error("not_owner", "only those who can change it say who else can"));
            }
            // Address books (can change / can look), groups and channels (co-owners).
            match db::purpose_of(&c, conv).await {
                Ok(p) if p == BOOK || p == SHARED_CAL || p.is_empty() => {}
                Ok(_) => return Some(error("not_allowed", "that one has a single owner")),
                Err(e) => return Some(failed(e)),
            }
            if info!(conv).0 == Kind::Direct {
                return Some(error("not_group", "a direct chat has no owners"));
            }
            let id = match user_ids(&c, &[user]).await {
                Ok(ids) => ids[0],
                Err(e) => return Some(*e),
            };
            if id == me.id && !owner {
                return Some(error("bad_request", "someone else has to change that for you"));
            }
            if !matches!(db::role_of(&c, conv, id).await, Ok(Some(_))) {
                return Some(error("not_member", "they aren't in it"));
            }
            let role = if owner { "owner" } else { "member" };
            if let Err(e) =
                c.execute("UPDATE members SET role = $3 WHERE conv = $1 AND user_id = $2", &[&conv, &id, &role]).await
            {
                return Some(failed(e));
            }
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::Things => Some(things::things(&c, me.id).await),
        ToServer::SaveThing { thing } => Some(things::save_thing(&c, me.id, thing).await),
        ToServer::RemoveThing { id } => Some(things::remove_thing(&c, me.id, id).await),
        ToServer::Bookings { things: list, from, to } => Some(things::bookings(&c, me.id, &list, from, to).await),
        ToServer::MyBookings => Some(things::my_bookings(&c, me.id).await),
        ToServer::Book { thing, start, end, meeting, tag } => {
            Some(things::book(s, &mut c, me.id, thing, start, end, meeting, tag).await)
        }
        ToServer::AnswerBooking { booking, yes } => things::answer(s, &c, me.id, booking, yes).await,
        ToServer::CancelBooking { booking } => things::cancel(s, &c, me.id, booking).await,
        ToServer::SetAway { away } => things::set_away(&c, me.id, away).await,
        ToServer::AwayOf { users } => Some(things::away_of(&c, &users).await),
        ToServer::BusyOf { users } => {
            if users.len() > 500 {
                return Some(error("bad_request", "ask about fewer people at once"));
            }
            Some(match db::busy_of(&c, &users, me.id).await {
                Ok(list) => ToApp::BusyOf { list },
                Err(e) => failed(e),
            })
        }
        ToServer::Subscribe { conv } => {
            let (kind, open) = info!(conv);
            if kind != Kind::Channel || !open {
                return Some(error("not_open", "only open channels can be subscribed to"));
            }
            if me.app {
                return Some(error("not_allowed", "apps join a channel when one of its owners adds them"));
            }
            if let Err(e) = db::add_member(&c, conv, me.id, None).await {
                return Some(failed(e));
            }
            s.notify(format!("c:{conv}")).await;
            view!(conv)
        }
        ToServer::AddPeople { conv, users } => {
            let role = role!(conv);
            let (kind, _) = info!(conv);
            if kind == Kind::Direct {
                return Some(error("not_group", "a direct chat is between two people: start a group instead"));
            }
            if kind == Kind::Channel && role != Role::Owner {
                return Some(error("not_owner", "only the channel's owners can add people"));
            }
            match db::purpose_of(&c, conv).await {
                Ok(p) if p == CALENDAR || p == CONTACTS => return Some(error("not_allowed", "that's yours alone")),
                Ok(p) if p == MEETING && role != Role::Owner => {
                    return Some(error("not_owner", "only the organiser can invite people"));
                }
                Ok(_) => {}
                Err(e) => return Some(failed(e)),
            }
            let ids = match user_ids(&c, &users).await {
                Ok(ids) => ids,
                Err(e) => return Some(*e),
            };
            for id in ids {
                if let Err(e) = db::add_member(&c, conv, id, Some(me.id)).await {
                    return Some(failed(e));
                }
            }
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::Call { conv, action, video } => s.call(me, conv, action, video).await,
        ToServer::RemovePerson { conv, user } => {
            if role!(conv) != Role::Owner && !may(&c, me.id, "chat.moderate").await {
                return Some(error("not_owner", "only owners can take people out"));
            }
            if info!(conv).0 == Kind::Direct {
                return Some(error("not_group", "nobody can be taken out of a direct chat"));
            }
            let id = match user_ids(&c, &[user]).await {
                Ok(ids) => ids[0],
                Err(e) => return Some(*e),
            };
            if id == me.id {
                return Some(error("bad_request", "to go yourself, leave the conversation"));
            }
            if let Err(e) = c.execute("DELETE FROM members WHERE conv = $1 AND user_id = $2", &[&conv, &id]).await {
                return Some(failed(e));
            }
            s.notify(format!("x:{conv}:{id}")).await;
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::Leave { conv } => {
            role!(conv);
            let (kind, _) = info!(conv);
            if kind == Kind::Direct {
                return Some(error("not_group", "direct chats can't be left"));
            }
            let gone = match db::leave(&mut c, conv, me.id).await {
                Ok(g) => g,
                Err(e) => return Some(failed(e)),
            };
            s.notify(format!("x:{conv}:{}", me.id)).await;
            if !gone {
                s.notify(format!("c:{conv}")).await;
            }
            None
        }
        ToServer::Send { conv, client_id, text } => {
            match db::may_post(&c, conv, me.id).await {
                Ok(Some(true)) => {}
                Ok(Some(false)) => return Some(error("not_allowed", "only the channel's owners post here")),
                Ok(None) => return Some(not_member()),
                Err(e) => return Some(failed(e)),
            }
            if text.trim().is_empty() || text.len() > 96 * 1024 {
                return Some(error("bad_message", "messages are 1 to 98304 bytes"));
            }
            match db::send(&mut c, conv, me.id, &client_id, &text).await {
                Ok(id) => {
                    if !me.app {
                        s.deliver(conv, id);
                    }
                    None
                }
                Err(e) => Some(failed(e)),
            }
        }
        ToServer::Reply { conv, thread, client_id, text } => {
            match db::may_post(&c, conv, me.id).await {
                Ok(Some(true)) => {}
                Ok(Some(false)) => return Some(error("not_allowed", "only the channel's owners post here")),
                Ok(None) => return Some(not_member()),
                Err(e) => return Some(failed(e)),
            }
            if !threads_on(&c).await {
                return Some(error("no_threads", "this server's admins turned threads off"));
            }
            if text.trim().is_empty() || text.len() > 96 * 1024 {
                return Some(error("bad_message", "messages are 1 to 98304 bytes"));
            }
            match db::thread_root(&c, conv, thread).await {
                Ok(None) => {}
                Ok(Some(why)) => return Some(error("no_thread", why)),
                Err(e) => return Some(failed(e)),
            }
            match db::reply(&mut c, conv, thread, me.id, &client_id, &text).await {
                Ok(id) => {
                    if !me.app {
                        s.deliver(conv, id);
                    }
                    None
                }
                Err(e) => Some(failed(e)),
            }
        }
        ToServer::Thread { conv, thread, tag } => {
            let public = matches!(db::conv_info(&c, conv).await, Ok(Some((Kind::Channel, true))));
            if !public {
                role!(conv);
            }
            Some(match db::thread(&c, conv, thread).await {
                Ok(messages) => ToApp::History { conv, messages, thread: Some(thread), tag },
                Err(e) => failed(e),
            })
        }
        ToServer::Edit { conv, seq, text } => {
            role!(conv);
            if text.trim().is_empty() || text.len() > 96 * 1024 {
                return Some(error("bad_message", "messages are 1 to 98304 bytes"));
            }
            edit(s, &c, me, conv, seq, Some(&text), false).await
        }
        ToServer::Delete { conv, seq } => {
            // Your own messages; in groups and channels, owners and
            // moderators take down anyone's.
            let role = role!(conv);
            let others =
                info!(conv).0 != Kind::Direct && (role == Role::Owner || may(&c, me.id, "chat.moderate").await);
            edit(s, &c, me, conv, seq, None, others).await
        }
        ToServer::Rename { conv, title } => {
            let role = role!(conv);
            let (kind, _) = info!(conv);
            let title = title.trim();
            if kind == Kind::Direct {
                return Some(error("not_group", "a direct chat is named after the person"));
            }
            if title.is_empty() || title.chars().count() > 100 {
                return Some(error("bad_request", "names are 1 to 100 characters"));
            }
            if !db::purpose_of(&c, conv).await.is_ok_and(|p| p.is_empty()) {
                return Some(error("not_allowed", "that isn't renamed here"));
            }
            if kind == Kind::Channel && role != Role::Owner && !may(&c, me.id, "chat.moderate").await {
                return Some(error("not_owner", "only the channel's owners can rename it"));
            }
            if let Err(e) = c.execute("UPDATE conversations SET title = $2 WHERE id = $1", &[&conv, &title]).await {
                return Some(failed(e));
            }
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::React { conv, seq, emoji, on } => {
            role!(conv);
            // One emoji (with its modifiers), not a message.
            if emoji.trim().is_empty()
                || emoji.len() > 32
                || emoji.chars().any(|ch| ch.is_alphanumeric() || ch.is_whitespace())
            {
                return Some(error("bad_request", "a reaction is a single emoji"));
            }
            let id: i64 = match c
                .query_opt("SELECT id FROM messages WHERE conv = $1 AND seq = $2 AND NOT deleted", &[&conv, &seq])
                .await
            {
                Ok(Some(r)) => r.get(0),
                Ok(None) => return Some(error("no_message", "that message isn't there any more")),
                Err(e) => return Some(failed(e)),
            };
            let r = if on {
                c.execute(
                    "INSERT INTO reactions (message_id, user_id, emoji) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                    &[&id, &me.id, &emoji],
                )
                .await
            } else {
                c.execute(
                    "DELETE FROM reactions WHERE message_id = $1 AND user_id = $2 AND emoji = $3",
                    &[&id, &me.id, &emoji],
                )
                .await
            };
            if let Err(e) = r {
                return Some(failed(e));
            }
            // Everyone gets the message again, with its reactions.
            s.notify(format!("m:{conv}:{id}")).await;
            None
        }
        ToServer::History { conv, before, limit, tag } => {
            // Anyone may read a public channel before deciding to follow it.
            let public = matches!(db::conv_info(&c, conv).await, Ok(Some((Kind::Channel, true))));
            if !public {
                role!(conv);
            }
            let threads = threads_on(&c).await;
            Some(match db::history(&c, conv, before, limit, threads).await {
                Ok(messages) => ToApp::History { conv, messages, thread: None, tag },
                Err(e) => failed(e),
            })
        }
        ToServer::Read { conv, seq } => {
            role!(conv);
            match c
                .query_opt(
                    "UPDATE members SET read_seq = GREATEST(read_seq, LEAST($3, (SELECT next_seq FROM conversations WHERE id = $1)))
                     WHERE conv = $1 AND user_id = $2 RETURNING read_seq",
                    &[&conv, &me.id, &seq],
                )
                .await
            {
                Ok(Some(r)) => {
                    let seq: i64 = r.get(0);
                    s.notify(format!("r:{conv}:{}:{seq}", me.id)).await;
                    None
                }
                Ok(None) => Some(not_member()),
                Err(e) => Some(failed(e)),
            }
        }
        ToServer::Unread { conv, seq } => {
            role!(conv);
            match c
                .query_opt(
                    "UPDATE members SET read_seq = LEAST(read_seq, GREATEST($3::bigint, 0)) WHERE conv = $1 AND user_id = $2 RETURNING read_seq",
                    &[&conv, &me.id, &seq],
                )
                .await
            {
                Ok(Some(r)) => {
                    let seq: i64 = r.get(0);
                    s.notify(format!("r:{conv}:{}:{seq}", me.id)).await;
                    None
                }
                Ok(None) => Some(error("not_member", "you're not in that chat")),
                Err(e) => Some(failed(e)),
            }
        }
        ToServer::SetNotify { conv, notify, muted_until } => {
            role!(conv);
            if let Err(e) = c
                .execute(
                    "UPDATE members SET notify = $3,
                            muted_until = CASE WHEN $4::bigint IS NULL THEN NULL ELSE to_timestamp($4::bigint / 1000.0) END
                     WHERE conv = $1 AND user_id = $2",
                    &[&conv, &me.id, &db::notify_str(notify), &muted_until],
                )
                .await
            {
                return Some(failed(e));
            }
            // Your other devices follow.
            s.notify(format!("p:{conv}:{}", me.id)).await;
            None
        }
        ToServer::SetStatus { status } => {
            let status = match status {
                Presence::Away => "away",
                Presence::Busy => "busy",
                _ => "online",
            };
            if let Err(e) = c.execute("UPDATE users SET status = $2 WHERE id = $1", &[&me.id, &status]).await {
                return Some(failed(e));
            }
            s.notify(format!("o:{}", me.id)).await;
            None
        }
        ToServer::GetPresence { users } => {
            // Only people you chat with.
            let known = match db::contacts(&c, me.id).await {
                Ok(k) => k,
                Err(e) => return Some(failed(e)),
            };
            let ids: Vec<i64> = users.into_iter().filter(|u| known.contains(u)).take(500).collect();
            Some(match db::presence_of(&c, &ids).await {
                Ok(list) => ToApp::Presence { list },
                Err(e) => failed(e),
            })
        }
        ToServer::CreateApp { name, display_name, description, webhook, mode } => {
            if me.app {
                return Some(error("not_allowed", "apps can't make apps"));
            }
            if !may(&c, me.id, "chat.create_apps").await {
                return Some(denied("make apps"));
            }
            let webhook = match check_webhook(&webhook, s.config.local_webhooks).await {
                Ok(w) => w,
                Err(m) => return Some(error("bad_webhook", &m)),
            };
            let name = name.trim().to_lowercase();
            if !valid_username(&name) {
                return Some(error("bad_request", "app names are 2–32 letters, digits, dots, dashes or underscores"));
            }
            let user = match db::create_user(&c, &name, &clean_name(&display_name), &hash_password(&random(32))).await {
                Ok(u) => u,
                Err(_) => return Some(error("taken", "that name is taken")),
            };
            let secret = random(32);
            let made = c
                .execute("UPDATE users SET app = TRUE WHERE id = $1", &[&user.id])
                .await
                .and(
                    c.execute(
                        "INSERT INTO apps (user_id, owner, description, webhook, secret, mode) VALUES ($1, $2, $3, $4, $5, $6)",
                        &[&user.id, &me.id, &description.trim(), &webhook, &secret, &db::mode_str(mode)],
                    )
                    .await,
                );
            if let Err(e) = made {
                return Some(failed(e));
            }
            let key = match issue_token(&s.pool, user.id, "app").await {
                Ok(k) => k,
                Err(e) => return Some(error("failed", &e)),
            };
            match db::app(&c, user.id).await {
                Ok(Some((app, _))) => Some(ToApp::AppKey { app, key, secret }),
                _ => Some(error("failed", "the app was made but couldn't be read back")),
            }
        }
        ToServer::UpdateApp { app, display_name, description, webhook, mode } => {
            if !owns(&c, me, app).await {
                return Some(error("not_owner", "that isn't your app"));
            }
            let webhook = match check_webhook(&webhook, s.config.local_webhooks).await {
                Ok(w) => w,
                Err(m) => return Some(error("bad_webhook", &m)),
            };
            let r = c
                .execute("UPDATE users SET display_name = $2 WHERE id = $1", &[&app, &clean_name(&display_name)])
                .await
                .and(
                    c.execute(
                        "UPDATE apps SET description = $2, webhook = $3, mode = $4 WHERE user_id = $1",
                        &[&app, &description.trim(), &webhook, &db::mode_str(mode)],
                    )
                    .await,
                );
            if let Err(e) = r {
                return Some(failed(e));
            }
            Some(match db::apps_owned(&c, me.id).await {
                Ok(list) => ToApp::Apps { list, conv: 0 },
                Err(e) => failed(e),
            })
        }
        ToServer::NewAppKey { app } => {
            if !owns(&c, me, app).await {
                return Some(error("not_owner", "that isn't your app"));
            }
            let secret = random(32);
            let r = c
                .execute("DELETE FROM sessions WHERE user_id = $1", &[&app])
                .await
                .and(c.execute("UPDATE apps SET secret = $2 WHERE user_id = $1", &[&app, &secret]).await);
            if let Err(e) = r {
                return Some(failed(e));
            }
            let key = match issue_token(&s.pool, app, "app").await {
                Ok(k) => k,
                Err(e) => return Some(error("failed", &e)),
            };
            match db::app(&c, app).await {
                Ok(Some((info, _))) => Some(ToApp::AppKey { app: info, key, secret }),
                _ => Some(error("failed", "couldn't read the app")),
            }
        }
        ToServer::DeleteApp { app } => {
            if !owns(&c, me, app).await {
                return Some(error("not_owner", "that isn't your app"));
            }
            // Its messages stay; it leaves every chat and can't be used again.
            let convs: Vec<i64> = c
                .query("SELECT conv FROM members WHERE user_id = $1", &[&app])
                .await
                .map(|rows| rows.iter().map(|r| r.get(0)).collect())
                .unwrap_or_default();
            let r = c
                .execute("UPDATE users SET disabled = TRUE WHERE id = $1", &[&app])
                .await
                .and(c.execute("DELETE FROM sessions WHERE user_id = $1", &[&app]).await)
                .and(c.execute("DELETE FROM members WHERE user_id = $1", &[&app]).await)
                .and(c.execute("DELETE FROM apps WHERE user_id = $1", &[&app]).await);
            if let Err(e) = r {
                return Some(failed(e));
            }
            for conv in convs {
                s.notify(format!("c:{conv}")).await;
            }
            Some(match db::apps_owned(&c, me.id).await {
                Ok(list) => ToApp::Apps { list, conv: 0 },
                Err(e) => failed(e),
            })
        }
        ToServer::ListApps => Some(match db::apps_owned(&c, me.id).await {
            Ok(list) => ToApp::Apps { list, conv: 0 },
            Err(e) => failed(e),
        }),
        ToServer::AppsIn { conv } => {
            role!(conv);
            Some(match db::apps_in(&c, conv).await {
                Ok(list) => ToApp::Apps { list, conv },
                Err(e) => failed(e),
            })
        }
        ToServer::NewApiKey => {
            if me.app {
                return Some(error("not_allowed", "apps use their own key"));
            }
            if !may(&c, me.id, "chat.create_apps").await {
                return Some(denied("make keys for scripts"));
            }
            Some(match issue_token(&s.pool, me.id, "api").await {
                Ok(key) => ToApp::ApiKey { key },
                Err(e) => error("failed", &e),
            })
        }
        ToServer::SetName { display_name } => {
            let name = clean_name(&display_name);
            if let Err(e) = c.execute("UPDATE users SET display_name = $2 WHERE id = $1", &[&me.id, &name]).await {
                return Some(failed(e));
            }
            // Everyone who chats with you sees the new name.
            let convs: Vec<i64> = match c.query("SELECT conv FROM members WHERE user_id = $1", &[&me.id]).await {
                Ok(rows) => rows.iter().map(|r| r.get(0)).collect(),
                Err(e) => return Some(failed(e)),
            };
            for conv in convs {
                s.notify(format!("c:{conv}")).await;
            }
            let settings = db::settings(&c, me.id).await.unwrap_or_else(|_| serde_json::json!({}));
            Some(welcome(s, &c, User { display_name: name, ..me.clone() }, settings).await)
        }
        ToServer::Typing { conv } => {
            role!(conv);
            s.notify(format!("t:{conv}:{}", me.id)).await;
            None
        }
        ToServer::FindPeople { q } => Some(match db::find_people(&c, &q, me.id).await {
            Ok(users) => ToApp::People { q, users },
            Err(e) => failed(e),
        }),
        ToServer::FindConversations { q } => Some(match db::find_conversations(&c, &q, me.id).await {
            Ok(conversations) => ToApp::Found { q, conversations },
            Err(e) => failed(e),
        }),
        ToServer::Search { q, conv, sort, after, tag } => {
            Some(match db::search(&c, me.id, &q, conv, sort, &after).await {
                Ok(page) => ToApp::Hits { q, hits: page.hits, sort, total: page.total, more: page.more, after, tag },
                Err(e) => failed(e),
            })
        }
        ToServer::Ask { q, conv, tag } => {
            let Some(ai) = s.assistant() else {
                return Some(no_answer(q, "This server has no AI.", tag));
            };
            if !ai.ready(me.id).await {
                return Some(no_answer(q, "Your server's admins haven't let you use its AI.", tag));
            }
            if q.trim().is_empty() || q.len() > 2000 {
                return Some(no_answer(q, "Ask a question of up to 2,000 letters.", tag));
            }
            // It takes a while: the answer comes on its own.
            let (s, me) = (s.clone(), me.clone());
            tokio::spawn(async move {
                let out = ask::ask(&s, ai, &me, &q, conv).await;
                let m = match out {
                    Ok((text, sources)) => ToApp::Answer { q, text, sources, error: String::new(), tag },
                    Err(error) => no_answer(q, &error, tag),
                };
                s.tell(&[me.id], m);
            });
            None
        }
        ToServer::SetKeep { conv, days } => {
            let role = role!(conv);
            let (kind, _) = info!(conv);
            if kind == Kind::Direct {
                return Some(error("not_group", "only groups and channels have that"));
            }
            if role != Role::Owner {
                return Some(error("not_owner", "only owners choose how long messages are kept"));
            }
            if days.is_some_and(|d| !(1..=36500).contains(&d)) {
                return Some(error("bad_request", "keep messages for 1 day or more"));
            }
            let days = days.map(|d| d as i32);
            if let Err(e) = c.execute("UPDATE conversations SET keep_days = $2 WHERE id = $1", &[&conv, &days]).await {
                return Some(failed(e));
            }
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::SaveSettings { settings } => {
            if !settings.is_object() || settings.to_string().len() > 64 * 1024 {
                return Some(error("bad_request", "settings are a JSON object of at most 64 KB"));
            }
            if let Err(e) = db::save_settings(&c, me.id, &settings).await {
                return Some(failed(e));
            }
            s.notify(format!("s:{}", me.id)).await;
            None
        }
        ToServer::SetReminders { list } => {
            if list.len() > 2000 || list.iter().any(|r| r.key.is_empty() || r.key.len() > 200) {
                return Some(error("bad_request", "at most 2000 reminders, each with a key"));
            }
            match db::set_reminders(&mut c, me.id, &list).await {
                // Any already due show straight away.
                Ok(()) => {
                    let _ = s.due_reminders().await;
                    None
                }
                Err(e) => Some(failed(e)),
            }
        }
        ToServer::Snooze { key, until } => {
            let r = c
                .execute(
                    "UPDATE reminders SET at = to_timestamp($3::bigint / 1000.0), shown = NULL WHERE user_id = $1 AND key = $2",
                    &[&me.id, &key, &until],
                )
                .await;
            if let Err(e) = r {
                return Some(failed(e));
            }
            s.notify(format!("rg:{}:{key}", me.id)).await;
            None
        }
        ToServer::Dismiss { key } => {
            // Kept as done, so sending the list again doesn't bring it back.
            if let Err(e) = c
                .execute(
                    "UPDATE reminders SET shown = COALESCE(shown, now()), done = TRUE WHERE user_id = $1 AND key = $2",
                    &[&me.id, &key],
                )
                .await
            {
                return Some(failed(e));
            }
            s.notify(format!("rg:{}:{key}", me.id)).await;
            None
        }
    }
}

/// Changes (or, with no text, deletes) your own message.
async fn edit(
    s: &Server,
    c: &deadpool_postgres::Client,
    me: &User,
    conv: i64,
    seq: i64,
    text: Option<&str>,
    // Deleting someone else's (owners, moderators).
    others: bool,
) -> Option<ToApp> {
    let r = match text {
        Some(t) => {
            c.query_opt(
                "UPDATE messages SET body = $4, edited = TRUE WHERE conv = $1 AND seq = $2 AND sender = $3 AND NOT deleted RETURNING id",
                &[&conv, &seq, &me.id, &t],
            )
            .await
        }
        None => {
            c.query_opt(
                "UPDATE messages SET body = '', deleted = TRUE WHERE conv = $1 AND seq = $2 AND (sender = $3 OR $4) RETURNING id",
                &[&conv, &seq, &me.id, &others],
            )
            .await
        }
    };
    match r {
        Ok(Some(row)) => {
            s.notify(format!("m:{conv}:{}", row.get::<_, i64>(0))).await;
            None
        }
        Ok(None) => Some(error("not_yours", "you can only change your own messages")),
        Err(e) => Some(failed(e)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_rate_limit_lets_a_burst_through_then_refills() {
        let mut b = (super::BURST, std::time::Instant::now());
        let sent = (0..1000).take_while(|_| super::take_token(&mut b)).count();
        assert!((600..=601).contains(&sent), "{sent}");
        b.1 -= std::time::Duration::from_secs(1);
        assert!((0..50).all(|_| super::take_token(&mut b)), "a second later, more");
    }
}
