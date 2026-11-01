use xenify_wp_doc::{Document, Editor, HfLine, ParaProps, Pos, SectionStart, Selection, StyleSheet, ops};
use xenify_wp_layout::{DocLayout, Engine, Fonts};

const PARA: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut \
                    labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco \
                    laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit.";

fn long_doc(n: usize) -> Document {
    Document::from_plain_text(&vec![PARA; n].join("\n"))
}

fn layout(doc: &Document) -> DocLayout {
    Engine::new(Fonts::bundled_only()).layout(doc)
}

fn page_of(l: &DocLayout, r: kurbo::Rect) -> usize {
    l.pages.iter().position(|p| p.rect.contains(r.center())).expect("on a page")
}

/// Every line of every paragraph is placed exactly once, in order, inside a page body.
fn check_complete(doc: &Document, l: &DocLayout) {
    for p in doc.paragraphs() {
        let frags = &l.by_para[&p.id];
        let mut next = 0;
        for &i in frags {
            let f = &l.fragments[i];
            assert_eq!(f.lines.start, next, "lines in order");
            next = f.lines.end;
            let r = f.rect();
            let page = &l.pages[page_of(l, r)];
            assert!(r.y0 >= page.body.y0 - 0.5 && r.y1 <= page.body.y1 + 0.5, "{r:?} inside {:?}", page.body);
        }
        assert_eq!(next, l.fragments[frags[0]].para.lines.len(), "all lines placed");
    }
}

#[test]
fn long_documents_flow_onto_pages() {
    let doc = long_doc(40);
    let l = layout(&doc);
    assert!(l.pages.len() >= 3, "{} pages", l.pages.len());
    check_complete(&doc, &l);
    // Pages stack without overlapping.
    for w in l.pages.windows(2) {
        assert!(w[1].rect.y0 > w[0].rect.y1);
    }
    assert_eq!(l.pages.iter().map(|p| p.number).collect::<Vec<_>>(), (1..=l.pages.len() as u32).collect::<Vec<_>>());
}

#[test]
fn widows_and_orphans_are_avoided() {
    let doc = long_doc(40);
    let l = layout(&doc);
    for f in &l.fragments {
        let total = f.para.lines.len();
        if f.lines.len() < total {
            assert!(f.lines.len() >= 2, "no single line split off a {total}-line paragraph: {:?}", f.lines);
        }
    }
}

#[test]
fn page_breaks_and_headings() {
    let mut ed = Editor::new(long_doc(12));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[3], 0));
    ops::apply_para_props(&mut ed, &ParaProps { page_break_before: Some(true), ..Default::default() }, "b").unwrap();
    let l = layout(&ed.doc);
    let first = l.by_para[&ids[3]][0];
    let pg = page_of(&l, l.fragments[first].rect());
    assert_eq!(pg, 1);
    assert!((l.fragments[first].rect().y0 - l.pages[1].body.y0).abs() < 1.0, "at the top of page 2");

    // A heading is never the last thing on a page.
    let mut text = Vec::new();
    for i in 0..30 {
        text.push(format!("Heading {i}"));
        text.push(PARA.to_string());
    }
    let mut ed = Editor::new(Document::from_plain_text(&text.join("\n")));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    for id in ids.iter().step_by(2) {
        ed.sel = Selection::caret(Pos::new(*id, 0));
        ops::set_style(&mut ed, &StyleSheet::heading(2)).unwrap();
    }
    let l = layout(&ed.doc);
    check_complete(&ed.doc, &l);
    for pair in ids.chunks(2) {
        let h = &l.fragments[l.by_para[&pair[0]][0]];
        let b = &l.fragments[l.by_para[&pair[1]][0]];
        assert_eq!(page_of(&l, h.rect()), page_of(&l, b.rect()), "heading kept with its text");
    }
}

#[test]
fn columns_and_continuous_sections() {
    let mut ed = Editor::new(long_doc(8));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::insert_section_break(&mut ed, SectionStart::Continuous).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[5], 0));
    ops::insert_section_break(&mut ed, SectionStart::Continuous).unwrap();
    let mut page = ed.doc.sections[1].page.clone();
    page.columns = 2;
    ops::set_section(&mut ed, 1, SectionStart::Continuous, page).unwrap();
    let l = layout(&ed.doc);
    check_complete(&ed.doc, &l);
    // Section 2 (paragraphs 1–4) is in two columns on page 1, balanced.
    let frags: Vec<_> = ids[1..5].iter().flat_map(|id| l.by_para[id].iter().map(|&i| l.fragments[i].rect())).collect();
    let xs: std::collections::BTreeSet<i64> = frags.iter().map(|r| r.x0 as i64).collect();
    assert_eq!(xs.len(), 2, "two columns");
    let col_bottom = |x: i64| frags.iter().filter(|r| r.x0 as i64 == x).map(|r| r.y1).fold(0.0, f64::max);
    let (a, b) = (col_bottom(*xs.first().unwrap()), col_bottom(*xs.last().unwrap()));
    assert!((a - b).abs() < 40.0, "balanced: {a} vs {b}");
    // The section after continues below both columns, full width.
    let after = l.fragments[l.by_para[&ids[5]][0]].rect();
    assert!(after.y0 >= a.max(b) - 0.5);
    assert!(after.width() > frags[0].width() * 1.5);
}

#[test]
fn headers_and_footers_with_fields() {
    let mut doc = long_doc(30);
    doc.meta.title = "Report".into();
    let ps = &mut doc.sections[0].page;
    ps.header = HfLine { left: "{title}".into(), ..Default::default() };
    ps.footer = HfLine { center: "Page {page} of {pages}".into(), ..Default::default() };
    ps.different_first = true;
    let l = layout(&doc);
    let n = l.pages.len();
    assert!(n >= 2);
    // First page: no header/footer (different first page, left empty); others: 2 each.
    assert_eq!(l.decor.len(), (n - 1) * 2);
    assert_eq!(
        xenify_wp_layout::fill_fields("Page {page} of {pages} – {title}", &doc, "2", n, n),
        format!("Page 2 of {n} – Report")
    );
    for f in &l.decor {
        let pg = &l.pages[page_of(&l, f.rect())];
        let r = f.rect();
        assert!(r.y1 <= pg.body.y0 + 0.5 || r.y0 >= pg.body.y1 - 0.5, "header/footer outside the text area: {r:?}");
    }
}

#[test]
fn pageless_is_one_page() {
    let mut doc = long_doc(30);
    doc.meta.pageless = true;
    let l = layout(&doc);
    assert_eq!(l.pages.len(), 1);
    check_complete(&doc, &l);
}

#[test]
fn odd_even_headers_number_formats_and_linked_sections() {
    let mut doc = long_doc(30);
    let ps = &mut doc.sections[0].page;
    ps.header = HfLine { right: "Odd {page}".into(), ..Default::default() };
    ps.different_odd_even = true;
    ps.even_header = HfLine { left: "{page}".into(), ..Default::default() };
    ps.number_format = xenify_wp_doc::PageNumberFormat::LowerRoman;
    let l = layout(&doc);
    assert!(l.pages.len() >= 3);
    // Header text length per page: "Odd i" (5), "ii" (2), "Odd iii" (7).
    let header_len = |l: &DocLayout, page: usize| {
        l.decor.iter().filter(|f| page_of(l, f.rect()) == page).map(|f| f.para.text_len).collect::<Vec<_>>()
    };
    assert_eq!(header_len(&l, 0), [5]);
    assert_eq!(header_len(&l, 1), [2]);
    assert_eq!(header_len(&l, 2), [7]);

    // A second section linked to the first shows the first's headers; unlinked, its own (none).
    let mut ed = Editor::new(doc);
    let last = ed.doc.paragraphs().nth(20).unwrap().id;
    ed.sel = Selection::caret(Pos::new(last, 0));
    ops::insert_section_break(&mut ed, SectionStart::NewPage).unwrap();
    ed.doc.sections[1].page.header = HfLine::default();
    ed.doc.sections[1].page.different_odd_even = false;
    ed.doc.sections[1].page.link_to_previous = true;
    let l = layout(&ed.doc);
    let last_page = l.pages.len() - 1;
    assert!(!header_len(&l, last_page).is_empty(), "linked section shows the previous headers");
    ed.doc.sections[1].page.link_to_previous = false;
    let l = layout(&ed.doc);
    assert!(header_len(&l, last_page).is_empty());
}

#[test]
fn rich_headers_on_every_page_and_live_when_edited() {
    use xenify_wp_doc::{Block, Field, HeaderPart, HfSlot, Paragraph};
    let mut doc = long_doc(30);
    let hid = doc.alloc_id();
    let pid = doc.alloc_id();
    let mut p = Paragraph::new(pid, StyleSheet::HEADER, "Report — page ");
    p.text.push(Field::CHAR);
    p.runs.push(xenify_wp_doc::Run {
        len: Field::CHAR.len_utf8(),
        fmt: xenify_wp_doc::CharFmt { field: Some(Field::Page), ..Default::default() },
    });
    doc.sections[0].headers.push(HeaderPart {
        id: hid,
        footer: false,
        slot: HfSlot::Default,
        blocks: vec![Block::Paragraph(p)],
    });
    let mut engine = Engine::new(Fonts::bundled_only());
    let l = engine.layout(&doc);
    let n = l.pages.len();
    assert!(n >= 2);
    // One header per page, in the top margin.
    let headers: Vec<_> = l.decor.iter().filter(|f| f.para.id == pid).collect();
    assert_eq!(headers.len(), n);
    for f in &headers {
        let pg = &l.pages[page_of(&l, f.rect())];
        assert!(f.rect().y1 <= pg.body.y0 + 0.5 && f.rect().y0 >= pg.rect.y0);
    }
    // Page numbers differ: page 1's header lays out apart from page 2's.
    assert!(!std::sync::Arc::ptr_eq(&headers[0].para, &headers[1].para));
    // Editing page 2's header: that copy is live (caret and selection work on it).
    engine.header_edit = Some(1);
    let l = engine.layout(&doc);
    assert_eq!(l.by_para.get(&pid).map(Vec::len), Some(1));
    let live = &l.fragments[l.by_para[&pid][0]];
    assert_eq!(page_of(&l, live.rect()), 1);
    assert!(l.caret(xenify_wp_doc::Pos::new(pid, 3), false).is_some());
    assert_eq!(l.decor.iter().filter(|f| f.para.id == pid).count(), n - 1);
    // A rich footer sits in the bottom margin.
    let fid = doc.alloc_id();
    let fp = doc.alloc_id();
    doc.sections[0].headers.push(HeaderPart {
        id: fid,
        footer: true,
        slot: HfSlot::Default,
        blocks: vec![Block::Paragraph(Paragraph::new(fp, StyleSheet::FOOTER, "Confidential"))],
    });
    engine.header_edit = None;
    let l = engine.layout(&doc);
    let f = l.decor.iter().find(|f| f.para.id == fp).unwrap();
    let pg = &l.pages[page_of(&l, f.rect())];
    assert!(f.rect().y0 >= pg.body.y1 - 0.5 && f.rect().y1 <= pg.rect.y1);
}

#[test]
fn a_title_page_is_centred_and_the_rest_follows_on_the_next_page() {
    let mut ed = Editor::new(Document::from_plain_text("Chapter one starts here."));
    ops::insert_title_page(&mut ed).unwrap();
    assert_eq!(ed.doc.sections.len(), 2);
    assert_eq!(ed.doc.sections[0].page.v_align, xenify_wp_doc::PageVAlign::Center);
    let l = layout(&ed.doc);
    let title = ed.doc.sections[0].blocks.iter().find_map(|b| match b {
        xenify_wp_doc::Block::Paragraph(p) => Some(p.id),
        _ => None,
    });
    let r = l.fragments[l.by_para[&title.unwrap()][0]].rect();
    let page = &l.pages[0];
    // Well down the page: around the middle, not at the top margin.
    assert!(r.y0 > page.body.y0 + page.body.height() * 0.3, "{r:?} on {:?}", page.body);
    let body = ed.doc.paragraphs().last().unwrap().id;
    assert_eq!(page_of(&l, l.fragments[l.by_para[&body][0]].rect()), 1, "the text after it starts a new page");
}
