//! Equations in the text: their LaTeX typeset by `xenify-math` with the
//! bundled STIX Two Math font, as glyphs and rules the page and PDF draw.

use crate::fonts::symbol_font;
use crate::para::{Glyph, Item};
use parley::FontData;
use std::sync::{Arc, OnceLock};
use xenify_wp_doc::Rgba;

fn math_font() -> Option<&'static xenify_math::MathFont> {
    static FONT: OnceLock<Option<xenify_math::MathFont>> = OnceLock::new();
    // Not yet handed over (the browser downloads it after the first page).
    if symbol_font().is_empty() {
        return None;
    }
    FONT.get_or_init(|| xenify_math::MathFont::new(symbol_font())).as_ref()
}

fn font_data() -> FontData {
    static DATA: OnceLock<FontData> = OnceLock::new();
    DATA.get_or_init(|| FontData::new(parley::fontique::Blob::new(Arc::new(symbol_font())), 0)).clone()
}

/// A typeset equation: what to draw (relative to its baseline, y down),
/// its width, and how far it reaches above and below the baseline.
pub struct Typeset {
    pub items: Vec<Item>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Typesets `latex` at `size` points in `color`; `display`: on a line of its own.
pub fn typeset(latex: &str, display: bool, size: f32, color: Rgba) -> Option<Typeset> {
    let font = math_font()?;
    let node = xenify_math::parse(latex);
    Some(from_box(&xenify_math::typeset(font, &node, size, display), color))
}

/// For the visual equation editor: the equation being edited, typeset with
/// its caret places and empty boxes (in the returned [`xenify_math::MathBox`]).
pub fn typeset_edit(
    ed: &xenify_math::edit::MathEdit,
    display: bool,
    size: f32,
    color: Rgba,
) -> Option<(Typeset, xenify_math::MathBox)> {
    let font = math_font()?;
    let b = ed.typeset(font, size, display);
    Some((from_box(&b, color), b))
}

fn from_box(b: &xenify_math::MathBox, color: Rgba) -> Typeset {
    let red = Rgba::rgb(0xc6, 0x28, 0x28);
    let data = font_data();
    let items = b
        .pieces
        .iter()
        .filter_map(|p| match *p {
            xenify_math::Piece::Glyph { id, x, y, size, error } => Some(Item::Glyphs {
                font: data.clone(),
                size,
                coords: Vec::new(),
                glyphs: vec![Glyph { id: id as u32, x, y }],
                color: if error { red } else { color },
                skew: None,
                text: 0..0,
                chars: vec![String::new()],
                line: 0,
            }),
            xenify_math::Piece::Rule { x, y, w, h } => Some(Item::Rect {
                rect: kurbo::Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64),
                color,
                line: 0,
            }),
            // Editing marks: drawn by the editor, not on the page.
            xenify_math::Piece::Mark { .. } | xenify_math::Piece::Slot { .. } => None,
        })
        .collect();
    Typeset { items, width: b.width, ascent: b.ascent, descent: b.descent }
}
