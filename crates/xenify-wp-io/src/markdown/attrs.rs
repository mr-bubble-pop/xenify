//! Pandoc-style attribute blocks: `{#id .class key=value key="quoted"}`.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs {
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub pairs: Vec<(String, String)>,
}

impl Attrs {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn has_class(&self, c: &str) -> bool {
        self.classes.iter().any(|x| x == c)
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.classes.is_empty() && self.pairs.is_empty()
    }

    /// Parses the inside of `{…}` (without the braces).
    pub fn parse(s: &str) -> Attrs {
        let mut out = Attrs::default();
        let mut chars = s.chars().peekable();
        loop {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            let Some(&c) = chars.peek() else { break };
            let word = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                let mut w = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '=' {
                        break;
                    }
                    w.push(c);
                    chars.next();
                }
                w
            };
            match c {
                '#' => {
                    chars.next();
                    out.id = Some(word(&mut chars));
                }
                '.' => {
                    chars.next();
                    out.classes.push(word(&mut chars));
                }
                _ => {
                    let key = word(&mut chars);
                    if chars.peek() == Some(&'=') {
                        chars.next();
                        let value = if chars.peek() == Some(&'"') {
                            chars.next();
                            let mut v = String::new();
                            while let Some(c) = chars.next() {
                                match c {
                                    '\\' => {
                                        if let Some(n) = chars.next() {
                                            v.push(n);
                                        }
                                    }
                                    '"' => break,
                                    _ => v.push(c),
                                }
                            }
                            v
                        } else {
                            word(&mut chars)
                        };
                        out.pairs.push((key, value));
                    } else if !key.is_empty() {
                        // A bare word: treat as a class (Pandoc's `::: note`).
                        out.classes.push(key);
                    } else {
                        chars.next();
                    }
                }
            }
        }
        out
    }

    /// Writes `{…}` (with braces).
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        if let Some(id) = &self.id {
            parts.push(format!("#{id}"));
        }
        for c in &self.classes {
            parts.push(format!(".{c}"));
        }
        for (k, v) in &self.pairs {
            let plain = !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || "#.-_%".contains(c));
            if plain {
                parts.push(format!("{k}={v}"));
            } else {
                parts.push(format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")));
            }
        }
        format!("{{{}}}", parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders() {
        let a = Attrs::parse(r#"#intro .note .underline custom-style="Block Text" size=14 color=#c00"#);
        assert_eq!(a.id.as_deref(), Some("intro"));
        assert_eq!(a.classes, ["note", "underline"]);
        assert_eq!(a.get("custom-style"), Some("Block Text"));
        assert_eq!(a.get("size"), Some("14"));
        assert_eq!(Attrs::parse(&a.render()[1..a.render().len() - 1]), a);
        assert_eq!(Attrs::parse("warning").classes, ["warning"]);
    }
}
