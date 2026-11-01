//! How the server is reached (Admin › Network): its HTTP and HTTPS ports,
//! announcing it to apps on the network, and Write and Draw for the browser.
//! Saved in the database and applied while it runs: the new listeners are
//! bound first (a port that's taken is refused before anything changes),
//! then the old ones stop taking connections and finish what they have.
//! Command-line flags win over what's saved; the admin pages show those as
//! fixed.

use crate::{Server, db, err, failed, unavailable};
use axum::Json;
use axum::extract::State;
use axum::routing::get;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub const HTTP_PORT: u16 = 7700;
pub const HTTPS_PORT: u16 = 7443;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Net {
    pub http_port: u16,
    /// None: no HTTPS (e.g. behind a reverse proxy that does it).
    pub https_port: Option<u16>,
    /// Apps on the network find the server under Settings › Server (mDNS).
    pub announce: bool,
    /// Always on: the server always offers the apps in the browser (kept
    /// for settings saved when it was a choice).
    pub apps: bool,
}

impl Default for Net {
    fn default() -> Self {
        Net { http_port: HTTP_PORT, https_port: Some(HTTPS_PORT), announce: true, apps: true }
    }
}

/// What came from the command line (each one can't be changed on the
/// admin pages). The addresses keep their host part, e.g. 127.0.0.1:7700.
#[derive(Clone, Debug, Default)]
pub struct Fixed {
    pub listen: Option<String>,
    /// An address, or "off".
    pub https: Option<String>,
    pub hidden: bool,
    pub apps: bool,
}

/// The listeners running now.
pub(crate) struct Running {
    pub net: Net,
    stop: tokio::sync::watch::Sender<bool>,
    _announced: Option<mdns_sd::ServiceDaemon>,
}

fn addr(fixed: &Option<String>, port: u16) -> String {
    match fixed {
        Some(a) => a.clone(),
        None => format!("0.0.0.0:{port}"),
    }
}

fn port_of(addr: &str) -> Option<u16> {
    addr.rsplit(':').next()?.parse().ok()
}

impl Server {
    /// What's saved, with the command line's choices on top.
    pub async fn net(&self) -> Net {
        let saved = match self.pool.get().await {
            Ok(c) => db::setting(&c, "net").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()),
            Err(_) => None,
        };
        let mut net: Net = saved.unwrap_or_default();
        let fixed = self.net_fixed.lock().unwrap().clone();
        if let Some(p) = fixed.listen.as_deref().and_then(port_of) {
            net.http_port = p;
        }
        match fixed.https.as_deref() {
            Some("off") => net.https_port = None,
            Some(a) => net.https_port = port_of(a),
            None => {}
        }
        if fixed.hidden {
            net.announce = false;
        }
        net.apps = true;
        net
    }

    /// Binds and serves `net` (replacing what ran before), until the server
    /// stops. Nothing changes if a port can't be had.
    pub async fn run_network(&self, net: Net) -> Result<(), String> {
        let fixed = self.net_fixed.lock().unwrap().clone();
        let http_addr = addr(&fixed.listen, net.http_port);
        let https_addr = net.https_port.map(|p| addr(&fixed.https.clone().filter(|a| a != "off"), p));
        // Ports are shared only with this same server (an update taking
        // over, or these listeners being replaced), never with another one.
        let ours = self.holds(&http_addr).await?;
        if let Some(a) = &https_addr
            && !ours
            && answers(a).await
        {
            return Err(format!("port {} is in use by another program", net.https_port.unwrap_or_default()));
        }
        let http = crate::listen(&http_addr).map_err(|e| format!("port {} can't be used: {e}", net.http_port))?;
        let https = match &https_addr {
            Some(a) => Some(
                crate::listen(a)
                    .map_err(|e| format!("port {} can't be used: {e}", net.https_port.unwrap_or_default()))?,
            ),
            None => None,
        };
        *self.hosting.lock().unwrap() = net.apps;
        let (stop, stopped) = tokio::sync::watch::channel(false);
        // The whole server stopping stops these too.
        let mut all = self.stopping();
        let s2 = stop.clone();
        tokio::spawn(async move {
            let _ = all.wait_for(|s| *s).await;
            let _ = s2.send(true);
        });
        let router = self.router();
        let serving = self.serving.clone();
        serving.send_modify(|n| *n += 1);
        let wait = stopped.clone();
        tokio::spawn(async move {
            crate::tls::serve_plain(http, router, wait).await;
            serving.send_modify(|n| *n -= 1);
        });
        if let Some(l) = https {
            self.serve_https_until(l, stopped)?;
        } else {
            *self.https_port.lock().unwrap() = None;
        }
        let announced = if net.announce {
            let name = self.name().await;
            let id = if self.setup_token().is_some() { "new" } else { "ready" };
            crate::tls::announce(
                if name.is_empty() { "Xenify server" } else { &name },
                net.http_port,
                net.https_port,
                id,
            )
            .map_err(|e| eprintln!("xenify-server: not announced on the network: {e}"))
            .ok()
        } else {
            None
        };
        let old = self.running.lock().unwrap().replace(Running { net: net.clone(), stop, _announced: announced });
        if let Some(old) = old {
            let _ = old.stop.send(true);
        }
        println!("xenify-server {}: listening on {http_addr}", crate::VERSION);
        if let Some(a) = https_addr {
            println!("HTTPS on {a}");
        }
        self.write_address();
        Ok(())
    }

    /// Whether this same server already listens at `addr` (Ok(false):
    /// nothing does). Another server or program there is an error.
    async fn holds(&self, addr: &str) -> Result<bool, String> {
        if !answers(addr).await {
            return Ok(false);
        }
        let ours = match self.pool.get().await {
            Ok(c) => db::setting(&c, "server_id").await.ok().flatten().unwrap_or_default(),
            Err(_) => String::new(),
        };
        let theirs = info_id(addr).await.unwrap_or_default();
        if !ours.is_empty() && ours == theirs {
            Ok(true)
        } else {
            Err(format!("port {} is in use by another program", port_of(addr).unwrap_or_default()))
        }
    }

    /// Once stopping: when the listeners have finished their connections.
    pub async fn drained(&self) {
        let _ = self.serving.subscribe().wait_for(|n| *n == 0).await;
    }

    /// `<data>/address.json`: the ports and, until it's set up, the setup
    /// link (readable only by the server's user), for Xenify Hub and
    /// `xenify-server setup-link`.
    pub(crate) fn write_address(&self) {
        let Some(net) = self.running.lock().unwrap().as_ref().map(|r| r.net.clone()) else { return };
        let setup = self.setup_token().map(|t| format!("/admin/#/setup/{t}"));
        let text =
            serde_json::json!({ "http_port": net.http_port, "https_port": net.https_port, "setup": setup }).to_string();
        let path = self.data.join("address.json");
        let _ = std::fs::create_dir_all(&self.data);
        let _ = crate::tls::write_private(&path, &text);
    }
}

/// Where to reach a listener bound to `addr` from this computer.
fn local(addr: &str) -> String {
    let port = port_of(addr).unwrap_or_default();
    match addr.rsplit_once(':').map(|(h, _)| h.trim_matches(['[', ']'])) {
        Some("0.0.0.0" | "" | "::") | None => format!("127.0.0.1:{port}"),
        Some(h) if h.contains(':') => format!("[{h}]:{port}"),
        Some(h) => format!("{h}:{port}"),
    }
}

/// Something accepts connections at `addr`.
async fn answers(addr: &str) -> bool {
    let to = local(addr);
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), tokio::net::TcpStream::connect(to)).await,
        Ok(Ok(_))
    )
}

/// The server id of a Xenify server at `addr` (plain HTTP).
async fn info_id(addr: &str) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let to = local(addr);
    let ask = async {
        let mut s = tokio::net::TcpStream::connect(&to).await.ok()?;
        s.write_all(format!("GET /v1/info HTTP/1.0\r\nHost: {to}\r\n\r\n").as_bytes()).await.ok()?;
        let mut out = Vec::new();
        s.take(64 << 10).read_to_end(&mut out).await.ok()?;
        let text = String::from_utf8_lossy(&out);
        let body = text.split_once("\r\n\r\n")?.1;
        serde_json::from_str::<serde_json::Value>(body).ok()?["server_id"].as_str().map(String::from)
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), ask).await.ok().flatten()
}

/// The apps for the browser sit beside the program (`apps/`), came with
/// --apps, or were downloaded (Admin › Network).
fn apps_available(s: &Server) -> bool {
    s.apps
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|d| ["write", "draw", "calc", "plan", "chat"].iter().any(|a| d.join(a).is_dir()))
}

pub(crate) async fn get_net(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let fixed = s.net_fixed.lock().unwrap().clone();
    let running = s.running.lock().unwrap().as_ref().map(|r| r.net.clone());
    Json(serde_json::json!({
        "net": running.unwrap_or(s.net().await),
        "fixed": {
            "http": fixed.listen.is_some(),
            "https": fixed.https.is_some(),
            "announce": fixed.hidden,
            "apps": fixed.apps,
        },
        "apps_available": apps_available(&s),
        // Downloading them: "downloading", or why it failed.
        "apps_job": crate::hub::web_job(&s),
        "apps_size": if apps_available(&s) { None } else { crate::hub::web_apps_size(&s).await },
    }))
    .into_response()
}

/// "Try again": the apps for the browser, from Xenify's release list (they
/// download by themselves; this is for when that failed).
pub(crate) async fn get_web_apps(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.updates").await {
        return e;
    }
    if s.net_fixed.lock().unwrap().apps {
        return err(StatusCode::CONFLICT, "apps_fixed", "the server was started with --apps: it serves that folder");
    }
    crate::hub::download_web_apps(&s);
    StatusCode::ACCEPTED.into_response()
}

pub(crate) async fn set_net(State(s): State<Server>, h: HeaderMap, Json(mut v): Json<Net>) -> Response {
    if let Err(e) = crate::admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let fixed = s.net_fixed.lock().unwrap().clone();
    let now = s.net().await;
    // What the command line set stays.
    if fixed.listen.is_some() {
        v.http_port = now.http_port;
    }
    if fixed.https.is_some() {
        v.https_port = now.https_port;
    }
    if fixed.hidden {
        v.announce = false;
    }
    v.apps = true;
    if v.http_port == 0 || v.https_port == Some(0) || v.https_port == Some(v.http_port) {
        return err(StatusCode::BAD_REQUEST, "bad_port", "choose two different ports between 1 and 65535");
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let running = s.running.lock().unwrap().as_ref().map(|r| r.net.clone());
    if running.as_ref() != Some(&v)
        && let Err(e) = s.run_network(v.clone()).await
    {
        return err(StatusCode::CONFLICT, "port_taken", &e);
    }
    match db::set_setting(&c, "net", &serde_json::to_string(&v).expect("serialises")).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

// ---- Behind a reverse proxy, and this server's address ---------------------

/// An address or a network ("10.0.0.0/8").
pub type Cidr = (IpAddr, u8);

/// "127.0.0.1, 10.0.0.0/8 ::1": addresses and networks, split by commas,
/// spaces or lines.
pub fn parse_proxies(text: &str) -> Result<Vec<Cidr>, String> {
    let mut out = Vec::new();
    for item in text.split(|c: char| c == ',' || c.is_whitespace()).filter(|i| !i.is_empty()) {
        let bad = || format!("“{item}” isn't an address or a network (like 10.0.0.0/8)");
        let (ip, bits) = match item.split_once('/') {
            Some((ip, bits)) => (ip, Some(bits.parse::<u8>().map_err(|_| bad())?)),
            None => (item, None),
        };
        let ip: IpAddr = ip.trim_matches(['[', ']']).parse().map_err(|_| bad())?;
        let max = if ip.is_ipv4() { 32 } else { 128 };
        let bits = bits.unwrap_or(max);
        if bits > max {
            return Err(bad());
        }
        out.push((ip.to_canonical(), bits));
    }
    Ok(out)
}

fn trusted(list: &[Cidr], ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    list.iter().any(|(net, bits)| crate::tls::in_subnet(ip, *net, *bits))
}

/// The address a request came from: its connection's, or (only from a
/// trusted proxy) the right-most one in X-Forwarded-For that isn't one of
/// the trusted proxies. Addresses further left could be made up.
pub fn client_ip_with(list: &[Cidr], peer: IpAddr, headers: &HeaderMap) -> IpAddr {
    if !trusted(list, peer) {
        return peer;
    }
    let mut ip = peer;
    let hops: Vec<String> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(|h| h.trim().to_string()))
        .collect();
    for hop in hops.iter().rev() {
        let Ok(h) = hop.trim_matches(['[', ']']).parse::<IpAddr>() else { break };
        ip = h;
        if !trusted(list, h) {
            break;
        }
    }
    ip
}

const PROXIES_FOR: std::time::Duration = std::time::Duration::from_secs(30);

impl Server {
    /// The trusted proxies (Admin › Network), read again every half minute.
    async fn proxies(&self) -> std::sync::Arc<Vec<Cidr>> {
        if let Some((at, list)) = self.proxies.lock().unwrap().as_ref()
            && at.elapsed() < PROXIES_FOR
        {
            return list.clone();
        }
        let text = match self.pool.get().await {
            Ok(c) => db::setting(&c, "trusted_proxies").await.ok().flatten().unwrap_or_default(),
            Err(_) => String::new(),
        };
        let list = std::sync::Arc::new(parse_proxies(&text).unwrap_or_default());
        *self.proxies.lock().unwrap() = Some((std::time::Instant::now(), list.clone()));
        list
    }

    pub(crate) async fn trusts_proxy(&self, peer: IpAddr) -> bool {
        trusted(&self.proxies().await, peer)
    }

    /// Where a request really came from (see [`client_ip_with`]).
    pub(crate) async fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        client_ip_with(&self.proxies().await, peer, headers)
    }

    /// Plain HTTP that should be HTTPS: a 301 (308 keeps the method) there.
    /// Only when HTTPS is on and the server is set up (setting it up works
    /// over plain HTTP on its network), not from this computer or a trusted
    /// proxy, and not for the health check or the certificate to trust.
    /// On its network, plain HTTP stays while the address has only the
    /// server's own certificate (computers that don't trust it yet would get
    /// a warning); from anywhere else, and at the domain name with Let's
    /// Encrypt's certificate, it goes to HTTPS.
    pub(crate) async fn https_redirect(&self, peer: IpAddr, req: &axum::http::request::Parts) -> Option<Response> {
        let peer = peer.to_canonical();
        let path = req.uri.path();
        if peer.is_loopback() || path == "/v1/health" || path == "/v1/ca.pem" || self.setup_token().is_some() {
            return None;
        }
        let port = (*self.https_port.lock().unwrap())?;
        let host_header = req.headers.get(axum::http::header::HOST)?.to_str().ok()?;
        let host = match host_header.strip_prefix('[') {
            Some(v6) => format!("[{}]", v6.split_once(']')?.0),
            None => host_header.rsplit_once(':').map(|(h, _)| h).unwrap_or(host_header).to_string(),
        };
        if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-[]:".contains(c)) {
            return None;
        }
        let public_name = self.acme.read().unwrap().as_ref().is_some_and(|a| {
            a.production
                && a.domains.iter().any(|d| d.eq_ignore_ascii_case(&host))
                && *a.status.lock().unwrap() == "ready"
        });
        if !public_name && crate::may_admin("lan", peer) {
            return None;
        }
        if self.trusts_proxy(peer).await {
            return None;
        }
        let rest = req.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        let to = if port == 443 { format!("https://{host}{rest}") } else { format!("https://{host}:{port}{rest}") };
        let keep = !matches!(req.method, axum::http::Method::GET | axum::http::Method::HEAD);
        let status = if keep { StatusCode::PERMANENT_REDIRECT } else { StatusCode::MOVED_PERMANENTLY };
        Some((status, [(axum::http::header::LOCATION, to)]).into_response())
    }
}

pub(crate) fn routes() -> axum::Router<Server> {
    axum::Router::new().route("/v1/admin/network/reach", get(get_reach).post(set_reach))
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct Reach {
    /// This server's address for links in email ("https://files.example.org"):
    /// empty goes by the domain name, else the address each request came to.
    address: String,
    /// Admin › Network › Trusted proxies, as typed.
    trusted_proxies: String,
}

/// Full admins only: these decide where every emailed link points, and
/// which addresses count as this network.
async fn full_admin(s: &Server, h: &HeaderMap) -> Result<(), Response> {
    let a = crate::signed_in(s, h).await?;
    if a.admin { Ok(()) } else { Err(crate::perms::full_admins_only()) }
}

async fn get_reach(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = full_admin(&s, &h).await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let get = |k: &'static str| {
        let c = &c;
        async move { db::setting(c, k).await.ok().flatten().unwrap_or_default() }
    };
    Json(Reach { address: get("address").await, trusted_proxies: get("trusted_proxies").await }).into_response()
}

/// "https://files.example.org": http(s), a host and maybe a port, nothing after.
fn clean_address(a: &str) -> Option<String> {
    let a = a.trim().trim_end_matches('/');
    if a.is_empty() {
        return Some(String::new());
    }
    let u = url::Url::parse(a).ok()?;
    let plain = matches!(u.scheme(), "http" | "https")
        && u.host_str().is_some_and(|h| !h.is_empty())
        && u.username().is_empty()
        && u.password().is_none()
        && u.path() == "/"
        && u.query().is_none()
        && u.fragment().is_none();
    plain.then(|| u.as_str().trim_end_matches('/').to_string())
}

async fn set_reach(State(s): State<Server>, h: HeaderMap, Json(v): Json<Reach>) -> Response {
    if let Err(e) = full_admin(&s, &h).await {
        return e;
    }
    let Some(address) = clean_address(&v.address) else {
        return err(StatusCode::BAD_REQUEST, "bad_address", "type the address as https://files.example.org");
    };
    let proxies = match parse_proxies(&v.trusted_proxies) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_proxy", &e),
    };
    let text = proxies
        .iter()
        .map(|(ip, bits)| if *bits == if ip.is_ipv4() { 32 } else { 128 } { ip.to_string() } else { format!("{ip}/{bits}") })
        .collect::<Vec<_>>()
        .join(", ");
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if db::set_setting(&c, "address", &address).await.is_err()
        || db::set_setting(&c, "trusted_proxies", &text).await.is_err()
    {
        return failed();
    }
    *s.proxies.lock().unwrap() = None;
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_trusted_proxies_say_where_requests_came_from() {
        let list = parse_proxies("127.0.0.1, 10.0.0.0/8\n::1").unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.2.3.4, 203.0.113.7, 10.1.1.1".parse().unwrap());
        // From the proxy: the right-most address that isn't a proxy.
        assert_eq!(client_ip_with(&list, ip("127.0.0.1"), &h), ip("203.0.113.7"));
        assert_eq!(client_ip_with(&list, ip("::ffff:127.0.0.1"), &h), ip("203.0.113.7"));
        // From anyone else, the header means nothing.
        assert_eq!(client_ip_with(&list, ip("198.51.100.1"), &h), ip("198.51.100.1"));
        assert_eq!(client_ip_with(&[], ip("127.0.0.1"), &h), ip("127.0.0.1"));
        h.insert("x-forwarded-for", "nonsense".parse().unwrap());
        assert_eq!(client_ip_with(&list, ip("127.0.0.1"), &h), ip("127.0.0.1"));
        assert!(parse_proxies("10.0.0.0/33").is_err());
        assert!(parse_proxies("proxy.lan").is_err());
        assert_eq!(parse_proxies("").unwrap(), vec![]);
    }

    #[test]
    fn addresses_are_plain() {
        assert_eq!(clean_address(" https://files.example.org/ ").as_deref(), Some("https://files.example.org"));
        assert_eq!(clean_address("http://192.168.1.5:7700").as_deref(), Some("http://192.168.1.5:7700"));
        assert_eq!(clean_address("").as_deref(), Some(""));
        assert_eq!(clean_address("https://x.org/path"), None);
        assert_eq!(clean_address("javascript:alert(1)"), None);
        assert_eq!(clean_address("https://a@x.org"), None);
    }
}
