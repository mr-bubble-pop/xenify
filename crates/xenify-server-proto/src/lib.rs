//! What the Xenify apps, the admin site and the Xenify server say to each
//! other: JSON over HTTP under `/v1/`, and the live-editing WebSocket.
//!
//! The server is trusted: apps send passwords, names and contents as they
//! are, over HTTPS, and the server reads them (to search, preview, send
//! email). Apps and server must speak the same [`VERSION`]: there's no
//! support for older or newer ones, an app that finds a different one asks
//! for an update.

use serde::{Deserialize, Serialize};

/// The protocol level of what's said here. Additions older apps can live
/// with (new routes, optional fields, new `features`) need no bump; bump it
/// only when an older app or server would misbehave, and keep [`OLDEST`] as
/// low as this build can still serve. See docs/formats.md.
pub const VERSION: u32 = 2;

/// The oldest protocol level, of an app or of a server, this build still
/// works with. A server says it in `/v1/info` (`oldest_protocol`).
pub const OLDEST: u32 = 2;

/// Who has to update for an app and a server to work together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// The server no longer serves an app this old.
    UpdateApp,
    /// The server is older than this app can work with.
    UpdateServer,
}

impl Mismatch {
    pub fn says(self) -> &'static str {
        match self {
            Mismatch::UpdateApp => "This server is newer than this app: update Xenify (Xenify Hub can do it).",
            Mismatch::UpdateServer => "This server needs updating before this app can use it.",
        }
    }
}

/// Whether this app works with a server that says `protocol` and
/// `oldest_protocol` in its `/v1/info` (servers from before
/// `oldest_protocol` existed served only their own level).
pub fn works_with(protocol: u32, oldest_protocol: Option<u32>) -> Result<(), Mismatch> {
    if VERSION < oldest_protocol.unwrap_or(protocol) {
        Err(Mismatch::UpdateApp)
    } else if protocol < OLDEST {
        Err(Mismatch::UpdateServer)
    } else {
        Ok(())
    }
}

/// `GET /v1/info`: what an app or browser learns before signing in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    /// What people call this server ("Office server").
    pub name: String,
    /// The server program's version ("1.4.0").
    pub version: String,
    pub protocol: u32,
    /// The oldest app protocol level this server still serves (missing from
    /// servers made before it existed: they served only `protocol`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_protocol: Option<u32>,
    /// Stays the same for the server's life; links between documents name it.
    pub server_id: String,
    /// Nobody has set the server up yet.
    pub setup_needed: bool,
    /// What this server offers, so apps don't find out by trying:
    /// "web_apps" (Write and the others in the browser), "public_links",
    /// "mail" (it can send email: forgot password works).
    pub features: Vec<String>,
}

/// `POST /v1/login/start`: the first step of signing in, the user name
/// (or email address) alone.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginStart {
    pub username: String,
}

/// What comes after the user name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginNext {
    /// The password (also for names nobody has, so this doesn't tell who
    /// has an account).
    Password,
    /// An account an admin added that has no password yet: the code or link
    /// from the admin (or one emailed now) picks it.
    PickPassword,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginStep {
    pub next: LoginNext,
    /// The account has an email address and the server can send email, so
    /// "Email me a link" works.
    #[serde(default)]
    pub can_email: bool,
}

/// `POST /v1/login`. `username` may be an email address.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Login {
    pub username: String,
    pub password: String,
    /// What this device is called ("Alex's laptop"), shown in sessions.
    #[serde(default)]
    pub device: String,
    /// This computer was remembered after a second step
    /// ([`Session::remember`]): no second step for 30 days.
    #[serde(default)]
    pub remember: String,
}

/// The password was right; now a code ("app": the authenticator app,
/// "email": one emailed now, "backup": a backup code), or, with `setup`,
/// setting a second step up first (the server requires one).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondStep {
    /// Goes with every call of this step; works for 10 minutes.
    pub ticket: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub setup: bool,
    /// What setting up may use ("app"; "email" for people who aren't
    /// admins, when the server sends email and they have an address).
    #[serde(default)]
    pub setup_methods: Vec<String>,
}

/// `POST /v1/login/code`: the second step's code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecondCode {
    pub ticket: String,
    pub method: String,
    pub code: String,
    /// Remember this computer for 30 days.
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub device: String,
}

/// `POST /v1/mfa/app/start`: a new authenticator secret to scan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSecret {
    /// Base32, for typing in.
    pub secret: String,
    /// `otpauth://…`, what the QR code holds.
    pub uri: String,
    /// The QR code as SVG.
    pub qr_svg: String,
}

/// The second step set up: one-time backup codes (shown once) and, when
/// it was set up while signing in, the session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecondStepReady {
    pub backup_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
}

/// `GET /v1/me/mfa`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondStepState {
    pub app: bool,
    pub email: bool,
    pub backup_left: u32,
    /// The server requires one of this person.
    pub required: bool,
    pub setup_methods: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub email: String,
}

/// The answer to signing in: keep the token (it's the session).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub account: Account,
    /// After a second step with "remember this computer": send it with the
    /// next `/v1/login` to skip the step (30 days).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remember: Option<String>,
}

/// `POST /v1/setup`: the first admin, once. `token` is the one the server
/// printed (and put in the address it opened).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Setup {
    pub token: String,
    pub server_name: String,
    /// Made from `email` when left out (people sign in with their address).
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub device: String,
}

/// A one-time link the server made: `<server>/join/#<link>`, where `link`
/// is `<id>.<secret>` (also given as a code to type). It welcomes someone
/// an admin added or resets a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Someone an admin added: they pick their password.
    Welcome,
    /// A forgotten password.
    Reset,
}

/// `POST /v1/link`: what a link is, before using it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkCheck {
    /// The code (or an older whole link).
    pub link: String,
    /// Whose code it is (older links name their account themselves).
    #[serde(default)]
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkInfo {
    pub kind: LinkKind,
    pub server_name: String,
    /// Welcome and reset: whose account it is.
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
}

/// `POST /v1/link/use`: pick a password with a link, and be signed in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkUse {
    pub link: String,
    #[serde(default)]
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub device: String,
}

/// `POST /v1/password`: change your password (signed in).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangePassword {
    pub old_password: String,
    pub password: String,
}

// ---- Admin -----------------------------------------------------------

/// A person as admins see them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    /// They can sign in with it instead of their user name ("" if none).
    #[serde(default)]
    pub email: String,
    pub admin: bool,
    pub disabled: bool,
    /// Hasn't picked a password yet (new, or their password was reset).
    pub pending: bool,
    pub groups: Vec<i64>,
    /// When they last used the server (RFC 3339), if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

/// `POST /v1/admin/people`: add someone. With `password` they can sign in
/// with it straight away; without, they get a welcome link.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewPerson {
    /// Made from `email` when left out (people sign in with their address).
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub groups: Vec<i64>,
    /// Server roles they have (Admin is `admin`).
    #[serde(default)]
    pub roles: Vec<i64>,
}

/// `POST /v1/admin/people/{id}`: change someone. Absent fields stay.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersonChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<i64>>,
    /// Their server roles (not Admin, which is `admin`); replaces them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<i64>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub members: Vec<i64>,
}

/// `POST /v1/admin/groups` (new) or `/v1/admin/groups/{id}` (change).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupChange {
    pub name: String,
    #[serde(default)]
    pub members: Vec<i64>,
}

// ---- Spaces, folders and files ---------------------------------------
//
// Names and details of folders and files (`meta`) are JSON text the apps
// write: `{"name": …, "kind": …}` and whatever else they keep. The server
// reads `name` to sort and search.

/// What someone may do. Each includes the ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Open and download.
    Viewer,
    /// And leave comments.
    Commenter,
    /// And change, add, move and delete.
    Editor,
    /// And change who has access.
    Manager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
    /// Made by an admin; access by grants.
    Shared,
    /// Each person's "My files": only they can open it (they can share
    /// single files and folders from it).
    Personal,
    /// The one templates space: everyone may use them, admins change them.
    Templates,
    /// A chat's whiteboards: everyone in the conversation may edit them,
    /// nobody else (admins included). Not listed with the other spaces.
    Chat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub id: i64,
    pub kind: SpaceKind,
    /// Empty for personal and templates spaces, which the apps name
    /// themselves.
    pub name: String,
    /// What you may do in it; None when you can only reach things shared
    /// with you inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
}

/// `POST /v1/spaces` (admins): a new shared space.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewSpace {
    pub name: String,
    /// Who may use a new space; left out: everyone on the server can read
    /// and change it. Whoever makes it always manages it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants: Option<Vec<Grant>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Folder,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: i64,
    pub space: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<i64>,
    pub kind: NodeKind,
    pub meta: String,
    /// Bytes in the latest version.
    pub size: i64,
    /// The latest version's number; 0 before anything is saved.
    pub version: i64,
    /// RFC 3339.
    pub changed: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_by: Option<i64>,
    pub role: Role,
    /// Has its own list of who has access (instead of its folder's).
    pub own_access: bool,
    /// When its picture (for showing it in other documents) was last
    /// put up, RFC 3339; it's renewed while people edit it together, between
    /// versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pictured: Option<String>,
}

/// `POST /v1/nodes`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewNode {
    pub space: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<i64>,
    pub kind: NodeKind,
    pub meta: String,
}

/// `POST /v1/nodes/{id}`: rename (new meta) or move. Absent fields stay.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<MoveTo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MoveTo {
    /// None: the top of the space.
    #[serde(default)]
    pub parent: Option<i64>,
}

/// Who a grant is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    User(i64),
    Group(i64),
    Everyone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub who: Who,
    pub role: Role,
    /// A document role an admin made (Admin › Roles): `role` is then its
    /// base, and Plan allows only the changes the role lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<i64>,
}

/// `GET`/`POST /v1/spaces/{id}/access` and `/v1/nodes/{id}/access`. For a
/// folder or file, `own: false` means it follows its folder (and `grants`
/// is ignored).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    #[serde(default)]
    pub own: bool,
    pub grants: Vec<Grant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub n: i64,
    pub size: i64,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<i64>,
    /// A checkpoint's description (File › Save a checkpoint…); versions
    /// saved as you type have none, and only the newest of those are kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The answer to saving (`POST /v1/nodes/{id}/content?base=<n>`, the
/// bytes as the body). Saving on top of anything but the latest
/// version is refused with 409 and `code: "behind"`, unless `base` is
/// left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub version: i64,
}

// ---- Editing together (WebSocket `/v1/live`) ---------------------------
//
// One JSON object per WebSocket text message, tagged by `"t"`. The first
// thing an app sends is `hello` with its session token (browsers can't put
// it in a header).
//
// The server puts each document's changes in one order and hands them to
// everyone with the document open: `data` is whatever the apps send (JSON
// text).
// An app sends its changes with the number of the last change it has
// (`base`). If someone got in first, the server answers `behind` with the
// changes it missed; the app adjusts its own to come after them and sends
// again. Saved versions record how many changes they include (`upto`), so
// opening a document is: the latest version, then the changes after it.

/// One change to a document, in the server's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub seq: i64,
    pub author: i64,
    /// The sending app's id for the batch it came in.
    pub client: String,
    pub data: String,
}

/// Someone with the document open (one per window).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Here {
    /// This window's connection, for telling people apart.
    pub conn: String,
    pub user: i64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToLive {
    Hello {
        token: String,
    },
    /// Open a document, having changes up to `since` (0: none).
    Join {
        node: i64,
        since: i64,
    },
    Leave {
        node: i64,
    },
    /// My changes, made on top of change number `base`.
    Send {
        node: i64,
        base: i64,
        client: String,
        ops: Vec<String>,
    },
    /// Something for the others only, not kept: where my caret is, what I
    /// have selected.
    Say {
        node: i64,
        data: String,
    },
    /// Ids for new things in the document, so two people never make the
    /// same one: `count` of them, all at least `at_least`.
    Lease {
        node: i64,
        count: i64,
        at_least: i64,
    },
    Ping,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum FromLive {
    Welcome {
        protocol: u32,
        account: Account,
        conn: String,
    },
    /// `version`/`upto`: the latest saved version and the changes it holds;
    /// `ops`: the changes after `since` (or after `upto` if that's later).
    Joined {
        node: i64,
        role: Role,
        seq: i64,
        version: i64,
        upto: i64,
        ops: Vec<Op>,
        people: Vec<Here>,
        /// A plan: the changes this person may make (a document role, or
        /// "can comment"); none for other documents.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        may: Option<Vec<String>>,
    },
    /// Changes from someone else.
    Ops {
        node: i64,
        ops: Vec<Op>,
    },
    /// Your batch `client` became changes `from`..=`to`.
    Accepted {
        node: i64,
        client: String,
        from: i64,
        to: i64,
    },
    /// Your batch `client` came too late: here's what you missed; adjust
    /// and send it again.
    Behind {
        node: i64,
        client: String,
        ops: Vec<Op>,
    },
    Said {
        node: i64,
        from: Here,
        data: String,
    },
    Arrived {
        node: i64,
        who: Here,
    },
    Left {
        node: i64,
        conn: String,
    },
    /// Ids `from`..`to` (not including `to`) are yours.
    Leased {
        node: i64,
        from: i64,
        to: i64,
    },
    Error {
        code: String,
        error: String,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_and_servers_on_nearby_levels_still_work_together() {
        assert_eq!(works_with(VERSION, None), Ok(()));
        // A newer server that still serves this level.
        assert_eq!(works_with(VERSION + 1, Some(VERSION)), Ok(()));
        // A newer server that doesn't, or an older one that never said.
        assert_eq!(works_with(VERSION + 1, Some(VERSION + 1)), Err(Mismatch::UpdateApp));
        assert_eq!(works_with(VERSION + 1, None), Err(Mismatch::UpdateApp));
        assert_eq!(works_with(OLDEST - 1, None), Err(Mismatch::UpdateServer));
    }
}
