//! Raster images embedded in a project.
//!
//! An [`ImageAsset`] holds the original encoded bytes (PNG, JPEG, WebP, …) so
//! it travels with the project (see [`crate::blobs`]); image layers refer to it by id. This
//! crate never decodes images: the caller supplies the size and MIME type.

use crate::Id;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageAsset {
    pub id: Id,
    pub name: String,
    /// `image/png`, `image/jpeg`, `image/webp`, …
    pub mime: String,
    /// Size in pixels.
    pub width: u32,
    pub height: u32,
    /// FNV-1a hash of `data`: spots duplicates and keys render caches.
    #[serde(with = "hex_u64")]
    pub hash: u64,
    /// The encoded file (see [`crate::blobs`]: by name on a server and in
    /// `.xdraw` files, else base64 in the JSON).
    #[serde(with = "crate::blobs::field")]
    pub data: Arc<[u8]>,
    /// For an animated GIF, WebP or PNG: how long each frame shows
    /// (seconds), in order. `None` for a still image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<f64>>,
    /// For a picture drawn from a 3D model (Insert › 3D model): the model
    /// and how it was turned, so it can be turned again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Box<ModelSource>>,
}

/// The 3D model file a picture was drawn from, and how it was turned (degrees).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelSource {
    pub name: String,
    /// The model file as it was imported (kept beside the project, see [`crate::blobs`]).
    #[serde(with = "crate::blobs::field")]
    pub data: Arc<[u8]>,
    #[serde(default)]
    pub rot_x: f64,
    #[serde(default)]
    pub rot_y: f64,
    #[serde(default)]
    pub rot_z: f64,
}

impl ImageAsset {
    pub fn new(
        id: Id,
        name: impl Into<String>,
        mime: impl Into<String>,
        width: u32,
        height: u32,
        data: Vec<u8>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            mime: mime.into(),
            width,
            height,
            hash: hash_bytes(&data),
            data: data.into(),
            frames: None,
            model: None,
        }
    }

    /// One pass of the animation (seconds); 0 for a still image.
    pub fn animation_length(&self) -> f64 {
        self.frames.as_ref().map_or(0.0, |f| f.iter().sum())
    }

    /// Which frame shows `t` seconds into the animation (from 0), played
    /// once (then held on the last frame) or `looped`. `None` for a still.
    pub fn frame_at(&self, t: f64, looped: bool) -> Option<usize> {
        let frames = self.frames.as_ref().filter(|f| !f.is_empty())?;
        let total = self.animation_length();
        let mut t = t.max(0.0);
        if looped && total > 0.0 {
            t = t.rem_euclid(total);
        }
        let mut acc = 0.0;
        for (i, d) in frames.iter().enumerate() {
            acc += d;
            if t < acc {
                return Some(i);
            }
        }
        Some(frames.len() - 1)
    }
}

/// How an image layer plays an animated image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePlayback {
    /// 1 = as made, 2 = twice as fast.
    pub speed: f64,
    /// Over and over (else once, then it holds the last frame).
    pub looped: bool,
    /// Seconds into the animation it starts at (at the layer's in point).
    pub offset: f64,
}

impl Default for ImagePlayback {
    fn default() -> Self {
        Self { speed: 1.0, looped: true, offset: 0.0 }
    }
}

impl ImagePlayback {
    /// Seconds into the animation at `local` seconds after the layer's in point.
    pub fn time(&self, local: f64) -> f64 {
        self.offset + local * self.speed.max(0.0)
    }
}

impl ImageAsset {
    /// Whether this asset holds exactly `data`.
    pub fn is(&self, data: &[u8]) -> bool {
        self.hash == hash_bytes(data) && *self.data == *data
    }

    /// `data:<mime>;base64,…`, for SVG and Lottie export.
    pub fn data_uri(&self) -> String {
        format!("data:{};base64,{}", self.mime, base64::engine::general_purpose::STANDARD.encode(&self.data))
    }
}

/// 64-bit FNV-1a.
pub fn hash_bytes(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3))
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

#[cfg(test)]
mod tests {
    #[test]
    fn frames_play_once_or_loop() {
        let mut a = ImageAsset::new(Id(1), "spin", "image/gif", 2, 2, vec![1, 2, 3]);
        assert_eq!(a.frame_at(0.3, true), None, "a still");
        a.frames = Some(vec![0.1, 0.2, 0.1]);
        assert!((a.animation_length() - 0.4).abs() < 1e-12);
        assert_eq!(a.frame_at(0.05, false), Some(0));
        assert_eq!(a.frame_at(0.25, false), Some(1));
        assert_eq!(a.frame_at(9.0, false), Some(2), "holds the last frame");
        assert_eq!(a.frame_at(0.45, true), Some(0), "wraps round");
        let p = ImagePlayback { speed: 2.0, offset: 0.1, ..Default::default() };
        assert!((p.time(0.1) - 0.3).abs() < 1e-12);
    }
    use super::*;

    #[test]
    fn json_round_trip_and_identity() {
        let a = ImageAsset::new(Id(3), "dot", "image/png", 2, 1, vec![1, 2, 3, 250]);
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains(r#""data":"AQID+g==""#), "{json}");
        let back: ImageAsset = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        assert!(a.is(&[1, 2, 3, 250]) && !a.is(&[1, 2, 3]));
        assert_eq!(a.data_uri(), "data:image/png;base64,AQID+g==");
    }
}
