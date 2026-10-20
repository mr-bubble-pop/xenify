//! The Xenify Vector AI assistant.
//!
//! An [`Agent`] talks to any OpenRouter chat model with tool calling. The
//! tools translate into [`xenify_ops::Command`]s, so every AI change is a
//! normal, undoable edit. The agent is a pure state machine: it never does
//! network I/O itself. It returns [`Next::Request`] bodies that the caller
//! sends (e.g. on a background thread) and feeds back with
//! [`Agent::handle_response`]. That keeps it testable and UI-thread friendly.

mod agent;
mod diagram_tools;
mod prompt;
pub mod requests;
mod tools;
pub mod values;

pub use agent::{Agent, ChatItem, Next, Purpose, ToolHost, Usage};
pub use xenify_openrouter::{AiError, AiSettings, BRAIN_PRESETS, ModelPreset, REVIEWER_PRESETS, images};
#[cfg(not(target_arch = "wasm32"))]
pub use xenify_openrouter::{ModelInfo, OpenRouter, fetch_models};
pub use tools::scene_summary;
