//! `xenify-server`: run the server.

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use xenify_server::{Config, Server, atrest, service};

#[derive(Parser)]
#[command(
    name = "xenify-server",
    version = xenify_server::VERSION,
    about = "The Xenify server: shared files, editing together, chat and AI for your home or office"
)]
struct Cli {
    /// A settings file with `database` and `data` (default: the one the
    /// installer wrote, e.g. /etc/xenify-server/config.toml, when it's there).
    #[arg(long, env = "XENIFY_CONFIG", global = true)]
    config: Option<PathBuf>,
    /// The PostgreSQL database (default: postgres://xenify@localhost/xenify).
    #[arg(long, env = "XENIFY_DATABASE", global = true)]
    database: Option<String>,
    /// Where files are kept (default: the user's data folder, xenify-server).
    #[arg(long, env = "XENIFY_DATA", global = true)]
    data: Option<PathBuf>,
    /// The admin pages' folder (default: `web` beside the program).
    #[arg(long, env = "XENIFY_WEB", global = true)]
    web: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

/// Everything else is set on the admin pages (Network); these flags, when
/// given, win over it.
#[derive(Subcommand)]
enum Command {
    /// Run the server (the default).
    Serve(Serve),
    /// Print the first-run setup link, while the server isn't set up.
    SetupLink,
    /// Someone (an admin, usually) can't sign in: prints a link to pick a
    /// new password, and signs them out everywhere.
    ResetPassword {
        /// Their user name or email address.
        user: String,
    },
    /// The server as a system service, started with the computer.
    #[command(subcommand)]
    Service(Service),
    /// The server's own database.
    #[command(subcommand)]
    Database(Database),
    /// Gives the encrypted folder its key after a start (encryption at
    /// rest): asks for the passphrase, or reads it from a file.
    Unlock {
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Encryption at rest: the database and people's files in a folder the
    /// computer encrypts (Linux, ext4).
    #[command(subcommand)]
    AtRest(AtRestCommand),
    /// Backups (set up on Admin › Backups): back up now, list them, or put
    /// a whole server back from one.
    #[command(subcommand)]
    Backup(BackupCommand),
    /// Change settings before the server runs (as on Admin › Network).
    Configure {
        #[arg(long)]
        http_port: Option<u16>,
        /// A port, or "off".
        #[arg(long)]
        https_port: Option<String>,
        /// The server's domain name, for a certificate from Let's Encrypt
        /// ("" for none).
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        email: Option<String>,
        /// Who can open the admin pages: local, lan or anywhere (over HTTPS).
        #[arg(long)]
        admin_access: Option<String>,
    },
}

/// Where the backups are, when this server's database doesn't say (a new
/// computer): a folder, s3:https://host/bucket[/folder] or
/// sftp:user@host[:port]/folder. The passphrase comes from
/// XENIFY_BACKUP_PASSPHRASE or is asked for; S3's keys from
/// AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY; an SFTP password from
/// XENIFY_SFTP_PASSWORD.
#[derive(clap::Args, Clone, Default)]
struct BackupFrom {
    #[arg(long)]
    from: Option<String>,
    /// For SFTP: the private key to sign in with.
    #[arg(long)]
    ssh_key: Option<PathBuf>,
}

#[derive(Subcommand)]
enum BackupCommand {
    /// Back up now, as set up on Admin › Backups.
    Now,
    /// The backups there, newest first.
    List {
        #[command(flatten)]
        from: BackupFrom,
    },
    /// Put the whole server back as it was: the database and files. Stop the
    /// server first.
    Restore {
        /// A backup's id from `backup list`, or "latest".
        #[arg(default_value = "latest")]
        snapshot: String,
        #[command(flatten)]
        from: BackupFrom,
        /// Don't ask before replacing the database.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum AtRestCommand {
    /// Makes the encrypted folder (new and empty) and prints the settings
    /// file's lines. With --move, also moves the database and files in
    /// (stop the service first).
    Setup {
        #[arg(long)]
        dir: PathBuf,
        /// Make a passphrase and print it, instead of asking for one.
        #[arg(long)]
        generate: bool,
        #[arg(long = "move")]
        move_in: bool,
    },
    /// Prints "unlocked" (exit 0), "locked" (exit 3) or "off" (exit 4).
    Status,
}

#[derive(Subcommand)]
enum Database {
    /// Set up a private PostgreSQL just for this server (the portable build
    /// from Xenify's signed release list; nothing installed on the system).
    /// Prints the lines for the settings file. Not as root.
    Setup {
        /// Its folder (it gets `versions/` and `data/`).
        #[arg(long)]
        dir: PathBuf,
        /// Another release list (tests, builds of your own).
        #[arg(long, hide = true)]
        source: Option<String>,
        /// A file of more keys to trust (JSON arrays of 32 bytes).
        #[arg(long, hide = true)]
        trusted_keys: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum Service {
    /// Run the server and keep it running (what the system starts).
    Run {
        /// The service's folder (its program and versions).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Started by Windows' service manager.
        #[arg(long, hide = true)]
        windows_service: bool,
    },
    /// Start the server with the computer (as root / administrator).
    Install {
        /// The service's folder (default: /opt/xenify-server, or Program
        /// Files\Xenify\Server).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Who it runs as (Linux and Mac; default: who runs this, via sudo).
        #[arg(long)]
        user: Option<String>,
    },
    /// Stop starting it with the computer.
    Uninstall,
    /// Prints whether it starts with the computer ("installed", exit 0) or
    /// not ("not installed", exit 3).
    Status,
}

#[derive(clap::Args, Default)]
struct Serve {
    /// Where to listen (default 0.0.0.0:7700; Admin › Network).
    #[arg(long, env = "XENIFY_LISTEN")]
    listen: Option<String>,
    /// Let chat apps' webhooks reach this computer or its network.
    #[arg(long)]
    allow_local_webhooks: bool,
    /// Where to listen for HTTPS, or "off", e.g. behind a reverse proxy
    /// (default 0.0.0.0:7443; Admin › Network).
    #[arg(long, env = "XENIFY_HTTPS")]
    https: Option<String>,
    /// Write and Draw for the browser to serve (default: `apps` beside the
    /// program; scripts/build-site.sh's target/site).
    #[arg(long, env = "XENIFY_APPS")]
    apps: Option<PathBuf>,
    /// Don't announce the server on the network.
    #[arg(long)]
    hidden: bool,
    /// The server's domain name (repeat for more): HTTPS then uses a
    /// certificate from Let's Encrypt (Admin › Network › Domain name).
    #[arg(long = "domain", env = "XENIFY_DOMAIN", value_delimiter = ',')]
    domains: Vec<String>,
    /// Where Let's Encrypt may write about the certificate (expiry).
    #[arg(long, env = "XENIFY_EMAIL")]
    email: Option<String>,
    /// Try Let's Encrypt's staging service first (certificates browsers
    /// don't trust, without its weekly limits).
    #[arg(long)]
    acme_staging: bool,
}

/// The settings file: what the server needs before it can read the rest
/// from its database. Keys it doesn't know are warned about, not refused
/// (see `unknown_keys`): after going back to an older version, a key a
/// newer one added mustn't stop the server from starting.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct File {
    database: Option<String>,
    data: Option<PathBuf>,
    web: Option<PathBuf>,
    apps: Option<PathBuf>,
    /// The server's own PostgreSQL, started by the service first.
    postgres: Option<service::Postgres>,
    /// Programs the service keeps running beside it (the AI runner).
    also: Vec<service::Also>,
    /// Encryption at rest: the encrypted folder (and a key file, if any).
    at_rest: Option<atrest::AtRest>,
    /// Like --listen and --https (fixed on Admin › Network), e.g.
    /// "127.0.0.1:7700" on a VPS: plain HTTP from this computer only (an
    /// SSH tunnel), everyone else over HTTPS.
    listen: Option<String>,
    https: Option<String>,
}

/// Where the installer puts the settings file.
fn default_config() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| r"C:\ProgramData".into())
            .join(r"Xenify\server\config.toml")
    } else {
        "/etc/xenify-server/config.toml".into()
    }
}

/// The settings file's keys this version doesn't read ("at_rest.colour"…).
fn unknown_keys(text: &str) -> Vec<String> {
    const TOP: [&str; 9] = ["database", "data", "web", "apps", "postgres", "also", "at_rest", "listen", "https"];
    const AT_REST: [&str; 2] = ["dir", "key_file"];
    let Ok(t) = text.parse::<toml::Table>() else { return Vec::new() };
    let mut out: Vec<String> = t.keys().filter(|k| !TOP.contains(&k.as_str())).cloned().collect();
    if let Some(a) = t.get("at_rest").and_then(|v| v.as_table()) {
        out.extend(a.keys().filter(|k| !AT_REST.contains(&k.as_str())).map(|k| format!("at_rest.{k}")));
    }
    out
}

fn read_config(path: Option<&Path>) -> File {
    let (path, must) = match path {
        Some(p) => (p.to_path_buf(), true),
        None => (default_config(), false),
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            for key in unknown_keys(&text) {
                eprintln!(
                    "xenify-server: {}: “{key}” isn't a setting this version knows (a typo, or from a newer version): ignored",
                    path.display()
                );
            }
            toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("xenify-server: {}: {e}", path.display());
                std::process::exit(2);
            })
        }
        Err(e) if must => {
            eprintln!("xenify-server: {}: {e}", path.display());
            std::process::exit(2);
        }
        Err(_) => File::default(),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Some(Command::Service(s)) = &cli.command {
        return service_command(s, cli.config.clone());
    }
    match &cli.command {
        Some(Command::Unlock { key_file }) => return unlock_command(cli.config.as_deref(), key_file.as_deref()),
        Some(Command::AtRest(c)) => return at_rest_command(cli.config.as_deref(), c),
        _ => {}
    }
    if let Some(Command::Database(Database::Setup { dir, source, trusted_keys })) = &cli.command {
        return database_setup(dir, source.as_deref(), trusted_keys.as_deref());
    }
    tokio::runtime::Runtime::new().expect("runtime").block_on(serve(cli));
}

/// Starts the server's own PostgreSQL if it isn't running yet, for a
/// command that needs the database; the caller stops it after.
fn own_postgres(file: &File) -> Option<xenify_hub_core::pg::Pg> {
    let pg = file.postgres.as_ref().and_then(|p| xenify_hub_core::pg::Pg::new(&p.program, p.data.clone()))?;
    if pg.running() {
        return None;
    }
    if let Err(e) = pg.start() {
        eprintln!("xenify-server: {e}");
        std::process::exit(1);
    }
    Some(pg)
}

async fn serve(cli: Cli) {
    let file = read_config(cli.config.as_deref());
    let data = cli.data.or(file.data.clone()).unwrap_or_else(default_data);
    let serve = match cli.command {
        None => Serve::default(),
        Some(Command::Serve(s)) => s,
        Some(Command::SetupLink) => return setup_link(&data),
        Some(Command::Service(_) | Command::Database(_) | Command::Unlock { .. } | Command::AtRest(_)) => {
            unreachable!()
        }
        Some(Command::Backup(c)) => {
            let database =
                cli.database.or(file.database.clone()).unwrap_or_else(|| "postgres://xenify@localhost/xenify".into());
            if let Some(at) = &file.at_rest
                && !atrest::unlocked(at)
            {
                eprintln!("xenify-server: the encrypted folder is locked: run `xenify-server unlock` first");
                std::process::exit(1);
            }
            let paths = xenify_server::backup::Paths {
                database,
                data: data.clone(),
                secure: file.at_rest.as_ref().map(|a| a.dir.clone()).unwrap_or(data),
                postgres: file.postgres.as_ref().map(|p| p.program.clone()),
            };
            let started = own_postgres(&file);
            let done = backup_command(c, paths).await;
            if let Some(pg) = started {
                pg.stop();
            }
            if let Err(e) = done {
                eprintln!("xenify-server: {e}");
                std::process::exit(1);
            }
            return;
        }
        Some(Command::ResetPassword { user }) => {
            let database =
                cli.database.or(file.database.clone()).unwrap_or_else(|| "postgres://xenify@localhost/xenify".into());
            let started = own_postgres(&file);
            let done = xenify_server::reset_password_code(&database, &user).await;
            if let Some(pg) = started {
                pg.stop();
            }
            match done {
                Ok(code) => println!(
                    "{code}\n\nSign in as {user} and type this code when asked, then pick a new password. It works once, for 14 days."
                ),
                Err(e) => {
                    eprintln!("xenify-server: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some(Command::Configure { http_port, https_port, domain, email, admin_access }) => {
            let database =
                cli.database.or(file.database.clone()).unwrap_or_else(|| "postgres://xenify@localhost/xenify".into());
            let https_port = match https_port.as_deref() {
                None => None,
                Some("off") => Some(None),
                Some(p) => match p.parse() {
                    Ok(p) => Some(Some(p)),
                    Err(_) => {
                        eprintln!("xenify-server: --https-port is a port or \"off\"");
                        std::process::exit(2);
                    }
                },
            };
            let c = xenify_server::Configure { http_port, https_port, domain, email, admin_access };
            let started = own_postgres(&file);
            let done = xenify_server::configure(&database, c).await;
            if let Some(pg) = started {
                pg.stop();
            }
            if let Err(e) = done {
                eprintln!("xenify-server: {e}");
                std::process::exit(1);
            }
            return;
        }
    };
    let database = cli.database.or(file.database).unwrap_or_else(|| "postgres://xenify@localhost/xenify".into());
    let web = cli.web.or(file.web).or_else(|| beside("web", "index.html"));
    // Encryption at rest: wait for the key, then start the database in it.
    if let Some(at) = &file.at_rest {
        if let Some(w) = atrest::mount_warning(&at.dir) {
            eprintln!("xenify-server: warning: {w}");
        }
        if !atrest::unlocked(at) {
            let listen = serve.listen.clone().or(file.listen.clone());
            match atrest::unlock_with_file(at) {
                Ok(true) => println!("xenify-server: unlocked with the key file"),
                Ok(false) => wait_locked(at, listen).await,
                Err(e) => {
                    eprintln!("xenify-server: the key file didn't unlock it: {e}");
                    wait_locked(at, listen).await
                }
            }
        }
        if let Some(pg) = file.postgres.as_ref().and_then(|p| xenify_hub_core::pg::Pg::new(&p.program, p.data.clone()))
            && !pg.running()
            && let Err(e) = pg.start()
        {
            eprintln!("xenify-server: can't start the database: {e}");
            std::process::exit(1);
        }
    }
    let secure = file.at_rest.as_ref().map(|a| a.dir.clone());
    let postgres = file.postgres.as_ref().map(|p| p.program.clone());
    let config = Config { database, data, local_webhooks: serve.allow_local_webhooks, web, secure, postgres };
    let server = match Server::start(config).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("xenify-server: can't start: {e}");
            std::process::exit(1);
        }
    };
    let apps_fixed = serve.apps.is_some();
    // The apps for the browser: given, beside the program, downloaded
    // before, or (none) downloaded now.
    let mut downloaded_apps = false;
    let mut get_apps = false;
    if let Some(dir) = serve.apps.or(file.apps).or_else(|| beside("apps", "write")) {
        server.host_apps(dir);
    } else if let Some(dir) = xenify_server::hub::downloaded_web_apps(&server) {
        server.host_apps(dir);
        downloaded_apps = true;
    } else {
        get_apps = true;
    }
    server.fix_network(xenify_server::net::Fixed {
        listen: serve.listen.or(file.listen),
        https: serve.https.or(file.https),
        hidden: serve.hidden,
        apps: apps_fixed,
    });
    if !serve.domains.is_empty() {
        server.fix_domains(serve.domains.clone(), serve.email.clone(), !serve.acme_staging);
        println!(
            "HTTPS for {} (certificate from Let's Encrypt{})",
            serve.domains.join(", "),
            if serve.acme_staging { ", staging" } else { "" }
        );
    } else {
        server.use_saved_domain().await;
    }
    let net = server.net().await;
    if let Err(e) = server.run_network(net.clone()).await {
        eprintln!("xenify-server: {e}");
        std::process::exit(1);
    }
    // A newer server fetches its own version of them.
    if downloaded_apps {
        xenify_server::hub::catch_up_web_apps(&server);
    }
    if get_apps {
        xenify_server::hub::get_web_apps(&server);
    }
    if let Some(token) = server.setup_token() {
        println!("Set the server up at http://localhost:{}/admin/#/setup/{token}", net.http_port);
        println!("(from another computer, use this computer's address instead of localhost)");
    }
    // Ctrl-C, or (from the Hub or systemd, during an update) SIGTERM: stop
    // taking new connections, finish what's going on, then go.
    // Under the service, also when its input closes (a newer version took over).
    if std::env::var_os(service::VERSIONS).is_some() {
        xenify_server::hub::update_myself(&server, std::env::var_os(service::VERSIONS).map(PathBuf::from).unwrap());
    }
    let s2 = server.clone();
    let stop = async move {
        let supervised = async {
            if std::env::var_os(service::SUPERVISED).is_none() {
                return std::future::pending().await;
            }
            use tokio::io::AsyncReadExt;
            let mut input = tokio::io::stdin();
            let mut buf = [0u8; 64];
            while input.read(&mut buf).await.is_ok_and(|n| n > 0) {}
        };
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signals");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
                _ = supervised => {}
            }
        }
        #[cfg(not(unix))]
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = supervised => {}
        }
        println!("xenify-server: stopping (finishing what's going on)");
        s2.stop();
        // Chat connections and slow downloads don't hold it up for long.
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            std::process::exit(0);
        });
    };
    stop.await;
    // The listeners finish what they have (at most 15 s, above).
    server.drained().await;
}

/// Locked (encryption at rest): answers only that it's locked, until the
/// folder's key is given (`xenify-server unlock`), then returns.
async fn wait_locked(at: &atrest::AtRest, listen: Option<String>) {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let addr = listen.unwrap_or_else(|| "0.0.0.0:7700".into());
    let info = xenify_server_proto::Info {
        name: String::new(),
        version: xenify_server::VERSION.into(),
        protocol: xenify_server_proto::VERSION,
        oldest_protocol: Some(xenify_server_proto::OLDEST),
        server_id: String::new(),
        setup_needed: false,
        features: vec!["locked".into()],
    };
    let app = axum::Router::new()
        .route(
            "/v1/info",
            axum::routing::get(move || {
                let i = info.clone();
                async move { axum::Json(i) }
            }),
        )
        .route(
            "/v1/{*rest}",
            axum::routing::any(|| async {
                let body = serde_json::json!({
                    "error": "this server is locked: an admin unlocks it on the server's computer",
                    "code": "locked",
                });
                (StatusCode::SERVICE_UNAVAILABLE, axum::Json(body)).into_response()
            }),
        )
        .fallback(|| async { axum::response::Html(LOCKED_PAGE) });
    let listener = match xenify_server::listen(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("xenify-server: {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("xenify-server: locked until its key is given (xenify-server unlock); listening on {addr}");
    let dir = at.clone();
    let unlocked = async move {
        while !atrest::unlocked(&dir) {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    };
    let _ = axum::serve(listener, app).with_graceful_shutdown(unlocked).await;
    println!("xenify-server: unlocked");
}

/// What a browser sees while the server is locked.
const LOCKED_PAGE: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Locked</title><body style="font-family:system-ui,sans-serif;background:#efece8;color:#1c1917;margin:0">
<main style="max-width:460px;margin:64px auto;background:#fff;border-radius:16px;padding:32px">
<h1 style="margin:0 0 12px;font-size:23px">This server is locked</h1>
<p style="color:#44403c;line-height:1.55">Its files are encrypted on its disk and stay locked after it starts, until an admin gives the key. On the server's computer, run:</p>
<pre style="background:#f5f3f0;border-radius:12px;padding:14px">xenify-server unlock</pre>
<p style="color:#57534e;font-size:14px">This page reloads by itself once it's unlocked.</p></main>
<script>setInterval(() => fetch("/v1/info").then((r) => r.json()).then((i) => { if (!i.features.includes("locked")) location.reload(); }).catch(() => {}), 3000)</script>"#;

/// Asks for something secret on the terminal without showing it.
/// The backup settings: this server's (its database), or from --from and
/// the environment. Returns them with this server's SFTP key.
async fn backup_place(paths: &xenify_server::backup::Paths, from: &BackupFrom) -> Result<(xenify_server::backup::Settings, String), String> {
    use xenify_server::backup;
    let saved = backup::saved(&paths.database).await;
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (mut set, mut key) = match (&from.from, saved) {
        (Some(f), saved) => {
            let mut s = backup::place_from(f)?;
            if let Some((old, _)) = &saved
                && old.place() == s.place()
            {
                s = old.clone();
            }
            (s, saved.map(|x| x.1).unwrap_or_default())
        }
        (None, Some(x)) if !x.0.place().is_empty() => x,
        (None, _) => return Err("this server has no backups set up: say where they are with --from".into()),
    };
    if let Some(k) = &from.ssh_key {
        key = std::fs::read_to_string(k).map_err(|e| format!("{}: {e}", k.display()))?;
    }
    if set.kind == "sftp" && key.is_empty() && set.sftp_password.is_empty() {
        match env("XENIFY_SFTP_PASSWORD") {
            Some(p) => {
                set.sftp_auth = "password".into();
                set.sftp_password = p;
            }
            None => return Err("SFTP needs --ssh-key or XENIFY_SFTP_PASSWORD".into()),
        }
    }
    if set.kind == "s3" {
        if let Some(k) = env("AWS_ACCESS_KEY_ID") {
            set.s3_key = k;
        }
        if let Some(k) = env("AWS_SECRET_ACCESS_KEY") {
            set.s3_secret = k;
        }
    }
    if set.passphrase.is_empty() {
        set.passphrase = env("XENIFY_BACKUP_PASSPHRASE").unwrap_or_else(|| ask_secret("The backups' passphrase (Enter for none): "));
        set.encrypt = !set.passphrase.is_empty();
    }
    Ok((set, key))
}

async fn backup_command(c: BackupCommand, paths: xenify_server::backup::Paths) -> Result<(), String> {
    use xenify_server::backup;
    // Off the async runtime: SFTP runs one of its own.
    async fn off<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.await.unwrap()
    }
    match c {
        BackupCommand::Now => {
            let (set, key) = backup_place(&paths, &BackupFrom::default()).await?;
            let d = off(move || backup::back_up(&set, &key, &paths, false, true, &|m| eprintln!("{m}…"))).await?;
            println!("Backed up ({} MB new). {} backups there.", d.added / 1_000_000, d.snapshots.len());
        }
        BackupCommand::List { from } => {
            let (set, key) = backup_place(&paths, &from).await?;
            let list = off(move || backup::snapshots(&set, &key)).await?;
            if list.is_empty() {
                println!("No backups there.");
            }
            for (id, t, size) in list {
                let when = chrono::DateTime::parse_from_rfc3339(&t)
                    .ok()
                    .map(|t| t.with_timezone(&chrono::Local).format("%a %e %b %Y %H:%M").to_string());
                println!("{}  {}  {:>8} MB", &id[..8], when.unwrap_or(t), size / 1_000_000);
            }
        }
        BackupCommand::Restore { snapshot, from, yes } => {
            if backup::database_in_use(&paths.database).await {
                return Err("the server is running (its database is in use): stop it first".into());
            }
            let (set, key) = backup_place(&paths, &from).await?;
            if !yes {
                use std::io::BufRead;
                eprint!(
                    "This replaces the database and puts back the files in {} from backup {snapshot}. Type yes to go on: ",
                    paths.data.display()
                );
                let mut line = String::new();
                let _ = std::io::stdin().lock().read_line(&mut line);
                if line.trim() != "yes" {
                    return Err("nothing changed".into());
                }
            }
            off(move || backup::restore(&set, &key, &snapshot, &paths, &|m| eprintln!("{m}…"))).await?;
            println!("Done. Start the server again.");
        }
    }
    Ok(())
}

fn ask_secret(prompt: &str) -> String {
    use std::io::Write;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    #[cfg(unix)]
    let old = {
        use std::os::fd::AsRawFd;
        let fd = std::io::stdin().as_raw_fd();
        // SAFETY: plain termios calls on stdin.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            (libc::tcgetattr(fd, &mut t) == 0).then(|| {
                let old = t;
                t.c_lflag &= !libc::ECHO;
                libc::tcsetattr(fd, libc::TCSANOW, &t);
                (fd, old)
            })
        }
    };
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    #[cfg(unix)]
    if let Some((fd, t)) = old {
        // SAFETY: puts back what tcgetattr gave.
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) };
        eprintln!();
    }
    line.trim_end_matches(['\r', '\n']).to_string()
}

fn at_rest_of(config: Option<&Path>) -> atrest::AtRest {
    match read_config(config).at_rest {
        Some(a) => a,
        None => {
            eprintln!("xenify-server: encryption at rest isn't set up (no [at_rest] in the settings file)");
            std::process::exit(4);
        }
    }
}

fn unlock_command(config: Option<&Path>, key_file: Option<&Path>) {
    let at = at_rest_of(config);
    if atrest::unlocked(&at) {
        println!("Already unlocked.");
        return;
    }
    let pass = match key_file {
        Some(f) => std::fs::read_to_string(f).unwrap_or_else(|e| {
            eprintln!("xenify-server: {}: {e}", f.display());
            std::process::exit(1);
        }),
        None => ask_secret("Passphrase: "),
    };
    match atrest::unlock(&at, pass.trim()) {
        Ok(()) => println!("Unlocked. The server starts now."),
        Err(e) => {
            eprintln!("xenify-server: {e}");
            std::process::exit(1);
        }
    }
}

fn at_rest_command(config: Option<&Path>, c: &AtRestCommand) {
    match c {
        AtRestCommand::Status => match read_config(config).at_rest {
            None => {
                println!("off");
                std::process::exit(4);
            }
            Some(at) if atrest::unlocked(&at) => println!("unlocked"),
            Some(_) => {
                println!("locked");
                std::process::exit(3);
            }
        },
        AtRestCommand::Setup { dir, generate, move_in } => {
            let pass = if *generate {
                atrest::new_passphrase()
            } else {
                let a = ask_secret("New passphrase (at least 12 characters): ");
                if a.chars().count() < 12 {
                    eprintln!("xenify-server: use at least 12 characters");
                    std::process::exit(2);
                }
                if ask_secret("Again: ") != a {
                    eprintln!("xenify-server: the two aren't the same");
                    std::process::exit(2);
                }
                a
            };
            if let Err(e) = atrest::setup(dir, &pass) {
                eprintln!("xenify-server: {e}");
                std::process::exit(1);
            }
            if *generate {
                println!(
                    "Passphrase (write it down: without it the files can't be read, not even by us):\n\n    {pass}\n"
                );
            }
            let file = read_config(config);
            let q = |p: &Path| serde_json::to_string(&p.display().to_string()).expect("a string");
            let mut moved_pg = None;
            if *move_in {
                if let Some(pg) =
                    file.postgres.as_ref().and_then(|p| xenify_hub_core::pg::Pg::new(&p.program, p.data.clone()))
                    && pg.running()
                {
                    eprintln!("xenify-server: its database is running: stop the service first");
                    std::process::exit(1);
                }
                let data = file.data.clone().unwrap_or_else(default_data);
                let mut moves: Vec<(PathBuf, PathBuf)> = ["blobs", "chat-files"]
                    .iter()
                    .map(|d| (data.join(d), dir.join(d)))
                    .filter(|(f, _)| f.exists())
                    .collect();
                if let Some(p) = &file.postgres {
                    moves.push((p.data.clone(), dir.join("postgres")));
                    moved_pg = Some(dir.join("postgres"));
                }
                for (from, to) in moves {
                    // Copied in (encrypted on the way), then the plain copy goes.
                    let ok = std::process::Command::new("cp")
                        .arg("-a")
                        .arg(&from)
                        .arg(&to)
                        .status()
                        .is_ok_and(|s| s.success());
                    if !ok || std::fs::remove_dir_all(&from).is_err() {
                        eprintln!("xenify-server: couldn't move {} into {}", from.display(), to.display());
                        std::process::exit(1);
                    }
                    println!("Moved {} into the encrypted folder.", from.display());
                }
            }
            println!(
                "Put this in the settings file{}:\n",
                config.map(|c| format!(" ({})", c.display())).unwrap_or_default()
            );
            println!("[at_rest]\ndir = {}", q(dir));
            if let Some(p) = moved_pg {
                println!("\nand in [postgres]:\ndata = {}", q(&p));
            }
        }
    }
}

fn database_setup(dir: &Path, source: Option<&str>, keys: Option<&Path>) {
    let keys: Vec<[u8; 32]> = keys
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    match xenify_server::database::setup(dir, source.unwrap_or(xenify_hub_core::RELEASES), &keys) {
        Ok(m) => {
            let q = |s: &str| serde_json::to_string(s).expect("a string");
            println!("database = {}", q(&m.url));
            println!("[postgres]");
            println!("program = {}", q(&m.program.display().to_string()));
            println!("data = {}", q(&m.data.display().to_string()));
        }
        Err(e) => {
            eprintln!("xenify-server: {e}");
            std::process::exit(1);
        }
    }
}

/// The service's folder when none is given.
fn default_service_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| r"C:\Program Files".into())
            .join(r"Xenify\Server")
    } else if cfg!(target_os = "macos") {
        "/Library/Application Support/Xenify/Server".into()
    } else {
        "/opt/xenify-server".into()
    }
}

fn service_command(s: &Service, config: Option<PathBuf>) {
    let config = config.unwrap_or_else(default_config);
    let done = match s {
        Service::Run { dir, windows_service } => {
            let file = read_config(Some(&config));
            let plan = service::Plan {
                dir: dir.clone().unwrap_or_else(default_service_dir),
                args: vec!["--config".into(), config.display().to_string()],
                postgres: file.postgres,
                also: file.also,
                at_rest: file.at_rest,
            };
            if *windows_service {
                windows::run(plan)
            } else {
                tokio::runtime::Runtime::new().expect("runtime").block_on(service::supervise(plan, stop_signal()));
                Ok(())
            }
        }
        Service::Install { dir, user } => {
            if !config.is_file() {
                eprintln!("xenify-server: write the settings file {} first (database, data)", config.display());
                std::process::exit(2);
            }
            let user = user.clone().or_else(|| std::env::var("SUDO_USER").ok()).filter(|_| !cfg!(windows));
            service::install(&service::Install { dir: dir.clone().unwrap_or_else(default_service_dir), config, user })
        }
        Service::Uninstall => service::uninstall(),
        Service::Status => {
            if service::installed() {
                println!("installed");
                Ok(())
            } else {
                println!("not installed");
                std::process::exit(3);
            }
        }
    };
    if let Err(e) = done {
        eprintln!("xenify-server: {e}");
        std::process::exit(1);
    }
}

/// Ctrl-C, or the system stopping the service.
async fn stop_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signals");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(not(windows))]
mod windows {
    pub fn run(_: xenify_server::service::Plan) -> Result<(), String> {
        Err("--windows-service is for Windows".into())
    }
}

/// Windows' service manager starts `service run --windows-service`: it
/// must be told the service is running, and asks it to stop.
#[cfg(windows)]
mod windows {
    use std::sync::Mutex;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use xenify_server::service::{Plan, WINDOWS_NAME};

    static PLAN: Mutex<Option<Plan>> = Mutex::new(None);

    windows_service::define_windows_service!(ffi_main, service_main);

    pub fn run(plan: Plan) -> Result<(), String> {
        *PLAN.lock().unwrap() = Some(plan);
        windows_service::service_dispatcher::start(WINDOWS_NAME, ffi_main).map_err(|e| e.to_string())
    }

    fn status(state: ServiceState) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::from_secs(45),
            process_id: None,
        }
    }

    fn service_main(_: Vec<std::ffi::OsString>) {
        let Some(plan) = PLAN.lock().unwrap().take() else { return };
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = Mutex::new(Some(tx));
        let Ok(handle) = service_control_handler::register(WINDOWS_NAME, move |c| match c {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }) else {
            return;
        };
        let _ = handle.set_service_status(status(ServiceState::Running));
        tokio::runtime::Runtime::new().expect("runtime").block_on(xenify_server::service::supervise(plan, async {
            let _ = rx.await;
        }));
        let _ = handle.set_service_status(status(ServiceState::Stopped));
    }
}

/// `xenify-server setup-link`: from what the running server wrote.
fn setup_link(data: &Path) {
    let Ok(text) = std::fs::read_to_string(data.join("address.json")) else {
        eprintln!("The server hasn't run with the data folder {} yet.", data.display());
        std::process::exit(1);
    };
    let a: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let port = a["http_port"].as_u64().unwrap_or(7700);
    match a["setup"].as_str() {
        Some(path) => {
            println!("http://localhost:{port}{path}");
            if let Some(p) = a["https_port"].as_u64() {
                println!("https://<this server's address>:{p}{path}");
            }
        }
        None => println!("It's set up: the admin pages are at http://localhost:{port}/admin/"),
    }
}

/// $XDG_DATA_HOME/xenify-server, ~/.local/share/xenify-server, or on
/// Windows %APPDATA%\\xenify-server.
fn default_data() -> PathBuf {
    let env = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
    };
    base.unwrap_or_else(|| ".".into()).join("xenify-server")
}

/// `<name>` beside the program (or, run from the source tree, the crate's
/// own `web`), when it holds `inside`.
fn beside(name: &str, inside: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.parent()?.join(name);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
    [exe, source].into_iter().find(|p| p.join(inside).exists())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_settings_file_from_a_newer_version_still_starts_the_server() {
        let text = "database = \"postgres://x\"\nlisten = \"127.0.0.1:7700\"\ncluster = \"a\"\n[at_rest]\ndir = \"/srv/x\"\ncolour = 1\n";
        let f: super::File = toml::from_str(text).unwrap();
        assert_eq!(f.database.as_deref(), Some("postgres://x"));
        assert_eq!(f.at_rest.unwrap().dir, std::path::PathBuf::from("/srv/x"));
        assert_eq!(super::unknown_keys(text), ["cluster", "at_rest.colour"]);
    }
}
