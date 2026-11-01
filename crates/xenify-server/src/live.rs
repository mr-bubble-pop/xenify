//! Editing together: `/v1/live`, a WebSocket per app window.
//!
//! Changes are put in order here (a row lock on the file) and kept; who has
//! a document open is kept in memory. Every server hears every change
//! through PostgreSQL notifications and passes it to the windows connected
//! to it, so several servers can share one database.

use crate::files::{Me, me, node_role};
use crate::{Server, hash_token};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, mpsc};
use xenify_server_proto::{FromLive, Here, Op, Role, ToLive};

/// The NOTIFY channel servers tell each other on.
pub const CHANNEL: &str = "xenify_live";
/// The most changes in one batch, and the largest change: a picture put
/// in a Draw canvas is one change with the whole picture in it.
pub const MAX_OPS: usize = 1000;
pub const MAX_OP: usize = 40 << 20;
/// The largest message a window may send (a batch, as JSON).
const MAX_MESSAGE: usize = 128 << 20;
/// What joining hands back at most; more means the app should open the
/// latest saved version first.
pub const MAX_CATCH_UP: i64 = 20_000;
/// How long what a window may do is trusted before Send or Lease checks it
/// again (sign-in, role); changes to access also tell windows at once (`check`).
const RECHECK: Duration = Duration::from_secs(5);
/// Every open window checks its sign-in and access this often anyway.
const RECHECK_ALL: Duration = Duration::from_secs(60);

struct Window {
    here: Here,
    tx: mpsc::UnboundedSender<FromLive>,
    /// Tells the window to check its sign-in and access again.
    poke: Arc<Notify>,
}

/// Who has which document open, on this server.
#[derive(Default)]
pub struct Rooms {
    rooms: Mutex<HashMap<i64, Vec<Window>>>,
}

impl Rooms {
    fn tell(&self, node: i64, skip: Option<&str>, m: &FromLive) {
        let mut rooms = self.rooms.lock().unwrap();
        if let Some(list) = rooms.get_mut(&node) {
            list.retain(|w| skip == Some(w.here.conn.as_str()) || w.tx.send(m.clone()).is_ok());
        }
    }

    /// Every window open here: (document, who, its poke).
    fn windows(&self) -> Vec<(i64, i64, Arc<Notify>)> {
        let rooms = self.rooms.lock().unwrap();
        rooms.iter().flat_map(|(node, l)| l.iter().map(|w| (*node, w.here.user, w.poke.clone()))).collect()
    }

    fn people(&self, node: i64) -> Vec<Here> {
        self.rooms.lock().unwrap().get(&node).map(|l| l.iter().map(|w| w.here.clone()).collect()).unwrap_or_default()
    }
}

/// What servers tell each other.
#[derive(Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
enum Note {
    /// Changes `from..=to` are in; `conn` sent them (and was told already).
    Ops {
        node: i64,
        from: i64,
        to: i64,
        conn: String,
    },
    Said {
        node: i64,
        from: Here,
        data: String,
    },
    Arrived {
        node: i64,
        who: Here,
    },
    Left {
        node: i64,
        conn: String,
    },
    /// Access may have changed: windows with that document (or one inside
    /// that folder, or in that space) or of that person check theirs again.
    /// Nothing given: every window. Also sent by the database (perms.rs).
    Check {
        #[serde(default)]
        node: Option<i64>,
        #[serde(default)]
        space: Option<i64>,
        #[serde(default)]
        user: Option<i64>,
    },
}

/// Tells every server that access to `node` (and what's inside it) may have
/// changed: its windows check again at once.
pub async fn check_node(s: &Server, node: i64) {
    s.note(&Note::Check { node: Some(node), space: None, user: None }).await;
}

/// The same for a whole space.
pub async fn check_space(s: &Server, space: i64) {
    s.note(&Note::Check { node: None, space: Some(space), user: None }).await;
}

/// The same for everything person `user` has open.
pub async fn check_user(s: &Server, user: i64) {
    s.note(&Note::Check { node: None, space: None, user: Some(user) }).await;
}

impl Server {
    /// Hears what every server did, and tells the windows connected here.
    pub(crate) async fn listen_live(&self, database: &str) -> Result<(), String> {
        let cfg: tokio_postgres::Config = database.parse().map_err(|e: tokio_postgres::Error| e.to_string())?;
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
        client.batch_execute(&format!("LISTEN {CHANNEL}")).await.map_err(|e| e.to_string())?;
        let s = self.clone();
        tokio::spawn(async move {
            let _keep = client;
            while let Some(payload) = rx.recv().await {
                if let Ok(note) = serde_json::from_str::<Note>(&payload) {
                    s.fan_out(note).await;
                }
            }
        });
        Ok(())
    }

    async fn fan_out(&self, note: Note) {
        match note {
            Note::Ops { node, from, to, conn } => {
                if self.rooms.people(node).is_empty() {
                    return;
                }
                let Ok(c) = self.pool.get().await else { return };
                if let Ok(ops) = ops_between(&c, node, from - 1, to).await {
                    self.rooms.tell(node, Some(&conn), &FromLive::Ops { node, ops });
                }
            }
            Note::Said { node, from, data } => {
                let skip = from.conn.clone();
                self.rooms.tell(node, Some(&skip), &FromLive::Said { node, from, data });
            }
            Note::Arrived { node, who } => {
                let skip = who.conn.clone();
                self.rooms.tell(node, Some(&skip), &FromLive::Arrived { node, who });
            }
            Note::Left { node, conn } => {
                self.rooms.tell(node, Some(&conn), &FromLive::Left { node, conn: conn.clone() })
            }
            Note::Check { node, space, user } => {
                let windows: Vec<_> =
                    self.rooms.windows().into_iter().filter(|w| user.is_none_or(|u| u == w.1)).collect();
                if windows.is_empty() {
                    return;
                }
                let open: Vec<i64> = windows.iter().map(|w| w.0).collect();
                // The documents open here that it's about.
                let hit: Option<Vec<i64>> = match (node, space) {
                    (Some(n), _) => {
                        let Ok(c) = self.pool.get().await else { return };
                        c.query(
                            "WITH RECURSIVE down(id) AS (SELECT $1::BIGINT UNION ALL SELECT n.id FROM nodes n JOIN down ON n.parent = down.id)
                             SELECT id FROM down WHERE id = ANY($2)",
                            &[&n, &open],
                        )
                        .await
                        .ok()
                        .map(|r| r.iter().map(|x| x.get(0)).collect())
                    }
                    (None, Some(sp)) => {
                        let Ok(c) = self.pool.get().await else { return };
                        c.query("SELECT id FROM nodes WHERE space = $1 AND id = ANY($2)", &[&sp, &open])
                            .await
                            .ok()
                            .map(|r| r.iter().map(|x| x.get(0)).collect())
                    }
                    // Can't tell (or about a person): all of theirs.
                    _ => None,
                };
                for (n, _, poke) in windows {
                    if hit.as_ref().is_none_or(|h| h.contains(&n)) {
                        poke.notify_one();
                    }
                }
            }
        }
    }

    async fn note(&self, note: &Note) {
        let Ok(payload) = serde_json::to_string(note) else { return };
        // NOTIFY carries under 8000 bytes; a caret position never needs more.
        if payload.len() > 7900 {
            return;
        }
        if let Ok(c) = self.pool.get().await {
            let _ = c.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &payload]).await;
        }
    }
}

async fn ops_between(
    c: &deadpool_postgres::Client,
    node: i64,
    after: i64,
    upto: i64,
) -> Result<Vec<Op>, tokio_postgres::Error> {
    let rows = c
        .query(
            "SELECT seq, COALESCE(author, 0), client, data FROM ops WHERE node = $1 AND seq > $2 AND seq <= $3 ORDER BY seq",
            &[&node, &after, &upto],
        )
        .await?;
    Ok(rows.iter().map(|r| Op { seq: r.get(0), author: r.get(1), client: r.get(2), data: r.get(3) }).collect())
}

pub async fn live(State(s): State<Server>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_MESSAGE).on_upgrade(move |socket| window(s, socket))
}

fn error(code: &str, msg: &str) -> FromLive {
    FromLive::Error { code: code.into(), error: msg.into() }
}

async fn window(s: Server, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let send = |m: &FromLive| Message::Text(serde_json::to_string(m).unwrap_or_default().into());

    // The first message signs in.
    let hello = tokio::time::timeout(std::time::Duration::from_secs(15), stream.next()).await;
    let token = match hello {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str::<ToLive>(&t) {
            Ok(ToLive::Hello { token }) => token,
            _ => return,
        },
        _ => return,
    };
    let token_hash = hash_token(&token);
    drop(token);
    let account = match s.pool.get().await {
        Ok(c) => crate::db::session_account(&c, &token_hash).await.ok().flatten(),
        Err(_) => None,
    };
    let Some(mut account) = account else {
        let _ = sink.send(send(&error("signed_out", "please sign in again"))).await;
        return;
    };
    let conn = crate::random(9);
    let here = Here {
        conn: conn.clone(),
        user: account.id,
        name: if account.display_name.is_empty() { account.username.clone() } else { account.display_name.clone() },
    };
    let _ = sink
        .send(send(&FromLive::Welcome {
            protocol: xenify_server_proto::VERSION,
            account: account.clone(),
            conn: conn.clone(),
        }))
        .await;

    let (tx, mut rx) = mpsc::unbounded_channel::<FromLive>();
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(send(&m)).await.is_err() {
                break;
            }
        }
    });

    let mut open: HashMap<i64, Opened> = HashMap::new();
    let mut stopping = s.stopping();
    let poke = Arc::new(Notify::new());
    let mut every = tokio::time::interval_at(tokio::time::Instant::now() + RECHECK_ALL, RECHECK_ALL);
    let ctx = Ctx { token_hash, here: here.clone(), tx: tx.clone(), poke: poke.clone() };
    loop {
        let msg = tokio::select! {
            m = stream.next() => Some(m),
            _ = stopping.wait_for(|stop| *stop) => {
                // An update is taking over: reconnect (to the new copy).
                let _ = tx.send(error("restarting", "the server is updating: reconnecting"));
                break;
            }
            // Access changed somewhere (or it's time): still signed in, still allowed?
            _ = poke.notified() => None,
            _ = every.tick() => None,
        };
        let Some(msg) = msg else {
            if recheck(&s, &ctx, &mut account, &mut open, None).await {
                continue;
            }
            break;
        };
        let Some(Ok(msg)) = msg else { break };
        let Message::Text(text) = msg else {
            if matches!(msg, Message::Close(_)) {
                break;
            }
            continue;
        };
        let Ok(m) = serde_json::from_str::<ToLive>(&text) else {
            let _ = tx.send(error("bad_message", "the server didn't understand that"));
            continue;
        };
        // Changing a document: what was allowed a moment ago, checked again;
        // opening one, still signed in (its access is worked out anyway).
        let stale = match &m {
            ToLive::Send { node, .. } | ToLive::Lease { node, .. } => {
                open.get(node).is_some_and(|o| o.checked.elapsed() >= RECHECK).then_some(*node)
            }
            ToLive::Join { node, .. } => Some(*node),
            _ => None,
        };
        if let Some(node) = stale
            && !recheck(&s, &ctx, &mut account, &mut open, Some(node)).await
        {
            break;
        }
        let answer = handle(&s, &account, &ctx, &mut open, m).await;
        if let Some(a) = answer {
            let _ = tx.send(a);
        }
    }
    for node in open.keys() {
        leave(&s, *node, &conn).await;
    }
    drop(ctx);
    drop(tx);
    let _ = writer.await;
}

/// What a window's messages are handled with.
struct Ctx {
    /// Its sign-in, checked again now and then.
    token_hash: String,
    here: Here,
    tx: mpsc::UnboundedSender<FromLive>,
    poke: Arc<Notify>,
}

/// Whether the window's sign-in still holds (false: it's told and closes),
/// and what it may do with each document open (or only `only`): one it may
/// no longer open is closed, so it hears nothing more of it.
async fn recheck(
    s: &Server,
    ctx: &Ctx,
    account: &mut xenify_server_proto::Account,
    open: &mut HashMap<i64, Opened>,
    only: Option<i64>,
) -> bool {
    // The database can't be reached: try again later, as before.
    let Ok(c) = s.pool.get().await else { return true };
    let row = c
        .query_opt(
            &format!(
                "SELECT u.id, u.username, u.display_name, u.admin, u.email FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = $1 AND NOT u.disabled AND s.last_seen > now() - interval '{}'",
                crate::db::SESSION_IDLE
            ),
            &[&ctx.token_hash],
        )
        .await;
    match row {
        Ok(Some(r)) => {
            *account = xenify_server_proto::Account {
                id: r.get(0),
                username: r.get(1),
                display_name: r.get(2),
                admin: r.get(3),
                email: r.get(4),
            }
        }
        Ok(None) => {
            let _ = ctx.tx.send(error("signed_out", "please sign in again"));
            return false;
        }
        Err(_) => return true,
    }
    let Ok(me) = me(&c, account.clone()).await else { return true };
    let nodes: Vec<i64> = match only {
        Some(n) => vec![n],
        None => open.keys().copied().collect(),
    };
    for node in nodes {
        let Some(o) = open.get_mut(&node) else { continue };
        let Ok(found) = node_role(&c, &me, node).await else { continue };
        match found.and_then(|(_, r)| r) {
            Some(role) => {
                o.role = role;
                if o.plan.is_some() {
                    o.plan = Some(crate::files::plan_perms(&c, &me, node).await);
                }
                o.checked = Instant::now();
            }
            None => {
                open.remove(&node);
                leave(s, node, &ctx.here.conn).await;
                let _ = ctx.tx.send(error("no_access", "you can't open this document any more"));
            }
        }
    }
    true
}

async fn leave(s: &Server, node: i64, conn: &str) {
    {
        let mut rooms = s.rooms.rooms.lock().unwrap();
        if let Some(list) = rooms.get_mut(&node) {
            list.retain(|w| w.here.conn != conn);
            if list.is_empty() {
                rooms.remove(&node);
            }
        }
    }
    s.note(&Note::Left { node, conn: conn.into() }).await;
}

/// A document someone has open: their role, and for a plan the changes
/// they may make (files::plan_perms), which alone decide there.
pub struct Opened {
    role: Role,
    plan: Option<Vec<String>>,
    /// A Write document: someone who may comment sends comments (checked).
    write: bool,
    /// When `role` and `plan` were last worked out.
    checked: Instant,
}

impl Opened {
    fn may_send(&self) -> bool {
        match &self.plan {
            Some(perms) => !perms.is_empty(),
            None => self.role >= Role::Editor || self.comments_only(),
        }
    }

    /// May send changes, but only comments (each change checked).
    fn comments_only(&self) -> bool {
        self.plan.is_none() && self.write && self.role == Role::Commenter
    }
}

/// The first change in `ops` (Plan steps) that `perms` don't allow: what it needs.
fn plan_refused(ops: &[String], perms: &[String]) -> Option<&'static str> {
    for data in ops {
        let Ok(step) = serde_json::from_str::<xenify_plan_core::Step>(data) else { return Some("plan.setup") };
        if let Some(p) = xenify_plan_core::access::refused(&step.ops, perms) {
            return Some(p);
        }
    }
    None
}

async fn handle(
    s: &Server,
    account: &xenify_server_proto::Account,
    ctx: &Ctx,
    open: &mut HashMap<i64, Opened>,
    m: ToLive,
) -> Option<FromLive> {
    let (here, tx) = (&ctx.here, &ctx.tx);
    let Ok(mut c) = s.pool.get().await else { return Some(error("database", "the server can't reach its database")) };
    match m {
        ToLive::Hello { .. } => None,
        ToLive::Ping => Some(FromLive::Pong),
        ToLive::Join { node, since } => {
            let Ok(me) = me(&c, account.clone()).await else { return Some(error("failed", "something went wrong")) };
            let role = role_of(&c, &me, node).await;
            let Some(role) = role else { return Some(error("not_found", "it isn't there, or you can't open it")) };
            let row = c
                .query_opt(
                    "SELECT n.seq, COALESCE(v.n, 0), COALESCE(v.upto, 0) FROM nodes n
                     LEFT JOIN LATERAL (SELECT n, upto FROM versions WHERE node = n.id ORDER BY n DESC LIMIT 1) v ON TRUE
                     WHERE n.id = $1 AND n.kind = 'file'",
                    &[&node],
                )
                .await
                .ok()
                .flatten();
            let Some(row) = row else { return Some(error("not_file", "only files can be opened")) };
            let (seq, version, upto): (i64, i64, i64) = (row.get(0), row.get(1), row.get(2));
            let after = since.max(upto);
            let ops = if seq - after > MAX_CATCH_UP {
                Vec::new()
            } else {
                ops_between(&c, node, after, seq).await.unwrap_or_default()
            };
            let people: Vec<Here> = s.rooms.people(node).into_iter().filter(|p| p.conn != here.conn).collect();
            {
                // Joining again (no Leave between) replaces this window's place.
                let mut rooms = s.rooms.rooms.lock().unwrap();
                let list = rooms.entry(node).or_default();
                list.retain(|w| w.here.conn != here.conn);
                list.push(Window { here: here.clone(), tx: tx.clone(), poke: ctx.poke.clone() });
            }
            // Whether it's a plan was settled when it was made (plan.rs): an
            // editor changing its details can't stop its changes being checked.
            let is_plan = crate::plan::is_plan(&c, node).await;
            let plan = if is_plan { Some(crate::files::plan_perms(&c, &me, node).await) } else { None };
            let may = plan.clone();
            let write = c
                .query_opt("SELECT meta FROM nodes WHERE id = $1", &[&node])
                .await
                .ok()
                .flatten()
                .is_some_and(|r| crate::pages::is_write(r.get(0)));
            open.insert(node, Opened { role, plan, write, checked: Instant::now() });
            s.note(&Note::Arrived { node, who: here.clone() }).await;
            Some(FromLive::Joined { node, role, seq, version, upto, ops, people, may })
        }
        ToLive::Leave { node } => {
            if open.remove(&node).is_some() {
                leave(s, node, &here.conn).await;
            }
            None
        }
        ToLive::Send { node, base, client, ops } => {
            let comments_only = open.get(&node).is_some_and(Opened::comments_only);
            match open.get(&node) {
                Some(o) if o.may_send() => {
                    if let Some(perms) = &o.plan
                        && let Some(need) = plan_refused(&ops, perms)
                    {
                        return Some(error("not_allowed", &xenify_plan_core::access::message(need)));
                    }
                    // Comments are only their author's, whatever else one may do
                    // (who may edit the plan may delete anyone's).
                    if let Some(perms) = &o.plan
                        && crate::plan::comments_refused(
                            s,
                            node,
                            base,
                            &ops,
                            &format!("u{}", account.id),
                            perms.iter().any(|p| p == "plan.edit"),
                        )
                        .await
                    {
                        return Some(error("not_allowed", xenify_plan_core::access::NOT_YOURS));
                    }
                }
                Some(_) => return Some(error("not_allowed", "you can only read this document")),
                None => return Some(error("not_open", "open the document first")),
            }
            if ops.is_empty() || ops.len() > MAX_OPS {
                return Some(error("too_big", "too many changes at once"));
            }
            if ops.iter().any(|o| o.len() > MAX_OP) {
                return Some(error("too_big", &format!("a change is too big to send (over {} MB): a smaller picture fits", MAX_OP >> 20)));
            }
            // A batch sent again after reconnecting, that was taken already.
            if let Ok(Some(r)) = c
                .query_opt(
                    "SELECT min(seq), max(seq) FROM ops WHERE node = $1 AND client = $2 AND author = $3 HAVING count(*) > 0",
                    &[&node, &client, &account.id],
                )
                .await
            {
                return Some(FromLive::Accepted { node, client, from: r.get(0), to: r.get(1) });
            }
            let Ok(t) = c.transaction().await else {
                return Some(error("database", "the server can't reach its database"));
            };
            let Ok(row) = t.query_one("SELECT seq FROM nodes WHERE id = $1 FOR UPDATE", &[&node]).await else {
                return Some(error("failed", "something went wrong"));
            };
            let seq: i64 = row.get(0);
            if base != seq {
                drop(t);
                // As joining: never more than MAX_CATCH_UP changes at once.
                let after = base.clamp(0, seq);
                if seq - after > MAX_CATCH_UP {
                    return Some(error("too_far_behind", "this document changed a lot meanwhile: open it again"));
                }
                let missed = ops_between(&c, node, after, seq).await.unwrap_or_default();
                return Some(FromLive::Behind { node, client, ops: missed });
            }
            // Someone who may only comment: checked on the document as it is now.
            if comments_only {
                let who = vec![account.display_name.clone(), account.username.clone()];
                if let Some(why) = crate::pages::comments_refused(s, &t, node, ops.clone(), who).await {
                    return Some(error("not_allowed", why));
                }
            }
            let done = async {
                for (i, data) in ops.iter().enumerate() {
                    t.execute(
                        "INSERT INTO ops (node, seq, author, client, data) VALUES ($1, $2, $3, $4, $5)",
                        &[&node, &(seq + 1 + i as i64), &account.id, &client, data],
                    )
                    .await?;
                }
                let to = seq + ops.len() as i64;
                t.execute(
                    "UPDATE nodes SET seq = $2, changed = now(), changed_by = $3 WHERE id = $1",
                    &[&node, &to, &account.id],
                )
                .await?;
                t.commit().await?;
                Ok::<_, tokio_postgres::Error>(to)
            }
            .await;
            match done {
                Ok(to) => {
                    s.note(&Note::Ops { node, from: seq + 1, to, conn: here.conn.clone() }).await;
                    crate::pages::changed(s, node);
                    Some(FromLive::Accepted { node, client, from: seq + 1, to })
                }
                Err(_) => Some(error("failed", "something went wrong")),
            }
        }
        ToLive::Say { node, data } => {
            if open.contains_key(&node) {
                s.note(&Note::Said { node, from: here.clone(), data }).await;
            }
            None
        }
        ToLive::Lease { node, count, at_least } => {
            match open.get(&node) {
                Some(o) if o.may_send() => {}
                _ => return Some(error("not_allowed", "you can only read this document")),
            }
            let count = count.clamp(1, 100_000);
            let r = c
                .query_one(
                    "UPDATE nodes SET next_id = GREATEST(next_id, $2) + $3 WHERE id = $1 RETURNING next_id",
                    &[&node, &at_least, &count],
                )
                .await;
            match r {
                Ok(r) => {
                    let to: i64 = r.get(0);
                    Some(FromLive::Leased { node, from: to - count, to })
                }
                Err(_) => Some(error("failed", "something went wrong")),
            }
        }
    }
}

async fn role_of(c: &deadpool_postgres::Client, me: &Me, node: i64) -> Option<Role> {
    node_role(c, me, node).await.ok().flatten().and_then(|(_, r)| r)
}
