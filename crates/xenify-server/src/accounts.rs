//! Signing in (the user name, then the password), setting the server up,
//! one-time links (welcome, reset) and changing passwords. Accounts are
//! made by admins; nobody joins by themselves.
//!
//! An account without a password yet (new, or reset by an admin) and a
//! forgotten password go the same way: the user name, then a one-time code
//! (emailed now, or given by an admin when the server can't send email),
//! then a new password, and they're signed in. Only a hash of a code is
//! kept (`invites.code_hash`; `invites.code` is a random id). Older
//! `<server>/join/#<id>.<secret>` links still work until they run out.

use crate::limit::{self, SignIn, name_key};
use crate::{
    Peer, Server, clean_name, db, err, failed, hash_password, hash_token, password_ok, random, signed_in, unavailable,
    valid_username,
};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::{Client, GenericClient};
use xenify_server_proto::{
    Account, ChangePassword, LinkCheck, LinkInfo, LinkKind, LinkUse, Login, LoginNext, LoginStart, LoginStep, Setup,
};

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/login/start", post(login_start))
        .route("/v1/login", post(login))
        .route("/v1/setup", post(setup))
        .route("/v1/link", post(check_link))
        .route("/v1/link/use", post(use_link))
        .route("/v1/forgot", post(forgot))
        .route("/v1/me", get(me))
        .route("/v1/password", post(change_password))
        .route("/v1/signout", post(sign_out))
        .route("/v1/signout-everywhere", post(sign_out_everywhere))
}

fn wrong() -> Response {
    err(StatusCode::UNAUTHORIZED, "wrong", "wrong email address or password")
}

fn too_many() -> Response {
    err(StatusCode::TOO_MANY_REQUESTS, "too_many", "too many tries: wait a few minutes and try again")
}

fn bad_link() -> Response {
    err(StatusCode::UNAUTHORIZED, "bad_link", "that code isn't right, or it was used or ran out")
}

/// What a sign-in counts on: where it came from, and the account (its id
/// when there is one, else the name typed, hashed; no name: the address
/// alone).
async fn keys_for(s: &Server, c: &impl GenericClient, peer: &Peer, headers: &HeaderMap, user: &str) -> SignIn {
    let ip = s.client_ip(peer.0.ip(), headers).await;
    let name = user.trim().to_lowercase();
    if name.is_empty() {
        return SignIn::new(ip, None);
    }
    let id = c
        .query_opt("SELECT id FROM users WHERE username = $1 OR (strpos($1, '@') > 0 AND lower(email) = $1)", &[&name])
        .await
        .ok()
        .flatten()
        .map(|r| r.get::<_, i64>(0));
    SignIn::new(ip, Some(id.map(|id| format!("acct:{id}")).unwrap_or_else(|| name_key(&name))))
}

/// The key wrong passwords and codes from someone already signed in (or
/// holding the password) count on.
pub(crate) fn own_key(account: i64) -> Vec<String> {
    vec![format!("own:{account}")]
}

/// Tells Chat (sockets, apps, calls) that `user`'s sessions changed, so
/// they check again at once.
pub(crate) async fn sessions_changed(c: &impl GenericClient, user: i64) {
    let _ = c.execute("SELECT pg_notify('xenify_chat', $1)", &[&format!("k:{user}")]).await;
}

/// Ends "remember this computer" for `user` (a new password, signing out
/// everywhere).
pub(crate) async fn forget_computers(c: &impl GenericClient, user: i64) -> Result<(), tokio_postgres::Error> {
    c.execute("DELETE FROM mfa_remember WHERE user_id = $1", &[&user]).await.map(|_| ())
}

/// Passwords: at least 8 characters, at most 200.
fn check_password(p: &str) -> Result<(), Response> {
    let n = p.chars().count();
    if n < 8 {
        return Err(err(StatusCode::BAD_REQUEST, "short_password", "passwords need at least 8 characters"));
    }
    if n > 200 {
        return Err(err(StatusCode::BAD_REQUEST, "long_password", "passwords can have at most 200 characters"));
    }
    Ok(())
}

async fn set_password(c: &impl GenericClient, user: i64, password: &str) -> Result<(), tokio_postgres::Error> {
    c.execute("UPDATE users SET password_hash = $2 WHERE id = $1", &[&user, &hash_password(password).await]).await?;
    Ok(())
}

pub(crate) async fn new_session(c: &Client, account: Account, device: &str) -> Response {
    let token = random(32);
    let mut device: String = device.chars().take(64).collect();
    // Those names mark Chat's keys, which aren't sign-ins.
    if db::KEY_DEVICES.contains(&device.as_str()) {
        device = format!("{device} (signed in)");
    }
    if c.execute(
        "INSERT INTO sessions (token_hash, user_id, device) VALUES ($1, $2, $3)",
        &[&hash_token(&token), &account.id, &device],
    )
    .await
    .is_err()
    {
        return failed();
    }
    Json(xenify_server_proto::Session { token, account, remember: None }).into_response()
}

/// The user name first: then the password, or, for an account an admin
/// added that has no password yet, picking one with the admin's code (or
/// a link emailed now). Names nobody has get "password", like everyone.
async fn login_start(State(s): State<Server>, peer: Peer, headers: HeaderMap, Json(l): Json<LoginStart>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let keys = keys_for(&s, &c, &peer, &headers, &l.username).await;
    if s.limit.sign_in_blocked(&keys) {
        return too_many();
    }
    let found = db::login_of(&c, &l.username.trim().to_lowercase()).await.ok().flatten();
    let step = match found {
        Some(f) if f.hash.is_empty() && !f.disabled => LoginStep {
            next: LoginNext::PickPassword,
            can_email: !f.account.email.is_empty() && crate::mail::mail(&c).await.is_some(),
        },
        _ => LoginStep { next: LoginNext::Password, can_email: false },
    };
    Json(step).into_response()
}

async fn login(State(s): State<Server>, peer: Peer, headers: HeaderMap, Json(l): Json<Login>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let keys = keys_for(&s, &c, &peer, &headers, &l.username).await;
    if s.limit.sign_in_blocked(&keys) {
        return too_many();
    }
    let found = db::login_of(&c, &l.username.trim().to_lowercase()).await.ok().flatten();
    // One hash checked whether or not there's such an account (or a
    // password yet), so the time taken doesn't tell.
    let hash = found.as_ref().map(|f| f.hash.as_str()).unwrap_or("");
    let ok = password_ok(&l.password, hash).await && found.as_ref().is_some_and(|f| !f.disabled);
    if !ok {
        s.limit.sign_in_failed(&keys);
        return wrong();
    }
    crate::mfa::after_password(&c, found.unwrap().account, &l.device, &l.remember).await
}

fn bad_name() -> Response {
    err(StatusCode::BAD_REQUEST, "bad_name", "user names are 2–32 letters, digits, dots, dashes or underscores")
}

async fn setup(State(s): State<Server>, peer: Peer, headers: HeaderMap, Json(p): Json<Setup>) -> Response {
    let keys = vec![crate::limit::ip_key(s.client_ip(peer.0.ip(), &headers).await)];
    if s.limit.blocked(&keys) {
        return too_many();
    }
    // Taking the token makes sure only one setup can happen.
    let token = {
        let mut t = s.setup_token.lock().unwrap();
        match t.as_deref() {
            None => return err(StatusCode::CONFLICT, "set_up", "this server is already set up"),
            // Compared as hashes, so the time taken says nothing about it.
            Some(t0) if hash_token(t0) != hash_token(p.token.trim()) => {
                s.limit.failed(&keys);
                return err(
                    StatusCode::FORBIDDEN,
                    "bad_token",
                    "that setup address is out of date: use the one the server shows",
                );
            }
            Some(_) => t.take().unwrap(),
        }
    };
    let give_back = || *s.setup_token.lock().unwrap() = Some(token.clone());
    if let Err(e) = check_password(&p.password) {
        give_back();
        return e;
    }
    let Some(email) = crate::admin::clean_email(&p.email) else {
        give_back();
        return crate::admin::bad_email();
    };
    // Signing in is with the email address; the user name underneath is
    // made from it (older callers may still give one).
    if p.username.trim().is_empty() && email.is_empty() {
        give_back();
        return err(StatusCode::BAD_REQUEST, "no_email", "type your email address");
    }
    let Ok(mut c) = s.pool.get().await else {
        give_back();
        return unavailable();
    };
    let name = if p.username.trim().is_empty() {
        crate::admin::username_for(&c, &email).await.unwrap_or_default()
    } else {
        p.username.trim().to_lowercase()
    };
    if !valid_username(&name) {
        give_back();
        return bad_name();
    }
    let hash = hash_password(&p.password).await;
    let done = async {
        let tx = c.transaction().await?;
        let id: i64 = tx
            .query_one(
                "INSERT INTO users (username, display_name, email, password_hash, admin) VALUES ($1, $2, $3, $4, TRUE) RETURNING id",
                &[&name, &clean_name(&p.display_name), &email, &hash],
            )
            .await?
            .get(0);
        let server_name = clean_name(&p.server_name);
        db::set_setting(&tx, "name", if server_name.is_empty() { "Xenify server" } else { &server_name }).await?;
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(Account {
            id,
            username: name.clone(),
            display_name: clean_name(&p.display_name),
            admin: true,
            email: email.clone(),
        })
    }
    .await;
    match done {
        Ok(account) => {
            // The setup link is used up.
            s.write_address();
            new_session(&c, account, &p.device).await
        }
        Err(_) => {
            give_back();
            failed()
        }
    }
}

/// A new one-time code for `for_user` (`len` letters, in fours), and when
/// it runs out. Any earlier unused one of theirs stops working.
pub(crate) async fn make_code(
    c: &impl GenericClient,
    kind: LinkKind,
    for_user: i64,
    made_by: Option<i64>,
    len: usize,
    minutes: u32,
) -> Result<(String, String), tokio_postgres::Error> {
    // A new code replaces the earlier ones, except that an emailed code
    // (short-lived) replaces only emailed ones: signing in mustn't spoil a
    // code an admin (or the server's own command) gave out. One used ends
    // the rest (see use_link).
    if minutes > EMAIL_CODE_MINUTES {
        c.execute("DELETE FROM invites WHERE for_user = $1 AND used_by IS NULL", &[&for_user]).await?;
    } else {
        c.execute(
            "DELETE FROM invites WHERE for_user = $1 AND used_by IS NULL
             AND expires <= now() + make_interval(mins => $2)",
            &[&for_user, &(EMAIL_CODE_MINUTES as i32 + 1)],
        )
        .await?;
    }
    // No 0/O or 1/I, so it's easy to read out and type.
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes = xenify_sync::random(len);
    let mut code = String::new();
    for (n, b) in bytes.iter().enumerate() {
        if n > 0 && n % 4 == 0 {
            code.push('-');
        }
        code.push(LETTERS[*b as usize % LETTERS.len()] as char);
    }
    let expires: String = c
        .query_one(
            "INSERT INTO invites (code, code_hash, kind, for_user, made_by, expires)
             VALUES ($1, $2, $3, $4, $5, now() + make_interval(mins => $6))
             RETURNING to_char(expires AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
            &[&random(9), &hash_token(&clean_code(&code)), &kind_name(kind), &for_user, &made_by, &(minutes as i32)],
        )
        .await?
        .get(0);
    Ok((code, expires))
}

/// A code as typed: capitals, no spaces or dashes.
fn clean_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// How long codes work: an admin's to pass on, one emailed now.
pub(crate) const ADMIN_CODE_MINUTES: u32 = 14 * 24 * 60;
pub(crate) const EMAIL_CODE_MINUTES: u32 = 30;

fn kind_name(k: LinkKind) -> &'static str {
    match k {
        LinkKind::Welcome => "welcome",
        LinkKind::Reset => "reset",
    }
}

/// A waiting link: its id, kind, who it's for, the email it went to.
struct Found {
    id: String,
    kind: LinkKind,
    for_user: i64,
    email: String,
}

/// A waiting code for `username` (or an older whole link, or the
/// `<id>.<secret>` from one, which name their account themselves).
async fn find_link(c: &impl GenericClient, username: &str, link: &str) -> Option<Found> {
    const WAITING: &str = "used_by IS NULL AND approved AND expires > now() AND kind IN ('welcome', 'reset') AND for_user IS NOT NULL";
    let link = link.trim();
    let r = if let Some((id, secret)) = link.rsplit('#').next().unwrap_or("").split_once('.') {
        c.query_opt(
            &format!("SELECT code, kind, for_user, email FROM invites WHERE code = $1 AND code_hash = $2 AND {WAITING} FOR UPDATE"),
            &[&id, &hash_token(secret)],
        )
        .await
    } else {
        let name = username.trim().to_lowercase();
        c.query_opt(
            &format!(
                "SELECT code, kind, for_user, email FROM invites
                 WHERE for_user = (SELECT id FROM users WHERE username = $1 OR (email <> '' AND lower(email) = $1))
                   AND code_hash = $2 AND {WAITING} FOR UPDATE"
            ),
            &[&name, &hash_token(&clean_code(link))],
        )
        .await
    };
    let r = r.ok().flatten()?;
    let kind = if r.get::<_, &str>(1) == "welcome" { LinkKind::Welcome } else { LinkKind::Reset };
    Some(Found { id: r.get(0), kind, for_user: r.get::<_, Option<i64>>(2)?, email: r.get(3) })
}

async fn check_link(State(s): State<Server>, peer: Peer, headers: HeaderMap, Json(l): Json<LinkCheck>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let keys = keys_for(&s, &c, &peer, &headers, &l.username).await;
    if s.limit.sign_in_blocked(&keys) {
        return too_many();
    }
    let Some(f) = find_link(&c, &l.username, &l.link).await else {
        s.limit.sign_in_failed(&keys);
        return bad_link();
    };
    let server_name = db::setting(&c, "name").await.ok().flatten().unwrap_or_default();
    let mut info =
        LinkInfo { kind: f.kind, server_name, username: String::new(), display_name: String::new(), email: f.email };
    let Ok(r) = c.query_one("SELECT username, display_name, email FROM users WHERE id = $1", &[&f.for_user]).await
    else {
        return failed();
    };
    (info.username, info.display_name, info.email) = (r.get(0), r.get(1), r.get(2));
    Json(info).into_response()
}

async fn use_link(State(s): State<Server>, peer: Peer, headers: HeaderMap, Json(u): Json<LinkUse>) -> Response {
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    let keys = keys_for(&s, &c, &peer, &headers, &u.username).await;
    if s.limit.sign_in_blocked(&keys) {
        return too_many();
    }
    if let Err(e) = check_password(&u.password) {
        return e;
    }
    // Hashed before the transaction, so it isn't held open meanwhile.
    let hash = hash_password(&u.password).await;
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let Some(f) = find_link(&tx, &u.username, &u.link).await else {
        s.limit.sign_in_failed(&keys);
        return bad_link();
    };
    let user = f.for_user;
    if tx.execute("UPDATE users SET password_hash = $2 WHERE id = $1", &[&user, &hash]).await.is_err() {
        return failed();
    }
    // Other devices sign in again with the new password, and the second
    // step is asked for again everywhere.
    let _ = tx.execute("DELETE FROM sessions WHERE user_id = $1", &[&user]).await;
    let _ = forget_computers(&tx, user).await;
    sessions_changed(&tx, user).await;
    if tx.execute("UPDATE invites SET used_by = $2 WHERE code = $1", &[&f.id, &user]).await.is_err()
        || tx.execute("DELETE FROM invites WHERE for_user = $1 AND used_by IS NULL", &[&user]).await.is_err()
        || tx.commit().await.is_err()
    {
        return failed();
    }
    let Ok(name) = db::username_of(&c, user).await else { return failed() };
    match db::login_of(&c, &name).await {
        Ok(Some(l)) if !l.disabled => crate::mfa::after_password(&c, l.account, &u.device, "").await,
        Ok(_) => err(StatusCode::FORBIDDEN, "disabled", "this account is turned off"),
        Err(_) => failed(),
    }
}

#[derive(serde::Deserialize)]
struct Forgot {
    /// Their email address or user name.
    username: String,
}

/// "Forgot password?", or an account without a password yet: emails a
/// code to pick one, when the account has an address and the server can
/// send email. Says the same either way, so it doesn't tell who has an
/// account; the old password keeps working until the code is used.
async fn forgot(State(s): State<Server>, peer: Peer, headers: HeaderMap, Json(f): Json<Forgot>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let keys = keys_for(&s, &c, &peer, &headers, &f.username).await;
    if s.limit.sign_in_blocked(&keys) {
        return too_many();
    }
    // Each ask counts, so nobody can ask on and on.
    s.limit.sign_in_failed(&keys);
    drop(c);
    // The rest happens after answering, so the time taken doesn't tell
    // whether there's such an account.
    let name = f.username.trim().to_lowercase();
    tokio::spawn(async move {
        let Ok(c) = s.pool.get().await else { return };
        let Some(l) = db::login_of(&c, &name).await.ok().flatten() else { return };
        if l.disabled || l.account.email.is_empty() || crate::mail::mail(&c).await.is_none() {
            return;
        }
        // A few emails an hour at most, so nobody fills someone's inbox.
        if s.limit.hit(&format!("forgot:{}", l.account.id), limit::FORGOT_WINDOW) > limit::FORGOT_EMAILS {
            return;
        }
        let new = l.hash.is_empty();
        let kind = if new { LinkKind::Welcome } else { LinkKind::Reset };
        let Ok((code, _)) = make_code(&c, kind, l.account.id, None, 8, EMAIL_CODE_MINUTES).await else { return };
        let server = db::setting(&c, "name").await.ok().flatten().unwrap_or_default();
        let e = crate::mail::code(&server, &crate::admin::by_name(&l.account), &code, new);
        if let Err(e) = crate::mail::send(&c, &l.account.email, e).await {
            eprintln!("xenify-server: a code to pick a password wasn't sent: {e}");
        }
    });
    StatusCode::NO_CONTENT.into_response()
}

async fn me(State(s): State<Server>, headers: HeaderMap) -> Response {
    match signed_in(&s, &headers).await {
        Ok(a) => Json(a).into_response(),
        Err(e) => e,
    }
}

async fn change_password(State(s): State<Server>, headers: HeaderMap, Json(p): Json<ChangePassword>) -> Response {
    let me = match signed_in(&s, &headers).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let keys = own_key(me.id);
    if s.limit.blocked(&keys) {
        return too_many();
    }
    if let Err(e) = check_password(&p.password) {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Ok(Some(l)) = db::login_of(&c, &me.username).await else { return failed() };
    if !password_ok(&p.old_password, &l.hash).await {
        s.limit.failed(&keys);
        return err(StatusCode::UNAUTHORIZED, "wrong", "your current password isn't right");
    }
    let token_hash = hash_token(bearer_token(&headers));
    let done = async {
        set_password(&c, me.id, &p.password).await?;
        // Other devices sign in again with the new password (and give
        // their second step again).
        forget_computers(&c, me.id).await?;
        c.execute("DELETE FROM sessions WHERE user_id = $1 AND token_hash <> $2", &[&me.id, &token_hash]).await
    }
    .await;
    sessions_changed(&c, me.id).await;
    if done.is_err() {
        return failed();
    }
    StatusCode::NO_CONTENT.into_response()
}

fn bearer_token(headers: &HeaderMap) -> &str {
    headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("")
}

async fn sign_out(State(s): State<Server>, headers: HeaderMap) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if let Ok(Some(r)) = c
        .query_opt("DELETE FROM sessions WHERE token_hash = $1 RETURNING user_id", &[&hash_token(bearer_token(&headers))])
        .await
    {
        sessions_changed(&c, r.get(0)).await;
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn sign_out_everywhere(State(s): State<Server>, headers: HeaderMap) -> Response {
    let me = match signed_in(&s, &headers).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let _ = c.execute("DELETE FROM sessions WHERE user_id = $1", &[&me.id]).await;
    let _ = forget_computers(&c, me.id).await;
    sessions_changed(&c, me.id).await;
    StatusCode::NO_CONTENT.into_response()
}
