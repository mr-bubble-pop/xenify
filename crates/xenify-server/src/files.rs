//! Spaces, folders and files: who may do what, and every saved version.
//!
//! Names and details (`meta`) are JSON text the apps write; contents are
//! whatever the apps send, kept in the data folder by their SHA-256, so a
//! version is never changed once written.
//!
//! Access: a space has a list of grants (people, groups or everyone, each
//! with a role). A folder or file follows its folder, unless it has its own
//! list. "My files" belongs to its owner alone, who may still give single
//! folders and files their own list to share them. Admins manage every
//! shared space.

use crate::{Server, err, failed, signed_in, unavailable};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::{Client, GenericClient};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path as FsPath, PathBuf};
use xenify_server_proto::{
    Access, Account, Grant, NewNode, NewSpace, Node, NodeChange, NodeKind, Role, Saved, Space, SpaceKind, Version, Who,
};

/// The largest single save.
pub const MAX_FILE: usize = 1 << 30;

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/spaces", get(spaces).post(add_space))
        .route("/v1/spaces/{id}", post(rename_space).delete(delete_space))
        .route("/v1/spaces/{id}/access", get(space_access).post(set_space_access))
        .route("/v1/nodes", get(children).post(add_node))
        .route("/v1/nodes/{id}", get(node).post(change_node).delete(trash_node))
        .route("/v1/nodes/{id}/restore", post(restore_node))
        .route("/v1/nodes/{id}/forever", axum::routing::delete(delete_forever))
        .route("/v1/trash", get(trash_list).delete(empty_trash))
        .route("/v1/nodes/{id}/access", get(node_access).post(set_node_access))
        .route("/v1/nodes/{id}/versions", get(versions).delete(forget_versions))
        .route("/v1/nodes/{id}/versions/{n}", post(label_version).delete(forget_version))
        .route("/v1/nodes/{id}/content", get(content).post(save).layer(DefaultBodyLimit::max(MAX_FILE)))
        .route("/v1/nodes/{id}/page", get(page))
        .route("/v1/nodes/{id}/compare", get(compare))
        .route("/v1/nodes/{id}/as-new", get(as_new))
        .route("/v1/nodes/{id}/blame", get(blame))
        .route(
            "/v1/nodes/{id}/pictures/{*name}",
            get(picture).put(put_picture).layer(DefaultBodyLimit::max(MAX_FILE)),
        )
        .route("/v1/nodes/{id}/preview", get(preview).post(save_preview).layer(DefaultBodyLimit::max(64 << 20)))
        .route("/v1/everything", get(everything))
        .route("/v1/search", get(search))
        .route("/v1/search/ai/words", post(ai_words))
        .route("/v1/search/ai/read", post(ai_read))
        .route("/v1/nodes/{id}/text", axum::routing::put(put_text).layer(DefaultBodyLimit::max(MAX_TEXT)))
        .route("/v1/recent", get(recent))
        .route("/v1/shared", get(shared))
        .route("/v1/directory", get(directory))
}

// ---- Blobs -------------------------------------------------------------

/// Files' bytes, named by their SHA-256.
pub struct Blobs {
    dir: PathBuf,
}

impl Blobs {
    pub fn open(dir: &FsPath) -> std::io::Result<Blobs> {
        std::fs::create_dir_all(dir)?;
        Ok(Blobs { dir: dir.to_path_buf() })
    }

    fn path(&self, hash: &str) -> PathBuf {
        self.dir.join(&hash[..2]).join(&hash[2..])
    }

    pub async fn put(&self, bytes: &[u8]) -> std::io::Result<String> {
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        let path = self.path(&hash);
        if tokio::fs::try_exists(&path).await? {
            touch(&path).await;
            return Ok(hash);
        }
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let tmp = path.with_extension(format!("tmp{}", crate::random(6)));
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(hash)
    }

    /// Where the file with this hash is kept (if the hash is one).
    pub fn file(&self, hash: &str) -> Option<PathBuf> {
        (hash.len() >= 3 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| self.path(hash))
    }

    /// Keeps file `from` (already checked to hold what `hash` names) as
    /// that hash; it's moved, not copied.
    pub async fn adopt(&self, from: &FsPath, hash: &str) -> std::io::Result<()> {
        let path = self.file(hash).ok_or(std::io::ErrorKind::InvalidInput)?;
        if tokio::fs::try_exists(&path).await? {
            touch(&path).await;
            return tokio::fs::remove_file(from).await;
        }
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        tokio::fs::rename(from, &path).await
    }

    pub async fn get(&self, hash: &str) -> std::io::Result<Vec<u8>> {
        if hash.len() < 3 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        tokio::fs::read(self.path(hash)).await
    }
}

/// Marks stored bytes as just used, so the clean-up (cleanup.rs) doesn't
/// take bytes an upload is about to name again.
async fn touch(path: &FsPath) {
    let path = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        std::fs::File::options().write(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()))
    })
    .await;
}

// ---- Roles -------------------------------------------------------------

fn role_of(s: &str) -> Role {
    match s {
        "manager" => Role::Manager,
        "editor" => Role::Editor,
        "commenter" => Role::Commenter,
        _ => Role::Viewer,
    }
}

fn role_name(r: Role) -> &'static str {
    match r {
        Role::Viewer => "viewer",
        Role::Commenter => "commenter",
        Role::Editor => "editor",
        Role::Manager => "manager",
    }
}

fn kind_of(s: &str) -> SpaceKind {
    match s {
        "personal" => SpaceKind::Personal,
        "templates" => SpaceKind::Templates,
        "chat" => SpaceKind::Chat,
        _ => SpaceKind::Shared,
    }
}

/// Who is asking: their account and groups.
pub struct Me {
    pub account: Account,
    pub groups: Vec<i64>,
}

pub(crate) async fn me(c: &Client, account: Account) -> Result<Me, tokio_postgres::Error> {
    let groups = c
        .query("SELECT group_id FROM group_members WHERE user_id = $1", &[&account.id])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    Ok(Me { account, groups })
}

/// The best role these grants give `me`.
fn best(grants: &[Grant], me: &Me) -> Option<Role> {
    grants
        .iter()
        .filter(|g| match g.who {
            Who::User(u) => u == me.account.id,
            Who::Group(gr) => me.groups.contains(&gr),
            Who::Everyone => true,
        })
        .map(|g| g.role)
        .max()
}

async fn grants_on(
    c: &impl GenericClient,
    space: Option<i64>,
    node: Option<i64>,
) -> Result<Vec<Grant>, tokio_postgres::Error> {
    let rows = c
        .query(
            "SELECT user_id, group_id, everyone, role, custom FROM grants
             WHERE ($1::BIGINT IS NOT NULL AND space = $1) OR ($2::BIGINT IS NOT NULL AND node = $2) ORDER BY id",
            &[&space, &node],
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let who = match (r.get::<_, Option<i64>>(0), r.get::<_, Option<i64>>(1), r.get::<_, bool>(2)) {
                (Some(u), _, _) => Who::User(u),
                (_, Some(g), _) => Who::Group(g),
                (_, _, true) => Who::Everyone,
                _ => return None,
            };
            Some(Grant { who, role: role_of(r.get(3)), custom: r.get(4) })
        })
        .collect())
}

struct SpaceRow {
    id: i64,
    kind: SpaceKind,
    owner: Option<i64>,
    name: String,
    /// Chat spaces: their conversation.
    conv: Option<i64>,
}

async fn space_row(c: &impl GenericClient, id: i64) -> Result<Option<SpaceRow>, tokio_postgres::Error> {
    Ok(c.query_opt("SELECT id, kind, owner, name, conv FROM spaces WHERE id = $1", &[&id]).await?.map(|r| SpaceRow {
        id: r.get(0),
        kind: kind_of(r.get(1)),
        owner: r.get(2),
        name: r.get(3),
        conv: r.get(4),
    }))
}

/// What `me` may do in a whole space.
async fn space_role(c: &impl GenericClient, me: &Me, s: &SpaceRow) -> Result<Option<Role>, tokio_postgres::Error> {
    Ok(match s.kind {
        SpaceKind::Personal => (s.owner == Some(me.account.id)).then_some(Role::Manager),
        // A chat's whiteboards: its members edit them; admins aren't special.
        SpaceKind::Chat => c
            .query_opt(
                "SELECT 1 FROM members WHERE conv = $1 AND user_id = $2",
                &[&s.conv.unwrap_or(-1), &me.account.id],
            )
            .await?
            .map(|_| Role::Editor),
        _ if me.account.admin => Some(Role::Manager),
        _ => best(&grants_on(c, Some(s.id), None).await?, me),
    })
}

/// What `me` may do with a folder or file: the first own list on the way up
/// decides, else the space's. The owner of "My files" and admins (in shared
/// spaces) may always do everything.
pub(crate) async fn node_role(
    c: &impl GenericClient,
    me: &Me,
    node: i64,
) -> Result<Option<(i64, Option<Role>)>, tokio_postgres::Error> {
    node_role_in(c, me, node, false).await
}

/// node_role, also for something in the bin: what `me` could do with it
/// before it was deleted (the own lists on the way up still count).
async fn role_before_bin(c: &impl GenericClient, me: &Me, node: i64) -> Option<Role> {
    node_role_in(c, me, node, true).await.ok().flatten().and_then(|(_, r)| r)
}

async fn node_role_in(
    c: &impl GenericClient,
    me: &Me,
    node: i64,
    binned: bool,
) -> Result<Option<(i64, Option<Role>)>, tokio_postgres::Error> {
    let chain = c
        .query(
            "WITH RECURSIVE up(id, parent, own, space, trashed, depth) AS (
                 SELECT id, parent, own_access, space, trashed, 0 FROM nodes WHERE id = $1
                 UNION ALL
                 SELECT n.id, n.parent, n.own_access, n.space, n.trashed, up.depth + 1 FROM nodes n JOIN up ON n.id = up.parent
             )
             SELECT id, own, space, trashed IS NOT NULL FROM up ORDER BY depth",
            &[&node],
        )
        .await?;
    let Some(first) = chain.first() else { return Ok(None) };
    if first.get::<_, bool>(3) && !binned {
        return Ok(None);
    }
    let space_id: i64 = first.get(2);
    let Some(space) = space_row(c, space_id).await? else { return Ok(None) };
    let whole = space_role(c, me, &space).await?;
    let always = match space.kind {
        SpaceKind::Personal | SpaceKind::Chat => whole,
        _ if me.account.admin => whole,
        _ => None,
    };
    if always.is_some() {
        return Ok(Some((space_id, always)));
    }
    for r in &chain {
        if r.get::<_, bool>(1) {
            let own: i64 = r.get(0);
            return Ok(Some((space_id, best(&grants_on(c, None, Some(own)).await?, me))));
        }
    }
    Ok(Some((space_id, whole)))
}

/// The Plan changes `me` may make in plan `node`, from the grants that
/// decide their access there: a built-in editor (or better) may make every
/// change, a commenter only comments, a document role exactly what it lists
/// (the most any of their grants allows), even when it works like "Can
/// edit". Empty (also when anything goes wrong): they may not change anything.
pub(crate) async fn plan_perms(c: &impl GenericClient, me: &Me, node: i64) -> Vec<String> {
    plan_perms_of(c, me, node).await.unwrap_or_default()
}

/// Whether `node` is a plan and `me`'s document role there doesn't allow
/// every change in `needs` (empty: every change a plain editor may make).
/// What changes the whole file (writing it over, moving it, deleting it, a
/// public link) needs everything; renaming it, its set-up. The changes such
/// a role allows are checked one by one while editing together (live.rs).
pub(crate) async fn plan_lacks(c: &impl GenericClient, me: &Me, node: i64, needs: &[&str]) -> bool {
    if !crate::plan::is_plan(c, node).await {
        return false;
    }
    let has = plan_perms(c, me, node).await;
    let all = crate::perms::all_plan();
    let needs: Vec<&str> = if needs.is_empty() { all.iter().map(String::as_str).collect() } else { needs.to_vec() };
    needs.iter().any(|n| !has.iter().any(|h| h == n))
}

/// Said when plan_lacks.
pub(crate) fn plan_refused() -> Response {
    err(StatusCode::FORBIDDEN, "not_allowed", "your role in this plan doesn’t allow that")
}

async fn plan_perms_of(c: &impl GenericClient, me: &Me, node: i64) -> Option<Vec<String>> {
    let (space_id, role) = node_role(c, me, node).await.ok()??;
    let role = role?;
    let space = space_row(c, space_id).await.ok()??;
    // Owners, admins and chat members aren't limited by grants.
    let grants = match space.kind {
        SpaceKind::Personal if space.owner == Some(me.account.id) => return Some(crate::perms::all_plan()),
        SpaceKind::Chat => return (role >= Role::Editor).then(crate::perms::all_plan),
        SpaceKind::Shared | SpaceKind::Templates if me.account.admin => return Some(crate::perms::all_plan()),
        _ => {
            let chain = c
                .query(
                    "WITH RECURSIVE up(id, parent, own, depth) AS (
                         SELECT id, parent, own_access, 0 FROM nodes WHERE id = $1
                         UNION ALL SELECT n.id, n.parent, n.own_access, up.depth + 1 FROM nodes n JOIN up ON n.id = up.parent)
                     SELECT id FROM up WHERE own ORDER BY depth LIMIT 1",
                    &[&node],
                )
                .await
                .ok()?;
            match chain.first() {
                Some(r) => grants_on(c, None, Some(r.get(0))).await.ok()?,
                None => grants_on(c, Some(space_id), None).await.ok()?,
            }
        }
    };
    let mut out: Vec<String> = Vec::new();
    for g in grants.iter().filter(|g| match g.who {
        Who::User(u) => u == me.account.id,
        Who::Group(gr) => me.groups.contains(&gr),
        Who::Everyone => true,
    }) {
        let more = match (g.custom, g.role) {
            (Some(id), _) => crate::perms::doc_role(c, id).await.map(|(_, p)| p).unwrap_or_default(),
            (None, Role::Editor | Role::Manager) => crate::perms::all_plan(),
            (None, Role::Commenter) => vec!["plan.comment".into()],
            (None, Role::Viewer) => Vec::new(),
        };
        for p in more {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    Some(out)
}

// ---- Handlers ------------------------------------------------------------

/// Signs in, gets a connection and works out who's asking.
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

pub(crate) fn need(role: Option<Role>, at_least: Role) -> Result<Role, Response> {
    match role {
        Some(r) if r >= at_least => Ok(r),
        Some(_) => Err(err(StatusCode::FORBIDDEN, "not_allowed", "you can't do that here: ask someone who manages it")),
        None => Err(not_found()),
    }
}

async fn ensure_personal(c: &Client, user: i64) -> Result<(), tokio_postgres::Error> {
    c.execute(
        "INSERT INTO spaces (kind, owner) VALUES ('personal', $1) ON CONFLICT (owner) WHERE kind = 'personal' DO NOTHING",
        &[&user],
    )
    .await?;
    Ok(())
}

async fn spaces(State(s): State<Server>, h: HeaderMap) -> Response {
    let (c, me) = who!(s, h);
    if ensure_personal(&c, me.account.id).await.is_err() {
        return failed();
    }
    let Ok(rows) = c
        .query(
            "SELECT id, kind, owner, name FROM spaces WHERE (kind <> 'personal' OR owner = $1) AND kind <> 'chat' ORDER BY
             CASE kind WHEN 'personal' THEN 0 WHEN 'shared' THEN 1 ELSE 2 END, id",
            &[&me.account.id],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        let row = SpaceRow { id: r.get(0), kind: kind_of(r.get(1)), owner: r.get(2), name: r.get(3), conv: None };
        let Ok(role) = space_role(&c, &me, &row).await else { return failed() };
        if role.is_some() {
            out.push(Space { id: row.id, kind: row.kind, name: row.name, role });
        }
    }
    Json(out).into_response()
}

async fn add_space(State(s): State<Server>, h: HeaderMap, Json(n): Json<NewSpace>) -> Response {
    let (c, me) = who!(s, h);
    if !crate::perms::can(&c, me.account.id, "files.spaces").await {
        return crate::perms::denied("files.spaces");
    }
    // Everyone can read and change it unless the maker chose who; the
    // maker always manages it (a space nobody may open helps no one).
    let mut grants = n.grants.unwrap_or_else(|| vec![Grant { who: Who::Everyone, role: Role::Editor, custom: None }]);
    grants.retain(|g| g.who != Who::User(me.account.id));
    grants.push(Grant { who: Who::User(me.account.id), role: Role::Manager, custom: None });
    let mut c = c;
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let Ok(r) = tx.query_one("INSERT INTO spaces (kind, name) VALUES ('shared', $1) RETURNING id", &[&n.name]).await else {
        return failed();
    };
    let id: i64 = r.get(0);
    if write_grants(&tx, Some(id), None, &grants).await.is_err() || tx.commit().await.is_err() {
        return err(StatusCode::BAD_REQUEST, "bad_grant", "someone or some group in the list doesn't exist");
    }
    Json(serde_json::json!({ "id": id })).into_response()
}

async fn rename_space(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Json(n): Json<NewSpace>) -> Response {
    let (c, me) = who!(s, h);
    let Ok(Some(row)) = space_row(&c, id).await else { return not_found() };
    if row.kind != SpaceKind::Shared {
        return err(StatusCode::BAD_REQUEST, "fixed", "this space's name can't change");
    }
    if let Err(e) = need(space_role(&c, &me, &row).await.ok().flatten(), Role::Manager) {
        return e;
    }
    match c.execute("UPDATE spaces SET name = $2 WHERE id = $1", &[&id, &n.name]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

async fn delete_space(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let Ok(Some(row)) = space_row(&c, id).await else { return not_found() };
    if row.kind != SpaceKind::Shared {
        return err(StatusCode::FORBIDDEN, "not_allowed", "only shared spaces can be removed");
    }
    if let Err(e) = need(space_role(&c, &me, &row).await.ok().flatten(), Role::Manager) {
        return e;
    }
    match c.execute("DELETE FROM spaces WHERE id = $1", &[&id]).await {
        Ok(_) => {
            crate::live::check_space(&s, id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => failed(),
    }
}

async fn space_access(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let Ok(Some(row)) = space_row(&c, id).await else { return not_found() };
    if let Err(e) = need(space_role(&c, &me, &row).await.ok().flatten(), Role::Viewer) {
        return e;
    }
    match grants_on(&c, Some(id), None).await {
        Ok(grants) => Json(Access { own: true, grants }).into_response(),
        Err(_) => failed(),
    }
}

async fn write_grants(
    c: &impl GenericClient,
    space: Option<i64>,
    node: Option<i64>,
    grants: &[Grant],
) -> Result<(), tokio_postgres::Error> {
    c.execute(
        "DELETE FROM grants WHERE ($1::BIGINT IS NOT NULL AND space = $1) OR ($2::BIGINT IS NOT NULL AND node = $2)",
        &[&space, &node],
    )
    .await?;
    for g in grants {
        let (u, gr, every) = match g.who {
            Who::User(u) => (Some(u), None, false),
            Who::Group(g) => (None, Some(g), false),
            Who::Everyone => (None, None, true),
        };
        // A document role: its base is what the grant is, whatever was sent.
        let (role, custom) = match g.custom {
            Some(id) => match crate::perms::doc_role(c, id).await {
                Some((base, _)) => (base, Some(id)),
                None => (role_name(g.role).to_string(), None),
            },
            None => (role_name(g.role).to_string(), None),
        };
        c.execute(
            "INSERT INTO grants (space, node, user_id, group_id, everyone, role, custom) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[&space, &node, &u, &gr, &every, &role, &custom],
        )
        .await?;
    }
    Ok(())
}

async fn set_space_access(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(a): Json<Access>,
) -> Response {
    let (mut c, me) = who!(s, h);
    let Ok(Some(row)) = space_row(&c, id).await else { return not_found() };
    if row.kind == SpaceKind::Personal {
        return err(
            StatusCode::BAD_REQUEST,
            "personal",
            "My files is only yours: share single folders or files instead",
        );
    }
    if let Err(e) = need(space_role(&c, &me, &row).await.ok().flatten(), Role::Manager) {
        return e;
    }
    let Ok(tx) = c.transaction().await else { return unavailable() };
    if write_grants(&tx, Some(id), None, &a.grants).await.is_err() || tx.commit().await.is_err() {
        return err(StatusCode::BAD_REQUEST, "bad_grant", "someone or some group in the list doesn't exist");
    }
    StatusCode::NO_CONTENT.into_response()
}

const NODE_COLS: &str = "n.id, n.space, n.parent, n.kind, n.meta, n.size, n.head,
    to_char(n.changed AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), n.changed_by, n.own_access,
    to_char(n.preview_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')";

fn node_of(r: &tokio_postgres::Row, role: Role) -> Node {
    Node {
        id: r.get(0),
        space: r.get(1),
        parent: r.get(2),
        kind: if r.get::<_, &str>(3) == "folder" { NodeKind::Folder } else { NodeKind::File },
        meta: r.get(4),
        size: r.get(5),
        version: r.get(6),
        changed: r.get(7),
        changed_by: r.get(8),
        role,
        own_access: r.get(9),
        pictured: r.get(10),
    }
}

async fn node_row(c: &impl GenericClient, id: i64) -> Result<Option<tokio_postgres::Row>, tokio_postgres::Error> {
    c.query_opt(&format!("SELECT {NODE_COLS} FROM nodes n WHERE n.id = $1 AND n.trashed IS NULL"), &[&id]).await
}

#[derive(Deserialize)]
struct ChildrenOf {
    space: i64,
    parent: Option<i64>,
}

async fn children(State(s): State<Server>, h: HeaderMap, Query(q): Query<ChildrenOf>) -> Response {
    let (c, me) = who!(s, h);
    let role = match q.parent {
        Some(p) => match node_role(&c, &me, p).await {
            Ok(Some((space, role))) if space == q.space => role,
            Ok(_) => None,
            Err(_) => return failed(),
        },
        None => match space_row(&c, q.space).await {
            Ok(Some(row)) => space_role(&c, &me, &row).await.ok().flatten(),
            _ => None,
        },
    };
    let Ok(role) = need(role, Role::Viewer) else { return not_found() };
    let Ok(rows) = c
        .query(
            &format!(
                "SELECT {NODE_COLS} FROM nodes n WHERE n.space = $1 AND n.parent IS NOT DISTINCT FROM $2
                 AND n.trashed IS NULL ORDER BY n.kind DESC, n.id"
            ),
            &[&q.space, &q.parent],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        let own: bool = r.get(9);
        let child_role = if own {
            match node_role(&c, &me, r.get(0)).await {
                Ok(Some((_, Some(cr)))) => cr,
                Ok(_) => continue,
                Err(_) => return failed(),
            }
        } else {
            role
        };
        out.push(node_of(&r, child_role));
    }
    Json(out).into_response()
}

async fn add_node(State(s): State<Server>, h: HeaderMap, Json(n): Json<NewNode>) -> Response {
    let (c, me) = who!(s, h);
    let role = match n.parent {
        Some(p) => match node_role(&c, &me, p).await {
            Ok(Some((space, role))) if space == n.space => role,
            _ => None,
        },
        None => match space_row(&c, n.space).await {
            Ok(Some(row)) => space_role(&c, &me, &row).await.ok().flatten(),
            _ => None,
        },
    };
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    if let Some(p) = n.parent {
        match holder(&c, p).await {
            Some(Holder::Folder) => {}
            // Under a page: only pages (a wiki's pages hold pages).
            Some(Holder::Page) if n.kind == NodeKind::File && is_page(&n.meta) => {}
            Some(Holder::Page) => return err(StatusCode::BAD_REQUEST, "not_folder", "only pages go under a page"),
            None => return err(StatusCode::BAD_REQUEST, "not_folder", "things can only go in folders"),
        }
    }
    let kind = if n.kind == NodeKind::Folder { "folder" } else { "file" };
    match c
        .query_one(
            "INSERT INTO nodes (space, parent, kind, meta, changed_by) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&n.space, &n.parent, &kind, &n.meta, &me.account.id],
        )
        .await
    {
        Ok(r) => Json(serde_json::json!({ "id": r.get::<_, i64>(0) })).into_response(),
        Err(_) => failed(),
    }
}

async fn node(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    let Ok(role) = need(role, Role::Viewer) else { return not_found() };
    match node_row(&c, id).await {
        Ok(Some(r)) => Json(node_of(&r, role)).into_response(),
        Ok(None) => not_found(),
        Err(_) => failed(),
    }
}

async fn change_node(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(ch): Json<NodeChange>,
) -> Response {
    let (mut c, me) = who!(s, h);
    let Some((space, role)) = node_role(&c, &me, id).await.ok().flatten() else { return not_found() };
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    // A plan shared with a document role: moving it needs every change; its
    // details (its name), changing its set-up.
    if (ch.move_to.is_some() && plan_lacks(&c, &me, id, &[]).await)
        || (ch.meta.is_some() && plan_lacks(&c, &me, id, &["plan.setup"]).await)
    {
        return plan_refused();
    }
    if let Some(to) = &ch.move_to {
        // Into a folder of the same space that you may change, and never
        // into itself or below itself.
        let target = match to.parent {
            Some(p) => match node_role(&c, &me, p).await {
                Ok(Some((sp, r))) if sp == space => r,
                _ => None,
            },
            None => match space_row(&c, space).await {
                Ok(Some(row)) => space_role(&c, &me, &row).await.ok().flatten(),
                _ => None,
            },
        };
        if let Err(e) = need(target, Role::Editor) {
            return e;
        }
        if let Some(p) = to.parent {
            let below = c
                .query_opt(
                    "WITH RECURSIVE up(id, parent) AS (SELECT id, parent FROM nodes WHERE id = $1
                     UNION ALL SELECT n.id, n.parent FROM nodes n JOIN up ON n.id = up.parent)
                     SELECT 1 FROM up WHERE id = $2",
                    &[&p, &id],
                )
                .await;
            // A folder holds anything; a page only pages.
            let page_now = match &ch.meta {
                Some(m) => is_page(m),
                None => c.query_one("SELECT meta FROM nodes WHERE id = $1", &[&id]).await.is_ok_and(|r| is_page(r.get(0))),
            };
            let fits = match holder(&c, p).await {
                Some(Holder::Folder) => true,
                Some(Holder::Page) => page_now,
                None => false,
            };
            if !matches!(below, Ok(None)) || !fits {
                return err(StatusCode::BAD_REQUEST, "bad_move", "it can't go there");
            }
        }
    }
    if let Some(m) = &ch.meta
        && !is_page(m)
        && c.query_opt("SELECT 1 FROM nodes WHERE parent = $1 AND trashed IS NULL", &[&id]).await.is_ok_and(|r| r.is_some())
        && holder(&c, id).await == Some(Holder::Page)
    {
        return err(StatusCode::CONFLICT, "has_pages", "it has pages under it, so it stays a page");
    }
    // Only its place among the pages beside it (Move up, Move down): tidying,
    // not a change to it, so it isn't "changed" (nor by whom).
    let tidying = match &ch.meta {
        Some(m) => c
            .query_opt("SELECT meta FROM nodes WHERE id = $1", &[&id])
            .await
            .ok()
            .flatten()
            .is_some_and(|r| same_but_order(r.get(0), m)),
        None => false,
    };
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let done = async {
        if let Some(m) = &ch.meta {
            if tidying {
                tx.execute("UPDATE nodes SET meta = $2 WHERE id = $1", &[&id, m]).await?;
            } else {
                tx.execute(
                    "UPDATE nodes SET meta = $2, changed = now(), changed_by = $3 WHERE id = $1",
                    &[&id, m, &me.account.id],
                )
                .await?;
            }
        }
        if let Some(to) = &ch.move_to {
            tx.execute("UPDATE nodes SET parent = $2 WHERE id = $1", &[&id, &to.parent]).await?;
        }
        tx.commit().await
    }
    .await;
    match done {
        Ok(()) => {
            // Somewhere else, other lists may apply: open windows check again.
            if ch.move_to.is_some() {
                crate::live::check_node(&s, id).await;
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => failed(),
    }
}

/// What can hold other things: a folder (anything) or a page (pages).
#[derive(PartialEq)]
enum Holder {
    Folder,
    Page,
}

async fn holder(c: &impl GenericClient, id: i64) -> Option<Holder> {
    let r = c.query_opt("SELECT kind, meta FROM nodes WHERE id = $1 AND trashed IS NULL", &[&id]).await.ok()??;
    match r.get::<_, &str>(0) {
        "folder" => Some(Holder::Folder),
        _ if is_page(r.get(1)) => Some(Holder::Page),
        _ => None,
    }
}

/// Whether two sets of details differ only in "order" (a page's place).
fn same_but_order(a: &str, b: &str) -> bool {
    let strip = |m: &str| {
        let mut v = serde_json::from_str::<serde_json::Value>(m).ok()?;
        v.as_object_mut()?.remove("order");
        Some(v)
    };
    matches!((strip(a), strip(b)), (Some(x), Some(y)) if x == y)
}

/// Whether a file's details (`meta`) say it's a page of a space's wiki.
fn is_page(meta: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(meta).is_ok_and(|d| d["page"] == true)
}

async fn trash_node(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    if plan_lacks(&c, &me, id, &[]).await {
        return plan_refused();
    }
    let r = c
        .execute(
            "WITH RECURSIVE down(id) AS (SELECT $1::BIGINT UNION ALL SELECT n.id FROM nodes n JOIN down ON n.parent = down.id)
             UPDATE nodes SET trashed = now(), trashed_by = $2 WHERE id IN (SELECT id FROM down) AND trashed IS NULL",
            &[&id, &me.account.id],
        )
        .await;
    match r {
        Ok(_) => {
            // Whoever has it (or something in it) open stops at once.
            crate::live::check_node(&s, id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct TrashOf {
    space: i64,
}

/// A space's bin, for whoever may change the space: what was deleted (the
/// folder, file or page deleted, not each thing that went with it), the
/// latest first, with when, by whom and how many went with it. Only what
/// they could change before it was deleted (a folder with its own list they
/// weren't on stays out).
async fn trash_list(State(s): State<Server>, h: HeaderMap, Query(q): Query<TrashOf>) -> Response {
    let (c, me) = who!(s, h);
    let role = match space_row(&c, q.space).await {
        Ok(Some(row)) => space_role(&c, &me, &row).await.ok().flatten(),
        _ => None,
    };
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    let Ok(rows) = c
        .query(
            &format!(
                "SELECT {NODE_COLS}, to_char(n.trashed AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), n.trashed_by,
                   (SELECT count(*) FROM nodes k WHERE k.space = n.space AND k.trashed = n.trashed AND k.id <> n.id)
                 FROM nodes n LEFT JOIN nodes p ON p.id = n.parent
                 WHERE n.space = $1 AND n.trashed IS NOT NULL
                   AND (p.id IS NULL OR p.trashed IS NULL OR p.trashed <> n.trashed)
                 ORDER BY n.trashed DESC LIMIT 200"
            ),
            &[&q.space],
        )
        .await
    else {
        return failed();
    };
    let days = crate::cleanup::bin_days(&c).await;
    let mut mine = Vec::new();
    for r in rows {
        let own: bool = r.get(9);
        // Only something with its own list, or under one, can differ from the space.
        let role = if own || r.get::<_, Option<i64>>(2).is_some() {
            role_before_bin(&c, &me, r.get(0)).await
        } else {
            role
        };
        if let Some(role) = role.filter(|r| *r >= Role::Editor) {
            mine.push((r, role));
        }
    }
    let list: Vec<_> = mine
        .iter()
        .map(|(r, role)| {
            let mut v = serde_json::to_value(node_of(r, *role)).unwrap_or_default();
            let trashed: String = r.get(11);
            // When it's deleted for good (cleanup.rs), unless the bin is kept.
            if let (Some(d), Ok(t)) = (days, chrono::DateTime::parse_from_rfc3339(&trashed)) {
                v["gone"] = serde_json::json!((t + chrono::Duration::days(d)).format("%Y-%m-%dT%H:%M:%SZ").to_string());
            }
            v["trashed"] = serde_json::json!(trashed);
            v["trashed_by"] = serde_json::json!(r.get::<_, Option<i64>>(12));
            v["with"] = serde_json::json!(r.get::<_, i64>(13));
            v
        })
        .collect();
    Json(list).into_response()
}

/// Whether `me` manages space `space` (who may empty its bin).
async fn manages_space(c: &deadpool_postgres::Client, me: &Me, space: i64) -> Result<(), Response> {
    let role = match space_row(c, space).await {
        Ok(Some(row)) => space_role(c, me, &row).await.ok().flatten(),
        _ => None,
    };
    need(role, Role::Manager).map(|_| ())
}

/// Deletes something in the bin for good, with what went with it (for
/// the space's managers).
async fn delete_forever(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let Ok(Some(r)) = c.query_opt("SELECT space FROM nodes WHERE id = $1 AND trashed IS NOT NULL", &[&id]).await else {
        return not_found();
    };
    if let Err(e) = manages_space(&c, &me, r.get(0)).await {
        return e;
    }
    // What's under it goes too (it was deleted with it).
    match c.execute("DELETE FROM nodes WHERE id = $1 AND trashed IS NOT NULL", &[&id]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// Empties a space's bin (for its managers).
async fn empty_trash(State(s): State<Server>, h: HeaderMap, Query(q): Query<TrashOf>) -> Response {
    let (c, me) = who!(s, h);
    if let Err(e) = manages_space(&c, &me, q.space).await {
        return e;
    }
    match c.execute("DELETE FROM nodes WHERE space = $1 AND trashed IS NOT NULL", &[&q.space]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// Puts back something deleted, with what went with it. Where it was, when
/// that's still there; otherwise at the top of its space, keeping who may
/// open it: the list it was under comes with it. For whoever could change
/// it before it was deleted, and may change where it goes back to.
async fn restore_node(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (mut c, me) = who!(s, h);
    let Ok(Some(r)) = c
        .query_opt(
            "SELECT n.space, n.parent, p.trashed IS NULL AND p.id IS NOT NULL, n.trashed IS NOT NULL
             FROM nodes n LEFT JOIN nodes p ON p.id = n.parent WHERE n.id = $1",
            &[&id],
        )
        .await
    else {
        return not_found();
    };
    if !r.get::<_, bool>(3) {
        return err(StatusCode::CONFLICT, "not_deleted", "it isn't deleted");
    }
    let space: i64 = r.get(0);
    let parent: Option<i64> = r.get(1);
    let parent_alive = r.get::<_, Option<bool>>(2).unwrap_or(false);
    // Whoever may change where it goes back to.
    let role = match parent.filter(|_| parent_alive) {
        Some(p) => node_role(&c, &me, p).await.ok().flatten().and_then(|(_, r)| r),
        None => match space_row(&c, space).await {
            Ok(Some(row)) => space_role(&c, &me, &row).await.ok().flatten(),
            _ => None,
        },
    };
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    if let Err(e) = need(role_before_bin(&c, &me, id).await, Role::Editor) {
        return e;
    }
    // To the top of the space: the nearest own list it was under (itself
    // or a folder it was in), so it doesn't open up to the whole space.
    let carry: Option<i64> = if parent.is_some() && !parent_alive {
        let Ok(r) = c
            .query_opt(
                "WITH RECURSIVE up(id, parent, own, depth) AS (
                     SELECT id, parent, own_access, 0 FROM nodes WHERE id = $1
                     UNION ALL SELECT n.id, n.parent, n.own_access, up.depth + 1 FROM nodes n JOIN up ON n.id = up.parent)
                 SELECT id FROM up WHERE own ORDER BY depth LIMIT 1",
                &[&id],
            )
            .await
        else {
            return failed();
        };
        r.map(|r| r.get::<_, i64>(0)).filter(|own| *own != id)
    } else {
        None
    };
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let done = async {
        tx.execute(
            "WITH RECURSIVE top AS (SELECT trashed FROM nodes WHERE id = $1),
               down(id) AS (SELECT $1::BIGINT UNION ALL SELECT n.id FROM nodes n JOIN down ON n.parent = down.id
                            WHERE n.trashed = (SELECT trashed FROM top))
             UPDATE nodes SET trashed = NULL, trashed_by = NULL,
               parent = CASE WHEN id = $1 AND NOT $2 THEN NULL ELSE parent END
             WHERE id IN (SELECT id FROM down)",
            &[&id, &parent_alive],
        )
        .await?;
        if let Some(from) = carry {
            tx.execute("DELETE FROM grants WHERE node = $1", &[&id]).await?;
            tx.execute(
                "INSERT INTO grants (node, user_id, group_id, everyone, role, custom)
                 SELECT $1, user_id, group_id, everyone, role, custom FROM grants WHERE node = $2",
                &[&id, &from],
            )
            .await?;
            tx.execute("UPDATE nodes SET own_access = TRUE WHERE id = $1", &[&id]).await?;
        }
        tx.commit().await
    }
    .await;
    match done {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

async fn node_access(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let own = c.query_one("SELECT own_access FROM nodes WHERE id = $1", &[&id]).await.is_ok_and(|r| r.get(0));
    match grants_on(&c, None, Some(id)).await {
        Ok(grants) => Json(Access { own, grants }).into_response(),
        Err(_) => failed(),
    }
}

/// Whether a file's details (`meta`) say it's a Draw canvas or a Calc book.
fn no_comments(meta: &str) -> bool {
    let d: serde_json::Value = serde_json::from_str(meta).unwrap_or_default();
    let ext = d["ext"].as_str().or_else(|| d["name"].as_str().and_then(|n| n.rsplit_once('.').map(|(_, e)| e))).unwrap_or("");
    matches!(d["app"].as_str(), Some("draw" | "calc")) || matches!(ext, "xdraw" | "xcalc")
}

async fn set_node_access(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(a): Json<Access>,
) -> Response {
    let (mut c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Manager) {
        return e;
    }
    let grants = if a.own { a.grants.as_slice() } else { &[] };
    // Draw and Calc have no comments: "can comment" would only be "can view".
    if grants.iter().any(|g| g.role == Role::Commenter && g.custom.is_none()) {
        let meta: Option<String> = c.query_opt("SELECT meta FROM nodes WHERE id = $1", &[&id]).await.ok().flatten().map(|r| r.get(0));
        if meta.as_deref().is_some_and(no_comments) {
            return err(StatusCode::BAD_REQUEST, "no_comments", "Draw and Calc files have no comments: share them to view or to edit");
        }
    }
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let done = async {
        write_grants(&tx, None, Some(id), grants).await?;
        tx.execute("UPDATE nodes SET own_access = $2 WHERE id = $1", &[&id, &a.own]).await?;
        tx.commit().await
    }
    .await;
    match done {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => err(StatusCode::BAD_REQUEST, "bad_grant", "someone or some group in the list doesn't exist"),
    }
}

async fn versions(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let rows = c
        .query(
            "SELECT n, size, to_char(created AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), author, label
             FROM versions WHERE node = $1 ORDER BY n DESC",
            &[&id],
        )
        .await;
    match rows {
        Ok(rows) => Json(
            rows.iter()
                .map(|r| Version { n: r.get(0), size: r.get(1), created: r.get(2), author: r.get(3), label: r.get(4) })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct Which {
    base: Option<i64>,
    version: Option<i64>,
    /// How many changes (from editing together) this version holds; by
    /// default all there are.
    upto: Option<i64>,
    /// Saved by the app as you type (not Save): it goes into the latest
    /// version when that's the same person's and under ten minutes old, so
    /// the history keeps checkpoints rather than every pause.
    #[serde(default)]
    auto: bool,
    /// A checkpoint: a version of its own with this description, kept
    /// however many are saved after it.
    label: Option<String>,
    /// The file as it's kept on a computer: a Draw canvas with its pictures
    /// and clips (an `.xdraw` zip), not the canvas alone.
    #[serde(default)]
    file: bool,
}

async fn save(
    State(s): State<Server>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Query(mut q): Query<Which>,
    body: Bytes,
) -> Response {
    let (mut c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    // A plan shared with a document role that allows only some changes: the
    // version is the plan as the server has it (the changes they made were
    // checked one by one), not what was sent, which could be anything.
    let body = if plan_lacks(&c, &me, id, &[]).await {
        let seq: i64 = match c.query_one("SELECT seq FROM nodes WHERE id = $1", &[&id]).await {
            Ok(r) => r.get(0),
            Err(_) => return failed(),
        };
        let upto = q.upto.unwrap_or(seq).clamp(0, seq);
        q.upto = Some(upto);
        q.file = false;
        match crate::plan::file_at(&s, id, upto).await {
            Ok(b) => Bytes::from(b),
            Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, "unreadable", &e),
        }
    } else {
        body
    };
    // Scanned for viruses first, when an admin turned that on.
    if let Err(e) = crate::scan::allow(&c, &format!("an upload to file {id} by {}", me.account.username), &[&body[..]]).await {
        return e;
    }
    // Within what this person may store (when an admin set a limit).
    if let Err(e) = crate::media::room_for(&c, me.account.id, body.len() as i64).await {
        return e;
    }
    // An `.xdraw` file (uploaded on the Files page): its pictures and clips
    // are kept apart, and the canvas is the version.
    let body = match c.query_opt("SELECT meta FROM nodes WHERE id = $1", &[&id]).await {
        Ok(Some(r)) if crate::media::is_draw(r.get(0)) => match crate::media::take_apart(&s, &c, id, me.account.id, &body).await {
            Ok(Some(doc)) => Bytes::from(doc),
            Ok(None) => body,
            Err(e) => return e,
        },
        _ => body,
    };
    let Ok(hash) = s.blobs.put(&body).await else { return failed() };
    let Ok(tx) = c.transaction().await else { return unavailable() };
    let Ok(Some(row)) = tx.query_opt("SELECT head, kind, seq FROM nodes WHERE id = $1 FOR UPDATE", &[&id]).await else {
        return not_found();
    };
    if row.get::<_, &str>(1) != "file" {
        return err(StatusCode::BAD_REQUEST, "not_file", "folders have no contents");
    }
    let head: i64 = row.get(0);
    if q.base.is_some_and(|b| b != head) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "someone saved a newer version", "code": "behind", "version": head })),
        )
            .into_response();
    }
    let size = body.len() as i64;
    let seq: i64 = row.get(2);
    let upto = q.upto.unwrap_or(seq).clamp(0, seq);
    // As you type: into the latest version while it's yours and recent; an
    // older copy than the latest (another window was quicker) changes nothing.
    let latest = if q.auto && head > 0 {
        tx.query_opt(
            "SELECT author, upto, created > now() - interval '10 minutes' AND label IS NULL FROM versions WHERE node = $1 AND n = $2",
            &[&id, &head],
        )
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    if let Some(l) = &latest
        && upto < l.get::<_, i64>(1)
    {
        return Json(Saved { version: head }).into_response();
    }
    let label = q.label.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.chars().take(200).collect::<String>());
    let fold = label.is_none() && latest.is_some_and(|l| l.get::<_, Option<i64>>(0) == Some(me.account.id) && l.get::<_, bool>(2));
    let n = if fold { head } else { head + 1 };
    let done = async {
        let mut pruned = Vec::new();
        if fold {
            tx.execute(
                "UPDATE versions SET blob = $3, size = $4, upto = $5 WHERE node = $1 AND n = $2",
                &[&id, &n, &hash, &size, &upto],
            )
            .await?;
        } else {
            tx.execute(
                "INSERT INTO versions (node, n, blob, size, author, upto, label) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&id, &n, &hash, &size, &me.account.id, &upto, &label],
            )
            .await?;
            // Only the newest versions saved as you type are kept; checkpoints stay.
            pruned = tx
                .query(
                    "DELETE FROM versions WHERE node = $1 AND label IS NULL AND n < $2 AND n NOT IN
                       (SELECT n FROM versions WHERE node = $1 AND label IS NULL ORDER BY n DESC LIMIT $3)
                     RETURNING blob",
                    &[&id, &n, &KEEP_AUTO],
                )
                .await?
                .iter()
                .map(|r| r.get::<_, String>(0))
                .collect();
        }
        tx.execute(
            "UPDATE nodes SET head = $2, size = $3, changed = now(), changed_by = $4 WHERE id = $1",
            &[&id, &n, &size, &me.account.id],
        )
        .await?;
        tx.commit().await.map(|()| pruned)
    }
    .await;
    match done {
        Ok(pruned) => {
            forget_blobs(&s, &c, pruned).await;
            // Its words, for searching, when the app doesn't send them (a
            // Write document uploaded on the Files page).
            if let Ok(Some(r)) = c.query_opt("SELECT meta FROM nodes WHERE id = $1", &[&id]).await
                && crate::pages::is_write(r.get(0))
                && let Some(words) = crate::pages::words(body.to_vec()).await
            {
                let _ = c.execute("UPDATE nodes SET search_text = $2 WHERE id = $1", &[&id, &words]).await;
            }
            Json(Saved { version: n }).into_response()
        }
        Err(_) => failed(),
    }
}

/// How many versions saved as you type a file keeps (besides its
/// checkpoints, which stay until someone deletes them).
const KEEP_AUTO: i64 = 20;

#[derive(Deserialize)]
struct Label {
    label: Option<String>,
}

/// Names version `n` (a checkpoint, kept), renames it, or with no name
/// makes it an ordinary version again. Editors.
async fn label_version(
    State(s): State<Server>,
    h: HeaderMap,
    Path((id, n)): Path<(i64, i64)>,
    Json(b): Json<Label>,
) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    let label = b.label.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.chars().take(200).collect::<String>());
    match c.execute("UPDATE versions SET label = $3 WHERE node = $1 AND n = $2", &[&id, &n, &label]).await {
        Ok(1) => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_found(),
        Err(_) => failed(),
    }
}

/// Deletes version `n`; never the current one. Managers.
async fn forget_version(State(s): State<Server>, h: HeaderMap, Path((id, n)): Path<(i64, i64)>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Manager) {
        return e;
    }
    let gone = c
        .query(
            "DELETE FROM versions v USING nodes n WHERE v.node = $1 AND v.n = $2 AND n.id = v.node AND v.n <> n.head
             RETURNING v.blob",
            &[&id, &n],
        )
        .await;
    match gone {
        Ok(rows) if rows.is_empty() => err(StatusCode::BAD_REQUEST, "current", "that's the current version (or it's gone already)"),
        Ok(rows) => {
            forget_blobs(&s, &c, rows.iter().map(|r| r.get(0)).collect()).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct Forget {
    /// Checkpoints go too (all but the current version).
    #[serde(default)]
    checkpoints: bool,
}

/// Deletes every version but the current one; checkpoints too only when
/// asked. Managers. Answers how many went.
async fn forget_versions(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Query(q): Query<Forget>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Manager) {
        return e;
    }
    let gone = c
        .query(
            "DELETE FROM versions v USING nodes n WHERE v.node = $1 AND n.id = v.node AND v.n <> n.head
               AND ($2 OR v.label IS NULL)
             RETURNING v.blob",
            &[&id, &q.checkpoints],
        )
        .await;
    match gone {
        Ok(rows) => {
            let count = rows.len();
            forget_blobs(&s, &c, rows.iter().map(|r| r.get(0)).collect()).await;
            Json(json!({ "deleted": count })).into_response()
        }
        Err(_) => failed(),
    }
}

/// Removes the bytes of deleted versions that nothing else uses (another
/// version, a picture, a clip or a thumbnail can hold the same bytes).
async fn forget_blobs(s: &Server, c: &Client, mut hashes: Vec<String>) {
    hashes.sort();
    hashes.dedup();
    for hash in hashes {
        let used = c
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM versions WHERE blob = $1)
                     OR EXISTS (SELECT 1 FROM node_pictures WHERE blob = $1)
                     OR EXISTS (SELECT 1 FROM node_media WHERE hash = $1)
                     OR EXISTS (SELECT 1 FROM nodes WHERE preview = $1)",
                &[&hash],
            )
            .await
            .map(|r| r.get::<_, bool>(0))
            .unwrap_or(true);
        if !used && let Some(path) = s.blobs.file(&hash) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

async fn content(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Query(q): Query<Which>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let row = c
        .query_opt(
            "SELECT v.blob, v.n, n.meta FROM versions v JOIN nodes n ON n.id = v.node
             WHERE v.node = $1 AND v.n = COALESCE($2, n.head)",
            &[&id, &q.version],
        )
        .await;
    let Ok(Some(row)) = row else {
        return err(StatusCode::NOT_FOUND, "empty", "nothing has been saved in this file yet");
    };
    let Ok(bytes) = s.blobs.get(row.get(0)).await else { return failed() };
    if q.file && crate::media::is_draw(row.get(2)) {
        return attachment(crate::media::put_together(&s, &c, id, bytes).await);
    }
    let n: i64 = row.get(1);
    attachment(
        ([(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::ETAG, format!("\"{n}\""))], bytes)
            .into_response(),
    )
}

/// Bytes people put on the server are only ever downloaded, never shown as
/// a page of this site (where a script in them would run as the server's).
pub(crate) fn attachment(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CONTENT_DISPOSITION, header::HeaderValue::from_static("attachment"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
    r
}

/// A picture's name inside a document ("media/…").
fn picture_name(name: &str) -> Option<String> {
    let name = format!("media/{name}");
    (name.len() <= 200 && !name.contains("..") && !name.contains('\\')).then_some(name)
}

/// A picture put in a document while editing together (`media/…` without
/// "media/"): part of the document now, without a version of its own.
async fn put_picture(
    State(s): State<Server>,
    h: HeaderMap,
    Path((id, name)): Path<(i64, String)>,
    body: Bytes,
) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    // (Plans have no pictures: this would only write over the file's parts.)
    if plan_lacks(&c, &me, id, &[]).await {
        return plan_refused();
    }
    let Some(name) = picture_name(&name) else { return err(StatusCode::BAD_REQUEST, "bad_name", "not a picture's name") };
    if let Err(e) = crate::media::room_for(&c, me.account.id, body.len() as i64).await {
        return e;
    }
    // Scanned for viruses first, when an admin turned that on.
    if let Err(e) = crate::scan::allow(&c, &format!("an upload to file {id} by {}", me.account.username), &[&body[..]]).await {
        return e;
    }
    let Ok(hash) = s.blobs.put(&body).await else { return failed() };
    match c
        .execute(
            "INSERT INTO node_pictures (node, name, blob) VALUES ($1, $2, $3)
             ON CONFLICT (node, name) DO UPDATE SET blob = EXCLUDED.blob",
            &[&id, &name, &hash],
        )
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// A picture put in a document (see put_picture); else the one in its
/// latest version.
async fn picture(State(s): State<Server>, h: HeaderMap, Path((id, name)): Path<(i64, String)>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let Some(name) = picture_name(&name) else { return not_found() };
    let row = c.query_opt("SELECT blob FROM node_pictures WHERE node = $1 AND name = $2", &[&id, &name]).await;
    let Ok(Some(row)) = row else {
        return err(StatusCode::NOT_FOUND, "no_picture", "that picture isn't in this document");
    };
    let Ok(bytes) = s.blobs.get(row.get(0)).await else { return failed() };
    attachment(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

#[derive(Deserialize)]
struct AtVersion {
    /// An older version (Files › Version history › Open).
    version: Option<i64>,
}

#[derive(Deserialize)]
struct AsNew {
    /// The address the apps reach this server at (they follow the template
    /// there for its later changes).
    origin: String,
}

/// A new Write document from document `id` (a template), as Write makes one:
/// its parts and styles, its {{fields}} as boxes to fill in, an empty title
/// and a link back so later changes to the template come in. The bytes of a
/// Write file, to save as a new document's first version.
async fn as_new(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Query(q): Query<AsNew>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let Ok(Some(doc)) = crate::pages::latest(&s, &c, id).await else {
        return err(StatusCode::NOT_FOUND, "no_template", "only a Write document with something in it can be a template");
    };
    let name: String = match c.query_opt("SELECT meta FROM nodes WHERE id = $1", &[&id]).await {
        Ok(Some(r)) => serde_json::from_str::<Value>(&r.get::<_, String>(0))
            .ok()
            .and_then(|m| m["name"].as_str().map(|n| n.trim_end_matches(".xwrite").to_string()))
            .unwrap_or_else(|| "Template".into()),
        _ => "Template".into(),
    };
    let from = format!("server:{}|{id}", q.origin.trim_end_matches('/'));
    let out = tokio::task::spawn_blocking(move || {
        let mut d = xenify_wp_doc::linked::new_document(&doc, &name, &from);
        xenify_wp_doc::controls::fill_ins(&mut d);
        d.meta.title.clear();
        // (Write's own file directly: save() would bring in the PDF and Word paths, and with them the
        // system's fonts, a library a server may not have.)
        Ok::<_, String>(xenify_wp_io::save_igdoc(&d))
    })
    .await;
    match out {
        Ok(Ok(bytes)) => attachment(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response()),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, "unreadable", &e),
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct Between {
    from: i64,
    to: i64,
}

/// Two versions of a Write document compared: the newer one as a page, with
/// the words added since `from` marked as inserted and the ones taken out
/// put back struck through. `{ html, title, added, removed }`.
async fn compare(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Query(q): Query<Between>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let (Ok(Some(old)), Ok(Some(new))) =
        (crate::pages::at_version(&s, &c, id, q.from).await, crate::pages::at_version(&s, &c, id, q.to).await)
    else {
        return err(StatusCode::NOT_FOUND, "no_versions", "only versions of Write documents can be compared");
    };
    let label = format!("Changes since version {}", q.from);
    let out = tokio::task::spawn_blocking(move || {
        let (added, removed) = xenify_wp_doc::compare::word_changes(&old, &new);
        let marked = xenify_wp_doc::compare::compare(&old, &new, &label);
        json!({
            "html": xenify_wp_io::html::write(&xenify_wp_doc::controls::without_prompts(&marked)),
            "title": new.meta.title,
            "added": added,
            "removed": removed,
        })
    })
    .await;
    match out {
        Ok(v) => Json(v).into_response(),
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct BlameOf {
    version: i64,
}

/// Who wrote what: each paragraph of version `version` of a Write document,
/// with the version it has been there unchanged since, and who saved that
/// version. `{ paragraphs: [{ text, n, author, created }] }`. Looks back over
/// the latest 100 versions at most.
async fn blame(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Query(q): Query<BlameOf>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let Ok(rows) = c
        .query(
            "SELECT n, author, to_char(created AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
             FROM versions WHERE node = $1 AND n <= $2 ORDER BY n DESC LIMIT 100",
            &[&id, &q.version],
        )
        .await
    else {
        return failed();
    };
    // Oldest first: each version's paragraphs.
    let mut seen: Vec<(i64, Option<i64>, String, Vec<String>)> = Vec::new();
    for r in rows.iter().rev() {
        let n: i64 = r.get(0);
        let Ok(Some(doc)) = crate::pages::at_version(&s, &c, id, n).await else { continue };
        seen.push((n, r.get(1), r.get(2), crate::pages::paragraphs(&doc)));
    }
    let Some((_, _, _, last)) = seen.last() else {
        return err(StatusCode::NOT_FOUND, "no_versions", "only Write documents show who wrote what");
    };
    // For each paragraph now: walk back while the version before had it too.
    let mut out = Vec::new();
    let mut used: Vec<std::collections::HashMap<&str, usize>> = Vec::new();
    for (_, _, _, ps) in &seen {
        let mut m = std::collections::HashMap::new();
        for p in ps {
            *m.entry(p.as_str()).or_insert(0) += 1;
        }
        used.push(m);
    }
    for text in last {
        let mut k = seen.len() - 1;
        while k > 0 && used[k - 1].contains_key(text.as_str()) {
            k -= 1;
        }
        let (n, author, created, _) = &seen[k];
        out.push(json!({ "text": text, "n": n, "author": author, "created": created }));
    }
    Json(json!({ "paragraphs": out })).into_response()
}

/// A Write document's latest version as a web page: `{ html, title }`
/// (the Files page's read view).
async fn page(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, Query(q): Query<AtVersion>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    match crate::pages::page_at(&s, &c, Some(&me), id, q.version).await {
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

/// Every folder and file in a space you may see (for searching names in
/// the app).
async fn everything(State(s): State<Server>, h: HeaderMap, Query(q): Query<ChildrenOf>) -> Response {
    let (c, me) = who!(s, h);
    let Ok(rows) = c
        .query(
            &format!("SELECT {NODE_COLS} FROM nodes n WHERE n.space = $1 AND n.trashed IS NULL ORDER BY n.id"),
            &[&q.space],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        if let Ok(Some((_, Some(role)))) = node_role(&c, &me, r.get(0)).await {
            out.push(node_of(&r, role));
        }
    }
    Json(out).into_response()
}

/// The most words a file's search copy keeps (bytes).
const MAX_TEXT: usize = 2 << 20;

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    /// Only in this space.
    #[serde(default)]
    space: Option<i64>,
}

/// Folders and files you may see whose name or words match `q`, best
/// first, each with a bit of the text around the match.
async fn search(State(s): State<Server>, h: HeaderMap, Query(q): Query<SearchQuery>) -> Response {
    let (c, me) = who!(s, h);
    let text = q.q.trim();
    if text.is_empty() {
        return Json(Vec::<Value>::new()).into_response();
    }
    // Every word, each as a prefix ("auto camp" finds "Autumn campaign").
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("{}:*", w.to_lowercase()))
        .collect();
    let like = format!("%{}%", text.to_lowercase().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let Ok(rows) = c
        .query(
            &format!(
                "SELECT {NODE_COLS},
                        ts_headline('simple', n.search_text, to_tsquery('simple', $1),
                                    'MaxWords=18, MinWords=8, StartSel=<<, StopSel=>>, MaxFragments=1')
                 FROM nodes n JOIN spaces s ON s.id = n.space
                 WHERE n.trashed IS NULL AND s.kind <> 'chat' AND ($3::BIGINT IS NULL OR n.space = $3)
                   AND (($1 <> '' AND n.search @@ to_tsquery('simple', $1)) OR lower(xenify_name(n.meta)) LIKE $2)
                 ORDER BY (lower(xenify_name(n.meta)) LIKE $2) DESC, ts_rank(n.search, to_tsquery('simple', $1)) DESC, n.changed DESC
                 LIMIT 300"
            ),
            &[&words.join(" & "), &like, &q.space],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        if let Ok(Some((_, Some(role)))) = node_role(&c, &me, r.get(0)).await {
            let mut v = serde_json::to_value(node_of(&r, role)).unwrap_or_default();
            let snippet: String = r.get(11);
            if snippet.contains("<<") {
                v["snippet"] = json!(snippet);
            }
            out.push(v);
            if out.len() == 50 {
                break;
            }
        }
    }
    Json(out).into_response()
}

// ---- AI search -----------------------------------------------------------
// Finding by meaning, in steps the page shows as it goes: the AI turns the
// question into words to search for (/words), the ordinary search finds
// files with them, and the AI reads those files a few at a time and keeps
// the ones that answer the question, saying why (/read).

#[derive(Deserialize)]
struct AiAsk {
    q: String,
    #[serde(default)]
    ids: Vec<i64>,
}

/// The first JSON array or object in an AI's answer (it may wrap it in words or ```).
fn json_in(text: &str, open: char, close: char) -> Option<Value> {
    let (a, b) = (text.find(open)?, text.rfind(close)?);
    (a < b).then(|| serde_json::from_str(&text[a..=b]).ok()).flatten()
}

async fn ask_ai(c: &Client, system: &str, user: String) -> Result<String, Response> {
    let messages = vec![json!({ "role": "system", "content": system }), json!({ "role": "user", "content": user })];
    match crate::ai::complete(c, messages).await {
        None => Err(err(StatusCode::NOT_FOUND, "no_ai", "this server has no AI set up (an admin can add one on the admin pages, AI)")),
        Some(Err(e)) => Err(err(StatusCode::BAD_GATEWAY, "ai_failed", &e)),
        Some(Ok(t)) => Ok(t),
    }
}

/// Words and short phrases to search for that would find what the question
/// is after (the question's own words too, and others that mean the same).
async fn ai_words(State(s): State<Server>, h: HeaderMap, Json(a): Json<AiAsk>) -> Response {
    let (c, _me) = who!(s, h);
    let q: String = a.q.trim().chars().take(500).collect();
    if q.is_empty() {
        return Json(json!({ "terms": [] })).into_response();
    }
    let system = "You help search a team's documents by keyword. Reply with only a JSON array of 10 to 16 single words that a document about what the person is looking for would contain. Cover: (1) the key words of the question itself; (2) other words for each of them, verbs included (\"set up\" -> install, configure, setup; \"fix\" -> repair, mend); (3) the specific kinds of each thing (\"bird\" -> chicken, hen, poultry, parrot, duck). Different meanings of the question count too. Single words only, no phrases, no explanations.";
    let text = match ask_ai(&c, system, q.clone()).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let mut terms: Vec<String> = json_in(&text, '[', ']')
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t.as_str().map(|t| t.trim().chars().take(60).collect::<String>()))
        .filter(|t| !t.is_empty())
        .collect();
    // The question's own words always count (and are all there is if the AI said nothing useful).
    if terms.is_empty() {
        terms.push(q.chars().take(60).collect());
    }
    terms.dedup();
    terms.truncate(16);
    Json(json!({ "terms": terms })).into_response()
}

/// Reads files `ids` (a few at a time; only ones you may open) and answers
/// which of them have what the question is after, with a line on why.
async fn ai_read(State(s): State<Server>, h: HeaderMap, Json(a): Json<AiAsk>) -> Response {
    let (c, me) = who!(s, h);
    let q: String = a.q.trim().chars().take(500).collect();
    let mut docs = Vec::new();
    for id in a.ids.iter().take(10) {
        if !matches!(node_role(&c, &me, *id).await, Ok(Some((_, Some(_))))) {
            continue;
        }
        let Ok(Some(r)) = c.query_opt("SELECT xenify_name(meta), COALESCE(search_text, '') FROM nodes WHERE id = $1 AND trashed IS NULL", &[id]).await
        else {
            continue;
        };
        let (name, text): (String, String) = (r.get(0), r.get(1));
        let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(3000).collect();
        docs.push((*id, name, text));
    }
    if q.is_empty() || docs.is_empty() {
        return Json(json!({ "found": [] })).into_response();
    }
    let mut user = format!("Looking for: {q}\n\nDocuments:\n");
    for (i, (_, name, text)) in docs.iter().enumerate() {
        user.push_str(&format!("\n[{}] {name}\n{text}\n", i + 1));
    }
    let system = "You help search a team's documents. Read the documents and decide which ones contain what the person is looking for (by meaning, not only the same words). Reply with only a JSON array of objects {\"n\": <document number>, \"why\": \"<one short sentence quoting or describing the part that matches>\"}, best match first. Leave out documents that don't match; reply [] if none do.";
    let text = match ask_ai(&c, system, user).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let found: Vec<Value> = json_in(&text, '[', ']')
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|f| {
            let n = f["n"].as_u64()? as usize;
            let (id, _, _) = docs.get(n.checked_sub(1)?)?;
            Some(json!({ "id": id, "why": f["why"].as_str().unwrap_or_default().chars().take(300).collect::<String>() }))
        })
        .collect();
    Json(json!({ "found": found })).into_response()
}

/// The words of a file as plain text, for searching (the apps send it when
/// they save).
async fn put_text(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, body: String) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    // A plan shared with a document role that allows only some changes: the
    // app sends its words after saving, but they're the plan's to set
    // (saving keeps the server's copy), so they're left as they are.
    if plan_lacks(&c, &me, id, &[]).await {
        return StatusCode::NO_CONTENT.into_response();
    }
    match c.execute("UPDATE nodes SET search_text = $2 WHERE id = $1", &[&id, &body]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// Files changed lately that you may open.
async fn recent(State(s): State<Server>, h: HeaderMap) -> Response {
    let (c, me) = who!(s, h);
    let Ok(rows) = c
        .query(
            &format!(
                "SELECT {NODE_COLS} FROM nodes n JOIN spaces s ON s.id = n.space
                 WHERE n.kind = 'file' AND n.trashed IS NULL AND s.kind <> 'chat' AND (s.kind <> 'personal' OR s.owner = $1 OR n.own_access
                       OR EXISTS (SELECT 1 FROM nodes p WHERE p.space = n.space AND p.own_access))
                 ORDER BY n.changed DESC LIMIT 200"
            ),
            &[&me.account.id],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        if let Ok(Some((_, Some(role)))) = node_role(&c, &me, r.get(0)).await {
            out.push(node_of(&r, role));
            if out.len() == 50 {
                break;
            }
        }
    }
    Json(out).into_response()
}

/// Folders and files with their own list that names you or your groups,
/// outside spaces you already have.
async fn shared(State(s): State<Server>, h: HeaderMap) -> Response {
    let (c, me) = who!(s, h);
    let Ok(rows) = c
        .query(
            &format!(
                "SELECT DISTINCT {NODE_COLS} FROM nodes n JOIN grants g ON g.node = n.id
                 WHERE n.own_access AND n.trashed IS NULL AND (g.user_id = $1 OR g.group_id = ANY($2) OR g.everyone)"
            ),
            &[&me.account.id, &me.groups],
        )
        .await
    else {
        return failed();
    };
    let mut out = Vec::new();
    for r in rows {
        let space: i64 = r.get(1);
        let whole = match space_row(&c, space).await {
            Ok(Some(row)) => space_role(&c, &me, &row).await.ok().flatten(),
            _ => None,
        };
        if whole.is_some() {
            continue;
        }
        if let Ok(Some((_, Some(role)))) = node_role(&c, &me, r.get(0)).await {
            out.push(node_of(&r, role));
        }
    }
    Json(out).into_response()
}

/// Names of people and groups, for "share with" lists.
async fn directory(State(s): State<Server>, h: HeaderMap) -> Response {
    let (c, _) = who!(s, h);
    let people =
        c.query("SELECT id, username, display_name, email FROM users WHERE NOT app AND NOT disabled ORDER BY id", &[])
            .await;
    let groups = c.query("SELECT id, name FROM groups ORDER BY id", &[]).await;
    let (Ok(people), Ok(groups)) = (people, groups) else { return failed() };
    Json(serde_json::json!({
        "people": people.iter().map(|r| serde_json::json!({
            "id": r.get::<_, i64>(0), "username": r.get::<_, String>(1), "display_name": r.get::<_, String>(2),
            "email": r.get::<_, String>(3)
        })).collect::<Vec<_>>(),
        "groups": groups.iter().map(|r| serde_json::json!({ "id": r.get::<_, i64>(0), "name": r.get::<_, String>(1) })).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// Keeps a picture of the file.
async fn save_preview(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>, body: Bytes) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Editor) {
        return e;
    }
    // Scanned for viruses first, when an admin turned that on.
    if let Err(e) = crate::scan::allow(&c, &format!("an upload to file {id} by {}", me.account.username), &[&body[..]]).await {
        return e;
    }
    let Ok(hash) = s.blobs.put(&body).await else { return failed() };
    match c
        .execute("UPDATE nodes SET preview = $2, preview_at = now() WHERE id = $1 AND kind = 'file'", &[&id, &hash])
        .await
    {
        Ok(1) => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_found(),
        Err(_) => failed(),
    }
}

async fn preview(State(s): State<Server>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    let (c, me) = who!(s, h);
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    if let Err(e) = need(role, Role::Viewer) {
        return e;
    }
    let Ok(Some(row)) = c
        .query_opt(
            "SELECT preview, to_char(preview_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') FROM nodes WHERE id = $1",
            &[&id],
        )
        .await
    else {
        return not_found();
    };
    let Some(hash) = row.get::<_, Option<String>>(0) else {
        return err(
            StatusCode::NOT_FOUND,
            "no_preview",
            "there's no picture of this file yet: it's made when it's saved",
        );
    };
    let Ok(bytes) = s.blobs.get(&hash).await else { return failed() };
    let at: String = row.get(1);
    attachment(
        ([(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::ETAG, format!("\"{at}\""))], bytes)
            .into_response(),
    )
}
