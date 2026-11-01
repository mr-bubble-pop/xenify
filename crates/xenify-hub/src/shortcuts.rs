//! What an installed app gets on the computer, kept pointing at the version
//! in use: a Start menu entry (Windows), an applications-menu entry and a
//! command on the PATH (Linux, in ~/.local/bin), an Applications entry
//! (Mac). Removed with the app.

use std::path::{Path, PathBuf};

/// Apps people open (the server and the AI runner get a command only).
fn has_window(product: &str) -> bool {
    matches!(product, "write" | "draw" | "calc" | "chat" | "plan")
}

/// The applications-menu categories for its Linux entry, and the kinds of
/// file it opens (its MimeType line; none for apps that only open their own).
/// Calc's are the ones its tauri.conf.json declares: .xcalc, .xlsx/.xlsm/.xls
/// and .csv.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_kinds(product: &str) -> (&'static str, &'static str) {
    match product {
        "plan" => ("Office;ProjectManagement;", "application/x-xenify-plan;"),
        "calc" => (
            "Office;Spreadsheet;",
            "application/x-xenify-calc;application/vnd.openxmlformats-officedocument.spreadsheetml.sheet;application/vnd.ms-excel.sheet.macroEnabled.12;application/vnd.ms-excel;text/csv;",
        ),
        _ => ("Office;Graphics;", ""),
    }
}

/// The kind of link that opens a server document in it, from the server's
/// Files page (`xenify-write://open?server=…&node=…`): apps that take one.
fn link_scheme(product: &str) -> Option<&'static str> {
    match product {
        "write" => Some("xenify-write"),
        "plan" => Some("xenify-plan"),
        _ => None,
    }
}

/// The command it's run by (`xenify-write`, …).
fn command(product: &str) -> String {
    format!("xenify-{product}")
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

#[cfg(target_os = "linux")]
fn applications() -> Option<PathBuf> {
    Some(
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local/share")))?
            .join("applications"),
    )
}

/// Puts (or moves) the app's entries to `program` (the version in use).
pub fn add(product: &str, name: &str, about: &str, program: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // A command on the PATH (~/.local/bin is on it on most systems).
        let bin = home().ok_or("there's no home folder")?.join(".local/bin");
        std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
        let link = bin.join(command(product));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(program, &link).map_err(|e| e.to_string())?;
        if has_window(product) {
            let dir = applications().ok_or("there's no home folder")?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            // The app's own icon, when its package has one beside the program.
            let icon = program
                .parent()
                .map(|d| d.join(format!("{}.png", command(product))))
                .filter(|p| p.is_file())
                .map_or_else(|| command(product), |p| p.display().to_string());
            let (categories, mime) = desktop_kinds(product);
            // An app that takes links gets addresses (%U: links, and files as file://).
            let scheme = link_scheme(product);
            let mime = format!("{mime}{}", scheme.map(|x| format!("x-scheme-handler/{x};")).unwrap_or_default());
            let mime = if mime.is_empty() { String::new() } else { format!("MimeType={mime}\n") };
            let text = format!(
                "[Desktop Entry]\nType=Application\nName=Xenify {name}\nComment={about}\nExec=\"{}\" {}\nIcon={icon}\nTerminal=false\nCategories={categories}\n{mime}StartupWMClass={}\n",
                program.display(),
                if scheme.is_some() { "%U" } else { "%F" },
                command(product)
            );
            let entry = format!("{}.desktop", command(product));
            std::fs::write(dir.join(&entry), text).map_err(|e| e.to_string())?;
            // Its links open in it (not left to whatever the desktop picks).
            if let Some(x) = scheme {
                let _ = std::process::Command::new("xdg-mime")
                    .args(["default", &entry, &format!("x-scheme-handler/{x}")])
                    .status();
            }
        }
    }
    #[cfg(windows)]
    if has_window(product) {
        let dir = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or("there's no Start menu folder")?
            .join(r"Microsoft\Windows\Start Menu\Programs\Xenify");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let lnk = dir.join(format!("Xenify {name}.lnk"));
        let ps = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.Description='{}';$s.Save()",
            lnk.display().to_string().replace('\'', "''"),
            program.display().to_string().replace('\'', "''"),
            program.parent().map(|p| p.display().to_string()).unwrap_or_default().replace('\'', "''"),
            about.replace('\'', "''")
        );
        let ok = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return Err("couldn't add it to the Start menu".into());
        }
        // Its links open in it: the link type, for this user only.
        if let Some(x) = link_scheme(product) {
            let key = format!(r"HKCU\Software\Classes\{x}");
            let command = format!(r"{key}\shell\open\command");
            let title = format!("URL:Xenify {name}");
            let open = format!("\"{}\" \"%1\"", program.display());
            for args in [
                [key.as_str(), "/ve", "/d", title.as_str()],
                [key.as_str(), "/v", "URL Protocol", "/d"],
                [command.as_str(), "/ve", "/d", open.as_str()],
            ] {
                // The second sets an empty value: /d with nothing after it.
                let mut c = std::process::Command::new("reg");
                c.arg("add").args(args);
                if args[1] == "/v" {
                    c.arg("");
                }
                let _ = c.arg("/f").status();
            }
        }
    }
    #[cfg(target_os = "macos")]
    if has_window(product) {
        // The .app bundle the program is inside.
        let bundle =
            program.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).ok_or("that isn't a Mac app")?;
        let dir = home().ok_or("there's no home folder")?.join("Applications");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let link = dir.join(format!("Xenify {name}.app"));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(bundle, &link).map_err(|e| e.to_string())?;
        // Its links (Info.plist) open in it now, not only once the Mac
        // happens to notice the new app.
        if link_scheme(product).is_some() {
            let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
            let _ = std::process::Command::new(lsregister).arg("-f").arg(bundle).status();
        }
    }
    let _ = (name, about, program);
    Ok(())
}

/// Takes the app's entries away (it was removed).
pub fn remove(product: &str, name: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Some(h) = home() {
            let _ = std::fs::remove_file(h.join(".local/bin").join(command(product)));
        }
        if let Some(d) = applications() {
            let _ = std::fs::remove_file(d.join(format!("{}.desktop", command(product))));
        }
    }
    #[cfg(windows)]
    {
        if let Some(d) = std::env::var_os("APPDATA").map(PathBuf::from) {
            let _ = std::fs::remove_file(
                d.join(format!(r"Microsoft\Windows\Start Menu\Programs\Xenify\Xenify {name}.lnk")),
            );
        }
        if let Some(x) = link_scheme(product) {
            let _ = std::process::Command::new("reg")
                .args(["delete", &format!(r"HKCU\Software\Classes\{x}"), "/f"])
                .status();
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(h) = home() {
        let _ = std::fs::remove_file(h.join("Applications").join(format!("Xenify {name}.app")));
    }
    let _ = (product, name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_gets_a_window_entry_that_opens_its_files() {
        assert!(has_window("calc"));
        assert_eq!(command("calc"), "xenify-calc");
        let (categories, mime) = desktop_kinds("calc");
        assert!(categories.contains("Spreadsheet"));
        for kind in [
            "application/x-xenify-calc",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "text/csv",
        ] {
            assert!(mime.split(';').any(|m| m == kind), "{kind}");
        }
        // The others keep their entry as it was.
        assert_eq!(desktop_kinds("draw"), ("Office;Graphics;", ""));
        assert!(!has_window("server"));
    }

    #[test]
    fn write_takes_links_to_server_documents() {
        assert_eq!(link_scheme("write"), Some("xenify-write"));
        assert_eq!(link_scheme("plan"), Some("xenify-plan"));
        assert_eq!(link_scheme("calc"), None);
    }
}
