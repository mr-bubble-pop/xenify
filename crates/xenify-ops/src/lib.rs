//! Undoable edits for Xenify Draw projects.
//!
//! Every change to a [`Project`] — from the UI, keyboard shortcuts, scripts or
//! the AI assistant — is a [`Command`]. Applying a command returns its exact
//! inverse, which the [`Editor`] keeps on the undo stack. Commands are serde
//! types so they can be logged, replayed and produced by tool-calling LLMs.

pub mod arrange;
pub mod assets;
pub mod blast;
pub mod camera;
pub mod collab;
mod command;
pub mod crop;
pub mod diagram;
pub mod draw;
mod editor;
pub mod effects;
pub mod group;
pub mod library;
pub mod liquid;
pub mod matte;
pub mod media;
pub mod modifiers;
pub mod order;
pub mod outline;
pub mod particles;
pub mod pathfinder;
pub mod physics;
pub mod precomp;
pub mod presets;
pub mod rig;
pub mod slides;
pub mod templates;
pub mod text_anim;
pub mod transform;

pub use command::{Command, apply};
pub use editor::Editor;

use xenify_doc::ProjectError;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum OpError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("{0}")]
    Invalid(String),
}

pub(crate) fn invalid(msg: impl Into<String>) -> OpError {
    OpError::Invalid(msg.into())
}
