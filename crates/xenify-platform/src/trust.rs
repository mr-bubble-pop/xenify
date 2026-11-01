//! Servers with their own certificate (a Xenify server on a home or office
//! network has no domain name, so it signs its HTTPS certificate with an
//! authority it made itself). The apps can't check such a certificate, so
//! the first time they ask: trust this server (remembered for this address,
//! by the authority's fingerprint), ignore it this time, or don't connect.
//! Nothing is added to the computer's own certificate list.
//!
//! A trusted authority vouches for its own server's address only: HTTPS to
//! any other site is checked against the usual authorities alone, so a
//! server's own authority (or whoever holds its key) can't stand in for
//! other sites.

use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What a server showed.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Seen {
    /// "host:port".
    pub host: String,
    /// The server's certificate (PEM), for the web view to allow.
    pub certificate: String,
    /// The authority that signed it (its own, when there's one in the
    /// chain; else the certificate itself), as SHA-256 "AB:CD:…".
    pub fingerprint: String,
    /// The authority's name, e.g. "Xenify server on office-pc".
    pub name: String,
    /// Remembered as trusted for this address.
    pub trusted: bool,
    /// Trusted before, but the authority is a different one now.
    pub changed: bool,
    /// Ignored for now (until the app closes).
    pub ignored: bool,
}

/// "host:port" of an https:// address.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?.to_string();
    if host.is_empty() {
        return None;
    }
    Some(if host.ends_with(']') || !host.contains(':') { format!("{host}:443") } else { host })
}

#[derive(Debug)]
struct Accept(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for Accept {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The certificates `host` ("name:port") sends, leaf first.
fn chain(host: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Accept(provider)))
        .with_no_client_auth();
    let name = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host).trim_matches(['[', ']']);
    let server = ServerName::try_from(name.to_string()).map_err(|e| e.to_string())?;
    let addr =
        host.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or_else(|| format!("{host} has no address"))?;
    let mut tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(6)).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(6))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(6))).ok();
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server).map_err(|e| e.to_string())?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp).map_err(|e| e.to_string())?;
    }
    let _ = conn.writer().flush();
    let certs = conn.peer_certificates().map(|c| c.to_vec()).unwrap_or_default();
    if certs.is_empty() {
        return Err(format!("{host} sent no certificate"));
    }
    Ok(certs)
}

fn pem(der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let lines: Vec<&str> = b64.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
}

fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// The common name in a certificate, found by its object id (2.5.4.3)
/// followed by a string; good enough for showing, never for checking.
fn common_name(der: &[u8]) -> String {
    let oid = [0x06, 0x03, 0x55, 0x04, 0x03];
    // The subject comes after the issuer, so the last one is the subject's.
    let Some(at) = der.windows(5).rposition(|w| w == oid) else { return String::new() };
    let rest = &der[at + 5..];
    match rest {
        [tag, len, body @ ..] if matches!(tag, 0x0c | 0x13 | 0x16) && (*len as usize) <= body.len() => {
            String::from_utf8_lossy(&body[..*len as usize]).into_owned()
        }
        _ => String::new(),
    }
}

/// Saved: address → the authority's fingerprint.
fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").filter(|_| cfg!(windows)).map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("xenify").join("trusted-servers.json"))
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    fingerprint: String,
    /// The authority (PEM), for HTTPS made outside the web view.
    authority: String,
}

/// The file as JSON, entries kept as they are (so ones this version can't
/// read, from a newer app, survive a save). None: it can't be read at all.
fn raw() -> Option<serde_json::Map<String, serde_json::Value>> {
    match path().map(std::fs::read_to_string) {
        Some(Ok(s)) => serde_json::from_str(&s).ok(),
        _ => Some(Default::default()),
    }
}

/// Changes the file: written whole and swapped in at once. A file that
/// can't be read is kept aside first ("….unreadable-<time>"), not lost.
fn store(change: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>)) -> Result<(), String> {
    let p = path().ok_or("no settings folder")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut all = raw().unwrap_or_else(|| {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = std::fs::rename(&p, p.with_extension(format!("json.unreadable-{at}")));
        Default::default()
    });
    change(&mut all);
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&all).unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

fn saved() -> BTreeMap<String, Saved> {
    raw()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(host, v)| Some((host, serde_json::from_value(v).ok()?)))
        .collect()
}

fn ignored() -> &'static Mutex<BTreeMap<String, Saved>> {
    static IGNORED: Mutex<BTreeMap<String, Saved>> = Mutex::new(BTreeMap::new());
    &IGNORED
}

/// Addresses ("host:port") trusted or ignored while this app runs.
fn here() -> &'static Mutex<std::collections::BTreeSet<String>> {
    static HERE: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());
    &HERE
}

/// The addresses trusted or ignored while this app runs ("host:port").
pub fn trusted_here() -> Vec<String> {
    here().lock().unwrap().iter().cloned().collect()
}

/// Connects to `url` (https://) and says what its certificate is and
/// whether this computer said to trust it.
pub fn look(url: &str) -> Result<Seen, String> {
    let host = host_of(url).ok_or("not an https:// address")?;
    let certs = chain(&host)?;
    let authority = if certs.len() > 1 { certs.last().unwrap() } else { &certs[0] };
    let fp = fingerprint(authority);
    let known = saved().get(&host).map(|s| s.fingerprint.clone());
    let ignored = ignored().lock().unwrap().get(&host).is_some_and(|s| s.fingerprint == fp);
    Ok(Seen {
        certificate: pem(&certs[0]),
        name: common_name(authority),
        trusted: known.as_deref() == Some(fp.as_str()),
        changed: known.is_some_and(|k| k != fp),
        ignored,
        fingerprint: fp,
        host,
    })
}

/// Trusts the server at `url` from now on (`remember`), or until the app
/// closes, if it still shows the authority with `fingerprint` (what the
/// person was shown). Returns what it shows, to allow in the web view.
pub fn trust(url: &str, fingerprint: &str, remember: bool) -> Result<Seen, String> {
    let mut seen = look(url)?;
    if seen.fingerprint != fingerprint {
        return Err("The server's certificate changed while you were deciding. Try again.".into());
    }
    let certs = chain(&seen.host)?;
    let authority = if certs.len() > 1 { certs.last().unwrap() } else { &certs[0] };
    let entry = Saved { fingerprint: seen.fingerprint.clone(), authority: pem(authority) };
    here().lock().unwrap().insert(seen.host.clone());
    if remember {
        let value = serde_json::to_value(&entry).map_err(|e| e.to_string())?;
        store(|all| {
            all.insert(seen.host.clone(), value);
        })?;
        seen.trusted = true;
        seen.changed = false;
    } else {
        ignored().lock().unwrap().insert(seen.host.clone(), entry);
        seen.ignored = true;
    }
    Ok(seen)
}

/// Forgets a server (Settings › Server › forget).
pub fn forget(url: &str) {
    let Some(host) = host_of(url) else { return };
    ignored().lock().unwrap().remove(&host);
    if raw().is_some_and(|all| all.contains_key(&host)) {
        let _ = store(|all| {
            all.remove(&host);
        });
    }
}

/// Lets a web view (WebKitGTK) accept what `seen` showed for its host.
#[cfg(target_os = "linux")]
pub fn allow(view: &webkit2gtk::WebView, seen: &Seen) {
    use webkit2gtk::{WebContextExt, WebViewExt};
    let Ok(cert) = webkit2gtk::gio::TlsCertificate::from_pem(&seen.certificate) else { return };
    let name = seen.host.rsplit_once(':').map(|(h, _)| h).unwrap_or(&seen.host).trim_matches(['[', ']']);
    if let Some(context) = view.context() {
        context.allow_tls_certificate_for_host(&cert, name);
    }
}

/// The authorities of trusted (or for now ignored) servers, PEM, with the
/// address each is trusted for ("host:port").
pub fn authorities() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = saved().into_iter().map(|(h, s)| (h, s.authority)).collect();
    out.extend(ignored().lock().unwrap().iter().map(|(h, s)| (h.clone(), s.authority.clone())));
    out
}

/// The authorities trusted for these addresses ("host:port") only, PEM.
pub fn authorities_for(hosts: &[String]) -> Vec<String> {
    authorities().into_iter().filter(|(h, _)| hosts.contains(h)).map(|(_, a)| a).collect()
}

/// A host's name as a certificate is checked for it: no port or brackets,
/// lower case; an IP address written the standard way.
fn name_of(host: &str) -> String {
    let name = match host.rsplit_once(':') {
        Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) && (!h.contains(':') || h.ends_with(']')) => h,
        _ => host,
    };
    let name = name.trim_matches(['[', ']']).trim_end_matches('.').to_ascii_lowercase();
    name.parse::<std::net::IpAddr>().map(|ip| ip.to_string()).unwrap_or(name)
}

/// Checks a server's certificate: for an address trusted here by its own
/// authority, against that authority (or, failing that, the usual ones);
/// for every other address against the usual authorities only.
#[derive(Debug)]
pub struct HostVerifier {
    /// Host name (see `name_of`) → its trusted authorities only.
    hosts: BTreeMap<String, Arc<rustls::client::WebPkiServerVerifier>>,
    /// The usual authorities (the system's or the built-in public ones).
    web: Arc<dyn ServerCertVerifier>,
}

impl HostVerifier {
    /// `trusted`: (address "host:port" or host, the authority). Authorities
    /// that can't be read are left out.
    pub fn new(
        trusted: &[(String, CertificateDer<'static>)],
        web: Arc<dyn ServerCertVerifier>,
        provider: Arc<rustls::crypto::CryptoProvider>,
    ) -> Self {
        let mut stores: BTreeMap<String, rustls::RootCertStore> = BTreeMap::new();
        for (host, cert) in trusted {
            let _ = stores.entry(name_of(host)).or_insert_with(rustls::RootCertStore::empty).add(cert.clone());
        }
        let hosts = stores
            .into_iter()
            .filter(|(_, s)| !s.is_empty())
            .filter_map(|(h, s)| {
                rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(s), provider.clone())
                    .build()
                    .ok()
                    .map(|v| (h, v))
            })
            .collect();
        Self { hosts, web }
    }

    /// With the servers trusted (or ignored for now) on this computer.
    pub fn saved(web: Arc<dyn ServerCertVerifier>, provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        use rustls::pki_types::pem::PemObject;
        let trusted: Vec<(String, CertificateDer<'static>)> = authorities()
            .into_iter()
            .filter_map(|(h, pem)| CertificateDer::from_pem_slice(pem.as_bytes()).ok().map(|c| (h, c)))
            .collect();
        Self::new(&trusted, web, provider)
    }
}

impl ServerCertVerifier for HostVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let name = match server_name {
            ServerName::DnsName(d) => name_of(d.as_ref()),
            ServerName::IpAddress(ip) => std::net::IpAddr::from(*ip).to_string(),
            _ => String::new(),
        };
        if let Some(own) = self.hosts.get(&name) {
            return match own.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
                Ok(v) => Ok(v),
                // Since given a certificate from a public authority.
                Err(e) => self.web.verify_server_cert(end_entity, intermediates, server_name, ocsp, now).map_err(|_| e),
            };
        }
        self.web.verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.web.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.web.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.web.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_get_their_port() {
        assert_eq!(host_of("https://192.168.1.5:7443/v1/info").as_deref(), Some("192.168.1.5:7443"));
        assert_eq!(host_of("https://files.example.com").as_deref(), Some("files.example.com:443"));
        assert_eq!(host_of("https://[fe80::1]").as_deref(), Some("[fe80::1]:443"));
        assert_eq!(host_of("http://x:7700"), None);
    }
    // Made with openssl for these tests (valid for 100 years): authorities
    // A and B; a.test (also 192.168.1.5) and b.test signed by A; b.test by B.
    const A: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBoTCCAUegAwIBAgIUB9Iy+gypFNPXsi5nXho1tb4ctEwwCgYIKoZIzj0EAwIw\n\
HTEbMBkGA1UEAwwSWGVuaWZ5IHNlcnZlciBvbiBhMCAXDTI2MTEwMTEzMTExN1oY\n\
DzIxMjYxMDA4MTMxMTE3WjAdMRswGQYDVQQDDBJYZW5pZnkgc2VydmVyIG9uIGEw\n\
WTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAASoBW1uu4oI7NC1uqQcg0ERAfkqlR9D\n\
QI82YKdxzE3gyjYHXQp2ofPbrvUw8O46dHOFaO8gcVva4KH+DxS8y5Qmo2MwYTAd\n\
BgNVHQ4EFgQUEr7W+lq4E+S+SL0RfvzBfm4nzeYwHwYDVR0jBBgwFoAUEr7W+lq4\n\
E+S+SL0RfvzBfm4nzeYwDwYDVR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8EBAMCAQYw\n\
CgYIKoZIzj0EAwIDSAAwRQIgYrDyLGtkKFKsDHvW3kmuljf25RFI5HsKQ+PeTUKS\n\
XOECIQDT4opis7UyIG0dinzSp9dI3tMsHjVuvDp/v3mlOEAJow==\n\
-----END CERTIFICATE-----\n";
    const B: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBoTCCAUegAwIBAgIUYrS7yatC380K494JppZBl8jvzGUwCgYIKoZIzj0EAwIw\n\
HTEbMBkGA1UEAwwSWGVuaWZ5IHNlcnZlciBvbiBiMCAXDTI2MTEwMTEzMTExN1oY\n\
DzIxMjYxMDA4MTMxMTE3WjAdMRswGQYDVQQDDBJYZW5pZnkgc2VydmVyIG9uIGIw\n\
WTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAQ9hRoFI30xu4c0es/3NzFBTETL/qWY\n\
CgPLQddOGcLJ49C+7rSJhYhIjbgoS0iYJdzMHp2GAt4RYBSNcmKESP1lo2MwYTAd\n\
BgNVHQ4EFgQUmAnQUlVh58sCIPq2gLS2Ndj9uQUwHwYDVR0jBBgwFoAUmAnQUlVh\n\
58sCIPq2gLS2Ndj9uQUwDwYDVR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8EBAMCAQYw\n\
CgYIKoZIzj0EAwIDSAAwRQIhAIIgvJ8S3SGeDnlzTDxmV4A9LxkD1K4w6L2bXAf6\n\
uHejAiApNjK2wL8Q7ZEx4+O9Vb6IpfZd5c8NVbh+Sznti5tlKg==\n\
-----END CERTIFICATE-----\n";
    const LEAF_A: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBvDCCAWKgAwIBAgIUc0Lxi2tormmIMkdLn65dVUtrcH4wCgYIKoZIzj0EAwIw\n\
HTEbMBkGA1UEAwwSWGVuaWZ5IHNlcnZlciBvbiBhMCAXDTI2MTEwMTEzMTExN1oY\n\
DzIxMjYxMDA4MTMxMTE3WjARMQ8wDQYDVQQDDAZsZWFmX2EwWTATBgcqhkjOPQIB\n\
BggqhkjOPQMBBwNCAAS+lf2LRb2dNrx1ySJgWljaBMFo4il5tdOEqUgR8QG4ycg4\n\
/wonjjHxS2QoXwHYOTEx1J1PHbQavmwOGGnNhLBPo4GJMIGGMBcGA1UdEQQQMA6C\n\
BmEudGVzdIcEwKgBBTAJBgNVHRMEAjAAMBMGA1UdJQQMMAoGCCsGAQUFBwMBMAsG\n\
A1UdDwQEAwIHgDAdBgNVHQ4EFgQUx0KCrKOHtyMqIdJXPgAuivRZ2NYwHwYDVR0j\n\
BBgwFoAUEr7W+lq4E+S+SL0RfvzBfm4nzeYwCgYIKoZIzj0EAwIDSAAwRQIhAIWd\n\
50eulmmdxDZVC0mK4driQIuedD+2bZg76l5WcvDiAiBbCTFfMRjgYyb4KfnBfPp5\n\
bwx/MIHiHppH5qhk90eF6A==\n\
-----END CERTIFICATE-----\n";
    const LEAF_B_BY_A: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBuzCCAWGgAwIBAgIUc0Lxi2tormmIMkdLn65dVUtrcH8wCgYIKoZIzj0EAwIw\n\
HTEbMBkGA1UEAwwSWGVuaWZ5IHNlcnZlciBvbiBhMCAXDTI2MTEwMTEzMTExN1oY\n\
DzIxMjYxMDA4MTMxMTE3WjAWMRQwEgYDVQQDDAtsZWFmX2JfYnlfYTBZMBMGByqG\n\
SM49AgEGCCqGSM49AwEHA0IABEVs++Tm5GhNi5P7c4821rclma0QGDKs9gmsmkBm\n\
fR17dZ1IraxgxTN1493iYaoZJb/jCD/F5pLpaT/KAaK53VSjgYMwgYAwEQYDVR0R\n\
BAowCIIGYi50ZXN0MAkGA1UdEwQCMAAwEwYDVR0lBAwwCgYIKwYBBQUHAwEwCwYD\n\
VR0PBAQDAgeAMB0GA1UdDgQWBBQDaVDSovv4t10Hliqariy2PpcheTAfBgNVHSME\n\
GDAWgBQSvtb6WrgT5L5IvRF+/MF+bifN5jAKBggqhkjOPQQDAgNIADBFAiBYNo7S\n\
Yb7bJh9XHqQIGdE/0W7q2dcFo4fip4WcR8hmQwIhAJTwgdEexQ7C/IAfUTYlEqfN\n\
40EkdUYdBNFnVy/OGEdx\n\
-----END CERTIFICATE-----\n";
    const LEAF_B: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBtzCCAVygAwIBAgIUIHOg1+PAiJVKj+rdeqeeJPJVzzEwCgYIKoZIzj0EAwIw\n\
HTEbMBkGA1UEAwwSWGVuaWZ5IHNlcnZlciBvbiBiMCAXDTI2MTEwMTEzMTExN1oY\n\
DzIxMjYxMDA4MTMxMTE3WjARMQ8wDQYDVQQDDAZsZWFmX2IwWTATBgcqhkjOPQIB\n\
BggqhkjOPQMBBwNCAASMppCXmQlOcaAjD25C0UBP8q0kz7pRJTjm0FKnWb/qDCCU\n\
WZnH2s4WnFSx60dwt1mr/fCVxG4SgmkrZtw9qzKIo4GDMIGAMBEGA1UdEQQKMAiC\n\
BmIudGVzdDAJBgNVHRMEAjAAMBMGA1UdJQQMMAoGCCsGAQUFBwMBMAsGA1UdDwQE\n\
AwIHgDAdBgNVHQ4EFgQUlDbq2u6oeMRAJnSbD46OGFUtu1cwHwYDVR0jBBgwFoAU\n\
mAnQUlVh58sCIPq2gLS2Ndj9uQUwCgYIKoZIzj0EAwIDSQAwRgIhAIE+J3y5IR//\n\
+7CVT8Sggaby0GkBpeQneKZ5DBxCxUiHAiEA3ECyE/jby4o5/50lCjRYDxXBgefA\n\
YxPufRZDnGKsxhY=\n\
-----END CERTIFICATE-----\n";

    fn der(pem: &str) -> CertificateDer<'static> {
        use rustls::pki_types::pem::PemObject;
        CertificateDer::from_pem_slice(pem.as_bytes()).unwrap()
    }

    /// Trusts A for a.test:7443; B stands for the usual public authorities.
    fn verifier() -> HostVerifier {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut web = rustls::RootCertStore::empty();
        web.add(der(B)).unwrap();
        let web = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(web), provider.clone())
            .build()
            .unwrap();
        HostVerifier::new(&[("a.test:7443".into(), der(A))], web, provider)
    }

    fn check(v: &HostVerifier, leaf: &str, name: &str) -> bool {
        let name = ServerName::try_from(name.to_string()).unwrap();
        v.verify_server_cert(&der(leaf), &[], &name, &[], UnixTime::now()).is_ok()
    }

    #[test]
    fn a_trusted_authority_vouches_for_its_own_server_only() {
        let v = verifier();
        assert!(check(&v, LEAF_A, "a.test"), "the trusted server");
        assert!(check(&v, LEAF_A, "A.Test"), "however it's written");
        assert!(!check(&v, LEAF_B_BY_A, "b.test"), "A signing for another site is refused");
        assert!(!check(&v, LEAF_A, "192.168.1.5"), "an address not trusted for A, though A signed it");
        assert!(check(&v, LEAF_B, "b.test"), "other sites: the usual authorities");
        assert!(!check(&v, LEAF_B, "a.test"), "the usual name check still applies");
    }

    #[test]
    fn a_server_trusted_by_its_address_is_checked_by_it() {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut web = rustls::RootCertStore::empty();
        web.add(der(B)).unwrap();
        let web = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(web), provider.clone())
            .build()
            .unwrap();
        let v = HostVerifier::new(&[("192.168.1.5:7443".into(), der(A))], web, provider);
        assert!(check(&v, LEAF_A, "192.168.1.5"));
        assert!(!check(&v, LEAF_A, "a.test"));
        assert!(!check(&v, LEAF_B_BY_A, "b.test"));
    }

    #[test]
    fn names_are_compared_without_port_or_brackets() {
        assert_eq!(name_of("Files.Example.com:443"), "files.example.com");
        assert_eq!(name_of("[FE80::1]:7443"), "fe80::1");
        assert_eq!(name_of("192.168.1.5:7443"), "192.168.1.5");
        assert_eq!(name_of("fe80::1"), "fe80::1");
    }
}

/// Certificates ureq should accept for HTTPS to `url`: the usual ones,
/// plus the authority trusted here for that address, if any (for no other).
pub fn ureq_tls_for(url: &str) -> ureq::tls::TlsConfig {
    ureq_tls_for_hosts(&host_of(url).into_iter().collect::<Vec<_>>())
}

/// The same for an agent that only talks to these addresses ("host:port").
pub fn ureq_tls_for_hosts(hosts: &[String]) -> ureq::tls::TlsConfig {
    use ureq::tls::{Certificate, RootCerts, TlsConfig};
    let trusted = authorities_for(hosts);
    if trusted.is_empty() {
        return TlsConfig::builder().build();
    }
    let mut certs: Vec<Certificate<'static>> = rustls_native_certs::load_native_certs()
        .certs
        .iter()
        .map(|c| Certificate::from_der(c.as_ref()).to_owned())
        .collect();
    for pem in trusted {
        if let Ok(c) = Certificate::from_pem(pem.as_bytes()) {
            certs.push(c.to_owned());
        }
    }
    TlsConfig::builder().root_certs(RootCerts::Specific(Arc::new(certs))).build()
}

/// Whether an HTTPS error means the certificate isn't trusted here.
pub fn untrusted(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("unknownissuer") || e.contains("unknown issuer") || e.contains("certificate") && e.contains("invalid")
}
