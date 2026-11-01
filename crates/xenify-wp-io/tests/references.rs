//! Footnotes, endnotes, fields, citations, sources and the bibliography
//! survive DOCX, Markdown and Xenify's own format, and show in HTML.
use xenify_wp_doc::cite::{self, CiteStyle, Name, Source, SourceKind};
use xenify_wp_doc::{Document, Editor, Field, NoteKind, Pos, Selection, TocKind, notes, ops, tables};
use xenify_wp_io::{Format, load, save};

fn build() -> Document {
    let mut ed = Editor::new(Document::from_plain_text(
        "Introduction\nCommas matter in prose.\nSee the introduction for more.\nReferences",
    ));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[0], 0));
    ops::set_style(&mut ed, "heading1").unwrap();
    cite::set_sources(
        &mut ed,
        vec![
            Source {
                key: "smith2020".into(),
                kind: SourceKind::Book,
                authors: vec![Name { family: "Smith".into(), given: "Jane".into() }],
                title: "Writing well".into(),
                publisher: "Harbour Press".into(),
                year: "2020".into(),
                ..Default::default()
            },
            Source {
                key: "lee2018".into(),
                kind: SourceKind::Article,
                authors: vec![Name { family: "Lee".into(), given: "Min".into() }],
                title: "Commas and clarity".into(),
                container: "Journal of Prose".into(),
                volume: "12".into(),
                pages: "45–67".into(),
                year: "2018".into(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    cite::set_style(&mut ed, CiteStyle::Harvard).unwrap();
    // "Commas matter[1] in prose (Smith 2020, p. 4; Lee 2018)."
    ed.sel = Selection::caret(Pos::new(ids[1], "Commas matter".len()));
    notes::insert_note(&mut ed, NoteKind::Foot).unwrap();
    ops::insert_text(&mut ed, "Mostly.", None).unwrap();
    let end_of = |ed: &Editor, id| ed.doc.para(id).unwrap().text.len() - 1;
    ed.sel = Selection::caret(Pos::new(ids[1], end_of(&ed, ids[1])));
    ops::insert_text(&mut ed, " ", None).unwrap();
    cite::insert_citation(&mut ed, vec!["smith2020".into(), "lee2018".into()], "p. 4".into()).unwrap();
    // An endnote and a cross-reference to the heading.
    ed.sel = Selection::caret(Pos::new(ids[2], "See ".len()));
    ops::insert_field(&mut ed, Field::Ref { target: ids[0] }, "Cross-reference").unwrap();
    notes::insert_note(&mut ed, NoteKind::End).unwrap();
    ops::insert_text(&mut ed, "An aside.", None).unwrap();
    // Page number and title fields, and a bibliography at the end.
    ed.sel = Selection::caret(Pos::new(ids[3], ed.doc.para(ids[3]).unwrap().text.len()));
    ops::insert_text(&mut ed, " p. ", None).unwrap();
    ops::insert_field(&mut ed, Field::Page, "Field").unwrap();
    tables::insert_generated(&mut ed, TocKind::Bibliography).unwrap();
    ed.doc
}

/// What matters: paragraphs with their fields spelt out, notes' text, sources, style, generated blocks.
fn shape(d: &Document) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in &d.sections {
        for b in &s.blocks {
            match b {
                xenify_wp_doc::Block::Paragraph(p) => {
                    let mut t = String::new();
                    for (range, fmt) in p.run_ranges() {
                        match &fmt.field {
                            Some(Field::Note { id }) => {
                                let n = d.note(*id).unwrap();
                                let text: Vec<String> = n
                                    .blocks
                                    .iter()
                                    .filter_map(|b| match b {
                                        xenify_wp_doc::Block::Paragraph(p) => Some(p.text.clone()),
                                        _ => None,
                                    })
                                    .collect();
                                t.push_str(&format!("<{:?} {}>", n.kind, text.join("/")));
                            }
                            Some(Field::Ref { target }) => {
                                t.push_str(&format!("<ref {}>", d.para(*target).map_or("?", |p| p.text.as_str())))
                            }
                            Some(f) => t.push_str(&format!("<{f:?}>")),
                            None => t.push_str(&p.text[range]),
                        }
                    }
                    out.push(format!("{} | {}", p.style, t));
                }
                xenify_wp_doc::Block::Toc(t) => out.push(format!("generated {:?}", t.kind)),
                _ => {}
            }
        }
    }
    out.push(format!("{:?}", d.meta.cite_style));
    for s in &d.meta.sources {
        out.push(format!(
            "{} {:?} {:?} {} {} {} {} {}",
            s.key, s.kind, s.authors, s.title, s.container, s.year, s.volume, s.pages
        ));
    }
    out
}

#[test]
fn notes_fields_and_citations_round_trip() {
    let doc = build();
    let want = shape(&doc);
    assert!(want.iter().any(|l| l.contains("<Foot Mostly.>")), "{want:#?}");
    for format in [Format::Igdoc, Format::Docx, Format::Markdown] {
        let bytes = save(&doc, format).unwrap();
        let back = load(&bytes, format).unwrap();
        assert_eq!(
            shape(&back.doc),
            want,
            "{format:?}\n{}",
            if format == Format::Markdown { String::from_utf8_lossy(&bytes).to_string() } else { String::new() }
        );
        assert!(!back.warnings.iter().any(|w| w.contains("footnote")), "{format:?}: {:?}", back.warnings);
    }
}

#[test]
fn markdown_uses_pandoc_syntax() {
    let md = String::from_utf8(save(&build(), Format::Markdown).unwrap()).unwrap();
    assert!(md.contains("Commas matter[^1] in prose [@smith2020; @lee2018, p. 4]"), "{md}");
    assert!(md.contains("[^1]: Mostly."), "{md}");
    assert!(md.contains("[^end-i]: An aside."), "{md}");
    assert!(md.contains("references:") && md.contains("id: smith2020"), "{md}");
    // Pandoc citations and footnotes written by hand are read.
    let back = load(b"Text [@smith2020, pp. 3-4] and a note[^n].\n\n[^n]: The note.\n", Format::Markdown).unwrap();
    let p = back.doc.paragraphs().next().unwrap();
    let fields: Vec<_> = p.runs.iter().filter_map(|r| r.fmt.field.clone()).collect();
    assert_eq!(fields.len(), 2, "{fields:?}");
    assert!(matches!(&fields[0], Field::Citation { keys, locator } if keys == &["smith2020"] && locator == "pp. 3-4"));
    assert_eq!(back.doc.sections[0].notes.len(), 1);
}

#[test]
fn html_shows_notes_citations_and_bibliography() {
    let html = String::from_utf8(save(&build(), Format::Html).unwrap()).unwrap();
    assert!(html.contains("<cite>(Smith 2020; Lee 2018, p. 4)</cite>"), "{html}");
    assert!(html.contains("class=\"footnotes\"") && html.contains("Mostly."));
    assert!(html.contains("class=\"endnotes\"") && html.contains("An aside.") && !html.contains(">Notes<"));
    assert!(html.contains("Smith, J. (2020) <i>Writing well</i>"), "{html}");
}

#[test]
fn docx_writes_word_notes_and_sources() {
    let bytes = save(&build(), Format::Docx).unwrap();
    let z = String::from_utf8_lossy(&bytes).to_string();
    for part in ["word/footnotes.xml", "word/endnotes.xml", "customXml/item1.xml"] {
        assert!(z.contains(part), "{part} missing");
    }
}

#[test]
fn odd_even_headers_and_page_number_formats_round_trip() {
    use xenify_wp_doc::{HfLine, PageNumberFormat, SectionStart};
    let mut ed = Editor::new(Document::from_plain_text("Front matter\nChapter one"));
    let second = ed.doc.paragraphs().nth(1).unwrap().id;
    ed.sel = Selection::caret(Pos::new(second, 0));
    ops::insert_section_break(&mut ed, SectionStart::NewPage).unwrap();
    {
        let p = &mut ed.doc.sections[0].page;
        p.header = HfLine { right: "Odd {page}".into(), ..Default::default() };
        p.different_odd_even = true;
        p.even_header = HfLine { left: "Even {page}".into(), ..Default::default() };
        p.even_footer = HfLine { center: "{title}".into(), ..Default::default() };
        p.number_format = PageNumberFormat::LowerRoman;
    }
    ed.doc.sections[1].page.link_to_previous = true;
    ed.doc.sections[1].page.start_number = Some(1);
    let doc = ed.doc;
    let key = |p: &xenify_wp_doc::PageSetup| {
        (p.header.clone(), p.different_odd_even, p.even_header.clone(), p.even_footer.clone(), p.number_format)
    };
    let back = load(&save(&doc, Format::Docx).unwrap(), Format::Docx).unwrap().doc;
    assert_eq!(key(&back.sections[0].page), key(&doc.sections[0].page));
    assert!(back.sections[1].page.link_to_previous);
    assert_eq!(back.sections[1].page.start_number, Some(1));
    let back = load(&save(&doc, Format::Igdoc).unwrap(), Format::Igdoc).unwrap().doc;
    assert_eq!(back.sections[1].page, doc.sections[1].page);
    let back = load(&save(&doc, Format::Markdown).unwrap(), Format::Markdown).unwrap().doc;
    assert_eq!(key(&back.sections[0].page), key(&doc.sections[0].page));
}
