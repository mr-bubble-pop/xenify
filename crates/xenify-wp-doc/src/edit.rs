use crate::{
    Block, Container, Document, ListItem, NodeId, PageSetup, ParaProps, ParaStyle, Paragraph, Section, SectionStart,
    Selection, Span,
};
use serde::{Deserialize, Serialize};

/// One invertible change. [`Step::apply`] returns the step that undoes it.
/// A document's whole content (for [`Step::SetContent`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Content {
    pub sections: Vec<crate::Section>,
    pub styles: crate::StyleSheet,
    pub lists: Vec<crate::ListDef>,
    pub meta: crate::Meta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    /// Replaces `remove` bytes at `offset` with `insert` (typing, deleting,
    /// and formatting: the same text with different runs).
    Replace { para: NodeId, offset: usize, remove: usize, insert: Span },
    /// Splits a paragraph at `offset`; the tail becomes paragraph `new_id`
    /// with style `style`, direct properties `props` and list membership `list`.
    Split {
        para: NodeId,
        offset: usize,
        new_id: NodeId,
        style: String,
        #[serde(default)]
        props: ParaProps,
        #[serde(default)]
        list: Option<ListItem>,
        /// Where the new paragraph goes (container, block index); by default
        /// right after `para`. Undoing a join across sections uses this.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<(Container, usize)>,
    },
    /// Appends paragraph `next` to `para` and removes `next`.
    Join { para: NodeId, next: NodeId },
    /// Changes a paragraph's style.
    SetStyle { para: NodeId, style: String },
    /// Replaces a paragraph's direct properties.
    SetProps { para: NodeId, props: ParaProps },
    /// Puts a paragraph in a list (or takes it out with `None`).
    SetList { para: NodeId, list: Option<ListItem> },
    /// Adds, replaces (`def` Some) or deletes (`def` None) paragraph style `id`.
    SetStyleDef { id: String, def: Option<ParaStyle> },
    /// Starts a new section (id `id`) at block `at` of section `section`:
    /// that block and the ones after it move into it.
    SplitSection {
        section: usize,
        at: usize,
        id: NodeId,
        start: SectionStart,
        page: PageSetup,
        /// Frames (by id) that move to the new section.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        frames: Vec<NodeId>,
    },
    /// Moves section `section + 1`'s blocks to the end of `section` and removes it.
    JoinSection { section: usize },
    /// Replaces a section's frames (add, move, resize, restyle, delete).
    SetFrames { section: usize, frames: Vec<crate::Frame> },
    /// Changes a section's start and page setup.
    SetSection { section: usize, start: SectionStart, page: PageSetup },
    /// Inserts a block at `index` of `container`.
    InsertBlock { container: Container, index: usize, block: Block },
    /// Removes the block at `index` of `container`.
    RemoveBlock { container: Container, index: usize },
    /// Replaces the block at `index` of `container` (table structure edits).
    ReplaceBlock { container: Container, index: usize, block: Block },
    /// Replaces the document's details (title, author…).
    SetMeta { meta: Box<crate::Meta> },
    /// Adds, replaces (`def` Some) or deletes (`def` None) list definition `id`.
    SetListDef { id: String, def: Option<crate::ListDef> },
    /// Replaces a section's footnotes and endnotes.
    SetNotes { section: usize, notes: Vec<crate::Note> },
    /// A section's rich headers and footers.
    SetHeaders { section: usize, headers: Vec<crate::HeaderPart> },
    /// Everything in the document at once (restoring an earlier version).
    SetContent { content: Box<Content> },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("no paragraph {0:?}")]
    NoParagraph(NodeId),
    #[error("offset {0} is outside the paragraph or not on a character boundary")]
    BadOffset(usize),
    #[error("no block at {1} of {0:?}")]
    NoBlock(Container, usize),
    #[error("that can't be done here")]
    NotHere,
}

impl Step {
    pub fn apply(self, doc: &mut Document) -> Result<Step, EditError> {
        doc.revision += 1;
        let rev = doc.revision;
        fn para(doc: &mut Document, id: NodeId) -> Result<&mut Paragraph, EditError> {
            doc.para_mut(id).ok_or(EditError::NoParagraph(id))
        }
        let check = |p: &Paragraph, o: usize| {
            if o <= p.text.len() && p.text.is_char_boundary(o) { Ok(()) } else { Err(EditError::BadOffset(o)) }
        };
        Ok(match self {
            Step::Replace { para: id, offset, remove, insert } => {
                let p = para(doc, id)?;
                check(p, offset)?;
                check(p, offset + remove)?;
                let len = insert.len();
                let removed = p.splice(offset..offset + remove, insert);
                p.rev = rev;
                Step::Replace { para: id, offset, remove: len, insert: removed }
            }
            Step::Split { para: id, offset, new_id, style, props, list, place } => {
                let (si, bi) = doc.locate(id).ok_or(EditError::NoParagraph(id))?;
                let p = para(doc, id)?;
                check(p, offset)?;
                let tail = p.splice(offset..p.text.len(), Span::default());
                p.rev = rev;
                let mut new = Paragraph { id: new_id, style, text: tail.text, runs: tail.runs, props, list, rev };
                new.normalize();
                let (c, bi) = place.unwrap_or((si, bi + 1));
                let blocks = doc.blocks_mut(&c).ok_or(EditError::NoBlock(c.clone(), bi))?;
                if bi > blocks.len() {
                    return Err(EditError::NoBlock(c, bi));
                }
                blocks.insert(bi, Block::Paragraph(new));
                Step::Join { para: id, next: new_id }
            }
            Step::Join { para: id, next } => {
                let (si, bi) = doc.locate(next).ok_or(EditError::NoParagraph(next))?;
                let blocks = doc.blocks_mut(&si).ok_or(EditError::NoParagraph(next))?;
                if !matches!(blocks.get(bi), Some(Block::Paragraph(_))) {
                    return Err(EditError::NoParagraph(next));
                }
                let Block::Paragraph(removed) = blocks.remove(bi) else { unreachable!() };
                let p = para(doc, id)?;
                let offset = p.text.len();
                p.splice(offset..offset, Span { text: removed.text, runs: removed.runs });
                p.rev = rev;
                Step::Split {
                    para: id,
                    offset,
                    new_id: next,
                    style: removed.style,
                    props: removed.props,
                    list: removed.list,
                    place: Some((si, bi)),
                }
            }
            Step::SetStyle { para: id, style } => {
                let p = para(doc, id)?;
                p.rev = rev;
                let old = std::mem::replace(&mut p.style, style);
                Step::SetStyle { para: id, style: old }
            }
            Step::SetProps { para: id, props } => {
                let p = para(doc, id)?;
                p.rev = rev;
                let old = std::mem::replace(&mut p.props, props);
                Step::SetProps { para: id, props: old }
            }
            Step::SetList { para: id, list } => {
                let p = para(doc, id)?;
                p.rev = rev;
                let old = std::mem::replace(&mut p.list, list);
                Step::SetList { para: id, list: old }
            }
            Step::SetStyleDef { id, def } => {
                // Every paragraph's resolved look may change: invalidate them all.
                let styles = &mut doc.styles.paragraph;
                let at = styles.iter().position(|s| s.id == id);
                let old = match (at, def) {
                    (Some(i), Some(d)) => Some(std::mem::replace(&mut styles[i], d)),
                    (Some(i), None) => Some(styles.remove(i)),
                    (None, Some(d)) => {
                        styles.push(d);
                        None
                    }
                    (None, None) => None,
                };
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetStyleDef { id, def: old }
            }
            Step::SplitSection { section, at, id, start, page, frames } => {
                let s = doc.sections.get_mut(section).ok_or(EditError::NoBlock(Container::Section(section), at))?;
                if at > s.blocks.len() {
                    return Err(EditError::NoBlock(Container::Section(section), at));
                }
                let blocks = s.blocks.split_off(at);
                let (moved, kept): (Vec<_>, Vec<_>) =
                    std::mem::take(&mut s.frames).into_iter().partition(|f| frames.contains(&f.id));
                s.frames = kept;
                doc.sections.insert(
                    section + 1,
                    Section { id, start, page, blocks, frames: moved, notes: Vec::new(), headers: Vec::new() },
                );
                Step::JoinSection { section }
            }
            Step::JoinSection { section } => {
                if section + 1 >= doc.sections.len() {
                    return Err(EditError::NoBlock(Container::Section(section + 1), 0));
                }
                let next = doc.sections.remove(section + 1);
                let at = doc.sections[section].blocks.len();
                doc.sections[section].blocks.extend(next.blocks);
                let frames = next.frames.iter().map(|f| f.id).collect();
                doc.sections[section].frames.extend(next.frames);
                // Notes go along (where a note is kept doesn't matter: its reference places it).
                doc.sections[section].notes.extend(next.notes);
                Step::SplitSection { section, at, id: next.id, start: next.start, page: next.page, frames }
            }
            Step::SetFrames { section, frames } => {
                let s = doc.sections.get_mut(section).ok_or(EditError::NoBlock(Container::Section(section), 0))?;
                let old = std::mem::replace(&mut s.frames, frames);
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetFrames { section, frames: old }
            }
            Step::SetSection { section, start, page } => {
                let s = doc.sections.get_mut(section).ok_or(EditError::NoBlock(Container::Section(section), 0))?;
                let old_start = std::mem::replace(&mut s.start, start);
                let old_page = std::mem::replace(&mut s.page, page);
                Step::SetSection { section, start: old_start, page: old_page }
            }
            Step::InsertBlock { container, index, block } => {
                let blocks = doc.blocks_mut(&container).ok_or(EditError::NoBlock(container.clone(), index))?;
                if index > blocks.len() {
                    return Err(EditError::NoBlock(container, index));
                }
                blocks.insert(index, block);
                Step::RemoveBlock { container, index }
            }
            Step::RemoveBlock { container, index } => {
                let blocks = doc.blocks_mut(&container).ok_or(EditError::NoBlock(container.clone(), index))?;
                if index >= blocks.len() {
                    return Err(EditError::NoBlock(container, index));
                }
                let block = blocks.remove(index);
                Step::InsertBlock { container, index, block }
            }
            Step::ReplaceBlock { container, index, block } => {
                let blocks = doc.blocks_mut(&container).ok_or(EditError::NoBlock(container.clone(), index))?;
                let slot = blocks.get_mut(index).ok_or(EditError::NoBlock(container.clone(), index))?;
                let old = std::mem::replace(slot, block);
                // Paragraphs inside may have changed places: redo their layout.
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::ReplaceBlock { container, index, block: old }
            }
            Step::SetNotes { section, notes } => {
                let s = doc.sections.get_mut(section).ok_or(EditError::NoBlock(Container::Section(section), 0))?;
                let old = std::mem::replace(&mut s.notes, notes);
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetNotes { section, notes: old }
            }
            Step::SetContent { content } => {
                let Content { sections, styles, lists, meta } = *content;
                let old = Content {
                    sections: std::mem::replace(&mut doc.sections, sections),
                    styles: std::mem::replace(&mut doc.styles, styles),
                    lists: std::mem::replace(&mut doc.lists, lists),
                    meta: std::mem::replace(&mut doc.meta, meta),
                };
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetContent { content: Box::new(old) }
            }
            Step::SetHeaders { section, headers } => {
                let s = doc.sections.get_mut(section).ok_or(EditError::NoBlock(Container::Section(section), 0))?;
                let old = std::mem::replace(&mut s.headers, headers);
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetHeaders { section, headers: old }
            }
            Step::SetListDef { id, def } => {
                let lists = &mut doc.lists;
                let at = lists.iter().position(|d| d.id == id);
                let old = match (at, def) {
                    (Some(i), Some(d)) => Some(std::mem::replace(&mut lists[i], d)),
                    (Some(i), None) => Some(lists.remove(i)),
                    (None, Some(d)) => {
                        lists.push(d);
                        None
                    }
                    (None, None) => None,
                };
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetListDef { id, def: old }
            }
            Step::SetMeta { meta } => {
                let old = std::mem::replace(&mut doc.meta, *meta);
                // Headers and footers may show the title or author.
                doc.for_each_para_mut(&mut |p| p.rev = rev);
                Step::SetMeta { meta: Box::new(old) }
            }
        })
    }
}

/// What lets consecutive transactions merge into one undo step (typing a
/// word undoes as a whole).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeKey {
    Typing(NodeId),
    Deleting(NodeId),
    /// Successive changes to one style in the style editor.
    Style(String),
    /// Successive changes from one control (a colour picker drag).
    Control(String),
}

/// A transaction in progress: steps are applied as they are added, and
/// their inverses recorded.
pub struct Tx<'a> {
    pub doc: &'a mut Document,
    inverse: Vec<Step>,
}

impl<'a> Tx<'a> {
    pub fn step(&mut self, step: Step) -> Result<(), EditError> {
        let inv = step.apply(self.doc)?;
        self.inverse.push(inv);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.inverse.is_empty()
    }

    fn rollback(&mut self) {
        while let Some(s) = self.inverse.pop() {
            let _ = s.apply(self.doc);
        }
    }
}

struct Entry {
    label: String,
    /// Steps that undo the transaction, in the order to apply them.
    undo: Vec<Step>,
    before: Selection,
    after: Selection,
    merge: Option<MergeKey>,
}

/// A document with a selection and undo history.
pub struct Editor {
    pub doc: Document,
    pub sel: Selection,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Stops the next transaction merging into the last one.
    sealed: bool,
    saved_revision: u64,
}

impl Editor {
    pub fn new(mut doc: Document) -> Self {
        doc.repair();
        let sel = Selection::caret(doc.start());
        let saved_revision = doc.revision;
        Self { doc, sel, undo: Vec::new(), redo: Vec::new(), sealed: false, saved_revision }
    }

    /// Runs `f` as one undoable transaction. `f` returns the selection after
    /// it; on error every step is rolled back.
    pub fn transact(
        &mut self,
        label: &str,
        merge: Option<MergeKey>,
        f: impl FnOnce(&mut Tx) -> Result<Selection, EditError>,
    ) -> Result<(), EditError> {
        let before = self.sel;
        let mut tx = Tx { doc: &mut self.doc, inverse: Vec::new() };
        let after = match f(&mut tx) {
            Ok(sel) => sel,
            Err(e) => {
                tx.rollback();
                return Err(e);
            }
        };
        let mut steps = tx.inverse;
        self.sel = Selection { anchor: self.doc.clamp(after.anchor), focus: self.doc.clamp(after.focus) };
        if steps.is_empty() {
            return Ok(());
        }
        steps.reverse();
        self.redo.clear();
        if let (Some(last), Some(key), false) = (self.undo.last_mut(), &merge, self.sealed)
            && last.merge.as_ref() == Some(key)
        {
            // The newest changes are undone first.
            steps.append(&mut last.undo);
            last.undo = steps;
            last.after = self.sel;
            return Ok(());
        }
        self.sealed = false;
        self.undo.push(Entry { label: label.into(), undo: steps, before, after: self.sel, merge });
        Ok(())
    }

    /// Runs `f` (any number of transactions) as one undo step labelled
    /// `label`. With `merge`, successive groups with the same key undo
    /// together (typing at several carets undoes word by word).
    pub fn group<R>(&mut self, label: &str, merge: Option<MergeKey>, f: impl FnOnce(&mut Editor) -> R) -> R {
        let start = self.undo.len();
        let was_sealed = self.sealed;
        // Nothing inside merges into what came before.
        self.sealed = true;
        let out = f(self);
        let mut added: Vec<Entry> = self.undo.drain(start.min(self.undo.len())..).collect();
        if added.is_empty() {
            self.sealed = was_sealed;
            return out;
        }
        let before = added[0].before;
        let after = added.last().map_or(self.sel, |e| e.after);
        // Undo the newest first.
        let mut steps = Vec::new();
        while let Some(mut e) = added.pop() {
            steps.append(&mut e.undo);
        }
        if let (Some(last), Some(key), false) = (self.undo.last_mut(), &merge, was_sealed)
            && last.merge.as_ref() == Some(key)
        {
            steps.append(&mut last.undo);
            last.undo = steps;
            last.after = after;
        } else {
            self.undo.push(Entry { label: label.into(), undo: steps, before, after, merge });
        }
        self.sealed = false;
        out
    }

    /// Ends the current merge run (after a pause, a click, a caret move…).
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else { return false };
        let redo = Self::run(&mut self.doc, e.undo);
        self.sel = e.before;
        self.redo.push(Entry { undo: redo, merge: None, ..e });
        self.sealed = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else { return false };
        let undo = Self::run(&mut self.doc, e.undo);
        self.sel = e.after;
        self.undo.push(Entry { undo, merge: None, ..e });
        self.sealed = true;
        true
    }

    fn run(doc: &mut Document, steps: Vec<Step>) -> Vec<Step> {
        let mut inv: Vec<Step> = steps.into_iter().filter_map(|s| s.apply(doc).ok()).collect();
        inv.reverse();
        inv
    }

    /// Whether there are changes since the last [`Editor::mark_saved`].
    pub fn is_dirty(&self) -> bool {
        self.doc.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.doc.revision;
    }
}

/// Replaces the whole document with `other` (an earlier version) as one
/// undoable step; its pictures come along.
pub fn restore(ed: &mut Editor, other: crate::Document) -> Result<(), EditError> {
    for (k, v) in &other.assets {
        ed.doc.assets.entry(k.clone()).or_insert_with(|| v.clone());
    }
    ed.doc.next_id = ed.doc.next_id.max(other.next_id);
    let content = Content { sections: other.sections, styles: other.styles, lists: other.lists, meta: other.meta };
    ed.seal();
    ed.transact("Restore version", None, |tx| {
        tx.step(Step::SetContent { content: Box::new(content) })?;
        let first = tx.doc.paragraphs().next().map(|p| p.id).unwrap_or(NodeId(0));
        Ok(Selection::caret(crate::Pos::new(first, 0)))
    })?;
    ed.seal();
    Ok(())
}
