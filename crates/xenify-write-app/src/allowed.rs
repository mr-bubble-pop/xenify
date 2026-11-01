//! Which files the screens may read and write through the desktop app.
//!
//! The screens run in a web view, and anything that runs there (a script
//! slipped into a page from a server, say) can call the app's commands. So
//! the commands only touch files the person chose themselves: in the app's
//! Open or Save windows, by dropping them on the window, by opening them
//! with the app (the command line, "Open with", the Finder), or earlier
//! (the list is kept between runs, so Recent and linked files keep
//! working). Folders of the app's own (its caches) can be read as well, and
//! files of some kinds (pictures, canvases, data) may be read from beside a
//! chosen file (a document's pictures). Everything else is refused.
//!
//! Shared by Write, Draw, Calc and Plan (the others include this file).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Chosen files kept (the newest; older ones are forgotten).
const KEEP: usize = 500;

/// The files the screens may use.
#[derive(Default)]
pub struct Allowed(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// Where the list is kept between runs.
    store: Option<PathBuf>,
    /// Chosen files and folders, as their real paths, newest first.
    granted: Vec<PathBuf>,
    /// The app's own folders: readable.
    roots: Vec<PathBuf>,
    /// Kinds of file (lower-case endings) readable beside a chosen file.
    beside: &'static [&'static str],
}

/// `p` as its real path (links followed): one that doesn't exist yet as its
/// real folder and its name. None for a relative path or one ending in `..`.
pub fn canon(p: &Path) -> Option<PathBuf> {
    if !p.is_absolute() {
        return None;
    }
    if let Ok(c) = std::fs::canonicalize(p) {
        return Some(c);
    }
    let name = p.file_name()?;
    Some(std::fs::canonicalize(p.parent()?).ok()?.join(name))
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .and_then(|h| std::fs::canonicalize(h).ok())
}

impl Allowed {
    /// `beside`: the kinds of file readable next to a chosen one.
    pub fn new(beside: &'static [&'static str]) -> Self {
        Self(Mutex::new(Inner { beside, ..Inner::default() }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps the list in `store` (loading what's there), from now on.
    pub fn keep_at(&self, store: PathBuf) {
        let mut i = self.lock();
        let kept: serde_json::Value = std::fs::read(&store)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut granted: Vec<PathBuf> = kept["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(PathBuf::from))
            .collect();
        for g in std::mem::take(&mut i.granted).into_iter().rev() {
            granted.retain(|x| *x != g);
            granted.insert(0, g);
        }
        granted.truncate(KEEP);
        i.granted = granted;
        i.store = Some(store);
        save(&i);
    }

    /// Lets the screens read files in `dir` (one of the app's own folders).
    pub fn add_root(&self, dir: &Path) {
        if let Some(c) = canon(dir) {
            self.lock().roots.push(c);
        }
    }

    /// The person chose `path`: the screens may read and write it from now on.
    pub fn grant(&self, path: impl AsRef<Path>) {
        let Some(c) = canon(path.as_ref()) else { return };
        let mut i = self.lock();
        if i.granted.first() == Some(&c) {
            return;
        }
        i.granted.retain(|x| *x != c);
        i.granted.insert(0, c);
        i.granted.truncate(KEEP);
        save(&i);
    }

    /// Whether the screens may read (or, with `write`, write) `path`.
    pub fn allows(&self, path: &Path, write: bool) -> bool {
        let Some(c) = canon(path) else { return false };
        let i = self.lock();
        if i.store.as_ref().and_then(|s| canon(s)).is_some_and(|s| s == c) {
            return false;
        }
        if i.granted.contains(&c) {
            return true;
        }
        if write {
            return false;
        }
        if i.roots.iter().any(|r| c.starts_with(r)) {
            return true;
        }
        let kind = c.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if !kind.is_some_and(|k| i.beside.contains(&k.as_str())) || !c.is_file() {
            return false;
        }
        // Beside a chosen file: in its folder (or below), or in the folder
        // above it unless that's the home folder or above (`../pictures/`).
        let home = home();
        i.granted.iter().filter(|g| !g.is_dir()).filter_map(|g| g.parent()).any(|dir| {
            c.starts_with(dir)
                || dir
                    .parent()
                    .filter(|up| up.parent().is_some() && !home.as_ref().is_some_and(|h| h.starts_with(up)))
                    .is_some_and(|up| c.starts_with(up))
        })
    }

    /// `path` when the screens may use it (see [`Self::allows`]), else why not.
    pub fn check(&self, path: &str, write: bool) -> Result<PathBuf, String> {
        let p = Path::new(path);
        if self.allows(p, write) {
            return Ok(p.to_path_buf());
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        Err(format!("{name} can't be used: this app only uses files you chose. Pick it with Open or Save first"))
    }
}

fn save(i: &Inner) {
    let Some(store) = &i.store else { return };
    if let Some(d) = store.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let files: Vec<String> = i.granted.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let body = serde_json::json!({ "files": files }).to_string();
    let tmp = store.with_extension("saving");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, store);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn dir(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("xenify-allowed-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("work/pics")).unwrap();
        std::fs::create_dir_all(d.join("work/doc")).unwrap();
        std::fs::create_dir_all(d.join("other")).unwrap();
        Dir(std::fs::canonicalize(&d).unwrap())
    }

    #[test]
    fn only_chosen_files_and_pictures_beside_them() {
        let d = dir("chosen");
        let doc = d.0.join("work/doc/notes.md");
        std::fs::write(&doc, "hi").unwrap();
        for f in ["work/doc/a.png", "work/pics/b.png", "work/doc/secret.txt", "other/c.png"] {
            std::fs::write(d.0.join(f), "x").unwrap();
        }
        let a = Allowed::new(&["png"]);
        let s = |p: &str| d.0.join(p).display().to_string();
        assert!(a.check(&s("work/doc/notes.md"), false).is_err(), "nothing chosen yet");
        a.grant(&doc);
        assert!(a.check(&s("work/doc/notes.md"), false).is_ok());
        assert!(a.check(&s("work/doc/notes.md"), true).is_ok());
        assert!(a.check(&s("work/doc/../doc/notes.md"), true).is_ok(), "the same file by another way");
        assert!(a.check(&s("work/doc/a.png"), false).is_ok(), "a picture beside it");
        assert!(a.check(&s("work/pics/b.png"), false).is_ok(), "../pics/");
        assert!(a.check(&s("work/doc/a.png"), true).is_err(), "only read");
        assert!(a.check(&s("work/doc/secret.txt"), false).is_err(), "not a picture");
        assert!(a.check(&s("other/c.png"), false).is_err(), "two folders up");
        assert!(a.check("relative.md", false).is_err());
        assert!(a.check("/etc/passwd", false).is_err());
        // A new file chosen in Save: allowed before it exists.
        a.grant(d.0.join("other/new.xdraw"));
        assert!(a.check(&s("other/new.xdraw"), true).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_followed_before_checking() {
        let d = dir("links");
        let doc = d.0.join("work/doc/notes.md");
        std::fs::write(&doc, "hi").unwrap();
        let a = Allowed::new(&["png", "txt"]);
        a.grant(&doc);
        std::os::unix::fs::symlink("/etc/hostname", d.0.join("work/doc/host.png")).unwrap();
        assert!(a.check(&d.0.join("work/doc/host.png").display().to_string(), false).is_err());
    }

    #[test]
    fn kept_between_runs() {
        let d = dir("kept");
        let store = d.0.join("allowed.json");
        let (f1, f2, f3) = (d.0.join("work/one.xcalc"), d.0.join("work/two.xcalc"), d.0.join("other/three.xcalc"));
        for f in [&f1, &f2, &f3] {
            std::fs::write(f, "x").unwrap();
        }
        let a = Allowed::new(&[]);
        a.keep_at(store.clone());
        a.grant(&f1);
        a.grant(&f2);
        let b = Allowed::new(&[]);
        b.keep_at(store.clone());
        assert!(b.allows(&f1, true) && b.allows(&f2, true));
        assert!(!b.allows(&f3, false));
        // The list itself can't be written by the screens.
        b.grant(&store);
        assert!(!b.allows(&store, true));
    }

    #[test]
    fn own_folders_are_readable_only() {
        let d = dir("roots");
        let f = d.0.join("other/clip.mp4");
        std::fs::write(&f, "x").unwrap();
        let a = Allowed::new(&[]);
        a.add_root(&d.0.join("other"));
        assert!(a.allows(&f, false));
        assert!(!a.allows(&f, true));
    }
}
