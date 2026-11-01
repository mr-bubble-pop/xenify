//! Which certificates the app trusts for HTTPS: this computer's own (the
//! system's list, where "Trust this server" on a Xenify server's admin
//! pages puts the server's own authority), so a server on the office
//! network works over HTTPS like any public site. Without a system list,
//! the public authorities built into the app. Plus servers trusted in the
//! app itself by their fingerprint (or ignored this time), shared with the
//! other Xenify apps (xenify_platform::trust): such a server's authority
//! is used for that server's address only, never for other sites.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

fn roots() -> RootCerts {
    static ROOTS: OnceLock<RootCerts> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let certs: Vec<Certificate<'static>> = rustls_native_certs::load_native_certs()
                .certs
                .iter()
                .map(|c| Certificate::from_der(c.as_ref()).to_owned())
                .collect();
            if certs.is_empty() { RootCerts::WebPki } else { RootCerts::Specific(Arc::new(certs)) }
        })
        .clone()
}

/// The servers Chat talks to ("host:port"): its own (as saved, or given
/// when it started), the Hub's, and any trusted while it runs. Chat's own
/// requests go to these only, so a trusted authority isn't used elsewhere.
fn servers() -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let saved = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|b| b.join("xenify").join("chat.json"));
    if let Some(v) = saved
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        && let Some(s) = v["server"].as_str()
    {
        urls.push(s.to_string());
    }
    if let Ok(s) = std::env::var("XENIFY_SERVER") {
        urls.push(s);
    }
    if let Some(h) = xenify_hub_core::signin::read() {
        urls.push(h.url);
    }
    let mut hosts: Vec<String> = urls.iter().filter_map(|u| xenify_platform::trust::host_of(u.trim())).collect();
    hosts.extend(xenify_platform::trust::trusted_here());
    hosts
}

/// The system's (or built-in) authorities, and those of servers trusted
/// here for `hosts` ("host:port") only.
fn all(hosts: &[String]) -> RootCerts {
    let trusted = xenify_platform::trust::authorities_for(hosts);
    if trusted.is_empty() {
        return roots();
    }
    // (No system list: only the trusted servers, which is all Chat talks to.)
    let mut certs: Vec<Certificate<'static>> = match roots() {
        RootCerts::Specific(c) => c.iter().cloned().collect(),
        _ => Vec::new(),
    };
    for pem in trusted {
        if let Ok(c) = Certificate::from_pem(pem.as_bytes()) {
            certs.push(c.to_owned());
        }
    }
    RootCerts::Specific(Arc::new(certs))
}

/// For every ureq agent the app makes (they talk to Chat's server only).
pub fn tls() -> TlsConfig {
    TlsConfig::builder().root_certs(all(&servers())).build()
}

/// For an agent that talks to `server` only (an address).
#[allow(dead_code)]
pub fn tls_for(server: &str) -> TlsConfig {
    TlsConfig::builder().root_certs(all(&xenify_platform::trust::host_of(server).into_iter().collect::<Vec<_>>())).build()
}

/// For the chat connection (WebSocket): each trusted server's authority
/// for its own address only; every other address the usual authorities.
pub fn websocket() -> Option<tokio_tungstenite::Connector> {
    if xenify_platform::trust::authorities().is_empty() {
        return None;
    }
    let mut store = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs().certs;
    if native.is_empty() {
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    } else {
        store.add_parsable_certificates(native);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let web = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(store), provider.clone()).build().ok()?;
    let verifier = xenify_platform::trust::HostVerifier::saved(web, provider.clone());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Some(tokio_tungstenite::Connector::Rustls(Arc::new(config)))
}

/// What to tell someone when HTTPS failed because the server's certificate
/// isn't trusted here (None: something else went wrong).
pub fn untrusted(error: &str) -> Option<String> {
    let e = error.to_lowercase();
    (e.contains("unknownissuer") || e.contains("unknown issuer") || e.contains("certificate") && e.contains("invalid"))
        .then(|| {
            "This computer doesn't trust the server's certificate yet.".to_string()
        })
}
