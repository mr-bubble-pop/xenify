//! Sending through a test SMTP server, with the Sent copy on the test IMAP server.

mod common;

use common::{imap_server, smtp_server};
use xenify_mail_core::account::Account;
use xenify_mail_core::imap::{Security, Server};
use xenify_mail_core::send::{self, Outgoing};

#[test]
fn sends_and_keeps_a_copy_in_sent() {
    let (imap_port, imap) = imap_server::start("sam", "pw");
    let (smtp_port, outbox) = smtp_server::start("sam", "pw");
    let local = |port| Server { host: "127.0.0.1".into(), port, security: Security::None };
    let account = Account {
        email: "sam@example.org".into(),
        name: "Sam Lee".into(),
        user: "sam".into(),
        imap: local(imap_port),
        smtp: local(smtp_port),
    };
    let o = Outgoing {
        from_name: account.name.clone(),
        from_email: account.email.clone(),
        to: "Ana <ana@example.com>".into(),
        bcc: "boss@example.com".into(),
        subject: "Totals".into(),
        text: "They add up.".into(),
        html: "<p>They add up.</p>".into(),
        ..Default::default()
    };
    let m = send::build(&o).unwrap();
    send::send(&account, "pw", &m).unwrap();
    let got = outbox.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].to.len(), 2, "Bcc is a recipient…");
    assert!(!got[0].data.contains("boss@example.com"), "…but not in the headers");
    assert!(got[0].data.contains("Subject: Totals"));
    send::save_sent_copy(&account, "pw", &m.formatted()).unwrap();
    assert_eq!(imap.lock().unwrap().boxes["Sent"].msgs.len(), 1);

    // A wrong password is explained.
    let err = send::send(&account, "bad", &m).unwrap_err();
    assert!(err.contains("didn't accept the password"), "{err}");
    // Plain connections only to this computer.
    let mut far = account.clone();
    far.smtp.host = "mail.example.com".into();
    assert!(send::send(&far, "pw", &m).unwrap_err().contains("only allowed"));
}

/// Pinned: the message goes to the checked address, not where the name
/// (and port) point, and so does the copy in Sent.
#[test]
fn sends_to_the_pinned_address() {
    let (imap_port, imap) = imap_server::start("sam", "pw");
    let (smtp_port, outbox) = smtp_server::start("sam", "pw");
    // Port 9 on this computer isn't a mail server: only the pins work.
    let named = Server { host: "localhost".into(), port: 9, security: Security::None };
    let account = Account {
        email: "sam@example.org".into(),
        name: "Sam".into(),
        user: "sam".into(),
        imap: named.clone(),
        smtp: named,
    };
    let o = Outgoing {
        from_name: "Sam".into(),
        from_email: account.email.clone(),
        to: "ana@example.com".into(),
        subject: "Pinned".into(),
        text: "Hi".into(),
        ..Default::default()
    };
    let m = send::build(&o).unwrap();
    let secret = xenify_mail_core::auth::Secret::Password("pw".into());
    let pin = |port| Some(std::net::SocketAddr::from(([127, 0, 0, 1], port)));
    send::send_pinned(&account, &secret, &m, pin(smtp_port)).unwrap();
    assert_eq!(outbox.lock().unwrap().len(), 1);
    send::save_sent_copy_pinned(&account, &secret, &m.formatted(), pin(imap_port)).unwrap();
    assert_eq!(imap.lock().unwrap().boxes["Sent"].msgs.len(), 1);
}
