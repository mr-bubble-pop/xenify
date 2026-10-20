//! Preparing raster images for embedding: format and size, and re-encoding
//! when the file is too big to draw well or not a format the renderer decodes.

use image::{ImageFormat, ImageReader};
use std::io::Cursor;

/// An image ready to embed as an [`xenify_doc::ImageAsset`].
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Frame timings of an animated GIF, WebP or PNG (kept as it is).
    pub frames: Option<Vec<f64>>,
}

/// Default longest side kept when importing (larger images are scaled down).
pub const DEFAULT_MAX_SIDE: u32 = 4096;

/// Reads `bytes` (PNG, JPEG, WebP, GIF, BMP, …) and returns them ready to
/// embed: kept as they are when possible, otherwise re-encoded as PNG,
/// scaled down so the longest side is at most `max_side`.
pub fn prepare(bytes: &[u8], max_side: u32) -> Result<Prepared, String> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format().ok_or("not an image Xenify Vector can read")?;
    let (width, height) = reader.into_dimensions().map_err(|e| format!("couldn't read the image: {e}"))?;
    let keep = match format {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::WebP => Some("image/webp"),
        ImageFormat::Gif => Some("image/gif"),
        _ => None,
    };
    if let Some(mime) = keep
        && width.max(height) <= max_side
    {
        let frames = xenify_render::images::frame_delays(bytes);
        // A still GIF is stored as PNG (smaller to decode, same pixels).
        if frames.is_some() || format != ImageFormat::Gif {
            return Ok(Prepared { mime, width, height, data: bytes.to_vec(), frames });
        }
    }
    let img = image::load_from_memory(bytes).map_err(|e| format!("couldn't decode the image: {e}"))?;
    Ok(encode_png(img, max_side))
}

/// Raw RGBA pixels (e.g. from the clipboard) as an embeddable PNG.
pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>, max_side: u32) -> Result<Prepared, String> {
    let buf = image::RgbaImage::from_raw(width, height, rgba).ok_or("the image data has the wrong size")?;
    Ok(encode_png(image::DynamicImage::ImageRgba8(buf), max_side))
}

fn encode_png(img: image::DynamicImage, max_side: u32) -> Prepared {
    let img = if img.width().max(img.height()) > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut data = Vec::new();
    img.write_to(&mut Cursor::new(&mut data), ImageFormat::Png).expect("PNG encoding to memory cannot fail");
    Prepared { mime: "image/png", width: img.width(), height: img.height(), data, frames: None }
}

/// The pixels an image shows as `(width, height, rgba)`: its file decoded,
/// or for a video frame, the frame (from the installed decoder).
pub fn scene_pixels(img: &xenify_anim::SceneImage) -> Option<(u32, u32, Vec<u8>)> {
    match &img.video {
        Some((media, t)) => {
            let f = xenify_anim::media::frame(media, *t)?;
            Some((f.width, f.height, (*f.pixels).clone()))
        }
        None => {
            let d = match img.frame {
                Some(n) => xenify_render::images::animation_frames(&img.data)?.into_iter().nth(n as usize)?.0,
                None => image::load_from_memory(&img.data).ok()?.into_rgba8(),
            };
            let (w, h) = d.dimensions();
            Some((w, h, d.into_raw()))
        }
    }
}

/// The image as a file to embed (`mime`, bytes): its own file, or a video
/// frame or an animated image's current frame as a PNG.
pub fn scene_file(img: &xenify_anim::SceneImage) -> Option<(String, Vec<u8>)> {
    if img.video.is_none() && img.frame.is_none() {
        return Some((img.mime.clone(), img.data.to_vec()));
    }
    let (w, h, px) = scene_pixels(img)?;
    let mut out = Vec::new();
    image::RgbaImage::from_raw(w, h, px)?
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(("image/png".into(), out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut data = Vec::new();
        image::DynamicImage::new_rgba8(w, h).write_to(&mut Cursor::new(&mut data), ImageFormat::Png).unwrap();
        data
    }

    #[test]
    fn keeps_small_images_and_shrinks_big_ones() {
        let small = png(30, 20);
        let p = prepare(&small, 100).unwrap();
        assert_eq!((p.mime, p.width, p.height), ("image/png", 30, 20));
        assert_eq!(p.data, small, "unchanged bytes");

        let p = prepare(&png(400, 100), 100).unwrap();
        assert_eq!((p.width, p.height), (100, 25));
        assert!(prepare(b"definitely not an image", 100).is_err());
    }

    #[test]
    fn raw_pixels_become_png() {
        let p = from_rgba(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255], 100).unwrap();
        assert_eq!((p.mime, p.width, p.height), ("image/png", 2, 1));
        assert!(from_rgba(2, 2, vec![0; 4], 100).is_err());
    }
}
