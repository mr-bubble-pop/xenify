//! Data sources for Xenify Calc: other databases (PostgreSQL, MySQL or
//! MariaDB, SQL Server, a SQLite file, a folder of CSV files) that Calc's
//! query sheets read and its table editor sheets change, through this
//! server. Their passwords stay here; apps never see them.
//!
//! - Reading runs as the source's reading user, in a read-only session
//!   where the database has one, one statement at a time, with a time and
//!   a row limit. Read-only is made sure of by that database user's rights
//!   (the admin is told so); the session and the one-statement rule are
//!   extra care, not the guarantee.
//! - Changing (table editor sheets) runs as the separate changing user,
//!   only on the tables the admin lets people edit, and only cell by cell
//!   when the cell still holds what Calc read (see `changes`).
//! - Values for `:names` in a query come from the sheet; they're bound as
//!   values, never written into the SQL.
//! - Connections are encrypted with a checked certificate unless the admin
//!   picks otherwise for a source (`tls`); sources from before that setting
//!   connect as they did, and Admin › Data sources says they aren't
//!   encrypted. A password never goes unencrypted to another computer.
//! - Sources reach only what they're let: files in the folders an admin
//!   picks (the data folder at first), and no address on this computer, its
//!   link-local network or the server's own database, unless an admin ticks
//!   Allow local addresses for that source. Saving a new address forgets
//!   the passwords kept for the old one.
//!
//! The server holds the passwords and sees the results: Admin › Data
//! sources says so before one is added.
//!
//! Who may do what:
//! - People with the "set up database connections" permission (and admins)
//!   add and change every source, and write queries on each.
//! - Each source lists who else may write queries on it (people, groups).
//! - Writing a query, or picking a saved one, in a file is for them. The
//!   server then hands back a ticket for exactly that query (or editable
//!   table) on that source, kept in the file; anyone signed in who has the
//!   file can run it again with it. Without a ticket nobody else can run
//!   anything, and a ticket never fits any other SQL.

use crate::{Server, admin_power, db, err, failed, signed_in, unavailable};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS data_sources (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    address TEXT NOT NULL DEFAULT '',
    database TEXT NOT NULL DEFAULT '',
    read_user TEXT NOT NULL DEFAULT '',
    read_password TEXT NOT NULL DEFAULT '',
    write_user TEXT NOT NULL DEFAULT '',
    write_password TEXT NOT NULL DEFAULT '',
    own_queries BOOLEAN NOT NULL DEFAULT FALSE,
    time_limit INT NOT NULL DEFAULT 15,
    row_limit INT NOT NULL DEFAULT 50000,
    groups BIGINT[] NOT NULL DEFAULT '{}',
    editable JSONB NOT NULL DEFAULT '[]',
    notes JSONB NOT NULL DEFAULT '{}',
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS data_queries (
    id BIGSERIAL PRIMARY KEY,
    source BIGINT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sql TEXT NOT NULL,
    notes JSONB NOT NULL DEFAULT '{}',
    author BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS data_log (
    id BIGSERIAL PRIMARY KEY,
    source BIGINT REFERENCES data_sources(id) ON DELETE CASCADE,
    user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    what TEXT NOT NULL,
    detail JSONB NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS data_log_by_source ON data_log(source, at DESC);
-- Who may write queries on a source (besides whoever may set up sources).
-- Before, 'use' and 'manage' were levels too (see below).
CREATE TABLE IF NOT EXISTS data_access (
    source BIGINT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    user_id BIGINT REFERENCES users(id) ON DELETE CASCADE,
    group_id BIGINT REFERENCES groups(id) ON DELETE CASCADE,
    level TEXT NOT NULL CHECK (level IN ('use', 'write', 'manage')),
    CHECK ((user_id IS NULL) <> (group_id IS NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS data_access_user ON data_access(source, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS data_access_group ON data_access(source, group_id) WHERE group_id IS NOT NULL;
-- Sources from before sharing: their groups use them, writing their own
-- queries when the source allowed that.
ALTER TABLE data_sources ADD COLUMN IF NOT EXISTS shared BOOLEAN NOT NULL DEFAULT FALSE;
INSERT INTO data_access (source, group_id, level)
    SELECT s.id, g.id, CASE WHEN s.own_queries THEN 'write' ELSE 'use' END
    FROM data_sources s JOIN groups g ON g.id = ANY(s.groups) WHERE NOT s.shared
    ON CONFLICT DO NOTHING;
UPDATE data_sources SET shared = TRUE WHERE NOT shared;
-- Running a query in a file needs no level of its own any more (its ticket
-- does), and setting sources up is the permission's: what's left is who
-- writes queries.
DELETE FROM data_access WHERE level = 'use';
UPDATE data_access SET level = 'write' WHERE level = 'manage';
-- How to connect: 'require' (encrypted, certificate checked), 'prefer'
-- (encrypted when offered, not checked) or 'off'. A CA certificate for
-- private ones.
ALTER TABLE data_sources ADD COLUMN IF NOT EXISTS tls TEXT NOT NULL DEFAULT 'require';
ALTER TABLE data_sources ALTER COLUMN tls SET DEFAULT 'require';
UPDATE data_sources SET tls = 'require' WHERE tls NOT IN ('require', 'prefer', 'off');
ALTER TABLE data_sources ADD COLUMN IF NOT EXISTS ca_cert TEXT NOT NULL DEFAULT '';
-- Addresses on this computer (and link-local ones) only when an admin
-- allows them.
ALTER TABLE data_sources ADD COLUMN IF NOT EXISTS allow_local BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE data_sources ALTER COLUMN allow_local SET DEFAULT FALSE;
";

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/admin/data", get(admin_list).post(admin_save))
        .route("/v1/admin/data/{id}", axum::routing::delete(admin_delete))
        .route("/v1/admin/data/{id}/test", post(admin_test))
        .route("/v1/admin/data/test", post(admin_test_form))
        .route("/v1/admin/data/folders", get(admin_folders).put(save_folders))
        .route("/v1/admin/data/{id}/queries", get(admin_queries).post(save_query))
        .route("/v1/admin/data/{id}/queries/{q}", axum::routing::delete(delete_query))
        .route("/v1/admin/data/{id}/log", get(admin_log))
        .route("/v1/data", get(app_list))
        .route("/v1/data/{id}/tables", get(app_tables))
        .route("/v1/data/{id}/queries", post(save_query))
        .route("/v1/data/{id}/queries/{q}", axum::routing::delete(delete_query))
        .route("/v1/data/{id}/query", post(app_query))
        .route("/v1/data/{id}/table", post(app_table))
        .route("/v1/data/{id}/changes", post(app_changes))
}

/// A table people may edit: its key columns, the ones they can't change,
/// and whether they may add and delete rows.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Editable {
    pub table: String,
    pub key: Vec<String>,
    pub locked: Vec<String>,
    pub add: bool,
    pub delete: bool,
}

impl Default for Editable {
    fn default() -> Self {
        Editable { table: String::new(), key: Vec::new(), locked: Vec::new(), add: true, delete: false }
    }
}

/// A data source as the admin sets it up (passwords only ever written here).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    pub id: i64,
    pub name: String,
    /// postgres, mysql, mssql, sqlite or csv.
    pub kind: String,
    /// host:port, or the file (SQLite) or folder (CSV) on this computer.
    pub address: String,
    pub database: String,
    pub read_user: String,
    #[serde(skip_serializing)]
    pub read_password: String,
    pub write_user: String,
    #[serde(skip_serializing)]
    pub write_password: String,
    pub time_limit: i32,
    pub row_limit: i32,
    pub editable: Vec<Editable>,
    pub notes: Value,
    /// "require", "prefer" or "off".
    pub tls: String,
    /// PEM certificates of a private certificate authority to trust.
    pub ca_cert: String,
    /// Addresses on this computer and link-local ones are allowed (only an
    /// admin turns this on).
    pub allow_local: bool,
    /// What it may reach (filled in by the server, never saved).
    #[serde(skip)]
    pub guard: Guard,
}

/// What a source may reach besides the open network: the folders file
/// sources may be in, and the server's own database's addresses (refused).
#[derive(Clone, Debug, Default)]
pub struct Guard {
    pub folders: Vec<std::path::PathBuf>,
    pub own_db: Vec<std::net::SocketAddr>,
}

impl Source {
    fn limits(&self) -> (Duration, usize) {
        (Duration::from_secs(self.time_limit.clamp(1, 600) as u64), self.row_limit.clamp(1, 1_000_000) as usize)
    }
}

const KINDS: [&str; 5] = ["postgres", "mysql", "mssql", "sqlite", "csv"];

fn row_to_source(r: &tokio_postgres::Row) -> Source {
    Source {
        id: r.get(0),
        name: r.get(1),
        kind: r.get(2),
        address: r.get(3),
        database: r.get(4),
        read_user: r.get(5),
        read_password: r.get(6),
        write_user: r.get(7),
        write_password: r.get(8),
        time_limit: r.get(9),
        row_limit: r.get(10),
        editable: serde_json::from_value(r.get::<_, Value>(11)).unwrap_or_default(),
        notes: r.get(12),
        tls: r.get(13),
        ca_cert: r.get(14),
        allow_local: r.get(15),
        guard: Guard::default(),
    }
}

const COLS: &str = "id, name, kind, address, database, read_user, read_password, write_user, write_password, time_limit, row_limit, editable, notes, tls, ca_cert, allow_local";

async fn sources(c: &deadpool_postgres::Client) -> db::Result<Vec<Source>> {
    Ok(c.query(&format!("SELECT {COLS} FROM data_sources ORDER BY name"), &[])
        .await?
        .iter()
        .map(row_to_source)
        .collect())
}

async fn source(c: &deadpool_postgres::Client, id: i64) -> db::Result<Option<Source>> {
    Ok(c.query_opt(&format!("SELECT {COLS} FROM data_sources WHERE id = $1"), &[&id])
        .await?
        .as_ref()
        .map(row_to_source))
}

/// How far someone may go with a source, least first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Write queries (their own SQL, or a saved one) in files, and save
    /// queries for others.
    Write,
    /// Also change the connection, who writes queries and every saved query:
    /// whoever may set up database connections.
    Manage,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Write => "write",
            Level::Manage => "manage",
        }
    }
}

type Account = xenify_server_proto::Account;

/// What `user` may do with source `src`: whoever may set up sources (and
/// admins) manages every one; the people (and groups) it lists write queries.
async fn level(c: &impl deadpool_postgres::GenericClient, src: i64, user: &Account) -> Option<Level> {
    if user.admin || crate::perms::can(c, user.id, "admin.data").await {
        return Some(Level::Manage);
    }
    let r = c
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM data_access WHERE source = $1
               AND (user_id = $2 OR group_id IN (SELECT group_id FROM group_members WHERE user_id = $2)))",
            &[&src, &user.id],
        )
        .await
        .ok()?;
    r.get::<_, bool>(0).then_some(Level::Write)
}

/// Someone (a person or a group) who may write queries on a source.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Writer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<i64>,
    /// Their name (for showing; ignored when saving).
    #[serde(default, skip_deserializing)]
    name: String,
}

async fn writers(c: &deadpool_postgres::Client, src: i64) -> Vec<Writer> {
    c.query(
        "SELECT a.user_id, a.group_id, COALESCE(NULLIF(u.display_name, ''), u.username, g.name, '')
         FROM data_access a LEFT JOIN users u ON u.id = a.user_id LEFT JOIN groups g ON g.id = a.group_id
         WHERE a.source = $1 ORDER BY a.group_id IS NULL, 3",
        &[&src],
    )
    .await
    .unwrap_or_default()
    .iter()
    .map(|r| Writer { user: r.get(0), group: r.get(1), name: r.get(2) })
    .collect()
}

/// The source `id`, when `me` may go as far as `want` with it (else why not).
async fn at_least(
    s: &Server,
    h: &HeaderMap,
    id: i64,
    want: Level,
) -> Result<(Source, Account, deadpool_postgres::Client, Level), Response> {
    let me = signed_in(s, h).await?;
    let c = s.pool().get().await.map_err(|_| unavailable())?;
    let src = find(s, &c, id).await?;
    match level(&c, id, &me).await {
        Some(l) if l >= want => Ok((src, me, c, l)),
        None => Err(err(
            StatusCode::FORBIDDEN,
            "not_writer",
            &format!("you can't write queries on {}: ask whoever sets up database connections", src.name),
        )),
        Some(_) => Err(err(
            StatusCode::FORBIDDEN,
            "not_manager",
            &format!("only people who set up database connections can change {}", src.name),
        )),
    }
}

async fn find(s: &Server, c: &deadpool_postgres::Client, id: i64) -> Result<Source, Response> {
    let mut src = source(c, id)
        .await
        .map_err(|_| failed())?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "missing", "no such data source"))?;
    src.guard = guard(s, c).await;
    Ok(src)
}

/// What sources may reach on this server.
async fn guard(s: &Server, c: &deadpool_postgres::Client) -> Guard {
    Guard { folders: folders(s, c).await, own_db: own_db(s).await }
}

/// The folders file sources may be in (settings key `data_folders`; until
/// an admin sets them, the data folder).
async fn folders(s: &Server, c: &deadpool_postgres::Client) -> Vec<std::path::PathBuf> {
    if let Some(v) = db::setting(c, "data_folders").await.ok().flatten()
        && let Ok(list) = serde_json::from_str::<Vec<String>>(&v)
    {
        return list.into_iter().map(Into::into).collect();
    }
    vec![s.data().to_path_buf()]
}

/// The addresses of the server's own PostgreSQL.
async fn own_db(s: &Server) -> Vec<std::net::SocketAddr> {
    let Ok(cfg) = s.backup_paths().database.parse::<tokio_postgres::Config>() else { return Vec::new() };
    let ports = cfg.get_ports();
    let mut out = Vec::new();
    for (k, h) in cfg.get_hosts().iter().enumerate() {
        let port = ports.get(k).or(ports.first()).copied().unwrap_or(5432);
        if let tokio_postgres::config::Host::Tcp(h) = h
            && let Ok(Ok(found)) =
                tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host((h.as_str(), port))).await
        {
            out.extend(found);
        }
        if let Some(ip) = cfg.get_hostaddrs().get(k) {
            out.push(std::net::SocketAddr::new(*ip, port));
        }
    }
    out
}

/// `GET /v1/admin/data/folders`: where file sources may be.
async fn admin_folders(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match admin_power(&s, &h, "admin.data").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let list = folders(&s, &c).await;
    Json(json!({ "folders": list, "data": s.data(), "can_change": me.admin })).into_response()
}

#[derive(Deserialize)]
struct SaveFolders {
    folders: Vec<String>,
}

/// `PUT /v1/admin/data/folders`: only admins pick them.
async fn save_folders(State(s): State<Server>, h: HeaderMap, Json(b): Json<SaveFolders>) -> Response {
    match signed_in(&s, &h).await {
        Ok(me) if me.admin => {}
        Ok(_) => return err(StatusCode::FORBIDDEN, "not_admin", "only admins can pick the folders for files"),
        Err(e) => return e,
    }
    let mut list: Vec<String> = Vec::new();
    for f in b.folders.iter().map(|f| f.trim()).filter(|f| !f.is_empty()) {
        let p = std::path::Path::new(f);
        if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return err(StatusCode::BAD_REQUEST, "folder", &format!("{f}: give the full path, like /srv/data"));
        }
        if !list.iter().any(|x| x == f) {
            list.push(f.to_string());
        }
    }
    let Ok(c) = s.pool().get().await else { return unavailable() };
    if db::set_setting(&c, "data_folders", &serde_json::to_string(&list).unwrap_or_default()).await.is_err() {
        return failed();
    }
    admin_folders(State(s), h).await
}

/// Whether `a` and `b` are the same place: the same kind, and the same
/// host and port (or file). Kept passwords only ever go to the place they
/// were given for.
fn same_place(a: &Source, b: &Source) -> bool {
    if a.kind != b.kind {
        return false;
    }
    let port = match a.kind.as_str() {
        "postgres" => 5432,
        "mysql" => 3306,
        "mssql" => 1433,
        _ => return a.address.trim() == b.address.trim(),
    };
    let (ha, pa) = host_port(a.address.trim(), port);
    let (hb, pb) = host_port(b.address.trim(), port);
    ha.trim_matches(['[', ']']).eq_ignore_ascii_case(hb.trim_matches(['[', ']'])) && pa == pb
}

const TLS_MODES: [&str; 3] = ["require", "prefer", "off"];

/// What a save or a test may use, from what was sent, what's kept (`old`)
/// and who's asking: the TLS setting, Allow local addresses and the kept
/// passwords (only for the same place). Err: why not.
fn settle(b: &SaveSource, old: Option<&Source>, admin: bool) -> Result<Source, Response> {
    let mut x = b.src.clone();
    let moved = old.is_none_or(|o| !same_place(o, &x));
    x.tls = match b.tls.as_deref() {
        None => old.map(|o| o.tls.clone()).unwrap_or_else(|| "require".into()),
        Some(t) if TLS_MODES.contains(&t) => t.to_string(),
        Some(_) => return Err(err(StatusCode::BAD_REQUEST, "tls", "pick how to encrypt the connection")),
    };
    x.ca_cert = x.ca_cert.trim().to_string();
    if !x.ca_cert.is_empty() && ca_certs(&x.ca_cert).is_err() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "ca_cert",
            "the CA certificate should be PEM text, from -----BEGIN CERTIFICATE----- to -----END CERTIFICATE-----",
        ));
    }
    let kept = old.is_some_and(|o| o.allow_local) && !moved;
    x.allow_local = match b.allow_local {
        Some(v) if admin => v,
        Some(true) if !kept => {
            return Err(err(StatusCode::FORBIDDEN, "not_admin", "only admins can allow local addresses"));
        }
        _ => kept,
    };
    x.read_password = b.read_password.clone();
    x.write_password = b.write_password.clone();
    if let Some(o) = old.filter(|_| !moved) {
        if x.read_password.is_empty() {
            x.read_password = o.read_password.clone();
        }
        if x.write_password.is_empty() {
            x.write_password = o.write_password.clone();
        }
    }
    Ok(x)
}

/// Whether the source's place is one it may use (a file in the folders, an
/// address that isn't local unless allowed). An address that can't be found
/// now is let through (the test says so).
async fn place_allowed(x: &Source) -> Result<(), String> {
    match x.kind.as_str() {
        "sqlite" | "csv" => in_folders_by_name(&x.address, &x.guard.folders).map(|_| ()),
        _ => match reach(x, 0).await {
            Err(Why::Blocked) => Err(Why::Blocked.text(&x.kind)),
            _ => Ok(()),
        },
    }
}

// ----- Tickets -----

/// The server's key for tickets (made the first time).
async fn ticket_key(c: &impl deadpool_postgres::GenericClient) -> Option<Vec<u8>> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    if let Some(k) = db::setting(c, "data_ticket_key").await.ok().flatten() {
        return URL_SAFE_NO_PAD.decode(k).ok();
    }
    let mut bytes = [0u8; 32];
    p256::elliptic_curve::rand_core::RngCore::fill_bytes(&mut p256::elliptic_curve::rand_core::OsRng, &mut bytes);
    // Two servers making one at once: the first kept wins.
    c.execute(
        "INSERT INTO settings (key, value) VALUES ('data_ticket_key', $1) ON CONFLICT DO NOTHING",
        &[&URL_SAFE_NO_PAD.encode(bytes)],
    )
    .await
    .ok()?;
    URL_SAFE_NO_PAD.decode(db::setting(c, "data_ticket_key").await.ok().flatten()?).ok()
}

/// What a ticket is for: one saved query, one piece of SQL, or one editable
/// table, on one source. A table's ticket is also for the server file it's
/// in (when it's in one), so its rows are changed only by people who may
/// edit that file, never through a copy of it.
enum For<'a> {
    Query(i64),
    Sql(&'a str),
    Table(&'a str, Option<i64>),
}

fn ticket_for(key: &[u8], src: i64, what: &For) -> String {
    use base64::Engine;
    use hmac::{KeyInit, Mac};
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).expect("any key length");
    let what = match what {
        For::Query(q) => format!("query\n{q}"),
        For::Sql(sql) => format!("sql\n{sql}"),
        For::Table(t, node) => format!("table\n{}\n{}", t.to_lowercase(), node.map(|n| n.to_string()).unwrap_or_default()),
    };
    m.update(format!("xenify data ticket\n{src}\n{what}").as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
}

/// A ticket for `what` on `src` (when `me` may write queries on it, they
/// get one to keep in the file).
async fn ticket(c: &deadpool_postgres::Client, src: i64, what: &For<'_>) -> Option<String> {
    Some(ticket_for(&ticket_key(c).await?, src, what))
}

/// Whether `given` is the ticket for `what` on `src`.
async fn ticket_fits(c: &deadpool_postgres::Client, src: i64, what: &For<'_>, given: &str) -> bool {
    let Some(want) = ticket(c, src, what).await else { return false };
    // Compared in full whatever differs, so timing says nothing.
    want.len() == given.len() && want.bytes().zip(given.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

/// The source `id` for running `what`: for whoever may write queries on it,
/// or anyone signed in with the ticket the file keeps. Also the ticket to
/// keep (for writers).
async fn runnable(
    s: &Server,
    h: &HeaderMap,
    id: i64,
    what: &For<'_>,
    given: Option<&str>,
) -> Result<(Source, Account, deadpool_postgres::Client, Option<String>), Response> {
    let me = signed_in(s, h).await?;
    let c = s.pool().get().await.map_err(|_| unavailable())?;
    let src = find(s, &c, id).await?;
    if level(&c, id, &me).await.is_some() {
        let t = ticket(&c, id, what).await;
        return Ok((src, me, c, t));
    }
    match given {
        Some(t) if ticket_fits(&c, id, what, t).await => Ok((src, me, c, None)),
        Some(_) => Err(err(
            StatusCode::FORBIDDEN,
            "changed",
            "this sheet's query was changed by someone who can't write queries: ask someone who can to run it once",
        )),
        None => Err(err(
            StatusCode::FORBIDDEN,
            "no_ticket",
            &format!("this sheet's query hasn't been run from {} yet by someone who can write queries: ask one of them to run it once", src.name),
        )),
    }
}

/// Whether `me` may edit server file `node`.
async fn may_edit_file(c: &deadpool_postgres::Client, me: &Account, node: Option<i64>) -> bool {
    let Some(node) = node else { return false };
    let Ok(me) = crate::files::me(c, me.clone()).await else { return false };
    matches!(crate::files::node_role(c, &me, node).await, Ok(Some((_, Some(r)))) if r >= xenify_server_proto::Role::Editor)
}

async fn log(c: &deadpool_postgres::Client, src: i64, user: i64, what: &str, detail: Value) {
    let _ = c
        .execute(
            "INSERT INTO data_log (source, user_id, what, detail) VALUES ($1, $2, $3, $4)",
            &[&src, &user, &what, &detail],
        )
        .await;
}

// ----- Managing (Admin › Data sources) -----

/// Every source, with who writes queries on it, for whoever may set them up.
async fn admin_list(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    if !(me.admin || crate::perms::can(&c, me.id, "admin.data").await) {
        return Json(json!([])).into_response();
    }
    let Ok(all) = sources(&c).await else { return failed() };
    let mut out = Vec::new();
    for x in all {
        let mut v = serde_json::to_value(&x).unwrap_or(Value::Null);
        v["read_password_set"] = json!(!x.read_password.is_empty());
        v["write_password_set"] = json!(!x.write_password.is_empty());
        v["writers"] = json!(writers(&c, x.id).await);
        out.push(v);
    }
    Json(out).into_response()
}

#[derive(Deserialize)]
struct SaveSource {
    #[serde(default)]
    id: Option<i64>,
    #[serde(flatten)]
    src: Source,
    /// New passwords (empty: keep the one there).
    #[serde(default)]
    read_password: String,
    #[serde(default)]
    write_password: String,
    /// Who may write queries on it (left out: as it is).
    #[serde(default)]
    writers: Option<Vec<Writer>>,
    /// How to encrypt (left out: as it is; "require" for a new one).
    #[serde(default)]
    tls: Option<String>,
    /// Allow local addresses (only admins turn it on; left out: as it is,
    /// unless the address changed).
    #[serde(default)]
    allow_local: Option<bool>,
}

/// Adds or changes a source (whoever may set up database connections).
async fn admin_save(State(s): State<Server>, h: HeaderMap, Json(b): Json<SaveSource>) -> Response {
    let me = match admin_power(&s, &h, "admin.data").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let old = match b.id {
        Some(id) => match source(&c, id).await {
            Ok(Some(o)) => Some(o),
            Ok(None) => return err(StatusCode::NOT_FOUND, "missing", "no such data source"),
            Err(_) => return failed(),
        },
        None => None,
    };
    let mut x = match settle(&b, old.as_ref(), me.admin) {
        Ok(x) => x,
        Err(e) => return e,
    };
    x.guard = guard(&s, &c).await;
    drop(c);
    // Limits left out: 15 seconds and 50,000 rows.
    if x.time_limit <= 0 {
        x.time_limit = 15;
    }
    if x.row_limit <= 0 {
        x.row_limit = 50_000;
    }
    if x.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name", "give the data source a name");
    }
    if !KINDS.contains(&x.kind.as_str()) {
        return err(
            StatusCode::BAD_REQUEST,
            "kind",
            "pick PostgreSQL, MySQL, SQL Server, a SQLite file or a folder of CSV files",
        );
    }
    if b.writers.iter().flatten().any(|w| w.user.is_some() == w.group.is_some()) {
        return err(StatusCode::BAD_REQUEST, "who", "pick a person or a group");
    }
    if let Err(e) = place_allowed(&x).await {
        return err(StatusCode::BAD_REQUEST, "place", &e);
    }
    let Ok(mut c) = s.pool().get().await else { return unavailable() };
    let Ok(t) = c.transaction().await else { return unavailable() };
    let editable = serde_json::to_value(&x.editable).unwrap_or(json!([]));
    let notes = if x.notes.is_null() { json!({}) } else { x.notes.clone() };
    let r = match b.id {
        Some(id) => t
            .query_one(
                "UPDATE data_sources SET name = $2, kind = $3, address = $4, database = $5, read_user = $6,
                   read_password = $7, write_user = $8, write_password = $9,
                   time_limit = $10, row_limit = $11, editable = $12, notes = $13, tls = $14, ca_cert = $15, allow_local = $16
                 WHERE id = $1 RETURNING id",
                &[&id, &x.name.trim(), &x.kind, &x.address, &x.database, &x.read_user, &x.read_password, &x.write_user, &x.write_password, &x.time_limit.clamp(1, 600), &x.row_limit.clamp(1, 1_000_000), &editable, &notes, &x.tls, &x.ca_cert, &x.allow_local],
            )
            .await,
        None => t
            .query_one(
                "INSERT INTO data_sources (name, kind, address, database, read_user, read_password, write_user, write_password,
                   time_limit, row_limit, editable, notes, shared, tls, ca_cert, allow_local)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, TRUE, $13, $14, $15) RETURNING id",
                &[&x.name.trim(), &x.kind, &x.address, &x.database, &x.read_user, &x.read_password, &x.write_user, &x.write_password, &x.time_limit.clamp(1, 600), &x.row_limit.clamp(1, 1_000_000), &editable, &notes, &x.tls, &x.ca_cert, &x.allow_local],
            )
            .await,
    };
    let id: i64 = match r {
        Ok(row) => row.get(0),
        Err(e) if e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) => {
            return err(StatusCode::CONFLICT, "taken", "there's already a data source with that name");
        }
        Err(_) => return failed(),
    };
    if let Some(list) = &b.writers {
        if t.execute("DELETE FROM data_access WHERE source = $1", &[&id]).await.is_err() {
            return failed();
        }
        for w in list {
            if t.execute(
                "INSERT INTO data_access (source, user_id, group_id, level) VALUES ($1, $2, $3, 'write')
                 ON CONFLICT DO NOTHING",
                &[&id, &w.user, &w.group],
            )
            .await
            .is_err()
            {
                return err(StatusCode::BAD_REQUEST, "who", "someone picked to write queries isn't on this server");
            }
        }
    }
    match t.commit().await {
        Ok(()) => Json(json!({ "id": id })).into_response(),
        Err(_) => failed(),
    }
}

async fn admin_delete(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let c = match at_least(&s, &h, id, Level::Manage).await {
        Ok((_, _, c, _)) => c,
        Err(e) => return e,
    };
    match c.execute("DELETE FROM data_sources WHERE id = $1", &[&id]).await {
        Ok(_) => Json(json!({})).into_response(),
        Err(_) => failed(),
    }
}

/// Tries the source: reads its tables as the reading user, and checks the
/// changing user can connect.
async fn admin_test(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    match at_least(&s, &h, id, Level::Manage).await {
        Ok((src, ..)) => test_source(&src).await,
        Err(e) => e,
    }
}

/// Tries a source as typed in the form, before it's saved (with the kept
/// passwords for any left empty when changing one).
async fn admin_test_form(State(s): State<Server>, h: HeaderMap, Json(b): Json<SaveSource>) -> Response {
    let (old, me) = match b.id {
        Some(id) => match at_least(&s, &h, id, Level::Manage).await {
            Ok((kept, me, ..)) => (Some(kept), me),
            Err(e) => return e,
        },
        None => match admin_power(&s, &h, "admin.data").await {
            Ok(me) => (None, me),
            Err(e) => return e,
        },
    };
    let mut src = match settle(&b, old.as_ref(), me.admin) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    src.guard = guard(&s, &c).await;
    drop(c);
    if let Some(o) = &old
        && !same_place(o, &src)
        && ((src.read_password.is_empty() && !o.read_password.is_empty())
            || (src.write_password.is_empty() && !o.write_password.is_empty() && !src.write_user.is_empty()))
    {
        return Json(json!({ "ok": false, "message": "Type the password again for the new address." })).into_response();
    }
    if src.time_limit <= 0 {
        src.time_limit = 15;
    }
    src.row_limit = 100_000;
    test_source(&src).await
}

async fn test_source(src: &Source) -> Response {
    let src = src.clone();
    let started = Instant::now();
    let tables = list_tables(&src).await;
    let names: Vec<String> = tables
        .as_ref()
        .map(|t| t.iter().filter_map(|x| x["table"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let write = if src.write_user.is_empty() { None } else { Some(connect_check(&src, true).await) };
    // PostgreSQL can say which tables each user may change: the reading user
    // should have none; the changing user's are the ones worth offering.
    let (mut reader_changes, mut changer_tables) = (Vec::new(), Vec::new());
    if src.kind == "postgres" && tables.is_ok() {
        let sql = "SELECT CASE WHEN n.nspname = 'public' THEN c.relname ELSE n.nspname || '.' || c.relname END
            FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.relkind IN ('r', 'p') AND n.nspname NOT IN ('pg_catalog', 'information_schema')
              AND (has_table_privilege(c.oid, 'INSERT') OR has_table_privilege(c.oid, 'UPDATE') OR has_table_privilege(c.oid, 'DELETE')
                   OR has_table_privilege(c.oid, 'TRUNCATE') OR has_any_column_privilege(c.oid, 'UPDATE'))
            ORDER BY 1";
        let first = |a: Answer| {
            a.rows
                .into_iter()
                .filter_map(|r| r.first().and_then(|v| v.as_str()).map(str::to_string))
                .collect::<Vec<_>>()
        };
        if let Ok(a) = read(&src, sql, &BTreeMap::new()).await {
            reader_changes = first(a);
        }
        if matches!(write, Some(Ok(()))) {
            let mut w = src.clone();
            w.read_user = w.write_user.clone();
            w.read_password = w.write_password.clone();
            if let Ok(a) = read(&w, sql, &BTreeMap::new()).await {
                changer_tables = first(a);
            }
        }
    }
    match tables {
        Ok(t) => Json(json!({
            "ok": true, "tables": t.len(), "names": names, "ms": started.elapsed().as_millis() as u64,
            "changes": write.map(|w| match w { Ok(()) => json!({ "ok": true, "tables": changer_tables }), Err(e) => json!({ "ok": false, "message": e }) }),
            "reader_can_change": reader_changes,
        }))
        .into_response(),
        Err(e) => Json(json!({ "ok": false, "message": e })).into_response(),
    }
}

async fn admin_queries(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let c = match at_least(&s, &h, id, Level::Manage).await {
        Ok((_, _, c, _)) => c,
        Err(e) => return e,
    };
    match c
        .query(
            "SELECT q.id, q.name, q.sql, q.notes, COALESCE(NULLIF(u.display_name, ''), u.username, '')
             FROM data_queries q LEFT JOIN users u ON u.id = q.author WHERE q.source = $1 ORDER BY q.name",
            &[&id],
        )
        .await
    {
        Ok(rows) => Json(rows.iter().map(|r| json!({ "id": r.get::<_, i64>(0), "name": r.get::<_, String>(1), "sql": r.get::<_, String>(2), "notes": r.get::<_, Value>(3), "params": param_names(&r.get::<_, String>(2)), "by": r.get::<_, String>(4) })).collect::<Vec<_>>()).into_response(),
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct SaveQuery {
    #[serde(default)]
    id: Option<i64>,
    name: String,
    sql: String,
    #[serde(default)]
    notes: Value,
}

/// Saves a query by name for whoever writes queries on the source to pick:
/// they add one and change their own; whoever sets up sources changes any.
async fn save_query(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Json(b): Json<SaveQuery>) -> Response {
    let (_, me, c, l) = match at_least(&s, &h, id, Level::Write).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    if b.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name", "give the query a name");
    }
    if let Err(e) = one_statement(&b.sql) {
        return err(StatusCode::BAD_REQUEST, "sql", &e);
    }
    let notes = if b.notes.is_null() { json!({}) } else { b.notes };
    let r = match b.id {
        Some(q) => {
            c.query_opt(
                "UPDATE data_queries SET name = $3, sql = $4, notes = $5
                 WHERE id = $1 AND source = $2 AND ($6 OR author = $7) RETURNING id",
                &[&q, &id, &b.name.trim(), &b.sql, &notes, &(l == Level::Manage), &me.id],
            )
            .await
        }
        None => {
            c.query_opt(
                "INSERT INTO data_queries (source, name, sql, notes, author) VALUES ($1, $2, $3, $4, $5) RETURNING id",
                &[&id, &b.name.trim(), &b.sql, &notes, &me.id],
            )
            .await
        }
    };
    match r {
        Ok(Some(row)) => Json(json!({ "id": row.get::<_, i64>(0) })).into_response(),
        Ok(None) => err(StatusCode::FORBIDDEN, "not_yours", "only its maker, or someone who sets up database connections, can change it"),
        Err(_) => failed(),
    }
}

async fn delete_query(State(s): State<Server>, h: HeaderMap, Path((id, q)): Path<(i64, i64)>) -> Response {
    let (_, me, c, l) = match at_least(&s, &h, id, Level::Write).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    match c
        .execute(
            "DELETE FROM data_queries WHERE id = $1 AND source = $2 AND ($3 OR author = $4)",
            &[&q, &id, &(l == Level::Manage), &me.id],
        )
        .await
    {
        Ok(1) => Json(json!({})).into_response(),
        Ok(_) => err(StatusCode::FORBIDDEN, "not_yours", "only its maker, or someone who sets up database connections, can remove it"),
        Err(_) => failed(),
    }
}

/// Who read and changed what, newest first.
async fn admin_log(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let c = match at_least(&s, &h, id, Level::Manage).await {
        Ok((_, _, c, _)) => c,
        Err(e) => return e,
    };
    match c
        .query(
            "SELECT l.at::TEXT, COALESCE(u.username, ''), l.what, l.detail FROM data_log l LEFT JOIN users u ON u.id = l.user_id
             WHERE l.source = $1 ORDER BY l.at DESC LIMIT 500",
            &[&id],
        )
        .await
    {
        Ok(rows) => Json(rows.iter().map(|r| json!({ "at": r.get::<_, String>(0), "who": r.get::<_, String>(1), "what": r.get::<_, String>(2), "detail": r.get::<_, Value>(3) })).collect::<Vec<_>>()).into_response(),
        Err(_) => failed(),
    }
}

// ----- Apps -----

/// The sources this person may write queries on, with their saved queries
/// and editable tables.
async fn app_list(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool().get().await else { return unavailable() };
    let Ok(all) = sources(&c).await else { return failed() };
    let mut out = Vec::new();
    for src in all {
        let Some(l) = level(&c, src.id, &me).await else { continue };
        let queries = c
            .query("SELECT id, name, sql, notes, author FROM data_queries WHERE source = $1 ORDER BY name", &[&src.id])
            .await
            .unwrap_or_default();
        let queries: Vec<Value> = queries
            .iter()
            .map(|r| {
                let sql: String = r.get(2);
                json!({ "id": r.get::<_, i64>(0), "name": r.get::<_, String>(1), "notes": r.get::<_, Value>(3), "params": param_names(&sql),
                    "sql": sql, "mine": r.get::<_, Option<i64>>(4) == Some(me.id) || l == Level::Manage })
            })
            .collect();
        out.push(json!({
            "id": src.id, "name": src.name, "kind": src.kind, "level": l.name(), "own_queries": true,
            "editable": src.editable, "can_change": !src.write_user.is_empty(), "notes": src.notes, "queries": queries,
            "row_limit": src.row_limit, "time_limit": src.time_limit,
        }));
    }
    Json(out).into_response()
}

async fn app_tables(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (src, ..) = match at_least(&s, &h, id, Level::Write).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    match list_tables(&src).await {
        Ok(t) => Json(t).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "source", &e),
    }
}

#[derive(Deserialize)]
struct RunQuery {
    /// SQL…
    #[serde(default)]
    sql: Option<String>,
    /// …or a saved query.
    #[serde(default)]
    query: Option<i64>,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    /// The ticket the file keeps for it (see the top of this file).
    #[serde(default)]
    ticket: Option<String>,
}

async fn app_query(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Json(b): Json<RunQuery>) -> Response {
    let what = match (b.query, &b.sql) {
        (Some(q), _) => For::Query(q),
        (None, Some(sql)) => For::Sql(sql),
        (None, None) => return err(StatusCode::BAD_REQUEST, "sql", "which query?"),
    };
    let (src, me, c, keep) = match runnable(&s, &h, id, &what, b.ticket.as_deref()).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let (sql, notes, which) = match what {
        For::Query(q) => match c
            .query_opt("SELECT sql, notes, name FROM data_queries WHERE id = $1 AND source = $2", &[&q, &id])
            .await
        {
            Ok(Some(r)) => (r.get::<_, String>(0), r.get::<_, Value>(1), json!({ "query": r.get::<_, String>(2) })),
            _ => return err(StatusCode::NOT_FOUND, "missing", "that saved query isn't there any more"),
        },
        For::Sql(sql) => (sql.to_string(), json!({}), json!({ "sql": sql.chars().take(500).collect::<String>() })),
        For::Table(..) => unreachable!(),
    };
    let started = Instant::now();
    let result = read(&src, &sql, &b.params).await;
    match result {
        Ok(mut r) => {
            log(
                &c,
                src.id,
                me.id,
                "read",
                json!({ "which": which, "rows": r.rows.len(), "ms": started.elapsed().as_millis() as u64 }),
            )
            .await;
            r.notes = notes;
            r.ticket = keep;
            Json(r).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, "source", &e),
    }
}

/// A table for a table editor sheet: some of its rows (by `order`, at most
/// `limit`), with the key columns (so changes can find their row).
#[derive(Deserialize)]
struct ReadTable {
    table: String,
    #[serde(default)]
    columns: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
    /// Only rows where each column holds its value.
    #[serde(default)]
    only: Vec<Only>,
    /// Ordered by this column (else by the key).
    #[serde(default)]
    order: Option<Order>,
    #[serde(default)]
    ticket: Option<String>,
    /// The server file the sheet is in, if it's in one.
    #[serde(default)]
    node: Option<i64>,
}

#[derive(Deserialize)]
struct Only {
    column: String,
    value: Value,
}

#[derive(Deserialize)]
struct Order {
    column: String,
    #[serde(default)]
    desc: bool,
}

async fn app_table(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Json(b): Json<ReadTable>) -> Response {
    let (src, me, c, keep) = match runnable(&s, &h, id, &For::Table(&b.table, b.node), b.ticket.as_deref()).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let Some(ed) = src.editable.iter().find(|e| e.table.eq_ignore_ascii_case(&b.table)).cloned() else {
        return err(StatusCode::FORBIDDEN, "not_editable", "that table can't be edited here: ask whoever manages the source");
    };
    let q = |n: &str| quote_ident(&src.kind, n);
    let cols = if b.columns.is_empty() {
        "*".to_string()
    } else {
        let mut all: Vec<String> = ed.key.clone();
        for col in &b.columns {
            if !all.iter().any(|k| k.eq_ignore_ascii_case(col)) {
                all.push(col.clone());
            }
        }
        all.iter().map(|n| q(n)).collect::<Vec<_>>().join(", ")
    };
    let mut order: Vec<String> = Vec::new();
    if let Some(o) = &b.order {
        order.push(format!("{} {}", q(&o.column), if o.desc { "DESC" } else { "ASC" }));
    }
    order.extend(ed.key.iter().map(|k| q(k)));
    let wh: Vec<String> = b
        .only
        .iter()
        .map(|o| {
            if o.value.is_null() {
                format!("{} IS NULL", q(&o.column))
            } else {
                format!("{} = {}", q(&o.column), literal(&src.kind, &o.value))
            }
        })
        .collect();
    let wh = if wh.is_empty() { String::new() } else { format!(" WHERE {}", wh.join(" AND ")) };
    let table = quote_table(&src.kind, &ed.table);
    let sql = format!("SELECT {cols} FROM {table}{wh} ORDER BY {}", order.join(", "));
    let mut src2 = src.clone();
    if let Some(l) = b.limit {
        src2.row_limit = src2.row_limit.min(l as i32);
    }
    match read(&src2, &sql, &BTreeMap::new()).await {
        Ok(mut r) => {
            let total = if r.truncated {
                read(&src, &format!("SELECT COUNT(*) FROM {table}{wh}"), &BTreeMap::new())
                    .await
                    .ok()
                    .and_then(|a| a.rows.first().and_then(|x| x.first()).and_then(|v| v.as_f64()))
            } else {
                Some(r.rows.len() as f64)
            };
            log(&c, src.id, me.id, "read table", json!({ "table": ed.table, "rows": r.rows.len() })).await;
            r.notes = json!({ "key": ed.key, "locked": ed.locked, "add": ed.add, "delete": ed.delete, "total": total });
            r.ticket = keep;
            Json(r).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, "source", &e),
    }
}

/// One change from a table editor sheet. "update" (the default): a cell, by
/// its row's key and column, with what Calc read there and the new value.
/// "insert": a new row's `values`. "delete": the row with `key`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Change {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    pub table: String,
    pub key: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub column: String,
    pub old: Value,
    pub new: Value,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

/// Changes for one table, with the ticket the file keeps for it (or, from
/// before tickets, the changes alone).
#[derive(Deserialize)]
#[serde(untagged)]
enum Changes {
    List(Vec<Change>),
    With {
        changes: Vec<Change>,
        #[serde(default)]
        ticket: Option<String>,
        /// The server file the sheet is in, if it's in one.
        #[serde(default)]
        node: Option<i64>,
    },
}

/// Applies changes in one transaction as the changing user. Each is refused
/// (and reported) when its cell no longer holds what Calc read, or when the
/// table or column isn't one people may change.
async fn app_changes(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Json(b): Json<Changes>) -> Response {
    let (changes, ticket, node) = match b {
        Changes::List(l) => (l, None, None),
        Changes::With { changes, ticket, node } => (changes, ticket, node),
    };
    // One table at a time for a ticket (it's for one table).
    let Some(first) = changes.first() else { return Json(json!({ "outcomes": [] })).into_response() };
    if changes.iter().any(|c| !c.table.eq_ignore_ascii_case(&first.table)) {
        return err(StatusCode::BAD_REQUEST, "tables", "send one table's changes at a time");
    }
    let (src, me, c, _) = match runnable(&s, &h, id, &For::Table(&first.table, node), ticket.as_deref()).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    // Whoever doesn't write queries here changes rows only in a server file
    // they may edit (its ticket is for that file).
    if level(&c, id, &me).await.is_none() && !may_edit_file(&c, &me, node).await {
        return err(StatusCode::FORBIDDEN, "cant_edit", "only people who can edit this file can change its rows");
    }
    if src.write_user.is_empty() {
        return err(StatusCode::FORBIDDEN, "read_only", "this data source is read only");
    }
    for ch in &changes {
        let Some(ed) = src.editable.iter().find(|e| e.table.eq_ignore_ascii_case(&ch.table)) else {
            return err(StatusCode::FORBIDDEN, "not_editable", &format!("{} can't be edited here", ch.table));
        };
        let locked = |c: &str| ed.locked.iter().any(|l| l.eq_ignore_ascii_case(c));
        match ch.kind.as_str() {
            "" | "update" => {
                if locked(&ch.column) || ed.key.iter().any(|k| k.eq_ignore_ascii_case(&ch.column)) {
                    return err(StatusCode::FORBIDDEN, "locked", &format!("{} can't be changed", ch.column));
                }
            }
            "insert" => {
                if !ed.add {
                    return err(StatusCode::FORBIDDEN, "no_add", &format!("rows can't be added to {}", ch.table));
                }
                if let Some(c) =
                    ch.values.keys().find(|c| locked(c) && !ed.key.iter().any(|k| k.eq_ignore_ascii_case(c)))
                {
                    return err(StatusCode::FORBIDDEN, "locked", &format!("{c} can't be set"));
                }
            }
            "delete" => {
                if !ed.delete {
                    return err(
                        StatusCode::FORBIDDEN,
                        "no_delete",
                        &format!("rows can't be deleted from {}", ch.table),
                    );
                }
            }
            k => return err(StatusCode::BAD_REQUEST, "kind", &format!("unknown kind of change: {k}")),
        }
        if ch.kind != "insert" && ed.key.iter().any(|k| !ch.key.contains_key(k)) {
            return err(StatusCode::BAD_REQUEST, "key", "a change is missing its row's key");
        }
    }
    match apply_changes(&src, &changes).await {
        Ok(outcomes) => {
            for (ch, o) in changes.iter().zip(&outcomes) {
                let what = match ch.kind.as_str() {
                    "insert" => "add row",
                    "delete" => "delete row",
                    _ => "change",
                };
                log(&c, src.id, me.id, what, json!({ "table": ch.table, "key": ch.key, "column": ch.column, "old": ch.old, "new": ch.new, "values": ch.values, "outcome": o })).await;
            }
            Json(json!({ "outcomes": outcomes })).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, "source", &e),
    }
}

// ----- SQL -----

/// The query as one statement (a trailing `;` dropped), or why it isn't one.
pub fn one_statement(sql: &str) -> Result<String, String> {
    let s = sql.trim();
    let mut out_end = s.len();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut semi_at: Option<usize> = None;
    while i < b.len() {
        let c = b[i];
        match c {
            '\'' | '"' | '`' => {
                i += 1;
                while i < b.len() && b[i] != c {
                    i += 1;
                }
            }
            '[' => {
                while i < b.len() && b[i] != ']' {
                    i += 1;
                }
            }
            '-' if b.get(i + 1) == Some(&'-') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                    i += 1;
                }
                i += 1;
            }
            ';' => {
                if semi_at.is_none() {
                    semi_at = Some(i);
                }
            }
            c if semi_at.is_some() && !c.is_whitespace() => return Err("Only one statement at a time".to_string()),
            _ => {}
        }
        i += 1;
    }
    if let Some(k) = semi_at {
        out_end = s.char_indices().nth(k).map(|(p, _)| p).unwrap_or(s.len());
    }
    let body = s[..out_end].trim().to_string();
    if body.is_empty() {
        return Err("The query is empty".to_string());
    }
    Ok(body)
}

/// The `:names` a query uses, once each, in order.
pub fn param_names(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, name) in scan_params(sql) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Where each `:name` is (byte offset of the `:`) and its name, outside
/// strings, quoted names, comments and `::` casts.
fn scan_params(sql: &str) -> Vec<(usize, String)> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b':' if b.get(i + 1) == Some(&b':') => i += 1,
            b':' if i > 0 && b[i - 1] == b':' => {}
            b':' if b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') => {
                let start = i;
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                out.push((start, sql[start + 1..j].to_string()));
                i = j - 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// `sql` with each `:name` replaced by `place(n, name)` (n from 1, in order).
fn replace_params(sql: &str, mut place: impl FnMut(usize, &str) -> String) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (n, (at, name)) in scan_params(sql).into_iter().enumerate() {
        out.push_str(&sql[last..at]);
        out.push_str(&place(n + 1, &name));
        last = at + 1 + name.len();
    }
    out.push_str(&sql[last..]);
    out
}

/// A value written into SQL as a quoted literal (doubled quotes; MySQL also
/// doubles backslashes).
fn literal(kind: &str, v: &Value) -> String {
    let text = match v {
        Value::Null => return "NULL".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => {
            if *b {
                "true".into()
            } else {
                "false".into()
            }
        }
        other => other.to_string(),
    };
    let text: String = text.chars().filter(|c| *c != '\0').collect();
    let mut q = text.replace('\'', "''");
    if kind == "mysql" {
        q = q.replace('\\', "\\\\");
    }
    if kind == "mssql" { format!("N'{q}'") } else { format!("'{q}'") }
}

fn quote_ident(kind: &str, name: &str) -> String {
    match kind {
        "mysql" => format!("`{}`", name.replace('`', "``")),
        "mssql" => format!("[{}]", name.replace(']', "]]")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// A table name, with its schema when it has one (`sales.orders`).
fn quote_table(kind: &str, name: &str) -> String {
    name.split('.').map(|p| quote_ident(kind, p)).collect::<Vec<_>>().join(".")
}

// ----- Running -----

/// A query's answer: its columns (name and kind: number, text, date, bool)
/// and rows, whether more rows were left out, and notes for its columns.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Answer {
    pub columns: Vec<(String, String)>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    #[serde(default)]
    pub notes: Value,
    /// How many decimals each column's numbers have, when the database says
    /// (numeric(10,2): 2, money: 2); none for the rest. Calc shows them so.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decimals: Vec<Option<u32>>,
    /// For whoever may write queries: the ticket for what they ran, to keep
    /// in the file so anyone with it can run it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

/// The decimals in a declared type like numeric(10,2) or DECIMAL(8, 3); money: 2.
fn declared_decimals(t: &str) -> Option<u32> {
    let t = t.trim().to_lowercase();
    if t.contains("money") {
        return Some(2);
    }
    if !(t.starts_with("num") || t.starts_with("dec")) {
        return None;
    }
    let (_, inner) = t.split_once('(')?;
    let (_, scale) = inner.trim_end_matches(')').split_once(',')?;
    scale.trim().parse::<u32>().ok().filter(|s| *s <= 15)
}

fn host_port(address: &str, default: u16) -> (String, u16) {
    match address.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() && !h.is_empty() => (h.to_string(), p.parse().unwrap_or(default)),
        _ => (if address.is_empty() { "localhost".to_string() } else { address.to_string() }, default),
    }
}

/// Runs a read on `src` (as its reading user) with its limits.
pub async fn read(src: &Source, sql: &str, params: &BTreeMap<String, Value>) -> Result<Answer, String> {
    let sql = one_statement(sql)?;
    for name in param_names(&sql) {
        if !params.contains_key(&name) {
            return Err(format!("No value for :{name}"));
        }
    }
    let (time, rows) = src.limits();
    let work = async {
        match src.kind.as_str() {
            "postgres" => read_postgres(src, &sql, params, time, rows).await,
            "mysql" => read_mysql(src, &sql, params, time, rows).await,
            "mssql" => read_mssql(src, &sql, params, rows).await,
            "sqlite" | "csv" => {
                let (src, sql, params) = (src.clone(), sql.clone(), params.clone());
                tokio::task::spawn_blocking(move || read_sqlite(&src, &sql, &params, time, rows))
                    .await
                    .map_err(|e| e.to_string())?
            }
            k => Err(format!("unknown kind of data source: {k}")),
        }
    };
    match tokio::time::timeout(time + Duration::from_secs(2), work).await {
        Ok(r) => r,
        Err(_) => Err(format!("The query took longer than {} seconds and was stopped", time.as_secs())),
    }
}

// ----- Reaching databases -----

/// Why a connection didn't work, in words that don't hand on what the far
/// end said (so the test can't be used to look around a network).
#[derive(Debug, PartialEq)]
pub enum Why {
    NotFound,
    Refused,
    Timeout,
    /// The encrypted connection didn't work: the certificate isn't trusted.
    Certificate,
    /// The database doesn't offer encryption.
    NoTls,
    /// Encryption went wrong some other way.
    Tls,
    Auth,
    /// It wanted the password unencrypted, to another computer.
    Cleartext,
    /// A local address, not allowed for this source.
    Blocked,
    /// The database itself said why (it answered as a database).
    Said(String),
    Other,
}

impl Why {
    fn text(&self, kind: &str) -> String {
        let name = match kind {
            "mysql" => "MySQL",
            "mssql" => "SQL Server",
            _ => "PostgreSQL",
        };
        match self {
            Why::NotFound => "Can’t connect: there’s no computer by that name.".into(),
            Why::Refused => "Can’t connect: the connection was refused. Check the address and port.".into(),
            Why::Timeout => "Can’t connect: no answer in time.".into(),
            Why::Certificate => "Can’t connect: the database’s certificate isn’t trusted. Paste its CA certificate below, or pick Prefer.".into(),
            Why::NoTls => "Can’t connect: the database doesn’t offer encryption. Turn it on there, or pick Off (only on a trusted network).".into(),
            Why::Tls => "Can’t connect: the encrypted connection didn’t work.".into(),
            Why::Auth => "Can’t connect: the database didn’t accept the user name or password.".into(),
            Why::Cleartext => "Can’t connect: the database asks for the password unencrypted. Turn encryption on for this data source.".into(),
            Why::Blocked => "That address is on this computer, its link-local network or this server’s own database. Only an admin can allow local addresses for a data source.".into(),
            Why::Said(m) => format!("Can’t connect: the database said: {m}"),
            Why::Other => format!("Can’t connect: it doesn’t answer like a {name} database."),
        }
    }
}

/// How a source connects.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tls {
    Require,
    Prefer,
    Off,
}

fn tls_of(src: &Source) -> Tls {
    match src.tls.as_str() {
        "prefer" => Tls::Prefer,
        "off" => Tls::Off,
        _ => Tls::Require,
    }
}

/// An address on this computer, or link-local (169.254/16, with cloud
/// servers' metadata address, and fe80::/10).
fn local_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_unspecified() || v.is_link_local() || v.is_broadcast(),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return local_ip(IpAddr::V4(v4));
            }
            v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Where a network source is: (host name, its addresses, whether they're
/// all on this computer). Refuses local addresses and the server's own
/// database unless the source allows local addresses.
async fn reach(src: &Source, default_port: u16) -> Result<(String, Vec<std::net::SocketAddr>, bool), Why> {
    let default_port = match (default_port, src.kind.as_str()) {
        (0, "mysql") => 3306,
        (0, "mssql") => 1433,
        (0, _) => 5432,
        (p, _) => p,
    };
    let (host, port) = host_port(src.address.trim(), default_port);
    let host = host.trim_matches(['[', ']']).to_string();
    let addrs: Vec<std::net::SocketAddr> =
        match tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host.as_str(), port))).await {
            Err(_) => return Err(Why::Timeout),
            Ok(Err(_)) => return Err(Why::NotFound),
            Ok(Ok(a)) => a.collect(),
        };
    if addrs.is_empty() {
        return Err(Why::NotFound);
    }
    if !src.allow_local
        && addrs.iter().any(|a| local_ip(a.ip()) || src.guard.own_db.iter().any(|o| o.ip() == a.ip() && o.port() == a.port()))
    {
        return Err(Why::Blocked);
    }
    let here = addrs.iter().all(|a| a.ip().is_loopback());
    Ok((host, addrs, here))
}

fn io_why(e: &std::io::Error) -> Why {
    match e.kind() {
        std::io::ErrorKind::ConnectionRefused => Why::Refused,
        std::io::ErrorKind::TimedOut => Why::Timeout,
        _ if e.get_ref().is_some_and(|i| i.is::<Cleartext>()) => Why::Cleartext,
        _ if e.get_ref().is_some_and(|i| i.is::<rustls::Error>()) => tls_why(e.get_ref().unwrap().to_string()),
        _ => Why::Other,
    }
}

fn tls_why(text: String) -> Why {
    let t = text.to_lowercase();
    if t.contains("certificate") || t.contains("unknownissuer") || t.contains("notvalidforname") {
        Why::Certificate
    } else {
        Why::Tls
    }
}

/// A TCP connection to the first of `addrs` that answers.
async fn tcp(addrs: &[std::net::SocketAddr]) -> Result<tokio::net::TcpStream, Why> {
    let mut last = Why::Other;
    for a in addrs {
        match tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(a)).await {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => last = io_why(&e),
            Err(_) => last = Why::Timeout,
        }
    }
    Err(last)
}

/// The certificates in pasted PEM text.
fn ca_certs(pem: &str) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, String> {
    use rustls::pki_types::pem::PemObject;
    let certs: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if certs.is_empty() {
        return Err("no certificate".into());
    }
    Ok(certs)
}

/// Accepts any certificate (Prefer: encrypted, not checked), still checking
/// the handshake's signatures.
#[derive(Debug)]
struct AnyCertificate(std::sync::Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The TLS client settings for a source: certificates checked against the
/// public authorities and its own CA certificate, or (`check` off) not.
fn tls_config(src: &Source, check: bool) -> Result<std::sync::Arc<rustls::ClientConfig>, Why> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| Why::Tls)?;
    let config = if check {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        if !src.ca_cert.trim().is_empty() {
            for c in ca_certs(&src.ca_cert).map_err(|_| Why::Certificate)? {
                roots.add(c).map_err(|_| Why::Certificate)?;
            }
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder.dangerous().with_custom_certificate_verifier(std::sync::Arc::new(AnyCertificate(provider))).with_no_client_auth()
    };
    Ok(std::sync::Arc::new(config))
}

/// The marker for a password asked for unencrypted.
#[derive(Debug)]
struct Cleartext;

impl std::fmt::Display for Cleartext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the database asks for the password unencrypted")
    }
}

impl std::error::Error for Cleartext {}

/// PostgreSQL's connection, watched until it's signed in: when it isn't
/// encrypted and the database asks for the password as it is
/// (AuthenticationCleartextPassword), it's stopped before the password goes.
struct PgWatch {
    io: tokio::net::TcpStream,
    state: Watching,
}

#[derive(Clone, Copy)]
enum Watching {
    /// The answer to asking for encryption: 'S' (yes) or 'N'.
    Answer,
    /// A message's type and length.
    Head([u8; 5], usize),
    /// Its body: (type, bytes left, first four, how many of those).
    Body(u8, usize, [u8; 4], usize),
    /// Encrypted, signed in, or not watched.
    Done,
}

impl PgWatch {
    fn new(io: tokio::net::TcpStream, asks_tls: bool, watch: bool) -> PgWatch {
        let state = match (watch, asks_tls) {
            (false, _) => Watching::Done,
            (true, true) => Watching::Answer,
            (true, false) => Watching::Head([0; 5], 0),
        };
        PgWatch { io, state }
    }
}

impl Watching {
    /// Follows what the database sent; Err when it asks for the password unencrypted.
    fn saw(&mut self, mut data: &[u8]) -> std::io::Result<()> {
        while !data.is_empty() {
            match self {
                Watching::Done => return Ok(()),
                Watching::Answer => {
                    if data[0] == b'S' {
                        *self = Watching::Done;
                        return Ok(());
                    }
                    *self = Watching::Head([0; 5], 0);
                    if data[0] == b'N' {
                        data = &data[1..];
                    }
                }
                Watching::Head(h, n) => {
                    h[*n] = data[0];
                    *n += 1;
                    data = &data[1..];
                    if *n == 5 {
                        let (kind, len) = (h[0], u32::from_be_bytes([h[1], h[2], h[3], h[4]]) as usize);
                        *self = Watching::Body(kind, len.saturating_sub(4), [0; 4], 0);
                        if len <= 4 {
                            *self = Watching::Head([0; 5], 0);
                        }
                    }
                }
                Watching::Body(kind, left, code, have) => {
                    if *kind == b'R' && *have < 4 {
                        code[*have] = data[0];
                        *have += 1;
                        *left -= 1;
                        data = &data[1..];
                        if *have == 4 {
                            match u32::from_be_bytes(*code) {
                                3 => return Err(std::io::Error::other(Cleartext)),
                                0 => {
                                    *self = Watching::Done;
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                    } else {
                        let k = (*left).min(data.len());
                        *left -= k;
                        data = &data[k..];
                    }
                    if let Watching::Body(_, 0, ..) = self {
                        *self = Watching::Head([0; 5], 0);
                    }
                }
            }
        }
        Ok(())
    }
}

impl tokio::io::AsyncRead for PgWatch {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = std::pin::Pin::new(&mut self.io).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &r
            && !matches!(self.state, Watching::Done)
        {
            let new = buf.filled()[before..].to_vec();
            if let Err(e) = self.state.saw(&new) {
                return std::task::Poll::Ready(Err(e));
            }
        }
        r
    }
}

impl tokio::io::AsyncWrite for PgWatch {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.io).poll_write(cx, data)
    }

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// TLS for tokio-postgres over rustls, for one server name.
struct PgTls {
    config: std::sync::Arc<rustls::ClientConfig>,
    name: rustls::pki_types::ServerName<'static>,
}

struct PgTlsStream(tokio_rustls::client::TlsStream<PgWatch>);

impl tokio_postgres::tls::TlsConnect<PgWatch> for PgTls {
    type Stream = PgTlsStream;
    type Error = std::io::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<PgTlsStream>> + Send>>;

    fn connect(self, stream: PgWatch) -> Self::Future {
        Box::pin(async move {
            let s = tokio_rustls::TlsConnector::from(self.config).connect(self.name, stream).await?;
            Ok(PgTlsStream(s))
        })
    }
}

impl tokio_postgres::tls::TlsStream for PgTlsStream {
    fn channel_binding(&self) -> tokio_postgres::tls::ChannelBinding {
        tokio_postgres::tls::ChannelBinding::none()
    }
}

impl tokio::io::AsyncRead for PgTlsStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for PgTlsStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, data)
    }

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

fn pg_why(e: &tokio_postgres::Error) -> Why {
    use tokio_postgres::error::SqlState;
    if let Some(db) = e.as_db_error() {
        if [SqlState::INVALID_PASSWORD, SqlState::INVALID_AUTHORIZATION_SPECIFICATION].contains(db.code()) {
            return Why::Auth;
        }
        return Why::Said(db.message().to_string());
    }
    let mut at: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(x) = at {
        if let Some(io) = x.downcast_ref::<std::io::Error>() {
            return io_why(io);
        }
        if x.is::<rustls::Error>() {
            return tls_why(x.to_string());
        }
        if x.to_string().contains("does not support TLS") {
            return Why::NoTls;
        }
        at = x.source();
    }
    if e.to_string().contains("does not support TLS") { Why::NoTls } else { Why::Other }
}

async fn pg_start<T>(cfg: &tokio_postgres::Config, io: PgWatch, tls: T) -> Result<tokio_postgres::Client, Why>
where
    T: tokio_postgres::tls::TlsConnect<PgWatch>,
    T::Stream: Send + 'static,
{
    let (client, conn) = cfg.connect_raw(io, tls).await.map_err(|e| pg_why(&e))?;
    tokio::spawn(conn);
    Ok(client)
}

/// A PostgreSQL connection as the reading user (or the changing one).
async fn pg_client(src: &Source, write: bool) -> Result<tokio_postgres::Client, String> {
    let (host, port) = host_port(src.address.trim(), 5432);
    let mut cfg = tokio_postgres::Config::new();
    cfg.dbname(&src.database).application_name("xenify-calc");
    if write {
        cfg.user(&src.write_user).password(&src.write_password);
    } else {
        cfg.user(&src.read_user).password(&src.read_password);
    }
    let client = if host.starts_with('/') {
        // A socket folder on this computer.
        if !src.allow_local {
            return Err(Why::Blocked.text(&src.kind));
        }
        cfg.host(&host).port(port).connect_timeout(Duration::from_secs(10));
        let (client, conn) = cfg.connect(tokio_postgres::NoTls).await.map_err(|e| pg_why(&e).text(&src.kind))?;
        tokio::spawn(conn);
        client
    } else {
        let mode = tls_of(src);
        let work = async {
            let (name, addrs, here) = reach(src, 5432).await?;
            let io = tcp(&addrs).await?;
            cfg.ssl_mode(match mode {
                Tls::Require => tokio_postgres::config::SslMode::Require,
                Tls::Prefer => tokio_postgres::config::SslMode::Prefer,
                Tls::Off => tokio_postgres::config::SslMode::Disable,
            });
            let watch = PgWatch::new(io, matches!(mode, Tls::Require | Tls::Prefer), !here);
            match mode {
                Tls::Off => pg_start(&cfg, watch, tokio_postgres::NoTls).await,
                Tls::Require | Tls::Prefer => {
                    let name = rustls::pki_types::ServerName::try_from(name).map_err(|_| Why::Tls)?;
                    pg_start(&cfg, watch, PgTls { config: tls_config(src, mode == Tls::Require)?, name }).await
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(15), work).await {
            Ok(r) => r.map_err(|w| w.text(&src.kind))?,
            Err(_) => return Err(Why::Timeout.text(&src.kind)),
        }
    };
    client.batch_execute("SET standard_conforming_strings = on").await.map_err(|e| e.to_string())?;
    if !write {
        client
            .batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(client)
}

fn pg_kind(t: &tokio_postgres::types::Type) -> &'static str {
    use tokio_postgres::types::Type;
    match *t {
        Type::INT2
        | Type::INT4
        | Type::INT8
        | Type::FLOAT4
        | Type::FLOAT8
        | Type::NUMERIC
        | Type::OID
        | Type::MONEY => "number",
        Type::BOOL => "bool",
        Type::DATE | Type::TIMESTAMP | Type::TIMESTAMPTZ | Type::TIME | Type::TIMETZ => "date",
        _ => "text",
    }
}

/// numeric(p,s)'s s (the type modifier is ((p << 16) | s) + 4; -1: none given); money: 2.
fn pg_decimals(c: &tokio_postgres::Column) -> Option<u32> {
    use tokio_postgres::types::Type;
    match *c.type_() {
        Type::MONEY => Some(2),
        Type::NUMERIC if c.type_modifier() >= 4 => Some(((c.type_modifier() - 4) & 0xffff) as u32).filter(|s| *s <= 15),
        _ => None,
    }
}

fn typed(kind: &str, text: Option<&str>) -> Value {
    match (kind, text) {
        (_, None) => Value::Null,
        ("number", Some(t)) => {
            t.trim_start_matches('$').replace(',', "").parse::<f64>().map(|n| json!(n)).unwrap_or_else(|_| json!(t))
        }
        ("bool", Some(t)) => json!(matches!(t, "t" | "true" | "1")),
        (_, Some(t)) => json!(t),
    }
}

async fn read_postgres(
    src: &Source,
    sql: &str,
    params: &BTreeMap<String, Value>,
    time: Duration,
    rows: usize,
) -> Result<Answer, String> {
    let client = pg_client(src, false).await?;
    // Each :name is $n, its value bound (as text, which PostgreSQL reads
    // as the type the query needs, as it would a quoted value).
    let mut names: Vec<String> = Vec::new();
    let sql = replace_params(sql, |_, name| {
        let n = names.iter().position(|x| x == name).unwrap_or_else(|| {
            names.push(name.to_string());
            names.len() - 1
        });
        format!("${}", n + 1)
    });
    let values: Vec<PgText> = names.iter().map(|n| PgText::of(params.get(n))).collect();
    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        values.iter().map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
    let said = |e: tokio_postgres::Error| e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string());
    // Its columns, without running it (this also refuses several statements).
    // A value PostgreSQL can't tell the type of (`:x IS NULL`) is text,
    // as a quoted value would be; the others are as the query needs.
    let unsaid = tokio_postgres::types::Type::new(String::new(), 0, tokio_postgres::types::Kind::Pseudo, String::new());
    let mut types: Vec<tokio_postgres::types::Type> = Vec::new();
    let stmt = loop {
        match client.prepare_typed(&sql, &types).await {
            Ok(s) => break s,
            Err(e) => {
                let m = said(e);
                match m.strip_prefix("could not determine data type of parameter $").and_then(|n| n.parse::<usize>().ok()) {
                    Some(n) if (1..=names.len()).contains(&n) && types.get(n - 1) != Some(&tokio_postgres::types::Type::TEXT) => {
                        if types.len() < n {
                            types.resize(n, unsaid.clone());
                        }
                        types[n - 1] = tokio_postgres::types::Type::TEXT;
                    }
                    _ => return Err(m),
                }
            }
        }
    };
    let columns: Vec<(String, String)> =
        stmt.columns().iter().map(|c| (c.name().to_string(), pg_kind(c.type_()).to_string())).collect();
    let ms = time.as_millis();
    client.batch_execute(&format!("BEGIN READ ONLY; SET LOCAL statement_timeout = {ms}")).await.map_err(|e| e.to_string())?;
    // The query on lines of its own: a `--` note at its end mustn't hide what follows.
    let declare = client
        .prepare_typed(&format!("DECLARE xenify_q NO SCROLL CURSOR FOR\n{sql}\n"), &types)
        .await
        .map_err(said)?;
    client.execute(&declare, &refs).await.map_err(said)?;
    let msgs = client.simple_query(&format!("FETCH {} FROM xenify_q", rows + 1)).await.map_err(|e| e.to_string())?;
    let _ = client.batch_execute("ROLLBACK").await;
    let decimals = stmt.columns().iter().map(pg_decimals).collect();
    let mut out = Answer { columns: columns.clone(), decimals, ..Default::default() };
    for m in msgs {
        if let tokio_postgres::SimpleQueryMessage::Row(r) = m {
            if out.rows.len() == rows {
                out.truncated = true;
                break;
            }
            out.rows.push((0..columns.len()).map(|k| typed(&columns[k].1, r.get(k))).collect());
        }
    }
    Ok(out)
}

/// A value for a PostgreSQL `$n`, sent as text whatever type the query
/// wants there (as a quoted value would be read), or NULL.
#[derive(Debug)]
struct PgText(Option<String>);

impl PgText {
    fn of(v: Option<&Value>) -> PgText {
        PgText(match v {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.chars().filter(|c| *c != '\0').collect()),
            Some(Value::Bool(b)) => Some(b.to_string()),
            Some(other) => Some(other.to_string()),
        })
    }
}

impl tokio_postgres::types::ToSql for PgText {
    fn to_sql(
        &self,
        _: &tokio_postgres::types::Type,
        out: &mut bytes::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            Some(t) => {
                out.extend_from_slice(t.as_bytes());
                Ok(tokio_postgres::types::IsNull::No)
            }
            None => Ok(tokio_postgres::types::IsNull::Yes),
        }
    }

    fn accepts(_: &tokio_postgres::types::Type) -> bool {
        true
    }

    fn encode_format(&self, _: &tokio_postgres::types::Type) -> tokio_postgres::types::Format {
        tokio_postgres::types::Format::Text
    }

    tokio_postgres::types::to_sql_checked!();
}

fn mysql_why(e: &mysql_async::Error) -> Why {
    use mysql_async::{DriverError, Error as E, IoError};
    match e {
        E::Io(IoError::Io(io)) => io_why(io),
        E::Io(IoError::Tls(t)) => tls_why(t.to_string()),
        E::Server(s) if [1044, 1045, 1698].contains(&s.code) => Why::Auth,
        E::Server(s) => Why::Said(s.message.clone()),
        E::Driver(DriverError::NoClientSslFlagFromServer) => Why::NoTls,
        E::Driver(DriverError::CleartextPluginDisabled) => Why::Cleartext,
        _ => Why::Other,
    }
}

/// A MySQL connection, to the address checked (with the certificate
/// checked for the name given).
async fn mysql_conn(src: &Source, write: bool) -> Result<mysql_async::Conn, String> {
    let (user, pass) =
        if write { (&src.write_user, &src.write_password) } else { (&src.read_user, &src.read_password) };
    let (name, addrs, _) = reach(src, 3306).await.map_err(|w| w.text(&src.kind))?;
    let opts = |tls: Option<mysql_async::SslOpts>| {
        mysql_async::OptsBuilder::default()
            .ip_or_hostname(addrs[0].ip().to_string())
            .tcp_port(addrs[0].port())
            .user(Some(user.clone()))
            .pass(Some(pass.clone()))
            .db_name(Some(src.database.clone()))
            .enable_cleartext_plugin(false)
            .ssl_opts(tls)
    };
    let connect = |o: mysql_async::OptsBuilder| async move {
        match tokio::time::timeout(Duration::from_secs(15), mysql_async::Conn::new(o)).await {
            Ok(r) => r.map_err(|e| mysql_why(&e)),
            Err(_) => Err(Why::Timeout),
        }
    };
    // mysql_async makes its TLS settings with the process's default (ring here).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let r = match tls_of(src) {
        Tls::Require => {
            let mut tls = mysql_async::SslOpts::default().with_danger_tls_hostname_override(Some(name));
            if !src.ca_cert.trim().is_empty() {
                tls = tls.with_root_certs(vec![src.ca_cert.clone().into_bytes().into()]);
            }
            connect(opts(Some(tls))).await
        }
        Tls::Prefer => {
            let tls = mysql_async::SslOpts::default().with_danger_accept_invalid_certs(true);
            match connect(opts(Some(tls))).await {
                Err(Why::NoTls) => connect(opts(None)).await,
                r => r,
            }
        }
        Tls::Off => connect(opts(None)).await,
    };
    r.map_err(|w| w.text(&src.kind))
}

fn mysql_value(v: &mysql_async::Value) -> Value {
    use mysql_async::Value as V;
    match v {
        V::NULL => Value::Null,
        V::Int(n) => json!(n),
        V::UInt(n) => json!(n),
        V::Float(f) => json!(f),
        V::Double(f) => json!(f),
        V::Bytes(b) => {
            let s = String::from_utf8_lossy(b).into_owned();
            s.parse::<f64>()
                .ok()
                .filter(|_| !s.starts_with('0') || s == "0" || s.starts_with("0."))
                .map(|n| json!(n))
                .unwrap_or(json!(s))
        }
        V::Date(y, mo, d, h, mi, s, _) => {
            if *h == 0 && *mi == 0 && *s == 0 {
                json!(format!("{y:04}-{mo:02}-{d:02}"))
            } else {
                json!(format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
            }
        }
        V::Time(neg, d, h, m, s, _) => {
            json!(format!("{}{:02}:{m:02}:{s:02}", if *neg { "-" } else { "" }, *d * 24 + *h as u32))
        }
    }
}

fn mysql_kind(t: mysql_async::consts::ColumnType) -> &'static str {
    use mysql_async::consts::ColumnType as C;
    match t {
        C::MYSQL_TYPE_TINY
        | C::MYSQL_TYPE_SHORT
        | C::MYSQL_TYPE_LONG
        | C::MYSQL_TYPE_LONGLONG
        | C::MYSQL_TYPE_INT24
        | C::MYSQL_TYPE_FLOAT
        | C::MYSQL_TYPE_DOUBLE
        | C::MYSQL_TYPE_DECIMAL
        | C::MYSQL_TYPE_NEWDECIMAL => "number",
        C::MYSQL_TYPE_DATE
        | C::MYSQL_TYPE_DATETIME
        | C::MYSQL_TYPE_TIMESTAMP
        | C::MYSQL_TYPE_TIME
        | C::MYSQL_TYPE_NEWDATE => "date",
        _ => "text",
    }
}

async fn read_mysql(
    src: &Source,
    sql: &str,
    params: &BTreeMap<String, Value>,
    time: Duration,
    rows: usize,
) -> Result<Answer, String> {
    use mysql_async::prelude::Queryable;
    let mut conn = mysql_conn(src, false).await?;
    conn.query_drop(format!("SET SESSION TRANSACTION READ ONLY, SESSION max_execution_time = {}", time.as_millis()))
        .await
        .map_err(|e| e.to_string())?;
    let names = scan_params(sql).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    let sql_q = replace_params(sql, |_, _| "?".to_string());
    let values: Vec<mysql_async::Value> = names
        .iter()
        .map(|n| match params.get(n) {
            Some(Value::Number(x)) => x.as_f64().map(mysql_async::Value::Double).unwrap_or(mysql_async::Value::NULL),
            Some(Value::String(s)) => mysql_async::Value::Bytes(s.clone().into_bytes()),
            Some(Value::Bool(b)) => mysql_async::Value::Int(*b as i64),
            _ => mysql_async::Value::NULL,
        })
        .collect();
    let mut result = conn.exec_iter(sql_q, mysql_async::Params::Positional(values)).await.map_err(|e| e.to_string())?;
    let columns: Vec<(String, String)> = result
        .columns_ref()
        .iter()
        .map(|c| (c.name_str().to_string(), mysql_kind(c.column_type()).to_string()))
        .collect();
    let decimals = result
        .columns_ref()
        .iter()
        .map(|c| {
            use mysql_async::consts::ColumnType as C;
            matches!(c.column_type(), C::MYSQL_TYPE_DECIMAL | C::MYSQL_TYPE_NEWDECIMAL)
                .then(|| c.decimals() as u32)
                .filter(|s| *s <= 15)
        })
        .collect();
    let mut out = Answer { columns, decimals, ..Default::default() };
    while let Some(row) = result.next().await.map_err(|e| e.to_string())? {
        if out.rows.len() == rows {
            out.truncated = true;
            break;
        }
        out.rows.push((0..row.len()).map(|k| row.as_ref(k).map(mysql_value).unwrap_or(Value::Null)).collect());
    }
    drop(result);
    let _ = conn.disconnect().await;
    Ok(out)
}

async fn mssql_client(
    src: &Source,
    write: bool,
) -> Result<tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>, String> {
    use tokio_util::compat::TokioAsyncWriteCompatExt;
    let (user, pass) =
        if write { (&src.write_user, &src.write_password) } else { (&src.read_user, &src.read_password) };
    let work = async {
        let (name, addrs, _) = reach(src, 1433).await?;
        let mut config = tiberius::Config::new();
        // The name the certificate is checked for; the connection goes to the address checked.
        config.host(&name);
        config.port(addrs[0].port());
        config.database(&src.database);
        config.authentication(tiberius::AuthMethod::sql_server(user, pass));
        config.application_name("xenify-calc");
        match tls_of(src) {
            Tls::Require => {
                config.encryption(tiberius::EncryptionLevel::Required);
                if !src.ca_cert.trim().is_empty() {
                    config.trust_cert_ca_bundle(src.ca_cert.clone().into_bytes());
                }
            }
            Tls::Prefer => {
                config.encryption(tiberius::EncryptionLevel::On);
                config.trust_cert();
            }
            // The sign-in is still encrypted when the server can.
            Tls::Off => {
                config.encryption(tiberius::EncryptionLevel::Off);
                config.trust_cert();
            }
        }
        let io = tcp(&addrs).await?;
        tiberius::Client::connect(config, io.compat_write()).await.map_err(|e| match e {
            tiberius::error::Error::Io { kind: std::io::ErrorKind::ConnectionRefused, .. } => Why::Refused,
            tiberius::error::Error::Io { kind: std::io::ErrorKind::TimedOut, .. } => Why::Timeout,
            tiberius::error::Error::Tls(t) => tls_why(t),
            tiberius::error::Error::Server(t) if t.code() == 18456 => Why::Auth,
            tiberius::error::Error::Server(t) => Why::Said(t.message().to_string()),
            _ => Why::Other,
        })
    };
    match tokio::time::timeout(Duration::from_secs(15), work).await {
        Ok(r) => r.map_err(|w| w.text(&src.kind)),
        Err(_) => Err(Why::Timeout.text(&src.kind)),
    }
}

fn mssql_value(d: tiberius::ColumnData<'static>) -> Value {
    use tiberius::ColumnData as D;
    match d {
        D::U8(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::I16(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::I32(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::I64(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::F32(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::F64(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::Bit(v) => v.map(|x| json!(x)).unwrap_or(Value::Null),
        D::String(v) => v.map(|x| json!(x.to_string())).unwrap_or(Value::Null),
        D::Guid(v) => v.map(|x| json!(x.to_string())).unwrap_or(Value::Null),
        D::Numeric(v) => v.map(|x| json!(f64::from(x))).unwrap_or(Value::Null),
        D::Binary(_) | D::Xml(_) => Value::Null,
        other => {
            // Dates and times, as text the sheet reads as dates.
            let n: Option<chrono::NaiveDateTime> = tiberius::FromSql::from_sql(&other).ok().flatten();
            match n {
                Some(dt) if dt.time() == chrono::NaiveTime::MIN => json!(dt.format("%Y-%m-%d").to_string()),
                Some(dt) => json!(dt.format("%Y-%m-%d %H:%M:%S").to_string()),
                None => {
                    let d: Option<chrono::NaiveDate> = tiberius::FromSql::from_sql(&other).ok().flatten();
                    d.map(|d| json!(d.format("%Y-%m-%d").to_string())).unwrap_or(Value::Null)
                }
            }
        }
    }
}

fn mssql_kind(t: tiberius::ColumnType) -> &'static str {
    use tiberius::ColumnType as C;
    match t {
        C::Int1
        | C::Int2
        | C::Int4
        | C::Int8
        | C::Intn
        | C::Float4
        | C::Float8
        | C::Floatn
        | C::Decimaln
        | C::Numericn
        | C::Money
        | C::Money4 => "number",
        C::Bit | C::Bitn => "bool",
        C::Datetime | C::Datetime4 | C::Datetimen | C::Daten | C::Timen | C::Datetime2 | C::DatetimeOffsetn => "date",
        _ => "text",
    }
}

/// Words a SQL Server query mustn't use outside strings, names and notes:
/// T-SQL needs no `;` between statements, so these could end the read-only
/// transaction or change something. (The reading user's own rights are the
/// real protection; Admin › Data sources says so.)
const MSSQL_REFUSED: [&str; 31] = [
    "COMMIT", "ROLLBACK", "SAVE", "BEGIN", "SET", "EXEC", "EXECUTE", "SP_EXECUTESQL", "INSERT", "UPDATE", "DELETE",
    "MERGE", "INTO", "DROP", "ALTER", "CREATE", "TRUNCATE", "GRANT", "REVOKE", "DENY", "USE", "SHUTDOWN", "KILL",
    "RECONFIGURE", "BACKUP", "RESTORE", "DBCC", "BULK", "OPENROWSET", "OPENDATASOURCE", "OPENQUERY",
];

/// The first word of `MSSQL_REFUSED` in `sql` outside strings ('…', N'…'),
/// quoted names ("…", […]) and notes (-- and /* */), if any.
pub fn mssql_refused(sql: &str) -> Option<String> {
    let b: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ ('\'' | '"') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
            }
            '[' => {
                while i < b.len() && b[i] != ']' {
                    i += 1;
                }
            }
            '-' if b.get(i + 1) == Some(&'-') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                    i += 1;
                }
                i += 1;
            }
            c if c.is_alphabetic() || c == '_' || c == '@' || c == '#' => {
                let start = i;
                while i < b.len() && (b[i].is_alphanumeric() || matches!(b[i], '_' | '@' | '#' | '$')) {
                    i += 1;
                }
                let word: String = b[start..i].iter().collect::<String>().to_uppercase();
                if MSSQL_REFUSED.contains(&word.as_str()) {
                    return Some(word);
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

async fn read_mssql(src: &Source, sql: &str, params: &BTreeMap<String, Value>, rows: usize) -> Result<Answer, String> {
    use futures_util::TryStreamExt;
    if let Some(word) = mssql_refused(sql) {
        return Err(format!("A SQL Server query here can only read: {word} isn’t allowed"));
    }
    let mut client = mssql_client(src, false).await?;
    let names = scan_params(sql).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    let body = replace_params(sql, |n, _| format!("@P{n}"));
    // At most the row limit (and one more, to know there were more); undone whatever it does.
    let script =
        format!("SET ROWCOUNT {}; BEGIN TRANSACTION;\n{body}\n; ROLLBACK TRANSACTION; SET ROWCOUNT 0", rows + 1);
    let mut q = tiberius::Query::new(script);
    for n in &names {
        match params.get(n) {
            Some(Value::Number(x)) => q.bind(x.as_f64()),
            Some(Value::String(s)) => q.bind(s.clone()),
            Some(Value::Bool(b)) => q.bind(*b),
            _ => q.bind(Option::<String>::None),
        }
    }
    let mut stream = q.query(&mut client).await.map_err(|e| e.to_string())?;
    let cols: Vec<tiberius::Column> = stream.columns().await.map_err(|e| e.to_string())?.map(|c| c.to_vec()).unwrap_or_default();
    let columns: Vec<(String, String)> =
        cols.iter().map(|c| (c.name().to_string(), mssql_kind(c.column_type()).to_string())).collect();
    // Money: 2 decimals; decimal(p,s): s, as the first value gives it.
    let mut decimals: Vec<Option<u32>> = cols
        .iter()
        .map(|c| matches!(c.column_type(), tiberius::ColumnType::Money | tiberius::ColumnType::Money4).then_some(2))
        .collect();
    let mut out = Answer { columns, ..Default::default() };
    // Read one row at a time, the first result only, never more than the
    // limit (whatever the query does to SET ROWCOUNT).
    let mut found = stream.into_row_stream();
    while let Some(r) = found.try_next().await.map_err(|e| e.to_string())? {
        if r.result_index() > 0 {
            break;
        }
        if out.rows.len() == rows {
            out.truncated = true;
            break;
        }
        let cells: Vec<tiberius::ColumnData<'static>> = r.into_iter().collect();
        for (k, d) in cells.iter().enumerate() {
            if let (Some(None), tiberius::ColumnData::Numeric(Some(n))) = (decimals.get(k), d) {
                decimals[k] = Some(n.scale() as u32).filter(|s| *s > 0 && *s <= 15);
            }
        }
        out.rows.push(cells.into_iter().map(mssql_value).collect());
    }
    out.decimals = decimals;
    Ok(out)
}

/// A SQLite connection for reading: the file read-only, or a folder of CSV
/// files loaded into memory (one table each, named after the file).
/// `path` as given, when it's a full path inside one of `folders` (by name:
/// the file needn't be there yet). Else why not.
fn in_folders_by_name(path: &str, folders: &[std::path::PathBuf]) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path.trim());
    if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!("{}: give the full path, like /srv/data/stock.sqlite", path.trim()));
    }
    let inside = |f: &std::path::PathBuf| p.starts_with(f) || f.canonicalize().is_ok_and(|f| p.starts_with(f));
    if folders.iter().any(inside) {
        Ok(p.to_path_buf())
    } else {
        Err(format!(
            "{} isn’t in a folder data sources may use. An admin can add its folder under Folders for files.",
            path.trim()
        ))
    }
}

/// The file or folder at `path`, links followed, when it's really inside
/// one of `folders`.
fn in_folders(path: &str, folders: &[std::path::PathBuf]) -> Result<std::path::PathBuf, String> {
    let p = in_folders_by_name(path, folders)?;
    let real = p.canonicalize().map_err(|_| format!("Couldn’t open {}", path.trim()))?;
    if folders.iter().filter_map(|f| f.canonicalize().ok()).any(|f| real.starts_with(f)) {
        Ok(real)
    } else {
        Err(format!(
            "{} isn’t in a folder data sources may use. An admin can add its folder under Folders for files.",
            path.trim()
        ))
    }
}

/// Queries can't open other database files (ATTACH), which could be
/// anywhere on this computer.
fn no_attach(conn: &rusqlite::Connection) {
    conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| match ctx.action {
        rusqlite::hooks::AuthAction::Attach { .. } => rusqlite::hooks::Authorization::Deny,
        _ => rusqlite::hooks::Authorization::Allow,
    }));
}

fn sqlite_conn(src: &Source) -> Result<rusqlite::Connection, String> {
    use rusqlite::OpenFlags;
    let at = in_folders(&src.address, &src.guard.folders)?;
    if src.kind == "csv" {
        let conn = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
        no_attach(&conn);
        let files: Vec<std::path::PathBuf> = if at.is_file() {
            vec![at.to_path_buf()]
        } else {
            std::fs::read_dir(&at)
                .map_err(|_| format!("Couldn’t open {}", src.address))?
                .flatten()
                .map(|e| e.path())
                .collect()
        };
        for p in files {
            if p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("csv")) {
                continue;
            }
            // A link out of the folder isn't followed.
            if in_folders(&p.to_string_lossy(), &src.guard.folders).is_err() {
                continue;
            }
            let name: String = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let rows = csv_rows(&text);
            let Some(header) = rows.first() else { continue };
            let cols: Vec<String> = header.iter().map(|h| format!("\"{}\"", h.replace('"', "\"\""))).collect();
            let table = format!("\"{}\"", name.replace('"', "\"\""));
            conn.execute(&format!("CREATE TABLE {table} ({})", cols.join(", ")), []).map_err(|e| e.to_string())?;
            let marks = vec!["?"; cols.len()].join(", ");
            let mut ins = conn.prepare(&format!("INSERT INTO {table} VALUES ({marks})")).map_err(|e| e.to_string())?;
            for r in rows.iter().skip(1) {
                let vals: Vec<rusqlite::types::Value> = (0..cols.len())
                    .map(|k| {
                        let v = r.get(k).cloned().unwrap_or_default();
                        // Codes with leading zeros (007) stay words.
                        let digits = v.trim_start_matches('-');
                        let code = digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.");
                        if v.is_empty() {
                            rusqlite::types::Value::Null
                        } else if code {
                            rusqlite::types::Value::Text(v)
                        } else if let Ok(n) = v.parse::<i64>() {
                            rusqlite::types::Value::Integer(n)
                        } else if let Ok(f) =
                            v.parse::<f64>().map_err(|_| ()).and_then(|f| if f.is_finite() { Ok(f) } else { Err(()) })
                        {
                            rusqlite::types::Value::Real(f)
                        } else {
                            rusqlite::types::Value::Text(v)
                        }
                    })
                    .collect();
                ins.execute(rusqlite::params_from_iter(vals)).map_err(|e| e.to_string())?;
            }
        }
        conn.pragma_update(None, "query_only", true).map_err(|e| e.to_string())?;
        return Ok(conn);
    }
    let conn = rusqlite::Connection::open_with_flags(
        &at,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Couldn’t open {}: {e}", src.address))?;
    no_attach(&conn);
    conn.pragma_update(None, "query_only", true).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sep =
        if text.lines().next().is_some_and(|l| l.matches(';').count() > l.matches(',').count()) { ';' } else { ',' };
    let mut rows = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn read_sqlite(
    src: &Source,
    sql: &str,
    params: &BTreeMap<String, Value>,
    time: Duration,
    rows: usize,
) -> Result<Answer, String> {
    let conn = sqlite_conn(src)?;
    let deadline = Instant::now() + time;
    conn.progress_handler(10_000, Some(move || Instant::now() > deadline));
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let columns: Vec<(String, String)> = stmt
        .columns()
        .iter()
        .map(|c| {
            let t = c.decl_type().unwrap_or("").to_lowercase();
            let kind = if t.contains("int")
                || t.contains("real")
                || t.contains("floa")
                || t.contains("doub")
                || t.contains("num")
                || t.contains("dec")
            {
                "number"
            } else if t.contains("date") || t.contains("time") {
                "date"
            } else if t.contains("bool") {
                "bool"
            } else {
                "text"
            };
            (c.name().to_string(), kind.to_string())
        })
        .collect();
    let decimals: Vec<Option<u32>> = stmt.columns().iter().map(|c| c.decl_type().and_then(declared_decimals)).collect();
    let named: Vec<(String, rusqlite::types::Value)> = param_names(sql)
        .into_iter()
        .map(|n| {
            let v = match params.get(&n) {
                Some(Value::Number(x)) => x
                    .as_i64()
                    .map(rusqlite::types::Value::Integer)
                    .or_else(|| x.as_f64().map(rusqlite::types::Value::Real))
                    .unwrap_or(rusqlite::types::Value::Null),
                Some(Value::String(s)) => rusqlite::types::Value::Text(s.clone()),
                Some(Value::Bool(b)) => rusqlite::types::Value::Integer(*b as i64),
                _ => rusqlite::types::Value::Null,
            };
            (format!(":{n}"), v)
        })
        .collect();
    let refs: Vec<(&str, &dyn rusqlite::ToSql)> =
        named.iter().map(|(n, v)| (n.as_str(), v as &dyn rusqlite::ToSql)).collect();
    let mut found = stmt.query(refs.as_slice()).map_err(|e| e.to_string())?;
    let mut out = Answer { columns: columns.clone(), decimals, ..Default::default() };
    while let Some(r) = found.next().map_err(|e| {
        if e.to_string().contains("interrupt") {
            format!("The query took longer than {} seconds and was stopped", time.as_secs())
        } else {
            e.to_string()
        }
    })? {
        if out.rows.len() == rows {
            out.truncated = true;
            break;
        }
        let mut line = Vec::with_capacity(columns.len());
        for k in 0..columns.len() {
            use rusqlite::types::ValueRef as V;
            line.push(match r.get_ref(k).map_err(|e| e.to_string())? {
                V::Null => Value::Null,
                V::Integer(n) => json!(n),
                V::Real(f) => json!(f),
                V::Text(t) => json!(String::from_utf8_lossy(t)),
                V::Blob(_) => Value::Null,
            });
        }
        out.rows.push(line);
    }
    // Columns without a declared type (expressions, CSV files): numbers when every value is one.
    for (k, col) in out.columns.iter_mut().enumerate() {
        if col.1 == "text"
            && !out.rows.is_empty()
            && out.rows.iter().all(|r| r[k].is_number() || r[k].is_null())
            && out.rows.iter().any(|r| r[k].is_number())
        {
            col.1 = "number".to_string();
        }
    }
    Ok(out)
}

async fn connect_check(src: &Source, write: bool) -> Result<(), String> {
    match src.kind.as_str() {
        "postgres" => pg_client(src, write).await.map(|_| ()),
        "mysql" => mysql_conn(src, write).await.map(|_| ()),
        "mssql" => mssql_client(src, write).await.map(|_| ()),
        _ => Ok(()),
    }
}

/// The tables (and views) a source has: their columns (name, kind) and any
/// descriptions kept in the database (PostgreSQL COMMENT, MySQL COMMENT,
/// SQL Server MS_Description).
pub async fn list_tables(src: &Source) -> Result<Vec<Value>, String> {
    let sql = match src.kind.as_str() {
        "postgres" => "SELECT CASE WHEN c.table_schema = 'public' THEN c.table_name ELSE c.table_schema || '.' || c.table_name END AS t,
              c.column_name, c.data_type,
              col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position::int) AS note,
              obj_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, 'pg_class') AS table_note
            FROM information_schema.columns c
            WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema')
            ORDER BY 1, c.ordinal_position"
            .to_string(),
        "mysql" => "SELECT c.TABLE_NAME, c.COLUMN_NAME, c.DATA_TYPE, c.COLUMN_COMMENT, t.TABLE_COMMENT
            FROM information_schema.COLUMNS c JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME
            WHERE c.TABLE_SCHEMA = DATABASE() ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION"
            .to_string(),
        "mssql" => "SELECT CASE WHEN c.TABLE_SCHEMA = 'dbo' THEN c.TABLE_NAME ELSE c.TABLE_SCHEMA + '.' + c.TABLE_NAME END, c.COLUMN_NAME, c.DATA_TYPE,
              CAST(ep.value AS NVARCHAR(4000)), CAST(tp.value AS NVARCHAR(4000))
            FROM INFORMATION_SCHEMA.COLUMNS c
            LEFT JOIN sys.extended_properties ep ON ep.major_id = OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME)
              AND ep.minor_id = COLUMNPROPERTY(OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME), c.COLUMN_NAME, 'ColumnId') AND ep.name = 'MS_Description'
            LEFT JOIN sys.extended_properties tp ON tp.major_id = OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME) AND tp.minor_id = 0 AND tp.name = 'MS_Description'
            ORDER BY 1, c.ORDINAL_POSITION"
            .to_string(),
        "sqlite" | "csv" => {
            let src = src.clone();
            return tokio::task::spawn_blocking(move || -> Result<Vec<Value>, String> {
                let conn = sqlite_conn(&src)?;
                let mut names = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name")
                    .map_err(|e| e.to_string())?;
                let tables: Vec<String> = names.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.flatten().collect();
                let mut out = Vec::new();
                for t in tables {
                    let mut info = conn.prepare(&format!("PRAGMA table_info(\"{}\")", t.replace('"', "\"\""))).map_err(|e| e.to_string())?;
                    let cols: Vec<Value> = info
                        .query_map([], |r| Ok(json!({ "name": r.get::<_, String>(1)?, "type": r.get::<_, String>(2)?, "note": "" })))
                        .map_err(|e| e.to_string())?
                        .flatten()
                        .collect();
                    out.push(json!({ "table": t, "note": "", "columns": cols }));
                }
                Ok(out)
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        k => return Err(format!("unknown kind of data source: {k}")),
    };
    let mut src2 = src.clone();
    src2.row_limit = 100_000;
    let a = read(&src2, &sql, &BTreeMap::new()).await?;
    let mut out: Vec<Value> = Vec::new();
    for r in a.rows {
        let s = |k: usize| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (t, col, ty, note, tnote) = (s(0), s(1), s(2), s(3), s(4));
        if out.last().is_none_or(|x| x["table"] != json!(t)) {
            out.push(json!({ "table": t, "note": tnote, "columns": [] }));
        }
        if let Some(last) = out.last_mut()
            && let Some(cols) = last["columns"].as_array_mut()
        {
            cols.push(json!({ "name": col, "type": ty, "note": note }));
        }
    }
    Ok(out)
}

/// A connection for changing, in a transaction (PostgreSQL, MySQL or SQL Server).
enum Changer<'a> {
    Pg(tokio_postgres::Transaction<'a>),
    My(mysql_async::Transaction<'a>),
    Ms(&'a mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>),
}

impl Changer<'_> {
    /// The first row `sql` gives, its values as text.
    async fn first(&mut self, sql: &str) -> Result<Option<Vec<Option<String>>>, String> {
        match self {
            Changer::Pg(tx) => {
                let msgs = tx.simple_query(sql).await.map_err(|e| e.to_string())?;
                Ok(msgs.iter().find_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(r) => {
                        Some((0..r.len()).map(|k| r.get(k).map(str::to_string)).collect())
                    }
                    _ => None,
                }))
            }
            Changer::My(tx) => {
                use mysql_async::prelude::Queryable;
                let row: Option<mysql_async::Row> = tx.query_first(sql).await.map_err(|e| e.to_string())?;
                Ok(row.map(|r| {
                    (0..r.len())
                        .map(|k| match r.as_ref(k) {
                            None | Some(mysql_async::Value::NULL) => None,
                            Some(v) => Some(match mysql_value(v) {
                                Value::String(s) => s,
                                other => other.to_string(),
                            }),
                        })
                        .collect()
                }))
            }
            Changer::Ms(client) => {
                let rows = client
                    .simple_query(sql)
                    .await
                    .map_err(|e| e.to_string())?
                    .into_first_result()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| (0..r.len()).map(|k| r.get::<&str, _>(k).map(str::to_string)).collect()))
            }
        }
    }

    async fn run(&mut self, sql: &str) -> Result<(), String> {
        match self {
            Changer::Pg(tx) => tx.simple_query(sql).await.map(|_| ()).map_err(|e| e.to_string()),
            Changer::My(tx) => {
                use mysql_async::prelude::Queryable;
                tx.query_drop(sql).await.map_err(|e| e.to_string())
            }
            Changer::Ms(client) => client
                .simple_query(sql)
                .await
                .map_err(|e| e.to_string())?
                .into_results()
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }

    async fn commit(self) -> Result<(), String> {
        match self {
            Changer::Pg(tx) => tx.commit().await.map_err(|e| e.to_string()),
            Changer::My(tx) => tx.commit().await.map_err(|e| e.to_string()),
            Changer::Ms(client) => client
                .simple_query("COMMIT TRANSACTION")
                .await
                .map_err(|e| e.to_string())?
                .into_results()
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }
}

/// A column read as text, for comparing with what Calc read.
fn as_text(kind: &str, col: &str) -> String {
    match kind {
        "postgres" => format!("{col}::TEXT"),
        "mysql" => format!("CAST({col} AS CHAR)"),
        _ => format!("CAST({col} AS NVARCHAR(4000))"),
    }
}

/// Applies `changes` as the changing user, in one transaction. A cell only
/// changes when it still holds `old` ("changed"; else "conflict" with what's
/// there now, or "missing" when the row's gone); a new row says its key
/// ("added"); a deleted one "deleted" (or "missing").
async fn apply_changes(src: &Source, changes: &[Change]) -> Result<Vec<Value>, String> {
    let kind = src.kind.as_str();
    let (mut pg, mut my, mut ms) = (None, None, None);
    let mut db = match kind {
        "postgres" => {
            pg = Some(pg_client(src, true).await?);
            Changer::Pg(pg.as_mut().unwrap().transaction().await.map_err(|e| e.to_string())?)
        }
        "mysql" => {
            my = Some(mysql_conn(src, true).await?);
            Changer::My(
                my.as_mut()
                    .unwrap()
                    .start_transaction(mysql_async::TxOpts::default())
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
        "mssql" => {
            ms = Some(mssql_client(src, true).await?);
            let client = ms.as_mut().unwrap();
            client
                .simple_query("BEGIN TRANSACTION")
                .await
                .map_err(|e| e.to_string())?
                .into_results()
                .await
                .map_err(|e| e.to_string())?;
            Changer::Ms(client)
        }
        _ => return Err("Only PostgreSQL, MySQL and SQL Server tables can be edited".to_string()),
    };
    let q = |n: &str| quote_ident(kind, n);
    let mut out = Vec::new();
    for ch in changes {
        let table = quote_table(kind, &ch.table);
        let wh: Vec<String> = ch
            .key
            .iter()
            .map(
                |(k, v)| {
                    if v.is_null() { format!("{} IS NULL", q(k)) } else { format!("{} = {}", q(k), literal(kind, v)) }
                },
            )
            .collect();
        let wh = wh.join(" AND ");
        let (lock_table, lock_end) = if kind == "mssql" { (" WITH (UPDLOCK)", "") } else { ("", " FOR UPDATE") };
        match ch.kind.as_str() {
            "insert" => {
                let ed = src.editable.iter().find(|e| e.table.eq_ignore_ascii_case(&ch.table));
                let key: Vec<String> = ed.map(|e| e.key.clone()).unwrap_or_default();
                let cols: Vec<String> = ch.values.keys().map(|c| q(c)).collect();
                let vals: Vec<String> = ch.values.values().map(|v| literal(kind, v)).collect();
                let keys_text: Vec<String> = key.iter().map(|k| as_text(kind, &q(k))).collect();
                let sql = match kind {
                    "postgres" if cols.is_empty() => {
                        format!("INSERT INTO {table} DEFAULT VALUES RETURNING {}", keys_text.join(", "))
                    }
                    "postgres" => format!(
                        "INSERT INTO {table} ({}) VALUES ({}) RETURNING {}",
                        cols.join(", "),
                        vals.join(", "),
                        keys_text.join(", ")
                    ),
                    "mysql" if cols.is_empty() => format!("INSERT INTO {table} () VALUES ()"),
                    "mysql" => format!("INSERT INTO {table} ({}) VALUES ({})", cols.join(", "), vals.join(", ")),
                    _ => {
                        let outs: Vec<String> =
                            key.iter().map(|k| format!("CAST(INSERTED.{} AS NVARCHAR(4000))", q(k))).collect();
                        if cols.is_empty() {
                            format!("INSERT INTO {table} OUTPUT {} DEFAULT VALUES", outs.join(", "))
                        } else {
                            format!(
                                "INSERT INTO {table} ({}) OUTPUT {} VALUES ({})",
                                cols.join(", "),
                                outs.join(", "),
                                vals.join(", ")
                            )
                        }
                    }
                };
                let got = if kind == "mysql" {
                    db.run(&sql).await?;
                    let missing: Vec<&String> =
                        key.iter().filter(|k| !ch.values.keys().any(|c| c.eq_ignore_ascii_case(k))).collect();
                    if missing.len() == 1 {
                        db.first("SELECT CAST(LAST_INSERT_ID() AS CHAR)").await?.map(|r| {
                            key.iter()
                                .map(|k| {
                                    if k == missing[0] {
                                        r.first().cloned().flatten()
                                    } else {
                                        ch.values
                                            .get(k)
                                            .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                                    }
                                })
                                .collect()
                        })
                    } else {
                        Some(
                            key.iter()
                                .map(|k| {
                                    ch.values
                                        .get(k)
                                        .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                                })
                                .collect(),
                        )
                    }
                } else {
                    db.first(&sql).await?
                };
                let row = got.unwrap_or_default();
                let made: serde_json::Map<String, Value> = key
                    .iter()
                    .enumerate()
                    .map(|(k, name)| {
                        (name.clone(), row.get(k).cloned().flatten().map(Value::String).unwrap_or(Value::Null))
                    })
                    .collect();
                out.push(json!({ "outcome": "added", "key": made }));
            }
            "delete" => {
                if db.first(&format!("SELECT 1 FROM {table}{lock_table} WHERE {wh}{lock_end}")).await?.is_none() {
                    out.push(json!({ "outcome": "missing" }));
                    continue;
                }
                db.run(&format!("DELETE FROM {table} WHERE {wh}")).await?;
                out.push(json!({ "outcome": "deleted" }));
            }
            _ => {
                let col = q(&ch.column);
                let now = db
                    .first(&format!("SELECT {} FROM {table}{lock_table} WHERE {wh}{lock_end}", as_text(kind, &col)))
                    .await?;
                let Some(now) = now else {
                    out.push(json!({ "outcome": "missing" }));
                    continue;
                };
                let now = now.into_iter().next().flatten();
                if !same(&now, &ch.old) {
                    out.push(json!({ "outcome": "conflict", "now": now }));
                    continue;
                }
                db.run(&format!("UPDATE {table} SET {col} = {} WHERE {wh}", literal(kind, &ch.new))).await?;
                out.push(json!({ "outcome": "changed" }));
            }
        }
    }
    db.commit().await?;
    drop((pg, my, ms));
    Ok(out)
}

/// Whether what the database holds now (as text) is what Calc read.
fn same(now: &Option<String>, old: &Value) -> bool {
    match (now, old) {
        (None, Value::Null) => true,
        (None, Value::String(s)) => s.is_empty(),
        (None, _) | (Some(_), Value::Null) => false,
        (Some(n), Value::String(s)) => n == s,
        (Some(n), Value::Number(x)) => n.parse::<f64>().ok().zip(x.as_f64()).is_some_and(|(a, b)| (a - b).abs() < 1e-9),
        (Some(n), Value::Bool(b)) => {
            matches!((n.as_str(), b), ("t" | "true" | "1", true) | ("f" | "false" | "0", false))
        }
        (Some(n), other) => n == &other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_statement_only() {
        assert_eq!(one_statement("SELECT 1;").unwrap(), "SELECT 1");
        assert_eq!(one_statement("SELECT ';' -- a; b\n").unwrap(), "SELECT ';' -- a; b");
        assert!(one_statement("SELECT 1; DELETE FROM t").is_err());
        assert!(one_statement("COMMIT; DROP TABLE x").is_err());
        assert!(one_statement("  ").is_err());
    }

    #[test]
    fn names_in_queries() {
        let sql = "SELECT a::text, ':not' FROM t WHERE y = :year AND m = :month -- :gone\n AND z = :year";
        assert_eq!(param_names(sql), vec!["year".to_string(), "month".to_string()]);
        let out = replace_params(sql, |n, name| format!("${n}<{name}>"));
        assert!(out.contains("y = $1<year>"));
        assert!(out.contains("z = $3<year>"));
        assert!(out.contains("a::text"));
    }

    #[test]
    fn sql_server_queries_only_read() {
        assert_eq!(mssql_refused("SELECT name, [set], 'COMMIT' FROM t -- delete\n WHERE x = N'exec'"), None);
        assert_eq!(mssql_refused("SELECT 1 COMMIT DELETE FROM t").as_deref(), Some("COMMIT"));
        assert_eq!(mssql_refused("select * from t\nexec('drop table t')").as_deref(), Some("EXEC"));
        assert_eq!(mssql_refused("SELECT a INTO copy FROM t").as_deref(), Some("INTO"));
        assert_eq!(mssql_refused("SELECT 1 /* x */ SET ROWCOUNT 0").as_deref(), Some("SET"));
        // Backticks aren't quotes in T-SQL: nothing hides behind them.
        assert_eq!(mssql_refused("SELECT 1 ` COMMIT `").as_deref(), Some("COMMIT"));
        assert_eq!(mssql_refused("SELECT updated_at, settings FROM t"), None);
    }

    #[test]
    fn local_addresses() {
        for ip in ["127.0.0.1", "0.0.0.0", "169.254.169.254", "::1", "fe80::1", "::ffff:127.0.0.1", "::"] {
            assert!(local_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["10.0.0.5", "192.168.1.9", "8.8.8.8", "fd00::1", "2001:db8::1"] {
            assert!(!local_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn same_place_is_host_and_port() {
        let a = Source { kind: "postgres".into(), address: "db.lan".into(), ..Default::default() };
        assert!(same_place(&a, &Source { address: "DB.lan:5432".into(), ..a.clone() }));
        assert!(!same_place(&a, &Source { address: "db.lan:5433".into(), ..a.clone() }));
        assert!(!same_place(&a, &Source { address: "evil.example".into(), ..a.clone() }));
        assert!(!same_place(&a, &Source { kind: "mysql".into(), ..a.clone() }));
        let f = Source { kind: "sqlite".into(), address: "/srv/a.sqlite".into(), ..Default::default() };
        assert!(!same_place(&f, &Source { address: "/srv/b.sqlite".into(), ..f.clone() }));
    }

    /// What a PostgreSQL server sends: R messages (authentication) and others.
    fn msg(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut m = vec![kind];
        m.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        m.extend_from_slice(body);
        m
    }

    #[test]
    fn unencrypted_passwords_are_refused() {
        let clear = msg(b'R', &3u32.to_be_bytes());
        let scram = msg(b'R', &[&10u32.to_be_bytes()[..], b"SCRAM-SHA-256\0\0"].concat());
        let ok = msg(b'R', &0u32.to_be_bytes());
        // Asked as it is: stopped, also when it comes a byte at a time.
        let mut w = Watching::Head([0; 5], 0);
        assert!(w.saw(&clear).is_err());
        let mut w = Watching::Answer;
        let mut sent = vec![b'N'];
        sent.extend(&clear);
        assert!(sent.iter().any(|b| w.saw(&[*b]).is_err()));
        // SCRAM, then signed in, then anything: fine.
        let mut w = Watching::Head([0; 5], 0);
        let mut sent = scram.clone();
        sent.extend(msg(b'R', &[&12u32.to_be_bytes()[..], b"v=abc"].concat()));
        sent.extend(&ok);
        sent.extend(&clear);
        assert!(w.saw(&sent).is_ok());
        assert!(matches!(w, Watching::Done));
        // Encrypted ('S'): not looked at.
        let mut w = Watching::Answer;
        assert!(w.saw(&[b'S']).is_ok() && w.saw(&clear).is_ok());
        // An error from the database before: still watched.
        let mut w = Watching::Head([0; 5], 0);
        let mut sent = msg(b'E', b"SFATAL\0\0");
        sent.extend(&clear);
        assert!(w.saw(&sent).is_err());
    }

    #[test]
    fn literals_are_quoted() {
        assert_eq!(literal("postgres", &json!("O'Brien")), "'O''Brien'");
        assert_eq!(literal("mysql", &json!("a\\'b")), "'a\\\\''b'");
        assert_eq!(literal("mssql", &json!(12)), "N'12'");
        assert_eq!(literal("postgres", &Value::Null), "NULL");
    }
}
