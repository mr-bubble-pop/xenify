//! Table, image and table-of-contents editing: insert, rows and columns,
//! merge and split, header rows, borders, shading, widths.

use crate::{
    Block, Border, Container, EditError, Editor, Image, NodeId, Paragraph, Pos, Rgba, Selection, Step, StyleSheet,
    Table, Toc,
};

/// The table cell the caret paragraph is in: (table, row, col).
pub fn cell_at(ed: &Editor, para: NodeId) -> Option<(NodeId, usize, usize)> {
    match ed.doc.locate(para)?.0 {
        Container::Cell { table, row, col } => Some((table, row, col)),
        Container::Section(_) | Container::Frame(_) | Container::Note(_) | Container::Header(_) => None,
    }
}

/// The cells the selection covers in one table: (table, rows, cols), as
/// grid ranges. A collapsed selection gives the caret's cell.
pub fn selected_cells(
    ed: &Editor,
) -> Option<(NodeId, std::ops::RangeInclusive<usize>, std::ops::RangeInclusive<usize>)> {
    let (ta, ra, ca) = cell_at(ed, ed.sel.anchor.para)?;
    let (tb, rb, cb) = cell_at(ed, ed.sel.focus.para).filter(|c| c.0 == ta).unwrap_or((ta, ra, ca));
    let _ = tb;
    Some((ta, ra.min(rb)..=ra.max(rb), ca.min(cb)..=ca.max(cb)))
}

/// The first paragraph of cell (row, col) of `table`.
pub fn first_para(t: &Table, row: usize, col: usize) -> Option<NodeId> {
    t.rows.get(row)?.cells.get(col)?.blocks.iter().find_map(|b| match b {
        Block::Paragraph(p) => Some(p.id),
        _ => None,
    })
}

/// Runs `f` on a copy of table `id` and puts the result back as one undoable step.
fn edit_table(
    ed: &mut Editor,
    id: NodeId,
    label: &str,
    f: impl FnOnce(&mut Table, &mut dyn FnMut() -> NodeId) -> Option<Selection>,
) -> Result<(), EditError> {
    let (container, index) = ed.doc.locate(id).ok_or(EditError::NotHere)?;
    let mut t = ed.doc.table(id).ok_or(EditError::NotHere)?.clone();
    let mut next = ed.doc.next_id;
    let mut alloc = || {
        let id = NodeId(next);
        next += 1;
        id
    };
    let after = f(&mut t, &mut alloc);
    t.normalize(&mut alloc);
    ed.doc.next_id = next;
    let sel = after.unwrap_or(ed.sel);
    ed.seal();
    ed.transact(label, None, |tx| {
        tx.step(Step::ReplaceBlock { container, index, block: Block::Table(t) })?;
        Ok(sel)
    })?;
    // The caret's paragraph may have gone (deleted row): put it somewhere valid.
    if ed.doc.para(ed.sel.focus.para).is_none() || ed.doc.para(ed.sel.anchor.para).is_none() {
        let p = ed.doc.table(id).and_then(|t| first_para(t, 0, 0)).unwrap_or_else(|| ed.doc.start().para);
        ed.sel = Selection::caret(Pos::new(p, 0));
    }
    Ok(())
}

/// Inserts `block` after the caret's paragraph (splitting it when the caret
/// is in the middle), making sure a paragraph follows so typing can go on.
fn insert_block(ed: &mut Editor, block: Block, label: &str) -> Result<(), EditError> {
    let at = ed.sel.focus;
    let len = ed.doc.para(at.para).map_or(0, |p| p.text.len());
    ed.seal();
    if at.offset > 0 && at.offset < len {
        crate::ops::split_paragraph(ed)?;
        // The new block goes between the halves.
        let first = ed.doc.prev_para(ed.sel.focus.para).unwrap_or(at.para);
        ed.sel = Selection::caret(Pos::new(first, at.offset));
    }
    let at = ed.sel.focus;
    // Inside a table cell the block goes into the cell (cells hold pictures and tables too).
    let empty = ed.doc.para(at.para).is_some_and(|p| p.text.is_empty());
    let (c, i) = ed.doc.locate(at.para).ok_or(EditError::NoParagraph(at.para))?;
    // In an empty paragraph the block goes before it (the paragraph stays as the one after).
    let index = if empty || at.offset == 0 && len > 0 { i } else { i + 1 };
    let needs_after = !empty && index == i + 1 && ed.doc.blocks(&c).is_some_and(|v| v.len() == i + 1);
    let after_id = ed.doc.alloc_id();
    let first_cell = match &block {
        Block::Table(t) => first_para(t, 0, 0),
        _ => None,
    };
    ed.transact(label, None, |tx| {
        tx.step(Step::InsertBlock { container: c.clone(), index, block })?;
        if needs_after {
            tx.step(Step::InsertBlock {
                container: c.clone(),
                index: index + 1,
                block: Block::Paragraph(Paragraph::new(after_id, StyleSheet::NORMAL, "")),
            })?;
        }
        Ok(Selection::caret(first_cell.map_or(at, |p| Pos::new(p, 0))))
    })
}

/// Insert › Table.
pub fn insert_table(ed: &mut Editor, rows: usize, cols: usize) -> Result<(), EditError> {
    let mut next = ed.doc.next_id;
    let t = Table::new(rows, cols, &mut || {
        let id = NodeId(next);
        next += 1;
        id
    });
    ed.doc.next_id = next;
    insert_block(ed, Block::Table(t), "Insert table")
}

/// Insert › Image (`src` is an asset key or a path).
pub fn insert_image(ed: &mut Editor, src: &str, width: f32, height: f32) -> Result<(), EditError> {
    let id = ed.doc.alloc_id();
    let img = Image { id, src: src.into(), width, height, ..Default::default() };
    insert_block(ed, Block::Image(img), "Insert image")
}

/// Insert › Table of contents.
pub fn insert_toc(ed: &mut Editor, levels: u8) -> Result<(), EditError> {
    let id = ed.doc.alloc_id();
    insert_block(
        ed,
        Block::Toc(Toc { id, levels, page_numbers: true, ..Default::default() }),
        "Insert table of contents",
    )
}

/// Replaces block `id` (an image or contents table) with `block` (its settings changed).
pub fn replace_block(ed: &mut Editor, id: NodeId, block: Block, label: &str) -> Result<(), EditError> {
    let (container, index) = ed.doc.locate(id).ok_or(EditError::NotHere)?;
    let sel = ed.sel;
    ed.seal();
    ed.transact(label, None, |tx| {
        tx.step(Step::ReplaceBlock { container, index, block })?;
        Ok(sel)
    })
}

/// Removes block `id` (a table, image or contents table).
pub fn delete_block(ed: &mut Editor, id: NodeId) -> Result<(), EditError> {
    let (container, index) = ed.doc.locate(id).ok_or(EditError::NotHere)?;
    // Somewhere for the caret afterwards: the paragraph after, else before.
    let after = crate::ops::neighbour_para(&ed.doc, id);
    ed.seal();
    ed.transact("Delete", None, |tx| {
        tx.step(Step::RemoveBlock { container: container.clone(), index })?;
        let blocks = tx.doc.blocks(&container).map_or(0, Vec::len);
        let pos = match after {
            Some(p) if tx.doc.para(p).is_some() => Pos::new(p, 0),
            _ if blocks == 0 || tx.doc.paragraphs().next().is_none() => {
                // Never leave a container (or the document) without a paragraph.
                let id = tx.doc.alloc_id();
                tx.step(Step::InsertBlock {
                    container: container.clone(),
                    index: index.min(blocks),
                    block: Block::Paragraph(Paragraph::new(id, StyleSheet::NORMAL, "")),
                })?;
                Pos::new(id, 0)
            }
            _ => tx.doc.start(),
        };
        Ok(Selection::caret(pos))
    })
}

pub fn delete_table(ed: &mut Editor) -> Result<(), EditError> {
    let (t, _, _) = cell_at(ed, ed.sel.focus.para).ok_or(EditError::NotHere)?;
    delete_block(ed, t)
}

/// Inserts a row above or below the selected rows.
pub fn insert_row(ed: &mut Editor, below: bool) -> Result<(), EditError> {
    let (id, rows, _) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Insert row", |t, alloc| {
        let at = if below { *rows.end() + 1 } else { *rows.start() };
        let cols = t.cols();
        let header = t.rows.get(at.min(t.rows.len().saturating_sub(1))).is_some_and(|r| r.header) && !below;
        let row = crate::Row {
            id: alloc(),
            header,
            footer: false,
            repeat: None,
            min_height: None,
            cells: (0..cols).map(|_| crate::Cell::new(alloc(), alloc())).collect(),
        };
        // Merged cells spanning the insertion point grow instead of splitting.
        for r in 0..at.min(t.rows.len()) {
            for c in 0..cols {
                let cell = &mut t.rows[r].cells[c];
                if !cell.covered && r + cell.row_span as usize > at {
                    cell.row_span += 1;
                }
            }
        }
        let first = row.cells.first().and_then(|c| c.blocks.first()).map(Block::id);
        t.rows.insert(at.min(t.rows.len()), row);
        first.map(|p| Selection::caret(Pos::new(p, 0)))
    })
}

/// Inserts a column left or right of the selected columns.
pub fn insert_column(ed: &mut Editor, right: bool) -> Result<(), EditError> {
    let (id, _, cols) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Insert column", |t, alloc| {
        let at = if right { *cols.end() + 1 } else { *cols.start() };
        let w = t.widths.get(at.min(t.cols() - 1)).copied().unwrap_or(1.0);
        t.widths.insert(at.min(t.cols()), w);
        for row in &mut t.rows {
            for (c, cell) in row.cells.iter_mut().enumerate().take(at) {
                if !cell.covered && c + cell.col_span as usize > at {
                    cell.col_span += 1;
                }
            }
            row.cells.insert(at.min(row.cells.len()), crate::Cell::new(alloc(), alloc()));
        }
        None
    })
}

/// Deletes the selected rows (the whole table when that's all of them).
pub fn delete_rows(ed: &mut Editor) -> Result<(), EditError> {
    let (id, rows, _) = selected_cells(ed).ok_or(EditError::NotHere)?;
    let n = ed.doc.table(id).map_or(0, |t| t.rows.len());
    if rows.clone().count() >= n {
        return delete_block(ed, id);
    }
    edit_table(ed, id, "Delete row", |t, _| {
        let (a, b) = (*rows.start(), *rows.end());
        // Merged cells starting above shrink; ones starting inside move their span down.
        for r in 0..t.rows.len() {
            for c in 0..t.cols() {
                let cell = &mut t.rows[r].cells[c];
                if cell.covered {
                    continue;
                }
                let (top, bottom) = (r, r + cell.row_span as usize - 1);
                let overlap = bottom.min(b) as isize - top.max(a) as isize + 1;
                if overlap > 0 && top < a {
                    cell.row_span -= overlap as u16;
                }
            }
        }
        t.rows.drain(a..=b);
        let r = a.min(t.rows.len() - 1);
        first_para(t, r, 0).map(|p| Selection::caret(Pos::new(p, 0)))
    })
}

/// Deletes the selected columns (the whole table when that's all of them).
pub fn delete_columns(ed: &mut Editor) -> Result<(), EditError> {
    let (id, _, cols) = selected_cells(ed).ok_or(EditError::NotHere)?;
    let n = ed.doc.table(id).map_or(0, Table::cols);
    if cols.clone().count() >= n {
        return delete_block(ed, id);
    }
    edit_table(ed, id, "Delete column", |t, _| {
        let (a, b) = (*cols.start(), *cols.end());
        for row in &mut t.rows {
            for (c, cell) in row.cells.iter_mut().enumerate() {
                if cell.covered || c >= a {
                    continue;
                }
                let right = c + cell.col_span as usize - 1;
                let overlap = right.min(b) as isize - a as isize + 1;
                if overlap > 0 {
                    cell.col_span -= overlap as u16;
                }
            }
            row.cells.drain(a..=b);
        }
        t.widths.drain(a..=b);
        first_para(t, 0, a.min(t.cols() - 1)).map(|p| Selection::caret(Pos::new(p, 0)))
    })
}

/// Merges the selected cells into one (their text is kept, one after another).
pub fn merge_cells(ed: &mut Editor) -> Result<(), EditError> {
    let (id, rows, cols) = selected_cells(ed).ok_or(EditError::NotHere)?;
    if rows.clone().count() * cols.clone().count() < 2 {
        return Ok(());
    }
    edit_table(ed, id, "Merge cells", |t, _| {
        let (r0, c0) = (*rows.start(), *cols.start());
        let mut moved = Vec::new();
        for r in rows.clone() {
            for c in cols.clone() {
                if (r, c) == (r0, c0) {
                    continue;
                }
                let cell = &mut t.rows[r].cells[c];
                // Keep text from the other cells (skip their empty paragraphs).
                for b in cell.blocks.drain(..) {
                    if !matches!(&b, Block::Paragraph(p) if p.text.is_empty()) {
                        moved.push(b);
                    }
                }
            }
        }
        let anchor = &mut t.rows[r0].cells[c0];
        anchor.col_span = cols.clone().count() as u16;
        anchor.row_span = rows.clone().count() as u16;
        anchor.blocks.extend(moved);
        first_para(t, r0, c0).map(|p| Selection::caret(Pos::new(p, 0)))
    })
}

/// Splits a merged cell back into single cells.
pub fn split_cell(ed: &mut Editor) -> Result<(), EditError> {
    let (id, r, c) = cell_at(ed, ed.sel.focus.para).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Split cell", |t, _| {
        let cell = &mut t.rows[r].cells[c];
        cell.col_span = 1;
        cell.row_span = 1;
        None
    })
}

/// Turns the selected rows into header rows (repeated on each page), or back.
pub fn toggle_header_rows(ed: &mut Editor) -> Result<(), EditError> {
    let (id, rows, _) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Header row", |t, _| {
        let on = !t.rows[*rows.start()].header;
        // Header rows must be at the top: this row and every one above it.
        for r in 0..=*rows.end() {
            if on || r >= *rows.start() {
                t.rows[r].header = on;
            }
        }
        None
    })
}

/// Background colour of the selected cells (`None` clears it).
pub fn set_shading(ed: &mut Editor, color: Option<Rgba>) -> Result<(), EditError> {
    let (id, rows, cols) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Cell shading", |t, _| {
        for r in rows.clone() {
            for c in cols.clone() {
                t.rows[r].cells[c].shading = color;
            }
        }
        None
    })
}

/// Table lines (`None`: no lines).
pub fn set_border(ed: &mut Editor, border: Option<Border>) -> Result<(), EditError> {
    let (id, _, _) = cell_at(ed, ed.sel.focus.para).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Table borders", |t, _| {
        t.border = border;
        None
    })
}

/// Column widths (relative) and the table's overall width share and alignment.
pub fn set_table_layout(
    ed: &mut Editor,
    id: NodeId,
    widths: Vec<f32>,
    width: f32,
    align: crate::Align,
    padding: f32,
) -> Result<(), EditError> {
    edit_table(ed, id, "Table layout", |t, _| {
        if widths.len() == t.widths.len() {
            t.widths = widths;
        }
        t.width = width.clamp(0.1, 1.0);
        t.align = align;
        t.padding = padding.clamp(0.0, 72.0);
        None
    })
}

/// Tab / Shift+Tab in a table: the next or previous cell (Tab in the last
/// cell adds a row). Returns false when the caret isn't in a table.
pub fn move_cell(ed: &mut Editor, forward: bool) -> Result<bool, EditError> {
    let Some((id, r, c)) = cell_at(ed, ed.sel.focus.para) else { return Ok(false) };
    let t = ed.doc.table(id).ok_or(EditError::NotHere)?;
    let cols = t.cols();
    let positions: Vec<(usize, usize)> = (0..t.rows.len())
        .flat_map(|r| (0..cols).map(move |c| (r, c)))
        .filter(|&(r, c)| !t.rows[r].cells[c].covered)
        .collect();
    let here = positions.iter().position(|&p| p == (r, c)).unwrap_or(0);
    let target = if forward { positions.get(here + 1) } else { here.checked_sub(1).and_then(|i| positions.get(i)) };
    match target {
        Some(&(r, c)) => {
            // Select the cell's text, like other word processors.
            let cell = &t.rows[r].cells[c];
            let paras: Vec<&Paragraph> = cell
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Paragraph(p) => Some(p),
                    _ => None,
                })
                .collect();
            if let (Some(first), Some(last)) = (paras.first(), paras.last()) {
                ed.sel = Selection { anchor: Pos::new(first.id, 0), focus: Pos::new(last.id, last.text.len()) };
            }
            ed.seal();
        }
        None if forward => {
            // Tab in the last cell adds a row.
            let last = positions.last().copied().unwrap_or((r, c));
            let p = first_para(t, last.0, last.1).unwrap_or(ed.sel.focus.para);
            ed.sel = Selection::caret(Pos::new(p, 0));
            insert_row(ed, true)?;
        }
        None => {}
    }
    Ok(true)
}

/// Mail merge: repeat the selected row once per item of list field `list`
/// (`None`: don't repeat).
pub fn set_row_repeat(ed: &mut Editor, list: Option<String>) -> Result<(), EditError> {
    let (id, rows, _) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Repeat row", |t, _| {
        for r in rows.clone() {
            t.rows[r].repeat = list.clone().filter(|l| !l.trim().is_empty());
        }
        None
    })
}

/// The last paragraph of cell (row, col).
fn last_para(t: &Table, row: usize, col: usize) -> Option<&Paragraph> {
    t.rows.get(row)?.cells.get(col)?.blocks.iter().rev().find_map(|b| match b {
        Block::Paragraph(p) => Some(p),
        _ => None,
    })
}

/// Selects cells (rows × cols) of table `id`: from the start of the first
/// to the end of the last.
pub fn select_cells(
    ed: &mut Editor,
    id: NodeId,
    rows: std::ops::RangeInclusive<usize>,
    cols: std::ops::RangeInclusive<usize>,
) -> bool {
    let Some(t) = ed.doc.table(id) else { return false };
    let (r1, c1) = (*rows.end().min(&(t.rows.len().saturating_sub(1))), *cols.end().min(&(t.cols().saturating_sub(1))));
    let (r1, c1) = t.anchor_of(r1, c1);
    let (Some(a), Some(b)) = (first_para(t, *rows.start(), *cols.start()), last_para(t, r1, c1)) else { return false };
    let end = b.text.len();
    let b = b.id;
    ed.seal();
    ed.sel = Selection { anchor: Pos::new(a, 0), focus: Pos::new(b, end) };
    true
}

/// Selects column `col` of table `id`.
pub fn select_column(ed: &mut Editor, id: NodeId, col: usize) -> bool {
    let rows = ed.doc.table(id).map_or(0, |t| t.rows.len());
    select_cells(ed, id, 0..=rows.saturating_sub(1), col..=col)
}

/// Selects row `row` of table `id`.
pub fn select_row(ed: &mut Editor, id: NodeId, row: usize) -> bool {
    let cols = ed.doc.table(id).map_or(0, |t| t.cols());
    select_cells(ed, id, row..=row, 0..=cols.saturating_sub(1))
}

/// Inserts a row before row `at` (`at` = the row count: at the end).
pub fn insert_row_at(ed: &mut Editor, id: NodeId, at: usize) -> Result<(), EditError> {
    let rows = ed.doc.table(id).map_or(0, |t| t.rows.len());
    let (row, below) = if at >= rows { (rows.saturating_sub(1), true) } else { (at, false) };
    let p = ed.doc.table(id).and_then(|t| first_para(t, row, 0)).ok_or(EditError::NotHere)?;
    ed.sel = Selection::caret(Pos::new(p, 0));
    insert_row(ed, below)
}

/// Inserts a column before column `at` (`at` = the column count: at the end).
pub fn insert_column_at(ed: &mut Editor, id: NodeId, at: usize) -> Result<(), EditError> {
    let cols = ed.doc.table(id).map_or(0, |t| t.cols());
    let (col, right) = if at >= cols { (cols.saturating_sub(1), true) } else { (at, false) };
    let p = ed.doc.table(id).and_then(|t| first_para(t, 0, col)).ok_or(EditError::NotHere)?;
    ed.sel = Selection::caret(Pos::new(p, 0));
    insert_column(ed, right)
}

/// Column widths of table `id` (relative); a drag's changes merge into one
/// undo step.
pub fn set_widths(ed: &mut Editor, id: NodeId, widths: Vec<f32>) -> Result<(), EditError> {
    let (container, index) = ed.doc.locate(id).ok_or(EditError::NotHere)?;
    let mut t = ed.doc.table(id).ok_or(EditError::NotHere)?.clone();
    if widths.len() != t.widths.len() || widths.iter().any(|w| !w.is_finite() || *w <= 0.0) {
        return Err(EditError::NotHere);
    }
    t.widths = widths;
    let sel = ed.sel;
    ed.transact("Column width", Some(crate::MergeKey::Control(format!("widths-{}", id.0))), |tx| {
        tx.step(Step::ReplaceBlock { container, index, block: Block::Table(t) })?;
        Ok(sel)
    })
}

/// Where content sits in the selected cells.
pub fn set_v_align(ed: &mut Editor, v: crate::VAlign) -> Result<(), EditError> {
    let (id, rows, cols) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Cell alignment", |t, _| {
        for r in rows.clone() {
            for c in cols.clone() {
                t.rows[r].cells[c].v_align = v;
            }
        }
        None
    })
}

/// Lines on the sides of the selected cells (`None`: the table's lines).
pub fn set_cell_borders(ed: &mut Editor, borders: Option<crate::ParaBorders>) -> Result<(), EditError> {
    let (id, rows, cols) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Cell borders", |t, _| {
        for r in rows.clone() {
            for c in cols.clone() {
                t.rows[r].cells[c].borders = borders;
            }
        }
        None
    })
}

/// Turns the header column on or off.
pub fn toggle_header_col(ed: &mut Editor) -> Result<(), EditError> {
    let (id, _, _) = cell_at(ed, ed.sel.focus.para).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Header column", |t, _| {
        t.header_col = !t.header_col;
        if t.header_col && t.header_fill.is_none() {
            t.header_fill = Some(Rgba::rgb(0xf1, 0xf3, 0xf5));
        }
        None
    })
}

/// The text of a cell: one paragraph saying `text` (in the style its first
/// paragraph had), replacing what was there.
fn set_cell_text(cell: &mut crate::Cell, text: &str, alloc: &mut dyn FnMut() -> NodeId) {
    let (id, style) = match cell.blocks.first() {
        Some(Block::Paragraph(p)) => (p.id, p.style.clone()),
        _ => (alloc(), crate::StyleSheet::NORMAL.to_string()),
    };
    cell.blocks = vec![Block::Paragraph(crate::Paragraph::new(id, &style, text))];
}

/// Adds rows to table `id` before row `at` (`None`: at the end, before any
/// footer rows), each cell saying what `rows` says. One undo step.
pub fn add_rows_with(ed: &mut Editor, id: NodeId, at: Option<usize>, rows: &[Vec<String>]) -> Result<(), EditError> {
    edit_table(ed, id, "Add rows", |t, alloc| {
        let cols = t.cols();
        let end = t.rows.len() - t.rows.iter().rev().take_while(|r| r.footer).count();
        let at = at.unwrap_or(end).min(t.rows.len());
        for (k, texts) in rows.iter().enumerate() {
            let mut row = crate::Row {
                id: alloc(),
                cells: (0..cols).map(|_| crate::Cell::new(alloc(), alloc())).collect(),
                ..Default::default()
            };
            for (c, text) in texts.iter().take(cols).enumerate() {
                set_cell_text(&mut row.cells[c], text, alloc);
            }
            t.rows.insert(at + k, row);
        }
        None
    })
}

/// Sets the text of cells (row, column, text) of table `id`. One undo step.
pub fn set_cell_texts(ed: &mut Editor, id: NodeId, cells: &[(usize, usize, String)]) -> Result<(), EditError> {
    edit_table(ed, id, "Change cells", |t, alloc| {
        for (r, c, text) in cells {
            if let Some(cell) = t.rows.get_mut(*r).and_then(|row| row.cells.get_mut(*c)).filter(|cell| !cell.covered) {
                set_cell_text(cell, text, alloc);
            }
        }
        None
    })
}

/// Table `id`'s headers: the top row (`row`) and the first column (`col`)
/// label the rest; drawn bold and shaded. One undo step.
pub fn set_headers(ed: &mut Editor, id: NodeId, row: bool, col: bool) -> Result<(), EditError> {
    edit_table(ed, id, "Table headers", |t, _| {
        let had = t.rows.first().is_some_and(|r| r.header);
        if row != had {
            for (i, r) in t.rows.iter_mut().enumerate() {
                // On: the top row; off: every header row.
                r.header = row && (i == 0 || r.header);
            }
        }
        t.header_col = col;
        if (row || col) && t.header_fill.is_none() {
            t.header_fill = Some(Rgba::rgb(0xf1, 0xf3, 0xf5));
        }
        None
    })
}

/// The last row of table `id` as a footer row (totals), or not. One undo step.
pub fn set_footer(ed: &mut Editor, id: NodeId, on: bool) -> Result<(), EditError> {
    edit_table(ed, id, "Footer row", |t, _| {
        let n = t.rows.len();
        for (i, r) in t.rows.iter_mut().enumerate() {
            // Never the only row, nor a header row.
            r.footer = on && i + 1 == n && n > 1 && !r.header;
        }
        if on && t.header_fill.is_none() {
            t.header_fill = Some(Rgba::rgb(0xf1, 0xf3, 0xf5));
        }
        None
    })
}

/// Whether table `id`'s header rows show again at the top of each page it
/// goes on to, and its footer rows at the bottom of each. One undo step.
pub fn set_repeats(ed: &mut Editor, id: NodeId, header: Option<bool>, footer: Option<bool>) -> Result<(), EditError> {
    edit_table(ed, id, "Repeat on each page", |t, _| {
        if let Some(h) = header {
            t.repeat_header = h;
        }
        if let Some(f) = footer {
            t.repeat_footer = f;
        }
        None
    })
}

/// Banded rows: every other body row in `band` (`None`: off).
pub fn set_band(ed: &mut Editor, band: Option<Rgba>) -> Result<(), EditError> {
    let (id, _, _) = cell_at(ed, ed.sel.focus.para).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Banded rows", |t, _| {
        t.band = band;
        None
    })
}

/// The minimum height of the selected rows (`None`: as tall as the content).
pub fn set_row_height(ed: &mut Editor, height: Option<f32>) -> Result<(), EditError> {
    let (id, rows, _) = selected_cells(ed).ok_or(EditError::NotHere)?;
    edit_table(ed, id, "Row height", |t, _| {
        for r in rows.clone() {
            t.rows[r].min_height = height.map(|h| h.clamp(6.0, 1000.0));
        }
        None
    })
}

/// Makes every column of table `id` the same width.
pub fn distribute_columns(ed: &mut Editor, id: NodeId) -> Result<(), EditError> {
    edit_table(ed, id, "Distribute columns", |t, _| {
        t.widths = vec![1.0; t.widths.len()];
        None
    })
}

/// A cell's text, for sorting.
fn cell_text(cell: &crate::Cell) -> String {
    cell.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sorts the body rows (not the header rows) of table `id` by column `col`:
/// as numbers when every value is one, else as text (ignoring case).
/// Tables with cells merged across rows can't be sorted.
/// Whether table `id`'s rows can be sorted: no cell below the header rows
/// spans more than one row (rows would be torn apart).
pub fn can_sort(doc: &crate::Document, id: NodeId) -> bool {
    doc.table(id).is_some_and(|t| {
        let head = t.rows.iter().take_while(|r| r.header).count();
        t.rows.len() > head + 1 && t.rows.iter().all(|r| r.cells.iter().all(|c| c.row_span <= 1))
    })
}

/// The visible cell showing column `col` of row `r` (a cell merged across
/// columns shows them all).
fn cell_showing(r: &crate::Row, col: usize) -> &crate::Cell {
    let col = col.min(r.cells.len().saturating_sub(1));
    // Cells covered by a merge to their left: the merged cell is the one before them.
    let mut i = col;
    while i > 0 && r.cells[i].covered {
        i -= 1;
    }
    &r.cells[i]
}

pub fn sort_rows(ed: &mut Editor, id: NodeId, col: usize, descending: bool) -> Result<(), EditError> {
    if !can_sort(&ed.doc, id) {
        return Err(EditError::NotHere);
    }
    edit_table(ed, id, "Sort", |t, _| {
        let head = t.rows.iter().take_while(|r| r.header).count();
        let mut body = t.rows.split_off(head);
        let key = |r: &crate::Row| cell_text(cell_showing(r, col)).trim().to_string();
        let number = |s: &str| s.replace([',', '$', '£', '€', '%'], "").trim().parse::<f64>().ok();
        let numeric = body.iter().all(|r| {
            let k = key(r);
            k.is_empty() || number(&k).is_some()
        });
        body.sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            let o = if numeric {
                number(&ka).unwrap_or(f64::MIN).total_cmp(&number(&kb).unwrap_or(f64::MIN))
            } else {
                ka.to_lowercase().cmp(&kb.to_lowercase())
            };
            if descending { o.reverse() } else { o }
        });
        t.rows.extend(body);
        None
    })
}

/// A look for a whole table: lines, header shading, bands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableStyle {
    /// Thin grey lines everywhere.
    Grid,
    /// Lines between rows only, a shaded header.
    Light,
    /// Banded rows, a shaded header, no lines.
    Striped,
    /// A strong header colour, thin lines.
    Bold,
    /// No lines, no colours.
    Plain,
}

impl TableStyle {
    pub const ALL: [TableStyle; 5] =
        [TableStyle::Grid, TableStyle::Light, TableStyle::Striped, TableStyle::Bold, TableStyle::Plain];

    pub fn name(self) -> &'static str {
        match self {
            TableStyle::Grid => "Grid",
            TableStyle::Light => "Light",
            TableStyle::Striped => "Striped",
            TableStyle::Bold => "Bold header",
            TableStyle::Plain => "Plain",
        }
    }
}

/// Gives table `id` the look of `style` (the first row becomes a header row,
/// except for Plain).
pub fn apply_style(ed: &mut Editor, id: NodeId, style: TableStyle) -> Result<(), EditError> {
    edit_table(ed, id, "Table style", |t, _| {
        style_table(t, style);
        None
    })
}

/// Gives table `t` the look of `style` (lines, header colour, bands).
pub fn style_table(t: &mut crate::Table, style: TableStyle) {
    let thin = |c: Rgba| Some(Border { width: 0.75, color: c });
    let grey = Rgba::rgb(0xc4, 0xc9, 0xd0);
    let rows_only = crate::ParaBorders { bottom: thin(grey), ..Default::default() };
    let (border, header_fill, band, cell_lines) = match style {
        TableStyle::Grid => (thin(grey), None, None, None),
        TableStyle::Light => (None, Some(Rgba::rgb(0xf1, 0xf3, 0xf5)), None, Some(rows_only)),
        TableStyle::Striped => (None, Some(Rgba::rgb(0xdd, 0xe6, 0xf3)), Some(Rgba::rgb(0xf4, 0xf7, 0xfb)), None),
        TableStyle::Bold => (thin(grey), Some(Rgba::rgb(0xcf, 0xdd, 0xfb)), None, None),
        TableStyle::Plain => (None, None, None, None),
    };
    t.border = border;
    t.header_fill = header_fill;
    t.band = band;
    for (r, row) in t.rows.iter_mut().enumerate() {
        if r == 0 && style != TableStyle::Plain {
            row.header = true;
        }
        for cell in &mut row.cells {
            cell.borders = cell_lines;
        }
    }
}

/// References › Bibliography / List of figures: a generated list that
/// updates like the table of contents.
pub fn insert_generated(ed: &mut Editor, kind: crate::TocKind) -> Result<(), EditError> {
    let id = ed.doc.alloc_id();
    let label = match kind {
        crate::TocKind::Contents => "Insert table of contents",
        crate::TocKind::Bibliography => "Insert bibliography",
        crate::TocKind::Figures => "Insert list of figures",
        crate::TocKind::ChangeLog => "Insert change log",
    };
    insert_block(
        ed,
        Block::Toc(Toc {
            id,
            kind,
            levels: 3,
            page_numbers: matches!(kind, crate::TocKind::Contents | crate::TocKind::Figures),
        }),
        label,
    )
}
