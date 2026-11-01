//! What other documents can show of a book (Write's Insert › From Calc):
//! each table, summary and chart, and each sheet's cells, as shown. Calc
//! puts this on the server (beside the book) each time it's saved,
//! and while people edit it together; Write fills its linked tables from it.

use crate::book::{Book, SheetKind};
use crate::tables;
use serde_json::{Value as Json, json};

/// How many rows and columns of a sheet go in at most.
const MOST_ROWS: i32 = 500;
const MOST_COLS: i32 = 40;

fn shown(book: &Book, i: u32, top: i32, left: i32, bottom: i32, right: i32) -> Vec<Vec<String>> {
    (top..=bottom.min(top + MOST_ROWS - 1))
        .filter(|r| !book.model.is_row_hidden(i, *r).unwrap_or(false))
        .map(|r| {
            (left..=right.min(left + MOST_COLS - 1))
                .map(|c| book.model.get_formatted_cell_value(i, r, c).unwrap_or_default())
                .collect()
        })
        .collect()
}

/// Which columns hold numbers (to line them up on the right).
fn numeric(book: &Book, i: u32, top: i32, left: i32, bottom: i32, right: i32) -> Vec<bool> {
    (left..=right.min(left + MOST_COLS - 1))
        .map(|c| {
            let mut nums = 0;
            let mut other = 0;
            for r in top..=bottom.min(top + 50) {
                match book.model.get_cell_value_by_index(i, r, c) {
                    Ok(ironcalc_base::cell::CellValue::Number(_)) => nums += 1,
                    Ok(ironcalc_base::cell::CellValue::String(s)) if !s.is_empty() => other += 1,
                    _ => {}
                }
            }
            nums > other
        })
        .collect()
}

pub fn snapshot(book: &Book) -> Json {
    let mut items = Vec::new();
    for t in tables::all(book) {
        let Ok(i) = book.index(t.sheet) else { continue };
        items.push(json!({
            "id": format!("table:{}", t.name), "kind": "table", "name": t.name, "sheet": book.sheet_name(t.sheet).unwrap_or_default(),
            "rows": shown(book, i, t.top, t.left, t.bottom, t.right), "header": t.header, "footer": t.totals,
            "numbers": numeric(book, i, t.top + 1, t.left, t.bottom, t.right),
        }));
    }
    for w in &book.model.workbook.worksheets {
        let info = book.info(w.sheet_id);
        let Ok(i) = book.index(w.sheet_id) else { continue };
        let d = w.dimension();
        if d.max_row < 1 || d.max_column < 1 || matches!(info.kind, SheetKind::Report) {
            continue;
        }
        let summary = matches!(info.kind, SheetKind::Summary);
        items.push(json!({
            "id": format!("sheet:{}", w.sheet_id), "kind": if summary { "summary" } else { "sheet" }, "name": w.get_name(), "sheet": w.get_name(),
            "rows": shown(book, i, 1, 1, d.max_row, d.max_column), "header": summary || tables::has_header(book, w.sheet_id, 1, 1, d.max_row, d.max_column),
            "footer": summary && book.summaries.get(&w.sheet_id).is_some_and(|s| s.totals),
            "numbers": numeric(book, i, 2, 1, d.max_row, d.max_column),
        }));
    }
    for c in &book.charts {
        items.push(json!({ "id": format!("chart:{}", c.id), "kind": "chart", "name": if c.title.is_empty() { "Chart".to_string() } else { c.title.clone() }, "sheet": book.sheet_name(c.sheet).unwrap_or_default(), "chart": c.id }));
    }
    json!({ "format": "xenify-calc-link", "version": 1, "items": items })
}
