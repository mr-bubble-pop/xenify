//! Reports: a printable page of blocks (titles, text, big numbers, cells or
//! a table, a summary, a chart) that follow the book. A Report sheet shows
//! the page instead of a grid; its numbers' formulas are kept in its cells
//! (one per row), so they're worked out like any other.

use crate::book::{Book, CellData};
use crate::edit::Range;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Title {
        text: String,
    },
    Text {
        text: String,
    },
    /// A big number: what it is and the formula that works it out.
    Number {
        label: String,
        formula: String,
    },
    /// Cells shown as a table: a table by name, or a range.
    Cells {
        sheet: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        table: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<Range>,
    },
    /// A Summary sheet's result.
    Summary {
        sheet: u32,
    },
    Chart {
        chart: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Def {
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub landscape: bool,
}

/// Puts each Number block's formula in the report sheet's column A (row =
/// block number), so the book works them out. Not an edit.
pub fn fill(book: &mut Book, sheet: u32, d: &Def) -> Result<(), String> {
    let i = book.index(sheet)?;
    let w = &mut book.model.workbook.worksheets[i as usize];
    let old: Vec<(i32, i32)> = w.sheet_data.cells().map(|(r, c, _)| (r, c)).collect();
    for (r, c) in old {
        w.sheet_data.remove_cell(r, c);
    }
    for (k, b) in d.blocks.iter().enumerate() {
        if let Block::Number { formula, .. } = b {
            let f = if formula.starts_with('=') { formula.clone() } else { format!("={formula}") };
            if book.put(sheet, k as i32 + 1, 1, Some(&CellData::Formula(f.clone()))).is_err() {
                book.put(sheet, k as i32 + 1, 1, Some(&CellData::Text(format!("can’t work out {f}"))))?;
            }
        }
    }
    Ok(())
}

pub fn refresh_all(book: &mut Book) {
    let defs: Vec<(u32, Def)> = book.reports.iter().map(|(k, d)| (*k, d.clone())).collect();
    for (sheet, d) in &defs {
        let _ = fill(book, *sheet, d);
    }
    if !defs.is_empty() {
        book.model.evaluate();
    }
}

/// A block's cells as rows of shown text (the first row heads the table).
fn cells_text(book: &Book, sheet: u32, r: Range) -> Vec<Vec<String>> {
    let Ok(i) = book.index(sheet) else { return Vec::new() };
    (r.top..=r.bottom.min(r.top + 300))
        .map(|row| {
            (r.left..=r.right.min(r.left + 30))
                .map(|c| book.model.get_formatted_cell_value(i, row, c).unwrap_or_default())
                .collect()
        })
        .collect()
}

/// What the page shows for each block (worked out now).
pub fn page(book: &Book, sheet: u32) -> Result<Json, String> {
    let d = book.reports.get(&sheet).ok_or("this sheet isn't a report")?;
    let i = book.index(sheet)?;
    let blocks: Vec<Json> = d
        .blocks
        .iter()
        .enumerate()
        .map(|(k, b)| match b {
            Block::Number { label, .. } => {
                let shown = book.model.get_formatted_cell_value(i, k as i32 + 1, 1).unwrap_or_default();
                json!({ "kind": "number", "label": label, "value": grouped(&shown) })
            }
            Block::Cells { sheet: s, table: Some(t), .. } => match crate::tables::get(book, t) {
                Some(td) => {
                    let r = Range { top: td.top, left: td.left, bottom: td.bottom, right: td.right };
                    json!({ "kind": "cells", "rows": cells_text(book, *s, r), "totals": td.totals, "name": t })
                }
                None => json!({ "kind": "missing", "text": format!("The table {t} isn't there any more") }),
            },
            Block::Cells { sheet: s, range: Some(r), .. } => json!({ "kind": "cells", "rows": cells_text(book, *s, *r), "totals": false }),
            Block::Cells { .. } => json!({ "kind": "missing", "text": "No cells picked" }),
            Block::Summary { sheet: s } => {
                let used = book.index(*s).ok().map(|si| book.model.workbook.worksheets[si as usize].dimension());
                match used {
                    Some(dim) => json!({ "kind": "cells", "rows": cells_text(book, *s, Range { top: 1, left: 1, bottom: dim.max_row, right: dim.max_column }), "totals": true, "name": book.sheet_name(*s).unwrap_or_default() }),
                    None => json!({ "kind": "missing", "text": "That summary isn't there any more" }),
                }
            }
            Block::Chart { chart } => match book.charts.iter().find(|c| c.id == *chart) {
                Some(_) => json!({ "kind": "chart", "chart": chart }),
                None => json!({ "kind": "missing", "text": "That chart isn't there any more" }),
            },
            Block::Title { text } => json!({ "kind": "title", "text": text }),
            Block::Text { text } => json!({ "kind": "text", "text": text }),
        })
        .collect();
    Ok(json!({ "blocks": blocks, "def": d }))
}

/// A plain number as a big number shows it: "27030" → "27,030",
/// "-1234567.5" → "-1,234,567.5". Anything else (already formatted, money,
/// a date, words) stays as it is.
pub fn grouped(shown: &str) -> String {
    let (sign, rest) = match shown.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", shown),
    };
    let (whole, frac) = match rest.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (rest, None),
    };
    let plain = !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()));
    if !plain || whole.len() <= 3 {
        return shown.to_string();
    }
    let mut out = String::new();
    for (k, c) in whole.chars().enumerate() {
        if k > 0 && (whole.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}{}", frac.map(|f| format!(".{f}")).unwrap_or_default())
}

/// A ready report from table `t`: a title, the total of each column of
/// numbers (the first three), and the table itself.
pub fn from_table(book: &Book, t: &crate::tables::TableDef, title: &str) -> Def {
    let mut blocks = vec![Block::Title { text: title.to_string() }];
    let numbers: Vec<&crate::tables::TableCol> = t
        .columns
        .iter()
        .enumerate()
        .filter(|(k, _)| crate::tables::numbers_in(book, t, t.left + *k as i32))
        .map(|(_, c)| c)
        .take(3)
        .collect();
    for c in numbers {
        blocks.push(Block::Number {
            label: format!("Total {}", c.name.to_lowercase()),
            formula: format!("SUM({}[{}])", t.name, crate::tables::escape_column(&c.name)),
        });
    }
    blocks.push(Block::Cells { sheet: t.sheet, table: Some(t.name.clone()), range: None });
    Def { blocks, landscape: false }
}
