//! New documents: a blank page. A template in Markdown can hold
//! `{{Your name}}`: it becomes a fill-in box on the page, grey "Your name"
//! until you click it and type.

use serde::Serialize;
use xenify_wp_doc::Document;

/// The tag of fill-in boxes made from `{{…}}`: once typed in they look like
/// ordinary text (other form controls stay tinted on screen).
pub const FILL_TAG: &str = xenify_wp_doc::controls::FILL_TAG;

/// One kind of document in New document.
#[derive(Clone, Debug, Serialize)]
pub struct Template {
    pub id: &'static str,
    pub group: &'static str,
    pub name: &'static str,
    /// One sentence on what it's for.
    pub about: &'static str,
    /// The writing profile it's checked with (xenify-grammar profile ids).
    pub profile: &'static str,
    /// Who usually reads it, and in what tone (the details' first guesses).
    pub reader: &'static str,
    pub tone: &'static str,
    #[serde(skip)]
    pub source: &'static str,
}

#[allow(clippy::too_many_arguments)]
const fn t(
    id: &'static str,
    group: &'static str,
    name: &'static str,
    about: &'static str,
    profile: &'static str,
    reader: &'static str,
    tone: &'static str,
    source: &'static str,
) -> Template {
    Template { id, group, name, about, profile, reader, tone, source }
}

/// Only the blank page: the built-in examples were dropped (2026-10-19, they
/// weren't good enough to start from). People's own templates come from
/// File › Save as template (`xenify-write-host`).
pub const TEMPLATES: &[Template] =
    &[t("blank", "Start", "Blank document", "An empty page.", "standard", "General readers", "Neutral", "")];

pub fn find(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

/// A new document from template `id` (blank when it has no source).
pub fn document(id: &str) -> Option<Document> {
    let t = find(id)?;
    let mut doc = if t.source.is_empty() {
        Document::default()
    } else {
        let mut doc = xenify_wp_io_load(t.source)?;
        fill_ins(&mut doc);
        doc
    };
    doc.meta.proofing_profile = Some(t.profile.to_string());
    Some(doc)
}

fn xenify_wp_io_load(src: &str) -> Option<Document> {
    xenify_wp_io::load(src.as_bytes(), xenify_wp_io::Format::Markdown).ok().map(|l| l.doc)
}

/// The fill-in labels a template has (for "You'll fill in … on the page").
pub fn fill_in_labels(id: &str) -> Vec<String> {
    let Some(t) = find(id) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    let mut rest = t.source;
    while let Some(a) = rest.find("{{") {
        let Some(b) = rest[a..].find("}}") else { break };
        let label = rest[a + 2..a + b].trim().to_string();
        if !out.contains(&label) {
            out.push(label);
        }
        rest = &rest[a + b + 2..];
    }
    out
}

/// Turns every `{{label}}` in the document's text into a fill-in box
/// showing "label" in grey until it is typed over.
pub fn fill_ins(doc: &mut Document) {
    xenify_wp_doc::controls::fill_ins(doc);
}

/// Puts `title` in the document: into the first fill-in box of a Title
/// paragraph when there is one, and always into the document's title.
pub fn apply_title(doc: Document, title: &str) -> Document {
    let title = title.trim();
    if title.is_empty() {
        return doc;
    }
    let mut ed = xenify_wp_doc::Editor::new(doc);
    let target = ed.doc.paragraphs().find(|p| p.style == xenify_wp_doc::StyleSheet::TITLE).and_then(|p| {
        p.run_ranges().find(|(_, f)| f.control.is_some()).map(|(r, _)| xenify_wp_doc::Pos::new(p.id, r.start))
    });
    if let Some(pos) = target {
        ed.sel = xenify_wp_doc::Selection::caret(pos);
        let _ = xenify_wp_doc::controls::set_value(&mut ed, pos, title);
    }
    let mut doc = ed.doc;
    doc.meta.title = title.to_string();
    doc
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_template_loads_and_fill_ins_become_boxes() {
        for t in super::TEMPLATES {
            let doc = super::document(t.id).unwrap_or_else(|| panic!("{} didn't load", t.id));
            if t.source.contains("{{") {
                assert!(!doc.meta.controls.is_empty(), "{}", t.id);
                assert!(doc.paragraphs().all(|p| !p.text.contains("{{")), "{}", t.id);
                assert!(!super::fill_in_labels(t.id).is_empty());
            }
        }
    }
}
