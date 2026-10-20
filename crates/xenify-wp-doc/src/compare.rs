//! Comparing two versions of a document: the newer one with what changed
//! marked as tracked changes (insertions underlined, deletions struck
//! through), so it shows, and can be accepted or rejected, like any edit.

use crate::review::{Change, ChangeKind};
use crate::{Block, CharFmt, Document, Paragraph, Run};

/// One step of a diff: the same in both, only in the old, only in the new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Same(usize, usize),
    Removed(usize),
    Added(usize),
}

/// The shortest edit script from `a` to `b` (Myers' algorithm).
pub fn diff<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Edit> {
    // Common ends first: the usual case is a small change in a big text.
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut out: Vec<Edit> = (0..pre).map(|i| Edit::Same(i, i)).collect();
    let (n, m) = (am.len() as isize, bm.len() as isize);
    let max = (n + m) as usize;
    let off = max as isize;
    let mut v = vec![0isize; 2 * max + 2];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'search: for d in 0..=max as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let i = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[i - 1] < v[i + 1]) { v[i + 1] } else { v[i - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && am[x as usize] == bm[y as usize] {
                x += 1;
                y += 1;
            }
            v[i] = x;
            if x >= n && y >= m {
                break 'search;
            }
            k += 2;
        }
    }
    // Walk back through the trace.
    let mut mid: Vec<Edit> = Vec::new();
    let (mut x, mut y) = (n, m);
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;
        let i = (k + off) as usize;
        let prev_k = if k == -d || (k != d && v[i - 1] < v[i + 1]) { k + 1 } else { k - 1 };
        let prev_x = v[(prev_k + off) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            mid.push(Edit::Same(x as usize + pre, y as usize + pre));
        }
        if d > 0 {
            if x == prev_x {
                mid.push(Edit::Added(prev_y as usize + pre));
            } else {
                mid.push(Edit::Removed(prev_x as usize + pre));
            }
        }
        (x, y) = (prev_x, prev_y);
    }
    mid.reverse();
    out.extend(mid);
    out.extend((0..suf).map(|i| Edit::Same(a.len() - suf + i, b.len() - suf + i)));
    out
}

/// Words and the spaces between them (each a token), as byte ranges.
fn tokens(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut kind = None;
    for (i, c) in text.char_indices() {
        let k = if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() {
            1
        } else {
            2 + i
        };
        if Some(k) != kind || k >= 2 {
            if i > start {
                out.push(start..i);
            }
            start = i;
            kind = Some(k);
        }
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

/// What a block is, for matching blocks between versions.
fn key(b: &Block) -> String {
    match b {
        Block::Paragraph(p) => format!("p\u{1}{}\u{1}{}", p.style, p.text),
        other => {
            // Tables, pictures, contents: compared whole (ids aside).
            let mut s = format!("{other:?}");
            s.retain(|c| !c.is_ascii_digit());
            s
        }
    }
}

/// `p`'s text in `range` appended to `out`, with `change` on it.
fn push_range(out: &mut Paragraph, p: &Paragraph, range: std::ops::Range<usize>, change: Option<&Change>) {
    let mut start = 0;
    for r in &p.runs {
        let (s, e) = (start, start + r.len);
        start = e;
        let (a, b) = (s.max(range.start), e.min(range.end));
        if a >= b {
            continue;
        }
        let fmt = CharFmt { change: change.cloned(), ..r.fmt.clone() };
        out.text.push_str(&p.text[a..b]);
        match out.runs.last_mut() {
            Some(last) if last.fmt == fmt => last.len += b - a,
            _ => out.runs.push(Run { len: b - a, fmt }),
        }
    }
}

/// A paragraph that changed: the new one, with removed words put back
/// struck through and added words underlined.
fn merge_paragraph(old: &Paragraph, new: &Paragraph, ins: &Change, del: &Change) -> Paragraph {
    let (ta, tb) = (tokens(&old.text), tokens(&new.text));
    let wa: Vec<&str> = ta.iter().map(|r| &old.text[r.clone()]).collect();
    let wb: Vec<&str> = tb.iter().map(|r| &new.text[r.clone()]).collect();
    let mut out = Paragraph { text: String::new(), runs: Vec::new(), ..new.clone() };
    for e in diff(&wa, &wb) {
        match e {
            Edit::Same(_, j) => push_range(&mut out, new, tb[j].clone(), None),
            Edit::Removed(i) => push_range(&mut out, old, ta[i].clone(), Some(del)),
            Edit::Added(j) => push_range(&mut out, new, tb[j].clone(), Some(ins)),
        }
    }
    out
}

/// A copy of paragraph `p` (new id) wholly marked with `change`.
fn marked(doc: &mut Document, p: &Paragraph, change: &Change) -> Paragraph {
    let mut q = Paragraph { id: doc.alloc_id(), text: String::new(), runs: Vec::new(), ..p.clone() };
    push_range(&mut q, p, 0..p.text.len(), Some(change));
    if q.text.is_empty() {
        // An empty paragraph removed or added: its break is the change.
        q.props.break_change = Some(change.clone());
    }
    q
}

/// How much two paragraphs share (0–1), by words.
fn likeness(a: &str, b: &str) -> f32 {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
    };
    let (wa, wb) = (words(a), words(b));
    if wa.is_empty() && wb.is_empty() {
        return 1.0;
    }
    let same = diff(&wa, &wb).iter().filter(|e| matches!(e, Edit::Same(..))).count();
    2.0 * same as f32 / (wa.len() + wb.len()) as f32
}

/// `new` with the differences from `old` marked as tracked changes by
/// `label` (e.g. "Changes since 14:03, 10 Oct"). Section by section; a
/// section only one has comes in whole.
pub fn compare(old: &Document, new: &Document, label: &str) -> Document {
    let mut out = new.clone();
    let date = crate::review::now();
    let ins = Change { kind: ChangeKind::Inserted, author: label.to_string(), date: date.clone() };
    let del = Change { kind: ChangeKind::Deleted, author: label.to_string(), date };
    for si in 0..out.sections.len() {
        let old_blocks: Vec<Block> = old.sections.get(si).map(|s| s.blocks.clone()).unwrap_or_default();
        let new_blocks = out.sections[si].blocks.clone();
        let ka: Vec<String> = old_blocks.iter().map(key).collect();
        let kb: Vec<String> = new_blocks.iter().map(key).collect();
        let edits = diff(&ka, &kb);
        let mut merged: Vec<Block> = Vec::new();
        let mut i = 0;
        while i < edits.len() {
            match edits[i] {
                Edit::Same(_, j) => {
                    merged.push(new_blocks[j].clone());
                    i += 1;
                }
                _ => {
                    // A run of removals and additions: pair up paragraphs that
                    // are alike (edited), the rest are whole removals/additions.
                    let mut removed = Vec::new();
                    let mut added = Vec::new();
                    while i < edits.len() && !matches!(edits[i], Edit::Same(..)) {
                        match edits[i] {
                            Edit::Removed(a) => removed.push(a),
                            Edit::Added(b) => added.push(b),
                            Edit::Same(..) => {}
                        }
                        i += 1;
                    }
                    let mut ri = 0;
                    // Additions waiting for the removals before them to go out first.
                    let mut pending: Vec<usize> = Vec::new();
                    for &b in &added {
                        // Removed blocks before the one this pairs with go first.
                        let pair = removed[ri..].iter().position(|&a| match (&old_blocks[a], &new_blocks[b]) {
                            (Block::Paragraph(pa), Block::Paragraph(pb)) => likeness(&pa.text, &pb.text) >= 0.4,
                            _ => false,
                        });
                        if let Some(off) = pair {
                            for &a in &removed[ri..ri + off] {
                                push_removed(&mut out, &mut merged, &old_blocks[a], &del);
                            }
                            for b in pending.drain(..) {
                                push_added(&mut merged, &new_blocks[b], &ins);
                            }
                            let a = removed[ri + off];
                            ri += off + 1;
                            if let (Block::Paragraph(pa), Block::Paragraph(pb)) = (&old_blocks[a], &new_blocks[b]) {
                                merged.push(Block::Paragraph(merge_paragraph(pa, pb, &ins, &del)));
                            }
                        } else {
                            pending.push(b);
                        }
                    }
                    for &a in &removed[ri..] {
                        push_removed(&mut out, &mut merged, &old_blocks[a], &del);
                    }
                    for b in pending {
                        push_added(&mut merged, &new_blocks[b], &ins);
                    }
                }
            }
        }
        out.sections[si].blocks = merged;
    }
    out
}

/// A block only the new version has, its text underlined.
fn push_added(merged: &mut Vec<Block>, b: &Block, ins: &Change) {
    match b {
        Block::Paragraph(p) => {
            let mut q = Paragraph { text: String::new(), runs: Vec::new(), ..p.clone() };
            push_range(&mut q, p, 0..p.text.len(), Some(ins));
            if q.text.is_empty() {
                q.props.break_change = Some(ins.clone());
            }
            merged.push(Block::Paragraph(q));
        }
        other => merged.push(other.clone()),
    }
}

/// A block only the old version has, put back struck through (other than
/// paragraphs: left out, as there's no way to mark them).
fn push_removed(doc: &mut Document, merged: &mut Vec<Block>, b: &Block, del: &Change) {
    if let Block::Paragraph(p) = b {
        let q = marked(doc, p, del);
        merged.push(Block::Paragraph(q));
    }
}

/// A short summary of what changed: (words added, words removed).
pub fn word_changes(old: &Document, new: &Document) -> (usize, usize) {
    let words = |d: &Document| -> Vec<String> {
        d.text_paragraphs()
            .flat_map(|p| {
                p.text
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| !w.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let (a, b) = (words(old), words(new));
    let mut added = 0;
    let mut removed = 0;
    for e in diff(&a, &b) {
        match e {
            Edit::Added(_) => added += 1,
            Edit::Removed(_) => removed += 1,
            Edit::Same(..) => {}
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn myers_finds_the_shortest_script() {
        let a: Vec<char> = "ABCABBA".chars().collect();
        let b: Vec<char> = "CBABAC".chars().collect();
        let e = diff(&a, &b);
        let changes = e.iter().filter(|x| !matches!(x, Edit::Same(..))).count();
        assert_eq!(changes, 5);
        // Applying the script to a gives b.
        let rebuilt: Vec<char> = e
            .iter()
            .filter_map(|x| match x {
                Edit::Same(_, j) | Edit::Added(j) => Some(b[*j]),
                Edit::Removed(_) => None,
            })
            .collect();
        assert_eq!(rebuilt, b);
        assert!(diff::<char>(&[], &[]).is_empty());
        assert_eq!(diff(&['x'], &[]), [Edit::Removed(0)]);
    }

    #[test]
    fn edited_paragraphs_show_word_changes() {
        let old = Document::from_plain_text("The cat sat on the mat.\nA paragraph that goes.\nThe end.");
        let new = Document::from_plain_text("The dog sat on the mat.\nA new paragraph.\nThe end.");
        let c = compare(&old, &new, "Since Tuesday");
        let shown: Vec<String> = c
            .paragraphs()
            .map(|p| {
                p.run_ranges()
                    .map(|(r, f)| match f.change.as_ref().map(|c| c.kind) {
                        Some(ChangeKind::Inserted) => format!("{{+{}+}}", &p.text[r]),
                        Some(ChangeKind::Deleted) => format!("{{-{}-}}", &p.text[r]),
                        None => p.text[r].to_string(),
                    })
                    .collect()
            })
            .collect();
        assert_eq!(shown[0], "The {-cat-}{+dog+} sat on the mat.");
        assert!(shown[1].contains("{-") && shown[1].contains("{+"), "{shown:?}");
        assert_eq!(shown.last().unwrap(), "The end.");
        // Rejecting everything gives the old text back; accepting, the new.
        let mut ed = crate::Editor::new(c.clone());
        crate::review::decide(&mut ed, false, true).unwrap();
        assert_eq!(ed.doc.plain_text(), old.plain_text());
        let mut ed = crate::Editor::new(c);
        crate::review::decide(&mut ed, true, true).unwrap();
        assert_eq!(ed.doc.plain_text(), new.plain_text());
        assert_eq!(word_changes(&old, &new), (2, 3));
    }
}
