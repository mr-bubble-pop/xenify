//! Comments (threads, replies, resolved) and tracked changes survive DOCX,
//! Markdown and Xenify's own format, and show in HTML.
use xenify_wp_doc::review::{self, ChangeKind};
use xenify_wp_doc::{Document, Editor, Pos, Selection, ops};
use xenify_wp_io::{Format, load, save};

fn build() -> Document {
    let mut ed = Editor::new(Document::from_plain_text("The cat sat on the mat.\nA second paragraph."));
    ed.doc.user = "Sam Lee".into();
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    // A comment across both paragraphs, with a reply; another resolved one.
    ed.sel = Selection { anchor: Pos::new(ids[0], 19), focus: Pos::new(ids[1], 8) };
    let c = review::add_comment(&mut ed, "Which mat?").unwrap();
    ed.doc.user = "Ana".into();
    review::reply(&mut ed, c, "The red one.").unwrap();
    ed.sel = Selection { anchor: Pos::new(ids[0], 0), focus: Pos::new(ids[0], 3) };
    let c2 = review::add_comment(&mut ed, "Fine.").unwrap();
    review::set_resolved(&mut ed, c2, true).unwrap();
    // Tracked: "cat" → "dog".
    review::set_tracking(&mut ed, true).unwrap();
    ed.sel = Selection { anchor: Pos::new(ids[0], 4), focus: Pos::new(ids[0], 7) };
    ops::insert_text(&mut ed, "dog", None).unwrap();
    ed.doc
}

/// Paragraph text with {+ins:who+}, {-del:who-} and [comment ids] spelt out, the threads, and tracking.
fn shape(d: &Document) -> Vec<String> {
    let mut out: Vec<String> = d
        .paragraphs()
        .map(|p| {
            p.run_ranges()
                .map(|(r, f)| {
                    let t = &p.text[r];
                    let t = match &f.change {
                        Some(c) if c.kind == ChangeKind::Inserted => format!("{{+{t}:{}+}}", c.author),
                        Some(c) => format!("{{-{t}:{}-}}", c.author),
                        None => t.to_string(),
                    };
                    let n: Vec<&str> = f
                        .comments
                        .iter()
                        .map(|c| d.meta.comments.iter().find(|x| x.id == *c).unwrap().text.as_str())
                        .collect();
                    if n.is_empty() { t } else { format!("[{t}]{n:?}") }
                })
                .collect::<String>()
        })
        .collect();
    let mut threads: Vec<_> = d.meta.comments.iter().collect();
    threads.sort_by_key(|c| c.text.clone());
    for c in threads {
        let replies: Vec<String> = c.replies.iter().map(|r| format!("{}: {}", r.author, r.text)).collect();
        out.push(format!(
            "{}: {} {:?} resolved={} dated={}",
            c.author,
            c.text,
            replies,
            c.resolved,
            !c.date.is_empty()
        ));
    }
    out.push(format!("tracking={}", d.meta.track_changes));
    out
}

#[test]
fn comments_and_changes_round_trip() {
    let doc = build();
    let want = shape(&doc);
    assert_eq!(want[0], "[The][\"Fine.\"] {+dog:Ana+}{-cat:Ana-} sat on the [mat.][\"Which mat?\"]", "{want:#?}");
    for format in [Format::Igdoc, Format::Docx, Format::Markdown] {
        let bytes = save(&doc, format).unwrap();
        let back = load(&bytes, format).unwrap();
        assert_eq!(
            shape(&back.doc),
            want,
            "{format:?}\n{}",
            if format == Format::Markdown { String::from_utf8_lossy(&bytes).to_string() } else { String::new() }
        );
        assert!(back.warnings.is_empty(), "{format:?}: {:?}", back.warnings);
    }
}

#[test]
fn markdown_uses_pandoc_track_changes_syntax() {
    let md = String::from_utf8(save(&build(), Format::Markdown).unwrap()).unwrap();
    assert!(md.contains("[dog]{.insertion author=Ana date="), "{md}");
    assert!(md.contains("[cat]{.deletion author=Ana"), "{md}");
    assert!(md.contains("[Which mat?]{.comment-start id="), "{md}");
    assert!(md.contains("[The red one.]{.comment-start id="), "{md}");
    assert!(md.contains(".comment-end id="), "{md}");
}

#[test]
fn html_shows_changes_and_comments() {
    let html = String::from_utf8(save(&build(), Format::Html).unwrap()).unwrap();
    assert!(html.contains("<ins title=\"Ana\"") && html.contains("<del title=\"Ana\""), "{html}");
    assert!(html.contains("class=\"comment\"") && html.contains("The red one."));
    // The resolved comment isn't listed.
    assert!(!html.contains("Fine."));
}

#[test]
fn tracked_breaks_round_trip() {
    let mut ed = Editor::new(Document::from_plain_text("One.\nTwo.\nThree."));
    ed.doc.user = "Ana".into();
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    review::set_tracking(&mut ed, true).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::backspace(&mut ed, false).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[2], 2));
    ops::split_paragraph(&mut ed).unwrap();
    let shape = |d: &Document| {
        d.paragraphs()
            .map(|p| format!("{} {:?}", p.text, p.props.break_change.as_ref().map(|c| (c.kind, c.author.clone()))))
            .collect::<Vec<_>>()
    };
    let want = shape(&ed.doc);
    assert_eq!(want.iter().filter(|s| s.contains("Ana")).count(), 2, "{want:?}");
    for format in [Format::Igdoc, Format::Docx, Format::Markdown] {
        let bytes = save(&ed.doc, format).unwrap();
        assert_eq!(shape(&load(&bytes, format).unwrap().doc), want, "{format:?} {}", String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn change_log_round_trips_and_shows_as_a_table() {
    use xenify_wp_doc::{TocKind, changelog, tables};
    let mut ed = Editor::new(Document::from_plain_text("Report text."));
    ed.doc.user = "Sam Lee".into();
    changelog::record(&mut ed, "First draft").unwrap();
    ed.doc.user = "Ana".into();
    changelog::record(&mut ed, "Edited “Method” (12 words added)").unwrap();
    let end = ed.doc.paragraphs().next().unwrap().id;
    ed.sel = Selection::caret(Pos::new(end, 12));
    tables::insert_generated(&mut ed, TocKind::ChangeLog).unwrap();
    let doc = ed.doc;
    let shape = |d: &Document| {
        let log: Vec<String> =
            d.meta.changelog.iter().map(|e| format!("{} {} {}", e.author, e.summary, !e.date.is_empty())).collect();
        let blocks = d.sections[0]
            .blocks
            .iter()
            .filter(|b| matches!(b, xenify_wp_doc::Block::Toc(t) if t.kind == TocKind::ChangeLog))
            .count();
        (log, blocks)
    };
    for format in [Format::Igdoc, Format::Docx, Format::Markdown] {
        let bytes = save(&doc, format).unwrap();
        assert_eq!(shape(&load(&bytes, format).unwrap().doc), shape(&doc), "{format:?}");
    }
    // HTML and the page show it as a table.
    let html = String::from_utf8(save(&doc, Format::Html).unwrap()).unwrap();
    assert!(html.contains("<table") && html.contains("Sam Lee") && html.contains("First draft"), "{html}");
    let l = xenify_wp_layout::Engine::new(xenify_wp_layout::Fonts::bundled_only()).layout(&doc);
    assert!(l.cells.len() >= 9, "3 rows × 3 cells");
}
