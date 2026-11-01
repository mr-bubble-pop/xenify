//! Xenify servers on this network (they announce themselves with mDNS as
//! `_xenify._tcp`), for Settings › Server in the apps.

use std::time::{Duration, Instant};

/// A server found on the network.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Found {
    pub name: String,
    /// Its address for the apps (plain http: desktop web views don't trust
    /// the server's own certificate authority unless the computer does).
    pub url: String,
    /// Its HTTPS address, when it offers one (for browsers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub https: Option<String>,
}

/// Listens for servers for `wait`, then lists them.
pub fn servers(wait: Duration) -> Vec<Found> {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else { return Vec::new() };
    let Ok(rx) = daemon.browse("_xenify._tcp.local.") else { return Vec::new() };
    let end = Instant::now() + wait;
    let mut out: Vec<Found> = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let Ok(event) = rx.recv_timeout(left) else { break };
        if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
            let name = info.get_fullname().trim_end_matches("._xenify._tcp.local.").to_string();
            // An IPv4 address reads best; IPv6 needs brackets.
            let mut addrs: Vec<String> = info.get_addresses().iter().map(|a| a.to_string()).collect();
            addrs.sort_by_key(|a| a.contains(':'));
            let Some(host) = addrs.first().map(|a| if a.contains(':') { format!("[{a}]") } else { a.clone() }) else {
                continue;
            };
            let url = format!("http://{host}:{}", info.get_port());
            let https = info
                .get_property_val_str("https")
                .and_then(|p| p.parse::<u16>().ok())
                .map(|p| format!("https://{host}:{p}"));
            if !out.iter().any(|f| f.name == name) {
                out.push(Found { name, url, https });
            }
        }
    }
    let _ = daemon.shutdown();
    out
}
