//! Video and audio through ffmpeg: reading a file's length, size and frame
//! rate (ffprobe), and decoding frames for video layers. For playback a
//! clip is decoded as one stream (frames read in order); jumping back, or
//! far ahead, restarts it at the new place.

use crate::PlatformError;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use xenify_anim::media::{Frame, FrameSource, MediaRef, frame_index};

/// What ffprobe says about a file.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
}

/// ffmpeg reads only files on this computer: never a web address, and no
/// other kind of input a file could lead it to.
const ONLY_FILES: [&str; 2] = ["-protocol_whitelist", "file"];

/// `path` when it's a file on this computer by its full path (what ffmpeg
/// may be given): not a web address or anything else ffmpeg would take.
pub fn local_file(path: &Path) -> Option<&Path> {
    let s = path.to_str()?;
    let url = s.contains("://") || s.split_once(':').is_some_and(|(scheme, _)| scheme.len() > 1 && !scheme.contains(['/', '\\']));
    (path.is_absolute() && !url && path.is_file()).then_some(path)
}

/// A folder only this account can use (others on the computer can't read
/// or plant files there): `$XDG_RUNTIME_DIR/xenify/<name>`, else the
/// user's cache folder. Made when missing, and narrowed to this account.
pub fn private_dir(name: &str) -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = env("XDG_RUNTIME_DIR")
        .filter(|d| d.is_absolute() && d.is_dir())
        .or_else(|| env("XDG_CACHE_HOME"))
        .or_else(|| env("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .or_else(|| {
            env("HOME").map(|h| if cfg!(target_os = "macos") { h.join("Library/Caches") } else { h.join(".cache") })
        })?;
    let dir = base.join("xenify").join(name);
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !std::fs::symlink_metadata(&dir).ok()?.is_dir() {
            return None;
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir)
}

/// `bytes` in `dir` as `name`: kept when the file there already holds
/// exactly them, else written afresh (through a temporary file).
pub fn write_once(dir: &Path, name: &str, bytes: &[u8]) -> Option<PathBuf> {
    let path = dir.join(name);
    let same = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() == bytes.len() as u64)
        && std::fs::read(&path).is_ok_and(|b| b == bytes);
    if !same {
        let tmp = dir.join(format!("{name}.{}.part", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let written = o.open(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, bytes));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path)
}

/// Reads a media file's facts with ffprobe.
pub fn probe(path: &Path) -> Result<MediaInfo, PlatformError> {
    let path = local_file(path).ok_or_else(|| PlatformError::Encoder("not a video or audio file on this computer".into()))?;
    let out = Command::new("ffprobe")
        .args(ONLY_FILES)
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,r_frame_rate,avg_frame_rate,duration"])
        .args(["-show_entries", "format=duration", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| PlatformError::Encoder(format!("could not start ffprobe ({e}); is ffmpeg installed?")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(PlatformError::Encoder(if err.is_empty() { "not a video or audio file".into() } else { err }));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| PlatformError::Encoder(format!("ffprobe: {e}")))?;
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let rate = |v: &serde_json::Value| -> f64 {
        let s = v.as_str().unwrap_or("0/1");
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let (n, d) = (n.parse::<f64>().unwrap_or(0.0), d.parse::<f64>().unwrap_or(1.0));
        if d > 0.0 { n / d } else { 0.0 }
    };
    let num = |v: &serde_json::Value| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64());
    let duration = num(&json["format"]["duration"]).or_else(|| video.and_then(|v| num(&v["duration"]))).unwrap_or(0.0);
    if video.is_none() && !audio {
        return Err(PlatformError::Encoder("the file has no picture or sound".into()));
    }
    Ok(MediaInfo {
        duration,
        width: video.and_then(|v| v["width"].as_u64()).unwrap_or(0) as u32,
        height: video.and_then(|v| v["height"].as_u64()).unwrap_or(0) as u32,
        fps: video
            .map(|v| {
                let r = rate(&v["avg_frame_rate"]);
                if r > 0.0 { r } else { rate(&v["r_frame_rate"]) }
            })
            .unwrap_or(0.0),
        has_video: video.is_some(),
        has_audio: audio,
    })
}

/// A file ffmpeg can read for `media`: its own path (a file on this
/// computer, by its full path), or (embedded) a copy written to a folder of
/// this account's own, reused while it holds the same bytes.
pub fn file_for(media: &MediaRef) -> Option<PathBuf> {
    if let Some(p) = &media.path {
        return local_file(Path::new(p)).map(Path::to_path_buf);
    }
    let data = media.data.as_ref()?;
    let dir = private_dir("media")?;
    let ext = match media.mime.as_str() {
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "audio/wav" => "wav",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        _ => "bin",
    };
    let name = format!("{:016x}.{ext}", media.hash);
    // Checked once per run (the clip is reopened on every seek).
    static CHECKED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    let path = dir.join(&name);
    if CHECKED.lock().is_ok_and(|c| c.contains(&path)) {
        return Some(path);
    }
    let path = write_once(&dir, &name, data)?;
    if let Ok(mut c) = CHECKED.lock() {
        c.push(path.clone());
    }
    Some(path)
}

/// Frames kept per clip (recent ones, for scrubbing back a little).
const KEEP: usize = 24;

struct Session {
    child: Child,
    out: ChildStdout,
    /// The index of the next frame the stream will give.
    next: u64,
    width: u32,
    height: u32,
    recent: VecDeque<(u64, Frame)>,
    done: bool,
}

impl Session {
    fn start(file: &Path, media: &MediaRef, from: u64) -> Option<Self> {
        let t = if media.fps > 0.0 { from as f64 / media.fps } else { 0.0 };
        let (w, h) = (media.width.max(2), media.height.max(2));
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-ss", &format!("{t:.6}")])
            .args(ONLY_FILES)
            .arg("-i")
            .arg(file)
            .args(["-an", "-sn", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let out = child.stdout.take()?;
        Some(Self { child, out, next: from, width: w, height: h, recent: VecDeque::new(), done: false })
    }

    fn read(&mut self) -> Option<Frame> {
        let mut buf = vec![0u8; self.width as usize * self.height as usize * 4];
        if self.out.read_exact(&mut buf).is_err() {
            self.done = true;
            return None;
        }
        let f = Frame { width: self.width, height: self.height, pixels: Arc::new(buf) };
        self.recent.push_back((self.next, f.clone()));
        if self.recent.len() > KEEP {
            self.recent.pop_front();
        }
        self.next += 1;
        Some(f)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decodes video frames with ffmpeg (one stream per clip).
#[derive(Default)]
pub struct FfmpegFrames {
    sessions: HashMap<u64, Session>,
}

impl FrameSource for FfmpegFrames {
    fn frame(&mut self, media: &MediaRef, t: f64) -> Option<Frame> {
        // Past the end: the last frame.
        let last = if media.duration > 0.0 && media.fps > 0.0 {
            ((media.duration * media.fps).ceil() as u64).saturating_sub(1)
        } else {
            u64::MAX
        };
        let want = frame_index(media.fps, t).min(last);
        if let Some(s) = self.sessions.get(&media.hash)
            && let Some((_, f)) = s.recent.iter().find(|(i, _)| *i == want)
        {
            return Some(f.clone());
        }
        let far = (media.fps * 2.0).max(10.0) as u64;
        let restart = match self.sessions.get(&media.hash) {
            Some(s) => want < s.next || want > s.next + far || (s.done && want >= s.next),
            None => true,
        };
        if restart {
            self.sessions.remove(&media.hash);
            let file = file_for(media)?;
            let s = Session::start(&file, media, want)?;
            self.sessions.insert(media.hash, s);
        }
        let s = self.sessions.get_mut(&media.hash)?;
        while s.next <= want {
            if s.read().is_none() {
                // Past the end: the last frame there is.
                return s.recent.back().map(|(_, f)| f.clone());
            }
        }
        s.recent.iter().find(|(i, _)| *i == want).map(|(_, f)| f.clone())
    }
}

/// Makes video layers show their frames and clips sound (with ffmpeg),
/// once per process.
pub fn install() {
    if !xenify_audio::has_pcm_source() {
        xenify_audio::set_pcm_source(Box::new(FfmpegPcm::default()));
    }
    if !xenify_anim::media::has_frame_source() && crate::native::FfmpegSink::available() {
        xenify_anim::media::set_frame_source(Box::new(FfmpegFrames::default()));
    }
}

/// Decodes clips' sound to 48 kHz stereo (WAV made by the program directly;
/// anything else with ffmpeg), keeping each once decoded.
#[derive(Default)]
pub struct FfmpegPcm {
    cache: HashMap<u64, Option<Arc<Vec<f32>>>>,
}

impl xenify_audio::PcmSource for FfmpegPcm {
    fn pcm(&mut self, media: &MediaRef) -> Option<Arc<Vec<f32>>> {
        self.cache.entry(media.hash).or_insert_with(|| decode_pcm(media).map(Arc::new)).clone()
    }
}

/// A clip's sound as 48 kHz stereo `f32`.
pub fn decode_pcm(media: &MediaRef) -> Option<Vec<f32>> {
    if let Some(pcm) = media.data.as_ref().and_then(|d| xenify_audio::read_wav(d)) {
        return Some(pcm);
    }
    let file = file_for(media)?;
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(ONLY_FILES)
        .arg("-i")
        .arg(&file)
        .args(["-vn", "-f", "f32le", "-ac", "2", "-ar", &xenify_audio::RATE.to_string(), "pipe:1"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(out.stdout.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
}

/// Plays sound on the computer's speakers (through ffmpeg: PulseAudio, else
/// ALSA). Dropping it stops.
pub struct Player {
    child: Child,
}

impl Player {
    /// Starts playing `pcm` (48 kHz stereo) now. `None` when there's no way
    /// to play sound here.
    pub fn start(pcm: Vec<f32>) -> Option<Self> {
        for (format, device) in [("pulse", "default"), ("alsa", "default")] {
            let Ok(mut child) = Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
                .args(["-f", "f32le", "-ar", &xenify_audio::RATE.to_string(), "-ac", "2", "-i", "pipe:0"])
                .args(["-f", format, device])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return None;
            };
            // A device that can't open makes ffmpeg quit at once.
            std::thread::sleep(std::time::Duration::from_millis(30));
            if child.try_wait().ok().flatten().is_some() {
                continue;
            }
            let Some(mut stdin) = child.stdin.take() else { continue };
            std::thread::Builder::new()
                .name("xenify-audio-out".into())
                .spawn(move || {
                    use std::io::Write;
                    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                    // Stops early (broken pipe) when the player is dropped.
                    let _ = stdin.write_all(&bytes);
                })
                .ok()?;
            return Some(Self { child });
        }
        None
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Puts the sound in `wav` into the video file `video` (AAC in MP4, Opus in
/// WebM), replacing it.
pub fn mux_audio(video: &Path, wav: &Path) -> Result<(), PlatformError> {
    let webm = video.extension().is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    let tmp = video.with_extension(if webm { "sound.webm" } else { "sound.mp4" });
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .arg("-i")
        .arg(wav)
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy"])
        .args(if webm { ["-c:a", "libopus", "-b:a", "160k"] } else { ["-c:a", "aac", "-b:a", "192k"] })
        .arg("-shortest")
        .arg(&tmp)
        .output()
        .map_err(|e| PlatformError::Encoder(format!("could not start ffmpeg ({e})")))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(PlatformError::Encoder(format!(
            "adding the sound failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    std::fs::rename(&tmp, video)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 s, 10 fps test pattern with a frame counter (ffmpeg's testsrc).
    fn clip(dir: &Path) -> Option<PathBuf> {
        let path = dir.join("testsrc.mp4");
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "testsrc=size=64x48:rate=10:duration=2", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(path)
    }

    #[test]
    fn only_local_files_reach_ffmpeg() {
        let dir = private_dir("media-test").expect("a private folder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let f = write_once(&dir, "a.wav", b"one").unwrap();
        assert!(local_file(&f).is_some());
        for bad in ["https://example.org/a.mp4", "concat:/a.mp4|/b.mp4", "relative.mp4", "/no/such/file.mp4"] {
            assert!(local_file(Path::new(bad)).is_none(), "{bad}");
        }
        // A file put there with other bytes is replaced, not used.
        std::fs::write(&f, b"planted").unwrap();
        let again = write_once(&dir, "a.wav", b"one").unwrap();
        assert_eq!(std::fs::read(again).unwrap(), b"one");
        let linked = MediaRef {
            asset: xenify_doc::Id(1),
            hash: 1,
            mime: "video/mp4".into(),
            path: Some("https://example.org/a.mp4".into()),
            data: None,
            fps: 0.0,
            width: 0,
            height: 0,
            duration: 0.0,
        };
        assert!(file_for(&linked).is_none());
        assert!(probe(Path::new("https://example.org/a.mp4")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probes_and_decodes_frames_in_order_and_after_seeking() {
        if !crate::native::FfmpegSink::available() {
            eprintln!("skipping: no ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("xenify-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = clip(&dir).expect("ffmpeg makes a test clip");
        let info = probe(&path).unwrap();
        assert_eq!((info.width, info.height), (64, 48));
        assert!((info.fps - 10.0).abs() < 1e-6);
        assert!((info.duration - 2.0).abs() < 0.1, "{}", info.duration);
        assert!(info.has_video);

        let media = MediaRef {
            asset: xenify_doc::Id(1),
            hash: 7,
            mime: "video/mp4".into(),
            path: Some(path.to_string_lossy().into()),
            data: None,
            fps: info.fps,
            width: info.width,
            height: info.height,
            duration: info.duration,
        };
        let mut dec = FfmpegFrames::default();
        let at = |dec: &mut FfmpegFrames, t: f64| dec.frame(&media, t).expect("a frame");
        let f0 = at(&mut dec, 0.0);
        assert_eq!((f0.width, f0.height, f0.pixels.len()), (64, 48, 64 * 48 * 4));
        let f5 = at(&mut dec, 0.5);
        assert_ne!(f0.pixels, f5.pixels, "the counter moved on");
        // Seeking back gives the same picture as playing through.
        let again = at(&mut dec, 0.0);
        assert_eq!(f0.pixels, again.pixels);
        let jump = at(&mut dec, 1.5);
        let mut fresh = FfmpegFrames::default();
        let through: Vec<Frame> = (0..=15).map(|i| at(&mut fresh, i as f64 / 10.0)).collect();
        assert_eq!(jump.pixels, through[15].pixels, "seek and play agree");
        // Past the end: the last frame.
        assert!(dec.frame(&media, 5.0).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
