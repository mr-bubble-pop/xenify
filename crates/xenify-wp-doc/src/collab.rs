//! Editing together: [`Change`] is a [`Step`] plus what others need to know
//! to fit their own changes around it, and [`Transform`] for it.
//!
//! Paragraphs are found by id, so most changes only need offsets moved:
//! typing before someone's caret shifts it, Enter moves the rest of the
//! line to a new paragraph, joining two paragraphs moves the second's text
//! onto the end of the first. Blocks are found by index, so inserts and
//! removals in the same list shift each other. Things set as a whole (a
//! style, the page setup, a paragraph's style) go to whoever's change comes
//! later. Anything aimed at what someone else removed is dropped.
//!
//! The adjustments aim to keep everyone's intent; when two changes really
//! clash (both split the same section in the same place) one may be lost.
//! The app also keeps the server's copy, so it can always fall back to it.

use crate::{Block, Container, Document, NodeId, Span, Step};
use serde::{Deserialize, Serialize};
pub use xenify_collab::Transform;

/// One change as sent to the others.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub step: Step,
    #[serde(default, skip_serializing_if = "Ctx::is_empty")]
    pub ctx: Ctx,
}

/// What a step doesn't say but others need: worked out when it's made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ctx {
    /// Split: where the new paragraph went. Join: where `next` was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<(Container, usize)>,
    /// Join: the first paragraph's length before. JoinSection: blocks in
    /// the first section before.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub len: usize,
    /// RemoveBlock, ReplaceBlock: ids that no longer exist after it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gone: Vec<NodeId>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Ctx {
    fn is_empty(&self) -> bool {
        self.place.is_none() && self.len == 0 && self.gone.is_empty()
    }
}

/// Every id inside a block (itself, cells, paragraphs in cells…).
pub fn ids_in(block: &Block, out: &mut Vec<NodeId>) {
    out.push(block.id());
    if let Block::Table(t) = block {
        for row in &t.rows {
            for cell in &row.cells {
                out.push(cell.id);
                for b in &cell.blocks {
                    ids_in(b, out);
                }
            }
        }
    }
}

impl Change {
    /// `step` was just applied to `doc`, and gave `inverse`.
    pub fn record(step: Step, inverse: &Step, doc: &Document) -> Change {
        let mut ctx = Ctx::default();
        match (&step, inverse) {
            (Step::Split { new_id, .. }, _) => ctx.place = doc.locate(*new_id),
            (Step::Join { .. }, Step::Split { offset, place, .. }) => {
                ctx.len = *offset;
                ctx.place = place.clone();
            }
            (Step::RemoveBlock { .. }, Step::InsertBlock { block, .. }) => ids_in(block, &mut ctx.gone),
            (Step::ReplaceBlock { block: new, .. }, Step::ReplaceBlock { block: old, .. }) => {
                let mut keep = Vec::new();
                ids_in(new, &mut keep);
                ids_in(old, &mut ctx.gone);
                ctx.gone.retain(|id| !keep.contains(id));
            }
            (Step::JoinSection { .. }, Step::SplitSection { at, .. }) => ctx.len = *at,
            _ => {}
        }
        Change { step, ctx }
    }

    fn removes(&self, id: NodeId) -> bool {
        match &self.step {
            Step::Join { next, .. } => *next == id,
            Step::RemoveBlock { .. } | Step::ReplaceBlock { .. } => self.ctx.gone.contains(&id),
            _ => false,
        }
    }
}

/// Where a caret at `offset` in paragraph `para` ends up after `b`.
/// `after`: a caret exactly where `b` inserts text goes after it.
/// None: its paragraph is gone.
pub fn map_point(para: NodeId, offset: usize, b: &Change, after: bool) -> Option<(NodeId, usize)> {
    if b.removes(para) && !matches!(b.step, Step::Join { .. }) {
        return None;
    }
    Some(match &b.step {
        Step::Replace { para: bp, offset: bo, remove: br, insert } if *bp == para => {
            let bl = insert.text.len();
            if offset < *bo || (offset == *bo && !after) {
                (para, offset)
            } else if offset >= bo + br && !(offset == *bo && *br == 0 && !after) {
                (para, offset - br + bl)
            } else if after {
                (para, bo + bl)
            } else {
                (para, *bo)
            }
        }
        Step::Split { para: bp, offset: bo, new_id, .. } if *bp == para && offset > *bo => (*new_id, offset - bo),
        Step::Join { para: bp, next, .. } if *next == para => (*bp, offset + b.ctx.len),
        Step::SetContent { .. } => return None,
        _ => (para, offset),
    })
}

/// `span` cut at byte `at` (a character boundary).
fn split_span(span: &Span, at: usize) -> (Span, Span) {
    let at = at.min(span.text.len());
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut pos = 0;
    for r in &span.runs {
        let end = pos + r.len;
        if end <= at {
            left.push(r.clone());
        } else if pos >= at {
            right.push(r.clone());
        } else {
            left.push(crate::Run { len: at - pos, fmt: r.fmt.clone() });
            right.push(crate::Run { len: end - at, fmt: r.fmt.clone() });
        }
        pos = end;
    }
    (Span { text: span.text[..at].to_string(), runs: left }, Span { text: span.text[at..].to_string(), runs: right })
}

fn replace(para: NodeId, offset: usize, remove: usize, insert: Span) -> Change {
    Change { step: Step::Replace { para, offset, remove, insert }, ctx: Ctx::default() }
}

/// `a`, a Replace, after `b`.
fn replace_over(a: &Change, b: &Change, later: bool) -> Vec<Change> {
    let Step::Replace { para, offset: ao, remove: ar, insert } = &a.step else { unreachable!() };
    let (para, ao, ar) = (*para, *ao, *ar);
    match &b.step {
        Step::Replace { para: bp, offset: bo, insert: bi, .. } if *bp == para => {
            let (bo, bl) = (*bo, bi.text.len());
            if ar == 0 {
                // Typing at a point: ties go by order.
                let (_, at) = map_point(para, ao, b, later).unwrap();
                return vec![replace(para, at, 0, insert.clone())];
            }
            // The range after b, less whatever b put in (never delete
            // someone else's new text).
            let s = map_point(para, ao, b, false).unwrap().1;
            let e = map_point(para, ao + ar, b, true).unwrap().1;
            let left = (s < bo).then(|| (s, e.min(bo)));
            let right = (e > bo + bl).then(|| (s.max(bo + bl), e));
            match (left, right) {
                (Some((ls, le)), Some((rs, re))) => {
                    // Formatting a stretch (same length in and out): the
                    // new look goes on both sides.
                    if insert.text.len() == ar {
                        let (li, ri) = split_span(insert, le - ls);
                        if li.text.len() == le - ls && ri.text.len() == re - rs {
                            return vec![replace(para, rs, re - rs, ri), replace(para, ls, le - ls, li)];
                        }
                    }
                    vec![replace(para, rs, re - rs, Span::default()), replace(para, ls, le - ls, insert.clone())]
                }
                (Some((ls, le)), None) => vec![replace(para, ls, le - ls, insert.clone())],
                (None, Some((rs, re))) => vec![replace(para, rs, re - rs, insert.clone())],
                (None, None) => {
                    // b replaced all of it: keep a's text next to b's.
                    if insert.text.is_empty() {
                        Vec::new()
                    } else {
                        vec![replace(para, if later { bo + bl } else { bo }, 0, insert.clone())]
                    }
                }
            }
        }
        Step::Split { para: bp, offset: bo, new_id, .. } if *bp == para => {
            let bo = *bo;
            let end = ao + ar;
            if end <= bo {
                // Text typed right where the line was broken stays on the
                // first line.
                vec![a.clone()]
            } else if ao >= bo && !(ao == bo && ar == 0) {
                vec![replace(*new_id, ao - bo, ar, insert.clone())]
            } else {
                vec![replace(*new_id, 0, end - bo, Span::default()), replace(para, ao, bo - ao, insert.clone())]
            }
        }
        Step::Join { para: bp, next, .. } if *next == para => vec![replace(*bp, ao + b.ctx.len, ar, insert.clone())],
        Step::SetContent { .. } => Vec::new(),
        _ if b.removes(para) => Vec::new(),
        _ => vec![a.clone()],
    }
}

/// Moves a block position (container, index) past `b`'s insert or removal
/// in the same container. `inserting`: `a` itself inserts there; on a tie
/// it goes after `b`'s new block if `later`. A split's new paragraph always
/// sits right after its first half, so it goes first in a tie either way.
fn shift(c: &Container, i: usize, inserting: bool, later: bool, b: &Change) -> Option<(Container, usize)> {
    let b_splits = matches!(b.step, Step::Split { .. });
    let later = later || b_splits;
    let (bc, bi, ins) = match (&b.step, &b.ctx.place) {
        (Step::InsertBlock { container, index, .. }, _) => (container.clone(), *index, true),
        (Step::RemoveBlock { container, index }, _) => (container.clone(), *index, false),
        (Step::Split { .. }, Some((pc, pi))) => (pc.clone(), *pi, true),
        (Step::Join { .. }, Some((pc, pi))) => (pc.clone(), *pi, false),
        (Step::SplitSection { section, at, .. }, _) => {
            return Some(match c {
                Container::Section(s) if s > section => (Container::Section(s + 1), i),
                Container::Section(s) if s == section && (i > *at || (i == *at && (!inserting || later))) => {
                    (Container::Section(s + 1), i - at)
                }
                _ => (c.clone(), i),
            });
        }
        (Step::JoinSection { section }, _) => {
            return Some(match c {
                Container::Section(s) if *s == section + 1 => (Container::Section(*section), i + b.ctx.len),
                Container::Section(s) if *s > section + 1 => (Container::Section(s - 1), i),
                _ => (c.clone(), i),
            });
        }
        _ => return Some((c.clone(), i)),
    };
    if bc != *c {
        return Some((c.clone(), i));
    }
    Some(if ins {
        if i > bi || (i == bi && (!inserting || later)) { (c.clone(), i + 1) } else { (c.clone(), i) }
    } else if i > bi {
        (c.clone(), i - 1)
    } else if i == bi && !inserting {
        // The same block: someone removed it.
        return None;
    } else {
        (c.clone(), i)
    })
}

/// Whether `c` sits inside something `b` removed.
fn container_gone(c: &Container, b: &Change) -> bool {
    match c {
        Container::Cell { table, .. } => b.removes(*table),
        Container::Frame(id) | Container::Note(id) | Container::Header(id) => b.removes(*id),
        Container::Section(_) => false,
    }
}

/// Moves a section number past `b`. None: that section went (joined in).
fn section_over(s: usize, b: &Change) -> Option<usize> {
    match &b.step {
        Step::SplitSection { section, .. } if s > *section => Some(s + 1),
        Step::JoinSection { section } if s == section + 1 => None,
        Step::JoinSection { section } if s > section + 1 => Some(s - 1),
        _ => Some(s),
    }
}

fn same_value(a: &Step, b: &Step) -> bool {
    use Step::*;
    match (a, b) {
        (SetStyle { para: x, .. }, SetStyle { para: y, .. })
        | (SetProps { para: x, .. }, SetProps { para: y, .. })
        | (SetList { para: x, .. }, SetList { para: y, .. }) => x == y,
        (SetStyleDef { id: x, .. }, SetStyleDef { id: y, .. })
        | (SetListDef { id: x, .. }, SetListDef { id: y, .. }) => x == y,
        (SetMeta { .. }, SetMeta { .. }) | (SetContent { .. }, SetContent { .. }) => true,
        (SetFrames { section: x, .. }, SetFrames { section: y, .. })
        | (SetSection { section: x, .. }, SetSection { section: y, .. })
        | (SetNotes { section: x, .. }, SetNotes { section: y, .. })
        | (SetHeaders { section: x, .. }, SetHeaders { section: y, .. }) => x == y,
        (ReplaceBlock { container: c1, index: i1, .. }, ReplaceBlock { container: c2, index: i2, .. }) => {
            c1 == c2 && i1 == i2
        }
        _ => false,
    }
}

impl Transform for Change {
    fn transform(a: &Change, b: &Change, later: bool) -> Vec<Change> {
        use Step::*;
        // Setting the same thing: the later one wins.
        if same_value(&a.step, &b.step) {
            return if later { vec![a.clone()] } else { Vec::new() };
        }
        // Everything restored to an earlier version: nothing else fits on it.
        if matches!(b.step, SetContent { .. }) {
            return Vec::new();
        }
        if matches!(a.step, SetContent { .. }) {
            return vec![a.clone()];
        }
        let mut out = a.clone();
        match &a.step {
            Replace { .. } => return replace_over(a, b, later),
            Split { para, offset, new_id, style, props, list, .. } => {
                if b.removes(*para) && !matches!(b.step, Join { .. }) {
                    return Vec::new();
                }
                let (p, o) = match &b.step {
                    // Splitting where someone else did too: theirs first,
                    // ours in the (empty) gap after it.
                    Split { para: bp, offset: bo, new_id: bn, .. } if bp == para && bo == offset => {
                        if later {
                            (*bn, 0)
                        } else {
                            (*para, *offset)
                        }
                    }
                    _ => map_point(*para, *offset, b, true).unwrap_or((*para, *offset)),
                };
                out.step = Split {
                    para: p,
                    offset: o,
                    new_id: *new_id,
                    style: style.clone(),
                    props: props.clone(),
                    list: list.clone(),
                    place: None,
                };
                if let Some((c, i)) = &a.ctx.place {
                    out.ctx.place = match &b.step {
                        // Both broke this paragraph: ours now breaks theirs
                        // (and goes after it) or stays in front.
                        Split { para: bp, new_id: bn, .. }
                            if bp == para && b.ctx.place.as_ref() == Some(&(c.clone(), *i)) =>
                        {
                            Some((c.clone(), if p == *bn { i + 1 } else { *i }))
                        }
                        _ => shift(c, *i, true, false, b),
                    };
                }
            }
            Join { para: ap, next } => {
                if b.removes(*next) || (b.removes(*ap) && !matches!(b.step, Join { .. })) {
                    return Vec::new();
                }
                let (para, len) = match &b.step {
                    Replace { para: bp, remove, insert, .. } if bp == ap => {
                        (*ap, a.ctx.len - remove + insert.text.len())
                    }
                    // Our first paragraph was split: join to its second half.
                    Split { para: bp, offset, new_id, .. } if bp == ap => (*new_id, a.ctx.len.saturating_sub(*offset)),
                    // Our first paragraph was joined onto another.
                    Join { para: bp, next: bn } if bn == ap => (*bp, b.ctx.len + a.ctx.len),
                    _ => (*ap, a.ctx.len),
                };
                out.step = Join { para, next: *next };
                out.ctx.len = len;
                if let Some((c, i)) = &a.ctx.place {
                    out.ctx.place = shift(c, *i, false, later, b);
                }
            }
            SetStyle { para, .. } | SetProps { para, .. } | SetList { para, .. } => {
                if b.removes(*para) {
                    return Vec::new();
                }
            }
            InsertBlock { container, index, block } | ReplaceBlock { container, index, block } => {
                if container_gone(container, b) {
                    return Vec::new();
                }
                let inserting = matches!(a.step, InsertBlock { .. });
                let Some((c, i)) = shift(container, *index, inserting, later, b) else { return Vec::new() };
                out.step = if inserting {
                    InsertBlock { container: c, index: i, block: block.clone() }
                } else {
                    ReplaceBlock { container: c, index: i, block: block.clone() }
                };
            }
            RemoveBlock { container, index } => {
                if container_gone(container, b) {
                    return Vec::new();
                }
                let Some((c, i)) = shift(container, *index, false, later, b) else { return Vec::new() };
                out.step = RemoveBlock { container: c, index: i };
            }
            SplitSection { section, at, id, start, page, frames } => {
                let (s, at) = match &b.step {
                    SplitSection { section: bs, at: bat, .. } if bs == section => match at.cmp(bat) {
                        std::cmp::Ordering::Equal => return Vec::new(),
                        std::cmp::Ordering::Greater => (section + 1, at - bat),
                        std::cmp::Ordering::Less => (*section, *at),
                    },
                    JoinSection { section: bs } if *section == bs + 1 => (*bs, at + b.ctx.len),
                    _ => {
                        let Some(s) = section_over(*section, b) else { return Vec::new() };
                        // Blocks added or taken out before the split point move it.
                        let (moved, ins) = match (&b.step, &b.ctx.place) {
                            (InsertBlock { container, index, .. }, _) => (Some((container, *index)), true),
                            (RemoveBlock { container, index }, _) => (Some((container, *index)), false),
                            (Split { .. }, Some((c, i))) => (Some((c, *i)), true),
                            (Join { .. }, Some((c, i))) => (Some((c, *i)), false),
                            _ => (None, false),
                        };
                        let at = match moved {
                            Some((Container::Section(bs), bi)) if bs == section && bi < *at => {
                                if ins {
                                    at + 1
                                } else {
                                    at - 1
                                }
                            }
                            _ => *at,
                        };
                        (s, at)
                    }
                };
                out.step =
                    SplitSection { section: s, at, id: *id, start: *start, page: page.clone(), frames: frames.clone() };
            }
            JoinSection { section } => match &b.step {
                JoinSection { section: bs } if bs == section => return Vec::new(),
                _ => {
                    let Some(s) = section_over(*section, b) else { return Vec::new() };
                    if section_over(section + 1, b).is_none() {
                        return Vec::new();
                    }
                    out.step = JoinSection { section: s };
                }
            },
            SetFrames { section, .. }
            | SetSection { section, .. }
            | SetNotes { section, .. }
            | SetHeaders { section, .. } => {
                let Some(s) = section_over(*section, b) else { return Vec::new() };
                match &mut out.step {
                    SetFrames { section, .. }
                    | SetSection { section, .. }
                    | SetNotes { section, .. }
                    | SetHeaders { section, .. } => *section = s,
                    _ => unreachable!(),
                }
            }
            SetStyleDef { .. } | SetListDef { .. } | SetMeta { .. } | SetContent { .. } => {}
        }
        vec![out]
    }
}

/// Whether two documents hold the same thing, as saved (layout revisions
/// and who's editing don't count).
pub fn same_content(x: &Document, y: &Document) -> bool {
    let saved = |d: &Document| serde_json::to_value((&d.sections, &d.styles, &d.lists, &d.meta)).ok();
    saved(x) == saved(y)
}

#[cfg(test)]
mod tests;
