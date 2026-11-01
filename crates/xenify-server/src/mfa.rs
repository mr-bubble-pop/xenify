//! Two-step sign-in: after the password, a code from an authenticator app
//! (RFC 6238, the usual six digits every 30 seconds), one emailed then, or
//! a one-time backup code. Admin › Settings can require it of admins or of
//! everyone; then signing in sets it up first.
//!
//! - A ticket (10 minutes, a few tries) carries someone between the password
//!   and the code, or through setting it up.
//! - "Remember this computer" keeps a token (hashed here) that skips the
//!   step there for 30 days.
//! - Emailed codes are a weaker second step when email is also how a
//!   password is reset, so admins can't use them alone.

use crate::accounts::new_session;
use crate::{Server, db, err, failed, hash_token, password_ok, random, signed_in, unavailable};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::{Client, GenericClient};
use serde::Deserialize;
use xenify_server_proto::{Account, AppSecret, SecondCode, SecondStep, SecondStepReady, SecondStepState};

pub const SCHEMA: &str = "
ALTER TABLE users ADD COLUMN IF NOT EXISTS mfa_secret TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS mfa_email BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN IF NOT EXISTS mfa_backup TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE users ADD COLUMN IF NOT EXISTS mfa_last_step BIGINT NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS mfa_tickets (
    hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device TEXT NOT NULL DEFAULT '',
    -- An emailed code waiting, or a new authenticator secret being set up.
    code_hash TEXT,
    setup_secret TEXT,
    tries INT NOT NULL DEFAULT 0,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS mfa_remember (
    hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires TIMESTAMPTZ NOT NULL
);
";

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/login/code", post(login_code))
        .route("/v1/login/email", post(login_email))
        .route("/v1/mfa/app/start", post(app_start))
        .route("/v1/mfa/app/finish", post(app_finish))
        .route("/v1/mfa/email", post(email_setup))
        .route("/v1/me/mfa", get(state))
        .route("/v1/me/mfa/off", post(turn_off))
        .route("/v1/me/mfa/backup", post(new_backup))
        .route("/v1/admin/people/{id}/mfa", axum::routing::delete(admin_clear))
}

/// How long a ticket works, and how many wrong codes it takes.
const TICKET_MINUTES: i32 = 10;
const TICKET_TRIES: i32 = 5;
const REMEMBER_DAYS: i32 = 30;

// ---- Authenticator codes (RFC 6238 over RFC 4226) -------------------------

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0);
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(s: &str) -> Option<Vec<u8>> {
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '=') {
        let v = B32.iter().position(|&b| b as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

/// The six digits for 30-second step `step`.
pub(crate) fn totp(secret: &[u8], step: u64) -> String {
    use hmac::{KeyInit, Mac};
    let mut m = hmac::Hmac::<sha1::Sha1>::new_from_slice(secret).expect("any key length");
    m.update(&step.to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    let n = (u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) & 0x7fff_ffff) % 1_000_000;
    format!("{n:06}")
}

fn now_step() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() / 30
}

/// The step `code` is right for (now, or one step either side), if any.
fn app_code_step(secret: &str, code: &str) -> Option<u64> {
    let key = unbase32(secret)?;
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    let now = now_step();
    [now, now - 1, now + 1].into_iter().find(|&s| totp(&key, s) == code)
}

fn qr_svg(text: &str) -> String {
    use qrcodegen::{QrCode, QrCodeEcc};
    let Ok(qr) = QrCode::encode_text(text, QrCodeEcc::Medium) else { return String::new() };
    let (size, border) = (qr.size(), 2);
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + border, y + border));
            }
        }
    }
    let n = size + border * 2;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n} {n}" shape-rendering="crispEdges" role="img" aria-label="QR code for the authenticator app"><rect width="100%" height="100%" fill="#fff"/><path d="{path}" fill="#000"/></svg>"##
    )
}

/// Ten one-time backup codes ("k7qm-2x9p"), and their hashes to keep.
fn backup_codes() -> (Vec<String>, Vec<String>) {
    let alphabet = b"abcdefghjkmnpqrstuvwxyz23456789";
    let codes: Vec<String> = (0..10)
        .map(|_| {
            let r = xenify_sync::random(8);
            let c: String = r.iter().map(|b| alphabet[*b as usize % alphabet.len()] as char).collect();
            format!("{}-{}", &c[..4], &c[4..])
        })
        .collect();
    let hashes = codes.iter().map(|c| hash_token(c)).collect();
    (codes, hashes)
}

fn clean_backup(code: &str) -> String {
    let c: String = code.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if c.len() == 8 { format!("{}-{}", &c[..4], &c[4..]) } else { c }
}

// ---- Who needs what --------------------------------------------------------

/// "off", "admins" or "everyone".
pub(crate) async fn required(c: &impl GenericClient) -> String {
    db::setting(c, "mfa_required")
        .await
        .ok()
        .flatten()
        .filter(|r| ["admins", "everyone"].contains(&r.as_str()))
        .unwrap_or_else(|| "off".into())
}

fn required_of(rule: &str, admin: bool) -> bool {
    rule == "everyone" || (rule == "admins" && admin)
}

struct Mfa {
    secret: Option<String>,
    email: bool,
    backup: Vec<String>,
}

async fn mfa_of(c: &impl GenericClient, user: i64) -> Option<Mfa> {
    let r = c.query_opt("SELECT mfa_secret, mfa_email, mfa_backup FROM users WHERE id = $1", &[&user]).await.ok()??;
    Some(Mfa { secret: r.get(0), email: r.get(1), backup: r.get(2) })
}

/// What someone may set up: an authenticator app always; emailed codes
/// for people who aren't admins, when the server sends email and they
/// have an address.
async fn setup_methods(c: &impl GenericClient, a: &Account) -> Vec<String> {
    let mut m = vec!["app".to_string()];
    if !a.admin && !a.email.is_empty() && crate::mail::mail(c).await.is_some() {
        m.push("email".into());
    }
    m
}

/// A second step is set up (backup codes alone don't count).
fn has_step(m: &Mfa, a: &Account) -> bool {
    m.secret.is_some() || (m.email && !a.email.is_empty())
}

fn methods(m: &Mfa, a: &Account) -> Vec<String> {
    let mut out = Vec::new();
    if m.secret.is_some() {
        out.push("app".to_string());
    }
    if m.email && !a.email.is_empty() {
        out.push("email".into());
    }
    if !m.backup.is_empty() {
        out.push("backup".into());
    }
    out
}

async fn new_ticket(c: &impl GenericClient, user: i64, device: &str) -> Result<String, tokio_postgres::Error> {
    let t = random(24);
    c.execute("DELETE FROM mfa_tickets WHERE created < now() - make_interval(mins => $1)", &[&TICKET_MINUTES]).await?;
    c.execute(
        "INSERT INTO mfa_tickets (hash, user_id, device) VALUES ($1, $2, $3)",
        &[&hash_token(&t), &user, &device.chars().take(64).collect::<String>()],
    )
    .await?;
    Ok(t)
}

/// After the right password (or a link): a session, or the second step.
/// `remember` is a token from an earlier "remember this computer".
pub(crate) async fn after_password(c: &Client, account: Account, device: &str, remember: &str) -> Response {
    let Some(m) = mfa_of(c, account.id).await else { return failed() };
    let has = has_step(&m, &account);
    let rule = required(c).await;
    if !has && !required_of(&rule, account.admin) {
        return new_session(c, account, device).await;
    }
    if has && !remember.is_empty() {
        let ok = c
            .query_opt(
                "SELECT 1 FROM mfa_remember WHERE hash = $1 AND user_id = $2 AND expires > now()",
                &[&hash_token(remember), &account.id],
            )
            .await
            .ok()
            .flatten()
            .is_some();
        if ok {
            return new_session(c, account, device).await;
        }
    }
    let Ok(ticket) = new_ticket(c, account.id, device).await else { return failed() };
    Json(SecondStep {
        ticket,
        methods: if has { methods(&m, &account) } else { Vec::new() },
        setup: !has,
        setup_methods: if has { Vec::new() } else { setup_methods(c, &account).await },
    })
    .into_response()
}

/// A ticket's account, while it works; a wrong try is counted by the caller.
struct Ticket {
    hash: String,
    account: Account,
    device: String,
    code_hash: Option<String>,
    setup_secret: Option<String>,
}

async fn ticket(c: &impl GenericClient, t: &str) -> Option<Ticket> {
    let r = c
        .query_opt(
            "SELECT t.hash, u.id, u.username, u.display_name, u.admin, u.email, t.device, t.code_hash, t.setup_secret
             FROM mfa_tickets t JOIN users u ON u.id = t.user_id
             WHERE t.hash = $1 AND NOT u.disabled AND t.tries < $2 AND t.created > now() - make_interval(mins => $3)",
            &[&hash_token(t), &TICKET_TRIES, &TICKET_MINUTES],
        )
        .await
        .ok()??;
    Some(Ticket {
        hash: r.get(0),
        account: Account { id: r.get(1), username: r.get(2), display_name: r.get(3), admin: r.get(4), email: r.get(5) },
        device: r.get(6),
        code_hash: r.get(7),
        setup_secret: r.get(8),
    })
}

fn bad_ticket() -> Response {
    err(StatusCode::UNAUTHORIZED, "bad_ticket", "that took too long or had too many tries: sign in again")
}

fn too_many() -> Response {
    err(StatusCode::TOO_MANY_REQUESTS, "too_many", "too many tries: wait a few minutes and try again")
}

fn wrong_code() -> Response {
    err(StatusCode::UNAUTHORIZED, "wrong_code", "that code isn't right")
}

async fn wrong_try(c: &impl GenericClient, hash: &str) {
    let _ = c.execute("UPDATE mfa_tickets SET tries = tries + 1 WHERE hash = $1", &[&hash]).await;
}

/// Signs in with a ticket that passed: a session (and a remember token).
async fn finish(c: &Client, t: &Ticket, remember: bool) -> Response {
    let _ = c.execute("DELETE FROM mfa_tickets WHERE hash = $1", &[&t.hash]).await;
    let session = new_session(c, t.account.clone(), &t.device).await;
    if !remember || !session.status().is_success() {
        return session;
    }
    let token = random(24);
    if c.execute(
        "INSERT INTO mfa_remember (hash, user_id, expires) VALUES ($1, $2, now() + make_interval(days => $3))",
        &[&hash_token(&token), &t.account.id, &REMEMBER_DAYS],
    )
    .await
    .is_err()
    {
        return session;
    }
    let Ok(body) = axum::body::to_bytes(session.into_body(), 1 << 16).await else { return failed() };
    let Ok(mut s) = serde_json::from_slice::<xenify_server_proto::Session>(&body) else { return failed() };
    s.remember = Some(token);
    Json(s).into_response()
}

/// The limiter's key for wrong codes and passwords from someone who
/// already has the password or is signed in, so they add up across
/// tickets. Strangers' wrong passwords don't count on it, so they can't
/// lock anyone out of this step.
fn user_key(a: &Account) -> Vec<String> {
    crate::accounts::own_key(a.id)
}

async fn login_code(State(s): State<Server>, Json(b): Json<SecondCode>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some(t) = ticket(&c, &b.ticket).await else { return bad_ticket() };
    // Each password gives a few tries; this keeps a known password from
    // buying endless ones.
    let keys = user_key(&t.account);
    if s.limit.blocked(&keys) {
        return too_many();
    }
    let Some(m) = mfa_of(&c, t.account.id).await else { return failed() };
    // Each check-and-use is one statement, so a code works only once even
    // when two arrive together.
    let ok = match b.method.as_str() {
        "app" => match m.secret.as_deref().and_then(|sec| app_code_step(sec, &b.code)) {
            Some(step) => c
                .execute(
                    "UPDATE users SET mfa_last_step = $2 WHERE id = $1 AND mfa_last_step < $2",
                    &[&t.account.id, &(step as i64)],
                )
                .await
                .is_ok_and(|n| n == 1),
            None => false,
        },
        "email" => m.email && t.code_hash.as_deref() == Some(hash_token(b.code.trim()).as_str()),
        "backup" => {
            let h = hash_token(&clean_backup(&b.code));
            c.execute(
                "UPDATE users SET mfa_backup = array_remove(mfa_backup, $2) WHERE id = $1 AND $2 = ANY(mfa_backup)",
                &[&t.account.id, &h],
            )
            .await
            .is_ok_and(|n| n == 1)
        }
        _ => false,
    };
    if !ok {
        s.limit.failed(&keys);
        wrong_try(&c, &t.hash).await;
        return wrong_code();
    }
    finish(&c, &t, b.remember).await
}

#[derive(Deserialize)]
struct TicketOnly {
    ticket: String,
}

/// Six digits, kept hashed on the ticket.
async fn send_code(c: &Client, t: &Ticket) -> Result<(), String> {
    let code = format!("{:06}", u32::from_be_bytes(xenify_sync::random(4).try_into().unwrap()) % 1_000_000);
    c.execute("UPDATE mfa_tickets SET code_hash = $2 WHERE hash = $1", &[&t.hash, &hash_token(&code)])
        .await
        .map_err(|e| e.to_string())?;
    let server = db::setting(c, "name").await.ok().flatten().unwrap_or_default();
    let e = crate::mail::Email {
        subject: format!("Your code for {server}: {code}"),
        heading: format!("Your code is {code}"),
        lines: vec![format!("Type it where you're signing in to {server}.")],
        button: None,
        small: vec![
            "It works for 10 minutes.".into(),
            "If you didn't just sign in, someone has your password: change it.".into(),
        ],
    };
    crate::mail::send(c, &t.account.email, e).await
}

/// "Email me a code" while signing in.
async fn login_email(State(s): State<Server>, Json(b): Json<TicketOnly>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some(t) = ticket(&c, &b.ticket).await else { return bad_ticket() };
    let Some(m) = mfa_of(&c, t.account.id).await else { return failed() };
    if !m.email || t.account.email.is_empty() {
        return err(StatusCode::BAD_REQUEST, "no_email", "codes by email aren't set up for this account");
    }
    // Each one counts, so nobody fills someone's inbox.
    let keys = vec![format!("mail:{}", t.account.id)];
    if s.limit.blocked(&keys) {
        return too_many();
    }
    s.limit.failed(&keys);
    match send_code(&c, &t).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "mail_failed", &e),
    }
}

// ---- Setting it up ---------------------------------------------------------
//
// Signed in (Settings › Your account), or with a ticket while signing in
// when the server requires it.

#[derive(Deserialize, Default)]
struct SetupBody {
    #[serde(default)]
    ticket: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    remember: bool,
    /// Replacing a second step that's set up: the password, or a code from
    /// it (`current`).
    #[serde(default)]
    password: String,
    #[serde(default)]
    current: String,
}

/// Who's setting up: signed in, or a sign-in ticket.
enum Who {
    Me(Account),
    Ticket(Ticket),
}

impl Who {
    fn account(&self) -> &Account {
        match self {
            Who::Me(a) => a,
            Who::Ticket(t) => &t.account,
        }
    }
}

/// A ticket sets up a second step only for someone who has none yet:
/// otherwise the password alone could replace theirs and sign in.
async fn who(s: &Server, c: &Client, h: &HeaderMap, ticket_text: &str) -> Result<Who, Response> {
    if !ticket_text.is_empty() {
        let t = ticket(c, ticket_text).await.ok_or_else(bad_ticket)?;
        let Some(m) = mfa_of(c, t.account.id).await else { return Err(failed()) };
        if has_step(&m, &t.account) {
            return Err(err(StatusCode::FORBIDDEN, "set_up", "a second step is set up already: sign in with it"));
        }
        return Ok(Who::Ticket(t));
    }
    signed_in(s, h).await.map(Who::Me)
}

/// Someone signed in who already has a second step replaces it only with
/// their password or a code from it: a session alone (a computer left
/// signed in) isn't enough.
async fn may_replace(s: &Server, c: &Client, w: &Who, b: &SetupBody) -> Result<(), Response> {
    let Who::Me(me) = w else { return Ok(()) };
    let Some(m) = mfa_of(c, me.id).await else { return Err(failed()) };
    if !has_step(&m, me) {
        return Ok(());
    }
    if !b.password.is_empty() {
        return check_password(s, c, me, &b.password).await;
    }
    if !b.current.is_empty() {
        let keys = user_key(me);
        if s.limit.blocked(&keys) {
            return Err(too_many());
        }
        let app = match m.secret.as_deref().and_then(|sec| app_code_step(sec, &b.current)) {
            Some(step) => c
                .execute(
                    "UPDATE users SET mfa_last_step = $2 WHERE id = $1 AND mfa_last_step < $2",
                    &[&me.id, &(step as i64)],
                )
                .await
                .is_ok_and(|n| n == 1),
            None => false,
        };
        let backup = !app
            && c.execute(
                "UPDATE users SET mfa_backup = array_remove(mfa_backup, $2) WHERE id = $1 AND $2 = ANY(mfa_backup)",
                &[&me.id, &hash_token(&clean_backup(&b.current))],
            )
            .await
            .is_ok_and(|n| n == 1);
        if app || backup {
            return Ok(());
        }
        s.limit.failed(&keys);
        return Err(wrong_code());
    }
    Err(err(
        StatusCode::FORBIDDEN,
        "password_needed",
        "a second step is set up already: type your password to change it",
    ))
}

/// A new authenticator secret: kept on the ticket (or a fresh one for
/// someone signed in) until a code from it comes back.
async fn app_start(State(s): State<Server>, h: HeaderMap, body: Option<Json<SetupBody>>) -> Response {
    let b = body.map(|b| b.0).unwrap_or_default();
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let w = match who(&s, &c, &h, &b.ticket).await {
        Ok(w) => w,
        Err(e) => return e,
    };
    if let Err(e) = may_replace(&s, &c, &w, &b).await {
        return e;
    }
    let secret = base32(&xenify_sync::random(20));
    let hash = match &w {
        Who::Ticket(t) => t.hash.clone(),
        Who::Me(a) => match new_ticket(&c, a.id, "").await {
            Ok(t) => hash_token(&t),
            Err(_) => return failed(),
        },
    };
    if c.execute("UPDATE mfa_tickets SET setup_secret = $2 WHERE hash = $1", &[&hash, &secret]).await.is_err() {
        return failed();
    }
    let server = db::setting(&c, "name").await.ok().flatten().unwrap_or_else(|| "Xenify".into());
    let label = |s: &str| s.replace(['/', '?', '#', '&', ':'], " ").replace(' ', "%20");
    let a = w.account();
    let who_label = if a.email.is_empty() { a.username.clone() } else { a.email.clone() };
    let uri =
        format!("otpauth://totp/{}:{}?secret={secret}&issuer={}", label(&server), label(&who_label), label(&server));
    Json(AppSecret { qr_svg: qr_svg(&uri), secret, uri }).into_response()
}

/// The newest secret set up for `user` and not yet confirmed.
async fn pending_secret(c: &Client, w: &Who) -> Option<(String, String)> {
    match w {
        Who::Ticket(t) => t.setup_secret.clone().map(|s| (t.hash.clone(), s)),
        Who::Me(a) => {
            let r = c
                .query_opt(
                    "SELECT hash, setup_secret FROM mfa_tickets WHERE user_id = $1 AND setup_secret IS NOT NULL
                     AND created > now() - make_interval(mins => $2) ORDER BY created DESC LIMIT 1",
                    &[&a.id, &TICKET_MINUTES],
                )
                .await
                .ok()??;
            Some((r.get(0), r.get(1)))
        }
    }
}

/// Turns the second step on: new backup codes, and (with a ticket) signs in.
async fn ready(c: &Client, w: Who, remember: bool) -> Response {
    let (codes, hashes) = backup_codes();
    if c.execute("UPDATE users SET mfa_backup = $2 WHERE id = $1", &[&w.account().id, &hashes]).await.is_err() {
        return failed();
    }
    let session = match &w {
        Who::Me(_) => None,
        Who::Ticket(t) => {
            let r = finish(c, t, remember).await;
            let Ok(body) = axum::body::to_bytes(r.into_body(), 1 << 16).await else { return failed() };
            serde_json::from_slice(&body).ok()
        }
    };
    Json(SecondStepReady { backup_codes: codes, session }).into_response()
}

async fn app_finish(State(s): State<Server>, h: HeaderMap, Json(b): Json<SetupBody>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let w = match who(&s, &c, &h, &b.ticket).await {
        Ok(w) => w,
        Err(e) => return e,
    };
    let Some((hash, secret)) = pending_secret(&c, &w).await else {
        return err(StatusCode::BAD_REQUEST, "not_started", "start again: scan the code first");
    };
    let Some(step) = app_code_step(&secret, &b.code) else {
        wrong_try(&c, &hash).await;
        return wrong_code();
    };
    if c.execute(
        "UPDATE users SET mfa_secret = $2, mfa_last_step = $3 WHERE id = $1",
        &[&w.account().id, &secret, &(step as i64)],
    )
    .await
    .is_err()
    {
        return failed();
    }
    if matches!(w, Who::Me(_)) {
        let _ = c.execute("DELETE FROM mfa_tickets WHERE hash = $1", &[&hash]).await;
    }
    ready(&c, w, b.remember).await
}

/// Codes by email: without `code`, sends one; with it, turns them on.
async fn email_setup(State(s): State<Server>, h: HeaderMap, Json(b): Json<SetupBody>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let w = match who(&s, &c, &h, &b.ticket).await {
        Ok(w) => w,
        Err(e) => return e,
    };
    // Sending the code is where it starts (confirming it needs that code).
    if b.code.is_empty()
        && let Err(e) = may_replace(&s, &c, &w, &b).await
    {
        return e;
    }
    if !setup_methods(&c, w.account()).await.iter().any(|m| m == "email") {
        return err(
            StatusCode::FORBIDDEN,
            "not_allowed",
            "codes by email need an email address and a server that sends email, and aren't for admins",
        );
    }
    let t = match &w {
        Who::Ticket(t) => clone_ticket(t),
        Who::Me(a) => {
            // The ticket this setup goes on: the newest one, or a new one.
            let existing = c
                .query_opt(
                    "SELECT hash FROM mfa_tickets WHERE user_id = $1 AND created > now() - make_interval(mins => $2)
                     ORDER BY created DESC LIMIT 1",
                    &[&a.id, &TICKET_MINUTES],
                )
                .await
                .ok()
                .flatten()
                .map(|r| r.get::<_, String>(0));
            let hash = match existing {
                Some(h) if !b.code.is_empty() => h,
                _ => match new_ticket(&c, a.id, "").await {
                    Ok(t) => hash_token(&t),
                    Err(_) => return failed(),
                },
            };
            let code_hash = c
                .query_opt("SELECT code_hash FROM mfa_tickets WHERE hash = $1", &[&hash])
                .await
                .ok()
                .flatten()
                .and_then(|r| r.get(0));
            Ticket { hash, account: a.clone(), device: String::new(), code_hash, setup_secret: None }
        }
    };
    if b.code.is_empty() {
        return match send_code(&c, &t).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => err(StatusCode::BAD_GATEWAY, "mail_failed", &e),
        };
    }
    if t.code_hash.as_deref() != Some(hash_token(b.code.trim()).as_str()) {
        wrong_try(&c, &t.hash).await;
        return wrong_code();
    }
    if c.execute("UPDATE users SET mfa_email = TRUE WHERE id = $1", &[&t.account.id]).await.is_err() {
        return failed();
    }
    if matches!(w, Who::Me(_)) {
        let _ = c.execute("DELETE FROM mfa_tickets WHERE hash = $1", &[&t.hash]).await;
    }
    ready(&c, w, b.remember).await
}

fn clone_ticket(t: &Ticket) -> Ticket {
    Ticket {
        hash: t.hash.clone(),
        account: t.account.clone(),
        device: t.device.clone(),
        code_hash: t.code_hash.clone(),
        setup_secret: t.setup_secret.clone(),
    }
}

async fn state(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some(m) = mfa_of(&c, me.id).await else { return failed() };
    Json(SecondStepState {
        app: m.secret.is_some(),
        email: m.email,
        backup_left: m.backup.len() as u32,
        required: required_of(&required(&c).await, me.admin),
        setup_methods: setup_methods(&c, &me).await,
    })
    .into_response()
}

#[derive(Deserialize)]
struct PasswordOnly {
    password: String,
}

async fn check_password(s: &Server, c: &Client, me: &Account, password: &str) -> Result<(), Response> {
    let keys = user_key(me);
    if s.limit.blocked(&keys) {
        return Err(too_many());
    }
    let Ok(Some(l)) = db::login_of(c, &me.username).await else { return Err(failed()) };
    if password_ok(password, &l.hash).await {
        Ok(())
    } else {
        s.limit.failed(&keys);
        Err(err(StatusCode::UNAUTHORIZED, "wrong", "your password isn't right"))
    }
}

/// Turns the second step off (not when the server requires it).
async fn turn_off(State(s): State<Server>, h: HeaderMap, Json(b): Json<PasswordOnly>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if let Err(e) = check_password(&s, &c, &me, &b.password).await {
        return e;
    }
    if required_of(&required(&c).await, me.admin) {
        return err(StatusCode::FORBIDDEN, "required", "this server requires a second step");
    }
    match clear(&c, me.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

pub(crate) async fn clear(c: &impl GenericClient, user: i64) -> Result<(), tokio_postgres::Error> {
    c.execute(
        "UPDATE users SET mfa_secret = NULL, mfa_email = FALSE, mfa_backup = '{}', mfa_last_step = 0 WHERE id = $1",
        &[&user],
    )
    .await?;
    c.execute("DELETE FROM mfa_remember WHERE user_id = $1", &[&user]).await?;
    Ok(())
}

/// New backup codes (the old ones stop working).
async fn new_backup(State(s): State<Server>, h: HeaderMap, Json(b): Json<PasswordOnly>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if let Err(e) = check_password(&s, &c, &me, &b.password).await {
        return e;
    }
    let Some(m) = mfa_of(&c, me.id).await else { return failed() };
    if m.secret.is_none() && !m.email {
        return err(StatusCode::BAD_REQUEST, "not_set_up", "set up a second step first");
    }
    ready(&c, Who::Me(me), false).await
}

/// An admin clears someone's second step (a lost phone): they sign in with
/// their password and set it up again if the server requires it.
async fn admin_clear(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let me = match crate::admin_power(&s, &h, "admin.people").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if !crate::perms::may_manage(&c, &me, id).await {
        return crate::perms::full_admins_only();
    }
    match clear(&c, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn codes_match_the_rfc() {
        // RFC 6238's SHA-1 key, at 59 s: 94287082 (the last six digits).
        let key = b"12345678901234567890";
        assert_eq!(super::totp(key, 59 / 30), "287082");
        assert_eq!(super::totp(key, 1111111109 / 30), "081804");
        let b = super::base32(key);
        assert_eq!(b, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(super::unbase32(&b.to_lowercase()).unwrap(), key);
        assert_eq!(super::clean_backup(" K7QM 2X9P "), "k7qm-2x9p");
    }
}
