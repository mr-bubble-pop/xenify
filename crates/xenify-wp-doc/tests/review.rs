//! Comments and tracked changes.
use xenify_wp_doc::review::{self, ChangeKind};
use xenify_wp_doc::{Document, Editor, Pos, Selection, ops};

fn editor(text: &str) -> Editor {
    let mut ed = Editor::new(Document::from_plain_text(text));
    ed.doc.user = "Sam".into();
    ed
}

fn first(ed: &Editor) -> xenify_wp_doc::NodeId {
    ed.doc.paragraphs().next().unwrap().id
}

/// The text with tracked changes shown as {+ins+} and {-del-}.
fn marked(ed: &Editor) -> String {
    let p = ed.doc.paragraphs().next().unwrap();
    p.run_ranges()
        .map(|(r, f)| match f.change.as_ref().map(|c| c.kind) {
            Some(ChangeKind::Inserted) => format!("{{+{}+}}", &p.text[r]),
            Some(ChangeKind::Deleted) => format!("{{-{}-}}", &p.text[r]),
            None => p.text[r].to_string(),
        })
        .collect()
}

#[test]
fn tracked_typing_and_deleting_mark_the_text() {
    let mut ed = editor("The cat sat.");
    let id = first(&ed);
    review::set_tracking(&mut ed, true).unwrap();
    // Delete "cat" with Backspace ×3 from after it, then type "dog".
    ed.sel = Selection::caret(Pos::new(id, 7));
    for _ in 0..3 {
        ops::backspace(&mut ed, false).unwrap();
    }
    assert_eq!(marked(&ed), "The {-cat-} sat.");
    assert_eq!(ed.sel.focus, Pos::new(id, 4));
    ops::insert_text(&mut ed, "dog", None).unwrap();
    assert_eq!(marked(&ed), "The {+dog+}{-cat-} sat.");
    // Deleting your own tracked insertion just removes it.
    ops::backspace(&mut ed, false).unwrap();
    assert_eq!(marked(&ed), "The {+do+}{-cat-} sat.");
    // Forward Delete marks and steps over.
    ed.sel = Selection::caret(Pos::new(id, 10));
    ops::delete_forward(&mut ed, false).unwrap();
    ops::delete_forward(&mut ed, false).unwrap();
    assert_eq!(marked(&ed), "The {+do+}{-cat-} {-sa-}t.");
    // The author and kind are recorded.
    let all = review::changes(&ed.doc);
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|c| c.change.author == "Sam" && c.change.date.ends_with('Z')));
}

#[test]
fn accept_and_reject() {
    let mut ed = editor("The cat sat.");
    let id = first(&ed);
    review::set_tracking(&mut ed, true).unwrap();
    ed.sel = Selection { anchor: Pos::new(id, 4), focus: Pos::new(id, 7) };
    ops::insert_text(&mut ed, "dog", None).unwrap();
    assert_eq!(marked(&ed), "The {+dog+}{-cat-} sat.");
    // Accept at the caret: only the change it's in.
    ed.sel = Selection::caret(Pos::new(id, 5));
    assert_eq!(review::decide(&mut ed, true, false).unwrap(), 1);
    assert_eq!(marked(&ed), "The dog{-cat-} sat.");
    // Undo brings the mark back; reject all restores the original wording.
    ed.undo();
    assert_eq!(review::decide(&mut ed, false, true).unwrap(), 2);
    assert_eq!(ed.doc.paragraphs().next().unwrap().text, "The cat sat.");
    assert!(review::changes(&ed.doc).is_empty());
}

#[test]
fn comments_anchor_reply_resolve_and_delete() {
    let mut ed = editor("Commas matter in prose.");
    let id = first(&ed);
    ed.sel = Selection { anchor: Pos::new(id, 0), focus: Pos::new(id, 6) };
    let c = review::add_comment(&mut ed, "Which commas?").unwrap();
    assert_eq!(review::anchors(&ed.doc), [(c, Pos::new(id, 0), Pos::new(id, 6))]);
    // Typing inside the anchor keeps it; typing at its end doesn't extend it.
    ed.sel = Selection::caret(Pos::new(id, 3));
    ops::insert_text(&mut ed, "x", None).unwrap();
    ed.sel = Selection::caret(Pos::new(id, 7));
    ops::insert_text(&mut ed, "y", None).unwrap();
    assert_eq!(review::anchors(&ed.doc), [(c, Pos::new(id, 0), Pos::new(id, 7))]);
    review::reply(&mut ed, c, "Oxford ones.").unwrap();
    review::set_resolved(&mut ed, c, true).unwrap();
    let thread = &ed.doc.meta.comments[0];
    assert_eq!((thread.author.as_str(), thread.replies.len(), thread.resolved), ("Sam", 1, true));
    assert!(review::open_comments(&ed.doc).is_empty());
    review::delete_comment(&mut ed, c).unwrap();
    assert!(ed.doc.meta.comments.is_empty() && review::anchors(&ed.doc).is_empty());
    // A comment with nothing selected takes the word at the caret.
    ed.sel = Selection::caret(Pos::new(id, 20));
    let c2 = review::add_comment(&mut ed, "Tone?").unwrap();
    let (_, a, b) = review::anchors(&ed.doc)[0];
    assert_eq!((c2, &ed.doc.paragraphs().next().unwrap().text[a.offset..b.offset]), (c2, "prose"));
}

#[test]
fn final_and_original_views() {
    let mut ed = editor("The cat sat.");
    let id = first(&ed);
    review::set_tracking(&mut ed, true).unwrap();
    ed.sel = Selection { anchor: Pos::new(id, 4), focus: Pos::new(id, 7) };
    ops::insert_text(&mut ed, "dog", None).unwrap();
    let text = |d: &Document| d.paragraphs().next().unwrap().text.clone();
    assert_eq!(text(&review::view_as(&ed.doc, review::Markup::Final)), "The dog sat.");
    assert_eq!(text(&review::view_as(&ed.doc, review::Markup::Original)), "The cat sat.");
    // The document itself is unchanged.
    assert_eq!(text(&ed.doc), "The dogcat sat.");
}

#[test]
fn tracked_paragraph_breaks() {
    let mut ed = editor("First part.\nSecond part.");
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    review::set_tracking(&mut ed, true).unwrap();
    // Backspace at the start of the second paragraph: the break is marked deleted, not removed.
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::backspace(&mut ed, false).unwrap();
    assert_eq!(ed.doc.paragraphs().count(), 2);
    let all = review::changes(&ed.doc);
    assert_eq!((all.len(), all[0].is_break, all[0].change.kind), (1, true, ChangeKind::Deleted));
    // Accepting it joins the paragraphs.
    review::decide(&mut ed, true, true).unwrap();
    let texts: Vec<String> = ed.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(texts, ["First part.Second part."]);
    // Enter while tracking: an inserted break; rejecting it joins them back.
    ed.sel = Selection::caret(Pos::new(ids[0], 11));
    ops::split_paragraph(&mut ed).unwrap();
    assert_eq!(ed.doc.paragraphs().count(), 2);
    let all = review::changes(&ed.doc);
    assert_eq!((all[0].is_break, all[0].change.kind), (true, ChangeKind::Inserted));
    // The new second paragraph doesn't carry the mark.
    assert!(ed.doc.paragraphs().nth(1).unwrap().props.break_change.is_none());
    review::decide(&mut ed, false, true).unwrap();
    assert_eq!(ed.doc.paragraphs().count(), 1);
    // Deleting your own inserted break just removes it.
    ed.sel = Selection::caret(Pos::new(ids[0], 5));
    ops::split_paragraph(&mut ed).unwrap();
    ops::backspace(&mut ed, false).unwrap();
    assert_eq!(ed.doc.paragraphs().count(), 1);
    assert!(review::changes(&ed.doc).is_empty());
    // Forward Delete at the end marks the break and steps over it.
    let mut ed = editor("One.\nTwo.");
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    review::set_tracking(&mut ed, true).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[0], 4));
    ops::delete_forward(&mut ed, false).unwrap();
    assert_eq!(ed.sel.focus, Pos::new(ids[1], 0));
    assert_eq!(review::changes(&ed.doc).len(), 1);
}
