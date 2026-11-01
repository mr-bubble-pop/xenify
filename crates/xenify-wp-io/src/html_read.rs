//! Reads HTML into a document: what other programs put on the clipboard
//! (browsers, Google Docs, Word, LibreOffice), so pasting keeps headings,
//! lists, emphasis, links, colours, tables and pictures. A forgiving reader
//! for fragments, not a full HTML parser: unknown tags are passed through.

use base64::Engine as _;
use xenify_wp_doc::{
    Block, CharFmt, Document, Image, ListDef, ListItem, ParaProps, Paragraph, Rgba, Span, StyleSheet, Table,
    lists::MAX_LEVEL,
};

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Open(String, Vec<(String, String)>, bool),
    Close(String),
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[1..].find(';').map(|e| e + 1).filter(|e| *e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "bull" => Some('•'),
            "copy" => Some('©'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn tokenize(html: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            out.push(Token::Text(decode_entities(rest)));
            break;
        };
        if lt > 0 {
            out.push(Token::Text(decode_entities(&rest[..lt])));
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |e| &r[e + 3..]);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        // The tag up to its '>' (outside quotes).
        let bytes = rest.as_bytes();
        let mut i = 1;
        let mut quote = None;
        while i < bytes.len() {
            match (quote, bytes[i]) {
                (None, b'"' | b'\'') => quote = Some(bytes[i]),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => break,
                _ => {}
            }
            i += 1;
        }
        let inner = &rest[1..i.min(rest.len())];
        rest = if i < rest.len() { &rest[i + 1..] } else { "" };
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Token::Close(name.trim().to_ascii_lowercase()));
            continue;
        }
        let self_closing = inner.trim_end().ends_with('/');
        let inner = inner.trim_end().trim_end_matches('/');
        let (name, attrs_src) = inner.split_once(|c: char| c.is_whitespace()).unwrap_or((inner, ""));
        let name = name.to_ascii_lowercase();
        let attrs = parse_attrs(attrs_src);
        // Skip what isn't content.
        if matches!(name.as_str(), "style" | "script" | "head" | "title" | "xml") && !self_closing {
            let close = format!("</{name}");
            let lower = rest.to_ascii_lowercase();
            rest = lower.find(&close).map_or("", |e| {
                let after = &rest[e..];
                after.find('>').map_or("", |g| &after[g + 1..])
            });
            continue;
        }
        out.push(Token::Open(name, attrs, self_closing));
    }
    out
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let name_end = rest.find(|c: char| c == '=' || c.is_whitespace()).unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let body = &r[1..];
                let end = body.find(q).unwrap_or(body.len());
                value = decode_entities(&body[..end]);
                rest = body.get(end + 1..).unwrap_or("");
            } else {
                let end = r.find(char::is_whitespace).unwrap_or(r.len());
                value = decode_entities(&r[..end]);
                rest = &r[end..];
            }
        }
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = rest.trim_start();
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// `property: value` pairs of a style attribute.
fn css(attrs: &[(String, String)]) -> Vec<(String, String)> {
    attr(attrs, "style")
        .unwrap_or("")
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase()))
        .collect()
}

fn parse_color(v: &str) -> Option<Rgba> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix('#') {
        let h: String = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
        if h.len() >= 6 {
            let n = u32::from_str_radix(&h[..6], 16).ok()?;
            return Some(Rgba::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
        }
        return None;
    }
    if let Some(inner) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let parts: Vec<f32> = inner.trim_end_matches(')').split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() >= 3 {
            if parts.get(3).is_some_and(|a| *a == 0.0) {
                return None;
            }
            return Some(Rgba::rgb(parts[0] as u8, parts[1] as u8, parts[2] as u8));
        }
        return None;
    }
    Some(match v {
        "black" => Rgba::rgb(0, 0, 0),
        "white" => Rgba::rgb(255, 255, 255),
        "red" => Rgba::rgb(255, 0, 0),
        "green" => Rgba::rgb(0, 128, 0),
        "blue" => Rgba::rgb(0, 0, 255),
        "yellow" => Rgba::rgb(255, 255, 0),
        "gray" | "grey" => Rgba::rgb(128, 128, 128),
        "orange" => Rgba::rgb(255, 165, 0),
        "purple" => Rgba::rgb(128, 0, 128),
        _ => return None,
    })
}

/// Character formatting from a tag and its style attribute.
fn inline_fmt(name: &str, attrs: &[(String, String)]) -> CharFmt {
    let mut f = CharFmt::default();
    match name {
        "b" | "strong" => f.bold = Some(true),
        "i" | "em" | "cite" => f.italic = Some(true),
        "u" | "ins" => f.underline = Some(true),
        "s" | "strike" | "del" => f.strike = Some(true),
        "sup" => f.vert = Some(xenify_wp_doc::VertAlign::Superscript),
        "sub" => f.vert = Some(xenify_wp_doc::VertAlign::Subscript),
        "code" | "kbd" | "samp" | "tt" => f.font = Some("monospace".into()),
        "mark" => f.highlight = Some(Rgba::rgb(255, 235, 59)),
        "a" => f.link = attr(attrs, "href").filter(|h| !h.is_empty()).map(str::to_string),
        "font" => f.color = attr(attrs, "color").and_then(parse_color),
        _ => {}
    }
    for (k, v) in css(attrs) {
        match k.as_str() {
            "font-weight" => f.bold = Some(v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|n| n >= 600)),
            "font-style" => f.italic = Some(v == "italic" || v == "oblique"),
            "text-decoration" | "text-decoration-line" => {
                if v.contains("underline") {
                    f.underline = Some(true);
                }
                if v.contains("line-through") {
                    f.strike = Some(true);
                }
            }
            "color" => f.color = parse_color(&v).or(f.color),
            "background-color" | "background" => {
                if let Some(c) = parse_color(&v).filter(|c| *c != Rgba::rgb(255, 255, 255)) {
                    f.highlight = Some(c);
                }
            }
            "font-size" => {
                let pt = v
                    .strip_suffix("pt")
                    .and_then(|n| n.parse::<f32>().ok())
                    .or_else(|| v.strip_suffix("px").and_then(|n| n.parse::<f32>().ok()).map(|px| px * 0.75));
                if let Some(pt) = pt.filter(|p| (4.0..=144.0).contains(p)) {
                    f.size = Some(pt);
                }
            }
            _ => {}
        }
    }
    f
}

struct TableBuild {
    rows: Vec<Vec<CellBuild>>,
}

struct CellBuild {
    header: bool,
    col_span: u16,
    row_span: u16,
    shading: Option<Rgba>,
    blocks: Vec<Block>,
}

struct Builder {
    doc: Document,
    /// Where finished blocks go: the body, or the innermost open cell.
    blocks: Vec<Block>,
    tables: Vec<(TableBuild, Vec<Block>)>,
    fmt: Vec<CharFmt>,
    para: Option<Paragraph>,
    style: Vec<String>,
    lists: Vec<(bool, usize)>,
    pre: usize,
    /// Inside an element to leave out (Word's list-bullet spans).
    skip: usize,
    /// Tables open inside the deepest one kept: their cells' contents go
    /// into its cell, one after another (see MAX_TABLES).
    flat_tables: usize,
}

/// Tables nest at most this deep (a saved document reads back only so deep,
/// and a made-up file nesting thousands would run out of stack).
const MAX_TABLES: usize = 16;

impl Builder {
    fn current_fmt(&self) -> CharFmt {
        self.fmt.iter().fold(CharFmt::default(), |a, f| a.with(f))
    }

    fn start_para(&mut self) {
        if self.para.is_some() {
            return;
        }
        let id = self.doc.alloc_id();
        let style = self.style.last().cloned().unwrap_or_else(|| StyleSheet::NORMAL.to_string());
        let mut p = Paragraph::new(id, &style, "");
        if let Some((ordered, _)) = self.lists.last() {
            let level = (self.lists.len() - 1).min(MAX_LEVEL as usize) as u8;
            let list = if *ordered { ListDef::NUMBERS } else { ListDef::BULLETS };
            p.list = Some(ListItem::new(list, level));
        }
        self.para = Some(p);
    }

    fn end_para(&mut self) {
        if let Some(mut p) = self.para.take() {
            // Trim the spaces collapsed whitespace leaves at the ends.
            let lead = p.text.len() - p.text.trim_start_matches(' ').len();
            if lead > 0 {
                p.replace_start(lead, Span::default());
            }
            let trailing = p.text.len() - p.text.trim_end_matches(' ').len();
            if trailing > 0 && self.pre == 0 {
                let s = p.slice(0..p.text.len() - trailing);
                p.text = s.text;
                p.runs = s.runs;
            }
            if !p.text.is_empty() || p.list.is_some() {
                p.normalize();
                self.blocks.push(Block::Paragraph(p));
            }
        }
    }

    fn text(&mut self, t: &str) {
        if self.skip > 0 {
            return;
        }
        let t = if self.pre > 0 {
            t.replace('\r', "")
        } else {
            let collapsed: String = t
                .chars()
                .map(|c| if c.is_whitespace() && c != '\u{a0}' { ' ' } else { c })
                .collect::<String>()
                .split(' ')
                .collect::<Vec<_>>()
                .join(" ");
            let mut c = collapsed;
            while c.contains("  ") {
                c = c.replace("  ", " ");
            }
            c
        };
        if t.is_empty() || (self.para.is_none() && t.trim().is_empty()) {
            return;
        }
        if self.pre > 0 && t.contains('\n') {
            let lines: Vec<&str> = t.split('\n').collect();
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    self.end_para();
                }
                self.push_text(line);
            }
            return;
        }
        let at_start = self.para.as_ref().is_none_or(|p| p.text.is_empty() || p.text.ends_with(' '));
        let t = if at_start { t.trim_start_matches(' ').to_string() } else { t };
        self.push_text(&t);
    }

    fn push_text(&mut self, t: &str) {
        if t.is_empty() {
            self.start_para();
            return;
        }
        self.start_para();
        let fmt = self.current_fmt();
        let p = self.para.as_mut().expect("started");
        let add = Span::plain(t, fmt);
        p.text.push_str(&add.text);
        p.runs.extend(add.runs);
    }

    fn image(&mut self, attrs: &[(String, String)]) {
        let Some(src) = attr(attrs, "src") else { return };
        let alt = attr(attrs, "alt").unwrap_or("").to_string();
        let num = |k: &str| attr(attrs, k).and_then(|v| v.trim_end_matches("px").parse::<f32>().ok());
        let (mut w, mut h) = (num("width").map(|v| v * 0.75), num("height").map(|v| v * 0.75));
        let key = if let Some(data) = src.strip_prefix("data:") {
            let (meta, payload) = data.split_once(',').unwrap_or(("", ""));
            if !meta.ends_with(";base64") {
                return;
            }
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload.trim()) else { return };
            if let Ok(img) = image::load_from_memory(&bytes) {
                w = w.or(Some(img.width() as f32 * 0.75));
                h = h.or(Some(img.height() as f32 * 0.75));
            }
            let ext = meta.split('/').nth(1).and_then(|m| m.split(';').next()).unwrap_or("png");
            let key = format!("media/pasted-{}.{ext}", self.doc.next_id);
            self.doc.assets.insert(key.clone(), std::sync::Arc::new(bytes));
            key
        } else {
            src.to_string()
        };
        self.end_para();
        let id = self.doc.alloc_id();
        self.blocks.push(Block::Image(Image {
            id,
            src: key,
            alt,
            width: w.unwrap_or(200.0),
            height: h.unwrap_or(150.0),
            ..Default::default()
        }));
    }

    fn open(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) {
        let styles = css(attrs);
        if self.skip > 0 {
            if !self_closing && !matches!(name, "br" | "img" | "hr") {
                self.skip += 1;
            }
            return;
        }
        if styles.iter().any(|(k, v)| k == "mso-list" && v.contains("ignore"))
            || styles.iter().any(|(k, v)| k == "display" && v == "none")
        {
            if !self_closing {
                self.skip = 1;
            }
            return;
        }
        match name {
            "p" | "div" | "section" | "article" | "header" | "footer" | "dt" | "dd" | "figcaption" => {
                // <li><p>…: the list item's paragraph is this one.
                if self.para.as_ref().is_some_and(|p| !p.text.is_empty()) {
                    self.end_para();
                }
                self.fmt.push(inline_fmt(name, attrs));
                self.start_para_with(attrs);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.end_para();
                let level = name[1..].parse().unwrap_or(1);
                self.style.push(StyleSheet::heading(level));
                self.fmt.push(inline_fmt(name, attrs));
                self.start_para_with(attrs);
            }
            "blockquote" => {
                self.end_para();
                self.style.push(StyleSheet::QUOTE.into());
            }
            "pre" => {
                self.end_para();
                self.pre += 1;
                self.style.push(StyleSheet::CODE.into());
            }
            "ul" | "ol" => {
                self.end_para();
                self.lists.push((name == "ol", 0));
            }
            "li" => {
                self.end_para();
                self.start_para();
            }
            "br" => {
                if self.para.is_some() {
                    self.end_para();
                    self.start_para();
                }
            }
            "hr" => self.end_para(),
            "img" => self.image(attrs),
            "table" | "tr" | "td" | "th" if self.flat_tables > 0 || name == "table" && self.tables.len() >= MAX_TABLES => {
                self.end_para();
                if name == "table" {
                    self.flat_tables += 1;
                }
            }
            "table" => {
                self.end_para();
                let outer = std::mem::take(&mut self.blocks);
                self.tables.push((TableBuild { rows: Vec::new() }, outer));
            }
            "tr" => {
                if let Some((t, _)) = self.tables.last_mut() {
                    t.rows.push(Vec::new());
                }
            }
            "td" | "th" => {
                self.end_para();
                let span = |k: &str| attr(attrs, k).and_then(|v| v.parse::<u16>().ok()).unwrap_or(1).max(1);
                let shading = styles
                    .iter()
                    .find(|(k, _)| k == "background-color" || k == "background")
                    .and_then(|(_, v)| parse_color(v))
                    .or_else(|| attr(attrs, "bgcolor").and_then(parse_color));
                if let Some((t, _)) = self.tables.last_mut() {
                    if t.rows.is_empty() {
                        t.rows.push(Vec::new());
                    }
                    let cell = CellBuild {
                        header: name == "th",
                        col_span: span("colspan"),
                        row_span: span("rowspan"),
                        shading,
                        blocks: Vec::new(),
                    };
                    t.rows.last_mut().expect("a row").push(cell);
                }
                self.blocks.clear();
                if name == "th" {
                    self.fmt.push(CharFmt { bold: Some(true), ..Default::default() });
                } else {
                    self.fmt.push(CharFmt::default());
                }
            }
            _ => {
                if !self_closing {
                    self.fmt.push(inline_fmt(name, attrs));
                }
            }
        }
    }

    fn start_para_with(&mut self, attrs: &[(String, String)]) {
        self.start_para();
        let mut props = ParaProps::default();
        for (k, v) in css(attrs) {
            if k == "text-align" {
                props.align = match v.as_str() {
                    "center" => Some(xenify_wp_doc::Align::Center),
                    "right" | "end" => Some(xenify_wp_doc::Align::Right),
                    "justify" => Some(xenify_wp_doc::Align::Justify),
                    _ => None,
                };
            }
        }
        if let Some(a) = attr(attrs, "align") {
            props.align = match a.to_ascii_lowercase().as_str() {
                "center" => Some(xenify_wp_doc::Align::Center),
                "right" => Some(xenify_wp_doc::Align::Right),
                "justify" => Some(xenify_wp_doc::Align::Justify),
                _ => props.align,
            };
        }
        if let Some(p) = self.para.as_mut() {
            p.props = props;
        }
    }

    fn close(&mut self, name: &str) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        match name {
            "p" | "div" | "section" | "article" | "header" | "footer" | "dt" | "dd" | "figcaption" => {
                self.end_para();
                self.fmt.pop();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.end_para();
                self.style.pop();
                self.fmt.pop();
            }
            "blockquote" => {
                self.end_para();
                self.style.pop();
            }
            "pre" => {
                self.end_para();
                self.pre = self.pre.saturating_sub(1);
                self.style.pop();
            }
            "ul" | "ol" => {
                self.end_para();
                self.lists.pop();
            }
            "li" => self.end_para(),
            "table" | "tr" | "td" | "th" if self.flat_tables > 0 => {
                self.end_para();
                if name == "table" {
                    self.flat_tables -= 1;
                }
            }
            "td" | "th" => {
                self.end_para();
                self.fmt.pop();
                let blocks = std::mem::take(&mut self.blocks);
                if let Some(cell) =
                    self.tables.last_mut().and_then(|(t, _)| t.rows.last_mut()).and_then(|r| r.last_mut())
                {
                    cell.blocks = blocks;
                }
            }
            "table" => {
                self.end_para();
                if let Some((t, outer)) = self.tables.pop() {
                    self.blocks = outer;
                    let table = self.build_table(t);
                    self.blocks.push(Block::Table(table));
                }
            }
            "br" | "img" | "hr" | "tr" | "tbody" | "thead" | "tfoot" => {}
            _ => {
                self.fmt.pop();
            }
        }
    }

    /// Places cells on a grid (row spans push later cells right).
    fn build_table(&mut self, t: TableBuild) -> Table {
        let rows: Vec<Vec<CellBuild>> = t.rows.into_iter().filter(|r| !r.is_empty()).collect();
        let nrows = rows.len().max(1);
        let mut taken: Vec<Vec<bool>> = vec![Vec::new(); nrows];
        let mut placed: Vec<(usize, usize, CellBuild)> = Vec::new();
        let mut cols = 1;
        for (r, row) in rows.into_iter().enumerate() {
            let mut c = 0;
            for cell in row {
                while taken[r].get(c).copied().unwrap_or(false) {
                    c += 1;
                }
                for row_taken in taken.iter_mut().take((r + cell.row_span as usize).min(nrows)).skip(r) {
                    for cc in c..c + cell.col_span as usize {
                        if row_taken.len() <= cc {
                            row_taken.resize(cc + 1, false);
                        }
                        row_taken[cc] = true;
                    }
                }
                cols = cols.max(c + cell.col_span as usize);
                let span = cell.col_span as usize;
                placed.push((r, c, cell));
                c += span;
            }
        }
        let doc = &mut self.doc;
        let mut alloc = || doc.alloc_id();
        let mut table = Table::new(nrows, cols, &mut alloc);
        for (r, c, cell) in placed {
            let target = &mut table.rows[r].cells[c];
            target.col_span = cell.col_span;
            target.row_span = cell.row_span;
            target.shading = cell.shading;
            if !cell.blocks.is_empty() {
                target.blocks = cell.blocks;
            }
            if cell.header && r == 0 {
                table.rows[0].header = true;
            }
        }
        table.normalize(&mut alloc);
        table
    }
}

/// Reads an HTML fragment (or page) into a document.
pub fn read(html: &str) -> Document {
    // The clipboard's HTML may wrap the fragment; keep what's marked.
    let html = match (html.find("<!--StartFragment-->"), html.find("<!--EndFragment-->")) {
        (Some(a), Some(b)) if b > a => &html[a + 20..b],
        _ => html,
    };
    let mut doc = Document::new();
    doc.sections[0].blocks.clear();
    let mut b = Builder {
        doc,
        blocks: Vec::new(),
        tables: Vec::new(),
        flat_tables: 0,
        fmt: Vec::new(),
        para: None,
        style: Vec::new(),
        lists: Vec::new(),
        pre: 0,
        skip: 0,
    };
    for t in tokenize(html) {
        match t {
            Token::Text(s) => b.text(&s),
            Token::Open(name, attrs, sc) => b.open(&name, &attrs, sc),
            Token::Close(name) => b.close(&name),
        }
    }
    b.end_para();
    while let Some((t, outer)) = b.tables.pop() {
        let mut blocks = outer;
        let table = b.build_table(t);
        blocks.push(Block::Table(table));
        b.blocks = blocks;
    }
    let mut doc = b.doc;
    doc.sections[0].blocks = b.blocks;
    if doc.sections[0].blocks.is_empty() {
        let id = doc.alloc_id();
        doc.sections[0].blocks.push(Block::Paragraph(Paragraph::new(id, StyleSheet::NORMAL, "")));
    }
    doc.repair();
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paras(d: &Document) -> Vec<(String, String)> {
        d.paragraphs().map(|p| (p.style.clone(), p.text.clone())).collect()
    }

    #[test]
    fn headings_paragraphs_and_formatting() {
        let d = read(
            "<h2>Plan</h2><p>Some <b>bold</b>, <em>italic</em> and <a href=\"https://x.org\">a link</a>&nbsp;here.</p>",
        );
        assert_eq!(paras(&d)[0], (StyleSheet::heading(2), "Plan".into()));
        let p = d.paragraphs().nth(1).unwrap();
        assert_eq!(p.text, "Some bold, italic and a link\u{a0}here.");
        let fmt_at = |w: &str| p.fmt_at(p.text.find(w).unwrap() + 1);
        assert_eq!(fmt_at("bold").bold, Some(true));
        assert_eq!(fmt_at("italic").italic, Some(true));
        assert_eq!(fmt_at("link").link.as_deref(), Some("https://x.org"));
    }

    #[test]
    fn google_docs_spans_and_lists() {
        let d = read(
            "<meta charset=\"utf-8\"><b style=\"font-weight:normal;\" id=\"docs-internal-guid-1\"><p dir=\"ltr\"><span style=\"font-weight:700;color:#ff0000\">Red bold</span><span style=\"font-weight:400\"> plain</span></p><ul><li><p><span>One</span></p></li><li><p><span>Two</span></p><ol><li>Nested</li></ol></li></ul></b>",
        );
        let ps: Vec<_> = d.paragraphs().collect();
        assert_eq!(ps[0].text, "Red bold plain");
        assert_eq!(ps[0].fmt_at(1).bold, Some(true));
        assert_eq!(ps[0].fmt_at(1).color, Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(ps[0].fmt_at(ps[0].text.len() - 1).bold, Some(false));
        let lists: Vec<_> =
            ps[1..].iter().map(|p| (p.text.as_str(), p.list.as_ref().map(|l| (l.list.as_str(), l.level)))).collect();
        assert_eq!(
            lists,
            [("One", Some(("bullets", 0))), ("Two", Some(("bullets", 0))), ("Nested", Some(("numbers", 1)))]
        );
    }

    #[test]
    fn tables_with_spans_and_pictures() {
        let png = {
            let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]));
            let mut v = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
            base64::engine::general_purpose::STANDARD.encode(v)
        };
        let d = read(&format!(
            "<table><tr><th>A</th><th>B</th></tr><tr><td colspan=2 style=\"background-color:#eeeeee\">wide</td></tr></table><p><img src=\"data:image/png;base64,{png}\" alt=\"dot\"></p>"
        ));
        let Block::Table(t) = &d.sections[0].blocks[0] else { panic!("{:?}", d.sections[0].blocks) };
        assert_eq!((t.rows.len(), t.cols()), (2, 2));
        assert!(t.rows[0].header);
        assert_eq!(t.rows[1].cells[0].col_span, 2);
        assert_eq!(t.rows[1].cells[0].shading, Some(Rgba::rgb(0xee, 0xee, 0xee)));
        let Block::Image(i) = &d.sections[0].blocks[1] else { panic!() };
        assert_eq!(i.alt, "dot");
        assert_eq!((i.width, i.height), (3.0, 1.5));
        assert!(d.assets.contains_key(&i.src));
    }

    #[test]
    fn word_list_markers_and_entities_are_handled() {
        let d = read(
            "<p class=MsoListParagraph style='mso-list:l0 level1 lfo1'><span style='mso-list:Ignore'>·<span>&nbsp;&nbsp;</span></span>Item &amp; more</p><!--[if gte mso 9]><xml>junk</xml><![endif]-->",
        );
        assert_eq!(d.paragraphs().next().unwrap().text, "Item & more");
    }
}
