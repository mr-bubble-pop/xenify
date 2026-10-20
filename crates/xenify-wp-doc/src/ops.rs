//! Editing operations used by the UI, the CLI and the AI: each is one
//! undoable transaction on an [`Editor`].

use crate::{
    Block, CharFmt, Container, Document, EditError, Editor, ListItem, MergeKey, NodeId, ParaProps, Pos, Selection,
    Span, Step, Tx,
};
use unicode_segmentation::UnicodeSegmentation;

/// The block after (or before) block `id` in reading order at its level:
/// the next block in the same container, or across a section boundary.
pub(crate) fn neighbour_block(doc: &Document, id: NodeId, forward: bool) -> Option<(Container, usize)> {
    let (c, i) = doc.locate(id)?;
    let len = doc.blocks(&c)?.len();
    if forward && i + 1 < len {
        return Some((c, i + 1));
    }
    if !forward && i > 0 {
        return Some((c, i - 1));
    }
    let Container::Section(si) = c else { return None };
    if forward {
        (si + 1..doc.sections.len()).find(|&s| !doc.sections[s].blocks.is_empty()).map(|s| (Container::Section(s), 0))
    } else {
        (0..si)
            .rev()
            .find(|&s| !doc.sections[s].blocks.is_empty())
            .map(|s| (Container::Section(s), doc.sections[s].blocks.len() - 1))
    }
}

/// A paragraph next to block `id` at its level (after it, else before).
pub(crate) fn neighbour_para(doc: &Document, id: NodeId) -> Option<NodeId> {
    for forward in [true, false] {
        if let Some((c, i)) = neighbour_block(doc, id, forward)
            && let Some(Block::Paragraph(p)) = doc.blocks(&c).and_then(|v| v.get(i))
        {
            return Some(p.id);
        }
    }
    None
}

/// The paragraph nearest block `id` (after it, else before): where the
/// caret goes to show the block.
pub fn para_after_block(doc: &Document, id: NodeId) -> Option<NodeId> {
    for forward in [true, false] {
        let mut cur = id;
        while let Some((c, i)) = neighbour_block(doc, cur, forward) {
            match doc.blocks(&c)?.get(i)? {
                Block::Paragraph(p) => return Some(p.id),
                b => cur = b.id(),
            }
        }
    }
    None
}

/// Whether paragraphs in `a` and `b` can be joined (same container, or both in section bodies).
fn joinable(a: &Container, b: &Container) -> bool {
    a == b || matches!((a, b), (Container::Section(_), Container::Section(_)))
}

/// Deletes from `a` to `b` (in document order); returns where the caret goes.
pub(crate) fn delete_range(tx: &mut Tx, a: Pos, b: Pos) -> Result<Pos, EditError> {
    // Tracking changes: the text stays, marked deleted.
    if tx.doc.meta.track_changes && a != b {
        return crate::review::mark_deleted(tx, a, b);
    }
    delete_range_now(tx, a, b)
}

/// Removes `a..b` (even while tracking changes).
pub(crate) fn delete_range_now(tx: &mut Tx, a: Pos, b: Pos) -> Result<Pos, EditError> {
    // Everything in the body: an empty page is left (see `clear_all`).
    let in_body = |p: Pos| matches!(tx.doc.locate(p.para), Some((Container::Section(_), _)));
    if a != b && a == tx.doc.start() && b == tx.doc.body_end() && in_body(a) && in_body(b) {
        return clear_all(tx, a, b);
    }
    if a.para == b.para {
        if b.offset > a.offset {
            tx.step(Step::Replace {
                para: a.para,
                offset: a.offset,
                remove: b.offset - a.offset,
                insert: Span::default(),
            })?;
        }
        return Ok(a);
    }
    let len = |tx: &Tx, id| tx.doc.para(id).map(|p| p.text.len()).ok_or(EditError::NoParagraph(id));
    let ca = tx.doc.locate(a.para).ok_or(EditError::NoParagraph(a.para))?.0;
    let cb = tx.doc.locate(b.para).ok_or(EditError::NoParagraph(b.para))?.0;
    if !joinable(&ca, &cb) {
        // Across table cells: clear the text in range, keep the structure.
        let ids: Vec<NodeId> = {
            let mut inside = false;
            let mut v = Vec::new();
            for p in tx.doc.paragraphs() {
                inside |= p.id == a.para;
                if inside {
                    v.push(p.id);
                }
                if p.id == b.para {
                    break;
                }
            }
            v
        };
        for id in ids {
            let l = len(tx, id)?;
            let start = if id == a.para { a.offset } else { 0 };
            let end = if id == b.para { b.offset } else { l };
            if end > start {
                tx.step(Step::Replace { para: id, offset: start, remove: end - start, insert: Span::default() })?;
            }
        }
        return Ok(a);
    }
    // Cut the tail of the first paragraph and the head of the last…
    let first_len = len(tx, a.para)?;
    if first_len > a.offset {
        tx.step(Step::Replace {
            para: a.para,
            offset: a.offset,
            remove: first_len - a.offset,
            insert: Span::default(),
        })?;
    }
    if b.offset > 0 {
        tx.step(Step::Replace { para: b.para, offset: 0, remove: b.offset, insert: Span::default() })?;
    }
    // …drop whole blocks in between (paragraphs, tables, images), then join the two ends.
    while let Some((c, i)) = neighbour_block(tx.doc, a.para, true) {
        let id = tx.doc.blocks(&c).and_then(|v| v.get(i)).map(Block::id);
        if id == Some(b.para) || id.is_none() {
            break;
        }
        tx.step(Step::RemoveBlock { container: c, index: i })?;
    }
    tx.step(Step::Join { para: a.para, next: b.para })?;
    drop_empty_sections(tx)?;
    Ok(a)
}

/// Deletes the whole body and what hangs off it, leaving one empty plain
/// paragraph: no list, style or direct formatting left on it, and no text
/// boxes, footnotes or endnotes left over.
fn clear_all(tx: &mut Tx, a: Pos, b: Pos) -> Result<Pos, EditError> {
    for si in 0..tx.doc.sections.len() {
        if !tx.doc.sections[si].frames.is_empty() {
            tx.step(Step::SetFrames { section: si, frames: Vec::new() })?;
        }
        if !tx.doc.sections[si].notes.is_empty() {
            tx.step(Step::SetNotes { section: si, notes: Vec::new() })?;
        }
    }
    // Text and blocks between the ends (the ordinary way, now it can't hit a note or box).
    let at = if a.para == b.para {
        if b.offset > a.offset {
            tx.step(Step::Replace {
                para: a.para,
                offset: a.offset,
                remove: b.offset - a.offset,
                insert: Span::default(),
            })?;
        }
        a
    } else {
        let a2 = tx.doc.start();
        let b2 = tx.doc.body_end();
        delete_between(tx, a2, b2)?
    };
    let p = tx.doc.para(at.para).ok_or(EditError::NoParagraph(at.para))?;
    let (list, style, props) =
        (p.list.is_some(), p.style != crate::StyleSheet::NORMAL, p.props != crate::ParaProps::default());
    if list {
        tx.step(Step::SetList { para: at.para, list: None })?;
    }
    if style {
        tx.step(Step::SetStyle { para: at.para, style: crate::StyleSheet::NORMAL.into() })?;
    }
    if props {
        tx.step(Step::SetProps { para: at.para, props: crate::ParaProps::default() })?;
    }
    Ok(at)
}

/// `delete_range_now` without the whole-body case.
fn delete_between(tx: &mut Tx, a: Pos, b: Pos) -> Result<Pos, EditError> {
    let len = |tx: &Tx, id| tx.doc.para(id).map(|p| p.text.len()).ok_or(EditError::NoParagraph(id));
    let first_len = len(tx, a.para)?;
    if first_len > a.offset {
        tx.step(Step::Replace {
            para: a.para,
            offset: a.offset,
            remove: first_len - a.offset,
            insert: Span::default(),
        })?;
    }
    if b.offset > 0 {
        tx.step(Step::Replace { para: b.para, offset: 0, remove: b.offset, insert: Span::default() })?;
    }
    while let Some((c, i)) = neighbour_block(tx.doc, a.para, true) {
        let id = tx.doc.blocks(&c).and_then(|v| v.get(i)).map(Block::id);
        if id == Some(b.para) || id.is_none() {
            break;
        }
        tx.step(Step::RemoveBlock { container: c, index: i })?;
    }
    tx.step(Step::Join { para: a.para, next: b.para })?;
    drop_empty_sections(tx)?;
    Ok(a)
}

/// Merges sections left without blocks into the one before.
fn drop_empty_sections(tx: &mut Tx) -> Result<(), EditError> {
    while let Some(i) = (1..tx.doc.sections.len()).find(|&i| tx.doc.sections[i].blocks.is_empty()) {
        // A section's rich headers go first (so undo brings them back).
        if !tx.doc.sections[i].headers.is_empty() {
            tx.step(Step::SetHeaders { section: i, headers: Vec::new() })?;
        }
        tx.step(Step::JoinSection { section: i - 1 })?;
    }
    Ok(())
}

/// Inserts `text` (which may contain newlines) at `at` with direct format `fmt`.
fn insert_at(tx: &mut Tx, at: Pos, text: &str, fmt: &CharFmt) -> Result<Pos, EditError> {
    let mut pos = at;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            pos = split_at(tx, pos)?;
        }
        let line = line.trim_end_matches('\r');
        if !line.is_empty() {
            tx.step(Step::Replace {
                para: pos.para,
                offset: pos.offset,
                remove: 0,
                insert: Span::plain(line, fmt.clone()),
            })?;
            pos.offset += line.len();
        }
    }
    Ok(pos)
}

pub(crate) fn split_at(tx: &mut Tx, at: Pos) -> Result<Pos, EditError> {
    let p = tx.doc.para(at.para).ok_or(EditError::NoParagraph(at.para))?;
    // Enter at the very end of a heading starts a Normal paragraph; anywhere
    // else the tail keeps the style.
    let style = if at.offset == p.text.len() { tx.doc.styles.next_style(&p.style) } else { p.style.clone() };
    // List items continue the list; a heading's direct formatting doesn't
    // carry over to the body text after it.
    let (mut props, list) = (if style == p.style { p.props.clone() } else { ParaProps::default() }, p.list.clone());
    // (Enter after more of an item starts the next item.)
    let mut list = list.map(|l| ListItem { restart: None, checked: false, follow: false, ..l });
    // The body text after a numbered heading isn't numbered with it: a numbering
    // that came with the style goes with it.
    if style != p.style {
        let old = tx.doc.styles.get(&p.style).and_then(|s| s.list.clone());
        let numbered_by_style =
            list.as_ref().is_some_and(|l| l.list == crate::ListDef::HEADINGS || Some(&l.list) == old.as_ref());
        if numbered_by_style {
            let new = tx.doc.styles.get(&style).map(|s| (s.list.clone(), s.outline_level.unwrap_or(1)));
            list = match new {
                Some((Some(l), level)) => Some(ListItem::new(&l, level.saturating_sub(1))),
                _ => None,
            };
        }
    }
    // The paragraph's own break (and any tracked change on it) ends up after the tail.
    let old_break = p.props.break_change.clone();
    props.break_change = old_break.clone();
    let new_id = tx.doc.alloc_id();
    tx.step(Step::Split { para: at.para, offset: at.offset, new_id, style, props, list, place: None })?;
    // The new break: marked inserted when tracking changes.
    let mark = tx.doc.meta.track_changes.then(|| crate::review::Change {
        kind: crate::review::ChangeKind::Inserted,
        author: crate::review::user(tx.doc),
        date: crate::review::now(),
    });
    if mark.is_some() || old_break.is_some() {
        let head = tx.doc.para(at.para).ok_or(EditError::NoParagraph(at.para))?;
        let props = ParaProps { break_change: mark, ..head.props.clone() };
        tx.step(Step::SetProps { para: at.para, props })?;
    }
    Ok(Pos::new(new_id, 0))
}

/// Types `text` over the selection. `fmt` overrides the format typed text
/// gets (e.g. after Ctrl+B with nothing selected).
pub fn insert_text(ed: &mut Editor, text: &str, fmt: Option<&CharFmt>) -> Result<(), EditError> {
    let sel = ed.sel;
    let (a, b) = ed.doc.ordered(sel);
    let fmt = match fmt {
        Some(f) => f.clone(),
        None => ed.doc.para(a.para).map(|p| p.fmt_at(a.offset)).unwrap_or_default(),
    };
    // Typed text never continues a field (a footnote number, a citation…),
    // continues a comment only inside it, and is marked inserted when tracking.
    let after =
        ed.doc.para(b.para).map(|p| p.fmt_at(b.offset + p.text[b.offset..].chars().next().map_or(0, char::len_utf8)));
    let after = after.filter(|_| b.offset < ed.doc.para(b.para).map_or(0, |p| p.text.len()));
    let fmt = crate::review::typed_fmt(&ed.doc, fmt, after.as_ref());
    let single_char = text.chars().count() == 1 && !text.contains('\n');
    let merge = (sel.is_collapsed() && single_char).then_some(MergeKey::Typing(a.para));
    // A word typed after a space starts a new undo step.
    let after_space = ed.doc.para(a.para).is_some_and(|p| p.text[..a.offset].ends_with(char::is_whitespace));
    if single_char && after_space && !text.chars().all(char::is_whitespace) {
        ed.seal();
    }
    ed.transact("Typing", merge, |tx| {
        let at = delete_range(tx, a, b)?;
        let end = insert_at(tx, at, text, &fmt)?;
        Ok(Selection::caret(end))
    })
}

/// Replaces the selection with formatted paragraphs (paste): the first
/// span joins the paragraph at the caret, the rest become new paragraphs.
pub fn insert_spans(ed: &mut Editor, spans: &[Span], label: &str) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    // Tracking changes: pasted text is marked inserted.
    let tracked: Vec<Span>;
    let spans = if ed.doc.meta.track_changes {
        let mark = crate::review::typed_fmt(&ed.doc, CharFmt::default(), None).change;
        tracked = spans
            .iter()
            .map(|s| {
                let mut s = s.clone();
                for r in &mut s.runs {
                    r.fmt.change = mark.clone();
                }
                s
            })
            .collect();
        &tracked[..]
    } else {
        spans
    };
    ed.transact(label, None, |tx| {
        let mut pos = delete_range(tx, a, b)?;
        for (i, span) in spans.iter().enumerate() {
            if i > 0 {
                pos = split_at(tx, pos)?;
            }
            if !span.is_empty() {
                tx.step(Step::Replace { para: pos.para, offset: pos.offset, remove: 0, insert: span.clone() })?;
                pos.offset += span.len();
            }
        }
        Ok(Selection::caret(pos))
    })
}

pub fn delete_selection(ed: &mut Editor) -> Result<(), EditError> {
    if ed.sel.is_collapsed() {
        return Ok(());
    }
    let (a, b) = ed.doc.ordered(ed.sel);
    ed.transact("Delete", None, |tx| delete_range(tx, a, b).map(Selection::caret))
}

/// The offset one grapheme (or word, with `word`) before `offset`.
fn step_back(text: &str, offset: usize, word: bool) -> usize {
    if word {
        let head = &text[..offset];
        // Skip trailing spaces, then the word before them.
        let trimmed = head.trim_end();
        trimmed.split_word_bound_indices().next_back().map(|(i, _)| i).unwrap_or(0)
    } else {
        text[..offset].grapheme_indices(true).next_back().map(|(i, _)| i).unwrap_or(0)
    }
}

/// The offset one grapheme after `offset`, or (with `word`) the start of
/// the next word.
fn step_forward(text: &str, offset: usize, word: bool) -> usize {
    let tail = &text[offset..];
    if !word {
        return offset + tail.graphemes(true).next().map_or(0, str::len);
    }
    let blank = |w: &str| w.chars().all(char::is_whitespace);
    let mut segs = tail.split_word_bound_indices().peekable();
    let mut end = 0;
    if let Some((_, w)) = segs.peek().copied()
        && !blank(w)
    {
        end = w.len();
        segs.next();
    }
    for (i, w) in segs {
        if !blank(w) {
            break;
        }
        end = i + w.len();
    }
    offset + end
}

/// Backspace (Ctrl+Backspace with `word`).
pub fn backspace(ed: &mut Editor, word: bool) -> Result<(), EditError> {
    if !ed.sel.is_collapsed() {
        return delete_selection(ed);
    }
    let at = ed.sel.focus;
    let p = ed.doc.para(at.para).ok_or(EditError::NoParagraph(at.para))?;
    if at.offset > 0 {
        let from = step_back(&p.text, at.offset, word);
        return ed.transact("Delete", (!word).then_some(MergeKey::Deleting(at.para)), |tx| {
            delete_range(tx, Pos::new(at.para, from), at).map(Selection::caret)
        });
    }
    // At the start of a list item, Backspace takes it out of the list first
    // (more of an item just joins the paragraph before, like plain text).
    if let Some(item) = p.list.clone().filter(|i| !i.follow) {
        let id = p.id;
        let list = (item.level > 0).then(|| ListItem { level: item.level - 1, ..item });
        ed.seal();
        return ed.transact("Remove from list", None, |tx| {
            tx.step(Step::SetList { para: id, list })?;
            Ok(Selection::caret(at))
        });
    }
    ed.seal();
    match neighbour_block(&ed.doc, at.para, false) {
        // Join with the paragraph before.
        Some((c, i)) if matches!(ed.doc.blocks(&c).and_then(|v| v.get(i)), Some(Block::Paragraph(_))) => {
            let Some(Block::Paragraph(prev)) = ed.doc.blocks(&c).and_then(|v| v.get(i)) else { unreachable!() };
            let prev = Pos::new(prev.id, prev.text.len());
            ed.transact("Delete", None, |tx| delete_range(tx, prev, at).map(Selection::caret))
        }
        // An image or contents table before: delete it.
        Some((c, i)) if matches!(ed.doc.blocks(&c).and_then(|v| v.get(i)), Some(Block::Image(_) | Block::Toc(_))) => ed
            .transact("Delete", None, |tx| {
                tx.step(Step::RemoveBlock { container: c, index: i })?;
                drop_empty_sections(tx)?;
                Ok(Selection::caret(at))
            }),
        // A table before (or the start of a cell): just move into it.
        Some(_) => {
            if let Some(prev) = ed.doc.prev_para(at.para) {
                let end = ed.doc.para(prev).map_or(0, |p| p.text.len());
                ed.sel = Selection::caret(Pos::new(prev, end));
            }
            Ok(())
        }
        None => Ok(()),
    }
}

/// Delete (Ctrl+Delete with `word`).
pub fn delete_forward(ed: &mut Editor, word: bool) -> Result<(), EditError> {
    if !ed.sel.is_collapsed() {
        return delete_selection(ed);
    }
    let at = ed.sel.focus;
    let p = ed.doc.para(at.para).ok_or(EditError::NoParagraph(at.para))?;
    if at.offset < p.text.len() {
        let to = step_forward(&p.text, at.offset, word);
        let tracking = ed.doc.meta.track_changes;
        ed.transact("Delete", None, |tx| delete_range(tx, at, Pos::new(at.para, to)).map(Selection::caret))?;
        // Tracked: the text stays (struck through); the caret moves past it.
        if tracking {
            let len = ed.doc.para(at.para).map_or(0, |p| p.text.len());
            let to = if crate::review::deleted_at(&ed.doc, at) { to.min(len) } else { at.offset };
            ed.sel = Selection::caret(Pos::new(at.para, to));
        }
        return Ok(());
    }
    match neighbour_block(&ed.doc, at.para, true) {
        Some((c, i)) => match ed.doc.blocks(&c).and_then(|v| v.get(i)) {
            Some(Block::Paragraph(next)) => {
                let next = Pos::new(next.id, 0);
                let tracking = ed.doc.meta.track_changes;
                ed.transact("Delete", None, |tx| delete_range(tx, at, next).map(Selection::caret))?;
                // Tracked: the break stays (marked deleted); the caret moves past it.
                if tracking && ed.doc.para(next.para).is_some() {
                    ed.sel = Selection::caret(next);
                }
                Ok(())
            }
            Some(Block::Image(_) | Block::Toc(_)) => ed.transact("Delete", None, |tx| {
                tx.step(Step::RemoveBlock { container: c, index: i })?;
                drop_empty_sections(tx)?;
                Ok(Selection::caret(at))
            }),
            _ => Ok(()),
        },
        None => Ok(()),
    }
}

/// Enter: splits the paragraph at the caret. In an empty list item it
/// moves the item out a level (out of the list at the top level) instead.
pub fn split_paragraph(ed: &mut Editor) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    if a == b
        && let Some(p) = ed.doc.para(a.para)
        && p.text.is_empty()
        && let Some(item) = p.list.clone()
    {
        let id = p.id;
        let list = (item.level > 0).then(|| ListItem { level: item.level - 1, ..item });
        ed.seal();
        return ed.transact("End list", None, |tx| {
            tx.step(Step::SetList { para: id, list })?;
            Ok(Selection::caret(a))
        });
    }
    ed.seal();
    ed.transact("New paragraph", None, |tx| {
        let at = delete_range(tx, a, b)?;
        split_at(tx, at).map(Selection::caret)
    })
}

/// The selection as formatted spans, one per paragraph (internal copy/paste).
pub fn copy_spans(ed: &Editor) -> Vec<Span> {
    let (a, b) = ed.doc.ordered(ed.sel);
    selected_paragraphs(ed)
        .into_iter()
        .filter_map(|id| {
            let p = ed.doc.para(id)?;
            let start = if id == a.para { a.offset } else { 0 };
            let end = if id == b.para { b.offset } else { p.text.len() };
            Some(p.slice(start.min(end)..end))
        })
        .collect()
}

/// Paragraph ids the selection touches, in order.
pub fn selected_paragraphs(ed: &Editor) -> Vec<NodeId> {
    let (a, b) = ed.doc.ordered(ed.sel);
    let mut out = Vec::new();
    let mut inside = false;
    for p in ed.doc.paragraphs() {
        inside |= p.id == a.para;
        if inside {
            out.push(p.id);
        }
        if p.id == b.para {
            break;
        }
    }
    out
}

/// Gives every selected paragraph paragraph style `style`.
pub fn set_style(ed: &mut Editor, style: &str) -> Result<(), EditError> {
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.transact("Paragraph style", None, |tx| {
        for id in paras {
            let Some(p) = tx.doc.para(id) else { continue };
            if p.style == style {
                continue;
            }
            // A style with a list numbers its paragraphs (and leaving it takes the number away).
            let old_list = tx.doc.styles.get(&p.style).and_then(|s| s.list.clone());
            let new = tx.doc.styles.get(style).map(|s| (s.list.clone(), s.outline_level.unwrap_or(1)));
            let had = p.list.clone();
            tx.step(Step::SetStyle { para: id, style: style.into() })?;
            match new {
                Some((Some(list), level)) => {
                    if had.as_ref().is_none_or(|l| l.list != list) {
                        let item = ListItem::new(&list, level.saturating_sub(1));
                        tx.step(Step::SetList { para: id, list: Some(item) })?;
                    }
                }
                _ => {
                    if old_list.is_some() && had.as_ref().map(|l| &l.list) == old_list.as_ref() {
                        tx.step(Step::SetList { para: id, list: None })?;
                    }
                }
            }
        }
        Ok(sel)
    })
}

/// Format › Number the headings: Heading 1–3 carry the numbered-headings list
/// (1, 1.1, 1.1.1), so every heading is numbered, now and from then on; or
/// not, with `on` false.
pub fn number_headings(ed: &mut Editor, on: bool) -> Result<(), EditError> {
    let styles: Vec<String> = (1..=3).map(crate::StyleSheet::heading).collect();
    set_style_lists(ed, &styles, on.then_some(crate::ListDef::HEADINGS))
}

/// Paragraphs in style `style` numbered (or bulleted) with list `list`, now
/// and whenever the style is used; `None` takes the numbering away. A heading
/// gets the level of its outline level.
pub fn set_style_list(ed: &mut Editor, style: &str, list: Option<&str>) -> Result<(), EditError> {
    set_style_lists(ed, &[style.to_string()], list)
}

fn set_style_lists(ed: &mut Editor, styles: &[String], list: Option<&str>) -> Result<(), EditError> {
    let sel = ed.sel;
    ed.seal();
    ed.transact(if list.is_some() { "Number a style" } else { "Unnumber a style" }, None, |tx| {
        if let Some(list) = list
            && tx.doc.list_def(list).is_none()
            && let Some(def) = crate::ListDef::builtin_def(list)
        {
            tx.step(Step::SetListDef { id: list.into(), def: Some(def) })?;
        }
        for id in styles {
            let Some(mut def) = tx.doc.styles.get(id).cloned() else { continue };
            let old = def.list.clone();
            let level = def.outline_level.unwrap_or(1).saturating_sub(1);
            def.list = list.map(str::to_string);
            tx.step(Step::SetStyleDef { id: id.clone(), def: Some(def) })?;
            let paras: Vec<(NodeId, Option<String>)> = tx
                .doc
                .paragraphs()
                .filter(|p| &p.style == id)
                .map(|p| (p.id, p.list.as_ref().map(|l| l.list.clone())))
                .collect();
            for (p, has) in paras {
                match list {
                    Some(l) => tx.step(Step::SetList { para: p, list: Some(ListItem::new(l, level)) })?,
                    // Only numbering the style gave (a list typed in by hand stays).
                    None if has.is_some() && (has == old || has.as_deref() == Some(crate::ListDef::HEADINGS)) => {
                        tx.step(Step::SetList { para: p, list: None })?
                    }
                    None => {}
                }
            }
        }
        Ok(sel)
    })?;
    ed.seal();
    Ok(())
}

/// Panels (info, warning…): (style id, name, tint, line colour).
pub const PANELS: &[(&str, &str, &str, &str)] = &[
    ("panel-info", "Info panel", "#EFF6FF", "#2563EB"),
    ("panel-note", "Note panel", "#F5F3FF", "#7C3AED"),
    ("panel-success", "Success panel", "#F0FDF4", "#16A34A"),
    ("panel-warning", "Warning panel", "#FFFBEB", "#D97706"),
    ("panel-alert", "Alert panel", "#FEF2F2", "#DC2626"),
];

/// The style of panel `id` (a tinted paragraph with a coloured line down its left).
pub fn panel_style(id: &str) -> Option<crate::ParaStyle> {
    let (id, name, tint, line) = PANELS.iter().find(|(p, ..)| *p == id)?;
    let line = crate::Border { width: 3.0, color: crate::Rgba::from_hex(line)? };
    Some(crate::ParaStyle {
        id: id.to_string(),
        name: name.to_string(),
        based_on: Some(crate::StyleSheet::NORMAL.into()),
        next: Some(crate::StyleSheet::NORMAL.into()),
        para: ParaProps {
            shading: crate::Rgba::from_hex(tint),
            borders: Some(crate::ParaBorders { left: Some(line), ..Default::default() }),
            indent_left: Some(10.0),
            indent_right: Some(6.0),
            space_before: Some(8.0),
            space_after: Some(8.0),
            ..Default::default()
        },
        ..Default::default()
    })
}

/// Makes the selected paragraphs panel `id` (adding its style when the
/// document hasn't got it yet); `None` takes them back to body text.
pub fn set_panel(ed: &mut Editor, id: Option<&str>) -> Result<(), EditError> {
    let style = id.unwrap_or(crate::StyleSheet::NORMAL);
    let def = match id {
        Some(i) if ed.doc.styles.get(i).is_none() => Some(panel_style(i).ok_or(EditError::NotHere)?),
        _ => None,
    };
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.seal();
    ed.transact("Panel", None, |tx| {
        if let Some(def) = def {
            tx.step(Step::SetStyleDef { id: style.into(), def: Some(def) })?;
        }
        for p in paras {
            tx.step(Step::SetStyle { para: p, style: style.into() })?;
        }
        Ok(sel)
    })?;
    ed.seal();
    Ok(())
}

/// Applies direct paragraph formatting `patch` to every selected paragraph.
pub fn apply_para_props(ed: &mut Editor, patch: &ParaProps, label: &str) -> Result<(), EditError> {
    apply_para_props_merged(ed, patch, label, None)
}

/// [`apply_para_props`], merging with the previous change of the same
/// `merge` key (dragging an indent on the ruler is one undo step).
pub fn apply_para_props_merged(
    ed: &mut Editor,
    patch: &ParaProps,
    label: &str,
    merge: Option<MergeKey>,
) -> Result<(), EditError> {
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.transact(label, merge, |tx| {
        for id in paras {
            let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
            let props = p.props.with(patch);
            if props != p.props {
                tx.step(Step::SetProps { para: id, props })?;
            }
        }
        Ok(sel)
    })
}

/// Clears direct paragraph formatting from the selected paragraphs.
pub fn clear_para_props(ed: &mut Editor) -> Result<(), EditError> {
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.transact("Clear paragraph formatting", None, |tx| {
        for id in paras {
            if tx.doc.para(id).is_some_and(|p| !p.props.is_empty()) {
                tx.step(Step::SetProps { para: id, props: ParaProps::default() })?;
            }
        }
        Ok(sel)
    })
}

/// Removes direct character formatting from the selection (links stay).
pub fn clear_format(ed: &mut Editor) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.transact("Clear formatting", None, |tx| {
        for id in paras {
            let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
            let start = if id == a.para { a.offset } else { 0 };
            let end = if id == b.para { b.offset } else { p.text.len() };
            if start >= end {
                continue;
            }
            let mut span = p.slice(start..end);
            for r in &mut span.runs {
                // Only the look goes: links, fields, comments, tracked changes and
                // form controls aren't formatting.
                r.fmt = CharFmt {
                    link: r.fmt.link.clone(),
                    field: r.fmt.field.clone(),
                    comments: r.fmt.comments.clone(),
                    change: r.fmt.change.clone(),
                    control: r.fmt.control,
                    ..Default::default()
                };
            }
            tx.step(Step::Replace { para: id, offset: start, remove: end - start, insert: span })?;
        }
        Ok(sel)
    })
}

/// Turns the selected paragraphs into items of list `list` — or, when they
/// all already are, takes them out of it (the toolbar's list buttons).
pub fn toggle_list(ed: &mut Editor, list: &str) -> Result<(), EditError> {
    let paras = selected_paragraphs(ed);
    let all_in =
        paras.iter().all(|id| ed.doc.para(*id).is_some_and(|p| p.list.as_ref().is_some_and(|l| l.list == list)));
    let sel = ed.sel;
    ed.seal();
    ed.transact(if all_in { "Remove list" } else { "List" }, None, |tx| {
        // A built-in list the document doesn't have yet (older files): add it.
        if !all_in
            && tx.doc.list_def(list).is_none()
            && let Some(def) = crate::ListDef::builtin_def(list)
        {
            tx.step(Step::SetListDef { id: list.into(), def: Some(def) })?;
        }
        for id in paras {
            let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
            let new = if all_in {
                None
            } else {
                let level = p.list.as_ref().map_or(0, |l| l.level);
                Some(ListItem::new(list, level))
            };
            if new != p.list {
                tx.step(Step::SetList { para: id, list: new })?;
            }
        }
        Ok(sel)
    })
}

/// Ticks or unticks checklist item `para`.
pub fn toggle_checked(ed: &mut Editor, para: NodeId) -> Result<(), EditError> {
    let Some(item) = ed.doc.para(para).and_then(|p| p.list.clone()) else { return Ok(()) };
    let sel = ed.sel;
    ed.seal();
    let label = if item.checked { "Untick" } else { "Tick" };
    ed.transact(label, None, |tx| {
        tx.step(Step::SetList { para, list: Some(ListItem { checked: !item.checked, ..item }) })?;
        Ok(sel)
    })
}

/// Which typed shortcuts [`autoformat`] turns into formatting.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Autoformat {
    /// `* `, `- `, `+ ` → bulleted list.
    pub bullets: bool,
    /// `1. `, `1) `, `a. `, `A. ` → numbered list.
    pub numbers: bool,
    /// `[ ] `, `[x] ` → checklist.
    pub checklist: bool,
    /// `# ` … `###### ` → Heading 1–6.
    pub headings: bool,
    /// `> ` → Quote.
    pub quote: bool,
}

impl Default for Autoformat {
    fn default() -> Self {
        Self { bullets: true, numbers: true, checklist: true, headings: true, quote: true }
    }
}

/// What a paragraph that starts with `prefix` (ending in a space) becomes.
enum Shortcut {
    List {
        list: &'static str,
        restart: Option<u32>,
        checked: bool,
    },
    Style(String),
    /// A numbered heading at this outline level.
    Heading(u8),
}

fn shortcut(prefix: &str, opts: &Autoformat) -> Option<Shortcut> {
    let body = prefix.strip_suffix(' ')?;
    let list = |list, restart, checked| Some(Shortcut::List { list, restart, checked });
    match body {
        "*" | "-" | "+" if opts.bullets => return list(crate::ListDef::BULLETS, None, false),
        "[]" | "[ ]" if opts.checklist => return list(crate::ListDef::CHECKLIST, None, false),
        "[x]" | "[X]" if opts.checklist => return list(crate::ListDef::CHECKLIST, None, true),
        ">" if opts.quote => return Some(Shortcut::Style(crate::StyleSheet::QUOTE.into())),
        _ => {}
    }
    if opts.headings && !body.is_empty() && body.len() <= 6 && body.chars().all(|c| c == '#') {
        return Some(Shortcut::Style(crate::StyleSheet::heading(body.len() as u8)));
    }
    if opts.numbers {
        let n = body.strip_suffix('.').or_else(|| body.strip_suffix(')'))?;
        if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) {
            let v: u32 = n.parse().ok()?;
            return list(crate::ListDef::NUMBERS, (v != 1).then_some(v), false);
        }
        if n == "a" || n == "A" {
            return list(crate::ListDef::LETTERS, None, false);
        }
    }
    None
}

/// After a space is typed: turns a Markdown-style start of a paragraph
/// (`* `, `1. `, `# `, `> `, `[ ] `) into the list or style it stands for,
/// removing the typed characters. Its own undo step, so Ctrl+Z gives the
/// characters back. Returns whether it did anything.
pub fn autoformat(ed: &mut Editor, opts: &Autoformat) -> Result<bool, EditError> {
    if !ed.sel.is_collapsed() {
        return Ok(false);
    }
    let at = ed.sel.focus;
    let Some(p) = ed.doc.para(at.para) else { return Ok(false) };
    let Some(prefix) = p.text.get(..at.offset) else { return Ok(false) };
    if prefix.len() > 8 {
        return Ok(false);
    }
    // In a list item: a different list ("1. " in a bulleted list) or a
    // heading takes over, at the item's level.
    let in_list = p.list.clone();
    let Some(sc) = shortcut(prefix, opts) else { return Ok(false) };
    // A heading stays a heading; "1. " in one numbers it (1, 1.1… counted apart from lists in the text).
    let heading = ed.doc.styles.get(&p.style).and_then(|s| s.outline_level);
    let sc = match (&sc, heading) {
        (_, None) if p.style != crate::StyleSheet::NORMAL && p.style != crate::StyleSheet::QUOTE => return Ok(false),
        (Shortcut::List { list, .. }, Some(level))
            if *list == crate::ListDef::NUMBERS || *list == crate::ListDef::LETTERS =>
        {
            Shortcut::Heading(level)
        }
        (_, Some(_)) => return Ok(false),
        _ => sc,
    };
    let (para, len) = (at.para, prefix.len());
    ed.seal();
    ed.transact("Autoformat", None, |tx| {
        tx.step(Step::Replace { para, offset: 0, remove: len, insert: crate::Span::default() })?;
        match sc {
            Shortcut::List { list, restart, checked } => {
                if tx.doc.list_def(list).is_none()
                    && let Some(def) = crate::ListDef::builtin_def(list)
                {
                    tx.step(Step::SetListDef { id: list.into(), def: Some(def) })?;
                }
                let level = in_list.as_ref().map_or(0, |l| l.level);
                let item = ListItem { restart, checked, ..ListItem::new(list, level) };
                tx.step(Step::SetList { para, list: Some(item) })?;
            }
            Shortcut::Style(style) => {
                if in_list.is_some() {
                    tx.step(Step::SetList { para, list: None })?;
                }
                tx.step(Step::SetStyle { para, style })?;
            }
            Shortcut::Heading(level) => {
                let list = crate::ListDef::HEADINGS;
                if tx.doc.list_def(list).is_none()
                    && let Some(def) = crate::ListDef::builtin_def(list)
                {
                    tx.step(Step::SetListDef { id: list.into(), def: Some(def) })?;
                }
                tx.step(Step::SetList { para, list: Some(ListItem::new(list, level.saturating_sub(1))) })?;
            }
        }
        Ok(Selection::caret(Pos { para, offset: 0 }))
    })?;
    ed.seal();
    Ok(true)
}

/// Moves the selected list items `delta` levels in (positive) or out.
/// Returns false when nothing selected is a list item.
pub fn change_list_level(ed: &mut Editor, delta: i8) -> Result<bool, EditError> {
    let paras: Vec<_> =
        selected_paragraphs(ed).into_iter().filter(|id| ed.doc.para(*id).is_some_and(|p| p.list.is_some())).collect();
    if paras.is_empty() {
        return Ok(false);
    }
    let sel = ed.sel;
    ed.seal();
    ed.transact(if delta > 0 { "Indent" } else { "Outdent" }, None, |tx| {
        for id in paras {
            let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
            let Some(item) = p.list.clone() else { continue };
            let level = (item.level as i16 + delta as i16).clamp(0, crate::lists::MAX_LEVEL as i16) as u8;
            if level != item.level {
                tx.step(Step::SetList { para: id, list: Some(ListItem { level, ..item }) })?;
            }
        }
        Ok(sel)
    })?;
    Ok(true)
}

/// Shift+Enter in a list: a new paragraph that's more of the same item (no
/// number of its own, lined up with the item's text), for more text,
/// pictures or a list inside it. Enter from there starts the next item.
/// Returns false outside lists.
pub fn continue_item(ed: &mut Editor) -> Result<bool, EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    let Some(item) = ed.doc.para(b.para).and_then(|p| p.list.clone()) else { return Ok(false) };
    ed.seal();
    ed.transact("More of the item", None, |tx| {
        let at = delete_range(tx, a, b)?;
        let new = split_at(tx, at)?;
        tx.step(Step::SetList {
            para: new.para,
            list: Some(ListItem { follow: true, restart: None, checked: false, ..item }),
        })?;
        Ok(Selection::caret(new))
    })?;
    ed.seal();
    Ok(true)
}

/// Restarts numbering at the caret's list item (or continues it again).
pub fn restart_numbering(ed: &mut Editor, at: Option<u32>) -> Result<(), EditError> {
    let id = ed.sel.focus.para;
    let Some(item) = ed.doc.para(id).and_then(|p| p.list.clone()) else { return Ok(()) };
    let sel = ed.sel;
    ed.transact("Restart numbering", None, |tx| {
        tx.step(Step::SetList { para: id, list: Some(ListItem { restart: at, ..item }) })?;
        Ok(sel)
    })
}

/// A format with "reset" values (transparent colour, empty link or font,
/// size 0) turned back into "as the style says".
pub fn normalize_fmt(mut f: CharFmt) -> CharFmt {
    if f.color.is_some_and(|c| c.0[3] == 0) {
        f.color = None;
    }
    if f.highlight.is_some_and(|c| c.0[3] == 0) {
        f.highlight = None;
    }
    if f.link.as_deref().is_some_and(|l| l.trim().is_empty()) {
        f.link = None;
    }
    if f.font.as_deref().is_some_and(|l| l.trim().is_empty()) {
        f.font = None;
    }
    if f.size.is_some_and(|s| s <= 0.0) {
        f.size = None;
    }
    if f.vert == Some(crate::VertAlign::Baseline) {
        f.vert = None;
    }
    f
}

/// Applies direct formatting `patch` to the selected text. Reset values
/// (see [`normalize_fmt`]) remove a property.
pub fn apply_format(ed: &mut Editor, patch: &CharFmt, label: &str) -> Result<(), EditError> {
    apply_format_merged(ed, patch, label, None)
}

/// Like [`apply_format`]; changes with the same `merge` key (e.g. dragging
/// in a colour picker) undo as one step.
pub fn apply_format_merged(
    ed: &mut Editor,
    patch: &CharFmt,
    label: &str,
    merge: Option<MergeKey>,
) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.transact(label, merge, |tx| {
        for id in paras {
            let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
            let start = if id == a.para { a.offset } else { 0 };
            let end = if id == b.para { b.offset } else { p.text.len() };
            if start >= end {
                continue;
            }
            let mut span = p.slice(start..end);
            for r in &mut span.runs {
                r.fmt = normalize_fmt(r.fmt.with(patch));
            }
            tx.step(Step::Replace { para: id, offset: start, remove: end - start, insert: span })?;
        }
        Ok(sel)
    })
}

/// Direct formatting shared by the whole selection (for toolbar toggles):
/// a property is set only when every selected character has it.
pub fn common_format(ed: &Editor) -> CharFmt {
    let (a, b) = ed.doc.ordered(ed.sel);
    if a == b {
        return ed.doc.para(a.para).map(|p| p.fmt_at(a.offset)).unwrap_or_default();
    }
    let mut out: Option<CharFmt> = None;
    for id in selected_paragraphs(ed) {
        let Some(p) = ed.doc.para(id) else { continue };
        let start = if id == a.para { a.offset } else { 0 };
        let end = if id == b.para { b.offset } else { p.text.len() };
        for (r, f) in p.run_ranges() {
            if r.end <= start || r.start >= end {
                continue;
            }
            out = Some(match out {
                None => f.clone(),
                Some(o) => CharFmt {
                    bold: o.bold.filter(|v| f.bold == Some(*v)),
                    italic: o.italic.filter(|v| f.italic == Some(*v)),
                    underline: o.underline.filter(|v| f.underline == Some(*v)),
                    strike: o.strike.filter(|v| f.strike == Some(*v)),
                    font: o.font.filter(|v| f.font.as_ref() == Some(v)),
                    size: o.size.filter(|v| f.size == Some(*v)),
                    color: o.color.filter(|v| f.color == Some(*v)),
                    highlight: o.highlight.filter(|v| f.highlight == Some(*v)),
                    field: None,
                    vert: o.vert.filter(|v| f.vert == Some(*v)),
                    comments: Vec::new(),
                    change: None,
                    control: None,
                    link: o.link.filter(|v| f.link.as_ref() == Some(v)),
                },
            });
        }
    }
    out.unwrap_or_default()
}

/// The word around `pos` (for double-click selection).
pub fn word_at(ed: &Editor, pos: Pos) -> Selection {
    let Some(p) = ed.doc.para(pos.para) else { return Selection::caret(pos) };
    for (i, w) in p.text.split_word_bound_indices() {
        if pos.offset >= i && pos.offset <= i + w.len() && !w.chars().all(char::is_whitespace) {
            return Selection { anchor: Pos::new(p.id, i), focus: Pos::new(p.id, i + w.len()) };
        }
    }
    Selection::caret(pos)
}

/// The next / previous word boundary from `pos`, crossing paragraphs.
pub fn word_move(ed: &Editor, pos: Pos, forward: bool) -> Pos {
    let Some(p) = ed.doc.para(pos.para) else { return pos };
    if forward {
        if pos.offset >= p.text.len() {
            return ed.doc.next_para(p.id).map_or(pos, |n| Pos::new(n, 0));
        }
        Pos::new(p.id, step_forward(&p.text, pos.offset, true))
    } else {
        if pos.offset == 0 {
            return ed.doc.prev_para(p.id).and_then(|n| ed.doc.para(n)).map_or(pos, |n| Pos::new(n.id, n.text.len()));
        }
        Pos::new(p.id, step_back(&p.text, pos.offset, true))
    }
}

/// The grapheme boundary next to `pos` in logical order, crossing paragraphs.
pub fn char_move(ed: &Editor, pos: Pos, forward: bool) -> Pos {
    let Some(p) = ed.doc.para(pos.para) else { return pos };
    if forward {
        if pos.offset >= p.text.len() {
            return ed.doc.next_para(p.id).map_or(pos, |n| Pos::new(n, 0));
        }
        Pos::new(p.id, step_forward(&p.text, pos.offset, false))
    } else {
        if pos.offset == 0 {
            return ed.doc.prev_para(p.id).and_then(|n| ed.doc.para(n)).map_or(pos, |n| Pos::new(n.id, n.text.len()));
        }
        Pos::new(p.id, step_back(&p.text, pos.offset, false))
    }
}

/// Adds or replaces a paragraph style definition (the style editor).
pub fn define_style(ed: &mut Editor, def: crate::ParaStyle) -> Result<(), EditError> {
    let sel = ed.sel;
    let id = def.id.clone();
    ed.seal();
    ed.transact("Modify style", Some(MergeKey::Style(id.clone())), |tx| {
        tx.step(Step::SetStyleDef { id, def: Some(def) })?;
        Ok(sel)
    })
}

/// Format › Styles › Update to match: style `id` takes the look of the
/// caret's paragraph and the selected text (their direct formatting), and
/// that formatting is taken off them (they now just use the style).
pub fn update_style_from_selection(ed: &mut Editor, id: &str) -> Result<(), EditError> {
    let Some(mut def) = ed.doc.styles.get(id).cloned() else { return Err(EditError::NotHere) };
    let para = ed.doc.para(ed.sel.focus.para).ok_or(EditError::NoParagraph(ed.sel.focus.para))?;
    let props = ParaProps { break_change: None, ..para.props.clone() };
    let fmt = common_format(ed);
    // Only the look: not links, fields, comments or tracked changes.
    let look = CharFmt { link: None, field: None, comments: Vec::new(), change: None, ..fmt };
    def.para = def.para.with(&props);
    def.char = def.char.with(&look);
    let paras: Vec<NodeId> =
        selected_paragraphs(ed).into_iter().filter(|p| ed.doc.para(*p).is_some_and(|p| p.style == id)).collect();
    let sel = ed.sel;
    let style_id = id.to_string();
    ed.seal();
    ed.transact("Update style", None, |tx| {
        tx.step(Step::SetStyleDef { id: style_id, def: Some(def) })?;
        for pid in paras {
            let p = tx.doc.para(pid).ok_or(EditError::NoParagraph(pid))?;
            let keep = ParaProps { break_change: p.props.break_change.clone(), ..Default::default() };
            let mut span = p.slice(0..p.text.len());
            tx.step(Step::SetProps { para: pid, props: keep })?;
            for r in &mut span.runs {
                r.fmt = CharFmt {
                    link: r.fmt.link.clone(),
                    field: r.fmt.field.clone(),
                    comments: r.fmt.comments.clone(),
                    change: r.fmt.change.clone(),
                    ..Default::default()
                };
            }
            let len = span.text.len();
            tx.step(Step::Replace { para: pid, offset: 0, remove: len, insert: span })?;
        }
        Ok(sel)
    })?;
    ed.seal();
    Ok(())
}

/// Deletes user style `id`; paragraphs using it become Normal.
pub fn delete_style(ed: &mut Editor, id: &str) -> Result<(), EditError> {
    let users: Vec<NodeId> = ed.doc.paragraphs().filter(|p| p.style == id).map(|p| p.id).collect();
    let sel = ed.sel;
    ed.seal();
    ed.transact("Delete style", None, |tx| {
        for p in users {
            tx.step(Step::SetStyle { para: p, style: crate::StyleSheet::NORMAL.into() })?;
        }
        tx.step(Step::SetStyleDef { id: id.into(), def: None })?;
        Ok(sel)
    })
}

/// A new paragraph style based on the caret paragraph's look (style,
/// direct paragraph and character formatting), applied to the selection.
pub fn new_style_from_selection(ed: &mut Editor, name: &str) -> Result<String, EditError> {
    let p = ed.doc.para(ed.sel.focus.para).ok_or(EditError::NoParagraph(ed.sel.focus.para))?;
    let base = ed.doc.styles.get(&p.style).cloned().unwrap_or_default();
    let fmt = p.runs.first().map(|r| r.fmt.clone()).unwrap_or_default();
    let mut id: String = name.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    if id.is_empty() {
        id = "style".into();
    }
    let stem = id.clone();
    let mut n = 2;
    while ed.doc.styles.get(&id).is_some() {
        id = format!("{stem}{n}");
        n += 1;
    }
    let def = crate::ParaStyle {
        id: id.clone(),
        name: name.trim().to_string(),
        based_on: Some(p.style.clone()),
        next: base.next.clone(),
        outline_level: base.outline_level,
        list: base.list.clone(),
        para: p.props.clone(),
        char: CharFmt { link: None, ..fmt },
    };
    let paras = selected_paragraphs(ed);
    let sel = ed.sel;
    ed.seal();
    ed.transact("New style", None, |tx| {
        tx.step(Step::SetStyleDef { id: id.clone(), def: Some(def) })?;
        for pid in paras {
            tx.step(Step::SetStyle { para: pid, style: id.clone() })?;
            if tx.doc.para(pid).is_some_and(|p| !p.props.is_empty()) {
                tx.step(Step::SetProps { para: pid, props: ParaProps::default() })?;
            }
        }
        Ok(sel)
    })?;
    Ok(id)
}

/// Ctrl+Enter: continues on a new page (splits the paragraph at the caret;
/// the new paragraph starts a page).
pub fn insert_page_break(ed: &mut Editor) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    ed.seal();
    ed.transact("Page break", None, |tx| {
        let at = delete_range(tx, a, b)?;
        let new = split_at(tx, at)?;
        let p = tx.doc.para(new.para).ok_or(EditError::NoParagraph(new.para))?;
        let props = p.props.with(&ParaProps { page_break_before: Some(true), ..Default::default() });
        tx.step(Step::SetProps { para: new.para, props })?;
        Ok(Selection::caret(new))
    })
}

/// Insert › Column break: the text after the caret starts at the top of
/// the next column.
pub fn insert_column_break(ed: &mut Editor) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    ed.seal();
    ed.transact("Column break", None, |tx| {
        let at = delete_range(tx, a, b)?;
        let new = split_at(tx, at)?;
        let p = tx.doc.para(new.para).ok_or(EditError::NoParagraph(new.para))?;
        let props = p.props.with(&ParaProps { column_break_before: Some(true), ..Default::default() });
        tx.step(Step::SetProps { para: new.para, props })?;
        Ok(Selection::caret(new))
    })
}

/// Starts a new section at the caret (splitting the paragraph there when
/// the caret is inside it). The new section copies the current page setup.
pub fn insert_section_break(ed: &mut Editor, start: crate::SectionStart) -> Result<(), EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    ed.seal();
    ed.transact("Section break", None, |tx| {
        let mut at = delete_range(tx, a, b)?;
        if at.offset > 0 {
            at = split_at(tx, at)?;
        }
        let (c, bi) = tx.doc.locate(at.para).ok_or(EditError::NoParagraph(at.para))?;
        let Container::Section(si) = c else { return Err(EditError::NotHere) };
        if bi == 0 && si == 0 {
            // Nothing before it: an empty paragraph goes first so there are two sections.
            let id = tx.doc.alloc_id();
            let style = crate::StyleSheet::NORMAL.to_string();
            tx.step(Step::InsertBlock {
                container: Container::Section(0),
                index: 0,
                block: crate::Block::Paragraph(crate::Paragraph::new(id, style, "")),
            })?;
            let page = tx.doc.sections[0].page.clone();
            let sid = tx.doc.alloc_id();
            tx.step(Step::SplitSection { section: 0, at: 1, id: sid, start, page, frames: Vec::new() })?;
        } else if bi > 0 {
            let page = tx.doc.sections[si].page.clone();
            let sid = tx.doc.alloc_id();
            tx.step(Step::SplitSection { section: si, at: bi, id: sid, start, page, frames: Vec::new() })?;
        } else {
            // Already at the start of a section: just change how it starts.
            let page = tx.doc.sections[si].page.clone();
            tx.step(Step::SetSection { section: si, start, page })?;
        }
        Ok(Selection::caret(at))
    })
}

/// Insert › Title page: a page of its own at the start of the document, its
/// text centred top to bottom — the title, a subtitle and the author — then
/// the rest starts on a new page. The title is selected, ready to type over.
pub fn insert_title_page(ed: &mut Editor) -> Result<(), EditError> {
    use crate::{Block, Paragraph, StyleSheet};
    ed.seal();
    let title = if ed.doc.meta.title.trim().is_empty() { "Title".to_string() } else { ed.doc.meta.title.clone() };
    let author = if ed.doc.meta.author.trim().is_empty() { "Author".to_string() } else { ed.doc.meta.author.clone() };
    ed.transact("Title page", None, |tx| {
        let lines =
            [(StyleSheet::TITLE, title), (StyleSheet::SUBTITLE, "Subtitle".to_string()), (StyleSheet::NORMAL, author)];
        let mut first = None;
        for (i, (style, text)) in lines.into_iter().enumerate() {
            let id = tx.doc.alloc_id();
            let mut p = Paragraph::new(id, style, text.clone());
            p.props.align = Some(crate::Align::Center);
            first.get_or_insert((id, text.len()));
            tx.step(Step::InsertBlock { container: Container::Section(0), index: i, block: Block::Paragraph(p) })?;
        }
        let page = tx.doc.sections[0].page.clone();
        let start = tx.doc.sections[0].start;
        let sid = tx.doc.alloc_id();
        tx.step(Step::SplitSection {
            section: 0,
            at: 3,
            id: sid,
            start: crate::SectionStart::NewPage,
            page: page.clone(),
            frames: Vec::new(),
        })?;
        let centred = crate::PageSetup { v_align: crate::PageVAlign::Center, ..page };
        tx.step(Step::SetSection { section: 0, start, page: centred })?;
        let (id, len) = first.expect("three lines");
        Ok(Selection { anchor: Pos::new(id, 0), focus: Pos::new(id, len) })
    })
}

/// Changes section `section`'s page setup and start (Page setup dialog).
pub fn set_section(
    ed: &mut Editor,
    section: usize,
    start: crate::SectionStart,
    page: crate::PageSetup,
) -> Result<(), EditError> {
    let sel = ed.sel;
    ed.transact("Page setup", Some(MergeKey::Control(format!("section-{section}"))), |tx| {
        tx.step(Step::SetSection { section, start, page })?;
        Ok(sel)
    })
}

/// Changes the document's details (title, author…); typing in the title
/// merges into one undo step.
pub fn set_meta(ed: &mut Editor, meta: crate::Meta, label: &str) -> Result<(), EditError> {
    if ed.doc.meta == meta {
        return Ok(());
    }
    let sel = ed.sel;
    ed.transact(label, Some(MergeKey::Control(format!("meta-{label}"))), |tx| {
        tx.step(Step::SetMeta { meta: Box::new(meta) })?;
        Ok(sel)
    })
}

/// Removes the section break before section `section` (its content joins the previous section).
pub fn remove_section_break(ed: &mut Editor, section: usize) -> Result<(), EditError> {
    if section == 0 {
        return Ok(());
    }
    let sel = ed.sel;
    ed.seal();
    ed.transact("Remove section break", None, |tx| {
        if !tx.doc.sections[section].headers.is_empty() {
            tx.step(Step::SetHeaders { section, headers: Vec::new() })?;
        }
        tx.step(Step::JoinSection { section: section - 1 })?;
        Ok(sel)
    })
}

/// Replaces `range` of paragraph `para` with `text` (in the format at the
/// start of the range): applying a spelling or AI suggestion.
pub fn replace_text(
    ed: &mut Editor,
    para: NodeId,
    range: std::ops::Range<usize>,
    text: &str,
    label: &str,
) -> Result<(), EditError> {
    let p = ed.doc.para(para).ok_or(EditError::NoParagraph(para))?;
    let fmt = p.fmt_at(range.start + usize::from(range.start < range.end));
    ed.seal();
    ed.transact(label, None, |tx| {
        tx.step(Step::Replace { para, offset: range.start, remove: range.len(), insert: Span::plain(text, fmt) })?;
        Ok(Selection::caret(Pos::new(para, range.start + text.len())))
    })
}

/// Applies byte-range `edits` (non-overlapping, in order) to paragraph
/// `para` as one undoable step: unchanged text keeps its formatting.
pub fn apply_edits(
    ed: &mut Editor,
    para: NodeId,
    edits: &[(std::ops::Range<usize>, String)],
    label: &str,
) -> Result<(), EditError> {
    let p = ed.doc.para(para).ok_or(EditError::NoParagraph(para))?.clone();
    ed.seal();
    ed.transact(label, None, |tx| {
        for (r, text) in edits.iter().rev() {
            let fmt = p.fmt_at(r.start + usize::from(r.start < r.end));
            tx.step(Step::Replace { para, offset: r.start, remove: r.len(), insert: Span::plain(text.as_str(), fmt) })?;
        }
        let end = tx.doc.para(para).map_or(0, |q| q.text.len());
        Ok(Selection::caret(Pos::new(para, end)))
    })
}

/// Insert › Field (page number, date, title…), and citations and
/// cross-references: the field replaces the selection.
pub fn insert_field(ed: &mut Editor, field: crate::Field, label: &str) -> Result<(), EditError> {
    let (start, end) = ed.doc.ordered(ed.sel);
    let base = ed.doc.para(start.para).map(|p| p.fmt_at(start.offset)).unwrap_or_default();
    let base = crate::review::typed_fmt(&ed.doc, base, None);
    let fmt = crate::CharFmt { field: Some(field), link: None, ..base };
    ed.seal();
    ed.transact(label, None, |tx| {
        if start != end {
            delete_range(tx, start, end)?;
        }
        tx.step(Step::Replace {
            para: start.para,
            offset: start.offset,
            remove: 0,
            insert: crate::Span::plain(crate::Field::STR, fmt),
        })?;
        Ok(Selection::caret(Pos::new(start.para, start.offset + crate::Field::STR.len())))
    })?;
    ed.seal();
    Ok(())
}

/// Words and characters in the selection (none when nothing is selected).
pub fn selection_counts(ed: &Editor) -> Option<(usize, usize)> {
    if ed.sel.is_collapsed() {
        return None;
    }
    let (a, b) = ed.doc.ordered(ed.sel);
    let (mut words, mut chars) = (0, 0);
    for id in selected_paragraphs(ed) {
        let Some(p) = ed.doc.para(id) else { continue };
        let start = if id == a.para { a.offset } else { 0 };
        let end = if id == b.para { b.offset } else { p.text.len() };
        let t = p.text.get(start.min(end)..end).unwrap_or("");
        words += t.unicode_words().count();
        chars += t.chars().count();
    }
    Some((words, chars))
}

/// Every place `query` occurs (paragraph, byte range), in reading order.
/// `case`: match capitals exactly; `whole`: only whole words.
pub fn find_all(doc: &Document, query: &str, case: bool, whole: bool) -> Vec<(NodeId, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let q = if case { query.to_string() } else { query.to_lowercase() };
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    for p in doc.paragraphs() {
        let hay = if case { p.text.clone() } else { p.text.to_lowercase() };
        // Lower-casing changed the length (rare letters): match exactly instead.
        let (hay, q) = if hay.len() == p.text.len() { (hay, q.clone()) } else { (p.text.clone(), query.to_string()) };
        let mut from = 0;
        while let Some(i) = hay[from..].find(&q) {
            let start = from + i;
            let end = start + q.len();
            let ok = !whole || (!word(p.text[..start].chars().next_back()) && !word(p.text[end..].chars().next()));
            if ok && !p.text[start..end].contains(crate::Field::CHAR) {
                out.push((p.id, start..end));
            }
            from = start + q.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Edit › Find with "Regular expression" on: every match of `pattern`
/// (`\b` around it for whole words), or why the pattern isn't valid.
/// Empty matches and matches over a field are left out.
pub fn find_regex(
    doc: &Document,
    pattern: &str,
    case: bool,
    whole: bool,
) -> Result<Vec<(NodeId, std::ops::Range<usize>)>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = build_regex(pattern, case, whole)?;
    let mut out = Vec::new();
    for p in doc.paragraphs() {
        for m in re.find_iter(&p.text) {
            if !m.is_empty() && !m.as_str().contains(crate::Field::CHAR) {
                out.push((p.id, m.range()));
            }
        }
    }
    Ok(out)
}

fn build_regex(pattern: &str, case: bool, whole: bool) -> Result<regex::Regex, String> {
    let pattern = if whole { format!(r"\b(?:{pattern})\b") } else { pattern.to_string() };
    regex::RegexBuilder::new(&pattern).case_insensitive(!case).build().map_err(|e| match e {
        regex::Error::Syntax(s) => {
            s.lines().last().unwrap_or("not a valid pattern").trim_start_matches("error: ").into()
        }
        _ => "the pattern is too big".into(),
    })
}

/// What each regex match in `found` becomes: `with` with `$1`, `${name}`
/// (and `$0` for the whole match) filled in from that match.
pub fn regex_replacements(
    doc: &Document,
    pattern: &str,
    case: bool,
    whole: bool,
    with: &str,
    found: &[(NodeId, std::ops::Range<usize>)],
) -> Result<Vec<String>, String> {
    let re = build_regex(pattern, case, whole)?;
    Ok(found
        .iter()
        .map(|(para, r)| {
            let text = doc.para(*para).map_or("", |p| p.text.as_str());
            let mut out = String::new();
            match re.captures_at(text, r.start).filter(|c| c.get(0).is_some_and(|m| m.range() == *r)) {
                Some(caps) => caps.expand(with, &mut out),
                None => out.push_str(with),
            }
            out
        })
        .collect())
}

/// Edit › Replace all: every match becomes `with`, as one undo step.
pub fn replace_all(
    ed: &mut Editor,
    found: &[(NodeId, std::ops::Range<usize>)],
    with: &str,
) -> Result<usize, EditError> {
    let each: Vec<String> = found.iter().map(|_| with.to_string()).collect();
    replace_each(ed, found, &each)
}

/// Replaces each match in `found` with its own text in `with` (same
/// order), as one undo step.
pub fn replace_each(
    ed: &mut Editor,
    found: &[(NodeId, std::ops::Range<usize>)],
    with: &[String],
) -> Result<usize, EditError> {
    if found.is_empty() {
        return Ok(0);
    }
    let sel = ed.sel;
    ed.seal();
    ed.transact("Replace all", None, |tx| {
        // From the end backwards so earlier offsets stay right.
        for ((para, r), with) in found.iter().zip(with).rev() {
            let p = tx.doc.para(*para).ok_or(EditError::NoParagraph(*para))?;
            let fmt = p.fmt_at(r.start + usize::from(r.start < r.end));
            let fmt = crate::review::typed_fmt(tx.doc, fmt, None);
            if tx.doc.meta.track_changes {
                delete_range(tx, Pos::new(*para, r.start), Pos::new(*para, r.end))?;
            } else {
                tx.step(Step::Replace { para: *para, offset: r.start, remove: r.len(), insert: Span::default() })?;
            }
            tx.step(Step::Replace { para: *para, offset: r.start, remove: 0, insert: Span::plain(with, fmt) })?;
        }
        Ok(sel)
    })?;
    ed.seal();
    ed.sel = Selection { anchor: ed.doc.clamp(sel.anchor), focus: ed.doc.clamp(sel.focus) };
    Ok(found.len())
}

/// Edit › Filter lines: removes the paragraphs of the main text that have
/// a match in `found` (or, with `keep`, the ones that don't), as one undo
/// step; returns how many went. Tables, headers, notes and text boxes are
/// left alone. While tracking changes the text is marked deleted instead.
pub fn filter_paragraphs(
    ed: &mut Editor,
    found: &[(NodeId, std::ops::Range<usize>)],
    keep: bool,
) -> Result<usize, EditError> {
    let hit: std::collections::HashSet<NodeId> = found.iter().map(|(p, _)| *p).collect();
    // (section, block index, paragraph, text length), in reading order.
    let mut gone: Vec<(usize, usize, NodeId, usize)> = Vec::new();
    for (si, s) in ed.doc.sections.iter().enumerate() {
        for (i, b) in s.blocks.iter().enumerate() {
            if let Block::Paragraph(p) = b
                && hit.contains(&p.id) != keep
            {
                gone.push((si, i, p.id, p.text.len()));
            }
        }
    }
    if gone.is_empty() {
        return Ok(0);
    }
    let n = gone.len();
    ed.seal();
    ed.transact(if keep { "Keep matching lines" } else { "Remove matching lines" }, None, |tx| {
        if tx.doc.meta.track_changes {
            for &(_, _, id, len) in &gone {
                delete_range(tx, Pos::new(id, 0), Pos::new(id, len))?;
            }
            return Ok(Selection::caret(tx.doc.clamp(tx.doc.start())));
        }
        for &(si, i, _, _) in gone.iter().rev() {
            tx.step(Step::RemoveBlock { container: Container::Section(si), index: i })?;
        }
        // Never leave a section without a paragraph.
        for si in 0..tx.doc.sections.len() {
            if tx.doc.sections[si].blocks.is_empty() {
                let id = tx.doc.alloc_id();
                let block = Block::Paragraph(crate::Paragraph::new(id, crate::StyleSheet::NORMAL, ""));
                tx.step(Step::InsertBlock { container: Container::Section(si), index: 0, block })?;
            }
        }
        Ok(Selection::caret(tx.doc.start()))
    })?;
    ed.seal();
    Ok(n)
}

/// Adds a caption under picture or table `block` ("Figure 2: " or
/// "Table 2: ", numbered after the captions before it) and puts the caret
/// at its end, ready to type. A block that already has a caption: the
/// caret goes to it. Captions are what References › List of figures lists.
pub fn add_caption(ed: &mut Editor, block: NodeId) -> Result<(), EditError> {
    let (c, i) = ed.doc.locate(block).ok_or(EditError::NotHere)?;
    let kind = match ed.doc.blocks(&c).and_then(|b| b.get(i)) {
        Some(Block::Table(_)) => "Table",
        Some(_) => "Figure",
        None => return Err(EditError::NotHere),
    };
    if let Some(Block::Paragraph(p)) = ed.doc.blocks(&c).and_then(|b| b.get(i + 1))
        && p.style == crate::StyleSheet::CAPTION
    {
        ed.sel = Selection::caret(Pos::new(p.id, p.text.len()));
        return Ok(());
    }
    // The first paragraph after the block: captions before it count.
    let mut after = None;
    let mut cur = block;
    while let Some((cc, ci)) = neighbour_block(&ed.doc, cur, true) {
        match ed.doc.blocks(&cc).and_then(|b| b.get(ci)) {
            Some(Block::Paragraph(p)) => {
                after = Some(p.id);
                break;
            }
            Some(b) => cur = b.id(),
            None => break,
        }
    }
    let n = ed
        .doc
        .paragraphs()
        .take_while(|p| Some(p.id) != after)
        .filter(|p| p.style == crate::StyleSheet::CAPTION && p.text.starts_with(kind))
        .count()
        + 1;
    let text = format!("{kind} {n}: ");
    let id = ed.doc.alloc_id();
    let end = text.len();
    ed.seal();
    ed.transact("Add caption", None, |tx| {
        tx.step(Step::InsertBlock {
            container: c.clone(),
            index: i + 1,
            block: Block::Paragraph(crate::Paragraph::new(id, crate::StyleSheet::CAPTION, text)),
        })?;
        Ok(Selection::caret(Pos::new(id, end)))
    })?;
    ed.seal();
    Ok(())
}

/// The equation at `pos` (the position before its character): its LaTeX
/// and whether it's on a line of its own.
pub fn equation_at(doc: &crate::Document, pos: Pos) -> Option<(String, bool)> {
    let p = doc.para(pos.para)?;
    if p.text.get(pos.offset..pos.offset + crate::Field::CHAR.len_utf8()) != Some(crate::Field::STR) {
        return None;
    }
    match p.fmt_at(pos.offset + crate::Field::CHAR.len_utf8()).field {
        Some(crate::Field::Equation { latex, display, .. }) => Some((latex, display)),
        _ => None,
    }
}

/// The field on the character at `pos`, if there is one.
pub fn field_at(doc: &crate::Document, pos: Pos) -> Option<crate::Field> {
    let p = doc.para(pos.para)?;
    if p.text.get(pos.offset..pos.offset + crate::Field::CHAR.len_utf8()) != Some(crate::Field::STR) {
        return None;
    }
    p.fmt_at(pos.offset + crate::Field::CHAR.len_utf8()).field
}

/// Puts `field` in place of the field at `pos`, keeping its formatting (one undo step).
pub fn set_field(ed: &mut Editor, pos: Pos, field: crate::Field, label: &str) -> Result<bool, EditError> {
    if field_at(&ed.doc, pos).is_none() {
        return Ok(false);
    }
    let w = crate::Field::CHAR.len_utf8();
    let fmt = ed.doc.para(pos.para).map(|p| p.fmt_at(pos.offset + w)).unwrap_or_default();
    let fmt = CharFmt { field: Some(field), ..fmt };
    let sel = ed.sel;
    ed.seal();
    ed.transact(label, None, |tx| {
        tx.step(Step::Replace {
            para: pos.para,
            offset: pos.offset,
            remove: w,
            insert: Span::plain(crate::Field::STR, fmt),
        })?;
        Ok(sel)
    })?;
    ed.seal();
    Ok(true)
}

/// Changes the equation at `pos` (one undo step). Word's own markup for it
/// is dropped (it's written again from the LaTeX).
pub fn set_equation(ed: &mut Editor, pos: Pos, latex: &str, display: bool) -> Result<bool, EditError> {
    let Some((old, was)) = equation_at(&ed.doc, pos) else { return Ok(false) };
    if old == latex && was == display {
        return Ok(false);
    }
    let w = crate::Field::CHAR.len_utf8();
    let fmt = ed.doc.para(pos.para).map(|p| p.fmt_at(pos.offset + w)).unwrap_or_default();
    let fmt = CharFmt {
        field: Some(crate::Field::Equation { latex: latex.to_string(), display, omml: String::new() }),
        ..fmt
    };
    let sel = ed.sel;
    ed.seal();
    ed.transact("Edit equation", None, |tx| {
        tx.step(Step::Replace {
            para: pos.para,
            offset: pos.offset,
            remove: w,
            insert: Span::plain(crate::Field::STR, fmt),
        })?;
        Ok(sel)
    })?;
    ed.seal();
    Ok(true)
}

/// Removes the equation at `pos` (one undo step).
pub fn remove_equation(ed: &mut Editor, pos: Pos) -> Result<bool, EditError> {
    if equation_at(&ed.doc, pos).is_none() {
        return Ok(false);
    }
    ed.seal();
    ed.transact("Delete equation", None, |tx| {
        tx.step(Step::Replace {
            para: pos.para,
            offset: pos.offset,
            remove: crate::Field::CHAR.len_utf8(),
            insert: Span::default(),
        })?;
        Ok(Selection::caret(pos))
    })?;
    ed.seal();
    Ok(true)
}


/// A divider: an empty paragraph with a thin line under it.
pub fn divider_props() -> ParaProps {
    let line = crate::Border { width: 0.75, color: crate::Rgba::rgb(0xc4, 0xc9, 0xd0) };
    ParaProps { borders: Some(crate::ParaBorders { bottom: Some(line), ..Default::default() }), ..Default::default() }
}

/// Whether `p` is a divider (see [`divider_props`]).
pub fn is_divider(p: &crate::Paragraph) -> bool {
    p.text.is_empty()
        && p.list.is_none()
        && p.props
            .borders
            .as_ref()
            .is_some_and(|b| b.bottom.is_some() && b.top.is_none() && b.left.is_none() && b.right.is_none())
}

/// Puts a divider line in at the caret: the caret's paragraph becomes one
/// when it's empty, else one goes after it; typing carries on below it.
pub fn insert_divider(ed: &mut Editor) -> Result<(), EditError> {
    ed.seal();
    let at = ed.sel.focus;
    let empty = ed.doc.para(at.para).is_some_and(|p| p.text.is_empty());
    ed.transact("Divider", None, |tx| {
        let line = if empty {
            at.para
        } else {
            let end = tx.doc.para(at.para).map_or(0, |p| p.text.len());
            split_at(tx, Pos::new(at.para, end))?.para
        };
        tx.step(Step::SetStyle { para: line, style: crate::StyleSheet::NORMAL.into() })?;
        tx.step(Step::SetList { para: line, list: None })?;
        tx.step(Step::SetProps { para: line, props: divider_props() })?;
        let end = tx.doc.para(line).map_or(0, |p| p.text.len());
        let next = split_at(tx, Pos::new(line, end))?;
        tx.step(Step::SetProps { para: next.para, props: ParaProps::default() })?;
        Ok(Selection::caret(next))
    })?;
    ed.seal();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_steps() {
        assert_eq!(step_back("hello big world", 15, true), 10);
        assert_eq!(step_back("hello big ", 10, true), 6);
        assert_eq!(step_forward("hello big world", 0, true), 6);
        assert_eq!(step_forward("hello big world", 5, true), 6);
        assert_eq!(step_forward("hello", 0, true), 5);
        assert_eq!(step_back("e\u{301}x", 4, false), 3);
        assert_eq!(step_back("e\u{301}x", 3, false), 0);
    }
}
