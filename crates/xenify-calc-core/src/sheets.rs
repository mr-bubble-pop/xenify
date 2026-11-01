//! What the All sheets page shows about each sheet, and copying a sheet.

use crate::book::Book;
use crate::ops::{CellEdit, LineSize, Op, StyleEdit};
use crate::tables;
use serde::Serialize;

#[derive(Serialize)]
pub struct Facts {
    pub id: u32,
    pub name: String,
    pub kind: crate::book::SheetKind,
    pub working: bool,
    /// Rows and columns in use.
    pub rows: i32,
    pub cols: i32,
    pub tables: Vec<String>,
    /// Sheets with formulas that use this one (by name or its tables).
    pub used_by: Vec<String>,
}

/// Whether `formula` names sheet `name` (Sales!A1, 'Shop sales'!A1).
fn names_sheet(formula: &str, name: &str) -> bool {
    let f = formula.to_lowercase();
    let n = name.to_lowercase();
    f.contains(&format!("{n}!")) || f.contains(&format!("'{}'!", n.replace('\'', "''")))
}

pub fn facts(book: &Book) -> Vec<Facts> {
    let formulas = book.formulas();
    let all_tables = tables::all(book);
    book.model
        .workbook
        .worksheets
        .iter()
        .map(|w| {
            let id = w.sheet_id;
            let name = w.get_name();
            let mine: Vec<String> = all_tables.iter().filter(|t| t.sheet == id).map(|t| t.name.clone()).collect();
            let mut used_by: Vec<String> = Vec::new();
            for ((sheet, _, _), f) in &formulas {
                if *sheet == id {
                    continue;
                }
                let uses = names_sheet(f, &name)
                    || mine.iter().any(|t| f.to_lowercase().contains(&format!("{}[", t.to_lowercase())));
                if uses {
                    let other = book.sheet_name(*sheet).unwrap_or_default();
                    if !used_by.contains(&other) {
                        used_by.push(other);
                    }
                }
            }
            used_by.sort();
            let d = w.dimension();
            let empty = w.sheet_data.is_empty();
            let info = book.info(id);
            Facts {
                id,
                name,
                kind: info.kind,
                working: info.working,
                rows: if empty { 0 } else { d.max_row },
                cols: if empty { 0 } else { d.max_column },
                tables: mine,
                used_by,
            }
        })
        .collect()
}

/// A free sheet name from `wanted`: "Orders (2)", "Orders (3)"…
pub fn free_name(book: &Book, wanted: &str) -> String {
    let taken = |n: &str| book.model.workbook.worksheets.iter().any(|w| w.get_name().eq_ignore_ascii_case(n));
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2..).map(|k| format!("{wanted} ({k})")).find(|n| !taken(n)).unwrap_or_default()
}

/// The ops that copy sheet `sheet` (cells, looks, sizes, frozen panes and merges)
/// to a new sheet just after it, and the new sheet's id. Its tables become
/// plain cells in the copy (table names are the book's, not the sheet's).
pub fn duplicate(book: &Book, sheet: u32) -> Result<(u32, Vec<Op>), String> {
    let i = book.index(sheet)?;
    let id = book.next_sheet_id();
    let name = free_name(book, &book.sheet_name(sheet)?);
    let mut info = book.info(sheet);
    info.working = false;
    let w = &book.model.workbook.worksheets[i as usize];
    let mut ops = vec![Op::AddSheet { sheet: id, name, at: i + 1, info }];
    let cells = book.cells_in(sheet, None, None);
    let plain = ironcalc_base::types::Style::default();
    let styles: Vec<StyleEdit> =
        cells.iter().filter(|c| c.3 != plain).map(|c| StyleEdit { row: c.0, col: c.1, style: c.3.clone() }).collect();
    if !styles.is_empty() {
        ops.push(Op::Styles { sheet: id, cells: styles });
    }
    let data: Vec<CellEdit> =
        cells.into_iter().filter(|c| c.2.is_some()).map(|(row, col, data, _)| CellEdit { row, col, data }).collect();
    if !data.is_empty() {
        ops.push(Op::Cells { sheet: id, cells: data });
    }
    let rows: Vec<LineSize> = w.rows.iter().filter(|r| r.custom_height || r.hidden).map(crate::ops::line_of).collect();
    if !rows.is_empty() {
        ops.push(Op::Rows { sheet: id, rows });
    }
    let mut columns = Vec::new();
    for c in w.cols.iter().filter(|c| c.custom_width || c.hidden) {
        for at in c.min..=c.max.min(c.min + 500) {
            columns.push(LineSize {
                at,
                size: c.custom_width.then(|| w.get_actual_column_width(at).ok()).flatten(),
                hidden: c.hidden,
            });
        }
    }
    if !columns.is_empty() {
        ops.push(Op::Columns { sheet: id, columns });
    }
    if w.frozen_rows > 0 || w.frozen_columns > 0 {
        ops.push(Op::Freeze { sheet: id, rows: w.frozen_rows, columns: w.frozen_columns });
    }
    if !w.merged_cells.is_empty() {
        ops.push(Op::Merged { sheet: id, cells: w.merged_cells.clone() });
    }
    Ok((id, ops))
}
