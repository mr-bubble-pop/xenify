//! The tables and the queries on them.

use deadpool_postgres::Client;
use xenify_chat_proto::{Conversation, Found, Kind, Message, Notify, Role, User};
use tokio_postgres::Row;

pub type Result<T> = std::result::Result<T, tokio_postgres::Error>;

/// The NOTIFY channel servers tell each other on.
pub const CHANNEL: &str = "xenify_chat";

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL DEFAULT '',
    password_hash TEXT NOT NULL,
    app BOOLEAN NOT NULL DEFAULT FALSE,
    disabled BOOLEAN NOT NULL DEFAULT FALSE,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device TEXT NOT NULL DEFAULT '',
    created TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS invites (
    code TEXT PRIMARY KEY,
    created TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires TIMESTAMPTZ NOT NULL,
    used_by BIGINT REFERENCES users(id)
);
CREATE TABLE IF NOT EXISTS conversations (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('direct', 'group', 'channel')),
    title TEXT NOT NULL DEFAULT '',
    -- Direct chats: \"<lower id>:<higher id>\", so there's one per pair.
    direct_key TEXT UNIQUE,
    open BOOLEAN NOT NULL DEFAULT FALSE,
    next_seq BIGINT NOT NULL DEFAULT 0,
    last_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS members (
    conv BIGINT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    read_seq BIGINT NOT NULL DEFAULT 0,
    notify TEXT NOT NULL DEFAULT 'all',
    muted_until TIMESTAMPTZ,
    joined TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (conv, user_id)
);
CREATE INDEX IF NOT EXISTS members_by_user ON members(user_id);
CREATE TABLE IF NOT EXISTS messages (
    id BIGSERIAL PRIMARY KEY,
    conv BIGINT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    seq BIGINT NOT NULL,
    sender BIGINT NOT NULL REFERENCES users(id),
    client_id TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited BOOLEAN NOT NULL DEFAULT FALSE,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (conv, seq)
);
CREATE TABLE IF NOT EXISTS reactions (
    message_id BIGINT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji TEXT NOT NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (message_id, user_id, emoji)
);
CREATE UNIQUE INDEX IF NOT EXISTS messages_once ON messages(conv, sender, client_id) WHERE client_id <> '';
ALTER TABLE users ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'online' CHECK (status IN ('online', 'away', 'busy'));
-- Apps: owned by the person who made them. The secret signs webhooks.
CREATE TABLE IF NOT EXISTS apps (
    user_id BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    owner BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    description TEXT NOT NULL DEFAULT '',
    webhook TEXT NOT NULL DEFAULT '',
    secret TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'personal' CHECK (mode IN ('personal', 'shared', 'both'))
);
-- Who's connected to which server; each server refreshes its rows, so a
-- server that stops leaves nobody looking online for long.
CREATE TABLE IF NOT EXISTS presence (
    server TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conns INT NOT NULL,
    seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (server, user_id)
);
";

/// How long a server's word that someone is connected lasts.
pub const PRESENCE_FRESH: &str = "90 seconds";

/// One more (or one fewer, with `delta` -1) connection of `user` here.
pub async fn presence_change(c: &Client, server: &str, user: i64, delta: i32) -> Result<()> {
    c.execute(
        "INSERT INTO presence (server, user_id, conns) VALUES ($1, $2, $3)
         ON CONFLICT (server, user_id) DO UPDATE SET conns = presence.conns + $3, seen = now()",
        &[&server, &user, &delta],
    )
    .await?;
    c.execute("DELETE FROM presence WHERE server = $1 AND user_id = $2 AND conns <= 0", &[&server, &user]).await?;
    Ok(())
}

/// Keeps this server's connections fresh, and forgets servers that stopped.
pub async fn presence_heartbeat(c: &Client, server: &str) -> Result<()> {
    c.execute("UPDATE presence SET seen = now() WHERE server = $1", &[&server]).await?;
    c.execute(&format!("DELETE FROM presence WHERE seen < now() - interval '{PRESENCE_FRESH}' * 2"), &[]).await?;
    Ok(())
}

pub async fn presence_of(c: &Client, users: &[i64]) -> Result<Vec<xenify_chat_proto::PresenceOf>> {
    use xenify_chat_proto::{Presence, PresenceOf};
    let rows = c
        .query(
            &format!(
                "SELECT u.id, u.status, EXISTS (SELECT 1 FROM presence p WHERE p.user_id = u.id AND p.conns > 0
                        AND p.seen > now() - interval '{PRESENCE_FRESH}')
                 FROM users u WHERE u.id = ANY($1)"
            ),
            &[&users],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let on: bool = r.get(2);
            let state = match (on, r.get::<_, &str>(1)) {
                (false, _) => Presence::Offline,
                (true, "away") => Presence::Away,
                (true, "busy") => Presence::Busy,
                _ => Presence::Online,
            };
            PresenceOf { user: r.get(0), state }
        })
        .collect())
}

/// Everyone you share a direct or group chat with (not channels: those can
/// be very big).
pub async fn contacts(c: &Client, user: i64) -> Result<Vec<i64>> {
    Ok(c.query(
        "SELECT DISTINCT m2.user_id FROM members m1
         JOIN conversations cv ON cv.id = m1.conv AND cv.kind <> 'channel'
         JOIN members m2 ON m2.conv = m1.conv AND m2.user_id <> $1
         WHERE m1.user_id = $1",
        &[&user],
    )
    .await?
    .iter()
    .map(|r| r.get(0))
    .collect())
}

pub fn kind_of(s: &str) -> Kind {
    match s {
        "direct" => Kind::Direct,
        "channel" => Kind::Channel,
        _ => Kind::Group,
    }
}

pub fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Direct => "direct",
        Kind::Group => "group",
        Kind::Channel => "channel",
    }
}

pub fn notify_of(s: &str) -> Notify {
    match s {
        "mentions" => Notify::Mentions,
        "off" => Notify::Off,
        _ => Notify::All,
    }
}

pub fn notify_str(n: Notify) -> &'static str {
    match n {
        Notify::All => "all",
        Notify::Mentions => "mentions",
        Notify::Off => "off",
    }
}

fn user_of(r: &Row, id: &str, name: &str, display: &str, app: &str) -> User {
    User { id: r.get(id), username: r.get(name), display_name: r.get(display), app: r.get(app) }
}

pub async fn user_by_name(c: &Client, name: &str) -> Result<Option<(User, String, bool)>> {
    let r = c
        .query_opt(
            "SELECT id, username, display_name, app, password_hash, disabled FROM users WHERE username = $1",
            &[&name],
        )
        .await?;
    Ok(r.map(|r| (user_of(&r, "id", "username", "display_name", "app"), r.get("password_hash"), r.get("disabled"))))
}

pub async fn create_user(c: &Client, name: &str, display: &str, hash: &str) -> Result<User> {
    let r = c
        .query_one(
            "INSERT INTO users (username, display_name, password_hash) VALUES ($1, $2, $3)
             RETURNING id, username, display_name, app",
            &[&name, &display, &hash],
        )
        .await?;
    Ok(user_of(&r, "id", "username", "display_name", "app"))
}

pub async fn session_user(c: &Client, token_hash: &str) -> Result<Option<User>> {
    let r = c
        .query_opt(
            "UPDATE sessions s SET last_seen = now() FROM users u
             WHERE s.token_hash = $1 AND u.id = s.user_id AND NOT u.disabled
             RETURNING u.id, u.username, u.display_name, u.app",
            &[&token_hash],
        )
        .await?;
    Ok(r.map(|r| user_of(&r, "id", "username", "display_name", "app")))
}

/// Someone who can be added to things (an account that isn't disabled).
pub async fn user_id(c: &Client, name: &str) -> Result<Option<i64>> {
    Ok(c.query_opt("SELECT id FROM users WHERE username = $1 AND NOT disabled", &[&name.trim().to_lowercase()])
        .await?
        .map(|r| r.get(0)))
}

pub async fn user(c: &Client, id: i64) -> Result<Option<User>> {
    Ok(c.query_opt("SELECT id, username, display_name, app FROM users WHERE id = $1", &[&id])
        .await?
        .map(|r| user_of(&r, "id", "username", "display_name", "app")))
}

pub async fn role_of(c: &Client, conv: i64, user: i64) -> Result<Option<Role>> {
    Ok(c.query_opt("SELECT role FROM members WHERE conv = $1 AND user_id = $2", &[&conv, &user])
        .await?
        .map(|r| if r.get::<_, &str>(0) == "owner" { Role::Owner } else { Role::Member }))
}

pub async fn member_ids(c: &Client, conv: i64) -> Result<Vec<i64>> {
    Ok(c.query("SELECT user_id FROM members WHERE conv = $1", &[&conv]).await?.iter().map(|r| r.get(0)).collect())
}

/// A conversation's kind and whether it's open.
pub async fn conv_info(c: &Client, conv: i64) -> Result<Option<(Kind, bool)>> {
    Ok(c.query_opt("SELECT kind, open FROM conversations WHERE id = $1", &[&conv])
        .await?
        .map(|r| (kind_of(r.get(0)), r.get(1))))
}

const MESSAGE_COLUMNS: &str = "m.id, m.conv, m.seq, m.body, m.client_id, m.edited, m.deleted,
    (extract(epoch from m.sent_at) * 1000)::bigint AS at, u.id AS uid, u.username, u.display_name, u.app";

fn message_of(r: &Row) -> Message {
    let deleted: bool = r.get("deleted");
    Message {
        id: r.get("id"),
        conv: r.get("conv"),
        seq: r.get("seq"),
        sender: user_of(r, "uid", "username", "display_name", "app"),
        text: if deleted { String::new() } else { r.get("body") },
        at: r.get("at"),
        client_id: r.get("client_id"),
        edited: r.get("edited"),
        deleted,
        reactions: Vec::new(),
    }
}

/// Fills in the reactions on `messages` (one query for all of them).
async fn add_reactions(c: &Client, messages: &mut [Message]) -> Result<()> {
    if messages.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = messages.iter().map(|m| m.id).collect();
    let rows = c
        .query(
            "SELECT r.message_id, r.emoji, u.id, u.username, u.display_name, u.app FROM reactions r JOIN users u ON u.id = r.user_id
             WHERE r.message_id = ANY($1) ORDER BY r.created",
            &[&ids],
        )
        .await?;
    for r in rows {
        let id: i64 = r.get(0);
        let emoji: String = r.get(1);
        let user = user_of(&r, "id", "username", "display_name", "app");
        if let Some(m) = messages.iter_mut().find(|m| m.id == id && !m.deleted) {
            match m.reactions.iter_mut().find(|x| x.emoji == emoji) {
                Some(x) => x.users.push(user),
                None => m.reactions.push(xenify_chat_proto::Reaction { emoji, users: vec![user] }),
            }
        }
    }
    Ok(())
}

pub async fn message(c: &Client, id: i64) -> Result<Option<Message>> {
    let r = c
        .query_opt(
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages m JOIN users u ON u.id = m.sender WHERE m.id = $1"),
            &[&id],
        )
        .await?;
    let mut m: Vec<Message> = r.as_ref().map(message_of).into_iter().collect();
    add_reactions(c, &mut m).await?;
    Ok(m.pop())
}

/// Up to `limit` messages before `before` (None: the newest), oldest first.
pub async fn history(c: &Client, conv: i64, before: Option<i64>, limit: u32) -> Result<Vec<Message>> {
    let before = before.unwrap_or(i64::MAX);
    let limit = limit.clamp(1, 200) as i64;
    let rows = c
        .query(
            &format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN users u ON u.id = m.sender
                 WHERE m.conv = $1 AND m.seq < $2 ORDER BY m.seq DESC LIMIT $3"
            ),
            &[&conv, &before, &limit],
        )
        .await?;
    let mut out: Vec<Message> = rows.iter().map(message_of).collect();
    out.reverse();
    add_reactions(c, &mut out).await?;
    Ok(out)
}

/// One conversation as `user` sees it (None: not theirs).
pub async fn conversation(c: &Client, conv: i64, user: i64) -> Result<Option<Conversation>> {
    let Some(r) = c
        .query_opt(
            "SELECT c.id, c.kind, c.title, c.open, c.next_seq, m.role, m.read_seq, m.notify,
                    CASE WHEN m.muted_until > now() THEN (extract(epoch from m.muted_until) * 1000)::bigint END AS muted_until,
                    (SELECT count(*) FROM members WHERE conv = c.id) AS member_count,
                    (SELECT app FROM users WHERE id = $2) AS me_app
             FROM conversations c JOIN members m ON m.conv = c.id AND m.user_id = $2
             WHERE c.id = $1",
            &[&conv, &user],
        )
        .await?
    else {
        return Ok(None);
    };
    let rows = c
        .query(
            "SELECT u.id, u.username, u.display_name, u.app, m.role = 'owner' AS owner FROM members m JOIN users u ON u.id = m.user_id
             WHERE m.conv = $1 ORDER BY (m.role = 'owner') DESC, m.joined, u.id LIMIT 50",
            &[&conv],
        )
        .await?;
    let members = rows.iter().map(|r| user_of(r, "id", "username", "display_name", "app")).collect();
    let owners = rows.iter().filter(|r| r.get::<_, bool>("owner")).map(|r| r.get::<_, i64>("id")).collect();
    let last = c
        .query_opt(
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages m JOIN users u ON u.id = m.sender WHERE m.conv = $1 ORDER BY m.seq DESC LIMIT 1"),
            &[&conv],
        )
        .await?
        .as_ref()
        .map(message_of);
    let kind = kind_of(r.get("kind"));
    let role = if r.get::<_, &str>("role") == "owner" { Role::Owner } else { Role::Member };
    let me_app: bool = r.get("me_app");
    Ok(Some(Conversation {
        id: r.get("id"),
        kind,
        title: r.get("title"),
        members,
        member_count: r.get("member_count"),
        owners,
        role,
        can_post: kind != Kind::Channel || role == Role::Owner || me_app,
        open: r.get("open"),
        last,
        last_seq: r.get("next_seq"),
        read_seq: r.get("read_seq"),
        notify: notify_of(r.get("notify")),
        muted_until: r.get("muted_until"),
    }))
}

/// All of `user`'s conversations, most recently active first.
pub async fn conversations(c: &Client, user: i64) -> Result<Vec<Conversation>> {
    let ids: Vec<i64> = c
        .query(
            "SELECT c.id FROM conversations c JOIN members m ON m.conv = c.id WHERE m.user_id = $1 ORDER BY c.last_at DESC",
            &[&user],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(conv) = conversation(c, id, user).await? {
            out.push(conv);
        }
    }
    Ok(out)
}

/// Stores a message (once per `client_id`) and tells every server. Returns its id.
pub async fn send(c: &mut Client, conv: i64, sender: i64, client_id: &str, text: &str) -> Result<i64> {
    let tx = c.transaction().await?;
    if !client_id.is_empty()
        && let Some(r) = tx
            .query_opt(
                "SELECT id FROM messages WHERE conv = $1 AND sender = $2 AND client_id = $3",
                &[&conv, &sender, &client_id],
            )
            .await?
    {
        return Ok(r.get(0));
    }
    // The row lock gives each message the next number, whichever server stores it.
    let seq: i64 = tx
        .query_one(
            "UPDATE conversations SET next_seq = next_seq + 1, last_at = now() WHERE id = $1 RETURNING next_seq",
            &[&conv],
        )
        .await?
        .get(0);
    let id: i64 = tx
        .query_one(
            "INSERT INTO messages (conv, seq, sender, client_id, body) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&conv, &seq, &sender, &client_id, &text],
        )
        .await?
        .get(0);
    // Your own message counts as read.
    tx.execute("UPDATE members SET read_seq = $3 WHERE conv = $1 AND user_id = $2", &[&conv, &sender, &seq]).await?;
    tx.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &format!("m:{conv}:{id}")]).await?;
    tx.commit().await?;
    Ok(id)
}

/// The direct chat between two people (one with yourself is "Notes to self").
pub async fn open_direct(c: &mut Client, me: i64, other: i64) -> Result<i64> {
    let key = format!("{}:{}", me.min(other), me.max(other));
    let tx = c.transaction().await?;
    let id: i64 = tx
        .query_one(
            "INSERT INTO conversations (kind, direct_key) VALUES ('direct', $1)
             ON CONFLICT (direct_key) DO UPDATE SET direct_key = excluded.direct_key RETURNING id",
            &[&key],
        )
        .await?
        .get(0);
    for u in [me, other] {
        tx.execute("INSERT INTO members (conv, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&id, &u]).await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// A new group or channel with `owner` and `members`.
pub async fn create(c: &mut Client, kind: Kind, title: &str, open: bool, owner: i64, members: &[i64]) -> Result<i64> {
    let tx = c.transaction().await?;
    let id: i64 = tx
        .query_one(
            "INSERT INTO conversations (kind, title, open) VALUES ($1, $2, $3) RETURNING id",
            &[&kind_str(kind), &title, &open],
        )
        .await?
        .get(0);
    tx.execute("INSERT INTO members (conv, user_id, role) VALUES ($1, $2, 'owner')", &[&id, &owner]).await?;
    for u in members.iter().filter(|u| **u != owner) {
        tx.execute("INSERT INTO members (conv, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&id, u]).await?;
    }
    tx.commit().await?;
    Ok(id)
}

pub async fn add_member(c: &Client, conv: i64, user: i64) -> Result<bool> {
    Ok(c.execute("INSERT INTO members (conv, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&conv, &user]).await?
        > 0)
}

/// Takes `user` out. An owner leaving hands the conversation to whoever has
/// been in it longest; the last one out deletes it. Returns whether it's gone.
pub async fn leave(c: &mut Client, conv: i64, user: i64) -> Result<bool> {
    let tx = c.transaction().await?;
    // One change at a time per conversation.
    tx.query_one("SELECT id FROM conversations WHERE id = $1 FOR UPDATE", &[&conv]).await?;
    tx.execute("DELETE FROM members WHERE conv = $1 AND user_id = $2", &[&conv, &user]).await?;
    let left: i64 = tx.query_one("SELECT count(*) FROM members WHERE conv = $1", &[&conv]).await?.get(0);
    let gone = left == 0;
    if gone {
        tx.execute("DELETE FROM conversations WHERE id = $1", &[&conv]).await?;
    } else {
        tx.execute(
            "UPDATE members SET role = 'owner' WHERE conv = $1 AND NOT EXISTS (SELECT 1 FROM members WHERE conv = $1 AND role = 'owner')
             AND user_id = (SELECT user_id FROM members WHERE conv = $1 ORDER BY joined, user_id LIMIT 1)",
            &[&conv],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(gone)
}

/// People whose user name or display name starts with `q` (not you, not apps).
pub async fn find_people(c: &Client, q: &str, me: i64) -> Result<Vec<User>> {
    let pattern = format!("{}%", like_escape(&q.trim().to_lowercase()));
    let rows = c
        .query(
            "SELECT id, username, display_name, app FROM users
             WHERE NOT disabled AND id <> $2
               AND (username LIKE $1 OR lower(display_name) LIKE $1 OR lower(display_name) LIKE '% ' || $1)
             ORDER BY username LIMIT 20",
            &[&pattern, &me],
        )
        .await?;
    Ok(rows.iter().map(|r| user_of(r, "id", "username", "display_name", "app")).collect())
}

/// Your groups and channels, and open channels, whose title contains `q`.
/// Groups you're not in stay private.
pub async fn find_conversations(c: &Client, q: &str, me: i64) -> Result<Vec<Found>> {
    let pattern = format!("%{}%", like_escape(&q.trim().to_lowercase()));
    let rows = c
        .query(
            "SELECT c.id, c.kind, c.title,
                    (SELECT count(*) FROM members WHERE conv = c.id) AS member_count,
                    EXISTS (SELECT 1 FROM members WHERE conv = c.id AND user_id = $2) AS joined
             FROM conversations c
             WHERE c.kind <> 'direct' AND lower(c.title) LIKE $1
               AND (EXISTS (SELECT 1 FROM members WHERE conv = c.id AND user_id = $2) OR (c.kind = 'channel' AND c.open))
             ORDER BY joined DESC, c.title LIMIT $3",
            // An empty search lists everything (the app's channel list).
            &[&pattern, &me, &if q.trim().is_empty() { 200i64 } else { 20 }],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Found {
            id: r.get(0),
            kind: kind_of(r.get(1)),
            title: r.get(2),
            member_count: r.get(3),
            joined: r.get(4),
        })
        .collect())
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

pub fn mode_str(m: xenify_chat_proto::AppMode) -> &'static str {
    match m {
        xenify_chat_proto::AppMode::Personal => "personal",
        xenify_chat_proto::AppMode::Shared => "shared",
        xenify_chat_proto::AppMode::Both => "both",
    }
}

fn mode_of(s: &str) -> xenify_chat_proto::AppMode {
    match s {
        "shared" => xenify_chat_proto::AppMode::Shared,
        "both" => xenify_chat_proto::AppMode::Both,
        _ => xenify_chat_proto::AppMode::Personal,
    }
}

const APP_COLUMNS: &str =
    "u.id, u.username, u.display_name, u.app, a.description, a.webhook, a.mode, a.owner, a.secret";

fn app_of(r: &Row, with_webhook: bool) -> xenify_chat_proto::AppInfo {
    xenify_chat_proto::AppInfo {
        user: user_of(r, "id", "username", "display_name", "app"),
        description: r.get("description"),
        webhook: if with_webhook { r.get("webhook") } else { String::new() },
        mode: mode_of(r.get("mode")),
        owner: r.get("owner"),
    }
}

/// An app and its webhook secret.
pub async fn app(c: &Client, id: i64) -> Result<Option<(xenify_chat_proto::AppInfo, String)>> {
    Ok(c.query_opt(
        &format!("SELECT {APP_COLUMNS} FROM apps a JOIN users u ON u.id = a.user_id WHERE a.user_id = $1"),
        &[&id],
    )
    .await?
    .map(|r| (app_of(&r, true), r.get("secret"))))
}

pub async fn apps_owned(c: &Client, owner: i64) -> Result<Vec<xenify_chat_proto::AppInfo>> {
    Ok(c.query(
        &format!(
            "SELECT {APP_COLUMNS} FROM apps a JOIN users u ON u.id = a.user_id WHERE a.owner = $1 ORDER BY u.username"
        ),
        &[&owner],
    )
    .await?
    .iter()
    .map(|r| app_of(r, true))
    .collect())
}

/// The apps in a conversation (without their webhooks).
pub async fn apps_in(c: &Client, conv: i64) -> Result<Vec<xenify_chat_proto::AppInfo>> {
    Ok(c.query(
        &format!(
            "SELECT {APP_COLUMNS} FROM apps a JOIN users u ON u.id = a.user_id JOIN members m ON m.user_id = a.user_id
             WHERE m.conv = $1 ORDER BY u.username"
        ),
        &[&conv],
    )
    .await?
    .iter()
    .map(|r| app_of(r, false))
    .collect())
}

/// Apps in `conv` (other than `sender`) that have a webhook, with their secrets.
pub async fn webhook_apps(c: &Client, conv: i64, sender: i64) -> Result<Vec<(xenify_chat_proto::AppInfo, String)>> {
    Ok(c.query(
        &format!(
            "SELECT {APP_COLUMNS} FROM apps a JOIN users u ON u.id = a.user_id JOIN members m ON m.user_id = a.user_id
             WHERE m.conv = $1 AND a.user_id <> $2 AND a.webhook <> '' AND NOT u.disabled"
        ),
        &[&conv, &sender],
    )
    .await?
    .iter()
    .map(|r| (app_of(r, true), r.get("secret")))
    .collect())
}
