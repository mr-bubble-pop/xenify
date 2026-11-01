//! Each person's own settings for an app ("plan": spelling on or off, which
//! AI to use), kept here so they follow them to every computer and browser.
//! What only makes sense on one device (recent files, window size) stays there.

use crate::{Server, err, failed, signed_in, unavailable};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS user_settings (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    app TEXT NOT NULL,
    value TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (user_id, app)
);
";

/// Settings are small: a few choices, not documents.
const MAX: usize = 16 * 1024;

pub fn routes() -> Router<Server> {
    Router::new().route("/v1/me/settings/{app}", get(read).put(write))
}

fn app_ok(app: &str) -> bool {
    !app.is_empty() && app.len() <= 20 && app.bytes().all(|b| b.is_ascii_lowercase())
}

async fn read(State(s): State<Server>, h: HeaderMap, Path(app): Path<String>) -> Response {
    let a = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !app_ok(&app) {
        return err(StatusCode::NOT_FOUND, "not_found", "no such app");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let v = c
        .query_opt("SELECT value FROM user_settings WHERE user_id = $1 AND app = $2", &[&a.id, &app])
        .await
        .ok()
        .flatten()
        .and_then(|r| serde_json::from_str::<Value>(r.get::<_, &str>(0)).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    Json(v).into_response()
}

/// Replaces them (the app sends all of its settings).
async fn write(State(s): State<Server>, h: HeaderMap, Path(app): Path<String>, body: Bytes) -> Response {
    let a = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !app_ok(&app) {
        return err(StatusCode::NOT_FOUND, "not_found", "no such app");
    }
    // Your words (spelling) can be a long list.
    let max = if app == "words" { 256 * 1024 } else { MAX };
    if body.len() > max {
        return err(StatusCode::PAYLOAD_TOO_LARGE, "too_big", "settings are too big");
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "settings are JSON");
    };
    if !v.is_object() {
        return err(StatusCode::BAD_REQUEST, "bad_request", "settings are a JSON object");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match c
        .execute(
            "INSERT INTO user_settings (user_id, app, value) VALUES ($1, $2, $3) ON CONFLICT (user_id, app) DO UPDATE SET value = $3",
            &[&a.id, &app, &v.to_string()],
        )
        .await
    {
        Ok(_) => Json(v).into_response(),
        Err(_) => failed(),
    }
}
