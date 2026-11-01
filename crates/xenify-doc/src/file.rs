//! `.xdraw` files: a zip holding `document.json` (the project, its pictures
//! and clips named by hash, see [`crate::blobs`]), each of those files once
//! as `media/<hash>` (stored as they are: they're compressed already), and
//! `preview.png`, a picture of the canvas for documents that show it, and
//! `info.json` (`{"animated": true}` when something in it moves).
//! A Xenify server keeps the same parts apart and zips them for downloads.

use crate::{Project, blobs};
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;

pub const DOCUMENT: &str = "document.json";
pub const PREVIEW: &str = "preview.png";
pub const INFO: &str = "info.json";

pub const TOO_BIG: &str = "This file is too big to open.";

/// What a zip may unpack to, all its entries together. Pictures and clips
/// are kept as they are (not squeezed), so a real file unpacks to about its
/// own size, its text to some tens of times less than it takes in the zip;
/// a made-up one squeezes far more (up to a thousand times), to fill the
/// memory. So: 64 times the file, and at least 64 MB.
pub struct Budget(u64);

impl Budget {
    pub fn for_file(len: usize) -> Budget {
        Budget((len as u64).saturating_mul(64).max(64 << 20))
    }

    /// An entry's bytes, read with what's left. `claimed`, what the zip says
    /// it holds, only decides refusing early; nothing is set aside on its
    /// word.
    pub fn read(&mut self, claimed: u64, r: impl Read) -> Result<Vec<u8>, String> {
        if claimed > self.0 {
            return Err(TOO_BIG.into());
        }
        let mut out = Vec::new();
        r.take(self.0 + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
        if out.len() as u64 > self.0 {
            return Err(TOO_BIG.into());
        }
        self.0 -= out.len() as u64;
        Ok(out)
    }
}

/// The project as an `.xdraw` file, with `preview` (PNG) and whether it's
/// `animated`, for documents that show it.
pub fn pack(p: &Project, preview: Option<&[u8]>, animated: bool) -> Result<Vec<u8>, String> {
    let json = blobs::by_reference(|| p.to_json());
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let squeeze = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let keep = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
    let e = |e: zip::result::ZipError| e.to_string();
    z.start_file(DOCUMENT, squeeze).map_err(e)?;
    z.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    for hash in blobs::refs(&json) {
        // One that never arrived stays named in the document, without bytes.
        let Some(data) = blobs::get(&hash) else { continue };
        z.start_file(format!("media/{hash}"), keep).map_err(e)?;
        z.write_all(&data).map_err(|e| e.to_string())?;
    }
    if let Some(png) = preview {
        z.start_file(PREVIEW, keep).map_err(e)?;
        z.write_all(png).map_err(|e| e.to_string())?;
    }
    if animated {
        z.start_file(INFO, squeeze).map_err(e)?;
        z.write_all(br#"{"animated":true}"#).map_err(|e| e.to_string())?;
    }
    Ok(z.finish().map_err(e)?.into_inner())
}

/// Reads an `.xdraw` file (or a project's JSON on its own).
pub fn unpack(bytes: &[u8]) -> Result<Project, String> {
    if bytes.first() == Some(&b'{') {
        return Project::from_json(std::str::from_utf8(bytes).map_err(|e| e.to_string())?).map_err(|e| e.to_string());
    }
    let mut z =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "That file isn’t a Xenify Draw canvas.".to_string())?;
    let mut budget = Budget::for_file(bytes.len());
    let not_one = || "That file isn’t a Xenify Draw canvas.".to_string();
    let json = {
        let f = z.by_name(DOCUMENT).map_err(|_| not_one())?;
        budget.read(f.size(), f)?
    };
    let json = String::from_utf8(json).map_err(|e| e.to_string())?;
    let named = blobs::refs(&json);
    let mut starts = std::collections::HashSet::new();
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(|e| e.to_string())?;
        // Two entries reading the same bytes: how a small zip claims a lot.
        if !starts.insert(f.header_start()) {
            return Err(TOO_BIG.into());
        }
        let Some(hash) = f.name().ok().and_then(|n| n.strip_prefix("media/").map(str::to_string)) else { continue };
        // Only what the canvas names, each once.
        if !named.contains(&hash) || blobs::get(&hash).is_some() {
            continue;
        }
        let data = budget.read(f.size(), f)?;
        // Named by what's in it: a file that isn't is left out.
        if blobs::sha256(&data) == hash {
            blobs::put(data);
        }
    }
    Project::from_json(&json).map_err(|e| e.to_string())
}

/// The picture of the canvas kept in an `.xdraw` file, if it has one.
pub fn preview(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let f = z.by_name(PREVIEW).ok()?;
    Budget::for_file(bytes.len()).read(f.size(), f).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Id, ImageAsset};

    #[test]
    fn pictures_go_beside_the_document_once() {
        let mut p = crate::Project::new();
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        for n in 0..2 {
            let a = ImageAsset::new(Id(900 + n), "a", "image/png", 4, 4, bytes.clone());
            let l = crate::Layer::image(Id(910 + n), "pic", &a, crate::Vec2::new(0.0, 0.0));
            p.assets.push(a);
            p.compositions[0].layers.push(l);
        }
        p.reserve_id(Id(920));
        let file = pack(&p, Some(b"png"), false).unwrap();
        let z = zip::ZipArchive::new(Cursor::new(&file)).unwrap();
        let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|n| n.to_string())).collect();
        assert!(names.iter().any(|n| n == DOCUMENT) && names.iter().any(|n| n == PREVIEW), "{names:?}");
        assert_eq!(names.iter().filter(|n| n.starts_with("media/")).count(), 1, "the same picture once: {names:?}");
        assert_eq!(preview(&file).as_deref(), Some(&b"png"[..]));
        assert!(file.len() < 2 * bytes.len(), "{}", file.len());
        assert_eq!(unpack(&file).unwrap(), p);
        assert!(unpack(b"not a canvas").is_err());
    }

    #[test]
    fn a_squeezed_bomb_is_refused_without_unpacking_it() {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let squeeze = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file(DOCUMENT, squeeze).unwrap();
        z.write_all(&vec![b' '; 80 << 20]).unwrap();
        let file = z.finish().unwrap().into_inner();
        assert!(file.len() < 1 << 20);
        assert_eq!(unpack(&file).unwrap_err(), TOO_BIG);
        // What the zip says it holds decides nothing but refusing early.
        let mut b = Budget(100);
        assert_eq!(b.read(1 << 45, &b"tiny"[..]).unwrap_err(), TOO_BIG);
        assert_eq!(b.read(4, &b"tiny"[..]).unwrap(), b"tiny");
        assert_eq!(b.read(0, &[0u8; 97][..]).unwrap_err(), TOO_BIG);
    }
}
