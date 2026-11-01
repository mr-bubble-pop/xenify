//! The server as a system service: started with the computer (before
//! anyone signs in), kept running, and updated with no downtime.
//!
//! `xenify-server service run` is a small supervisor. It starts the server's
//! own database (when it has one), then the current version of the server
//! from its folder of versions, and restarts it if it stops. The running
//! server updates itself into that folder (following Admin › Apps &
//! updates); the supervisor sees the new version, starts it beside the old
//! one (they share the ports), and once it listens, tells the old one to
//! finish (by closing its input) and go.
//!
//! `service install` registers the supervisor with the system: a systemd
//! unit on Linux, a launch daemon on a Mac, a service on Windows.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, ChildStdin, Command};

/// What a server started by the supervisor sees, to stop when its input
/// closes and to update itself into `XENIFY_SERVICE_VERSIONS`.
pub const SUPERVISED: &str = "XENIFY_SUPERVISED";
pub const VERSIONS: &str = "XENIFY_SERVICE_VERSIONS";

pub fn program_name() -> &'static str {
    if cfg!(windows) { "xenify-server.exe" } else { "xenify-server" }
}

/// Another program kept running beside the server (the AI runner).
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Also {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Where it runs (its work files).
    pub dir: Option<PathBuf>,
}

/// The server's own PostgreSQL (`program`: its bin/postgres).
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Postgres {
    pub program: PathBuf,
    pub data: PathBuf,
}

pub struct Plan {
    /// The service's folder: the supervisor's own copy and `versions/`.
    pub dir: PathBuf,
    /// The server's arguments before `serve` (e.g. --config <file>).
    pub args: Vec<String>,
    pub postgres: Option<Postgres>,
    pub also: Vec<Also>,
    /// Encryption at rest: while the folder is locked, the database (in it)
    /// waits; the server starts it once it's unlocked.
    pub at_rest: Option<crate::atrest::AtRest>,
}

impl Plan {
    fn store(&self) -> xenify_hub_core::Store {
        xenify_hub_core::Store::new(self.dir.join("versions"))
    }

    /// The version to run: the one installed by updating, unless the
    /// service's own copy (put in place by the installer since) is newer.
    fn program(&self) -> PathBuf {
        let store = self.store();
        let own = semver::Version::parse(env!("CARGO_PKG_VERSION")).ok();
        let older = |v: &str| matches!((semver::Version::parse(v).ok(), &own), (Some(v), Some(own)) if v < *own);
        store
            .installed("server")
            .filter(|i| !older(&i.version))
            .and_then(|_| store.program("server"))
            .filter(|p| p.exists())
            .unwrap_or_else(|| self.dir.join(program_name()))
    }
}

struct Worker {
    program: PathBuf,
    child: Child,
    /// Closed to ask it to finish and stop.
    input: Option<ChildStdin>,
    listening: tokio::sync::watch::Receiver<bool>,
}

fn start(plan: &Plan, program: &Path) -> std::io::Result<Worker> {
    let mut cmd = Command::new(program);
    cmd.args(&plan.args)
        .arg("serve")
        .env(SUPERVISED, "1")
        .env(VERSIONS, plan.dir.join("versions"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    let input = child.stdin.take();
    let out = child.stdout.take();
    let (tx, listening) = tokio::sync::watch::channel(false);
    if let Some(out) = out {
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                println!("{line}");
                if line.contains(": listening on ") {
                    let _ = tx.send(true);
                }
            }
        });
    }
    println!("xenify-server service: started {}", program.display());
    Ok(Worker { program: program.to_path_buf(), child, input, listening })
}

impl Worker {
    /// Waits until it listens (true), or it stops or takes too long.
    async fn ready(&mut self) -> bool {
        let mut listening = self.listening.clone();
        tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(90), listening.wait_for(|l| *l)) => matches!(r, Ok(Ok(_))),
            _ = self.child.wait() => false,
        }
    }

    /// Asks it to finish what it's doing and stop; after 30 s it's ended.
    async fn finish(mut self) {
        drop(self.input.take());
        if tokio::time::timeout(Duration::from_secs(30), self.child.wait()).await.is_err() {
            let _ = self.child.kill().await;
        }
    }
}

/// Starts the database, if it's the server's own.
async fn start_postgres(pg: &Postgres) {
    let pg = pg.clone();
    let r = tokio::task::spawn_blocking(move || {
        let p =
            xenify_hub_core::pg::Pg::new(&pg.program, pg.data.clone()).ok_or("PostgreSQL isn't where it should be")?;
        p.start()
    })
    .await;
    match r {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("xenify-server service: {e}"),
        Err(e) => eprintln!("xenify-server service: {e}"),
    }
}

async fn stop_postgres(pg: &Postgres) {
    let pg = pg.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(p) = xenify_hub_core::pg::Pg::new(&pg.program, pg.data.clone()) {
            p.stop();
        }
    })
    .await;
}

async fn stopped(stop: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// Keeps `also` running until `stop`.
fn keep_running(also: Also, mut stop: tokio::sync::watch::Receiver<bool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut wait = 2;
        loop {
            let spawned = {
                let mut cmd = Command::new(&also.program);
                cmd.args(&also.args).kill_on_drop(true);
                if let Some(d) = &also.dir {
                    let _ = std::fs::create_dir_all(d);
                    cmd.current_dir(d);
                }
                cmd.spawn()
            };
            let started = std::time::Instant::now();
            match spawned {
                Ok(mut child) => {
                    tokio::select! {
                        _ = child.wait() => {}
                        _ = stopped(&mut stop) => {
                            let _ = child.kill().await;
                            return;
                        }
                    }
                }
                Err(e) => eprintln!("xenify-server service: couldn't start {}: {e}", also.program.display()),
            }
            if started.elapsed() > Duration::from_secs(60) {
                wait = 2;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = stopped(&mut stop) => return,
            }
            wait = (wait * 2).min(60);
        }
    })
}

/// Runs the server until `stop` finishes.
pub async fn supervise(plan: Plan, stop: impl std::future::Future<Output = ()>) {
    #[cfg(windows)]
    if let Err(e) = check_protected(&plan) {
        eprintln!("xenify-server service: {e}");
        return;
    }
    let locked = plan.at_rest.as_ref().is_some_and(|a| !crate::atrest::unlocked(a));
    if let Some(pg) = plan.postgres.as_ref().filter(|_| !locked) {
        start_postgres(pg).await;
    }
    let (others_stop, others) = tokio::sync::watch::channel(false);
    let helpers: Vec<_> = plan.also.iter().cloned().map(|a| keep_running(a, others.clone())).collect();
    tokio::pin!(stop);
    let mut current: Option<Worker> = None;
    // A version that didn't start: not tried again until another comes.
    let mut bad: Option<PathBuf> = None;
    let mut wait = 2;
    loop {
        // Not running (first start, or it stopped): start it.
        if current.is_none() {
            let program = plan.program();
            let program = if bad.as_ref() == Some(&program) { plan.dir.join(program_name()) } else { program };
            match start(&plan, &program) {
                Ok(mut w) => {
                    if w.ready().await {
                        wait = 2;
                        current = Some(w);
                    } else {
                        eprintln!("xenify-server service: {} didn't start", program.display());
                        w.finish().await;
                    }
                }
                Err(e) => eprintln!("xenify-server service: couldn't start {}: {e}", program.display()),
            }
            if current.is_none() {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                    _ = &mut stop => break,
                }
                wait = (wait * 2).min(60);
                continue;
            }
        }
        let w = current.as_mut().expect("running");
        tokio::select! {
            _ = &mut stop => break,
            r = w.child.wait() => {
                eprintln!("xenify-server service: the server stopped ({}); starting it again", r.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string()));
                current = None;
                continue;
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
        // A newer version installed: it takes over.
        let want = plan.program();
        if want != w.program && bad.as_ref() != Some(&want) {
            match start(&plan, &want) {
                Ok(mut new) => {
                    if new.ready().await {
                        let old = current.replace(new).expect("running");
                        tokio::spawn(old.finish());
                        bad = None;
                    } else {
                        eprintln!(
                            "xenify-server service: {} didn't start; staying on the version running",
                            want.display()
                        );
                        new.finish().await;
                        bad = Some(want);
                    }
                }
                Err(e) => {
                    eprintln!("xenify-server service: couldn't start {}: {e}", want.display());
                    bad = Some(want);
                }
            }
        }
    }
    println!("xenify-server service: stopping");
    let _ = others_stop.send(true);
    if let Some(w) = current {
        w.finish().await;
    }
    for h in helpers {
        let _ = h.await;
    }
    if let Some(pg) = &plan.postgres {
        stop_postgres(pg).await;
    }
}

// ---- Installing the service --------------------------------------------

/// What `service install` sets up.
pub struct Install {
    /// The service's folder (its copy of the program, the versions).
    pub dir: PathBuf,
    /// The settings file it starts with.
    pub config: PathBuf,
    /// Who it runs as (Linux and Mac; never root). Windows services run as
    /// their own account, NT SERVICE\XenifyServer.
    pub user: Option<String>,
}

fn other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Copies a folder. Links are refused: run as root or an administrator, a
/// link could lead it to read or write files somewhere else.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        // (Doesn't follow a link.)
        let kind = e.file_type()?;
        let target = to.join(e.file_name());
        if kind.is_symlink() {
            return Err(other(format!("{} is a link: not copied", e.path().display())));
        } else if kind.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            copy_file(&e.path(), &target)?;
        }
    }
    Ok(())
}

/// Copies one file, following a link at neither end.
fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(to) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(to)?,
        Ok(_) => std::fs::remove_file(to)?,
        Err(_) => {}
    }
    let mut src = std::fs::OpenOptions::new();
    src.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut src, libc::O_NOFOLLOW);
    // FILE_FLAG_OPEN_REPARSE_POINT: the link itself, not where it leads.
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut src, 0x0020_0000);
    let mut src = src.open(from)?;
    let meta = src.metadata()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(other(format!("{} isn't a plain file: not copied", from.display())));
    }
    let mut out = std::fs::OpenOptions::new();
    out.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        out.mode(meta.permissions().mode() & 0o755).custom_flags(libc::O_NOFOLLOW);
    }
    let mut dst = out.open(to)?;
    std::io::copy(&mut src, &mut dst)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        dst.set_permissions(std::fs::Permissions::from_mode(meta.permissions().mode() & 0o755))?;
    }
    Ok(())
}

/// Removes what's at `path` (a link itself, not where it leads).
fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// As root: the service's folder belongs to root and nobody else can
/// change it or any folder above it (else someone could swap a file or
/// folder for a link while root writes there). Only `versions/` is the
/// service user's.
#[cfg(unix)]
fn prepare_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let unsafe_dir = || {
        format!(
            "{}: put the service in a folder only root can change (the default, {}, is)",
            dir.display(),
            if cfg!(target_os = "macos") { "/Library/Application Support/Xenify/Server" } else { "/opt/xenify-server" }
        )
    };
    if std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err(unsafe_dir());
    }
    let real = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for above in real.ancestors().skip(1) {
        let m = std::fs::metadata(above).map_err(|e| format!("{}: {e}", above.display()))?;
        if m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err(unsafe_dir());
        }
    }
    std::os::unix::fs::lchown(dir, Some(0), Some(0)).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    // Left by an earlier install, which gave the whole folder to the user.
    for old in ["web", "xenify-server.part", "service.log"] {
        remove_any(&dir.join(old)).map_err(|e| format!("{}: {e}", dir.join(old).display()))?;
    }
    Ok(())
}

#[cfg(windows)]
fn prepare_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Administrators and the system may change it; everyone else may only
    // read and run what's in it (by their ids, whatever Windows' language).
    icacls(dir, &["/inheritance:r", "/grant:r", "*S-1-5-32-544:(OI)(CI)F", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-545:(OI)(CI)RX"])?;
    remove_any(&dir.join("web")).map_err(|e| e.to_string())?;
    Ok(())
}

/// Puts this program (and its admin pages) in the service's folder, which
/// stays put while versions come and go.
fn place_program(dir: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = dir.join(program_name());
    prepare_dir(dir)?;
    if exe != target {
        let part = target.with_extension("part");
        copy_file(&exe, &part).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        if let Some(web) = exe.parent().map(|p| p.join("web")).filter(|w| w.join("index.html").is_file()) {
            remove_any(&dir.join("web")).map_err(|e| e.to_string())?;
            copy_dir(&web, &dir.join("web")).map_err(|e| e.to_string())?;
        }
    }
    Ok(target)
}

/// The service user's own part of the folder: the versions it updates to.
#[cfg(unix)]
fn give_versions(dir: &Path, user: &str) -> Result<(), String> {
    let versions = dir.join("versions");
    if std::fs::symlink_metadata(&versions).is_ok_and(|m| !m.is_dir()) {
        remove_any(&versions).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&versions).map_err(|e| format!("{}: {e}", versions.display()))?;
    // -h: a link is changed itself, never what it leads to.
    run(std::process::Command::new("chown").args(["-R", "-h", "--"]).arg(user).arg(&versions))
}

/// Who the service runs as: a plain account name (it goes into the
/// system's service file), never nobody (which would be root).
#[cfg(unix)]
fn service_user(i: &Install) -> Result<&str, String> {
    let u = i.user.as_deref().unwrap_or("");
    if u.is_empty() {
        return Err("say who the service runs as (--user <name>): it doesn't run as root".into());
    }
    if u == "root" || u.starts_with('-') || !u.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err(format!("“{u}” can't be the service's user"));
    }
    Ok(u)
}

/// Paths written into the system's service file: nothing that could end
/// the line or the quotes they're in.
#[cfg(unix)]
fn plain_path(p: &Path) -> Result<String, String> {
    let s = p.display().to_string();
    if s.contains(['\n', '\r', '"', '\\', '%']) {
        return Err(format!("{s}: use a path without quotes, backslashes, % or line breaks"));
    }
    Ok(s)
}

#[cfg(target_os = "linux")]
const UNIT: &str = "/etc/systemd/system/xenify-server.service";

#[cfg(target_os = "linux")]
pub fn unit_text(program: &Path, i: &Install) -> String {
    let user = i.user.as_deref().map(|u| format!("User={u}\n")).unwrap_or_default();
    format!(
        "# Made by `xenify-server service install`.\n\
         [Unit]\n\
         Description=Xenify server\n\
         Wants=network-online.target\n\
         After=network-online.target postgresql.service\n\
         \n\
         [Service]\n\
         {user}\
         ExecStart=\"{}\" --config \"{}\" service run --dir \"{}\"\n\
         # The supervisor tells the server to finish; then everything stops.\n\
         KillMode=mixed\n\
         TimeoutStopSec=45\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         # Ports below 1024 (443, for a certificate from Let's Encrypt).\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\
         NoNewPrivileges=true\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        program.display(),
        i.config.display(),
        i.dir.display()
    )
}

fn run(cmd: &mut std::process::Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{:?}: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[cfg(target_os = "linux")]
pub fn install(i: &Install) -> Result<(), String> {
    let user = service_user(i)?;
    for p in [&i.dir, &i.config] {
        plain_path(p)?;
    }
    let program = place_program(&i.dir)?;
    give_versions(&i.dir, user)?;
    std::fs::write(UNIT, unit_text(&program, i)).map_err(|e| format!("{UNIT}: {e} (run it as root)"))?;
    run(std::process::Command::new("systemctl").arg("daemon-reload"))?;
    run(std::process::Command::new("systemctl").args(["enable", "--now", "xenify-server"]))
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<(), String> {
    let _ = run(std::process::Command::new("systemctl").args(["disable", "--now", "xenify-server"]));
    match std::fs::remove_file(UNIT) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{UNIT}: {e} (run it as root)")),
    }
    run(std::process::Command::new("systemctl").arg("daemon-reload"))
}

#[cfg(target_os = "linux")]
pub fn installed() -> bool {
    Path::new(UNIT).exists()
}

#[cfg(target_os = "macos")]
const PLIST: &str = "/Library/LaunchDaemons/net.xenify.server.plist";

#[cfg(target_os = "macos")]
pub fn install(i: &Install) -> Result<(), String> {
    let user = service_user(i)?;
    let program = place_program(&i.dir)?;
    give_versions(&i.dir, user)?;
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
    let arg = |s: String| format!("<string>{}</string>", esc(&s));
    let args = [
        program.display().to_string(),
        "--config".into(),
        i.config.display().to_string(),
        "service".into(),
        "run".into(),
        "--dir".into(),
        i.dir.display().to_string(),
    ]
    .into_iter()
    .map(arg)
    .collect::<String>();
    let user = format!("<key>UserName</key>{}", arg(user.into()));
    // In the folder only root can change (launchd writes it as root).
    let log = i.dir.join("service.log").display().to_string();
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict><key>Label</key><string>net.xenify.server</string>\
         <key>ProgramArguments</key><array>{args}</array>{user}\
         <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\
         <key>ExitTimeOut</key><integer>45</integer>\
         <key>StandardOutPath</key>{l}<key>StandardErrorPath</key>{l}</dict></plist>\n",
        l = arg(log)
    );
    std::fs::write(PLIST, text).map_err(|e| format!("{PLIST}: {e} (run it with sudo)"))?;
    let _ = run(std::process::Command::new("launchctl").args(["bootout", "system/net.xenify.server"]));
    run(std::process::Command::new("launchctl").args(["bootstrap", "system", PLIST]))
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<(), String> {
    let _ = run(std::process::Command::new("launchctl").args(["bootout", "system/net.xenify.server"]));
    match std::fs::remove_file(PLIST) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{PLIST}: {e} (run it with sudo)")),
    }
}

#[cfg(target_os = "macos")]
pub fn installed() -> bool {
    Path::new(PLIST).exists()
}

#[cfg(windows)]
pub const WINDOWS_NAME: &str = "XenifyServer";

/// The service's own account (a virtual account: no password, no rights
/// beyond what it's given).
#[cfg(windows)]
const WINDOWS_ACCOUNT: &str = r"NT SERVICE\XenifyServer";

#[cfg(windows)]
fn icacls(path: &Path, args: &[&str]) -> Result<(), String> {
    run(std::process::Command::new("icacls").arg(path).args(args))
}

/// The settings and the programs the service starts (its database, the AI
/// runner), copied into the service's folder, which only administrators
/// can change: run by the service, nothing may come from a folder that
/// someone without administrator rights can change. Their data folders
/// stay where they are, for the service's account to use. Returns the
/// settings file the service starts with.
#[cfg(windows)]
fn protect(i: &Install) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let text = std::fs::read_to_string(&i.config).map_err(|e| format!("{}: {e}", i.config.display()))?;
    let mut t: toml::Table = text.parse().map_err(|e| format!("{}: {e}", i.config.display()))?;
    let parts = i.dir.join("parts");
    remove_any(&parts).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&parts).map_err(|e| e.to_string())?;
    let mut data: Vec<PathBuf> = Vec::new();
    if let Some(d) = t.get("data").and_then(|v| v.as_str()) {
        data.push(d.into());
    }
    // PostgreSQL: its whole folder (bin/, lib/, share/).
    if let Some(pg) = t.get_mut("postgres").and_then(|v| v.as_table_mut()) {
        let program = PathBuf::from(pg.get("program").and_then(|v| v.as_str()).unwrap_or_default());
        let bin = program.parent().ok_or("the database's program isn't named")?;
        let root = bin.parent().ok_or("the database's program isn't in a bin folder")?;
        let to = parts.join("postgres");
        copy_dir(root, &to).map_err(|e| format!("{}: {e}", root.display()))?;
        let placed = to.join(bin.file_name().unwrap_or_default()).join(program.file_name().unwrap_or_default());
        pg.insert("program".into(), placed.display().to_string().into());
        if let Some(d) = pg.get("data").and_then(|v| v.as_str()) {
            data.push(d.into());
        }
    }
    // Other programs: their folders.
    if let Some(also) = t.get_mut("also").and_then(|v| v.as_array_mut()) {
        for (n, a) in also.iter_mut().filter_map(|a| a.as_table_mut()).enumerate() {
            let program = PathBuf::from(a.get("program").and_then(|v| v.as_str()).unwrap_or_default());
            let from = program.parent().ok_or("a program to start isn't named")?;
            let to = parts.join(format!("also-{n}"));
            copy_dir(from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
            a.insert("program".into(), to.join(program.file_name().unwrap_or_default()).display().to_string().into());
            if let Some(d) = a.get("dir").and_then(|v| v.as_str()) {
                data.push(d.into());
            }
            // Its models folder, when it's given one.
            let args: Vec<String> =
                a.get("args").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()).map(String::from).collect();
            if let Some(m) = args.iter().position(|x| x == "--models-path").and_then(|k| args.get(k + 1)) {
                data.push(m.into());
            }
        }
    }
    let config = i.dir.join("config.toml");
    remove_any(&config).map_err(|e| e.to_string())?;
    std::fs::write(&config, format!("# Copied from {} by `service install`.\n{t}", i.config.display()))
        .map_err(|e| format!("{}: {e}", config.display()))?;
    // The settings hold the database's password: administrators and the
    // service only. So do the programs (copied from someone's folders).
    let own = |p: &Path, inherit: &str| -> Result<(), String> {
        icacls(p, &[
            "/inheritance:r",
            "/grant:r",
            &format!("*S-1-5-32-544:{inherit}F"),
            &format!("*S-1-5-18:{inherit}F"),
            &format!("{WINDOWS_ACCOUNT}:{inherit}RX"),
        ])
    };
    own(&config, "")?;
    own(&parts, "(OI)(CI)")?;
    Ok((config, data))
}

/// Whether `path` is inside the service's folder (which only
/// administrators can change).
#[cfg(windows)]
fn inside(dir: &Path, path: &Path) -> bool {
    match (std::fs::canonicalize(dir), std::fs::canonicalize(path)) {
        (Ok(d), Ok(p)) => p.starts_with(d),
        _ => false,
    }
}

/// Run as a Windows service: the settings and every program it starts
/// must be in the service's folder (as `service install` puts them).
#[cfg(windows)]
fn check_protected(plan: &Plan) -> Result<(), String> {
    let config = plan.args.iter().position(|a| a == "--config").and_then(|k| plan.args.get(k + 1)).map(PathBuf::from);
    let programs = config
        .iter()
        .cloned()
        .chain(plan.postgres.iter().map(|p| p.program.clone()))
        .chain(plan.also.iter().map(|a| a.program.clone()));
    for p in programs {
        if !inside(&plan.dir, &p) {
            return Err(format!(
                "{} is outside {}, where people without administrator rights may be able to change it. Turn starting the server with the computer off and on again in Xenify Hub (or run `service install` again).",
                p.display(),
                plan.dir.display()
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn install(i: &Install) -> Result<(), String> {
    let program = place_program(&i.dir)?;
    let (config, data) = protect(i)?;
    let bin = format!(
        "\"{}\" --config \"{}\" service run --dir \"{}\" --windows-service",
        program.display(),
        config.display(),
        i.dir.display()
    );
    let _ = uninstall();
    run(std::process::Command::new("sc.exe").args([
        "create",
        WINDOWS_NAME,
        "start=",
        "auto",
        "DisplayName=",
        "Xenify server",
        "binPath=",
        &bin,
    ]))?;
    // Its own account, not the system's.
    let as_itself = run(std::process::Command::new("sc.exe").args(["config", WINDOWS_NAME, "obj=", WINDOWS_ACCOUNT]))
        .and_then(|_| {
            // It updates itself into versions/, and uses its data folders.
            let versions = i.dir.join("versions");
            std::fs::create_dir_all(&versions).map_err(|e| e.to_string())?;
            let grant = format!("{WINDOWS_ACCOUNT}:(OI)(CI)M");
            icacls(&versions, &["/grant", &grant])?;
            for d in &data {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                icacls(d, &["/grant", &grant])?;
            }
            Ok(())
        });
    if let Err(e) = as_itself {
        let _ = uninstall();
        return Err(e);
    }
    let _ = run(std::process::Command::new("sc.exe").args([
        "description",
        WINDOWS_NAME,
        "Shared files, editing together, chat and AI for Xenify.",
    ]));
    // Started again if it stops.
    let _ = run(std::process::Command::new("sc.exe").args([
        "failure",
        WINDOWS_NAME,
        "reset=",
        "86400",
        "actions=",
        "restart/5000/restart/5000/restart/60000",
    ]));
    run(std::process::Command::new("sc.exe").args(["start", WINDOWS_NAME]))
}

#[cfg(windows)]
pub fn uninstall() -> Result<(), String> {
    let _ = run(std::process::Command::new("sc.exe").args(["stop", WINDOWS_NAME]));
    run(std::process::Command::new("sc.exe").args(["delete", WINDOWS_NAME]))
}

#[cfg(windows)]
pub fn installed() -> bool {
    std::process::Command::new("sc.exe").args(["query", WINDOWS_NAME]).output().is_ok_and(|o| o.status.success())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn the_unit_runs_the_supervisor_as_the_user() {
        let i = super::Install {
            dir: "/opt/xenify server".into(),
            config: "/etc/xenify-server/config.toml".into(),
            user: Some("xenify".into()),
        };
        let t = super::unit_text(std::path::Path::new("/opt/xenify server/xenify-server"), &i);
        assert!(t.contains("User=xenify\n"));
        assert!(t.contains("ExecStart=\"/opt/xenify server/xenify-server\" --config \"/etc/xenify-server/config.toml\" service run --dir \"/opt/xenify server\"\n"));
        assert!(t.contains("AmbientCapabilities=CAP_NET_BIND_SERVICE"));
        assert!(t.contains("KillMode=mixed"));
    }

    #[test]
    fn the_service_never_runs_as_root_or_nobody() {
        let i = |u: Option<&str>| super::Install { dir: "/opt/x".into(), config: "/etc/x.toml".into(), user: u.map(String::from) };
        assert!(super::service_user(&i(None)).is_err(), "no user: refused, not root");
        assert!(super::service_user(&i(Some(""))).is_err());
        assert!(super::service_user(&i(Some("root"))).is_err());
        assert!(super::service_user(&i(Some("me\nExecStartPre=+/bin/sh"))).is_err(), "nothing that ends the line");
        assert!(super::service_user(&i(Some("-R"))).is_err());
        assert_eq!(super::service_user(&i(Some("anna.b"))), Ok("anna.b"));
        assert!(super::plain_path(std::path::Path::new("/home/a\"/x")).is_err());
        assert!(super::plain_path(std::path::Path::new("/opt/xenify server")).is_ok());
    }

    #[test]
    fn copying_the_admin_pages_refuses_links() {
        let base = std::env::temp_dir().join(format!("xenify-service-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dir = Dir(base);
        let web = dir.path().join("web");
        std::fs::create_dir_all(web.join("css")).unwrap();
        std::fs::write(web.join("index.html"), "<p>hi").unwrap();
        std::fs::write(web.join("css/a.css"), "p{}").unwrap();
        let out = dir.path().join("out");
        super::copy_dir(&web, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("css/a.css")).unwrap(), "p{}");
        // A link to a file elsewhere (say /etc/shadow) is never followed.
        let secret = dir.path().join("secret");
        std::fs::write(&secret, "s").unwrap();
        std::os::unix::fs::symlink(&secret, web.join("leak.txt")).unwrap();
        let out2 = dir.path().join("out2");
        assert!(super::copy_dir(&web, &out2).is_err());
        assert!(!out2.join("leak.txt").exists());
        // Nor a link where a file is written.
        std::fs::remove_file(web.join("leak.txt")).unwrap();
        std::os::unix::fs::symlink(&secret, out.join("index.html.new")).unwrap();
        super::copy_file(&web.join("index.html"), &out.join("index.html.new")).unwrap();
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "s", "the link was replaced, not written through");
    }

    /// A folder removed afterwards.
    struct Dir(std::path::PathBuf);
    impl Dir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
