use crate::Fonts;
use xenify_wp_doc::{
    Align, DEFAULT_TAB, Document, Paragraph, ResolvedChar, ResolvedPara, Rgba, TabKind, TabLeader, TabStop,
};
use kurbo::Rect;
use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontData, FontFamily, FontStyle, FontWeight, Layout, LineHeight,
    PositionedLayoutItem, Selection, StyleProperty,
};
use std::hash::{Hash, Hasher};
use std::ops::Range;

/// A glyph placed in paragraph space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// Something to draw, in paragraph space.
#[derive(Clone)]
pub enum Item {
    Glyphs {
        font: FontData,
        size: f32,
        coords: Vec<i16>,
        glyphs: Vec<Glyph>,
        color: Rgba,
        /// Synthetic oblique angle in degrees (italic of a font without one).
        skew: Option<f32>,
        /// The text these glyphs draw (for PDF text extraction).
        text: Range<usize>,
        /// Per glyph, the characters it stands for (empty for the extra
        /// glyphs of a cluster): searchable, copyable text in exports.
        chars: Vec<String>,
        line: usize,
    },
    Rect {
        rect: Rect,
        color: Rgba,
        line: usize,
    },
    /// An emoji drawn as a picture (see [`crate::emoji`]) filling `rect`.
    Emoji {
        rect: Rect,
        glyph: String,
        line: usize,
    },
}

impl Item {
    pub fn line(&self) -> usize {
        match self {
            Item::Glyphs { line, .. } | Item::Rect { line, .. } | Item::Emoji { line, .. } => *line,
        }
    }
}

/// A line's vertical extent and baseline, in paragraph space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBox {
    pub top: f32,
    pub bottom: f32,
    pub baseline: f32,
    pub text: (usize, usize),
}

/// One laid-out paragraph.
pub struct ParaLayout {
    pub id: xenify_wp_doc::NodeId,
    pub layout: Layout<Rgba>,
    /// Length of the paragraph's text (an empty paragraph is laid out as one
    /// space so it has a line to put the caret on).
    pub text_len: usize,
    /// Left indent: parley's x = 0 is here.
    pub indent: f32,
    /// Width of the column the paragraph was laid out in.
    pub width: f32,
    pub height: f32,
    pub space_before: f32,
    pub space_after: f32,
    pub keep_with_next: bool,
    pub keep_together: bool,
    pub page_break_before: bool,
    pub column_break_before: bool,
    pub widow_control: bool,
    /// The list the paragraph is an item of (items of one list sit closer together).
    pub list: Option<String>,
    /// Paragraph style id (consecutive Code paragraphs sit flush).
    pub style: String,
    /// Where clicking it goes (contents entries: their heading).
    pub link: Option<xenify_wp_doc::NodeId>,
    /// The generated list (contents, bibliography…) this entry belongs to:
    /// right-clicking it offers the list's settings.
    pub generated: Option<xenify_wp_doc::NodeId>,
    pub lines: Vec<LineBox>,
    /// Background (highlights), then glyphs and decorations.
    pub items: Vec<Item>,
    /// Drawn on screen only, never printed or exported: hints and
    /// markers for broken fields.
    pub screen: Vec<Item>,
}

impl ParaLayout {
    /// The byte offset nearest to paragraph-space point (x, y).
    pub fn hit(&self, x: f32, y: f32) -> usize {
        let c = Cursor::from_point(&self.layout, x - self.indent, y);
        c.index().min(self.text_len)
    }

    fn cursor(&self, offset: usize, upstream: bool) -> Cursor {
        let affinity = if upstream { Affinity::Upstream } else { Affinity::Downstream };
        Cursor::from_byte_index(&self.layout, offset.min(self.text_len), affinity)
    }

    /// The caret at `offset`, in paragraph space.
    pub fn caret(&self, offset: usize, upstream: bool) -> Rect {
        let b = self.cursor(offset, upstream).geometry(&self.layout, 1.0);
        Rect::new(b.x0 + self.indent as f64, b.y0, b.x1 + self.indent as f64, b.y1)
    }

    /// The line the caret at `offset` is on.
    pub fn line_of(&self, offset: usize, upstream: bool) -> usize {
        let c = self.caret(offset, upstream).center().y as f32;
        self.lines.iter().position(|l| c >= l.top && c < l.bottom).unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// Boxes covering the text in `range`, in paragraph space.
    pub fn range_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let (s, e) = (range.start.min(self.text_len), range.end.min(self.text_len));
        let sel = Selection::new(self.cursor(s, false), self.cursor(e, true));
        sel.geometry(&self.layout)
            .into_iter()
            .map(|(b, _)| Rect::new(b.x0 + self.indent as f64, b.y0, b.x1 + self.indent as f64, b.y1))
            .collect()
    }

    /// The start or end of the visual line holding `offset`.
    pub fn line_edge(&self, offset: usize, upstream: bool, end: bool) -> usize {
        let sel = Selection::from_byte_index(&self.layout, offset.min(self.text_len), affinity(upstream));
        let moved = if end { sel.line_end(&self.layout, false) } else { sel.line_start(&self.layout, false) };
        moved.focus().index().min(self.text_len)
    }
}

fn affinity(upstream: bool) -> Affinity {
    if upstream { Affinity::Upstream } else { Affinity::Downstream }
}

/// Hash of everything the paragraph's layout depends on.
pub(crate) fn cache_key(doc: &Document, p: &Paragraph, width: f32, label: Option<&str>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.text.hash(&mut h);
    p.style.hash(&mut h);
    width.to_bits().hash(&mut h);
    label.hash(&mut h);
    format!("{:?}", p.runs).hash(&mut h);
    format!("{:?}", doc.resolve_para(p)).hash(&mut h);
    for (at, text, sup, hint) in field_texts(doc, p) {
        (at, text, sup, hint).hash(&mut h);
    }
    p.props.break_change.hash(&mut h);
    // Comments drawn highlighted only while open.
    for r in &p.runs {
        for c in &r.fmt.comments {
            doc.meta.comments.iter().find(|x| x.id == *c).map(|x| x.resolved).hash(&mut h);
        }
    }
    h.finish()
}

/// Links look like links unless formatted otherwise.
const LINK: Rgba = Rgba::rgb(0x1a, 0x5f, 0xd0);

fn char_with(base: &ResolvedChar, fmt: &xenify_wp_doc::CharFmt) -> ResolvedChar {
    let mut c = base.with(fmt);
    if fmt.link.is_some() {
        if fmt.color.is_none() {
            c.color = LINK;
        }
        if fmt.underline.is_none() {
            c.underline = true;
        }
    }
    // Superscript and subscript are smaller (and moved when drawn).
    if matches!(fmt.vert, Some(xenify_wp_doc::VertAlign::Superscript | xenify_wp_doc::VertAlign::Subscript)) {
        c.size *= 0.65;
    }
    // Tracked changes: underlined (inserted) or struck through (deleted), in the author's colour.
    if let Some(ch) = &fmt.change {
        c.color = author_color(&ch.author);
        match ch.kind {
            xenify_wp_doc::review::ChangeKind::Inserted => c.underline = true,
            xenify_wp_doc::review::ChangeKind::Deleted => c.strike = true,
        }
    }
    // Open comments: a pale highlight under the text they're about.
    if c.highlight.is_none() && fmt.comments.iter().any(|id| OPEN_COMMENTS.with(|o| o.borrow().contains(id))) {
        c.highlight = Some(COMMENT_HIGHLIGHT);
    }
    c
}

/// The highlight under text that has an open comment.
pub const COMMENT_HIGHLIGHT: Rgba = Rgba([255, 226, 140, 150]);

/// A reviewer's colour (the same name always gets the same one).
pub fn author_color(name: &str) -> Rgba {
    const COLOURS: [Rgba; 6] = [
        Rgba::rgb(0xb4, 0x23, 0x18),
        Rgba::rgb(0x1f, 0x5f, 0xbf),
        Rgba::rgb(0x2e, 0x7d, 0x32),
        Rgba::rgb(0x8e, 0x24, 0xaa),
        Rgba::rgb(0xc4, 0x5a, 0x00),
        Rgba::rgb(0x00, 0x79, 0x7a),
    ];
    let h = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    COLOURS[h as usize % COLOURS.len()]
}

thread_local! {
    /// The document's open comments while its paragraphs are laid out.
    static OPEN_COMMENTS: std::cell::RefCell<std::collections::HashSet<xenify_wp_doc::NodeId>> =
        std::cell::RefCell::new(Default::default());
}

fn family_stack(font: &str) -> String {
    let f = font.trim();
    match f {
        "" => "\"Inter\", \"STIX Two Math\", sans-serif".into(),
        "serif" | "sans-serif" | "monospace" | "cursive" | "system-ui" => format!("{f}, \"Inter\", \"STIX Two Math\""),
        _ => format!("\"{}\", \"Inter\", \"STIX Two Math\", sans-serif", f.replace('"', "")),
    }
}

fn push_char<'a>(props: &mut Vec<StyleProperty<'a, Rgba>>, c: &ResolvedChar, base: Option<&ResolvedChar>) {
    let differs = |f: &dyn Fn(&ResolvedChar) -> bool| base.is_none_or(f);
    if differs(&|b| b.font != c.font) {
        props.push(StyleProperty::FontFamily(FontFamily::Source(family_stack(&c.font).into())));
    }
    if differs(&|b| b.size != c.size) {
        props.push(StyleProperty::FontSize(c.size.max(1.0)));
    }
    if differs(&|b| b.bold != c.bold) {
        props.push(StyleProperty::FontWeight(if c.bold { FontWeight::BOLD } else { FontWeight::NORMAL }));
    }
    if differs(&|b| b.italic != c.italic) {
        props.push(StyleProperty::FontStyle(if c.italic { FontStyle::Italic } else { FontStyle::Normal }));
    }
    if differs(&|b| b.color != c.color) {
        props.push(StyleProperty::Brush(c.color));
    }
    if differs(&|b| b.underline != c.underline) {
        props.push(StyleProperty::Underline(c.underline));
    }
    if differs(&|b| b.strike != c.strike) {
        props.push(StyleProperty::Strikethrough(c.strike));
    }
}

/// Where one line may go: given the line's proposed top (paragraph space)
/// and its expected height, the top to use (pushed down past obstacles),
/// and the free span's left edge and width, both relative to the column.
pub(crate) type LineShape<'a> = &'a dyn Fn(f32, f32) -> (f32, f32, f32);

pub(crate) fn layout_paragraph(
    fonts: &mut Fonts,
    doc: &Document,
    p: &Paragraph,
    width: f32,
    label: Option<&str>,
) -> ParaLayout {
    layout_paragraph_shaped(fonts, doc, p, width, label, None)
}

/// Like [`layout_paragraph`], with lines placed around obstacles (text wrap).
pub(crate) fn layout_paragraph_shaped(
    fonts: &mut Fonts,
    doc: &Document,
    p: &Paragraph,
    width: f32,
    label: Option<&str>,
    shape: Option<LineShape>,
) -> ParaLayout {
    let mut style = doc.resolve_para(p);
    // A display equation alone on its line is centred.
    if p.text == xenify_wp_doc::Field::STR
        && matches!(
            p.fmt_at(xenify_wp_doc::Field::CHAR.len_utf8()).field,
            Some(xenify_wp_doc::Field::Equation { display: true, .. })
        )
        && p.props.align.is_none()
    {
        style.align = xenify_wp_doc::Align::Center;
    }
    let text: &str = if p.text.is_empty() { " " } else { &p.text };
    if p.runs.iter().any(|r| !r.fmt.comments.is_empty()) {
        OPEN_COMMENTS.with(|o| *o.borrow_mut() = xenify_wp_doc::review::open_comments(doc));
    }
    let mut highlights = Vec::new();
    // Form controls' text (tinted on screen); prompts are grey.
    let mut controls: Vec<Range<usize>> = Vec::new();
    let mut prompts: Vec<Range<usize>> = Vec::new();
    for (range, fmt) in p.run_ranges() {
        if let Some(xenify_wp_doc::Field::Badge { color, .. }) = &fmt.field {
            highlights.push((range.clone(), xenify_wp_doc::badge_colors(color).0));
        } else if let Some(h) = char_with(&style.char, fmt).highlight {
            highlights.push((range.clone(), h));
        }
        if let Some(c) = fmt.control.and_then(|id| doc.control(id)) {
            // A template's fill-in box, once filled, is just text.
            if c.tag == "xenify-fill" && !c.placeholder {
                continue;
            }
            controls.push(range.clone());
            if c.placeholder {
                prompts.push(range.clone());
            }
        }
    }
    // Fields (note numbers, citations…): a box as wide as the text shown;
    // equations typeset (as tall as they are).
    let fields: Vec<(usize, Vec<Item>, f32, f32, bool, f32)> = field_texts(doc, p)
        .into_iter()
        .map(|(at, text, sup, hint)| {
            let c = char_with(&style.char, &p.fmt_at(at + 1));
            if let Some(xenify_wp_doc::Field::Equation { latex, display, .. }) = p.fmt_at(at + 1).field
                && let Some(t) = crate::equation::typeset(&latex, display, c.size, c.color)
            {
                // A display equation sits on its own line: raised so nothing hangs below it.
                let (rise, height) = if display { (t.descent, t.ascent + t.descent) } else { (0.0, t.ascent) };
                return (at, t.items, t.width + c.size * 0.1, rise, false, height);
            }
            let c = if sup { ResolvedChar { size: c.size * 0.62, ..c } } else { c };
            // A badge: small bold capitals in its colour (the background is a highlight, below).
            let c = match p.fmt_at(at + 1).field {
                Some(xenify_wp_doc::Field::Badge { color, .. }) => {
                    ResolvedChar { size: c.size * 0.78, bold: true, color: xenify_wp_doc::badge_colors(&color).1, ..c }
                }
                _ => c,
            };
            // A broken field: a red marker on screen only.
            let c = if hint { ResolvedChar { color: Rgba::rgb(0xc6, 0x28, 0x28), ..c } } else { c };
            let rise = if sup { c.size * 0.6 } else { 0.0 };
            let (items, w) = layout_label(fonts, &text, &c);
            (at, items, w, rise, hint, 0.0)
        })
        .collect();
    // Each shrunk character holds its line open as text of its size would
    // (a line of only a field or emoji keeps its height and baseline).
    let field_boxes: Vec<(usize, f32, f32, f32)> = fields
        .iter()
        .map(|(at, _, w, _, _, h)| {
            if *h > 0.0 {
                return (*at, *w, *h, 0.0);
            }
            let (h, lh) = strut(fonts, &style, &char_with(&style.char, &p.fmt_at(at + 1)));
            (*at, *w, h, lh)
        })
        .collect();
    // Emoji: a box in the line, drawn as a picture; the characters themselves
    // are shaped tiny so they take no room (and stay in the text for copying).
    let emojis: Vec<(Range<usize>, f32, f32, f32)> = crate::emoji::ranges(&p.text)
        .into_iter()
        .map(|(r, _)| {
            let c = char_with(&style.char, &p.fmt_at(r.start + 1));
            let (h, lh) = strut(fonts, &style, &c);
            (r, c.size, h, lh)
        })
        .collect();
    // Tabs: each gets a box, widened until the text after it reaches its stop.
    let tabs: Vec<usize> = p.text.match_indices('\t').map(|(i, _)| i).collect();
    let mut widths = vec![0.0f32; tabs.len()];
    let mut layout = shape_text(
        fonts,
        p,
        &style,
        text,
        &InlineBoxes { tabs: &tabs, widths: &widths, fields: &field_boxes, emojis: &emojis, grey: &prompts },
    );
    // Wrapped round a text box, the first line may start further right; the
    // list label goes with it.
    let shaped = shape.is_some();
    break_and_align(&mut layout, &style, width, shape);
    let mut leaders = Vec::new();
    for _ in 0..4 {
        if tabs.is_empty() {
            break;
        }
        let (want, lead) = tab_widths(&layout, p, &style, &tabs, &widths);
        leaders = lead;
        if want.iter().zip(&widths).all(|(a, b)| (a - b).abs() < 0.25) {
            break;
        }
        widths = want;
        layout = shape_text(
            fonts,
            p,
            &style,
            text,
            &InlineBoxes { tabs: &tabs, widths: &widths, fields: &field_boxes, emojis: &emojis, grey: &prompts },
        );
        break_and_align(&mut layout, &style, width, shape);
    }

    let indent = style.indent_left;
    let mut lines = Vec::new();
    let mut items = Vec::new();
    let pl = ParaLayout {
        id: p.id,
        layout,
        text_len: p.text.len(),
        indent,
        width,
        height: 0.0,
        space_before: style.space_before,
        space_after: style.space_after,
        keep_with_next: style.keep_with_next,
        keep_together: style.keep_together,
        page_break_before: style.page_break_before,
        column_break_before: style.column_break_before,
        widow_control: style.widow_control,
        list: p.list.as_ref().map(|l| l.list.clone()),
        style: p.style.clone(),
        link: None,
        generated: None,
        lines: Vec::new(),
        items: Vec::new(),
        screen: Vec::new(),
    };
    for (range, color) in highlights {
        for r in pl.range_rects(range) {
            let line = lines_hint(&pl.layout, r.center().y as f32);
            items.push(Item::Rect { rect: r, color, line });
        }
    }
    let mut decorations = Vec::new();
    for (li, line) in pl.layout.lines().enumerate() {
        let m = line.metrics();
        let r = line.text_range();
        lines.push(LineBox {
            top: m.block_min_coord,
            bottom: m.block_max_coord,
            baseline: m.baseline,
            text: (r.start, r.end),
        });
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            let rr = run.run();
            // Superscript up, subscript down (relative to the smaller size).
            let shift = match p.fmt_at(rr.text_range().start + 1).vert {
                Some(xenify_wp_doc::VertAlign::Superscript) => -rr.font_size() * 0.55,
                Some(xenify_wp_doc::VertAlign::Subscript) => rr.font_size() * 0.3,
                _ => 0.0,
            };
            let glyphs: Vec<Glyph> =
                run.positioned_glyphs().map(|g| Glyph { id: g.id, x: g.x + indent, y: g.y + shift }).collect();
            let st = run.style();
            let rm = rr.metrics();
            let x0 = (run.offset() + indent) as f64;
            let x1 = x0 + run.advance() as f64;
            if let Some(u) = &st.underline {
                let size = u.size.unwrap_or(rm.underline_size).max(0.5);
                let top = run.baseline() - u.offset.unwrap_or(rm.underline_offset);
                decorations.push(Item::Rect {
                    rect: Rect::new(x0, top as f64, x1, (top + size) as f64),
                    color: u.brush,
                    line: li,
                });
            }
            if let Some(s) = &st.strikethrough {
                let size = s.size.unwrap_or(rm.strikethrough_size).max(0.5);
                let top = run.baseline() - s.offset.unwrap_or(rm.strikethrough_offset);
                decorations.push(Item::Rect {
                    rect: Rect::new(x0, top as f64, x1, (top + size) as f64),
                    color: s.brush,
                    line: li,
                });
            }
            let text = rr.text_range();
            let mut chars = glyph_chars(&run, &glyphs, if p.text.is_empty() { "" } else { &p.text });
            // A tab draws nothing (its space is the tab box): drop its glyph.
            let mut glyphs = glyphs;
            if chars.iter().any(|c| c.contains(['\t', '\u{FFFC}'])) {
                let keep: Vec<bool> = chars.iter().map(|c| !c.contains(['\t', '\u{FFFC}'])).collect();
                let mut k = keep.iter();
                glyphs.retain(|_| *k.next().unwrap_or(&true));
                let mut k = keep.iter();
                chars.retain(|_| *k.next().unwrap_or(&true));
            }
            items.push(Item::Glyphs {
                font: rr.font().clone(),
                size: rr.font_size(),
                coords: rr.normalized_coords().to_vec(),
                glyphs,
                color: st.brush,
                skew: rr.synthesis().skew(),
                text: text.start.min(pl.text_len)..text.end.min(pl.text_len),
                chars,
                line: li,
            });
        }
    }
    if p.text.is_empty() {
        // The placeholder space draws nothing.
        items.retain(|i| !matches!(i, Item::Glyphs { .. }));
    }
    items.extend(decorations);
    leader_items(&leaders, &lines, indent, style.char.color, &mut items);
    // Where the boxes set into the lines (fields, emoji) ended up.
    let boxes: std::collections::HashMap<u64, (f32, f32)> = pl
        .layout
        .lines()
        .flat_map(|l| {
            l.items()
                .filter_map(|it| match it {
                    PositionedLayoutItem::InlineBox(b) => Some((b.id, (b.x, b.y + b.height / 2.0))),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let placed = |id: u64, at: usize| boxes.get(&id).copied().unwrap_or_else(|| spot(&pl.layout, at, p.text.len()));
    for (i, (r, size, _, _)) in emojis.iter().enumerate() {
        let (x, top) = placed(2_000_000 + i as u64, r.start);
        let Some((li, line)) = lines.iter().enumerate().find(|(_, l)| top >= l.top - 0.5 && top < l.bottom) else {
            continue;
        };
        let (x0, base) = ((x + indent) as f64 + (size * EMOJI_GAP) as f64, line.baseline as f64);
        let s = (*size * EMOJI_SCALE) as f64;
        items.push(Item::Emoji {
            rect: Rect::new(x0, base - s * 0.82, x0 + s, base + s * 0.18),
            glyph: p.text[r.clone()].to_string(),
            line: li,
        });
    }
    let mut screen = Vec::new();
    // A faint tint marks where to fill in a form (screen only).
    for r in controls {
        for rect in pl.range_rects(r) {
            let line = lines_hint(&pl.layout, rect.center().y as f32);
            screen.push(Item::Rect { rect, color: Rgba([0x3b, 0x82, 0xf6, 0x22]), line });
        }
    }
    for (i, (at, glyph_items, _, rise, hint, _)) in fields.iter().enumerate() {
        let (x, top) = placed(1_000_000 + i as u64, *at);
        let Some((li, line)) = lines.iter().enumerate().find(|(_, l)| top >= l.top - 0.5 && top < l.bottom) else {
            continue;
        };
        for it in glyph_items {
            let mut it = it.clone();
            match &mut it {
                Item::Glyphs { glyphs, line: l, .. } => {
                    *l = li;
                    for g in glyphs.iter_mut() {
                        *g = Glyph { id: g.id, x: g.x + x + indent, y: g.y + line.baseline - rise };
                    }
                }
                // An equation's rules (fraction bars).
                Item::Rect { rect, line: l, .. } => {
                    *l = li;
                    *rect = *rect + kurbo::Vec2::new((x + indent) as f64, (line.baseline - rise) as f64);
                }
                _ => {}
            }
            if *hint { screen.push(it) } else { items.push(it) }
        }
    }
    // A tracked paragraph break: a small ¶ after the text, struck through
    // (deleted) or underlined (inserted), in the author's colour.
    if let Some(ch) = &p.props.break_change
        && let Some((li, line)) = lines.len().checked_sub(1).map(|i| (i, &lines[i]))
    {
        let x = if p.text.is_empty() { 0.0 } else { spot(&pl.layout, p.text.len(), p.text.len()).0 };
        let c = ResolvedChar { color: author_color(&ch.author), size: style.char.size * 0.8, ..style.char.clone() };
        let (marks, w) = layout_label(fonts, "¶", &c);
        let x0 = x + indent + 1.5;
        for mut it in marks {
            if let Item::Glyphs { glyphs, line: l, .. } = &mut it {
                *l = li;
                for g in glyphs.iter_mut() {
                    *g = Glyph { id: g.id, x: g.x + x0, y: g.y + line.baseline };
                }
            }
            items.push(it);
        }
        let y = match ch.kind {
            xenify_wp_doc::review::ChangeKind::Deleted => line.baseline - c.size * 0.3,
            xenify_wp_doc::review::ChangeKind::Inserted => line.baseline + 1.5,
        };
        items.push(Item::Rect {
            rect: Rect::new(x0 as f64, y as f64, (x0 + w) as f64, (y + 0.8) as f64),
            color: c.color,
            line: li,
        });
    }
    box_items(&style, &lines, width, &mut items);
    // The list label hangs left of the first line, on its baseline.
    if let (Some(label), Some(first)) = (label.filter(|l| !l.is_empty()), lines.first()) {
        let c = p.runs.first().map_or(style.char.clone(), |r| style.char.with(&r.fmt));
        // List labels hang by the list's amount; a note's number sits at the margin.
        let shift = if shaped { pl.layout.lines().next().map_or(0.0, |l| l.metrics().inline_min_coord) } else { 0.0 };
        let x0 = if p.list.is_some() { indent - style.hanging + shift } else { shift };
        // A checklist box: an outlined square (filled rectangles, so PDF
        // draws it too), with a tick when checked.
        let checked = label == xenify_wp_doc::lists::CHECKED;
        let label = if checked || label == xenify_wp_doc::lists::UNCHECKED {
            let side = (c.size * 0.72) as f64;
            let t = (c.size / 14.0).max(0.8) as f64;
            let (x, y) = (x0 as f64, first.baseline as f64 - side);
            let ink = if checked { Rgba([47, 111, 237, 255]) } else { c.color };
            for r in [
                Rect::new(x, y, x + side, y + t),
                Rect::new(x, y + side - t, x + side, y + side),
                Rect::new(x, y, x + t, y + side),
                Rect::new(x + side - t, y, x + side, y + side),
            ] {
                items.push(Item::Rect { rect: r, color: ink, line: 0 });
            }
            if !checked {
                return finish(pl, lines, items, screen);
            }
            "\u{2713}"
        } else {
            label
        };
        let (label_items, _) = layout_label(fonts, label, &c);
        let x0 = if checked { x0 + c.size * 0.08 } else { x0 };
        for mut it in label_items {
            if let Item::Glyphs { glyphs, .. } = &mut it {
                for g in glyphs.iter_mut() {
                    *g = Glyph { id: g.id, x: g.x + x0, y: g.y + first.baseline };
                }
            }
            items.push(it);
        }
    }
    finish(pl, lines, items, screen)
}

fn finish(mut pl: ParaLayout, lines: Vec<LineBox>, items: Vec<Item>, screen: Vec<Item>) -> ParaLayout {
    pl.height = lines.last().map_or(0.0, |l| l.bottom);
    pl.lines = lines;
    pl.items = items;
    pl.screen = screen;
    pl
}

/// The boxes set into a paragraph's line: tabs (each `widths[i]` wide),
/// fields (offset, width) and emoji (range, font size).
struct InlineBoxes<'a> {
    tabs: &'a [usize],
    widths: &'a [f32],
    /// (offset, width, height, line height of the shrunk character; 0 = none).
    fields: &'a [(usize, f32, f32, f32)],
    /// (range, font size, height, line height of the shrunk characters).
    emojis: &'a [(Range<usize>, f32, f32, f32)],
    /// Text drawn grey (a form's prompt).
    grey: &'a [Range<usize>],
}

/// Shapes the paragraph's text with its formatting and its [`InlineBoxes`].
fn shape_text(fonts: &mut Fonts, p: &Paragraph, style: &ResolvedPara, text: &str, boxes: &InlineBoxes) -> Layout<Rgba> {
    let mut builder = fonts.lcx.ranged_builder(&mut fonts.cx, text, 1.0, false);
    let mut defaults = Vec::new();
    push_char(&mut defaults, &style.char, None);
    for prop in defaults {
        builder.push_default(prop);
    }
    builder.push_default(StyleProperty::LineHeight(match style.line_exact {
        Some(pt) => LineHeight::Absolute(pt.max(1.0)),
        None => LineHeight::FontSizeRelative(style.line_height),
    }));
    // A word too long for the line (a link, a long name) breaks where it has to,
    // rather than running past the column or out of a text box.
    builder.push_default(StyleProperty::OverflowWrap(parley::style::OverflowWrap::Anywhere));
    for (range, fmt) in p.run_ranges() {
        if fmt.is_empty() {
            continue;
        }
        let c = char_with(&style.char, fmt);
        let mut props = Vec::new();
        push_char(&mut props, &c, Some(&style.char));
        for prop in props {
            builder.push(prop, range.clone());
        }
    }
    let InlineBoxes { tabs, widths, fields, emojis, grey } = boxes;
    for r in grey.iter() {
        builder.push(StyleProperty::Brush(Rgba::rgb(0x80, 0x80, 0x80)), r.clone());
    }
    for (i, (&at, &w)) in tabs.iter().zip(widths.iter()).enumerate() {
        builder.push_inline_box(parley::InlineBox {
            id: i as u64,
            kind: parley::InlineBoxKind::InFlow,
            index: at,
            width: w,
            height: 0.0,
        });
    }
    for (i, (at, w, h, lh)) in fields.iter().enumerate() {
        // The field's own character takes no room: its box does.
        let r = *at..*at + xenify_wp_doc::Field::CHAR.len_utf8();
        builder.push(StyleProperty::FontSize(0.01), r.clone());
        if *lh > 0.0 {
            builder.push(StyleProperty::LineHeight(LineHeight::Absolute(*lh)), r);
        }
        builder.push_inline_box(parley::InlineBox {
            id: 1_000_000 + i as u64,
            kind: parley::InlineBoxKind::InFlow,
            index: *at,
            width: *w,
            height: *h,
        });
    }
    for (i, (r, size, h, lh)) in emojis.iter().enumerate() {
        builder.push(StyleProperty::FontSize(0.01), r.clone());
        builder.push(StyleProperty::LineHeight(LineHeight::Absolute(*lh)), r.clone());
        builder.push_inline_box(parley::InlineBox {
            id: 2_000_000 + i as u64,
            kind: parley::InlineBoxKind::InFlow,
            index: r.start,
            width: size * (EMOJI_SCALE + 2.0 * EMOJI_GAP),
            height: *h,
        });
    }
    let mut layout = builder.build(text);
    if style.first_line != 0.0 {
        layout.set_text_indent(style.first_line, parley::IndentOptions::default());
    }
    layout
}

/// A strut for a character shrunk to make room for a box: the box height and
/// line height that give its line the height and baseline text in `c` would.
/// With the shrunk character's own ascent and descent ~0, a box of
/// `ascent - descent` puts the baseline where the text's would be.
fn strut(fonts: &mut Fonts, style: &ResolvedPara, c: &ResolvedChar) -> (f32, f32) {
    let mut b = fonts.lcx.ranged_builder(&mut fonts.cx, "x", 1.0, false);
    let mut props = Vec::new();
    push_char(&mut props, c, None);
    for prop in props {
        b.push_default(prop);
    }
    let mut l = b.build("x");
    l.break_all_lines(None);
    let (ascent, descent) = l.lines().next().map_or((c.size * 0.8, c.size * 0.2), |line| {
        let m = line.metrics();
        (m.ascent, m.descent)
    });
    let lh = match style.line_exact {
        Some(pt) => pt.max(1.0),
        None => style.line_height * c.size,
    };
    ((ascent - descent).max(0.0), lh)
}

/// Space each side of an emoji, as a share of the font size.
const EMOJI_GAP: f32 = 0.06;
/// An emoji's picture size, as a share of the font size (as emoji fonts draw them).
const EMOJI_SCALE: f32 = 1.15;

/// Breaks the paragraph into lines (around obstacles with `shape`) and aligns them.
fn break_and_align(layout: &mut Layout<Rgba>, style: &ResolvedPara, width: f32, shape: Option<LineShape>) {
    let avail = (width - style.indent_left - style.indent_right).max(18.0);
    match shape {
        None => layout.break_all_lines(Some(avail)),
        Some(shape) => {
            let est = style.char.size * style.line_height;
            let mut b = layout.break_lines();
            b.state_mut().set_layout_max_advance(avail);
            loop {
                let top = b.committed_y() as f32;
                let (y, x, w) = shape(top, est);
                // Column span → the paragraph's own span (inside its indents).
                let left = (x - style.indent_left).max(0.0);
                let right = (width - x - w - style.indent_right).max(0.0);
                let line_w = (avail - left - right).max(18.0).min(avail);
                let st = b.state_mut();
                st.set_line_y(y.max(top) as f64);
                st.set_line_x(left.min(avail - line_w));
                st.set_line_max_advance(line_w);
                if b.break_next().is_none() {
                    break;
                }
            }
            b.finish();
        }
    }
    let align = match style.align {
        Align::Left => Alignment::Start,
        Align::Center => Alignment::Center,
        Align::Right => Alignment::End,
        Align::Justify => Alignment::Justify,
    };
    layout.align(align, AlignmentOptions::default());
}

/// Where the caret at byte `at` is: x and top, in layout space.
fn spot(layout: &Layout<Rgba>, at: usize, len: usize) -> (f32, f32) {
    let g = Cursor::from_byte_index(layout, at.min(len), Affinity::Downstream).geometry(layout, 1.0);
    (g.x0 as f32, g.y0 as f32)
}

/// A tab's leader to draw: from x0 to x1 at the line whose top is `top`.
pub(crate) struct Leader {
    x0: f32,
    x1: f32,
    top: f32,
    kind: TabLeader,
}

/// The tab boxes' widths that put the text after each tab on its stop
/// (from the current layout, which used `widths`), and the leaders to draw.
fn tab_widths(
    layout: &Layout<Rgba>,
    p: &Paragraph,
    style: &ResolvedPara,
    tabs: &[usize],
    widths: &[f32],
) -> (Vec<f32>, Vec<Leader>) {
    let len = p.text.len();
    let mut out = widths.to_vec();
    let mut leaders = Vec::new();
    for (i, &t) in tabs.iter().enumerate() {
        let (before, top) = spot(layout, t, len);
        let (after, top_after) = spot(layout, t + 1, len);
        if (top - top_after).abs() > 0.5 {
            continue; // The line broke at the tab.
        }
        // Positions from the left margin.
        let m = before + style.indent_left;
        let stop = style.tabs.iter().find(|s| s.pos > m + 0.5).copied().unwrap_or(TabStop {
            pos: ((m + 0.5) / DEFAULT_TAB).floor() * DEFAULT_TAB + DEFAULT_TAB,
            ..Default::default()
        });
        // The text after the tab, up to the next tab (on this line).
        let seg_end = tabs.get(i + 1).copied().unwrap_or(len);
        let (end_x, end_top) = spot(layout, seg_end, len);
        let seg_w = if (end_top - top).abs() < 0.5 { (end_x - after).max(0.0) } else { 0.0 };
        let lead_in = match stop.kind {
            TabKind::Left => 0.0,
            TabKind::Center => seg_w / 2.0,
            TabKind::Right => seg_w,
            TabKind::Decimal => p.text[t + 1..seg_end]
                .find(['.', ','])
                .map_or(seg_w, |d| (spot(layout, t + 1 + d, len).0 - after).max(0.0)),
        };
        let target = (stop.pos - lead_in - style.indent_left).max(before);
        out[i] = (widths[i] + target - after).max(0.0);
        if stop.leader != TabLeader::None {
            leaders.push(Leader { x0: before, x1: after, top, kind: stop.leader });
        }
    }
    (out, leaders)
}

/// Paragraph shading (behind the text) and borders, drawn line by line so a
/// paragraph split across pages is boxed on each part.
fn box_items(style: &ResolvedPara, lines: &[LineBox], width: f32, items: &mut Vec<Item>) {
    let b = &style.borders;
    if style.shading.is_none() && b.is_empty() || lines.is_empty() {
        return;
    }
    // A little room between the text and the lines.
    let pad = 4.0;
    let x0 = (style.indent_left - pad) as f64;
    let x1 = (width - style.indent_right + pad) as f64;
    let last = lines.len() - 1;
    let mut back = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        let top = if li == 0 { l.top - 2.0 } else { l.top } as f64;
        let bottom = if li == last { l.bottom + 2.0 } else { l.bottom } as f64;
        if let Some(c) = style.shading {
            back.push(Item::Rect { rect: Rect::new(x0, top, x1, bottom), color: c, line: li });
        }
        let mut edge = |r: Rect, line: Option<xenify_wp_doc::Border>| {
            if let Some(line) = line.filter(|l| l.width > 0.0) {
                items.push(Item::Rect { rect: r, color: line.color, line: li });
            }
        };
        let w = |line: Option<xenify_wp_doc::Border>| line.map_or(0.0, |l| l.width as f64);
        edge(Rect::new(x0 - w(b.left), top, x0, bottom), b.left);
        edge(Rect::new(x1, top, x1 + w(b.right), bottom), b.right);
        if li == 0 {
            edge(Rect::new(x0 - w(b.left), top - w(b.top), x1 + w(b.right), top), b.top);
        }
        if li == last {
            edge(Rect::new(x0 - w(b.left), bottom, x1 + w(b.right), bottom + w(b.bottom)), b.bottom);
        }
    }
    items.splice(0..0, back);
}

/// Dots, dashes or a line filling a tab's gap, on the line's baseline.
fn leader_items(leaders: &[Leader], lines: &[LineBox], indent: f32, color: Rgba, items: &mut Vec<Item>) {
    for l in leaders {
        let Some((li, line)) = lines.iter().enumerate().find(|(_, b)| l.top >= b.top - 0.5 && l.top < b.bottom) else {
            continue;
        };
        let (x0, x1) = ((l.x0 + indent + 2.0) as f64, (l.x1 + indent - 2.0) as f64);
        let y = line.baseline as f64;
        let mut push = |a: f64, b: f64, h: f64| {
            items.push(Item::Rect { rect: Rect::new(a, y - h, b, y), color, line: li });
        };
        match l.kind {
            TabLeader::Dots => {
                let mut x = x0;
                while x + 1.2 <= x1 {
                    push(x, x + 1.2, 1.2);
                    x += 4.0;
                }
            }
            TabLeader::Dashes => {
                let mut x = x0;
                while x + 3.0 <= x1 {
                    push(x, x + 3.0, 0.8);
                    x += 6.0;
                }
            }
            TabLeader::Line => push(x0, x1, 0.7),
            TabLeader::None => {}
        }
    }
}

fn lines_hint(layout: &Layout<Rgba>, y: f32) -> usize {
    layout.lines().position(|l| y >= l.metrics().block_min_coord && y < l.metrics().block_max_coord).unwrap_or(0)
}

/// A short single-line text (list label, page number) as glyph items with
/// the baseline at y = 0 and x from 0; returns them and the text's width.
pub(crate) fn layout_label(fonts: &mut Fonts, text: &str, c: &ResolvedChar) -> (Vec<Item>, f32) {
    let mut b = fonts.lcx.ranged_builder(&mut fonts.cx, text, 1.0, false);
    let mut props = Vec::new();
    push_char(&mut props, &ResolvedChar { underline: false, strike: false, ..c.clone() }, None);
    for prop in props {
        b.push_default(prop);
    }
    let mut l = b.build(text);
    l.break_all_lines(None);
    let mut items = Vec::new();
    let mut width = 0.0;
    if let Some(line) = l.lines().next() {
        let base = line.metrics().baseline;
        width = line.metrics().advance - line.metrics().trailing_whitespace;
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            let rr = run.run();
            items.push(Item::Glyphs {
                font: rr.font().clone(),
                size: rr.font_size(),
                coords: rr.normalized_coords().to_vec(),
                glyphs: run.positioned_glyphs().map(|g| Glyph { id: g.id, x: g.x, y: g.y - base }).collect(),
                color: run.style().brush,
                skew: rr.synthesis().skew(),
                text: 0..0,
                chars: glyph_chars(&run, &[], text),
                line: 0,
            });
        }
    }
    (items, width)
}

/// The characters each glyph of `run` draws (text of the first glyph of a
/// cluster; empty for the others). `glyphs` are the run's glyphs as placed
/// (empty: all of them).
fn glyph_chars(run: &parley::GlyphRun<'_, Rgba>, glyphs: &[Glyph], text: &str) -> Vec<String> {
    let all: Vec<(u32, String)> = run
        .run()
        .visual_clusters()
        .flat_map(|c| {
            let r = c.text_range();
            let s = text.get(r).unwrap_or("").to_string();
            c.glyphs().enumerate().map(move |(i, g)| (g.id, if i == 0 { s.clone() } else { String::new() }))
        })
        .collect();
    if glyphs.is_empty() {
        return all.into_iter().map(|a| a.1).collect();
    }
    let ids: Vec<u32> = glyphs.iter().map(|g| g.id).collect();
    let skip = (0..=all.len().saturating_sub(ids.len()))
        .find(|&k| all[k..].iter().zip(&ids).all(|(a, b)| a.0 == *b))
        .unwrap_or(0);
    (0..ids.len()).map(|i| all.get(skip + i).map_or(String::new(), |a| a.1.clone())).collect()
}

/// The fields in paragraph `p`: each one's offset, the text it shows, and
/// whether that's raised (note numbers).
/// (offset, text shown, raised?, broken: shown on screen only).
pub(crate) fn field_texts(doc: &Document, p: &Paragraph) -> Vec<(usize, String, bool, bool)> {
    use xenify_wp_doc::Field;
    let mut out = Vec::new();
    for (range, fmt) in p.run_ranges() {
        let Some(f) = &fmt.field else { continue };
        for (i, ch) in p.text[range.clone()].char_indices() {
            if ch != Field::CHAR {
                continue;
            }
            let mut hint = false;
            let (text, sup) = match f {
                Field::Note { id } => match note_numbers(doc).get(id) {
                    Some(n) => (n.clone(), true),
                    None => {
                        hint = true;
                        ("?".into(), true)
                    }
                },
                Field::Page => (
                    CURRENT_PAGE.with(|c| c.borrow().as_ref().map(|(n, _)| n.clone())).unwrap_or_else(|| {
                        PAGE_CONTEXT.with(|c| {
                            c.borrow().0.get(&p.id).map_or("#".into(), |n| {
                                let section = doc.section_of(p.id).and_then(|s| doc.sections.get(s));
                                section.map_or(n.to_string(), |s| s.page.number_format.format(*n))
                            })
                        })
                    }),
                    false,
                ),
                Field::Pages => (
                    match CURRENT_PAGE
                        .with(|c| c.borrow().as_ref().map(|(_, t)| *t))
                        .unwrap_or_else(|| PAGE_CONTEXT.with(|c| c.borrow().1))
                    {
                        0 => "#".into(),
                        n => n.to_string(),
                    },
                    false,
                ),
                Field::Date => (crate::paginate::today(), false),
                // Spaces either side so the colour reaches past the letters.
                Field::Badge { text, .. } => (format!(" {} ", text.to_uppercase()), false),
                Field::DateTime { format, fixed } => match fixed {
                    Some(text) => (text.clone(), false),
                    None => (xenify_wp_doc::datetime::format(format, &crate::paginate::now()), false),
                },
                Field::Title => (doc.meta.title.clone(), false),
                Field::Author => (doc.meta.author.clone(), false),
                Field::Citation { keys, locator } => {
                    hint = keys.iter().any(|k| !doc.meta.sources.iter().any(|s| &s.key == k));
                    (xenify_wp_doc::cite::citation(doc, keys, locator), false)
                }
                Field::Ref { target } => match doc.para(*target) {
                    Some(h) => (h.text.clone(), false),
                    None => {
                        hint = true;
                        ("⚠ heading deleted".into(), false)
                    }
                },
                // Shown as its LaTeX until equations are typeset.
                Field::Equation { latex, .. } => (latex.clone(), false),
                Field::Check { checked } => ((if *checked { "☒" } else { "☐" }).into(), false),
            };
            out.push((range.start + i, text, sup, hint));
        }
    }
    out
}

/// Note numbers, worked out once per document version.
fn note_numbers(doc: &Document) -> std::sync::Arc<std::collections::HashMap<xenify_wp_doc::NodeId, String>> {
    use std::cell::RefCell;
    use std::sync::Arc;
    type Memo = Option<((usize, u64, u64), Arc<std::collections::HashMap<xenify_wp_doc::NodeId, String>>)>;
    thread_local! {
        static MEMO: RefCell<Memo> = const { RefCell::new(None) };
    }
    let key = (doc as *const Document as usize, doc.revision, doc.next_id);
    MEMO.with(|m| {
        if let Some((k, v)) = &*m.borrow()
            && *k == key
        {
            return v.clone();
        }
        let v = Arc::new(xenify_wp_doc::notes::numbers(doc));
        *m.borrow_mut() = Some((key, v.clone()));
        v
    })
}

thread_local! {
    /// While paginating: the page each paragraph with a page field was on
    /// last time, and the number of pages.
    static PAGE_CONTEXT: std::cell::RefCell<(std::collections::HashMap<xenify_wp_doc::NodeId, u32>, u32)> =
        std::cell::RefCell::new(Default::default());
}

pub(crate) fn set_page_context(pages: &std::collections::HashMap<xenify_wp_doc::NodeId, u32>, total: u32) {
    PAGE_CONTEXT.with(|c| *c.borrow_mut() = (pages.clone(), total));
}

thread_local! {
    /// While laying out a page's header or footer: its page number (as
    /// shown) and the number of pages.
    static CURRENT_PAGE: std::cell::RefCell<Option<(String, u32)>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn set_current_page(page: Option<(String, u32)>) {
    CURRENT_PAGE.with(|c| *c.borrow_mut() = page);
}
