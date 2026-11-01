//! Editing a plan together: one person's step adjusted to come after
//! someone else's made at the same time (xenify-collab's `Transform`).
//!
//! - Two people setting the same thing (a task's title, its people, a
//!   status in the list, a setting): the later one wins.
//! - Edits to a task someone deleted are dropped; a task placed next to one
//!   that went goes to the end; a parent that went leaves the task at the top.
//! - Two new tasks given the same key at once: the later gets the next number.
//!
//! Dates are never stored (schedule.rs), so links and moves can't disagree.
//! Whatever slips through still ends the same everywhere: each copy keeps
//! the server's copy beside its own and takes it when they differ (remote.rs).

use crate::model::Step;
use crate::ops::Op;
use serde_json::Value as Json;
use xenify_collab::Transform;

fn same_target(a: &Op, b: &Op) -> bool {
    match (a, b) {
        (Op::SetTask { id: x, field: f, .. }, Op::SetTask { id: y, field: g, .. }) => x == y && f == g,
        (Op::Place { id: x, .. }, Op::Place { id: y, .. }) => x == y,
        (Op::SetPlan { field: f, .. }, Op::SetPlan { field: g, .. }) => f == g,
        (Op::SetItem { list: l, id: x, .. }, Op::SetItem { list: m, id: y, .. }) => l == m && x == y,
        (Op::DeleteTask { id: x }, Op::DeleteTask { id: y }) => x == y,
        _ => false,
    }
}

fn bump(key: &str) -> String {
    match key.rsplit_once('-') {
        Some((p, n)) if n.parse::<u32>().is_ok() => format!("{p}-{}", n.parse::<u32>().unwrap() + 1),
        _ => format!("{key}-2"),
    }
}

/// `a` adjusted to apply after `b`; none when nothing is left of it.
fn after(a: &Op, b: &Op, later: bool) -> Vec<Op> {
    if same_target(a, b) {
        // The later setting wins; a second delete has nothing left to do.
        return if later && !matches!(a, Op::DeleteTask { .. }) { vec![a.clone()] } else { Vec::new() };
    }
    let gone = match b {
        Op::DeleteTask { id } => Some(id.as_str()),
        _ => None,
    };
    let mut a = a.clone();
    if let Some(g) = gone {
        match &mut a {
            Op::SetTask { id, .. } | Op::Place { id, .. } if id == g => return Vec::new(),
            Op::SetTask { field, value, .. } if field == "parent" && value.as_str() == Some(g) => *value = Json::Null,
            Op::Place { before, .. } | Op::AddTask { before, .. } if before.as_deref() == Some(g) => *before = None,
            Op::AddTask { task, .. } if task.parent.as_deref() == Some(g) => task.parent = None,
            _ => {}
        }
    }
    let mut extra = Vec::new();
    if let (Op::AddTask { task, before }, Op::AddTask { task: other, before: other_before }) = (&mut a, b) {
        let clash = !task.key.is_empty() && task.key == other.key;
        if later {
            if clash {
                task.key = bump(&task.key);
            }
        } else {
            // The earlier new task goes first, and keeps the key; the later one
            // (already in place here) takes the next number.
            if before == other_before {
                *before = Some(other.id.clone());
            }
            if clash {
                extra.push(Op::SetTask {
                    id: other.id.clone(),
                    field: "key".into(),
                    value: Json::String(bump(&other.key)),
                });
            }
        }
    }
    if let (Op::SetTask { field, value, .. }, Op::AddTask { task: other, .. }) = (&mut a, b)
        && later
        && field == "key"
        && value.as_str() == Some(other.key.as_str())
    {
        *value = Json::String(bump(&other.key));
    }
    let mut out = vec![a];
    out.extend(extra);
    out
}

impl Transform for Step {
    fn transform(a: &Step, b: &Step, later: bool) -> Vec<Step> {
        let mut ops: Vec<Op> = a.ops.clone();
        for bo in &b.ops {
            ops = ops.iter().flat_map(|ao| after(ao, bo, later)).collect();
        }
        if ops.is_empty() && !a.ops.is_empty() {
            return Vec::new();
        }
        vec![Step { ops, ..a.clone() }]
    }
}
