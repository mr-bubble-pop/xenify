//! Calls: the web view's camera, microphone and screen capture. The calls
//! themselves (encoding with WebCodecs, the relay) live in the screens
//! (`ui/calls.js`).

/// Linux's web view (WebKitGTK) keeps cameras and microphones off unless
/// asked. `XENIFY_FAKE_MEDIA=1` gives it made-up ones (for tests on
/// computers without any).
pub fn enable_media(app: &tauri::App) {
    #[cfg(target_os = "linux")]
    {
        use tauri::Manager;
        let fake = std::env::var_os("XENIFY_FAKE_MEDIA").is_some();
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.with_webview(move |wv| {
                use webkit2gtk::{SettingsExt, WebViewExt};
                if let Some(s) = wv.inner().settings() {
                    s.set_enable_media_stream(true);
                    s.set_enable_mock_capture_devices(fake);
                }
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

/// Whether the screen can be captured by the app itself: Linux on X11 (the
/// desktop's screen-sharing service is missing on some, XFCE for one).
#[tauri::command]
pub fn screen_x11() -> bool {
    cfg!(target_os = "linux")
        && std::env::var("XDG_SESSION_TYPE").map_or(std::env::var_os("WAYLAND_DISPLAY").is_none(), |t| t == "x11")
        && std::env::var_os("DISPLAY").is_some()
}

/// The screen being captured: ffmpeg, and its newest picture (JPEG).
#[derive(Default)]
pub struct Grab {
    on: std::sync::Mutex<Option<std::process::Child>>,
    latest: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

impl Drop for Grab {
    fn drop(&mut self) {
        if let Some(mut c) = self.on.lock().unwrap().take() {
            let _ = c.kill();
        }
    }
}

/// Width and height of the pictures (the screen scaled to 1280 wide).
const GRAB_W: u32 = 1280;

/// Starts capturing the whole screen (X11) with ffmpeg: 10 JPEG pictures a
/// second, 1280 wide. Returns the pictures' width and height.
#[tauri::command]
pub fn screen_grab_start(grab: tauri::State<'_, Grab>) -> Result<(u32, u32), String> {
    use std::io::Read;
    if !screen_x11() {
        return Err("this computer's desktop can't be captured".into());
    }
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    let (sw, sh) = screen_size(&display).unwrap_or((1920, 1080));
    let h = ((GRAB_W as f64 * sh as f64 / sw as f64).round() as u32).max(2) & !1;
    let mut on = grab.on.lock().unwrap();
    if let Some(mut c) = on.take() {
        let _ = c.kill();
    }
    let mut child = xenify_hub_core::command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "x11grab", "-framerate", "10", "-draw_mouse", "1"])
        .args(["-video_size", &format!("{sw}x{sh}"), "-i", &display])
        .args(["-vf", &format!("scale={GRAB_W}:{h}"), "-q:v", "6", "-f", "image2pipe", "-c:v", "mjpeg", "pipe:1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "screen sharing needs ffmpeg on this computer (install it from your software manager).".to_string()
            } else {
                e.to_string()
            }
        })?;
    let mut out = child.stdout.take().ok_or("ffmpeg didn't start")?;
    let latest = grab.latest.clone();
    latest.lock().unwrap().clear();
    std::thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
        let mut chunk = vec![0u8; 1 << 16];
        loop {
            let n = match out.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if let Some(pic) = newest_jpeg(&mut buf, &chunk[..n]) {
                *latest.lock().unwrap() = pic;
            }
        }
    });
    *on = Some(child);
    Ok((GRAB_W, h))
}

/// ffmpeg's pictures arrive as JPEGs one after another, in pieces: adds
/// `more` to `buf` and returns the newest whole picture in it, if any. Each
/// ends at FF D9 (inside a picture an FF is always followed by 00 or a
/// marker, so this is only ever the end).
fn newest_jpeg(buf: &mut Vec<u8>, more: &[u8]) -> Option<Vec<u8>> {
    let from = buf.len().saturating_sub(1);
    buf.extend_from_slice(more);
    let (mut start, mut i, mut newest) = (0, from, None);
    while i + 1 < buf.len() {
        if buf[i] == 0xFF && buf[i + 1] == 0xD9 {
            if buf[start..].starts_with(&[0xFF, 0xD8]) {
                newest = Some(buf[start..i + 2].to_vec());
            }
            start = i + 2;
            i += 2;
        } else {
            i += 1;
        }
    }
    buf.drain(..start);
    // A stream that never ends a picture: start again rather than grow.
    if buf.len() > 16 << 20 {
        buf.clear();
    }
    newest
}

/// The newest picture of the screen (JPEG; empty before the first).
#[tauri::command]
pub fn screen_grab_frame(grab: tauri::State<'_, Grab>) -> tauri::ipc::Response {
    tauri::ipc::Response::new(grab.latest.lock().unwrap().clone())
}

#[tauri::command]
pub fn screen_grab_stop(grab: tauri::State<'_, Grab>) {
    if let Some(mut c) = grab.on.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    grab.latest.lock().unwrap().clear();
}

/// The X11 screen's size, from xdpyinfo or xrandr (whichever is there).
fn screen_size(display: &str) -> Option<(u32, u32)> {
    let parse = |s: &str| -> Option<(u32, u32)> {
        let (w, h) = s.trim().split_once('x')?;
        let h: String = h.chars().take_while(char::is_ascii_digit).collect();
        Some((w.trim().parse().ok()?, h.parse().ok()?))
    };
    if let Ok(o) = xenify_hub_core::command("xdpyinfo").args(["-display", display]).output() {
        let t = String::from_utf8_lossy(&o.stdout);
        if let Some(l) = t.lines().find(|l| l.trim_start().starts_with("dimensions:"))
            && let Some(d) = l.split_whitespace().nth(1)
            && let Some(x) = parse(d)
        {
            return Some(x);
        }
    }
    let o = xenify_hub_core::command("xrandr").arg("--current").env("DISPLAY", display).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout);
    let l = t.lines().find(|l| l.contains("current "))?;
    let cur = l.split("current ").nth(1)?.split(',').next()?.replace(' ', "");
    parse(&cur)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pictures_are_found_however_the_pipe_cuts_them() {
        // Two tiny "JPEGs" (start, a stuffed FF 00, end), cut at every place.
        let a = [0xFF, 0xD8, 1, 0xFF, 0x00, 2, 0xFF, 0xD9];
        let b = [0xFF, 0xD8, 3, 4, 0xFF, 0xD9];
        let all: Vec<u8> = a.iter().chain(b.iter()).copied().collect();
        for cut in 0..=all.len() {
            let mut buf = Vec::new();
            let mut got = Vec::new();
            got.extend(super::newest_jpeg(&mut buf, &all[..cut]));
            got.extend(super::newest_jpeg(&mut buf, &all[cut..]));
            assert_eq!(got.last().map(Vec::as_slice), Some(&b[..]), "cut at {cut}");
            assert!(buf.is_empty());
        }
        // A real capture (when the test file exists): every picture found.
        if let Some(p) = std::env::var_os("XENIFY_TEST_MJPEG") {
            let data = std::fs::read(p).unwrap();
            let (mut buf, mut n) = (Vec::new(), 0);
            for piece in data.chunks(65536) {
                if super::newest_jpeg(&mut buf, piece).is_some() {
                    n += 1;
                }
            }
            assert!(n > 0 && buf.is_empty());
        }
    }

    #[test]
    fn x11_is_only_claimed_with_a_display() {
        // (Whatever the computer running the test, it never panics and is
        // false without a DISPLAY.)
        if std::env::var_os("DISPLAY").is_none() {
            assert!(!super::screen_x11());
        }
    }
}
