//! Visual equation editing: a maths tree with a caret that sits in rows —
//! the equation itself and the boxes of fractions, roots, scripts… Typing
//! fills the box the caret is in; `/` makes a fraction of what's before it,
//! `^` and `_` a superscript or subscript, `(` brackets that grow, and a
//! `\name` command any structure or symbol. Arrows, Tab and clicks move
//! between boxes. The tree is saved as LaTeX ([`MathEdit::latex`]).
//!
//! Every box is a [`Node::Row`] here (see [`normalize`]), so the caret is
//! always "before item `idx` of some row".

use crate::latex::to_latex;
use crate::layout::{MathBox, Piece};
use crate::parse::{Class, Env, Node, big_op, parse};

/// Where the caret is: the row reached by `path` (at each level the item
/// in the row, then which of its boxes), before item `idx`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub path: Vec<(usize, usize)>,
    pub idx: usize,
}

pub struct MathEdit {
    /// Always a [`Node::Row`].
    pub root: Node,
    pub cursor: Cursor,
}

/// A box as a row (one thing becomes a row of one).
fn slot(n: Node) -> Node {
    match normalize(n) {
        Node::Row(v) => Node::Row(v),
        other => Node::Row(vec![other]),
    }
}

/// Rows inside rows are spliced in.
fn splice(v: Vec<Node>) -> Vec<Node> {
    let mut out = Vec::new();
    for n in v {
        match normalize(n) {
            Node::Row(inner) => out.extend(inner),
            n => out.push(n),
        }
    }
    out
}

/// Makes every box of `n` a row, for editing. (A script's base stays as it
/// is: `x` in `x^2` isn't a box of its own.)
pub fn normalize(n: Node) -> Node {
    match n {
        Node::Row(v) => Node::Row(splice(v)),
        Node::Frac { num, den, bar, style } => {
            Node::Frac { num: Box::new(slot(*num)), den: Box::new(slot(*den)), bar, style }
        }
        Node::Scripts { base, sub, sup } => Node::Scripts {
            base: Box::new(normalize(*base)),
            sub: sub.map(|s| Box::new(slot(*s))),
            sup: sup.map(|s| Box::new(slot(*s))),
        },
        Node::Sqrt { index, body } => {
            Node::Sqrt { index: index.map(|i| Box::new(slot(*i))), body: Box::new(slot(*body)) }
        }
        Node::Fenced { open, close, body } => Node::Fenced { open, close, body: Box::new(slot(*body)) },
        Node::Accent { ch, body, wide } => Node::Accent { ch, body: Box::new(slot(*body)), wide },
        Node::Over { body } => Node::Over { body: Box::new(slot(*body)) },
        Node::Under { body } => Node::Under { body: Box::new(slot(*body)) },
        Node::Matrix { env, rows } => {
            Node::Matrix { env, rows: rows.into_iter().map(|r| r.into_iter().map(slot).collect()).collect() }
        }
        n => n,
    }
}

/// The boxes of `n`, in order (Tab order).
fn slots(n: &Node) -> Vec<&Node> {
    match n {
        Node::Frac { num, den, .. } => vec![num, den],
        Node::Scripts { base, sub, sup } => {
            let mut v: Vec<&Node> = Vec::new();
            if matches!(**base, Node::Row(_)) {
                v.push(base);
            }
            v.extend(sub.as_deref());
            v.extend(sup.as_deref());
            v
        }
        Node::Sqrt { index, body } => index.as_deref().into_iter().chain([&**body]).collect(),
        Node::Fenced { body, .. } | Node::Accent { body, .. } | Node::Over { body } | Node::Under { body } => {
            vec![body]
        }
        Node::Matrix { rows, .. } => rows.iter().flatten().collect(),
        _ => Vec::new(),
    }
}

fn slots_mut(n: &mut Node) -> Vec<&mut Node> {
    match n {
        Node::Frac { num, den, .. } => vec![num, den],
        Node::Scripts { base, sub, sup } => {
            let mut v: Vec<&mut Node> = Vec::new();
            if matches!(**base, Node::Row(_)) {
                v.push(base);
            }
            v.extend(sub.as_deref_mut());
            v.extend(sup.as_deref_mut());
            v
        }
        Node::Sqrt { index, body } => index.as_deref_mut().into_iter().chain([&mut **body]).collect(),
        Node::Fenced { body, .. } | Node::Accent { body, .. } | Node::Over { body } | Node::Under { body } => {
            vec![body]
        }
        Node::Matrix { rows, .. } => rows.iter_mut().flatten().collect(),
        _ => Vec::new(),
    }
}

fn items(n: &Node) -> &[Node] {
    match n {
        Node::Row(v) => v,
        _ => &[],
    }
}

fn items_mut(n: &mut Node) -> &mut Vec<Node> {
    if !matches!(n, Node::Row(_)) {
        *n = Node::Row(vec![std::mem::replace(n, Node::Row(Vec::new()))]);
    }
    match n {
        Node::Row(v) => v,
        _ => unreachable!("made a row above"),
    }
}

/// Whether every box of `n` is empty.
fn all_empty(n: &Node) -> bool {
    slots(n).iter().all(|s| items(s).is_empty())
}

/// What's left of `n` when its boxes go: a script's base, else nothing.
fn residue(n: Node) -> Vec<Node> {
    match n {
        Node::Scripts { base, .. } => match *base {
            Node::Row(v) => v,
            b => vec![b],
        },
        _ => Vec::new(),
    }
}

/// `n` taken apart: a script's base, then what was in its boxes.
fn unwrap(n: Node) -> Vec<Node> {
    let mut n = n;
    let mut out = Vec::new();
    if let Node::Scripts { base, .. } = &n
        && !matches!(**base, Node::Row(_))
    {
        out.push((**base).clone());
    }
    for s in slots_mut(&mut n) {
        out.append(items_mut(s));
    }
    out
}

/// The spacing class of an item (for what `/` takes as the numerator).
fn class(n: &Node) -> Class {
    match n {
        Node::Sym { class, .. } | Node::Text { class, .. } => *class,
        Node::BigOp { .. } => Class::Op,
        Node::Scripts { base, .. } => class(base),
        _ => Class::Ord,
    }
}

/// The first empty box inside `n` (depth first), as a path below it.
fn first_empty(n: &Node) -> Option<Vec<(usize, usize)>> {
    for (k, s) in slots(n).into_iter().enumerate() {
        let v = items(s);
        if v.is_empty() {
            return Some(vec![(usize::MAX, k)]);
        }
        for (i, it) in v.iter().enumerate() {
            if let Some(mut p) = first_empty(it) {
                p.insert(0, (usize::MAX, k));
                p[1].0 = i;
                return Some(p);
            }
        }
    }
    None
}

impl MathEdit {
    pub fn new(latex: &str) -> Self {
        let root = slot(parse(latex));
        let idx = items(&root).len();
        Self { root, cursor: Cursor { path: Vec::new(), idx } }
    }

    /// The equation as LaTeX.
    pub fn latex(&self) -> String {
        to_latex(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        items(&self.root).is_empty()
    }

    fn row_at(&self, path: &[(usize, usize)]) -> &Node {
        let mut n = &self.root;
        for &(i, k) in path {
            n = slots(&items(n)[i])[k];
        }
        n
    }

    fn row_mut(&mut self, path: &[(usize, usize)]) -> &mut Vec<Node> {
        let mut n = &mut self.root;
        for &(i, k) in path {
            let it = &mut items_mut(n)[i];
            n = slots_mut(it).into_iter().nth(k).expect("a box");
        }
        items_mut(n)
    }

    fn row(&self) -> &[Node] {
        items(self.row_at(&self.cursor.path))
    }

    /// The address of the row the caret is in (as [`Piece::Mark::row`]).
    fn row_addr(&self) -> usize {
        self.row_at(&self.cursor.path) as *const Node as usize
    }

    /// Puts `nodes` at the caret; the caret goes into the first empty box
    /// among them, else after them.
    pub fn insert_nodes(&mut self, nodes: Vec<Node>) {
        let nodes: Vec<Node> = nodes.into_iter().map(normalize).collect();
        let at = self.cursor.idx;
        let n = nodes.len();
        let row = self.row_mut(&self.cursor.path.clone());
        row.splice(at..at, nodes);
        for i in at..at + n {
            if let Some(mut p) = first_empty(&self.row()[i]) {
                p[0].0 = i;
                self.cursor.path.extend(p);
                self.cursor.idx = 0;
                return;
            }
        }
        self.cursor.idx = at + n;
    }

    /// Inserts LaTeX (a palette button, a paste): `^{}` and `_{}` attach to
    /// what's before the caret.
    pub fn insert_latex(&mut self, latex: &str) {
        match latex.trim() {
            "^{}" | "^" => self.script(true),
            "_{}" | "_" => self.script(false),
            s => {
                let nodes = match slot(parse(s)) {
                    Node::Row(v) => v,
                    n => vec![n],
                };
                self.insert_nodes(nodes);
            }
        }
    }

    /// A `\name` command typed in: a structure with its boxes, or a symbol.
    pub fn command(&mut self, name: &str) {
        let empty = || Box::new(Node::Row(Vec::new()));
        let node = match name {
            "frac" | "dfrac" => Node::Frac { num: empty(), den: empty(), bar: true, style: None },
            "binom" => Node::Fenced {
                open: Some('('),
                close: Some(')'),
                body: Box::new(Node::Frac { num: empty(), den: empty(), bar: false, style: None }),
            },
            "sqrt" => Node::Sqrt { index: None, body: empty() },
            "root" | "nthroot" => Node::Sqrt { index: Some(empty()), body: empty() },
            "lim" => Node::Scripts {
                base: Box::new(Node::Text { text: "lim".into(), class: Class::Op }),
                sub: Some(empty()),
                sup: None,
            },
            n if big_op(n).is_some() => {
                let (ch, limits) = big_op(n).expect("checked");
                Node::Scripts { base: Box::new(Node::BigOp { ch, limits }), sub: Some(empty()), sup: Some(empty()) }
            }
            "matrix" | "pmatrix" | "bmatrix" | "vmatrix" | "cases" => {
                let env = match name {
                    "pmatrix" => Env::Delimited('(', ')'),
                    "bmatrix" => Env::Delimited('[', ']'),
                    "vmatrix" => Env::Delimited('|', '|'),
                    "cases" => Env::Cases,
                    _ => Env::Matrix,
                };
                let cell = || Node::Row(Vec::new());
                Node::Matrix { env, rows: vec![vec![cell(), cell()], vec![cell(), cell()]] }
            }
            _ => {
                // Accents and the rest: as LaTeX with empty arguments where they take one.
                let takes = [
                    "hat",
                    "widehat",
                    "check",
                    "tilde",
                    "widetilde",
                    "acute",
                    "grave",
                    "dot",
                    "ddot",
                    "breve",
                    "bar",
                    "vec",
                    "overline",
                    "underline",
                    "mathbf",
                    "mathbb",
                    "mathcal",
                    "mathrm",
                    "text",
                ];
                let s = if takes.contains(&name) { format!("\\{name}{{}}") } else { format!("\\{name}") };
                return self.insert_latex(&s);
            }
        };
        self.insert_nodes(vec![node]);
    }

    /// Types one character.
    pub fn type_char(&mut self, c: char) {
        match c {
            '/' => self.fraction(),
            '^' => self.script(true),
            '_' => self.script(false),
            '(' | '[' => {
                let close = if c == '(' { ')' } else { ']' };
                self.insert_nodes(vec![Node::Fenced {
                    open: Some(c),
                    close: Some(close),
                    body: Box::new(Node::Row(Vec::new())),
                }]);
            }
            ')' | ']' if self.at_end_of_fence(c) => self.right(),
            ' ' => {
                // Out of the box, past what it belongs to (x^2 then space: on the line again).
                if !self.cursor.path.is_empty() {
                    let (i, _) = self.cursor.path.pop().expect("not empty");
                    self.cursor.idx = i + 1;
                }
            }
            c if c.is_control() => {}
            c => {
                let nodes = match parse(&c.to_string()) {
                    Node::Row(v) => v,
                    n => vec![n],
                };
                self.insert_nodes(nodes);
            }
        }
    }

    /// Whether the caret is at the end of brackets closed by `close`.
    fn at_end_of_fence(&self, close: char) -> bool {
        let Some((&(i, _), parent)) = self.cursor.path.split_last() else { return false };
        self.cursor.idx == self.row().len()
            && matches!(&items(self.row_at(parent))[i], Node::Fenced { close: Some(c), .. } if *c == close)
    }

    /// `/`: what's just before the caret (back to an operator) becomes the
    /// numerator; the caret goes to the denominator.
    fn fraction(&mut self) {
        let at = self.cursor.idx;
        let row = self.row();
        let mut from = at;
        while from > 0
            && !matches!(class(&row[from - 1]), Class::Bin | Class::Rel | Class::Punct | Class::Open | Class::Op)
        {
            from -= 1;
        }
        let path = self.cursor.path.clone();
        let num: Vec<Node> = self.row_mut(&path).drain(from..at).collect();
        let frac =
            Node::Frac { num: Box::new(Node::Row(num)), den: Box::new(Node::Row(Vec::new())), bar: true, style: None };
        self.row_mut(&path).insert(from, frac);
        self.cursor.path.push((from, 1));
        self.cursor.idx = 0;
    }

    /// `^` or `_`: a script on what's before the caret (an empty base at the start).
    fn script(&mut self, sup: bool) {
        // At the end of a subscript, ^ adds the superscript to the same base (x_i^2), and the other way round.
        if let Some((&(i, k), parent)) = self.cursor.path.split_last()
            && self.cursor.idx == self.row().len()
            && let Node::Scripts { sub, sup: s, .. } = &items(self.row_at(parent))[i]
        {
            let n = slots(&items(self.row_at(parent))[i]).len();
            let in_sub = sub.is_some() && k + usize::from(s.is_some()) + 1 == n;
            let in_sup = s.is_some() && k + 1 == n;
            if (sup && in_sub) || (!sup && in_sup) {
                self.cursor.path.pop();
                self.cursor.idx = i + 1;
            }
        }
        let path = self.cursor.path.clone();
        let at = self.cursor.idx;
        let row = self.row_mut(&path);
        let (i, node) = if at > 0 {
            let prev = row.remove(at - 1);
            let node = match prev {
                Node::Scripts { base, sub, sup: s } => {
                    let sub = if !sup { sub.or_else(|| Some(Box::new(Node::Row(Vec::new())))) } else { sub };
                    let s = if sup { s.or_else(|| Some(Box::new(Node::Row(Vec::new())))) } else { s };
                    Node::Scripts { base, sub, sup: s }
                }
                base => {
                    let e = Some(Box::new(Node::Row(Vec::new())));
                    if sup {
                        Node::Scripts { base: Box::new(base), sub: None, sup: e }
                    } else {
                        Node::Scripts { base: Box::new(base), sub: e, sup: None }
                    }
                }
            };
            (at - 1, node)
        } else {
            let e = || Some(Box::new(Node::Row(Vec::new())));
            let node = if sup {
                Node::Scripts { base: Box::new(Node::Row(Vec::new())), sub: None, sup: e() }
            } else {
                Node::Scripts { base: Box::new(Node::Row(Vec::new())), sub: e(), sup: None }
            };
            (at, node)
        };
        row.insert(i, node);
        // Into the script just made (or already there): the last box for ^, the one before for _.
        let n = &items(self.row_at(&path))[i];
        let k = match n {
            Node::Scripts { sub, sup: s, .. } => {
                let k = slots(n).len() - 1;
                if sup || s.is_none() { k } else { k - usize::from(sub.is_some() && s.is_some()) }
            }
            _ => 0,
        };
        let len = items(slots(n)[k]).len();
        self.cursor.path.push((i, k));
        self.cursor.idx = len;
    }

    pub fn backspace(&mut self) {
        let path = self.cursor.path.clone();
        let at = self.cursor.idx;
        if at > 0 {
            let prev = &self.row()[at - 1];
            if slots(prev).is_empty() {
                self.row_mut(&path).remove(at - 1);
                self.cursor.idx -= 1;
            } else if all_empty(prev) {
                let prev = self.row_mut(&path).remove(at - 1);
                let left = residue(prev);
                let n = left.len();
                self.row_mut(&path).splice(at - 1..at - 1, left);
                self.cursor.idx = at - 1 + n;
            } else {
                // Into its last box, at the end.
                let k = slots(prev).len() - 1;
                self.cursor.path.push((at - 1, k));
                self.cursor.idx = self.row().len();
            }
            return;
        }
        // At the start of a box: the structure is taken apart.
        let Some((i, k)) = self.cursor.path.pop() else { return };
        let parent = self.cursor.path.clone();
        let node = self.row_mut(&parent).remove(i);
        let before: usize = {
            let base = usize::from(matches!(&node, Node::Scripts { base, .. } if !matches!(**base, Node::Row(_))));
            base + slots(&node).iter().take(k).map(|s| items(s).len()).sum::<usize>()
        };
        let parts = if all_empty(&node) { residue(node) } else { unwrap(node) };
        let before = before.min(parts.len());
        self.row_mut(&parent).splice(i..i, parts);
        self.cursor.idx = i + before;
    }

    pub fn delete(&mut self) {
        let path = self.cursor.path.clone();
        let at = self.cursor.idx;
        let Some(next) = self.row().get(at) else { return };
        if slots(next).is_empty() {
            self.row_mut(&path).remove(at);
        } else if all_empty(next) {
            let next = self.row_mut(&path).remove(at);
            self.row_mut(&path).splice(at..at, residue(next));
        } else {
            self.cursor.path.push((at, 0));
            self.cursor.idx = 0;
        }
    }

    pub fn right(&mut self) {
        let at = self.cursor.idx;
        if let Some(next) = self.row().get(at) {
            if slots(next).is_empty() {
                self.cursor.idx += 1;
            } else {
                self.cursor.path.push((at, 0));
                self.cursor.idx = 0;
            }
            return;
        }
        let Some((i, k)) = self.cursor.path.pop() else { return };
        let count = slots(&items(self.row_at(&self.cursor.path))[i]).len();
        if k + 1 < count {
            self.cursor.path.push((i, k + 1));
            self.cursor.idx = 0;
        } else {
            self.cursor.idx = i + 1;
        }
    }

    pub fn left(&mut self) {
        let at = self.cursor.idx;
        if at > 0 {
            let prev = &self.row()[at - 1];
            if slots(prev).is_empty() {
                self.cursor.idx -= 1;
            } else {
                let k = slots(prev).len() - 1;
                self.cursor.path.push((at - 1, k));
                self.cursor.idx = self.row().len();
            }
            return;
        }
        let Some((i, k)) = self.cursor.path.pop() else { return };
        if k > 0 {
            self.cursor.path.push((i, k - 1));
            self.cursor.idx = self.row().len();
        } else {
            self.cursor.idx = i;
        }
    }

    pub fn home(&mut self) {
        self.cursor.idx = 0;
    }

    pub fn end(&mut self) {
        self.cursor.idx = self.row().len();
    }

    /// Up (or down, `up` false): to the box above — numerator, superscript,
    /// the matrix row above.
    pub fn vertical(&mut self, up: bool) {
        for level in (0..self.cursor.path.len()).rev() {
            let (i, k) = self.cursor.path[level];
            let parent = items(self.row_at(&self.cursor.path[..level]));
            let node = &parent[i];
            let target = match node {
                Node::Frac { .. } => match (up, k) {
                    (true, 1) => Some(0),
                    (false, 0) => Some(1),
                    _ => None,
                },
                Node::Scripts { sub, sup, .. } => {
                    let has_base = slots(node).len() > usize::from(sub.is_some()) + usize::from(sup.is_some());
                    let b = usize::from(has_base);
                    let (sub_k, sup_k) =
                        (sub.as_ref().map(|_| b), sup.as_ref().map(|_| b + usize::from(sub.is_some())));
                    if up && Some(k) != sup_k {
                        sup_k
                    } else if !up && Some(k) != sub_k {
                        sub_k
                    } else {
                        None
                    }
                }
                Node::Matrix { rows, .. } => {
                    let cols = rows.first().map_or(1, Vec::len).max(1);
                    if up { k.checked_sub(cols) } else { Some(k + cols).filter(|&t| t < rows.len() * cols) }
                }
                _ => None,
            };
            if let Some(t) = target {
                self.cursor.path.truncate(level);
                self.cursor.path.push((i, t));
                self.cursor.idx = self.cursor.idx.min(self.row().len());
                return;
            }
        }
    }

    /// Tab: to the start of the next box (Shift+Tab: the end of the one before).
    pub fn next_box(&mut self, back: bool) {
        let start = self.cursor.clone();
        for _ in 0..10_000 {
            let before = self.cursor.clone();
            if back {
                self.left()
            } else {
                self.right()
            };
            if self.cursor == before {
                return;
            }
            let entered = self.cursor.path != start.path && self.cursor.path.len() >= before.path.len();
            if entered && (if back { self.cursor.idx == self.row().len() } else { self.cursor.idx == 0 }) {
                return;
            }
        }
    }

    /// Typesets for editing; see [`crate::typeset_edit`].
    pub fn typeset(&self, font: &crate::MathFont, size: f32, display: bool) -> MathBox {
        crate::typeset_edit(font, &self.root, size, display)
    }

    /// The caret in `b` (typeset from this editor): x, top y, height.
    pub fn caret(&self, b: &MathBox) -> Option<(f32, f32, f32)> {
        let (row, idx) = (self.row_addr(), self.cursor.idx);
        b.pieces.iter().find_map(|p| match *p {
            Piece::Mark { row: r, index, x, y, h } if r == row && index == idx => Some((x, y, h)),
            _ => None,
        })
    }

    /// Puts the caret at the place nearest (x, y) in `b` (typeset from this editor).
    pub fn click(&mut self, b: &MathBox, x: f32, y: f32) {
        let best = b
            .pieces
            .iter()
            .filter_map(|p| match *p {
                Piece::Mark { row, index, x: mx, y: my, h } => {
                    let inside = y >= my && y <= my + h;
                    let dy = if inside { 0.0 } else { (y - my).abs().min((y - my - h).abs()) };
                    // Inside a box beats outside; then nearest; then the smaller (deeper) box.
                    Some(((!inside, (dy + (x - mx).abs()) * 1000.0, h * 10.0), row, index))
                }
                _ => None,
            })
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let Some((_, row, index)) = best else { return };
        if let Some(path) = self.path_of(row) {
            self.cursor = Cursor { path, idx: index };
        }
    }

    /// The path of the row at address `addr`.
    fn path_of(&self, addr: usize) -> Option<Vec<(usize, usize)>> {
        fn walk(n: &Node, addr: usize, path: &mut Vec<(usize, usize)>) -> bool {
            if n as *const Node as usize == addr {
                return true;
            }
            for (i, it) in items(n).iter().enumerate() {
                for (k, s) in slots(it).into_iter().enumerate() {
                    path.push((i, k));
                    if walk(s, addr, path) {
                        return true;
                    }
                    path.pop();
                }
            }
            false
        }
        let mut path = Vec::new();
        walk(&self.root, addr, &mut path).then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> MathEdit {
        let mut e = MathEdit::new("");
        for c in s.chars() {
            match c {
                '→' => e.right(),
                '←' => e.left(),
                '⌫' => e.backspace(),
                '↑' => e.vertical(true),
                '↓' => e.vertical(false),
                '⇥' => e.next_box(false),
                c => e.type_char(c),
            }
        }
        e
    }

    #[test]
    fn typing_builds_structures() {
        assert_eq!(typed("1+2/3").latex(), "1+\\frac{2}{3}");
        assert_eq!(typed("x^2 +1").latex(), "x^{2}+1");
        assert_eq!(typed("x_i^2").latex(), "x_{i}^{2}");
        assert_eq!(typed("a/b→+c").latex(), "\\frac{a}{b}+c");
        assert_eq!(typed("(a+b)c").latex(), "\\left( a+b \\right)c");
        // Up and down between numerator and denominator.
        assert_eq!(typed("/b↑a").latex(), "\\frac{a}{b}");
    }

    #[test]
    fn commands_make_boxes_to_fill() {
        let mut e = MathEdit::new("");
        e.command("sqrt");
        e.type_char('x');
        e.right();
        e.type_char('+');
        e.command("frac");
        e.type_char('1');
        e.next_box(false);
        e.type_char('n');
        assert_eq!(e.latex(), "\\sqrt{x}+\\frac{1}{n}");
        let mut e = MathEdit::new("");
        e.command("sum");
        for c in "i=1".chars() {
            e.type_char(c);
        }
        e.next_box(false);
        e.type_char('n');
        assert_eq!(e.latex(), "\\sum_{i=1}^{n}");
        e.right();
        e.command("alpha");
        assert_eq!(e.latex(), "\\sum_{i=1}^{n}\\alpha");
    }

    #[test]
    fn backspace_takes_structures_apart() {
        // An empty fraction goes; one with something in it is entered first.
        assert_eq!(typed("a/⌫").latex(), "a");
        assert_eq!(typed("/⌫").latex(), "");
        assert_eq!(typed("x^2→⌫").latex(), "x^{2}");
        assert_eq!(typed("x^2→⌫⌫").latex(), "x^{}");
        assert_eq!(typed("x^2→⌫⌫⌫").latex(), "x");
        // At the start of a box: the structure is unwrapped.
        let mut e = MathEdit::new("\\sqrt{ab}");
        e.cursor = Cursor { path: vec![(0, 0)], idx: 0 };
        e.backspace();
        assert_eq!(e.latex(), "ab");
    }

    #[test]
    fn loads_saves_and_clicks() {
        let e = MathEdit::new("\\frac{a}{b} + \\sqrt[3]{x}");
        assert_eq!(parse(&e.latex()), parse("\\frac{a}{b} + \\sqrt[3]{x}"));
        let font = crate::MathFont::new(include_bytes!("../../../assets/fonts/STIXTwoMath-Regular.ttf"));
        let Some(font) = font else { return };
        let mut e = MathEdit::new("\\sqrt{x} + y");
        let b = e.typeset(&font, 20.0, false);
        // Every caret place has a mark; the caret is at the end.
        let (x_end, _, _) = e.caret(&b).expect("caret");
        assert!(x_end > 10.0);
        // A click inside the root's box puts the caret in it.
        e.click(&b, 16.0, -5.0);
        assert_eq!(e.cursor.path, vec![(0, 0)]);
        e.type_char('2');
        assert!(e.latex().starts_with("\\sqrt{"), "{}", e.latex());
        // Empty boxes are drawn as slots.
        let e = MathEdit::new("\\frac{}{}");
        assert!(e.typeset(&font, 20.0, false).pieces.iter().filter(|p| matches!(p, Piece::Slot { .. })).count() == 2);
    }
}
