//! Layout of blocks other than paragraphs: tables, images and the table of contents.

use crate::para::{self, Glyph, Item, ParaLayout};
use crate::{Engine, gap_between};
use xenify_wp_doc::{Align, Block, Document, NodeId, Paragraph, Rgba, Table};
use kurbo::Rect;
use std::sync::Arc;

/// A picture inside a cell: x from the cell's content edge, y from the cell top.
#[derive(Clone)]
pub struct CellImage {
    pub id: NodeId,
    pub src: String,
    pub alt: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub border: Option<xenify_wp_doc::Border>,
}

/// A cell laid out, relative to its row's top-left.
#[derive(Clone)]
pub struct CellLayout {
    /// Grid column of the cell (its left when merged) and its width in columns.
    pub col: usize,
    pub col_span: usize,
    /// The cell's box (x from the table's left edge; y from the row top, height filled in later).
    pub x: f32,
    pub width: f32,
    pub row_span: usize,
    pub shading: Option<Rgba>,
    pub borders: Option<xenify_wp_doc::ParaBorders>,
    pub v_align: xenify_wp_doc::VAlign,
    /// Paragraphs and their tops (paragraph y = 0) inside the cell.
    pub paras: Vec<(Arc<ParaLayout>, f32)>,
    pub images: Vec<CellImage>,
    /// Tables inside the cell and their tops.
    pub tables: Vec<(Arc<TableLayout>, f32)>,
    /// Height of the content plus padding.
    pub content_height: f32,
    /// Space between the cell's edge and its content.
    pub pad: f32,
}

#[derive(Clone)]
pub struct RowLayout {
    pub header: bool,
    pub footer: bool,
    pub height: f32,
    pub cells: Vec<CellLayout>,
}

/// A laid-out table: rows with fixed heights (rows don't split across pages).
#[derive(Clone)]
pub struct TableLayout {
    pub id: NodeId,
    /// Left edge relative to the column.
    pub x: f32,
    pub rows: Vec<RowLayout>,
    pub border: Option<xenify_wp_doc::Border>,
    /// Header rows shown again on each page; footer rows at the bottom of each.
    pub repeat_header: bool,
    pub repeat_footer: bool,
}

impl TableLayout {
    /// Consecutive rows that must stay together (joined by merged cells).
    pub fn groups(&self) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut reach = 0;
        for (r, row) in self.rows.iter().enumerate() {
            for c in &row.cells {
                reach = reach.max(r + c.row_span);
            }
            if reach <= r + 1 {
                out.push(start..r + 1);
                start = r + 1;
            }
        }
        if start < self.rows.len() {
            out.push(start..self.rows.len());
        }
        out
    }

    pub fn header_rows(&self) -> usize {
        self.rows.iter().take_while(|r| r.header).count()
    }

    /// Where the footer rows start (the row count when there are none).
    pub fn footer_start(&self) -> usize {
        let heads = self.header_rows();
        self.rows.len() - self.rows.iter().rev().take_while(|r| r.footer).count().min(self.rows.len() - heads)
    }
}

pub(crate) fn layout_table(
    engine: &mut Engine,
    doc: &Document,
    t: &Table,
    avail: f32,
    labels: &std::collections::HashMap<NodeId, String>,
) -> TableLayout {
    let width = (avail * t.width.clamp(0.1, 1.0)).max(36.0);
    let x = match t.align {
        Align::Center => (avail - width) / 2.0,
        Align::Right => avail - width,
        _ => 0.0,
    };
    let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
    let col_x: Vec<f32> = std::iter::once(0.0)
        .chain(t.widths.iter().scan(0.0, |acc, w| {
            *acc += w / total * width;
            Some(*acc)
        }))
        .collect();
    let pad = t.padding;
    let mut rows: Vec<RowLayout> = Vec::new();
    let mut body_row = 0;
    for row in &t.rows {
        let mut cells = Vec::new();
        for (c, cell) in row.cells.iter().enumerate() {
            if cell.covered {
                continue;
            }
            let end = (c + cell.col_span as usize).min(t.widths.len());
            let (cx, cw) = (col_x[c], col_x[end] - col_x[c]);
            let inner = (cw - 2.0 * pad).max(12.0);
            let mut y = pad;
            let mut prev: Option<Arc<ParaLayout>> = None;
            let mut paras = Vec::new();
            let mut images = Vec::new();
            let mut tables = Vec::new();
            // Header cells (header and footer rows, the first column when it labels the rows): bold.
            let header = row.header || row.footer || t.header_col && c == 0;
            for b in &cell.blocks {
                match b {
                    Block::Paragraph(p) => {
                        let bold;
                        let p = if header {
                            let mut q = p.clone();
                            for r in &mut q.runs {
                                r.fmt.bold = r.fmt.bold.or(Some(true));
                            }
                            bold = q;
                            &bold
                        } else {
                            p
                        };
                        let pl = engine.para(doc, p, inner, labels.get(&p.id).map(String::as_str));
                        if prev.is_some() {
                            y += gap_between(prev.as_deref(), &pl);
                        }
                        paras.push((pl.clone(), y));
                        y += pl.height;
                        prev = Some(pl);
                    }
                    Block::Image(img) => {
                        let (w, h) = image_size(img, inner);
                        let x = match img.align {
                            Align::Center => (inner - w) / 2.0,
                            Align::Right => inner - w,
                            _ => 0.0,
                        };
                        images.push(CellImage {
                            id: img.id,
                            src: img.src.clone(),
                            alt: img.alt.clone(),
                            x,
                            y,
                            w,
                            h,
                            border: img.border,
                        });
                        y += h + 4.0;
                        prev = None;
                    }
                    Block::Table(inner_t) => {
                        let tl = layout_table(engine, doc, inner_t, inner, labels);
                        let h: f32 = tl.rows.iter().map(|r| r.height).sum();
                        tables.push((Arc::new(tl), y));
                        y += h + 4.0;
                        prev = None;
                    }
                    Block::Toc(_) => {}
                }
            }
            // Header rows, the header column and bands: shading unless the cell has its own.
            let shading = cell
                .shading
                .or(if header { t.header_fill } else { None })
                .or(if !header && body_row % 2 == 1 { t.band } else { None });
            cells.push(CellLayout {
                col: c,
                col_span: end - c,
                x: cx,
                width: cw,
                row_span: cell.row_span as usize,
                shading,
                borders: cell.borders,
                v_align: cell.v_align,
                paras,
                images,
                tables,
                content_height: y + pad,
                pad,
            });
        }
        if !row.header {
            body_row += 1;
        }
        let height = cells
            .iter()
            .filter(|c| c.row_span == 1)
            .map(|c| c.content_height)
            .fold(pad * 2.0 + 10.0, f32::max)
            .max(row.min_height.unwrap_or(0.0));
        rows.push(RowLayout { header: row.header, footer: row.footer, height, cells });
    }
    // Merged cells taller than their rows make the last row they cover taller.
    for r in 0..rows.len() {
        for c in 0..rows[r].cells.len() {
            let span = rows[r].cells[c].row_span;
            if span <= 1 {
                continue;
            }
            let last = (r + span - 1).min(rows.len() - 1);
            let have: f32 = rows[r..=last].iter().map(|x| x.height).sum();
            let need = rows[r].cells[c].content_height;
            if need > have {
                rows[last].height += need - have;
            }
        }
    }
    TableLayout { id: t.id, x, rows, border: t.border, repeat_header: t.repeat_header, repeat_footer: t.repeat_footer }
}

/// A picture's size on the page (fitted to the column).
pub(crate) fn image_size(img: &xenify_wp_doc::Image, avail: f32) -> (f32, f32) {
    let (mut w, mut h) = if img.width > 0.0 && img.height > 0.0 { (img.width, img.height) } else { (200.0, 150.0) };
    if w > avail {
        h *= avail / w;
        w = avail;
    }
    (w, h)
}

/// The headings a table of contents lists: (heading id, outline level, text).
pub fn toc_entries(doc: &Document, levels: u8) -> Vec<(NodeId, u8, String)> {
    doc.paragraphs()
        .filter_map(|p| {
            let level = doc.styles.resolve(&p.style).outline_level?;
            (level <= levels && !p.text.trim().is_empty()).then(|| (p.id, level, p.text.clone()))
        })
        .collect()
}

/// One contents entry: the heading's text, dot leaders and its page number.
pub(crate) fn layout_toc_entry(
    engine: &mut Engine,
    doc: &Document,
    heading: NodeId,
    level: u8,
    text: &str,
    page: Option<u32>,
    width: f32,
) -> ParaLayout {
    let number = page.map_or(String::new(), |n| n.to_string());
    let mut p = Paragraph::new(NodeId(0), format!("toc{}", level.clamp(1, 3)), text);
    // Leave room on the right for the number.
    p.props.indent_right = Some(if number.is_empty() { 0.0 } else { 40.0 });
    let mut pl = para::layout_paragraph(&mut engine.fonts, doc, &p, width, None);
    pl.link = Some(heading);
    if number.is_empty() {
        return pl;
    }
    let Some(last) = pl.lines.last().copied() else { return pl };
    let line_index = pl.lines.len() - 1;
    let style = doc.resolve_para(&p).char;
    let (items, num_w) = para::layout_label(&mut engine.fonts, &number, &style);
    let x0 = width - num_w;
    for mut it in items {
        if let Item::Glyphs { glyphs, line, .. } = &mut it {
            *line = line_index;
            for g in glyphs.iter_mut() {
                *g = Glyph { id: g.id, x: g.x + x0, y: g.y + last.baseline };
            }
        }
        pl.items.push(it);
    }
    // Dot leaders from the end of the text to the number.
    let text_end = pl
        .layout
        .lines()
        .last()
        .map_or(0.0, |l| l.metrics().offset + l.metrics().advance - l.metrics().trailing_whitespace)
        + pl.indent;
    let mut x = (text_end + 6.0).ceil();
    let dot = (style.size * 0.12).max(1.0) as f64;
    while x + 4.0 < x0 - 4.0 {
        let r = Rect::new(x as f64, last.baseline as f64 - dot, x as f64 + dot, last.baseline as f64);
        pl.items.push(Item::Rect { rect: r, color: Rgba::rgb(0x70, 0x70, 0x70), line: line_index });
        x += 4.0;
    }
    pl
}
