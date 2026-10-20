//! Decoded image cache: each embedded image is decoded once and handed to
//! Vello as the same [`peniko::ImageData`] every frame (so Vello's atlas keeps
//! the upload too).

use xenify_anim::SceneImage;
use std::cell::RefCell;
use std::collections::HashMap;
use vello::peniko::{self, Blob, ImageAlphaType, ImageData, ImageFormat};

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
}

/// The decoded image, or `None` if it can't be decoded.
pub fn decoded(img: &SceneImage) -> Option<ImageData> {
    // Colour-adjusted versions are cached separately (settings rounded so an
    // animated adjustment reuses nearby frames' work).
    let key = adjusted_key(img.hash, &img.adjust);
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
pub fn animation_frames(bytes: &[u8]) -> Option<Vec<(image::RgbaImage, f64)>> {
    use image::AnimationDecoder;
    use std::io::Cursor;
    let frames = if bytes.starts_with(b"GIF8") {
        image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames()
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let d = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
        if !d.has_animation() {
            return None;
        }
        d.into_frames()
    } else if bytes.starts_with(b"\x89PNG") {
        let d = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).ok()?;
        if !d.is_apng().ok()? {
            return None;
        }
        d.apng().ok()?.into_frames()
    } else {
        return None;
    };
    let out: Vec<(image::RgbaImage, f64)> = frames
        .take(10_000)
        .map_while(Result::ok)
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
pub fn draw(out: &mut vello::Scene, img: &SceneImage, transform: kurbo::Affine, clip: Option<&kurbo::BezPath>) {
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
