//! Xenify Hub without its window. The Hub installs and updates every Xenify
//! app (Write, Draw, Calc, Chat, the server and its parts: PostgreSQL, LocalAI),
//! like a toolbox app: it works on its own, reading a signed list of
//! releases, and when it's signed in to a Xenify server the server's admin
//! decides (which apps, which channel or version, when).
//!
//! - [`manifest`]: the release list and its signature (ed25519).
//! - [`choose`]: channels, fixed versions and the server's policy.
//! - [`install`]: where apps go, installing, rolling back, removing.
//! - [`signin`]: the Hub's server sign-in, which the apps follow.

pub mod choose;
pub mod install;
pub mod manifest;
pub mod pg;
pub mod signin;

/// A program to run with no console window of its own. (On Windows a
/// console program started by a windowed one, such as the Hub, otherwise
/// opens an empty command prompt that stays while it runs.)
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    without_appimage(&mut c);
    c
}

/// The Hub as an AppImage points its own process at the libraries inside
/// it (LD_LIBRARY_PATH, GStreamer's and GTK's paths, XDG_DATA_DIRS, all
/// under $APPDIR). A program it starts would inherit that and load the
/// Hub's web engine instead of the computer's: without GStreamer's
/// plugins, so Chat found no microphone or camera and couldn't play video.
/// Every path inside the AppImage is taken out again, so programs start as
/// they would from the desktop's menu.
fn without_appimage(c: &mut std::process::Command) {
    let Some(dir) = std::env::var_os("APPDIR").map(|d| d.to_string_lossy().into_owned()) else { return };
    let vars = std::env::vars_os().map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()));
    for (k, v) in appimage_env(&dir, vars) {
        match v {
            Some(v) => c.env(k, v),
            None => c.env_remove(k),
        };
    }
}

/// The changes [`without_appimage`] makes: each variable naming a path in
/// `dir` loses those paths (None: nothing left, so it goes).
fn appimage_env(dir: &str, vars: impl Iterator<Item = (String, String)>) -> Vec<(String, Option<String>)> {
    if dir.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<(String, Option<String>)> = vars
        .filter(|(_, v)| v.contains(dir))
        .map(|(k, v)| {
            let kept: Vec<&str> = v.split(':').filter(|p| !p.is_empty() && !p.contains(dir)).collect();
            (k, (!kept.is_empty()).then(|| kept.join(":")))
        })
        .collect();
    for k in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
        out.retain(|(x, _)| x != k);
        out.push((k.to_string(), None));
    }
    out
}

#[cfg(test)]
mod appimage_tests {
    #[test]
    fn programs_started_from_the_appimage_hub_get_the_computers_libraries() {
        let vars = [
            ("LD_LIBRARY_PATH", "/tmp/.mount_Xen1/usr/lib"),
            ("XDG_DATA_DIRS", "/tmp/.mount_Xen1/usr/share:/usr/local/share:/usr/share"),
            ("GST_PLUGIN_SYSTEM_PATH_1_0", "/tmp/.mount_Xen1/usr/lib/gstreamer-1.0"),
            ("HOME", "/home/me"),
            ("PATH", "/tmp/.mount_Xen1/usr/bin:/usr/bin"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let out = super::appimage_env("/tmp/.mount_Xen1", vars.into_iter());
        let get = |k: &str| out.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
        assert_eq!(get("LD_LIBRARY_PATH"), Some(None));
        assert_eq!(get("GST_PLUGIN_SYSTEM_PATH_1_0"), Some(None));
        assert_eq!(get("XDG_DATA_DIRS"), Some(Some("/usr/local/share:/usr/share".into())));
        assert_eq!(get("PATH"), Some(Some("/usr/bin".into())));
        assert_eq!(get("HOME"), None, "left alone");
        assert_eq!(get("APPIMAGE"), Some(None));
        // Not an AppImage: nothing changes.
        assert!(super::appimage_env("", std::iter::empty()).is_empty());
    }
}

#[cfg(test)]
mod tests;

pub use choose::{AppDefaults, Choice, Effective, Local, Policy, ProductPolicy, UpdateMode};
pub use install::{Installed, Store};
pub use manifest::{Channel, Download, Manifest, Product, Release, Signed};

/// The keys Xenify's release lists are signed with.
pub const TRUSTED: &[[u8; 32]] = &[
    // Xenify's release key (made 2026-10-27; the 2026-10-24 one was never
    // used for a release).
    [
        36, 117, 127, 248, 165, 205, 96, 128, 124, 152, 7, 3, 14, 143, 116, 111, 180, 104, 248, 116, 58, 251, 213, 68,
        254, 167, 3, 181, 232, 104, 47, 165,
    ],
];

/// Xenify's own signed release list (made by the Release workflow, .github/workflows/release.yml).
pub const RELEASES: &str =
    "https://github.com/mr-bubble-pop/xenify/releases/download/hub-releases/manifest.json";

/// This computer's system and processor, as release files name them.
pub fn platform() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    (os, arch)
}
