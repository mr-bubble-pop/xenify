//! Xenify Write with HTML screens. The screens are in `ui/`; they talk to
//! the documents through one command, `engine`, whose calls are the same
//! ones a browser build will answer from WebAssembly.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod allowed;
mod cli;

use allowed::Allowed;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use xenify_write_host::docs::Docs;
use xenify_write_host::proof;

/// Every call from the screens: `name` says what, `args` carries the rest.
/// Async, so it runs off the window's main thread: key presses and clicks
/// keep flowing while a document is laid out.
#[tauri::command]
async fn engine(
    state: tauri::State<'_, Mutex<Docs>>,
    allowed: tauri::State<'_, Allowed>,
    name: String,
    args: Value,
) -> Result<Value, String> {
    let trace = std::env::var_os("XENIFY_TRACE").is_some();
    let t0 = std::time::Instant::now();
    let mut docs = state.lock().map_err(|_| "Xenify Write hit a problem; please restart it".to_string())?;
    if !guard(&mut docs, &allowed, &name, &args)? {
        return Ok(Value::Null);
    }
    let out = docs.call(&name, &args);
    if trace {
        let size = out.as_ref().map_or(0, |v| v.to_string().len());
        eprintln!("call {name} {args} → {size} bytes in {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
        if std::env::var("XENIFY_TRACE").as_deref() == Ok(name.as_str()) {
            eprintln!("  = {}", out.as_ref().map(|v| v.to_string()).unwrap_or_default());
        }
    }
    out
}

/// The kinds of file a document may show from beside it (its pictures,
/// Draw canvases and Calc charts) without each being chosen.
const BESIDE: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", "xdraw", "xenify", "xcalc"];

/// `p` (relative to the front document's folder, when it's relative) as a
/// full path; None for a relative one in a document never saved.
fn resolve(docs: &mut Docs, p: &str) -> Option<PathBuf> {
    let p = Path::new(p);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    let info = docs.call("save_info", &Value::Null).ok()?;
    if info["has_path"] != true {
        return None;
    }
    Some(PathBuf::from(info["dir"].as_str()?).join(p))
}

/// Calls naming a file on this computer only go ahead for files the person
/// chose (see `allowed.rs`). Ok(false): answer "nothing" (a picture that
/// can't be shown); Err: refused, with why.
fn guard(docs: &mut Docs, allowed: &Allowed, name: &str, args: &Value) -> Result<bool, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str);
    let check = |docs: &mut Docs, p: Option<&str>, write: bool| -> Result<(), String> {
        let Some(p) = p else { return Ok(()) };
        // A Calc chart: `book.xcalc#<item>` reads the book.
        let p = p.rsplit_once('#').filter(|(b, _)| b.to_ascii_lowercase().ends_with(".xcalc")).map_or(p, |(b, _)| b);
        match resolve(docs, p) {
            Some(full) => allowed.check(&full.to_string_lossy(), write).map(|_| ()),
            // Relative in a document never saved: there's no file to reach.
            None => Ok(()),
        }
    };
    match name {
        "open" | "template_edit" | "insert_picture" | "image_link" | "merge_load" => check(docs, s("path"), false)?,
        "save" | "export" => check(docs, s("path"), true)?,
        "merge_run" => check(docs, s("dir"), true)?,
        "calc_file_snapshot" | "calc_file_picture_insert" => check(docs, s("file"), false)?,
        "asset" => {
            let Some(src) = s("src") else { return Ok(true) };
            // A picture kept inside the document.
            if docs.front_editor().doc.assets.contains_key(src) {
                return Ok(true);
            }
            let full = src.rsplit_once('#').filter(|(b, _)| b.to_ascii_lowercase().ends_with(".xcalc")).map_or(src, |(b, _)| b);
            if let Some(full) = resolve(docs, full)
                && full.exists()
                && !allowed.allows(&full, false)
            {
                return Ok(false);
            }
        }
        _ => {}
    }
    Ok(true)
}

/// The system's Open or Save window. `kind` is "open", "save" or
/// "export"; `name` is the suggested file name for saving.
#[tauri::command]
async fn choose(
    app: tauri::AppHandle,
    allowed: tauri::State<'_, Allowed>,
    kind: String,
    name: Option<String>,
    dir: Option<String>,
) -> Result<Option<String>, ()> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file();
    if let Some(dir) = dir.filter(|d| !d.is_empty()) {
        d = d.set_directory(dir);
    }
    let picked = match kind.as_str() {
        "picture" => d
            .add_filter("Pictures and Draw canvases", &["png", "jpg", "jpeg", "webp", "gif", "bmp", "xdraw", "xenify"])
            .blocking_pick_file(),
        "data" => d.add_filter("Spreadsheet or data", &["csv", "json"]).blocking_pick_file(),
        "calc" => d.add_filter("Calc books", &["xcalc"]).blocking_pick_file(),
        "folder" => d.blocking_pick_folder(),
        "open" => d
            .add_filter("Documents", &["md", "markdown", "docx", "xwrite", "igdoc", "html", "htm", "txt"])
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

/// Checks the front document's spelling and grammar (off the main thread,
/// without holding the documents while it works). The screens then ask for
/// a fresh view, which carries the underlines.
#[tauri::command]
async fn proofread(docs: tauri::State<'_, Mutex<Docs>>, proof: tauri::State<'_, proof::Proof>) -> Result<(), String> {
    let (id, req) = docs.lock().map_err(|e| e.to_string())?.proof_request();
    let texts = req.paras.clone();
    let issues = proof.check(req);
    docs.lock().map_err(|e| e.to_string())?.set_issues(id, &texts, issues);
    Ok(())
}

/// The words you've added to the dictionary.
#[tauri::command]
async fn words(proof: tauri::State<'_, proof::Proof>) -> Result<Vec<String>, String> {
    Ok(proof.words())
}

/// What happened when the mistake spotter last loaded (for Settings).
#[tauri::command]
async fn spotter_status(proof: tauri::State<'_, proof::Proof>) -> Result<String, String> {
    Ok(proof.spotter_status())
}

#[tauri::command]
async fn set_words(proof: tauri::State<'_, proof::Proof>, words: Vec<String>) -> Result<(), String> {
    proof.set_words(words);
    Ok(())
}

/// The AI assistant (see `ai.rs`).
#[tauri::command]
async fn ai(docs: tauri::State<'_, Mutex<Docs>>, name: String, args: Value) -> Result<Value, String> {
    ai::call(&docs, &name, &args)
}

/// Opens a web or email link in the computer's own browser or mail app.
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    xenify_write_host::print::open_link(&url, true)
}

/// Closes Write (after the screens have asked about unsaved work).
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// Whether see-through windows show what's behind them (a compositor is running).
fn composited() -> bool {
    #[cfg(target_os = "linux")]
    {
        gtk::gdk::Screen::default().is_some_and(|s| s.is_composited())
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    // macOS needs Apple's private window API for this.
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

/// Documents the system asked to open after start-up (macOS Finder), not yet
/// opened by the screens.
#[derive(Default)]
struct Opened(Mutex<Vec<String>>);

/// The documents waiting to be opened (taken: each is given once).
#[tauri::command]
async fn opened_files(opened: tauri::State<'_, Opened>) -> Result<Vec<String>, String> {
    Ok(opened.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default())
}

/// Links to server documents Write was started with (`xenify-write://open?
/// server=…&node=…`, from the server's Files page), not yet opened.
#[derive(Default)]
struct Linked(Mutex<Vec<String>>);

/// The links waiting to be opened (taken: each is given once).
#[tauri::command]
async fn opened_links(linked: tauri::State<'_, Linked>) -> Result<Vec<String>, String> {
    Ok(linked.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default())
}

/// A link to a server document, or a file the system passed as a file://
/// address (menu entries take addresses so links reach Write too).
fn sort_arg(a: String) -> Result<String, String> {
    if a.to_ascii_lowercase().starts_with("xenify-write://") {
        return Err(a);
    }
    match a.strip_prefix("file://") {
        Some(rest) => Ok(percent_decode(rest)),
        None => Ok(a),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

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
    std::process::Command::new(program).spawn().map_err(|e| format!("couldn't start it: {e}"))?;
    app.exit(0);
    Ok(())
}

/// A clip of the animated Draw canvas at `path`, or of the canvas `data`
/// (base64, one on a server), as WebM in base64: made by Xenify Draw
/// without a window and kept until the canvas changes. Off the documents'
/// lock: typing carries on meanwhile.
#[tauri::command]
async fn canvas_clip(allowed: tauri::State<'_, Allowed>, path: String, data: Option<String>) -> Result<String, String> {
    if data.is_none() {
        if !xenify_write_host::docs::is_canvas(Path::new(&path)) {
            return Err("that isn't a Draw canvas".into());
        }
        allowed.check(&path, false)?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        use base64::Engine;
        use std::hash::{Hash, Hasher};
        let bytes = match &data {
            Some(d) => base64::engine::general_purpose::STANDARD.decode(d).map_err(|e| e.to_string())?,
            None => std::fs::read(&path).map_err(|e| format!("couldn't read the canvas: {e}"))?,
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").filter(|_| cfg!(windows)).map(std::path::PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
            .join("xenify")
            .join("canvas-clips");
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let clip = cache.join(format!("{:016x}.webm", h.finish()));
        // A canvas from a server goes to Draw as a file beside its clip.
        let path = if data.is_some() {
            let file = clip.with_extension("xdraw");
            std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
            file.display().to_string()
        } else {
            path
        };
        if !clip.exists() {
            let draw =
                draw_program().ok_or("playing it needs Xenify Draw: install it (Xenify Hub does), then try again")?;
            let part = clip.with_extension("part.webm");
            let mut cmd = std::process::Command::new(&draw);
            cmd.arg("--export-video");
            // A canvas from a server never plays files from this computer.
            if data.is_some() {
                cmd.arg("--no-linked");
            }
            let out = cmd
                .arg(&path)
                .arg(&part)
                .output()
                .map_err(|e| format!("couldn't start Xenify Draw: {e}"))?;
            if !out.status.success() {
                let why = String::from_utf8_lossy(&out.stderr);
                return Err(format!("Xenify Draw couldn't make the clip: {}", why.trim().lines().last().unwrap_or("")));
            }
            std::fs::rename(&part, &clip).map_err(|e| e.to_string())?;
        }
        if data.is_some() {
            let _ = std::fs::remove_file(&path);
        }
        let out = std::fs::read(&clip).map_err(|e| e.to_string())?;
        Ok(base64::engine::general_purpose::STANDARD.encode(out))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Xenify Draw: `$XENIFY_DRAW`, the one Xenify Hub installed, one beside
/// Write, or one on the PATH.
fn draw_program() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("XENIFY_DRAW").map(std::path::PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let data = std::env::var_os("XENIFY_DATA_DIR").map(std::path::PathBuf::from).or_else(|| {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").filter(|_| cfg!(windows)).map(std::path::PathBuf::from))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| {
                    if cfg!(target_os = "macos") {
                        std::path::PathBuf::from(h).join("Library/Application Support")
                    } else {
                        std::path::PathBuf::from(h).join(".local/share")
                    }
                })
            })?;
        Some(base.join("xenify"))
    });
    if let Some(p) =
        data.and_then(|d| xenify_hub_core::Store::new(d.join("hub")).program("draw")).filter(|p| p.is_file())
    {
        return Some(p);
    }
    let name = if cfg!(windows) { "xenify-draw.exe" } else { "xenify-draw" };
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))).filter(|p| p.is_file())
    {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file()))
}

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).is_some_and(|a| cli::COMMANDS.contains(&a.as_str())) {
        return cli::run();
    }
    let (files, links): (Vec<_>, Vec<_>) =
        std::env::args().skip(1).filter(|a| !a.starts_with('-')).map(sort_arg).partition(Result::is_ok);
    let files: Vec<String> = files.into_iter().flatten().collect();
    let links: Vec<String> = links.into_iter().filter_map(Result::err).collect();
    // Without a desktop session bus GTK can't reach dconf and warns on every
    // settings change; it keeps them in memory instead (Write doesn't use them).
    #[cfg(target_os = "linux")]
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && std::env::var_os("GSETTINGS_BACKEND").is_none() {
        // SAFETY: set before any thread is started.
        unsafe { std::env::set_var("GSETTINGS_BACKEND", "memory") };
    }
    // The dictionary loads in the background, ready for the first Look up.
    xenify_wp_check::lexicon::preload();
    let allowed = Allowed::new(BESIDE);
    for f in &files {
        allowed.grant(std::path::absolute(f).unwrap_or_else(|_| PathBuf::from(f)));
    }
    let docs = Docs::new(&files);
    // Recent documents and the ones open last time stay usable.
    for f in docs.prefs.recent.iter().chain(&docs.prefs.open_files) {
        allowed.grant(f);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(docs))
        .manage(allowed)
        .manage(proof::Proof::new())
        .manage(Opened::default())
        .manage(Linked(Mutex::new(links)))
        .setup(|app| {
            // The mistake spotter model installed with the app.
            use tauri::Manager;
            if let Ok(dir) = app.path().resource_dir() {
                xenify_wp_check::set_bundled_spotter_dir(dir.join("spotter"));
            }
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<Allowed>().keep_at(dir.join("allowed-files.json"));
            }
            // Or next to the program (the unpacked .tar.gz, which has no resources folder).
            cli::set_bundled_spotter_dir();
            // No system border; rounded corners only where the desktop draws
            // see-through windows (without a compositor they'd be black).
            let rounded = composited();
            // XENIFY_WRITE_TEST=file.js: a script run once the screens are up (for testing the desktop build).
            let test = std::env::var("XENIFY_WRITE_TEST")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            let init = format!(
                "{}{}",
                if rounded { "window.XENIFY_ROUNDED = true;" } else { "" },
                if test.is_empty() {
                    String::new()
                } else {
                    format!("window.XENIFY_TEST = async (t) => {{ {test} }};")
                }
            );
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Xenify Write")
                .inner_size(1400.0, 900.0)
                .min_inner_size(760.0, 520.0)
                .decorations(false)
                .transparent(rounded)
                .initialization_script(&init)
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            use tauri::{Emitter, Manager};
            // Files dropped on the window may be used.
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                for p in paths {
                    window.state::<Allowed>().grant(p);
                }
            }
            // Unsaved work: the screens ask first, then call `quit`.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let dirty = window.state::<Mutex<Docs>>().lock().map(|d| d.any_dirty()).unwrap_or(false);
                if dirty {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            canvas_clip,
            update_ready,
            restart_updated,
            discover,
            certificate,
            hub_server,
            engine,
            choose,
            quit,
            proofread,
            words,
            set_words,
            spotter_status,
            ai,
            open_link,
            opened_files,
            opened_links
        ])
        .build(tauri::generate_context!())
        .expect("Xenify Write failed to start")
        .run(|_app, _event| {
            // macOS hands over documents opened from the Finder, and links
            // to server documents (whether Write is starting or running), as
            // an event.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                use tauri::{Emitter, Manager};
                let (links, files): (Vec<_>, Vec<_>) = urls.iter().partition(|u| u.scheme() == "xenify-write");
                let paths: Vec<String> = files
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                // Kept until the screens ask (they may not be loaded yet), and told now.
                if !paths.is_empty() {
                    for p in &paths {
                        _app.state::<Allowed>().grant(p);
                    }
                    if let Ok(mut q) = _app.state::<Opened>().0.lock() {
                        q.extend(paths.iter().cloned());
                    }
                    let _ = _app.emit("open-files", ());
                }
                if !links.is_empty() {
                    if let Ok(mut q) = _app.state::<Linked>().0.lock() {
                        q.extend(links.iter().map(|u| u.to_string()));
                    }
                    let _ = _app.emit("open-links", ());
                }
            }
        });
    std::process::ExitCode::SUCCESS
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
    use super::sort_arg;

    #[test]
    fn links_are_kept_apart_from_files() {
        assert_eq!(sort_arg("notes.xwrite".into()), Ok("notes.xwrite".into()));
        assert_eq!(sort_arg("file:///home/sam/My%20notes.xwrite".into()), Ok("/home/sam/My notes.xwrite".into()));
        let link = "xenify-write://open?server=https%3A%2F%2Foffice.example.org&node=7";
        assert_eq!(sort_arg(link.into()), Err(link.into()));
        assert_eq!(sort_arg("file:///a%2".into()), Ok("/a%2".into()));
    }
}
