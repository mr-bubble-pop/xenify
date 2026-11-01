//! Xenify Hub: installs and updates the Xenify apps. The window lists
//! what's installed and what else there is; updates happen in the
//! background on the channel each app is set to (or as the Xenify server
//! this computer is signed in to decides). It stays in the tray (where the
//! desktop has one) so updates carry on with the window closed.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod elevate;
mod hub;
mod selfupdate;
mod shortcuts;

use hub::Hub;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Manager;

/// The tray icon is up: closing the window keeps the Hub running.
static IN_TRAY: AtomicBool = AtomicBool::new(false);

/// Runs on a worker thread, not the window's: installing, starting the
/// server (which sets up its database first) and checking for updates take
/// a while, and the window would stop responding until they finish.
#[tauri::command]
async fn hub(app: tauri::AppHandle, state: tauri::State<'_, Hub>, name: String, args: Value) -> Result<Value, String> {
    let hub = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match name.as_str() {
        // "Restart now": the new Hub goes in and starts; this one ends.
        "hub_restart" => {
            hub.apply_hub_update(false)?;
            app.exit(0);
            Ok(Value::Null)
        }
        "autostart" => Ok(serde_json::json!({ "on": autostart::is_on() })),
        "set_autostart" => {
            autostart::set(args.get("on").and_then(Value::as_bool).unwrap_or(false))?;
            Ok(serde_json::json!({ "on": autostart::is_on() }))
        }
        _ => hub.call(&name, &args),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Test scripts write here (stdout).
/// Ctrl+Q: the Hub ends (closing its window only puts it in the tray).
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn log(text: String) {
    println!("{text}");
}

/// The window (made again after it was closed to the tray).
fn show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let test = std::env::var("XENIFY_HUB_TEST").ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let init = if test.is_empty() { String::new() } else { format!("window.XENIFY_TEST = async (t) => {{ {test} }};") };
    let _ = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("Xenify Hub")
        .inner_size(960.0, 760.0)
        .min_inner_size(420.0, 480.0)
        .decorations(false)
        .initialization_script(&init)
        .build();
}

/// Whether the desktop can show a tray icon (Linux needs the indicator
/// library; without it the Hub is an ordinary window).
fn tray_possible() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    ["libayatana-appindicator3.so.1", "libappindicator3.so.1"].iter().any(|lib| {
        ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"]
            .iter()
            .any(|d| std::path::Path::new(d).join(lib).exists())
    })
}

fn tray(app: &tauri::App, hub: Hub) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    let open = MenuItem::with_id(app, "open", "Open Xenify Hub", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Xenify Hub", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &quit])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Xenify Hub")
        .menu(&menu)
        .on_menu_event(move |app, e| match e.id.as_ref() {
            "open" => show(app),
            "check" => {
                let hub = hub.clone();
                let app = app.clone();
                std::thread::spawn(move || {
                    if hub.check().is_ok() {
                        use chrono::Timelike;
                        hub.auto_update(chrono::Local::now().hour() as u8);
                        hub.check_hub();
                        update_hub_quietly(&app, &hub);
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// With the window closed (in the tray) and nothing under way, a
/// downloaded Hub goes in by itself: the new one starts in the tray. With
/// the window open the window offers "Restart now" instead.
fn update_hub_quietly(app: &tauri::AppHandle, hub: &Hub) {
    let open = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
    if open || !IN_TRAY.load(Ordering::SeqCst) || !hub.hub_ready_quietly() {
        return;
    }
    if hub.apply_hub_update(true).is_ok() {
        app.exit(0);
    }
}

fn main() {
    // Started with the computer: in the tray, no window.
    let hidden = std::env::args().any(|a| a == "--hidden");
    let the_hub = Hub::new(hub::data_dir());
    // What's running, looked at in the background (the window just reads it).
    the_hub.keep_looking();
    // Updates arrive in the background: the Hub waits in the tray from
    // sign-in, and an app picks its new version up when it's next opened.
    if the_hub.first_start() && std::env::var_os("XENIFY_HUB_TEST").is_none() {
        let _ = autostart::set(true);
    }
    let bg = the_hub.clone();
    let for_tray = the_hub.clone();
    xenify_hub_core::signin::remember_hub();
    let app = tauri::Builder::default()
        // Opened again (from the menu, or an app's "Open Xenify Hub"): the
        // running Hub shows its window instead of a second one starting.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)))
        .manage(the_hub)
        .setup(move |app| {
            if tray_possible() && tray(app, for_tray).is_ok() {
                IN_TRAY.store(true, Ordering::SeqCst);
            }
            if !hidden || !IN_TRAY.load(Ordering::SeqCst) {
                show(app.handle());
            }
            // Check now and every half hour; install what's due, and a new
            // Hub too (put in place by itself while the window is closed).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                // The server starts beside the list being fetched: starting
                // its database can take a minute, and the apps showed only
                // after it. (Whether the system service runs it is looked at
                // first, so the Hub doesn't start a second one.)
                let server = bg.clone();
                std::thread::spawn(move || {
                    server.look();
                    server.start_server_if_wanted();
                    server.look();
                });
                loop {
                    if bg.check().is_ok() {
                        use chrono::Timelike;
                        bg.auto_update(chrono::Local::now().hour() as u8);
                        bg.check_hub();
                        update_hub_quietly(&handle, &bg);
                    }
                    std::thread::sleep(Duration::from_secs(30 * 60));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![hub, log, quit])
        .build(tauri::generate_context!())
        .expect("Xenify Hub couldn't start");
    app.run(|_, e| {
        // In the tray: the last window closing doesn't end the Hub.
        if let tauri::RunEvent::ExitRequested { api, code: None, .. } = e
            && IN_TRAY.load(Ordering::SeqCst)
        {
            api.prevent_exit();
        }
    });
}
