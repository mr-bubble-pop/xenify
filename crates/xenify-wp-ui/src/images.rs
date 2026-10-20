//! Pictures: reading image files into the document and decoding them for drawing.

use xenify_wp_doc::Document;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vello::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

/// Extensions Insert › Image offers.
pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/// Decoded pictures by source, so each is decoded once.
#[derive(Default)]
pub struct ImageCache {
    map: HashMap<String, Option<ImageBrush>>,
}

/// Decodes image file bytes into an RGBA brush.
pub fn decode(bytes: &[u8]) -> Option<ImageBrush> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    let data = ImageData {
        data: Blob::new(Arc::new(img.into_raw())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    };
    Some(ImageBrush::new(data))
}

/// Pixel size of an image file.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    Some((img.width(), img.height()))
}

/// Where a picture's bytes come from: embedded in the document, or a file
/// (relative to the document's folder).
pub fn bytes_for(src: &str, doc: &Document, base: Option<&Path>) -> Option<Arc<Vec<u8>>> {
    if let Some(b) = doc.assets.get(src) {
        return Some(b.clone());
    }
    let path = PathBuf::from(src);
    let path = if path.is_absolute() { path } else { base?.join(path) };
    std::fs::read(path).ok().map(Arc::new)
}

/// Like [`bytes_for`] but with only the assets (usable off the UI thread).
pub fn bytes_for_parts(
    src: &str,
    assets: &std::collections::BTreeMap<String, Arc<Vec<u8>>>,
    base: Option<&Path>,
) -> Option<Arc<Vec<u8>>> {
    if let Some(b) = assets.get(src) {
        return Some(b.clone());
    }
    let path = PathBuf::from(src);
    let path = if path.is_absolute() { path } else { base?.join(path) };
    std::fs::read(path).ok().map(Arc::new)
}

impl ImageCache {
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn get(&mut self, src: &str, doc: &Document, base: Option<&Path>) -> Option<ImageBrush> {
        self.map.entry(src.to_string()).or_insert_with(|| bytes_for(src, doc, base).and_then(|b| decode(&b))).clone()
    }

    /// Decodes every picture the layout shows (so painting needs no I/O).
    pub fn prepare(&mut self, layout: &xenify_wp_layout::DocLayout, doc: &Document, base: Option<&Path>) {
        for i in &layout.images {
            self.get(&i.src, doc, base);
        }
    }

    pub fn lookup(&self, src: &str) -> Option<ImageBrush> {
        self.map.get(src).cloned().flatten()
    }
}

/// Reads a picture file into the document's assets (under `media/`);
/// returns its key and size in points (at 96 dpi).
pub fn embed(doc: &mut Document, path: &Path) -> Result<(String, f32, f32), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = size(&bytes).ok_or_else(|| format!("{} isn't a picture Xenify Write can read", path.display()))?;
    let name = path.file_name().map_or("image.png".into(), |n| n.to_string_lossy().replace(' ', "-"));
    let mut key = format!("media/{name}");
    let mut n = 2;
    while doc.assets.get(&key).is_some_and(|b| **b != bytes) {
        key = format!("media/{n}-{name}");
        n += 1;
    }
    doc.assets.insert(key.clone(), Arc::new(bytes));
    Ok((key, w as f32 * 0.75, h as f32 * 0.75))
}
