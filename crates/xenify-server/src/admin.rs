//! What admins do: add and change people, reset passwords, groups and the
//! server's name.

use crate::accounts::{ADMIN_CODE_MINUTES, make_code};
use crate::{Server, admin_power, base_url, clean_name, db, err, failed, unavailable, valid_username};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use xenify_server_proto::{GroupChange, LinkKind, NewPerson, PersonChange};

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/admin/people", get(people).post(add_person))
        .route("/v1/admin/people/{id}", post(change_person))
        .route("/v1/admin/people/{id}/reset", post(reset))
        .route("/v1/admin/groups", get(groups).post(add_group))
        .route("/v1/admin/groups/{id}", post(change_group).delete(delete_group))
        .route("/v1/admin/settings", get(get_settings).post(settings))
        .route("/v1/admin/https", get(get_https).post(set_https))
        .route("/v1/admin/https/authority", axum::routing::post(new_authority))
        .route("/v1/admin/network", get(crate::net::get_net).post(crate::net::set_net))
        .route("/v1/admin/network/apps", post(crate::net::get_web_apps))
}

/// An admin, or someone with that part of running the server (Roles ›
/// Advanced).
macro_rules! admin {
    ($s:expr, $h:expr, $power:expr) => {
        match admin_power(&$s, &$h, $power).await {
            Ok(a) => a,
            Err(e) => return e,
        }
    };
}

async fn people(State(s): State<Server>, h: HeaderMap) -> Response {
    // Also for Roles (giving a role to people and groups).
    if let Err(e) = crate::admin_any(&s, &h, &["admin.people", "admin.roles"]).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match db::people(&c).await {
        Ok(p) => Json(p).into_response(),
        Err(_) => failed(),
    }
}

/// An email address someone signs in with: lower case, "" for none, None
/// if it isn't one.
pub(crate) fn clean_email(e: &str) -> Option<String> {
    let e = e.trim().to_lowercase();
    if e.is_empty() {
        return Some(e);
    }
    let (local, domain) = e.split_once('@')?;
    let ok = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && e.len() <= 254
        && !e.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\",;:()[]".contains(c))
        && !domain.contains('@');
    ok.then_some(e)
}

pub(crate) fn bad_email() -> Response {
    err(StatusCode::BAD_REQUEST, "bad_email", "that doesn't look like an email address")
}

fn email_conflict() -> Response {
    err(StatusCode::CONFLICT, "email_taken", "someone else here already has that email address")
}

/// A free user name made from an email address (never shown: people sign
/// in with the address itself): its part before the @, then 2, 3, … if
/// that's taken.
pub(crate) async fn username_for(c: &impl deadpool_postgres::GenericClient, email: &str) -> Option<String> {
    let local = email.split('@').next().unwrap_or("").to_lowercase();
    let mut base: String = local.chars().filter(|c| c.is_alphanumeric() || "._-".contains(*c)).take(28).collect();
    if base.chars().count() < 2 {
        base = format!("user{base}");
    }
    for n in 1..10_000 {
        let name = if n == 1 { base.clone() } else { format!("{base}{n}") };
        let taken = c.query_opt("SELECT 1 FROM users WHERE username = $1", &[&name]).await.ok()?;
        if taken.is_none() {
            return Some(name);
        }
    }
    None
}

/// Someone other than `id` signs in with `email`.
async fn email_taken(c: &deadpool_postgres::Client, email: &str, id: i64) -> bool {
    !email.is_empty()
        && c.query_opt("SELECT 1 FROM users WHERE lower(email) = $1 AND id <> $2", &[&email, &id])
            .await
            .ok()
            .flatten()
            .is_some()
}

/// Whether `email` is at a domain the company's mailboxes cover (Admin ›
/// Email, usermail.rs): whoever has that address gets that mailbox.
async fn company_mail(c: &impl deadpool_postgres::GenericClient, email: &str) -> bool {
    let Some((_, domain)) = email.rsplit_once('@') else { return false };
    let domain = domain.to_ascii_lowercase();
    let Some(v) = db::setting(c, "user_mail").await.ok().flatten() else { return false };
    let v: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
    if v["org"].as_str().unwrap_or("").is_empty() {
        return false;
    }
    v["domains"]
        .as_str()
        .unwrap_or("")
        .split([',', ' ', ';', '\n'])
        .map(|d| d.trim().trim_start_matches('@').to_ascii_lowercase())
        .any(|d| d == domain)
}

/// Giving someone an address at the company's mail domain gives them that
/// mailbox: for full admins and those who look after the network and email.
async fn may_give_email(c: &impl deadpool_postgres::GenericClient, me: &xenify_server_proto::Account, email: &str) -> bool {
    me.admin || !company_mail(c, email).await || crate::perms::can(c, me.id, "admin.network").await
}

fn company_mail_refused() -> Response {
    err(
        StatusCode::FORBIDDEN,
        "company_mail",
        "that address comes with a company mailbox: only admins who look after email can give it",
    )
}

/// Adds someone, with their roles (`admin` is the Admin role). With an
/// email address and a mail server, the invitation goes straight to them
/// and they pick their password when they first sign in (a code by email
/// first); otherwise the answer carries a code for the admin to pass on.
async fn add_person(State(s): State<Server>, h: HeaderMap, Json(p): Json<NewPerson>) -> Response {
    let me = admin!(s, h, "admin.people");
    if !me.admin {
        let Ok(c) = s.pool.get().await else { return unavailable() };
        if p.admin || crate::perms::powerful(&c, &p.groups).await || crate::perms::powerful_roles(&c, &p.roles).await {
            return crate::perms::full_admins_only();
        }
    }
    let Some(email) = clean_email(&p.email) else { return bad_email() };
    // People sign in with their email address: the user name underneath
    // is made from it (older callers may still give one).
    if p.username.trim().is_empty() && email.is_empty() {
        return err(StatusCode::BAD_REQUEST, "no_email", "type their email address");
    }
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    if !may_give_email(&c, &me, &email).await {
        return company_mail_refused();
    }
    if email_taken(&c, &email, 0).await {
        return email_conflict();
    }
    let name = if p.username.trim().is_empty() {
        match username_for(&c, &email).await {
            Some(n) => n,
            None => return failed(),
        }
    } else {
        p.username.trim().to_lowercase()
    };
    if !valid_username(&name) {
        return err(
            StatusCode::BAD_REQUEST,
            "bad_name",
            "user names are 2–32 letters, digits, dots, dashes or underscores",
        );
    }
    let mailing = !email.is_empty() && crate::mail::mail(&c).await.is_some();
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let id: i64 = match tx
        .query_one(
            "INSERT INTO users (username, display_name, password_hash, admin, email) VALUES ($1, $2, '', $3, $4) RETURNING id",
            &[&name, &clean_name(&p.display_name), &p.admin, &email],
        )
        .await
    {
        Ok(r) => r.get(0),
        Err(_) => return err(StatusCode::CONFLICT, "taken", "someone here already has that user name"),
    };
    let base = base_url(&tx, &h).await;
    let done = async {
        db::set_groups_of(&tx, id, &p.groups).await?;
        crate::perms::set_roles_of(&tx, id, &p.roles).await?;
        // Without email, a code for the admin to pass on.
        let code = if mailing {
            None
        } else {
            Some(make_code(&tx, LinkKind::Welcome, id, Some(me.id), 12, ADMIN_CODE_MINUTES).await?)
        };
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(code)
    }
    .await;
    let Ok(code) = done else { return failed() };
    let mut emailed = false;
    let mut mail_error = String::new();
    let mut code = code;
    if mailing {
        let server = db::setting(&c, "name").await.ok().flatten().unwrap_or_default();
        let shown = if p.display_name.trim().is_empty() { name.clone() } else { clean_name(&p.display_name) };
        let e = crate::mail::invitation(&server, &by_name(&me), &shown, &email, &crate::mail::sign_in_link(&base, &email));
        match crate::mail::send(&c, &email, e).await {
            Ok(()) => emailed = true,
            Err(e) => {
                mail_error = e;
                // It didn't go: a code to pass on instead.
                code = make_code(&c, LinkKind::Welcome, id, Some(me.id), 12, ADMIN_CODE_MINUTES).await.ok();
            }
        }
    }
    let (code, expires) = code.unwrap_or_default();
    Json(serde_json::json!({ "id": id, "emailed": emailed, "mail_error": mail_error, "code": code, "expires": expires }))
        .into_response()
}

/// How an email names the admin who did something.
pub(crate) fn by_name(a: &xenify_server_proto::Account) -> String {
    if a.display_name.is_empty() { a.username.clone() } else { a.display_name.clone() }
}

async fn change_person(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(p): Json<PersonChange>,
) -> Response {
    let me = admin!(s, h, "admin.people");
    if !me.admin {
        let Ok(c) = s.pool.get().await else { return unavailable() };
        if p.admin.is_some() || !crate::perms::may_manage(&c, &me, id).await {
            return crate::perms::full_admins_only();
        }
        // Into or out of a role that holds admin powers.
        if let Some(new) = &p.roles {
            let old: Vec<i64> = c
                .query("SELECT role FROM role_members WHERE user_id = $1", &[&id])
                .await
                .map(|r| r.iter().map(|x| x.get(0)).collect())
                .unwrap_or_default();
            let moved: Vec<i64> =
                old.iter().filter(|g| !new.contains(g)).chain(new.iter().filter(|g| !old.contains(g))).copied().collect();
            if crate::perms::powerful_roles(&c, &moved).await {
                return crate::perms::full_admins_only();
            }
        }
        // Into or out of a group that holds admin powers.
        if let Some(new) = &p.groups {
            let old: Vec<i64> = c
                .query("SELECT group_id FROM group_members WHERE user_id = $1", &[&id])
                .await
                .map(|r| r.iter().map(|x| x.get(0)).collect())
                .unwrap_or_default();
            let moved: Vec<i64> = old
                .iter()
                .filter(|g| !new.contains(g))
                .chain(new.iter().filter(|g| !old.contains(g)))
                .copied()
                .collect();
            if crate::perms::powerful(&c, &moved).await {
                return crate::perms::full_admins_only();
            }
        }
    }
    let email = match &p.email {
        Some(e) => match clean_email(e) {
            // They sign in with it.
            Some(e) if e.is_empty() => {
                return err(StatusCode::BAD_REQUEST, "no_email", "people sign in with their email address: keep one");
            }
            Some(e) => Some(e),
            None => return bad_email(),
        },
        None => None,
    };
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    if let Some(e) = &email
        && !may_give_email(&c, &me, e).await
    {
        // (Their own address again, unchanged, is fine.)
        let same = c
            .query_opt("SELECT 1 FROM users WHERE id = $1 AND lower(email) = lower($2)", &[&id, e])
            .await
            .ok()
            .flatten()
            .is_some();
        if !same {
            return company_mail_refused();
        }
    }
    if let Some(e) = &email
        && email_taken(&c, e, id).await
    {
        return email_conflict();
    }
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let exists = tx.query_opt("SELECT 1 FROM users WHERE id = $1 AND NOT app FOR UPDATE", &[&id]).await.ok().flatten();
    if exists.is_none() {
        return err(StatusCode::NOT_FOUND, "no_person", "there's nobody like that");
    }
    // Someone must always be able to run the server.
    if (p.admin == Some(false) || p.disabled == Some(true)) && db::other_admins(&tx, id).await.unwrap_or(0) == 0 {
        let is_admin: bool =
            tx.query_one("SELECT admin FROM users WHERE id = $1", &[&id]).await.is_ok_and(|r| r.get(0));
        if is_admin {
            return err(StatusCode::CONFLICT, "last_admin", "make someone else an admin first");
        }
    }
    let done = async {
        if let Some(n) = &p.display_name {
            tx.execute("UPDATE users SET display_name = $2 WHERE id = $1", &[&id, &clean_name(n)]).await?;
        }
        if let Some(e) = &email {
            let changed = tx
                .execute("UPDATE users SET email = $2 WHERE id = $1 AND email IS DISTINCT FROM $2", &[&id, e])
                .await?;
            // They sign in with it: signed out everywhere (not an admin
            // changing their own, who'd only lose this page).
            if changed == 1 && id != me.id {
                tx.execute("DELETE FROM sessions WHERE user_id = $1", &[&id]).await?;
            }
        }
        if let Some(a) = p.admin {
            tx.execute("UPDATE users SET admin = $2 WHERE id = $1", &[&id, &a]).await?;
        }
        if let Some(d) = p.disabled {
            tx.execute("UPDATE users SET disabled = $2 WHERE id = $1", &[&id, &d]).await?;
            if d {
                tx.execute("DELETE FROM sessions WHERE user_id = $1", &[&id]).await?;
            }
        }
        if let Some(g) = &p.groups {
            db::set_groups_of(&tx, id, g).await?;
        }
        if let Some(r) = &p.roles {
            crate::perms::set_roles_of(&tx, id, r).await?;
        }
        tx.commit().await
    }
    .await;
    match done {
        Ok(()) => {
            // Turned off (or signed out by a new email): their chats and calls drop now too.
            if p.disabled == Some(true) || (email.is_some() && id != me.id) {
                crate::accounts::sessions_changed(&c, id).await;
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => failed(),
    }
}

/// Forgot their password: signs them out everywhere, and they pick a new
/// one when they next sign in, with a code by email (they're told) or, the
/// server can't email them, the code in the answer to pass on.
async fn reset(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let me = admin!(s, h, "admin.people");
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    if !crate::perms::may_manage(&c, &me, id).await {
        return crate::perms::full_admins_only();
    }
    let Ok(Some(who)) =
        c.query_opt("SELECT username, display_name, email FROM users WHERE id = $1 AND NOT app", &[&id]).await
    else {
        return err(StatusCode::NOT_FOUND, "no_person", "there's nobody like that");
    };
    let (user, display, email): (String, String, String) = (who.get(0), who.get(1), who.get(2));
    let mailing = !email.is_empty() && crate::mail::mail(&c).await.is_some();
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let base = base_url(&tx, &h).await;
    let done = async {
        tx.execute("UPDATE users SET password_hash = '' WHERE id = $1", &[&id]).await?;
        tx.execute("DELETE FROM sessions WHERE user_id = $1", &[&id]).await?;
        // Computers that skipped the second step are asked again.
        crate::accounts::forget_computers(&tx, id).await?;
        tx.execute("DELETE FROM invites WHERE for_user = $1 AND used_by IS NULL", &[&id]).await?;
        let code = if mailing { None } else { Some(make_code(&tx, LinkKind::Reset, id, Some(me.id), 12, ADMIN_CODE_MINUTES).await?) };
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(code)
    }
    .await;
    let Ok(mut code) = done else { return failed() };
    crate::accounts::sessions_changed(&c, id).await;
    let mut emailed = false;
    let mut mail_error = String::new();
    if mailing {
        let server = db::setting(&c, "name").await.ok().flatten().unwrap_or_default();
        let e = crate::mail::reset(&server, if display.is_empty() { &user } else { &display }, &email, &crate::mail::sign_in_link(&base, &email));
        match crate::mail::send(&c, &email, e).await {
            Ok(()) => emailed = true,
            Err(e) => {
                mail_error = e;
                code = make_code(&c, LinkKind::Reset, id, Some(me.id), 12, ADMIN_CODE_MINUTES).await.ok();
            }
        }
    }
    let (code, expires) = code.unwrap_or_default();
    Json(serde_json::json!({ "code": code, "expires": expires, "emailed": emailed, "mail_error": mail_error })).into_response()
}

async fn groups(State(s): State<Server>, h: HeaderMap) -> Response {
    // Also for Roles (giving a role to people and groups).
    if let Err(e) = crate::admin_any(&s, &h, &["admin.people", "admin.roles"]).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match db::groups(&c).await {
        Ok(g) => Json(g).into_response(),
        Err(_) => failed(),
    }
}

fn group_name(g: &GroupChange) -> Result<String, Response> {
    let n = clean_name(&g.name);
    if n.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "bad_name", "give the group a name"));
    }
    Ok(n)
}

async fn add_group(State(s): State<Server>, h: HeaderMap, Json(g): Json<GroupChange>) -> Response {
    admin!(s, h, "admin.people");
    let name = match group_name(&g) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let id: i64 = match tx.query_one("INSERT INTO groups (name) VALUES ($1) RETURNING id", &[&name]).await {
        Ok(r) => r.get(0),
        Err(_) => return err(StatusCode::CONFLICT, "taken", "there's already a group with that name"),
    };
    if db::set_members(&tx, id, &g.members).await.is_err() || tx.commit().await.is_err() {
        return failed();
    }
    Json(serde_json::json!({ "id": id })).into_response()
}

async fn change_group(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(g): Json<GroupChange>,
) -> Response {
    let me = admin!(s, h, "admin.people");
    if !me.admin {
        let Ok(c) = s.pool.get().await else { return unavailable() };
        if crate::perms::powerful(&c, &[id]).await {
            return crate::perms::full_admins_only();
        }
    }
    let name = match group_name(&g) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    let Ok(tx) = c.transaction().await else { return unavailable() };
    match tx.execute("UPDATE groups SET name = $2 WHERE id = $1", &[&id, &name]).await {
        Ok(0) => return err(StatusCode::NOT_FOUND, "no_group", "there's no such group"),
        Ok(_) => {}
        Err(_) => return err(StatusCode::CONFLICT, "taken", "there's already a group with that name"),
    }
    if db::set_members(&tx, id, &g.members).await.is_err() || tx.commit().await.is_err() {
        return failed();
    }
    // Plans that give work to the group: who can do it changed.
    crate::plan::world_changed(&s).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn delete_group(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let me = admin!(s, h, "admin.people");
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if !me.admin && crate::perms::powerful(&c, &[id]).await {
        return crate::perms::full_admins_only();
    }
    match c.execute("DELETE FROM groups WHERE id = $1", &[&id]).await {
        Ok(_) => {
            crate::plan::world_changed(&s).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct Settings {
    name: Option<String>,
    /// "local", "lan" or "anywhere".
    admin_access: Option<String>,
    /// The largest file people can send in chats, in MB.
    chat_file_limit_mb: Option<i64>,
    /// People can record calls (each person is in it only once they turn
    /// something back on).
    call_recording: Option<bool>,
    /// People can reply to a message in its thread (on by default).
    chat_threads: Option<bool>,
    /// How many days chats keep messages (0: for ever).
    chat_keep_days: Option<i64>,
    /// People can share pages with anyone through a link (Admin › Public
    /// links); off stops every link until it's back on.
    public_links: Option<bool>,
    /// Who must sign in with a second step: "off", "admins" or "everyone".
    mfa_required: Option<String>,
}

async fn get_settings(State(s): State<Server>, h: HeaderMap) -> Response {
    // Read on the People, Public links and Network pages.
    if let Err(e) = crate::admin_any(&s, &h, &["admin.network", "admin.people", "admin.links"]).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let get = |k: &'static str| {
        let c = &c;
        async move { db::setting(c, k).await.ok().flatten().unwrap_or_default() }
    };
    let access = get("admin_access").await;
    Json(serde_json::json!({
        "name": get("name").await,
        "admin_access": if access.is_empty() { "lan".to_string() } else { access },
        "chat_file_limit_mb": crate::chatfiles::limit_bytes(&c).await / (1024 * 1024),
        "call_recording": get("call_recording").await != "off",
        "chat_threads": get("chat_threads").await != "off",
        "chat_keep_days": crate::chatjobs::keep_days(&c).await.unwrap_or(0),
        "public_links": crate::publinks::allowed(&c).await,
        "mfa_required": crate::mfa::required(&c).await,
    }))
    .into_response()
}

async fn settings(State(s): State<Server>, h: HeaderMap, Json(v): Json<Settings>) -> Response {
    // Each part needs its power; who opens the admin pages and who must use
    // a second step are for full admins only.
    let me = match crate::signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if !me.admin {
        if v.admin_access.is_some() || v.mfa_required.is_some() {
            return crate::perms::full_admins_only();
        }
        let network = v.name.is_some()
            || v.chat_file_limit_mb.is_some()
            || v.call_recording.is_some()
            || v.chat_threads.is_some()
            || v.chat_keep_days.is_some();
        if network && !crate::perms::can(&c, me.id, "admin.network").await
            || v.public_links.is_some() && !crate::perms::can(&c, me.id, "admin.links").await
        {
            return err(StatusCode::FORBIDDEN, "not_admin", "only admins can do that");
        }
    }
    if let Some(n) = v.name.map(|n| clean_name(&n)).filter(|n| !n.is_empty())
        && db::set_setting(&c, "name", &n).await.is_err()
    {
        return failed();
    }
    if let Some(a) = v.admin_access {
        if !["local", "lan", "anywhere"].contains(&a.as_str()) {
            return err(StatusCode::BAD_REQUEST, "bad_access", "admin pages are open to local, lan or anywhere");
        }
        if db::set_setting(&c, "admin_access", &a).await.is_err() {
            return failed();
        }
    }
    if let Some(mb) = v.chat_file_limit_mb {
        if !(1..=100 * 1024).contains(&mb) {
            return err(StatusCode::BAD_REQUEST, "bad_limit", "the largest file is 1 MB to 100 GB");
        }
        if db::set_setting(&c, "chat_file_limit_mb", &mb.to_string()).await.is_err() {
            return failed();
        }
    }
    if let Some(on) = v.call_recording
        && db::set_setting(&c, "call_recording", if on { "on" } else { "off" }).await.is_err()
    {
        return failed();
    }
    if let Some(on) = v.chat_threads
        && db::set_setting(&c, "chat_threads", if on { "on" } else { "off" }).await.is_err()
    {
        return failed();
    }
    if let Some(days) = v.chat_keep_days {
        if !(0..=36500).contains(&days) {
            return err(StatusCode::BAD_REQUEST, "bad_limit", "keep messages for 1 day or more (0: for ever)");
        }
        if db::set_setting(&c, "chat_keep_days", &days.to_string()).await.is_err() {
            return failed();
        }
        s.chat.set_keep_days(Some(days));
    }
    if let Some(on) = v.public_links
        && db::set_setting(&c, "public_links", if on { "on" } else { "off" }).await.is_err()
    {
        return failed();
    }
    if let Some(m) = v.mfa_required {
        if !["off", "admins", "everyone"].contains(&m.as_str()) {
            return err(StatusCode::BAD_REQUEST, "bad_rule", "a second step is required of off, admins or everyone");
        }
        if db::set_setting(&c, "mfa_required", &m).await.is_err() {
            return failed();
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Admin › Network › the server's domain name, for a certificate from
/// Let's Encrypt.
#[derive(Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub(crate) struct Https {
    pub domain: String,
    /// Where Let's Encrypt may write (a certificate about to run out).
    pub email: String,
}

pub(crate) async fn https_setting(c: &deadpool_postgres::Client) -> Https {
    db::setting(c, "https").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// A domain name: letters, digits, dashes and dots, with at least one dot,
/// not an address.
pub(crate) fn valid_domain(d: &str) -> bool {
    d.len() <= 253
        && d.contains('.')
        && d.parse::<std::net::IpAddr>().is_err()
        && d.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

async fn get_https(State(s): State<Server>, h: HeaderMap) -> Response {
    admin!(s, h, "admin.network");
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let saved = https_setting(&c).await;
    let acme = s.acme.read().unwrap().clone();
    let port = *s.https_port.lock().unwrap();
    Json(serde_json::json!({
        "domain": acme.as_ref().map(|a| a.domains.join(", ")).unwrap_or(saved.domain),
        "email": saved.email,
        "fixed": s.acme_fixed.load(std::sync::atomic::Ordering::SeqCst),
        "status": acme.map(|a| a.status.lock().unwrap().clone()),
        "port": port,
        "authority": crate::tls::authority(&s.data.join("tls")).map(|(fingerprint, limited)| serde_json::json!({
            "fingerprint": fingerprint, "limited": limited,
        })),
    }))
    .into_response()
}

/// Admin › Network › Make a new certificate authority: for one made before
/// authorities were limited to this network, or one that may have been
/// copied. Computers that trusted the old one must trust the new one.
async fn new_authority(State(s): State<Server>, h: HeaderMap) -> Response {
    admin!(s, h, "admin.network");
    if let Err(e) = crate::tls::forget_authority(&s.data.join("tls")) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "failed", &e);
    }
    let net = s.net().await;
    if net.https_port.is_some()
        && let Err(e) = s.run_network(net).await
    {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "failed", &e);
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn set_https(State(s): State<Server>, h: HeaderMap, Json(v): Json<Https>) -> Response {
    admin!(s, h, "admin.network");
    if s.acme_fixed.load(std::sync::atomic::Ordering::SeqCst) {
        return err(StatusCode::CONFLICT, "fixed", "the domain name was set when the server was started (--domain)");
    }
    if s.https_port.lock().unwrap().is_none() {
        return err(StatusCode::CONFLICT, "no_https", "HTTPS is off: turn it on above first");
    }
    let domain = v.domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let email = v.email.trim().to_string();
    if !domain.is_empty() && !valid_domain(&domain) {
        return err(StatusCode::BAD_REQUEST, "bad_domain", "that isn't a domain name (like files.example.com)");
    }
    if !email.is_empty() && !email.contains('@') {
        return err(StatusCode::BAD_REQUEST, "bad_email", "that isn't an email address");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let set = Https { domain: domain.clone(), email: email.clone() };
    if db::set_setting(&c, "https", &serde_json::to_string(&set).expect("serialises")).await.is_err() {
        return failed();
    }
    let unchanged = s.acme.read().unwrap().as_ref().is_some_and(|a| a.domains == [domain.clone()]);
    if !unchanged {
        s.use_domains(
            if domain.is_empty() { vec![] } else { vec![domain] },
            Some(email).filter(|e| !e.is_empty()),
            true,
        );
    }
    StatusCode::NO_CONTENT.into_response()
}
