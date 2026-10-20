use xenify_wp_doc::Document;
use xenify_wp_engine::{FontTable, Op, Session};
use xenify_wp_layout::Fonts;

fn session(text: &str) -> Session {
    Session::new(Document::from_plain_text(text), Fonts::bundled_only())
}

#[test]
fn typing_shows_on_the_page_with_outlines() {
    let mut s = session("");
    for c in "Hello world".chars() {
        s.type_text(&c.to_string());
    }
    let v = s.view();
    assert_eq!(v.pages.len(), 1);
    assert!(v.caret.is_some());
    assert_eq!(v.state.words, 2);
    let mut fonts = FontTable::default();
    let scenes = s.scenes(&[0], &mut fonts);
    let glyphs: Vec<_> = scenes[0]
        .ops
        .iter()
        .filter_map(|o| match o {
            Op::Glyphs { f, g, .. } => Some((*f, g.clone())),
            _ => None,
        })
        .collect();
    assert!(!glyphs.is_empty());
    let (f, g) = &glyphs[0];
    let paths = fonts.outlines(*f, &[g[0] as u32]);
    assert!(paths[0].starts_with('M'), "{paths:?}");
}

#[test]
fn keys_select_undo_and_format() {
    let mut s = session("one two");
    let m = |shift, command| serde_json::from_value(serde_json::json!({"shift": shift, "command": command})).unwrap();
    assert!(s.key("End", m(false, false)));
    assert!(s.key("ArrowLeft", m(true, true)));
    assert_eq!(s.selected_text(), "two");
    s.key("b", m(false, true));
    assert!(s.state().bold);
    s.key("z", m(false, true));
    assert!(!s.state().bold);
    let sig_before = s.view().pages[0].sig.clone();
    s.type_text("!");
    assert_ne!(s.view().pages[0].sig, sig_before);
}

#[test]
fn clicks_place_the_caret_and_double_click_selects_a_word() {
    let mut s = session("alpha beta gamma");
    let v = s.view();
    let c = v.caret.unwrap();
    s.pointer_down(c[0] + 40.0, c[1] + c[3] / 2.0, 2, Default::default());
    s.pointer_up();
    assert_eq!(s.selected_text(), "beta");
}

#[test]
fn tracked_typing_and_comments() {
    let mut s = session("alpha beta gamma");
    s.ed.doc.user = "Sam".into();
    xenify_wp_doc::review::set_tracking(&mut s.ed, true).unwrap();
    let m = |shift, command| serde_json::from_value(serde_json::json!({"shift": shift, "command": command})).unwrap();
    s.key("End", m(false, false));
    for c in " new".chars() {
        s.type_text(&c.to_string());
    }
    let changes = xenify_wp_doc::review::changes(&s.ed.doc);
    assert!(!changes.is_empty(), "typing while tracking makes a change");
    // A double click selects a word; a comment on it quotes it.
    let v = s.view();
    let c = v.caret.unwrap();
    s.key("Home", m(false, false));
    let v = s.view();
    let c0 = v.caret.unwrap();
    s.pointer_down(c0[0] + 2.0, c[1] + c[3] / 2.0, 2, Default::default());
    s.pointer_up();
    assert_eq!(s.selected_text(), "alpha");
    let id = xenify_wp_doc::review::add_comment(&mut s.ed, "why?").unwrap();
    let anchors = xenify_wp_doc::review::anchors(&s.ed.doc);
    let (_, a, b) = anchors.iter().find(|(i, _, _)| *i == id).unwrap();
    assert!(b.offset > a.offset, "{a:?} {b:?}");
}

#[test]
fn clicking_a_checklist_box_ticks_it() {
    let mut s = session("buy milk");
    s.toggle_list("checklist");
    let v = s.view();
    let c = v.caret.unwrap();
    let para = s.ed.doc.paragraphs().next().unwrap().id;
    // Left of the text, on the first line: the box.
    s.pointer_down(c[0] - 8.0, c[1] + c[3] / 2.0, 1, Default::default());
    s.pointer_up();
    assert!(s.ed.doc.para(para).unwrap().list.as_ref().unwrap().checked);
}

#[test]
fn copy_and_paste_keep_formatting_and_html_comes_in() {
    let mut s = session("plain words");
    let m = |shift, command| serde_json::from_value(serde_json::json!({"shift": shift, "command": command})).unwrap();
    // "plain" bold, copied, pasted at the end: still bold.
    s.key("Home", m(false, false));
    s.key("ArrowRight", m(true, true));
    s.toggle(xenify_wp_engine::Toggle::Bold);
    s.copy();
    let text = s.selected_text();
    s.key("End", m(false, false));
    s.paste(&text, "", false);
    let p = s.ed.doc.paragraphs().next().unwrap().clone();
    let at = p.text.rfind("plain").unwrap();
    assert_eq!(p.fmt_at(at + 1).bold, Some(true), "{:?}", p.runs);
    // HTML from another program keeps its emphasis.
    s.paste("Big", "<p><b>Big</b></p>", false);
    assert!(s.ed.doc.plain_text().contains("Big"));
    // Plain paste is just the text.
    s.paste("x", "<p><i>x</i></p>", true);
    assert!(s.ed.doc.plain_text().contains('x'));
}

#[test]
fn select_all_and_delete_leaves_an_empty_page() {
    use xenify_wp_doc::{NoteKind, notes, ops};
    let mut s = session("one\ntwo\nthree");
    let m = |shift, command| serde_json::from_value(serde_json::json!({"shift": shift, "command": command})).unwrap();
    // A bulleted and a numbered item, a footnote, an endnote and a text box.
    ops::toggle_list(&mut s.ed, xenify_wp_doc::ListDef::BULLETS).unwrap();
    s.key("End", m(false, true));
    notes::insert_note(&mut s.ed, NoteKind::Foot).unwrap();
    s.type_text("a footnote");
    s.key("a", m(false, true));
    let first = s.ed.doc.paragraphs().next().unwrap().id;
    s.ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(first, 3));
    notes::insert_note(&mut s.ed, NoteKind::End).unwrap();
    xenify_wp_doc::frames::insert_frame(&mut s.ed, 0, xenify_wp_doc::Frame::default()).unwrap();
    // Select all, delete.
    assert!(s.key("a", m(false, true)));
    assert!(s.key("Backspace", m(false, false)));
    let d = &s.ed.doc;
    let paras: Vec<_> = d.paragraphs().collect();
    assert_eq!(paras.len(), 1, "{:#?}", paras.iter().map(|p| (&p.text, &p.style, &p.list)).collect::<Vec<_>>());
    assert!(paras[0].text.is_empty() && paras[0].list.is_none());
    assert!(d.sections[0].frames.is_empty() && d.sections[0].notes.is_empty());
    // One undo brings it all back.
    s.undo();
    assert!(s.ed.doc.paragraphs().count() > 3);
}

#[test]
fn shift_enter_adds_more_to_a_list_item() {
    use xenify_wp_doc::ops;
    let mut s = session("First");
    let m = |shift| serde_json::from_value(serde_json::json!({"shift": shift})).unwrap();
    ops::toggle_list(&mut s.ed, xenify_wp_doc::ListDef::NUMBERS).unwrap();
    s.key("End", m(false));
    // Shift+Enter: more of item 1, no number; Enter: item 2.
    assert!(s.key("Enter", m(true)));
    s.type_text("still the first");
    assert!(s.key("Enter", m(false)));
    s.type_text("Second");
    let labels = xenify_wp_doc::lists::labels(&s.ed.doc);
    let paras: Vec<_> = s.ed.doc.paragraphs().collect();
    assert_eq!(labels.get(&paras[0].id).map(String::as_str), Some("1."));
    assert!(paras[1].list.as_ref().is_some_and(|l| l.follow) && !labels.contains_key(&paras[1].id));
    assert_eq!(labels.get(&paras[2].id).map(String::as_str), Some("2."));
    // Markdown keeps it: the follow-on paragraph is indented under item 1.
    let md = String::from_utf8(xenify_wp_io::save(&s.ed.doc, xenify_wp_io::Format::Markdown).unwrap()).unwrap();
    let back = xenify_wp_io::load(md.as_bytes(), xenify_wp_io::Format::Markdown).unwrap().doc;
    let again = xenify_wp_doc::lists::labels(&back);
    let bp: Vec<_> = back.paragraphs().collect();
    assert_eq!(bp.len(), 3, "{md}");
    assert!(bp[1].list.as_ref().is_some_and(|l| l.follow), "{md}");
    assert_eq!(again.get(&bp[2].id).map(String::as_str), Some("2."), "{md}");
}

#[test]
fn headings_can_be_numbered_by_their_style() {
    use xenify_wp_doc::ops;
    let mut s = session("Intro\nBody text\nDetails");
    let ids: Vec<_> = s.ed.doc.paragraphs().map(|p| p.id).collect();
    let at = |s: &mut Session, id| s.ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(id, 0));
    at(&mut s, ids[0]);
    ops::set_style(&mut s.ed, "heading1").unwrap();
    ops::number_headings(&mut s.ed, true).unwrap();
    // A heading made after that is numbered too, under the one before.
    at(&mut s, ids[2]);
    ops::set_style(&mut s.ed, "heading2").unwrap();
    let labels = xenify_wp_doc::lists::labels(&s.ed.doc);
    assert_eq!(labels.get(&ids[0]).map(String::as_str), Some("1"));
    assert_eq!(labels.get(&ids[2]).map(String::as_str), Some("1.1"));
    assert!(!labels.contains_key(&ids[1]));
    // Back to body text: no number.
    ops::set_style(&mut s.ed, "normal").unwrap();
    assert!(s.ed.doc.para(ids[2]).unwrap().list.is_none());
    // Enter at the end of a numbered heading: body text, not numbered.
    let end = s.ed.doc.para(ids[0]).unwrap().text.len();
    s.ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(ids[0], end));
    ops::split_paragraph(&mut s.ed).unwrap();
    let body = s.ed.doc.para(s.ed.sel.focus.para).unwrap();
    assert_eq!((body.style.as_str(), body.list.is_none()), ("normal", true));
    // A body style can be numbered too, and stop being.
    ops::set_style_list(&mut s.ed, "normal", Some(xenify_wp_doc::ListDef::NUMBERS)).unwrap();
    assert!(s.ed.doc.para(ids[1]).unwrap().list.is_some());
    ops::set_style_list(&mut s.ed, "normal", None).unwrap();
    assert!(s.ed.doc.para(ids[1]).unwrap().list.is_none());
    assert!(s.ed.doc.para(ids[0]).unwrap().list.is_some(), "the headings keep theirs");
}

#[test]
fn a_click_low_in_a_text_box_goes_into_it() {
    use xenify_wp_doc::{Frame, Wrap, frames, ops};
    let mut s =
        session("Some body text in the page that runs along for a while so that there is something under it.\nMore.");
    let id = frames::insert_frame(
        &mut s.ed,
        0,
        Frame { x: 72.0, y: 100.0, width: 451.0, height: 72.0, wrap: Wrap::TopBottom, ..Default::default() },
    )
    .unwrap();
    ops::insert_text(&mut s.ed, "Box one", None).unwrap();
    let inside = s.ed.sel.focus.para;
    s.ed.sel = xenify_wp_doc::Selection::caret(s.ed.doc.start());
    let fr = s.layout().frames.iter().find(|f| f.id == id).unwrap().rect;
    s.pointer_down(fr.center().x + 100.0, fr.y1 - 6.0, 1, Default::default());
    s.pointer_up();
    assert_eq!(s.ed.sel.focus.para, inside, "frame {fr:?}");
}
