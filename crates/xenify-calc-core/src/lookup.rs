//! Look up by example. With the cursor where the answers go:
//! 1. pick the table to look in (a click inside it takes all of it),
//! 2. click the column to match on: Calc finds the column of your rows
//!    whose values are in it (the one with the most matches),
//! 3. click the value you'd expect in any row: its column comes back.
//!
//! The column is then filled with XLOOKUP formulas (shown, editable). When
//! the table looked in is a table, its column names are used, so it keeps
//! working as rows are added there.

use crate::book::{Book, CellData};
use crate::edit::Range;
use crate::ops::{CellEdit, Op};
use crate::tables::{self, TableCol, TableDef};
use ironcalc_base::cell::CellValue;
use serde::{Deserialize, Serialize};

/// Rows of cells with a header row: a table, or a block of filled cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rows {
    pub sheet: u32,
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
    /// The first row names the columns.
    pub header: bool,
    /// The table it is, when it is one.
    #[serde(default)]
    pub table: Option<String>,
}

impl Rows {
    pub fn data_top(&self) -> i32 {
        self.top + self.header as i32
    }

    /// The last row with data (a table's totals row isn't).
    fn data_bottom(&self, book: &Book) -> i32 {
        match self.table.as_deref().and_then(|t| tables::get(book, t)) {
            Some(t) if t.totals => self.bottom - 1,
            _ => self.bottom,
        }
    }
}

/// The rows around (`row`, `col`): its table, or the block of filled cells.
pub fn rows_at(book: &Book, sheet: u32, row: i32, col: i32) -> Option<Rows> {
    if let Some(t) = tables::at(book, sheet, row, col) {
        return Some(Rows {
            sheet,
            top: t.top,
            left: t.left,
            bottom: t.bottom,
            right: t.right,
            header: t.header,
            table: Some(t.name),
        });
    }
    book.cell_data(sheet, row, col)?;
    let (top, left, bottom, right) = tables::block(book, sheet, row, col);
    Some(Rows {
        sheet,
        top,
        left,
        bottom,
        right,
        header: tables::has_header(book, sheet, top, left, bottom, right),
        table: None,
    })
}

/// Where answers in column `col` go beside: the rows of the table or block
/// that column is in, or the one just left of it.
pub fn your_rows(book: &Book, sheet: u32, row: i32, col: i32) -> Option<Rows> {
    if let Some(r) = rows_at(book, sheet, row, col).filter(|r| r.right > r.left || r.left != col) {
        return Some(r);
    }
    if col > 1 {
        // Find a filled cell in the column to the left, near this row.
        for r in [row, row - 1, row + 1, 1, 2] {
            if r >= 1 && (book.cell_data(sheet, r, col - 1).is_some() || tables::at(book, sheet, r, col - 1).is_some())
            {
                return rows_at(book, sheet, r, col - 1);
            }
        }
    }
    None
}

/// The whole table to look in for a click at (`row`, `col`) of `sheet`, or
/// when nothing was clicked, the one table on that sheet (or what's in use).
pub fn look_in(book: &Book, sheet: u32, at: Option<(i32, i32)>) -> Option<Rows> {
    if let Some((row, col)) = at {
        return rows_at(book, sheet, row, col);
    }
    let on_sheet: Vec<TableDef> = tables::all(book).into_iter().filter(|t| t.sheet == sheet).collect();
    if let [t] = on_sheet.as_slice() {
        return Some(Rows {
            sheet,
            top: t.top,
            left: t.left,
            bottom: t.bottom,
            right: t.right,
            header: t.header,
            table: Some(t.name.clone()),
        });
    }
    rows_at(book, sheet, 1, 1)
}

/// A value as XLOOKUP matches it: numbers by value, words ignoring case.
#[derive(PartialEq)]
enum Key {
    Number(f64),
    Text(String),
    Bool(bool),
}

fn key(book: &Book, sheet: u32, row: i32, col: i32) -> Option<Key> {
    let i = book.index(sheet).ok()?;
    match book.model.get_cell_value_by_index(i, row, col).ok()? {
        CellValue::Number(n) => Some(Key::Number(n)),
        CellValue::String(s) if !s.is_empty() => Some(Key::Text(s.to_lowercase())),
        CellValue::Boolean(b) => Some(Key::Bool(b)),
        _ => None,
    }
}

fn keys(book: &Book, rows: &Rows, col: i32) -> Vec<Option<Key>> {
    (rows.data_top()..=rows.data_bottom(book)).map(|r| key(book, rows.sheet, r, col)).collect()
}

/// A column of your rows and how many of its values are in the column matched on.
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub col: i32,
    pub name: String,
    pub matches: usize,
    pub rows: usize,
}

fn header_name(book: &Book, rows: &Rows, col: i32) -> String {
    if rows.header
        && let Some(CellData::Text(s)) = book.cell_data(rows.sheet, rows.top, col)
    {
        return s;
    }
    crate::tables::column_letters(col)
}

/// Your columns that could be matched with column `on` of `look`, best first.
pub fn candidates(book: &Book, yours: &Rows, look: &Rows, on: i32) -> Vec<Candidate> {
    let wanted = keys(book, look, on);
    let mut out: Vec<Candidate> = (yours.left..=yours.right)
        .map(|col| {
            let mine = keys(book, yours, col);
            let filled = mine.iter().filter(|k| k.is_some()).count();
            let matches = mine.iter().flatten().filter(|k| wanted.iter().flatten().any(|w| w == *k)).count();
            Candidate { col, name: header_name(book, yours, col), matches, rows: filled }
        })
        .filter(|c| c.rows > 0)
        .collect();
    out.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.col.cmp(&b.col)));
    out
}

/// Everything a look-up needs.
#[derive(Clone, Debug, Deserialize)]
pub struct Plan {
    pub yours: Rows,
    /// The column of your rows to match.
    pub key: i32,
    pub look: Rows,
    /// The column of the table looked in that's matched…
    pub on: i32,
    /// …and the one that comes back.
    pub ret: i32,
    /// The column the answers go in.
    pub target: i32,
    /// What shows when there's no match ("" leaves the cell empty).
    #[serde(default)]
    pub missing: String,
}

/// What the first `n` rows will show: (the value matched, the answer or none).
pub fn preview(book: &Book, p: &Plan, n: usize) -> Vec<(String, Option<String>)> {
    let Ok(yi) = book.index(p.yours.sheet) else { return Vec::new() };
    let Ok(li) = book.index(p.look.sheet) else { return Vec::new() };
    let wanted = keys(book, &p.look, p.on);
    let mut out = Vec::new();
    for row in p.yours.data_top()..=p.yours.data_bottom(book) {
        if out.len() >= n {
            break;
        }
        let shown = book.model.get_formatted_cell_value(yi, row, p.key).unwrap_or_default();
        let found =
            key(book, p.yours.sheet, row, p.key).and_then(|k| wanted.iter().position(|w| w.as_ref() == Some(&k)));
        let answer = found
            .map(|k| book.model.get_formatted_cell_value(li, p.look.data_top() + k as i32, p.ret).unwrap_or_default());
        out.push((shown, answer));
    }
    out
}

fn quote_sheet(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// The formula for row `row`.
pub fn formula(book: &Book, p: &Plan, row: i32) -> Result<String, String> {
    let me = format!("{}{row}", tables::column_letters(p.key));
    let missing = format!("\"{}\"", p.missing.replace('"', "\"\""));
    let (on, ret) = match p.look.table.as_deref().and_then(|t| tables::get(book, t)) {
        Some(t) => {
            // (The table may have changed since the columns were picked.)
            let name = |c: i32| {
                let col = usize::try_from(c - t.left).ok().and_then(|k| t.columns.get(k));
                let col = col.ok_or_else(|| format!("that column isn't in {} any more", t.name))?;
                Ok::<_, String>(format!("{}[{}]", t.name, crate::tables::escape_column(&col.name)))
            };
            (name(p.on)?, name(p.ret)?)
        }
        None => {
            let sheet = if p.look.sheet == p.yours.sheet {
                String::new()
            } else {
                format!("{}!", quote_sheet(&book.sheet_name(p.look.sheet)?))
            };
            let col = |c: i32| {
                let l = tables::column_letters(c);
                format!("{sheet}${l}${}:${l}${}", p.look.data_top(), p.look.data_bottom(book))
            };
            (col(p.on), col(p.ret))
        }
    };
    Ok(format!("=XLOOKUP({me},{on},{ret},{missing})"))
}

/// The ops that fill the answers column, name it after the column that
/// comes back (when its header is empty), and take it into your table when
/// it's just beside it.
pub fn fill(book: &Book, p: &Plan) -> Result<Vec<Op>, String> {
    let sheet = p.yours.sheet;
    let mut ops = Vec::new();
    let title = header_name(book, &p.look, p.ret);
    if let Some(t) = p.yours.table.as_deref().and_then(|t| tables::get(book, t)) {
        if p.target == t.right + 1 {
            let mut grown = t.clone();
            grown.right += 1;
            let mut name = title.clone();
            let mut k = 2;
            while grown.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
                name = format!("{title}{k}");
                k += 1;
            }
            grown.columns.push(TableCol { name: name.clone(), total: None });
            ops.push(Op::Cells {
                sheet,
                cells: vec![CellEdit { row: t.top, col: p.target, data: Some(CellData::Text(name)) }],
            });
            ops.push(Op::Table { name: t.name.clone(), table: Some(grown) });
        }
    } else if p.yours.header && book.cell_data(sheet, p.yours.top, p.target).is_none() {
        ops.push(Op::Cells {
            sheet,
            cells: vec![CellEdit { row: p.yours.top, col: p.target, data: Some(CellData::Text(title)) }],
        });
    }
    let mut cells = Vec::new();
    for row in p.yours.data_top()..=p.yours.data_bottom(book) {
        cells.push(CellEdit { row, col: p.target, data: Some(CellData::Formula(formula(book, p, row)?)) });
    }
    ops.push(Op::Cells { sheet, cells });
    Ok(ops)
}

/// The range a `Rows` covers.
pub fn range_of(r: &Rows) -> Range {
    Range { top: r.top, left: r.left, bottom: r.bottom, right: r.right }
}
