use crate::{Command, OpError, apply};
use xenify_doc::Project;

/// Maximum number of undo steps kept.
const HISTORY_LIMIT: usize = 500;

struct Entry {
    label: String,
    inverse: Command,
    /// Consecutive commands with the same key (e.g. one drag gesture) collapse
    /// into a single undo step.
    merge_key: Option<String>,
}

struct Group {
    label: String,
    inverses: Vec<Command>,
    depth: usize,
}

/// A project plus its undo/redo history. The only way to mutate the project.
pub struct Editor {
    project: Project,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    group: Option<Group>,
    revision: u64,
    saved_revision: u64,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Self { project, undo: Vec::new(), redo: Vec::new(), group: None, revision: 0, saved_revision: 0 }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Replaces the whole project (e.g. after opening a file) and clears history.
    pub fn replace_project(&mut self, project: Project) {
        *self = Self { revision: self.revision + 1, saved_revision: self.revision + 1, ..Self::new(project) };
    }

    /// Increments on every change, including undo/redo. Use it to invalidate caches.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    /// Flags the project as having unsaved changes (e.g. recovered work).
    pub fn mark_unsaved(&mut self) {
        self.saved_revision = u64::MAX;
    }

    /// Applies `cmd` as its own undo step (or as part of the open group).
    pub fn execute(&mut self, label: impl Into<String>, cmd: Command) -> Result<(), OpError> {
        self.execute_inner(label.into(), None, cmd)
    }

    /// Like [`Self::execute`], but merges into the previous undo step if it had
    /// the same `merge_key`. Use one key per continuous gesture, e.g.
    /// `"drag-7"` while dragging layer 7.
    pub fn execute_merged(
        &mut self,
        label: impl Into<String>,
        merge_key: impl Into<String>,
        cmd: Command,
    ) -> Result<(), OpError> {
        self.execute_inner(label.into(), Some(merge_key.into()), cmd)
    }

    fn execute_inner(&mut self, label: String, merge_key: Option<String>, cmd: Command) -> Result<(), OpError> {
        let inverse = apply(&mut self.project, cmd)?;
        self.revision += 1;
        self.redo.clear();

        if let Some(group) = &mut self.group {
            group.inverses.push(inverse);
            return Ok(());
        }
        if let (Some(key), Some(top)) = (&merge_key, self.undo.last_mut())
            && top.merge_key.as_ref() == Some(key)
        {
            top.inverse = merge_inverses(std::mem::replace(&mut top.inverse, Command::batch(vec![])), inverse);
            top.label = label;
            return Ok(());
        }
        self.push_undo(Entry { label, inverse, merge_key });
        Ok(())
    }

    fn push_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Starts collecting commands into one undo step. Groups nest; only the
    /// outermost label is kept.
    pub fn begin_group(&mut self, label: impl Into<String>) {
        match &mut self.group {
            Some(g) => g.depth += 1,
            None => self.group = Some(Group { label: label.into(), inverses: Vec::new(), depth: 1 }),
        }
    }

    /// Closes the innermost group; closing the outermost one records the undo step.
    pub fn end_group(&mut self) {
        let Some(g) = &mut self.group else { return };
        g.depth -= 1;
        if g.depth > 0 {
            return;
        }
        let g = self.group.take().unwrap();
        if g.inverses.is_empty() {
            return;
        }
        let mut inverses = g.inverses;
        inverses.reverse();
        self.push_undo(Entry { label: g.label, inverse: Command::batch(inverses), merge_key: None });
    }

    /// Reverts everything applied since the outermost [`Self::begin_group`] and
    /// discards the group (e.g. when the user rejects an AI change mid-way).
    pub fn cancel_group(&mut self) {
        let Some(g) = self.group.take() else { return };
        for inv in g.inverses.into_iter().rev() {
            apply(&mut self.project, inv).expect("inverse of an applied command must apply");
            self.revision += 1;
        }
    }

    pub fn in_group(&self) -> bool {
        self.group.is_some()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// Undoes the last step, returning its label. Closes any open group first.
    pub fn undo(&mut self) -> Option<String> {
        self.close_open_group();
        let entry = self.undo.pop()?;
        let redo = apply(&mut self.project, entry.inverse).expect("undo command must apply");
        self.revision += 1;
        self.redo.push(Entry { label: entry.label.clone(), inverse: redo, merge_key: None });
        Some(entry.label)
    }

    /// Redoes the last undone step, returning its label.
    pub fn redo(&mut self) -> Option<String> {
        self.close_open_group();
        let entry = self.redo.pop()?;
        let undo = apply(&mut self.project, entry.inverse).expect("redo command must apply");
        self.revision += 1;
        self.push_undo(Entry { label: entry.label.clone(), inverse: undo, merge_key: None });
        Some(entry.label)
    }

    fn close_open_group(&mut self) {
        if let Some(g) = &mut self.group {
            g.depth = 1;
            self.end_group();
        }
    }
}

/// Combines the undo command of an earlier step (`older`) with that of a newer
/// one so that applying the result undoes both.
fn merge_inverses(older: Command, newer: Command) -> Command {
    // If the newer undo only restores properties the older one already
    // snapshots, the older snapshot alone restores the pre-gesture state.
    if let (Some(o), Some(n)) = (snapshots(&older), snapshots(&newer))
        && n.iter().all(|k| o.contains(k))
    {
        return older;
    }
    match (&older, &newer) {
        // Both snapshot the same property: the older snapshot alone restores
        // the state from before the gesture.
        (Command::SetProperty { layer: a, path: pa, .. }, Command::SetProperty { layer: b, path: pb, .. })
            if a == b && pa == pb =>
        {
            older
        }
        (Command::SetLayerMeta { layer: a, .. }, Command::SetLayerMeta { layer: b, .. }) if a == b => older,
        (Command::SetText { layer: a, .. }, Command::SetText { layer: b, .. }) if a == b => older,
        (Command::SetShape { layer: a, index: i, .. }, Command::SetShape { layer: b, index: j, .. })
            if a == b && i == j =>
        {
            older
        }
        _ => Command::batch(vec![newer, older]),
    }
}

/// The (layer, property) pairs a command restores wholesale, if it consists
/// only of `SetProperty` snapshots.
fn snapshots(cmd: &Command) -> Option<Vec<(xenify_doc::Id, xenify_doc::PropPath)>> {
    match cmd {
        Command::SetProperty { layer, path, .. } => Some(vec![(*layer, *path)]),
        Command::Batch { commands } => {
            let mut out = Vec::new();
            for c in commands {
                out.extend(snapshots(c)?);
            }
            Some(out)
        }
        _ => None,
    }
}
