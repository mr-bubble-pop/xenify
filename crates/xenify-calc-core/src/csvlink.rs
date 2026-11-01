//! Linked CSV sheets: a sheet whose values come from a CSV (or other
//! separated text) file and follow it. The book keeps where the file is (a
//! path on the computer, or a file on a Xenify server), how it's read (the
//! separator, and whether values become numbers and dates), and when it was
//! last read. The app reads the file (when the book opens, when the file
//! changes, with Update now) and hands its text to `fill`, which writes it
//! as an edit, so formulas on other sheets recalculate and Undo puts the
//! last values back. The sheet can't be typed in; Unlink makes it a normal
//! sheet with the values it has. Excel files keep only the values.

use crate::book::{Book, CellData};
use crate::csv::{self, Values};
use crate::ops::{CellEdit, Op};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

/// Where a linked sheet's file is and how it's read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Def {
    /// On the computer: the file's path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// On a Xenify server: its address and the file's id there.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<i64>,
    /// The file's name, as shown.
    pub name: String,
    /// The separator; empty: the likeliest one, found each time it's read.
    pub sep: String,
    pub values: Values,
    /// When it was last read (the app's clock, ISO 8601), and its size then
    /// in rows and columns.
    pub read: String,
    pub rows: usize,
    pub columns: usize,
}

impl Def {
    fn separator(&self, text: &str) -> char {
        self.sep.chars().next().unwrap_or_else(|| csv::separator(text))
    }
}

/// The edits that put the file's `text` on linked sheet `sheet` in place of
/// what's there, and record `def` with how it went. Columns still at the
/// normal width are sized to fit (a width set since stays).
pub fn fill(book: &Book, sheet: u32, mut def: Def, text: &str) -> Result<Vec<Op>, String> {
    let i = book.index(sheet)? as usize;
    let rows = csv::parse(text, def.separator(text));
    let mut cells = Vec::new();
    let mut written = std::collections::HashSet::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let at = (r as i32 + 1, c as i32 + 1);
            written.insert(at);
            cells.push(CellEdit { row: at.0, col: at.1, data: Some(csv::cell_of(v, def.values)) });
        }
    }
    // What the file no longer has goes (the sheet holds only the file).
    for (r, c, _) in book.model.workbook.worksheets[i].sheet_data.cells() {
        if !written.contains(&(r, c)) && book.cell_data(sheet, r, c).is_some() {
            cells.push(CellEdit { row: r, col: c, data: None });
        }
    }
    let mut ops = vec![Op::Cells { sheet, cells }];
    let columns: Vec<_> = csv::fit_widths(&rows)
        .into_iter()
        .filter(|c| crate::ops::column_size(book, sheet, c.at).size.is_none())
        .collect();
    if !columns.is_empty() {
        ops.push(Op::Columns { sheet, columns });
    }
    def.rows = rows.len();
    def.columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    ops.push(Op::CsvLink { sheet, def: Some(def) });
    Ok(ops)
}

/// What a sheet's cells hold (to tell whether reading the file changed any).
pub(crate) fn values(book: &Book, sheet: u32) -> Vec<(i32, i32, Option<CellData>)> {
    book.cells_in(sheet, None, None).into_iter().map(|(r, c, d, _)| (r, c, d)).filter(|x| x.2.is_some()).collect()
}

/// Every linked sheet: its id, name and link (to read them all).
pub fn all(book: &Book) -> Vec<Json> {
    book.csv_links
        .iter()
        .filter_map(|(id, d)| book.sheet_name(*id).ok().map(|name| json!({ "sheet": id, "name": name, "def": d })))
        .collect()
}
