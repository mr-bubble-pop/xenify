//! LaTeX (the maths part) to a [`Node`] tree. Lenient: an unknown command
//! is kept as its name (shown in red), so typing never fails.

/// How an atom spaces itself from its neighbours (TeX's classes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Ord,
    /// Big operator (∑, ∫) or a function name (sin).
    Op,
    /// Binary operator (+, ×).
    Bin,
    /// Relation (=, <, →).
    Rel,
    Open,
    Close,
    Punct,
}

/// Letter styles (`\mathrm`, `\mathbf`, `\mathbb`…).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Variant {
    /// Letters italic, digits upright (TeX's default).
    #[default]
    Normal,
    Upright,
    Bold,
    Blackboard,
    Calligraphic,
    Fraktur,
    Mono,
    Sans,
}

/// Matrix-like environments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Env {
    Matrix,
    /// `pmatrix` ( ), `bmatrix` [ ], `vmatrix` | |, `Bmatrix` { }.
    Delimited(char, char),
    /// `cases`: a left brace, rows left-aligned.
    Cases,
    /// `aligned`: rows lined up at `&`.
    Aligned,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// One symbol: a letter, digit, operator…
    Sym {
        ch: char,
        class: Class,
        variant: Variant,
    },
    /// Upright text (`\text{}` or a function name like `sin`).
    Text {
        text: String,
        class: Class,
    },
    Row(Vec<Node>),
    Frac {
        num: Box<Node>,
        den: Box<Node>,
        bar: bool,
        style: Option<bool>,
    },
    /// Base with scripts.
    Scripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    Sqrt {
        index: Option<Box<Node>>,
        body: Box<Node>,
    },
    /// `\sum`, `\int`… (`limits`: scripts above and below in display style).
    BigOp {
        ch: char,
        limits: bool,
    },
    /// `\left( … \right)`: delimiters as tall as what's inside.
    Fenced {
        open: Option<char>,
        close: Option<char>,
        body: Box<Node>,
    },
    /// `\hat`, `\vec`…: an accent character over the body.
    Accent {
        ch: char,
        body: Box<Node>,
        wide: bool,
    },
    Over {
        body: Box<Node>,
    },
    Under {
        body: Box<Node>,
    },
    /// Space in mu (1/18 em; negative for `\!`).
    Space(f32),
    Matrix {
        env: Env,
        rows: Vec<Vec<Node>>,
    },
    /// Something not understood (shown as is, in red).
    Error(String),
}

impl Node {
    pub fn row(v: Vec<Node>) -> Node {
        if v.len() == 1 { v.into_iter().next().unwrap() } else { Node::Row(v) }
    }
}

fn sym(ch: char, class: Class) -> Node {
    Node::Sym { ch, class, variant: Variant::Normal }
}

/// Named symbols: `\alpha` → α, `\le` → ≤ (with their spacing class).
pub fn symbol(name: &str) -> Option<(char, Class)> {
    use Class::*;
    Some(match name {
        "alpha" => ('α', Ord),
        "beta" => ('β', Ord),
        "gamma" => ('γ', Ord),
        "delta" => ('δ', Ord),
        "epsilon" => ('ϵ', Ord),
        "varepsilon" => ('ε', Ord),
        "zeta" => ('ζ', Ord),
        "eta" => ('η', Ord),
        "theta" => ('θ', Ord),
        "vartheta" => ('ϑ', Ord),
        "iota" => ('ι', Ord),
        "kappa" => ('κ', Ord),
        "lambda" => ('λ', Ord),
        "mu" => ('μ', Ord),
        "nu" => ('ν', Ord),
        "xi" => ('ξ', Ord),
        "pi" => ('π', Ord),
        "varpi" => ('ϖ', Ord),
        "rho" => ('ρ', Ord),
        "varrho" => ('ϱ', Ord),
        "sigma" => ('σ', Ord),
        "varsigma" => ('ς', Ord),
        "tau" => ('τ', Ord),
        "upsilon" => ('υ', Ord),
        "phi" => ('ϕ', Ord),
        "varphi" => ('φ', Ord),
        "chi" => ('χ', Ord),
        "psi" => ('ψ', Ord),
        "omega" => ('ω', Ord),
        "Gamma" => ('Γ', Ord),
        "Delta" => ('Δ', Ord),
        "Theta" => ('Θ', Ord),
        "Lambda" => ('Λ', Ord),
        "Xi" => ('Ξ', Ord),
        "Pi" => ('Π', Ord),
        "Sigma" => ('Σ', Ord),
        "Upsilon" => ('Υ', Ord),
        "Phi" => ('Φ', Ord),
        "Psi" => ('Ψ', Ord),
        "Omega" => ('Ω', Ord),
        "infty" => ('∞', Ord),
        "partial" => ('∂', Ord),
        "nabla" => ('∇', Ord),
        "emptyset" | "varnothing" => ('∅', Ord),
        "forall" => ('∀', Ord),
        "exists" => ('∃', Ord),
        "neg" | "lnot" => ('¬', Ord),
        "hbar" => ('ℏ', Ord),
        "ell" => ('ℓ', Ord),
        "Re" => ('ℜ', Ord),
        "Im" => ('ℑ', Ord),
        "aleph" => ('ℵ', Ord),
        "angle" => ('∠', Ord),
        "triangle" => ('△', Ord),
        "prime" => ('′', Ord),
        "degree" => ('°', Ord),
        "ldots" | "dots" => ('…', Ord),
        "cdots" => ('⋯', Ord),
        "vdots" => ('⋮', Ord),
        "ddots" => ('⋱', Ord),
        "pm" => ('±', Bin),
        "mp" => ('∓', Bin),
        "times" => ('×', Bin),
        "div" => ('÷', Bin),
        "cdot" => ('⋅', Bin),
        "ast" => ('∗', Bin),
        "star" => ('⋆', Bin),
        "circ" => ('∘', Bin),
        "bullet" => ('∙', Bin),
        "oplus" => ('⊕', Bin),
        "otimes" => ('⊗', Bin),
        "cup" => ('∪', Bin),
        "cap" => ('∩', Bin),
        "wedge" | "land" => ('∧', Bin),
        "vee" | "lor" => ('∨', Bin),
        "setminus" => ('∖', Bin),
        "le" | "leq" => ('≤', Rel),
        "ge" | "geq" => ('≥', Rel),
        "ne" | "neq" => ('≠', Rel),
        "approx" => ('≈', Rel),
        "equiv" => ('≡', Rel),
        "sim" => ('∼', Rel),
        "simeq" => ('≃', Rel),
        "cong" => ('≅', Rel),
        "propto" => ('∝', Rel),
        "ll" => ('≪', Rel),
        "gg" => ('≫', Rel),
        "in" => ('∈', Rel),
        "notin" => ('∉', Rel),
        "ni" => ('∋', Rel),
        "subset" => ('⊂', Rel),
        "supset" => ('⊃', Rel),
        "subseteq" => ('⊆', Rel),
        "supseteq" => ('⊇', Rel),
        "perp" => ('⊥', Rel),
        "parallel" => ('∥', Rel),
        "mid" => ('∣', Rel),
        "to" | "rightarrow" => ('→', Rel),
        "leftarrow" | "gets" => ('←', Rel),
        "leftrightarrow" => ('↔', Rel),
        "Rightarrow" | "implies" => ('⇒', Rel),
        "Leftarrow" => ('⇐', Rel),
        "Leftrightarrow" | "iff" => ('⇔', Rel),
        "mapsto" => ('↦', Rel),
        "uparrow" => ('↑', Rel),
        "downarrow" => ('↓', Rel),
        "colon" => (':', Punct),
        _ => return None,
    })
}

pub(crate) fn big_op(name: &str) -> Option<(char, bool)> {
    Some(match name {
        "sum" => ('∑', true),
        "prod" => ('∏', true),
        "coprod" => ('∐', true),
        "bigcup" => ('⋃', true),
        "bigcap" => ('⋂', true),
        "bigoplus" => ('⨁', true),
        "bigotimes" => ('⨂', true),
        "bigvee" => ('⋁', true),
        "bigwedge" => ('⋀', true),
        "int" => ('∫', false),
        "iint" => ('∬', false),
        "iiint" => ('∭', false),
        "oint" => ('∮', false),
        _ => return None,
    })
}

pub(crate) const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log",
    "ln", "lg", "exp", "det", "dim", "gcd", "deg", "arg", "ker", "hom", "Pr",
];
/// Functions whose scripts go above and below in display style.
pub(crate) const LIMIT_FUNCTIONS: &[&str] = &["lim", "limsup", "liminf", "max", "min", "sup", "inf"];

pub(crate) fn accent(name: &str) -> Option<(char, bool)> {
    Some(match name {
        "hat" => ('\u{302}', false),
        "widehat" => ('\u{302}', true),
        "check" => ('\u{30C}', false),
        "tilde" => ('\u{303}', false),
        "widetilde" => ('\u{303}', true),
        "acute" => ('\u{301}', false),
        "grave" => ('\u{300}', false),
        "dot" => ('\u{307}', false),
        "ddot" => ('\u{308}', false),
        "breve" => ('\u{306}', false),
        "bar" => ('\u{304}', false),
        "vec" => ('\u{20D7}', false),
        _ => return None,
    })
}

fn delimiter(name: &str) -> Option<Option<char>> {
    Some(Some(match name {
        "(" => '(',
        ")" => ')',
        "[" => '[',
        "]" => ']',
        "\\{" | "\\lbrace" => '{',
        "\\}" | "\\rbrace" => '}',
        "|" | "\\vert" => '|',
        "\\|" | "\\Vert" => '‖',
        "\\langle" => '⟨',
        "\\rangle" => '⟩',
        "\\lfloor" => '⌊',
        "\\rfloor" => '⌋',
        "\\lceil" => '⌈',
        "\\rceil" => '⌉',
        "/" => '/',
        "." => return Some(None),
        _ => return None,
    }))
}

struct Parser<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.s[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// A command name after `\`: letters, or one other character.
    fn command(&mut self) -> String {
        let start = self.at;
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.bump();
            }
        } else {
            self.bump();
        }
        self.s[start..self.at].to_string()
    }

    /// `{…}` as a raw string (for `\text`, `\begin{…}`).
    fn raw_group(&mut self) -> String {
        self.skip_space();
        if self.peek() != Some('{') {
            return self.bump().map(String::from).unwrap_or_default();
        }
        self.bump();
        let start = self.at;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '\\' => {
                    self.bump();
                }
                _ => {}
            }
            self.bump();
        }
        let out = self.s[start..self.at].to_string();
        self.bump();
        out
    }

    /// One argument: a `{group}` or a single atom.
    fn argument(&mut self, variant: Variant) -> Node {
        self.skip_space();
        match self.peek() {
            Some('{') => {
                self.bump();
                let r = self.list(variant, &['}']);
                self.bump();
                r
            }
            Some(_) => self.atom(variant).unwrap_or(Node::Row(Vec::new())),
            None => Node::Row(Vec::new()),
        }
    }

    /// An optional `[…]` argument.
    fn optional(&mut self, variant: Variant) -> Option<Node> {
        self.skip_space();
        if self.peek() != Some('[') {
            return None;
        }
        self.bump();
        let r = self.list(variant, &[']']);
        self.bump();
        Some(r)
    }

    /// A delimiter after `\left`, `\right`, `\big`…
    fn delimiter(&mut self) -> Option<char> {
        self.skip_space();
        let start = self.at;
        let tok = if self.peek() == Some('\\') {
            self.bump();
            format!("\\{}", self.command())
        } else {
            self.bump().map(String::from).unwrap_or_default()
        };
        match delimiter(&tok) {
            Some(d) => d,
            None => {
                self.at = start;
                None
            }
        }
    }

    /// Atoms until one of `end` (not consumed) or the end of input.
    fn list(&mut self, variant: Variant, end: &[char]) -> Node {
        let mut out: Vec<Node> = Vec::new();
        loop {
            self.skip_space();
            let Some(c) = self.peek() else { break };
            if end.contains(&c) {
                break;
            }
            match c {
                '^' | '_' => {
                    self.bump();
                    let arg = self.argument(variant);
                    let base = out.pop().unwrap_or(Node::Row(Vec::new()));
                    out.push(attach(base, c == '^', arg));
                }
                '\'' => {
                    self.bump();
                    let base = out.pop().unwrap_or(Node::Row(Vec::new()));
                    out.push(attach(base, true, sym('′', Class::Ord)));
                }
                '&' => {
                    // Column breaks are handled by environments; a stray one is ignored.
                    self.bump();
                }
                _ => {
                    if let Some(n) = self.atom(variant) {
                        out.push(n);
                    } else {
                        break;
                    }
                }
            }
        }
        Node::row(out)
    }

    fn atom(&mut self, variant: Variant) -> Option<Node> {
        let c = self.peek()?;
        match c {
            '{' => {
                self.bump();
                let r = self.list(variant, &['}']);
                self.bump();
                Some(r)
            }
            '\\' => {
                self.bump();
                let name = self.command();
                Some(self.command_node(&name, variant))
            }
            '}' | ']' => {
                self.bump();
                Some(Node::Error(c.to_string()))
            }
            _ => {
                self.bump();
                use Class::*;
                let class = match c {
                    '+' | '-' | '−' | '*' | '×' | '·' | '±' => Bin,
                    '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '→' | ':' => Rel,
                    '(' | '[' => Open,
                    ')' | ']' | '!' | '?' => Close,
                    ',' | ';' => Punct,
                    _ => Ord,
                };
                let ch = if c == '-' {
                    '−'
                } else if c == '*' {
                    '∗'
                } else {
                    c
                };
                Some(Node::Sym { ch, class, variant })
            }
        }
    }

    fn command_node(&mut self, name: &str, variant: Variant) -> Node {
        if let Some((ch, class)) = symbol(name) {
            return Node::Sym { ch, class, variant: if class == Class::Ord { variant } else { Variant::Normal } };
        }
        if let Some((ch, limits)) = big_op(name) {
            return Node::BigOp { ch, limits };
        }
        if FUNCTIONS.contains(&name) {
            return Node::Text { text: name.into(), class: Class::Op };
        }
        if LIMIT_FUNCTIONS.contains(&name) {
            // Shown as text; scripts go under it (see layout).
            return Node::Text { text: name.into(), class: Class::Op };
        }
        if let Some((ch, wide)) = accent(name) {
            let body = self.argument(variant);
            return Node::Accent { ch, body: Box::new(body), wide };
        }
        let v = |name: &str| match name {
            "mathrm" | "rm" | "operatorname" => Some(Variant::Upright),
            "mathbf" | "boldsymbol" | "bm" => Some(Variant::Bold),
            "mathbb" => Some(Variant::Blackboard),
            "mathcal" => Some(Variant::Calligraphic),
            "mathfrak" => Some(Variant::Fraktur),
            "mathtt" => Some(Variant::Mono),
            "mathsf" => Some(Variant::Sans),
            "mathit" => Some(Variant::Normal),
            _ => None,
        };
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.argument(variant);
                let den = self.argument(variant);
                let style = match name {
                    "dfrac" | "cfrac" => Some(true),
                    "tfrac" => Some(false),
                    _ => None,
                };
                Node::Frac { num: Box::new(num), den: Box::new(den), bar: true, style }
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.argument(variant);
                let den = self.argument(variant);
                Node::Fenced {
                    open: Some('('),
                    close: Some(')'),
                    body: Box::new(Node::Frac { num: Box::new(num), den: Box::new(den), bar: false, style: None }),
                }
            }
            "sqrt" => {
                let index = self.optional(variant).map(Box::new);
                let body = self.argument(variant);
                Node::Sqrt { index, body: Box::new(body) }
            }
            "left" => {
                let open = self.delimiter();
                let body = self.list_until_right(variant);
                let close = self.delimiter();
                Node::Fenced { open, close, body: Box::new(body) }
            }
            "right" => Node::Error("\\right".into()),
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" => {
                let d = self.delimiter();
                d.map_or(Node::Row(Vec::new()), |c| sym(c, Class::Ord))
            }
            "overline" => Node::Over { body: Box::new(self.argument(variant)) },
            "underline" => Node::Under { body: Box::new(self.argument(variant)) },
            "text" | "textrm" | "mbox" | "textit" | "textbf" => {
                Node::Text { text: self.raw_group(), class: Class::Ord }
            }
            "operatorname" => Node::Text { text: self.raw_group(), class: Class::Op },
            "," | "thinspace" => Node::Space(3.0),
            ":" | ">" | "medspace" => Node::Space(4.0),
            ";" | "thickspace" => Node::Space(5.0),
            "!" => Node::Space(-3.0),
            "quad" => Node::Space(18.0),
            "qquad" => Node::Space(36.0),
            " " => Node::Space(6.0),
            "{" => sym('{', Class::Open),
            "}" => sym('}', Class::Close),
            "|" => sym('‖', Class::Ord),
            "%" | "#" | "$" | "&" | "_" => sym(name.chars().next().unwrap_or('?'), Class::Ord),
            "\\" => Node::Error("\\\\".into()),
            "begin" => self.environment(variant),
            "end" => {
                self.raw_group();
                Node::Row(Vec::new())
            }
            "displaystyle" | "textstyle" | "limits" | "nolimits" => Node::Row(Vec::new()),
            _ => match v(name) {
                Some(var) => self.argument(var),
                None => Node::Error(format!("\\{name}")),
            },
        }
    }

    fn list_until_right(&mut self, variant: Variant) -> Node {
        let mut out = Vec::new();
        loop {
            self.skip_space();
            if self.peek().is_none() {
                break;
            }
            if self.s[self.at..].starts_with("\\right") {
                self.at += "\\right".len();
                break;
            }
            let part = self.list(variant, &['\\']);
            out.push(part);
            if self.s[self.at..].starts_with("\\right") {
                continue;
            }
            // Another command: take it as an atom.
            if let Some(n) = self.atom(variant) {
                out.push(n);
            } else {
                break;
            }
        }
        Node::row(flatten(out))
    }

    fn environment(&mut self, variant: Variant) -> Node {
        let name = self.raw_group();
        let env = match name.as_str() {
            "pmatrix" => Env::Delimited('(', ')'),
            "bmatrix" => Env::Delimited('[', ']'),
            "Bmatrix" => Env::Delimited('{', '}'),
            "vmatrix" => Env::Delimited('|', '|'),
            "Vmatrix" => Env::Delimited('‖', '‖'),
            "cases" => Env::Cases,
            "aligned" | "align" | "align*" | "split" | "gathered" => Env::Aligned,
            _ => Env::Matrix,
        };
        let end = format!("\\end{{{name}}}");
        let mut rows: Vec<Vec<Node>> = vec![Vec::new()];
        let mut cell: Vec<Node> = Vec::new();
        loop {
            self.skip_space();
            if self.peek().is_none() {
                break;
            }
            let rest = &self.s[self.at..];
            if rest.starts_with(&end) {
                self.at += end.len();
                break;
            }
            if rest.starts_with("\\\\") {
                self.at += 2;
                rows.last_mut().expect("a row").push(Node::row(std::mem::take(&mut cell)));
                rows.push(Vec::new());
                continue;
            }
            if rest.starts_with('&') {
                self.at += 1;
                rows.last_mut().expect("a row").push(Node::row(std::mem::take(&mut cell)));
                continue;
            }
            let part = self.list(variant, &['&', '\\']);
            if part != Node::Row(Vec::new()) {
                cell.push(part);
            }
            let rest = &self.s[self.at..];
            if rest.starts_with('\\')
                && !rest.starts_with("\\\\")
                && !rest.starts_with(&end)
                && let Some(n) = self.atom(variant)
            {
                cell.push(n);
            }
        }
        if !cell.is_empty() {
            rows.last_mut().expect("a row").push(Node::row(cell));
        }
        rows.retain(|r| !r.is_empty());
        Node::Matrix {
            env,
            rows: rows.into_iter().map(|r| r.into_iter().map(|c| Node::row(flatten(vec![c]))).collect()).collect(),
        }
    }
}

/// Rows inside rows, made one.
fn flatten(v: Vec<Node>) -> Vec<Node> {
    let mut out = Vec::new();
    for n in v {
        match n {
            Node::Row(inner) => out.extend(inner),
            n => out.push(n),
        }
    }
    out
}

/// Adds a super- or subscript to `base` (joining `x_1^2`).
fn attach(base: Node, sup: bool, arg: Node) -> Node {
    match base {
        Node::Scripts { base, sub, sup: s } if (sup && s.is_none()) || (!sup && sub.is_none()) => {
            if sup {
                Node::Scripts { base, sub, sup: Some(Box::new(arg)) }
            } else {
                Node::Scripts { base, sub: Some(Box::new(arg)), sup: s }
            }
        }
        base => {
            let (sub, s) = if sup { (None, Some(Box::new(arg))) } else { (Some(Box::new(arg)), None) };
            Node::Scripts { base: Box::new(base), sub, sup: s }
        }
    }
}

/// Parses LaTeX maths (without `$`).
pub fn parse(latex: &str) -> Node {
    let mut p = Parser { s: latex, at: 0 };
    let mut out = Vec::new();
    while p.at < latex.len() {
        let n = p.list(Variant::Normal, &['}', ']']);
        out.push(n);
        if p.at < latex.len() {
            // A stray closing bracket: shown, then go on.
            let c = p.bump().unwrap_or(' ');
            out.push(Node::Error(c.to_string()));
        }
    }
    Node::row(flatten(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_maths() {
        let n = parse("\\frac{a}{b}");
        assert!(matches!(n, Node::Frac { .. }));
        let n = parse("x^2 + y_i^{n+1}");
        let Node::Row(parts) = n else { panic!() };
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[2], Node::Scripts { sub: Some(_), sup: Some(_), .. }));
        assert!(matches!(parse("\\sqrt[3]{x}"), Node::Sqrt { index: Some(_), .. }));
        assert!(matches!(parse("\\left( x \\right)"), Node::Fenced { open: Some('('), close: Some(')'), .. }));
        let Node::Matrix { rows, .. } = parse("\\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix}") else { panic!() };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
        assert!(matches!(parse("\\alpha"), Node::Sym { ch: 'α', .. }));
        assert!(matches!(parse("\\nosuch"), Node::Error(_)));
        // Unbalanced input still parses.
        let _ = parse("\\frac{a}{");
        let _ = parse("}x{");
    }
}
