//! A small, blocking IMAP client (RFC 3501 plus MOVE, UIDPLUS, IDLE and
//! CONDSTORE where the server has them). Responses are parsed by
//! `imap-proto`; TLS is rustls with the Mozilla root certificates. Run it on
//! a background thread.

pub mod utf7;

use imap_proto::{AttributeValue, MailboxDatum, NameAttribute, Response, ResponseCode, Status};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How the connection is protected. A value this version doesn't know (from
/// a newer app or server) reads as `Tls`, the safest choice, instead of
/// making the whole accounts list unreadable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub enum Security {
    /// TLS from the start (port 993).
    #[default]
    Tls,
    /// Plain first, then STARTTLS (port 143).
    StartTls,
    /// No encryption: only for a server on this computer (tests).
    None,
}

impl<'de> serde::Deserialize<'de> for Security {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "StartTls" => Security::StartTls,
            "None" => Security::None,
            _ => Security::Tls,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    #[error("couldn't connect to {0}")]
    Connect(String),
    #[error("secure connection failed: {0}")]
    Tls(String),
    #[error("connection problem: {0}")]
    Io(String),
    #[error("the server didn't accept the user name or password: {0}")]
    Auth(String),
    #[error("the server refused: {0}")]
    No(String),
    #[error("the server didn't understand a request: {0}")]
    Bad(String),
    #[error("the server closed the connection: {0}")]
    Bye(String),
    #[error("unexpected reply from the server: {0}")]
    Protocol(String),
}

impl From<std::io::Error> for ImapError {
    fn from(e: std::io::Error) -> Self {
        ImapError::Io(e.to_string())
    }
}

/// A folder (IMAP mailbox).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// The name as the server knows it (modified UTF-7).
    pub wire: String,
    /// The name to show ("Sent", "Entwürfe").
    pub name: String,
    pub delimiter: Option<String>,
    /// What it's for, when the server says (RFC 6154) or the name tells.
    pub role: Option<Role>,
    /// Holds only other folders (can't be opened).
    pub no_select: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    All,
    Flagged,
}

/// What SELECT says about a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selected {
    pub exists: u32,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub highest_modseq: Option<u64>,
}

/// A message's list entry: flags, size and the headers the list needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub uid: u32,
    pub flags: Vec<String>,
    pub size: u32,
    pub internal_date: String,
    /// The raw header lines (From, To, Cc, Subject, Date, Message-ID…).
    pub header: Vec<u8>,
}

/// The connection: plain (tests, or before STARTTLS) or TLS.
enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Conn {
    fn set_timeout(&mut self, t: Option<Duration>) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.set_read_timeout(t),
            Conn::Tls(s) => s.sock.set_read_timeout(t),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

pub struct Client {
    stream: Conn,
    buf: Vec<u8>,
    tag: u32,
    pub capabilities: Vec<String>,
    /// Untagged news that came with other replies (new mail, expunges).
    pub changed: bool,
}

/// Normal wait for a reply.
const TIMEOUT: Duration = Duration::from_secs(60);
/// Longest single reply we accept (a huge message is fetched as one literal).
const MAX_REPLY: usize = 200 * 1024 * 1024;

fn tls_config() -> Arc<rustls::ClientConfig> {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring supports the default versions")
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn wrap_tls(host: &str, tcp: TcpStream) -> Result<Conn, ImapError> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| ImapError::Tls(e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config(), name).map_err(|e| ImapError::Tls(e.to_string()))?;
    Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

impl Client {
    pub fn connect(server: &Server) -> Result<Client, ImapError> {
        Client::connect_pinned(server, None)
    }

    /// [`Client::connect`], to `pin` when given: an address of the server
    /// the caller has already checked (so a name that resolves differently
    /// a moment later can't send it elsewhere). The secure connection is
    /// still checked against `server.host`'s certificate.
    pub fn connect_pinned(server: &Server, pin: Option<SocketAddr>) -> Result<Client, ImapError> {
        // A password must never cross the network in the clear.
        if server.security == Security::None && !matches!(server.host.as_str(), "127.0.0.1" | "localhost" | "::1") {
            return Err(ImapError::Connect(
                "An unencrypted connection is only allowed to a server on this computer.".into(),
            ));
        }
        let addrs: Vec<_> = match pin {
            Some(a) => vec![a],
            None => (server.host.as_str(), server.port)
                .to_socket_addrs()
                .map_err(|e| ImapError::Connect(format!("{}: {e}", server.host)))?
                .collect(),
        };
        let tcp = addrs
            .iter()
            .find_map(|a| TcpStream::connect_timeout(a, Duration::from_secs(20)).ok())
            .ok_or_else(|| ImapError::Connect(format!("{}:{}", server.host, server.port)))?;
        tcp.set_read_timeout(Some(TIMEOUT))?;
        tcp.set_write_timeout(Some(TIMEOUT))?;
        let stream = match server.security {
            Security::Tls => wrap_tls(&server.host, tcp)?,
            Security::StartTls | Security::None => Conn::Plain(tcp),
        };
        let mut c = Client { stream, buf: Vec::new(), tag: 0, capabilities: Vec::new(), changed: false };
        // The greeting.
        match c.read_response()? {
            Response::Data { status: Status::Ok | Status::PreAuth, outcome } => {
                if let Some(ResponseCode::Capabilities(caps)) = outcome.code {
                    c.capabilities = caps.iter().map(cap_name).collect();
                }
            }
            Response::Data { status: Status::Bye, outcome } => {
                return Err(ImapError::Bye(outcome.information.unwrap_or_default().to_string()));
            }
            other => return Err(ImapError::Protocol(format!("{other:?}"))),
        }
        if server.security == Security::StartTls {
            if !c.has("STARTTLS") {
                return Err(ImapError::Tls("the server doesn't offer a secure connection on this port".into()));
            }
            c.run("STARTTLS")?;
            let Conn::Plain(tcp) = c.stream else { unreachable!("plain until STARTTLS") };
            // Capabilities are asked again over the secure connection.
            c = Client {
                stream: wrap_tls(&server.host, tcp)?,
                buf: Vec::new(),
                tag: c.tag,
                capabilities: Vec::new(),
                changed: false,
            };
        }
        if c.capabilities.is_empty() {
            c.refresh_capabilities()?;
        }
        Ok(c)
    }

    pub fn has(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c.eq_ignore_ascii_case(cap))
    }

    fn refresh_capabilities(&mut self) -> Result<(), ImapError> {
        let replies = self.run("CAPABILITY")?;
        for r in replies {
            if let Response::Capabilities(caps) = r {
                self.capabilities = caps.iter().map(cap_name).collect();
            }
        }
        Ok(())
    }

    /// Signs in (AUTHENTICATE PLAIN where offered, else LOGIN).
    pub fn login(&mut self, user: &str, password: &str) -> Result<(), ImapError> {
        let result = if self.has("AUTH=PLAIN") {
            use base64::Engine;
            let token = base64::engine::general_purpose::STANDARD.encode(format!("\0{user}\0{password}"));
            if self.has("SASL-IR") {
                self.run(&format!("AUTHENTICATE PLAIN {token}"))
            } else {
                self.run_continued("AUTHENTICATE PLAIN", token.as_bytes())
            }
        } else if self.has("LOGINDISABLED") {
            return Err(ImapError::Auth("the server only allows sign-in over a secure connection".into()));
        } else {
            let cmd = format!("LOGIN {} {}", astring(user), astring(password));
            self.run(&cmd)
        };
        match result {
            Ok(_) => {
                // Servers often announce more after sign-in.
                self.refresh_capabilities()
            }
            Err(ImapError::No(m) | ImapError::Bad(m)) => Err(ImapError::Auth(m)),
            Err(e) => Err(e),
        }
    }

    /// Signs in with a password or a provider's token (XOAUTH2).
    pub fn sign_in(&mut self, user: &str, secret: &crate::auth::Secret) -> Result<(), ImapError> {
        match secret {
            crate::auth::Secret::Password(p) => self.login(user, p),
            crate::auth::Secret::Token(t) => {
                let token = t.token().map_err(ImapError::Auth)?;
                self.login_xoauth2(user, &token)
            }
        }
    }

    /// AUTHENTICATE XOAUTH2. A refused token comes back as a `+` with the
    /// reason; an empty line ends it and the NO follows.
    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<(), ImapError> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(crate::auth::xoauth2(user, token));
        let tag = self.next_tag();
        if self.has("SASL-IR") {
            self.send(&format!("{tag} AUTHENTICATE XOAUTH2 {b64}\r\n"))?;
        } else {
            self.send(&format!("{tag} AUTHENTICATE XOAUTH2\r\n"))?;
            self.await_continue(&tag)?;
            self.send(&format!("{b64}\r\n"))?;
        }
        loop {
            match self.read_response()? {
                Response::Continue(_) => self.send("\r\n")?,
                Response::Done { tag: t, status, outcome } if t.0 == tag => {
                    if status == Status::Ok {
                        return self.refresh_capabilities();
                    }
                    let why =
                        outcome.information.as_deref().unwrap_or("the mail server refused the sign-in").to_string();
                    return Err(ImapError::Auth(why));
                }
                other => self.note(&other),
            }
        }
    }

    pub fn list(&mut self) -> Result<Vec<Folder>, ImapError> {
        let cmd = if self.has("SPECIAL-USE") { "LIST \"\" \"*\" RETURN (SPECIAL-USE)" } else { "LIST \"\" \"*\"" };
        let mut out = Vec::new();
        for r in self.run(cmd)? {
            if let Response::MailboxData(MailboxDatum::List(l)) = r {
                let wire = l.name.to_string();
                let name = if wire.eq_ignore_ascii_case("INBOX") { "Inbox".to_string() } else { utf7::decode(&wire) };
                let mut role = l.name_attributes.iter().find_map(|a| match a {
                    NameAttribute::Drafts => Some(Role::Drafts),
                    NameAttribute::Sent => Some(Role::Sent),
                    NameAttribute::Archive => Some(Role::Archive),
                    NameAttribute::Junk => Some(Role::Junk),
                    NameAttribute::Trash => Some(Role::Trash),
                    NameAttribute::All => Some(Role::All),
                    NameAttribute::Flagged => Some(Role::Flagged),
                    _ => None,
                });
                if wire.eq_ignore_ascii_case("INBOX") {
                    role = Some(Role::Inbox);
                }
                let leaf =
                    name.rsplit(l.delimiter.as_deref().unwrap_or("/")).next().unwrap_or(&name).to_ascii_lowercase();
                role = role.or(match leaf.as_str() {
                    "sent" | "sent items" | "sent messages" | "sent mail" => Some(Role::Sent),
                    "drafts" | "draft" => Some(Role::Drafts),
                    "trash" | "deleted items" | "deleted messages" | "bin" => Some(Role::Trash),
                    "junk" | "spam" | "junk e-mail" | "bulk mail" => Some(Role::Junk),
                    "archive" | "archives" => Some(Role::Archive),
                    _ => None,
                });
                let no_select = l.name_attributes.iter().any(|a| matches!(a, NameAttribute::NoSelect));
                out.push(Folder { wire, name, delimiter: l.delimiter.map(|d| d.to_string()), role, no_select });
            }
        }
        Ok(out)
    }

    /// Opens a folder (read-only with `examine`).
    pub fn select(&mut self, wire: &str, examine: bool) -> Result<Selected, ImapError> {
        let verb = if examine { "EXAMINE" } else { "SELECT" };
        let condstore = if self.has("CONDSTORE") { " (CONDSTORE)" } else { "" };
        let mut s = Selected::default();
        for r in self.run(&format!("{verb} {}{condstore}", quote(wire)))? {
            match r {
                Response::MailboxData(MailboxDatum::Exists(n)) => s.exists = n,
                Response::Data { outcome, .. } => match outcome.code {
                    Some(ResponseCode::UidValidity(v)) => s.uid_validity = v,
                    Some(ResponseCode::UidNext(v)) => s.uid_next = v,
                    Some(ResponseCode::HighestModSeq(v)) => s.highest_modseq = Some(v),
                    _ => {}
                },
                _ => {}
            }
        }
        self.changed = false;
        Ok(s)
    }

    /// UIDs matching `criteria` (`ALL`, `UID 120:*`, `UNSEEN`…).
    pub fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>, ImapError> {
        let mut out = Vec::new();
        for r in self.run(&format!("UID SEARCH {criteria}"))? {
            if let Response::MailboxData(MailboxDatum::Search(v)) = r {
                out.extend(v);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// List entries for the messages in `uids` (a UID set like `1:40,52`).
    pub fn fetch_summaries(&mut self, uids: &str) -> Result<Vec<Summary>, ImapError> {
        let cmd = format!(
            "UID FETCH {uids} (UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES CONTENT-TYPE)])"
        );
        let mut out = Vec::new();
        for r in self.run(&cmd)? {
            if let Response::Fetch(_, attrs) = r {
                let mut s = Summary::default();
                for a in attrs {
                    match a {
                        AttributeValue::Uid(u) => s.uid = u,
                        AttributeValue::Flags(f) => s.flags = f.iter().map(|x| x.to_string()).collect(),
                        AttributeValue::Rfc822Size(n) => s.size = n,
                        AttributeValue::InternalDate(d) => s.internal_date = d.to_string(),
                        AttributeValue::BodySection { data: Some(d), .. } => s.header = d.to_vec(),
                        _ => {}
                    }
                }
                if s.uid != 0 {
                    out.push(s);
                }
            }
        }
        Ok(out)
    }

    /// Flags for the messages in `uids` (to see what changed elsewhere).
    pub fn fetch_flags(&mut self, uids: &str) -> Result<Vec<(u32, Vec<String>)>, ImapError> {
        let mut out = Vec::new();
        for r in self.run(&format!("UID FETCH {uids} (UID FLAGS)"))? {
            if let Response::Fetch(_, attrs) = r {
                let mut uid = 0;
                let mut flags = Vec::new();
                for a in attrs {
                    match a {
                        AttributeValue::Uid(u) => uid = u,
                        AttributeValue::Flags(f) => flags = f.iter().map(|x| x.to_string()).collect(),
                        _ => {}
                    }
                }
                if uid != 0 {
                    out.push((uid, flags));
                }
            }
        }
        Ok(out)
    }

    /// The whole message (without marking it read).
    pub fn fetch_message(&mut self, uid: u32) -> Result<Option<Vec<u8>>, ImapError> {
        for r in self.run(&format!("UID FETCH {uid} (UID BODY.PEEK[])"))? {
            if let Response::Fetch(_, attrs) = r {
                let mut found = None;
                let mut this_uid = None;
                for a in attrs {
                    match a {
                        AttributeValue::Uid(u) => this_uid = Some(u),
                        AttributeValue::BodySection { data: Some(d), .. } => found = Some(d.to_vec()),
                        AttributeValue::Rfc822(Some(d)) => found = Some(d.to_vec()),
                        _ => {}
                    }
                }
                if this_uid.is_none_or(|u| u == uid) && found.is_some() {
                    return Ok(found);
                }
            }
        }
        Ok(None)
    }

    /// Adds (or removes) flags such as `\Seen` or `\Flagged`.
    pub fn store_flags(&mut self, uids: &str, add: bool, flags: &[&str]) -> Result<(), ImapError> {
        let sign = if add { '+' } else { '-' };
        self.run(&format!("UID STORE {uids} {sign}FLAGS.SILENT ({})", flags.join(" "))).map(|_| ())
    }

    /// Moves messages to another folder (MOVE where offered, else copy,
    /// mark deleted and expunge).
    pub fn move_to(&mut self, uids: &str, target_wire: &str) -> Result<(), ImapError> {
        if self.has("MOVE") {
            self.run(&format!("UID MOVE {uids} {}", quote(target_wire)))?;
        } else {
            self.run(&format!("UID COPY {uids} {}", quote(target_wire)))?;
            self.store_flags(uids, true, &["\\Deleted"])?;
            if self.has("UIDPLUS") {
                self.run(&format!("UID EXPUNGE {uids}"))?;
            } else {
                self.run("EXPUNGE")?;
            }
        }
        Ok(())
    }

    /// Adds a message to a folder (a sent copy, a draft).
    pub fn append(&mut self, target_wire: &str, flags: &[&str], message: &[u8]) -> Result<(), ImapError> {
        let cmd = format!("APPEND {} ({})", quote(target_wire), flags.join(" "));
        self.run_literal(&cmd, message).map(|_| ())
    }

    /// Removes messages marked deleted: just these (UIDPLUS).
    pub fn uid_expunge(&mut self, uids: &str) -> Result<(), ImapError> {
        self.run(&format!("UID EXPUNGE {uids}")).map(|_| ())
    }

    /// Removes every message marked deleted in the folder.
    pub fn expunge(&mut self) -> Result<(), ImapError> {
        self.run("EXPUNGE").map(|_| ())
    }

    pub fn noop(&mut self) -> Result<(), ImapError> {
        self.run("NOOP").map(|_| ())
    }

    /// Waits for news in the selected folder (IDLE, RFC 2177) for up to
    /// `max`, or until `stop`. True if something changed.
    pub fn idle(&mut self, max: Duration, stop: &AtomicBool) -> Result<bool, ImapError> {
        if !self.has("IDLE") {
            // Poll instead.
            let until = Instant::now() + max;
            while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
            }
            self.noop()?;
            return Ok(std::mem::take(&mut self.changed));
        }
        let tag = self.next_tag();
        self.send(&format!("{tag} IDLE\r\n"))?;
        loop {
            match self.read_response()? {
                Response::Continue(_) => break,
                Response::Done { status, outcome, .. } => {
                    return Err(fail(status, outcome.information.as_deref().unwrap_or("IDLE")));
                }
                other => self.note(&other),
            }
        }
        let until = Instant::now() + max;
        self.stream.set_timeout(Some(Duration::from_millis(500)))?;
        let mut changed = false;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) && !changed {
            match self.try_read_response() {
                Ok(Some(r)) => {
                    self.note(&r);
                    changed = self.changed;
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = self.stream.set_timeout(Some(TIMEOUT));
                    return Err(e);
                }
            }
        }
        self.stream.set_timeout(Some(TIMEOUT))?;
        self.send("DONE\r\n")?;
        loop {
            match self.read_response()? {
                Response::Done { tag: t, status, outcome } if t.0 == tag => {
                    if status != Status::Ok {
                        return Err(fail(status, outcome.information.as_deref().unwrap_or("IDLE")));
                    }
                    break;
                }
                other => self.note(&other),
            }
        }
        Ok(std::mem::take(&mut self.changed))
    }

    pub fn logout(mut self) {
        let _ = self.run("LOGOUT");
    }

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("A{:04}", self.tag)
    }

    fn send(&mut self, s: &str) -> Result<(), ImapError> {
        self.stream.write_all(s.as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }

    /// Untagged replies that mean the folder changed.
    fn note(&mut self, r: &Response) {
        if matches!(
            r,
            Response::MailboxData(MailboxDatum::Exists(_) | MailboxDatum::Recent(_))
                | Response::Expunge(_)
                | Response::Vanished { .. }
                | Response::Fetch(..)
        ) {
            self.changed = true;
        }
    }

    /// Sends `cmd` and gathers the replies until its tagged result.
    fn run(&mut self, cmd: &str) -> Result<Vec<Response<'static>>, ImapError> {
        let tag = self.next_tag();
        self.send(&format!("{tag} {cmd}\r\n"))?;
        self.collect(&tag)
    }

    /// A command whose last argument is sent after the server's go-ahead
    /// (`+`): a literal `{n}` (APPEND) or a SASL response.
    fn run_literal(&mut self, cmd: &str, data: &[u8]) -> Result<Vec<Response<'static>>, ImapError> {
        let tag = self.next_tag();
        self.send(&format!("{tag} {cmd} {{{}}}\r\n", data.len()))?;
        self.await_continue(&tag)?;
        self.stream.write_all(data)?;
        self.send("\r\n")?;
        self.collect(&tag)
    }

    fn run_continued(&mut self, cmd: &str, data: &[u8]) -> Result<Vec<Response<'static>>, ImapError> {
        let tag = self.next_tag();
        self.send(&format!("{tag} {cmd}\r\n"))?;
        self.await_continue(&tag)?;
        self.stream.write_all(data)?;
        self.send("\r\n")?;
        self.collect(&tag)
    }

    fn await_continue(&mut self, tag: &str) -> Result<(), ImapError> {
        loop {
            match self.read_response()? {
                Response::Continue(_) => return Ok(()),
                Response::Done { tag: t, status, outcome } if t.0 == tag => {
                    return Err(fail(status, outcome.information.as_deref().unwrap_or("")));
                }
                other => self.note(&other),
            }
        }
    }

    fn collect(&mut self, tag: &str) -> Result<Vec<Response<'static>>, ImapError> {
        let mut out = Vec::new();
        loop {
            let r = self.read_response()?;
            match r {
                Response::Done { tag: t, status, outcome } if t.0 == tag => {
                    if status == Status::Ok {
                        if let Some(ResponseCode::Capabilities(caps)) = &outcome.code {
                            self.capabilities = caps.iter().map(cap_name).collect();
                        }
                        out.push(Response::Done { tag: t, status, outcome });
                        return Ok(out);
                    }
                    return Err(fail(status, outcome.information.as_deref().unwrap_or("")));
                }
                Response::Data { status: Status::Bye, outcome } => {
                    return Err(ImapError::Bye(outcome.information.unwrap_or_default().to_string()));
                }
                other => {
                    self.note(&other);
                    out.push(other);
                }
            }
        }
    }

    fn read_response(&mut self) -> Result<Response<'static>, ImapError> {
        loop {
            if let Some(r) = self.parse_buffered()? {
                return Ok(r);
            }
            self.fill()?;
        }
    }

    /// Like `read_response`, but None when nothing arrives before the read
    /// timeout (used while idling).
    fn try_read_response(&mut self) -> Result<Option<Response<'static>>, ImapError> {
        if let Some(r) = self.parse_buffered()? {
            return Ok(Some(r));
        }
        match self.fill() {
            Ok(()) => self.parse_buffered(),
            Err(ImapError::Io(e))
                if e.contains("timed out")
                    || e.contains("would block")
                    || e.contains("Resource temporarily unavailable") =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    fn parse_buffered(&mut self) -> Result<Option<Response<'static>>, ImapError> {
        loop {
            if self.buf.is_empty() {
                return Ok(None);
            }
            match Response::parse(&self.buf) {
                Ok((rest, r)) => {
                    let used = self.buf.len() - rest.len();
                    let r = r.into_owned();
                    self.buf.drain(..used);
                    return Ok(Some(r));
                }
                Err(nom::Err::Incomplete(_)) => return Ok(None),
                Err(_) => {
                    // A line we can't read: skip it rather than stall, and
                    // go on with what came after it (often the tagged reply
                    // we're waiting for, already here).
                    let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") else { return Ok(None) };
                    let line = String::from_utf8_lossy(&self.buf[..i]).to_string();
                    self.buf.drain(..i + 2);
                    if !line.starts_with('*') {
                        return Err(ImapError::Protocol(line));
                    }
                }
            }
        }
    }

    fn fill(&mut self) -> Result<(), ImapError> {
        let mut chunk = [0u8; 64 * 1024];
        let n = self.stream.read(&mut chunk)?;
        if n == 0 {
            return Err(ImapError::Bye("connection closed".into()));
        }
        self.buf.extend_from_slice(&chunk[..n]);
        if self.buf.len() > MAX_REPLY {
            return Err(ImapError::Protocol("reply too large".into()));
        }
        Ok(())
    }
}

fn fail(status: Status, info: &str) -> ImapError {
    match status {
        Status::No => ImapError::No(info.to_string()),
        Status::Bad => ImapError::Bad(info.to_string()),
        Status::Bye => ImapError::Bye(info.to_string()),
        _ => ImapError::Protocol(info.to_string()),
    }
}

fn cap_name(c: &imap_proto::Capability) -> String {
    match c {
        imap_proto::Capability::Imap4rev1 => "IMAP4rev1".into(),
        imap_proto::Capability::Auth(a) => format!("AUTH={a}"),
        imap_proto::Capability::Atom(a) => a.to_string(),
    }
}

/// A quoted string, for names and search terms.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A user name or password: quoted, unless it has characters a quoted
/// string can't hold (then the caller should use a literal; we refuse).
fn astring(s: &str) -> String {
    quote(&s.replace(['\r', '\n'], ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// An unreadable untagged line that arrives together with the tagged
    /// reply mustn't leave the client waiting for more from the network.
    #[test]
    fn a_line_it_cannot_read_doesnt_hide_the_reply_after_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n").unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let tag = line.split(' ').next().unwrap().to_string();
            s.write_all(format!("* 1 FETCH (((( nonsense\r\n{tag} OK done\r\n").as_bytes()).unwrap();
            // Keep the connection open (silent) until the client is done.
            let _ = r.read_line(&mut line);
        });
        let mut c =
            Client::connect(&Server { host: "127.0.0.1".into(), port, security: Security::None }).expect("connects");
        c.stream.set_timeout(Some(Duration::from_secs(3))).unwrap();
        c.noop().expect("the tagged OK is read without waiting");
        drop(c);
        server.join().unwrap();
    }

    /// A pinned address is connected to, not what the name (and port) say.
    #[test]
    fn a_pinned_address_is_the_one_connected_to() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let pin = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n").unwrap();
        });
        // Port 9 on this computer isn't an IMAP server: only the pin works.
        let at = Server { host: "localhost".into(), port: 9, security: Security::None };
        let c = Client::connect_pinned(&at, Some(pin)).expect("connects to the pin");
        assert!(c.has("IMAP4rev1"));
        drop(c);
        server.join().unwrap();
    }
}
