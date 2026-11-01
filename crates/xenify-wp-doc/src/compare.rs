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

/// How much work [`diff`] does looking for the shortest script before it
/// settles for a good one: (tokens) × (changes) steps. About a second at
/// most; two versions of a long document with a few thousand changes
/// between them are well inside it.
const WORK: usize = 200_000_000;
/// The most changes it looks for the shortest script with. Its memory
/// grows with the square of this (8 MB at 1,000).
const MAX_CHANGES: usize = 1_000;

/// An edit script from `a` to `b`: the shortest (Myers' algorithm) when
/// they differ by at most a few thousand tokens; else one found by lining
/// up what each has only once (patience diff), which is good but can be a
/// little longer. Its time and memory stay modest however big and however
/// different `a` and `b` are.
pub fn diff<T: Eq + std::hash::Hash>(a: &[T], b: &[T]) -> Vec<Edit> {
    let mut out = Vec::new();
    // Ranges still to compare (in order, last first), and matches to give.
    enum Job {
        Diff(usize, usize, usize, usize),
        Same(usize, usize),
    }
    let mut jobs = vec![Job::Diff(0, a.len(), 0, b.len())];
    while let Some(job) = jobs.pop() {
        let (a0, a1, b0, b1) = match job {
            Job::Same(i, j) => {
                out.push(Edit::Same(i, j));
                continue;
            }
            Job::Diff(a0, a1, b0, b1) => (a0, a1, b0, b1),
        };
        // Common ends first: the usual case is a small change in a big text.
        let (am, bm) = (&a[a0..a1], &b[b0..b1]);
        let pre = am.iter().zip(bm).take_while(|(x, y)| x == y).count();
        let suf = am[pre..].iter().rev().zip(bm[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
        out.extend((0..pre).map(|i| Edit::Same(a0 + i, b0 + i)));
        let (a0, a1, b0, b1) = (a0 + pre, a1 - suf, b0 + pre, b1 - suf);
        let tail = (0..suf).map(|i| Edit::Same(a1 + i, b1 + i));
        let (am, bm) = (&a[a0..a1], &b[b0..b1]);
        if am.is_empty() || bm.is_empty() {
            out.extend((a0..a1).map(Edit::Removed));
            out.extend((b0..b1).map(Edit::Added));
            out.extend(tail);
            continue;
        }
        let limit = MAX_CHANGES.min(WORK / (am.len() + bm.len()));
        if let Some(mid) = myers(am, bm, limit) {
            out.extend(mid.into_iter().map(|e| match e {
                Edit::Same(i, j) => Edit::Same(a0 + i, b0 + j),
                Edit::Removed(i) => Edit::Removed(a0 + i),
                Edit::Added(j) => Edit::Added(b0 + j),
            }));
            out.extend(tail);
            continue;
        }
        // Too different for the shortest script: line up the tokens each
        // has only once, and compare what's between them the same way.
        let anchors = anchors(am, bm);
        if anchors.is_empty() {
            out.extend((a0..a1).map(Edit::Removed));
            out.extend((b0..b1).map(Edit::Added));
            out.extend(tail);
            continue;
        }
        let tail: Vec<Edit> = tail.collect();
        for e in tail.into_iter().rev() {
            if let Edit::Same(i, j) = e {
                jobs.push(Job::Same(i, j));
            }
        }
        let (mut ea, mut eb) = (a1, b1);
        for &(i, j) in anchors.iter().rev() {
            let (i, j) = (a0 + i, b0 + j);
            jobs.push(Job::Diff(i + 1, ea, j + 1, eb));
            jobs.push(Job::Same(i, j));
            (ea, eb) = (i, j);
        }
        jobs.push(Job::Diff(a0, ea, b0, eb));
    }
    out
}

/// Pairs (index in `a`, index in `b`) of tokens each has exactly once, the
/// longest run of them in the same order in both.
fn anchors<T: Eq + std::hash::Hash>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    // Per token: times in a, times in b, where in a, where in b.
    let mut seen: HashMap<&T, (u32, u32, usize, usize)> = HashMap::new();
    for (i, t) in a.iter().enumerate() {
        let e = seen.entry(t).or_insert((0, 0, i, 0));
        e.0 = e.0.saturating_add(1);
    }
    for (j, t) in b.iter().enumerate() {
        if let Some(e) = seen.get_mut(t) {
            e.1 = e.1.saturating_add(1);
            e.3 = j;
        }
    }
    let mut pairs: Vec<(usize, usize)> =
        seen.into_values().filter(|e| e.0 == 1 && e.1 == 1).map(|e| (e.2, e.3)).collect();
    pairs.sort_unstable();
    // Longest increasing run of the b indices (patience sorting).
    let mut tops: Vec<usize> = Vec::new();
    let mut back: Vec<Option<usize>> = vec![None; pairs.len()];
    for (n, &(_, j)) in pairs.iter().enumerate() {
        let pile = tops.partition_point(|&t| pairs[t].1 < j);
        back[n] = pile.checked_sub(1).map(|p| tops[p]);
        if pile == tops.len() {
            tops.push(n);
        } else {
            tops[pile] = n;
        }
    }
    let mut out = Vec::new();
    let mut at = tops.last().copied();
    while let Some(n) = at {
        out.push(pairs[n]);
        at = back[n];
    }
    out.reverse();
    out
}

/// The shortest edit script from `a` to `b` (Myers' algorithm), or None
/// when it would take more than `limit` changes.
fn myers<T: PartialEq>(a: &[T], b: &[T], limit: usize) -> Option<Vec<Edit>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    let off = max as isize;
    let mut v = vec![0isize; 2 * max + 2];
    // Before each pass d, the part of `v` it reads (diagonals -(d-1)..=d-1).
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = false;
    'search: for d in 0..=(max.min(limit)) as isize {
        let lo = (off - d + 1) as usize;
        trace.push(if d == 0 { Vec::new() } else { v[lo..lo + (2 * d - 1) as usize].to_vec() });
        let mut k = -d;
        while k <= d {
            let i = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[i - 1] < v[i + 1]) { v[i + 1] } else { v[i - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[i] = x;
            if x >= n && y >= m {
                found = true;
                break 'search;
            }
            k += 2;
        }
    }
    if !found {
        return None;
    }
    // Walk back through the trace.
    let at = |d: isize, k: isize| -> isize {
        if d >= 1 && k.abs() < d { trace[d as usize][(k + d - 1) as usize] } else { 0 }
    };
    let mut mid: Vec<Edit> = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(d, k - 1) < at(d, k + 1)) { k + 1 } else { k - 1 };
        let prev_x = at(d, prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            mid.push(Edit::Same(x as usize, y as usize));
        }
        if d > 0 {
            if x == prev_x {
                mid.push(Edit::Added(prev_y as usize));
            } else {
                mid.push(Edit::Removed(prev_x as usize));
            }
        }
        (x, y) = (prev_x, prev_y);
    }
    mid.reverse();
    Some(mid)
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

    /// The script turns `a` into `b`, each token of each used once, in order.
    fn check<T: PartialEq + Clone + std::fmt::Debug>(a: &[T], b: &[T], e: &[Edit]) {
        let (mut i, mut j) = (0, 0);
        for x in e {
            match *x {
                Edit::Same(p, q) => {
                    assert_eq!((p, q), (i, j));
                    assert_eq!(a[p], b[q]);
                    (i, j) = (i + 1, j + 1);
                }
                Edit::Removed(p) => {
                    assert_eq!(p, i);
                    i += 1;
                }
                Edit::Added(q) => {
                    assert_eq!(q, j);
                    j += 1;
                }
            }
        }
        assert_eq!((i, j), (a.len(), b.len()));
    }

    fn words(n: usize, seed: u64) -> Vec<String> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                format!("w{}", (s >> 33) % 5000)
            })
            .collect()
    }

    #[test]
    fn small_changes_still_get_the_shortest_script() {
        let a = words(3000, 1);
        let mut b = a.clone();
        for k in (0..3000).step_by(37) {
            b[k] = "changed".into();
        }
        b.insert(1500, "new".into());
        let e = diff(&a, &b);
        check(&a, &b, &e);
        let changes = e.iter().filter(|x| !matches!(x, Edit::Same(..))).count();
        assert_eq!(changes, 2 * 82 + 1);
    }

    #[test]
    fn two_long_different_versions_are_compared_quickly_in_little_memory() {
        // 50,000 words each, nothing alike but a few sentences: once, this
        // needed some 160 GB.
        let mut a = words(50_000, 2);
        let mut b = words(50_000, 3);
        for (k, w) in ["alpha", "bravo", "charlie", "delta"].iter().enumerate() {
            a[10_000 * (k + 1)] = w.to_string();
            b[9_000 * (k + 1)] = w.to_string();
        }
        let t = std::time::Instant::now();
        let e = diff(&a, &b);
        assert!(t.elapsed() < std::time::Duration::from_secs(60));
        check(&a, &b, &e);
        // The words both have once are lined up.
        assert!(e.contains(&Edit::Same(20_000, 18_000)));
        // Mostly the same with many changes: still close to the shortest.
        let mut c = a.clone();
        for k in (0..50_000).step_by(20) {
            c[k] = format!("x{k}");
        }
        let e = diff(&a, &c);
        check(&a, &c, &e);
        let changes = e.iter().filter(|x| !matches!(x, Edit::Same(..))).count();
        assert!(changes <= 2 * 2500 * 2, "{changes}");
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
