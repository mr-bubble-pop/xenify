//! What Xenify Chat and its server say to each other: one JSON
//! object per WebSocket message, tagged by `"t"`. Signing in is plain HTTP
//! (`/v1/login`, `/v1/register`), which hands out a session token.
//!
//! Conversations come in three kinds: direct chats (two people), groups
//! (everyone talks; anyone in it can add people; anyone can leave) and
//! channels (owners and apps post, others follow; open ones can be found
//! and subscribed to). Message search happens in the app, on its own copy.

use serde::{Deserialize, Serialize};

/// Bumped when the protocol changes in a way older apps can't follow.
pub const VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    /// An app (a bot or a feed), not a person.
    #[serde(default)]
    pub app: bool,
}

impl User {
    /// The name to show: the display name, else the user name.
    pub fn name(&self) -> &str {
        if self.display_name.is_empty() { &self.username } else { &self.display_name }
    }
}

/// `POST /v1/login` or `/v1/register` (with an invite code unless the
/// server is open).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Login {
    pub username: String,
    pub password: String,
    /// What this device is called ("Alex's laptop"), shown in sessions.
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub invite: String,
    /// When creating an account: the name others see ("Alex Lane").
    #[serde(default)]
    pub display_name: String,
}

/// The answer to signing in: keep the token (it's the session).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub user: User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Direct,
    Group,
    Channel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Member,
}

/// When to be told about new messages in a conversation (each person's own
/// choice; "muted until" is separate).
/// Who an app talks with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl Conversation {
    /// Something you haven't seen.
    pub fn unread(&self) -> bool {
        self.last_seq > self.read_seq
    }
}

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
    },
    CreateChannel {
        title: String,
        open: bool,
    },
    /// Follow an open channel.
    Subscribe {
        conv: i64,
    },
    /// Add people to a group (anyone in it) or channel (owners).
    AddPeople {
        conv: i64,
        users: Vec<String>,
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
    Ping,
}

/// From the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToApp {
    Welcome {
        user: User,
        version: u32,
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
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tag: String,
    },
    Typing {
        conv: i64,
        user: User,
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
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

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
