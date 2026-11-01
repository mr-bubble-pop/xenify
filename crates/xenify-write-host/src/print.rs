//! Printing through CUPS (`lp`, `lpstat`) on Linux and Mac: documents are
//! printed as PDF. Everywhere (Windows too) the PDF can also be opened in the
//! computer's PDF viewer, to print from there.

use std::path::PathBuf;
use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Why printing isn't possible here, if it isn't.
pub fn unavailable() -> Option<String> {
    if cfg!(windows) {
        return Some("print from your PDF viewer: Open in PDF viewer".into());
    }
    if run("lpstat", &["-r"]).is_some() {
        return None;
    }
    let container = std::path::Path::new("/.dockerenv").exists() || std::env::var_os("REMOTE_CONTAINERS").is_some();
    Some(if container {
        "this is running in a container, which can't reach your printers".into()
    } else {
        "printing needs CUPS (the lp and lpstat commands)".into()
    })
}

/// Installed printers, the default first.
pub fn printers() -> Vec<String> {
    let mut all: Vec<String> = run("lpstat", &["-a"])
        .map(|o| o.lines().filter_map(|l| l.split_whitespace().next().map(str::to_string)).collect())
        .unwrap_or_default();
    if let Some(d) = run("lpstat", &["-d"]).and_then(|o| o.rsplit(':').next().map(|s| s.trim().to_string()))
        && let Some(i) = all.iter().position(|p| *p == d)
    {
        let d = all.remove(i);
        all.insert(0, d);
    }
    all
}

/// Prints `pdf` on `printer`: `copies` of it, only `pages` ("1-3,5") when given.
pub fn print_pdf(pdf: &[u8], printer: &str, copies: u32, pages: Option<&str>) -> Result<(), String> {
    let path = private_file(pdf)?;
    let out = Command::new("lp")
        .args(["-d", printer, "-n", &copies.max(1).to_string()])
        .args(pages.map(|p| ["-P", p]).into_iter().flatten())
        .arg(&path)
        .output()
        .map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    let out = out?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Opens `pdf` in the computer's PDF viewer (to print from there).
pub fn open_in_viewer(pdf: &[u8]) -> Result<(), String> {
    let path = private_file(pdf)?;
    open_outside(path.as_os_str())
}

/// A folder only this account can use (others on the computer can't read
/// or plant files there): `$XDG_RUNTIME_DIR/xenify/<name>`, else the
/// user's cache folder. Made when missing, and narrowed to this account.
pub fn private_dir(name: &str) -> Result<PathBuf, String> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = env("XDG_RUNTIME_DIR")
        .filter(|d| d.is_absolute() && d.is_dir())
        .or_else(|| env("XDG_CACHE_HOME"))
        .or_else(|| env("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .or_else(|| {
            env("HOME").map(|h| if cfg!(target_os = "macos") { h.join("Library/Caches") } else { h.join(".cache") })
        })
        .ok_or("there's no folder for temporary files")?;
    let dir = base.join("xenify").join(name);
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
        if !m.is_dir() {
            return Err(format!("{} isn't a folder", dir.display()));
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// `pdf` in a new file of its own in the private print folder; files left
/// there over an hour ago (opened in a viewer) are removed first.
fn private_file(pdf: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = private_dir("print")?;
    let hour = std::time::Duration::from_secs(3600);
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok());
        if old.is_some_and(|d| d > hour) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("print-{}-{n}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(&path).and_then(|mut f| f.write_all(pdf)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Opens a file, or a web or email link, with the computer's own program
/// for it. Never through a shell: on Windows `cmd /C start` would run
/// whatever follows a `&` or `|` in a link.
pub fn open_outside(target: &std::ffi::OsStr) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// A web or email link (`https://`, `http://`, `mailto:` when `mail`) made
/// safe to hand to the system: spaces, quotes and other characters a link
/// can't hold are percent-encoded. None for anything else.
pub fn clean_link(url: &str, mail: bool) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let web = ["https://", "http://"].iter().any(|p| lower.starts_with(p) && lower.len() > p.len());
    if !web && !(mail && lower.starts_with("mailto:")) {
        return None;
    }
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        let keep = b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Some(out)
}

/// Opens a web (or, with `mail`, email) link in the computer's own browser
/// or mail app.
pub fn open_link(url: &str, mail: bool) -> Result<(), String> {
    let url = clean_link(url, mail).ok_or(if mail {
        "only web and email links open from here"
    } else {
        "only web links open from here"
    })?;
    open_outside(std::ffi::OsStr::new(&url)).map_err(|e| format!("couldn't open the link: {e}"))
}

#[cfg(test)]
mod tests {
    use super::clean_link;

    #[test]
    fn links_are_cleaned_before_the_system_sees_them() {
        assert_eq!(clean_link("https://example.org/a?b=1&c=2", false).as_deref(), Some("https://example.org/a?b=1&c=2"));
        // Nothing a Windows command line would act on gets through as itself.
        let odd = clean_link("https://x/\"&calc.exe|^%x <y> `z`\"", false).unwrap();
        assert!(!odd.contains(['"', ' ', '|', '^', '<', '>', '`', '\\']), "{odd}");
        assert_eq!(clean_link("https://x/é", false).as_deref(), Some("https://x/%C3%A9"));
        assert!(clean_link("file:///etc/passwd", true).is_none());
        assert!(clean_link("javascript:alert(1)", true).is_none());
        assert!(clean_link("mailto:sam@example.org", false).is_none());
        assert!(clean_link("mailto:sam@example.org", true).is_some());
        assert!(clean_link("https://", false).is_none());
    }

    #[test]
    fn print_files_are_private() {
        let p = super::private_file(b"%PDF").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_file(p);
    }
}
