//! The local copy of an account's mail (SQLite): folders, the message list,
//! messages already downloaded, and a full-text index for search. Mail stays
//! readable offline, and lists open instantly.

use crate::imap::{Folder, Role, Summary};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct FolderRow {
    pub wire: String,
    pub name: String,
    pub role: Option<Role>,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub unread: u32,
    pub total: u32,
}

/// A message in a list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub folder: String,
    pub uid: u32,
    pub from: String,
    pub from_address: String,
    pub to: String,
    pub subject: String,
    /// Unix seconds.
    pub date: i64,
    pub flags: Vec<String>,
    pub size: u32,
    /// The whole message is stored here.
    pub downloaded: bool,
}

impl Row {
    pub fn is_read(&self) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen"))
    }
    pub fn is_flagged(&self) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged"))
    }
}

pub struct Store {
    db: Connection,
}

pub type Result<T> = rusqlite::Result<T>;

fn role_name(r: Option<Role>) -> Option<&'static str> {
    r.map(|r| match r {
        Role::Inbox => "inbox",
        Role::Drafts => "drafts",
        Role::Sent => "sent",
        Role::Archive => "archive",
        Role::Junk => "junk",
        Role::Trash => "trash",
        Role::All => "all",
        Role::Flagged => "flagged",
    })
}

fn role_of(s: Option<String>) -> Option<Role> {
    Some(match s?.as_str() {
        "inbox" => Role::Inbox,
        "drafts" => Role::Drafts,
        "sent" => Role::Sent,
        "archive" => Role::Archive,
        "junk" => Role::Junk,
        "trash" => Role::Trash,
        "all" => Role::All,
        "flagged" => Role::Flagged,
        _ => return None,
    })
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::setup(Connection::open(path)?)
    }

    /// A store kept in memory (tests).
    pub fn in_memory() -> Result<Store> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(db: Connection) -> Result<Store> {
        // The window reads while the sync thread writes: wait for each other.
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        // WAL lets them work at the same time. Switching needs the file to
        // itself (the busy wait doesn't cover it), so only the first opener
        // switches, retrying briefly if another opens it at that moment.
        let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            let mut tries = 0;
            loop {
                match db.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
                    Ok(_) => break,
                    Err(_) if tries < 50 => {
                        tries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        db.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS folders (
                 wire TEXT PRIMARY KEY, name TEXT NOT NULL, role TEXT,
                 uid_validity INTEGER NOT NULL DEFAULT 0, uid_next INTEGER NOT NULL DEFAULT 0, ord INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS messages (
                 id INTEGER PRIMARY KEY,
                 folder TEXT NOT NULL REFERENCES folders(wire) ON DELETE CASCADE,
                 uid INTEGER NOT NULL, flags TEXT NOT NULL DEFAULT '', date INTEGER NOT NULL DEFAULT 0,
                 from_name TEXT NOT NULL DEFAULT '', from_address TEXT NOT NULL DEFAULT '', to_list TEXT NOT NULL DEFAULT '',
                 subject TEXT NOT NULL DEFAULT '', size INTEGER NOT NULL DEFAULT 0, message_id TEXT NOT NULL DEFAULT '',
                 raw BLOB,
                 UNIQUE(folder, uid));
             CREATE INDEX IF NOT EXISTS messages_by_date ON messages(folder, date DESC);
             CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(subject, people, body, tokenize = 'unicode61 remove_diacritics 2');",
        )?;
        Ok(Store { db })
    }

    /// Records the server's folders (keeping what we know about each).
    pub fn set_folders(&mut self, folders: &[Folder]) -> Result<()> {
        let tx = self.db.transaction()?;
        let wires: Vec<&str> = folders.iter().filter(|f| !f.no_select).map(|f| f.wire.as_str()).collect();
        {
            let mut existing = tx.prepare("SELECT wire FROM folders")?;
            let gone: Vec<String> = existing
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|w| w.ok())
                .filter(|w| !wires.contains(&w.as_str()))
                .collect();
            for w in gone {
                delete_folder_index(&tx, &w)?;
                tx.execute("DELETE FROM folders WHERE wire = ?1", [&w])?;
            }
        }
        for (i, f) in folders.iter().filter(|f| !f.no_select).enumerate() {
            tx.execute(
                "INSERT INTO folders (wire, name, role, ord) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(wire) DO UPDATE SET name = excluded.name, role = excluded.role, ord = excluded.ord",
                params![f.wire, f.name, role_name(f.role), i as i64],
            )?;
        }
        tx.commit()
    }

    pub fn folders(&self) -> Result<Vec<FolderRow>> {
        let mut q = self.db.prepare(
            "SELECT f.wire, f.name, f.role, f.uid_validity, f.uid_next,
                    (SELECT COUNT(*) FROM messages m WHERE m.folder = f.wire AND instr(m.flags, '\\Seen') = 0),
                    (SELECT COUNT(*) FROM messages m WHERE m.folder = f.wire)
             FROM folders f ORDER BY f.ord",
        )?;
        let rows = q.query_map([], |r| {
            Ok(FolderRow {
                wire: r.get(0)?,
                name: r.get(1)?,
                role: role_of(r.get(2)?),
                uid_validity: r.get::<_, i64>(3)? as u32,
                uid_next: r.get::<_, i64>(4)? as u32,
                unread: r.get::<_, i64>(5)? as u32,
                total: r.get::<_, i64>(6)? as u32,
            })
        })?;
        rows.collect()
    }

    /// The folder with `role` (e.g. Trash), if the account has one.
    pub fn folder_with(&self, role: Role) -> Result<Option<String>> {
        self.db
            .query_row("SELECT wire FROM folders WHERE role = ?1 ORDER BY ord LIMIT 1", [role_name(Some(role))], |r| {
                r.get(0)
            })
            .optional()
    }

    /// Remembers where a folder's sync got to. A new UIDVALIDITY means the
    /// server renumbered everything: the folder's copy is thrown away.
    pub fn set_folder_state(&mut self, wire: &str, uid_validity: u32, uid_next: u32) -> Result<()> {
        let old: Option<i64> =
            self.db.query_row("SELECT uid_validity FROM folders WHERE wire = ?1", [wire], |r| r.get(0)).optional()?;
        let tx = self.db.transaction()?;
        if old.is_some_and(|v| v != 0 && v as u32 != uid_validity) {
            delete_folder_index(&tx, wire)?;
            tx.execute("DELETE FROM messages WHERE folder = ?1", [wire])?;
        }
        tx.execute(
            "UPDATE folders SET uid_validity = ?2, uid_next = ?3 WHERE wire = ?1",
            params![wire, uid_validity as i64, uid_next as i64],
        )?;
        tx.commit()
    }

    /// The highest UID stored for a folder (new mail is above it).
    pub fn last_uid(&self, wire: &str) -> Result<u32> {
        self.db
            .query_row("SELECT COALESCE(MAX(uid), 0) FROM messages WHERE folder = ?1", [wire], |r| r.get::<_, i64>(0))
            .map(|v| v as u32)
    }

    pub fn uids(&self, wire: &str) -> Result<Vec<u32>> {
        let mut q = self.db.prepare("SELECT uid FROM messages WHERE folder = ?1 ORDER BY uid")?;
        q.query_map([wire], |r| r.get::<_, i64>(0).map(|u| u as u32))?.collect()
    }

    /// Adds list entries from the server.
    pub fn add_summaries(&mut self, wire: &str, list: &[Summary]) -> Result<()> {
        let tx = self.db.transaction()?;
        for s in list {
            let h = parse_header(&s.header);
            let date = h.date.unwrap_or_else(|| internal_date(&s.internal_date));
            tx.execute(
                "INSERT INTO messages (folder, uid, flags, date, from_name, from_address, to_list, subject, size, message_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(folder, uid) DO UPDATE SET flags = excluded.flags",
                params![
                    wire,
                    s.uid as i64,
                    s.flags.join(" "),
                    date,
                    h.from,
                    h.from_address,
                    h.to,
                    h.subject,
                    s.size as i64,
                    h.message_id
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute("DELETE FROM search WHERE rowid = ?1", [id])?;
            tx.execute(
                "INSERT INTO search (rowid, subject, people, body) VALUES (?1, ?2, ?3, '')",
                params![id, h.subject, format!("{} {} {}", h.from, h.from_address, h.to)],
            )?;
        }
        tx.commit()
    }

    pub fn set_flags(&mut self, wire: &str, uid: u32, flags: &[String]) -> Result<()> {
        self.db
            .execute(
                "UPDATE messages SET flags = ?3 WHERE folder = ?1 AND uid = ?2",
                params![wire, uid as i64, flags.join(" ")],
            )
            .map(|_| ())
    }

    /// Adds or removes one flag locally (before the server confirms).
    pub fn toggle_flag(&mut self, wire: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
        let flags: Option<String> = self
            .db
            .query_row("SELECT flags FROM messages WHERE folder = ?1 AND uid = ?2", params![wire, uid as i64], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(flags) = flags else { return Ok(()) };
        let mut v: Vec<String> =
            flags.split_whitespace().filter(|f| !f.eq_ignore_ascii_case(flag)).map(str::to_string).collect();
        if on {
            v.push(flag.to_string());
        }
        self.set_flags(wire, uid, &v)
    }

    /// Forgets messages no longer on the server (deleted or moved elsewhere).
    pub fn keep_only(&mut self, wire: &str, server_uids: &[u32]) -> Result<usize> {
        let mine = self.uids(wire)?;
        let gone: Vec<u32> = mine.into_iter().filter(|u| server_uids.binary_search(u).is_err()).collect();
        self.remove(wire, &gone)?;
        Ok(gone.len())
    }

    pub fn remove(&mut self, wire: &str, uids: &[u32]) -> Result<()> {
        let tx = self.db.transaction()?;
        for u in uids {
            tx.execute(
                "DELETE FROM search WHERE rowid = (SELECT id FROM messages WHERE folder = ?1 AND uid = ?2)",
                params![wire, *u as i64],
            )?;
            tx.execute("DELETE FROM messages WHERE folder = ?1 AND uid = ?2", params![wire, *u as i64])?;
        }
        tx.commit()
    }

    /// Stores a downloaded message and indexes its text for search.
    pub fn set_raw(&mut self, wire: &str, uid: u32, raw: &[u8]) -> Result<()> {
        let id: Option<i64> = self
            .db
            .query_row("SELECT id FROM messages WHERE folder = ?1 AND uid = ?2", params![wire, uid as i64], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(id) = id else { return Ok(()) };
        self.db.execute("UPDATE messages SET raw = ?2 WHERE id = ?1", params![id, raw])?;
        if let Some(m) = crate::message::parse(raw) {
            let body: String = m.text.chars().take(200_000).collect();
            self.db.execute("UPDATE search SET body = ?2 WHERE rowid = ?1", params![id, body])?;
        }
        Ok(())
    }

    pub fn raw(&self, wire: &str, uid: u32) -> Result<Option<Vec<u8>>> {
        self.db
            .query_row("SELECT raw FROM messages WHERE folder = ?1 AND uid = ?2", params![wire, uid as i64], |r| {
                r.get(0)
            })
            .optional()
            .map(Option::flatten)
    }

    /// A folder's messages, newest first.
    pub fn list(&self, wire: &str, limit: usize) -> Result<Vec<Row>> {
        self.rows("WHERE m.folder = ?1 ORDER BY m.date DESC, m.uid DESC LIMIT ?2", params![wire, limit as i64])
    }

    pub fn row(&self, wire: &str, uid: u32) -> Result<Option<Row>> {
        Ok(self.rows("WHERE m.folder = ?1 AND m.uid = ?2", params![wire, uid as i64])?.into_iter().next())
    }

    /// Messages anywhere in the account matching `query` (words in the
    /// subject, people or text), newest first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Row>> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        self.rows(
            "JOIN search s ON s.rowid = m.id WHERE search MATCH ?1 ORDER BY m.date DESC LIMIT ?2",
            params![fts, limit as i64],
        )
    }

    fn rows(&self, tail: &str, p: impl rusqlite::Params) -> Result<Vec<Row>> {
        let sql = format!(
            "SELECT m.folder, m.uid, m.from_name, m.from_address, m.to_list, m.subject, m.date, m.flags, m.size, m.raw IS NOT NULL
             FROM messages m {tail}"
        );
        let mut q = self.db.prepare(&sql)?;
        let rows = q.query_map(p, |r| {
            Ok(Row {
                folder: r.get(0)?,
                uid: r.get::<_, i64>(1)? as u32,
                from: r.get(2)?,
                from_address: r.get(3)?,
                to: r.get(4)?,
                subject: r.get(5)?,
                date: r.get(6)?,
                flags: r.get::<_, String>(7)?.split_whitespace().map(str::to_string).collect(),
                size: r.get::<_, i64>(8)? as u32,
                downloaded: r.get(9)?,
            })
        })?;
        rows.collect()
    }
}

fn delete_folder_index(tx: &rusqlite::Transaction, wire: &str) -> Result<()> {
    tx.execute("DELETE FROM search WHERE rowid IN (SELECT id FROM messages WHERE folder = ?1)", [wire]).map(|_| ())
}

/// Each word must appear (as a word or the start of one); quotes keep
/// FTS syntax out of the user's text.
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '@' || *c == '.' || *c == '-' || *c == '\'')
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
struct Header {
    from: String,
    from_address: String,
    to: String,
    subject: String,
    message_id: String,
    date: Option<i64>,
}

fn parse_header(raw: &[u8]) -> Header {
    let mut h = Header::default();
    let Some(m) = mail_parser::MessageParser::default().parse_headers(raw) else { return h };
    if let Some(a) = m.from().and_then(|a| a.first()) {
        h.from = a.name().filter(|n| !n.is_empty()).or(a.address()).unwrap_or("").to_string();
        h.from_address = a.address().unwrap_or("").to_ascii_lowercase();
    }
    h.to = m
        .to()
        .map(|a| {
            a.iter().filter_map(|x| x.name().filter(|n| !n.is_empty()).or(x.address())).collect::<Vec<_>>().join(", ")
        })
        .unwrap_or_default();
    h.subject = m.subject().unwrap_or("").to_string();
    h.message_id = m.message_id().unwrap_or("").to_string();
    h.date = m.date().map(|d| d.to_timestamp());
    h
}

/// "15-Oct-2026 09:41:00 +0100" → Unix seconds (the server's arrival time,
/// used when a message has no Date header).
fn internal_date(s: &str) -> i64 {
    let fake = format!("Date: {}\r\n\r\n", s.replacen('-', " ", 2));
    mail_parser::MessageParser::default()
        .parse_headers(fake.as_bytes())
        .and_then(|m| m.date().map(|d| d.to_timestamp()))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(uid: u32, from: &str, subject: &str, flags: &[&str]) -> Summary {
        Summary {
            uid,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            size: 100,
            internal_date: "15-Oct-2026 09:41:00 +0100".into(),
            header: format!("From: {from}\r\nSubject: {subject}\r\nDate: Thu, 1{uid} Oct 2026 09:41:00 +0000\r\n\r\n")
                .into_bytes(),
        }
    }

    fn folder(wire: &str, role: Option<Role>) -> Folder {
        Folder { wire: wire.into(), name: wire.into(), delimiter: Some("/".into()), role, no_select: false }
    }

    #[test]
    fn lists_counts_flags_and_uidvalidity() {
        let mut s = Store::in_memory().unwrap();
        s.set_folders(&[folder("INBOX", Some(Role::Inbox)), folder("Trash", Some(Role::Trash))]).unwrap();
        s.set_folder_state("INBOX", 7, 4).unwrap();
        s.add_summaries(
            "INBOX",
            &[
                summary(1, "Ana <ana@x.org>", "Lunch", &[]),
                summary(2, "Bo <bo@x.org>", "Report", &["\\Seen"]),
                summary(3, "Cy <cy@x.org>", "Hi", &[]),
            ],
        )
        .unwrap();
        let list = s.list("INBOX", 10).unwrap();
        assert_eq!(list.iter().map(|r| r.uid).collect::<Vec<_>>(), [3, 2, 1], "newest first");
        assert_eq!(list[2].from, "Ana");
        assert_eq!(list[2].from_address, "ana@x.org");
        let inbox = &s.folders().unwrap()[0];
        assert_eq!((inbox.unread, inbox.total), (2, 3));
        assert_eq!(s.folder_with(Role::Trash).unwrap().as_deref(), Some("Trash"));
        assert_eq!(s.last_uid("INBOX").unwrap(), 3);

        s.toggle_flag("INBOX", 1, "\\Seen", true).unwrap();
        assert_eq!(s.folders().unwrap()[0].unread, 1);
        assert_eq!(s.keep_only("INBOX", &[1, 3]).unwrap(), 1);
        assert_eq!(s.uids("INBOX").unwrap(), [1, 3]);

        // Renumbered by the server: start again.
        s.set_folder_state("INBOX", 8, 1).unwrap();
        assert!(s.uids("INBOX").unwrap().is_empty());
    }

    #[test]
    fn search_finds_subject_people_and_body() {
        let mut s = Store::in_memory().unwrap();
        s.set_folders(&[folder("INBOX", Some(Role::Inbox))]).unwrap();
        s.add_summaries(
            "INBOX",
            &[summary(1, "Ana <ana@x.org>", "Lunch plans", &[]), summary(2, "Bo <bo@x.org>", "Quarterly report", &[])],
        )
        .unwrap();
        s.set_raw(
            "INBOX",
            2,
            b"From: bo@x.org\r\nSubject: Quarterly report\r\n\r\nThe totals are attached, see the spreadsheet.\r\n",
        )
        .unwrap();
        let find = |q: &str| s.search(q, 10).unwrap().iter().map(|r| r.uid).collect::<Vec<_>>();
        assert_eq!(find("lunch"), [1]);
        assert_eq!(find("ana"), [1]);
        assert_eq!(find("spreadsheet"), [2]);
        assert_eq!(find("quart"), [2], "word starts match");
        // Search syntax in what you type is treated as plain words.
        s.search("\"unbalanced OR ( NEAR", 10).unwrap();
        assert!(s.raw("INBOX", 2).unwrap().is_some());
        assert!(s.raw("INBOX", 1).unwrap().is_none());
    }
}
