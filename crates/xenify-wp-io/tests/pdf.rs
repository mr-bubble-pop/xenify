use xenify_wp_doc::{CharFmt, Document, Editor, HfLine, Pos, Selection, StyleSheet, ops, tables};
use xenify_wp_io::{Format, save_with};
use xenify_wp_layout::{Engine, Fonts};

fn export(doc: &Document) -> Vec<u8> {
    let mut engine = Engine::new(Fonts::bundled_only());
    let assets = doc.assets.clone();
    save_with(doc, Format::Pdf, &mut engine, &move |s| assets.get(s).cloned()).unwrap()
}

#[test]
fn pdf_has_pages_real_text_and_bookmarks() {
    let mut text = vec!["Report".to_string()];
    for i in 1..=3 {
        text.push(format!("Chapter {i}"));
        for _ in 0..12 {
            text.push("The quick brown fox jumps over the lazy dog, café naïve ﬁnal — ✓ 1,234.".into());
        }
    }
    let mut ed = Editor::new(Document::from_plain_text(&text.join("\n")));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[0], 0));
    ops::set_style(&mut ed, StyleSheet::TITLE).unwrap();
    for i in [1, 14, 27] {
        ed.sel = Selection::caret(Pos::new(ids[i], 0));
        ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    }
    ed.sel = Selection { anchor: Pos::new(ids[2], 4), focus: Pos::new(ids[2], 9) };
    ops::apply_format(&mut ed, &CharFmt { bold: Some(true), ..Default::default() }, "b").unwrap();
    ed.sel = Selection::caret(Pos::new(ids[0], 6));
    tables::insert_table(&mut ed, 2, 2).unwrap();
    ops::insert_text(&mut ed, "Cell text", None).unwrap();
    ed.doc.sections[0].page.footer = HfLine { center: "Page {page} of {pages}".into(), ..Default::default() };
    ed.doc.meta.title = "Test report".into();

    let bytes = export(&ed.doc);
    assert!(bytes.starts_with(b"%PDF-"));
    let pdf = lopdf::Document::load_mem(&bytes).expect("a valid PDF");
    let pages = pdf.get_pages();
    assert!(pages.len() >= 2, "{} pages", pages.len());
    let all = pdf.extract_text(&pages.keys().copied().collect::<Vec<_>>()).unwrap();
    let squashed: String = all.split_whitespace().collect();
    for want in ["Report", "Chapter1", "quickbrownfox", "café", "naïve", "ﬁnal", "Celltext", "Page1of"] {
        let w: String = want.split_whitespace().collect();
        let w = w.replace("ﬁ", "fi");
        assert!(squashed.contains(&w) || squashed.contains(want), "{want:?} not in extracted text:\n{all}");
    }
    // Fonts are embedded, bookmarks exist.
    let catalog = pdf.catalog().unwrap();
    assert!(catalog.get(b"Outlines").is_ok(), "bookmarks");
    let embedded = pdf.objects.values().filter(|o| o.as_dict().is_ok_and(|d| d.has(b"FontFile2"))).count();
    assert!(embedded >= 2, "regular and bold Inter embedded: {embedded}");
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xenify-write-sample.pdf");
    std::fs::write(out, &bytes).unwrap();
}

#[test]
fn opentype_cff_fonts_embed_as_font_file3() {
    // CMU Serif (bundled) has CFF outlines: embedded as /FontFile3 /OpenType, text still extractable.
    let mut ed = Editor::new(Document::from_plain_text("A paper in the classic look.\nWith italics."));
    ed.doc.styles.default_font = xenify_wp_layout::SERIF_FAMILY.into();
    let id = ed.doc.paragraphs().nth(1).unwrap().id;
    ed.sel = Selection { anchor: Pos::new(id, 5), focus: Pos::new(id, 13) };
    ops::apply_format(&mut ed, &CharFmt { italic: Some(true), ..Default::default() }, "i").unwrap();
    let bytes = export(&ed.doc);
    let pdf = lopdf::Document::load_mem(&bytes).expect("a valid PDF");
    let file3 = pdf.objects.values().filter(|o| o.as_dict().is_ok_and(|d| d.has(b"FontFile3"))).count();
    assert!(file3 >= 2, "roman and italic CMU embedded as FontFile3: {file3}");
    let cid0 = pdf
        .objects
        .values()
        .filter(|o| {
            o.as_dict().is_ok_and(|d| d.get(b"Subtype").is_ok_and(|s| s.as_name().is_ok_and(|n| n == b"CIDFontType0")))
        })
        .count();
    assert!(cid0 >= 2);
    let pages = pdf.get_pages();
    let all = pdf.extract_text(&pages.keys().copied().collect::<Vec<_>>()).unwrap();
    let squashed: String = all.split_whitespace().collect();
    assert!(squashed.contains("classiclook") && squashed.contains("italics"), "{all}");
}
