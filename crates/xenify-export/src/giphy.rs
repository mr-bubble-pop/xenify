//! GIPHY search: the user's own API key (kept in the system keyring), their
//! search or what's trending, and the GIF they pick, which is added as an
//! ordinary animated image layer. GIPHY's terms ask for "Powered by GIPHY"
//! wherever results are shown; the user is responsible for the rights to
//! what they place.

use serde_json::Value as Json;

/// Name of the keyring entry holding the user's GIPHY API key.
pub const KEY_NAME: &str = "giphy_api_key";
/// Shown with results (GIPHY's attribution requirement).
pub const ATTRIBUTION: &str = "Powered by GIPHY";
#[cfg(not(target_arch = "wasm32"))]
const MAX_JSON: u64 = 4 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_GIF: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    pub id: String,
    pub title: String,
    /// The creator's GIPHY name ("" when anonymous).
    pub username: String,
    /// Its page on giphy.com (credit link).
    pub page: String,
    /// A small preview (about 100 px wide).
    pub preview: String,
    /// The GIF to add (GIPHY's "downsized" version: under 2 MB).
    pub full: String,
    pub width: u32,
    pub height: u32,
}

impl Gif {
    /// A name for the layer and image: the title, and who made it.
    pub fn credit(&self) -> String {
        let title = if self.title.trim().is_empty() { "GIF" } else { self.title.trim() };
        if self.username.is_empty() {
            format!("{title} (GIPHY)")
        } else {
            format!("{title} by @{} (GIPHY)", self.username)
        }
    }
}

/// The API URL for `query` (trending when empty); `limit` results from `offset`.
pub fn url(key: &str, query: &str, limit: u32, offset: u32) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                b' ' => "+".into(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    let q = query.trim();
    let base = if q.is_empty() {
        "https://api.giphy.com/v1/gifs/trending?".to_string()
    } else {
        format!("https://api.giphy.com/v1/gifs/search?q={}&", enc(q))
    };
    format!("{base}api_key={}&limit={limit}&offset={offset}&rating=g&bundle=messaging_non_clips", enc(key))
}

/// Reads a search/trending response.
pub fn parse(data: &[u8]) -> Result<Vec<Gif>, String> {
    let json: Json = serde_json::from_slice(data).map_err(|e| format!("GIPHY's answer is unreadable: {e}"))?;
    if let Some(msg) = json["meta"]["msg"].as_str()
        && json["meta"]["status"].as_i64().is_some_and(|s| s >= 400)
    {
        return Err(format!("GIPHY says: {msg}"));
    }
    if let Some(msg) = json["message"].as_str() {
        return Err(format!("GIPHY says: {msg}"));
    }
    let list = json["data"].as_array().ok_or("GIPHY's answer is unreadable (no data)")?;
    Ok(list
        .iter()
        .filter_map(|g| {
            let im = &g["images"];
            let pick = |k: &str| im[k]["url"].as_str().filter(|u| u.starts_with("https://")).map(str::to_string);
            let full = pick("downsized").or_else(|| pick("original"))?;
            let size = |k: &str| im["downsized"][k].as_str().and_then(|v| v.parse().ok()).unwrap_or(0);
            Some(Gif {
                id: g["id"].as_str()?.to_string(),
                title: g["title"].as_str().unwrap_or("").to_string(),
                username: g["username"].as_str().unwrap_or("").to_string(),
                page: g["url"].as_str().unwrap_or("").to_string(),
                preview: pick("fixed_width_small").or_else(|| pick("preview_gif")).unwrap_or_else(|| full.clone()),
                full,
                width: size("width"),
                height: size("height"),
            })
        })
        .collect())
}

#[cfg(not(target_arch = "wasm32"))]
/// Searches (or lists what's trending). Blocks.
pub fn search(key: &str, query: &str, limit: u32, offset: u32) -> Result<Vec<Gif>, String> {
    parse(&xenify_platform::download::get(&url(key, query, limit, offset), MAX_JSON)?)
}

#[cfg(not(target_arch = "wasm32"))]
/// A preview's bytes. Blocks.
pub fn preview(g: &Gif) -> Result<Vec<u8>, String> {
    xenify_platform::download::get(&g.preview, MAX_GIF)
}

#[cfg(not(target_arch = "wasm32"))]
/// The GIF to add. Blocks.
pub fn download(g: &Gif) -> Result<Vec<u8>, String> {
    xenify_platform::download::get(&g.full, MAX_GIF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_results() {
        let u = url("k&y", "happy dance!", 24, 48);
        assert!(
            u.starts_with("https://api.giphy.com/v1/gifs/search?q=happy+dance%21&api_key=k%26y&limit=24&offset=48")
        );
        assert!(url("k", " ", 10, 0).starts_with("https://api.giphy.com/v1/gifs/trending?api_key=k"));
        let body = r#"{"data":[{"id":"abc","title":"Party Parrot GIF","username":"parrots","url":"https://giphy.com/gifs/abc",
            "images":{"fixed_width_small":{"url":"https://media.giphy.com/abc/100w.gif"},
                      "downsized":{"url":"https://media.giphy.com/abc/giphy-downsized.gif","width":"240","height":"180"}}},
            {"id":"bad","images":{"downsized":{"url":"http://insecure"}}}],"meta":{"status":200,"msg":"OK"}}"#;
        let gifs = parse(body.as_bytes()).unwrap();
        assert_eq!(gifs.len(), 1, "no HTTPS, no result");
        assert_eq!(gifs[0].credit(), "Party Parrot GIF by @parrots (GIPHY)");
        assert_eq!((gifs[0].width, gifs[0].height), (240, 180));
        let err = parse(br#"{"meta":{"status":401,"msg":"Unauthorized"},"data":[]}"#).unwrap_err();
        assert!(err.contains("Unauthorized"));
    }
}
