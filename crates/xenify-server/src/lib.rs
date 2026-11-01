//! The Xenify server.
//!
//! - A trusted server: apps send passwords, names and contents over HTTPS
//!   and the server reads them (to search, preview and send email).
//!   Passwords are kept hashed with argon2; sessions are tokens stored only
//!   as hashes.
//! - Set up once by the first admin; people are added by admins (with a
//!   one-time welcome link or code, or a starting password); nobody joins
//!   by themselves. Groups hold people.
//! - Everything is in PostgreSQL, so several servers can share one database.

// Handlers hand back a ready error response; boxing it buys nothing here.
#![allow(clippy::result_large_err)]

pub mod accounts;
pub mod admin;
pub mod ai;
pub mod atrest;
pub mod backup;
mod chatask;
pub mod chatfiles;
pub mod chatjobs;
pub mod cleanup;
pub mod comfy;
pub mod database;
pub mod datasources;
pub mod db;
pub mod files;
pub mod hub;
mod limit;
pub mod live;
pub mod mail;
pub mod media;
pub mod mcp;
pub mod mfa;
pub mod mysettings;
pub mod net;
mod pages;
pub mod perms;
pub mod plan;
pub mod publinks;
pub mod push;
pub mod scan;
pub mod secrets;
pub mod service;
pub mod tls;
pub mod usermail;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use deadpool_postgres::Pool;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use xenify_server_proto::{Account, Info};

/// This build's version: a Preview or Nightly build stamps its own (e.g.
/// 0.1.0-nightly.20261027, scripts/hub-packages.sh sets XENIFY_VERSION),
/// so updates and the browser apps match the release it came from.
pub const VERSION: &str = match option_env!("XENIFY_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// postgres://user:password@host/db
    pub database: String,
    /// Where files' bytes are kept (`blobs/` inside).
    pub data: std::path::PathBuf,
    /// Let chat apps' webhooks point at this computer or its network
    /// (otherwise only public https:// addresses).
    pub local_webhooks: bool,
    /// The admin pages (crates/xenify-server/web), served at /admin/.
    pub web: Option<std::path::PathBuf>,
    /// People's files (`blobs/`, `chat-files/`) when they're kept apart
    /// from `data`: the encrypted folder (encryption at rest).
    pub secure: Option<std::path::PathBuf>,
    /// The server's own PostgreSQL (its bin/postgres), when the settings
    /// file has one: backups use the pg_dump beside it.
    pub postgres: Option<std::path::PathBuf>,
}

/// What `xenify-server serve` adds around the server.
#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// Where to listen for HTTPS (the server's own certificate).
    pub https: Option<String>,
    /// Xenify Write and Draw for the browser (scripts/build-site.sh's
    /// target/site), served at / so they work from this server.
    pub apps: Option<std::path::PathBuf>,
    /// Let apps on the network find the server.
    pub announce: bool,
}

#[derive(Clone)]
pub struct Server {
    pool: Pool,
    /// Needed once, to set the server up; None once it is.
    setup_token: Arc<Mutex<Option<String>>>,
    limit: Arc<limit::Limiter>,
    blobs: Arc<files::Blobs>,
    rooms: Arc<live::Rooms>,
    chat: xenify_chat_server::Server,
    jobs: Arc<ai::Jobs>,
    /// Flips to true when this copy is stopping (an update took over).
    stopping: tokio::sync::watch::Sender<bool>,
    web: Option<std::path::PathBuf>,
    data: std::path::PathBuf,
    /// Where people's files are: `data`, or the encrypted folder.
    pub(crate) secure: std::path::PathBuf,
    apps: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// The release list for Hubs (when it was fetched, from where, the list).
    hub_list: Arc<Mutex<Option<(std::time::Instant, String, serde_json::Value)>>>,
    /// Downloads being copied here for Hubs, one at a time each.
    hub_fetching: Arc<Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Let's Encrypt's certificate for the server's domain name, if it has one.
    acme: tls::AcmeSlot,
    /// The domain name came with --domain (not changed on the admin pages).
    acme_fixed: Arc<std::sync::atomic::AtomicBool>,
    https_port: Arc<Mutex<Option<u16>>>,
    /// The listeners (Admin › Network), and what the command line fixed.
    running: Arc<Mutex<Option<net::Running>>>,
    net_fixed: Arc<Mutex<net::Fixed>>,
    /// Write and Draw for the browser are served (when `apps` is set).
    hosting: Arc<Mutex<bool>>,
    /// How many plain HTTP listeners are still finishing their connections.
    serving: Arc<tokio::sync::watch::Sender<usize>>,
    /// Run by the service: it updates itself (not the Hub).
    self_updating: Arc<Mutex<bool>>,
    /// Downloading the apps for the browser: "" (not), "downloading", or
    /// why it failed.
    web_job: Arc<Mutex<String>>,
    /// What backups need from the settings file, and the one running now.
    backup_paths: backup::Paths,
    backup_running: Arc<backup::Running>,
    /// Admin › Network › Trusted proxies, read again now and then (several
    /// servers may share the database).
    proxies: Arc<Mutex<Option<(std::time::Instant, Arc<Vec<net::Cidr>>)>>>,
}

pub fn pool(database: &str) -> Result<Pool, String> {
    let cfg: tokio_postgres::Config = database.parse().map_err(|e| format!("database address: {e}"))?;
    let mgr = deadpool_postgres::Manager::new(cfg, tokio_postgres::NoTls);
    Pool::builder(mgr).max_size(32).build().map_err(|e| e.to_string())
}

/// The database's schema level, kept in `settings` as "schema_level". Raise
/// it with each release that changes the schema; changes stay additive
/// (new tables, columns with a default or nullable), so an older server
/// still runs on a database a newer one upgraded. See docs/formats.md.
pub const SCHEMA_LEVEL: i64 = 1;

/// Creates the tables (safe to run every start).
pub async fn migrate(pool: &Pool) -> Result<(), String> {
    let c = pool.get().await.map_err(|e| e.to_string())?;
    // Several servers starting at once: one creates, the others wait. The
    // same lock as the chat server's, since they share tables.
    c.execute("SELECT pg_advisory_lock(7420426)", &[]).await.map_err(|e| e.to_string())?;
    // Ours, then Calc's data sources, Plan's register, two-step sign-in and
    // browser notifications (their own tables).
    let mut r = Ok(());
    let level: i64 = match c.query_opt("SELECT value FROM settings WHERE key = 'schema_level'", &[]).await {
        Ok(Some(row)) => row.get::<_, String>(0).parse().unwrap_or(0),
        _ => 0,
    };
    // A newer server set this database up: its schema has all of ours, and
    // our older steps (constraints narrower than its) mustn't undo its.
    let newer = level > SCHEMA_LEVEL;
    if newer {
        eprintln!(
            "xenify-server: this database was upgraded by a newer Xenify server (schema {level}, this one knows {SCHEMA_LEVEL}). \
             It runs, but update the server to use everything that version added."
        );
    }
    let all = [
        db::SCHEMA,
        datasources::SCHEMA,
        plan::SCHEMA,
        mfa::SCHEMA,
        push::SCHEMA,
        perms::SCHEMA,
        mysettings::SCHEMA,
        secrets::SCHEMA,
        usermail::SCHEMA,
    ];
    for schema in if newer { &all[..0] } else { &all[..] } {
        if let Err(e) = c.batch_execute(schema).await {
            r = Err(e.to_string());
            break;
        }
    }
    if r.is_ok()
        && let Err(e) = perms::seed(&c).await
    {
        r = Err(e.to_string());
    }
    if r.is_ok()
        && !newer
        && let Err(e) = c
            .execute(
                "INSERT INTO settings (key, value) VALUES ('schema_level', $1) ON CONFLICT (key) DO UPDATE SET value = $1",
                &[&SCHEMA_LEVEL.to_string()],
            )
            .await
    {
        r = Err(e.to_string());
    }
    let _ = c.execute("SELECT pg_advisory_unlock(7420426)", &[]).await;
    r?;
    // Things that stay the same for the server's life.
    for (key, value) in [("server_id", random(12)), ("secret", random(32)), ("name", "Xenify server".into())] {
        c.execute("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&key, &value])
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What `xenify-server configure` changes (before the server runs, e.g.
/// from an install script; afterwards these are on Admin › Network).
#[derive(Debug, Default)]
pub struct Configure {
    pub http_port: Option<u16>,
    /// Some(None): HTTPS off.
    pub https_port: Option<Option<u16>>,
    pub domain: Option<String>,
    pub email: Option<String>,
    /// "local", "lan" or "anywhere".
    pub admin_access: Option<String>,
}

/// A code for `user` (user name or email address) to pick a new password
/// with when signing in: `xenify-server reset-password`, for an admin who
/// can't sign in. Signs them out everywhere.
pub async fn reset_password_code(database: &str, user: &str) -> Result<String, String> {
    let pool = pool(database)?;
    let c = pool.get().await.map_err(|e| e.to_string())?;
    let e = |e: tokio_postgres::Error| e.to_string();
    let Some(l) = db::login_of(&c, &user.trim().to_lowercase()).await.map_err(e)? else {
        return Err(format!("there's nobody called {user} on this server"));
    };
    c.execute("UPDATE users SET password_hash = '', disabled = FALSE WHERE id = $1", &[&l.account.id])
        .await
        .map_err(e)?;
    c.execute("DELETE FROM sessions WHERE user_id = $1", &[&l.account.id]).await.map_err(e)?;
    // Locked out may mean a lost phone too.
    mfa::clear(&c, l.account.id).await.map_err(e)?;
    accounts::sessions_changed(&c, l.account.id).await;
    let (code, _) = accounts::make_code(
        &c,
        xenify_server_proto::LinkKind::Reset,
        l.account.id,
        None,
        12,
        accounts::ADMIN_CODE_MINUTES,
    )
    .await
    .map_err(e)?;
    Ok(code)
}

/// Saves `c` in the server's database (making its tables first).
pub async fn configure(database: &str, c: Configure) -> Result<(), String> {
    let pool = pool(database)?;
    xenify_chat_server::migrate(&pool).await?;
    migrate(&pool).await?;
    let db = pool.get().await.map_err(|e| e.to_string())?;
    let e = |e: tokio_postgres::Error| e.to_string();
    if c.http_port.is_some() || c.https_port.is_some() {
        let mut net: net::Net =
            db::setting(&db, "net").await.map_err(e)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        if let Some(p) = c.http_port {
            net.http_port = p;
        }
        if let Some(p) = c.https_port {
            net.https_port = p;
        }
        if net.http_port == 0 || net.https_port == Some(0) || net.https_port == Some(net.http_port) {
            return Err("choose two different ports between 1 and 65535".into());
        }
        db::set_setting(&db, "net", &serde_json::to_string(&net).expect("serialises")).await.map_err(e)?;
    }
    if c.domain.is_some() || c.email.is_some() {
        let mut h = admin::https_setting(&db).await;
        if let Some(d) = c.domain {
            h.domain = d.trim().trim_end_matches('.').to_ascii_lowercase();
            if !h.domain.is_empty() && !admin::valid_domain(&h.domain) {
                return Err("that isn't a domain name (like files.example.com)".into());
            }
        }
        if let Some(m) = c.email {
            h.email = m.trim().to_string();
        }
        db::set_setting(&db, "https", &serde_json::to_string(&h).expect("serialises")).await.map_err(e)?;
    }
    if let Some(a) = c.admin_access {
        if !["local", "lan", "anywhere"].contains(&a.as_str()) {
            return Err("admin pages are open to local, lan or anywhere".into());
        }
        db::set_setting(&db, "admin_access", &a).await.map_err(e)?;
    }
    Ok(())
}

impl Server {
    pub async fn start(config: Config) -> Result<Server, String> {
        let pool = pool(&config.database)?;
        // Chat's tables first: ours add to its accounts.
        xenify_chat_server::migrate(&pool).await?;
        migrate(&pool).await?;
        let chat = xenify_chat_server::Server::start(xenify_chat_server::Config {
            database: config.database.clone(),
            local_webhooks: config.local_webhooks,
        })
        .await?;
        let c = pool.get().await.map_err(|e| e.to_string())?;
        let fresh = db::people_count(&c).await.map_err(|e| e.to_string())? == 0;
        drop(c);
        let setup_token = fresh.then(|| random(18));
        let secure = config.secure.clone().unwrap_or_else(|| config.data.clone());
        let blobs = files::Blobs::open(&secure.join("blobs")).map_err(|e| format!("data folder: {e}"))?;
        let s = Server {
            pool,
            setup_token: Arc::new(Mutex::new(setup_token)),
            limit: Arc::default(),
            blobs: Arc::new(blobs),
            rooms: Arc::default(),
            chat,
            jobs: Arc::default(),
            stopping: tokio::sync::watch::channel(false).0,
            web: config.web.clone(),
            data: config.data.clone(),
            secure,
            apps: Arc::default(),
            hub_list: Arc::default(),
            hub_fetching: Arc::default(),
            acme: Arc::default(),
            acme_fixed: Arc::default(),
            https_port: Arc::default(),
            running: Arc::default(),
            net_fixed: Arc::default(),
            hosting: Arc::new(Mutex::new(true)),
            serving: Arc::new(tokio::sync::watch::channel(0).0),
            self_updating: Arc::default(),
            web_job: Arc::default(),
            backup_paths: backup::Paths {
                database: config.database.clone(),
                data: config.data.clone(),
                secure: config.secure.clone().unwrap_or_else(|| config.data.clone()),
                postgres: config.postgres.clone(),
            },
            backup_running: Arc::default(),
            proxies: Arc::default(),
        };
        s.listen_live(&config.database).await?;
        chatjobs::start(s.clone());
        cleanup::start(s.clone());
        backup::start(s.clone());
        usermail::start(s.clone());
        db::start_cleanup(s.pool.clone(), s.stopping());
        // Chat's "Ask AI" uses the server's AI.
        s.chat.set_assistant(Arc::new(chatask::ChatAi { pool: s.pool.clone() }));
        Ok(s)
    }

    /// The token the first admin needs, while the server isn't set up.
    pub fn setup_token(&self) -> Option<String> {
        self.setup_token.lock().unwrap().clone()
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    pub fn router(&self) -> Router {
        let mut r = Router::new();
        if let Some(web) = &self.web {
            use axum::http::{HeaderValue, header};
            use tower_http::set_header::SetResponseHeaderLayer;
            // The admin pages run only their own code, and never inside
            // another site's frame.
            let csp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                       img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'";
            let pages = tower_http::services::ServeDir::new(web).append_index_html_on_directories(true);
            let service = tower::ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(csp),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::REFERRER_POLICY,
                    HeaderValue::from_static("no-referrer"),
                ))
                // Check for a newer copy each time (cheap: unchanged files
                // answer 304), so pages match the server
                // after an update.
                .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
                .service(pages);
            // The Files page (for everyone with an account) and pages opened
            // with a public link: one page, its files beside the admin pages'.
            let files_page = tower::ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(csp),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::REFERRER_POLICY,
                    HeaderValue::from_static("no-referrer"),
                ))
                .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
                .service(tower_http::services::ServeFile::new(web.join("files.html")));
            // A one-time secret's page: kept nowhere on the way.
            let secret_page = tower::ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(csp),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::REFERRER_POLICY,
                    HeaderValue::from_static("no-referrer"),
                ))
                .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
                .service(tower_http::services::ServeFile::new(web.join("secret.html")));
            r = r
                .route("/files", get(|| async { axum::response::Redirect::permanent("/files/") }))
                .route_service("/files/", files_page.clone())
                // Its places are paths (/files/wiki/3/4): the same page.
                .route_service("/files/{*place}", files_page.clone())
                .route_service("/p/{token}", files_page)
                .route("/secret", get(|| async { axum::response::Redirect::permanent("/secret/") }))
                .route_service("/secret/", secret_page)
                .nest_service("/web", service.clone());
            // The same pages at /join/: welcome and reset links work from anywhere,
            // while /admin/ may be kept to this network.
            r = r.nest_service("/admin", service.clone()).nest_service("/join", service);
            // The server's own address: Files (which leads to the apps and
            // the admin pages), or the admin pages while it still needs
            // setting up.
            let s = self.clone();
            r = r.route(
                "/",
                get(move || async move {
                    axum::response::Redirect::to(if s.setup_token().is_some() { "/admin/" } else { "/files/" })
                }),
            );
        }
        // Write and Draw for the browser, set up for this server.
        if let Some(apps) = self.hosted_apps() {
            use axum::http::{HeaderValue, header};
            use tower_http::set_header::SetResponseHeaderLayer;
            // They run only their own code (and WebAssembly), talk only to
            // this server, and open only inside its own pages.
            let files = tower::ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ))
                .service(tower_http::services::ServeDir::new(apps).append_index_html_on_directories(true));
            let apps = Router::new().fallback_service(files).layer(axum::middleware::from_fn(apps_csp));
            r = r.fallback_service(apps);
        }
        r.route("/v1/info", get(info))
            .route("/v1/health", get(|| async { "ok" }))
            .route("/v1/live", get(live::live))
            .route("/v1/ca.pem", get(ca_pem))
            .merge(accounts::routes())
            .merge(admin::routes())
            .merge(backup::routes())
            .merge(mail::routes())
            .merge(mfa::routes())
            .merge(files::routes())
            .merge(chatfiles::routes())
            .merge(media::routes())
            .merge(push::routes())
            .merge(ai::routes())
            .merge(datasources::routes())
            .merge(plan::routes())
            .merge(perms::routes())
            .merge(mysettings::routes())
            .merge(publinks::routes())
            .merge(pages::routes())
            .merge(mcp::routes())
            .merge(hub::routes())
            .merge(secrets::routes())
            .merge(usermail::routes())
            .merge(scan::routes())
            .merge(net::routes())
            .layer(axum::middleware::from_fn_with_state(self.clone(), admin_gate))
            // The desktop apps' screens come from their own origin. Sign-in
            // is by bearer token, never cookies, so any origin may call.
            .layer(
                tower_http::cors::CorsLayer::new()
                    .allow_origin(tower_http::cors::Any)
                    .allow_methods(tower_http::cors::Any)
                    .allow_headers([axum::http::header::AUTHORIZATION, axum::http::header::CONTENT_TYPE]),
            )
            .with_state(self.clone())
            .merge(self.chat.chat_routes())
            .layer(axum::middleware::from_fn_with_state(self.clone(), transport))
    }

    /// Starts stopping: live editing windows are asked to reconnect (to the
    /// newer copy, during an update).
    pub fn stop(&self) {
        let _ = self.stopping.send(true);
    }

    pub(crate) fn backup_paths(&self) -> backup::Paths {
        self.backup_paths.clone()
    }

    pub(crate) fn backup_running(&self) -> Arc<backup::Running> {
        self.backup_running.clone()
    }

    pub(crate) fn stopping(&self) -> tokio::sync::watch::Receiver<bool> {
        self.stopping.subscribe()
    }

    /// Serves the web apps from `dir` (call before `router`/`serve`).
    pub fn host_apps(&self, dir: std::path::PathBuf) {
        *self.apps.lock().unwrap() = Some(dir);
    }

    /// Write and Draw for the browser, when they're served.
    pub(crate) fn hosted_apps(&self) -> Option<std::path::PathBuf> {
        self.apps.lock().unwrap().clone().filter(|_| *self.hosting.lock().unwrap())
    }

    /// The command line's choices for Admin › Network (call before
    /// `run_network`).
    pub fn fix_network(&self, fixed: net::Fixed) {
        *self.net_fixed.lock().unwrap() = fixed;
    }

    /// HTTPS on `listener` until `stop`: this server's own certificate
    /// (made for every name and address of this computer), and Let's
    /// Encrypt's for its domain name when it has one.
    pub(crate) fn serve_https_until(
        &self,
        listener: tokio::net::TcpListener,
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), String> {
        let (_, chain, key) = tls::local_certificate(&self.data.join("tls"), &tls::local_names())?;
        let own = tls::config(&chain, &key)?;
        *self.https_port.lock().unwrap() = listener.local_addr().ok().map(|a| a.port());
        let certs = tls::Certs { own, acme: self.acme.clone() };
        tokio::spawn(tls::serve(listener, certs, self.router(), stop));
        Ok(())
    }

    /// Let's Encrypt certificates for `domains`, from the command line (the
    /// admin pages then show them but can't change them).
    pub fn fix_domains(&self, domains: Vec<String>, email: Option<String>, production: bool) {
        self.acme_fixed.store(true, std::sync::atomic::Ordering::SeqCst);
        self.use_domains(domains, email, production);
    }

    /// Gets (and renews) Let's Encrypt certificates for `domains`; none
    /// stops that.
    pub(crate) fn use_domains(&self, domains: Vec<String>, email: Option<String>, production: bool) {
        let acme = (!domains.is_empty())
            .then(|| Arc::new(tls::acme(domains, email, &self.data.join("tls").join("acme"), production)));
        *self.acme.write().unwrap() = acme;
    }

    /// The domain name saved on Admin › Network, when not given with --domain.
    pub async fn use_saved_domain(&self) {
        if self.acme_fixed.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Ok(c) = self.pool.get().await else { return };
        let set = admin::https_setting(&c).await;
        if !set.domain.is_empty() {
            self.use_domains(vec![set.domain], Some(set.email).filter(|e| !e.is_empty()), true);
        }
    }

    /// What people call this server.
    pub async fn name(&self) -> String {
        match self.pool.get().await {
            Ok(c) => db::setting(&c, "name").await.ok().flatten().unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    pub fn data(&self) -> &std::path::Path {
        &self.data
    }

    /// Serves on `listener` until the future `stop` finishes.
    pub async fn serve(
        &self,
        listener: tokio::net::TcpListener,
        stop: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(stop)
            .await
    }
}

/// The apps for the browser (Write, Draw, Calc, Plan, Chat): their own
/// scripts and WebAssembly; pictures, sounds and workers they make
/// themselves (blob:, data:); only this server to talk to.
/// Draw also reaches OpenRouter (AI with someone's own key), GIPHY and the
/// animated emoji (Google's Noto).
pub const APPS_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https://*.giphy.com; \
    media-src 'self' data: blob:; font-src 'self' data:; worker-src 'self' blob:; \
    connect-src 'self' https://openrouter.ai https://api.giphy.com https://*.giphy.com \
    https://fonts.gstatic.com https://googlefonts.github.io; frame-src 'self' blob:; \
    frame-ancestors 'self'; object-src 'none'; base-uri 'none'; form-action 'self'";

/// The apps' pages set a few settings in small scripts of their own
/// (where their WebAssembly is): those exact scripts may run too.
async fn apps_csp(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    use axum::http::{HeaderValue, header};
    let r = next.run(req).await;
    let html = r
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/html"));
    let (mut parts, body) = r.into_parts();
    let mut csp = APPS_CSP.to_string();
    let body = if html {
        let Ok(bytes) = axum::body::to_bytes(body, 8 << 20).await else { return failed() };
        let hashes = inline_script_hashes(&String::from_utf8_lossy(&bytes));
        if !hashes.is_empty() {
            csp = csp.replacen("'wasm-unsafe-eval'", &format!("'wasm-unsafe-eval' {}", hashes.join(" ")), 1);
        }
        axum::body::Body::from(bytes)
    } else {
        body
    };
    if let Ok(v) = HeaderValue::from_str(&csp) {
        parts.headers.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    Response::from_parts(parts, body)
}

/// 'sha256-…' for each `<script>` without `src` in `html`.
pub fn inline_script_hashes(html: &str) -> Vec<String> {
    use sha2::{Digest, Sha256};
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<script") {
        let open = at + i;
        let Some(gt) = lower[open..].find('>').map(|g| open + g) else { break };
        let Some(end) = lower[gt..].find("</script").map(|e| gt + e) else { break };
        let tag = &lower[open..gt];
        if !tag.contains(" src=") && !tag.contains("\tsrc=") && !tag.contains("\nsrc=") {
            let digest = Sha256::digest(html[gt + 1..end].as_bytes());
            use base64::Engine;
            out.push(format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(digest)));
        }
        at = end;
    }
    out
}

/// The server's own certificate authority, for computers to trust once.
async fn ca_pem(State(s): State<Server>) -> Response {
    match std::fs::read_to_string(s.data.join("tls/ca.pem")) {
        Ok(pem) => (
            [
                (axum::http::header::CONTENT_TYPE, "application/x-pem-file"),
                (axum::http::header::CONTENT_DISPOSITION, "attachment; filename=\"xenify-server.crt\""),
            ],
            pem,
        )
            .into_response(),
        Err(_) => err(StatusCode::NOT_FOUND, "no_https", "this server doesn't use its own certificate"),
    }
}

async fn info(State(s): State<Server>, h: HeaderMap) -> Response {
    // The version only for people signed in: nobody else learns which
    // version (and its known problems) a server runs.
    let signed_in = signed_in(&s, &h).await.is_ok();
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let get = |k: &'static str| {
        let c = &c;
        async move { db::setting(c, k).await.ok().flatten().unwrap_or_default() }
    };
    Json(Info {
        name: get("name").await,
        version: if signed_in { VERSION.into() } else { String::new() },
        protocol: xenify_server_proto::VERSION,
        oldest_protocol: Some(xenify_server_proto::OLDEST),
        server_id: get("server_id").await,
        setup_needed: s.setup_token().is_some(),
        features: [
            ("web_apps", s.hosted_apps().is_some()),
            ("public_links", get("public_links").await != "off"),
            ("mail", mail::mail(&c).await.is_some()),
            // People add their own email accounts (Chat's Mail).
            ("email", true),
        ]
        .into_iter()
        .filter(|f| f.1)
        .map(|f| f.0.to_string())
        .collect(),
    })
    .into_response()
}

// ---- Helpers shared by the handlers ------------------------------------

pub(crate) type Peer = ConnectInfo<SocketAddr>;

pub(crate) fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    xenify_sync::b64(&Sha256::digest(token.as_bytes()))
}

pub(crate) fn random(n: usize) -> String {
    xenify_sync::b64(&xenify_sync::random(n))
}

/// Hashing passwords takes a lot of memory and time on purpose: a few at
/// once, off the threads that answer requests.
static ARGON: std::sync::LazyLock<tokio::sync::Semaphore> = std::sync::LazyLock::new(|| {
    tokio::sync::Semaphore::new(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 8))
});

/// A hash checked when there's no real one (no such account, no password
/// yet), so the time taken doesn't tell.
static DUMMY_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| hash_now("not anyone's password"));

fn hash_now(auth: &str) -> String {
    use argon2::password_hash::PasswordHasher;
    argon2::Argon2::default().hash_password_with_salt(auth.as_bytes(), &xenify_sync::random(16)).expect("hash").to_string()
}

fn check_now(auth: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    match PasswordHash::new(hash) {
        Ok(h) => argon2::Argon2::default().verify_password(auth.as_bytes(), &h).is_ok(),
        Err(_) => {
            let h = PasswordHash::new(&DUMMY_HASH).expect("our own hash");
            let _ = argon2::Argon2::default().verify_password(auth.as_bytes(), &h);
            false
        }
    }
}

/// Hashes a password (or a link's secret) for keeping.
pub(crate) async fn hash_password(auth: &str) -> String {
    let auth = auth.to_string();
    let _turn = ARGON.acquire().await.expect("never closed");
    tokio::task::spawn_blocking(move || hash_now(&auth)).await.expect("hashing doesn't panic")
}

/// Whether `auth` matches `hash`; an empty or broken hash takes as long and
/// never matches.
pub(crate) async fn password_ok(auth: &str, hash: &str) -> bool {
    let (auth, hash) = (auth.to_string(), hash.to_string());
    let _turn = ARGON.acquire().await.expect("never closed");
    tokio::task::spawn_blocking(move || check_now(&auth, &hash)).await.unwrap_or(false)
}

/// This server's address as people reach it ("https://files.example.org"),
/// for links: the one a full admin saved on Admin › Network, else its
/// domain name's, else (not kept) the address this request came to.
pub(crate) async fn base_url(c: &impl deadpool_postgres::GenericClient, headers: &HeaderMap) -> String {
    address_for(c, headers).await
}

/// The same, for links made by anyone (a one-time secret's).
pub(crate) async fn link_base(c: &impl deadpool_postgres::GenericClient, headers: &HeaderMap) -> String {
    address_for(c, headers).await
}

/// The saved address, else the domain name's (for email sent with no
/// request to go by); empty when there's neither.
pub(crate) async fn server_address(c: &impl deadpool_postgres::GenericClient) -> String {
    if let Some(a) = db::setting(c, "address").await.ok().flatten().filter(|a| !a.is_empty()) {
        return a;
    }
    let https: serde_json::Value =
        db::setting(c, "https").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    let domain = https["domain"].as_str().unwrap_or("").trim();
    if domain.is_empty() {
        return String::new();
    }
    let net: net::Net =
        db::setting(c, "net").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    match net.https_port {
        // Off: a proxy in front does HTTPS, on the usual port.
        Some(443) | None => format!("https://{domain}"),
        Some(p) => format!("https://{domain}:{p}"),
    }
}

async fn address_for(c: &impl deadpool_postgres::GenericClient, headers: &HeaderMap) -> String {
    let known = server_address(c).await;
    if !known.is_empty() {
        return known;
    }
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    // The Origin counts only when it's this server's own address (the
    // desktop apps' pages have origins of their own).
    let host = header("host").unwrap_or_default();
    let own = |o: &String| o.starts_with("http") && o.split_once("://").is_some_and(|(_, h)| h == host);
    if let Some(o) = header("origin").filter(own) {
        return o;
    }
    // Else the request's own: a proxy says which; without one, the plain
    // HTTP port (or a server with HTTPS off) is http.
    let net: net::Net =
        db::setting(c, "net").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    let port = host.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok());
    let scheme = match header("x-forwarded-proto") {
        Some(p) if p == "http" || p == "https" => p,
        _ if port == Some(net.http_port) || net.https_port.is_none() => "http".into(),
        _ => "https".into(),
    };
    format!("{scheme}://{}", if host.is_empty() { "localhost" } else { &host })
}

pub(crate) fn err(status: StatusCode, code: &str, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg, "code": code }))).into_response()
}

pub(crate) fn unavailable() -> Response {
    err(StatusCode::SERVICE_UNAVAILABLE, "database", "the server can't reach its database")
}

pub(crate) fn failed() -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, "failed", "something went wrong on the server")
}

/// A display name: trimmed, single spaces, at most 64 characters.
pub fn clean_name(n: &str) -> String {
    n.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(64).collect()
}

/// User names: 2–32 letters, digits, dots, dashes or underscores.
pub fn valid_username(n: &str) -> bool {
    (2..=32).contains(&n.chars().count()) && n.chars().all(|c| c.is_alphanumeric() || "._-".contains(c))
}

/// The signed-in account behind `Authorization: Bearer <token>`.
pub(crate) async fn signed_in(s: &Server, headers: &HeaderMap) -> Result<Account, Response> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let Ok(c) = s.pool.get().await else { return Err(unavailable()) };
    db::session_account(&c, &hash_token(token))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "signed_out", "please sign in again"))
}

/// Signed in as an admin, or someone whose role holds `power` (one part of
/// running the server, Admin › Roles › Advanced: "admin.ai" …).
pub(crate) async fn admin_power(s: &Server, headers: &HeaderMap, power: &str) -> Result<Account, Response> {
    let a = signed_in(s, headers).await?;
    if a.admin {
        return Ok(a);
    }
    let c = s.pool.get().await.map_err(|_| unavailable())?;
    if crate::perms::can(&c, a.id, power).await {
        return Ok(a);
    }
    Err(err(StatusCode::FORBIDDEN, "not_admin", "only admins can do that"))
}

/// As admin_power, with any one of `powers`.
pub(crate) async fn admin_any(s: &Server, headers: &HeaderMap, powers: &[&str]) -> Result<Account, Response> {
    let mut last = None;
    for p in powers {
        match admin_power(s, headers, p).await {
            Ok(a) => return Ok(a),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| err(StatusCode::FORBIDDEN, "not_admin", "only admins can do that")))
}

/// Listens on `addr`, letting a newer copy of the server listen on the same
/// port at the same time (on Linux and macOS), so updating needs no
/// downtime: the new copy starts, then the old one stops taking new
/// connections and finishes what it's doing.
pub fn listen(addr: &str) -> std::io::Result<tokio::net::TcpListener> {
    use std::net::ToSocketAddrs;
    let addr = addr.to_socket_addrs()?.next().ok_or(std::io::ErrorKind::InvalidInput)?;
    let socket = socket2::Socket::new(socket2::Domain::for_address(addr), socket2::Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    tokio::net::TcpListener::from_std(socket.into())
}

/// Who may reach the admin pages and `/v1/admin/…`: "local" (this computer),
/// "lan" (home and office addresses; the default) or "anywhere".
pub fn may_admin(setting: &str, ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    let local = ip.is_loopback();
    let lan = match ip {
        // 100.64.0.0/10 too: VPNs like Tailscale give out these addresses.
        IpAddr::V4(v) => {
            v.is_private()
                || v.is_link_local()
                || v.is_loopback()
                || (v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(v) => {
            v.is_loopback()
                // Unique local addresses (fc00::/7), Tailscale's among them.
                || (v.segments()[0] & 0xfe00) == 0xfc00
                || (v.segments()[0] & 0xffc0) == 0xfe80
                || v.to_ipv4_mapped().is_some_and(|m| m.is_private() || m.is_loopback() || m.is_link_local())
        }
    };
    match setting {
        "local" => local,
        "anywhere" => true,
        _ => lan,
    }
}

/// Marks requests that came over HTTPS.
#[derive(Clone, Copy)]
pub(crate) struct Secure;

/// Marks HTTPS with a certificate every browser trusts (Let's Encrypt's for
/// the domain name): only then may browsers be told to always use HTTPS.
#[derive(Clone, Copy)]
pub(crate) struct PublicCert;

/// Turns admin requests away from addresses the server isn't open to.
async fn admin_gate(
    State(s): State<Server>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path();
    // Behind a trusted proxy: where the request really came from, and
    // whether it reached the proxy over HTTPS.
    let ip = s.client_ip(peer.ip(), req.headers()).await;
    let proxied_https = s.trusts_proxy(peer.ip()).await
        && req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https");
    let secure = req.extensions().get::<Secure>().is_some() || proxied_https;
    // Until it's set up, setting it up needs the token the server printed,
    // and either this computer, its network (or a VPN, or an SSH tunnel), or
    // HTTPS: the token never crosses the internet unencrypted.
    if s.setup_token().is_some() {
        if (path == "/v1/setup" || path.starts_with("/v1/admin")) && !secure && !may_admin("lan", ip) {
            return err(
                StatusCode::FORBIDDEN,
                "setup_elsewhere",
                "set the server up over HTTPS, or from its own network (an SSH tunnel or a VPN works too)",
            );
        }
    } else if path.starts_with("/v1/admin") || path == "/admin" || path.starts_with("/admin/") {
        let setting = match s.pool.get().await {
            Ok(c) => db::setting(&c, "admin_access").await.ok().flatten().unwrap_or_default(),
            Err(_) => return unavailable(),
        };
        if !may_admin(&setting, ip) {
            return err(
                StatusCode::FORBIDDEN,
                "admin_elsewhere",
                "admin pages can't be opened from here: use a computer on the server's network",
            );
        }
        // Open to anywhere: from outside the network, only over HTTPS.
        if setting == "anywhere" && !secure && !may_admin("lan", ip) {
            return err(
                StatusCode::FORBIDDEN,
                "https_only",
                "from outside its network, open the admin pages over HTTPS",
            );
        }
    }
    // The pages load their code from beside them: "/admin" (typed without
    // the last slash) would find none.
    if path == "/admin" || path == "/join" {
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        return axum::response::Redirect::permanent(&format!("{path}/{query}")).into_response();
    }
    next.run(req).await
}

/// Around every request: plain HTTP from outside sent to HTTPS, and the
/// headers every answer gets.
async fn transport(
    State(s): State<Server>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use axum::http::{HeaderValue, header};
    let public = req.extensions().get::<PublicCert>().is_some();
    let (parts, body) = req.into_parts();
    if parts.extensions.get::<Secure>().is_none()
        && let Some(to) = s.https_redirect(peer.ip(), &parts).await
    {
        return to;
    }
    let req = axum::extract::Request::from_parts(parts, body);
    let api = req.uri().path().starts_with("/v1/");
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    if api {
        h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    }
    if public {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn admin_pages_open_to_the_network_by_default() {
        use super::may_admin;
        let ip = |s: &str| s.parse().unwrap();
        assert!(may_admin("", ip("192.168.1.30")));
        assert!(may_admin("", ip("10.0.0.4")));
        assert!(may_admin("", ip("fe80::1")));
        assert!(may_admin("", ip("::ffff:192.168.1.30")));
        assert!(!may_admin("", ip("8.8.8.8")));
        assert!(may_admin("", ip("100.101.2.3")));
        assert!(!may_admin("", ip("100.128.0.1")));
        assert!(may_admin("", ip("fd7a:115c:a1e0::1")));
        assert!(!may_admin("lan", ip("2001:db8::1")));
        assert!(may_admin("local", ip("127.0.0.1")));
        assert!(!may_admin("local", ip("192.168.1.30")));
        assert!(may_admin("anywhere", ip("8.8.8.8")));
    }
}

#[cfg(test)]
mod client_copies {
    /// Write, Draw, Calc and Plan carry copies of client/server.js (scripts/sync-server-client.sh).
    #[test]
    fn the_apps_have_the_current_server_client() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for file in ["server.js", "serverui.js", "server.css"] {
            let source = std::fs::read_to_string(root.join("client").join(file)).unwrap();
            for app in ["xenify-write-app", "xenify-draw-app", "xenify-calc-app", "xenify-plan-app"] {
                let copy = std::fs::read_to_string(root.join(format!("../{app}/ui/{file}"))).unwrap_or_default();
                assert!(copy == source, "{app}/ui/{file} is out of date: run scripts/sync-server-client.sh");
            }
        }
    }

    /// The client's protocol levels follow xenify-server-proto's.
    #[test]
    fn the_client_speaks_the_protocol_levels_of_this_build() {
        let js = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("client/server.js")).unwrap();
        assert!(js.contains(&format!("const PROTOCOL = {};", xenify_server_proto::VERSION)), "client/server.js PROTOCOL");
        assert!(js.contains(&format!("const OLDEST = {};", xenify_server_proto::OLDEST)), "client/server.js OLDEST");
    }
}
