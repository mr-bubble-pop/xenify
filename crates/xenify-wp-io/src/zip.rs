//! Minimal zip reading and writing (stored and deflated entries), enough
//! for .igdoc and .docx packages.

/// Largest entry we'll inflate (guards against zip bombs).
const MAX_ENTRY: usize = 1 << 30;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ZipError {
    #[error("not a readable zip package")]
    NotZip,
    #[error("{0} is missing from the package")]
    Missing(String),
    #[error("{0} in the package is damaged")]
    Damaged(String),
    #[error("{0} uses an unsupported zip compression method")]
    Unsupported(String),
}

struct Entry {
    name: String,
    method: u16,
    size: usize,
    crc: u32,
    local: usize,
}

pub struct ZipReader<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

impl<'a> ZipReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, ZipError> {
        let bad = || ZipError::NotZip;
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
            let crc = u32_at(bytes, at + 16).ok_or_else(bad)?;
            let size = u32_at(bytes, at + 20).ok_or_else(bad)? as usize;
            let name_len = u16_at(bytes, at + 28).ok_or_else(bad)? as usize;
            let extra = u16_at(bytes, at + 30).ok_or_else(bad)? as usize;
            let comment = u16_at(bytes, at + 32).ok_or_else(bad)? as usize;
            let local = u32_at(bytes, at + 42).ok_or_else(bad)? as usize;
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
            entries.push(Entry { name: String::from_utf8_lossy(name).into_owned(), method, size, crc, local });
            at += 46 + name_len + extra + comment;
        }
        Ok(Self { bytes, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn find(&self, name: &str) -> Option<&Entry> {
        let name = name.trim_start_matches('/');
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// An entry's contents (names match without a leading `/`, ignoring case).
    pub fn read(&self, name: &str) -> Result<Vec<u8>, ZipError> {
        let e = self.find(name).ok_or_else(|| ZipError::Missing(name.into()))?;
        let bad = || ZipError::Damaged(name.into());
        if u32_at(self.bytes, e.local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let n = u16_at(self.bytes, e.local + 26).ok_or_else(bad)? as usize;
        let x = u16_at(self.bytes, e.local + 28).ok_or_else(bad)? as usize;
        let start = e.local + 30 + n + x;
        let data = self.bytes.get(start..start + e.size).ok_or_else(bad)?;
        let out = match e.method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_ENTRY).map_err(|_| bad())?,
            _ => return Err(ZipError::Unsupported(name.into())),
        };
        if e.crc != 0 && crc32fast::hash(&out) != e.crc {
            return Err(bad());
        }
        Ok(out)
    }

    pub fn read_string(&self, name: &str) -> Result<String, ZipError> {
        String::from_utf8(self.read(name)?).map_err(|_| ZipError::Damaged(name.into()))
    }
}

/// Builds a zip in memory.
#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file; `compress` deflates it (skip for PNG/JPEG, already compressed).
    pub fn add(&mut self, name: &str, data: &[u8], compress: bool) {
        let crc = crc32fast::hash(data);
        let packed;
        let (method, body): (u16, &[u8]) = if compress {
            packed = miniz_oxide::deflate::compress_to_vec(data, 6);
            (8, &packed)
        } else {
            (0, data)
        };
        let local = self.out.len() as u32;
        // 1980-01-01 00:00 (a fixed time keeps saves byte-identical).
        let (time, date) = (0u16, 0x21u16);
        let header = |b: &mut Vec<u8>| {
            b.extend(method.to_le_bytes());
            b.extend(time.to_le_bytes());
            b.extend(date.to_le_bytes());
            b.extend(crc.to_le_bytes());
            b.extend((body.len() as u32).to_le_bytes());
            b.extend((data.len() as u32).to_le_bytes());
            b.extend((name.len() as u16).to_le_bytes());
            b.extend(0u16.to_le_bytes()); // extra
        };
        self.out.extend(0x0403_4b50u32.to_le_bytes());
        self.out.extend(20u16.to_le_bytes()); // version needed
        self.out.extend(0x0800u16.to_le_bytes()); // flags: UTF-8 names
        header(&mut self.out);
        self.out.extend(name.as_bytes());
        self.out.extend(body);

        self.central.extend(0x0201_4b50u32.to_le_bytes());
        self.central.extend(20u16.to_le_bytes()); // version made by
        self.central.extend(20u16.to_le_bytes()); // version needed
        self.central.extend(0x0800u16.to_le_bytes());
        header(&mut self.central);
        self.central.extend([0u8; 10]); // comment len, disk, internal attrs, external attrs
        self.central.extend(local.to_le_bytes());
        self.central.extend(name.as_bytes());
        self.count += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        let cd = self.out.len() as u32;
        let cd_len = self.central.len() as u32;
        self.out.append(&mut self.central);
        self.out.extend(0x0605_4b50u32.to_le_bytes());
        self.out.extend([0u8; 4]);
        self.out.extend(self.count.to_le_bytes());
        self.out.extend(self.count.to_le_bytes());
        self.out.extend(cd_len.to_le_bytes());
        self.out.extend(cd.to_le_bytes());
        self.out.extend(0u16.to_le_bytes());
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_wrote() {
        let mut w = ZipWriter::new();
        w.add("a.txt", b"hello hello hello hello", true);
        w.add("media/x.png", b"\x89PNG....", false);
        let z = w.finish();
        let zip = ZipReader::new(&z).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a.txt", "media/x.png"]);
        assert_eq!(zip.read("A.TXT").unwrap(), b"hello hello hello hello");
        assert_eq!(zip.read("/media/x.png").unwrap(), b"\x89PNG....");
        assert!(matches!(zip.read("missing"), Err(ZipError::Missing(_))));
        assert!(ZipReader::new(b"not a zip").is_err());
        // A corrupted byte is caught by the CRC.
        let mut bad = z.clone();
        let at = bad.windows(4).position(|w| w == b"\x89PNG").unwrap();
        bad[at + 5] ^= 1;
        assert!(ZipReader::new(&bad).unwrap().read("media/x.png").is_err());
    }
}
