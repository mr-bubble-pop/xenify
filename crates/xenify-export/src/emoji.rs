//! The bundled emoji (Microsoft Fluent Emoji, flat style, MIT; see
//! assets/SOURCES.toml and scripts/fetch-emoji.sh): searching them, and
//! turning one into editable vector layers.

use xenify_doc::Layer;
use kurbo::{Affine, Vec2};
use serde::Deserialize;
use std::sync::OnceLock;

const PACKED: &[u8] = include_bytes!("../../../assets/emoji/fluent-emoji.json.z");

/// The credit the emoji set's licence asks for.
pub const CREDIT: &str = "Fluent Emoji © Microsoft Corporation (MIT License)";

#[derive(Clone, Debug, Deserialize)]
pub struct Emoji {
    pub name: String,
    pub glyph: String,
    pub group: String,
    pub keywords: Vec<String>,
    pub svg: String,
}

/// Every bundled emoji (unpacked the first time).
pub fn library() -> &'static [Emoji] {
    static LIB: OnceLock<Vec<Emoji>> = OnceLock::new();
    LIB.get_or_init(|| {
        miniz_oxide::inflate::decompress_to_vec_zlib(PACKED)
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default()
    })
}

/// The groups, in library order.
pub fn groups() -> Vec<&'static str> {
    let mut out: Vec<&str> = Vec::new();
    for e in library() {
        if !out.contains(&e.group.as_str()) {
            out.push(&e.group);
        }
    }
    out
}

/// The emoji with this glyph (🎉) or name ("party popper").
pub fn find(key: &str) -> Option<&'static Emoji> {
    let k = key.trim();
    let lower = k.to_lowercase();
    library().iter().find(|e| {
        e.glyph == k
            || e.glyph.trim_end_matches('\u{fe0f}') == k.trim_end_matches('\u{fe0f}')
            || e.name.to_lowercase() == lower
    })
}

/// Emoji whose name or keywords contain every word of `query` (or whose glyph it is).
pub fn search(query: &str) -> Vec<&'static Emoji> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return library().iter().collect();
    }
    library()
        .iter()
        .filter(|e| {
            e.glyph == query.trim()
                || words.iter().all(|w| {
                    e.name.to_lowercase().contains(w) || e.keywords.iter().any(|k| k.to_lowercase().contains(w))
                })
        })
        .collect()
}

/// `emoji` as vector layers (ids from `first_id`), `size` pixels across,
/// centred on `centre` (composition space).
pub fn layers(emoji: &Emoji, first_id: u64, centre: Vec2, size: f64) -> Result<Vec<Layer>, String> {
    let (w, h) = crate::svg::svg_size(emoji.svg.as_bytes()).unwrap_or((32, 32));
    let k = size / (w.max(h).max(1) as f64);
    let place =
        Affine::translate(centre) * Affine::scale(k) * Affine::translate((-(w as f64) / 2.0, -(h as f64) / 2.0));
    let opts = crate::svg::SvgImportOptions {
        keep_groups: false,
        keep_images: false,
        place: Some(place),
        ..Default::default()
    };
    Ok(crate::svg::import_svg_document(emoji.svg.as_bytes(), first_id, 1, 1, opts)?.layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_unpacks_and_searches() {
        assert!(library().len() > 500, "{}", library().len());
        let party = find("🎉").expect("party popper");
        assert_eq!(party.name, "party popper");
        assert!(search("party").iter().any(|e| e.glyph == "🎉"));
        assert!(search("rocket").iter().any(|e| e.glyph == "🚀"));
        assert!(find("sparkles").is_some());
        assert!(groups().contains(&"Smileys & Emotion"));
    }

    #[test]
    fn an_emoji_becomes_vector_shapes_where_asked() {
        let e = find("😀").or_else(|| find("grinning face")).unwrap();
        let layers = layers(e, 100, Vec2::new(500.0, 400.0), 200.0).unwrap();
        assert!(!layers.is_empty());
        assert!(layers.iter().all(|l| matches!(l.kind, xenify_doc::LayerKind::Shape { .. })));
        let mut c = xenify_doc::Project::new().compositions.remove(0);
        c.layers = layers;
        let scene = xenify_anim::evaluate(&c, 0.0);
        let b = scene
            .layers
            .iter()
            .filter_map(|l| Some(l.transform.transform_rect_bbox(l.bounds(&mut xenify_anim::ApproxText)?)))
            .reduce(|a, b| a.union(b))
            .unwrap();
        assert!((b.center().x - 500.0).abs() < 20.0 && (b.center().y - 400.0).abs() < 20.0, "{b:?}");
        assert!(b.width() > 150.0 && b.width() <= 201.0, "{b:?}");
    }
}
