//! The IMAP client against the in-memory test server.

mod common;

use common::imap_server::{self, message};
use xenify_mail_core::imap::{Client, ImapError, Role, Security, Server};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn connect(port: u16) -> Client {
    Client::connect(&Server { host: "127.0.0.1".into(), port, security: Security::None }).unwrap()
}

#[test]
fn signs_in_lists_folders_and_reads_mail() {
    let (port, st) = imap_server::start("sam@example.org", "pa\"ss wörd");
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", &message("Ana <ana@example.com>", "Lunch", "Pizza at noon?"), &[]);
        s.add("INBOX", &message("Bo <bo@example.com>", "Report", "Attached."), &["\\Seen"]);
    }
    let mut bad = connect(port);
    assert!(matches!(bad.login("sam@example.org", "wrong"), Err(ImapError::Auth(_))));

    let mut c = connect(port);
    c.login("sam@example.org", "pa\"ss wörd").unwrap();
    assert!(c.has("IDLE") && c.has("MOVE"));

    let folders = c.list().unwrap();
    let find = |n: &str| folders.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("{n} in {folders:?}"));
    assert_eq!(find("Inbox").role, Some(Role::Inbox));
    assert_eq!(find("Sent").role, Some(Role::Sent));
    assert_eq!(find("Entwürfe").wire, "Entw&APw-rfe");

    let sel = c.select("INBOX", false).unwrap();
    assert_eq!((sel.exists, sel.uid_validity, sel.uid_next), (2, 7, 3));
    assert_eq!(c.uid_search("ALL").unwrap(), [1, 2]);
    let list = c.fetch_summaries("1:*").unwrap();
    assert_eq!(list.len(), 2);
    assert!(String::from_utf8_lossy(&list[0].header).contains("Subject: Lunch"));
    assert_eq!(list[1].flags, ["\\Seen"]);

    let raw = c.fetch_message(1).unwrap().unwrap();
    let m = xenify_mail_core::message::parse(&raw).unwrap();
    assert_eq!(m.subject, "Lunch");
    assert!(m.text.contains("Pizza at noon?"));
    c.logout();
}

#[test]
fn flags_moves_and_appends() {
    let (port, st) = imap_server::start("u", "p");
    st.lock().unwrap().add("INBOX", &message("a@example.com", "One", "1"), &[]);
    let mut c = connect(port);
    c.login("u", "p").unwrap();
    c.select("INBOX", false).unwrap();
    c.store_flags("1", true, &["\\Seen", "\\Flagged"]).unwrap();
    assert_eq!(c.fetch_flags("1").unwrap(), [(1, vec!["\\Seen".to_string(), "\\Flagged".to_string()])]);
    c.store_flags("1", false, &["\\Flagged"]).unwrap();
    assert_eq!(c.fetch_flags("1").unwrap()[0].1, ["\\Seen"]);

    c.move_to("1", "Trash").unwrap();
    assert!(c.uid_search("ALL").unwrap().is_empty());
    assert_eq!(st.lock().unwrap().boxes["Trash"].msgs.len(), 1);

    c.append("Sent", &["\\Seen"], message("u@example.org", "Hi", "hello").as_bytes()).unwrap();
    assert_eq!(st.lock().unwrap().boxes["Sent"].msgs.len(), 1);
    assert!(matches!(c.move_to("1", "Nowhere"), Err(ImapError::No(_))));
}

#[test]
fn idle_wakes_up_when_mail_arrives() {
    let (port, st) = imap_server::start("u", "p");
    let mut c = connect(port);
    c.login("u", "p").unwrap();
    c.select("INBOX", false).unwrap();
    let stop = AtomicBool::new(false);
    // Nothing happens: comes back after the wait with no news.
    let t = Instant::now();
    assert!(!c.idle(Duration::from_millis(300), &stop).unwrap());
    assert!(t.elapsed() >= Duration::from_millis(250));
    // Mail arrives while idling: it returns early with news.
    let st2 = st.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        st2.lock().unwrap().add("INBOX", &message("x@example.com", "New", "!"), &[]);
    });
    let t = Instant::now();
    assert!(c.idle(Duration::from_secs(10), &stop).unwrap());
    assert!(t.elapsed() < Duration::from_secs(5));
    // The connection still works afterwards.
    assert_eq!(c.uid_search("ALL").unwrap(), [1]);
}

/// Against a real IMAP server (e.g. a local Dovecot), with
/// `XENIFY_TEST_IMAP=host:port:user:password` (plain connection). Run with
/// `--ignored`.
#[test]
#[ignore]
fn against_a_real_server() {
    let spec = std::env::var("XENIFY_TEST_IMAP").expect("XENIFY_TEST_IMAP=host:port:user:password");
    let mut parts = spec.splitn(4, ':');
    let (host, port, user, pass) =
        (parts.next().unwrap(), parts.next().unwrap().parse().unwrap(), parts.next().unwrap(), parts.next().unwrap());
    let server = Server { host: host.into(), port, security: Security::None };
    let open = || {
        let mut c = Client::connect(&server).unwrap();
        c.login(user, pass).unwrap();
        c
    };
    let mut c = open();
    let folders = c.list().unwrap();
    assert!(folders.iter().any(|f| f.role == Some(Role::Inbox)), "{folders:?}");
    let trash = folders.iter().find(|f| f.role == Some(Role::Trash)).expect("a Trash folder").wire.clone();
    let before = c.select("INBOX", false).unwrap();
    let subject = format!("Xenify test {}", std::process::id());
    c.append("INBOX", &[], message("Ana <ana@example.com>", &subject, "Hello from the test, with ünïcödé.").as_bytes())
        .unwrap();
    let sel = c.select("INBOX", false).unwrap();
    assert_eq!(sel.exists, before.exists + 1);
    let uids = c.uid_search("ALL").unwrap();
    let uid = *uids.last().unwrap();
    let list = c.fetch_summaries(&uid.to_string()).unwrap();
    assert!(String::from_utf8_lossy(&list[0].header).contains(&subject));
    assert!(!list[0].flags.iter().any(|f| f == "\\Seen"), "fetching the list doesn't mark it read");
    let raw = c.fetch_message(uid).unwrap().unwrap();
    assert!(xenify_mail_core::message::parse(&raw).unwrap().text.contains("ünïcödé"));
    assert!(!c.fetch_flags(&uid.to_string()).unwrap()[0].1.iter().any(|f| f == "\\Seen"), "PEEK keeps it unread");
    c.store_flags(&uid.to_string(), true, &["\\Seen"]).unwrap();
    assert!(c.fetch_flags(&uid.to_string()).unwrap()[0].1.iter().any(|f| f == "\\Seen"));

    // IDLE: another connection delivers mail.
    let stop = AtomicBool::new(false);
    let s2 = server.clone();
    let (u2, p2) = (user.to_string(), pass.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        let mut d = Client::connect(&s2).unwrap();
        d.login(&u2, &p2).unwrap();
        d.append("INBOX", &[], message("b@example.com", "Pushed", "!").as_bytes()).unwrap();
    });
    let t = Instant::now();
    assert!(c.idle(Duration::from_secs(20), &stop).unwrap());
    assert!(t.elapsed() < Duration::from_secs(10));

    c.move_to(&uid.to_string(), &trash).unwrap();
    assert!(!c.uid_search("ALL").unwrap().contains(&uid));
    c.logout();
}

/// TLS against a public server: connects and reads the capabilities, without
/// signing in. Needs the internet; run with `--ignored`.
#[test]
#[ignore]
fn tls_to_a_public_server() {
    for (host, port, security) in [("imap.gmail.com", 993, Security::Tls), ("imap.fastmail.com", 993, Security::Tls)] {
        let c =
            Client::connect(&Server { host: host.into(), port, security }).unwrap_or_else(|e| panic!("{host}: {e}"));
        assert!(c.has("IMAP4rev1"), "{host}: {:?}", c.capabilities);
        c.logout();
    }
}

#[test]
fn never_sends_a_password_unencrypted_over_the_network() {
    let e = Client::connect(&Server { host: "mail.example.org".into(), port: 143, security: Security::None });
    assert!(matches!(e, Err(ImapError::Connect(m)) if m.contains("only allowed")));
}
