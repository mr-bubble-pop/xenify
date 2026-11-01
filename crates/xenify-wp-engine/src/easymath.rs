//! Plain maths, typed like a calculator, to LaTeX and back, so equations
//! don't need LaTeX: `(-b +- sqrt(b^2 - 4ac)) / (2a)`, `x^2 + y_1`,
//! `sum(i=1, n) i^2`, `alpha <= pi`.
//!
//! - `a/b` is a fraction; brackets around a part are dropped (`(a+b)/2`).
//! - `^` and `_` raise and lower the next part: `x^(n+1)`, `x_i`.
//! - `sqrt(x)`, `root(3, x)`, `abs(x)`, `sum(i=1, n)`, `prod(…)`,
//!   `int(a, b)`, `lim(x->0)`; `sin`, `cos`, `log`… stay upright.
//! - Greek letters by name (`alpha`, `Omega`), `inf`, `deg`.
//! - `+-` ±, `*` ×, `<=` ≤, `>=` ≥, `!=` ≠, `~=` ≈, `->` →.
//!
//! [`to_easy`] turns LaTeX back into this form when it can (LaTeX made
//! here always can), so an equation reopens the way it was typed.

const GREEK: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu", "xi",
    "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega", "Gamma", "Delta", "Theta", "Lambda", "Xi",
    "Pi", "Sigma", "Phi", "Psi", "Omega",
];
const UPRIGHT: &[&str] = &["sin", "cos", "tan", "log", "ln", "exp", "min", "max", "det"];
/// Typed operator → LaTeX (longest first).
const OPS: &[(&str, &str)] = &[
    ("+-", "\\pm"),
    ("-+", "\\mp"),
    ("<=", "\\le"),
    (">=", "\\ge"),
    ("!=", "\\ne"),
    ("~=", "\\approx"),
    ("->", "\\to"),
    ("=>", "\\Rightarrow"),
    ("*", "\\times"),
    ("±", "\\pm"),
    ("×", "\\times"),
    ("·", "\\cdot"),
    ("≤", "\\le"),
    ("≥", "\\ge"),
    ("≠", "\\ne"),
    ("≈", "\\approx"),
    ("→", "\\to"),
    ("+", "+"),
    ("-", "-"),
    ("=", "="),
    ("<", "<"),
    (">", ">"),
];
const WORDS: &[(&str, &str)] = &[("inf", "\\infty"), ("infinity", "\\infty"), ("∞", "\\infty"), ("deg", "^{\\circ}")];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(String),
    Word(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
    Pow,
    Sub,
    Div,
    Bang,
}

fn tokens(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    'outer: while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(chars[start..i].iter().collect()));
            continue;
        }
        if c.is_alphabetic() || c == '∞' {
            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            if i == start {
                i += 1;
            }
            out.push(Tok::Word(chars[start..i].iter().collect()));
            continue;
        }
        let single = match c {
            '(' | '[' | '{' => Some(Tok::Open),
            ')' | ']' | '}' => Some(Tok::Close),
            ',' => Some(Tok::Comma),
            '^' => Some(Tok::Pow),
            '_' => Some(Tok::Sub),
            '/' => Some(Tok::Div),
            '!' if chars.get(i + 1) != Some(&'=') => Some(Tok::Bang),
            _ => None,
        };
        if let Some(t) = single {
            out.push(t);
            i += 1;
            continue;
        }
        let rest: String = chars[i..].iter().take(2).collect();
        for (typed, _) in OPS {
            if rest.starts_with(typed) {
                out.push(Tok::Op(typed));
                i += typed.chars().count();
                continue 'outer;
            }
        }
        // Anything else is kept as it is (a symbol from the keyboard).
        out.push(Tok::Word(c.to_string()));
        i += 1;
    }
    out
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

/// A parsed part: its LaTeX, and whether it was a bracketed group (whose
/// brackets a fraction or power drops).
struct Part {
    tex: String,
    group: Option<String>,
}

impl Part {
    fn plain(tex: String) -> Self {
        Part { tex, group: None }
    }
    /// The LaTeX without the outer brackets, for a fraction or power.
    fn inner(&self) -> &str {
        self.group.as_deref().unwrap_or(&self.tex)
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.at).cloned();
        self.at += 1;
        t
    }

    /// Terms joined by operators, up to a closing bracket, comma or the end.
    fn sum(&mut self) -> String {
        let mut out: Vec<String> = Vec::new();
        while let Some(t) = self.peek() {
            match t {
                Tok::Close | Tok::Comma => break,
                Tok::Op(op) => {
                    let op = *op;
                    self.at += 1;
                    out.push(OPS.iter().find(|(t, _)| *t == op).map(|(_, l)| l.to_string()).unwrap_or_default());
                }
                _ => match self.fraction() {
                    Some(p) => out.push(p),
                    None => break,
                },
            }
        }
        out.join(" ")
    }

    /// Products divided by products: `2a/b`, `(a+b)/(c+d)`.
    fn fraction(&mut self) -> Option<String> {
        let mut top = self.product()?;
        while self.peek() == Some(&Tok::Div) {
            self.at += 1;
            let Some(bottom) = self.product() else { break };
            top = Part::plain(format!("\\frac{{{}}}{{{}}}", top.inner(), bottom.inner()));
        }
        Some(top.tex)
    }

    /// Parts written side by side: `2a`, `4ac`, `2 sqrt(x)`.
    fn product(&mut self) -> Option<Part> {
        let mut parts = vec![self.power()?];
        // A big operator (sum, limit…) applies to what follows; it isn't part of a fraction.
        if is_big(&parts[0].tex) {
            return parts.pop();
        }
        while matches!(self.peek(), Some(Tok::Num(_) | Tok::Word(_) | Tok::Open)) {
            if matches!(self.peek(), Some(Tok::Word(w)) if ["sum", "prod", "int", "lim"].contains(&w.as_str())) {
                break;
            }
            let Some(p) = self.power() else { break };
            parts.push(p);
        }
        Some(if parts.len() == 1 {
            parts.pop().expect("one")
        } else {
            Part::plain(parts.iter().map(|p| p.tex.as_str()).collect::<Vec<_>>().join(" "))
        })
    }

    /// A part with what's raised, lowered or factorial after it.
    fn power(&mut self) -> Option<Part> {
        let mut base = self.atom()?;
        loop {
            match self.peek() {
                Some(Tok::Pow) => {
                    self.at += 1;
                    let Some(e) = self.atom() else { break };
                    base = Part::plain(format!("{}^{{{}}}", base.tex, e.inner()));
                }
                Some(Tok::Sub) => {
                    self.at += 1;
                    let Some(e) = self.atom() else { break };
                    base = Part::plain(format!("{}_{{{}}}", base.tex, e.inner()));
                }
                Some(Tok::Bang) => {
                    self.at += 1;
                    base = Part::plain(format!("{}!", base.tex));
                }
                _ => break,
            }
        }
        Some(base)
    }

    /// The arguments of a function, `(a, b)`; empty when there are no brackets.
    fn args(&mut self) -> Vec<String> {
        if self.peek() != Some(&Tok::Open) {
            return Vec::new();
        }
        self.at += 1;
        let mut out = vec![self.sum()];
        while self.peek() == Some(&Tok::Comma) {
            self.at += 1;
            out.push(self.sum());
        }
        if self.peek() == Some(&Tok::Close) {
            self.at += 1;
        }
        out
    }

    fn atom(&mut self) -> Option<Part> {
        match self.next()? {
            Tok::Num(n) => Some(Part::plain(n)),
            Tok::Open => {
                let inner = self.sum();
                if self.peek() == Some(&Tok::Close) {
                    self.at += 1;
                }
                Some(Part { tex: format!("\\left( {inner} \\right)"), group: Some(inner) })
            }
            Tok::Op("-") => {
                let a = self.atom()?;
                Some(Part::plain(format!("-{}", a.tex)))
            }
            Tok::Word(w) => Some(Part::plain(self.word(&w))),
            Tok::Close | Tok::Comma | Tok::Pow | Tok::Sub | Tok::Div | Tok::Bang | Tok::Op(_) => {
                self.at -= 1;
                None
            }
        }
    }

    fn word(&mut self, w: &str) -> String {
        let limits = |name: &str, a: &[String]| match a {
            [] => format!("\\{name}"),
            [lo] => format!("\\{name}_{{{lo}}}"),
            [lo, hi, ..] => format!("\\{name}_{{{lo}}}^{{{hi}}}"),
        };
        match w {
            "sqrt" => {
                let a = self.args();
                format!("\\sqrt{{{}}}", a.first().map(String::as_str).unwrap_or(""))
            }
            "root" => match self.args().as_slice() {
                [n, x, ..] => format!("\\sqrt[{n}]{{{x}}}"),
                [x] => format!("\\sqrt{{{x}}}"),
                [] => "\\sqrt{}".into(),
            },
            "abs" => {
                let a = self.args();
                format!("\\left| {} \\right|", a.first().map(String::as_str).unwrap_or(""))
            }
            "sum" | "prod" | "int" => {
                let a = self.args();
                limits(w, &a)
            }
            "lim" => {
                let a = self.args();
                match a.first() {
                    Some(x) => format!("\\lim_{{{x}}}"),
                    None => "\\lim".into(),
                }
            }
            _ if GREEK.contains(&w) => format!("\\{w}"),
            _ if UPRIGHT.contains(&w) => format!("\\{w}"),
            _ => WORDS.iter().find(|(t, _)| *t == w).map(|(_, l)| l.to_string()).unwrap_or_else(|| w.to_string()),
        }
    }
}

/// Plain maths to LaTeX.
pub fn to_latex(easy: &str) -> String {
    let mut p = Parser { toks: tokens(easy), at: 0 };
    let mut out = Vec::new();
    while p.at < p.toks.len() {
        let s = p.sum();
        if !s.is_empty() {
            out.push(s);
        }
        // A stray closing bracket or comma: keep going after it.
        if p.at < p.toks.len() {
            match p.next() {
                Some(Tok::Comma) => out.push(",".into()),
                Some(Tok::Close) => out.push(")".into()),
                _ => {}
            }
        }
    }
    out.join(" ")
}

/// LaTeX back to plain maths, when it only uses what [`to_latex`] makes;
/// `None` otherwise (the equation is then edited as LaTeX).
pub fn to_easy(latex: &str) -> Option<String> {
    let easy = Reader { s: latex.chars().collect(), at: 0 }.seq(None)?;
    let easy = easy.split_whitespace().collect::<Vec<_>>().join(" ");
    (normal(&to_latex(&easy)) == normal(latex)).then_some(easy)
}

/// LaTeX with its spacing evened out, for comparing.
fn normal(latex: &str) -> String {
    latex.chars().filter(|c| !c.is_whitespace()).collect()
}

struct Reader {
    s: Vec<char>,
    at: usize,
}

impl Reader {
    fn skip_space(&mut self) {
        while self.s.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }
    fn eat(&mut self, c: char) -> bool {
        self.skip_space();
        if self.s.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn command(&mut self) -> Option<String> {
        self.skip_space();
        if self.s.get(self.at) != Some(&'\\') {
            return None;
        }
        let start = self.at + 1;
        let mut end = start;
        while self.s.get(end).is_some_and(|c| c.is_ascii_alphabetic()) {
            end += 1;
        }
        if end == start {
            end += 1; // \| \, and the like
        }
        let name: String = self.s.get(start..end)?.iter().collect();
        self.at = end;
        Some(name)
    }
    /// `{…}` as plain maths.
    fn group(&mut self) -> Option<String> {
        if !self.eat('{') {
            return None;
        }
        self.seq(Some('}'))
    }
    /// A group, bracketed when it has more than one part (for `/`, `^`, `_`).
    fn operand(&mut self) -> Option<String> {
        let g = self.group()?;
        Some(if simple(&g) { g } else { format!("({g})") })
    }

    /// Plain maths up to `end` (consumed).
    fn seq(&mut self, end: Option<char>) -> Option<String> {
        let mut out = String::new();
        loop {
            self.skip_space();
            let Some(&c) = self.s.get(self.at) else {
                return end.is_none().then_some(out);
            };
            if Some(c) == end {
                self.at += 1;
                return Some(out);
            }
            match c {
                '\\' => {
                    let save = self.at;
                    let name = self.command()?;
                    let piece = match name.as_str() {
                        "frac" => {
                            let a = self.operand()?;
                            let b = self.operand()?;
                            format!("{a}/{b}")
                        }
                        "sqrt" => {
                            if self.eat('[') {
                                let n = self.seq(Some(']'))?;
                                let x = self.group()?;
                                format!("root({n}, {x})")
                            } else {
                                format!("sqrt({})", self.group()?)
                            }
                        }
                        "left" => {
                            if self.eat('(') {
                                let inner = self.until_right(')')?;
                                format!("({inner})")
                            } else if self.eat('|') {
                                let inner = self.until_right('|')?;
                                format!("abs({inner})")
                            } else {
                                return None;
                            }
                        }
                        "sum" | "prod" | "int" | "lim" => {
                            let lo = if self.eat('_') { Some(self.group()?) } else { None };
                            let hi = if self.eat('^') { Some(self.group()?) } else { None };
                            match (lo, hi) {
                                (None, None) => name.clone(),
                                (Some(lo), None) => format!("{name}({lo})"),
                                (Some(lo), Some(hi)) => format!("{name}({lo}, {hi})"),
                                (None, Some(_)) => return None,
                            }
                        }
                        n if GREEK.contains(&n) || UPRIGHT.contains(&n) => n.to_string(),
                        "infty" => "inf".into(),
                        _ => {
                            match OPS.iter().find(|(t, l)| l.strip_prefix('\\') == Some(name.as_str()) && t.is_ascii())
                            {
                                Some((typed, _)) => format!(" {typed} "),
                                None => {
                                    self.at = save;
                                    return None;
                                }
                            }
                        }
                    };
                    // A word right after a word needs a space ("2 alpha", not "2alpha" → fine, but "sin x").
                    if out.chars().last().is_some_and(|c| c.is_alphanumeric())
                        && piece.chars().next().is_some_and(|c| c.is_alphabetic())
                    {
                        out.push(' ');
                    }
                    out.push_str(&piece);
                }
                '^' | '_' => {
                    self.at += 1;
                    // `^{\circ}` is "deg".
                    if c == '^' && self.s[self.at..].iter().collect::<String>().starts_with("{\\circ}") {
                        self.at += "{\\circ}".len();
                        out.push_str(" deg");
                        continue;
                    }
                    let g = self.operand()?;
                    out.push(c);
                    out.push_str(&g);
                }
                '{' | '}' => return None,
                _ => {
                    self.at += 1;
                    if "+-=<>".contains(c) {
                        out.push(' ');
                        out.push(c);
                        out.push(' ');
                    } else {
                        if c.is_alphanumeric()
                            && out.chars().last().is_some_and(|l| l.is_alphabetic())
                            && !self.prev_was_letter_run(&out)
                        {
                            out.push(' ');
                        }
                        out.push(c);
                    }
                }
            }
        }
    }

    /// Whether the end of `out` is a run of plain letters (so "ac" stays "ac").
    fn prev_was_letter_run(&self, out: &str) -> bool {
        let word: String = out.chars().rev().take_while(|c| c.is_alphabetic()).collect();
        let word: String = word.chars().rev().collect();
        !(GREEK.contains(&word.as_str())
            || UPRIGHT.contains(&word.as_str())
            || ["sqrt", "abs", "sum", "prod", "int", "lim", "inf", "deg"].contains(&word.as_str()))
    }

    /// Up to `\right<c>` (consumed).
    fn until_right(&mut self, c: char) -> Option<String> {
        let rest: String = self.s[self.at..].iter().collect();
        let marker = format!("\\right{c}");
        // The matching \right: count nested \left.
        let mut depth = 0usize;
        let mut i = 0;
        let bytes: Vec<char> = rest.chars().collect();
        while i < bytes.len() {
            let tail: String = bytes[i..].iter().take(7).collect();
            if tail.starts_with("\\left") {
                depth += 1;
                i += 5;
                continue;
            }
            if tail.starts_with("\\right") {
                if depth == 0 {
                    if !bytes[i..].iter().collect::<String>().starts_with(&marker) {
                        return None;
                    }
                    break;
                }
                depth -= 1;
                i += 6;
                continue;
            }
            i += 1;
        }
        let inner: String = bytes[..i].iter().collect();
        let easy = Reader { s: inner.chars().collect(), at: 0 }.seq(None)?;
        self.at += i + marker.chars().count();
        Some(easy.trim().to_string())
    }
}

fn is_big(tex: &str) -> bool {
    ["\\sum", "\\prod", "\\int", "\\lim"].iter().any(|b| tex.starts_with(b))
}

/// One part that needs no brackets after `/`, `^` or `_`: `2`, `x`, `ab`, `sqrt(x)`.
fn simple(easy: &str) -> bool {
    let e = easy.trim();
    !e.is_empty() && !e.contains([' ', '/', '^', '_']) && !e.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_maths_to_latex() {
        let cases = [
            ("x^2 + y_1", "x^{2} + y_{1}"),
            ("a/b", "\\frac{a}{b}"),
            ("(a+b)/2", "\\frac{a + b}{2}"),
            ("(-b +- sqrt(b^2 - 4ac)) / (2a)", "\\frac{- b \\pm \\sqrt{b^{2} - 4 ac}}{2 a}"),
            ("E = mc^2", "E = mc^{2}"),
            ("sum(i=1, n) i^2", "\\sum_{i = 1}^{n} i^{2}"),
            ("alpha <= pi", "\\alpha \\le \\pi"),
            ("root(3, x)", "\\sqrt[3]{x}"),
            ("x^(n+1)", "x^{n + 1}"),
            ("lim(x->0) sin(x)/x", "\\lim_{x \\to 0} \\frac{\\sin \\left( x \\right)}{x}"),
            ("2 * 3 != 5", "2 \\times 3 \\ne 5"),
            ("90 deg", "90 ^{\\circ}"),
        ];
        for (easy, want) in cases {
            assert_eq!(to_latex(easy), want, "{easy}");
        }
    }

    #[test]
    fn back_to_plain_maths() {
        for easy in [
            "x^2 + y_1",
            "a/b",
            "(a + b)/2",
            "(-b +- sqrt(b^2 - 4ac))/(2a)",
            "E = mc^2",
            "sum(i = 1, n) i^2",
            "alpha <= pi",
            "root(3, x)",
            "abs(x - 1)",
            "90 deg",
        ] {
            let latex = to_latex(easy);
            let back = to_easy(&latex).unwrap_or_else(|| panic!("{easy} → {latex} didn't come back"));
            assert_eq!(normal(&to_latex(&back)), normal(&latex), "{easy} → {latex} → {back}");
        }
        // LaTeX that plain maths can't say stays LaTeX.
        assert_eq!(to_easy("\\begin{matrix} a & b \\end{matrix}"), None);
        assert_eq!(to_easy("\\mathbb{R}"), None);
    }
}
