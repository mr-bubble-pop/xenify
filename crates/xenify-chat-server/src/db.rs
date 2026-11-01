//! The tables and the queries on them.

use deadpool_postgres::Client;
use tokio_postgres::Row;
use xenify_chat_proto::{Conversation, Found, Kind, Message, Notify, Role, User};

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
ALTER TABLE users ADD COLUMN IF NOT EXISTS email TEXT NOT NULL DEFAULT '';
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
-- Conversations that aren't chats: a meeting, your calendar, your busy times
-- (xenify_chat_proto::MEETING…). Empty for chats.
ALTER TABLE conversations ADD COLUMN IF NOT EXISTS purpose TEXT NOT NULL DEFAULT '';
-- Gone from older databases: nothing uses these any more.
DROP TABLE IF EXISTS conv_keys;
ALTER TABLE conversations DROP COLUMN IF EXISTS e2e;
ALTER TABLE conversations DROP COLUMN IF EXISTS key_epoch;
ALTER TABLE conversations DROP COLUMN IF EXISTS rekey;
ALTER TABLE conversations DROP COLUMN IF EXISTS readable_default;
ALTER TABLE members DROP COLUMN IF EXISTS key_from;
CREATE INDEX IF NOT EXISTS messages_by_age ON messages(conv, sent_at);
-- Groups and channels whose owners keep messages for less time than the
-- server does (days).
ALTER TABLE conversations ADD COLUMN IF NOT EXISTS keep_days INT;
-- The last message an email about missed messages told you of.
ALTER TABLE members ADD COLUMN IF NOT EXISTS mailed_seq BIGINT NOT NULL DEFAULT 0;
-- Each person's Chat settings, shared by their devices.
CREATE TABLE IF NOT EXISTS chat_settings (
    user_id BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    value JSONB NOT NULL DEFAULT '{}'
);
-- Reminders, shown on all of someone's devices when due (`shown`: since
-- when it's been shown, NULL: still to come; `done`: dismissed).
CREATE TABLE IF NOT EXISTS reminders (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    starts BIGINT NOT NULL DEFAULT 0,
    link TEXT NOT NULL DEFAULT '',
    -- When the app asked for it (snoozing moves `at`, not this).
    wanted BIGINT NOT NULL,
    shown TIMESTAMPTZ,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (user_id, key)
);
CREATE INDEX IF NOT EXISTS reminders_due ON reminders(at) WHERE shown IS NULL;
-- Threads: a reply's `thread` is the seq of the message it replies to; that
-- message keeps how many replies it has and the newest one's seq and time.
ALTER TABLE messages ADD COLUMN IF NOT EXISTS thread BIGINT;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS replies INT NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS last_reply_seq BIGINT NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS last_reply_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS messages_thread ON messages(conv, thread, seq) WHERE thread IS NOT NULL;
-- Who added someone (NULL: they made it or followed it themselves). An app
-- posts in a channel only when one of its owners added it.
ALTER TABLE members ADD COLUMN IF NOT EXISTS added_by BIGINT;
";

/// Searching messages: the words in each, kept beside it. Files are found
/// by their names (and voice messages by what was said), whiteboards by
/// their titles; other posts apps make (`~name1:{…}`) aren't words. Words
/// are split at every mark, so `report_final.pdf`, `alex@example.org` and
/// `xenify.net/docs` are found by any of their parts.
pub const SEARCH_SCHEMA: &str = r#"
CREATE OR REPLACE FUNCTION chat_text1(body TEXT) RETURNS TEXT LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT CASE
        WHEN body LIKE '~file:{%' THEN concat_ws(' ',
            substring(body FROM '"name":"((?:[^"\\]|\\.)*)"'), substring(body FROM '"text":"((?:[^"\\]|\\.)*)"'))
        WHEN body LIKE '~board:{%' THEN substring(body FROM '"title":"((?:[^"\\]|\\.)*)"')
        WHEN body ~ '^~[a-z]+[0-9]*:[{\[]' THEN ''
        ELSE body
    END
$$;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS words tsvector GENERATED ALWAYS AS (
    to_tsvector('simple', regexp_replace(chat_text1(body), '[[:space:][:punct:]“”‘’«»—–…·•]+', ' ', 'g'))
) STORED;
CREATE INDEX IF NOT EXISTS messages_words ON messages USING GIN (words);
DROP INDEX IF EXISTS messages_search;
"#;

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
    // The email column is there when the query asked for it.
    let email = r.try_get::<_, String>("email").unwrap_or_default();
    User { id: r.get(id), username: r.get(name), display_name: r.get(display), app: r.get(app), email }
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

/// Sessions not used for this long stop working (as on the Xenify server,
/// `SESSION_IDLE`). Apps' own keys don't: they're made to sit in a script.
pub const SESSION_IDLE: &str = "90 days";
/// Sign-ins end this long after they were made, however busy (as on the
/// server); apps' keys and keys for scripts don't.
pub const SESSION_LIFETIME: &str = "365 days";

/// The account behind a session, and the session's kind (`app`: an app's
/// own key; `api`: someone's key for scripts; anything else: signed in). A
/// person who is disabled, or an app whose owner is, gets nothing.
pub async fn session(c: &Client, token_hash: &str) -> Result<Option<(User, String)>> {
    let r = c
        .query_opt(
            &format!(
                "UPDATE sessions s SET last_seen = now() FROM users u
                 WHERE s.token_hash = $1 AND u.id = s.user_id AND NOT u.disabled
                   AND (u.app OR s.last_seen > now() - interval '{SESSION_IDLE}')
                   AND (s.device IN ('app', 'api') OR s.created > now() - interval '{SESSION_LIFETIME}')
                   AND NOT EXISTS (SELECT 1 FROM apps a JOIN users o ON o.id = a.owner
                                   WHERE a.user_id = u.id AND o.disabled)
                 RETURNING u.id, u.username, u.display_name, u.app, u.email, s.device"
            ),
            &[&token_hash],
        )
        .await?;
    Ok(r.map(|r| (user_of(&r, "id", "username", "display_name", "app"), r.get("device"))))
}

pub async fn session_user(c: &Client, token_hash: &str) -> Result<Option<User>> {
    Ok(session(c, token_hash).await?.map(|(u, _)| u))
}

/// Whether the session still works (not signed out, idle or disabled),
/// without counting as a use.
pub async fn session_alive(c: &Client, token_hash: &str) -> Result<bool> {
    Ok(c.query_opt(
        &format!(
            "SELECT 1 FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND NOT u.disabled
               AND (u.app OR s.last_seen > now() - interval '{SESSION_IDLE}')
               AND (s.device IN ('app', 'api') OR s.created > now() - interval '{SESSION_LIFETIME}')
               AND NOT EXISTS (SELECT 1 FROM apps a JOIN users o ON o.id = a.owner
                               WHERE a.user_id = u.id AND o.disabled)"
        ),
        &[&token_hash],
    )
    .await?
    .is_some())
}

/// Someone who can be added to things (an account that isn't disabled).
pub async fn user_id(c: &Client, name: &str) -> Result<Option<i64>> {
    Ok(c.query_opt("SELECT id FROM users WHERE username = $1 AND NOT disabled", &[&name.trim().to_lowercase()])
        .await?
        .map(|r| r.get(0)))
}

pub async fn user(c: &Client, id: i64) -> Result<Option<User>> {
    Ok(c.query_opt("SELECT id, username, display_name, app, email FROM users WHERE id = $1", &[&id])
        .await?
        .map(|r| user_of(&r, "id", "username", "display_name", "app")))
}

pub async fn role_of(c: &Client, conv: i64, user: i64) -> Result<Option<Role>> {
    Ok(c.query_opt("SELECT role FROM members WHERE conv = $1 AND user_id = $2", &[&conv, &user])
        .await?
        .map(|r| if r.get::<_, &str>(0) == "owner" { Role::Owner } else { Role::Member }))
}

/// Whether `user` may post in `conv` (None: not in it). In channels, only
/// owners, and apps an owner added.
pub async fn may_post(c: &Client, conv: i64, user: i64) -> Result<Option<bool>> {
    Ok(c.query_opt(
        "SELECT c.kind <> 'channel' OR m.role = 'owner' OR (u.app AND m.added_by IS NOT NULL)
         FROM members m JOIN conversations c ON c.id = m.conv JOIN users u ON u.id = m.user_id
         WHERE m.conv = $1 AND m.user_id = $2",
        &[&conv, &user],
    )
    .await?
    .map(|r| r.get(0)))
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
    (extract(epoch from m.sent_at) * 1000)::bigint AS at, u.id AS uid, u.username, u.display_name, u.app,
    m.thread, m.replies::bigint AS replies, m.last_reply_seq,
    COALESCE((extract(epoch from m.last_reply_at) * 1000)::bigint, 0) AS last_reply_at";

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
        thread: r.get("thread"),
        replies: r.get("replies"),
        last_reply_seq: r.get("last_reply_seq"),
        last_reply_at: r.get("last_reply_at"),
        thread_people: Vec::new(),
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
    // A reply: who's in its thread.
    if let Some(msg) = m.first_mut()
        && let Some(t) = msg.thread
    {
        msg.thread_people = c
            .query(
                "SELECT DISTINCT sender FROM messages WHERE conv = $1 AND (seq = $2 OR thread = $2)",
                &[&msg.conv, &t],
            )
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
    }
    Ok(m.pop())
}

/// Up to `limit` messages before `before` (None: the newest), oldest first.
/// With `threads` (the server has them), replies stay in their threads.
pub async fn history(c: &Client, conv: i64, before: Option<i64>, limit: u32, threads: bool) -> Result<Vec<Message>> {
    let before = before.unwrap_or(i64::MAX);
    let limit = limit.clamp(1, 200) as i64;
    let rows = c
        .query(
            &format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN users u ON u.id = m.sender
                 WHERE m.conv = $1 AND m.seq < $2 AND (NOT $4 OR m.thread IS NULL) ORDER BY m.seq DESC LIMIT $3"
            ),
            &[&conv, &before, &limit, &threads],
        )
        .await?;
    let mut out: Vec<Message> = rows.iter().map(message_of).collect();
    out.reverse();
    add_reactions(c, &mut out).await?;
    Ok(out)
}

/// One conversation as `user` sees it (None: not theirs).
pub async fn conversation(c: &Client, conv: i64, user: i64) -> Result<Option<Conversation>> {
    Ok(views(c, Some(&[conv]), &[user]).await?.pop().map(|(_, v)| v))
}

/// All of `user`'s conversations, most recently active first.
pub async fn conversations(c: &Client, user: i64) -> Result<Vec<Conversation>> {
    Ok(views(c, None, &[user]).await?.into_iter().map(|(_, v)| v).collect())
}

/// One conversation as each of `users` in it sees it.
pub async fn conversation_for(c: &Client, conv: i64, users: &[i64]) -> Result<Vec<(i64, Conversation)>> {
    views(c, Some(&[conv]), users).await
}

/// Conversations (`convs`, None: all of theirs) as each of `users` sees
/// them, most recently active first, in three queries however many there
/// are: someone in hundreds of chats, meetings and calendars still gets
/// their list at once.
async fn views(c: &Client, convs: Option<&[i64]>, users: &[i64]) -> Result<Vec<(i64, Conversation)>> {
    let rows = c
        .query(
            "SELECT m.user_id, c.id, c.kind, c.title, c.open, c.next_seq, m.role, m.read_seq, m.notify,
                    c.purpose, c.keep_days::bigint AS keep_days, u.app AS me_app, m.added_by IS NOT NULL AS added,
                    CASE WHEN m.muted_until > now() THEN (extract(epoch from m.muted_until) * 1000)::bigint END AS muted_until
             FROM members m JOIN conversations c ON c.id = m.conv JOIN users u ON u.id = m.user_id
             WHERE m.user_id = ANY($2) AND ($1::bigint[] IS NULL OR m.conv = ANY($1))
             ORDER BY c.last_at DESC, c.id",
            &[&convs, &users],
        )
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
    ids.sort_unstable();
    ids.dedup();
    // The first 50 people in each (owners first), and how many there are.
    let people = c
        .query(
            "SELECT conv, id, username, display_name, app, owner, total FROM (
                 SELECT m.conv, u.id, u.username, u.display_name, u.app, m.role = 'owner' AS owner,
                        count(*) OVER (PARTITION BY m.conv) AS total,
                        row_number() OVER (PARTITION BY m.conv ORDER BY (m.role = 'owner') DESC, m.joined, u.id) AS n
                 FROM members m JOIN users u ON u.id = m.user_id WHERE m.conv = ANY($1)
             ) p WHERE n <= 50 ORDER BY conv, n",
            &[&ids],
        )
        .await?;
    let last = c
        .query(
            &format!(
                "SELECT {MESSAGE_COLUMNS} FROM unnest($1::bigint[]) AS k(id)
                 CROSS JOIN LATERAL (SELECT * FROM messages WHERE conv = k.id ORDER BY seq DESC LIMIT 1) m
                 JOIN users u ON u.id = m.sender"
            ),
            &[&ids],
        )
        .await?;
    let mut members: std::collections::HashMap<i64, (Vec<User>, Vec<i64>, i64)> = Default::default();
    for r in &people {
        let e = members.entry(r.get("conv")).or_default();
        let user = user_of(r, "id", "username", "display_name", "app");
        if r.get::<_, bool>("owner") {
            e.1.push(user.id);
        }
        e.0.push(user);
        e.2 = r.get("total");
    }
    let mut lasts: std::collections::HashMap<i64, Message> =
        last.iter().map(|r| (r.get("conv"), message_of(r))).collect();
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let id: i64 = r.get("id");
        let (people, owners, member_count) = members.get(&id).cloned().unwrap_or_default();
        let kind = kind_of(r.get("kind"));
        let role = if r.get::<_, &str>("role") == "owner" { Role::Owner } else { Role::Member };
        // Apps post in channels an owner added them to.
        let me_app: bool = r.get::<_, bool>("me_app") && r.get::<_, bool>("added");
        let last = if users.len() == 1 { lasts.remove(&id) } else { lasts.get(&id).cloned() };
        out.push((
            r.get("user_id"),
            Conversation {
                id,
                kind,
                title: r.get("title"),
                members: people,
                member_count,
                owners,
                role,
                can_post: kind != Kind::Channel || role == Role::Owner || me_app,
                open: r.get("open"),
                last,
                last_seq: r.get("next_seq"),
                read_seq: r.get("read_seq"),
                notify: notify_of(r.get("notify")),
                muted_until: r.get("muted_until"),
                purpose: r.get("purpose"),
                keep_days: r.get("keep_days"),
            },
        ));
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

/// A thread: message `thread` (a seq) and its replies, oldest first (the
/// newest 1,000 replies at most).
pub async fn thread(c: &Client, conv: i64, thread: i64) -> Result<Vec<Message>> {
    let rows = c
        .query(
            &format!(
                "SELECT * FROM (SELECT {MESSAGE_COLUMNS} FROM messages m JOIN users u ON u.id = m.sender
                 WHERE m.conv = $1 AND (m.seq = $2 OR m.thread = $2) ORDER BY m.seq DESC LIMIT 1001) t ORDER BY seq"
            ),
            &[&conv, &thread],
        )
        .await?;
    let mut out: Vec<Message> = rows.iter().map(message_of).collect();
    add_reactions(c, &mut out).await?;
    Ok(out)
}

/// Where a reply goes: Ok(None) when `thread` is a message in the chat
/// itself that can have replies, else why not.
pub async fn thread_root(c: &Client, conv: i64, thread: i64) -> Result<Option<&'static str>> {
    let r = c.query_opt("SELECT thread, deleted FROM messages WHERE conv = $1 AND seq = $2", &[&conv, &thread]).await?;
    Ok(match r {
        None => Some("that message isn't there any more"),
        Some(r) if r.get::<_, Option<i64>>(0).is_some() => Some("replies go in the thread of the first message"),
        Some(r) if r.get::<_, bool>(1) => Some("that message was deleted"),
        Some(_) => None,
    })
}

/// Stores a reply in the thread of message `thread` (once per
/// `client_id`), counts it on that message and tells every server about
/// both. Returns the reply's id.
pub async fn reply(c: &mut Client, conv: i64, thread: i64, sender: i64, client_id: &str, text: &str) -> Result<i64> {
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
    let seq: i64 = tx
        .query_one(
            "UPDATE conversations SET next_seq = next_seq + 1, last_at = now() WHERE id = $1 RETURNING next_seq",
            &[&conv],
        )
        .await?
        .get(0);
    let id: i64 = tx
        .query_one(
            "INSERT INTO messages (conv, seq, sender, client_id, body, thread) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
            &[&conv, &seq, &sender, &client_id, &text, &thread],
        )
        .await?
        .get(0);
    let root: i64 = tx
        .query_one(
            "UPDATE messages SET replies = replies + 1, last_reply_seq = $3, last_reply_at = now()
             WHERE conv = $1 AND seq = $2 RETURNING id",
            &[&conv, &thread, &seq],
        )
        .await?
        .get(0);
    tx.execute("UPDATE members SET read_seq = $3 WHERE conv = $1 AND user_id = $2", &[&conv, &sender, &seq]).await?;
    tx.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &format!("m:{conv}:{id}")]).await?;
    tx.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &format!("m:{conv}:{root}")]).await?;
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
#[allow(clippy::too_many_arguments)] // The new conversation's settings.
pub async fn create(
    c: &mut Client,
    kind: Kind,
    title: &str,
    open: bool,
    owner: i64,
    members: &[i64],
    purpose: &str,
) -> Result<i64> {
    let tx = c.transaction().await?;
    let id: i64 = tx
        .query_one(
            "INSERT INTO conversations (kind, title, open, purpose) VALUES ($1, $2, $3, $4) RETURNING id",
            &[&kind_str(kind), &title, &open, &purpose],
        )
        .await?
        .get(0);
    tx.execute("INSERT INTO members (conv, user_id, role) VALUES ($1, $2, 'owner')", &[&id, &owner]).await?;
    for u in members.iter().filter(|u| **u != owner) {
        tx.execute(
            "INSERT INTO members (conv, user_id, added_by) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            &[&id, u, &owner],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// The calendar or busy-times conversation `owner` already has.
pub async fn owned_with_purpose(c: &Client, owner: i64, purpose: &str) -> Result<Option<i64>> {
    Ok(c.query_opt(
        "SELECT c.id FROM conversations c JOIN members m ON m.conv = c.id AND m.user_id = $1 AND m.role = 'owner'
         WHERE c.purpose = $2 ORDER BY c.id LIMIT 1",
        &[&owner, &purpose],
    )
    .await?
    .map(|r| r.get(0)))
}

/// A conversation's purpose (empty for chats).
pub async fn purpose_of(c: &Client, conv: i64) -> Result<String> {
    Ok(c.query_opt("SELECT purpose FROM conversations WHERE id = $1", &[&conv])
        .await?
        .map(|r| r.get(0))
        .unwrap_or_default())
}

/// The busy-times channels of `users`, as `me` may reach them.
pub async fn busy_of(c: &Client, users: &[i64], me: i64) -> Result<Vec<xenify_chat_proto::BusyChannel>> {
    let rows = c
        .query(
            "SELECT o.user_id, c.id, c.open, EXISTS (SELECT 1 FROM members WHERE conv = c.id AND user_id = $2)
             FROM conversations c JOIN members o ON o.conv = c.id AND o.role = 'owner'
             WHERE c.purpose = 'busy' AND o.user_id = ANY($1)",
            &[&users, &me],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| xenify_chat_proto::BusyChannel { user: r.get(0), conv: r.get(1), open: r.get(2), member: r.get(3) })
        .collect())
}

/// Adds `user` (`by`: who added them; None when they joined themselves).
pub async fn add_member(c: &Client, conv: i64, user: i64, by: Option<i64>) -> Result<bool> {
    Ok(c.execute(
        "INSERT INTO members (conv, user_id, added_by) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        &[&conv, &user, &by],
    )
    .await?
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

/// Everyone on the server (not you) whose user name or name has a word
/// starting with each word of `q` ("luke", "Luke S", "smith"). Anyone
/// signed in finds anyone: names only.
pub async fn find_people(c: &Client, q: &str, me: i64) -> Result<Vec<User>> {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(|w| format!("{}%", like_escape(w))).collect();
    let rows = c
        .query(
            "SELECT id, username, display_name, app, email FROM users u
             WHERE NOT disabled AND id <> $2
               AND NOT EXISTS (SELECT 1 FROM unnest($1::TEXT[]) w
                    WHERE NOT (u.username LIKE w OR lower(u.email) LIKE w OR lower(u.display_name) LIKE w
                               OR lower(u.display_name) LIKE '% ' || w))
             ORDER BY username LIMIT $3",
            // Empty: everyone (the Everyone here address book).
            &[&words, &me, &if words.is_empty() { 1000i64 } else { 20 }],
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
             WHERE c.kind <> 'direct' AND c.purpose = '' AND lower(c.title) LIKE $1
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
             WHERE m.conv = $1 AND a.user_id <> $2 AND a.webhook <> '' AND NOT u.disabled
               AND NOT EXISTS (SELECT 1 FROM users o WHERE o.id = a.owner AND o.disabled)"
        ),
        &[&conv, &sender],
    )
    .await?
    .iter()
    .map(|r| (app_of(r, true), r.get("secret")))
    .collect())
}

/// Your Chat settings (an empty object at first).
pub async fn settings(c: &Client, user: i64) -> Result<serde_json::Value> {
    Ok(c.query_opt("SELECT value FROM chat_settings WHERE user_id = $1", &[&user])
        .await?
        .map(|r| r.get(0))
        .unwrap_or_else(|| serde_json::json!({})))
}

pub async fn save_settings(c: &Client, user: i64, v: &serde_json::Value) -> Result<()> {
    c.execute(
        "INSERT INTO chat_settings (user_id, value) VALUES ($1, $2) ON CONFLICT (user_id) DO UPDATE SET value = $2",
        &[&user, v],
    )
    .await?;
    Ok(())
}

/// `q` as a text search query: every word, each matching itself or a
/// longer word it starts (None: no words in it).
pub fn search_query(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(12)
        // One letter is a word on its own: "b" would start a fifth of them.
        .map(|w| if w.chars().count() > 1 { format!("{}:*", w.to_lowercase()) } else { w.to_lowercase() })
        .collect();
    (!words.is_empty()).then(|| words.join(" & "))
}

/// A page of search results.
#[derive(Debug, Default)]
pub struct Page {
    pub hits: Vec<xenify_chat_proto::Hit>,
    /// How many match, counting up to [`RANKED`] + 1.
    pub total: i64,
    /// The next page's `after` (empty: no more).
    pub more: String,
}

/// Up to this many matches are counted, and ranked all together; a word
/// in more messages than that (most of them, perhaps) is ranked among its
/// newest [`WINDOW`] matches.
pub const RANKED: i64 = 1000;
const WINDOW: i64 = 2000;

/// The chats `user` searches: theirs (or the one asked about, if it's
/// theirs), not the things apps keep in conversations.
async fn searched(c: &Client, user: i64, conv: Option<i64>) -> Result<Vec<i64>> {
    Ok(c.query(
        "SELECT c.id FROM members me JOIN conversations c ON c.id = me.conv AND c.purpose = ''
         WHERE me.user_id = $1 AND ($2::bigint IS NULL OR c.id = $2)",
        &[&user, &conv],
    )
    .await?
    .iter()
    .map(|r| r.get(0))
    .collect())
}

/// Messages `user` may read (in their conversations, which include the
/// channels they follow) matching `q`, a page at a time in `sort` order
/// (`after`: the previous page's `Page::more`).
pub async fn search(
    c: &Client,
    user: i64,
    q: &str,
    conv: Option<i64>,
    sort: xenify_chat_proto::SearchSort,
    after: &str,
) -> Result<Page> {
    let Some(query) = search_query(q) else { return Ok(Page::default()) };
    find(c, user, &query, conv, sort, after, xenify_chat_proto::SEARCH_PAGE).await
}

/// The messages `user` may read with any of `words` (or the start of one),
/// best first: what the AI is given to answer from.
pub async fn search_any(
    c: &Client,
    user: i64,
    words: &[String],
    conv: Option<i64>,
    limit: usize,
) -> Result<Vec<xenify_chat_proto::Hit>> {
    let words: Vec<String> = words.iter().filter_map(|w| search_query(w)).map(|w| format!("({w})")).collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let query = words.join(" | ");
    Ok(find(c, user, &query, conv, xenify_chat_proto::SearchSort::Best, "", limit).await?.hits)
}

/// A page of the messages `user` may read matching text search `query`.
/// Best: pages by position ("o:<offset>"); by date: from the last one
/// shown ("k:<sent, microseconds>:<id>"), so new messages never repeat or
/// skip one.
async fn find(
    c: &Client,
    user: i64,
    query: &str,
    conv: Option<i64>,
    sort: xenify_chat_proto::SearchSort,
    after: &str,
    page: usize,
) -> Result<Page> {
    use tokio_postgres::types::ToSql;
    use xenify_chat_proto::SearchSort;
    let mine = searched(c, user, conv).await?;
    if mine.is_empty() {
        return Ok(Page::default());
    }
    let total: i64 = c
        .query_one(
            "SELECT count(*) FROM (SELECT 1 FROM messages m WHERE m.conv = ANY($1) AND m.words @@ to_tsquery('simple', $2) LIMIT $3) x",
            &[&mine, &query, &(RANKED + 1)],
        )
        .await?
        .get(0);
    if total == 0 {
        return Ok(Page::default());
    }
    let many = total > RANKED;
    let options = format!(
        "StartSel={}, StopSel={}, MaxWords=24, MinWords=8, MaxFragments=1, FragmentDelimiter=…",
        xenify_chat_proto::MARK_START,
        xenify_chat_proto::MARK_END
    );
    // Short messages whole; longer ones, the part that matched.
    let whole = format!(
        "StartSel={}, StopSel={}, HighlightAll=true",
        xenify_chat_proto::MARK_START,
        xenify_chat_proto::MARK_END
    );
    // The page (and one more, to know there's another), then the
    // snippets for those only.
    let outer = |inner: &str, order: &str| {
        format!(
            "SELECT {MESSAGE_COLUMNS}, (extract(epoch from m.sent_at) * 1000000)::bigint AS at_us,
                    ts_headline('simple', chat_text1(m.body), q, CASE WHEN length(m.body) <= 200 THEN $4 ELSE $3 END) AS snippet
             FROM ({inner}) m JOIN users u ON u.id = m.sender, to_tsquery('simple', $1) q ORDER BY {order}"
        )
    };
    // Each chat's newest (or oldest) messages, read in order until enough
    // match: quick for a word said often, which an index of words isn't.
    let fetch = page as i64 + 1;
    let (rows, more) = if sort == SearchSort::Best {
        let offset: i64 = after.strip_prefix("o:").and_then(|o| o.parse().ok()).unwrap_or(0).max(0);
        let cap = if many { WINDOW } else { RANKED };
        if offset >= cap {
            return Ok(Page { hits: Vec::new(), total, more: String::new() });
        }
        let inner = if many {
            format!(
                "SELECT w.*, ts_rank(w.words, to_tsquery('simple', $1)) AS rank FROM (
                     SELECT n.* FROM unnest($2::bigint[]) AS k(id),
                     LATERAL (SELECT * FROM (SELECT * FROM messages WHERE conv = k.id ORDER BY seq DESC LIMIT 20000) n
                              WHERE n.words @@ to_tsquery('simple', $1) AND NOT n.deleted LIMIT {WINDOW}) n
                     ORDER BY n.sent_at DESC LIMIT {WINDOW}
                 ) w ORDER BY rank DESC, w.sent_at DESC, w.id DESC LIMIT $5 OFFSET $6"
            )
        } else {
            "SELECT m.*, ts_rank(m.words, to_tsquery('simple', $1)) AS rank FROM messages m
             WHERE m.conv = ANY($2) AND m.words @@ to_tsquery('simple', $1) AND NOT m.deleted
             ORDER BY rank DESC, m.sent_at DESC, m.id DESC LIMIT $5 OFFSET $6"
                .to_string()
        };
        let sql = outer(&inner, "m.rank DESC, m.sent_at DESC, m.id DESC");
        let params: [&(dyn ToSql + Sync); 6] = [&query, &mine, &options, &whole, &fetch, &offset];
        let rows = c.query(&sql, &params).await?;
        let next = offset + page as i64;
        let more = if rows.len() > page && next < cap { format!("o:{next}") } else { String::new() };
        (rows, more)
    } else {
        let (ord, cmp) = if sort == SearchSort::Newest { ("DESC", "<") } else { ("ASC", ">") };
        let mut key = after.strip_prefix("k:").unwrap_or("").splitn(2, ':').map(|n| n.parse::<i64>().ok());
        let (at, id) = match (key.next().flatten(), key.next().flatten()) {
            (Some(a), Some(i)) => (a, i),
            _ => (0, 0),
        };
        let first = at == 0 && id == 0;
        let t = "('epoch'::timestamptz + ($6::bigint::text || ' microseconds')::interval)";
        let cond = |a: &str| format!("($5 OR ({a}.sent_at {cmp}= {t} AND ({a}.sent_at {cmp} {t} OR {a}.id {cmp} $7)))");
        let plain = format!(
            "SELECT m.*, 0::real AS rank FROM messages m
             WHERE m.conv = ANY($2) AND m.words @@ to_tsquery('simple', $1) AND NOT m.deleted AND {}
             ORDER BY m.sent_at {ord}, m.id {ord} LIMIT $8",
            cond("m")
        );
        let order = format!("m.sent_at {ord}, m.id {ord}");
        let params: [&(dyn ToSql + Sync); 8] = [&query, &mine, &options, &whole, &first, &at, &id, &fetch];
        let mut rows = Vec::new();
        if many {
            let walk = format!(
                "SELECT n.*, 0::real AS rank FROM unnest($2::bigint[]) AS k(id),
                 LATERAL (SELECT * FROM (SELECT * FROM messages x WHERE x.conv = k.id AND {} ORDER BY x.sent_at {ord} LIMIT 20000) n
                          WHERE n.words @@ to_tsquery('simple', $1) AND NOT n.deleted ORDER BY n.sent_at {ord}, n.id {ord} LIMIT $8) n
                 ORDER BY n.sent_at {ord}, n.id {ord} LIMIT $8",
                cond("x")
            );
            rows = c.query(&outer(&walk, &order), &params).await?;
        }
        // Not many, or said a lot but not lately (or early on): all of them.
        if rows.len() < fetch as usize {
            rows = c.query(&outer(&plain, &order), &params).await?;
        }
        let more = match rows.get(page.saturating_sub(1)) {
            Some(last) if rows.len() > page => {
                format!("k:{}:{}", last.get::<_, i64>("at_us"), last.get::<_, i64>("id"))
            }
            _ => String::new(),
        };
        (rows, more)
    };
    let hits = rows
        .iter()
        .take(page)
        .map(|r| xenify_chat_proto::Hit { msg: message_of(r), snippet: r.get("snippet") })
        .collect();
    Ok(Page { hits, total, more })
}

/// Deletes messages older than their conversation's limit: the shorter of
/// the server's (`server_days`, None: for ever) and the chat's own. Things
/// the apps keep in conversations (calendars, meetings, address books)
/// stay. Returns the conversations changed, each with the time (Unix ms)
/// before which everything went.
pub async fn expire(c: &Client, server_days: Option<i64>) -> Result<Vec<(i64, i64)>> {
    let rows = c
        .query(
            "WITH limits AS (
                 SELECT id, LEAST(keep_days::bigint, $1::bigint) AS days FROM conversations
                 WHERE purpose = '' AND (keep_days IS NOT NULL OR $1::bigint IS NOT NULL)
             ), gone AS (
                 DELETE FROM messages m USING limits l
                 WHERE m.conv = l.id AND m.sent_at < now() - make_interval(days => l.days::int)
                 RETURNING m.conv
             )
             SELECT DISTINCT g.conv, (extract(epoch from now() - make_interval(days => l.days::int)) * 1000)::bigint
             FROM gone g JOIN limits l ON l.id = g.conv",
            &[&server_days],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

pub fn reminder_of(r: &Row) -> xenify_chat_proto::Reminder {
    xenify_chat_proto::Reminder {
        key: r.get("key"),
        at: r.get("at_ms"),
        title: r.get("title"),
        body: r.get("body"),
        starts: r.get("starts"),
        link: r.get("link"),
    }
}

pub const REMINDER_COLUMNS: &str =
    "user_id, key, (extract(epoch from at) * 1000)::bigint AS at_ms, title, body, starts, link";

/// Replaces `user`'s reminders with `list`. One whose key and time are
/// unchanged keeps where it is (snoozed, or shown); so does one already
/// shown that's moved to a time gone by, and one shown that's left out
/// stays a while as done (devices whose calendars differ for a moment
/// don't show it again).
pub async fn set_reminders(c: &mut Client, user: i64, list: &[xenify_chat_proto::Reminder]) -> Result<()> {
    let tx = c.transaction().await?;
    let keys: Vec<&str> = list.iter().map(|r| r.key.as_str()).collect();
    tx.execute(
        "DELETE FROM reminders WHERE user_id = $1 AND NOT (key = ANY($2)) AND (shown IS NULL OR shown < now() - interval '2 days')",
        &[&user, &keys],
    )
    .await?;
    tx.execute("UPDATE reminders SET done = TRUE WHERE user_id = $1 AND NOT (key = ANY($2))", &[&user, &keys]).await?;
    for r in list {
        tx.execute(
            "INSERT INTO reminders (user_id, key, at, title, body, starts, link, wanted)
             VALUES ($1, $2, to_timestamp($3::bigint / 1000.0), $4, $5, $6, $7, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET title = $4, body = $5, starts = $6, link = $7,
                 at = CASE WHEN reminders.wanted = $3 THEN reminders.at ELSE excluded.at END,
                 shown = CASE WHEN reminders.wanted = $3 OR excluded.at <= now() THEN reminders.shown END,
                 done = (reminders.wanted = $3 OR excluded.at <= now()) AND reminders.done,
                 wanted = $3",
            &[&user, &r.key, &r.at, &r.title, &r.body, &r.starts, &r.link],
        )
        .await?;
    }
    tx.commit().await
}

/// Reminders now due, marked shown (each is handed out once, whichever
/// server asks).
pub async fn due_reminders(c: &Client) -> Result<Vec<(i64, xenify_chat_proto::Reminder)>> {
    let rows = c
        .query(
            &format!(
                "UPDATE reminders SET shown = now() WHERE (user_id, key) IN (
                     SELECT user_id, key FROM reminders WHERE shown IS NULL AND at <= now() LIMIT 500 FOR UPDATE SKIP LOCKED
                 ) RETURNING {REMINDER_COLUMNS}"
            ),
            &[],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get("user_id"), reminder_of(r))).collect())
}

/// `user`'s reminders shown in the last twelve hours and not dismissed
/// (for a device connecting now).
pub async fn open_reminders(c: &Client, user: i64) -> Result<Vec<xenify_chat_proto::Reminder>> {
    Ok(c.query(
        &format!(
            "SELECT {REMINDER_COLUMNS} FROM reminders
             WHERE user_id = $1 AND shown > now() - interval '12 hours' AND NOT done ORDER BY at"
        ),
        &[&user],
    )
    .await?
    .iter()
    .map(reminder_of)
    .collect())
}

/// One of `user`'s reminders.
pub async fn reminder(c: &Client, user: i64, key: &str) -> Result<Option<xenify_chat_proto::Reminder>> {
    Ok(c.query_opt(
        &format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE user_id = $1 AND key = $2"),
        &[&user, &key],
    )
    .await?
    .as_ref()
    .map(reminder_of))
}
