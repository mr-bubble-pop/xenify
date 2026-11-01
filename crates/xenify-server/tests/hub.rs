//! Xenify Hub and this server: the admin's update policy, and copies of the
//! signed release list and its downloads for the office's computers.

mod common;

use axum::routing::get as on_get;
use common::*;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use xenify_hub_core::manifest::{new_key, sha256_hex, sign, verify};
use xenify_hub_core::{Channel, Download, Manifest, Product, Release, Signed};

const APP: &[u8] = b"pretend Write program";

/// A pretend release site: its signed list (naming one real download and
/// one that doesn't match its bytes), and how often each file was fetched.
async fn releases() -> (String, [u8; 32], Arc<AtomicUsize>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let key = new_key();
    let dl = |name: &str, sha: String| Download {
        os: "linux".into(),
        arch: "x86_64".into(),
        url: format!("{site}/{name}"),
        size: APP.len() as u64,
        sha256: sha,
        run: "xenify-write".into(),
    };
    let m = Manifest {
        published: "2026-10-24T10:00:00Z".into(),
        products: vec![Product {
            id: "write".into(),
            name: "Write".into(),
            about: String::new(),
            component: false,
            requires: Vec::new(),
            releases: vec![
                Release {
                    version: "1.0.0".into(),
                    channel: Channel::Release,
                    date: "2026-10-24".into(),
                    notes: String::new(),
                    downloads: vec![dl("write", sha256_hex(APP))],
                },
                Release {
                    version: "1.1.0".into(),
                    channel: Channel::Preview,
                    date: "2026-10-24".into(),
                    notes: String::new(),
                    downloads: vec![dl("changed", "ab".repeat(32))],
                },
            ],
        }],
    };
    let signed = sign(&m, &key);
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = hits.clone();
    let app = axum::Router::new()
        .route("/manifest.json", on_get(move || async move { axum::Json(signed) }))
        .route(
            "/write",
            on_get(move || async move {
                h2.fetch_add(1, Ordering::SeqCst);
                APP
            }),
        )
        .route("/changed", on_get(|| async { APP }));
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (site, key.verifying_key().to_bytes(), hits)
}

async fn raw_get(addr: &str, path: &str, token: &str) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = String::from_utf8_lossy(&out[..split]).split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = out[split + 4..].to_vec();
    let chunked = String::from_utf8_lossy(&out[..split]).to_ascii_lowercase().contains("transfer-encoding: chunked");
    (status, if chunked { dechunk(&String::from_utf8_lossy(&body)).into_bytes() } else { body })
}

#[tokio::test]
async fn the_admin_sets_the_policy_and_the_server_keeps_copies_of_updates() {
    let Some(db) = fresh_db("hub").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (site, release_key, hits) = releases().await;

    // Nothing set: no copies, every choice the person's own.
    let (status, p) = get(&addr, "/v1/hub/policy", Some(&admin)).await;
    assert_eq!(status, 200);
    assert_eq!(p["mirror"], false);
    assert_eq!(get(&addr, "/v1/hub/manifest", Some(&admin)).await.0, 404);
    assert_eq!(get(&addr, "/v1/hub/policy", None).await.0, 401);

    let (status, _) =
        post(&addr, "/v1/admin/hub", Some(&admin), json!({ "source": "file:///etc/passwd", "mirror": true })).await;
    assert_eq!(status, 400);
    let (status, _) = post(&addr, "/v1/admin/hub", Some(&admin), json!({
        "source": format!("{site}/manifest.json"), "mirror": true,
        "products": { "write": { "allowed": true, "required": true, "choice": { "fixed": "1.0.0" }, "window": [22, 6] }, "draw": { "allowed": false } },
        "all_apps": { "choice": { "channel": "preview" }, "mode": "manual" },
    })).await;
    assert_eq!(status, 204);

    let (_, p) = get(&addr, "/v1/hub/policy", Some(&admin)).await;
    assert_eq!(p["mirror"], true);
    let policy: xenify_hub_core::Policy = serde_json::from_value(p["policy"].clone()).unwrap();
    assert_eq!(policy.products["write"].choice, Some(xenify_hub_core::Choice::Fixed("1.0.0".into())));
    assert!(!policy.products["draw"].allowed);
    // "All apps" reaches the Hubs too.
    assert_eq!(policy.all_apps.choice, Some(xenify_hub_core::Choice::Channel(xenify_hub_core::Channel::Preview)));
    assert_eq!(policy.all_apps.mode, Some(xenify_hub_core::UpdateMode::Manual));

    // The list as signed: the Hub checks it with the release key.
    let (status, list) = get(&addr, "/v1/hub/manifest", Some(&admin)).await;
    assert_eq!(status, 200);
    let signed: Signed = serde_json::from_value(list).unwrap();
    let m = verify(&signed, &[release_key], None).unwrap();
    let d = &m.products[0].releases[0].downloads[0];

    // A download is fetched once, then served from here.
    for _ in 0..3 {
        let (status, body) = raw_get(&addr, &format!("/v1/hub/files/{}", d.sha256), &admin).await;
        assert_eq!(status, 200);
        assert_eq!(body, APP);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // Not in the list, or not matching it: refused.
    assert_eq!(raw_get(&addr, &format!("/v1/hub/files/{}", "cd".repeat(32)), &admin).await.0, 404);
    assert_eq!(raw_get(&addr, &format!("/v1/hub/files/{}", "ab".repeat(32)), &admin).await.0, 502);
    assert_eq!(raw_get(&addr, "/v1/hub/files/..%2F..%2Fetc", &admin).await.0, 400);

    // Hubs say what they have; the admin sees how many have each version.
    for (id, v) in [("pc-1", "1.0.0"), ("pc-2", "1.0.0"), ("pc-3", "0.9.0"), ("pc-1", "1.0.0")] {
        let (status, _) = post(
            &addr,
            "/v1/hub/report",
            Some(&admin),
            json!({ "computer": id, "name": id, "products": { "write": v } }),
        )
        .await;
        assert_eq!(status, 204);
    }
    let (status, st) = get(&addr, "/v1/admin/hub/status", Some(&admin)).await;
    assert_eq!(status, 200);
    assert_eq!(st["computers"], 3);
    assert_eq!(st["products"]["write"]["1.0.0"], 2);
    assert_eq!(st["products"]["write"]["0.9.0"], 1);
    // Fixed at 1.0.0 by the policy.
    assert_eq!(st["latest"]["write"], "1.0.0");

    // "Update server now": the Hub on the server's computer is told.
    assert_eq!(post(&addr, "/v1/admin/hub/update-server", Some(&admin), json!({ "version": "nope" })).await.0, 400);
    assert_eq!(post(&addr, "/v1/admin/hub/update-server", Some(&admin), json!({ "version": "9.0.0" })).await.0, 204);
    let (_, p) = get(&addr, "/v1/hub/policy", Some(&admin)).await;
    assert_eq!(p["update_server"], "9.0.0");
}
