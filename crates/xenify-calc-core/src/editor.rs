//! Table editor sheets: rows of one database table, in a table on a sheet of
//! their own, that people change like any cells. The book keeps the rows as
//! they were read (or last sent); what to send is worked out by comparing
//! them with the sheet, row by row by the table's key: changed cells are
//! updates, rows without a key (or with one the database didn't give) are
//! new rows, and rows that aren't there any more are deleted. So Undo,
//! sorting and recovery copies need nothing special.
//!
//! Save sends the changes (the app talks to the server) and hands the
//! answers to `sent`. A cell someone else changed first isn't sent: it
//! waits for a choice, theirs or mine (`resolve`).

use crate::book::{Book, CellData};
use crate::ops::{CellEdit, Op};
use crate::query::Answer;
use crate::tables::{self, TableCol, TableDef};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Def {
    pub server: String,
    pub source: i64,
    pub source_name: String,
    /// The database table, and the Calc table its rows are in.
    pub table: String,
    pub calc_table: String,
    pub key: Vec<String>,
    pub locked: Vec<String>,
    pub add: bool,
    pub delete: bool,
    /// Each column's name and kind (number, text, date, bool).
    pub columns: Vec<(String, String)>,
    /// The columns asked for (the key columns come too); none: all of them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<String>,
    /// Only rows where these columns hold these values; in this order.
    pub only: Vec<(String, Json)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<(String, bool)>,
    pub refreshed: String,
    /// How many rows the table has in all (with `only`), and whether the
    /// source's row limit left some out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    pub truncated: bool,
    /// The rows as read, or as last sent.
    pub loaded: Vec<Vec<Json>>,
    /// Cells someone else changed first: waiting for a choice.
    pub conflicts: Vec<Conflict>,
    /// Cells sent with the last Save (shown until the next refresh).
    pub sent: Vec<(String, String)>,
    /// The server's ticket for this table on this source: whoever has the
    /// file may read and change it with it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ticket: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conflict {
    pub key: BTreeMap<String, Json>,
    pub column: String,
    /// What the database holds now (as text), and what was sent.
    pub now: Json,
    pub mine: Json,
    /// The row was deleted in the database.
    pub gone: bool,
}

/// One change to send (the server's datasources::Change), with the sheet
/// row a new row is on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Change {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    pub table: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub key: BTreeMap<String, Json>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub column: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<Json>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<i32>,
}

impl Def {
    fn col(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|(n, _)| n.eq_ignore_ascii_case(name))
    }
    fn key_cols(&self) -> Vec<usize> {
        self.key.iter().filter_map(|k| self.col(k)).collect()
    }
    fn is_locked(&self, k: usize) -> bool {
        let n = &self.columns[k].0;
        self.key.iter().chain(&self.locked).any(|x| x.eq_ignore_ascii_case(n))
    }
    /// A row's key as text (to match rows up), None when it has none.
    fn key_of(&self, row: &[Json]) -> Option<String> {
        let parts: Vec<String> = self.key_cols().iter().map(|&k| text(row.get(k).unwrap_or(&Json::Null))).collect();
        if parts.is_empty() || parts.iter().all(|p| p.is_empty()) { None } else { Some(parts.join("\u{1f}")) }
    }
    fn key_map(&self, row: &[Json]) -> BTreeMap<String, Json> {
        self.key_cols()
            .iter()
            .map(|&k| (self.columns[k].0.clone(), row.get(k).cloned().unwrap_or(Json::Null)))
            .collect()
    }
}

fn text(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        Json::Number(n) => n
            .as_f64()
            .map(|f| if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { f.to_string() })
            .unwrap_or_default(),
        other => other.to_string(),
    }
}

/// Whether two keys name the same row (1 and 1.0 alike).
fn same_key(a: &BTreeMap<String, Json>, b: &BTreeMap<String, Json>) -> bool {
    a.len() == b.len()
        && a.iter().all(|(k, v)| b.iter().any(|(k2, v2)| k2.eq_ignore_ascii_case(k) && text(v) == text(v2)))
}

fn same(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            x.as_f64().zip(y.as_f64()).is_some_and(|(x, y)| (x - y).abs() <= 1e-9 * x.abs().max(1.0))
        }
        (Json::Null, Json::String(s)) | (Json::String(s), Json::Null) => s.is_empty(),
        _ => a == b,
    }
}

/// A date's serial number (days since 1899-12-30) as the database writes it.
fn date_text(serial: f64) -> String {
    let days = serial.floor() as i64;
    let secs = ((serial - serial.floor()) * 86400.0).round() as i64;
    // Days from 1970-01-01, then the civil date (Howard Hinnant's method).
    let z = days - 25569 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    if secs == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    }
}

/// What a sheet cell holds, as the database would take it.
fn cell_value(book: &Book, i: u32, row: i32, col: i32, kind: &str) -> Json {
    use ironcalc_base::cell::CellValue as V;
    match book.model.get_cell_value_by_index(i, row, col).unwrap_or(V::None) {
        V::None => Json::Null,
        V::Number(n) if kind == "date" => json!(date_text(n)),
        V::Number(n) => json!(n),
        V::String(s) if s.is_empty() => Json::Null,
        V::String(s) if kind == "number" => s.trim().parse::<f64>().map(|n| json!(n)).unwrap_or(json!(s)),
        V::String(s) => json!(s),
        V::Boolean(b) => json!(b),
    }
}

fn data(kind: &str, v: &Json) -> Option<CellData> {
    match v {
        Json::Null => None,
        Json::Bool(b) => Some(CellData::Bool(*b)),
        // The number as written, exactly.
        Json::Number(n) => ironcalc_base::num::Num::parse(&n.to_string())
            .or_else(|| n.as_f64().map(ironcalc_base::num::Num::from))
            .map(CellData::Number),
        Json::String(s) if s.is_empty() => None,
        Json::String(s) if kind == "date" => Some(CellData::Input(s.clone())),
        Json::String(s) if kind == "number" => {
            Some(ironcalc_base::num::Num::parse(s).map(CellData::Number).unwrap_or_else(|| CellData::Text(s.clone())))
        }
        Json::String(s) => Some(CellData::Text(s.clone())),
        other => Some(CellData::Text(other.to_string())),
    }
}

/// The sheet's table rows: (sheet row, values by `def.columns`).
fn sheet_rows(book: &Book, sheet: u32, def: &Def) -> Vec<(i32, Vec<Json>)> {
    let Some(t) = tables::get(book, &def.calc_table) else { return Vec::new() };
    let Ok(i) = book.index(sheet) else { return Vec::new() };
    // Columns by their header names (people can move columns around).
    let at: Vec<Option<i32>> = def
        .columns
        .iter()
        .map(|(n, _)| t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(n)).map(|k| t.left + k as i32))
        .collect();
    let last = if t.totals { t.bottom - 1 } else { t.bottom };
    (t.top + 1..=last)
        .map(|r| {
            (
                r,
                def.columns
                    .iter()
                    .zip(&at)
                    .map(|((_, kind), c)| c.map(|c| cell_value(book, i, r, c, kind)).unwrap_or(Json::Null))
                    .collect(),
            )
        })
        .collect()
}

/// What Save would send, worked out now.
pub fn changes(book: &Book, sheet: u32, def: &Def) -> Vec<Change> {
    let loaded: HashMap<String, &Vec<Json>> = def.loaded.iter().filter_map(|r| def.key_of(r).map(|k| (k, r))).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (r, row) in sheet_rows(book, sheet, def) {
        match def.key_of(&row).filter(|k| loaded.contains_key(k)) {
            Some(k) => {
                seen.insert(k.clone());
                let old = loaded[&k];
                for (c, (name, _)) in def.columns.iter().enumerate() {
                    if def.is_locked(c) {
                        continue;
                    }
                    let (o, n) = (old.get(c).unwrap_or(&Json::Null), row.get(c).unwrap_or(&Json::Null));
                    if !same(o, n) {
                        out.push(Change {
                            table: def.table.clone(),
                            key: def.key_map(old),
                            column: name.clone(),
                            old: Some(o.clone()),
                            new: Some(n.clone()),
                            row: Some(r),
                            ..Default::default()
                        });
                    }
                }
            }
            None => {
                if row.iter().all(Json::is_null) {
                    continue;
                }
                let values = def
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(c, _)| !def.is_locked(*c) || def.key_cols().contains(c))
                    .filter(|(c, _)| !row[*c].is_null())
                    .map(|(c, (n, _))| (n.clone(), row[c].clone()))
                    .collect();
                out.push(Change {
                    kind: "insert".into(),
                    table: def.table.clone(),
                    values,
                    row: Some(r),
                    ..Default::default()
                });
            }
        }
    }
    for r in &def.loaded {
        if let Some(k) = def.key_of(r)
            && !seen.contains(&k)
        {
            out.push(Change {
                kind: "delete".into(),
                table: def.table.clone(),
                key: def.key_map(r),
                ..Default::default()
            });
        }
    }
    out
}

/// The edits that put rows read from the database on editor sheet `sheet`
/// (in place of what's there), recording `def` with them as loaded.
pub fn fill(book: &Book, sheet: u32, mut def: Def, answer: &Answer) -> Result<Vec<Op>, String> {
    let i = book.index(sheet)? as usize;
    if answer.columns.is_empty() {
        return Err("The table has no columns".to_string());
    }
    def.columns = answer.columns.clone();
    let width = def.columns.len() as i32;
    let height = answer.rows.len() as i32;
    let mut cells = Vec::new();
    let mut written = HashSet::new();
    for (k, (n, _)) in def.columns.iter().enumerate() {
        cells.push(CellEdit { row: 1, col: k as i32 + 1, data: Some(CellData::Text(n.clone())) });
        written.insert((1, k as i32 + 1));
    }
    for (r, row) in answer.rows.iter().enumerate() {
        for (k, (_, kind)) in def.columns.iter().enumerate() {
            let at = (r as i32 + 2, k as i32 + 1);
            let d = row.get(k).and_then(|v| data(kind, v));
            if d.is_some() {
                written.insert(at);
            }
            cells.push(CellEdit { row: at.0, col: at.1, data: d });
        }
    }
    for (r, c, _) in book.model.workbook.worksheets[i].sheet_data.cells() {
        if !written.contains(&(r, c)) && (r > height + 1 || c > width) {
            cells.push(CellEdit { row: r, col: c, data: None });
        }
    }
    cells.retain(|e| e.data.is_some() || book.cell_data(sheet, e.row, e.col).is_some());
    let old = tables::get(book, &def.calc_table).filter(|t| t.sheet == sheet);
    let name = match &old {
        Some(t) => t.name.clone(),
        None => tables::free_name(book, if def.calc_table.is_empty() { &def.table } else { &def.calc_table }),
    };
    let table = TableDef {
        name: name.clone(),
        sheet,
        top: 1,
        left: 1,
        bottom: height.max(1) + 1,
        right: width,
        header: true,
        totals: false,
        columns: def.columns.iter().map(|(n, _)| TableCol { name: n.clone(), total: None }).collect(),
        stripes: old.as_ref().map(|t| t.stripes).unwrap_or(true),
    };
    let mut ops = Vec::new();
    if old.is_some() {
        ops.push(Op::Table { name: name.clone(), table: None });
    }
    ops.push(Op::Cells { sheet, cells });
    ops.push(Op::Table { name: name.clone(), table: Some(table) });
    let names: Vec<String> = def.columns.iter().map(|(n, _)| n.clone()).collect();
    ops.extend(crate::query::layout(book, sheet, &names, answer));
    // What the sheet shows now, as the database gave it (dates as the sheet reads them back).
    def.calc_table = name;
    def.loaded = answer.rows.clone();
    def.truncated = answer.truncated;
    def.conflicts.clear();
    def.sent.clear();
    if let Some(n) = answer.notes.get("total").and_then(Json::as_f64) {
        def.total = Some(n);
    }
    ops.push(Op::Editor { sheet, def: Some(def) });
    Ok(ops)
}

/// After the edits from `fill` are applied: the loaded rows as the sheet
/// reads them back (a date the database wrote with a time zone, or a number
/// written as text, must not look changed).
pub fn settle(book: &mut Book, sheet: u32) {
    let Some(mut def) = book.editors.get(&sheet).cloned() else { return };
    let rows = sheet_rows(book, sheet, &def);
    if rows.len() == def.loaded.len() {
        for ((_, now), was) in rows.iter().zip(def.loaded.iter_mut()) {
            for (k, v) in now.iter().enumerate() {
                if let Some(w) = was.get_mut(k)
                    && text(w) != text(v)
                    && (def.columns[k].1 == "date" || def.columns[k].1 == "number")
                {
                    *w = v.clone();
                }
            }
        }
    }
    book.editors.insert(sheet, def);
}

/// Records how Save went: `changes` as sent and the server's `outcomes`
/// (in the same order). Returns edits for the sheet (new rows' keys).
pub fn sent(book: &mut Book, sheet: u32, changes: &[Change], outcomes: &[Json]) -> Result<Vec<Op>, String> {
    let mut def = book.editors.get(&sheet).cloned().ok_or("this sheet isn't a table editor")?;
    let i = book.index(sheet)?;
    let t = tables::get(book, &def.calc_table).ok_or("the table isn't there any more")?;
    let col_at =
        |name: &str| t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name)).map(|k| t.left + k as i32);
    let mut cells = Vec::new();
    def.sent.clear();
    for (ch, o) in changes.iter().zip(outcomes) {
        let outcome = o["outcome"].as_str().unwrap_or("");
        let row_at = |def: &Def, key: &BTreeMap<String, Json>| {
            def.loaded.iter().position(|r| {
                def.key_cols().iter().all(|&k| {
                    text(r.get(k).unwrap_or(&Json::Null)) == text(key.get(&def.columns[k].0).unwrap_or(&Json::Null))
                })
            })
        };
        match (ch.kind.as_str(), outcome) {
            ("insert", "added") => {
                let Some(r) = ch.row else { continue };
                // The key the database gave goes in the row's key cells.
                let mut row: Vec<Json> = def
                    .columns
                    .iter()
                    .map(|(n, kind)| {
                        col_at(n)
                            .map(|c| cell_value(book, i, r, c, kind))
                            .unwrap_or_else(|| ch.values.get(n).cloned().unwrap_or(Json::Null))
                    })
                    .collect();
                for &k in &def.key_cols() {
                    let (n, kind) = &def.columns[k];
                    let got = o["key"].get(n).cloned().unwrap_or(Json::Null);
                    let v = match (&got, kind.as_str()) {
                        (Json::String(s), "number") => s.parse::<f64>().map(|f| json!(f)).unwrap_or(got.clone()),
                        _ => got.clone(),
                    };
                    if let Some(c) = col_at(n) {
                        cells.push(CellEdit { row: r, col: c, data: data(kind, &v) });
                    }
                    row[k] = v;
                }
                for (k, (n, _)) in def.columns.iter().enumerate() {
                    if !def.is_locked(k) {
                        def.sent.push((def.key_of(&row).unwrap_or_default(), n.clone()));
                    }
                }
                def.loaded.push(row);
            }
            ("delete", "deleted") | ("delete", "missing") => {
                if let Some(at) = row_at(&def, &ch.key) {
                    def.loaded.remove(at);
                }
            }
            (_, "changed") => {
                if let (Some(at), Some(c)) = (row_at(&def, &ch.key), def.col(&ch.column)) {
                    def.loaded[at][c] = ch.new.clone().unwrap_or(Json::Null);
                    let k = def.key_of(&def.loaded[at]).unwrap_or_default();
                    def.sent.push((k, ch.column.clone()));
                }
            }
            (_, "conflict") | (_, "missing") => {
                def.conflicts.retain(|x| !(same_key(&x.key, &ch.key) && x.column.eq_ignore_ascii_case(&ch.column)));
                def.conflicts.push(Conflict {
                    key: ch.key.clone(),
                    column: ch.column.clone(),
                    now: o["now"].clone(),
                    mine: ch.new.clone().unwrap_or(Json::Null),
                    gone: outcome == "missing",
                });
            }
            _ => {}
        }
    }
    // Choices still waiting are only for cells that still differ.
    book.editors.insert(sheet, def);
    Ok(if cells.is_empty() { Vec::new() } else { vec![Op::Cells { sheet, cells }] })
}

/// A choice for a cell someone else changed first: "theirs" (the sheet takes
/// what the database holds) or "mine" (the next Save sends it anyway).
/// Returns the sheet edits it needs.
pub fn resolve(
    book: &mut Book,
    sheet: u32,
    key: &BTreeMap<String, Json>,
    column: &str,
    choice: &str,
) -> Result<Vec<Op>, String> {
    let mut def = book.editors.get(&sheet).cloned().ok_or("this sheet isn't a table editor")?;
    let at = def
        .conflicts
        .iter()
        .position(|x| same_key(&x.key, key) && x.column.eq_ignore_ascii_case(column))
        .ok_or("there's no choice waiting there")?;
    let cf = def.conflicts.remove(at);
    let lrow = def
        .loaded
        .iter()
        .position(|r| def.key_map(r).iter().all(|(k, v)| text(v) == text(key.get(k).unwrap_or(&Json::Null))));
    let mut ops = Vec::new();
    let t = tables::get(book, &def.calc_table).ok_or("the table isn't there any more")?;
    let srow = sheet_rows(book, sheet, &def)
        .into_iter()
        .find(|(_, r)| def.key_map(r).iter().all(|(k, v)| text(v) == text(key.get(k).unwrap_or(&Json::Null))))
        .map(|(r, _)| r);
    if cf.gone {
        // The row was deleted in the database: theirs takes it off the sheet too.
        if let Some(l) = lrow {
            def.loaded.remove(l);
        }
        if choice == "theirs"
            && let Some(r) = srow
        {
            ops.push(Op::DeleteRows { sheet, at: r, count: 1 });
        }
        book.editors.insert(sheet, def);
        return Ok(ops);
    }
    let Some(c) = def.col(column) else { return Err("no such column".into()) };
    let kind = def.columns[c].1.clone();
    let now = match (&cf.now, kind.as_str()) {
        (Json::String(s), "number") => s.parse::<f64>().map(|f| json!(f)).unwrap_or(cf.now.clone()),
        (Json::String(s), "bool") => json!(matches!(s.as_str(), "t" | "true" | "1")),
        _ => cf.now.clone(),
    };
    if let Some(l) = lrow {
        def.loaded[l][c] = now.clone();
    }
    if choice == "theirs"
        && let (Some(r), Some(k)) = (srow, t.columns.iter().position(|x| x.name.eq_ignore_ascii_case(column)))
    {
        ops.push(Op::Cells {
            sheet,
            cells: vec![CellEdit { row: r, col: t.left + k as i32, data: data(&kind, &now) }],
        });
    }
    book.editors.insert(sheet, def);
    Ok(ops)
}

/// What the sheet shows about its changes: counts, marks for cells (not
/// sent yet, new rows, needs a choice, sent with the last Save) and the
/// choices waiting.
pub fn state(book: &Book, sheet: u32) -> Json {
    let Some(def) = book.editors.get(&sheet) else { return Json::Null };
    let list = changes(book, sheet, def);
    let t = tables::get(book, &def.calc_table);
    let col_at = |name: &str| {
        t.as_ref()
            .and_then(|t| t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name)).map(|k| t.left + k as i32))
    };
    let rows = sheet_rows(book, sheet, def);
    let row_of = |key: &BTreeMap<String, Json>| {
        rows.iter()
            .find(|(_, r)| def.key_map(r).iter().all(|(k, v)| text(v) == text(key.get(k).unwrap_or(&Json::Null))))
            .map(|(r, _)| *r)
    };
    let mut marks = Vec::new();
    let conflict_at = |key: &BTreeMap<String, Json>, col: &str| {
        def.conflicts.iter().any(|x| same_key(&x.key, key) && x.column.eq_ignore_ascii_case(col))
    };
    for ch in &list {
        match ch.kind.as_str() {
            "insert" => {
                if let (Some(r), Some(t)) = (ch.row, &t) {
                    marks.push(json!({ "row": r, "col": t.left, "right": t.right, "kind": "new" }));
                }
            }
            "" => {
                if let (Some(r), Some(c)) = (ch.row, col_at(&ch.column))
                    && !conflict_at(&ch.key, &ch.column)
                {
                    marks.push(json!({ "row": r, "col": c, "kind": "pending" }));
                }
            }
            _ => {}
        }
    }
    let mut choices = Vec::new();
    for cf in &def.conflicts {
        let (r, c) = (row_of(&cf.key), col_at(&cf.column));
        if let (Some(r), Some(c)) = (r, c) {
            marks.push(json!({ "row": r, "col": c, "kind": "conflict" }));
        }
        choices.push(json!({ "row": r, "col": c, "key": cf.key, "column": cf.column, "now": cf.now, "mine": cf.mine, "gone": cf.gone }));
    }
    let pending: HashSet<(i32, String)> =
        list.iter().filter_map(|c| c.row.map(|r| (r, c.column.to_lowercase()))).collect();
    for (k, col) in &def.sent {
        if let Some((r, _)) = rows.iter().find(|(_, row)| def.key_of(row).as_deref() == Some(k.as_str()))
            && let Some(c) = col_at(col)
            && !pending.contains(&(*r, col.to_lowercase()))
        {
            marks.push(json!({ "row": r, "col": c, "kind": "sent" }));
        }
    }
    let count = |k: &str| list.iter().filter(|c| c.kind == k).count();
    let updates = list.iter().filter(|c| c.kind.is_empty() && !conflict_at(&c.key, &c.column)).count();
    let mut info = def.clone();
    info.loaded.clear();
    json!({
        "def": info, "rows": def.loaded.len(), "updates": updates, "inserts": count("insert"), "deletes": count("delete"),
        "waiting": def.conflicts.len(), "marks": marks, "choices": choices,
    })
}

/// Whether people may change these cells of an editor sheet: not its header,
/// not the key or locked columns of rows the database gave, no new rows or
/// deleted rows unless the admin allows them.
pub fn check(book: &Book, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Cells { sheet, cells } => {
                let Some(def) = book.editors.get(sheet) else { continue };
                let Some(t) = tables::get(book, &def.calc_table) else { continue };
                let rows: HashMap<i32, Vec<Json>> = sheet_rows(book, *sheet, def).into_iter().collect();
                let known: HashSet<String> = def.loaded.iter().filter_map(|r| def.key_of(r)).collect();
                for c in cells {
                    if c.col < t.left || c.col > t.right {
                        if c.row >= t.top && c.row <= t.bottom + 1 && c.data.is_some() {
                            return Err(
                                "Only the table’s columns go to the database: put other things on a normal sheet"
                                    .to_string(),
                            );
                        }
                        continue;
                    }
                    if c.row == t.top {
                        return Err("The column names come from the database".to_string());
                    }
                    let name = &t.columns[(c.col - t.left) as usize].name;
                    let Some(k) = def.col(name) else { continue };
                    let existing = rows.get(&c.row).and_then(|r| def.key_of(r)).is_some_and(|key| known.contains(&key));
                    if existing && def.is_locked(k) {
                        return Err(format!("{name} can’t be changed here"));
                    }
                    if !existing && !def.add && c.data.is_some() && c.row > t.bottom {
                        return Err("Rows can’t be added to this table".to_string());
                    }
                }
            }
            Op::InsertColumns { sheet, .. } | Op::DeleteColumns { sheet, .. } if book.editors.contains_key(sheet) => {
                return Err("The columns come from the database".to_string());
            }
            Op::InsertRows { sheet, .. } if book.editors.get(sheet).is_some_and(|d| !d.add) => {
                return Err("Rows can’t be added to this table".to_string());
            }
            Op::DeleteRows { sheet, at, count } => {
                let Some(def) = book.editors.get(sheet) else { continue };
                if def.delete {
                    continue;
                }
                let known: HashSet<String> = def.loaded.iter().filter_map(|r| def.key_of(r)).collect();
                if sheet_rows(book, *sheet, def)
                    .iter()
                    .any(|(r, row)| *r >= *at && *r < at + count && def.key_of(row).is_some_and(|k| known.contains(&k)))
                {
                    return Err("Rows can’t be deleted from this table: an admin chooses that".to_string());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Every table editor sheet: its id and what's waiting (for Save).
pub fn all(book: &Book) -> Vec<Json> {
    book.editors
        .iter()
        .map(|(id, def)| {
            let list = changes(book, *id, def);
            json!({ "sheet": id, "name": book.sheet_name(*id).unwrap_or_default(), "server": def.server, "source": def.source, "table": def.table,
                    "ticket": def.ticket, "changes": list, "waiting": def.conflicts.len() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::date_text;

    #[test]
    fn dates_as_the_database_writes_them() {
        assert_eq!(date_text(46026.0), "2026-01-04");
        assert_eq!(date_text(25569.0), "1970-01-01");
        assert_eq!(date_text(46026.5), "2026-01-04 12:00:00");
    }
}
