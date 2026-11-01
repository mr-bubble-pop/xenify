//! Several windows editing one Write document at once, messages arriving
//! in any order the network allows. Each window ends with the server's
//! document; the test also counts how often a window had to fall back to
//! the server's copy (the adjustments should make that rare).

use super::*;
use crate::{Block, Document, NodeId, Paragraph, Span, Step};
use std::collections::VecDeque;
use xenify_collab::{Batch, Client};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn start() -> Document {
    let mut d = Document::new();
    let ids: Vec<NodeId> = (0..4).map(|_| d.alloc_id()).collect();
    d.sections[0].blocks = ["alpha beta", "gamma", "delta epsilon", "zeta"]
        .iter()
        .zip(&ids)
        .map(|(t, id)| Block::Paragraph(Paragraph::new(*id, "Normal", *t)))
        .collect();
    d
}

/// A random edit that makes sense on `doc`.
fn edit(rng: &mut Rng, doc: &Document, next_id: &mut u64) -> Option<Step> {
    let paras: Vec<&Paragraph> = doc.sections[0]
        .blocks
        .iter()
        .filter_map(|b| if let Block::Paragraph(p) = b { Some(p) } else { None })
        .collect();
    let p = paras.get(rng.below(paras.len()))?;
    let len = p.text.len();
    let words = ["x", "yy", "zzz", " ", "q"];
    Some(match rng.below(10) {
        0..=3 => {
            let at = rng.below(len + 1);
            let w = words[rng.below(words.len())];
            Step::Replace { para: p.id, offset: at, remove: 0, insert: Span::plain(w, Default::default()) }
        }
        4 | 5 if len > 0 => {
            let at = rng.below(len);
            let n = 1 + rng.below((len - at).min(4));
            Step::Replace { para: p.id, offset: at, remove: n, insert: Span::default() }
        }
        6 => {
            *next_id += 1;
            Step::Split {
                para: p.id,
                offset: rng.below(len + 1),
                new_id: NodeId(*next_id),
                style: "Normal".into(),
                props: Default::default(),
                list: None,
                place: None,
            }
        }
        7 => {
            // Backspace at the start of a paragraph.
            let i = paras.iter().position(|q| q.id == p.id)?;
            let prev = paras.get(i.checked_sub(1)?)?;
            let bi = doc.sections[0].blocks.iter().position(|b| b.id() == p.id)?;
            let bp = doc.sections[0].blocks.iter().position(|b| b.id() == prev.id)?;
            if bp + 1 != bi {
                return None;
            }
            Step::Join { para: prev.id, next: p.id }
        }
        8 => Step::SetStyle { para: p.id, style: ["Heading1", "Normal", "Quote"][rng.below(3)].into() },
        _ => {
            *next_id += 1;
            let n = doc.sections[0].blocks.len();
            Step::InsertBlock {
                container: crate::Container::Section(0),
                index: rng.below(n + 1),
                block: Block::Paragraph(Paragraph::new(NodeId(*next_id), "Normal", "new")),
            }
        }
    })
}

#[allow(clippy::large_enum_variant)]
enum Down {
    Ops(i64, Change),
    Accepted(String, i64, i64),
    Behind(String, Vec<(i64, Change)>),
}

struct Window {
    doc: Document,
    client: Client<Change>,
    up: VecDeque<Batch<Change>>,
    direct: VecDeque<Down>,
    fanout: VecDeque<Down>,
    next_id: u64,
}

impl Window {
    fn take(&mut self, ops: Vec<Change>, next: Option<Batch<Change>>) {
        for c in ops {
            let _ = c.step.apply(&mut self.doc);
        }
        self.up.extend(next);
    }

    fn deliver(&mut self, m: Down) {
        match m {
            Down::Ops(seq, op) => {
                let ops = self.client.remote(seq, op);
                self.take(ops, None);
            }
            Down::Accepted(c, from, to) => {
                let (ops, next) = self.client.accepted(&c, from, to).unwrap();
                self.take(ops, next);
            }
            Down::Behind(c, missed) => {
                let (ops, next) = self.client.behind(&c, missed);
                self.take(ops, next);
            }
        }
    }
}

/// Returns how many windows needed the server's copy at the end.
fn run(seed: u64, windows: usize, steps: usize) -> usize {
    let mut rng = Rng(seed);
    let mut log: Vec<Change> = Vec::new();
    let mut server = start();
    let mut w: Vec<Window> = (0..windows)
        .map(|i| Window {
            doc: server.clone(),
            client: Client::new(0, &format!("w{i}")),
            up: VecDeque::new(),
            direct: VecDeque::new(),
            fanout: VecDeque::new(),
            next_id: 1000 * (i as u64 + 1),
        })
        .collect();
    for step in 0..steps + 200_000 {
        let quiet = step >= steps;
        let i = rng.below(windows);
        match rng.below(4) {
            0 if !quiet => {
                let mut id = w[i].next_id;
                let Some(s) = edit(&mut rng, &w[i].doc, &mut id) else { continue };
                w[i].next_id = id;
                let Ok(inv) = s.clone().apply(&mut w[i].doc) else { continue };
                let c = Change::record(s, &inv, &w[i].doc);
                let next = w[i].client.local(vec![c]);
                w[i].up.extend(next);
            }
            1 => {
                let Some(b) = w[i].up.pop_front() else { continue };
                let seq = log.len() as i64;
                if b.base != seq {
                    let missed = (b.base..seq).map(|s| (s + 1, log[s as usize].clone())).collect();
                    w[i].direct.push_back(Down::Behind(b.client, missed));
                    continue;
                }
                for op in &b.ops {
                    let _ = op.step.clone().apply(&mut server);
                    log.push(op.clone());
                }
                w[i].direct.push_back(Down::Accepted(b.client, seq + 1, log.len() as i64));
                for (j, other) in w.iter_mut().enumerate() {
                    if j != i {
                        for (k, op) in b.ops.iter().enumerate() {
                            other.fanout.push_back(Down::Ops(seq + 1 + k as i64, op.clone()));
                        }
                    }
                }
            }
            _ => {
                let m = if rng.below(2) == 0 { w[i].direct.pop_front() } else { w[i].fanout.pop_front() };
                if let Some(m) = m {
                    w[i].deliver(m);
                }
            }
        }
        if quiet && w.iter().all(|x| x.up.is_empty() && x.direct.is_empty() && x.fanout.is_empty()) {
            break;
        }
    }
    let mut fell_back = 0;
    for x in &mut w {
        assert!(x.client.settled(), "seed {seed}: changes still out");
        if !same_content(&x.doc, &server) {
            fell_back += 1;
            x.doc = server.clone();
        }
    }
    fell_back
}

#[test]
fn windows_editing_one_document_agree() {
    let (mut runs, mut fell_back, mut windows) = (0, 0, 0);
    for seed in 1..400u64 {
        let n = 2 + (seed as usize % 3);
        fell_back += run(seed * 104_729, n, 60);
        runs += 1;
        windows += n;
    }
    eprintln!("{runs} runs, {windows} windows, {fell_back} needed the server's copy");
    assert!(fell_back * 100 <= windows, "too many windows disagreed: {fell_back} of {windows}");
}

#[test]
fn typing_moves_around_someone_elses_enter() {
    let doc = start();
    let p = doc.sections[0].blocks[0].id();
    // One person presses Enter after "alpha"; the other types at the end.
    let enter = Change {
        step: Step::Split {
            para: p,
            offset: 5,
            new_id: NodeId(500),
            style: "Normal".into(),
            props: Default::default(),
            list: None,
            place: None,
        },
        ctx: Ctx { place: Some((crate::Container::Section(0), 1)), ..Default::default() },
    };
    let typing = Change {
        step: Step::Replace { para: p, offset: 10, remove: 0, insert: Span::plain("!", Default::default()) },
        ctx: Ctx::default(),
    };
    let t = Change::transform(&typing, &enter, true);
    assert_eq!(
        t[0].step,
        Step::Replace { para: NodeId(500), offset: 5, remove: 0, insert: Span::plain("!", Default::default()) }
    );
    let mut a = doc.clone();
    enter.step.clone().apply(&mut a).unwrap();
    t[0].step.clone().apply(&mut a).unwrap();
    let mut b = doc.clone();
    typing.step.clone().apply(&mut b).unwrap();
    for c in Change::transform(&enter, &typing, false) {
        c.step.apply(&mut b).unwrap();
    }
    assert!(same_content(&a, &b));
    assert_eq!(a.para(NodeId(500)).unwrap().text, " beta!");
}

/// Two changes made on the same document give the same result whichever
/// goes first (after adjusting the other).
#[test]
fn pairs_agree_either_way() {
    let mut rng = Rng(99);
    let mut bad = Vec::new();
    for _ in 0..20_000 {
        let mut doc = start();
        // Vary the start a little.
        let mut id = 5000;
        for _ in 0..rng.below(3) {
            if let Some(s) = edit(&mut rng, &doc, &mut id) {
                let _ = s.apply(&mut doc);
            }
        }
        let (mut ia, mut ib) = (100, 200);
        let (Some(sa), Some(sb)) = (edit(&mut rng, &doc, &mut ia), edit(&mut rng, &doc, &mut ib)) else { continue };
        let mut da = doc.clone();
        let Ok(inv) = sa.clone().apply(&mut da) else { continue };
        let a = Change::record(sa, &inv, &da);
        let mut db = doc.clone();
        let Ok(inv) = sb.clone().apply(&mut db) else { continue };
        let b = Change::record(sb, &inv, &db);
        // Server order: b, then a.
        let mut x = db.clone();
        for c in Change::transform(&a, &b, true) {
            let _ = c.step.apply(&mut x);
        }
        let mut y = da.clone();
        for c in Change::transform(&b, &a, false) {
            let _ = c.step.apply(&mut y);
        }
        if !same_content(&x, &y) && bad.len() < 6 {
            bad.push(format!("a = {:?}\nb = {:?}\n", a, b));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The same for short runs of changes on each side. A few runs can't fit
/// together exactly (one joins two paragraphs while the other puts a block
/// between them); the app falls back to the server's copy then. They must
/// stay rare.
#[test]
fn runs_agree_either_way() {
    let mut rng = Rng(4242);
    let mut bad = Vec::new();
    let mut tried = 0;
    for _ in 0..20_000 {
        let doc = start();
        let side = |rng: &mut Rng, base: u64| {
            let mut d = doc.clone();
            let mut id = base;
            let mut out = Vec::new();
            for _ in 0..1 + rng.below(3) {
                if let Some(s) = edit(rng, &d, &mut id)
                    && let Ok(inv) = s.clone().apply(&mut d)
                {
                    out.push(Change::record(s, &inv, &d));
                }
            }
            (d, out)
        };
        let (da, a) = side(&mut rng, 100);
        let (db, b) = side(&mut rng, 200);
        tried += 1;
        let (a2, b2) = xenify_collab::transform(&a, &b);
        let mut x = db.clone();
        for c in &a2 {
            let _ = c.step.clone().apply(&mut x);
        }
        let mut y = da.clone();
        for c in &b2 {
            let _ = c.step.clone().apply(&mut y);
        }
        if !same_content(&x, &y) {
            let short =
                |v: &[Change]| v.iter().map(|c| format!("{:?} {:?}", short_step(&c.step), c.ctx)).collect::<Vec<_>>();
            bad.push(format!("a = {:?}\nb = {:?}\n", short(&a), short(&b)));
        }
    }
    eprintln!("{} of {tried} runs didn't fit together", bad.len());
    assert!(bad.len() * 200 <= tried, "{} of {tried}:\n{}", bad.len(), bad[..3.min(bad.len())].join("\n"));
}

fn short_step(s: &Step) -> String {
    match s {
        Step::Replace { para, offset, remove, insert } => {
            format!("Replace({},{offset},{remove},{:?})", para.0, insert.text)
        }
        Step::Split { para, offset, new_id, .. } => format!("Split({},{offset}->{})", para.0, new_id.0),
        Step::Join { para, next } => format!("Join({},{})", para.0, next.0),
        Step::SetStyle { para, style } => format!("Style({},{style})", para.0),
        Step::InsertBlock { index, block, .. } => format!("Insert({index},{})", block.id().0),
        other => format!("{other:?}").chars().take(60).collect(),
    }
}

#[test]
fn undo_takes_back_only_my_own_typing() {
    use crate::{Editor, Pos, Selection};
    let doc = start();
    let first = doc.sections[0].blocks[0].id();
    let mut me = Editor::new(doc.clone());
    let mut them = Editor::new(doc);
    me.set_live(true);
    them.set_live(true);
    let typing = |ed: &mut Editor, at: usize, text: &str| {
        ed.transact("Typing", None, |tx| {
            tx.step(Step::Replace {
                para: first,
                offset: at,
                remove: 0,
                insert: Span::plain(text, Default::default()),
            })?;
            Ok(Selection::caret(Pos::new(first, at + text.len())))
        })
        .unwrap();
    };
    typing(&mut me, 0, "My ");
    typing(&mut them, 10, " theirs");
    // Server order: mine, then theirs (adjusted to come after mine).
    let mine = me.take_changes();
    let theirs = them.take_changes();
    let (theirs2, mine2) = xenify_collab::transform(&theirs, &mine);
    for c in &theirs2 {
        me.apply_remote(c).unwrap();
    }
    for c in &mine2 {
        them.apply_remote(c).unwrap();
    }
    assert_eq!(me.doc.para(first).unwrap().text, "My alpha beta theirs");
    assert!(same_content(&me.doc, &them.doc));
    // My caret stayed after my text.
    assert_eq!(me.sel.focus, Pos::new(first, 3));

    assert!(me.undo());
    assert_eq!(me.doc.para(first).unwrap().text, "alpha beta theirs");
    for c in me.take_changes() {
        them.apply_remote(&c).unwrap();
    }
    assert!(same_content(&me.doc, &them.doc));
}
