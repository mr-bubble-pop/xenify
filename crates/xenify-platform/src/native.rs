//! Native (desktop) implementations.

use crate::{FileStore, PlatformError, VideoFormat, VideoSettings, VideoSink};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

/// The local filesystem. Writes go to a temporary file first and are renamed
/// into place, so a crash never leaves a half-written project.
pub struct NativeFs;

impl FileStore for NativeFs {
    fn read_to_string(&self, path: &Path) -> Result<String, PlatformError> {
        Ok(std::fs::read_to_string(path)?)
    }

    fn write(&self, path: &Path, contents: &[u8]) -> Result<(), PlatformError> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, contents)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Encodes frames by piping raw RGBA into an `ffmpeg` child process.
pub struct FfmpegSink {
    child: Child,
    stdin: Option<ChildStdin>,
    frame_len: usize,
}

impl FfmpegSink {
    /// Whether an `ffmpeg` binary is available on `PATH`.
    pub fn available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    pub fn spawn(settings: &VideoSettings, output: &Path) -> Result<Self, PlatformError> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", settings.width, settings.height)])
            .args(["-r", &settings.fps.to_string()])
            .args(["-i", "-"]);
        match settings.format {
            VideoFormat::Mp4 => {
                cmd.args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"])
                    .args(["-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p"])
                    .args(["-movflags", "+faststart"]);
            }
            VideoFormat::WebM => {
                cmd.args(["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "24", "-row-mt", "1"]);
            }
            VideoFormat::Gif => {
                cmd.args([
                    "-vf",
                    "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=dither=sierra2_4a",
                ]);
            }
            VideoFormat::PngSequence => {
                cmd.args(["-f", "image2", "-c:v", "png"]);
            }
        }
        cmd.arg(output).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            PlatformError::Encoder(format!("could not start ffmpeg ({e}); is it installed and on PATH?"))
        })?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin, frame_len: (settings.width * settings.height * 4) as usize })
    }

    fn stderr(&mut self) -> String {
        let mut s = String::new();
        if let Some(mut e) = self.child.stderr.take() {
            let _ = e.read_to_string(&mut s);
        }
        s.trim().to_string()
    }
}

impl VideoSink for FfmpegSink {
    fn write_frame(&mut self, rgba: &[u8]) -> Result<(), PlatformError> {
        if rgba.len() != self.frame_len {
            return Err(PlatformError::Encoder(format!("frame is {} bytes, expected {}", rgba.len(), self.frame_len)));
        }
        let stdin = self.stdin.as_mut().expect("stdin is present until finish");
        if let Err(e) = stdin.write_all(rgba) {
            // ffmpeg exited early; its stderr says why.
            self.stdin = None;
            let _ = self.child.wait();
            let msg = self.stderr();
            return Err(PlatformError::Encoder(if msg.is_empty() { e.to_string() } else { msg }));
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), PlatformError> {
        drop(self.stdin.take()); // EOF tells ffmpeg to finalise the file.
        let status = self.child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(PlatformError::Encoder(format!("ffmpeg failed ({status}): {}", self.stderr())))
        }
    }
}

impl Drop for FfmpegSink {
    fn drop(&mut self) {
        // Cancelled/abandoned export: don't leave a zombie process behind.
        if self.stdin.take().is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
