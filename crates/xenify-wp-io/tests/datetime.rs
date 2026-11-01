//! Date and time fields keep their format (and fixed text) through Markdown,
//! Xenify documents and Word.

use xenify_wp_doc::{Editor, Field, ops};
use xenify_wp_io::{Format, load, save};

fn fields(doc: &xenify_wp_doc::Document) -> Vec<Field> {
    doc.paragraphs().flat_map(|p| p.runs.iter().filter_map(|r| r.fmt.field.clone())).collect()
}

#[test]
fn date_fields_survive_saving() {
    let mut ed = Editor::new(xenify_wp_doc::Document::from_plain_text("Written on "));
    let end = ed.doc.paragraphs().next().unwrap().text.len();
    ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(ed.doc.paragraphs().next().unwrap().id, end));
    ops::insert_field(&mut ed, Field::DateTime { format: "d MMMM yyyy".into(), fixed: None }, "Date").unwrap();
    ops::insert_field(&mut ed, Field::DateTime { format: "HH:mm".into(), fixed: Some("09:30".into()) }, "Time")
        .unwrap();
    let want = fields(&ed.doc);
    for format in [Format::Markdown, Format::Igdoc] {
        let back = load(&save(&ed.doc, format).unwrap(), format).unwrap().doc;
        assert_eq!(fields(&back), want, "{format:?}");
    }
    // Word: the live one stays a DATE field with its format; the fixed one is its text.
    let word = load(&save(&ed.doc, Format::Docx).unwrap(), Format::Docx).unwrap().doc;
    assert_eq!(fields(&word), vec![Field::DateTime { format: "d MMMM yyyy".into(), fixed: None }]);
    assert!(word.paragraphs().next().unwrap().text.contains("09:30"));
}
