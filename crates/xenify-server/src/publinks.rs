//! Public links: a file, or a folder and everything in it, that anyone with
//! the link may read in a browser without an account.
//!
//! The server reads what's shared itself: a folder's link shows whatever
//! is in it now, moved-in files included, but never more than the person
//! who made the link may open (a folder in it with its own list they're
//! not on stays out, also for an admin: what admins may manage isn't what
//! they're given). A link stops when its maker is turned off.
//!
//! Stopping a link, or making a new one, makes the old link stop at once. Admins can turn public links off for the whole
//! server (Admin › Public links): then every link stops answering until
//! they're turned back on.

use crate::files::{Me, attachment, me, node_role};
use crate::{Server, db, err, failed, random, signed_in, unavailable};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use deadpool_postgres::{Client, GenericClient};
use serde::Deserialize;
use serde_json::json;
use xenify_server_proto::Role;

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/nodes/{id}/public", get(link_of).post(make).delete(stop))
        .route("/v1/p/{token}", get(open))
        .route("/v1/p/{token}/content/{node}", get(content))
        .route("/v1/p/{token}/page/{node}", get(page))
        .route("/v1/p/{token}/preview/{node}", get(preview))
        .route("/v1/admin/public-links", get(admin_list))
        .route("/v1/admin/public-links/{id}", axum::routing::delete(admin_stop))
}

/// Whether admins let people make and use public links (on unless turned off).
pub(crate) async fn allowed(c: &impl GenericClient) -> bool {
    db::setting(c, "public_links").await.ok().flatten().as_deref() != Some("off")
}

macro_rules! who {
    ($s:expr, $h:expr) => {{
        let account = match signed_in(&$s, &$h).await {
            Ok(a) => a,
            Err(e) => return e,
        };
        let Ok(c) = $s.pool.get().await else { return unavailable() };
        let Ok(me) = me(&c, account).await else { return failed() };
        (c, me)
    }};
}

fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "not_found", "it isn't there, or you can't open it")
}

fn turned_off() -> Response {
    err(StatusCode::FORBIDDEN, "links_off", "public links are turned off on this server")
}

/// Links that no longer work look the same as ones that never did.
fn gone() -> Response {
    err(StatusCode::NOT_FOUND, "gone", "this link doesn't work any more")
}

async fn role(c: &impl GenericClient, me: &Me, node: i64) -> Option<Role> {
    node_role(c, me, node).await.ok().flatten().and_then(|(_, r)| r)
}

/// Whether `node` is `root` or inside it, not in the bin, and (with every
/// folder between) something the link's maker may open.
async fn inside(c: &impl GenericClient, link: &Link, node: i64) -> bool {
    let Ok(rows) = c
        .query(
            "WITH RECURSIVE up(id, parent, trashed, own, depth) AS (
                 SELECT id, parent, trashed, own_access, 0 FROM nodes WHERE id = $1
                 UNION ALL
                 SELECT n.id, n.parent, n.trashed, n.own_access, up.depth + 1 FROM nodes n JOIN up ON n.id = up.parent
             )
             SELECT id, trashed IS NULL, own FROM up ORDER BY depth",
            &[&node],
        )
        .await
    else {
        return false;
    };
    // Up to the link's own: those to check (as `open` does).
    let mut check = Vec::new();
    let mut found = false;
    for r in &rows {
        if !r.get::<_, bool>(1) {
            return false;
        }
        let id: i64 = r.get(0);
        if id == link.root || r.get::<_, bool>(2) {
            check.push(id);
        }
        if id == link.root {
            found = true;
            break;
        }
    }
    if !found {
        return false;
    }
    for id in check {
        if !link.may_open(c, id).await {
            return false;
        }
    }
    true
}

/// A link that works: its id, the folder or file it shares, and who made it
/// (what they may open is the most it shows; None: made before that was kept).
struct Link {
    id: i64,
    root: i64,
    /// As the maker's own grants have it (not as an admin).
    maker: Option<Me>,
    /// The maker is an admin: the link's own folder or file is theirs to
    /// share however they reach it; what's in it still goes by their grants.
    admin: bool,
}

impl Link {
    async fn may_open(&self, c: &impl GenericClient, node: i64) -> bool {
        match &self.maker {
            Some(me) if node == self.root && self.admin => {
                let me = Me { account: xenify_server_proto::Account { admin: true, ..me.account.clone() }, groups: me.groups.clone() };
                role(c, &me, node).await.is_some()
            }
            Some(me) => role(c, me, node).await.is_some(),
            None => true,
        }
    }
}

fn link_json(r: &tokio_postgres::Row) -> serde_json::Value {
    json!({
        "id": r.get::<_, i64>(0),
        "token": r.get::<_, String>(1),
        "made": r.get::<_, String>(2),
        "opened": r.get::<_, Option<String>>(3),
        "files": r.get::<_, i64>(4),
    })
}

const LINK_COLS: &str = "l.id, l.token,
    to_char(l.made AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
    to_char(l.opened AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
    (WITH RECURSIVE down(id) AS (
         SELECT l.node UNION ALL SELECT n.id FROM nodes n JOIN down ON n.parent = down.id WHERE n.trashed IS NULL
     ) SELECT count(*) FROM down JOIN nodes n ON n.id = down.id WHERE n.kind = 'file')";

/// The node's public link, for people who may change it.
async fn link_of(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    if !role(&c, &me, id).await.is_some_and(|r| r >= Role::Editor) {
        return not_found();
    }
    let Ok(row) = c.query_opt(&format!("SELECT {LINK_COLS} FROM public_links l WHERE l.node = $1"), &[&id]).await
    else {
        return failed();
    };
    Json(json!({ "allowed": allowed(&c).await, "link": row.as_ref().map(link_json) })).into_response()
}

/// Makes the node's link, replacing any it had (so the old one stops).
async fn make(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (mut c, me) = who!(s, h);
    if !allowed(&c).await {
        return turned_off();
    }
    if !role(&c, &me, id).await.is_some_and(|r| r >= Role::Editor) {
        return err(StatusCode::FORBIDDEN, "not_allowed", "only people who can change it can share it with a link");
    }
    if !crate::perms::can(&c, me.account.id, "files.public_links").await {
        return crate::perms::denied("files.public_links");
    }
    if crate::files::plan_lacks(&c, &me, id, &[]).await {
        return crate::files::plan_refused();
    }
    let token = random(16);
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let done = async {
        tx.execute("DELETE FROM public_links WHERE node = $1", &[&id]).await?;
        let link: i64 = tx
            .query_one(
                "INSERT INTO public_links (token, node, made_by) VALUES ($1, $2, $3) RETURNING id",
                &[&token, &id, &me.account.id],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(link)
    }
    .await;
    match done {
        Ok(link) => Json(json!({ "id": link, "token": token })).into_response(),
        Err(_) => failed(),
    }
}

async fn stop(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    if !role(&c, &me, id).await.is_some_and(|r| r >= Role::Editor) {
        return not_found();
    }
    match c.execute("DELETE FROM public_links WHERE node = $1", &[&id]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

// ---- Reading with a link (no account) ------------------------------------

/// The link, while it works.
async fn live_link(c: &Client, token: &str) -> Result<Link, Response> {
    if !allowed(c).await {
        return Err(turned_off());
    }
    let r = match c
        .query_opt(
            "SELECT l.id, l.node, u.id, u.username, u.display_name, u.admin, u.email, u.disabled
             FROM public_links l LEFT JOIN users u ON u.id = l.made_by WHERE l.token = $1",
            &[&token],
        )
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return Err(gone()),
        Err(_) => return Err(failed()),
    };
    // Its maker turned off: as if it were stopped.
    if r.get::<_, Option<bool>>(7) == Some(true) {
        return Err(gone());
    }
    let admin = r.get::<_, Option<bool>>(5) == Some(true);
    let maker = match r.get::<_, Option<i64>>(2) {
        Some(id) => {
            let account = xenify_server_proto::Account {
                id,
                username: r.get(3),
                display_name: r.get(4),
                admin: false,
                email: r.get(6),
            };
            Some(me(c, account).await.map_err(|_| failed())?)
        }
        None => None,
    };
    Ok(Link { id: r.get(0), root: r.get(1), maker, admin })
}

/// Everything the link shares: the folder or file and, for a folder,
/// everything in it now (not what's in the bin).
async fn open(State(s): State<Server>, Path(token): Path<String>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let link = match live_link(&c, &token).await {
        Ok(l) => l,
        Err(e) => return e,
    };
    let root = link.root;
    let Ok(rows) = c
        .query(
            "WITH RECURSIVE down(id) AS (
                 SELECT id FROM nodes WHERE id = $1 AND trashed IS NULL
                 UNION ALL
                 SELECT n.id FROM nodes n JOIN down ON n.parent = down.id WHERE n.trashed IS NULL
             )
             SELECT n.id, n.parent, n.kind, n.meta, n.head, n.size,
                    to_char(n.changed AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), n.own_access
             FROM down JOIN nodes n ON n.id = down.id ORDER BY n.id",
            &[&root],
        )
        .await
    else {
        return failed();
    };
    // What the maker may open, from the top down: a folder they can't open
    // keeps everything in it out too. Only the link's own folder or file and
    // those with their own list can differ from the folder they're in.
    let mut inside: std::collections::HashMap<i64, Vec<&tokio_postgres::Row>> = std::collections::HashMap::new();
    for r in &rows {
        if let Some(p) = r.get::<_, Option<i64>>(1) {
            inside.entry(p).or_default().push(r);
        }
    }
    let mut shown = std::collections::HashSet::new();
    let mut todo: Vec<(i64, bool)> = vec![(root, true)];
    while let Some((id, own)) = todo.pop() {
        if own && !link.may_open(&c, id).await {
            continue;
        }
        shown.insert(id);
        if let Some(kids) = inside.get(&id) {
            todo.extend(kids.iter().map(|r| (r.get::<_, i64>(0), r.get::<_, bool>(7))));
        }
    }
    let items: Vec<_> = rows
        .iter()
        .filter(|r| shown.contains(&r.get::<_, i64>(0)))
        .map(|r| {
            let id: i64 = r.get(0);
            json!({
                "id": id,
                "parent": if id == root { None } else { r.get::<_, Option<i64>>(1) },
                "kind": r.get::<_, String>(2),
                "meta": r.get::<_, String>(3),
                "version": r.get::<_, i64>(4),
                "size": r.get::<_, i64>(5),
                "changed": r.get::<_, String>(6),
            })
        })
        .collect();
    // When it was last opened, at most once a minute.
    let _ = c
        .execute(
            "UPDATE public_links SET opened = now() WHERE id = $1 AND (opened IS NULL OR opened < now() - interval '1 minute')",
            &[&link.id],
        )
        .await;
    let name = db::setting(&c, "name").await.ok().flatten().unwrap_or_default();
    Json(json!({ "root": root, "server": name, "items": items })).into_response()
}

#[derive(Deserialize)]
struct AsFile {
    /// A Draw canvas with its pictures and clips (an `.xdraw` zip).
    #[serde(default)]
    file: bool,
}

/// A shared file's latest version.
async fn content(State(s): State<Server>, Path((token, node)): Path<(String, i64)>, Query(q): Query<AsFile>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let link = match live_link(&c, &token).await {
        Ok(l) => l,
        Err(e) => return e,
    };
    if !inside(&c, &link, node).await {
        return gone();
    }
    let row = c
        .query_opt(
            "SELECT v.blob, v.n, n.meta FROM versions v JOIN nodes n ON n.id = v.node WHERE v.node = $1 AND v.n = n.head",
            &[&node],
        )
        .await;
    let Ok(Some(row)) = row else {
        return err(StatusCode::NOT_FOUND, "empty", "nothing has been saved in this file yet");
    };
    let Ok(bytes) = s.blobs.get(row.get(0)).await else { return failed() };
    if q.file && crate::media::is_draw(row.get(2)) {
        return attachment(crate::media::put_together(&s, &c, node, bytes).await);
    }
    let n: i64 = row.get(1);
    attachment(
        ([(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::ETAG, format!("\"{n}\""))], bytes)
            .into_response(),
    )
}

/// A shared Write document as a web page: `{ html, title }`.
async fn page(State(s): State<Server>, Path((token, node)): Path<(String, i64)>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let link = match live_link(&c, &token).await {
        Ok(l) => l,
        Err(e) => return e,
    };
    if !inside(&c, &link, node).await {
        return gone();
    }
    match crate::pages::page(&s, &c, None, node).await {
        Ok(v) => Json(v).into_response(),
        Err(None) => err(
            StatusCode::NOT_FOUND,
            "no_page",
            "only Write documents with something saved in them can be shown as a page",
        ),
        Err(Some(e)) => {
            err(StatusCode::UNPROCESSABLE_ENTITY, "unreadable", &format!("this document can't be shown: {e}"))
        }
    }
}

/// A shared file's picture (a Draw canvas, a Calc book's tables).
async fn preview(State(s): State<Server>, Path((token, node)): Path<(String, i64)>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let link = match live_link(&c, &token).await {
        Ok(l) => l,
        Err(e) => return e,
    };
    if !inside(&c, &link, node).await {
        return gone();
    }
    let Ok(Some(row)) = c.query_opt("SELECT preview FROM nodes WHERE id = $1", &[&node]).await else {
        return gone();
    };
    let Some(hash) = row.get::<_, Option<String>>(0) else {
        return err(StatusCode::NOT_FOUND, "no_preview", "there's no picture of this file yet");
    };
    let Ok(bytes) = s.blobs.get(&hash).await else { return failed() };
    attachment(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

// ---- Admin › Public links ------------------------------------------------

/// Every link, with who made it and when it was last opened.
async fn admin_list(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.links").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Ok(rows) = c
        .query(
            &format!(
                "SELECT {LINK_COLS}, n.meta, n.kind, sp.name, sp.kind, u.username, u.display_name
                 FROM public_links l JOIN nodes n ON n.id = l.node JOIN spaces sp ON sp.id = n.space
                 LEFT JOIN users u ON u.id = l.made_by
                 ORDER BY l.made DESC"
            ),
            &[],
        )
        .await
    else {
        return failed();
    };
    let links: Vec<_> = rows
        .iter()
        .map(|r| {
            let mut l = link_json(r);
            // Kept: admins open a link to check what it shows (what anyone with it sees).
            l["meta"] = json!(r.get::<_, String>(5));
            l["kind"] = json!(r.get::<_, String>(6));
            l["space"] = json!(r.get::<_, String>(7));
            l["space_kind"] = json!(r.get::<_, String>(8));
            let user: Option<String> = r.get(9);
            let display: Option<String> = r.get(10);
            l["made_by"] = json!(display.filter(|d| !d.is_empty()).or(user));
            l
        })
        .collect();
    Json(json!({ "on": allowed(&c).await, "links": links })).into_response()
}

async fn admin_stop(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.links").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match c.execute("DELETE FROM public_links WHERE id = $1", &[&id]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}
