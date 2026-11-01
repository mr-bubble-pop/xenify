use xenify_wp_doc::{Document, Editor, ListDef, Pos, Selection, StyleSheet, ops, tables};
use xenify_wp_io::{Format, save};

#[test]
fn html_export() {
    let mut ed = Editor::new(Document::from_plain_text("Title <&>\nOne\nTwo\nbody"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[0], 0));
    ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    ed.sel = Selection { anchor: Pos::new(ids[1], 0), focus: Pos::new(ids[2], 1) };
    ops::toggle_list(&mut ed, ListDef::NUMBERS).unwrap();
    ed.sel = Selection::caret(ed.doc.end());
    tables::insert_table(&mut ed, 2, 2).unwrap();
    ops::insert_text(&mut ed, "cell", None).unwrap();
    let html = String::from_utf8(save(&ed.doc, Format::Html).unwrap()).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<h1 id=\"p"), "{html}");
    assert!(html.contains("Title &lt;&amp;&gt;"));
    assert!(html.contains("<ol") && html.contains("<li"));
    assert!(html.contains("<table") && html.contains(">cell</p>"));
    assert!(html.contains("@page { size: 595.28pt"));
    assert_eq!(html.matches("<ol").count(), html.matches("</ol>").count());
}
