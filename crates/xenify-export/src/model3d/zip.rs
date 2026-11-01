//! Just enough zip reading for 3MF: stored and deflated entries.

use super::{ModelError, invalid};

/// Largest entry we'll inflate (guards against zip bombs).
const MAX_ENTRY: usize = 1 << 30;

pub struct Zip<'a> {
    bytes: &'a [u8],
    /// (name, method, compressed size, local header offset)
    entries: Vec<(String, u16, usize, usize)>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

impl<'a> Zip<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, ModelError> {
        let bad = || invalid("this isn't a readable 3MF (zip) file");
        // The end-of-central-directory record is in the last 64 KiB + 22 bytes.
        let from = bytes.len().saturating_sub(65_557);
        let eocd = (from..bytes.len().saturating_sub(21))
            .rev()
            .find(|&o| u32_at(bytes, o) == Some(0x0605_4b50))
            .ok_or_else(bad)?;
        let count = u16_at(bytes, eocd + 10).ok_or_else(bad)? as usize;
        let mut at = u32_at(bytes, eocd + 16).ok_or_else(bad)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(bytes, at) != Some(0x0201_4b50) {
                return Err(bad());
            }
            let method = u16_at(bytes, at + 10).ok_or_else(bad)?;
            let size = u32_at(bytes, at + 20).ok_or_else(bad)? as usize;
            let name_len = u16_at(bytes, at + 28).ok_or_else(bad)? as usize;
            let extra = u16_at(bytes, at + 30).ok_or_else(bad)? as usize;
            let comment = u16_at(bytes, at + 32).ok_or_else(bad)? as usize;
            let local = u32_at(bytes, at + 42).ok_or_else(bad)? as usize;
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
            entries.push((String::from_utf8_lossy(name).into_owned(), method, size, local));
            at += 46 + name_len + extra + comment;
        }
        Ok(Self { bytes, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.0.as_str())
    }

    /// An entry's contents (names match without a leading `/`, ignoring case).
    pub fn read(&self, name: &str) -> Result<Vec<u8>, ModelError> {
        let name = name.trim_start_matches('/');
        let (_, method, size, local) = self
            .entries
            .iter()
            .find(|e| e.0.eq_ignore_ascii_case(name))
            .ok_or_else(|| invalid(format!("the 3MF file has no {name}")))?;
        let bad = || invalid(format!("{name} in the 3MF file is damaged"));
        if u32_at(self.bytes, *local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let n = u16_at(self.bytes, local + 26).ok_or_else(bad)? as usize;
        let e = u16_at(self.bytes, local + 28).ok_or_else(bad)? as usize;
        let start = local + 30 + n + e;
        let data = self.bytes.get(start..start + size).ok_or_else(bad)?;
        match method {
            0 => Ok(data.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_ENTRY).map_err(|_| bad()),
            m => Err(invalid(format!("{name} uses zip compression method {m}, which isn't supported"))),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A zip with the given (name, contents), deflated.
    pub fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let packed = miniz_oxide::deflate::compress_to_vec(data, 6);
            let local = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method, time, date, crc
            out.extend((packed.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&packed);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend((packed.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]); // extra, comment, disk, attributes
            central.extend(local.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_what_it_wrote() {
        let z = make_zip(&[("a.txt", b"hello hello hello"), ("3D/3dmodel.model", b"<model/>")]);
        let zip = Zip::new(&z).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a.txt", "3D/3dmodel.model"]);
        assert_eq!(zip.read("/3d/3DModel.model").unwrap(), b"<model/>");
        assert_eq!(zip.read("a.txt").unwrap(), b"hello hello hello");
        assert!(zip.read("missing").is_err());
        assert!(Zip::new(b"not a zip").is_err());
    }
}
