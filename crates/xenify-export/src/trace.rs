//! Image Trace: raster images (PNG/JPEG/WebP) → editable vector layers.

use kurbo::Shape;
use xenify_doc::{Geometry, Layer, bezier};

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
        // The pixels are already reduced to a few colours (see `flatten`).
        color_precision: 8,
        layer_difference: 8,
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
    let mut rgba = rgba;
    let key = flatten(&mut rgba, w, h, opts);
    let svg = trace_to_svg(rgba, w, h, opts)?;
    let svg_opts = crate::svg::SvgImportOptions { keep_groups: false, keep_images: false, place, ..Default::default() };
    let mut layers = crate::svg::import_svg_document(svg.as_bytes(), first_id, comp_w, comp_h, svg_opts)?.layers;
    if let Some(key) = key {
        // The cut-away background (and see-through parts) were painted in the key colour.
        layers.retain(|l| {
            !l.shapes().iter().any(|s| s.fill.as_ref().is_some_and(|f| near(f.color.value_at(0.0).to_rgba8(), key, 24)))
        });
    }

    // The traced image's full extent (in composition space).
    let boxes: Vec<kurbo::Rect> = layers.iter().map(layer_box).collect();
    let all = boxes.iter().copied().reduce(|a, b| a.union(b)).unwrap_or_default();
    if key.is_none() && opts.remove_background && layers.len() > 1 {
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

/// Palette sizes for `colors` 1 … 8.
const PALETTE: [usize; 8] = [2, 3, 4, 6, 8, 10, 12, 16];

fn dist2(a: [u8; 4], b: [u8; 4]) -> u32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2) as u32).sum()
}

fn near(a: [u8; 4], b: [u8; 4], tol: u32) -> bool {
    dist2(a, b) <= tol * tol
}

/// Prepares pixels for tracing: the background (one plain colour around the
/// edges) is flood-filled away from the edges, so a subject part of the same
/// colour inside an outline stays; then the rest is reduced to a palette of a
/// few colours. Pictures with soft shading otherwise become thousands of
/// shapes and take minutes. Returns the key colour that cut-away and
/// see-through pixels were painted in, when there are any.
fn flatten(rgba: &mut [u8], w: u32, h: u32, opts: &TraceOptions) -> Option<[u8; 4]> {
    let (w, h) = (w as usize, h as usize);
    let px = |rgba: &[u8], i: usize| [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]];
    let n = w * h;
    // Clear: already see-through, or cut away below.
    let mut clear: Vec<bool> = (0..n).map(|i| rgba[i * 4 + 3] < 128).collect();
    if opts.remove_background && w > 2 && h > 2 {
        let border: Vec<usize> =
            (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((1..h - 1).flat_map(|y| [y * w, y * w + w - 1])).collect();
        // The commonest border colour (in coarse buckets), if it's most of the border.
        let mut counts = std::collections::HashMap::<[u8; 3], (usize, [u32; 3])>::new();
        let mut opaque = 0;
        for &i in &border {
            let c = px(rgba, i);
            if c[3] < 128 {
                continue;
            }
            opaque += 1;
            let e = counts.entry([c[0] >> 4, c[1] >> 4, c[2] >> 4]).or_default();
            e.0 += 1;
            (0..3).for_each(|k| e.1[k] += c[k] as u32);
        }
        if let Some((count, sum)) = counts.values().max_by_key(|v| v.0)
            && *count * 10 >= opaque * 6
        {
            let bg =
                [(sum[0] / *count as u32) as u8, (sum[1] / *count as u32) as u8, (sum[2] / *count as u32) as u8, 255];
            let mut stack: Vec<usize> = border.iter().copied().filter(|&i| near(px(rgba, i), bg, 40)).collect();
            for &i in &stack {
                clear[i] = true;
            }
            while let Some(i) = stack.pop() {
                let (x, y) = (i % w, i / w);
                let mut go = |j: usize| {
                    if !clear[j] && near(px(rgba, j), bg, 40) {
                        clear[j] = true;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    go(i - 1);
                }
                if x + 1 < w {
                    go(i + 1);
                }
                if y > 0 {
                    go(i - w);
                }
                if y + 1 < h {
                    go(i + w);
                }
            }
        }
    }

    // A palette by k-means over a sample of the kept pixels (k-means++ start, fixed seed).
    let k = PALETTE[opts.colors.clamp(1, 8) as usize - 1];
    let kept: Vec<usize> = (0..n).filter(|&i| !clear[i]).collect();
    let step = (kept.len() / 40_000).max(1);
    let sample: Vec<[f32; 3]> = kept
        .iter()
        .step_by(step)
        .map(|&i| {
            let c = px(rgba, i);
            [c[0] as f32, c[1] as f32, c[2] as f32]
        })
        .collect();
    let d2 = |a: &[f32; 3], b: &[f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>();
    let mut centres: Vec<[f32; 3]> = Vec::new();
    if let Some(first) = sample.first() {
        centres.push(*first);
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut best: Vec<f32> = sample.iter().map(|s| d2(s, first)).collect();
        while centres.len() < k {
            let total: f32 = best.iter().sum();
            if total <= 1.0 {
                break; // fewer distinct colours than asked for
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let mut at = (seed >> 11) as f32 / (1u64 << 53) as f32 * total;
            let pick = best
                .iter()
                .position(|d| {
                    at -= d;
                    at <= 0.0
                })
                .unwrap_or(sample.len() - 1);
            let c = sample[pick];
            centres.push(c);
            best.iter_mut().zip(&sample).for_each(|(b, s)| *b = b.min(d2(s, &c)));
        }
        for _ in 0..12 {
            let mut sums = vec![([0f32; 3], 0usize); centres.len()];
            for s in &sample {
                let j = (0..centres.len()).min_by(|&a, &b| d2(s, &centres[a]).total_cmp(&d2(s, &centres[b]))).unwrap();
                (0..3).for_each(|i| sums[j].0[i] += s[i]);
                sums[j].1 += 1;
            }
            for (c, (sum, count)) in centres.iter_mut().zip(sums) {
                if count > 0 {
                    *c = sum.map(|v| v / count as f32);
                }
            }
        }
    }
    let palette: Vec<[u8; 4]> =
        centres.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]).collect();
    for &i in &kept {
        let c = px(rgba, i);
        let p = palette.iter().min_by_key(|p| dist2(c, **p)).copied().unwrap_or(c);
        rgba[i * 4..i * 4 + 4].copy_from_slice(&p);
    }
    if kept.len() == n {
        return None;
    }
    // A key colour far from every palette colour.
    let key = [
        [255, 0, 255, 255],
        [0, 255, 0, 255],
        [0, 255, 255, 255],
        [255, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 0, 0, 255],
    ]
    .into_iter()
    .max_by_key(|c| palette.iter().map(|p| dist2(*c, *p)).min().unwrap_or(u32::MAX))
    .unwrap();
    for i in (0..n).filter(|&i| clear[i]) {
        rgba[i * 4..i * 4 + 4].copy_from_slice(&key);
    }
    Some(key)
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
    fn white_inside_the_subject_stays_when_the_white_background_goes() {
        // A red ring with a white middle, on white, with soft noise like a painted picture.
        let mut img = image::RgbaImage::from_pixel(300, 300, image::Rgba([255, 255, 255, 255]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            let d = ((x as f64 - 150.0).powi(2) + (y as f64 - 150.0).powi(2)).sqrt();
            let n = ((x * 7 + y * 13) % 9) as u8;
            *p = if (60.0..90.0).contains(&d) {
                image::Rgba([200 + n, 40 + n, 30, 255])
            } else {
                image::Rgba([250 - n, 251 - n, 249 - n, 255])
            };
        }
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let layers = trace_to_layers(png.get_ref(), &TraceOptions::default(), 1, 300, 300).unwrap();
        let colours: Vec<[u8; 4]> =
            layers.iter().map(|l| l.shapes()[0].fill.as_ref().unwrap().color.value_at(0.0).to_rgba8()).collect();
        assert!(colours.iter().any(|c| c[0] > 230 && c[1] > 230), "the white middle is kept: {colours:?}");
        assert!(colours.iter().any(|c| c[0] > 180 && c[1] < 80), "the ring: {colours:?}");
        // Nothing spans the whole picture (the white around it is gone).
        assert!(
            layers.iter().all(|l| layer_box(l).width() < 250.0),
            "{:?}",
            layers.iter().map(layer_box).collect::<Vec<_>>()
        );
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
