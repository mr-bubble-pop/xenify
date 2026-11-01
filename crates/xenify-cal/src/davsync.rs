//! Two-way sync of one CalDAV calendar with its copy in the store:
//! deletions and changes made here go up, then the server's changes come
//! down. When both sides changed an event, the server's version wins (the
//! report names it).

use crate::caldav::{Dav, DavError, RemoteCalendar};
use crate::store::Store;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub pulled: usize,
    pub pushed: usize,
    pub deleted: usize,
    /// Events changed in both places (the server's version was kept).
    pub conflicts: Vec<String>,
}

/// Where a new event goes on the server.
fn new_href(calendar_url: &str, uid: &str) -> String {
    let safe: String =
        uid.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    format!("{}/{safe}.ics", calendar_url.trim_end_matches('/'))
}

pub fn sync(dav: &Dav, store: &mut Store, calendar: i64, url: &str) -> Result<Report, DavError> {
    let db = |e: rusqlite::Error| DavError::Bad(format!("calendar store: {e}"));
    let remote = RemoteCalendar { url: url.to_string(), ..Default::default() };
    let mut report = Report::default();

    // 1. Deletions made here.
    for (href, etag) in store.removed(calendar).map_err(db)? {
        match dav.delete(&href, (!etag.is_empty()).then_some(etag.as_str())) {
            // Changed on the server since: it comes back with the pull.
            Ok(()) | Err(DavError::Conflict) => {}
            Err(e) => return Err(e),
        }
        store.clear_removed(calendar, &href).map_err(db)?;
        report.deleted += 1;
    }

    // 2. Changes made here.
    for crate::store::Pending { uid, href, etag, events } in store.dirty(calendar).map_err(db)? {
        if events.is_empty() {
            continue;
        }
        let href = if href.is_empty() { new_href(url, &uid) } else { href };
        let text = crate::event::calendar(&events, None);
        match dav.put(&href, &text, (!etag.is_empty()).then_some(etag.as_str())) {
            Ok(new_etag) => {
                // Without an ETag in the reply, the pull fetches it again.
                store.mark_clean(calendar, &uid, &href, new_etag.as_deref().unwrap_or("")).map_err(db)?;
                report.pushed += 1;
            }
            Err(DavError::Conflict) => {
                report.conflicts.push(events[0].summary.clone());
                // Take the server's version below.
                store.mark_clean(calendar, &uid, &href, "").map_err(db)?;
            }
            Err(e) => return Err(e),
        }
    }

    // 3. The server's changes.
    let token = store.sync_state(calendar).map_err(db)?;
    let changes = match dav.changes(&remote, &token) {
        Err(DavError::TokenExpired) => dav.changes(&remote, "")?,
        other => other?,
    };
    let known: std::collections::HashMap<String, String> =
        store.remote_items(calendar).map_err(db)?.into_iter().collect();
    let wanted: Vec<String> =
        changes.changed.iter().filter(|i| known.get(&i.url) != Some(&i.etag)).map(|i| i.url.clone()).collect();
    for chunk in wanted.chunks(50) {
        for (href, etag, text) in dav.get(&remote, chunk)? {
            store.apply_remote(calendar, &href, &etag, &text).map_err(db)?;
            report.pulled += 1;
        }
    }
    for href in &changes.removed {
        store.remove_href(calendar, href).map_err(db)?;
    }
    // A full listing (first sync, or no sync support): anything not on the
    // server any more is gone.
    if token.is_empty() || changes.token.is_empty() {
        let on_server: std::collections::HashSet<&String> = changes.changed.iter().map(|i| &i.url).collect();
        for (href, _) in store.remote_items(calendar).map_err(db)? {
            if !on_server.contains(&href) {
                store.remove_href(calendar, &href).map_err(db)?;
            }
        }
    }
    store.set_sync_state(calendar, &changes.token).map_err(db)?;
    Ok(report)
}
