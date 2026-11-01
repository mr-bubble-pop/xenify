//! HTTPS on a home or office network: the server makes its own certificate
//! authority once (kept in the data folder) and a certificate for every name
//! and address it can be reached at. Each computer trusts the authority once
//! ("Trust this server" on the admin pages, `/v1/ca.pem`); after that there
//! are no warnings, and browsers allow what needs HTTPS (offline web apps,
//! the clipboard). The desktop apps can instead trust a server by its
//! authority's fingerprint, without touching the computer's list.
//!
//! The authority may only vouch for names on a local network (.local,
//! .lan, home.arpa, …), private addresses, and this computer's own names
//! and addresses when it was made (name constraints), so trusting it can't
//! be used to pretend to be any other site.

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DnType, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

/// This computer's name, if it has one.
fn host_name() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

/// The names and addresses this computer answers to.
pub fn local_names() -> Vec<String> {
    let mut names = vec!["localhost".to_string()];
    if let Some(h) = host_name() {
        names.push(h.clone());
        names.push(format!("{h}.local"));
    }
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        names.push(iface.ip().to_string());
    }
    names.sort();
    names.dedup();
    names
}

/// Names on a local network, and private address ranges (with prefixes).
const LOCAL_DNS: &[&str] = &["localhost", "local", "lan", "home.arpa", "internal", "home", "corp", "intranet"];
const PRIVATE_NETS: &[(&str, u8)] = &[
    ("10.0.0.0", 8),
    ("172.16.0.0", 12),
    ("192.168.0.0", 16),
    ("127.0.0.0", 8),
    ("169.254.0.0", 16),
    ("100.64.0.0", 10),
    ("::1", 128),
    ("fc00::", 7),
    ("fe80::", 10),
];

/// Whether a certificate from an authority limited to `allowed` (DNS names
/// and IP subnets) may carry `name`.
fn permitted(allowed: &[String], name: &str) -> bool {
    if let Ok(ip) = name.parse::<IpAddr>() {
        return allowed.iter().any(|a| match a.split_once('/') {
            Some((net, bits)) => match (net.parse::<IpAddr>(), bits.parse::<u8>()) {
                (Ok(net), Ok(bits)) => in_subnet(ip, net, bits),
                _ => false,
            },
            None => false,
        });
    }
    let name = name.to_ascii_lowercase();
    allowed
        .iter()
        .filter(|a| !a.contains('/'))
        .any(|a| name == *a || name.ends_with(&format!(".{a}")))
}

pub(crate) fn in_subnet(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let mask = u32::MAX.checked_shl(32 - bits as u32).unwrap_or(0);
            u32::from(a) & mask == u32::from(b) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let mask = u128::MAX.checked_shl(128 - bits as u32).unwrap_or(0);
            u128::from(a) & mask == u128::from(b) & mask
        }
        _ => false,
    }
}

/// What a new authority may vouch for: local names and private ranges, plus
/// this computer's own `names` (its host name, public addresses).
fn allowed_for(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = LOCAL_DNS.iter().map(|s| s.to_string()).collect();
    out.extend(PRIVATE_NETS.iter().map(|(n, b)| format!("{n}/{b}")));
    for n in names {
        if permitted(&out, n) {
            continue;
        }
        match n.parse::<IpAddr>() {
            Ok(IpAddr::V4(_)) => out.push(format!("{n}/32")),
            Ok(IpAddr::V6(_)) => out.push(format!("{n}/128")),
            Err(_) => out.push(n.to_ascii_lowercase()),
        }
    }
    out
}

fn constraints(allowed: &[String]) -> NameConstraints {
    let permitted_subtrees = allowed
        .iter()
        .filter_map(|a| match a.split_once('/') {
            Some((net, bits)) => {
                Some(GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(net.parse().ok()?, bits.parse().ok()?)))
            }
            None => Some(GeneralSubtree::DnsName(a.clone())),
        })
        .collect();
    NameConstraints { permitted_subtrees, excluded_subtrees: Vec::new() }
}

/// The certificate authority (made once) and a server certificate for
/// `names` signed by it, as PEM: (ca, cert chain, key).
pub fn local_certificate(dir: &Path, names: &[String]) -> Result<(String, String, String), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (ca_pem, ca_key, ca_names) = (dir.join("ca.pem"), dir.join("ca.key"), dir.join("ca-names.json"));
    // What the authority may vouch for (none saved: one made before limits).
    let mut allowed: Option<Vec<String>> =
        std::fs::read_to_string(&ca_names).ok().and_then(|t| serde_json::from_str(&t).ok());
    let issuer = if ca_pem.exists() && ca_key.exists() {
        let key = KeyPair::from_pem(&std::fs::read_to_string(&ca_key).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Issuer::from_ca_cert_pem(&std::fs::read_to_string(&ca_pem).map_err(|e| e.to_string())?, key)
            .map_err(|e| e.to_string())?
    } else {
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let limits = allowed_for(names);
        params.name_constraints = Some(constraints(&limits));
        params.distinguished_name.push(
            DnType::CommonName,
            format!("Xenify server on {}", host_name().unwrap_or_else(|| "this computer".into())),
        );
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after = rcgen::date_time_ymd(2046, 1, 1);
        let key = KeyPair::generate().map_err(|e| e.to_string())?;
        let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
        write_private(&ca_key, &key.serialize_pem())?;
        std::fs::write(&ca_pem, cert.pem()).map_err(|e| e.to_string())?;
        std::fs::write(&ca_names, serde_json::to_string(&limits).unwrap()).map_err(|e| e.to_string())?;
        allowed = Some(limits);
        Issuer::new(params, key)
    };
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    // A name the authority can't vouch for would spoil the whole certificate.
    params.subject_alt_names = names
        .iter()
        .filter(|n| allowed.as_ref().is_none_or(|a| permitted(a, n)))
        .filter_map(|n| match n.parse::<IpAddr>() {
            Ok(ip) => Some(SanType::IpAddress(ip)),
            Err(_) => n.clone().try_into().ok().map(SanType::DnsName),
        })
        .collect();
    params.distinguished_name.push(DnType::CommonName, "Xenify server");
    // Browsers refuse server certificates valid for over 398 days; a new one
    // is made at every start.
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(390);
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.signed_by(&key, &issuer).map_err(|e| e.to_string())?;
    let ca = std::fs::read_to_string(&ca_pem).map_err(|e| e.to_string())?;
    Ok((ca.clone(), format!("{}{}", cert.pem(), ca), key.serialize_pem()))
}

/// The authority's SHA-256 fingerprint ("AB:CD:…", what the desktop apps
/// show before trusting the server) and whether it's limited to this
/// network (one made before limits isn't).
pub fn authority(dir: &Path) -> Option<(String, bool)> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    use sha2::{Digest, Sha256};
    let pem = std::fs::read_to_string(dir.join("ca.pem")).ok()?;
    let der = CertificateDer::from_pem_slice(pem.as_bytes()).ok()?;
    let fp = Sha256::digest(der.as_ref()).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    Some((fp, dir.join("ca-names.json").exists()))
}

/// Throws the authority away; the next certificate makes a new one.
pub fn forget_authority(dir: &Path) -> Result<(), String> {
    for f in ["ca.pem", "ca.key", "ca-names.json"] {
        match std::fs::remove_file(dir.join(f)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub(crate) fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    // Private from the start (never readable by others, not even briefly),
    // and put in place whole, so a reader never sees half of it.
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        // An earlier leftover keeps the mode it was made with.
        if tmp.exists() {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        }
    }
    let mut f = options.open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// A TLS configuration for a certificate chain and key (PEM).
pub fn config(chain: &str, key: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(chain.as_bytes()).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(|e| e.to_string())?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    let mut config = config;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Certificates from Let's Encrypt for the server's domain names, renewed
/// in the background (until dropped).
pub struct Acme {
    pub domains: Vec<String>,
    /// Answers Let's Encrypt's checks (TLS-ALPN-01, on port 443).
    challenge: Arc<rustls::ServerConfig>,
    normal: Arc<rustls::ServerConfig>,
    /// "waiting", "ready", or what went wrong last.
    pub status: Arc<std::sync::Mutex<String>>,
    /// Let's Encrypt's real service (not staging): browsers trust these.
    pub production: bool,
    task: tokio::task::AbortHandle,
}

impl Drop for Acme {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The domain names' certificates in use, if any (changed on Admin › Network
/// while the server runs).
pub type AcmeSlot = Arc<std::sync::RwLock<Option<Arc<Acme>>>>;

/// Where HTTPS certificates come from: Let's Encrypt's for the domain
/// names, the server's own for every other name and address (the network's).
#[derive(Clone)]
pub struct Certs {
    pub own: Arc<rustls::ServerConfig>,
    pub acme: AcmeSlot,
}

/// Certificates from Let's Encrypt for `domains`, kept in `dir`.
/// `production`: false uses Let's Encrypt's staging service (for trying it
/// out: browsers don't trust its certificates).
pub fn acme(domains: Vec<String>, email: Option<String>, dir: &Path, production: bool) -> Acme {
    use futures_util::StreamExt;
    let mut state = rustls_acme::AcmeConfig::new(domains.clone())
        .contact(email.into_iter().map(|e| format!("mailto:{e}")))
        .cache(rustls_acme::caches::DirCache::new(dir.to_path_buf()))
        .directory_lets_encrypt(production)
        .state();
    let challenge = state.challenge_rustls_config();
    let mut normal = (*state.default_rustls_config()).clone();
    normal.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let status = Arc::new(std::sync::Mutex::new("waiting".to_string()));
    let shown = status.clone();
    let task = tokio::spawn(async move {
        while let Some(event) = state.next().await {
            let now = match &event {
                Ok(rustls_acme::EventOk::DeployedCachedCert | rustls_acme::EventOk::DeployedNewCert) => {
                    Some("ready".to_string())
                }
                Ok(_) => None,
                Err(e) => Some(e.to_string()),
            };
            match &event {
                Ok(e) => println!("HTTPS certificate: {e:?}"),
                Err(e) => eprintln!("xenify-server: HTTPS certificate: {e}"),
            }
            if let Some(now) = now {
                *shown.lock().expect("not poisoned") = now;
            }
        }
    })
    .abort_handle();
    Acme { domains, challenge, normal: Arc::new(normal), status, production, task }
}

/// A TLS connection, or None (a Let's Encrypt check, answered, or a
/// failure), and whether its certificate is one browsers trust anyway.
async fn handshake(
    certs: &Certs,
    tcp: tokio::net::TcpStream,
) -> Option<(tokio_rustls::server::TlsStream<tokio::net::TcpStream>, bool)> {
    let start = tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tcp).await.ok()?;
    let acme = certs.acme.read().expect("not poisoned").clone();
    let hello = start.client_hello();
    let named =
        acme.filter(|a| hello.server_name().is_some_and(|n| a.domains.iter().any(|d| d.eq_ignore_ascii_case(n))));
    match named {
        Some(a) if rustls_acme::is_tls_alpn_challenge(&hello) => {
            use tokio::io::AsyncWriteExt;
            let mut tls = start.into_stream(a.challenge.clone()).await.ok()?;
            let _ = tls.shutdown().await;
            None
        }
        Some(a) => start.into_stream(a.normal.clone()).await.ok().map(|t| (t, a.production)),
        None => start.into_stream(certs.own.clone()).await.ok().map(|t| (t, false)),
    }
}

/// How long a client may take to send a request's headers, and to start
/// the next one on a kept-open connection (slow clients can't hold
/// connections open for ever).
pub const HEADER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Serves one connection until it closes, or (once `stop` flips) finishes
/// what it's doing and closes.
async fn serve_connection<I>(io: I, router: axum::Router, mut stop: tokio::sync::watch::Receiver<bool>)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
    use hyper_util::server::conn::auto::Builder;
    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let mut svc = router.clone();
        async move {
            use tower::Service;
            svc.call(req.map(axum::body::Body::new)).await
        }
    });
    let mut b = Builder::new(TokioExecutor::new());
    b.http1().timer(TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
    // HTTP/2: a connection that stops answering pings is closed.
    b.http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Some(std::time::Duration::from_secs(60)))
        .keep_alive_timeout(std::time::Duration::from_secs(20));
    let conn = b.serve_connection_with_upgrades(TokioIo::new(io), svc);
    tokio::pin!(conn);
    tokio::select! {
        _ = conn.as_mut() => return,
        _ = stop.wait_for(|s| *s) => conn.as_mut().graceful_shutdown(),
    }
    let _ = conn.await;
}

/// Serves `router` over HTTPS on `listener` until `stop` finishes.
pub async fn serve(
    listener: tokio::net::TcpListener,
    certs: Certs,
    router: axum::Router,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let (tcp, peer) = tokio::select! {
            r = listener.accept() => match r { Ok(x) => x, Err(_) => continue },
            _ = stop.wait_for(|s| *s) => return,
        };
        let _ = tcp.set_nodelay(true);
        let certs = certs.clone();
        let router = router.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let Ok(Some((tls, public))) =
                tokio::time::timeout(std::time::Duration::from_secs(10), handshake(&certs, tcp)).await
            else {
                return;
            };
            let mut svc = router
                .layer(axum::Extension(axum::extract::ConnectInfo(peer)))
                .layer(axum::Extension(crate::Secure));
            if public {
                svc = svc.layer(axum::Extension(crate::PublicCert));
            }
            serve_connection(tls, svc, stop).await;
        });
    }
}

/// Serves `router` over plain HTTP on `listener` until `stop` flips, then
/// waits for its connections to finish.
pub async fn serve_plain(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let mut open = tokio::task::JoinSet::new();
    loop {
        let (tcp, peer) = tokio::select! {
            r = listener.accept() => match r { Ok(x) => x, Err(_) => continue },
            _ = stop.wait_for(|s| *s) => break,
        };
        while open.try_join_next().is_some() {}
        let _ = tcp.set_nodelay(true);
        let svc = router.clone().layer(axum::Extension(axum::extract::ConnectInfo(peer)));
        open.spawn(serve_connection(tcp, svc, stop.clone()));
    }
    drop(listener);
    while open.join_next().await.is_some() {}
}

/// Lets apps on the network find the server (mDNS, `_xenify._tcp`). Keep
/// the daemon alive for as long as the server is announced.
pub fn announce(name: &str, port: u16, https: Option<u16>, id: &str) -> Result<mdns_sd::ServiceDaemon, String> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let host =
        std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()).unwrap_or_else(|_| "xenify".into());
    let mut props = vec![("id", id.to_string()), ("v", crate::VERSION.to_string())];
    if let Some(p) = https {
        props.push(("https", p.to_string()));
    }
    let info = mdns_sd::ServiceInfo::new("_xenify._tcp.local.", name, &format!("{host}.local."), "", port, &props[..])
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
    daemon.register(info).map_err(|e| e.to_string())?;
    Ok(daemon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authority_vouches_only_for_this_network_and_computer() {
        let allowed = allowed_for(&["office-pc".into(), "203.0.113.9".into(), "192.168.1.5".into()]);
        for ok in ["office-pc", "office-pc.local", "nas.lan", "192.168.1.5", "10.2.3.4", "203.0.113.9", "fe80::1", "localhost"] {
            assert!(permitted(&allowed, ok), "{ok}");
        }
        for no in ["example.com", "google.com", "local.example.com", "8.8.8.8", "203.0.113.10", "2001:db8::1"] {
            assert!(!permitted(&allowed, no), "{no}");
        }
    }

    #[test]
    fn certificates_check_out_against_their_limited_authority() {
        let dir = std::env::temp_dir().join(format!("xenify-tls-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let names: Vec<String> = vec!["localhost".into(), "office-pc".into(), "192.168.1.5".into()];
        let (ca, chain, _) = local_certificate(&dir, &names).unwrap();
        // A name added later that the authority can't vouch for is left out.
        let (_, chain2, _) = local_certificate(&dir, &[names.clone(), vec!["shop.example.com".into()]].concat()).unwrap();
        use rustls::client::danger::ServerCertVerifier;
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from_pem_slice(ca.as_bytes()).unwrap()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .unwrap();
        for chain in [&chain, &chain2] {
            let certs: Vec<CertificateDer> =
                CertificateDer::pem_slice_iter(chain.as_bytes()).collect::<Result<_, _>>().unwrap();
            for name in ["office-pc", "192.168.1.5"] {
                verifier
                    .verify_server_cert(&certs[0], &certs[1..], &ServerName::try_from(name).unwrap(), &[], UnixTime::now())
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
