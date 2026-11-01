//! A table's look (header row/column, bands, header colour) survives saving
//! and opening in every format that holds tables, so Table properties shows
//! the same settings afterwards.

use xenify_wp_doc::{Block, Document, Editor, Rgba, Table, ops, tables};
use xenify_wp_io::{Format, load, save};

fn the_table(d: &Document) -> Table {
    d.sections[0].blocks.iter().find_map(|b| if let Block::Table(t) = b { Some(t.clone()) } else { None }).unwrap()
}

#[test]
fn table_look_round_trips() {
    let mut ed = Editor::new(Document::new());
    tables::insert_table(&mut ed, 4, 3).unwrap();
    ops::insert_text(&mut ed, "Head", None).unwrap();
    let mut t = the_table(&ed.doc);
    t.band = Some(Rgba::rgb(0xf4, 0xf7, 0xfb));
    t.header_col = true;
    t.header_fill = Some(Rgba::rgb(0xdd, 0xee, 0xff));
    t.rows[0].header = true;
    // One cell coloured on its own.
    t.rows[2].cells[2].shading = Some(Rgba::rgb(0xff, 0x00, 0x00));
    tables::replace_block(&mut ed, t.id, Block::Table(t), "look").unwrap();
    for f in [Format::Markdown, Format::Docx, Format::Igdoc] {
        let back = the_table(&load(&save(&ed.doc, f).unwrap(), f).unwrap().doc);
        assert_eq!(back.band, Some(Rgba::rgb(0xf4, 0xf7, 0xfb)), "{f:?} bands");
        assert!(back.header_col, "{f:?} header column");
        assert!(back.rows[0].header, "{f:?} header row");
        assert_eq!(back.header_fill, Some(Rgba::rgb(0xdd, 0xee, 0xff)), "{f:?} header colour");
        // Band and header colours aren't stuck onto the cells (they'd hide later changes)…
        let own: Vec<_> = back.rows.iter().flat_map(|r| &r.cells).filter_map(|c| c.shading).collect();
        // …but the cell coloured on its own keeps its colour.
        assert_eq!(own, vec![Rgba::rgb(0xff, 0, 0)], "{f:?} cell colours");
    }
}

#[test]
fn a_centred_title_page_round_trips() {
    let mut ed = Editor::new(Document::from_plain_text("Body text"));
    ops::insert_title_page(&mut ed).unwrap();
    for f in [Format::Markdown, Format::Docx, Format::Igdoc] {
        let back = load(&save(&ed.doc, f).unwrap(), f).unwrap().doc;
        assert_eq!(back.sections.len(), 2, "{f:?}");
        assert_eq!(back.sections[0].page.v_align, xenify_wp_doc::PageVAlign::Center, "{f:?}");
        assert_eq!(back.sections[1].page.v_align, xenify_wp_doc::PageVAlign::Top, "{f:?}");
    }
}
