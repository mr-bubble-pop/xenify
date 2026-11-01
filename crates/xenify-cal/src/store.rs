//! Calendars kept on this computer (SQLite): your own, and copies of ones
//! you subscribe to. Each event is stored as its iCalendar text, so nothing
//! an invitation says is lost.

use crate::event::{Event, When};
use crate::ics;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    pub id: i64,
    pub name: String,
    /// "#rrggbb".
    pub color: String,
    /// "local", "subscription" or "caldav".
    pub kind: String,
    /// The address it's read from (subscriptions, CalDAV).
    pub url: String,
    pub visible: bool,
}

pub struct Store {
    db: Connection,
}

/// A CalDAV resource changed here and waiting to be uploaded.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub uid: String,
    /// Where it is on the server (empty: new).
    pub href: String,
    /// The version we changed (empty: new).
    pub etag: String,
    /// The event and its changed occurrences.
    pub events: Vec<Event>,
}

pub type Result<T> = rusqlite::Result<T>;

/// Colours given to new calendars in turn.
pub const COLORS: [&str; 8] = ["#3b82f6", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6", "#ec4899", "#14b8a6", "#64748b"];

fn rec_key(e: &Event) -> String {
    e.recurrence_id.map(|r| r.to_prop("X").value).unwrap_or_default()
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::setup(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Store> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(db: Connection) -> Result<Store> {
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS calendars (
                 id INTEGER PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'local',
                 url TEXT NOT NULL DEFAULT '', visible INTEGER NOT NULL DEFAULT 1, etag TEXT NOT NULL DEFAULT '');
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY,
                 calendar INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
                 uid TEXT NOT NULL, recurrence TEXT NOT NULL DEFAULT '', ics TEXT NOT NULL,
                 UNIQUE(calendar, uid, recurrence));",
        )?;
        // Older stores: add what two-way sync needs.
        for (table, col, def) in [
            ("events", "href", "TEXT NOT NULL DEFAULT ''"),
            ("events", "etag", "TEXT NOT NULL DEFAULT ''"),
            ("events", "dirty", "INTEGER NOT NULL DEFAULT 0"),
            ("calendars", "account", "TEXT NOT NULL DEFAULT ''"),
        ] {
            let has: bool =
                db.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?.exists([col])?;
            if !has {
                db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
            }
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS removed (calendar INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE, href TEXT NOT NULL, etag TEXT NOT NULL);",
        )?;
        let s = Store { db };
        if s.calendars()?.is_empty() {
            s.add_calendar("My calendar", "local", "")?;
        }
        Ok(s)
    }

    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        let mut q = self.db.prepare("SELECT id, name, color, kind, url, visible FROM calendars ORDER BY id")?;
        q.query_map([], |r| {
            Ok(Calendar {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                kind: r.get(3)?,
                url: r.get(4)?,
                visible: r.get::<_, i64>(5)? != 0,
            })
        })?
        .collect()
    }

    pub fn add_calendar(&self, name: &str, kind: &str, url: &str) -> Result<i64> {
        let n: i64 = self.db.query_row("SELECT COUNT(*) FROM calendars", [], |r| r.get(0))?;
        let color = COLORS[n as usize % COLORS.len()];
        self.db.execute(
            "INSERT INTO calendars (name, color, kind, url) VALUES (?1, ?2, ?3, ?4)",
            params![name, color, kind, url],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// What the last refresh left to check for changes (an ETag, a date or
    /// a sync token).
    pub fn sync_state(&self, id: i64) -> Result<String> {
        self.db.query_row("SELECT etag FROM calendars WHERE id = ?1", [id], |r| r.get(0))
    }

    pub fn set_sync_state(&self, id: i64, state: &str) -> Result<()> {
        self.db.execute("UPDATE calendars SET etag = ?2 WHERE id = ?1", params![id, state]).map(|_| ())
    }

    /// Removes a calendar and its events from this computer.
    pub fn delete_calendar(&self, id: i64) -> Result<()> {
        self.db.execute("DELETE FROM calendars WHERE id = ?1", [id]).map(|_| ())
    }

    /// Adds or replaces an event (matched by UID and which occurrence it
    /// changes). In a CalDAV calendar it's marked to be uploaded.
    pub fn put(&self, calendar: i64, e: &Event) -> Result<()> {
        let text = e.to_component().to_ics();
        let dirty = self.is_caldav(calendar)? as i64;
        // A new occurrence change of an event already on the server shares its address.
        let href: String = self
            .db
            .query_row(
                "SELECT href FROM events WHERE calendar = ?1 AND uid = ?2 AND href != '' LIMIT 1",
                params![calendar, e.uid],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default();
        self.db
            .execute(
                "INSERT INTO events (calendar, uid, recurrence, ics, dirty, href) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(calendar, uid, recurrence) DO UPDATE SET ics = excluded.ics, dirty = excluded.dirty",
                params![calendar, e.uid, rec_key(e), text, dirty, href],
            )
            .map(|_| ())
    }

    fn is_caldav(&self, calendar: i64) -> Result<bool> {
        Ok(self
            .db
            .query_row("SELECT kind FROM calendars WHERE id = ?1", [calendar], |r| r.get::<_, String>(0))
            .optional()?
            .is_some_and(|k| k == "caldav"))
    }

    /// Which CalDAV account a calendar belongs to ("user@host").
    pub fn set_account(&self, calendar: i64, account: &str) -> Result<()> {
        self.db.execute("UPDATE calendars SET account = ?2 WHERE id = ?1", params![calendar, account]).map(|_| ())
    }

    pub fn account(&self, calendar: i64) -> Result<String> {
        self.db.query_row("SELECT account FROM calendars WHERE id = ?1", [calendar], |r| r.get(0))
    }

    /// Events changed here and not yet uploaded, one entry per UID.
    pub fn dirty(&self, calendar: i64) -> Result<Vec<Pending>> {
        let mut q = self.db.prepare("SELECT DISTINCT uid FROM events WHERE calendar = ?1 AND dirty = 1")?;
        let uids: Vec<String> = q.query_map([calendar], |r| r.get(0))?.collect::<Result<_>>()?;
        let mut out = Vec::new();
        for uid in uids {
            let mut q = self.db.prepare("SELECT ics, href, etag FROM events WHERE calendar = ?1 AND uid = ?2")?;
            let rows: Vec<(String, String, String)> = q
                .query_map(params![calendar, uid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_>>()?;
            let href = rows.iter().find(|r| !r.1.is_empty()).map(|r| r.1.clone()).unwrap_or_default();
            let etag = rows.iter().find(|r| !r.2.is_empty()).map(|r| r.2.clone()).unwrap_or_default();
            let events = rows.iter().filter_map(|r| parse_event(&r.0)).collect();
            out.push(Pending { uid, href, etag, events });
        }
        Ok(out)
    }

    /// Uploaded: remember where and which version.
    pub fn mark_clean(&self, calendar: i64, uid: &str, href: &str, etag: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE events SET dirty = 0, href = ?3, etag = ?4 WHERE calendar = ?1 AND uid = ?2",
                params![calendar, uid, href, etag],
            )
            .map(|_| ())
    }

    /// The server's copy of one resource (all events in it), replacing ours.
    pub fn apply_remote(&mut self, calendar: i64, href: &str, etag: &str, text: &str) -> Result<()> {
        let events = crate::event::events_in(text);
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM events WHERE calendar = ?1 AND href = ?2", params![calendar, href])?;
        for e in &events {
            tx.execute(
                "INSERT OR REPLACE INTO events (calendar, uid, recurrence, ics, href, etag, dirty) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![calendar, e.uid, rec_key(e), e.to_component().to_ics(), href, etag],
            )?;
        }
        tx.commit()
    }

    /// The resources we have: (href, etag).
    pub fn remote_items(&self, calendar: i64) -> Result<Vec<(String, String)>> {
        let mut q = self.db.prepare("SELECT DISTINCT href, etag FROM events WHERE calendar = ?1 AND href != ''")?;
        q.query_map([calendar], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    /// Gone from the server.
    pub fn remove_href(&self, calendar: i64, href: &str) -> Result<()> {
        self.db.execute("DELETE FROM events WHERE calendar = ?1 AND href = ?2", params![calendar, href]).map(|_| ())
    }

    /// Deletions to make on the server: (href, etag).
    pub fn removed(&self, calendar: i64) -> Result<Vec<(String, String)>> {
        let mut q = self.db.prepare("SELECT href, etag FROM removed WHERE calendar = ?1")?;
        q.query_map([calendar], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    pub fn clear_removed(&self, calendar: i64, href: &str) -> Result<()> {
        self.db.execute("DELETE FROM removed WHERE calendar = ?1 AND href = ?2", params![calendar, href]).map(|_| ())
    }

    /// Removes an event: one changed occurrence, or (when `recurrence` is
    /// None) the event and all its changed occurrences.
    pub fn delete(&self, calendar: i64, uid: &str, recurrence: Option<&When>) -> Result<()> {
        // On a CalDAV calendar, the server copy is deleted at the next sync.
        if recurrence.is_none() && self.is_caldav(calendar)? {
            self.db.execute(
                "INSERT INTO removed (calendar, href, etag) SELECT DISTINCT calendar, href, etag FROM events WHERE calendar = ?1 AND uid = ?2 AND href != ''",
                params![calendar, uid],
            )?;
        }
        match recurrence {
            Some(r) => self.db.execute(
                "DELETE FROM events WHERE calendar = ?1 AND uid = ?2 AND recurrence = ?3",
                params![calendar, uid, r.to_prop("X").value],
            ),
            None => self.db.execute("DELETE FROM events WHERE calendar = ?1 AND uid = ?2", params![calendar, uid]),
        }
        .map(|_| ())
    }

    /// Every event in the calendar.
    pub fn events(&self, calendar: i64) -> Result<Vec<Event>> {
        let mut q = self.db.prepare("SELECT ics FROM events WHERE calendar = ?1")?;
        let texts: Vec<String> = q.query_map([calendar], |r| r.get(0))?.collect::<Result<_>>()?;
        Ok(texts.iter().filter_map(|t| parse_event(t)).collect())
    }

    pub fn find(&self, calendar: i64, uid: &str) -> Result<Option<Event>> {
        let t: Option<String> = self
            .db
            .query_row(
                "SELECT ics FROM events WHERE calendar = ?1 AND uid = ?2 AND recurrence = ''",
                params![calendar, uid],
                |r| r.get(0),
            )
            .optional()?;
        Ok(t.as_deref().and_then(parse_event))
    }

    /// Adds every event in an .ics file; returns how many.
    pub fn import(&self, calendar: i64, text: &str) -> Result<usize> {
        let events = crate::event::events_in(text);
        for e in &events {
            self.put(calendar, e)?;
        }
        Ok(events.len())
    }

    /// The calendar as an .ics file.
    pub fn export(&self, calendar: i64) -> Result<String> {
        Ok(crate::event::calendar(&self.events(calendar)?, None))
    }

    /// Replaces a calendar's events (refreshing a subscription).
    pub fn replace_all(&mut self, calendar: i64, events: &[Event]) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM events WHERE calendar = ?1", [calendar])?;
        for e in events {
            tx.execute(
                "INSERT OR REPLACE INTO events (calendar, uid, recurrence, ics) VALUES (?1, ?2, ?3, ?4)",
                params![calendar, e.uid, rec_key(e), e.to_component().to_ics()],
            )?;
        }
        tx.commit()
    }
}

fn parse_event(text: &str) -> Option<Event> {
    let wrapped = format!("BEGIN:VCALENDAR\r\n{text}END:VCALENDAR\r\n");
    let cals = ics::parse(&wrapped).ok()?;
    cals[0].children_named("VEVENT").next().and_then(Event::from_component)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn caldav_calendars_track_changes_to_upload() {
        let mut s = Store::in_memory().unwrap();
        let cal = s.add_calendar("Work", "caldav", "https://dav.example.com/sam/work/").unwrap();
        s.set_account(cal, "sam@dav.example.com").unwrap();
        assert_eq!(s.account(cal).unwrap(), "sam@dav.example.com");
        // From the server: clean.
        s.apply_remote(cal, "https://dav.example.com/sam/work/a.ics", "\"1\"", "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:A\r\nDTSTART:20261021T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n").unwrap();
        assert!(s.dirty(cal).unwrap().is_empty());
        // Changed here: to upload, keeping its address and version.
        let mut e = s.events(cal).unwrap()[0].clone();
        e.summary = "A2".into();
        s.put(cal, &e).unwrap();
        let d = s.dirty(cal).unwrap();
        assert_eq!((d[0].href.as_str(), d[0].etag.as_str()), ("https://dav.example.com/sam/work/a.ics", "\"1\""));
        s.mark_clean(cal, "a", "https://dav.example.com/sam/work/a.ics", "\"2\"").unwrap();
        assert!(s.dirty(cal).unwrap().is_empty());
        // Deleted here: queued for the server.
        s.delete(cal, "a", None).unwrap();
        assert_eq!(
            s.removed(cal).unwrap(),
            [("https://dav.example.com/sam/work/a.ics".to_string(), "\"2\"".to_string())]
        );
        // Local calendars never queue anything.
        let mine = s.calendars().unwrap()[0].id;
        s.put(mine, &e).unwrap();
        assert!(s.dirty(mine).unwrap().is_empty());
    }

    #[test]
    fn calendars_events_import_export() {
        let s = Store::in_memory().unwrap();
        let cals = s.calendars().unwrap();
        assert_eq!(cals.len(), 1, "a calendar to start with");
        let mine = cals[0].id;
        let day = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let mut e = Event::new("e1", "Dentist", When::Floating(day.and_hms_opt(9, 0, 0).unwrap()), None);
        e.alarms = vec![30];
        s.put(mine, &e).unwrap();
        e.summary = "Dentist (moved)".into();
        s.put(mine, &e).unwrap();
        let got = s.events(mine).unwrap();
        assert_eq!(got.len(), 1, "same UID replaces");
        assert_eq!(got[0].summary, "Dentist (moved)");
        assert_eq!(got[0].alarms, [30]);

        let work = s.add_calendar("Work", "local", "").unwrap();
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:A\r\nDTSTART:20261021T100000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:B\r\nDTSTART;VALUE=DATE:20261022\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(s.import(work, ics).unwrap(), 2);
        assert_eq!(crate::event::events_in(&s.export(work).unwrap()).len(), 2);
        assert_ne!(s.calendars().unwrap()[1].color, s.calendars().unwrap()[0].color);

        s.delete(work, "a", None).unwrap();
        assert_eq!(s.events(work).unwrap().len(), 1);
        s.delete_calendar(work).unwrap();
        assert_eq!(s.calendars().unwrap().len(), 1);
    }
}
