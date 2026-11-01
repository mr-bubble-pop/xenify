//! Starting Xenify Hub (in the tray) when the person signs in to the
//! computer: an autostart entry on Linux, a Run key on Windows, a launch
//! agent on a Mac.

use std::path::PathBuf;

fn program() -> Result<String, String> {
    std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn entry() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("xenify-hub.desktop"))
}

#[cfg(target_os = "macos")]
fn entry() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/LaunchAgents/net.xenify.hub.plist"))
}

#[cfg(not(windows))]
pub fn is_on() -> bool {
    entry().is_some_and(|p| p.exists())
}

#[cfg(not(windows))]
pub fn set(on: bool) -> Result<(), String> {
    let path = entry().ok_or("there's no home folder")?;
    if !on {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let program = program()?;
    #[cfg(target_os = "linux")]
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Xenify Hub\nComment=Keeps the Xenify apps up to date\nExec=\"{program}\" --hidden\nIcon=xenify-hub\nX-GNOME-Autostart-enabled=true\n"
    );
    #[cfg(target_os = "macos")]
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>net.xenify.hub</string><key>ProgramArguments</key><array><string>{program}</string><string>--hidden</string></array><key>RunAtLoad</key><true/></dict></plist>\n"
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn is_on() -> bool {
    std::process::Command::new("reg").args(["query", KEY, "/v", "XenifyHub"]).output().is_ok_and(|o| o.status.success())
}

#[cfg(windows)]
pub fn set(on: bool) -> Result<(), String> {
    let status = if on {
        let value = format!("\"{}\" --hidden", program()?);
        std::process::Command::new("reg")
            .args(["add", KEY, "/v", "XenifyHub", "/t", "REG_SZ", "/d", &value, "/f"])
            .status()
    } else {
        std::process::Command::new("reg").args(["delete", KEY, "/v", "XenifyHub", "/f"]).status()
    };
    match status {
        Ok(s) if s.success() || !on => Ok(()),
        _ => Err("couldn't change what starts with Windows".into()),
    }
}
