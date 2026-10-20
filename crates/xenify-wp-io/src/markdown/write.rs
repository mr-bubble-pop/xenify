//! Document → Markdown (Pandoc flavour: front matter, fenced divs for
//! custom styles, bracketed spans for formatting Markdown has no syntax for).

use super::attrs::Attrs;
use super::front;
use xenify_wp_doc::{Block, CharFmt, Document, NodeId, NumberKind, Paragraph, StyleSheet, Table, lists};
use serde_json::Map;
use std::collections::HashMap;

/// Built-in styles with their own Markdown syntax.
fn heading_level(style: &str) -> Option<usize> {
    style.strip_prefix("heading").and_then(|n| n.parse().ok()).filter(|n| (1..=6).contains(n))
}

/// The font that marks inline code.
pub const MONO: &str = "monospace";

pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    if let Some(fm) = front::write(doc) {
        out.push_str("---\n");
        out.push_str(&fm);
        out.push_str("---\n\n");
    }
    let labels = lists::labels(doc);
    // Frames first (their position on the page is in their attributes).
    let all_frames: Vec<&xenify_wp_doc::Frame> = doc.sections.iter().flat_map(|s| &s.frames).collect();
    let name = |id: NodeId| all_frames.iter().position(|f| f.id == id).map(|i| format!("frame{}", i + 1));
    for (i, f) in all_frames.iter().enumerate() {
        let mut a = Attrs { id: Some(format!("frame{}", i + 1)), ..Default::default() };
        a.classes.push("frame".into());
        let r2 = |v: f32| format!("{}", (v * 100.0).round() / 100.0);
        if f.page > 0 {
            a.pairs.push(("page".into(), (f.page + 1).to_string()));
        }
        for (k, v) in [("x", f.x), ("y", f.y), ("width", f.width), ("height", f.height)] {
            a.pairs.push((k.into(), r2(v)));
        }
        let d = xenify_wp_doc::Frame::default();
        if f.wrap != d.wrap {
            a.pairs.push(("wrap".into(), f.wrap.key().into()));
        }
        if f.gap != d.gap {
            a.pairs.push(("gap".into(), r2(f.gap)));
        }
        if f.padding != d.padding {
            a.pairs.push(("padding".into(), r2(f.padding)));
        }
        if let Some(b) = f.border {
            a.pairs.push(("border".into(), r2(b.width)));
            a.pairs.push(("border-color".into(), b.color.to_hex()));
        }
        if let Some(c) = f.fill {
            a.pairs.push(("fill".into(), c.to_hex()));
        }
        if let Some(n) = f.next.and_then(name) {
            a.pairs.push(("next".into(), n));
        }
        out.push_str(&format!("::: {}\n", a.render()));
        let inner: Vec<&Block> = f.blocks.iter().collect();
        let mut body = String::new();
        write_blocks(doc, &labels, &inner, &mut body);
        out.push_str(body.trim_end_matches('\n'));
        out.push_str("\n:::\n\n");
    }
    // Rich headers and footers: `::: {.header}` divs before the text.
    if let Some(s) = doc.sections.first() {
        for h in &s.headers {
            let mut a = Attrs::default();
            a.classes.push(if h.footer { "footer" } else { "header" }.into());
            match h.slot {
                xenify_wp_doc::HfSlot::Default => {}
                xenify_wp_doc::HfSlot::First => a.pairs.push(("slot".into(), "first".into())),
                xenify_wp_doc::HfSlot::Even => a.pairs.push(("slot".into(), "even".into())),
            }
            out.push_str(&format!("::: {}\n", a.render()));
            let inner: Vec<&Block> = h.blocks.iter().collect();
            let mut body = String::new();
            write_blocks(doc, &labels, &inner, &mut body);
            out.push_str(body.trim_end_matches('\n'));
            out.push_str("\n:::\n\n");
        }
    }
    // Later sections (after a section break): `::: {.section}` divs with their start and page setup.
    for (i, s) in doc.sections.iter().enumerate() {
        let blocks: Vec<&Block> = s.blocks.iter().collect();
        if i == 0 {
            write_blocks(doc, &labels, &blocks, &mut out);
            continue;
        }
        let mut a = Attrs::default();
        a.classes.push("section".into());
        let start = match s.start {
            xenify_wp_doc::SectionStart::NewPage => "new-page",
            xenify_wp_doc::SectionStart::Continuous => "continuous",
        };
        a.pairs.push(("start".into(), start.into()));
        a.pairs.push(("page".into(), super::front::section_page_json(&s.page)));
        out.push_str(&format!("::: {}\n", a.render()));
        let mut body = String::new();
        write_blocks(doc, &labels, &blocks, &mut body);
        out.push_str(body.trim_end_matches('\n'));
        out.push_str("\n:::\n\n");
    }
    let notes = note_definitions(doc);
    if !notes.is_empty() {
        let trimmed = out.trim_end_matches('\n').len();
        out.truncate(trimmed);
        out.push_str("\n\n");
        out.push_str(&notes);
    }
    let trimmed = out.trim_end_matches('\n').len();
    out.truncate(trimmed);
    out
}

fn write_blocks(doc: &Document, labels: &HashMap<NodeId, String>, blocks: &[&Block], out: &mut String) {
    let mut i = 0;
    while i < blocks.len() {
        match blocks[i] {
            Block::Paragraph(_) => {
                let mut run = Vec::new();
                while let Some(Block::Paragraph(p)) = blocks.get(i) {
                    run.push(p);
                    i += 1;
                }
                write_paras(doc, labels, &run, out);
                continue;
            }
            Block::Table(t) => write_table(doc, t, out),
            Block::Image(img) => {
                let mut a = Attrs::default();
                a.classes.push("figure".into());
                for (k, v) in [("width", img.width), ("height", img.height)] {
                    if v > 0.0 {
                        a.pairs.push((k.into(), format!("{}", (v * 100.0).round() / 100.0)));
                    }
                }
                if img.align != xenify_wp_doc::Align::Left {
                    a.pairs.push(("align".into(), format!("{:?}", img.align).to_lowercase()));
                }
                if let Some(b) = img.border.filter(|b| b.width > 0.0) {
                    a.pairs
                        .push(("border".into(), format!("{} {}", (b.width * 100.0).round() / 100.0, b.color.to_hex())));
                }
                let src = if img.src.contains([' ', '(', ')']) { format!("<{}>", img.src) } else { img.src.clone() };
                out.push_str(&format!(
                    "::: {}
![{}]({src})
:::

",
                    a.render(),
                    escape(&img.alt, false)
                ));
            }
            Block::Toc(t) => {
                let mut a = Attrs::default();
                a.classes.push(
                    match t.kind {
                        xenify_wp_doc::TocKind::Contents => "toc",
                        xenify_wp_doc::TocKind::Bibliography => "bibliography",
                        xenify_wp_doc::TocKind::Figures => "figures",
                        xenify_wp_doc::TocKind::ChangeLog => "changelog",
                    }
                    .into(),
                );
                a.pairs.push(("levels".into(), t.levels.to_string()));
                if !t.page_numbers {
                    a.pairs.push(("page-numbers".into(), "false".into()));
                }
                out.push_str(&format!(
                    "::: {}
:::

",
                    a.render()
                ));
            }
        }
        i += 1;
    }
}

/// A table as a pipe table (in a `.table` div when it has settings Markdown can't show).
fn write_table(doc: &Document, t: &Table, out: &mut String) {
    let cols = t.cols();
    let cell_text = |r: usize, c: usize| -> String {
        let cell = &t.rows[r].cells[c];
        if cell.covered {
            return String::new();
        }
        cell.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(inline(doc, p)),
                // A picture in a cell: inline, its size in the title.
                Block::Image(img) => Some(format!(
                    "![{}]({} \"width={} height={}\")",
                    img.alt.replace(['[', ']', '|'], " "),
                    img.src.replace(' ', "%20"),
                    (img.width * 100.0).round() / 100.0,
                    (img.height * 100.0).round() / 100.0
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("<br>")
    };
    let mut a = Attrs::default();
    a.classes.push("table".into());
    for (i, r) in t.rows.iter().enumerate() {
        if let Some(list) = &r.repeat {
            a.pairs.push(("repeat".into(), format!("{}:{list}", i + 1)));
        }
    }
    let heads = t.rows.iter().take_while(|r| r.header).count();
    if heads != 1 {
        a.pairs.push(("header-rows".into(), heads.to_string()));
    }
    let total: f32 = t.widths.iter().sum();
    if t.widths.iter().any(|w| (w - t.widths[0]).abs() > 0.001) {
        let w: Vec<String> = t.widths.iter().map(|w| format!("{}", (w / total * 1000.0).round() / 10.0)).collect();
        a.pairs.push(("widths".into(), w.join(",")));
    }
    if (t.width - 1.0).abs() > 0.001 {
        a.pairs.push(("width".into(), format!("{}", (t.width * 1000.0).round() / 10.0)));
    }
    if t.align != xenify_wp_doc::Align::Left {
        a.pairs.push(("align".into(), format!("{:?}", t.align).to_lowercase()));
    }
    match t.border {
        None => a.pairs.push(("border".into(), "none".into())),
        Some(b) if b != xenify_wp_doc::Border::default() => {
            a.pairs.push(("border".into(), format!("{}", b.width)));
            a.pairs.push(("border-color".into(), b.color.to_hex()));
        }
        _ => {}
    }
    if t.header_col {
        a.pairs.push(("header-col".into(), "1".into()));
    }
    let feet = t.rows.iter().rev().take_while(|r| r.footer).count();
    if feet > 0 {
        a.pairs.push(("footer-rows".into(), feet.to_string()));
    }
    if t.repeat_header {
        a.pairs.push(("repeat-header".into(), "1".into()));
    }
    if t.repeat_footer {
        a.pairs.push(("repeat-footer".into(), "1".into()));
    }
    if let Some(c) = t.band {
        a.pairs.push(("band".into(), c.to_hex()));
    }
    if let Some(c) = t.header_fill {
        a.pairs.push(("header-fill".into(), c.to_hex()));
    }
    let heights: Vec<String> = t
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.min_height.map(|h| format!("{}:{}", i + 1, (h * 100.0).round() / 100.0)))
        .collect();
    if !heights.is_empty() {
        a.pairs.push(("heights".into(), heights.join(",")));
    }
    // What pipe tables can't hold, cell by cell: "row.col key=value …; …".
    let line = |b: &xenify_wp_doc::Border| format!("{}{}", (b.width * 100.0).round() / 100.0, b.color.to_hex());
    let mut cells = Vec::new();
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if cell.covered {
                continue;
            }
            let mut parts = Vec::new();
            if cell.col_span > 1 || cell.row_span > 1 {
                parts.push(format!("span={}x{}", cell.col_span, cell.row_span));
            }
            if let Some(f) = cell.shading {
                parts.push(format!("fill={}", f.to_hex()));
            }
            match cell.v_align {
                xenify_wp_doc::VAlign::Top => {}
                xenify_wp_doc::VAlign::Middle => parts.push("valign=middle".into()),
                xenify_wp_doc::VAlign::Bottom => parts.push("valign=bottom".into()),
            }
            if let Some(b) = cell.borders {
                for (k, side) in [("top", b.top), ("bottom", b.bottom), ("left", b.left), ("right", b.right)] {
                    if let Some(l) = side {
                        parts.push(format!("{k}={}", line(&l)));
                    }
                }
            }
            if !parts.is_empty() {
                cells.push(format!("{}.{} {}", r + 1, c + 1, parts.join(" ")));
            }
        }
    }
    if !cells.is_empty() {
        a.pairs.push(("cells".into(), cells.join("; ")));
    }
    let fenced = !a.pairs.is_empty();
    if fenced {
        out.push_str(&format!(
            "::: {}
",
            a.render()
        ));
    }
    for r in 0..t.rows.len() {
        let cells: Vec<String> = (0..cols).map(|c| cell_text(r, c)).collect();
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
        if r == 0 {
            out.push_str(&format!("|{}\n", "---|".repeat(cols)));
        }
    }
    if fenced {
        out.push_str(":::\n");
    }
    out.push('\n');
}

fn write_paras(doc: &Document, labels: &HashMap<NodeId, String>, paras: &[&Paragraph], out: &mut String) {
    let mut i = 0;
    // Content indents of the open list levels (for nesting).
    let mut list_indents: Vec<usize> = Vec::new();
    let mut prev_list: Option<(String, u8)> = None;
    while i < paras.len() {
        let p = paras[i];
        // Consecutive Code paragraphs become one fenced code block.
        if p.style == StyleSheet::CODE && p.list.is_none() && p.props.is_empty() {
            let mut lines = Vec::new();
            while i < paras.len() && paras[i].style == StyleSheet::CODE && paras[i].list.is_none() {
                lines.push(paras[i].text.as_str());
                i += 1;
            }
            let body = lines.join("\n");
            let fence = "`".repeat(longest_run(&body, '`').max(2) + 1);
            out.push_str(&format!("{fence}\n{body}\n{fence}\n\n"));
            list_indents.clear();
            prev_list = None;
            continue;
        }
        if xenify_wp_doc::ops::is_divider(p) {
            out.push_str("---\n\n");
            list_indents.clear();
            prev_list = None;
            i += 1;
            continue;
        }
        let text = inline(doc, p);
        // More of the item above: a paragraph indented under it (a blank line first).
        if let Some(item) = p.list.as_ref().filter(|i| i.follow) {
            let indent = list_indents.get(item.level as usize).copied().or(list_indents.last().copied()).unwrap_or(3);
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&" ".repeat(indent));
            out.push_str(&if text.is_empty() { "&nbsp;".to_string() } else { text });
            out.push('\n');
            let next_same = paras
                .get(i + 1)
                .and_then(|n| n.list.as_ref())
                .is_some_and(|n| n.list == item.list && n.restart.is_none());
            out.push('\n');
            if !next_same {
                list_indents.clear();
                prev_list = None;
            }
            i += 1;
            continue;
        }
        if let Some(item) = &p.list {
            let level = item.level as usize;
            // A different list (or a restart) starts a new Markdown list.
            let continues = prev_list.as_ref().is_some_and(|(l, _)| *l == item.list) && item.restart.is_none();
            if !continues {
                if !list_indents.is_empty() {
                    // Separate two adjacent lists so they don't merge.
                    out.push_str("<!-- -->\n\n");
                }
                list_indents.clear();
            }
            list_indents.truncate(level.min(list_indents.len()));
            let indent = list_indents.last().copied().unwrap_or(0);
            let def = doc.list_def(&item.list);
            let kind = def.map(|d| d.level(item.level).kind);
            let bullet = kind.is_none_or(|k| k == NumberKind::Bullet);
            let marker = if kind == Some(NumberKind::Check) {
                // GitHub task list.
                if item.checked { "- [x]".to_string() } else { "- [ ]".to_string() }
            } else if bullet {
                "-".to_string()
            } else {
                // The number of this item at its own level.
                let label = labels.get(&p.id).cloned().unwrap_or_default();
                let n = number_of(&label, def.map(|d| d.level(item.level).kind));
                format!("{n}.")
            };
            out.push_str(&" ".repeat(indent));
            out.push_str(&marker);
            out.push(' ');
            out.push_str(&if text.is_empty() { "&nbsp;".to_string() } else { text });
            out.push('\n');
            // Nested items line up with the text after "- " (a task box isn't part of the marker).
            list_indents.push(indent + if marker.starts_with("- [") { 2 } else { marker.len() + 1 });
            prev_list = Some((item.list.clone(), item.level));
            // Tight lists: no blank line between items of one list.
            let next_same = paras
                .get(i + 1)
                .and_then(|n| n.list.as_ref())
                .is_some_and(|n| n.list == item.list && n.restart.is_none());
            if !next_same {
                out.push('\n');
                list_indents.clear();
                prev_list = None;
            }
            i += 1;
            continue;
        }
        let body = if text.is_empty() { "&nbsp;".to_string() } else { text };
        let (block, needs_div) = match heading_level(&p.style) {
            Some(n) => {
                // A heading cross-references point at gets an anchor.
                let link = format!("#p{}", p.id.0);
                let referred = doc.paragraphs().any(|q| {
                    q.runs.iter().any(|r| {
                        matches!(&r.fmt.field, Some(xenify_wp_doc::Field::Ref { target }) if *target == p.id)
                            || r.fmt.link.as_deref() == Some(link.as_str())
                    })
                });
                let anchor = if referred { format!(" {{#ref-{}}}", p.id.0) } else { String::new() };
                (format!("{} {body}{anchor}", "#".repeat(n)), false)
            }
            None if p.style == StyleSheet::QUOTE => {
                (body.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n"), false)
            }
            None if p.style == StyleSheet::NORMAL => (body, false),
            None => (body, true),
        };
        let mut attrs = Attrs::default();
        if needs_div {
            let name = doc.styles.get(&p.style).map_or(p.style.clone(), |s| s.name.clone());
            attrs.pairs.push(("custom-style".into(), name));
        }
        let mut m = Map::new();
        front::props_json(&CharFmt::default(), &p.props, &mut m);
        for (k, v) in m {
            attrs.pairs.push((k, v.as_str().map_or(v.to_string(), str::to_string)));
        }
        if attrs.is_empty() {
            out.push_str(&block);
            out.push_str("\n\n");
        } else {
            out.push_str(&format!("::: {}\n{block}\n:::\n\n", attrs.render()));
        }
        i += 1;
    }
}

/// The number in a list label for the item's own level (the last number in
/// `1.2.a`-style labels), as decimal for Markdown.
fn number_of(label: &str, kind: Option<NumberKind>) -> u32 {
    let last = label.trim_end_matches(['.', ')']).rsplit(['.', ')']).next().unwrap_or("1");
    match kind {
        Some(NumberKind::LowerAlpha | NumberKind::UpperAlpha) => {
            last.to_ascii_lowercase().bytes().fold(0u32, |n, b| n * 26 + (b.saturating_sub(b'a') as u32 + 1))
        }
        Some(NumberKind::LowerRoman | NumberKind::UpperRoman) => from_roman(last),
        _ => last.parse().unwrap_or(1),
    }
    .max(1)
}

fn from_roman(s: &str) -> u32 {
    let v = |c: char| match c.to_ascii_uppercase() {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        'M' => 1000,
        _ => 0,
    };
    let chars: Vec<u32> = s.chars().map(v).collect();
    let mut n = 0i64;
    for (i, &c) in chars.iter().enumerate() {
        if chars.get(i + 1).is_some_and(|&next| next > c) {
            n -= c as i64;
        } else {
            n += c as i64;
        }
    }
    n.max(1) as u32
}

fn longest_run(s: &str, c: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for ch in s.chars() {
        if ch == c {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Escapes Markdown punctuation in running text.
fn escape(text: &str, at_line_start: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let start = at_line_start && i == 0;
        let needs = match c {
            // `$` too: `$…$` reads as maths.
            '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '~' | '|' | '^' | '$' => true,
            '{' => chars.get(i + 1) == Some(&'{'),
            '&' => {
                // Only when it would read as an entity.
                let rest: String = chars[i + 1..].iter().take(12).collect();
                rest.starts_with('#')
                    || rest.split(';').next().is_some_and(|w| {
                        !w.is_empty() && w.len() < 11 && w.chars().all(char::is_alphanumeric) && rest.contains(';')
                    })
            }
            '#' | '>' | '+' | '-' | '=' if start => true,
            _ => false,
        };
        if needs {
            out.push('\\');
        }
        out.push(c);
    }
    // "1. " at the start would start a list.
    if at_line_start
        && let Some(dot) = out.find(['.', ')'])
        && dot > 0
        && out[..dot].chars().all(|c| c.is_ascii_digit())
    {
        out.insert(dot, '\\');
    }
    out
}

/// What an inline run is wrapped in, outermost first.
#[derive(Clone, PartialEq)]
enum Mark {
    Link(String),
    Span(String),
    Strike,
    Bold,
    Italic,
    Sup,
    Sub,
}

impl Mark {
    fn open(&self) -> String {
        match self {
            Mark::Link(_) | Mark::Span(_) => "[".into(),
            Mark::Strike => "~~".into(),
            Mark::Bold => "**".into(),
            Mark::Italic => "*".into(),
            Mark::Sup => "^".into(),
            Mark::Sub => "~".into(),
        }
    }

    fn close(&self) -> String {
        match self {
            // `<…>` destinations may hold spaces and parentheses as they are.
            Mark::Link(url) if url.contains([' ', '(', ')', '<', '>']) => {
                format!("](<{}>)", url.replace('<', "%3C").replace('>', "%3E"))
            }
            Mark::Link(url) => format!("]({url})"),
            Mark::Span(attrs) => format!("]{attrs}"),
            Mark::Strike => "~~".into(),
            Mark::Bold => "**".into(),
            Mark::Italic => "*".into(),
            Mark::Sup => "^".into(),
            Mark::Sub => "~".into(),
        }
    }
}

/// `spaced`: the text has spaces (no `^…^` then: a span class).
fn marks_for(fmt: &CharFmt, style: &xenify_wp_doc::ResolvedChar, spaced: bool) -> (Vec<Mark>, bool) {
    let mut v = Vec::new();
    if let Some(l) = &fmt.link {
        // A link to a paragraph (#p12) points at its heading's anchor (#ref-12).
        let l = match l.strip_prefix("#p").filter(|n| n.parse::<u64>().is_ok()) {
            Some(n) => format!("#ref-{n}"),
            None => l.clone(),
        };
        v.push(Mark::Link(l));
    }
    let code = fmt.font.as_deref() == Some(MONO);
    // Everything without its own Markdown syntax goes in one bracketed span.
    let mut attrs = Attrs::default();
    if fmt.underline == Some(true) && fmt.link.is_none() {
        attrs.classes.push("underline".into());
    }
    if spaced {
        match fmt.vert {
            Some(xenify_wp_doc::VertAlign::Superscript) => attrs.classes.push("superscript".into()),
            Some(xenify_wp_doc::VertAlign::Subscript) => attrs.classes.push("subscript".into()),
            _ => {}
        }
    }
    if let Some(h) = fmt.highlight {
        if h == xenify_wp_doc::Rgba([255, 235, 59, 255]) {
            attrs.classes.push("mark".into());
        } else {
            attrs.pairs.push(("highlight".into(), h.to_hex()));
        }
    }
    if let Some(c) = fmt.color {
        attrs.pairs.push(("color".into(), c.to_hex()));
    }
    if let Some(s) = fmt.size {
        attrs.pairs.push(("size".into(), format!("{}", (s * 100.0).round() / 100.0)));
    }
    if let Some(f) = fmt.font.as_deref().filter(|f| *f != MONO) {
        attrs.pairs.push(("font".into(), f.to_string()));
    }
    // Turning off what the style turns on has no Markdown syntax either.
    for (k, direct, styled) in [
        ("bold", fmt.bold, style.bold),
        ("italic", fmt.italic, style.italic),
        ("underline", fmt.underline, style.underline),
        ("strike", fmt.strike, style.strike),
    ] {
        if direct == Some(false) && styled {
            attrs.pairs.push((k.into(), "false".into()));
        }
    }
    if !attrs.is_empty() {
        v.push(Mark::Span(attrs.render()));
    }
    if fmt.strike == Some(true) {
        v.push(Mark::Strike);
    }
    if fmt.bold == Some(true) {
        v.push(Mark::Bold);
    }
    if fmt.italic == Some(true) {
        v.push(Mark::Italic);
    }
    // Pandoc's ^superscript^ and ~subscript~.
    match fmt.vert.filter(|_| !spaced) {
        Some(xenify_wp_doc::VertAlign::Superscript) => v.push(Mark::Sup),
        Some(xenify_wp_doc::VertAlign::Subscript) => v.push(Mark::Sub),
        _ => {}
    }
    (v, code)
}

/// A paragraph's text as Markdown inline syntax.
/// A note's Markdown label: `1` for footnotes, `end-i` for endnotes.
fn note_label(doc: &Document, id: NodeId) -> String {
    let n = xenify_wp_doc::notes::numbers(doc).get(&id).cloned().unwrap_or_else(|| id.0.to_string());
    match doc.note(id).map(|n| n.kind) {
        Some(xenify_wp_doc::NoteKind::End) => format!("end-{n}"),
        _ => n,
    }
}

/// A field in Markdown: `[^1]` notes, Pandoc `[@key, p. 4]` citations,
/// and `[shown text]{field=page}` spans for the rest.
fn field_md(doc: &Document, field: &xenify_wp_doc::Field) -> String {
    use xenify_wp_doc::Field;
    let span = |shown: &str, attrs: &str| format!("[{}]{{{attrs}}}", escape(shown, false));
    match field {
        Field::Note { id } => format!("[^{}]", note_label(doc, *id)),
        Field::Citation { keys, locator } => {
            let keys: Vec<String> = keys.iter().map(|k| format!("@{k}")).collect();
            if locator.is_empty() {
                format!("[{}]", keys.join("; "))
            } else {
                format!("[{}, {locator}]", keys.join("; "))
            }
        }
        Field::Page => span("page", "field=page"),
        Field::Pages => span("pages", "field=pages"),
        Field::Date => span("date", "field=date"),
        Field::Badge { text, color } => {
            let q = text.replace('\\', "\\\\").replace('"', "\\\"");
            span(text, &format!("field=badge color={color} text=\"{q}\""))
        }
        Field::DateTime { format, fixed } => {
            let q = |v: &str| v.replace('\\', "\\\\").replace('"', "\\\"");
            let shown =
                fixed.clone().unwrap_or_else(|| xenify_wp_doc::datetime::format(format, &xenify_wp_layout::now()));
            let fixed = fixed.as_ref().map(|f| format!(" fixed=\"{}\"", q(f))).unwrap_or_default();
            span(&shown, &format!("field=datetime format=\"{}\"{fixed}", q(format)))
        }
        Field::Title => span(if doc.meta.title.is_empty() { "title" } else { &doc.meta.title }, "field=title"),
        Field::Author => span(if doc.meta.author.is_empty() { "author" } else { &doc.meta.author }, "field=author"),
        Field::Ref { target } => {
            let text = doc.para(*target).map_or("reference".to_string(), |p| p.text.clone());
            span(&text, &format!("field=ref target=#ref-{}", target.0))
        }
        // Pandoc's TeX maths.
        Field::Equation { latex, display: false, .. } => format!("${latex}$"),
        Field::Equation { latex, display: true, .. } => format!("$${latex}$$"),
        Field::Check { checked: true } => span("☒", "field=check checked=true"),
        Field::Check { checked: false } => span("☐", "field=check"),
    }
}

/// Footnote and endnote definitions (`[^1]: text`), after the body.
fn note_definitions(doc: &Document) -> String {
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    for p in doc.paragraphs() {
        for (_, id) in xenify_wp_doc::notes::references(p) {
            if !seen.insert(id) {
                continue;
            }
            let Some(note) = doc.note(id) else { continue };
            let paras: Vec<String> = note
                .blocks
                .iter()
                .filter_map(|b| if let Block::Paragraph(p) = b { Some(inline(doc, p)) } else { None })
                .collect();
            out.push_str(&format!("[^{}]: {}\n", note_label(doc, id), paras.first().cloned().unwrap_or_default()));
            for more in paras.iter().skip(1) {
                out.push_str(&format!("\n    {more}\n"));
            }
            out.push('\n');
        }
    }
    out
}

/// Pandoc's comment markers at offset `at` of `p`: `[text]{.comment-start …}`
/// where a comment's anchor starts (its replies after it) and
/// `[]{.comment-end id=…}` where it ends.
fn comment_markers(doc: &Document, p: &Paragraph, at: usize) -> String {
    if doc.meta.comments.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (cid, start, end) in xenify_wp_doc::review::anchors(doc) {
        let Some(c) = doc.meta.comments.iter().find(|c| c.id == cid) else { continue };
        if end.para == p.id && end.offset == at && (start.para != p.id || start.offset != at) {
            out.push_str(&format!("[]{{.comment-end id={}}}", cid.0));
        }
        if start.para == p.id && start.offset == at {
            let mut entries = vec![(format!("{}", cid.0), None, c.author.as_str(), c.date.as_str(), c.text.as_str())];
            for (i, r) in c.replies.iter().enumerate() {
                entries.push((
                    format!("{}-{}", cid.0, i + 1),
                    Some(cid.0),
                    r.author.as_str(),
                    r.date.as_str(),
                    r.text.as_str(),
                ));
            }
            for (i, (id, parent, author, date, text)) in entries.into_iter().enumerate() {
                let mut a = Attrs::default();
                a.classes.push("comment-start".into());
                a.pairs.push(("id".into(), id));
                if let Some(pid) = parent {
                    a.pairs.push(("parent".into(), pid.to_string()));
                }
                a.pairs.push(("author".into(), author.to_string()));
                if !date.is_empty() {
                    a.pairs.push(("date".into(), date.to_string()));
                }
                if i == 0 && c.resolved {
                    a.pairs.push(("done".into(), "true".into()));
                }
                let body = escape(&text.replace('\n', " "), false);
                out.push_str(&format!("[{body}]{}", a.render()));
            }
        }
    }
    out
}

fn inline(doc: &Document, p: &Paragraph) -> String {
    inline_with(doc, p, true)
}

/// `markers`: write comment markers (not for a piece of a paragraph).
fn inline_with(doc: &Document, p: &Paragraph, markers: bool) -> String {
    let style = doc.resolve_para(p).char;
    let mut out = String::new();
    let mut open: Vec<Mark> = Vec::new();
    for (range, fmt) in p.run_ranges() {
        let text = &p.text[range.clone()];
        let markers = if markers { comment_markers(doc, p, range.start) } else { String::new() };
        // Tracked changes and comment markers go outside other formatting.
        if fmt.change.is_some() || !markers.is_empty() {
            let trimmed_len = out.trim_end_matches([' ', '\t']).len();
            let ws = out[trimmed_len..].to_string();
            out.truncate(trimmed_len);
            for m in open.drain(..).rev() {
                out.push_str(&m.close());
            }
            out.push_str(&ws);
            out.push_str(&markers);
        }
        if let Some(change) = &fmt.change {
            // `[text]{.insertion author="…" date="…"}` (Pandoc's track-changes spans).
            let mut q = Paragraph::new(p.id, p.style.clone(), text);
            q.runs[0].fmt = CharFmt { change: None, comments: Vec::new(), ..fmt.clone() };
            let mut a = Attrs::default();
            a.classes.push(
                match change.kind {
                    xenify_wp_doc::review::ChangeKind::Inserted => "insertion",
                    xenify_wp_doc::review::ChangeKind::Deleted => "deletion",
                }
                .into(),
            );
            a.pairs.push(("author".into(), change.author.clone()));
            if !change.date.is_empty() {
                a.pairs.push(("date".into(), change.date.clone()));
            }
            out.push_str(&format!("[{}]{}", inline_with(doc, &q, false), a.render()));
            continue;
        }
        if let Some(field) = &fmt.field {
            // Fields go outside other formatting.
            let trimmed_len = out.trim_end_matches([' ', '\t']).len();
            let ws = out[trimmed_len..].to_string();
            out.truncate(trimmed_len);
            for m in open.drain(..).rev() {
                out.push_str(&m.close());
            }
            out.push_str(&ws);
            for _ in text.matches(xenify_wp_doc::Field::CHAR) {
                out.push_str(&field_md(doc, field));
            }
            continue;
        }
        let (want, code) = marks_for(fmt, &style, text.contains(char::is_whitespace));
        // Close marks down to the first that differs…
        let keep = open.iter().zip(&want).take_while(|(a, b)| a == b).count();
        // …moving whitespace from the end of the closed text after the closers.
        let trimmed_len = out.trim_end_matches([' ', '\t']).len();
        if keep < open.len() {
            let ws = out[trimmed_len..].to_string();
            out.truncate(trimmed_len);
            for m in open.drain(keep..).rev() {
                out.push_str(&m.close());
            }
            out.push_str(&ws);
        }
        // Leading whitespace goes before the openers.
        let lead_len = text.len() - text.trim_start_matches([' ', '\t']).len();
        let (lead, core) = text.split_at(lead_len);
        out.push_str(lead);
        for m in &want[keep..] {
            out.push_str(&m.open());
            open.push(m.clone());
        }
        let at_start = out.is_empty();
        if code {
            let ticks = "`".repeat(longest_run(core, '`') + 1);
            let pad = if core.starts_with('`') || core.ends_with('`') { " " } else { "" };
            out.push_str(&format!("{ticks}{pad}{core}{pad}{ticks}"));
        } else {
            out.push_str(&escape(core, at_start));
        }
    }
    let trimmed_len = out.trim_end_matches([' ', '\t']).len();
    let tail = out[trimmed_len..].to_string();
    out.truncate(trimmed_len);
    for m in open.drain(..).rev() {
        out.push_str(&m.close());
    }
    // Trailing spaces would be lost (or read as a line break); keep them visible.
    out.push_str(&tail.replace(' ', "&#32;"));
    if markers {
        out.push_str(&comment_markers(doc, p, p.text.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markdown_syntax() {
        assert_eq!(escape("a*b_c [d]", false), r"a\*b\_c \[d\]");
        assert_eq!(escape("# not a heading", true), r"\# not a heading");
        assert_eq!(escape("1. not a list", true), r"1\. not a list");
        assert_eq!(escape("Tom & Jerry", false), "Tom & Jerry");
        assert_eq!(escape("&amp;", false), r"\&amp;");
    }

    #[test]
    fn list_numbers() {
        assert_eq!(number_of("3.", Some(NumberKind::Decimal)), 3);
        assert_eq!(number_of("1.2.", Some(NumberKind::Decimal)), 2);
        assert_eq!(number_of("c.", Some(NumberKind::LowerAlpha)), 3);
        assert_eq!(number_of("iv.", Some(NumberKind::LowerRoman)), 4);
    }
}
