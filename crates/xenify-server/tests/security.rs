//! Sign-in and network safety against a real PostgreSQL: Chat's keys
//! aren't sign-ins, sign-ins run out, a new second step needs the
//! password, links' address isn't learned from requests, trusted proxies,
//! and the headers the apps and the API are served with.

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

/// One request as written; returns (status, head, body).
async fn raw(addr: &str, request: &str) -> (u16, String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    (status, head.to_lowercase(), body.to_string())
}

#[tokio::test]
async fn chats_keys_are_not_sign_ins_and_sign_ins_run_out() {
    let db = db_or_skip!("sec_keys");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let c = server.pool().get().await.unwrap();
    let sam_id: i64 = c.query_one("SELECT id FROM users WHERE username = 'sam'", &[]).await.unwrap().get(0);
    let hash = |t: &str| {
        use sha2::{Digest, Sha256};
        xenify_sync::b64(&Sha256::digest(t.as_bytes()))
    };
    for (token, device) in [("app-key", "app"), ("api-key", "api")] {
        c.execute("INSERT INTO sessions (token_hash, user_id, device) VALUES ($1, $2, $3)", &[&hash(token), &sam_id, &device])
            .await
            .unwrap();
        assert_eq!(get(&addr, "/v1/me", Some(token)).await.0, 401, "{device}");
        assert_eq!(get(&addr, "/v1/spaces", Some(token)).await.0, 401, "{device}");
    }
    // An app's own account can't use the server either.
    let bot: i64 = c
        .query_one("INSERT INTO users (username, password_hash, app) VALUES ('bot', '', TRUE) RETURNING id", &[])
        .await
        .unwrap()
        .get(0);
    c.execute("INSERT INTO sessions (token_hash, user_id, device) VALUES ($1, $2, 'test')", &[&hash("bot-key"), &bot])
        .await
        .unwrap();
    assert_eq!(get(&addr, "/v1/me", Some("bot-key")).await.0, 401);
    // A sign-in calling itself "app" is still a sign-in.
    let (status, s) =
        post(&addr, "/v1/login", None, json!({ "username": "sam", "password": "sam password 1", "device": "app" })).await;
    assert_eq!(status, 200, "{s}");
    assert_eq!(get(&addr, "/v1/me", s["token"].as_str()).await.0, 200);

    // Sam's sign-in works until it's a year old, however much it's used.
    assert_eq!(get(&addr, "/v1/me", Some(&sam)).await.0, 200);
    c.execute("UPDATE sessions SET created = now() - interval '400 days' WHERE token_hash = $1", &[&hash(&sam)])
        .await
        .unwrap();
    assert_eq!(get(&addr, "/v1/me", Some(&sam)).await.0, 401);
    // Cleaning up removes it, and keeps the app's key.
    xenify_server::db::remove_old_sessions(&c).await.unwrap();
    let left: i64 =
        c.query_one("SELECT count(*) FROM sessions WHERE token_hash = $1", &[&hash(&sam)]).await.unwrap().get(0);
    assert_eq!(left, 0);
    let kept: i64 =
        c.query_one("SELECT count(*) FROM sessions WHERE token_hash = $1", &[&hash("app-key")]).await.unwrap().get(0);
    assert_eq!(kept, 1);
}

#[tokio::test]
async fn a_second_step_is_replaced_only_with_the_password_and_signing_out_forgets_computers() {
    let db = db_or_skip!("sec_mfa");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    // Setting one up the first time needs nothing more.
    let (status, app) = post(&addr, "/v1/mfa/app/start", Some(&sam), json!({})).await;
    assert_eq!(status, 200, "{app}");
    let c = server.pool().get().await.unwrap();
    c.execute("UPDATE users SET mfa_secret = 'JBSWY3DPEHPK3PXP' WHERE username = 'sam'", &[]).await.unwrap();
    // Replacing it does: a computer left signed in isn't enough.
    let (status, e) = post(&addr, "/v1/mfa/app/start", Some(&sam), json!({})).await;
    assert_eq!((status, e["code"].clone()), (403, json!("password_needed")));
    assert_eq!(post(&addr, "/v1/mfa/app/start", Some(&sam), json!({ "password": "wrong one!" })).await.0, 401);
    assert_eq!(post(&addr, "/v1/mfa/app/start", Some(&sam), json!({ "current": "000000" })).await.0, 401);
    let (status, _) = post(&addr, "/v1/mfa/app/start", Some(&sam), json!({ "password": "sam password 1" })).await;
    assert_eq!(status, 200);

    // "Remember this computer" ends with signing out everywhere.
    let sam_id: i64 = c.query_one("SELECT id FROM users WHERE username = 'sam'", &[]).await.unwrap().get(0);
    c.execute(
        "INSERT INTO mfa_remember (hash, user_id, expires) VALUES ('x', $1, now() + interval '30 days')",
        &[&sam_id],
    )
    .await
    .unwrap();
    assert_eq!(post(&addr, "/v1/signout-everywhere", Some(&sam), json!({})).await.0, 204);
    let left: i64 =
        c.query_one("SELECT count(*) FROM mfa_remember WHERE user_id = $1", &[&sam_id]).await.unwrap().get(0);
    assert_eq!(left, 0);
}

#[tokio::test]
async fn links_address_is_set_by_full_admins_not_learned() {
    let db = db_or_skip!("sec_address");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    // An admin's request naming another host and origin isn't remembered.
    let req = format!(
        "GET /v1/admin/people HTTP/1.1\r\nHost: evil.example\r\nOrigin: http://evil.example\r\nAuthorization: Bearer {admin}\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(raw(&addr, &req).await.0, 200);
    let c = server.pool().get().await.unwrap();
    let saved = xenify_server::db::setting(&c, "address").await.unwrap();
    assert!(saved.as_deref().is_none_or(str::is_empty), "{saved:?}");

    // Only a full admin sets it, and only a plain address.
    let (status, r) = get(&addr, "/v1/admin/network/reach", Some(&admin)).await;
    assert_eq!(status, 200, "{r}");
    let body = json!({ "address": "https://files.example.org/", "trusted_proxies": "" });
    assert_eq!(post(&addr, "/v1/admin/network/reach", Some(&admin), body.clone()).await.0, 204);
    let (_, r) = get(&addr, "/v1/admin/network/reach", Some(&admin)).await;
    assert_eq!(r["address"], "https://files.example.org");
    let bad = json!({ "address": "https://x.org/a?b", "trusted_proxies": "" });
    assert_eq!(post(&addr, "/v1/admin/network/reach", Some(&admin), bad).await.0, 400);
    let bad = json!({ "address": "", "trusted_proxies": "proxy.lan" });
    assert_eq!(post(&addr, "/v1/admin/network/reach", Some(&admin), bad).await.0, 400);
    let sam = add_person(&addr, &admin, "sam", false).await;
    assert_eq!(post(&addr, "/v1/admin/network/reach", Some(&sam), body).await.0, 403);
}

#[tokio::test]
async fn behind_a_trusted_proxy_the_real_address_counts() {
    let db = db_or_skip!("sec_proxy");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let c = server.pool().get().await.unwrap();
    xenify_server::db::set_setting(&c, "admin_access", "lan").await.unwrap();
    let from = |xff: &str| {
        format!(
            "GET /v1/admin/people HTTP/1.1\r\nHost: {addr}\r\nX-Forwarded-For: {xff}\r\nAuthorization: Bearer {admin}\r\nConnection: close\r\n\r\n"
        )
    };
    // Not trusted: the header means nothing (this computer is on the network).
    assert_eq!(raw(&addr, &from("8.8.8.8")).await.0, 200);
    let body = json!({ "address": "", "trusted_proxies": "127.0.0.1" });
    assert_eq!(post(&addr, "/v1/admin/network/reach", Some(&admin), body).await.0, 204);
    // Trusted: the request came from the internet, so no admin pages.
    let (status, _, text) = raw(&addr, &from("8.8.8.8")).await;
    assert_eq!(status, 403, "{text}");
    assert_eq!(raw(&addr, &from("8.8.8.8, 192.168.1.20")).await.0, 200);

    // Wrong passwords count per real address: one address's don't stop another's.
    let login = |xff: &str, password: &str| {
        let body = json!({ "username": "alex@example.org", "password": password, "device": "test" }).to_string();
        format!(
            "POST /v1/login HTTP/1.1\r\nHost: {addr}\r\nX-Forwarded-For: {xff}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    for _ in 0..10 {
        assert_eq!(raw(&addr, &login("203.0.113.9", "wrong password")).await.0, 401);
    }
    assert_eq!(raw(&addr, &login("203.0.113.9", "alex password 1")).await.0, 429);
    assert_eq!(raw(&addr, &login("198.51.100.4", "alex password 1")).await.0, 200);
}

#[tokio::test]
async fn the_apps_and_the_api_are_served_with_safe_headers() {
    let db = db_or_skip!("sec_headers");
    let data = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("sec_headers_data");
    let apps = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("sec_headers_apps");
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(apps.join("chat")).unwrap();
    let inline = "window.XENIFY_DRAW_AT = '../draw/';";
    std::fs::write(
        apps.join("chat/index.html"),
        format!("<!doctype html><script>{inline}</script><script type=\"module\" src=\"app.js\"></script>"),
    )
    .unwrap();
    std::fs::write(apps.join("chat/app.js"), "1").unwrap();
    let s = xenify_server::Server::start(xenify_server::Config {
        database: db.clone(),
        data,
        local_webhooks: true,
        web: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web")),
        secure: None,
        postgres: None,
    })
    .await
    .unwrap();
    s.host_apps(apps);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
    let s2 = s.clone();
    tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });

    let get_raw = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (status, head, _) = raw(&addr, &get_raw("/chat/")).await;
    assert_eq!(status, 200);
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
    let csp = head.lines().find(|l| l.starts_with("content-security-policy:")).unwrap_or_default().to_string();
    assert!(csp.contains("frame-ancestors 'self'") && csp.contains("object-src 'none'"), "{csp}");
    // The page's own small script may run (by its hash), nothing else inline.
    let hash = xenify_server::inline_script_hashes(&format!("<script>{inline}</script>"));
    assert_eq!(hash.len(), 1);
    assert!(csp.contains(&hash[0].to_lowercase()), "{csp}");
    assert!(!csp.contains("'unsafe-inline' 'self'") && !csp.split("script-src").nth(1).unwrap().split(';').next().unwrap().contains("unsafe-inline"));
    let (_, head, _) = raw(&addr, &get_raw("/chat/app.js")).await;
    assert!(head.contains("content-security-policy:") && head.contains("nosniff"), "{head}");
    // The API answers say what they are, too.
    let (_, head, _) = raw(&addr, &get_raw("/v1/info")).await;
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
    assert!(!head.contains("strict-transport-security"), "{head}");
}
