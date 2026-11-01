//! Browser push subscriptions: someone else's can't be taken over, and
//! each person keeps at most a few (the oldest go).

mod common;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::*;
use serde_json::json;

fn subscription(endpoint: &str) -> serde_json::Value {
    let key = p256::SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let public = key.public_key().to_sec1_bytes();
    json!({ "endpoint": endpoint, "keys": { "p256dh": URL_SAFE_NO_PAD.encode(public), "auth": URL_SAFE_NO_PAD.encode([7u8; 16]) } })
}

#[tokio::test]
async fn push_subscriptions_stay_with_their_owner() {
    let Some(db) = fresh_db("pushsubs").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let mallory = add_person(&addr, &admin, "mallory", false).await;
    let (c, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    let owner = |e: &'static str| {
        let c = &c;
        async move {
            c.query_opt("SELECT u.username FROM push_subs p JOIN users u ON u.id = p.user_id WHERE p.endpoint = $1", &[&e])
                .await
                .unwrap()
                .map(|r| r.get::<_, String>(0))
        }
    };

    let mine = "https://push.example.org/send/alex-browser";
    assert_eq!(post(&addr, "/v1/push/subscribe", Some(&admin), subscription(mine)).await.0, 204);
    // Mallory sends the same endpoint: nothing said, nothing taken.
    assert_eq!(post(&addr, "/v1/push/subscribe", Some(&mallory), subscription(mine)).await.0, 204);
    assert_eq!(owner(mine).await.as_deref(), Some("alex"));
    // Alex subscribing again (new keys) is fine.
    assert_eq!(post(&addr, "/v1/push/subscribe", Some(&admin), subscription(mine)).await.0, 204);
    assert_eq!(owner(mine).await.as_deref(), Some("alex"));

    // Twelve more browsers: the ten newest are kept.
    for i in 0..12 {
        let e = format!("https://push.example.org/send/m{i}");
        assert_eq!(post(&addr, "/v1/push/subscribe", Some(&mallory), subscription(&e)).await.0, 204);
    }
    let kept: Vec<String> = c
        .query("SELECT endpoint FROM push_subs p JOIN users u ON u.id = p.user_id WHERE u.username = 'mallory' ORDER BY endpoint", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(kept.len(), xenify_server::push::MOST_PER_PERSON as usize, "{kept:?}");
    assert!(!kept.iter().any(|e| e.ends_with("/m0") || e.ends_with("/m1")), "the oldest went: {kept:?}");
    assert_eq!(owner(mine).await.as_deref(), Some("alex"), "Alex's own isn't touched");
}
