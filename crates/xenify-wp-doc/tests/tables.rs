use xenify_wp_doc::{Block, Container, Document, Editor, Pos, Selection, ops, tables};

fn table(ed: &Editor) -> xenify_wp_doc::Table {
    ed.doc.sections[0]
        .blocks
        .iter()
        .find_map(|b| if let Block::Table(t) = b { Some(t.clone()) } else { None })
        .expect("a table")
}

fn cell_text(t: &xenify_wp_doc::Table, r: usize, c: usize) -> String {
    t.rows[r].cells[c]
        .blocks
        .iter()
        .filter_map(|b| if let Block::Paragraph(p) = b { Some(p.text.clone()) } else { None })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn insert_type_tab_and_undo() {
    let mut ed = Editor::new(Document::from_plain_text("before"));
    ed.sel = Selection::caret(ed.doc.end());
    tables::insert_table(&mut ed, 2, 3).unwrap();
    let t = table(&ed);
    assert_eq!((t.rows.len(), t.cols()), (2, 3));
    // The caret is in the first cell; a paragraph follows the table.
    assert!(matches!(ed.doc.locate(ed.sel.focus.para).unwrap().0, Container::Cell { row: 0, col: 0, .. }));
    assert!(matches!(ed.doc.sections[0].blocks.last(), Some(Block::Paragraph(_))));
    for word in ["a", "b", "c", "d", "e", "f", "g"] {
        ops::insert_text(&mut ed, word, None).unwrap();
        tables::move_cell(&mut ed, true).unwrap();
    }
    let t = table(&ed);
    assert_eq!(t.rows.len(), 3, "Tab in the last cell added a row");
    assert_eq!(cell_text(&t, 1, 2), "f");
    assert_eq!(cell_text(&t, 2, 0), "g");
    // The last Tab moved to cell (2,1); Enter there makes a second paragraph in the cell.
    ops::insert_text(&mut ed, "h", None).unwrap();
    ops::split_paragraph(&mut ed).unwrap();
    ops::insert_text(&mut ed, "i", None).unwrap();
    assert_eq!(cell_text(&table(&ed), 2, 1), "h/i");
    // Backspace at the start of a cell doesn't join with the previous cell.
    let first = tables::first_para(&table(&ed), 1, 0).unwrap();
    ed.sel = Selection::caret(Pos::new(first, 0));
    ops::backspace(&mut ed, false).unwrap();
    assert_eq!(cell_text(&table(&ed), 1, 0), "d");
    while ed.undo() {}
    assert_eq!(ed.doc.plain_text(), "before");
}

#[test]
fn rows_columns_merge_and_delete() {
    let mut ed = Editor::new(Document::new());
    tables::insert_table(&mut ed, 3, 3).unwrap();
    tables::insert_column(&mut ed, true).unwrap();
    assert_eq!(table(&ed).cols(), 4);
    tables::insert_row(&mut ed, false).unwrap();
    assert_eq!(table(&ed).rows.len(), 4);
    // Select cells (1,0)–(2,1) and merge them.
    let t = table(&ed);
    ed.sel = Selection {
        anchor: Pos::new(tables::first_para(&t, 1, 0).unwrap(), 0),
        focus: Pos::new(tables::first_para(&t, 2, 1).unwrap(), 0),
    };
    ops::insert_text(&mut ed, "", None).ok();
    ed.sel = Selection {
        anchor: Pos::new(tables::first_para(&t, 1, 0).unwrap(), 0),
        focus: Pos::new(tables::first_para(&t, 2, 1).unwrap(), 0),
    };
    tables::merge_cells(&mut ed).unwrap();
    let t = table(&ed);
    assert_eq!((t.rows[1].cells[0].col_span, t.rows[1].cells[0].row_span), (2, 2));
    assert!(t.rows[2].cells[1].covered);
    // Covered cells aren't in reading order.
    let paras = ed.doc.paragraphs().count();
    assert_eq!(paras, 4 * 4 - 3 + 1);
    // Deleting a column through the merged cell shrinks it.
    ed.sel = Selection::caret(Pos::new(tables::first_para(&t, 1, 0).unwrap(), 0));
    ed.sel = Selection::caret(Pos::new(tables::first_para(&t, 0, 1).unwrap(), 0));
    tables::delete_columns(&mut ed).unwrap();
    let t = table(&ed);
    assert_eq!(t.cols(), 3);
    assert_eq!(t.rows[1].cells[0].col_span, 1);
    tables::toggle_header_rows(&mut ed).ok();
    tables::delete_table(&mut ed).unwrap();
    assert!(ed.doc.sections[0].blocks.iter().all(|b| matches!(b, Block::Paragraph(_))));
    while ed.undo() {}
    assert_eq!(ed.doc.paragraphs().count(), 1);
}

#[test]
fn deleting_across_a_table_removes_it() {
    let mut ed = Editor::new(Document::from_plain_text("top"));
    ed.sel = Selection::caret(ed.doc.end());
    tables::insert_table(&mut ed, 1, 2).unwrap();
    let last = ed.doc.sections[0].blocks.last().unwrap().id();
    ed.sel = Selection::caret(Pos::new(last, 0));
    ops::insert_text(&mut ed, "bottom", None).unwrap();
    let first = ed.doc.start().para;
    ed.sel = Selection { anchor: Pos::new(first, 1), focus: Pos::new(last, 3) };
    ops::delete_selection(&mut ed).unwrap();
    assert_eq!(ed.doc.plain_text(), "ttom");
    assert_eq!(ed.doc.sections[0].blocks.len(), 1);
    ed.undo();
    assert_eq!(ed.doc.paragraphs().count(), 4);
}

#[test]
fn tables_survive_json() {
    let mut ed = Editor::new(Document::new());
    tables::insert_table(&mut ed, 2, 2).unwrap();
    ops::insert_text(&mut ed, "x", None).unwrap();
    let json = serde_json::to_string(&ed.doc).unwrap();
    let mut back: Document = serde_json::from_str(&json).unwrap();
    back.repair();
    assert_eq!(back.plain_text(), ed.doc.plain_text());
}
