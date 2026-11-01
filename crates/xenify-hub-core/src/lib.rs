//! Xenify Hub without its window. The Hub installs and updates every Xenify
//! app (Write, Draw, Calc, Chat, the server and its parts: PostgreSQL, LocalAI),
//! like a toolbox app: it works on its own, reading a signed list of
//! releases, and when it's signed in to a Xenify server the server's admin
//! decides (which apps, which channel or version, when).
//!
//! - [`manifest`]: the release list and its signature (ed25519).
//! - [`choose`]: channels, fixed versions and the server's policy.
//! - [`install`]: where apps go, installing, rolling back, removing.
//! - [`signin`]: the Hub's server sign-in, which the apps follow.

pub mod choose;
pub mod install;
pub mod manifest;
pub mod pg;
pub mod signin;

#[cfg(test)]
mod tests;

pub use choose::{Choice, Effective, Local, Policy, ProductPolicy, UpdateMode};
pub use install::{Installed, Store};
pub use manifest::{Channel, Download, Manifest, Product, Release, Signed};

/// The keys Xenify's release lists are signed with.
pub const TRUSTED: &[[u8; 32]] = &[
    // Xenify's release key (made 2026-10-27; the 2026-10-24 one was never
    // used for a release).
    [
        36, 117, 127, 248, 165, 205, 96, 128, 124, 152, 7, 3, 14, 143, 116, 111, 180, 104, 248, 116, 58, 251, 213, 68,
        254, 167, 3, 181, 232, 104, 47, 165,
    ],
];

/// Xenify's own signed release list (made by the Release workflow on xenify-releases).
pub const RELEASES: &str =
    "https://github.com/mr-bubble-pop/xenify-releases/releases/download/hub-releases/manifest.json";

/// This computer's system and processor, as release files name them.
pub fn platform() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    (os, arch)
}
