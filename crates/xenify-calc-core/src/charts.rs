//! Charts: bars (up, sideways or stacked), line, area, pie, scatter,
//! heatmap, histogram and box, floating over a sheet. A chart of a table
//! names its columns, so it grows with the table; one of plain cells moves
//! with them as rows and columns are added or taken away. Rows hidden by a
//! filter are left out, as in the sheet.
//!
//! A chart either shows each row as it is (`series`), or adds rows up the
//! way Summarise does (`values`): one point for each value of its `x`
//! column, a line or colour for each value of `split`.

use crate::book::{Book, CellData};
use crate::edit::Range;
use crate::summary::{self, Condition, Show};
use crate::tables;
use ironcalc_base::cell::CellValue;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    /// Bars across, longest on top: for long names.
    Sideways,
    /// Bars with each series on top of the one before.
    Stacked,
    Line,
    Area,
    Pie,
    Scatter,
    Histogram,
    Box,
    /// A grid of coloured squares: the points along, the series down.
    Heatmap,
}

impl ChartKind {
    /// Kinds that show each number as it is (they can't add rows up).
    pub fn each_row(self) -> bool {
        matches!(self, ChartKind::Scatter | ChartKind::Histogram | ChartKind::Box)
    }
}

/// Where a chart's numbers come from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub sheet: u32,
    /// A table's columns by name (it grows with the table)…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default)]
    pub series: Vec<String>,
    /// …or cells: the first row names the series, the first column (when
    /// `labels`) names the points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(default)]
    pub labels: bool,
    /// Whether the cells' first row names the columns (without it they're
    /// called by their letters, "Column B").
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub header: bool,
    /// Added up: what's worked out for each point (by column name; `x`
    /// names the column the points are for)…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Show>,
    /// …a series for each value of this column…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
    /// …and only the rows that pass these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<Condition>,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

impl Source {
    /// The rows, as Summarise reads them.
    pub fn from(&self) -> summary::From {
        summary::From { sheet: self.sheet, table: self.table.clone(), range: self.range, header: self.header }
    }

    pub fn grouped(&self) -> bool {
        !self.values.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub id: u32,
    /// The sheet it floats on, and its top left (a cell, plus pixels into it).
    pub sheet: u32,
    pub row: i32,
    pub col: i32,
    #[serde(default)]
    pub dx: f64,
    #[serde(default)]
    pub dy: f64,
    pub w: f64,
    pub h: f64,
    pub kind: ChartKind,
    #[serde(default)]
    pub title: String,
    pub source: Source,
}

impl Chart {
    /// After `count` rows (or columns) were inserted (or, when negative,
    /// deleted) at `at` on `sheet`: the chart and its cells move with them.
    pub fn shift(&mut self, sheet: u32, at: i32, count: i32, rows: bool) {
        let mv = |v: i32| if v >= at { (v + count).max(at) } else { v };
        if self.sheet == sheet {
            if rows {
                self.row = mv(self.row).max(1);
            } else {
                self.col = mv(self.col).max(1);
            }
        }
        if let (Some(r), true) = (self.source.range.as_mut(), self.source.sheet == sheet) {
            if rows {
                let (a, b) = (mv(r.top), if r.bottom >= at { (r.bottom + count).max(at - 1) } else { r.bottom });
                r.top = a.max(1);
                r.bottom = b.max(r.top);
            } else {
                let (a, b) = (mv(r.left), if r.right >= at { (r.right + count).max(at - 1) } else { r.right });
                r.left = a.max(1);
                r.right = b.max(r.left);
            }
        }
    }
}

/// Each series' name and column.
pub type SeriesColumns = Vec<(String, i32)>;

/// The columns a chart reads: (rows top..=bottom, x column, series columns with names).
pub(crate) fn columns(book: &Book, s: &Source) -> Result<(i32, i32, Option<i32>, SeriesColumns), String> {
    if let Some(name) = &s.table {
        let t = tables::get(book, name).ok_or_else(|| format!("the table {name} isn't there any more"))?;
        let col = |n: &str| t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(n)).map(|k| t.left + k as i32);
        let x = s.x.as_deref().and_then(col);
        let series = s.series.iter().filter_map(|n| col(n).map(|c| (n.clone(), c))).collect();
        return Ok((t.top + t.header as i32, t.bottom - t.totals as i32, x, series));
    }
    let r = s.range.ok_or("a chart needs a table or cells")?;
    let i = book.index(s.sheet)?;
    let name = |c: i32| {
        if s.header {
            book.model.get_formatted_cell_value(i, r.top, c).unwrap_or_default()
        } else {
            tables::letter_name(c)
        }
    };
    let top = r.top + s.header as i32;
    // Columns picked by name (in the chart panel)…
    if !s.series.is_empty() {
        let col = |n: &str| (r.left..=r.right).find(|c| name(*c).eq_ignore_ascii_case(n));
        let x = s.x.as_deref().and_then(col);
        let series = s.series.iter().filter_map(|n| col(n).map(|c| (n.clone(), c))).collect();
        return Ok((top, r.bottom, x, series));
    }
    // …or all of them, after the point names.
    let first = if s.labels { r.left + 1 } else { r.left };
    let series = (first..=r.right).map(|c| (name(c), c)).collect();
    Ok((top, r.bottom, s.labels.then_some(r.left), series))
}

/// The columns a chart can use, by name (for the chart panel).
pub fn column_names(book: &Book, s: &Source) -> Result<Vec<String>, String> {
    summary::columns(book, &s.from())
}

/// A shown value: numbers in the format given, words as they are.
fn shown(v: &CellData, fmt: &str) -> String {
    match v {
        CellData::Number(n) => match ironcalc_base::locale::get_locale("en") {
            Ok(locale) => ironcalc_base::formatter::format::format_number(*n, fmt, locale).text,
            Err(_) => n.to_string(),
        },
        CellData::Text(t) => t.clone(),
        _ => String::new(),
    }
}

/// An added-up chart's points and series (worked out as a summary).
fn grouped_data(book: &Book, s: &Source) -> Result<Json, String> {
    let mut group: Vec<String> = s.x.iter().cloned().collect();
    group.extend(s.split.iter().cloned());
    let def = summary::Def {
        from: s.from(),
        group: group.clone(),
        show: s.values.clone(),
        only: s.only.clone(),
        order: None,
        desc: false,
        as_found: true,
        totals: false,
    };
    let r = summary::compute(book, &def)?;
    let g = group.len();
    let head: Vec<String> = r.rows.first().map(|h| h.iter().map(|v| shown(v, "general")).collect()).unwrap_or_default();
    let body = &r.rows[1.min(r.rows.len())..];
    let fmt = |k: usize| r.formats.get(k).cloned().unwrap_or_else(|| "general".into());
    let num = |v: &CellData| if let CellData::Number(n) = v { Some(n.f()) } else { None };
    // The points: the x column's values as they first come (or one point for everything).
    let mut labels: Vec<String> = Vec::new();
    let point = |line: &[CellData]| if s.x.is_some() { shown(&line[0], &fmt(0)) } else { String::new() };
    for line in body {
        let p = point(line);
        if !labels.contains(&p) {
            labels.push(p);
        }
    }
    let mut series: Vec<Json> = Vec::new();
    if s.split.is_some() {
        let k_split = g - 1;
        let mut splits: Vec<String> = Vec::new();
        for line in body {
            let v = shown(&line[k_split], &fmt(k_split));
            if !splits.contains(&v) {
                splits.push(v);
            }
        }
        for v in &splits {
            for (k, _) in s.values.iter().enumerate() {
                let mut values: Vec<Option<f64>> = vec![None; labels.len()];
                for line in body.iter().filter(|l| shown(&l[k_split], &fmt(k_split)) == *v) {
                    if let Some(i) = labels.iter().position(|l| *l == point(line)) {
                        values[i] = num(&line[g + k]);
                    }
                }
                let name = if s.values.len() == 1 { v.clone() } else { format!("{v} · {}", head[g + k]) };
                series.push(json!({ "name": name, "values": values, "format": fmt(g + k) }));
            }
        }
    } else {
        for (k, _) in s.values.iter().enumerate() {
            let values: Vec<Option<f64>> = body.iter().map(|l| num(&l[g + k])).collect();
            series.push(json!({ "name": head[g + k], "values": values, "format": fmt(g + k) }));
        }
    }
    Ok(json!({ "labels": labels, "x": null, "series": series }))
}

/// What the chart shows: point names, and each series' numbers (none where
/// a cell isn't a number).
pub fn data(book: &Book, chart: &Chart) -> Result<Json, String> {
    let s = &chart.source;
    if s.grouped() && !chart.kind.each_row() {
        return grouped_data(book, s);
    }
    let i = book.index(s.sheet)?;
    let (top, bottom, x, series) = columns(book, s)?;
    let rows: Vec<i32> = (top..=bottom).filter(|r| !book.model.is_row_hidden(i, *r).unwrap_or(false)).collect();
    let labels: Vec<String> = rows
        .iter()
        .map(|r| {
            x.map(|c| book.model.get_formatted_cell_value(i, *r, c).unwrap_or_default())
                .unwrap_or_else(|| r.to_string())
        })
        .collect();
    let number = |r: i32, c: i32| match book.model.get_cell_value_by_index(i, r, c) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    };
    let xs: Option<Vec<Option<f64>>> = x.map(|c| rows.iter().map(|r| number(*r, c)).collect());
    let series: Vec<Json> = series
        .iter()
        .map(|(name, c)| {
            let values: Vec<Option<f64>> = rows.iter().map(|r| number(*r, *c)).collect();
            let fmt = book.style(s.sheet, rows.first().copied().unwrap_or(top), *c).num_fmt;
            json!({ "name": name, "values": values, "format": fmt })
        })
        .collect();
    Ok(json!({ "labels": labels, "x": xs, "series": series }))
}

/// A chart for the cells selected: a table's columns when it's in a table,
/// and a kind that suits them (words then numbers: bars, or a line when
/// there are many points; one column of numbers: a histogram).
pub fn suggest(book: &Book, sheet: u32, range: Range, id: u32, at: (i32, i32)) -> Result<Chart, String> {
    let i = book.index(sheet)?;
    let is_number = |r: i32, c: i32| matches!(book.model.get_cell_value_by_index(i, r, c), Ok(CellValue::Number(_)));
    let source;
    let (n_points, labels, n_series);
    if let Some(t) = tables::at(book, sheet, range.top, range.left) {
        let data_top = t.top + t.header as i32;
        let cols: Vec<i32> = if range.left == range.right && range.top != range.bottom {
            (range.left..=range.right).collect()
        } else if range.left == range.right {
            (t.left..=t.right).collect()
        } else {
            (range.left.max(t.left)..=range.right.min(t.right)).collect()
        };
        let numeric = |c: i32| is_number(data_top, c);
        let x = cols.iter().copied().find(|c| !numeric(*c)).or_else(|| (t.left..=t.right).find(|c| !numeric(*c)));
        let series: Vec<String> = cols
            .iter()
            .copied()
            .filter(|c| numeric(*c) && Some(*c) != x)
            .map(|c| t.columns[(c - t.left) as usize].name.clone())
            .collect();
        if series.is_empty() {
            return Err("Pick a column of numbers to chart".to_string());
        }
        n_points = (t.bottom - t.totals as i32 - data_top + 1).max(0);
        labels = x.is_some();
        n_series = series.len();
        source = Source {
            sheet,
            table: Some(t.name.clone()),
            x: x.map(|c| t.columns[(c - t.left) as usize].name.clone()),
            series,
            header: true,
            ..Default::default()
        };
    } else {
        let mut r = range;
        if r.top == r.bottom && r.left == r.right {
            let (a, b, c, d) = tables::block(book, sheet, r.top, r.left);
            r = Range { top: a, left: b, bottom: c, right: d };
        }
        let header = tables::has_header(book, sheet, r.top, r.left, r.bottom, r.right);
        let first = r.top + header as i32;
        let lab = !is_number(first, r.left) && r.right > r.left;
        n_series = (r.right - r.left + 1 - lab as i32) as usize;
        if n_series == 0 || !(r.left..=r.right).any(|c| is_number(first, c)) {
            return Err("Select cells with numbers to make a chart of".to_string());
        }
        n_points = r.bottom - first + 1;
        labels = lab;
        source = Source { sheet, range: Some(r), labels: lab, header, ..Default::default() };
    }
    let kind = if !labels && n_series == 1 {
        ChartKind::Histogram
    } else if n_points > 24 {
        ChartKind::Line
    } else {
        ChartKind::Bar
    };
    // Columns without a header have no name worth showing.
    let title = match &source.table {
        Some(_) if source.series.iter().any(|n| n.starts_with("Column ")) => String::new(),
        Some(_) if n_series == 1 => source.series[0].clone(),
        Some(_) => source.series.join(" and "),
        None => String::new(),
    };
    Ok(Chart { id, sheet, row: at.0, col: at.1, dx: 8.0, dy: 8.0, w: 520.0, h: 300.0, kind, title, source })
}
