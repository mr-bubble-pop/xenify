//! Small helpers several Xenify crates share, so each keeps one copy:
//! escaping text for XML and HTML, and the time in milliseconds. No
//! dependencies; builds for the browser (WebAssembly) too.

/// Text made safe inside XML or HTML, in text and in "double-quoted"
/// attributes: `&`, `<`, `>` and `"` become entities.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Milliseconds since 1970 (0 if the clock is before then). Not in the
/// browser build, whose clock comes from the page.
#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup() {
        assert_eq!(esc(r#"a < b & "c" > d"#), "a &lt; b &amp; &quot;c&quot; &gt; d");
        assert_eq!(esc("plain"), "plain");
        assert!(now_ms() > 1_700_000_000_000);
    }
}
