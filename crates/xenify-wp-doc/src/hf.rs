//! Rich headers and footers: which one a page shows, and turning a simple
//! three-part line into one to edit on the page.

use crate::edit::{EditError, Editor, Step};
use crate::{
    Block, CharFmt, Document, Field, HeaderPart, HfLine, HfSlot, NodeId, Paragraph, Pos, Run, Selection, StyleSheet,
};

/// The section whose headers a page of section `section` shows (following
/// "link to previous"), and which of them: the first page's, even pages', or
/// the usual one.
pub fn slot_for(doc: &Document, section: usize, page_number: u32, first_of_section: bool) -> (usize, HfSlot) {
    let mut si = section.min(doc.sections.len().saturating_sub(1));
    while si > 0 && doc.sections[si].page.link_to_previous {
        si -= 1;
    }
    let ps = &doc.sections[si].page;
    let slot = if first_of_section && ps.different_first {
        HfSlot::First
    } else if ps.different_odd_even && page_number.is_multiple_of(2) {
        HfSlot::Even
    } else {
        HfSlot::Default
    };
    (si, slot)
}

/// The simple line for a slot.
fn line(doc: &Document, si: usize, footer: bool, slot: HfSlot) -> HfLine {
    let p = &doc.sections[si].page;
    match (footer, slot) {
        (false, HfSlot::Default) => p.header.clone(),
        (true, HfSlot::Default) => p.footer.clone(),
        (false, HfSlot::First) => p.first_header.clone(),
        (true, HfSlot::First) => p.first_footer.clone(),
        (false, HfSlot::Even) => p.even_header.clone(),
        (true, HfSlot::Even) => p.even_footer.clone(),
    }
}

/// `text` with `{page}`, `{pages}`, `{date}`, `{title}`, `{author}` as fields, appended to `p`.
fn push_with_fields(p: &mut Paragraph, text: &str) {
    let mut rest = text;
    while !rest.is_empty() {
        let next = [
            ("{page}", Field::Page),
            ("{pages}", Field::Pages),
            ("{date}", Field::Date),
            ("{title}", Field::Title),
            ("{author}", Field::Author),
        ]
        .into_iter()
        .filter_map(|(k, f)| rest.find(k).map(|i| (i, k, f)))
        .min_by_key(|(i, _, _)| *i);
        let Some((i, k, f)) = next else {
            push_plain(p, rest);
            break;
        };
        push_plain(p, &rest[..i]);
        p.text.push(Field::CHAR);
        p.runs.push(Run { len: Field::CHAR.len_utf8(), fmt: CharFmt { field: Some(f), ..Default::default() } });
        rest = &rest[i + k.len()..];
    }
}

fn push_plain(p: &mut Paragraph, t: &str) {
    if t.is_empty() {
        return;
    }
    p.text.push_str(t);
    match p.runs.last_mut() {
        Some(r) if r.fmt.is_empty() => r.len += t.len(),
        _ => p.runs.push(Run { len: t.len(), fmt: CharFmt::default() }),
    }
}

/// Starts editing the header (or footer) a page shows: makes it rich if
/// it's a simple line (keeping its text, with tab stops for the centre
/// and right parts), and puts the caret in it.
pub fn edit_on_page(
    ed: &mut Editor,
    section: usize,
    page_number: u32,
    first_of_section: bool,
    footer: bool,
) -> Result<NodeId, EditError> {
    let (si, slot) = slot_for(&ed.doc, section, page_number, first_of_section);
    if let Some(part) = ed.doc.sections[si].headers.iter().find(|h| h.footer == footer && h.slot == slot)
        && let Some(Block::Paragraph(p)) = part.blocks.first()
    {
        let id = p.id;
        ed.sel = Selection::caret(Pos::new(id, p.text.len()));
        return Ok(id);
    }
    let l = line(&ed.doc, si, footer, slot);
    let width = ed.doc.sections[si].page.content_width();
    let pid = ed.doc.alloc_id();
    let hid = ed.doc.alloc_id();
    let mut p = Paragraph::new(pid, if footer { StyleSheet::FOOTER } else { StyleSheet::HEADER }, "");
    if !l.is_empty() {
        // "left ⇥ centre ⇥ right", with a centre and a right tab stop.
        push_with_fields(&mut p, &l.left);
        if !l.center.is_empty() || !l.right.is_empty() {
            push_plain(&mut p, "\t");
            push_with_fields(&mut p, &l.center);
        }
        if !l.right.is_empty() {
            push_plain(&mut p, "\t");
            push_with_fields(&mut p, &l.right);
        }
        p.props.tabs = Some(vec![
            crate::TabStop { pos: width / 2.0, kind: crate::TabKind::Center, ..Default::default() },
            crate::TabStop { pos: width, kind: crate::TabKind::Right, ..Default::default() },
        ]);
    }
    let end = p.text.len();
    let mut headers = ed.doc.sections[si].headers.clone();
    headers.push(HeaderPart { id: hid, footer, slot, blocks: vec![Block::Paragraph(p)] });
    ed.seal();
    ed.transact(if footer { "Edit footer" } else { "Edit header" }, None, |tx| {
        tx.step(Step::SetHeaders { section: si, headers })?;
        Ok(Selection::caret(Pos::new(pid, end)))
    })?;
    ed.seal();
    Ok(pid)
}

/// Goes back to the simple line for a header or footer (its rich content is removed).
pub fn make_plain(ed: &mut Editor, section: usize, footer: bool, slot: HfSlot) -> Result<(), EditError> {
    let mut headers = ed.doc.sections.get(section).ok_or(EditError::NotHere)?.headers.clone();
    let before = headers.len();
    headers.retain(|h| !(h.footer == footer && h.slot == slot));
    if headers.len() == before {
        return Ok(());
    }
    let sel = Selection::caret(ed.doc.clamp(Pos::new(ed.doc.paragraphs().next().map_or(NodeId(0), |p| p.id), 0)));
    ed.seal();
    ed.transact(if footer { "Plain footer" } else { "Plain header" }, None, |tx| {
        tx.step(Step::SetHeaders { section, headers })?;
        Ok(sel)
    })?;
    ed.seal();
    Ok(())
}

/// Whether paragraph `id` is in a header or footer.
pub fn in_header(doc: &Document, id: NodeId) -> bool {
    matches!(doc.locate(id), Some((crate::Container::Header(_), _)))
}
