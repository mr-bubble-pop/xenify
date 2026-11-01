#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Xenify Calc on the desktop: the screens in ui/ and the engine (as
//! WebAssembly) run in the system's web view, exactly as in the browser.
//! This side does what a web page can't: file dialogs, reading and writing
//! files, and keeping unsaved books safe if Calc closes unexpectedly.

use allowed::Allowed;
use base64::Engine as _;
use serde_json::Value;
use std::sync::Mutex;

/// Which files the screens may use (shared with Write, Draw and Plan).
#[path = "../../xenify-write-app/src/allowed.rs"]
mod allowed;

/// Asks for a file: `open` (a book), or where to save one of kind `kind`
/// (xcalc, xlsx, xlsm, csv, tsv; `name` suggests the file name).
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
            .add_filter("Spreadsheets", &["xcalc", "xlsx", "xlsm", "xls", "ods", "csv", "tsv", "txt", "psv", "tab"])
            .add_filter("Xenify Calc books", &["xcalc"])
            .add_filter("Spreadsheets (.xlsx, .xlsm, .xls)", &["xlsx", "xlsm", "xls"])
            .add_filter("OpenDocument spreadsheets", &["ods"])
            .add_filter("Text (CSV, tab, pipe…)", &["csv", "tsv", "txt", "psv", "tab"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        // A text file to link a sheet to.
        "csv" => d
            .add_filter("Text (CSV, tab, pipe…)", &["csv", "tsv", "txt", "psv", "tab"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        _ => {
            if let Some(n) = name {
                d = d.set_file_name(n);
            }
            // The kind asked for first; the name's ending picks the kind written.
            let all: [(&str, &str); 5] = [
                ("xcalc", "Xenify Calc book"),
                ("xlsx", "Spreadsheet (.xlsx)"),
                ("xlsm", "Spreadsheet with macros (.xlsm)"),
                ("csv", "CSV (this sheet)"),
                ("tsv", "Tab-separated (this sheet)"),
            ];
            let first = all.iter().position(|(e, _)| *e == kind).unwrap_or(0);
            d = d.add_filter(all[first].1, &[all[first].0]);
            for (k, (e, label)) in all.iter().enumerate() {
                if k != first {
                    d = d.add_filter(*label, &[*e]);
                }
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

/// A file's bytes, as base64.
#[tauri::command]
async fn read_file(allowed: tauri::State<'_, Allowed>, path: String) -> Result<String, String> {
    allowed.check(&path, false)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("couldn't open {path}: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// When a file last changed and its size ("<ms since 1970>:<bytes>"), to
/// see that a linked file changed; none when it isn't there.
#[tauri::command]
async fn file_stamp(allowed: tauri::State<'_, Allowed>, path: String) -> Result<Option<String>, ()> {
    if !allowed.allows(std::path::Path::new(&path), false) {
        return Ok(None);
    }
    Ok(stamp(&path))
}

fn stamp(path: &str) -> Option<String> {
    let m = std::fs::metadata(&path).ok()?;
    let ms = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some(format!("{ms}:{}", m.len()))
}

/// Writes the bytes in `base64` to `path` (through a temporary file, so a
/// failed save never leaves half a file).
#[tauri::command]
async fn write_file(allowed: tauri::State<'_, Allowed>, path: String, base64: String) -> Result<(), String> {
    allowed.check(&path, true)?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64).map_err(|e| e.to_string())?;
    let tmp = format!("{path}.saving");
    std::fs::write(&tmp, bytes).map_err(|e| format!("couldn't save {path}: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("couldn't save {path}: {e}"))
}

/// Where unsaved books are kept while Calc runs (deleted when saved or closed).
fn autosave_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let d = app.path().app_data_dir().ok()?.join("autosave");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn safe_key(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Keeps an unsaved book (`base64`, its tab `name` and file `path`) under
/// `key` (this run and tab).
#[tauri::command]
async fn autosave_write(
    app: tauri::AppHandle,
    key: String,
    name: String,
    path: Option<String>,
    base64: String,
) -> Result<(), String> {
    let dir = autosave_dir(&app).ok_or("no place for autosave")?;
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let body = serde_json::json!({ "name": name, "path": path, "base64": base64, "when": when });
    let file = dir.join(format!("{}.json", safe_key(&key)));
    let tmp = file.with_extension("saving");
    std::fs::write(&tmp, body.to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

#[tauri::command]
async fn autosave_clear(app: tauri::AppHandle, key: String) {
    if let Some(dir) = autosave_dir(&app) {
        let _ = std::fs::remove_file(dir.join(format!("{}.json", safe_key(&key))));
    }
}

/// Books left from a run that ended without saving them: key, name, path,
/// when (Unix seconds), base64.
#[tauri::command]
async fn autosave_list(app: tauri::AppHandle, keep: String) -> Vec<Value> {
    let Some(dir) = autosave_dir(&app) else { return Vec::new() };
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = e.path();
        let Some(key) = file.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        // This run's own files aren't left over.
        if file.extension().is_none_or(|x| x != "json") || key.starts_with(&safe_key(&keep)) {
            continue;
        }
        if let Ok(v) = std::fs::read_to_string(&file)
            .map_err(|_| ())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ()))
        {
            out.push(serde_json::json!({ "key": key, "name": v["name"], "path": v["path"], "when": v["when"], "base64": v["base64"] }));
        }
    }
    out
}

/// Texts shipped inside the program: the licences.
#[tauri::command]
fn bundled(name: String) -> Result<tauri::ipc::Response, String> {
    let text: &str = match name.as_str() {
        "LICENSE.txt" => include_str!("../../../LICENSE"),
        "THIRD_PARTY_LICENSES.txt" => include_str!("../../../THIRD_PARTY_LICENSES.txt"),
        _ => return Err(format!("{name} isn't shipped with Calc")),
    };
    Ok(tauri::ipc::Response::new(text.as_bytes().to_vec()))
}

/// Script errors go to the terminal Calc was started from.
#[tauri::command]
fn log(text: String) {
    eprintln!("{text}");
}

/// Files named on the command line (opened once the screens are ready).
#[tauri::command]
fn opened_files(state: tauri::State<'_, Opened>) -> Vec<String> {
    state.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
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
    std::process::Command::new(program).spawn().map_err(|e| format!("couldn't start it: {e}"))?;
    app.exit(0);
    Ok(())
}

mod mcp;

/// MCP: connecting in (on, off, who's connected, stop one, the page's
/// answers) and the tool servers Ask uses (start, call, end, by address).
#[tauri::command]
async fn mcp(
    app: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<mcp::Mcp>>,
    name: String,
    args: Value,
) -> Result<Value, String> {
    let m = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match name.as_str() {
            "in_on" => m.listen(app).map(|port| serde_json::json!({ "port": port })),
            "in_off" => {
                m.close();
                Ok(Value::Null)
            }
            "in_programs" => Ok(Value::from(m.programs())),
            "in_drop" => {
                m.drop_program(args["id"].as_u64().unwrap_or(0));
                Ok(Value::Null)
            }
            "in_reply" => {
                m.answer(args["id"].as_u64().unwrap_or(0), args["out"].clone());
                Ok(Value::Null)
            }
            // How another program starts Calc's MCP side.
            "in_setup" => {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "mcpServers": { "xenify-calc": { "command": exe.to_string_lossy(), "args": ["--mcp"] } } }))
            }
            "out_start" => {
                let list = |k: &str| args.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default();
                let env: std::collections::HashMap<String, String> = args.get("env").and_then(Value::as_object).map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
                m.start(&s("server"), &s("command"), &list("args"), &env)
            }
            "out_call" => m.call(&s("server"), &s("method"), args.get("params").cloned().unwrap_or(Value::Null)),
            "out_end" => {
                m.end(&s("server"));
                Ok(Value::Null)
            }
            "out_http" => {
                let headers: std::collections::HashMap<String, String> = args.get("headers").and_then(Value::as_object).map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
                let session = args.get("session").and_then(Value::as_str);
                mcp::http(&s("url"), &headers, session, &args["body"])
            }
            other => Err(format!("unknown MCP call {other}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    // `--mcp`: an AI program started us to talk to the running Calc.
    if std::env::args().any(|a| a == "--mcp") {
        mcp::bridge();
    }
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let allowed = Allowed::new(&["csv", "tsv", "txt", "psv", "tab"]);
    for f in &files {
        allowed.grant(std::path::absolute(f).unwrap_or_else(|_| f.into()));
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && std::env::var_os("GSETTINGS_BACKEND").is_none() {
        // SAFETY: set before any thread is started.
        unsafe { std::env::set_var("GSETTINGS_BACKEND", "memory") };
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(xenify_write_host::proof::Proof::new())
        .manage(Opened(Mutex::new(files)))
        .manage(allowed)
        .manage(std::sync::Arc::new(mcp::Mcp::default()))
        .setup(|app| {
            use tauri::Manager;
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<Allowed>().keep_at(dir.join("allowed-files.json"));
            }
            let rounded = composited();
            // XENIFY_CALC_TEST=file.js: a script run once the screens are up (for testing the desktop build).
            let test = std::env::var("XENIFY_CALC_TEST")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            let init = format!(
                "{}{}",
                if rounded { "window.XENIFY_ROUNDED = true;" } else { "" },
                if test.is_empty() { String::new() } else { format!("window.XENIFY_TEST = async () => {{ {test} }};") }
            );
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Xenify Calc")
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
            choose,
            bundled,
            read_file,
            file_stamp,
            write_file,
            log,
            opened_files,
            quit,
            open_link,
            autosave_write,
            autosave_clear,
            autosave_list,
            discover,
            certificate,
            hub_server,
            mcp,
            check,
            words,
            set_words,
            look_up
        ])
        .run(tauri::generate_context!())
        .expect("Xenify Calc failed to start");
}

/// Opens a web link in the computer's own browser.
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    xenify_write_host::print::open_link(&url, false)
}

/// Spelling and grammar for the given texts (the same request and answer
/// as Xenify Write's checker).
#[tauri::command]
async fn check(
    proof: tauri::State<'_, xenify_write_host::proof::Proof>,
    request: xenify_write_host::proof::Request,
) -> Result<serde_json::Value, String> {
    serde_json::to_value(proof.check(request)).map_err(|e| e.to_string())
}

/// The words added to the dictionary (shared with the other Xenify apps).
#[tauri::command]
async fn words(proof: tauri::State<'_, xenify_write_host::proof::Proof>) -> Result<Vec<String>, String> {
    Ok(proof.words())
}

#[tauri::command]
async fn set_words(proof: tauri::State<'_, xenify_write_host::proof::Proof>, words: Vec<String>) -> Result<(), String> {
    proof.set_words(words);
    Ok(())
}

/// A word's meanings, and words to use instead (the dictionary and thesaurus).
#[tauri::command]
async fn look_up(word: String) -> Result<serde_json::Value, String> {
    Ok(xenify_write_host::proof::look_up(&word))
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

/// Xenify servers on this network, for File › Xenify server.
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


/// Not signed in to a server: Xenify Hub signs every app in, so the app
/// opens it (or brings the running one forward) instead of asking itself.
#[tauri::command]
fn open_hub() -> Result<(), String> {
    xenify_hub_core::signin::open_hub()
}
