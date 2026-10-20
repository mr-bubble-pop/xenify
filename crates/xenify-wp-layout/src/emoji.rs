//! Emoji pictures: the bundled Microsoft Fluent Emoji (flat style, MIT; see
//! assets/SOURCES.toml), so emoji look the same on every system, in the
//! browser and in PDFs, whatever fonts are installed. The text keeps the real
//! Unicode emoji; only the drawing uses these pictures.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};


/// The credit the emoji set's licence asks for.
pub const CREDIT: &str = "Fluent Emoji © Microsoft Corporation (MIT License)";

#[derive(Clone, Debug, Deserialize)]
pub struct Emoji {
    pub name: String,
    pub glyph: String,
    pub group: String,
    pub keywords: Vec<String>,
    svg: String,
}

/// The usual order of emoji groups (as on phone keyboards).
const GROUPS: [&str; 8] = [
    "Smileys & Emotion",
    "People & Body",
    "Animals & Nature",
    "Food & Drink",
    "Travel & Places",
    "Activities",
    "Objects",
    "Symbols",
];

/// Every bundled emoji, grouped in the usual order (unpacked the first time).
pub fn library() -> &'static [Emoji] {
    static LIB: OnceLock<Vec<Emoji>> = OnceLock::new();
    let packed = crate::fonts::emoji_packed();
    // Not yet handed over (the browser downloads the set after the first page).
    if packed.is_empty() {
        return &[];
    }
    LIB.get_or_init(|| {
        let mut all: Vec<Emoji> = miniz_oxide::inflate::decompress_to_vec_zlib(packed)
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default();
        // By group, then roughly keyboard order.
        all.sort_by_key(|e| {
            let group = GROUPS.iter().position(|g| *g == e.group).unwrap_or(GROUPS.len());
            let cp = e.glyph.chars().next().map_or(0, |c| c as u32);
            // Faces first (😀–🙏, then 🤐–🧿), then the rest of the group.
            let block = match cp {
                0x1F600..=0x1F64F => 0,
                0x1F900..=0x1F9FF => 1,
                _ => 2,
            };
            (group, block, cp)
        });
        all
    })
}

impl Emoji {
    /// The picture, as SVG.
    pub fn svg(&self) -> &str {
        &self.svg
    }
}

/// Without the emoji-style marker (U+FE0F), which the set leaves out.
fn key(s: &str) -> String {
    s.chars().filter(|&c| c != '\u{FE0F}').collect()
}

fn by_glyph() -> &'static HashMap<String, usize> {
    static MAP: OnceLock<HashMap<String, usize>> = OnceLock::new();
    static NONE: OnceLock<HashMap<String, usize>> = OnceLock::new();
    if library().is_empty() {
        return NONE.get_or_init(HashMap::new);
    }
    MAP.get_or_init(|| library().iter().enumerate().map(|(i, e)| (key(&e.glyph), i)).collect())
}

/// The emoji `grapheme` is (one user-perceived character), if it's one we
/// draw. Symbols that are text by default (©, ™, ↔, ♠) stay text unless
/// written with the emoji marker U+FE0F.
pub fn find(grapheme: &str) -> Option<&'static Emoji> {
    let mut chars = grapheme.chars();
    let first = chars.next()?;
    let pictographic = first as u32 >= 0x1F000
        || grapheme.contains(['\u{FE0F}', '\u{200D}'])
        || EMOJI_PRESENTATION.contains(first) && grapheme.chars().count() == 1;
    if !pictographic {
        return None;
    }
    by_glyph().get(&key(grapheme)).map(|&i| &library()[i])
}

/// Characters below U+1F000 that show as emoji by default.
const EMOJI_PRESENTATION: &str = "⌚⌛⏩⏪⏫⏬⏰⏳◽◾☔☕♈♉♊♋♌♍♎♏♐♑♒♓♿⚓⚡⚪⚫⚽⚾⛄⛅⛎⛔⛪⛲⛳⛵⛺⛽✅✊✋✨❌❎❓❔❕❗➕➖➗➰➿⬛⬜⭐⭕";

/// The byte ranges of the emoji we draw in `text`.
pub fn ranges(text: &str) -> Vec<(std::ops::Range<usize>, &'static Emoji)> {
    use unicode_segmentation::UnicodeSegmentation;
    if text.is_ascii() {
        return Vec::new();
    }
    text.grapheme_indices(true).filter_map(|(i, g)| find(g).map(|e| (i..i + g.len(), e))).collect()
}

/// Emoji matching `query` by name or keyword (all of them when it's empty).
pub fn search(query: &str) -> Vec<&'static Emoji> {
    let q = query.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    library()
        .iter()
        .filter(|e| words.iter().all(|w| e.name.to_lowercase().contains(w) || e.keywords.iter().any(|k| k.contains(w))))
        .collect()
}

/// The emoji drawn into a `px` × `px` square: premultiplied RGBA, shared
/// between calls (cached by glyph and size).
pub fn picture(emoji: &Emoji, px: u32) -> Option<Arc<Vec<u8>>> {
    type Pictures = HashMap<(String, u32), Option<Arc<Vec<u8>>>>;
    static CACHE: OnceLock<Mutex<Pictures>> = OnceLock::new();
    let px = px.clamp(8, 512);
    let cache = CACHE.get_or_init(Default::default);
    let k = (emoji.glyph.clone(), px);
    if let Some(hit) = cache.lock().ok()?.get(&k) {
        return hit.clone();
    }
    let drawn = draw(&emoji.svg, px).map(Arc::new);
    let mut c = cache.lock().ok()?;
    if c.len() > 4000 {
        c.clear();
    }
    c.insert(k, drawn.clone());
    drawn
}

fn draw(svg: &str, px: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(pixmap.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_emoji_but_leaves_text_symbols() {
        assert!(library().len() > 500);
        assert_eq!(find("😀").map(|e| e.name.as_str()), Some("grinning face"));
        assert!(find("❤\u{FE0F}").is_some());
        assert!(find("☕").is_some());
        assert!(find("©").is_none());
        assert!(find("a").is_none());
        let t = "Hi 👋 there ☕!";
        let r: Vec<&str> = ranges(t).into_iter().map(|(r, _)| &t[r]).collect();
        assert_eq!(r, ["👋", "☕"]);
    }

    #[test]
    fn searches_and_draws() {
        let cats = search("cat face");
        assert!(cats.iter().any(|e| e.glyph == "🐱"));
        let px = picture(find("😀").unwrap(), 32).unwrap();
        assert_eq!(px.len(), 32 * 32 * 4);
        assert!(px.chunks(4).any(|p| p[3] > 0), "draws something");
    }
}
