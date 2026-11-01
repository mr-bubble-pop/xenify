//! Where backups go: a folder on this computer (a second disk, a USB disk,
//! a network share the system has mounted), S3-compatible storage or an
//! SFTP server. Each keeps restic's repository layout (`config`, `keys/`,
//! `snapshots/`, `index/`, `data/<2 hex>/`), so restic itself can read it.
//!
//! rustic calls these from its own worker threads (not async); SFTP runs
//! its connection on a small runtime of its own.

use bytes::Bytes;
use rustic_core::{BytesList, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn fail(what: &str, e: impl std::fmt::Display) -> Box<RusticError> {
    RusticError::new(ErrorKind::Backend, "{what}: {why}").attach_context("what", what.to_string()).attach_context("why", e.to_string())
}

/// `config`, or `<dir>/<id>` (`data/<2 hex>/<id>` for data).
fn rel(tpe: FileType, id: &Id) -> String {
    let hex = id.to_hex();
    let hex: &str = hex.as_str();
    match tpe {
        FileType::Config => "config".into(),
        FileType::Pack => format!("data/{}/{hex}", &hex[..2]),
        _ => format!("{}/{hex}", tpe.dirname()),
    }
}

fn id_of(name: &str) -> Option<Id> {
    (name.len() == 64).then(|| name.parse::<Id>().ok()).flatten()
}

fn join(parts: &BytesList) -> Vec<u8> {
    let mut v = Vec::with_capacity(parts.size());
    for b in parts.slice() {
        v.extend_from_slice(b);
    }
    v
}

// ---- A folder ---------------------------------------------------------------

#[derive(Debug)]
pub struct Folder {
    root: PathBuf,
}

impl Folder {
    pub fn new(root: PathBuf) -> Folder {
        Folder { root }
    }
}

impl ReadBackend for Folder {
    fn location(&self) -> String {
        self.root.display().to_string()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        if tpe == FileType::Config {
            return Ok(match std::fs::metadata(self.root.join("config")) {
                Ok(m) => vec![(Id::default(), m.len() as u32)],
                Err(_) => vec![],
            });
        }
        let dir = self.root.join(tpe.dirname());
        let mut out = vec![];
        let mut add = |d: &std::path::Path| {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                if let Some(id) = id_of(&e.file_name().to_string_lossy())
                    && let Ok(m) = e.metadata()
                    && m.is_file()
                {
                    out.push((id, m.len() as u32));
                }
            }
        };
        if tpe == FileType::Pack {
            for sub in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                add(&sub.path());
            }
        } else {
            add(&dir);
        }
        Ok(out)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        std::fs::read(self.root.join(rel(tpe, id))).map(Bytes::from).map_err(|e| fail("reading the backup", e))
    }

    fn read_partial(&self, tpe: FileType, id: &Id, _: bool, offset: u32, length: u32) -> RusticResult<Bytes> {
        let mut f = std::fs::File::open(self.root.join(rel(tpe, id))).map_err(|e| fail("reading the backup", e))?;
        f.seek(SeekFrom::Start(offset.into())).map_err(|e| fail("reading the backup", e))?;
        let mut v = vec![0; length as usize];
        f.read_exact(&mut v).map_err(|e| fail("reading the backup", e))?;
        Ok(v.into())
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        rel(tpe, id)
    }
}

impl WriteBackend for Folder {
    fn create(&self) -> RusticResult<()> {
        for t in ["keys", "snapshots", "index", "data"] {
            std::fs::create_dir_all(self.root.join(t)).map_err(|e| fail("making the backup folder", e))?;
        }
        for i in 0..=255u8 {
            std::fs::create_dir_all(self.root.join("data").join(format!("{i:02x}")))
                .map_err(|e| fail("making the backup folder", e))?;
        }
        Ok(())
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, _: bool, content: BytesList) -> RusticResult<()> {
        let path = self.root.join(rel(tpe, id));
        let tmp = path.with_extension("part");
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let mut f = std::fs::File::create(&tmp)?;
            for b in content.slice() {
                f.write_all(b)?;
            }
            f.sync_all()?;
            std::fs::rename(&tmp, &path)
        };
        write().map_err(|e| fail("writing the backup", e))
    }

    fn remove(&self, tpe: FileType, id: &Id, _: bool) -> RusticResult<()> {
        match std::fs::remove_file(self.root.join(rel(tpe, id))) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(fail("removing from the backup", e)),
            _ => Ok(()),
        }
    }
}

// ---- S3-compatible storage -------------------------------------------------------

#[derive(Debug)]
pub struct S3 {
    bucket: rusty_s3::Bucket,
    creds: rusty_s3::Credentials,
    /// Inside the bucket: "" or "some/folder/".
    prefix: String,
    agent: ureq::Agent,
}

const SIGNED_FOR: Duration = Duration::from_secs(3600);

impl S3 {
    /// `endpoint` like https://s3.eu-central-003.backblazeb2.com; `bucket`
    /// the bucket's name, optionally followed by a folder in it.
    pub fn new(endpoint: &str, bucket: &str, key: &str, secret: &str, region: Option<&str>) -> Result<S3, String> {
        let endpoint = endpoint.trim().trim_end_matches('/');
        let endpoint = if endpoint.contains("://") { endpoint.to_string() } else { format!("https://{endpoint}") };
        let url: url::Url = endpoint.parse().map_err(|_| "that storage address isn't a web address".to_string())?;
        let host = url.host_str().unwrap_or_default().to_string();
        let (name, prefix) = match bucket.trim().trim_matches('/').split_once('/') {
            Some((b, p)) => (b.to_string(), format!("{}/", p.trim_matches('/'))),
            None => (bucket.trim().trim_matches('/').to_string(), String::new()),
        };
        if name.is_empty() {
            return Err("the bucket's name is missing".into());
        }
        let region = region.map(str::to_string).unwrap_or_else(|| region_of(&host));
        // AWS wants the bucket in the host name; the others take either, and
        // a NAS (MinIO) often only the path.
        let style = if host.ends_with("amazonaws.com") { rusty_s3::UrlStyle::VirtualHost } else { rusty_s3::UrlStyle::Path };
        let bucket = rusty_s3::Bucket::new(url, style, name, region).map_err(|e| e.to_string())?;
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .new_agent();
        Ok(S3 { bucket, creds: rusty_s3::Credentials::new(key.trim(), secret.trim()), prefix, agent })
    }

    fn key(&self, tpe: FileType, id: &Id) -> String {
        format!("{}{}", self.prefix, rel(tpe, id))
    }

    fn get(&self, key: &str, range: Option<(u32, u32)>) -> RusticResult<Bytes> {
        use rusty_s3::S3Action;
        let url = self.bucket.get_object(Some(&self.creds), key).sign(SIGNED_FOR);
        let mut r = retry(|| {
            let mut req = self.agent.get(url.as_str());
            if let Some((at, len)) = range {
                req = req.header("Range", &format!("bytes={}-{}", at, at as u64 + len as u64 - 1));
            }
            req.call()
        })
        .map_err(|e| fail("reading the backup", e))?;
        if !r.status().is_success() {
            return Err(fail("reading the backup", status_text(&mut r)));
        }
        let v = r.body_mut().with_config().limit(u64::MAX).read_to_vec().map_err(|e| fail("reading the backup", e))?;
        Ok(v.into())
    }

    /// Every key under `prefix`, with its size.
    fn list(&self, prefix: &str) -> RusticResult<Vec<(String, u64)>> {
        use rusty_s3::S3Action;
        let mut out = vec![];
        let mut token: Option<String> = None;
        loop {
            let mut a = self.bucket.list_objects_v2(Some(&self.creds));
            a.with_prefix(prefix);
            if let Some(t) = &token {
                a.with_continuation_token(t.clone());
            }
            let url = a.sign(SIGNED_FOR);
            let mut r = retry(|| self.agent.get(url.as_str()).call()).map_err(|e| fail("listing the backup", e))?;
            if !r.status().is_success() {
                return Err(fail("listing the backup", status_text(&mut r)));
            }
            let text = r.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| fail("listing the backup", e))?;
            let page = rusty_s3::actions::ListObjectsV2::parse_response(&text).map_err(|e| fail("listing the backup", e))?;
            out.extend(page.contents.into_iter().map(|o| (o.key, o.size.unwrap_or(0))));
            match page.next_continuation_token {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(out)
    }
}

/// The region from a provider's address (s3.<region>.amazonaws.com,
/// s3.<region>.backblazeb2.com, s3.<region>.wasabisys.com …); "auto"
/// (Cloudflare R2) or us-east-1 otherwise, which the others accept.
fn region_of(host: &str) -> String {
    if host.ends_with("r2.cloudflarestorage.com") {
        return "auto".into();
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 4 && parts[0] == "s3" && !parts[1].is_empty() && parts[1].contains('-') {
        return parts[1].to_string();
    }
    if parts.len() >= 4 && parts[0].starts_with("s3-") {
        return parts[0][3..].to_string();
    }
    "us-east-1".into()
}

fn status_text(r: &mut ureq::http::Response<ureq::Body>) -> String {
    let code = r.status();
    let body = r.body_mut().with_config().limit(64 << 10).read_to_string().unwrap_or_default();
    // S3's errors are XML: <Code>…</Code><Message>…</Message>.
    let pick = |tag: &str| body.split(&format!("<{tag}>")).nth(1).and_then(|s| s.split(&format!("</{tag}>")).next()).map(str::to_string);
    match (pick("Code"), pick("Message")) {
        (Some(c), Some(m)) => format!("{c}: {m}"),
        (Some(c), None) => c,
        _ => format!("the storage answered {code}"),
    }
}

/// Network trouble is tried again (3 times, waiting longer each time).
fn retry<T>(mut f: impl FnMut() -> Result<T, ureq::Error>) -> Result<T, ureq::Error> {
    let mut wait = Duration::from_secs(2);
    for _ in 0..3 {
        match f() {
            Err(ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed) => {
                std::thread::sleep(wait);
                wait *= 4;
            }
            other => return other,
        }
    }
    f()
}

impl ReadBackend for S3 {
    fn location(&self) -> String {
        format!("s3:{}/{}", self.bucket.base_url(), self.prefix)
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        if tpe == FileType::Config {
            let key = format!("{}config", self.prefix);
            return Ok(self.list(&key)?.into_iter().filter(|(k, _)| *k == key).map(|(_, n)| (Id::default(), n as u32)).collect());
        }
        let prefix = format!("{}{}/", self.prefix, tpe.dirname());
        Ok(self
            .list(&prefix)?
            .into_iter()
            .filter_map(|(k, n)| Some((id_of(k.rsplit('/').next()?)?, n as u32)))
            .collect())
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.get(&self.key(tpe, id), None)
    }

    fn read_partial(&self, tpe: FileType, id: &Id, _: bool, offset: u32, length: u32) -> RusticResult<Bytes> {
        self.get(&self.key(tpe, id), Some((offset, length)))
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        self.key(tpe, id)
    }
}

impl WriteBackend for S3 {
    fn write_bytes(&self, tpe: FileType, id: &Id, _: bool, content: BytesList) -> RusticResult<()> {
        use rusty_s3::S3Action;
        let body = join(&content);
        let url = self.bucket.put_object(Some(&self.creds), &self.key(tpe, id)).sign(SIGNED_FOR);
        let mut r = retry(|| self.agent.put(url.as_str()).send(&body[..])).map_err(|e| fail("writing the backup", e))?;
        if !r.status().is_success() {
            return Err(fail("writing the backup", status_text(&mut r)));
        }
        Ok(())
    }

    fn remove(&self, tpe: FileType, id: &Id, _: bool) -> RusticResult<()> {
        use rusty_s3::S3Action;
        let url = self.bucket.delete_object(Some(&self.creds), &self.key(tpe, id)).sign(SIGNED_FOR);
        let mut r = retry(|| self.agent.delete(url.as_str()).call()).map_err(|e| fail("removing from the backup", e))?;
        if !r.status().is_success() && r.status().as_u16() != 404 {
            return Err(fail("removing from the backup", status_text(&mut r)));
        }
        Ok(())
    }
}

// ---- SFTP ----------------------------------------------------------------------

/// How to sign in to an SFTP server.
#[derive(Clone)]
pub enum SshAuth {
    /// The server's own key (its public half goes in authorized_keys).
    Key(String),
    Password(String),
}

pub struct Sftp {
    rt: tokio::runtime::Runtime,
    sftp: russh_sftp::client::SftpSession,
    _session: russh::client::Handle<HostCheck>,
    root: String,
    label: String,
}

impl std::fmt::Debug for Sftp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Checks the SFTP server's key against the one seen the first time.
pub struct HostCheck {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl russh::client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let sha = russh::keys::HashAlg::Sha256;
        let fp = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(sha),
            russh::keys::PublicKeyOrCertificate::Certificate(c) => c.public_key().fingerprint(sha),
        }
        .to_string();
        *self.seen.lock().unwrap() = Some(fp.clone());
        Ok(self.expected.as_deref().is_none_or(|e| e == fp))
    }
}

/// How a refused SFTP server key is reported (Admin › Backups offers to
/// take the new one on the next Check).
pub const KEY_CHANGED: &str = "The backup server's key changed.";

/// `user@host[:port]`, split.
pub fn ssh_address(a: &str) -> Result<(String, String, u16), String> {
    let (user, rest) = a.trim().split_once('@').ok_or("the SFTP address needs a user name: user@host")?;
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.starts_with('[') => {
            (h.trim_matches(['[', ']']).to_string(), p.parse().map_err(|_| "the port after : isn't a number")?)
        }
        _ => (rest.to_string(), 22),
    };
    if user.is_empty() || host.is_empty() {
        return Err("the SFTP address should look like user@host".into());
    }
    Ok((user.to_string(), host, port))
}

impl Sftp {
    /// Signs in. `host_key` is the SHA256 fingerprint seen before (None the
    /// first time); the one seen comes back either way, as does whether
    /// it didn't match.
    pub fn connect(address: &str, folder: &str, auth: &SshAuth, host_key: Option<&str>) -> Result<(Sftp, String), String> {
        let (user, host, port) = ssh_address(address)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let seen = Arc::new(Mutex::new(None));
        let check = HostCheck { expected: host_key.map(str::to_string), seen: seen.clone() };
        let (session, sftp) = rt.block_on(async {
            let config = Arc::new(russh::client::Config {
                inactivity_timeout: Some(Duration::from_secs(300)),
                keepalive_interval: Some(Duration::from_secs(30)),
                ..Default::default()
            });
            let mut s = match tokio::time::timeout(Duration::from_secs(20), russh::client::connect(config, (host.as_str(), port), check)).await {
                Err(_) => return Err(format!("{host}:{port} didn't answer")),
                Ok(Err(russh::Error::UnknownKey)) => {
                    return Err(format!(
                        "{KEY_CHANGED} If that's expected, press Check to accept the new one ({}). If not, someone may be pretending to be {host}.",
                        seen.lock().unwrap().clone().unwrap_or_default()
                    ));
                }
                Ok(Err(e)) => return Err(format!("can't connect to {host}:{port}: {e}")),
                Ok(Ok(s)) => s,
            };
            let ok = match auth {
                SshAuth::Password(p) => s.authenticate_password(&user, p).await.map_err(|e| e.to_string())?.success(),
                SshAuth::Key(pem) => {
                    let key = russh::keys::decode_secret_key(pem, None).map_err(|e| e.to_string())?;
                    let hash = s.best_supported_rsa_hash().await.ok().flatten().flatten();
                    s.authenticate_publickey(&user, russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                        .await
                        .map_err(|e| e.to_string())?
                        .success()
                }
            };
            if !ok {
                return Err(match auth {
                    SshAuth::Password(_) => format!("{host} didn't accept that user name and password"),
                    SshAuth::Key(_) => format!("{host} didn't accept this server's key: add it to ~/.ssh/authorized_keys of {user} there"),
                });
            }
            let ch = s.channel_open_session().await.map_err(|e| e.to_string())?;
            ch.request_subsystem(true, "sftp").await.map_err(|e| e.to_string())?;
            let sftp = russh_sftp::client::SftpSession::new(ch.into_stream())
                .await
                .map_err(|e| format!("{host} doesn't offer SFTP: {e}"))?;
            Ok((s, sftp))
        })?;
        let fp = seen.lock().unwrap().clone().unwrap_or_default();
        let root = folder.trim().trim_end_matches('/').to_string();
        let root = if root.is_empty() { ".".to_string() } else { root };
        Ok((Sftp { rt, sftp, _session: session, root, label: format!("sftp:{user}@{host}:{port}/{folder}") }, fp))
    }

    fn path(&self, rel: &str) -> String {
        format!("{}/{rel}", self.root)
    }

    fn mkdirs(&self, path: &str) -> Result<(), String> {
        self.rt.block_on(async {
            let mut at = String::new();
            for part in path.split('/') {
                if at.is_empty() && part.is_empty() {
                    at.push('/');
                    continue;
                }
                at = if at.is_empty() || at == "/" { format!("{at}{part}") } else { format!("{at}/{part}") };
                if part == "." || self.sftp.try_exists(&at).await.unwrap_or(false) {
                    continue;
                }
                self.sftp.create_dir(&at).await.map_err(|e| format!("can't make {at}: {e}"))?;
            }
            Ok(())
        })
    }
}

impl ReadBackend for Sftp {
    fn location(&self) -> String {
        self.label.clone()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.rt.block_on(async {
            if tpe == FileType::Config {
                return Ok(match self.sftp.metadata(self.path("config")).await {
                    Ok(m) => vec![(Id::default(), m.size.unwrap_or(0) as u32)],
                    Err(_) => vec![],
                });
            }
            let mut dirs = vec![self.path(tpe.dirname())];
            if tpe == FileType::Pack {
                dirs = match self.sftp.read_dir(&dirs[0]).await {
                    Ok(r) => r.map(|e| format!("{}/{}", dirs[0], e.file_name())).collect(),
                    Err(_) => vec![],
                };
            }
            let mut out = vec![];
            for d in dirs {
                let Ok(r) = self.sftp.read_dir(&d).await else { continue };
                for e in r {
                    if let Some(id) = id_of(&e.file_name()) {
                        out.push((id, e.metadata().size.unwrap_or(0) as u32));
                    }
                }
            }
            Ok(out)
        })
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.rt
            .block_on(self.sftp.read(self.path(&rel(tpe, id))))
            .map(Bytes::from)
            .map_err(|e| fail("reading the backup", e))
    }

    fn read_partial(&self, tpe: FileType, id: &Id, _: bool, offset: u32, length: u32) -> RusticResult<Bytes> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        self.rt
            .block_on(async {
                let mut f = self.sftp.open(self.path(&rel(tpe, id))).await?;
                f.seek(SeekFrom::Start(offset.into())).await?;
                let mut v = vec![0; length as usize];
                f.read_exact(&mut v).await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(v)
            })
            .map(Bytes::from)
            .map_err(|e| fail("reading the backup", e))
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        rel(tpe, id)
    }
}

impl WriteBackend for Sftp {
    fn create(&self) -> RusticResult<()> {
        let mut dirs: Vec<String> = ["keys", "snapshots", "index", "data"].iter().map(|d| self.path(d)).collect();
        dirs.extend((0..=255u8).map(|i| self.path(&format!("data/{i:02x}"))));
        self.mkdirs(&self.root).map_err(|e| fail("making the backup folder", e))?;
        self.rt
            .block_on(async {
                for d in dirs {
                    if !self.sftp.try_exists(&d).await.unwrap_or(false) {
                        self.sftp.create_dir(&d).await?;
                    }
                }
                Ok::<_, russh_sftp::client::error::Error>(())
            })
            .map_err(|e| fail("making the backup folder", e))
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, _: bool, content: BytesList) -> RusticResult<()> {
        use tokio::io::AsyncWriteExt;
        let path = self.path(&rel(tpe, id));
        let tmp = format!("{path}.part");
        self.rt
            .block_on(async {
                let mut f = self.sftp.create(&tmp).await?;
                for b in content.slice() {
                    f.write_all(b).await?;
                }
                f.shutdown().await?;
                let _ = self.sftp.remove_file(&path).await;
                self.sftp.rename(&tmp, &path).await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
            })
            .map_err(|e| fail("writing the backup", e))
    }

    fn remove(&self, tpe: FileType, id: &Id, _: bool) -> RusticResult<()> {
        let path = self.path(&rel(tpe, id));
        self.rt.block_on(async {
            if self.sftp.try_exists(&path).await.unwrap_or(true) {
                self.sftp.remove_file(&path).await.map_err(|e| fail("removing from the backup", e))?;
            }
            Ok(())
        })
    }
}

/// A new key for this server to sign in to SFTP servers with: (private
/// key in OpenSSH form, the line for authorized_keys).
pub fn new_ssh_key(name: &str) -> Result<(String, String), String> {
    use russh::keys::ssh_key::LineEnding;
    let mut k = russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).map_err(|e| e.to_string())?;
    k.set_comment(name);
    let private = k.to_openssh(LineEnding::LF).map_err(|e| e.to_string())?.to_string();
    let public = k.public_key().to_openssh().map_err(|e| e.to_string())?;
    Ok((private, public))
}

// ---- Upload speed limit --------------------------------------------------------

/// Keeps uploads under a speed (bytes a second), for a home connection.
#[derive(Debug)]
pub struct Throttled {
    inner: Arc<dyn WriteBackend>,
    per_second: u64,
    next: Mutex<Instant>,
}

impl Throttled {
    pub fn new(inner: Arc<dyn WriteBackend>, per_second: u64) -> Throttled {
        Throttled { inner, per_second: per_second.max(1), next: Mutex::new(Instant::now()) }
    }
}

impl ReadBackend for Throttled {
    fn location(&self) -> String {
        self.inner.location()
    }
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.inner.list_with_size(tpe)
    }
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.inner.read_full(tpe, id)
    }
    fn read_partial(&self, tpe: FileType, id: &Id, c: bool, offset: u32, length: u32) -> RusticResult<Bytes> {
        self.inner.read_partial(tpe, id, c, offset, length)
    }
    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        self.inner.warmup_path(tpe, id)
    }
}

impl WriteBackend for Throttled {
    fn create(&self) -> RusticResult<()> {
        self.inner.create()
    }
    fn write_bytes(&self, tpe: FileType, id: &Id, c: bool, content: BytesList) -> RusticResult<()> {
        // Each upload takes its turn: it may start when the one before
        // would have finished at the limit.
        let took = Duration::from_secs_f64(content.size() as f64 / self.per_second as f64);
        let start = {
            let mut next = self.next.lock().unwrap();
            let start = (*next).max(Instant::now());
            *next = start + took;
            start
        };
        if let Some(wait) = start.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        self.inner.write_bytes(tpe, id, c, content)
    }
    fn remove(&self, tpe: FileType, id: &Id, c: bool) -> RusticResult<()> {
        self.inner.remove(tpe, id, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_and_addresses() {
        assert_eq!(region_of("s3.eu-central-003.backblazeb2.com"), "eu-central-003");
        assert_eq!(region_of("s3.eu-west-2.amazonaws.com"), "eu-west-2");
        assert_eq!(region_of("abc.r2.cloudflarestorage.com"), "auto");
        assert_eq!(region_of("nas.local"), "us-east-1");
        assert_eq!(ssh_address("bk@nas.local").unwrap(), ("bk".into(), "nas.local".into(), 22));
        assert_eq!(ssh_address("u123@u123.your-storagebox.de:23").unwrap(), ("u123".into(), "u123.your-storagebox.de".into(), 23));
        assert!(ssh_address("nas.local").is_err());
    }
}
