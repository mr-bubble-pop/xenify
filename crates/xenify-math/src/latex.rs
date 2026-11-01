//! A [`Node`] tree back to LaTeX (what the visual editor saves). Parsing the
//! result gives the same tree.

use crate::parse::{Class, Env, FUNCTIONS, LIMIT_FUNCTIONS, Node, Variant, accent, big_op, symbol};

/// Named symbols, by the name written for them (the first of any aliases).
const SYMBOL_NAMES: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "varepsilon",
    "zeta",
    "eta",
    "theta",
    "vartheta",
    "iota",
    "kappa",
    "lambda",
    "mu",
    "nu",
    "xi",
    "pi",
    "varpi",
    "rho",
    "varrho",
    "sigma",
    "varsigma",
    "tau",
    "upsilon",
    "phi",
    "varphi",
    "chi",
    "psi",
    "omega",
    "Gamma",
    "Delta",
    "Theta",
    "Lambda",
    "Xi",
    "Pi",
    "Sigma",
    "Upsilon",
    "Phi",
    "Psi",
    "Omega",
    "infty",
    "partial",
    "nabla",
    "emptyset",
    "forall",
    "exists",
    "neg",
    "hbar",
    "ell",
    "Re",
    "Im",
    "aleph",
    "angle",
    "triangle",
    "prime",
    "degree",
    "ldots",
    "cdots",
    "vdots",
    "ddots",
    "pm",
    "mp",
    "times",
    "div",
    "cdot",
    "ast",
    "star",
    "circ",
    "bullet",
    "oplus",
    "otimes",
    "cup",
    "cap",
    "wedge",
    "vee",
    "setminus",
    "le",
    "ge",
    "ne",
    "approx",
    "equiv",
    "sim",
    "simeq",
    "cong",
    "propto",
    "ll",
    "gg",
    "in",
    "notin",
    "ni",
    "subset",
    "supset",
    "subseteq",
    "supseteq",
    "perp",
    "parallel",
    "mid",
    "to",
    "leftarrow",
    "leftrightarrow",
    "Rightarrow",
    "Leftarrow",
    "Leftrightarrow",
    "mapsto",
    "uparrow",
    "downarrow",
];

const BIG_OPS: &[&str] = &[
    "sum",
    "prod",
    "coprod",
    "bigcup",
    "bigcap",
    "bigoplus",
    "bigotimes",
    "bigvee",
    "bigwedge",
    "int",
    "iint",
    "iiint",
    "oint",
];

const ACCENTS: &[&str] =
    &["hat", "widehat", "check", "tilde", "widetilde", "acute", "grave", "dot", "ddot", "breve", "bar", "vec"];

/// `node` as LaTeX.
pub fn to_latex(node: &Node) -> String {
    let mut out = String::new();
    write(node, &mut out);
    out.trim().to_string()
}

/// Appends `s`, with a space where a command name would run into a letter.
fn push(out: &mut String, s: &str) {
    let ends_in_command = {
        let letters = out.len() - out.trim_end_matches(|c: char| c.is_ascii_alphabetic()).len();
        letters > 0 && out[..out.len() - letters].ends_with('\\')
    };
    if ends_in_command && s.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.push(' ');
    }
    out.push_str(s);
}

/// `{…}` around `n`.
fn group(n: &Node) -> String {
    format!("{{{}}}", to_latex(n))
}

fn variant_command(v: Variant) -> Option<&'static str> {
    Some(match v {
        Variant::Normal => return None,
        Variant::Upright => "mathrm",
        Variant::Bold => "mathbf",
        Variant::Blackboard => "mathbb",
        Variant::Calligraphic => "mathcal",
        Variant::Fraktur => "mathfrak",
        Variant::Mono => "mathtt",
        Variant::Sans => "mathsf",
    })
}

fn sym_text(ch: char) -> String {
    match ch {
        '−' => "-".into(),
        '∗' => "*".into(),
        '{' => "\\{".into(),
        '}' => "\\}".into(),
        '‖' => "\\|".into(),
        '%' | '#' | '$' | '&' | '_' => format!("\\{ch}"),
        c if c.is_ascii() => c.to_string(),
        c => SYMBOL_NAMES
            .iter()
            .find(|n| symbol(n).is_some_and(|(s, _)| s == c))
            .map_or_else(|| c.to_string(), |n| format!("\\{n}")),
    }
}

fn delim(d: Option<char>) -> &'static str {
    match d {
        None => ".",
        Some('(') => "(",
        Some(')') => ")",
        Some('[') => "[",
        Some(']') => "]",
        Some('{') => "\\{",
        Some('}') => "\\}",
        Some('|') => "|",
        Some('‖') => "\\|",
        Some('⟨') => "\\langle",
        Some('⟩') => "\\rangle",
        Some('⌊') => "\\lfloor",
        Some('⌋') => "\\rfloor",
        Some('⌈') => "\\lceil",
        Some('⌉') => "\\rceil",
        Some('/') => "/",
        Some(_) => ".",
    }
}

fn write(n: &Node, out: &mut String) {
    match n {
        Node::Row(items) => {
            let mut i = 0;
            while i < items.len() {
                // A run of letters in one style: \mathbf{xy}.
                if let Node::Sym { variant, class: Class::Ord, .. } = &items[i]
                    && let Some(cmd) = variant_command(*variant)
                {
                    let mut text = String::new();
                    while let Some(Node::Sym { ch, variant: v, class: Class::Ord }) = items.get(i)
                        && v == variant
                    {
                        push(&mut text, &sym_text(*ch));
                        i += 1;
                    }
                    push(out, &format!("\\{cmd}{{{text}}}"));
                    continue;
                }
                write(&items[i], out);
                i += 1;
            }
        }
        Node::Sym { ch, variant, .. } => {
            let t = sym_text(*ch);
            match variant_command(*variant) {
                Some(cmd) => push(out, &format!("\\{cmd}{{{t}}}")),
                None => push(out, &t),
            }
        }
        Node::Text { text, class } => {
            let s = if *class == Class::Op
                && (FUNCTIONS.contains(&text.as_str()) || LIMIT_FUNCTIONS.contains(&text.as_str()))
            {
                format!("\\{text}")
            } else if *class == Class::Op {
                format!("\\operatorname{{{text}}}")
            } else {
                format!("\\text{{{text}}}")
            };
            push(out, &s);
        }
        Node::Error(t) => push(out, t),
        Node::Space(mu) => {
            let s = match *mu as i32 {
                3 => "\\,",
                4 => "\\:",
                5 => "\\;",
                -3 => "\\!",
                18 => "\\quad",
                36 => "\\qquad",
                _ => "\\ ",
            };
            push(out, s);
        }
        Node::Frac { num, den, bar, style } => {
            let cmd = match (bar, style) {
                (false, _) => "binom",
                (_, Some(true)) => "dfrac",
                (_, Some(false)) => "tfrac",
                _ => "frac",
            };
            push(out, &format!("\\{cmd}{}{}", group(num), group(den)));
        }
        Node::Scripts { base, sub, sup } => {
            match &**base {
                Node::Row(v) if v.len() != 1 => push(out, &group(base)),
                Node::Scripts { .. } => push(out, &group(base)),
                b => write(b, out),
            }
            if let Some(s) = sub {
                push(out, &format!("_{}", group(s)));
            }
            if let Some(s) = sup {
                push(out, &format!("^{}", group(s)));
            }
        }
        Node::Sqrt { index, body } => {
            let ix = index.as_ref().map(|i| format!("[{}]", to_latex(i))).unwrap_or_default();
            push(out, &format!("\\sqrt{ix}{}", group(body)));
        }
        Node::BigOp { ch, .. } => {
            let name = BIG_OPS.iter().find(|n| big_op(n).is_some_and(|(c, _)| c == *ch)).unwrap_or(&"sum");
            push(out, &format!("\\{name}"));
        }
        Node::Fenced { open: Some('('), close: Some(')'), body }
            if matches!(&**body, Node::Frac { bar: false, .. }) =>
        {
            write(body, out);
        }
        Node::Fenced { open, close, body } => {
            push(out, &format!("\\left{} {} \\right{}", delim(*open), to_latex(body), delim(*close)));
        }
        Node::Accent { ch, body, wide } => {
            let name = ACCENTS.iter().find(|n| accent(n) == Some((*ch, *wide))).unwrap_or(&"hat");
            push(out, &format!("\\{name}{}", group(body)));
        }
        Node::Over { body } => push(out, &format!("\\overline{}", group(body))),
        Node::Under { body } => push(out, &format!("\\underline{}", group(body))),
        Node::Matrix { env, rows } => {
            let name = match env {
                Env::Matrix => "matrix",
                Env::Delimited('(', _) => "pmatrix",
                Env::Delimited('[', _) => "bmatrix",
                Env::Delimited('{', _) => "Bmatrix",
                Env::Delimited('‖', _) => "Vmatrix",
                Env::Delimited(..) => "vmatrix",
                Env::Cases => "cases",
                Env::Aligned => "aligned",
            };
            let body: Vec<String> =
                rows.iter().map(|r| r.iter().map(to_latex).collect::<Vec<_>>().join(" & ")).collect();
            push(out, &format!("\\begin{{{name}}} {} \\end{{{name}}}", body.join(" \\\\ ")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn writes_what_parses_back_the_same() {
        for s in [
            "x^2 + y_i^{n+1}",
            "\\frac{a+b}{c}",
            "\\dfrac{1}{2}",
            "\\sqrt[3]{x} + \\sqrt{y}",
            "\\left( x \\right) + \\left[ \\frac{a}{b} \\right.",
            "\\sum_{i=1}^{n} i^2 = \\frac{n(n+1)(2n+1)}{6}",
            "\\int_0^\\infty e^{-x} dx",
            "\\alpha \\beta \\le \\gamma \\to \\infty",
            "\\hat{x} + \\vec{v} + \\overline{z} + \\underline{w}",
            "\\mathbf{xy} + \\mathbb{R} + \\mathcal{L}",
            "\\sin x + \\lim_{x \\to 0} f(x) + \\operatorname{tr} A",
            "\\text{if } x > 0",
            "\\binom{n}{k}",
            "\\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix}",
            "f(x) = \\begin{cases} 1 & x > 0 \\\\ 0 & x \\le 0 \\end{cases}",
            "a \\, b \\quad c",
            "\\{ x \\} \\% \\nosuch",
            "{ab}^2",
        ] {
            let n = parse(s);
            let back = to_latex(&n);
            assert_eq!(parse(&back), n, "{s} → {back}");
        }
    }
}
