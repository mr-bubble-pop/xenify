//! Review: comment threads anchored to text, and tracked changes (text
//! marked inserted or deleted until someone accepts or rejects it).

use crate::edit::{EditError, Editor, Step, Tx};
use crate::{CharFmt, Document, NodeId, Pos, Selection, Span};
use serde::{Deserialize, Serialize};

/// Inserted or deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    #[default]
    Inserted,
    Deleted,
}

/// A tracked change on a stretch of text: what happened, who did it, when.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Change {
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// ISO 8601 (UTC), e.g. "2026-10-10T14:03:00Z".
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
}

/// An answer in a comment thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reply {
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
    pub text: String,
}

/// A comment thread on some text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Comment {
    pub id: NodeId,
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Reply>,
    /// Done with: kept, but no longer highlighted.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
}

/// Now, as ISO 8601 UTC (empty where the platform has no clock).
pub fn now() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
    }
}

pub(crate) fn user(doc: &Document) -> String {
    if doc.user.trim().is_empty() { "Author".into() } else { doc.user.clone() }
}

/// The format typed text gets, from `base` (the text around it): no
/// field, the comments it's inside, and an insertion mark when tracking.
pub fn typed_fmt(doc: &Document, base: CharFmt, after: Option<&CharFmt>) -> CharFmt {
    let comments = base.comments.iter().filter(|c| after.is_some_and(|a| a.comments.contains(c))).copied().collect();
    // Typing on from your own insertion continues it (one change, not one per
    // second the typing took).
    let change = doc.meta.track_changes.then(|| match &base.change {
        Some(c) if c.kind == ChangeKind::Inserted && c.author == user(doc) => c.clone(),
        _ => Change { kind: ChangeKind::Inserted, author: user(doc), date: now() },
    });
    CharFmt { field: None, comments, change, ..base }
}

/// Track changes: instead of removing `a..b`, marks it deleted (text this
/// user inserted while tracking just goes). Paragraphs aren't joined.
pub(crate) fn mark_deleted(tx: &mut Tx, a: Pos, b: Pos) -> Result<Pos, EditError> {
    let who = user(tx.doc);
    let mark = Change { kind: ChangeKind::Deleted, author: who.clone(), date: now() };
    let ids: Vec<NodeId> = if a.para == b.para {
        vec![a.para]
    } else {
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
    let mut caret = a;
    for &id in &ids {
        let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
        let start = if id == a.para { a.offset } else { 0 };
        let end = if id == b.para { b.offset } else { p.text.len() };
        if end <= start {
            continue;
        }
        let old = p.slice(start..end);
        let mut new = Span::default();
        let mut at = 0;
        for r in &old.runs {
            let text = &old.text[at..at + r.len];
            at += r.len;
            let own_insert = r.fmt.change.as_ref().is_some_and(|c| c.kind == ChangeKind::Inserted && c.author == who);
            if own_insert {
                continue;
            }
            let fmt = match &r.fmt.change {
                Some(c) if c.kind == ChangeKind::Deleted => r.fmt.clone(),
                _ => CharFmt { change: Some(mark.clone()), ..r.fmt.clone() },
            };
            new.push(text, fmt);
        }
        if id == a.para {
            caret = Pos::new(id, start);
        }
        if new != old {
            tx.step(Step::Replace { para: id, offset: start, remove: end - start, insert: new })?;
        }
    }
    // The breaks between the paragraphs: marked deleted (one this user just
    // inserted goes for real).
    let mut breaks: Vec<NodeId> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        if i + 1 == ids.len() {
            break;
        }
        // Only between paragraphs that follow each other (not across table cells).
        if next_para(tx.doc, *id) == Some(ids[i + 1]) {
            breaks.push(*id);
        }
    }
    for id in breaks.into_iter().rev() {
        let p = tx.doc.para(id).ok_or(EditError::NoParagraph(id))?;
        match &p.props.break_change {
            Some(c) if c.kind == ChangeKind::Inserted && c.author == who => {
                let end = Pos::new(id, p.text.len());
                let next = next_para(tx.doc, id).ok_or(EditError::NoParagraph(id))?;
                join_now(tx, end, next)?;
            }
            Some(c) if c.kind == ChangeKind::Deleted => {}
            _ => {
                let props = crate::ParaProps { break_change: Some(mark.clone()), ..p.props.clone() };
                tx.step(Step::SetProps { para: id, props })?;
            }
        }
    }
    Ok(caret)
}

/// Joins paragraph `next` onto the end `end` of the one before; the joined
/// paragraph keeps `next`'s break (and its tracked change).
fn join_now(tx: &mut Tx, end: Pos, next: NodeId) -> Result<(), EditError> {
    let tail_break = tx.doc.para(next).and_then(|p| p.props.break_change.clone());
    let head = tx.doc.para(end.para).ok_or(EditError::NoParagraph(end.para))?;
    let props = crate::ParaProps { break_change: tail_break, ..head.props.clone() };
    tx.step(Step::SetProps { para: end.para, props })?;
    crate::ops::delete_range_now(tx, end, Pos::new(next, 0))?;
    Ok(())
}

/// Whether the text at `offset` in `para` (the character after) is marked deleted.
pub fn deleted_at(doc: &Document, pos: Pos) -> bool {
    doc.para(pos.para).is_some_and(|p| {
        p.run_ranges()
            .any(|(r, f)| r.contains(&pos.offset) && f.change.as_ref().is_some_and(|c| c.kind == ChangeKind::Deleted))
    })
}

/// Review › Track changes on or off.
pub fn set_tracking(ed: &mut Editor, on: bool) -> Result<(), EditError> {
    let meta = crate::Meta { track_changes: on, ..ed.doc.meta.clone() };
    crate::ops::set_meta(ed, meta, "Track changes")
}

/// A tracked change in the document (for the reviewing pane).
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeAt {
    pub para: NodeId,
    pub range: std::ops::Range<usize>,
    pub change: Change,
    pub text: String,
    /// The paragraph's break (inserted or deleted), not its text.
    pub is_break: bool,
}

/// Every tracked change, in reading order (neighbouring runs of one change joined).
pub fn changes(doc: &Document) -> Vec<ChangeAt> {
    let mut out: Vec<ChangeAt> = Vec::new();
    for p in doc.paragraphs() {
        for (r, f) in p.run_ranges() {
            let Some(c) = &f.change else { continue };
            match out.last_mut() {
                Some(last) if last.para == p.id && last.range.end == r.start && &last.change == c => {
                    last.range.end = r.end;
                    last.text.push_str(&p.text[r.clone()]);
                }
                _ => out.push(ChangeAt {
                    para: p.id,
                    range: r.clone(),
                    change: c.clone(),
                    text: p.text[r].to_string(),
                    is_break: false,
                }),
            }
        }
        if let Some(c) = &p.props.break_change {
            let end = p.text.len();
            out.push(ChangeAt { para: p.id, range: end..end, change: c.clone(), text: "¶".into(), is_break: true });
        }
    }
    out
}

/// The paragraph right after `id` in the same flow (none past a table or picture).
fn next_para(doc: &Document, id: NodeId) -> Option<NodeId> {
    let (c, i) = crate::ops::neighbour_block(doc, id, true)?;
    match doc.blocks(&c)?.get(i)? {
        crate::Block::Paragraph(p) => Some(p.id),
        _ => None,
    }
}

/// Accept or reject tracked changes: in the selection, the change at the
/// caret (when nothing is selected), or everywhere (`all`).
pub fn decide(ed: &mut Editor, accept: bool, all: bool) -> Result<usize, EditError> {
    let (a, b) = ed.doc.ordered(ed.sel);
    let found = changes(&ed.doc);
    let targets: Vec<ChangeAt> = if all {
        found
    } else if a == b {
        found
            .into_iter()
            .filter(|c| c.para == a.para && c.range.start <= a.offset && a.offset <= c.range.end)
            .take(1)
            .collect()
    } else {
        let order: Vec<NodeId> = ed.doc.paragraphs().map(|p| p.id).collect();
        let idx = |id: NodeId| order.iter().position(|x| *x == id).unwrap_or(0);
        let (ia, ib) = (idx(a.para), idx(b.para));
        found
            .into_iter()
            .filter(|c| {
                let i = idx(c.para);
                let after_start = i > ia || (i == ia && c.range.end > a.offset);
                let before_end = i < ib || (i == ib && c.range.start < b.offset);
                after_start && before_end
            })
            .collect()
    };
    if targets.is_empty() {
        return Ok(0);
    }
    let n = targets.len();
    let label = match (accept, all) {
        (true, true) => "Accept all changes",
        (false, true) => "Reject all changes",
        (true, false) => "Accept change",
        (false, false) => "Reject change",
    };
    let caret = ed.sel.focus;
    ed.seal();
    ed.transact(label, None, |tx| {
        // From the end backwards, so earlier offsets stay right.
        for c in targets.iter().rev() {
            if c.is_break {
                // Keep the break (accept an insertion, reject a deletion) or join the paragraphs.
                let keep = (c.change.kind == ChangeKind::Inserted) == accept;
                let p = tx.doc.para(c.para).ok_or(EditError::NoParagraph(c.para))?;
                if keep {
                    let props = crate::ParaProps { break_change: None, ..p.props.clone() };
                    tx.step(Step::SetProps { para: c.para, props })?;
                } else if let Some(next) = next_para(tx.doc, c.para) {
                    let end = Pos::new(c.para, p.text.len());
                    join_now(tx, end, next)?;
                }
                continue;
            }
            let p = tx.doc.para(c.para).ok_or(EditError::NoParagraph(c.para))?;
            let span = p.slice(c.range.clone());
            let keep = (c.change.kind == ChangeKind::Inserted) == accept;
            let insert = if keep {
                let mut s = span;
                for r in &mut s.runs {
                    r.fmt.change = None;
                }
                s
            } else {
                Span::default()
            };
            tx.step(Step::Replace { para: c.para, offset: c.range.start, remove: c.range.len(), insert })?;
        }
        Ok(Selection::caret(caret))
    })?;
    ed.seal();
    Ok(n)
}

/// The next (or previous) tracked change after the caret, to select it.
pub fn next_change(doc: &Document, from: Pos, forward: bool) -> Option<Selection> {
    let order: Vec<NodeId> = doc.paragraphs().map(|p| p.id).collect();
    let key = |id: NodeId, off: usize| (order.iter().position(|x| *x == id).unwrap_or(0), off);
    let here = key(from.para, from.offset);
    let all = changes(doc);
    let pick = if forward {
        all.iter().find(|c| key(c.para, c.range.start) > here).or(all.first())
    } else {
        all.iter().rev().find(|c| key(c.para, c.range.end) < here).or(all.last())
    }?;
    Some(Selection { anchor: Pos::new(pick.para, pick.range.start), focus: Pos::new(pick.para, pick.range.end) })
}

/// Where each comment is anchored: (comment, start, end), in reading order.
pub fn anchors(doc: &Document) -> Vec<(NodeId, Pos, Pos)> {
    let mut out: Vec<(NodeId, Pos, Pos)> = Vec::new();
    for p in doc.paragraphs() {
        for (r, f) in p.run_ranges() {
            for id in &f.comments {
                match out.iter_mut().find(|(c, _, _)| c == id) {
                    Some((_, _, end)) => *end = Pos::new(p.id, r.end),
                    None => out.push((*id, Pos::new(p.id, r.start), Pos::new(p.id, r.end))),
                }
            }
        }
    }
    out
}

/// Review › New comment: comments on the selection (or the word at the caret).
pub fn add_comment(ed: &mut Editor, text: &str) -> Result<NodeId, EditError> {
    if ed.sel.is_collapsed() {
        ed.sel = crate::ops::word_at(ed, ed.sel.focus);
    }
    let (a, b) = ed.doc.ordered(ed.sel);
    if a == b {
        return Err(EditError::NotHere);
    }
    let id = ed.doc.alloc_id();
    let mut meta = ed.doc.meta.clone();
    meta.comments.push(Comment {
        id,
        author: user(&ed.doc),
        date: now(),
        text: text.trim().to_string(),
        ..Default::default()
    });
    let paras = crate::ops::selected_paragraphs(ed);
    let sel = ed.sel;
    ed.seal();
    ed.transact("New comment", None, |tx| {
        tx.step(Step::SetMeta { meta: Box::new(meta) })?;
        for pid in paras {
            let p = tx.doc.para(pid).ok_or(EditError::NoParagraph(pid))?;
            let start = if pid == a.para { a.offset } else { 0 };
            let end = if pid == b.para { b.offset } else { p.text.len() };
            if start >= end {
                continue;
            }
            let mut span = p.slice(start..end);
            for r in &mut span.runs {
                r.fmt.comments.push(id);
            }
            tx.step(Step::Replace { para: pid, offset: start, remove: end - start, insert: span })?;
        }
        Ok(sel)
    })?;
    ed.seal();
    Ok(id)
}

fn update_comment(ed: &mut Editor, id: NodeId, label: &str, f: impl FnOnce(&mut Comment)) -> Result<(), EditError> {
    let mut meta = ed.doc.meta.clone();
    let c = meta.comments.iter_mut().find(|c| c.id == id).ok_or(EditError::NotHere)?;
    f(c);
    ed.seal();
    crate::ops::set_meta(ed, meta, label)?;
    ed.seal();
    Ok(())
}

pub fn reply(ed: &mut Editor, id: NodeId, text: &str) -> Result<(), EditError> {
    let author = user(&ed.doc);
    update_comment(ed, id, "Reply", |c| c.replies.push(Reply { author, date: now(), text: text.trim().into() }))
}

pub fn set_resolved(ed: &mut Editor, id: NodeId, resolved: bool) -> Result<(), EditError> {
    update_comment(ed, id, if resolved { "Resolve comment" } else { "Reopen comment" }, |c| c.resolved = resolved)
}

pub fn edit_comment(ed: &mut Editor, id: NodeId, text: &str) -> Result<(), EditError> {
    update_comment(ed, id, "Edit comment", |c| c.text = text.trim().into())
}

/// Deletes the thread and its anchor.
pub fn delete_comment(ed: &mut Editor, id: NodeId) -> Result<(), EditError> {
    let mut meta = ed.doc.meta.clone();
    meta.comments.retain(|c| c.id != id);
    let paras: Vec<NodeId> =
        ed.doc.paragraphs().filter(|p| p.runs.iter().any(|r| r.fmt.comments.contains(&id))).map(|p| p.id).collect();
    let sel = ed.sel;
    ed.seal();
    ed.transact("Delete comment", None, |tx| {
        tx.step(Step::SetMeta { meta: Box::new(meta) })?;
        for pid in paras {
            let p = tx.doc.para(pid).ok_or(EditError::NoParagraph(pid))?;
            let mut span = p.slice(0..p.text.len());
            for r in &mut span.runs {
                r.fmt.comments.retain(|c| *c != id);
            }
            let len = p.text.len();
            tx.step(Step::Replace { para: pid, offset: 0, remove: len, insert: span })?;
        }
        Ok(sel)
    })?;
    ed.seal();
    Ok(())
}

/// Comments shown highlighted (not resolved).
pub fn open_comments(doc: &Document) -> std::collections::HashSet<NodeId> {
    doc.meta.comments.iter().filter(|c| !c.resolved).map(|c| c.id).collect()
}

/// How tracked changes are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Markup {
    /// Insertions underlined, deletions struck through (and editable).
    #[default]
    All,
    /// As if every change were accepted.
    Final,
    /// As if every change were rejected.
    Original,
}

impl Markup {
    pub const ALL: [Markup; 3] = [Markup::All, Markup::Final, Markup::Original];

    pub fn name(self) -> &'static str {
        match self {
            Markup::All => "All markup",
            Markup::Final => "No markup",
            Markup::Original => "Original",
        }
    }
}

/// `doc` as `markup` shows it (a copy; [`Markup::All`] is the document itself).
pub fn view_as(doc: &Document, markup: Markup) -> Document {
    let mut ed = Editor::new(doc.clone());
    ed.doc.meta.track_changes = false;
    if markup != Markup::All {
        let _ = decide(&mut ed, markup == Markup::Final, true);
    }
    ed.doc
}
