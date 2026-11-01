//! What Ask (the AI) can see of a book, through tools rather than by
//! reading every row: the sheets, tables and their columns (with notes and
//! definitions), a few rows, and any formula worked out here. Only what a
//! question needs goes to the model.

use crate::book::{Book, ColumnNote, NoteTarget, SheetKind};
use crate::edit::Range;
use crate::tables::{self, TableDef};
use ironcalc_base::cell::CellValue;
use serde_json::{Value as Json, json};
use std::collections::BTreeSet;

/// How many rows a tool shows at most.
const MOST_ROWS: usize = 50;

fn kind_name(k: SheetKind) -> &'static str {
    match k {
        SheetKind::Normal => "sheet",
        SheetKind::Query => "query (read-only, from a database)",
        SheetKind::TableEditor => "table editor (a database table people change)",
        SheetKind::Summary => "summary",
        SheetKind::Report => "report",
        SheetKind::LinkedCsv => "linked CSV file (read-only, follows the file)",
    }
}

/// The book in brief: sheets, tables with their columns, definitions.
pub fn describe(book: &Book) -> Json {
    let sheets: Vec<Json> = book
        .model
        .workbook
        .worksheets
        .iter()
        .map(|w| {
            let info = book.info(w.sheet_id);
            let d = w.dimension();
            let mut s = json!({ "name": w.get_name(), "kind": kind_name(info.kind), "rows": d.max_row, "columns": d.max_column });
            if info.working {
                s["working"] = json!(true);
            }
            if let Some(q) = book.queries.get(&w.sheet_id) {
                s["from"] = json!(format!("{} ({})", q.source_name, if q.query_name.is_empty() { "own query" } else { &q.query_name }));
            }
            if let Some(e) = book.editors.get(&w.sheet_id) {
                s["from"] = json!(format!("{} table {}", e.source_name, e.table));
            }
            s
        })
        .collect();
    let tables: Vec<Json> = tables::all(book)
        .iter()
        .map(|t| {
            json!({
                "name": t.name, "sheet": book.sheet_name(t.sheet).unwrap_or_default(),
                "rows": (t.bottom - t.top + 1 - i32::from(t.header) - i32::from(t.totals)).max(0),
                "columns": t.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                "noted": t.columns.iter().filter(|c| note_for_table(book, &t.name, &c.name).is_some()).map(|c| c.name.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let defs: Vec<Json> =
        book.definitions.values().map(|d| json!({ "name": d.name, "formula": d.formula, "about": d.about })).collect();
    json!({ "sheets": sheets, "tables": tables, "definitions": defs })
}

fn note_for_table<'a>(book: &'a Book, table: &str, column: &str) -> Option<&'a ColumnNote> {
    book.notes.iter().find(|n| matches!(&n.target, NoteTarget::Table { table: t, column: c } if t.eq_ignore_ascii_case(table) && c.eq_ignore_ascii_case(column)))
}

fn note_json(n: &ColumnNote) -> Json {
    let mut v = json!({});
    for (k, s) in [
        ("means", &n.meaning),
        ("unit", &n.unit),
        ("worked_out_as", &n.how),
        ("leave_out", &n.leave_out),
        ("comes_from", &n.source),
    ] {
        if !s.is_empty() {
            v[k] = json!(s);
        }
    }
    if let Some(t) = n.with_tax {
        v["includes_tax"] = json!(t);
    }
    if n.key {
        v["identifies_rows"] = json!(true);
    }
    v
}

/// Where to look: a table by name, or a sheet (all it uses, or a range).
pub struct Area {
    pub sheet: u32,
    pub range: Range,
    pub header: bool,
    pub table: Option<TableDef>,
}

pub fn area(book: &Book, table: Option<&str>, sheet: Option<&str>, range: Option<Range>) -> Result<Area, String> {
    if let Some(name) = table.filter(|t| !t.is_empty()) {
        let t = tables::get(book, name).ok_or_else(|| format!("there's no table called {name}"))?;
        let bottom = if t.totals { t.bottom - 1 } else { t.bottom };
        return Ok(Area {
            sheet: t.sheet,
            range: Range { top: t.top, left: t.left, bottom, right: t.right },
            header: t.header,
            table: Some(t),
        });
    }
    let w = match sheet.filter(|s| !s.is_empty()) {
        Some(name) => book
            .model
            .workbook
            .worksheets
            .iter()
            .find(|w| w.get_name().eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("there's no sheet called {name}"))?,
        None => book.model.workbook.worksheets.first().ok_or("the book is empty")?,
    };
    let d = w.dimension();
    let range = range.unwrap_or(Range { top: 1, left: 1, bottom: d.max_row.max(1), right: d.max_column.max(1) });
    let header = tables::has_header(book, w.sheet_id, range.top, range.left, range.bottom, range.right);
    Ok(Area { sheet: w.sheet_id, range, header, table: None })
}

fn value(book: &Book, i: u32, r: i32, c: i32) -> CellValue {
    book.model.get_cell_value_by_index(i, r, c).unwrap_or(CellValue::None)
}

fn shown(book: &Book, i: u32, r: i32, c: i32) -> String {
    book.model.get_formatted_cell_value(i, r, c).unwrap_or_default()
}

/// Each column of an area: its name, what it holds, a few values, totals
/// for numbers, and its note.
pub fn columns(book: &Book, a: &Area) -> Result<Json, String> {
    let i = book.index(a.sheet)?;
    let first = if a.header { a.range.top + 1 } else { a.range.top };
    let last = a.range.bottom;
    let mut out = Vec::new();
    for c in a.range.left..=a.range.right {
        let name = if a.header { shown(book, i, a.range.top, c) } else { crate::tables::column_letters(c) };
        let (mut numbers, mut texts, mut dates, mut empty, mut bools) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let (mut sum, mut min, mut max) = (0.0f64, f64::INFINITY, f64::NEG_INFINITY);
        let mut examples: Vec<String> = Vec::new();
        let mut distinct: BTreeSet<String> = BTreeSet::new();
        for r in first..=last {
            let v = value(book, i, r, c);
            let text = shown(book, i, r, c);
            match v {
                CellValue::None => empty += 1,
                CellValue::Number(n) => {
                    let fmt = book.style(a.sheet, r, c).num_fmt.to_lowercase();
                    if fmt.contains('y') || (fmt.contains('d') && fmt.contains('m')) {
                        dates += 1;
                    } else {
                        numbers += 1;
                        sum += n;
                        min = min.min(n);
                        max = max.max(n);
                    }
                }
                CellValue::Boolean(_) => bools += 1,
                CellValue::String(ref s) if s.is_empty() => empty += 1,
                CellValue::String(_) => texts += 1,
            }
            if !text.is_empty() && distinct.len() < 1000 {
                distinct.insert(text.clone());
            }
            if !text.is_empty() && examples.len() < 5 && !examples.contains(&text) {
                examples.push(text);
            }
        }
        let filled = numbers + texts + dates + bools;
        let kind = [("numbers", numbers), ("words", texts), ("dates", dates), ("true/false", bools)]
            .iter()
            .filter(|(_, n)| *n > 0)
            .max_by_key(|(_, n)| *n)
            .map(|(k, n)| if *n == filled { k.to_string() } else { format!("mostly {k}") })
            .unwrap_or_else(|| "empty".into());
        let mut col = json!({ "name": name, "holds": kind, "filled": filled, "empty": empty, "different_values": distinct.len(), "examples": examples });
        if numbers > 0 {
            col["sum"] = json!(round(sum));
            col["smallest"] = json!(round(min));
            col["largest"] = json!(round(max));
            col["average"] = json!(round(sum / numbers as f64));
        }
        let note = match &a.table {
            Some(t) => note_for_table(book, &t.name, &name),
            None => book
                .notes
                .iter()
                .find(|n| matches!(n.target, NoteTarget::Column { sheet, column } if sheet == a.sheet && column == c)),
        };
        if let Some(n) = note {
            col["note"] = note_json(n);
        }
        if let Some(q) = book.queries.get(&a.sheet)
            && let Some(n) = q.notes.get(&name).or_else(|| q.notes.get("columns").and_then(|x| x.get(&name)))
        {
            col["database_note"] = n.clone();
        }
        out.push(col);
    }
    let mut v = json!({ "rows": (last - first + 1).max(0), "columns": out });
    if let Some(t) = &a.table {
        v["table"] = json!(t.name);
        v["formulas_use"] = json!(format!("{}[Column name]", t.name));
    } else {
        v["sheet"] = json!(book.sheet_name(a.sheet)?);
        v["cells"] = json!(format!(
            "{}{}:{}{}",
            tables::column_letters(a.range.left),
            a.range.top,
            tables::column_letters(a.range.right),
            a.range.bottom
        ));
    }
    Ok(v)
}

fn round(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

/// Some rows of an area, as shown (at most 50).
pub fn rows(book: &Book, a: &Area, from: usize, count: usize) -> Result<Json, String> {
    let i = book.index(a.sheet)?;
    let first = if a.header { a.range.top + 1 } else { a.range.top };
    let header: Vec<String> = (a.range.left..=a.range.right)
        .map(|c| if a.header { shown(book, i, a.range.top, c) } else { crate::tables::column_letters(c) })
        .collect();
    let start = first + from as i32;
    let end = (start + count.min(MOST_ROWS) as i32 - 1).min(a.range.bottom);
    let rows: Vec<Vec<String>> =
        (start..=end).map(|r| (a.range.left..=a.range.right).map(|c| shown(book, i, r, c)).collect()).collect();
    Ok(
        json!({ "columns": header, "rows": rows, "first_row": start, "rows_in_all": (a.range.bottom - first + 1).max(0) }),
    )
}

/// Works `formula` out on `sheet` (its references are read from there)
/// without changing the book: it's put in a corner cell, worked out, and
/// taken away again.
pub fn calculate(book: &mut Book, sheet: Option<&str>, formula: &str) -> Result<Json, String> {
    let i = match sheet.filter(|s| !s.is_empty()) {
        Some(name) => book
            .model
            .workbook
            .worksheets
            .iter()
            .position(|w| w.get_name().eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("there's no sheet called {name}"))? as u32,
        None => 0,
    };
    let (r, c) = (1_048_576, 16_384);
    if book.model.workbook.worksheets[i as usize].cell(r, c).is_some() {
        return Err("the sheet's last cell is in use".into());
    }
    let f = if formula.starts_with('=') { formula.to_string() } else { format!("={formula}") };
    let body = f[1..].to_string();
    // Put in the corner cell, worked out and read back (it's taken away below).
    let work = |book: &mut Book, f: String| -> Result<(String, CellValue, String), String> {
        book.model.set_user_input(i, r, c, f).map_err(|e| e.to_string())?;
        book.model.evaluate();
        Ok((
            shown(book, i, r, c),
            value(book, i, r, c),
            book.model.get_style_for_cell(i, r, c).map(|s| s.num_fmt).unwrap_or_default(),
        ))
    };
    let got = work(book, f.clone());
    // In the corner any array spills off the sheet (#SPILL!): ask its size
    // and its first values instead.
    let mut array = None;
    if let Ok((ref s, ..)) = got
        && s == "#SPILL!"
    {
        let count = |book: &mut Book, f: String| match work(book, f) {
            Ok((_, CellValue::Number(n), _)) if n >= 1.0 => Some(n as usize),
            _ => None,
        };
        if let (Some(rows), Some(cols)) =
            (count(book, format!("=ROWS({body})")), count(book, format!("=COLUMNS({body})")))
        {
            let mut first = Vec::new();
            for k in 0..(rows * cols).min(3) {
                let at = format!("=INDEX({body},{},{})", k / cols + 1, k % cols + 1);
                first.push(work(book, at).map(|x| x.0).unwrap_or_default());
            }
            array = Some((rows, cols, first));
        }
    }
    book.model.workbook.worksheets[i as usize].sheet_data.remove_cell(r, c);
    book.model.evaluate();
    let (mut shown_text, raw, fmt) = got?;
    let mut v = json!({ "formula": f });
    if let Some((rows, cols, first)) = array {
        let n = rows * cols;
        shown_text = format!("{n} values: {}{}", first.join(", "), if n > first.len() { ", …" } else { "" });
        v["array"] = json!({ "values": n, "rows": rows, "columns": cols, "first": first });
        v["note"] = json!(format!(
            "This formula returns an array of {n} values ({rows} row{} × {cols} column{}), not one value. For one number, wrap it: INDEX(…,1) for the first, or SUM, MAX, COUNT.",
            if rows == 1 { "" } else { "s" },
            if cols == 1 { "" } else { "s" }
        ));
        v["shows"] = json!(shown_text);
        return Ok(v);
    }
    // Errors show as #NAME?, #REF!, #DIV/0! and the like.
    let error = shown_text.starts_with('#').then(|| shown_text.clone());
    // A number shows the way the columns it reads do (money as 0.00), when
    // the engine didn't already give it a format.
    let mut used_fmt = (!fmt.is_empty() && !fmt.eq_ignore_ascii_case("general")).then_some(fmt);
    if let (CellValue::Number(n), None, None) = (&raw, &used_fmt, &error)
        && let Some(src) = source_format(book, i, &body)
        && let Ok(locale) = ironcalc_base::locale::get_locale("en")
    {
        shown_text = ironcalc_base::formatter::format::format_number(*n, &src, locale).text;
        used_fmt = Some(src);
    }
    v["shows"] = json!(shown_text);
    match raw {
        CellValue::Number(n) => v["value"] = json!(n),
        CellValue::String(s) => v["value"] = json!(s),
        CellValue::Boolean(b) => v["value"] = json!(b),
        CellValue::None => {}
    }
    if let (Some(f), true) = (used_fmt, v["value"].is_number()) {
        v["format"] = json!(f);
    }
    if let Some(e) = error {
        v["error"] = json!(e);
    }
    Ok(v)
}

/// Functions whose answer is a count or a position, not an amount of what
/// they read: their result keeps the plain number format.
const COUNTS: &[&str] = &[
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "COUNTIF",
    "COUNTIFS",
    "ROWS",
    "COLUMNS",
    "MATCH",
    "XMATCH",
    "RANK",
    "RANK.EQ",
    "RANK.AVG",
    "LEN",
    "YEAR",
    "MONTH",
    "DAY",
    "HOUR",
    "MINUTE",
    "WEEKDAY",
    "WEEKNUM",
    "ISOWEEKNUM",
];

fn dated_format(f: &str) -> bool {
    let f = f.to_lowercase();
    f.contains('y') || (f.contains('d') && f.contains('m')) || f.contains("h:") || f.contains("mm:")
}

/// Decimals in a number format's first part ("#,##0.00" → 2).
fn format_decimals(f: &str) -> usize {
    let first = f.split(';').next().unwrap_or("");
    first.split_once('.').map(|(_, d)| d.chars().take_while(|c| *c == '0' || *c == '#').count()).unwrap_or(0)
}

/// The number format of what `body` (a formula without its "=") reads, on
/// sheet `i`: table columns (`Sales[Revenue]`) and cell references. When
/// they agree, theirs; when they all have two decimals (money), "#,##0.00";
/// otherwise none. Counts and dates keep their own.
fn source_format(book: &Book, i: u32, body: &str) -> Option<String> {
    let lead: String = body.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
    if COUNTS.iter().any(|n| n.eq_ignore_ascii_case(&lead)) {
        return None;
    }
    let lower = body.to_lowercase();
    let mut formats: Vec<String> = Vec::new();
    // The format of the first number in rows top..=bottom of column col.
    let first_number = |sheet: u32, top: i32, bottom: i32, col: i32| -> Option<String> {
        let k = book.index(sheet).ok()?;
        (top..=bottom.min(top + 200))
            .find(|r| matches!(value(book, k, *r, col), CellValue::Number(_)))
            .map(|r| book.style(sheet, r, col).num_fmt)
    };
    for t in tables::all(book) {
        let tn = t.name.to_lowercase();
        if !lower.contains(&format!("{tn}[")) {
            continue;
        }
        for (k, col) in t.columns.iter().enumerate() {
            let cn = col.name.to_lowercase();
            if lower.contains(&format!("{tn}[{cn}]")) || lower.contains(&format!("[{cn}]")) {
                let top = t.top + i32::from(t.header);
                let bottom = t.bottom - i32::from(t.totals);
                if let Some(f) = first_number(t.sheet, top, bottom, t.left + k as i32) {
                    formats.push(f);
                }
            }
        }
    }
    let sheet = book.model.workbook.worksheets.get(i as usize)?.sheet_id;
    for (on, top, left, bottom, right) in cell_refs(body) {
        let id = match on {
            Some(name) => {
                book.model.workbook.worksheets.iter().find(|w| w.get_name().eq_ignore_ascii_case(&name))?.sheet_id
            }
            None => sheet,
        };
        if let Some(f) = (left..=right.min(left + 20)).find_map(|c| first_number(id, top, bottom, c)) {
            formats.push(f);
        }
    }
    let first = formats.first()?.clone();
    if formats.iter().any(|f| dated_format(f) || f.contains('@')) {
        return None;
    }
    if formats.iter().all(|f| *f == first) {
        return (!first.eq_ignore_ascii_case("general")).then_some(first);
    }
    formats.iter().all(|f| format_decimals(f) == 2 && !f.contains('%')).then(|| "#,##0.00".to_string())
}

/// The plain cell references in a formula ("B2", "Sheet2!A1:C9", "$D:$D"),
/// as (sheet name, top, left, bottom, right); words in quotes, function and
/// table names are skipped.
fn cell_refs(body: &str) -> Vec<(Option<String>, i32, i32, i32, i32)> {
    let ch: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    // A column and/or row ("$B$2", "B", "2") at k: (column, row, next k).
    let part = |mut k: usize| -> (Option<i32>, Option<i32>, usize) {
        let mut col = None;
        let mut row = None;
        if ch.get(k) == Some(&'$') {
            k += 1;
        }
        let s = k;
        while k < ch.len() && ch[k].is_ascii_alphabetic() && k - s < 3 {
            k += 1;
        }
        if k > s {
            col = Some(ch[s..k].iter().fold(0, |n, c| n * 26 + (c.to_ascii_uppercase() as i32 - 'A' as i32 + 1)));
        }
        let save = k;
        if ch.get(k) == Some(&'$') {
            k += 1;
        }
        let d = k;
        while k < ch.len() && ch[k].is_ascii_digit() {
            k += 1;
        }
        if k > d {
            row = ch[d..k].iter().collect::<String>().parse().ok();
        } else {
            k = save;
        }
        (col, row, k)
    };
    while k < ch.len() {
        let c = ch[k];
        if c == '"' {
            k += 1;
            while k < ch.len() && ch[k] != '"' {
                k += 1;
            }
            k += 1;
            continue;
        }
        let starts = k == 0 || !(ch[k - 1].is_alphanumeric() || matches!(ch[k - 1], '_' | '.' | '[' | '$'));
        if !starts || !(c.is_alphanumeric() || c == '$' || c == '\'') {
            k += 1;
            continue;
        }
        // An optional sheet name: Name! or 'A name'!
        let mut on = None;
        let mut at = k;
        if c == '\'' {
            if let Some(end) = (k + 1..ch.len()).find(|&j| ch[j] == '\'')
                && ch.get(end + 1) == Some(&'!')
            {
                on = Some(ch[k + 1..end].iter().collect::<String>());
                at = end + 2;
            } else {
                k += 1;
                continue;
            }
        } else {
            let mut j = k;
            while j < ch.len() && (ch[j].is_alphanumeric() || matches!(ch[j], '_' | '.')) {
                j += 1;
            }
            if ch.get(j) == Some(&'!') {
                on = Some(ch[k..j].iter().collect::<String>());
                at = j + 1;
            }
        }
        let (c1, r1, mut e) = part(at);
        let mut end = (c1, r1);
        if ch.get(e) == Some(&':') {
            let (c2, r2, e2) = part(e + 1);
            if (c2.is_some() || r2.is_some()) && (c2.is_some() == c1.is_some()) && (r2.is_some() == r1.is_some()) {
                end = (c2, r2);
                e = e2;
            }
        }
        let next = ch.get(e).copied().unwrap_or(' ');
        let whole = e > at && !(next.is_alphanumeric() || matches!(next, '(' | '[' | '_' | '.' | '!'));
        let ranged = end != (c1, r1);
        if whole && (c1.is_some() && (r1.is_some() || ranged)) {
            let (top, bottom) = match (r1, end.1) {
                (Some(a), Some(b)) => (a.min(b), a.max(b)),
                _ => (1, 1_048_576),
            };
            let (a, b) = (c1.unwrap_or(1), end.0.unwrap_or(1));
            if a <= 16_384 && b <= 16_384 {
                out.push((on, top, a.min(b), bottom, a.max(b)));
            }
        }
        k = e.max(k + 1);
    }
    out
}
