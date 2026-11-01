//! Signing in with a provider's token (XOAUTH2) instead of a password, and a
//! Microsoft 365 mailbox through a stand-in for Microsoft Graph.

mod common;

use common::{imap_server, smtp_server};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xenify_mail_core::account::Account;
use xenify_mail_core::auth::{Secret, TokenSource};
use xenify_mail_core::imap::{Client, Security, Server};
use xenify_mail_core::store::Store;
use xenify_mail_core::sync::{Cmd, Event};
use xenify_mail_core::{graph, send};

struct Fixed(String);
impl TokenSource for Fixed {
    fn token(&self) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

fn token(t: &str) -> Secret {
    Secret::Token(Arc::new(Fixed(t.into())))
}

#[test]
fn imap_and_smtp_take_tokens() {
    let (iport, st) = imap_server::start("sam@example.org", "tok-123");
    st.lock().unwrap().add("INBOX", &imap_server::message("Ana <ana@example.com>", "Hi", "Hello"), &[]);
    let (sport, sent) = smtp_server::start("sam@example.org", "tok-123");
    let local = |port| Server { host: "127.0.0.1".into(), port, security: Security::None };
    let a = Account {
        email: "sam@example.org".into(),
        name: "Sam".into(),
        user: "sam@example.org".into(),
        imap: local(iport),
        smtp: local(sport),
    };

    let mut c = Client::connect(&a.imap).unwrap();
    assert!(c.sign_in(&a.user, &token("wrong")).is_err(), "a wrong token is refused");
    let mut c2 = Client::connect(&a.imap).unwrap();
    c2.sign_in(&a.user, &token("tok-123")).unwrap();
    assert!(!c2.list().unwrap().is_empty());
    drop(c);

    let m = send::build(&send::Outgoing {
        from_email: a.email.clone(),
        to: "ana@example.com".into(),
        subject: "Re: Hi".into(),
        text: "Yes".into(),
        ..Default::default()
    })
    .unwrap();
    send::send_with(&a, &token("tok-123"), &m).unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert!(send::send_with(&a, &token("nope"), &m).is_err());

    // The sync worker signs in with it too.
    let dir = std::env::temp_dir().join(format!("xenify-mail-tok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("m.sqlite");
    let h = xenify_mail_core::sync::start_with(a.clone(), token("tok-123"), path.clone());
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        while let Ok(e) = h.events.try_recv() {
            if let Event::SignInFailed(t) | Event::Problem(t) = e {
                panic!("{t}");
            }
        }
        if Store::open(&path)
            .ok()
            .and_then(|s| s.folder_with(xenify_mail_core::imap::Role::Inbox).ok().flatten())
            .is_some_and(|w| Store::open(&path).unwrap().list(&w, 10).unwrap().len() == 1)
        {
            break;
        }
        assert!(Instant::now() < until, "synced with a token");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A small stand-in for Graph: two folders, one message, and what was asked.
#[derive(Default)]
struct FakeGraph {
    asked: Vec<String>,
    read: bool,
    sent: Vec<String>,
    deleted: bool,
}

fn fake_graph() -> (String, Arc<Mutex<FakeGraph>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}/v1.0", l.local_addr().unwrap().port());
    let st: Arc<Mutex<FakeGraph>> = Arc::default();
    let s2 = st.clone();
    let b2 = base.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut conn) = conn else { continue };
            let (st, base) = (s2.clone(), b2.clone());
            std::thread::spawn(move || {
                let mut rd = BufReader::new(conn.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if rd.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let (method, path) =
                        (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                    let mut len = 0;
                    let mut auth = String::new();
                    loop {
                        let mut h = String::new();
                        rd.read_line(&mut h).unwrap();
                        let h = h.trim_end();
                        if h.is_empty() {
                            break;
                        }
                        let lower = h.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        if lower.starts_with("authorization:") {
                            auth = h[14..].trim().to_string();
                        }
                    }
                    let mut body = vec![0; len];
                    rd.read_exact(&mut body).unwrap();
                    let mut g = st.lock().unwrap();
                    g.asked.push(format!("{method} {path}"));
                    let p = path.split('?').next().unwrap_or("").to_string();
                    let (code, ctype, out): (u16, &str, String) = if auth != "Bearer graph-token" {
                        (401, "application/json", r#"{"error":{"message":"bad token"}}"#.into())
                    } else if p.ends_with("/mailFolders/inbox") {
                        (200, "application/json", r#"{"id":"IN"}"#.into())
                    } else if p.ends_with("/mailFolders/deleteditems") {
                        (200, "application/json", r#"{"id":"DEL"}"#.into())
                    } else if p.contains("/mailFolders/") && !p.contains("/messages") {
                        (404, "application/json", r#"{"error":{"message":"none"}}"#.into())
                    } else if p.ends_with("/mailFolders") {
                        (200, "application/json", r#"{"value":[{"id":"IN","displayName":"Inbox"},{"id":"DEL","displayName":"Deleted Items"}]}"#.into())
                    } else if p.contains("/mailFolders/IN/messages/delta") && !g.deleted {
                        let m = format!(
                            r#"{{"value":[{{"id":"M1","subject":"Quarter numbers","isRead":{},"from":{{"emailAddress":{{"name":"Ana","address":"ana@example.com"}}}},"receivedDateTime":"2026-10-30T10:00:00Z"}}],"@odata.deltaLink":"{base}/users/sam%40example.org/mailFolders/IN/messages/delta?token=1"}}"#,
                            g.read
                        );
                        (200, "application/json", m)
                    } else if p.contains("/messages/delta") {
                        let removed =
                            if p.contains("/IN/") { r#"{"id":"M1","@removed":{"reason":"deleted"}}"# } else { "" };
                        (
                            200,
                            "application/json",
                            format!(
                                r#"{{"value":[{removed}],"@odata.deltaLink":"{base}/users/sam%40example.org/mailFolders/DEL/messages/delta?token=2"}}"#
                            ),
                        )
                    } else if p.ends_with("/messages/M1/$value") {
                        (
                            200,
                            "message/rfc822",
                            "From: Ana <ana@example.com>\r\nSubject: Quarter numbers\r\n\r\nUp 12%.\r\n".into(),
                        )
                    } else if method == "PATCH" && p.ends_with("/messages/M1") {
                        g.read = String::from_utf8_lossy(&body).contains("\"isRead\":true");
                        (200, "application/json", "{}".into())
                    } else if p.ends_with("/messages/M1/move") {
                        g.deleted = true;
                        (201, "application/json", r#"{"id":"M1b"}"#.into())
                    } else if p.ends_with("/sendMail") {
                        g.sent.push(String::from_utf8_lossy(&body).into_owned());
                        (202, "application/json", String::new())
                    } else {
                        (404, "application/json", r#"{"error":{"message":"not here"}}"#.into())
                    };
                    drop(g);
                    let _ = write!(
                        conn,
                        "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n{out}",
                        out.len()
                    );
                }
            });
        }
    });
    (base, st)
}

#[test]
fn a_microsoft_365_mailbox_through_graph() {
    let (base, st) = fake_graph();
    let dir = std::env::temp_dir().join(format!("xenify-mail-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("m.sqlite");
    let mb = graph::Mailbox {
        base: base.clone(),
        user: "sam@example.org".into(),
        tokens: Arc::new(Fixed("graph-token".into())),
    };
    graph::check(&mb).unwrap();
    let bad = graph::Mailbox { tokens: Arc::new(Fixed("old".into())), ..mb.clone() };
    assert!(graph::check(&bad).unwrap_err().contains("didn't allow"));

    let h = graph::start(mb.clone(), path.clone());
    let wait = |what: &str, ok: &dyn Fn(&Store) -> bool| {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            while let Ok(e) = h.events.try_recv() {
                if let Event::SignInFailed(t) | Event::Problem(t) = e {
                    panic!("{what}: {t}");
                }
            }
            if Store::open(&path).is_ok_and(|s| ok(&s)) {
                return;
            }
            assert!(Instant::now() < until, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    wait("the inbox lists the message", &|s| {
        s.list("IN", 10).is_ok_and(|l| l.len() == 1 && l[0].subject == "Quarter numbers" && !l[0].is_read())
    });
    h.send(Cmd::Download { folder: "IN".into(), uid: 1 });
    wait("downloaded", &|s| {
        s.raw("IN", 1).is_ok_and(|r| r.is_some_and(|r| String::from_utf8_lossy(&r).contains("Up 12%")))
    });
    h.send(Cmd::SetFlag { folder: "IN".into(), uid: 1, flag: "\\Seen".into(), on: true });
    wait("read", &|s| s.row("IN", 1).is_ok_and(|r| r.is_some_and(|r| r.is_read())));
    assert!(st.lock().unwrap().read);
    h.send(Cmd::Delete { folder: "IN".into(), uid: 1 });
    wait("deleted (moved to Deleted Items)", &|s| s.list("IN", 10).is_ok_and(|l| l.is_empty()));
    assert!(st.lock().unwrap().deleted);

    graph::send(&mb, b"From: sam@example.org\r\nTo: ana@example.com\r\nSubject: Hi\r\n\r\nHello\r\n").unwrap();
    let sent = st.lock().unwrap().sent.clone();
    assert_eq!(sent.len(), 1);
    use base64::Engine;
    let mime = base64::engine::general_purpose::STANDARD.decode(sent[0].trim()).unwrap();
    assert!(String::from_utf8_lossy(&mime).contains("Subject: Hi"));
}
