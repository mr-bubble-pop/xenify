//! Keeping the data folder from growing for ever:
//!
//! - things in a space's bin are deleted for good [`bin_days`] after they
//!   were deleted (Admin › Backups › Deleted files; 30 days unless changed,
//!   or never);
//! - stored bytes (`blobs/`) that no version, picture, Draw media or
//!   preview names any more are removed, a day after they were last
//!   written or used, so an upload whose row isn't written yet keeps its
//!   bytes;
//! - parts of Draw uploads nobody carried on with for a week are removed.
//!
//! Every hour, the first a few minutes after starting; with several
//! servers on one database a lock keeps it to one of them.

use crate::{Server, db};
use deadpool_postgres::GenericClient;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// The bin's default: 30 days.
pub const BIN_DAYS: i64 = 30;
/// How long unused bytes are kept after they were last written or used.
const GRACE: Duration = Duration::from_secs(24 * 3600);
/// How long a Draw upload that stopped is kept to carry on with.
const UPLOAD_DAYS: u64 = 7;
const LOCK: i64 = 7420433;

pub(crate) fn start(s: Server) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        tokio::time::sleep(Duration::from_secs(300)).await;
        loop {
            tick.tick().await;
            if *s.stopping().borrow() {
                break;
            }
            if let Err(e) = run(&s).await {
                eprintln!("Cleaning up: {e}");
            }
        }
    });
}

/// How many days things stay in a bin (None: until someone empties it).
pub async fn bin_days(c: &impl GenericClient) -> Option<i64> {
    match db::setting(c, "bin_days").await.ok().flatten() {
        None => Some(BIN_DAYS),
        Some(v) => v.parse().ok().filter(|d: &i64| *d > 0),
    }
}

/// What a run removed.
#[derive(Debug, Default, PartialEq)]
pub struct Removed {
    /// Things deleted for good from bins (each folder, file or page).
    pub from_bins: u64,
    /// Stored files no longer used, and their bytes.
    pub blobs: u64,
    pub bytes: u64,
    /// Uploads nobody carried on with.
    pub uploads: u64,
}

/// One run (the timer's, or a test's).
pub async fn run(s: &Server) -> Result<Removed, String> {
    let c = s.pool.get().await.map_err(|e| e.to_string())?;
    let r = c.query_one("SELECT pg_try_advisory_lock($1)", &[&LOCK]).await.map_err(|e| e.to_string())?;
    if !r.get::<_, bool>(0) {
        return Ok(Removed::default());
    }
    let out = run_locked(s, &c).await;
    let _ = c.execute("SELECT pg_advisory_unlock($1)", &[&LOCK]).await;
    out
}

async fn run_locked(s: &Server, c: &deadpool_postgres::Client) -> Result<Removed, String> {
    let mut out = Removed::default();
    if let Some(days) = bin_days(c).await {
        out.from_bins = c
            .execute(
                "DELETE FROM nodes WHERE trashed IS NOT NULL AND trashed < now() - make_interval(days => $1::int)",
                &[&(days as i32)],
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    // What's in use, read before looking at the folder: bytes written after
    // this are younger than GRACE anyway.
    let rows = c
        .query(
            "SELECT blob FROM versions UNION SELECT blob FROM node_pictures
             UNION SELECT hash FROM node_media UNION SELECT preview FROM nodes WHERE preview IS NOT NULL",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    let used: HashSet<String> = rows.iter().map(|r| r.get(0)).collect();
    let blobs = s.secure.join("blobs");
    let uploads = s.secure.join("uploads");
    let (n, bytes, up) = tokio::task::spawn_blocking(move || {
        let (n, bytes) = sweep_blobs(&blobs, &used, SystemTime::now());
        let up = sweep_uploads(&uploads, SystemTime::now());
        (n, bytes, up)
    })
    .await
    .map_err(|e| e.to_string())?;
    out.blobs = n;
    out.bytes = bytes;
    out.uploads = up;
    if out != Removed::default() {
        eprintln!(
            "Cleaned up: {} from bins, {} unused stored files ({} MB), {} stopped uploads",
            out.from_bins,
            out.blobs,
            out.bytes / 1_000_000,
            out.uploads
        );
    }
    Ok(out)
}

fn older_than(p: &Path, now: SystemTime, age: Duration) -> Option<u64> {
    let m = std::fs::metadata(p).ok()?;
    let at = m.modified().ok()?;
    (now.duration_since(at).unwrap_or_default() > age).then_some(m.len())
}

/// Removes files in `dir` (`ab/cdef…`, as files::Blobs keeps them) that
/// `used` doesn't name and that weren't written or used for [`GRACE`];
/// also half-written ones (`….tmp…`) as old. Returns (files, bytes).
fn sweep_blobs(dir: &Path, used: &HashSet<String>, now: SystemTime) -> (u64, u64) {
    let (mut n, mut bytes) = (0, 0);
    let Ok(tops) = std::fs::read_dir(dir) else { return (0, 0) };
    for top in tops.flatten() {
        let prefix = top.file_name().to_string_lossy().into_owned();
        if prefix.len() != 2 || !top.path().is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(top.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let path = f.path();
            let hash = format!("{prefix}{name}");
            let half_written = name.contains(".tmp");
            if !half_written && (used.contains(&hash) || !hash.bytes().all(|b| b.is_ascii_hexdigit())) {
                continue;
            }
            if let Some(len) = older_than(&path, now, GRACE)
                && std::fs::remove_file(&path).is_ok()
            {
                n += 1;
                bytes += len;
            }
        }
        // An empty `ab/` goes too (put() makes it again).
        let _ = std::fs::remove_dir(top.path());
    }
    (n, bytes)
}

/// Removes Draw upload parts nobody added to for [`UPLOAD_DAYS`].
fn sweep_uploads(dir: &Path, now: SystemTime) -> u64 {
    let mut n = 0;
    let Ok(files) = std::fs::read_dir(dir) else { return 0 };
    for f in files.flatten() {
        if older_than(&f.path(), now, Duration::from_secs(UPLOAD_DAYS * 24 * 3600)).is_some()
            && std::fs::remove_file(f.path()).is_ok()
        {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeps_only_old_unused_blobs() {
        let dir = std::env::temp_dir().join(format!("xenify-sweep-{}", crate::random(6)));
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(blobs.join("ab")).unwrap();
        std::fs::create_dir_all(blobs.join("cd")).unwrap();
        std::fs::write(blobs.join("ab/1111"), b"used").unwrap();
        std::fs::write(blobs.join("ab/2222"), b"unused!").unwrap();
        std::fs::write(blobs.join("cd/3333.tmpxyz"), b"half").unwrap();
        let used: HashSet<String> = ["ab1111".to_string()].into();
        // Just written: nothing goes.
        assert_eq!(sweep_blobs(&blobs, &used, SystemTime::now()), (0, 0));
        // Two days on: the unused one and the half-written one go.
        let later = SystemTime::now() + Duration::from_secs(48 * 3600);
        assert_eq!(sweep_blobs(&blobs, &used, later), (2, 11));
        assert!(blobs.join("ab/1111").exists());
        assert!(!blobs.join("cd").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sweeps_stopped_uploads() {
        let dir = std::env::temp_dir().join(format!("xenify-uploads-{}", crate::random(6)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("5-abc"), b"part").unwrap();
        assert_eq!(sweep_uploads(&dir, SystemTime::now()), 0);
        assert_eq!(sweep_uploads(&dir, SystemTime::now() + Duration::from_secs(8 * 24 * 3600)), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
