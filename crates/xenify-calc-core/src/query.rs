//! Query sheets: what a query on a server data source gave, in a table on
//! a sheet of its own that can't be typed in. The book keeps the query (a
//! saved one by name, or the SQL) and the values its `:names` take, from
//! cells or typed in; the app runs it on the server and hands the answer to
//! `fill`, which writes it as an edit (so Undo puts the last answer back).
//! Formulas elsewhere use its table (`=SUM(Orders[Total])`), which grows and
//! shrinks with each refresh.

use crate::book::{Book, CellData};
use crate::ops::{CellEdit, Op};
use crate::tables::{self, TableCol, TableDef};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

/// What a query sheet shows and where it comes from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Def {
    /// The server (its address), and the data source there.
    pub server: String,
    pub source: i64,
    pub source_name: String,
    /// A saved query on the server…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<i64>,
    pub query_name: String,
    /// …or the person's own SQL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    /// What each `:name` in the query is.
    pub params: BTreeMap<String, Param>,
    /// The table the answer goes in.
    pub table: String,
    /// When it last came (as the app's clock said, ISO 8601), how many rows,
    /// and whether the source's row limit left some out.
    pub refreshed: String,
    pub rows: usize,
    pub truncated: bool,
    /// When it loads new values: "open" (when the file opens, and with
    /// Refresh), "refresh" (only with Refresh) or "every15" (also every 15
    /// minutes while open).
    pub load: String,
    /// Notes the server keeps for its columns (for Ask).
    #[serde(skip_serializing_if = "Json::is_null")]
    pub notes: Json,
    /// The server's ticket for this query on this source: whoever has the
    /// file may load new values with it (it fits no other query).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ticket: String,
}

/// A `:name`'s value: from a cell (`Inputs!B2`), or as typed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Param {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<String>,
    #[serde(skip_serializing_if = "Json::is_null")]
    pub value: Json,
}

/// What the server answered (see xenify-server's datasources::Answer).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Answer {
    pub columns: Vec<(String, String)>,
    pub rows: Vec<Vec<Json>>,
    pub truncated: bool,
    pub notes: Json,
    /// How many decimals each column's numbers have, when the database
    /// says (numeric(10,2), money); none for the rest.
    pub decimals: Vec<Option<u32>>,
}

/// A number format showing `d` decimals (#,##0.00 for 2).
fn decimals_format(d: u32) -> String {
    format!("#,##0.{}", "0".repeat(d as usize))
}

/// A value as it will show (near enough, to size its column by).
fn shown(v: &Json, decimals: Option<u32>) -> String {
    match (v, decimals) {
        (Json::Null, _) => String::new(),
        (Json::String(s), _) => s.clone(),
        (Json::Bool(b), _) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        (Json::Number(n), Some(d)) => {
            let f = n.as_f64().unwrap_or(0.0);
            let digits = format!("{:.*}", d as usize, f.abs());
            let whole = digits.split('.').next().unwrap_or("").len();
            // Its thousands separators, and the minus sign.
            "0".repeat(digits.len() + whole.saturating_sub(1) / 3 + usize::from(f < 0.0))
        }
        (other, _) => other.to_string(),
    }
}

/// After a query or table editor sheet is filled (its header `names` in row
/// 1, `answer` under it): columns still at the normal width sized to fit,
/// as opening a CSV does (so a refresh keeps any width set since), and
/// columns the database gives decimals for shown with that many (unless
/// they have a number format of their own).
pub(crate) fn layout(book: &Book, sheet: u32, names: &[String], answer: &Answer) -> Vec<Op> {
    let dec = |k: usize| answer.decimals.get(k).copied().flatten().filter(|d| *d > 0);
    let mut texts: Vec<Vec<String>> = vec![names.iter().map(|n| format!("{n}  ")).collect()];
    texts.extend(answer.rows.iter().map(|r| r.iter().enumerate().map(|(k, v)| shown(v, dec(k))).collect()));
    let columns: Vec<_> = crate::csv::fit_widths(&texts)
        .into_iter()
        .filter(|c| c.at <= names.len() as i32 && crate::ops::column_size(book, sheet, c.at).size.is_none())
        .collect();
    let mut styles = Vec::new();
    for k in 0..names.len() {
        let (Some(d), Some((_, kind))) = (dec(k), answer.columns.get(k)) else { continue };
        if kind != "number" {
            continue;
        }
        let fmt = decimals_format(d);
        for r in 0..answer.rows.len() as i32 {
            let (row, col) = (r + 2, k as i32 + 1);
            let mut style = book.style(sheet, row, col);
            if style.num_fmt.eq_ignore_ascii_case("general") || style.num_fmt.is_empty() {
                style.num_fmt = fmt.clone();
                styles.push(crate::ops::StyleEdit { row, col, style });
            }
        }
    }
    let mut ops = Vec::new();
    if !columns.is_empty() {
        ops.push(Op::Columns { sheet, columns });
    }
    if !styles.is_empty() {
        ops.push(Op::Styles { sheet, cells: styles });
    }
    ops
}

/// Column names a table can have: none empty, none the same as another.
fn column_names(answer: &Answer) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (k, (name, _)) in answer.columns.iter().enumerate() {
        let base = if name.trim().is_empty() { format!("Column{}", k + 1) } else { name.trim().to_string() };
        let mut n = base.clone();
        let mut i = 2;
        while out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            n = format!("{base} {i}");
            i += 1;
        }
        out.push(n);
    }
    out
}

fn cell(kind: &str, v: &Json) -> Option<CellData> {
    match v {
        Json::Null => None,
        Json::Bool(b) => Some(CellData::Bool(*b)),
        // The number as written, exactly.
        Json::Number(n) => ironcalc_base::num::Num::parse(&n.to_string())
            .or_else(|| n.as_f64().map(ironcalc_base::num::Num::from))
            .map(CellData::Number),
        // Dates as the sheet reads them (so they sort and add up as dates).
        Json::String(s) if kind == "date" && !s.is_empty() => Some(CellData::Input(s.clone())),
        Json::String(s) if s.is_empty() => None,
        Json::String(s) => Some(CellData::Text(s.clone())),
        other => Some(CellData::Text(other.to_string())),
    }
}

/// The edits that put `answer` on query sheet `sheet` (in place of what's
/// there) and record `def` with how it went.
pub fn fill(book: &Book, sheet: u32, mut def: Def, answer: &Answer) -> Result<Vec<Op>, String> {
    book.index(sheet)?;
    let names = column_names(answer);
    if names.is_empty() {
        return Err("The query gave no columns".to_string());
    }
    let width = names.len() as i32;
    let height = answer.rows.len() as i32;
    let mut cells = Vec::new();
    // What's there now goes (all of it: a query sheet holds only its table).
    let mut written = std::collections::HashSet::new();
    for (k, n) in names.iter().enumerate() {
        cells.push(CellEdit { row: 1, col: k as i32 + 1, data: Some(CellData::Text(n.clone())) });
        written.insert((1, k as i32 + 1));
    }
    for (r, row) in answer.rows.iter().enumerate() {
        for (k, (_, kind)) in answer.columns.iter().enumerate() {
            let at = (r as i32 + 2, k as i32 + 1);
            let data = row.get(k).and_then(|v| cell(kind, v));
            if data.is_some() {
                written.insert(at);
            }
            cells.push(CellEdit { row: at.0, col: at.1, data });
        }
    }
    let i = book.index(sheet)? as usize;
    for (r, c, _) in book.model.workbook.worksheets[i].sheet_data.cells() {
        if !written.contains(&(r, c)) && (r > height + 1 || c > width) {
            cells.push(CellEdit { row: r, col: c, data: None });
        }
    }
    // Leave out empties for cells that are empty already.
    let i32s = |e: &CellEdit| (e.row, e.col);
    cells.retain(|e| e.data.is_some() || book.cell_data(sheet, i32s(e).0, i32s(e).1).is_some());

    let mut ops = Vec::new();
    let old = tables::get(book, &def.table).filter(|t| t.sheet == sheet);
    let name = match &old {
        Some(t) => t.name.clone(),
        None => tables::free_name(book, if def.table.is_empty() { &def.query_name } else { &def.table }),
    };
    // The table: the header and every row (one empty row when there are none:
    // a table needs a row under its header).
    let table = TableDef {
        name: name.clone(),
        sheet,
        top: 1,
        left: 1,
        bottom: height.max(1) + 1,
        right: width,
        header: true,
        totals: false,
        columns: names.iter().map(|n| TableCol { name: n.clone(), total: None }).collect(),
        stripes: old.as_ref().map(|t| t.stripes).unwrap_or(true),
    };
    // The old table goes first so cells under its header can change.
    if old.is_some() {
        ops.push(Op::Table { name: name.clone(), table: None });
    }
    ops.push(Op::Cells { sheet, cells });
    ops.push(Op::Table { name: name.clone(), table: Some(table) });
    ops.extend(layout(book, sheet, &names, answer));
    def.table = name;
    def.rows = answer.rows.len();
    def.truncated = answer.truncated;
    if !answer.notes.is_null() && answer.notes != json!({}) {
        def.notes = answer.notes.clone();
    }
    ops.push(Op::Query { sheet, def: Some(def) });
    Ok(ops)
}

/// The values for a query's `:names` now (cells read as they show).
pub fn params(book: &Book, def: &Def) -> BTreeMap<String, Json> {
    def.params
        .iter()
        .map(|(name, p)| {
            let v = match &p.cell {
                Some(at) => read_cell(book, at).unwrap_or(Json::Null),
                None => p.value.clone(),
            };
            (name.clone(), v)
        })
        .collect()
}

/// A cell's value by its address (`B2`, `Inputs!B2`, `'My sheet'!B2`).
fn read_cell(book: &Book, at: &str) -> Option<Json> {
    let (sheet, cell) = match at.rsplit_once('!') {
        Some((s, c)) => (Some(s.trim().trim_matches('\'').replace("''", "'")), c),
        None => (None, at),
    };
    let (row, col) = tables::parse_cell_pub(cell.trim().trim_start_matches('$').replace('$', "").as_str())?;
    let i = match sheet {
        Some(name) => book.model.workbook.worksheets.iter().position(|w| w.name.eq_ignore_ascii_case(&name))? as u32,
        None => 0,
    };
    use ironcalc_base::cell::CellValue as V;
    match book.model.get_cell_value_by_index(i, row, col).ok()? {
        V::None => Some(Json::Null),
        V::Number(n) => Some(json!(n)),
        V::String(s) => Some(json!(s)),
        V::Boolean(b) => Some(json!(b)),
    }
}

/// Every query sheet: its id, name and query (to refresh them all).
pub fn all(book: &Book) -> Vec<Json> {
    book.queries
        .iter()
        .filter_map(|(id, d)| {
            book.sheet_name(*id)
                .ok()
                .map(|name| json!({ "sheet": id, "name": name, "def": d, "params": params(book, d) }))
        })
        .collect()
}
