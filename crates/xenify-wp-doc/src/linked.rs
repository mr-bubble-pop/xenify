//! Documents made from a template stay linked to it: the template owns the
//! styles and the page setup (size, margins, headers and footers, including
//! different first, odd and even pages), the document owns its text.
//! Changing the template and updating a document brings the template's look
//! in; nothing in a document ever changes the template.

use crate::controls::FILL_TAG;
use crate::edit::Content;
use crate::{Block, CharFmt, Document, EditError, Editor, NodeId, Run, Step};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Which template a document follows, and the version of it last applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateLink {
    pub name: String,
    /// [`version`] of the template when it was last applied.
    pub version: String,
    /// Where the template is: `mine:NAME` (File › Save as template),
    /// `file:PATH`, or `server:URL|NODE`. Empty: one of the person's own, by name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// How many blocks (all sections in order) at the start and at the end
    /// are the template's own: read-only here, apart from its fields.
    #[serde(skip_serializing_if = "is_zero")]
    pub before: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub after: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A fingerprint of what a template gives its documents (styles, page
/// setup, its own colours, and its fixed parts).
pub fn version(template: &Document) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&template.styles).unwrap_or_default().hash(&mut h);
    if template.meta.body_slot.is_some() {
        serde_json::to_string(&template.sections.iter().map(|s| &s.blocks).collect::<Vec<_>>())
            .unwrap_or_default()
            .hash(&mut h);
    }
    if !template.meta.colors.is_empty() || template.meta.table_colors.is_some() {
        serde_json::to_string(&(&template.meta.colors, &template.meta.table_colors)).unwrap_or_default().hash(&mut h);
    }
    for s in &template.sections {
        serde_json::to_string(&s.page).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Whether `doc` follows `name` at an older version than `template`.
pub fn out_of_date(doc: &Document, template: &Document) -> bool {
    doc.meta.template.as_ref().is_some_and(|t| t.version != version(template))
}

/// Brings the template's styles, page setup and colours into the document (one undo
/// step), and links it to the template. Styles only the document has stay;
/// the text, pictures and comments aren't touched.
pub fn apply(ed: &mut Editor, template: &Document, name: &str) -> Result<(), EditError> {
    let doc = &ed.doc;
    let mut styles = template.styles.clone();
    for own in &doc.styles.paragraph {
        if styles.get(&own.id).is_none() {
            styles.paragraph.push(own.clone());
        }
    }
    let mut sections = doc.sections.clone();
    for (i, s) in sections.iter_mut().enumerate() {
        // Section i takes the template's section i (or its last one).
        if let Some(t) = template.sections.get(i).or_else(|| template.sections.last()) {
            s.page = t.page.clone();
        }
    }
    let mut meta = doc.meta.clone();
    let from = meta
        .template
        .as_ref()
        .filter(|t| t.name == name)
        .map(|t| (t.from.clone(), t.before, t.after))
        .unwrap_or_default();
    meta.template = Some(TemplateLink {
        name: name.to_string(),
        version: version(template),
        from: from.0,
        before: from.1,
        after: from.2,
    });
    if template.meta.table_colors.is_some() {
        meta.table_colors = template.meta.table_colors.clone();
    }
    // The template's colours first, then the document's own.
    let own = std::mem::take(&mut meta.colors);
    for c in template.meta.colors.iter().chain(&own) {
        if !meta.colors.contains(c) {
            meta.colors.push(*c);
        }
    }
    let content = Content { sections, styles, lists: doc.lists.clone(), meta };
    let sel = ed.sel;
    ed.seal();
    ed.transact("Update from template", None, |tx| {
        tx.step(Step::SetContent { content: Box::new(content) })?;
        Ok(sel)
    })?;
    ed.seal();
    Ok(())
}

/// Every block of the document's sections, in order.
pub fn flat(doc: &Document) -> Vec<&Block> {
    doc.sections.iter().flat_map(|s| s.blocks.iter()).collect()
}

/// The template's blocks at the start and the end of `doc` (empty when it
/// has no template with fixed parts).
pub fn fixed_blocks(doc: &Document) -> (Vec<&Block>, Vec<&Block>) {
    let Some(t) = &doc.meta.template else { return (Vec::new(), Vec::new()) };
    let all = flat(doc);
    let before = t.before.min(all.len());
    let after = t.after.min(all.len() - before);
    (all[..before].to_vec(), all[all.len() - after..].to_vec())
}

/// Whether paragraph `para` is in a part that comes from the template.
pub fn in_fixed(doc: &Document, para: NodeId) -> bool {
    let (a, b) = fixed_blocks(doc);
    a.iter().chain(&b).any(|blk| holds(blk, para))
}

fn holds(b: &Block, para: NodeId) -> bool {
    match b {
        Block::Paragraph(p) => p.id == para,
        Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).any(|b| holds(b, para)),
        _ => false,
    }
}

/// The text of a template field (a `{{label}}` fill-in) is the document's;
/// the rest of these paragraphs is the template's.
fn is_field(doc: &Document, fmt: &CharFmt) -> bool {
    fmt.control.and_then(|id| doc.control(id)).is_some_and(|c| c.tag == FILL_TAG)
}

fn without_fields(doc: &Document, b: &Block) -> Block {
    let mut b = b.clone();
    fn go(doc: &Document, b: &mut Block) {
        match b {
            Block::Paragraph(p) => {
                let (mut text, mut runs) = (String::new(), Vec::new());
                for (r, f) in p.run_ranges() {
                    if !is_field(doc, f) {
                        text.push_str(&p.text[r.clone()]);
                        runs.push(Run { len: r.len(), fmt: f.clone() });
                    }
                }
                p.text = text;
                p.runs = runs;
            }
            Block::Table(t) => {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    for b in &mut c.blocks {
                        go(doc, b);
                    }
                }
            }
            _ => {}
        }
    }
    go(doc, &mut b);
    b
}

/// A fingerprint of the template's parts of `doc`, fields left out: an edit
/// that changes it changed something only the template may.
pub fn fixed_print(doc: &Document) -> Option<u64> {
    let t = doc.meta.template.as_ref()?;
    if t.before + t.after == 0 {
        return None;
    }
    let (a, b) = fixed_blocks(doc);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for blk in a.iter().chain(&b) {
        serde_json::to_string(&without_fields(doc, blk)).unwrap_or_default().hash(&mut h);
    }
    (a.len(), b.len()).hash(&mut h);
    Some(h.finish())
}

/// What the fields in the template's parts of `doc` hold: label → text
/// (only the ones typed in).
pub fn field_values(doc: &Document) -> Vec<(String, String)> {
    let (a, b) = fixed_blocks(doc);
    let mut out: Vec<(String, String, NodeId)> = Vec::new();
    fn go(doc: &Document, b: &Block, out: &mut Vec<(String, String, NodeId)>) {
        match b {
            Block::Paragraph(p) => {
                for (r, f) in p.run_ranges() {
                    let Some(c) = f.control.and_then(|id| doc.control(id)) else { continue };
                    if c.tag != FILL_TAG || c.placeholder {
                        continue;
                    }
                    match out.iter_mut().find(|(l, _, _)| *l == c.title) {
                        // One field split over runs (bold in the middle): its pieces join.
                        Some((_, v, id)) if *id == c.id => v.push_str(&p.text[r]),
                        // Another field with the same label: the first one counts.
                        Some(_) => {}
                        None => out.push((c.title.clone(), p.text[r].to_string(), c.id)),
                    }
                }
            }
            Block::Table(t) => {
                t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).for_each(|b| go(doc, b, out))
            }
            _ => {}
        }
    }
    for blk in a.iter().chain(&b) {
        go(doc, blk, &mut out);
    }
    out.into_iter().map(|(l, v, _)| (l, v)).collect()
}

/// Puts `values` (label → text) into the fields of `doc`'s paragraphs in
/// `blocks` (the first field with each label; others with it too).
pub fn set_field_values(doc: &mut Document, values: &[(String, String)]) {
    let controls = doc.meta.controls.clone();
    let mut filled: Vec<NodeId> = Vec::new();
    fn go(b: &mut Block, controls: &[crate::controls::Control], values: &[(String, String)], filled: &mut Vec<NodeId>) {
        match b {
            Block::Paragraph(p) => {
                let (mut text, mut runs) = (String::new(), Vec::<Run>::new());
                let mut done: Vec<NodeId> = Vec::new();
                for (r, f) in p.run_ranges() {
                    let field = f.control.and_then(|id| controls.iter().find(|c| c.id == id && c.tag == FILL_TAG));
                    let value = field.and_then(|c| values.iter().find(|(l, _)| *l == c.title)).map(|(_, v)| v);
                    match (field, value) {
                        (Some(c), Some(v)) => {
                            // The whole field once, in its first run's format.
                            if !done.contains(&c.id) {
                                done.push(c.id);
                                text.push_str(v);
                                runs.push(Run { len: v.len(), fmt: f.clone() });
                                filled.push(c.id);
                            }
                        }
                        _ => {
                            text.push_str(&p.text[r.clone()]);
                            runs.push(Run { len: r.len(), fmt: f.clone() });
                        }
                    }
                }
                runs.retain(|r| r.len > 0);
                p.text = text;
                p.runs = runs;
            }
            Block::Table(t) => {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    for b in &mut c.blocks {
                        go(b, controls, values, filled);
                    }
                }
            }
            _ => {}
        }
    }
    for s in &mut doc.sections {
        for b in &mut s.blocks {
            go(b, &controls, values, &mut filled);
        }
    }
    for c in &mut doc.meta.controls {
        if filled.contains(&c.id) {
            c.placeholder = false;
        }
    }
}

/// Where the body goes in `template`: (flat index of the body paragraph,
/// number of blocks after it). None when it has no body marked.
pub fn slot(template: &Document) -> Option<(usize, usize)> {
    let id = template.meta.body_slot?;
    let all = flat(template);
    let i = all.iter().position(|b| matches!(b, Block::Paragraph(p) if p.id == id))?;
    Some((i, all.len() - i - 1))
}

/// A new document from `template` (its fields still prompts): the template's
/// parts, and an empty paragraph where the body goes. `from` says where the template is.
pub fn new_document(template: &Document, name: &str, from: &str) -> Document {
    let mut doc = template.clone();
    let (before, after) = slot(template).unwrap_or((0, 0));
    if let Some(id) = template.meta.body_slot {
        // The body starts as one empty paragraph in the slot's style.
        for s in &mut doc.sections {
            for b in &mut s.blocks {
                if let Block::Paragraph(p) = b
                    && p.id == id
                {
                    p.text.clear();
                    p.runs.clear();
                }
            }
        }
    }
    doc.meta.is_template = false;
    doc.meta.body_slot = None;
    doc.meta.template = Some(TemplateLink {
        name: name.to_string(),
        version: version(template),
        from: from.to_string(),
        before: if template.meta.body_slot.is_some() { before } else { 0 },
        after,
    });
    doc
}

/// `doc` with the template's parts as `template` has them now (field values
/// kept by label, its body untouched), as content for one undo step.
/// `template` must already have its fields as fill-ins, with ids above
/// `doc`'s (see `Document::renumber`).
pub fn refreshed(doc: &Document, template: &Document) -> Option<Document> {
    let link = doc.meta.template.clone()?;
    let (t_before, t_after) = slot(template)?;
    let values = field_values(doc);
    // The body: the document's blocks between the template's parts, by section.
    let mut body: Vec<(usize, Vec<Block>)> = Vec::new();
    let total: usize = doc.sections.iter().map(|s| s.blocks.len()).sum();
    let (lo, hi) = (link.before.min(total), total - link.after.min(total - link.before.min(total)));
    let mut at = 0;
    for (si, s) in doc.sections.iter().enumerate() {
        let blocks: Vec<Block> =
            s.blocks.iter().enumerate().filter(|(k, _)| (lo..hi).contains(&(at + k))).map(|(_, b)| b.clone()).collect();
        if !blocks.is_empty() {
            body.push((si, blocks));
        }
        at += s.blocks.len();
    }
    if body.is_empty() {
        body.push((0, vec![Block::Paragraph(crate::Paragraph { id: NodeId(template.next_id), ..Default::default() })]));
    }
    // The template's sections, the body in place of the slot (several of the
    // document's sections when the body has section breaks of its own).
    let mut out = template.clone();
    let mut sections: Vec<crate::Section> = Vec::new();
    let mut at = 0;
    for s in &template.sections {
        let n = s.blocks.len();
        if (at..at + n).contains(&t_before) {
            let k = t_before - at;
            let (head, tail) = (s.blocks[..k].to_vec(), s.blocks[k + 1..].to_vec());
            let count = body.len();
            for (j, (si, blocks)) in body.iter().enumerate() {
                let mut sec = if j == 0 { s.clone() } else { doc.sections[*si].clone() };
                sec.blocks = Vec::new();
                if j == 0 {
                    sec.blocks.extend(head.clone());
                }
                sec.blocks.extend(blocks.iter().cloned());
                if j + 1 == count {
                    sec.blocks.extend(tail.clone());
                }
                sections.push(sec);
            }
        } else {
            sections.push(s.clone());
        }
        at += n;
    }
    out.sections = sections;
    // The document's own things stay: its details, comments, sources, notes, pictures.
    let mut meta = doc.meta.clone();
    meta.controls.retain(|c| c.tag != FILL_TAG || !template.meta.controls.iter().any(|t| t.id == c.id));
    for c in &template.meta.controls {
        if !meta.controls.iter().any(|m| m.id == c.id) {
            meta.controls.push(c.clone());
        }
    }
    meta.template = Some(TemplateLink { version: version(template), before: t_before, after: t_after, ..link });
    if template.meta.table_colors.is_some() {
        meta.table_colors = template.meta.table_colors.clone();
    }
    out.meta = meta;
    out.lists = doc.lists.clone();
    for l in &template.lists {
        if out.lists.iter().all(|m| m.id != l.id) {
            out.lists.push(l.clone());
        }
    }
    for own in &doc.styles.paragraph {
        if out.styles.get(&own.id).is_none() {
            out.styles.paragraph.push(own.clone());
        }
    }
    set_field_values(&mut out, &values);
    // Fields of the template's old parts that went with them.
    let mut used: Vec<NodeId> = Vec::new();
    out.for_each_para_mut(&mut |p| used.extend(p.runs.iter().filter_map(|r| r.fmt.control)));
    out.meta.controls.retain(|c| c.tag != FILL_TAG || used.contains(&c.id));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::{Control, ControlKind};

    /// A template: a title line with a field, the body spot, a closing line.
    fn quote_template(closing: &str) -> Document {
        let mut t = Document::from_plain_text(&format!("Quote for Client\nBODY\n{closing}"));
        t.next_id += 10;
        let ids: Vec<NodeId> = t.paragraphs().map(|p| p.id).collect();
        t.meta.body_slot = Some(ids[1]);
        t.meta.is_template = true;
        t
    }

    /// "Client" in paragraph 0 as a field (what `{{Client}}` becomes).
    fn with_field(mut d: Document) -> Document {
        let id = d.alloc_id();
        d.meta.controls.push(Control {
            id,
            kind: ControlKind::Text { rich: false },
            title: "Client".into(),
            tag: FILL_TAG.into(),
            placeholder: true,
        });
        let p = d.paragraphs().next().unwrap().id;
        d.for_each_para_mut(&mut |q| {
            if q.id == p {
                let fmt = CharFmt { control: Some(id), ..Default::default() };
                q.runs = vec![Run { len: 10, fmt: CharFmt::default() }, Run { len: 6, fmt }];
            }
        });
        d
    }

    #[test]
    fn a_document_from_a_template_keeps_its_parts_and_takes_its_changes() {
        let t = with_field(quote_template("Valid for 30 days"));
        let doc = new_document(&t, "Quote", "mine:Quote");
        let texts: Vec<String> = doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["Quote for Client", "", "Valid for 30 days"], "the body starts empty");
        let link = doc.meta.template.clone().unwrap();
        assert_eq!((link.before, link.after), (1, 1));
        assert!(!doc.meta.is_template);
        let mut ed = Editor::new(doc);
        // The body is the document's: typing there leaves the template's parts as they were.
        let print = fixed_print(&ed.doc).unwrap();
        let body = ed.doc.paragraphs().nth(1).unwrap().id;
        ed.sel = crate::Selection::caret(crate::Pos::new(body, 0));
        crate::ops::insert_text(&mut ed, "Two chairs, £40.", None).unwrap();
        assert_eq!(fixed_print(&ed.doc), Some(print));
        assert!(!in_fixed(&ed.doc, body));
        // A field typed in is the document's too.
        let top = ed.doc.paragraphs().next().unwrap().id;
        assert!(in_fixed(&ed.doc, top));
        assert!(crate::controls::set_value(&mut ed, crate::Pos::new(top, 12), "Ada Shop").unwrap());
        assert_eq!(fixed_print(&ed.doc), Some(print));
        assert_eq!(field_values(&ed.doc), [("Client".to_string(), "Ada Shop".to_string())]);
        // Typing in the template's own words changes its part: rolled back exactly.
        let cp = ed.checkpoint();
        ed.sel = crate::Selection::caret(crate::Pos::new(top, 0));
        crate::ops::insert_text(&mut ed, "X", None).unwrap();
        assert_ne!(fixed_print(&ed.doc), Some(print));
        ed.roll_back(cp);
        assert_eq!(ed.doc.paragraphs().next().unwrap().text, "Quote for Ada Shop");
        assert_eq!(ed.doc.paragraphs().nth(1).unwrap().text, "Two chairs, £40.", "earlier typing stays");
        // The template changes its closing line: the document takes it, keeping its body and field.
        let mut t2 = with_field(quote_template("Valid for 60 days"));
        t2.renumber(ed.doc.next_id);
        let new = refreshed(&ed.doc, &t2).unwrap();
        let texts: Vec<String> = new.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["Quote for Ada Shop", "Two chairs, £40.", "Valid for 60 days"]);
        assert_ne!(new.meta.template.as_ref().unwrap().version, link.version);
        assert_eq!(new.meta.controls.len(), 1, "the old field went with the old part");
        // Printed or exported, an unfilled field shows nothing.
        let fresh = new_document(&t, "Quote", "");
        assert_eq!(crate::controls::without_prompts(&fresh).paragraphs().next().unwrap().text, "Quote for ");
    }

    #[test]
    fn a_template_brings_its_look_and_not_its_text() {
        let mut tpl = Document::from_plain_text("Template text");
        tpl.sections[0].page.footer.right = "{page}".into();
        tpl.sections[0].page.margins.left = 100.0;
        tpl.styles.get_mut(crate::StyleSheet::NORMAL).unwrap().char.size = Some(14.0);
        let mut ed = Editor::new(Document::from_plain_text("My own words"));
        apply(&mut ed, &tpl, "Report").unwrap();
        let d = &ed.doc;
        assert_eq!(d.paragraphs().next().unwrap().text, "My own words");
        assert_eq!(d.sections[0].page.footer.right, "{page}");
        assert_eq!(d.styles.get(crate::StyleSheet::NORMAL).unwrap().char.size, Some(14.0));
        assert!(!out_of_date(d, &tpl));
        // Its colours come too, the document's own stay after them.
        let brand = crate::Rgba::rgb(0x12, 0x34, 0x56);
        let mine = crate::Rgba::rgb(1, 2, 3);
        tpl.meta.colors = vec![brand];
        ed.doc.meta.colors = vec![mine, brand];
        apply(&mut ed, &tpl, "Report").unwrap();
        assert_eq!(ed.doc.meta.colors, [brand, mine]);
        ed.undo();
        tpl.meta.colors.clear();
        let d = &ed.doc;
        // The template changes: the document is behind until it's updated again.
        tpl.sections[0].page.margins.left = 72.0;
        assert!(out_of_date(d, &tpl));
        // One undo puts the document back as it was.
        ed.undo();
        assert!(ed.doc.meta.template.is_none());
    }
}
