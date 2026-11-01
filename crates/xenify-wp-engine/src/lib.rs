//! Xenify Write without a toolkit.
//!
//! A [`Session`] is one open document: its editor, its layout, and what
//! keys, typing and the mouse do to it. The screens (HTML, on the desktop
//! or in a browser) send it input and draw what it returns:
//!
//! - [`View`]: where the pages, the caret and the selection are, and what
//!   the toolbar should show;
//! - [`PageScene`]: everything drawn on one page, as a display list;
//! - glyph outlines from [`FontTable`], drawn once and cached by the screen.
//!
//! Units are points (1/72 inch) in document space: pages stacked top to
//! bottom, the first page's top-left corner at (0, 0).

pub mod easymath;
mod glyphs;
mod scene;
mod session;
pub mod templates;

pub use glyphs::FontTable;
pub use scene::{Op, PageInfo, PageScene};
pub use session::{Mods, Session, Toggle, ToolState, View};
