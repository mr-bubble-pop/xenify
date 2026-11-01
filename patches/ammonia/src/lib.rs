//! Stand-in for `ammonia` (see `patches/README.md`): returns the input with
//! all HTML escaped, which is always safe to show.

/// Escapes `src` so no markup in it takes effect.
pub fn clean(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for c in src.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_markup() {
        assert_eq!(super::clean("<b>a & 'b'</b>"), "&lt;b&gt;a &amp; &#39;b&#39;&lt;/b&gt;");
    }
}
