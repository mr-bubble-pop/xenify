//! Platform services behind traits so the rest of Xenify Draw stays portable.
//!
//! Native implementations live in [`native`]; browser implementations
//! (WebCodecs, File System Access API) will live alongside them later.

use std::path::Path;

pub mod desktop;
#[cfg(all(not(target_arch = "wasm32"), feature = "discover"))]
pub mod discover;
#[cfg(all(not(target_arch = "wasm32"), feature = "trust"))]
pub mod trust;
#[cfg(not(target_arch = "wasm32"))]
pub mod download;
#[cfg(not(target_arch = "wasm32"))]
pub mod media;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
pub mod secrets;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Encoder(String),
}

/// Output container/codec for rendered animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// H.264 in MP4 (opaque; dimensions rounded up to even).
    Mp4,
    /// VP9 in WebM with alpha channel.
    WebM,
    /// Animated GIF with an optimised palette.
    Gif,
    /// Numbered PNG files; the output path is used as a `printf` pattern,
    /// e.g. `frames/frame_%05d.png`.
    PngSequence,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 4] = [VideoFormat::Mp4, VideoFormat::WebM, VideoFormat::Gif, VideoFormat::PngSequence];

    /// Whether the format can store transparency.
    pub fn supports_alpha(self) -> bool {
        !matches!(self, VideoFormat::Mp4)
    }

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "MP4 (H.264)",
            VideoFormat::WebM => "WebM (VP9, alpha)",
            VideoFormat::Gif => "GIF",
            VideoFormat::PngSequence => "PNG sequence",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "mp4",
            VideoFormat::WebM => "webm",
            VideoFormat::Gif => "gif",
            VideoFormat::PngSequence => "png",
        }
    }

    /// Guesses the format from a file name's extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.extension() == ext)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoSettings {
    pub format: VideoFormat,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// Receives rendered frames (straight-alpha RGBA8, `width * height * 4` bytes).
pub trait VideoSink {
    fn write_frame(&mut self, rgba: &[u8]) -> Result<(), PlatformError>;
    /// Flushes and closes the output. Must be called for a valid file.
    fn finish(self: Box<Self>) -> Result<(), PlatformError>;
}

/// Reading and writing project files.
pub trait FileStore {
    fn read_to_string(&self, path: &Path) -> Result<String, PlatformError>;
    fn write(&self, path: &Path, contents: &[u8]) -> Result<(), PlatformError>;
}
