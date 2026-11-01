use xenify_wp_doc::{
    Align, CharFmt, Document, Editor, ListDef, PageSetup, ParaProps, Pos, Rgba, Selection, StyleSheet, ops,
};
use xenify_wp_io::{Format, load, save};

fn md(doc: &Document) -> String {
    String::from_utf8(save(doc, Format::Markdown).unwrap()).unwrap()
}

fn read(s: &str) -> Document {
    load(s.as_bytes(), Format::Markdown).unwrap().doc
}

/// Everything that matters about a document, ids aside.
fn shape(d: &Document) -> Vec<String> {
    let labels = xenify_wp_doc::lists::labels(d);
    let mut v: Vec<String> = d
        .paragraphs()
        .map(|p| {
            format!(
                "{} | {:?} | {:?} | {:?} | {:?}",
                p.style,
                p.text,
                p.runs.iter().map(|r| (r.len, r.fmt.clone())).collect::<Vec<_>>(),
                p.props,
                (p.list.as_ref().map(|l| (&l.list, l.level)), labels.get(&p.id)),
            )
        })
        .collect();
    v.push(format!("{:?}", d.meta));
    v.push(format!("{:?}", d.sections[0].page));
    v.push(format!("{:?}", d.styles));
    v
}

fn rich() -> Document {
    let text = "Report title\nA subtitle\nIntroduction\nPlain, bold, italic, bold italic, struck, underlined, red, big, highlighted, code, a link, and *stars* _unders_ [brackets] #hash.\nA quote to remember.\nFirst point\nSecond point\nNested point\nStep one\nStep two\nlet x = 1;\nlet y = `2`;\nCentred and spaced\n\nThe end.";
    let mut ed = Editor::new(Document::from_plain_text(text));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    let para = |ed: &mut Editor, i: usize| {
        let len = ed.doc.para(ids[i]).unwrap().text.len();
        ed.sel = Selection { anchor: Pos::new(ids[i], 0), focus: Pos::new(ids[i], len) };
    };
    for (i, s) in [
        (0, StyleSheet::TITLE),
        (1, StyleSheet::SUBTITLE),
        (2, "heading2"),
        (4, StyleSheet::QUOTE),
        (10, StyleSheet::CODE),
        (11, StyleSheet::CODE),
    ] {
        para(&mut ed, i);
        ops::set_style(&mut ed, s).unwrap();
    }
    let body = ed.doc.para(ids[3]).unwrap().text.clone();
    let mark = |ed: &mut Editor, word: &str, f: CharFmt| {
        let at = body.find(word).unwrap();
        ed.sel = Selection { anchor: Pos::new(ids[3], at), focus: Pos::new(ids[3], at + word.len()) };
        ops::apply_format(ed, &f, "f").unwrap();
    };
    mark(&mut ed, "bold,", CharFmt { bold: Some(true), ..Default::default() });
    mark(&mut ed, "italic,", CharFmt { italic: Some(true), ..Default::default() });
    mark(&mut ed, "bold italic", CharFmt { bold: Some(true), italic: Some(true), ..Default::default() });
    mark(&mut ed, "struck", CharFmt { strike: Some(true), ..Default::default() });
    mark(&mut ed, "underlined", CharFmt { underline: Some(true), ..Default::default() });
    mark(&mut ed, "red", CharFmt { color: Some(Rgba::rgb(0xcc, 0, 0)), ..Default::default() });
    mark(&mut ed, "big", CharFmt { size: Some(18.0), bold: Some(true), ..Default::default() });
    mark(&mut ed, "highlighted", CharFmt { highlight: Some(Rgba([255, 235, 59, 255])), ..Default::default() });
    mark(&mut ed, "code", CharFmt { font: Some(xenify_wp_io::markdown::MONO.into()), ..Default::default() });
    mark(&mut ed, "a link", CharFmt { link: Some("https://example.com/a b".into()), ..Default::default() });
    for i in 5..=7 {
        para(&mut ed, i);
        ops::toggle_list(&mut ed, ListDef::BULLETS).unwrap();
    }
    para(&mut ed, 7);
    ops::change_list_level(&mut ed, 1).unwrap();
    for i in 8..=9 {
        para(&mut ed, i);
        ops::toggle_list(&mut ed, ListDef::NUMBERS).unwrap();
    }
    para(&mut ed, 12);
    ops::apply_para_props(
        &mut ed,
        &ParaProps { align: Some(Align::Center), space_after: Some(20.0), ..Default::default() },
        "p",
    )
    .unwrap();
    // A changed built-in style, a new style, document settings.
    ed.doc.styles.get_mut(StyleSheet::NORMAL).unwrap().char.font = Some("Source Serif 4".into());
    ed.doc.styles.paragraph.push(xenify_wp_doc::ParaStyle {
        id: "note".into(),
        name: "Side Note".into(),
        based_on: Some("normal".into()),
        char: CharFmt { italic: Some(true), color: Some(Rgba::rgb(0, 0x55, 0xaa)), ..Default::default() },
        ..Default::default()
    });
    para(&mut ed, 14);
    ops::set_style(&mut ed, "note").unwrap();
    // No title: Markdown files take their title from the file name.
    ed.doc.meta.audience = "board members".into();
    let (w, h) = PageSetup::LETTER;
    ed.doc.sections[0].page.width = h;
    ed.doc.sections[0].page.height = w;
    ed.doc.sections[0].page.margins.left = 54.0;
    ed.doc.sections[0].page.columns = 2;
    ed.doc.sections[0].page.footer =
        xenify_wp_doc::HfLine { center: "Page {page} of {pages}".into(), ..Default::default() };
    ed.doc.sections[0].page.different_first = true;
    ed.doc.sections[0].page.start_number = Some(3);
    ed.doc.meta.pageless = true;
    ed.doc
}

#[test]
fn markdown_round_trips_the_model() {
    let doc = rich();
    let text = md(&doc);
    let back = read(&text);
    let (a, b) = (shape(&doc), shape(&back));
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x, y, "\n--- markdown ---\n{text}");
    }
    assert_eq!(a.len(), b.len(), "\n{text}");
    // And writing again gives the same text.
    assert_eq!(md(&back), text);
}

#[test]
fn reads_ordinary_markdown() {
    let doc = read(
        "# Title\n\nSome *emphasis* and **strong** text\nwrapped over lines.\n\n* one\n* two\n  * two a\n\n3. three\n4. four\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n- [x] done\n- [ ] todo\n",
    );
    let paras: Vec<_> = doc.paragraphs().map(|p| (p.style.clone(), p.text.clone())).collect();
    assert_eq!(paras[0], ("heading1".into(), "Title".into()));
    assert_eq!(paras[1].1, "Some emphasis and strong text wrapped over lines.");
    let labels = xenify_wp_doc::lists::labels(&doc);
    let lab: Vec<_> = doc.paragraphs().map(|p| labels.get(&p.id).cloned().unwrap_or_default()).collect();
    assert_eq!(&lab[2..7], ["•", "•", "◦", "3.", "4."]);
    assert_eq!(paras[7], ("quote".into(), "quoted".into()));
    assert_eq!(paras[8], ("code".into(), "fn main() {}".into()));
    // Task lists are checklists.
    assert_eq!(paras[9].1, "done");
    assert_eq!(&lab[9..11], [xenify_wp_doc::lists::CHECKED, xenify_wp_doc::lists::UNCHECKED]);
}

#[test]
fn separate_numbered_lists_restart() {
    let doc = read("1. a\n2. b\n\npara\n\n1. c\n2. d\n");
    let labels = xenify_wp_doc::lists::labels(&doc);
    let lab: Vec<_> = doc.paragraphs().filter_map(|p| labels.get(&p.id).cloned()).collect();
    assert_eq!(lab, ["1.", "2.", "1.", "2."]);
}

#[test]
fn front_matter_is_only_written_when_needed() {
    let plain = md(&Document::from_plain_text("just text"));
    assert_eq!(plain, "just text\n");
    assert!(md(&rich()).starts_with("---\n"));
}

#[test]
fn tables_images_and_contents_round_trip() {
    let mut ed = Editor::new(Document::from_plain_text("Intro\nChapter\nafter"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::set_style(&mut ed, "heading1").unwrap();
    ed.sel = Selection::caret(Pos::new(ids[0], 5));
    xenify_wp_doc::tables::insert_toc(&mut ed, 2).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[1], 7));
    xenify_wp_doc::tables::insert_table(&mut ed, 2, 3).unwrap();
    for w in ["Name", "Qty", "Price | total", "*Apple*", "3", "1.50"] {
        ops::insert_text(&mut ed, w, None).unwrap();
        xenify_wp_doc::tables::move_cell(&mut ed, true).unwrap();
    }
    // Undo the row Tab added at the end.
    ed.undo();
    xenify_wp_doc::tables::toggle_header_rows(&mut ed).ok();
    ed.sel = Selection::caret(Pos::new(ids[2], 5));
    xenify_wp_doc::tables::insert_image(&mut ed, "media/logo.png", 120.0, 60.0).unwrap();
    let text = md(&ed.doc);
    let back = read(&text);
    let kinds = |d: &Document| {
        d.sections[0]
            .blocks
            .iter()
            .map(|b| match b {
                xenify_wp_doc::Block::Paragraph(p) => format!("p:{}:{}", p.style, p.text),
                xenify_wp_doc::Block::Table(t) => format!("t:{}x{}:{}", t.rows.len(), t.cols(), t.rows[0].header),
                xenify_wp_doc::Block::Image(i) => format!("i:{}:{}x{}", i.src, i.width, i.height),
                xenify_wp_doc::Block::Toc(t) => format!("toc:{}", t.levels),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(&back), kinds(&ed.doc), "\n{text}");
    assert_eq!(back.plain_text(), ed.doc.plain_text(), "\n{text}");
    assert_eq!(md(&back), text);
}

#[test]
fn proofing_choices_round_trip() {
    let mut doc = Document::from_plain_text("Teh text");
    doc.meta.proofing_profile = Some("workplace-email".into());
    doc.meta.ignore_words = vec!["Teh".into(), "Okonkwo".into()];
    doc.meta.ignore_issues = vec!["Rule:word:abc123".into()];
    let text = md(&doc);
    let back = read(&text);
    assert_eq!(back.meta.proofing_profile.as_deref(), Some("workplace-email"), "{text}");
    assert_eq!(back.meta.ignore_words, doc.meta.ignore_words);
    assert_eq!(back.meta.ignore_issues, doc.meta.ignore_issues);
}

#[test]
fn dividers_are_lines_in_markdown() {
    let mut ed = Editor::new(Document::from_plain_text("Above"));
    let end = ed.doc.paragraphs().next().unwrap().text.len();
    ed.sel = Selection::caret(Pos::new(ed.doc.paragraphs().next().unwrap().id, end));
    ops::insert_divider(&mut ed).unwrap();
    ops::insert_text(&mut ed, "Below", None).unwrap();
    let out = md(&ed.doc);
    assert!(out.contains("Above\n\n---\n\nBelow"), "{out}");
    let back = read(&out);
    let paras: Vec<_> = back.paragraphs().collect();
    assert_eq!(paras.len(), 3);
    assert!(ops::is_divider(paras[1]));
    assert_eq!(md(&back), out);
}
