//! The admin pages: served at /admin/ and /join/ (and the Files page at
//! /files/ and /p/…) with a strict content policy; admin pages keep to
//! this network by default.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn admin_pages_are_served_safely() {
    let Some(db) = fresh_db("web").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let raw = |path: &'static str| {
        let addr = addr.clone();
        async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut out = Vec::new();
            s.read_to_end(&mut out).await.unwrap();
            String::from_utf8_lossy(&out).to_string()
        }
    };
    for path in ["/admin/", "/join/"] {
        let page = raw(path).await;
        assert!(page.starts_with("HTTP/1.1 200"), "{path}: {}", &page[..60]);
        assert!(page.to_lowercase().contains("content-security-policy: default-src 'self'"));
        assert!(page.contains("<script type=\"module\" src=\"app.js\">"));
    }
    // The Files page, and pages opened with a public link: the same page,
    // its code at /web/.
    for path in ["/files/", "/p/someToken123"] {
        let page = raw(path).await;
        assert!(page.starts_with("HTTP/1.1 200"), "{path}: {}", &page[..60]);
        assert!(page.to_lowercase().contains("content-security-policy: default-src 'self'"));
        assert!(page.contains("src=\"/web/files.js\""));
    }
    assert!(raw("/files").await.contains("location: /files/"));
    assert!(raw("/web/files.js").await.starts_with("HTTP/1.1 200"));
    // Not set up yet: the address leads to setting it up.
    assert!(raw("/").await.contains("location: /admin/"));
    // Typed without the last slash (the pages' code is found beside them).
    assert!(raw("/admin").await.contains("location: /admin/"));
    assert!(raw("/join?code=AB").await.contains("location: /join/?code=AB"));
    let admin = set_up(&addr, &server).await;
    assert!(raw("/").await.contains("location: /files/"));
    let (_, s) = get(&addr, "/v1/admin/settings", Some(&admin)).await;
    assert_eq!(s["admin_access"], "lan");
    let (status, _) = post(&addr, "/v1/admin/settings", Some(&admin), json!({ "admin_access": "everywhere" })).await;
    assert_eq!(status, 400);
    // From this computer, "local" still lets the admin in.
    post(&addr, "/v1/admin/settings", Some(&admin), json!({ "admin_access": "local" })).await;
    assert_eq!(get(&addr, "/v1/admin/people", Some(&admin)).await.0, 200);
}

/// Two servers kept running for looking at the admin pages by hand or in a
/// headless browser: one waiting to be set up, one with people, a welcome,
/// and a reset link.
/// `cargo test -p xenify-server --test web screens -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn screens() {
    let Some(db) = fresh_db("screens_new").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (fresh, fresh_server) = serve(&db).await;
    let Some(db) = fresh_db("screens").await else { return };
    let (addr, server) = serve(&db).await;
    // The apps in the browser (XENIFY_DEMO_APPS: a folder with write/ in it).
    if let Ok(apps) = std::env::var("XENIFY_DEMO_APPS") {
        server.host_apps(apps.into());
    }
    let admin = set_up(&addr, &server).await;
    // A mail server (nothing listens there) so email features show.
    let mail = json!({ "host": "127.0.0.1", "port": 9, "security": "none", "from_address": "files@example.org" });
    post(&addr, "/v1/admin/mail", Some(&admin), mail).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design" })).await;
    let design = g["id"].as_i64().unwrap();
    add_person(&addr, &admin, "sam", true).await;
    let (_, p) = post(
        &addr,
        "/v1/admin/people",
        Some(&admin),
        json!({ "username": "priya", "display_name": "Priya Shah", "email": "priya@example.org", "groups": [design] }),
    )
    .await;
    let (_, kim) = post(
        &addr,
        "/v1/admin/people",
        Some(&admin),
        json!({ "username": "kim", "display_name": "Kim Lee" }),
    )
    .await;
    let (_, reset) = post(&addr, &format!("/v1/admin/people/{}/reset", kim["id"]), Some(&admin), json!({})).await;
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    println!("SCREENS fresh=http://{fresh}/admin/#/setup/{}", fresh_server.setup_token().unwrap());
    println!("SCREENS addr=http://{addr}");
    println!("SCREENS session={}", json!({ "token": admin, "account": me }));
    println!("SCREENS welcome=priya {}", p["code"].as_str().unwrap());
    println!("SCREENS reset=kim {}", reset["code"].as_str().unwrap());
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
}
