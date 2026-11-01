//! What someone may change in a plan, when the server limits it (a
//! document role, or "can comment"). The server checks every change the
//! same way (xenify-server live.rs); the app checks first so nothing is
//! done here that the server would refuse.

use crate::model::Comment;
use crate::ops::Op;

/// Each permission and what it lets you do.
pub const PERMS: &[(&str, &str)] = &[
    ("plan.comment", "Comment on issues"),
    ("plan.create", "Create issues"),
    ("plan.edit", "Edit issues: summary, description, fields, labels, priority, due date"),
    ("plan.status", "Move issues: status and rank (the board and the backlog)"),
    ("plan.assign", "Change who an issue is given to"),
    ("plan.schedule", "Change estimates, holds and “waits for” links"),
    ("plan.delete", "Delete issues"),
    ("plan.setup", "Change the plan’s set-up: columns, issue types, fields, its name and settings"),
];

/// Everything (a plain editor).
pub fn all() -> Vec<String> {
    PERMS.iter().map(|p| p.0.to_string()).collect()
}

/// Which permission a change needs.
pub fn need(op: &Op) -> &'static str {
    match op {
        Op::AddTask { .. } => "plan.create",
        Op::DeleteTask { .. } => "plan.delete",
        Op::Place { .. } => "plan.status",
        // Releasing is moving done issues on, like moving cards.
        Op::SetItem { list, .. } if list == "releases" => "plan.status",
        // Keeping the expected dates is about the schedule.
        Op::SetItem { list, .. } if list == "baselines" => "plan.schedule",
        Op::SetPlan { .. } | Op::SetItem { .. } => "plan.setup",
        Op::SetTask { field, .. } => match field.as_str() {
            "comments" => "plan.comment",
            "status" | "started" | "finished" | "done" | "release" => "plan.status",
            "who" => "plan.assign",
            "work" | "start" | "after" | "milestone" | "overbook" | "mirror" => "plan.schedule",
            _ => "plan.edit",
        },
    }
}

/// The first permission `ops` need that `perms` lack. Changes to an issue
/// made in the same step count as making it.
pub fn refused(ops: &[Op], perms: &[String]) -> Option<&'static str> {
    let made: Vec<&str> = ops
        .iter()
        .filter_map(|o| match o {
            Op::AddTask { task, .. } => Some(task.id.as_str()),
            _ => None,
        })
        .collect();
    for op in ops {
        let mut n = need(op);
        if op.task().is_some_and(|t| made.contains(&t)) {
            n = "plan.create";
        }
        if !perms.iter().any(|p| p == n) {
            return Some(n);
        }
    }
    None
}

/// "Your access to this plan doesn't allow this: …".
pub fn message(perm: &str) -> String {
    let what = PERMS.iter().find(|p| p.0 == perm).map(|p| p.1).unwrap_or(perm);
    format!("Your access to this plan doesn’t allow this: {}. Ask whoever manages it.", what.to_lowercase())
}

/// Said when someone changes another person's comment, writes one in
/// someone else's name, or deletes another's without being able to edit
/// the plan.
pub const NOT_YOURS: &str = "You can only change your own comments (deleting other people’s needs “Edit issues”).";

/// Whether changing an issue's comments from `before` to `after` keeps to
/// "only your own": new comments are by `me` (as the register knows them,
/// "u12"), and only `me`'s comments are changed. Deleting: your own, or
/// anyone's for someone who may edit the plan (`editor`), who may also put
/// back one deleted before (Undo) exactly as it was (`known`: every comment
/// the plan has had).
pub fn comments_ok(before: &[Comment], after: &[Comment], me: &str, editor: bool, known: &[Comment]) -> bool {
    let mine = |c: &Comment| !me.is_empty() && c.by == me;
    for c in after {
        match before.iter().find(|b| b.id == c.id) {
            None if !mine(c) && !(editor && known.contains(c)) => return false,
            Some(b) if b != c && !(mine(b) && mine(c)) => return false,
            _ => {}
        }
    }
    before.iter().all(|b| after.iter().any(|c| c.id == b.id) || mine(b) || editor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, by: &str, html: &str) -> Comment {
        Comment { id: id.into(), at: 1, who: "x".into(), by: by.into(), html: html.into() }
    }

    #[test]
    fn comments_are_only_their_authors() {
        let before = vec![c("a", "u1", "hi"), c("b", "u2", "yo")];
        let ok = |b: &[Comment], a: &[Comment], me: &str| comments_ok(b, a, me, false, &[]);
        assert!(ok(&before, &[before.clone(), vec![c("n", "u1", "new")]].concat(), "u1"));
        assert!(!ok(&before, &[before.clone(), vec![c("n", "u2", "fake")]].concat(), "u1"), "in another's name");
        assert!(!ok(&before, &[before.clone(), vec![c("n", "", "nobody")]].concat(), "u1"));
        assert!(ok(&before, &[c("a", "u1", "changed"), c("b", "u2", "yo")], "u1"));
        assert!(!ok(&before, &[c("a", "u1", "hi"), c("b", "u2", "changed")], "u1"));
        assert!(!ok(&before, &[c("a", "u1", "hi"), c("b", "u1", "yo")], "u1"), "taking it over");
        assert!(ok(&before, &[c("b", "u2", "yo")], "u1"), "deleting my own");
        assert!(!ok(&before, &[c("a", "u1", "hi")], "u1"), "deleting another's");
        assert!(!ok(&[], &[c("n", "u1", "x")], ""), "nobody isn't anybody");
    }

    #[test]
    fn editors_may_delete_anyones_comments_but_not_change_them() {
        let before = vec![c("a", "u1", "hi"), c("b", "u2", "yo")];
        assert!(comments_ok(&before, &[c("a", "u1", "hi")], "u1", true, &before), "deleting another's");
        assert!(!comments_ok(&before, &[c("a", "u1", "hi"), c("b", "u2", "changed")], "u1", true, &before));
        assert!(!comments_ok(&before, &[before.clone(), vec![c("n", "u2", "fake")]].concat(), "u1", true, &before));
        // Undo: back exactly as it was, only.
        let gone = vec![c("a", "u1", "hi")];
        assert!(comments_ok(&gone, &before, "u1", true, &before), "putting it back");
        assert!(!comments_ok(&gone, &[c("a", "u1", "hi"), c("b", "u2", "other words")], "u1", true, &before));
        assert!(!comments_ok(&gone, &before, "u1", false, &before), "only editors put others' back");
    }
}
