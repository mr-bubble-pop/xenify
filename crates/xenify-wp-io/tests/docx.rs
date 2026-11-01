use xenify_wp_doc::{
    Align, Block, CharFmt, Document, Editor, HfLine, ListDef, ParaProps, Pos, Rgba, SectionStart, Selection,
    StyleSheet, ops, tables,
};
use xenify_wp_io::{Format, load, save, zip::ZipReader};

fn build() -> Document {
    let mut ed = Editor::new(Document::from_plain_text(
        "Annual report\nIntroduction\nPlain, bold and italic text with a link.\nFirst\nSecond\nNested\nStep\nCentred\nSecond section in two columns",
    ));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    let at = |ed: &mut Editor, i: usize| {
        let len = ed.doc.para(ids[i]).unwrap().text.len();
        ed.sel = Selection { anchor: Pos::new(ids[i], 0), focus: Pos::new(ids[i], len) };
    };
    at(&mut ed, 0);
    ops::set_style(&mut ed, StyleSheet::TITLE).unwrap();
    at(&mut ed, 1);
    ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    let t = ed.doc.para(ids[2]).unwrap().text.clone();
    for (w, f) in [
        ("bold", CharFmt { bold: Some(true), ..Default::default() }),
        ("italic", CharFmt { italic: Some(true), color: Some(Rgba::rgb(0xcc, 0, 0)), ..Default::default() }),
        ("link", CharFmt { link: Some("https://example.com/".into()), ..Default::default() }),
    ] {
        let i = t.find(w).unwrap();
        ed.sel = Selection { anchor: Pos::new(ids[2], i), focus: Pos::new(ids[2], i + w.len()) };
        ops::apply_format(&mut ed, &f, "f").unwrap();
    }
    for i in 3..=5 {
        at(&mut ed, i);
        ops::toggle_list(&mut ed, ListDef::BULLETS).unwrap();
    }
    at(&mut ed, 5);
    ops::change_list_level(&mut ed, 1).unwrap();
    at(&mut ed, 6);
    ops::toggle_list(&mut ed, ListDef::NUMBERS).unwrap();
    at(&mut ed, 7);
    ops::apply_para_props(
        &mut ed,
        &ParaProps { align: Some(Align::Center), space_after: Some(20.0), ..Default::default() },
        "p",
    )
    .unwrap();
    // A table with a merged header cell and shading.
    ed.sel = Selection::caret(Pos::new(ids[7], 7));
    tables::insert_table(&mut ed, 3, 3).unwrap();
    for w in ["H1", "H2", "H3", "a", "b", "c", "d", "e", "f"] {
        ops::insert_text(&mut ed, w, None).unwrap();
        tables::move_cell(&mut ed, true).unwrap();
    }
    ed.undo();
    let tid =
        ed.doc.sections[0].blocks.iter().find_map(|b| if let Block::Table(t) = b { Some(t.id) } else { None }).unwrap();
    let t = ed.doc.table(tid).unwrap().clone();
    ed.sel = Selection {
        anchor: Pos::new(tables::first_para(&t, 1, 0).unwrap(), 0),
        focus: Pos::new(tables::first_para(&t, 2, 0).unwrap(), 0),
    };
    tables::merge_cells(&mut ed).unwrap();
    ed.sel = Selection::caret(Pos::new(tables::first_para(&t, 0, 0).unwrap(), 0));
    tables::toggle_header_rows(&mut ed).unwrap();
    tables::set_shading(&mut ed, Some(Rgba::rgb(0xee, 0xee, 0xee))).unwrap();
    // A picture.
    let png = {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    };
    ed.doc.assets.insert("media/dot.png".into(), std::sync::Arc::new(png));
    ed.sel = Selection::caret(Pos::new(ids[7], 0));
    tables::insert_image(&mut ed, "media/dot.png", 60.0, 60.0).unwrap();
    // A second section: continuous, two columns, with a footer.
    ed.sel = Selection::caret(Pos::new(ids[8], 0));
    ops::insert_section_break(&mut ed, SectionStart::Continuous).unwrap();
    let mut page = ed.doc.sections[1].page.clone();
    page.columns = 2;
    ops::set_section(&mut ed, 1, SectionStart::Continuous, page).unwrap();
    ed.doc.sections[0].page.footer = HfLine { center: "Page {page} of {pages}".into(), ..Default::default() };
    ed.doc.sections[0].page.header = HfLine { left: "Report".into(), right: "{date}".into(), ..Default::default() };
    ed.doc.meta.title = "Annual report".into();
    ed.doc
}

fn shape(d: &Document) -> Vec<String> {
    let labels = xenify_wp_doc::lists::labels(d);
    let mut v: Vec<String> = d
        .paragraphs()
        .map(|p| {
            let r = d.resolve_para(p);
            format!(
                "{}|{}|{:?}|{:?}|{}|{}",
                p.style,
                p.text,
                p.runs
                    .iter()
                    .map(|r| (r.len, r.fmt.bold, r.fmt.italic, r.fmt.color, r.fmt.link.clone()))
                    .collect::<Vec<_>>(),
                r.align,
                r.space_after,
                labels.get(&p.id).cloned().unwrap_or_default()
            )
        })
        .collect();
    for s in &d.sections {
        v.push(format!(
            "section {:?} cols={} {}x{} m={:?} h={:?} f={:?}",
            s.start, s.page.columns, s.page.width, s.page.height, s.page.margins, s.page.header, s.page.footer
        ));
        for b in &s.blocks {
            match b {
                Block::Table(t) => v.push(format!(
                    "table {}x{} header={} span={}x{} shade={:?}",
                    t.rows.len(),
                    t.cols(),
                    t.rows[0].header,
                    t.rows[1].cells[0].row_span,
                    t.rows[1].cells[0].col_span,
                    t.rows[0].cells[0].shading
                )),
                Block::Image(i) => v.push(format!(
                    "image {}x{} bytes={}",
                    i.width,
                    i.height,
                    d.assets.get(&i.src).map_or(0, |b| b.len())
                )),
                _ => {}
            }
        }
    }
    v.push(d.meta.title.clone());
    v
}

#[test]
fn docx_round_trip() {
    let doc = build();
    let bytes = save(&doc, Format::Docx).unwrap();
    // Every XML part is well-formed.
    let zip = ZipReader::new(&bytes).unwrap();
    for name in zip.names().filter(|n| n.ends_with(".xml") || n.ends_with(".rels")) {
        let xml = zip.read_string(name).unwrap();
        roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name}: {e}\n{xml}"));
    }
    let back = load(&bytes, Format::Docx).unwrap();
    let (a, b) = (shape(&doc), shape(&back.doc));
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x, y, "\n{a:#?}\n{b:#?}");
    }
    assert_eq!(a.len(), b.len(), "{a:#?}\n{b:#?}");
    // Writing what was read gives the same document again.
    let again = load(&save(&back.doc, Format::Docx).unwrap(), Format::Docx).unwrap();
    assert_eq!(shape(&again.doc), b);
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xenify-write-sample.docx");
    std::fs::write(out, &bytes).unwrap();
}

#[test]
fn header_rows_repeat_in_word_only_when_asked_and_footers_come_back() {
    use xenify_wp_doc::{Editor, tables};
    let mut ed = Editor::new(xenify_wp_doc::Document::new());
    tables::insert_table(&mut ed, 3, 2).unwrap();
    let id = ed.doc.sections[0]
        .blocks
        .iter()
        .find_map(|b| if let xenify_wp_doc::Block::Table(t) = b { Some(t.id) } else { None })
        .unwrap();
    tables::set_headers(&mut ed, id, true, false).unwrap();
    tables::set_footer(&mut ed, id, true).unwrap();
    let back = |doc: &xenify_wp_doc::Document| {
        let bytes = xenify_wp_io::save(doc, xenify_wp_io::Format::Docx).unwrap();
        let d = xenify_wp_io::load(&bytes, xenify_wp_io::Format::Docx).unwrap().doc;
        d.sections[0]
            .blocks
            .iter()
            .find_map(|b| if let xenify_wp_doc::Block::Table(t) = b { Some(t.clone()) } else { None })
            .unwrap()
    };
    let t = back(&ed.doc);
    assert!(t.rows[0].header && !t.repeat_header && t.rows[2].footer);
    tables::set_repeats(&mut ed, id, Some(true), None).unwrap();
    assert!(back(&ed.doc).repeat_header);
}
