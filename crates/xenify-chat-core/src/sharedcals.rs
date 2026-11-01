//! Shared calendars: a team's, a project's or a rota, that several people
//! see. Each is a channel the chat list doesn't show (`SHARED_CAL`): its
//! owners can change it, everyone else in it can look. Each change is a
//! post (`~cal1:`, as in your own calendar), its name `~calname1:`. The app
//! reads one whole when it connects and keeps it in memory; its events don't
//! make you busy or remind you.

use crate::calendar::{Calendar, Event};
use crate::meetings::{Inner, Sharing, TAG, share_posts};
use crate::{Core, Host};
use serde::{Deserialize, Serialize};
use xenify_chat_proto::{Conversation, Message, Role, SHARED_CAL, ToServer, User};

const CAL: &str = "~cal1:";
const NAME: &str = "~calname1:";
const PAGE: u32 = 200;

pub(crate) struct Shared {
    name: String,
    cal: Calendar,
    /// Posts gathered while reading it the first time.
    loading: Vec<Message>,
    loaded: bool,
    seen: i64,
}

#[derive(Serialize, Deserialize)]
struct Post {
    /// "put" or "delete".
    op: String,
    uid: String,
    #[serde(default)]
    event: Option<Event>,
}

#[derive(Clone, Serialize)]
struct Person {
    user: User,
    owner: bool,
}

/// A shared calendar for the screens.
#[derive(Clone, Serialize)]
pub struct SharedOut {
    conv: i64,
    name: String,
    /// You can change it.
    yours: bool,
    people: Vec<Person>,
    member_count: i64,
}

fn random_id() -> String {
    xenify_sync::random(8).iter().map(|b| format!("{b:02x}")).collect()
}

fn post(conv: i64, text: String) -> ToServer {
    ToServer::Send { conv, client_id: random_id(), text }
}

fn name_post(conv: i64, name: &str) -> ToServer {
    post(conv, format!("{NAME}{}", serde_json::json!({ "name": name })))
}

/// A shared calendar arrived (in the list, made, or changed): one you just
/// made gets its name and people; one new here is read.
pub(crate) fn note<H: Host>(app: &Core<H>, inner: &mut Inner, c: &Conversation) -> Vec<ToServer> {
    let mut send = Vec::new();
    if let std::collections::hash_map::Entry::Vacant(slot) = inner.shared.entry(c.id) {
        let cal = Calendar::load("");
        cal.set_tz(app.cal.tz());
        let name = if c.title.is_empty() { "Shared calendar".to_string() } else { c.title.clone() };
        slot.insert(Shared { name, cal, loading: Vec::new(), loaded: false, seen: 0 });
        if c.role == Role::Owner
            && let Some(i) = inner.pending_cals.iter().position(|(t, _)| *t == c.title)
        {
            let (name, people) = inner.pending_cals.remove(i);
            send.push(name_post(c.id, &name));
            send.extend(share_posts(c.id, &people));
            inner.shared.get_mut(&c.id).expect("just put").loaded = true;
        } else {
            send.push(ToServer::History { conv: c.id, before: None, limit: PAGE, tag: TAG.into() });
        }
    }
    app.emit("calendar", ());
    send
}

/// Posts in a shared calendar: pages read here (older ones next, until
/// the first), or new ones as they come.
pub(crate) fn posts<H: Host>(
    app: &Core<H>,
    inner: &mut Inner,
    conv: i64,
    list: Vec<Message>,
    asked_here: bool,
) -> Vec<ToServer> {
    let mut send = Vec::new();
    let owners: Vec<i64> = inner.hidden.get(&conv).map(|c| c.owners.clone()).unwrap_or_default();
    let Some(s) = inner.shared.get_mut(&conv) else { return send };
    if !s.loaded {
        let first = list.iter().map(|m| m.seq).min();
        let full = list.len() as u32 == PAGE;
        s.loading.extend(list);
        if asked_here {
            match first {
                Some(first) if full && first > 1 => {
                    send.push(ToServer::History { conv, before: Some(first), limit: PAGE, tag: TAG.into() })
                }
                _ => {
                    let all = std::mem::take(&mut s.loading);
                    s.loaded = true;
                    apply(s, &owners, all);
                    app.emit("calendar", ());
                }
            }
        }
        return send;
    }
    if apply(s, &owners, list) {
        app.emit("calendar", ());
    }
    send
}

/// Changes posted by its owners, in order; whether anything changed.
fn apply(s: &mut Shared, owners: &[i64], mut list: Vec<Message>) -> bool {
    list.sort_by_key(|m| m.seq);
    let mut changed = false;
    for m in list {
        if m.seq <= s.seen {
            continue;
        }
        s.seen = m.seq;
        if m.deleted || !owners.contains(&m.sender.id) {
            continue;
        }
        if let Some(json) = m.text.strip_prefix(NAME) {
            if let Some(name) =
                serde_json::from_str::<serde_json::Value>(json).ok().and_then(|v| v["name"].as_str().map(String::from))
            {
                s.name = name;
                changed = true;
            }
        } else if let Some(p) = m.text.strip_prefix(CAL).and_then(|j| serde_json::from_str::<Post>(j).ok()) {
            changed = true;
            match (p.op.as_str(), p.event) {
                ("put", Some(e)) => {
                    let _ = s.cal.put_from_server(&Event { uid: p.uid, meeting: 0, remind: None, ..e });
                }
                ("delete", _) => {
                    let _ = s.cal.delete_from_server(&p.uid);
                }
                _ => {}
            }
        }
    }
    changed
}

pub fn list<H: Host>(app: &Core<H>) -> Vec<SharedOut> {
    let inner = app.inner.lock().unwrap();
    let mut out: Vec<SharedOut> = inner
        .shared
        .iter()
        .filter_map(|(conv, s)| {
            let c = inner.hidden.get(conv)?;
            Some(SharedOut {
                conv: *conv,
                name: s.name.clone(),
                yours: c.role == Role::Owner,
                people: c
                    .members
                    .iter()
                    .filter(|u| !u.app)
                    .map(|u| Person { user: u.clone(), owner: c.owners.contains(&u.id) })
                    .collect(),
                member_count: c.member_count,
            })
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Events in every shared calendar between `from` and `to`, each saying
/// which calendar it's in (`shared`), with ids of their own (below 0).
pub fn between<H: Host>(app: &Core<H>, from: i64, to: i64) -> Result<Vec<Event>, String> {
    let inner = app.inner.lock().unwrap();
    let mut out = Vec::new();
    for (conv, s) in &inner.shared {
        for e in s.cal.between(from, to)? {
            out.push(Event { id: -(conv * 1_000_000 + e.id), shared: *conv, remind: None, ..e });
        }
    }
    out.sort_by_key(|e| e.start);
    Ok(out)
}

pub fn new<H: Host>(app: &Core<H>, name: String, people: Vec<Sharing>) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give it a name.".into());
    }
    let mut inner = app.inner.lock().unwrap();
    if inner.me.is_none() {
        return Err("Connect to the server to share a calendar.".into());
    }
    // Found again by its title when the server answers.
    inner.pending_cals.push((name.clone(), people));
    app.tell(ToServer::CreateChannel { title: name, open: false, purpose: SHARED_CAL.into() });
    Ok(())
}

fn owned(inner: &Inner, conv: i64) -> Result<(), String> {
    match inner.hidden.get(&conv) {
        Some(c) if c.purpose == SHARED_CAL && c.role == Role::Owner => Ok(()),
        _ => Err("Only people who can change this calendar can do that.".into()),
    }
}

/// Shares a calendar you can change with more people, takes people out, and
/// says who can change it.
pub fn share<H: Host>(
    app: &Core<H>,
    conv: i64,
    add: Vec<Sharing>,
    remove: Vec<String>,
    roles: Vec<Sharing>,
) -> Result<(), String> {
    owned(&app.inner.lock().unwrap(), conv)?;
    for m in share_posts(conv, &add) {
        app.tell(m);
    }
    for user in remove {
        app.tell(ToServer::RemovePerson { conv, user });
    }
    for r in roles {
        app.tell(ToServer::SetRole { conv, user: r.username, owner: r.owner });
    }
    Ok(())
}

pub fn rename<H: Host>(app: &Core<H>, conv: i64, name: String) -> Result<(), String> {
    let mut inner = app.inner.lock().unwrap();
    owned(&inner, conv)?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give it a name.".into());
    }
    if let Some(s) = inner.shared.get_mut(&conv) {
        s.name = name.clone();
    }
    app.tell(name_post(conv, &name));
    app.emit("calendar", ());
    Ok(())
}

/// Stops seeing a shared calendar.
pub fn leave<H: Host>(app: &Core<H>, conv: i64) {
    app.inner.lock().unwrap().shared.remove(&conv);
    app.tell(ToServer::Leave { conv });
    app.emit("calendar", ());
}

/// Deletes a shared calendar you can change: everyone else is taken out,
/// then you leave, and the server lets it go with its last person.
pub fn remove<H: Host>(app: &Core<H>, conv: i64) -> Result<(), String> {
    let mut inner = app.inner.lock().unwrap();
    owned(&inner, conv)?;
    let me = inner.me.as_ref().map(|u| u.id);
    let others: Vec<String> = inner
        .hidden
        .get(&conv)
        .map(|c| c.members.iter().filter(|u| Some(u.id) != me).map(|u| u.username.clone()).collect())
        .unwrap_or_default();
    inner.shared.remove(&conv);
    drop(inner);
    for user in others {
        app.tell(ToServer::RemovePerson { conv, user });
    }
    app.tell(ToServer::Leave { conv });
    app.emit("calendar", ());
    Ok(())
}

/// Adds or changes an event in a shared calendar you can change.
pub fn save<H: Host>(app: &Core<H>, conv: i64, event: Event) -> Result<Event, String> {
    let inner = app.inner.lock().unwrap();
    owned(&inner, conv)?;
    let s = inner.shared.get(&conv).ok_or("That calendar isn't here yet.")?;
    let uid = if event.uid.is_empty() { random_id() } else { event.uid.clone() };
    let e = Event {
        id: 0,
        uid: uid.clone(),
        shared: 0,
        remind: None,
        reminded: false,
        meeting: 0,
        answer: String::new(),
        series_start: 0,
        series_end: 0,
        ..event
    };
    if e.title.trim().is_empty() {
        return Err("Give it a name.".into());
    }
    if e.end < e.start {
        return Err("It ends before it starts.".into());
    }
    // Shown straight away; the server's copy comes back the same.
    s.cal.put_from_server(&e)?;
    let p = Post { op: "put".into(), uid, event: Some(e.clone()) };
    app.tell(post(conv, format!("{CAL}{}", serde_json::to_string(&p).unwrap_or_default())));
    app.emit("calendar", ());
    Ok(Event { shared: conv, ..e })
}

pub fn delete<H: Host>(app: &Core<H>, conv: i64, uid: String) -> Result<(), String> {
    let inner = app.inner.lock().unwrap();
    owned(&inner, conv)?;
    if let Some(s) = inner.shared.get(&conv) {
        s.cal.delete_from_server(&uid)?;
    }
    let p = Post { op: "delete".into(), uid, event: None };
    app.tell(post(conv, format!("{CAL}{}", serde_json::to_string(&p).unwrap_or_default())));
    app.emit("calendar", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::incoming;
    use serde_json::json;
    use std::sync::Mutex;
    use xenify_chat_proto::ToApp;

    #[derive(Default)]
    struct Fake {
        told: Mutex<Vec<ToServer>>,
        saved: Mutex<Vec<(String, String)>>,
    }

    impl Host for Fake {
        fn emit(&self, _: &str, _: serde_json::Value) {}
        fn tell(&self, m: ToServer) {
            self.told.lock().unwrap().push(m);
        }
        fn save(&self, what: &str, json: String) {
            self.saved.lock().unwrap().push((what.into(), json));
        }
        fn now(&self) -> i64 {
            1_800_000_000_000
        }
        fn tz(&self) -> chrono_tz::Tz {
            chrono_tz::UTC
        }
        fn server(&self) -> String {
            "test".into()
        }
    }

    fn user(id: i64, name: &str) -> User {
        User { id, username: name.into(), ..Default::default() }
    }

    fn signed_in(id: i64, name: &str) -> Core<Fake> {
        let app = Core::new(Fake::default(), "", "");
        let welcome = json!({ "t": "welcome", "user": user(id, name), "version": 4 });
        incoming(&app, serde_json::from_value::<ToApp>(welcome).unwrap());
        app
    }

    #[test]
    fn an_unreadable_calendar_is_kept_aside_before_starting_empty() {
        let bad = r#"{"events": "from a newer version"}"#;
        let ok = r#"{"contacts": [], "a_newer_field": 1}"#;
        let app = Core::new(Fake::default(), bad, ok);
        let saved = app.host.saved.lock().unwrap().clone();
        assert_eq!(saved, vec![(format!("calendar-unreadable-{}", 1_800_000_000_000i64), bad.to_string())]);
        assert!(Core::new(Fake::default(), "", "").host.saved.lock().unwrap().is_empty());
    }

    fn told(app: &Core<Fake>) -> Vec<ToServer> {
        std::mem::take(&mut *app.host.told.lock().unwrap())
    }

    fn rota(role: &str) -> ToApp {
        serde_json::from_value(json!({ "t": "conversation", "conv": {
            "id": 10, "kind": "channel", "title": "Rota", "members": [user(1, "ana"), user(2, "bo")], "member_count": 2,
            "owners": [1], "role": role, "can_post": role == "owner", "open": false, "last": null, "last_seq": 3,
            "read_seq": 0, "notify": "all", "muted_until": null, "purpose": SHARED_CAL,
        }}))
        .unwrap()
    }

    fn message(seq: i64, from: User, text: String) -> Message {
        Message {
            id: seq,
            conv: 10,
            seq,
            sender: from,
            text,
            at: 0,
            client_id: String::new(),
            edited: false,
            deleted: false,
            reactions: Vec::new(),
            thread: None,
            replies: 0,
            last_reply_seq: 0,
            last_reply_at: 0,
            thread_people: Vec::new(),
        }
    }

    #[test]
    fn owners_change_a_shared_calendar_and_everyone_in_it_sees_it() {
        // Ana makes "Rota", shared with Bo (who can look).
        let ana = signed_in(1, "ana");
        new(&ana, "Rota".into(), vec![Sharing { username: "bo".into(), owner: false }]).unwrap();
        assert!(matches!(&told(&ana)[..], [ToServer::CreateChannel { purpose, .. }] if purpose == SHARED_CAL));
        incoming(&ana, rota("owner"));
        let sent = told(&ana);
        assert!(sent.iter().any(|m| matches!(m, ToServer::Send { text, .. } if text.starts_with(NAME))));
        assert!(sent.iter().any(|m| matches!(m, ToServer::AddPeople { users, .. } if users == &["bo"])));
        let shift = Event {
            title: "Early shift".into(),
            start: 1_800_000_000_000,
            end: 1_800_003_600_000,
            ..Default::default()
        };
        let saved = save(&ana, 10, shift).unwrap();
        assert_eq!(saved.shared, 10);
        let posted = told(&ana)
            .into_iter()
            .find_map(|m| match m {
                ToServer::Send { text, .. } if text.starts_with(CAL) => Some(text),
                _ => None,
            })
            .unwrap();
        assert_eq!(between(&ana, 1_799_000_000_000, 1_801_000_000_000).unwrap().len(), 1);

        // Bo reads it when it arrives; Bo's own posts in it don't count.
        let bo = signed_in(2, "bo");
        incoming(&bo, rota("member"));
        assert!(told(&bo).iter().any(|m| matches!(m, ToServer::History { conv: 10, .. })));
        let forged = format!(
            "{CAL}{}",
            json!({ "op": "put", "uid": "x", "event": { "title": "Bo's day off", "start": 1_800_000_000_000i64, "end": 1_800_000_100_000i64 } })
        );
        let history = ToApp::History {
            conv: 10,
            messages: vec![
                message(1, user(1, "ana"), format!("{NAME}{}", json!({ "name": "Weekend rota" }))),
                message(2, user(1, "ana"), posted),
                message(3, user(2, "bo"), forged),
            ],
            thread: None,
            tag: TAG.into(),
        };
        incoming(&bo, history);
        let seen = between(&bo, 1_799_000_000_000, 1_801_000_000_000).unwrap();
        assert_eq!(seen.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Early shift"]);
        assert_eq!(list(&bo)[0].name, "Weekend rota");
        assert!(!list(&bo)[0].yours);
        assert!(save(&bo, 10, Event { title: "Mine".into(), ..Default::default() }).is_err());

        // A deletion by an owner reaches Bo live.
        let gone = format!("{CAL}{}", json!({ "op": "delete", "uid": seen[0].uid }));
        incoming(&bo, ToApp::Message { msg: message(4, user(1, "ana"), gone) });
        assert!(between(&bo, 1_799_000_000_000, 1_801_000_000_000).unwrap().is_empty());
        // Leaving: it's gone from here.
        incoming(&bo, ToApp::Removed { conv: 10 });
        assert!(list(&bo).is_empty());
    }

    #[test]
    fn an_owner_deletes_a_shared_calendar_for_everyone() {
        let ana = signed_in(1, "ana");
        incoming(&ana, rota("owner"));
        told(&ana);
        // Only someone who can change it may; Bo only looks.
        let bo = signed_in(2, "bo");
        incoming(&bo, rota("member"));
        assert!(remove(&bo, 10).is_err());
        // Everyone else is taken out, then Ana leaves (the last one out
        // takes it with them).
        remove(&ana, 10).unwrap();
        let sent = told(&ana);
        assert!(
            matches!(&sent[..], [ToServer::RemovePerson { conv: 10, user }, ToServer::Leave { conv: 10 }] if user == "bo")
        );
        assert!(list(&ana).is_empty());
    }
}
