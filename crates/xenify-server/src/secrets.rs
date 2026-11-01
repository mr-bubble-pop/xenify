//! One-time secrets: a password (or any short text) shared as a link that
//! can be opened once, within 1 to 7 days.
//!
//! - The app encrypts it (AES-GCM, a new 256-bit key) before it comes here;
//!   the key is only in the link's `#` part, which browsers never send. The
//!   server keeps what it can't read: the encrypted text, who made it and
//!   until when.
//! - Opening the page (`/secret/`) reads nothing, so link previews and
//!   mail scanners don't use it up; "Show" asks for it, and it's deleted in
//!   that same step: whoever asks second finds it gone.
//! - Someone who has the whole link can read it once; anyone else can't.
//! - A passphrase, if the sender adds one, is needed too. The key is then
//!   made from the link's key and the passphrase (PBKDF2), so the link alone
//!   opens nothing. So that a wrong guess doesn't use the secret up, the app
//!   also sends a check (an HMAC of the passphrase's key, keyed with the
//!   link's key): the server hands the secret over only when the check
//!   matches. It can't test passphrases itself without the link's key.
//!   After `TRIES` wrong ones the secret is deleted, so it can't be guessed
//!   at for days.

use crate::{Peer, Server, err, failed, random, signed_in, unavailable};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY,
    sealed TEXT NOT NULL,
    made_by BIGINT REFERENCES users(id) ON DELETE CASCADE,
    expires TIMESTAMPTZ NOT NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE secrets ADD COLUMN IF NOT EXISTS checks TEXT;
ALTER TABLE secrets ADD COLUMN IF NOT EXISTS tries INT NOT NULL DEFAULT 0;
";

/// Wrong passphrases before a secret is deleted.
pub const TRIES: i32 = 5;

/// The encrypted text, base64: a few kilobytes of secret at most.
const MOST: usize = 16 * 1024;

pub fn routes() -> Router<Server> {
    Router::new().route("/v1/secrets", post(make)).route("/v1/secrets/{id}/open", post(open))
}

#[derive(Deserialize)]
struct Make {
    /// The encrypted secret (base64 of nonce and ciphertext).
    sealed: String,
    /// How long it may wait to be opened: 1 to 7 days.
    #[serde(default = "one")]
    days: i32,
    /// With a passphrase: the check the opener's app must send (base64).
    #[serde(default)]
    check: Option<String>,
}

#[derive(Deserialize, Default)]
struct Open {
    #[serde(default)]
    check: Option<String>,
}

fn one() -> i32 {
    1
}

async fn make(State(s): State<Server>, h: HeaderMap, Json(b): Json<Make>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if b.sealed.is_empty() || b.sealed.len() > MOST {
        return err(StatusCode::BAD_REQUEST, "bad_secret", "a secret is 1 to 16,000 characters");
    }
    if !(1..=7).contains(&b.days) {
        return err(StatusCode::BAD_REQUEST, "bad_days", "a secret lasts 1 to 7 days");
    }
    let check = b.check.filter(|c| !c.is_empty());
    if check.as_ref().is_some_and(|c| c.len() > 100) {
        return err(StatusCode::BAD_REQUEST, "bad_check", "the passphrase check is too long");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let _ = c.execute("DELETE FROM secrets WHERE expires < now()", &[]).await;
    let id = random(16);
    let made = c
        .query_one(
            "INSERT INTO secrets (id, sealed, made_by, expires, checks)
             VALUES ($1, $2, $3, now() + make_interval(days => $4), $5)
             RETURNING (extract(epoch FROM expires) * 1000)::BIGINT",
            &[&id, &b.sealed, &me.id, &b.days, &check],
        )
        .await;
    let Ok(row) = made else { return failed() };
    let base = crate::link_base(&c, &h).await;
    Json(json!({ "id": id, "expires": row.get::<_, i64>(0), "link": format!("{base}/secret/#{id}") })).into_response()
}

/// Hands the encrypted secret over once, and forgets it (with a
/// passphrase: only when the check matches).
async fn open(
    State(s): State<Server>,
    peer: Peer,
    h: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<Open>>,
) -> Response {
    let check = body.map(|Json(b)| b).unwrap_or_default().check;
    let keys = [format!("secret:{}", crate::limit::ip_key(s.client_ip(peer.0.ip(), &h).await))];
    if s.limit.blocked(&keys) {
        return err(StatusCode::TOO_MANY_REQUESTS, "slow_down", "too many tries: wait a few minutes");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let gone = c.execute("DELETE FROM secrets WHERE expires < now()", &[]).await;
    if gone.is_err() {
        return failed();
    }
    // One step, so two people asking at once can't both have it.
    let got = c
        .query_opt(
            "DELETE FROM secrets WHERE id = $1 AND (checks IS NULL OR checks = $2) RETURNING sealed",
            &[&id, &check],
        )
        .await;
    match got {
        Ok(Some(row)) => Json(json!({ "sealed": row.get::<_, String>(0) })).into_response(),
        Ok(None) => {
            // Still there: the passphrase was wrong.
            let wrong = c
                .query_opt(
                    "UPDATE secrets SET tries = tries + 1 WHERE id = $1 AND checks IS NOT NULL RETURNING tries",
                    &[&id],
                )
                .await;
            if let Ok(Some(row)) = wrong {
                s.limit.failed(&keys);
                let tries: i32 = row.get(0);
                if tries >= TRIES {
                    let _ = c.execute("DELETE FROM secrets WHERE id = $1", &[&id]).await;
                }
                let left = (TRIES - tries).max(0);
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "code": "wrong_passphrase", "error": "that passphrase is wrong", "left": left })),
                )
                    .into_response();
            }
            s.limit.failed(&keys);
            err(StatusCode::NOT_FOUND, "gone", "this secret was already viewed, or it expired")
        }
        Err(_) => failed(),
    }
}
