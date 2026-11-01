//! Browser notifications with the tab closed (Web Push): Chat in the
//! browser asks for this server's key, subscribes its service worker, and
//! the server sends the browser's push service a message only that browser
//! can read (RFC 8291), signed with the server's VAPID key (RFC 8292).
//!
//! A reminder's notification carries a token for its Snooze and Dismiss
//! buttons, which work without the page (the service worker has no
//! session).

use crate::{Server, db, err, failed, signed_in, unavailable};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use deadpool_postgres::GenericClient;
use serde::Deserialize;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS push_subs (
    endpoint TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS push_subs_by_user ON push_subs(user_id);
";

/// The most browsers one person keeps subscribed (the oldest go).
pub const MOST_PER_PERSON: i64 = 10;
/// How long one push service gets, and all of one person's.
const EACH: std::time::Duration = std::time::Duration::from_secs(10);
const ALL: std::time::Duration = std::time::Duration::from_secs(20);
/// How many pushes go out at once, whoever they're for.
const AT_ONCE: usize = 16;

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/push/key", get(key))
        .route("/v1/push/subscribe", post(subscribe))
        .route("/v1/push/unsubscribe", post(unsubscribe))
        .route("/v1/push/act", post(act))
}

/// The server's VAPID key (made the first time).
async fn signing_key(c: &impl GenericClient) -> Option<p256::ecdsa::SigningKey> {
    if let Some(k) = db::setting(c, "vapid_key").await.ok().flatten()
        && let Ok(bytes) = URL_SAFE_NO_PAD.decode(k)
        && let Ok(key) = p256::ecdsa::SigningKey::from_slice(&bytes)
    {
        return Some(key);
    }
    let key = p256::ecdsa::SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let text = URL_SAFE_NO_PAD.encode(key.to_bytes());
    // Two servers making one at once: the first kept wins.
    c.execute("INSERT INTO settings (key, value) VALUES ('vapid_key', $1) ON CONFLICT DO NOTHING", &[&text])
        .await
        .ok()?;
    let kept = db::setting(c, "vapid_key").await.ok().flatten()?;
    p256::ecdsa::SigningKey::from_slice(&URL_SAFE_NO_PAD.decode(kept).ok()?).ok()
}

fn public_of(key: &p256::ecdsa::SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(false).as_bytes())
}

/// `GET /v1/push/key`: what the browser subscribes with.
async fn key(State(s): State<Server>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match signing_key(&c).await {
        Some(k) => Json(serde_json::json!({ "key": public_of(&k) })).into_response(),
        None => failed(),
    }
}

#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}

/// A browser's subscription, as `PushSubscription.toJSON()` gives it.
#[derive(Deserialize)]
struct Subscription {
    endpoint: String,
    keys: Keys,
}

async fn subscribe(State(s): State<Server>, h: HeaderMap, Json(sub): Json<Subscription>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let ok_key = URL_SAFE_NO_PAD
        .decode(sub.keys.p256dh.trim_end_matches('='))
        .ok()
        .is_some_and(|k| p256::PublicKey::from_sec1_bytes(&k).is_ok());
    let ok_auth = URL_SAFE_NO_PAD.decode(sub.keys.auth.trim_end_matches('=')).ok().is_some_and(|a| a.len() == 16);
    if !sub.endpoint.starts_with("https://") || sub.endpoint.len() > 1000 || !ok_key || !ok_auth {
        return err(StatusCode::BAD_REQUEST, "bad_subscription", "that isn't a browser's push subscription");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match subscribe_as(&c, me.id, &sub.endpoint, sub.keys.p256dh.trim_end_matches('='), sub.keys.auth.trim_end_matches('=')).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// Keeps `endpoint` for `user`. One someone else subscribed stays theirs
/// (nothing said: it's quietly not taken over). Past the most a person
/// keeps, their oldest go.
pub async fn subscribe_as(c: &impl GenericClient, user: i64, endpoint: &str, p256dh: &str, auth: &str) -> Result<(), tokio_postgres::Error> {
    c.execute(
        "INSERT INTO push_subs (endpoint, user_id, p256dh, auth) VALUES ($1, $2, $3, $4)
         ON CONFLICT (endpoint) DO UPDATE SET p256dh = $3, auth = $4, created = now()
         WHERE push_subs.user_id = $2",
        &[&endpoint, &user, &p256dh, &auth],
    )
    .await?;
    c.execute(
        "DELETE FROM push_subs WHERE user_id = $1 AND endpoint IN
           (SELECT endpoint FROM push_subs WHERE user_id = $1 ORDER BY created DESC, endpoint OFFSET $2)",
        &[&user, &MOST_PER_PERSON],
    )
    .await?;
    Ok(())
}

#[derive(Deserialize)]
struct Unsubscribe {
    endpoint: String,
}

async fn unsubscribe(State(s): State<Server>, h: HeaderMap, Json(u): Json<Unsubscribe>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match c.execute("DELETE FROM push_subs WHERE endpoint = $1 AND user_id = $2", &[&u.endpoint, &me.id]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// The secret that signs reminder tokens (from the VAPID key, so there's
/// nothing more to keep).
fn token_secret(key: &p256::ecdsa::SigningKey) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::new().chain_update(b"xenify reminder buttons").chain_update(key.to_bytes()).finalize().to_vec()
}

fn sign(secret: &[u8], data: &str) -> String {
    use hmac::{KeyInit, Mac};
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret).expect("any key length");
    m.update(data.as_bytes());
    URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
}

/// A token that lets whoever holds it snooze or dismiss `user`'s reminder.
pub(crate) fn reminder_token(key: &p256::ecdsa::SigningKey, user: i64, reminder: &str) -> String {
    let data = URL_SAFE_NO_PAD.encode(format!("{user}:{reminder}"));
    let mac = sign(&token_secret(key), &data);
    format!("{data}.{mac}")
}

/// `user` and the reminder's key, from a token the server made.
fn read_token(key: &p256::ecdsa::SigningKey, token: &str) -> Option<(i64, String)> {
    let (data, mac) = token.split_once('.')?;
    let want = sign(&token_secret(key), data);
    // The same length and every byte the same, looked at in full.
    if want.len() != mac.len() || want.bytes().zip(mac.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
        return None;
    }
    let text = String::from_utf8(URL_SAFE_NO_PAD.decode(data).ok()?).ok()?;
    let (user, reminder) = text.split_once(':')?;
    Some((user.parse().ok()?, reminder.to_string()))
}

#[derive(Deserialize)]
struct Act {
    token: String,
    /// Remind again then (Unix ms); without it, dismissed.
    #[serde(default)]
    until: Option<i64>,
}

/// `POST /v1/push/act {token, until?}`: a notification's Snooze or Dismiss.
async fn act(State(s): State<Server>, Json(a): Json<Act>) -> Response {
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some(key) = signing_key(&c).await else { return failed() };
    let Some((user, reminder)) = read_token(&key, &a.token) else {
        return err(StatusCode::FORBIDDEN, "bad_token", "that reminder can't be changed from here");
    };
    let r = match a.until {
        Some(until) => {
            c.execute(
                "UPDATE reminders SET at = to_timestamp($3::bigint / 1000.0), shown = NULL WHERE user_id = $1 AND key = $2",
                &[&user, &reminder, &until],
            )
            .await
        }
        None => {
            c.execute(
                "UPDATE reminders SET shown = COALESCE(shown, now()), done = TRUE WHERE user_id = $1 AND key = $2",
                &[&user, &reminder],
            )
            .await
        }
    };
    if r.is_err() {
        return failed();
    }
    let note = format!("rg:{user}:{reminder}");
    let _ = c.execute("SELECT pg_notify($1, $2)", &[&xenify_chat_server::db::CHANNEL, &note]).await;
    StatusCode::NO_CONTENT.into_response()
}

/// The `Authorization` header for one push service (RFC 8292).
fn vapid(key: &p256::ecdsa::SigningKey, endpoint: &str, contact: &str) -> Option<String> {
    use p256::ecdsa::signature::Signer;
    let u = url_origin(endpoint)?;
    let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() + 12 * 3600;
    let head = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = URL_SAFE_NO_PAD.encode(serde_json::json!({ "aud": u, "exp": exp, "sub": contact }).to_string());
    let signed = format!("{head}.{claims}");
    let sig: p256::ecdsa::Signature = key.sign(signed.as_bytes());
    Some(format!("vapid t={signed}.{}, k={}", URL_SAFE_NO_PAD.encode(sig.to_bytes()), public_of(key)))
}

/// `https://host[:port]` of an address.
fn url_origin(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://")?;
    let host = rest.split('/').next().filter(|h| !h.is_empty())?;
    Some(format!("https://{host}"))
}

/// Sends `payload` (JSON the service worker shows) to every browser `user`
/// subscribed, in the background: the caller doesn't wait for push
/// services. Subscriptions the push service says are gone are forgotten.
pub(crate) async fn send(s: &Server, user: i64, payload: serde_json::Value) {
    let Ok(c) = s.pool.get().await else { return };
    let Ok(rows) = c.query("SELECT endpoint, p256dh, auth FROM push_subs WHERE user_id = $1", &[&user]).await else {
        return;
    };
    if rows.is_empty() {
        return;
    }
    let Some(key) = signing_key(&c).await else { return };
    // Push services want a way to reach whoever runs the server.
    let contact = match crate::mail::mail(&c).await {
        Some(m) => format!("mailto:{}", m.from_address),
        None => "https://xenify.net".to_string(),
    };
    drop(c);
    let mut body = payload.to_string();
    // Push messages hold about 4 KB.
    if body.len() > 3000 {
        let mut p = payload.clone();
        if let Some(b) = p.get_mut("body") {
            *b = serde_json::Value::String(b.as_str().unwrap_or("").chars().take(300).collect());
        }
        body = p.to_string();
    }
    let mut out = Vec::new();
    for r in rows.iter().take(MOST_PER_PERSON as usize) {
        let (endpoint, p256dh, auth): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        let Some(ua) = URL_SAFE_NO_PAD.decode(&p256dh).ok().and_then(|k| p256::PublicKey::from_sec1_bytes(&k).ok())
        else {
            continue;
        };
        let Some(auth) = URL_SAFE_NO_PAD.decode(&auth).ok().filter(|a| a.len() == 16) else { continue };
        let auth = web_push_native::Auth::clone_from_slice(&auth);
        let Ok(sealed) = web_push_native::encrypt(body.clone().into_bytes(), &ua, &auth) else { continue };
        let Some(authorization) = vapid(&key, &endpoint, &contact) else { continue };
        out.push((endpoint, sealed, authorization));
    }
    let pool = s.pool.clone();
    tokio::spawn(async move {
        let each = out.into_iter().map(|(endpoint, sealed, authorization)| async move {
            let status = deliver(&endpoint, sealed, authorization).await;
            (endpoint, status)
        });
        // All of them at once, for at most ALL; whatever hasn't answered by then is left.
        let Ok(done) = tokio::time::timeout(ALL, futures_util::future::join_all(each)).await else { return };
        let gone: Vec<String> =
            done.into_iter().filter(|(_, s)| matches!(s, Some(404 | 410))).map(|(e, _)| e).collect();
        if !gone.is_empty()
            && let Ok(c) = pool.get().await
        {
            let _ = c.execute("DELETE FROM push_subs WHERE endpoint = ANY($1)", &[&gone]).await;
        }
    });
}

/// Pushes waiting for their turn (at most AT_ONCE go at a time).
fn turns() -> &'static tokio::sync::Semaphore {
    static T: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    T.get_or_init(|| tokio::sync::Semaphore::new(AT_ONCE))
}

/// One push to one push service; its HTTP status.
async fn deliver(endpoint: &str, sealed: Vec<u8>, authorization: String) -> Option<u16> {
    let _turn = turns().acquire().await.ok()?;
    // Only public push services (never this computer or its network), and
    // the request goes to the very address checked. The look-up gets 5 seconds.
    let pinned = tokio::time::timeout(std::time::Duration::from_secs(5), xenify_chat_server::pin_webhook(endpoint, false))
        .await
        .ok()?
        .ok()?;
    let work = tokio::task::spawn_blocking(move || -> Option<u16> {
        let r = pinned
            .agent(EACH)
            .post(&pinned.url)
            .header("TTL", "43200")
            .header("Urgency", "high")
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", &authorization)
            .send(&sealed[..])
            .ok()?;
        Some(r.status().as_u16())
    });
    tokio::time::timeout(EACH + std::time::Duration::from_secs(1), work).await.ok()?.ok().flatten()
}

/// The token for a reminder's buttons (None when the server has no key).
pub(crate) async fn token_for(c: &impl GenericClient, user: i64, reminder: &str) -> Option<String> {
    Some(reminder_token(&signing_key(c).await?, user, reminder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_tokens_only_work_unchanged() {
        let key = p256::ecdsa::SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let t = reminder_token(&key, 7, "ev:abc:123");
        assert_eq!(read_token(&key, &t), Some((7, "ev:abc:123".into())));
        let (data, mac) = t.split_once('.').unwrap();
        let other = URL_SAFE_NO_PAD.encode("8:ev:abc:123");
        assert_eq!(read_token(&key, &format!("{other}.{mac}")), None);
        assert_eq!(read_token(&key, &format!("{data}.x{mac}")), None);
    }

    #[test]
    fn vapid_headers_are_signed_jwts() {
        use p256::ecdsa::signature::Verifier;
        let key = p256::ecdsa::SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let h = vapid(&key, "https://push.example.org/send/abc", "mailto:a@example.org").unwrap();
        let t = h.strip_prefix("vapid t=").unwrap().split(", k=").next().unwrap();
        let (signed, sig) = t.rsplit_once('.').unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(signed.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.org");
        let sig = p256::ecdsa::Signature::from_slice(&URL_SAFE_NO_PAD.decode(sig).unwrap()).unwrap();
        assert!(key.verifying_key().verify(signed.as_bytes(), &sig).is_ok());
    }
}
