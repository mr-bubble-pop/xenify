//! Typesetting a [`Node`] tree with an OpenType MATH font, after TeX's rules
//! (fractions, scripts, radicals, big operators, growing delimiters, accents,
//! matrices), using the font's own MATH constants and glyph variants.

use crate::parse::{Class, Env, Node, Variant};
use ttf_parser::{Face, GlyphId};

/// A font with a MATH table (STIX Two Math).
pub struct MathFont {
    face: Face<'static>,
    upem: f32,
}

impl MathFont {
    /// None when `data` isn't a font with a MATH table.
    pub fn new(data: &'static [u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        face.tables().math?.constants?;
        let upem = face.units_per_em() as f32;
        Some(Self { face, upem })
    }

    fn constants(&self) -> ttf_parser::math::Constants<'static> {
        self.face.tables().math.and_then(|m| m.constants).expect("checked in new")
    }

    fn gid(&self, c: char) -> Option<GlyphId> {
        self.face.glyph_index(c)
    }

    fn advance(&self, g: GlyphId) -> f32 {
        self.face.glyph_hor_advance(g).unwrap_or(0) as f32
    }

    fn bbox(&self, g: GlyphId) -> (f32, f32) {
        // (height above the baseline, depth below), in font units.
        self.face.glyph_bounding_box(g).map_or((0.0, 0.0), |b| (b.y_max as f32, -(b.y_min as f32)))
    }

    fn italic(&self, g: GlyphId) -> f32 {
        let info = self.face.tables().math.and_then(|m| m.glyph_info);
        info.and_then(|i| i.italic_corrections).and_then(|v| v.get(g)).map_or(0.0, |v| v.value as f32)
    }

    fn top_accent(&self, g: GlyphId) -> Option<f32> {
        let info = self.face.tables().math.and_then(|m| m.glyph_info);
        info.and_then(|i| i.top_accent_attachments).and_then(|v| v.get(g)).map(|v| v.value as f32)
    }
}

/// Something drawn, with its origin on the equation's baseline (y down).
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    /// A glyph of the maths font, at `size` points; `error` draws it red.
    Glyph { id: u16, x: f32, y: f32, size: f32, error: bool },
    /// A filled rectangle (fraction bars, radical overbars), top-left at x, y.
    Rule { x: f32, y: f32, w: f32, h: f32 },
    /// Editing only ([`typeset_edit`]): a place the caret can be — before
    /// item `index` of the row node at address `row` — as a line from (x, y) down `h`.
    Mark { row: usize, index: usize, x: f32, y: f32, h: f32 },
    /// Editing only: an empty box waiting to be filled, top-left at x, y.
    Slot { x: f32, y: f32, w: f32, h: f32 },
}

/// A typeset box: its extent around the baseline and what it draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MathBox {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub pieces: Vec<Piece>,
    /// Italic correction of the last glyph (for superscripts).
    italic: f32,
    /// A single character: scripts attach closer (TeX's rule).
    simple: bool,
}

impl MathBox {
    fn shifted(mut self, dx: f32, dy: f32) -> Self {
        for p in &mut self.pieces {
            match p {
                Piece::Glyph { x, y, .. }
                | Piece::Rule { x, y, .. }
                | Piece::Mark { x, y, .. }
                | Piece::Slot { x, y, .. } => {
                    *x += dx;
                    *y += dy;
                }
            }
        }
        self
    }

    /// Puts `other` into this box at (dx, dy) without changing the extent.
    fn add(&mut self, other: MathBox, dx: f32, dy: f32) {
        self.pieces.extend(other.shifted(dx, dy).pieces);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    Display,
    Text,
    Script,
    ScriptScript,
}

#[derive(Clone, Copy, Debug)]
struct Style {
    level: Level,
    cramped: bool,
}

impl Style {
    fn sup(self) -> Style {
        let level = match self.level {
            Level::Display | Level::Text => Level::Script,
            _ => Level::ScriptScript,
        };
        Style { level, cramped: self.cramped }
    }

    fn sub(self) -> Style {
        Style { cramped: true, ..self.sup() }
    }

    fn num(self) -> Style {
        let level = match self.level {
            Level::Display => Level::Text,
            Level::Text => Level::Script,
            _ => Level::ScriptScript,
        };
        Style { level, cramped: self.cramped }
    }

    fn den(self) -> Style {
        Style { cramped: true, ..self.num() }
    }

    fn cramp(self) -> Style {
        Style { cramped: true, ..self }
    }

    fn display(self) -> bool {
        self.level == Level::Display
    }

    fn script(self) -> bool {
        matches!(self.level, Level::Script | Level::ScriptScript)
    }
}

struct Ctx<'a> {
    font: &'a MathFont,
    /// The equation's font size, in points.
    size: f32,
    /// For the visual editor: caret marks and empty boxes.
    edit: bool,
}

impl Ctx<'_> {
    fn scale(&self, s: Style) -> f32 {
        let c = self.font.constants();
        let pct = match s.level {
            Level::Display | Level::Text => 100.0,
            Level::Script => c.script_percent_scale_down() as f32,
            Level::ScriptScript => c.script_script_percent_scale_down() as f32,
        };
        self.size * pct / 100.0 / self.font.upem
    }

    /// A MATH constant at style `s`, in points.
    fn k(&self, s: Style, v: ttf_parser::math::MathValue) -> f32 {
        v.value as f32 * self.scale(s)
    }

    /// One em at `s`, in points.
    fn em(&self, s: Style) -> f32 {
        self.font.upem * self.scale(s)
    }

    fn glyph_box(&self, g: GlyphId, s: Style, error: bool) -> MathBox {
        let k = self.scale(s);
        let (a, d) = self.font.bbox(g);
        MathBox {
            width: self.font.advance(g) * k,
            ascent: a * k,
            descent: d * k,
            pieces: vec![Piece::Glyph {
                id: g.0,
                x: 0.0,
                y: 0.0,
                size: self.size * k * self.font.upem / self.size,
                error,
            }],
            italic: self.font.italic(g) * k,
            simple: true,
        }
    }

    fn char_box(&self, c: char, s: Style, error: bool) -> MathBox {
        match self.font.gid(c).or_else(|| self.font.gid('?')) {
            Some(g) => self.glyph_box(g, s, error),
            None => MathBox::default(),
        }
    }

    fn text_box(&self, text: &str, s: Style, error: bool) -> MathBox {
        hbox(text.chars().map(|c| self.char_box(c, s, error)).collect())
    }
}

/// Boxes side by side on one baseline.
fn hbox(parts: Vec<MathBox>) -> MathBox {
    let mut out = MathBox { simple: parts.len() == 1 && parts[0].simple, ..Default::default() };
    for p in parts {
        out.ascent = out.ascent.max(p.ascent);
        out.descent = out.descent.max(p.descent);
        let w = p.width;
        out.italic = p.italic;
        out.add(p, out.width, 0.0);
        out.width += w;
    }
    out
}

/// `c` in letter style `v` (Unicode's mathematical alphabets).
fn styled(c: char, v: Variant) -> char {
    let off = |base: u32, from: char| char::from_u32(base + (c as u32 - from as u32)).unwrap_or(c);
    let latin = |upper: u32, lower: u32, digits: Option<u32>, exceptions: &[(char, char)]| {
        if let Some((_, e)) = exceptions.iter().find(|(k, _)| *k == c) {
            return *e;
        }
        match c {
            'A'..='Z' => off(upper, 'A'),
            'a'..='z' => off(lower, 'a'),
            '0'..='9' => digits.map_or(c, |d| off(d, '0')),
            _ => c,
        }
    };
    match v {
        Variant::Normal => match c {
            'h' => 'ℎ',
            'A'..='Z' | 'a'..='z' => latin(0x1D434, 0x1D44E, None, &[]),
            'α'..='ω' => off(0x1D6FC, 'α'),
            'ϵ' => '\u{1D716}',
            'ϑ' => '\u{1D717}',
            'ϰ' => '\u{1D718}',
            'ϕ' => '\u{1D719}',
            'ϱ' => '\u{1D71A}',
            'ϖ' => '\u{1D71B}',
            _ => c,
        },
        Variant::Upright => c,
        Variant::Bold => latin(0x1D400, 0x1D41A, Some(0x1D7CE), &[]),
        Variant::Blackboard => latin(
            0x1D538,
            0x1D552,
            Some(0x1D7D8),
            &[('C', 'ℂ'), ('H', 'ℍ'), ('N', 'ℕ'), ('P', 'ℙ'), ('Q', 'ℚ'), ('R', 'ℝ'), ('Z', 'ℤ')],
        ),
        Variant::Calligraphic => latin(
            0x1D49C,
            0x1D4B6,
            None,
            &[
                ('B', 'ℬ'),
                ('E', 'ℰ'),
                ('F', 'ℱ'),
                ('H', 'ℋ'),
                ('I', 'ℐ'),
                ('L', 'ℒ'),
                ('M', 'ℳ'),
                ('R', 'ℛ'),
                ('e', 'ℯ'),
                ('g', 'ℊ'),
                ('o', 'ℴ'),
            ],
        ),
        Variant::Fraktur => {
            latin(0x1D504, 0x1D51E, None, &[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')])
        }
        Variant::Mono => latin(0x1D670, 0x1D68A, Some(0x1D7F6), &[]),
        Variant::Sans => latin(0x1D5A0, 0x1D5BA, Some(0x1D7E2), &[]),
    }
}

/// Space between two atoms, in mu (TeX's table; none in scripts but thin).
fn spacing(left: Class, right: Class, s: Style) -> f32 {
    use Class::*;
    let (thin, med, thick) = (3.0, 4.0, 5.0);
    let v = match (left, right) {
        (Ord, Op) | (Op, Ord) | (Op, Op) | (Close, Op) => thin,
        (Ord, Bin) | (Bin, Ord) | (Bin, Op) | (Op, Bin) | (Close, Bin) | (Bin, Open) => med,
        (Ord, Rel) | (Rel, Ord) | (Rel, Op) | (Op, Rel) | (Close, Rel) | (Rel, Open) => thick,
        (Punct, _) => thin,
        _ => 0.0,
    };
    if s.script() && v != thin { 0.0 } else { v }
}

/// A node's spacing class (first, last).
fn class_of(n: &Node) -> Class {
    match n {
        Node::Sym { class, .. } | Node::Text { class, .. } => *class,
        Node::BigOp { .. } => Class::Op,
        Node::Scripts { base, .. } => class_of(base),
        Node::Fenced { .. } => Class::Ord,
        Node::Row(v) => v.first().map_or(Class::Ord, class_of),
        _ => Class::Ord,
    }
}

/// A delimiter or radical sign at least `height` tall (points): a bigger
/// variant of the glyph, or one built from its parts.
fn stretchy(cx: &Ctx, c: char, height: f32, s: Style) -> MathBox {
    let font = cx.font;
    let Some(g) = font.gid(c) else { return cx.char_box(c, s, true) };
    let k = cx.scale(s);
    let base = cx.glyph_box(g, s, false);
    if base.ascent + base.descent >= height {
        return base;
    }
    let variants = font.face.tables().math.and_then(|m| m.variants);
    let Some(cons) = variants.and_then(|v| v.vertical_constructions.get(g)) else { return base };
    for v in cons.variants {
        let b = cx.glyph_box(v.variant_glyph, s, false);
        if v.advance_measurement as f32 * k >= height {
            return b;
        }
    }
    // Built from parts: the extenders repeated until it's tall enough.
    let Some(asm) = cons.assembly else {
        return cons.variants.last().map_or(base, |v| cx.glyph_box(v.variant_glyph, s, false));
    };
    let overlap = variants.map_or(0.0, |v| v.min_connector_overlap as f32) * k;
    let parts: Vec<_> = asm.parts.into_iter().collect();
    let fixed: f32 =
        parts.iter().filter(|p| !p.part_flags.extender()).map(|p| p.full_advance as f32 * k - overlap).sum();
    let ext: f32 = parts.iter().filter(|p| p.part_flags.extender()).map(|p| p.full_advance as f32 * k - overlap).sum();
    let reps = if ext > 0.0 { ((height - fixed - overlap) / ext).ceil().max(0.0) as usize } else { 0 };
    let mut seq = Vec::new();
    for p in &parts {
        let n = if p.part_flags.extender() { reps } else { 1 };
        for _ in 0..n {
            seq.push(*p);
        }
    }
    let total: f32 =
        seq.iter().map(|p| p.full_advance as f32 * k).sum::<f32>() - overlap * (seq.len().saturating_sub(1)) as f32;
    // Parts stack bottom to top; centred on the maths axis.
    let axis = cx.k(s, font.constants().axis_height());
    let mut out = MathBox { ascent: total / 2.0 + axis, descent: total / 2.0 - axis, ..Default::default() };
    let mut y = out.descent;
    for p in &seq {
        let adv = p.full_advance as f32 * k;
        let b = cx.glyph_box(p.glyph_id, s, false);
        out.width = out.width.max(b.width);
        // The part's bottom at y: its own baseline is its bottom here (bbox
        // depth added so the glyph sits on the stacking point).
        out.add(b.clone(), 0.0, y - b.descent);
        y -= adv - overlap;
    }
    out
}

/// A horizontally stretched accent (wide hat, tilde) at least `width` wide.
fn wide(cx: &Ctx, c: char, width: f32, s: Style) -> MathBox {
    let font = cx.font;
    let Some(g) = font.gid(c) else { return MathBox::default() };
    let k = cx.scale(s);
    let variants = font.face.tables().math.and_then(|m| m.variants);
    let mut best = cx.glyph_box(g, s, false);
    if let Some(cons) = variants.and_then(|v| v.horizontal_constructions.get(g)) {
        for v in cons.variants {
            best = cx.glyph_box(v.variant_glyph, s, false);
            if v.advance_measurement as f32 * k >= width {
                break;
            }
        }
    }
    best
}

fn layout(cx: &Ctx, n: &Node, s: Style) -> MathBox {
    let c = cx.font.constants();
    match n {
        Node::Sym { ch, variant, .. } => cx.char_box(styled(*ch, *variant), s, false),
        Node::Text { text, .. } => cx.text_box(text, s, false),
        Node::Error(t) => cx.text_box(t, s, true),
        Node::Space(mu) => MathBox { width: mu / 18.0 * cx.em(s), ..Default::default() },
        Node::Row(items) if cx.edit && items.is_empty() => {
            // An empty box to fill, with the caret's place in it.
            let em = cx.em(s);
            let (w, a, d) = (0.6 * em, 0.72 * em, 0.12 * em);
            let row = n as *const Node as usize;
            MathBox {
                width: w,
                ascent: a,
                descent: d,
                pieces: vec![
                    Piece::Slot { x: 0.0, y: -a, w, h: a + d },
                    Piece::Mark { row, index: 0, x: 0.15 * w, y: -a, h: a + d },
                ],
                ..Default::default()
            }
        }
        Node::Row(items) => {
            let mut parts = Vec::new();
            let mut prev: Option<Class> = None;
            // Where each item starts (for the caret marks).
            let mut starts = Vec::new();
            for it in items {
                let mut cls = class_of(it);
                // A binary operator with nothing on its left is an ordinary symbol (a minus sign).
                if cls == Class::Bin
                    && matches!(prev, None | Some(Class::Bin | Class::Rel | Class::Open | Class::Punct | Class::Op))
                {
                    cls = Class::Ord;
                }
                if let Some(p) = prev {
                    let mu = spacing(p, cls, s);
                    if mu != 0.0 {
                        parts.push(MathBox { width: mu / 18.0 * cx.em(s), ..Default::default() });
                    }
                }
                starts.push(parts.iter().map(|p: &MathBox| p.width).sum::<f32>());
                parts.push(layout(cx, it, s));
                prev = Some(cls);
            }
            let mut b = hbox(parts);
            b.simple = items.len() == 1 && b.simple;
            if cx.edit {
                let row = n as *const Node as usize;
                let em = cx.em(s);
                let (a, d) = (b.ascent.max(0.72 * em), b.descent.max(0.12 * em));
                starts.push(b.width);
                for (index, x) in starts.into_iter().enumerate() {
                    b.pieces.push(Piece::Mark { row, index, x, y: -a, h: a + d });
                }
            }
            b
        }
        Node::Frac { num, den, bar, style } => {
            let s = match style {
                Some(true) => Style { level: Level::Display, ..s },
                Some(false) => Style { level: Level::Text, ..s },
                None => s,
            };
            let nb = layout(cx, num, s.num());
            let db = layout(cx, den, s.den());
            let d = s.display();
            let axis = cx.k(s, c.axis_height());
            let rule = if *bar { cx.k(s, c.fraction_rule_thickness()) } else { 0.0 };
            let mut up = cx
                .k(s, if d { c.fraction_numerator_display_style_shift_up() } else { c.fraction_numerator_shift_up() });
            let mut down = cx.k(
                s,
                if d { c.fraction_denominator_display_style_shift_down() } else { c.fraction_denominator_shift_down() },
            );
            let gap_n =
                cx.k(s, if d { c.fraction_num_display_style_gap_min() } else { c.fraction_numerator_gap_min() });
            let gap_d =
                cx.k(s, if d { c.fraction_denom_display_style_gap_min() } else { c.fraction_denominator_gap_min() });
            // Keep the gaps around the bar.
            up = up.max(axis + rule / 2.0 + gap_n + nb.descent);
            down = down.max(-(axis - rule / 2.0) + gap_d + db.ascent);
            let pad = 0.12 * cx.em(s);
            let w = nb.width.max(db.width) + 2.0 * pad;
            let mut out =
                MathBox { width: w, ascent: up + nb.ascent, descent: down + db.descent, ..Default::default() };
            let (nw, dw) = (nb.width, db.width);
            out.add(nb, (w - nw) / 2.0, -up);
            out.add(db, (w - dw) / 2.0, down);
            if *bar {
                out.pieces.push(Piece::Rule { x: pad / 2.0, y: -axis - rule / 2.0, w: w - pad, h: rule });
            }
            out
        }
        Node::Scripts { base, sub, sup } => {
            // A limits operator in display style: scripts above and below.
            let limits = matches!(&**base, Node::BigOp { limits: true, .. })
                || matches!(&**base, Node::Text { class: Class::Op, text } if ["lim", "limsup", "liminf", "max", "min", "sup", "inf"].contains(&text.as_str()));
            let bb = layout(cx, base, s);
            if limits && s.display() {
                return limits_box(cx, bb, sub.as_deref(), sup.as_deref(), s);
            }
            scripts(cx, bb, sub.as_deref(), sup.as_deref(), s)
        }
        Node::BigOp { ch, .. } => {
            let mut b = cx.char_box(*ch, s, false);
            if s.display() {
                let min = c.display_operator_min_height() as f32 * cx.scale(s);
                b = stretchy(cx, *ch, min, s);
            }
            // Centred on the maths axis.
            let axis = cx.k(s, c.axis_height());
            let shift = (b.ascent - b.descent) / 2.0 - axis;
            let mut out = MathBox {
                width: b.width,
                ascent: b.ascent - shift,
                descent: b.descent + shift,
                italic: b.italic,
                ..Default::default()
            };
            out.add(b, 0.0, shift);
            out
        }
        Node::Sqrt { index, body } => {
            let bb = layout(cx, body, s.cramp());
            let d = s.display();
            let gap = cx.k(s, if d { c.radical_display_style_vertical_gap() } else { c.radical_vertical_gap() });
            let rule = cx.k(s, c.radical_rule_thickness());
            let extra = cx.k(s, c.radical_extra_ascender());
            let need = bb.ascent + bb.descent + gap + rule;
            let sign = stretchy(cx, '√', need, s);
            // The sign's top is where the bar goes.
            let top = bb.ascent + gap + rule;
            // Moved down until its top meets the bar.
            let shift = sign.ascent - top;
            let sw = sign.width;
            let mut out =
                MathBox { ascent: top + extra, descent: bb.descent.max(sign.descent + shift), ..Default::default() };
            let mut x = 0.0;
            if let Some(ix) = index {
                let ib = layout(cx, ix, Style { level: Level::ScriptScript, cramped: true });
                let before = cx.k(s, c.radical_kern_before_degree());
                let after = cx.k(s, c.radical_kern_after_degree());
                let raise = c.radical_degree_bottom_raise_percent() as f32 / 100.0 * (sign.ascent + sign.descent);
                let iy = -(raise - sign.descent - shift) - ib.descent;
                out.ascent = out.ascent.max(-iy + ib.ascent);
                let iw = ib.width;
                out.add(ib, before, iy);
                x = (before + iw + after).max(0.0);
            }
            out.add(sign, x, shift);
            x += sw;
            out.pieces.push(Piece::Rule { x, y: -top, w: bb.width, h: rule });
            let w = bb.width;
            out.add(bb, x, 0.0);
            out.width = x + w;
            out
        }
        Node::Fenced { open, close, body } => {
            let bb = layout(cx, body, s);
            let axis = cx.k(s, c.axis_height());
            // Tall enough to cover the body around the axis.
            let half = (bb.ascent - axis).max(bb.descent + axis);
            let height = (2.0 * half * 0.95).max(c.delimited_sub_formula_min_height() as f32 * cx.scale(s) * 0.5);
            let mut parts = Vec::new();
            let fence = |ch: char| {
                let mut f = stretchy(cx, ch, height, s);
                // Centred on the axis.
                let shift = (f.ascent - f.descent) / 2.0 - axis;
                let mut out = MathBox {
                    width: f.width,
                    ascent: f.ascent - shift,
                    descent: f.descent + shift,
                    ..Default::default()
                };
                f.italic = 0.0;
                out.add(f, 0.0, shift);
                out
            };
            if let Some(o) = open {
                parts.push(fence(*o));
            }
            parts.push(bb);
            if let Some(cl) = close {
                parts.push(fence(*cl));
            }
            let mut b = hbox(parts);
            b.simple = false;
            b
        }
        Node::Accent { ch, body, wide: is_wide } => {
            let bb = layout(cx, body, s.cramp());
            let acc = if *is_wide { wide(cx, *ch, bb.width, s) } else { cx.char_box(*ch, s, false) };
            // Accents sit at the accent base height, centred (or on the
            // glyph's own attachment point).
            let base_h = cx.k(s, c.accent_base_height());
            let lift = (bb.ascent - base_h).max(0.0);
            let inner = match &**body {
                Node::Row(v) if v.len() == 1 => &v[0],
                b => b,
            };
            let attach = match (inner, bb.pieces.first()) {
                (Node::Sym { .. }, Some(Piece::Glyph { id, .. })) => {
                    cx.font.top_accent(GlyphId(*id)).map(|a| a * cx.scale(s)).unwrap_or(bb.width / 2.0)
                }
                _ => bb.width / 2.0,
            };
            let acc_attach = acc.pieces.first().and_then(|p| match p {
                Piece::Glyph { id, .. } => cx.font.top_accent(GlyphId(*id)).map(|a| a * cx.scale(s)),
                _ => None,
            });
            let ax = attach - acc_attach.unwrap_or(acc.width / 2.0);
            let mut out = MathBox {
                width: bb.width,
                ascent: bb.ascent.max(acc.ascent + lift),
                descent: bb.descent,
                italic: bb.italic,
                ..Default::default()
            };
            out.add(acc, ax, -lift);
            out.add(bb, 0.0, 0.0);
            out
        }
        Node::Over { body } => {
            let bb = layout(cx, body, s.cramp());
            let gap = cx.k(s, c.overbar_vertical_gap());
            let rule = cx.k(s, c.overbar_rule_thickness());
            let extra = cx.k(s, c.overbar_extra_ascender());
            let mut out = MathBox {
                width: bb.width,
                ascent: bb.ascent + gap + rule + extra,
                descent: bb.descent,
                ..Default::default()
            };
            out.pieces.push(Piece::Rule { x: 0.0, y: -(bb.ascent + gap + rule), w: bb.width, h: rule });
            out.add(bb, 0.0, 0.0);
            out
        }
        Node::Under { body } => {
            let bb = layout(cx, body, s);
            let gap = cx.k(s, c.underbar_vertical_gap());
            let rule = cx.k(s, c.underbar_rule_thickness());
            let extra = cx.k(s, c.underbar_extra_descender());
            let mut out = MathBox {
                width: bb.width,
                ascent: bb.ascent,
                descent: bb.descent + gap + rule + extra,
                ..Default::default()
            };
            out.pieces.push(Piece::Rule { x: 0.0, y: bb.descent + gap, w: bb.width, h: rule });
            out.add(bb, 0.0, 0.0);
            out
        }
        Node::Matrix { env, rows } => matrix(cx, *env, rows, s),
    }
}

/// Scripts beside the base (TeX rule 18).
fn scripts(cx: &Ctx, base: MathBox, sub: Option<&Node>, sup: Option<&Node>, s: Style) -> MathBox {
    let c = cx.font.constants();
    let sup_b = sup.map(|n| layout(cx, n, s.sup()));
    let sub_b = sub.map(|n| layout(cx, n, s.sub()));
    let simple = base.simple;
    // A big operator's slant: the subscript tucks under it (∫).
    let op = base.pieces.len() == 1 && base.italic > 0.0 && !simple;
    let mut sup_y = 0.0f32;
    let mut sub_y = 0.0f32;
    if let Some(sp) = &sup_b {
        let shift = cx.k(s, if s.cramped { c.superscript_shift_up_cramped() } else { c.superscript_shift_up() });
        let drop = if simple { 0.0 } else { base.ascent - cx.k(s.sup(), c.superscript_baseline_drop_max()) };
        sup_y = shift.max(drop).max(sp.descent + cx.k(s, c.superscript_bottom_min()));
    }
    if let Some(sb) = &sub_b {
        let shift = cx.k(s, c.subscript_shift_down());
        let drop = if simple { 0.0 } else { base.descent + cx.k(s.sub(), c.subscript_baseline_drop_min()) };
        sub_y = shift.max(drop).max(sb.ascent - cx.k(s, c.subscript_top_max()));
    }
    if let (Some(sp), Some(sb)) = (&sup_b, &sub_b) {
        // Room between them.
        let gap = (sup_y - sp.descent) - (sb.ascent - sub_y);
        let min = cx.k(s, c.sub_superscript_gap_min());
        if gap < min {
            sub_y += min - gap;
            let max_bottom = cx.k(s, c.superscript_bottom_max_with_subscript());
            let lift = max_bottom - (sup_y - sp.descent);
            if lift > 0.0 {
                sup_y += lift;
                sub_y -= lift;
            }
        }
    }
    let after = cx.k(s, c.space_after_script());
    let bw = base.width;
    let italic = base.italic;
    let mut out = MathBox { width: bw, ascent: base.ascent, descent: base.descent, ..Default::default() };
    out.add(base, 0.0, 0.0);
    let mut w = bw;
    if let Some(sp) = sup_b {
        out.ascent = out.ascent.max(sup_y + sp.ascent);
        w = w.max(bw + italic + sp.width);
        out.add(sp, bw + italic, -sup_y);
    }
    if let Some(sb) = sub_b {
        out.descent = out.descent.max(sub_y + sb.descent);
        let x = if op { bw - italic } else { bw };
        w = w.max(x + sb.width);
        out.add(sb, x, sub_y);
    }
    out.width = w + after;
    out
}

/// Limits above and below (∑ in display style, lim).
fn limits_box(cx: &Ctx, base: MathBox, sub: Option<&Node>, sup: Option<&Node>, s: Style) -> MathBox {
    let c = cx.font.constants();
    let sup_b = sup.map(|n| layout(cx, n, s.sup()));
    let sub_b = sub.map(|n| layout(cx, n, s.sub()));
    let w = base.width.max(sup_b.as_ref().map_or(0.0, |b| b.width)).max(sub_b.as_ref().map_or(0.0, |b| b.width));
    let mut out = MathBox { width: w, ascent: base.ascent, descent: base.descent, ..Default::default() };
    if let Some(sp) = sup_b {
        let y = (base.ascent + cx.k(s, c.upper_limit_gap_min()) + sp.descent)
            .max(base.ascent + cx.k(s, c.upper_limit_baseline_rise_min()));
        out.ascent = y + sp.ascent;
        let sw = sp.width;
        out.add(sp, (w - sw) / 2.0, -y);
    }
    if let Some(sb) = sub_b {
        let y = (base.descent + cx.k(s, c.lower_limit_gap_min()) + sb.ascent)
            .max(base.descent + cx.k(s, c.lower_limit_baseline_drop_min()));
        out.descent = y + sb.descent;
        let sw = sb.width;
        out.add(sb, (w - sw) / 2.0, y);
    }
    let bw = base.width;
    out.add(base, (w - bw) / 2.0, 0.0);
    out
}

fn matrix(cx: &Ctx, env: Env, rows: &[Vec<Node>], s: Style) -> MathBox {
    let cells: Vec<Vec<MathBox>> = rows.iter().map(|r| r.iter().map(|n| layout(cx, n, s)).collect()).collect();
    let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
    let em = cx.em(s);
    let col_gap = if env == Env::Aligned { 0.0 } else { em * 0.8 };
    let row_gap = em * 0.25;
    let widths: Vec<f32> =
        (0..cols).map(|j| cells.iter().filter_map(|r| r.get(j)).map(|b| b.width).fold(0.0, f32::max)).collect();
    let heights: Vec<(f32, f32)> = cells
        .iter()
        .map(|r| {
            (r.iter().map(|b| b.ascent).fold(em * 0.7, f32::max), r.iter().map(|b| b.descent).fold(em * 0.3, f32::max))
        })
        .collect();
    let total: f32 =
        heights.iter().map(|(a, d)| a + d).sum::<f32>() + row_gap * (heights.len().saturating_sub(1)) as f32;
    let axis = cx.k(s, cx.font.constants().axis_height());
    let top = total / 2.0 + axis;
    let width: f32 = widths.iter().sum::<f32>() + col_gap * (cols.saturating_sub(1)) as f32;
    let mut grid = MathBox { width, ascent: top, descent: total - top, ..Default::default() };
    let mut y = -top;
    for (r, row) in cells.into_iter().enumerate() {
        let (a, d) = heights[r];
        let base = y + a;
        let mut x = 0.0;
        for (j, cell) in row.into_iter().enumerate() {
            let cw = cell.width;
            // Aligned: columns alternate right- and left-aligned (at `&`); cases: left; else centred.
            let dx = match env {
                Env::Aligned => {
                    if j % 2 == 0 {
                        widths[j] - cw
                    } else {
                        0.0
                    }
                }
                Env::Cases => 0.0,
                _ => (widths[j] - cw) / 2.0,
            };
            grid.add(cell, x + dx, base);
            x += widths[j] + col_gap;
        }
        y = base + d + row_gap;
    }
    let fenced = |open: Option<char>, close: Option<char>, grid: MathBox| {
        let mut parts = Vec::new();
        let h = grid.ascent + grid.descent;
        let fence = |ch: char| {
            let f = stretchy(cx, ch, h * 0.95, s);
            let shift = (f.ascent - f.descent) / 2.0 - axis;
            let mut out =
                MathBox { width: f.width, ascent: f.ascent - shift, descent: f.descent + shift, ..Default::default() };
            out.add(f, 0.0, shift);
            out
        };
        if let Some(o) = open {
            parts.push(fence(o));
            parts.push(MathBox { width: em * 0.1, ..Default::default() });
        }
        parts.push(grid);
        if let Some(cl) = close {
            parts.push(MathBox { width: em * 0.1, ..Default::default() });
            parts.push(fence(cl));
        }
        hbox(parts)
    };
    match env {
        Env::Delimited(o, c) => fenced(Some(o), Some(c), grid),
        Env::Cases => fenced(Some('{'), None, grid),
        _ => grid,
    }
}

/// Typesets `node` at `size` points: in display style (on a line of its
/// own) or text style (in a line of text).
pub fn typeset(font: &MathFont, node: &Node, size: f32, display: bool) -> MathBox {
    typeset_with(Ctx { font, size, edit: false }, node, display)
}

/// Like [`typeset`], for the visual editor: also [`Piece::Mark`]s where the
/// caret can go and [`Piece::Slot`]s for empty boxes.
pub fn typeset_edit(font: &MathFont, node: &Node, size: f32, display: bool) -> MathBox {
    typeset_with(Ctx { font, size, edit: true }, node, display)
}

fn typeset_with(cx: Ctx, node: &Node, display: bool) -> MathBox {
    let level = if display { Level::Display } else { Level::Text };
    let mut b = layout(&cx, node, Style { level, cramped: false });
    b.width = b.width.max(0.0);
    b
}
