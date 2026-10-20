//! Contents tables to and from Word: the levels and page-number setting of a
//! TOC inside a content control, and page numbers in the TOC we write.

use xenify_wp_doc::{Block, Document, Editor, Pos, Selection, StyleSheet, ops, tables};
use xenify_wp_io::zip::{ZipReader, ZipWriter};
use xenify_wp_io::{Format, load, save};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

fn docx(body: &str) -> Vec<u8> {
    let mut z = ZipWriter::new();
    z.add("[Content_Types].xml", br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#, true);
    z.add("_rels/.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#, true);
    z.add(
        "word/document.xml",
        format!(r#"<?xml version="1.0"?><w:document {W}><w:body>{body}</w:body></w:document>"#).as_bytes(),
        true,
    );
    z.finish()
}

fn tocs(doc: &Document) -> Vec<(u8, bool)> {
    doc.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            Block::Toc(t) => Some((t.levels, t.page_numbers)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_toc_content_control_keeps_its_levels() {
    let sdt = |instr: &str| {
        format!(
            r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/></w:docPartObj></w:sdtPr><w:sdtContent><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">{instr}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Old entry</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:sdtContent></w:sdt>"#
        )
    };
    let body = format!(
        "{}{}<w:p><w:r><w:t>Text</w:t></w:r></w:p>",
        sdt(r#" TOC \o "1-2" \h \z \u "#),
        sdt(r#" TOC \o "1-4" \n \h "#)
    );
    let doc = load(&docx(&body), Format::Docx).unwrap().doc;
    assert_eq!(tocs(&doc), [(2, true), (4, false)]);
}

#[test]
fn our_toc_has_page_numbers_for_word() {
    let mut ed = Editor::new(Document::from_plain_text("Contents\nIntroduction\nBody text\nResults\nMore text"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    for i in [1, 3] {
        ed.sel = Selection::caret(Pos::new(ids[i], 0));
        ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    }
    // Results on page 2.
    ed.sel = Selection::caret(Pos::new(ids[3], 0));
    ops::insert_page_break(&mut ed).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[0], 0));
    tables::insert_toc(&mut ed, 3).unwrap();
    let bytes = save(&ed.doc, Format::Docx).unwrap();
    let xml = ZipReader::new(&bytes).unwrap().read_string("word/document.xml").unwrap();
    assert!(xml.contains(r#"w:leader="dot""#), "dot leaders");
    assert!(xml.contains("Introduction</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>1</w:t>"), "{xml}");
    assert!(xml.contains("Results</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>2</w:t>"), "{xml}");
    // And it reads back as a live contents table.
    assert_eq!(tocs(&load(&bytes, Format::Docx).unwrap().doc), [(3, true)]);
}
