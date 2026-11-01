use crate::*;
use serde_json::json;

fn text(s: &Session, sheet: u32, row: i32, col: i32) -> String {
    let i = s.book.index(sheet).unwrap();
    s.book.model.get_formatted_cell_value(i, row, col).unwrap()
}

fn input(s: &Session, sheet: u32, row: i32, col: i32) -> String {
    let i = s.book.index(sheet).unwrap();
    s.book.model.get_localized_cell_content(i, row, col).unwrap()
}

/// Months and revenue in A1:B4 of the first sheet.
fn sales() -> (Session, u32) {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    s.call("rename_sheet", &json!({ "sheet": sheet, "name": "Sales" })).unwrap();
    for (r, (m, v)) in [("Month", "Revenue"), ("Jan", "10"), ("Feb", "20"), ("Mar", "30")].iter().enumerate() {
        s.type_in(sheet, r as i32 + 1, 1, m).unwrap();
        s.type_in(sheet, r as i32 + 1, 2, v).unwrap();
    }
    (s, sheet)
}

#[test]
fn typing_and_formulas() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 6, 2, "=SUM(B2:B4)").unwrap();
    assert_eq!(text(&s, sheet, 6, 2), "60");
    s.type_in(sheet, 2, 2, "15").unwrap();
    assert_eq!(text(&s, sheet, 6, 2), "65");
    s.undo().unwrap();
    assert_eq!(text(&s, sheet, 6, 2), "60");
    s.redo().unwrap();
    assert_eq!(text(&s, sheet, 6, 2), "65");
}

#[test]
fn header_and_block_found() {
    let (s, sheet) = sales();
    assert_eq!(tables::block(&s.book, sheet, 3, 2), (1, 1, 4, 2));
    assert!(tables::has_header(&s.book, sheet, 1, 1, 4, 2));
    // Only words: no header to be sure of.
    let mut t = Session::new();
    let id = t.book.id_at(0).unwrap();
    for r in 1..=3 {
        t.type_in(id, r, 1, &format!("name {r}")).unwrap();
    }
    assert!(!tables::has_header(&t.book, id, 1, 1, 3, 1));
}

#[test]
fn totals_row_grows_with_the_table() {
    let (mut s, sheet) = sales();
    let t = s.call("make_table", &json!({ "sheet": sheet, "row": 2, "col": 2 })).unwrap();
    assert_eq!(t["name"], "Sales");
    s.call("total", &json!({ "table": "Sales", "column": "Revenue", "total": "sum" })).unwrap();
    assert_eq!(input(&s, sheet, 5, 2), "=SUBTOTAL(109,Sales[Revenue])");
    assert_eq!(text(&s, sheet, 5, 2), "60");
    assert_eq!(text(&s, sheet, 5, 1), "3 months");

    // The "type to add a row" line: a row above the totals row.
    s.call("add_row", &json!({ "table": "Sales", "values": { "Month": "Apr", "Revenue": "40" } })).unwrap();
    assert_eq!(text(&s, sheet, 5, 1), "Apr");
    assert_eq!(text(&s, sheet, 6, 2), "100");
    assert_eq!(text(&s, sheet, 6, 1), "4 months");
    assert_eq!(input(&s, sheet, 6, 2), "=SUBTOTAL(109,Sales[Revenue])");
    let t = tables::get(&s.book, "Sales").unwrap();
    assert_eq!((t.top, t.bottom), (1, 6));

    // Inserting rows inside the table grows it too.
    s.call("insert_rows", &json!({ "sheet": sheet, "at": 3 })).unwrap();
    s.type_in(sheet, 3, 2, "5").unwrap();
    assert_eq!(text(&s, sheet, 7, 2), "105");

    // Undo all the way back to before the totals row.
    for _ in 0..3 {
        s.undo().unwrap();
    }
    assert_eq!(text(&s, sheet, 5, 2), "60");
    s.undo().unwrap();
    assert_eq!(text(&s, sheet, 5, 2), "");
    assert!(!tables::get(&s.book, "Sales").unwrap().totals);
}

#[test]
fn typing_under_a_table_without_totals_takes_the_row_in() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.type_in(sheet, 7, 4, "=SUM(Sales[Revenue])").unwrap();
    s.type_in(sheet, 5, 2, "40").unwrap();
    assert_eq!(tables::get(&s.book, "Sales").unwrap().bottom, 5);
    assert_eq!(text(&s, sheet, 7, 4), "100");
    // Further down is outside.
    s.type_in(sheet, 9, 2, "1000").unwrap();
    assert_eq!(text(&s, sheet, 7, 4), "100");
}

#[test]
fn calculated_columns_fill_new_rows() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 1, 3, "Double").unwrap();
    for r in 2..=4 {
        s.type_in(sheet, r, 3, &format!("=B{r}*2")).unwrap();
    }
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("add_row", &json!({ "table": "Sales", "values": { "Month": "Apr", "Revenue": "7" } })).unwrap();
    assert_eq!(input(&s, sheet, 5, 3), "=B5*2");
    assert_eq!(text(&s, sheet, 5, 3), "14");
}

#[test]
fn deleting_rows_undoes_exactly() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 6, 2, "=B3*10").unwrap();
    s.type_in(sheet, 7, 2, "=SUM(B2:B4)").unwrap();
    s.call("delete_rows", &json!({ "sheet": sheet, "at": 3 })).unwrap();
    assert_eq!(input(&s, sheet, 5, 2), "=#REF!*10");
    assert_eq!(text(&s, sheet, 6, 2), "40");
    s.undo().unwrap();
    assert_eq!(input(&s, sheet, 3, 1), "Feb");
    assert_eq!(input(&s, sheet, 6, 2), "=B3*10");
    assert_eq!(text(&s, sheet, 6, 2), "200");
    assert_eq!(text(&s, sheet, 7, 2), "60");
    s.redo().unwrap();
    assert_eq!(input(&s, sheet, 5, 2), "=#REF!*10");
}

#[test]
fn sheets_add_rename_remove_and_undo() {
    let (mut s, sheet) = sales();
    let other = s.call("add_sheet", &json!({ "name": "Summary" })).unwrap().as_u64().unwrap() as u32;
    s.type_in(other, 1, 1, "=SUM(Sales!B2:B4)").unwrap();
    assert_eq!(text(&s, other, 1, 1), "60");
    s.call("rename_sheet", &json!({ "sheet": sheet, "name": "Money" })).unwrap();
    assert_eq!(input(&s, other, 1, 1), "=SUM(Money!B2:B4)");
    s.call("remove_sheet", &json!({ "sheet": sheet })).unwrap();
    assert_eq!(text(&s, other, 1, 1), "#REF!");
    s.undo().unwrap();
    assert_eq!(input(&s, other, 1, 1), "=SUM(Money!B2:B4)");
    assert_eq!(text(&s, other, 1, 1), "60");
    assert_eq!(s.book.index(sheet).unwrap(), 0);
}

#[test]
fn working_sheets_are_hidden_in_the_workbook() {
    let (mut s, sheet) = sales();
    s.call("working", &json!({ "sheet": sheet, "working": true })).unwrap();
    assert!(s.book.info(sheet).working);
    assert_eq!(s.book.model.workbook.worksheets[0].state, ironcalc_base::types::SheetState::Hidden);
    s.undo().unwrap();
    assert_eq!(s.book.model.workbook.worksheets[0].state, ironcalc_base::types::SheetState::Visible);
}

#[test]
fn query_sheets_are_locked() {
    let (mut s, sheet) = sales();
    s.edit(&[Op::SheetInfo { sheet, info: SheetInfo { kind: SheetKind::Query, working: true } }]).unwrap();
    assert!(s.type_in(sheet, 1, 1, "x").is_err());
}

#[test]
fn save_and_open_keep_everything() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("total", &json!({ "table": "Sales", "column": "Revenue", "total": "average" })).unwrap();
    let summary = s.call("add_sheet", &json!({ "name": "Summary" })).unwrap().as_u64().unwrap() as u32;
    s.type_in(summary, 2, 2, "=XLOOKUP(\"Feb\",Sales[Month],Sales[Revenue])").unwrap();
    s.type_in(summary, 3, 2, "'0012").unwrap();
    s.type_in(summary, 4, 2, "TRUE").unwrap();
    let mut bold = s.book.style(summary, 2, 2);
    bold.font.b = true;
    bold.num_fmt = "#,##0.00".to_string();
    s.edit(&[
        Op::Styles { sheet: summary, cells: vec![ops::StyleEdit { row: 2, col: 2, style: bold.clone() }] },
        Op::Columns { sheet: summary, columns: vec![ops::LineSize { at: 2, size: Some(140.0), hidden: false }] },
        Op::Freeze { sheet: summary, rows: 1, columns: 0 },
        Op::SheetInfo { sheet, info: SheetInfo { kind: SheetKind::Normal, working: true } },
        Op::Note {
            target: NoteTarget::Table { table: "Sales".into(), column: "Revenue".into() },
            note: Some(ColumnNote {
                meaning: "Money taken, before refunds".into(),
                unit: "GBP".into(),
                with_tax: Some(false),
                ..Default::default()
            }),
        },
        Op::Definition {
            name: "Profit".into(),
            definition: Some(Definition {
                name: "Profit".into(),
                formula: "=Sales[Revenue]-Sales[Costs]".into(),
                about: "Before tax".into(),
            }),
        },
    ])
    .unwrap();
    assert!(s.unsaved());
    let bytes = s.save().unwrap();
    assert!(!s.unsaved());

    let o = Session::open(&bytes).unwrap();
    assert_eq!(text(&o, summary, 2, 2), "20.00");
    assert_eq!(input(&o, summary, 2, 2), "=XLOOKUP(\"Feb\",Sales[Month],Sales[Revenue])");
    assert_eq!(input(&o, summary, 3, 2), "'0012");
    assert_eq!(text(&o, summary, 4, 2), "TRUE");
    assert_eq!(o.book.style(summary, 2, 2), bold);
    let i = o.book.index(summary).unwrap();
    assert_eq!(o.book.model.get_column_width(i, 2).unwrap(), 140.0);
    assert_eq!(o.book.model.get_frozen_rows_count(i).unwrap(), 1);
    assert!(o.book.info(sheet).working);
    assert_eq!(tables::get(&o.book, "Sales"), tables::get(&s.book, "Sales"));
    assert_eq!(text(&o, sheet, 5, 2), "20");
    assert_eq!(o.book.notes, s.book.notes);
    assert_eq!(o.book.definitions, s.book.definitions);
    assert_eq!(o.sheets_json(), s.sheets_json());
}

#[test]
fn view_lists_what_the_grid_shows() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    let v = s.call("view", &json!({ "sheet": sheet, "top": 1, "left": 1, "bottom": 20, "right": 10 })).unwrap();
    assert_eq!(v["cells"].as_array().unwrap().len(), 8);
    assert_eq!(v["tables"][0]["name"], "Sales");
    assert_eq!(v["widths"].as_array().unwrap().len(), 10);
}

#[test]
fn table_names_are_checked() {
    assert!(tables::valid_name("Sales"));
    assert!(tables::valid_name("Sales_2026"));
    assert!(!tables::valid_name("A1"));
    assert!(!tables::valid_name("2026"));
    assert!(!tables::valid_name("Shop sales"));
    let (s, _) = sales();
    assert_eq!(tables::free_name(&s.book, "Shop sales!"), "Shopsales");
    assert_eq!(tables::free_name(&s.book, "A1"), "Table1");
}

#[test]
fn renaming_a_header_renames_the_column_everywhere() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("total", &json!({ "table": "Sales", "column": "Revenue", "total": "sum" })).unwrap();
    s.type_in(sheet, 8, 4, "=XLOOKUP(\"Feb\",Sales[Month],Sales[Revenue])").unwrap();
    s.edit(&[Op::Note {
        target: NoteTarget::Table { table: "Sales".into(), column: "Revenue".into() },
        note: Some(ColumnNote { meaning: "Money in".into(), ..Default::default() }),
    }])
    .unwrap();
    s.type_in(sheet, 1, 2, "Takings").unwrap();
    assert_eq!(input(&s, sheet, 5, 2), "=SUBTOTAL(109,Sales[Takings])");
    assert_eq!(input(&s, sheet, 8, 4), "=XLOOKUP(\"Feb\",Sales[Month],Sales[Takings])");
    assert_eq!(text(&s, sheet, 8, 4), "20");
    assert_eq!(s.book.notes[0].target, NoteTarget::Table { table: "Sales".into(), column: "Takings".into() });
    // A name already used gets a number.
    s.type_in(sheet, 1, 2, "Month").unwrap();
    assert_eq!(input(&s, sheet, 1, 2), "Month1");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(input(&s, sheet, 5, 2), "=SUBTOTAL(109,Sales[Revenue])");
    assert_eq!(input(&s, sheet, 1, 2), "Revenue");
    assert_eq!(s.book.notes[0].target, NoteTarget::Table { table: "Sales".into(), column: "Revenue".into() });
}

#[test]
fn renaming_a_table_renames_it_in_formulas() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.type_in(sheet, 8, 4, "=SUM(Sales[Revenue])").unwrap();
    let mut t = tables::get(&s.book, "Sales").unwrap();
    t.name = "Money".into();
    s.edit(&[Op::Table { name: "Sales".into(), table: Some(t) }]).unwrap();
    assert_eq!(input(&s, sheet, 8, 4), "=SUM(Money[Revenue])");
    assert_eq!(text(&s, sheet, 8, 4), "60");
    s.undo().unwrap();
    assert_eq!(input(&s, sheet, 8, 4), "=SUM(Sales[Revenue])");
    assert!(tables::get(&s.book, "Money").is_none());
}

fn range(top: i32, left: i32, bottom: i32, right: i32) -> serde_json::Value {
    json!({ "top": top, "left": left, "bottom": bottom, "right": right })
}

#[test]
fn formats_and_decimals() {
    let (mut s, sheet) = sales();
    s.call(
        "format",
        &json!({ "sheet": sheet, "range": range(1, 1, 1, 2), "set": { "bold": true, "fill": "#FFF1E8" } }),
    )
    .unwrap();
    assert!(s.book.style(sheet, 1, 2).font.b);
    s.call(
        "format",
        &json!({ "sheet": sheet, "range": range(2, 2, 4, 2), "set": { "num_fmt": "#,##0", "decimals": 2 } }),
    )
    .unwrap();
    assert_eq!(text(&s, sheet, 2, 2), "10.00");
    s.call("format", &json!({ "sheet": sheet, "range": range(2, 2, 4, 2), "set": { "decimals": -1 } })).unwrap();
    assert_eq!(text(&s, sheet, 2, 2), "10.0");
    assert_eq!(edit::change_decimals("General", 1), "0.0");
    assert_eq!(edit::change_decimals("0.00%", -2), "0%");
    assert_eq!(edit::change_decimals("#,##0;[Red]-#,##0", 1), "#,##0.0;[Red]-#,##0");
    let v = s.call("view", &json!({ "sheet": sheet, "top": 1, "left": 1, "bottom": 1, "right": 2 })).unwrap();
    assert_eq!(v["cells"][1]["look"]["b"], true);
    assert_eq!(v["cells"][1]["look"]["fill"], "#FFF1E8");
    s.call("clear", &json!({ "sheet": sheet, "range": range(1, 1, 4, 2), "what": "formats" })).unwrap();
    assert!(!s.book.style(sheet, 1, 2).font.b);
    assert_eq!(text(&s, sheet, 2, 2), "10");
}

#[test]
fn sorting_keeps_rows_together_and_moves_formulas() {
    let (mut s, sheet) = sales();
    for r in 2..=4 {
        s.type_in(sheet, r, 3, &format!("=B{r}*2")).unwrap();
    }
    s.call(
        "sort",
        &json!({ "sheet": sheet, "range": range(1, 1, 4, 3), "by": [{ "col": 2, "desc": true }], "header": true }),
    )
    .unwrap();
    assert_eq!(text(&s, sheet, 2, 1), "Mar");
    assert_eq!(text(&s, sheet, 4, 1), "Jan");
    assert_eq!(input(&s, sheet, 2, 3), "=B2*2");
    assert_eq!(text(&s, sheet, 2, 3), "60");
    // Words, then the empty cell last.
    s.type_in(sheet, 5, 1, "Apr").unwrap();
    s.call("sort", &json!({ "sheet": sheet, "range": range(2, 1, 5, 2), "by": [{ "col": 2 }] })).unwrap();
    assert_eq!(text(&s, sheet, 2, 1), "Jan");
    assert_eq!(text(&s, sheet, 5, 1), "Apr");
    s.undo().unwrap();
    assert_eq!(text(&s, sheet, 2, 1), "Mar");
}

#[test]
fn filters_hide_rows_and_totals_follow() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("total", &json!({ "table": "Sales", "column": "Revenue", "total": "sum" })).unwrap();
    let vals = s.call("column_values", &json!({ "table": "Sales", "column": "Month" })).unwrap();
    assert_eq!(vals["values"].as_array().unwrap().len(), 3);
    s.call("filter", &json!({ "table": "Sales", "column": "Month", "keep": ["Jan", "Mar"] })).unwrap();
    let i = s.book.index(sheet).unwrap();
    assert!(s.book.model.is_row_hidden(i, 3).unwrap());
    assert_eq!(text(&s, sheet, 5, 2), "40");
    assert_eq!(text(&s, sheet, 5, 1), "2 months");
    let st = s.call("stats", &json!({ "sheet": sheet, "range": range(2, 2, 4, 2) })).unwrap();
    assert_eq!(st["sum"], 40.0);
    let bytes = s.save().unwrap();
    let o = Session::open(&bytes).unwrap();
    assert_eq!(o.book.filters, s.book.filters);
    s.call("filter", &json!({ "table": "Sales", "column": "Month", "keep": null })).unwrap();
    assert!(!s.book.model.is_row_hidden(i, 3).unwrap());
    assert_eq!(text(&s, sheet, 5, 2), "60");
}

#[test]
fn find_and_replace() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 6, 2, "=SUM(B2:B4)").unwrap();
    let f = s.call("find", &json!({ "query": "60" })).unwrap();
    assert_eq!(f.as_array().unwrap().len(), 1);
    let f = s.call("find", &json!({ "query": "sum", "options": { "formulas": true } })).unwrap();
    assert_eq!(f[0]["row"], 6);
    let n = s.call("replace", &json!({ "query": "ja", "with": "Jan-", "options": { "whole": false } })).unwrap();
    assert_eq!(n, 1);
    assert_eq!(text(&s, sheet, 2, 1), "Jan-n");
    let n = s
        .call(
            "replace",
            &json!({ "query": "Feb", "with": "February", "options": { "whole": true, "match_case": true } }),
        )
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(text(&s, sheet, 3, 1), "February");
}

#[test]
fn copy_and_paste() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 2, 3, "=B2*2").unwrap();
    s.call("format", &json!({ "sheet": sheet, "range": range(2, 3, 2, 3), "set": { "bold": true } })).unwrap();
    let clip = s.call("copy", &json!({ "sheet": sheet, "range": range(2, 2, 2, 3) })).unwrap();
    assert_eq!(clip["text"], "10\t20");
    s.call("paste", &json!({ "sheet": sheet, "row": 4, "col": 5, "clip": clip })).unwrap();
    assert_eq!(input(&s, sheet, 4, 6), "=E4*2");
    assert!(s.book.style(sheet, 4, 6).font.b);
    s.call("paste", &json!({ "sheet": sheet, "row": 8, "col": 1, "clip": clip, "values": true })).unwrap();
    assert_eq!(input(&s, sheet, 8, 2), "20");
    s.call("paste", &json!({ "sheet": sheet, "row": 10, "col": 1, "text": "a\t1\nb\t2\n" })).unwrap();
    assert_eq!(text(&s, sheet, 11, 2), "2");
}

#[test]
fn ctrl_arrows_jump_like_other_spreadsheets() {
    let (mut s, sheet) = sales();
    let edge = |s: &mut Session, r, c, dr, dc| {
        let v = s.call("edge", &json!({ "sheet": sheet, "row": r, "col": c, "dr": dr, "dc": dc })).unwrap();
        (v["row"].as_i64().unwrap(), v["col"].as_i64().unwrap())
    };
    assert_eq!(edge(&mut s, 1, 1, 1, 0), (4, 1));
    assert_eq!(edge(&mut s, 4, 1, 1, 0), (1_048_576, 1));
    assert_eq!(edge(&mut s, 1, 1, 0, 1), (1, 2));
    s.type_in(sheet, 9, 1, "x").unwrap();
    assert_eq!(edge(&mut s, 4, 1, 1, 0), (9, 1));
    assert_eq!(edge(&mut s, 9, 1, -1, 0), (4, 1));
    let l = s.call("layout", &json!({ "sheet": sheet })).unwrap();
    assert_eq!(l["used"][0], 9);
}

#[test]
fn look_up_by_example() {
    let mut s = Session::new();
    let orders = s.book.id_at(0).unwrap();
    s.call("rename_sheet", &json!({ "sheet": orders, "name": "Orders" })).unwrap();
    for (r, (o, c)) in [("Order", "Customer"), ("A-1", "1284"), ("A-2", "1279"), ("A-3", "9999")].iter().enumerate() {
        s.type_in(orders, r as i32 + 1, 1, o).unwrap();
        s.type_in(orders, r as i32 + 1, 2, c).unwrap();
    }
    let cust = s.call("add_sheet", &json!({ "name": "Customers" })).unwrap().as_u64().unwrap() as u32;
    for (r, row) in [["id", "name", "city"], ["1279", "Ann", "Skipton"], ["1284", "Bo", "York"]].iter().enumerate() {
        for (k, v) in row.iter().enumerate() {
            s.type_in(cust, r as i32 + 1, k as i32 + 1, v).unwrap();
        }
    }
    s.call("make_table", &json!({ "sheet": cust, "row": 1, "col": 1 })).unwrap();
    // Cursor in C2 of Orders: answers go beside A1:B4.
    let yours = s.call("lookup_rows", &json!({ "sheet": orders, "row": 2, "col": 3 })).unwrap();
    assert_eq!(
        (yours["top"].as_i64(), yours["right"].as_i64(), yours["header"].as_bool()),
        (Some(1), Some(2), Some(true))
    );
    let look = s.call("lookup_in", &json!({ "sheet": cust, "row": 3, "col": 2 })).unwrap();
    assert_eq!(look["table"], "Customers");
    // Clicked the id column: Customer matches 2 of 3.
    let c = s.call("lookup_candidates", &json!({ "yours": yours, "look": look, "on": 1 })).unwrap();
    assert_eq!(c[0]["name"], "Customer");
    assert_eq!((c[0]["matches"].as_u64(), c[0]["rows"].as_u64()), (Some(2), Some(3)));
    let plan =
        json!({ "yours": yours, "key": 2, "look": look, "on": 1, "ret": 3, "target": 3, "missing": "not found" });
    let p = s.call("lookup_preview", &json!({ "plan": plan })).unwrap();
    assert_eq!(p["rows"][0]["answer"], "York");
    assert!(p["rows"][2]["answer"].is_null());
    assert_eq!(p["formula"], "=XLOOKUP(B2,Customers[id],Customers[city],\"not found\")");
    s.call("lookup_fill", &json!({ "plan": plan })).unwrap();
    assert_eq!(text(&s, orders, 1, 3), "city");
    assert_eq!(text(&s, orders, 2, 3), "York");
    assert_eq!(text(&s, orders, 3, 3), "Skipton");
    assert_eq!(text(&s, orders, 4, 3), "not found");
    // A new customer in the table is found without changing the formulas.
    s.call("add_row", &json!({ "table": "Customers", "values": { "id": "9999", "name": "Cy", "city": "Leeds" } }))
        .unwrap();
    assert_eq!(text(&s, orders, 4, 3), "Leeds");
}

#[test]
fn number_summary() {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    for (k, v) in [18240, 16910, 21480, 22950, 24310, 26780, 25120, 23890, 27640, 29310].iter().enumerate() {
        s.type_in(sheet, k as i32 + 1, 1, &v.to_string()).unwrap();
    }
    let st = s.call("stats", &json!({ "sheet": sheet, "range": range(1, 1, 10, 1) })).unwrap();
    assert_eq!(st["sum"], 236630.0);
    assert_eq!(st["median"], 24100.0);
    let bins = &st["bins"];
    assert_eq!(bins["step"], 2500.0);
    assert_eq!(bins["start"], 15000.0);
    assert_eq!(bins["counts"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).sum::<u64>(), 10);
}

#[test]
fn sheet_facts_and_duplicates() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    let summary = s.call("add_sheet", &json!({ "name": "Summary" })).unwrap().as_u64().unwrap() as u32;
    s.type_in(summary, 1, 1, "=SUM(Sales[Revenue])").unwrap();
    let other = s.call("add_sheet", &json!({ "name": "Notes" })).unwrap().as_u64().unwrap() as u32;
    s.type_in(other, 1, 1, "=Sales!B2").unwrap();
    let f = s.call("sheet_facts", &json!({})).unwrap();
    assert_eq!(f[0]["name"], "Sales");
    assert_eq!(f[0]["rows"], 4);
    assert_eq!(f[0]["tables"][0], "Sales");
    assert_eq!(f[0]["used_by"], json!(["Notes", "Summary"]));
    let copy = s.call("duplicate_sheet", &json!({ "sheet": summary })).unwrap().as_u64().unwrap() as u32;
    assert_eq!(s.book.sheet_name(copy).unwrap(), "Summary (2)");
    assert_eq!(text(&s, copy, 1, 1), "60");
    assert_eq!(s.book.index(copy).unwrap(), 2);
    s.undo().unwrap();
    assert!(s.book.index(copy).is_err());
}

#[test]
fn charts_grow_with_tables_and_move_with_rows() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    let c = s.call("add_chart", &json!({ "sheet": sheet, "range": range(1, 2, 1, 2), "row": 2, "col": 5 })).unwrap();
    assert_eq!(c["kind"], "bar");
    assert_eq!(c["source"]["table"], "Sales");
    assert_eq!(c["source"]["x"], "Month");
    let id = c["id"].as_u64().unwrap();
    let d = s.call("chart_data", &json!({ "id": id })).unwrap();
    assert_eq!(d["labels"], json!(["Jan", "Feb", "Mar"]));
    assert_eq!(d["series"][0]["values"], json!([10.0, 20.0, 30.0]));
    // A new row: in the chart.
    s.type_in(sheet, 5, 1, "Apr").unwrap();
    s.type_in(sheet, 5, 2, "40").unwrap();
    let d = s.call("chart_data", &json!({ "id": id })).unwrap();
    assert_eq!(d["labels"].as_array().unwrap().len(), 4);
    // Filtered rows are left out.
    s.call("filter", &json!({ "table": "Sales", "column": "Month", "keep": ["Jan", "Apr"] })).unwrap();
    let d = s.call("chart_data", &json!({ "id": id })).unwrap();
    assert_eq!(d["labels"], json!(["Jan", "Apr"]));
    // Rows inserted above move it down; undo puts it back.
    s.call("insert_rows", &json!({ "sheet": sheet, "at": 1, "count": 2 })).unwrap();
    assert_eq!(s.book.charts[0].row, 4);
    s.call("delete_rows", &json!({ "sheet": sheet, "at": 1, "count": 2 })).unwrap();
    assert_eq!(s.book.charts[0].row, 2);
    s.undo().unwrap();
    assert_eq!(s.book.charts[0].row, 4);
    let o = Session::open(&s.save().unwrap()).unwrap();
    assert_eq!(o.book.charts, s.book.charts);
    // Cells, not a table: a histogram of one column of numbers.
    let mut t = Session::new();
    let id2 = t.book.id_at(0).unwrap();
    t.type_in(id2, 1, 1, "Weight").unwrap();
    for r in 2..10 {
        t.type_in(id2, r, 1, &(r * 3).to_string()).unwrap();
    }
    let h = t.call("add_chart", &json!({ "sheet": id2, "range": range(1, 1, 9, 1), "row": 1, "col": 3 })).unwrap();
    assert_eq!(h["kind"], "histogram");
    t.call("insert_rows", &json!({ "sheet": id2, "at": 5 })).unwrap();
    assert_eq!(t.book.charts[0].source.range.unwrap().bottom, 10);
}

#[test]
fn excel_files_keep_tables_totals_and_working_sheets() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("total", &json!({ "table": "Sales", "column": "Revenue", "total": "sum" })).unwrap();
    s.call("filter", &json!({ "table": "Sales", "column": "Month", "keep": ["Jan", "Mar"] })).unwrap();
    let helper = s.call("add_sheet", &json!({ "name": "Helper" })).unwrap().as_u64().unwrap() as u32;
    s.type_in(helper, 1, 1, "=SUM(Sales[Revenue])*2").unwrap();
    s.call("working", &json!({ "sheet": helper, "working": true })).unwrap();
    s.edit(&[Op::Definition {
        name: "Profit".into(),
        definition: Some(Definition { formula: "=Sales[Revenue]".into(), ..Default::default() }),
    }])
    .unwrap();
    let bytes = s.save_as(Format::Xlsx, None).unwrap();
    assert!(!s.unsaved());
    assert_eq!(s.format, Format::Xlsx);

    // The parts Excel reads.
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut read = |n: &str| {
        let mut t = String::new();
        std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut t).unwrap();
        t
    };
    let table = read("xl/tables/table1.xml");
    assert!(table.contains("name=\"Sales\""), "{table}");
    assert!(table.contains("ref=\"A1:B5\""), "{table}");
    assert!(table.contains("totalsRowFunction=\"sum\""), "{table}");
    assert!(table.contains("<filter val=\"Jan\"/>"), "{table}");
    let sheet1 = read("xl/worksheets/sheet1.xml");
    assert!(sheet1.contains("<tableParts count=\"1\">"), "{sheet1}");
    assert!(sheet1.contains("SUBTOTAL(109,Sales[Revenue])"), "{sheet1}");
    assert!(read("xl/workbook.xml").contains("state=\"hidden\""));
    assert!(read("[Content_Types].xml").contains("/xl/tables/table1.xml"));

    // And back.
    let o = Session::open_file(&bytes, "Shop.xlsx").unwrap();
    let t = tables::get(&o.book, "Sales").unwrap();
    assert!(t.totals);
    assert_eq!(t.columns[1].total, Some(Total::Sum));
    assert_eq!(input(&o, sheet, 5, 2), "=SUBTOTAL(109,Sales[Revenue])");
    assert_eq!(text(&o, sheet, 5, 2), "40");
    assert!(o.book.info(helper).working);
    // SUM counts filtered rows (only SUBTOTAL leaves them out).
    assert_eq!(text(&o, helper, 1, 1), "120");
    assert_eq!(o.book.filters, s.book.filters);
    assert_eq!(o.book.definitions.len(), 1);
    // A new row still grows the totals.
    let mut o = o;
    o.call("add_row", &json!({ "table": "Sales", "values": { "Month": "Apr", "Revenue": "5" } })).unwrap();
    assert_eq!(input(&o, sheet, 6, 2), "=SUBTOTAL(109,Sales[Revenue])");
}

#[test]
fn csv_in_and_out() {
    let text = "Name;Amount;When\r\n\"Smith; J\";1.234,5;2026-10-24\n\"Say \"\"hi\"\"\";12;\n=1+1;3;x\n";
    assert_eq!(csv::separator(text), ';');
    let rows = csv::parse(text, ';');
    assert_eq!(rows[1][0], "Smith; J");
    assert_eq!(rows[2][0], "Say \"hi\"");
    let mut s = Session::open_file(text.as_bytes(), "Money.csv").unwrap();
    let id = s.book.id_at(0).unwrap();
    assert_eq!(s.book.sheet_name(id).unwrap(), "Money");
    assert_eq!(text_of(&s, id, 3, 2), "12");
    assert_eq!(input(&s, id, 4, 1), "'=1+1");
    assert_eq!(text_of(&s, id, 4, 1), "=1+1");
    let out = String::from_utf8(s.save_as(Format::Csv, Some(id)).unwrap()).unwrap();
    assert!(out.starts_with("Name,Amount,When\r\n"));
    assert!(out.contains("\"Say \"\"hi\"\"\""));
    assert_eq!(s.format, Format::Csv);
}

fn text_of(s: &Session, sheet: u32, row: i32, col: i32) -> String {
    text(s, sheet, row, col)
}

/// Every xlsx in IronCalc's test files: open, save, open again; the values
/// and formulas come back, and every part the saved file points to is in it.
#[test]
fn excel_corpus_round_trips() {
    let dirs: Vec<std::path::PathBuf> = std::fs::read_dir("/usr/local/cargo/registry/src")
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .filter(|e| e.file_name().to_string_lossy().starts_with("ironcalc-0.9"))
        .map(|e| e.path().join("tests"))
        .collect();
    let mut files = Vec::new();
    fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            let q = e.path();
            if q.is_dir() {
                walk(&q, out);
            } else if q.extension().is_some_and(|x| x == "xlsx") {
                out.push(q);
            }
        }
    }
    for d in &dirs {
        walk(d, &mut files);
    }
    if files.is_empty() {
        eprintln!("no corpus here: skipped");
        return;
    }
    let (mut opened, mut failed_open, mut problems) = (0, Vec::new(), Vec::new());
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let mut a = match Session::open_file(&bytes, &name) {
            Ok(s) => s,
            Err(e) => {
                failed_open.push(format!("{name}: {e}"));
                continue;
            }
        };
        opened += 1;
        let saved = match a.save_as(Format::Xlsx, None) {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("{name}: save: {e}"));
                continue;
            }
        };
        // Every internal relationship points at a part that's there.
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.clone())).unwrap();
        let names: Vec<String> = (0..z.len()).map(|k| z.by_index(k).unwrap().name().unwrap().to_string()).collect();
        // And what Calc keeps is still there.
        let orig_names: Vec<String> = {
            let mut oz = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
            (0..oz.len()).map(|k| oz.by_index(k).unwrap().name().unwrap().to_string()).collect()
        };
        for kind in
            ["xl/media/", "xl/pivotTables/", "xl/pivotCache/", "xl/comments", "xl/chartsheets/", "xl/vbaProject.bin"]
        {
            let before = orig_names.iter().filter(|n| n.starts_with(kind)).count();
            let after = names.iter().filter(|n| n.starts_with(kind)).count();
            if before != after {
                problems.push(format!("{name}: {kind} {before} parts → {after}"));
            }
        }
        // Charts: the same number (Excel's became Calc's and are written anew).
        let is_chart = |n: &&String| {
            n.starts_with("xl/charts/")
                && !n.contains("/_rels/")
                && (n.contains("/chart") || n.contains("/calcchart"))
                && n.ends_with(".xml")
        };
        let (before, after) = (orig_names.iter().filter(is_chart).count(), names.iter().filter(is_chart).count());
        if before != after {
            problems.push(format!("{name}: {before} charts → {after}"));
        }
        for n in names.iter().filter(|n| n.ends_with(".rels")) {
            let mut t = String::new();
            std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut t).unwrap();
            let base = n.replace("_rels/", "").trim_end_matches(".rels").to_string();
            for rel in t.split("<Relationship ").skip(1) {
                if rel.contains("TargetMode=\"External\"") {
                    continue;
                }
                let target = rel.split("Target=\"").nth(1).and_then(|x| x.split('"').next()).unwrap_or("");
                let full = if let Some(abs) = target.strip_prefix('/') {
                    abs.to_string()
                } else {
                    let mut parts: Vec<&str> = base.split('/').collect();
                    parts.pop();
                    for seg in target.split('/') {
                        match seg {
                            ".." => {
                                parts.pop();
                            }
                            "." | "" => {}
                            x => parts.push(x),
                        }
                    }
                    parts.join("/")
                };
                if !names.contains(&full) {
                    problems.push(format!("{name}: {n} points at missing {full}"));
                }
            }
        }
        let b = match Session::open_file(&saved, &name) {
            Ok(s) => s,
            Err(e) => {
                problems.push(format!("{name}: reopen: {e}"));
                continue;
            }
        };
        for (i, w) in a.book.model.workbook.worksheets.iter().enumerate() {
            // Cells a random array spills into change too.
            let random_spills = (0..1).any(|_| {
                w.sheet_data.cells().any(|(r, c, _)| {
                    a.book
                        .model
                        .get_cell_formula(i as u32, r, c)
                        .ok()
                        .flatten()
                        .is_some_and(|f| f.to_uppercase().contains("RANDARRAY"))
                })
            });
            for (r, c, _) in w.sheet_data.cells() {
                let (i, j) = (i as u32, b.book.index(w.sheet_id).unwrap_or(u32::MAX));
                let va = a.book.model.get_formatted_cell_value(i, r, c).unwrap_or_default();
                let vb = b.book.model.get_formatted_cell_value(j, r, c).unwrap_or_default();
                let fa = a.book.model.get_cell_formula(i, r, c).ok().flatten();
                let fb = b.book.model.get_cell_formula(j, r, c).ok().flatten();
                // Random and time functions are allowed to change.
                let volatile = (fa.is_none() && random_spills)
                    || fa.as_deref().is_some_and(|f| {
                        ["RAND", "NOW", "TODAY", "RANDBETWEEN", "RANDARRAY", "INFO"]
                            .iter()
                            .any(|v| f.to_uppercase().contains(v))
                    });
                if fa != fb || (va != vb && !volatile) {
                    problems.push(format!("{name}: {}!R{r}C{c}: {fa:?} {va:?} → {fb:?} {vb:?}", w.name));
                    break;
                }
            }
        }
        a.book.kept = None;
    }
    eprintln!("corpus: {} files, {opened} opened, {} could not be opened", files.len(), failed_open.len());
    for f in &failed_open {
        eprintln!("  open: {f}");
    }
    for p in problems.iter().take(40) {
        eprintln!("  {p}");
    }
    assert!(problems.is_empty(), "{} round-trip problems", problems.len());
}

/// Macros, choices for cells (data validation) and sparklines aren't
/// edited in Calc, but come back out as they went in.
#[test]
fn excel_keeps_macros_validation_and_sparklines() {
    use std::io::{Read, Write};
    let (mut s, _) = sales();
    let base = s.save_as(Format::Xlsx, None).unwrap();
    // Add them to the file, as Excel would have written them.
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(base)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for k in 0..src.len() {
        let mut f = src.by_index(k).unwrap();
        let name = f.name().unwrap().to_string();
        let mut t = Vec::new();
        f.read_to_end(&mut t).unwrap();
        let mut text = String::from_utf8_lossy(&t).into_owned();
        match name.as_str() {
            "xl/worksheets/sheet1.xml" => {
                text = text.replace("</sheetData>", "</sheetData><dataValidations count=\"1\"><dataValidation type=\"list\" sqref=\"C2:C4\"><formula1>\"Yes,No\"</formula1></dataValidation></dataValidations>");
                text = text.replace("</worksheet>", "<extLst><ext uri=\"{05C60535-1F16-4fd2-B633-F4F36F0B64E0}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:sparklineGroups><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">Sales!B2:B4</xm:f><xm:sqref xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">D2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst></worksheet>");
            }
            "xl/_rels/workbook.xml.rels" => {
                text = text.replace("</Relationships>", "<Relationship Id=\"rIdVba\" Type=\"http://schemas.microsoft.com/office/2006/relationships/vbaProject\" Target=\"vbaProject.bin\"/></Relationships>");
            }
            "[Content_Types].xml" => {
                text = text.replace(
                    "</Types>",
                    "<Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/></Types>",
                );
            }
            _ => {}
        }
        out.start_file(name.as_str(), opts).unwrap();
        out.write_all(text.as_bytes()).unwrap();
    }
    out.start_file("xl/vbaProject.bin", opts).unwrap();
    out.write_all(b"not really VBA").unwrap();
    let with = out.finish().unwrap().into_inner();

    let mut o = Session::open_file(&with, "Shop.xlsm").unwrap();
    assert_eq!(o.format, Format::Xlsm);
    let about = o.call("about_file", &json!({})).unwrap();
    assert_eq!(about["macros"], true);
    assert!(about["kept"].as_array().unwrap().iter().any(|k| k.as_str().unwrap().contains("Macros")));
    let again = o.save_as(Format::Xlsm, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(again)).unwrap();
    let mut read = |n: &str| {
        let mut t = String::new();
        z.by_name(n).unwrap().read_to_string(&mut t).unwrap();
        t
    };
    assert_eq!(read("xl/vbaProject.bin"), "not really VBA");
    let sheet = read("xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<dataValidations count=\"1\">"), "{sheet}");
    assert!(sheet.contains("sparklineGroups"), "{sheet}");
    // In the order Excel needs: validation after the sheet data, extensions last.
    assert!(sheet.find("</sheetData>").unwrap() < sheet.find("<dataValidations").unwrap());
    assert!(sheet.rfind("<extLst>").unwrap() > sheet.find("<dataValidations").unwrap());
    let types = read("[Content_Types].xml");
    assert!(types.contains("macroEnabled.main+xml"), "{types}");
    assert!(read("xl/_rels/workbook.xml.rels").contains("vbaProject.bin"));
    // Through a .xcalc save too.
    let x = o.save_as(Format::Xcalc, None).unwrap();
    let mut back = Session::open(&x).unwrap();
    let again = back.save_as(Format::Xlsm, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(again)).unwrap();
    assert!(z.by_name("xl/vbaProject.bin").is_ok());
}

#[test]
fn merging_cells() {
    let (mut s, sheet) = sales();
    let check = s.call("merge", &json!({ "sheet": sheet, "range": range(1, 1, 2, 2), "check": true })).unwrap();
    assert_eq!(check["lost"], 3);
    s.call("merge", &json!({ "sheet": sheet, "range": range(1, 1, 2, 2) })).unwrap();
    assert_eq!(text(&s, sheet, 1, 1), "Month");
    assert_eq!(text(&s, sheet, 2, 2), "");
    let l = s.call("layout", &json!({ "sheet": sheet })).unwrap();
    assert_eq!(l["merged"], json!([[1, 1, 2, 2]]));
    s.undo().unwrap();
    assert_eq!(text(&s, sheet, 2, 2), "10");
    assert!(s.book.model.workbook.worksheets[0].merged_cells.is_empty());
    // An empty top left takes the first value.
    s.call("merge", &json!({ "sheet": sheet, "range": range(6, 1, 6, 3) })).unwrap();
    s.type_in(sheet, 8, 2, "x").unwrap();
    s.call("merge", &json!({ "sheet": sheet, "range": range(8, 1, 8, 3) })).unwrap();
    assert_eq!(text(&s, sheet, 8, 1), "x");
    // Deleting a row across a merge and undoing brings it back as it was.
    s.call("delete_rows", &json!({ "sheet": sheet, "at": 8 })).unwrap();
    s.undo().unwrap();
    assert!(s.book.model.workbook.worksheets[0].merged_cells.iter().any(|m| m.row == 8 && m.width == 3));
    s.call("unmerge", &json!({ "sheet": sheet, "range": range(8, 2, 8, 2) })).unwrap();
    assert!(!s.book.model.workbook.worksheets[0].merged_cells.iter().any(|m| m.row == 8));
    let o = Session::open(&s.save().unwrap()).unwrap();
    assert_eq!(o.book.model.workbook.worksheets[0].merged_cells, s.book.model.workbook.worksheets[0].merged_cells);
}

#[test]
fn charts_go_to_excel_and_back() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    let c = s.call("add_chart", &json!({ "sheet": sheet, "range": range(1, 2, 1, 2), "row": 2, "col": 5 })).unwrap();
    let id = c["id"].as_u64().unwrap() as u32;
    let mut ch = s.book.charts[0].clone();
    ch.title = "Revenue by month".into();
    ch.kind = charts::ChartKind::Line;
    s.edit(&[Op::Chart { id, chart: Some(ch.clone()) }]).unwrap();
    // A block of cells (not a table) with a pie.
    let other = s.call("add_sheet", &json!({ "name": "Pie" })).unwrap().as_u64().unwrap() as u32;
    for (r, (a, b)) in [("Kind", "Count"), ("a", "3"), ("b", "5"), ("c", "2")].iter().enumerate() {
        s.type_in(other, r as i32 + 1, 1, a).unwrap();
        s.type_in(other, r as i32 + 1, 2, b).unwrap();
    }
    s.call("add_chart", &json!({ "sheet": other, "range": range(1, 1, 4, 2), "row": 1, "col": 4 })).unwrap();
    let pie_id = s.book.charts[1].id;
    let mut pie = s.book.charts[1].clone();
    pie.kind = charts::ChartKind::Pie;
    s.edit(&[Op::Chart { id: pie_id, chart: Some(pie) }]).unwrap();
    // Excel sees real charts, and no Calc extras are needed to get them back.
    let bytes = s.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut read = |n: &str| {
        let mut t = String::new();
        std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut t).unwrap();
        t
    };
    let chart = read("xl/charts/calcchart1.xml");
    assert!(chart.contains("<c:lineChart>"), "{chart}");
    assert!(chart.contains("<c:f>'Sales'!$B$2:$B$4</c:f>"), "{chart}");
    assert!(chart.contains("<c:f>'Sales'!$A$2:$A$4</c:f>"), "{chart}");
    assert!(chart.contains("Revenue by month"));
    assert!(read("xl/worksheets/sheet1.xml").contains("<drawing r:id="));
    assert!(
        read("xl/drawings/calcdrawing1.xml").contains("calcchart")
            || read("xl/drawings/_rels/calcdrawing1.xml.rels").contains("calcchart1.xml")
    );
    assert!(read("xl/charts/calcchart2.xml").contains("<c:doughnutChart>"));
    // Without Calc's own part, the charts come back from Excel's.
    let mut stripped = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    for k in 0..src.len() {
        let mut f = src.by_index(k).unwrap();
        let name = f.name().unwrap().to_string();
        if name == "xenify/calc.json" {
            continue;
        }
        let mut b = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut b).unwrap();
        stripped.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut stripped, &b).unwrap();
    }
    let excel_only = stripped.finish().unwrap().into_inner();
    let mut o = Session::open_file(&excel_only, "Charts.xlsx").unwrap();
    assert_eq!(o.book.charts.len(), 2);
    let back = &o.book.charts[0];
    assert_eq!(back.kind, charts::ChartKind::Line);
    assert_eq!(back.title, "Revenue by month");
    assert_eq!(back.source.table.as_deref(), Some("Sales"));
    assert_eq!(back.source.series, vec!["Revenue".to_string()]);
    assert_eq!((back.row, back.col), (2, 5));
    let pie_back = o.book.charts.iter().find(|c| c.sheet == other).unwrap();
    assert_eq!(pie_back.kind, charts::ChartKind::Pie);
    assert_eq!(pie_back.source.range, Some(edit::Range { top: 1, left: 1, bottom: 4, right: 2 }));
    // And saved again: still two charts (the Excel ones were replaced, not doubled).
    let again = o.save_as(Format::Xlsx, None).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(again)).unwrap();
    let charts: Vec<String> = z
        .file_names()
        .flatten()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with("xl/charts/") && n.ends_with(".xml") && !n.contains("_rels"))
        .collect();
    assert_eq!(charts.len(), 2, "{charts:?}");
}

#[test]
fn excel_pictures_are_shown_and_kept() {
    use std::io::{Read, Write};
    let (mut s, _) = sales();
    let base = s.save_as(Format::Xlsx, None).unwrap();
    let png: &[u8] = b"\x89PNG\r\n\x1a\nnot a real picture";
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(base)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for k in 0..src.len() {
        let mut f = src.by_index(k).unwrap();
        let name = f.name().unwrap().to_string();
        let mut t = Vec::new();
        f.read_to_end(&mut t).unwrap();
        let mut text = String::from_utf8_lossy(&t).into_owned();
        if name == "xl/worksheets/sheet1.xml" {
            text = text.replace("</worksheet>", "<drawing r:id=\"rIdD\"/></worksheet>");
        }
        if name == "[Content_Types].xml" {
            text = text.replace("</Types>", "<Default Extension=\"png\" ContentType=\"image/png\"/><Override PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/></Types>");
        }
        out.start_file(name.as_str(), opts).unwrap();
        out.write_all(text.as_bytes()).unwrap();
    }
    out.start_file("xl/worksheets/_rels/sheet1.xml.rels", opts).unwrap();
    out.write_all(br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdD" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#).unwrap();
    out.start_file("xl/drawings/drawing1.xml", opts).unwrap();
    out.write_all(br#"<?xml version="1.0"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:oneCellAnchor><xdr:from><xdr:col>3</xdr:col><xdr:colOff>95250</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="1905000" cy="952500"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Logo"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rIdP"/></xdr:blipFill><xdr:spPr/></xdr:pic><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>"#).unwrap();
    out.start_file("xl/drawings/_rels/drawing1.xml.rels", opts).unwrap();
    out.write_all(br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdP" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#).unwrap();
    out.start_file("xl/media/image1.png", opts).unwrap();
    out.write_all(png).unwrap();
    let with = out.finish().unwrap().into_inner();

    let mut o = Session::open_file(&with, "Logo.xlsx").unwrap();
    let id = o.book.id_at(0).unwrap();
    let pics = o.call("pictures", &json!({ "sheet": id })).unwrap();
    assert_eq!(pics.as_array().unwrap().len(), 1);
    assert_eq!((pics[0]["row"].as_i64(), pics[0]["col"].as_i64()), (Some(2), Some(4)));
    assert_eq!(
        (pics[0]["dx"].as_f64(), pics[0]["w"].as_f64(), pics[0]["h"].as_f64()),
        (Some(10.0), Some(200.0), Some(100.0))
    );
    assert_eq!(pics[0]["mime"], "image/png");
    // A chart added in Calc goes into the same drawing layer, beside the picture.
    o.call("add_chart", &json!({ "sheet": id, "range": range(1, 1, 4, 2), "row": 6, "col": 4 })).unwrap();
    let again = o.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(again)).unwrap();
    let mut d = String::new();
    z.by_name("xl/drawings/drawing1.xml").unwrap().read_to_string(&mut d).unwrap();
    assert!(d.contains("r:embed=\"rIdP\""), "{d}");
    assert!(d.contains("<c:chart"), "{d}");
    assert_eq!(d.matches("<xdr:wsDr").count(), 1);
    assert!(z.by_name("xl/media/image1.png").is_ok());
    let mut sh = String::new();
    z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sh).unwrap();
    assert_eq!(sh.matches("<drawing ").count(), 1, "{sh}");
}

#[test]
fn opendocument_files_open() {
    use std::io::Write;
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2" office:version="1.2">
<office:body><office:spreadsheet>
<table:table table:name="Money">
<table:table-row><table:table-cell office:value-type="string"><text:p>Item</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>Cost</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Tea</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="2.5"><text:p>2.5</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Cake</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="4"><text:p>4</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Total</text:p></table:table-cell><table:table-cell table:formula="of:=SUM([.B2:.B3])" office:value-type="float" office:value="6.5"><text:p>6.5</text:p></table:table-cell></table:table-row>
</table:table>
<table:table table:name="Notes"><table:table-row><table:table-cell office:value-type="string"><text:p>hello</text:p></table:table-cell></table:table-row></table:table>
</office:spreadsheet></office:body></office:document-content>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/vnd.oasis.opendocument.spreadsheet").unwrap();
    z.start_file("content.xml", stored).unwrap();
    z.write_all(content.as_bytes()).unwrap();
    z.start_file("META-INF/manifest.xml", stored).unwrap();
    z.write_all(br#"<?xml version="1.0"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#).unwrap();
    let bytes = z.finish().unwrap().into_inner();
    let mut s = Session::open_file(&bytes, "Money.ods").unwrap();
    let sheets = s.call("sheets", &json!({})).unwrap();
    assert_eq!(sheets[0]["name"], "Money");
    assert_eq!(sheets[1]["name"], "Notes");
    let id = sheets[0]["id"].as_u64().unwrap() as u32;
    assert_eq!(text(&s, id, 3, 2), "4");
    assert_eq!(input(&s, id, 4, 2), "=SUM(B2:B3)");
    assert_eq!(text(&s, id, 4, 2), "6.5");
    // Saved as an Excel file (Calc doesn't write .ods); it isn't saved yet.
    assert!(s.unsaved());
    assert_eq!(s.format, Format::Xlsx);
    assert!(s.save_as(Format::Ods, None).is_err());
}

#[test]
fn opendocument_formulas_read_as_excel_writes_them() {
    assert_eq!(oldfiles::odf_formula("of:=SUM([.B2:.B3])"), "SUM(B2:B3)");
    assert_eq!(oldfiles::odf_formula("of:=IF([.A1]>0;\"a;b\";[$Sheet2.$C$4])"), "IF(A1>0,\"a;b\",Sheet2!$C$4)");
    assert_eq!(oldfiles::odf_formula("of:=SUM(['My sheet'.A1:.A9])"), "SUM('My sheet'!A1:A9)");
}

#[test]
fn sparklines_in_cells() {
    let (mut s, sheet) = sales();
    // Monthly numbers across, one sparkline for the row.
    for (k, v) in ["4", "6", "3", "8"].iter().enumerate() {
        s.type_in(sheet, 7, k as i32 + 1, v).unwrap();
    }
    assert_eq!(
        s.call("add_sparklines", &json!({ "sheet": sheet, "range": range(7, 1, 7, 4), "kind": "column" })).unwrap(),
        1
    );
    let v = s.call("view", &json!({ "sheet": sheet, "top": 7, "left": 1, "bottom": 7, "right": 6 })).unwrap();
    let cell = v["cells"].as_array().unwrap().iter().find(|c| c["c"] == 5).unwrap().clone();
    assert_eq!(cell["spark"]["values"], json!([4.0, 6.0, 3.0, 8.0]));
    // Into Excel's extension and back.
    let bytes = s.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut sheet_xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("xl/worksheets/sheet1.xml").unwrap(), &mut sheet_xml).unwrap();
    assert!(sheet_xml.contains("<xm:sqref>E7</xm:sqref>"), "{sheet_xml}");
    assert!(sheet_xml.contains("type=\"column\""));
    let mut o = Session::open_file(&bytes, "Spark.xlsx").unwrap();
    assert_eq!(o.book.sparklines, s.book.sparklines);
    // Saved again from the opened file: still one, not two.
    let again = o.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(again)).unwrap();
    let mut x = String::new();
    std::io::Read::read_to_string(&mut z.by_name("xl/worksheets/sheet1.xml").unwrap(), &mut x).unwrap();
    assert_eq!(x.matches("<x14:sparkline>").count(), 1);
    // A row inserted above moves it; undo puts it back.
    s.call("insert_rows", &json!({ "sheet": sheet, "at": 1 })).unwrap();
    assert_eq!(s.book.sparklines[0].row, 8);
    assert_eq!(s.book.sparklines[0].source.top, 8);
    s.undo().unwrap();
    assert_eq!(s.book.sparklines[0].row, 7);
}

#[test]
fn summaries_follow_their_table() {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    s.call("rename_sheet", &json!({ "sheet": sheet, "name": "Orders" })).unwrap();
    let rows = [
        ["Order", "City", "Total", "Items"],
        ["A1", "York", "184.5", "3"],
        ["A2", "Skipton", "1240", "12"],
        ["A3", "Leeds", "96", "1"],
        ["A4", "Leeds", "205.4", "4"],
        ["A5", "York", "20", "1"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            s.type_in(sheet, r as i32 + 1, c as i32 + 1, v).unwrap();
        }
    }
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("format", &json!({ "sheet": sheet, "range": range(2, 3, 6, 3), "set": { "num_fmt": "#,##0.00" } })).unwrap();
    let from = s.call("summary_from", &json!({ "sheet": sheet, "row": 2, "col": 2 })).unwrap();
    assert_eq!(from["from"]["table"], "Orders");
    assert_eq!(from["columns"], json!(["Order", "City", "Total", "Items"]));
    let def = json!({
        "from": from["from"], "group": ["City"],
        "show": [{ "how": "rows" }, { "column": "Total", "how": "sum" }, { "column": "Items", "how": "average" }],
        "order": 1, "desc": true, "totals": true,
    });
    let p = s.call("summary_preview", &json!({ "def": def })).unwrap();
    assert_eq!(p["rows"][0], json!(["City", "Rows", "Total", "Average items"]));
    assert_eq!(p["rows"][1], json!(["Skipton", "1", "1,240.00", "12.00"]));
    assert_eq!(p["rows"][2][0], "Leeds");
    assert_eq!(p["rows"][2][2], "301.40");
    assert_eq!(p["rows"][4], json!(["3 groups", "5", "1,745.90", "4.20"]));
    let id = s.call("add_summary", &json!({ "def": def, "name": "Orders by city" })).unwrap().as_u64().unwrap() as u32;
    assert_eq!(s.book.info(id).kind, SheetKind::Summary);
    assert_eq!(text(&s, id, 2, 1), "Skipton");
    assert!(s.type_in(id, 2, 1, "x").is_err());
    // A new order: the summary follows.
    s.call(
        "add_row",
        &json!({ "table": "Orders", "values": { "Order": "A6", "City": "York", "Total": "2000", "Items": "9" } }),
    )
    .unwrap();
    assert_eq!(text(&s, id, 2, 1), "York");
    assert_eq!(text(&s, id, 2, 3), "2,204.50");
    // Filtered rows are left out, as in the sheet.
    s.call("filter", &json!({ "table": "Orders", "column": "City", "keep": ["Leeds"] })).unwrap();
    assert_eq!(text(&s, id, 2, 1), "Leeds");
    assert_eq!(text(&s, id, 3, 1), "1 group");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(text(&s, id, 2, 1), "Skipton");
    // Only rows where…
    let mut d: summary::Def = serde_json::from_value(def.clone()).unwrap();
    d.only = vec![summary::Condition { column: "Total".into(), test: summary::Test::More, value: "100".into() }];
    s.edit(&[Op::Summary { sheet: id, def: Some(d) }]).unwrap();
    assert_eq!(text(&s, id, 5, 2), "3");
    // Kept in both kinds of file.
    let o = Session::open(&s.save().unwrap()).unwrap();
    assert_eq!(o.book.summaries, s.book.summaries);
    let x = Session::open_file(&s.save_as(Format::Xlsx, None).unwrap(), "Orders.xlsx").unwrap();
    assert_eq!(x.book.summaries, s.book.summaries);
    assert_eq!(x.book.info(id).kind, SheetKind::Summary);
    // Taking the sheet away (and back).
    s.call("remove_sheet", &json!({ "sheet": id })).unwrap();
    assert!(s.book.summaries.is_empty());
    s.undo().unwrap();
    assert_eq!(text(&s, id, 2, 1), "Skipton");
}

#[test]
fn years_from_day_numbers() {
    // 2026-10-25 is day 46320; 1900-03-01 is day 61.
    assert_eq!(summary_year(46320.0), 2026);
    assert_eq!(summary_year(61.0), 1900);
}

fn summary_year(n: f64) -> i32 {
    // Through a summary condition: a date is in the year or not.
    let mut s = Session::new();
    let id = s.book.id_at(0).unwrap();
    s.type_in(id, 1, 1, "When").unwrap();
    s.book.put(id, 2, 1, Some(&CellData::Number(ironcalc_base::num::Num::from(n)))).unwrap();
    for y in 1899..2100 {
        let def = json!({ "from": { "sheet": id, "range": range(1, 1, 2, 1) }, "group": [], "show": [{ "how": "rows" }],
            "only": [{ "column": "When", "test": "year", "value": y.to_string() }], "totals": false });
        let p = s.call("summary_preview", &json!({ "def": def })).unwrap();
        if p["rows"].as_array().unwrap().len() > 1 {
            return y;
        }
    }
    0
}

#[test]
fn reports_follow_the_book() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    let id = s.call("add_report", &json!({ "name": "October report" })).unwrap().as_u64().unwrap() as u32;
    assert_eq!(s.book.info(id).kind, SheetKind::Report);
    let def = json!({ "blocks": [
        { "kind": "title", "text": "October report" },
        { "kind": "number", "label": "Revenue", "formula": "SUM(Sales[Revenue])" },
        { "kind": "text", "text": "Busy month." },
        { "kind": "cells", "sheet": sheet, "table": "Sales" },
    ] });
    s.call("set_report", &json!({ "sheet": id, "def": def })).unwrap();
    let p = s.call("report", &json!({ "sheet": id })).unwrap();
    assert_eq!(p["blocks"][1]["value"], "60");
    assert_eq!(p["blocks"][3]["rows"][1], json!(["Jan", "10"]));
    // The numbers follow the table.
    s.call("add_row", &json!({ "table": "Sales", "values": { "Month": "Apr", "Revenue": "40" } })).unwrap();
    let p = s.call("report", &json!({ "sheet": id })).unwrap();
    assert_eq!(p["blocks"][1]["value"], "100");
    assert_eq!(p["blocks"][3]["rows"].as_array().unwrap().len(), 5);
    let o = Session::open(&s.save().unwrap()).unwrap();
    assert_eq!(o.book.reports, s.book.reports);
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.book.reports[&id].blocks.len(), 1);
}

#[test]
fn a_report_from_a_table_is_ready_to_read() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    for (m, r) in [("Apr", "1200"), ("May", "26000")] {
        s.call("add_row", &json!({ "table": "Sales", "values": { "Month": m, "Revenue": r } })).unwrap();
    }
    let id = s.call("add_report", &json!({ "name": "Sales report", "table": "Sales" })).unwrap().as_u64().unwrap() as u32;
    let p = s.call("report", &json!({ "sheet": id })).unwrap();
    let kinds: Vec<&str> = p["blocks"].as_array().unwrap().iter().map(|b| b["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["title", "number", "cells"]);
    assert_eq!(p["blocks"][1]["label"], "Total revenue");
    assert_eq!(p["blocks"][1]["value"], "27,260", "big numbers have thousands separators");
    assert_eq!(report::grouped("-1234567.5"), "-1,234,567.5");
    assert_eq!(report::grouped("£1,800"), "£1,800");
    assert_eq!(report::grouped("999"), "999");
}

#[test]
fn query_sheets_fill_refresh_and_keep() {
    let (mut s, sales_sheet) = sales();
    s.type_in(sales_sheet, 6, 1, "2026").unwrap();
    let def = json!({ "server": "http://office:7700", "source": 3, "source_name": "Shop", "query": 9, "query_name": "Orders",
        "params": { "year": { "cell": "Sales!A6" }, "region": { "value": "North" } } });
    let answer = json!({ "columns": [["Item", "text"], ["Code", "text"], ["Total", "number"], ["Day", "date"]],
        "rows": [["Tea", "007", 12.5, "2026-01-04"], ["Jam", null, 3, null], ["Coffee", "012", 4, "2026-02-01"]], "truncated": false,
        "notes": { "Total": "Before tax" } });
    let q = s.call("add_query", &json!({ "def": def, "answer": answer })).unwrap().as_u64().unwrap() as u32;
    assert_eq!(s.book.sheet_name(q).unwrap(), "Orders");
    assert_eq!(s.book.info(q).kind, SheetKind::Query);
    assert_eq!(text(&s, q, 1, 3), "Total");
    assert_eq!(text(&s, q, 2, 2), "007", "codes stay words");
    assert_eq!(input(&s, q, 3, 3), "3");
    let t = tables::get(&s.book, "Orders").unwrap();
    assert_eq!((t.top, t.bottom, t.right), (1, 4, 4));
    assert!(s.type_in(q, 2, 1, "x").is_err(), "a query sheet can't be typed in");

    // The values for its :names, from a cell and as typed.
    let got = s.call("query", &json!({ "sheet": q })).unwrap();
    assert_eq!(got["params"], json!({ "year": 2026.0, "region": "North" }));
    assert_eq!(got["def"]["rows"], 3);

    // Formulas elsewhere follow the table.
    s.type_in(sales_sheet, 8, 1, "=SUM(Orders[Total])").unwrap();
    assert_eq!(text(&s, sales_sheet, 8, 1), "19.5");

    // A refresh with fewer rows and a column fewer: the rest goes.
    let fewer = json!({ "columns": [["Item", "text"], ["Code", "text"], ["Total", "number"]], "rows": [["Tea", "007", 100]], "truncated": true });
    s.call("fill_query", &json!({ "sheet": q, "answer": fewer, "refreshed": "2026-10-25T12:00:00Z" })).unwrap();
    assert_eq!(text(&s, sales_sheet, 8, 1), "100");
    assert_eq!(input(&s, q, 3, 1), "");
    assert_eq!(input(&s, q, 2, 4), "");
    let t = tables::get(&s.book, "Orders").unwrap();
    assert_eq!((t.bottom, t.right), (2, 3));
    let d = &s.book.queries[&q];
    assert!(d.truncated);
    assert_eq!(d.refreshed, "2026-10-25T12:00:00Z");
    assert_eq!(d.notes, json!({ "Total": "Before tax" }), "notes kept when a refresh brings none");

    // Renaming its table: formulas and the query follow.
    s.call("rename_table", &json!({ "name": "Orders", "to": "Sales2026" })).unwrap();
    assert_eq!(input(&s, sales_sheet, 8, 1), "=SUM(Sales2026[Total])");
    assert_eq!(s.book.queries[&q].table, "Sales2026");
    assert!(s.call("rename_table", &json!({ "name": "Sales2026", "to": "Sales 2026" })).is_err(), "no spaces");
    s.undo().unwrap();
    assert_eq!(s.book.queries[&q].table, "Orders");

    // Undo puts the last answer back; undo again takes the sheet away.
    s.undo().unwrap();
    assert_eq!(text(&s, sales_sheet, 8, 1), "19.5");
    assert_eq!(tables::get(&s.book, "Orders").unwrap().bottom, 4);

    // Kept in the file (and in Excel files).
    for format in [Format::Xcalc, Format::Xlsx] {
        let bytes = s.save_as(format, None).unwrap();
        let name = if format == Format::Xlsx { "b.xlsx" } else { "b.xcalc" };
        let back = Session::open_file(&bytes, name).unwrap();
        assert_eq!(back.book.info(q).kind, SheetKind::Query, "{name}");
        assert_eq!(back.book.queries[&q].query, Some(9), "{name}");
        assert_eq!(text(&back, q, 4, 1), "Coffee", "{name}");
    }

    s.undo().unwrap();
    s.undo().unwrap();
    assert!(s.book.index(q).is_err());
    assert!(s.book.queries.is_empty());
    assert!(tables::get(&s.book, "Orders").is_none());
}

#[test]
fn database_sheets_fit_their_columns_and_show_decimals() {
    let mut s = Session::new();
    let def = json!({ "server": "http://office:7700", "source": 3, "source_name": "Shop", "table": "shops", "key": ["id"], "add": true, "delete": true });
    let answer = json!({ "columns": [["id", "number"], ["name", "text"], ["price", "number"]],
        "rows": [[1, "Corner Bakery and Coffee House", 21.5], [2, "Mill", 24]], "truncated": false, "decimals": [null, null, 2] });
    let e = s.call("add_editor", &json!({ "def": def, "answer": answer })).unwrap().as_u64().unwrap() as u32;
    let i = s.book.index(e).unwrap();
    assert!(s.book.model.get_column_width(i, 2).unwrap() > 200.0, "names fit");
    assert_eq!(s.book.model.get_column_width(i, 1).unwrap(), 90.0, "short columns stay as they are");
    assert_eq!(text(&s, e, 2, 3), "21.50");
    assert_eq!(text(&s, e, 3, 3), "24.00");
    assert_eq!(text(&s, e, 2, 1), "1", "only columns the database gives decimals for");
    let st = s.call("editor", &json!({ "sheet": e })).unwrap();
    assert_eq!(st["updates"].as_u64(), Some(0), "formats change nothing to send: {st}");
    // A width set since is kept by a refresh.
    s.call("fill_editor", &json!({ "sheet": e, "answer": answer })).unwrap();
    assert!(s.book.model.get_column_width(i, 2).unwrap() > 200.0);
    s.edit(&[Op::Columns {
        sheet: e,
        columns: vec![crate::ops::LineSize { at: 2, size: Some(120.0), hidden: false }],
    }])
    .unwrap();
    s.call("fill_editor", &json!({ "sheet": e, "answer": answer })).unwrap();
    assert_eq!(s.book.model.get_column_width(i, 2).unwrap(), 120.0);

    // Query sheets too.
    let qdef = json!({ "server": "http://office:7700", "source": 3, "source_name": "Shop", "sql": "SELECT name, price FROM shops", "query_name": "Shops" });
    let qa = json!({ "columns": [["name", "text"], ["price", "number"]], "rows": [["Corner Bakery and Coffee House", 21.5]], "truncated": false, "decimals": [null, 2] });
    let q = s.call("add_query", &json!({ "def": qdef, "answer": qa })).unwrap().as_u64().unwrap() as u32;
    let i = s.book.index(q).unwrap();
    assert!(s.book.model.get_column_width(i, 1).unwrap() > 200.0);
    assert_eq!(text(&s, q, 2, 2), "21.50");
}

#[test]
fn table_editors_work_out_what_to_send() {
    let mut s = Session::new();
    let def = json!({ "server": "http://office:7700", "source": 3, "source_name": "Shop", "table": "products", "key": ["id"], "locked": ["added"],
        "add": true, "delete": false });
    let answer = json!({ "columns": [["id", "number"], ["name", "text"], ["price", "number"], ["added", "date"]],
        "rows": [[1, "Tea", 3.5, "2026-01-04"], [2, "Coffee", 4.25, "2026-02-10"], [3, "Jam", 2.8, null]], "truncated": false, "notes": { "total": 3 } });
    let e = s.call("add_editor", &json!({ "def": def, "answer": answer })).unwrap().as_u64().unwrap() as u32;
    assert_eq!(s.book.info(e).kind, SheetKind::TableEditor);
    assert_eq!(s.book.sheet_name(e).unwrap(), "products");
    let st = s.call("editor", &json!({ "sheet": e })).unwrap();
    assert_eq!(
        (st["updates"].as_u64(), st["inserts"].as_u64(), st["deletes"].as_u64()),
        (Some(0), Some(0), Some(0)),
        "dates read back unchanged: {st}"
    );

    // What people may change.
    assert!(s.type_in(e, 1, 2, "Title").is_err(), "the header comes from the database");
    assert!(s.type_in(e, 2, 1, "9").is_err(), "the key");
    assert!(s.type_in(e, 2, 4, "2026-01-01").is_err(), "a locked column");
    assert!(s.call("delete_rows", &json!({ "sheet": e, "at": 3 })).is_err(), "no deleting here");
    assert!(s.call("insert_columns", &json!({ "sheet": e, "at": 2 })).is_err());

    s.type_in(e, 2, 3, "3.75").unwrap();
    s.type_in(e, 5, 2, "Cocoa").unwrap(); // just under the table: a new row
    s.type_in(e, 5, 3, "5").unwrap();
    let st = s.call("editor", &json!({ "sheet": e })).unwrap();
    assert_eq!(st["updates"], 1);
    assert_eq!(st["inserts"], 1);
    let all = s.call("editors", &json!({})).unwrap();
    let changes = all[0]["changes"].clone();
    assert_eq!(
        changes[0],
        json!({ "table": "products", "key": { "id": 1 }, "column": "price", "old": 3.5, "new": 3.75, "row": 2 })
    );
    assert_eq!(
        changes[1],
        json!({ "kind": "insert", "table": "products", "values": { "name": "Cocoa", "price": 5.0 }, "row": 5 })
    );

    // Sorting moves whole rows: nothing more to send.
    s.call("sort", &json!({ "sheet": e, "range": { "top": 1, "left": 1, "bottom": 5, "right": 4 }, "by": [{ "col": 3, "desc": true }], "header": true })).unwrap();
    assert_eq!(s.call("editor", &json!({ "sheet": e })).unwrap()["updates"], 1);
    s.undo().unwrap();

    // Saved: the price went in; the new row got its key.
    let outcomes = json!([{ "outcome": "changed" }, { "outcome": "added", "key": { "id": "4" } }]);
    s.call("editor_sent", &json!({ "sheet": e, "changes": changes, "outcomes": outcomes })).unwrap();
    assert_eq!(text(&s, e, 5, 1), "4");
    let st = s.call("editor", &json!({ "sheet": e })).unwrap();
    assert_eq!((st["updates"].as_u64(), st["inserts"].as_u64()), (Some(0), Some(0)), "{st}");
    assert!(st["marks"].as_array().unwrap().iter().any(|m| m["kind"] == "sent" && m["row"] == 2 && m["col"] == 3));
    assert!(s.type_in(e, 5, 1, "7").is_err(), "a sent row's key is the database's");

    // Someone else changed Coffee's price first: it waits for a choice.
    s.type_in(e, 3, 3, "4.5").unwrap();
    let ch = s.call("editors", &json!({})).unwrap()[0]["changes"].clone();
    s.call("editor_sent", &json!({ "sheet": e, "changes": ch, "outcomes": [{ "outcome": "conflict", "now": "4.4" }] }))
        .unwrap();
    let st = s.call("editor", &json!({ "sheet": e })).unwrap();
    assert_eq!(st["waiting"], 1);
    assert_eq!(st["updates"], 0, "a cell waiting for a choice isn't sent again");
    assert_eq!(st["choices"][0]["mine"], 4.5);
    // Mine: the next Save sends it, checked against what's there now.
    s.call("editor_resolve", &json!({ "sheet": e, "key": { "id": 2.0 }, "column": "price", "choice": "mine" }))
        .unwrap();
    let ch = s.call("editors", &json!({})).unwrap()[0]["changes"].clone();
    assert_eq!(ch[0]["old"], 4.4);
    assert_eq!(ch[0]["new"], 4.5);
    // Theirs (after another refusal): the sheet takes what's there.
    s.call("editor_sent", &json!({ "sheet": e, "changes": ch, "outcomes": [{ "outcome": "conflict", "now": "4.6" }] }))
        .unwrap();
    s.call("editor_resolve", &json!({ "sheet": e, "key": { "id": 2.0 }, "column": "price", "choice": "theirs" }))
        .unwrap();
    assert_eq!(text(&s, e, 3, 3), "4.6");
    assert_eq!(s.call("editor", &json!({ "sheet": e })).unwrap()["updates"], 0);

    // Kept in the file with what's not sent yet.
    s.type_in(e, 4, 2, "Marmalade").unwrap();
    let back = Session::open(&s.save().unwrap()).unwrap();
    let st = editor::state(&back.book, e);
    assert_eq!(st["updates"], 1);
    assert_eq!(st["def"]["key"], json!(["id"]));

    // Refreshing would drop unsent changes: it asks first.
    let fresh = json!({ "columns": [["id", "number"], ["name", "text"], ["price", "number"], ["added", "date"]], "rows": [[1, "Tea", 3.75, "2026-01-04"]] });
    assert_eq!(s.call("fill_editor", &json!({ "sheet": e, "answer": fresh })).unwrap_err(), "unsent");
    s.call("fill_editor", &json!({ "sheet": e, "answer": fresh, "force": true })).unwrap();
    assert_eq!(input(&s, e, 3, 2), "");
    assert_eq!(s.call("editor", &json!({ "sheet": e })).unwrap()["deletes"], 0);
}

#[test]
fn ask_sees_the_book_through_tools() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 1, 3, "Costs").unwrap();
    for (r, v) in [(2, "6"), (3, "11"), (4, "17")] {
        s.type_in(sheet, r, 3, v).unwrap();
    }
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("set_note", &json!({ "target": { "on": "table", "table": "Sales", "column": "Revenue" }, "note": { "target": { "on": "table", "table": "Sales", "column": "Revenue" }, "meaning": "Money in from orders", "unit": "GBP", "with_tax": false } })).unwrap();
    s.call("set_definition", &json!({ "name": "Profit", "definition": { "name": "Profit", "formula": "=SUM(Sales[Revenue])-SUM(Sales[Costs])", "about": "Before tax" } })).unwrap();

    let d = s.call("ai_describe", &json!({})).unwrap();
    assert_eq!(d["tables"][0]["name"], "Sales");
    assert_eq!(d["tables"][0]["rows"], 3);
    assert_eq!(d["tables"][0]["noted"], json!(["Revenue"]));
    assert_eq!(d["definitions"][0]["name"], "Profit");

    let c = s.call("ai_columns", &json!({ "table": "Sales" })).unwrap();
    let rev = &c["columns"][1];
    assert_eq!(
        (rev["name"].clone(), rev["holds"].clone(), rev["sum"].clone()),
        (json!("Revenue"), json!("numbers"), json!(60.0))
    );
    assert_eq!(rev["note"]["means"], "Money in from orders");
    assert_eq!(rev["note"]["includes_tax"], false);
    assert_eq!(c["columns"][0]["holds"], "words");

    let r = s.call("ai_rows", &json!({ "table": "Sales", "count": 2 })).unwrap();
    assert_eq!(r["rows"], json!([["Jan", "10", "6"], ["Feb", "20", "11"]]));
    assert_eq!(r["rows_in_all"], 3);

    // Worked out without changing anything (not even undo).
    let before = s.unsaved();
    let undo_before = s.call("state", &json!({})).ok();
    let v = s.call("ai_calc", &json!({ "formula": "SUM(Sales[Revenue])-SUM(Sales[Costs])" })).unwrap();
    assert_eq!(v["value"], 26.0);
    let e = s.call("ai_calc", &json!({ "formula": "=NOSUCH(1)" })).unwrap();
    assert!(e["error"].as_str().unwrap().starts_with('#'), "{e}");
    assert_eq!(s.unsaved(), before);
    assert_eq!(s.call("state", &json!({})).ok(), undo_before);
    let i = s.book.index(sheet).unwrap();
    assert!(s.book.model.workbook.worksheets[i as usize].cell(1_048_576, 16_384).is_none());
}

/// Ask's calculate says when a formula gives many values (an array) rather
/// than #SPILL!, and shows a number the way the money columns it reads do.
#[test]
fn ask_calculate_arrays_and_formats() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 1, 3, "Costs").unwrap();
    for (r, v) in [(2, "6.1"), (3, "11.25"), (4, "17")] {
        s.type_in(sheet, r, 3, v).unwrap();
    }
    s.type_in(sheet, 2, 2, "10.2").unwrap();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("format", &json!({ "sheet": sheet, "range": range(2, 2, 4, 3), "set": { "num_fmt": "0.00" } })).unwrap();
    let before = s.unsaved();

    // An array: its size and first values, no error.
    let v = s.call("ai_calc", &json!({ "formula": "=SORTBY(Sales[Month],Sales[Revenue],-1)" })).unwrap();
    assert_eq!(v["array"]["values"], 3, "{v}");
    assert_eq!(v["array"]["rows"], 3);
    assert_eq!(v["array"]["first"], json!(["Mar", "Feb", "Jan"]));
    assert!(v["error"].is_null(), "{v}");
    assert!(v["note"].as_str().unwrap().contains("array of 3 values"), "{v}");
    let v = s.call("ai_calc", &json!({ "formula": "=INDEX(SORTBY(Sales[Month],Sales[Revenue],-1),1)" })).unwrap();
    assert_eq!(v["shows"], "Mar", "{v}");
    assert!(v["array"].is_null());

    // Money columns formatted 0.00: their sums, differences and maxima too.
    let v = s.call("ai_calc", &json!({ "formula": "=SUM(Sales[Revenue])-SUM(Sales[Costs])" })).unwrap();
    assert_eq!(v["shows"], "25.85", "{v}");
    let v = s.call("ai_calc", &json!({ "formula": "=MAX(Sales[Revenue])" })).unwrap();
    assert_eq!((v["shows"].as_str(), v["format"].as_str()), (Some("30.00"), Some("0.00")), "{v}");
    let v = s.call("ai_calc", &json!({ "formula": "=MAX(B2:B4)-MIN(C2:C4)" })).unwrap();
    assert_eq!(v["shows"], "23.90", "{v}");
    // A count stays a plain number.
    let v = s.call("ai_calc", &json!({ "formula": "=COUNT(Sales[Revenue])" })).unwrap();
    assert_eq!(v["shows"], "3", "{v}");
    assert!(v["format"].is_null(), "{v}");
    // Errors stay errors.
    let v = s.call("ai_calc", &json!({ "formula": "=SUM(Sales[Revenue])/0" })).unwrap();
    assert_eq!(v["error"], "#DIV/0!", "{v}");
    assert_eq!(s.unsaved(), before);
}

/// Apple 1, Banana 2, Pear 3 in A1:B3: no header row.
fn fruit() -> (Session, u32) {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    for (r, (a, b)) in [("Apple", "1"), ("Banana", "2"), ("Pear", "3")].iter().enumerate() {
        s.type_in(sheet, r as i32 + 1, 1, a).unwrap();
        s.type_in(sheet, r as i32 + 1, 2, b).unwrap();
    }
    (s, sheet)
}

#[test]
fn cells_without_a_header_are_all_data() {
    let (mut s, sheet) = fruit();
    // Summarise: the first row is data, the columns are called by their letters.
    let from = s.call("summary_from", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    assert_eq!(from["columns"], json!(["Column A", "Column B"]));
    let def = json!({ "from": from["from"], "group": ["Column A"], "show": [{ "column": "Column B", "how": "sum" }], "as_found": true, "totals": true });
    let p = s.call("summary_preview", &json!({ "def": def })).unwrap();
    assert_eq!(p["rows"][1], json!(["Apple", "1"]));
    assert_eq!(p["rows"][4], json!(["3 groups", "6"]));
    // A chart keeps Apple and has no made-up title.
    let c = s.call("add_chart", &json!({ "sheet": sheet, "range": range(1, 1, 3, 2), "row": 1, "col": 4 })).unwrap();
    assert_eq!(c["title"], "");
    let d = s.call("chart_data", &json!({ "id": c["id"] })).unwrap();
    assert_eq!(d["labels"], json!(["Apple", "Banana", "Pear"]));
    assert_eq!(d["series"][0]["values"], json!([1.0, 2.0, 3.0]));
}

#[test]
fn totals_on_cells_without_a_header_move_nothing() {
    let (mut s, sheet) = fruit();
    let undo_before = s.undo.len();
    // The cursor on a word: the total goes under the numbers.
    let got = s
        .call("totals_here", &json!({ "sheet": sheet, "row": 1, "col": 1, "total": "sum", "range": range(1, 1, 3, 2) }))
        .unwrap();
    assert_eq!(got["columns"], json!(["Column B"]));
    assert_eq!(text(&s, sheet, 1, 1), "Apple");
    assert_eq!(text(&s, sheet, 4, 2), "6");
    assert_eq!(text(&s, sheet, 4, 1), "3 rows");
    let t = tables::at(&s.book, sheet, 1, 1).unwrap();
    assert!(!t.header && t.totals);
    // One step of undo takes the table and its totals away.
    assert_eq!(s.undo.len(), undo_before + 1);
    s.undo().unwrap();
    assert!(tables::all(&s.book).is_empty());
    assert_eq!(input(&s, sheet, 4, 2), "");
    // An empty cell makes no table.
    assert!(s.call("make_table", &json!({ "sheet": sheet, "row": 10, "col": 6 })).is_err());
    assert!(tables::all(&s.book).is_empty());
}

#[test]
fn charts_add_rows_up() {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    let rows = [
        ["Month", "City", "Sales"],
        ["Jan", "York", "120"],
        ["Jan", "Leeds", "80"],
        ["Feb", "York", "150"],
        ["Feb", "York", "30"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            s.type_in(sheet, r as i32 + 1, c as i32 + 1, v).unwrap();
        }
    }
    let c = s.call("add_chart", &json!({ "sheet": sheet, "range": range(1, 1, 5, 3), "row": 1, "col": 6 })).unwrap();
    let mut chart: charts::Chart = serde_json::from_value(c.clone()).unwrap();
    assert_eq!(s.call("chart_columns", &json!({ "source": chart.source })).unwrap(), json!(["Month", "City", "Sales"]));
    chart.kind = charts::ChartKind::Stacked;
    chart.source.x = Some("Month".into());
    chart.source.split = Some("City".into());
    chart.source.values = vec![summary::Show { column: "Sales".into(), how: summary::How::Sum }];
    let undo_before = s.undo.len();
    s.call("set_chart", &json!({ "chart": chart })).unwrap();
    chart.title = "Sales".into();
    s.call("set_chart", &json!({ "chart": chart, "again": true })).unwrap();
    assert_eq!(s.undo.len(), undo_before + 1);
    let d = s.call("chart_data", &json!({ "id": chart.id })).unwrap();
    assert_eq!(d["labels"], json!(["Jan", "Feb"]));
    assert_eq!(d["series"][0]["name"], "York");
    assert_eq!(d["series"][0]["values"], json!([120.0, 180.0]));
    assert_eq!(d["series"][1]["values"], json!([80.0, null]));
    // To Excel as numbers (no cells hold them), stacked.
    let bytes = s.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut x = String::new();
    std::io::Read::read_to_string(&mut z.by_name("xl/charts/calcchart1.xml").unwrap(), &mut x).unwrap();
    assert!(
        x.contains("<c:grouping val=\"stacked\"/>") && x.contains("<c:numLit>") && x.contains("<c:v>York</c:v>"),
        "{x}"
    );
    // A copy on another sheet.
    let other = s.call("add_sheet", &json!({ "name": "Other" })).unwrap();
    let pasted = s.call("paste_chart", &json!({ "chart": chart, "sheet": other, "row": 2, "col": 2 })).unwrap();
    assert_ne!(pasted["id"], json!(chart.id));
    assert_eq!(s.book.charts.len(), 2);
    // After the paste and the new sheet, one undo takes the panel's changes back.
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.book.charts[0].kind, charts::ChartKind::Bar);
    assert!(s.book.charts[0].source.values.is_empty());
}

#[test]
fn complex_numbers_typed_with_a_sign_stay_text() {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    s.type_in(sheet, 1, 1, "-238+240i").unwrap();
    s.type_in(sheet, 2, 1, "-1+2").unwrap();
    s.type_in(sheet, 3, 1, "=IMREAL(A1)").unwrap();
    assert_eq!(text(&s, sheet, 1, 1), "-238+240i");
    assert_eq!(text(&s, sheet, 2, 1), "1");
    assert_eq!(text(&s, sheet, 3, 1), "-238");
}

#[test]
fn csv_keeps_long_numbers_and_other_separators() {
    let text = "id|amount|code|when\n12345678901234567890|1234.5678|007|2026-01-31\n2|0.1|+44|2026-02-01\n";
    assert_eq!(csv::separator(text), '|');
    let mut s = Session::new();
    s.call("import_csv", &json!({ "text": text, "name": "Pipes" })).unwrap();
    let sheet = s.book.id_at(0).unwrap();
    // A 20 digit id is an exact number (see ironcalc_base::num).
    assert_eq!(
        s.book.cell_data(sheet, 2, 1),
        Some(CellData::Number(ironcalc_base::num::Num::parse("12345678901234567890").unwrap()))
    );
    assert_eq!(stored_text(&s, sheet, 2, 3), "007");
    assert_eq!(stored_text(&s, sheet, 3, 3), "+44");
    assert_eq!(
        s.book.cell_data(sheet, 2, 2),
        Some(CellData::Number(ironcalc_base::num::Num::parse("1234.5678").unwrap()))
    );
    // Saved back with the same separator, numbers in full.
    let out = String::from_utf8(s.save_as(Format::Csv, None).unwrap()).unwrap();
    assert!(out.starts_with("id|amount|code|when\r\n12345678901234567890|1234.5678|007|"), "{out}");
    // Everything as text.
    let mut t = Session::new();
    t.call("import_csv", &json!({ "text": text, "values": "text" })).unwrap();
    assert_eq!(t.book.cell_data(t.book.id_at(0).unwrap(), 2, 2), Some(CellData::Text("1234.5678".into())));
    let p = s.call("csv_preview", &json!({ "text": "a;b\n1;2\n" })).unwrap();
    assert_eq!(p["sep"], ";");
}

#[test]
fn long_numbers_edit_without_losing_digits() {
    // What editing starts from (the formula bar) has every digit, so typing
    // it back in changes nothing (see ironcalc_base::num).
    let mut s = Session::new();
    s.call("import_csv", &json!({ "text": "id\n12345678901234567890\n" })).unwrap();
    let sheet = s.book.id_at(0).unwrap();
    let cell = s.call("cell", &json!({ "sheet": sheet, "row": 2, "col": 1 })).unwrap();
    let input = cell["input"].as_str().unwrap().to_string();
    assert_eq!(input, "12345678901234567890");
    s.call("type", &json!({ "sheet": sheet, "row": 2, "col": 1, "input": input })).unwrap();
    assert_eq!(
        s.book.cell_data(sheet, 2, 1),
        Some(CellData::Number(ironcalc_base::num::Num::parse("12345678901234567890").unwrap()))
    );
}

fn stored_text(s: &Session, sheet: u32, row: i32, col: i32) -> String {
    match s.book.cell_data(sheet, row, col) {
        Some(CellData::Text(t)) => t,
        other => format!("{other:?}"),
    }
}

#[test]
fn pictures_go_in_files_and_move_with_rows() {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    s.type_in(sheet, 1, 1, "Logo").unwrap();
    // A 1×1 PNG.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let pic = json!({ "sheet": sheet, "row": 2, "col": 2, "dx": 4, "dy": 4, "w": 120, "h": 80, "mime": "image/png", "data": png, "name": "logo.png", "alt": "Our logo" });
    let id = s.call("add_picture", &json!({ "picture": pic })).unwrap().as_u64().unwrap() as u32;
    // Rows added above: it moves down with its cell; undo brings it back.
    s.call("insert_rows", &json!({ "sheet": sheet, "at": 1, "count": 2 })).unwrap();
    assert_eq!(s.book.pictures[0].row, 4);
    s.undo().unwrap();
    assert_eq!(s.book.pictures[0].row, 2);
    // .xcalc: the bytes are a file of their own and come back.
    let bytes = s.save_as(Format::Xcalc, None).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    assert!(z.file_names().flatten().any(|n| n == format!("pictures/{id}.png")));
    let o = Session::open(&bytes).unwrap();
    assert_eq!(o.book.pictures.len(), 1);
    assert_eq!(o.book.pictures[0].data, png);
    assert_eq!(o.book.pictures[0].alt, "Our logo");
    // .xlsx: a real Excel picture, and Calc's again when opened.
    let x = s.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(x.clone())).unwrap();
    let mut drawing = String::new();
    std::io::Read::read_to_string(&mut z.by_name("xl/drawings/calcdrawing1.xml").unwrap(), &mut drawing).unwrap();
    assert!(drawing.contains("<xdr:pic>") && drawing.contains("descr=\"Our logo\""), "{drawing}");
    assert!(z.by_name("xl/media/calcpicture1.png").is_ok());
    let mut back = Session::open_file(&x, "Logo.xlsx").unwrap();
    assert_eq!(back.book.pictures.len(), 1);
    assert_eq!(back.book.pictures[0].data, png);
    let shown = back.call("pictures", &json!({ "sheet": back.book.id_at(0).unwrap() })).unwrap();
    assert_eq!(shown, json!([]), "Calc's picture isn't shown twice");
    // Saved again: still one picture in the drawing.
    let again = back.save_as(Format::Xlsx, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(again)).unwrap();
    let names: Vec<String> = z.file_names().flatten().map(|n| n.to_string()).collect();
    let drawing_name = names.iter().find(|n| n.starts_with("xl/drawings/") && n.ends_with(".xml")).unwrap().clone();
    let mut d = String::new();
    std::io::Read::read_to_string(&mut z.by_name(&drawing_name).unwrap(), &mut d).unwrap();
    assert_eq!(d.matches("<xdr:pic>").count(), 1, "{d}");
}

#[test]
fn a_calc_file_carries_what_write_links_to() {
    let (mut s, _) = sales();
    let snap = s.call("link_snapshot", &json!({})).unwrap();
    assert_eq!(snap["format"], "xenify-calc-link");
    s.call("set_link", &json!({ "link": snap })).unwrap();
    let bytes = s.save_as(Format::Xcalc, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut link = String::new();
    std::io::Read::read_to_string(&mut z.by_name("link.json").unwrap(), &mut link).unwrap();
    let back: serde_json::Value = serde_json::from_str(&link).unwrap();
    assert_eq!(back, snap);
    // The file still opens as a book.
    assert!(Session::open(&bytes).is_ok());
}

#[test]
fn merging_into_an_empty_top_left_loses_nothing() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 8, 2, "x").unwrap();
    let check = s.call("merge", &json!({ "sheet": sheet, "range": range(8, 1, 8, 3), "check": true })).unwrap();
    assert_eq!(check["lost"], 0, "the only value moves to the top left");
    s.type_in(sheet, 8, 3, "y").unwrap();
    let check = s.call("merge", &json!({ "sheet": sheet, "range": range(8, 1, 8, 3), "check": true })).unwrap();
    assert_eq!(check["lost"], 1);
}

#[test]
fn csv_writes_numbers_in_full_under_coloured_formats() {
    let (mut s, sheet) = sales();
    s.type_in(sheet, 2, 2, "1234.5678").unwrap();
    s.call("format", &json!({ "sheet": sheet, "range": range(2, 2, 2, 2), "set": { "num_fmt": "#,##0;[Red]-#,##0" } }))
        .unwrap();
    s.type_in(sheet, 3, 2, "46000").unwrap();
    s.call("format", &json!({ "sheet": sheet, "range": range(3, 2, 3, 2), "set": { "num_fmt": "yyyy-mm-dd" } }))
        .unwrap();
    let out = String::from_utf8(s.save_as(Format::Csv, Some(sheet)).unwrap()).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[1], "Jan,1234.5678");
    assert_eq!(lines[2], "Feb,2025-12-09", "dates as shown");
}

#[test]
fn replacing_any_case_where_lowercase_changes_lengths() {
    let mut s = Session::new();
    let sheet = s.book.id_at(0).unwrap();
    // The Kelvin sign lowercases to one byte, İ to three.
    s.type_in(sheet, 1, 1, "\u{212A}İİi").unwrap();
    s.type_in(sheet, 2, 1, "Grüße GRÜSSE").unwrap();
    let n = s.call("replace", &json!({ "query": "i", "with": "o", "options": {} })).unwrap();
    assert_eq!(n, 1);
    assert_eq!(text(&s, sheet, 1, 1), "\u{212A}İİo");
    s.call("replace", &json!({ "query": "GRÜ", "with": "x", "options": {} })).unwrap();
    assert_eq!(text(&s, sheet, 2, 1), "xße xSSE");
}

#[test]
fn summaries_without_groups_line_their_totals_up() {
    let (s, sheet) = sales();
    let d: summary::Def = serde_json::from_value(json!({
        "from": { "sheet": sheet, "range": { "top": 1, "left": 1, "bottom": 4, "right": 2 } },
        "group": [], "show": [{ "how": "rows" }, { "column": "Revenue", "how": "sum" }], "totals": true
    }))
    .unwrap();
    let r = summary::compute(&s.book, &d).unwrap();
    assert!(r.rows.iter().all(|l| l.len() == 2), "{:?}", r.rows);
    let last = r.rows.last().unwrap();
    assert!(matches!(&last[1], CellData::Number(n) if n.f() == 60.0), "{last:?}");
}

#[test]
fn duplicated_sheets_keep_their_merges() {
    let (mut s, sheet) = sales();
    s.call("merge", &json!({ "sheet": sheet, "range": range(6, 1, 6, 3) })).unwrap();
    let id = s.call("duplicate_sheet", &json!({ "sheet": sheet })).unwrap().as_u64().unwrap() as u32;
    let i = s.book.index(id).unwrap() as usize;
    assert_eq!(s.book.model.workbook.worksheets[i].merged_cells, s.book.model.workbook.worksheets[0].merged_cells);
}

#[test]
fn database_sheets_go_to_the_others_when_editing_together() {
    let mut s = Session::new();
    s.call("remote_open", &json!({ "server": "s", "node": 3, "name": "Shop", "data": "" })).unwrap();
    s.call("live_start", &json!({ "seq": 0, "conn": "a", "from": 1000, "to": 2000 })).unwrap();
    let def = json!({ "server": "http://office:7700", "source": 3, "source_name": "Shop", "query_name": "Orders", "sql": "select 1" });
    let answer = json!({ "columns": [["Item", "text"]], "rows": [["Tea"]], "truncated": false });
    s.call("add_query", &json!({ "def": def, "answer": answer })).unwrap();
    let def =
        json!({ "server": "http://office:7700", "source": 3, "source_name": "Shop", "table": "shops", "key": ["id"] });
    let answer = json!({ "columns": [["id", "number"], ["name", "text"]], "rows": [[1, "Mill"]], "truncated": false });
    s.call("add_editor", &json!({ "def": def, "answer": answer })).unwrap();
    let out = s.live_out().unwrap();
    let ops: Vec<String> = out["ops"].as_array().unwrap().iter().map(|o| o.as_str().unwrap().to_string()).collect();
    let has = |kind: &str| ops.iter().filter(|o| o.contains(&format!("\"op\":\"{kind}\""))).count();
    assert_eq!(has("add_sheet"), 2, "{ops:?}");
    assert_eq!(has("query"), 1, "{ops:?}");
    assert_eq!(has("editor"), 1, "{ops:?}");
    assert!(has("cells") >= 2, "{ops:?}");
}

#[test]
fn hidden_rows_and_columns_keep_their_sizes() {
    let (mut s, sheet) = sales();
    let i = s.book.index(sheet).unwrap();
    s.call("sizes", &json!({ "sheet": sheet, "rows": true, "lines": [{ "at": 3, "size": 40.0 }] })).unwrap();
    s.call("sizes", &json!({ "sheet": sheet, "lines": [{ "at": 2, "size": 150.0 }] })).unwrap();
    // A filter hides Feb's row, then shows it again: its height comes back.
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    s.call("filter", &json!({ "table": "Sales", "column": "Month", "keep": ["Jan", "Mar"] })).unwrap();
    assert!(s.book.model.is_row_hidden(i, 3).unwrap());
    s.call("filter", &json!({ "table": "Sales", "column": "Month", "keep": null })).unwrap();
    assert_eq!(s.book.model.get_row_height(i, 3).unwrap(), 40.0);
    // Hidden ones saved and opened, then shown: their sizes too.
    s.call("sizes", &json!({ "sheet": sheet, "rows": true, "lines": [{ "at": 3, "size": 40.0, "hidden": true }] }))
        .unwrap();
    s.call("sizes", &json!({ "sheet": sheet, "lines": [{ "at": 2, "size": 150.0, "hidden": true }] })).unwrap();
    let mut o = Session::open(&s.save().unwrap()).unwrap();
    let oi = o.book.index(sheet).unwrap();
    o.book.model.set_row_hidden(oi, 3, false).unwrap();
    o.book.model.set_column_hidden(oi, 2, false).unwrap();
    assert_eq!(o.book.model.get_row_height(oi, 3).unwrap(), 40.0);
    assert_eq!(o.book.model.get_column_width(oi, 2).unwrap(), 150.0);
    // Undoing the hiding gives back the sizes, not nothing.
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.book.model.get_row_height(i, 3).unwrap(), 40.0);
    assert_eq!(s.book.model.get_column_width(i, 2).unwrap(), 150.0);
    s.redo().unwrap();
    s.redo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.book.model.get_row_height(i, 3).unwrap(), 40.0);
    assert_eq!(s.book.model.get_column_width(i, 2).unwrap(), 150.0);
}

#[test]
fn undoing_a_taller_row_gives_back_its_height() {
    let (mut s, sheet) = sales();
    let i = s.book.index(sheet).unwrap();
    s.type_in(sheet, 6, 1, "one\ntwo\nthree").unwrap();
    assert!(s.book.model.get_row_height(i, 6).unwrap() > 25.0);
    s.undo().unwrap();
    assert_eq!(s.book.model.get_row_height(i, 6).unwrap(), 25.0);
}

#[test]
fn charts_summaries_and_filters_follow_renamed_tables_and_columns() {
    let (mut s, sheet) = sales();
    s.call("make_table", &json!({ "sheet": sheet, "row": 1, "col": 1 })).unwrap();
    let chart =
        s.call("add_chart", &json!({ "sheet": sheet, "range": range(1, 1, 4, 2), "row": 1, "col": 5 })).unwrap();
    let id = chart["id"].as_u64().unwrap() as u32;
    let d = json!({ "from": { "sheet": sheet, "table": "Sales" }, "group": ["Month"], "show": [{ "column": "Revenue", "how": "sum" }] });
    let sum = s.call("add_summary", &json!({ "def": d })).unwrap().as_u64().unwrap() as u32;
    s.call("filter", &json!({ "table": "Sales", "column": "Revenue", "keep": ["10", "30"] })).unwrap();

    // The header typed over, then the table renamed.
    s.type_in(sheet, 1, 2, "Takings").unwrap();
    s.call("rename_table", &json!({ "name": "Sales", "to": "Money" })).unwrap();
    let series =
        |s: &mut Session| s.call("chart_data", &json!({ "id": id })).unwrap()["series"].as_array().unwrap().len();
    assert_eq!(series(&mut s), 1, "the chart still has its series");
    let c = s.book.charts.iter().find(|c| c.id == id).unwrap();
    assert_eq!((c.source.table.as_deref(), c.source.series.clone()), (Some("Money"), vec!["Takings".to_string()]));
    let d = &s.book.summaries[&sum];
    assert_eq!((d.from.table.as_deref(), d.show[0].column.as_str()), (Some("Money"), "Takings"));
    assert_eq!(text(&s, sum, 1, 2), "Takings");
    assert_eq!(text(&s, sum, 3, 2), "30", "the summary still works out (Feb filtered out)");
    assert_eq!(s.book.filters["Money"][0].column, "Takings");
    assert!(!s.book.filters.contains_key("Sales"));

    // Undone, they all go back.
    s.undo().unwrap();
    s.undo().unwrap();
    let c = s.book.charts.iter().find(|c| c.id == id).unwrap();
    assert_eq!((c.source.table.as_deref(), c.source.series.clone()), (Some("Sales"), vec!["Revenue".to_string()]));
    assert_eq!(s.book.summaries[&sum].show[0].column, "Revenue");
    assert_eq!(s.book.filters["Sales"][0].column, "Revenue");
}

/// A sheet linked to a CSV file: read-only, read again when the file
/// changes (other sheets' formulas follow), kept in .xcalc files, only the
/// values in Excel files; Unlink makes it a normal sheet.
#[test]
fn linked_csv_sheets_follow_their_file() {
    let (mut s, sales) = sales();
    let def = json!({ "path": "/data/prices.csv", "name": "prices.csv", "sep": "", "values": "typed" });
    let text = "Item;Price\nApple;1.5\nPear;2\n";
    let id = s
        .call("add_csv_link", &json!({ "def": def, "text": text, "read": "2026-11-01T10:00:00Z" }))
        .unwrap()
        .as_u64()
        .unwrap() as u32;
    assert_eq!(s.book.sheet_name(id).unwrap(), "prices");
    assert_eq!(s.book.info(id).kind, SheetKind::LinkedCsv);
    assert_eq!(text_of(&s, id, 3, 2), "2");
    assert!(s.type_in(id, 2, 2, "9").is_err(), "a linked sheet can't be typed in");
    let got = s.call("csv_link", &json!({ "sheet": id })).unwrap();
    assert_eq!(got["path"], "/data/prices.csv");
    assert_eq!((got["rows"].as_u64(), got["columns"].as_u64()), (Some(3), Some(2)));
    assert_eq!(s.call("csv_links", &json!({})).unwrap()[0]["name"], "prices");

    // A formula elsewhere uses it, and follows the file.
    s.type_in(sales, 8, 1, "=SUM(prices!B:B)").unwrap();
    assert_eq!(text_of(&s, sales, 8, 1), "3.5");
    let changed = s
        .call(
            "fill_csv_link",
            &json!({ "sheet": id, "text": "Item;Price\nApple;1.5\nPear;2\nPlum;4\n", "read": "2026-11-01T10:05:00Z" }),
        )
        .unwrap();
    assert_eq!(changed, json!(true));
    assert_eq!(text_of(&s, sales, 8, 1), "7.5");
    // Fewer rows: what the file no longer has goes.
    s.call("fill_csv_link", &json!({ "sheet": id, "text": "Item;Price\nApple;1\n" })).unwrap();
    assert_eq!(text_of(&s, sales, 8, 1), "1");
    assert_eq!(text_of(&s, id, 3, 1), "");
    // The same text again changes nothing (no undo step), but notes when it was read.
    s.save().unwrap();
    let same = s
        .call("fill_csv_link", &json!({ "sheet": id, "text": "Item;Price\nApple;1\n", "read": "2026-11-01T11:00:00Z" }))
        .unwrap();
    assert_eq!(same, json!(false));
    assert!(!s.unsaved());
    assert_eq!(s.book.csv_links[&id].read, "2026-11-01T11:00:00Z");
    // Undo puts the last values back.
    s.undo().unwrap();
    assert_eq!(text_of(&s, sales, 8, 1), "7.5");
    s.redo().unwrap();

    // Saved and opened again: still linked, same values.
    let bytes = s.save().unwrap();
    let back = Session::open(&bytes).unwrap();
    assert_eq!(calc_json(&bytes)["version"], 2, "older Calcs can't read linked sheets: they're told to update");
    assert_eq!(back.book.info(id).kind, SheetKind::LinkedCsv);
    assert_eq!(back.book.csv_links[&id].path, "/data/prices.csv");
    assert_eq!(back.book.csv_links[&id].values, csv::Values::Typed);
    assert_eq!(text_of(&back, id, 2, 2), "1");
    // An Excel file keeps the values only.
    let x = xlsx::save(&s.book).unwrap();
    let xb = Session::open_file(&x, "book.xlsx").unwrap();
    let xid = xb.book.model.workbook.worksheets.iter().find(|w| w.name == "prices").unwrap().sheet_id;
    assert_eq!(xb.book.info(xid).kind, SheetKind::Normal);
    assert!(xb.book.csv_links.is_empty());
    assert_eq!(text_of(&xb, xid, 2, 2), "1");

    // Unlink: a normal sheet that keeps its values (one step of undo).
    s.call("unlink_csv", &json!({ "sheet": id })).unwrap();
    assert_eq!(s.book.info(id).kind, SheetKind::Normal);
    assert!(s.book.csv_links.is_empty());
    s.type_in(id, 2, 2, "5").unwrap();
    assert_eq!(text_of(&s, sales, 8, 1), "5");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.book.info(id).kind, SheetKind::LinkedCsv);

    // Deleting the sheet takes the link; undo brings it back.
    s.call("remove_sheet", &json!({ "sheet": id })).unwrap();
    assert!(s.book.csv_links.is_empty());
    s.undo().unwrap();
    assert!(s.book.csv_links.contains_key(&id));
}

/// Opening a CSV kept linked: a Calc book (Save asks where) whose sheet follows the file.
#[test]
fn opening_a_csv_linked() {
    let mut s = Session::new();
    let link = json!({ "path": "/data/a.csv", "name": "a.csv", "values": "text" });
    s.call("import_csv", &json!({ "text": "Code,N\n007,1\n", "name": "a", "values": "text", "link": link })).unwrap();
    let id = s.book.id_at(0).unwrap();
    assert_eq!(s.book.info(id).kind, SheetKind::LinkedCsv);
    assert_eq!(s.format, Format::Xcalc);
    assert!(s.unsaved());
    assert_eq!(s.book.csv_links[&id].rows, 2);
    s.call("fill_csv_link", &json!({ "sheet": id, "text": "Code,N\n007,1\n008,2\n" })).unwrap();
    assert_eq!(text_of(&s, id, 3, 1), "008", "read as text, as chosen");
}

fn calc_json(bytes: &[u8]) -> serde_json::Value {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    serde_json::from_reader(zip.by_name("calc.json").unwrap()).unwrap()
}

#[test]
fn plain_books_stay_readable_by_older_calcs_and_newer_books_say_update() {
    let (mut s, _) = sales();
    let bytes = s.save().unwrap();
    let v = calc_json(&bytes);
    assert_eq!((v["version"].as_u64(), v["saved_with"].as_u64()), (Some(1), Some(u64::from(file::VERSION))));

    let mut newer = v.clone();
    newer["version"] = (file::VERSION + 1).into();
    let err = file::FileBook::read(newer.to_string().as_bytes()).err().unwrap();
    assert!(err.contains("newer Calc"), "{err}");
    // Readable by its level, but with a kind of sheet only a newer Calc knows.
    let mut newer = v.clone();
    newer["saved_with"] = (file::VERSION + 1).into();
    newer["sheets"][0]["info"]["kind"] = "pivot_board".into();
    let err = file::FileBook::read(newer.to_string().as_bytes()).err().unwrap();
    assert!(err.contains("newer Calc"), "{err}");
    newer["saved_with"] = file::VERSION.into();
    assert!(file::FileBook::read(newer.to_string().as_bytes()).err().unwrap().contains("damaged"));
}
