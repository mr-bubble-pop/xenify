//! `xenify-server database setup`: a private PostgreSQL just for this
//! server (one computer, nothing installed on the system): the portable
//! build from the signed release list (the same part Xenify Hub installs),
//! its own folder, reachable from this computer only, with a random
//! password. The service starts and stops it (`[postgres]` in the settings
//! file).

use std::path::{Path, PathBuf};
use xenify_hub_core::pg::Pg;

pub struct Made {
    /// The server's `database` address.
    pub url: String,
    /// Its bin/postgres and data folder, for `[postgres]`.
    pub program: PathBuf,
    pub data: PathBuf,
}

/// Installs PostgreSQL into `dir` (`versions/`, `data/`) from the release
/// list at `source` (signed with Xenify's key, or one in `keys`), makes its
/// database and stops it again. Run as the user the service runs as
/// (PostgreSQL refuses to run as root).
pub fn setup(dir: &Path, source: &str, keys: &[[u8; 32]]) -> Result<Made, String> {
    let data = dir.join("data");
    if data.join("PG_VERSION").exists() {
        return Err(format!("{} already holds a database", data.display()));
    }
    let bytes = crate::hub::blocking_get(source, 16 << 20).map_err(|e| format!("the release list: {e}"))?;
    let signed: xenify_hub_core::Signed =
        serde_json::from_slice(&bytes).map_err(|_| "the release list can't be read")?;
    let mut trusted = xenify_hub_core::TRUSTED.to_vec();
    trusted.extend_from_slice(keys);
    let m = xenify_hub_core::manifest::verify(&signed, &trusted, None).map_err(|e| e.to_string())?;
    let p = m.product("postgres").ok_or("the release list has no PostgreSQL")?;
    let (os, arch) = xenify_hub_core::platform();
    let r = xenify_hub_core::choose::pick(p, &xenify_hub_core::Choice::default(), os, arch)
        .ok_or("the release list has no PostgreSQL for this computer")?;
    let d = r.download(os, arch).cloned().ok_or("the release list has no PostgreSQL for this computer")?;
    let store = xenify_hub_core::Store::new(dir.join("versions"));
    let downloads = dir.join("downloads");
    let part = downloads.join("postgres.part");
    eprintln!("Downloading PostgreSQL {}…", r.version);
    let file = crate::hub::copy_checked(&d.url, d.size, &d.sha256, &downloads, &part)
        .map_err(|e| format!("the download: {e}"))?;
    let installed = store.install_file("postgres", &r.version, &d, &file).map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&downloads);
    installed?;
    let program = store.program("postgres").ok_or("PostgreSQL didn't install")?;
    let pg = Pg::new(&program, data.clone()).ok_or("PostgreSQL isn't where it should be")?;
    let password = crate::random(24);
    pg.init(&password)?;
    pg.start()?;
    let made = pg.create_db(&password);
    pg.stop();
    made?;
    Ok(Made { url: Pg::url(&password), program, data })
}
