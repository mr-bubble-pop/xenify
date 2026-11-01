//! Every change to a plan is a list of these ops. Applying one gives back
//! the op that undoes it exactly; they are also what goes to the others when
//! editing together (collab.rs) and what History replays (history.rs).
//!
//! Ops set whole values (a task's title, its people, a status in the list):
//! two people setting the same thing at once is settled by "the later wins".

use crate::model::{Id, Plan, Task};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A new task, placed before `before` in the breakdown (at the end when none).
    AddTask {
        task: Box<Task>,
        before: Option<Id>,
    },
    DeleteTask {
        id: Id,
    },
    /// One of a task's fields (by its name in the file: "title", "people" …).
    SetTask {
        id: Id,
        field: String,
        value: Json,
    },
    /// Moves a task in the breakdown order.
    Place {
        id: Id,
        before: Option<Id>,
    },
    /// The plan's title, or a setting ("settings.keep_up_to_date").
    SetPlan {
        field: String,
        value: Json,
    },
    /// An item in one of the plan's lists (projects, types, statuses, fields,
    /// resources, baselines): set (added where `before` says) or removed.
    SetItem {
        list: String,
        id: Id,
        value: Option<Json>,
        before: Option<Id>,
    },
}

pub const LISTS: &[&str] = &["projects", "types", "statuses", "fields", "baselines", "releases"];

const TASK_FIELDS: &[&str] = &[
    "key",
    "title",
    "kind",
    "status",
    "projects",
    "parent",
    "description",
    "fields",
    "after",
    "relates",
    "duplicates",
    "start",
    "milestone",
    "work",
    "done",
    "started",
    "finished",
    "who",
    "priority",
    "due",
    "labels",
    "comments",
    "overbook",
    "release",
    "mirror",
];

impl Op {
    /// The task this op is about, if any.
    pub fn task(&self) -> Option<&str> {
        match self {
            Op::AddTask { task, .. } => Some(&task.id),
            Op::DeleteTask { id } | Op::SetTask { id, .. } | Op::Place { id, .. } => Some(id),
            _ => None,
        }
    }
}

fn next_in(order: &[Id], id: &str) -> Option<Id> {
    let i = order.iter().position(|x| x == id)?;
    order.get(i + 1).cloned()
}

fn place(order: &mut Vec<Id>, id: &str, before: Option<&str>) {
    order.retain(|x| x != id);
    let at = before.and_then(|b| order.iter().position(|x| x == b)).unwrap_or(order.len());
    order.insert(at, id.to_string());
}

/// Applies one op; returns the op that undoes it.
pub fn apply(plan: &mut Plan, op: &Op) -> Result<Op, String> {
    match op {
        Op::AddTask { task, before } => {
            if task.id.is_empty() || plan.tasks.contains_key(&task.id) {
                return Err(format!("task {} already there", task.id));
            }
            plan.tasks.insert(task.id.clone(), (**task).clone());
            place(&mut plan.order, &task.id, before.as_deref());
            Ok(Op::DeleteTask { id: task.id.clone() })
        }
        Op::DeleteTask { id } => {
            let task = plan.tasks.remove(id).ok_or_else(|| format!("no task {id}"))?;
            let before = next_in(&plan.order, id);
            plan.order.retain(|x| x != id);
            Ok(Op::AddTask { task: Box::new(task), before })
        }
        Op::SetTask { id, field, value } => {
            if !TASK_FIELDS.contains(&field.as_str()) {
                return Err(format!("tasks have no “{field}”"));
            }
            let task = plan.tasks.get_mut(id).ok_or_else(|| format!("no task {id}"))?;
            let mut v = serde_json::to_value(&*task).map_err(|e| e.to_string())?;
            let old = v.get(field).cloned().unwrap_or(Json::Null);
            v[field.as_str()] = value.clone();
            *task = serde_json::from_value(v).map_err(|e| format!("{field}: {e}"))?;
            Ok(Op::SetTask { id: id.clone(), field: field.clone(), value: old })
        }
        Op::Place { id, before } => {
            if !plan.tasks.contains_key(id) {
                return Err(format!("no task {id}"));
            }
            let old = next_in(&plan.order, id);
            place(&mut plan.order, id, before.as_deref());
            Ok(Op::Place { id: id.clone(), before: old })
        }
        Op::SetPlan { field, value } => {
            let mut v = serde_json::to_value(&*plan).map_err(|e| e.to_string())?;
            let (head, tail) = field.split_once('.').unwrap_or((field, ""));
            if !matches!(head, "title" | "settings") || (head == "settings") == tail.is_empty() {
                return Err(format!("plans have no “{field}”"));
            }
            let slot = if tail.is_empty() { &mut v[head] } else { &mut v[head][tail] };
            let old = slot.take();
            *slot = value.clone();
            let log = std::mem::take(&mut plan.log);
            v["log"] = json!([]);
            let mut next: Plan = serde_json::from_value(v).map_err(|e| format!("{field}: {e}"))?;
            next.log = log;
            *plan = next;
            Ok(Op::SetPlan { field: field.clone(), value: old })
        }
        Op::SetItem { list, id, value, before } => {
            if !LISTS.contains(&list.as_str()) {
                return Err(format!("plans have no list “{list}”"));
            }
            let mut items = list_json(plan, list);
            let pos = items.iter().position(|x| x["id"] == json!(id));
            let old = pos.map(|i| items[i].clone());
            let old_next = pos.and_then(|i| items.get(i + 1)).and_then(|x| x["id"].as_str()).map(String::from);
            match (value, pos) {
                (None, Some(i)) => {
                    items.remove(i);
                }
                (None, None) => {}
                (Some(v), Some(i)) if before.is_none() || before == &old_next => {
                    let mut v = v.clone();
                    v["id"] = json!(id);
                    items[i] = v;
                }
                (Some(v), _) => {
                    if let Some(i) = pos {
                        items.remove(i);
                    }
                    let mut v = v.clone();
                    v["id"] = json!(id);
                    let at = before
                        .as_ref()
                        .and_then(|b| items.iter().position(|x| x["id"] == json!(b)))
                        .unwrap_or(items.len());
                    items.insert(at, v);
                }
            }
            set_list(plan, list, items)?;
            Ok(Op::SetItem { list: list.clone(), id: id.clone(), value: old, before: old_next })
        }
    }
}

fn list_json(plan: &Plan, list: &str) -> Vec<Json> {
    let v = match list {
        "projects" => serde_json::to_value(&plan.projects),
        "types" => serde_json::to_value(&plan.types),
        "statuses" => serde_json::to_value(&plan.statuses),
        "fields" => serde_json::to_value(&plan.fields),
        "releases" => serde_json::to_value(&plan.releases),
        _ => serde_json::to_value(&plan.baselines),
    };
    match v {
        Ok(Json::Array(a)) => a,
        _ => Vec::new(),
    }
}

fn set_list(plan: &mut Plan, list: &str, items: Vec<Json>) -> Result<(), String> {
    let v = Json::Array(items);
    let e = |e: serde_json::Error| format!("{list}: {e}");
    match list {
        "projects" => plan.projects = serde_json::from_value(v).map_err(e)?,
        "types" => plan.types = serde_json::from_value(v).map_err(e)?,
        "statuses" => plan.statuses = serde_json::from_value(v).map_err(e)?,
        "fields" => plan.fields = serde_json::from_value(v).map_err(e)?,
        "releases" => plan.releases = serde_json::from_value(v).map_err(e)?,
        _ => plan.baselines = serde_json::from_value(v).map_err(e)?,
    }
    Ok(())
}

/// Applies ops in order; on a failure, undoes what went in and fails.
/// Returns the undo ops (to apply in the order given).
pub fn apply_all(plan: &mut Plan, ops: &[Op]) -> Result<Vec<Op>, String> {
    let mut undo = Vec::new();
    for op in ops {
        match apply(plan, op) {
            Ok(u) => undo.push(u),
            Err(e) => {
                for u in undo.iter().rev() {
                    let _ = apply(plan, u);
                }
                return Err(e);
            }
        }
    }
    undo.reverse();
    Ok(undo)
}

/// Applies what can be applied (someone else's change that no longer fits
/// partly is kept as far as it goes).
pub fn apply_lenient(plan: &mut Plan, ops: &[Op]) {
    for op in ops {
        let _ = apply(plan, op);
    }
}

/// The ops that turn `from` into `to` (Restore in History).
pub fn diff(from: &Plan, to: &Plan) -> Vec<Op> {
    let mut ops = Vec::new();
    if from.title != to.title {
        ops.push(Op::SetPlan { field: "title".into(), value: json!(to.title) });
    }
    if from.settings != to.settings {
        let a = serde_json::to_value(&from.settings).unwrap_or_default();
        let b = serde_json::to_value(&to.settings).unwrap_or_default();
        if let Some(bo) = b.as_object() {
            for (k, v) in bo {
                if a.get(k) != Some(v) {
                    ops.push(Op::SetPlan { field: format!("settings.{k}"), value: v.clone() });
                }
            }
        }
    }
    for list in LISTS {
        let a = list_json(from, list);
        let b = list_json(to, list);
        let ids = |v: &[Json]| v.iter().filter_map(|x| x["id"].as_str().map(String::from)).collect::<Vec<_>>();
        let (ia, ib) = (ids(&a), ids(&b));
        for id in ia.iter().filter(|i| !ib.contains(i)) {
            ops.push(Op::SetItem { list: list.to_string(), id: id.clone(), value: None, before: None });
        }
        // Last first, so each item's neighbour after it is already in place.
        for (k, x) in b.iter().enumerate().rev() {
            let id = x["id"].as_str().unwrap_or_default().to_string();
            let same = a.iter().position(|y| y["id"] == x["id"]);
            let before = ib.get(k + 1).cloned();
            if same.is_none_or(|i| &a[i] != x || ia.get(i + 1) != before.as_ref()) {
                ops.push(Op::SetItem { list: list.to_string(), id, value: Some(x.clone()), before });
            }
        }
    }
    for id in from.order.iter().filter(|i| !to.tasks.contains_key(*i)) {
        ops.push(Op::DeleteTask { id: id.clone() });
    }
    for (k, id) in to.order.iter().enumerate() {
        let Some(t) = to.tasks.get(id) else { continue };
        let before = to.order.get(k + 1).cloned();
        match from.tasks.get(id) {
            None => ops.push(Op::AddTask { task: Box::new(t.clone()), before }),
            Some(old) => {
                let (a, b) =
                    (serde_json::to_value(old).unwrap_or_default(), serde_json::to_value(t).unwrap_or_default());
                for f in TASK_FIELDS {
                    if a.get(*f) != b.get(*f) {
                        ops.push(Op::SetTask {
                            id: id.clone(),
                            field: f.to_string(),
                            value: b.get(*f).cloned().unwrap_or(Json::Null),
                        });
                    }
                }
            }
        }
    }
    // Order last: everything exists by then.
    let mut order = from.order.iter().filter(|i| to.tasks.contains_key(*i)).cloned().collect::<Vec<_>>();
    for id in &to.order {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    for (k, id) in to.order.iter().enumerate().rev() {
        let before = to.order.get(k + 1).cloned();
        let now = order.iter().position(|x| x == id).and_then(|i| order.get(i + 1)).cloned();
        if now != before {
            place(&mut order, id, before.as_deref());
            ops.push(Op::Place { id: id.clone(), before });
        }
    }
    ops
}
