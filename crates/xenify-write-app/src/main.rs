//! Xenify Write with HTML screens. The screens are in `ui/`; they talk to
//! the documents through one command, `engine`, whose calls are the same
//! ones a browser build will answer from WebAssembly.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod cli;

use xenify_write_host::docs::Docs;
use xenify_write_host::proof;
use serde_json::Value;
use std::sync::Mutex;

/// Every call from the screens: `name` says what, `args` carries the rest.
/// Async, so it runs off the window's main thread: key presses and clicks
/// keep flowing while a document is laid out.
#[tauri::command]
async fn engine(state: tauri::State<'_, Mutex<Docs>>, name: String, args: Value) -> Result<Value, String> {
    let trace = std::env::var_os("XENIFY_TRACE").is_some();
    let t0 = std::time::Instant::now();
    let mut docs = state.lock().map_err(|_| "Xenify Write hit a problem; please restart it".to_string())?;
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

/// The system's Open or Save window. `kind` is "open", "save" or
/// "export"; `name` is the suggested file name for saving.
#[tauri::command]
async fn choose(app: tauri::AppHandle, kind: String, name: Option<String>, dir: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file();
    if let Some(dir) = dir.filter(|d| !d.is_empty()) {
        d = d.set_directory(dir);
    }
    let picked = match kind.as_str() {
        "picture" => d.add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).blocking_pick_file(),
        "data" => d.add_filter("Spreadsheet or data", &["csv", "json"]).blocking_pick_file(),
        "folder" => d.blocking_pick_folder(),
        "open" => d
            .add_filter("Documents", &["md", "markdown", "docx", "igdoc", "html", "htm", "txt"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        _ => {
            if let Some(n) = name {
                d = d.set_file_name(n);
            }
            d.blocking_save_file()
        }
    };
    picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
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
    let ok = ["https://", "http://", "mailto:"].iter().any(|p| url.to_ascii_lowercase().starts_with(p));
    if !ok {
        return Err("only web and email links open from here".into());
    }
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
    r.map(|_| ()).map_err(|e| format!("couldn't open the link: {e}"))
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

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).is_some_and(|a| cli::COMMANDS.contains(&a.as_str())) {
        return cli::run();
    }
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    // Without a desktop session bus GTK can't reach dconf and warns on every
    // settings change; it keeps them in memory instead (Write doesn't use them).
    #[cfg(target_os = "linux")]
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && std::env::var_os("GSETTINGS_BACKEND").is_none() {
        // SAFETY: set before any thread is started.
        unsafe { std::env::set_var("GSETTINGS_BACKEND", "memory") };
    }
    // The dictionary loads in the background, ready for the first Look up.
    xenify_wp_check::lexicon::preload();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(Docs::new(&files)))
        .manage(proof::Proof::new())
        .manage(Opened::default())
        .setup(|app| {
            // The mistake spotter model installed with the app.
            use tauri::Manager;
            if let Ok(dir) = app.path().resource_dir() {
                xenify_wp_check::set_bundled_spotter_dir(dir.join("spotter"));
            }
            // Or next to the program (the unpacked .tar.gz, which has no resources folder).
            cli::set_bundled_spotter_dir();
            // No system border; rounded corners only where the desktop draws
            // see-through windows (without a compositor they'd be black).
            let rounded = composited();
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Xenify Write")
                .inner_size(1400.0, 900.0)
                .min_inner_size(760.0, 520.0)
                .decorations(false)
                .transparent(rounded)
                .initialization_script(if rounded { "window.XENIFY_ROUNDED = true;" } else { "" })
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            use tauri::{Emitter, Manager};
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
            engine,
            choose,
            quit,
            proofread,
            words,
            set_words,
            spotter_status,
            ai,
            open_link,
            opened_files
        ])
        .build(tauri::generate_context!())
        .expect("Xenify Write failed to start")
        .run(|_app, _event| {
            // macOS hands over documents opened from the Finder as an event.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                use tauri::{Emitter, Manager};
                let paths: Vec<String> = urls
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                // Kept until the screens ask (they may not be loaded yet), and told now.
                if let Ok(mut q) = _app.state::<Opened>().0.lock() {
                    q.extend(paths.iter().cloned());
                }
                let _ = _app.emit("open-files", ());
            }
        });
    std::process::ExitCode::SUCCESS
}
