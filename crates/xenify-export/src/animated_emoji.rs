//! Animated emoji: Google's Noto Emoji Animation (CC BY 4.0), downloaded on
//! demand — the list once, each animation when it's first used — and kept
//! in the data folder. Each is a Lottie file, imported as ordinary layers
//! (see [`crate::lottie_import`]).

use serde_json::Value as Json;

/// The list of animated emoji.
pub const INDEX_URL: &str = "https://googlefonts.github.io/noto-emoji-animation/data/api.json";
/// Shown wherever they're offered (the licence needs it).
pub const CREDIT: &str = "Animated emoji: Noto Emoji Animation © Google LLC, CC BY 4.0";
/// Largest download accepted (the biggest animations are a few hundred KB).
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedEmoji {
    /// Hex code points joined by `_` (`1f389`, `2764_fe0f`): names the file.
    pub codepoint: String,
    /// `party popper`
    pub name: String,
    pub category: String,
    pub glyph: String,
    pub tags: Vec<String>,
    pub popularity: i64,
}

impl AnimatedEmoji {
    pub fn lottie_url(&self) -> String {
        format!("https://fonts.gstatic.com/s/e/notoemoji/latest/{}/lottie.json", self.codepoint)
    }

    /// Where its download is kept (under the data folder's downloads).
    pub fn cache_name(&self) -> String {
        format!("noto-emoji-animation/{}.json", self.codepoint)
    }
}

/// Reads the list (`api.json`), most popular first.
pub fn parse_index(data: &[u8]) -> Result<Vec<AnimatedEmoji>, String> {
    let json: Json = serde_json::from_slice(data).map_err(|e| format!("the animated emoji list is unreadable: {e}"))?;
    let icons = json["icons"].as_array().ok_or("the animated emoji list is unreadable (no icons)")?;
    let mut out: Vec<AnimatedEmoji> = icons
        .iter()
        .filter_map(|i| {
            let codepoint = i["codepoint"].as_str()?.to_ascii_lowercase();
            if !codepoint.chars().all(|c| c.is_ascii_hexdigit() || c == '_') {
                return None;
            }
            let glyph: String = codepoint
                .split('_')
                .filter_map(|h| u32::from_str_radix(h, 16).ok())
                .filter_map(char::from_u32)
                .collect();
            let tags: Vec<String> = i["tags"]
                .as_array()?
                .iter()
                .filter_map(Json::as_str)
                .map(|t| t.trim_matches(':').replace('-', " "))
                .collect();
            Some(AnimatedEmoji {
                name: tags.first().cloned().unwrap_or_else(|| codepoint.clone()),
                category: i["categories"].get(0).and_then(Json::as_str).unwrap_or("Other").to_string(),
                popularity: i["popularity"].as_i64().unwrap_or(0),
                codepoint,
                glyph,
                tags,
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.popularity));
    Ok(out)
}

/// Finds one by glyph (variation selectors ignored), name or tag.
pub fn find<'a>(list: &'a [AnimatedEmoji], key: &str) -> Option<&'a AnimatedEmoji> {
    let bare = |s: &str| s.chars().filter(|c| *c != '\u{fe0f}').collect::<String>();
    let (k, lower) = (bare(key.trim()), key.trim().to_lowercase());
    list.iter()
        .find(|e| bare(&e.glyph) == k)
        .or_else(|| list.iter().find(|e| e.name == lower || e.tags.contains(&lower)))
        .or_else(|| list.iter().find(|e| e.tags.iter().any(|t| t.contains(&lower))))
}

/// Those matching every word of `query` (in name, tags or category).
pub fn search<'a>(list: &'a [AnimatedEmoji], query: &str) -> Vec<&'a AnimatedEmoji> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    list.iter()
        .filter(|e| {
            let hay = format!("{} {} {}", e.tags.join(" "), e.category, e.name).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// The list: from the cache, else downloaded. Blocks.
pub fn index() -> Result<Vec<AnimatedEmoji>, String> {
    parse_index(&xenify_platform::download::fetch(INDEX_URL, "noto-emoji-animation/api.json", MAX_BYTES)?)
}

/// `e`'s Lottie file: from the cache, else downloaded. Blocks.
pub fn lottie(e: &AnimatedEmoji) -> Result<Vec<u8>, String> {
    xenify_platform::download::fetch(&e.lottie_url(), &e.cache_name(), MAX_BYTES)
}

/// Whether `e` was downloaded before (inserting it won't need the network).
pub fn is_cached(e: &AnimatedEmoji) -> bool {
    xenify_platform::download::cache_path(&e.cache_name()).is_some_and(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"{"icons":[
        {"name":"emoji_u1f600","popularity":881,"codepoint":"1f600","categories":["Smileys and emotions"],"tags":[":smile:"]},
        {"name":"emoji_u2764_fe0f","popularity":900,"codepoint":"2764_fe0f","categories":["Hearts"],"tags":[":red-heart:",":love:"]},
        {"name":"bad","codepoint":"../x","tags":[":x:"]}
    ]}"#;

    #[test]
    fn reads_the_list_and_finds_by_glyph_or_name() {
        let list = parse_index(INDEX.as_bytes()).unwrap();
        assert_eq!(list.len(), 2, "odd code points are dropped");
        assert_eq!(list[0].name, "red heart", "most popular first");
        assert_eq!(list[0].glyph, "\u{2764}\u{fe0f}");
        assert_eq!(find(&list, "❤").unwrap().codepoint, "2764_fe0f", "without the variation selector");
        assert_eq!(find(&list, "smile").unwrap().glyph, "😀");
        assert_eq!(search(&list, "love").len(), 1);
        assert_eq!(list[1].lottie_url(), "https://fonts.gstatic.com/s/e/notoemoji/latest/1f600/lottie.json");
    }
}
