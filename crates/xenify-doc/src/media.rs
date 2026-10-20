//! Video and audio in a project: the files ([`MediaAsset`], embedded or
//! linked) and the layers that play them. This crate never decodes media:
//! the caller supplies the length, size and frame rate (from ffprobe).

use crate::{Id, Property};
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
}

/// Where a media file's bytes are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaSource {
    /// Inside the project file (it travels with it, but makes it big).
    Embedded {
        #[serde(with = "b64")]
        data: Arc<[u8]>,
    },
    /// A file on disk, by path (the project stays small).
    Linked { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: Id,
    pub name: String,
    pub kind: MediaKind,
    /// `video/mp4`, `audio/wav`, …
    pub mime: String,
    pub source: MediaSource,
    /// Length in seconds.
    pub duration: f64,
    /// Picture size in pixels (0 for audio).
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Frames per second (0 for audio).
    #[serde(default)]
    pub fps: f64,
    /// Whether a video has sound.
    #[serde(default)]
    pub has_audio: bool,
    /// Identifies the content (embedded: of the bytes; linked: of the path
    /// and size): keys decode caches.
    #[serde(with = "hex_u64")]
    pub hash: u64,
    /// How an audio clip was synthesized, when it was: its recipe, kept so
    /// it can be made again with other settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<serde_json::Value>,
}

impl MediaAsset {
    pub fn is_video(&self) -> bool {
        self.kind == MediaKind::Video
    }

    pub fn is_embedded(&self) -> bool {
        matches!(self.source, MediaSource::Embedded { .. })
    }
}

/// A video on the canvas: it plays from `source_in` seconds into the file,
/// at `speed`, from the layer's in point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoLayer {
    pub asset: Id,
    #[serde(default)]
    pub source_in: f64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default = "full")]
    pub volume: Property<f64>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub looped: bool,
}

/// Sound on the timeline (no picture): the same timing as a video, plus
/// fades.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioLayer {
    pub asset: Id,
    #[serde(default)]
    pub source_in: f64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default = "full")]
    pub volume: Property<f64>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub looped: bool,
    /// Seconds to fade in from silence and out to it.
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

fn one() -> f64 {
    1.0
}

fn full() -> Property<f64> {
    1.0.into()
}

/// Seconds into the file at `local` seconds after the layer's in point, or
/// `None` once a clip that doesn't loop has run out.
pub fn source_time(source_in: f64, speed: f64, looped: bool, duration: f64, local: f64) -> Option<f64> {
    if local < 0.0 {
        return None;
    }
    let t = source_in + local * speed;
    if duration <= 0.0 {
        return Some(t.max(0.0));
    }
    if looped {
        Some(t.rem_euclid(duration))
    } else if t < 0.0 || t > duration + 1e-9 {
        None
    } else {
        Some(t.min(duration))
    }
}

impl VideoLayer {
    pub fn new(asset: Id) -> Self {
        Self { asset, source_in: 0.0, speed: 1.0, volume: full(), muted: false, looped: false }
    }
}

impl AudioLayer {
    pub fn new(asset: Id) -> Self {
        Self {
            asset,
            source_in: 0.0,
            speed: 1.0,
            volume: full(),
            muted: false,
            looped: false,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }
}

/// Music (or any sound) playing across a whole presentation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Soundtrack {
    pub clips: Vec<SoundtrackClip>,
    /// While presenting waits at a pause, keep the soundtrack going
    /// (looping) instead of holding it.
    #[serde(default)]
    pub keep_playing: bool,
}

impl Soundtrack {
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
}

/// A clip of the soundtrack, starting `start` seconds into the presentation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundtrackClip {
    pub start: f64,
    pub audio: AudioLayer,
}

/// How long a clip plays on the timeline (before looping), in seconds.
pub fn play_length(duration: f64, source_in: f64, speed: f64) -> f64 {
    ((duration - source_in).max(0.0) / speed.abs().max(1e-6)).max(0.0)
}

mod hex_u64 {
    use super::*;
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{v:016x}"))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let s = String::deserialize(d)?;
        u64::from_str_radix(&s, 16).map_err(serde::de::Error::custom)
    }
}

mod b64 {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Arc<[u8]>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<[u8]>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(s).map(Into::into).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_play_from_their_in_point_at_their_speed() {
        assert_eq!(source_time(2.0, 1.0, false, 10.0, 3.0), Some(5.0));
        assert_eq!(source_time(0.0, 2.0, false, 10.0, 3.0), Some(6.0));
        assert_eq!(source_time(0.0, 1.0, false, 10.0, 11.0), None, "ran out");
        assert_eq!(source_time(0.0, 1.0, true, 10.0, 11.0), Some(1.0), "looped");
        assert_eq!(source_time(0.0, 1.0, false, 10.0, -1.0), None, "not started");
        assert_eq!(play_length(10.0, 2.0, 2.0), 4.0);
    }

    #[test]
    fn assets_round_trip() {
        let a = MediaAsset {
            id: Id(3),
            name: "clip".into(),
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            source: MediaSource::Embedded { data: vec![1u8, 2, 3].into() },
            duration: 2.0,
            width: 64,
            height: 48,
            fps: 25.0,
            has_audio: true,
            hash: 42,
            recipe: None,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<MediaAsset>(&json).unwrap(), a);
    }
}
