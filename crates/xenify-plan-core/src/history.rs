//! History: every step kept in the plan (who, when, what), the plan as it
//! was at any moment (replaying the steps), how milestones moved over time,
//! and going back (a new step that sets things as they were: nothing is
//! ever rewritten).

use crate::model::{Plan, Step};
use crate::ops;
use crate::schedule::{World, schedule};
use serde::Serialize;
use std::collections::BTreeMap;
use xenify_holidays::Date;

/// The plan just after the steps made up to `at` (Unix ms).
pub fn state_at(plan: &Plan, at: i64) -> Plan {
    let mut p = Plan::default();
    for s in plan.log.iter().take_while(|s| s.at <= at) {
        ops::apply_lenient(&mut p, &s.ops);
    }
    p
}

/// The plan after the first `n` steps.
pub fn state_after(plan: &Plan, n: usize) -> Plan {
    let mut p = Plan::default();
    for s in plan.log.iter().take(n) {
        ops::apply_lenient(&mut p, &s.ops);
    }
    p
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub index: usize,
    pub at: i64,
    pub who: String,
    pub what: String,
    /// Tasks it touched.
    pub tasks: Vec<String>,
    /// "dates", "people", "status", "links", "fields", "plan", "tasks".
    pub kinds: Vec<&'static str>,
}

fn kind_of(op: &ops::Op) -> &'static str {
    match op {
        ops::Op::SetTask { field, .. } => match field.as_str() {
            "start" | "work" | "started" | "finished" | "milestone" | "due" => "dates",
            "people" | "things" | "who" => "people",
            "comments" => "comments",
            "status" | "done" => "status",
            "after" | "relates" | "duplicates" => "links",
            "fields" => "fields",
            _ => "tasks",
        },
        ops::Op::AddTask { .. } | ops::Op::DeleteTask { .. } | ops::Op::Place { .. } => "tasks",
        _ => "plan",
    }
}

/// Newest first; `task` keeps the ones about that task.
pub fn entries(plan: &Plan, task: Option<&str>) -> Vec<Entry> {
    let mut out: Vec<Entry> = plan
        .log
        .iter()
        .enumerate()
        .map(|(index, s)| {
            let mut tasks: Vec<String> = s.ops.iter().filter_map(|o| o.task().map(String::from)).collect();
            tasks.dedup();
            let mut kinds: Vec<&'static str> = s.ops.iter().map(kind_of).collect();
            kinds.sort();
            kinds.dedup();
            Entry { index, at: s.at, who: s.who.clone(), what: s.what.clone(), tasks, kinds }
        })
        .filter(|e| task.is_none_or(|t| e.tasks.iter().any(|x| x == t)))
        .collect();
    out.reverse();
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct Slip {
    pub id: String,
    pub title: String,
    /// (when we looked, Unix ms; the planned day then).
    pub points: Vec<(i64, Date)>,
}

/// Each milestone's planned day after every day that had changes.
pub fn slips(plan: &Plan, world: &World) -> Vec<Slip> {
    let ms: Vec<(String, String)> = plan
        .order
        .iter()
        .filter_map(|id| plan.tasks.get(id))
        .filter(|t| plan.is_milestone(t))
        .map(|t| (t.id.clone(), t.title.clone()))
        .collect();
    let mut by: BTreeMap<String, Vec<(i64, Date)>> = BTreeMap::new();
    let mut p = Plan::default();
    let day = 86_400_000;
    let n = plan.log.len();
    for (k, s) in plan.log.iter().enumerate() {
        ops::apply_lenient(&mut p, &s.ops);
        let last_of_day = k + 1 == n || plan.log[k + 1].at / day != s.at / day;
        if !last_of_day {
            continue;
        }
        let sched = schedule(&p, world);
        for (id, _) in &ms {
            if let Some(d) = sched.tasks.get(id).filter(|d| d.placed) {
                let pts = by.entry(id.clone()).or_default();
                if pts.last().is_none_or(|(_, x)| *x != d.finish) {
                    pts.push((s.at, d.finish));
                }
            }
        }
    }
    let now = plan.log.last().map_or(0, |s| s.at);
    ms.into_iter()
        .map(|(id, title)| {
            let mut points = by.remove(&id).unwrap_or_default();
            if let Some(&(_, d)) = points.last() {
                points.push((now, d));
            }
            Slip { id, title, points }
        })
        .collect()
}

/// A step that makes the plan as it was before step `index` (Go back to before).
pub fn restore_ops(plan: &Plan, index: usize) -> Vec<ops::Op> {
    let old = state_after(plan, index);
    ops::diff(plan, &old)
}

/// The first step for a plan that has none (opened from an older file or
/// made some other way): everything it holds, as of `at`.
pub fn first_step(plan: &Plan, at: i64, who: &str, what: &str) -> Step {
    Step { at, who: who.into(), what: what.into(), ops: ops::diff(&Plan::default(), plan) }
}
