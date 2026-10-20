//! Markdown → document.

use super::attrs::Attrs;
use super::front;
use super::write::MONO;
use xenify_wp_doc::{
    Block, Cell, CharFmt, Document, Image, ListDef, ListItem, NodeId, Paragraph, Rgba, Row, Run, StyleSheet, Table, Toc,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const DIV_OPEN: &str = "<!--wp-div ";
const DIV_CLOSE: &str = "<!--/wp-div-->";
const SPAN_OPEN: &str = "<span data-wp=\"";

/// Rewrites Pandoc syntax pulldown-cmark doesn't know into HTML it passes
/// through: fenced divs become comment markers, bracketed spans `<span>`s.
fn preprocess(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    let mut fence: Option<String> = None;
    let mut depth = 0usize;
    for line in src.lines() {
        let t = line.trim_start();
        // Leave code blocks alone.
        if let Some(f) = &fence {
            if t.starts_with(f.as_str()) {
                fence = None;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            let c = t.chars().next().expect("non-empty");
            fence = Some(t.chars().take_while(|&x| x == c).collect());
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if t.starts_with(":::") {
            let rest = t.trim_start_matches(':').trim();
            if rest.is_empty() {
                if depth > 0 {
                    depth -= 1;
                    out.push_str(&format!("\n{DIV_CLOSE}\n\n"));
                    continue;
                }
            } else {
                let inner = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}')).unwrap_or(rest);
                depth += 1;
                out.push_str(&format!("\n{DIV_OPEN}{}-->\n\n", encode(inner)));
                continue;
            }
        }
        out.push_str(&spans(&sup_sub(&citations(line))));
        out.push('\n');
    }
    out
}

/// Pandoc's `^sup^` and `~sub~` (no spaces inside; also within words,
/// which the Markdown parser doesn't allow) → `<sup>`/`<sub>`.
fn sup_sub(line: &str) -> String {
    if !line.contains(['^', '~']) {
        return line.to_string();
    }
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 16);
    let mut i = 0;
    let mut ticks = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            ticks += 1;
        }
        let escaped = i > 0 && chars[i - 1] == '\\';
        let single = (c == '^' || c == '~')
            && !escaped
            && ticks % 2 == 0
            && chars.get(i + 1).is_some_and(|n| *n != c && !n.is_whitespace())
            && (i == 0 || chars[i - 1] != c);
        if single
            && let Some(len) = chars[i + 1..].iter().position(|x| *x == c || x.is_whitespace())
            && chars[i + 1 + len] == c
            && chars.get(i + 2 + len) != Some(&c)
            && !chars[i + len].eq(&'\\')
        {
            let inner: String = chars[i + 1..i + 1 + len].iter().collect();
            let tag = if c == '^' { "sup" } else { "sub" };
            out.push_str(&format!("<{tag}>{inner}</{tag}>"));
            i += len + 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Pandoc citations `[@smith2020; @lee2018, p. 4]` → `[c]{cite="smith2020;lee2018" locator="p. 4"}`.
fn citations(line: &str) -> String {
    if !line.contains("[@") && !line.contains("[-@") {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find('[') {
        let (before, from) = rest.split_at(i);
        out.push_str(before);
        let in_code = out.matches('`').count() % 2 == 1;
        let escaped = before.ends_with('\\');
        let inner_end = from.find(']');
        let parsed = (!in_code && !escaped)
            .then_some(inner_end)
            .flatten()
            .and_then(|e| parse_cite(&from[1..e]).map(|c| (e, c)))
            .filter(|(e, _)| !from[e + 1..].starts_with(['(', '{', '[']));
        match parsed {
            Some((e, (keys, locator))) => {
                let loc = if locator.is_empty() {
                    String::new()
                } else {
                    format!(" locator=\"{}\"", locator.replace('"', "'"))
                };
                out.push_str(&format!("[c]{{cite=\"{}\"{loc}}}", keys.join(";")));
                rest = &from[e + 1..];
            }
            None => {
                out.push('[');
                rest = &from[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// "@a; @b, p. 4" → (keys, locator).
fn parse_cite(s: &str) -> Option<(Vec<String>, String)> {
    let mut keys = Vec::new();
    let mut locator = String::new();
    for part in s.split(';') {
        let part = part.trim();
        let part = part.strip_prefix('-').unwrap_or(part);
        let body = part.strip_prefix('@')?;
        let end = body.find(|c: char| !(c.is_alphanumeric() || ":.#$%&-+?<>~/_".contains(c))).unwrap_or(body.len());
        let key = body[..end].trim_end_matches(['.', ':']);
        if key.is_empty() {
            return None;
        }
        keys.push(key.to_string());
        let after = body[key.len()..].trim_start_matches(',').trim();
        if !after.is_empty() {
            locator = after.to_string();
        }
    }
    Some((keys, locator))
}

fn encode(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace("--", "&#45;&#45;").replace('<', "&lt;").replace('>', "&gt;")
}

fn decode(s: &str) -> String {
    s.replace("&#45;", "-").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// `[text]{attrs}` → `<span data-wp="attrs">text</span>` (outside code spans).
fn spans(line: &str) -> String {
    let b = line.as_bytes();
    // Byte ranges inside backtick code spans.
    let mut code = vec![false; b.len()];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            let n = b[i..].iter().take_while(|&&c| c == b'`').count();
            let fence = &line[i..i + n];
            if let Some(end) = line[i + n..].find(fence) {
                for c in &mut code[i..i + n + end + n] {
                    *c = true;
                }
                i += n + end + n;
                continue;
            }
            i += n;
        } else {
            i += 1;
        }
    }
    let mut opens: Vec<usize> = Vec::new();
    // (open '[', close ']', end of '}', attrs)
    let mut found: Vec<(usize, usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if code[i] || (i > 0 && b[i - 1] == b'\\') {
            i += 1;
            continue;
        }
        match b[i] {
            b'[' => opens.push(i),
            b']' => {
                if let Some(o) = opens.pop()
                    && b.get(i + 1) == Some(&b'{')
                    && let Some(end) = line[i + 1..].find('}')
                {
                    let attrs = &line[i + 2..i + 1 + end];
                    found.push((o, i, i + 1 + end, attrs.to_string()));
                    i += end + 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if found.is_empty() {
        return line.to_string();
    }
    // Replace from the end so earlier offsets stay valid.
    let mut s = line.to_string();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (o, c, e, attrs) in found {
        edits.push((c, e + 1, "</span>".into()));
        edits.push((o, o + 1, format!("{SPAN_OPEN}{}\">", encode(&attrs))));
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (from, to, with) in edits {
        s.replace_range(from..to, &with);
    }
    s
}

struct ListCtx {
    ordered: bool,
    start: u64,
    /// Items seen so far in this list.
    items: usize,
}

/// A table being read: (rows of (header?, cells of blocks), the row in progress, the cell in progress, in a header row?).
type TableInProgress = (Vec<(bool, Vec<Vec<Block>>)>, Vec<Vec<Block>>, Option<Vec<Block>>, bool);

#[derive(Default)]
struct Reader {
    doc: Document,
    warnings: Vec<String>,
    /// The paragraph being built.
    cur: Option<Paragraph>,
    fmt: Vec<CharFmt>,
    lists: Vec<ListCtx>,
    /// Paragraphs started in the current list item (continuations are indented, not numbered).
    item_paras: usize,
    quote: usize,
    divs: Vec<Attrs>,
    heading: Option<u8>,
    code_block: bool,
    /// Table being read: rows of cells of blocks; the row and cell in progress.
    table: Option<TableInProgress>,
    /// Character formatting from the enclosing divs (e.g. `color=`), under the paragraph's runs.
    div_fmt: CharFmt,
    /// A frame being read, and its name (for `next=` links).
    frame: Option<(xenify_wp_doc::Frame, Option<String>, Option<String>)>,
    /// Frames read: (name, next name) to link up at the end.
    frame_links: Vec<(NodeId, Option<String>, Option<String>)>,
    /// An image seen in the current paragraph: (src, alt so far).
    image: Option<(String, String)>,
    /// The picture's title: in table cells it carries the size ("width=… height=…").
    image_title: String,
    in_image: bool,
    unsupported: std::collections::BTreeSet<&'static str>,
    /// Open spans: true for a field's span (its text is the field's, not kept).
    span_kinds: Vec<bool>,
    /// A `.comment-start` span being read: its attributes and text.
    comment_capture: Option<(Attrs, String)>,
    /// Comments whose anchor is open, and Markdown comment ids → threads.
    open_comments: Vec<NodeId>,
    comment_ids: std::collections::HashMap<String, NodeId>,
    /// Inside a field's span: text is skipped.
    skip_text: usize,
    /// Notes by Markdown label, and the definition being read.
    notes: std::collections::HashMap<String, NodeId>,
    note_blocks: std::collections::HashMap<String, Vec<Block>>,
    in_note: Option<(String, Vec<Block>)>,
    /// A `::: {.header}` / `{.footer}` div being read.
    in_header: Option<xenify_wp_doc::HeaderPart>,
    /// Heading anchors (`{#ref-12}`) → paragraph, and cross-references to resolve.
    anchors: std::collections::HashMap<String, NodeId>,
    refs: Vec<(NodeId, String)>,
}

impl Reader {
    fn style_by_name(&self, name: &str) -> String {
        let s = &self.doc.styles;
        s.paragraph
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(name) || x.id.eq_ignore_ascii_case(name))
            .map_or_else(|| StyleSheet::NORMAL.into(), |x| x.id.clone())
    }

    fn start_para(&mut self) {
        if self.cur.is_some() {
            return;
        }
        let id = self.doc.alloc_id();
        let mut p = Paragraph::new(id, StyleSheet::NORMAL, "");
        if let Some(level) = self.heading {
            p.style = StyleSheet::heading(level);
        } else if self.code_block {
            p.style = StyleSheet::CODE.into();
        } else if self.quote > 0 {
            p.style = StyleSheet::QUOTE.into();
        }
        self.div_fmt = CharFmt::default();
        for d in self.divs.clone() {
            if let Some(name) = d.get("custom-style") {
                p.style = self.style_by_name(name);
            }
            let (chars, props) = front::props_from(&|k| d.get(k).map(|v| serde_json::Value::String(v.into())));
            p.props = p.props.with(&props);
            self.div_fmt = self.div_fmt.with(&chars);
        }
        if let Some(l) = self.lists.last() {
            let level = (self.lists.len() - 1).min(xenify_wp_doc::lists::MAX_LEVEL as usize) as u8;
            let list = if l.ordered { ListDef::NUMBERS } else { ListDef::BULLETS };
            if self.item_paras == 0 {
                let restart = (l.items == 1 && l.ordered).then_some(l.start as u32);
                p.list = Some(ListItem { list: list.into(), level, restart, checked: false, follow: false });
            } else {
                // A second paragraph in one item: more of that item.
                p.list = Some(ListItem { follow: true, ..ListItem::new(list, level) });
            }
            self.item_paras += 1;
        }
        self.cur = Some(p);
    }

    /// A `.comment-start` span read: a new thread (its anchor opens here), or a reply.
    fn finish_comment(&mut self, a: &Attrs, text: &str) {
        use xenify_wp_doc::review::{Comment, Reply};
        let author = a.get("author").unwrap_or("").to_string();
        let date = a.get("date").unwrap_or("").to_string();
        let md_id = a.get("id").unwrap_or("").to_string();
        let parent = a.get("parent").and_then(|p| self.comment_ids.get(p)).copied();
        if let Some(t) = parent.and_then(|p| self.doc.meta.comments.iter_mut().find(|c| c.id == p)) {
            t.replies.push(Reply { author, date, text: text.into() });
            let id = t.id;
            self.comment_ids.insert(md_id, id);
            return;
        }
        let id = self.alloc();
        let resolved = a.get("done").is_some_and(|d| d == "true");
        self.doc.meta.comments.push(Comment { id, author, date, text: text.into(), resolved, ..Default::default() });
        self.comment_ids.insert(md_id, id);
        self.open_comments.push(id);
    }

    /// A field at the end of the current paragraph.
    fn field(&mut self, f: xenify_wp_doc::Field) {
        self.start_para();
        let fmt = self.fmt.iter().fold(self.div_fmt.clone(), |a, f| a.with(f));
        let fmt = CharFmt { field: Some(f), ..fmt };
        let p = self.cur.as_mut().expect("just started");
        p.text.push(xenify_wp_doc::Field::CHAR);
        p.runs.push(Run { len: xenify_wp_doc::Field::CHAR.len_utf8(), fmt });
    }

    /// The note for Markdown label `label` (made on first sight).
    fn note_id(&mut self, label: &str) -> NodeId {
        if let Some(id) = self.notes.get(label) {
            return *id;
        }
        let id = self.alloc();
        self.notes.insert(label.to_string(), id);
        id
    }

    fn push_block(&mut self, b: Block) {
        if let Some((_, blocks)) = &mut self.in_note {
            blocks.push(b);
            return;
        }
        if self.table.is_none()
            && let Some(h) = &mut self.in_header
        {
            h.blocks.push(b);
            return;
        }
        match (&mut self.table, &mut self.frame) {
            (Some((_, _, Some(cell), _)), _) => cell.push(b),
            (_, Some((f, _, _))) => f.blocks.push(b),
            _ => self.doc.sections.last_mut().expect("a section").blocks.push(b),
        }
    }

    fn start_frame(&mut self, a: &Attrs) {
        let num = |k: &str| a.get(k).and_then(|v| front::length(&serde_json::Value::String(v.into())));
        let d = xenify_wp_doc::Frame::default();
        let border = match a.get("border") {
            None | Some("none") => None,
            Some(w) => w.parse().ok().map(|width| xenify_wp_doc::Border {
                width,
                color: a.get("border-color").and_then(Rgba::from_hex).unwrap_or(xenify_wp_doc::Border::default().color),
            }),
        };
        let frame = xenify_wp_doc::Frame {
            id: self.alloc(),
            page: a.get("page").and_then(|v| v.parse::<u32>().ok()).map_or(0, |p| p.saturating_sub(1)),
            x: num("x").unwrap_or(d.x),
            y: num("y").unwrap_or(d.y),
            width: num("width").unwrap_or(d.width),
            height: num("height").unwrap_or(d.height),
            wrap: a.get("wrap").and_then(xenify_wp_doc::Wrap::from_key).unwrap_or_default(),
            gap: num("gap").unwrap_or(d.gap),
            border,
            fill: a.get("fill").and_then(Rgba::from_hex),
            padding: num("padding").unwrap_or(d.padding),
            blocks: Vec::new(),
            next: None,
        };
        self.frame = Some((frame, a.id.clone(), a.get("next").map(str::to_string)));
    }

    fn finish_frame(&mut self) {
        if let Some((f, name, next)) = self.frame.take() {
            self.frame_links.push((f.id, name, next));
            self.doc.sections[0].frames.push(f);
        }
    }

    fn end_para(&mut self) {
        if let Some(mut p) = self.cur.take() {
            if p.text == "\u{a0}" {
                // `&nbsp;` marks an empty paragraph.
                p.text.clear();
                p.runs.clear();
            }
            // A paragraph that is just a picture becomes a picture block.
            if let Some((src, alt)) = self.image.take()
                && p.text.trim() == format!("[image: {alt}]").trim()
            {
                let mut fig = self.divs.iter().rev().find(|d| d.has_class("figure")).cloned().unwrap_or_default();
                // In a table cell the size comes in the title: "width=120 height=80".
                if self.table.is_some() {
                    for kv in self.image_title.split_whitespace() {
                        if let Some((k, v)) = kv.split_once('=') {
                            fig.pairs.push((k.to_string(), v.to_string()));
                        }
                    }
                }
                let num = |k: &str| fig.get(k).and_then(|v| front::length(&serde_json::Value::String(v.into())));
                let block = Image {
                    id: self.doc.alloc_id(),
                    src,
                    alt,
                    width: num("width").unwrap_or(0.0),
                    height: num("height").unwrap_or(0.0),
                    align: fig.get("align").and_then(front::parse_align).unwrap_or_default(),
                    border: fig.get("border").and_then(|v| {
                        let mut it = v.split_whitespace();
                        let width = it.next()?.parse().ok()?;
                        let color = it.next().and_then(xenify_wp_doc::Rgba::from_hex)?;
                        Some(xenify_wp_doc::Border { width, color })
                    }),
                };
                self.push_block(Block::Image(block));
                return;
            }
            self.push_block(Block::Paragraph(p));
        }
    }

    fn alloc(&mut self) -> NodeId {
        self.doc.alloc_id()
    }

    fn finish_table(&mut self) {
        let Some((rows, _, _, _)) = self.table.take() else { return };
        let attrs = self.divs.iter().rev().find(|d| d.has_class("table")).cloned().unwrap_or_default();
        let cols = rows.iter().map(|r| r.1.len()).max().unwrap_or(1).max(1);
        let heads: Option<usize> = attrs.get("header-rows").and_then(|v| v.parse().ok());
        let mut t = Table { id: self.alloc(), widths: vec![1.0; cols], ..Default::default() };
        for (i, (head, cells)) in rows.into_iter().enumerate() {
            let header = heads.map_or(head, |n| i < n);
            let row_id = self.alloc();
            let mut row = Row { id: row_id, header, footer: false, repeat: None, min_height: None, cells: Vec::new() };
            for blocks in cells {
                let id = self.alloc();
                let mut cell = Cell::new(id, NodeId(0));
                cell.blocks = blocks;
                row.cells.push(cell);
            }
            t.rows.push(row);
        }
        if let Some((row, list)) = attrs.get("repeat").and_then(|v| v.split_once([':', '=']))
            && let Ok(row) = row.trim().parse::<usize>()
            && let Some(r) = t.rows.get_mut(row.saturating_sub(1))
        {
            r.repeat = Some(list.trim().to_string());
        }
        if let Some(w) = attrs.get("widths") {
            let ws: Vec<f32> = w.split(',').filter_map(|x| x.trim().trim_end_matches('%').parse().ok()).collect();
            if ws.len() == cols {
                t.widths = ws;
            }
        }
        if let Some(w) = attrs.get("width").and_then(|v| v.trim_end_matches('%').parse::<f32>().ok()) {
            t.width = (w / 100.0).clamp(0.1, 1.0);
        }
        t.align = attrs.get("align").and_then(front::parse_align).unwrap_or_default();
        t.header_col = attrs.get("header-col").is_some_and(|v| v != "0");
        t.repeat_header = attrs.get("repeat-header").is_some_and(|v| v != "0");
        t.repeat_footer = attrs.get("repeat-footer").is_some_and(|v| v != "0");
        let feet = attrs.get("footer-rows").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        let n = t.rows.len();
        for r in t.rows.iter_mut().skip(n.saturating_sub(feet.min(n.saturating_sub(1)))) {
            r.footer = true;
        }
        t.band = attrs.get("band").and_then(Rgba::from_hex);
        t.header_fill = attrs.get("header-fill").and_then(Rgba::from_hex);
        for h in attrs.get("heights").unwrap_or("").split(',') {
            if let Some((r, v)) = h.split_once(':')
                && let (Ok(r), Ok(v)) = (r.trim().parse::<usize>(), v.trim().parse::<f32>())
                && let Some(row) = t.rows.get_mut(r.saturating_sub(1))
            {
                row.min_height = Some(v);
            }
        }
        // Cell by cell: "row.col span=2x1 fill=#… valign=middle top=1#… …".
        let line = |v: &str| {
            let (w, c) = v.split_at(v.find('#').unwrap_or(v.len()));
            Some(xenify_wp_doc::Border { width: w.parse().ok()?, color: Rgba::from_hex(c)? })
        };
        for entry in attrs.get("cells").unwrap_or("").split(';') {
            let mut words = entry.split_whitespace();
            let Some((r, c)) = words.next().and_then(|rc| rc.split_once('.')) else { continue };
            let (Ok(r), Ok(c)) = (r.parse::<usize>(), c.parse::<usize>()) else { continue };
            let Some(cell) = t.rows.get_mut(r.saturating_sub(1)).and_then(|row| row.cells.get_mut(c.saturating_sub(1)))
            else {
                continue;
            };
            for w in words {
                let Some((k, v)) = w.split_once('=') else { continue };
                match k {
                    "span" => {
                        if let Some((cs, rs)) = v.split_once('x') {
                            cell.col_span = cs.parse().unwrap_or(1);
                            cell.row_span = rs.parse().unwrap_or(1);
                        }
                    }
                    "fill" => cell.shading = Rgba::from_hex(v),
                    "valign" => {
                        cell.v_align = match v {
                            "middle" => xenify_wp_doc::VAlign::Middle,
                            "bottom" => xenify_wp_doc::VAlign::Bottom,
                            _ => xenify_wp_doc::VAlign::Top,
                        }
                    }
                    "top" | "bottom" | "left" | "right" => {
                        let b = cell.borders.get_or_insert_with(Default::default);
                        let l = line(v);
                        match k {
                            "top" => b.top = l,
                            "bottom" => b.bottom = l,
                            "left" => b.left = l,
                            _ => b.right = l,
                        }
                    }
                    _ => {}
                }
            }
        }
        match attrs.get("border") {
            Some("none") => t.border = None,
            Some(w) => {
                let color = attrs
                    .get("border-color")
                    .and_then(Rgba::from_hex)
                    .unwrap_or(xenify_wp_doc::Border::default().color);
                t.border = w.parse().ok().map(|width| xenify_wp_doc::Border { width, color });
            }
            None => {}
        }
        let mut next = self.doc.next_id;
        t.normalize(&mut || {
            let id = NodeId(next);
            next += 1;
            id
        });
        self.doc.next_id = next;
        self.push_block(Block::Table(t));
    }

    fn text(&mut self, t: &str) {
        if let Some((_, buf)) = &mut self.comment_capture {
            buf.push_str(t);
            return;
        }
        if t.is_empty() || self.skip_text > 0 {
            return;
        }
        if self.in_image
            && let Some((_, alt)) = &mut self.image
            && t != "[image: "
        {
            alt.push_str(t);
        }
        self.start_para();
        let mut fmt = self.fmt.iter().fold(self.div_fmt.clone(), |a, f| a.with(f));
        fmt.comments.extend(self.open_comments.iter().copied());
        let p = self.cur.as_mut().expect("just started");
        p.text.push_str(t);
        match p.runs.last_mut() {
            Some(r) if r.fmt == fmt => r.len += t.len(),
            _ => p.runs.push(Run { len: t.len(), fmt }),
        }
    }

    fn span_fmt(a: &Attrs) -> CharFmt {
        let (mut c, _) = front::props_from(&|k| a.get(k).map(|v| serde_json::Value::String(v.into())));
        if a.has_class("underline") {
            c.underline = Some(true);
        }
        if a.has_class("superscript") {
            c.vert = Some(xenify_wp_doc::VertAlign::Superscript);
        }
        if a.has_class("subscript") {
            c.vert = Some(xenify_wp_doc::VertAlign::Subscript);
        }
        if a.has_class("mark") && c.highlight.is_none() {
            c.highlight = Some(Rgba([255, 235, 59, 255]));
        }
        c
    }

    fn event(&mut self, e: Event) {
        match e {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.start_para(),
                Tag::Heading { level, id, .. } => {
                    self.end_para();
                    let anchor = id.map(|s| s.to_string());
                    self.heading = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    });
                    self.start_para();
                    if let (Some(a), Some(p)) = (anchor, &self.cur) {
                        self.anchors.insert(a, p.id);
                    }
                }
                Tag::BlockQuote(_) => {
                    self.end_para();
                    self.quote += 1;
                }
                Tag::CodeBlock(_) => {
                    self.end_para();
                    self.code_block = true;
                }
                Tag::List(start) => {
                    self.end_para();
                    self.lists.push(ListCtx { ordered: start.is_some(), start: start.unwrap_or(1), items: 0 });
                }
                Tag::Item => {
                    self.end_para();
                    if let Some(l) = self.lists.last_mut() {
                        l.items += 1;
                    }
                    self.item_paras = 0;
                }
                Tag::Emphasis => self.fmt.push(CharFmt { italic: Some(true), ..Default::default() }),
                Tag::Strong => self.fmt.push(CharFmt { bold: Some(true), ..Default::default() }),
                Tag::Strikethrough => self.fmt.push(CharFmt { strike: Some(true), ..Default::default() }),
                Tag::Superscript => {
                    self.fmt.push(CharFmt { vert: Some(xenify_wp_doc::VertAlign::Superscript), ..Default::default() })
                }
                Tag::Subscript => {
                    self.fmt.push(CharFmt { vert: Some(xenify_wp_doc::VertAlign::Subscript), ..Default::default() })
                }
                Tag::Link { dest_url, .. } => {
                    self.fmt.push(CharFmt { link: Some(dest_url.to_string()), ..Default::default() })
                }
                Tag::Image { dest_url, title, .. } => {
                    self.image = Some((dest_url.to_string(), String::new()));
                    self.image_title = title.to_string();
                    self.in_image = true;
                    self.fmt.push(CharFmt { link: Some(dest_url.to_string()), ..Default::default() });
                    self.text("[image: ");
                }
                Tag::Table(_) => {
                    self.end_para();
                    self.table = Some((Vec::new(), Vec::new(), None, false));
                }
                Tag::TableHead => {
                    if let Some(t) = &mut self.table {
                        t.3 = true;
                    }
                }
                Tag::TableRow => {}
                Tag::TableCell => {
                    if let Some(t) = &mut self.table {
                        t.2 = Some(Vec::new());
                    }
                }
                Tag::MetadataBlock(_) | Tag::HtmlBlock => {}
                Tag::FootnoteDefinition(label) => {
                    self.end_para();
                    self.in_note = Some((label.to_string(), Vec::new()));
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    self.end_para();
                }
                TagEnd::Heading(_) => {
                    self.end_para();
                    self.heading = None;
                }
                TagEnd::BlockQuote(_) => {
                    self.end_para();
                    self.quote = self.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    self.end_para();
                    self.code_block = false;
                }
                TagEnd::List(_) => {
                    self.end_para();
                    self.lists.pop();
                    self.item_paras = 1;
                }
                TagEnd::Item => {
                    self.end_para();
                }
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::Link => {
                    self.fmt.pop();
                }
                TagEnd::Image => {
                    self.in_image = false;
                    self.text("]");
                    self.fmt.pop();
                }
                TagEnd::TableCell => {
                    self.end_para();
                    if let Some(t) = &mut self.table {
                        let cell = t.2.take().unwrap_or_default();
                        t.1.push(cell);
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    self.end_para();
                    if let Some(t) = &mut self.table {
                        let cells = std::mem::take(&mut t.1);
                        let head = std::mem::replace(&mut t.3, false);
                        t.0.push((head, cells));
                    }
                }
                TagEnd::Table => self.finish_table(),
                TagEnd::FootnoteDefinition => {
                    self.end_para();
                    if let Some((label, blocks)) = self.in_note.take() {
                        self.note_blocks.insert(label, blocks);
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if self.code_block {
                    // One Code paragraph per line.
                    let t = t.strip_suffix('\n').unwrap_or(&t).to_string();
                    for (i, line) in t.split('\n').enumerate() {
                        if i > 0 {
                            self.end_para();
                        }
                        self.start_para();
                        self.text(line);
                    }
                } else {
                    self.text(&t);
                }
            }
            Event::Code(t) => {
                self.fmt.push(CharFmt { font: Some(MONO.into()), ..Default::default() });
                self.text(&t);
                self.fmt.pop();
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => {
                // No line breaks inside paragraphs yet: start a new paragraph like it.
                let like = self.cur.as_ref().map(|p| (p.style.clone(), p.props.clone()));
                self.end_para();
                self.start_para();
                if let (Some((style, props)), Some(p)) = (like, self.cur.as_mut()) {
                    p.style = style;
                    p.props = props;
                    p.list = None;
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => self.html(&h),
            Event::TaskListMarker(done) => {
                // "- [ ] …": a checklist item (its paragraph starts here in tight lists).
                if self.cur.is_none() && !self.lists.is_empty() {
                    self.start_para();
                }
                if let Some(item) = self.cur.as_mut().and_then(|p| p.list.as_mut()) {
                    item.list = ListDef::CHECKLIST.into();
                    item.checked = done;
                    item.restart = None;
                    if self.doc.list_def(ListDef::CHECKLIST).is_none() {
                        self.doc.lists.extend(ListDef::builtin_def(ListDef::CHECKLIST));
                    }
                } else {
                    self.text(if done { "☑ " } else { "☐ " });
                }
            }
            Event::Rule => {
                // A divider line.
                self.end_para();
                self.start_para();
                if let Some(p) = self.cur.as_mut() {
                    p.props = xenify_wp_doc::ops::divider_props();
                }
                self.end_para();
            }
            Event::FootnoteReference(label) => {
                let id = self.note_id(&label);
                self.field(xenify_wp_doc::Field::Note { id });
            }
            Event::InlineMath(tex) => {
                self.field(xenify_wp_doc::Field::Equation {
                    latex: tex.trim().to_string(),
                    display: false,
                    omml: String::new(),
                });
            }
            Event::DisplayMath(tex) => {
                self.field(xenify_wp_doc::Field::Equation {
                    latex: tex.trim().to_string(),
                    display: true,
                    omml: String::new(),
                });
            }
        }
    }

    fn html(&mut self, h: &str) {
        let t = h.trim();
        if let Some(rest) = t.strip_prefix(DIV_OPEN) {
            self.end_para();
            let inner = rest.trim_end_matches("-->").trim();
            let a = Attrs::parse(&decode(inner));
            let kind = if a.has_class("toc") {
                Some(xenify_wp_doc::TocKind::Contents)
            } else if a.has_class("bibliography") {
                Some(xenify_wp_doc::TocKind::Bibliography)
            } else if a.has_class("figures") {
                Some(xenify_wp_doc::TocKind::Figures)
            } else if a.has_class("changelog") {
                Some(xenify_wp_doc::TocKind::ChangeLog)
            } else {
                None
            };
            if let Some(kind) = kind {
                let id = self.alloc();
                let levels = a.get("levels").and_then(|v| v.parse().ok()).unwrap_or(3);
                let page_numbers = a.get("page-numbers") != Some("false");
                self.push_block(Block::Toc(Toc { id, levels, page_numbers, kind }));
            }
            if a.has_class("section") && self.frame.is_none() && self.table.is_none() && self.in_header.is_none() {
                // A section break: what follows is a new section.
                let start = match a.get("start") {
                    Some("continuous") => xenify_wp_doc::SectionStart::Continuous,
                    _ => xenify_wp_doc::SectionStart::NewPage,
                };
                let page = super::front::section_page_from(a.get("page").unwrap_or("{}"));
                let id = self.alloc();
                self.doc.sections.push(xenify_wp_doc::Section {
                    id,
                    start,
                    page,
                    blocks: Vec::new(),
                    frames: Vec::new(),
                    notes: Vec::new(),
                    headers: Vec::new(),
                });
            }
            if a.has_class("frame") && self.frame.is_none() && self.table.is_none() {
                self.start_frame(&a);
            }
            if (a.has_class("header") || a.has_class("footer")) && self.in_header.is_none() && self.table.is_none() {
                let slot = match a.get("slot") {
                    Some("first") => xenify_wp_doc::HfSlot::First,
                    Some("even") => xenify_wp_doc::HfSlot::Even,
                    _ => xenify_wp_doc::HfSlot::Default,
                };
                let id = self.alloc();
                self.in_header =
                    Some(xenify_wp_doc::HeaderPart { id, footer: a.has_class("footer"), slot, blocks: Vec::new() });
            }
            self.divs.push(a);
        } else if t.starts_with(DIV_CLOSE) {
            self.end_para();
            let closed = self.divs.pop();
            if closed.as_ref().is_some_and(|d| d.has_class("frame")) {
                self.finish_frame();
            }
            if closed.as_ref().is_some_and(|d| d.has_class("header") || d.has_class("footer"))
                && let Some(h) = self.in_header.take()
            {
                self.doc.sections[0].headers.push(h);
            }
        } else if let Some(rest) = t.strip_prefix(SPAN_OPEN) {
            let inner = rest.strip_suffix("\">").unwrap_or(rest);
            let a = Attrs::parse(&decode(inner));
            let field = match (a.get("field"), a.get("cite")) {
                (_, Some(keys)) => Some(xenify_wp_doc::Field::Citation {
                    keys: keys.split(';').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect(),
                    locator: a.get("locator").unwrap_or("").to_string(),
                }),
                (Some("page"), _) => Some(xenify_wp_doc::Field::Page),
                (Some("pages"), _) => Some(xenify_wp_doc::Field::Pages),
                (Some("date"), _) => Some(xenify_wp_doc::Field::Date),
                (Some("badge"), _) => Some(xenify_wp_doc::Field::Badge {
                    text: a.get("text").unwrap_or("").to_string(),
                    color: a.get("color").unwrap_or("grey").to_string(),
                }),
                (Some("datetime"), _) => Some(xenify_wp_doc::Field::DateTime {
                    format: a.get("format").unwrap_or(xenify_wp_doc::datetime::PRESETS[0]).to_string(),
                    fixed: a.get("fixed").map(str::to_string),
                }),
                (Some("title"), _) => Some(xenify_wp_doc::Field::Title),
                (Some("author"), _) => Some(xenify_wp_doc::Field::Author),
                (Some("check"), _) => Some(xenify_wp_doc::Field::Check { checked: a.get("checked") == Some("true") }),
                (Some("ref"), _) => {
                    let placeholder = self.alloc();
                    self.refs.push((placeholder, a.get("target").unwrap_or("").trim_start_matches('#').to_string()));
                    Some(xenify_wp_doc::Field::Ref { target: placeholder })
                }
                _ => None,
            };
            if a.has_class("comment-start") && self.comment_capture.is_none() {
                self.comment_capture = Some((a, String::new()));
                self.skip_text += 1;
                self.span_kinds.push(true);
                return;
            }
            if a.has_class("comment-end") {
                if let Some(id) = a.get("id").and_then(|i| self.comment_ids.get(i)).copied() {
                    self.open_comments.retain(|c| *c != id);
                }
                self.skip_text += 1;
                self.span_kinds.push(true);
                return;
            }
            if a.has_class("insertion") || a.has_class("deletion") {
                use xenify_wp_doc::review::{Change, ChangeKind};
                let kind = if a.has_class("insertion") { ChangeKind::Inserted } else { ChangeKind::Deleted };
                let change = Change {
                    kind,
                    author: a.get("author").unwrap_or("").to_string(),
                    date: a.get("date").unwrap_or("").to_string(),
                };
                self.fmt.push(CharFmt { change: Some(change), ..Default::default() });
                self.span_kinds.push(false);
                return;
            }
            match field {
                Some(f) if self.skip_text == 0 => {
                    self.field(f);
                    self.skip_text += 1;
                    self.span_kinds.push(true);
                }
                _ => {
                    self.fmt.push(Self::span_fmt(&a));
                    self.span_kinds.push(false);
                }
            }
        } else if t == "</span>" {
            if self.span_kinds.pop() == Some(true) {
                self.skip_text -= 1;
                if self.skip_text == 0
                    && let Some((a, text)) = self.comment_capture.take()
                {
                    self.finish_comment(&a, text.trim());
                }
            } else {
                self.fmt.pop();
            }
        } else if t == "<sup>" || t == "<sub>" {
            let vert =
                if t == "<sup>" { xenify_wp_doc::VertAlign::Superscript } else { xenify_wp_doc::VertAlign::Subscript };
            self.fmt.push(CharFmt { vert: Some(vert), ..Default::default() });
        } else if t == "</sup>" || t == "</sub>" {
            self.fmt.pop();
        } else if t == "<!-- -->" || t.starts_with("<!--") {
            // Comments (e.g. list separators) carry no content.
        } else if t.eq_ignore_ascii_case("<br>") || t.eq_ignore_ascii_case("<br/>") || t.eq_ignore_ascii_case("<br />")
        {
            self.event(Event::HardBreak);
        } else {
            self.unsupported.insert("raw HTML");
        }
    }
}

pub fn read(src: &str) -> (Document, Vec<String>) {
    let mut r = Reader { doc: Document::new(), ..Default::default() };
    r.doc.sections[0].blocks.clear();
    // Front matter first: styles it defines are used by the body.
    let (front_src, body) = split_front_matter(src);
    if let Some(fm) = front_src {
        match front::yaml_to_json(fm) {
            Ok(v) => front::read(&v, &mut r.doc, &mut r.warnings),
            Err(e) => r.warnings.push(format!("the front matter couldn't be read: {e}")),
        }
    }
    let pre = preprocess(body);
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT
        | Options::ENABLE_MATH;
    for e in Parser::new_ext(&pre, opts) {
        r.event(e);
    }
    r.end_para();
    r.finish_frame();
    // Notes: definitions found for the labels the text refers to.
    let mut notes: Vec<xenify_wp_doc::Note> = Vec::new();
    for (label, id) in std::mem::take(&mut r.notes) {
        let mut blocks = r.note_blocks.remove(&label).unwrap_or_default();
        for b in &mut blocks {
            if let Block::Paragraph(p) = b
                && p.style == StyleSheet::NORMAL
            {
                p.style = StyleSheet::NOTE.into();
            }
        }
        if blocks.is_empty() {
            blocks.push(Block::Paragraph(Paragraph::new(r.alloc(), StyleSheet::NOTE, "")));
        }
        let kind = if label.starts_with("end-") { xenify_wp_doc::NoteKind::End } else { xenify_wp_doc::NoteKind::Foot };
        notes.push(xenify_wp_doc::Note { id, kind, blocks });
    }
    notes.sort_by_key(|n| n.id);
    r.doc.sections[0].notes = notes;
    if r.doc.styles.get(StyleSheet::NOTE).is_none()
        && !r.doc.sections[0].notes.is_empty()
        && let Some(def) = StyleSheet::default().get(StyleSheet::NOTE).cloned()
    {
        r.doc.styles.paragraph.push(def);
    }
    // Cross-references find their headings.
    let (refs, anchors) = (std::mem::take(&mut r.refs), std::mem::take(&mut r.anchors));
    // Links to heading anchors go to the paragraph (#p12).
    if !anchors.is_empty() {
        r.doc.for_each_para_mut(&mut |p| {
            for run in &mut p.runs {
                if let Some(t) = run.fmt.link.as_deref().and_then(|l| l.strip_prefix('#')).and_then(|n| anchors.get(n))
                {
                    run.fmt.link = Some(format!("#p{}", t.0));
                }
            }
        });
    }
    if !refs.is_empty() {
        let map: std::collections::HashMap<NodeId, NodeId> =
            refs.into_iter().filter_map(|(ph, name)| anchors.get(&name).map(|t| (ph, *t))).collect();
        r.doc.for_each_para_mut(&mut |p| {
            for run in &mut p.runs {
                if let Some(xenify_wp_doc::Field::Ref { target }) = &mut run.fmt.field
                    && let Some(t) = map.get(target)
                {
                    *target = *t;
                }
            }
        });
    }
    // Threads: `next=` names another frame's `#id`.
    let names: std::collections::HashMap<String, NodeId> =
        r.frame_links.iter().filter_map(|(id, name, _)| name.clone().map(|n| (n, *id))).collect();
    for (id, _, next) in r.frame_links.clone() {
        let target = next.and_then(|n| names.get(n.trim_start_matches('#')).copied());
        if let Some(f) = r.doc.frame_mut(id) {
            f.next = target;
        }
    }
    for u in &r.unsupported {
        r.warnings.push(format!("This Markdown has {u}, which Xenify Write doesn't fully support yet."));
    }
    let mut doc = r.doc;
    doc.repair();
    (doc, r.warnings)
}

/// Splits `---\n…\n---` front matter from the body.
fn split_front_matter(src: &str) -> (Option<&str>, &str) {
    let s = src.strip_prefix('\u{feff}').unwrap_or(src);
    let Some(rest) = s.strip_prefix("---\n").or_else(|| s.strip_prefix("---\r\n")) else { return (None, s) };
    for end in ["\n---\n", "\n---\r\n", "\n...\n"] {
        if let Some(i) = rest.find(end) {
            return (Some(&rest[..i + 1]), &rest[i + end.len()..]);
        }
    }
    if let Some(fm) = rest.strip_suffix("\n---").or_else(|| rest.strip_suffix("\n---\n")) {
        return (Some(fm), "");
    }
    (None, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_become_html() {
        assert_eq!(spans("a [red]{color=#f00} b"), "a <span data-wp=\"color=#f00\">red</span> b");
        assert_eq!(spans("`[not]{x}` [yes]{.underline}"), "`[not]{x}` <span data-wp=\".underline\">yes</span>");
        assert_eq!(spans("[link](url) and [x]"), "[link](url) and [x]");
        assert_eq!(spans("[outer [inner]{.mark}]{size=20}").matches("<span").count(), 2);
    }

    #[test]
    fn front_matter_split() {
        assert_eq!(split_front_matter("---\na: 1\n---\nbody"), (Some("a: 1\n"), "body"));
        assert_eq!(split_front_matter("no front"), (None, "no front"));
    }
}

#[cfg(test)]
mod div_format_tests {
    #[test]
    fn character_attributes_on_divs_apply_to_the_text() {
        let (doc, _) = super::read("::: {color=#ff0000 bold=true}\nRed *and italic*\n:::\n");
        let p = doc.paragraphs().next().unwrap();
        assert!(
            p.runs.iter().all(|r| r.fmt.color == Some(xenify_wp_doc::Rgba::rgb(255, 0, 0)) && r.fmt.bold == Some(true)),
            "{:?}",
            p.runs
        );
        assert_eq!(p.runs.last().unwrap().fmt.italic, Some(true));
    }
}
