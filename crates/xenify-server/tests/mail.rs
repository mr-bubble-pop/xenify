//! The server's email against a small mail server of our own: welcome and
//! reset links, "Forgot password?" and "Email me a link" for new accounts.

mod common;

use common::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

macro_rules! db_or_skip {
    ($name:expr) => {
        match fresh_db($name).await {
            Some(db) => db,
            None => {
                eprintln!("no PostgreSQL to test with: skipped");
                return;
            }
        }
    };
}

/// One email the fake mail server took: who to, and the whole message.
#[derive(Clone, Debug)]
struct Got {
    to: String,
    data: String,
}

/// A mail server on this computer that keeps every message (plain SMTP, no
/// password: only allowed to 127.0.0.1).
async fn fake_smtp() -> (u16, Arc<Mutex<Vec<Got>>>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(Vec::new()));
    let kept = got.clone();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = l.accept().await else { return };
            let kept = kept.clone();
            tokio::spawn(async move {
                let (r, mut w) = s.into_split();
                let mut r = BufReader::new(r);
                w.write_all(b"220 fake ready\r\n").await.unwrap();
                let (mut to, mut line) = (String::new(), String::new());
                loop {
                    line.clear();
                    if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let cmd = line.to_ascii_uppercase();
                    if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                        w.write_all(b"250-fake\r\n250 8BITMIME\r\n").await.unwrap();
                    } else if cmd.starts_with("RCPT TO:") {
                        to = line[8..].trim().trim_matches(['<', '>']).to_string();
                        w.write_all(b"250 ok\r\n").await.unwrap();
                    } else if cmd.starts_with("DATA") {
                        w.write_all(b"354 go on\r\n").await.unwrap();
                        let mut data = String::new();
                        loop {
                            line.clear();
                            r.read_line(&mut line).await.unwrap();
                            if line == ".\r\n" {
                                break;
                            }
                            data.push_str(&line);
                        }
                        kept.lock().unwrap().push(Got { to: to.clone(), data });
                        w.write_all(b"250 kept\r\n").await.unwrap();
                    } else if cmd.starts_with("QUIT") {
                        w.write_all(b"221 bye\r\n").await.unwrap();
                        return;
                    } else {
                        w.write_all(b"250 ok\r\n").await.unwrap();
                    }
                }
            });
        }
    });
    (port, got)
}

/// The next email after the first `before` (the server sends some, such as
/// "Forgot password?", after answering): waits up to 5 seconds for it.
async fn next_mail(got: &Arc<Mutex<Vec<Got>>>, before: usize) -> Got {
    for _ in 0..100 {
        if let Some(m) = got.lock().unwrap().get(before) {
            return m.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no email came");
}

/// The code in an email ("Your code is XXXX-XXXX").
fn code_in(data: &str) -> String {
    let plain = data.replace("=\r\n", "").replace("=3D", "=");
    let start = plain.find("Your code is ").unwrap_or_else(|| panic!("no code in {plain}")) + "Your code is ".len();
    plain[start..start + 9].to_string()
}

async fn mail_server(addr: &str, admin: &str, port: u16) {
    let body = json!({ "host": "127.0.0.1", "port": port, "security": "none", "from_name": "Office server", "from_address": "files@example.org" });
    assert_eq!(post(addr, "/v1/admin/mail", Some(admin), body).await.0, 204);
}

#[tokio::test]
async fn people_get_their_links_by_email() {
    let db = db_or_skip!("mail");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, info) = get(&addr, "/v1/info", None).await;
    assert!(!info["features"].as_array().unwrap().contains(&json!("mail")));

    // Without a mail server: the admin gets a code to pass on.
    let (_, p) =
        post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "kim", "email": "kim@example.org" })).await;
    assert_eq!(p["emailed"], false);
    assert_eq!(p["code"].as_str().unwrap().len(), 14);

    // Only plain SMTP to this computer is allowed; the password is kept, never shown.
    let (port, got) = fake_smtp().await;
    let bad = json!({ "host": "mail.example.org", "security": "none", "from_address": "files@example.org" });
    assert_eq!(post(&addr, "/v1/admin/mail", Some(&admin), bad).await.0, 400);
    let body = json!({ "host": "127.0.0.1", "port": port, "security": "none", "username": "", "password": "secret", "from_name": "Office server", "from_address": "files@example.org" });
    assert_eq!(post(&addr, "/v1/admin/mail", Some(&admin), body).await.0, 204);
    let (_, m) = get(&addr, "/v1/admin/mail", Some(&admin)).await;
    assert_eq!((m["password"].clone(), m["password_set"].clone()), (json!(""), json!(true)));
    mail_server(&addr, &admin, port).await;
    let (_, info) = get(&addr, "/v1/info", None).await;
    assert!(info["features"].as_array().unwrap().contains(&json!("mail")));

    // The test email.
    let (status, t) = post(&addr, "/v1/admin/mail/test", Some(&admin), json!({ "to": "alex@example.org" })).await;
    assert_eq!(status, 200, "{t}");
    {
        let got = got.lock().unwrap();
        assert_eq!(got[0].to, "alex@example.org");
        assert!(got[0].data.contains("Subject: Test email from Office server"));
        assert!(got[0].data.contains("text/html") && got[0].data.contains("text/plain"));
    }

    // Adding someone with an address: an invitation goes to them, and the
    // admin gets no code.
    let (_, p) = post(
        &addr,
        "/v1/admin/people",
        Some(&admin),
        json!({ "username": "priya", "display_name": "Priya Shah", "email": "priya@example.org" }),
    )
    .await;
    assert_eq!(p["emailed"], true, "{p}");
    assert_eq!(p["code"], "");
    let welcome = got.lock().unwrap().last().unwrap().clone();
    assert_eq!(welcome.to, "priya@example.org");
    assert!(welcome.data.contains("Subject: You're invited to Office server"), "{}", welcome.data);
    assert!(welcome.data.contains("Welcome, Priya Shah"));
    assert!(welcome.data.replace("=\r\n", "").contains("/files/?user=3Dpriya"), "{}", welcome.data);
    // Signing in, she has a code emailed and picks her password.
    let (_, step) = post(&addr, "/v1/login/start", None, json!({ "username": "priya" })).await;
    assert_eq!(step, json!({ "next": "pick_password", "can_email": true }));
    let before = got.lock().unwrap().len();
    assert_eq!(post(&addr, "/v1/forgot", None, json!({ "username": "priya" })).await.0, 204);
    let code = next_mail(&got, before).await;
    assert_eq!(code.to, "priya@example.org");
    let (status, s) = use_link(&addr, &code_in(&code.data), "priya", "priya password").await;
    assert!(status == 200 && s["token"].is_string(), "{s}");

    // Forgot password: the same answer for anyone; only real accounts get mail.
    let before = got.lock().unwrap().len();
    assert_eq!(post(&addr, "/v1/forgot", None, json!({ "username": "nobody@example.org" })).await.0, 204);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(got.lock().unwrap().len(), before);
    assert_eq!(post(&addr, "/v1/forgot", None, json!({ "username": "priya" })).await.0, 204);
    let reset = next_mail(&got, before).await;
    assert!(reset.data.contains("Subject: Your code for Office server"), "{}", reset.data);
    // The old password works until the code is used.
    assert_eq!(sign_in(&addr, "priya", "priya password").await.0, 200);
    assert_eq!(use_link(&addr, &code_in(&reset.data), "priya", "priya password 2").await.0, 200);
    assert_eq!(sign_in(&addr, "priya", "priya password 2").await.0, 200);

    // An admin's reset goes by email too.
    let id = p["id"].as_i64().unwrap();
    let (_, r) = post(&addr, &format!("/v1/admin/people/{id}/reset"), Some(&admin), json!({})).await;
    assert_eq!((r["emailed"].clone(), r["code"].clone()), (json!(true), json!("")));
    let told = got.lock().unwrap().last().unwrap().clone();
    assert!(told.data.contains("Subject: New password for Office server"));
}

#[tokio::test]
async fn a_new_account_can_have_a_code_emailed_when_signing_in() {
    let db = db_or_skip!("mail_new_account");
    let (port, got) = fake_smtp().await;
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    // Added before the server could send email: the code only went to the admin.
    let (_, p) = post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "noor", "email": "noor@example.org" })).await;
    assert_eq!(p["emailed"], false);
    mail_server(&addr, &admin, port).await;
    let (_, step) = post(&addr, "/v1/login/start", None, json!({ "username": "noor" })).await;
    assert_eq!(step, json!({ "next": "pick_password", "can_email": true }));
    // A code by email, for a new account. The admin's code still works
    // (signing in mustn't spoil it) until one of them is used.
    let before = got.lock().unwrap().len();
    assert_eq!(post(&addr, "/v1/forgot", None, json!({ "username": "noor" })).await.0, 204);
    let welcome = next_mail(&got, before).await;
    assert_eq!(welcome.to, "noor@example.org");
    assert!(welcome.data.contains("then pick your password"), "{}", welcome.data);
    let (status, _) = post(&addr, "/v1/link", None, json!({ "link": p["code"], "username": "noor" })).await;
    assert_eq!(status, 200, "the admin's code still works");
    // A second email replaces the first emailed code, not the admin's.
    let before = got.lock().unwrap().len();
    assert_eq!(post(&addr, "/v1/forgot", None, json!({ "username": "noor" })).await.0, 204);
    let again = next_mail(&got, before).await;
    assert_eq!(use_link(&addr, &code_in(&welcome.data), "noor", "noor password").await.0, 401);
    assert_eq!(use_link(&addr, &code_in(&again.data), "noor", "noor password").await.0, 200);
    assert_eq!(sign_in(&addr, "noor", "noor password").await.0, 200);
    // Used one, the others end.
    assert_eq!(use_link(&addr, p["code"].as_str().unwrap(), "noor", "other password").await.0, 401);
}

#[tokio::test]
async fn missed_messages_are_emailed_once_as_people_choose() {
    let db = db_or_skip!("mail_digest");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    add_person(&addr, &admin, "kim", false).await;
    let (port, got) = fake_smtp().await;
    mail_server(&addr, &admin, port).await;
    let c = server.pool().get().await.unwrap();
    c.execute("UPDATE users SET email = 'kim@example.org', display_name = 'Kim' WHERE username = 'kim'", &[])
        .await
        .unwrap();
    let id = |n: &'static str| {
        let c = &c;
        async move { c.query_one("SELECT id FROM users WHERE username = $1", &[&n]).await.unwrap().get::<_, i64>(0) }
    };
    let (alex, kim) = (id("alex").await, id("kim").await);
    let mut cc = server.pool().get().await.unwrap();
    let dm = xenify_chat_server::db::open_direct(&mut cc, alex, kim).await.unwrap();
    let group =
        xenify_chat_server::db::create(&mut cc, xenify_chat_proto::Kind::Group, "Launch", false, alex, &[kim], "")
            .await
            .unwrap();
    for (conv, text) in [(dm, "Lunch at noon?"), (group, "Agenda is up"), (group, "@kim can you check the budget")] {
        xenify_chat_server::db::send(&mut cc, conv, alex, "", text).await.unwrap();
    }
    // Too new: nothing yet.
    let before = got.lock().unwrap().len();
    xenify_server::chatjobs::digests(&server).await;
    assert_eq!(got.lock().unwrap().len(), before);
    c.execute("UPDATE messages SET sent_at = now() - interval '11 minutes'", &[]).await.unwrap();
    xenify_server::chatjobs::digests(&server).await;
    let all = got.lock().unwrap().clone();
    assert_eq!(all.len(), before + 1);
    let m = all.last().unwrap();
    let plain = m.data.replace("=\r\n", "");
    assert_eq!(m.to, "kim@example.org");
    assert!(plain.contains("2 messages you missed"), "{plain}");
    assert!(plain.contains("Lunch at noon?") && plain.contains("can you check the budget"));
    assert!(!plain.contains("Agenda is up"), "not mentioned, not direct");
    // Once only.
    xenify_server::chatjobs::digests(&server).await;
    assert_eq!(got.lock().unwrap().len(), before + 1);
    // Everything, unless the chat's muted; or nothing at all.
    c.execute("INSERT INTO chat_settings (user_id, value) VALUES ($1, '{\"digest\": \"all\"}')", &[&kim])
        .await
        .unwrap();
    xenify_chat_server::db::send(&mut cc, group, alex, "", "Launch moved to May").await.unwrap();
    xenify_chat_server::db::send(&mut cc, dm, alex, "", "Also, the slides").await.unwrap();
    c.execute(
        "UPDATE members SET muted_until = now() + interval '1 day' WHERE conv = $1 AND user_id = $2",
        &[&dm, &kim],
    )
    .await
    .unwrap();
    c.execute("UPDATE messages SET sent_at = now() - interval '11 minutes'", &[]).await.unwrap();
    xenify_server::chatjobs::digests(&server).await;
    let m = got.lock().unwrap().last().unwrap().clone();
    let plain = m.data.replace("=\r\n", "");
    assert!(plain.contains("Launch moved to May") && !plain.contains("Also, the slides"), "{plain}");
    c.execute("UPDATE chat_settings SET value = '{\"digest\": \"off\"}' WHERE user_id = $1", &[&kim]).await.unwrap();
    xenify_chat_server::db::send(&mut cc, group, alex, "", "@kim are you there").await.unwrap();
    c.execute("UPDATE messages SET sent_at = now() - interval '11 minutes'", &[]).await.unwrap();
    let n = got.lock().unwrap().len();
    xenify_server::chatjobs::digests(&server).await;
    assert_eq!(got.lock().unwrap().len(), n);
}

#[tokio::test]
async fn a_made_up_host_cannot_get_into_reset_emails() {
    let db = db_or_skip!("mail_forged_host");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (port, got) = fake_smtp().await;
    mail_server(&addr, &admin, port).await;
    let c = server.pool().get().await.unwrap();
    c.execute("UPDATE users SET email = 'alex@example.org' WHERE username = 'alex'", &[]).await.unwrap();
    c.execute(
        "INSERT INTO settings (key, value) VALUES ('address', 'https://files.example.org')
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[],
    )
    .await
    .unwrap();

    // "Forgot password?" for Alex, from someone naming their own site.
    let before = got.lock().unwrap().len();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = json!({ "username": "alex" }).to_string();
    let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let req = format!(
        "POST /v1/forgot HTTP/1.1\r\nHost: evil.example\r\nOrigin: https://evil.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 204"), "{out}");

    let reset = next_mail(&got, before).await;
    assert!(code_in(&reset.data).len() == 9);
    assert!(!reset.data.contains("evil.example"));
    let kept: String = c.query_one("SELECT value FROM settings WHERE key = 'address'", &[]).await.unwrap().get(0);
    assert_eq!(kept, "https://files.example.org");
}
