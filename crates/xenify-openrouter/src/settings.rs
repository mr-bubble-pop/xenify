use serde::{Deserialize, Serialize};

/// A suggested model shown in the settings picker (any OpenRouter id works).
#[derive(Clone, Copy, Debug)]
pub struct ModelPreset {
    pub id: &'static str,
    pub note: &'static str,
}

/// Models for the "brain": must support tool calling. Cheapest first.
/// ZDR = has a zero-data-retention provider (works with OpenRouter's ZDR guardrail).
pub const BRAIN_PRESETS: &[ModelPreset] = &[
    ModelPreset { id: "deepseek/deepseek-v4-flash", note: "default · very cheap · ZDR · text only" },
    ModelPreset { id: "qwen/qwen3.8-27b:free", note: "free · rate-limited · ZDR · sees images" },
    ModelPreset { id: "deepseek/deepseek-v4.1-flash", note: "cheap · sees images" },
    ModelPreset { id: "qwen/qwen3.7-flash", note: "very cheap · sees images · not ZDR" },
    ModelPreset { id: "deepseek/deepseek-v4-pro", note: "stronger · pricier" },
];

/// Models that look at rendered frames: must accept image input.
pub const REVIEWER_PRESETS: &[ModelPreset] = &[
    ModelPreset { id: "qwen/qwen3-vl-30b-a3b-instruct", note: "default · cheap · ZDR · strong vision" },
    ModelPreset { id: "google/gemma-3-27b-it", note: "cheapest · ZDR" },
    ModelPreset { id: "qwen/qwen3.8-27b:free", note: "free · rate-limited · ZDR" },
    ModelPreset { id: "qwen/qwen3.7-flash", note: "very cheap · not ZDR" },
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// Model that plans and edits (needs tool calling).
    pub brain_model: String,
    /// Model that looks at rendered frames; empty disables `look_at_frame`.
    pub reviewer_model: String,
    /// Model that generates illustrations (image output), later traced to vectors.
    pub image_model: String,
    /// Model for Xenify Write (review, rewrite, continue, chat); empty uses
    /// [`AiSettings::brain_model`].
    pub writer_model: String,
    /// A key typed into the settings, not yet saved. Never written to the
    /// settings file: [`AiSettings::store_api_key`] puts it in the system keyring.
    /// (Older versions saved it here; [`AiSettings::load`] moves it.)
    #[serde(skip_serializing)]
    pub api_key: String,
    pub temperature: f32,
    /// Safety limit on model round-trips per user message.
    pub max_steps: usize,
    /// How many frames the reviewer may look at per request (each costs a round trip).
    pub max_reviews: usize,
    /// For briefs that create something new, offer 2-3 concepts to pick from first.
    pub propose_concepts: bool,
    pub base_url: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            brain_model: BRAIN_PRESETS[0].id.into(),
            reviewer_model: REVIEWER_PRESETS[0].id.into(),
            image_model: crate::images::DEFAULT_IMAGE_MODEL.into(),
            writer_model: String::new(),
            api_key: String::new(),
            temperature: 0.4,
            max_steps: 40,
            max_reviews: 2,
            propose_concepts: true,
            base_url: "https://openrouter.ai/api/v1".into(),
        }
    }
}

impl AiSettings {
    /// The model Xenify Write uses.
    pub fn writer_model(&self) -> &str {
        if self.writer_model.trim().is_empty() { &self.brain_model } else { &self.writer_model }
    }

    /// The API key to use: the `OPENROUTER_API_KEY` environment variable, then a
    /// `.env` file (developer setups), then the key the user saved.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Ok(k) = std::env::var("OPENROUTER_API_KEY")
            && !k.trim().is_empty()
        {
            return Some(k.trim().to_string());
        }
        if let Some(k) = dotenv_key() {
            return Some(k);
        }
        crate::secrets::load(KEY_NAME).map(|(k, _)| k)
    }

    /// Where the key comes from, for display (never the key itself).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn api_key_source(&self) -> &'static str {
        if std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
            "environment variable"
        } else if dotenv_key().is_some() {
            ".env file"
        } else {
            match crate::secrets::load(KEY_NAME) {
                Some((_, loc)) => loc.describe(),
                None => "not set",
            }
        }
    }

    /// Saves the user's key securely (system keyring when available) and
    /// returns where it went.
    pub fn store_api_key(key: &str) -> Result<crate::secrets::SecretLocation, String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("the key is empty".into());
        }
        crate::secrets::save(KEY_NAME, key)
    }

    /// Removes the saved key.
    pub fn forget_api_key() -> Result<(), String> {
        crate::secrets::delete(KEY_NAME)
    }

    /// Whether the user has saved a key (not counting environment / `.env`).
    pub fn has_stored_api_key() -> bool {
        crate::secrets::load(KEY_NAME).is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn path() -> Option<std::path::PathBuf> {
        // Windows has no HOME; per-user app data lives under %APPDATA%.
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").filter(|_| cfg!(windows)).map(std::path::PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(base.join("xenify").join("ai.json"))
    }

    /// Loads saved settings (defaults if none). A key left in the settings file
    /// by an older version is moved to the keyring and removed from the file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load() -> Self {
        let mut s: Self = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let old = std::mem::take(&mut s.api_key);
        if !old.trim().is_empty() && Self::store_api_key(&old).is_ok() {
            let _ = s.save();
        }
        s
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Self::path() else { return Ok(()) };
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, serde_json::to_string_pretty(self).unwrap())?;
        // The file may hold an API key: keep it private.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

/// Name of the OpenRouter key in the secret store.
const KEY_NAME: &str = "openrouter_api_key";

/// Looks for `OPENROUTER_API_KEY=` in `.env` in the working directory or its parents.
#[cfg(not(target_arch = "wasm32"))]
fn dotenv_key() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if let Ok(text) = std::fs::read_to_string(dir.join(".env")) {
            for line in text.lines() {
                let line = line.trim().trim_start_matches("export ");
                if let Some(v) = line.strip_prefix("OPENROUTER_API_KEY=") {
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_never_written_to_the_settings_file() {
        let s = AiSettings { api_key: "sk-or-v1-secret".into(), ..Default::default() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("sk-or") && !json.contains("api_key"), "{json}");
        // Old files with a key still load (so it can be moved to the keyring).
        let old: AiSettings = serde_json::from_str(r#"{"api_key": "sk-old"}"#).unwrap();
        assert_eq!(old.api_key, "sk-old");
    }
}
