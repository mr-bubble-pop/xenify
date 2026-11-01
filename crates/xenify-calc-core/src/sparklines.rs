//! Sparklines: a small line or bars drawn inside one cell from a row or
//! column of numbers. Read from and written to Excel files (where they
//! live in an extension of the sheet).

use crate::book::Book;
use crate::edit::Range;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SparkKind {
    Line,
    Column,
    /// Ups and downs: a bar up for more than nothing, down for less.
    WinLoss,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sparkline {
    /// The cell it's drawn in.
    pub sheet: u32,
    pub row: i32,
    pub col: i32,
    pub kind: SparkKind,
    /// The numbers it draws.
    pub source_sheet: u32,
    pub source: Range,
}

impl Sparkline {
    /// After `count` rows (or columns) were inserted (or, when negative,
    /// deleted) at `at` on `sheet`: none when its cell went.
    pub fn shift(&self, sheet: u32, at: i32, count: i32, rows: bool) -> Option<Sparkline> {
        let mut s = self.clone();
        let mv = |v: i32| -> Option<i32> {
            if v < at {
                Some(v)
            } else if count < 0 && v < at - count {
                None
            } else {
                Some(v + count)
            }
        };
        if s.sheet == sheet {
            if rows {
                s.row = mv(s.row)?;
            } else {
                s.col = mv(s.col)?;
            }
        }
        if s.source_sheet == sheet {
            let r = &mut s.source;
            let (a, b) = if rows { (&mut r.top, &mut r.bottom) } else { (&mut r.left, &mut r.right) };
            let na = if *a >= at { (*a + count).max(at) } else { *a };
            let nb = if *b >= at { (*b + count).max(at - 1) } else { *b };
            if nb < na {
                return None;
            }
            *a = na;
            *b = nb;
        }
        Some(s)
    }
}

/// The ext uri sparklines are written under.
pub const EXT_URI: &str = "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}";

/// The sparklines in a sheet's XML (from its x14 extension).
pub fn read(book: &Book, sheet: u32, xml: &str) -> Vec<Sparkline> {
    let mut out = Vec::new();
    let mut r = Reader::from_str(xml);
    let mut kind = SparkKind::Line;
    let mut path: Vec<String> = Vec::new();
    let (mut f, mut sqref) = (String::new(), String::new());
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                let l = name.rsplit(':').next().unwrap_or(&name).to_string();
                if l == "sparklineGroup" {
                    let t = e.attributes().flatten().find(|a| a.key.as_ref() == "type").and_then(|a| {
                        a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned())
                    });
                    kind = match t.as_deref() {
                        Some("column") => SparkKind::Column,
                        Some("stacked") => SparkKind::WinLoss,
                        _ => SparkKind::Line,
                    };
                }
                if l == "sparkline" {
                    f.clear();
                    sqref.clear();
                }
                path.push(l);
            }
            Ok(Event::Text(t)) => match path.last().map(String::as_str) {
                Some("f") => f.push_str(t.as_ref()),
                Some("sqref") => sqref.push_str(t.as_ref()),
                _ => {}
            },
            Ok(Event::End(_)) => {
                if path.last().is_some_and(|x| x == "sparkline")
                    && let (Some((row, col)), Some((src_sheet, source))) =
                        (crate::tables::parse_cell_pub(sqref.trim()), source_of(book, sheet, &f))
                {
                    out.push(Sparkline { sheet, row, col, kind, source_sheet: src_sheet, source });
                }
                path.pop();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// `Sheet1!B2:F2` (or `B2:F2` on the same sheet) → its sheet id and cells.
fn source_of(book: &Book, sheet: u32, f: &str) -> Option<(u32, Range)> {
    let f = f.trim();
    let (name, cells) = match f.rsplit_once('!') {
        Some((n, c)) => (Some(n.trim_matches('\'').replace("''", "'")), c),
        None => (None, f),
    };
    let id = match name {
        Some(n) => book.model.workbook.worksheets.iter().find(|w| w.get_name().eq_ignore_ascii_case(&n))?.sheet_id,
        None => sheet,
    };
    let (a, b) = cells.split_once(':').unwrap_or((cells, cells));
    let (r1, c1) = crate::tables::parse_cell_pub(a)?;
    let (r2, c2) = crate::tables::parse_cell_pub(b)?;
    Some((id, Range { top: r1, left: c1, bottom: r2, right: c2 }.normal()))
}

/// The numbers a sparkline draws (none where a cell isn't a number).
pub fn values(book: &Book, s: &Sparkline) -> Vec<Option<f64>> {
    let Ok(i) = book.index(s.source_sheet) else { return Vec::new() };
    let r = s.source;
    let mut out = Vec::new();
    for row in r.top..=r.bottom.min(r.top + 500) {
        for col in r.left..=r.right.min(r.left + 500) {
            out.push(match book.model.get_cell_value_by_index(i, row, col) {
                Ok(ironcalc_base::cell::CellValue::Number(n)) => Some(n),
                _ => None,
            });
        }
    }
    out
}

/// The x14 extension holding a sheet's sparklines.
pub fn ext_xml(book: &Book, sheet: u32) -> Option<String> {
    let mine: Vec<&Sparkline> = book.sparklines.iter().filter(|s| s.sheet == sheet).collect();
    if mine.is_empty() {
        return None;
    }
    let mut groups = String::new();
    for s in mine {
        let src = book.sheet_name(s.source_sheet).ok()?;
        let col = crate::tables::column_letters;
        let f = format!(
            "'{}'!{}{}:{}{}",
            src.replace('\'', "''"),
            col(s.source.left),
            s.source.top,
            col(s.source.right),
            s.source.bottom
        );
        let kind = match s.kind {
            SparkKind::Line => "",
            SparkKind::Column => " type=\"column\"",
            SparkKind::WinLoss => " type=\"stacked\" negative=\"1\"",
        };
        groups.push_str(&format!(
            "<x14:sparklineGroup displayEmptyCellsAs=\"gap\"{kind}><x14:colorSeries rgb=\"FFC2410C\"/><x14:colorNegative rgb=\"FFB91C1C\"/><x14:colorAxis rgb=\"FF000000\"/><x14:colorMarkers rgb=\"FFC2410C\"/><x14:colorFirst rgb=\"FFC2410C\"/><x14:colorLast rgb=\"FFC2410C\"/><x14:colorHigh rgb=\"FFC2410C\"/><x14:colorLow rgb=\"FFC2410C\"/><x14:sparklines><x14:sparkline><xm:f>{}</xm:f><xm:sqref>{}{}</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup>",
            f.replace('&', "&amp;").replace('<', "&lt;"),
            col(s.col),
            s.row
        ));
    }
    Some(format!(
        "<ext uri=\"{EXT_URI}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:sparklineGroups xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">{groups}</x14:sparklineGroups></ext>"
    ))
}
