//! Video frames: scenes say which frame of which file to show; a
//! [`FrameSource`] (ffmpeg on the desktop, installed by the app) decodes it.
//! This crate stays free of decoders so it builds for the web.

use std::sync::{Arc, Mutex};
use xenify_doc::{Id, MediaAsset, MediaSource};

/// A media file as a decoder needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaRef {
    pub asset: Id,
    pub hash: u64,
    pub mime: String,
    /// A file on disk, or the bytes themselves.
    pub path: Option<String>,
    pub data: Option<Arc<[u8]>>,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}

impl MediaRef {
    pub fn of(a: &MediaAsset) -> Self {
        let (path, data) = match &a.source {
            MediaSource::Linked { path } => (Some(path.clone()), None),
            MediaSource::Embedded { data } => (None, Some(data.clone())),
        };
        Self {
            asset: a.id,
            hash: a.hash,
            mime: a.mime.clone(),
            path,
            data,
            fps: a.fps,
            width: a.width,
            height: a.height,
            duration: a.duration,
        }
    }
}

/// One decoded frame: RGBA, 8 bits a channel, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
}

/// Decodes video frames.
pub trait FrameSource: Send {
    /// The frame showing `t` seconds into `media`.
    fn frame(&mut self, media: &MediaRef, t: f64) -> Option<Frame>;
}

static SOURCE: Mutex<Option<Box<dyn FrameSource>>> = Mutex::new(None);

/// Sets how video frames are decoded (the app does this at start).
pub fn set_frame_source(source: Box<dyn FrameSource>) {
    *SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
}

/// Whether frames can be decoded here.
pub fn has_frame_source() -> bool {
    SOURCE.lock().map(|s| s.is_some()).unwrap_or(false)
}

/// The frame showing `t` seconds into `media`, if a decoder is installed and it can.
pub fn frame(media: &MediaRef, t: f64) -> Option<Frame> {
    SOURCE.lock().ok()?.as_mut()?.frame(media, t)
}

/// The frame number showing at `t` seconds (frame `n` covers `n/fps ..= (n+1)/fps`).
pub fn frame_index(fps: f64, t: f64) -> u64 {
    if fps <= 0.0 { 0 } else { (t * fps + 1e-6).floor().max(0.0) as u64 }
}
