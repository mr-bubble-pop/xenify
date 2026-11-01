//! Minimal zip reading and writing (stored and deflated entries), enough
//! for .igdoc and .docx packages.

/// Largest entry we'll inflate (guards against zip bombs). A picture or
/// clip in a document is rarely over a few hundred MB.
const MAX_ENTRY: usize = 1 << 30;
/// The most all of a package's entries inflate to, together. Pictures are
/// stored as they are (already compressed), so a real package inflates to
/// about its own size plus a few times its text; this is twice the largest
/// entry, far over any real document, and stops a few MB from asking for
/// many GB.
const MAX_TOTAL: usize = 2 * MAX_ENTRY;

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
    #[error("This file is too big to open.")]
    TooBig,
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
    /// What's been inflated so far, and which entries (each counts once).
    inflated: std::cell::Cell<usize>,
    counted: std::cell::RefCell<Vec<bool>>,
    /// Lowercased name → the first entry of that name (a package can hold
    /// up to 65,535 entries; looking each up in a list would be slow).
    by_name: std::collections::HashMap<String, usize>,
    /// MAX_ENTRY and MAX_TOTAL (smaller in tests).
    limits: (usize, usize),
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
        // At most 65,535 entries (a u16), each with its own bytes (below).
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
        // Each entry's data is its own: entries pointing at the same bytes
        // (or overlapping) are how a small zip claims to hold a lot.
        let mut spans: Vec<(usize, usize)> = entries
            .iter()
            .map(|e| {
                let head = || -> Option<usize> {
                    if u32_at(bytes, e.local)? != 0x0403_4b50 {
                        return None;
                    }
                    let n = u16_at(bytes, e.local + 26)? as usize;
                    let x = u16_at(bytes, e.local + 28)? as usize;
                    // A Zip64 entry (not read here) says no size of its own.
                    if e.size == u32::MAX as usize {
                        return None;
                    }
                    e.local.checked_add(30 + n + x)?.checked_add(e.size)
                };
                (e.local, head().unwrap_or(e.local.saturating_add(1)))
            })
            .collect();
        spans.sort_unstable();
        if spans.windows(2).any(|w| w[1].0 < w[0].1) {
            return Err(bad());
        }
        let counted = std::cell::RefCell::new(vec![false; entries.len()]);
        let mut by_name = std::collections::HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_name.entry(e.name.to_ascii_lowercase()).or_insert(i);
        }
        Ok(Self { bytes, entries, inflated: std::cell::Cell::new(0), counted, by_name, limits: (MAX_ENTRY, MAX_TOTAL) })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn find(&self, name: &str) -> Option<&Entry> {
        self.find_at(name).map(|i| &self.entries[i])
    }

    fn find_at(&self, name: &str) -> Option<usize> {
        self.by_name.get(&name.trim_start_matches('/').to_ascii_lowercase()).copied()
    }

    /// An entry's contents (names match without a leading `/`, ignoring case).
    pub fn read(&self, name: &str) -> Result<Vec<u8>, ZipError> {
        let i = self.find_at(name).ok_or_else(|| ZipError::Missing(name.into()))?;
        let e = &self.entries[i];
        let first = !self.counted.borrow()[i];
        // What this package may still inflate to (an entry read again
        // doesn't count twice).
        let (max_entry, max_total) = self.limits;
        let left = if first { max_total.saturating_sub(self.inflated.get()) } else { max_total };
        let limit = max_entry.min(left);
        let bad = || ZipError::Damaged(name.into());
        if u32_at(self.bytes, e.local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let n = u16_at(self.bytes, e.local + 26).ok_or_else(bad)? as usize;
        let x = u16_at(self.bytes, e.local + 28).ok_or_else(bad)? as usize;
        let start = e.local + 30 + n + x;
        let data = self.bytes.get(start..start + e.size).ok_or_else(bad)?;
        let out = match e.method {
            0 if data.len() > limit => return Err(ZipError::TooBig),
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, limit).map_err(|e| match e.status {
                miniz_oxide::inflate::TINFLStatus::HasMoreOutput => ZipError::TooBig,
                _ => bad(),
            })?,
            _ => return Err(ZipError::Unsupported(name.into())),
        };
        if e.crc != 0 && crc32fast::hash(&out) != e.crc {
            return Err(bad());
        }
        if first {
            self.counted.borrow_mut()[i] = true;
            self.inflated.set(self.inflated.get() + out.len());
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

    /// Where the central directory starts, and each central entry's offset.
    fn central(z: &[u8]) -> Vec<usize> {
        let eocd = z.len() - 22;
        let count = u16_at(z, eocd + 10).unwrap() as usize;
        let mut at = u32_at(z, eocd + 16).unwrap() as usize;
        let mut out = Vec::new();
        for _ in 0..count {
            out.push(at);
            at += 46 + u16_at(z, at + 28).unwrap() as usize;
        }
        out
    }

    #[test]
    fn entries_sharing_bytes_are_refused() {
        let mut w = ZipWriter::new();
        w.add("a.txt", &vec![b'a'; 4096], true);
        w.add("b.txt", b"bbbb", true);
        let mut z = w.finish();
        assert!(ZipReader::new(&z).is_ok());
        // b's central entry points at a's bytes.
        let b = central(&z)[1];
        z[b + 42..b + 46].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(ZipReader::new(&z).err(), Some(ZipError::NotZip));
    }

    #[test]
    fn many_entries_naming_one_bomb_are_refused_quickly() {
        // 60,000 central entries all naming one deflated entry: once each
        // would inflate to 60,000 times its size.
        let mut w = ZipWriter::new();
        w.add("bomb", &vec![0u8; 1 << 20], true);
        let one = w.finish();
        let c = central(&one)[0];
        let entry = one[c..one.len() - 22].to_vec();
        let mut z = one[..c].to_vec();
        let cd = z.len() as u32;
        for i in 0..60_000u32 {
            let mut e = entry.clone();
            e[46..50].copy_from_slice(&(i % 9999).to_le_bytes()[..4]);
            z.extend(e);
        }
        let cd_len = z.len() as u32 - cd;
        z.extend(0x0605_4b50u32.to_le_bytes());
        z.extend([0u8; 4]);
        z.extend(60_000u16.to_le_bytes());
        z.extend(60_000u16.to_le_bytes());
        z.extend(cd_len.to_le_bytes());
        z.extend(cd.to_le_bytes());
        z.extend(0u16.to_le_bytes());
        let t = std::time::Instant::now();
        assert_eq!(ZipReader::new(&z).err(), Some(ZipError::NotZip));
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn inflating_past_the_total_is_refused() {
        let mut w = ZipWriter::new();
        for n in ["a", "b", "c"] {
            w.add(n, &vec![0u8; 1 << 20], true);
        }
        let z = w.finish();
        let mut r = ZipReader::new(&z).unwrap();
        r.limits = (2 << 20, (5 << 20) / 2);
        assert_eq!(r.read("a").unwrap().len(), 1 << 20);
        // Read again: still counted once.
        assert!(r.read("a").is_ok());
        assert!(r.read("b").is_ok());
        assert_eq!(r.read("c"), Err(ZipError::TooBig));
        // An entry over the per-entry limit.
        let mut r = ZipReader::new(&z).unwrap();
        r.limits = (1000, 1 << 30);
        assert_eq!(r.read("a"), Err(ZipError::TooBig));
    }

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
