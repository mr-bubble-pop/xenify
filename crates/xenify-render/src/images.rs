//! Decoded image cache: each embedded image is decoded once and handed to
//! Vello as the same [`peniko::ImageData`] every frame (so Vello's atlas keeps
//! the upload too).

use peniko::{self, Blob, ImageAlphaType, ImageData, ImageFormat};
use std::cell::RefCell;
use std::collections::HashMap;
use xenify_anim::SceneImage;

/// Decoded bytes kept per thread before the least recently used are dropped.
const BUDGET: usize = 512 << 20;

#[derive(Default)]
struct Cache {
    map: HashMap<u64, (Option<ImageData>, u64)>,
    bytes: usize,
    clock: u64,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    /// Pixels being painted, by the hash of the picture they replace (see [`set_live`]).
    static LIVE: RefCell<HashMap<u64, ImageData>> = RefCell::new(HashMap::new());
}

/// While a raster layer is painted: picture `hash` shows these straight-alpha
/// RGBA pixels instead of its file, until [`clear_live`]. A renderer that can
/// change an upload in place keeps the first upload and writes each change
/// into it (see [`live_blob`]); others draw these pixels.
pub fn set_live(hash: u64, width: u32, height: u32, rgba: std::sync::Arc<Vec<u8>>) {
    let data = ImageData {
        data: Blob::new(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    };
    LIVE.with_borrow_mut(|l| l.insert(hash, data));
}

/// The id of the pixels picture `hash` shows while painted (the screens' renderer keeps its upload
/// by this id and writes each change into it).
pub fn live_blob(hash: u64) -> Option<u64> {
    LIVE.with_borrow(|l| l.get(&hash).map(|d| d.data.id()))
}

pub fn clear_live(hash: u64) {
    LIVE.with_borrow_mut(|l| l.remove(&hash));
}

/// Whether any picture is being painted (the screens' renderer then frees old uploads at once).
pub fn painting() -> bool {
    LIVE.with_borrow(|l| !l.is_empty())
}

/// Picture `hash` is already decoded: these pixels (a painted raster layer's
/// new file), so it isn't decoded again.
pub fn prime(hash: u64, width: u32, height: u32, rgba: std::sync::Arc<Vec<u8>>) {
    let data = ImageData {
        data: Blob::new(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    };
    CACHE.with_borrow_mut(|c| {
        c.clock += 1;
        c.bytes += data.data.len();
        let key = adjusted_key(hash, &[]);
        if let Some((Some(old), _)) = c.map.insert(key, (Some(data), c.clock)) {
            c.bytes -= old.data.len();
        }
    });
}

/// The decoded image, or `None` if it can't be decoded.
pub fn decoded(img: &SceneImage) -> Option<ImageData> {
    // Colour-adjusted versions are cached separately (settings rounded so an
    // animated adjustment reuses nearby frames' work).
    let key = adjusted_key(img.hash, &img.adjust);
    if img.video.is_none()
        && let Some(live) = LIVE.with_borrow(|l| l.get(&img.hash).cloned())
    {
        if img.adjust.is_empty() {
            return Some(live);
        }
        let rgba = image::RgbaImage::from_raw(live.width, live.height, live.data.data().to_vec())?;
        return finish(rgba, &img.adjust);
    }
    CACHE.with_borrow_mut(|c| {
        c.clock += 1;
        let now = c.clock;
        if let Some((data, used)) = c.map.get_mut(&key) {
            *used = now;
            return data.clone();
        }
        let data = match (&img.video, img.frame) {
            (Some((media, t)), _) => video_frame(media, *t, &img.adjust),
            // An animated image: decode every frame once and keep them all.
            (None, Some(n)) => {
                let still = img.hash ^ frame_salt(n);
                let frames = animation_frames(&img.data).unwrap_or_default();
                let mut wanted = None;
                for (i, (rgba, _)) in frames.into_iter().enumerate() {
                    let data = finish(rgba, &img.adjust);
                    let k = adjusted_key(still ^ frame_salt(i as u32), &img.adjust);
                    if i as u32 == n {
                        // Kept below, like any picture.
                        wanted = data;
                        continue;
                    }
                    c.bytes += data.as_ref().map_or(0, |d| d.data.len());
                    c.map.insert(k, (data, now));
                }
                // A frame past the end (or not animated after all): the still picture.
                wanted.or_else(|| decode(&img.data, &img.adjust))
            }
            (None, None) => decode(&img.data, &img.adjust),
        };
        // A frame that couldn't be decoded yet may work next time (don't remember the failure).
        if data.is_none() && img.video.is_some() {
            return None;
        }
        c.bytes += data.as_ref().map_or(0, |d| d.data.len());
        c.map.insert(key, (data.clone(), now));
        while c.bytes > BUDGET && c.map.len() > 1 {
            let Some((&oldest, _)) = c.map.iter().filter(|(k, _)| **k != key).min_by_key(|(_, (_, u))| *u) else {
                break;
            };
            if let Some((Some(d), _)) = c.map.remove(&oldest) {
                c.bytes -= d.data.len();
            }
        }
        data
    })
}

/// The cache key for picture `hash` with colour adjustments `adjust`
/// (settings rounded so an animated adjustment reuses nearby frames' work).
fn adjusted_key(hash: u64, adjust: &[[f64; 4]]) -> u64 {
    adjust
        .iter()
        .flatten()
        .fold(hash, |h, v| (h ^ ((v * 500.0).round() as i64 as u64)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// What [`xenify_anim`] mixes into an animated image's hash for frame `n`.
fn frame_salt(n: u32) -> u64 {
    (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The frames of an animated GIF, WebP or PNG (each composited to the full
/// picture) and how long each shows, in seconds. `None` for a still image
/// or one that can't be read.
/// The most one animation's frames may take, all together (1 GiB: a
/// thousand frames of 500 × 500, or 120 of full HD). Every frame is the
/// whole picture, so a few hundred KB of GIF could otherwise ask for
/// hundreds of GB. Frames past it are left out (the animation stops sooner).
const MAX_ANIMATION_BYTES: u64 = 1 << 30;
/// The largest frame (256 MiB: 8192 × 8192).
const MAX_FRAME_BYTES: u64 = 256 << 20;

pub fn animation_frames(bytes: &[u8]) -> Option<Vec<(image::RgbaImage, f64)>> {
    animation_frames_within(bytes, MAX_ANIMATION_BYTES)
}

fn animation_frames_within(bytes: &[u8], cap: u64) -> Option<Vec<(image::RgbaImage, f64)>> {
    use image::{AnimationDecoder, ImageDecoder};
    use std::io::Cursor;
    // Each frame's size (the whole picture), checked before any is made.
    let fits = |(w, h): (u32, u32)| w as u64 * h as u64 * 4 <= MAX_FRAME_BYTES;
    let frames = if bytes.starts_with(b"GIF8") {
        let d = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
        if !fits(d.dimensions()) {
            return None;
        }
        d.into_frames()
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let d = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
        if !d.has_animation() || !fits(d.dimensions()) {
            return None;
        }
        d.into_frames()
    } else if bytes.starts_with(b"\x89PNG") {
        let d = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).ok()?;
        if !d.is_apng().ok()? || !fits(d.dimensions()) {
            return None;
        }
        d.apng().ok()?.into_frames()
    } else {
        return None;
    };
    let mut total = 0u64;
    let out: Vec<(image::RgbaImage, f64)> = frames
        .take(10_000)
        .map_while(Result::ok)
        .map_while(|f| {
            total += f.buffer().as_raw().len() as u64;
            (total <= cap).then_some(f)
        })
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            let ms = if d == 0 { 0.0 } else { n as f64 / d as f64 };
            // Browsers treat 0–10 ms as 100 ms; so do we.
            let secs = if ms <= 10.0 { 0.1 } else { ms / 1000.0 };
            (f.into_buffer(), secs)
        })
        .collect();
    (out.len() > 1).then_some(out)
}

/// Frame timings of an animated image (see [`animation_frames`]).
pub fn frame_delays(bytes: &[u8]) -> Option<Vec<f64>> {
    Some(animation_frames(bytes)?.into_iter().map(|(_, d)| d).collect())
}

/// A video frame from the installed decoder (see [`xenify_anim::media`]).
fn video_frame(media: &xenify_anim::media::MediaRef, t: f64, adjust: &[[f64; 4]]) -> Option<ImageData> {
    let f = xenify_anim::media::frame(media, t)?;
    let rgba = image::RgbaImage::from_raw(f.width, f.height, (*f.pixels).clone())?;
    finish(rgba, adjust)
}

fn decode(bytes: &[u8], adjust: &[[f64; 4]]) -> Option<ImageData> {
    finish(image::load_from_memory(bytes).ok()?.into_rgba8(), adjust)
}

fn finish(mut rgba: image::RgbaImage, adjust: &[[f64; 4]]) -> Option<ImageData> {
    let (width, height) = rgba.dimensions();
    if !adjust.is_empty() {
        // A lookup per 8-bit colour would miss hue/saturation (they mix
        // channels), so adjust each distinct pixel colour, remembering results.
        let mut seen: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
        for px in rgba.pixels_mut() {
            let rgb = [px[0], px[1], px[2]];
            let out = *seen.entry(rgb).or_insert_with(|| {
                let mut c = xenify_doc::Color::rgb(rgb[0] as f64 / 255.0, rgb[1] as f64 / 255.0, rgb[2] as f64 / 255.0);
                for [b, k, s, h] in adjust {
                    c = xenify_doc::adjust_color(c, *b, *k, *s, *h);
                }
                let [r, g, b, _] = c.to_rgba8();
                [r, g, b]
            });
            px[0] = out[0];
            px[1] = out[1];
            px[2] = out[2];
        }
    }
    Some(ImageData {
        data: Blob::new(std::sync::Arc::new(rgba.into_raw())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

/// Draws `img` at `transform` (layer space: one unit per pixel), clipped to
/// `clip` when given (a perspective-mapped frame). Undecodable images show as a
/// magenta outline.
pub fn draw(out: &mut impl crate::Painter, img: &SceneImage, transform: kurbo::Affine, clip: Option<&kurbo::BezPath>) {
    let Some(data) = decoded(img) else {
        let magenta = peniko::Color::from_rgba8(255, 0, 255, 255);
        out.stroke(&kurbo::Stroke::new(2.0), transform, magenta, None, &img.rect());
        return;
    };
    // The file's own size may differ from the recorded one (a replaced asset): stretch to fit.
    let fit = kurbo::Affine::scale_non_uniform(
        img.width as f64 / data.width.max(1) as f64,
        img.height as f64 / data.height.max(1) as f64,
    );
    let brush = peniko::ImageBrush::new(data);
    match clip {
        Some(path) => out.fill(peniko::Fill::NonZero, transform, &brush, Some(fit), path),
        None => out.draw_image(&brush, transform * fit),
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;

    /// A GIF whose picture is `w` × `h` but whose `n` frames are one pixel.
    fn gif(w: u16, h: u16, n: usize) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend(w.to_le_bytes());
        g.extend(h.to_le_bytes());
        g.extend([0, 0, 0]);
        for _ in 0..n {
            g.extend([0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            g.extend([0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x80]);
            g.extend([0, 0, 0, 255, 255, 255]);
            g.extend([2, 2, 0x44, 1, 0]);
        }
        g.push(0x3B);
        g
    }

    #[test]
    fn small_gifs_asking_for_much_memory_are_cut_short() {
        // 64 MB a frame, 50 frames: 3.2 GB, from a file of under 1 KB.
        let g = gif(4000, 4000, 50);
        assert!(g.len() < 1500);
        let frames = animation_frames_within(&g, 256 << 20).unwrap();
        assert_eq!(frames.len(), 4);
        // Ordinary ones keep every frame.
        assert_eq!(animation_frames(&gif(20, 20, 50)).unwrap().len(), 50);
        // A picture too big for one frame isn't animated.
        assert!(animation_frames(&gif(65_000, 65_000, 2)).is_none());
    }
}
