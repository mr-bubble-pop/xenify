//! A [`Node`] tree as Word's equation markup (OMML), so equations written
//! here are real, editable equations in Word.

use crate::parse::{Env, Node, Variant};

use xenify_util::esc;

fn run(text: &str, plain: bool) -> String {
    let pr = if plain { "<m:rPr><m:sty m:val=\"p\"/></m:rPr>" } else { "" };
    format!("<m:r>{pr}<m:t>{}</m:t></m:r>", esc(text))
}

fn e(n: &Node) -> String {
    format!("<m:e>{}</m:e>", node(n))
}

fn wrap(tag: &str, n: &Node) -> String {
    format!("<m:{tag}>{}</m:{tag}>", node(n))
}

fn node(n: &Node) -> String {
    match n {
        Node::Sym { ch, variant, .. } => run(&ch.to_string(), *variant == Variant::Upright),
        Node::Text { text, .. } => run(text, true),
        Node::Error(t) => run(t, true),
        Node::Space(_) => run(" ", true),
        Node::Row(v) => v.iter().map(node).collect(),
        Node::Frac { num, den, bar, .. } => {
            let pr = if *bar { "" } else { "<m:fPr><m:type m:val=\"noBar\"/></m:fPr>" };
            format!("<m:f>{pr}{}{}</m:f>", wrap("num", num), wrap("den", den))
        }
        Node::Scripts { base, sub, sup } => {
            // A big operator with scripts: Word's n-ary.
            if let Node::BigOp { ch, limits } = &**base {
                let loc = if *limits { "undOvr" } else { "subSup" };
                let mut pr = format!("<m:naryPr><m:chr m:val=\"{ch}\"/><m:limLoc m:val=\"{loc}\"/>");
                if sub.is_none() {
                    pr.push_str("<m:subHide m:val=\"1\"/>");
                }
                if sup.is_none() {
                    pr.push_str("<m:supHide m:val=\"1\"/>");
                }
                pr.push_str("</m:naryPr>");
                let empty = Node::Row(Vec::new());
                return format!(
                    "<m:nary>{pr}{}{}<m:e/></m:nary>",
                    wrap("sub", sub.as_deref().unwrap_or(&empty)),
                    wrap("sup", sup.as_deref().unwrap_or(&empty))
                );
            }
            match (sub, sup) {
                (Some(b), Some(p)) => format!("<m:sSubSup>{}{}{}</m:sSubSup>", e(base), wrap("sub", b), wrap("sup", p)),
                (Some(b), None) => format!("<m:sSub>{}{}</m:sSub>", e(base), wrap("sub", b)),
                (None, Some(p)) => format!("<m:sSup>{}{}</m:sSup>", e(base), wrap("sup", p)),
                (None, None) => node(base),
            }
        }
        Node::BigOp { ch, .. } => {
            format!(
                "<m:nary><m:naryPr><m:chr m:val=\"{ch}\"/><m:subHide m:val=\"1\"/><m:supHide m:val=\"1\"/></m:naryPr><m:sub/><m:sup/><m:e/></m:nary>"
            )
        }
        Node::Sqrt { index, body } => match index {
            Some(i) => format!("<m:rad>{}{}</m:rad>", wrap("deg", i), e(body)),
            None => format!("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>{}</m:rad>", e(body)),
        },
        Node::Fenced { open, close, body } => {
            let o = open.map_or(String::new(), |c| c.to_string());
            let c = close.map_or(String::new(), |c| c.to_string());
            format!(
                "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/></m:dPr>{}</m:d>",
                esc(&o),
                esc(&c),
                e(body)
            )
        }
        Node::Accent { ch, body, .. } => {
            format!("<m:acc><m:accPr><m:chr m:val=\"{ch}\"/></m:accPr>{}</m:acc>", e(body))
        }
        Node::Over { body } => format!("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr>{}</m:bar>", e(body)),
        Node::Under { body } => format!("<m:bar><m:barPr><m:pos m:val=\"bot\"/></m:barPr>{}</m:bar>", e(body)),
        Node::Matrix { env, rows } => {
            if *env == Env::Aligned {
                let lines: String =
                    rows.iter().map(|r| format!("<m:e>{}</m:e>", r.iter().map(node).collect::<String>())).collect();
                return format!("<m:eqArr>{lines}</m:eqArr>");
            }
            let m: String =
                rows.iter().map(|r| format!("<m:mr>{}</m:mr>", r.iter().map(e).collect::<String>())).collect();
            let m = format!("<m:m>{m}</m:m>");
            match env {
                Env::Delimited(o, c) => format!(
                    "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/></m:dPr><m:e>{m}</m:e></m:d>",
                    esc(&o.to_string()),
                    esc(&c.to_string())
                ),
                Env::Cases => {
                    format!("<m:d><m:dPr><m:begChr m:val=\"{{\"/><m:endChr m:val=\"\"/></m:dPr><m:e>{m}</m:e></m:d>")
                }
                _ => m,
            }
        }
    }
}

/// `n` as an `m:oMath` element.
pub fn to_omml(n: &Node) -> String {
    format!("<m:oMath>{}</m:oMath>", node(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    #[test]
    fn writes_word_equations() {
        assert_eq!(
            to_omml(&parse("\\frac{a}{b}")),
            "<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>"
        );
        let s = to_omml(&parse("\\sum_{i=1}^{n} x_i"));
        assert!(s.contains("<m:nary><m:naryPr><m:chr m:val=\"∑\"/><m:limLoc m:val=\"undOvr\"/>"), "{s}");
        assert!(to_omml(&parse("\\sqrt{x}")).contains("<m:degHide m:val=\"1\"/>"));
        assert!(to_omml(&parse("\\begin{pmatrix}1&0\\\\0&1\\end{pmatrix}")).contains("<m:m><m:mr>"));
    }
}
