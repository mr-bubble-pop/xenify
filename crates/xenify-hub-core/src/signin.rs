//! The Hub's sign-in to a Xenify server, shared with the apps: sign in once
//! in the Hub and Write, Draw, Calc, Plan and Chat use that server too
//! (unless one is set to use a different server). Changing the server in
//! the Hub moves them over; detaching signs them out.
//!
//! It's kept in `server.json` in Xenify's settings folder
//! (`$XENIFY_DATA_DIR`, else `~/.config/xenify` or the platform's), which
//! only you can read: the address, the names and the session, as each app
//! would keep its own. Tokens from "Remember this computer" (the second
//! sign-in step) stay after signing out.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignIn {
    /// New for every sign-in, so the apps notice a change of server or account.
    pub id: String,
    pub url: String,
    /// The server's name and yours, to show.
    pub name: String,
    pub user: String,
    /// What the server answered at sign-in (`token`, `account`, …).
    pub session: serde_json::Value,
    /// Server address -> token from "Remember this computer for 30 days":
    /// signing in there again skips the second step.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub remember: std::collections::BTreeMap<String, String>,
}

fn path() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("XENIFY_DATA_DIR") {
        return Some(PathBuf::from(d).join("server.json"));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").filter(|_| cfg!(windows)).map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("xenify").join("server.json"))
}

fn read_any() -> Option<SignIn> {
    serde_json::from_str(&std::fs::read_to_string(path()?).ok()?).ok()
}

/// The Hub's sign-in, if it has one.
pub fn read() -> Option<SignIn> {
    read_any().filter(|s| !s.url.is_empty())
}

/// The token that skips the second step at `url`, if this computer was remembered there.
pub fn remembered(url: &str) -> Option<String> {
    read_any()?.remember.get(url).cloned()
}

/// Shares `s` (the Hub, after signing in).
/// Remembered computers already kept stay, with `s`'s added.
pub fn write(s: &SignIn) -> Result<(), String> {
    let mut s = s.clone();
    for (url, token) in read_any().map(|o| o.remember).unwrap_or_default() {
        s.remember.entry(url).or_insert(token);
    }
    let p = path().ok_or("no home folder")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = p.with_extension("json.part");
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&part).map_err(|e| e.to_string())?;
        f.write_all(serde_json::to_string_pretty(&s).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&part, &p).map_err(|e| e.to_string())
}

/// Stops sharing (the Hub detached from the server): the apps sign out.
/// Remembered computers stay remembered.
pub fn clear() {
    let remember = read_any().map(|s| s.remember).unwrap_or_default();
    if !remember.is_empty() {
        let _ = write(&SignIn { remember, ..Default::default() });
    } else if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_read_cleared() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: the only test here that sets it.
        unsafe { std::env::set_var("XENIFY_DATA_DIR", dir.path()) };
        assert!(read().is_none());
        let s = SignIn {
            id: "a".into(),
            url: "https://x.example".into(),
            name: "X".into(),
            user: "Priya".into(),
            session: serde_json::json!({ "token": "t" }),
            remember: Default::default(),
        };
        write(&s).unwrap();
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(
                &std::fs::metadata(dir.path().join("server.json")).unwrap().permissions()
            ) & 0o777,
            0o600
        );
        assert_eq!(read(), Some(s.clone()));
        clear();
        assert!(read().is_none());
        // A remembered computer outlives signing out.
        let mut s = s;
        s.remember.insert(s.url.clone(), "r".into());
        write(&s).unwrap();
        clear();
        assert!(read().is_none());
        assert_eq!(remembered("https://x.example").as_deref(), Some("r"));
        assert_eq!(remembered("https://y.example"), None);
    }
}
