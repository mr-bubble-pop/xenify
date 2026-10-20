//! The sync worker against the in-memory test server.

mod common;

use common::imap_server::{self, message};
use xenify_mail_core::account::Account;
use xenify_mail_core::imap::{Security, Server};
use xenify_mail_core::store::Store;
use xenify_mail_core::sync::{self, Cmd, Event, Handle};
use std::time::{Duration, Instant};

fn account(port: u16) -> Account {
    let local = Server { host: "127.0.0.1".into(), port, security: Security::None };
    Account {
        email: "sam@example.org".into(),
        name: "Sam".into(),
        user: "sam".into(),
        imap: local.clone(),
        smtp: local,
    }
}

fn temp_store(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xenify-mail-sync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("mail.sqlite")
}

/// Waits until `ok` holds for the store (reading it fresh each time).
fn wait_for(h: &Handle, path: &std::path::Path, what: &str, ok: impl Fn(&Store) -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        while let Ok(e) = h.events.try_recv() {
            if std::env::var_os("SYNC_DEBUG").is_some() {
                eprintln!("{what}: {e:?}");
            }
            if let Event::Problem(p) | Event::SignInFailed(p) = &e {
                panic!("{what}: {p}");
            }
        }
        if let Ok(s) = Store::open(path)
            && ok(&s)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

#[test]
fn syncs_new_mail_flags_downloads_and_deletes() {
    let (port, st) = imap_server::start("sam", "pw");
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", &message("Ana <ana@example.com>", "Lunch", "Pizza at noon?"), &[]);
        s.add("INBOX", &message("Bo <bo@example.com>", "Report", "Totals inside."), &["\\Seen"]);
        s.add("Sent", &message("Sam <sam@example.org>", "Re: Lunch", "Yes!"), &["\\Seen"]);
    }
    let path = temp_store("main");
    let h = sync::start(account(port), "pw".into(), path.clone());

    wait_for(&h, &path, "first sync", |s| {
        s.list("INBOX", 10).is_ok_and(|l| l.len() == 2) && s.list("Sent", 10).is_ok_and(|l| l.len() == 1)
    });
    let s = Store::open(&path).unwrap();
    let inbox = s.folders().unwrap().into_iter().find(|f| f.wire == "INBOX").unwrap();
    assert_eq!((inbox.unread, inbox.total), (1, 2));
    assert_eq!(s.list("INBOX", 10).unwrap()[1].subject, "Lunch");
    drop(s);

    // New mail arrives while the worker is idling: it shows up by itself.
    st.lock().unwrap().add("INBOX", &message("Cy <cy@example.com>", "Urgent", "Call me"), &[]);
    wait_for(&h, &path, "pushed mail", |s| s.list("INBOX", 10).is_ok_and(|l| l.iter().any(|r| r.subject == "Urgent")));

    // Opening a message downloads it.
    h.send(Cmd::Download { folder: "INBOX".into(), uid: 1 });
    wait_for(&h, &path, "download", |s| s.raw("INBOX", 1).is_ok_and(|r| r.is_some()));
    let s = Store::open(&path).unwrap();
    assert_eq!(s.search("pizza", 10).unwrap().len(), 1, "downloaded text is searchable");
    drop(s);

    // Marking read reaches the server.
    h.send(Cmd::SetFlag { folder: "INBOX".into(), uid: 1, flag: "\\Seen".into(), on: true });
    wait_for(&h, &path, "flag", |s| s.row("INBOX", 1).is_ok_and(|r| r.is_some_and(|r| r.is_read())));
    assert!(st.lock().unwrap().boxes["INBOX"].msgs[0].flags.contains(&"\\Seen".to_string()));

    // Read elsewhere (another device): picked up on the next check.
    st.lock().unwrap().boxes.get_mut("INBOX").unwrap().msgs[2].flags.push("\\Seen".into());
    h.send(Cmd::SyncNow);
    wait_for(&h, &path, "flag from elsewhere", |s| s.row("INBOX", 3).is_ok_and(|r| r.is_some_and(|r| r.is_read())));

    // Delete: into the Trash.
    h.send(Cmd::Delete { folder: "INBOX".into(), uid: 2 });
    wait_for(&h, &path, "delete", |s| {
        s.row("INBOX", 2).is_ok_and(|r| r.is_none()) && s.list("Trash", 10).is_ok_and(|l| l.len() == 1)
    });
    assert_eq!(st.lock().unwrap().boxes["Trash"].msgs.len(), 1);
}

#[test]
fn a_wrong_password_stops_and_says_so() {
    let (port, _) = imap_server::start("sam", "pw");
    let h = sync::start(account(port), "nope".into(), temp_store("auth"));
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < until, "no sign-in failure reported");
        if let Ok(Event::SignInFailed(m)) = h.events.recv_timeout(Duration::from_millis(100)) {
            assert!(m.contains("Invalid credentials"), "{m}");
            break;
        }
    }
}
