//! Files made to crash a reader (deep nesting, zip tricks) are read or
//! refused without running out of stack or memory.

use xenify_wp_io::zip::ZipWriter;
use xenify_wp_io::{Format, load};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

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

fn nested(open: &str, inner: &str, close: &str, n: usize) -> String {
    let mut s = String::with_capacity((open.len() + close.len()) * n + inner.len());
    for _ in 0..n {
        s.push_str(open);
    }
    s.push_str(inner);
    for _ in 0..n {
        s.push_str(close);
    }
    s
}

const P: &str = "<w:p><w:r><w:t>deep</w:t></w:r></w:p>";

#[test]
fn deep_custom_xml_in_word() {
    let body = format!("<w:p><w:r><w:t>top</w:t></w:r></w:p>{}", nested("<w:customXml>", P, "</w:customXml>", 200_000));
    // Whatever the XML reader makes of it, no crash.
    let _ = load(&docx(&body), Format::Docx);
    // Past the limit on nesting blocks (64), within the XML one: the top is kept.
    let body = format!("<w:p><w:r><w:t>top</w:t></w:r></w:p>{}", nested("<w:customXml>", P, "</w:customXml>", 90));
    let doc = load(&docx(&body), Format::Docx).unwrap().doc;
    assert!(doc.paragraphs().any(|p| p.text == "top"));
    // A few levels in a table cell, as real files have: kept.
    let body = format!("<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>", nested("<w:customXml>", P, "</w:customXml>", 5));
    let doc = load(&docx(&body), Format::Docx).unwrap().doc;
    assert!(doc.plain_text().contains("deep"));
}

#[test]
fn deep_runs_tables_and_maths_in_word() {
    let runs = format!("<w:p>{}</w:p>", nested("<w:smartTag>", "<w:r><w:t>x</w:t></w:r>", "</w:smartTag>", 20_000));
    let _ = load(&docx(&runs), Format::Docx);
    let tables = nested("<w:tbl><w:tr><w:tc>", P, "</w:tc></w:tr></w:tbl>", 5_000);
    let _ = load(&docx(&tables), Format::Docx);
    let maths = format!("<w:p><m:oMath>{}</m:oMath></w:p>", nested("<m:d><m:e>", "<m:r><m:t>x</m:t></m:r>", "</m:e></m:d>", 20_000));
    let _ = load(&docx(&maths), Format::Docx);
}

#[test]
fn deep_html() {
    for (open, close) in [
        ("<div>", "</div>"),
        ("<blockquote>", "</blockquote>"),
        ("<ul><li>", "</li></ul>"),
        ("<table><tr><td>", "</td></tr></table>"),
        ("<b><i>", "</i></b>"),
    ] {
        let html = nested(open, "deep", close, 50_000);
        let _ = load(html.as_bytes(), Format::Html);
    }
}

#[test]
fn deep_markdown() {
    for (open, close) in [(">", ""), ("- ", ""), ("*", "*"), ("[", "](x)"), ("<div>\n\n", "\n\n</div>\n\n")] {
        let md = nested(open, "deep", close, 50_000);
        let _ = load(md.as_bytes(), Format::Markdown);
    }
    let mut md = String::new();
    for i in 0..2_000 {
        md.push_str(&" ".repeat(i * 2));
        md.push_str("- item\n");
    }
    let _ = load(md.as_bytes(), Format::Markdown);
    // Front matter nested deeply.
    let front = format!("---\ntitle: {}\n---\nText\n", nested("[", "1", "]", 50_000));
    let _ = load(front.as_bytes(), Format::Markdown);
}
