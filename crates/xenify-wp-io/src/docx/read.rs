//! .docx → document. Reads what the model holds: styles, direct formatting,
//! lists, sections (page size, margins, columns, headers/footers with page
//! fields), tables (merged cells, header rows, shading), pictures, links.
//! Anything else is reported.

use crate::zip::ZipReader;
use crate::{IoError, Loaded};
use crate::xml as Xml;
use roxmltree::Node;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use xenify_wp_doc::{
    Align, Block, Border, Cell, CharFmt, Document, Frame, HfLine, Image, ListDef, ListItem, ListLevel, NodeId,
    NumberKind, PageSetup, ParaProps, ParaStyle, Paragraph, Rgba, Row, Run, Section, SectionStart, StyleSheet, Table,
    Wrap,
};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn is(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

fn child<'a, 'i>(n: &Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(c, name))
}

fn wattr<'a>(n: &Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute((W, name)).or_else(|| n.attribute(name))
}

fn val<'a>(n: &Node<'a, '_>) -> Option<&'a str> {
    wattr(n, "val")
}

/// A `<w:b/>`-style switch: present and not `0`/`false`.
fn on(n: &Node) -> bool {
    !matches!(val(n), Some("0" | "false" | "off" | "none"))
}

fn twips(s: Option<&str>) -> Option<f32> {
    s?.parse::<f32>().ok().map(|v| v / 20.0)
}

fn color(s: Option<&str>) -> Option<Rgba> {
    let s = s?;
    if s == "auto" {
        return None;
    }
    Rgba::from_hex(s)
}

/// Word's highlight colour names.
fn highlight(name: &str) -> Option<Rgba> {
    Some(match name {
        "yellow" => Rgba([255, 255, 0, 255]),
        "green" => Rgba([0, 255, 0, 255]),
        "cyan" => Rgba([0, 255, 255, 255]),
        "magenta" => Rgba([255, 0, 255, 255]),
        "blue" => Rgba([0, 0, 255, 255]),
        "red" => Rgba([255, 0, 0, 255]),
        "darkBlue" => Rgba([0, 0, 128, 255]),
        "darkCyan" => Rgba([0, 128, 128, 255]),
        "darkGreen" => Rgba([0, 128, 0, 255]),
        "darkMagenta" => Rgba([128, 0, 128, 255]),
        "darkRed" => Rgba([128, 0, 0, 255]),
        "darkYellow" => Rgba([128, 128, 0, 255]),
        "darkGray" => Rgba([128, 128, 128, 255]),
        "lightGray" => Rgba([192, 192, 192, 255]),
        "black" => Rgba([0, 0, 0, 255]),
        _ => return None,
    })
}

thread_local! {
    /// The theme's heading and body fonts (word/theme/theme1.xml), while reading.
    static THEME_FONTS: std::cell::RefCell<(String, String)> = std::cell::RefCell::new(Default::default());
}

/// Reads the theme's major (headings) and minor (body) Latin fonts.
fn read_theme_fonts(zip: &ZipReader) -> (String, String) {
    let Ok(xml) = zip.read_string("word/theme/theme1.xml") else { return Default::default() };
    let Ok(x) = Xml::parse(&xml) else { return Default::default() };
    let face = |which: &str| {
        x.descendants()
            .find(|n| n.tag_name().name() == which)
            .and_then(|n| n.children().find(|c| c.tag_name().name() == "latin"))
            .and_then(|l| l.attribute("typeface"))
            .unwrap_or("")
            .to_string()
    };
    (face("majorFont"), face("minorFont"))
}

/// The line a `w:tblBorders` draws (its first visible edge).
fn table_border(b: &Node) -> Option<Border> {
    let edges: Vec<Node> = b.children().filter(Node::is_element).collect();
    let visible = edges.iter().find(|e| !matches!(val(e), Some("nil" | "none")))?;
    Some(Border {
        width: wattr(visible, "sz").and_then(|v| v.parse::<f32>().ok()).map_or(0.75, |v| v / 8.0),
        color: color(wattr(visible, "color")).unwrap_or(Border::default().color),
    })
}

/// Table styles' lines (`w:type="table"`), following `basedOn`.
fn read_table_styles(zip: &ZipReader) -> HashMap<String, Option<Border>> {
    let mut raw: HashMap<String, (Option<String>, Option<Option<Border>>)> = HashMap::new();
    if let Ok(xml) = zip.read_string("word/styles.xml")
        && let Ok(x) = Xml::parse(&xml)
    {
        for s in x.root_element().children().filter(|c| is(c, "style") && wattr(c, "type") == Some("table")) {
            let Some(id) = wattr(&s, "styleId") else { continue };
            let based = child(&s, "basedOn").and_then(|b| val(&b)).map(String::from);
            let borders = child(&s, "tblPr").and_then(|p| child(&p, "tblBorders")).map(|b| table_border(&b));
            raw.insert(id.to_string(), (based, borders));
        }
    }
    let mut out = HashMap::new();
    for id in raw.keys() {
        let mut cur = Some(id.clone());
        let mut found = None;
        for _ in 0..16 {
            let Some(c) = cur.take() else { break };
            let Some((based, borders)) = raw.get(&c) else { break };
            if let Some(b) = borders {
                found = Some(*b);
                break;
            }
            cur = based.clone();
        }
        out.insert(id.clone(), found.flatten());
    }
    out
}

/// Character styles (`w:type="character"`) flattened through `basedOn`.
fn read_char_styles(zip: &ZipReader) -> HashMap<String, CharFmt> {
    let mut raw: HashMap<String, (Option<String>, CharFmt)> = HashMap::new();
    if let Ok(xml) = zip.read_string("word/styles.xml")
        && let Ok(x) = Xml::parse(&xml)
    {
        for s in x.root_element().children().filter(|c| is(c, "style") && wattr(c, "type") == Some("character")) {
            let Some(id) = wattr(&s, "styleId") else { continue };
            let based = child(&s, "basedOn").and_then(|b| val(&b)).map(String::from);
            raw.insert(id.to_string(), (based, read_rpr(child(&s, "rPr"))));
        }
    }
    let mut out = HashMap::new();
    for id in raw.keys() {
        let mut chain = Vec::new();
        let mut cur = Some(id.clone());
        while let Some(c) = cur.take() {
            if chain.contains(&c) || chain.len() > 16 {
                break;
            }
            if let Some((based, _)) = raw.get(&c) {
                cur = based.clone();
            }
            chain.push(c);
        }
        let mut f = CharFmt::default();
        for c in chain.iter().rev() {
            if let Some((_, fmt)) = raw.get(c) {
                f = f.with(fmt);
            }
        }
        out.insert(id.clone(), f);
    }
    out
}

fn read_rpr(n: Option<Node>) -> CharFmt {
    let mut f = CharFmt::default();
    let Some(n) = n else { return f };
    for c in n.children().filter(Node::is_element) {
        match c.tag_name().name() {
            "b" => f.bold = Some(on(&c)),
            "i" => f.italic = Some(on(&c)),
            "strike" | "dstrike" => f.strike = Some(on(&c)),
            "u" => f.underline = Some(val(&c).is_some_and(|v| v != "none")),
            "color" => f.color = color(val(&c)),
            "sz" => f.size = val(&c).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 2.0),
            "highlight" => f.highlight = val(&c).and_then(highlight),
            "vertAlign" => {
                f.vert = match val(&c) {
                    Some("superscript") => Some(xenify_wp_doc::VertAlign::Superscript),
                    Some("subscript") => Some(xenify_wp_doc::VertAlign::Subscript),
                    _ => None,
                }
            }
            "shd" => {
                if let Some(fill) = color(wattr(&c, "fill")) {
                    f.highlight = Some(fill);
                }
            }
            "rFonts" => {
                // Our exports write the generic monospace font as Consolas.
                f.font = wattr(&c, "ascii")
                    .or_else(|| wattr(&c, "hAnsi"))
                    .map(|f| if f == "Consolas" { "monospace".into() } else { f.to_string() })
                    // Theme fonts ("minorHAnsi" → the theme's body font).
                    .or_else(|| {
                        let t = wattr(&c, "asciiTheme").or_else(|| wattr(&c, "hAnsiTheme"))?;
                        THEME_FONTS.with(|f| {
                            let (major, minor) = &*f.borrow();
                            let name = if t.starts_with("major") { major } else { minor };
                            (!name.is_empty()).then(|| name.clone())
                        })
                    });
            }
            _ => {}
        }
    }
    f
}

fn read_ppr(n: Option<Node>) -> ParaProps {
    let mut p = ParaProps::default();
    let Some(n) = n else { return p };
    for c in n.children().filter(Node::is_element) {
        match c.tag_name().name() {
            "jc" => {
                p.align = match val(&c) {
                    Some("center") => Some(Align::Center),
                    Some("right" | "end") => Some(Align::Right),
                    Some("both" | "distribute") => Some(Align::Justify),
                    Some("left" | "start") => Some(Align::Left),
                    _ => None,
                }
            }
            "spacing" => {
                p.space_before = twips(wattr(&c, "before"));
                p.space_after = twips(wattr(&c, "after"));
                if let Some(line) = wattr(&c, "line").and_then(|v| v.parse::<f32>().ok())
                    && matches!(wattr(&c, "lineRule"), Some("exact" | "atLeast"))
                {
                    p.line_exact = Some(line / 20.0);
                } else if let Some(line) = wattr(&c, "line").and_then(|v| v.parse::<f32>().ok())
                    && matches!(wattr(&c, "lineRule"), None | Some("auto"))
                {
                    p.line_height = Some(line / 240.0);
                }
            }
            "ind" => {
                p.indent_left = twips(wattr(&c, "left").or_else(|| wattr(&c, "start")));
                p.indent_right = twips(wattr(&c, "right").or_else(|| wattr(&c, "end")));
                p.first_line = twips(wattr(&c, "firstLine")).or_else(|| twips(wattr(&c, "hanging")).map(|h| -h));
            }
            "tabs" => {
                let mut stops: Vec<xenify_wp_doc::TabStop> = c
                    .children()
                    .filter(|t| is(t, "tab") && val(t) != Some("clear"))
                    .filter_map(|t| {
                        let pos = twips(wattr(&t, "pos"))?;
                        let kind = match val(&t) {
                            Some("center") => xenify_wp_doc::TabKind::Center,
                            Some("right" | "end") => xenify_wp_doc::TabKind::Right,
                            Some("decimal") => xenify_wp_doc::TabKind::Decimal,
                            _ => xenify_wp_doc::TabKind::Left,
                        };
                        let leader = match wattr(&t, "leader") {
                            Some("dot" | "middleDot") => xenify_wp_doc::TabLeader::Dots,
                            Some("hyphen") => xenify_wp_doc::TabLeader::Dashes,
                            Some("underscore" | "heavy") => xenify_wp_doc::TabLeader::Line,
                            _ => xenify_wp_doc::TabLeader::None,
                        };
                        Some(xenify_wp_doc::TabStop { pos, kind, leader })
                    })
                    .collect();
                stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
                if !stops.is_empty() {
                    p.tabs = Some(stops);
                }
            }
            "pBdr" => {
                let side = |name: &str| {
                    c.children().find(|e| is(e, name)).filter(|e| !matches!(val(e), Some("nil" | "none"))).map(|e| {
                        Border {
                            width: wattr(&e, "sz").and_then(|v| v.parse::<f32>().ok()).map_or(0.75, |v| v / 8.0),
                            color: color(wattr(&e, "color")).unwrap_or(Rgba::rgb(0, 0, 0)),
                        }
                    })
                };
                let b = xenify_wp_doc::ParaBorders {
                    top: side("top"),
                    bottom: side("bottom"),
                    left: side("left").or_else(|| side("start")),
                    right: side("right").or_else(|| side("end")),
                };
                if !b.is_empty() {
                    p.borders = Some(b);
                }
            }
            "shd" => p.shading = color(wattr(&c, "fill")),
            "keepNext" => p.keep_with_next = Some(on(&c)),
            "keepLines" => p.keep_together = Some(on(&c)),
            "pageBreakBefore" => p.page_break_before = Some(on(&c)),
            "widowControl" => p.widow_control = Some(on(&c)),
            _ => {}
        }
    }
    p
}

struct Numbering {
    /// numId → (list def id, start overrides by level).
    nums: HashMap<String, (String, HashMap<u8, u32>)>,
}

struct Reader<'a> {
    doc: Document,
    warnings: BTreeSet<String>,
    rels: HashMap<String, String>,
    zip: &'a ZipReader<'a>,
    numbering: Numbering,
    /// numIds seen (for restarts: the first paragraph of a numId with a start override).
    seen_nums: BTreeSet<String>,
    /// A paragraph became a checklist item (its list needs defining).
    uses_checklist: bool,
    style_ids: BTreeSet<String>,
    /// Frames of the section being read, with what's left to settle once its
    /// page setup is known: (x from the margin?, y from the margin?, x align, y align, auto height).
    frames: Vec<Frame>,
    pending: Vec<FramePending>,
    /// The `framePr` of the frame being filled (consecutive paragraphs with the same one share a frame).
    frame_key: Option<String>,
    /// Footnotes and endnotes read so far (they go in the first section).
    notes: Vec<xenify_wp_doc::Note>,
    /// Open complex fields (`w:fldChar`): the instruction so far, and
    /// whether its shown result is being skipped (a field we keep live).
    fields: Vec<(String, bool)>,
    /// The older form fields' settings (`w:ffData`), one per open field.
    form_data: Vec<Option<FormData>>,
    /// The form control the text being read belongs to.
    control: Option<NodeId>,
    /// Bookmark names → the paragraph they're in (for cross-references).
    bookmarks: HashMap<String, NodeId>,
    /// Cross-references waiting for their bookmark: placeholder id → name.
    refs: HashMap<NodeId, String>,
    /// settings.xml has `w:evenAndOddHeaders`.
    even_and_odd: bool,
    /// Inside `w:ins` / `w:del`: the change the runs carry.
    change: Option<xenify_wp_doc::review::Change>,
    /// Word comment ids whose range is open, and each Word id's thread.
    open_comments: Vec<String>,
    comment_threads: HashMap<String, NodeId>,
    /// Character styles by id.
    char_styles: HashMap<String, CharFmt>,
    /// Rich headers and footers of the section being read.
    pending_headers: Vec<xenify_wp_doc::HeaderPart>,
    /// Table styles' lines by id.
    table_styles: HashMap<String, Option<Border>>,
    /// Floating pictures' sides, by picture.
    float_aligns: HashMap<NodeId, Align>,
    /// Text boxes met in the paragraph being read (they follow it).
    textboxes: Vec<Block>,
    /// How deeply blocks and runs are nested where we are (see MAX_DEPTH).
    depth: usize,
    /// Tables we're inside (see MAX_TABLES).
    tables: usize,
}

/// Tables nested deeper than this become their cells' contents, one after
/// another (a saved document reads back only so deep).
pub(crate) const MAX_TABLES: usize = 16;

/// How deeply tables, content controls and other wrappers may nest. Real
/// documents go a few levels; a made-up one nesting thousands would run out
/// of stack. What's deeper is left out.
const MAX_DEPTH: usize = 64;

/// Comment threads from comments.xml (and commentsExtended.xml for replies
/// and resolved state), with each Word comment id's thread.
fn read_comments(
    zip: &ZipReader,
    alloc: &mut dyn FnMut() -> NodeId,
) -> (Vec<xenify_wp_doc::review::Comment>, HashMap<String, NodeId>) {
    use xenify_wp_doc::review::{Comment, Reply};
    let mut threads: Vec<Comment> = Vec::new();
    let mut by_word: HashMap<String, NodeId> = HashMap::new();
    let Ok(xml) = zip.read_string("word/comments.xml") else { return (threads, by_word) };
    let Ok(x) = Xml::parse(&xml) else { return (threads, by_word) };
    // paraId → (parent paraId, done)
    let mut ext: HashMap<String, (Option<String>, bool)> = HashMap::new();
    if let Ok(e) = zip.read_string("word/commentsExtended.xml")
        && let Ok(ex) = Xml::parse(&e)
    {
        for n in ex.descendants().filter(|n| n.tag_name().name() == "commentEx") {
            let get = |k: &str| n.attributes().find(|a| a.name() == k).map(|a| a.value().to_string());
            if let Some(pid) = get("paraId") {
                ext.insert(pid, (get("paraIdParent"), matches!(get("done").as_deref(), Some("1" | "true"))));
            }
        }
    }
    // paraId → thread
    let mut by_para: HashMap<String, NodeId> = HashMap::new();
    for c in x.root_element().children().filter(|c| is(c, "comment")) {
        let Some(wid) = wattr(&c, "id") else { continue };
        let author = wattr(&c, "author").unwrap_or("").to_string();
        let date = wattr(&c, "date").unwrap_or("").to_string();
        let paras: Vec<String> = c
            .children()
            .filter(|p| is(p, "p"))
            .map(|p| p.descendants().filter(|t| is(t, "t")).filter_map(|t| t.text()).collect::<String>())
            .collect();
        let text = paras.join("\n").trim().to_string();
        let para_id = c
            .children()
            .rfind(|p| is(p, "p"))
            .and_then(|p| p.attributes().find(|a| a.name() == "paraId").map(|a| a.value().to_string()));
        let (parent, done) = para_id.as_ref().and_then(|p| ext.get(p).cloned()).unwrap_or((None, false));
        let parent_thread = parent.as_ref().and_then(|p| by_para.get(p).copied());
        match parent_thread.and_then(|t| threads.iter_mut().find(|c| c.id == t)) {
            Some(thread) => {
                thread.replies.push(Reply { author, date, text });
                by_word.insert(wid.to_string(), thread.id);
                if let Some(p) = para_id {
                    by_para.insert(p, thread.id);
                }
            }
            None => {
                let id = alloc();
                threads.push(Comment { id, author, date, text, resolved: done, ..Default::default() });
                by_word.insert(wid.to_string(), id);
                if let Some(p) = para_id {
                    by_para.insert(p, id);
                }
            }
        }
    }
    (threads, by_word)
}

/// Word keeps header and band colours in table styles; Xenify Write writes
/// them onto the cells. Back again: a colour most header cells (or most
/// banded-row cells) share becomes the table's header colour (band colour),
/// and those cells lose it, so the table's own settings keep working.
fn table_look_from_fills(t: &mut Table, banding: bool) {
    let mut body = 0;
    let mut groups: [Vec<(usize, usize)>; 2] = [Vec::new(), Vec::new()];
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if cell.covered {
                continue;
            }
            if row.header || t.header_col && c == 0 {
                groups[0].push((r, c));
            } else if banding && body % 2 == 1 {
                groups[1].push((r, c));
            }
        }
        if !row.header {
            body += 1;
        }
    }
    for (g, cells) in groups.iter().enumerate() {
        let mut counts: Vec<(Rgba, usize)> = Vec::new();
        for &(r, c) in cells {
            if let Some(f) = t.rows[r].cells[c].shading {
                match counts.iter_mut().find(|(x, _)| *x == f) {
                    Some(e) => e.1 += 1,
                    None => counts.push((f, 1)),
                }
            }
        }
        // On a tie, the lighter colour (header and band colours are pale).
        let light = |c: &Rgba| c.0[0] as u32 + c.0[1] as u32 + c.0[2] as u32;
        let Some(&(fill, n)) = counts.iter().max_by_key(|(c, n)| (*n, light(c))) else { continue };
        if n * 2 < cells.len() {
            continue;
        }
        for &(r, c) in cells {
            let cell = &mut t.rows[r].cells[c];
            if cell.shading == Some(fill) {
                cell.shading = None;
            }
        }
        if g == 0 {
            t.header_fill = Some(fill);
        } else {
            t.band = Some(fill);
        }
    }
}

impl Reader<'_> {
    /// A field Write keeps live, from Word's field instruction.
    fn field_of(&mut self, instr: &str) -> Option<xenify_wp_doc::Field> {
        use xenify_wp_doc::Field;
        let mut words = instr.split_whitespace();
        let name = words.next()?.to_ascii_uppercase();
        Some(match name.as_str() {
            "PAGE" => Field::Page,
            "NUMPAGES" => Field::Pages,
            // DATE \@ "d MMMM yyyy": the same format codes as Write's.
            "DATE" if instr.contains("\\@") => {
                let format = instr.split('"').nth(1).unwrap_or(xenify_wp_doc::datetime::PRESETS[0]).to_string();
                Field::DateTime { format, fixed: None }
            }
            "DATE" | "CREATEDATE" | "SAVEDATE" => Field::Date,
            "TITLE" => Field::Title,
            "AUTHOR" => Field::Author,
            "CITATION" => {
                let mut keys = vec![words.next()?.to_string()];
                let mut locator = String::new();
                let rest: Vec<&str> = words.collect();
                let mut i = 0;
                while i < rest.len() {
                    match rest[i] {
                        "\\m" if i + 1 < rest.len() => {
                            keys.push(rest[i + 1].to_string());
                            i += 1;
                        }
                        "\\p" if i + 1 < rest.len() => {
                            let v = rest[i + 1].trim_matches('"');
                            locator = if v.contains(['-', '–']) { format!("pp. {v}") } else { format!("p. {v}") };
                            i += 1;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                Field::Citation { keys, locator }
            }
            "REF" => {
                let name = words.next()?.to_string();
                let placeholder = self.id();
                self.refs.insert(placeholder, name);
                Field::Ref { target: placeholder }
            }
            _ => return None,
        })
    }

    /// Footnote or endnote `id` from footnotes.xml / endnotes.xml, as a note.
    fn read_note(&mut self, kind: xenify_wp_doc::NoteKind, word_id: &str) -> Option<NodeId> {
        let (part, tag) = match kind {
            xenify_wp_doc::NoteKind::Foot => ("word/footnotes.xml", "footnote"),
            xenify_wp_doc::NoteKind::End => ("word/endnotes.xml", "endnote"),
        };
        let xml = self.zip.read_string(part).ok()?;
        let x = Xml::parse(&xml).ok()?;
        let n = x.root_element().children().find(|c| is(c, tag) && wattr(c, "id") == Some(word_id))?;
        // Notes have their own fields; keep the body's state apart.
        let outer = std::mem::take(&mut self.fields);
        let mut blocks = self.blocks(&n);
        self.fields = outer;
        // Word starts a note with its number mark and a space.
        if let Some(Block::Paragraph(p)) = blocks.first_mut() {
            let lead = p.text.len() - p.text.trim_start().len();
            if lead > 0 {
                p.replace_start(lead, xenify_wp_doc::Span::default());
            }
        }
        if blocks.is_empty() {
            blocks.push(Block::Paragraph(Paragraph::new(self.id(), StyleSheet::NOTE, "")));
        }
        for b in &mut blocks {
            if let Block::Paragraph(p) = b
                && (p.style == StyleSheet::NORMAL || p.style.contains("text"))
            {
                p.style = StyleSheet::NOTE.into();
            }
        }
        let id = self.id();
        self.notes.push(xenify_wp_doc::Note { id, kind, blocks });
        Some(id)
    }
}

/// A Word frame from a paragraph's `w:framePr` (and its border and shading).
/// What's left to settle about a frame once its section's page setup is known:
/// (x from the margin?, y from the margin?, x align, y align, auto height).
type FramePending = (bool, bool, Option<String>, Option<String>, bool);

fn read_frame(pr: &Node, ppr: &Node, id: NodeId) -> (Frame, FramePending) {
    let tw = |k: &str| wattr(pr, k).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 20.0);
    let bdr = child(ppr, "pBdr");
    let border = bdr
        .and_then(|b| b.children().find(|e| e.is_element() && !matches!(val(e), Some("nil" | "none"))))
        .map(|e| Border {
            width: wattr(&e, "sz").and_then(|v| v.parse::<f32>().ok()).map_or(0.75, |v| v / 8.0),
            color: color(wattr(&e, "color")).unwrap_or(Border::default().color),
        });
    let padding = bdr
        .and_then(|b| b.children().find(|e| e.is_element()))
        .and_then(|e| wattr(&e, "space"))
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(Frame::default().padding);
    let wrap = match wattr(pr, "wrap") {
        Some("notBeside") => Wrap::TopBottom,
        Some("none") => Wrap::InFront,
        _ => Wrap::Around,
    };
    let auto_h = !matches!(wattr(pr, "hRule"), Some("exact" | "atLeast")) || tw("h").is_none();
    let f = Frame {
        id,
        page: 0,
        x: tw("x").unwrap_or(0.0) - padding,
        y: tw("y").unwrap_or(0.0) - padding,
        width: tw("w").unwrap_or(144.0) + 2.0 * padding,
        height: tw("h").unwrap_or(0.0) + 2.0 * padding,
        wrap,
        gap: tw("hSpace").unwrap_or(Frame::default().gap),
        border,
        fill: child(ppr, "shd").and_then(|sh| color(wattr(&sh, "fill"))),
        padding,
        blocks: Vec::new(),
        next: None,
    };
    let page = |k: &str| matches!(wattr(pr, k), Some("page"));
    (
        f,
        (
            !page("hAnchor"),
            !page("vAnchor"),
            wattr(pr, "xAlign").map(str::to_string),
            wattr(pr, "yAlign").map(str::to_string),
            auto_h,
        ),
    )
}

impl Reader<'_> {
    /// Puts the section's frames in page coordinates now its page setup is known.
    fn settle_frames(&mut self, page: &PageSetup) -> Vec<Frame> {
        let mut frames = std::mem::take(&mut self.frames);
        for (f, (hm, vm, xa, ya, auto_h)) in frames.iter_mut().zip(std::mem::take(&mut self.pending)) {
            let m = &page.margins;
            if auto_h {
                // Word sizes these to their text: about a line per paragraph.
                f.height = f.height.max(f.blocks.len() as f32 * 16.0 + 2.0 * f.padding);
            }
            let (left, right) = if hm { (m.left, page.width - m.right) } else { (0.0, page.width) };
            f.x = match xa.as_deref() {
                Some("center") => (left + right - f.width) / 2.0,
                Some("right" | "outside") => right - f.width,
                Some("left" | "inside") => left,
                _ => f.x + if hm { m.left } else { 0.0 },
            };
            let (top, bottom) = if vm { (m.top, page.height - m.bottom) } else { (0.0, page.height) };
            f.y = match ya.as_deref() {
                Some("center") => (top + bottom - f.height) / 2.0,
                Some("bottom" | "outside") => bottom - f.height,
                Some("top" | "inside") => top,
                _ => f.y + if vm { m.top } else { 0.0 },
            };
        }
        frames
    }

    fn id(&mut self) -> NodeId {
        self.doc.alloc_id()
    }

    /// A content control around whole paragraphs: their text is its value.
    fn block_control(&mut self, sdt: &Node, content: &Node) -> Vec<Block> {
        let outer = self.control;
        if let Some(control) = child(sdt, "sdtPr").and_then(|pr| self.new_control(&pr)) {
            self.control = Some(control);
        }
        let blocks = self.blocks(content);
        self.control = outer;
        blocks
    }

    /// A content control's settings (`w:sdtPr`) as a [`Control`] in the
    /// document; None for other content controls (citations, contents…).
    fn new_control(&mut self, pr: &Node) -> Option<NodeId> {
        use xenify_wp_doc::controls::{Control, ControlKind};
        let items = |list: &Node| -> Vec<String> {
            list.children()
                .filter(|i| is(i, "listItem"))
                .filter_map(|i| wattr(&i, "displayText").or_else(|| wattr(&i, "value")).map(str::to_string))
                .collect()
        };
        let kind = if child(pr, "text").is_some() {
            ControlKind::Text { rich: false }
        } else if let Some(d) = child(pr, "date") {
            ControlKind::Date { format: child(&d, "dateFormat").and_then(|f| val(&f)).unwrap_or("").to_string() }
        } else if let Some(l) = child(pr, "dropDownList") {
            ControlKind::Dropdown { items: items(&l) }
        } else if let Some(l) = child(pr, "comboBox") {
            ControlKind::ComboBox { items: items(&l) }
        } else if ["citation", "docPartObj", "docPartList", "bibliography", "equation", "picture", "group"]
            .iter()
            .any(|k| child(pr, k).is_some())
        {
            return None;
        } else {
            ControlKind::Text { rich: true }
        };
        let id = self.id();
        self.doc.meta.controls.push(Control {
            id,
            kind,
            title: child(pr, "alias").and_then(|a| val(&a)).unwrap_or("").to_string(),
            tag: child(pr, "tag").and_then(|a| val(&a)).unwrap_or("").to_string(),
            placeholder: child(pr, "showingPlcHdr").is_some_and(|s| on(&s)),
        });
        Some(id)
    }

    /// One of Word's older form fields (FORMCHECKBOX, FORMDROPDOWN,
    /// FORMTEXT), as a checkbox or a control. `shown`: its result follows.
    /// Returns whether `instr` was one.
    fn form_field(&mut self, instr: &str, p: &mut Paragraph, fmt: &xenify_wp_doc::CharFmt, shown: bool) -> bool {
        use xenify_wp_doc::controls::{Control, ControlKind};
        let name = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        let data = self.form_data.last().cloned().flatten().unwrap_or_default();
        match name.as_str() {
            "FORMCHECKBOX" => {
                let plain = xenify_wp_doc::CharFmt { control: None, ..fmt.clone() };
                push_field(p, xenify_wp_doc::Field::Check { checked: data.checked }, plain);
            }
            "FORMDROPDOWN" => {
                let id = self.id();
                let pick = data.items.get(data.selected).cloned().unwrap_or_default();
                self.doc.meta.controls.push(Control {
                    id,
                    kind: ControlKind::Dropdown { items: data.items },
                    title: data.name,
                    tag: String::new(),
                    placeholder: false,
                });
                let at = p.text.len();
                p.text.push_str(&pick);
                if !pick.is_empty() {
                    p.runs.push(Run {
                        len: p.text.len() - at,
                        fmt: xenify_wp_doc::CharFmt { control: Some(id), ..fmt.clone() },
                    });
                }
            }
            // The typed text is the field's result: it's read as the control's text.
            "FORMTEXT" if shown => {
                let id = self.id();
                self.doc.meta.controls.push(Control {
                    id,
                    kind: ControlKind::Text { rich: false },
                    title: data.name,
                    tag: String::new(),
                    placeholder: false,
                });
                self.control = Some(id);
            }
            _ => return false,
        }
        true
    }

    fn paragraph(&mut self, n: &Node) -> Vec<Block> {
        let id = self.id();
        let mut p = Paragraph::new(id, StyleSheet::NORMAL, "");
        let ppr = child(n, "pPr");
        if let Some(ppr) = ppr {
            if let Some(s) = child(&ppr, "pStyle").and_then(|s| val(&s)) {
                let s = s.to_ascii_lowercase();
                p.style = if self.style_ids.contains(&s) { s } else { StyleSheet::NORMAL.into() };
            }
            if let Some(np) = child(&ppr, "numPr") {
                let level = child(&np, "ilvl").and_then(|v| val(&v)).and_then(|v| v.parse().ok()).unwrap_or(0);
                if let Some(num) = child(&np, "numId").and_then(|v| val(&v))
                    && let Some((list, starts)) = self.numbering.nums.get(num).cloned()
                {
                    let first = self.seen_nums.insert(num.to_string());
                    let restart = if first { starts.get(&level).copied() } else { None };
                    p.list = Some(ListItem { list, level, restart, checked: false, follow: false });
                }
            }
            p.props = read_ppr(Some(ppr));
            // A tracked change on the paragraph mark (its break inserted or deleted).
            if let Some(mark) = child(&ppr, "rPr")
                && let Some(c) = mark.children().find(|c| is(c, "ins") || is(c, "del"))
            {
                p.props.break_change = Some(xenify_wp_doc::review::Change {
                    kind: if is(&c, "ins") {
                        xenify_wp_doc::review::ChangeKind::Inserted
                    } else {
                        xenify_wp_doc::review::ChangeKind::Deleted
                    },
                    author: wattr(&c, "author").unwrap_or("").to_string(),
                    date: wattr(&c, "date").unwrap_or("").to_string(),
                });
            }
            // List items take their indent from the list.
            if p.list.is_some() {
                p.props.indent_left = None;
            }
        }
        let p_align = p.props.align.unwrap_or_default();
        let mut images = Vec::new();
        self.runs(n, &mut p, None, &mut images);
        // A checklist item: a box character, then a tab or space.
        if p.list.is_none() {
            let box_char = ["\u{2610}", "\u{2611}", "\u{2612}"].into_iter().find(|b| p.text.starts_with(b));
            if let Some(b) = box_char {
                let gap =
                    p.text[b.len()..].chars().next().filter(|c| *c == '\t' || *c == ' ').map_or(0, |c| c.len_utf8());
                p.replace_start(b.len() + gap, Default::default());
                p.list =
                    Some(ListItem { checked: b != "\u{2610}", ..ListItem::new(xenify_wp_doc::ListDef::CHECKLIST, 0) });
                p.props.indent_left = None;
                p.props.first_line = None;
                self.uses_checklist = true;
            }
        }
        p.normalize();
        let mut out = Vec::new();
        // A paragraph holding only a picture becomes a picture block.
        let has_text = !p.text.trim().is_empty();
        if has_text || images.is_empty() {
            out.push(Block::Paragraph(p));
        }
        let align = p_align;
        out.extend(images.into_iter().map(|mut i| {
            i.align = self.float_aligns.remove(&i.id).unwrap_or(align);
            Block::Image(i)
        }));
        out.append(&mut self.textboxes);
        out
    }

    fn runs(&mut self, n: &Node, p: &mut Paragraph, link: Option<&str>, images: &mut Vec<Image>) {
        if self.depth >= MAX_DEPTH {
            self.warnings.insert("some deeply nested text was left out".into());
            return;
        }
        self.depth += 1;
        self.runs_in(n, p, link, images);
        self.depth -= 1;
    }

    fn runs_in(&mut self, n: &Node, p: &mut Paragraph, link: Option<&str>, images: &mut Vec<Image>) {
        for c in n.children().filter(Node::is_element) {
            match c.tag_name().name() {
                "r" => self.run(&c, p, link, images),
                "hyperlink" => {
                    let url = c.attribute((R, "id")).and_then(|id| self.rels.get(id).cloned());
                    let url = url.or_else(|| wattr(&c, "anchor").map(|a| format!("#{a}")));
                    self.runs(&c, p, url.as_deref(), images);
                }
                "fldSimple" => {
                    let instr = wattr(&c, "instr").unwrap_or("").to_string();
                    match self.field_of(&instr) {
                        Some(f) => {
                            let fmt =
                                c.descendants().find(|d| is(d, "rPr")).map(|r| read_rpr(Some(r))).unwrap_or_default();
                            push_field(p, f, fmt);
                        }
                        None => self.runs(&c, p, link, images),
                    }
                }
                "bookmarkStart" => {
                    if let Some(name) = wattr(&c, "name") {
                        self.bookmarks.insert(name.to_string(), p.id);
                    }
                }
                "ins" | "del" => {
                    let kind = if c.tag_name().name() == "ins" {
                        xenify_wp_doc::review::ChangeKind::Inserted
                    } else {
                        xenify_wp_doc::review::ChangeKind::Deleted
                    };
                    let outer = self.change.replace(xenify_wp_doc::review::Change {
                        kind,
                        author: wattr(&c, "author").unwrap_or("").to_string(),
                        date: wattr(&c, "date").unwrap_or("").to_string(),
                    });
                    self.runs(&c, p, link, images);
                    self.change = outer;
                }
                "commentRangeStart" => {
                    if let Some(id) = wattr(&c, "id") {
                        self.open_comments.push(id.to_string());
                    }
                }
                "commentRangeEnd" => {
                    if let Some(id) = wattr(&c, "id") {
                        self.open_comments.retain(|x| x != id);
                    }
                }
                // A content control: a checkbox, or text to fill in.
                "sdt" => {
                    let pr = child(&c, "sdtPr");
                    if let Some(checked) = pr.as_ref().and_then(checkbox) {
                        push_field(p, xenify_wp_doc::Field::Check { checked }, Default::default());
                    } else if let Some(content) = child(&c, "sdtContent") {
                        let outer = self.control;
                        if let Some(control) = pr.as_ref().and_then(|pr| self.new_control(pr)) {
                            self.control = Some(control);
                        }
                        self.runs(&content, p, link, images);
                        self.control = outer;
                    }
                }
                "smartTag" | "sdtContent" | "customXml" => self.runs(&c, p, link, images),
                // An equation: kept as LaTeX, with Word's own markup to write back unchanged.
                name @ ("oMath" | "oMathPara") => {
                    let display = name == "oMathPara";
                    let raw = &c.document().input_text()[c.range()];
                    let omml = if raw.starts_with("<m:") { raw.to_string() } else { String::new() };
                    if display && p.text.is_empty() {
                        p.props.align = Some(xenify_wp_doc::Align::Center);
                    }
                    let latex = super::omml::to_latex(&c);
                    push_field(p, xenify_wp_doc::Field::Equation { latex, display, omml }, Default::default());
                }
                _ => {}
            }
        }
    }

    fn run(&mut self, r: &Node, p: &mut Paragraph, link: Option<&str>, images: &mut Vec<Image>) {
        let rpr = child(r, "rPr");
        let mut fmt = read_rpr(rpr);
        // A character style under the run's own formatting (Word's note and
        // comment marks are drawn by Write itself).
        if let Some(id) = rpr.and_then(|p| child(&p, "rStyle")).and_then(|s| val(&s))
            && !id.ends_with("Reference")
            && let Some(base) = self.char_styles.get(id)
        {
            fmt = base.with(&fmt);
        }
        // All caps: the text as capitals.
        let caps = rpr.and_then(|p| child(&p, "caps").or_else(|| child(&p, "smallCaps"))).is_some_and(|c| on(&c));
        if let Some(l) = link {
            fmt.link = Some(l.to_string());
        }
        fmt.change = self.change.clone();
        fmt.control = self.control;
        for w in &self.open_comments {
            if let Some(t) = self.comment_threads.get(w)
                && !fmt.comments.contains(t)
            {
                fmt.comments.push(*t);
            }
        }
        for c in r.children().filter(Node::is_element) {
            // Complex fields: begin, instruction, separate, (result), end.
            match c.tag_name().name() {
                "fldChar" => {
                    match wattr(&c, "fldCharType") {
                        Some("begin") => {
                            self.fields.push((String::new(), false));
                            self.form_data.push(child(&c, "ffData").map(|d| form_data(&d)));
                        }
                        Some("separate") => {
                            if let Some((instr, skip)) = self.fields.last().cloned()
                                && !skip
                                && self.form_field(&instr, p, &fmt, true)
                            {
                                if let Some(top) = self.fields.last_mut() {
                                    top.1 = !instr.trim_start().starts_with("FORMTEXT");
                                }
                            } else if let Some((instr, skip)) = self.fields.last().cloned()
                                && !skip
                                && let Some(f) = self.field_of(&instr)
                            {
                                push_field(p, f, fmt.clone());
                                if let Some(top) = self.fields.last_mut() {
                                    top.1 = true;
                                }
                            }
                        }
                        Some("end") => {
                            let form = self.form_data.pop().flatten();
                            let popped = self.fields.pop();
                            if popped.as_ref().is_some_and(|(i, _)| i.trim_start().starts_with("FORMTEXT")) {
                                self.control = None;
                            }
                            // A field with no shown result.
                            if let Some((instr, false)) = popped {
                                self.form_data.push(form);
                                let done = self.form_field(&instr, p, &fmt, false);
                                self.form_data.pop();
                                if !done && let Some(f) = self.field_of(&instr) {
                                    push_field(p, f, fmt.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                "instrText" => {
                    if let Some(top) = self.fields.last_mut() {
                        top.0.push_str(c.text().unwrap_or(""));
                    }
                    continue;
                }
                _ => {}
            }
            // The cached result of a field that's kept live.
            if self.fields.iter().any(|(_, skip)| *skip) {
                continue;
            }
            let text = match c.tag_name().name() {
                "t" | "delText" => c.text().unwrap_or("").to_string(),
                "tab" => "\t".into(),
                // A page or column break starts the paragraph on a new page or column.
                "br" if matches!(wattr(&c, "type"), Some("page" | "column")) => {
                    if wattr(&c, "type") == Some("page") {
                        p.props.page_break_before = Some(true);
                    } else {
                        p.props.column_break_before = Some(true);
                    }
                    continue;
                }
                "br" | "cr" => " ".into(),
                "noBreakHyphen" => "\u{2011}".into(),
                "softHyphen" => "\u{ad}".into(),
                "sym" => wattr(&c, "char")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .and_then(char::from_u32)
                    .map_or(String::new(), |ch| ch.to_string()),
                "drawing" | "pict" | "AlternateContent" => {
                    // Newer Word wraps shapes in mc:AlternateContent: take its first choice.
                    let node = if is(&c, "AlternateContent") {
                        match c
                            .children()
                            .find(|x| is(x, "Choice"))
                            .or_else(|| c.children().find(|x| is(x, "Fallback")))
                        {
                            Some(n) => n,
                            None => continue,
                        }
                    } else {
                        c
                    };
                    // A text box: its paragraphs follow, boxed.
                    if let Some(txbx) = node.descendants().find(|d| is(d, "txbxContent")) {
                        let outer = std::mem::take(&mut self.fields);
                        let mut blocks = self.blocks(&txbx);
                        self.fields = outer;
                        let line =
                            xenify_wp_doc::Border { width: 0.75, color: xenify_wp_doc::Rgba::rgb(0x80, 0x80, 0x80) };
                        for b in &mut blocks {
                            if let Block::Paragraph(p) = b {
                                p.props.borders = Some(xenify_wp_doc::ParaBorders::all(line));
                            }
                        }
                        self.textboxes.extend(blocks);
                        continue;
                    }
                    if let Some(img) = self.drawing(&node) {
                        images.push(img);
                    }
                    continue;
                }
                "footnoteReference" | "endnoteReference" => {
                    let kind = if c.tag_name().name() == "footnoteReference" {
                        xenify_wp_doc::NoteKind::Foot
                    } else {
                        xenify_wp_doc::NoteKind::End
                    };
                    if let Some(id) = wattr(&c, "id").and_then(|w| self.read_note(kind, w)) {
                        push_field(p, xenify_wp_doc::Field::Note { id }, fmt.clone());
                    }
                    continue;
                }
                _ => continue,
            };
            if text.is_empty() {
                continue;
            }
            let text = if caps { text.to_uppercase() } else { text };
            p.text.push_str(&text);
            p.runs.push(Run { len: text.len(), fmt: fmt.clone() });
        }
    }

    fn drawing(&mut self, n: &Node) -> Option<Image> {
        // DrawingML `a:blip r:embed`, or VML `v:imagedata r:id`.
        let rid = match n.descendants().find(|d| is(d, "blip")) {
            Some(blip) => blip.attribute((R, "embed"))?,
            None => n.descendants().find(|d| d.tag_name().name() == "imagedata")?.attribute((R, "id"))?,
        };
        let target = self.rels.get(rid)?.clone();
        let path = match target.strip_prefix('/') {
            Some(abs) => abs.to_string(),
            None => format!("word/{target}"),
        };
        let bytes = self.zip.read(&path).ok()?;
        let name = target.rsplit('/').next().unwrap_or("image.png").to_string();
        let key = format!("media/{name}");
        self.doc.assets.insert(key.clone(), Arc::new(bytes));
        let extent = n.descendants().find(|d| is(d, "extent"));
        let emu =
            |a: &str| extent.and_then(|e| e.attribute(a)).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 12700.0);
        let alt = n.descendants().find(|d| is(d, "docPr")).and_then(|d| d.attribute("descr")).unwrap_or("").to_string();
        let id = self.id();
        // A floating picture stays with its paragraph, on its own line, on the side it floated to.
        if let Some(h) = n.descendants().find(|d| d.tag_name().name() == "positionH") {
            let align = h.descendants().find(|d| d.tag_name().name() == "align").and_then(|a| a.text());
            let offset = h
                .descendants()
                .find(|d| d.tag_name().name() == "posOffset")
                .and_then(|o| o.text()?.parse::<f32>().ok());
            let side = match (align, offset) {
                (Some("center"), _) => Align::Center,
                (Some("right" | "outside"), _) => Align::Right,
                (_, Some(emu)) if emu / 12700.0 > 220.0 => Align::Right,
                _ => Align::Left,
            };
            self.float_aligns.insert(id, side);
        }
        // VML size: style="width:120pt;height:80pt".
        let vml_size = |k: &str| {
            let style = n.descendants().find(|d| d.tag_name().name() == "shape")?.attribute("style")?;
            let v = style.split(';').find_map(|kv| kv.trim().strip_prefix(k)?.trim().strip_prefix(':'))?;
            let v = v.trim();
            let (num, unit) = v.split_at(v.find(|c: char| c.is_alphabetic()).unwrap_or(v.len()));
            let n: f32 = num.trim().parse().ok()?;
            Some(match unit {
                "in" => n * 72.0,
                "cm" => n * 72.0 / 2.54,
                "mm" => n * 72.0 / 25.4,
                "px" => n * 0.75,
                _ => n,
            })
        };
        Some(Image {
            id,
            src: key,
            alt,
            width: emu("cx").or_else(|| vml_size("width")).unwrap_or(0.0),
            height: emu("cy").or_else(|| vml_size("height")).unwrap_or(0.0),
            align: Align::Left,
            // A line round the picture: <a:ln w="…"><a:solidFill><a:srgbClr val="…"/>.
            border: n
                .descendants()
                .find(|d| is(d, "ln"))
                .filter(|ln| !ln.descendants().any(|d| is(&d, "noFill")))
                .and_then(|ln| {
                    let width = ln.attribute("w").and_then(|v| v.parse::<f32>().ok()).map_or(0.75, |v| v / 12700.0);
                    let color = ln.descendants().find(|d| is(d, "srgbClr")).and_then(|c| color(c.attribute("val")))?;
                    Some(xenify_wp_doc::Border { width, color })
                }),
        })
    }

    fn table(&mut self, n: &Node) -> Table {
        // No lines unless the table or its style has them.
        let mut t = Table { id: self.id(), border: None, ..Default::default() };
        let mut banding = false;
        let mut first_row = false;
        let mut last_row = false;
        let grid: Vec<f32> = child(n, "tblGrid")
            .map(|g| g.children().filter(|c| is(c, "gridCol")).map(|c| twips(wattr(&c, "w")).unwrap_or(72.0)).collect())
            .unwrap_or_default();
        if let Some(pr) = child(n, "tblPr") {
            if let Some(jc) = child(&pr, "jc") {
                t.align = match val(&jc) {
                    Some("center") => Align::Center,
                    Some("right" | "end") => Align::Right,
                    _ => Align::Left,
                };
            }
            // Lines: the table's own, else its table style's, else none (Word's "Normal Table").
            let style = child(&pr, "tblStyle").and_then(|s| val(&s)).map(String::from);
            t.border = match child(&pr, "tblBorders") {
                Some(b) => table_border(&b),
                None => style.and_then(|s| self.table_styles.get(&s).copied()).flatten(),
            };
            if let Some(look) = child(&pr, "tblLook") {
                t.header_col = matches!(wattr(&look, "firstColumn"), Some("1" | "true" | "on"));
                first_row = matches!(wattr(&look, "firstRow"), Some("1" | "true" | "on"));
                last_row = matches!(wattr(&look, "lastRow"), Some("1" | "true" | "on"));
                banding = matches!(wattr(&look, "noHBand"), Some("0" | "false" | "off"));
            }
        }
        let cols = grid.len().max(1);
        t.widths = if grid.is_empty() { vec![1.0] } else { grid.clone() };
        // vMerge continuation: the anchor cell above grows.
        let mut rows: Vec<Row> = Vec::new();
        for tr in n.children().filter(|c| is(c, "tr")) {
            // A repeating header row; or the first row, when the table's look says so.
            let repeats = child(&tr, "trPr").and_then(|p| child(&p, "tblHeader")).is_some_and(|h| on(&h));
            t.repeat_header |= repeats;
            let header = repeats || first_row && rows.is_empty();
            let min_height = child(&tr, "trPr")
                .and_then(|p| child(&p, "trHeight"))
                .filter(|h| wattr(h, "hRule") != Some("exact"))
                .and_then(|h| twips(val(&h)));
            let mut row = Row { id: self.id(), header, footer: false, repeat: None, min_height, cells: Vec::new() };
            let mut col = 0;
            for tc in tr.children().filter(|c| is(c, "tc")) {
                let pr = child(&tc, "tcPr");
                let span = pr
                    .and_then(|p| child(&p, "gridSpan"))
                    .and_then(|g| val(&g))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let vmerge = pr.and_then(|p| child(&p, "vMerge"));
                let cont = vmerge.is_some_and(|v| val(&v) != Some("restart"));
                let shading = pr.and_then(|p| child(&p, "shd")).and_then(|s| color(wattr(&s, "fill")));
                let id = self.id();
                let mut cell = Cell::new(id, NodeId(0));
                cell.blocks = self.blocks(&tc);
                cell.col_span = span as u16;
                cell.shading = shading;
                if let Some(v) = pr.and_then(|p| child(&p, "vAlign")) {
                    cell.v_align = match val(&v) {
                        Some("center") => xenify_wp_doc::VAlign::Middle,
                        Some("bottom") => xenify_wp_doc::VAlign::Bottom,
                        _ => xenify_wp_doc::VAlign::Top,
                    };
                }
                if let Some(b) = pr.and_then(|p| child(&p, "tcBorders")) {
                    let side = |name: &str| {
                        b.children().find(|e| is(e, name)).filter(|e| !matches!(val(e), Some("nil" | "none"))).map(
                            |e| Border {
                                width: wattr(&e, "sz").and_then(|v| v.parse::<f32>().ok()).map_or(0.75, |v| v / 8.0),
                                color: color(wattr(&e, "color")).unwrap_or(Rgba::rgb(0, 0, 0)),
                            },
                        )
                    };
                    let sides = xenify_wp_doc::ParaBorders {
                        top: side("top"),
                        bottom: side("bottom"),
                        left: side("left").or_else(|| side("start")),
                        right: side("right").or_else(|| side("end")),
                    };
                    if !sides.is_empty() {
                        cell.borders = Some(sides);
                    }
                }
                if cont {
                    // Continue the merge from the row above.
                    let r = rows.len();
                    if let Some(above) = (0..r).rev().find_map(|ar| {
                        let c = &rows[ar].cells.get(col)?;
                        (!c.covered).then_some(ar)
                    }) {
                        rows[above].cells[col].row_span += 1;
                    }
                    cell.covered = true;
                    cell.blocks.clear();
                }
                row.cells.push(cell);
                for _ in 1..span {
                    let id = self.id();
                    let mut filler = Cell::new(id, NodeId(0));
                    filler.covered = true;
                    filler.blocks.clear();
                    row.cells.push(filler);
                }
                col += span;
            }
            rows.push(row);
        }
        t.rows = rows;
        if last_row
            && t.rows.len() > 1
            && let Some(r) = t.rows.last_mut()
        {
            r.footer = true;
        }
        let _ = cols;
        table_look_from_fills(&mut t, banding);
        let mut next = self.doc.next_id;
        t.normalize(&mut || {
            let id = NodeId(next);
            next += 1;
            id
        });
        self.doc.next_id = next;
        t
    }

    /// Body-level blocks of `n` (body, table cell, header).
    fn blocks(&mut self, n: &Node) -> Vec<Block> {
        if self.depth >= MAX_DEPTH {
            self.warnings.insert("some deeply nested text was left out".into());
            return Vec::new();
        }
        self.depth += 1;
        let out = self.blocks_in(n);
        self.depth -= 1;
        out
    }

    fn blocks_in(&mut self, n: &Node) -> Vec<Block> {
        let mut out = Vec::new();
        for c in n.children().filter(Node::is_element) {
            match c.tag_name().name() {
                "p" => out.extend(self.paragraph(&c)),
                "tbl" if self.tables >= MAX_TABLES => {
                    for tc in c.children().filter(|r| is(r, "tr")).flat_map(|r| r.children().filter(|c| is(c, "tc"))) {
                        out.extend(self.blocks(&tc));
                    }
                }
                "tbl" => {
                    self.tables += 1;
                    let t = self.table(&c);
                    self.tables -= 1;
                    out.push(Block::Table(t));
                }
                "sdt" => {
                    if let Some(content) = child(&c, "sdtContent") {
                        out.extend(self.block_control(&c, &content));
                    }
                }
                "customXml" => out.extend(self.blocks(&c)),
                _ => {}
            }
        }
        out
    }

    /// A header or footer part read as blocks, when it is more than one
    /// plain line (pictures, tables, formatting, several paragraphs).
    fn hf_rich(&mut self, rid: &str) -> Option<Vec<Block>> {
        let target = self.rels.get(rid).cloned()?;
        let xml = self.zip.read_string(&format!("word/{target}")).ok()?;
        let x = Xml::parse(&xml).ok()?;
        let root = x.root_element();
        let paras = root.descendants().filter(|n| is(n, "p") && n.descendants().any(|t| is(&t, "t"))).count();
        let objects =
            root.descendants().any(|n| matches!(n.tag_name().name(), "drawing" | "pict" | "tbl" | "AlternateContent"));
        let formatted = root.descendants().filter(|n| is(n, "rPr")).any(|r| {
            r.children()
                .filter(Node::is_element)
                .any(|c| !matches!(c.tag_name().name(), "rStyle" | "noProof" | "lang" | "rFonts" | "sz" | "szCs"))
        });
        if paras <= 1 && !objects && !formatted {
            return None;
        }
        // The header's own rels (its pictures).
        let rels_path = match target.rsplit_once('/') {
            Some((dir, file)) => format!("word/{dir}/_rels/{file}.rels"),
            None => format!("word/_rels/{target}.rels"),
        };
        let mut own = HashMap::new();
        if let Ok(rx) = self.zip.read_string(&rels_path)
            && let Ok(x) = Xml::parse(&rx)
        {
            for r in x.descendants().filter(|n| n.tag_name().name() == "Relationship") {
                if let (Some(id), Some(t)) = (r.attribute("Id"), r.attribute("Target")) {
                    own.insert(id.to_string(), t.to_string());
                }
            }
        }
        let body_rels = std::mem::replace(&mut self.rels, own);
        let outer = std::mem::take(&mut self.fields);
        let blocks = self.blocks(&root);
        self.fields = outer;
        self.rels = body_rels;
        Some(blocks)
    }

    fn hf_line(&mut self, rid: &str) -> HfLine {
        let Some(target) = self.rels.get(rid).cloned() else { return HfLine::default() };
        let Ok(xml) = self.zip.read_string(&format!("word/{target}")) else { return HfLine::default() };
        let Ok(x) = Xml::parse(&xml) else { return HfLine::default() };
        // Text with PAGE / NUMPAGES fields turned back into {page} / {pages};
        // tabs separate left, centre and right.
        let mut parts = vec![String::new()];
        // Field results (the page number Word last showed) are skipped.
        let mut in_result = false;
        for n in x.descendants() {
            if !n.is_element() {
                continue;
            }
            let in_simple = n.ancestors().skip(1).any(|a| is(&a, "fldSimple"));
            match n.tag_name().name() {
                "t" if !in_result && !in_simple => {
                    parts.last_mut().expect("non-empty").push_str(n.text().unwrap_or(""))
                }
                "tab" if n.parent().is_some_and(|p| is(&p, "r")) => parts.push(String::new()),
                "fldSimple" => {
                    let f = field(wattr(&n, "instr").unwrap_or(""));
                    parts.last_mut().expect("non-empty").push_str(&f);
                }
                "instrText" => {
                    let f = field(n.text().unwrap_or(""));
                    parts.last_mut().expect("non-empty").push_str(&f);
                }
                "fldChar" => match wattr(&n, "fldCharType") {
                    Some("separate") => in_result = true,
                    Some("end") => in_result = false,
                    _ => {}
                },
                _ => {}
            }
        }
        // Complex fields: drop cached results between separate and end (approximate: keep fields).
        let clean = |s: &str| s.trim().to_string();
        match parts.len() {
            1 => HfLine { center: clean(&parts[0]), ..Default::default() },
            2 => HfLine { left: clean(&parts[0]), center: clean(&parts[1]), ..Default::default() },
            _ => HfLine { left: clean(&parts[0]), center: clean(&parts[1]), right: clean(&parts[2..].join(" ")) },
        }
    }

    fn sect(&mut self, n: &Node) -> (PageSetup, SectionStart) {
        let mut p = PageSetup::default();
        if let Some(sz) = child(n, "pgSz") {
            p.width = twips(wattr(&sz, "w")).unwrap_or(p.width);
            p.height = twips(wattr(&sz, "h")).unwrap_or(p.height);
        }
        if let Some(m) = child(n, "pgMar") {
            p.margins.top = twips(wattr(&m, "top")).unwrap_or(p.margins.top).abs();
            p.margins.bottom = twips(wattr(&m, "bottom")).unwrap_or(p.margins.bottom).abs();
            p.margins.left = twips(wattr(&m, "left")).unwrap_or(p.margins.left);
            p.margins.right = twips(wattr(&m, "right")).unwrap_or(p.margins.right);
        }
        // Twips round A4's 595.28 pt to 595.3: snap back to the named sizes.
        for (w, h) in [PageSetup::A4, PageSetup::LETTER, (612.0, 1008.0), (419.53, 595.28), (841.89, 1190.55)] {
            for (a, b) in [(w, h), (h, w)] {
                if (p.width - a).abs() < 0.5 && (p.height - b).abs() < 0.5 {
                    (p.width, p.height) = (a, b);
                }
            }
        }
        if let Some(c) = child(n, "cols") {
            p.columns = wattr(&c, "num").and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 12);
            p.column_gap = twips(wattr(&c, "space")).unwrap_or(36.0);
            p.column_line = matches!(wattr(&c, "sep"), Some("1" | "true" | "on"));
        }
        if let Some(pn) = child(n, "pgNumType") {
            p.start_number = wattr(&pn, "start").and_then(|v| v.parse().ok());
            p.number_format = match wattr(&pn, "fmt") {
                Some("lowerRoman") => xenify_wp_doc::PageNumberFormat::LowerRoman,
                Some("upperRoman") => xenify_wp_doc::PageNumberFormat::UpperRoman,
                Some("lowerLetter") => xenify_wp_doc::PageNumberFormat::LowerLetter,
                Some("upperLetter") => xenify_wp_doc::PageNumberFormat::UpperLetter,
                _ => xenify_wp_doc::PageNumberFormat::Decimal,
            };
        }
        p.different_first = child(n, "titlePg").is_some_and(|t| on(&t));
        p.v_align = match child(n, "vAlign").as_ref().and_then(val) {
            Some("center") => xenify_wp_doc::PageVAlign::Center,
            Some("bottom") => xenify_wp_doc::PageVAlign::Bottom,
            _ => xenify_wp_doc::PageVAlign::Top,
        };
        let mut refs = 0;
        for r in n.children().filter(|c| is(c, "headerReference") || is(c, "footerReference")) {
            let Some(rid) = r.attribute((R, "id")) else { continue };
            refs += 1;
            // Pictures, tables, formatting or several paragraphs: a rich header.
            if let Some(blocks) = self.hf_rich(rid) {
                let slot = match wattr(&r, "type") {
                    Some("first") => xenify_wp_doc::HfSlot::First,
                    Some("even") => xenify_wp_doc::HfSlot::Even,
                    _ => xenify_wp_doc::HfSlot::Default,
                };
                let id = self.id();
                self.pending_headers.push(xenify_wp_doc::HeaderPart {
                    id,
                    footer: is(&r, "footerReference"),
                    slot,
                    blocks,
                });
                continue;
            }
            let line = self.hf_line(rid);
            match (is(&r, "headerReference"), wattr(&r, "type").unwrap_or("default")) {
                (true, "first") => p.first_header = line,
                (false, "first") => p.first_footer = line,
                (true, "even") => p.even_header = line,
                (false, "even") => p.even_footer = line,
                (true, _) => p.header = line,
                (false, _) => p.footer = line,
            }
        }
        // Word's odd/even setting is for the whole document.
        p.different_odd_even = self.even_and_odd;
        // A later section with no headers of its own uses the previous section's.
        p.link_to_previous = refs == 0 && !self.doc.sections.is_empty();
        let start = match child(n, "type").and_then(|t| val(&t)) {
            Some("continuous") => SectionStart::Continuous,
            _ => SectionStart::NewPage,
        };
        (p, start)
    }
}

fn field(instr: &str) -> String {
    let word = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    match word.as_str() {
        "PAGE" => "{page}".into(),
        "NUMPAGES" => "{pages}".into(),
        "SECTIONPAGES" => "{section-pages}".into(),
        "DATE" | "TIME" => "{date}".into(),
        "TITLE" => "{title}".into(),
        "AUTHOR" => "{author}".into(),
        _ => String::new(),
    }
}

fn read_styles(zip: &ZipReader, doc: &mut Document, warnings: &mut BTreeSet<String>) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for s in &doc.styles.paragraph {
        ids.insert(s.id.clone());
    }
    let Ok(xml) = zip.read_string("word/styles.xml") else { return ids };
    let Ok(x) = Xml::parse(&xml) else {
        warnings.insert("the styles couldn't be read".into());
        return ids;
    };
    let root = x.root_element();
    if let Some(def) = child(&root, "docDefaults").and_then(|d| child(&d, "rPrDefault")).and_then(|d| child(&d, "rPr"))
    {
        let f = read_rpr(Some(def));
        if let Some(font) = f.font {
            doc.styles.default_font = font;
        }
        if let Some(sz) = f.size {
            doc.styles.default_size = sz;
        }
    }
    for s in root.children().filter(|c| is(c, "style")) {
        if wattr(&s, "type") != Some("paragraph") {
            continue;
        }
        let Some(raw_id) = wattr(&s, "styleId") else { continue };
        let name = child(&s, "name").and_then(|n| val(&n)).unwrap_or(raw_id).to_string();
        // Map Word's built-in styles onto ours.
        let lower = name.to_ascii_lowercase();
        let id = if lower == "normal" {
            StyleSheet::NORMAL.to_string()
        } else if let Some(n) = lower.strip_prefix("heading ") {
            format!("heading{n}")
        } else if lower == "title"
            || lower == "subtitle"
            || lower == "quote"
            || lower == "caption"
            || lower == "header"
            || lower == "footer"
        {
            lower.clone()
        } else if let Some(n) = lower.strip_prefix("toc ") {
            format!("toc{n}")
        } else {
            raw_id.to_ascii_lowercase()
        };
        let based = child(&s, "basedOn").and_then(|b| val(&b)).map(|b| b.to_ascii_lowercase());
        let def = ParaStyle {
            id: id.clone(),
            name: if lower.starts_with("heading ") {
                format!("Heading {}", &name[8..])
            } else if lower.starts_with("toc ") {
                format!("Contents {}", &name[4..])
            } else if matches!(lower.as_str(), "header" | "footer" | "title" | "subtitle" | "quote" | "caption") {
                let mut c = lower.chars();
                c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
            } else {
                name
            },
            based_on: if id == StyleSheet::NORMAL {
                None
            } else {
                Some(based.unwrap_or_else(|| StyleSheet::NORMAL.into()))
            },
            next: child(&s, "next").and_then(|b| val(&b)).map(|b| b.to_ascii_lowercase()),
            list: None,
            outline_level: child(&s, "pPr")
                .and_then(|p| child(&p, "outlineLvl"))
                .and_then(|o| val(&o))
                .and_then(|v| v.parse::<u8>().ok())
                .map(|l| l + 1)
                .or_else(|| id.strip_prefix("heading").and_then(|n| n.parse().ok())),
            para: read_ppr(child(&s, "pPr")),
            char: read_rpr(child(&s, "rPr")),
        };
        ids.insert(id.clone());
        match doc.styles.get_mut(&id) {
            Some(slot) => *slot = def,
            None => doc.styles.paragraph.push(def),
        }
    }
    // Word style ids (e.g. "Heading1") → ours ("heading1"): styles refer to each other by those.
    ids
}

fn read_numbering(zip: &ZipReader, doc: &mut Document) -> Numbering {
    let mut nums = HashMap::new();
    let Ok(xml) = zip.read_string("word/numbering.xml") else { return Numbering { nums } };
    let Ok(x) = Xml::parse(&xml) else { return Numbering { nums } };
    let root = x.root_element();
    let mut abstracts: HashMap<String, String> = HashMap::new();
    for a in root.children().filter(|c| is(c, "abstractNum")) {
        let Some(aid) = wattr(&a, "abstractNumId") else { continue };
        let mut levels = Vec::new();
        for l in a.children().filter(|c| is(c, "lvl")) {
            let kind = match child(&l, "numFmt").and_then(|f| val(&f)) {
                Some("bullet") => NumberKind::Bullet,
                Some("lowerLetter") => NumberKind::LowerAlpha,
                Some("upperLetter") => NumberKind::UpperAlpha,
                Some("lowerRoman") => NumberKind::LowerRoman,
                Some("upperRoman") => NumberKind::UpperRoman,
                Some("none") => NumberKind::None,
                _ => NumberKind::Decimal,
            };
            let mut format = child(&l, "lvlText").and_then(|t| val(&t)).unwrap_or("%1.").to_string();
            if kind == NumberKind::Bullet
                && (format.is_empty() || format.chars().all(|c| ('\u{f000}'..='\u{f0ff}').contains(&c)))
            {
                // Symbol-font bullets: use a real bullet.
                format = "•".into();
            }
            let ind = child(&l, "pPr").and_then(|p| child(&p, "ind"));
            levels.push(ListLevel {
                kind,
                format,
                start: child(&l, "start").and_then(|s| val(&s)).and_then(|v| v.parse().ok()).unwrap_or(1),
                indent: ind
                    .and_then(|i| twips(wattr(&i, "left").or_else(|| wattr(&i, "start"))))
                    .unwrap_or(22.0 * (levels.len() as f32 + 1.0)),
                hanging: ind.and_then(|i| twips(wattr(&i, "hanging"))).unwrap_or(18.0),
            });
        }
        let bullet = levels.first().is_some_and(|l| l.kind == NumberKind::Bullet);
        // Reuse a list definition that looks the same (the built-ins, or one
        // imported already), so lists stay simple.
        let same = |d: &ListDef| {
            (0..3u8).all(|i| {
                let (a, b) = (d.level(i), levels.get(i as usize).cloned().unwrap_or_default());
                a.kind == b.kind
                    && (a.kind != NumberKind::Bullet || a.format == b.format)
                    && (a.kind == NumberKind::Bullet || a.format == b.format)
            })
        };
        let id = match doc.lists.iter().find(|d| same(d)) {
            Some(d) => d.id.clone(),
            None => {
                let id = format!("docx{aid}");
                let name = if bullet { "Bullets (imported)" } else { "Numbers (imported)" };
                doc.lists.push(ListDef { id: id.clone(), name: format!("{name} {aid}"), levels });
                id
            }
        };
        abstracts.insert(aid.to_string(), id);
    }
    for n in root.children().filter(|c| is(c, "num")) {
        let Some(nid) = wattr(&n, "numId") else { continue };
        let Some(aid) = child(&n, "abstractNumId").and_then(|a| val(&a)) else { continue };
        let Some(list) = abstracts.get(aid) else { continue };
        let mut starts = HashMap::new();
        for o in n.children().filter(|c| is(c, "lvlOverride")) {
            let lvl = wattr(&o, "ilvl").and_then(|v| v.parse().ok()).unwrap_or(0);
            if let Some(s) = child(&o, "startOverride").and_then(|s| val(&s)).and_then(|v| v.parse().ok()) {
                starts.insert(lvl, s);
            }
        }
        nums.insert(nid.to_string(), (list.clone(), starts));
    }
    Numbering { nums }
}

pub fn read(bytes: &[u8]) -> Result<Loaded, IoError> {
    let zip = ZipReader::new(bytes)?;
    let mut warnings = BTreeSet::new();
    let mut doc = Document::new();
    doc.sections.clear();
    let theme = read_theme_fonts(&zip);
    THEME_FONTS.with(|f| *f.borrow_mut() = theme);
    let style_ids = read_styles(&zip, &mut doc, &mut warnings);
    let numbering = read_numbering(&zip, &mut doc);
    let rels_xml = zip.read_string("word/_rels/document.xml.rels").unwrap_or_default();
    let mut rels = HashMap::new();
    if let Ok(x) = Xml::parse(&rels_xml) {
        for r in x.descendants().filter(|n| n.tag_name().name() == "Relationship") {
            if let (Some(id), Some(t)) = (r.attribute("Id"), r.attribute("Target")) {
                rels.insert(id.to_string(), t.to_string());
            }
        }
    }
    if let Ok(core) = zip.read_string("docProps/core.xml")
        && let Ok(x) = Xml::parse(&core)
    {
        for n in x.descendants() {
            match n.tag_name().name() {
                "title" => doc.meta.title = n.text().unwrap_or("").to_string(),
                "creator" => doc.meta.author = n.text().unwrap_or("").to_string(),
                _ => {}
            }
        }
    }
    let xml = zip.read_string("word/document.xml")?;
    let x = Xml::parse(&xml).map_err(|e| IoError::Damaged(e.to_string()))?;
    let body = x.root_element().children().find(|c| is(c, "body")).ok_or_else(|| IoError::Damaged("no body".into()))?;
    let page_color = x.root_element().children().find(|c| is(c, "background")).and_then(|b| color(wattr(&b, "color")));
    let mut r = Reader {
        doc,
        warnings,
        rels,
        zip: &zip,
        numbering,
        seen_nums: BTreeSet::new(),
        uses_checklist: false,
        style_ids,
        frames: Vec::new(),
        pending: Vec::new(),
        frame_key: None,
        notes: Vec::new(),
        fields: Vec::new(),
        form_data: Vec::new(),
        control: None,
        bookmarks: HashMap::new(),
        refs: HashMap::new(),
        even_and_odd: zip.read_string("word/settings.xml").is_ok_and(|s| s.contains("evenAndOddHeaders")),
        change: None,
        open_comments: Vec::new(),
        comment_threads: HashMap::new(),
        char_styles: read_char_styles(&zip),
        table_styles: read_table_styles(&zip),
        pending_headers: Vec::new(),
        float_aligns: HashMap::new(),
        textboxes: Vec::new(),
        depth: 0,
        tables: 0,
    };
    let (threads, by_word) = {
        let doc = &mut r.doc;
        read_comments(&zip, &mut || doc.alloc_id())
    };
    r.doc.meta.comments = threads;
    r.comment_threads = by_word;
    r.doc.meta.track_changes = zip.read_string("word/settings.xml").is_ok_and(|s| s.contains("trackRevisions"));
    // Sections end at paragraphs carrying a sectPr; the last sectPr is the body's.
    let mut blocks: Vec<Block> = Vec::new();
    // Inside a TOC field: its cached entries are skipped (the contents table is live).
    let mut in_toc = false;
    for c in body.children().filter(Node::is_element) {
        if is(&c, "p") {
            let instr: String = c.descendants().filter(|d| is(d, "instrText")).filter_map(|d| d.text()).collect();
            let ends = c.descendants().any(|d| is(&d, "fldChar") && wattr(&d, "fldCharType") == Some("end"));
            if !in_toc && instr.trim_start().starts_with("BIBLIOGRAPHY") {
                let id = r.id();
                blocks.push(Block::Toc(xenify_wp_doc::Toc {
                    id,
                    kind: xenify_wp_doc::TocKind::Bibliography,
                    ..Default::default()
                }));
                in_toc = !ends;
                continue;
            }
            if !in_toc && instr.trim_start().starts_with("TOC") && (instr.contains("\\c") || instr.contains("Caption"))
            {
                let id = r.id();
                blocks.push(Block::Toc(xenify_wp_doc::Toc {
                    id,
                    kind: xenify_wp_doc::TocKind::Figures,
                    ..Default::default()
                }));
                in_toc = !ends;
                continue;
            }
            if !in_toc && instr.trim_start().starts_with("TOC") {
                let id = r.id();
                let (levels, page_numbers) = toc_switches(&instr);
                blocks.push(Block::Toc(xenify_wp_doc::Toc { id, levels, page_numbers, ..Default::default() }));
                in_toc = !ends;
                continue;
            }
            if in_toc {
                in_toc = !ends;
                continue;
            }
        }
        match c.tag_name().name() {
            "p" => {
                let sect = child(&c, "pPr").and_then(|p| child(&p, "sectPr"));
                let parsed = r.paragraph(&c);
                let fpr = child(&c, "pPr").and_then(|p| child(&p, "framePr").map(|f| (p, f)));
                if let Some((ppr, fpr)) = fpr.filter(|_| sect.is_none()) {
                    let key: String = fpr.attributes().map(|a| format!("{}={};", a.name(), a.value())).collect();
                    if r.frame_key.as_deref() != Some(key.as_str()) {
                        let id = r.id();
                        let (f, pend) = read_frame(&fpr, &ppr, id);
                        r.frames.push(f);
                        r.pending.push(pend);
                        r.frame_key = Some(key);
                    }
                    // The frame has the lines and shading, not its paragraphs.
                    let parsed: Vec<Block> = parsed
                        .into_iter()
                        .map(|mut b| {
                            if let Block::Paragraph(p) = &mut b {
                                p.props.borders = None;
                                p.props.shading = None;
                            }
                            b
                        })
                        .collect();
                    if let Some(f) = r.frames.last_mut() {
                        f.blocks.extend(parsed);
                    }
                    continue;
                }
                r.frame_key = None;
                // An empty paragraph only there to carry a section break isn't content.
                let carrier = sect.is_some()
                    && !blocks.is_empty()
                    && matches!(parsed.as_slice(), [Block::Paragraph(p)] if p.text.is_empty());
                if !carrier {
                    blocks.extend(parsed);
                }
                if let Some(sp) = sect {
                    let (page, start) = r.sect(&sp);
                    let id = r.id();
                    let frames = r.settle_frames(&page);
                    r.doc.sections.push(Section {
                        id,
                        start,
                        page,
                        blocks: std::mem::take(&mut blocks),
                        frames,
                        notes: Vec::new(),
                        headers: std::mem::take(&mut r.pending_headers),
                    });
                }
            }
            "tbl" => {
                r.frame_key = None;
                blocks.push(Block::Table(r.table(&c)));
            }
            "sdt" => {
                if let Some(content) = child(&c, "sdtContent") {
                    // Table of contents content controls: keep as a live contents table.
                    let is_toc = c
                        .descendants()
                        .any(|d| is(&d, "docPartGallery") && val(&d).is_some_and(|v| v.contains("Table of Contents")));
                    let is_bib = c
                        .descendants()
                        .any(|d| is(&d, "docPartGallery") && val(&d).is_some_and(|v| v.contains("Bibliograph")));
                    let is_log = c.descendants().any(|d| is(&d, "tag") && val(&d) == Some("xenify-changelog"));
                    if is_log {
                        let id = r.id();
                        blocks.push(Block::Toc(xenify_wp_doc::Toc {
                            id,
                            kind: xenify_wp_doc::TocKind::ChangeLog,
                            page_numbers: false,
                            ..Default::default()
                        }));
                    } else if is_bib {
                        let id = r.id();
                        blocks.push(Block::Toc(xenify_wp_doc::Toc {
                            id,
                            kind: xenify_wp_doc::TocKind::Bibliography,
                            ..Default::default()
                        }));
                    } else if is_toc {
                        let id = r.id();
                        // The TOC field inside says which levels (and whether page numbers).
                        let instr: String =
                            c.descendants().filter(|d| is(d, "instrText")).filter_map(|d| d.text()).collect();
                        let (levels, page_numbers) = toc_switches(&instr);
                        blocks.push(Block::Toc(xenify_wp_doc::Toc { id, levels, page_numbers, ..Default::default() }));
                    } else {
                        blocks.extend(r.block_control(&c, &content));
                    }
                }
            }
            "sectPr" => {
                let (page, start) = r.sect(&c);
                let id = r.id();
                let frames = r.settle_frames(&page);
                r.doc.sections.push(Section {
                    id,
                    start,
                    page,
                    blocks: std::mem::take(&mut blocks),
                    frames,
                    notes: Vec::new(),
                    headers: std::mem::take(&mut r.pending_headers),
                });
            }
            _ => {}
        }
    }
    if !blocks.is_empty() {
        let id = r.id();
        let frames = r.settle_frames(&PageSetup::default());
        r.doc.sections.push(Section { id, blocks, frames, ..Default::default() });
    }
    // In Word a section's start type is on its own sectPr; ours matches.
    r.doc.sections.retain(|s| !s.blocks.is_empty() || !s.frames.is_empty());
    if r.uses_checklist && r.doc.list_def(xenify_wp_doc::ListDef::CHECKLIST).is_none() {
        r.doc.lists.extend(xenify_wp_doc::ListDef::builtin_def(xenify_wp_doc::ListDef::CHECKLIST));
    }
    // Notes go with the first section; cross-references find their bookmarks.
    if let Some(s) = r.doc.sections.first_mut() {
        s.notes = std::mem::take(&mut r.notes);
    }
    let (refs, bookmarks) = (std::mem::take(&mut r.refs), std::mem::take(&mut r.bookmarks));
    r.doc.for_each_para_mut(&mut |p| {
        for run in &mut p.runs {
            if let Some(xenify_wp_doc::Field::Ref { target }) = &mut run.fmt.field
                && let Some(name) = refs.get(target)
                && let Some(&para) = bookmarks.get(name)
            {
                *target = para;
            }
            // Links to bookmarks go to the paragraph (#p12).
            if let Some(name) = run.fmt.link.as_deref().and_then(|l| l.strip_prefix('#'))
                && let Some(&para) = bookmarks.get(name)
            {
                run.fmt.link = Some(format!("#p{}", para.0));
            }
        }
    });
    r.doc.meta.sources = read_sources(&zip);
    // The change log (a custom document property Xenify Write adds).
    if let Ok(custom) = zip.read_string("docProps/custom.xml")
        && let Ok(x) = Xml::parse(&custom)
        && let Some(p) = x
            .descendants()
            .find(|n| n.tag_name().name() == "property" && n.attribute("name") == Some("XenifyChangeLog"))
        && let Some(text) = p.descendants().find(|n| n.tag_name().name() == "lpwstr").and_then(|n| n.text())
    {
        r.doc.meta.changelog = serde_json::from_str(text).unwrap_or_default();
    }
    if let Some(style) = read_source_style(&zip) {
        r.doc.meta.cite_style = style;
    }
    let mut doc = r.doc;
    doc.meta.page_color = page_color;
    doc.repair();
    let mut lost = r.warnings;
    lost.extend(not_kept(&body, &zip));
    let mut warnings: Vec<String> = lost.into_iter().map(|w| format!("Not imported: {w}")).collect();
    warnings.sort();
    Ok(Loaded { doc, warnings })
}

/// Adds field `f` (its placeholder character) to the end of `p`.
fn push_field(p: &mut Paragraph, f: xenify_wp_doc::Field, fmt: xenify_wp_doc::CharFmt) {
    let fmt = xenify_wp_doc::CharFmt { field: Some(f), ..fmt };
    p.text.push(xenify_wp_doc::Field::CHAR);
    p.runs.push(Run { len: xenify_wp_doc::Field::CHAR.len_utf8(), fmt });
}

/// The `b:Sources` part (Word's bibliography sources), if any.
fn sources_part(zip: &ZipReader) -> Option<String> {
    let names: Vec<String> =
        zip.names().filter(|n| n.starts_with("customXml/item") && !n.contains("Props")).map(String::from).collect();
    names
        .into_iter()
        .filter_map(|n| zip.read_string(&n).ok())
        .find(|x| x.contains("officeDocument/2006/bibliography") && x.contains("Sources"))
}

fn read_source_style(zip: &ZipReader) -> Option<xenify_wp_doc::cite::CiteStyle> {
    use xenify_wp_doc::cite::CiteStyle;
    let xml = sources_part(zip)?;
    let x = Xml::parse(&xml).ok()?;
    let sel = x.root_element().attributes().find(|a| a.name() == "SelectedStyle")?.value().to_ascii_lowercase();
    Some(if sel.contains("apa") {
        CiteStyle::Apa
    } else if sel.contains("harvard") {
        CiteStyle::Harvard
    } else if sel.contains("ieee") {
        CiteStyle::Ieee
    } else if sel.contains("chicago") {
        CiteStyle::Chicago
    } else {
        return None;
    })
}

/// Word bibliography sources as the document's sources.
fn read_sources(zip: &ZipReader) -> Vec<xenify_wp_doc::cite::Source> {
    use xenify_wp_doc::cite::{Name, Source, SourceKind};
    let Some(xml) = sources_part(zip) else { return Vec::new() };
    let Ok(x) = Xml::parse(&xml) else { return Vec::new() };
    let text = |n: &Node, tag: &str| {
        n.children().find(|c| c.tag_name().name() == tag).and_then(|c| c.text()).unwrap_or("").trim().to_string()
    };
    x.root_element()
        .children()
        .filter(|n| n.tag_name().name() == "Source")
        .map(|n| {
            let kind = match text(&n, "SourceType").as_str() {
                "Book" => SourceKind::Book,
                "JournalArticle" | "ArticleInAPeriodical" => SourceKind::Article,
                "InternetSite" | "DocumentFromInternetSite" => SourceKind::Web,
                "Report" => SourceKind::Report,
                "BookSection" => SourceKind::Chapter,
                _ => SourceKind::Other,
            };
            let container = ["JournalName", "BookTitle", "InternetSiteTitle", "PeriodicalTitle"]
                .iter()
                .map(|t| text(&n, t))
                .find(|t| !t.is_empty())
                .unwrap_or_default();
            // Authors: b:Author/b:Author/b:NameList/b:Person (or a corporate name).
            let mut authors = Vec::new();
            if let Some(a) = n.children().find(|c| c.tag_name().name() == "Author") {
                for person in a.descendants().filter(|d| d.tag_name().name() == "Person") {
                    let given = [text(&person, "First"), text(&person, "Middle")]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" ");
                    authors.push(Name { family: text(&person, "Last"), given });
                }
                if authors.is_empty()
                    && let Some(c) = a.descendants().find(|d| d.tag_name().name() == "Corporate")
                {
                    authors.push(Name { family: c.text().unwrap_or("").trim().into(), given: String::new() });
                }
            }
            Source {
                key: text(&n, "Tag"),
                kind,
                authors,
                title: text(&n, "Title"),
                container,
                publisher: text(&n, "Publisher"),
                year: text(&n, "Year"),
                volume: text(&n, "Volume"),
                issue: text(&n, "Issue"),
                pages: text(&n, "Pages"),
                url: text(&n, "URL"),
                doi: text(&n, "DOI"),
                accessed: String::new(),
            }
        })
        .filter(|s| !s.key.is_empty())
        .collect()
}

/// A TOC field's heading levels (`\o "1-3"`, 3 when not given) and whether
/// it shows page numbers (`\n` hides them).
fn toc_switches(instr: &str) -> (u8, bool) {
    let levels = instr
        .split("\\o")
        .nth(1)
        .and_then(|r| r.split('"').nth(1))
        .and_then(|r| r.split('-').nth(1))
        .and_then(|n| n.trim().parse().ok())
        .filter(|n: &u8| (1..=9).contains(n))
        .unwrap_or(3);
    (levels, !instr.contains("\\n"))
}

/// The settings of one of Word's older form fields (`w:ffData`).
#[derive(Clone, Default)]
struct FormData {
    name: String,
    checked: bool,
    items: Vec<String>,
    selected: usize,
}

fn form_data(d: &Node) -> FormData {
    let mut f =
        FormData { name: child(d, "name").and_then(|n| val(&n)).unwrap_or("").to_string(), ..Default::default() };
    if let Some(cb) = child(d, "checkBox") {
        // `w:checked` wins over the default.
        f.checked =
            child(&cb, "checked").map(|c| on(&c)).or_else(|| child(&cb, "default").map(|c| on(&c))).unwrap_or(false);
    }
    if let Some(dd) = child(d, "ddList") {
        f.items = dd.children().filter(|e| is(e, "listEntry")).filter_map(|e| val(&e).map(str::to_string)).collect();
        f.selected = child(&dd, "result").and_then(|r| val(&r)).and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    f
}

/// A Word 2010 checkbox content control's state (`w14:checkbox`), if it is one.
fn checkbox(pr: &Node) -> Option<bool> {
    let cb = pr.children().find(|c| is(c, "checkbox"))?;
    let checked = cb.children().find(|c| is(c, "checked"));
    Some(
        checked
            .and_then(|c| c.attributes().find(|a| a.name() == "val").map(|a| a.value() == "1" || a.value() == "true"))
            .unwrap_or(false),
    )
}

/// What the document has that Write doesn't keep (yet), for the "Not
/// imported" list, so nothing disappears without a word.
fn not_kept(body: &Node, zip: &ZipReader) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut count = |what: &str| {
        out.insert(what.to_string());
    };
    for d in body.descendants().filter(Node::is_element) {
        match d.tag_name().name() {
            "chart" => count("charts"),
            "relIds" => count("SmartArt diagrams"),
            "OLEObject" => count("embedded objects (like spreadsheets)"),
            "control" if d.tag_name().namespace() == Some(W) => count("ActiveX controls"),
            "contentPart" => count("handwriting (ink)"),
            // Drawn shapes with no picture or text (lines, arrows, boxes) and groups of them.
            "wgp" => count("drawn shapes and groups"),
            "wsp" if !d.descendants().any(|x| is(&x, "txbx")) => count("drawn shapes and groups"),
            _ => {}
        }
    }
    if zip.contains("word/vbaProject.bin") {
        count("macros");
    }
    if zip.names().any(|n| n.starts_with("word/fonts/")) {
        count("embedded fonts (the fonts installed here are used)");
    }
    out
}
