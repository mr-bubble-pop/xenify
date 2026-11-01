//! Table editing: selecting rows and columns, sorting, widths, styles,
//! pictures and tables inside cells.
use xenify_wp_doc::{Block, Document, Editor, Pos, Selection, VAlign, ops, tables};

/// A table with a header row and three body rows, filled with `cells`.
fn table(cells: &[[&str; 2]]) -> (Editor, xenify_wp_doc::NodeId) {
    let mut ed = Editor::new(Document::from_plain_text(""));
    tables::insert_table(&mut ed, cells.len(), 2).unwrap();
    let id = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, Block::Table(_))).unwrap().id();
    for (r, row) in cells.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            let p = tables::first_para(ed.doc.table(id).unwrap(), r, c).unwrap();
            ed.sel = Selection::caret(Pos::new(p, 0));
            ops::insert_text(&mut ed, text, None).unwrap();
        }
    }
    (ed, id)
}

fn column(ed: &Editor, id: xenify_wp_doc::NodeId, c: usize) -> Vec<String> {
    let t = ed.doc.table(id).unwrap();
    (0..t.rows.len())
        .map(|r| {
            let p = tables::first_para(t, r, c).unwrap();
            ed.doc.para(p).unwrap().text.clone()
        })
        .collect()
}

#[test]
fn sorting_keeps_the_header_and_reads_numbers() {
    let (mut ed, id) = table(&[["Fruit", "Price"], ["pear", "10"], ["Apple", "9.5"], ["fig", "100"]]);
    let p = tables::first_para(ed.doc.table(id).unwrap(), 0, 0).unwrap();
    ed.sel = Selection::caret(Pos::new(p, 0));
    tables::toggle_header_rows(&mut ed).unwrap();
    tables::sort_rows(&mut ed, id, 0, false).unwrap();
    assert_eq!(column(&ed, id, 0), ["Fruit", "Apple", "fig", "pear"]);
    tables::sort_rows(&mut ed, id, 1, true).unwrap();
    assert_eq!(column(&ed, id, 1), ["Price", "100", "10", "9.5"], "numbers, largest first");
    ed.undo();
    assert_eq!(column(&ed, id, 0), ["Fruit", "Apple", "fig", "pear"]);
}

#[test]
fn selecting_columns_and_rows_then_formatting_them() {
    let (mut ed, id) = table(&[["a", "b"], ["c", "d"], ["e", "f"]]);
    assert!(tables::select_column(&mut ed, id, 1));
    assert_eq!(tables::selected_cells(&ed).map(|(_, r, c)| (r, c)), Some((0..=2, 1..=1)));
    tables::set_v_align(&mut ed, VAlign::Middle).unwrap();
    let t = ed.doc.table(id).unwrap();
    assert!(t.rows.iter().all(|r| r.cells[1].v_align == VAlign::Middle && r.cells[0].v_align == VAlign::Top));
    assert!(tables::select_row(&mut ed, id, 2));
    assert_eq!(tables::selected_cells(&ed).map(|(_, r, c)| (r, c)), Some((2..=2, 0..=1)));
    tables::set_row_height(&mut ed, Some(40.0)).unwrap();
    assert_eq!(ed.doc.table(id).unwrap().rows[2].min_height, Some(40.0));
}

#[test]
fn widths_merge_while_dragging_and_styles_apply() {
    let (mut ed, id) = table(&[["a", "b"], ["c", "d"]]);
    ed.seal();
    for w in [1.1, 1.2, 1.4] {
        tables::set_widths(&mut ed, id, vec![w, 2.0 - w]).unwrap();
    }
    assert_eq!(ed.doc.table(id).unwrap().widths, [1.4, 0.6]);
    ed.undo();
    assert_eq!(ed.doc.table(id).unwrap().widths, [1.0, 1.0], "one undo step for the whole drag");
    tables::apply_style(&mut ed, id, tables::TableStyle::Striped).unwrap();
    let t = ed.doc.table(id).unwrap();
    assert!(t.rows[0].header && t.band.is_some() && t.border.is_none());
    // Inserting a row and a column at the edges.
    tables::insert_column_at(&mut ed, id, 2).unwrap();
    tables::insert_row_at(&mut ed, id, 0).unwrap();
    let t = ed.doc.table(id).unwrap();
    assert_eq!((t.rows.len(), t.cols()), (3, 3));
}

#[test]
fn pictures_and_tables_go_inside_cells() {
    let (mut ed, id) = table(&[["a", "b"]]);
    let p = tables::first_para(ed.doc.table(id).unwrap(), 0, 1).unwrap();
    ed.sel = Selection::caret(Pos::new(p, 1));
    tables::insert_image(&mut ed, "media/x.png", 40.0, 20.0).unwrap();
    let cell = &ed.doc.table(id).unwrap().rows[0].cells[1];
    assert!(cell.blocks.iter().any(|b| matches!(b, Block::Image(_))), "the picture is in the cell");
    let p = tables::first_para(ed.doc.table(id).unwrap(), 0, 0).unwrap();
    ed.sel = Selection::caret(Pos::new(p, 1));
    tables::insert_table(&mut ed, 2, 2).unwrap();
    let cell = &ed.doc.table(id).unwrap().rows[0].cells[0];
    assert!(cell.blocks.iter().any(|b| matches!(b, Block::Table(_))), "a table in the cell");
}

#[test]
fn copying_cells_and_pasting_spreadsheet_text() {
    use xenify_wp_doc::clip;
    let (mut ed, id) = table(&[["a", "b"], ["c", "d"], ["e", "f"]]);
    // Copy the bottom-right 2×1 block and paste it at the top-left.
    tables::select_cells(&mut ed, id, 1..=2, 1..=1);
    let frag = clip::copy_fragment(&ed);
    let Block::Table(t) = &frag.sections[0].blocks[0] else { panic!("a table: {:?}", frag.sections[0].blocks) };
    assert_eq!((t.rows.len(), t.cols()), (2, 1));
    let p = tables::first_para(ed.doc.table(id).unwrap(), 0, 0).unwrap();
    ed.sel = Selection::caret(Pos::new(p, 0));
    clip::paste_fragment(&mut ed, &frag, "Paste").unwrap();
    assert_eq!(column(&ed, id, 0), ["d", "f", "e"]);
    // Spreadsheet text pasted in the last row grows the table.
    let p = tables::first_para(ed.doc.table(id).unwrap(), 2, 0).unwrap();
    ed.sel = Selection::caret(Pos::new(p, 0));
    let tsv = clip::tsv_fragment("1\t2\n3\t4\n").unwrap();
    clip::paste_fragment(&mut ed, &tsv, "Paste").unwrap();
    assert_eq!(column(&ed, id, 1), ["b", "d", "2", "4"]);
    // Outside a table it makes a new one; one line of text isn't a table.
    assert!(clip::tsv_fragment("just\ttext").is_none());
    let mut plain = Editor::new(Document::from_plain_text("x"));
    clip::paste_fragment(&mut plain, &clip::tsv_fragment("a\tb\nc\td").unwrap(), "Paste").unwrap();
    assert!(plain.doc.sections[0].blocks.iter().any(|b| matches!(b, Block::Table(_))));
}

#[test]
fn sorting_reads_merged_cells_and_refuses_row_spans() {
    let (mut ed, id) = table(&[["b", "2"], ["c", "3"], ["a", "1"]]);
    // Merge row 1's two cells: its text ("c") shows in both columns.
    let t = ed.doc.table(id).unwrap();
    let (p0, p1) = (tables::first_para(t, 1, 0).unwrap(), tables::first_para(t, 1, 1).unwrap());
    ed.sel = Selection { anchor: Pos::new(p0, 0), focus: Pos::new(p1, 0) };
    tables::merge_cells(&mut ed).unwrap();
    assert!(tables::can_sort(&ed.doc, id));
    // Sorting by the second column uses the merged cell's text for that row.
    tables::sort_rows(&mut ed, id, 1, false).unwrap();
    assert_eq!(column(&ed, id, 0)[0], "a");
    // A cell spanning two rows: rows can't be sorted.
    let t = ed.doc.table(id).unwrap();
    let (a, b) = (tables::first_para(t, 0, 0).unwrap(), tables::first_para(t, 1, 0).unwrap());
    ed.sel = Selection { anchor: Pos::new(a, 0), focus: Pos::new(b, 0) };
    tables::merge_cells(&mut ed).unwrap();
    assert!(!tables::can_sort(&ed.doc, id));
    assert!(tables::sort_rows(&mut ed, id, 0, false).is_err());
}

#[test]
fn captions_number_themselves_and_are_found_again() {
    let mut ed = Editor::new(Document::from_plain_text("Intro"));
    let p = ed.doc.paragraphs().next().unwrap().id;
    ed.sel = Selection::caret(Pos::new(p, 5));
    tables::insert_table(&mut ed, 2, 2).unwrap();
    let t1 = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, Block::Table(_))).unwrap().id();
    ops::add_caption(&mut ed, t1).unwrap();
    ops::insert_text(&mut ed, "Prices", None).unwrap();
    let cap = ed.doc.para(ed.sel.focus.para).unwrap();
    assert_eq!(cap.text, "Table 1: Prices");
    assert_eq!(cap.style, xenify_wp_doc::StyleSheet::CAPTION);
    // Again: the caret goes to the caption that's there.
    ed.sel = Selection::caret(Pos::new(p, 0));
    ops::add_caption(&mut ed, t1).unwrap();
    assert_eq!(ed.doc.para(ed.sel.focus.para).unwrap().text, "Table 1: Prices");
    let captions = ed.doc.paragraphs().filter(|p| p.style == xenify_wp_doc::StyleSheet::CAPTION).count();
    assert_eq!(captions, 1);
    ed.undo();
    ed.undo();
    assert_eq!(ed.doc.paragraphs().filter(|p| p.style == xenify_wp_doc::StyleSheet::CAPTION).count(), 0);
}

#[test]
fn a_caption_moves_with_its_picture() {
    use xenify_wp_doc::{Wrap, frames};
    let mut ed = Editor::new(Document::from_plain_text("Intro"));
    let p = ed.doc.paragraphs().next().unwrap().id;
    ed.sel = Selection::caret(Pos::new(p, 5));
    tables::insert_image(&mut ed, "media/a.png", 100.0, 80.0).unwrap();
    let img = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, Block::Image(_))).unwrap().id();
    ops::add_caption(&mut ed, img).unwrap();
    ops::insert_text(&mut ed, "A hill", None).unwrap();
    let frame = frames::float_image(&mut ed, img, 0, 0, (50.0, 50.0), Wrap::Around).unwrap();
    let f = ed.doc.frame(frame).unwrap();
    assert_eq!(f.blocks.len(), 2, "picture and caption");
    assert!(
        !ed.doc.sections[0].blocks.iter().any(|b| matches!(b, Block::Paragraph(p) if p.text == "Figure 1: A hill"))
    );
    frames::inline_image(&mut ed, frame, None).unwrap();
    let body = &ed.doc.sections[0].blocks;
    let i = body.iter().position(|b| matches!(b, Block::Image(_))).unwrap();
    assert!(matches!(&body[i + 1], Block::Paragraph(p) if p.text == "Figure 1: A hill"));
}
