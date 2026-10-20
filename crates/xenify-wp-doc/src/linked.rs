//! Documents made from a template stay linked to it: the template owns the
//! styles and the page setup (size, margins, headers and footers, including
//! different first, odd and even pages), the document owns its text.
//! Changing the template and updating a document brings the template's look
//! in; nothing in a document ever changes the template.

use crate::edit::Content;
use crate::{Document, EditError, Editor, Step};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Which template a document follows, and the version of it last applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateLink {
    pub name: String,
    /// [`version`] of the template when it was last applied.
    pub version: String,
}

/// A fingerprint of what a template gives its documents (styles and page setup).
pub fn version(template: &Document) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&template.styles).unwrap_or_default().hash(&mut h);
    for s in &template.sections {
        serde_json::to_string(&s.page).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Whether `doc` follows `name` at an older version than `template`.
pub fn out_of_date(doc: &Document, template: &Document) -> bool {
    doc.meta.template.as_ref().is_some_and(|t| t.version != version(template))
}

/// Brings the template's styles and page setup into the document (one undo
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
    meta.template = Some(TemplateLink { name: name.to_string(), version: version(template) });
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // The template changes: the document is behind until it's updated again.
        tpl.sections[0].page.margins.left = 72.0;
        assert!(out_of_date(d, &tpl));
        // One undo puts the document back as it was.
        ed.undo();
        assert!(ed.doc.meta.template.is_none());
    }
}
