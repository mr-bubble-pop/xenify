//! Messages kept on this computer (one SQLite file per account), so search
//! covers everything, not just what's on screen. Search happens here, never
//! on the server. The file is removed when you sign out.

use xenify_chat_proto::Message;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// How many older messages to fetch at a time when filling in history.
pub const BACKFILL_PAGE: u32 = 200;

#[derive(Default)]
pub struct Store {
    db: Mutex<Option<(PathBuf, Connection)>>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    conv INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    at INTEGER NOT NULL,
    deleted INTEGER NOT NULL,
    -- Lower-cased text, for searching in any language.
    folded TEXT NOT NULL,
    json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS by_conv ON messages (conv, seq);
-- Chats whose whole history is here.
CREATE TABLE IF NOT EXISTS complete (conv INTEGER PRIMARY KEY);
";

/// A short fixed name for a server address (the file name shouldn't hold it).
fn server_key(server: &str) -> String {
    // FNV-1a: stable across runs and versions.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in server.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl Store {
    /// Opens (or makes) the file for `user` on `server`.
    pub fn open(&self, server: &str, user: i64) {
        let dir = xenify_wp_check::data_dir().join("chat");
        let path = dir.join(format!("{}-{user}.sqlite", server_key(server)));
        let mut db = self.db.lock().unwrap();
        if db.as_ref().is_some_and(|(p, _)| *p == path) {
            return;
        }
        let _ = std::fs::create_dir_all(&dir);
        // Your messages: readable only by you.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            if !path.exists() {
                let _ = std::fs::OpenOptions::new().create(true).append(true).open(&path);
            }
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        *db = Connection::open(&path)
            .and_then(|c| {
                c.execute_batch(SCHEMA)?;
                Ok(c)
            })
            .ok()
            .map(|c| (path, c));
    }

    /// Closes the file and removes it (signing out).
    pub fn forget(&self) {
        if let Some((path, c)) = self.db.lock().unwrap().take() {
            drop(c);
            let _ = std::fs::remove_file(&path);
        }
    }

    pub fn put(&self, messages: &[Message]) {
        let mut db = self.db.lock().unwrap();
        let Some((_, c)) = db.as_mut() else { return };
        let Ok(tx) = c.transaction() else { return };
        for m in messages {
            let _ = tx.execute(
                "INSERT INTO messages (id, conv, seq, at, deleted, folded, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO UPDATE SET deleted = ?5, folded = ?6, json = ?7",
                params![
                    m.id,
                    m.conv,
                    m.seq,
                    m.at,
                    m.deleted,
                    m.text.to_lowercase(),
                    serde_json::to_string(m).unwrap_or_default()
                ],
            );
        }
        let _ = tx.commit();
    }

    /// The oldest message kept for `conv`, unless its history is complete.
    pub fn needs_older(&self, conv: i64) -> Option<Option<i64>> {
        let db = self.db.lock().unwrap();
        let (_, c) = db.as_ref()?;
        let done = c
            .query_row("SELECT 1 FROM complete WHERE conv = ?1", [conv], |_| Ok(true))
            .optional()
            .ok()
            .flatten()
            .unwrap_or(false);
        if done {
            return None;
        }
        Some(c.query_row("SELECT min(seq) FROM messages WHERE conv = ?1", [conv], |r| r.get(0)).ok().flatten())
    }

    pub fn mark_complete(&self, conv: i64) {
        if let Some((_, c)) = self.db.lock().unwrap().as_ref() {
            let _ = c.execute("INSERT OR IGNORE INTO complete (conv) VALUES (?1)", [conv]);
        }
    }

    fn query(&self, sql: &str, params: impl rusqlite::Params) -> Vec<Message> {
        let db = self.db.lock().unwrap();
        let Some((_, c)) = db.as_ref() else { return Vec::new() };
        let Ok(mut st) = c.prepare(sql) else { return Vec::new() };
        st.query_map(params, |r| r.get::<_, String>(0))
            .map(|rows| rows.flatten().filter_map(|j| serde_json::from_str(&j).ok()).collect())
            .unwrap_or_default()
    }

    pub fn search(&self, q: &str, limit: u32) -> Vec<Message> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.query(
            "SELECT json FROM messages WHERE deleted = 0 AND instr(folded, ?1) > 0 ORDER BY at DESC LIMIT ?2",
            params![q, limit],
        )
    }

    /// Messages `from`..=`to` of `conv`, oldest first.
    pub fn between(&self, conv: i64, from: i64, to: i64) -> Vec<Message> {
        self.query(
            "SELECT json FROM messages WHERE conv = ?1 AND seq BETWEEN ?2 AND ?3 ORDER BY seq",
            params![conv, from, to],
        )
    }
}

/// Every kept message containing `q` (any case), newest first.
#[tauri::command]
pub fn search_messages(store: State<'_, Store>, q: String) -> Vec<Message> {
    store.search(&q, 300)
}

/// Kept messages of `conv` from `from` to `to` (to show an older search hit).
#[tauri::command]
pub fn messages_between(store: State<'_, Store>, conv: i64, from: i64, to: i64) -> Vec<Message> {
    store.between(conv, from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_chat_proto::User;

    fn msg(id: i64, seq: i64, text: &str) -> Message {
        Message {
            id,
            conv: 7,
            seq,
            sender: User { id: 1, username: "ana".into(), display_name: String::new(), app: false },
            text: text.into(),
            at: 1_000 + seq,
            client_id: String::new(),
            edited: false,
            deleted: false,
            reactions: Vec::new(),
        }
    }

    #[test]
    fn keeps_searches_and_forgets() {
        let tmp = std::env::temp_dir().join(format!("xenify-store-test-{}", std::process::id()));
        // SAFETY: the only test touching this variable.
        unsafe { std::env::set_var("XDG_DATA_HOME", &tmp) };
        let s = Store::default();
        s.open("https://chat.example.org", 1);
        s.put(&[msg(1, 1, "Lunch at Café Rouge"), msg(2, 2, "See the CAFÉ menu"), msg(3, 3, "nothing")]);
        assert_eq!(s.search("café", 10).iter().map(|m| m.seq).collect::<Vec<_>>(), [2, 1], "any case, newest first");
        // Deleted messages aren't found; edits replace.
        let mut gone = msg(2, 2, "");
        gone.deleted = true;
        s.put(&[gone]);
        assert_eq!(s.search("café", 10).len(), 1);
        assert_eq!(s.between(7, 1, 3).len(), 3);
        assert_eq!(s.needs_older(7), Some(Some(1)));
        s.mark_complete(7);
        assert_eq!(s.needs_older(7), None);
        let file = s.db.lock().unwrap().as_ref().unwrap().0.clone();
        assert!(file.exists());
        s.forget();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(tmp);
    }
}
