//! Downloads on demand (animated emoji and the like): each file is fetched
//! once over HTTPS, kept in the data folder, and logged (when, where from,
//! where to, size) in `downloads.log` beside it.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

/// The program's data folder (`$XENIFY_DATA_DIR`, else the platform's).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("XENIFY_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").filter(|_| cfg!(windows)).map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("xenify"))
}

/// Where `rel` (e.g. `emoji/1f389.json`) is kept in the download cache.
pub fn cache_path(rel: &str) -> Option<PathBuf> {
    Some(data_dir()?.join("downloads").join(rel))
}

/// `rel` from the cache, if it was downloaded before.
pub fn cached(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(cache_path(rel)?).ok()
}

/// `rel` from the cache, else downloaded from `url` (at most `max` bytes),
/// kept and logged. Blocks: call it off the UI thread.
pub fn fetch(url: &str, rel: &str, max: u64) -> Result<Vec<u8>, String> {
    if let Some(bytes) = cached(rel) {
        return Ok(bytes);
    }
    let bytes = get(url, max)?;
    if let Some(path) = cache_path(rel) {
        let saved = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && {
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_ok()
        };
        if saved {
            log(url, rel, bytes.len());
        }
    }
    Ok(bytes)
}

/// Downloads `url` (at most `max` bytes) without keeping it. Blocks. Error
/// messages leave out the URL's query (it may hold an API key).
pub fn get(url: &str, max: u64) -> Result<Vec<u8>, String> {
    let shown = url.split('?').next().unwrap_or(url);
    if !url.starts_with("https://") {
        return Err(format!("{shown}: only HTTPS downloads are allowed"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into();
    let strip = |e: ureq::Error| e.to_string().replace(url, shown);
    let mut resp = agent.get(url).call().map_err(|e| format!("couldn't download {shown}: {}", strip(e)))?;
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("couldn't download {shown}: {e}"))?;
    if bytes.len() as u64 > max {
        return Err(format!("{shown}: bigger than {} KB", max / 1024));
    }
    Ok(bytes)
}

fn log(url: &str, rel: &str, size: usize) {
    use std::io::Write;
    let Some(path) = data_dir().map(|d| d.join("downloads").join("downloads.log")) else { return };
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}\t{url}\t{rel}\t{size} bytes", civil(secs));
    }
}

/// `secs` since 1970 as `YYYY-MM-DD HH:MM:SS` UTC.
fn civil(secs: u64) -> String {
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rest / 3600, rest / 60 % 60, rest % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::civil(0), "1970-01-01 00:00:00");
        assert_eq!(super::civil(1_790_000_000), "2026-09-21 14:13:20");
    }

    #[test]
    fn only_https() {
        assert!(super::fetch("http://example.com/x", "test/never-cached-x", 10).is_err());
    }
}
