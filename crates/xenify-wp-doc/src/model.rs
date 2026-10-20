use crate::{Container, Frame, Image, ListDef, ListItem, ParaProps, Rgba, StyleSheet, Table, Toc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::ops::Range;

/// A stable id for a paragraph, table, cell, frame… Never reused in a document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Document {
    pub meta: Meta,
    pub styles: StyleSheet,
    /// List definitions (bullets, numbering schemes).
    pub lists: Vec<ListDef>,
    pub sections: Vec<Section>,
    /// Embedded files (images) by name, e.g. `media/photo.png`. Stored in
    /// the package, not in the JSON.
    #[serde(skip)]
    pub assets: std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
    /// The next unused [`NodeId`].
    pub next_id: u64,
    /// Bumped by every change (layout caches and the dirty flag key on it).
    #[serde(skip)]
    pub revision: u64,
    /// Who is editing (signs comments and tracked changes). Not saved.
    #[serde(skip)]
    pub user: String,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub title: String,
    pub author: String,
    /// Who the document is for (used by AI review), e.g. "board members".
    pub audience: String,
    /// The voice wanted, e.g. "friendly but professional".
    pub tone: String,
    /// Show as one continuous page (no page breaks on screen).
    pub pageless: bool,
    /// The colour of the pages (white when unset).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_color: Option<crate::Rgba>,
    /// Works the document cites (References › Manage sources).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<crate::cite::Source>,
    /// How citations and the bibliography look.
    #[serde(skip_serializing_if = "is_default_style")]
    pub cite_style: crate::cite::CiteStyle,
    /// Comment threads (anchored by [`CharFmt::comments`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<crate::review::Comment>,
    /// File › Record a change: who changed the document, when, and how.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changelog: Vec<crate::changelog::LogEntry>,
    /// Review › Track changes: edits are marked, not made, until accepted.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub track_changes: bool,
    /// Words the spelling check accepts in this document.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore_words: Vec<String>,
    /// Individual proofing issues dismissed ("ignore once").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore_issues: Vec<String>,
    /// Mail merge settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<crate::merge::MergeConfig>,
    /// The writing profile proofing uses for this document (by id; `None`:
    /// the user's default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proofing_profile: Option<String>,
    /// Form controls (text, dates, lists) from Word, by id ([`CharFmt::control`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<crate::controls::Control>,
    /// The template the document was made from: its styles and page setup
    /// come from there (see [`crate::linked`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<crate::linked::TemplateLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Section {
    pub id: NodeId,
    /// How the section starts: on a new page, or right after the previous one.
    pub start: SectionStart,
    pub page: PageSetup,
    pub blocks: Vec<Block>,
    /// Boxes placed at fixed spots on the section's pages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<Frame>,
    /// Footnotes and endnotes referred to from this section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Rich headers and footers (any blocks); they replace the simple
    /// [`HfLine`]s of the same kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderPart>,
}

/// Which pages a header or footer is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HfSlot {
    #[default]
    Default,
    /// The section's first page (with "different first page").
    First,
    /// Even pages (with "different odd and even pages").
    Even,
}

/// A rich header or footer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeaderPart {
    pub id: NodeId,
    /// A footer (else a header).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub footer: bool,
    pub slot: HfSlot,
    pub blocks: Vec<Block>,
}

impl Default for Section {
    fn default() -> Self {
        Self {
            id: NodeId(0),
            start: SectionStart::NewPage,
            page: PageSetup::default(),
            blocks: Vec::new(),
            frames: Vec::new(),
            notes: Vec::new(),
            headers: Vec::new(),
        }
    }
}

impl Section {
    /// The section body, then each frame's blocks, then each note's, with their containers.
    pub fn block_lists(&self, index: usize) -> impl Iterator<Item = (Container, &Vec<Block>)> {
        std::iter::once((Container::Section(index), &self.blocks))
            .chain(self.frames.iter().map(|f| (Container::Frame(f.id), &f.blocks)))
            .chain(self.notes.iter().map(|n| (Container::Note(n.id), &n.blocks)))
            .chain(self.headers.iter().map(|h| (Container::Header(h.id), &h.blocks)))
    }

    pub fn block_lists_mut(&mut self) -> impl Iterator<Item = &mut Vec<Block>> {
        std::iter::once(&mut self.blocks)
            .chain(self.frames.iter_mut().map(|f| &mut f.blocks))
            .chain(self.notes.iter_mut().map(|n| &mut n.blocks))
            .chain(self.headers.iter_mut().map(|h| &mut h.blocks))
    }
}

/// Footnote (at the bottom of the page) or endnote (at the end of the document).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    #[default]
    Foot,
    End,
}

/// A footnote's or endnote's text; the text refers to it with a
/// [`Field::Note`] character.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: NodeId,
    pub kind: NoteKind,
    pub blocks: Vec<Block>,
}

/// Something the text shows that Write works out: a note number, a page
/// number, a citation… It sits on one placeholder character
/// ([`Field::CHAR`]) whose run carries it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum Field {
    /// The number of footnote/endnote `id`.
    Note { id: NodeId },
    /// This page's number.
    Page,
    /// The number of pages.
    Pages,
    /// Today's date.
    Date,
    /// The date and/or time in `format` (see [`crate::datetime`]): the
    /// current one, or `fixed` (the text when it was last updated) when it
    /// doesn't update by itself.
    DateTime {
        format: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixed: Option<String>,
    },
    /// The document's title.
    Title,
    /// The document's author.
    Author,
    /// A citation of these sources (keys into the document's sources).
    Citation {
        keys: Vec<String>,
        /// Page or chapter cited ("p. 12").
        #[serde(default, skip_serializing_if = "String::is_empty")]
        locator: String,
    },
    /// A reference to a heading: its text.
    Ref { target: NodeId },
    /// An equation, written in LaTeX (`\frac{a}{b}`). `display` puts it on
    /// a line of its own.
    Equation {
        latex: String,
        #[serde(default)]
        display: bool,
        /// Word's own markup for it (OMML), when it came from a .docx and
        /// hasn't been edited: written back unchanged.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        omml: String,
    },
    /// A checkbox to tick (from a Word form).
    Check { checked: bool },
    /// A small coloured label in the text ("IN PROGRESS", "DONE"): `color`
    /// is one of [`BADGE_COLORS`]' names.
    Badge { text: String, color: String },
}

/// Badge colours: (name, background, text).
pub const BADGE_COLORS: &[(&str, &str, &str)] = &[
    ("grey", "#E7E5E4", "#44403C"),
    ("blue", "#DBEAFE", "#1E40AF"),
    ("green", "#DCFCE7", "#166534"),
    ("yellow", "#FEF3C7", "#92400E"),
    ("red", "#FEE2E2", "#991B1B"),
    ("purple", "#EDE9FE", "#5B21B6"),
];

/// A badge colour's (background, text), grey when unknown.
pub fn badge_colors(name: &str) -> (Rgba, Rgba) {
    let (_, bg, fg) = BADGE_COLORS.iter().find(|(n, ..)| *n == name).unwrap_or(&BADGE_COLORS[0]);
    (
        Rgba::from_hex(bg).unwrap_or(Rgba::rgb(0xe7, 0xe5, 0xe4)),
        Rgba::from_hex(fg).unwrap_or(Rgba::rgb(0x44, 0x40, 0x3c)),
    )
}

impl Field {
    /// The character a field sits on.
    pub const CHAR: char = '\u{FFFC}';
    pub const STR: &'static str = "\u{FFFC}";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionStart {
    #[default]
    NewPage,
    /// On the same page (e.g. to switch to two columns part-way down a page).
    Continuous,
}

/// A header or footer line: left, centred and right parts. Each may use
/// fields: `{page}`, `{pages}`, `{section-pages}`, `{date}`, `{title}`,
/// `{author}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HfLine {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub left: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub center: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub right: String,
}

impl HfLine {
    pub fn is_empty(&self) -> bool {
        self.left.trim().is_empty() && self.center.trim().is_empty() && self.right.trim().is_empty()
    }
}

/// Page size and margins, in points (1/72 inch).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub margins: Margins,
    /// Text columns (1 = none).
    pub columns: u8,
    /// Space between columns, in points.
    pub column_gap: f32,
    /// A thin line down the middle of each gap between columns.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub column_line: bool,
    pub header: HfLine,
    pub footer: HfLine,
    /// The first page of the section has its own header and footer.
    pub different_first: bool,
    pub first_header: HfLine,
    pub first_footer: HfLine,
    /// Page numbering starts again at this number in this section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_number: Option<u32>,
    /// Even-numbered pages have their own header and footer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub different_odd_even: bool,
    #[serde(skip_serializing_if = "HfLine::is_empty")]
    pub even_header: HfLine,
    #[serde(skip_serializing_if = "HfLine::is_empty")]
    pub even_footer: HfLine,
    /// How page numbers are written in this section.
    #[serde(skip_serializing_if = "PageNumberFormat::is_decimal")]
    pub number_format: PageNumberFormat,
    /// The section uses the previous section's headers and footers.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub link_to_previous: bool,
    /// Where the text sits on each page between the top and bottom margins
    /// (a title page: centred).
    #[serde(skip_serializing_if = "PageVAlign::is_top")]
    pub v_align: PageVAlign,
}

/// Where a section's text sits on its pages, top to bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageVAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

impl PageVAlign {
    pub fn is_top(&self) -> bool {
        *self == PageVAlign::Top
    }
}

/// How page numbers are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageNumberFormat {
    /// 1, 2, 3
    #[default]
    Decimal,
    /// i, ii, iii
    LowerRoman,
    /// I, II, III
    UpperRoman,
    /// a, b, c
    LowerLetter,
    /// A, B, C
    UpperLetter,
}

impl PageNumberFormat {
    pub const ALL: [PageNumberFormat; 5] = [
        PageNumberFormat::Decimal,
        PageNumberFormat::LowerRoman,
        PageNumberFormat::UpperRoman,
        PageNumberFormat::LowerLetter,
        PageNumberFormat::UpperLetter,
    ];

    pub fn is_decimal(&self) -> bool {
        *self == PageNumberFormat::Decimal
    }

    /// Page `n` in this format.
    pub fn format(self, n: u32) -> String {
        use crate::NumberKind;
        let kind = match self {
            PageNumberFormat::Decimal => NumberKind::Decimal,
            PageNumberFormat::LowerRoman => NumberKind::LowerRoman,
            PageNumberFormat::UpperRoman => NumberKind::UpperRoman,
            PageNumberFormat::LowerLetter => NumberKind::LowerAlpha,
            PageNumberFormat::UpperLetter => NumberKind::UpperAlpha,
        };
        crate::lists::format_number(n, kind)
    }

    pub fn name(self) -> &'static str {
        match self {
            PageNumberFormat::Decimal => "1, 2, 3",
            PageNumberFormat::LowerRoman => "i, ii, iii",
            PageNumberFormat::UpperRoman => "I, II, III",
            PageNumberFormat::LowerLetter => "a, b, c",
            PageNumberFormat::UpperLetter => "A, B, C",
        }
    }
}

impl PageSetup {
    pub const A4: (f32, f32) = (595.28, 841.89);
    pub const LETTER: (f32, f32) = (612.0, 792.0);

    /// Width available for text between the side margins.
    pub fn content_width(&self) -> f32 {
        (self.width - self.margins.left - self.margins.right).max(36.0)
    }

    /// Width of one text column.
    pub fn column_width(&self) -> f32 {
        let n = self.columns.max(1) as f32;
        ((self.content_width() - self.column_gap * (n - 1.0)) / n).max(36.0)
    }

    /// Height available for text between the top and bottom margins.
    pub fn content_height(&self) -> f32 {
        (self.height - self.margins.top - self.margins.bottom).max(36.0)
    }
}

impl Default for PageSetup {
    fn default() -> Self {
        let (width, height) = Self::A4;
        Self {
            width,
            height,
            margins: Margins::default(),
            columns: 1,
            column_gap: 36.0,
            column_line: false,
            header: HfLine::default(),
            footer: HfLine::default(),
            different_first: false,
            first_header: HfLine::default(),
            first_footer: HfLine::default(),
            start_number: None,
            different_odd_even: false,
            even_header: HfLine::default(),
            even_footer: HfLine::default(),
            number_format: PageNumberFormat::Decimal,
            link_to_previous: false,
            v_align: PageVAlign::Top,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Margins {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl Default for Margins {
    fn default() -> Self {
        // 2.54 cm / 1 inch all round.
        Self { top: 72.0, bottom: 72.0, left: 72.0, right: 72.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// Paragraphs are most blocks: boxing them would cost more than the size gap.
#[allow(clippy::large_enum_variant)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    Image(Image),
    Toc(Toc),
}

impl Block {
    pub fn id(&self) -> NodeId {
        match self {
            Block::Paragraph(p) => p.id,
            Block::Table(t) => t.id,
            Block::Image(i) => i.id,
            Block::Toc(t) => t.id,
        }
    }
}

/// Paragraphs of `blocks` (and of table cells in them), in reading order.
fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Paragraph>) {
    for b in blocks {
        match b {
            Block::Paragraph(p) => out.push(p),
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in row.cells.iter().filter(|c| !c.covered) {
                        walk(&cell.blocks, out);
                    }
                }
            }
            Block::Image(_) | Block::Toc(_) => {}
        }
    }
}

fn para_mut_in(blocks: &mut [Block], id: NodeId) -> Option<&mut Paragraph> {
    for b in blocks {
        match b {
            Block::Paragraph(p) if p.id == id => return Some(p),
            Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        if let Some(p) = para_mut_in(&mut cell.blocks, id) {
                            return Some(p);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Finds block `id` under `blocks` (which live in `here`).
fn locate_in(blocks: &[Block], here: &Container, id: NodeId) -> Option<(Container, usize)> {
    for (i, b) in blocks.iter().enumerate() {
        if b.id() == id {
            return Some((here.clone(), i));
        }
        if let Block::Table(t) = b {
            for (r, row) in t.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    let inner = Container::Cell { table: t.id, row: r, col: c };
                    if let Some(found) = locate_in(&cell.blocks, &inner, id) {
                        return Some(found);
                    }
                }
            }
        }
    }
    None
}

fn table_in(blocks: &[Block], id: NodeId) -> Option<&Table> {
    for b in blocks {
        if let Block::Table(t) = b {
            if t.id == id {
                return Some(t);
            }
            for row in &t.rows {
                for cell in &row.cells {
                    if let Some(found) = table_in(&cell.blocks, id) {
                        return Some(found);
                    }
                }
            }
        }
    }
    None
}

fn table_mut_in(blocks: &mut [Block], id: NodeId) -> Option<&mut Table> {
    for b in blocks {
        if let Block::Table(t) = b {
            if t.id == id {
                return Some(t);
            }
            for row in &mut t.rows {
                for cell in &mut row.cells {
                    if let Some(found) = table_mut_in(&mut cell.blocks, id) {
                        return Some(found);
                    }
                }
            }
        }
    }
    None
}

fn max_id(blocks: &[Block], max: &mut u64) {
    for b in blocks {
        *max = (*max).max(b.id().0);
        if let Block::Table(t) = b {
            for row in &t.rows {
                *max = (*max).max(row.id.0);
                for cell in &row.cells {
                    *max = (*max).max(cell.id.0);
                    max_id(&cell.blocks, max);
                }
            }
        }
    }
}

/// A paragraph: its text, the paragraph style it uses and character
/// formatting runs covering the text exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Paragraph {
    pub id: NodeId,
    /// Paragraph style id (see [`StyleSheet`]).
    pub style: String,
    pub text: String,
    /// Direct formatting. Lengths add up to `text.len()`; neighbours differ.
    pub runs: Vec<Run>,
    /// Direct paragraph formatting on top of the style.
    #[serde(skip_serializing_if = "ParaProps::is_empty")]
    pub props: ParaProps,
    /// The list this paragraph is an item of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<ListItem>,
    /// Content revision, for layout caches (not saved).
    #[serde(skip)]
    pub rev: u64,
}

impl Default for Paragraph {
    fn default() -> Self {
        Self {
            id: NodeId(0),
            style: StyleSheet::NORMAL.into(),
            text: String::new(),
            runs: Vec::new(),
            props: ParaProps::default(),
            list: None,
            rev: 0,
        }
    }
}

/// A stretch of text with the same direct formatting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub len: usize,
    #[serde(default, skip_serializing_if = "CharFmt::is_empty")]
    pub fmt: CharFmt,
}

/// Character formatting on top of the paragraph style. `None` means "as
/// the style says".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharFmt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgba>,
    /// Highlight (background) colour; fully transparent means "none".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Rgba>,
    /// Hyperlink target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The field this character stands for (on a [`Field::CHAR`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<Field>,
    /// Raised (superscript) or lowered (subscript) and smaller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert: Option<VertAlign>,
    /// Comments this text is the anchor of (ids in [`Meta::comments`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<NodeId>,
    /// A tracked change: the text was inserted or deleted (and not yet accepted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<crate::review::Change>,
    /// The form control this text is the value of (an id in [`Meta::controls`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<NodeId>,
}

impl CharFmt {
    pub fn is_empty(&self) -> bool {
        *self == CharFmt::default()
    }

    /// `self` with every property `patch` sets replaced.
    pub fn with(&self, patch: &CharFmt) -> CharFmt {
        CharFmt {
            bold: patch.bold.or(self.bold),
            italic: patch.italic.or(self.italic),
            underline: patch.underline.or(self.underline),
            strike: patch.strike.or(self.strike),
            font: patch.font.clone().or_else(|| self.font.clone()),
            size: patch.size.or(self.size),
            color: patch.color.or(self.color),
            highlight: patch.highlight.or(self.highlight),
            link: patch.link.clone().or_else(|| self.link.clone()),
            field: patch.field.clone().or_else(|| self.field.clone()),
            vert: patch.vert.or(self.vert),
            comments: if patch.comments.is_empty() { self.comments.clone() } else { patch.comments.clone() },
            change: patch.change.clone().or_else(|| self.change.clone()),
            control: patch.control.or(self.control),
        }
    }
}

/// Text with its formatting, cut out of (or to be put into) a paragraph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub runs: Vec<Run>,
}

impl Span {
    pub fn plain(text: impl Into<String>, fmt: CharFmt) -> Self {
        let text = text.into();
        let runs = if text.is_empty() { Vec::new() } else { vec![Run { len: text.len(), fmt }] };
        Self { text, runs }
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Appends `text` in format `fmt` (joining the last run when it matches).
    pub fn push(&mut self, text: &str, fmt: CharFmt) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        match self.runs.last_mut() {
            Some(r) if r.fmt == fmt => r.len += text.len(),
            _ => self.runs.push(Run { len: text.len(), fmt }),
        }
    }
}

impl Paragraph {
    pub fn new(id: NodeId, style: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let runs = if text.is_empty() { Vec::new() } else { vec![Run { len: text.len(), fmt: CharFmt::default() }] };
        Self { id, style: style.into(), text, runs, ..Default::default() }
    }

    /// The direct formatting that text typed at `offset` gets: that of the
    /// character before it (or after it, at the start).
    pub fn fmt_at(&self, offset: usize) -> CharFmt {
        let mut start = 0;
        for r in &self.runs {
            let end = start + r.len;
            if offset <= end && offset > start || (offset == 0 && start == 0) {
                return r.fmt.clone();
            }
            start = end;
        }
        self.runs.last().map(|r| r.fmt.clone()).unwrap_or_default()
    }

    /// The runs as `(byte range, format)`.
    pub fn run_ranges(&self) -> impl Iterator<Item = (Range<usize>, &CharFmt)> {
        let mut start = 0;
        self.runs.iter().map(move |r| {
            let range = start..start + r.len;
            start += r.len;
            (range, &r.fmt)
        })
    }

    /// Copies `range` out with its formatting.
    pub fn slice(&self, range: Range<usize>) -> Span {
        let mut runs = Vec::new();
        for (r, fmt) in self.run_ranges() {
            let (s, e) = (r.start.max(range.start), r.end.min(range.end));
            if s < e {
                runs.push(Run { len: e - s, fmt: fmt.clone() });
            }
        }
        Span { text: self.text[range].to_string(), runs }
    }

    /// Replaces the first `len` bytes with `with` (importers and exporters:
    /// checklist boxes written as characters).
    pub fn replace_start(&mut self, len: usize, with: Span) {
        let len = len.min(self.text.len());
        self.splice(0..len, with);
    }

    /// Replaces `range` with `with`, returning what was there.
    pub(crate) fn splice(&mut self, range: Range<usize>, with: Span) -> Span {
        let removed = self.slice(range.clone());
        let head = self.slice(0..range.start);
        let tail = self.slice(range.end..self.text.len());
        self.text.replace_range(range, &with.text);
        self.runs = head.runs.into_iter().chain(with.runs).chain(tail.runs).collect();
        self.normalize();
        removed
    }

    /// Merges neighbouring runs with the same format and drops empty ones.
    pub fn normalize(&mut self) {
        let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
        for r in self.runs.drain(..) {
            if r.len == 0 {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.fmt == r.fmt => last.len += r.len,
                _ => out.push(r),
            }
        }
        let covered: usize = out.iter().map(|r| r.len).sum();
        if covered < self.text.len() {
            // Defensive: never leave text without a run.
            match out.last_mut() {
                Some(last) => last.len += self.text.len() - covered,
                None => out.push(Run { len: self.text.len(), fmt: CharFmt::default() }),
            }
        }
        self.runs = out;
    }

    /// The char boundary at or before `offset`.
    pub fn floor(&self, offset: usize) -> usize {
        let mut o = offset.min(self.text.len());
        while !self.text.is_char_boundary(o) {
            o -= 1;
        }
        o
    }
}

/// A caret position: a byte offset in a paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub para: NodeId,
    pub offset: usize,
}

impl Pos {
    pub fn new(para: NodeId, offset: usize) -> Self {
        Self { para, offset }
    }
}

/// A text selection from `anchor` (where it started) to `focus` (the caret).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub anchor: Pos,
    pub focus: Pos,
}

impl Selection {
    pub fn caret(pos: Pos) -> Self {
        Self { anchor: pos, focus: pos }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }
}

impl Document {
    /// An empty document: one section with one empty Normal paragraph.
    pub fn new() -> Self {
        let mut doc = Self {
            meta: Meta::default(),
            styles: StyleSheet::default(),
            lists: ListDef::builtin(),
            sections: Vec::new(),
            assets: Default::default(),
            next_id: 1,
            revision: 0,
            user: String::new(),
        };
        let section = doc.alloc_id();
        let para = doc.alloc_id();
        doc.sections.push(Section {
            id: section,
            start: SectionStart::NewPage,
            frames: Vec::new(),
            notes: Vec::new(),
            headers: Vec::new(),
            page: PageSetup::default(),
            blocks: vec![Block::Paragraph(Paragraph::new(para, StyleSheet::NORMAL, ""))],
        });
        doc
    }

    /// A document with one Normal paragraph per line of `text`.
    pub fn from_plain_text(text: &str) -> Self {
        let mut doc = Self::new();
        let blocks = text
            .split('\n')
            .map(|line| {
                let id = doc.alloc_id();
                Block::Paragraph(Paragraph::new(id, StyleSheet::NORMAL, line.trim_end_matches('\r')))
            })
            .collect();
        doc.sections[0].blocks = blocks;
        doc
    }

    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Makes sure the document is editable after loading: at least one
    /// section and paragraph, runs covering the text, ids below `next_id`.
    pub fn repair(&mut self) {
        if self.sections.is_empty() {
            let id = self.alloc_id();
            self.sections.push(Section { id, ..Default::default() });
        }
        let mut max = 0;
        for s in &self.sections {
            max = max.max(s.id.0);
            max_id(&s.blocks, &mut max);
            for f in &s.frames {
                max = max.max(f.id.0);
                max_id(&f.blocks, &mut max);
            }
        }
        self.next_id = self.next_id.max(max + 1);
        let mut next = self.next_id;
        let mut alloc = || {
            let id = NodeId(next);
            next += 1;
            id
        };
        fn fix(blocks: &mut [Block], alloc: &mut dyn FnMut() -> NodeId) {
            for b in blocks {
                match b {
                    Block::Paragraph(p) => {
                        p.normalize();
                        let covered: usize = p.runs.iter().map(|r| r.len).sum();
                        if covered != p.text.len() {
                            p.runs = vec![Run { len: p.text.len(), fmt: CharFmt::default() }];
                            p.normalize();
                        }
                    }
                    Block::Table(t) => {
                        let mut a = || alloc();
                        t.normalize(&mut a);
                        for row in &mut t.rows {
                            for cell in &mut row.cells {
                                fix(&mut cell.blocks, alloc);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for s in &mut self.sections {
            for blocks in s.block_lists_mut() {
                fix(blocks, &mut alloc);
            }
            // A frame needs a paragraph to type into.
            for f in &mut s.frames {
                if f.blocks.is_empty() {
                    f.blocks.push(Block::Paragraph(Paragraph::new(alloc(), StyleSheet::NORMAL, "")));
                }
                f.width = f.width.max(8.0);
                f.height = f.height.max(8.0);
            }
        }
        self.next_id = next;
        if self.paragraphs().next().is_none() {
            let id = self.alloc_id();
            self.sections[0].blocks.push(Block::Paragraph(Paragraph::new(id, StyleSheet::NORMAL, "")));
        }
    }

    /// Paragraph `p`'s resolved formatting: its style, then its direct
    /// properties, then its list indent.
    pub fn resolve_para(&self, p: &Paragraph) -> crate::ResolvedPara {
        let mut r = self.styles.resolve(&p.style);
        r.apply(&p.props);
        if let Some(item) = &p.list
            && let Some(def) = self.lists.iter().find(|d| d.id == item.list)
        {
            let lv = def.level(item.level);
            if p.props.indent_left.is_none() {
                r.indent_left += lv.indent;
            }
            r.hanging = lv.hanging;
        }
        r
    }

    pub fn list_def(&self, id: &str) -> Option<&ListDef> {
        self.lists.iter().find(|d| d.id == id)
    }

    /// Every paragraph (including those in table cells), in reading order.
    pub fn paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        let mut out = Vec::new();
        for (i, s) in self.sections.iter().enumerate() {
            for (_, blocks) in s.block_lists(i) {
                walk(blocks, &mut out);
            }
        }
        out.into_iter()
    }

    /// Where block `id` is: its container and index in it.
    pub fn locate(&self, id: NodeId) -> Option<(Container, usize)> {
        self.sections
            .iter()
            .enumerate()
            .find_map(|(si, s)| s.block_lists(si).find_map(|(c, blocks)| locate_in(blocks, &c, id)))
    }

    /// The section holding block `id` (directly, or inside a table).
    pub fn section_of(&self, id: NodeId) -> Option<usize> {
        let (mut c, _) = self.locate(id)?;
        loop {
            match c {
                Container::Section(s) => return Some(s),
                Container::Cell { table, .. } => c = self.locate(table)?.0,
                Container::Frame(f) => return self.sections.iter().position(|s| s.frames.iter().any(|x| x.id == f)),
                Container::Note(n) => return self.sections.iter().position(|s| s.notes.iter().any(|x| x.id == n)),
                Container::Header(h) => return self.sections.iter().position(|s| s.headers.iter().any(|x| x.id == h)),
            }
        }
    }

    pub fn blocks(&self, c: &Container) -> Option<&Vec<Block>> {
        match c {
            Container::Section(s) => self.sections.get(*s).map(|s| &s.blocks),
            Container::Cell { table, row, col } => Some(&self.table(*table)?.rows.get(*row)?.cells.get(*col)?.blocks),
            Container::Frame(f) => self.frame(*f).map(|f| &f.blocks),
            Container::Note(n) => self.note(*n).map(|n| &n.blocks),
            Container::Header(h) => {
                self.sections.iter().flat_map(|s| &s.headers).find(|x| x.id == *h).map(|x| &x.blocks)
            }
        }
    }

    pub fn note(&self, id: NodeId) -> Option<&Note> {
        self.sections.iter().flat_map(|s| &s.notes).find(|n| n.id == id)
    }

    pub fn note_mut(&mut self, id: NodeId) -> Option<&mut Note> {
        self.sections.iter_mut().flat_map(|s| &mut s.notes).find(|n| n.id == id)
    }

    pub fn blocks_mut(&mut self, c: &Container) -> Option<&mut Vec<Block>> {
        match c {
            Container::Section(s) => self.sections.get_mut(*s).map(|s| &mut s.blocks),
            Container::Cell { table, row, col } => {
                Some(&mut self.table_mut(*table)?.rows.get_mut(*row)?.cells.get_mut(*col)?.blocks)
            }
            Container::Frame(f) => self.frame_mut(*f).map(|f| &mut f.blocks),
            Container::Note(n) => self.note_mut(*n).map(|n| &mut n.blocks),
            Container::Header(h) => {
                self.sections.iter_mut().flat_map(|s| &mut s.headers).find(|x| x.id == *h).map(|x| &mut x.blocks)
            }
        }
    }

    pub fn frame(&self, id: NodeId) -> Option<&Frame> {
        self.sections.iter().flat_map(|s| &s.frames).find(|f| f.id == id)
    }

    pub fn frame_mut(&mut self, id: NodeId) -> Option<&mut Frame> {
        self.sections.iter_mut().flat_map(|s| &mut s.frames).find(|f| f.id == id)
    }

    /// Calls `f` on every paragraph (including those in tables).
    pub fn for_each_para_mut(&mut self, f: &mut dyn FnMut(&mut Paragraph)) {
        fn go(blocks: &mut [Block], f: &mut dyn FnMut(&mut Paragraph)) {
            for b in blocks {
                match b {
                    Block::Paragraph(p) => f(p),
                    Block::Table(t) => {
                        for row in &mut t.rows {
                            for cell in &mut row.cells {
                                go(&mut cell.blocks, f);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for s in &mut self.sections {
            for blocks in s.block_lists_mut() {
                go(blocks, f);
            }
        }
    }

    pub fn table(&self, id: NodeId) -> Option<&Table> {
        self.sections.iter().enumerate().find_map(|(i, s)| s.block_lists(i).find_map(|(_, b)| table_in(b, id)))
    }

    pub fn table_mut(&mut self, id: NodeId) -> Option<&mut Table> {
        self.sections.iter_mut().find_map(|s| s.block_lists_mut().find_map(|b| table_mut_in(b, id)))
    }

    /// A block by id (any kind, anywhere).
    pub fn block(&self, id: NodeId) -> Option<&Block> {
        let (c, i) = self.locate(id)?;
        self.blocks(&c)?.get(i)
    }

    pub fn para(&self, id: NodeId) -> Option<&Paragraph> {
        match self.block(id)? {
            Block::Paragraph(p) => Some(p),
            _ => None,
        }
    }

    pub fn para_mut(&mut self, id: NodeId) -> Option<&mut Paragraph> {
        self.sections.iter_mut().find_map(|s| s.block_lists_mut().find_map(|b| para_mut_in(b, id)))
    }

    /// Reading-order index of paragraph `id`.
    pub fn para_index(&self, id: NodeId) -> Option<usize> {
        self.paragraphs().position(|p| p.id == id)
    }

    pub fn next_para(&self, id: NodeId) -> Option<NodeId> {
        let mut it = self.paragraphs().skip_while(|p| p.id != id);
        it.next()?;
        it.next().map(|p| p.id)
    }

    pub fn prev_para(&self, id: NodeId) -> Option<NodeId> {
        let mut prev = None;
        for p in self.paragraphs() {
            if p.id == id {
                return prev;
            }
            prev = Some(p.id);
        }
        None
    }

    pub fn start(&self) -> Pos {
        Pos::new(self.paragraphs().next().expect("repaired documents have a paragraph").id, 0)
    }

    /// The end of the body text: the last section's own blocks, not its
    /// text boxes, notes or headers (which come after it in [`Document::paragraphs`]).
    pub fn body_end(&self) -> Pos {
        let mut out = Vec::new();
        for s in &self.sections {
            walk(&s.blocks, &mut out);
        }
        match out.last() {
            Some(p) => Pos::new(p.id, p.text.len()),
            None => self.end(),
        }
    }

    pub fn end(&self) -> Pos {
        let p = self.paragraphs().last().expect("repaired documents have a paragraph");
        Pos::new(p.id, p.text.len())
    }

    /// Document order of two positions.
    pub fn cmp_pos(&self, a: Pos, b: Pos) -> Ordering {
        if a.para == b.para {
            return a.offset.cmp(&b.offset);
        }
        self.para_index(a.para).cmp(&self.para_index(b.para))
    }

    /// The selection's ends in document order.
    pub fn ordered(&self, sel: Selection) -> (Pos, Pos) {
        if self.cmp_pos(sel.anchor, sel.focus) == Ordering::Greater {
            (sel.focus, sel.anchor)
        } else {
            (sel.anchor, sel.focus)
        }
    }

    /// Clamps a position to an existing paragraph and char boundary.
    pub fn clamp(&self, pos: Pos) -> Pos {
        match self.para(pos.para) {
            Some(p) => Pos::new(p.id, p.floor(pos.offset)),
            None => self.start(),
        }
    }

    /// The selected text, paragraphs separated by `\n`.
    pub fn text_in(&self, sel: Selection) -> String {
        let (a, b) = self.ordered(sel);
        let mut out = String::new();
        let mut inside = false;
        for p in self.paragraphs() {
            if p.id == a.para {
                inside = true;
            }
            if !inside {
                continue;
            }
            let start = if p.id == a.para { a.offset } else { 0 };
            let end = if p.id == b.para { b.offset } else { p.text.len() };
            if !out.is_empty() || p.id != a.para {
                out.push('\n');
            }
            out.push_str(&p.text[start.min(end)..end]);
            if p.id == b.para {
                break;
            }
        }
        out
    }

    /// Words in the document (Unicode word segmentation).
    pub fn word_count(&self) -> usize {
        use unicode_segmentation::UnicodeSegmentation;
        self.text_paragraphs().map(|p| p.text.unicode_words().count()).sum()
    }

    /// Characters in the document, not counting paragraph breaks.
    pub fn char_count(&self) -> usize {
        self.text_paragraphs().map(|p| p.text.chars().count()).sum()
    }

    /// The paragraphs of the text itself (not headers and footers).
    pub fn text_paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        let mut skip = std::collections::HashSet::new();
        for s in &self.sections {
            let mut v = Vec::new();
            for h in &s.headers {
                walk(&h.blocks, &mut v);
            }
            skip.extend(v.iter().map(|p| p.id));
        }
        self.paragraphs().filter(move |p| !skip.contains(&p.id))
    }

    /// All the text, paragraphs separated by `\n`.
    pub fn plain_text(&self) -> String {
        self.paragraphs().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")
    }
}

fn is_default_style(s: &crate::cite::CiteStyle) -> bool {
    *s == crate::cite::CiteStyle::default()
}

/// Superscript or subscript ([`VertAlign::Baseline`] in a patch turns either off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertAlign {
    Baseline,
    Superscript,
    Subscript,
}
