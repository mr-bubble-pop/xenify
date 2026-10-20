//! Several open files, one tab each.
//!
//! The document being edited lives in `XenifyApp`'s own fields (the editor,
//! selection, time, view …), so the rest of the app works on "the" document.
//! The other tabs are parked as [`Document`]s and swapped in when chosen;
//! [`XenifyApp::swap_document`] is the one place that lists those fields.

use crate::app::{XenifyApp, KeySel};
use xenify_doc::{Id, Project};
use xenify_ops::Editor;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub u64);

/// A parked (not currently shown) document.
pub struct Document {
    editor: Editor,
    comp: Id,
    file: Option<PathBuf>,
    selected: Option<Id>,
    extra_selected: Vec<Id>,
    selected_keys: Vec<KeySel>,
    time: f64,
    view: crate::viewport::ViewState,
    timeline: crate::timeline::TimelineState,
    graph: crate::graph::GraphState,
}

impl Document {
    fn new(mut project: Project, file: Option<PathBuf>) -> Self {
        xenify_ops::order::nulls_to_groups(&mut project);
        let comp = project.compositions[0].id;
        let mut editor = Editor::new(project);
        editor.mark_saved();
        Self {
            editor,
            comp,
            file,
            selected: None,
            extra_selected: Vec::new(),
            selected_keys: Vec::new(),
            time: 0.0,
            view: Default::default(),
            timeline: Default::default(),
            graph: Default::default(),
        }
    }
}

/// One tab: the shown document has `parked: None`.
pub struct Tab {
    pub id: DocId,
    parked: Option<Box<Document>>,
}

/// What the tab strip shows.
pub struct TabInfo {
    pub id: DocId,
    pub name: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub active: bool,
}

/// All tabs, in strip order.
pub struct Documents {
    pub tabs: Vec<Tab>,
    next: u64,
}

impl Default for Documents {
    fn default() -> Self {
        Self { tabs: vec![Tab { id: DocId(1), parked: None }], next: 2 }
    }
}

impl Documents {
    pub fn active(&self) -> DocId {
        self.tabs.iter().find(|t| t.parked.is_none()).expect("one tab is shown").id
    }

    pub fn ids(&self) -> Vec<DocId> {
        self.tabs.iter().map(|t| t.id).collect()
    }

    pub fn index(&self, id: DocId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
}

fn name_of(file: &Option<PathBuf>) -> String {
    file.as_ref().and_then(|p| p.file_stem()).map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
}

impl XenifyApp {
    /// Exchanges the shown document's state with `doc`.
    fn swap_document(&mut self, doc: &mut Document) {
        std::mem::swap(&mut self.editor, &mut doc.editor);
        std::mem::swap(&mut self.comp, &mut doc.comp);
        std::mem::swap(&mut self.file, &mut doc.file);
        std::mem::swap(&mut self.selected, &mut doc.selected);
        std::mem::swap(&mut self.extra_selected, &mut doc.extra_selected);
        std::mem::swap(&mut self.selected_keys, &mut doc.selected_keys);
        std::mem::swap(&mut self.time, &mut doc.time);
        std::mem::swap(&mut self.timeline, &mut doc.timeline);
        std::mem::swap(&mut self.graph, &mut doc.graph);
        let view = self.viewport.view_state();
        self.viewport.set_view_state(std::mem::replace(&mut doc.view, view));
    }

    pub fn tab_info(&self) -> Vec<TabInfo> {
        self.docs
            .tabs
            .iter()
            .map(|t| match &t.parked {
                None => TabInfo {
                    id: t.id,
                    name: name_of(&self.file),
                    path: self.file.clone(),
                    dirty: self.editor.is_dirty(),
                    active: true,
                },
                Some(d) => TabInfo {
                    id: t.id,
                    name: name_of(&d.file),
                    path: d.file.clone(),
                    dirty: d.editor.is_dirty(),
                    active: false,
                },
            })
            .collect()
    }

    /// Why switching tabs isn't possible right now, if it isn't.
    pub fn switch_blocked(&self) -> Option<&'static str> {
        self.assistant.busy().then_some("The assistant is working on this file — stop it or wait to switch tabs.")
    }

    /// Shows document `id`.
    pub fn activate(&mut self, id: DocId) {
        if self.docs.active() == id {
            return;
        }
        if let Some(why) = self.switch_blocked() {
            self.status = why.into();
            return;
        }
        let Some(to) = self.docs.index(id) else { return };
        // Settle what's in progress on the current document.
        self.settle_ai();
        self.playing = false;
        self.viewport.cancel_tool();
        self.paint_synced = None;
        let from = self.docs.index(self.docs.active()).expect("active tab exists");
        let mut doc = self.docs.tabs[to].parked.take().expect("inactive tab is parked");
        self.swap_document(&mut doc);
        self.docs.tabs[from].parked = Some(doc);
    }

    /// Next / previous tab (wrapping).
    pub fn cycle_tab(&mut self, forward: bool) {
        let n = self.docs.tabs.len();
        let i = self.docs.index(self.docs.active()).unwrap_or(0);
        let j = if forward { (i + 1) % n } else { (i + n - 1) % n };
        self.activate(self.docs.tabs[j].id);
    }

    /// The shown document is untouched and not from a file: opening
    /// something else can take its place, like a new browser tab.
    fn pristine(&self) -> bool {
        self.file.is_none() && !self.editor.is_dirty() && self.editor.undo_label().is_none()
    }

    /// Shows `project` in a tab of its own (switching to it if `file` is
    /// already open). Returns its tab.
    pub fn open_document(&mut self, project: Project, file: Option<PathBuf>) -> DocId {
        if let Some(f) = &file
            && let Some(t) = self.tab_info().iter().find(|t| t.path.as_ref() == Some(f))
        {
            let id = t.id;
            self.activate(id);
            return id;
        }
        if self.switch_blocked().is_some() || self.pristine() {
            self.load(project, file);
            return self.docs.active();
        }
        let id = DocId(self.docs.next);
        self.docs.next += 1;
        let at = self.docs.index(self.docs.active()).map_or(self.docs.tabs.len(), |i| i + 1);
        self.docs.tabs.insert(at, Tab { id, parked: Some(Box::new(Document::new(project, file))) });
        self.activate(id);
        self.viewport.request_fit();
        id
    }

    /// Closes tabs without asking (see `session::guard` for the prompt).
    /// Closing the last one leaves an empty Untitled document.
    pub fn close_documents(&mut self, ids: &[DocId]) {
        for id in ids {
            crate::session::clear_autosave(*id);
        }
        let active = self.docs.active();
        if ids.contains(&active) {
            // Show the nearest tab that stays open, or a fresh one.
            let i = self.docs.index(active).unwrap_or(0);
            let keep = self.docs.tabs[i..]
                .iter()
                .chain(self.docs.tabs[..i].iter().rev())
                .find(|t| !ids.contains(&t.id))
                .map(|t| t.id);
            match keep {
                Some(k) => self.activate(k),
                None => {
                    // Everything closes: start over with one empty document.
                    let next = self.docs.next;
                    self.docs = Documents { tabs: vec![Tab { id: DocId(next), parked: None }], next: next + 1 };
                    self.load(Project::new(), None);
                    return;
                }
            }
        }
        self.docs.tabs.retain(|t| !ids.contains(&t.id) || t.parked.is_none());
    }

    /// Unsaved documents among `ids`.
    pub fn dirty_documents(&self, ids: &[DocId]) -> Vec<DocId> {
        self.tab_info().into_iter().filter(|t| t.dirty && ids.contains(&t.id)).map(|t| t.id).collect()
    }

    /// Every open document's project with its id and file, for autosave.
    pub fn each_document(&self) -> Vec<(DocId, &Editor, &Option<PathBuf>)> {
        self.docs
            .tabs
            .iter()
            .map(|t| match &t.parked {
                None => (t.id, &self.editor, &self.file),
                Some(d) => (t.id, &d.editor, &d.file),
            })
            .collect()
    }

    /// Moves tab `id` to position `to` in the strip.
    pub fn move_tab(&mut self, id: DocId, to: usize) {
        if let Some(from) = self.docs.index(id) {
            let tab = self.docs.tabs.remove(from);
            let to = to.min(self.docs.tabs.len());
            self.docs.tabs.insert(to, tab);
        }
    }
}

/// Tabs to close for the tab context menu's "Close others" / "Close to the right".
pub fn others(tabs: &[DocId], keep: DocId) -> Vec<DocId> {
    tabs.iter().copied().filter(|t| *t != keep).collect()
}

pub fn to_the_right(tabs: &[DocId], of: DocId) -> Vec<DocId> {
    tabs.iter().position(|t| *t == of).map_or_else(Vec::new, |i| tabs[i + 1..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_menu_sets() {
        let tabs = [DocId(1), DocId(2), DocId(3), DocId(4)];
        assert_eq!(others(&tabs, DocId(2)), [DocId(1), DocId(3), DocId(4)]);
        assert_eq!(to_the_right(&tabs, DocId(2)), [DocId(3), DocId(4)]);
        assert!(to_the_right(&tabs, DocId(4)).is_empty());
        assert!(to_the_right(&tabs, DocId(9)).is_empty());
    }
}
