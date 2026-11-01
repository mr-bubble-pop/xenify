//! Xenify Plan's register: who and what plans can book, and every plan's
//! bookings, so Plan adds up how busy people are across plans without
//! opening them.
//!
//! - People are the server's accounts ("u<id>"), groups its groups
//!   ("g<id>", members joining or leaving on a day if an admin says so),
//!   things are added by anyone ("t<id>").
//! - Time away is Chat's out of office (`users.away`): one record for both.
//! - The server works every plan out itself (plan/engine.rs: the saved
//!   version and the changes since, through Plan's engine): its bookings
//!   (who, when, how much; a task's title only for people who may open that
//!   plan) and the index below.
//! - The index: every plan's issues (key, title, worked-out dates, status,
//!   labels, who, what each waits for). Other plans wait for them
//!   ("n<plan>:<issue>"), other plans mirror them, and links that would make a
//!   loop across plans are found. Linking follows the plan's access: anyone
//!   who can open a plan can wait for its issues. Someone who can't open it
//!   sees an issue's key, its dates and the plan's name (to ask for access),
//!   never the issue's words.
//! - The company's holidays (country, region, company days) are a setting
//!   (Admin › Holidays) Plan and Chat's calendar both use.

mod engine;
pub use engine::{comments_refused, world_changed};

use crate::files::{me, node_role};
use crate::{Server, admin_power, db, err, failed, signed_in, unavailable};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

pub const SCHEMA: &str = "
ALTER TABLE users ADD COLUMN IF NOT EXISTS away TEXT NOT NULL DEFAULT '';
CREATE TABLE IF NOT EXISTS plan_people (
    user_id BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    day_minutes INT NOT NULL DEFAULT 0,
    country TEXT NOT NULL DEFAULT '',
    region TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS plan_member_dates (
    group_id BIGINT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    from_day TEXT NOT NULL DEFAULT '',
    to_day TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (group_id, user_id)
);
CREATE TABLE IF NOT EXISTS plan_things (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Where each plan was when the server last worked it out.
CREATE TABLE IF NOT EXISTS plan_state (
    node BIGINT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    head BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    built TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS plan_bookings (
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    resource TEXT NOT NULL,
    task TEXT NOT NULL,
    from_day TEXT NOT NULL,
    to_day TEXT NOT NULL,
    percent INT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (node, resource, task)
);
CREATE INDEX IF NOT EXISTS plan_bookings_by_resource ON plan_bookings(resource);
-- One row per piece of work (time away splits an issue into pieces).
ALTER TABLE plan_bookings DROP CONSTRAINT IF EXISTS plan_bookings_pkey;
CREATE UNIQUE INDEX IF NOT EXISTS plan_bookings_piece ON plan_bookings(node, resource, task, from_day);
-- Set to go ahead even where its person is booked in plans made before.
ALTER TABLE plan_bookings ADD COLUMN IF NOT EXISTS over BOOLEAN NOT NULL DEFAULT FALSE;
CREATE TABLE IF NOT EXISTS plan_issues (
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    task TEXT NOT NULL,
    key TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    epic TEXT NOT NULL DEFAULT '',
    placed BOOLEAN NOT NULL DEFAULT FALSE,
    start_day TEXT NOT NULL DEFAULT '',
    finish_day TEXT NOT NULL DEFAULT '',
    milestone BOOLEAN NOT NULL DEFAULT FALSE,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    work INT,
    status TEXT NOT NULL DEFAULT '',
    labels TEXT NOT NULL DEFAULT '[]',
    who TEXT NOT NULL DEFAULT '',
    rank INT NOT NULL DEFAULT 0,
    waits TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (node, task)
);
-- Each plan also has a row for itself as a whole: task '*' (epic '(plan)'),
-- what other plans mirror or wait for when they pick the whole plan.
-- What the index came to (hash) and when that last changed: plans waiting
-- for this one are worked out again only when it did.
ALTER TABLE plan_state ADD COLUMN IF NOT EXISTS hash TEXT NOT NULL DEFAULT '';
ALTER TABLE plan_state ADD COLUMN IF NOT EXISTS changed TIMESTAMPTZ NOT NULL DEFAULT now();
-- Whether a file is a plan, settled when it's made (its details say Plan
-- made it): changing the details later doesn't change it, so a document
-- role's Plan permissions can't be got round by saying it's something else.
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS plan BOOLEAN;
UPDATE nodes SET plan = (kind = 'file' AND meta LIKE '%\"app\":\"plan\"%') WHERE plan IS NULL;
CREATE OR REPLACE FUNCTION xenify_node_plan() RETURNS trigger LANGUAGE plpgsql AS $f$
BEGIN
    IF TG_OP = 'INSERT' OR OLD.plan IS NULL THEN
        NEW.plan := NEW.kind = 'file' AND NEW.meta LIKE '%\"app\":\"plan\"%';
    ELSE
        NEW.plan := OLD.plan;
    END IF;
    RETURN NEW;
END $f$;
DROP TRIGGER IF EXISTS nodes_plan ON nodes;
CREATE TRIGGER nodes_plan BEFORE INSERT OR UPDATE OF plan ON nodes FOR EACH ROW EXECUTE FUNCTION xenify_node_plan();
";

/// Whether file `node` is a plan (settled when it was made).
pub(crate) async fn is_plan(c: &impl deadpool_postgres::GenericClient, node: i64) -> bool {
    c.query_opt("SELECT 1 FROM nodes WHERE id = $1 AND plan", &[&node]).await.ok().flatten().is_some()
}

/// Plan `node`'s file as it is after change `upto` (its version in use,
/// then the changes since): what the server saves for someone whose
/// document role lets them make only some changes, instead of what they sent.
pub(crate) async fn file_at(s: &Server, node: i64, upto: i64) -> Result<Vec<u8>, String> {
    let c = s.pool().get().await.map_err(|e| e.to_string())?;
    let e = |e: tokio_postgres::Error| e.to_string();
    let version = c
        .query_opt(
            "SELECT v.blob, v.upto FROM nodes n JOIN versions v ON v.node = n.id AND v.n = n.head WHERE n.id = $1",
            &[&node],
        )
        .await
        .map_err(e)?;
    let mut p = xenify_plan_core::Session::new();
    let from = match version {
        Some(v) => {
            let bytes = s.blobs.get(v.get::<_, &str>(0)).await.map_err(|e| e.to_string())?;
            p.open_file(&bytes)?;
            v.get::<_, i64>(1)
        }
        None => {
            p.fresh("");
            0
        }
    };
    let rows = c
        .query("SELECT data FROM ops WHERE node = $1 AND seq > $2 AND seq <= $3 ORDER BY seq", &[&node, &from, &upto])
        .await
        .map_err(e)?;
    for r in rows {
        if let Ok(step) = serde_json::from_str::<xenify_plan_core::Step>(r.get::<_, &str>(0)) {
            xenify_plan_core::ops::apply_lenient(&mut p.plan, &step.ops);
        }
    }
    // As the app saves a plan on a server (its log kept).
    xenify_plan_core::file::save(&p.plan, None)
}

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/plan/register", get(register))
        .route("/v1/plan/things", axum::routing::post(add_thing))
        .route("/v1/plan/things/{id}", put(rename_thing).delete(delete_thing))
        .route("/v1/plan/people/{user}", put(put_person))
        .route("/v1/plan/away/{user}", put(put_away))
        .route("/v1/plan/groups/{group}/dates", put(put_member_dates))
        .route("/v1/plan/plans", get(plans))
        .route("/v1/plan/issues/{node}", get(issues_of))
        .route("/v1/plan/ask/{node}", get(ask_for))
        .route("/v1/plan/outside", axum::routing::post(outside))
        .route("/v1/plan/waiting/{node}", get(waiting))
        .route("/v1/plan/loop", axum::routing::post(would_loop))
        .route("/v1/plan/holidays", get(get_holidays))
        .route("/v1/plan/holidays/{year}", get(holiday_days))
        .route("/v1/plan/countries", get(countries))
        .route("/v1/admin/plan/holidays", put(put_holidays))
}

fn id_num(s: &str, prefix: char) -> Option<i64> {
    s.strip_prefix(prefix)?.parse().ok()
}

fn valid_day(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.chars().filter(|c| c.is_ascii_digit()).count() == 8
}

/// Days since 1970 → Unix ms at midnight UTC (Chat keeps out of office in ms).
fn day_ms(day: &str) -> Option<i64> {
    if !valid_day(day) {
        return None;
    }
    let y: i64 = day[..4].parse().ok()?;
    let m: i64 = day[5..7].parse().ok()?;
    let d: i64 = day[8..].parse().ok()?;
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe - 719468) * 86_400_000)
}

fn ms_day(ms: i64) -> String {
    let z = ms.div_euclid(86_400_000) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", if m <= 2 { y + 1 } else { y })
}

/// The server's people (with their hours, regions and time away), groups
/// (with who joins and leaves when) and things, as Plan's engine takes them.
async fn register_world(
    c: &deadpool_postgres::Client,
) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>), tokio_postgres::Error> {
    let rows = c
        .query(
            "SELECT u.id, u.username, u.display_name, u.away, p.day_minutes, p.country, p.region
             FROM users u LEFT JOIN plan_people p ON p.user_id = u.id
             WHERE NOT u.app AND NOT u.disabled ORDER BY lower(coalesce(nullif(u.display_name, ''), u.username))",
            &[],
        )
        .await?;
    let people: Vec<Value> = rows
        .iter()
        .map(|r| {
            let away: String = r.get(3);
            let times: Vec<Value> = serde_json::from_str::<Value>(&away)
                .ok()
                .and_then(|v| v["times"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|t| Some(json!([ms_day(t[0].as_i64()?), ms_day(t[1].as_i64()? - 1)])))
                .collect();
            let name: String = r.get(2);
            json!({
                "id": format!("u{}", r.get::<_, i64>(0)), "kind": "person", "account": r.get::<_, i64>(0),
                "name": if name.is_empty() { r.get::<_, String>(1) } else { name },
                "day_minutes": r.get::<_, Option<i32>>(4).unwrap_or(0), "country": r.get::<_, Option<String>>(5).unwrap_or_default(),
                "region": r.get::<_, Option<String>>(6).unwrap_or_default(), "away": times,
            })
        })
        .collect();
    let groups = c.query("SELECT id, name FROM groups ORDER BY lower(name)", &[]).await.unwrap_or_default();
    let mut gs = Vec::new();
    for g in groups {
        let gid: i64 = g.get(0);
        let members = c
            .query(
                "SELECT m.user_id, d.from_day, d.to_day FROM group_members m
                 LEFT JOIN plan_member_dates d ON d.group_id = m.group_id AND d.user_id = m.user_id WHERE m.group_id = $1",
                &[&gid],
            )
            .await
            .unwrap_or_default();
        let members: Vec<Value> = members
            .iter()
            .map(|m| {
                let f: Option<String> = m.get(1);
                let t: Option<String> = m.get(2);
                json!({ "person": format!("u{}", m.get::<_, i64>(0)), "from": f.filter(|x| !x.is_empty()), "to": t.filter(|x| !x.is_empty()) })
            })
            .collect();
        gs.push(json!({ "id": format!("g{gid}"), "kind": "group", "name": g.get::<_, String>(1), "members": members }));
    }
    let things: Vec<Value> = c
        .query("SELECT id, name FROM plan_things ORDER BY id", &[])
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| json!({ "id": format!("t{}", r.get::<_, i64>(0)), "kind": "thing", "name": r.get::<_, String>(1) }))
        .collect();
    Ok((people, gs, things))
}

/// Everything Plan needs from the server, for the signed-in person.
async fn register(State(s): State<Server>, h: HeaderMap) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    engine::refresh(&s, None).await;
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    let Ok((people, gs, things)) = register_world(&c).await else { return failed() };
    let bookings = c
        .query(
            "SELECT b.node, b.resource, b.task, b.from_day, b.to_day, b.percent, b.title, b.over, n.meta
             FROM plan_bookings b JOIN nodes n ON n.id = b.node",
            &[],
        )
        .await
        .unwrap_or_default();
    let mut readable: std::collections::HashMap<i64, bool> = std::collections::HashMap::new();
    // Your own bookings (and your groups') show their titles even in plans you can't open.
    let mine: Vec<String> =
        std::iter::once(format!("u{}", me.account.id)).chain(me.groups.iter().map(|g| format!("g{g}"))).collect();
    let mut out = Vec::new();
    for b in bookings {
        let node: i64 = b.get(0);
        let may = match readable.get(&node) {
            Some(x) => *x,
            None => {
                let x = node_role(&c, &me, node).await.ok().flatten().is_some_and(|(_, r)| r.is_some());
                readable.insert(node, x);
                x
            }
        };
        let resource: String = b.get(1);
        let shown = may || mine.contains(&resource);
        out.push(json!({
            "node": node, "resource": resource, "task": b.get::<_, String>(2),
            "from": b.get::<_, String>(3), "to": b.get::<_, String>(4), "percent": b.get::<_, i32>(5),
            "title": if shown { b.get::<_, String>(6) } else { String::new() }, "readable": may,
            "over": b.get::<_, bool>(7), "plan": if may { plan_name(&b.get::<_, String>(8)) } else { String::new() },
        }));
    }
    let holidays = holidays(&c).await;
    Json(json!({ "people": people, "groups": gs, "things": things, "bookings": out, "holidays": holidays, "me": format!("u{}", me.account.id) })).into_response()
}

async fn holidays(c: &deadpool_postgres::Client) -> Value {
    db::setting(c, "plan_holidays")
        .await
        .ok()
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({ "country": "", "region": "", "company_days": [] }))
}

async fn get_holidays(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = signed_in(&s, &h).await {
        return e;
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    Json(holidays(&c).await).into_response()
}

/// Countries and regions with built-in holidays.
async fn countries() -> Response {
    Json(json!(xenify_holidays::countries())).into_response()
}

/// The server's days off in `year`: public holidays for its region, and company days.
async fn holiday_days(State(s): State<Server>, h: HeaderMap, Path(year): Path<i32>) -> Response {
    if let Err(e) = signed_in(&s, &h).await {
        return e;
    }
    if !(1900..=2200).contains(&year) {
        return err(StatusCode::BAD_REQUEST, "bad_request", "pick a year from 1900 to 2200");
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let set = holidays(&c).await;
    let mut out: Vec<Value> = xenify_holidays::holidays(
        set["country"].as_str().unwrap_or(""),
        set["region"].as_str().unwrap_or(""),
        year,
    )
    .into_iter()
    .map(|h| json!({ "from": h.date, "to": h.date, "name": h.name, "moved_from": h.moved_from, "company": false }))
    .collect();
    for d in set["company_days"].as_array().cloned().unwrap_or_default() {
        let (f, t) = (d[0].as_str().unwrap_or(""), d[1].as_str().unwrap_or(""));
        if f.starts_with(&year.to_string()) || t.starts_with(&year.to_string()) {
            out.push(json!({ "from": f, "to": t, "name": d[2], "company": true }));
        }
    }
    out.sort_by(|a, b| a["from"].as_str().cmp(&b["from"].as_str()));
    Json(json!(out)).into_response()
}

#[derive(Deserialize)]
struct Holidays {
    country: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    company_days: Vec<(String, String, String)>,
}

async fn put_holidays(State(s): State<Server>, h: HeaderMap, Json(b): Json<Holidays>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.holidays").await {
        return e;
    }
    if b.country.len() > 3 || b.region.len() > 40 || b.company_days.len() > 200 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "that's too much");
    }
    let mut days = Vec::new();
    for (f, t, n) in b.company_days {
        if !valid_day(&f) || !valid_day(&t) || t < f || n.chars().count() > 80 {
            return err(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "a company day needs a first and last day, and a short name",
            );
        }
        days.push(json!([f, t, n.trim()]));
    }
    let v = json!({ "country": b.country.to_uppercase(), "region": b.region, "company_days": days });
    let Ok(c) = s.pool().get().await else { return unavailable() };
    if db::set_setting(&c, "plan_holidays", &v.to_string()).await.is_err() {
        return failed();
    }
    engine::world_changed(&s).await;
    Json(v).into_response()
}

#[derive(Deserialize)]
struct ThingIn {
    name: String,
}

async fn add_thing(State(s): State<Server>, h: HeaderMap, Json(b): Json<ThingIn>) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if b.name.trim().is_empty() || b.name.len() > 2000 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "a thing needs a name");
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    match c
        .query_one(
            "INSERT INTO plan_things (name, created_by) VALUES ($1, $2) RETURNING id",
            &[&b.name.trim(), &account.id],
        )
        .await
    {
        Ok(r) => Json(json!({ "id": format!("t{}", r.get::<_, i64>(0)) })).into_response(),
        Err(_) => failed(),
    }
}

/// Who may rename or delete thing `n`: whoever added it, or someone whose
/// role allows changing people's hours (admins too).
async fn may_change_thing(s: &Server, h: &HeaderMap, n: i64) -> Result<(), Response> {
    let a = signed_in(s, h).await?;
    let c = s.pool().get().await.map_err(|_| unavailable())?;
    let Ok(Some(r)) = c.query_opt("SELECT created_by FROM plan_things WHERE id = $1", &[&n]).await else {
        return Err(err(StatusCode::NOT_FOUND, "not_found", "no such thing"));
    };
    if r.get::<_, Option<i64>>(0) == Some(a.id) || crate::perms::can(&c, a.id, "plan.people").await {
        return Ok(());
    }
    Err(err(
        StatusCode::FORBIDDEN,
        "not_yours",
        "only whoever added it, or someone whose role allows changing people’s hours, can change that",
    ))
}

async fn rename_thing(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<ThingIn>,
) -> Response {
    let Some(n) = id_num(&id, 't') else { return err(StatusCode::NOT_FOUND, "not_found", "no such thing") };
    if let Err(e) = may_change_thing(&s, &h, n).await {
        return e;
    }
    if b.name.trim().is_empty() || b.name.len() > 2000 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "a thing needs a name");
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    match c.execute("UPDATE plan_things SET name = $2 WHERE id = $1", &[&n, &b.name.trim()]).await {
        Ok(1) => Json(json!({ "id": id })).into_response(),
        Ok(_) => err(StatusCode::NOT_FOUND, "not_found", "no such thing"),
        Err(_) => failed(),
    }
}

async fn delete_thing(State(s): State<Server>, h: HeaderMap, Path(id): Path<String>) -> Response {
    let Some(n) = id_num(&id, 't') else { return err(StatusCode::NOT_FOUND, "not_found", "no such thing") };
    if let Err(e) = may_change_thing(&s, &h, n).await {
        return e;
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    match c.execute("DELETE FROM plan_things WHERE id = $1", &[&n]).await {
        Ok(_) => Json(json!({})).into_response(),
        Err(_) => failed(),
    }
}

/// Only the person, or an admin.
async fn self_or_admin(s: &Server, h: &HeaderMap, user: &str) -> Result<i64, Response> {
    let a = signed_in(s, h).await?;
    let Some(u) = id_num(user, 'u') else { return Err(err(StatusCode::NOT_FOUND, "not_found", "no such person")) };
    if a.id != u {
        let c = s.pool.get().await.map_err(|_| crate::unavailable())?;
        if !crate::perms::can(&c, a.id, "plan.people").await {
            return Err(err(
                StatusCode::FORBIDDEN,
                "not_yours",
                "only they, or someone whose role allows changing people’s hours, can change that",
            ));
        }
    }
    Ok(u)
}

#[derive(Deserialize)]
struct PersonIn {
    #[serde(default)]
    day_minutes: i32,
    #[serde(default)]
    country: String,
    #[serde(default)]
    region: String,
}

async fn put_person(
    State(s): State<Server>,
    h: HeaderMap,
    Path(user): Path<String>,
    Json(b): Json<PersonIn>,
) -> Response {
    let u = match self_or_admin(&s, &h, &user).await {
        Ok(u) => u,
        Err(e) => return e,
    };
    if !(0..=1440).contains(&b.day_minutes) || b.country.len() > 3 || b.region.len() > 40 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "hours a day go from 0 to 24");
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    match c
        .execute(
            "INSERT INTO plan_people (user_id, day_minutes, country, region) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id) DO UPDATE SET day_minutes = $2, country = $3, region = $4",
            &[&u, &b.day_minutes, &b.country.to_uppercase(), &b.region],
        )
        .await
    {
        Ok(_) => {
            engine::world_changed(&s).await;
            Json(json!({})).into_response()
        }
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct AwayIn {
    /// First and last days away.
    times: Vec<(String, String)>,
}

/// Time away: the same out of office Chat shows (its automatic reply and
/// notice stay as Chat set them).
async fn put_away(State(s): State<Server>, h: HeaderMap, Path(user): Path<String>, Json(b): Json<AwayIn>) -> Response {
    let u = match self_or_admin(&s, &h, &user).await {
        Ok(u) => u,
        Err(e) => return e,
    };
    if b.times.len() > 100 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "that's too much time away");
    }
    let mut times = Vec::new();
    for (f, t) in &b.times {
        let (Some(a), Some(z)) = (day_ms(f), day_ms(t)) else {
            return err(StatusCode::BAD_REQUEST, "bad_request", "time away needs a first and last day");
        };
        if z < a {
            return err(StatusCode::BAD_REQUEST, "bad_request", "the last day comes before the first");
        }
        times.push(json!([a, z + 86_400_000]));
    }
    times.sort_by_key(|t| t[0].as_i64());
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let old: String = c
        .query_opt("SELECT away FROM users WHERE id = $1", &[&u])
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
        .unwrap_or_default();
    let mut v: Value = serde_json::from_str(&old).unwrap_or_else(|_| json!({}));
    v["user"] = json!(u);
    v["times"] = json!(times);
    if v.get("text").is_none() {
        v["text"] = json!("");
    }
    let text =
        if times.is_empty() && v["text"].as_str().unwrap_or("").is_empty() { String::new() } else { v.to_string() };
    match c.execute("UPDATE users SET away = $2 WHERE id = $1", &[&u, &text]).await {
        Ok(_) => {
            engine::world_changed(&s).await;
            Json(json!({ "times": b.times })).into_response()
        }
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct MemberDates {
    person: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

/// When someone joins or leaves a group, for planning (admins).
async fn put_member_dates(
    State(s): State<Server>,
    h: HeaderMap,
    Path(group): Path<String>,
    Json(b): Json<MemberDates>,
) -> Response {
    if let Err(e) = crate::perms::need(&s, &h, "plan.people").await {
        return e;
    }
    let (Some(g), Some(u)) = (id_num(&group, 'g'), id_num(&b.person, 'u')) else {
        return err(StatusCode::NOT_FOUND, "not_found", "no such group or person");
    };
    let f = b.from.unwrap_or_default();
    let t = b.to.unwrap_or_default();
    if (!f.is_empty() && !valid_day(&f)) || (!t.is_empty() && !valid_day(&t)) {
        return err(StatusCode::BAD_REQUEST, "bad_request", "dates look like 2026-12-18");
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let r = if f.is_empty() && t.is_empty() {
        c.execute("DELETE FROM plan_member_dates WHERE group_id = $1 AND user_id = $2", &[&g, &u]).await
    } else {
        c.execute(
            "INSERT INTO plan_member_dates (group_id, user_id, from_day, to_day) VALUES ($1, $2, $3, $4)
             ON CONFLICT (group_id, user_id) DO UPDATE SET from_day = $3, to_day = $4",
            &[&g, &u, &f, &t],
        )
        .await
    };
    match r {
        Ok(_) => {
            engine::world_changed(&s).await;
            Json(json!({})).into_response()
        }
        Err(_) => failed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_ms() {
        assert_eq!(day_ms("1970-01-02"), Some(86_400_000));
        assert_eq!(ms_day(day_ms("2026-12-21").unwrap()), "2026-12-21");
        assert_eq!(ms_day(day_ms("2027-01-01").unwrap() - 1), "2026-12-31");
        assert!(!valid_day("2026-1-01"));
    }
}

// ----- Plans waiting for and mirroring each other -----

/// A plan's name (from its details), or "" when it can't be read.
fn plan_name(meta: &str) -> String {
    serde_json::from_str::<Value>(meta).ok().and_then(|v| v["name"].as_str().map(String::from)).unwrap_or_default()
}

/// May `me` open plan `node`?
async fn readable(c: &deadpool_postgres::Client, me: &crate::files::Me, node: i64) -> bool {
    node_role(c, me, node).await.ok().flatten().is_some_and(|(_, r)| r.is_some())
}

/// Whether a plan `me` may open waits for or mirrors something in plan
/// `node`: then they may see its name, whom to ask, and its issues' keys and
/// dates (never their words), as that plan shows them.
async fn referred(c: &deadpool_postgres::Client, me: &crate::files::Me, node: i64) -> bool {
    let Ok(rows) = c
        .query(
            "SELECT DISTINCT i.node FROM plan_issues i CROSS JOIN LATERAL jsonb_array_elements_text(i.waits::jsonb) w
             JOIN nodes n ON n.id = i.node
             WHERE i.node <> $1 AND n.trashed IS NULL AND w LIKE $2 LIMIT 500",
            &[&node, &format!("n{node}:%")],
        )
        .await
    else {
        return false;
    };
    for r in rows {
        if readable(c, me, r.get(0)).await {
            return true;
        }
    }
    false
}

/// Who to ask for access to plan `node`: the person whose own files it's
/// in, or the shared space it's in.
async fn ask_whom(c: &deadpool_postgres::Client, node: i64) -> String {
    c.query_opt(
        "SELECT CASE WHEN sp.kind = 'personal' THEN coalesce(nullif(u.display_name, ''), u.username) ELSE sp.name END
         FROM nodes n JOIN spaces sp ON sp.id = n.space LEFT JOIN users u ON u.id = sp.owner WHERE n.id = $1",
        &[&node],
    )
    .await
    .ok()
    .flatten()
    .and_then(|r| r.get::<_, Option<String>>(0))
    .unwrap_or_default()
}

fn day(s: String) -> Value {
    if s.is_empty() { Value::Null } else { json!(s) }
}

/// One indexed issue as JSON: everything for someone who may open its plan;
/// otherwise its key and dates only.
fn issue_json(r: &tokio_postgres::Row, readable: bool) -> Value {
    let mut v = json!({
        "task": r.get::<_, String>("task"), "key": r.get::<_, String>("key"), "placed": r.get::<_, bool>("placed"),
        "start": day(r.get("start_day")), "finish": day(r.get("finish_day")), "milestone": r.get::<_, bool>("milestone"),
        "done": r.get::<_, bool>("done"), "work": r.get::<_, Option<i32>>("work"), "readable": readable,
    });
    if readable {
        v["title"] = json!(r.get::<_, String>("title"));
        v["epic"] = json!(r.get::<_, String>("epic"));
        v["status"] = json!(r.get::<_, String>("status"));
        v["labels"] = serde_json::from_str(&r.get::<_, String>("labels")).unwrap_or(json!([]));
        v["who"] = json!(r.get::<_, String>("who"));
    }
    v
}

const ISSUE_COLS: &str =
    "node, task, key, title, epic, placed, start_day, finish_day, milestone, done, work, status, labels, who, rank";

/// The plans on the server the signed-in person can open (to link to or mirror).
async fn plans(State(s): State<Server>, h: HeaderMap) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    engine::refresh(&s, None).await;
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    let Ok(rows) = c
        .query(
            "SELECT n.id, n.meta, (SELECT count(*) FROM plan_issues i WHERE i.node = n.id AND i.epic NOT IN ('(epic)', '(plan)'))
             FROM nodes n WHERE n.kind = 'file' AND n.trashed IS NULL AND n.meta LIKE '%\"app\":\"plan\"%' ORDER BY n.changed DESC LIMIT 2000",
            &[],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        let node: i64 = r.get(0);
        if readable(&c, &me, node).await {
            out.push(json!({ "node": node, "name": plan_name(&r.get::<_, String>(1)), "issues": r.get::<_, i64>(2) }));
        }
    }
    Json(json!(out)).into_response()
}

/// A plan you were sent a link to but can't open: its name and whom to ask
/// (as other plans' "waits for" show it).
/// Only for someone who may open it, or a plan of theirs that waits for or
/// mirrors it (others can't go through the plans one by one).
async fn ask_for(State(s): State<Server>, h: HeaderMap, Path(node): Path<i64>) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    let Ok(Some(r)) = c.query_opt("SELECT meta FROM nodes WHERE id = $1 AND kind = 'file' AND trashed IS NULL AND meta LIKE '%\"app\":\"plan\"%'", &[&node]).await else {
        return err(StatusCode::NOT_FOUND, "not_found", "no such plan");
    };
    if !readable(&c, &me, node).await && !referred(&c, &me, node).await {
        return err(StatusCode::NOT_FOUND, "not_found", "no such plan");
    }
    Json(json!({ "name": plan_name(&r.get::<_, String>(0)), "ask": ask_whom(&c, node).await })).into_response()
}

/// Every issue in a plan you can open (picking one to wait for).
async fn issues_of(State(s): State<Server>, h: HeaderMap, Path(node): Path<i64>) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    engine::refresh(&s, Some(&[node])).await;
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    if !readable(&c, &me, node).await {
        return err(StatusCode::NOT_FOUND, "not_found", "no such plan");
    }
    let Ok(rows) =
        c.query(&format!("SELECT {ISSUE_COLS} FROM plan_issues WHERE node = $1 AND task <> '*' ORDER BY rank"), &[&node]).await
    else {
        return failed();
    };
    let meta: String = c
        .query_opt("SELECT meta FROM nodes WHERE id = $1", &[&node])
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
        .unwrap_or_default();
    Json(json!({ "node": node, "name": plan_name(&meta), "issues": rows.iter().map(|r| issue_json(r, true)).collect::<Vec<_>>() })).into_response()
}

/// What someone sees of an issue in plan `node` ("n12:t3"): everything when
/// they may open the plan; otherwise its key and dates, the plan's name and
/// who to ask for access. None: no such issue.
struct Seen {
    readable: bool,
    name: String,
    ask: String,
}

/// `related`: the caller knows `node` is tied to a plan `me` may open
/// (waiting); otherwise one of their plans must wait for or mirror it.
async fn seen_of(
    c: &deadpool_postgres::Client,
    me: &crate::files::Me,
    cache: &mut HashMap<i64, Seen>,
    node: i64,
    related: bool,
) -> bool {
    if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(node) {
        let meta: Option<String> = c
            .query_opt(
                "SELECT meta FROM nodes WHERE id = $1 AND kind = 'file' AND trashed IS NULL AND meta LIKE '%\"app\":\"plan\"%'",
                &[&node],
            )
            .await
            .ok()
            .flatten()
            .map(|r| r.get(0));
        // Only plans: other files' names stay with those who may open them.
        let Some(meta) = meta else { return false };
        let r = readable(c, me, node).await;
        if !r && !related && !referred(c, me, node).await {
            return false;
        }
        let ask = if r { String::new() } else { ask_whom(c, node).await };
        e.insert(Seen { readable: r, name: plan_name(&meta), ask });
    }
    true
}

async fn outside_of(
    c: &deadpool_postgres::Client,
    me: &crate::files::Me,
    cache: &mut HashMap<i64, Seen>,
    r: &str,
    related: bool,
) -> Value {
    let Some((n, task)) = r.split_once(':') else { return json!({ "gone": true }) };
    let Some(node) = id_num(n, 'n') else { return json!({ "gone": true }) };
    if !seen_of(c, me, cache, node, related).await {
        return json!({ "gone": true });
    }
    let seen = &cache[&node];
    let row = c
        .query_opt(&format!("SELECT {ISSUE_COLS} FROM plan_issues WHERE node = $1 AND task = $2"), &[&node, &task])
        .await
        .ok()
        .flatten();
    match row {
        Some(row) => {
            let mut v = issue_json(&row, seen.readable);
            v["plan"] = json!(seen.name);
            if !seen.readable {
                v["ask"] = json!(seen.ask);
            }
            v
        }
        None => json!({ "gone": true, "readable": seen.readable, "plan": seen.name }),
    }
}

/// Issues in other plans that wait for plan `node`'s issues ("Holds up"),
/// by its issue: each as `outside` shows it (words only if you may open it).
async fn waiting(State(s): State<Server>, h: HeaderMap, Path(node): Path<i64>) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    if !readable(&c, &me, node).await {
        return err(StatusCode::NOT_FOUND, "not_found", "no such plan");
    }
    let prefix = format!("n{node}:");
    let Ok(rows) = c
        .query("SELECT node, task, w FROM plan_issues, jsonb_array_elements_text(waits::jsonb) w WHERE node <> $1 AND task <> '*' AND w LIKE $2 || '%'", &[&node, &prefix])
        .await
    else {
        return failed();
    };
    let mut cache = HashMap::new();
    let mut out: serde_json::Map<String, Value> = serde_json::Map::new();
    for r in rows {
        let (other, task, w): (i64, String, String) = (r.get(0), r.get(1), r.get(2));
        // They wait for this plan's issues: what of them shows, as they show this one.
        let mut v = outside_of(&c, &me, &mut cache, &format!("n{other}:{task}"), true).await;
        v["ref"] = json!(format!("n{other}:{task}"));
        let mine = w[prefix.len()..].to_string();
        if let Some(Value::Array(a)) = out.get_mut(&mine) {
            a.push(v);
        } else {
            out.insert(mine, json!([v]));
        }
    }
    Json(Value::Object(out)).into_response()
}

#[derive(Deserialize)]
struct Refs {
    refs: Vec<String>,
    /// The plan asking: refs that wait (through other plans) for it are marked `looped`.
    #[serde(default)]
    node: Option<i64>,
}

/// Issues in other plans that a plan waits for ("n12:t3"), as the
/// signed-in person may see them: key, dates and the plan's name always;
/// the issue's words only when they may open that plan.
async fn outside(State(s): State<Server>, h: HeaderMap, Json(b): Json<Refs>) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if b.refs.len() > 2000 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "that's too many");
    }
    let mut nodes: Vec<i64> =
        b.refs.iter().filter_map(|r| r.split_once(':').and_then(|(n, _)| id_num(n, 'n'))).collect();
    // The asking plan too: what it waits for decides what it may see.
    nodes.extend(b.node);
    engine::refresh(&s, Some(&nodes)).await;
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    let mut cache = HashMap::new();
    let mut out = serde_json::Map::new();
    for r in &b.refs {
        let mut v = outside_of(&c, &me, &mut cache, r, false).await;
        if let Some(n) = b.node
            && loops_back(&c, r, n).await
        {
            v["looped"] = json!(true);
        }
        out.insert(r.clone(), v);
    }
    Json(Value::Object(out)).into_response()
}

/// Does `start` ("n7:t3", or a whole plan "n7:*") wait, through any
/// plans, for something in plan `node`? (A whole plan waits for whatever
/// its issues wait for.)
pub async fn loops_back(c: &deadpool_postgres::Client, start: &str, node: i64) -> bool {
    let mine = format!("n{node}:");
    if start.starts_with(&mine) {
        return false;
    }
    let mut queue = std::collections::VecDeque::from([start.to_string()]);
    let mut seen = std::collections::HashSet::from([start.to_string()]);
    while let Some(r) = queue.pop_front() {
        if seen.len() > 20000 {
            return false;
        }
        let Some((n, task)) = r.split_once(':') else { continue };
        let Some(on) = id_num(n, 'n') else { continue };
        let Ok(Some(row)) =
            c.query_opt("SELECT waits FROM plan_issues WHERE node = $1 AND task = $2", &[&on, &task]).await
        else {
            continue;
        };
        for w in serde_json::from_str::<Vec<String>>(&row.get::<_, String>(0)).unwrap_or_default() {
            if w.starts_with(&mine) {
                return true;
            }
            if seen.insert(w.clone()) {
                queue.push_back(w);
            }
        }
    }
    false
}

#[derive(Deserialize)]
struct LoopIn {
    /// The issue that would be waited for, and the one that would wait ("n<plan>:<issue>").
    from: String,
    to: String,
}

/// Would "`to` waits for `from`" make a loop? It does when `from` already
/// waits, through any plans, for `to`. Then the whole loop comes back, issue
/// by issue (each as `outside` shows it, so nothing you may not read), to
/// see where to break it.
async fn would_loop(State(s): State<Server>, h: HeaderMap, Json(b): Json<LoopIn>) -> Response {
    let account = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let nodes: Vec<i64> =
        [&b.from, &b.to].iter().filter_map(|r| r.split_once(':').and_then(|(n, _)| id_num(n, 'n'))).collect();
    engine::refresh(&s, Some(&nodes)).await;
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(me) = me(&c, account).await else { return failed() };
    // Breadth first, so the loop shown is the shortest.
    let mut came: HashMap<String, String> = HashMap::new();
    let mut queue = std::collections::VecDeque::from([b.from.clone()]);
    let mut seen = std::collections::HashSet::from([b.from.clone()]);
    let mut found = false;
    // Waiting for a whole plan ("n7:*") waits for every issue in it.
    let whole = b.to.strip_suffix('*').map(String::from);
    // Where the search came round: `to`, a whole plan with `to` in it, or
    // (`to` a whole plan) an issue in it.
    let mut hit = b.to.clone();
    while let Some(r) = queue.pop_front() {
        let all_of = r.strip_suffix('*').is_some_and(|p| b.to.starts_with(p));
        if r == b.to || all_of || whole.as_deref().is_some_and(|p| r.starts_with(p) && r != b.from) {
            hit = r;
            found = true;
            break;
        }
        if seen.len() > 20000 {
            break;
        }
        let Some((n, task)) = r.split_once(':') else { continue };
        let Some(node) = id_num(n, 'n') else { continue };
        let Ok(Some(row)) =
            c.query_opt("SELECT waits FROM plan_issues WHERE node = $1 AND task = $2", &[&node, &task]).await
        else {
            continue;
        };
        for w in serde_json::from_str::<Vec<String>>(&row.get::<_, String>(0)).unwrap_or_default() {
            if seen.insert(w.clone()) {
                came.insert(w.clone(), r.clone());
                queue.push_back(w);
            }
        }
    }
    if !found {
        return Json(json!({ "loop": false })).into_response();
    }
    // from → … → to, then back to from (the new link closes it).
    let mut path = vec![hit.clone()];
    while let Some(p) = came.get(path.last().unwrap()) {
        path.push(p.clone());
    }
    path.reverse();
    // Came round through a whole plan with `to` in it: `to` closes the loop.
    if hit != b.to && hit.ends_with(":*") {
        path.push(b.to.clone());
    }
    let mut cache = HashMap::new();
    let mut chain = Vec::new();
    for r in &path {
        let mut v = outside_of(&c, &me, &mut cache, r, false).await;
        v["ref"] = json!(r);
        chain.push(v);
    }
    Json(json!({ "loop": true, "chain": chain })).into_response()
}
