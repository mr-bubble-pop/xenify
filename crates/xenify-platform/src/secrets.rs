//! Secrets (API keys) in the OS credential store: Secret Service (GNOME Keyring /
//! KWallet) on Linux, Keychain on macOS, Credential Manager on Windows.
//!
//! Where no credential store is running (e.g. a bare container or a minimal
//! window manager without a keyring daemon), secrets fall back to a private file
//! (`~/.config/xenify/secrets.json`, readable only by the user), and
//! [`location`] says so, so the UI can tell the user.

use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(not(target_arch = "wasm32"))]
const SERVICE: &str = "xenify";

/// Where a secret is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretLocation {
    /// The OS credential store (encrypted, unlocked with the user's login).
    Keyring,
    /// A file only the user can read (no credential store was available).
    PrivateFile,
}

impl SecretLocation {
    pub fn describe(self) -> &'static str {
        match self {
            SecretLocation::Keyring => "system keyring",
            SecretLocation::PrivateFile => "private file (no system keyring found)",
        }
    }
}

/// Lookups hit D-Bus / the Keychain, so answers are cached per process.
type Cache = HashMap<String, Option<(String, SecretLocation)>>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The secret called `name` and where it is stored, if set.
pub fn load(name: &str) -> Option<(String, SecretLocation)> {
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(v) = cache.get(name) {
        return v.clone();
    }
    let found = backend::load(name);
    cache.insert(name.to_string(), found.clone());
    found
}

/// Stores `value` as secret `name` (the keyring if possible) and returns where.
pub fn save(name: &str, value: &str) -> Result<SecretLocation, String> {
    let location = backend::save(name, value)?;
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.into(), Some((value.into(), location)));
    Ok(location)
}

/// Removes secret `name` from everywhere it may be stored.
pub fn delete(name: &str) -> Result<(), String> {
    backend::delete(name)?;
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.into(), None);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use super::{SERVICE, SecretLocation};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn keyring(name: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, name).ok()
    }

    pub fn load(name: &str) -> Option<(String, SecretLocation)> {
        if let Some(e) = keyring(name)
            && let Ok(v) = e.get_password()
        {
            return Some((v, SecretLocation::Keyring));
        }
        file_read().remove(name).map(|v| (v, SecretLocation::PrivateFile))
    }

    pub fn save(name: &str, value: &str) -> Result<SecretLocation, String> {
        if let Some(e) = keyring(name)
            && e.set_password(value).is_ok()
        {
            // Don't leave an older copy in the fallback file.
            let mut map = file_read();
            if map.remove(name).is_some() {
                file_write(&map)?;
            }
            return Ok(SecretLocation::Keyring);
        }
        let mut map = file_read();
        map.insert(name.to_string(), value.to_string());
        file_write(&map)?;
        Ok(SecretLocation::PrivateFile)
    }

    pub fn delete(name: &str) -> Result<(), String> {
        if let Some(e) = keyring(name) {
            let _ = e.delete_credential();
        }
        let mut map = file_read();
        if map.remove(name).is_some() {
            file_write(&map)?;
        }
        Ok(())
    }

    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").filter(|_| cfg!(windows)).map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("xenify").join("secrets.json"))
    }

    fn file_read() -> BTreeMap<String, String> {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn file_write(map: &BTreeMap<String, String>) -> Result<(), String> {
        let p = path().ok_or("no home directory")?;
        let dir = p.parent().unwrap();
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(map).unwrap();
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            // Created private from the start (no window where others could read it).
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&p)
                .map_err(|e| e.to_string())?;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
            f.write_all(json.as_bytes()).map_err(|e| e.to_string())
        }
        #[cfg(not(unix))]
        std::fs::write(&p, json).map_err(|e| e.to_string())
    }
}

/// The web build will keep secrets in the browser's local storage.
#[cfg(target_arch = "wasm32")]
mod backend {
    use super::SecretLocation;

    pub fn load(_name: &str) -> Option<(String, SecretLocation)> {
        None
    }
    pub fn save(_name: &str, _value: &str) -> Result<SecretLocation, String> {
        Err("secret storage isn't available in the browser yet".into())
    }
    pub fn delete(_name: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_and_stay_private() {
        let dir = std::env::temp_dir().join(format!("xenify-secrets-{}", std::process::id()));
        // SAFETY: this crate's only test that touches the environment.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let name = format!("test_key_{}", std::process::id());
        assert!(load(&name).is_none());
        let loc = save(&name, "sk-test-123").unwrap();
        assert_eq!(load(&name).map(|(v, _)| v).as_deref(), Some("sk-test-123"));
        if loc == SecretLocation::PrivateFile {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(dir.join("xenify/secrets.json")).unwrap().permissions().mode();
                assert_eq!(mode & 0o077, 0, "only the owner may read it");
            }
        }
        // A fresh lookup (not the cache) finds it too.
        CACHE.lock().unwrap().take();
        assert_eq!(load(&name).map(|(v, _)| v).as_deref(), Some("sk-test-123"));
        delete(&name).unwrap();
        CACHE.lock().unwrap().take();
        assert!(load(&name).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
