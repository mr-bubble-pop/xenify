//! Printing through CUPS (`lp`, `lpstat`) on Linux and Mac: documents are
//! printed as PDF. Everywhere (Windows too) the PDF can also be opened in the
//! computer's PDF viewer, to print from there.

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
    let path = std::env::temp_dir().join(format!("xenify-write-print-{}.pdf", std::process::id()));
    std::fs::write(&path, pdf).map_err(|e| e.to_string())?;
    let out = Command::new("lp")
        .args(["-d", printer, "-n", &copies.max(1).to_string()])
        .args(pages.map(|p| ["-P", p]).into_iter().flatten())
        .arg(&path)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Opens `pdf` in the computer's PDF viewer (to print from there).
pub fn open_in_viewer(pdf: &[u8]) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("xenify-write-print-{}.pdf", std::process::id()));
    std::fs::write(&path, pdf).map_err(|e| e.to_string())?;
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
}
