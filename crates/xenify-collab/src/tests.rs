//! Several windows typing into one line of text at once, with messages
//! arriving in any order the network allows, must all end with the
//! server's text.

use super::*;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
enum Text {
    Ins(usize, char),
    Del(usize),
}

impl Transform for Text {
    fn transform(a: &Self, b: &Self, later: bool) -> Vec<Self> {
        use Text::*;
        vec![match (a, b) {
            (Ins(p, c), Ins(q, _)) => Ins(if *p < *q || (*p == *q && !later) { *p } else { p + 1 }, *c),
            (Ins(p, c), Del(q)) => Ins(if *p <= *q { *p } else { p - 1 }, *c),
            (Del(p), Ins(q, _)) => Del(if *p < *q { *p } else { p + 1 }),
            (Del(p), Del(q)) if p == q => return Vec::new(),
            (Del(p), Del(q)) => Del(if *p < *q { *p } else { p - 1 }),
        }]
    }
}

fn apply(s: &mut Vec<char>, op: &Text) {
    match op {
        Text::Ins(p, c) => s.insert(*p, *c),
        Text::Del(p) => {
            s.remove(*p);
        }
    }
}

/// Small, repeatable randomness.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

enum Down {
    Ops(i64, Text),
    Accepted(String, i64, i64),
    Behind(String, Vec<(i64, Text)>),
}

struct Window {
    text: Vec<char>,
    client: Client<Text>,
    up: VecDeque<Batch<Text>>,
    /// Answers to this window's own sends, and everyone's changes, come on
    /// different paths and may overtake each other.
    direct: VecDeque<Down>,
    fanout: VecDeque<Down>,
}

impl Window {
    fn take(&mut self, ops: Vec<Text>, next: Option<Batch<Text>>) {
        for op in &ops {
            apply(&mut self.text, op);
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

fn run(seed: u64, windows: usize, steps: usize) {
    let mut rng = Rng(seed);
    let mut log: Vec<Text> = Vec::new();
    let mut server: Vec<char> = "hello".chars().collect();
    let mut w: Vec<Window> = (0..windows)
        .map(|i| Window {
            text: server.clone(),
            client: Client::new(0, &format!("w{i}")),
            up: VecDeque::new(),
            direct: VecDeque::new(),
            fanout: VecDeque::new(),
        })
        .collect();
    let letters = ['a', 'b', 'c', 'd', 'e', 'f', 'g'];
    for step in 0..steps + 100_000 {
        let quiet = step >= steps;
        let i = rng.below(windows);
        match rng.below(4) {
            0 if !quiet => {
                // Type or delete somewhere.
                let len = w[i].text.len();
                let op = if len > 0 && rng.below(3) == 0 {
                    Text::Del(rng.below(len))
                } else {
                    Text::Ins(rng.below(len + 1), letters[rng.below(letters.len())])
                };
                apply(&mut w[i].text, &op);
                let next = w[i].client.local(vec![op]);
                w[i].up.extend(next);
            }
            1 => {
                // The server takes one batch from window i.
                let Some(b) = w[i].up.pop_front() else { continue };
                let seq = log.len() as i64;
                if b.base != seq {
                    let missed = (b.base..seq).map(|s| (s + 1, log[s as usize].clone())).collect();
                    w[i].direct.push_back(Down::Behind(b.client, missed));
                    continue;
                }
                for op in &b.ops {
                    apply(&mut server, op);
                    log.push(op.clone());
                }
                let to = log.len() as i64;
                w[i].direct.push_back(Down::Accepted(b.client, seq + 1, to));
                for (j, other) in w.iter_mut().enumerate() {
                    if j != i {
                        for (k, op) in b.ops.iter().enumerate() {
                            other.fanout.push_back(Down::Ops(seq + 1 + k as i64, op.clone()));
                        }
                    }
                }
            }
            _ => {
                let from_direct = rng.below(2) == 0;
                let m = if from_direct { w[i].direct.pop_front() } else { w[i].fanout.pop_front() };
                if let Some(m) = m {
                    w[i].deliver(m);
                }
            }
        }
        if quiet && w.iter().all(|x| x.up.is_empty() && x.direct.is_empty() && x.fanout.is_empty()) {
            break;
        }
    }
    for (i, x) in w.iter().enumerate() {
        assert!(x.client.settled(), "seed {seed}: window {i} still has changes out");
        assert_eq!(x.text, server, "seed {seed}: window {i} differs");
        assert_eq!(x.client.seq(), log.len() as i64);
    }
}

#[test]
fn everyone_ends_with_the_same_text() {
    for seed in 1..300 {
        run(seed * 7919, 2 + (seed as usize % 3), 120);
    }
}

#[test]
fn a_lost_batch_is_sent_again_under_its_id() {
    let mut c: Client<Text> = Client::new(4, "w");
    let b = c.local(vec![Text::Ins(0, 'x')]).unwrap();
    assert_eq!((b.base, b.client.as_str()), (4, "w-1"));
    assert!(c.local(vec![Text::Ins(1, 'y')]).is_none(), "one batch out at a time");
    let again = c.resend().unwrap();
    assert_eq!((again.client, again.ops), (b.client.clone(), b.ops));
    let (apply, next) = c.accepted("w-1", 5, 5).unwrap();
    assert!(apply.is_empty());
    assert_eq!(next.unwrap().ops, vec![Text::Ins(1, 'y')]);
    assert_eq!(c.accepted("w-1", 5, 5), Ok((Vec::new(), None)), "an old answer is ignored");
}
