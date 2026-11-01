//! The Xenify Write document model.
//!
//! A [`Document`] is a list of [`Section`]s (each with its own page setup)
//! holding [`Block`]s. Paragraph text is one `String` with formatting runs,
//! so byte offsets line up with the layout engine's. Every change goes
//! through an invertible [`Step`] inside a transaction, which is what undo,
//! redo and anchors (comments, spelling issues, AI suggestions) build on.
//!
//! Nothing here touches the file system or threads, so the crate builds for
//! wasm32.

pub mod autocorrect;
pub mod changelog;
pub mod cite;
pub mod clip;
pub mod collab;
mod color;
pub mod compare;
pub mod controls;
pub mod datetime;
mod edit;
pub mod frames;
pub mod hf;
pub mod linked;
pub mod lists;
pub mod merge;
mod model;
pub mod multi;
pub mod notes;
pub mod ops;
pub mod review;
mod style;
pub mod symbols;
mod table;
pub mod tables;

pub use color::Rgba;
pub use edit::{Checkpoint, Content, EditError, Editor, MergeKey, Step, Tx, restore};
pub use lists::{ListDef, ListItem, ListLevel, NumberKind};
pub use model::{
    BADGE_COLORS, Block, CharFmt, Document, Field, HeaderPart, HfLine, HfSlot, Margins, Meta, NodeId, Note, NoteKind,
    PageNumberFormat, PageSetup, PageVAlign, Paragraph, Pos, Run, Section, SectionStart, Selection, Span, VertAlign,
    badge_colors,
};
pub use style::{
    Align, DEFAULT_TAB, ParaBorders, ParaProps, ParaStyle, ResolvedChar, ResolvedPara, StyleSheet, TabKind, TabLeader,
    TabStop,
};
pub use table::{Border, Cell, Container, Frame, Image, Row, Table, Toc, TocKind, VAlign, Wrap};
