//! The tables and the queries on them.
//!
//! `users`, `sessions` and `invites` start out exactly as the chat server's
//! (`xenify-chat-server/src/db.rs`), then gain columns, so chat can move onto
//! the same database and accounts.

use deadpool_postgres::Client;
use xenify_server_proto::{Account, Group, Person};

pub type Result<T> = std::result::Result<T, tokio_postgres::Error>;

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
ALTER TABLE users ADD COLUMN IF NOT EXISTS admin BOOLEAN NOT NULL DEFAULT FALSE;
-- One-time links (`invites.code` is the link's id; only a hash of its
-- secret is kept): welcome someone an admin added (for_user), reset a
-- password (for_user); invites for anyone are gone (2026-10-29).
ALTER TABLE invites ADD COLUMN IF NOT EXISTS code_hash TEXT;
ALTER TABLE invites ADD COLUMN IF NOT EXISTS for_user BIGINT REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE invites ADD COLUMN IF NOT EXISTS groups BIGINT[] NOT NULL DEFAULT '{}';
ALTER TABLE invites ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'invite';
ALTER TABLE invites ADD COLUMN IF NOT EXISTS email TEXT NOT NULL DEFAULT '';
ALTER TABLE invites ADD COLUMN IF NOT EXISTS made_by BIGINT REFERENCES users(id) ON DELETE SET NULL;
-- An invite someone who isn't an admin asked for waits for an admin.
ALTER TABLE invites ADD COLUMN IF NOT EXISTS approved BOOLEAN NOT NULL DEFAULT TRUE;
CREATE INDEX IF NOT EXISTS invites_for_user ON invites(for_user) WHERE used_by IS NULL;
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS groups (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS group_members (
    group_id BIGINT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX IF NOT EXISTS group_members_by_user ON group_members(user_id);
-- Spaces hold folders and files. Names and details (meta) are JSON text the
-- apps write.
CREATE TABLE IF NOT EXISTS spaces (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('shared', 'personal', 'templates')),
    owner BIGINT REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL DEFAULT '',
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS spaces_personal ON spaces(owner) WHERE kind = 'personal';
CREATE UNIQUE INDEX IF NOT EXISTS spaces_templates ON spaces(kind) WHERE kind = 'templates';
CREATE TABLE IF NOT EXISTS nodes (
    id BIGSERIAL PRIMARY KEY,
    space BIGINT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    parent BIGINT REFERENCES nodes(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('folder', 'file')),
    meta TEXT NOT NULL DEFAULT '',
    own_access BOOLEAN NOT NULL DEFAULT FALSE,
    size BIGINT NOT NULL DEFAULT 0,
    head BIGINT NOT NULL DEFAULT 0,
    created TIMESTAMPTZ NOT NULL DEFAULT now(),
    changed TIMESTAMPTZ NOT NULL DEFAULT now(),
    changed_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    trashed TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS nodes_children ON nodes(space, parent);
CREATE INDEX IF NOT EXISTS nodes_changed ON nodes(changed DESC) WHERE kind = 'file' AND trashed IS NULL;
-- Every saved version of a file; the bytes live in the data folder by hash.
CREATE TABLE IF NOT EXISTS versions (
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    n BIGINT NOT NULL,
    blob TEXT NOT NULL,
    size BIGINT NOT NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now(),
    author BIGINT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (node, n)
);
-- Who may do what: on a whole space, or on a folder or file with its own list.
CREATE TABLE IF NOT EXISTS grants (
    id BIGSERIAL PRIMARY KEY,
    space BIGINT REFERENCES spaces(id) ON DELETE CASCADE,
    node BIGINT REFERENCES nodes(id) ON DELETE CASCADE,
    user_id BIGINT REFERENCES users(id) ON DELETE CASCADE,
    group_id BIGINT REFERENCES groups(id) ON DELETE CASCADE,
    everyone BOOLEAN NOT NULL DEFAULT FALSE,
    role TEXT NOT NULL CHECK (role IN ('viewer', 'commenter', 'editor', 'manager')),
    CHECK ((space IS NULL) <> (node IS NULL))
);
CREATE INDEX IF NOT EXISTS grants_space ON grants(space) WHERE space IS NOT NULL;
CREATE INDEX IF NOT EXISTS grants_node ON grants(node) WHERE node IS NOT NULL;
CREATE INDEX IF NOT EXISTS grants_user ON grants(user_id) WHERE user_id IS NOT NULL;
-- Editing together: every change in order since the file was made. Saved
-- versions say how many they hold; ids for new things are leased out.
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS seq BIGINT NOT NULL DEFAULT 0;
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS next_id BIGINT NOT NULL DEFAULT 1;
ALTER TABLE versions ADD COLUMN IF NOT EXISTS upto BIGINT NOT NULL DEFAULT 0;
-- A checkpoint's description; versions saved as you type have none.
ALTER TABLE versions ADD COLUMN IF NOT EXISTS label TEXT;
-- Who deleted it (shown in a space's bin).
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS trashed_by BIGINT REFERENCES users(id) ON DELETE SET NULL;
-- A picture of the file, for showing it inside other
-- documents and as a thumbnail.
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS preview TEXT;
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS preview_at TIMESTAMPTZ;
-- Pictures put in a document while editing together (changes don't carry
-- them): part of the document as it is now, without saving a version.
CREATE TABLE IF NOT EXISTS node_pictures (
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    blob TEXT NOT NULL,
    PRIMARY KEY (node, name)
);
-- Pictures and clips in Draw canvases, by hash (the bytes in blobs/): the
-- canvas's versions and changes name them, so each is kept once.
CREATE TABLE IF NOT EXISTS node_media (
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    hash TEXT NOT NULL,
    size BIGINT NOT NULL,
    PRIMARY KEY (node, hash)
);
-- Computers whose Xenify Hub is signed in here: what they have installed
-- (Admin › Apps & updates counts them).
CREATE TABLE IF NOT EXISTS hub_computers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    user_id BIGINT REFERENCES users(id) ON DELETE CASCADE,
    products TEXT NOT NULL DEFAULT '{}',
    seen TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS ops (
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    seq BIGINT NOT NULL,
    author BIGINT REFERENCES users(id) ON DELETE SET NULL,
    client TEXT NOT NULL DEFAULT '',
    data TEXT NOT NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (node, seq)
);
CREATE INDEX IF NOT EXISTS ops_by_batch ON ops(node, client);
-- People can sign in with their email address instead of their user name.
ALTER TABLE users ADD COLUMN IF NOT EXISTS email TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS users_email ON users (lower(email)) WHERE email <> '';
-- Chats' whiteboards live in a space of their own per conversation.
ALTER TABLE spaces ADD COLUMN IF NOT EXISTS conv BIGINT;
ALTER TABLE spaces DROP CONSTRAINT IF EXISTS spaces_kind_check;
ALTER TABLE spaces ADD CONSTRAINT spaces_kind_check CHECK (kind IN ('shared', 'personal', 'templates', 'chat'));
CREATE UNIQUE INDEX IF NOT EXISTS spaces_by_conv ON spaces(conv) WHERE kind = 'chat';
-- Files sent in chats: their parts are in the data folder (chat-files/<id>).
CREATE TABLE IF NOT EXISTS chat_files (
    id TEXT PRIMARY KEY,
    conv BIGINT NOT NULL,
    owner BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    size BIGINT NOT NULL,
    parts INT NOT NULL,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Links anyone may read with, no account needed: a file, or a folder and
-- everything in it.
CREATE TABLE IF NOT EXISTS public_links (
    id BIGSERIAL PRIMARY KEY,
    token TEXT NOT NULL UNIQUE,
    node BIGINT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    made_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    made TIMESTAMPTZ NOT NULL DEFAULT now(),
    opened TIMESTAMPTZ
);
CREATE UNIQUE INDEX IF NOT EXISTS public_links_node ON public_links(node);
-- The one templates space: everyone may use what's in it.
INSERT INTO spaces (kind) SELECT 'templates' WHERE NOT EXISTS (SELECT 1 FROM spaces WHERE kind = 'templates');
INSERT INTO grants (space, everyone, role)
    SELECT id, TRUE, 'viewer' FROM spaces s WHERE kind = 'templates'
    AND NOT EXISTS (SELECT 1 FROM grants g WHERE g.space = s.id);
-- Searching: names (in meta) and a plain-text copy of each file's words
-- the apps send when they save. Only the name counts from meta: its other
-- details (the app, the file type) would match every file of a kind.
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS search_text TEXT NOT NULL DEFAULT '';
CREATE OR REPLACE FUNCTION xenify_name(meta TEXT) RETURNS TEXT LANGUAGE plpgsql IMMUTABLE AS $f$
BEGIN
    RETURN COALESCE(meta::jsonb->>'name', '');
EXCEPTION WHEN others THEN
    RETURN meta;
END
$f$;
DO $d$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'nodes' AND column_name = 'search'
               AND generation_expression NOT LIKE '%xenify_name%') THEN
        ALTER TABLE nodes DROP COLUMN search;
    END IF;
END $d$;
ALTER TABLE nodes ADD COLUMN IF NOT EXISTS search tsvector
    GENERATED ALWAYS AS (to_tsvector('simple', xenify_name(meta) || ' ' || search_text)) STORED;
CREATE INDEX IF NOT EXISTS nodes_search ON nodes USING GIN (search);
-- Gone with encryption in the apps (protocol 2): drop what older test
-- databases still have.
DROP TABLE IF EXISTS person_keys;
DROP TABLE IF EXISTS public_link_files;
ALTER TABLE users DROP COLUMN IF EXISTS salt, DROP COLUMN IF EXISTS kdf_m, DROP COLUMN IF EXISTS kdf_t,
    DROP COLUMN IF EXISTS sealed_key, DROP COLUMN IF EXISTS keyring, DROP COLUMN IF EXISTS root_gift;
ALTER TABLE invites DROP COLUMN IF EXISTS sealed_key, DROP COLUMN IF EXISTS sealed_intro;
ALTER TABLE nodes DROP COLUMN IF EXISTS file_key;
ALTER TABLE public_links DROP COLUMN IF EXISTS sealed_key;
DELETE FROM settings WHERE key IN ('encrypted', 'root', 'recovery_hash', 'recovery_key', 'recovery_root');
-- An invite from inside a chat: the new person joins that chat too.
ALTER TABLE invites ADD COLUMN IF NOT EXISTS conv BIGINT REFERENCES conversations(id) ON DELETE SET NULL;
DELETE FROM invites WHERE for_user IS NULL AND used_by IS NULL;
-- Who sent each Draw picture or clip, and what each person saved: what
-- counts towards their storage (Admin › Network › Storage, media.rs).
ALTER TABLE node_media ADD COLUMN IF NOT EXISTS added_by BIGINT REFERENCES users(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS node_media_added_by ON node_media(added_by);
CREATE INDEX IF NOT EXISTS versions_author ON versions(author);
";

/// Sessions not used for this long stop working.
pub const SESSION_IDLE: &str = "90 days";
/// However much they're used, sign-ins end this long after they started.
pub const SESSION_LIFETIME: &str = "365 days";
/// Sessions' kinds (`device`) that aren't sign-ins: an app's own key and
/// someone's key for scripts, made by Chat. They work for Chat only.
pub const KEY_DEVICES: [&str; 2] = ["app", "api"];

pub async fn setting(c: &impl deadpool_postgres::GenericClient, key: &str) -> Result<Option<String>> {
    Ok(c.query_opt("SELECT value FROM settings WHERE key = $1", &[&key]).await?.map(|r| r.get(0)))
}

pub async fn set_setting(c: &impl deadpool_postgres::GenericClient, key: &str, value: &str) -> Result<()> {
    c.execute(
        "INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = $2",
        &[&key, &value],
    )
    .await?;
    Ok(())
}

pub async fn people_count(c: &Client) -> Result<i64> {
    Ok(c.query_one("SELECT count(*) FROM users WHERE NOT app", &[]).await?.get(0))
}

/// Everything signing in needs about an account.
pub struct Login {
    pub account: Account,
    pub hash: String,
    pub disabled: bool,
}

pub async fn username_of(c: &impl deadpool_postgres::GenericClient, id: i64) -> Result<String> {
    Ok(c.query_one("SELECT username FROM users WHERE id = $1", &[&id]).await?.get(0))
}

/// `username`: lower case; an email address finds its account too.
pub async fn login_of(c: &impl deadpool_postgres::GenericClient, username: &str) -> Result<Option<Login>> {
    let r = c
        .query_opt(
            "SELECT id, username, display_name, admin, password_hash, disabled, email
             FROM users WHERE (username = $1 OR (strpos($1, '@') > 0 AND lower(email) = $1)) AND NOT app",
            &[&username],
        )
        .await?;
    Ok(r.map(|r| Login {
        account: Account { id: r.get(0), username: r.get(1), display_name: r.get(2), admin: r.get(3), email: r.get(6) },
        hash: r.get(4),
        disabled: r.get(5),
    }))
}

pub async fn session_account(c: &Client, token_hash: &str) -> Result<Option<Account>> {
    let r = c
        .query_opt(
            &format!(
                "UPDATE sessions s SET last_seen = now() FROM users u
                 WHERE s.token_hash = $1 AND u.id = s.user_id AND NOT u.disabled AND NOT u.app
                   AND s.device <> ALL($2)
                   AND s.last_seen > now() - interval '{SESSION_IDLE}'
                   AND s.created > now() - interval '{SESSION_LIFETIME}'
                 RETURNING u.id, u.username, u.display_name, u.admin, u.email"
            ),
            &[&token_hash, &&KEY_DEVICES[..]],
        )
        .await?;
    Ok(r.map(|r| Account {
        id: r.get(0),
        username: r.get(1),
        display_name: r.get(2),
        admin: r.get(3),
        email: r.get(4),
    }))
}

pub async fn people(c: &Client) -> Result<Vec<Person>> {
    let rows = c
        .query(
            "SELECT u.id, u.username, u.display_name, u.admin, u.disabled,
                    EXISTS (SELECT 1 FROM invites i WHERE i.for_user = u.id AND i.used_by IS NULL AND i.expires > now()),
                    COALESCE((SELECT array_agg(g.group_id ORDER BY g.group_id) FROM group_members g WHERE g.user_id = u.id), '{}'),
                    to_char((SELECT max(s.last_seen) FROM sessions s WHERE s.user_id = u.id) AT TIME ZONE 'UTC',
                            'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
                    u.email
             FROM users u WHERE NOT u.app ORDER BY lower(COALESCE(NULLIF(u.display_name, ''), u.username))",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Person {
            id: r.get(0),
            username: r.get(1),
            display_name: r.get(2),
            admin: r.get(3),
            disabled: r.get(4),
            pending: r.get(5),
            groups: r.get(6),
            last_seen: r.get(7),
            email: r.get(8),
        })
        .collect())
}

pub async fn groups(c: &Client) -> Result<Vec<Group>> {
    let rows = c
        .query(
            "SELECT g.id, g.name,
                    COALESCE((SELECT array_agg(m.user_id ORDER BY m.user_id) FROM group_members m WHERE m.group_id = g.id), '{}')
             FROM groups g ORDER BY lower(g.name)",
            &[],
        )
        .await?;
    Ok(rows.iter().map(|r| Group { id: r.get(0), name: r.get(1), members: r.get(2) }).collect())
}

/// Puts `user` in exactly these groups.
pub async fn set_groups_of(c: &impl deadpool_postgres::GenericClient, user: i64, groups: &[i64]) -> Result<()> {
    c.execute("DELETE FROM group_members WHERE user_id = $1 AND NOT (group_id = ANY($2))", &[&user, &groups]).await?;
    c.execute(
        "INSERT INTO group_members (group_id, user_id) SELECT g, $1 FROM unnest($2::BIGINT[]) g
         WHERE EXISTS (SELECT 1 FROM groups WHERE id = g) ON CONFLICT DO NOTHING",
        &[&user, &groups],
    )
    .await?;
    Ok(())
}

/// Makes `members` exactly the people in `group`.
pub async fn set_members(c: &impl deadpool_postgres::GenericClient, group: i64, members: &[i64]) -> Result<()> {
    c.execute("DELETE FROM group_members WHERE group_id = $1 AND NOT (user_id = ANY($2))", &[&group, &members]).await?;
    c.execute(
        "INSERT INTO group_members (group_id, user_id) SELECT $1, u FROM unnest($2::BIGINT[]) u
         WHERE EXISTS (SELECT 1 FROM users WHERE id = u AND NOT app) ON CONFLICT DO NOTHING",
        &[&group, &members],
    )
    .await?;
    Ok(())
}

/// How many admins can still sign in, not counting `except`.
pub async fn other_admins(c: &impl deadpool_postgres::GenericClient, except: i64) -> Result<i64> {
    Ok(c.query_one("SELECT count(*) FROM users WHERE admin AND NOT disabled AND id <> $1", &[&except]).await?.get(0))
}

/// Removes sign-ins that no longer work (idle too long, or too old), and
/// what two-step sign-in left behind; Chat's keys stay (apps' keys never
/// run out; idle keys for scripts go like sign-ins).
pub async fn remove_old_sessions(c: &impl deadpool_postgres::GenericClient) -> Result<u64> {
    let n = c
        .execute(
            &format!(
                "DELETE FROM sessions s USING users u WHERE u.id = s.user_id AND NOT u.app
                 AND (s.last_seen < now() - interval '{SESSION_IDLE}'
                      OR (s.device <> ALL($1) AND s.created < now() - interval '{SESSION_LIFETIME}'))"
            ),
            &[&&KEY_DEVICES[..]],
        )
        .await?;
    c.execute("DELETE FROM mfa_remember WHERE expires < now()", &[]).await?;
    c.execute("DELETE FROM mfa_tickets WHERE created < now() - interval '1 day'", &[]).await?;
    Ok(n)
}

/// Runs [`remove_old_sessions`] a few minutes after starting, then every
/// few hours, until the server stops.
pub(crate) fn start_cleanup(pool: deadpool_postgres::Pool, mut stopping: tokio::sync::watch::Receiver<bool>) {
    tokio::spawn(async move {
        let mut wait = std::time::Duration::from_secs(240);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = stopping.wait_for(|s| *s) => return,
            }
            if let Ok(c) = pool.get().await
                && let Err(e) = remove_old_sessions(&c).await
            {
                eprintln!("xenify-server: removing old sign-ins: {e}");
            }
            wait = std::time::Duration::from_secs(6 * 3600);
        }
    });
}
