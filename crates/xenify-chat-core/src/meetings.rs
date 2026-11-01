//! Meetings, your calendar on the server, and busy times, each kept in a
//! conversation the chat list doesn't show (`Conversation::purpose`):
//!
//! - A meeting is a group. Its owner (the organiser) posts the details
//!   (`~meet1:` and JSON), everyone invited answers in it (`~answer1:yes`),
//!   and its call is the meeting's call.
//! - Your calendar is a group of one. Each change to one of your events is a
//!   post (`~cal1:`), which your other computers read and apply.
//! - Your busy times are a channel (`~busy1:`), followed by whoever may see
//!   them (Settings › Calendar › Who can see when you're busy). Others' apps
//!   download them for Find a time.
//!
//! - Address books work like the calendar: My contacts is a group of one,
//!   a shared book a channel whose owners can change it (`~contact1:`, and
//!   its name in `~bookname1:`).
//!
//! People invited by email get an ordinary invitation from your email
//! account instead.

use crate::calendar::{Calendar, Event};
use crate::contacts::Contact;
use crate::{Core, Host};
use chrono::{Datelike, NaiveTime, TimeZone};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use xenify_chat_proto::{
    Away, BOOK, BUSY, CALENDAR, CONTACTS, Conversation, Kind, MEETING, Message, Role, SHARED_CAL, ToApp, ToServer, User,
};

const MEET: &str = "~meet1:";
const ANSWER: &str = "~answer1:";
const CAL: &str = "~cal1:";
const BUSY_POST: &str = "~busy1:";
const CONTACT: &str = "~contact1:";
const BOOK_NAME: &str = "~bookname1:";
const BOOK_SHARE: &str = "~bookshare1:";
/// A file sent in a conversation (the screens' own kind of post).
const FILE: &str = "~file:";
/// Marks history asked for here (the answers aren't for the screens).
pub(crate) const TAG: &str = "meetings";
const DAY: i64 = 86_400_000;
/// How far ahead your busy times are shared.
const AHEAD: i64 = 60 * DAY;
/// Calendar posts read per request while catching up.
const PAGE: u32 = 200;

/// Someone invited by email.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Guest {
    pub email: String,
    #[serde(default)]
    pub name: String,
    /// "yes", "maybe", "no" or "" (not yet), from their email reply.
    #[serde(default)]
    pub answer: String,
}

/// A meeting as its organiser describes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Details {
    pub title: String,
    /// Unix ms.
    pub start: i64,
    pub end: i64,
    /// Where, or empty for a Xenify call.
    #[serde(default)]
    pub place: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub guests: Vec<Guest>,
    #[serde(default)]
    pub cancelled: bool,
    /// The invitation's id for email calendars, and how often it changed.
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub sequence: u32,
    /// The email account invitations go from.
    #[serde(default)]
    pub mail_from: String,
    /// Rooms and things booked with it.
    #[serde(default)]
    pub things: Vec<ThingRef>,
    /// A Xenify call as well as a place or room.
    #[serde(default)]
    pub also_call: bool,
}

/// A room or thing booked with a meeting.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThingRef {
    pub id: i64,
    pub name: String,
}

/// Books a meeting's rooms and things (an error comes back if one's taken).
fn book_things<H: Host>(app: &Core<H>, conv: i64, d: &Details) {
    for t in &d.things {
        app.tell(ToServer::Book {
            thing: t.id,
            start: d.start,
            end: d.end,
            meeting: Some(conv),
            tag: format!("meeting:{conv}"),
        });
    }
}

#[derive(Clone, Default)]
struct Meeting {
    details: Option<Details>,
    details_seq: i64,
    /// Person → (answer, the post's seq).
    answers: HashMap<i64, (String, i64)>,
}

#[derive(Clone, Serialize)]
struct Answered {
    user: User,
    answer: String,
}

/// A meeting for the screens.
#[derive(Clone, Serialize)]
pub struct MeetingOut {
    conv: i64,
    #[serde(flatten)]
    details: Details,
    organiser: Option<User>,
    /// Everyone on the server invited (organiser first), with their answers.
    people: Vec<Answered>,
    /// Your answer ("" not yet).
    mine: String,
    /// You organise it.
    yours: bool,
}

/// Someone's busy times for Find a time.
#[derive(Clone, Serialize)]
pub struct BusyOut {
    user: i64,
    /// False: they don't share them with you (or have none to share yet).
    known: bool,
    busy: Vec<(i64, i64)>,
    /// Their working hours, as stretches of time.
    work: Vec<(i64, i64)>,
    /// When they're away (out of office).
    away: Vec<(i64, i64)>,
}

#[derive(Default, Serialize, Deserialize, PartialEq)]
struct BusyPost {
    from: i64,
    to: i64,
    busy: Vec<(i64, i64)>,
    work: Vec<(i64, i64)>,
    #[serde(default)]
    away: Vec<(i64, i64)>,
}

#[derive(Serialize, Deserialize)]
struct ContactPost {
    /// "put" or "delete".
    op: String,
    uid: String,
    #[serde(default)]
    contact: Option<Contact>,
}

/// Someone a shared book is shared with.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sharing {
    pub username: String,
    /// Can change it (else only look).
    pub owner: bool,
}

/// A group chat a shared book is shared with: everyone in it, now and
/// later, gets the book.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupShare {
    pub conv: i64,
    /// Its people can change the book (else only look).
    pub owner: bool,
}

/// Who a shared book is shared with, as its owners last said.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct BookSharing {
    #[serde(default)]
    groups: Vec<GroupShare>,
    /// People shared with one by one (user names).
    #[serde(default)]
    people: Vec<String>,
}

#[derive(Clone, Serialize)]
struct SharedGroup {
    conv: i64,
    title: String,
    count: i64,
    owner: bool,
}

/// A shared address book for the screens.
#[derive(Clone, Serialize)]
pub struct BookOut {
    conv: i64,
    name: String,
    count: i64,
    /// You can change it.
    yours: bool,
    people: Vec<SharedWith>,
    /// Group chats it's shared with.
    groups: Vec<SharedGroup>,
}

#[derive(Clone, Serialize)]
struct SharedWith {
    user: User,
    owner: bool,
}

#[derive(Serialize, Deserialize)]
struct CalPost {
    /// "put" or "delete".
    op: String,
    uid: String,
    #[serde(default)]
    event: Option<Event>,
}

/// Settings › Calendar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Who sees when you're busy: "everyone" (on this server), "chats"
    /// (people you chat with) or "nobody".
    pub share: String,
    /// Working hours ("08:00"–"18:00") on these days (1 Monday … 7 Sunday).
    pub work_from: String,
    pub work_to: String,
    pub work_days: Vec<u32>,
    /// The email account invitations to people outside the server go from
    /// (empty: the first one).
    #[serde(default)]
    pub mail_from: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            share: "everyone".into(),
            work_from: "08:00".into(),
            work_to: "18:00".into(),
            work_days: vec![1, 2, 3, 4, 5],
            mail_from: String::new(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Inner {
    server: String,
    pub(crate) me: Option<User>,
    /// The reminders the server has (as last sent).
    pub(crate) reminders_sent: Option<Vec<xenify_chat_proto::Reminder>>,
    /// This computer, in the ids of calendar posts (so it knows its own).
    device: String,
    /// Conversations that aren't chats.
    pub(crate) hidden: HashMap<i64, Conversation>,
    /// People you chat with (for "People I chat with").
    chat_people: HashMap<i64, String>,
    /// Names of people seen on the server, by user name.
    chat_people_names: HashMap<String, String>,
    /// Your group chats (for books shared with a group).
    groups: HashMap<i64, Conversation>,
    calendar: Option<i64>,
    busy: Option<i64>,
    /// Asked the server to make your calendar or busy-times conversation.
    asked: HashSet<&'static str>,
    meetings: HashMap<i64, Meeting>,
    /// New meetings waiting for their conversation: (its title, details).
    pending: Vec<(String, Details)>,
    /// Your address book's conversation (My contacts).
    contacts: Option<i64>,
    /// New shared books waiting for their channel: (its title, people,
    /// groups).
    pending_books: Vec<(String, Vec<Sharing>, Vec<GroupShare>)>,
    /// Calendar and address book posts gathered while catching up, by
    /// conversation.
    catching_up: HashMap<i64, Vec<Message>>,
    /// Others' busy times, as last downloaded.
    busy_of: HashMap<i64, BusyOut>,
    /// The people Find a time is waiting on.
    wanted: HashSet<i64>,
    /// Your out of office, as the server has it.
    pub(crate) away: Option<Away>,
    /// How far the server's clock is ahead of this device's (ms), so
    /// reminders go off when they should even when one of them is wrong.
    pub(crate) skew: i64,
    /// Shared calendars you're in.
    pub(crate) shared: HashMap<i64, crate::sharedcals::Shared>,
    /// New shared calendars waiting for their channel: (title, people).
    pub(crate) pending_cals: Vec<(String, Vec<Sharing>)>,
}

/// Where the meeting helpers live.
struct Meetings;

fn key(inner: &Inner, what: &str) -> String {
    format!("{what}:{}:{}", inner.server, inner.me.as_ref().map_or(0, |u| u.id))
}

fn random_id() -> String {
    xenify_sync::random(8).iter().map(|b| format!("{b:02x}")).collect()
}

/// The text of a post (None once deleted).
fn text_of(m: &Message) -> Option<&str> {
    (!m.deleted && !m.text.is_empty()).then_some(m.text.as_str())
}

fn settings_of(cal: &Calendar) -> Settings {
    cal.get_meta("calendar_settings").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Your working hours from `from` for `days` days, as stretches of time.
fn working_hours(s: &Settings, from: i64, days: i64, tz: Tz) -> Vec<(i64, i64)> {
    let parse = |t: &str| NaiveTime::parse_from_str(t, "%H:%M").ok();
    let (Some(a), Some(b)) = (parse(&s.work_from), parse(&s.work_to)) else { return Vec::new() };
    let Some(first) = chrono::DateTime::from_timestamp_millis(from).map(|d| d.with_timezone(&tz).date_naive()) else {
        return Vec::new();
    };
    (0..days)
        .filter_map(|i| {
            let day = first + chrono::Duration::days(i);
            if !s.work_days.contains(&day.weekday().number_from_monday()) {
                return None;
            }
            let at = |t: NaiveTime| tz.from_local_datetime(&day.and_time(t)).earliest().map(|d| d.timestamp_millis());
            Some((at(a)?, at(b)?)).filter(|(x, y)| y > x)
        })
        .collect()
}

fn away_times(cal: &Calendar) -> Vec<(i64, i64)> {
    cal.away_events().unwrap_or_default().into_iter().map(|e| (e.start, e.end)).collect()
}

/// Whether `from`..`to` falls in a time you're away.
fn away_then(inner: &Inner, from: i64, to: i64) -> bool {
    inner.away.as_ref().is_some_and(|a| a.times.iter().any(|(s, e)| *s < to && *e > from))
}

fn start_of_today<H: Host>(app: &Core<H>) -> i64 {
    let tz = app.cal.tz();
    let today =
        chrono::DateTime::from_timestamp_millis(app.host.now()).unwrap_or_default().with_timezone(&tz).date_naive();
    tz.from_local_datetime(&today.and_time(NaiveTime::MIN)).earliest().map_or(0, |d| d.timestamp_millis())
}

/// Today as people say it ("12 October").
pub(crate) fn today<H: Host>(app: &Core<H>) -> String {
    today_words(app)
}

fn today_words<H: Host>(app: &Core<H>) -> String {
    let tz = app.cal.tz();
    chrono::DateTime::from_timestamp_millis(app.host.now())
        .unwrap_or_default()
        .with_timezone(&tz)
        .format("%-d %B")
        .to_string()
}

impl Meetings {
    fn meeting_out(inner: &Inner, conv: i64) -> Option<MeetingOut> {
        let c = inner.hidden.get(&conv)?;
        let m = inner.meetings.get(&conv)?;
        let details = m.details.clone()?;
        let me = inner.me.as_ref()?.id;
        let organiser = c.members.iter().find(|u| c.owners.contains(&u.id)).cloned();
        let answer_of = |u: &User| {
            m.answers
                .get(&u.id)
                .map(|(a, _)| a.clone())
                .unwrap_or_else(|| if c.owners.contains(&u.id) { "yes".into() } else { String::new() })
        };
        let mut people: Vec<Answered> =
            c.members.iter().filter(|u| !u.app).map(|u| Answered { user: u.clone(), answer: answer_of(u) }).collect();
        people.sort_by_key(|a| !c.owners.contains(&a.user.id));
        let mine = c.members.iter().find(|u| u.id == me).map(answer_of).unwrap_or_default();
        Some(MeetingOut { conv, details, organiser, people, mine, yours: c.owners.contains(&me) })
    }

    fn all_out(inner: &Inner) -> Vec<MeetingOut> {
        let mut list: Vec<MeetingOut> = inner.meetings.keys().filter_map(|c| Self::meeting_out(inner, *c)).collect();
        list.sort_by_key(|m| m.details.start);
        list
    }
}

/// Tells the screens the meetings changed (and the calendar with them).
fn meetings_changed<H: Host>(app: &Core<H>, inner: &Inner) {
    app.emit("meetings", Meetings::all_out(inner));
    app.emit("calendar", ());
}

/// Puts meeting `conv` in your calendar as it now stands, or takes it out
/// (cancelled, or you said no). Returns an invitation to tell you about.
fn file_meeting<H: Host>(app: &Core<H>, inner: &Inner, conv: i64) -> Option<MeetingOut> {
    let cal = &app.cal;
    let out = Meetings::meeting_out(inner, conv)?;
    let d = &out.details;
    if d.cancelled || out.mine == "no" {
        let _ = cal.drop_meeting(conv);
        return None;
    }
    let link = serde_json::json!({ "type": "meeting", "conv": conv, "label": format!("the meeting “{}”", d.title) });
    let rooms = d.things.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ");
    let notes = [rooms.as_str(), d.place.as_str(), d.notes.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = cal.put_meeting(&Event {
        title: d.title.clone(),
        start: d.start,
        end: d.end.max(d.start),
        notes,
        kind: "event".into(),
        link: link.to_string(),
        remind: Some(cal.get_meta("meeting_remind").and_then(|m| m.parse().ok()).unwrap_or(10)),
        meeting: conv,
        answer: out.mine.clone(),
        ..Event::default()
    });
    // An invitation you haven't answered, for a meeting still to come: tell
    // you once.
    let told = key(inner, &format!("told{conv}"));
    if !out.yours && out.mine.is_empty() && d.start > app.host.now() && cal.get_meta(&told).is_none() {
        cal.set_meta(&told, "1");
        return Some(out);
    }
    None
}

/// Reads posts in a meeting (any order).
fn read_meeting(inner: &mut Inner, conv: i64, posts: &[Message]) -> bool {
    let owners = inner.hidden.get(&conv).map(|c| c.owners.clone()).unwrap_or_default();
    let m = inner.meetings.entry(conv).or_default();
    let mut changed = false;
    for p in posts {
        let Some(text) = text_of(p) else { continue };
        if let Some(json) = text.strip_prefix(MEET) {
            if owners.contains(&p.sender.id)
                && p.seq > m.details_seq
                && let Ok(d) = serde_json::from_str::<Details>(json)
            {
                m.details = Some(d);
                m.details_seq = p.seq;
                changed = true;
            }
        } else if let Some(a) = text.strip_prefix(ANSWER)
            && ["yes", "maybe", "no"].contains(&a)
            && m.answers.get(&p.sender.id).is_none_or(|(_, s)| p.seq > *s)
        {
            m.answers.insert(p.sender.id, (a.to_string(), p.seq));
            changed = true;
        }
    }
    changed
}

/// Applies calendar or address book posts, oldest first; returns whether
/// anything changed.
fn apply_log<H: Host>(app: &Core<H>, inner: &Inner, conv: i64, mut posts: Vec<Message>) -> bool {
    let cal = &app.cal;
    let book_store = &app.contacts;
    let Some(c) = inner.hidden.get(&conv) else { return false };
    let purpose = c.purpose.as_str();
    // My contacts is book 0 here.
    let book = if purpose == BOOK { conv } else { 0 };
    let seen_key = key(inner, &format!("calseq{conv}"));
    let seen: i64 = cal.get_meta(&seen_key).and_then(|s| s.parse().ok()).unwrap_or(0);
    posts.sort_by_key(|p| p.seq);
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    let mut changed = false;
    let mut upto = seen;
    for p in posts.iter().filter(|p| p.seq > seen) {
        upto = upto.max(p.seq);
        // Your own calendar and book; a shared book's owners.
        let allowed = if purpose == BOOK { c.owners.contains(&p.sender.id) } else { p.sender.id == me };
        let Some(text) = text_of(p).filter(|_| allowed) else { continue };
        if let Some(json) = text.strip_prefix(BOOK_SHARE) {
            if serde_json::from_str::<BookSharing>(json).is_ok() {
                book_store.set_book_sharing(conv, json);
                changed = true;
            }
            continue;
        }
        if let Some(json) = text.strip_prefix(BOOK_NAME) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
                book_store.set_book_name(conv, v["name"].as_str().unwrap_or("Contact list"));
                changed = true;
            }
            continue;
        }
        // This computer's own post: the server has that change now.
        if let Some(rest) = p.client_id.strip_prefix(&format!("{}.", inner.device)) {
            match (purpose, rest.rsplit_once('.')) {
                (CALENDAR, Some((uid, "del"))) => {
                    let _ = cal.gone_sent(uid);
                }
                (CALENDAR, Some((uid, n))) => {
                    let _ = cal.sent(uid, n.parse().unwrap_or(-1));
                }
                (_, Some((uid, "del"))) => book_store.gone_sent(book, uid),
                (_, Some((uid, n))) => book_store.sent(book, uid, n.parse().unwrap_or(-1)),
                _ => {}
            }
            continue;
        }
        if purpose == CALENDAR {
            let Some(post) = text.strip_prefix(CAL).and_then(|j| serde_json::from_str::<CalPost>(j).ok()) else {
                continue;
            };
            changed = true;
            match (post.op.as_str(), post.event) {
                ("put", Some(e)) => {
                    let _ = cal.put_from_server(&Event { uid: post.uid, ..e });
                }
                ("delete", _) => {
                    let _ = cal.delete_from_server(&post.uid);
                }
                _ => {}
            }
        } else {
            let Some(post) = text.strip_prefix(CONTACT).and_then(|j| serde_json::from_str::<ContactPost>(j).ok())
            else {
                continue;
            };
            changed = true;
            match (post.op.as_str(), post.contact) {
                ("put", Some(e)) => book_store.put_from_server(&Contact { book, uid: post.uid, ..e }),
                ("delete", _) => book_store.delete_from_server(book, &post.uid),
                _ => {}
            }
        }
    }
    if upto > seen {
        cal.set_meta(&seen_key, &upto.to_string());
    }
    changed
}

/// Sends address book changes the server hasn't had: yours to My contacts,
/// shared books' to them (if you can change them).
fn send_contacts<H: Host>(app: &Core<H>, inner: &Inner) {
    let st = &app.contacts;
    let conv_of = |book: i64| -> Option<i64> {
        if book == 0 { inner.contacts } else { inner.hidden.get(&book).filter(|c| c.role == Role::Owner).map(|c| c.id) }
    };
    for (e, n) in st.unsent().unwrap_or_default() {
        let Some(conv) = conv_of(e.book) else { continue };
        let uid = e.uid.clone();
        let post = ContactPost { op: "put".into(), uid: uid.clone(), contact: Some(Contact { book: 0, ..e }) };
        app.tell(ToServer::Send {
            conv,
            client_id: format!("{}.{uid}.{n}", inner.device),
            text: format!("{CONTACT}{}", serde_json::to_string(&post).unwrap_or_default()),
        });
    }
    for (book, uid) in st.gone().unwrap_or_default() {
        let Some(conv) = conv_of(book) else { continue };
        let post = ContactPost { op: "delete".into(), uid: uid.clone(), contact: None };
        app.tell(ToServer::Send {
            conv,
            client_id: format!("{}.{uid}.del", inner.device),
            text: format!("{CONTACT}{}", serde_json::to_string(&post).unwrap_or_default()),
        });
    }
}

/// Sends your calendar's changes the server hasn't had.
fn send_calendar<H: Host>(app: &Core<H>, inner: &Inner) {
    let Some(conv) = inner.calendar else { return };
    let cal = &app.cal;
    for (e, n) in cal.unsent().unwrap_or_default() {
        let uid = e.uid.clone();
        let event = Event { id: 0, reminded: false, ..e };
        let post = CalPost { op: "put".into(), uid: uid.clone(), event: Some(event) };
        app.tell(ToServer::Send {
            conv,
            client_id: format!("{}.{uid}.{n}", inner.device),
            text: format!("{CAL}{}", serde_json::to_string(&post).unwrap_or_default()),
        });
    }
    for uid in cal.gone().unwrap_or_default() {
        let post = CalPost { op: "delete".into(), uid: uid.clone(), event: None };
        app.tell(ToServer::Send {
            conv,
            client_id: format!("{}.{uid}.del", inner.device),
            text: format!("{CAL}{}", serde_json::to_string(&post).unwrap_or_default()),
        });
    }
}

/// Posts your busy times when they've changed (and you share them).
fn send_busy<H: Host>(app: &Core<H>, inner: &Inner) {
    let Some(conv) = inner.busy else { return };
    let cal = &app.cal;
    let s = settings_of(cal);
    if s.share == "nobody" {
        return;
    }
    let from = start_of_today(app);
    let post = BusyPost {
        from,
        to: from + AHEAD,
        busy: cal.busy(from, from + AHEAD).unwrap_or_default(),
        work: working_hours(&s, from, AHEAD / DAY, app.cal.tz()),
        away: away_times(cal),
    };
    let text = format!("{BUSY_POST}{}", serde_json::to_string(&post).unwrap_or_default());
    let posted = key(inner, "busyposted");
    if cal.get_meta(&posted).as_deref() == Some(text.as_str()) {
        return;
    }
    cal.set_meta(&posted, &text);
    app.tell(ToServer::Send { conv, client_id: random_id(), text });
}

/// Keeps who follows your busy times in line with the setting.
fn keep_busy_followers<H: Host>(app: &Core<H>, inner: &Inner) {
    let Some(c) = inner.busy.and_then(|b| inner.hidden.get(&b)) else { return };
    let s = settings_of(&app.cal);
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    let everyone = s.share == "everyone";
    if c.open != everyone {
        app.tell(ToServer::SetOpen { conv: c.id, open: everyone });
    }
    if everyone {
        // Everyone follows it from the start, so they have its key before
        // they look (it's handed over while you're online): who's here?
        app.tell(ToServer::FindPeople { q: String::new() });
        return;
    }
    let want: HashSet<&String> = if s.share == "chats" { inner.chat_people.values().collect() } else { HashSet::new() };
    let have: HashSet<&String> = c.members.iter().filter(|u| u.id != me).map(|u| &u.username).collect();
    for gone in have.difference(&want) {
        app.tell(ToServer::RemovePerson { conv: c.id, user: (*gone).clone() });
    }
    let add: Vec<String> = want.difference(&have).map(|u| (*u).clone()).collect();
    if !add.is_empty() {
        app.tell(ToServer::AddPeople { conv: c.id, users: add });
    }
}

fn read_busy<H: Host>(app: &Core<H>, inner: &mut Inner, conv: i64, posts: &[Message]) {
    let Some(owner) = inner.hidden.get(&conv).and_then(|c| c.owners.first().copied()) else { return };
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    if owner == me {
        return;
    }
    let Some(post) = posts
        .iter()
        .filter(|p| p.sender.id == owner)
        .max_by_key(|p| p.seq)
        .and_then(text_of)
        .and_then(|t| t.strip_prefix(BUSY_POST))
        .and_then(|j| serde_json::from_str::<BusyPost>(j).ok())
    else {
        return;
    };
    let out = BusyOut { user: owner, known: true, busy: post.busy, work: post.work, away: post.away };
    inner.busy_of.insert(owner, out.clone());
    app.emit("busy", vec![out]);
}

/// What the server says, before the screens hear it: takes what belongs to
/// meetings, your calendar and busy times. Returns what the screens should
/// still see. Anything to send back goes the usual way.
pub fn incoming<H: Host>(app: &Core<H>, m: ToApp) -> Option<ToApp> {
    let mut inner = app.inner.lock().unwrap();
    let mut send = Vec::new();
    let shown = match m {
        ToApp::Welcome { ref user, now, version, .. } => {
            // A server on another protocol level: the screens say who should update.
            if version != xenify_chat_proto::VERSION {
                app.emit("outdated", serde_json::json!({ "server": version, "app": xenify_chat_proto::VERSION }));
            }
            let server = app.host.server();
            let cal = &app.cal;
            let device = cal.get_meta("device").unwrap_or_else(|| {
                let d = random_id();
                cal.set_meta("device", &d);
                d
            });
            let skew = if now > 0 { now - app.host.now() } else { 0 };
            *inner = Inner { server, me: Some(user.clone()), device, skew, ..Inner::default() };
            Some(m)
        }
        ToApp::Conversations { list } => {
            let (hidden, chats): (Vec<Conversation>, Vec<Conversation>) =
                list.into_iter().partition(|c| !c.purpose.is_empty());
            note_chats(&mut inner, &chats);
            let mut shown = chats;
            for c in hidden {
                if c.purpose == MEETING {
                    shown.push(c.clone());
                }
                send.extend(note(app, &mut inner, c, false));
            }
            send.extend(start_up(app, &mut inner));
            Some(ToApp::Conversations { list: shown })
        }
        ToApp::Conversation { conv } => {
            if conv.purpose.is_empty() {
                note_chats(&mut inner, std::slice::from_ref(&conv));
                if conv.kind == Kind::Group {
                    // Someone joined or left a group a book is shared with.
                    let books: Vec<i64> = inner.hidden.values().filter(|c| c.purpose == BOOK).map(|c| c.id).collect();
                    for b in books {
                        send.extend(keep_book_members(app, &inner, b));
                    }
                }
                Some(ToApp::Conversation { conv })
            } else {
                let show = (conv.purpose == MEETING).then(|| conv.clone());
                send.extend(note(app, &mut inner, conv, true));
                show.map(|conv| ToApp::Conversation { conv })
            }
        }
        ToApp::Removed { conv } => {
            if inner.hidden.get(&conv).is_some_and(|c| c.purpose == BOOK) {
                app.contacts.drop_book(conv);
                app.emit("contacts", ());
            }
            if inner.meetings.remove(&conv).is_some() {
                let _ = app.cal.drop_meeting(conv);
                meetings_changed(app, &inner);
            }
            if inner.shared.remove(&conv).is_some() {
                app.emit("calendar", ());
            }
            inner.hidden.remove(&conv);
            Some(ToApp::Removed { conv })
        }
        ToApp::Message { msg } => match inner.hidden.get(&msg.conv).map(|c| c.purpose.clone()) {
            None => Some(ToApp::Message { msg }),
            // Files in a meeting (its recordings) are for the screens.
            Some(p) if p == MEETING && msg.text.starts_with(FILE) => Some(ToApp::Message { msg }),
            Some(p) => {
                send.extend(posts(app, &mut inner, &p, msg.conv, vec![msg], false));
                None
            }
        },
        ToApp::History { conv, messages, thread, tag } => match inner.hidden.get(&conv).map(|c| c.purpose.clone()) {
            None => Some(ToApp::History { conv, messages, thread, tag }),
            Some(p) => {
                let files: Vec<Message> = if p == MEETING {
                    messages.iter().filter(|m| m.text.starts_with(FILE)).cloned().collect()
                } else {
                    Vec::new()
                };
                let asked_here = tag == TAG;
                send.extend(posts(app, &mut inner, &p, conv, messages, asked_here));
                (!asked_here && p == MEETING).then_some(ToApp::History { conv, messages: files, thread, tag })
            }
        },
        // Everyone on the server: those not yet following your busy times
        // (when everyone may see them) start following.
        ToApp::People { ref q, ref users } if q.is_empty() => {
            let me = inner.me.as_ref().map_or(0, |u| u.id);
            let everyone = settings_of(&app.cal).share == "everyone";
            if let Some(c) = inner.busy.and_then(|b| inner.hidden.get(&b)).filter(|_| everyone) {
                let have: HashSet<i64> = c.members.iter().map(|u| u.id).collect();
                let add: Vec<String> = users
                    .iter()
                    .filter(|u| !u.app && u.id != me && !have.contains(&u.id))
                    .map(|u| u.username.clone())
                    .collect();
                if !add.is_empty() && c.member_count <= 50 {
                    send.push(ToServer::AddPeople { conv: c.id, users: add });
                }
            }
            Some(m)
        }
        ToApp::AwayOf { list } => {
            let me = inner.me.as_ref().map_or(0, |u| u.id);
            if let Some(mine) = list.iter().find(|a| a.user == me) {
                inner.away = Some(mine.clone());
            }
            Some(ToApp::AwayOf { list })
        }
        ToApp::BusyOf { list } => {
            let mut unknown: HashSet<i64> = std::mem::take(&mut inner.wanted);
            for b in list {
                if b.member {
                    send.push(ToServer::History { conv: b.conv, before: None, limit: 1, tag: TAG.into() });
                } else if b.open {
                    send.push(ToServer::Subscribe { conv: b.conv });
                    send.push(ToServer::History { conv: b.conv, before: None, limit: 1, tag: TAG.into() });
                } else {
                    continue;
                }
                unknown.remove(&b.user);
            }
            let out: Vec<BusyOut> = unknown
                .into_iter()
                .map(|user| BusyOut { user, known: false, busy: Vec::new(), work: Vec::new(), away: Vec::new() })
                .collect();
            if !out.is_empty() {
                app.emit("busy", out);
            }
            None
        }
        other => Some(other),
    };
    drop(inner);
    for m in send {
        app.tell(m);
    }
    shown
}

/// People you chat with, from your chats.
fn note_chats(inner: &mut Inner, chats: &[Conversation]) {
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    for c in chats.iter().filter(|c| c.kind == Kind::Group) {
        inner.groups.insert(c.id, c.clone());
    }
    for c in chats.iter().filter(|c| c.kind != Kind::Channel) {
        for u in c.members.iter().filter(|u| !u.app && u.id != me) {
            inner.chat_people.insert(u.id, u.username.clone());
            inner.chat_people_names.insert(u.username.clone(), u.name().to_string());
        }
    }
}

/// Records a conversation that isn't a chat; `fresh`: it just arrived on
/// its own (made, joined or changed), not in the list.
fn note<H: Host>(app: &Core<H>, inner: &mut Inner, c: Conversation, fresh: bool) -> Vec<ToServer> {
    let mut send = Vec::new();
    let mine = c.role == Role::Owner;
    let id = c.id;
    let purpose = c.purpose.clone();
    let title = c.title.clone();
    let last_seq = c.last_seq;
    let first_time = !inner.hidden.contains_key(&id);
    inner.hidden.insert(id, c);
    match purpose.as_str() {
        CALENDAR if mine => {
            let new = inner.calendar != Some(id);
            inner.calendar = Some(id);
            if new && fresh {
                send.extend(catch_up(app, inner, id, last_seq));
                send_calendar(app, inner);
            }
        }
        CONTACTS if mine => {
            let new = inner.contacts != Some(id);
            inner.contacts = Some(id);
            if new && fresh {
                send.extend(catch_up(app, inner, id, last_seq));
                send_contacts(app, inner);
            }
        }
        BOOK => {
            // A book you just made: its name, then who it's shared with.
            if mine && let Some(i) = inner.pending_books.iter().position(|(t, _, _)| *t == title) {
                let (name, people, groups) = inner.pending_books.remove(i);
                app.contacts.set_book_name(id, &name);
                send.push(ToServer::Send {
                    conv: id,
                    client_id: random_id(),
                    text: format!("{BOOK_NAME}{}", serde_json::json!({ "name": name })),
                });
                send.extend(share_posts(id, &people));
                if !groups.is_empty() {
                    let sharing = BookSharing { groups, people: people.iter().map(|p| p.username.clone()).collect() };
                    send.push(sharing_post(id, &sharing));
                    app.contacts.set_book_sharing(id, &serde_json::to_string(&sharing).unwrap_or_default());
                    send.extend(keep_book_members(app, inner, id));
                }
            } else if first_time || fresh {
                send.extend(catch_up(app, inner, id, last_seq));
            }
            app.emit("contacts", ());
        }
        SHARED_CAL => {
            let c = inner.hidden.get(&id).cloned().expect("just put");
            send.extend(crate::sharedcals::note(app, inner, &c));
        }
        BUSY if mine => {
            let new = inner.busy != Some(id);
            inner.busy = Some(id);
            if new && fresh {
                keep_busy_followers(app, inner);
                send_busy(app, inner);
            }
        }
        MEETING => {
            // A meeting you just made: post its details, and say you're going.
            if mine && let Some(i) = inner.pending.iter().position(|(t, _)| *t == title) {
                let (_, d) = inner.pending.remove(i);
                send.push(details_post(id, &d));
                book_things(app, id, &d);
                inner.meetings.entry(id).or_default();
                email_guests(app, inner, id, &d, false);
            } else if first_time {
                send.push(ToServer::History { conv: id, before: None, limit: 200, tag: TAG.into() });
            }
            if !first_time && inner.meetings.contains_key(&id) {
                // Who's in it changed.
                let _ = file_meeting(app, inner, id);
                meetings_changed(app, inner);
            }
        }
        _ => {}
    }
    send
}

/// After the list arrives: make what's missing, catch up, send what's waiting.
fn start_up<H: Host>(app: &Core<H>, inner: &mut Inner) -> Vec<ToServer> {
    let mut send = Vec::new();
    match inner.calendar.and_then(|c| inner.hidden.get(&c)).map(|c| (c.id, c.last_seq)) {
        Some((conv, last)) => {
            send.extend(catch_up(app, inner, conv, last));
            send_calendar(app, inner);
        }
        None if inner.asked.insert(CALENDAR) => {
            send.push(ToServer::CreateGroup { title: "Calendar".into(), members: Vec::new(), purpose: CALENDAR.into() })
        }
        None => {}
    }
    match inner.contacts.and_then(|c| inner.hidden.get(&c)).map(|c| (c.id, c.last_seq)) {
        Some((conv, last)) => {
            send.extend(catch_up(app, inner, conv, last));
            send_contacts(app, inner);
        }
        None if inner.asked.insert(CONTACTS) => {
            send.push(ToServer::CreateGroup { title: "Contacts".into(), members: Vec::new(), purpose: CONTACTS.into() })
        }
        None => {}
    }
    let books: Vec<(i64, i64)> =
        inner.hidden.values().filter(|c| c.purpose == BOOK).map(|c| (c.id, c.last_seq)).collect();
    for (conv, last) in &books {
        send.extend(catch_up(app, inner, *conv, *last));
    }
    // Books shared with groups follow who's in the groups.
    for (conv, _) in &books {
        send.extend(keep_book_members(app, inner, *conv));
    }
    // Books you're no longer in leave this computer.
    let st = &app.contacts;
    if let Ok(names) = st.book_convs() {
        for conv in names.into_iter().filter(|b| !books.iter().any(|(c, _)| c == b)) {
            st.drop_book(conv);
        }
    }
    if inner.busy.is_some() {
        keep_busy_followers(app, inner);
        send_busy(app, inner);
    } else if inner.asked.insert(BUSY) {
        let open = settings_of(&app.cal).share == "everyone";
        send.push(ToServer::CreateChannel { title: "Busy times".into(), open, purpose: BUSY.into() });
    }
    // Your own out of office.
    if let Some(me) = inner.me.as_ref().map(|u| u.id) {
        send.push(ToServer::AwayOf { users: vec![me] });
    }
    // Meetings no longer on the server leave your calendar.
    let cal = &app.cal;
    let now = app.host.now();
    let from = now - 365 * DAY;
    for e in cal.between(from, now + 5 * 365 * DAY).unwrap_or_default() {
        if e.meeting != 0 && !inner.hidden.contains_key(&e.meeting) {
            let _ = cal.drop_meeting(e.meeting);
        }
    }
    send
}

/// Asks for calendar or address book posts not read yet (the newest page first).
fn catch_up<H: Host>(app: &Core<H>, inner: &mut Inner, conv: i64, last_seq: i64) -> Vec<ToServer> {
    let seen: i64 = app.cal.get_meta(&key(inner, &format!("calseq{conv}"))).and_then(|s| s.parse().ok()).unwrap_or(0);
    if last_seq <= seen || inner.catching_up.contains_key(&conv) {
        return Vec::new();
    }
    inner.catching_up.insert(conv, Vec::new());
    vec![ToServer::History { conv, before: None, limit: PAGE, tag: TAG.into() }]
}

/// Posts in a conversation that isn't a chat.
fn posts<H: Host>(
    app: &Core<H>,
    inner: &mut Inner,
    purpose: &str,
    conv: i64,
    list: Vec<Message>,
    asked_here: bool,
) -> Vec<ToServer> {
    let mut send = Vec::new();
    match purpose {
        CALENDAR | CONTACTS | BOOK => {
            let seen: i64 =
                app.cal.get_meta(&key(inner, &format!("calseq{conv}"))).and_then(|s| s.parse().ok()).unwrap_or(0);
            let changed = if asked_here && inner.catching_up.contains_key(&conv) {
                // A page while catching up: older ones too, or apply them all.
                let first = list.iter().map(|m| m.seq).min();
                let full = list.len() as u32 == PAGE;
                inner.catching_up.get_mut(&conv).expect("catching up").extend(list);
                match first {
                    Some(first) if full && first > seen + 1 => {
                        send.push(ToServer::History { conv, before: Some(first), limit: PAGE, tag: TAG.into() });
                        false
                    }
                    _ => {
                        let all = inner.catching_up.remove(&conv).unwrap_or_default();
                        apply_log(app, inner, conv, all)
                    }
                }
            } else if let Some(waiting) = inner.catching_up.get_mut(&conv) {
                // Live posts while catching up wait their turn.
                waiting.extend(list);
                false
            } else if list.iter().map(|m| m.seq).min().is_some_and(|first| first > seen + 1) {
                // Some were missed: read back to them first.
                inner.catching_up.insert(conv, list);
                send.push(ToServer::History { conv, before: None, limit: PAGE, tag: TAG.into() });
                false
            } else {
                apply_log(app, inner, conv, list)
            };
            if changed && purpose == CALENDAR {
                app.emit("calendar", ());
                send_busy(app, inner);
            } else if changed {
                if purpose == BOOK {
                    send.extend(keep_book_members(app, inner, conv));
                }
                app.emit("contacts", ());
            }
        }
        MEETING => {
            if read_meeting(inner, conv, &list) {
                if let Some(invite) = file_meeting(app, inner, conv) {
                    if inner.away.as_ref().is_some_and(|a| a.decline)
                        && away_then(inner, invite.details.start, invite.details.end)
                    {
                        // Away then: no, with the automatic reply as the reason.
                        send.push(ToServer::Send { conv, client_id: random_id(), text: format!("{ANSWER}no") });
                    } else {
                        app.emit("meeting_invite", invite);
                    }
                }
                meetings_changed(app, inner);
                send_busy(app, inner);
            }
        }
        BUSY => read_busy(app, inner, conv, &list),
        SHARED_CAL => send.extend(crate::sharedcals::posts(app, inner, conv, list, asked_here)),
        _ => {}
    }
    send
}

fn details_post(conv: i64, d: &Details) -> ToServer {
    ToServer::Send {
        conv,
        client_id: random_id(),
        text: format!("{MEET}{}", serde_json::to_string(d).unwrap_or_default()),
    }
}

/// Sends people invited by email the invitation (or, `cancel`, the
/// cancellation), from the meeting's email account. Runs in the background;
/// a failure is shown as a message.
fn email_guests<H: Host>(app: &Core<H>, inner: &Inner, conv: i64, d: &Details, cancel: bool) {
    if d.guests.is_empty() {
        return;
    }
    let organiser = inner.me.as_ref().map(|u| u.name().to_string()).unwrap_or_default();
    let names: Vec<String> = inner
        .hidden
        .get(&conv)
        .map(|c| c.members.iter().filter(|u| !u.app).map(|u| u.name().to_string()).collect())
        .unwrap_or_default();
    app.host.email_guests(d, &organiser, &names, cancel);
}

/// The calendar changed on this computer: send it, and your busy times.
pub fn calendar_changed<H: Host>(app: &Core<H>) {
    let inner = app.inner.lock().unwrap();
    send_calendar(app, &inner);
    send_busy(app, &inner);
}

/// Someone invited by email answered (an iCalendar reply to meeting `uid`).
/// `email` must be the address the answer came from, not just the one the
/// reply names: callers check they're the same (anyone can write a reply
/// naming someone else). Only guests of a meeting you organise count.
pub fn guest_answered<H: Host>(app: &Core<H>, uid: &str, email: &str, answer: &str) {
    let mut inner = app.inner.lock().unwrap();
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    let found = inner.meetings.iter().find_map(|(conv, m)| {
        let d = m.details.as_ref()?;
        let yours = inner.hidden.get(conv).is_some_and(|c| c.owners.contains(&me));
        (yours && d.uid == uid).then(|| (*conv, d.clone()))
    });
    let Some((conv, mut d)) = found else { return };
    let Some(g) = d.guests.iter_mut().find(|g| g.email.eq_ignore_ascii_case(email)) else { return };
    if g.answer == answer {
        return;
    }
    g.answer = answer.into();
    if let Some(m) = inner.meetings.get_mut(&conv) {
        m.details = Some(d.clone());
    }
    app.tell(details_post(conv, &d));
    meetings_changed(app, &inner);
}

/// The address books changed on this computer: send them, tell the screens.
pub fn contacts_changed<H: Host>(app: &Core<H>) {
    let inner = app.inner.lock().unwrap();
    send_contacts(app, &inner);
    app.emit("contacts", ());
}

/// Whether you can change shared book `conv`.
pub fn can_change_book<H: Host>(app: &Core<H>, conv: i64) -> bool {
    let inner = app.inner.lock().unwrap();
    inner.hidden.get(&conv).is_some_and(|c| c.purpose == BOOK && c.role == Role::Owner)
}

fn sharing_post(conv: i64, sharing: &BookSharing) -> ToServer {
    ToServer::Send {
        conv,
        client_id: random_id(),
        text: format!("{BOOK_SHARE}{}", serde_json::to_string(sharing).unwrap_or_default()),
    }
}

fn sharing_of<H: Host>(app: &Core<H>, conv: i64) -> Option<BookSharing> {
    serde_json::from_str(&app.contacts.book_sharing(conv)).ok()
}

/// A book you can change that's shared with groups: everyone in them gets
/// it (and can change it if the group can); people who left them, and
/// weren't shared with one by one, are taken out.
fn keep_book_members<H: Host>(app: &Core<H>, inner: &Inner, conv: i64) -> Vec<ToServer> {
    let mut send = Vec::new();
    let Some(book) = inner.hidden.get(&conv).filter(|c| c.role == Role::Owner) else { return send };
    let Some(sharing) = sharing_of(app, conv).filter(|s| !s.groups.is_empty()) else { return send };
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    let mut want: HashMap<String, bool> = sharing.people.iter().map(|p| (p.clone(), false)).collect();
    // Only take people out when every group's members can be seen here.
    let mut all_seen = true;
    for g in &sharing.groups {
        match inner.groups.get(&g.conv) {
            Some(c) => {
                for u in c.members.iter().filter(|u| !u.app && u.id != me) {
                    let owner = want.entry(u.username.clone()).or_insert(false);
                    *owner |= g.owner;
                }
            }
            None => all_seen = false,
        }
    }
    let have: HashMap<&str, bool> = book
        .members
        .iter()
        .filter(|u| u.id != me)
        .map(|u| (u.username.as_str(), book.owners.contains(&u.id)))
        .collect();
    let add: Vec<String> = want.keys().filter(|u| !have.contains_key(u.as_str())).cloned().collect();
    if !add.is_empty() {
        send.push(ToServer::AddPeople { conv, users: add });
    }
    for (u, owner) in &want {
        // Can change through a group; individuals keep what they were given.
        if *owner && have.get(u.as_str()) == Some(&false) {
            send.push(ToServer::SetRole { conv, user: u.clone(), owner: true });
        }
    }
    let new_owners = want.iter().filter(|(u, o)| **o && !have.contains_key(u.as_str())).map(|(u, _)| u.clone());
    for u in new_owners {
        send.push(ToServer::SetRole { conv, user: u, owner: true });
    }
    if all_seen && book.member_count <= 50 {
        for u in have.keys().filter(|u| !want.contains_key(**u)) {
            send.push(ToServer::RemovePerson { conv, user: u.to_string() });
        }
    }
    send
}

/// Adding people to a book, and saying who can change it.
pub(crate) fn share_posts(conv: i64, people: &[Sharing]) -> Vec<ToServer> {
    let mut send = Vec::new();
    if !people.is_empty() {
        send.push(ToServer::AddPeople { conv, users: people.iter().map(|p| p.username.clone()).collect() });
    }
    for p in people.iter().filter(|p| p.owner) {
        send.push(ToServer::SetRole { conv, user: p.username.clone(), owner: true });
    }
    send
}

pub fn books_list<H: Host>(app: &Core<H>) -> Vec<BookOut> {
    let inner = app.inner.lock().unwrap();
    let mut list: Vec<BookOut> = inner
        .hidden
        .values()
        .filter(|c| c.purpose == BOOK)
        .map(|c| BookOut {
            conv: c.id,
            name: app
                .contacts
                .book_name(c.id)
                .unwrap_or_else(|| if c.title.is_empty() { "Contact list".into() } else { c.title.clone() }),
            count: app.contacts.count(c.id),
            yours: c.role == Role::Owner,
            people: c
                .members
                .iter()
                .filter(|u| !u.app)
                .map(|u| SharedWith { user: u.clone(), owner: c.owners.contains(&u.id) })
                .collect(),
            groups: serde_json::from_str::<BookSharing>(&app.contacts.book_sharing(c.id))
                .map(|s| s.groups)
                .unwrap_or_default()
                .into_iter()
                .map(|g| {
                    let chat = inner.groups.get(&g.conv);
                    SharedGroup {
                        conv: g.conv,
                        title: chat.map(|c| c.title.clone()).unwrap_or_else(|| "A group you're not in".into()),
                        count: chat.map_or(0, |c| c.member_count),
                        owner: g.owner,
                    }
                })
                .collect(),
        })
        .collect();
    list.sort_by_key(|b| b.name.to_lowercase());
    list
}

/// A new shared address book, shared with `people`.
pub fn book_new<H: Host>(
    app: &Core<H>,
    name: String,
    people: Vec<Sharing>,
    groups: Vec<GroupShare>,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give it a name.".into());
    }
    let mut inner = app.inner.lock().unwrap();
    if inner.me.is_none() {
        return Err("Connect to the server to share a contact list.".into());
    }
    // Found again by its title when the server answers.
    inner.pending_books.push((name.clone(), people, groups));
    app.tell(ToServer::CreateChannel { title: name, open: false, purpose: BOOK.into() });
    Ok(())
}

/// Shares a book you can change with more people and groups, takes people
/// out, and says who can change it. `groups`: every group it's shared with
/// from now on.
#[allow(clippy::too_many_arguments)] // The share dialog, as it's filled in.
pub fn book_share<H: Host>(
    app: &Core<H>,
    conv: i64,
    add: Vec<Sharing>,
    remove: Vec<String>,
    roles: Vec<Sharing>,
    groups: Vec<GroupShare>,
) -> Result<(), String> {
    let inner = app.inner.lock().unwrap();
    let Some(book) = inner.hidden.get(&conv).filter(|c| c.role == Role::Owner) else {
        return Err("Only people who can change this contact list can share it.".into());
    };
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    for m in share_posts(conv, &add) {
        app.tell(m);
    }
    for user in &remove {
        app.tell(ToServer::RemovePerson { conv, user: user.clone() });
    }
    for r in roles {
        app.tell(ToServer::SetRole { conv, user: r.username, owner: r.owner });
    }
    // Who it's shared with one by one: as before (everyone in it, the first
    // time groups come in), plus those added, less those taken out.
    let before = sharing_of(app, conv);
    if before.is_some() || !groups.is_empty() {
        let mut people: Vec<String> = match &before {
            Some(s) => s.people.clone(),
            None => book.members.iter().filter(|u| u.id != me && !u.app).map(|u| u.username.clone()).collect(),
        };
        people.extend(add.iter().map(|a| a.username.clone()));
        people.retain(|p| !remove.contains(p));
        people.sort();
        people.dedup();
        let sharing = BookSharing { groups, people };
        if before.as_ref() != Some(&sharing) {
            app.contacts.set_book_sharing(conv, &serde_json::to_string(&sharing).unwrap_or_default());
            app.tell(sharing_post(conv, &sharing));
        }
        for m in keep_book_members(app, &inner, conv) {
            app.tell(m);
        }
    }
    Ok(())
}

/// Renames a book you can change.
pub fn book_rename<H: Host>(app: &Core<H>, conv: i64, name: String) -> Result<(), String> {
    let inner = app.inner.lock().unwrap();
    if !inner.hidden.get(&conv).is_some_and(|c| c.role == Role::Owner) || name.trim().is_empty() {
        return Err("Only people who can change this contact list can rename it.".into());
    }
    app.contacts.set_book_name(conv, name.trim());
    app.tell(ToServer::Send {
        conv,
        client_id: random_id(),
        text: format!("{BOOK_NAME}{}", serde_json::json!({ "name": name.trim() })),
    });
    app.emit("contacts", ());
    Ok(())
}

/// Stops following a shared book (it leaves this computer).
pub fn book_leave<H: Host>(app: &Core<H>, conv: i64) {
    app.tell(ToServer::Leave { conv });
}

/// Your out of office.
pub fn my_away<H: Host>(app: &Core<H>) -> Option<Away> {
    app.inner.lock().unwrap().away.clone()
}

pub fn away_get<H: Host>(app: &Core<H>) -> Away {
    app.inner.lock().unwrap().away.clone().unwrap_or(Away {
        notice: true,
        block: true,
        decline: true,
        email_to: "contacts".into(),
        ..Away::default()
    })
}

/// Saves your out of office: on the server (so people are told while this
/// computer is off) and, if it blocks your calendar, as times away in it.
pub fn away_save<H: Host>(app: &Core<H>, mut away: Away) -> Result<(), String> {
    away.times.retain(|(a, b)| b > a);
    away.times.sort();
    if !["everyone", "contacts", "nobody"].contains(&away.email_to.as_str()) {
        away.email_to = "contacts".into();
    }
    {
        let mut inner = app.inner.lock().unwrap();
        let me = inner.me.as_ref().map(|u| u.id).ok_or("Connect to the server to set out of office.")?;
        away.user = me;
        inner.away = Some(away.clone());
    }
    app.tell(ToServer::SetAway { away: away.clone() });
    let cal = &app.cal;
    for e in cal.away_events()? {
        cal.delete(e.id)?;
    }
    if away.block {
        for (start, end) in &away.times {
            cal.save(&Event {
                title: "Away".into(),
                start: *start,
                end: *end,
                all_day: true,
                kind: "away".into(),
                ..Event::default()
            })?;
        }
    }
    calendar_changed(app);
    app.emit("calendar", ());
    Ok(())
}

// The screens' commands.

pub fn meetings_list<H: Host>(app: &Core<H>) -> Vec<MeetingOut> {
    Meetings::all_out(&app.inner.lock().unwrap())
}

/// A new meeting with people on the server (user names) and `details`
/// (its guests are invited by email).
pub fn meeting_new<H: Host>(app: &Core<H>, mut details: Details, people: Vec<String>) -> Result<(), String> {
    check(&details)?;
    let mut inner = app.inner.lock().unwrap();
    if inner.me.is_none() {
        return Err("Connect to the server to invite people.".into());
    }
    details.uid = format!("{}@xenify", random_id());
    invited(app, &inner, &details, &people);
    if !details.guests.is_empty() && details.mail_from.is_empty() {
        details.mail_from = mail_from(app)?;
    }
    // The conversation's title only finds it again: the name is in the details.
    let token = format!("meeting-{}", random_id());
    inner.pending.push((token.clone(), details));
    app.tell(ToServer::CreateGroup { title: token, members: people, purpose: MEETING.into() });
    Ok(())
}

/// People you invite go to My contacts (unless they're in a book already).
fn invited<H: Host>(app: &Core<H>, inner: &Inner, d: &Details, people: &[String]) {
    let st = &app.contacts;
    let day = today_words(app);
    let mut added = false;
    for g in &d.guests {
        added |= st.add_if_new(&g.name, &g.email, "", "invited them", &day);
    }
    for username in people {
        let name = inner.chat_people_names.get(username).cloned().unwrap_or_else(|| username.clone());
        added |= st.add_if_new(&name, "", username, "invited them", &day);
    }
    if added {
        send_contacts(app, inner);
        app.emit("contacts", ());
    }
}

fn check(d: &Details) -> Result<(), String> {
    if d.title.trim().is_empty() {
        return Err("Give it a name.".into());
    }
    if d.end < d.start {
        return Err("It ends before it starts.".into());
    }
    if let Some(g) = d.guests.iter().find(|g| !g.email.contains('@')) {
        return Err(format!("“{}” isn't an email address.", g.email));
    }
    Ok(())
}

/// The account invitations go from: the one chosen in Settings, else the first.
fn mail_from<H: Host>(app: &Core<H>) -> Result<String, String> {
    let chosen = settings_of(&app.cal).mail_from;
    let accounts = app.host.mail_accounts();
    if accounts.contains(&chosen) {
        return Ok(chosen);
    }
    accounts
        .into_iter()
        .next()
        .ok_or_else(|| "To invite people by email, add your email account first (Settings › Email).".to_string())
}

/// Changes a meeting you organise: new details, people added and taken out.
#[allow(clippy::too_many_arguments)] // The meeting's changes, as the screens give them.
pub fn meeting_change<H: Host>(
    app: &Core<H>,
    conv: i64,
    mut details: Details,
    add: Vec<String>,
    remove: Vec<String>,
    // Its bookings so far: made again for the new time and rooms.
    bookings: Vec<i64>,
) -> Result<(), String> {
    check(&details)?;
    let mut inner = app.inner.lock().unwrap();
    let was = inner.meetings.get(&conv).and_then(|m| m.details.clone()).ok_or("That meeting isn't here any more.")?;
    details.uid = was.uid.clone();
    details.sequence = was.sequence + 1;
    if !details.guests.is_empty() && details.mail_from.is_empty() {
        details.mail_from = if was.mail_from.is_empty() { mail_from(app)? } else { was.mail_from.clone() };
    }
    // Keep the answers of guests still invited.
    for g in &mut details.guests {
        if let Some(old) = was.guests.iter().find(|o| o.email.eq_ignore_ascii_case(&g.email)) {
            g.answer =
                if details.start == was.start && details.end == was.end { old.answer.clone() } else { String::new() };
        }
    }
    invited(app, &inner, &details, &add);
    if !add.is_empty() {
        app.tell(ToServer::AddPeople { conv, users: add });
    }
    for user in remove {
        app.tell(ToServer::RemovePerson { conv, user });
    }
    app.tell(details_post(conv, &details));
    if details.start != was.start || details.end != was.end || details.things != was.things {
        for booking in bookings {
            app.tell(ToServer::CancelBooking { booking });
        }
        book_things(app, conv, &details);
    }
    // Guests taken off the list hear it's off for them.
    let dropped: Vec<Guest> = was
        .guests
        .iter()
        .filter(|o| !details.guests.iter().any(|g| g.email.eq_ignore_ascii_case(&o.email)))
        .cloned()
        .collect();
    if !dropped.is_empty() {
        email_guests(app, &inner, conv, &Details { guests: dropped, ..details.clone() }, true);
    }
    email_guests(app, &inner, conv, &details, false);
    if let Some(m) = inner.meetings.get_mut(&conv) {
        m.details = Some(details);
    }
    let _ = file_meeting(app, &inner, conv);
    meetings_changed(app, &inner);
    Ok(())
}

/// Calls a meeting off (you organise it): everyone is told.
pub fn meeting_cancel<H: Host>(app: &Core<H>, conv: i64, bookings: Vec<i64>) -> Result<(), String> {
    for booking in bookings {
        app.tell(ToServer::CancelBooking { booking });
    }
    let mut inner = app.inner.lock().unwrap();
    let mut d = inner.meetings.get(&conv).and_then(|m| m.details.clone()).ok_or("That meeting isn't here any more.")?;
    d.cancelled = true;
    d.sequence += 1;
    app.tell(details_post(conv, &d));
    email_guests(app, &inner, conv, &d, true);
    if let Some(m) = inner.meetings.get_mut(&conv) {
        m.details = Some(d);
    }
    let _ = file_meeting(app, &inner, conv);
    meetings_changed(app, &inner);
    Ok(())
}

/// Your answer to an invitation: "yes", "maybe" or "no".
pub fn meeting_answer<H: Host>(app: &Core<H>, conv: i64, answer: String) -> Result<(), String> {
    if !["yes", "maybe", "no"].contains(&answer.as_str()) {
        return Err("Answer yes, maybe or no.".into());
    }
    let mut inner = app.inner.lock().unwrap();
    let me = inner.me.as_ref().map(|u| u.id).ok_or("Connect to the server to answer.")?;
    let m = inner.meetings.get_mut(&conv).ok_or("That meeting isn't here any more.")?;
    // Shown at once; the post confirms it.
    let next = m.answers.values().map(|(_, s)| *s).max().unwrap_or(0).max(m.details_seq) + 1;
    m.answers.insert(me, (answer.clone(), next));
    app.tell(ToServer::Send { conv, client_id: random_id(), text: format!("{ANSWER}{answer}") });
    let _ = file_meeting(app, &inner, conv);
    meetings_changed(app, &inner);
    drop(inner);
    calendar_changed(app);
    Ok(())
}

/// Find a time: these people's busy times arrive as `busy` events (yours
/// from this computer).
pub fn meeting_busy<H: Host>(app: &Core<H>, users: Vec<i64>) {
    let mut inner = app.inner.lock().unwrap();
    let me = inner.me.as_ref().map_or(0, |u| u.id);
    let mut out = Vec::new();
    let mut ask = Vec::new();
    for u in users {
        if u == me {
            let cal = &app.cal;
            let from = start_of_today(app) - 7 * DAY;
            out.push(BusyOut {
                user: me,
                known: true,
                busy: cal.busy(from, from + AHEAD + 7 * DAY).unwrap_or_default(),
                work: working_hours(&settings_of(cal), from, AHEAD / DAY + 7, app.cal.tz()),
                away: away_times(cal),
            });
            continue;
        }
        if let Some(b) = inner.busy_of.get(&u) {
            out.push(b.clone());
        }
        ask.push(u);
    }
    if !out.is_empty() {
        app.emit("busy", out);
    }
    if !ask.is_empty() {
        inner.wanted.extend(ask.iter().copied());
        app.tell(ToServer::BusyOf { users: ask });
    }
}

pub fn calendar_settings<H: Host>(app: &Core<H>) -> Settings {
    settings_of(&app.cal)
}

pub fn calendar_settings_save<H: Host>(app: &Core<H>, settings: Settings) -> Result<(), String> {
    let parse = |t: &str| NaiveTime::parse_from_str(t, "%H:%M").is_ok();
    if !parse(&settings.work_from) || !parse(&settings.work_to) || settings.work_to <= settings.work_from {
        return Err("Working hours need a start and a later end.".into());
    }
    if !["everyone", "chats", "nobody"].contains(&settings.share.as_str()) {
        return Err("Choose who can see when you're busy.".into());
    }
    app.cal.set_meta("calendar_settings", &serde_json::to_string(&settings).unwrap_or_default());
    let inner = app.inner.lock().unwrap();
    keep_busy_followers(app, &inner);
    send_busy(app, &inner);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_hours_are_weekdays_only() {
        // Monday 26 October 2026, 00:00 UTC (tests run in UTC unless TZ says).
        let monday = 1_793_059_200_000;
        let s = Settings::default();
        let w = working_hours(&s, monday, 7, chrono_tz::UTC);
        assert_eq!(w.len(), 5);
        assert_eq!(w[0], (monday + 8 * 3_600_000, monday + 18 * 3_600_000));
        let none = Settings { work_days: Vec::new(), ..Settings::default() };
        assert!(working_hours(&none, monday, 7, chrono_tz::UTC).is_empty());
    }

    #[test]
    fn meeting_posts_are_read_in_order() {
        let alex = User { id: 1, username: "alex".into(), ..Default::default() };
        let priya = User { id: 2, username: "priya".into(), ..Default::default() };
        let mut inner = Inner { me: Some(priya.clone()), ..Inner::default() };
        inner.hidden.insert(
            9,
            Conversation {
                id: 9,
                kind: Kind::Group,
                title: String::new(),
                members: vec![alex.clone(), priya.clone()],
                member_count: 2,
                owners: vec![1],
                role: Role::Member,
                can_post: true,
                open: false,
                last: None,
                last_seq: 3,
                read_seq: 0,
                notify: Default::default(),
                muted_until: None,
                purpose: MEETING.into(),
                keep_days: None,
            },
        );
        let post = |seq, sender: &User, text: String| Message {
            id: seq,
            conv: 9,
            seq,
            sender: sender.clone(),
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
        };
        let d = |title: &str| {
            format!(
                "{MEET}{}",
                serde_json::to_string(&Details { title: title.into(), start: 10, end: 20, ..Details::default() })
                    .unwrap()
            )
        };
        // Newest first, as a page might come; Priya can't change the details.
        let changed = read_meeting(
            &mut inner,
            9,
            &[
                post(3, &priya, format!("{ANSWER}maybe")),
                post(2, &alex, d("Launch review")),
                post(1, &alex, d("Launch")),
                post(4, &priya, d("Mine")),
            ],
        );
        assert!(changed);
        let out = Meetings::meeting_out(&inner, 9).unwrap();
        assert_eq!(out.details.title, "Launch review");
        assert_eq!(out.mine, "maybe");
        assert!(!out.yours);
        assert_eq!(out.people[0].user.id, 1, "organiser first");
        assert_eq!(out.people[0].answer, "yes");
        // An older answer doesn't undo a newer one.
        read_meeting(&mut inner, 9, &[post(2, &priya, format!("{ANSWER}no"))]);
        assert_eq!(Meetings::meeting_out(&inner, 9).unwrap().mine, "maybe");
    }
}
