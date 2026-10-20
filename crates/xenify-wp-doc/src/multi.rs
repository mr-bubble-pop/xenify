//! Several carets at once (Alt+click, Ctrl+D): an edit runs at each, last
//! to first, as one undo step.
//!
//! An edit only changes text near the caret it runs at, so carets before
//! it keep their distance from the start of the document's text
//! (paragraphs joined by a break) and carets after it keep their distance
//! from the end, even when paragraphs are split or joined.

use crate::{Document, Editor, MergeKey, Pos, Selection};

/// Characters before `pos` in the document.
fn from_start(doc: &Document, pos: Pos) -> usize {
    total(doc) - from_end(doc, pos)
}

fn total(doc: &Document) -> usize {
    doc.paragraphs().map(|p| p.text.len() + 1).sum()
}

/// The position `g` characters into the document.
fn at_from_start(doc: &Document, g: usize) -> Pos {
    at_from_end(doc, total(doc).saturating_sub(g))
}

/// Characters from `pos` to the end of the document.
fn from_end(doc: &Document, pos: Pos) -> usize {
    let mut total = 0;
    let mut at = None;
    for p in doc.paragraphs() {
        if p.id == pos.para {
            at = Some(total + pos.offset.min(p.text.len()));
        }
        total += p.text.len() + 1;
    }
    total - at.unwrap_or(total)
}

/// The position `d` characters from the end.
fn at_from_end(doc: &Document, d: usize) -> Pos {
    let paras: Vec<_> = doc.paragraphs().map(|p| (p.id, p.text.len())).collect();
    let total: usize = paras.iter().map(|(_, l)| l + 1).sum();
    let mut g = total.saturating_sub(d);
    for (id, len) in &paras {
        if g <= *len {
            // Inside a character (the text changed under it): back to a boundary.
            let text = &doc.para(*id).map(|p| p.text.as_str()).unwrap_or("");
            while g > 0 && !text.is_char_boundary(g) {
                g -= 1;
            }
            return Pos::new(*id, g);
        }
        g -= len + 1;
    }
    doc.end()
}

/// The selections in document order, overlapping ones joined.
pub fn normalize(doc: &Document, sels: &[Selection]) -> Vec<Selection> {
    let mut v: Vec<(usize, usize, Selection)> = sels
        .iter()
        .map(|s| {
            let (a, b) = (from_end(doc, s.anchor), from_end(doc, s.focus));
            (a.max(b), a.min(b), *s)
        })
        .collect();
    // Furthest from the end first = document order.
    v.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)));
    // (start, end) as distances from the end: start >= end.
    let mut out: Vec<(usize, usize, Selection)> = Vec::new();
    for (start, end, s) in v {
        if let Some(last) = out.last_mut() {
            let caret = start == end || last.0 == last.1;
            // Starts inside the one before (or is a caret touching it): join them.
            if start > last.1 || (start == last.1 && caret) {
                if end < last.1 {
                    last.1 = end;
                    last.2 = Selection { anchor: at_from_end(doc, last.0), focus: at_from_end(doc, end) };
                }
                continue;
            }
        }
        out.push((start, end, s));
    }
    out.into_iter().map(|(_, _, s)| s).collect()
}

/// Runs `edit` at each of `sels` (as the editor's selection), last first,
/// as one undo step; returns where each selection ended up, in document
/// order. The editor's selection is left at the last one.
pub fn edit_each(
    ed: &mut Editor,
    sels: &[Selection],
    label: &str,
    merge: Option<MergeKey>,
    mut edit: impl FnMut(&mut Editor),
) -> Vec<Selection> {
    let sels = normalize(&ed.doc, sels);
    // Not yet edited at: before every edit so far, so counted from the start.
    let dists: Vec<(usize, usize)> =
        sels.iter().map(|s| (from_start(&ed.doc, s.anchor), from_start(&ed.doc, s.focus))).collect();
    // Done: after every edit still to come, so counted from the end.
    let mut after = vec![(0, 0); dists.len()];
    ed.group(label, merge, |ed| {
        for i in (0..dists.len()).rev() {
            let (a, f) = dists[i];
            ed.sel = Selection { anchor: at_from_start(&ed.doc, a), focus: at_from_start(&ed.doc, f) };
            edit(ed);
            after[i] = (from_end(&ed.doc, ed.sel.anchor), from_end(&ed.doc, ed.sel.focus));
        }
    });
    let out: Vec<Selection> = after
        .iter()
        .map(|(a, f)| Selection { anchor: at_from_end(&ed.doc, *a), focus: at_from_end(&ed.doc, *f) })
        .collect();
    if let Some(last) = out.last() {
        ed.sel = *last;
    }
    out
}

/// The next place `text` occurs after `after` (wrapping round to the
/// start), skipping places in `taken`: Ctrl+D's next match.
pub fn next_match(doc: &Document, text: &str, after: Pos, taken: &[Selection]) -> Option<Selection> {
    if text.is_empty() {
        return None;
    }
    let found = crate::ops::find_all(doc, text, true, false);
    let here = from_end(doc, after);
    let free = |(p, r): &(crate::NodeId, std::ops::Range<usize>)| {
        let s = Selection { anchor: Pos::new(*p, r.start), focus: Pos::new(*p, r.end) };
        let (a, b) = doc.ordered(s);
        (!taken.iter().any(|t| doc.ordered(*t) == (a, b))).then_some(s)
    };
    found
        .iter()
        .filter(|(p, r)| from_end(doc, Pos::new(*p, r.start)) <= here)
        .find_map(free)
        .or_else(|| found.iter().find_map(free))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    #[test]
    fn typing_at_three_carets_is_one_undo() {
        let mut ed = Editor::new(Document::from_plain_text("one\ntwo\nthree"));
        let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
        let sels: Vec<Selection> = ids.iter().map(|id| Selection::caret(Pos::new(*id, 0))).collect();
        let sels = edit_each(&mut ed, &sels, "Typing", None, |ed| {
            let _ = ops::insert_text(ed, "- ", None);
        });
        let texts: Vec<_> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["- one", "- two", "- three"]);
        assert_eq!(sels.iter().map(|s| s.focus.offset).collect::<Vec<_>>(), [2, 2, 2]);
        ed.undo();
        let texts: Vec<_> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["one", "two", "three"]);
    }

    #[test]
    fn enter_and_backspace_keep_the_other_carets() {
        let mut ed = Editor::new(Document::from_plain_text("ab cd\nef"));
        let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
        let sels = [
            Selection::caret(Pos::new(ids[0], 2)),
            Selection::caret(Pos::new(ids[0], 4)),
            Selection::caret(Pos::new(ids[1], 0)),
        ];
        let sels = edit_each(&mut ed, &sels, "Enter", None, |ed| {
            let _ = ops::split_paragraph(ed);
        });
        let texts: Vec<_> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["ab", " c", "d", "", "ef"]);
        assert_eq!(sels.len(), 3);
        let sels = edit_each(&mut ed, &sels, "Delete", None, |ed| {
            let _ = ops::backspace(ed, false);
        });
        let texts: Vec<_> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["ab cd", "ef"]);
        assert_eq!(sels.iter().map(|s| s.focus.offset).collect::<Vec<_>>(), [2, 4, 0]);
    }

    #[test]
    fn ctrl_d_finds_the_next_match() {
        let ed = Editor::new(Document::from_plain_text("cat dog cat\ncat"));
        let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
        let first = Selection { anchor: Pos::new(ids[0], 0), focus: Pos::new(ids[0], 3) };
        let next = next_match(&ed.doc, "cat", first.focus, &[first]).unwrap();
        assert_eq!(next.anchor, Pos::new(ids[0], 8));
        let third = next_match(&ed.doc, "cat", next.focus, &[first, next]).unwrap();
        assert_eq!(third.anchor, Pos::new(ids[1], 0));
        assert!(next_match(&ed.doc, "cat", third.focus, &[first, next, third]).is_none());
    }
}
