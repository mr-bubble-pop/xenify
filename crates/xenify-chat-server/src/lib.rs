//! The Xenify Chat server.
//!
//! - Accounts with passwords (argon2) and revocable sessions (tokens stored
//!   only as hashes); registration by invite code unless opened up.
//! - Direct chats, groups and channels in PostgreSQL. Message numbers come
//!   from a row lock, so order holds across servers.
//! - Servers are interchangeable: each listens for PostgreSQL notifications
//!   (LISTEN/NOTIFY) and pushes to the people connected to it. Put several
//!   behind a load balancer.
//! - No message search here: apps search their own copy.

pub mod db;

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::Pool;
use futures_util::{SinkExt, StreamExt};
use xenify_chat_proto::{AppMode, Kind, Login, Presence, Role, Session, ToApp, ToServer, User};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// postgres://user:password@host/db
    pub database: String,
    /// Anyone may create an account (otherwise an invite code is needed).
    pub open_registration: bool,
    /// Let app webhooks point at this computer or its network (for tests;
    /// otherwise only public https:// addresses, so nobody can use the
    /// server to reach machines behind it).
    pub local_webhooks: bool,
}

type Online = HashMap<i64, Vec<mpsc::UnboundedSender<ToApp>>>;

#[derive(Clone)]
pub struct Server {
    pool: Pool,
    config: Config,
    /// People connected to this server: user id → their connections.
    online: Arc<Mutex<Online>>,
    /// This server's name in the presence table (new each start).
    id: Arc<str>,
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
    let r = c.batch_execute(db::SCHEMA).await.map_err(|e| e.to_string());
    let _ = c.execute("SELECT pg_advisory_unlock(7420426)", &[]).await;
    r
}

impl Server {
    pub async fn start(config: Config) -> Result<Server, String> {
        let pool = pool(&config.database)?;
        migrate(&pool).await?;
        let s = Server { pool, config, online: Arc::default(), id: random(12).into() };
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

    pub fn router(&self) -> Router {
        Router::new()
            .route("/v1/login", post(login))
            .route("/v1/register", post(register))
            .route("/v1/ws", get(ws))
            .route("/v1/health", get(|| async { "ok" }))
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
    /// - `o:user`: someone came online, went offline or changed status.
    async fn fan_out(&self, payload: &str) -> Result<(), String> {
        let parts: Vec<&str> = payload.split(':').collect();
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
                for u in here(db::member_ids(&c, conv).await.map_err(|e| e.to_string())?) {
                    if let Some(conv) = db::conversation(&c, conv, u).await.map_err(|e| e.to_string())? {
                        self.tell(&[u], ToApp::Conversation { conv });
                    }
                }
            }
            Some("p") => {
                if let Some(conv) = db::conversation(&c, conv, num(2)).await.map_err(|e| e.to_string())? {
                    self.tell(&[num(2)], ToApp::Conversation { conv });
                }
            }
            Some("x") => self.tell(&[num(2)], ToApp::Removed { conv }),
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

    fn tell(&self, users: &[i64], m: ToApp) {
        let mut online = self.online.lock().unwrap();
        for u in users {
            if let Some(list) = online.get_mut(u) {
                list.retain(|tx| tx.send(m.clone()).is_ok());
            }
        }
    }

    async fn notify(&self, payload: String) {
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

pub fn hash_password(password: &str) -> String {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("random");
    let salt = SaltString::encode_b64(&salt).expect("salt");
    argon2::Argon2::default().hash_password(password.as_bytes(), &salt).expect("hash").to_string()
}

fn password_ok(password: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(hash).is_ok_and(|h| argon2::Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
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

async fn new_session(s: &Server, user: User, device: &str) -> Response {
    let token = random(32);
    let Ok(c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    if c.execute(
        "INSERT INTO sessions (token_hash, user_id, device) VALUES ($1, $2, $3)",
        &[&hash_token(&token), &user.id, &device],
    )
    .await
    .is_err()
    {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't sign in");
    }
    Json(Session { token, user }).into_response()
}

async fn login(State(s): State<Server>, Json(l): Json<Login>) -> Response {
    let Ok(c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let found = db::user_by_name(&c, &l.username.trim().to_lowercase()).await.ok().flatten();
    // Hash anyway when there's no such user, so timing doesn't tell.
    let (user, hash, disabled) = found.unwrap_or((
        User { id: 0, username: String::new(), display_name: String::new(), app: false },
        hash_password("x"),
        true,
    ));
    if disabled || !password_ok(&l.password, &hash) {
        return err(StatusCode::UNAUTHORIZED, "wrong user name or password");
    }
    new_session(&s, user, &l.device).await
}

async fn register(State(s): State<Server>, Json(l): Json<Login>) -> Response {
    let name = l.username.trim().to_lowercase();
    if !valid_username(&name) {
        return err(StatusCode::BAD_REQUEST, "user names are 2–32 letters, digits, dots, dashes or underscores");
    }
    if l.password.chars().count() < 10 {
        return err(StatusCode::BAD_REQUEST, "use a password of at least 10 characters");
    }
    let Ok(mut c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let Ok(tx) = c.transaction().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    if !s.config.open_registration {
        let ok = tx
            .query_opt(
                "SELECT 1 FROM invites WHERE code = $1 AND used_by IS NULL AND expires > now() FOR UPDATE",
                &[&l.invite.trim()],
            )
            .await
            .ok()
            .flatten()
            .is_some();
        if !ok {
            return err(StatusCode::FORBIDDEN, "an invite code is needed (or it was used or ran out)");
        }
    }
    let user = match tx
        .query_one(
            "INSERT INTO users (username, display_name, password_hash) VALUES ($1, $2, $3) RETURNING id, username, display_name, app",
            &[&name, &clean_name(&l.display_name), &hash_password(&l.password)],
        )
        .await
    {
        Ok(r) => User { id: r.get(0), username: r.get(1), display_name: r.get(2), app: r.get(3) },
        Err(_) => return err(StatusCode::CONFLICT, "that user name is taken"),
    };
    let _ = tx.execute("UPDATE invites SET used_by = $2 WHERE code = $1", &[&l.invite.trim(), &user.id]).await;
    if tx.commit().await.is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't create the account");
    }
    drop(c);
    new_session(&s, user, &l.device).await
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
    db::session_user(&c, &hash_token(token)).await.ok().flatten().ok_or_else(|| {
        Box::new(err(StatusCode::UNAUTHORIZED, "unknown key: make one with `xenify-chat-server app token`"))
    })
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
    let role = match db::role_of(&c, conv, me.id).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::FORBIDDEN, "this app isn't in that conversation"),
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "database error"),
    };
    if matches!(db::conv_info(&c, conv).await, Ok(Some((Kind::Channel, _)))) && role != Role::Owner && !me.app {
        return err(StatusCode::FORBIDDEN, "only the channel's owners and apps post here");
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
    let u =
        url::Url::parse(url).map_err(|_| "a webhook is a web address, like https://example.org/xenify".to_string())?;
    match u.scheme() {
        "https" => {}
        "http" if local_ok => {}
        _ => return Err("webhooks must use https://".into()),
    }
    let host = u.host_str().ok_or("that web address has no host")?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    if !local_ok {
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port))
            .await
            .map_err(|_| format!("{host} can't be found"))?
            .collect();
        if addrs.is_empty() || addrs.iter().any(|a| !public(a.ip())) {
            return Err(format!("{host} is on a private network; webhooks must be public addresses"));
        }
    }
    Ok(u.to_string())
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
                || (o[0] == 100 && (64..128).contains(&o[1])))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let first = v.segments()[0];
            !(v.is_loopback() || v.is_unspecified() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80)
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
                // Checked again: the address could have moved since it was saved.
                if !wanted || check_webhook(&app.webhook, s.config.local_webhooks).await.is_err() {
                    continue;
                }
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
                let url = app.webhook.clone();
                let reply = tokio::task::spawn_blocking(move || -> Option<String> {
                    let agent: ureq::Agent = ureq::Agent::config_builder()
                        .timeout_global(Some(std::time::Duration::from_secs(10)))
                        .http_status_as_error(false)
                        .max_redirects(0)
                        .build()
                        .into();
                    let mut r = agent
                        .post(&url)
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

async fn ws(State(s): State<Server>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let Ok(c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let Some(user) = db::session_user(&c, &hash_token(token)).await.ok().flatten() else {
        return err(StatusCode::UNAUTHORIZED, "sign in again");
    };
    drop(c);
    upgrade.max_message_size(256 * 1024).on_upgrade(move |socket| connection(s, user, socket))
}

/// One connected app.
async fn connection(s: Server, user: User, socket: WebSocket) {
    let (mut out, mut inp) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ToApp>();
    s.online.lock().unwrap().entry(user.id).or_default().push(tx.clone());
    let _ = tx.send(ToApp::Welcome { user: user.clone(), version: xenify_chat_proto::VERSION });
    // You're around now; and here's who else is.
    if let Ok(c) = s.pool.get().await {
        let _ = db::presence_change(&c, &s.id, user.id, 1).await;
        if let Ok(ids) = db::contacts(&c, user.id).await
            && let Ok(list) = db::presence_of(&c, &ids).await
        {
            let _ = tx.send(ToApp::Presence { list });
        }
    }
    s.notify(format!("o:{}", user.id)).await;
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let text = serde_json::to_string(&m).expect("serialises");
            if out.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(frame)) = inp.next().await {
        let WsMessage::Text(text) = frame else {
            if matches!(frame, WsMessage::Close(_)) {
                break;
            }
            continue;
        };
        let reply = match serde_json::from_str::<ToServer>(&text) {
            Ok(m) => handle(&s, &user, m).await,
            Err(e) => Some(ToApp::Error { code: "bad_request".into(), message: e.to_string() }),
        };
        if let Some(r) = reply
            && tx.send(r).is_err()
        {
            break;
        }
    }
    {
        let mut online = s.online.lock().unwrap();
        if let Some(list) = online.get_mut(&user.id) {
            list.retain(|x| !x.same_channel(&tx));
            if list.is_empty() {
                online.remove(&user.id);
            }
        }
    }
    writer.abort();
    if let Ok(c) = s.pool.get().await {
        let _ = db::presence_change(&c, &s.id, user.id, -1).await;
    }
    s.notify(format!("o:{}", user.id)).await;
}

fn error(code: &str, message: &str) -> ToApp {
    ToApp::Error { code: code.into(), message: message.into() }
}

fn failed(e: impl std::fmt::Display) -> ToApp {
    error("server", &format!("something went wrong on the server ({e})"))
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
        ToServer::CreateGroup { title, members } => {
            let ids = match user_ids(&c, &members).await {
                Ok(ids) => ids,
                Err(e) => return Some(*e),
            };
            let conv = match db::create(&mut c, Kind::Group, title.trim(), false, me.id, &ids).await {
                Ok(id) => id,
                Err(e) => return Some(failed(e)),
            };
            s.notify(format!("c:{conv}")).await;
            view!(conv)
        }
        ToServer::CreateChannel { title, open } => {
            if title.trim().is_empty() {
                return Some(error("bad_request", "a channel needs a name"));
            }
            let conv = match db::create(&mut c, Kind::Channel, title.trim(), open, me.id, &[]).await {
                Ok(id) => id,
                Err(e) => return Some(failed(e)),
            };
            view!(conv)
        }
        ToServer::Subscribe { conv } => {
            let (kind, open) = info!(conv);
            if kind != Kind::Channel || !open {
                return Some(error("not_open", "only open channels can be subscribed to"));
            }
            if let Err(e) = db::add_member(&c, conv, me.id).await {
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
            let ids = match user_ids(&c, &users).await {
                Ok(ids) => ids,
                Err(e) => return Some(*e),
            };
            for id in ids {
                if let Err(e) = db::add_member(&c, conv, id).await {
                    return Some(failed(e));
                }
            }
            s.notify(format!("c:{conv}")).await;
            None
        }
        ToServer::RemovePerson { conv, user } => {
            if role!(conv) != Role::Owner {
                return Some(error("not_owner", "only owners can take people out"));
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
            let role = role!(conv);
            let (kind, _) = info!(conv);
            if kind == Kind::Channel && role != Role::Owner && !me.app {
                return Some(error("not_allowed", "only the channel's owners post here: reply in a thread"));
            }
            if text.trim().is_empty() || text.len() > 64 * 1024 {
                return Some(error("bad_message", "messages are 1 to 65536 bytes"));
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
        ToServer::Edit { conv, seq, text } => {
            role!(conv);
            if text.trim().is_empty() || text.len() > 64 * 1024 {
                return Some(error("bad_message", "messages are 1 to 65536 bytes"));
            }
            edit(s, &c, me, conv, seq, Some(&text)).await
        }
        ToServer::Delete { conv, seq } => {
            role!(conv);
            edit(s, &c, me, conv, seq, None).await
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
            Some(match db::history(&c, conv, before, limit).await {
                Ok(messages) => ToApp::History { conv, messages, tag },
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
            Some(ToApp::Welcome {
                user: User { display_name: name, ..me.clone() },
                version: xenify_chat_proto::VERSION,
            })
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
                "UPDATE messages SET body = '', deleted = TRUE WHERE conv = $1 AND seq = $2 AND sender = $3 RETURNING id",
                &[&conv, &seq, &me.id],
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

/// A new invite code, good for `days` days.
pub async fn invite(pool: &Pool, days: i32) -> Result<String, String> {
    let code = random(9);
    let c = pool.get().await.map_err(|e| e.to_string())?;
    c.execute("INSERT INTO invites (code, expires) VALUES ($1, now() + make_interval(days => $2))", &[&code, &days])
        .await
        .map_err(|e| e.to_string())?;
    Ok(code)
}
