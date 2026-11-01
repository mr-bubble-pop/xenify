//! Which version each app should be at: the person's choice (a channel or
//! a fixed version, updates automatic or by a click), unless a Xenify
//! server manages this computer, whose admin then decides.

use crate::manifest::{Channel, Product, Release};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// The newest release on this channel (or a steadier one).
    Channel(Channel),
    /// Stays at this version.
    Fixed(String),
}

impl Default for Choice {
    fn default() -> Self {
        Choice::Channel(Channel::Release)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Downloaded in the background, used from the app's next start.
    #[default]
    Automatic,
    /// Offered; installed when the person clicks Update.
    Manual,
}

/// The person's own settings for one product.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Local {
    #[serde(default, deserialize_with = "or_default")]
    pub choice: Choice,
    #[serde(default, deserialize_with = "or_default")]
    pub mode: UpdateMode,
}

/// What a server's admin set for one product (Admin › Apps & updates).
/// Anything left out stays the person's choice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductPolicy {
    /// May be installed at all.
    #[serde(default = "yes")]
    pub allowed: bool,
    /// Installed on every computer the server manages.
    #[serde(default)]
    pub required: bool,
    #[serde(default, deserialize_with = "or_default")]
    pub choice: Option<Choice>,
    #[serde(default, deserialize_with = "or_default")]
    pub mode: Option<UpdateMode>,
    /// Updates only between these hours (local time, `[from, to)`; may wrap
    /// past midnight), so nobody's work is interrupted.
    #[serde(default)]
    pub window: Option<(u8, u8)>,
}

/// A value this Hub can't read (a channel or mode from a newer release)
/// reads as the default: for a policy, "the person's choice"; for the
/// person's own settings, the Release channel and automatic updates. One
/// such value mustn't make the whole policy or saved settings unreadable.
fn or_default<'de, D: serde::Deserializer<'de>, T: Default + serde::de::DeserializeOwned>(d: D) -> Result<T, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).unwrap_or_default())
}

fn yes() -> bool {
    true
}

/// A server's policy for the computers it manages.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// The server's name, shown as "Managed by …".
    pub server: String,
    #[serde(default)]
    pub products: BTreeMap<String, ProductPolicy>,
}

/// What applies to one product, and who decided.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Effective {
    pub choice: Choice,
    pub mode: UpdateMode,
    pub window: Option<(u8, u8)>,
    pub allowed: bool,
    pub required: bool,
    /// The choice (channel or version) is the server's: the Hub shows it
    /// but can't change it.
    pub locked: bool,
    /// So is when it updates.
    pub mode_locked: bool,
    pub managed_by: Option<String>,
}

pub fn effective(local: &Local, policy: Option<&Policy>, product: &str) -> Effective {
    let p = policy.and_then(|p| p.products.get(product));
    Effective {
        choice: p.and_then(|p| p.choice.clone()).unwrap_or_else(|| local.choice.clone()),
        mode: p.and_then(|p| p.mode).unwrap_or(local.mode),
        window: p.and_then(|p| p.window),
        allowed: p.is_none_or(|p| p.allowed),
        required: p.is_some_and(|p| p.required),
        locked: p.is_some_and(|p| p.choice.is_some()),
        mode_locked: p.is_some_and(|p| p.mode.is_some() || matches!(p.choice, Some(Choice::Fixed(_)))),
        managed_by: policy.map(|p| p.server.clone()),
    }
}

/// The release a choice means on this system, if there is one.
pub fn pick<'a>(product: &'a Product, choice: &Choice, os: &str, arch: &str) -> Option<&'a Release> {
    let here = product.releases.iter().filter(|r| r.download(os, arch).is_some());
    match choice {
        Choice::Fixed(v) => here.into_iter().find(|r| &r.version == v),
        Choice::Channel(c) => {
            here.filter(|r| r.channel <= *c).filter(|r| r.semver().is_some()).max_by_key(|r| r.semver())
        }
    }
}

/// The release to move to from `installed` (None: stay). Moving to an
/// older one happens only when the choice says so (a fixed version, or a
/// steadier channel picked after a newer one).
pub fn update<'a>(
    product: &'a Product,
    installed: Option<&str>,
    e: &Effective,
    os: &str,
    arch: &str,
) -> Option<&'a Release> {
    if !e.allowed {
        return None;
    }
    let target = pick(product, &e.choice, os, arch)?;
    (installed != Some(target.version.as_str())).then_some(target)
}

/// Whether updating is allowed at `hour` (0–23).
pub fn in_window(window: Option<(u8, u8)>, hour: u8) -> bool {
    match window {
        None => true,
        Some((from, to)) if from <= to => (from..to).contains(&hour),
        Some((from, to)) => hour >= from || hour < to,
    }
}
