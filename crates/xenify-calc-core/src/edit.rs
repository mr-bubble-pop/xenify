//! Everyday edits on a selection, each worked out as [`Op`]s: formats,
//! clearing, sorting, filtering a table, find and replace, copy and paste.
//! And what the screens show about a selection: its numbers' summary and
//! each cell's look.

use crate::book::{Book, CellData};
use crate::ops::{CellEdit, LineSize, Op, StyleEdit};
use crate::tables;
use ironcalc_base::cell::CellValue;
use ironcalc_base::expressions::types::CellReferenceIndex;
use ironcalc_base::types::{Alignment, BorderItem, BorderStyle, Color, HorizontalAlignment, Style, VerticalAlignment};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

/// Rows `top..=bottom`, columns `left..=right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
}

impl Range {
    pub fn cell(row: i32, col: i32) -> Range {
        Range { top: row, left: col, bottom: row, right: col }
    }

    /// In order (a drag can end above or left of where it started).
    pub fn normal(self) -> Range {
        Range {
            top: self.top.min(self.bottom),
            left: self.left.min(self.right),
            bottom: self.top.max(self.bottom),
            right: self.left.max(self.right),
        }
    }

    fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        (self.top..=self.bottom).flat_map(move |r| (self.left..=self.right).map(move |c| (r, c)))
    }

    fn count(self) -> i64 {
        (self.bottom - self.top + 1) as i64 * (self.right - self.left + 1) as i64
    }
}

/// The most cells one edit changes cell by cell (a whole column is cut to
/// the part in use).
const MOST: i64 = 200_000;

/// `range` cut to the rows and columns in use on `sheet` (plus the part of
/// it near the top left, so formatting empty cells still works).
fn used(book: &Book, sheet: u32, range: Range) -> Range {
    let r = range.normal();
    if r.count() <= MOST {
        return r;
    }
    let Ok(i) = book.index(sheet) else { return r };
    let d = book.model.workbook.worksheets[i as usize].dimension();
    Range {
        top: r.top,
        left: r.left,
        bottom: r.bottom.min(d.max_row.max(r.top + 99)),
        right: r.right.min(d.max_column.max(r.left + 25)),
    }
}

/// What a format change sets; anything left out stays as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FormatChange {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub size: Option<i32>,
    pub font: Option<String>,
    /// `#RRGGBB`, or "" for automatic.
    pub color: Option<String>,
    pub fill: Option<String>,
    pub num_fmt: Option<String>,
    /// left, center, right or general.
    pub align: Option<String>,
    /// top, center or bottom.
    pub valign: Option<String>,
    pub wrap: Option<bool>,
    /// all, outside, none, bottom, top, left, right.
    pub border: Option<String>,
    /// One more (+1) or one fewer (-1) decimal place.
    pub decimals: Option<i32>,
}

fn colour(s: &str) -> Color {
    if s.is_empty() { Color::None } else { Color::Rgb(s.to_uppercase()) }
}

fn h_align(s: &str) -> HorizontalAlignment {
    match s {
        "left" => HorizontalAlignment::Left,
        "center" => HorizontalAlignment::Center,
        "right" => HorizontalAlignment::Right,
        _ => HorizontalAlignment::General,
    }
}

fn v_align(s: &str) -> VerticalAlignment {
    match s {
        "top" => VerticalAlignment::Top,
        "center" => VerticalAlignment::Center,
        _ => VerticalAlignment::Bottom,
    }
}

/// "0.00" → "0.000" (more) or "0.0" (fewer); "General" becomes "0.0"…
pub fn change_decimals(fmt: &str, by: i32) -> String {
    let base = if fmt.eq_ignore_ascii_case("general") { "0" } else { fmt };
    // The first number part (before any ';' section) decides.
    let (head, rest) = match base.find(';') {
        Some(k) => (&base[..k], &base[k..]),
        None => (base, ""),
    };
    let last_zero = head.rfind(['0', '#']);
    let Some(z) = last_zero else { return fmt.to_string() };
    let (before, after) = head.split_at(z + 1);
    let new_head = match before.find('.') {
        Some(dot) => {
            let places = before.len() - dot - 1;
            let n = (places as i32 + by).max(0) as usize;
            if n == 0 {
                format!("{}{after}", &before[..dot])
            } else {
                format!("{}.{}{after}", &before[..dot], "0".repeat(n))
            }
        }
        None if by > 0 => format!("{before}.{}{after}", "0".repeat(by as usize)),
        None => head.to_string(),
    };
    format!("{new_head}{rest}")
}

fn thin() -> Option<BorderItem> {
    Some(BorderItem { style: BorderStyle::Thin, color: Color::Rgb("#1C1917".to_string()) })
}

/// The ops that apply `change` to every cell of `range`.
pub fn format(book: &Book, sheet: u32, range: Range, change: &FormatChange) -> Vec<Op> {
    let r = used(book, sheet, range);
    let mut cells = Vec::new();
    for (row, col) in r.cells() {
        let mut s: Style = book.style(sheet, row, col);
        let before = s.clone();
        if let Some(v) = change.bold {
            s.font.b = v;
        }
        if let Some(v) = change.italic {
            s.font.i = v;
        }
        if let Some(v) = change.underline {
            s.font.u = v;
        }
        if let Some(v) = change.strike {
            s.font.strike = v;
        }
        if let Some(v) = change.size {
            s.font.sz = v.clamp(4, 400);
        }
        if let Some(v) = &change.font {
            s.font.name = v.clone();
        }
        if let Some(v) = &change.color {
            s.font.color = colour(v);
        }
        if let Some(v) = &change.fill {
            s.fill.color = colour(v);
        }
        if let Some(v) = &change.num_fmt {
            s.num_fmt = v.clone();
        }
        if let Some(by) = change.decimals {
            s.num_fmt = change_decimals(&s.num_fmt, by);
        }
        if change.align.is_some() || change.valign.is_some() || change.wrap.is_some() {
            let mut a = s.alignment.clone().unwrap_or_default();
            if let Some(v) = &change.align {
                a.horizontal = h_align(v);
            }
            if let Some(v) = &change.valign {
                a.vertical = v_align(v);
            }
            if let Some(v) = change.wrap {
                a.wrap_text = v;
            }
            s.alignment = (a != Alignment::default()).then_some(a);
        }
        if let Some(b) = &change.border {
            let (top, bottom, left, right) = (row == r.top, row == r.bottom, col == r.left, col == r.right);
            let bd = &mut s.border;
            match b.as_str() {
                "none" => {
                    bd.top = None;
                    bd.bottom = None;
                    bd.left = None;
                    bd.right = None;
                }
                "all" => {
                    bd.top = thin();
                    bd.bottom = thin();
                    bd.left = thin();
                    bd.right = thin();
                }
                "outside" => {
                    if top {
                        bd.top = thin();
                    }
                    if bottom {
                        bd.bottom = thin();
                    }
                    if left {
                        bd.left = thin();
                    }
                    if right {
                        bd.right = thin();
                    }
                }
                "top" if top => bd.top = thin(),
                "bottom" if bottom => bd.bottom = thin(),
                "left" if left => bd.left = thin(),
                "right" if right => bd.right = thin(),
                _ => {}
            }
        }
        if s != before {
            cells.push(StyleEdit { row, col, style: s });
        }
    }
    if cells.is_empty() { Vec::new() } else { vec![Op::Styles { sheet, cells }] }
}

/// The ops that merge `range` into one cell: the top left one's content
/// stays, the others' goes (`keep_all` false refuses when there's more).
pub fn merge(book: &Book, sheet: u32, range: Range) -> Result<Vec<Op>, String> {
    let r = range.normal();
    if r.top == r.bottom && r.left == r.right {
        return Err("Pick more than one cell to merge".to_string());
    }
    let i = book.index(sheet)?;
    let w = &book.model.workbook.worksheets[i as usize];
    let (width, height) = (r.right - r.left + 1, r.bottom - r.top + 1);
    // Merges it overlaps are taken into it.
    let mut list: Vec<ironcalc_base::types::MergedCell> =
        w.merged_cells.iter().filter(|m| !m.intersects(r.top, r.left, width, height)).copied().collect();
    list.push(ironcalc_base::types::MergedCell { row: r.top, column: r.left, width, height });
    let covered: Vec<CellEdit> = book
        .cells_in(sheet, Some((r.top, r.bottom)), Some((r.left, r.right)))
        .into_iter()
        .filter(|c| c.2.is_some() && !(c.0 == r.top && c.1 == r.left))
        .map(|c| CellEdit { row: c.0, col: c.1, data: None })
        .collect();
    let mut ops = Vec::new();
    // What's in the other cells moves to the top left when that's empty.
    if book.cell_data(sheet, r.top, r.left).is_none()
        && let Some(first) =
            book.cells_in(sheet, Some((r.top, r.bottom)), Some((r.left, r.right))).into_iter().find(|c| c.2.is_some())
    {
        ops.push(Op::Cells { sheet, cells: vec![CellEdit { row: r.top, col: r.left, data: first.2 }] });
    }
    if !covered.is_empty() {
        ops.push(Op::Cells { sheet, cells: covered });
    }
    ops.push(Op::Merged { sheet, cells: list });
    Ok(ops)
}

/// The ops that split the merged cells `range` touches.
pub fn unmerge(book: &Book, sheet: u32, range: Range) -> Result<Vec<Op>, String> {
    let r = range.normal();
    let i = book.index(sheet)?;
    let w = &book.model.workbook.worksheets[i as usize];
    let (width, height) = (r.right - r.left + 1, r.bottom - r.top + 1);
    let list: Vec<_> = w.merged_cells.iter().filter(|m| !m.intersects(r.top, r.left, width, height)).copied().collect();
    if list.len() == w.merged_cells.len() {
        return Ok(Vec::new());
    }
    Ok(vec![Op::Merged { sheet, cells: list }])
}

/// What Clear takes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Clear {
    Contents,
    Formats,
    All,
}

pub fn clear(book: &Book, sheet: u32, range: Range, what: Clear) -> Vec<Op> {
    let r = used(book, sheet, range);
    let mut ops = Vec::new();
    let present = book.cells_in(sheet, Some((r.top, r.bottom)), Some((r.left, r.right)));
    if what != Clear::Formats {
        let cells: Vec<CellEdit> =
            present.iter().filter(|c| c.2.is_some()).map(|c| CellEdit { row: c.0, col: c.1, data: None }).collect();
        if !cells.is_empty() {
            ops.push(Op::Cells { sheet, cells });
        }
    }
    if what != Clear::Contents {
        let plain = Style::default();
        let cells: Vec<StyleEdit> = present
            .iter()
            .filter(|c| c.3 != plain)
            .map(|c| StyleEdit { row: c.0, col: c.1, style: plain.clone() })
            .collect();
        if !cells.is_empty() {
            ops.push(Op::Styles { sheet, cells });
        }
    }
    ops
}

/// One column to sort by.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct SortKey {
    pub col: i32,
    #[serde(default)]
    pub desc: bool,
}

/// What a cell holds, for sorting: numbers, then words, then true/false,
/// then errors; empty cells always last.
#[derive(PartialEq, PartialOrd)]
enum SortValue {
    Number(f64),
    Text(String),
    Bool(bool),
    Error,
    Empty,
}

fn sort_value(book: &Book, index: u32, row: i32, col: i32) -> SortValue {
    match book.model.get_cell_value_by_index(index, row, col) {
        Ok(CellValue::Number(n)) => SortValue::Number(n),
        Ok(CellValue::String(s)) if s.is_empty() => SortValue::Empty,
        Ok(CellValue::String(s))
            if s.starts_with('#')
                && book.model.get_formatted_cell_value(index, row, col).is_ok_and(|t| t == s)
                && is_error(&s) =>
        {
            SortValue::Error
        }
        Ok(CellValue::String(s)) => SortValue::Text(s.to_lowercase()),
        Ok(CellValue::Boolean(b)) => SortValue::Bool(b),
        _ => SortValue::Empty,
    }
}

fn is_error(s: &str) -> bool {
    matches!(
        s,
        "#N/A"
            | "#VALUE!"
            | "#REF!"
            | "#DIV/0!"
            | "#NAME?"
            | "#NUM!"
            | "#NULL!"
            | "#ERROR!"
            | "#SPILL!"
            | "#CALC!"
            | "#CIRC!"
    )
}

/// The ops that sort the rows of `range` by `keys` (the first row stays
/// on top when `header`). Formulas move with their row, as if copied there.
/// Inside a table, its data rows are sorted.
pub fn sort(book: &mut Book, sheet: u32, range: Range, keys: &[SortKey], header: bool) -> Result<Vec<Op>, String> {
    let mut r = used(book, sheet, range);
    if let Some(t) = tables::at(book, sheet, r.top, r.left) {
        r = Range { top: t.top + t.header as i32, left: t.left, bottom: t.bottom - t.totals as i32, right: t.right };
    } else if header {
        r.top += 1;
    }
    if r.bottom <= r.top || keys.is_empty() {
        return Ok(Vec::new());
    }
    let i = book.index(sheet)?;
    let rows: Vec<i32> = (r.top..=r.bottom).collect();
    let values: Vec<Vec<SortValue>> =
        rows.iter().map(|row| keys.iter().map(|k| sort_value(book, i, *row, k.col)).collect()).collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|a, b| {
        for (k, key) in keys.iter().enumerate() {
            let (x, y) = (&values[*a][k], &values[*b][k]);
            // Empty cells stay last either way.
            let o = match (x, y) {
                (SortValue::Empty, SortValue::Empty) => std::cmp::Ordering::Equal,
                (SortValue::Empty, _) => std::cmp::Ordering::Greater,
                (_, SortValue::Empty) => std::cmp::Ordering::Less,
                _ => {
                    let o = x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal);
                    if key.desc { o.reverse() } else { o }
                }
            };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        a.cmp(b)
    });
    if order.iter().enumerate().all(|(k, v)| k == *v) {
        return Ok(Vec::new());
    }
    let mut cells = Vec::new();
    let mut styles = Vec::new();
    let sources: Vec<i32> = order.iter().map(|k| rows[*k]).collect();
    for (k, from) in sources.iter().enumerate() {
        let to = r.top + k as i32;
        for col in r.left..=r.right {
            let data = match book.cell_data(sheet, *from, col) {
                Some(CellData::Formula(f)) => Some(CellData::Formula(moved(book, &f, i, *from, col, to, col)?)),
                d => d,
            };
            if data != book.cell_data(sheet, to, col) {
                cells.push(CellEdit { row: to, col, data });
            }
            let style = book.style(sheet, *from, col);
            if style != book.style(sheet, to, col) {
                styles.push(StyleEdit { row: to, col, style });
            }
        }
    }
    let mut ops = Vec::new();
    if !styles.is_empty() {
        ops.push(Op::Styles { sheet, cells: styles });
    }
    if !cells.is_empty() {
        ops.push(Op::Cells { sheet, cells });
    }
    Ok(ops)
}

/// Formula `f` in (`row`, `col`) as it reads once copied to (`to_row`, `to_col`).
fn moved(book: &mut Book, f: &str, index: u32, row: i32, col: i32, to_row: i32, to_col: i32) -> Result<String, String> {
    let source = CellReferenceIndex { sheet: index, row, column: col };
    let target = CellReferenceIndex { sheet: index, row: to_row, column: to_col };
    book.model.extend_copied_value(f, &source, &target)
}

/// A table column's filter: rows whose cell shows one of `keep` stay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnFilter {
    pub column: String,
    pub keep: Vec<String>,
}

/// The values a table column shows, each once, in order, with how many rows
/// show it (for the filter list).
pub fn column_values(book: &Book, table: &str, column: &str) -> Result<Vec<(String, usize)>, String> {
    let t = tables::get(book, table).ok_or_else(|| format!("no table {table}"))?;
    let k = t
        .columns
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(column))
        .ok_or_else(|| format!("no column {column}"))?;
    let i = book.index(t.sheet)?;
    let mut out: Vec<(String, usize)> = Vec::new();
    for row in t.top + t.header as i32..=t.bottom - t.totals as i32 {
        let v = book.model.get_formatted_cell_value(i, row, t.left + k as i32).unwrap_or_default();
        match out.iter_mut().find(|(x, _)| *x == v) {
            Some(e) => e.1 += 1,
            None => out.push((v, 1)),
        }
    }
    out.sort_by(|a, b| natural(&a.0, &b.0));
    Ok(out)
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.replace(',', "").parse::<f64>(), b.replace(',', "").parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        _ => {
            if a.is_empty() != b.is_empty() {
                return if a.is_empty() { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
            }
            a.to_lowercase().cmp(&b.to_lowercase())
        }
    }
}

/// The ops that set the filter on `column` of `table` (none: show every
/// row again) and hide or show its rows to match all its filters.
pub fn filter(book: &Book, table: &str, column: &str, keep: Option<Vec<String>>) -> Result<Vec<Op>, String> {
    let t = tables::get(book, table).ok_or_else(|| format!("no table {table}"))?;
    let mut filters = book.filters.get(&t.name).cloned().unwrap_or_default();
    filters.retain(|f| !f.column.eq_ignore_ascii_case(column));
    if let Some(keep) = keep {
        filters.push(ColumnFilter { column: column.to_string(), keep });
    }
    let i = book.index(t.sheet)?;
    let mut rows = Vec::new();
    for row in t.top + t.header as i32..=t.bottom - t.totals as i32 {
        let hide = filters.iter().any(|f| {
            let Some(k) = t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(&f.column)) else { return false };
            let v = book.model.get_formatted_cell_value(i, row, t.left + k as i32).unwrap_or_default();
            !f.keep.contains(&v)
        });
        let now = book.model.is_row_hidden(i, row).unwrap_or(false);
        if hide != now {
            rows.push(LineSize { hidden: hide, ..crate::ops::row_size(book, t.sheet, row) });
        }
    }
    let mut ops = vec![Op::Filter { table: t.name.clone(), filters }];
    if !rows.is_empty() {
        ops.push(Op::Rows { sheet: t.sheet, rows });
    }
    Ok(ops)
}

/// Find: where to look and how to match.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FindOptions {
    pub match_case: bool,
    /// The whole cell, not part of it.
    pub whole: bool,
    /// Look in formulas as typed (else in what cells show).
    pub formulas: bool,
    /// Only this sheet (else every sheet, this one first).
    pub sheet: Option<u32>,
}

/// One cell Find found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub sheet: u32,
    pub row: i32,
    pub col: i32,
    pub text: String,
}

pub fn find(book: &Book, query: &str, o: &FindOptions) -> Vec<Found> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = if o.match_case { query.to_string() } else { query.to_lowercase() };
    let mut out = Vec::new();
    for (i, w) in book.model.workbook.worksheets.iter().enumerate() {
        if o.sheet.is_some_and(|s| s != w.sheet_id) {
            continue;
        }
        let mut cells: Vec<(i32, i32)> = w.sheet_data.cells().map(|(r, c, _)| (r, c)).collect();
        cells.sort();
        for (row, col) in cells {
            let text = if o.formulas {
                book.model.get_localized_cell_content(i as u32, row, col).unwrap_or_default()
            } else {
                book.model.get_formatted_cell_value(i as u32, row, col).unwrap_or_default()
            };
            let t = if o.match_case { text.clone() } else { text.to_lowercase() };
            if (o.whole && t == q) || (!o.whole && t.contains(&q)) {
                out.push(Found { sheet: w.sheet_id, row, col, text });
            }
        }
    }
    out
}

/// The ops that replace `query` with `with` in what was typed in each
/// cell Find finds (formulas included when looking in formulas).
pub fn replace(book: &Book, query: &str, with: &str, o: &FindOptions) -> Vec<Op> {
    let o = FindOptions { formulas: true, ..o.clone() };
    let mut by_sheet: Vec<(u32, Vec<CellEdit>)> = Vec::new();
    for f in find(book, query, &o) {
        let Ok(i) = book.index(f.sheet) else { continue };
        let typed = book.model.get_localized_cell_content(i, f.row, f.col).unwrap_or_default();
        let new = if o.whole {
            with.to_string()
        } else if o.match_case {
            typed.replace(query, with)
        } else {
            replace_any_case(&typed, query, with)
        };
        if new == typed {
            continue;
        }
        let edit = CellEdit { row: f.row, col: f.col, data: Some(CellData::Input(new)) };
        match by_sheet.iter_mut().find(|(s, _)| *s == f.sheet) {
            Some((_, v)) => v.push(edit),
            None => by_sheet.push((f.sheet, vec![edit])),
        }
    }
    by_sheet.into_iter().map(|(sheet, cells)| Op::Cells { sheet, cells }).collect()
}

fn replace_any_case(text: &str, query: &str, with: &str) -> String {
    // Compared a character at a time (lowercasing can change a text's length).
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let mut lower = Vec::new();
        let mut end = 0;
        for (k, ch) in rest.char_indices() {
            lower.extend(ch.to_lowercase());
            end = k + ch.len_utf8();
            if lower.len() >= q.len() {
                break;
            }
        }
        if lower == q {
            out.push_str(with);
            rest = &rest[end..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// What Copy keeps: the cells (formulas as typed) and their looks, and
/// where they came from, so pasted formulas point the same way.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub sheet: u32,
    pub top: i32,
    pub left: i32,
    pub cells: Vec<Vec<Option<CellData>>>,
    pub styles: Vec<Vec<Style>>,
    /// Tab-separated text of what the cells show (for other programs).
    pub text: String,
}

pub fn copy(book: &Book, sheet: u32, range: Range) -> Result<Clip, String> {
    let r = used(book, sheet, range);
    if r.count() > MOST {
        return Err("That's too many cells to copy at once".to_string());
    }
    let i = book.index(sheet)?;
    let mut cells = Vec::new();
    let mut styles = Vec::new();
    let mut lines = Vec::new();
    for row in r.top..=r.bottom {
        let mut cr = Vec::new();
        let mut sr = Vec::new();
        let mut tr = Vec::new();
        for col in r.left..=r.right {
            cr.push(book.cell_data(sheet, row, col));
            sr.push(book.style(sheet, row, col));
            let shown = book.model.get_formatted_cell_value(i, row, col).unwrap_or_default();
            tr.push(shown.replace(['\t', '\n'], " "));
        }
        cells.push(cr);
        styles.push(sr);
        lines.push(tr.join("\t"));
    }
    Ok(Clip { sheet, top: r.top, left: r.left, cells, styles, text: lines.join("\n") })
}

/// The ops that paste `clip` with its top left at (`row`, `col`). Formulas
/// move as if copied; `values`: what the cells show instead of formulas.
pub fn paste(book: &mut Book, sheet: u32, row: i32, col: i32, clip: &Clip, values: bool) -> Result<Vec<Op>, String> {
    let from = book.index(clip.sheet).ok();
    let to_index = book.index(sheet)?;
    let mut cells = Vec::new();
    let mut styles = Vec::new();
    for (dr, line) in clip.cells.iter().enumerate() {
        for (dc, data) in line.iter().enumerate() {
            let (r, c) = (row + dr as i32, col + dc as i32);
            let (sr, sc) = (clip.top + dr as i32, clip.left + dc as i32);
            let data = match data {
                Some(CellData::Formula(f)) if values => match from {
                    Some(fi) => {
                        Some(CellData::Input(book.model.get_formatted_cell_value(fi, sr, sc).unwrap_or_default()))
                    }
                    None => Some(CellData::Formula(f.clone())),
                },
                Some(CellData::Formula(f)) => {
                    let source = CellReferenceIndex { sheet: from.unwrap_or(to_index), row: sr, column: sc };
                    let target = CellReferenceIndex { sheet: to_index, row: r, column: c };
                    Some(CellData::Formula(book.model.extend_copied_value(f, &source, &target)?))
                }
                d => d.clone(),
            };
            cells.push(CellEdit { row: r, col: c, data });
            if !values {
                styles.push(StyleEdit { row: r, col: c, style: clip.styles[dr][dc].clone() });
            }
        }
    }
    let mut ops = Vec::new();
    if !styles.is_empty() {
        ops.push(Op::Styles { sheet, cells: styles });
    }
    ops.push(Op::Cells { sheet, cells });
    Ok(ops)
}

/// The ops that paste text from another program: lines of tab-separated
/// cells, read the way typing is.
pub fn paste_text(sheet: u32, row: i32, col: i32, text: &str) -> Vec<Op> {
    let mut cells = Vec::new();
    for (dr, line) in text.trim_end_matches(['\n', '\r']).split('\n').enumerate() {
        for (dc, v) in line.trim_end_matches('\r').split('\t').enumerate() {
            let data = (!v.is_empty()).then(|| CellData::Input(v.to_string()));
            cells.push(CellEdit { row: row + dr as i32, col: col + dc as i32, data });
        }
    }
    vec![Op::Cells { sheet, cells }]
}

/// The numbers in a selection, summed up (for the status bar and the
/// number summary).
pub fn stats(book: &Book, sheet: u32, range: Range) -> Json {
    let r = used(book, sheet, range);
    let Ok(i) = book.index(sheet) else { return json!(null) };
    let mut numbers = Vec::new();
    let mut count = 0usize;
    for (row, col) in
        book.cells_in(sheet, Some((r.top, r.bottom)), Some((r.left, r.right))).into_iter().map(|c| (c.0, c.1))
    {
        if book.model.is_row_hidden(i, row).unwrap_or(false) {
            continue;
        }
        match book.model.get_cell_value_by_index(i, row, col) {
            Ok(CellValue::Number(n)) => {
                numbers.push(n);
                count += 1;
            }
            Ok(CellValue::String(s)) if !s.is_empty() => count += 1,
            Ok(CellValue::Boolean(_)) => count += 1,
            _ => {}
        }
    }
    if numbers.is_empty() {
        return json!({ "count": count, "numbers": 0 });
    }
    let n = numbers.len() as f64;
    let sum: f64 = numbers.iter().sum();
    numbers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| {
        let k = p * (numbers.len() - 1) as f64;
        let (lo, hi) = (k.floor() as usize, k.ceil() as usize);
        numbers[lo] + (numbers[hi] - numbers[lo]) * (k - lo as f64)
    };
    // How the numbers spread: up to 8 bars between round edges.
    let (lo, hi) = (numbers[0], numbers[numbers.len() - 1]);
    let bars = ((n.sqrt() * 2.0).round() as usize).clamp(3, 10);
    let step = nice_step((hi - lo) / bars as f64);
    let start = if step > 0.0 { (lo / step).floor() * step } else { lo };
    let count_bars = if step > 0.0 { (((hi - start) / step).floor() as usize + 1).min(12) } else { 1 };
    let mut bins = vec![0usize; count_bars];
    for v in &numbers {
        let k = if step > 0.0 { (((v - start) / step).floor() as usize).min(count_bars - 1) } else { 0 };
        bins[k] += 1;
    }
    let mean = sum / n;
    let sd = if numbers.len() > 1 {
        (numbers.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    json!({
        "count": count, "numbers": numbers.len(), "sum": sum, "average": mean,
        "min": lo, "max": hi, "median": q(0.5),
        "q1": q(0.25), "q3": q(0.75), "sd": sd,
        "bins": { "start": start, "step": step, "counts": bins },
    })
}

/// A round width near `raw`: 1, 2 or 5 times a power of ten.
fn nice_step(raw: f64) -> f64 {
    if raw <= 0.0 || !raw.is_finite() {
        return 0.0;
    }
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    p * if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 2.5 {
        2.5
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    }
}

/// A cell's look, in the terms the screens draw with.
pub fn look(book: &Book, style: &Style) -> Json {
    let theme = &book.model.workbook.theme;
    let rgb = |c: &Color| c.to_rgb(theme);
    let side = |b: &Option<BorderItem>| {
        b.as_ref().map(|b| {
            let w = match b.style {
                BorderStyle::Medium
                | BorderStyle::MediumDashed
                | BorderStyle::MediumDashDot
                | BorderStyle::MediumDashDotDot => 2,
                BorderStyle::Thick | BorderStyle::Double => 3,
                _ => 1,
            };
            let c = rgb(&b.color);
            json!([w, if c.is_empty() { "#1C1917".to_string() } else { c }])
        })
    };
    let a = style.alignment.clone().unwrap_or_default();
    let mut out = serde_json::Map::new();
    let mut put = |k: &str, v: Json| {
        out.insert(k.to_string(), v);
    };
    if style.font.b {
        put("b", json!(true));
    }
    if style.font.i {
        put("i", json!(true));
    }
    if style.font.u {
        put("u", json!(true));
    }
    if style.font.strike {
        put("s", json!(true));
    }
    if style.font.sz != 12 && style.font.sz > 0 {
        put("size", json!(style.font.sz));
    }
    if style.font.name != "Inter" && !style.font.name.is_empty() {
        put("font", json!(style.font.name));
    }
    let c = rgb(&style.font.color);
    if !c.is_empty() && c != "#000000" {
        put("color", json!(c));
    }
    let f = rgb(&style.fill.color);
    if !f.is_empty() {
        put("fill", json!(f));
    }
    match a.horizontal {
        HorizontalAlignment::Left => put("align", json!("left")),
        HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => put("align", json!("center")),
        HorizontalAlignment::Right => put("align", json!("right")),
        _ => {}
    }
    match a.vertical {
        VerticalAlignment::Top => put("valign", json!("top")),
        VerticalAlignment::Center => put("valign", json!("center")),
        _ => {}
    }
    if a.wrap_text {
        put("wrap", json!(true));
    }
    let b = &style.border;
    let borders = [side(&b.top), side(&b.right), side(&b.bottom), side(&b.left)];
    if borders.iter().any(Option::is_some) {
        put("border", json!(borders));
    }
    if style.num_fmt != "general" && !style.num_fmt.is_empty() {
        put("fmt", json!(style.num_fmt));
    }
    Json::Object(out)
}
