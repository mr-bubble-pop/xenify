//! Mail merge: `{{field}}` placeholders in the text are filled from a
//! record (a JSON object: a CSV row, or an object from a JSON file).
//!
//! - `{{name}}`, `{{address.city}}` (dotted paths into nested objects),
//!   `{{name|Friend}}` (a fallback when the field is missing or empty).
//! - A table row whose `repeat` names a list field (e.g. `items`) is
//!   repeated once per item; inside it `{{items.qty}}` or `{{.qty}}` refer to
//!   the current item (invoice lines).
//! - `{{#}}` is the record's number (1-based).

use crate::{Block, Document, NodeId, Paragraph, Section, SectionStart, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a document's merge data comes from, and how outputs are named.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MergeConfig {
    /// Data file (CSV or JSON), relative to the document.
    pub source: String,
    /// Output file names, with fields, e.g. `{{last}}-{{first}}`.
    pub file_name: String,
    /// Rows with the same value in this column make one record (their rows
    /// are in `rows`, e.g. for invoice lines from a spreadsheet).
    pub group_by: String,
}

/// Every `{{…}}` placeholder: (byte range, path, fallback).
fn placeholders(text: &str) -> Vec<(std::ops::Range<usize>, String, Option<String>)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("{{") {
        let start = from + i;
        // `\{{` is a literal.
        if start > 0 && text.as_bytes()[start - 1] == b'\\' {
            from = start + 2;
            continue;
        }
        let Some(j) = text[start + 2..].find("}}") else { break };
        let end = start + 2 + j + 2;
        let inner = text[start + 2..end - 2].trim();
        let (path, fallback) = match inner.split_once('|') {
            Some((p, f)) => (p.trim().to_string(), Some(f.trim().to_string())),
            None => (inner.to_string(), None),
        };
        if !path.is_empty() && !path.contains('{') {
            out.push((start..end, path, fallback));
        }
        from = end;
    }
    out
}

/// The field names used in `doc` (top-level names, in order of appearance).
pub fn fields(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in doc.paragraphs() {
        for (_, path, _) in placeholders(&p.text) {
            let top = path.trim_start_matches('.').split('.').next().unwrap_or("").to_string();
            if !top.is_empty() && top != "#" && !out.contains(&top) {
                out.push(top);
            }
        }
    }
    out
}

/// The value at dotted `path` in `v`, as text.
pub fn lookup(v: &Value, path: &str) -> Option<String> {
    let mut cur = v;
    for part in path.split('.').filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(m) => {
                m.get(part).or_else(|| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(part)).map(|(_, v)| v))?
            }
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(match cur {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    })
}

/// Fills the placeholders of one paragraph (keeping formatting: each
/// placeholder takes the format of its first character).
fn fill(p: &mut Paragraph, record: &Value, item: Option<(&str, &Value)>, number: usize) {
    let found = placeholders(&p.text);
    for (range, path, fallback) in found.into_iter().rev() {
        let value = if path == "#" {
            Some(number.to_string())
        } else if let Some((list, it)) = item
            && let Some(rest) =
                path.strip_prefix('.').or_else(|| path.strip_prefix(list).and_then(|r| r.strip_prefix('.')))
        {
            lookup(it, rest)
        } else {
            lookup(record, &path)
        };
        let value = match value {
            Some(v) if !v.is_empty() => v,
            _ => fallback.unwrap_or_default(),
        };
        let fmt = p.fmt_at(range.start + 1);
        p.splice(range, Span::plain(value, fmt));
    }
}

fn fill_blocks(blocks: &mut [Block], record: &Value, item: Option<(&str, &Value)>, number: usize) {
    for b in blocks.iter_mut() {
        match b {
            Block::Paragraph(p) => fill(p, record, item, number),
            Block::Table(t) => {
                // Repeat rows first, then fill.
                let mut rows = Vec::new();
                for row in t.rows.drain(..) {
                    match row.repeat.as_deref().filter(|r| !r.is_empty()) {
                        Some(list) => {
                            let items = match serde_json::from_str::<Value>(&lookup(record, list).unwrap_or_default()) {
                                Ok(Value::Array(a)) => a,
                                _ => Vec::new(),
                            };
                            for it in &items {
                                let mut r = row.clone();
                                for cell in &mut r.cells {
                                    fill_blocks(&mut cell.blocks, record, Some((list, it)), number);
                                }
                                r.repeat = None;
                                rows.push(r);
                            }
                        }
                        None => {
                            let mut r = row;
                            for cell in &mut r.cells {
                                fill_blocks(&mut cell.blocks, record, item, number);
                            }
                            rows.push(r);
                        }
                    }
                }
                t.rows = rows;
            }
            Block::Image(img) => {
                // Pictures can come from the data too (`{{photo}}` as the source).
                if img.src.contains("{{") {
                    let mut p = Paragraph::new(NodeId(0), "", img.src.clone());
                    fill(&mut p, record, item, number);
                    img.src = p.text;
                }
            }
            Block::Toc(_) => {}
        }
    }
}

/// `doc` filled in with `record` (the `number`th, from 1).
pub fn instantiate(doc: &Document, record: &Value, number: usize) -> Document {
    let mut out = doc.clone();
    for s in &mut out.sections {
        for blocks in s.block_lists_mut() {
            fill_blocks(blocks, record, None, number);
        }
    }
    // Headers, footers and the title can use fields too.
    let mut t = Paragraph::new(NodeId(0), "", out.meta.title.clone());
    fill(&mut t, record, None, number);
    out.meta.title = t.text;
    for s in &mut out.sections {
        for line in [
            &mut s.page.header,
            &mut s.page.footer,
            &mut s.page.first_header,
            &mut s.page.first_footer,
            &mut s.page.even_header,
            &mut s.page.even_footer,
        ] {
            for part in [&mut line.left, &mut line.center, &mut line.right] {
                let mut p = Paragraph::new(NodeId(0), "", part.clone());
                fill(&mut p, record, None, number);
                *part = p.text;
            }
        }
    }
    out.revision += 1;
    out.for_each_para_mut(&mut |p| p.rev = out_rev(p.rev));
    out.repair();
    out
}

fn out_rev(r: u64) -> u64 {
    r.wrapping_add(1)
}

/// A file name for a record from `template` (unsafe characters removed).
pub fn file_name(template: &str, record: &Value, number: usize) -> String {
    let t = if template.trim().is_empty() { "{{#}}" } else { template };
    let mut p = Paragraph::new(NodeId(0), "", t);
    fill(&mut p, record, None, number);
    let name: String = p
        .text
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.,()".contains(c) { c } else { '_' })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    if name.is_empty() { format!("{number}") } else { name }
}

/// Several filled-in documents as one, each starting on a new page.
pub fn combine(docs: Vec<Document>) -> Document {
    let mut iter = docs.into_iter();
    let Some(mut out) = iter.next() else { return Document::new() };
    for d in iter {
        let offset = out.next_id;
        let mut d = d;
        d.renumber(offset);
        out.next_id = d.next_id;
        for (k, v) in d.assets {
            out.assets.entry(k).or_insert(v);
        }
        for (i, mut s) in d.sections.into_iter().enumerate() {
            if i == 0 {
                s.start = SectionStart::NewPage;
            }
            out.sections.push(Section { ..s });
        }
    }
    out.revision += 1;
    out
}

impl Document {
    /// Adds `offset` to every node id (so documents can be combined).
    pub fn renumber(&mut self, offset: u64) {
        fn go(blocks: &mut [Block], o: u64) {
            for b in blocks {
                match b {
                    Block::Paragraph(p) => {
                        p.id.0 += o;
                        // Form controls (a template's fields) are numbered too.
                        for r in &mut p.runs {
                            if let Some(c) = &mut r.fmt.control {
                                c.0 += o;
                            }
                        }
                    }
                    Block::Image(i) => i.id.0 += o,
                    Block::Toc(t) => t.id.0 += o,
                    Block::Table(t) => {
                        t.id.0 += o;
                        for r in &mut t.rows {
                            r.id.0 += o;
                            for c in &mut r.cells {
                                c.id.0 += o;
                                go(&mut c.blocks, o);
                            }
                        }
                    }
                }
            }
        }
        for s in &mut self.sections {
            s.id.0 += offset;
            go(&mut s.blocks, offset);
            for f in &mut s.frames {
                f.id.0 += offset;
                f.next = f.next.map(|n| NodeId(n.0 + offset));
                go(&mut f.blocks, offset);
            }
        }
        for c in &mut self.meta.controls {
            c.id.0 += offset;
        }
        if let Some(b) = &mut self.meta.body_slot {
            b.0 += offset;
        }
        self.next_id += offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CharFmt, Pos, Selection, ops, tables};
    use serde_json::json;

    #[test]
    fn fills_fields_with_formatting_and_fallbacks() {
        let mut ed = crate::Editor::new(Document::from_plain_text(
            "Dear {{first|Friend}} {{last}},\nYou live in {{address.city}}. #{{#}} \\{{literal}}",
        ));
        let p = ed.doc.start().para;
        ed.sel = Selection { anchor: Pos::new(p, 5), focus: Pos::new(p, 24) };
        ops::apply_format(&mut ed, &CharFmt { bold: Some(true), ..Default::default() }, "b").unwrap();
        assert_eq!(fields(&ed.doc), ["first", "last", "address"]);
        let rec = json!({"first": "", "last": "Smith", "address": {"city": "Leeds"}});
        let out = instantiate(&ed.doc, &rec, 7);
        let texts: Vec<_> = out.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["Dear Friend Smith,", "You live in Leeds. #7 \\{{literal}}"]);
        // "Friend" kept the bold of the placeholder.
        let p0 = out.paragraphs().next().unwrap();
        assert_eq!(p0.runs[1].fmt.bold, Some(true));
        assert_eq!(file_name("{{last}}/{{first|x}}", &rec, 3), "Smith_x");
    }

    #[test]
    fn repeating_rows_for_line_items() {
        let mut ed = crate::Editor::new(Document::from_plain_text("Invoice {{number}}"));
        ed.sel = Selection::caret(ed.doc.end());
        tables::insert_table(&mut ed, 2, 2).unwrap();
        for w in ["Item", "Qty", "{{items.name}}", "{{.qty}}"] {
            ops::insert_text(&mut ed, w, None).unwrap();
            tables::move_cell(&mut ed, true).unwrap();
        }
        ed.undo();
        let Block::Table(t) = ed.doc.sections[0].blocks.iter_mut().find(|b| matches!(b, Block::Table(_))).unwrap()
        else {
            unreachable!()
        };
        t.rows[1].repeat = Some("items".into());
        let rec = json!({"number": "42", "items": [{"name": "Tea", "qty": 2}, {"name": "Cake", "qty": 1}]});
        let out = instantiate(&ed.doc, &rec, 1);
        let texts: Vec<_> = out.paragraphs().map(|p| p.text.clone()).filter(|t| !t.is_empty()).collect();
        assert_eq!(texts, ["Invoice 42", "Item", "Qty", "Tea", "2", "Cake", "1"]);
    }

    #[test]
    fn combining_keeps_ids_unique() {
        let doc = Document::from_plain_text("Hi {{n}}");
        let docs: Vec<_> = (1..=3).map(|i| instantiate(&doc, &json!({"n": i}), i)).collect();
        let all = combine(docs);
        assert_eq!(all.sections.len(), 3);
        let ids: std::collections::BTreeSet<_> = all.paragraphs().map(|p| p.id).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(all.plain_text(), "Hi 1\nHi 2\nHi 3");
    }
}
