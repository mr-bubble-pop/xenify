//! What Xenify Chat and its server say to each other: one JSON
//! object per WebSocket message, tagged by `"t"`. Signing in happens on the
//! Xenify server's account API (`xenify-server-proto`), which hands out a
//! session token.
//!
//! Conversations come in three kinds: direct chats (two people), groups
//! (everyone talks; anyone in it can add people; anyone can leave) and
//! channels (owners and apps post, others follow; open ones can be found
//! and subscribed to). The server searches messages, keeps each person's
//! settings and reminders, and deletes messages older than the server's
//! (or a chat's) limit.

use serde::{Deserialize, Serialize};

/// Bumped when the protocol changes in a way older apps can't follow.
///
/// Changes that older apps and servers can live with need no bump: new
/// optional fields (`#[serde(default)]`), new message types (each side drops
/// a message it can't read), and new values of the small enums below, which
/// read as their `lenient!` fallback on an older side. See docs/formats.md.
pub const VERSION: u32 = 4;

/// Reads an enum from its snake_case name; a name this version doesn't know
/// (sent by a newer app or server) reads as `$fallback` instead of making
/// the whole message unreadable.
macro_rules! lenient {
    ($t:ident, $fallback:ident, { $($name:literal => $v:ident),* $(,)? }) => {
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = std::borrow::Cow::<str>::deserialize(d)?;
                Ok(match s.as_ref() { $($name => $t::$v,)* _ => $t::$fallback })
            }
        }
    };
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    /// Under the hood only: people are known by their name and email address.
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    /// An app (a bot or a feed), not a person.
    #[serde(default)]
    pub app: bool,
    /// What they sign in with (where the server has it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
}

impl User {
    /// The name to show: the display name, else the user name.
    pub fn name(&self) -> &str {
        if self.display_name.is_empty() { &self.username } else { &self.display_name }
    }
}

/// Signed in: keep the token (it's the session).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub user: User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Direct,
    Group,
    Channel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Member,
}

/// When to be told about new messages in a conversation (each person's own
/// choice; "muted until" is separate).
/// Who an app talks with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    /// Each person in their own 1:1 chat with it; nobody sees anyone else's.
    #[default]
    Personal,
    /// In the groups and channels it's added to, when mentioned.
    Shared,
    Both,
}

/// An app (a bot or feed) as its owner manages it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub user: User,
    #[serde(default)]
    pub description: String,
    /// Where the server sends messages for the app (empty: nowhere; shown
    /// only to the owner).
    #[serde(default)]
    pub webhook: String,
    pub mode: AppMode,
    /// Who made it.
    pub owner: i64,
}

/// Whether someone's around: what they chose (online, away, busy), or
/// offline when no app of theirs is connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    #[default]
    Online,
    Away,
    /// Do not disturb.
    Busy,
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceOf {
    pub user: i64,
    pub state: Presence,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Notify {
    #[default]
    All,
    Mentions,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub conv: i64,
    /// Order within the conversation (1, 2, 3…).
    pub seq: i64,
    pub sender: User,
    pub text: String,
    /// Unix milliseconds.
    pub at: i64,
    /// The sender's own id for it, so an app knows its message arrived.
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub edited: bool,
    #[serde(default)]
    pub deleted: bool,
    /// Quick reactions (👍, ❤️…), each with who chose it, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    /// A reply in a thread: the `seq` of the message it replies to (None:
    /// in the chat itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<i64>,
    /// A message with replies: how many, and the newest one's `seq` and
    /// time (Unix ms).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub replies: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_reply_seq: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_reply_at: i64,
    /// A reply: who's in its thread (whoever wrote the first message or
    /// replied), so their apps tell them about it; others aren't told.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thread_people: Vec<i64>,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<User>,
}

/// A conversation as one person sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: i64,
    pub kind: Kind,
    /// Empty for direct chats (show the other person).
    #[serde(default)]
    pub title: String,
    /// The people in it (the first 50 for big channels; see `member_count`).
    pub members: Vec<User>,
    pub member_count: i64,
    /// Who among `members` owns it (for a channel: who posts there, with
    /// any apps in `members`).
    #[serde(default)]
    pub owners: Vec<i64>,
    pub role: Role,
    /// Whether you can write in it (channels: owners and apps only).
    pub can_post: bool,
    /// Channels: anyone can find it and subscribe.
    #[serde(default)]
    pub open: bool,
    /// The newest message, for the list.
    pub last: Option<Message>,
    pub last_seq: i64,
    /// What you've read up to (on any of your devices).
    pub read_seq: i64,
    pub notify: Notify,
    /// Muted until then (Unix ms), if muted for a while.
    #[serde(default)]
    pub muted_until: Option<i64>,
    /// Not a chat but something the app keeps in one (empty for chats):
    /// [`MEETING`], [`CALENDAR`] or [`BUSY`]. Apps leave these out of the
    /// chat list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    /// Groups and channels: how many days its owners keep messages (None:
    /// as long as the server does). The server's own limit wins when it's
    /// shorter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<i64>,
}

/// A group holding one meeting: the organiser (its owner) posts the
/// details, everyone invited answers in it, and its call is the meeting's.
pub const MEETING: &str = "meeting";
/// A group of one: your calendar, shared between your devices.
pub const CALENDAR: &str = "calendar";
/// A channel of one owner's busy times, followed by whoever may see them.
pub const BUSY: &str = "busy";
/// A group of one: your own address book (My contacts).
pub const CONTACTS: &str = "contacts";
/// A shared address book: a channel whose owners can change it and whose
/// followers can look.
pub const BOOK: &str = "book";
/// A shared calendar: a channel whose owners can change it and whose other
/// people can look.
pub const SHARED_CAL: &str = "sharedcal";

impl Conversation {
    /// Something you haven't seen.
    pub fn unread(&self) -> bool {
        self.last_seq > self.read_seq
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallAction {
    Start,
    Join,
    Leave,
    Decline,
    /// Start recording the call (everyone's app turns everything off).
    Record,
    /// The person recording stops.
    StopRecording,
    /// Your app turned your microphone, camera and screen off for the
    /// recording.
    RecordingOff,
    /// You turned something back on, so you're in the recording.
    InRecording,
}

/// A recording going on in a call. The recording is made by `by`'s app,
/// which only keeps the sound and pictures of the people in `in`; the
/// server only keeps track of who's in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRecording {
    pub by: User,
    /// Counts up with each recording in the call (apps react to a new one).
    pub round: i64,
    /// When it started (Unix ms).
    pub since: i64,
    /// Whose apps have turned everything off for it.
    #[serde(default)]
    pub off: Vec<i64>,
    /// Who turned something back on, and so is in it.
    #[serde(default, rename = "in")]
    pub in_it: Vec<i64>,
}

/// A call in a conversation, as everyone in it sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfo {
    pub conv: i64,
    pub id: String,
    /// Who started it.
    pub by: User,
    /// Who's in it now (empty: it ended).
    pub people: Vec<User>,
    pub video: bool,
    /// When it started (Unix ms).
    pub at: i64,
    /// It ended because the person called said no (direct chats).
    #[serde(default)]
    pub declined: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<CallRecording>,
    /// This server lets people record calls (an admin setting).
    #[serde(default)]
    pub can_record: bool,
}

/// A reminder kept on the server, shown on all your devices when it's due.
/// The app makes them from your calendar (events, meetings and "Remind
/// me…"), keyed by what they're about, and sends the whole list again when
/// it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    /// The app's own name for it (an event's uid and start, say).
    pub key: String,
    /// When to remind (Unix ms; moved by snoozing).
    pub at: i64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// When what it's about starts (meetings and events), for "1 minute
    /// before it starts"; 0 for plain reminders.
    #[serde(default)]
    pub starts: i64,
    /// What it's about, as the app describes it (JSON; opened with Open).
    #[serde(default)]
    pub link: String,
}

/// How search results are ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    /// The best matches first (among the newest 2,000 matches when a word
    /// is in more messages than that).
    #[default]
    Best,
    Newest,
    Oldest,
}

/// How many messages a page of search results has.
pub const SEARCH_PAGE: usize = 30;

/// A message found by search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub msg: Message,
    /// The part of the text that matched, with each match between
    /// [`MARK_START`] and [`MARK_END`].
    pub snippet: String,
}

/// What a server's roles (Admin › Roles) let people do in Chat: make
/// channels, make channels anyone can follow, make apps and API keys, and
/// moderate (take down anyone's messages and take people out of groups
/// and channels they're in).
pub const PERMS: [&str; 4] = ["chat.create_channel", "chat.create_open_channel", "chat.create_apps", "chat.moderate"];

/// Marks a match in [`Hit::snippet`].
pub const MARK_START: char = '\u{1}';
pub const MARK_END: char = '\u{2}';

/// A group or channel found by search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub id: i64,
    pub kind: Kind,
    pub title: String,
    pub member_count: i64,
    /// You're already in it.
    pub joined: bool,
}

/// From the app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToServer {
    /// Your conversations (answered with `Conversations`).
    List,
    /// The direct chat with someone (made the first time).
    OpenDirect {
        user: String,
    },
    CreateGroup {
        title: String,
        members: Vec<String>,
        /// See [`Conversation::purpose`]. You have one calendar: asking for
        /// another answers with the one you have.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        purpose: String,
    },
    CreateChannel {
        title: String,
        open: bool,
        /// See [`Conversation::purpose`] (one busy-times channel each).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        purpose: String,
    },
    /// Channels (owners): whether anyone can find and subscribe to it.
    SetOpen {
        conv: i64,
        open: bool,
    },
    /// These people's busy-times channels (answered with `BusyOf`).
    BusyOf {
        users: Vec<i64>,
    },
    /// Owners: makes someone an owner too, or not. Shared address books:
    /// whether someone can change it or only look; groups and channels:
    /// co-owners.
    SetRole {
        conv: i64,
        user: String,
        owner: bool,
    },
    /// Rooms and things you can book (answered with `Things`).
    Things,
    /// Admins: adds (`id` 0) or changes something to book.
    SaveThing {
        thing: Thing,
    },
    /// Admins: takes something away (its bookings go with it).
    RemoveThing {
        id: i64,
    },
    /// Bookings of these things between `from` and `to` (answered with
    /// `Bookings`).
    Bookings {
        things: Vec<i64>,
        from: i64,
        to: i64,
    },
    /// Your bookings, and requests waiting for you to answer (`Bookings`).
    MyBookings,
    /// Books something from `start` to `end` (Unix ms), perhaps for a
    /// meeting. Answered with `Booked`, or an error when it's taken.
    Book {
        thing: i64,
        start: i64,
        end: i64,
        #[serde(default)]
        meeting: Option<i64>,
        /// Echoed in the answer.
        #[serde(default)]
        tag: String,
    },
    /// Whoever looks after the thing: yes or no to a request.
    AnswerBooking {
        booking: i64,
        yes: bool,
    },
    /// The booker (or whoever looks after it): calls a booking off.
    CancelBooking {
        booking: i64,
    },
    /// Out of office: when you're away and what people are told.
    SetAway {
        away: Away,
    },
    /// Whether these people are away (answered with `AwayOf`).
    AwayOf {
        users: Vec<i64>,
    },
    /// Follow an open channel.
    Subscribe {
        conv: i64,
    },
    /// Add people to a group (anyone in it) or channel (owners). They see
    /// the earlier messages too.
    AddPeople {
        conv: i64,
        users: Vec<String>,
    },
    /// Calls: `start` rings everyone in the conversation, `join` answers,
    /// `leave` hangs up, `decline` says no to a ringing call. Sound and
    /// pictures then go over `/v1/call/{call}/media`.
    Call {
        conv: i64,
        action: CallAction,
        /// Starting a video call (rather than voice only).
        #[serde(default)]
        video: bool,
    },
    /// Groups (anyone in it) and channels (owners): a new name.
    Rename {
        conv: i64,
        title: String,
    },
    /// Take someone out (owners only; to go yourself, `Leave`).
    RemovePerson {
        conv: i64,
        user: String,
    },
    /// Leave a group, or unsubscribe from a channel.
    Leave {
        conv: i64,
    },
    Send {
        conv: i64,
        client_id: String,
        text: String,
    },
    /// A reply in the thread of message `thread` (its `seq`), when the
    /// server has threads (`Welcome::threads`).
    Reply {
        conv: i64,
        thread: i64,
        client_id: String,
        text: String,
    },
    /// A thread: the message `thread` and all its replies (answered with
    /// `History` carrying `thread`).
    Thread {
        conv: i64,
        thread: i64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    Edit {
        conv: i64,
        seq: i64,
        text: String,
    },
    Delete {
        conv: i64,
        seq: i64,
    },
    /// Adds (`on`) or takes back your reaction on a message.
    React {
        conv: i64,
        seq: i64,
        emoji: String,
        on: bool,
    },
    /// Messages before `before` (None: the newest), newest last.
    History {
        conv: i64,
        before: Option<i64>,
        limit: u32,
        /// Echoed in the answer, so an app can tell its requests apart.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    /// You've read up to `seq`.
    Read {
        conv: i64,
        seq: i64,
    },
    /// Marks everything after `seq` as not read yet ("mark unread from here").
    Unread {
        conv: i64,
        seq: i64,
    },
    SetNotify {
        conv: i64,
        notify: Notify,
        muted_until: Option<i64>,
    },
    Typing {
        conv: i64,
    },
    /// What others see while you're connected: online, away or busy.
    SetStatus {
        status: Presence,
    },
    /// Whether these people are around (answered with `Presence`).
    GetPresence {
        users: Vec<i64>,
    },
    /// Makes an app you own (answered with `AppKey`, the only time its key
    /// and webhook secret are shown).
    CreateApp {
        name: String,
        #[serde(default)]
        display_name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        webhook: String,
        #[serde(default)]
        mode: AppMode,
    },
    UpdateApp {
        app: i64,
        display_name: String,
        description: String,
        webhook: String,
        mode: AppMode,
    },
    /// A new key and webhook secret for an app (the old ones stop working).
    NewAppKey {
        app: i64,
    },
    DeleteApp {
        app: i64,
    },
    /// Your apps (answered with `Apps`).
    ListApps,
    /// The apps in a conversation, and how each talks (answered with `Apps`).
    AppsIn {
        conv: i64,
    },
    /// A key for using the HTTP API yourself, e.g. from a script (answered
    /// with `ApiKey`; shown once).
    NewApiKey,
    /// Changes the name others see (answered with `Welcome`).
    SetName {
        display_name: String,
    },
    /// People on the server by name.
    FindPeople {
        q: String,
    },
    /// Your groups and channels, and open channels, by title.
    FindConversations {
        q: String,
    },
    /// Messages you may read containing these words (each word or the
    /// start of one), a page ([`SEARCH_PAGE`]) at a time in `sort` order;
    /// in one conversation with `conv`. `after`: the previous page's
    /// `Hits::more`, for the next page (answered with `Hits`).
    Search {
        q: String,
        #[serde(default)]
        conv: Option<i64>,
        #[serde(default)]
        sort: SearchSort,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        after: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    /// A question for the server's AI about your messages (when
    /// `Welcome::ai`): it finds messages you may read that may help and
    /// answers from those alone (answered with `Answer`). Nobody else's
    /// messages ever go to the AI.
    Ask {
        q: String,
        /// Only this conversation (None: all of yours).
        #[serde(default)]
        conv: Option<i64>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    /// Groups and channels (owners): keep messages this many days (None:
    /// as long as the server does).
    SetKeep {
        conv: i64,
        days: Option<i64>,
    },
    /// Your Chat settings, kept with your account so all your devices share
    /// them (whatever the app keeps; the server reads `digest`, `quiet` and
    /// `tz`). Your other devices get `Settings`.
    SaveSettings {
        settings: serde_json::Value,
    },
    /// Your reminders from now on, replacing those sent before (one that
    /// keeps its key and time keeps its snoozing and showing).
    SetReminders {
        list: Vec<Reminder>,
    },
    /// Remind again at `until` (Unix ms).
    Snooze {
        key: String,
        until: i64,
    },
    /// Done with a reminder: your other devices stop showing it.
    Dismiss {
        key: String,
    },
    Ping,
}

/// From the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToApp {
    Welcome {
        user: User,
        version: u32,
        /// Your Chat settings (see `SaveSettings`; empty at first).
        #[serde(default)]
        settings: serde_json::Value,
        /// How many days the server keeps messages (None: for ever).
        #[serde(default)]
        keep_days: Option<i64>,
        /// Which of [`PERMS`] you have (the server's roles decide).
        #[serde(default)]
        perms: Vec<String>,
        /// The server's clock (Unix ms), so apps on computers whose clocks
        /// are off can still tell how long ago something happened.
        #[serde(default)]
        now: i64,
        /// People can reply in threads (Admin › Network › Chat; on unless
        /// an admin turned it off). Off: replies show in the chat itself.
        #[serde(default)]
        threads: bool,
        /// You may ask the server's AI about your messages (`Ask`).
        #[serde(default)]
        ai: bool,
    },
    /// Your settings changed (on another device).
    Settings {
        settings: serde_json::Value,
    },
    /// Answers `Search`.
    Hits {
        q: String,
        hits: Vec<Hit>,
        #[serde(default)]
        sort: SearchSort,
        /// How many messages match, counting up to 1,001 (more: "1,000+").
        #[serde(default)]
        total: i64,
        /// Ask with this as `after` for the next page (empty: no more).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        more: String,
        /// Echoes `Search::after` (empty: the first page).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        after: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    /// Answers `Ask`: the AI's answer, which cites the messages it used as
    /// [1], [2]… (`sources`, in that order; all of them messages you may
    /// read), or what went wrong (`error`).
    Answer {
        q: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        sources: Vec<Hit>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        error: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    /// A reminder is due (to every device of yours).
    Reminder {
        reminder: Reminder,
    },
    /// A reminder was snoozed or dismissed (on one of your devices).
    ReminderGone {
        key: String,
    },
    /// All your conversations, most recently active first.
    Conversations {
        list: Vec<Conversation>,
    },
    /// One conversation, new or changed.
    Conversation {
        conv: Conversation,
    },
    /// You're no longer in it (left, or taken out).
    Removed {
        conv: i64,
    },
    /// A new message, or one edited or deleted.
    Message {
        msg: Message,
    },
    History {
        conv: i64,
        messages: Vec<Message>,
        /// Answers `Thread`: whose thread (its first message and replies).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<i64>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    Typing {
        conv: i64,
        user: User,
    },
    /// A call started, someone joined or left, or it ended.
    Call {
        call: CallInfo,
    },
    /// You read up to `seq` (on another device, perhaps).
    Read {
        conv: i64,
        seq: i64,
    },
    People {
        q: String,
        users: Vec<User>,
    },
    Found {
        q: String,
        conversations: Vec<Found>,
    },
    Error {
        code: String,
        message: String,
    },
    Apps {
        list: Vec<AppInfo>,
        /// The conversation asked about with `AppsIn` (0 for `ListApps`).
        #[serde(default)]
        conv: i64,
    },
    /// A new app key (and, for webhooks, the secret that signs them).
    AppKey {
        app: AppInfo,
        key: String,
        secret: String,
    },
    ApiKey {
        key: String,
    },
    /// Who's around: sent when you connect (everyone you chat with), when
    /// asked, and whenever it changes.
    Presence {
        list: Vec<PresenceOf>,
    },
    Things {
        list: Vec<Thing>,
        /// You may add and change them.
        admin: bool,
    },
    Bookings {
        list: Vec<Booking>,
    },
    /// A booking made, answered or called off (to the booker and whoever
    /// looks after the thing); `tag` echoes `Book`'s.
    Booked {
        booking: Booking,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    /// Answers `AwayOf`: people with times away still to come or now.
    AwayOf {
        list: Vec<Away>,
    },
    /// Answers `BusyOf`: the channels there are (someone without one isn't
    /// listed).
    BusyOf {
        list: Vec<BusyChannel>,
    },
    Pong,
}

/// A room or thing people book.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thing {
    /// 0 for a new one.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    /// "room", "equipment", "book", "vehicle" or "other".
    pub kind: String,
    #[serde(default)]
    pub place: String,
    /// Rooms: how many it seats (0: not said).
    #[serde(default)]
    pub seats: i64,
    /// Booked by the day (books, cars) rather than by the hour (rooms).
    #[serde(default)]
    pub by_day: bool,
    /// Who says yes or no (None: yes by itself when it's free).
    #[serde(default)]
    pub approver: Option<User>,
}

/// A booking of a thing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Booking {
    pub id: i64,
    pub thing: i64,
    pub user: User,
    pub start: i64,
    pub end: i64,
    /// "yes", "asked" (waiting for whoever looks after it), "no" or
    /// "cancelled".
    pub status: String,
    /// The meeting it's for, seen only by the booker.
    #[serde(default)]
    pub meeting: Option<i64>,
}

/// Someone's out of office.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Away {
    #[serde(default)]
    pub user: i64,
    /// Times away (Unix ms), earliest first.
    pub times: Vec<(i64, i64)>,
    /// The automatic reply.
    #[serde(default)]
    pub text: String,
    /// Show a notice in chats with you.
    #[serde(default)]
    pub notice: bool,
    /// Your own settings (others' apps ignore them): block your calendar,
    /// say no to invitations then, and who gets the reply by email
    /// ("everyone", "contacts" or "nobody").
    #[serde(default)]
    pub block: bool,
    #[serde(default)]
    pub decline: bool,
    #[serde(default)]
    pub email_to: String,
    /// When it was turned on (Unix ms; set by the server): only messages
    /// after this get the automatic reply.
    #[serde(default)]
    pub since: i64,
}

/// Someone's busy-times channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusyChannel {
    pub user: i64,
    pub conv: i64,
    /// Anyone may subscribe.
    pub open: bool,
    /// You follow it already.
    pub member: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_enums_round_trip_and_unknown_values_fall_back() {
        for k in [Kind::Direct, Kind::Group, Kind::Channel] {
            assert_eq!(serde_json::from_value::<Kind>(serde_json::to_value(k).unwrap()).unwrap(), k);
        }
        for n in [Notify::All, Notify::Mentions, Notify::Off] {
            assert_eq!(serde_json::from_str::<Notify>(&serde_json::to_string(&n).unwrap()).unwrap(), n);
        }
        assert_eq!(serde_json::from_str::<Kind>("\"forum\"").unwrap(), Kind::Group);
        assert_eq!(serde_json::from_str::<Role>("\"admin\"").unwrap(), Role::Member);
        assert_eq!(serde_json::from_str::<Presence>("\"in_a_call\"").unwrap(), Presence::Online);
        assert_eq!(serde_json::from_str::<AppMode>("\"both\"").unwrap(), AppMode::Both);
    }

    #[test]
    fn messages_are_tagged_json() {
        let m = ToServer::Send { conv: 3, client_id: "a1".into(), text: "Hi".into() };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(s, r#"{"t":"send","conv":3,"client_id":"a1","text":"Hi"}"#);
        assert_eq!(serde_json::from_str::<ToServer>(&s).unwrap(), m);
        let n = ToServer::SetNotify { conv: 1, notify: Notify::Mentions, muted_until: None };
        assert!(serde_json::to_string(&n).unwrap().contains(r#""notify":"mentions""#));
    }
}

// Unknown values from newer versions: the least surprising reading.
lenient!(Kind, Group, { "direct" => Direct, "group" => Group, "channel" => Channel });
lenient!(Role, Member, { "owner" => Owner, "member" => Member });
lenient!(AppMode, Personal, { "personal" => Personal, "shared" => Shared, "both" => Both });
lenient!(Presence, Online, { "online" => Online, "away" => Away, "busy" => Busy, "offline" => Offline });
lenient!(Notify, All, { "all" => All, "mentions" => Mentions, "off" => Off });
