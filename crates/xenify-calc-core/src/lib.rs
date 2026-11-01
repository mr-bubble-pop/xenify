//! Xenify Calc's engine. One [`Session`] is one open book: its sheets,
//! edit history and saved state. The screens talk to it with
//! [`Session::call`] (a name and JSON arguments, JSON back), the same on
//! the desktop and in the browser.
//!
//! IronCalc works out the formulas (patched so tables grow: see
//! patches/README.md); Calc adds the kinds of sheet, working sheets,
//! tables with totals rows, column notes, and the `.xcalc` file.

pub mod ask;
pub mod book;
pub mod charts;
pub mod collab;
pub mod csv;
pub mod csvlink;
pub mod drawings;
pub mod edit;
pub mod editor;
pub mod file;
pub mod functions;
pub mod link;
pub mod lookup;
pub mod oldfiles;
pub mod ops;
pub mod pictures;
pub mod query;
pub mod remote;
pub mod report;
pub mod sheets;
pub mod sparklines;
pub mod summary;
pub mod tables;
pub mod xlsx;

#[cfg(test)]
mod tests;

pub use book::{Book, CellData, ColumnNote, Definition, NoteTarget, SheetInfo, SheetKind};
pub use ops::{CellEdit, Op};
pub use tables::{TableDef, Total};

use base64::Engine;
use serde_json::{Value as Json, json};

/// The kind of file a book is saved as by Save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Xcalc,
    Xlsx,
    Xlsm,
    Csv,
    Tsv,
    /// Opened only: saved as a Calc book or an Excel file.
    Xls,
    Ods,
}

impl Format {
    /// From a file's name, or (for Excel files) its first bytes.
    pub fn of(name: &str, bytes: &[u8]) -> Format {
        let lower = name.to_lowercase();
        if lower.ends_with(".xlsm") {
            Format::Xlsm
        } else if lower.ends_with(".xlsx") {
            Format::Xlsx
        } else if lower.ends_with(".csv") {
            Format::Csv
        } else if lower.ends_with(".tsv") || lower.ends_with(".tab") || lower.ends_with(".txt") {
            Format::Tsv
        } else if lower.ends_with(".xls") {
            Format::Xls
        } else if lower.ends_with(".ods") {
            Format::Ods
        } else if bytes.starts_with(b"PK") && bytes.windows(15).any(|w| w == b"xl/workbook.xml") {
            Format::Xlsx
        } else {
            Format::Xcalc
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Xcalc => "xcalc",
            Format::Xlsx => "xlsx",
            Format::Xlsm => "xlsm",
            Format::Csv => "csv",
            Format::Tsv => "tsv",
            Format::Xls => "xls",
            Format::Ods => "ods",
        }
    }
}

pub struct Session {
    pub book: Book,
    /// What Save writes (the kind of file it was opened from).
    pub format: Format,
    undo: Vec<Vec<Op>>,
    redo: Vec<Vec<Op>>,
    /// Counts edits; `saved` is its value when the file was last written.
    changes: u64,
    saved: u64,
    /// When the book is on a Xenify server (remote.rs).
    remote: Option<remote::Remote>,
    /// Edits made while editing together, not yet handed out to send.
    outbox: Vec<Op>,
    /// What a CSV file of this book is separated by (comma unless it came with another).
    pub csv_sep: char,
    /// What Write shows of the book (`set_link`), written into the next .xcalc as link.json.
    pub link: Option<Vec<u8>>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

/// "-3+4i", "+2.5-1j", "-4i": complex numbers as IMSUM and the like take
/// them (typed with a sign first they'd otherwise be read as a formula).
fn complex_text(input: &str) -> bool {
    let t = input.trim();
    let Some(body) = t.strip_prefix(['+', '-']) else { return false };
    let Some(body) = body.strip_suffix(['i', 'j']) else { return false };
    let num = |s: &str| s.is_empty() || s.parse::<f64>().is_ok_and(|_| !s.contains(['e', 'E', 'i', 'n']));
    match body.rfind(['+', '-']).filter(|k| *k > 0 && !body[..*k].ends_with(['e', 'E'])) {
        Some(k) => num(&body[..k]) && !body[..k].is_empty() && num(&body[k + 1..]),
        None => num(body),
    }
}

fn arg_u32(a: &Json, k: &str) -> Result<u32, String> {
    a[k].as_u64().map(|v| v as u32).ok_or_else(|| format!("missing {k}"))
}

fn arg_i32(a: &Json, k: &str) -> Result<i32, String> {
    a[k].as_i64().map(|v| v as i32).ok_or_else(|| format!("missing {k}"))
}

fn arg_range(a: &Json) -> Result<edit::Range, String> {
    let r = &a["range"];
    Ok(edit::Range {
        top: arg_i32(r, "top")?,
        left: arg_i32(r, "left")?,
        bottom: arg_i32(r, "bottom")?,
        right: arg_i32(r, "right")?,
    }
    .normal())
}

fn from_json<T: serde::de::DeserializeOwned>(v: &Json) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|e| e.to_string())
}

fn arg_str<'a>(a: &'a Json, k: &str) -> Result<&'a str, String> {
    a[k].as_str().ok_or_else(|| format!("missing {k}"))
}

impl Session {
    pub fn new() -> Session {
        Session::with_book(Book::new())
    }

    pub fn with_book(book: Book) -> Session {
        Session {
            book,
            format: Format::Xcalc,
            undo: Vec::new(),
            redo: Vec::new(),
            changes: 0,
            saved: 0,
            remote: None,
            outbox: Vec::new(),
            csv_sep: ',',
            link: None,
        }
    }

    /// Opens the bytes of a `.xcalc` file.
    pub fn open(bytes: &[u8]) -> Result<Session, String> {
        Ok(Session::with_book(file::open(bytes)?))
    }

    /// Opens a file of any kind Calc reads, by its name (and first bytes).
    pub fn open_file(bytes: &[u8], name: &str) -> Result<Session, String> {
        let format = Format::of(name, bytes);
        let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let stem = stem.rsplit_once('.').map_or(stem, |(a, _)| a);
        let book = match format {
            Format::Xcalc => file::open(bytes)?,
            Format::Xlsx | Format::Xlsm => xlsx::open(bytes, stem)?,
            Format::Csv | Format::Tsv => csv::open(&String::from_utf8_lossy(bytes), stem)?,
            Format::Xls => oldfiles::open(bytes, ".xls")?,
            Format::Ods => oldfiles::open(bytes, "OpenDocument")?,
        };
        let mut s = Session::with_book(book);
        // Calc doesn't write .xls or .ods: Save asks where to keep it (an Excel file by default).
        s.format = match format {
            Format::Xls | Format::Ods => Format::Xlsx,
            f => f,
        };
        if matches!(format, Format::Xls | Format::Ods) {
            s.mark_unsaved();
        }
        Ok(s)
    }

    /// The book as a file of `format` (CSV and TSV: sheet `sheet`). Saving
    /// in the book's own kind of file counts as saved.
    pub fn save_as(&mut self, format: Format, sheet: Option<u32>) -> Result<Vec<u8>, String> {
        let bytes = match format {
            Format::Xcalc => file::save_with(&self.book, self.link.as_deref())?,
            Format::Xlsx | Format::Xlsm => xlsx::save(&self.book)?,
            Format::Csv | Format::Tsv => {
                let id = sheet.or_else(|| self.book.id_at(0)).ok_or("no sheet")?;
                csv::save(&self.book, id, if format == Format::Tsv { '\t' } else { self.csv_sep })?.into_bytes()
            }
            Format::Xls | Format::Ods => {
                return Err("Calc can open these but saves as an .xlsx file or a Calc book".to_string());
            }
        };
        if !matches!(format, Format::Csv | Format::Tsv) || format == self.format {
            self.format = format;
            self.saved = self.changes;
        }
        Ok(bytes)
    }

    /// The bytes of a `.xcalc` file; the book counts as saved.
    pub fn save(&mut self) -> Result<Vec<u8>, String> {
        let bytes = file::save(&self.book)?;
        self.saved = self.changes;
        Ok(bytes)
    }

    /// Counts as changed since the last save (writing the file failed, or
    /// it came back from a recovery copy).
    pub fn mark_unsaved(&mut self) {
        self.saved = self.changes.wrapping_add(u64::MAX / 2);
    }

    /// Changed since the last save.
    pub fn unsaved(&self) -> bool {
        // Editing together, changes go to the server as they're made (and
        // ones not sent yet are kept apart by the app until they are).
        self.changes != self.saved && !self.editing_together()
    }

    /// Applies `ops` as one step of undo.
    pub fn edit(&mut self, ops: &[Op]) -> Result<(), String> {
        // Table editor sheets: not their header, key or locked cells.
        editor::check(&self.book, ops)?;
        self.edit_unchecked(ops)
    }

    /// An edit Calc makes itself (rows from a database, a sort).
    fn edit_unchecked(&mut self, ops: &[Op]) -> Result<(), String> {
        if ops.is_empty() {
            return Ok(());
        }
        let undo = ops::apply_all(&mut self.book, ops)?;
        self.made(ops);
        self.undo.push(undo);
        self.redo.clear();
        self.changes += 1;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<bool, String> {
        let Some(ops) = self.undo.pop() else { return Ok(false) };
        let redo = ops::apply_all(&mut self.book, &ops)?;
        self.made(&ops);
        self.redo.push(redo);
        self.changes += 1;
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, String> {
        let Some(ops) = self.redo.pop() else { return Ok(false) };
        let undo = ops::apply_all(&mut self.book, &ops)?;
        self.made(&ops);
        self.undo.push(undo);
        self.changes += 1;
        Ok(true)
    }

    /// What someone typed into a cell. Typing just under a table without a
    /// totals row makes the table take that row in.
    pub fn type_in(&mut self, sheet: u32, row: i32, col: i32, input: &str) -> Result<(), String> {
        let kind = self.book.info(sheet).kind;
        if kind.locked() {
            return Err("This sheet shows what its source gives and can't be typed in".to_string());
        }
        // A complex number with a sign in front (-3+4i) is text, not a sum.
        let data = (!input.is_empty()).then(|| {
            if complex_text(input) { CellData::Text(input.to_string()) } else { CellData::Input(input.to_string()) }
        });
        let mut ops = Vec::new();
        if data.is_some()
            && let Some(grow) = tables::grow_for(&self.book, sheet, row, col)
        {
            ops.push(grow);
        }
        ops.push(Op::Cells { sheet, cells: vec![CellEdit { row, col, data }] });
        // Lines (Alt+Enter): the row grows to show them all.
        if !input.starts_with('=') && input.contains('\n') {
            let lines = input.lines().count().max(1) as f64;
            let want = lines * 17.0 + 8.0;
            let i = self.book.index(sheet)?;
            if self.book.model.get_row_height(i, row).unwrap_or(25.0) < want {
                ops.push(Op::Rows { sheet, rows: vec![ops::LineSize { at: row, size: Some(want), hidden: false }] });
            }
        }
        // A table's header: its column gets the new name, everywhere.
        if let Some(mut t) = tables::at(&self.book, sheet, row, col)
            && t.header
            && t.top == row
        {
            let k = (col - t.left) as usize;
            let mut name = input.trim().to_string();
            let taken = |n: &str, t: &TableDef| {
                t.columns.iter().enumerate().any(|(j, c)| j != k && c.name.eq_ignore_ascii_case(n))
            };
            if name.is_empty() || taken(&name, &t) {
                let base = if name.is_empty() { "Column".to_string() } else { name.clone() };
                name = (1..).map(|n| format!("{base}{n}")).find(|n| !taken(n, &t)).unwrap_or(base);
                ops.pop();
                ops.push(Op::Cells {
                    sheet,
                    cells: vec![CellEdit { row, col, data: Some(CellData::Text(name.clone())) }],
                });
            }
            if t.columns[k].name != name {
                let tname = t.name.clone();
                t.columns[k].name = name;
                ops.push(Op::Table { name: tname, table: Some(t) });
            }
        }
        self.edit(&ops)
    }

    fn sheets_json(&self) -> Json {
        let b = &self.book;
        let sheets: Vec<Json> = b
            .model
            .workbook
            .worksheets
            .iter()
            .map(|w| {
                let info = b.info(w.sheet_id);
                json!({ "id": w.sheet_id, "name": w.get_name(), "kind": info.kind, "working": info.working })
            })
            .collect();
        json!(sheets)
    }

    /// What the grid shows for rows `top..=bottom` and columns `left..=right`.
    fn view(&self, sheet: u32, top: i32, left: i32, bottom: i32, right: i32) -> Result<Json, String> {
        let b = &self.book;
        let i = b.index(sheet)?;
        let w = &b.model.workbook.worksheets[i as usize];
        // Comments, by cell.
        let comments: std::collections::HashMap<(i32, i32), (&str, &str)> = w
            .comments
            .iter()
            .filter_map(|c| {
                let (row, col) = tables::parse_cell_pub(&c.cell_ref)?;
                Some(((row, col), (c.author_name.as_str(), c.text.as_str())))
            })
            .collect();
        let theme = &b.model.workbook.theme;
        let mut cells = Vec::new();
        for (r, c, data, style) in b.cells_in(sheet, Some((top, bottom)), Some((left, right))) {
            let text = b.model.get_formatted_cell_value(i, r, c).unwrap_or_default();
            let number =
                matches!(b.model.get_cell_value_by_index(i, r, c), Ok(ironcalc_base::cell::CellValue::Number(_)));
            let comment = comments.get(&(r, c));
            if data.is_none() && style == Default::default() && comment.is_none() {
                continue;
            }
            // The look with conditional formatting on top (colours, data bars, icons).
            let ext = b.model.get_extended_style_for_cell(i, r, c).ok();
            let mut cell = json!({
                "r": r, "c": c, "text": text, "number": number,
                "formula": matches!(data, Some(CellData::Formula(_)) | Some(CellData::Array { .. })),
                "look": edit::look(b, ext.as_ref().map_or(&style, |e| &e.style)),
            });
            if let Some(e) = &ext {
                if let Some(bar) = &e.data_bar {
                    cell["bar"] = json!({
                        "value": bar.value, "axis": bar.axis_position, "color": bar.positive_color.to_rgb(theme),
                        "negative": bar.negative_color.to_rgb(theme), "gradient": bar.is_gradient, "show": bar.show_value,
                    });
                }
                if let Some(icon) = &e.icon {
                    cell["icon"] = json!({ "name": format!("{:?}", icon.icon), "color": icon.color.to_rgb(theme), "show": icon.show_value });
                }
                if let Some(rating) = &e.rating {
                    cell["rating"] = json!({
                        "name": format!("{:?}", rating.icon), "count": rating.count, "max": rating.max,
                        "color": rating.color.to_rgb(theme), "show": rating.show_value,
                    });
                }
            }
            if let Some((author, note)) = comment {
                cell["comment"] = json!({ "author": author, "text": note });
            }
            cells.push(cell);
        }
        // Sparklines (their cells may be empty).
        for sp in b
            .sparklines
            .iter()
            .filter(|s| s.sheet == sheet && s.row >= top && s.row <= bottom && s.col >= left && s.col <= right)
        {
            let spark = json!({ "kind": sp.kind, "values": sparklines::values(b, sp) });
            match cells.iter_mut().find(|c| c["r"] == sp.row && c["c"] == sp.col) {
                Some(c) => c["spark"] = spark,
                None => cells.push(json!({ "r": sp.row, "c": sp.col, "text": "", "number": false, "formula": false, "look": {}, "spark": spark })),
            }
        }
        let tables: Vec<TableDef> = tables::all(b)
            .into_iter()
            .filter(|t| t.sheet == sheet && t.bottom >= top && t.top <= bottom && t.right >= left && t.left <= right)
            .collect();
        let widths: Vec<Json> = (left..=right).map(|c| json!(b.model.get_column_width(i, c).unwrap_or(90.0))).collect();
        let heights: Vec<Json> = (top..=bottom).map(|r| json!(b.model.get_row_height(i, r).unwrap_or(25.0))).collect();
        Ok(json!({
            "cells": cells, "tables": tables, "widths": widths, "heights": heights,
            "frozen": [w.frozen_rows, w.frozen_columns], "grid": w.show_grid_lines,
        }))
    }

    /// The screens' way in: `name` with JSON arguments, JSON back.
    pub fn call(&mut self, name: &str, a: &Json) -> Result<Json, String> {
        if let Some(out) = self.remote_call(name, a) {
            return out;
        }
        Ok(match name {
            "sheets" => self.sheets_json(),
            "state" => {
                json!({ "can_undo": !self.undo.is_empty(), "can_redo": !self.redo.is_empty(), "unsaved": self.unsaved() })
            }
            "view" => self.view(
                arg_u32(a, "sheet")?,
                arg_i32(a, "top")?,
                arg_i32(a, "left")?,
                arg_i32(a, "bottom")?,
                arg_i32(a, "right")?,
            )?,
            "cell" => {
                let (sheet, row, col) = (arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?);
                let i = self.book.index(sheet)?;
                let m = &self.book.model;
                json!({
                    "input": m.get_localized_cell_content(i, row, col).unwrap_or_default(),
                    "text": m.get_formatted_cell_value(i, row, col).unwrap_or_default(),
                    "table": tables::at(&self.book, sheet, row, col).map(|t| t.name),
                })
            }
            "type" => {
                self.type_in(
                    arg_u32(a, "sheet")?,
                    arg_i32(a, "row")?,
                    arg_i32(a, "col")?,
                    a["input"].as_str().unwrap_or(""),
                )?;
                json!(true)
            }
            "edit" => {
                let ops: Vec<Op> = serde_json::from_value(a["ops"].clone()).map_err(|e| e.to_string())?;
                self.edit(&ops)?;
                json!(true)
            }
            "set_format" => {
                // A recovered book goes back to the kind of file it came from.
                self.format = from_json(&a["format"])?;
                json!(true)
            }
            // Opening a CSV: how its first rows read with a separator (none: the likeliest).
            "csv_preview" => {
                let text = arg_str(a, "text")?;
                let sep = a["sep"].as_str().and_then(|s| s.chars().next()).unwrap_or_else(|| csv::separator(text));
                let values: csv::Values = serde_json::from_value(a["values"].clone()).unwrap_or_default();
                let rows: Vec<Vec<String>> = csv::parse(text, sep).into_iter().take(12).collect();
                let exact: Vec<Vec<bool>> = rows
                    .iter()
                    .map(|r| r.iter().map(|v| matches!(csv::cell_of(v, values), CellData::Text(_))).collect())
                    .collect();
                let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
                json!({ "sep": sep.to_string(), "rows": rows, "text": exact, "lines": lines })
            }
            // …then read it into this (new, empty) book.
            "import_csv" => {
                let text = arg_str(a, "text")?;
                let sep = a["sep"].as_str().and_then(|s| s.chars().next());
                let values: csv::Values = serde_json::from_value(a["values"].clone()).unwrap_or_default();
                self.book = csv::open_with(text, a["name"].as_str().unwrap_or("Sheet1"), sep, values)?;
                let sep = sep.unwrap_or_else(|| csv::separator(text));
                self.format = if sep == '\t' { Format::Tsv } else { Format::Csv };
                self.csv_sep = sep;
                self.undo.clear();
                self.redo.clear();
                self.changes = 0;
                self.saved = 0;
                // Kept linked to the file: a Calc book that follows it (Save
                // asks where to keep the book; the file stays as it is).
                if !a["link"].is_null() {
                    let mut d: csvlink::Def = from_json(&a["link"])?;
                    if let Some(t) = a["read"].as_str() {
                        d.read = t.to_string();
                    }
                    let sheet = self.book.id_at(0).ok_or("no sheet")?;
                    let rows = csv::parse(text, sep);
                    d.rows = rows.len();
                    d.columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                    self.book.info.insert(sheet, SheetInfo { kind: SheetKind::LinkedCsv, working: false });
                    self.book.csv_links.insert(sheet, d);
                    self.format = Format::Xcalc;
                    self.mark_unsaved();
                }
                json!(true)
            }
            // What Write shows of the book ({format: "xenify-calc-link", items}), for the next .xcalc saved.
            "set_link" => {
                self.link = if a["link"].is_null() {
                    None
                } else {
                    Some(serde_json::to_vec(&a["link"]).map_err(|e| e.to_string())?)
                };
                json!(true)
            }
            "csv_sep" => {
                if let Some(c) = a["sep"].as_str().and_then(|s| s.chars().next()) {
                    self.csv_sep = c;
                }
                json!(self.csv_sep.to_string())
            }
            "mark_unsaved" => {
                self.mark_unsaved();
                json!(true)
            }
            "undo" => json!(self.undo()?),
            "redo" => json!(self.redo()?),
            "tables" => json!(tables::all(&self.book)),
            "make_table" => {
                let range = if a["range"].is_object() { Some(arg_range(a)?) } else { None };
                let (def, ops) =
                    tables::make(&self.book, arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?, range)?;
                self.edit(&ops)?;
                json!(tables::get(&self.book, &def.name))
            }
            // Σ on a selection: totals under its columns of numbers (or, when
            // it has none, the column the cursor is in), making the table
            // first when it isn't one. One step of undo.
            "totals_here" => {
                let (sheet, row, col) = (arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?);
                let range = if a["range"].is_object() { Some(arg_range(a)?) } else { None };
                let total: Option<Total> = serde_json::from_value(a["total"].clone()).map_err(|e| e.to_string())?;
                let made = tables::at(&self.book, sheet, row, col).is_none();
                if made {
                    let (_, ops) = tables::make(&self.book, sheet, row, col, range)?;
                    self.edit(&ops)?;
                }
                let t = tables::at(&self.book, sheet, row, col).ok_or("no table here")?;
                let (from, to) = match range {
                    Some(r) if r.right > r.left => (r.left.max(t.left), r.right.min(t.right)),
                    _ => (col.clamp(t.left, t.right), col.clamp(t.left, t.right)),
                };
                let numbers = |c: i32| tables::numbers_in(&self.book, &t, c);
                let mut cols: Vec<i32> = (from..=to).filter(|c| numbers(*c)).collect();
                if cols.is_empty() {
                    cols = (t.left..=t.right).filter(|c| numbers(*c)).collect();
                }
                if cols.is_empty() {
                    cols = vec![from];
                }
                let names: Vec<String> = cols.iter().map(|c| t.columns[(c - t.left) as usize].name.clone()).collect();
                let mut step = Vec::new();
                for name in &names {
                    let ops = tables::set_total(&self.book, &t.name, name, total)?;
                    editor::check(&self.book, &ops)?;
                    step.push(ops::apply_all(&mut self.book, &ops)?);
                    self.made(&ops);
                }
                // Undone in the opposite order, after the table goes (when it was made here).
                let mut undo: Vec<Op> = step.into_iter().rev().flatten().collect();
                if made && let Some(last) = self.undo.pop() {
                    undo.extend(last);
                } else {
                    self.redo.clear();
                    self.changes += 1;
                }
                self.undo.push(undo);
                json!({ "table": tables::get(&self.book, &t.name), "columns": names })
            }
            "total" => {
                let total: Option<Total> = serde_json::from_value(a["total"].clone()).map_err(|e| e.to_string())?;
                let ops = tables::set_total(&self.book, arg_str(a, "table")?, arg_str(a, "column")?, total)?;
                self.edit(&ops)?;
                json!(true)
            }
            "add_row" => {
                let values: Vec<(String, CellData)> = a["values"]
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), CellData::Input(s.to_string()))))
                            .collect()
                    })
                    .unwrap_or_default();
                let ops = tables::add_row(&self.book, arg_str(a, "table")?, &values)?;
                self.edit(&ops)?;
                json!(true)
            }
            "add_sheet" => {
                let id = self.book.next_sheet_id();
                let name = match a["name"].as_str() {
                    Some(n) => n.to_string(),
                    None => self.free_sheet_name(),
                };
                let at = a["at"].as_u64().map(|v| v as u32).unwrap_or(self.book.model.workbook.worksheets.len() as u32);
                self.edit(&[Op::AddSheet { sheet: id, name, at, info: SheetInfo::default() }])?;
                json!(id)
            }
            "rename_sheet" => {
                self.edit(&[Op::RenameSheet { sheet: arg_u32(a, "sheet")?, name: arg_str(a, "name")?.to_string() }])?;
                json!(true)
            }
            "move_sheet" => {
                self.edit(&[Op::MoveSheet { sheet: arg_u32(a, "sheet")?, to: arg_u32(a, "to")? }])?;
                json!(true)
            }
            "remove_sheet" => {
                self.edit(&[Op::RemoveSheet { sheet: arg_u32(a, "sheet")? }])?;
                json!(true)
            }
            "working" => {
                let sheet = arg_u32(a, "sheet")?;
                let mut info = self.book.info(sheet);
                info.working = a["working"].as_bool().unwrap_or(true);
                self.edit(&[Op::SheetInfo { sheet, info }])?;
                json!(true)
            }
            "insert_rows" | "delete_rows" | "insert_columns" | "delete_columns" => {
                let (sheet, at, count) =
                    (arg_u32(a, "sheet")?, arg_i32(a, "at")?, a["count"].as_i64().unwrap_or(1) as i32);
                let op = match name {
                    "insert_rows" => Op::InsertRows { sheet, at, count },
                    "delete_rows" => Op::DeleteRows { sheet, at, count },
                    "insert_columns" => Op::InsertColumns { sheet, at, count },
                    _ => Op::DeleteColumns { sheet, at, count },
                };
                self.edit(&[op])?;
                json!(true)
            }
            "format" => {
                let ops = edit::format(&self.book, arg_u32(a, "sheet")?, arg_range(a)?, &from_json(&a["set"])?);
                self.edit(&ops)?;
                json!(true)
            }
            "clear" => {
                let what = if a["what"].is_null() { edit::Clear::Contents } else { from_json(&a["what"])? };
                let sheet = arg_u32(a, "sheet")?;
                if what != edit::Clear::Formats && self.book.info(sheet).kind.locked() {
                    return Err("This sheet shows what its source gives and can't be changed".to_string());
                }
                let ops = edit::clear(&self.book, sheet, arg_range(a)?, what);
                self.edit(&ops)?;
                json!(true)
            }
            "sort" => {
                let keys: Vec<edit::SortKey> = from_json(&a["by"])?;
                let ops = edit::sort(
                    &mut self.book,
                    arg_u32(a, "sheet")?,
                    arg_range(a)?,
                    &keys,
                    a["header"].as_bool().unwrap_or(false),
                )?;
                // Rows move whole, keys and all.
                self.edit_unchecked(&ops)?;
                json!(true)
            }
            "column_values" => {
                let v = edit::column_values(&self.book, arg_str(a, "table")?, arg_str(a, "column")?)?;
                let filters = self.book.filters.get(arg_str(a, "table")?).cloned().unwrap_or_default();
                let keep = filters
                    .iter()
                    .find(|f| f.column.eq_ignore_ascii_case(arg_str(a, "column").unwrap_or("")))
                    .map(|f| f.keep.clone());
                json!({ "values": v.into_iter().map(|(v, n)| json!({ "value": v, "rows": n })).collect::<Vec<_>>(), "keep": keep })
            }
            "filter" => {
                let keep: Option<Vec<String>> = from_json(&a["keep"])?;
                let ops = edit::filter(&self.book, arg_str(a, "table")?, arg_str(a, "column")?, keep)?;
                self.edit(&ops)?;
                json!(true)
            }
            "find" => {
                json!(edit::find(&self.book, arg_str(a, "query")?, &from_json(&a["options"]).unwrap_or_default()))
            }
            "replace" => {
                let ops = edit::replace(
                    &self.book,
                    arg_str(a, "query")?,
                    a["with"].as_str().unwrap_or(""),
                    &from_json(&a["options"]).unwrap_or_default(),
                );
                let n: usize = ops.iter().map(|o| if let Op::Cells { cells, .. } = o { cells.len() } else { 0 }).sum();
                self.edit(&ops)?;
                json!(n)
            }
            "copy" => json!(edit::copy(&self.book, arg_u32(a, "sheet")?, arg_range(a)?)?),
            "paste" => {
                let (sheet, row, col) = (arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?);
                if self.book.info(sheet).kind.locked() {
                    return Err("This sheet shows what its source gives and can't be pasted into".to_string());
                }
                let mut ops = if a["clip"].is_object() {
                    let clip: edit::Clip = from_json(&a["clip"])?;
                    edit::paste(&mut self.book, sheet, row, col, &clip, a["values"].as_bool().unwrap_or(false))?
                } else {
                    edit::paste_text(sheet, row, col, arg_str(a, "text")?)
                };
                // More in the same step (a cut's cells emptied).
                if a["then"].is_array() {
                    let more: Vec<Op> = from_json(&a["then"])?;
                    ops.extend(more);
                }
                self.edit(&ops)?;
                json!(true)
            }
            "stats" => edit::stats(&self.book, arg_u32(a, "sheet")?, arg_range(a)?),
            "merge" => {
                let ops = edit::merge(&self.book, arg_u32(a, "sheet")?, arg_range(a)?)?;
                // More than one value: only the first is kept, so the screens ask first.
                // (The first value is kept: in the top left, or moved there when that's empty.)
                if a["check"].as_bool().unwrap_or(false) {
                    let r = arg_range(a)?;
                    let values = self
                        .book
                        .cells_in(arg_u32(a, "sheet")?, Some((r.top, r.bottom)), Some((r.left, r.right)))
                        .iter()
                        .filter(|c| c.2.is_some())
                        .count();
                    return Ok(json!({ "lost": values.saturating_sub(1) }));
                }
                self.edit(&ops)?;
                json!(true)
            }
            "unmerge" => {
                let ops = edit::unmerge(&self.book, arg_u32(a, "sheet")?, arg_range(a)?)?;
                self.edit(&ops)?;
                json!(true)
            }
            "freeze" => {
                self.edit(&[Op::Freeze {
                    sheet: arg_u32(a, "sheet")?,
                    rows: arg_i32(a, "rows")?,
                    columns: arg_i32(a, "columns")?,
                }])?;
                json!(true)
            }
            "sizes" => {
                let sheet = arg_u32(a, "sheet")?;
                let lines: Vec<ops::LineSize> = from_json(&a["lines"])?;
                let op = if a["rows"].as_bool().unwrap_or(false) {
                    Op::Rows { sheet, rows: lines }
                } else {
                    Op::Columns { sheet, columns: lines }
                };
                self.edit(&[op])?;
                json!(true)
            }
            "lookup_rows" => {
                // Where the answers go beside, from the cursor.
                let yours = lookup::your_rows(&self.book, arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?);
                json!(yours)
            }
            "lookup_in" => {
                let at = match (a["row"].as_i64(), a["col"].as_i64()) {
                    (Some(r), Some(c)) => Some((r as i32, c as i32)),
                    _ => None,
                };
                json!(lookup::look_in(&self.book, arg_u32(a, "sheet")?, at))
            }
            "lookup_candidates" => {
                let yours: lookup::Rows = from_json(&a["yours"])?;
                let look: lookup::Rows = from_json(&a["look"])?;
                json!(lookup::candidates(&self.book, &yours, &look, arg_i32(a, "on")?))
            }
            "lookup_preview" => {
                let p: lookup::Plan = from_json(&a["plan"])?;
                let rows = lookup::preview(&self.book, &p, a["rows"].as_u64().unwrap_or(4) as usize);
                let first = p.yours.data_top();
                json!({
                    "rows": rows.into_iter().map(|(k, v)| json!({ "key": k, "answer": v })).collect::<Vec<_>>(),
                    "formula": lookup::formula(&self.book, &p, first)?,
                })
            }
            "lookup_fill" => {
                let p: lookup::Plan = from_json(&a["plan"])?;
                let ops = lookup::fill(&self.book, &p)?;
                self.edit(&ops)?;
                json!(true)
            }
            // Spelling: the cells on a sheet holding words (typed text, not formulas), at most 3000.
            "texts" => {
                let sheet = arg_u32(a, "sheet")?;
                let i = self.book.index(sheet)? as usize;
                let w = &self.book.model.workbook.worksheets[i];
                let mut out = Vec::new();
                for (row, col, _) in w.sheet_data.cells() {
                    if let Some(CellData::Text(t)) = self.book.cell_data(sheet, row, col)
                        && t.chars().any(char::is_alphabetic)
                    {
                        out.push(json!([row, col, t]));
                        if out.len() >= 3000 {
                            break;
                        }
                    }
                }
                json!(out)
            }
            // The function dictionary: all names (for typing), or one whole entry.
            "functions" => functions::list(),
            "function" => functions::entry(arg_str(a, "name")?).ok_or("no such function")?,
            "charts" => {
                let sheet = arg_u32(a, "sheet")?;
                json!(self.book.charts.iter().filter(|c| c.sheet == sheet).collect::<Vec<_>>())
            }
            "chart_data" => {
                let id = arg_u32(a, "id")?;
                let c = self.book.charts.iter().find(|c| c.id == id).ok_or("no such chart")?;
                charts::data(&self.book, c)?
            }
            "add_chart" => {
                let id = self.book.next_chart_id();
                let at = (arg_i32(a, "row")?, arg_i32(a, "col")?);
                let chart = charts::suggest(&self.book, arg_u32(a, "sheet")?, arg_range(a)?, id, at)?;
                self.edit(&[Op::Chart { id, chart: Some(chart.clone()) }])?;
                json!(chart)
            }
            "set_chart" => {
                let chart: charts::Chart = from_json(&a["chart"])?;
                let id = chart.id;
                let ops = [Op::Chart { id, chart: Some(chart) }];
                // `again`: one more change from the chart panel, undone with the ones before it.
                let same =
                    matches!(self.undo.last().map(Vec::as_slice), Some([Op::Chart { id: last, .. }]) if *last == id);
                if a["again"].as_bool().unwrap_or(false) && same {
                    ops::apply_all(&mut self.book, &ops)?;
                    self.made(&ops);
                    self.redo.clear();
                    self.changes += 1;
                } else {
                    self.edit(&ops)?;
                }
                json!(true)
            }
            // The columns a chart's rows have, by name (for the chart panel).
            "chart_columns" => {
                let source: charts::Source = from_json(&a["source"])?;
                json!(charts::column_names(&self.book, &source)?)
            }
            // A copy of a chart (from this sheet or another) with its top left at a cell.
            "paste_chart" => {
                let mut chart: charts::Chart = from_json(&a["chart"])?;
                chart.id = self.book.next_chart_id();
                chart.sheet = arg_u32(a, "sheet")?;
                chart.row = arg_i32(a, "row")?;
                chart.col = arg_i32(a, "col")?;
                self.edit(&[Op::Chart { id: chart.id, chart: Some(chart.clone()) }])?;
                json!(chart)
            }
            "remove_chart" => {
                self.edit(&[Op::Chart { id: arg_u32(a, "id")?, chart: None }])?;
                json!(true)
            }
            "pictures" => json!(xlsx::pictures(&self.book, arg_u32(a, "sheet")?)),
            // Insert › Picture: the ones Calc has on a sheet, and adding, changing, removing.
            "my_pictures" => {
                let sheet = arg_u32(a, "sheet")?;
                json!(self.book.pictures.iter().filter(|p| p.sheet == sheet).collect::<Vec<_>>())
            }
            "add_picture" => {
                let mut p: pictures::Picture = from_json(&a["picture"])?;
                p.id = self.book.next_picture_id();
                if p.data.is_empty() {
                    return Err("That picture has nothing in it".to_string());
                }
                self.edit(&[Op::Picture { id: p.id, picture: Some(p.clone()) }])?;
                json!(p.id)
            }
            "set_picture" => {
                let p: pictures::Picture = from_json(&a["picture"])?;
                self.edit(&[Op::Picture { id: p.id, picture: Some(p) }])?;
                json!(true)
            }
            "remove_picture" => {
                self.edit(&[Op::Picture { id: arg_u32(a, "id")?, picture: None }])?;
                json!(true)
            }
            "add_sparklines" => {
                // One for each row (or column) of the cells picked, in the column (or row) beside them.
                let (sheet, src) = (arg_u32(a, "sheet")?, arg_range(a)?);
                let kind: sparklines::SparkKind =
                    if a["kind"].is_null() { sparklines::SparkKind::Line } else { from_json(&a["kind"])? };
                let mut ops = Vec::new();
                if src.right - src.left >= src.bottom - src.top {
                    for row in src.top..=src.bottom {
                        let s = sparklines::Sparkline {
                            sheet,
                            row,
                            col: src.right + 1,
                            kind,
                            source_sheet: sheet,
                            source: edit::Range { top: row, bottom: row, ..src },
                        };
                        ops.push(Op::Sparkline { sheet, row, col: src.right + 1, spark: Some(s) });
                    }
                } else {
                    for col in src.left..=src.right {
                        let s = sparklines::Sparkline {
                            sheet,
                            row: src.bottom + 1,
                            col,
                            kind,
                            source_sheet: sheet,
                            source: edit::Range { left: col, right: col, ..src },
                        };
                        ops.push(Op::Sparkline { sheet, row: src.bottom + 1, col, spark: Some(s) });
                    }
                }
                self.edit(&ops)?;
                json!(ops.len())
            }
            "remove_sparklines" => {
                let (sheet, r) = (arg_u32(a, "sheet")?, arg_range(a)?);
                let ops: Vec<Op> = self
                    .book
                    .sparklines
                    .iter()
                    .filter(|s| {
                        s.sheet == sheet && s.row >= r.top && s.row <= r.bottom && s.col >= r.left && s.col <= r.right
                    })
                    .map(|s| Op::Sparkline { sheet, row: s.row, col: s.col, spark: None })
                    .collect();
                self.edit(&ops)?;
                json!(ops.len())
            }
            "summary_columns" => json!(summary::columns(&self.book, &from_json(&a["from"])?)?),
            "summary_from" => {
                // What the cursor is in: its table, or its block of cells.
                let (sheet, row, col) = (arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?);
                let rows = lookup::rows_at(&self.book, sheet, row, col)
                    .ok_or("Put the cursor in the table or cells to summarise")?;
                let from = match rows.table {
                    Some(t) => summary::From { sheet, table: Some(t), range: None, header: true },
                    None => {
                        summary::From { sheet, table: None, range: Some(lookup::range_of(&rows)), header: rows.header }
                    }
                };
                json!({ "from": from, "columns": summary::columns(&self.book, &from)? })
            }
            "summary_preview" => {
                let d: summary::Def = from_json(&a["def"])?;
                let r = summary::compute(&self.book, &d)?;
                let locale = ironcalc_base::locale::get_locale("en").map_err(|e| e.to_string())?;
                let shown: Vec<Vec<String>> = r
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(k, line)| {
                        line.iter()
                            .enumerate()
                            .map(|(c, v)| match v {
                                CellData::Number(n) if k > 0 => {
                                    let fmt = r.formats.get(c).map(String::as_str).unwrap_or("general");
                                    ironcalc_base::formatter::format::format_number(*n, fmt, locale).text
                                }
                                CellData::Number(n) => n.to_string(),
                                CellData::Text(s) => s.clone(),
                                _ => String::new(),
                            })
                            .collect()
                    })
                    .collect();
                json!({ "rows": shown, "totals": r.totals })
            }
            "add_summary" => {
                let d: summary::Def = from_json(&a["def"])?;
                let id = self.book.next_sheet_id();
                let name = sheets::free_name(&self.book, a["name"].as_str().unwrap_or("Summary"));
                let at = self.book.model.workbook.worksheets.len() as u32;
                let info = SheetInfo { kind: SheetKind::Summary, working: false };
                self.edit(&[Op::AddSheet { sheet: id, name, at, info }, Op::Summary { sheet: id, def: Some(d) }])?;
                json!(id)
            }
            "summary" => json!(self.book.summaries.get(&arg_u32(a, "sheet")?)),
            // Query sheets: the app runs the query on the server, then hands the answer here.
            "add_query" => {
                let d: query::Def = from_json(&a["def"])?;
                let answer: query::Answer = from_json(&a["answer"])?;
                let id = self.book.next_sheet_id();
                let wanted = a["name"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(if d.query_name.is_empty() { "Query" } else { &d.query_name });
                let name = sheets::free_name(&self.book, wanted);
                let at = self.book.model.workbook.worksheets.len() as u32;
                let info = SheetInfo { kind: SheetKind::Query, working: a["working"].as_bool().unwrap_or(false) };
                // The sheet first (its table needs it there), then the answer.
                self.edit(&[Op::AddSheet { sheet: id, name, at, info }])?;
                match query::fill(&self.book, id, d, &answer) {
                    Ok(ops) => {
                        let undo = ops::apply_all(&mut self.book, &ops)?;
                        self.made(&ops);
                        // One step to undo: adding the sheet and its answer.
                        if let Some(last) = self.undo.last_mut() {
                            let mut both = undo;
                            both.extend(std::mem::take(last));
                            *last = both;
                        }
                    }
                    Err(e) => {
                        self.undo()?;
                        self.redo.pop();
                        return Err(e);
                    }
                }
                json!(id)
            }
            "fill_query" => {
                let sheet = arg_u32(a, "sheet")?;
                let mut d = match a.get("def").filter(|v| !v.is_null()) {
                    Some(v) => from_json::<query::Def>(v)?,
                    None => self.book.queries.get(&sheet).cloned().ok_or("this sheet isn't a query")?,
                };
                if let Some(t) = a["refreshed"].as_str() {
                    d.refreshed = t.to_string();
                }
                let answer: query::Answer = from_json(&a["answer"])?;
                let ops = query::fill(&self.book, sheet, d, &answer)?;
                self.edit(&ops)?;
                json!(true)
            }
            "query" => {
                let sheet = arg_u32(a, "sheet")?;
                match self.book.queries.get(&sheet) {
                    Some(d) => json!({ "def": d, "params": query::params(&self.book, d) }),
                    None => Json::Null,
                }
            }
            "queries" => json!(query::all(&self.book)),
            // Linked CSV sheets: the app reads the file, then hands its text here.
            "add_csv_link" => {
                let mut d: csvlink::Def = from_json(&a["def"])?;
                if let Some(t) = a["read"].as_str() {
                    d.read = t.to_string();
                }
                let text = arg_str(a, "text")?;
                let id = self.book.next_sheet_id();
                let stem = d.name.rsplit_once('.').map_or(d.name.as_str(), |(s, _)| s);
                let wanted = a["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(stem);
                let clean: String = wanted.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
                let name = sheets::free_name(&self.book, if clean.trim().is_empty() { "Linked" } else { clean.trim() });
                let at = self.book.model.workbook.worksheets.len() as u32;
                let info = SheetInfo { kind: SheetKind::LinkedCsv, working: false };
                let mut ops = vec![Op::AddSheet { sheet: id, name, at, info }];
                let undo_sheet = ops::apply_all(&mut self.book, &ops)?;
                match csvlink::fill(&self.book, id, d, text) {
                    Ok(fill) => {
                        let mut undo = ops::apply_all(&mut self.book, &fill)?;
                        undo.extend(undo_sheet);
                        self.undo.push(undo);
                        self.redo.clear();
                        self.changes += 1;
                        ops.extend(fill);
                        self.made(&ops);
                    }
                    Err(e) => {
                        ops::apply_all(&mut self.book, &undo_sheet)?;
                        return Err(e);
                    }
                }
                json!(id)
            }
            // The file read again: true when that changed the sheet (one
            // step of undo); otherwise only when it was read is noted.
            "fill_csv_link" => {
                let sheet = arg_u32(a, "sheet")?;
                let mut d = self.book.csv_links.get(&sheet).cloned().ok_or("this sheet isn't linked to a file")?;
                if let Some(t) = a["read"].as_str() {
                    d.read = t.to_string();
                }
                let ops = csvlink::fill(&self.book, sheet, d.clone(), arg_str(a, "text")?)?;
                let before = csvlink::values(&self.book, sheet);
                let undo = ops::apply_all(&mut self.book, &ops)?;
                if csvlink::values(&self.book, sheet) == before {
                    ops::apply_all(&mut self.book, &undo)?;
                    let d = match ops.last() {
                        Some(Op::CsvLink { def: Some(d), .. }) => d.clone(),
                        _ => d,
                    };
                    self.book.csv_links.insert(sheet, d);
                    json!(false)
                } else {
                    self.made(&ops);
                    self.undo.push(undo);
                    self.redo.clear();
                    self.changes += 1;
                    json!(true)
                }
            }
            "csv_link" => json!(self.book.csv_links.get(&arg_u32(a, "sheet")?)),
            "csv_links" => json!(csvlink::all(&self.book)),
            // Unlink: a normal sheet with the values it has.
            "unlink_csv" => {
                let sheet = arg_u32(a, "sheet")?;
                if !self.book.csv_links.contains_key(&sheet) {
                    return Err("this sheet isn't linked to a file".to_string());
                }
                let info = SheetInfo { kind: SheetKind::Normal, ..self.book.info(sheet) };
                self.edit(&[Op::CsvLink { sheet, def: None }, Op::SheetInfo { sheet, info }])?;
                json!(true)
            }
            // What Ask (the AI) can see: the book in brief, columns, some rows, a formula worked out.
            "ai_describe" => ask::describe(&self.book),
            // What Write shows of this book (Insert › From Calc).
            "link_snapshot" => link::snapshot(&self.book),
            "ai_columns" | "ai_rows" => {
                let range = if a["range"].is_object() { Some(arg_range(a)?) } else { None };
                let area = ask::area(&self.book, a["table"].as_str(), a["sheet"].as_str(), range)?;
                if name == "ai_columns" {
                    ask::columns(&self.book, &area)?
                } else {
                    ask::rows(
                        &self.book,
                        &area,
                        a["from"].as_u64().unwrap_or(0) as usize,
                        a["count"].as_u64().unwrap_or(20) as usize,
                    )?
                }
            }
            "ai_calc" => ask::calculate(&mut self.book, a["sheet"].as_str(), arg_str(a, "formula")?)?,
            // Column notes and definitions (Data › Definitions, About this column).
            "notes" => json!(self.book.notes),
            "set_note" => {
                let target: NoteTarget = from_json(&a["target"])?;
                let note: Option<ColumnNote> = if a["note"].is_null() { None } else { Some(from_json(&a["note"])?) };
                self.edit(&[Op::Note { target, note }])?;
                json!(true)
            }
            "definitions" => json!(self.book.definitions.values().collect::<Vec<_>>()),
            "set_definition" => {
                let name = arg_str(a, "name")?.trim().to_string();
                let definition: Option<Definition> =
                    if a["definition"].is_null() { None } else { Some(from_json(&a["definition"])?) };
                self.edit(&[Op::Definition { name, definition }])?;
                json!(true)
            }
            // Table editor sheets.
            "add_editor" => {
                let d: editor::Def = from_json(&a["def"])?;
                let answer: query::Answer = from_json(&a["answer"])?;
                let id = self.book.next_sheet_id();
                let wanted = a["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(&d.table).to_string();
                let name = sheets::free_name(&self.book, &wanted);
                let at = self.book.model.workbook.worksheets.len() as u32;
                let info = SheetInfo { kind: SheetKind::TableEditor, working: false };
                let mut ops = vec![Op::AddSheet { sheet: id, name, at, info }];
                let undo_sheet = ops::apply_all(&mut self.book, &ops)?;
                match editor::fill(&self.book, id, d, &answer) {
                    Ok(fill) => {
                        let mut undo = ops::apply_all(&mut self.book, &fill)?;
                        undo.extend(undo_sheet);
                        self.undo.push(undo);
                        self.redo.clear();
                        self.changes += 1;
                        ops.extend(fill);
                        self.made(&ops);
                    }
                    Err(e) => {
                        ops::apply_all(&mut self.book, &undo_sheet)?;
                        return Err(e);
                    }
                }
                editor::settle(&mut self.book, id);
                json!(id)
            }
            "fill_editor" => {
                let sheet = arg_u32(a, "sheet")?;
                let mut d = match a.get("def").filter(|v| !v.is_null()) {
                    Some(v) => from_json::<editor::Def>(v)?,
                    None => self.book.editors.get(&sheet).cloned().ok_or("this sheet isn't a table editor")?,
                };
                let kept = self.book.editors.get(&sheet).cloned().ok_or("this sheet isn't a table editor")?;
                if !a["force"].as_bool().unwrap_or(false) && !editor::changes(&self.book, sheet, &kept).is_empty() {
                    return Err("unsent".to_string());
                }
                if let Some(t) = a["refreshed"].as_str() {
                    d.refreshed = t.to_string();
                }
                let answer: query::Answer = from_json(&a["answer"])?;
                let ops = editor::fill(&self.book, sheet, d, &answer)?;
                self.edit_unchecked(&ops)?;
                editor::settle(&mut self.book, sheet);
                json!(true)
            }
            "editor" => editor::state(&self.book, arg_u32(a, "sheet")?),
            "editors" => json!(editor::all(&self.book)),
            "editor_sent" => {
                let sheet = arg_u32(a, "sheet")?;
                let changes: Vec<editor::Change> = from_json(&a["changes"])?;
                let outcomes: Vec<Json> = from_json(&a["outcomes"])?;
                let ops = editor::sent(&mut self.book, sheet, &changes, &outcomes)?;
                // New rows' keys: part of what was saved, not something to undo.
                ops::apply_all(&mut self.book, &ops)?;
                self.made(&ops);
                self.changes += 1;
                json!(true)
            }
            "editor_resolve" => {
                let sheet = arg_u32(a, "sheet")?;
                let key: std::collections::BTreeMap<String, Json> = from_json(&a["key"])?;
                let ops = editor::resolve(&mut self.book, sheet, &key, arg_str(a, "column")?, arg_str(a, "choice")?)?;
                self.edit_unchecked(&ops)?;
                self.changes += 1;
                json!(true)
            }
            "set_query" => {
                let sheet = arg_u32(a, "sheet")?;
                if !self.book.queries.contains_key(&sheet) {
                    return Err("this sheet isn't a query".to_string());
                }
                self.edit(&[Op::Query { sheet, def: Some(from_json(&a["def"])?) }])?;
                json!(true)
            }
            // The values a query's :names would take now (a query being changed).
            "query_values" => json!(query::params(&self.book, &from_json::<query::Def>(&a["def"])?)),
            // A table's name in formulas (formulas using it follow).
            "rename_table" => {
                let name = arg_str(a, "name")?;
                let to = arg_str(a, "to")?.trim();
                let mut t = tables::get(&self.book, name).ok_or("there's no table by that name")?;
                if to == t.name {
                    return Ok(json!(t.name));
                }
                if !tables::valid_name(to) || !to.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
                    return Err(
                        "A table's name is letters, digits and _, starting with a letter (no spaces)".to_string()
                    );
                }
                if !to.eq_ignore_ascii_case(&t.name) && tables::free_name(&self.book, to) != to {
                    return Err(format!("There's already something called {to}"));
                }
                let old = t.name.clone();
                t.name = to.to_string();
                // Query and table editor sheets, charts, summaries… follow (ops::rename_table).
                self.edit(&[Op::Table { name: old, table: Some(t.clone()) }])?;
                json!(t.name)
            }
            "add_report" => {
                let id = self.book.next_sheet_id();
                let name = sheets::free_name(&self.book, a["name"].as_str().unwrap_or("Report"));
                let at = self.book.model.workbook.worksheets.len() as u32;
                let info = SheetInfo { kind: SheetKind::Report, working: false };
                // From the table the cursor is in (`table`): a ready report; else just a title.
                let def = match a["table"].as_str().and_then(|t| tables::get(&self.book, t)) {
                    Some(t) => report::from_table(&self.book, &t, &name),
                    None => report::Def { blocks: vec![report::Block::Title { text: name.clone() }], landscape: false },
                };
                self.edit(&[Op::AddSheet { sheet: id, name, at, info }, Op::Report { sheet: id, def: Some(def) }])?;
                json!(id)
            }
            "report" => report::page(&self.book, arg_u32(a, "sheet")?)?,
            "set_report" => {
                let d: report::Def = from_json(&a["def"])?;
                self.edit(&[Op::Report { sheet: arg_u32(a, "sheet")?, def: Some(d) }])?;
                json!(true)
            }
            "set_summary" => {
                let d: summary::Def = from_json(&a["def"])?;
                self.edit(&[Op::Summary { sheet: arg_u32(a, "sheet")?, def: Some(d) }])?;
                json!(true)
            }
            "sheet_facts" => json!(sheets::facts(&self.book)),
            "duplicate_sheet" => {
                let (id, ops) = sheets::duplicate(&self.book, arg_u32(a, "sheet")?)?;
                self.edit(&ops)?;
                json!(id)
            }
            "layout" => self.layout(arg_u32(a, "sheet")?)?,
            "edge" => {
                let (sheet, row, col) = (arg_u32(a, "sheet")?, arg_i32(a, "row")?, arg_i32(a, "col")?);
                let (dr, dc) = (arg_i32(a, "dr")?, arg_i32(a, "dc")?);
                let (r, c) = self.edge(sheet, row, col, dr, dc)?;
                json!({ "row": r, "col": c })
            }
            "save" => json!(base64::engine::general_purpose::STANDARD.encode(self.save()?)),
            "about_file" => {
                // What a CSV can't hold, for the warning before saving one.
                let sheets = self.book.model.workbook.worksheets.len();
                let formatted = self.book.model.workbook.styles.cell_xfs.len() > 1;
                json!({
                    "format": self.format,
                    "kept": xlsx::kept_summary(&self.book),
                    "macros": xlsx::has_macros(&self.book),
                    "sheets": sheets,
                    "tables": tables::all(&self.book).len(),
                    "charts": self.book.charts.len(),
                    "formatted": formatted,
                })
            }
            _ => return Err(format!("unknown call {name}")),
        })
    }

    /// What the grid needs to place rows and columns: sizes that aren't the
    /// normal ones (hidden ones are 0), frozen panes and the part in use.
    fn layout(&self, sheet: u32) -> Result<Json, String> {
        let b = &self.book;
        let i = b.index(sheet)?;
        let w = &b.model.workbook.worksheets[i as usize];
        let rows: Vec<Json> = w
            .rows
            .iter()
            .filter(|r| r.custom_height || r.hidden)
            .map(|r| json!([r.r, if r.hidden { 0.0 } else { b.model.get_row_height(i, r.r).unwrap_or(25.0) }]))
            .collect();
        let mut cols = Vec::new();
        for c in w.cols.iter().filter(|c| c.custom_width || c.hidden) {
            let width = if c.hidden { 0.0 } else { b.model.get_column_width(i, c.min).unwrap_or(90.0) };
            cols.push(json!([c.min, c.max.min(16384), width]));
        }
        let d = w.dimension();
        let info = b.info(sheet);
        Ok(json!({
            "rows": rows, "cols": cols, "row_height": 25.0, "col_width": 90.0,
            "frozen": [w.frozen_rows, w.frozen_columns], "grid": w.show_grid_lines,
            "used": [d.max_row, d.max_column], "kind": info.kind, "working": info.working,
            "merged": w.merged_cells.iter().map(|m| [m.row, m.column, m.width, m.height]).collect::<Vec<_>>(),
            "choices": xlsx::choices(b, sheet),
            "tables": tables::all(b).into_iter().filter(|t| t.sheet == sheet).collect::<Vec<_>>(),
            "filters": b.filters,
        }))
    }

    /// Where Ctrl+arrow goes from (`row`, `col`) in direction (`dr`, `dc`):
    /// the end of the run of filled (or empty) cells, as in other spreadsheets.
    fn edge(&self, sheet: u32, row: i32, col: i32, dr: i32, dc: i32) -> Result<(i32, i32), String> {
        let b = &self.book;
        let i = b.index(sheet)?;
        let filled = |r: i32, c: i32| !b.model.is_empty_cell(i, r, c).unwrap_or(true);
        let d = b.model.workbook.worksheets[i as usize].dimension();
        let (max_r, max_c) = (1_048_576, 16_384);
        let inside = |r: i32, c: i32| r >= 1 && c >= 1 && r <= max_r && c <= max_c;
        let (mut r, mut c) = (row, col);
        if !inside(r + dr, c + dc) {
            return Ok((r, c));
        }
        let here = filled(r, c);
        let next = filled(r + dr, c + dc);
        if here && next {
            // Along the filled run to its last cell.
            while inside(r + dr, c + dc) && filled(r + dr, c + dc) {
                r += dr;
                c += dc;
            }
        } else {
            // To the next filled cell, or the sheet's edge.
            r += dr;
            c += dc;
            while inside(r, c) && !filled(r, c) {
                let past = (dr > 0 && r > d.max_row) || (dc > 0 && c > d.max_column);
                if past {
                    r = if dr > 0 { max_r } else { r };
                    c = if dc > 0 { max_c } else { c };
                    break;
                }
                if !inside(r + dr, c + dc) {
                    break;
                }
                r += dr;
                c += dc;
            }
        }
        Ok((r.clamp(1, max_r), c.clamp(1, max_c)))
    }

    /// "Sheet2", "Sheet3"… whichever is free.
    pub fn free_sheet_name(&self) -> String {
        let names: Vec<String> =
            self.book.model.workbook.worksheets.iter().map(|w| w.get_name().to_lowercase()).collect();
        (1..).map(|k| format!("Sheet{k}")).find(|n| !names.contains(&n.to_lowercase())).unwrap_or_default()
    }
}
