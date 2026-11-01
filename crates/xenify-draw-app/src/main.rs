#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Xenify Draw on the desktop: the screens in ui/ and the engine (as
//! WebAssembly, drawing with WebGL2) run in the system's web view, exactly
//! as in the browser. This side does what a web page can't: file dialogs,
//! reading and writing files, and spelling and grammar with Xenify Write's
//! checker (and its mistake-spotter model, when installed).

use allowed::Allowed;
use base64::Engine as _;
use serde_json::Value;
use std::sync::Mutex;
use xenify_write_host::proof;

/// Which files the screens may use (shared with Write, Calc and Plan).
#[path = "../../xenify-write-app/src/allowed.rs"]
mod allowed;

/// The picture on the system clipboard (the web view can't read it by itself): raw pixels
/// (`rgba`, straight alpha, with its size), an image file's bytes (`bytes`), or nothing.
#[tauri::command]
async fn clipboard_picture() -> Option<Value> {
    use xenify_platform::desktop::{ClipContent, Clipboard as _, native::SystemClipboard};
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    // A picture copied in a browser also comes as HTML naming where it's from: an animated GIF is
    // fetched from there (the pixels copied with it are only its first frame).
    let html = SystemClipboard::read_mime("text/html").map(|b| String::from_utf8_lossy(&b).into_owned());
    if let Some(src) = html.as_deref().and_then(img_src).filter(|u| u.contains(".gif") || u.contains("giphy.com")) {
        return Some(serde_json::json!({ "url": src }));
    }
    match SystemClipboard.read()? {
        ClipContent::Image { width, height, rgba } => {
            Some(serde_json::json!({ "width": width, "height": height, "rgba": b64(&rgba) }))
        }
        ClipContent::Encoded { bytes } => Some(serde_json::json!({ "bytes": b64(&bytes) })),
        // A picture file copied in the file manager.
        ClipContent::Files(files) => {
            let f = files.into_iter().find(|f| {
                f.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    ["png", "jpg", "jpeg", "webp", "gif", "bmp"].contains(&e.to_ascii_lowercase().as_str())
                })
            })?;
            let bytes = std::fs::read(&f).ok()?;
            Some(
                serde_json::json!({ "bytes": b64(&bytes), "name": f.file_name().map(|n| n.to_string_lossy().into_owned()) }),
            )
        }
        _ => None,
    }
}

/// The first `<img src="…">` in some HTML.
fn img_src(html: &str) -> Option<String> {
    let at = html.find("<img")?;
    let rest = &html[at..];
    let s = rest.find("src=")? + 4;
    let q = rest[s..].chars().next()?;
    let body = &rest[s + 1..];
    let end = body.find(q)?;
    Some(body[..end].replace("&amp;", "&"))
}

/// Downloads a picture (HTTPS only, from the internet: not this computer or
/// its network; at most 40 MB): its bytes, as base64.
#[tauri::command]
async fn fetch_picture(url: String) -> Result<String, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || download_picture(&url, 40 << 20))
        .await
        .map_err(|e| e.to_string())??;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Whether `ip` is on the internet (not this computer, its network, or
/// an address kept for other uses).
fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v) => {
            let [a, b, ..] = v.octets();
            !(v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_documentation()
                || v.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v.segments()[0];
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || first == 0x2001 && v.segments()[1] == 0xdb8
                || first == 0x64 && v.segments()[1] == 0xff9b)
        }
    }
}

/// The host of an `https://` address, after checking that every address it
/// has is on the internet.
fn public_https(url: &str) -> Result<(), String> {
    use std::net::ToSocketAddrs;
    let rest = url.strip_prefix("https://").ok_or("only pictures from https:// addresses can be fetched")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = match host_port.strip_prefix('[') {
        Some(r) => {
            let (h, p) = r.split_once(']').ok_or("that isn't a web address")?;
            (h, p.strip_prefix(':').map_or(Ok(443), str::parse).map_err(|_| "that isn't a web address")?)
        }
        None => match host_port.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().map_err(|_| "that isn't a web address")?),
            None => (host_port, 443),
        },
    };
    if host.is_empty() {
        return Err("that isn't a web address".into());
    }
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|e| format!("couldn't find {host}: {e}"))?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| !is_public(a.ip())) {
        return Err("pictures from this computer or its network can't be fetched".into());
    }
    Ok(())
}

/// `url`'s bytes (at most `max`), following up to 5 redirects, each to a
/// public HTTPS address.
fn download_picture(url: &str, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .max_redirects(0)
        .build()
        .into();
    let mut at = url.to_string();
    for _ in 0..6 {
        let shown = at.split('?').next().unwrap_or(&at).to_string();
        public_https(&at)?;
        let mut resp = agent.get(&at).call().map_err(|e| format!("couldn't download {shown}: {e}"))?;
        if resp.status().is_redirection() {
            let to = resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("a redirect to nowhere")?;
            at = if to.starts_with("https://") || to.starts_with("http://") {
                to.to_string()
            } else if to.starts_with('/') && !to.starts_with("//") {
                let rest = &at["https://".len()..];
                let host = rest.split(['/', '?', '#']).next().unwrap_or("");
                format!("https://{host}{to}")
            } else {
                return Err(format!("couldn't download {shown}: an odd redirect"));
            };
            continue;
        }
        let mut bytes = Vec::new();
        resp.body_mut()
            .as_reader()
            .take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("couldn't download {shown}: {e}"))?;
        if bytes.len() as u64 > max {
            return Err(format!("{shown}: bigger than {} MB", max >> 20));
        }
        return Ok(bytes);
    }
    Err("too many redirects".into())
}

/// Asks for a file: `open` (a canvas), `picture`, or anything else to save
/// (`name` suggests the file name).
#[tauri::command]
async fn choose(
    app: tauri::AppHandle,
    allowed: tauri::State<'_, Allowed>,
    kind: String,
    name: Option<String>,
) -> Result<Option<String>, ()> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file();
    let picked = match kind.as_str() {
        "open" => d
            .add_filter("Xenify Draw canvases", &["xdraw", "xenify"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        "model" => d.add_filter("3D models", &["stl", "obj", "gltf", "glb", "3mf"]).blocking_pick_file(),
        "video" => d.add_filter("Video", &["mp4", "webm", "mov", "m4v", "mkv", "avi"]).blocking_pick_file(),
        "sound" => {
            d.add_filter("Sound", &["wav", "mp3", "ogg", "oga", "opus", "flac", "m4a", "aac"]).blocking_pick_file()
        }
        "picture" => {
            d.add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"]).blocking_pick_file()
        }
        "import" => d
            .add_filter(
                "Pictures, SVG, Lottie and sound",
                &[
                    "svg", "json", "lottie", "png", "jpg", "jpeg", "webp", "gif", "bmp", "wav", "mp3", "ogg", "oga",
                    "opus", "flac", "m4a", "aac", "mp4", "webm", "mov", "m4v", "mkv", "avi", "stl", "obj", "gltf",
                    "glb", "3mf",
                ],
            )
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        _ => {
            if let Some(n) = name {
                d = d.set_file_name(n);
            }
            d.blocking_save_file()
        }
    };
    let picked = picked.and_then(|p| p.into_path().ok());
    if let Some(p) = &picked {
        allowed.grant(p);
    }
    Ok(picked.map(|p| p.display().to_string()))
}

/// Whether `path` is among the files copied in the file manager (pasting
/// them into Draw reads them).
fn on_clipboard(path: &str) -> bool {
    use xenify_platform::desktop::{ClipContent, Clipboard as _, native::SystemClipboard};
    let want = allowed::canon(std::path::Path::new(path));
    let files = match SystemClipboard.read() {
        Some(ClipContent::Files(f)) => f,
        _ => SystemClipboard::read_mime("text/uri-list")
            .map(|b| xenify_platform::desktop::parse_uri_list(&String::from_utf8_lossy(&b)))
            .unwrap_or_default(),
    };
    want.is_some() && files.iter().any(|f| allowed::canon(f) == want)
}

/// A file's contents: text, or base64 for anything that isn't UTF-8.
#[tauri::command]
async fn read_file(allowed: tauri::State<'_, Allowed>, path: String) -> Result<Value, String> {
    if !allowed.allows(std::path::Path::new(&path), false) {
        if !on_clipboard(&path) {
            allowed.check(&path, false)?;
        }
        allowed.grant(&path);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("couldn't open {path}: {e}"))?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => serde_json::json!({ "text": text }),
        Err(e) => serde_json::json!({ "base64": base64::engine::general_purpose::STANDARD.encode(e.into_bytes()) }),
    })
}

/// Writes `text`, or the bytes in `base64`, to `path` (through a temporary
/// file, so a failed save never leaves half a file).
#[tauri::command]
async fn write_file(
    allowed: tauri::State<'_, Allowed>,
    path: String,
    text: Option<String>,
    base64: Option<String>,
) -> Result<(), String> {
    allowed.check(&path, true)?;
    let bytes = match (text, base64) {
        (Some(t), _) => t.into_bytes(),
        (None, Some(b)) => base64::engine::general_purpose::STANDARD.decode(b).map_err(|e| e.to_string())?,
        (None, None) => return Err("nothing to write".into()),
    };
    let tmp = format!("{path}.saving");
    std::fs::write(&tmp, bytes).map_err(|e| format!("couldn't save {path}: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("couldn't save {path}: {e}"))
}

/// Spelling and grammar for the given texts (the same request and answer
/// as Xenify Write's checker).
#[tauri::command]
async fn check(proof: tauri::State<'_, proof::Proof>, request: proof::Request) -> Result<Value, String> {
    serde_json::to_value(proof.check(request)).map_err(|e| e.to_string())
}

#[tauri::command]
async fn words(proof: tauri::State<'_, proof::Proof>) -> Result<Vec<String>, String> {
    Ok(proof.words())
}

#[tauri::command]
async fn set_words(proof: tauri::State<'_, proof::Proof>, words: Vec<String>) -> Result<(), String> {
    proof.set_words(words);
    Ok(())
}

#[tauri::command]
async fn look_up(word: String) -> Result<Value, String> {
    Ok(proof::look_up(&word))
}

/// Where unsaved canvases are kept while Draw runs (deleted when saved or closed).
fn autosave_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let d = app.path().app_data_dir().ok()?.join("autosave");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn safe_key(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Keeps an unsaved canvas (`text`, its tab `name`) under `key` (this run and tab).
#[tauri::command]
async fn autosave_write(app: tauri::AppHandle, key: String, name: String, text: String) -> Result<(), String> {
    let dir = autosave_dir(&app).ok_or("no place for autosave")?;
    let body = serde_json::json!({ "name": name, "text": text, "when": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) });
    let path = dir.join(format!("{}.json", safe_key(&key)));
    let tmp = path.with_extension("saving");
    std::fs::write(&tmp, body.to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn autosave_clear(app: tauri::AppHandle, key: String) {
    if let Some(dir) = autosave_dir(&app) {
        let _ = std::fs::remove_file(dir.join(format!("{}.json", safe_key(&key))));
    }
}

/// Canvases left from a run that ended without saving them: key, name, when (Unix seconds), text.
#[tauri::command]
async fn autosave_list(app: tauri::AppHandle, keep: String) -> Vec<Value> {
    let Some(dir) = autosave_dir(&app) else { return Vec::new() };
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = e.path();
        let Some(key) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        // This run's own files aren't left over.
        if path.extension().is_none_or(|x| x != "json") || key.starts_with(&safe_key(&keep)) {
            continue;
        }
        if let Ok(v) = std::fs::read_to_string(&path)
            .map_err(|_| ())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ()))
        {
            out.push(serde_json::json!({ "key": key, "name": v["name"], "when": v["when"], "text": v["text"] }));
        }
    }
    out
}

/// Video frames for the canvas, decoded by ffmpeg (WebKitGTK can't hand
/// video pixels to a page): RGBA, after an 8-byte header (width, height as
/// little-endian u32). `path` is the clip's file (linked), or one written by
/// `media_file` for clips kept inside the canvas.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn video_frame(
    frames: tauri::State<'_, Frames>,
    allowed: tauri::State<'_, Allowed>,
    hash: String,
    path: String,
    mime: String,
    fps: f64,
    width: u32,
    height: u32,
    duration: f64,
    t: f64,
) -> Result<tauri::ipc::Response, String> {
    use xenify_anim::media::FrameSource;
    allowed.check(&path, false)?;
    let media = xenify_anim::media::MediaRef {
        asset: xenify_doc::Id(0),
        hash: hash.parse().unwrap_or(0),
        mime,
        path: Some(path),
        data: None,
        fps,
        width,
        height,
        duration,
    };
    let f = frames.0.lock().map_err(|e| e.to_string())?.frame(&media, t).ok_or("couldn't read that frame")?;
    let mut out = Vec::with_capacity(8 + f.pixels.len());
    out.extend_from_slice(&f.width.to_le_bytes());
    out.extend_from_slice(&f.height.to_le_bytes());
    out.extend_from_slice(&f.pixels);
    Ok(tauri::ipc::Response::new(out))
}

/// A video or sound file's size, length, frame rate and whether it has sound, read by ffprobe
/// (any format ffmpeg reads: AVI, MKV, MOV…).
#[tauri::command]
async fn probe_media(allowed: tauri::State<'_, Allowed>, path: String) -> Result<serde_json::Value, String> {
    allowed.check(&path, false)?;
    let i = xenify_platform::media::probe(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({ "width": i.width, "height": i.height, "duration": i.duration, "fps": i.fps, "has_audio": i.has_audio }),
    )
}

/// The folder of this account's own where clips kept inside canvases are
/// written for ffmpeg to read.
fn media_dir() -> Result<std::path::PathBuf, String> {
    xenify_platform::media::private_dir("draw-media").ok_or_else(|| "there's no folder for temporary files".into())
}

/// Writes a clip kept inside the canvas to a temporary file (kept while it
/// holds the same bytes), for ffmpeg to read.
#[tauri::command]
async fn media_file(hash: String, ext: String, base64: String) -> Result<String, String> {
    let dir = media_dir()?;
    let name = format!(
        "{}.{}",
        hash.chars().filter(|c| c.is_ascii_alphanumeric()).take(64).collect::<String>(),
        ext.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>()
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64).map_err(|e| e.to_string())?;
    let path = xenify_platform::media::write_once(&dir, &name, &bytes).ok_or("couldn't keep the clip for playing")?;
    Ok(path.display().to_string())
}

#[derive(Default)]
struct Frames(Mutex<xenify_platform::media::FfmpegFrames>);

static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Exports the canvas (`project` as saved, canvas `comp`) to `path` here, the
/// way the egui app does: video (mp4, webm), GIF, PNG frames (a folder) or
/// PDF, rendered offscreen and encoded by ffmpeg. Progress goes to the
/// window as `export-progress` events (`[done, total]`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn export_native(
    window: tauri::Window,
    allowed: tauri::State<'_, Allowed>,
    project: String,
    comp: u64,
    path: String,
    format: String,
    scale: f64,
    transparent: bool,
    sound: bool,
    time: f64,
    size: Option<(u32, u32)>,
    letterbox: Option<String>,
    deck: Option<bool>,
    slides: Option<(usize, usize)>,
    notes: Option<bool>,
) -> Result<(), String> {
    use tauri::Emitter;
    use xenify_export::{ExportOptions, export_deck_to_file, export_to_file};
    allowed.check(&path, true)?;
    CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    let project = xenify_doc::Project::from_json(&project).map_err(|e| e.to_string())?;
    let comp = xenify_doc::Id(comp);
    let letterbox = letterbox.and_then(|c| xenify_doc::Color::from_hex(&c)).unwrap_or(xenify_doc::Color::BLACK);
    tauri::async_runtime::spawn_blocking(move || {
        // Sound and video frames for the export, decoded by ffmpeg (as in the egui app).
        xenify_platform::media::install();
        let mut gpu = xenify_render_offscreen()?;
        if format == "pdf" {
            let options =
                xenify_export::pdf::PdfOptions { scale: scale.max(0.25) * 2.0, notes: notes.unwrap_or(false) };
            let bytes = xenify_export::pdf::export_pdf(&project, comp, time, options, &mut gpu)?;
            return std::fs::write(&path, bytes).map_err(|e| format!("couldn't save {path}: {e}"));
        }
        // Every slide, finished, as its own PNG: `path` is the first one's name (…-1.png).
        if format == "slides_png" {
            let total = project.slides.len() as u64;
            let base = path.strip_suffix(".png").unwrap_or(&path).to_string();
            for (i, sl) in project.slides.iter().enumerate() {
                let c = project.comp(sl.comp).map_err(|e| e.to_string())?;
                let (w, h) = size.unwrap_or_else(|| xenify_export::output_size(c, scale));
                let img = gpu
                    .render_frame(&project, sl.comp, c.settings.duration, w, h, transparent)
                    .map_err(|e| e.to_string())?;
                std::fs::write(format!("{base}-{}.png", i + 1), img.to_png()).map_err(|e| e.to_string())?;
                let _ = window.emit("export-progress", [i as u64 + 1, total]);
                if CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("cancelled".into());
                }
            }
            return Ok(());
        }
        let format = match format.as_str() {
            "mp4" => xenify_export::Format::Mp4,
            "webm" => xenify_export::Format::WebM,
            "gif" => xenify_export::Format::Gif,
            "png" => xenify_export::Format::PngSequence,
            f => return Err(format!("no export format {f}")),
        };
        let options = ExportOptions {
            format,
            scale: scale.clamp(0.1, 8.0),
            transparent,
            with_sound: sound,
            size,
            letterbox,
            slides,
        };
        let mut last = std::time::Instant::now();
        let mut progress = |done: u64, total: u64| {
            if last.elapsed().as_millis() > 100 || done == total {
                last = std::time::Instant::now();
                let _ = window.emit("export-progress", [done, total]);
            }
            !CANCEL.load(std::sync::atomic::Ordering::SeqCst)
        };
        if deck.unwrap_or(false) {
            return export_deck_to_file(&project, std::path::Path::new(&path), &options, &mut gpu, &mut progress)
                .map_err(|e| e.to_string());
        }
        export_to_file(&project, comp, std::path::Path::new(&path), &options, &mut gpu, &mut progress)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn xenify_render_offscreen() -> Result<xenify_export::Offscreen, String> {
    xenify_export::Offscreen::new().map_err(|e| format!("couldn't start the GPU renderer: {e}"))
}

#[tauri::command]
fn export_cancel() {
    CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// The printers Draw can print to directly (CUPS), default first, or why it can't.
#[tauri::command]
async fn printers() -> (Option<String>, Vec<String>) {
    use xenify_platform::desktop::Printer;
    let p = xenify_platform::desktop::native::CupsPrinter;
    match p.unavailable() {
        Some(why) => (Some(why), Vec::new()),
        None => (None, p.printers()),
    }
}

/// Prints a PNG (the frame, rendered by the page) on `printer`, `copies` times.
#[tauri::command]
async fn print_png(base64: String, printer: Option<String>, copies: u32) -> Result<(), String> {
    use base64::Engine as _;
    use xenify_platform::desktop::Printer;
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64).map_err(|e| e.to_string())?;
    let dir = xenify_platform::media::private_dir("print").ok_or("there's no folder for temporary files")?;
    let name = format!("draw-print-{}.png", std::process::id());
    let path = xenify_platform::media::write_once(&dir, &name, &bytes).ok_or("couldn't keep the picture for printing")?;
    let opts = xenify_platform::desktop::PrintOptions { printer, copies: copies.clamp(1, 99) };
    // lp has taken a copy once it returns.
    let r = xenify_platform::desktop::native::CupsPrinter.print_file(&path, &opts).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    r
}

/// Texts shipped inside the program: the licences.
#[tauri::command]
fn bundled(name: String) -> Result<tauri::ipc::Response, String> {
    let text: &str = match name.as_str() {
        "LICENSE.txt" => include_str!("../../../LICENSE"),
        "THIRD_PARTY_LICENSES.txt" => include_str!("../../../THIRD_PARTY_LICENSES.txt"),
        _ => return Err(format!("{name} isn't shipped with Draw")),
    };
    Ok(tauri::ipc::Response::new(text.as_bytes().to_vec()))
}

/// The computer's fonts, read once (the page's engine only has Inter built in).
fn system_text() -> &'static std::sync::Mutex<xenify_render::TextEngine> {
    static T: std::sync::OnceLock<std::sync::Mutex<xenify_render::TextEngine>> = std::sync::OnceLock::new();
    T.get_or_init(|| std::sync::Mutex::new(xenify_render::TextEngine::new()))
}

/// The font families installed on this computer.
#[tauri::command]
async fn system_fonts() -> Vec<String> {
    system_text().lock().map(|mut t| t.families()).unwrap_or_default()
}

/// The files of font family `family`: each as a 4-byte length (little-endian) and its bytes.
#[tauri::command]
async fn font_files(family: String) -> tauri::ipc::Response {
    let files = system_text().lock().map(|mut t| t.family_files(&family)).unwrap_or_default();
    let mut out = Vec::new();
    for f in files {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(&f);
    }
    tauri::ipc::Response::new(out)
}

/// Whether ffmpeg is installed (video, GIF and PNG frames need it).
#[tauri::command]
async fn has_ffmpeg() -> bool {
    xenify_hub_core::command("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Script errors and the frame-time log go to the terminal Draw was started from.
#[tauri::command]
fn log(text: String) {
    eprintln!("{text}");
}

/// Files named on the command line (opened once the screens are ready).
#[tauri::command]
fn opened_files(state: tauri::State<'_, Opened>) -> Vec<String> {
    state.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}


/// Shows `path` in the system's file manager (its folder, with it picked where the system can).
#[tauri::command]
fn reveal(allowed: tauri::State<'_, Allowed>, path: String) -> Result<(), String> {
    allowed.check(&path, false)?;
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("{path} isn't there any more"));
    }
    #[cfg(target_os = "windows")]
    let r = xenify_hub_core::command("explorer").arg(format!("/select,{path}")).spawn();
    #[cfg(target_os = "macos")]
    let r = xenify_hub_core::command("open").arg("-R").arg(&path).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let r = xenify_hub_core::command("xdg-open").arg(p.parent().unwrap_or(p)).spawn();
    r.map(|_| ()).map_err(|e| format!("Couldn't open the folder: {e}"))
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

fn composited() -> bool {
    #[cfg(target_os = "linux")]
    {
        gtk::gdk::Screen::default().is_some_and(|s| s.is_composited())
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

#[derive(Default)]
struct Opened(Mutex<Vec<String>>);

/// Xenify Hub put a newer version of this app in place since it started:
/// that version (Help › Restart to use it), or null.
#[tauri::command]
fn update_ready() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    xenify_hub_core::install::newer_installed(&exe).map(|(v, _)| v)
}

/// Starts the newer version and closes this one (after the screens have
/// asked about unsaved work).
#[tauri::command]
fn restart_updated(app: tauri::AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (_, program) = xenify_hub_core::install::newer_installed(&exe).ok_or("there's no newer version")?;
    xenify_hub_core::command(program).spawn().map_err(|e| format!("couldn't start it: {e}"))?;
    app.exit(0);
    Ok(())
}

/// `xenify-draw --export-video [--no-linked] <canvas> <clip.webm>`: a clip
/// of the canvas (at most 720 pixels across), made without a window. Xenify
/// Write plays an animated canvas shown in a document this way;
/// `--no-linked` (a canvas from a server) leaves out clips linked to files.
fn export_video(canvas: Option<&String>, out: Option<&String>, no_linked: bool) -> Result<(), String> {
    let (canvas, out) = (canvas.ok_or("which canvas?")?, out.ok_or("where to put the clip?")?);
    let bytes = std::fs::read(canvas).map_err(|e| format!("{canvas}: {e}"))?;
    let mut project = xenify_doc::file::unpack(&bytes)?;
    // A canvas from a server: clips linked to files aren't played (they'd
    // be files on this computer that it names).
    if no_linked {
        drop_linked(&mut project);
    }
    let comp = project.main_comp().ok_or("the canvas is empty")?;
    let (id, w, h) = (comp.id, comp.settings.width as f64, comp.settings.height as f64);
    let options = xenify_export::ExportOptions {
        format: xenify_export::Format::WebM,
        scale: (720.0 / w.max(h)).min(1.0),
        transparent: false,
        with_sound: false,
        ..xenify_export::ExportOptions::default()
    };
    xenify_platform::media::install();
    let mut gpu = xenify_render_offscreen()?;
    xenify_export::export_to_file(&project, id, std::path::Path::new(out), &options, &mut gpu, &mut |_, _| true)
        .map_err(|e| e.to_string())
}

/// Clips linked to files become empty ones (shown blank, silent).
fn drop_linked(project: &mut xenify_doc::Project) {
    for m in &mut project.media {
        if matches!(m.source, xenify_doc::MediaSource::Linked { .. }) {
            m.source = xenify_doc::MediaSource::Embedded { data: std::sync::Arc::from(Vec::new()) };
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--export-video") {
        let no_linked = args.get(2).map(String::as_str) == Some("--no-linked");
        let at = if no_linked { 3 } else { 2 };
        std::process::exit(match export_video(args.get(at), args.get(at + 1), no_linked) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("xenify-draw: {e}");
                1
            }
        });
    }
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let allowed = Allowed::new(&[]);
    for f in &files {
        allowed.grant(std::path::absolute(f).unwrap_or_else(|_| f.into()));
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && std::env::var_os("GSETTINGS_BACKEND").is_none() {
        // SAFETY: set before any thread is started.
        unsafe { std::env::set_var("GSETTINGS_BACKEND", "memory") };
    }
    xenify_wp_check::lexicon::preload();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(proof::Proof::new())
        .manage(Opened(Mutex::new(files)))
        .manage(Frames::default())
        .manage(allowed)
        .setup(|app| {
            use tauri::Manager;
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<Allowed>().keep_at(dir.join("allowed-files.json"));
            }
            if let Ok(dir) = media_dir() {
                app.state::<Allowed>().add_root(&dir);
            }
            // The mistake spotter model installed with the app.
            if let Ok(dir) = app.path().resource_dir() {
                xenify_wp_check::set_bundled_spotter_dir(dir.join("spotter"));
            }
            let rounded = composited();
            // XENIFY_DRAW_BENCH=1: run the speed test once the screens are up (its result goes to the terminal).
            let bench = std::env::var_os("XENIFY_DRAW_BENCH").is_some();
            // XENIFY_DRAW_TEST=file.js: a script run once the screens are up (for testing the desktop build).
            let test = std::env::var("XENIFY_DRAW_TEST")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            let init = format!(
                "{}{}{}",
                if rounded { "window.XENIFY_ROUNDED = true;" } else { "" },
                if bench { "window.XENIFY_BENCH = true;" } else { "" },
                if test.is_empty() { String::new() } else { format!("window.XENIFY_TEST = async () => {{ {test} }};") }
            );
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Xenify Draw")
                .inner_size(1440.0, 900.0)
                .min_inner_size(900.0, 600.0)
                .decorations(false)
                .transparent(rounded)
                .initialization_script(&init)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_hub,
            update_ready,
            restart_updated,
            discover,
            certificate,
            hub_server,
            choose,
            clipboard_picture,
            fetch_picture,
            system_fonts,
            probe_media,
            printers,
            print_png,
            bundled,
            font_files,
            read_file,
            write_file,
            check,
            words,
            set_words,
            look_up,
            log,
            opened_files,
            quit,
            reveal,
            export_native,
            export_cancel,
            has_ffmpeg,
            autosave_write,
            autosave_clear,
            autosave_list,
            video_frame,
            media_file
        ])
        .run(tauri::generate_context!())
        .expect("Xenify Draw failed to start");
}

/// A server's own HTTPS certificate (servers without a domain name): what
/// it is and whether it's trusted here, or (with `fingerprint`, what the
/// person was shown) trusting it from now on (`remember`) or this time.
/// The web view then accepts it.
#[tauri::command]
async fn certificate(
    window: tauri::WebviewWindow,
    url: String,
    fingerprint: Option<String>,
    remember: bool,
) -> Result<xenify_platform::trust::Seen, String> {
    let seen = tauri::async_runtime::spawn_blocking(move || match fingerprint {
        Some(f) => xenify_platform::trust::trust(&url, &f, remember),
        None => xenify_platform::trust::look(&url),
    })
    .await
    .map_err(|e| e.to_string())??;
    #[cfg(target_os = "linux")]
    if seen.trusted || seen.ignored {
        let s = seen.clone();
        let _ = window.with_webview(move |w| xenify_platform::trust::allow(&w.inner(), &s));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = window;
    Ok(seen)
}

/// Xenify servers on this network, for Settings › Server.
#[tauri::command]
async fn discover() -> Vec<xenify_platform::discover::Found> {
    tauri::async_runtime::spawn_blocking(|| xenify_platform::discover::servers(std::time::Duration::from_secs(3)))
        .await
        .unwrap_or_default()
}

/// The Hub's server sign-in, which the app follows (unless set to use a
/// different server): the address and the session.
#[tauri::command]
async fn hub_server() -> Option<xenify_hub_core::signin::SignIn> {
    tauri::async_runtime::spawn_blocking(xenify_hub_core::signin::read).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_only_from_the_internet() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.56.107", "172.16.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:4700::1111", "151.101.1.1"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for url in ["http://example.org/a.gif", "https://127.0.0.1/a.gif", "https://[::1]:8080/a.gif", "https://user@localhost/a.gif", "file:///etc/passwd", "https:///a.gif"] {
            assert!(public_https(url).is_err(), "{url}");
        }
    }

    #[test]
    fn server_canvases_lose_linked_clips() {
        let mut p = xenify_doc::Project::default();
        p.media.push(xenify_doc::MediaAsset {
            id: xenify_doc::Id(1),
            name: "clip".into(),
            kind: xenify_doc::MediaKind::Video,
            mime: "video/mp4".into(),
            source: xenify_doc::MediaSource::Linked { path: "/home/sam/secret.mp4".into() },
            duration: 1.0,
            width: 2,
            height: 2,
            fps: 1.0,
            has_audio: false,
            hash: 1,
            recipe: None,
        });
        drop_linked(&mut p);
        assert!(matches!(p.media[0].source, xenify_doc::MediaSource::Embedded { .. }));
    }
}

/// Not signed in to a server: Xenify Hub signs every app in, so the app
/// opens it (or brings the running one forward) instead of asking itself.
#[tauri::command]
fn open_hub() -> Result<(), String> {
    xenify_hub_core::signin::open_hub()
}
