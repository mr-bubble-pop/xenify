//! Footnotes and endnotes: inserting them, and their numbers (in the order
//! the text refers to them; footnotes 1, 2, 3…, endnotes i, ii, iii…).

use crate::{
    Block, CharFmt, EditError, Editor, Field, NodeId, Note, NoteKind, Paragraph, Pos, Selection, Span, Step, StyleSheet,
};
use std::collections::HashMap;

/// Each referred-to note's number as shown.
pub fn numbers(doc: &crate::Document) -> HashMap<NodeId, String> {
    let mut out = HashMap::new();
    let (mut foot, mut end) = (0u32, 0u32);
    for p in doc.paragraphs() {
        for run in &p.runs {
            if let Some(Field::Note { id }) = &run.fmt.field
                && !out.contains_key(id)
                && let Some(n) = doc.note(*id)
            {
                let label = match n.kind {
                    NoteKind::Foot => {
                        foot += 1;
                        foot.to_string()
                    }
                    NoteKind::End => {
                        end += 1;
                        crate::lists::format_number(end, crate::NumberKind::LowerRoman)
                    }
                };
                out.insert(*id, label);
            }
        }
    }
    out
}

/// The notes referred to in paragraph `p`, with the byte offset of each reference.
pub fn references(p: &Paragraph) -> Vec<(usize, NodeId)> {
    p.run_ranges()
        .filter_map(|(r, f)| match &f.field {
            Some(Field::Note { id }) => Some((r.start, *id)),
            _ => None,
        })
        .collect()
}

/// References › Insert footnote / endnote: a note number at the caret, and
/// the caret in the new note's text. One undo step.
pub fn insert_note(ed: &mut Editor, kind: NoteKind) -> Result<NodeId, EditError> {
    let at = ed.doc.ordered(ed.sel).1;
    let si = ed.doc.section_of(at.para).unwrap_or(0);
    // Not inside another note.
    if matches!(ed.doc.locate(at.para), Some((crate::Container::Note(_), _))) {
        return Err(EditError::NotHere);
    }
    let note_id = ed.doc.alloc_id();
    let para_id = ed.doc.alloc_id();
    let mut notes = ed.doc.sections.get(si).ok_or(EditError::NotHere)?.notes.clone();
    notes.push(Note {
        id: note_id,
        kind,
        blocks: vec![Block::Paragraph(Paragraph::new(para_id, StyleSheet::NOTE, ""))],
    });
    let base = ed.doc.para(at.para).map(|p| p.fmt_at(at.offset)).unwrap_or_default();
    let base = crate::review::typed_fmt(&ed.doc, base, None);
    let fmt = CharFmt { field: Some(Field::Note { id: note_id }), link: None, ..base };
    let need_style = ed.doc.styles.get(StyleSheet::NOTE).is_none();
    ed.seal();
    ed.transact(if kind == NoteKind::Foot { "Insert footnote" } else { "Insert endnote" }, None, |tx| {
        if need_style && let Some(def) = StyleSheet::default().get(StyleSheet::NOTE).cloned() {
            tx.step(Step::SetStyleDef { id: StyleSheet::NOTE.into(), def: Some(def) })?;
        }
        tx.step(Step::SetNotes { section: si, notes })?;
        tx.step(Step::Replace { para: at.para, offset: at.offset, remove: 0, insert: Span::plain(Field::STR, fmt) })?;
        Ok(Selection::caret(Pos::new(para_id, 0)))
    })?;
    ed.seal();
    Ok(note_id)
}
