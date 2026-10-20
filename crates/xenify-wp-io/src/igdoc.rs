//! `.igdoc`: a zip holding `document.json` (the model, versioned) and, later,
//! `media/` files. It keeps everything the editor can express.

use crate::zip::{ZipReader, ZipWriter};
use crate::{IoError, Loaded};
use xenify_wp_doc::Document;
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
const MIMETYPE: &str = "application/vnd.xenify-write";

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    format_version: u32,
    document: Document,
}

pub fn write(doc: &Document) -> Vec<u8> {
    let env = Envelope { format: MIMETYPE.into(), format_version: FORMAT_VERSION, document: doc.clone() };
    let json = serde_json::to_vec_pretty(&env).expect("documents serialise");
    let mut z = ZipWriter::new();
    // Stored first and uncompressed, so `file` can identify the package.
    z.add("mimetype", MIMETYPE.as_bytes(), false);
    z.add("document.json", &json, true);
    for (name, bytes) in &doc.assets {
        // PNG/JPEG/WebP are compressed already.
        let packed =
            !name.ends_with(".png") && !name.ends_with(".jpg") && !name.ends_with(".jpeg") && !name.ends_with(".webp");
        z.add(name, bytes, packed);
    }
    z.finish()
}

pub fn read(bytes: &[u8]) -> Result<Loaded, IoError> {
    let zip = ZipReader::new(bytes)?;
    let json = zip.read("document.json")?;
    let v: serde_json::Value = serde_json::from_slice(&json).map_err(|e| IoError::Damaged(e.to_string()))?;
    let version = v["format_version"].as_u64().unwrap_or(0) as u32;
    if version > FORMAT_VERSION {
        return Err(IoError::TooNew(version));
    }
    let env: Envelope = serde_json::from_value(v).map_err(|e| IoError::Damaged(e.to_string()))?;
    let mut doc = env.document;
    let mut warnings = Vec::new();
    let names: Vec<String> = zip.names().filter(|n| n.starts_with("media/")).map(str::to_string).collect();
    for name in names {
        match zip.read(&name) {
            Ok(b) => {
                doc.assets.insert(name, std::sync::Arc::new(b));
            }
            Err(e) => warnings.push(e.to_string()),
        }
    }
    Ok(Loaded { doc, warnings })
}
