//! Tables: a block of rows with a header row and, often, a totals row.
//! Totals use the table's columns (`=SUBTOTAL(109,Sales[Revenue])`), so
//! they keep covering every row as the table grows: adding a row above the
//! totals row, typing just under a table without one, or a database
//! refresh that brings more rows.
//!
//! Making one from what's on the sheet: click inside a block, and Calc
//! finds where it starts and ends and whether its first row is a header.

use crate::book::{Book, CellData};
use crate::edit::Range;
use crate::ops::{CellEdit, Op, StyleEdit};
use ironcalc_base::types::{Table, TableColumn, TableStyleInfo};
use serde::{Deserialize, Serialize};

/// A table as edits carry it (IronCalc's `Table` by sheet id and numbers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableDef {
    pub name: String,
    pub sheet: u32,
    /// First and last row (header and totals included) and column.
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
    pub header: bool,
    pub totals: bool,
    pub columns: Vec<TableCol>,
    #[serde(default)]
    pub stripes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableCol {
    pub name: String,
    /// What the totals row shows under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<Total>,
}

/// The choices in a totals cell's picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Total {
    Sum,
    Average,
    Count,
    Min,
    Max,
    Median,
}

impl Total {
    /// The totals cell's formula for column `column` of table `table`.
    /// SUBTOTAL leaves out rows hidden by a filter.
    pub fn formula(self, table: &str, column: &str) -> String {
        let r = format!("{table}[{}]", escape_column(column));
        match self {
            Total::Sum => format!("=SUBTOTAL(109,{r})"),
            Total::Average => format!("=SUBTOTAL(101,{r})"),
            Total::Count => format!("=SUBTOTAL(103,{r})"),
            Total::Min => format!("=SUBTOTAL(105,{r})"),
            Total::Max => format!("=SUBTOTAL(104,{r})"),
            Total::Median => format!("=MEDIAN({r})"),
        }
    }

    /// xlsx's name for it (`totalsRowFunction`).
    fn xlsx(self) -> &'static str {
        match self {
            Total::Sum => "sum",
            Total::Average => "average",
            Total::Count => "count",
            Total::Min => "min",
            Total::Max => "max",
            Total::Median => "custom",
        }
    }

    fn from_xlsx(s: &str, formula: Option<&str>) -> Option<Total> {
        Some(match s {
            "sum" => Total::Sum,
            "average" => Total::Average,
            "count" | "countNums" => Total::Count,
            "min" => Total::Min,
            "max" => Total::Max,
            "custom" if formula.is_some_and(|f| f.to_uppercase().contains("MEDIAN(")) => Total::Median,
            _ => return None,
        })
    }
}

pub fn escape_column(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if matches!(ch, '\'' | '[' | ']' | '#' | '@') {
            out.push('\'');
        }
        out.push(ch);
    }
    out
}

pub fn column_letters(mut c: i32) -> String {
    let mut s = Vec::new();
    while c > 0 {
        let r = ((c - 1) % 26) as u8;
        s.push(b'A' + r);
        c = (c - 1) / 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// "B12" (or "$B$12") → (12, 2).
pub fn parse_cell_pub(s: &str) -> Option<(i32, i32)> {
    parse_cell(&s.replace('$', "")).filter(|(r, c)| *r > 0 && *c > 0)
}

fn parse_cell(s: &str) -> Option<(i32, i32)> {
    let letters: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let digits = &s[letters.len()..];
    let mut c = 0;
    for ch in letters.to_ascii_uppercase().chars() {
        c = c * 26 + (ch as i32 - 'A' as i32 + 1);
    }
    Some((digits.parse().ok()?, c))
}

fn to_ironcalc(book: &Book, t: &TableDef) -> Result<Table, String> {
    let reference = format!("{}{}:{}{}", column_letters(t.left), t.top, column_letters(t.right), t.bottom);
    let columns = t
        .columns
        .iter()
        .enumerate()
        .map(|(k, c)| TableColumn {
            id: k as u32 + 1,
            name: c.name.clone(),
            totals_row_function: c.total.map(|f| f.xlsx().to_string()),
            ..Default::default()
        })
        .collect();
    Ok(Table {
        name: t.name.clone(),
        display_name: t.name.clone(),
        sheet_name: book.sheet_name(t.sheet)?,
        reference,
        totals_row_count: t.totals as u32,
        header_row_count: t.header as u32,
        header_row_dxf_id: None,
        data_dxf_id: None,
        totals_row_dxf_id: None,
        columns,
        style_info: TableStyleInfo {
            name: Some("TableStyleLight1".to_string()),
            show_row_stripes: t.stripes,
            ..Default::default()
        },
        // Rows hidden in a table with a filter on count as filtered (SUBTOTAL leaves them out).
        has_filters: book.filters.get(&t.name).is_some_and(|f| !f.is_empty()),
    })
}

fn from_ironcalc(book: &Book, t: &Table) -> Option<TableDef> {
    let (a, b) = t.reference.split_once(':')?;
    let (top, left) = parse_cell(a)?;
    let (bottom, right) = parse_cell(b)?;
    let index = book.model.workbook.worksheets.iter().position(|w| w.get_name() == t.sheet_name)?;
    let sheet = book.model.workbook.worksheets[index].sheet_id;
    let totals = t.totals_row_count > 0;
    let columns = t
        .columns
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let formula = match book.cell_data(sheet, bottom, left + k as i32) {
                Some(CellData::Formula(f)) if totals => Some(f),
                _ => None,
            };
            TableCol {
                name: c.name.clone(),
                total: c.totals_row_function.as_deref().and_then(|f| Total::from_xlsx(f, formula.as_deref())),
            }
        })
        .collect();
    Some(TableDef {
        name: t.name.clone(),
        sheet,
        top,
        left,
        bottom,
        right,
        header: t.header_row_count > 0,
        totals,
        columns,
        stripes: t.style_info.show_row_stripes,
    })
}

/// The table called `name` (case doesn't matter, as in formulas).
pub fn get(book: &Book, name: &str) -> Option<TableDef> {
    let t = book.model.workbook.tables.values().find(|t| t.name.eq_ignore_ascii_case(name))?;
    from_ironcalc(book, t)
}

/// Every table, in name order.
pub fn all(book: &Book) -> Vec<TableDef> {
    let mut v: Vec<TableDef> = book.model.workbook.tables.values().filter_map(|t| from_ironcalc(book, t)).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// The table cell (`row`, `column`) of sheet `sheet` is in.
pub fn at(book: &Book, sheet: u32, row: i32, column: i32) -> Option<TableDef> {
    all(book)
        .into_iter()
        .find(|t| t.sheet == sheet && (t.top..=t.bottom).contains(&row) && (t.left..=t.right).contains(&column))
}

/// Puts (or with none, takes away) table `name`. The caller works out the
/// formulas again (`Model::refresh_tables`).
pub(crate) fn set(book: &mut Book, name: &str, table: Option<&TableDef>) -> Result<(), String> {
    let key = book.model.workbook.tables.keys().find(|k| k.eq_ignore_ascii_case(name)).cloned();
    if let Some(k) = key {
        book.model.workbook.tables.remove(&k);
    }
    if let Some(t) = table {
        if !valid_name(&t.name) {
            return Err(format!("“{}” can't be a table name: use letters, digits and _", t.name));
        }
        let it = to_ironcalc(book, t)?;
        book.model.workbook.tables.insert(t.name.clone(), it);
    }
    Ok(())
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        // Not something that reads as a cell (A1, XFD10) or R1C1.
        && parse_cell(name).is_none_or(|(r, c)| r == 0 || c == 0 || c > 16384)
        && !matches!(name.to_ascii_uppercase().as_str(), "R" | "C")
        && !is_r1c1(name)
}

/// R1, C3, R2C4: names formulas would read as references.
fn is_r1c1(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    let rest = |s: &str| s.is_empty() || s.chars().all(|c| c.is_ascii_digit());
    match u.strip_prefix('R') {
        Some(r) => match r.split_once('C') {
            Some((a, b)) => rest(a) && rest(b),
            None => !r.is_empty() && rest(r),
        },
        None => u.strip_prefix('C').is_some_and(|c| !c.is_empty() && rest(c)),
    }
}

/// A table name from `wanted` that isn't taken: "Sales", "Sales2"…
pub fn free_name(book: &Book, wanted: &str) -> String {
    let mut base: String = wanted.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
    if !base.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') || !valid_name(&base) {
        base = "Table".to_string();
    }
    let taken = |n: &str| {
        book.model.workbook.tables.keys().any(|k| k.eq_ignore_ascii_case(n))
            || book.model.workbook.defined_names.iter().any(|d| d.name.eq_ignore_ascii_case(n))
    };
    if !taken(&base) && base != "Table" {
        return base;
    }
    (if base == "Table" { 1 } else { 2 }..).map(|k| format!("{base}{k}")).find(|n| !taken(n)).unwrap_or(base)
}

/// What a column of cells without a header row is called: "Column B".
pub fn letter_name(col: i32) -> String {
    format!("Column {}", column_letters(col))
}

/// Whether column `col` of table `t` holds numbers (in its first few rows).
pub fn numbers_in(book: &Book, t: &TableDef, col: i32) -> bool {
    let first = t.top + t.header as i32;
    let last = (t.bottom - t.totals as i32).min(first + 5);
    (first..=last).any(|r| kind(book, t.sheet, r, col) == Kind::Number)
}

/// What a cell looks like to the header guess.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Empty,
    Text,
    Number,
    Other,
}

fn kind(book: &Book, sheet: u32, row: i32, column: i32) -> Kind {
    match book.cell_data(sheet, row, column) {
        None => Kind::Empty,
        Some(CellData::Text(s)) if s.trim().is_empty() => Kind::Empty,
        Some(CellData::Text(_)) => Kind::Text,
        Some(CellData::Number(_)) => Kind::Number,
        Some(CellData::Formula(_)) => {
            let i = book.index(sheet).unwrap_or(0);
            match book.model.get_cell_value_by_index(i, row, column) {
                Ok(ironcalc_base::cell::CellValue::Number(_)) => Kind::Number,
                Ok(ironcalc_base::cell::CellValue::String(_)) => Kind::Text,
                _ => Kind::Other,
            }
        }
        Some(_) => Kind::Other,
    }
}

/// The block around (`row`, `column`): rows and columns joined to it by
/// something in them, as Ctrl+A finds it.
pub fn block(book: &Book, sheet: u32, row: i32, column: i32) -> (i32, i32, i32, i32) {
    let filled = |r: i32, c: i32| r >= 1 && c >= 1 && kind(book, sheet, r, c) != Kind::Empty;
    let (mut top, mut left, mut bottom, mut right) = (row, column, row, column);
    loop {
        let mut grew = false;
        if (left - 1..=right + 1).any(|c| filled(top - 1, c)) {
            top -= 1;
            grew = true;
        }
        if (left - 1..=right + 1).any(|c| filled(bottom + 1, c)) {
            bottom += 1;
            grew = true;
        }
        if (top..=bottom).any(|r| filled(r, left - 1)) {
            left -= 1;
            grew = true;
        }
        if (top..=bottom).any(|r| filled(r, right + 1)) {
            right += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    (top, left, bottom, right)
}

/// Whether the first row of a block is a header: words in every cell, all
/// different, over a row with at least one cell that isn't words (or a
/// first row in bold over one that isn't).
pub fn has_header(book: &Book, sheet: u32, top: i32, left: i32, bottom: i32, right: i32) -> bool {
    if bottom <= top {
        return false;
    }
    let first: Vec<Kind> = (left..=right).map(|c| kind(book, sheet, top, c)).collect();
    if first.iter().any(|k| *k != Kind::Text) {
        return false;
    }
    let names: Vec<String> = (left..=right)
        .map(|c| match book.cell_data(sheet, top, c) {
            Some(CellData::Text(s)) => s.trim().to_lowercase(),
            _ => String::new(),
        })
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != names.len() {
        return false;
    }
    let below = (top + 1..=bottom.min(top + 5))
        .any(|r| (left..=right).any(|c| matches!(kind(book, sheet, r, c), Kind::Number | Kind::Other)));
    let bold = |r: i32| (left..=right).all(|c| book.style(sheet, r, c).font.b);
    below || (bold(top) && !bold(top + 1))
}

/// The ops that make a table of `range` (or, with none, of the block around
/// (`row`, `column`)). Nothing moves: without a header row the table has
/// none, and its columns are called by their letters ("Column B").
pub fn make(
    book: &Book,
    sheet: u32,
    row: i32,
    column: i32,
    range: Option<Range>,
) -> Result<(TableDef, Vec<Op>), String> {
    if at(book, sheet, row, column).is_some() {
        return Err("This is already in a table".to_string());
    }
    let (top, left, bottom, right) = match range {
        Some(r) if r.bottom > r.top || r.right > r.left => (r.top, r.left, r.bottom, r.right),
        _ => {
            if kind(book, sheet, row, column) == Kind::Empty {
                return Err("Select the cells to make into a table first (this cell is empty)".to_string());
            }
            block(book, sheet, row, column)
        }
    };
    if (top..=bottom).all(|r| (left..=right).all(|c| kind(book, sheet, r, c) == Kind::Empty)) {
        return Err("There's nothing in these cells to make into a table".to_string());
    }
    if let Some(t) = all(book)
        .into_iter()
        .find(|t| t.sheet == sheet && t.top <= bottom && t.bottom >= top && t.left <= right && t.right >= left)
    {
        return Err(format!("These cells overlap the table {}", t.name));
    }
    let header = has_header(book, sheet, top, left, bottom, right);
    let names: Vec<String> = if header {
        (left..=right)
            .map(|c| match book.cell_data(sheet, top, c) {
                Some(CellData::Text(s)) => s.trim().to_string(),
                _ => String::new(),
            })
            .collect()
    } else {
        (left..=right).map(letter_name).collect()
    };
    let base = book.sheet_name(sheet).unwrap_or_default();
    let def = TableDef {
        name: free_name(book, &base),
        sheet,
        top,
        left,
        bottom: if header { bottom.max(top + 1) } else { bottom },
        right,
        header,
        totals: false,
        columns: names.into_iter().map(|name| TableCol { name, total: None }).collect(),
        stripes: true,
    };
    Ok((def.clone(), vec![Op::Table { name: def.name.clone(), table: Some(def) }]))
}

/// "months" from a column called "Month": what the row count is of.
fn counted(column: &str) -> String {
    let w = column.trim().to_lowercase();
    // A column without a header ("Column B") counts rows.
    let letters = w.strip_prefix("column ").is_some_and(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_lowercase()));
    if w.is_empty() || letters {
        "rows".to_string()
    } else if w.ends_with('s') {
        w
    } else if w.ends_with('y') && !w.ends_with("ay") && !w.ends_with("ey") && !w.ends_with("oy") {
        format!("{}ies", &w[..w.len() - 1])
    } else {
        format!("{w}s")
    }
}

/// The formula that counts the rows, for the first column of a totals row.
pub fn count_label(table: &str, column: &str) -> String {
    format!("=SUBTOTAL(103,{table}[{}])&\" {}\"", escape_column(column), counted(column).replace('"', "\"\""))
}

/// The ops that show `total` (none: nothing) under `column` of table
/// `name`, adding the totals row first when there isn't one. A new totals
/// row counts the rows in its first column.
pub fn set_total(book: &Book, name: &str, column: &str, total: Option<Total>) -> Result<Vec<Op>, String> {
    let mut t = get(book, name).ok_or_else(|| format!("no table {name}"))?;
    let k = t
        .columns
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(column))
        .ok_or_else(|| format!("no column {column} in {name}"))?;
    let mut ops = Vec::new();
    let mut cells = Vec::new();
    if !t.totals {
        // The row under the table, made free if something is there.
        let below = t.bottom + 1;
        if (t.left..=t.right).any(|c| book.cell_data(t.sheet, below, c).is_some()) {
            ops.push(Op::InsertRows { sheet: t.sheet, at: below, count: 1 });
        }
        t.bottom = below;
        t.totals = true;
        if k != 0 {
            let first = t.columns[0].name.clone();
            cells.push(CellEdit {
                row: below,
                col: t.left,
                data: Some(CellData::Formula(count_label(&t.name, &first))),
            });
        }
    }
    t.columns[k].total = total;
    // The table first: the formulas below name its totals row.
    ops.push(Op::Table { name: t.name.clone(), table: Some(t.clone()) });
    let col = t.left + k as i32;
    let data = match total {
        Some(f) => Some(CellData::Formula(f.formula(&t.name, &t.columns[k].name))),
        None if k == 0 => Some(CellData::Formula(count_label(&t.name, &t.columns[0].name))),
        None => None,
    };
    cells.push(CellEdit { row: t.bottom, col, data });
    // The total shows numbers the way the column does (a count as a whole number).
    let last = t.bottom - 1;
    if last >= t.top + t.header as i32 && total.is_some() {
        let mut style = book.style(t.sheet, t.bottom, col);
        style.num_fmt = match total {
            Some(Total::Count) => "general".to_string(),
            _ => book.style(t.sheet, last, col).num_fmt,
        };
        ops.push(Op::Styles { sheet: t.sheet, cells: vec![StyleEdit { row: t.bottom, col, style }] });
    }
    ops.push(Op::Cells { sheet: t.sheet, cells });
    Ok(ops)
}

/// The ops that add a row at the end of table `name` (above its totals
/// row) with `values` by column name. Columns that work something out the
/// same way in every row get the same formula in the new row.
pub fn add_row(book: &Book, name: &str, values: &[(String, CellData)]) -> Result<Vec<Op>, String> {
    let t = get(book, name).ok_or_else(|| format!("no table {name}"))?;
    let data_top = t.top + t.header as i32;
    let last = t.bottom - t.totals as i32;
    let new_row = last + 1;
    let mut ops = Vec::new();
    if t.totals {
        // Inserting at the totals row is inside the table: it grows by one.
        ops.push(Op::InsertRows { sheet: t.sheet, at: t.bottom, count: 1 });
    } else {
        if (t.left..=t.right).any(|c| book.cell_data(t.sheet, new_row, c).is_some()) {
            ops.push(Op::InsertRows { sheet: t.sheet, at: new_row, count: 1 });
        }
        let mut grown = t.clone();
        grown.bottom = new_row;
        ops.push(Op::Table { name: t.name.clone(), table: Some(grown) });
    }
    let mut cells = Vec::new();
    // A new row looks like the one above it.
    if last >= data_top {
        let styles: Vec<StyleEdit> = (t.left..=t.right)
            .map(|col| StyleEdit { row: new_row, col, style: book.style(t.sheet, last, col) })
            .filter(|s| s.style != ironcalc_base::types::Style::default())
            .collect();
        if !styles.is_empty() {
            ops.push(Op::Styles { sheet: t.sheet, cells: styles });
        }
    }
    for (k, c) in t.columns.iter().enumerate() {
        let col = t.left + k as i32;
        if let Some((_, v)) = values.iter().find(|(n, _)| n.eq_ignore_ascii_case(&c.name)) {
            cells.push(CellEdit { row: new_row, col, data: Some(v.clone()) });
        } else if last >= data_top
            && let Some(f) = calculated_column(book, t.sheet, data_top, last, col, new_row)
        {
            cells.push(CellEdit { row: new_row, col, data: Some(CellData::Formula(f)) });
        }
    }
    if !cells.is_empty() {
        ops.push(Op::Cells { sheet: t.sheet, cells });
    }
    Ok(ops)
}

/// When every data row of column `col` holds the same formula (the same
/// once moved to its row), that formula moved to `to_row`.
fn calculated_column(book: &Book, sheet: u32, from: i32, to: i32, col: i32, to_row: i32) -> Option<String> {
    let i = book.index(sheet).ok()?;
    let w = &book.model.workbook.worksheets[i as usize];
    let mut shared = None;
    for r in from..=to {
        match w.cell(r, col)?.as_ref() {
            ironcalc_base::types::Cell::CellFormula { f, .. } => match shared {
                None => shared = Some(*f),
                Some(s) if s == *f => {}
                _ => return None,
            },
            _ => return None,
        }
    }
    shared?;
    book.model.extend_to(i, to, col, to_row, col).ok()
}

/// When typing at (`row`, `column`) lands just under a table that has no
/// totals row, the ops that make the table take that row in.
pub fn grow_for(book: &Book, sheet: u32, row: i32, column: i32) -> Option<Op> {
    let t = all(book)
        .into_iter()
        .find(|t| t.sheet == sheet && !t.totals && t.bottom + 1 == row && (t.left..=t.right).contains(&column))?;
    let mut grown = t.clone();
    grown.bottom = row;
    Some(Op::Table { name: t.name, table: Some(grown) })
}
