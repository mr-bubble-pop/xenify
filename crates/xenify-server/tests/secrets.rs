//! One-time secrets: kept encrypted, handed over once, then gone.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn a_secret_opens_once_and_only_its_link_holder_has_the_key() {
    let Some(db) = fresh_db("secrets").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let priya = add_person(&addr, &admin, "priya", false).await;

    // Only people with an account make them, for 1 to 7 days.
    let sealed = "bm9uY2UuLi5jaXBoZXJ0ZXh0";
    let (status, _) = post(&addr, "/v1/secrets", None, json!({ "sealed": sealed, "days": 1 })).await;
    assert_eq!(status, 401);
    for days in [0, 8] {
        let (status, _) = post(&addr, "/v1/secrets", Some(&priya), json!({ "sealed": sealed, "days": days })).await;
        assert_eq!(status, 400);
    }
    let (status, _) = post(&addr, "/v1/secrets", Some(&priya), json!({ "sealed": "x".repeat(20_000) })).await;
    assert_eq!(status, 400);
    let (status, made) = post(&addr, "/v1/secrets", Some(&priya), json!({ "sealed": sealed, "days": 3 })).await;
    assert_eq!(status, 200, "{made}");
    let id = made["id"].as_str().unwrap();
    // The link names the page and the secret after the #, which browsers
    // don't send (the app adds the key there too).
    assert!(made["link"].as_str().unwrap().ends_with(&format!("/secret/#{id}")), "{made}");
    let days = (made["expires"].as_i64().unwrap() - now_ms()) as f64 / 86_400_000.0;
    assert!((2.9..3.1).contains(&days), "{days}");

    // Opening the page reads nothing (a link preview doesn't use it up).
    let (status, _) = call(&addr, "GET", "/secret/", None, None).await;
    assert_eq!(status, 200);
    // Whoever asks first gets it, without signing in; then it's gone.
    let (status, got) = post(&addr, &format!("/v1/secrets/{id}/open"), None, json!({})).await;
    assert_eq!((status, got["sealed"].as_str()), (200, Some(sealed)));
    let (status, again) = post(&addr, &format!("/v1/secrets/{id}/open"), None, json!({})).await;
    assert_eq!((status, again["code"].as_str()), (404, Some("gone")));

    // An expired one can't be opened, and goes.
    let (_, old) = post(&addr, "/v1/secrets", Some(&priya), json!({ "sealed": sealed })).await;
    let (c, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    c.execute("UPDATE secrets SET expires = now() - interval '1 minute'", &[]).await.unwrap();
    let (status, _) = post(&addr, &format!("/v1/secrets/{}/open", old["id"].as_str().unwrap()), None, json!({})).await;
    assert_eq!(status, 404);
    let left: i64 = c.query_one("SELECT count(*) FROM secrets", &[]).await.unwrap().get(0);
    assert_eq!(left, 0);

    // Guessing is slowed down.
    let mut last = 0;
    for n in 0..12 {
        last = post(&addr, &format!("/v1/secrets/guess{n}/open"), None, json!({})).await.0;
    }
    assert_eq!(last, 429);
}

/// With a passphrase: a wrong one doesn't use the secret up, the right one
/// opens it once, and after five wrong ones it's deleted.
#[tokio::test]
async fn a_passphrase_is_checked_before_the_secret_is_given_up() {
    let Some(db) = fresh_db("secrets_pass").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sealed = "bm9uY2UuLi5jaXBoZXJ0ZXh0";
    let open = |id: String, check: Option<&'static str>| {
        let addr = addr.clone();
        async move { post(&addr, &format!("/v1/secrets/{id}/open"), None, json!({ "check": check })).await }
    };

    let (_, made) = post(&addr, "/v1/secrets", Some(&admin), json!({ "sealed": sealed, "check": "right" })).await;
    let id = made["id"].as_str().unwrap().to_string();
    let (status, got) = open(id.clone(), Some("wrong")).await;
    assert_eq!((status, got["code"].as_str(), got["left"].as_i64()), (403, Some("wrong_passphrase"), Some(4)));
    // Without the check (the link alone) it's a wrong try too.
    let (status, got) = open(id.clone(), None).await;
    assert_eq!((status, got["left"].as_i64()), (403, Some(3)));
    let (status, got) = open(id.clone(), Some("right")).await;
    assert_eq!((status, got["sealed"].as_str()), (200, Some(sealed)));
    let (status, _) = open(id, Some("right")).await;
    assert_eq!(status, 404);

    // Five wrong ones and it's gone, even for the right passphrase.
    let (_, made) = post(&addr, "/v1/secrets", Some(&admin), json!({ "sealed": sealed, "check": "right" })).await;
    let id = made["id"].as_str().unwrap().to_string();
    for left in (0..5).rev() {
        let (status, got) = open(id.clone(), Some("wrong")).await;
        assert_eq!((status, got["left"].as_i64()), (403, Some(left)));
    }
    let (status, _) = open(id, Some("right")).await;
    assert_eq!(status, 404);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
