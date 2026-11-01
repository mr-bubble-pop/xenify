//! What the Hub's window shows and does: the release list (checked against
//! Xenify's signature), what's installed here, the person's choices or the
//! server's policy, and downloads in the background.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenify_hub_core::choose::{self, Local};
use xenify_hub_core::manifest::{self, Signed};
use xenify_hub_core::{Choice, Manifest, Policy, Store, UpdateMode};
use crate::selfupdate;

/// Xenify's release list (published by .github/workflows/release.yml).
pub const SOURCE: &str = xenify_hub_core::RELEASES;

/// Xenify's release keys (the public halves). Builds made elsewhere can add
/// theirs in `trusted-keys.json` beside the Hub's settings.
const TRUSTED: &[[u8; 32]] = xenify_hub_core::TRUSTED;

/// The Hub's data folder (`$XENIFY_DATA_DIR/hub`, else the platform's).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XENIFY_DATA_DIR") {
        return PathBuf::from(d).join("hub");
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").filter(|_| cfg!(windows)).map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                if cfg!(target_os = "macos") {
                    PathBuf::from(h).join("Library/Application Support")
                } else {
                    PathBuf::from(h).join(".local/share")
                }
            })
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("xenify").join("hub")
}

/// The server this Hub is signed in to.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
    pub token: String,
    pub name: String,
    pub user: String,
}

/// Kept in `hub.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// This computer, to the server (random, made once).
    pub computer: String,
    /// The person's choices, by product.
    pub local: BTreeMap<String, Local>,
    /// The newest release list seen (`published`): an older one is refused.
    pub seen: Option<String>,
    /// Another release list (a mirror, a build of your own).
    pub source: Option<String>,
    pub server: Option<Server>,
    /// The server running on this computer, started by the Hub: its
    /// program arguments (to start a new version the same way).
    pub server_args: Vec<String>,
    /// Don't start the installed server when the Hub starts (it does
    /// unless this is turned off in Settings).
    pub server_stays_off: bool,
    /// The password of the server's own database (made once, kept here).
    pub pg_password: String,
    /// Start the AI runner (LocalAI) when the Hub starts.
    pub localai_autostart: bool,
    /// Starting with the computer was turned on once, the first time the
    /// Hub ran (so updates happen in the background); after that it's the
    /// person's choice.
    pub autostart_done: bool,
    /// Settings › "Channel for all apps": every app was moved to it, and an
    /// app with no choice of its own yet starts on it (None: Release).
    pub all_apps: Option<xenify_hub_core::Channel>,
}

impl Saved {
    /// The person's choice for `product` (the all-apps channel if it has none).
    fn local(&self, product: &str) -> Local {
        self.local.get(product).cloned().unwrap_or_else(|| self.new_local())
    }

    fn new_local(&self) -> Local {
        Local { choice: Choice::Channel(self.all_apps.unwrap_or(xenify_hub_core::Channel::Release)), ..Default::default() }
    }

    fn local_mut(&mut self, product: &str) -> &mut Local {
        let fresh = self.new_local();
        self.local.entry(product.to_string()).or_insert(fresh)
    }
}

/// A download in progress.
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub version: String,
    pub state: &'static str,
    pub done: u64,
    pub total: u64,
    pub message: String,
}

#[derive(Default)]
pub struct Inner {
    pub saved: Saved,
    pub list: Option<Manifest>,
    /// What the list says beyond its products (the oldest safe versions).
    pub limits: manifest::Limits,
    pub policy: Option<Policy>,
    /// The server keeps copies of downloads.
    pub mirror: bool,
    /// The server's admin asked for this server version.
    pub update_server: Option<String>,
    pub jobs: BTreeMap<String, Job>,
    pub checking: bool,
    pub error: String,
    /// Restart the app to use the new version.
    pub ready: BTreeMap<String, String>,
    /// The server starts with the computer (a system service), as last seen.
    pub service: Option<bool>,
    /// Installing or removing the service.
    pub service_busy: bool,
    /// The server and the AI runner are running, as last looked (by
    /// `look`, in the background: the window never waits on it).
    pub server_up: bool,
    pub ai_up: bool,
    /// Signing in to a server, password right: the second step.
    pub step: Option<Step>,
    /// A newer Xenify Hub (it updates itself).
    pub hub_update: Option<HubUpdate>,
}

/// A newer Xenify Hub than this one.
#[derive(Clone, Debug)]
pub struct HubUpdate {
    pub version: String,
    /// Downloaded and checked against the signed list: ready to put in place.
    pub file: Option<PathBuf>,
    /// This copy can't replace itself (the .deb, a folder it can't write):
    /// the window offers the download page.
    pub by_hand: bool,
}

/// A sign-in waiting for its second step (a code, or setting one up).
#[derive(Default)]
pub struct Step {
    pub url: String,
    pub name: String,
    pub username: String,
    pub ticket: String,
    /// What the server answered: `methods`, `setup`, `setup_methods`.
    pub step: Value,
    /// The authenticator key being set up, kept so it doesn't change once scanned.
    pub key: Option<Value>,
}

#[derive(Clone)]
pub struct Hub {
    pub inner: Arc<Mutex<Inner>>,
    pub dir: PathBuf,
}

/// For one address: a server trusted here by its own certificate is
/// checked against that certificate's authority, and only for its own
/// address; every other site against the usual authorities.
fn agent(timeout: u64, url: &str) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .tls_config(xenify_platform::trust::ureq_tls_for(url))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get_json(url: &str, token: Option<&str>) -> Result<Value, String> {
    get_json_within(url, token, 30)
}

fn get_json_within(url: &str, token: Option<&str>, timeout: u64) -> Result<Value, String> {
    let mut req = agent(timeout, url).get(url).header("Accept", "application/json");
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let mut r = req.call().map_err(|e| format!("couldn't reach {url}: {e}"))?;
    let status = r.status().as_u16();
    let text = r
        .body_mut()
        .with_config()
        .limit(16 << 20)
        .read_to_string()
        .map_err(|_| format!("{url} sent something unexpected"))?;
    let v: Value = serde_json::from_str(&text).map_err(|_| format!("{url} sent something unexpected"))?;
    if status >= 400 {
        return Err(v["error"].as_str().unwrap_or("the server refused").to_string());
    }
    Ok(v)
}

fn post_json(url: &str, token: Option<&str>, body: &Value) -> Result<Value, String> {
    post_coded(url, token, body).map_err(|(_, e)| e)
}

/// As post_json, with the server's error code (`wrong_code`, …) beside the message.
fn post_coded(url: &str, token: Option<&str>, body: &Value) -> Result<Value, (String, String)> {
    let mut req = agent(30, url).post(url).header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let mut r = req
        .send(serde_json::to_vec(body).expect("serialises"))
        .map_err(|e| (String::new(), format!("couldn't reach the server: {e}")))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 {
        let message = v["error"].as_str().map(sentence).unwrap_or_else(|| "The server refused.".into());
        return Err((v["code"].as_str().unwrap_or("").to_string(), message));
    }
    Ok(v)
}

/// Where the AI runner listens (this computer only).
pub const LOCALAI_PORT: u16 = 8080;

/// Whether the program whose id is in `pid_file` is running (and is `name`).
fn running(pid_file: &std::path::Path, name: &str) -> bool {
    std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|pid| pid.trim().parse::<u32>().ok())
        .is_some_and(|pid| process_named(pid, name))
}

/// Process `pid` is running `name` (its program's file name). Asked of
/// the system directly: no program started (on Windows `tasklist` took a
/// good part of a second each time).
#[cfg(target_os = "linux")]
fn process_named(pid: u32, name: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.contains(name))
}

#[cfg(target_os = "macos")]
fn process_named(pid: u32, name: &str) -> bool {
    xenify_hub_core::command("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(name))
}

#[cfg(windows)]
fn process_named(pid: u32, name: &str) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    // SAFETY: a handle opened here and closed here; the buffer's length is passed.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        ok && String::from_utf16_lossy(&buf[..len as usize]).contains(name)
    }
}

/// This computer's address on its network (the one its traffic leaves
/// from). Nothing is sent: a UDP "connection" only picks the route.
fn lan_address() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("1.1.1.1:53").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

/// The installed products that need `id`.
fn m_needs(list: Option<&Manifest>, id: &str, store: &Store) -> Vec<String> {
    list.into_iter()
        .flat_map(|m| m.products.iter())
        .filter(|p| p.requires.iter().any(|r| r == id) && store.installed(&p.id).is_some())
        .map(|p| p.name.clone())
        .collect()
}

/// The host of an address (no port, no brackets).
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("").to_string();
    }
    authority.split(':').next().unwrap_or("").to_string()
}

/// Opens a web address in the person's browser (only http and https; on
/// Windows not through cmd, whose `start` would run what follows an `&`).
fn open_web(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("that isn't a web address".into());
    }
    #[cfg(target_os = "linux")]
    let r = xenify_hub_core::command("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let r = xenify_hub_core::command("open").arg(url).spawn();
    #[cfg(windows)]
    let r = xenify_hub_core::command("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// Stops the program whose id is in `pid_file` (if it's still `name`).
fn stop_pid(pid_file: &std::path::Path, name: &str) {
    if !running(pid_file, name) {
        return;
    }
    if let Ok(id) = std::fs::read_to_string(pid_file) {
        #[cfg(unix)]
        let _ = xenify_hub_core::command("kill").args(["-TERM", id.trim()]).status();
        #[cfg(windows)]
        let _ = xenify_hub_core::command("taskkill").args(["/PID", id.trim()]).status();
    }
    let _ = std::fs::remove_file(pid_file);
}

/// This computer, or one on its own network (private, link-local and
/// Tailscale-style addresses): every address the name has.
fn local_host(host: &str) -> bool {
    use std::net::{IpAddr, ToSocketAddrs};
    let addrs: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => (host, 0).to_socket_addrs().map(|a| a.map(|s| s.ip()).collect()).unwrap_or_default(),
    };
    !addrs.is_empty() && addrs.iter().all(local_ip)
}

fn local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            let [a, b, ..] = v.octets();
            v.is_loopback() || v.is_private() || v.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        std::net::IpAddr::V6(v) => {
            let first = v.segments()[0];
            v.is_loopback()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v.to_ipv4_mapped().is_some_and(|m| local_ip(&std::net::IpAddr::V4(m)))
        }
    }
}

/// Plain HTTP to a computer that isn't on this network.
fn plain(url: &str) -> bool {
    url.starts_with("http://") && !local_host(&host_of(url))
}

/// What applies to `product`: the person's choice or the server's, except
/// that a fixed version older than the list's oldest safe one becomes the
/// newest release (it's never installed, whatever the policy says).
fn effective(i: &Inner, product: &str) -> choose::Effective {
    let mut e = choose::effective(&i.saved.local(product), i.policy.as_ref(), product);
    if let Choice::Fixed(v) = &e.choice
        && !manifest::safe(&i.limits, product, v)
    {
        e.choice = Choice::Channel(xenify_hub_core::Channel::Release);
    }
    e
}

fn sentence(s: &str) -> String {
    let mut c = s.chars();
    let first = c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default();
    let mut out = first + c.as_str();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

impl Hub {
    pub fn new(dir: PathBuf) -> Self {
        let mut saved: Saved = std::fs::read_to_string(dir.join("hub.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if saved.computer.is_empty() {
            let mut b = [0u8; 12];
            let _ = getrandom::fill(&mut b);
            saved.computer = b.iter().map(|x| format!("{x:02x}")).collect();
        }
        let hub = Self { inner: Arc::new(Mutex::new(Inner { saved, ..Default::default() })), dir };
        hub.load_list();
        hub.save();
        hub
    }

    /// The release list last fetched, checked again: the apps show at once,
    /// before the Hub has heard from anywhere (a server that's off can take
    /// a while to give up on).
    fn load_list(&self) {
        let Some(signed) = std::fs::read_to_string(self.dir.join("list.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Signed>(&t).ok())
        else {
            return;
        };
        let seen = self.inner.lock().unwrap().saved.seen.clone();
        if let Ok(m) = manifest::verify(&signed, &self.trusted(), seen.as_deref()) {
            let mut i = self.inner.lock().unwrap();
            i.limits = manifest::limits(&signed.manifest);
            i.list = Some(m);
        }
    }

    pub fn store(&self) -> Store {
        Store::new(self.dir.clone())
    }

    /// True the first time the Hub runs on this computer (once).
    pub fn first_start(&self) -> bool {
        let first = !std::mem::replace(&mut self.inner.lock().unwrap().saved.autostart_done, true);
        if first {
            self.save();
        }
        first
    }

    fn save(&self) {
        let saved = self.inner.lock().unwrap().saved.clone();
        let _ = std::fs::create_dir_all(&self.dir);
        let tmp = self.dir.join("hub.json.part");
        // It holds the server sign-in's token and the database's password:
        // readable by this user only (as server.json and server.toml are).
        let written = (|| {
            use std::io::Write;
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
            o.open(&tmp)?.write_all(serde_json::to_string_pretty(&saved).expect("serialises").as_bytes())?;
            // A file kept from before this was made 0600.
            #[cfg(unix)]
            std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
            std::io::Result::Ok(())
        })();
        if written.is_ok() {
            let _ = std::fs::rename(tmp, self.dir.join("hub.json"));
        }
    }

    fn trusted(&self) -> Vec<[u8; 32]> {
        let mut keys = TRUSTED.to_vec();
        // Builds of your own: their keys, as JSON arrays of 32 bytes.
        if let Ok(t) = std::fs::read_to_string(self.dir.join("trusted-keys.json"))
            && let Ok(more) = serde_json::from_str::<Vec<[u8; 32]>>(&t)
        {
            keys.extend(more);
        }
        keys
    }

    /// Fetches the policy (when signed in to a server) and the release list,
    /// checking its signature.
    pub fn check(&self) -> Result<(), String> {
        let (server, source, seen) = {
            let mut i = self.inner.lock().unwrap();
            i.checking = true;
            (i.saved.server.clone(), i.saved.source.clone(), i.saved.seen.clone())
        };
        // Said once the list is in: the server's settings couldn't be read.
        let mut note = None;
        let out: Result<(), String> = (|| {
            let own = source.unwrap_or_else(|| SOURCE.to_string());
            // The server's policy and the usual list at the same time: a
            // server that's off or away doesn't hold up the list (and only
            // gets 10 seconds).
            let list = std::thread::spawn(move || get_json(&own, None));
            let policy = server.as_ref().map(|s| get_json_within(&format!("{}/v1/hub/policy", s.url), Some(&s.token), 10));
            let mut mirrored = None;
            if let (Some(s), Some(policy)) = (&server, policy) {
                match policy {
                    Ok(p) => {
                        let policy: Option<Policy> = serde_json::from_value(p["policy"].clone()).ok();
                        if policy.is_none() && !p["policy"].is_null() {
                            note = Some(format!(
                                "{}'s update settings need a newer Xenify Hub, so your own choices apply for now. Update the Hub",
                                p["policy"]["server"].as_str().unwrap_or(&s.url)
                            ));
                        }
                        let mirror = p["mirror"].as_bool().unwrap_or(false);
                        let mut i = self.inner.lock().unwrap();
                        i.policy = policy;
                        i.mirror = mirror;
                        i.update_server = p["update_server"].as_str().map(String::from);
                        if mirror {
                            mirrored = Some((format!("{}/v1/hub/manifest", s.url), s.token.clone()));
                        }
                    }
                    // Signed out elsewhere: the person's own choices again.
                    Err(e) if e.contains("sign in") => {
                        xenify_hub_core::signin::clear();
                        let mut i = self.inner.lock().unwrap();
                        i.saved.server = None;
                        i.policy = None;
                        i.mirror = false;
                    }
                    Err(_) => {}
                }
            }
            let v = match mirrored {
                Some((from, token)) => get_json(&from, Some(&token))?,
                None => list.join().unwrap_or_else(|_| Err("stopped".into()))?,
            };
            let signed: Signed = serde_json::from_value(v).map_err(|_| "the release list can't be read".to_string())?;
            let m = manifest::verify(&signed, &self.trusted(), seen.as_deref()).map_err(|e| e.to_string())?;
            // Kept for the next start (checked again then).
            if let Ok(t) = serde_json::to_string(&signed) {
                let tmp = self.dir.join("list.json.part");
                if std::fs::write(&tmp, t).is_ok() {
                    let _ = std::fs::rename(&tmp, self.dir.join("list.json"));
                }
            }
            let mut i = self.inner.lock().unwrap();
            i.saved.seen = Some(m.published.clone());
            i.limits = manifest::limits(&signed.manifest);
            i.list = Some(m);
            Ok(())
        })();
        {
            let mut i = self.inner.lock().unwrap();
            i.checking = false;
            i.error = out.as_ref().err().map(|e| sentence(e)).or(note.as_deref().map(sentence)).unwrap_or_default();
        }
        self.save();
        self.report();
        out
    }

    /// Tells the server what's installed here (for Admin › Apps & updates).
    fn report(&self) {
        let (server, computer, products) = {
            let i = self.inner.lock().unwrap();
            let store = self.store();
            let mut products: BTreeMap<String, String> = i
                .list
                .iter()
                .flat_map(|m| m.products.iter())
                .filter_map(|p| store.installed(&p.id).map(|x| (p.id.clone(), x.version)))
                .collect();
            // The Hub itself too (Admin › Apps & updates shows who has an old one).
            products.insert("hub".into(), selfupdate::VERSION.into());
            (i.saved.server.clone(), i.saved.computer.clone(), products)
        };
        if let Some(s) = server {
            let name = std::env::var("HOSTNAME")
                .ok()
                .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()))
                .or_else(|| std::env::var("COMPUTERNAME").ok())
                .unwrap_or_default();
            let _ = post_json(
                &format!("{}/v1/hub/report", s.url),
                Some(&s.token),
                &json!({ "computer": computer, "name": name, "products": products }),
            );
        }
    }

    /// Everything the window shows.
    pub fn state(&self) -> Value {
        let (running, localai_running) = {
            let i = self.inner.lock().unwrap();
            (i.server_up, i.ai_up)
        };
        let admin = running.then(|| self.server_admin()).flatten();
        let setup_needed = running && self.own_server_needs_setup();
        let own_port = running.then(|| self.server_port()).flatten();
        let i = self.inner.lock().unwrap();
        let store = self.store();
        let (os, arch) = xenify_hub_core::platform();
        let products: Vec<Value> = i
            .list
            .iter()
            .flat_map(|m| m.products.iter())
            // The Hub itself isn't one of the things it installs.
            .filter(|p| p.id != "hub")
            .map(|p| {
                let e = effective(&i, &p.id);
                let installed = store.installed(&p.id);
                let to = choose::update(p, installed.as_ref().map(|x| x.version.as_str()), &e, os, arch);
                let versions: Vec<Value> = p
                    .releases
                    .iter()
                    .filter(|r| r.download(os, arch).is_some())
                    .map(|r| json!({ "version": r.version, "channel": r.channel, "date": r.date }))
                    .collect();
                json!({
                    "id": p.id, "name": p.name, "about": p.about, "component": p.component,
                    "installed": installed.as_ref().map(|x| &x.version),
                    "previous": installed.as_ref().and_then(|x| x.previous.as_ref()).map(|x| &x.version),
                    "to": to.map(|r| json!({ "version": r.version, "notes": r.notes })),
                    "choice": e.choice, "mode": e.mode, "locked": e.locked, "mode_locked": e.mode_locked, "allowed": e.allowed, "required": e.required,
                    "versions": versions,
                    "job": i.jobs.get(&p.id),
                    // Needed by something installed: not removed on its own.
                    "needed_by": m_needs(i.list.as_ref(), &p.id, &store),
                    "ready": i.ready.get(&p.id),
                })
            })
            .collect();
        json!({
            "products": products,
            "managed_by": i.policy.as_ref().map(|p| &p.server),
            "server": i.saved.server.as_ref().map(|s| json!({ "url": s.url, "name": s.name, "user": s.user })),
            "step": i.step.as_ref().map(|s| json!({
                "name": s.name, "methods": s.step["methods"], "setup": s.step["setup"], "setup_methods": s.step["setup_methods"],
            })),
            "checking": i.checking,
            "error": i.error,
            "listed": i.list.is_some(),
            "source": i.saved.source,
            "server_autostart": !i.saved.server_stays_off,
            "server_service": i.service.unwrap_or(false),
            "service_busy": i.service_busy,
            "server_running": running,
            "localai_running": localai_running,
            "localai": { "address": format!("http://127.0.0.1:{LOCALAI_PORT}/v1"), "models": self.dir.join("localai-models").display().to_string() },
            "localai_autostart": i.saved.localai_autostart,
            "all_apps": i.saved.all_apps.unwrap_or(xenify_hub_core::Channel::Release),
            // There's an AI runner for this computer (none for Windows).
            "localai_offered": i.list.as_ref().and_then(|m| m.product("localai"))
                .is_some_and(|p| p.releases.iter().any(|r| r.download(os, arch).is_some())),
            "server_admin": admin,
            // The server here is waiting for its first account (the Hub makes it).
            "server_setup": setup_needed,
            // Its address, to sign in to it when it was set up elsewhere.
            "own_server": own_port.map(|p| format!("http://127.0.0.1:{p}")),
            // What other computers type to sign in to the server here.
            "lan_address": running.then(lan_address).flatten(),
            "hub_version": selfupdate::VERSION,
            "hub_update": i.hub_update.as_ref().map(|u| json!({
                "version": u.version, "ready": u.file.is_some(),
                "download": u.by_hand.then_some(selfupdate::DOWNLOAD_PAGE),
            })),
        })
    }

    pub fn set_choice(&self, product: &str, choice: Option<Choice>, mode: Option<UpdateMode>) -> Result<(), String> {
        {
            let mut i = self.inner.lock().unwrap();
            let e = choose::effective(&i.saved.local(product), i.policy.as_ref(), product);
            if (choice.is_some() && e.locked) || (mode.is_some() && e.mode_locked) {
                return Err(format!("{} decides this", e.managed_by.unwrap_or_else(|| "The server".into())));
            }
            let l = i.saved.local_mut(product);
            if let Some(c) = choice {
                l.choice = c;
            }
            if let Some(m) = mode {
                l.mode = m;
            }
        }
        self.save();
        Ok(())
    }

    /// Settings › "Channel for all apps": moves every app (the Hub too) to
    /// `channel`, except the ones the server decides, and starts apps
    /// installed later on it. One app can still be set apart afterwards.
    pub fn set_all_channel(&self, channel: xenify_hub_core::Channel) {
        {
            let mut i = self.inner.lock().unwrap();
            i.saved.all_apps = Some(channel);
            let ids: Vec<String> = i.list.iter().flat_map(|m| m.products.iter()).map(|p| p.id.clone()).collect();
            for id in ids {
                if !choose::effective(&i.saved.local(&id), i.policy.as_ref(), &id).locked {
                    i.saved.local_mut(&id).choice = Choice::Channel(channel);
                }
            }
        }
        self.save();
    }

    /// Downloads and installs what the choice says, in the background (and
    /// the parts it needs that aren't here yet).
    pub fn install(&self, product: &str) -> Result<(), String> {
        let needs: Vec<String> = {
            let i = self.inner.lock().unwrap();
            i.list.as_ref().and_then(|m| m.product(product)).map(|p| p.requires.clone()).unwrap_or_default()
        };
        for n in needs {
            if self.store().installed(&n).is_none() {
                self.install_one(&n)?;
            }
        }
        self.install_one(product)
    }

    fn install_one(&self, product: &str) -> Result<(), String> {
        let (os, arch) = xenify_hub_core::platform();
        let (release, download, server, mirror) = {
            let mut i = self.inner.lock().unwrap();
            // Already on its way (checked and noted under one lock, so two
            // calls don't download into the same file).
            if i.jobs.get(product).is_some_and(|j| j.state == "downloading" || j.state == "installing") {
                return Ok(());
            }
            let m = i.list.as_ref().ok_or("the release list hasn't been fetched yet")?;
            let p = m.product(product).ok_or("that app isn't in the release list")?;
            let e = effective(&i, product);
            if !e.allowed {
                return Err(format!("{} doesn't allow it", e.managed_by.unwrap_or_default()));
            }
            let r = choose::pick(p, &e.choice, os, arch).ok_or("there's no version of it for this computer")?.clone();
            let d = r.download(os, arch).cloned().ok_or("there's no version of it for this computer")?;
            let job = Job {
                version: r.version.clone(),
                state: "downloading",
                done: 0,
                total: d.size,
                message: String::new(),
            };
            i.jobs.insert(product.to_string(), job);
            let (server, mirror) = (i.saved.server.clone(), i.mirror);
            (r, d, server, mirror)
        };
        let hub = self.clone();
        let product = product.to_string();
        std::thread::spawn(move || {
            let url = match (&server, mirror) {
                (Some(s), true) => format!("{}/v1/hub/files/{}", s.url, download.sha256),
                _ => download.url.clone(),
            };
            let token = server.filter(|_| mirror).map(|s| s.token);
            let got = hub.fetch(&product, &url, token.as_deref(), &download).and_then(|file| {
                hub.set_job(&product, "installing", download.size, "");
                let r = hub
                    .store()
                    .install_file(&product, &release.version, &download, &file)
                    .map_err(|e| sentence(&e.to_string()));
                // A file that didn't match is fetched afresh next time. The
                // server's is kept: setting up its service checks it again.
                if product == "server" && r.is_ok() {
                    hub.keep_server_archive(&file, &download);
                } else {
                    let _ = std::fs::remove_file(&file);
                }
                r
            });
            let installed = got.is_ok();
            match got {
                Ok(now) => {
                    hub.place(&product);
                    let mut i = hub.inner.lock().unwrap();
                    i.jobs.remove(&product);
                    // An update: the app may be open, on the version before.
                    if now.previous.is_some() {
                        i.ready.insert(product.clone(), release.version.clone());
                    }
                }
                Err(e) => hub.set_job(&product, "failed", 0, &e),
            }
            hub.report();
            if product == "server" && installed {
                hub.restart_server();
                // Just installed: it starts now (as with the Hub from now on).
                hub.start_server_if_wanted();
                hub.look();
            }
        });
        Ok(())
    }

    fn set_job(&self, product: &str, state: &'static str, done: u64, message: &str) {
        if let Some(j) = self.inner.lock().unwrap().jobs.get_mut(product) {
            j.state = state;
            j.done = done.max(j.done);
            j.message = message.into();
        }
    }

    /// Downloads into `downloads/<sha>.part` with progress, carrying on
    /// from what an earlier, interrupted try got.
    fn fetch(
        &self,
        product: &str,
        url: &str,
        token: Option<&str>,
        d: &xenify_hub_core::Download,
    ) -> Result<PathBuf, String> {
        use std::io::{Read, Write};
        let dir = self.dir.join("downloads");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let part = dir.join(format!("{}.part", d.sha256.chars().filter(char::is_ascii_hexdigit).collect::<String>()));
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0).min(d.size);
        let mut req = agent(3600, url).get(url);
        if let Some(t) = token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        if have > 0 && have < d.size {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let mut r = req.call().map_err(|e| format!("Couldn't download it: {e}"))?;
        let status = r.status().as_u16();
        if status >= 400 {
            return Err(format!("Couldn't download it (the server answered {status})."));
        }
        // 206: the rest of it; 200: all of it again.
        let resumed = status == 206 && have > 0 && have < d.size;
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&part)
            .map_err(|e| e.to_string())?;
        let mut done = if resumed { have } else { 0 };
        let mut body = r.body_mut().with_config().limit(d.size + 1).reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("The download stopped: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if let Some(j) = self.inner.lock().unwrap().jobs.get_mut(product) {
                j.done = done;
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        Ok(part)
    }

    /// Points the app's Start menu / menu entry and command at the version
    /// in use (after installing or going back).
    /// Parts (the database, the shared spelling model) get no entry.
    fn place(&self, product: &str) {
        let Some(program) = self.store().program(product) else { return };
        let Some((name, about)) = ({
            let i = self.inner.lock().unwrap();
            match i.list.as_ref().and_then(|m| m.product(product)) {
                Some(p) if p.component => None,
                Some(p) => Some((p.name.clone(), p.about.clone())),
                None => Some((product.to_string(), String::new())),
            }
        }) else {
            return;
        };
        if let Err(e) = crate::shortcuts::add(product, &name, &about, &program) {
            self.inner.lock().unwrap().error = sentence(&e);
        }
    }

    /// Starts an installed app (the Hub doesn't wait for it).
    pub fn open(&self, product: &str) -> Result<(), String> {
        let program = self.store().program(product).ok_or("it isn't installed")?;
        let mut cmd = xenify_hub_core::command(&program);
        if product == "server" {
            let args = self.server_args()?;
            cmd.args(&args);
            // Its messages (the first-run setup link among them) go to a log.
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join("server.log"))
                .map_err(|e| e.to_string())?;
            cmd.stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
            let child = cmd.spawn().map_err(|e| format!("couldn't start it: {e}"))?;
            let _ = std::fs::write(self.dir.join("server.pid"), child.id().to_string());
            return Ok(());
        }
        if product == "localai" {
            // The AI runner for the server: this computer only; its models
            // in the Hub's folder (Admin › AI › its models folder).
            let models = self.dir.join("localai-models");
            std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
            cmd.args(["run", "--address", &format!("127.0.0.1:{LOCALAI_PORT}"), "--models-path"]).arg(&models);
            // It keeps work files where it starts.
            let work = self.dir.join("localai-work");
            std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
            cmd.current_dir(work);
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join("localai.log"))
                .map_err(|e| e.to_string())?;
            cmd.stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
            let child = cmd.spawn().map_err(|e| format!("couldn't start it: {e}"))?;
            let _ = std::fs::write(self.dir.join("localai.pid"), child.id().to_string());
            return Ok(());
        }
        cmd.spawn().map(|_| ()).map_err(|e| format!("couldn't start it: {e}"))
    }

    /// Looks again whether the server and the AI runner are running (and,
    /// the first time, whether the server starts with the computer).
    pub fn look(&self) {
        if self.inner.lock().unwrap().service.is_none() {
            self.refresh_service();
        }
        let (server, ai) = (self.server_running(), self.localai_running());
        let mut i = self.inner.lock().unwrap();
        i.server_up = server;
        i.ai_up = ai;
    }

    /// Keeps `look`ing every two seconds, for the window.
    pub fn keep_looking(&self) {
        let hub = self.clone();
        std::thread::spawn(move || {
            loop {
                hub.look();
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }

    /// Fetches the release list again in the background (after signing
    /// in or out): the window shows the result when it's in.
    fn check_soon(&self) {
        let hub = self.clone();
        std::thread::spawn(move || {
            let _ = hub.check();
        });
    }

    /// The AI runner started by the Hub is running.
    pub fn localai_running(&self) -> bool {
        running(&self.dir.join("localai.pid"), "local-ai")
    }

    /// How the server is started. The first time, with its own database
    /// (PostgreSQL, installed with it); which is started first each time.
    /// Someone using their own database edits `server_args` in hub.json.
    fn server_args(&self) -> Result<Vec<String>, String> {
        let (mut args, mut password) = {
            let i = self.inner.lock().unwrap();
            (i.saved.server_args.clone(), i.saved.pg_password.clone())
        };
        let own_db =
            args.is_empty() || args.iter().any(|a| a.contains(&format!("127.0.0.1:{}", xenify_hub_core::pg::PORT)));
        if own_db {
            let program =
                self.store().program("postgres").ok_or("the server's database (PostgreSQL) isn't installed")?;
            let pg = xenify_hub_core::pg::Pg::new(&program, self.dir.join("postgres-data"))
                .ok_or("PostgreSQL isn't where it should be")?;
            if password.is_empty() {
                let mut b = [0u8; 18];
                getrandom::fill(&mut b).map_err(|e| e.to_string())?;
                password = b.iter().map(|x| format!("{x:02x}")).collect();
                self.inner.lock().unwrap().saved.pg_password = password.clone();
                self.save();
            }
            pg.init(&password)?;
            pg.start()?;
            pg.create_db(&password)?;
            if args.is_empty() {
                args = vec![
                    "--database".into(),
                    xenify_hub_core::pg::Pg::url(&password),
                    "--data".into(),
                    self.dir.join("server-data").display().to_string(),
                    "serve".into(),
                ];
                self.inner.lock().unwrap().saved.server_args = args.clone();
                self.save();
            }
        }
        Ok(args)
    }

    /// The server started by the Hub is running.
    pub fn server_running(&self) -> bool {
        if self.service() {
            return self
                .server_port()
                .is_some_and(|p| get_json_within(&format!("http://127.0.0.1:{p}/v1/info"), None, 3).is_ok());
        }
        running(&self.dir.join("server.pid"), "xenify-server")
    }

    /// The server's data folder (where it writes address.json).
    fn server_data(&self) -> PathBuf {
        let args = self.inner.lock().unwrap().saved.server_args.clone();
        args.iter()
            .position(|a| a == "--data")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dir.join("server-data"))
    }

    /// What the server wrote about itself: its ports and, until it's set
    /// up, the setup link.
    fn server_address(&self) -> Option<Value> {
        serde_json::from_str(&std::fs::read_to_string(self.server_data().join("address.json")).ok()?).ok()
    }

    fn server_port(&self) -> Option<u64> {
        self.server_address()?["http_port"].as_u64()
    }

    /// The server starts with the computer (as last seen).
    pub fn service(&self) -> bool {
        self.inner.lock().unwrap().service.unwrap_or(false)
    }

    /// Asks the installed server whether its service is installed.
    pub fn refresh_service(&self) {
        let on = self.store().program("server").is_some_and(|p| {
            xenify_hub_core::command(p).args(["service", "status"]).output().is_ok_and(|o| o.status.success())
        });
        self.inner.lock().unwrap().service = Some(on);
    }

    /// Where the installed server's download is kept (for setting up its
    /// service, which checks it again).
    fn kept_archive(&self, d: &xenify_hub_core::Download) -> PathBuf {
        let sha: String = d.sha256.chars().filter(char::is_ascii_hexdigit).collect();
        self.dir.join("downloads").join(format!("server-{sha}.archive"))
    }

    /// Keeps the server's download that was just installed (and no other).
    fn keep_server_archive(&self, file: &std::path::Path, d: &xenify_hub_core::Download) {
        let keep = self.kept_archive(d);
        if let Ok(entries) = std::fs::read_dir(self.dir.join("downloads")) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("server-") && name.ends_with(".archive") && e.path() != keep {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        if std::fs::rename(file, &keep).is_err() {
            let _ = std::fs::remove_file(file);
        }
    }

    /// The installed server's download, as the signed release list names
    /// it: the one kept, or downloaded again.
    fn server_download(&self) -> Result<(PathBuf, xenify_hub_core::Download), String> {
        let installed = self.store().installed("server").ok_or("the server isn't installed")?;
        if self.inner.lock().unwrap().list.is_none() {
            self.check()?;
        }
        let (os, arch) = xenify_hub_core::platform();
        let (d, server, mirror) = {
            let i = self.inner.lock().unwrap();
            let r = i
                .list
                .as_ref()
                .and_then(|m| m.product("server"))
                .and_then(|p| p.releases.iter().find(|r| r.version == installed.version))
                .ok_or(
                    "this version of the server isn't in the release list (or isn't safe any more): update it first",
                )?;
            let d = r.download(os, arch).cloned().ok_or("there's no version of it for this computer")?;
            (d, i.saved.server.clone(), i.mirror)
        };
        let keep = self.kept_archive(&d);
        if manifest::check_path(&d, &keep).is_ok() {
            return Ok((keep, d));
        }
        let url = match (&server, mirror) {
            (Some(s), true) => format!("{}/v1/hub/files/{}", s.url, d.sha256),
            _ => d.url.clone(),
        };
        let token = server.filter(|_| mirror).map(|s| s.token);
        let file = self.fetch("server", &url, token.as_deref(), &d)?;
        if manifest::check_path(&d, &file).is_err() {
            let _ = std::fs::remove_file(&file);
            return Err(manifest::Refused::WrongFile.to_string());
        }
        self.keep_server_archive(&file, &d);
        Ok((keep, d))
    }

    /// `server.toml`: what the service starts the server with (the
    /// database's password is in it: readable by this user only).
    fn write_server_config(&self) -> Result<PathBuf, String> {
        let args = self.server_args()?;
        let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let q = |s: &str| serde_json::to_string(s).expect("a string");
        let mut text = String::from("# Made by Xenify Hub: what the server's service starts it with.\n");
        if let Some(d) = after("--database") {
            text += &format!("database = {}\n", q(&d));
        }
        text += &format!("data = {}\n", q(&self.server_data().display().to_string()));
        let own_db = args.iter().any(|a| a.contains(&format!("127.0.0.1:{}", xenify_hub_core::pg::PORT)));
        let (ai, store) = (self.inner.lock().unwrap().saved.localai_autostart, self.store());
        if ai && let Some(program) = store.program("localai") {
            let models = self.dir.join("localai-models");
            text += &format!(
                "\n[[also]]\nprogram = {}\nargs = [\"run\", \"--address\", \"127.0.0.1:{LOCALAI_PORT}\", \"--models-path\", {}]\ndir = {}\n",
                q(&program.display().to_string()),
                q(&models.display().to_string()),
                q(&self.dir.join("localai-work").display().to_string())
            );
        }
        if own_db && let Some(program) = store.program("postgres") {
            text += &format!(
                "\n[postgres]\nprogram = {}\ndata = {}\n",
                q(&program.display().to_string()),
                q(&self.dir.join("postgres-data").display().to_string())
            );
        }
        // Readable by this user only from the start (then put in place).
        let path = self.dir.join("server.toml");
        let part = self.dir.join("server.toml.part");
        xenify_hub_core::pg::write_private(&part, text.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Starts the server with the computer (a system service; asks for the
    /// computer's password), or stops doing so. What runs as the
    /// administrator is never the copy in this person's folder: the
    /// server's download is checked again against the signed release list,
    /// as the administrator, in a folder only the administrator can change.
    pub fn set_service(&self, on: bool) -> Result<(), String> {
        let result = if on {
            let config = self.write_server_config()?;
            let (archive, d) = self.server_download()?;
            let mut args =
                vec!["--config".to_string(), config.display().to_string(), "service".into(), "install".into()];
            if !cfg!(windows) {
                // As this person: never as nobody (that would be root).
                let me = std::env::var("USER")
                    .ok()
                    .filter(|u| !u.trim().is_empty())
                    .ok_or("this computer doesn't say who you are (USER isn't set)")?;
                args.extend(["--user".into(), me]);
            }
            crate::elevate::run_verified(&archive, &d.sha256, &d.run, &args)
        } else {
            crate::elevate::remove_service()
        };
        self.refresh_service();
        result?;
        if on {
            // The service's server takes over (it shares the ports with
            // this one for a moment); then the Hub's copies stop.
            std::thread::sleep(Duration::from_secs(5));
            for (pid, name) in [("server.pid", "xenify-server"), ("localai.pid", "local-ai")] {
                let file = self.dir.join(pid);
                if running(&file, name)
                    && (name == "xenify-server" || self.inner.lock().unwrap().saved.localai_autostart)
                    && let Ok(id) = std::fs::read_to_string(&file)
                {
                    #[cfg(unix)]
                    let _ = xenify_hub_core::command("kill").args(["-TERM", id.trim()]).status();
                    #[cfg(windows)]
                    let _ = xenify_hub_core::command("taskkill").args(["/PID", id.trim()]).status();
                    let _ = std::fs::remove_file(file);
                }
            }
        } else {
            self.start_server_if_wanted();
        }
        self.look();
        Ok(())
    }

    /// Its admin pages: the first-run setup link while it hasn't been set
    /// up, else the admin address it printed.
    pub fn server_admin(&self) -> Option<String> {
        if let Some(a) = self.server_address()
            && let Some(port) = a["http_port"].as_u64()
        {
            return Some(format!("http://localhost:{port}{}", a["setup"].as_str().unwrap_or("/admin/")));
        }
        let log = std::fs::read_to_string(self.dir.join("server.log")).ok()?;
        let setup = log
            .lines()
            .rev()
            .find_map(|l| l.split_whitespace().find(|w| w.contains("/admin/#/setup/")).map(String::from));
        let listening = log.lines().rev().find_map(|l| {
            l.strip_prefix("xenify-server ").and_then(|r| r.split("listening on ").nth(1)).map(|a| a.trim().to_string())
        })?;
        let port = listening.rsplit(':').next()?.to_string();
        let admin = format!("http://localhost:{port}/admin/");
        // Set up since it printed the link: the ordinary admin pages.
        if setup.is_some() {
            let needed = get_json(&format!("http://127.0.0.1:{port}/v1/info"), None)
                .ok()
                .and_then(|v| v["setup_needed"].as_bool());
            if needed == Some(false) {
                return Some(admin);
            }
        }
        Some(setup.unwrap_or(admin))
    }

    /// The service's settings start the AI runner too.
    fn service_runs_ai(&self) -> bool {
        std::fs::read_to_string(self.dir.join("server.toml")).is_ok_and(|t| t.contains("[[also]]"))
    }

    /// The server, if it should run whenever the Hub does and isn't yet.
    pub fn start_server_if_wanted(&self) {
        let (server, ai) = {
            let i = self.inner.lock().unwrap();
            (!i.saved.server_stays_off, i.saved.localai_autostart)
        };
        // The service runs them (the AI runner too, when it's started with the server).
        if self.service() {
            if ai && self.store().installed("localai").is_some() && !self.localai_running() && !self.service_runs_ai() {
                let _ = self.open("localai");
            }
            return;
        }
        if ai && self.store().installed("localai").is_some() && !self.localai_running() {
            let _ = self.open("localai");
        }
        if server && self.store().installed("server").is_some() && !self.server_running() {
            let _ = self.open("server");
        }
    }

    /// A new server version takes over without downtime: it starts beside the
    /// old one (sharing the port), then the old one finishes and stops.
    fn restart_server(&self) {
        if self.service() {
            return;
        }
        // Only a server the Hub started that's still running (its id may
        // belong to another program by now).
        let pid_file = self.dir.join("server.pid");
        if !running(&pid_file, "xenify-server") {
            return;
        }
        let Ok(pid) = std::fs::read_to_string(&pid_file) else { return };
        let old = pid.trim().to_string();
        if self.open("server").is_ok() {
            std::thread::sleep(Duration::from_secs(3));
            #[cfg(unix)]
            let _ = xenify_hub_core::command("kill").args(["-TERM", &old]).status();
            #[cfg(windows)]
            let _ = xenify_hub_core::command("taskkill").args(["/PID", &old]).status();
        }
    }

    /// Signs in to a Xenify server. Its admin then decides about updates,
    /// and the apps use the server too (they follow this sign-in).
    pub fn connect(&self, url: &str, username: &str, password: &str, plain_ok: bool) -> Result<(), String> {
        let (url, info) = Self::reach(url)?;
        Self::password_may_go(&url, plain_ok)?;
        // Remembered after an earlier second step: no step this time.
        let remember = xenify_hub_core::signin::remembered(&url).unwrap_or_default();
        let s = post_json(
            &format!("{url}/v1/login"),
            None,
            &json!({ "username": username, "password": password, "device": "Xenify Hub", "remember": remember }),
        )?;
        self.answered(url, &info, username, s)
    }

    /// Signing in starts with the email address: `{ next: "password" |
    /// "pick_password", can_email, plain }` ("pick_password": an account an
    /// admin added that has no password yet; the code from the admin picks
    /// one; `plain`: the server is reached over plain HTTP beyond this
    /// network, which the screen warns about before the password).
    pub fn start(&self, url: &str, username: &str) -> Result<Value, String> {
        let (url, _) = Self::reach(url)?;
        let at = format!("{url}/v1/login/start");
        // A server from before this step: the password, as always.
        let mut out =
            post_json(&at, None, &json!({ "username": username })).unwrap_or_else(|_| json!({ "next": "password" }));
        if out.is_object() {
            out["plain"] = json!(plain(&url));
        }
        Ok(out)
    }

    /// An account without a password yet, or a forgotten one: picks
    /// `password` with `username`'s code (emailed, or from an admin), and
    /// signs in.
    pub fn claim(&self, url: &str, username: &str, code: &str, password: &str, plain_ok: bool) -> Result<(), String> {
        let (url, info) = Self::reach(url)?;
        Self::password_may_go(&url, plain_ok)?;
        let code = code.trim().rsplit('#').next().unwrap_or("").trim().to_string();
        let username = username.trim().to_lowercase();
        let found = post_json(&format!("{url}/v1/link"), None, &json!({ "link": code, "username": username }))?;
        let username = found["username"].as_str().unwrap_or(&username).to_string();
        let s = post_json(
            &format!("{url}/v1/link/use"),
            None,
            &json!({ "link": code, "username": username, "password": password, "device": "Xenify Hub" }),
        )?;
        self.answered(url, &info, &username, s)
    }

    /// Makes the first account on the server this Hub runs (its one-time
    /// setup link is in the server's address.json until then) and signs in
    /// with it, so every app is connected with nothing more to type.
    pub fn set_up_server(&self, name: &str, email: &str, password: &str) -> Result<(), String> {
        let a = self.server_address().ok_or("The server isn't running yet.")?;
        let port = a["http_port"].as_u64().ok_or("The server isn't running yet.")?;
        let token = a["setup"]
            .as_str()
            .and_then(|s| s.rsplit('/').next())
            .filter(|t| !t.is_empty())
            .ok_or("This server is already set up: sign in instead.")?
            .to_string();
        let (url, info) = Self::reach(&format!("http://127.0.0.1:{port}"))?;
        let s = post_json(
            &format!("{url}/v1/setup"),
            None,
            &json!({ "token": token, "server_name": "", "display_name": name.trim(), "email": email.trim(), "password": password, "device": "Xenify Hub" }),
        )?;
        self.answered(url, &info, email.trim(), s)
    }

    /// The server this Hub runs, while it's waiting for its first account.
    fn own_server_needs_setup(&self) -> bool {
        self.server_address().is_some_and(|a| a["setup"].as_str().is_some_and(|s| !s.is_empty()))
    }

    /// Has the server email `username` a code to pick a password (if the
    /// account has an address; it says the same either way).
    pub fn forgot(&self, url: &str, username: &str) -> Result<Value, String> {
        let (url, _) = Self::reach(url)?;
        post_json(&format!("{url}/v1/forgot"), None, &json!({ "username": username.trim().to_lowercase() }))?;
        Ok(json!({}))
    }

    /// The server's address as typed, checked: its `/v1/info`. Just an
    /// address ("files.example.com", "192.168.1.20:7443"): HTTPS first (a
    /// domain name's certificate on 443, or the server's own on 7443), then
    /// plain HTTP (7700) only to a computer on this network.
    fn reach(url: &str) -> Result<(String, Value), String> {
        let typed = url.trim().trim_end_matches('/').to_string();
        let tries = if typed.contains("://") {
            vec![typed.clone()]
        } else {
            let port =
                typed.rsplit_once(':').is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
            let mut t = vec![format!("https://{typed}")];
            if !port {
                t.push(format!("https://{typed}:7443"));
            }
            if local_host(&host_of(&format!("http://{typed}"))) {
                t.push(if port { format!("http://{typed}") } else { format!("http://{typed}:7700") });
            }
            t
        };
        // Found a moment ago (the email step, then the password step): not
        // looked for again.
        static LAST: std::sync::Mutex<Option<(String, String, Value, std::time::Instant)>> =
            std::sync::Mutex::new(None);
        if let Some((t, url, info, at)) = LAST.lock().unwrap().clone()
            && t == typed
            && at.elapsed() < Duration::from_secs(120)
        {
            return Ok((url, info));
        }
        // Every address at once: one that doesn't answer (each may take up
        // to 10 seconds) doesn't hold up the others. The first in the order
        // above that answers wins, once the ones before it have failed.
        let (tx, rx) = std::sync::mpsc::channel();
        for (n, url) in tries.iter().cloned().enumerate() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let r = get_json_within(&format!("{url}/v1/info"), None, 10);
                let _ = tx.send((n, r));
            });
        }
        drop(tx);
        let mut results: Vec<Option<Result<Value, String>>> = vec![None; tries.len()];
        for (n, r) in rx.iter() {
            results[n] = Some(r);
            match results.iter().position(|x| !matches!(x, Some(Err(_)))) {
                Some(k) if results[k].is_none() => {}
                _ => break,
            }
        }
        let mut found = None;
        for (url, r) in tries.into_iter().zip(results) {
            match r {
                Some(Ok(info)) => {
                    found = Some((url, info));
                    break;
                }
                // Its own certificate: the screen asks whether to trust it.
                Some(Err(e)) if url.starts_with("https://") && xenify_platform::trust::untrusted(&e) => {
                    return Err(format!("certificate:{url}"));
                }
                _ => {}
            }
        }
        let (url, info) = found.ok_or("There's no Xenify server at that address.")?;
        *LAST.lock().unwrap() = Some((typed, url.clone(), info.clone(), std::time::Instant::now()));
        // The server and this app must work with each other's protocol level.
        let level = |k: &str| info[k].as_u64().map(|v| v.min(u64::from(u32::MAX)) as u32);
        xenify_server_proto::works_with(level("protocol").unwrap_or(0), level("oldest_protocol"))
            .map_err(|m| m.says().to_string())?;
        Ok((url, info))
    }

    /// Refuses to send a password over plain HTTP beyond this computer's
    /// network, unless the person was warned (`plain_ok`).
    fn password_may_go(url: &str, plain_ok: bool) -> Result<(), String> {
        if plain(url) && !plain_ok {
            return Err("This server doesn't use HTTPS, so your password would travel unprotected.".into());
        }
        Ok(())
    }

    /// The server answered a password: signed in, or the second step next.
    fn answered(&self, url: String, info: &Value, username: &str, s: Value) -> Result<(), String> {
        let name = info["name"].as_str().unwrap_or("Xenify server").to_string();
        if let Some(ticket) = s["ticket"].as_str() {
            // The password was right: a code next, or setting one up.
            let step = Step {
                url,
                name,
                username: username.to_string(),
                ticket: ticket.to_string(),
                step: s.clone(),
                key: None,
            };
            self.inner.lock().unwrap().step = Some(step);
            return Ok(());
        }
        self.signed_in(url, name, username, &s)
    }

    /// Signed in (`s`: the session the server answered): the apps follow.
    fn signed_in(&self, url: String, name: String, username: &str, s: &Value) -> Result<(), String> {
        let token = s["token"].as_str().ok_or("The server sent something unexpected.")?.to_string();
        let a = &s["account"];
        let user = a["display_name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .or(a["email"].as_str().filter(|e| !e.is_empty()))
            .unwrap_or(username)
            .to_string();
        self.inner.lock().unwrap().step = None;
        // Another server or account before: sign that one out first.
        if let Some(old) = self.inner.lock().unwrap().saved.server.take() {
            let _ = post_json(&format!("{}/v1/signout", old.url), Some(&old.token), &json!({}));
        }
        let mut id = [0u8; 12];
        getrandom::fill(&mut id).map_err(|e| e.to_string())?;
        let shared = xenify_hub_core::signin::SignIn {
            id: id.iter().map(|x| format!("{x:02x}")).collect(),
            url: url.clone(),
            name: name.clone(),
            user: user.clone(),
            session: s.clone(),
            remember: s["remember"].as_str().map(|r| [(url.clone(), r.to_string())].into()).unwrap_or_default(),
        };
        // The apps use this sign-in too.
        xenify_hub_core::signin::write(&shared)?;
        self.inner.lock().unwrap().saved.server = Some(Server { url, token, name, user });
        self.save();
        Ok(())
    }

    /// A call of the second step, with its ticket. A ticket that ran out
    /// ends the step: signing in starts again.
    fn step_post(&self, path: &str, mut body: Value) -> Result<Value, String> {
        let (url, ticket) = {
            let i = self.inner.lock().unwrap();
            let s = i.step.as_ref().ok_or("Sign in again.")?;
            (s.url.clone(), s.ticket.clone())
        };
        body["ticket"] = json!(ticket);
        post_coded(&format!("{url}{path}"), None, &body).map_err(|(code, e)| {
            if code == "bad_ticket" {
                self.inner.lock().unwrap().step = None;
            }
            e
        })
    }

    /// Signs in with the second step's session.
    fn step_done(&self, session: &Value) -> Result<(), String> {
        let (url, name, username) = {
            let i = self.inner.lock().unwrap();
            let s = i.step.as_ref().ok_or("Sign in again.")?;
            (s.url.clone(), s.name.clone(), s.username.clone())
        };
        self.signed_in(url, name, &username, session)
    }

    pub fn disconnect(&self) {
        let server = {
            let mut i = self.inner.lock().unwrap();
            i.policy = None;
            i.mirror = false;
            i.update_server = None;
            i.step = None;
            i.saved.server.take()
        };
        xenify_hub_core::signin::clear();
        if let Some(s) = server {
            let _ = post_json(&format!("{}/v1/signout", s.url), Some(&s.token), &json!({}));
        }
        self.save();
    }

    /// Automatic updates (and what the server requires), when allowed now.
    pub fn auto_update(&self, hour: u8) {
        let (os, arch) = xenify_hub_core::platform();
        let service = self.service();
        let todo: Vec<String> = {
            let i = self.inner.lock().unwrap();
            let Some(m) = i.list.as_ref() else { return };
            let store = self.store();
            m.products
                .iter()
                .filter(|p| {
                    let e = effective(&i, &p.id);
                    let installed = store.installed(&p.id);
                    let wanted = installed.is_some() || e.required;
                    let automatic = e.mode == UpdateMode::Automatic || installed.is_none();
                    // The server's service updates it.
                    // The Hub updates itself separately (check_hub).
                    p.id != "hub"
                        && !(p.id == "server" && service)
                        && wanted
                        && automatic
                        && choose::in_window(e.window, hour)
                        && choose::update(p, installed.as_ref().map(|x| x.version.as_str()), &e, os, arch).is_some()
                        && !i.ready.contains_key(&p.id)
                })
                .map(|p| p.id.clone())
                .collect()
        };
        for p in todo {
            let _ = self.install(&p);
        }
        // "Update server now" on the admin pages.
        let want = self.inner.lock().unwrap().update_server.clone();
        if let Some(v) = want
            && self.store().installed("server").is_some_and(|x| x.version != v)
        {
            let mut i = self.inner.lock().unwrap();
            let local = i.saved.local_mut("server");
            local.choice = Choice::Fixed(v);
            drop(i);
            let _ = self.install("server");
        }
    }

    /// Settings › "Run AI models on this computer for the server": installs
    /// the AI runner (LocalAI) when it isn't yet, starts it (and with the
    /// Hub from now on), and points the server this Hub runs at it. Off:
    /// stops it and no longer starts it.
    pub fn set_local_ai(&self, on: bool) {
        self.inner.lock().unwrap().saved.localai_autostart = on;
        self.save();
        if !on {
            stop_pid(&self.dir.join("localai.pid"), "local-ai");
            return;
        }
        if self.store().installed("localai").is_none() && self.install("localai").is_err() {
            return;
        }
        // Installing happens in the background: start it once it's in.
        let hub = self.clone();
        std::thread::spawn(move || {
            for _ in 0..3600 {
                if hub.store().installed("localai").is_some() {
                    break;
                }
                if hub.inner.lock().unwrap().jobs.get("localai").is_some_and(|j| j.state == "failed") {
                    return;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            if !hub.inner.lock().unwrap().saved.localai_autostart {
                return;
            }
            if !hub.localai_running() && hub.open("localai").is_err() {
                return;
            }
            hub.point_server_at_local_ai();
        });
    }

    /// The server this Hub runs (signed in to it on this computer): its
    /// Admin › AI set to the AI runner here. Signed in elsewhere, or not at
    /// all: left alone (Settings says how to set it up).
    fn point_server_at_local_ai(&self) {
        let Some(s) = self.inner.lock().unwrap().saved.server.clone() else { return };
        if !local_host(&host_of(&s.url)) || self.server_port().is_none() {
            return;
        }
        let body = json!({
            "runner": "localai",
            "url": format!("http://127.0.0.1:{LOCALAI_PORT}/v1"),
            "models_dir": self.dir.join("localai-models").display().to_string(),
        });
        if let Err(e) = post_json(&format!("{}/v1/admin/ai", s.url), Some(&s.token), &body) {
            self.inner.lock().unwrap().error =
                sentence(&format!("The AI runner started, but the server's AI settings couldn't be changed ({e}): set it in Admin › AI"));
        }
    }

    /// After each check: a newer Hub on its channel is downloaded and
    /// checked against the signed list, ready to put in place (or, for a
    /// copy that can't replace itself, noted so the window offers the
    /// download page).
    pub fn check_hub(&self) {
        let (os, arch) = xenify_hub_core::platform();
        let found = {
            let i = self.inner.lock().unwrap();
            let Some(p) = i.list.as_ref().and_then(|m| m.product("hub")) else { return };
            let e = effective(&i, "hub");
            selfupdate::newer(p, selfupdate::VERSION, &e, os, arch)
                .and_then(|r| r.download(os, arch).cloned().map(|d| (r.version.clone(), d)))
        };
        let dir = self.dir.join("hub-update");
        let Some((version, d)) = found else {
            // Up to date (an installer left from the last update goes).
            self.inner.lock().unwrap().hub_update = None;
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        let have = self.inner.lock().unwrap().hub_update.as_ref().is_some_and(|u| {
            u.version == version && (u.by_hand || u.file.as_ref().is_some_and(|f| f.exists()))
        });
        if have {
            return;
        }
        let how = selfupdate::how();
        if how == selfupdate::How::ByHand {
            self.inner.lock().unwrap().hub_update = Some(HubUpdate { version, file: None, by_hand: true });
            return;
        }
        let Ok(part) = self.fetch("hub", &d.url, None, &d) else { return };
        if manifest::check_path(&d, &part).is_err() {
            let _ = std::fs::remove_file(&part);
            return;
        }
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let file = dir.join(selfupdate::file_name(&how, &version));
        if std::fs::rename(&part, &file).is_err() {
            if std::fs::copy(&part, &file).is_err() {
                return;
            }
            let _ = std::fs::remove_file(&part);
        }
        self.inner.lock().unwrap().hub_update = Some(HubUpdate { version, file: Some(file), by_hand: false });
    }

    /// A new Hub is downloaded and nothing would be interrupted by
    /// restarting into it (no download, install or service change under way).
    pub fn hub_ready_quietly(&self) -> bool {
        let i = self.inner.lock().unwrap();
        i.hub_update.as_ref().is_some_and(|u| u.file.is_some())
            && i.jobs.iter().all(|(id, j)| id == "hub" || j.state == "failed")
            && !i.service_busy
            && i.step.is_none()
    }

    /// Puts the downloaded Hub in place and starts it (in the tray when
    /// `hidden`); the caller ends this one straight after. When it can't,
    /// the window offers the download page instead.
    pub fn apply_hub_update(&self, hidden: bool) -> Result<(), String> {
        let file = self
            .inner
            .lock()
            .unwrap()
            .hub_update
            .as_ref()
            .and_then(|u| u.file.clone())
            .ok_or("There's no new Xenify Hub to put in place.")?;
        selfupdate::apply(&selfupdate::how(), &file, hidden).inspect_err(|e| {
            let mut i = self.inner.lock().unwrap();
            i.error = sentence(e);
            if let Some(u) = i.hub_update.as_mut() {
                u.by_hand = true;
                u.file = None;
            }
        })
    }

    pub fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        // Warned that the server doesn't use HTTPS (sign-in screen).
        let plain_ok = args.get("plain_ok").and_then(Value::as_bool).unwrap_or(false);
        match name {
            "state" => Ok(self.state()),
            "local_ai" => {
                self.set_local_ai(args.get("on").and_then(Value::as_bool).unwrap_or(false));
                self.look();
                Ok(self.state())
            }
            "localai_autostart" => {
                self.inner.lock().unwrap().saved.localai_autostart =
                    args.get("on").and_then(Value::as_bool).unwrap_or(false);
                self.save();
                self.start_server_if_wanted();
                self.look();
                Ok(self.state())
            }
            "server_service" => {
                // The password prompt can take a while: not on the window's thread.
                let on = args.get("on").and_then(Value::as_bool).unwrap_or(false);
                self.inner.lock().unwrap().service_busy = true;
                let hub = self.clone();
                std::thread::spawn(move || {
                    let r = hub.set_service(on);
                    let mut i = hub.inner.lock().unwrap();
                    i.service_busy = false;
                    if let Err(e) = r {
                        i.error = sentence(&format!("couldn't change how the server starts: {e}"));
                    }
                });
                Ok(self.state())
            }
            "server_autostart" => {
                self.inner.lock().unwrap().saved.server_stays_off =
                    !args.get("on").and_then(Value::as_bool).unwrap_or(true);
                self.save();
                self.start_server_if_wanted();
                Ok(self.state())
            }
            "check" => {
                let hub = self.clone();
                std::thread::spawn(move || {
                    let _ = hub.check();
                });
                Ok(self.state())
            }
            "install" | "update" => self.install(&s("product")).map(|_| self.state()),
            "open" => {
                self.open(&s("product"))?;
                self.look();
                Ok(Value::Null)
            }
            "roll_back" => {
                // Not back to a version the release list says isn't safe.
                let before = self.store().installed(&s("product")).and_then(|x| x.previous).map(|x| x.version);
                if let Some(v) = before
                    && !manifest::safe(&self.inner.lock().unwrap().limits, &s("product"), &v)
                {
                    return Err(format!("Version {v} has a known security problem, so it isn't used again."));
                }
                let back = self.store().roll_back(&s("product")).map_err(|e| sentence(&e.to_string()))?;
                self.inner.lock().unwrap().ready.remove(&s("product"));
                self.place(&s("product"));
                // Stays there (no automatic update straight back) until a
                // channel is picked again; the server's choice wins if it has one.
                let _ = self.set_choice(&s("product"), Some(Choice::Fixed(back.version)), None);
                self.report();
                Ok(self.state())
            }
            "admin" => {
                let url = self.server_admin().ok_or("the server hasn't started yet")?;
                open_web(&url).map(|_| Value::Null)
            }
            // A Hub that can't replace itself: the download page.
            "hub_download" => open_web(selfupdate::DOWNLOAD_PAGE).map(|_| Value::Null),
            "remove" => {
                let needed = m_needs(self.inner.lock().unwrap().list.as_ref(), &s("product"), &self.store());
                if !needed.is_empty() {
                    return Err(format!("{} needs it", needed.join(" and ")));
                }
                self.store().remove(&s("product")).map_err(|e| sentence(&e.to_string()))?;
                let name = self
                    .inner
                    .lock()
                    .unwrap()
                    .list
                    .as_ref()
                    .and_then(|m| m.product(&s("product")))
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                crate::shortcuts::remove(&s("product"), &name);
                self.report();
                Ok(self.state())
            }
            "folder" => {
                let dir = self.dir.join("apps").join(s("product"));
                #[cfg(target_os = "linux")]
                let r = xenify_hub_core::command("xdg-open").arg(&dir).spawn();
                #[cfg(target_os = "macos")]
                let r = xenify_hub_core::command("open").arg(&dir).spawn();
                #[cfg(windows)]
                let r = xenify_hub_core::command("explorer").arg(&dir).spawn();
                r.map(|_| Value::Null).map_err(|e| e.to_string())
            }
            "choose" => {
                let choice = match (s("channel").as_str(), s("fixed")) {
                    ("", v) if !v.is_empty() => Some(Choice::Fixed(v)),
                    ("", _) => None,
                    (c, _) => Some(Choice::Channel(serde_json::from_value(json!(c)).map_err(|_| "unknown channel")?)),
                };
                let mode = match s("mode").as_str() {
                    "automatic" => Some(UpdateMode::Automatic),
                    "manual" => Some(UpdateMode::Manual),
                    _ => None,
                };
                self.set_choice(&s("product"), choice, mode)?;
                Ok(self.state())
            }
            "choose_all" => {
                let c = serde_json::from_value(json!(s("channel"))).map_err(|_| "unknown channel")?;
                self.set_all_channel(c);
                Ok(self.state())
            }
            "update_all" => {
                let ids: Vec<String> = self.state()["products"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| p["installed"].is_string() && p["to"].is_object())
                    .filter_map(|p| p["id"].as_str().map(String::from))
                    .collect();
                for id in ids {
                    let _ = self.install(&id);
                }
                Ok(self.state())
            }
            "set_up_server" => {
                self.set_up_server(&s("name"), &s("email"), &s("password"))?;
                self.check_soon();
                Ok(self.state())
            }
            "connect" => {
                self.connect(&s("url"), &s("username"), &s("password"), plain_ok)?;
                self.check_soon();
                Ok(self.state())
            }
            // The first step of signing in: just the address, checked.
            "find" => {
                let (url, info) = Self::reach(&s("url"))?;
                Ok(json!({ "url": url, "name": info["name"].as_str().unwrap_or("Xenify server"), "plain": plain(&url) }))
            }
            "login_start" => self.start(&s("url"), &s("username")),
            "forgot" => self.forgot(&s("url"), &s("username")),
            "claim" => {
                self.claim(&s("url"), &s("username"), &s("code"), &s("password"), plain_ok)?;
                self.check_soon();
                Ok(self.state())
            }
            // A server's own certificate: what it is, or (with a
            // fingerprint) trusting it for good or this time.
            "certificate" => {
                let f = s("fingerprint");
                let seen = if f.is_empty() {
                    xenify_platform::trust::look(&s("url"))?
                } else {
                    xenify_platform::trust::trust(&s("url"), &f, args["remember"].as_bool().unwrap_or(false))?
                };
                Ok(serde_json::to_value(seen).unwrap_or_default())
            }
            // The second step: `method` "app", "email" or "backup".
            "step_code" => {
                let remember = args.get("remember").and_then(Value::as_bool).unwrap_or(false);
                let body = json!({ "method": s("method"), "code": s("code").trim(), "remember": remember, "device": "Xenify Hub" });
                let session = self.step_post("/v1/login/code", body)?;
                self.step_done(&session)?;
                self.check_soon();
                Ok(self.state())
            }
            "step_email" => self.step_post("/v1/login/email", json!({})),
            "step_cancel" => {
                self.inner.lock().unwrap().step = None;
                Ok(self.state())
            }
            // Setting a second step up (the server requires one): the
            // authenticator key to scan, or a code by email; then the code.
            "setup_app" => {
                if let Some(k) = self.inner.lock().unwrap().step.as_ref().and_then(|s| s.key.clone()) {
                    return Ok(k);
                }
                let k = self.step_post("/v1/mfa/app/start", json!({}))?;
                if let Some(s) = self.inner.lock().unwrap().step.as_mut() {
                    s.key = Some(k.clone());
                }
                Ok(k)
            }
            "setup_email" => self.step_post("/v1/mfa/email", json!({})),
            // { backup_codes, state }: the codes are shown once.
            "setup_finish" => {
                let path = if s("method") == "app" { "/v1/mfa/app/finish" } else { "/v1/mfa/email" };
                let remember = args.get("remember").and_then(Value::as_bool).unwrap_or(false);
                let out = self.step_post(path, json!({ "code": s("code").trim(), "remember": remember }))?;
                self.step_done(&out["session"])?;
                self.check_soon();
                Ok(json!({ "backup_codes": out["backup_codes"], "state": self.state() }))
            }
            // Signed in: two-step sign-in settings, on the
            // server the Hub is signed in to (these paths only).
            "server" => {
                let (method, path) = (s("method"), s("path"));
                let allowed = [
                    ("GET", "/v1/me/mfa"),
                    ("GET", "/v1/info"),
                    ("POST", "/v1/mfa/app/start"),
                    ("POST", "/v1/mfa/app/finish"),
                    ("POST", "/v1/mfa/email"),
                    ("POST", "/v1/me/mfa/off"),
                    ("POST", "/v1/me/mfa/backup"),
                ];
                if !allowed.contains(&(method.as_str(), path.as_str())) {
                    return Err("The Hub doesn't do that.".into());
                }
                let (url, token) = {
                    let i = self.inner.lock().unwrap();
                    let srv = i.saved.server.as_ref().ok_or("Sign in to a server first.")?;
                    (srv.url.clone(), srv.token.clone())
                };
                let at = format!("{url}{path}");
                if method == "GET" {
                    get_json(&at, Some(&token))
                } else {
                    post_json(&at, Some(&token), args.get("body").unwrap_or(&json!({})))
                }
            }
            "disconnect" => {
                self.disconnect();
                self.check_soon();
                Ok(self.state())
            }
            "set_source" => {
                let src = s("source");
                self.inner.lock().unwrap().saved.source = (!src.trim().is_empty()).then(|| src.trim().to_string());
                self.save();
                self.check_soon();
                Ok(self.state())
            }
            _ => Err(format!("unknown call “{name}”")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_list_shows_at_once() {
        let dir = std::env::temp_dir().join(format!("xenify-hub-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = manifest::new_key();
        let trusted = vec![key.verifying_key().to_bytes().to_vec()];
        std::fs::write(dir.join("trusted-keys.json"), serde_json::to_string(&trusted).unwrap()).unwrap();
        let m = Manifest { published: "2026-11-02T00:00:00Z".into(), products: vec![] };
        std::fs::write(dir.join("list.json"), serde_json::to_string(&manifest::sign(&m, &key)).unwrap()).unwrap();
        assert!(Hub::new(dir.clone()).inner.lock().unwrap().list.is_some());
        // Signed by a key this Hub doesn't trust: not shown.
        let other = manifest::sign(&m, &manifest::new_key());
        std::fs::write(dir.join("list.json"), serde_json::to_string(&other).unwrap()).unwrap();
        assert!(Hub::new(dir.clone()).inner.lock().unwrap().list.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_http_is_for_this_network_only() {
        assert_eq!(super::host_of("http://office-pc:7700/x"), "office-pc");
        assert_eq!(super::host_of("https://[fe80::1]:7443"), "fe80::1");
        assert_eq!(super::host_of("http://user@10.0.0.2:7700"), "10.0.0.2");
        for local in [
            "http://127.0.0.1:7700",
            "http://192.168.1.20:7700",
            "http://10.1.2.3",
            "http://172.20.0.5",
            "http://100.101.102.103:7700",
            "http://[fd00::5]:7700",
            "http://[fe80::1]",
            "http://localhost:7700",
        ] {
            assert!(!super::plain(local), "{local} is on this network");
        }
        for far in ["http://8.8.8.8:7700", "http://[2001:4860::8888]", "http://172.32.0.1"] {
            assert!(super::plain(far), "{far} is beyond this network");
        }
        assert!(!super::plain("https://8.8.8.8"), "HTTPS is fine anywhere");
        assert!(super::Hub::password_may_go("http://8.8.8.8:7700", false).is_err());
        assert!(super::Hub::password_may_go("http://8.8.8.8:7700", true).is_ok());
        assert!(super::Hub::password_may_go("http://192.168.1.20:7700", false).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn its_settings_with_the_token_are_this_users_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xenify-hub-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Written before by an older Hub, readable by everyone.
        std::fs::write(dir.join("hub.json.part"), "{}").unwrap();
        std::fs::set_permissions(dir.join("hub.json.part"), std::fs::Permissions::from_mode(0o644)).unwrap();
        let hub = Hub::new(dir.clone());
        hub.inner.lock().unwrap().saved.server =
            Some(Server { url: "http://x".into(), token: "secret".into(), ..Default::default() });
        hub.save();
        let meta = std::fs::metadata(dir.join("hub.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_to_string(dir.join("hub.json")).unwrap().contains("secret"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
