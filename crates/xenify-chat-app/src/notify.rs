//! Desktop notifications, and your settings (kept on this computer).
//!
//! The screens decide whether something deserves a notification (mutes, Do
//! not disturb, quiet hours); this side shows it and, when it's clicked,
//! brings Xenify Chat forward on that chat.

use std::path::PathBuf;
use tauri::AppHandle;
#[cfg(all(unix, not(target_os = "macos")))]
use tauri::{Emitter, Manager};

/// Shows a notification; clicking it opens chat `conv`.
#[tauri::command]
pub fn notify(app: AppHandle, title: String, body: String, conv: i64, sound: bool) {
    // For tests: write notifications to a file as well.
    if let Some(log) = std::env::var_os("XENIFY_NOTIFY_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{title} | {body} | {conv} | sound={sound}");
        }
    }
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname("Xenify Chat").summary(&title).body(&body).action("default", "Open");
        if sound {
            n.sound_name("message-new-instant");
        }
        let Ok(shown) = n.show() else { return };
        #[cfg(all(unix, not(target_os = "macos")))]
        shown.wait_for_action(|action| {
            if action == "default" {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit("open_chat", conv);
            }
        });
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        let _ = (shown, app);
    });
}

fn prefs_path() -> PathBuf {
    xenify_wp_check::config_dir().join("chat-settings.json")
}

/// Your settings as the screens saved them (an empty object the first time).
#[tauri::command]
pub fn load_settings() -> serde_json::Value {
    std::fs::read(prefs_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

#[tauri::command]
pub fn save_settings(settings: serde_json::Value) -> Result<(), String> {
    let p = prefs_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
