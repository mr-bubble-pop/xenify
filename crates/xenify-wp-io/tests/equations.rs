//! Equations from Word: kept as LaTeX, shown, and written back to Word as
//! Word's own equation markup.

use xenify_wp_doc::{Align, Field};
use xenify_wp_io::zip::{ZipReader, ZipWriter};
use xenify_wp_io::{Format, load, save};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

const INLINE: &str =
    r#"<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>"#;
const DISPLAY: &str = r#"<m:oMathPara><m:oMath><m:sSup><m:e><m:r><m:t>E</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath></m:oMathPara>"#;

fn docx() -> Vec<u8> {
    let body = format!(
        r#"<w:p><w:r><w:t xml:space="preserve">Half is </w:t></w:r>{INLINE}<w:r><w:t xml:space="preserve"> here.</w:t></w:r></w:p><w:p>{DISPLAY}</w:p>"#
    );
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

#[test]
fn word_equations_come_in_and_go_back_out() {
    let doc = load(&docx(), Format::Docx).unwrap().doc;
    let eqs: Vec<(String, bool)> = doc
        .paragraphs()
        .flat_map(|p| p.runs.iter().filter_map(|r| r.fmt.field.clone()))
        .filter_map(|f| match f {
            Field::Equation { latex, display, omml } => {
                assert!(omml.starts_with("<m:oMath"), "Word's markup kept");
                Some((latex, display))
            }
            _ => None,
        })
        .collect();
    assert_eq!(eqs, [("\\frac{a}{b}".to_string(), false), ("{E}^{2}".to_string(), true)]);
    // The display equation's line is centred; the text around the inline one stays.
    let paras: Vec<_> = doc.paragraphs().collect();
    assert_eq!(paras[0].text, "Half is \u{FFFC} here.");
    assert_eq!(paras[1].props.align, Some(Align::Center));
    // Back to Word: the same markup.
    let out = save(&doc, Format::Docx).unwrap();
    let xml = ZipReader::new(&out).unwrap().read_string("word/document.xml").unwrap();
    assert!(xml.contains(INLINE), "{xml}");
    assert!(xml.contains(DISPLAY), "{xml}");
    // To Markdown as TeX maths, and back.
    let md = String::from_utf8(save(&doc, Format::Markdown).unwrap()).unwrap();
    assert!(md.contains("Half is $\\frac{a}{b}$ here."), "{md}");
    assert!(md.contains("$${E}^{2}$$"), "{md}");
    let again = load(md.as_bytes(), Format::Markdown).unwrap().doc;
    let n = again
        .paragraphs()
        .flat_map(|p| &p.runs)
        .filter(|r| matches!(r.fmt.field, Some(Field::Equation { .. })))
        .count();
    assert_eq!(n, 2);
}

#[test]
fn dollar_signs_in_text_stay_text() {
    let doc = load(b"It costs $5 and $6 today.", Format::Markdown).unwrap().doc;
    let md = String::from_utf8(save(&doc, Format::Markdown).unwrap()).unwrap();
    let back = load(md.as_bytes(), Format::Markdown).unwrap().doc;
    assert_eq!(back.paragraphs().next().unwrap().text, "It costs $5 and $6 today.");
}
