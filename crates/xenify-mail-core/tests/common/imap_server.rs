//! A small in-memory IMAP server for tests (plain TCP on 127.0.0.1). It
//! speaks just what Xenify Mail uses, the way real servers answer.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Msg {
    pub uid: u32,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Mailbox {
    pub uid_validity: u32,
    pub uid_next: u32,
    pub special: Option<&'static str>,
    pub msgs: Vec<Msg>,
}

#[derive(Default)]
pub struct State {
    pub boxes: BTreeMap<String, Mailbox>,
    pub user: String,
    pub password: String,
    /// Commands received (without tags), for assertions.
    pub log: Vec<String>,
    /// Bumped on every change, so IDLE can tell clients.
    pub version: u64,
}

impl State {
    pub fn add(&mut self, mailbox: &str, raw: &str, flags: &[&str]) -> u32 {
        let b = self.boxes.get_mut(mailbox).expect("mailbox");
        let uid = b.uid_next;
        b.uid_next += 1;
        b.msgs.push(Msg {
            uid,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            raw: raw.replace('\n', "\r\n").into_bytes(),
        });
        self.version += 1;
        uid
    }
}

pub type Shared = Arc<Mutex<State>>;

/// Starts a server with Inbox, Sent, Drafts, Trash and a non-ASCII folder.
/// Returns its port and state.
pub fn start(user: &str, password: &str) -> (u16, Shared) {
    let mut st = State { user: user.into(), password: password.into(), ..Default::default() };
    for (name, special) in [
        ("INBOX", None),
        ("Sent", Some("\\Sent")),
        ("Drafts", Some("\\Drafts")),
        ("Trash", Some("\\Trash")),
        ("Entw&APw-rfe", None),
    ] {
        st.boxes.insert(name.into(), Mailbox { uid_validity: 7, uid_next: 1, special, msgs: Vec::new() });
    }
    let shared = Arc::new(Mutex::new(st));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let s = shared.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let s = s.clone();
            std::thread::spawn(move || {
                let _ = session(conn, s);
            });
        }
    });
    (port, shared)
}

fn session(conn: TcpStream, st: Shared) -> std::io::Result<()> {
    let mut out = conn.try_clone()?;
    let mut rd = BufReader::new(conn);
    let caps = "IMAP4rev1 AUTH=PLAIN SASL-IR IDLE MOVE UIDPLUS SPECIAL-USE";
    write!(out, "* OK [CAPABILITY {caps}] test server ready\r\n")?;
    let mut selected: Option<String> = None;
    let mut authed = false;
    loop {
        let mut line = String::new();
        if rd.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end().to_string();
        let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
        let (cmd, args) = rest.split_once(' ').unwrap_or((rest, ""));
        let cmd = cmd.to_ascii_uppercase();
        st.lock().unwrap().log.push(if cmd == "LOGIN" || cmd == "AUTHENTICATE" {
            cmd.clone()
        } else {
            rest.to_string()
        });
        match cmd.as_str() {
            "CAPABILITY" => write!(out, "* CAPABILITY {caps}\r\n{tag} OK done\r\n")?,
            "NOOP" => write!(out, "{tag} OK done\r\n")?,
            "LOGOUT" => {
                write!(out, "* BYE bye\r\n{tag} OK done\r\n")?;
                return Ok(());
            }
            "LOGIN" => {
                let parts = quoted_args(args);
                let s = st.lock().unwrap();
                if parts.len() == 2 && parts[0] == s.user && parts[1] == s.password {
                    authed = true;
                    write!(out, "{tag} OK logged in\r\n")?;
                } else {
                    write!(out, "{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")?;
                }
            }
            "AUTHENTICATE" => {
                use base64::Engine;
                let token = args.split_whitespace().nth(1).unwrap_or("");
                let decoded = base64::engine::general_purpose::STANDARD.decode(token).unwrap_or_default();
                let parts: Vec<&[u8]> = decoded.split(|b| *b == 0).collect();
                let s = st.lock().unwrap();
                if parts.len() == 3 && parts[1] == s.user.as_bytes() && parts[2] == s.password.as_bytes() {
                    authed = true;
                    write!(out, "{tag} OK [CAPABILITY {caps}] authenticated\r\n")?;
                } else {
                    write!(out, "{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")?;
                }
            }
            _ if !authed => write!(out, "{tag} BAD sign in first\r\n")?,
            "LIST" => {
                let s = st.lock().unwrap();
                for (name, b) in &s.boxes {
                    let attrs = b.special.unwrap_or("\\HasNoChildren");
                    write!(out, "* LIST ({attrs}) \"/\" \"{name}\"\r\n")?;
                }
                write!(out, "{tag} OK done\r\n")?;
            }
            "SELECT" | "EXAMINE" => {
                let name = quoted_args(args).first().cloned().unwrap_or_default();
                let s = st.lock().unwrap();
                match s.boxes.get(&name) {
                    Some(b) => {
                        write!(out, "* {} EXISTS\r\n* 0 RECENT\r\n", b.msgs.len())?;
                        write!(
                            out,
                            "* OK [UIDVALIDITY {}] ok\r\n* OK [UIDNEXT {}] ok\r\n",
                            b.uid_validity, b.uid_next
                        )?;
                        write!(out, "* FLAGS (\\Seen \\Answered \\Flagged \\Deleted \\Draft)\r\n")?;
                        write!(out, "{tag} OK [READ-WRITE] selected\r\n")?;
                        selected = Some(name);
                    }
                    None => write!(out, "{tag} NO no such mailbox\r\n")?,
                }
            }
            "UID" => {
                let (sub, rest) = args.split_once(' ').unwrap_or((args, ""));
                let Some(sel) = selected.clone() else {
                    write!(out, "{tag} BAD select first\r\n")?;
                    continue;
                };
                uid_command(&mut out, &st, &sel, &sub.to_ascii_uppercase(), rest, tag)?;
            }
            "APPEND" => {
                // APPEND "box" (flags) {n}
                let name = quoted_args(args).first().cloned().unwrap_or_default();
                let flags: Vec<String> = args
                    .split_once('(')
                    .and_then(|(_, r)| r.split_once(')'))
                    .map(|(f, _)| f.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default();
                let n: usize =
                    args.rsplit_once('{').and_then(|(_, r)| r.trim_end_matches('}').parse().ok()).unwrap_or(0);
                write!(out, "+ go ahead\r\n")?;
                out.flush()?;
                let mut data = vec![0; n];
                rd.read_exact(&mut data)?;
                let mut crlf = String::new();
                rd.read_line(&mut crlf)?;
                let mut s = st.lock().unwrap();
                let s = &mut *s;
                match s.boxes.get_mut(&name) {
                    Some(b) => {
                        let uid = b.uid_next;
                        b.uid_next += 1;
                        b.msgs.push(Msg { uid, flags, raw: data });
                        s.version += 1;
                        write!(out, "{tag} OK [APPENDUID {} {uid}] appended\r\n", b.uid_validity)?;
                    }
                    None => write!(out, "{tag} NO [TRYCREATE] no such mailbox\r\n")?,
                }
            }
            "IDLE" => {
                write!(out, "+ idling\r\n")?;
                out.flush()?;
                let start = st.lock().unwrap().version;
                let count = selected.as_ref().map(|m| st.lock().unwrap().boxes[m].msgs.len()).unwrap_or(0);
                rd.get_ref().set_read_timeout(Some(Duration::from_millis(50)))?;
                let mut told = false;
                // Kept across timeouts: a read can stop halfway through "DONE".
                let mut l = String::new();
                loop {
                    match rd.read_line(&mut l) {
                        Ok(0) => return Ok(()),
                        Ok(_) if l.trim().eq_ignore_ascii_case("DONE") => break,
                        Ok(_) => l.clear(),
                        Err(_) => {}
                    }
                    let s = st.lock().unwrap();
                    if !told && s.version != start {
                        let n = selected.as_ref().map(|m| s.boxes[m].msgs.len()).unwrap_or(count);
                        write!(out, "* {n} EXISTS\r\n")?;
                        out.flush()?;
                        told = true;
                    }
                }
                rd.get_ref().set_read_timeout(None)?;
                write!(out, "{tag} OK idle done\r\n")?;
            }
            _ => write!(out, "{tag} BAD unknown command\r\n")?,
        }
        out.flush()?;
    }
}

fn uid_command(out: &mut TcpStream, st: &Shared, sel: &str, sub: &str, rest: &str, tag: &str) -> std::io::Result<()> {
    let mut s = st.lock().unwrap();
    let s = &mut *s;
    match sub {
        "SEARCH" => {
            let b = &s.boxes[sel];
            let uids: Vec<u32> = if let Some(set) = rest.strip_prefix("UID ") {
                b.msgs.iter().map(|m| m.uid).filter(|u| in_set(set, *u, b.uid_next)).collect()
            } else if rest.eq_ignore_ascii_case("UNSEEN") {
                b.msgs.iter().filter(|m| !m.flags.iter().any(|f| f == "\\Seen")).map(|m| m.uid).collect()
            } else {
                b.msgs.iter().map(|m| m.uid).collect()
            };
            let list: Vec<String> = uids.iter().map(|u| u.to_string()).collect();
            write!(out, "* SEARCH {}\r\n{tag} OK done\r\n", list.join(" "))?;
        }
        "FETCH" => {
            let (set, items) = rest.split_once(' ').unwrap_or((rest, ""));
            let b = &s.boxes[sel];
            for (i, m) in b.msgs.iter().enumerate() {
                if !in_set(set, m.uid, b.uid_next) {
                    continue;
                }
                let mut parts = vec![format!("UID {}", m.uid)];
                let upper = items.to_ascii_uppercase();
                if upper.contains("FLAGS") {
                    parts.push(format!("FLAGS ({})", m.flags.join(" ")));
                }
                if upper.contains("RFC822.SIZE") {
                    parts.push(format!("RFC822.SIZE {}", m.raw.len()));
                }
                if upper.contains("INTERNALDATE") {
                    parts.push("INTERNALDATE \"15-Oct-2026 09:41:00 +0100\"".into());
                }
                let mut tail: Vec<u8> = Vec::new();
                if upper.contains("HEADER.FIELDS") {
                    let end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(m.raw.len());
                    let header = &m.raw[..end];
                    tail.extend(
                        format!(" BODY[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID)] {{{}}}\r\n", header.len())
                            .bytes(),
                    );
                    tail.extend_from_slice(header);
                } else if upper.contains("BODY.PEEK[]") {
                    tail.extend(format!(" BODY[] {{{}}}\r\n", m.raw.len()).bytes());
                    tail.extend_from_slice(&m.raw);
                }
                write!(out, "* {} FETCH ({}", i + 1, parts.join(" "))?;
                out.write_all(&tail)?;
                write!(out, ")\r\n")?;
            }
            write!(out, "{tag} OK done\r\n")?;
        }
        "STORE" => {
            // set +FLAGS.SILENT (\Seen)
            let mut it = rest.splitn(3, ' ');
            let set = it.next().unwrap_or("");
            let op = it.next().unwrap_or("");
            let flags: Vec<String> =
                it.next().unwrap_or("").trim_matches(['(', ')']).split_whitespace().map(str::to_string).collect();
            let b = s.boxes.get_mut(sel).unwrap();
            let next = b.uid_next;
            for m in b.msgs.iter_mut().filter(|m| in_set(set, m.uid, next)) {
                for f in &flags {
                    if op.starts_with('+') && !m.flags.contains(f) {
                        m.flags.push(f.clone());
                    } else if op.starts_with('-') {
                        m.flags.retain(|x| x != f);
                    }
                }
            }
            s.version += 1;
            write!(out, "{tag} OK done\r\n")?;
        }
        "MOVE" => {
            let (set, target) = rest.split_once(' ').unwrap_or((rest, ""));
            let target = quoted_args(target).first().cloned().unwrap_or_default();
            if !s.boxes.contains_key(&target) {
                write!(out, "{tag} NO [TRYCREATE] no such mailbox\r\n")?;
                return Ok(());
            }
            let next = s.boxes[sel].uid_next;
            let moving: Vec<Msg> = s.boxes[sel].msgs.iter().filter(|m| in_set(set, m.uid, next)).cloned().collect();
            s.boxes.get_mut(sel).unwrap().msgs.retain(|m| !in_set(set, m.uid, next));
            let t = s.boxes.get_mut(&target).unwrap();
            for mut m in moving {
                m.uid = t.uid_next;
                t.uid_next += 1;
                t.msgs.push(m);
            }
            s.version += 1;
            write!(out, "{tag} OK moved\r\n")?;
        }
        _ => write!(out, "{tag} BAD unknown UID command\r\n")?,
    }
    Ok(())
}

/// `1:4,7,9:*` contains `uid`?
fn in_set(set: &str, uid: u32, next: u32) -> bool {
    set.split(',').any(|part| {
        let num = |s: &str| if s == "*" { next.saturating_sub(1).max(1) } else { s.parse().unwrap_or(0) };
        match part.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (num(a), if b == "*" { u32::MAX } else { num(b) });
                (a.min(b)..=a.max(b)).contains(&uid)
            }
            None => num(part) == uid,
        }
    })
}

/// The quoted (or bare) arguments: `"a b" c` → [a b, c].
fn quoted_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == ' ' {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut v = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => v.extend(chars.next()),
                    '"' => break,
                    c => v.push(c),
                }
            }
            out.push(v);
        } else {
            let mut v = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' {
                    break;
                }
                v.push(c);
                chars.next();
            }
            out.push(v);
        }
    }
    out
}

/// A simple message.
pub fn message(from: &str, subject: &str, body: &str) -> String {
    format!(
        "From: {from}\nTo: Sam Lee <sam@example.org>\nSubject: {subject}\nDate: Thu, 15 Oct 2026 09:41:00 +0100\nMessage-ID: <{}@test>\nContent-Type: text/plain; charset=utf-8\n\n{body}\n",
        subject.replace(' ', "-")
    )
}
