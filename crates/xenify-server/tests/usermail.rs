//! People's email kept by the server (Chat's Mail): the company's mail
//! server connected once by an admin, an account added with a password,
//! reading, sending, drafts, and what's kept sealed.

mod common;

#[allow(dead_code)]
#[path = "../../xenify-mail-core/tests/common/imap_server.rs"]
mod imap_server;
#[allow(dead_code)]
#[path = "../../xenify-mail-core/tests/common/smtp_server.rs"]
mod smtp_server;

use common::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

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

/// Adds someone with an email address and signs them in.
async fn person(addr: &str, admin: &str, user: &str, email: &str) -> String {
    let (s, made) = post(addr, "/v1/admin/people", Some(admin), json!({ "username": user, "email": email })).await;
    assert_eq!(s, 200, "{made}");
    let (s, session) = use_link(addr, made["code"].as_str().unwrap(), email, &format!("{user} password 1")).await;
    assert_eq!(s, 200, "{session}");
    session["token"].as_str().unwrap().to_string()
}

async fn mail(addr: &str, token: &str, call: &str, args: Value) -> (u16, Value) {
    post(addr, &format!("/v1/mail/{call}"), Some(token), args).await
}

/// Calls `call` until `ok` holds for its answer.
async fn until(addr: &str, token: &str, call: &str, args: Value, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let (s, v) = mail(addr, token, call, args.clone()).await;
        if s == 200 && ok(&v) {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}: {s} {v}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn company_mail_and_own_accounts() {
    let db = db_or_skip!("usermail");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, info) = get(&addr, "/v1/info", None).await;
    assert!(info["features"].as_array().unwrap().contains(&json!("email")));

    // The company's mail server, with one master login ("{email}" as the
    // sign-in name here, since the test server knows one person).
    let (iport, ist) = imap_server::start("sam@example.org", "master-pw");
    ist.lock().unwrap().add(
        "INBOX",
        &imap_server::message("Ana Ruiz <ana@example.com>", "Quarter numbers", "Up 12%."),
        &[],
    );
    let (sport, sent) = smtp_server::start("sam@example.org", "master-pw");
    let sam = person(&addr, &admin, "sam", "sam@example.org").await;
    let kim = person(&addr, &admin, "kim", "kim@elsewhere.net").await;

    // Nobody but admins sees or changes Admin › Email.
    assert_eq!(get(&addr, "/v1/admin/usermail", Some(&sam)).await.0, 403);
    let local = |p: u16| json!({ "host": "127.0.0.1", "port": p, "security": "none" });
    let set = json!({
        "org": "imap", "domains": "example.org", "imap": local(iport), "smtp": local(sport),
        "master_user": "office", "master_password": "master-pw", "login_as": "{email}", "private_hosts": true,
    });
    let (s, v) = post(&addr, "/v1/admin/usermail", Some(&admin), set.clone()).await;
    assert_eq!(s, 204, "{v}");
    let (_, back) = get(&addr, "/v1/admin/usermail", Some(&admin)).await;
    assert_eq!((back["master_password"].clone(), back["has_master_password"].clone()), (json!(""), json!(true)));
    assert!(back["redirect"].as_str().unwrap().ends_with("/v1/mail/connected"));
    // Saving again without the password keeps it.
    let mut again = set_obj(&set);
    again.insert("master_password".into(), json!(""));
    assert_eq!(post(&addr, "/v1/admin/usermail", Some(&admin), Value::Object(again)).await.0, 204);
    let (s, v) = post(&addr, "/v1/admin/usermail/test", Some(&admin), json!({ "email": "sam@example.org" })).await;
    assert_eq!(s, 204, "{v}");
    let (s, _) = post(&addr, "/v1/admin/usermail/test", Some(&admin), json!({ "email": "kim@elsewhere.net" })).await;
    assert_eq!(s, 400, "not at the company's domain");

    // Sam has the company's mailbox, with nothing to set up; Kim (another domain) hasn't.
    let accounts = until(&addr, &sam, "mail_accounts", json!({}), "the company mailbox", |v| {
        v.as_array().is_some_and(|l| l.len() == 1)
    })
    .await;
    assert_eq!(
        (accounts[0]["email"].clone(), accounts[0]["kind"].clone()),
        (json!("sam@example.org"), json!("company"))
    );
    let (_, none) = mail(&addr, &kim, "mail_accounts", json!({})).await;
    assert_eq!(none, json!([]));
    let (s, v) = mail(&addr, &sam, "mail_remove", json!({ "account": "sam@example.org" })).await;
    assert_eq!(s, 400, "the company's mailbox can't be removed: {v}");

    let list = until(&addr, &sam, "mail_list", json!({ "account": "", "folder": "", "limit": 50 }), "the inbox", |v| {
        v.as_array().is_some_and(|l| l.len() == 1)
    })
    .await;
    let r = &list[0];
    assert_eq!(r["subject"], "Quarter numbers");
    let (folder, uid) = (r["folder"].as_str().unwrap().to_string(), r["uid"].as_u64().unwrap());
    let args = json!({ "account": "sam@example.org", "folder": folder, "uid": uid });
    let opened = until(&addr, &sam, "mail_open", args.clone(), "the opened email", |v| !v.is_null()).await;
    assert!(opened["text"].as_str().unwrap().contains("Up 12%"));

    // Sending goes through the company's server, from Sam.
    let (s, v) = mail(&addr, &sam, "mail_send", json!({ "account": "sam@example.org", "to": "Ana Ruiz <ana@example.com>", "cc": "", "subject": "Re: Quarter numbers", "text": "Great news.", "reply_to": [folder, uid], "attachments": [] })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["people"], json!([["Ana Ruiz", "ana@example.com"]]));
    assert_eq!(sent.lock().unwrap().len(), 1);

    // Drafts are kept on the server, per person.
    let (s, id) = mail(
        &addr,
        &sam,
        "mail_draft_save",
        json!({ "draft": { "account": "sam@example.org", "to": "bo@example.com", "subject": "Later", "text": "…" } }),
    )
    .await;
    assert_eq!(s, 200, "{id}");
    let (_, drafts) = mail(&addr, &sam, "mail_drafts", json!({})).await;
    assert_eq!(drafts[0]["subject"], "Later");
    assert_eq!(mail(&addr, &kim, "mail_drafts", json!({})).await.1, json!([]));
    mail(&addr, &sam, "mail_draft_delete", json!({ "id": id })).await;
    assert_eq!(mail(&addr, &sam, "mail_drafts", json!({})).await.1, json!([]));

    // Kim adds an account with a password (another test mail server); the
    // password is kept sealed, not as typed.
    let (kport, _kst) = imap_server::start("kim@elsewhere.net", "kim-mail-pw");
    let (ksport, _) = smtp_server::start("kim@elsewhere.net", "kim-mail-pw");
    let (s, opts) = mail(&addr, &kim, "mail_options", json!({})).await;
    assert_eq!((s, opts["passwords"].clone(), opts["microsoft"].clone()), (200, json!(true), json!(false)));
    let add = json!({ "email": "kim@elsewhere.net", "name": "Kim", "user": "kim@elsewhere.net", "password": "wrong", "imap": local(kport), "smtp": local(ksport) });
    assert_eq!(mail(&addr, &kim, "mail_add", add.clone()).await.0, 400, "a wrong password is refused");
    let mut add = add;
    add["password"] = json!("kim-mail-pw");
    let (s, v) = mail(&addr, &kim, "mail_add", add.clone()).await;
    assert_eq!(s, 200, "{v}");
    let (c, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    let kept: String =
        c.query_one("SELECT secret FROM mail_accounts WHERE email = 'kim@elsewhere.net'", &[]).await.unwrap().get(0);
    assert!(!kept.contains("kim-mail-pw") && kept.len() > 30, "sealed: {kept}");
    let setting: String = c.query_one("SELECT value FROM settings WHERE key = 'user_mail'", &[]).await.unwrap().get(0);
    assert!(!setting.contains("master-pw"), "the master password is sealed too");
    until(&addr, &kim, "mail_accounts", json!({}), "Kim's account", |v| {
        v.as_array().is_some_and(|l| l.len() == 1 && l[0]["kind"] == "password")
    })
    .await;
    assert_eq!(mail(&addr, &kim, "mail_remove", json!({ "account": "kim@elsewhere.net" })).await.0, 200);
    until(&addr, &kim, "mail_accounts", json!({}), "Kim's account gone", |v| v == &json!([])).await;

    // The admin can turn passwords off.
    let mut off = set_obj(&set);
    off.insert("no_passwords".into(), json!(true));
    assert_eq!(post(&addr, "/v1/admin/usermail", Some(&admin), Value::Object(off)).await.0, 204);
    assert_eq!(mail(&addr, &kim, "mail_add", add).await.0, 400);

    // Connecting one's own Microsoft or Google mail needs the admin's app first.
    let (s, _) = mail(&addr, &kim, "mail_connect", json!({ "provider": "google" })).await;
    assert_eq!(s, 400);
    let mut g = set_obj(&set);
    g.insert("personal_google".into(), json!(true));
    g.insert("pg_client_id".into(), json!("client-123.apps.googleusercontent.com"));
    g.insert("pg_secret".into(), json!("g-secret"));
    assert_eq!(post(&addr, "/v1/admin/usermail", Some(&admin), Value::Object(g)).await.0, 204);
    let (s, v) = mail(&addr, &kim, "mail_connect", json!({ "provider": "google" })).await;
    assert_eq!(s, 200, "{v}");
    let url = v["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?")
            && url.contains("code_challenge_method=S256")
            && url.contains("client_id=client-123"),
        "{url}"
    );
    assert!(url.contains("redirect_uri=http") && url.contains("%2Fv1%2Fmail%2Fconnected"), "{url}");
    // Coming back with a state the server didn't make: a plain page saying so.
    let (s, page) = call(&addr, "GET", "/v1/mail/connected?code=x&state=forged", None, None).await;
    assert_eq!(s, 200);
    assert!(page.to_string().contains("run out") || page.is_null(), "{page}");
}

fn set_obj(v: &Value) -> serde_json::Map<String, Value> {
    v.as_object().unwrap().clone()
}
