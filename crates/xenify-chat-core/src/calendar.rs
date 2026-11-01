//! Your calendar on this device: events you add, reminders made from a
//! chat message or an email ("Remind me tomorrow"), and meetings you're
//! invited to. The screens draw the Today panel and the week view from it,
//! and show reminders when they're due. Your own events also go to the
//! server (see `meetings`), so your other devices have them; each has a
//! `uid` for that, and counts changes not yet sent.
//!
//! Kept in memory and saved whole by the host (it's small).

use chrono::TimeZone;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use xenify_cal::event::{Event as CalEvent, When};

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
    /// "event", "reminder" or "away" (out of office).
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
    /// The same event on all your devices.
    #[serde(default)]
    pub uid: String,
    /// A meeting's conversation (0: not a meeting). Meetings come from their
    /// conversation, not from your calendar on the server.
    #[serde(default)]
    pub meeting: i64,
    /// Your answer to a meeting: "yes", "maybe" or "" (not yet).
    #[serde(default)]
    pub answer: String,
    /// The shared calendar it's in (0: yours).
    #[serde(default)]
    pub shared: i64,
}

fn event_kind() -> String {
    "event".into()
}

/// An event as kept: with what the screens don't see.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Kept {
    #[serde(flatten)]
    pub event: Event,
    /// Changes not yet on the server.
    #[serde(default)]
    pub dirty: i64,
    /// The latest appearance of a repeating event whose reminder was shown.
    #[serde(default)]
    pub reminded_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Data {
    #[serde(default)]
    pub events: Vec<Kept>,
    #[serde(default)]
    pub next_id: i64,
    /// Deleted here, not yet on the server.
    #[serde(default)]
    pub gone: BTreeSet<String>,
    /// How far each synced conversation has been read, and settings.
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
}

pub struct Calendar {
    data: Mutex<Data>,
    tz: Mutex<Tz>,
    changed: Mutex<bool>,
}

fn new_uid() -> String {
    xenify_sync::random(16).iter().map(|b| format!("{b:02x}")).collect()
}

/// A repeating event's appearances overlapping `from..to` (Unix ms), with
/// the wall-clock time kept across daylight-saving changes.
fn appearances(e: &Event, from: i64, to: i64, tz: Tz) -> Vec<(i64, i64)> {
    let at = |ms: i64| chrono::Utc.timestamp_millis_opt(ms).single().unwrap_or_default();
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

impl Calendar {
    /// From what was kept ("" for none).
    /// Whether `json` (what was kept, "" for none) reads as kept data.
    pub fn readable(json: &str) -> bool {
        json.trim().is_empty() || serde_json::from_str::<Data>(json).is_ok()
    }

    pub fn load(json: &str) -> Calendar {
        Calendar::from_data(serde_json::from_str(json).unwrap_or_default())
    }

    pub fn from_data(data: Data) -> Calendar {
        Calendar { data: Mutex::new(data), tz: Mutex::new(chrono_tz::UTC), changed: Mutex::new(false) }
    }

    pub fn set_tz(&self, tz: Tz) {
        *self.tz.lock().unwrap() = tz;
    }

    pub fn tz(&self) -> Tz {
        *self.tz.lock().unwrap()
    }

    /// What to keep, if anything changed since last asked.
    pub fn changed(&self) -> Option<String> {
        let mut c = self.changed.lock().unwrap();
        if !*c {
            return None;
        }
        *c = false;
        serde_json::to_string(&*self.data.lock().unwrap()).ok()
    }

    fn change<T>(&self, f: impl FnOnce(&mut Data) -> T) -> T {
        let out = f(&mut self.data.lock().unwrap());
        *self.changed.lock().unwrap() = true;
        out
    }

    fn read<T>(&self, f: impl FnOnce(&Data) -> T) -> T {
        f(&self.data.lock().unwrap())
    }

    /// Events (and appearances of repeating ones) overlapping `from`..`to`,
    /// earliest first.
    pub fn between(&self, from: i64, to: i64) -> Result<Vec<Event>, String> {
        let tz = self.tz();
        let mut out = Vec::new();
        self.read(|d| {
            for k in &d.events {
                let e = &k.event;
                if e.repeat.is_empty() {
                    if (e.start < to && e.end > from) || (e.start >= from && e.start < to) {
                        out.push(Event { series_start: e.start, series_end: e.end, ..e.clone() });
                    }
                } else if e.start < to {
                    for (start, end) in appearances(e, from, to, tz) {
                        out.push(Event { start, end, series_start: e.start, series_end: e.end, ..e.clone() });
                    }
                }
            }
        });
        out.sort_by_key(|e| e.start);
        Ok(out)
    }

    /// Reminders due by `now` and not yet shown (for a repeating event, its
    /// latest appearance that's due).
    pub fn due(&self, now: i64) -> Result<Vec<Event>, String> {
        let tz = self.tz();
        let mut out = Vec::new();
        self.read(|d| {
            for k in &d.events {
                let e = &k.event;
                let Some(remind) = e.remind else { continue };
                if e.done {
                    continue;
                }
                if e.repeat.is_empty() {
                    if !e.reminded && e.start - remind * 60_000 <= now && e.end >= now - 86_400_000 {
                        out.push(Event { series_start: e.start, series_end: e.end, ..e.clone() });
                    }
                } else if e.start <= now + 2 * 86_400_000 {
                    let ahead = remind * 60_000;
                    let next = appearances(e, now - 86_400_000, now + ahead + 60_000, tz)
                        .into_iter()
                        .rfind(|(start, _)| *start - ahead <= now && *start > k.reminded_at);
                    if let Some((start, end)) = next {
                        out.push(Event { start, end, series_start: e.start, series_end: e.end, ..e.clone() });
                    }
                }
            }
        });
        out.sort_by_key(|e| e.start);
        Ok(out)
    }

    /// The reminders for the server to show when due: each event's next
    /// one (from a day ago), keyed by the event and when it starts, so
    /// changing either makes a new one. Meetings and events say when they
    /// start; reminders made with "Remind me" don't.
    pub fn reminders(&self, now: i64) -> Vec<xenify_chat_proto::Reminder> {
        let tz = self.tz();
        let from = now - 86_400_000;
        let mut out = Vec::new();
        self.read(|d| {
            for k in &d.events {
                let e = &k.event;
                let Some(remind) = e.remind else { continue };
                if e.done || e.kind == "away" {
                    continue;
                }
                let ahead = remind * 60_000;
                let start = if e.repeat.is_empty() {
                    (!e.reminded && e.start - ahead >= from).then_some(e.start)
                } else if e.start <= now + 60 * 86_400_000 {
                    // Repeating: from the next one (not one already gone by today).
                    let soon = now - 15 * 60_000;
                    appearances(e, soon + ahead, now + ahead + 60 * 86_400_000, tz)
                        .into_iter()
                        .map(|(start, _)| start)
                        .find(|start| *start > k.reminded_at && *start - ahead >= soon)
                } else {
                    None
                };
                let Some(start) = start else { continue };
                let reminder = e.kind == "reminder";
                out.push(xenify_chat_proto::Reminder {
                    key: format!("{}@{start}", if e.uid.is_empty() { e.id.to_string() } else { e.uid.clone() }),
                    at: start - ahead,
                    title: e.title.clone(),
                    body: if reminder { String::new() } else { e.notes.lines().next().unwrap_or("").to_string() },
                    starts: if reminder { 0 } else { start },
                    link: e.link.clone(),
                });
            }
        });
        out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.key.cmp(&b.key)));
        out.truncate(2000);
        out
    }

    /// Reminders about a chat, message or email still to come ("Later"); some
    /// also hide their chat or email until they're due.
    pub fn snoozed(&self, now: i64) -> Result<Vec<Event>, String> {
        let mut out: Vec<Event> = self.read(|d| {
            d.events
                .iter()
                .map(|k| &k.event)
                .filter(|e| (e.hides || (e.kind == "reminder" && !e.link.is_empty())) && !e.done && e.start > now)
                .cloned()
                .collect()
        });
        out.sort_by_key(|e| e.start);
        Ok(out)
    }

    /// The event added from invitation `uid`, if there is one.
    pub fn by_uid(&self, uid: &str) -> Result<Option<Event>, String> {
        if uid.is_empty() {
            return Ok(None);
        }
        Ok(self.read(|d| d.events.iter().find(|k| k.event.ical_uid == uid).map(|k| k.event.clone())))
    }

    /// Adds (`id` 0) or changes an event; returns it as saved. Your own
    /// events are then due to go to the server.
    pub fn save(&self, e: &Event) -> Result<Event, String> {
        self.store(e, true)
    }

    /// Saves an event that came from the server (your other device's), or a
    /// meeting: nothing to send.
    pub fn save_from_server(&self, e: &Event) -> Result<Event, String> {
        self.store(e, false)
    }

    fn store(&self, e: &Event, mine: bool) -> Result<Event, String> {
        if e.title.trim().is_empty() {
            return Err("Give it a name.".into());
        }
        if e.end < e.start {
            return Err("It ends before it starts.".into());
        }
        let counts = mine && e.meeting == 0;
        self.change(|d| {
            let mut ev = Event {
                title: e.title.trim().to_string(),
                repeat: e.repeat.trim().to_string(),
                series_start: 0,
                series_end: 0,
                ..e.clone()
            };
            match d.events.iter_mut().find(|k| e.id != 0 && k.event.id == e.id) {
                Some(k) => {
                    // A new time means the reminder is due again.
                    ev.reminded = e.reminded && k.event.start == e.start;
                    if ev.uid.is_empty() {
                        ev.uid = k.event.uid.clone();
                    }
                    k.event = ev.clone();
                    if counts {
                        k.dirty += 1;
                    }
                    Ok(ev)
                }
                None => {
                    d.next_id = d.next_id.max(d.events.iter().map(|k| k.event.id).max().unwrap_or(0)) + 1;
                    ev.id = d.next_id;
                    if ev.uid.is_empty() && ev.meeting == 0 {
                        ev.uid = new_uid();
                    }
                    d.events.push(Kept { event: ev.clone(), dirty: i64::from(counts), reminded_at: 0 });
                    Ok(ev)
                }
            }
        })
    }

    /// Deletes one of your events (and, later, on the server).
    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.change(|d| {
            if let Some(k) = d.events.iter().find(|k| k.event.id == id)
                && k.event.meeting == 0
                && !k.event.uid.is_empty()
            {
                d.gone.insert(k.event.uid.clone());
            }
            d.events.retain(|k| k.event.id != id);
        });
        Ok(())
    }

    /// The reminder for event `id` (its appearance at `at`, if it repeats)
    /// has been shown.
    pub fn mark_reminded(&self, id: i64, at: i64) -> Result<(), String> {
        self.change(|d| {
            if let Some(k) = d.events.iter_mut().find(|k| k.event.id == id) {
                k.event.reminded = true;
                k.reminded_at = k.reminded_at.max(at);
            }
        });
        Ok(())
    }

    pub fn delete_uid(&self, uid: &str) -> Result<(), String> {
        self.change(|d| {
            for k in
                d.events.iter().filter(|k| k.event.ical_uid == uid && k.event.meeting == 0 && !k.event.uid.is_empty())
            {
                d.gone.insert(k.event.uid.clone());
            }
            d.events.retain(|k| k.event.ical_uid != uid);
        });
        Ok(())
    }

    // Syncing (see `meetings`).

    /// Your events with changes the server hasn't had, and how many changes.
    pub fn unsent(&self) -> Result<Vec<(Event, i64)>, String> {
        Ok(self.read(|d| {
            d.events
                .iter()
                .filter(|k| k.dirty > 0 && k.event.meeting == 0)
                .map(|k| (k.event.clone(), k.dirty))
                .collect()
        }))
    }

    /// The server has the event as it was after `changes` changes.
    pub fn sent(&self, uid: &str, changes: i64) -> Result<(), String> {
        self.change(|d| {
            if let Some(k) = d.events.iter_mut().find(|k| k.event.uid == uid && k.dirty == changes) {
                k.dirty = 0;
            }
        });
        Ok(())
    }

    /// Events deleted here that the server still has.
    pub fn gone(&self) -> Result<Vec<String>, String> {
        Ok(self.read(|d| d.gone.iter().cloned().collect()))
    }

    pub fn gone_sent(&self, uid: &str) -> Result<(), String> {
        self.change(|d| d.gone.remove(uid));
        Ok(())
    }

    /// Your event `uid` as another device of yours saved it (unless it
    /// changed here meanwhile: then this device's goes to the server).
    pub fn put_from_server(&self, e: &Event) -> Result<(), String> {
        let (here, gone) = self.read(|d| {
            (
                d.events.iter().find(|k| k.event.uid == e.uid && k.event.meeting == 0).map(|k| (k.event.id, k.dirty)),
                d.gone.contains(&e.uid),
            )
        });
        if gone {
            return Ok(());
        }
        match here {
            Some((_, dirty)) if dirty > 0 => Ok(()),
            Some((id, _)) => self.save_from_server(&Event { id, meeting: 0, ..e.clone() }).map(|_| ()),
            None => self.save_from_server(&Event { id: 0, meeting: 0, ..e.clone() }).map(|_| ()),
        }
    }

    pub fn delete_from_server(&self, uid: &str) -> Result<(), String> {
        self.change(|d| d.events.retain(|k| !(k.event.uid == uid && k.event.meeting == 0)));
        Ok(())
    }

    /// The meeting held in conversation `conv`, if it's in your calendar.
    pub fn meeting(&self, conv: i64) -> Result<Option<Event>, String> {
        Ok(self.read(|d| d.events.iter().find(|k| k.event.meeting == conv).map(|k| k.event.clone())))
    }

    /// Puts a meeting in your calendar or changes it (keeping your reminder).
    pub fn put_meeting(&self, e: &Event) -> Result<(), String> {
        let was = self.meeting(e.meeting)?;
        let e = match was {
            Some(w) => Event { id: w.id, remind: w.remind, reminded: w.reminded && w.start == e.start, ..e.clone() },
            None => e.clone(),
        };
        self.save_from_server(&e).map(|_| ())
    }

    /// When a meeting reminds you (minutes before; None: not at all).
    pub fn set_meeting_remind(&self, conv: i64, remind: Option<i64>) -> Result<(), String> {
        self.change(|d| {
            for k in d.events.iter_mut().filter(|k| k.event.meeting == conv) {
                k.event.remind = remind;
                k.event.reminded = false;
            }
        });
        Ok(())
    }

    pub fn drop_meeting(&self, conv: i64) -> Result<(), String> {
        self.change(|d| d.events.retain(|k| k.event.meeting != conv));
        Ok(())
    }

    /// Your times away (out of office), as saved here.
    pub fn away_events(&self) -> Result<Vec<Event>, String> {
        Ok(self.read(|d| d.events.iter().filter(|k| k.event.kind == "away").map(|k| k.event.clone()).collect()))
    }

    /// When you're busy between `from` and `to` (Unix ms), as merged
    /// stretches: events and meetings you haven't said no to, and times
    /// away; not all-day events or reminders.
    pub fn busy(&self, from: i64, to: i64) -> Result<Vec<(i64, i64)>, String> {
        let mut spans: Vec<(i64, i64)> = self
            .between(from, to)?
            .into_iter()
            .filter(|e| ((e.kind == "event" && !e.all_day) || e.kind == "away") && e.end > e.start)
            .map(|e| (e.start.max(from), e.end.min(to)))
            .collect();
        spans.sort();
        let mut out: Vec<(i64, i64)> = Vec::new();
        for (s, e) in spans {
            match out.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => out.push((s, e)),
            }
        }
        Ok(out)
    }

    pub fn get_meta(&self, key: &str) -> Option<String> {
        self.read(|d| d.meta.get(key).cloned())
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        self.change(|d| d.meta.insert(key.to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    #[test]
    fn the_server_gets_each_events_next_reminder() {
        let cal = Calendar::load("");
        let lunch = cal
            .save(&Event {
                title: "Lunch".into(),
                start: 30 * H,
                end: 31 * H,
                remind: Some(10),
                notes: "Café\nUpstairs".into(),
                ..Event::default()
            })
            .unwrap();
        cal.save(&Event { title: "Old".into(), start: H, end: H, remind: Some(0), ..Event::default() }).unwrap();
        cal.save(&Event { title: "No reminder".into(), start: 40 * H, end: 41 * H, ..Event::default() }).unwrap();
        cal.save(&Event {
            title: "Stand-up".into(),
            start: 9 * H,
            end: 9 * H + H / 4,
            remind: Some(5),
            repeat: "FREQ=DAILY".into(),
            ..Event::default()
        })
        .unwrap();
        cal.save(&Event {
            title: "Call Bo".into(),
            start: 50 * H,
            end: 50 * H,
            kind: "reminder".into(),
            remind: Some(0),
            ..Event::default()
        })
        .unwrap();
        let list = cal.reminders(28 * H);
        let titles: Vec<&str> = list.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Lunch", "Stand-up", "Call Bo"], "{list:?}");
        assert_eq!((list[0].at, list[0].starts, list[0].body.as_str()), (30 * H - 10 * 60_000, 30 * H, "Café"));
        assert_eq!(list[0].key, format!("{}@{}", lunch.uid, 30 * H));
        // The daily one's next time; the plain reminder doesn't say when anything starts.
        assert_eq!(list[1].starts, 33 * H);
        assert_eq!(list[2].starts, 0);
    }

    #[test]
    fn events_and_reminders() {
        let cal = Calendar::load("");
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
        assert_eq!(cal.gone().unwrap(), [moved.uid], "the server hears it's gone");
        // Kept whole and read back.
        let again = Calendar::load(&cal.changed().unwrap());
        assert_eq!(again.between(0, 48 * H).unwrap().len(), 1);
        assert!(cal.changed().is_none(), "nothing new to keep");
    }

    #[test]
    fn repeating_events_appear_and_remind_each_time() {
        let cal = Calendar::load("");
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
        let due = cal.due(3 * day + 9 * H - 5 * 60_000).unwrap();
        assert_eq!(due.iter().map(|e| e.start).collect::<Vec<_>>(), [3 * day + 9 * H]);
        cal.mark_reminded(standup.id, due[0].start).unwrap();
        assert!(cal.due(3 * day + 9 * H).unwrap().is_empty());
        assert_eq!(cal.due(4 * day + 9 * H - 60_000).unwrap()[0].start, 4 * day + 9 * H, "the next day's");
    }

    #[test]
    fn changes_sync_once() {
        let cal = Calendar::load("");
        let e = cal.save(&Event { title: "Dentist".into(), start: H, end: 2 * H, ..Event::default() }).unwrap();
        let (_, n) = cal.unsent().unwrap()[0].clone();
        // Changed again before the server had the first: still due.
        cal.save(&Event { title: "Dentist (moved)".into(), ..e.clone() }).unwrap();
        cal.sent(&e.uid, n).unwrap();
        assert_eq!(cal.unsent().unwrap().len(), 1);
        // Another device's copy doesn't overwrite a change not yet sent.
        cal.put_from_server(&Event { title: "Theirs".into(), ..e.clone() }).unwrap();
        assert_eq!(cal.between(0, 3 * H).unwrap()[0].title, "Dentist (moved)");
    }
}
