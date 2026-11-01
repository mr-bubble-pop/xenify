//! Asset provenance: `assets/SOURCES.toml`, the one record of where every
//! bundled, downloaded or generated asset came from and its licence. The
//! app's credits window, `scripts/licenses.sh` and the provenance test all
//! read it.

use serde::Deserialize;

/// The record as compiled into this build.
pub const SOURCES_TOML: &str = include_str!("../../../assets/SOURCES.toml");

/// Licences an asset may use. CC-BY-4.0 needs its attribution shown on
/// screen wherever the asset appears, as well as in the credits.
pub const ACCEPTED: &[&str] =
    &["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "CC0-1.0", "OFL-1.1", "CC-BY-4.0"];

include!(concat!(env!("OUT_DIR"), "/licence_texts.rs"));

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: Option<String>,
    pub name: String,
    #[serde(default)]
    pub source: Origin,
    pub source_url: Option<String>,
    pub generator: Option<String>,
    /// For generated assets: where the model that made it comes from.
    pub model_source_url: Option<String>,
    pub version: Option<String>,
    pub license: String,
    pub license_file: Option<String>,
    pub attribution: Option<String>,
    pub fetched: Option<String>,
    pub sha256: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    #[default]
    Download,
    Generated,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    asset: Vec<Source>,
}

pub fn parse(text: &str) -> Result<Vec<Source>, String> {
    toml::from_str::<File>(text).map(|f| f.asset).map_err(|e| e.to_string())
}

/// The entries compiled into this build.
pub fn bundled() -> Vec<Source> {
    parse(SOURCES_TOML).expect("assets/SOURCES.toml is checked by tests/provenance.rs")
}

/// The text of a licence file named by an entry's `license_file`.
pub fn licence_text(path: &str) -> Option<&'static str> {
    LICENCE_TEXTS.iter().find(|(p, _)| *p == path).map(|(_, t)| *t)
}
