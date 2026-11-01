use xenify_wp_doc::{Block, Document, Editor, Pos, Selection, StyleSheet, ops, tables};
use xenify_wp_layout::{DocLayout, Engine, Fonts};

fn layout(doc: &Document) -> DocLayout {
    Engine::new(Fonts::bundled_only()).layout(doc)
}

fn table_id(ed: &Editor) -> xenify_wp_doc::NodeId {
    ed.doc.sections[0].blocks.iter().find_map(|b| if let Block::Table(t) = b { Some(t.id) } else { None }).unwrap()
}

#[test]
fn table_cells_are_laid_out_in_a_grid() {
    let mut ed = Editor::new(Document::from_plain_text("Above"));
    ed.sel = Selection::caret(ed.doc.end());
    tables::insert_table(&mut ed, 2, 3).unwrap();
    for w in ["a", "b", "c", "d", "e", "a much longer cell that wraps onto more lines than the others do"] {
        ops::insert_text(&mut ed, w, None).unwrap();
        tables::move_cell(&mut ed, true).unwrap();
    }
    let t = ed.doc.table(table_id(&ed)).unwrap().clone();
    let l = layout(&ed.doc);
    let rect = |r: usize, c: usize| l.fragments[l.by_para[&tables::first_para(&t, r, c).unwrap()][0]].rect();
    // Same row: same top; later columns further right.
    assert!((rect(0, 0).y0 - rect(0, 2).y0).abs() < 0.5);
    assert!(rect(0, 1).x0 > rect(0, 0).x0 && rect(0, 2).x0 > rect(0, 1).x0);
    // The second row starts below the first; the tall cell made row 2 taller.
    assert!(rect(1, 0).y0 > rect(0, 0).y1);
    assert!(rect(1, 2).height() > rect(1, 0).height());
    // Borders were drawn; the caret and clicks work in cells.
    assert!(l.shapes.iter().any(|s| s.front));
    let p = tables::first_para(&t, 1, 1).unwrap();
    let c = l.caret(Pos::new(p, 0), false).unwrap();
    assert_eq!(l.hit(kurbo::Point::new(c.x0 + 0.5, c.center().y)).unwrap().para, p);
}

#[test]
fn long_tables_break_between_rows_and_can_repeat_headers_and_footers() {
    let mut ed = Editor::new(Document::new());
    tables::insert_table(&mut ed, 60, 2).unwrap();
    tables::toggle_header_rows(&mut ed).unwrap();
    let t = ed.doc.table(table_id(&ed)).unwrap().clone();
    let l = layout(&ed.doc);
    assert!(l.pages.len() >= 2, "{} pages", l.pages.len());
    // Every row is whole on one page.
    for r in 0..t.rows.len() {
        let f = l.fragments[l.by_para[&tables::first_para(&t, r, 0).unwrap()][0]].rect();
        let pg = l.pages.iter().find(|p| p.rect.contains(f.center())).unwrap();
        assert!(f.y1 <= pg.body.y1 + 0.5);
    }
    // The header row isn't repeated on the second page unless that's asked for.
    let p2 = &l.pages[1];
    assert!(!l.decor.iter().any(|d| p2.body.contains(d.rect().center())));
    let id = table_id(&ed);
    tables::set_repeats(&mut ed, id, Some(true), None).unwrap();
    let l = layout(&ed.doc);
    let p2 = &l.pages[1];
    assert!(l.decor.iter().any(|d| p2.body.contains(d.rect().center())), "a copy on page 2");
    // A footer row shown at the bottom of each page too: a copy on page 1.
    tables::set_repeats(&mut ed, id, Some(false), Some(true)).unwrap();
    tables::set_footer(&mut ed, id, true).unwrap();
    let l = layout(&ed.doc);
    let p1 = &l.pages[0];
    assert!(l.decor.iter().any(|d| p1.body.contains(d.rect().center())), "the footer's copy on page 1");
    assert!(
        !l.decor.iter().any(|d| l.pages[1].body.contains(d.rect().center()) && d.rect().y0 < l.pages[1].body.y0 + 30.0),
        "no header copy"
    );
}

#[test]
fn images_and_contents() {
    let mut ed = Editor::new(Document::from_plain_text("Intro\nChapter one\ntext\nChapter two\nmore"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    for i in [1, 3] {
        ed.sel = Selection::caret(Pos::new(ids[i], 0));
        ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    }
    ed.sel = Selection::caret(Pos::new(ids[3], 0));
    ops::insert_page_break(&mut ed).ok();
    ed.sel = Selection::caret(Pos::new(ids[0], 5));
    tables::insert_toc(&mut ed, 3).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[2], 4));
    tables::insert_image(&mut ed, "media/x.png", 300.0, 150.0).unwrap();
    let l = layout(&ed.doc);
    assert_eq!(l.images.len(), 1);
    assert!((l.images[0].rect.width() - 300.0).abs() < 0.1);
    // Two contents entries that link to their headings.
    let entries: Vec<_> = l.fragments.iter().filter(|f| f.para.link.is_some()).collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        l.link_at(entries[1].rect().center()),
        ed.doc.paragraphs().find(|p| p.text == "Chapter two").map(|p| p.id)
    );
}

#[test]
fn emoji_in_a_cell_stay_inside_it_and_the_cell_is_easy_to_click() {
    let mut ed = Editor::new(Document::new());
    tables::insert_table(&mut ed, 2, 2).unwrap();
    ops::insert_text(&mut ed, "😀😀", None).unwrap();
    let t = ed.doc.table(table_id(&ed)).unwrap().clone();
    let l = layout(&ed.doc);
    let p = tables::first_para(&t, 0, 0).unwrap();
    let f = &l.fragments[l.by_para[&p][0]];
    let cell = l.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap().rect;
    let line = f.rect();
    assert!(line.height() > 12.0, "a line of emoji is as tall as text: {line:?}");
    for i in &f.para.items {
        if let xenify_wp_layout::Item::Emoji { rect, .. } = i {
            let r = *rect + f.origin.to_vec2();
            assert!(r.y0 >= cell.y0 && r.y0 >= line.y0 - 0.01 && r.y1 <= line.y1 + 0.01, "{r:?} in {line:?}");
            assert!(r.height() > 11.0, "drawn a little larger than the text");
        }
    }
    for dy in [-6.0, 0.0, 6.0] {
        assert_eq!(l.hit(kurbo::Point::new(line.x0 + 30.0, line.center().y + dy)).unwrap().para, p);
    }
}
