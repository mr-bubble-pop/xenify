//! Word forms: content controls and the older form fields come in with their
//! values, can be filled in, and go back to Word as content controls.

use xenify_wp_doc::Field;
use xenify_wp_doc::controls::ControlKind;
use xenify_wp_io::zip::{ZipReader, ZipWriter};
use xenify_wp_io::{Format, load, save};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

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

const FORM: &str = r#"
<w:p><w:r><w:t xml:space="preserve">Colour: </w:t></w:r><w:sdt><w:sdtPr><w:alias w:val="Colour"/><w:tag w:val="colour"/><w:showingPlcHdr/><w:dropDownList><w:listItem w:displayText="Red" w:value="R"/><w:listItem w:displayText="Blue" w:value="B"/></w:dropDownList></w:sdtPr><w:sdtContent><w:r><w:t>Choose an item.</w:t></w:r></w:sdtContent></w:sdt></w:p>
<w:p><w:r><w:t xml:space="preserve">Agree: </w:t></w:r><w:sdt><w:sdtPr><w14:checkbox><w14:checked w14:val="1"/></w14:checkbox></w:sdtPr><w:sdtContent><w:r><w:t>☒</w:t></w:r></w:sdtContent></w:sdt></w:p>
<w:p><w:r><w:t xml:space="preserve">Signed: </w:t></w:r><w:sdt><w:sdtPr><w:date><w:dateFormat w:val="d/MM/yyyy"/></w:date></w:sdtPr><w:sdtContent><w:r><w:t>14/10/2026</w:t></w:r></w:sdtContent></w:sdt></w:p>
<w:p><w:r><w:t xml:space="preserve">Old box </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Check1"/><w:checkBox><w:default w:val="0"/><w:checked/></w:checkBox></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMCHECKBOX </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Old list </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Size"/><w:ddList><w:result w:val="1"/><w:listEntry w:val="Small"/><w:listEntry w:val="Large"/></w:ddList></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMDROPDOWN </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Old name </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Name"/><w:textInput/></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMTEXT </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Jane</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t xml:space="preserve"> done.</w:t></w:r></w:p>
"#;

fn summary(doc: &xenify_wp_doc::Document) -> Vec<String> {
    doc.paragraphs()
        .map(|p| {
            p.run_ranges()
                .map(|(r, f)| match (&f.field, f.control.and_then(|c| doc.control(c))) {
                    (Some(Field::Check { checked }), _) => format!("[{}]", if *checked { "x" } else { " " }),
                    (_, Some(c)) => {
                        let kind = match &c.kind {
                            ControlKind::Dropdown { items } => format!("list {}", items.join("/")),
                            ControlKind::Date { format } => format!("date {format}"),
                            ControlKind::Text { .. } => "text".into(),
                            ControlKind::ComboBox { .. } => "combo".into(),
                        };
                        format!("<{kind}{}: {}>", if c.placeholder { " prompt" } else { "" }, &p.text[r])
                    }
                    _ => p.text[r].to_string(),
                })
                .collect()
        })
        .collect()
}

#[test]
fn word_forms_keep_their_values_and_go_back_as_controls() {
    let doc = load(&docx(FORM), Format::Docx).unwrap().doc;
    let want = [
        "Colour: <list Red/Blue prompt: Choose an item.>",
        "Agree: [x]",
        "Signed: <date d/MM/yyyy: 14/10/2026>",
        "Old box [x]",
        "Old list <list Small/Large: Large>",
        "Old name <text: Jane> done.",
    ];
    assert_eq!(summary(&doc), want);
    // Back to Word as content controls, and in again the same.
    let out = save(&doc, Format::Docx).unwrap();
    let xml = ZipReader::new(&out).unwrap().read_string("word/document.xml").unwrap();
    assert!(xml.contains("<w:dropDownList><w:listItem w:displayText=\"Red\""), "{xml}");
    assert!(xml.contains("<w14:checked w14:val=\"1\"/>"), "{xml}");
    assert!(xml.contains("<w:showingPlcHdr/>"), "{xml}");
    assert_eq!(summary(&load(&out, Format::Docx).unwrap().doc), want);
}

#[test]
fn what_isnt_kept_is_listed() {
    let body = r#"<w:p><w:r><w:t>Before</w:t></w:r><w:r><w:drawing><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/></w:drawing></w:r></w:p>"#;
    let loaded = load(&docx(body), Format::Docx).unwrap();
    assert_eq!(loaded.warnings, ["Not imported: charts"]);
    // A form and equations alone lose nothing.
    assert!(load(&docx(FORM), Format::Docx).unwrap().warnings.is_empty());
}
