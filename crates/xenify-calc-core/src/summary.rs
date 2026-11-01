//! Summarise: one row for each value (or pair of values) of some columns of
//! a table or block, with counts, totals, averages… of others. A Summary
//! sheet shows the result and follows its source: it's worked out again
//! after every change. (In Excel files it's saved as its values.)

use crate::book::{Book, CellData, SheetKind};
use crate::edit::Range;
use crate::tables;
use ironcalc_base::cell::CellValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What's summarised: a table (by name) or cells whose first row names the
/// columns (or, without `header`, cells that are all data: their columns are
/// called by their letters, "Column B").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct From {
    pub sheet: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub header: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// How many rows (the column isn't needed).
    Rows,
    /// How many different values.
    Distinct,
    Sum,
    Average,
    Median,
    Min,
    Max,
}

impl How {
    pub fn word(self) -> &'static str {
        match self {
            How::Rows => "Rows",
            How::Distinct => "Different",
            How::Sum => "Total",
            How::Average => "Average",
            How::Median => "Middle",
            How::Min => "Smallest",
            How::Max => "Largest",
        }
    }
}

/// One shown column: what's worked out of which column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Show {
    #[serde(default)]
    pub column: String,
    pub how: How,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Test {
    Is,
    IsNot,
    Contains,
    More,
    Less,
    /// A date in the year given.
    Year,
}

/// "Only rows where…"
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub column: String,
    pub test: Test,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Def {
    pub from: From,
    /// One row for each value of these columns.
    pub group: Vec<String>,
    pub show: Vec<Show>,
    #[serde(default)]
    pub only: Vec<Condition>,
    /// Ordered by this shown column (by its place in `show`), biggest first
    /// when `desc`; none: by the grouped values.
    #[serde(default)]
    pub order: Option<usize>,
    #[serde(default)]
    pub desc: bool,
    /// With no `order`: the groups as they first come in the data (months
    /// stay in their order) rather than by their values.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub as_found: bool,
    #[serde(default = "yes")]
    pub totals: bool,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

/// A column's value in a row, as summarising sees it.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum V {
    Num(f64),
    Text(String),
    Empty,
}

impl V {
    fn key(&self) -> String {
        match self {
            V::Num(n) => format!("{n}"),
            V::Text(s) => s.to_lowercase(),
            V::Empty => String::new(),
        }
    }
}

/// A source's columns (name and column number), data rows and sheet position.
type Source = (Vec<(String, i32)>, Vec<i32>, u32);

/// The source's columns (name → column number) and data rows.
fn source(book: &Book, f: &From) -> Result<Source, String> {
    let i = book.index(f.sheet)?;
    if let Some(name) = &f.table {
        let t = tables::get(book, name).ok_or_else(|| format!("the table {name} isn't there any more"))?;
        let cols = t.columns.iter().enumerate().map(|(k, c)| (c.name.clone(), t.left + k as i32)).collect();
        let rows = (t.top + t.header as i32..=t.bottom - t.totals as i32).collect();
        return Ok((cols, rows, i));
    }
    let r = f.range.ok_or("Summarise needs a table or cells")?;
    if !f.header {
        let cols = (r.left..=r.right).map(|c| (tables::letter_name(c), c)).collect();
        return Ok((cols, (r.top..=r.bottom).collect(), i));
    }
    let cols =
        (r.left..=r.right).map(|c| (book.model.get_formatted_cell_value(i, r.top, c).unwrap_or_default(), c)).collect();
    Ok((cols, (r.top + 1..=r.bottom).collect(), i))
}

fn value(book: &Book, i: u32, row: i32, col: i32) -> V {
    match book.model.get_cell_value_by_index(i, row, col) {
        Ok(CellValue::Number(n)) => V::Num(n),
        Ok(CellValue::String(s)) if !s.is_empty() => V::Text(s),
        Ok(CellValue::Boolean(b)) => V::Text(if b { "TRUE".into() } else { "FALSE".into() }),
        _ => V::Empty,
    }
}

fn passes(book: &Book, i: u32, row: i32, col: i32, c: &Condition) -> bool {
    let v = value(book, i, row, col);
    let shown = book.model.get_formatted_cell_value(i, row, col).unwrap_or_default().to_lowercase();
    let want = c.value.trim().to_lowercase();
    let num = want.replace(',', "").parse::<f64>().ok();
    match c.test {
        Test::Is => shown == want || matches!((&v, num), (V::Num(n), Some(w)) if (*n - w).abs() < 1e-9),
        Test::IsNot => !(shown == want || matches!((&v, num), (V::Num(n), Some(w)) if (*n - w).abs() < 1e-9)),
        Test::Contains => shown.contains(&want),
        Test::More => matches!((&v, num), (V::Num(n), Some(w)) if *n > w),
        Test::Less => matches!((&v, num), (V::Num(n), Some(w)) if *n < w),
        // Dates are day numbers from 1899-12-30.
        Test::Year => match (&v, want.parse::<i32>().ok()) {
            (V::Num(n), Some(y)) => year_of(*n) == y,
            _ => false,
        },
    }
}

/// The year of a day number (as spreadsheets count days).
fn year_of(serial: f64) -> i32 {
    // Days from 1970-01-01, then the civil year (Howard Hinnant's algorithm).
    let days = serial.floor() as i64 - 25569;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (yoe + era * 400 + if month <= 2 { 1 } else { 0 }) as i32
}

fn work_out(how: How, vals: &[V]) -> CellData {
    let nums: Vec<f64> = vals.iter().filter_map(|v| if let V::Num(n) = v { Some(*n) } else { None }).collect();
    let n = |x: f64| CellData::Number(ironcalc_base::num::Num::from(x));
    match how {
        How::Rows => n(vals.len() as f64),
        How::Distinct => {
            let mut keys: Vec<String> = vals.iter().filter(|v| **v != V::Empty).map(V::key).collect();
            keys.sort();
            keys.dedup();
            n(keys.len() as f64)
        }
        How::Sum => n(nums.iter().sum()),
        How::Average if nums.is_empty() => CellData::Text(String::new()),
        How::Average => n(nums.iter().sum::<f64>() / nums.len() as f64),
        How::Median if nums.is_empty() => CellData::Text(String::new()),
        How::Median => {
            let mut s = nums.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let m = s.len() / 2;
            n(if s.len() % 2 == 1 { s[m] } else { (s[m - 1] + s[m]) / 2.0 })
        }
        How::Min => nums.iter().copied().reduce(f64::min).map(n).unwrap_or(CellData::Text(String::new())),
        How::Max => nums.iter().copied().reduce(f64::max).map(n).unwrap_or(CellData::Text(String::new())),
    }
}

/// The summary: its header row, one row for each group, and a totals row;
/// and the number format each column shows with.
pub struct Computed {
    pub rows: Vec<Vec<CellData>>,
    pub formats: Vec<String>,
    pub totals: bool,
}

pub fn compute(book: &Book, d: &Def) -> Result<Computed, String> {
    let (cols, rows, i) = source(book, &d.from)?;
    let col = |name: &str| cols.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, c)| *c);
    let group: Vec<i32> =
        d.group.iter().map(|g| col(g).ok_or_else(|| format!("no column {g}"))).collect::<Result<_, _>>()?;
    let shows: Vec<(Option<i32>, How)> = d.show.iter().map(|s| (col(&s.column), s.how)).collect();
    let conds: Vec<(i32, &Condition)> = d.only.iter().filter_map(|c| col(&c.column).map(|k| (k, c))).collect();
    // Rows hidden by a filter are left out, as in the sheet.
    let rows: Vec<i32> = rows
        .into_iter()
        .filter(|r| !book.model.is_row_hidden(i, *r).unwrap_or(false))
        .filter(|r| conds.iter().all(|(k, c)| passes(book, i, *r, *k, c)))
        .collect();
    // Groups, in the order of their (lowercased) values.
    let mut groups: BTreeMap<Vec<String>, (Vec<V>, Vec<i32>)> = BTreeMap::new();
    for r in &rows {
        let vals: Vec<V> = group.iter().map(|c| value(book, i, *r, *c)).collect();
        let key: Vec<String> = vals.iter().map(V::key).collect();
        groups.entry(key).or_insert_with(|| (vals, Vec::new())).1.push(*r);
    }
    let figure = |rows: &[i32], (c, how): (Option<i32>, How)| -> CellData {
        let vals: Vec<V> = match c {
            Some(c) => rows.iter().map(|r| value(book, i, *r, c)).collect(),
            None => rows.iter().map(|_| V::Num(1.0)).collect(),
        };
        work_out(how, &vals)
    };
    let mut found: Vec<(&Vec<V>, &Vec<i32>)> = groups.values().map(|(v, r)| (v, r)).collect();
    if d.as_found && d.order.is_none() {
        found.sort_by_key(|(_, rs)| rs.first().copied().unwrap_or(i32::MAX));
    }
    let mut body: Vec<Vec<CellData>> = found
        .into_iter()
        .map(|(vals, rs)| {
            let mut line: Vec<CellData> = vals
                .iter()
                .map(|v| match v {
                    V::Num(n) => CellData::Number(ironcalc_base::num::Num::from(*n)),
                    V::Text(s) => CellData::Text(s.clone()),
                    V::Empty => CellData::Text("(empty)".into()),
                })
                .collect();
            line.extend(shows.iter().map(|s| figure(rs, *s)));
            line
        })
        .collect();
    if let Some(k) = d.order.filter(|k| *k < shows.len()) {
        let at = group.len() + k;
        let num = |l: &Vec<CellData>| if let CellData::Number(n) = l[at] { n.f() } else { f64::MIN };
        body.sort_by(|a, b| num(a).partial_cmp(&num(b)).unwrap_or(std::cmp::Ordering::Equal));
        if d.desc {
            body.reverse();
        }
    } else if d.desc {
        body.reverse();
    }
    let mut header: Vec<CellData> = d.group.iter().map(|g| CellData::Text(g.clone())).collect();
    // A total is called after its column ("Total", "Items"); the others say what they are.
    header.extend(d.show.iter().map(|s| {
        CellData::Text(match s.how {
            How::Rows => "Rows".into(),
            How::Sum => s.column.clone(),
            how => format!("{} {}", how.word(), s.column.to_lowercase()),
        })
    }));
    let mut out = vec![header];
    let groups_n = body.len();
    out.extend(body);
    if d.totals {
        // How many groups, in the first grouped column (none grouped: just the figures).
        let mut line = Vec::new();
        if !group.is_empty() {
            line.push(CellData::Text(format!("{groups_n} {}", if groups_n == 1 { "group" } else { "groups" })));
            line.extend((1..group.len()).map(|_| CellData::Text(String::new())));
        }
        line.extend(shows.iter().map(|s| figure(&rows, *s)));
        out.push(line);
    }
    // Number formats: grouped columns as in the source; totals, averages… too; counts plain.
    let fmt = |c: Option<i32>| {
        c.map(|c| book.style(d.from.sheet, rows.first().copied().unwrap_or(1), c).num_fmt)
            .unwrap_or_else(|| "general".into())
    };
    let mut formats: Vec<String> = group.iter().map(|c| fmt(Some(*c))).collect();
    formats.extend(shows.iter().enumerate().map(|(k, (c, how))| match how {
        How::Rows | How::Distinct => "general".into(),
        // Averages of plain numbers: whole when they all are, else two places.
        How::Average | How::Median if fmt(*c) == "general" => {
            let at = group.len() + k;
            let whole =
                out.iter().skip(1).all(|l| !matches!(l.get(at), Some(CellData::Number(n)) if n.fract().abs() > 1e-9));
            if whole { "#,##0".into() } else { "#,##0.00".into() }
        }
        _ => fmt(*c),
    }));
    Ok(Computed { rows: out, formats, totals: d.totals })
}

/// Writes summary `d` into its sheet `sheet` (everything there is replaced).
/// Not an edit: the sheet only ever shows what its source gives.
pub fn fill(book: &mut Book, sheet: u32, d: &Def) -> Result<(), String> {
    let i = book.index(sheet)?;
    let res = compute(book, d);
    let w = &mut book.model.workbook.worksheets[i as usize];
    let old: Vec<(i32, i32)> = w.sheet_data.cells().map(|(r, c, _)| (r, c)).collect();
    for (r, c) in old {
        w.sheet_data.remove_cell(r, c);
    }
    let res = match res {
        Ok(r) => r,
        Err(e) => {
            book.put(sheet, 1, 1, Some(&CellData::Text(format!("This summary can’t be worked out: {e}"))))?;
            return Ok(());
        }
    };
    let last = res.rows.len() as i32;
    for (r, line) in res.rows.iter().enumerate() {
        let row = r as i32 + 1;
        for (c, v) in line.iter().enumerate() {
            let col = c as i32 + 1;
            let mut style = ironcalc_base::types::Style::default();
            if row == 1 || (res.totals && row == last) {
                style.font.b = true;
            }
            if row > 1 {
                style.num_fmt = res.formats.get(c).cloned().unwrap_or_else(|| "general".into());
            }
            if res.totals && row == last {
                style.border.top = Some(ironcalc_base::types::BorderItem {
                    style: ironcalc_base::types::BorderStyle::Medium,
                    color: ironcalc_base::types::Color::Rgb("#1C1917".into()),
                });
            }
            book.model.set_cell_style(i, row, col, &style)?;
            book.put(sheet, row, col, Some(v))?;
        }
    }
    Ok(())
}

/// Works out every Summary sheet again (after any change).
pub fn refresh_all(book: &mut Book) {
    let defs: Vec<(u32, Def)> = book.summaries.iter().map(|(k, d)| (*k, d.clone())).collect();
    for (sheet, d) in defs {
        if book.info(sheet).kind == SheetKind::Summary {
            let _ = fill(book, sheet, &d);
        }
    }
    if !book.summaries.is_empty() {
        book.model.evaluate();
    }
}

/// The columns Summarise can use from a source: their names.
pub fn columns(book: &Book, f: &From) -> Result<Vec<String>, String> {
    Ok(source(book, f)?.0.into_iter().map(|(n, _)| n).collect())
}
