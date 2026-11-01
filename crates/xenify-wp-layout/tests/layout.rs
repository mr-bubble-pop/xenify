use xenify_wp_doc::{Document, Editor, Pos, Selection, StyleSheet, ops};
use xenify_wp_layout::{Engine, Fonts};

fn engine() -> Engine {
    Engine::new(Fonts::bundled_only())
}

const LONG: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut \
                    labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco \
                    laboris nisi ut aliquip ex ea commodo consequat.";

#[test]
fn paragraphs_stack_and_wrap() {
    let doc = Document::from_plain_text(&format!("Short\n{LONG}\n\nEnd"));
    let l = engine().layout_pageless(&doc);
    assert_eq!(l.fragments.len(), 4);
    assert_eq!(l.fragments[0].lines.len(), 1);
    assert!(l.fragments[1].lines.len() >= 3, "long paragraph wraps");
    // An empty paragraph still has a line (for the caret).
    assert_eq!(l.fragments[2].lines.len(), 1);
    for w in l.fragments.windows(2) {
        assert!(w[1].rect().y0 >= w[0].rect().y1, "paragraphs don't overlap");
    }
    // Every line fits the column.
    for f in &l.fragments {
        for line in f.para.layout.lines() {
            assert!(line.metrics().advance - line.metrics().trailing_whitespace <= l.width + 0.5);
        }
    }
}

#[test]
fn headings_are_bigger() {
    let mut ed = Editor::new(Document::from_plain_text("Title\nbody"));
    ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    let l = engine().layout_pageless(&ed.doc);
    let h = |i: usize| l.fragments[i].para.lines[0].bottom - l.fragments[i].para.lines[0].top;
    assert!(h(0) > h(1) * 1.4, "{} vs {}", h(0), h(1));
}

#[test]
fn hit_testing_finds_the_caret_again() {
    let doc = Document::from_plain_text(&format!("{LONG}\nsecond paragraph"));
    let l = engine().layout_pageless(&doc);
    for p in doc.paragraphs() {
        for (offset, _) in p.text.char_indices().step_by(7) {
            let pos = Pos::new(p.id, offset);
            let c = l.caret(pos, false).unwrap();
            // Just right of the caret, halfway down.
            let hit = l.hit(kurbo::Point::new(c.x0 + 0.5, c.center().y)).unwrap();
            assert_eq!(hit, pos, "offset {offset}");
        }
    }
    // Far below the text: the end of the last line.
    let end = l.hit(kurbo::Point::new(1e4, 1e5)).unwrap();
    assert_eq!(end, doc.end());
}

#[test]
fn arrow_keys_move_between_lines_and_paragraphs() {
    let doc = Document::from_plain_text(&format!("one\n{LONG}\nlast"));
    let l = engine().layout_pageless(&doc);
    let ids: Vec<_> = doc.paragraphs().map(|p| p.id).collect();
    let start = Pos::new(ids[0], 1);
    let x = l.caret(start, false).unwrap().x0;
    let down = l.line_move(start, false, 1, x).unwrap();
    assert_eq!(down.para, ids[1]);
    assert!(down.offset <= 2);
    let lines = l.fragments[1].lines.len() as isize;
    let bottom = l.line_move(down, false, lines, x).unwrap();
    assert_eq!(bottom.para, ids[2]);
    assert!(l.line_move(bottom, false, 1, x).is_none(), "nothing below the last line");
    assert!(l.line_move(start, false, -1, x).is_none());
}

#[test]
fn selection_covers_every_line() {
    let doc = Document::from_plain_text(&format!("{LONG}\nnext"));
    let l = engine().layout_pageless(&doc);
    let sel = Selection { anchor: doc.start(), focus: doc.end() };
    let rects = l.selection_rects(&doc, sel);
    assert!(rects.len() > l.fragments[0].lines.len());
}

#[test]
fn cache_reuses_unchanged_paragraphs() {
    let mut e = engine();
    let mut ed = Editor::new(Document::from_plain_text("a\nb"));
    let l1 = e.layout_pageless(&ed.doc);
    ed.sel = Selection::caret(ed.doc.end());
    ops::insert_text(&mut ed, "c", None).unwrap();
    let l2 = e.layout_pageless(&ed.doc);
    assert!(std::sync::Arc::ptr_eq(&l1.fragments[0].para, &l2.fragments[0].para));
    assert!(!std::sync::Arc::ptr_eq(&l1.fragments[1].para, &l2.fragments[1].para));
}

/// The x of the caret before byte `at` of paragraph `i`, from the column's left edge.
fn x_of(l: &xenify_wp_layout::DocLayout, doc: &Document, i: usize, at: usize) -> f64 {
    let id = doc.paragraphs().nth(i).unwrap().id;
    let col_left = l.fragments[i].rect().x0;
    l.caret(Pos::new(id, at), false).unwrap().x0 - col_left
}

#[test]
fn tab_stops_place_text() {
    use xenify_wp_doc::{ParaProps, TabKind, TabLeader, TabStop};
    let mut ed = Editor::new(Document::from_plain_text("a\tb\nName\t12\nTotal\t3.25\nx\ty\tz"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    let tabs = |stops: Vec<TabStop>| ParaProps { tabs: Some(stops), ..Default::default() };
    let set = |ed: &mut Editor, i: usize, p: ParaProps| {
        ed.sel = Selection::caret(Pos::new(ids[i], 0));
        ops::apply_para_props(ed, &p, "Tabs").unwrap();
    };
    set(&mut ed, 0, tabs(vec![TabStop { pos: 100.0, ..Default::default() }]));
    set(&mut ed, 1, tabs(vec![TabStop { pos: 200.0, kind: TabKind::Right, leader: TabLeader::Dots }]));
    set(&mut ed, 2, tabs(vec![TabStop { pos: 150.0, kind: TabKind::Decimal, ..Default::default() }]));
    // Paragraph 3 has no stops: the defaults every half inch.
    let doc = ed.doc.clone();
    let l = engine().layout_pageless(&doc);
    let near = |a: f64, b: f64| (a - b).abs() < 1.5;
    assert!(near(x_of(&l, &doc, 0, 2), 100.0), "left stop: {}", x_of(&l, &doc, 0, 2));
    assert!(near(x_of(&l, &doc, 1, 7), 200.0), "right stop: text ends at {}", x_of(&l, &doc, 1, 7));
    assert!(near(x_of(&l, &doc, 2, 7), 150.0), "decimal stop: point at {}", x_of(&l, &doc, 2, 7));
    assert!(near(x_of(&l, &doc, 3, 2), 36.0), "default stop: {}", x_of(&l, &doc, 3, 2));
    assert!(near(x_of(&l, &doc, 3, 4), 72.0), "second default stop: {}", x_of(&l, &doc, 3, 4));
    // The right tab's dot leader is drawn.
    let dots = l.fragments[1].para.items.iter().filter(|i| matches!(i, xenify_wp_layout::Item::Rect { .. })).count();
    assert!(dots > 20, "dot leader: {dots} dots");
}

#[test]
fn first_line_and_hanging_indents() {
    use xenify_wp_doc::ParaProps;
    let mut ed = Editor::new(Document::from_plain_text(&format!("{LONG}\n{LONG}")));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[0], 0));
    ops::apply_para_props(&mut ed, &ParaProps { first_line: Some(36.0), ..Default::default() }, "Indent").unwrap();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    let hang = ParaProps { indent_left: Some(36.0), first_line: Some(-36.0), ..Default::default() };
    ops::apply_para_props(&mut ed, &hang, "Indent").unwrap();
    let doc = ed.doc.clone();
    let l = engine().layout_pageless(&doc);
    let line2 = |i: usize| {
        let p = &l.fragments[i].para;
        let second = p.layout.lines().nth(1).unwrap().text_range().start;
        x_of(&l, &doc, i, second)
    };
    assert!((x_of(&l, &doc, 0, 0) - 36.0).abs() < 1.0, "first line indented");
    assert!(line2(0).abs() < 1.0, "other lines at the margin");
    assert!(x_of(&l, &doc, 1, 0).abs() < 1.0, "hanging: first line at the margin");
    assert!((line2(1) - 36.0).abs() < 1.0, "hanging: other lines indented");
}

#[test]
fn column_break_and_line_between() {
    use xenify_wp_doc::ParaProps;
    let mut ed = Editor::new(Document::from_plain_text("Left column\nRight column"));
    let mut page = ed.doc.sections[0].page.clone();
    page.columns = 2;
    page.column_line = true;
    let start = ed.doc.sections[0].start;
    ops::set_section(&mut ed, 0, start, page).unwrap();
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::apply_para_props(&mut ed, &ParaProps { column_break_before: Some(true), ..Default::default() }, "Break")
        .unwrap();
    let l = engine().layout(&ed.doc);
    let (a, b) = (l.fragments[0].rect(), l.fragments[1].rect());
    assert!(b.x0 > a.x1, "the second paragraph is in the second column: {a:?} {b:?}");
    assert!((b.y0 - a.y0).abs() < 1.0, "both at the top");
    let line = l.shapes.iter().find(|s| s.rect.width() < 1.0 && s.rect.height() > 5.0).expect("a line between");
    assert!(line.rect.x0 > a.x1 && line.rect.x1 < b.x0);
}

#[test]
fn footnotes_go_to_the_page_bottom_and_endnotes_to_the_end() {
    use xenify_wp_doc::{NoteKind, notes};
    let mut ed = Editor::new(Document::from_plain_text("A claim that needs a source.\nMore text."));
    let first = ed.doc.paragraphs().next().unwrap().id;
    ed.sel = Selection::caret(Pos::new(first, 7));
    let foot = notes::insert_note(&mut ed, NoteKind::Foot).unwrap();
    ops::insert_text(&mut ed, "See Smith (2020).", None).unwrap();
    ed.sel = Selection::caret(ed.doc.paragraphs().nth(1).map(|p| Pos::new(p.id, 4)).unwrap());
    let end = notes::insert_note(&mut ed, NoteKind::End).unwrap();
    ops::insert_text(&mut ed, "An aside.", None).unwrap();
    let doc = ed.doc.clone();
    assert_eq!(notes::numbers(&doc)[&foot], "1");
    assert_eq!(notes::numbers(&doc)[&end], "i");
    let l = engine().layout(&doc);
    let note_para = doc.note(foot).unwrap().blocks[0].id();
    let frag = |id| l.fragments.iter().find(|f| f.para.id == id).unwrap().rect();
    let page = &l.pages[0];
    let n = frag(note_para);
    assert!(
        n.y1 <= page.body.y1 + 0.5 && n.y0 > page.body.y1 - 80.0,
        "the footnote sits at the bottom: {n:?} {:?}",
        page.body
    );
    let body_text = frag(first);
    assert!(n.y0 > body_text.y1 + 100.0);
    // The endnote comes after the text, under a short rule.
    let e = frag(doc.note(end).unwrap().blocks[0].id());
    let second = frag(doc.paragraphs().nth(1).unwrap().id);
    assert!(e.y0 > second.y1, "endnote after the text");
    // The note number in the text is drawn (a glyph run beyond the text's own).
    let label_runs = l
        .fragments
        .iter()
        .find(|f| f.para.id == first)
        .unwrap()
        .para
        .items
        .iter()
        .filter(|i| matches!(i, xenify_wp_layout::Item::Glyphs { .. }))
        .count();
    assert!(label_runs >= 2, "the reference number is drawn");
}

#[test]
fn hints_and_broken_fields_are_screen_only() {
    use xenify_wp_doc::{Field, TocKind, tables};
    let mut ed = Editor::new(Document::from_plain_text("See below."));
    let id = ed.doc.paragraphs().next().unwrap().id;
    ed.sel = Selection::caret(Pos::new(id, 4));
    // A cross-reference to a paragraph that doesn't exist, and an empty change log.
    ops::insert_field(&mut ed, Field::Ref { target: xenify_wp_doc::NodeId(9999) }, "Field").unwrap();
    tables::insert_generated(&mut ed, TocKind::ChangeLog).unwrap();
    let l = engine().layout(&ed.doc);
    // Nothing printable says "heading deleted" or "Change log": those marks are screen-only.
    let printed_glyphs: usize = l
        .fragments
        .iter()
        .map(|f| f.para.items.iter().filter(|i| matches!(i, xenify_wp_layout::Item::Glyphs { .. })).count())
        .sum();
    let screen: usize = l.fragments.iter().map(|f| f.para.screen.len()).sum();
    assert!(screen >= 2, "the hint and the marker are drawn on screen");
    assert!(printed_glyphs > 0);
    // What prints is the text itself: no hint, no error marker.
    let printed = xenify_wp_io_free_check(&ed.doc);
    assert!(printed.contains("See"));
    assert!(!printed.contains("Change") && !printed.contains("deleted"), "{printed}");
}

/// The text of the layout's printable glyph runs (what a PDF would hold), as characters.
fn xenify_wp_io_free_check(doc: &Document) -> String {
    let l = engine().layout(doc);
    l.fragments
        .iter()
        .flat_map(|f| f.para.items.iter())
        .filter_map(|i| match i {
            xenify_wp_layout::Item::Glyphs { chars, .. } => Some(chars.concat()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_line_of_only_a_field_or_emoji_is_as_tall_as_text() {
    use xenify_wp_doc::Field;
    let line_of = |text: &str, field: Option<Field>| {
        let mut ed = Editor::new(Document::from_plain_text(text));
        let id = ed.doc.paragraphs().next().unwrap().id;
        ed.sel = Selection::caret(Pos::new(id, 0));
        if let Some(f) = field {
            ops::insert_field(&mut ed, f, "Field").unwrap();
        }
        let l = engine().layout(&ed.doc);
        let line = l.fragments[0].para.lines[0];
        let len = ed.doc.para(id).unwrap().text.len();
        let caret = l.fragments[0].para.caret(len, false);
        (line.top, line.bottom, line.baseline, caret.height())
    };
    let text = line_of("1", None);
    // Typing next to the field mustn't move the line: same height, same baseline, full caret.
    for (t, f) in [("", Some(Field::Page)), ("😀", None)] {
        let got = line_of(t, f.clone());
        for (a, b) in [(got.0, text.0), (got.1, text.1), (got.2, text.2), (got.3 as f32, text.3 as f32)] {
            assert!((a - b).abs() < 0.05, "{t:?} {f:?}: {got:?} vs text {text:?}");
        }
    }
}

#[test]
fn empty_generated_lists_show_a_faded_example_on_screen_only() {
    use xenify_wp_doc::{Editor, TocKind, tables};
    let mut ed = Editor::new(Document::from_plain_text(""));
    tables::insert_toc(&mut ed, 3).unwrap();
    tables::insert_generated(&mut ed, TocKind::ChangeLog).unwrap();
    tables::insert_generated(&mut ed, TocKind::Bibliography).unwrap();
    tables::insert_generated(&mut ed, TocKind::Figures).unwrap();
    let l = engine().layout(&ed.doc);
    let ghosts: Vec<_> = l.fragments.iter().filter(|f| f.para.generated.is_some()).collect();
    // One line saying what the contents will list, two change log rows, two sources, two captions.
    assert_eq!(ghosts.len(), 7);
    assert!(ghosts.iter().all(|f| f.para.items.is_empty() && !f.para.screen.is_empty()), "nothing printed");
    // Right-clicking one finds its list.
    let toc = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, xenify_wp_doc::Block::Toc(_))).unwrap().id();
    assert_eq!(l.generated_at(ghosts[0].rect().center()), Some(toc));
}
