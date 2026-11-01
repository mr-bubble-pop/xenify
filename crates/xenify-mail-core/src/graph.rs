//! A Microsoft 365 mailbox through Microsoft Graph, for a company that let
//! its Xenify server use its mailboxes (an admin's consent once; no
//! passwords). It keeps the same local copy as IMAP accounts do and answers
//! the same commands, so the screens can't tell the difference.
//!
//! - Folders: the mailbox's top-level folders (Inbox, Sent Items…).
//! - New mail, changes and deletions: each folder's changes since last time
//!   (`messages/delta`), checked every minute and on request. The first
//!   look covers the last 180 days.
//! - Opening a message downloads it whole (`$value`, the MIME text).
//! - Sending hands Graph the MIME message (`sendMail`), which keeps a copy
//!   in Sent Items itself.

use crate::auth::TokenSource;
use crate::imap::{Folder, Role, Summary};
use crate::store::Store;
use crate::sync::{Cmd, Event, Handle};
use base64::Engine;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// Graph's address (a test server's in tests).
pub const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// How often folders are checked for changes.
const EVERY: Duration = Duration::from_secs(60);
/// How far back the first look goes.
const FIRST_DAYS: i64 = 180;

/// Where to reach Graph and as which mailbox.
#[derive(Clone)]
pub struct Mailbox {
    /// Graph's address, normally [`GRAPH`].
    pub base: String,
    /// The mailbox's address (or Graph user id).
    pub user: String,
    pub tokens: Arc<dyn TokenSource>,
}

#[derive(Debug)]
enum GraphError {
    /// Not allowed (the company's consent was taken back, or the mailbox
    /// isn't covered): stop until the settings change.
    Refused(String),
    Other(String),
}

type R<T> = Result<T, GraphError>;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .into()
}

impl Mailbox {
    /// The address for `path`: under the mailbox, or a whole link Graph
    /// gave (nextLink, deltaLink), which is followed only when it is on
    /// Graph's own address, so the token never goes anywhere else.
    fn url(&self, path: &str) -> R<String> {
        let base = self.base.trim_end_matches('/');
        if path.contains("://") {
            let ours = path
                .strip_prefix(base)
                .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?'));
            if ours {
                Ok(path.to_string())
            } else {
                Err(GraphError::Other("Microsoft 365 sent a link to somewhere else, which wasn't followed.".into()))
            }
        } else {
            Ok(format!("{base}/users/{}{}", enc(&self.user), path))
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<(&str, Vec<u8>)>, prefer: Option<&str>) -> R<Vec<u8>> {
        let url = self.url(path)?;
        let token = self.tokens.token().map_err(GraphError::Refused)?;
        let a = agent();
        let auth = format!("Bearer {token}");
        let resp = match (method, body) {
            ("GET", _) => {
                let mut r = a.get(&url).header("Authorization", &auth);
                if let Some(p) = prefer {
                    r = r.header("Prefer", p);
                }
                r.call()
            }
            ("DELETE", _) => a.delete(&url).header("Authorization", &auth).call(),
            (m, body) => {
                let (ctype, bytes) = body.unwrap_or(("application/json", b"{}".to_vec()));
                let r = if m == "PATCH" { a.patch(&url) } else { a.post(&url) };
                r.header("Authorization", &auth).header("Content-Type", ctype).send(&bytes[..])
            }
        };
        let mut resp = resp.map_err(|e| GraphError::Other(format!("Couldn't reach Microsoft 365: {e}")))?;
        let status = resp.status().as_u16();
        let bytes = resp.body_mut().with_config().limit(60 << 20).read_to_vec().unwrap_or_default();
        match status {
            200..=299 => Ok(bytes),
            401 | 403 => {
                let why = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(String::from))
                    .unwrap_or_default();
                Err(GraphError::Refused(format!(
                    "Microsoft 365 didn't allow the server to use {}'s mailbox{}",
                    self.user,
                    if why.is_empty() { String::new() } else { format!(" ({why})") }
                )))
            }
            _ => {
                let why = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(String::from))
                    .unwrap_or_else(|| format!("status {status}"));
                Err(GraphError::Other(format!("Microsoft 365: {why}")))
            }
        }
    }

    fn json(&self, path: &str, prefer: Option<&str>) -> R<Value> {
        let b = self.request("GET", path, None, prefer)?;
        serde_json::from_slice(&b)
            .map_err(|e| GraphError::Other(format!("Microsoft 365 sent something unexpected: {e}")))
    }

    fn post_json(&self, method: &str, path: &str, body: &Value) -> R<Value> {
        let b = self.request(method, path, Some(("application/json", body.to_string().into_bytes())), None)?;
        Ok(serde_json::from_slice(&b).unwrap_or(Value::Null))
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Sends a whole MIME message from the mailbox (Graph keeps the copy in
/// Sent Items).
pub fn send(m: &Mailbox, mime: &[u8]) -> Result<(), String> {
    let body = base64::engine::general_purpose::STANDARD.encode(mime).into_bytes();
    m.request("POST", "/sendMail", Some(("text/plain", body)), None).map(|_| ()).map_err(|e| match e {
        GraphError::Refused(t) | GraphError::Other(t) => format!("Couldn't send: {t}"),
    })
}

/// Checks the mailbox can be read (the admin's Test button).
pub fn check(m: &Mailbox) -> Result<(), String> {
    m.json("/mailFolders/inbox?$select=id", None).map(|_| ()).map_err(|e| match e {
        GraphError::Refused(t) | GraphError::Other(t) => t,
    })
}

/// Starts keeping `mailbox` in the store at `store_path`.
pub fn start(mailbox: Mailbox, store_path: PathBuf) -> Handle {
    let (tx, rx) = channel();
    let (etx, events) = channel();
    let wake = Arc::new(AtomicBool::new(false));
    let w = wake.clone();
    std::thread::Builder::new()
        .name(format!("mail-graph {}", mailbox.user))
        .spawn(move || {
            let store = match Store::open(&store_path) {
                Ok(s) => s,
                Err(e) => {
                    let _ = etx.send(Event::Problem(format!("Couldn't open the mail store: {e}")));
                    return;
                }
            };
            Worker { m: mailbox, store, rx, events: etx, wake: w }.run();
        })
        .expect("thread");
    Handle::new(tx, events, wake)
}

struct Worker {
    m: Mailbox,
    store: Store,
    rx: Receiver<Cmd>,
    events: Sender<Event>,
    wake: Arc<AtomicBool>,
}

fn db(e: rusqlite::Error) -> GraphError {
    GraphError::Other(format!("mail store: {e}"))
}

/// Graph's well-known folders and what they're for.
const KNOWN: [(&str, Role); 6] = [
    ("inbox", Role::Inbox),
    ("sentitems", Role::Sent),
    ("drafts", Role::Drafts),
    ("deleteditems", Role::Trash),
    ("junkemail", Role::Junk),
    ("archive", Role::Archive),
];

impl Worker {
    fn tell(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn run(mut self) {
        let mut backoff = Duration::from_secs(5);
        let mut ready = false;
        loop {
            let r = if ready { self.sync_all() } else { self.folders().and_then(|_| self.sync_all()) };
            match r {
                Ok(()) => {
                    ready = true;
                    backoff = Duration::from_secs(5);
                    self.tell(Event::Status(String::new()));
                    // Commands now; changes every minute.
                    let until = std::time::Instant::now() + EVERY;
                    loop {
                        let left = until.saturating_duration_since(std::time::Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        match self.rx.recv_timeout(left.min(Duration::from_millis(500))) {
                            Ok(Cmd::Stop) => return,
                            Ok(Cmd::SyncNow) => break,
                            Ok(cmd) => {
                                if let Err(e) = self.command(cmd) {
                                    self.tell(Event::Problem(match e {
                                        GraphError::Refused(t) | GraphError::Other(t) => t,
                                    }));
                                }
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(_) => return,
                        }
                    }
                    self.wake.store(false, Ordering::Relaxed);
                }
                Err(GraphError::Refused(t)) => {
                    self.tell(Event::SignInFailed(t));
                    while let Ok(c) = self.rx.recv() {
                        if c == Cmd::Stop {
                            return;
                        }
                    }
                    return;
                }
                Err(GraphError::Other(t)) => {
                    self.tell(Event::Problem(t));
                    self.tell(Event::Status(String::new()));
                    let until = std::time::Instant::now() + backoff;
                    while std::time::Instant::now() < until {
                        match self.rx.recv_timeout(Duration::from_millis(250)) {
                            Ok(Cmd::Stop) => return,
                            Ok(Cmd::SyncNow) => break,
                            _ => {}
                        }
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(300));
                }
            }
        }
    }

    /// The top-level folders, with what the well-known ones are for.
    fn folders(&mut self) -> R<()> {
        self.tell(Event::Status("Connecting to Microsoft 365…".into()));
        let mut roles = Vec::new();
        for (name, role) in KNOWN {
            if let Ok(v) = self.m.json(&format!("/mailFolders/{name}?$select=id"), None)
                && let Some(id) = v["id"].as_str()
            {
                roles.push((id.to_string(), role));
            }
        }
        let mut list = Vec::new();
        let mut next = Some("/mailFolders?$top=100&$select=id,displayName".to_string());
        while let Some(path) = next.take() {
            let v = self.m.json(&path, None)?;
            for f in v["value"].as_array().into_iter().flatten() {
                let (Some(id), Some(name)) = (f["id"].as_str(), f["displayName"].as_str()) else { continue };
                let role = roles.iter().find(|(r, _)| r == id).map(|(_, role)| *role);
                list.push(Folder { wire: id.into(), name: name.into(), delimiter: None, role, no_select: false });
            }
            next = v["@odata.nextLink"].as_str().map(String::from);
        }
        if !list.iter().any(|f| f.role == Some(Role::Inbox)) {
            return Err(GraphError::Other("Microsoft 365 didn't list an Inbox for this mailbox.".into()));
        }
        // The Inbox first, then the others in Outlook's order.
        list.sort_by_key(|f| (f.role != Some(Role::Inbox)) as u8);
        self.store.set_folders(&list).map_err(db)?;
        self.tell(Event::Changed);
        Ok(())
    }

    fn sync_all(&mut self) -> R<()> {
        let folders = self.store.folders().map_err(db)?;
        let mut order: Vec<_> = folders.into_iter().collect();
        order.sort_by_key(|f| match f.role {
            Some(Role::Inbox) => 0,
            Some(Role::Sent) => 1,
            Some(Role::Drafts) => 2,
            _ => 5,
        });
        for f in order {
            if let Ok(c) = self.rx.try_recv() {
                // A command waits at most one folder.
                if c == Cmd::Stop {
                    return Ok(());
                }
                if let Err(GraphError::Refused(t)) = self.command(c) {
                    return Err(GraphError::Refused(t));
                }
            }
            self.tell(Event::Status(format!("Checking {}…", f.name)));
            self.sync_folder(&f.wire)?;
            self.tell(Event::Changed);
        }
        Ok(())
    }

    /// One folder's changes since last time (all of the last 180 days the
    /// first time).
    fn sync_folder(&mut self, wire: &str) -> R<()> {
        const SELECT: &str = "subject,from,toRecipients,receivedDateTime,isRead,flag,internetMessageId,bodyPreview";
        let mut next = self.store.graph_delta(wire).map_err(db)?;
        if next.is_none() {
            let since = days_ago(FIRST_DAYS);
            next = Some(format!(
                "/mailFolders/{}/messages/delta?$select={SELECT}&$filter=receivedDateTime+ge+{since}",
                enc(wire)
            ));
        }
        while let Some(path) = next.take() {
            let v = match self.m.json(&path, Some("odata.maxpagesize=200")) {
                Ok(v) => v,
                // An old delta link Graph forgot (410/400): start this folder again.
                Err(GraphError::Other(t)) if t.contains("sync") || t.contains("410") || t.contains("delta") => {
                    self.store.set_graph_delta(wire, None).map_err(db)?;
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            let mut add = Vec::new();
            for m in v["value"].as_array().into_iter().flatten() {
                let Some(gid) = m["id"].as_str() else { continue };
                if m.get("@removed").is_some() {
                    if let Some(uid) = self.store.graph_forget(wire, gid).map_err(db)? {
                        self.store.remove(wire, &[uid]).map_err(db)?;
                    }
                    continue;
                }
                let uid = self.store.graph_uid(wire, gid).map_err(db)?;
                add.push(summary(uid, m));
            }
            if !add.is_empty() {
                self.store.add_summaries(wire, &add).map_err(db)?;
            }
            next = v["@odata.nextLink"].as_str().map(String::from);
            if next.is_none()
                && let Some(link) = v["@odata.deltaLink"].as_str()
                && self.m.url(link).is_ok()
            {
                self.store.set_graph_delta(wire, Some(link)).map_err(db)?;
            }
        }
        Ok(())
    }

    fn gid(&self, wire: &str, uid: u32) -> R<String> {
        self.store.graph_gid(wire, uid).map_err(db)?.ok_or_else(|| GraphError::Other("That email is gone.".into()))
    }

    fn command(&mut self, cmd: Cmd) -> R<()> {
        match cmd {
            Cmd::Stop | Cmd::SyncNow => {}
            Cmd::Download { folder, uid } => {
                let gid = self.gid(&folder, uid)?;
                let raw = self.m.request("GET", &format!("/messages/{}/$value", enc(&gid)), None, None)?;
                self.store.set_raw(&folder, uid, &raw).map_err(db)?;
                self.tell(Event::Downloaded { folder, uid });
            }
            Cmd::SetFlag { folder, uid, flag, on } => {
                let gid = self.gid(&folder, uid)?;
                let body = if flag.eq_ignore_ascii_case("\\Seen") {
                    json!({ "isRead": on })
                } else if flag.eq_ignore_ascii_case("\\Flagged") {
                    json!({ "flag": { "flagStatus": if on { "flagged" } else { "notFlagged" } } })
                } else {
                    return Ok(());
                };
                self.m.post_json("PATCH", &format!("/messages/{}", enc(&gid)), &body)?;
                self.store.toggle_flag(&folder, uid, &flag, on).map_err(db)?;
                self.tell(Event::Changed);
            }
            Cmd::Move { folder, uid, to } => {
                let gid = self.gid(&folder, uid)?;
                self.m.post_json("POST", &format!("/messages/{}/move", enc(&gid)), &json!({ "destinationId": to }))?;
                self.store.graph_forget(&folder, &gid).map_err(db)?;
                self.store.remove(&folder, &[uid]).map_err(db)?;
                self.sync_folder(&to)?;
                self.tell(Event::Changed);
            }
            Cmd::Delete { folder, uid } => {
                let gid = self.gid(&folder, uid)?;
                let trash = self.store.folder_with(Role::Trash).map_err(db)?;
                match trash.filter(|t| *t != folder) {
                    Some(t) => {
                        self.m.post_json(
                            "POST",
                            &format!("/messages/{}/move", enc(&gid)),
                            &json!({ "destinationId": t }),
                        )?;
                        self.store.graph_forget(&folder, &gid).map_err(db)?;
                        self.store.remove(&folder, &[uid]).map_err(db)?;
                        self.sync_folder(&t)?;
                    }
                    None => {
                        self.m.request("DELETE", &format!("/messages/{}", enc(&gid)), None, None)?;
                        self.store.graph_forget(&folder, &gid).map_err(db)?;
                        self.store.remove(&folder, &[uid]).map_err(db)?;
                    }
                }
                self.tell(Event::Changed);
            }
        }
        Ok(())
    }
}

/// A list entry made from Graph's fields (headers written as MIME would
/// have them, so the store reads them as it reads IMAP's).
fn summary(uid: u32, m: &Value) -> Summary {
    let word = |s: &str| {
        if s.is_ascii() && !s.contains(['\r', '\n']) {
            s.to_string()
        } else {
            format!("=?utf-8?b?{}?=", base64::engine::general_purpose::STANDARD.encode(s))
        }
    };
    let person = |p: &Value| {
        let a = p["emailAddress"]["address"].as_str().unwrap_or("");
        let n = p["emailAddress"]["name"].as_str().unwrap_or("");
        if n.is_empty() || n == a { format!("<{a}>") } else { format!("\"{}\" <{a}>", word(&n.replace('"', ""))) }
    };
    let mut h = String::new();
    if m["from"].is_object() {
        h.push_str(&format!("From: {}\r\n", person(&m["from"])));
    }
    let to: Vec<String> = m["toRecipients"].as_array().into_iter().flatten().map(person).collect();
    if !to.is_empty() {
        h.push_str(&format!("To: {}\r\n", to.join(", ")));
    }
    h.push_str(&format!("Subject: {}\r\n", word(m["subject"].as_str().unwrap_or(""))));
    let date = m["receivedDateTime"].as_str().map(rfc2822).unwrap_or_default();
    if !date.is_empty() {
        h.push_str(&format!("Date: {date}\r\n"));
    }
    if let Some(id) = m["internetMessageId"].as_str() {
        h.push_str(&format!("Message-ID: {id}\r\n"));
    }
    h.push_str("\r\n");
    let mut flags = Vec::new();
    if m["isRead"].as_bool() == Some(true) {
        flags.push("\\Seen".to_string());
    }
    if m["flag"]["flagStatus"].as_str() == Some("flagged") {
        flags.push("\\Flagged".to_string());
    }
    Summary { uid, flags, size: 0, internal_date: String::new(), header: h.into_bytes() }
}

/// "2026-10-15T09:41:00Z" → "15 Oct 2026 09:41:00 +0000".
fn rfc2822(iso: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (Some(y), Some(mo), Some(d), Some(t)) = (iso.get(0..4), iso.get(5..7), iso.get(8..10), iso.get(11..19)) else {
        return String::new();
    };
    let Some(month) = mo.parse::<usize>().ok().and_then(|m| MONTHS.get(m.wrapping_sub(1))) else {
        return String::new();
    };
    format!("{} {month} {y} {t} +0000", d.trim_start_matches('0'))
}

/// The date `days` ago, as Graph filters take it ("2026-05-04T00:00:00Z").
fn days_ago(days: i64) -> String {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let day = now / 86400 - days;
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}T00:00:00Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoTokens;
    impl TokenSource for NoTokens {
        fn token(&self) -> Result<String, String> {
            Err("no".into())
        }
    }

    #[test]
    fn links_elsewhere_are_not_followed() {
        let m = Mailbox { base: GRAPH.into(), user: "a b@x.org".into(), tokens: Arc::new(NoTokens) };
        assert_eq!(m.url("/mailFolders").unwrap(), format!("{GRAPH}/users/a%20b@x.org/mailFolders"));
        let next = format!("{GRAPH}/users/a@x.org/messages/delta?$skiptoken=1");
        assert_eq!(m.url(&next).unwrap(), next);
        for bad in [
            "https://evil.example/v1.0/users/a/messages",
            "https://graph.microsoft.com/v1.0.evil.example/x",
            "https://graph.microsoft.com/v1.0@evil.example/x",
            "http://graph.microsoft.com/v1.0/users/a",
            "https://graph.microsoft.com/beta/users/a",
        ] {
            assert!(m.url(bad).is_err(), "{bad}");
        }
        // Refused before any token is asked for.
        assert!(matches!(m.request("GET", "https://evil.example/x", None, None), Err(GraphError::Other(_))));
    }

    #[test]
    fn graph_messages_read_as_list_entries() {
        let m = json!({
            "id": "AAA", "subject": "Café plans", "isRead": true, "flag": {"flagStatus": "flagged"},
            "from": {"emailAddress": {"name": "Sam Lee", "address": "sam@example.com"}},
            "toRecipients": [{"emailAddress": {"name": "Ana", "address": "ana@example.com"}}],
            "receivedDateTime": "2026-10-05T09:41:00Z", "internetMessageId": "<x@y>"
        });
        let s = summary(7, &m);
        let mut store = Store::in_memory().unwrap();
        store
            .set_folders(&[Folder {
                wire: "in".into(),
                name: "Inbox".into(),
                delimiter: None,
                role: Some(Role::Inbox),
                no_select: false,
            }])
            .unwrap();
        store.add_summaries("in", &[s]).unwrap();
        let r = store.row("in", 7).unwrap().unwrap();
        assert_eq!(
            (r.subject.as_str(), r.from.as_str(), r.from_address.as_str()),
            ("Café plans", "Sam Lee", "sam@example.com")
        );
        assert!(r.is_read() && r.is_flagged());
        assert_eq!(r.date, 1_791_193_260);
        assert_eq!(rfc2822("2026-01-02T03:04:05Z"), "2 Jan 2026 03:04:05 +0000");
        assert_eq!(days_ago(0).len(), 20);
    }

    #[test]
    fn graph_ids_get_small_numbers() {
        let mut s = Store::in_memory().unwrap();
        assert_eq!(s.graph_uid("f", "long-id-1").unwrap(), 1);
        assert_eq!(s.graph_uid("f", "long-id-2").unwrap(), 2);
        assert_eq!(s.graph_uid("f", "long-id-1").unwrap(), 1);
        assert_eq!(s.graph_gid("f", 2).unwrap().as_deref(), Some("long-id-2"));
        assert_eq!(s.graph_forget("f", "long-id-1").unwrap(), Some(1));
        assert_eq!(s.graph_gid("f", 1).unwrap(), None);
    }
}
