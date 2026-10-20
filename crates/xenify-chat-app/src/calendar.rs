//! Your calendar, kept on this computer: events you add, and reminders made
//! from a chat message or an email ("Remind me tomorrow"). The screens
//! draw the Today panel and the week view from it, and show reminders when
//! they're due.

use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use xenify_cal::event::{Event as CalEvent, When};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// 0 for a new one.
    #[serde(default)]
    pub id: i64,
    pub title: String,
    /// Unix milliseconds.
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub notes: String,
    /// "event" or "reminder".
    #[serde(default = "event_kind")]
    pub kind: String,
    /// What a reminder is about, as the screens describe it (a chat message
    /// or an email), as JSON; empty for plain events.
    #[serde(default)]
    pub link: String,
    /// Minutes before the start to remind you (None: no reminder).
    #[serde(default)]
    pub remind: Option<i64>,
    /// The reminder has been shown.
    #[serde(default)]
    pub reminded: bool,
    /// Ticked off (reminders).
    #[serde(default)]
    pub done: bool,
    /// How it repeats (an iCalendar RRULE such as "FREQ=WEEKLY"), or empty.
    #[serde(default)]
    pub repeat: String,
    /// The invitation's UID, for events added from an invitation.
    #[serde(default)]
    pub ical_uid: String,
    /// A reminder that also hides its chat or email until it's due ("Later").
    #[serde(default)]
    pub hides: bool,
    /// For one appearance of a repeating event: when the series starts and
    /// ends (start and end above are this appearance's).
    #[serde(default)]
    pub series_start: i64,
    #[serde(default)]
    pub series_end: i64,
}

fn event_kind() -> String {
    "event".into()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    start INTEGER NOT NULL,
    end_at INTEGER NOT NULL,
    all_day INTEGER NOT NULL DEFAULT 0,
    notes TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT 'event',
    link TEXT NOT NULL DEFAULT '',
    remind INTEGER,
    reminded INTEGER NOT NULL DEFAULT 0,
    done INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS by_time ON events (start);
";

/// Columns added later (each fails harmlessly when it's already there).
const ADDED: [&str; 5] = [
    "ALTER TABLE events ADD COLUMN repeat TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE events ADD COLUMN ical_uid TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE events ADD COLUMN hides INTEGER NOT NULL DEFAULT 0",
    // The latest appearance of a repeating event whose reminder was shown.
    "ALTER TABLE events ADD COLUMN reminded_at INTEGER NOT NULL DEFAULT 0",
    "CREATE INDEX IF NOT EXISTS by_uid ON events (ical_uid)",
];

/// This computer's time zone (UTC if it can't be told).
pub fn local_tz() -> Tz {
    iana_time_zone::get_timezone().ok().and_then(|n| n.parse().ok()).unwrap_or(chrono_tz::UTC)
}

/// A repeating event's appearances overlapping `from..to` (Unix ms), with
/// the wall-clock time kept across daylight-saving changes.
fn appearances(e: &Event, from: i64, to: i64, tz: Tz) -> Vec<(i64, i64)> {
    let at = |ms: i64| Utc.timestamp_millis_opt(ms).single().unwrap_or_default();
    let when = |ms: i64| {
        let local = at(ms).with_timezone(&tz);
        if e.all_day { When::Date(local.date_naive()) } else { When::Zoned(local.naive_local(), tz) }
    };
    let mut c = CalEvent::new(e.id.to_string(), e.title.clone(), when(e.start), Some(when(e.end.max(e.start))));
    c.rrule = Some(e.repeat.clone());
    xenify_cal::expand::occurrences(std::slice::from_ref(&c), at(from), at(to), tz)
        .into_iter()
        .map(|o| (o.start.timestamp_millis(), o.end.timestamp_millis()))
        .collect()
}

pub struct Calendar {
    db: Mutex<Option<Connection>>,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar::open(&xenify_wp_check::data_dir().join("calendar.sqlite"))
    }
}

fn event_of(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        title: r.get(1)?,
        start: r.get(2)?,
        end: r.get(3)?,
        all_day: r.get(4)?,
        notes: r.get(5)?,
        kind: r.get(6)?,
        link: r.get(7)?,
        remind: r.get(8)?,
        reminded: r.get(9)?,
        done: r.get(10)?,
        repeat: r.get(11)?,
        ical_uid: r.get(12)?,
        hides: r.get(13)?,
        series_start: r.get(2)?,
        series_end: r.get(3)?,
    })
}

const COLUMNS: &str = "id, title, start, end_at, all_day, notes, kind, link, remind, reminded, done, repeat, ical_uid, hides, reminded_at";

impl Calendar {
    pub fn open(path: &std::path::Path) -> Calendar {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let db = Connection::open(path)
            .and_then(|c| {
                c.execute_batch(SCHEMA)?;
                for a in ADDED {
                    let _ = c.execute_batch(a);
                }
                Ok(c)
            })
            .ok();
        Calendar { db: Mutex::new(db) }
    }

    fn with<T: Default>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let db = self.db.lock().unwrap();
        let c = db.as_ref().ok_or("The calendar file couldn't be opened.")?;
        f(c).map_err(|e| e.to_string())
    }

    /// Events (and appearances of repeating ones) overlapping `from`..`to`,
    /// earliest first.
    pub fn between(&self, from: i64, to: i64) -> Result<Vec<Event>, String> {
        let (once, repeating) = self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLUMNS} FROM events WHERE repeat = '' AND (start < ?2 AND end_at > ?1 OR (start >= ?1 AND start < ?2))"
            ))?;
            let once: Vec<Event> = st.query_map(params![from, to], event_of)?.collect::<rusqlite::Result<_>>()?;
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM events WHERE repeat <> '' AND start < ?1"))?;
            let repeating: Vec<Event> = st.query_map(params![to], event_of)?.collect::<rusqlite::Result<_>>()?;
            Ok((once, repeating))
        })?;
        let tz = local_tz();
        let mut out = once;
        for e in repeating {
            for (start, end) in appearances(&e, from, to, tz) {
                out.push(Event { start, end, ..e.clone() });
            }
        }
        out.sort_by_key(|e| e.start);
        Ok(out)
    }

    /// Reminders due by `now` and not yet shown (for a repeating event, its
    /// latest appearance that's due).
    pub fn due(&self, now: i64) -> Result<Vec<Event>, String> {
        let (mut out, repeating) = self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLUMNS} FROM events WHERE repeat = '' AND remind IS NOT NULL AND reminded = 0 AND done = 0
                 AND start - remind * 60000 <= ?1 AND end_at >= ?1 - 86400000 ORDER BY start"
            ))?;
            let once: Vec<Event> = st.query_map(params![now], event_of)?.collect::<rusqlite::Result<_>>()?;
            let mut st = c.prepare(&format!(
                "SELECT {COLUMNS}, reminded_at FROM events WHERE repeat <> '' AND remind IS NOT NULL AND done = 0 AND start <= ?1 + 2 * 86400000"
            ))?;
            let repeating: Vec<(Event, i64)> = st
                .query_map(params![now], |r| Ok((event_of(r)?, r.get(15)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok((once, repeating))
        })?;
        let tz = local_tz();
        for (e, reminded_at) in repeating {
            let ahead = e.remind.unwrap_or(0) * 60_000;
            let next = appearances(&e, now - 86_400_000, now + ahead + 60_000, tz)
                .into_iter()
                .rfind(|(start, _)| *start - ahead <= now && *start > reminded_at);
            if let Some((start, end)) = next {
                out.push(Event { start, end, ..e });
            }
        }
        Ok(out)
    }

    /// Reminders that hide their chat or email until they're due.
    pub fn snoozed(&self, now: i64) -> Result<Vec<Event>, String> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLUMNS} FROM events WHERE hides = 1 AND done = 0 AND start > ?1 ORDER BY start"
            ))?;
            st.query_map(params![now], event_of)?.collect()
        })
    }

    /// The event added from invitation `uid`, if there is one.
    pub fn by_uid(&self, uid: &str) -> Result<Option<Event>, String> {
        if uid.is_empty() {
            return Ok(None);
        }
        self.with(|c| {
            c.query_row(&format!("SELECT {COLUMNS} FROM events WHERE ical_uid = ?1"), [uid], event_of).optional()
        })
    }

    /// Adds (`id` 0) or changes an event; returns it as saved.
    pub fn save(&self, e: &Event) -> Result<Event, String> {
        if e.title.trim().is_empty() {
            return Err("Give it a name.".into());
        }
        if e.end < e.start {
            return Err("It ends before it starts.".into());
        }
        self.with(|c| {
            let id = if e.id == 0 {
                c.execute(
                    "INSERT INTO events (title, start, end_at, all_day, notes, kind, link, remind, reminded, done, repeat, ical_uid, hides)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        e.title.trim(),
                        e.start,
                        e.end,
                        e.all_day,
                        e.notes,
                        e.kind,
                        e.link,
                        e.remind,
                        e.reminded,
                        e.done,
                        e.repeat.trim(),
                        e.ical_uid,
                        e.hides
                    ],
                )?;
                c.last_insert_rowid()
            } else {
                // A new time means the reminder is due again.
                let was: Option<i64> =
                    c.query_row("SELECT start FROM events WHERE id = ?1", [e.id], |r| r.get(0)).optional()?;
                let reminded = e.reminded && was == Some(e.start);
                c.execute(
                    "UPDATE events SET title = ?2, start = ?3, end_at = ?4, all_day = ?5, notes = ?6, kind = ?7, link = ?8,
                     remind = ?9, reminded = ?10, done = ?11, repeat = ?12, ical_uid = ?13, hides = ?14 WHERE id = ?1",
                    params![
                        e.id,
                        e.title.trim(),
                        e.start,
                        e.end,
                        e.all_day,
                        e.notes,
                        e.kind,
                        e.link,
                        e.remind,
                        reminded,
                        e.done,
                        e.repeat.trim(),
                        e.ical_uid,
                        e.hides
                    ],
                )?;
                e.id
            };
            c.query_row(&format!("SELECT {COLUMNS} FROM events WHERE id = ?1"), [id], event_of)
        })
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.with(|c| c.execute("DELETE FROM events WHERE id = ?1", [id]).map(|_| ()))
    }

    /// The reminder for event `id` (its appearance at `at`, if it repeats)
    /// has been shown.
    pub fn mark_reminded(&self, id: i64, at: i64) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "UPDATE events SET reminded = 1, reminded_at = max(reminded_at, ?2) WHERE id = ?1",
                params![id, at],
            )
            .map(|_| ())
        })
    }

    pub fn delete_uid(&self, uid: &str) -> Result<(), String> {
        self.with(|c| c.execute("DELETE FROM events WHERE ical_uid = ?1", [uid]).map(|_| ()))
    }
}

#[tauri::command]
pub fn cal_between(cal: State<'_, Calendar>, from: i64, to: i64) -> Result<Vec<Event>, String> {
    cal.between(from, to)
}

#[tauri::command]
pub fn cal_due(cal: State<'_, Calendar>, now: i64) -> Result<Vec<Event>, String> {
    cal.due(now)
}

#[tauri::command]
pub fn cal_save(cal: State<'_, Calendar>, event: Event) -> Result<Event, String> {
    cal.save(&event)
}

#[tauri::command]
pub fn cal_delete(cal: State<'_, Calendar>, id: i64) -> Result<(), String> {
    cal.delete(id)
}

#[tauri::command]
pub fn cal_reminded(cal: State<'_, Calendar>, id: i64, at: i64) -> Result<(), String> {
    cal.mark_reminded(id, at)
}

#[tauri::command]
pub fn cal_snoozed(cal: State<'_, Calendar>, now: i64) -> Result<Vec<Event>, String> {
    cal.snoozed(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    #[test]
    fn events_and_reminders() {
        let path = std::env::temp_dir().join(format!("xenify-cal-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let cal = Calendar::open(&path);
        let lunch = cal
            .save(&Event {
                title: "Lunch with Ana".into(),
                start: 12 * H,
                end: 13 * H,
                remind: Some(10),
                ..Event::default()
            })
            .unwrap();
        cal.save(&Event {
            title: "Read the contract".into(),
            start: 30 * H,
            end: 30 * H,
            kind: "reminder".into(),
            remind: Some(0),
            ..Event::default()
        })
        .unwrap();
        assert_eq!(cal.between(0, 24 * H).unwrap().len(), 1, "only today's");
        // Ten minutes before lunch it's due, once.
        assert!(cal.due(12 * H - 11 * 60_000).unwrap().is_empty());
        assert_eq!(cal.due(12 * H - 10 * 60_000).unwrap()[0].id, lunch.id);
        cal.mark_reminded(lunch.id, lunch.start).unwrap();
        assert!(cal.due(12 * H).unwrap().is_empty());
        // Moved: due again at the new time.
        let moved = cal.save(&Event { start: 14 * H, end: 15 * H, reminded: true, ..lunch.clone() }).unwrap();
        assert!(!moved.reminded);
        assert!(cal.save(&Event { title: " ".into(), ..lunch.clone() }).is_err());
        assert!(cal.save(&Event { end: 0, ..lunch }).is_err());
        cal.delete(moved.id).unwrap();
        assert_eq!(cal.between(0, 48 * H).unwrap().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn repeating_events_appear_and_remind_each_time() {
        let path = std::env::temp_dir().join(format!("xenify-cal-repeat-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let cal = Calendar::open(&path);
        // Daily from day 0, 09:00–09:30 (in this computer's zone, which is
        // UTC in tests unless TZ says otherwise).
        let day = 24 * H;
        let standup = cal
            .save(&Event {
                title: "Stand-up".into(),
                start: 9 * H,
                end: 9 * H + H / 2,
                repeat: "FREQ=DAILY".into(),
                remind: Some(5),
                ..Event::default()
            })
            .unwrap();
        let week = cal.between(0, 7 * day).unwrap();
        assert_eq!(week.len(), 7, "{week:?}");
        assert!(week.iter().all(|e| e.id == standup.id && e.series_start == 9 * H));
        // Due five minutes before day 3's, once.
        let due = cal.due(3 * day + 9 * H - 5 * 60_000).unwrap();
        assert_eq!(due.iter().map(|e| e.start).collect::<Vec<_>>(), [3 * day + 9 * H]);
        cal.mark_reminded(standup.id, due[0].start).unwrap();
        assert!(cal.due(3 * day + 9 * H).unwrap().is_empty());
        assert_eq!(cal.due(4 * day + 9 * H - 60_000).unwrap()[0].start, 4 * day + 9 * H, "the next day's");
        let _ = std::fs::remove_file(&path);
    }
}
