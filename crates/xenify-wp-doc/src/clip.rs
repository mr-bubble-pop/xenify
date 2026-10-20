//! Copy and paste of document fragments: the selection as a small
//! [`Document`] (paragraphs with their styles and lists, tables, pictures),
//! and pasting such a fragment, whether it came from Write itself or was
//! read from HTML or Markdown on the clipboard.

use crate::{Block, Container, Document, EditError, Editor, NodeId, Pos, Selection, Span, Step, Tx, ops};

/// The top-level block holding `id` (a paragraph may sit in a table cell):
/// (section, index).
fn top_block(doc: &Document, id: NodeId) -> Option<(usize, usize)> {
    let (mut c, mut i) = doc.locate(id)?;
    loop {
        match c {
            Container::Section(s) => return Some((s, i)),
            Container::Cell { table, .. } => (c, i) = doc.locate(table)?,
            Container::Frame(_) | Container::Note(_) | Container::Header(_) => return None,
        }
    }
}

/// The selection as a document of its own (styles, lists and pictures
/// included). Paragraphs at the ends are cut at the selection; tables and
/// pictures in between come whole.
pub fn copy_fragment(ed: &Editor) -> Document {
    let doc = &ed.doc;
    let (a, b) = doc.ordered(ed.sel);
    let mut frag = Document::new();
    frag.styles = doc.styles.clone();
    frag.lists = doc.lists.clone();
    frag.sections[0].blocks.clear();
    let trimmed = |id: NodeId| -> Option<Block> {
        let p = doc.para(id)?;
        let start = if id == a.para { a.offset } else { 0 };
        let end = if id == b.para { b.offset } else { p.text.len() };
        let mut q = p.clone();
        let s = p.slice(start.min(end)..end);
        q.text = s.text;
        q.runs = s.runs;
        Some(Block::Paragraph(q))
    };
    // Cells of one table: the rectangle of cells, as a table.
    if let Some((id, rows, cols)) = crate::tables::selected_cells(ed)
        && (rows.start() != rows.end() || cols.start() != cols.end())
        && let Some(t) = doc.table(id)
    {
        let mut sub = t.clone();
        sub.rows = t.rows[rows.clone()]
            .iter()
            .map(|r| {
                let mut r = r.clone();
                r.cells = r.cells[cols.clone()].to_vec();
                r
            })
            .collect();
        sub.widths = t.widths[cols.clone()].to_vec();
        for row in &mut sub.rows {
            for c in &mut row.cells {
                c.covered = false;
            }
        }
        let mut next = frag.next_id.max(doc.next_id);
        sub.normalize(&mut || {
            next += 1;
            NodeId(next)
        });
        frag.next_id = next + 1;
        frag.sections[0].blocks = vec![Block::Table(sub)];
        frag.repair();
        return frag;
    }
    let (ta, tb) = (top_block(doc, a.para), top_block(doc, b.para));
    let blocks: Vec<Block> = match (ta, tb) {
        // Both ends in one table (or the selection inside a frame): just the paragraphs.
        (Some(x), Some(y))
            if x == y
                && !matches!(
                    doc.blocks(&Container::Section(x.0)).and_then(|b| b.get(x.1)),
                    Some(Block::Paragraph(_))
                ) =>
        {
            ops::selected_paragraphs(ed).into_iter().filter_map(trimmed).collect()
        }
        (Some((sa, ia)), Some((sb, ib))) => {
            let mut out = Vec::new();
            for s in sa..=sb {
                let blocks = &doc.sections[s].blocks;
                let from = if s == sa { ia } else { 0 };
                let to = if s == sb { ib } else { blocks.len().saturating_sub(1) };
                for (i, block) in blocks.iter().enumerate().take(to + 1).skip(from) {
                    let edge = (s == sa && i == ia) || (s == sb && i == ib);
                    match block {
                        Block::Paragraph(p) if edge => out.extend(trimmed(p.id)),
                        other => out.push(other.clone()),
                    }
                }
            }
            out
        }
        _ => ops::selected_paragraphs(ed).into_iter().filter_map(trimmed).collect(),
    };
    for src in images_in(&blocks) {
        if let Some(bytes) = doc.assets.get(&src) {
            frag.assets.insert(src, bytes.clone());
        }
    }
    frag.sections[0].blocks = blocks;
    frag.repair();
    frag
}

/// Gives `block` and everything inside it new ids from `doc`.
fn reid(block: &mut Block, doc: &mut Document) {
    reid_with(block, &mut || doc.alloc_id());
}

/// Gives `block` and everything inside it new ids from `alloc`.
fn reid_with(block: &mut Block, alloc: &mut dyn FnMut() -> NodeId) {
    match block {
        Block::Paragraph(p) => p.id = alloc(),
        Block::Image(i) => i.id = alloc(),
        Block::Toc(t) => t.id = alloc(),
        Block::Table(t) => {
            t.id = alloc();
            for row in &mut t.rows {
                row.id = alloc();
                for cell in &mut row.cells {
                    cell.id = alloc();
                    for b in &mut cell.blocks {
                        reid_with(b, alloc);
                    }
                }
            }
        }
    }
}

/// Makes the styles and lists `frag` uses exist in the target document
/// (the target's own definitions win when both have one).
fn bring_definitions(tx: &mut Tx, frag: &Document, blocks: &[Block]) -> Result<(), EditError> {
    let mut paras = Vec::new();
    paragraphs_in(blocks, &mut paras);
    for p in paras {
        if tx.doc.styles.get(&p.style).is_none()
            && let Some(def) = frag.styles.get(&p.style)
        {
            tx.step(Step::SetStyleDef { id: p.style.clone(), def: Some(def.clone()) })?;
        }
        if let Some(item) = &p.list
            && tx.doc.list_def(&item.list).is_none()
            && let Some(def) = frag.list_def(&item.list)
        {
            tx.step(Step::SetListDef { id: item.list.clone(), def: Some(def.clone()) })?;
        }
    }
    Ok(())
}

/// Pastes `frag` over the selection. One paragraph goes into the text at
/// the caret; several paragraphs, tables and pictures go in as blocks, the
/// first and last joining the text around the caret.
pub fn paste_fragment(ed: &mut Editor, frag: &Document, label: &str) -> Result<(), EditError> {
    let blocks: Vec<Block> = frag.sections.iter().flat_map(|s| s.blocks.iter().cloned()).collect();
    if blocks.is_empty() {
        return Ok(());
    }
    // Pictures' bytes travel with the fragment.
    for (src, bytes) in &frag.assets {
        ed.doc.assets.entry(src.clone()).or_insert_with(|| bytes.clone());
    }
    // A table pasted into a table: its cells fill the cells from the caret's on.
    if let [Block::Table(src)] = blocks.as_slice()
        && let Some((id, r0, c0)) = crate::tables::cell_at(ed, ed.sel.focus.para)
    {
        return paste_into_table(ed, id, r0, c0, src, label);
    }
    let (a, b) = ed.doc.ordered(ed.sel);
    ed.seal();
    ed.transact(label, None, |tx| {
        bring_definitions(tx, frag, &blocks)?;
        let pos = ops::delete_range(tx, a, b)?;
        // One paragraph: just its text.
        if let [Block::Paragraph(p)] = blocks.as_slice() {
            let span = Span { text: p.text.clone(), runs: p.runs.clone() };
            let len = span.len();
            if len > 0 {
                tx.step(Step::Replace { para: pos.para, offset: pos.offset, remove: 0, insert: span })?;
            }
            return Ok(Selection::caret(Pos::new(pos.para, pos.offset + len)));
        }
        // Several blocks: split at the caret, then fill in between.
        let head = pos.para;
        let head_empty = tx.doc.para(head).is_some_and(|p| pos.offset == 0 && p.text.is_empty());
        let tail = ops::split_at(tx, pos)?;
        let (container, _) = tx.doc.locate(tail.para).ok_or(EditError::NoParagraph(tail.para))?;
        let mut rest: &[Block] = &blocks;
        // The first paragraph joins the text before the caret.
        if let Some((Block::Paragraph(first), more)) = rest.split_first() {
            let span = Span { text: first.text.clone(), runs: first.runs.clone() };
            let at = tx.doc.para(head).map_or(0, |p| p.text.len());
            if !span.is_empty() {
                tx.step(Step::Replace { para: head, offset: at, remove: 0, insert: span })?;
            }
            if head_empty {
                tx.step(Step::SetStyle { para: head, style: first.style.clone() })?;
                tx.step(Step::SetProps { para: head, props: first.props.clone() })?;
                tx.step(Step::SetList { para: head, list: first.list.clone() })?;
            }
            rest = more;
        }
        // The last paragraph joins the text after the caret.
        let last = match rest.split_last() {
            Some((Block::Paragraph(p), more)) => {
                rest = more;
                Some(p.clone())
            }
            _ => None,
        };
        for block in rest {
            let mut block = block.clone();
            reid(&mut block, tx.doc);
            let index = tx.doc.locate(tail.para).map_or(0, |(_, i)| i);
            tx.step(Step::InsertBlock { container: container.clone(), index, block })?;
        }
        let mut caret = tail;
        if let Some(p) = last {
            let tail_empty = tx.doc.para(tail.para).is_some_and(|q| q.text.is_empty());
            let span = Span { text: p.text.clone(), runs: p.runs.clone() };
            let len = span.len();
            if len > 0 {
                tx.step(Step::Replace { para: tail.para, offset: 0, remove: 0, insert: span })?;
            }
            if tail_empty {
                tx.step(Step::SetStyle { para: tail.para, style: p.style.clone() })?;
                tx.step(Step::SetProps { para: tail.para, props: p.props.clone() })?;
                tx.step(Step::SetList { para: tail.para, list: p.list.clone() })?;
            }
            caret = Pos::new(tail.para, len);
        }
        Ok(Selection::caret(caret))
    })
}

/// Every paragraph in `blocks`, table cells included.
fn paragraphs_in<'a>(blocks: &'a [Block], out: &mut Vec<&'a crate::Paragraph>) {
    for b in blocks {
        match b {
            Block::Paragraph(p) => out.push(p),
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        paragraphs_in(&cell.blocks, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The sources of the pictures in `blocks`.
fn images_in(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Image(i) => out.push(i.src.clone()),
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        out.extend(images_in(&cell.blocks));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Fills table `id` from cell (`r0`, `c0`) with the cells of `src`, adding
/// rows at the bottom when needed (columns past the edge are dropped).
fn paste_into_table(
    ed: &mut Editor,
    id: NodeId,
    r0: usize,
    c0: usize,
    src: &crate::Table,
    label: &str,
) -> Result<(), EditError> {
    let (container, index) = ed.doc.locate(id).ok_or(EditError::NotHere)?;
    let mut t = ed.doc.table(id).ok_or(EditError::NotHere)?.clone();
    let mut next = ed.doc.next_id;
    let mut alloc = || {
        let n = NodeId(next);
        next += 1;
        n
    };
    let cols = t.cols();
    for (dr, row) in src.rows.iter().enumerate() {
        let r = r0 + dr;
        while t.rows.len() <= r {
            let new = crate::Row {
                id: alloc(),
                header: false,
                footer: false,
                repeat: None,
                min_height: None,
                cells: (0..cols).map(|_| crate::Cell::new(alloc(), alloc())).collect(),
            };
            t.rows.push(new);
        }
        for (dc, cell) in row.cells.iter().enumerate().filter(|(_, c)| !c.covered) {
            let c = c0 + dc;
            if c >= cols || t.rows[r].cells[c].covered {
                continue;
            }
            let mut blocks = cell.blocks.clone();
            for b in &mut blocks {
                reid_with(b, &mut alloc);
            }
            if blocks.iter().any(|b| matches!(b, Block::Paragraph(_))) {
                t.rows[r].cells[c].blocks = blocks;
            }
        }
    }
    t.normalize(&mut alloc);
    ed.doc.next_id = next;
    let first = crate::tables::first_para(&t, r0, c0);
    let sel = first.map_or(ed.sel, |p| Selection::caret(Pos::new(p, 0)));
    ed.seal();
    ed.transact(label, None, |tx| {
        tx.step(Step::ReplaceBlock { container, index, block: Block::Table(t) })?;
        Ok(sel)
    })
}

/// Tab-separated text (copied from a spreadsheet) as a table, when every
/// line has the same number of tabs. `None` for anything else.
pub fn tsv_fragment(text: &str) -> Option<Document> {
    let lines: Vec<&str> = text.trim_end_matches(['\n', '\r']).lines().collect();
    let tabs = lines.first()?.matches('\t').count();
    if lines.len() < 2 || tabs == 0 || lines.iter().any(|l| l.matches('\t').count() != tabs) {
        return None;
    }
    let mut doc = Document::new();
    let mut next = doc.next_id;
    let mut alloc = || {
        let n = NodeId(next);
        next += 1;
        n
    };
    let mut t = crate::Table::new(lines.len(), tabs + 1, &mut alloc);
    for (r, line) in lines.iter().enumerate() {
        for (c, value) in line.split('\t').enumerate() {
            let id = alloc();
            t.rows[r].cells[c].blocks = vec![Block::Paragraph(crate::Paragraph::new(
                id,
                crate::StyleSheet::NORMAL,
                value.trim_end_matches('\r'),
            ))];
        }
    }
    doc.next_id = next;
    doc.sections[0].blocks = vec![Block::Table(t)];
    doc.repair();
    Some(doc)
}
