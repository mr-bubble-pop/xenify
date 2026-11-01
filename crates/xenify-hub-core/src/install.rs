//! Where the Hub puts apps: `<root>/apps/<product>/<version>/`, with the
//! version in use and the one before it (for Roll back) noted beside them.
//! Older versions are removed once a newer one is in use.

use crate::manifest::{Download, Refused, check_file};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A product as it is on this computer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    pub version: String,
    /// What to start, inside the version's folder.
    pub run: String,
    /// The version before, kept for Roll back.
    #[serde(default)]
    pub previous: Option<Box<Installed>>,
}

pub struct Store {
    pub root: PathBuf,
}

#[derive(Debug)]
pub enum InstallError {
    Refused(Refused),
    Io(String),
    Unpack(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Refused(r) => write!(f, "{r}"),
            InstallError::Io(e) => write!(f, "couldn't write the app's files: {e}"),
            InstallError::Unpack(e) => write!(f, "the download couldn't be unpacked: {e}"),
        }
    }
}

fn io(e: std::io::Error) -> InstallError {
    InstallError::Io(e.to_string())
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn product_dir(&self, product: &str) -> PathBuf {
        // Product ids come from the signed list, but stay plain names here.
        let safe: String = product.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        self.root.join("apps").join(safe)
    }

    fn version_dir(&self, product: &str, version: &str) -> PathBuf {
        let safe: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || ".-+".contains(*c)).collect();
        self.product_dir(product).join(safe)
    }

    pub fn installed(&self, product: &str) -> Option<Installed> {
        let text = std::fs::read_to_string(self.product_dir(product).join("current.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The program to start for a product, if it's installed.
    pub fn program(&self, product: &str) -> Option<PathBuf> {
        let i = self.installed(product)?;
        Some(self.version_dir(product, &i.version).join(&i.run))
    }

    fn set_current(&self, product: &str, i: &Installed) -> Result<(), InstallError> {
        let dir = self.product_dir(product);
        std::fs::create_dir_all(&dir).map_err(io)?;
        let tmp = dir.join("current.json.part");
        std::fs::write(&tmp, serde_json::to_string_pretty(i).expect("serialises")).map_err(io)?;
        std::fs::rename(&tmp, dir.join("current.json")).map_err(io)
    }

    /// Installs `version` from its downloaded bytes (checked against the
    /// list first) and uses it from the next start; the one before is kept.
    pub fn install(&self, product: &str, version: &str, d: &Download, bytes: &[u8]) -> Result<Installed, InstallError> {
        check_file(d, bytes).map_err(InstallError::Refused)?;
        self.put(product, version, d, Source::Bytes(bytes))
    }

    /// The same, from a downloaded file (big downloads aren't held in memory).
    pub fn install_file(
        &self,
        product: &str,
        version: &str,
        d: &Download,
        file: &Path,
    ) -> Result<Installed, InstallError> {
        crate::manifest::check_path(d, file).map_err(InstallError::Refused)?;
        self.put(product, version, d, Source::File(file))
    }

    fn put(&self, product: &str, version: &str, d: &Download, src: Source) -> Result<Installed, InstallError> {
        let dest = self.version_dir(product, version);
        let part = dest.with_extension("part");
        let _ = std::fs::remove_dir_all(&part);
        std::fs::create_dir_all(&part).map_err(io)?;
        unpack(&d.url, &d.run, src, &part)?;
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&part, &dest).map_err(io)?;
        // Putting the version in use back again keeps the one kept for Roll back.
        let previous = match self.installed(product) {
            Some(b) if b.version == version => b.previous,
            b => b.map(|mut b| {
                b.previous = None;
                Box::new(b)
            }),
        };
        let now = Installed { version: version.into(), run: d.run.clone(), previous };
        self.set_current(product, &now)?;
        self.prune(product, &now);
        Ok(now)
    }

    /// Goes back to the version before (and keeps this one to come back to).
    pub fn roll_back(&self, product: &str) -> Result<Installed, InstallError> {
        let mut now = self.installed(product).ok_or_else(|| InstallError::Io("it isn't installed".into()))?;
        let mut back =
            *now.previous.take().ok_or_else(|| InstallError::Io("there's no earlier version kept".into()))?;
        back.previous = Some(Box::new(now));
        self.set_current(product, &back)?;
        Ok(back)
    }

    pub fn remove(&self, product: &str) -> Result<(), InstallError> {
        let dir = self.product_dir(product);
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(io)?;
        }
        Ok(())
    }

    /// Removes versions that are neither in use nor kept for Roll back.
    fn prune(&self, product: &str, keep: &Installed) {
        let keep: Vec<PathBuf> = std::iter::once(keep)
            .chain(keep.previous.as_deref())
            .map(|i| self.version_dir(product, &i.version))
            .collect();
        let Ok(entries) = std::fs::read_dir(self.product_dir(product)) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && !keep.contains(&p) {
                let _ = std::fs::remove_dir_all(p);
            }
        }
    }
}

enum Source<'a> {
    Bytes(&'a [u8]),
    File(&'a Path),
}

impl Source<'_> {
    fn reader(&self) -> Result<Box<dyn ReadSeek + '_>, InstallError> {
        Ok(match self {
            Source::Bytes(b) => Box::new(std::io::Cursor::new(*b)),
            Source::File(p) => Box::new(std::fs::File::open(p).map_err(io)?),
        })
    }
}

trait ReadSeek: std::io::Read + std::io::Seek {}
impl<T: std::io::Read + std::io::Seek> ReadSeek for T {}

/// Unpacks a download into `dir`: a .tar.gz or .zip, or else the program itself.
fn unpack(url: &str, run: &str, src: Source, dir: &Path) -> Result<(), InstallError> {
    let name = url.rsplit('/').next().unwrap_or(url).to_ascii_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let mut a = tar::Archive::new(flate2::read::GzDecoder::new(src.reader()?));
        a.set_preserve_permissions(true);
        // unpack_in refuses entries that would land outside `dir`.
        for entry in a.entries().map_err(|e| InstallError::Unpack(e.to_string()))? {
            let mut entry = entry.map_err(|e| InstallError::Unpack(e.to_string()))?;
            if !entry.unpack_in(dir).map_err(|e| InstallError::Unpack(e.to_string()))? {
                return Err(InstallError::Unpack("a file would go outside the app's folder".into()));
            }
        }
    } else if name.ends_with(".zip") {
        let mut z = zip::ZipArchive::new(src.reader()?).map_err(|e| InstallError::Unpack(e.to_string()))?;
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| InstallError::Unpack(e.to_string()))?;
            let rel = f
                .enclosed_name()
                .ok_or_else(|| InstallError::Unpack("a file would go outside the app's folder".into()))?;
            let out = dir.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(io)?;
                continue;
            }
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(io)?;
            }
            let mut w = std::fs::File::create(&out).map_err(io)?;
            std::io::copy(&mut f, &mut w).map_err(io)?;
            #[cfg(unix)]
            if let Some(mode) = f.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o755)).map_err(io)?;
            }
        }
    } else {
        let out = dir.join(run);
        if out.parent() != Some(dir) {
            return Err(InstallError::Unpack("the program's name isn't a plain file name".into()));
        }
        std::io::copy(&mut src.reader()?, &mut std::fs::File::create(&out).map_err(io)?).map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
        }
    }
    if !dir.join(run).is_file() {
        return Err(InstallError::Unpack(format!("the download has no “{run}”")));
    }
    Ok(())
}

/// For an app the Hub installed: a newer version it has put in place since
/// this one started (version, program to start). `exe` is the running
/// program, `<root>/apps/<product>/<version>/…`.
pub fn newer_installed(exe: &Path) -> Option<(String, PathBuf)> {
    let mut dir = exe.parent()?;
    // Up to the version folder (the program may be deeper inside it).
    loop {
        let parent = dir.parent()?;
        if parent.join("current.json").is_file() {
            break;
        }
        dir = parent;
    }
    let product_dir = dir.parent()?;
    let running = dir.file_name()?.to_str()?.to_string();
    let now: Installed = serde_json::from_str(&std::fs::read_to_string(product_dir.join("current.json")).ok()?).ok()?;
    (now.version != running).then(|| (now.version.clone(), product_dir.join(&now.version).join(&now.run)))
}
