//! Markdown, the default file format: CommonMark plus the Pandoc extensions
//! Xenify Write needs. Documents written here read back the same, and stay
//! readable (and mostly meaningful) in any other Markdown tool.
//!
//! - YAML front matter: title, author, audience, tone, page setup, changed
//!   or added style definitions, custom lists.
//! - `::: {custom-style="Name" align=center}` fenced divs: paragraph styles
//!   without Markdown syntax (Title, Subtitle, Caption, your own) and direct
//!   paragraph formatting.
//! - `[text]{.underline .mark color=#c00 size=14 font="Name"}` bracketed
//!   spans: character formatting without Markdown syntax.

mod attrs;
mod front;
mod read;
mod write;

use xenify_wp_doc::{Document, ListDef, StyleSheet};

pub use front::source_from_csl;
pub use write::MONO;

pub fn read(bytes: &[u8]) -> crate::Loaded {
    let text = String::from_utf8_lossy(bytes);
    let (doc, warnings) = read::read(&text);
    crate::Loaded { doc, warnings }
}

pub fn write(doc: &Document) -> String {
    let mut s = write::write(doc);
    s.push('\n');
    s
}

/// What saving `doc` as Markdown would lose (empty: nothing).
pub fn losses(doc: &Document) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    if doc.sections.iter().skip(1).any(|s| !s.headers.is_empty()) {
        out.insert("rich headers and footers of later sections".to_string());
    }
    for s in &doc.sections {
        for b in &s.blocks {
            let xenify_wp_doc::Block::Table(t) = b else { continue };
            for row in &t.rows {
                for cell in &row.cells {
                    for b in &cell.blocks {
                        match b {
                            xenify_wp_doc::Block::Paragraph(p)
                                if p.style != StyleSheet::NORMAL || p.list.is_some() || !p.props.is_empty() =>
                            {
                                out.insert("paragraph styles inside table cells".to_string());
                            }
                            xenify_wp_doc::Block::Paragraph(_) | xenify_wp_doc::Block::Image(_) => {}
                            _ => {
                                out.insert("tables inside table cells".to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    for p in doc.paragraphs() {
        if let Some(item) = &p.list {
            if item.list != ListDef::BULLETS && item.list != ListDef::NUMBERS && item.list != ListDef::CHECKLIST {
                let name = doc.list_def(&item.list).map_or(item.list.clone(), |d| d.name.clone());
                out.insert(format!("the \"{name}\" list style (becomes plain numbering)"));
            }
            if p.style != StyleSheet::NORMAL || !p.props.is_empty() {
                out.insert("paragraph styles and formatting on list items".to_string());
            }
        }
        if p.style == StyleSheet::CODE && p.runs.iter().any(|r| !r.fmt.is_empty()) {
            out.insert("formatting inside code blocks".to_string());
        }
    }
    out.into_iter().collect()
}
