//! The release list: every product, its releases by channel and a download
//! per system. It's published with each release and signed with Xenify's
//! release key; the Hub refuses a list or a file that doesn't match.
//!
//! A list may also say, per product, the oldest version that's still safe
//! (`min_safe`: older releases are left out of the list, whatever a
//! server's policy asks for). A list never runs out: a list older than one
//! already seen is refused, but an unchanged list stays good for ever.
//!
//! Lists from newer releases must stay readable by older Hubs, or an old
//! Hub could never update itself: a product or release this Hub can't read
//! (a new channel, a field that changed shape) is left out rather than
//! failing the whole list, and a list that truly needs a newer Hub says so
//! (`min_hub`), so the Hub can say "update the Hub" instead of "unreadable".

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How settled a release is. Each channel also gets the steadier ones: on
/// Preview you get a Release when it's newer than the last Preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Release,
    Preview,
    Nightly,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Release => "Release",
            Channel::Preview => "Preview",
            Channel::Nightly => "Nightly",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// When this list was made (RFC 3339), to refuse an older list served
    /// again (a mirror that's behind, or someone holding updates back).
    pub published: String,
    #[serde(deserialize_with = "readable")]
    pub products: Vec<Product>,
}

/// The items this version can read; the rest (from newer releases) are left out.
fn readable<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Vec::<serde_json::Value>::deserialize(d)?.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Product {
    /// "write", "draw", "calc", "chat", "server", "postgres", "localai".
    pub id: String,
    pub name: String,
    /// One line about it, for the Hub's list.
    #[serde(default)]
    pub about: String,
    /// A part other products need (PostgreSQL for the server), not an app.
    #[serde(default)]
    pub component: bool,
    /// Parts installed with it (the server: "postgres").
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(deserialize_with = "readable")]
    pub releases: Vec<Release>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub channel: Channel,
    pub date: String,
    /// What changed, in a few lines.
    #[serde(default)]
    pub notes: String,
    pub downloads: Vec<Download>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Download {
    /// "linux", "windows", "macos".
    pub os: String,
    /// "x86_64", "aarch64".
    pub arch: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// The program to start, inside the unpacked download.
    pub run: String,
}

impl Release {
    pub fn semver(&self) -> Option<semver::Version> {
        semver::Version::parse(&self.version).ok()
    }

    pub fn download(&self, os: &str, arch: &str) -> Option<&Download> {
        self.downloads.iter().find(|d| d.os == os && d.arch == arch)
    }
}

impl Manifest {
    pub fn product(&self, id: &str) -> Option<&Product> {
        self.products.iter().find(|p| p.id == id)
    }
}

/// A release list as published: the list's exact bytes and their signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signed {
    /// The list, as JSON text (signed as these exact bytes).
    pub manifest: String,
    /// Which release key signed it (the key's first 8 bytes, hex).
    pub key: String,
    /// ed25519, base64.
    pub signature: String,
}

/// Why a list or a file was refused.
#[derive(Debug, PartialEq)]
pub enum Refused {
    UnknownKey,
    BadSignature,
    Unreadable(String),
    /// Made for a newer Hub (the list's `min_hub`).
    NeedsNewerHub(String),
    /// Older than the list already seen.
    Older,
    /// A download that isn't what the list says.
    WrongFile,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refused::UnknownKey => "the release list is signed with a key this Hub doesn't know",
            Refused::BadSignature => "the release list's signature doesn't match: it was changed after signing",
            Refused::Unreadable(_) => "the release list can't be read",
            Refused::NeedsNewerHub(_) => {
                "the release list needs a newer Xenify Hub: download the latest Hub from xenify.net to keep getting updates"
            }
            Refused::Older => "the release list is older than the one already seen",
            Refused::WrongFile => "the download doesn't match the release list (it was changed or cut short)",
        })
    }
}

fn key_id(key: &VerifyingKey) -> String {
    key.as_bytes()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Signs a release list (the release tool, with the release key).
pub fn sign(manifest: &Manifest, key: &SigningKey) -> Signed {
    sign_with(manifest, &Limits::default(), key)
}

/// What a list says beyond its products: the oldest safe version of each
/// product, and the oldest Hub that can read it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limits {
    /// Product id → its oldest safe version.
    pub min_safe: std::collections::BTreeMap<String, String>,
    /// The oldest Hub that can use this list (set only when a list changes
    /// in a way older Hubs would misread).
    pub min_hub: Option<String>,
}

/// Signs a list with its limits (written into the signed text).
pub fn sign_with(manifest: &Manifest, limits: &Limits, key: &SigningKey) -> Signed {
    #[derive(Serialize)]
    struct Out<'a> {
        published: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_hub: Option<&'a str>,
        products: Vec<OutProduct<'a>>,
    }
    #[derive(Serialize)]
    struct OutProduct<'a> {
        #[serde(flatten)]
        product: &'a Product,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_safe: Option<&'a str>,
    }
    let out = Out {
        published: &manifest.published,
        min_hub: limits.min_hub.as_deref(),
        products: manifest
            .products
            .iter()
            .map(|p| OutProduct { product: p, min_safe: limits.min_safe.get(&p.id).map(String::as_str) })
            .collect(),
    };
    let text = serde_json::to_string_pretty(&out).expect("a manifest serialises");
    let sig = key.sign(text.as_bytes());
    Signed {
        manifest: text,
        key: key_id(&key.verifying_key()),
        signature: base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
    }
}

/// The limits written in a list's (signed) text.
pub fn limits(manifest_text: &str) -> Limits {
    #[derive(Deserialize, Default)]
    struct In {
        #[serde(default)]
        min_hub: Option<String>,
        #[serde(default)]
        products: Vec<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct InProduct {
        id: String,
        #[serde(default)]
        min_safe: Option<String>,
    }
    let v: In = serde_json::from_str(manifest_text).unwrap_or_default();
    Limits {
        min_hub: v.min_hub,
        min_safe: v
            .products
            .into_iter()
            .filter_map(|p| serde_json::from_value::<InProduct>(p).ok())
            .filter_map(|p| Some((p.id, p.min_safe?)))
            .collect(),
    }
}

/// Whether `version` is at least `floor`. A floor that isn't a version
/// keeps nothing out; a version that isn't one can't be shown to be safe.
fn safe_version(floor: &str, version: &str) -> bool {
    match (semver::Version::parse(floor), semver::Version::parse(version)) {
        (Ok(floor), Ok(v)) => v >= floor,
        (Err(_), _) => true,
        (Ok(_), Err(_)) => false,
    }
}

/// Whether `version` is at least `product`'s oldest safe version (true
/// when the list names none).
pub fn safe(limits: &Limits, product: &str, version: &str) -> bool {
    limits.min_safe.get(product).is_none_or(|floor| safe_version(floor, version))
}

/// Checks a list against the release keys this Hub trusts, and that it
/// isn't older than `seen` (the last list's `published`). Releases older
/// than their product's oldest safe version are left out of what it
/// returns.
pub fn verify(signed: &Signed, trusted: &[[u8; 32]], seen: Option<&str>) -> Result<Manifest, Refused> {
    let key = trusted
        .iter()
        .filter_map(|k| VerifyingKey::from_bytes(k).ok())
        .find(|k| key_id(k) == signed.key)
        .ok_or(Refused::UnknownKey)?;
    let sig = base64::engine::general_purpose::STANDARD
        .decode(&signed.signature)
        .ok()
        .and_then(|b| Signature::from_slice(&b).ok())
        .ok_or(Refused::BadSignature)?;
    key.verify(signed.manifest.as_bytes(), &sig).map_err(|_| Refused::BadSignature)?;
    let mut m: Manifest = serde_json::from_str(&signed.manifest).map_err(|e| Refused::Unreadable(e.to_string()))?;
    // RFC 3339 times in UTC compare as text.
    if seen.is_some_and(|s| m.published.as_str() < s) {
        return Err(Refused::Older);
    }
    let limits = limits(&signed.manifest);
    if let Some(min) = &limits.min_hub
        && !safe_version(min, env!("CARGO_PKG_VERSION"))
    {
        return Err(Refused::NeedsNewerHub(min.clone()));
    }
    for p in &mut m.products {
        p.releases.retain(|r| safe(&limits, &p.id, &r.version));
    }
    Ok(m)
}

/// UTC now, RFC 3339 (lists compare by it).
pub fn now() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    rfc3339(s as i64)
}

/// Seconds since 1970 as RFC 3339 (UTC).
pub fn rfc3339(s: i64) -> String {
    let (d, t) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let (y, m, day) = civil(d);
    format!("{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

/// Days since 1970-01-01 to (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A new release key (its public half goes into the Hub).
pub fn new_key() -> SigningKey {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("the system gives random bytes");
    SigningKey::from_bytes(&seed)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks a downloaded file against the list.
pub fn check_file(d: &Download, bytes: &[u8]) -> Result<(), Refused> {
    if bytes.len() as u64 != d.size || sha256_hex(bytes) != d.sha256.to_ascii_lowercase() {
        return Err(Refused::WrongFile);
    }
    Ok(())
}

/// Checks a downloaded file on disk against the list (read in pieces).
pub fn check_path(d: &Download, path: &std::path::Path) -> Result<(), Refused> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|_| Refused::WrongFile)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        let k = f.read(&mut buf).map_err(|_| Refused::WrongFile)?;
        if k == 0 {
            break;
        }
        n += k as u64;
        h.update(&buf[..k]);
    }
    let sha: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if n != d.size || sha != d.sha256.to_ascii_lowercase() {
        return Err(Refused::WrongFile);
    }
    Ok(())
}
