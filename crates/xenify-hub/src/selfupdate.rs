//! Xenify Hub keeps itself up to date, as it does the apps: the release
//! list has the Hub as product "hub" (the Windows installer, the Linux
//! AppImage, the Mac app as a .tar.gz). A newer one is downloaded and
//! checked against the signed list, then put in place when the Hub
//! restarts: by itself when its window is closed, or from the window's
//! "Restart now". A copy it can't replace (the .deb, an app in a folder it
//! can't write) points to the download page instead.

use std::path::{Path, PathBuf};
use xenify_hub_core::{Effective, Product, Release, choose};

/// This Hub's version as the release list names it (a nightly is
/// "0.1.5-nightly.202611020600": scripts/package-hub.sh sets XENIFY_VERSION).
pub const VERSION: &str = match option_env!("XENIFY_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};

/// Where a newer Hub can be downloaded by hand.
pub const DOWNLOAD_PAGE: &str = "https://xenify.net/download/";

/// The release to move to: only one newer than `own` (a Hub never goes back
/// by itself, whatever the channel says).
pub fn newer<'a>(p: &'a Product, own: &str, e: &Effective, os: &str, arch: &str) -> Option<&'a Release> {
    if !e.allowed {
        return None;
    }
    let target = choose::pick(p, &e.choice, os, arch)?;
    let own = semver::Version::parse(own).ok()?;
    (target.semver()? > own).then_some(target)
}

/// How this Hub was installed: what replacing it takes.
#[derive(Clone, Debug, PartialEq)]
pub enum How {
    /// The Windows installer (for this user): run the new one quietly.
    Windows,
    /// An AppImage at this path: swap the file.
    AppImage(PathBuf),
    /// A Mac app bundle at this path: swap the bundle.
    MacApp(PathBuf),
    /// Installed some other way (the .deb, a folder it can't write): the
    /// person downloads the new one.
    ByHand,
}

pub fn how() -> How {
    if cfg!(windows) {
        return How::Windows;
    }
    if cfg!(target_os = "linux") {
        return match std::env::var_os("APPIMAGE").map(PathBuf::from) {
            Some(p) if writable_dir(p.parent()) => How::AppImage(p),
            _ => How::ByHand,
        };
    }
    let exe = std::env::current_exe().ok();
    match exe.as_deref().and_then(app_bundle) {
        Some(b) if writable_dir(b.parent()) => How::MacApp(b),
        _ => How::ByHand,
    }
}

/// The `.app` folder around a Mac program.
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

fn writable_dir(dir: Option<&Path>) -> bool {
    let Some(dir) = dir else { return false };
    let probe = dir.join(format!(".xenify-hub-write-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

/// The name the downloaded file is kept under (by what it is).
pub fn file_name(how: &How, version: &str) -> String {
    match how {
        How::Windows => format!("Xenify-Hub-{version}-setup.exe"),
        How::AppImage(_) => format!("Xenify-Hub-{version}.AppImage"),
        _ => format!("Xenify-Hub-{version}.tar.gz"),
    }
}

/// A string as one PowerShell argument in single quotes.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// What the hidden PowerShell runs on Windows: waits for this Hub to end,
/// runs the new installer quietly (it installs over this one), starts the
/// new Hub.
fn windows_script(setup: &Path, exe: &Path, hidden: bool) -> String {
    let start = if hidden {
        format!("Start-Process -FilePath {} -ArgumentList '--hidden'", ps_quote(&exe.display().to_string()))
    } else {
        format!("Start-Process -FilePath {}", ps_quote(&exe.display().to_string()))
    };
    format!(
        "Start-Sleep -Seconds 2; Start-Process -FilePath {} -ArgumentList '/S' -Wait; {start}",
        ps_quote(&setup.display().to_string())
    )
}

/// Puts the downloaded Hub (`file`, already checked) in place and starts
/// it. The caller ends this Hub straight after. `hidden`: the new one
/// starts in the tray, as this one was.
pub fn apply(how: &How, file: &Path, hidden: bool) -> Result<(), String> {
    match how {
        How::Windows => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            xenify_hub_core::command("powershell")
                .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &windows_script(file, &exe, hidden)])
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Couldn't start the new Xenify Hub's installer: {e}"))
        }
        How::AppImage(target) => {
            let new = target.with_extension("AppImage.new");
            std::fs::copy(file, &new).map_err(|e| format!("Couldn't put the new Xenify Hub beside this one: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&new, target).map_err(|e| {
                let _ = std::fs::remove_file(&new);
                format!("Couldn't replace Xenify Hub: {e}")
            })?;
            let mut c = xenify_hub_core::command(target);
            if hidden {
                c.arg("--hidden");
            }
            c.spawn().map(|_| ()).map_err(|e| format!("The new Xenify Hub didn't start: {e}"))
        }
        How::MacApp(bundle) => {
            let parent = bundle.parent().ok_or("Xenify Hub isn't in a folder")?;
            let tmp = parent.join(".xenify-hub-update");
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            let ok = xenify_hub_core::command("tar")
                .arg("-xzf")
                .arg(file)
                .arg("-C")
                .arg(&tmp)
                .status()
                .is_ok_and(|s| s.success());
            let new = std::fs::read_dir(&tmp)
                .ok()
                .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "app")));
            let Some(new) = new.filter(|_| ok) else {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err("The new Xenify Hub couldn't be unpacked.".into());
            };
            let old = bundle.with_extension("app.old");
            let _ = std::fs::remove_dir_all(&old);
            std::fs::rename(bundle, &old).map_err(|e| format!("Couldn't replace Xenify Hub: {e}"))?;
            if let Err(e) = std::fs::rename(&new, bundle) {
                let _ = std::fs::rename(&old, bundle);
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(format!("Couldn't replace Xenify Hub: {e}"));
            }
            let _ = std::fs::remove_dir_all(&old);
            let _ = std::fs::remove_dir_all(&tmp);
            let mut c = xenify_hub_core::command("open");
            c.arg("-n").arg(bundle);
            if hidden {
                c.args(["--args", "--hidden"]);
            }
            c.spawn().map(|_| ()).map_err(|e| format!("The new Xenify Hub didn't start: {e}"))
        }
        How::ByHand => Err(format!("Download the new Xenify Hub from {DOWNLOAD_PAGE}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_hub_core::{Channel, Choice, Download};

    fn hub(versions: &[(&str, Channel)]) -> Product {
        let releases = versions
            .iter()
            .map(|(v, c)| Release {
                version: v.to_string(),
                channel: *c,
                date: String::new(),
                notes: String::new(),
                downloads: vec![Download {
                    os: "windows".into(),
                    arch: "x86_64".into(),
                    url: format!("https://example.org/hub-{v}.exe"),
                    size: 1,
                    sha256: String::new(),
                    run: String::new(),
                }],
            })
            .collect();
        Product { id: "hub".into(), name: "Hub".into(), about: String::new(), component: false, requires: vec![], releases }
    }

    fn on(channel: Channel) -> Effective {
        let mut e = choose::effective(&Default::default(), None, "hub");
        e.choice = Choice::Channel(channel);
        e
    }

    #[test]
    fn the_hub_moves_only_to_a_newer_version() {
        let p = hub(&[("0.1.3", Channel::Release), ("0.1.4", Channel::Release), ("0.1.5-nightly.1", Channel::Nightly)]);
        let to = |own: &str, c| newer(&p, own, &on(c), "windows", "x86_64").map(|r| r.version.clone());
        assert_eq!(to("0.1.3", Channel::Release).as_deref(), Some("0.1.4"));
        assert_eq!(to("0.1.4", Channel::Release), None, "already the newest");
        // A nightly Hub on the release channel doesn't go back to 0.1.4.
        assert_eq!(to("0.1.5-nightly.1", Channel::Release), None);
        assert_eq!(to("0.1.4", Channel::Nightly).as_deref(), Some("0.1.5-nightly.1"));
        // Nothing for this computer.
        assert!(newer(&p, "0.1.0", &on(Channel::Release), "linux", "x86_64").is_none());
    }

    #[test]
    fn paths_with_quotes_reach_powershell_whole() {
        assert_eq!(ps_quote(r"C:\Users\O'Brien\x.exe"), r"'C:\Users\O''Brien\x.exe'");
        let s = windows_script(Path::new(r"C:\a b\setup.exe"), Path::new(r"C:\Hub\xenify-hub.exe"), true);
        assert!(s.contains(r"Start-Process -FilePath 'C:\a b\setup.exe' -ArgumentList '/S' -Wait"), "{s}");
        assert!(s.ends_with(r"Start-Process -FilePath 'C:\Hub\xenify-hub.exe' -ArgumentList '--hidden'"), "{s}");
    }

    #[test]
    fn a_mac_program_is_found_in_its_bundle() {
        let b = app_bundle(Path::new("/Applications/Xenify Hub.app/Contents/MacOS/xenify-hub"));
        assert_eq!(b, Some(PathBuf::from("/Applications/Xenify Hub.app")));
        assert_eq!(app_bundle(Path::new("/usr/bin/xenify-hub")), None);
    }
}
