use xenify_wp_doc::{Align, CharFmt, Document, Editor, ListDef, ParaProps, Pos, Selection, StyleSheet, lists, ops};

fn texts(ed: &Editor) -> Vec<String> {
    ed.doc.paragraphs().map(|p| p.text.clone()).collect()
}

fn caret(ed: &Editor) -> (usize, usize) {
    let f = ed.sel.focus;
    (ed.doc.para_index(f.para).unwrap(), f.offset)
}

#[test]
fn typing_enter_and_backspace() {
    let mut ed = Editor::new(Document::new());
    for c in "Hello world".chars() {
        ops::insert_text(&mut ed, &c.to_string(), None).unwrap();
    }
    assert_eq!(texts(&ed), ["Hello world"]);
    // Enter in the middle splits.
    let p = ed.doc.start().para;
    ed.sel = Selection::caret(Pos::new(p, 5));
    ops::split_paragraph(&mut ed).unwrap();
    assert_eq!(texts(&ed), ["Hello", " world"]);
    assert_eq!(caret(&ed), (1, 0));
    // Backspace at the start of a paragraph joins it to the one before.
    ops::backspace(&mut ed, false).unwrap();
    assert_eq!(texts(&ed), ["Hello world"]);
    assert_eq!(caret(&ed), (0, 5));
    ops::delete_forward(&mut ed, true).unwrap();
    assert_eq!(texts(&ed), ["Helloworld"]);
}

#[test]
fn typing_undoes_by_word() {
    let mut ed = Editor::new(Document::new());
    for c in "one two".chars() {
        ops::insert_text(&mut ed, &c.to_string(), None).unwrap();
    }
    ed.undo();
    assert_eq!(texts(&ed), ["one "]);
    ed.undo();
    assert_eq!(texts(&ed), [""]);
    assert!(!ed.can_undo());
    ed.redo();
    ed.redo();
    assert_eq!(texts(&ed), ["one two"]);
}

#[test]
fn deleting_across_paragraphs() {
    let mut ed = Editor::new(Document::from_plain_text("alpha\nbeta\ngamma\ndelta"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection { anchor: Pos::new(ids[3], 2), focus: Pos::new(ids[0], 3) };
    assert_eq!(ed.doc.text_in(ed.sel), "ha\nbeta\ngamma\nde");
    ops::insert_text(&mut ed, "X", None).unwrap();
    assert_eq!(texts(&ed), ["alpXlta"]);
    ed.undo();
    assert_eq!(texts(&ed), ["alpha", "beta", "gamma", "delta"]);
    assert_eq!(ed.doc.paragraphs().map(|p| p.id).collect::<Vec<_>>(), ids);
}

#[test]
fn pasting_multiple_lines() {
    let mut ed = Editor::new(Document::from_plain_text("start end"));
    let p = ed.doc.start().para;
    ed.sel = Selection::caret(Pos::new(p, 6));
    ops::insert_text(&mut ed, "one\ntwo\r\nthree ", None).unwrap();
    assert_eq!(texts(&ed), ["start one", "two", "three end"]);
    assert_eq!(caret(&ed), (2, 6));
}

#[test]
fn enter_after_heading_starts_normal_text() {
    let mut ed = Editor::new(Document::from_plain_text("Intro"));
    ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    ed.sel = Selection::caret(ed.doc.end());
    ops::split_paragraph(&mut ed).unwrap();
    let styles: Vec<_> = ed.doc.paragraphs().map(|p| p.style.clone()).collect();
    assert_eq!(styles, ["heading1", "normal"]);
}

#[test]
fn formatting_splits_and_merges_runs() {
    let mut ed = Editor::new(Document::from_plain_text("make this bold"));
    let p = ed.doc.start().para;
    ed.sel = Selection { anchor: Pos::new(p, 5), focus: Pos::new(p, 9) };
    let bold = CharFmt { bold: Some(true), ..Default::default() };
    ops::apply_format(&mut ed, &bold, "Bold").unwrap();
    let para = ed.doc.para(p).unwrap();
    assert_eq!(para.runs.len(), 3);
    assert_eq!(ops::common_format(&ed).bold, Some(true));
    // Typing inside bold text stays bold.
    ed.sel = Selection::caret(Pos::new(p, 7));
    ops::insert_text(&mut ed, "!", None).unwrap();
    assert_eq!(ed.doc.para(p).unwrap().runs.len(), 3);
    // Un-bolding everything merges the runs back into one.
    ed.sel = Selection { anchor: Pos::new(p, 0), focus: ed.doc.end() };
    ops::apply_format(&mut ed, &CharFmt { bold: Some(false), ..Default::default() }, "Bold").unwrap();
    assert_eq!(ed.doc.para(p).unwrap().runs.len(), 1);
}

/// Random edits, then undoing everything, gives back the original document;
/// redoing everything gives back the edited one.
#[test]
fn undo_redo_round_trip_random_edits() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rand = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };
    for _round in 0..20 {
        let original = Document::from_plain_text("The quick brown fox\njumps over\n\nthe lazy dög ✓");
        let mut ed = Editor::new(original.clone());
        let mut steps = 0;
        for _ in 0..40 {
            // Random selection on char boundaries.
            let paras: Vec<_> = ed.doc.paragraphs().map(|p| (p.id, p.text.clone())).collect();
            let pick = |r: &mut dyn FnMut(usize) -> usize| {
                let (id, text) = &paras[r(paras.len())];
                let bounds: Vec<usize> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
                Pos::new(*id, bounds[r(bounds.len())])
            };
            let (a, f) = (pick(&mut rand), pick(&mut rand));
            ed.sel = Selection { anchor: a, focus: if rand(3) == 0 { a } else { f } };
            ed.seal();
            let r = match rand(10) {
                8 => ops::insert_section_break(&mut ed, xenify_wp_doc::SectionStart::Continuous),
                9 => ops::insert_page_break(&mut ed),
                0 => ops::insert_text(&mut ed, "ab", None),
                1 => ops::insert_text(&mut ed, "x\ny", None),
                2 => ops::backspace(&mut ed, rand(2) == 0),
                3 => ops::delete_forward(&mut ed, false),
                4 => ops::split_paragraph(&mut ed),
                5 => ops::set_style(&mut ed, StyleSheet::QUOTE),
                6 if rand(2) == 0 => ops::toggle_list(&mut ed, ListDef::NUMBERS),
                6 => ops::change_list_level(&mut ed, 1).map(|_| ()),
                _ => ops::apply_format(&mut ed, &CharFmt { italic: Some(true), ..Default::default() }, "Italic"),
            };
            r.unwrap();
            steps += 1;
        }
        assert!(steps > 0);
        let edited: Vec<_> = ed.doc.paragraphs().cloned().collect();
        while ed.undo() {}
        let strip = |d: &Document| {
            assert!(d.sections.iter().all(|s| !s.blocks.is_empty()), "no empty sections");
            d.paragraphs()
                .map(|p| (p.id, p.style.clone(), p.text.clone(), p.runs.clone(), p.list.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(strip(&ed.doc), strip(&original));
        while ed.redo() {}
        let again: Vec<_> = ed.doc.paragraphs().cloned().collect();
        let key = |v: &[xenify_wp_doc::Paragraph]| {
            v.iter()
                .map(|p| (p.id, p.style.clone(), p.text.clone(), p.runs.clone(), p.list.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&again), key(&edited));
    }
}

#[test]
fn documents_round_trip_through_json() {
    let mut ed = Editor::new(Document::from_plain_text("a\nb"));
    ops::apply_format(&mut ed, &CharFmt { bold: Some(true), ..Default::default() }, "Bold").unwrap();
    let json = serde_json::to_string(&ed.doc).unwrap();
    let mut back: Document = serde_json::from_str(&json).unwrap();
    back.repair();
    assert_eq!(back.plain_text(), "a\nb");
    assert_eq!(back.next_id, ed.doc.next_id);
}

fn labels(ed: &Editor) -> Vec<String> {
    let l = lists::labels(&ed.doc);
    ed.doc.paragraphs().map(|p| l.get(&p.id).cloned().unwrap_or_default()).collect()
}

#[test]
fn numbered_lists_count_and_nest() {
    let mut ed = Editor::new(Document::from_plain_text("one\ntwo\nsub a\nsub b\nthree"));
    ed.sel = Selection { anchor: ed.doc.start(), focus: ed.doc.end() };
    ops::toggle_list(&mut ed, ListDef::NUMBERS).unwrap();
    assert_eq!(labels(&ed), ["1.", "2.", "3.", "4.", "5."]);
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection { anchor: Pos::new(ids[2], 0), focus: Pos::new(ids[3], 1) };
    assert!(ops::change_list_level(&mut ed, 1).unwrap());
    assert_eq!(labels(&ed), ["1.", "2.", "a.", "b.", "3."]);
    // Enter at the end of an item continues the list…
    ed.sel = Selection::caret(Pos::new(ids[4], 5));
    ops::split_paragraph(&mut ed).unwrap();
    assert_eq!(labels(&ed), ["1.", "2.", "a.", "b.", "3.", "4."]);
    // …and Enter on the empty new item ends it.
    ops::split_paragraph(&mut ed).unwrap();
    assert_eq!(labels(&ed), ["1.", "2.", "a.", "b.", "3.", ""]);
    // Toggling the same list off takes items out; restart numbering works.
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::restart_numbering(&mut ed, Some(10)).unwrap();
    assert_eq!(labels(&ed)[..5], ["1.", "10.", "a.", "b.", "11."]);
}

#[test]
fn backspace_at_start_of_item_leaves_the_list() {
    let mut ed = Editor::new(Document::from_plain_text("a\nb"));
    ed.sel = Selection { anchor: ed.doc.start(), focus: ed.doc.end() };
    ops::toggle_list(&mut ed, ListDef::BULLETS).unwrap();
    let second = ed.doc.paragraphs().nth(1).unwrap().id;
    ed.sel = Selection::caret(Pos::new(second, 0));
    ops::backspace(&mut ed, false).unwrap();
    assert!(ed.doc.para(second).unwrap().list.is_none());
    assert_eq!(texts(&ed), ["a", "b"], "text is kept");
    ops::backspace(&mut ed, false).unwrap();
    assert_eq!(texts(&ed), ["ab"]);
}

#[test]
fn paragraph_properties_override_the_style() {
    let mut ed = Editor::new(Document::from_plain_text("centred"));
    ops::apply_para_props(&mut ed, &ParaProps { align: Some(Align::Center), ..Default::default() }, "Centre").unwrap();
    let p = ed.doc.paragraphs().next().unwrap();
    assert_eq!(ed.doc.resolve_para(p).align, Align::Center);
    ed.undo();
    let p = ed.doc.paragraphs().next().unwrap();
    assert_eq!(ed.doc.resolve_para(p).align, Align::Left);
}

#[test]
fn section_breaks_split_and_join() {
    let mut ed = Editor::new(Document::from_plain_text("one\ntwo\nthree"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::insert_section_break(&mut ed, xenify_wp_doc::SectionStart::Continuous).unwrap();
    assert_eq!(ed.doc.sections.len(), 2);
    assert_eq!(ed.doc.sections[1].blocks.len(), 2);
    assert_eq!(ed.doc.sections[1].start, xenify_wp_doc::SectionStart::Continuous);
    let mut page = ed.doc.sections[1].page.clone();
    page.columns = 2;
    ops::set_section(&mut ed, 1, xenify_wp_doc::SectionStart::Continuous, page).unwrap();
    assert_eq!(ed.doc.sections[1].page.columns, 2);
    // Deleting across the break merges the sections again.
    ed.sel = Selection { anchor: Pos::new(ids[0], 2), focus: Pos::new(ids[2], 2) };
    ops::delete_selection(&mut ed).unwrap();
    assert_eq!(ed.doc.sections.len(), 1);
    assert_eq!(texts(&ed), ["onree"]);
    while ed.undo() {}
    assert_eq!(ed.doc.sections.len(), 1);
    assert_eq!(texts(&ed), ["one", "two", "three"]);
}

#[test]
fn page_break_starts_a_page() {
    let mut ed = Editor::new(Document::from_plain_text("before after"));
    let p = ed.doc.start().para;
    ed.sel = Selection::caret(Pos::new(p, 7));
    ops::insert_page_break(&mut ed).unwrap();
    let second = ed.doc.paragraphs().nth(1).unwrap();
    assert_eq!(second.text, "after");
    assert_eq!(second.props.page_break_before, Some(true));
}

/// Types `s` one character at a time, running autoformat after spaces
/// like the editor does.
fn type_with_autoformat(ed: &mut Editor, s: &str) {
    for c in s.chars() {
        ops::insert_text(ed, &c.to_string(), None).unwrap();
        if c == ' ' {
            ops::autoformat(ed, &ops::Autoformat::default()).unwrap();
        }
    }
}

#[test]
fn markdown_shortcuts_make_lists_and_headings() {
    let cases: [(&str, &str, Option<&str>, &str); 7] = [
        ("* Milk", "Milk", Some(ListDef::BULLETS), StyleSheet::NORMAL),
        ("- Eggs", "Eggs", Some(ListDef::BULLETS), StyleSheet::NORMAL),
        ("1. First", "First", Some(ListDef::NUMBERS), StyleSheet::NORMAL),
        ("a. Alpha", "Alpha", Some(ListDef::LETTERS), StyleSheet::NORMAL),
        ("[ ] Call Sam", "Call Sam", Some(ListDef::CHECKLIST), StyleSheet::NORMAL),
        ("> Wise words", "Wise words", None, StyleSheet::QUOTE),
        ("Not * a list", "Not * a list", None, StyleSheet::NORMAL),
    ];
    for (typed, text, list, style) in cases {
        let mut ed = Editor::new(Document::new());
        type_with_autoformat(&mut ed, typed);
        let p = ed.doc.paragraphs().next().unwrap();
        assert_eq!(p.text, text, "{typed}");
        assert_eq!(p.list.as_ref().map(|l| l.list.as_str()), list, "{typed}");
        assert_eq!(p.style, style, "{typed}");
    }
    for n in 1..=6u8 {
        let mut ed = Editor::new(Document::new());
        type_with_autoformat(&mut ed, &format!("{} Title", "#".repeat(n as usize)));
        let p = ed.doc.paragraphs().next().unwrap();
        assert_eq!((p.text.as_str(), p.style.clone()), ("Title", StyleSheet::heading(n)));
    }
    // "3. " starts numbering at 3.
    let mut ed = Editor::new(Document::new());
    type_with_autoformat(&mut ed, "3. Third");
    assert_eq!(ed.doc.paragraphs().next().unwrap().list.as_ref().unwrap().restart, Some(3));
}

#[test]
fn undo_after_autoformat_gives_the_characters_back() {
    let mut ed = Editor::new(Document::new());
    type_with_autoformat(&mut ed, "* ");
    assert_eq!(texts(&ed), [""]);
    ed.undo();
    let p = ed.doc.paragraphs().next().unwrap();
    assert_eq!(p.text, "* ");
    assert!(p.list.is_none());
    // Turned off: nothing happens.
    let mut ed = Editor::new(Document::new());
    for c in "# Hi".chars() {
        ops::insert_text(&mut ed, &c.to_string(), None).unwrap();
        ops::autoformat(&mut ed, &ops::Autoformat { headings: false, ..Default::default() }).unwrap();
    }
    assert_eq!(texts(&ed), ["# Hi"]);
}

#[test]
fn tab_indents_a_list_item_and_checklists_tick() {
    let mut ed = Editor::new(Document::new());
    type_with_autoformat(&mut ed, "* One");
    ops::split_paragraph(&mut ed).unwrap();
    type_with_autoformat(&mut ed, "Two");
    assert!(ops::change_list_level(&mut ed, 1).unwrap());
    let levels: Vec<u8> = ed.doc.paragraphs().map(|p| p.list.as_ref().unwrap().level).collect();
    assert_eq!(levels, [0, 1]);
    // Checklist: tick, labels, untick.
    let mut ed = Editor::new(Document::new());
    type_with_autoformat(&mut ed, "[ ] Buy milk");
    let id = ed.doc.start().para;
    assert_eq!(lists::labels(&ed.doc)[&id], lists::UNCHECKED);
    ops::toggle_checked(&mut ed, id).unwrap();
    assert!(ed.doc.para(id).unwrap().list.as_ref().unwrap().checked);
    assert_eq!(lists::labels(&ed.doc)[&id], lists::CHECKED);
    ed.undo();
    assert!(!ed.doc.para(id).unwrap().list.as_ref().unwrap().checked);
    // Enter after a ticked item makes an unticked one.
    ops::toggle_checked(&mut ed, id).unwrap();
    ed.sel = Selection::caret(ed.doc.end());
    ops::split_paragraph(&mut ed).unwrap();
    let second = ed.doc.paragraphs().nth(1).unwrap();
    assert!(!second.list.as_ref().unwrap().checked);
}

#[test]
fn renaming_the_document_is_undoable() {
    let mut ed = Editor::new(Document::new());
    let mut meta = ed.doc.meta.clone();
    meta.title = "Plan".into();
    ops::set_meta(&mut ed, meta, "Rename").unwrap();
    assert_eq!(ed.doc.meta.title, "Plan");
    assert!(ed.is_dirty());
    ed.undo();
    assert_eq!(ed.doc.meta.title, "");
}

#[test]
fn copy_and_paste_keep_styles_lists_and_tables() {
    use xenify_wp_doc::{Block, clip, tables};
    // Source: a heading, a bullet, a table, a normal paragraph.
    let mut src = Editor::new(Document::from_plain_text("Heading here\nBullet item\nAfter table"));
    let ids: Vec<_> = src.doc.paragraphs().map(|p| p.id).collect();
    src.sel = Selection::caret(Pos::new(ids[0], 0));
    ops::set_style(&mut src, &StyleSheet::heading(1)).unwrap();
    src.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::toggle_list(&mut src, ListDef::BULLETS).unwrap();
    src.sel = Selection::caret(Pos::new(ids[1], 11));
    tables::insert_table(&mut src, 2, 2).unwrap();
    // Select everything from "Heading" to "After".
    src.sel = Selection { anchor: Pos::new(ids[0], 0), focus: Pos::new(ids[2], 5) };
    let frag = clip::copy_fragment(&src);
    let kinds: Vec<&str> = frag.sections[0]
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => "p",
            Block::Table(_) => "table",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["p", "p", "table", "p"], "{kinds:?}");

    // Paste into the middle of "Start | end".
    let mut dst = Editor::new(Document::from_plain_text("Start end"));
    let p = dst.doc.start().para;
    dst.sel = Selection::caret(Pos::new(p, 6));
    clip::paste_fragment(&mut dst, &frag, "Paste").unwrap();
    let paras: Vec<(String, String, bool)> =
        dst.doc.paragraphs().map(|p| (p.text.clone(), p.style.clone(), p.list.is_some())).collect();
    assert_eq!(paras.first().unwrap().0, "Start Heading here");
    assert!(paras.iter().any(|(t, _, l)| t == "Bullet item" && *l), "{paras:?}");
    assert_eq!(paras.last().unwrap().0, "Afterend");
    assert!(dst.doc.sections[0].blocks.iter().any(|b| matches!(b, Block::Table(_))));
    // Ids stay unique.
    let mut seen = std::collections::HashSet::new();
    assert!(dst.doc.paragraphs().all(|p| seen.insert(p.id)));
    // One undo removes the whole paste.
    dst.undo();
    assert_eq!(texts(&dst), ["Start end"]);

    // Into an empty paragraph, the first pasted paragraph keeps its heading style.
    let mut dst = Editor::new(Document::new());
    clip::paste_fragment(&mut dst, &frag, "Paste").unwrap();
    assert_eq!(dst.doc.paragraphs().next().unwrap().style, StyleSheet::heading(1));
}

#[test]
fn pictures_float_and_come_back() {
    use xenify_wp_doc::{Block, Wrap, frames, tables};
    let mut ed = Editor::new(Document::from_plain_text("Text"));
    tables::insert_image(&mut ed, "media/a.png", 100.0, 50.0).unwrap();
    let img = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, Block::Image(_))).unwrap().id();
    let frame = frames::float_image(&mut ed, img, 0, 0, (72.0, 144.0), Wrap::Around).unwrap();
    assert!(!ed.doc.sections[0].blocks.iter().any(|b| matches!(b, Block::Image(_))));
    let f = ed.doc.frame(frame).unwrap();
    assert_eq!((f.x, f.y, f.width, f.height, f.wrap), (72.0, 144.0, 100.0, 50.0, Wrap::Around));
    // One undo puts it back.
    ed.undo();
    assert!(ed.doc.sections[0].blocks.iter().any(|b| matches!(b, Block::Image(_))));
    ed.redo();
    let first = ed.doc.paragraphs().next().unwrap().id;
    frames::inline_image(&mut ed, frame, Some(first)).unwrap();
    assert!(ed.doc.frame(frame).is_none());
    assert!(matches!(ed.doc.sections[0].blocks[0], Block::Image(_)));
}

#[test]
fn update_a_style_to_match_the_selection() {
    use xenify_wp_doc::{CharFmt, StyleSheet};
    let mut ed = Editor::new(Document::from_plain_text("Title text\nAnother heading"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    for id in &ids {
        ed.sel = Selection::caret(Pos::new(*id, 0));
        ops::set_style(&mut ed, &StyleSheet::heading(1)).unwrap();
    }
    // Make the first one red and update Heading 1 from it.
    ed.sel = Selection { anchor: Pos::new(ids[0], 0), focus: Pos::new(ids[0], 10) };
    let red = xenify_wp_doc::Rgba::rgb(200, 0, 0);
    ops::apply_format(&mut ed, &CharFmt { color: Some(red), ..Default::default() }, "Colour").unwrap();
    ops::update_style_from_selection(&mut ed, &StyleSheet::heading(1)).unwrap();
    assert_eq!(ed.doc.styles.get(&StyleSheet::heading(1)).unwrap().char.color, Some(red));
    // Both headings are red now, through the style; the first has no direct colour left.
    assert!(ed.doc.para(ids[0]).unwrap().runs.iter().all(|r| r.fmt.color.is_none()));
    assert_eq!(ed.doc.resolve_para(ed.doc.para(ids[1]).unwrap()).char.color, red);
    ed.undo();
    assert_ne!(ed.doc.styles.get(&StyleSheet::heading(1)).unwrap().char.color, Some(red));
}

#[test]
fn regex_find_and_replace() {
    let mut ed = Editor::new(Document::from_plain_text("Smith, John\nDoe, Jane\nnobody"));
    let found = ops::find_regex(&ed.doc, r"(\w+), (\w+)", true, false).unwrap();
    assert_eq!(found.len(), 2);
    let with = ops::regex_replacements(&ed.doc, r"(\w+), (\w+)", true, false, "$2 $1", &found).unwrap();
    ops::replace_each(&mut ed, &found, &with).unwrap();
    let text: Vec<String> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(text, ["John Smith", "Jane Doe", "nobody"]);
    assert!(ops::find_regex(&ed.doc, "(", true, false).is_err());
    assert_eq!(ops::find_regex(&ed.doc, "JOHN", false, true).unwrap().len(), 1);
}

#[test]
fn filter_lines() {
    let doc = || Document::from_plain_text("error: one\nok\nerror: two\nfine");
    let mut ed = Editor::new(doc());
    let found = ops::find_all(&ed.doc, "error", false, false);
    assert_eq!(ops::filter_paragraphs(&mut ed, &found, false).unwrap(), 2);
    let text: Vec<String> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(text, ["ok", "fine"]);
    ed.undo();
    assert_eq!(ed.doc.paragraphs().count(), 4);
    let mut ed = Editor::new(doc());
    let found = ops::find_all(&ed.doc, "error", false, false);
    assert_eq!(ops::filter_paragraphs(&mut ed, &found, true).unwrap(), 2);
    let text: Vec<String> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(text, ["error: one", "error: two"]);
    // Nothing matches: keeping only matches leaves one empty paragraph.
    let mut ed = Editor::new(doc());
    ops::filter_paragraphs(&mut ed, &[], true).unwrap();
    assert_eq!(ed.doc.paragraphs().map(|p| p.text.as_str()).collect::<Vec<_>>(), [""]);
}

#[test]
fn find_and_replace_all() {
    let mut ed = Editor::new(Document::from_plain_text("The cat sat. Cats like the CAT tree.\nconcatenate the cat"));
    assert_eq!(ops::find_all(&ed.doc, "cat", false, false).len(), 5);
    assert_eq!(ops::find_all(&ed.doc, "cat", true, false).len(), 3);
    assert_eq!(ops::find_all(&ed.doc, "cat", false, true).len(), 3);
    let found = ops::find_all(&ed.doc, "cat", false, true);
    assert_eq!(ops::replace_all(&mut ed, &found, "dog").unwrap(), 3);
    let text: Vec<String> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(text, ["The dog sat. Cats like the dog tree.", "concatenate the dog"]);
    ed.undo();
    assert_eq!(ed.doc.paragraphs().next().unwrap().text, "The cat sat. Cats like the CAT tree.");
}
