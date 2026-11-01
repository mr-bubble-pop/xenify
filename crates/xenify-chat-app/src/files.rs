//! Files, pictures and video clips in chats. A file goes to the server in
//! parts of 4 MiB, and the message says which file it is. Pictures and
//! videos play from `xfile:` addresses that fetch just the parts needed, so
//! long clips can be skipped through.

use crate::net::Net;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use xenify_sync::b64;

/// What a message says about a file (as JSON after `~file:`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    /// Bytes per part.
    pub part: usize,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(crate::roots::tls())
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn said(status: u16, text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v["error"].as_str().map(|e| {
                let mut c = e.chars();
                c.next().map(|f| format!("{}{}.", f.to_uppercase(), c.as_str())).unwrap_or_default()
            })
        })
        .unwrap_or_else(|| format!("The server said no ({status})."))
}

/// Largest part read back from the server (it sends 4 MiB ones). A file's
/// details come from a message anyone in the chat could write, so they're
/// checked before they size anything.
const MAX_PART: usize = 16 << 20;

/// The largest file a server takes (Admin › Chat goes up to 100 GiB).
const MAX_SIZE: u64 = 100 << 30;

/// Whether a message's file details can be fetched: the server's kind of
/// id (URL-safe base64, so it stays one piece of the address), and a part
/// size and file size it could have sent.
fn fetchable(info: &FileInfo) -> bool {
    !info.id.is_empty()
        && info.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && (1..=MAX_PART).contains(&info.part)
        && info.size <= MAX_SIZE
}

/// The type a chat file is shown as: pictures, sound and video the
/// webview plays; anything else (SVG, HTML, XML…, which could run scripts)
/// as plain bytes. The type comes from the message, so anyone could pick it.
fn shown_type(mime: &str) -> &str {
    // Without parameters ("video/webm;codecs=vp8" plays as video/webm).
    let base = mime.split(';').next().unwrap_or_default().trim();
    let m = base.to_ascii_lowercase();
    let ok = matches!(
        m.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/bmp"
            | "video/mp4"
            | "video/webm"
            | "video/ogg"
            | "video/quicktime"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/aac"
            | "audio/ogg"
            | "audio/opus"
            | "audio/wav"
            | "audio/x-wav"
            | "audio/webm"
            | "audio/flac"
    );
    if ok { base } else { "application/octet-stream" }
}

/// A file's part (file id, part number) and its bytes.
type Cached = ((String, usize), Arc<Vec<u8>>);

/// Uploads in progress (id → part size), and recently fetched parts.
#[derive(Default)]
pub struct Files {
    uploads: Mutex<HashMap<String, usize>>,
    cache: Mutex<VecDeque<Cached>>,
}

#[derive(Clone, Serialize)]
struct Progress {
    upload: String,
    sent: u64,
    total: u64,
}

fn start_upload(server: &str, token: &str, conv: i64, size: u64) -> Result<(String, usize), String> {
    let mut r = agent()
        .post(&format!("{server}/v1/chat/files"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(serde_json::json!({ "conv": conv, "size": size }).to_string())
        .map_err(|_| "Can't reach the server.".to_string())?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    if status != 200 {
        return Err(said(status, &text));
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok((v["id"].as_str().unwrap_or_default().to_string(), v["part"].as_u64().unwrap_or(4 << 20) as usize))
}

fn put_part(server: &str, token: &str, id: &str, n: usize, bytes: &[u8]) -> Result<(), String> {
    for attempt in 0..3 {
        let sent = agent()
            .put(&format!("{server}/v1/chat/files/{id}/{n}"))
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/octet-stream")
            .send(bytes);
        match sent {
            Ok(r) if r.status().as_u16() == 204 => return Ok(()),
            Ok(mut r) if r.status().as_u16() < 500 => {
                let status = r.status().as_u16();
                return Err(said(status, &r.body_mut().read_to_string().unwrap_or_default()));
            }
            _ => std::thread::sleep(Duration::from_millis(500 * (attempt + 1))),
        }
    }
    Err("Can't reach the server.".into())
}

fn finish(server: &str, token: &str, id: &str) -> Result<(), String> {
    let mut r = agent()
        .post(&format!("{server}/v1/chat/files/{id}/done"))
        .header("Authorization", &format!("Bearer {token}"))
        .send_empty()
        .map_err(|_| "Can't reach the server.".to_string())?;
    let status = r.status().as_u16();
    if status == 204 { Ok(()) } else { Err(said(status, &r.body_mut().read_to_string().unwrap_or_default())) }
}

fn mime_of(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "oga") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Sends a file from this computer (dropped on the window): uploads it, telling the screens how far it got (`upload` events).
#[tauri::command]
pub async fn file_send_path(
    app: AppHandle,
    net: State<'_, Net>,
    conv: i64,
    path: String,
    upload: String,
) -> Result<FileInfo, String> {
    let (server, token) = net.creds();
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        let p = std::path::PathBuf::from(&path);
        let meta = std::fs::metadata(&p).map_err(|_| "That file can't be read.".to_string())?;
        if !meta.is_file() {
            return Err("Only files can be sent (not folders).".to_string());
        }
        let size = meta.len();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        let (id, part) = start_upload(&server, &token, conv, size)?;
        let mut f = std::fs::File::open(&p).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; part];
        let mut sent = 0u64;
        let mut n = 0;
        loop {
            let mut got = 0;
            while got < part {
                match f.read(&mut buf[got..]).map_err(|e| e.to_string())? {
                    0 => break,
                    k => got += k,
                }
            }
            if got == 0 && n > 0 {
                break;
            }
            put_part(&server, &token, &id, n, &buf[..got])?;
            sent += got as u64;
            n += 1;
            let _ = app.emit("upload", Progress { upload: upload.clone(), sent, total: size });
            if got < part {
                break;
            }
        }
        finish(&server, &token, &id)?;
        Ok(FileInfo { id, mime: mime_of(&name).into(), name, size, part })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sends a file picked in the screens, a part at a time: first `file_begin`,
/// then `file_part` for each part (the bytes as the call's body, the upload
/// id and part number in the headers), then `file_end`.
#[tauri::command]
pub async fn file_begin(
    net: State<'_, Net>,
    files: State<'_, Files>,
    conv: i64,
    size: u64,
) -> Result<(String, usize), String> {
    let (server, token) = net.creds();
    let (id, part) = tauri::async_runtime::spawn_blocking(move || start_upload(&server, &token, conv, size))
        .await
        .map_err(|e| e.to_string())??;
    files.uploads.lock().unwrap().insert(id.clone(), part);
    Ok((id, part))
}

#[tauri::command]
pub async fn file_part(
    net: State<'_, Net>,
    files: State<'_, Files>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let h = request.headers();
    let id = h.get("upload").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let n: usize = h.get("part").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).ok_or("which part?")?;
    // The bytes as they are, or (where the web view can't send raw bytes)
    // as a list of numbers.
    let bytes: Vec<u8> = match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => b.clone(),
        tauri::ipc::InvokeBody::Json(v) => match v.as_array() {
            Some(list) => list.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect(),
            None => {
                return Err(format!(
                    "send the part's bytes (got {})",
                    v.to_string().chars().take(40).collect::<String>()
                ));
            }
        },
    };
    let part = files.uploads.lock().unwrap().get(&id).copied().ok_or("that upload isn't going")?;
    if bytes.len() > part {
        return Err("that part is too big".into());
    }
    let (server, token) = net.creds();
    tauri::async_runtime::spawn_blocking(move || put_part(&server, &token, &id, n, &bytes))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn file_end(
    net: State<'_, Net>,
    files: State<'_, Files>,
    id: String,
    name: String,
    size: u64,
    mime: String,
) -> Result<FileInfo, String> {
    let part = files.uploads.lock().unwrap().remove(&id).ok_or("that upload isn't going")?;
    let (server, token) = net.creds();
    let i = id.clone();
    tauri::async_runtime::spawn_blocking(move || finish(&server, &token, &i)).await.map_err(|e| e.to_string())??;
    let mime = if mime.is_empty() { mime_of(&name).to_string() } else { mime };
    Ok(FileInfo { id, name, size, mime, part })
}

/// One part (kept for a while: a video asks for the same parts again and
/// again).
fn fetch(files: &Files, server: &str, token: &str, info: &FileInfo, n: usize) -> Result<Arc<Vec<u8>>, String> {
    if !fetchable(info) {
        return Err("That file isn't on the server.".into());
    }
    let k = (info.id.clone(), n);
    if let Some((_, v)) = files.cache.lock().unwrap().iter().find(|(c, _)| *c == k) {
        return Ok(v.clone());
    }
    let mut r = agent()
        .get(&format!("{server}/v1/chat/files/{}/{n}", info.id))
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|_| "Can't reach the server.".to_string())?;
    let status = r.status().as_u16();
    if status != 200 {
        return Err(said(status, &r.body_mut().read_to_string().unwrap_or_default()));
    }
    let bytes = Arc::new(r.body_mut().with_config().limit(MAX_PART as u64).read_to_vec().map_err(|e| e.to_string())?);
    let mut c = files.cache.lock().unwrap();
    c.push_back((k, bytes.clone()));
    while c.len() > 12 {
        c.pop_front();
    }
    Ok(bytes)
}

/// Saves a file in Downloads (named like the original, numbered if taken).
#[tauri::command]
pub async fn file_save(app: AppHandle, info: FileInfo) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let net = app.state::<Net>();
        let files = app.state::<Files>();
        let (server, token) = net.creds();
        let dir = xenify_wp_check::home_dir().join("Downloads");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = xenify_mail_core::message::safe_file_name(&info.name);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let mut path = dir.join(&name);
        let mut i = 2;
        while path.exists() {
            path = dir.join(format!("{stem} ({i}){ext}"));
            i += 1;
        }
        if !fetchable(&info) {
            return Err("That file isn't on the server.".into());
        }
        // Written part by part (a long clip needn't fit in memory).
        let tmp = path.with_extension("download");
        let write = || -> Result<(), String> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            for n in 0..info.size.div_ceil(info.part as u64).max(1) {
                let n = usize::try_from(n).map_err(|e| e.to_string())?;
                f.write_all(&fetch(&files, &server, &token, &info, n)?).map_err(|e| e.to_string())?;
            }
            f.sync_all().map_err(|e| e.to_string())
        };
        if let Err(e) = write().and_then(|_| std::fs::rename(&tmp, &path).map_err(|e| e.to_string())) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `xfile://localhost/<id>/<size>/<part>?t=<type>`: a chat file for
/// pictures and players, with byte ranges.
pub fn protocol(app: &AppHandle, request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode, header};
    let bad = |s: StatusCode| Response::builder().status(s).body(Vec::new()).unwrap();
    let uri = request.uri();
    // (The screens' address helper encodes the slashes.)
    let path = uri.path().replace("%2F", "/").replace("%2f", "/");
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let [id, size, part] = parts[..] else { return bad(StatusCode::NOT_FOUND) };
    let (Ok(size), Ok(part)) = (size.parse::<u64>(), part.parse::<usize>()) else { return bad(StatusCode::NOT_FOUND) };
    let mime = uri
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("t=")))
        .map(|t| shown_type(&t.replace("%2F", "/").replace("%2f", "/")).to_string())
        .unwrap_or_else(|| "application/octet-stream".into());
    let info = FileInfo { id: id.into(), name: String::new(), size, mime: mime.clone(), part };
    if !fetchable(&info) {
        return bad(StatusCode::NOT_FOUND);
    }
    // Which bytes: all, or the range asked for (players ask in pieces; a
    // couple of parts at a time is plenty for them to go on with).
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split_once('-'))
        .map(|(a, b)| (a.parse::<u64>().unwrap_or(0), b.parse::<u64>().ok()));
    let last = size.saturating_sub(1);
    let (from, to) = match range {
        Some((a, b)) => (a, b.unwrap_or(last).min(last).min(a + part as u64 * 2 - 1)),
        None => (0, last),
    };
    if size == 0 || from > to {
        return Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, mime).body(Vec::new()).unwrap();
    }
    let net = app.state::<Net>();
    let files = app.state::<Files>();
    let (server, token) = net.creds();
    // Grown as parts come (the size is the message's word, not the server's).
    let mut out = Vec::new();
    for n in (from as usize / part)..=(to as usize / part) {
        match fetch(&files, &server, &token, &info, n) {
            Ok(p) => {
                let start = n as u64 * part as u64;
                let a = from.saturating_sub(start) as usize;
                let b = ((to + 1).min(start + p.len() as u64) - start) as usize;
                if a < b {
                    out.extend_from_slice(&p[a..b]);
                }
            }
            Err(_) => return bad(StatusCode::BAD_GATEWAY),
        }
    }
    let ranged = range.is_some();
    let mut r = Response::builder()
        // The screens fetch clips from here (a different origin to theirs).
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, out.len());
    if ranged {
        r = r.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {from}-{to}/{size}"));
    } else {
        r = r.status(StatusCode::OK);
    }
    r.body(out).unwrap()
}

/// A whiteboard shared in a chat (as JSON after `~board:` in a message):
/// a Draw file on the server, in the chat's own space.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardInfo {
    pub node: i64,
    pub title: String,
}

fn post_json(server: &str, token: &str, path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut r = agent()
        .post(&format!("{server}{path}"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|_| "Can't reach the server.".to_string())?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    if status != 200 {
        return Err(said(status, &text));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Makes a new whiteboard for a chat: a Draw file in the chat's space.
#[tauri::command]
pub async fn board_new(net: State<'_, Net>, conv: i64, title: String) -> Result<BoardInfo, String> {
    let (server, token) = net.creds();
    tauri::async_runtime::spawn_blocking(move || {
        let space = post_json(&server, &token, "/v1/chat/boards", serde_json::json!({ "conv": conv }))?["space"]
            .as_i64()
            .ok_or("The server sent something unexpected.")?;
        let title = if title.trim().is_empty() { "Whiteboard".to_string() } else { title.trim().to_string() };
        let meta = serde_json::json!({ "name": title, "app": "draw", "ext": "xdraw" }).to_string();
        let node = post_json(
            &server,
            &token,
            "/v1/nodes",
            serde_json::json!({ "space": space, "parent": null, "kind": "file", "meta": meta }),
        )?["id"]
            .as_i64()
            .ok_or("The server sent something unexpected.")?;
        Ok(BoardInfo { node, title })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What the whiteboard frame needs to reach the server as you.
#[tauri::command]
pub fn board_session(net: State<'_, Net>) -> serde_json::Value {
    let (server, token) = net.creds();
    serde_json::json!({ "url": server, "token": token, "account": net.me() })
}

/// Saves something the screens made (a board as a picture, PDF or Draw
/// file) in Downloads: the bytes as the call's body, the name in a header.
#[tauri::command]
pub fn save_download(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let name = request
        .headers()
        .get("name")
        .and_then(|v| v.to_str().ok())
        .map(|n| n.replace("%20", " "))
        .unwrap_or_else(|| "download".into());
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("send the file's bytes".into()) };
    let dir = xenify_wp_check::home_dir().join("Downloads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = xenify_mail_core::message::safe_file_name(&name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut path = dir.join(&name);
    let mut i = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({i}){ext}"));
        i += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Opens a chat's whiteboard in Xenify Draw (still shared live): a small
/// file only you can read, with the server and the board, that Draw
/// opens and this app removes a minute later.
#[tauri::command]
pub fn board_open_in_draw(net: State<'_, Net>, info: BoardInfo) -> Result<(), String> {
    let (server, _) = net.creds();
    let dir = xenify_wp_check::data_dir().join("boards");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.xboard", b64(&xenify_sync::random(9))));
    let text = serde_json::json!({ "server": server, "node": info.node, "title": info.title }).to_string();
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&path).map_err(|e| e.to_string())?, text.as_bytes())
        .map_err(|e| e.to_string())?;
    let started = start_draw(&path);
    let gone = path.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(if started { 60 } else { 0 }));
        let _ = std::fs::remove_file(gone);
    });
    if started { Ok(()) } else { Err("Xenify Draw isn't installed on this computer: get it from Xenify Hub.".into()) }
}

/// Starts Xenify Draw with a file: from the PATH, or where Xenify Hub puts
/// it (Linux ~/.local/bin, the Windows Start menu, the Mac's ~/Applications).
fn start_draw(file: &std::path::Path) -> bool {
    use std::process::Command;
    let exe = if cfg!(windows) { "xenify-draw.exe" } else { "xenify-draw" };
    if Command::new(exe).arg(file).spawn().is_ok() {
        return true;
    }
    let home = xenify_wp_check::home_dir();
    if cfg!(target_os = "macos") {
        for app in [home.join("Applications/Xenify Draw.app"), "/Applications/Xenify Draw.app".into()] {
            if app.exists() && Command::new("open").arg("-a").arg(&app).arg(file).spawn().is_ok() {
                return true;
            }
        }
        return false;
    }
    if cfg!(windows) {
        let link = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(r"Microsoft\Windows\Start Menu\Programs\Xenify\Xenify Draw.lnk");
        // Started through the shortcut by the shell itself (not cmd.exe,
        // which would read characters like & in the folder names as
        // commands); the paths go as environment variables, never as code.
        return link.exists()
            && Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-WindowStyle",
                    "Hidden",
                    "-Command",
                    "Start-Process -FilePath $env:XENIFY_LINK -ArgumentList ([char]34 + $env:XENIFY_FILE + [char]34)",
                ])
                .env("XENIFY_LINK", &link)
                .env("XENIFY_FILE", file)
                .spawn()
                .is_ok();
    }
    Command::new(home.join(".local/bin").join(exe)).arg(file).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, size: u64, part: usize) -> FileInfo {
        FileInfo { id: id.into(), name: "a.png".into(), size, mime: "image/png".into(), part }
    }

    /// A message's file details size buffers and make addresses: nonsense
    /// from someone in the chat is refused, not fetched.
    #[test]
    fn only_details_the_server_could_have_given_are_fetched() {
        assert!(fetchable(&info("Ab_9-xY", 10, 4 << 20)));
        assert!(!fetchable(&info("Ab", 1 << 50, 1 << 50)), "a part too big to hold");
        assert!(!fetchable(&info("Ab", 10, 0)));
        assert!(!fetchable(&info("../../admin/people", 10, 4 << 20)));
        assert!(!fetchable(&info("", 10, 4 << 20)));
        assert!(!fetchable(&info("Ab", 1_000_000_000_000_000, 4 << 20)), "a file bigger than any server takes");
    }

    #[test]
    fn only_pictures_sound_and_video_show_as_themselves() {
        assert_eq!(shown_type("image/png"), "image/png");
        assert_eq!(shown_type("video/mp4"), "video/mp4");
        assert_eq!(shown_type("audio/webm;codecs=opus"), "audio/webm");
        assert_eq!(shown_type("image/svg+xml"), "application/octet-stream");
        assert_eq!(shown_type("text/html"), "application/octet-stream");
        assert_eq!(shown_type("application/xhtml+xml"), "application/octet-stream");
    }
}
