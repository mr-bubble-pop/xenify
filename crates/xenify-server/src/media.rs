//! Pictures and clips in Draw canvases, kept apart from the canvas.
//!
//! A canvas's versions and changes name each picture or clip by its
//! SHA-256 (`"sha256:<hex>"`, see xenify_doc::blobs); the bytes are kept
//! once in `blobs/` and listed per file in `node_media`, so a picture is
//! sent and stored once however many versions and changes use it. The
//! apps send a file in parts of at most [`PART`] bytes and carry on where
//! they stopped (`POST` says how much has arrived); it's read back whole or
//! by range (video plays while it loads). Downloads put the parts together
//! as an `.xdraw` file (a zip, see xenify_doc::file), and an `.xdraw` file
//! uploaded is taken apart again.

use crate::files::{Me, need, node_role};
use crate::{Server, err, failed, signed_in, unavailable};
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::GenericClient;
use serde::Deserialize;
use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;
use xenify_server_proto::Role;

/// The most a part holds (4 MiB).
pub const PART: usize = 4 << 20;
/// The largest picture or clip (16 GiB).
pub const MAX_MEDIA: i64 = 16 << 30;
/// The most one person may have half sent at once, all files together: two
/// of the largest clips. Uploads they stopped over an hour ago make way.
const MAX_PENDING: u64 = 2 * MAX_MEDIA as u64;
/// The most files one person may have half sent at once (the apps send one
/// at a time; several windows or computers can each be sending).
const MAX_PENDING_FILES: usize = 64;
/// Space always left free on the disk, so the database and the server's
/// own files keep working however much people send.
const MIN_FREE: u64 = 1 << 30;
/// What an `.xdraw` file taken apart may hold: its entries, and its bytes
/// unpacked. The file itself is at most MAX_FILE (1 GiB) and keeps clips
/// and pictures as they are (not squeezed again), so a real one unpacks to
/// about its own size; four times that is far over any real canvas.
const MAX_ENTRIES: usize = 100_000;
const MAX_UNPACKED: u64 = 4 * crate::files::MAX_FILE as u64;
/// The canvas itself (`document.json`) is saved as a version: at most
/// MAX_FILE, like any save.
const MAX_DOC: u64 = crate::files::MAX_FILE as u64;

const TOO_BIG: &str = "This file is too big to open.";

pub fn routes() -> Router<Server> {
    Router::new()
        .route(
            "/v1/nodes/{id}/media/{hash}",
            post(start).put(put_part).get(fetch).layer(DefaultBodyLimit::max(PART + 1024)),
        )
        .route("/v1/admin/storage", get(get_storage).post(set_storage))
}

// ---- Room on the server --------------------------------------------------

/// Bytes free on the disk holding `path` (None: not known here).
fn free_bytes(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid C string and `st` a statvfs to fill.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn disk_full() -> Response {
    err(StatusCode::INSUFFICIENT_STORAGE, "disk_full", "the server's disk is nearly full: ask an admin to make room")
}

/// Whether `more` bytes still leave [`MIN_FREE`] on the disk.
fn room_on_disk(s: &Server, more: u64) -> Result<(), Response> {
    let dir = s.secure.join("uploads");
    let at = if dir.exists() { dir } else { s.secure.clone() };
    match free_bytes(&at) {
        Some(free) if free < MIN_FREE.saturating_add(more) => Err(disk_full()),
        _ => Ok(()),
    }
}

/// The most each person may store, in bytes (None: no limit; the default).
pub(crate) async fn quota(c: &impl GenericClient) -> Option<i64> {
    crate::db::setting(c, "storage_quota_gb")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|g| *g > 0)
        .map(|g| g.saturating_mul(1 << 30))
}

/// What `account` has stored: the versions they saved and the pictures and
/// clips they sent.
pub(crate) async fn used(c: &impl GenericClient, account: i64) -> i64 {
    c.query_one(
        "SELECT COALESCE((SELECT SUM(size) FROM versions WHERE author = $1), 0)::BIGINT
              + COALESCE((SELECT SUM(size) FROM node_media WHERE added_by = $1), 0)::BIGINT",
        &[&account],
    )
    .await
    .map_or(0, |r| r.get(0))
}

/// Whether `account` may store `more` bytes under the admins' limit.
pub(crate) async fn room_for(c: &impl GenericClient, account: i64, more: i64) -> Result<(), Response> {
    let Some(q) = quota(c).await else { return Ok(()) };
    if used(c, account).await.saturating_add(more) > q {
        return Err(err(
            StatusCode::INSUFFICIENT_STORAGE,
            "quota",
            &format!("you've used the {} GB you can store on this server: delete something or ask an admin for more", q >> 30),
        ));
    }
    Ok(())
}

/// Files `account` has half sent (in `dir`): each one's path, size and
/// when it last grew.
fn pending(dir: &std::path::Path, account: i64) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let prefix = format!("{account}-");
    let Ok(files) = std::fs::read_dir(dir) else { return Vec::new() };
    files
        .flatten()
        .filter(|f| f.file_name().to_str().is_some_and(|n| n.starts_with(&prefix)))
        .filter_map(|f| {
            let m = f.metadata().ok()?;
            Some((f.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .collect()
}

/// Whether `account` may add `more` bytes to file `path` being sent: what
/// they have half sent stays under [`MAX_PENDING`] and [`MAX_PENDING_FILES`].
/// Their uploads stopped for over an hour are dropped first to make room.
fn room_to_send(dir: &std::path::Path, account: i64, path: &std::path::Path, more: u64) -> bool {
    let mut files = pending(dir, account);
    let fits = |files: &[(PathBuf, u64, std::time::SystemTime)]| {
        let others = files.iter().filter(|f| f.0 != path).count();
        let bytes: u64 = files.iter().map(|f| f.1).sum();
        others < MAX_PENDING_FILES && bytes.saturating_add(more) <= MAX_PENDING
    };
    if fits(&files) {
        return true;
    }
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    files.sort_by_key(|f| f.2);
    while !fits(&files) {
        let Some(i) = files.iter().position(|f| f.0 != path && f.2 < hour_ago) else { return false };
        let _ = std::fs::remove_file(&files[i].0);
        files.remove(i);
    }
    true
}

#[derive(Deserialize)]
struct Storage {
    /// The most each person may store, in GB (0: no limit).
    quota_gb: i64,
}

async fn get_storage(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let free = free_bytes(&s.secure);
    Json(json!({ "quota_gb": quota(&c).await.map_or(0, |q| q >> 30), "free_gb": free.map(|f| f >> 30) })).into_response()
}

async fn set_storage(State(s): State<Server>, h: HeaderMap, Json(v): Json<Storage>) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.network").await {
        return e;
    }
    if !(0..=1 << 20).contains(&v.quota_gb) {
        return err(StatusCode::BAD_REQUEST, "bad_limit", "give a number of GB (0: no limit)");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match crate::db::set_setting(&c, "storage_quota_gb", &v.quota_gb.to_string()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

fn valid(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The hashes `json` names, each once.
pub fn refs(json: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(json);
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices("sha256:") {
        if let Some(h) = text.get(i + 7..i + 71)
            && valid(h)
            && !out.iter().any(|o| o == h)
        {
            out.push(h.to_string());
        }
    }
    out
}

async fn allowed(
    s: &Server,
    h: &HeaderMap,
    id: i64,
    at_least: Role,
) -> Result<(deadpool_postgres::Client, Me), Response> {
    let account = signed_in(s, h).await?;
    let Ok(c) = s.pool.get().await else { return Err(unavailable()) };
    let Ok(me) = crate::files::me(&c, account).await else { return Err(failed()) };
    let role = node_role(&c, &me, id).await.ok().flatten().and_then(|(_, r)| r);
    need(role, at_least)?;
    Ok((c, me))
}

async fn in_file(c: &impl GenericClient, id: i64, hash: &str) -> Option<i64> {
    c.query_opt("SELECT size FROM node_media WHERE node = $1 AND hash = $2", &[&id, &hash])
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
}

/// Where a file `account` is sending to file `id` grows (named by who
/// sends it, so what each person has half sent can be counted).
fn partial(s: &Server, account: i64, id: i64, hash: &str) -> PathBuf {
    s.secure.join("uploads").join(format!("{account}-{id}-{hash}"))
}

#[derive(Deserialize)]
struct Start {
    size: i64,
}

/// `POST {size}`: how much of it has arrived (`have`), or `done` when the
/// file has it already.
async fn start(
    State(s): State<Server>,
    h: HeaderMap,
    Path((id, hash)): Path<(i64, String)>,
    Json(st): Json<Start>,
) -> Response {
    let (c, me) = match allowed(&s, &h, id, Role::Editor).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid(&hash) {
        return err(StatusCode::BAD_REQUEST, "bad_hash", "that isn't a file's name");
    }
    if in_file(&c, id, &hash).await.is_some() {
        return Json(json!({ "done": true })).into_response();
    }
    if st.size < 0 || st.size > MAX_MEDIA {
        return err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_big",
            &format!("pictures and clips can be up to {} GB", MAX_MEDIA >> 30),
        );
    }
    let have = tokio::fs::metadata(partial(&s, me.account.id, id, &hash)).await.map_or(0, |m| m.len() as i64);
    // Said now, before any of it is sent (each part is checked again).
    if let Err(e) = room_for(&c, me.account.id, st.size).await {
        return e;
    }
    if let Err(e) = room_on_disk(&s, (st.size - have).max(0) as u64) {
        return e;
    }
    Json(json!({ "have": have.min(st.size), "part": PART })).into_response()
}

#[derive(Deserialize)]
struct At {
    at: i64,
    size: i64,
}

/// `PUT ?at&size`: the part starting `at` bytes in. The last one makes the
/// file, once what arrived is what its name says.
async fn put_part(
    State(s): State<Server>,
    h: HeaderMap,
    Path((id, hash)): Path<(i64, String)>,
    Query(q): Query<At>,
    body: Bytes,
) -> Response {
    let (c, me) = match allowed(&s, &h, id, Role::Editor).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid(&hash) {
        return err(StatusCode::BAD_REQUEST, "bad_hash", "that isn't a file's name");
    }
    if in_file(&c, id, &hash).await.is_some() {
        return Json(json!({ "done": true })).into_response();
    }
    let end = q.at + body.len() as i64;
    if q.size < 0 || q.size > MAX_MEDIA || q.at < 0 || end > q.size || body.len() > PART {
        return err(StatusCode::BAD_REQUEST, "bad_part", "that part doesn't belong to this file");
    }
    let account = me.account.id;
    // A new file to send: it must fit what they may store.
    if q.at == 0
        && let Err(e) = room_for(&c, account, q.size).await
    {
        return e;
    }
    if let Err(e) = room_on_disk(&s, body.len() as u64) {
        return e;
    }
    let path = partial(&s, account, id, &hash);
    let appended = {
        let path = path.clone();
        let at = q.at as u64;
        tokio::task::spawn_blocking(move || -> std::io::Result<Option<(u64, bool)>> {
            let dir = path.parent().unwrap();
            std::fs::create_dir_all(dir)?;
            if !room_to_send(dir, account, &path, body.len() as u64) {
                return Ok(None);
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
            let have = f.metadata()?.len();
            if have != at {
                return Ok(Some((have, false)));
            }
            f.write_all(&body)?;
            f.sync_data()?;
            Ok(Some((have + body.len() as u64, true)))
        })
        .await
    };
    let (have, wrote) = match appended {
        Ok(Ok(Some((n, wrote)))) => (n as i64, wrote),
        Ok(Ok(None)) => {
            return err(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_uploads",
                "you're sending too much at once: wait for your other uploads to finish",
            );
        }
        _ => return failed(),
    };
    if !wrote {
        // Not where this part goes (one went missing, or came twice).
        return (
            StatusCode::CONFLICT,
            Json(json!({ "code": "elsewhere", "error": "that part goes elsewhere", "have": have })),
        )
            .into_response();
    }
    if have < q.size {
        return Json(json!({ "have": have })).into_response();
    }
    // All there: it must be what its name says.
    let sum = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<String> {
            use sha2::{Digest, Sha256};
            let mut f = std::fs::File::open(&path)?;
            let mut d = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                d.update(&buf[..n]);
            }
            Ok(d.finalize().iter().map(|b| format!("{b:02x}")).collect())
        })
        .await
    };
    if !matches!(&sum, Ok(Ok(h)) if *h == hash) {
        let _ = tokio::fs::remove_file(&path).await;
        return err(StatusCode::UNPROCESSABLE_ENTITY, "damaged", "the file arrived damaged: it's sent again");
    }
    if let Err(e) = crate::scan::allow_files(
        &c,
        &format!("a picture in file {id} from {}", me.account.username),
        std::slice::from_ref(&path),
    )
    .await
    {
        let _ = tokio::fs::remove_file(&path).await;
        return e;
    }
    if s.blobs.adopt(&path, &hash).await.is_err() {
        return failed();
    }
    match c
        .execute(
            "INSERT INTO node_media (node, hash, size, added_by) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            &[&id, &hash, &q.size, &account],
        )
        .await
    {
        Ok(_) => Json(json!({ "have": have, "done": true })).into_response(),
        Err(_) => failed(),
    }
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` of a file of `size` bytes: the
/// first and last byte.
fn range(h: &HeaderMap, size: u64) -> Option<(u64, u64)> {
    let r = h.get(header::RANGE)?.to_str().ok()?.strip_prefix("bytes=")?;
    let (a, b) = r.split(',').next()?.trim().split_once('-')?;
    let (a, b) = match (a.parse::<u64>().ok(), b.parse::<u64>().ok()) {
        (Some(a), Some(b)) => (a, b.min(size.saturating_sub(1))),
        (Some(a), None) => (a, size.saturating_sub(1)),
        (None, Some(n)) => (size.saturating_sub(n), size.saturating_sub(1)),
        (None, None) => return None,
    };
    (a <= b && b < size).then_some((a, b))
}

/// The file's bytes (or the range asked for).
async fn fetch(State(s): State<Server>, h: HeaderMap, Path((id, hash)): Path<(i64, String)>) -> Response {
    let (c, _) = match allowed(&s, &h, id, Role::Viewer).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid(&hash) || in_file(&c, id, &hash).await.is_none() {
        return err(StatusCode::NOT_FOUND, "no_media", "that picture isn't in this file");
    }
    serve(&s, &hash, &h).await
}

/// The bytes of picture or clip `hash`, only ever as a download (see
/// files::attachment); media elements still play it.
pub(crate) async fn serve(s: &Server, hash: &str, h: &HeaderMap) -> Response {
    crate::files::attachment(serve_bytes(s, hash, h).await)
}

async fn serve_bytes(s: &Server, hash: &str, h: &HeaderMap) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let Some(path) = s.blobs.file(hash) else { return failed() };
    let Ok(mut f) = tokio::fs::File::open(&path).await else {
        return err(StatusCode::NOT_FOUND, "no_media", "that picture isn't on this server any more");
    };
    let Ok(size) = f.metadata().await.map(|m| m.len()) else { return failed() };
    let common = [
        (header::CONTENT_TYPE, "application/octet-stream".to_string()),
        (header::ACCEPT_RANGES, "bytes".to_string()),
        // Named by what's in it: it never changes.
        (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string()),
    ];
    match range(h, size) {
        Some((a, b)) => {
            if f.seek(std::io::SeekFrom::Start(a)).await.is_err() {
                return failed();
            }
            let body = Body::from_stream(tokio_util::io::ReaderStream::new(f.take(b - a + 1)));
            let mut r = (StatusCode::PARTIAL_CONTENT, common, body).into_response();
            let hs = r.headers_mut();
            hs.insert(header::CONTENT_RANGE, format!("bytes {a}-{b}/{size}").parse().unwrap());
            hs.insert(header::CONTENT_LENGTH, (b - a + 1).into());
            r
        }
        None if h.contains_key(header::RANGE) => {
            (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{size}"))]).into_response()
        }
        None => {
            let mut r = (common, Body::from_stream(tokio_util::io::ReaderStream::new(f))).into_response();
            r.headers_mut().insert(header::CONTENT_LENGTH, size.into());
            r
        }
    }
}

/// Whether a file's details (`meta`) say it's a Draw canvas.
pub fn is_draw(meta: &str) -> bool {
    let d: serde_json::Value = serde_json::from_str(meta).unwrap_or_default();
    d["app"].as_str() == Some("draw")
        || d["ext"].as_str() == Some("xdraw")
        || d["name"].as_str().is_some_and(|n| n.to_lowercase().ends_with(".xdraw"))
}

/// An `.xdraw` file uploaded (a zip): its pictures and clips kept for file
/// `id`, and the canvas itself (`document.json`) to save as the version.
/// None: not one (saved as it is). The zip's own word on sizes isn't
/// trusted: everything is read with a limit, and pictures and clips go
/// straight to disk, never whole into memory.
pub(crate) async fn take_apart(
    s: &Server,
    c: &impl GenericClient,
    id: i64,
    by: i64,
    body: &Bytes,
) -> Result<Option<Vec<u8>>, Response> {
    if !body.starts_with(b"PK") {
        return Ok(None);
    }
    room_on_disk(s, body.len() as u64)?;
    let dir = s.secure.join("uploads");
    let unpacked = {
        let (body, dir) = (body.clone(), dir.clone());
        tokio::task::spawn_blocking(move || unpack(&body, &dir, MAX_UNPACKED)).await
    };
    let (doc, media) = match unpacked {
        Ok(Ok(Some(v))) => v,
        Ok(Ok(None)) => return Ok(None),
        Ok(Err(Unpack::TooBig)) => return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "too_big", TOO_BIG)),
        Ok(Err(Unpack::Io)) | Err(_) => return Err(failed()),
    };
    let mut kept = media.into_iter();
    while let Some((hash, path, size)) = kept.next() {
        let stored = s.blobs.adopt(&path, &hash).await.is_ok()
            && c.execute(
                "INSERT INTO node_media (node, hash, size, added_by) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&id, &hash, &(size as i64), &by],
            )
            .await
            .is_ok();
        if !stored {
            let _ = tokio::fs::remove_file(&path).await;
            for (_, p, _) in kept {
                let _ = tokio::fs::remove_file(p).await;
            }
            return Err(failed());
        }
    }
    Ok(Some(doc))
}

#[derive(Debug, PartialEq)]
enum Unpack {
    TooBig,
    Io,
}

impl From<std::io::Error> for Unpack {
    fn from(_: std::io::Error) -> Self {
        Unpack::Io
    }
}

/// Takes an `.xdraw` file apart (see [`take_apart`]): `document.json`, and
/// each picture or clip it names written to a file in `dir` (hash, file,
/// size), checked to be what its name says. At most `cap` bytes come out
/// in all. None: not one.
#[allow(clippy::type_complexity)]
fn unpack(body: &[u8], dir: &std::path::Path, cap: u64) -> Result<Option<(Vec<u8>, Vec<(String, PathBuf, u64)>)>, Unpack> {
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(body)) else { return Ok(None) };
    if z.len() > MAX_ENTRIES {
        return Err(Unpack::TooBig);
    }
    let mut doc = Vec::new();
    match z.by_name("document.json") {
        Ok(f) => {
            f.take(MAX_DOC.min(cap) + 1).read_to_end(&mut doc)?;
        }
        Err(_) => return Ok(None),
    };
    if doc.len() as u64 > MAX_DOC.min(cap) {
        return Err(Unpack::TooBig);
    }
    let mut total = doc.len() as u64;
    let named = refs(&doc);
    let mut out: Vec<(String, PathBuf, u64)> = Vec::new();
    let mut starts = std::collections::HashSet::new();
    let result = (|| {
        for i in 0..z.len() {
            let Ok(f) = z.by_index(i) else { continue };
            // Two entries reading the same bytes: how a small zip claims a lot.
            if !starts.insert(f.header_start()) {
                return Err(Unpack::TooBig);
            }
            let Some(hash) = f.name().ok().and_then(|n| n.strip_prefix("media/").map(str::to_string)) else { continue };
            if !named.contains(&hash) || out.iter().any(|(h, _, _)| *h == hash) {
                continue;
            }
            std::fs::create_dir_all(dir)?;
            let path = dir.join(format!("unpack-{}", crate::random(9)));
            let left = (cap - total).min(MAX_MEDIA as u64);
            let got = (|| -> Result<(String, u64), Unpack> {
                use sha2::{Digest, Sha256};
                let mut to = std::fs::File::create(&path)?;
                let mut from = f.take(left + 1);
                let mut d = Sha256::new();
                let mut buf = vec![0u8; 1 << 20];
                let mut n = 0u64;
                loop {
                    let k = from.read(&mut buf)?;
                    if k == 0 {
                        break;
                    }
                    n += k as u64;
                    if n > left {
                        return Err(Unpack::TooBig);
                    }
                    d.update(&buf[..k]);
                    to.write_all(&buf[..k])?;
                }
                to.sync_data()?;
                Ok((d.finalize().iter().map(|b| format!("{b:02x}")).collect(), n))
            })();
            match got {
                // Named by what's in it: one that isn't stays out.
                Ok((sum, n)) if sum == hash => {
                    total += n;
                    out.push((hash, path, n));
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&path);
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (_, p, _) in &out {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    Ok(Some((doc, out)))
}

/// Version `doc` of Draw canvas `id` as an `.xdraw` file: the canvas with
/// its pictures and clips (and the file's picture, `preview`).
pub(crate) async fn put_together(s: &Server, c: &impl GenericClient, id: i64, doc: Vec<u8>) -> Response {
    let mut parts: Vec<(String, PathBuf)> = Vec::new();
    for hash in refs(&doc) {
        if in_file(c, id, &hash).await.is_some()
            && let Some(p) = s.blobs.file(&hash)
        {
            parts.push((format!("media/{hash}"), p));
        }
    }
    if let Ok(Some(r)) = c.query_opt("SELECT preview FROM nodes WHERE id = $1", &[&id]).await
        && let Some(h) = r.get::<_, Option<String>>(0)
        && let Some(p) = s.blobs.file(&h)
    {
        parts.push(("preview.png".into(), p));
    }
    // Written to a file first (clips can be big), then sent from it.
    let out = s.secure.join("uploads").join(format!("download-{}", crate::random(9)));
    let made = {
        let out = out.clone();
        tokio::task::spawn_blocking(move || -> zip::result::ZipResult<()> {
            std::fs::create_dir_all(out.parent().unwrap())?;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&out)?);
            let squeeze = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let keep = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(true);
            z.start_file("document.json", squeeze)?;
            z.write_all(&doc)?;
            for (name, path) in parts {
                z.start_file(name, keep)?;
                std::io::copy(&mut std::fs::File::open(path)?, &mut z)?;
            }
            z.finish()?;
            Ok(())
        })
        .await
    };
    let f = match made {
        Ok(Ok(())) => tokio::fs::File::open(&out).await,
        _ => {
            let _ = tokio::fs::remove_file(&out).await;
            return failed();
        }
    };
    // Gone from the folder at once; the open file is read to its end.
    let _ = tokio::fs::remove_file(&out).await;
    let Ok(f) = f else { return failed() };
    let size = f.metadata().await.map_or(0, |m| m.len());
    let mut r = ([(header::CONTENT_TYPE, "application/zip")], Body::from_stream(tokio_util::io::ReaderStream::new(f)))
        .into_response();
    r.headers_mut().insert(header::CONTENT_LENGTH, size.into());
    crate::files::attachment(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::RANGE, v.parse().unwrap());
            m
        };
        assert_eq!(range(&h("bytes=0-99"), 1000), Some((0, 99)));
        assert_eq!(range(&h("bytes=900-"), 1000), Some((900, 999)));
        assert_eq!(range(&h("bytes=-100"), 1000), Some((900, 999)));
        assert_eq!(range(&h("bytes=990-2000"), 1000), Some((990, 999)));
        assert_eq!(range(&h("bytes=1000-"), 1000), None);
        assert_eq!(range(&HeaderMap::new(), 1000), None);
    }

    fn hex(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("xenify-unpack-{}", crate::random(6)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn xdraw(doc: &str, media: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        let squeeze = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("document.json", squeeze).unwrap();
        z.write_all(doc.as_bytes()).unwrap();
        for (name, bytes) in media {
            z.start_file(format!("media/{name}"), squeeze).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn a_canvas_is_taken_apart() {
        let dir = scratch();
        let pic = b"a picture".to_vec();
        let h = hex(&pic);
        let doc = format!(r#"{{"src":"sha256:{h}"}}"#);
        let (got, media) = unpack(&xdraw(&doc, &[(&h, &pic), (&"0".repeat(64), b"not named")]), &dir, 1 << 30)
            .unwrap()
            .unwrap();
        assert_eq!(got, doc.as_bytes());
        assert_eq!(media.len(), 1);
        assert_eq!(std::fs::read(&media[0].1).unwrap(), pic);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn more_than_the_cap_unpacked_is_refused() {
        let dir = scratch();
        // 8 MB of zeros squeezes to a few KB.
        let big = vec![0u8; 8 << 20];
        let h = hex(&big);
        let doc = format!(r#"{{"src":"sha256:{h}"}}"#);
        let z = xdraw(&doc, &[(&h, &big)]);
        assert!(z.len() < 64 << 10);
        assert_eq!(unpack(&z, &dir, 4 << 20).unwrap_err(), Unpack::TooBig);
        // Nothing is left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert!(unpack(&z, &dir, 16 << 20).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_zip_lying_about_its_sizes_is_read_safely() {
        let dir = scratch();
        let pic = vec![7u8; 1000];
        let h = hex(&pic);
        let doc = format!(r#"{{"src":"sha256:{h}"}}"#);
        let mut z = xdraw(&doc, &[(&h, &pic)]);
        // Every unpacked size the zip gives (local and central) claims
        // nearly 4 GiB: nothing may be set aside on its word.
        let claim: u32 = u32::MAX - 1;
        for sig in [[0x50u8, 0x4b, 3, 4], [0x50, 0x4b, 1, 2]] {
            let mut at = 0;
            while let Some(p) = z[at..].windows(4).position(|w| w == sig) {
                let p = at + p;
                let off = if sig[2] == 3 { 22 } else { 24 };
                z[p + off..p + off + 4].copy_from_slice(&claim.to_le_bytes());
                at = p + 4;
            }
        }
        let t = std::time::Instant::now();
        // Read or refused without a huge allocation (which would abort the
        // test), and quickly (generous: a busy machine mustn't fail it).
        let _ = unpack(&z, &dir, 1 << 30);
        assert!(t.elapsed() < std::time::Duration::from_secs(30));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uploads_stopped_long_ago_make_way() {
        let dir = scratch();
        let old = dir.join("5-1-old");
        std::fs::write(&old, vec![0u8; 10]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
        let new = dir.join("5-1-new");
        // Someone else's don't count.
        std::fs::write(dir.join("6-1-x"), vec![0u8; 10]).unwrap();
        assert!(room_to_send(&dir, 5, &new, 100));
        assert!(old.exists());
        assert!(room_to_send(&dir, 5, &new, MAX_PENDING - 5));
        assert!(!old.exists());
        assert!(!room_to_send(&dir, 5, &new, MAX_PENDING + 1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn free_space_is_known() {
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|f| f > 0));
    }

    #[test]
    fn names_in_a_canvas() {
        let a = "a".repeat(64);
        let doc = format!(r#"{{"x":"sha256:{a}","y":"sha256:{a}","z":"sha256:nothex"}}"#);
        assert_eq!(refs(doc.as_bytes()), vec![a]);
    }
}
