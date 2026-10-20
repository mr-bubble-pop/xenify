//! Image Trace: raster images (PNG/JPEG/WebP) → editable vector layers.

use xenify_doc::{Geometry, Layer, bezier};
use kurbo::Shape;

/// Tracing settings (Illustrator-style knobs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceOptions {
    /// 1 (few colours, posterised) … 8 (many colours, close to the original).
    pub colors: u8,
    /// Ignore specks smaller than this many pixels (1 … 64): higher = cleaner.
    pub noise: usize,
    /// Smooth curves (true) or straight segments (false).
    pub smooth: bool,
    /// Drop the plain background shape that fills the whole image.
    pub remove_background: bool,
    /// Path simplification tolerance in composition pixels (0 = off).
    pub simplify: f64,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { colors: 5, noise: 8, smooth: true, remove_background: true, simplify: 0.75 }
    }
}

/// Largest side traced (bigger images are scaled down first: faster, cleaner).
const MAX_SIDE: u32 = 1024;

/// Decodes and (if needed) downscales an image to RGBA.
pub fn decode(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("couldn't read the image: {e}"))?;
    let img = if img.width().max(img.height()) > MAX_SIDE {
        img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((rgba.into_raw(), w, h))
}

/// Traces RGBA pixels into an SVG document.
pub fn trace_to_svg(rgba: Vec<u8>, width: u32, height: u32, opts: &TraceOptions) -> Result<String, String> {
    let img = vtracer::ColorImage { pixels: rgba, width: width as usize, height: height as usize };
    let config = vtracer::Config {
        color_mode: vtracer::ColorMode::Color,
        hierarchical: vtracer::Hierarchical::Stacked,
        filter_speckle: opts.noise.clamp(1, 64),
        color_precision: opts.colors.clamp(1, 8) as i32,
        layer_difference: 16,
        mode: if opts.smooth {
            visioncortex::PathSimplifyMode::Spline
        } else {
            visioncortex::PathSimplifyMode::Polygon
        },
        corner_threshold: 60,
        length_threshold: 4.0,
        max_iterations: 10,
        splice_threshold: 45,
        path_precision: Some(2),
    };
    Ok(vtracer::convert(img, config)?.to_string())
}

/// Traces an encoded image into path layers (ids from `first_id`), centred and
/// fitted in a `comp_w`×`comp_h` composition.
pub fn trace_to_layers(
    bytes: &[u8],
    opts: &TraceOptions,
    first_id: u64,
    comp_w: u32,
    comp_h: u32,
) -> Result<Vec<Layer>, String> {
    trace_layers(bytes, opts, first_id, comp_w, comp_h, None)
}

/// Like [`trace_to_layers`], but placed by `place` (the image's pixels →
/// composition), so the tracing lands exactly over the picture it came from.
pub fn trace_to_layers_at(
    bytes: &[u8],
    opts: &TraceOptions,
    first_id: u64,
    place: kurbo::Affine,
) -> Result<Vec<Layer>, String> {
    trace_layers(bytes, opts, first_id, 1, 1, Some(place))
}

fn trace_layers(
    bytes: &[u8],
    opts: &TraceOptions,
    first_id: u64,
    comp_w: u32,
    comp_h: u32,
    place: Option<kurbo::Affine>,
) -> Result<Vec<Layer>, String> {
    let (rgba, w, h) = decode(bytes)?;
    // The tracing is in (possibly shrunk) pixels of the original.
    let place = match place {
        Some(p) => {
            let (ow, _) = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .unwrap_or((w, h));
            Some(p * kurbo::Affine::scale(ow as f64 / w.max(1) as f64))
        }
        None => None,
    };
    let svg = trace_to_svg(rgba, w, h, opts)?;
    let svg_opts = crate::svg::SvgImportOptions { keep_groups: false, keep_images: false, place, ..Default::default() };
    let mut layers = crate::svg::import_svg_document(svg.as_bytes(), first_id, comp_w, comp_h, svg_opts)?.layers;

    // The traced image's full extent (in composition space).
    let boxes: Vec<kurbo::Rect> = layers.iter().map(layer_box).collect();
    let all = boxes.iter().copied().reduce(|a, b| a.union(b)).unwrap_or_default();
    if opts.remove_background && layers.len() > 1 {
        // Shapes spanning (almost) the whole image are the background.
        let is_bg = |b: &kurbo::Rect| b.width() >= all.width() * 0.97 && b.height() >= all.height() * 0.97;
        let keep: Vec<bool> = boxes.iter().map(|b| !is_bg(b)).collect();
        if keep.iter().any(|k| *k) {
            let mut i = 0;
            layers.retain(|_| {
                i += 1;
                keep[i - 1]
            });
        }
    }
    if opts.simplify > 0.0 {
        for l in &mut layers {
            if let Some(shapes) = l.shapes_mut() {
                for s in shapes {
                    if let Geometry::Path { path } = &mut s.geometry {
                        *path = bezier::simplify(&path.value_at(0.0), opts.simplify).into();
                    }
                }
            }
        }
    }
    // Friendlier names than vtracer's anonymous paths.
    for (i, l) in layers.iter_mut().enumerate() {
        l.meta.name = format!("Traced {}", i + 1);
    }
    if layers.is_empty() {
        return Err("nothing to trace — the image looks empty".into());
    }
    Ok(layers)
}

fn layer_box(l: &Layer) -> kurbo::Rect {
    let p = l.transform.position.value_at(0.0);
    l.shapes()
        .iter()
        .map(|s| xenify_anim::geometry_path(&s.geometry, 0.0).bounding_box() + p)
        .reduce(|a, b| a.union(b))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white 200×200 PNG with a red disc and a blue square.
    fn sample_png() -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(200, 200, image::Rgba([255, 255, 255, 255]));
        for y in 0..200 {
            for x in 0..200 {
                let (dx, dy) = (x as f64 - 60.0, y as f64 - 100.0);
                if dx * dx + dy * dy < 40.0 * 40.0 {
                    img.put_pixel(x, y, image::Rgba([220, 30, 30, 255]));
                }
                if (120..180).contains(&x) && (70..130).contains(&y) {
                    img.put_pixel(x, y, image::Rgba([30, 60, 220, 255]));
                }
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn traces_shapes_and_drops_the_background() {
        let layers = trace_to_layers(&sample_png(), &TraceOptions::default(), 1, 1000, 1000).unwrap();
        assert_eq!(layers.len(), 2, "red disc + blue square, background removed");
        let colours: Vec<_> =
            layers.iter().map(|l| l.shapes()[0].fill.as_ref().unwrap().color.value_at(0.0).to_hex()).collect();
        assert!(colours.iter().any(|c| c.starts_with("#d") || c.starts_with("#e")), "{colours:?}"); // red-ish
        // The disc is traced with curves and simplified to few points.
        let disc = &layers[0].shapes()[0];
        if let Geometry::Path { path } = &disc.geometry {
            assert!(bezier::point_count(&path.value_at(0.0)) < 30);
        }
        let with_bg = trace_to_layers(
            &sample_png(),
            &TraceOptions { remove_background: false, ..Default::default() },
            1,
            1000,
            1000,
        )
        .unwrap();
        assert_eq!(with_bg.len(), 3);
    }

    #[test]
    fn rejects_non_images() {
        assert!(trace_to_layers(b"hello", &TraceOptions::default(), 1, 100, 100).is_err());
    }
}

#[cfg(test)]
mod timing {
    #[test]
    #[ignore]
    fn time_trace_file() {
        let path = std::env::var("TRACE_FILE").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let t = std::time::Instant::now();
        let (rgba, w, h) = super::decode(&bytes).unwrap();
        let svg = super::trace_to_svg(rgba, w, h, &super::TraceOptions::default()).unwrap();
        eprintln!("trace: {:?}, svg {} KB, {} paths", t.elapsed(), svg.len() / 1024, svg.matches("<path").count());
        let t = std::time::Instant::now();
        let layers = super::trace_to_layers(&bytes, &super::TraceOptions::default(), 1, 1920, 1080).unwrap();
        eprintln!("full pipeline: {:?}, {} layers", t.elapsed(), layers.len());
    }
}
