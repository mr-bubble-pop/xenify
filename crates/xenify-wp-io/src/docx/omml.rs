//! Word's equations (Office Math Markup, OMML) to LaTeX: fractions, scripts,
//! roots, sums and integrals, brackets, functions, limits, accents, bars,
//! matrices and aligned lines. Anything else keeps its text.

use roxmltree::Node;

const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

fn is(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

fn child<'a, 'i>(n: &Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(c, name))
}

/// A property of `n`: `<m:XPr><m:name m:val="…"/></m:XPr>`.
fn prop<'a>(n: &Node<'a, '_>, pr: &str, name: &str) -> Option<&'a str> {
    let p = child(n, pr)?;
    let c = child(&p, name)?;
    Some(c.attribute((M, "val")).or_else(|| c.attribute("val")).unwrap_or("on"))
}

/// A switch property that's on (`<m:subHide/>` or `m:val="1"`).
fn switch(n: &Node, pr: &str, name: &str) -> bool {
    prop(n, pr, name).is_some_and(|v| !matches!(v, "0" | "off" | "false"))
}

/// The LaTeX for the children of `n` (an argument: `e`, `num`, `sub`…).
fn arg(n: Option<Node>) -> String {
    n.map_or(String::new(), |n| n.children().map(|c| node(&c)).collect::<String>()).trim().to_string()
}

/// `{…}` around an argument.
fn braced(n: Option<Node>) -> String {
    format!("{{{}}}", arg(n))
}

/// Greek letters and common symbols as LaTeX commands.
fn symbol(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' => "\\epsilon",
        'ϵ' => "\\epsilon",
        'ζ' => "\\zeta",
        'η' => "\\eta",
        'θ' => "\\theta",
        'ι' => "\\iota",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' => "\\mu",
        'ν' => "\\nu",
        'ξ' => "\\xi",
        'π' => "\\pi",
        'ρ' => "\\rho",
        'σ' => "\\sigma",
        'τ' => "\\tau",
        'υ' => "\\upsilon",
        'φ' => "\\phi",
        'ϕ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'Γ' => "\\Gamma",
        'Δ' => "\\Delta",
        'Θ' => "\\Theta",
        'Λ' => "\\Lambda",
        'Ξ' => "\\Xi",
        'Π' => "\\Pi",
        'Σ' => "\\Sigma",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Ω' => "\\Omega",
        '≤' => "\\le",
        '≥' => "\\ge",
        '≠' => "\\ne",
        '≈' => "\\approx",
        '≡' => "\\equiv",
        '±' => "\\pm",
        '∓' => "\\mp",
        '×' => "\\times",
        '÷' => "\\div",
        '·' | '⋅' => "\\cdot",
        '∞' => "\\infty",
        '→' => "\\to",
        '←' => "\\leftarrow",
        '⇒' => "\\Rightarrow",
        '⇔' => "\\Leftrightarrow",
        '∈' => "\\in",
        '∉' => "\\notin",
        '⊂' => "\\subset",
        '⊆' => "\\subseteq",
        '∪' => "\\cup",
        '∩' => "\\cap",
        '∀' => "\\forall",
        '∃' => "\\exists",
        '∂' => "\\partial",
        '∇' => "\\nabla",
        '∅' => "\\emptyset",
        '…' => "\\ldots",
        '⋯' => "\\cdots",
        '∝' => "\\propto",
        '∠' => "\\angle",
        '⊥' => "\\perp",
        '°' => "^{\\circ}",
        '′' => "'",
        _ => return None,
    })
}

/// A run's text as LaTeX.
fn text(t: &str, plain: bool) -> String {
    if plain && t.chars().any(char::is_alphabetic) {
        return format!("\\text{{{t}}}");
    }
    let mut out = String::new();
    for c in t.chars() {
        match symbol(c) {
            Some(cmd) => {
                out.push_str(cmd);
                // A command followed by a letter needs a space ("\alpha x").
                out.push(' ');
            }
            None if matches!(c, '{' | '}' | '%' | '#' | '_' | '$') => {
                out.push('\\');
                out.push(c);
            }
            None => out.push(c),
        }
    }
    out
}

/// A big operator's LaTeX.
fn nary_op(c: &str) -> &'static str {
    match c {
        "∑" => "\\sum",
        "∏" => "\\prod",
        "∐" => "\\coprod",
        "∬" => "\\iint",
        "∭" => "\\iiint",
        "∮" => "\\oint",
        "⋃" => "\\bigcup",
        "⋂" => "\\bigcap",
        "⋁" => "\\bigvee",
        "⋀" => "\\bigwedge",
        _ => "\\int",
    }
}

/// A delimiter for `\left`/`\right`.
fn delim(c: &str) -> String {
    match c {
        "" => ".".into(),
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "⟨" | "〈" => "\\langle".into(),
        "⟩" | "〉" => "\\rangle".into(),
        "|" => "|".into(),
        "‖" => "\\|".into(),
        "⌊" => "\\lfloor".into(),
        "⌋" => "\\rfloor".into(),
        "⌈" => "\\lceil".into(),
        "⌉" => "\\rceil".into(),
        c => c.into(),
    }
}

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "log", "ln", "exp",
    "lim", "max", "min", "sup", "inf", "det", "dim", "gcd", "deg", "arg",
];

/// How deep maths may nest (real equations go a few levels; a made-up
/// file nesting thousands would run out of stack). Deeper parts are left out.
const MAX_DEPTH: usize = 64;

thread_local! {
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One level deeper while it lives.
struct Deeper;

impl Deeper {
    fn enter() -> Option<Deeper> {
        DEPTH.with(|d| {
            (d.get() < MAX_DEPTH).then(|| {
                d.set(d.get() + 1);
                Deeper
            })
        })
    }
}

impl Drop for Deeper {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// The LaTeX for one OMML node.
fn node(n: &Node) -> String {
    if !n.is_element() {
        return String::new();
    }
    let Some(_deeper) = Deeper::enter() else { return String::new() };
    let e = || child(n, "e");
    match n.tag_name().name() {
        "r" => {
            let plain = child(n, "rPr").and_then(|p| child(&p, "nor")).is_some() || prop(n, "rPr", "sty") == Some("p");
            let t: String = n.children().filter(|c| is(c, "t")).filter_map(|c| c.text()).collect();
            text(&t, plain && t.chars().count() > 1)
        }
        "f" => {
            let (num, den) = (child(n, "num"), child(n, "den"));
            match prop(n, "fPr", "type") {
                Some("noBar") => format!("\\binom{}{}", braced(num), braced(den)),
                Some("lin") => format!("{}/{}", arg(num), arg(den)),
                _ => format!("\\frac{}{}", braced(num), braced(den)),
            }
        }
        "sSup" => format!("{}^{}", braced(e()), braced(child(n, "sup"))),
        "sSub" => format!("{}_{}", braced(e()), braced(child(n, "sub"))),
        "sSubSup" => format!("{}_{}^{}", braced(e()), braced(child(n, "sub")), braced(child(n, "sup"))),
        "sPre" => format!("{{}}_{}^{}{}", braced(child(n, "sub")), braced(child(n, "sup")), braced(e())),
        "rad" => {
            let deg = arg(child(n, "deg"));
            if deg.is_empty() || switch(n, "radPr", "degHide") {
                format!("\\sqrt{}", braced(e()))
            } else {
                format!("\\sqrt[{deg}]{}", braced(e()))
            }
        }
        "nary" => {
            let op = nary_op(prop(n, "naryPr", "chr").unwrap_or("∫"));
            let mut s = op.to_string();
            let sub = arg(child(n, "sub"));
            let sup = arg(child(n, "sup"));
            if !switch(n, "naryPr", "subHide") && !sub.is_empty() {
                s.push_str(&format!("_{{{sub}}}"));
            }
            if !switch(n, "naryPr", "supHide") && !sup.is_empty() {
                s.push_str(&format!("^{{{sup}}}"));
            }
            format!("{s} {}", arg(e()))
        }
        "d" => {
            let beg = prop(n, "dPr", "begChr").unwrap_or("(");
            let end = prop(n, "dPr", "endChr").unwrap_or(")");
            let sep = prop(n, "dPr", "sepChr").unwrap_or("|");
            let parts: Vec<String> = n.children().filter(|c| is(c, "e")).map(|c| arg(Some(c))).collect();
            format!("\\left{} {} \\right{}", delim(beg), parts.join(&format!(" {} ", text(sep, false))), delim(end))
        }
        "func" => {
            let name = arg(child(n, "fName"));
            let name = if FUNCTIONS.contains(&name.as_str()) {
                format!("\\{name}")
            } else if name.starts_with('\\') {
                name
            } else {
                format!("\\operatorname{{{name}}}")
            };
            format!("{name} {}", arg(e()))
        }
        "limLow" => {
            let base = arg(e());
            let lim = arg(child(n, "lim"));
            if FUNCTIONS.contains(&base.trim_start_matches("\\text{").trim_end_matches('}')) || base == "\\lim" {
                format!("\\lim_{{{lim}}}")
            } else {
                format!("\\underset{{{lim}}}{{{base}}}")
            }
        }
        "limUpp" => format!("\\overset{}{}", braced(child(n, "lim")), braced(e())),
        "acc" => {
            let cmd = match prop(n, "accPr", "chr").unwrap_or("\u{302}") {
                "\u{303}" => "\\tilde",
                "\u{304}" | "\u{305}" | "¯" => "\\bar",
                "\u{307}" => "\\dot",
                "\u{308}" => "\\ddot",
                "\u{20d7}" | "→" => "\\vec",
                "\u{306}" => "\\breve",
                "\u{30c}" => "\\check",
                _ => "\\hat",
            };
            format!("{cmd}{}", braced(e()))
        }
        "bar" => {
            let cmd = if prop(n, "barPr", "pos") == Some("top") { "\\overline" } else { "\\underline" };
            format!("{cmd}{}", braced(e()))
        }
        "groupChr" => {
            let top = prop(n, "groupChrPr", "pos") == Some("top");
            format!("{}{}", if top { "\\overbrace" } else { "\\underbrace" }, braced(e()))
        }
        "borderBox" => format!("\\boxed{}", braced(e())),
        "phant" => format!("\\phantom{}", braced(e())),
        "m" => {
            let rows: Vec<String> = n
                .children()
                .filter(|c| is(c, "mr"))
                .map(|r| r.children().filter(|c| is(c, "e")).map(|c| arg(Some(c))).collect::<Vec<_>>().join(" & "))
                .collect();
            format!("\\begin{{matrix}} {} \\end{{matrix}}", rows.join(" \\\\ "))
        }
        "eqArr" => {
            let rows: Vec<String> = n.children().filter(|c| is(c, "e")).map(|c| arg(Some(c))).collect();
            format!("\\begin{{aligned}} {} \\end{{aligned}}", rows.join(" \\\\ "))
        }
        "oMathPara" => n.children().filter(|c| is(c, "oMath")).map(|c| node(&c)).collect::<Vec<_>>().join(" \\\\ "),
        // Properties and Word's own runs inside maths draw nothing here.
        name if name.ends_with("Pr") => String::new(),
        _ => n.children().map(|c| node(&c)).collect(),
    }
}

/// An `m:oMath` or `m:oMathPara` element as LaTeX.
pub fn to_latex(n: &Node) -> String {
    let s = node(n);
    // Tidy spaces left by symbols ("\alpha  + x").
    let mut out = String::with_capacity(s.len());
    for c in s.trim().chars() {
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out.replace(" }", "}").replace(" ^", "^").replace(" _", "_")
}

#[cfg(test)]
mod tests {
    use super::to_latex;

    fn latex(inner: &str) -> String {
        let xml = format!(
            r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{inner}</m:oMath>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        to_latex(&doc.root_element())
    }

    fn r(t: &str) -> String {
        format!("<m:r><m:t>{t}</m:t></m:r>")
    }

    #[test]
    fn common_equations() {
        // Fraction and power.
        let frac = format!("<m:f><m:num>{}</m:num><m:den>{}</m:den></m:f>", r("a"), r("b"));
        assert_eq!(latex(&frac), "\\frac{a}{b}");
        let sq = format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", r("x"), r("2"));
        assert_eq!(latex(&format!("{sq}{}", r("+1"))), "{x}^{2}+1");
        // Square root and a sum with limits.
        let root = format!("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>", r("y"));
        assert_eq!(latex(&root), "\\sqrt{y}");
        let sum = format!(
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>",
            r("i=1"),
            r("n"),
            r("i")
        );
        assert_eq!(latex(&sum), "\\sum_{i=1}^{n} i");
        // Brackets, Greek, functions.
        let d = format!("<m:d><m:e>{}</m:e></m:d>", r("α+β"));
        assert_eq!(latex(&d), "\\left( \\alpha +\\beta \\right)");
        let f = format!("<m:func><m:fName>{}</m:fName><m:e>{}</m:e></m:func>", r("sin"), r("θ"));
        assert_eq!(latex(&f), "\\sin \\theta");
        // A matrix.
        let m = format!(
            "<m:m><m:mr><m:e>{}</m:e><m:e>{}</m:e></m:mr><m:mr><m:e>{}</m:e><m:e>{}</m:e></m:mr></m:m>",
            r("1"),
            r("0"),
            r("0"),
            r("1")
        );
        assert_eq!(latex(&m), "\\begin{matrix} 1 & 0 \\\\ 0 & 1 \\end{matrix}");
    }
}
