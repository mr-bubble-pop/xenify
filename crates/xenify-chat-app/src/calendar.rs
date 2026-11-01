//! This computer's time zone, and moving the calendar and address books
//! from the SQLite files earlier versions kept to the core's own files (see
//! `host`). The calendar itself is `xenify_chat_core::calendar`.

use chrono_tz::Tz;
use rusqlite::Connection;
use xenify_chat_core::calendar::{self, Event, Kept};
use xenify_chat_core::contacts::{self, Contact};

/// This computer's time zone (UTC if it can't be told).
pub fn local_tz() -> Tz {
    iana_time_zone::get_timezone().ok().and_then(|n| n.parse().ok()).unwrap_or(chrono_tz::UTC)
}

/// The calendar earlier versions kept in `calendar.sqlite`, if there is one.
pub fn migrate() -> Option<calendar::Data> {
    let path = xenify_wp_check::data_dir().join("calendar.sqlite");
    if !path.exists() {
        return None;
    }
    let c = Connection::open(path).ok()?;
    let has = |col: &str| c.prepare(&format!("SELECT {col} FROM events LIMIT 0")).is_ok();
    let col = |name: &'static str, or: &'static str| if has(name) { name } else { or };
    let sql = format!(
        "SELECT id, title, start, end_at, all_day, notes, kind, link, remind, reminded, done, {}, {}, {}, {}, {}, {}, {}, {} FROM events",
        col("repeat", "''"),
        col("ical_uid", "''"),
        col("hides", "0"),
        col("reminded_at", "0"),
        col("uid", "''"),
        col("dirty", "1"),
        col("meeting", "0"),
        col("answer", "''"),
    );
    let mut st = c.prepare(&sql).ok()?;
    let events: Vec<Kept> = st
        .query_map([], |r| {
            let uid: String = r.get(15)?;
            Ok(Kept {
                event: Event {
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
                    uid: if uid.is_empty() {
                        xenify_sync::random(16).iter().map(|b| format!("{b:02x}")).collect()
                    } else {
                        uid
                    },
                    meeting: r.get(17)?,
                    answer: r.get(18)?,
                    ..Event::default()
                },
                reminded_at: r.get(14)?,
                dirty: r.get(16)?,
            })
        })
        .ok()?
        .filter_map(Result::ok)
        .collect();
    let mut data =
        calendar::Data { next_id: events.iter().map(|k| k.event.id).max().unwrap_or(0), events, ..Default::default() };
    if let Ok(mut st) = c.prepare("SELECT uid FROM gone") {
        data.gone =
            st.query_map([], |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default();
    }
    if let Ok(mut st) = c.prepare("SELECT key, value FROM meta") {
        data.meta = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
    }
    Some(data)
}

/// The address books earlier versions kept in `contacts.sqlite`, if any.
pub fn migrate_contacts() -> Option<contacts::Data> {
    let path = xenify_wp_check::data_dir().join("contacts.sqlite");
    if !path.exists() {
        return None;
    }
    let c = Connection::open(path).ok()?;
    let mut st = c
        .prepare("SELECT book, uid, name, email, phone, company, role, notes, username, added, dirty FROM contacts")
        .ok()?;
    let list = st
        .query_map([], |r| {
            Ok(contacts::Kept {
                contact: Contact {
                    book: r.get(0)?,
                    uid: r.get(1)?,
                    name: r.get(2)?,
                    email: r.get(3)?,
                    phone: r.get(4)?,
                    company: r.get(5)?,
                    role: r.get(6)?,
                    notes: r.get(7)?,
                    username: r.get(8)?,
                    added: r.get(9)?,
                },
                dirty: r.get(10)?,
            })
        })
        .ok()?
        .filter_map(Result::ok)
        .collect();
    let mut data = contacts::Data { contacts: list, ..Default::default() };
    if let Ok(mut st) = c.prepare("SELECT conv, name, COALESCE(sharing, '') FROM books") {
        data.books = st
            .query_map([], |r| Ok((r.get(0)?, contacts::Book { name: r.get(1)?, sharing: r.get(2)? })))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
    }
    if let Ok(mut st) = c.prepare("SELECT book, uid FROM contacts_gone") {
        data.gone = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
    }
    Some(data)
}
