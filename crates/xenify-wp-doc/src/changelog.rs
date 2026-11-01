//! The change log: "Sam changed this on 11 Oct 2026: …" entries kept in
//! the document (File › Record a change), which can be shown in it as a
//! table that updates itself (Insert › Change log).

use crate::compare::{Edit, diff};
use crate::{Document, Editor, StyleSheet};
use serde::{Deserialize, Serialize};

/// One entry: who, when, what.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogEntry {
    pub author: String,
    /// ISO 8601 (UTC).
    pub date: String,
    pub summary: String,
}

/// The headings (outline level) above each paragraph, by paragraph index.
fn headings(doc: &Document) -> Vec<(String, Option<String>)> {
    let mut current: Option<String> = None;
    doc.text_paragraphs()
        .map(|p| {
            if doc.styles.resolve(&p.style).outline_level.is_some() && !p.text.trim().is_empty() {
                current = Some(p.text.trim().to_string());
            }
            (p.text.clone(), current.clone())
        })
        .collect()
}

/// A suggested summary of what changed from `old` to `new`: the sections
/// (by heading) that changed and how many words came and went.
pub fn summarize(old: &Document, new: &Document) -> String {
    let (a, b) = (headings(old), headings(new));
    let ta: Vec<&str> = a.iter().map(|(t, _)| t.as_str()).collect();
    let tb: Vec<&str> = b.iter().map(|(t, _)| t.as_str()).collect();
    let mut places: Vec<String> = Vec::new();
    let mut note = |h: &Option<String>| {
        let name = h.clone().unwrap_or_else(|| "the opening".into());
        if !places.contains(&name) {
            places.push(name);
        }
    };
    for e in diff(&ta, &tb) {
        match e {
            Edit::Added(j) => note(&b[j].1),
            Edit::Removed(i) => note(&a[i].1),
            Edit::Same(..) => {}
        }
    }
    let (added, removed) = crate::compare::word_changes(old, new);
    if places.is_empty() && added == 0 && removed == 0 {
        return "No changes to the text".into();
    }
    let counts = match (added, removed) {
        (0, 0) => "layout only".to_string(),
        (a, 0) => format!("{a} word{} added", if a == 1 { "" } else { "s" }),
        (0, r) => format!("{r} word{} removed", if r == 1 { "" } else { "s" }),
        (a, r) => format!("{a} added, {r} removed"),
    };
    let quoted: Vec<String> = places.iter().take(3).map(|p| format!("“{p}”")).collect();
    let more = places.len().saturating_sub(3);
    let list = match quoted.len() {
        0 => String::new(),
        1 => quoted[0].clone(),
        n => format!("{} and {}", quoted[..n - 1].join(", "), quoted[n - 1]),
    };
    let more = if more > 0 { format!(" and {more} more") } else { String::new() };
    if list.is_empty() { format!("Edited ({counts})") } else { format!("Edited {list}{more} ({counts})") }
}

/// File › Record a change: adds an entry (signed by the document's user, dated now).
pub fn record(ed: &mut Editor, summary: &str) -> Result<(), crate::EditError> {
    let author = if ed.doc.user.trim().is_empty() { "Author".to_string() } else { ed.doc.user.clone() };
    let mut meta = ed.doc.meta.clone();
    meta.changelog.push(LogEntry { author, date: crate::review::now(), summary: summary.trim().to_string() });
    crate::ops::set_meta(ed, meta, "Record a change")
}

/// Removes entry `index`.
pub fn remove(ed: &mut Editor, index: usize) -> Result<(), crate::EditError> {
    let mut meta = ed.doc.meta.clone();
    if index < meta.changelog.len() {
        meta.changelog.remove(index);
    }
    crate::ops::set_meta(ed, meta, "Remove change log entry")
}

/// "11 Oct 2026" from an ISO date.
pub fn day(iso: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let d = iso.split('T').next().unwrap_or(iso);
    let parts: Vec<&str> = d.split('-').collect();
    match parts.as_slice() {
        [y, m, dd] => {
            let month = m.parse::<usize>().ok().and_then(|m| MONTHS.get(m.wrapping_sub(1))).copied().unwrap_or(m);
            format!("{} {month} {y}", dd.trim_start_matches('0'))
        }
        _ => iso.to_string(),
    }
}

/// The log as rows (date, name, change), oldest first, with a heading row.
pub fn rows(doc: &Document) -> Vec<[String; 3]> {
    let mut v = vec![["Date".to_string(), "Name".to_string(), "Change".to_string()]];
    v.extend(doc.meta.changelog.iter().map(|e| [day(&e.date), e.author.clone(), e.summary.clone()]));
    v
}

/// The log as a table (header row, then one row per entry), for showing
/// and exporting. Its ids are made up: it isn't part of the document.
pub fn table(doc: &Document) -> crate::Table {
    let mut n = 0u64;
    let mut id = || {
        n += 1;
        crate::NodeId(u64::MAX - n)
    };
    let mut t = crate::Table::new(1, 3, &mut id);
    t.rows.clear();
    for (i, row) in rows(doc).into_iter().enumerate() {
        let cells = row
            .into_iter()
            .map(|text| {
                let (cid, pid) = (id(), id());
                let mut c = crate::Cell::new(cid, pid);
                let mut p = crate::Paragraph::new(pid, StyleSheet::NORMAL, text);
                if i == 0 {
                    for r in &mut p.runs {
                        r.fmt.bold = Some(true);
                    }
                }
                c.blocks = vec![crate::Block::Paragraph(p)];
                c
            })
            .collect();
        t.rows.push(crate::Row { id: id(), header: i == 0, footer: false, repeat: None, min_height: None, cells });
    }
    t.widths = vec![1.0, 1.0, 3.0];
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_name_the_sections_that_changed() {
        let old = Document::from_plain_text(
            "Introduction\nWe start here.\nMethod\nWe measured things.\nResults\nThey were good.",
        );
        let mut new = old.clone();
        let set_heading = |d: &mut Document| {
            let ids: Vec<_> = d.paragraphs().map(|p| p.id).collect();
            for i in [0, 2, 4] {
                d.para_mut(ids[i]).unwrap().style = StyleSheet::heading(1);
            }
        };
        let mut old = old;
        set_heading(&mut old);
        set_heading(&mut new);
        let ids: Vec<_> = new.paragraphs().map(|p| p.id).collect();
        new.para_mut(ids[3]).unwrap().text = "We measured many more things carefully.".into();
        new.para_mut(ids[3]).unwrap().runs[0].len = new.para(ids[3]).unwrap().text.len();
        assert_eq!(
            summarize(&old, &new),
            "Edited “Method” (3 added, 0 removed)".replace(", 0 removed", "").replace("3 added", "3 words added")
        );
        assert_eq!(summarize(&old, &old), "No changes to the text");
        assert_eq!(day("2026-10-11T08:00:00Z"), "11 Oct 2026");
    }

    #[test]
    fn entries_are_recorded_and_listed() {
        let mut ed = Editor::new(Document::from_plain_text("Text."));
        ed.doc.user = "Sam".into();
        record(&mut ed, "First draft").unwrap();
        let r = rows(&ed.doc);
        assert_eq!(r.len(), 2);
        assert_eq!((r[1][1].as_str(), r[1][2].as_str()), ("Sam", "First draft"));
        ed.undo();
        assert!(ed.doc.meta.changelog.is_empty());
    }
}
