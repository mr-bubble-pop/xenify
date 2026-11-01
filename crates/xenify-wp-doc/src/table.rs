//! Tables, images and the table of contents: blocks other than paragraphs.

use crate::{Align, Block, NodeId, Paragraph, Rgba, StyleSheet};
use serde::{Deserialize, Serialize};

/// Where a list of blocks lives: a section's body or a table cell.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Section(usize),
    Cell {
        table: NodeId,
        row: usize,
        col: usize,
    },
    /// A positioned frame's own blocks.
    Frame(NodeId),
    /// A footnote's or endnote's text.
    Note(NodeId),
    /// A rich header's or footer's content.
    Header(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Border {
    /// Line width in points (0 = none).
    pub width: f32,
    pub color: Rgba,
}

impl Default for Border {
    fn default() -> Self {
        Self { width: 0.75, color: Rgba::rgb(0x9e, 0x9e, 0x9e) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Table {
    pub id: NodeId,
    /// Relative column widths (any positive numbers; they share the width).
    pub widths: Vec<f32>,
    pub rows: Vec<Row>,
    /// Lines around and between cells (`None`: no lines).
    pub border: Option<Border>,
    /// Space between a cell's edge and its text, in points.
    pub padding: f32,
    /// Share of the available width the table takes (0–1).
    pub width: f32,
    pub align: Align,
    /// The first column labels the rows (drawn like a header).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub header_col: bool,
    /// Every other body row shaded with this colour (banded rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<Rgba>,
    /// Colour behind header rows and the header column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_fill: Option<Rgba>,
    /// Header rows shown again at the top of each page the table goes on to.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeat_header: bool,
    /// Footer rows (totals) shown at the bottom of each page it runs over.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeat_footer: bool,
    /// Shows something from another document and follows it (Insert › From
    /// Calc): `server|node|item`. None once it's the document's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked: Option<String>,
}

/// Where a cell's content sits when the cell is taller than it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            id: NodeId(0),
            widths: Vec::new(),
            rows: Vec::new(),
            border: Some(Border::default()),
            padding: 5.0,
            width: 1.0,
            align: Align::Left,
            header_col: false,
            band: None,
            header_fill: None,
            repeat_header: false,
            repeat_footer: false,
            linked: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Row {
    pub id: NodeId,
    /// A header row (at the top; repeated on each page with
    /// [`Table::repeat_header`]).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub header: bool,
    /// A footer row, e.g. totals (at the bottom; repeated on each page with
    /// [`Table::repeat_footer`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub footer: bool,
    /// Mail merge: repeat this row once per item of this list field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<String>,
    /// The row is at least this tall, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_height: Option<f32>,
    /// Exactly one cell per column (cells covered by a merge are marked).
    pub cells: Vec<Cell>,
}

fn one() -> u16 {
    1
}

fn is_one(v: &u16) -> bool {
    *v == 1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub id: NodeId,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub col_span: u16,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub row_span: u16,
    /// Hidden under a merged cell to its left or above.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub covered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<Rgba>,
    /// Lines on this cell's sides, over the table's own lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borders: Option<crate::ParaBorders>,
    #[serde(default, skip_serializing_if = "is_top")]
    pub v_align: VAlign,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

fn is_top(v: &VAlign) -> bool {
    *v == VAlign::Top
}

impl Cell {
    pub fn new(id: NodeId, para: NodeId) -> Self {
        Self {
            id,
            col_span: 1,
            row_span: 1,
            covered: false,
            shading: None,
            borders: None,
            v_align: VAlign::Top,
            blocks: vec![Block::Paragraph(Paragraph::new(para, StyleSheet::NORMAL, ""))],
        }
    }
}

impl Table {
    /// A `rows`×`cols` table with empty cells; ids come from `alloc`.
    pub fn new(rows: usize, cols: usize, alloc: &mut impl FnMut() -> NodeId) -> Self {
        let rows = (0..rows.max(1))
            .map(|_| Row {
                id: alloc(),
                header: false,
                footer: false,
                repeat: None,
                min_height: None,
                cells: (0..cols.max(1)).map(|_| Cell::new(alloc(), alloc())).collect(),
            })
            .collect();
        Self { id: alloc(), widths: vec![1.0; cols.max(1)], rows, ..Default::default() }
    }

    pub fn cols(&self) -> usize {
        self.widths.len()
    }

    /// The cell covering grid position (row, col): itself, or the merged cell over it.
    pub fn anchor_of(&self, row: usize, col: usize) -> (usize, usize) {
        for r in (0..=row).rev() {
            for c in (0..=col).rev() {
                let cell = &self.rows[r].cells[c];
                if !cell.covered && r + cell.row_span as usize > row && c + cell.col_span as usize > col {
                    return (r, c);
                }
            }
        }
        (row, col)
    }

    /// Makes the grid consistent: every row has one cell per column, spans
    /// stay inside the table, covered cells match the spans.
    pub fn normalize(&mut self, alloc: &mut impl FnMut() -> NodeId) {
        let cols = self.widths.len().max(self.rows.iter().map(|r| r.cells.len()).max().unwrap_or(1)).max(1);
        self.widths.resize(cols, 1.0);
        for w in &mut self.widths {
            if w.is_nan() || *w <= 0.0 {
                *w = 1.0;
            }
        }
        let nrows = self.rows.len();
        for row in &mut self.rows {
            while row.cells.len() < cols {
                row.cells.push(Cell::new(alloc(), alloc()));
            }
            row.cells.truncate(cols);
        }
        for r in 0..nrows {
            for c in 0..cols {
                self.rows[r].cells[c].covered = false;
            }
        }
        for r in 0..nrows {
            for c in 0..cols {
                let (cs, rs) = {
                    let cell = &mut self.rows[r].cells[c];
                    if cell.covered {
                        continue;
                    }
                    cell.col_span = cell.col_span.clamp(1, (cols - c) as u16);
                    cell.row_span = cell.row_span.clamp(1, (nrows - r) as u16);
                    (cell.col_span as usize, cell.row_span as usize)
                };
                for rr in r..r + rs {
                    for cc in c..c + cs {
                        if (rr, cc) != (r, c) {
                            self.rows[rr].cells[cc].covered = true;
                            self.rows[rr].cells[cc].col_span = 1;
                            self.rows[rr].cells[cc].row_span = 1;
                        }
                    }
                }
            }
        }
        for row in &mut self.rows {
            for cell in &mut row.cells {
                if !cell.blocks.iter().any(|b| matches!(b, Block::Paragraph(_))) {
                    cell.blocks.push(Block::Paragraph(Paragraph::new(alloc(), StyleSheet::NORMAL, "")));
                }
            }
        }
    }
}

/// A picture on its own line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Image {
    pub id: NodeId,
    /// Key in [`crate::Document::assets`] (e.g. `media/photo.png`), or a
    /// path/URL for images kept outside the document.
    pub src: String,
    /// Description for screen readers and when the image can't be shown.
    pub alt: String,
    /// Size on the page in points (0: the image's own size, fitted to the column).
    pub width: f32,
    pub height: f32,
    pub align: Align,
    /// A line around the picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
}

/// A table of contents built from the headings (outline levels 1…`levels`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toc {
    pub id: NodeId,
    pub levels: u8,
    /// Show page numbers (with dot leaders).
    pub page_numbers: bool,
    /// What the block lists: headings, cited sources, or captions.
    #[serde(default, skip_serializing_if = "is_contents")]
    pub kind: TocKind,
}

impl Default for Toc {
    fn default() -> Self {
        Self { id: NodeId(0), levels: 3, page_numbers: true, kind: TocKind::Contents }
    }
}

/// How body text treats a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wrap {
    /// Text flows beside the frame, on the wider side.
    #[default]
    Around,
    /// Text stops above the frame and continues below it.
    TopBottom,
    /// The frame sits on top of the text.
    InFront,
    /// The frame sits behind the text (e.g. a watermark).
    Behind,
}

impl Wrap {
    pub const ALL: [Wrap; 4] = [Wrap::Around, Wrap::TopBottom, Wrap::InFront, Wrap::Behind];

    pub fn name(self) -> &'static str {
        match self {
            Wrap::Around => "Around",
            Wrap::TopBottom => "Top and bottom",
            Wrap::InFront => "In front of text",
            Wrap::Behind => "Behind text",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Wrap::Around => "around",
            Wrap::TopBottom => "top-bottom",
            Wrap::InFront => "in-front",
            Wrap::Behind => "behind",
        }
    }

    pub fn from_key(s: &str) -> Option<Wrap> {
        Wrap::ALL.into_iter().find(|w| w.key() == s)
    }
}

/// A box placed at a fixed spot on a page (a text box, a positioned picture,
/// an address block), holding its own blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    pub id: NodeId,
    /// The page of its section it is on (0 = the section's first page).
    pub page: u32,
    /// Position and size on the page, in points from the page's top-left corner.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub wrap: Wrap,
    /// Space kept between the frame and wrapped text.
    pub gap: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Rgba>,
    /// Space between the frame's edge and its content.
    pub padding: f32,
    pub blocks: Vec<Block>,
    /// Threaded text: the frame the story continues in when it overflows (M11).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<NodeId>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            id: NodeId(0),
            page: 0,
            x: 72.0,
            y: 72.0,
            width: 200.0,
            height: 100.0,
            wrap: Wrap::Around,
            gap: 8.0,
            border: None,
            fill: None,
            padding: 4.0,
            blocks: Vec::new(),
            next: None,
        }
    }
}

impl Frame {
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.x + self.width, self.y + self.height)
    }
}

/// What a generated list block shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TocKind {
    /// Table of contents (headings).
    #[default]
    Contents,
    /// The cited sources (References › Bibliography).
    Bibliography,
    /// Captions of figures and tables.
    Figures,
    /// The change log (File › Record a change), as a table.
    ChangeLog,
}

fn is_contents(k: &TocKind) -> bool {
    *k == TocKind::Contents
}
