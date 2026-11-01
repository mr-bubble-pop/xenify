//! Old Excel files (.xls) and OpenDocument spreadsheets (.ods): their
//! sheets, values and formulas (read by calamine). Formats, merged cells
//! and charts in them aren't read; saving writes a Calc book or an Excel file.

use crate::book::{Book, CellData, SheetInfo};
use crate::ops::{CellEdit, Op};
use calamine::{Data, Reader, SheetType, SheetVisible};
use std::io::Cursor;

/// A book from the bytes of an .xls or .ods file.
pub fn open(bytes: &[u8], what: &str) -> Result<Book, String> {
    let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| format!("Couldn’t read this {what} file: {e}"))?;
    let sheets: Vec<(String, bool)> = wb
        .sheets_metadata()
        .iter()
        .filter(|s| matches!(s.typ, SheetType::WorkSheet))
        .map(|s| (s.name.clone(), !matches!(s.visible, SheetVisible::Visible)))
        .collect();
    if sheets.is_empty() {
        return Err(format!("This {what} file has no sheets"));
    }
    let mut book = Book::new();
    let first = book.id_at(0).ok_or("no sheet")?;
    // Sheets first, so formulas can name them.
    let mut ids = Vec::new();
    for (k, (name, hidden)) in sheets.iter().enumerate() {
        let id = if k == 0 {
            crate::ops::apply(&mut book, &Op::RenameSheet { sheet: first, name: name.clone() })?;
            first
        } else {
            let id = book.next_sheet_id();
            crate::ops::apply(
                &mut book,
                &Op::AddSheet { sheet: id, name: name.clone(), at: k as u32, info: SheetInfo::default() },
            )?;
            id
        };
        if *hidden {
            crate::ops::apply(
                &mut book,
                &Op::SheetInfo { sheet: id, info: SheetInfo { working: true, ..Default::default() } },
            )?;
        }
        ids.push(id);
    }
    for ((name, _), id) in sheets.iter().zip(&ids) {
        let mut cells = Vec::new();
        let formulas = wb.worksheet_formula(name).ok();
        if let Ok(range) = wb.worksheet_range(name) {
            let (r0, c0) = range.start().unwrap_or((0, 0));
            for (dr, dc, v) in range.cells() {
                let (row, col) = ((r0 as usize + dr) as i32 + 1, (c0 as usize + dc) as i32 + 1);
                let data = match v {
                    Data::Empty => continue,
                    Data::Int(n) => CellData::Number(ironcalc_base::num::Num::from(*n)),
                    Data::Float(n) => CellData::Number(ironcalc_base::num::Num::from(*n)),
                    Data::String(s) => CellData::Text(s.clone()),
                    Data::Bool(b) => CellData::Bool(*b),
                    Data::DateTime(d) => CellData::Number(ironcalc_base::num::Num::from(d.as_f64())),
                    Data::DateTimeIso(s) | Data::DurationIso(s) => CellData::Input(s.clone()),
                    Data::Error(e) => CellData::Error(format!("{e}")),
                };
                cells.push(CellEdit { row, col, data: Some(data) });
            }
        }
        if let Some(f) = formulas {
            let (r0, c0) = f.start().unwrap_or((0, 0));
            for (dr, dc, text) in f.cells() {
                if text.is_empty() {
                    continue;
                }
                let (row, col) = ((r0 as usize + dr) as i32 + 1, (c0 as usize + dc) as i32 + 1);
                let formula = if text.starts_with("of:") || text.contains("[.") || text.contains('[') {
                    format!("={}", odf_formula(text))
                } else if text.starts_with('=') {
                    text.clone()
                } else {
                    format!("={text}")
                };
                cells.retain(|c| !(c.row == row && c.col == col));
                cells.push(CellEdit { row, col, data: Some(CellData::Formula(formula)) });
            }
        }
        // A formula that can't be read stays as its text.
        let mut failed = Vec::new();
        for c in cells {
            let ok = book.put(*id, c.row, c.col, c.data.as_ref());
            if ok.is_err()
                && let Some(CellData::Formula(f)) = &c.data
            {
                failed.push(CellEdit { row: c.row, col: c.col, data: Some(CellData::Text(f.clone())) });
            }
        }
        for c in failed {
            let _ = book.put(*id, c.row, c.col, c.data.as_ref());
        }
    }
    book.model.evaluate();
    Ok(book)
}

/// An OpenDocument formula (`of:=SUM([.B2:.B3];[Sheet2.A1])`) as Excel
/// writes it (`SUM(B2:B3,Sheet2!A1)`), without its `=`.
pub fn odf_formula(text: &str) -> String {
    let body = text.trim().trim_start_matches("of:").trim_start_matches("msoxl:").trim_start_matches('=');
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // A string, as it is ("" inside stays).
                out.push(c);
                while let Some(d) = chars.next() {
                    out.push(d);
                    if d == '"' {
                        if chars.peek() == Some(&'"') {
                            out.push(chars.next().unwrap_or('"'));
                        } else {
                            break;
                        }
                    }
                }
            }
            '[' => {
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d);
                }
                let parts: Vec<String> = inner.split(':').map(odf_reference).collect();
                // The sheet is named once, on the first part.
                out.push_str(&parts.join(":"));
            }
            ';' => out.push(','),
            _ => out.push(c),
        }
    }
    out
}

/// `.A1` → `A1`; `Sheet2.A1` → `Sheet2!A1`; `$'My sheet'.$A$1` → `'My sheet'!$A$1`.
fn odf_reference(r: &str) -> String {
    let r = r.trim();
    match r.rfind('.') {
        Some(k) => {
            let (sheet, cell) = (r[..k].trim_start_matches('$'), &r[k + 1..]);
            if sheet.is_empty() { cell.to_string() } else { format!("{sheet}!{cell}") }
        }
        None => r.to_string(),
    }
}
