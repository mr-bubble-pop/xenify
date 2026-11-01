//! Accounts against a real PostgreSQL (XENIFY_TEST_PG, default
//! postgres://postgres@127.0.0.1:55432/postgres). Skipped without one.

mod common;

use common::*;
use serde_json::json;

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

#[tokio::test]
async fn first_admin_sets_up_the_server_and_signs_in() {
    let db = db_or_skip!("setup");
    let (addr, server) = serve(&db).await;
    let (_, info) = get(&addr, "/v1/info", None).await;
    assert_eq!(info["setup_needed"], true);
    assert_eq!(info["protocol"], 2);

    // A wrong token is refused and the real one keeps working.
    let (status, _) = post(
        &addr,
        "/v1/setup",
        None,
        json!({ "token": "nope", "server_name": "x", "username": "eve", "password": "eve password" }),
    )
    .await;
    assert_eq!(status, 403);
    // Too short a password is refused too.
    let (status, e) = post(
        &addr,
        "/v1/setup",
        None,
        json!({ "token": server.setup_token().unwrap(), "server_name": "x", "username": "eve", "password": "short" }),
    )
    .await;
    assert_eq!((status, e["code"].clone()), (400, json!("short_password")));
    let token = set_up(&addr, &server).await;

    let (_, info) = get(&addr, "/v1/info", None).await;
    assert_eq!(info["setup_needed"], false);
    assert_eq!(info["name"], "Office server");
    assert!(info.get("encrypted").is_none());
    assert!(server.setup_token().is_none());

    let (_, me) = get(&addr, "/v1/me", Some(&token)).await;
    assert_eq!(
        (me["username"].clone(), me["admin"].clone(), me["email"].clone()),
        (json!("alex"), json!(true), json!("alex@example.org"))
    );

    // Signing in again: the wrong password fails, the right one works, by
    // user name (any case) or email address.
    assert_eq!(sign_in(&addr, "alex", "not my password").await.0, 401);
    assert_eq!(sign_in(&addr, "nobody", "alex password 1").await.0, 401);
    let (status, session) = sign_in(&addr, "Alex", "alex password 1").await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["account"]["username"], "alex");
    assert_eq!(sign_in(&addr, "Alex@Example.org", "alex password 1").await.0, 200);
}

#[tokio::test]
async fn admins_add_people_who_pick_a_password_with_their_welcome_link() {
    let db = db_or_skip!("people");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;

    let (status, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design" })).await;
    assert_eq!(status, 200);
    let design = g["id"].as_i64().unwrap();

    let (status, p) = post(
        &addr,
        "/v1/admin/people",
        Some(&admin),
        json!({ "username": "priya", "display_name": "Priya Shah", "email": "priya@example.org", "groups": [design] }),
    )
    .await;
    assert_eq!(status, 200, "{p}");
    // No mail server: a code for the admin to pass on.
    let code = p["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 14, "{code}");
    let (_, people) = get(&addr, "/v1/admin/people", Some(&admin)).await;
    let priya = people.as_array().unwrap().iter().find(|p| p["username"] == "priya").unwrap().clone();
    assert_eq!(priya["pending"], true);
    assert_eq!(priya["groups"], json!([design]));

    // Priya can't sign in yet. The code is checked with her user name; a
    // made-up one, or hers with someone else's name, fails.
    assert_eq!(sign_in(&addr, "priya", "anything at all").await.0, 401);
    let (status, _) = post(&addr, "/v1/link", None, json!({ "link": "ABCD-EFGH-JKLM", "username": "priya" })).await;
    assert_eq!(status, 401);
    let (status, _) = post(&addr, "/v1/link", None, json!({ "link": code, "username": "alex" })).await;
    assert_eq!(status, 401);
    let typed = code.to_lowercase().replace('-', " ");
    let (status, info) = post(&addr, "/v1/link", None, json!({ "link": typed, "username": "Priya" })).await;
    assert_eq!(status, 200, "{info}");
    assert_eq!(
        (info["kind"].clone(), info["username"].clone(), info["email"].clone()),
        (json!("welcome"), json!("priya"), json!("priya@example.org"))
    );
    assert_eq!(info["server_name"], "Office server");

    let (status, s) = use_link(&addr, &code, "priya", "priya password").await;
    assert_eq!(status, 200, "{s}");
    assert_eq!(s["account"]["admin"], false);

    // The code is used up; the password works.
    assert_eq!(use_link(&addr, &code, "priya", "other password").await.0, 401);
    assert_eq!(sign_in(&addr, "priya", "priya password").await.0, 200);
    let (_, people) = get(&addr, "/v1/admin/people", Some(&admin)).await;
    assert!(people.as_array().unwrap().iter().all(|p| p["pending"] == false));

    // There are no starting passwords.
    let (status, p) =
        post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "sam", "password": "sam start 1" })).await;
    assert_eq!(status, 200, "{p}");
    assert_eq!(sign_in(&addr, "sam", "sam start 1").await.0, 401);

    // Only admins add people.
    let sam = add_person(&addr, &admin, "sam2", false).await;
    assert_eq!(post(&addr, "/v1/admin/people", Some(&sam), json!({ "username": "kim" })).await.0, 403);
}

#[tokio::test]
async fn email_addresses_are_checked_and_never_shared() {
    let db = db_or_skip!("email");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (status, e) =
        post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "kim", "email": "not an address" })).await;
    assert_eq!((status, e["code"].clone()), (400, json!("bad_email")));
    let (status, _) =
        post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "kim", "email": "ALEX@example.org" })).await;
    assert_eq!(status, 409, "someone else has it");
    let (status, p) = post(
        &addr,
        "/v1/admin/people",
        Some(&admin),
        json!({ "username": "kim", "email": "kim@example.org" }),
    )
    .await;
    assert_eq!(status, 200);
    let kim = p["id"].as_i64().unwrap();
    assert_eq!(use_link(&addr, p["code"].as_str().unwrap(), "kim@example.org", "kim password").await.0, 200);
    let (status, _) =
        post(&addr, &format!("/v1/admin/people/{kim}"), Some(&admin), json!({ "email": "alex@example.org" })).await;
    assert_eq!(status, 409);
    assert_eq!(sign_in(&addr, "kim@example.org", "kim password").await.0, 200);
}

#[tokio::test]
async fn admins_reset_forgotten_passwords_with_a_code() {
    let db = db_or_skip!("reset");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, people) = get(&addr, "/v1/admin/people", Some(&admin)).await;
    let id = people.as_array().unwrap().iter().find(|p| p["username"] == "sam").unwrap()["id"].as_i64().unwrap();

    let (status, made) = post(&addr, &format!("/v1/admin/people/{id}/reset"), Some(&admin), json!({})).await;
    assert_eq!(status, 200, "{made}");
    // Signed out everywhere, and the old password stops working.
    assert_eq!(get(&addr, "/v1/me", Some(&sam)).await.0, 401);
    assert_eq!(sign_in(&addr, "sam", "sam password 1").await.0, 401);
    let code = made["code"].as_str().unwrap();
    // Signing in, Sam is asked to pick one.
    let (_, step) = post(&addr, "/v1/login/start", None, json!({ "username": "sam" })).await;
    assert_eq!(step["next"], "pick_password");
    let (_, info) = post(&addr, "/v1/link", None, json!({ "link": code, "username": "sam" })).await;
    assert_eq!(info["kind"], "reset");
    assert_eq!(use_link(&addr, code, "sam", "sam password 2").await.0, 200);
    assert_eq!(sign_in(&addr, "sam", "sam password 2").await.0, 200);

    // A newer reset code replaces an older one.
    let (_, first) = post(&addr, &format!("/v1/admin/people/{id}/reset"), Some(&admin), json!({})).await;
    let (_, second) = post(&addr, &format!("/v1/admin/people/{id}/reset"), Some(&admin), json!({})).await;
    assert_eq!(use_link(&addr, first["code"].as_str().unwrap(), "sam", "sam password 3").await.0, 401);
    assert_eq!(use_link(&addr, second["code"].as_str().unwrap(), "sam", "sam password 3").await.0, 200);
}

#[tokio::test]
async fn the_command_line_reset_lets_a_locked_out_admin_back_in() {
    let db = db_or_skip!("cli_reset");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let code = xenify_server::reset_password_code(&db, "alex@example.org").await.unwrap();
    assert_eq!(get(&addr, "/v1/me", Some(&admin)).await.0, 401, "signed out everywhere");
    assert_eq!(use_link(&addr, &code, "alex", "alex password 9").await.0, 200);
    assert_eq!(sign_in(&addr, "alex", "alex password 9").await.0, 200);
    assert!(xenify_server::reset_password_code(&db, "nobody").await.is_err());
}

#[tokio::test]
async fn signing_in_starts_with_the_user_name_and_new_accounts_pick_a_password() {
    let db = db_or_skip!("login_start");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let start = |user: &str| post(&addr, "/v1/login/start", None, json!({ "username": user }));
    // Accounts with a password, and names nobody has, look the same.
    assert_eq!(start("alex").await.1, json!({ "next": "password", "can_email": false }));
    assert_eq!(start("ghost").await.1, json!({ "next": "password", "can_email": false }));

    // An admin adds Noor: she picks a password with the code.
    let (_, made) =
        post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "noor", "email": "noor@example.org" })).await;
    let code = made["code"].as_str().unwrap().to_string();
    assert_eq!(start("NOOR").await.1["next"], "pick_password");
    assert_eq!(start("noor@example.org").await.1["next"], "pick_password");
    assert_eq!(start("noor").await.1["can_email"], false, "no mail server yet");
    let (_, info) = post(&addr, "/v1/link", None, json!({ "link": code, "username": "noor@example.org" })).await;
    assert_eq!(info["kind"], "welcome");
    assert_eq!(info["username"], "noor");
    // Setting it signs her in straight away.
    let (status, s) = use_link(&addr, &code, "noor", "noor password").await;
    assert!(s["token"].is_string(), "{s}");
    assert_eq!(status, 200, "{s}");
    assert_eq!(start("noor").await.1["next"], "password");
    assert_eq!(sign_in(&addr, "noor", "noor password").await.0, 200);

    // Nobody joins by themselves any more.
    assert!(post(&addr, "/v1/admin/invites", Some(&admin), json!({ "days": 7 })).await.0 >= 400);
    assert!(post(&addr, "/v1/invites", Some(&admin), json!({ "emails": ["x@example.org"] })).await.0 >= 400);
}

#[tokio::test]
async fn guessing_passwords_gets_slowed_down() {
    let db = db_or_skip!("limit");
    let (addr, server) = serve(&db).await;
    set_up(&addr, &server).await;
    for _ in 0..10 {
        assert_eq!(sign_in(&addr, "alex", "wrong guess").await.0, 401);
    }
    assert_eq!(sign_in(&addr, "alex", "alex password 1").await.0, 429);
}

#[tokio::test]
async fn the_last_admin_stays_and_turned_off_people_are_signed_out() {
    let db = db_or_skip!("admins");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    let alex = me["id"].as_i64().unwrap();
    let (status, _) = post(&addr, &format!("/v1/admin/people/{alex}"), Some(&admin), json!({ "admin": false })).await;
    assert_eq!(status, 409);
    let (status, _) = post(&addr, &format!("/v1/admin/people/{alex}"), Some(&admin), json!({ "disabled": true })).await;
    assert_eq!(status, 409);

    let robin_token = add_person(&addr, &admin, "robin", false).await;
    let (_, people) = get(&addr, "/v1/admin/people", Some(&admin)).await;
    let robin = people.as_array().unwrap().iter().find(|p| p["username"] == "robin").unwrap()["id"].as_i64().unwrap();
    let (status, _) =
        post(&addr, &format!("/v1/admin/people/{robin}"), Some(&admin), json!({ "disabled": true })).await;
    assert_eq!(status, 204);
    assert_eq!(get(&addr, "/v1/me", Some(&robin_token)).await.0, 401);
    assert_eq!(sign_in(&addr, "robin", "robin password 1").await.0, 401);

    // With a second admin, the first may step down.
    post(&addr, &format!("/v1/admin/people/{robin}"), Some(&admin), json!({ "disabled": false, "admin": true })).await;
    let (status, _) = post(&addr, &format!("/v1/admin/people/{alex}"), Some(&admin), json!({ "admin": false })).await;
    assert_eq!(status, 204);
}

#[tokio::test]
async fn changing_your_password_signs_out_your_other_devices() {
    let db = db_or_skip!("password");
    let (addr, server) = serve(&db).await;
    let first = set_up(&addr, &server).await;
    let (_, second) = sign_in(&addr, "alex", "alex password 1").await;
    let second = second["token"].as_str().unwrap();
    let (status, _) =
        post(&addr, "/v1/password", Some(second), json!({ "old_password": "wrong", "password": "alex password 2" }))
            .await;
    assert_eq!(status, 401);
    let (status, _) = post(
        &addr,
        "/v1/password",
        Some(second),
        json!({ "old_password": "alex password 1", "password": "alex password 2" }),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(get(&addr, "/v1/me", Some(second)).await.0, 200, "this device stays signed in");
    assert_eq!(get(&addr, "/v1/me", Some(&first)).await.0, 401);
    assert_eq!(sign_in(&addr, "alex", "alex password 2").await.0, 200);
}

/// People are their email address: the first admin and everyone added
/// give only a name and an address (the user name underneath is made from
/// it), and sign in, pick a password and are found by it.
#[tokio::test]
async fn people_sign_in_with_their_email_address() {
    let db = db_or_skip!("email_only");
    let (addr, server) = serve(&db).await;
    let body = json!({
        "token": server.setup_token().unwrap(), "server_name": "Office server", "display_name": "Alex Morgan",
        "email": "Alex.Morgan@example.org", "password": "alex password 1", "device": "test",
    });
    // Without an address there's nothing to sign in with.
    let mut none = body.clone();
    none["email"] = json!("");
    let (status, _) = post(&addr, "/v1/setup", None, none).await;
    assert_eq!(status, 400);
    let (status, alex) = post(&addr, "/v1/setup", None, body).await;
    assert_eq!(status, 200, "{alex}");
    assert_eq!(alex["account"]["email"], "alex.morgan@example.org");
    let admin = alex["token"].as_str().unwrap().to_string();
    let (status, _) = sign_in(&addr, "alex.morgan@example.org", "alex password 1").await;
    assert_eq!(status, 200);

    // Two people whose addresses start the same both get in.
    let mut ids = Vec::new();
    for email in ["priya@example.org", "priya@other.example"] {
        let (status, made) =
            post(&addr, "/v1/admin/people", Some(&admin), json!({ "email": email, "display_name": "Priya" })).await;
        assert_eq!(status, 200, "{made}");
        ids.push(made["id"].as_i64().unwrap());
        let (status, session) = use_link(&addr, made["code"].as_str().unwrap(), email, "priya password 1").await;
        assert_eq!(status, 200, "{session}");
        assert_eq!(session["account"]["email"], email);
        let (status, _) = sign_in(&addr, email, "priya password 1").await;
        assert_eq!(status, 200);
    }
    // No address, no account; and nobody loses theirs.
    let (status, _) = post(&addr, "/v1/admin/people", Some(&admin), json!({ "display_name": "Nobody" })).await;
    assert_eq!(status, 400);
    let (status, _) = post(&addr, &format!("/v1/admin/people/{}", ids[0]), Some(&admin), json!({ "email": "" })).await;
    assert_eq!(status, 400);
    // Everyone's address is in the directory, for picking people.
    let (_, dir) = get(&addr, "/v1/directory", Some(&admin)).await;
    assert!(dir["people"].as_array().unwrap().iter().any(|p| p["email"] == "priya@other.example"));
}
