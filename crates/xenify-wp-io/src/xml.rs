//! XML parsing with a nesting limit. roxmltree reads nested elements by
//! recursion, so a made-up file nesting a few hundred thousand elements
//! would run out of stack and crash whatever reads it (a server too). Real
//! Office files nest a few dozen levels; [`MAX_DEPTH`] is well over that,
//! and stays inside a 2 MiB thread stack even in a debug build (about
//! 160 levels fit there).

/// The deepest elements may nest.
pub const MAX_DEPTH: usize = 100;

#[derive(Debug)]
pub enum XmlError {
    TooDeep,
    Xml(roxmltree::Error),
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XmlError::TooDeep => f.write_str("its parts nest too deeply"),
            XmlError::Xml(e) => e.fmt(f),
        }
    }
}

/// `roxmltree::Document::parse`, refusing text nested deeper than [`MAX_DEPTH`].
pub fn parse(text: &str) -> Result<roxmltree::Document<'_>, XmlError> {
    if depth(text.as_bytes(), MAX_DEPTH) > MAX_DEPTH {
        return Err(XmlError::TooDeep);
    }
    roxmltree::Document::parse(text).map_err(XmlError::Xml)
}

/// How deeply `b`'s elements nest (it stops counting once past `max`).
/// It errs on the deep side: anything it doesn't follow counts as opening.
fn depth(b: &[u8], max: usize) -> usize {
    let find = |from: usize, what: &[u8]| b.get(from..)?.windows(what.len()).position(|w| w == what).map(|p| from + p);
    let (mut i, mut d, mut most) = (0, 0usize, 0);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        let skip_to = |end: &[u8], from: usize| find(from, end).map_or(b.len(), |p| p + end.len());
        if rest.starts_with(b"<!--") {
            i = skip_to(b"-->", i + 4);
        } else if rest.starts_with(b"<![CDATA[") {
            i = skip_to(b"]]>", i + 9);
        } else if rest.starts_with(b"<?") {
            i = skip_to(b"?>", i + 2);
        } else if rest.starts_with(b"<!") {
            // A declaration: what's in it counts as more elements, if any.
            i = skip_to(b">", i + 2);
        } else if rest.starts_with(b"</") {
            d = d.saturating_sub(1);
            i = skip_to(b">", i + 2);
        } else {
            // An opening tag: to its end, past quoted values.
            let mut j = i + 1;
            let mut quote = None;
            let mut closed = false;
            while j < b.len() {
                match (quote, b[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, c @ (b'"' | b'\'')) => quote = Some(c),
                    (None, b'>') => {
                        closed = b[j - 1] == b'/';
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if !closed {
                d += 1;
                most = most.max(d);
                if most > max {
                    return most;
                }
            }
            i = j + 1;
        }
    }
    most
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_nesting() {
        assert_eq!(depth(b"<a><b/><c x='/>'><d></d></c></a>", 99), 3);
        assert_eq!(depth(b"<?xml version='1.0'?><!-- <x><x> --><a><![CDATA[<y><y>]]></a>", 99), 1);
        let deep = "<a>".repeat(100_000);
        assert!(parse(&deep).is_err_and(|e| matches!(e, XmlError::TooDeep)));
        let fine = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        assert!(parse(&fine).is_ok());
    }
}
