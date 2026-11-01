//! Two-step sign-in against a real PostgreSQL: an authenticator app,
//! backup codes, remembering a computer, and an admin requiring it.

mod common;

use common::*;
use serde_json::{Value, json};

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

/// What an authenticator app shows now for `secret` (base32), as RFC 6238
/// works it out.
fn code_now(secret: &str, ahead: i64) -> String {
    use hmac::{KeyInit, Mac};
    let mut key = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in secret.chars() {
        buf = (buf << 5) | "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".find(c).unwrap() as u32;
        bits += 5;
        if bits >= 8 {
            key.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    let step =
        (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() / 30) as i64 + ahead;
    let mut m = hmac::Hmac::<sha1::Sha1>::new_from_slice(&key).unwrap();
    m.update(&(step as u64).to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    format!("{:06}", (u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) & 0x7fff_ffff) % 1_000_000)
}

async fn login(addr: &str, user: &str, password: &str, remember: &str) -> (u16, Value) {
    post(
        addr,
        "/v1/login",
        None,
        json!({ "username": user, "password": password, "device": "test", "remember": remember }),
    )
    .await
}

#[tokio::test]
async fn an_authenticator_app_adds_a_second_step() {
    let db = db_or_skip!("mfa");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, st) = get(&addr, "/v1/me/mfa", Some(&sam)).await;
    assert_eq!((st["app"].clone(), st["required"].clone()), (json!(false), json!(false)));

    // Sam sets up the app: a secret and a QR code, then a code from it.
    let (status, app) = post(&addr, "/v1/mfa/app/start", Some(&sam), json!({})).await;
    assert_eq!(status, 200, "{app}");
    let secret = app["secret"].as_str().unwrap().to_string();
    assert!(app["uri"].as_str().unwrap().starts_with("otpauth://totp/Office%20server:sam?secret="));
    assert!(app["qr_svg"].as_str().unwrap().starts_with("<svg"));
    assert_eq!(post(&addr, "/v1/mfa/app/finish", Some(&sam), json!({ "code": "000000" })).await.0, 401);
    // Kept: the same code must be refused later (worked out again, it may
    // be the next 30 seconds' code, which is rightly accepted).
    let setup_code = code_now(&secret, 0);
    let (status, ready) = post(&addr, "/v1/mfa/app/finish", Some(&sam), json!({ "code": setup_code })).await;
    assert_eq!(status, 200, "{ready}");
    let backup: Vec<String> = serde_json::from_value(ready["backup_codes"].clone()).unwrap();
    assert_eq!(backup.len(), 10);

    // Signing in now takes a code; a used code doesn't work twice.
    let (status, step) = login(&addr, "sam", "sam password 1", "").await;
    assert_eq!(status, 200);
    assert!(step.get("token").is_none(), "{step}");
    assert_eq!(step["methods"], json!(["app", "backup"]));
    let ticket = step["ticket"].as_str().unwrap();
    let wrong = json!({ "ticket": ticket, "method": "app", "code": "123456" });
    assert_eq!(post(&addr, "/v1/login/code", None, wrong).await.0, 401);
    let used = setup_code.clone();
    let (_, step) = login(&addr, "sam", "sam password 1", "").await;
    let (status, _) =
        post(&addr, "/v1/login/code", None, json!({ "ticket": step["ticket"], "method": "app", "code": used })).await;
    assert_eq!(status, 401, "the code from setting up was used already");
    let (_, step) = login(&addr, "sam", "sam password 1", "").await;
    let (status, s) = post(
        &addr,
        "/v1/login/code",
        None,
        json!({ "ticket": step["ticket"], "method": "app", "code": code_now(&secret, 1), "remember": true }),
    )
    .await;
    assert_eq!(status, 200, "{s}");
    let remember = s["remember"].as_str().unwrap().to_string();

    // A remembered computer skips the step; another doesn't.
    let (_, s) = login(&addr, "sam", "sam password 1", &remember).await;
    assert!(s["token"].is_string());
    let (_, s) = login(&addr, "sam", "sam password 1", "made-up").await;
    assert!(s["ticket"].is_string());

    // A backup code works once.
    let (_, step) = login(&addr, "sam", "sam password 1", "").await;
    let b = json!({ "ticket": step["ticket"], "method": "backup", "code": backup[0].to_uppercase().replace('-', " ") });
    assert_eq!(post(&addr, "/v1/login/code", None, b).await.0, 200);
    let (_, step) = login(&addr, "sam", "sam password 1", "").await;
    let b = json!({ "ticket": step["ticket"], "method": "backup", "code": backup[0] });
    assert_eq!(post(&addr, "/v1/login/code", None, b).await.0, 401);

    // Too many wrong codes end the ticket.
    let (_, step) = login(&addr, "sam", "sam password 1", "").await;
    for _ in 0..5 {
        post(&addr, "/v1/login/code", None, json!({ "ticket": step["ticket"], "method": "app", "code": "111111" }))
            .await;
    }
    let (status, e) = post(
        &addr,
        "/v1/login/code",
        None,
        json!({ "ticket": step["ticket"], "method": "app", "code": code_now(&secret, 0) }),
    )
    .await;
    assert_eq!((status, e["code"].clone()), (401, json!("bad_ticket")));

    // Turning it off needs the password; an admin can clear it too.
    assert_eq!(post(&addr, "/v1/me/mfa/off", Some(&sam), json!({ "password": "nope" })).await.0, 401);
    let (_, people) = get(&addr, "/v1/admin/people", Some(&admin)).await;
    let id = people.as_array().unwrap().iter().find(|p| p["username"] == "sam").unwrap()["id"].as_i64().unwrap();
    assert_eq!(call(&addr, "DELETE", &format!("/v1/admin/people/{id}/mfa"), Some(&admin), None).await.0, 204);
    let (_, s) = login(&addr, "sam", "sam password 1", "").await;
    assert!(s["token"].is_string());
}

#[tokio::test]
async fn an_admin_can_require_a_second_step() {
    let db = db_or_skip!("mfa_required");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&sam), json!({ "mfa_required": "admins" })).await.0, 403);
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&admin), json!({ "mfa_required": "admins" })).await.0, 204);

    // Sam isn't an admin: signs in as before. Alex must set it up first.
    assert!(login(&addr, "sam", "sam password 1", "").await.1["token"].is_string());
    let (_, step) = login(&addr, "alex", "alex password 1", "").await;
    assert_eq!((step["setup"].clone(), step["setup_methods"].clone()), (json!(true), json!(["app"])));
    let ticket = step["ticket"].as_str().unwrap();
    // The ticket only sets up: it isn't a session.
    assert_eq!(get(&addr, "/v1/me", Some(ticket)).await.0, 401);
    let (_, app) = post(&addr, "/v1/mfa/app/start", None, json!({ "ticket": ticket })).await;
    let secret = app["secret"].as_str().unwrap();
    let (status, ready) =
        post(&addr, "/v1/mfa/app/finish", None, json!({ "ticket": ticket, "code": code_now(secret, 0) })).await;
    assert_eq!(status, 200, "{ready}");
    let token = ready["session"]["token"].as_str().unwrap();
    assert_eq!(get(&addr, "/v1/me", Some(token)).await.0, 200);
    // Required: it can't be turned off.
    assert_eq!(post(&addr, "/v1/me/mfa/off", Some(token), json!({ "password": "alex password 1" })).await.0, 403);

    // Everyone: Sam sets it up at the next sign-in too.
    post(&addr, "/v1/admin/settings", Some(&admin), json!({ "mfa_required": "everyone" })).await;
    assert_eq!(login(&addr, "sam", "sam password 1", "").await.1["setup"], true);
    // The command-line reset clears it, for a lost phone.
    let code = xenify_server::reset_password_code(&db, "alex").await.unwrap();
    let (_, after) = use_link(&addr, &code, "alex", "alex password 2").await;
    assert_eq!(after["setup"], true, "required, so set up again: {after}");
}

#[tokio::test]
async fn the_password_alone_cannot_replace_a_second_step() {
    let db = db_or_skip!("mfa_replace");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, app) = post(&addr, "/v1/mfa/app/start", Some(&sam), json!({})).await;
    let secret = app["secret"].as_str().unwrap().to_string();
    assert_eq!(post(&addr, "/v1/mfa/app/finish", Some(&sam), json!({ "code": code_now(&secret, 0) })).await.0, 200);

    // Someone with only Sam's password gets a ticket, but it can't set up
    // an authenticator of their own and sign in with that.
    let (_, step) = login(&addr, "sam", "sam password 1", "").await;
    let ticket = step["ticket"].as_str().unwrap();
    let (status, e) = post(&addr, "/v1/mfa/app/start", None, json!({ "ticket": ticket })).await;
    assert_eq!((status, e["code"].clone()), (403, json!("set_up")));
    let (status, _) = post(&addr, "/v1/mfa/app/finish", None, json!({ "ticket": ticket, "code": "123456" })).await;
    assert_eq!(status, 403);
    let (status, _) = post(&addr, "/v1/mfa/email", None, json!({ "ticket": ticket })).await;
    assert_eq!(status, 403);
    let c = server.pool().get().await.unwrap();
    let kept: Option<String> =
        c.query_one("SELECT mfa_secret FROM users WHERE username = 'sam'", &[]).await.unwrap().get(0);
    assert_eq!(kept.as_deref(), Some(secret.as_str()));

    // Wrong codes add up across tickets: a known password doesn't buy
    // endless guesses.
    let mut last = 0;
    for _ in 0..12 {
        let (_, step) = login(&addr, "sam", "sam password 1", "").await;
        if step["ticket"].is_null() {
            last = 429;
            break;
        }
        last = post(
            &addr,
            "/v1/login/code",
            None,
            json!({ "ticket": step["ticket"], "method": "backup", "code": "zzzz-zzzz" }),
        )
        .await
        .0;
    }
    assert_eq!(last, 429);
}
