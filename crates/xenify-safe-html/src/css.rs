//! Just enough CSS for email: `<style>` rules with simple selectors (tag,
//! `.class`, `#id`, and descendants of those) applied as inline styles, and
//! an allowlist of properties and values. Anything we don't understand is
//! dropped, never passed on.

/// An element as selectors see it.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub tag: String,
    pub classes: Vec<String>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
}

impl Compound {
    fn matches(&self, e: &Info) -> bool {
        self.tag.as_ref().is_none_or(|t| *t == e.tag)
            && self.id.as_ref().is_none_or(|i| e.id.as_ref() == Some(i))
            && self.classes.iter().all(|c| e.classes.contains(c))
    }
}

#[derive(Clone, Debug)]
struct Rule {
    /// Ancestors first; the last one is the element itself.
    selector: Vec<Compound>,
    specificity: u32,
    decls: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    rules: Vec<Rule>,
}

/// More rules than any real email has; stops a hostile one making us slow.
const MAX_RULES: usize = 4000;

impl Sheet {
    pub fn parse(css: &str) -> Sheet {
        let css = strip_comments(css);
        let mut rules = Vec::new();
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let prelude = rest[..open].trim();
            let Some(close) = block_end(&rest[open..]).map(|e| open + e) else { break };
            let body = &rest[open + 1..close];
            rest = &rest[close + 1..];
            // At-rules (@media, @font-face, @import…) are skipped whole.
            if prelude.starts_with('@') || prelude.contains('@') {
                continue;
            }
            let decls = parse_decls(body);
            if decls.is_empty() {
                continue;
            }
            for sel in prelude.split(',') {
                if let Some(selector) = parse_selector(sel.trim()) {
                    let specificity = selector
                        .iter()
                        .map(|c| c.id.is_some() as u32 * 100 + c.classes.len() as u32 * 10 + c.tag.is_some() as u32)
                        .sum();
                    rules.push(Rule { selector, specificity, decls: decls.clone() });
                }
            }
            if rules.len() > MAX_RULES {
                break;
            }
        }
        // Stable sort: equal specificity keeps source order (later wins).
        rules.sort_by_key(|r| r.specificity);
        Sheet { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The declarations for `el` (inside `ancestors`), weakest first.
    pub fn matching(&self, ancestors: &[Info], el: &Info) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for r in &self.rules {
            let (last, before) = r.selector.split_last().expect("selectors aren't empty");
            if !last.matches(el) {
                continue;
            }
            // Descendant combinators: each earlier part matches some ancestor,
            // in order, nearest first.
            let mut anc = ancestors.iter().rev();
            if before.iter().rev().all(|c| anc.by_ref().any(|a| c.matches(a))) {
                out.extend(r.decls.iter().cloned());
            }
        }
        out
    }
}

/// Where the `{…}` block starting at `s[0]` ends (nested braces counted).
fn block_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `div.note p` → [div.note, p]. None for anything fancier (`>`, `+`, `:hover`,
/// `[attr]`…): those rules are skipped.
fn parse_selector(sel: &str) -> Option<Vec<Compound>> {
    if sel.is_empty() || sel.contains(['>', '+', '~', ':', '[', '*', '(', '\\']) {
        return None;
    }
    let mut out = Vec::new();
    for part in sel.split_whitespace() {
        let mut c = Compound::default();
        let mut cur = String::new();
        let mut kind = 't';
        let flush = |kind: char, cur: &mut String, c: &mut Compound| {
            let v = std::mem::take(cur).to_ascii_lowercase();
            if v.is_empty() {
                return;
            }
            match kind {
                '.' => c.classes.push(v),
                '#' => c.id = Some(v),
                _ => c.tag = Some(v),
            }
        };
        for ch in part.chars() {
            match ch {
                '.' | '#' => {
                    flush(kind, &mut cur, &mut c);
                    kind = ch;
                }
                c2 if c2.is_alphanumeric() || c2 == '-' || c2 == '_' => cur.push(c2),
                _ => return None,
            }
        }
        flush(kind, &mut cur, &mut c);
        if c == Compound::default() {
            return None;
        }
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

/// `color: red; font-weight: bold !important` → [(color, red), (font-weight, bold)].
pub fn parse_decls(s: &str) -> Vec<(String, String)> {
    let s = strip_comments(s);
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().trim_end_matches("!important").trim();
            (!k.is_empty() && !v.is_empty()).then(|| (k, v.to_string()))
        })
        .collect()
}

/// Properties that pass through (others are dropped).
const ALLOWED: &[&str] = &[
    "color",
    "background-color",
    "background",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "text-decoration",
    "text-align",
    "text-transform",
    "vertical-align",
    "line-height",
    "letter-spacing",
    "white-space",
    "width",
    "max-width",
    "min-width",
    "height",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-color",
    "border-style",
    "border-width",
    "border-collapse",
    "display",
    "list-style-type",
];

/// The declaration if it's harmless, cleaned up; None to drop it.
pub fn safe_decl(prop: &str, value: &str) -> Option<(String, String)> {
    if !ALLOWED.contains(&prop) {
        return None;
    }
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    // Anything that can load, run or hide things.
    const BAD: &[&str] =
        &["url(", "expression", "javascript", "vbscript", "behavior", "binding", "image-set", "var(", "attr(", "@"];
    let font = prop == "font-family" || prop == "font";
    if v.len() > 300
        || BAD.iter().any(|b| lower.contains(b))
        || v.contains(['\\', '<', '>', '{', '}', ';'])
        || (!font && v.contains(['"', '\'']))
    {
        return None;
    }
    if prop == "display" && !matches!(lower.as_str(), "none" | "block" | "inline" | "inline-block") {
        return None;
    }
    let v = if font { v.replace(['"', '\''], "") } else { v.to_string() };
    Some((prop.to_string(), v))
}

/// True when these declarations hide the element (it isn't shown at all).
pub fn hides(decls: &[(String, String)]) -> bool {
    let get = |k: &str| decls.iter().rev().find(|(p, _)| p == k).map(|(_, v)| v.to_ascii_lowercase());
    let zero = |v: Option<String>| v.is_some_and(|v| matches!(v.trim(), "0" | "0px" | "0pt" | "0em" | "0%"));
    get("display").as_deref() == Some("none")
        || get("visibility").as_deref() == Some("hidden")
        || get("opacity").is_some_and(|v| v.trim().parse::<f32>().is_ok_and(|o| o < 0.05))
        || zero(get("font-size"))
        || ((zero(get("max-height")) || zero(get("height"))) && get("overflow").as_deref() == Some("hidden"))
}

/// A length in CSS pixels (`12px`, `9pt`, `12`), if it is one.
pub fn px(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(n) = v.strip_suffix("px") {
        n.trim().parse().ok()
    } else if let Some(n) = v.strip_suffix("pt") {
        n.trim().parse::<f32>().ok().map(|n| n * 4.0 / 3.0)
    } else {
        v.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(tag: &str, classes: &[&str], id: Option<&str>) -> Info {
        Info { tag: tag.into(), classes: classes.iter().map(|c| c.to_string()).collect(), id: id.map(str::to_string) }
    }

    #[test]
    fn rules_match_by_tag_class_id_and_descendant() {
        let sheet = Sheet::parse(
            "/* c */ p { color: red } .note { color: blue } #main p { font-weight: bold }
             @media (max-width: 600px) { p { color: green } } a:hover { color: pink } td > p { color: grey }",
        );
        let body = info("div", &[], Some("main"));
        let p = info("p", &["note"], None);
        assert_eq!(
            sheet.matching(&[body], &p),
            [("color".into(), "red".into()), ("color".into(), "blue".into()), ("font-weight".into(), "bold".into())]
        );
        assert!(sheet.matching(&[], &info("a", &[], None)).is_empty());
    }

    #[test]
    fn dangerous_values_are_dropped() {
        assert!(safe_decl("background", "url(http://tracker/x.gif)").is_none());
        assert!(safe_decl("width", "expression(alert(1))").is_none());
        assert!(safe_decl("position", "fixed").is_none());
        assert!(safe_decl("color", "red\\3b background:url(x)").is_none());
        assert!(safe_decl("display", "flex").is_none());
        assert_eq!(
            safe_decl("font-family", "'Segoe UI', Arial"),
            Some(("font-family".into(), "Segoe UI, Arial".into()))
        );
        assert_eq!(safe_decl("color", "#333"), Some(("color".into(), "#333".into())));
    }

    #[test]
    fn hidden_preheaders_are_recognised() {
        assert!(hides(&parse_decls("display:none")));
        assert!(hides(&parse_decls("max-height:0; overflow:hidden")));
        assert!(hides(&parse_decls("font-size:0px")));
        assert!(!hides(&parse_decls("color:red; height:0")));
    }
}
