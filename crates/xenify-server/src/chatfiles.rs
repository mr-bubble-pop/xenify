//! Files, pictures and video clips sent in chats.
//!
//! The apps send a file in parts of at most [`PART`] bytes. Anyone in the
//! chat may fetch the parts; only the sender adds them, until the file is
//! done.

use crate::{Server, db, err, failed, signed_in, unavailable};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

/// The most a part holds (4 MiB).
pub const PART: usize = 4 << 20;
/// The largest file people may send unless an admin chooses otherwise (MB).
pub const DEFAULT_LIMIT_MB: i64 = 2048;

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/chat/files", post(start))
        .route("/v1/chat/files/{id}/{n}", get(part).put(put_part).layer(DefaultBodyLimit::max(PART)))
        .route("/v1/chat/files/{id}/done", post(done))
        .route("/v1/chat/file-limit", get(limit))
        .route("/v1/chat/boards", post(board_space))
}

#[derive(Deserialize)]
struct Start {
    conv: i64,
    size: i64,
}

#[derive(Serialize)]
struct Started {
    id: String,
    part: usize,
}

/// The largest file a chat takes, in bytes (Admin › Chat sets it).
pub async fn limit_bytes(c: &impl deadpool_postgres::GenericClient) -> i64 {
    let mb = db::setting(c, "chat_file_limit_mb")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_LIMIT_MB);
    mb.max(1) * 1024 * 1024
}

async fn limit(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = signed_in(&s, &h).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    Json(serde_json::json!({ "bytes": limit_bytes(&c).await })).into_response()
}

async fn member(c: &deadpool_postgres::Client, conv: i64, user: i64) -> bool {
    c.query_opt("SELECT 1 FROM members WHERE conv = $1 AND user_id = $2", &[&conv, &user])
        .await
        .ok()
        .flatten()
        .is_some()
}

fn dir(s: &Server, id: &str) -> std::path::PathBuf {
    s.secure.join("chat-files").join(id)
}

fn valid_id(id: &str) -> bool {
    id.len() == 24 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

async fn start(State(s): State<Server>, h: HeaderMap, Json(st): Json<Start>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if !member(&c, st.conv, me.id).await {
        return err(StatusCode::FORBIDDEN, "not_member", "you're not in that conversation");
    }
    let limit = limit_bytes(&c).await;
    if st.size < 0 || st.size > limit {
        let mb = limit / (1024 * 1024);
        return err(StatusCode::PAYLOAD_TOO_LARGE, "too_big", &format!("files in chats can be up to {mb} MB here"));
    }
    let parts = ((st.size as usize).div_ceil(PART)).max(1) as i32;
    let id = crate::random(18);
    let id: String = id.chars().take(24).collect();
    if c.execute(
        "INSERT INTO chat_files (id, conv, owner, size, parts) VALUES ($1, $2, $3, $4, $5)",
        &[&id, &st.conv, &me.id, &st.size, &parts],
    )
    .await
    .is_err()
    {
        return failed();
    }
    if std::fs::create_dir_all(dir(&s, &id)).is_err() {
        return failed();
    }
    Json(Started { id, part: PART }).into_response()
}

/// (conv, owner, parts, done) of a file.
async fn file_of(c: &deadpool_postgres::Client, id: &str) -> Option<(i64, i64, i32, bool)> {
    c.query_opt("SELECT conv, owner, parts, done FROM chat_files WHERE id = $1", &[&id])
        .await
        .ok()
        .flatten()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
}

async fn put_part(State(s): State<Server>, h: HeaderMap, Path((id, n)): Path<(String, i32)>, body: Bytes) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !valid_id(&id) {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some((_, owner, parts, done)) = file_of(&c, &id).await else {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    };
    if owner != me.id || done {
        return err(StatusCode::FORBIDDEN, "not_yours", "only the sender adds to a file, before it's done");
    }
    if n < 0 || n >= parts || body.len() > PART {
        return err(StatusCode::BAD_REQUEST, "bad_part", "that part doesn't belong to this file");
    }
    let path = dir(&s, &id).join(n.to_string());
    let tmp = path.with_extension("part");
    if tokio::fs::write(&tmp, &body).await.is_err() || tokio::fs::rename(&tmp, &path).await.is_err() {
        return failed();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn done(State(s): State<Server>, h: HeaderMap, Path(id): Path<String>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !valid_id(&id) {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some((_, owner, parts, _)) = file_of(&c, &id).await else {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    };
    if owner != me.id {
        return err(StatusCode::FORBIDDEN, "not_yours", "only the sender finishes a file");
    }
    let base = dir(&s, &id);
    if (0..parts).any(|n| !base.join(n.to_string()).is_file()) {
        return err(StatusCode::CONFLICT, "missing_parts", "some parts haven't arrived yet");
    }
    // Scanned for viruses as a whole, when an admin turned that on; a
    // refused file is gone (nobody can fetch its parts).
    let files: Vec<_> = (0..parts).map(|n| base.join(n.to_string())).collect();
    if let Err(e) = crate::scan::allow_files(&c, &format!("a chat file from {}", me.username), &files).await {
        if e.status() == StatusCode::UNPROCESSABLE_ENTITY {
            let _ = c.execute("DELETE FROM chat_files WHERE id = $1", &[&id]).await;
            let _ = tokio::fs::remove_dir_all(&base).await;
        }
        return e;
    }
    match c.execute("UPDATE chat_files SET done = TRUE WHERE id = $1", &[&id]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

async fn part(State(s): State<Server>, h: HeaderMap, Path((id, n)): Path<(String, i32)>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !valid_id(&id) {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let Some((conv, _, parts, done)) = file_of(&c, &id).await else {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    };
    if !done || n < 0 || n >= parts || !member(&c, conv, me.id).await {
        return err(StatusCode::NOT_FOUND, "no_file", "there's no such file");
    }
    match tokio::fs::read(dir(&s, &id).join(n.to_string())).await {
        Ok(b) => crate::files::attachment(([(header::CONTENT_TYPE, "application/octet-stream")], b).into_response()),
        Err(_) => err(StatusCode::NOT_FOUND, "no_file", "there's no such file"),
    }
}

#[derive(Deserialize)]
struct BoardSpace {
    conv: i64,
}

/// `POST /v1/chat/boards {conv}`: the space that conversation's whiteboards
/// go in (made the first time). Boards are ordinary files in it, opened and
/// edited together like any other; only the chat's members get in.
async fn board_space(State(s): State<Server>, h: HeaderMap, Json(b): Json<BoardSpace>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if !member(&c, b.conv, me.id).await {
        return err(StatusCode::FORBIDDEN, "not_member", "you're not in that conversation");
    }
    let found = c
        .query_one(
            "INSERT INTO spaces (kind, name, conv) VALUES ('chat', '', $1)
             ON CONFLICT (conv) WHERE kind = 'chat' DO UPDATE SET conv = excluded.conv RETURNING id",
            &[&b.conv],
        )
        .await;
    match found {
        Ok(r) => Json(serde_json::json!({ "space": r.get::<_, i64>(0) })).into_response(),
        Err(_) => failed(),
    }
}
