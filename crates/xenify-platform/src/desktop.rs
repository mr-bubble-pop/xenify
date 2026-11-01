//! OS services: the system clipboard (images, SVG, files), showing files in
//! the file manager, and printing through CUPS. Traits and parsing helpers
//! are portable; the implementations for desktops are in [`native`].

use crate::PlatformError;
use std::path::{Path, PathBuf};

/// What the system clipboard holds, richest form first.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipContent {
    /// Files copied in a file manager.
    Files(Vec<PathBuf>),
    /// An SVG document (copied from a vector editor or a browser).
    Svg(String),
    /// Raw pixels, straight-alpha RGBA8 (screenshots, images copied in a browser).
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    /// An encoded image file (PNG, JPEG, …).
    Encoded {
        bytes: Vec<u8>,
    },
    Text(String),
}

/// The system clipboard.
pub trait Clipboard {
    /// The clipboard's content, or `None` when it's empty or unreadable.
    fn read(&mut self) -> Option<ClipContent>;
    /// Puts an image on the clipboard.
    fn write_image(&mut self, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), PlatformError>;
}

/// Showing files with the desktop.
pub trait Shell {
    /// Opens the file manager with `path` selected.
    fn reveal(&self, path: &Path) -> Result<(), PlatformError>;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrintOptions {
    /// `None`: the system default printer.
    pub printer: Option<String>,
    pub copies: u32,
}

/// Printing files (images, PDFs).
pub trait Printer {
    /// Why printing isn't possible here, if it isn't.
    fn unavailable(&self) -> Option<String>;
    /// Installed printers, default first.
    fn printers(&self) -> Vec<String>;
    fn print_file(&self, path: &Path, opts: &PrintOptions) -> Result<(), PlatformError>;
}

/// Paths from a `text/uri-list` (or a plain list of paths), skipping comments
/// and non-file URIs.
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            if let Some(rest) = l.strip_prefix("file://") {
                // file:///path or file://localhost/path
                let path = rest.strip_prefix("localhost").unwrap_or(rest);
                path.starts_with('/').then(|| PathBuf::from(percent_decode(path)))
            } else if l.starts_with('/') {
                Some(PathBuf::from(l))
            } else {
                None
            }
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Printer names from `lpstat -a` output ("NAME accepting requests since …").
pub fn parse_lpstat(output: &str) -> Vec<String> {
    output.lines().filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect()
}

/// Folders shared into a container (dev container, Docker, Podman): pairs of
/// (path on the host computer, path in here), from `/proc/self/mountinfo`.
/// Empty when not in a container.
pub fn shared_folders(mountinfo: &str) -> Vec<(PathBuf, PathBuf)> {
    mountinfo
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let (root, mount) = (*f.get(3)?, *f.get(4)?);
            // Bind mounts of a folder (not whole filesystems or system files).
            (root != "/" && !root.starts_with("/var/lib/docker") && !root.contains("/containers/") && root.len() > 1)
                .then(|| (PathBuf::from(unescape(root)), PathBuf::from(unescape(mount))))
        })
        .filter(|(host, _)| !host.starts_with("/etc") && !host.starts_with("/proc"))
        .collect()
}

/// mountinfo escapes spaces and the like as `\040`.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let code: String = chars.by_ref().take(3).collect();
            if let Ok(v) = u8::from_str_radix(&code, 8) {
                out.push(v as char);
                continue;
            }
            out.push(c);
            out.push_str(&code);
        } else {
            out.push(c);
        }
    }
    out
}

/// `path` as seen from here, if it's a host path inside a shared folder.
pub fn host_to_local(path: &Path, shared: &[(PathBuf, PathBuf)]) -> Option<PathBuf> {
    shared.iter().find_map(|(host, local)| path.strip_prefix(host).ok().map(|rest| local.join(rest)))
}

/// `path` as the host computer sees it, if it's in a shared folder.
pub fn local_to_host(path: &Path, shared: &[(PathBuf, PathBuf)]) -> Option<PathBuf> {
    shared.iter().find_map(|(host, local)| path.strip_prefix(local).ok().map(|rest| host.join(rest)))
}

/// Whether this process runs in a container (so desktop services such as
/// the file manager and printers belong to another system).
pub fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

/// A screen (monitor or projector), in desktop pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// Screens from `xrandr --listmonitors` output, e.g.
/// ` 0: +*DP-1 3440/797x1440/334+0+0  DP-1`.
pub fn parse_xrandr_monitors(text: &str) -> Vec<Monitor> {
    text.lines()
        .skip_while(|l| !l.starts_with("Monitors"))
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            parts.next()?; // "0:"
            let flags_name = parts.next()?;
            let geometry = parts.next()?;
            let primary = flags_name.contains('*');
            let name = flags_name.trim_start_matches(['+', '*']).to_string();
            // WIDTH/mm x HEIGHT/mm +X +Y
            let (w, rest) = geometry.split_once('x')?;
            let width = w.split('/').next()?.parse().ok()?;
            let mut hxy = rest.split('+');
            let height = hxy.next()?.split('/').next()?.parse().ok()?;
            let x = hxy.next()?.parse().ok()?;
            let y = hxy.next()?.parse().ok()?;
            Some(Monitor { name, x, y, width, height, primary })
        })
        .collect()
}

/// Arguments for `lp` printing `path` scaled to fit the page.
pub fn lp_args(path: &Path, opts: &PrintOptions) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(p) = &opts.printer {
        args.extend(["-d".to_string(), p.clone()]);
    }
    args.extend(["-n".to_string(), opts.copies.max(1).to_string(), "-o".into(), "fit-to-page".into()]);
    args.push("--".into());
    args.push(path.to_string_lossy().into_owned());
    args
}

/// Desktop implementations (arboard, `xdg-open`/DBus, CUPS `lp`).
#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    use super::*;
    use std::process::{Command, Stdio};

    fn run(cmd: &str, args: &[&str]) -> Option<Vec<u8>> {
        let out = Command::new(cmd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        out.status.success().then_some(out.stdout)
    }

    /// The system clipboard via arboard, plus `wl-paste`/`xclip` on Linux for
    /// SVG (arboard can't read arbitrary MIME types).
    #[derive(Default)]
    pub struct SystemClipboard;

    impl SystemClipboard {
        /// The clipboard's content of type `mime` (Linux: `wl-paste`/`xclip`; elsewhere nothing).
        #[cfg(target_os = "linux")]
        pub fn read_mime(mime: &str) -> Option<Vec<u8>> {
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
            let types = if wayland { run("wl-paste", &["--list-types"]) } else { None }
                .or_else(|| run("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"]))?;
            if !String::from_utf8_lossy(&types).lines().any(|t| t.trim() == mime) {
                return None;
            }
            (if wayland { run("wl-paste", &["--no-newline", "--type", mime]) } else { None })
                .or_else(|| run("xclip", &["-selection", "clipboard", "-t", mime, "-o"]))
        }

        #[cfg(not(target_os = "linux"))]
        pub fn read_mime(_mime: &str) -> Option<Vec<u8>> {
            None
        }
    }

    impl Clipboard for SystemClipboard {
        fn read(&mut self) -> Option<ClipContent> {
            let mut cb = arboard::Clipboard::new().ok();
            if let Some(files) = cb.as_mut().and_then(|c| c.get().file_list().ok()).filter(|f| !f.is_empty()) {
                return Some(ClipContent::Files(files));
            }
            if let Some(uris) = Self::read_mime("text/uri-list") {
                let files = parse_uri_list(&String::from_utf8_lossy(&uris));
                if !files.is_empty() {
                    return Some(ClipContent::Files(files));
                }
            }
            if let Some(svg) = Self::read_mime("image/svg+xml") {
                return Some(ClipContent::Svg(String::from_utf8_lossy(&svg).into_owned()));
            }
            if let Some(img) = cb.as_mut().and_then(|c| c.get_image().ok()) {
                return Some(ClipContent::Image {
                    width: img.width as u32,
                    height: img.height as u32,
                    rgba: img.bytes.into_owned(),
                });
            }
            for mime in ["image/png", "image/jpeg", "image/webp"] {
                if let Some(bytes) = Self::read_mime(mime) {
                    return Some(ClipContent::Encoded { bytes });
                }
            }
            let text = cb.as_mut().and_then(|c| c.get_text().ok())?;
            Some(if text.trim_start().starts_with("<svg") || text.trim_start().starts_with("<?xml") {
                ClipContent::Svg(text)
            } else {
                ClipContent::Text(text)
            })
        }

        fn write_image(&mut self, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), PlatformError> {
            let img = arboard::ImageData { width: width as usize, height: height as usize, bytes: rgba.into() };
            arboard::Clipboard::new()
                .and_then(|mut c| c.set_image(img))
                .map_err(|e| PlatformError::Encoder(format!("clipboard: {e}")))
        }
    }

    /// Reveals files with the desktop's file manager.
    #[derive(Default)]
    pub struct SystemShell;

    impl Shell for SystemShell {
        fn reveal(&self, path: &Path) -> Result<(), PlatformError> {
            if in_container() {
                return Err(PlatformError::Encoder(
                    "Xenify Draw is running in a container, which can't open your file manager".into(),
                ));
            }
            let spawn = |cmd: &str, args: &[&str]| {
                Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
            };
            let path_str = path.to_string_lossy();
            #[cfg(target_os = "macos")]
            spawn("open", &["-R", &path_str])?;
            #[cfg(target_os = "windows")]
            spawn("explorer", &[&format!("/select,{path_str}")])?;
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                // Freedesktop FileManager1 selects the file (Nautilus, Dolphin, Thunar, Nemo…).
                let uri = format!("array:string:file://{path_str}");
                let shown = run(
                    "dbus-send",
                    &[
                        "--session",
                        "--print-reply",
                        "--dest=org.freedesktop.FileManager1",
                        "--type=method_call",
                        "/org/freedesktop/FileManager1",
                        "org.freedesktop.FileManager1.ShowItems",
                        &uri,
                        "string:",
                    ],
                )
                .is_some();
                if !shown {
                    let dir = path.parent().unwrap_or(path);
                    spawn("xdg-open", &[&dir.to_string_lossy()])?;
                }
            }
            Ok(())
        }
    }

    /// Printing through CUPS (`lp`, Linux and macOS).
    #[derive(Default)]
    pub struct CupsPrinter;

    impl Printer for CupsPrinter {
        fn unavailable(&self) -> Option<String> {
            if run("lpstat", &["-r"]).is_some() {
                return None;
            }
            Some(if in_container() {
                "Xenify Draw is running in a dev container, which can't reach your printers".into()
            } else {
                "printing needs CUPS (the lp and lpstat commands)".into()
            })
        }

        fn printers(&self) -> Vec<String> {
            let mut all = run("lpstat", &["-a"]).map_or_else(Vec::new, |o| parse_lpstat(&String::from_utf8_lossy(&o)));
            // `lpstat -d`: "system default destination: NAME"
            if let Some(d) = run("lpstat", &["-d"])
                .and_then(|o| String::from_utf8_lossy(&o).rsplit(':').next().map(|s| s.trim().to_string()))
                && let Some(i) = all.iter().position(|p| *p == d)
            {
                let default = all.remove(i);
                all.insert(0, default);
            }
            all
        }

        fn print_file(&self, path: &Path, opts: &PrintOptions) -> Result<(), PlatformError> {
            let out = Command::new("lp").args(lp_args(path, opts)).output()?;
            if out.status.success() {
                Ok(())
            } else {
                Err(PlatformError::Encoder(String::from_utf8_lossy(&out.stderr).trim().to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists() {
        let text =
            "# copied\nfile:///home/me/My%20Pictures/a.png\r\nfile://localhost/tmp/b.svg\nhttps://x.y/z\n/tmp/c.jpg\n";
        assert_eq!(
            parse_uri_list(text),
            vec![PathBuf::from("/home/me/My Pictures/a.png"), PathBuf::from("/tmp/b.svg"), PathBuf::from("/tmp/c.jpg")]
        );
    }

    #[test]
    fn monitor_list() {
        let out = "Monitors: 2\n 0: +*DisplayPort-3 3440/797x1440/334+0+0  DisplayPort-3\n 1: +HDMI-1 1920/510x1080/290+3440+0  HDMI-1\n";
        let m = parse_xrandr_monitors(out);
        assert_eq!(m.len(), 2);
        assert_eq!(
            m[0],
            Monitor { name: "DisplayPort-3".into(), x: 0, y: 0, width: 3440, height: 1440, primary: true }
        );
        assert_eq!((m[1].x, m[1].width, m[1].primary), (3440, 1920, false));
        assert!(parse_xrandr_monitors("garbage").is_empty());
    }

    #[test]
    fn container_paths() {
        let info = "36 35 254:0 /home/me/Desktop/xenify /workspaces/xenify rw,noatime - ext4 /dev/x rw\n\
                    37 35 254:0 /var/lib/docker/volumes/cfg/_data /home/vscode/.claude rw - ext4 /dev/x rw\n\
                    38 35 0:5 / /dev rw - tmpfs tmpfs rw\n\
                    39 35 254:0 /home/me/My\\040Pictures /pics ro - ext4 /dev/x ro\n";
        let shared = shared_folders(info);
        assert_eq!(
            shared,
            vec![
                (PathBuf::from("/home/me/Desktop/xenify"), PathBuf::from("/workspaces/xenify")),
                (PathBuf::from("/home/me/My Pictures"), PathBuf::from("/pics")),
            ]
        );
        assert_eq!(
            host_to_local(Path::new("/home/me/Desktop/xenify/art/a.png"), &shared),
            Some(PathBuf::from("/workspaces/xenify/art/a.png"))
        );
        assert_eq!(host_to_local(Path::new("/home/me/Downloads/a.png"), &shared), None);
        assert_eq!(
            local_to_host(Path::new("/workspaces/xenify/x.xenify"), &shared),
            Some(PathBuf::from("/home/me/Desktop/xenify/x.xenify"))
        );
    }

    #[test]
    fn printing_commands() {
        let out = "Office_Laser accepting requests since Mon 01 Jan\nPDF accepting requests since Tue\n";
        assert_eq!(parse_lpstat(out), vec!["Office_Laser", "PDF"]);
        let opts = PrintOptions { printer: Some("PDF".into()), copies: 2 };
        assert_eq!(
            lp_args(Path::new("/tmp/-frame.png"), &opts),
            ["-d", "PDF", "-n", "2", "-o", "fit-to-page", "--", "/tmp/-frame.png"]
        );
        let default = lp_args(Path::new("/tmp/f.png"), &PrintOptions::default());
        assert_eq!(default[..2], ["-n", "1"]);
    }
}
