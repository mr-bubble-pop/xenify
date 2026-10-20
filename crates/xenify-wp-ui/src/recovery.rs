//! Recovering unsaved work: while a document has unsaved changes it is
//! saved every minute to the user's data folder
//! (`write-recovery` in Xenify's data folder), never over the document itself.
//! Saving (or choosing not to save) removes it; if Xenify Write closes
//! without either, the work is offered back next time.

use xenify_wp_doc::Document;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Seconds between autosaves while there are unsaved changes.
pub const AUTOSAVE_SECS: u64 = 60;

/// What an autosave is of.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Info {
    /// The document's file (none: never saved).
    pub path: Option<PathBuf>,
    /// The document's title then.
    pub title: String,
    /// When it was taken (ms since 1970).
    pub time: u64,
    /// The process that took it (a running one's autosaves aren't orphans).
    pub pid: u32,
}

/// `write-recovery` in Xenify's data folder (`~/.local/share/xenify` on Linux).
pub fn root() -> Option<PathBuf> {
    Some(xenify_wp_check::data_dir().join("write-recovery")).filter(|p| p.is_absolute())
}

/// FNV-1a (stable across builds, unlike the standard hasher).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The autosave's name for a document: its path's hash, or this run's
/// session for one not saved yet.
fn key(path: Option<&Path>, session: u64) -> String {
    match path {
        Some(p) => {
            let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            format!("{:016x}", fnv(canon.to_string_lossy().as_bytes()))
        }
        None => format!("untitled-{session}"),
    }
}

/// The autosave of the document being edited.
#[derive(Default)]
pub struct Autosave {
    /// Where autosaves go (default: [`root`]); `None` with `off`: none.
    pub dir: Option<PathBuf>,
    /// No autosaves (tests).
    pub off: bool,
    /// This run (names autosaves of documents not saved yet).
    pub session: u64,
    /// The document's file.
    path: Option<PathBuf>,
    /// When the last autosave was taken, and of which revision.
    last: Option<(std::time::Instant, u64)>,
}

impl Autosave {
    /// Autosaves kept in `dir` (instead of the user's data folder).
    pub fn in_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir), ..Default::default() }
    }

    fn root(&mut self) -> Option<PathBuf> {
        if self.off {
            return None;
        }
        if self.dir.is_none() {
            self.dir = root();
        }
        self.dir.clone()
    }

    fn files(&mut self) -> Option<(PathBuf, PathBuf)> {
        if self.session == 0 {
            self.session = now_ms();
        }
        let root = self.root()?;
        let k = key(self.path.as_deref(), self.session);
        Some((root.join(format!("{k}.igdoc")), root.join(format!("{k}.json"))))
    }

    /// Now editing the document at `path` (or a new one).
    pub fn bind(&mut self, path: Option<&Path>) {
        self.path = path.map(Path::to_path_buf);
        self.last = None;
    }

    /// Every frame: autosaves when there are unsaved changes newer than the
    /// last autosave and a minute has passed.
    /// Autosaves for another open document of this run (`n` tells it apart).
    pub fn sibling(&self, n: u64) -> Self {
        Self {
            dir: self.dir.clone(),
            off: self.off,
            session: self.session.wrapping_add(n << 32),
            path: None,
            last: None,
        }
    }

    pub fn tick(&mut self, doc: &Document, dirty: bool) {
        let due = match self.last {
            None => true,
            Some((at, rev)) => rev != doc.revision && at.elapsed().as_secs() >= AUTOSAVE_SECS,
        };
        if dirty && due {
            let _ = self.write(doc);
        }
        if !dirty {
            self.last = Some((std::time::Instant::now(), doc.revision));
        }
    }

    /// Autosaves now.
    pub fn write(&mut self, doc: &Document) -> std::io::Result<()> {
        self.last = Some((std::time::Instant::now(), doc.revision));
        let Some((file, info)) = self.files() else { return Ok(()) };
        let bytes =
            xenify_wp_io::save(doc, xenify_wp_io::Format::Igdoc).map_err(|e| std::io::Error::other(e.to_string()))?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("igdoc.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &file)?;
        let meta =
            Info { path: self.path.clone(), title: doc.meta.title.clone(), time: now_ms(), pid: std::process::id() };
        std::fs::write(info, serde_json::to_vec_pretty(&meta).map_err(std::io::Error::other)?)
    }

    /// The work was saved, or the user chose not to keep it: the autosave goes.
    pub fn clear(&mut self) {
        if let Some((file, info)) = self.files() {
            let _ = std::fs::remove_file(file);
            let _ = std::fs::remove_file(info);
        }
        self.last = None;
    }

    /// An autosave of the document at `path` left by an earlier run, newer
    /// than the file.
    pub fn left_for(&mut self, path: &Path) -> Option<(Info, Document)> {
        let root = self.root()?;
        let k = key(Some(path), 0);
        let info: Info = serde_json::from_slice(&std::fs::read(root.join(format!("{k}.json"))).ok()?).ok()?;
        if info.pid == std::process::id() || running(info.pid) {
            return None;
        }
        let modified = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        if info.time <= modified {
            // Saved since: the autosave is stale.
            let _ = std::fs::remove_file(root.join(format!("{k}.igdoc")));
            let _ = std::fs::remove_file(root.join(format!("{k}.json")));
            return None;
        }
        let bytes = std::fs::read(root.join(format!("{k}.igdoc"))).ok()?;
        let doc = xenify_wp_io::load(&bytes, xenify_wp_io::Format::Igdoc).ok()?.doc;
        Some((info, doc))
    }

    /// Autosaves of never-saved documents left by runs that ended: (info, document, its key).
    pub fn orphans(&mut self) -> Vec<(Info, Document, String)> {
        let Some(root) = self.root() else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(&root) else { return Vec::new() };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(k) = name.strip_suffix(".json").filter(|k| k.starts_with("untitled-")) else { continue };
            let Some(info) = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Info>(&b).ok()) else {
                continue;
            };
            if info.pid == std::process::id() || running(info.pid) {
                continue;
            }
            let Some(doc) = std::fs::read(root.join(format!("{k}.igdoc")))
                .ok()
                .and_then(|b| xenify_wp_io::load(&b, xenify_wp_io::Format::Igdoc).ok())
                .map(|l| l.doc)
            else {
                continue;
            };
            out.push((info, doc, k.to_string()));
        }
        out.sort_by_key(|(i, _, _)| std::cmp::Reverse(i.time));
        out
    }

    /// Removes a left-over autosave (recovered or discarded).
    pub fn forget(&mut self, path: Option<&Path>, untitled_key: Option<&str>) {
        let Some(root) = self.root() else { return };
        let k = match (path, untitled_key) {
            (Some(p), _) => key(Some(p), 0),
            (None, Some(k)) => k.to_string(),
            (None, None) => return,
        };
        let _ = std::fs::remove_file(root.join(format!("{k}.igdoc")));
        let _ = std::fs::remove_file(root.join(format!("{k}.json")));
    }
}

/// Whether process `pid` is still running (Linux: /proc; elsewhere assume not).
fn running(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// The local time's offset from UTC at `secs` (since 1970), in seconds,
/// from the system's time zone file (`/etc/localtime`); 0 when unknown.
#[cfg(not(windows))]
pub fn local_offset(secs: i64) -> i64 {
    thread_local! {
        static ZONE: Option<Vec<u8>> = std::fs::read("/etc/localtime").ok();
    }
    ZONE.with(|z| z.as_deref().and_then(|b| tzif_offset(b, secs))).unwrap_or(0)
}

/// Windows has no zone file: its time zone settings, through jiff.
#[cfg(windows)]
pub fn local_offset(secs: i64) -> i64 {
    let at = jiff::Timestamp::from_second(secs).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    i64::from(jiff::tz::TimeZone::system().to_offset(at).seconds())
}

/// The offset a TZif file gives at `secs`: from its 64-bit data (version
/// 2+, what "slim" zone files hold), else its 32-bit data.
#[cfg_attr(windows, allow(dead_code))]
fn tzif_offset(b: &[u8], secs: i64) -> Option<i64> {
    if b.get(..4)? != b"TZif" {
        return None;
    }
    let n = |at: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize) };
    // Header: counts of isut, isstd, leap, time, type, char.
    let counts = |h: usize| -> Option<[usize; 6]> {
        Some([n(h + 20)?, n(h + 24)?, n(h + 28)?, n(h + 32)?, n(h + 36)?, n(h + 40)?])
    };
    let [isut, isstd, leap, time, typ, chars] = counts(0)?;
    let (base, tsize, [_, _, _, timecnt, typecnt, _]) = if *b.get(4)? >= b'2' {
        let v1 = 44 + time * 4 + time + typ * 6 + chars + leap * 8 + isstd + isut;
        (v1, 8usize, counts(v1)?)
    } else {
        (0, 4usize, [isut, isstd, leap, time, typ, chars])
    };
    let times = base + 44;
    let idx = times + timecnt * tsize;
    let types = idx + timecnt;
    let mut ty = 0usize;
    for i in 0..timecnt {
        let raw = b.get(times + i * tsize..times + (i + 1) * tsize)?;
        let t = if tsize == 8 {
            i64::from_be_bytes(raw.try_into().ok()?)
        } else {
            i32::from_be_bytes(raw.try_into().ok()?) as i64
        };
        if t > secs {
            break;
        }
        ty = *b.get(idx + i)? as usize;
    }
    if ty >= typecnt {
        return None;
    }
    let off = i32::from_be_bytes(b.get(types + ty * 6..types + ty * 6 + 4)?.try_into().ok()?);
    Some(off as i64)
}

/// "Today 14:03", "Yesterday 09:12", "10 Oct 09:12", "10 Oct 2025" (local time).
pub fn when(ms: u64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let secs = (ms / 1000) as i64;
    let secs = secs + local_offset(secs);
    let civil = |secs: i64| {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        (y, m, d, rem / 3600, rem / 60 % 60, days)
    };
    let (y, m, d, hh, mm, day) = civil(secs);
    let now = (now_ms() / 1000) as i64;
    let (ny, _, _, _, _, today) = civil(now + local_offset(now));
    let time = format!("{hh:02}:{mm:02}");
    let month = MONTHS[(m - 1).clamp(0, 11) as usize];
    if day == today {
        format!("Today {time}")
    } else if day + 1 == today {
        format!("Yesterday {time}")
    } else if y == ny {
        format!("{d} {month} {time}")
    } else {
        format!("{d} {month} {y}")
    }
}

/// Milliseconds since 1970 from an ISO 8601 UTC time ("2026-10-10T14:03:00Z").
pub fn parse_iso(s: &str) -> Option<u64> {
    let (d, t) = s.split_once('T')?;
    let mut ymd = d.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, dd) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let t = t.trim_end_matches('Z');
    let mut hms = t.split(':').map(|x| x.split('.').next().and_then(|v| v.parse::<i64>().ok()));
    let (h, mi, se) = (hms.next()??, hms.next().flatten().unwrap_or(0), hms.next().flatten().unwrap_or(0));
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from((days * 86_400 + h * 3600 + mi * 60 + se) * 1000).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosave_is_kept_apart_and_cleared_on_save() {
        let root = std::env::temp_dir().join(format!("xenify-recovery-test-{}", now_ms()));
        let file = root.join("report.md");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&file, "Saved text.").unwrap();
        // Saved a little while ago: an autosave in the same millisecond would count as stale.
        let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(10);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(earlier).unwrap();
        let mut a = Autosave { dir: Some(root.join("rec")), ..Default::default() };
        a.bind(Some(&file));
        let doc = Document::from_plain_text("Unsaved work.");
        a.write(&doc).unwrap();
        // The document's own file is untouched.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Saved text.");
        // This run's own autosave isn't offered back to it…
        assert!(a.left_for(&file).is_none());
        // …but another run (a fake pid) finds it.
        let info = root.join("rec").join(format!("{}.json", key(Some(&file), 0)));
        let mut meta: Info = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
        meta.pid = u32::MAX;
        std::fs::write(&info, serde_json::to_vec(&meta).unwrap()).unwrap();
        let mut b = Autosave { dir: Some(root.join("rec")), ..Default::default() };
        let (got, rec) = b.left_for(&file).expect("left over");
        assert_eq!((got.path.as_deref(), rec.plain_text().as_str()), (Some(file.as_path()), "Unsaved work."));
        // Saving clears it.
        a.clear();
        assert!(b.left_for(&file).is_none());
        // Never-saved documents: found as orphans once their run is gone.
        let mut c = Autosave { dir: Some(root.join("rec")), session: 99, ..Default::default() };
        c.bind(None);
        c.write(&doc).unwrap();
        let untitled = root.join("rec/untitled-99.json");
        let mut meta: Info = serde_json::from_slice(&std::fs::read(&untitled).unwrap()).unwrap();
        meta.pid = u32::MAX;
        std::fs::write(&untitled, serde_json::to_vec(&meta).unwrap()).unwrap();
        let found = b.orphans();
        assert_eq!(found.len(), 1);
        b.forget(None, Some(&found[0].2));
        assert!(b.orphans().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn iso_dates_and_zones() {
        assert_eq!(parse_iso("1970-01-02T00:00:00Z"), Some(86_400_000));
        assert_eq!(parse_iso("2026-10-10T14:03:00Z").map(|ms| ms / 1000 % 86_400), Some(14 * 3600 + 180));
        // A TZif file with one zone at +10:00.
        let mut z = b"TZif\0".to_vec();
        z.extend([0u8; 15]);
        for v in [0u32, 0, 0, 0, 1, 4] {
            z.extend(v.to_be_bytes());
        }
        z.extend(36_000i32.to_be_bytes());
        z.extend([0u8, 0]);
        z.extend(b"AEST");
        assert_eq!(tzif_offset(&z, 0), Some(36_000));
    }

    #[test]
    fn real_zone_files() {
        // 2026-01-15 (summer in Sydney, winter in London) and 2026-07-15.
        let (jan, jul) = (1_768_435_200i64, 1_784_073_600i64);
        for (zone, want) in [("Australia/Sydney", [39_600, 36_000]), ("Europe/London", [0, 3_600])] {
            let Ok(b) = std::fs::read(format!("/usr/share/zoneinfo/{zone}")) else { continue };
            assert_eq!([tzif_offset(&b, jan), tzif_offset(&b, jul)], want.map(Some), "{zone}");
        }
    }
}
