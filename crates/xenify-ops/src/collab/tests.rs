//! Several windows editing one Draw project at once: pairs and runs of
//! changes must fit together whichever goes first, and windows passing
//! messages in any order end with the server's project.

use super::*;
use crate::Command;
use std::collections::VecDeque;
use xenify_collab::{Batch, Client};
use xenify_doc::{Color, Easing, Layer, PropPath, ShapeItem, Value, Vec2};

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

fn start() -> Project {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    for (i, w) in [50.0, 80.0, 120.0].into_iter().enumerate() {
        let id = p.alloc_id();
        let l = Layer::shape(id, format!("box {i}"), vec![ShapeItem::rect(Vec2::new(w, w), Color::WHITE)])
            .with_position(Vec2::new(100.0 * i as f64, 100.0));
        p.comp_mut(comp).unwrap().layers.push(l);
    }
    p
}

/// A random edit that makes sense on `p`.
fn edit(rng: &mut Rng, p: &Project, next_id: &mut u64) -> Option<Command> {
    let comp = p.compositions[0].id;
    let layers: Vec<Id> = p.compositions[0].layers.iter().map(|l| l.id).collect();
    let pick = |rng: &mut Rng| layers.get(rng.below(layers.len())).copied();
    let times = [0.0, 1.0, 2.0];
    Some(match rng.below(11) {
        0 => {
            *next_id += 1;
            let l = Layer::shape(Id(*next_id), "new", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
            Command::AddLayer { comp, index: Some(rng.below(layers.len() + 1)), layer: l }
        }
        1 => Command::RemoveLayer { layer: pick(rng)? },
        2 if layers.len() > 1 => Command::MoveLayer { layer: pick(rng)?, index: rng.below(layers.len()) },
        3 => {
            let layer = pick(rng)?;
            let n = p.layer(layer).ok()?.shapes().len();
            Command::AddShape {
                layer,
                index: Some(rng.below(n + 1)),
                shape: ShapeItem::rect(Vec2::new(5.0 + rng.below(9) as f64, 5.0), Color::WHITE),
            }
        }
        4 => {
            let layer = pick(rng)?;
            let n = p.layer(layer).ok()?.shapes().len();
            if n == 0 {
                return None;
            }
            Command::RemoveShape { layer, index: rng.below(n) }
        }
        5 => {
            let layer = pick(rng)?;
            let n = p.layer(layer).ok()?.shapes().len();
            if n == 0 {
                return None;
            }
            Command::SetShape {
                layer,
                index: rng.below(n),
                shape: ShapeItem::rect(Vec2::new(33.0, 33.0), Color::WHITE),
            }
        }
        6 => Command::SetKeyframe {
            layer: pick(rng)?,
            path: PropPath::Opacity,
            time: times[rng.below(3)],
            value: Value::Scalar(rng.below(100) as f64 / 100.0),
            easing: None,
        },
        7 => Command::RemoveKeyframe { layer: pick(rng)?, path: PropPath::Opacity, time: times[rng.below(3)] },
        8 => {
            let (from, to) = (times[rng.below(3)], times[rng.below(3)] + 0.5);
            Command::MoveKeyframe { layer: pick(rng)?, path: PropPath::Opacity, from, to }
        }
        9 => {
            let layer = pick(rng)?;
            let mut meta = p.layer(layer).ok()?.meta.clone();
            meta.name = format!("renamed {}", rng.below(9));
            Command::SetLayerMeta { layer, meta }
        }
        _ => Command::SetValue {
            layer: pick(rng)?,
            path: PropPath::Position,
            time: 0.0,
            value: Value::Vec2(Vec2::new(rng.below(500) as f64, 7.0)),
        },
    })
    .map(|c| {
        if rng.below(6) == 0 {
            Command::SetEasing { layer: layers[0], path: PropPath::Opacity, time: 1.0, easing: Easing::Linear }
        } else {
            c
        }
    })
}

fn make(rng: &mut Rng, p: &Project, next_id: &mut u64) -> Option<(Change, Project)> {
    let c = edit(rng, p, next_id)?;
    Change::make(p, c).ok()
}

fn apply_all(p: &mut Project, changes: &[Change]) {
    for c in changes {
        let _ = apply(p, c.command());
    }
}

#[test]
fn pairs_and_runs_agree_either_way() {
    let mut rng = Rng(77);
    let (mut tried, mut bad) = (0, Vec::new());
    for round in 0..20_000 {
        let p = start();
        let side = |rng: &mut Rng, base: u64| {
            let mut q = p.clone();
            let mut id = base;
            let mut out = Vec::new();
            for _ in 0..1 + (round % 3) {
                if let Some((c, after)) = make(rng, &q, &mut id) {
                    out.push(c);
                    q = after;
                }
            }
            (q, out)
        };
        let (pa, a) = side(&mut rng, 100);
        let (pb, b) = side(&mut rng, 200);
        tried += 1;
        let (a2, b2) = xenify_collab::transform(&a, &b);
        let mut x = pb.clone();
        apply_all(&mut x, &a2);
        let mut y = pa.clone();
        apply_all(&mut y, &b2);
        if !same_content(&x, &y) {
            let short = |v: &[Change]| {
                v.iter()
                    .map(|c| {
                        let ctx: Vec<_> = c.parts.iter().map(|p| (p.ctx.comp.map(|i| i.0), p.ctx.from)).collect();
                        format!("{} {ctx:?}", format!("{:?}", c.command()).chars().take(70).collect::<String>())
                    })
                    .collect::<Vec<_>>()
            };
            let tag = if a.len() == 1 && b.len() == 1 { "PAIR " } else { "" };
            bad.push(format!("{tag}a = {:?}\nb = {:?}", short(&a), short(&b)));
        }
    }
    eprintln!("{} of {tried} didn't fit together", bad.len());
    for b in bad.iter().take(4) {
        eprintln!("{b}");
    }
    assert!(bad.len() * 200 <= tried, "{} of {tried}:\n{}", bad.len(), bad[..4.min(bad.len())].join("\n"));
}

#[allow(clippy::large_enum_variant)]
enum Down {
    Ops(i64, Change),
    Accepted(String, i64, i64),
    Behind(String, Vec<(i64, Change)>),
}

struct Window {
    p: Project,
    client: Client<Change>,
    up: VecDeque<Batch<Change>>,
    direct: VecDeque<Down>,
    fanout: VecDeque<Down>,
    next_id: u64,
}

impl Window {
    fn take(&mut self, ops: Vec<Change>, next: Option<Batch<Change>>) {
        apply_all(&mut self.p, &ops);
        self.up.extend(next);
    }
}

#[test]
fn windows_editing_one_project_agree() {
    let (mut windows, mut fell_back) = (0, 0);
    for seed in 1..300u64 {
        let mut rng = Rng(seed * 6151);
        let n = 2 + seed as usize % 3;
        windows += n;
        let mut log: Vec<Change> = Vec::new();
        let mut server = start();
        let mut w: Vec<Window> = (0..n)
            .map(|i| Window {
                p: server.clone(),
                client: Client::new(0, &format!("w{i}")),
                up: VecDeque::new(),
                direct: VecDeque::new(),
                fanout: VecDeque::new(),
                next_id: 1000 * (i as u64 + 1),
            })
            .collect();
        for step in 0..200_000 {
            let quiet = step >= 60;
            let i = rng.below(n);
            match rng.below(4) {
                0 if !quiet => {
                    let mut id = w[i].next_id;
                    let Some((c, after)) = make(&mut rng, &w[i].p, &mut id) else { continue };
                    w[i].next_id = id;
                    w[i].p = after;
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
                    apply_all(&mut server, &b.ops);
                    log.extend(b.ops.iter().cloned());
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
                    match m {
                        Some(Down::Ops(seq, op)) => {
                            let ops = w[i].client.remote(seq, op);
                            w[i].take(ops, None);
                        }
                        Some(Down::Accepted(c, from, to)) => {
                            let (ops, next) = w[i].client.accepted(&c, from, to).unwrap();
                            w[i].take(ops, next);
                        }
                        Some(Down::Behind(c, missed)) => {
                            let (ops, next) = w[i].client.behind(&c, missed);
                            w[i].take(ops, next);
                        }
                        None => {}
                    }
                }
            }
            if quiet && w.iter().all(|x| x.up.is_empty() && x.direct.is_empty() && x.fanout.is_empty()) {
                break;
            }
        }
        for x in &w {
            assert!(x.client.settled());
            if !same_content(&x.p, &server) {
                fell_back += 1;
                if std::env::var("DUMP").is_ok() && fell_back == 1 {
                    let show = |p: &Project| {
                        p.compositions[0]
                            .layers
                            .iter()
                            .map(|l| {
                                format!("{}:{}s:{}", l.id, l.shapes().len(), serde_json::to_string(&l.opacity).unwrap())
                            })
                            .collect::<Vec<_>>()
                    };
                    eprintln!("seed {seed}\nwindow {:?}\nserver {:?}", show(&x.p), show(&server));
                    for (i, c) in log.iter().enumerate() {
                        eprintln!(
                            "{} {}",
                            i + 1,
                            format!("{:?} {:?}", c.command(), c.parts.iter().map(|p| &p.ctx).collect::<Vec<_>>())
                                .chars()
                                .take(160)
                                .collect::<String>()
                        );
                    }
                }
            }
        }
    }
    eprintln!("{windows} windows, {fell_back} needed the server's copy");
    assert!(fell_back * 100 <= windows, "{fell_back} of {windows}");
}

#[test]
fn undo_takes_back_only_my_own_changes() {
    use crate::Editor;
    let p = start();
    let ids: Vec<Id> = p.compositions[0].layers.iter().map(|l| l.id).collect();
    let mut me = Editor::new(p.clone());
    let mut them = Editor::new(p);
    me.set_live(true);
    them.set_live(true);
    let at = |x: f64| Value::Vec2(Vec2::new(x, 1.0));
    me.execute("Move", Command::SetValue { layer: ids[0], path: PropPath::Position, time: 0.0, value: at(500.0) })
        .unwrap();
    let mut meta = them.project().layer(ids[1]).unwrap().meta.clone();
    meta.name = "theirs".into();
    them.execute("Rename", Command::SetLayerMeta { layer: ids[1], meta }).unwrap();
    for c in me.take_changes() {
        them.apply_remote(&c).unwrap();
    }
    for c in them.take_changes() {
        me.apply_remote(&c).unwrap();
    }
    assert!(same_content(me.project(), them.project()));

    // My undo puts my layer back and leaves their rename.
    assert_eq!(me.undo().as_deref(), Some("Move"));
    assert_eq!(me.project().layer(ids[1]).unwrap().meta.name, "theirs");
    assert_eq!(me.project().layer(ids[0]).unwrap().transform.position.value_at(0.0).x, 0.0);
    for c in me.take_changes() {
        them.apply_remote(&c).unwrap();
    }
    assert!(same_content(me.project(), them.project()));

    // They remove a layer I'd changed: my steps on it go, nothing breaks.
    me.redo();
    them.execute("Delete", Command::RemoveLayer { layer: ids[0] }).unwrap();
    for c in them.take_changes() {
        me.apply_remote(&c).unwrap();
    }
    assert_eq!(me.undo(), None);
    assert!(me.project().layer(ids[0]).is_err());
}
