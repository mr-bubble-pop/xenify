//! Xenify Hub on the computers this server manages: the admin's update
//! policy (which apps, which channel or version, when), and a copy of the
//! release list and its downloads, so office computers fetch updates over
//! the network once instead of each from the internet.
//!
//! Hubs check every list and download against Xenify's release key and the
//! list's sha256 themselves: the server is a convenience, not trusted with
//! what gets installed.

use crate::{Server, admin_power, db, err, failed, signed_in, unavailable};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use xenify_hub_core::{AppDefaults, Policy, ProductPolicy};

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/hub/policy", get(policy))
        .route("/v1/hub/manifest", get(manifest))
        .route("/v1/hub/files/{sha}", get(file))
        .route("/v1/hub/report", axum::routing::post(report))
        .route("/v1/admin/hub", get(admin_settings).post(change_settings))
        .route("/v1/admin/hub/status", get(status))
        .route("/v1/admin/hub/update-server", axum::routing::post(update_server))
}

/// Admin › Apps & updates.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where the signed release list comes from: empty for Xenify's own
    /// (not on the admin pages; for tests and private mirrors).
    pub source: String,
    /// Keep a copy of the list and the downloads here for the Hubs.
    pub mirror: bool,
    pub products: std::collections::BTreeMap<String, ProductPolicy>,
    /// The "All apps" row: for every app whose own row leaves it open.
    pub all_apps: AppDefaults,
}

impl Settings {
    fn policy(&self, server: String) -> Policy {
        Policy { server, products: self.products.clone(), all_apps: self.all_apps.clone() }
    }

    fn source(&self) -> &str {
        if self.source.is_empty() { xenify_hub_core::RELEASES } else { &self.source }
    }
}

async fn settings(c: &deadpool_postgres::Client) -> Settings {
    db::setting(c, "hub").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

async fn admin_settings(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.updates").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    Json(settings(&c).await).into_response()
}

async fn change_settings(State(s): State<Server>, h: HeaderMap, Json(v): Json<Settings>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.updates").await {
        return e;
    }
    if !v.source.is_empty() && !(v.source.starts_with("https://") || v.source.starts_with("http://")) {
        return err(StatusCode::BAD_REQUEST, "bad_source", "the release list's address must start with https://");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match db::set_setting(&c, "hub", &serde_json::to_string(&v).expect("serialises")).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// The policy for this computer's Hub (anyone signed in).
async fn policy(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = signed_in(&s, &h).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let set = settings(&c).await;
    // Asked for on Admin › Apps & updates: the Hub on this server's own
    // computer moves it to this version (the new one takes over, no downtime).
    let update_server = db::setting(&c, "hub_update_server").await.ok().flatten().filter(|v| v != crate::VERSION);
    Json(serde_json::json!({
        "policy": set.policy(s.name().await),
        "mirror": set.mirror,
        "update_server": update_server,
    }))
    .into_response()
}

pub(crate) fn blocking_get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into();
    let mut r = agent.get(url).call().map_err(|e| e.to_string())?;
    r.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())
}

/// Downloads `url` into `part`, checking its size and sha256 as it goes
/// ("wrong" when it doesn't match; the part is removed then).
pub(crate) fn copy_checked(
    url: &str,
    size: u64,
    sha: &str,
    dir: &std::path::Path,
    part: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    use sha2::Digest;
    use std::io::{Read, Write};
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(3600))).build().into();
    let mut r = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut body = r.body_mut().with_config().limit(size + 1).reader();
    let mut out = std::fs::File::create(part).map_err(|e| e.to_string())?;
    let mut h = sha2::Sha256::new();
    let (mut n, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        let k = body.read(&mut buf).map_err(|e| e.to_string())?;
        if k == 0 {
            break;
        }
        n += k as u64;
        h.update(&buf[..k]);
        out.write_all(&buf[..k]).map_err(|e| e.to_string())?;
    }
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if n != size || got != sha {
        let _ = std::fs::remove_file(part);
        return Err("wrong".into());
    }
    Ok(part.to_path_buf())
}

/// The release list, fetched from the source at most every 10 minutes.
async fn current_list(s: &Server, source: &str) -> Result<Value, String> {
    {
        let cache = s.hub_list.lock().expect("not poisoned");
        if let Some((when, from, list)) = cache.as_ref()
            && from == source
            && when.elapsed() < Duration::from_secs(600)
        {
            return Ok(list.clone());
        }
    }
    let url = source.to_string();
    let bytes =
        tokio::task::spawn_blocking(move || blocking_get(&url, 16 << 20)).await.map_err(|e| e.to_string())??;
    let list: Value = serde_json::from_slice(&bytes).map_err(|_| "the release list can't be read".to_string())?;
    *s.hub_list.lock().expect("not poisoned") = Some((std::time::Instant::now(), source.to_string(), list.clone()));
    Ok(list)
}

async fn mirror_source(s: &Server, h: &HeaderMap) -> Result<String, Response> {
    signed_in(s, h).await?;
    let Ok(c) = s.pool.get().await else { return Err(unavailable()) };
    let set = settings(&c).await;
    if !set.mirror {
        return Err(err(StatusCode::NOT_FOUND, "no_mirror", "this server doesn't keep copies of updates"));
    }
    Ok(set.source().to_string())
}

/// The signed release list as published (the Hub checks the signature).
async fn manifest(State(s): State<Server>, h: HeaderMap) -> Response {
    let source = match mirror_source(&s, &h).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    match current_list(&s, &source).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "source", &format!("the release list couldn't be fetched: {e}")),
    }
}

/// A download from the list, by its sha256: fetched once, then kept here.
async fn file(State(s): State<Server>, h: HeaderMap, Path(sha): Path<String>) -> Response {
    let source = match mirror_source(&s, &h).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let sha = sha.to_ascii_lowercase();
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return err(StatusCode::BAD_REQUEST, "bad_file", "that isn't a download's sha256");
    }
    let dir = s.data.join("hub");
    let path = dir.join(&sha);
    if !path.exists() {
        // Only downloads the list names are fetched (by their own address).
        let list = match current_list(&s, &source).await {
            Ok(v) => v,
            Err(e) => {
                return err(StatusCode::BAD_GATEWAY, "source", &format!("the release list couldn't be fetched: {e}"));
            }
        };
        let Some(d) = find_download(&list, &sha) else {
            return err(StatusCode::NOT_FOUND, "not_listed", "that download isn't in the release list");
        };
        let (url, size) = (d.url, d.size);
        // One download per file at a time.
        let lock = s.hub_fetching.lock().expect("not poisoned").entry(sha.clone()).or_default().clone();
        let _held = lock.lock().await;
        if !path.exists() {
            let (part, sha2, dir2) = (dir.join(format!("{sha}.part")), sha.clone(), dir.clone());
            let got = tokio::task::spawn_blocking(move || copy_checked(&url, size, &sha2, &dir2, &part)).await;
            match got {
                Ok(Ok(file)) => {
                    if std::fs::rename(&file, &path).is_err() {
                        return failed();
                    }
                }
                Ok(Err(e)) if e == "wrong" => {
                    return err(StatusCode::BAD_GATEWAY, "wrong_file", "the download doesn't match the release list");
                }
                _ => return err(StatusCode::BAD_GATEWAY, "source", "the download couldn't be fetched"),
            }
        }
    }
    match tokio::fs::File::open(&path).await {
        Ok(f) => crate::files::attachment(
            (
                [(header::CONTENT_TYPE, "application/octet-stream")],
                Body::from_stream(tokio_util::io::ReaderStream::new(f)),
            )
                .into_response(),
        ),
        Err(_) => failed(),
    }
}

/// The download with this sha256 in a signed list (`{manifest: "<json>", …}`).
fn find_download(list: &Value, sha: &str) -> Option<xenify_hub_core::Download> {
    let m: xenify_hub_core::Manifest = serde_json::from_str(list["manifest"].as_str()?).ok()?;
    m.products
        .into_iter()
        .flat_map(|p| p.releases)
        .flat_map(|r| r.downloads)
        .find(|d| d.sha256.eq_ignore_ascii_case(sha))
}

#[derive(Deserialize)]
struct Report {
    /// The Hub's own id for this computer (random, made once).
    computer: String,
    name: String,
    /// Product id → installed version.
    products: std::collections::BTreeMap<String, String>,
}

/// A Hub tells the server what this computer has installed.
async fn report(State(s): State<Server>, h: HeaderMap, Json(r): Json<Report>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if r.computer.is_empty() || r.computer.len() > 64 || r.products.len() > 64 {
        return err(StatusCode::BAD_REQUEST, "bad_report", "that isn't a Hub's report");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let products = serde_json::to_string(&r.products).expect("serialises");
    let name = crate::clean_name(&r.name);
    match c
        .execute(
            "INSERT INTO hub_computers (id, name, user_id, products, seen) VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (id) DO UPDATE SET name = $2, user_id = $3, products = $4, seen = now()",
            &[&r.computer, &name, &me.id, &products],
        )
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// Admin › Apps & updates: how many computers have each version, this
/// server's version and whether a newer one is out.
async fn status(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.updates").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    // Computers not heard from in 60 days don't count.
    let rows = c
        .query("SELECT products FROM hub_computers WHERE seen > now() - interval '60 days'", &[])
        .await
        .unwrap_or_default();
    let mut products: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u32>> = Default::default();
    for r in &rows {
        let p: std::collections::BTreeMap<String, String> =
            serde_json::from_str(r.get::<_, &str>(0)).unwrap_or_default();
        for (id, v) in p {
            *products.entry(id).or_default().entry(v).or_default() += 1;
        }
    }
    let set = settings(&c).await;
    // What the release list offers this server, on its channel.
    let mut latest = serde_json::Map::new();
    if let Ok(list) = current_list(&s, set.source()).await
        && let Some(m) =
            list["manifest"].as_str().and_then(|t| serde_json::from_str::<xenify_hub_core::Manifest>(t).ok())
    {
        let (os, arch) = xenify_hub_core::platform();
        for p in &m.products {
            let local = xenify_hub_core::Local::default();
            let policy = set.policy(String::new());
            let e = xenify_hub_core::choose::effective(&local, Some(&policy), &p.id);
            if let Some(r) = xenify_hub_core::choose::pick(p, &e.choice, os, arch) {
                latest.insert(p.id.clone(), serde_json::json!(r.version));
            }
        }
    }
    Json(serde_json::json!({
        "server": crate::VERSION,
        "computers": rows.len(),
        "products": products,
        "latest": latest,
        "update_requested": db::setting(&c, "hub_update_server").await.ok().flatten(),
        "hosting": s.hosted_apps().is_some(),
        "self_updating": *s.self_updating.lock().expect("not poisoned"),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct UpdateServer {
    version: String,
}

/// "Update server now": the server's service, or the Hub on this computer,
/// picks it up.
async fn update_server(State(s): State<Server>, h: HeaderMap, Json(u): Json<UpdateServer>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.updates").await {
        return e;
    }
    if semver::Version::parse(&u.version).is_err() {
        return err(StatusCode::BAD_REQUEST, "bad_version", "that isn't a version");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    match db::set_setting(&c, "hub_update_server", &u.version).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

// ---- Updating itself, under the service ---------------------------------

/// Under the service: keeps this server up to date into `versions`,
/// following Admin › Apps & updates for the server (and "Update server
/// now"). The service starts the new version beside this one, which then
/// finishes and stops.
pub fn update_myself(s: &Server, versions: std::path::PathBuf) {
    *s.self_updating.lock().expect("not poisoned") = true;
    let s = s.clone();
    tokio::spawn(async move {
        // A moment after starting (tests: XENIFY_UPDATE_SOON).
        let first = if std::env::var_os("XENIFY_UPDATE_SOON").is_some() { 1 } else { 20 };
        tokio::time::sleep(Duration::from_secs(first)).await;
        let mut since = std::time::Instant::now() - Duration::from_secs(3600);
        loop {
            let asked = match s.pool.get().await {
                Ok(c) => db::setting(&c, "hub_update_server").await.ok().flatten().filter(|v| v != crate::VERSION),
                Err(_) => None,
            };
            if asked.is_some() || since.elapsed() > Duration::from_secs(30 * 60) {
                since = std::time::Instant::now();
                match update_once(&s, &versions, asked).await {
                    Ok(Some(v)) => println!("xenify-server: version {v} is ready; it takes over now"),
                    Ok(None) => {}
                    Err(e) => eprintln!("xenify-server: updating: {e}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(first.min(30))).await;
        }
    });
}

/// Installs the version this server should move to, if there is one.
async fn update_once(s: &Server, versions: &std::path::Path, asked: Option<String>) -> Result<Option<String>, String> {
    use xenify_hub_core::{Choice, UpdateMode, choose};
    let c = s.pool.get().await.map_err(|e| e.to_string())?;
    let set = settings(&c).await;
    let list = current_list(s, set.source()).await?;
    let signed: xenify_hub_core::Signed = serde_json::from_value(list).map_err(|_| "the release list can't be read")?;
    let seen = db::setting(&c, "hub_seen").await.ok().flatten();
    let m = xenify_hub_core::manifest::verify(&signed, &trusted_keys(s), seen.as_deref()).map_err(|e| e.to_string())?;
    db::set_setting(&c, "hub_seen", &m.published).await.map_err(|e| e.to_string())?;
    let Some(p) = m.product("server") else { return Ok(None) };
    let policy = set.policy(String::new());
    let mut e = choose::effective(&xenify_hub_core::Local::default(), Some(&policy), "server");
    if let Some(v) = asked {
        e.choice = Choice::Fixed(v);
    } else {
        use chrono::Timelike;
        if e.mode != UpdateMode::Automatic || !choose::in_window(e.window, chrono::Local::now().hour() as u8) {
            return Ok(None);
        }
    }
    let (os, arch) = xenify_hub_core::platform();
    let store = xenify_hub_core::Store::new(versions);
    let Some(r) = choose::update(p, Some(crate::VERSION), &e, os, arch) else { return Ok(None) };
    if store.installed("server").is_some_and(|i| i.version == r.version) {
        return Ok(None);
    }
    let d = r.download(os, arch).cloned().ok_or("no download for this computer")?;
    let (version, dir) = (r.version.clone(), versions.join("downloads"));
    tokio::task::spawn_blocking(move || {
        let part = dir.join(format!("{}.part", d.sha256.chars().filter(char::is_ascii_hexdigit).collect::<String>()));
        let file = copy_checked(&d.url, d.size, &d.sha256, &dir, &part).map_err(|e| format!("the download: {e}"))?;
        let r = store.install_file("server", &version, &d, &file).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&file);
        r.map(|_| Some(version))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Xenify's release key, and builds of your own: their keys too (JSON
/// arrays of 32 bytes in the data folder's trusted-keys.json).
fn trusted_keys(s: &Server) -> Vec<[u8; 32]> {
    let mut keys = xenify_hub_core::TRUSTED.to_vec();
    if let Ok(t) = std::fs::read_to_string(s.data.join("trusted-keys.json"))
        && let Ok(more) = serde_json::from_str::<Vec<[u8; 32]>>(&t)
    {
        keys.extend(more);
    }
    keys
}

// ---- The apps for the browser -------------------------------------------
//
// Write, Draw, Calc, Plan and Chat as web pages (the website's build), one
// package in the release list ("web", for any computer). The server
// downloads it when an admin asks (Admin › Network), checks it against the
// signed list like the Hub does, and keeps it in its data folder (`web/`).
// After the server updates, the matching version is fetched the same way.

const WEB: (&str, &str) = ("any", "any");

/// The browser apps downloaded earlier, if there are any.
pub fn downloaded_web_apps(s: &Server) -> Option<std::path::PathBuf> {
    let store = xenify_hub_core::Store::new(s.data.join("web"));
    store.program("web")?.parent().map(std::path::Path::to_path_buf).filter(|d| d.is_dir())
}

/// The release of the browser apps for this server: the same version as
/// the server when there is one, else the newest on the server's channel.
fn web_release(m: &xenify_hub_core::Manifest, set: &Settings) -> Result<xenify_hub_core::Release, String> {
    let p = m.product("web").ok_or("Xenify's release list doesn't have the browser apps yet")?;
    if let Some(r) = p.releases.iter().find(|r| r.version == crate::VERSION && r.download(WEB.0, WEB.1).is_some()) {
        return Ok(r.clone());
    }
    let policy = set.policy(String::new());
    let e = xenify_hub_core::choose::effective(&xenify_hub_core::Local::default(), Some(&policy), "server");
    xenify_hub_core::choose::pick(p, &e.choice, WEB.0, WEB.1)
        .cloned()
        .ok_or_else(|| "no version of the browser apps matches this server's updates".into())
}

/// Puts the browser apps for this server in the data folder (if they
/// aren't the right version already) and returns their folder.
async fn fetch_web_apps(s: &Server) -> Result<std::path::PathBuf, String> {
    let c = s.pool.get().await.map_err(|e| e.to_string())?;
    let set = settings(&c).await;
    let list = current_list(s, set.source()).await?;
    let signed: xenify_hub_core::Signed = serde_json::from_value(list).map_err(|_| "the release list can't be read")?;
    let seen = db::setting(&c, "hub_seen").await.ok().flatten();
    let m = xenify_hub_core::manifest::verify(&signed, &trusted_keys(s), seen.as_deref()).map_err(|e| e.to_string())?;
    let r = web_release(&m, &set)?;
    let root = s.data.join("web");
    let store = xenify_hub_core::Store::new(&root);
    // A newer version put in place by hand (a test build) stays.
    let newer = |v: &str| matches!((semver::Version::parse(v), semver::Version::parse(&r.version)), (Ok(a), Ok(b)) if a > b);
    if store.installed("web").is_none_or(|i| i.version != r.version && !newer(&i.version)) {
        let d = r.download(WEB.0, WEB.1).cloned().ok_or("no download")?;
        let (version, dir) = (r.version.clone(), root.join("downloads"));
        tokio::task::spawn_blocking(move || {
            let part =
                dir.join(format!("{}.part", d.sha256.chars().filter(char::is_ascii_hexdigit).collect::<String>()));
            let file =
                copy_checked(&d.url, d.size, &d.sha256, &dir, &part).map_err(|e| format!("the download: {e}"))?;
            let r = store.install_file("web", &version, &d, &file).map_err(|e| e.to_string());
            let _ = std::fs::remove_file(&file);
            r.map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    downloaded_web_apps(s).ok_or_else(|| "the browser apps didn't unpack".into())
}

/// How big the browser apps' download is (for the button), when the
/// release list can be had.
pub(crate) async fn web_apps_size(s: &Server) -> Option<u64> {
    let c = s.pool.get().await.ok()?;
    let set = settings(&c).await;
    let list = current_list(s, set.source()).await.ok()?;
    let signed: xenify_hub_core::Signed = serde_json::from_value(list).ok()?;
    let m = xenify_hub_core::manifest::verify(&signed, &trusted_keys(s), None).ok()?;
    web_release(&m, &set).ok()?.download(WEB.0, WEB.1).map(|d| d.size)
}

pub(crate) fn web_job(s: &Server) -> String {
    s.web_job.lock().expect("not poisoned").clone()
}

/// Downloads the browser apps in the background, then offers them at the
/// server's address.
pub(crate) fn download_web_apps(s: &Server) {
    {
        let mut job = s.web_job.lock().expect("not poisoned");
        if *job == "downloading" {
            return;
        }
        *job = "downloading".into();
    }
    let s = s.clone();
    tokio::spawn(async move {
        let r = async {
            let dir = fetch_web_apps(&s).await?;
            let moved = s.apps.lock().expect("not poisoned").as_ref() != Some(&dir);
            s.host_apps(dir);
            // New listeners serve the new folder.
            if moved { s.run_network(s.net().await).await } else { Ok(()) }
        }
        .await;
        if let Err(e) = &r {
            eprintln!("xenify-server: the browser apps: {e}");
        }
        *s.web_job.lock().expect("not poisoned") = r.err().unwrap_or_default();
    });
}

/// At start, with no browser apps beside the program: fetches them, and
/// tries again every few hours until it has them (offline, or no release
/// for this version yet).
pub fn get_web_apps(s: &Server) {
    let s = s.clone();
    tokio::spawn(async move {
        loop {
            download_web_apps(&s);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if *s.web_job.lock().expect("not poisoned") != "downloading" {
                    break;
                }
            }
            if s.web_job.lock().expect("not poisoned").is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
        }
    });
}

/// At start: browser apps downloaded for an older server version are
/// brought up to this one.
pub fn catch_up_web_apps(s: &Server) {
    let store = xenify_hub_core::Store::new(s.data.join("web"));
    if store.installed("web").is_some_and(|i| i.version != crate::VERSION) {
        download_web_apps(s);
    }
}
