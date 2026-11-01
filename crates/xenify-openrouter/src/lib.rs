//! OpenRouter plumbing shared by Xenify and Xenify Write: the HTTP client
//! (plain and streamed chat completions, model catalogue), the AI settings
//! file, API-key storage and image-generation helpers. Nothing here knows
//! about either app's document model.

pub mod images;
pub mod secrets;
mod settings;
#[cfg(not(target_arch = "wasm32"))]
mod transport;

pub use settings::{AiSettings, BRAIN_PRESETS, ModelPreset, REVIEWER_PRESETS};
#[cfg(not(target_arch = "wasm32"))]
pub use transport::{ModelInfo, OpenRouter, fetch_models};

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("no OpenRouter API key — add your own in AI settings (get one at openrouter.ai/keys)")]
    NoApiKey,
    #[error("network error: {0}")]
    Network(String),
    #[error("OpenRouter error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("unexpected response: {0}")]
    BadResponse(String),
}
