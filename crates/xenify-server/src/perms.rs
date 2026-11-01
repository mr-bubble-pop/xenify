//! Roles: what people may do on this server, and custom roles for documents.
//!
//! - Server roles hold permissions ("ai.pictures", "chat.create_channel" …).
//!   The built-in "Everyone" role applies to every signed-in person; other
//!   roles are given to people or groups. Admins may do everything. Anything
//!   (also other crates) asks the database: `SELECT xenify_can(user, perm)`.
//! - Document roles are made by admins for sharing: a base ("can view",
//!   "can comment", "can edit") plus, for Plan, exactly which changes are
//!   allowed ("plan.create", "plan.status" …). The server checks every Plan
//!   change against them (live.rs); other apps follow the base.

use crate::{Server, err, failed, signed_in, unavailable};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::GenericClient;
use serde::Deserialize;
use serde_json::{Value, json};

/// Server permissions: id, app, what it lets you do, on for Everyone at first.
pub const PERMS: &[(&str, &str, &str, bool)] = &[
    ("files.spaces", "Files", "Make shared spaces", false),
    ("files.public_links", "Files", "Make public links anyone can read", true),
    ("ai.text", "AI", "Use AI for writing and questions", true),
    ("ai.pictures", "AI", "Make and change pictures with AI", true),
    ("ai.video", "AI", "Make video clips with AI", true),
    ("ai.tools", "AI", "Use the tools an admin added for AI (MCP) in Calc’s Ask", true),
    ("chat.create_channel", "Chat", "Make private channels", true),
    ("chat.create_open_channel", "Chat", "Make open channels anyone can join", true),
    ("chat.create_apps", "Chat", "Add apps and webhooks to chats", true),
    ("chat.moderate", "Chat", "Moderate: remove people, delete others’ messages, close channels", false),
    ("plan.people", "Plan", "Change other people’s working hours, time away and team dates", false),
];

/// Parts of running the server a role can hold (Roles › Advanced), so
/// someone can look after some of it without being an admin. Admins have
/// them all; only admins give them out, make admins, or change who's in a
/// group or role that holds them. Never for Everyone.
pub const POWERS: &[(&str, &str)] = &[
    ("admin.people", "People and groups (not admins)"),
    ("admin.roles", "Roles and permissions (not these powers)"),
    ("admin.links", "Public links"),
    ("admin.ai", "AI: programs, models and picture styles"),
    ("admin.data", "Set up database connections for Calc, and AI tools"),
    ("admin.holidays", "Company holidays"),
    ("admin.updates", "Apps and updates"),
    ("admin.backups", "Backups, and how long bins keep deleted things"),
    ("admin.network", "Network, HTTPS, outgoing mail, chat limits and the server’s name"),
];

/// What a document role may change in a plan (the server checks each change).
pub use xenify_plan_core::access::PERMS as PLAN_PERMS;

/// Everything a plain editor may do in a plan.
pub fn all_plan() -> Vec<String> {
    xenify_plan_core::access::all()
}

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS roles (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('server', 'document')),
    perms TEXT[] NOT NULL DEFAULT '{}',
    -- Documents: what the role is at least (viewer, commenter or editor).
    base TEXT NOT NULL DEFAULT 'viewer' CHECK (base IN ('viewer', 'commenter', 'editor')),
    -- The built-in role every signed-in person has.
    everyone BOOLEAN NOT NULL DEFAULT FALSE,
    about TEXT NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX IF NOT EXISTS roles_name ON roles (kind, lower(name));
CREATE UNIQUE INDEX IF NOT EXISTS roles_everyone ON roles (everyone) WHERE everyone;
CREATE TABLE IF NOT EXISTS role_members (
    role BIGINT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    user_id BIGINT REFERENCES users(id) ON DELETE CASCADE,
    group_id BIGINT REFERENCES groups(id) ON DELETE CASCADE,
    CHECK ((user_id IS NULL) <> (group_id IS NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS role_members_once ON role_members (role, coalesce(user_id, 0), coalesce(group_id, 0));
-- A grant can use a document role instead of a built-in one (role then holds its base).
ALTER TABLE grants ADD COLUMN IF NOT EXISTS custom BIGINT REFERENCES roles(id) ON DELETE SET NULL;
CREATE OR REPLACE FUNCTION xenify_can(u BIGINT, p TEXT) RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM users WHERE id = u AND admin AND NOT disabled)
        OR EXISTS (
            SELECT 1 FROM roles r WHERE r.kind = 'server' AND p = ANY(r.perms)
            AND EXISTS (SELECT 1 FROM users WHERE id = u AND NOT disabled)
            AND (r.everyone OR EXISTS (
                SELECT 1 FROM role_members m WHERE m.role = r.id
                AND (m.user_id = u OR m.group_id IN (SELECT group_id FROM group_members WHERE user_id = u)))))
$$;
-- Invites are gone (2026-10-29): admins add people.
UPDATE roles SET perms = array_remove(array_remove(perms, 'people.invite'), 'people.invite_ask')
    WHERE perms && ARRAY['people.invite', 'people.invite_ask'];
-- Documents open for editing together check who may still do what as soon
-- as that changes (live.rs, a 'check' note): someone signed out or turned
-- off, out of a group or a chat, a share changed, a document role changed.
CREATE OR REPLACE FUNCTION xenify_live_check() RETURNS trigger LANGUAGE plpgsql AS $f$
DECLARE r RECORD; note JSON;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    IF TG_TABLE_NAME = 'users' THEN
        note := json_build_object('k', 'check', 'user', r.id);
    ELSIF TG_TABLE_NAME = 'grants' THEN
        note := json_build_object('k', 'check', 'node', r.node, 'space', r.space);
    ELSIF TG_TABLE_NAME = 'roles' THEN
        note := json_build_object('k', 'check');
    ELSE
        note := json_build_object('k', 'check', 'user', r.user_id);
    END IF;
    PERFORM pg_notify('xenify_live', note::text);
    RETURN NULL;
END $f$;
DROP TRIGGER IF EXISTS live_check ON sessions;
CREATE TRIGGER live_check AFTER DELETE ON sessions FOR EACH ROW EXECUTE FUNCTION xenify_live_check();
DROP TRIGGER IF EXISTS live_check ON members;
CREATE TRIGGER live_check AFTER DELETE ON members FOR EACH ROW EXECUTE FUNCTION xenify_live_check();
DROP TRIGGER IF EXISTS live_check ON group_members;
CREATE TRIGGER live_check AFTER DELETE ON group_members FOR EACH ROW EXECUTE FUNCTION xenify_live_check();
DROP TRIGGER IF EXISTS live_check ON grants;
CREATE TRIGGER live_check AFTER INSERT OR UPDATE OR DELETE ON grants FOR EACH ROW EXECUTE FUNCTION xenify_live_check();
DROP TRIGGER IF EXISTS live_check ON users;
CREATE TRIGGER live_check AFTER UPDATE OF disabled, admin ON users FOR EACH ROW
    WHEN ((NEW.disabled AND NOT OLD.disabled) OR (OLD.admin AND NOT NEW.admin)) EXECUTE FUNCTION xenify_live_check();
DROP TRIGGER IF EXISTS live_check ON roles;
CREATE TRIGGER live_check AFTER UPDATE OR DELETE ON roles FOR EACH ROW
    WHEN (OLD.kind = 'document') EXECUTE FUNCTION xenify_live_check();
";

/// Makes the Everyone role on a new server, with the permissions on by default.
pub async fn seed(c: &impl GenericClient) -> Result<(), tokio_postgres::Error> {
    let on: Vec<String> = PERMS.iter().filter(|p| p.3).map(|p| p.0.to_string()).collect();
    c.execute(
        "INSERT INTO roles (name, kind, perms, everyone, about) SELECT 'Everyone', 'server', $1, TRUE, 'Everyone signed in to this server'
         WHERE NOT EXISTS (SELECT 1 FROM roles WHERE everyone)",
        &[&on],
    )
    .await?;
    Ok(())
}

/// Whether `user` may do `perm` here.
pub async fn can(c: &impl GenericClient, user: i64, perm: &str) -> bool {
    c.query_one("SELECT xenify_can($1, $2)", &[&user, &perm]).await.map(|r| r.get::<_, bool>(0)).unwrap_or(false)
}

/// Every server permission `user` has.
pub async fn of(c: &impl GenericClient, user: i64) -> Vec<String> {
    let mut out = Vec::new();
    for p in PERMS {
        if can(c, user, p.0).await {
            out.push(p.0.to_string());
        }
    }
    out
}

/// Whether `me` may change person `target`'s account (reset a password,
/// change their email, clear their second step, turn them off): full admins
/// anyone; someone with only the "admin.people" power, never an admin nor
/// anyone holding a part of running the server, themselves or through a
/// group (or they could take over that account and its powers).
pub async fn may_manage(c: &impl GenericClient, me: &xenify_server_proto::Account, target: i64) -> bool {
    if me.admin {
        return true;
    }
    if c.query_opt("SELECT 1 FROM users WHERE id = $1 AND NOT admin", &[&target]).await.ok().flatten().is_none() {
        return false;
    }
    let Ok(roles) = c
        .query("SELECT role FROM role_members WHERE user_id = $1", &[&target])
        .await
        .map(|r| r.iter().map(|x| x.get::<_, i64>(0)).collect::<Vec<_>>())
    else {
        return false;
    };
    let Ok(groups) = c
        .query("SELECT group_id FROM group_members WHERE user_id = $1", &[&target])
        .await
        .map(|r| r.iter().map(|x| x.get::<_, i64>(0)).collect::<Vec<_>>())
    else {
        return false;
    };
    !powerful_roles(c, &roles).await && !powerful(c, &groups).await
}

/// Groups some role gives a part of running the server ("admin.…"):
/// who's in them is for full admins to decide.
pub async fn powerful(c: &impl GenericClient, groups: &[i64]) -> bool {
    !groups.is_empty()
        && c.query_opt(
            "SELECT 1 FROM role_members m JOIN roles r ON r.id = m.role
             WHERE m.group_id = ANY($1) AND EXISTS (SELECT 1 FROM unnest(r.perms) p WHERE p LIKE 'admin.%')",
            &[&groups],
        )
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Whether any of these roles gives a part of running the server.
pub async fn powerful_roles(c: &impl GenericClient, roles: &[i64]) -> bool {
    !roles.is_empty()
        && c.query_opt(
            "SELECT 1 FROM roles WHERE id = ANY($1) AND EXISTS (SELECT 1 FROM unnest(perms) p WHERE p LIKE 'admin.%')",
            &[&roles],
        )
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Gives `user` exactly these server roles (not Everyone, not Admin).
pub async fn set_roles_of(c: &impl GenericClient, user: i64, roles: &[i64]) -> Result<(), tokio_postgres::Error> {
    c.execute(
        "DELETE FROM role_members WHERE user_id = $1 AND role IN (SELECT id FROM roles WHERE kind = 'server')",
        &[&user],
    )
    .await?;
    c.execute(
        "INSERT INTO role_members (role, user_id) SELECT id, $2 FROM roles WHERE id = ANY($1) AND kind = 'server' AND NOT everyone
         ON CONFLICT DO NOTHING",
        &[&roles, &user],
    )
    .await?;
    Ok(())
}

/// The Admin role is the `admin` mark on accounts; Roles shows it as a
/// role with this id, holding everything.
pub const ADMIN_ROLE: i64 = 0;

/// Said when a delegated admin reaches for what only full admins may do.
pub fn full_admins_only() -> Response {
    err(StatusCode::FORBIDDEN, "not_admin", "only full admins can do that")
}

/// The "you may not" answer, naming what's missing.
pub fn denied(perm: &str) -> Response {
    let what = PERMS.iter().find(|p| p.0 == perm).map(|p| p.2).unwrap_or(perm);
    err(
        StatusCode::FORBIDDEN,
        "not_allowed",
        &format!("Your role here doesn’t allow this ({}). Ask an admin.", what.to_lowercase()),
    )
}

/// Signed in and allowed `perm`.
pub async fn need(s: &Server, h: &HeaderMap, perm: &str) -> Result<xenify_server_proto::Account, Response> {
    let a = signed_in(s, h).await?;
    let c = s.pool.get().await.map_err(|_| unavailable())?;
    if !can(&c, a.id, perm).await {
        return Err(denied(perm));
    }
    Ok(a)
}

/// A document role's base and its Plan permissions.
pub async fn doc_role(c: &impl GenericClient, id: i64) -> Option<(String, Vec<String>)> {
    c.query_opt("SELECT base, perms FROM roles WHERE id = $1 AND kind = 'document'", &[&id])
        .await
        .ok()
        .flatten()
        .map(|r| (r.get(0), r.get(1)))
}

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/me/perms", get(my_perms))
        .route("/v1/roles", get(list))
        .route("/v1/admin/roles", post(save))
        .route("/v1/admin/roles/{id}", post(update).delete(remove))
}

async fn my_perms(State(s): State<Server>, h: HeaderMap) -> Response {
    let a = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let mut powers = Vec::new();
    for p in POWERS {
        if can(&c, a.id, p.0).await {
            powers.push(p.0);
        }
    }
    Json(json!({ "admin": a.admin, "perms": of(&c, a.id).await, "powers": powers })).into_response()
}

/// Every role (anyone signed in sees the document roles, to share with;
/// admins and role managers also the server roles and their members).
async fn list(State(s): State<Server>, h: HeaderMap) -> Response {
    let a = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let manager = a.admin || can(&c, a.id, "admin.roles").await;
    let Ok(rows) = c
        .query("SELECT id, name, kind, perms, base, everyone, about FROM roles WHERE kind = 'document' OR $1 ORDER BY kind, everyone DESC, lower(name)", &[&manager])
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    if manager {
        let admins = c
            .query("SELECT id FROM users WHERE admin AND NOT app ORDER BY lower(username)", &[])
            .await
            .unwrap_or_default();
        out.push(json!({ "id": ADMIN_ROLE, "name": "Admin", "kind": "server", "perms": [], "base": "viewer", "everyone": false,
            "admin": true, "about": "Can do everything here, also make other admins",
            "members": admins.iter().map(|x| json!({ "user": x.get::<_, i64>(0) })).collect::<Vec<_>>() }));
    }
    for r in rows {
        let id: i64 = r.get(0);
        let mut v = json!({ "id": id, "name": r.get::<_, String>(1), "kind": r.get::<_, String>(2), "perms": r.get::<_, Vec<String>>(3),
            "base": r.get::<_, String>(4), "everyone": r.get::<_, bool>(5), "about": r.get::<_, String>(6) });
        if manager {
            let m =
                c.query("SELECT user_id, group_id FROM role_members WHERE role = $1", &[&id]).await.unwrap_or_default();
            v["members"] = json!(
                m.iter()
                    .map(|x| match (x.get::<_, Option<i64>>(0), x.get::<_, Option<i64>>(1)) {
                        (Some(u), _) => json!({ "user": u }),
                        (_, g) => json!({ "group": g }),
                    })
                    .collect::<Vec<_>>()
            );
        }
        out.push(v);
    }
    let catalogue = json!({
        "server": PERMS.iter().map(|p| json!({ "id": p.0, "app": p.1, "name": p.2 })).collect::<Vec<_>>(),
        // Roles › Advanced: parts of running the server.
        "admin": POWERS.iter().map(|p| json!({ "id": p.0, "name": p.1 })).collect::<Vec<_>>(),
        "plan": PLAN_PERMS.iter().map(|p| json!({ "id": p.0, "name": p.1 })).collect::<Vec<_>>(),
    });
    Json(json!({ "roles": out, "catalogue": catalogue })).into_response()
}

#[derive(Deserialize)]
struct RoleIn {
    name: Option<String>,
    kind: Option<String>,
    perms: Option<Vec<String>>,
    base: Option<String>,
    about: Option<String>,
    /// Server roles: who has it ({user} or {group}); replaces the list.
    members: Option<Vec<Value>>,
}

fn check(kind: &str, perms: &[String]) -> Result<(), Response> {
    let known = |p: &str| {
        if kind == "server" {
            PERMS.iter().any(|x| x.0 == p) || POWERS.iter().any(|x| x.0 == p)
        } else {
            PLAN_PERMS.iter().any(|x| x.0 == p)
        }
    };
    match perms.iter().find(|p| !known(p)) {
        Some(p) => Err(err(StatusCode::BAD_REQUEST, "unknown_perm", &format!("there's no permission called {p}"))),
        None => Ok(()),
    }
}

fn is_power(p: &str) -> bool {
    p.starts_with("admin.")
}

/// Who may make or change a role: admins any; someone with the
/// "admin.roles" power only roles that hold no admin powers, before or after.
async fn may_change_role(s: &Server, h: &HeaderMap, holds: &[String], b: &RoleIn) -> Result<(), Response> {
    let a = crate::admin_power(s, h, "admin.roles").await?;
    if a.admin {
        return Ok(());
    }
    if holds.iter().chain(b.perms.iter().flatten()).any(|p| is_power(p)) {
        return Err(full_admins_only());
    }
    Ok(())
}

/// Changes role `id` as `b` says. In a transaction: a bad part (an unknown
/// member, a name taken) leaves the role as it was.
async fn write(c: &impl GenericClient, id: i64, b: &RoleIn, kind: &str, everyone: bool) -> Result<(), Response> {
    if let Some(p) = &b.perms {
        check(kind, p)?;
        if everyone && p.iter().any(|x| is_power(x)) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "power_for_everyone",
                "parts of running the server go to people or groups, not Everyone",
            ));
        }
        c.execute("UPDATE roles SET perms = $2 WHERE id = $1", &[&id, p]).await.map_err(|_| failed())?;
    }
    if let Some(n) = &b.name {
        let n = n.trim();
        if n.is_empty() || n.chars().count() > 60 {
            return Err(err(StatusCode::BAD_REQUEST, "bad_name", "give the role a name (up to 60 letters)"));
        }
        if everyone && n != "Everyone" {
            return Err(err(StatusCode::BAD_REQUEST, "built_in", "the Everyone role keeps its name"));
        }
        c.execute("UPDATE roles SET name = $2 WHERE id = $1", &[&id, &n])
            .await
            .map_err(|_| err(StatusCode::CONFLICT, "taken", "there's a role with that name already"))?;
    }
    if let Some(a) = &b.about {
        c.execute("UPDATE roles SET about = $2 WHERE id = $1", &[&id, &a.trim()]).await.map_err(|_| failed())?;
    }
    if let Some(base) = &b.base {
        if kind != "document" || !["viewer", "commenter", "editor"].contains(&base.as_str()) {
            return Err(err(StatusCode::BAD_REQUEST, "bad_base", "a document role is based on view, comment or edit"));
        }
        c.execute("UPDATE roles SET base = $2 WHERE id = $1", &[&id, base]).await.map_err(|_| failed())?;
    }
    if let Some(m) = &b.members {
        if kind != "server" || everyone {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "no_members",
                "only server roles (not Everyone) are given to people",
            ));
        }
        c.execute("DELETE FROM role_members WHERE role = $1", &[&id]).await.map_err(|_| failed())?;
        for x in m {
            let (u, g) = (x["user"].as_i64(), x["group"].as_i64());
            if u.is_none() == g.is_none() {
                continue;
            }
            c.execute(
                "INSERT INTO role_members (role, user_id, group_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                &[&id, &u, &g],
            )
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "bad_member", "no such person or group"))?;
        }
    }
    Ok(())
}

async fn save(State(s): State<Server>, h: HeaderMap, Json(b): Json<RoleIn>) -> Response {
    if let Err(e) = may_change_role(&s, &h, &[], &b).await {
        return e;
    }
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    let kind = b.kind.clone().unwrap_or_else(|| "server".into());
    if kind != "server" && kind != "document" {
        return err(StatusCode::BAD_REQUEST, "bad_kind", "a role is for the server or for documents");
    }
    let name = b.name.clone().unwrap_or_default();
    if name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "bad_name", "give the role a name");
    }
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let Ok(row) = tx
        .query_one(
            "INSERT INTO roles (name, kind, base) VALUES ($1, $2, $3) RETURNING id",
            &[
                &name.trim(),
                &kind,
                &b.base.clone().unwrap_or_else(|| if kind == "document" { "editor".into() } else { "viewer".into() }),
            ],
        )
        .await
    else {
        return err(StatusCode::CONFLICT, "taken", "there's a role with that name already");
    };
    let id: i64 = row.get(0);
    let rest = RoleIn { name: None, base: None, ..b };
    if let Err(e) = write(&tx, id, &rest, &kind, false).await {
        return e;
    }
    if tx.commit().await.is_err() {
        return failed();
    }
    Json(json!({ "id": id })).into_response()
}

async fn update(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Json(b): Json<RoleIn>) -> Response {
    if id == ADMIN_ROLE {
        return set_admins(&s, &h, &b).await;
    }
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    let holds: Vec<String> = c
        .query_opt("SELECT perms FROM roles WHERE id = $1", &[&id])
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
        .unwrap_or_default();
    if let Err(e) = may_change_role(&s, &h, &holds, &b).await {
        return e;
    }
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let Ok(Some(r)) = tx.query_opt("SELECT kind, everyone FROM roles WHERE id = $1 FOR UPDATE", &[&id]).await else {
        return err(StatusCode::NOT_FOUND, "not_found", "no such role");
    };
    let (kind, everyone): (String, bool) = (r.get(0), r.get(1));
    if let Err(e) = write(&tx, id, &b, &kind, everyone).await {
        return e;
    }
    match tx.commit().await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(_) => failed(),
    }
}

/// Who has the Admin role: people only (not groups), and never nobody.
async fn set_admins(s: &Server, h: &HeaderMap, b: &RoleIn) -> Response {
    let a = match crate::admin_power(s, h, "admin.roles").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !a.admin {
        return full_admins_only();
    }
    if b.name.is_some() || b.perms.is_some() || b.base.is_some() || b.about.is_some() {
        return err(StatusCode::BAD_REQUEST, "built_in", "the Admin role holds everything; only who has it changes");
    }
    let Some(m) = &b.members else { return Json(json!({ "ok": true })).into_response() };
    if m.iter().any(|x| x.get("group").is_some()) {
        return err(StatusCode::BAD_REQUEST, "people_only", "the Admin role goes to people, not groups");
    }
    let users: Vec<i64> = m.iter().filter_map(|x| x["user"].as_i64()).collect();
    let Ok(mut c) = s.pool.get().await else { return unavailable() };
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let done = async {
        tx.execute("UPDATE users SET admin = (id = ANY($1)) WHERE NOT app AND admin <> (id = ANY($1))", &[&users]).await?;
        tx.query_one("SELECT count(*) FROM users WHERE admin AND NOT disabled", &[]).await.map(|r| r.get::<_, i64>(0))
    }
    .await;
    match done {
        Ok(0) => err(StatusCode::CONFLICT, "last_admin", "someone must keep the Admin role"),
        Ok(_) if tx.commit().await.is_ok() => Json(json!({ "ok": true })).into_response(),
        _ => failed(),
    }
}

/// Grants that used a removed document role fall back to its base.
async fn remove(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let holds: Vec<String> = c
        .query_opt("SELECT perms FROM roles WHERE id = $1", &[&id])
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
        .unwrap_or_default();
    let none = RoleIn { name: None, kind: None, perms: None, base: None, about: None, members: None };
    if let Err(e) = may_change_role(&s, &h, &holds, &none).await {
        return e;
    }
    match c.execute("DELETE FROM roles WHERE id = $1 AND NOT everyone", &[&id]).await {
        Ok(1) => Json(json!({ "ok": true })).into_response(),
        Ok(_) => err(
            StatusCode::BAD_REQUEST,
            "built_in",
            "the Everyone role can't be removed (turn its permissions off instead)",
        ),
        Err(_) => failed(),
    }
}
