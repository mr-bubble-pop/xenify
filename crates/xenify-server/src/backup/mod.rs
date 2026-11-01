//! Backups (Admin › Backups): every day at a set hour, the database (a
//! `pg_dump`) and the data folder (and the encrypted folder, with
//! encryption at rest) go to a folder, S3-compatible storage or an SFTP
//! server, as one snapshot in restic's repository format (rustic_core).
//!
//! Each snapshot is whole (restore any one by itself) but only what changed
//! is stored: files are cut into pieces named by their hash, and pieces
//! already there aren't sent again. Old snapshots go by a rule (7 daily, 4
//! weekly, 6 monthly unless changed); about once a week, pieces no snapshot
//! uses any more are removed (prune).
//!
//! Encryption (on unless turned off) uses a passphrase: lose it and the
//! backups can't be read. Off, the repository has an empty passphrase
//! (restic's `--insecure-no-password`).
//!
//! `xenify-server backup …` lists and restores, also when the server or
//! its database is gone.

pub mod store;
/// For tests (a place that fails partway).
pub use rustic_core;

use crate::{Server, db, err, failed, unavailable};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rustic_core::{
    BackupOptions, ConfigOptions, Credentials, ForgetGroups, Grouped, KeepOptions, KeyOptions, LocalSourceFilterOptions,
    PathList, PruneOptions, Repository, RepositoryBackends, RepositoryOptions, SnapshotGroupCriterion, SnapshotOptions,
    WriteBackend,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use store::SshAuth;

const LOCK: i64 = 7420434;

/// What admins set (settings key `backup`); the secrets never go back to
/// the page.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub on: bool,
    /// "folder", "s3" or "sftp".
    pub kind: String,
    pub folder: String,
    pub s3_endpoint: String,
    /// The bucket, optionally followed by a folder in it ("b/xenify").
    pub s3_bucket: String,
    pub s3_key: String,
    pub s3_secret: String,
    /// user@host[:port]
    pub sftp_address: String,
    pub sftp_folder: String,
    /// "key" (this server's) or "password".
    pub sftp_auth: String,
    pub sftp_password: String,
    /// The SFTP server's key (SHA256 fingerprint), trusted the first time.
    pub sftp_host_key: String,
    pub encrypt: bool,
    pub passphrase: String,
    /// The hour (server's time) the daily backup starts.
    pub hour: u32,
    pub keep_daily: i32,
    pub keep_weekly: i32,
    pub keep_monthly: i32,
    /// Upload speed limit in Mbit/s (0: none).
    pub limit_mbit: f64,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            on: false,
            kind: "folder".into(),
            folder: String::new(),
            s3_endpoint: String::new(),
            s3_bucket: String::new(),
            s3_key: String::new(),
            s3_secret: String::new(),
            sftp_address: String::new(),
            sftp_folder: "xenify-backups".into(),
            sftp_auth: "key".into(),
            sftp_password: String::new(),
            sftp_host_key: String::new(),
            encrypt: true,
            passphrase: String::new(),
            hour: 2,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            limit_mbit: 0.0,
        }
    }
}

impl Settings {
    /// Where, in one line (also what `backup restore --from` takes).
    pub fn place(&self) -> String {
        match self.kind.as_str() {
            "s3" => format!("s3:{}/{}", self.s3_endpoint.trim().trim_end_matches('/'), self.s3_bucket.trim().trim_matches('/')),
            "sftp" => format!("sftp:{}/{}", self.sftp_address.trim(), self.sftp_folder.trim().trim_start_matches('/')),
            _ => self.folder.trim().to_string(),
        }
    }

    fn password(&self) -> String {
        if self.encrypt { self.passphrase.clone() } else { String::new() }
    }

    /// The same place (so the same repository, passphrase and all)?
    fn same_place(&self, o: &Settings) -> bool {
        self.kind == o.kind && self.place() == o.place()
    }
}

/// How the last runs went (settings key `backup_status`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// RFC 3339 times.
    pub last_try: String,
    pub last_ok: String,
    pub last_prune: String,
    /// Why the last one failed ("" when it worked).
    pub error: String,
    /// The last good one: bytes looked at, bytes added (after compression),
    /// seconds taken.
    pub size: u64,
    pub added: u64,
    pub took: u64,
    /// The snapshots there (newest first): id, time, size.
    pub snapshots: Vec<(String, String, u64)>,
    /// The place they're at (a new place starts a new list).
    pub place: String,
}

/// A backup running now.
#[derive(Default)]
pub struct Running {
    pub now: Mutex<Option<String>>,
}

/// What a backup needs from the server's settings file.
#[derive(Clone, Debug, Default)]
pub struct Paths {
    pub database: String,
    pub data: PathBuf,
    pub secure: PathBuf,
    /// The server's own PostgreSQL's program (bin/postgres), when it has one:
    /// its pg_dump and pg_restore are beside it.
    pub postgres: Option<PathBuf>,
}

/// Where the backup's own files are made (the database dump): in the
/// encrypted folder with encryption at rest.
fn work_dir(p: &Paths) -> PathBuf {
    p.secure.join("backup-work")
}

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/admin/backups", get(page).put(save))
        .route("/v1/admin/backups/check", post(check))
        .route("/v1/admin/backups/run", post(run_now))
}

pub(crate) async fn settings(c: &impl deadpool_postgres::GenericClient) -> Settings {
    db::setting(c, "backup").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

pub(crate) async fn status(c: &impl deadpool_postgres::GenericClient) -> Status {
    db::setting(c, "backup_status").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// This server's key for SFTP: (private, public line), made the first time.
async fn ssh_key(c: &impl deadpool_postgres::GenericClient) -> Result<(String, String), String> {
    if let Some(v) = db::setting(c, "backup_ssh_key").await.map_err(|e| e.to_string())?
        && let Ok(k) = russh::keys::decode_secret_key(&v, None)
    {
        let public = k.public_key().to_openssh().map_err(|e| e.to_string())?;
        return Ok((v, public));
    }
    let name = db::setting(c, "name").await.ok().flatten().unwrap_or_else(|| "Xenify server".into());
    let (private, public) = store::new_ssh_key(&format!("{name} backups").replace(' ', "-"))?;
    db::set_setting(c, "backup_ssh_key", &private).await.map_err(|e| e.to_string())?;
    Ok((private, public))
}

/// Opens the place: (the backend, the SFTP server's key when it's SFTP).
pub fn open_place(s: &Settings, ssh_private: &str) -> Result<(Arc<dyn WriteBackend>, Option<String>), String> {
    let be: Arc<dyn WriteBackend> = match s.kind.as_str() {
        "folder" => {
            let f = s.folder.trim();
            if f.is_empty() {
                return Err("pick a folder for the backups".into());
            }
            if !Path::new(f).is_absolute() {
                return Err("the folder must be a full path, like /mnt/backup/xenify".into());
            }
            Arc::new(store::Folder::new(f.into()))
        }
        "s3" => {
            if s.s3_key.trim().is_empty() || s.s3_secret.trim().is_empty() {
                return Err("the storage's access key and secret key are needed".into());
            }
            Arc::new(store::S3::new(&s.s3_endpoint, &s.s3_bucket, &s.s3_key, &s.s3_secret, None)?)
        }
        "sftp" => {
            let auth = if s.sftp_auth == "password" {
                SshAuth::Password(s.sftp_password.clone())
            } else {
                SshAuth::Key(ssh_private.to_string())
            };
            let known = Some(s.sftp_host_key.as_str()).filter(|k| !k.is_empty());
            let (sftp, fp) = store::Sftp::connect(&s.sftp_address, &s.sftp_folder, &auth, known)?;
            return Ok((throttle(Arc::new(sftp), s), Some(fp)));
        }
        _ => return Err("pick where backups go".into()),
    };
    Ok((throttle(be, s), None))
}

fn throttle(be: Arc<dyn WriteBackend>, s: &Settings) -> Arc<dyn WriteBackend> {
    if s.limit_mbit > 0.0 {
        Arc::new(store::Throttled::new(be, (s.limit_mbit * 125_000.0) as u64))
    } else {
        be
    }
}

type Repo = Repository<rustic_core::OpenStatus>;

/// Opens the repository at `be`, making it the first time.
fn repository(be: Arc<dyn WriteBackend>, s: &Settings, make: bool) -> Result<Repo, String> {
    let opts = RepositoryOptions::default().no_cache(true);
    let repo = Repository::new(&opts, &RepositoryBackends::new(be, None)).map_err(|e| e.to_string())?;
    let creds = Credentials::password(s.password());
    let exists = repo.config_id().map_err(|e| e.to_string())?.is_some();
    if exists {
        return repo.open(&creds).map_err(|e| {
            let e = e.to_string();
            if e.to_lowercase().contains("password") || e.to_lowercase().contains("key") {
                if s.encrypt {
                    "there are backups there already, and this passphrase doesn't open them".into()
                } else {
                    "there are encrypted backups there already: turn encryption on and give their passphrase, or pick another place".into()
                }
            } else {
                e
            }
        });
    }
    if !make {
        return Err("there are no backups there".into());
    }
    if s.encrypt && s.passphrase.chars().count() < 8 {
        return Err("the passphrase needs at least 8 characters".into());
    }
    repo.init(&creds, &KeyOptions::default(), &ConfigOptions::default()).map_err(|e| e.to_string())
}

/// pg_dump or pg_restore: beside the server's own PostgreSQL, else the
/// newest under /usr/lib/postgresql, else the one on PATH.
pub fn pg_tool(name: &str, own: Option<&Path>) -> PathBuf {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    if let Some(p) = own.and_then(|p| p.parent()).map(|d| d.join(&exe)).filter(|p| p.exists()) {
        return p;
    }
    let mut versions: Vec<(u32, PathBuf)> = std::fs::read_dir("/usr/lib/postgresql")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((e.file_name().to_str()?.parse().ok()?, e.path().join("bin").join(&exe))))
        .filter(|(_, p): &(u32, PathBuf)| p.exists())
        .collect();
    versions.sort();
    versions.pop().map(|v| v.1).unwrap_or_else(|| exe.into())
}

/// What one backup did.
pub struct Done {
    pub id: String,
    pub size: u64,
    pub added: u64,
    pub snapshots: Vec<(String, String, u64)>,
    pub pruned: bool,
    /// The SFTP server's key seen (to keep, the first time).
    pub host_key: Option<String>,
}

/// `database` (a PostgreSQL address) without its password, and the
/// password: pg_dump and pg_restore get the password in their environment
/// (PGPASSWORD), where other users of the computer can't see it, not on
/// their command line, where they can.
pub fn without_password(database: &str) -> (String, Option<String>) {
    let d = database.trim();
    if d.starts_with("postgres://") || d.starts_with("postgresql://") {
        let Ok(mut u) = url::Url::parse(d) else { return (d.to_string(), None) };
        let mut password = u.password().map(|p| {
            url::form_urlencoded::parse(format!("p={p}").as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
        });
        let _ = u.set_password(None);
        // ?password=… too.
        let query: Vec<(String, String)> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        if query.iter().any(|(k, _)| k == "password") {
            let kept: Vec<&(String, String)> = query.iter().filter(|(k, _)| k != "password").collect();
            if password.is_none() {
                password = query.iter().find(|(k, _)| k == "password").map(|(_, v)| v.clone());
            }
            if kept.is_empty() {
                u.set_query(None);
            } else {
                u.query_pairs_mut().clear().extend_pairs(kept.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            }
        }
        return (u.to_string(), password.filter(|p| !p.is_empty()));
    }
    // key=value form: password=… (maybe quoted) taken out.
    let mut out = Vec::new();
    let mut password = None;
    let mut rest = d;
    while !rest.is_empty() {
        rest = rest.trim_start();
        let Some(eq) = rest.find('=') else {
            out.push(rest.to_string());
            break;
        };
        let key = rest[..eq].trim().to_string();
        let mut after = rest[eq + 1..].trim_start();
        let value;
        if let Some(q) = after.strip_prefix('\'') {
            let mut v = String::new();
            let mut chars = q.char_indices();
            let mut end = q.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            v.push(n);
                        }
                    }
                    '\'' => {
                        end = i + 1;
                        break;
                    }
                    c => v.push(c),
                }
            }
            value = (v, format!("'{}", &q[..end]));
            after = &q[end..];
        } else {
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            value = (after[..end].to_string(), after[..end].to_string());
            after = &after[end..];
        }
        if key == "password" {
            password = Some(value.0);
        } else {
            out.push(format!("{key}={}", value.1));
        }
        rest = after;
    }
    (out.join(" "), password.filter(|p| !p.is_empty()))
}

/// pg_dump or pg_restore for `database`, its password in the environment.
fn pg_command(tool: &Path, database: &str) -> std::process::Command {
    let (dbname, password) = without_password(database);
    let mut cmd = xenify_hub_core::command(tool);
    cmd.arg(format!("--dbname={dbname}"));
    if let Some(p) = password {
        cmd.env("PGPASSWORD", p);
    }
    cmd
}

/// One backup, start to end (blocking; run it on a thread of its own, not
/// the async runtime's: SFTP runs a runtime of its own). `progress` says
/// what it's doing.
pub fn back_up(s: &Settings, ssh_private: &str, p: &Paths, prune: bool, repair: bool, progress: &dyn Fn(&str)) -> Result<Done, String> {
    progress("Connecting");
    let (be, host_key) = open_place(s, ssh_private)?;
    back_up_to(be, s, p, prune, repair, progress).map(|d| Done { host_key, ..d })
}

/// The same, to a place already open. `repair` after a backup that
/// stopped partway: what it sent since it last wrote its index (every 5
/// minutes) is listed, so it isn't sent again.
pub fn back_up_to(
    be: Arc<dyn WriteBackend>,
    s: &Settings,
    p: &Paths,
    prune: bool,
    repair: bool,
    progress: &dyn Fn(&str),
) -> Result<Done, String> {
    progress("Saving the database");
    let work = work_dir(p);
    std::fs::create_dir_all(&work).map_err(|e| format!("can't make {}: {e}", work.display()))?;
    let dump = work.join("database.pgdump");
    // Plain (not compressed) so unchanged tables come out the same and
    // aren't stored again; the backup compresses.
    let tool = pg_tool("pg_dump", p.postgres.as_deref());
    let out = pg_command(&tool, &p.database)
        .args(["--format=custom", "--compress=0", "--no-owner", "--file"])
        .arg(&dump)
        .output()
        .map_err(|e| format!("can't run {}: {e} (install PostgreSQL's client programs)", tool.display()))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&dump);
        return Err(format!("saving the database failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let repo = repository(be, s, true)?;
    if repair {
        progress("Carrying on from last time");
        repo.repair_index(&rustic_core::RepairIndexOptions::default(), false).map_err(|e| e.to_string())?;
    }
    progress("Backing up files");
    let mut sources = vec![p.data.clone()];
    if p.secure != p.data && !p.secure.starts_with(&p.data) {
        sources.push(p.secure.clone());
    }
    let list = PathList::from_iter(sources.iter().map(|d| d.as_path())).sanitize().map_err(|e| e.to_string())?;
    let mut opts = BackupOptions::default();
    // The browser apps (downloaded again when needed) and unfinished Draw
    // uploads stay out; so does any PostgreSQL data folder (the dump has it).
    for skip in [p.data.join("web"), p.secure.join("uploads"), p.data.join("uploads")] {
        opts.excludes.globs.push(format!("!{}", skip.display()));
    }
    opts.ignore_filter_opts = LocalSourceFilterOptions::default().exclude_if_present(vec!["PG_VERSION".to_string()]);
    let about = json!({
        "data": p.data, "secure": p.secure, "dump": dump, "version": crate::VERSION, "schema": crate::SCHEMA_LEVEL,
    });
    let snap = SnapshotOptions::default()
        .tags(vec!["xenify".parse::<rustic_core::StringList>().map_err(|e| e.to_string())?])
        .description(about.to_string())
        .to_snapshot()
        .map_err(|e| e.to_string())?;
    let repo = repo.to_indexed_ids().map_err(|e| e.to_string())?;
    let snap = repo.backup(&opts, &list, snap).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&dump);
    let snap = snap?;
    let (size, added) = snap.summary.as_ref().map_or((0, 0), |s| (s.total_bytes_processed, s.data_added_packed));
    progress("Removing old backups");
    let keep = KeepOptions::default()
        .keep_daily(s.keep_daily.max(1))
        .keep_weekly(s.keep_weekly.max(0))
        .keep_monthly(s.keep_monthly.max(0))
        .keep_last(1);
    let all = repo.get_all_snapshots().map_err(|e| e.to_string())?;
    let grouped = Grouped::from_items(all, SnapshotGroupCriterion::new());
    let now = rustic_core::jiff::Zoned::now();
    let forget = ForgetGroups::from_grouped_snapshots_with_retention(grouped, &keep, &now).map_err(|e| e.to_string())?;
    let ids = forget.into_forget_ids();
    if !ids.is_empty() {
        repo.delete_snapshots(&ids).map_err(|e| e.to_string())?;
    }
    let mut pruned = false;
    if prune {
        progress("Freeing space");
        let opts = PruneOptions::default();
        let plan = repo.prune_plan(&opts).map_err(|e| e.to_string())?;
        repo.prune(&opts, plan).map_err(|e| e.to_string())?;
        pruned = true;
    }
    let snapshots = list_snapshots(&repo)?;
    Ok(Done { id: snap.id.to_string(), size, added, snapshots, pruned, host_key: None })
}

fn list_snapshots<S: rustic_core::Open>(repo: &Repository<S>) -> Result<Vec<(String, String, u64)>, String> {
    let mut all = repo.get_all_snapshots().map_err(|e| e.to_string())?;
    all.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(all
        .iter()
        .map(|s| {
            (
                s.id.to_string(),
                s.time.timestamp().to_string(),
                s.summary.as_ref().map_or(0, |x| x.total_bytes_processed),
            )
        })
        .collect())
}

/// The snapshots at a place (for the CLI's list).
pub fn snapshots(s: &Settings, ssh_private: &str) -> Result<Vec<(String, String, u64)>, String> {
    let (be, _) = open_place(s, ssh_private)?;
    let repo = repository(be, s, false)?;
    list_snapshots(&repo)
}

/// Restores snapshot `id` ("latest" for the newest): its files into
/// `p.data` / `p.secure`, then its database into `p.database` (which the
/// server mustn't be using).
/// Refuses a snapshot a newer server made: its database may hold what this
/// version can't use (snapshots from before "schema" was kept are judged by
/// the server version).
fn newer_check(about: &Value) -> Result<(), String> {
    let newer = match about["schema"].as_i64() {
        Some(n) => n > crate::SCHEMA_LEVEL,
        None => match (about["version"].as_str().map(semver::Version::parse), semver::Version::parse(crate::VERSION)) {
            (Some(Ok(made)), Ok(me)) => made > me,
            _ => false,
        },
    };
    if newer {
        return Err(format!(
            "that backup was made by a newer Xenify server ({}): update this server first, then restore it",
            about["version"].as_str().unwrap_or("a later version")
        ));
    }
    Ok(())
}

pub fn restore(s: &Settings, ssh_private: &str, id: &str, p: &Paths, say: &dyn Fn(&str)) -> Result<(), String> {
    use rustic_core::{LocalDestination, LsOptions, RestoreOptions};
    let (be, _) = open_place(s, ssh_private)?;
    let repo = repository(be, s, false)?.to_indexed().map_err(|e| e.to_string())?;
    let snap = repo.get_snapshot_from_str(id, |_| true).map_err(|e| e.to_string())?;
    let about: Value = snap.description.as_deref().and_then(|d| serde_json::from_str(d).ok()).unwrap_or_default();
    let from = |k: &str| about[k].as_str().map(PathBuf::from);
    let (Some(data), Some(dump)) = (from("data"), from("dump")) else {
        return Err("that snapshot wasn't made by a Xenify server".into());
    };
    newer_check(&about)?;
    let secure = from("secure").unwrap_or_else(|| data.clone());
    let mut parts = vec![(data.clone(), p.data.clone())];
    if secure != data && !secure.starts_with(&data) {
        parts.push((secure.clone(), p.secure.clone()));
    }
    let opts = RestoreOptions::default();
    for (inside, to) in &parts {
        say(&format!("Restoring {} into {}", inside.display(), to.display()));
        let node = repo.node_from_snapshot_and_path(&snap, &inside.to_string_lossy()).map_err(|e| e.to_string())?;
        let dest = LocalDestination::new(&to.to_string_lossy(), true, !node.is_dir()).map_err(|e| e.to_string())?;
        let ls = repo.ls(&node, &LsOptions::default()).map_err(|e| e.to_string())?;
        let plan = repo.prepare_restore(&opts, ls.clone(), &dest, false).map_err(|e| e.to_string())?;
        repo.restore(plan, &opts, ls, &dest).map_err(|e| e.to_string())?;
    }
    // Where the dump landed: its place inside the folder it was in.
    let (inside, to) = parts.iter().find(|(i, _)| dump.starts_with(i)).ok_or("the snapshot has no database")?;
    let restored = to.join(dump.strip_prefix(inside).unwrap());
    say("Restoring the database");
    let tool = pg_tool("pg_restore", p.postgres.as_deref());
    let out = pg_command(&tool, &p.database)
        .args(["--clean", "--if-exists", "--no-owner", "--no-privileges", "--single-transaction"])
        .arg(&restored)
        .output()
        .map_err(|e| format!("can't run {}: {e}", tool.display()))?;
    let _ = std::fs::remove_file(&restored);
    if !out.status.success() {
        return Err(format!("restoring the database failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// `--from`: a folder, s3:https://host/bucket[/folder] or
/// sftp:user@host[:port]/folder.
pub fn place_from(from: &str) -> Result<Settings, String> {
    let mut s = Settings::default();
    if let Some(rest) = from.strip_prefix("s3:") {
        let url: url::Url = rest.parse().map_err(|_| "s3: needs a web address: s3:https://host/bucket")?;
        let path = url.path().trim_matches('/').to_string();
        if path.is_empty() {
            return Err("s3: needs the bucket after the address: s3:https://host/bucket".into());
        }
        s.kind = "s3".into();
        s.s3_endpoint = format!("{}://{}", url.scheme(), &url[url::Position::BeforeHost..url::Position::AfterPort]);
        s.s3_bucket = path;
    } else if let Some(rest) = from.strip_prefix("sftp:") {
        let (addr, folder) = match rest.split_once('/') {
            Some((a, f)) => (a, f),
            None => (rest, ""),
        };
        s.kind = "sftp".into();
        s.sftp_address = addr.into();
        s.sftp_folder = folder.into();
    } else {
        s.kind = "folder".into();
        s.folder = from.into();
    }
    Ok(s)
}

/// The settings saved in `database` and this server's SFTP key, when the
/// database is there and has them.
pub async fn saved(database: &str) -> Option<(Settings, String)> {
    let (c, conn) = tokio_postgres::connect(database, tokio_postgres::NoTls).await.ok()?;
    tokio::spawn(conn);
    let get = |k: &'static str| {
        let c = &c;
        async move { c.query_opt("SELECT value FROM settings WHERE key = $1", &[&k]).await.ok().flatten().map(|r| r.get::<_, String>(0)) }
    };
    let set: Settings = serde_json::from_str(&get("backup").await?).ok()?;
    Some((set, get("backup_ssh_key").await.unwrap_or_default()))
}

/// Whether something else (a running server) is connected to `database`.
pub async fn database_in_use(database: &str) -> bool {
    let Ok((c, conn)) = tokio_postgres::connect(database, tokio_postgres::NoTls).await else { return false };
    tokio::spawn(conn);
    c.query_one(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
        &[],
    )
    .await
    .is_ok_and(|r| r.get::<_, i64>(0) > 0)
}

// ---- On the server ------------------------------------------------------------

fn now_text() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn days_since(t: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(t).ok().map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days())
}

/// Every 5 minutes: is today's backup due?
pub(crate) fn start(s: Server) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            tick.tick().await;
            if *s.stopping().borrow() {
                break;
            }
            let Ok(c) = s.pool.get().await else { continue };
            let set = settings(&c).await;
            if !set.on {
                continue;
            }
            let st = status(&c).await;
            use chrono::Timelike;
            let now = chrono::Local::now();
            let today = now.date_naive();
            let tried_today = chrono::DateTime::parse_from_rfc3339(&st.last_try)
                .is_ok_and(|t| t.with_timezone(&chrono::Local).date_naive() == today);
            // Due at its hour, or later the same day if the server was off then.
            if now.hour() >= set.hour && !tried_today {
                let _ = run(&s).await;
            }
        }
    });
}

/// A backup now (unless one's running here or on another server on the
/// same database). Returns when it's done.
pub async fn run(s: &Server) -> Result<(), String> {
    let c = s.pool.get().await.map_err(|e| e.to_string())?;
    let r = c.query_one("SELECT pg_try_advisory_lock($1)", &[&LOCK]).await.map_err(|e| e.to_string())?;
    if !r.get::<_, bool>(0) {
        return Err("a backup is running already".into());
    }
    let out = run_locked(s, &c).await;
    let _ = c.execute("SELECT pg_advisory_unlock($1)", &[&LOCK]).await;
    out
}

async fn run_locked(s: &Server, c: &deadpool_postgres::Client) -> Result<(), String> {
    let set = settings(c).await;
    let mut st = status(c).await;
    if st.place != set.place() {
        st = Status { place: set.place(), ..Status::default() };
    }
    st.last_try = now_text();
    let _ = db::set_setting(c, "backup_status", &serde_json::to_string(&st).unwrap()).await;
    let (private, _) = ssh_key(c).await?;
    let paths = s.backup_paths();
    let repair = !st.error.is_empty();
    let prune = st.last_prune.is_empty() || days_since(&st.last_prune).is_none_or(|d| d >= 7);
    let running = s.backup_running();
    *running.now.lock().unwrap() = Some("Starting".into());
    let started = std::time::Instant::now();
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let (set, running) = (set.clone(), running.clone());
        std::thread::spawn(move || {
            let say = |m: &str| *running.now.lock().unwrap() = Some(m.to_string());
            let _ = tx.send(back_up(&set, &private, &paths, prune, repair, &say));
        });
    }
    let out = rx.await.map_err(|e| e.to_string()).and_then(|r| r);
    *running.now.lock().unwrap() = None;
    match &out {
        Ok(d) => {
            st.last_ok = now_text();
            st.error.clear();
            st.size = d.size;
            st.added = d.added;
            st.took = started.elapsed().as_secs();
            st.snapshots = d.snapshots.clone();
            if d.pruned {
                st.last_prune = now_text();
            }
            // The SFTP server's key, the first time it's seen: from now on
            // a different one stops backups.
            if let Some(fp) = d.host_key.as_deref().filter(|k| !k.is_empty()) {
                let mut saved = settings(c).await;
                if saved.sftp_host_key.is_empty() && saved.same_place(&set) {
                    saved.sftp_host_key = fp.to_string();
                    let _ = db::set_setting(c, "backup", &serde_json::to_string(&saved).unwrap()).await;
                }
            }
        }
        Err(e) => {
            eprintln!("Backup failed: {e}");
            st.error = e.clone();
        }
    }
    let _ = db::set_setting(c, "backup_status", &serde_json::to_string(&st).unwrap()).await;
    if let Err(e) = &out {
        tell_admins(c, e).await;
    }
    out.map(|_| ())
}

/// A backup failed: every admin with an email address hears about it.
async fn tell_admins(c: &deadpool_postgres::Client, why: &str) {
    let server = db::setting(c, "name").await.ok().flatten().unwrap_or_else(|| "Xenify server".into());
    let address = crate::server_address(c).await;
    let Ok(rows) = c
        .query("SELECT email FROM users WHERE admin AND NOT disabled AND email <> ''", &[])
        .await
    else {
        return;
    };
    for r in rows {
        let to: String = r.get(0);
        let e = crate::mail::Email {
            subject: format!("The backup of {server} failed"),
            heading: "Tonight's backup didn't work".into(),
            lines: vec![format!("{server} couldn't back up: {why}."), "It tries again tomorrow; Admin › Backups has a Back up now button.".into()],
            button: (!address.is_empty())
                .then(|| ("Open Backups".to_string(), format!("{}/admin/#backups", address.trim_end_matches('/')))),
            small: vec!["Sent to the server's admins.".into()],
        };
        let _ = crate::mail::send(c, &to, e).await;
    }
}

// ---- Admin › Backups ----------------------------------------------------------

macro_rules! may {
    ($s:expr, $h:expr) => {
        match crate::admin_power(&$s, &$h, "admin.backups").await {
            Ok(a) => a,
            Err(e) => return e,
        }
    };
}

/// What only admins may change (people who may only manage backups run
/// them and set when): where backups go, and turning encryption off.
fn admins_only(new: &Settings, old: &Settings) -> Option<&'static str> {
    if !new.same_place(old) {
        return Some("only admins can change where backups go");
    }
    if old.encrypt && !new.encrypt {
        return Some("only admins can turn encryption off");
    }
    None
}

fn public(set: &Settings) -> Value {
    let mut v = serde_json::to_value(set).unwrap();
    for k in ["s3_secret", "sftp_password", "passphrase"] {
        let had = v[k].as_str().is_some_and(|x| !x.is_empty());
        v[k] = json!("");
        v[format!("{k}_set")] = json!(had);
    }
    v
}

async fn page(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = may!(s, h);
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let set = settings(&c).await;
    let st = status(&c).await;
    let public_key = ssh_key(&c).await.map(|k| k.1).unwrap_or_default();
    let p = s.backup_paths();
    Json(json!({
        "settings": public(&set),
        "status": if st.place == set.place() { serde_json::to_value(&st).unwrap() } else { json!({}) },
        "running": *s.backup_running().now.lock().unwrap(),
        "public_key": public_key,
        "bin_days": crate::cleanup::bin_days(&c).await.unwrap_or(0),
        "place": set.place(),
        "pg_dump": pg_tool("pg_dump", p.postgres.as_deref()).display().to_string(),
        "data": p.data, "secure": p.secure,
        // Whether this person may change where backups go and encryption.
        "can_change_place": me.admin,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct Save {
    settings: Option<Settings>,
    bin_days: Option<i64>,
}

/// Keeps secrets the page left empty (it never sees them), but only for
/// the place they were given for: a new address, storage or bucket needs
/// them typed again, so they never go somewhere else.
fn merged(mut new: Settings, old: &Settings) -> Settings {
    let same_s3 = new.kind == old.kind
        && new.s3_endpoint.trim().trim_end_matches('/') == old.s3_endpoint.trim().trim_end_matches('/')
        && new.s3_bucket.trim().trim_matches('/') == old.s3_bucket.trim().trim_matches('/');
    if new.s3_secret.is_empty() && same_s3 {
        new.s3_secret = old.s3_secret.clone();
    }
    let same_sftp = new.kind == old.kind && new.sftp_address.trim() == old.sftp_address.trim();
    if new.sftp_password.is_empty() && same_sftp {
        new.sftp_password = old.sftp_password.clone();
    }
    if new.passphrase.is_empty() {
        new.passphrase = old.passphrase.clone();
    }
    // The SFTP server's key is only trusted by Check.
    if !new.same_place(old) || new.sftp_address != old.sftp_address {
        new.sftp_host_key.clear();
    } else {
        new.sftp_host_key = old.sftp_host_key.clone();
    }
    new.hour = new.hour.min(23);
    new
}

async fn save(State(s): State<Server>, h: HeaderMap, Json(v): Json<Save>) -> Response {
    let me = may!(s, h);
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if let Some(d) = v.bin_days {
        // 0: until someone empties it.
        if !(0..=3650).contains(&d) || db::set_setting(&c, "bin_days", &d.to_string()).await.is_err() {
            return failed();
        }
    }
    if let Some(new) = v.settings {
        let old = settings(&c).await;
        let new = merged(new, &old);
        if !me.admin
            && let Some(why) = admins_only(&new, &old)
        {
            return err(StatusCode::FORBIDDEN, "not_admin", why);
        }
        if new.on && new.encrypt && new.passphrase.chars().count() < 8 {
            return err(StatusCode::BAD_REQUEST, "passphrase", "the passphrase needs at least 8 characters");
        }
        if db::set_setting(&c, "backup", &serde_json::to_string(&new).unwrap()).await.is_err() {
            return failed();
        }
    }
    page(State(s), h).await
}

/// Tries the place in `settings` (not saved yet): signs in, and says
/// whether backups are there already and whether the passphrase opens
/// them. For SFTP, the server's key seen is trusted from then on when
/// `trust` is set.
#[derive(Deserialize)]
struct Check {
    settings: Settings,
    #[serde(default)]
    trust: bool,
}

async fn check(State(s): State<Server>, h: HeaderMap, Json(v): Json<Check>) -> Response {
    may!(s, h);
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let old = settings(&c).await;
    let mut set = merged(v.settings, &old);
    if v.trust {
        set.sftp_host_key.clear();
    }
    let Ok((private, _)) = ssh_key(&c).await else { return failed() };
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let set = set.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<Value, String> {
                if set.kind == "folder" {
                    std::fs::create_dir_all(set.folder.trim()).map_err(|e| format!("can't use {}: {e}", set.folder.trim()))?;
                }
                let (be, fp) = open_place(&set, &private)?;
                let repo = Repository::new(&RepositoryOptions::default().no_cache(true), &RepositoryBackends::new(be.clone(), None))
                    .map_err(|e| e.to_string())?;
                let there = repo.config_id().map_err(|e| e.to_string())?.is_some();
                let opens = if there { Some(repository(be.clone(), &set, false).is_ok()) } else { None };
                if !there && set.kind != "folder" {
                    // Can we write there? A small file, then gone again.
                    let id = rustic_core::Id::random();
                    be.write_bytes(rustic_core::FileType::Key, &id, false, bytes::Bytes::from_static(b"{}").into())
                        .map_err(|e| e.to_string())?;
                    be.remove(rustic_core::FileType::Key, &id, false).map_err(|e| e.to_string())?;
                }
                Ok(json!({ "ok": true, "there": there, "opens": opens, "host_key": fp }))
            })();
            let _ = tx.send(r);
        });
    }
    match rx.await {
        Ok(Ok(v)) => {
            // Trust the SFTP server's key from now on, for this place.
            if let Some(fp) = v["host_key"].as_str()
                && set.kind == "sftp"
                && (v.get("ok").is_some())
            {
                let mut saved = settings(&c).await;
                if saved.same_place(&set) || saved.sftp_address.is_empty() {
                    saved.sftp_host_key = fp.to_string();
                    let _ = db::set_setting(&c, "backup", &serde_json::to_string(&saved).unwrap()).await;
                }
            }
            Json(v).into_response()
        }
        Ok(Err(e)) => {
            // Check again (with trust) takes the new key.
            let changed = e.starts_with(store::KEY_CHANGED);
            Json(json!({ "ok": false, "error": e, "key_changed": changed })).into_response()
        }
        Err(_) => failed(),
    }
}

async fn run_now(State(s): State<Server>, h: HeaderMap) -> Response {
    may!(s, h);
    if s.backup_running().now.lock().unwrap().is_some() {
        return err(StatusCode::CONFLICT, "running", "a backup is running already");
    }
    let s2 = s.clone();
    tokio::spawn(async move {
        let _ = run(&s2).await;
    });
    StatusCode::ACCEPTED.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_from_a_newer_server_are_refused() {
        use serde_json::json;
        assert!(super::newer_check(&json!({ "version": "0.0.1" })).is_ok());
        assert!(super::newer_check(&json!({ "version": "99.0.0", "schema": crate::SCHEMA_LEVEL })).is_ok(), "same schema");
        assert!(super::newer_check(&json!({ "version": "0.0.1", "schema": crate::SCHEMA_LEVEL + 1 })).unwrap_err().contains("newer"));
        assert!(super::newer_check(&json!({ "version": "99.0.0" })).unwrap_err().contains("99.0.0"));
    }

    #[test]
    fn places() {
        let s = place_from("s3:https://s3.eu-central-003.backblazeb2.com/my-bucket/xenify").unwrap();
        assert_eq!((s.kind.as_str(), s.s3_endpoint.as_str(), s.s3_bucket.as_str()), ("s3", "https://s3.eu-central-003.backblazeb2.com", "my-bucket/xenify"));
        assert_eq!(s.place(), "s3:https://s3.eu-central-003.backblazeb2.com/my-bucket/xenify");
        let s = place_from("sftp:bk@nas.local:2222/backups/xenify").unwrap();
        assert_eq!((s.sftp_address.as_str(), s.sftp_folder.as_str()), ("bk@nas.local:2222", "backups/xenify"));
        assert_eq!(s.place(), "sftp:bk@nas.local:2222/backups/xenify");
        assert_eq!(place_from("/mnt/usb/xenify").unwrap().place(), "/mnt/usb/xenify");
    }

    #[test]
    fn secrets_stay() {
        let old = Settings { s3_secret: "s".into(), passphrase: "p".into(), ..Default::default() };
        let m = merged(Settings::default(), &old);
        assert_eq!((m.s3_secret.as_str(), m.passphrase.as_str()), ("s", "p"));
        assert_eq!(public(&old)["passphrase"], "");
        assert_eq!(public(&old)["passphrase_set"], true);
    }

    /// A new address, storage or bucket doesn't get the kept secrets.
    #[test]
    fn secrets_stay_with_their_place() {
        let old = Settings {
            kind: "sftp".into(),
            sftp_address: "bk@nas.local".into(),
            sftp_password: "pw".into(),
            s3_endpoint: "https://s3.example.org".into(),
            s3_bucket: "b".into(),
            s3_secret: "s".into(),
            passphrase: "p".into(),
            ..Default::default()
        };
        let moved = Settings { sftp_address: "bk@elsewhere.example".into(), ..old.clone() };
        let m = merged(Settings { sftp_password: String::new(), ..moved.clone() }, &old);
        assert_eq!((m.sftp_password.as_str(), m.passphrase.as_str()), ("", "p"));
        let m = merged(Settings { sftp_password: "new".into(), ..moved }, &old);
        assert_eq!(m.sftp_password, "new");
        let same = merged(Settings { sftp_password: String::new(), s3_secret: String::new(), ..old.clone() }, &old);
        assert_eq!((same.sftp_password.as_str(), same.s3_secret.as_str()), ("pw", "s"));
        for o in [
            Settings { kind: "s3".into(), s3_secret: String::new(), s3_bucket: "other".into(), ..old.clone() },
            Settings { kind: "s3".into(), s3_secret: String::new(), s3_endpoint: "https://evil.example".into(), ..old.clone() },
        ] {
            let was = Settings { kind: "s3".into(), ..old.clone() };
            assert_eq!(merged(o, &was).s3_secret, "");
        }
    }

    #[test]
    fn only_admins_move_backups_or_stop_encrypting() {
        let old = Settings { kind: "folder".into(), folder: "/mnt/b".into(), encrypt: true, ..Default::default() };
        assert_eq!(admins_only(&Settings { hour: 5, ..old.clone() }, &old), None);
        assert!(admins_only(&Settings { folder: "/tmp/x".into(), ..old.clone() }, &old).is_some());
        assert!(admins_only(&Settings { kind: "s3".into(), ..old.clone() }, &old).is_some());
        assert!(admins_only(&Settings { encrypt: false, ..old.clone() }, &old).is_some());
    }

    #[test]
    fn database_passwords_stay_off_the_command_line() {
        let (d, p) = without_password("postgres://xen:s%40cret@db.lan:5433/xenify?sslmode=require");
        assert_eq!(p.as_deref(), Some("s@cret"));
        assert!(!d.contains("cret"), "{d}");
        assert!(d.contains("db.lan:5433/xenify") && d.contains("sslmode=require"), "{d}");
        let (d, p) = without_password("postgresql://xen@localhost/x?password=pw&application_name=a");
        assert_eq!((p.as_deref(), d.contains("pw")), (Some("pw"), false), "{d}");
        let (d, p) = without_password("host=/run/pg user=xen password='it\\'s odd' dbname=x");
        assert_eq!(d, "host=/run/pg user=xen dbname=x");
        assert_eq!(p.as_deref(), Some("it's odd"));
        assert_eq!(without_password("postgres://localhost/x"), ("postgres://localhost/x".to_string(), None));
    }
}
