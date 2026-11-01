//! Equations for Xenify Write: LaTeX parsed into a maths tree ([`parse`]),
//! typeset with an OpenType MATH font after TeX's rules ([`typeset`]),
//! written back as LaTeX ([`to_latex`]) or Word's equation markup
//! ([`to_omml`]), and edited visually ([`edit::MathEdit`]). No platform
//! code, so it builds for the browser too.

pub mod edit;
mod latex;
mod layout;
mod omml;
mod parse;

pub use latex::to_latex;
pub use layout::{MathBox, MathFont, Piece, typeset, typeset_edit};
pub use omml::to_omml;
pub use parse::{Class, Env, Node, Variant, parse};
