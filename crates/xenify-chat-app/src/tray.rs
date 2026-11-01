//! Xenify Chat in the system tray: closing the window hides it (messages
//! still arrive and notify), the tray icon brings it back and gets a dot
//! when something new is waiting, and it can start when you sign in to
//! your computer (started that way, it waits in the tray).
//!
//! Getting attention: the taskbar entry flashes until the window is seen.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

/// Started by signing in to the computer: stay in the tray.
pub const HIDDEN_ARG: &str = "--in-tray";

pub struct Tray {
    icon: TrayIcon,
    plain: Image<'static>,
    dotted: Image<'static>,
}

/// Brings the window forward.
pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// The app's icon with a red dot in its top right corner.
fn with_dot(icon: &Image<'_>) -> Image<'static> {
    let (w, h) = (icon.width(), icon.height());
    let mut rgba = icon.rgba().to_vec();
    let r = (w.min(h) as f32 * 0.22).max(3.0);
    let (cx, cy) = (w as f32 - r - 1.0, r + 1.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let i = ((y * w + x) * 4) as usize;
            if d <= r {
                // A white ring keeps it clear on dark and light panels.
                let c = if d > r - (r * 0.25).max(1.0) { [255, 255, 255, 255] } else { [196, 48, 32, 255] };
                rgba[i..i + 4].copy_from_slice(&c);
            }
        }
    }
    Image::new_owned(rgba, w, h)
}

/// Puts Xenify Chat in the tray (None where the desktop has no tray).
pub fn start(app: &AppHandle) -> Option<Tray> {
    // With a clear margin, so it doesn't run into its neighbours in the tray.
    let plain = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png")).ok()?.to_owned();
    let dotted = with_dot(&plain);
    let open = MenuItem::with_id(app, "open", "Open Xenify Chat", true, None::<&str>).ok()?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).ok()?;
    let menu = Menu::with_items(app, &[&open, &quit]).ok()?;
    let icon = TrayIconBuilder::with_id("main")
        .icon(plain.clone())
        .tooltip("Xenify Chat")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show(tray.app_handle());
            }
        })
        .build(app)
        .ok()?;
    Some(Tray { icon, plain, dotted })
}

/// Something new is waiting (`on`), or it's been seen. `flash`: the taskbar
/// entry flashes too (not with reduce motion on).
#[tauri::command]
pub fn attention(app: AppHandle, on: bool, flash: bool) {
    if let Some(t) = app.try_state::<Tray>() {
        let _ = t.icon.set_icon(Some(if on { t.dotted.clone() } else { t.plain.clone() }));
    }
    if let Some(w) = app.get_webview_window("main") {
        let focused = w.is_focused().unwrap_or(false);
        let kind = (on && flash && !focused).then_some(tauri::UserAttentionType::Informational);
        let _ = w.request_user_attention(kind);
    }
}

/// Whether Xenify Chat starts when you sign in to this computer.
/// Ctrl+W: the window goes as when it's closed (to the tray when there is one).
#[tauri::command]
pub fn close_window(window: tauri::WebviewWindow) {
    let _ = window.close();
}

/// Ctrl+Q: Chat ends (closing its window only puts it in the tray).
#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn autostart_get() -> bool {
    autostart::on()
}

#[tauri::command]
pub fn autostart_set(on: bool) -> Result<(), String> {
    autostart::set(on)
}

#[cfg(all(unix, not(target_os = "macos")))]
mod autostart {
    //! An entry in ~/.config/autostart (the freedesktop way).
    fn path() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
        Some(base.join("autostart").join("xenify-chat.desktop"))
    }

    pub fn on() -> bool {
        path().is_some_and(|p| p.exists())
    }

    pub fn set(on: bool) -> Result<(), String> {
        let p = path().ok_or("There's no home folder to put it in.")?;
        if !on {
            let _ = std::fs::remove_file(&p);
            return Ok(());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Xenify Chat\nExec=\"{}\" {}\nX-GNOME-Autostart-enabled=true\nTerminal=false\n",
            exe.display(),
            super::HIDDEN_ARG
        );
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, entry).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
mod autostart {
    //! A launch agent in ~/Library/LaunchAgents.
    fn path() -> Option<std::path::PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(std::path::Path::new(&home).join("Library/LaunchAgents/app.xenify.chat.plist"))
    }

    pub fn on() -> bool {
        path().is_some_and(|p| p.exists())
    }

    pub fn set(on: bool) -> Result<(), String> {
        let p = path().ok_or("There's no home folder to put it in.")?;
        if !on {
            let _ = std::fs::remove_file(&p);
            return Ok(());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>app.xenify.chat</string>\n<key>ProgramArguments</key><array><string>{}</string><string>{}</string></array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n",
            esc(&exe.display().to_string()),
            super::HIDDEN_ARG
        );
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, plist).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod autostart {
    //! The Run key in the registry (through reg.exe).
    use std::os::windows::process::CommandExt;
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const NO_WINDOW: u32 = 0x0800_0000;

    pub fn on() -> bool {
        std::process::Command::new("reg")
            .args(["query", KEY, "/v", "Xenify Chat"])
            .creation_flags(NO_WINDOW)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    pub fn set(on: bool) -> Result<(), String> {
        let mut c = std::process::Command::new("reg");
        if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            c.args(["add", KEY, "/v", "Xenify Chat", "/t", "REG_SZ", "/f", "/d"]).arg(format!(
                "\"{}\" {}",
                exe.display(),
                super::HIDDEN_ARG
            ));
        } else {
            c.args(["delete", KEY, "/v", "Xenify Chat", "/f"]);
        }
        let out = c.creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
        if out.status.success() || !on {
            Ok(())
        } else {
            Err("Windows wouldn't add it to the programs that start.".into())
        }
    }
}
