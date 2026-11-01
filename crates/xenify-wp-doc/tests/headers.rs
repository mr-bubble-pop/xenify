//! Rich headers and footers: made from the simple line, edited like text.
use xenify_wp_doc::{Block, Document, Editor, Field, HfLine, HfSlot, hf, ops};

#[test]
fn simple_header_becomes_rich_and_edits_like_text() {
    let mut doc = Document::from_plain_text("Body text.");
    doc.sections[0].page.header = HfLine { left: "Acme".into(), right: "Page {page}".into(), ..Default::default() };
    let mut ed = Editor::new(doc);
    let words_before = ed.doc.word_count();
    let pid = hf::edit_on_page(&mut ed, 0, 1, true, false).unwrap();
    let part = &ed.doc.sections[0].headers[0];
    assert_eq!((part.footer, part.slot), (false, HfSlot::Default));
    let Block::Paragraph(p) = &part.blocks[0] else { panic!() };
    // "Acme ⇥ ⇥ Page #": the right part after two tabs, the page number a field.
    assert_eq!(p.text, format!("Acme\t\tPage {}", Field::STR));
    assert!(p.runs.iter().any(|r| r.fmt.field == Some(Field::Page)));
    assert_eq!(p.props.tabs.as_ref().map(Vec::len), Some(2));
    assert!(hf::in_header(&ed.doc, pid));
    // The caret is at its end; typing and Enter stay in the header.
    ops::insert_text(&mut ed, " — draft", None).unwrap();
    ops::split_paragraph(&mut ed).unwrap();
    ops::insert_text(&mut ed, "Second line", None).unwrap();
    let blocks = &ed.doc.sections[0].headers[0].blocks;
    assert_eq!(blocks.len(), 2);
    assert!(matches!(&blocks[1], Block::Paragraph(p) if p.text == "Second line"));
    // Headers don't count as words of the text.
    assert_eq!(ed.doc.word_count(), words_before);
    // Asking again goes to the same header.
    assert_eq!(hf::edit_on_page(&mut ed, 0, 3, false, false).unwrap(), pid);
    // Undo all the way: no rich header.
    while ed.can_undo() {
        ed.undo();
    }
    assert!(ed.doc.sections[0].headers.is_empty());
    // Back to plain removes it.
    hf::edit_on_page(&mut ed, 0, 1, true, true).unwrap();
    assert_eq!(ed.doc.sections[0].headers.len(), 1);
    hf::make_plain(&mut ed, 0, true, HfSlot::Default).unwrap();
    assert!(ed.doc.sections[0].headers.is_empty());
}
