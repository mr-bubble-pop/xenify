//! Frame editing: insert, move and resize, properties, delete.

use crate::{Block, EditError, Editor, Frame, NodeId, Paragraph, Pos, Selection, Step, StyleSheet};

/// The section and index of frame `id`.
pub fn find(ed: &Editor, id: NodeId) -> Option<(usize, usize)> {
    ed.doc.sections.iter().enumerate().find_map(|(si, s)| s.frames.iter().position(|f| f.id == id).map(|fi| (si, fi)))
}

/// The frame holding paragraph `para`, if any.
pub fn frame_of(ed: &Editor, para: NodeId) -> Option<NodeId> {
    let mut c = ed.doc.locate(para)?.0;
    loop {
        match c {
            crate::Container::Frame(f) => return Some(f),
            crate::Container::Cell { table, .. } => c = ed.doc.locate(table)?.0,
            crate::Container::Section(_) | crate::Container::Note(_) | crate::Container::Header(_) => return None,
        }
    }
}

/// Adds `frame` (its id and content are filled in when empty) to section
/// `section`; the caret goes into it.
pub fn insert_frame(ed: &mut Editor, section: usize, mut frame: Frame) -> Result<NodeId, EditError> {
    frame.id = ed.doc.alloc_id();
    if frame.blocks.is_empty() {
        let p = ed.doc.alloc_id();
        frame.blocks.push(Block::Paragraph(Paragraph::new(p, StyleSheet::NORMAL, "")));
    }
    let first = frame.blocks.iter().find_map(|b| if let Block::Paragraph(p) = b { Some(p.id) } else { None });
    let id = frame.id;
    let mut frames = ed.doc.sections.get(section).ok_or(EditError::NotHere)?.frames.clone();
    frames.push(frame);
    let sel = first.map_or(ed.sel, |p| Selection::caret(Pos::new(p, 0)));
    ed.seal();
    ed.transact("Insert text box", None, |tx| {
        tx.step(Step::SetFrames { section, frames })?;
        Ok(sel)
    })?;
    Ok(id)
}

/// Changes frame `id` with `f` (moving, resizing, properties). Successive
/// changes from one drag merge into one undo step.
pub fn update_frame(ed: &mut Editor, id: NodeId, label: &str, f: impl FnOnce(&mut Frame)) -> Result<(), EditError> {
    let (si, fi) = find(ed, id).ok_or(EditError::NotHere)?;
    let mut frames = ed.doc.sections[si].frames.clone();
    let before = frames[fi].clone();
    f(&mut frames[fi]);
    let fr = &mut frames[fi];
    fr.width = fr.width.max(8.0);
    fr.height = fr.height.max(8.0);
    if frames[fi] == before {
        return Ok(());
    }
    let sel = ed.sel;
    ed.transact(label, Some(crate::MergeKey::Control(format!("frame-{}-{label}", id.0))), |tx| {
        tx.step(Step::SetFrames { section: si, frames })?;
        Ok(sel)
    })
}

/// Removes frame `id`; the caret goes back to the body if it was inside.
pub fn delete_frame(ed: &mut Editor, id: NodeId) -> Result<(), EditError> {
    let (si, fi) = find(ed, id).ok_or(EditError::NotHere)?;
    let mut frames = ed.doc.sections[si].frames.clone();
    frames.remove(fi);
    let inside = frame_of(ed, ed.sel.focus.para) == Some(id) || frame_of(ed, ed.sel.anchor.para) == Some(id);
    let sel = if inside { Selection::caret(ed.doc.start()) } else { ed.sel };
    ed.seal();
    ed.transact("Delete text box", None, |tx| {
        // Threads that ran into this frame now end before it.
        for f in &mut frames {
            if f.next == Some(id) {
                f.next = None;
            }
        }
        tx.step(Step::SetFrames { section: si, frames })?;
        Ok(sel)
    })
}

/// Floats picture block `image`: it moves into a frame its own size at
/// (`x`, `y`) on page `page` of section `section` (from the section's first
/// page), and text flows around it as `wrap` says. One undo step.
pub fn float_image(
    ed: &mut Editor,
    image: NodeId,
    section: usize,
    page: u32,
    (x, y): (f32, f32),
    wrap: crate::Wrap,
) -> Result<NodeId, EditError> {
    let (container, index) = ed.doc.locate(image).ok_or(EditError::NotHere)?;
    let Some(Block::Image(img)) = ed.doc.blocks(&container).and_then(|b| b.get(index)).cloned() else {
        return Err(EditError::NotHere);
    };
    // Its caption goes with it.
    let caption = match ed.doc.blocks(&container).and_then(|b| b.get(index + 1)) {
        Some(Block::Paragraph(p)) if p.style == StyleSheet::CAPTION => Some(p.clone()),
        _ => None,
    };
    let id = ed.doc.alloc_id();
    let (w, mut h) = if img.width > 0.0 { (img.width, img.height) } else { (200.0, 150.0) };
    let mut blocks = vec![Block::Image(img)];
    if let Some(c) = &caption {
        h += 24.0;
        blocks.push(Block::Paragraph(c.clone()));
    }
    let frame =
        Frame { id, page, x, y, width: w, height: h, wrap, border: None, padding: 0.0, blocks, ..Default::default() };
    let mut frames = ed.doc.sections.get(section).ok_or(EditError::NotHere)?.frames.clone();
    frames.push(frame);
    let sel = ed.sel;
    ed.seal();
    ed.transact("Wrap text around picture", None, |tx| {
        if caption.is_some() {
            tx.step(Step::RemoveBlock { container: container.clone(), index: index + 1 })?;
        }
        tx.step(Step::RemoveBlock { container: container.clone(), index })?;
        // A container never ends up empty.
        if tx.doc.blocks(&container).is_some_and(|b| b.is_empty()) {
            let p = tx.doc.alloc_id();
            let block = Block::Paragraph(Paragraph::new(p, StyleSheet::NORMAL, ""));
            tx.step(Step::InsertBlock { container: container.clone(), index: 0, block })?;
        }
        tx.step(Step::SetFrames { section, frames })?;
        let caret = if tx.doc.para(sel.focus.para).is_some() { sel } else { Selection::caret(tx.doc.start()) };
        Ok(caret)
    })?;
    Ok(id)
}

/// The picture in frame `frame` goes back into the text (as its own line,
/// before block `before` of the frame's section, or at the end), and the
/// frame goes. One undo step.
pub fn inline_image(ed: &mut Editor, frame: NodeId, before: Option<NodeId>) -> Result<(), EditError> {
    let (si, fi) = find(ed, frame).ok_or(EditError::NotHere)?;
    let mut frames = ed.doc.sections[si].frames.clone();
    let f = frames.remove(fi);
    let Some(Block::Image(img)) = f.blocks.iter().find(|b| matches!(b, Block::Image(_))).cloned() else {
        return Err(EditError::NotHere);
    };
    // Its caption comes back with it.
    let caption = f.blocks.into_iter().find(|b| matches!(b, Block::Paragraph(p) if p.style == StyleSheet::CAPTION));
    let container = crate::Container::Section(si);
    let index = before
        .and_then(|b| ed.doc.locate(b))
        .filter(|(c, _)| *c == container)
        .map_or(ed.doc.sections[si].blocks.len(), |(_, i)| i);
    let sel = ed.sel;
    ed.seal();
    ed.transact("Picture in line with text", None, |tx| {
        tx.step(Step::SetFrames { section: si, frames })?;
        tx.step(Step::InsertBlock { container: container.clone(), index, block: Block::Image(img) })?;
        if let Some(c) = caption {
            tx.step(Step::InsertBlock { container, index: index + 1, block: c })?;
        }
        Ok(if tx.doc.para(sel.focus.para).is_some() { sel } else { Selection::caret(tx.doc.start()) })
    })
}

/// A picture's caption: `Some(true)` above it, `Some(false)` below, `None`
/// none. The caret goes into a new caption, ready to type.
pub fn set_caption(ed: &mut Editor, image: NodeId, place: Option<bool>) -> Result<(), EditError> {
    let (container, index) = ed.doc.locate(image).ok_or(EditError::NotHere)?;
    let blocks = ed.doc.blocks(&container).ok_or(EditError::NotHere)?.clone();
    let is_caption = |i: usize| matches!(blocks.get(i), Some(Block::Paragraph(p)) if p.style == StyleSheet::CAPTION);
    let above = index.checked_sub(1).filter(|&i| is_caption(i));
    let below = Some(index + 1).filter(|&i| is_caption(i));
    // What the existing caption says (kept when it moves).
    let old = [above, below].into_iter().flatten().find_map(|i| match &blocks[i] {
        Block::Paragraph(p) => Some(p.clone()),
        _ => None,
    });
    let mut sel = ed.sel;
    ed.seal();
    ed.transact("Caption", None, |tx| {
        let mut at = index;
        if let Some(i) = below {
            tx.step(Step::RemoveBlock { container: container.clone(), index: i })?;
        }
        if let Some(i) = above {
            tx.step(Step::RemoveBlock { container: container.clone(), index: i })?;
            at -= 1;
        }
        if let Some(top) = place {
            let p = match &old {
                Some(p) => p.clone(),
                None => {
                    let id = tx.doc.alloc_id();
                    let mut p = Paragraph::new(id, StyleSheet::CAPTION, "");
                    p.props.align = Some(crate::Align::Center);
                    p
                }
            };
            let pid = p.id;
            let len = p.text.len();
            let index = if top { at } else { at + 1 };
            tx.step(Step::InsertBlock { container: container.clone(), index, block: Block::Paragraph(p) })?;
            sel = Selection::caret(Pos::new(pid, len));
        }
        // A picture in a box: the box grows (or shrinks) to fit the caption.
        if let crate::Container::Frame(f) = container
            && let Some((si, fi)) = tx
                .doc
                .sections
                .iter()
                .enumerate()
                .find_map(|(si, s)| s.frames.iter().position(|x| x.id == f).map(|fi| (si, fi)))
            && let Some(Block::Image(img)) = tx.doc.block(image)
        {
            let mut frames = tx.doc.sections[si].frames.clone();
            frames[fi].height = img.height.max(8.0) + if place.is_some() { 22.0 } else { 0.0 };
            tx.step(Step::SetFrames { section: si, frames })?;
        }
        Ok(fix_sel(tx.doc, sel))
    })?;
    ed.seal();
    Ok(())
}

/// `sel` if its paragraphs still exist, else the start of the document.
fn fix_sel(doc: &crate::Document, sel: Selection) -> Selection {
    if doc.para(sel.anchor.para).is_some() && doc.para(sel.focus.para).is_some() {
        sel
    } else {
        Selection::caret(doc.start())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, ops};

    #[test]
    fn frames_are_edited_and_undone() {
        let mut ed = Editor::new(Document::from_plain_text("Body"));
        let id = insert_frame(&mut ed, 0, Frame { x: 300.0, y: 80.0, ..Default::default() }).unwrap();
        assert_eq!(frame_of(&ed, ed.sel.focus.para), Some(id));
        ops::insert_text(&mut ed, "Boxed", None).unwrap();
        ops::split_paragraph(&mut ed).unwrap();
        ops::insert_text(&mut ed, "more", None).unwrap();
        let texts: Vec<_> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["Body", "Boxed", "more"]);
        ed.seal();
        for dx in [10.0, 20.0, 30.0] {
            update_frame(&mut ed, id, "Move", |f| f.x = 300.0 + dx).unwrap();
        }
        assert_eq!(ed.doc.frame(id).unwrap().x, 330.0);
        ed.undo();
        assert_eq!(ed.doc.frame(id).unwrap().x, 300.0, "one drag is one undo step");
        delete_frame(&mut ed, id).unwrap();
        assert!(ed.doc.frame(id).is_none());
        assert_eq!(ed.doc.paragraphs().count(), 1);
        while ed.undo() {}
        assert!(ed.doc.sections[0].frames.is_empty());
        // Frames survive JSON and repair.
        insert_frame(&mut ed, 0, Frame::default()).unwrap();
        let json = serde_json::to_string(&ed.doc).unwrap();
        let mut back: Document = serde_json::from_str(&json).unwrap();
        back.repair();
        assert_eq!(back.sections[0].frames.len(), 1);
    }
}
