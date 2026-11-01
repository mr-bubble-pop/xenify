//! Flows blocks into columns and pages: paragraphs split between lines,
//! table rows stay whole (header rows repeat), pictures stay whole.

use crate::blocks::{self, TableLayout};
use crate::para::{self, ParaLayout};
use crate::{DocLayout, Engine, Fonts, Fragment, Page, PlacedCell, PlacedFrame, PlacedImage, Shape, gap_between};
use kurbo::{Point, Rect};
use std::collections::HashMap;
use std::sync::Arc;
use xenify_wp_doc::{Block, Document, Frame, HfLine, NodeId, Paragraph, Rgba, SectionStart, StyleSheet, Wrap};

/// Space between pages on screen, in points.
pub const PAGE_GAP: f32 = 18.0;
/// Space around tables and pictures.
const BLOCK_GAP: f32 = 8.0;

/// Something the flow places.
enum Unit<'a> {
    /// A paragraph's layout, and the paragraph (with its list label) so it
    /// can be laid out again around frames.
    Para(Arc<ParaLayout>, Option<(&'a Paragraph, Option<String>)>),
    /// A table: its row groups are placed one at a time.
    Table(Arc<TableLayout>),
    Image {
        id: NodeId,
        src: String,
        alt: String,
        w: f32,
        h: f32,
        align: xenify_wp_doc::Align,
        border: Option<xenify_wp_doc::Border>,
        /// From the left of the column: a picture inside a list item lines up with its text.
        indent: f32,
    },
}

#[derive(Clone, Copy, Debug)]
struct Region {
    x: f32,
    bottom: f32,
}

#[derive(Clone)]
struct Cursor {
    page: usize,
    col: usize,
    y: f32,
    /// Top of the column area on this page (below a continuous section above).
    area_top: f32,
    /// Nothing placed in this region yet.
    fresh: bool,
}

/// A frame body text must avoid: its page, its rectangle relative to the
/// page (grown by its gap), and how text wraps around it.
#[derive(Clone, Copy, Debug)]
struct Obstacle {
    page: usize,
    rect: Rect,
    wrap: Wrap,
}

struct Flow<'a> {
    doc: &'a Document,
    out: &'a mut DocLayout,
    fonts: &'a mut Fonts,
    obstacles: &'a [Obstacle],
    pageless: bool,
    /// Where each piece went: (unit, first piece, page).
    placed: Vec<(usize, usize, usize)>,
    /// Footnotes placed so far, page by page.
    notes: &'a mut Footnotes,
    /// Per section: where the text must start and end so rich headers and
    /// footers taller than their margin don't run into it (page-relative).
    hf_room: &'a [(f32, f32)],
}

/// Footnotes at the bottom of pages: what each page holds and the room it takes.
#[derive(Default)]
struct Footnotes {
    /// Page → (height including the separator, the notes' paragraphs laid out).
    pages: HashMap<usize, (f32, Vec<Arc<ParaLayout>>)>,
    /// Notes already placed (a note referred to twice goes where it's first).
    placed: std::collections::HashSet<NodeId>,
    /// Laid-out notes by id.
    laid_out: HashMap<NodeId, (f32, Vec<Arc<ParaLayout>>)>,
}

/// Space between the text and the footnotes (with the separator line).
const NOTE_GAP: f32 = 14.0;

impl Flow<'_> {
    /// Footnote `id` laid out at the body width (its number hangs before the first paragraph).
    fn note_layout(&mut self, id: NodeId, width: f32) -> (f32, Vec<Arc<ParaLayout>>) {
        if let Some(l) = self.notes.laid_out.get(&id) {
            return l.clone();
        }
        let numbers = xenify_wp_doc::notes::numbers(self.doc);
        let label = numbers.get(&id).cloned().unwrap_or_default();
        let mut out = Vec::new();
        let mut h = 0.0;
        if let Some(n) = self.doc.note(id) {
            for (i, b) in n.blocks.iter().enumerate() {
                let Block::Paragraph(p) = b else { continue };
                let mut q = p.clone();
                if i == 0 && q.props.indent_left.is_none() {
                    // Room for the number, which hangs.
                    q.props.indent_left = Some(14.0);
                }
                let pl = Arc::new(para::layout_paragraph(
                    self.fonts,
                    self.doc,
                    &q,
                    width,
                    (i == 0).then_some(label.as_str()),
                ));
                h += pl.height + 2.0;
                out.push(pl);
            }
        }
        self.notes.laid_out.insert(id, (h, out.clone()));
        (h, out)
    }

    /// How many of lines `li..li + k` of `pl` fit at the cursor once the
    /// footnotes they refer to take their room at the page bottom (and
    /// reserves that room).
    fn fit_notes(&mut self, pl: &ParaLayout, li: usize, mut k: usize, c: &Cursor, bottom: f32, width: f32) -> usize {
        if self.pageless {
            return k;
        }
        let Some(p) = self.doc.para(pl.id) else { return k };
        let refs = xenify_wp_doc::notes::references(p);
        if refs.is_empty() {
            return k;
        }
        let lines: Vec<std::ops::Range<usize>> = pl.layout.lines().map(|l| l.text_range()).collect();
        while k > 0 {
            let range = lines.get(li).map_or(0, |r| r.start)..lines.get(li + k - 1).map_or(usize::MAX, |r| r.end);
            let new: Vec<NodeId> = refs
                .iter()
                .filter(|(at, id)| range.contains(at) && !self.notes.placed.contains(id))
                .filter(|(_, id)| self.doc.note(*id).is_some_and(|n| n.kind == xenify_wp_doc::NoteKind::Foot))
                .map(|(_, id)| *id)
                .collect();
            if new.is_empty() {
                return k;
            }
            let have = self.notes.pages.get(&c.page).map_or(0.0, |p| p.0);
            let extra: f32 = new.iter().map(|id| self.note_layout(*id, width).0).sum::<f32>()
                + if have == 0.0 { NOTE_GAP } else { 0.0 };
            let text_bottom = c.y + (pl.lines[li + k - 1].bottom - piece_top(pl, li));
            if text_bottom <= bottom - extra || (k == 1 && c.fresh) {
                for id in new {
                    let (h, ls) = self.note_layout(id, width);
                    let e = self.notes.pages.entry(c.page).or_insert((NOTE_GAP, Vec::new()));
                    e.0 += h;
                    e.1.extend(ls);
                    self.notes.placed.insert(id);
                }
                return k;
            }
            k -= 1;
        }
        0
    }
}

/// Where a line may go among `obs` (absolute rects) in the column at
/// `colx`..`colx + colw`, for a paragraph whose top is at `origin_y`: see [`para::LineShape`].
fn line_slot(obs: &[Rect], tb: &[bool], colx: f32, colw: f32, origin_y: f32, t: f32, h: f32) -> (f32, f32, f32) {
    let mut y = t;
    for _ in 0..64 {
        let (a, b) = ((origin_y + y) as f64, (origin_y + y + h) as f64);
        let hits: Vec<usize> = (0..obs.len()).filter(|&i| obs[i].y0 < b && obs[i].y1 > a).collect();
        if hits.is_empty() {
            return (y, 0.0, colw);
        }
        // "Top and bottom" frames: the line goes below them.
        if let Some(bottom) = hits.iter().filter(|&&i| tb[i]).map(|&i| obs[i].y1).reduce(f64::max) {
            y = bottom as f32 - origin_y;
            continue;
        }
        // The widest free span beside the frames.
        let mut free = vec![(colx as f64, (colx + colw) as f64)];
        for &i in &hits {
            let (x0, x1) = (obs[i].x0, obs[i].x1);
            free = free
                .into_iter()
                .flat_map(|(a, b)| {
                    let mut v = Vec::new();
                    if x0 > a {
                        v.push((a, x0.min(b)));
                    }
                    if x1 < b {
                        v.push((x1.max(a), b));
                    }
                    v
                })
                .filter(|(a, b)| b > a)
                .collect();
        }
        match free.iter().max_by(|p, q| (p.1 - p.0).total_cmp(&(q.1 - q.0))) {
            Some(&(a, b)) if b - a >= 36.0 => return (y, (a - colx as f64) as f32, (b - a) as f32),
            _ => y = hits.iter().map(|&i| obs[i].y1).fold(f64::INFINITY, f64::min) as f32 - origin_y,
        }
    }
    (y, 0.0, colw)
}

impl Flow<'_> {
    /// Frames on page `page` that meet the column `x`..`x + w` between `y0` and `y1` (absolute).
    fn obstacles_near(&self, page: usize, x: f32, w: f32, y0: f32, y1: f32) -> (Vec<Rect>, Vec<bool>) {
        let top = self.out.pages[page].rect.y0;
        let mut rects = Vec::new();
        let mut tb = Vec::new();
        for o in self.obstacles.iter().filter(|o| o.page == page && matches!(o.wrap, Wrap::Around | Wrap::TopBottom)) {
            let r = o.rect + kurbo::Vec2::new(0.0, top);
            if r.x0 < (x + w) as f64 && r.x1 > x as f64 && r.y1 > y0 as f64 && r.y0 < y1 as f64 {
                rects.push(r);
                tb.push(o.wrap == Wrap::TopBottom);
            }
        }
        (rects, tb)
    }

    /// The first y at or below `y` where a block `h` tall misses every frame in the column.
    fn clear_of_frames(&self, page: usize, x: f32, w: f32, y: f32, h: f32) -> f32 {
        let mut y = y;
        for _ in 0..32 {
            let (rects, _) = self.obstacles_near(page, x, w, y, y + h);
            match rects.iter().map(|r| r.y1).reduce(f64::max) {
                Some(b) => y = b as f32,
                None => break,
            }
        }
        y
    }

    fn new_page(&mut self, section: usize) -> usize {
        let ps = &self.doc.sections[section].page;
        let top = self.out.pages.last().map_or(0.0, |p| p.rect.y1 as f32 + PAGE_GAP);
        let height = if self.pageless { 1.0e7 } else { ps.height };
        let rect = Rect::new(0.0, top as f64, ps.width as f64, (top + height) as f64);
        let m = &ps.margins;
        let (room_top, room_bottom) =
            if self.pageless { (0.0, 0.0) } else { self.hf_room.get(section).copied().unwrap_or_default() };
        let body = Rect::new(
            m.left as f64,
            (top + m.top.max(room_top)) as f64,
            (ps.width - m.right) as f64,
            (top + height - m.bottom.max(room_bottom)) as f64,
        );
        self.out.pages.push(Page { number: 0, section, rect, body, first_of_section: false });
        self.out.pages.len() - 1
    }

    fn columns(&self, section: usize) -> usize {
        if self.pageless { 1 } else { self.doc.sections[section].page.columns.max(1) as usize }
    }

    fn column_width(&self, section: usize) -> f32 {
        let ps = &self.doc.sections[section].page;
        if self.pageless { ps.content_width() } else { ps.column_width() }
    }

    fn region(&self, section: usize, c: &Cursor) -> Region {
        let ps = &self.doc.sections[section].page;
        let page = &self.out.pages[c.page];
        let col = c.col.min(self.columns(section) - 1);
        let notes = self.notes.pages.get(&c.page).map_or(0.0, |p| p.0);
        Region {
            x: page.body.x0 as f32 + col as f32 * (self.column_width(section) + ps.column_gap),
            bottom: page.body.y1 as f32 - notes,
        }
    }

    fn next_region(&mut self, section: usize, c: &mut Cursor) {
        if c.col + 1 < self.columns(section) {
            c.col += 1;
        } else {
            c.page = self.new_page(section);
            c.col = 0;
            c.area_top = self.out.pages[c.page].body.y0 as f32;
        }
        c.y = c.area_top;
        c.fresh = true;
    }

    fn new_page_cursor(&mut self, section: usize, c: &mut Cursor) {
        c.page = self.new_page(section);
        c.col = 0;
        c.area_top = self.out.pages[c.page].body.y0 as f32;
        c.y = c.area_top;
        c.fresh = true;
    }

    fn at_page_top(&self, c: &Cursor) -> bool {
        c.fresh && c.col == 0 && c.y <= self.out.pages[c.page].body.y0 as f32 + 0.01
    }

    /// Draws rows `rows` of `t` at (x, y); `copy` for repeated header rows.
    fn place_rows(&mut self, t: &TableLayout, rows: std::ops::Range<usize>, x: f32, y: f32, copy: bool) -> f32 {
        place_rows(self.out, t, rows, x, y, copy)
    }
}

/// Draws rows `rows` of `t` at (x, y) into `out`; `copy` for repeated header rows.
fn place_rows(out: &mut DocLayout, t: &TableLayout, rows: std::ops::Range<usize>, x: f32, y: f32, copy: bool) -> f32 {
    let mut ry = y;
    let border = t.border.filter(|b| b.width > 0.0);
    for r in rows.clone() {
        let row = &t.rows[r];
        for cell in &row.cells {
            // A merged cell spans its rows (clipped to what's placed here).
            let last = (r + cell.row_span).min(rows.end);
            let h: f32 = t.rows[r..last].iter().map(|x| x.height).sum();
            let rect = Rect::new((x + cell.x) as f64, ry as f64, (x + cell.x + cell.width) as f64, (ry + h) as f64);
            out.cells.push(PlacedCell {
                table: t.id,
                row: r,
                col: cell.col,
                row_span: cell.row_span,
                col_span: cell.col_span,
                rect,
                repeat: copy,
            });
            if let Some(color) = cell.shading {
                out.shapes.push(Shape { rect, color, front: false });
            }
            // Each side: the cell's own line, else the table's.
            let sides = cell.borders.unwrap_or_default();
            let side = |own: Option<xenify_wp_doc::Border>| own.or(border).filter(|b| b.width > 0.0);
            for (k, line) in [sides.top, sides.bottom, sides.left, sides.right].into_iter().map(side).enumerate() {
                let Some(b) = line else { continue };
                let w = b.width as f64 / 2.0;
                let r = match k {
                    0 => Rect::new(rect.x0, rect.y0 - w, rect.x1, rect.y0 + w),
                    1 => Rect::new(rect.x0, rect.y1 - w, rect.x1, rect.y1 + w),
                    2 => Rect::new(rect.x0 - w, rect.y0, rect.x0 + w, rect.y1),
                    _ => Rect::new(rect.x1 - w, rect.y0, rect.x1 + w, rect.y1),
                };
                out.shapes.push(Shape { rect: r, color: b.color, front: true });
            }
            // Vertical alignment: the content moves down in a taller cell.
            let spare = (h - cell.content_height).max(0.0);
            let dy = match cell.v_align {
                xenify_wp_doc::VAlign::Top => 0.0,
                xenify_wp_doc::VAlign::Middle => spare / 2.0,
                xenify_wp_doc::VAlign::Bottom => spare,
            };
            let left = x + cell.x + cell.pad;
            for (pl, py) in &cell.paras {
                let origin = Point::new(left as f64, (ry + dy + py - pl.lines[0].top) as f64);
                let f = Fragment { para: pl.clone(), origin, lines: 0..pl.lines.len() };
                if copy {
                    out.decor.push(f);
                } else {
                    out.push(f);
                }
            }
            for img in &cell.images {
                let r = Rect::new(
                    (left + img.x) as f64,
                    (ry + dy + img.y) as f64,
                    (left + img.x + img.w) as f64,
                    (ry + dy + img.y + img.h) as f64,
                );
                out.images.push(PlacedImage { id: img.id, src: img.src.clone(), alt: img.alt.clone(), rect: r });
                image_border(&mut out.shapes, r, img.border);
            }
            for (inner, ty) in &cell.tables {
                place_rows(out, inner, 0..inner.rows.len(), left + inner.x, ry + dy + ty, copy);
            }
        }
        ry += row.height;
    }
    ry
}

/// Where `pl`'s piece starting at line `from` begins, in paragraph space: a
/// continuation starts at its first line, the paragraph itself at 0 (so a
/// first line pushed down past a frame keeps its gap).
fn piece_top(pl: &ParaLayout, from: usize) -> f32 {
    if from == 0 { pl.lines[0].top.min(0.0) } else { pl.lines[from].top }
}

/// How many of `pl`'s lines from `from` fit between `y` and `bottom`.
fn lines_fitting(pl: &ParaLayout, from: usize, y: f32, bottom: f32) -> usize {
    let top = piece_top(pl, from);
    pl.lines[from..].iter().take_while(|l| y + (l.bottom - top) <= bottom + 0.01).count()
}

fn gap(prev: Option<&Unit>, next: &Unit) -> f32 {
    match (prev, next) {
        (Some(Unit::Para(a, _)), Unit::Para(b, _)) => gap_between(Some(a), b),
        (None, Unit::Para(b, _)) => b.space_before,
        (Some(Unit::Para(a, _)), _) => a.space_after.max(BLOCK_GAP),
        (_, Unit::Para(b, _)) => b.space_before.max(BLOCK_GAP),
        _ => BLOCK_GAP,
    }
}

/// Places `units` of section `section` from piece `start`, at `c`. `limit`
/// caps the columns on the page the flow starts on (column balancing).
fn flow_section(
    f: &mut Flow,
    section: usize,
    units: &[Unit],
    start: (usize, usize),
    c: &mut Cursor,
    limit: Option<f32>,
) {
    let first_page = c.page;
    let (mut ui, mut li) = start;
    // Paragraphs laid out again around frames.
    let mut shaped: HashMap<usize, Arc<ParaLayout>> = HashMap::new();
    while ui < units.len() {
        let unit = &units[ui];
        let prev = ui.checked_sub(1).map(|i| &units[i]);
        let mut region = f.region(section, c);
        if c.page == first_page
            && let Some(l) = limit
        {
            region.bottom = region.bottom.min(l);
        }
        let col_w = f.column_width(section);
        match unit {
            Unit::Para(pl, src) => {
                if li == 0 {
                    if pl.page_break_before && !f.pageless && !f.at_page_top(c) {
                        f.new_page_cursor(section, c);
                        continue;
                    }
                    if pl.column_break_before && !f.pageless && !c.fresh {
                        f.next_region(section, c);
                        continue;
                    }
                    if !c.fresh {
                        c.y += gap(prev, unit);
                    }
                    // Frames in the way: lay the lines out around them.
                    shaped.remove(&ui);
                    if let Some((p, label)) = src {
                        let (obs, tb) = f.obstacles_near(c.page, region.x, col_w, c.y, region.bottom);
                        if !obs.is_empty() {
                            let (colx, top) = (region.x, c.y);
                            let shape = |t: f32, h: f32| line_slot(&obs, &tb, colx, col_w, top, t, h);
                            let l =
                                para::layout_paragraph_shaped(f.fonts, f.doc, p, col_w, label.as_deref(), Some(&shape));
                            shaped.insert(ui, Arc::new(l));
                        }
                    }
                }
                let pl = shaped.get(&ui).unwrap_or(pl);
                let remaining = pl.lines.len() - li;
                let mut k = lines_fitting(pl, li, c.y, region.bottom);
                if k < remaining {
                    if pl.keep_together && li == 0 && !c.fresh {
                        k = 0;
                    } else if pl.widow_control && remaining >= 2 {
                        if li == 0 && k == 1 && !c.fresh {
                            k = 0;
                        }
                        if k > 0 && remaining - k == 1 {
                            k = if k >= 2 && (li > 0 || k > 2) {
                                k - 1
                            } else if c.fresh {
                                k
                            } else {
                                0
                            };
                        }
                    }
                } else if pl.keep_with_next
                    && !c.fresh
                    && let Some(next) = units.get(ui + 1)
                {
                    let bottom = c.y + (pl.lines[pl.lines.len() - 1].bottom - piece_top(pl, li));
                    let y_next = bottom + gap(Some(unit), next);
                    let fits = match next {
                        Unit::Para(n, _) => {
                            let need = n.lines.len().min(if n.widow_control { 2 } else { 1 });
                            need == 0 || lines_fitting(n, 0, y_next, region.bottom) >= need
                        }
                        Unit::Table(t) => {
                            let first: f32 =
                                t.groups().first().map_or(0.0, |g| t.rows[g.clone()].iter().map(|r| r.height).sum());
                            y_next + first <= region.bottom
                        }
                        Unit::Image { h, .. } => y_next + h <= region.bottom,
                    };
                    if !fits {
                        k = 0;
                    }
                }
                let body_w = f.out.pages[c.page].body.width() as f32;
                k = f.fit_notes(pl, li, k, c, region.bottom, body_w);
                if k == 0 {
                    if c.fresh {
                        k = 1;
                    } else {
                        f.next_region(section, c);
                        continue;
                    }
                }
                let origin = Point::new(region.x as f64, (c.y - piece_top(pl, li)) as f64);
                f.placed.push((ui, li, c.page));
                f.out.push(Fragment { para: pl.clone(), origin, lines: li..li + k });
                c.y = origin.y as f32 + pl.lines[li + k - 1].bottom;
                c.fresh = false;
                li += k;
                if li >= pl.lines.len() {
                    ui += 1;
                    li = 0;
                } else {
                    f.next_region(section, c);
                }
            }
            Unit::Table(t) => {
                let groups = t.groups();
                if groups.is_empty() {
                    ui += 1;
                    li = 0;
                    continue;
                }
                if li == 0 && !c.fresh {
                    c.y += gap(prev, unit);
                }
                let g = groups[li].clone();
                let h: f32 = t.rows[g.clone()].iter().map(|r| r.height).sum();
                // Header rows again at the top of each continuation (when asked for).
                let headers = t.header_rows();
                let repeat = t.repeat_header && li > 0 && c.fresh && headers > 0 && g.start >= headers;
                let head_h: f32 = if repeat { t.rows[..headers].iter().map(|r| r.height).sum() } else { 0.0 };
                // Footer rows at the bottom of each page it runs over: room is kept for them.
                let foot = t.footer_start();
                let reserve = t.repeat_footer && foot < t.rows.len() && g.end <= foot;
                let foot_h: f32 = if reserve { t.rows[foot..].iter().map(|r| r.height).sum() } else { 0.0 };
                if c.y + head_h + h + foot_h > region.bottom + 0.01 && !c.fresh {
                    // The table breaks here: its footer rows close this page's part.
                    if reserve && li > 0 && f.placed.last().is_some_and(|(u, _, p)| *u == ui && *p == c.page) {
                        let x = region.x + t.x;
                        f.place_rows(t, foot..t.rows.len(), x, c.y, true);
                    }
                    f.next_region(section, c);
                    continue;
                }
                let x = region.x + t.x;
                let clear = f.clear_of_frames(c.page, region.x, col_w, c.y, head_h + h);
                if clear > c.y {
                    c.y = clear;
                    if c.y + head_h + h > region.bottom + 0.01 {
                        f.next_region(section, c);
                        continue;
                    }
                }
                if repeat {
                    c.y = f.place_rows(t, 0..headers, x, c.y, true);
                }
                f.placed.push((ui, li, c.page));
                c.y = f.place_rows(t, g, x, c.y, false);
                c.fresh = false;
                li += 1;
                if li >= groups.len() {
                    ui += 1;
                    li = 0;
                }
            }
            Unit::Image { id, src, alt, w, h, align, border, indent } => {
                if !c.fresh {
                    c.y += gap(prev, unit);
                }
                if c.y + h > region.bottom + 0.01 && !c.fresh {
                    f.next_region(section, c);
                    continue;
                }
                let clear = f.clear_of_frames(c.page, region.x, col_w, c.y, *h);
                if clear > c.y {
                    c.y = clear;
                    if c.y + h > region.bottom + 0.01 {
                        f.next_region(section, c);
                        continue;
                    }
                }
                let dx = indent
                    + match align {
                        xenify_wp_doc::Align::Center => (col_w - indent - w) / 2.0,
                        xenify_wp_doc::Align::Right => col_w - indent - w,
                        _ => 0.0,
                    };
                let rect = Rect::new((region.x + dx) as f64, c.y as f64, (region.x + dx + w) as f64, (c.y + h) as f64);
                f.placed.push((ui, 0, c.page));
                f.out.images.push(PlacedImage { id: *id, src: src.clone(), alt: alt.clone(), rect });
                image_border(&mut f.out.shapes, rect, *border);
                c.y += h;
                c.fresh = false;
                ui += 1;
                li = 0;
            }
        }
    }
}

/// The units of `blocks` laid out `width` wide.
fn units_of<'a>(
    engine: &mut Engine,
    doc: &Document,
    blocks: &'a [Block],
    width: f32,
    labels: &HashMap<NodeId, String>,
    toc_pages: &HashMap<NodeId, u32>,
) -> Vec<Unit<'a>> {
    let mut out = Vec::new();
    // The text indent of the list item just before (a picture after it goes in the item).
    // (Only when it's really in the item: after more of the item, or with the list going on below it.)
    let mut list_indent: Option<f32> = None;
    for (bi, b) in blocks.iter().enumerate() {
        let start = out.len();
        let next_in_list = matches!(blocks.get(bi + 1), Some(Block::Paragraph(n)) if n.list.is_some());
        let after_item = list_indent.take();
        match b {
            Block::Paragraph(p) => {
                if let Some(item) = &p.list
                    && (item.follow
                        || next_in_list
                        || matches!(blocks.get(bi + 2), Some(Block::Paragraph(n)) if n.list.is_some()))
                {
                    list_indent = Some(doc.resolve_para(p).indent_left.clamp(0.0, width * 0.6));
                }
                let label = labels.get(&p.id).cloned();
                let mut pl = engine.para(doc, p, width, label.as_deref());
                // In a template, where each document's text goes says so (on screen only).
                if doc.meta.is_template && doc.meta.body_slot == Some(p.id) && p.text.is_empty() {
                    let hint = Paragraph {
                        text: "The document’s text goes here".into(),
                        runs: vec![xenify_wp_doc::Run {
                            len: "The document’s text goes here".len(),
                            fmt: Default::default(),
                        }],
                        ..p.clone()
                    };
                    let g = ghost(para::layout_paragraph(&mut engine.fonts, doc, &hint, width, None));
                    let mut shown = para::layout_paragraph(&mut engine.fonts, doc, p, width, label.as_deref());
                    shown.screen.extend(g.screen);
                    pl = Arc::new(shown);
                }
                out.push(Unit::Para(pl, Some((p, label))))
            }
            Block::Table(t) => out.push(Unit::Table(Arc::new(blocks::layout_table(engine, doc, t, width, labels)))),
            Block::Image(img) => {
                let indent = after_item.unwrap_or(0.0);
                let (w, h) = blocks::image_size(img, width - indent);
                out.push(Unit::Image {
                    id: img.id,
                    src: img.src.clone(),
                    alt: img.alt.clone(),
                    w,
                    h,
                    align: img.align,
                    border: img.border,
                    indent,
                });
                // A list continues past a picture inside it.
                list_indent = after_item;
            }
            Block::Toc(toc) if toc.kind == xenify_wp_doc::TocKind::Bibliography => {
                let entries = xenify_wp_doc::cite::bibliography(doc);
                let sample = entries.is_empty();
                let entries = if sample { sample_sources() } else { entries };
                let hang = if doc.meta.cite_style == xenify_wp_doc::cite::CiteStyle::Ieee { 26.0 } else { 36.0 };
                for runs in entries {
                    let mut p = Paragraph::new(NodeId(0), xenify_wp_doc::StyleSheet::NORMAL, "");
                    for (text, italic) in runs {
                        let fmt = xenify_wp_doc::CharFmt { italic: italic.then_some(true), ..Default::default() };
                        let s = xenify_wp_doc::Span::plain(text, fmt);
                        p.text.push_str(&s.text);
                        p.runs.extend(s.runs);
                    }
                    p.props.indent_left = Some(hang);
                    p.props.first_line = Some(-hang);
                    p.props.space_after = Some(6.0);
                    let pl = para::layout_paragraph(&mut engine.fonts, doc, &p, width, None);
                    out.push(Unit::Para(Arc::new(if sample { ghost(pl) } else { pl }), None));
                }
            }
            Block::Toc(toc) if toc.kind == xenify_wp_doc::TocKind::ChangeLog => {
                if doc.meta.changelog.is_empty() {
                    let today = xenify_wp_doc::changelog::day(&xenify_wp_doc::review::now());
                    let rows = [["Date", "Name", "Change"], [today.as_str(), "Your name", "What changed, and why"]];
                    for (i, row) in rows.iter().enumerate() {
                        out.push(Unit::Para(
                            Arc::new(ghost_row(engine, doc, row, &[1.0, 1.0, 3.0], i == 0, width)),
                            None,
                        ));
                    }
                } else {
                    let mut t = xenify_wp_doc::changelog::table(doc);
                    // Its cells answer to the change log block (for its right-click menu).
                    t.id = toc.id;
                    out.push(Unit::Table(Arc::new(blocks::layout_table(engine, doc, &t, width, labels))));
                }
            }
            Block::Toc(toc) if toc.kind == xenify_wp_doc::TocKind::Figures => {
                let entries: Vec<(NodeId, String)> = doc
                    .paragraphs()
                    .filter(|p| p.style == xenify_wp_doc::StyleSheet::CAPTION && !p.text.trim().is_empty())
                    .map(|p| (p.id, p.text.clone()))
                    .collect();
                if entries.is_empty() {
                    for (i, text) in ["Figure 1: A picture's caption", "Figure 2: Another caption"].iter().enumerate() {
                        let page = toc.page_numbers.then_some(i as u32 + 1);
                        let pl = blocks::layout_toc_entry(engine, doc, NodeId(0), 1, text, page, width);
                        out.push(Unit::Para(Arc::new(ghost(pl)), None));
                    }
                }
                for (id, text) in entries {
                    let page = toc.page_numbers.then(|| toc_pages.get(&id).copied().unwrap_or(0));
                    out.push(Unit::Para(
                        Arc::new(blocks::layout_toc_entry(engine, doc, id, 1, &text, page, width)),
                        None,
                    ));
                }
            }
            Block::Toc(toc) => {
                let entries = blocks::toc_entries(doc, toc.id, toc.levels);
                if entries.is_empty() {
                    // Screen only: what will go here (made-up headings looked real).
                    let levels = toc.levels.clamp(1, 6);
                    let text = if levels == 1 {
                        "Your Heading 1 headings will be listed here".to_string()
                    } else {
                        format!("Your Heading 1 to {levels} headings will be listed here")
                    };
                    let pl = blocks::layout_toc_entry(engine, doc, NodeId(0), 1, &text, None, width);
                    out.push(Unit::Para(Arc::new(ghost(pl)), None));
                }
                for (id, level, text) in entries {
                    let page = toc.page_numbers.then(|| toc_pages.get(&id).copied().unwrap_or(0));
                    out.push(Unit::Para(
                        Arc::new(blocks::layout_toc_entry(engine, doc, id, level, &text, page, width)),
                        None,
                    ));
                }
            }
        }
        // Each entry knows its list (for the list's right-click menu).
        if let Block::Toc(toc) = b {
            for u in &mut out[start..] {
                if let Unit::Para(pl, _) = u
                    && let Some(pl) = Arc::get_mut(pl)
                {
                    pl.generated = Some(toc.id);
                }
            }
        }
    }
    out
}

/// Each page's footnotes at its bottom, under a short line.
fn place_footnotes(out: &mut DocLayout, notes: &Footnotes) {
    for (&pi, (height, paras)) in &notes.pages {
        let Some(page) = out.pages.get(pi) else { continue };
        let body = page.body;
        let mut y = body.y1 as f32 - height + NOTE_GAP;
        out.shapes.push(Shape {
            rect: Rect::new(
                body.x0,
                (y - NOTE_GAP / 2.0) as f64 - 0.3,
                body.x0 + body.width() / 3.0,
                (y - NOTE_GAP / 2.0) as f64 + 0.3,
            ),
            color: Rgba::rgb(0x80, 0x80, 0x80),
            front: true,
        });
        for pl in paras {
            let origin = Point::new(body.x0, (y - pl.lines[0].top.min(0.0)) as f64);
            out.push(Fragment { para: pl.clone(), origin, lines: 0..pl.lines.len() });
            y += pl.height + 2.0;
        }
    }
}

/// Endnotes, at the end of the document under a "Notes" heading.
fn place_endnotes(
    engine: &mut Engine,
    doc: &Document,
    pageless: bool,
    out: &mut DocLayout,
    notes: &mut Footnotes,
    c: &mut Cursor,
    (obstacles, hf_room): (&[Obstacle], &[(f32, f32)]),
) {
    let numbers = xenify_wp_doc::notes::numbers(doc);
    let mut ends: Vec<(NodeId, String)> = doc
        .sections
        .iter()
        .flat_map(|s| &s.notes)
        .filter(|n| n.kind == xenify_wp_doc::NoteKind::End)
        .filter_map(|n| numbers.get(&n.id).map(|l| (n.id, l.clone())))
        .collect();
    if ends.is_empty() || out.pages.is_empty() {
        return;
    }
    // In the order they're numbered.
    let order: Vec<NodeId> = doc.paragraphs().flat_map(xenify_wp_doc::notes::references).map(|(_, id)| id).collect();
    ends.sort_by_key(|(id, _)| order.iter().position(|o| o == id));
    let last = doc.sections.len() - 1;
    let width = if pageless { doc.sections[last].page.content_width() } else { doc.sections[last].page.column_width() };
    let mut units: Vec<Unit> = Vec::new();
    // A short rule above the endnotes (no heading: nothing the writer didn't write).
    let mut rule = Paragraph::new(NodeId(0), xenify_wp_doc::StyleSheet::NOTE, "");
    rule.props.space_before = Some(18.0);
    rule.props.space_after = Some(4.0);
    rule.props.indent_right = Some(width * 2.0 / 3.0);
    rule.props.borders = Some(xenify_wp_doc::ParaBorders {
        top: Some(xenify_wp_doc::Border { width: 0.6, color: Rgba::rgb(0x80, 0x80, 0x80) }),
        ..Default::default()
    });
    units.push(Unit::Para(Arc::new(para::layout_paragraph(&mut engine.fonts, doc, &rule, width, None)), None));
    for (id, label) in ends {
        let Some(n) = doc.note(id) else { continue };
        for (i, b) in n.blocks.iter().enumerate() {
            let Block::Paragraph(p) = b else { continue };
            let mut q = p.clone();
            if i == 0 && q.props.indent_left.is_none() {
                q.props.indent_left = Some(18.0);
            }
            let pl = para::layout_paragraph(&mut engine.fonts, doc, &q, width, (i == 0).then_some(label.as_str()));
            units.push(Unit::Para(Arc::new(pl), None));
        }
    }
    let mut f = Flow { doc, out, fonts: &mut engine.fonts, obstacles, pageless, placed: Vec::new(), notes, hf_room };
    c.fresh = false;
    flow_section(&mut f, last, &units, (0, 0), c, None);
}

/// A faded example of what an empty generated list will look like once
/// it fills: drawn on screen only, never printed or exported.
fn ghost(mut pl: ParaLayout) -> ParaLayout {
    let faded = Rgba::rgb(0xb4, 0xb4, 0xb4);
    for it in &mut pl.items {
        match it {
            crate::Item::Glyphs { color, .. } | crate::Item::Rect { color, .. } => *color = faded,
            crate::Item::Emoji { .. } => {}
        }
    }
    pl.link = None;
    pl.screen = std::mem::take(&mut pl.items);
    pl
}

/// One row of a faded example table (columns `widths` shares of `width`,
/// `bold` for the header row), as one screen-only paragraph.
fn ghost_row(
    engine: &mut Engine,
    doc: &Document,
    cells: &[&str],
    widths: &[f32],
    bold: bool,
    width: f32,
) -> ParaLayout {
    let p = Paragraph::new(NodeId(0), xenify_wp_doc::StyleSheet::NORMAL, "");
    let mut pl = para::layout_paragraph(&mut engine.fonts, doc, &p, width, None);
    let Some(line) = pl.lines.first().copied() else { return pl };
    let mut style = doc.resolve_para(&p).char;
    style.bold = bold;
    // Room above and below the text, as in a table cell.
    let pad = 4.0;
    let (top, bottom) = (line.top - pad, line.bottom + pad);
    pl.height += 2.0 * pad;
    let baseline = line.baseline + pad;
    let total: f32 = widths.iter().sum();
    let rule = |x0: f32, y0: f32, x1: f32, y1: f32| crate::Item::Rect {
        rect: Rect::new(x0 as f64, (y0 + pad) as f64, x1 as f64, (y1 + pad) as f64),
        color: Rgba::rgb(0, 0, 0),
        line: 0,
    };
    let mut x = 0.0;
    for (text, w) in cells.iter().zip(widths) {
        let (items, _) = para::layout_label(&mut engine.fonts, text, &style);
        for mut it in items {
            if let crate::Item::Glyphs { glyphs, .. } = &mut it {
                for g in glyphs.iter_mut() {
                    *g = para::Glyph { id: g.id, x: g.x + x + 5.0, y: g.y + baseline };
                }
            }
            pl.items.push(it);
        }
        pl.items.push(rule(x, top, x + 0.6, bottom));
        x += width * w / total;
    }
    pl.items.push(rule(width - 0.6, top, width, bottom));
    pl.items.push(rule(0.0, top, width, top + 0.6));
    pl.items.push(rule(0.0, bottom - 0.6, width, bottom));
    ghost(pl)
}

/// Made-up sources for the faded example of an empty bibliography.
fn sample_sources() -> Vec<Vec<(String, bool)>> {
    vec![
        vec![("Author, A. (2024). ".into(), false), ("Title of a book".into(), true), (". Publisher.".into(), false)],
        vec![
            ("Writer, B., & Other, C. (2023). An article. ".into(), false),
            ("Journal Name".into(), true),
            (", 12(3), 45–67.".into(), false),
        ],
    ]
}

/// Lays out `doc` on pages (or one tall page when `pageless`).
pub(crate) fn paginate(engine: &mut Engine, doc: &Document, pageless: bool) -> DocLayout {
    let has_toc = doc
        .sections
        .iter()
        .any(|s| s.blocks.iter().any(|b| matches!(b, Block::Toc(t) if t.kind != xenify_wp_doc::TocKind::Bibliography)));
    let has_page_fields = !pageless
        && doc.paragraphs().any(|p| {
            p.runs.iter().any(|r| matches!(r.fmt.field, Some(xenify_wp_doc::Field::Page | xenify_wp_doc::Field::Pages)))
        });
    let mut toc_pages = engine.toc_pages.clone();
    let mut total = engine.page_count;
    para::set_page_context(&toc_pages, total);
    let mut out = paginate_once(engine, doc, pageless, &toc_pages);
    // Contents page numbers (and page fields) depend on the layout: repeat until they settle.
    if has_toc || has_page_fields {
        for _ in 0..3 {
            let pages = heading_pages(doc, &out);
            let n = out.pages.len() as u32;
            if pages == toc_pages && n == total {
                break;
            }
            toc_pages = pages;
            total = n;
            para::set_page_context(&toc_pages, total);
            out = paginate_once(engine, doc, pageless, &toc_pages);
        }
        engine.toc_pages = toc_pages;
        engine.page_count = total;
    }
    para::set_page_context(&HashMap::new(), 0);
    out
}

/// The page number each heading is on.
pub(crate) fn heading_pages(doc: &Document, out: &DocLayout) -> HashMap<NodeId, u32> {
    let mut m = HashMap::new();
    for p in doc.paragraphs() {
        // Headings (contents), captions (list of figures), and paragraphs with page fields.
        let page_field = p.runs.iter().any(|r| matches!(r.fmt.field, Some(xenify_wp_doc::Field::Page)));
        if doc.styles.resolve(&p.style).outline_level.is_none()
            && p.style != xenify_wp_doc::StyleSheet::CAPTION
            && !page_field
        {
            continue;
        }
        if let Some(&i) = out.by_para.get(&p.id).and_then(|v| v.first()) {
            let y = out.fragments[i].rect().y0;
            if let Some(pg) = out.pages.iter().find(|pg| y >= pg.rect.y0 && y <= pg.rect.y1) {
                m.insert(p.id, pg.number);
            }
        }
    }
    m
}

/// A picture's border, drawn just outside it.
fn image_border(shapes: &mut Vec<Shape>, r: Rect, border: Option<xenify_wp_doc::Border>) {
    let Some(b) = border.filter(|b| b.width > 0.0) else { return };
    let w = b.width as f64;
    for edge in [
        Rect::new(r.x0 - w, r.y0 - w, r.x1 + w, r.y0),
        Rect::new(r.x0 - w, r.y1, r.x1 + w, r.y1 + w),
        Rect::new(r.x0 - w, r.y0, r.x0, r.y1),
        Rect::new(r.x1, r.y0, r.x1 + w, r.y1),
    ] {
        shapes.push(Shape { rect: edge, color: b.color, front: true });
    }
}

/// The line between columns ("line between" in the page setup): on each
/// page, from the top to the bottom of the section's text there.
fn column_lines(f: &mut Flow, section: usize, frag_start: usize) {
    let ps = &f.doc.sections[section].page;
    let (cols, gap, col_w) = (f.columns(section), ps.column_gap, f.column_width(section));
    let pages: Vec<(usize, Rect)> = f.out.pages.iter().map(|p| p.rect).enumerate().collect();
    for (pi, page) in pages {
        let mut span: Option<(f64, f64)> = None;
        for fr in &f.out.fragments[frag_start..] {
            let r = fr.rect();
            if r.y0 >= page.y0 && r.y1 <= page.y1 {
                span = Some(span.map_or((r.y0, r.y1), |(a, b)| (a.min(r.y0), b.max(r.y1))));
            }
        }
        let Some((top, bottom)) = span else { continue };
        let left = f.out.pages[pi].body.x0 as f32;
        for k in 1..cols {
            let x = (left + k as f32 * (col_w + gap) - gap / 2.0) as f64;
            let rect = Rect::new(x - 0.3, top, x + 0.3, bottom);
            f.out.shapes.push(Shape { rect, color: Rgba::rgb(0x80, 0x80, 0x80), front: true });
        }
    }
}

fn paginate_once(engine: &mut Engine, doc: &Document, pageless: bool, toc_pages: &HashMap<NodeId, u32>) -> DocLayout {
    let mut out = DocLayout::default();
    let labels = xenify_wp_doc::lists::labels(doc);
    let mut c = Cursor { page: 0, col: 0, y: 0.0, area_top: 0.0, fresh: true };
    let mut obstacles: Vec<Obstacle> = Vec::new();
    let mut notes = Footnotes::default();
    let hf_room = header_room(engine, doc);
    for (si, section) in doc.sections.iter().enumerate() {
        let ps = &section.page;
        let width = if pageless { ps.content_width() } else { ps.column_width() };
        let units = units_of(engine, doc, &section.blocks, width, &labels, toc_pages);
        // Where this section starts: a new page, or the current one.
        let starts_new = si == 0
            || (!pageless
                && (section.start == SectionStart::NewPage
                    || !out.pages.last().is_some_and(|p| {
                        (p.rect.width() as f32 - ps.width).abs() < 0.5
                            && (p.rect.height() as f32 - ps.height).abs() < 0.5
                    })));
        let start_page = if starts_new { out.pages.len() } else { out.pages.len() - 1 };
        for fr in &section.frames {
            let page = start_page + if pageless { 0 } else { fr.page as usize };
            let g = fr.gap as f64;
            let rect = Rect::new(
                fr.x as f64 - g,
                fr.y as f64 - g,
                (fr.x + fr.width) as f64 + g,
                (fr.y + fr.height) as f64 + g,
            );
            obstacles.push(Obstacle { page, rect, wrap: fr.wrap });
        }
        let mut f = Flow {
            doc,
            out: &mut out,
            fonts: &mut engine.fonts,
            obstacles: &obstacles,
            pageless,
            placed: Vec::new(),
            notes: &mut notes,
            hf_room: &hf_room,
        };
        let same_size = f.out.pages.last().is_some_and(|p| (p.rect.width() as f32 - ps.width).abs() < 0.5)
            && (pageless || f.out.pages.last().is_some_and(|p| (p.rect.height() as f32 - ps.height).abs() < 0.5));
        let new_page = si == 0 || (!pageless && (section.start == SectionStart::NewPage || !same_size));
        if new_page {
            f.new_page_cursor(si, &mut c);
            f.out.pages[c.page].first_of_section = true;
        } else {
            c.col = 0;
            c.area_top = c.y + if c.fresh { 0.0 } else { 6.0 };
            c.y = c.area_top;
            c.fresh = true;
        }
        let frag_start = f.out.fragments.len();
        flow_section(&mut f, si, &units, (0, 0), &mut c, None);

        let columns = f.columns(si);
        let next_continuous = doc.sections.get(si + 1).is_some_and(|n| n.start == SectionStart::Continuous);
        if columns > 1 && next_continuous {
            balance(&mut f, si, &units, &mut c, frag_start);
        }
        if columns > 1 {
            // The next section starts below the longest column.
            let page = f.out.pages[c.page].rect;
            let bottom = f.out.fragments[frag_start..]
                .iter()
                .map(Fragment::rect)
                .chain(f.out.images.iter().map(|i| i.rect))
                .filter(|r| r.y0 >= page.y0 && r.y1 <= page.y1)
                .map(|r| r.y1 as f32)
                .fold(c.area_top, f32::max);
            c.y = bottom;
        }
        if columns > 1 && ps.column_line {
            column_lines(&mut f, si, frag_start);
        }
        drop(f);
        place_frames(engine, doc, si, start_page, &labels, toc_pages, &mut out);
    }
    place_endnotes(engine, doc, pageless, &mut out, &mut notes, &mut c, (&obstacles, &hf_room));
    if !pageless {
        align_pages_vertically(doc, &mut out);
    }
    place_footnotes(&mut out, &notes);
    if pageless && let Some(p) = out.pages.last_mut() {
        // Trim the one tall page to its content.
        let ps = &doc.sections[p.section].page;
        let bottom = out
            .fragments
            .iter()
            .map(|f| f.rect().y1)
            .chain(out.images.iter().map(|i| i.rect.y1))
            .chain(out.shapes.iter().map(|s| s.rect.y1))
            .chain(out.frames.iter().map(|f| f.rect.y1))
            .fold(p.body.y0, f64::max);
        p.body.y1 = bottom;
        p.rect.y1 = (bottom + ps.margins.bottom as f64).max(p.rect.y0 + ps.height as f64);
    }
    out.page_color = doc.meta.page_color.filter(|c| c.0[3] > 0);
    number_pages(doc, &mut out);
    if !pageless {
        headers_and_footers(engine, doc, &mut out);
    }
    out.width = out.pages.iter().map(|p| p.rect.width() as f32).fold(0.0, f32::max);
    out.height = out.pages.last().map_or(0.0, |p| p.rect.y1 as f32);
    out
}

/// Sections set to centre (or bottom-align) their text on the page: what's
/// on each of their pages moves down into the middle (or to the bottom) of
/// the space between the margins. A title page, centred.
fn align_pages_vertically(doc: &Document, out: &mut DocLayout) {
    for page in out.pages.clone() {
        let Some(ps) = doc.sections.get(page.section).map(|s| &s.page) else { continue };
        if ps.v_align == xenify_wp_doc::PageVAlign::Top {
            continue;
        }
        let body = page.body;
        let on = |r: Rect| body.contains(r.center());
        let bottom = out
            .fragments
            .iter()
            .map(Fragment::rect)
            .chain(out.images.iter().map(|i| i.rect))
            .chain(out.shapes.iter().map(|s| s.rect))
            .filter(|r| on(*r))
            .map(|r| r.y1)
            .fold(f64::NAN, f64::max);
        if bottom.is_nan() {
            continue;
        }
        let free = (body.y1 - bottom).max(0.0);
        let dy = if ps.v_align == xenify_wp_doc::PageVAlign::Center { free / 2.0 } else { free };
        if dy < 0.01 {
            continue;
        }
        for f in &mut out.fragments {
            if on(f.rect()) {
                f.origin.y += dy;
            }
        }
        for i in &mut out.images {
            if on(i.rect) {
                i.rect = i.rect + kurbo::Vec2::new(0.0, dy);
            }
        }
        for s in &mut out.shapes {
            if on(s.rect) {
                s.rect = s.rect + kurbo::Vec2::new(0.0, dy);
            }
        }
        for c in &mut out.cells {
            if on(c.rect) {
                c.rect = c.rect + kurbo::Vec2::new(0.0, dy);
            }
        }
    }
}

/// Re-flows what's on the section's last page into columns of equal height.
fn balance(f: &mut Flow, si: usize, units: &[Unit], c: &mut Cursor, frag_start: usize) {
    let last_page = c.page;
    let Some(&(ui, li, _)) = f.placed.iter().find(|p| p.2 == last_page) else { return };
    let page_top = f.out.pages[last_page].body.y0 as f32;
    // Where this section's columns begin on that page.
    let area_top = if f.placed.first().is_some_and(|p| p.2 == last_page) { c.area_top } else { page_top };
    let full_bottom = f.out.pages[last_page].body.y1 as f32;
    let (mut lo, mut hi) = (area_top, full_bottom);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        let mut trial = DocLayout { pages: f.out.pages[..=last_page].to_vec(), ..Default::default() };
        let mut tc = Cursor { page: last_page, col: 0, y: area_top, area_top, fresh: true };
        let mut scratch = Footnotes::default();
        let mut tf = Flow {
            doc: f.doc,
            out: &mut trial,
            fonts: &mut *f.fonts,
            obstacles: f.obstacles,
            pageless: false,
            placed: Vec::new(),
            notes: &mut scratch,
            hf_room: f.hf_room,
        };
        flow_section(&mut tf, si, units, (ui, li), &mut tc, Some(mid));
        if trial.pages.len() == last_page + 1 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    // Drop what this section placed on that page and place it again, balanced.
    let page = f.out.pages[last_page].rect;
    let on_page = |r: Rect| r.y0 >= page.y0 && r.y1 <= page.y1 && r.y0 >= area_top as f64 - 0.5;
    let kept: Vec<Fragment> = f
        .out
        .fragments
        .drain(..)
        .enumerate()
        .filter(|(i, fr)| *i < frag_start || !on_page(fr.rect()))
        .map(|(_, fr)| fr)
        .collect();
    f.out.by_para.clear();
    for fr in kept {
        f.out.push(fr);
    }
    f.out.images.retain(|i| !on_page(i.rect));
    f.out.shapes.retain(|s| !on_page(s.rect));
    f.out.decor.retain(|d| !on_page(d.rect()));
    f.out.pages.truncate(last_page + 1);
    *c = Cursor { page: last_page, col: 0, y: area_top, area_top, fresh: true };
    flow_section(f, si, units, (ui, li), c, Some(hi + 0.01));
}

fn number_pages(doc: &Document, out: &mut DocLayout) {
    let mut n = 0u32;
    for p in &mut out.pages {
        let s = &doc.sections[p.section].page;
        n = match (p.first_of_section, s.start_number) {
            (true, Some(start)) => start,
            _ => n + 1,
        };
        p.number = n;
    }
}

/// Fills in `{page}`, `{pages}`, `{section-pages}`, `{date}`, `{title}`, `{author}`.
pub fn fill_fields(template: &str, doc: &Document, page: &str, pages: usize, section_pages: usize) -> String {
    template
        .replace("{page}", page)
        .replace("{pages}", &pages.to_string())
        .replace("{section-pages}", &section_pages.to_string())
        .replace("{date}", &today())
        .replace("{title}", &doc.meta.title)
        .replace("{author}", &doc.meta.author)
}

/// Today's date as YYYY-MM-DD (UTC; no time-zone database needed).
/// The local date and time now.
pub fn now() -> xenify_wp_doc::datetime::DateParts {
    xenify_wp_doc::datetime::now()
}

pub(crate) fn today() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    #[cfg(target_arch = "wasm32")]
    let secs = 0u64;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn headers_and_footers(engine: &mut Engine, doc: &Document, out: &mut DocLayout) {
    let total = out.pages.len();
    for i in 0..out.pages.len() {
        let page = out.pages[i].clone();
        let own = &doc.sections[page.section].page;
        // Linked to the previous section: its headers and footers.
        let mut si = page.section;
        while si > 0 && doc.sections[si].page.link_to_previous {
            si -= 1;
        }
        let ps = &doc.sections[si].page;
        let first = page.first_of_section && ps.different_first;
        let even = ps.different_odd_even && page.number.is_multiple_of(2);
        let section_pages = out.pages.iter().filter(|p| p.section == page.section).count();
        let (header, footer) = if first {
            (&ps.first_header, &ps.first_footer)
        } else if even {
            (&ps.even_header, &ps.even_footer)
        } else {
            (&ps.header, &ps.footer)
        };
        let number = own.number_format.format(page.number);
        // Where a double-click edits the header and footer.
        out.hf_areas.push((i, false, Rect::new(page.body.x0, page.rect.y0, page.body.x1, page.body.y0)));
        out.hf_areas.push((i, true, Rect::new(page.body.x0, page.body.y1, page.body.x1, page.rect.y1)));
        let slot = if first {
            xenify_wp_doc::HfSlot::First
        } else if even {
            xenify_wp_doc::HfSlot::Even
        } else {
            xenify_wp_doc::HfSlot::Default
        };
        for (line, style, top) in [(header, StyleSheet::HEADER, true), (footer, StyleSheet::FOOTER, false)] {
            // A rich header or footer for these pages replaces the simple line.
            if let Some(part) = doc.sections[si].headers.iter().find(|h| h.footer != top && h.slot == slot) {
                para::set_current_page(Some((number.clone(), total as u32)));
                rich_header(engine, doc, out, i, part, top);
                para::set_current_page(None);
                continue;
            }
            if line.is_empty() {
                continue;
            }
            let HfLine { left, center, right } = line;
            for (text, align) in [
                (left, xenify_wp_doc::Align::Left),
                (center, xenify_wp_doc::Align::Center),
                (right, xenify_wp_doc::Align::Right),
            ] {
                if text.trim().is_empty() {
                    continue;
                }
                let filled = fill_fields(text, doc, &number, total, section_pages);
                let pl = engine.decor(doc, &filled, style, align, page.body.width() as f32);
                let h = pl.height as f64;
                let y = if top {
                    page.rect.y0 + (page.body.y0 - page.rect.y0 - h) / 2.0
                } else {
                    page.body.y1 + (page.rect.y1 - page.body.y1 - h) / 2.0
                };
                let lines = 0..pl.lines.len();
                out.decor.push(Fragment { para: pl, origin: Point::new(page.body.x0, y), lines });
            }
        }
    }
}

/// Per section: the top and bottom margins the text needs so the section's
/// rich headers and footers fit (half the margin, the header, a little gap).
fn header_room(engine: &mut Engine, doc: &Document) -> Vec<(f32, f32)> {
    let labels = xenify_wp_doc::lists::labels(doc);
    let mut out = Vec::with_capacity(doc.sections.len());
    for si in 0..doc.sections.len() {
        let mut sj = si;
        while sj > 0 && doc.sections[sj].page.link_to_previous {
            sj -= 1;
        }
        let ps = &doc.sections[si].page;
        let width = ps.content_width();
        let (mut top, mut bottom) = (0.0f32, 0.0f32);
        for part in &doc.sections[sj].headers {
            let units = units_of(engine, doc, &part.blocks, width, &labels, &HashMap::new());
            let mut scratch = DocLayout::default();
            place_in_box(&mut scratch, &units, (0, 0), Rect::new(0.0, 0.0, width as f64, 100_000.0));
            let h = scratch
                .fragments
                .iter()
                .map(|f| f.rect().y1)
                .chain(scratch.images.iter().map(|i| i.rect.y1))
                .fold(0.0f64, f64::max) as f32;
            if part.footer {
                bottom = bottom.max(ps.margins.bottom / 2.0 + h + 8.0);
            } else {
                top = top.max(ps.margins.top / 2.0 + h + 8.0);
            }
        }
        out.push((top, bottom));
    }
    out
}

/// A rich header (`top`) or footer laid out in the page's margin. On the
/// page being edited its paragraphs are live (caret, selection); elsewhere
/// they're drawn only.
fn rich_header(
    engine: &mut Engine,
    doc: &Document,
    out: &mut DocLayout,
    page_index: usize,
    part: &xenify_wp_doc::HeaderPart,
    top: bool,
) {
    let page = out.pages[page_index].clone();
    let ps = &doc.sections[page.section].page;
    let labels = xenify_wp_doc::lists::labels(doc);
    let width = page.body.width() as f32;
    let units = units_of(engine, doc, &part.blocks, width, &labels, &HashMap::new());
    let mut scratch = DocLayout { pages: vec![page.clone()], ..Default::default() };
    place_in_box(&mut scratch, &units, (0, 0), Rect::new(page.body.x0, 0.0, page.body.x1, 100_000.0));
    let height = scratch
        .fragments
        .iter()
        .map(|f| f.rect().y1)
        .chain(scratch.images.iter().map(|i| i.rect.y1))
        .chain(scratch.shapes.iter().map(|s| s.rect.y1))
        .fold(0.0f64, f64::max);
    // Header: half the top margin from the top edge; footer: likewise from the bottom.
    let dy = if top {
        page.rect.y0 + (ps.margins.top / 2.0) as f64
    } else {
        page.rect.y1 - (ps.margins.bottom / 2.0) as f64 - height
    };
    let live = engine.header_edit == Some(page_index);
    for mut f in scratch.fragments {
        f.origin.y += dy;
        if live {
            out.push(f);
        } else {
            out.decor.push(f);
        }
    }
    for mut img in scratch.images {
        img.rect = img.rect + kurbo::Vec2::new(0.0, dy);
        out.images.push(img);
    }
    for mut s in scratch.shapes {
        s.rect = s.rect + kurbo::Vec2::new(0.0, dy);
        out.shapes.push(s);
    }
    if live {
        out.header_rects.push((page_index, part.id, Rect::new(page.body.x0, dy, page.body.x1, dy + height)));
    }
}

pub(crate) fn layout_decor(
    fonts: &mut crate::Fonts,
    doc: &Document,
    text: &str,
    style: &str,
    align: xenify_wp_doc::Align,
    width: f32,
) -> ParaLayout {
    let mut p = Paragraph::new(NodeId(0), style, text);
    p.props.align = Some(align);
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    para::layout_paragraph(fonts, doc, &p, width, None)
}

/// Places `units` from piece `start` in the box `inner` (a frame's inside),
/// top to bottom; returns where it stopped (all placed: `units.len()`).
fn place_in_box(out: &mut DocLayout, units: &[Unit], start: (usize, usize), inner: Rect) -> (usize, usize) {
    let (mut ui, mut li) = start;
    let (x, bottom, width) = (inner.x0 as f32, inner.y1 as f32, inner.width() as f32);
    let mut y = inner.y0 as f32;
    let mut fresh = true;
    while ui < units.len() {
        let unit = &units[ui];
        if li == 0 && !fresh {
            y += gap(ui.checked_sub(1).map(|i| &units[i]), unit);
        }
        match unit {
            Unit::Para(pl, _) => {
                let mut k = lines_fitting(pl, li, y, bottom);
                if k == 0 && fresh {
                    // Too tall for the box: show its first line anyway.
                    k = 1;
                }
                if k == 0 {
                    return (ui, li);
                }
                let origin = Point::new(x as f64, (y - piece_top(pl, li)) as f64);
                out.push(Fragment { para: pl.clone(), origin, lines: li..li + k });
                y = origin.y as f32 + pl.lines[li + k - 1].bottom;
                li += k;
                if li < pl.lines.len() {
                    return (ui, li);
                }
            }
            Unit::Table(t) => {
                let groups = t.groups();
                while li < groups.len() {
                    let g = groups[li].clone();
                    let h: f32 = t.rows[g.clone()].iter().map(|r| r.height).sum();
                    if y + h > bottom + 0.01 && !fresh {
                        return (ui, li);
                    }
                    y = place_rows(out, t, g, x + t.x, y, false);
                    fresh = false;
                    li += 1;
                }
            }
            Unit::Image { id, src, alt, w, h, align, border, indent } => {
                if y + h > bottom + 0.01 && !fresh {
                    return (ui, 0);
                }
                let dx = indent
                    + match align {
                        xenify_wp_doc::Align::Center => (width - indent - w) / 2.0,
                        xenify_wp_doc::Align::Right => width - indent - w,
                        _ => 0.0,
                    };
                let rect = Rect::new((x + dx) as f64, y as f64, (x + dx + w) as f64, (y + h) as f64);
                out.images.push(PlacedImage { id: *id, src: src.clone(), alt: alt.clone(), rect });
                image_border(&mut out.shapes, rect, *border);
                y += h;
            }
        }
        fresh = false;
        ui += 1;
        li = 0;
    }
    (ui, 0)
}

/// Section `si`'s frames on its pages (from `start_page`), with their content;
/// threaded frames continue one story.
fn place_frames(
    engine: &mut Engine,
    doc: &Document,
    si: usize,
    start_page: usize,
    labels: &HashMap<NodeId, String>,
    toc_pages: &HashMap<NodeId, u32>,
    out: &mut DocLayout,
) {
    let section = &doc.sections[si];
    if section.frames.is_empty() || out.pages.is_empty() {
        return;
    }
    let last_page = out.pages.len() - 1;
    let outer = |fr: &Frame| {
        let page = (start_page + fr.page as usize).min(last_page);
        let top = out.pages[page].rect.y0;
        (page, Rect::new(fr.x as f64, top + fr.y as f64, (fr.x + fr.width) as f64, top + (fr.y + fr.height) as f64))
    };
    let placed: Vec<(usize, Rect)> = section.frames.iter().map(outer).collect();
    for (fr, (page, rect)) in section.frames.iter().zip(&placed) {
        if let Some(fill) = fr.fill {
            out.shapes.push(Shape { rect: *rect, color: fill, front: false });
        }
        if let Some(b) = fr.border.filter(|b| b.width > 0.0) {
            let w = b.width as f64;
            for edge in [
                Rect::new(rect.x0, rect.y0 - w / 2.0, rect.x1, rect.y0 + w / 2.0),
                Rect::new(rect.x0, rect.y1 - w / 2.0, rect.x1, rect.y1 + w / 2.0),
                Rect::new(rect.x0 - w / 2.0, rect.y0, rect.x0 + w / 2.0, rect.y1),
                Rect::new(rect.x1 - w / 2.0, rect.y0, rect.x1 + w / 2.0, rect.y1),
            ] {
                out.shapes.push(Shape { rect: edge, color: b.color, front: true });
            }
        }
        out.frames.push(PlacedFrame {
            id: fr.id,
            section: si,
            page: *page,
            rect: *rect,
            overflow: false,
            threaded_in: false,
        });
    }
    // Stories: each chain head's blocks flow through the chain.
    let is_target: std::collections::HashSet<NodeId> = section.frames.iter().filter_map(|f| f.next).collect();
    let first = out.frames.len() - section.frames.len();
    for (i, head) in section.frames.iter().enumerate() {
        if is_target.contains(&head.id) {
            continue;
        }
        // The chain, stopping at loops.
        let mut chain = vec![i];
        let mut cur = head;
        while let Some(n) = cur.next {
            match section.frames.iter().position(|f| f.id == n) {
                Some(j) if !chain.contains(&j) => {
                    chain.push(j);
                    cur = &section.frames[j];
                }
                _ => break,
            }
        }
        let pad = head.padding.max(0.0) as f64;
        let inner_w = (head.width as f64 - 2.0 * pad).max(12.0) as f32;
        let units = units_of(engine, doc, &head.blocks, inner_w, labels, toc_pages);
        let mut at = (0, 0);
        for (k, &j) in chain.iter().enumerate() {
            let fr = &section.frames[j];
            let pad = fr.padding.max(0.0) as f64;
            let rect = placed[j].1;
            let inner = Rect::new(rect.x0 + pad, rect.y0 + pad, rect.x1 - pad, rect.y1 - pad);
            if k > 0 {
                out.frames[first + j].threaded_in = true;
            }
            if at.0 < units.len() {
                at = place_in_box(out, &units, at, inner);
            }
        }
        if at.0 < units.len() {
            out.frames[first + *chain.last().expect("non-empty")].overflow = true;
        }
    }
}
