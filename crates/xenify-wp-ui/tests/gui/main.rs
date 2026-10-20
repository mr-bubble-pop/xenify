//! Headless GUI tests: the real app in egui_kittest, driven by egui events
//! (no window, no GPU).

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use xenify_wp_doc::StyleSheet;

mod a11y;

type Harness = egui_kittest::Harness<'static, xenify_wp_ui::WriteApp>;

fn harness() -> Harness {
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| xenify_wp_ui::testing::app(cc));
    h.run_steps(2);
    h
}

fn texts(h: &Harness) -> Vec<String> {
    h.state().editor.doc.paragraphs().map(|p| p.text.clone()).collect()
}

/// Presses a key (with modifiers) and runs a frame.
fn press(h: &mut Harness, m: Modifiers, key: Key) {
    h.key_press_modifiers(m, key);
    h.run();
}

fn type_text(h: &mut Harness, s: &str) {
    for c in s.chars() {
        if c == '\n' {
            press(h, Modifiers::NONE, Key::Enter);
        } else {
            h.event(Event::Text(c.to_string()));
            h.run();
        }
    }
    h.run();
}

#[test]
fn typing_editing_and_undo() {
    let mut h = harness();
    type_text(&mut h, "Hello world\nSecond line");
    assert_eq!(texts(&h), ["Hello world", "Second line"]);
    press(&mut h, Modifiers::NONE, Key::Backspace);
    press(&mut h, Modifiers::COMMAND, Key::Backspace);
    assert_eq!(texts(&h), ["Hello world", "Second "]);
    // Home, then Backspace joins the paragraphs.
    press(&mut h, Modifiers::NONE, Key::Home);
    press(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(texts(&h), ["Hello worldSecond "]);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(texts(&h), ["Hello world", "Second "]);
    assert!(h.state().editor.is_dirty());
}

#[test]
fn autocorrect_as_you_type_and_undo() {
    let mut h = harness();
    type_text(&mut h, "I saw teh \"cat\" (c) ");
    assert_eq!(texts(&h), ["I saw the \u{201c}cat\u{201d} © "]);
    // Ctrl+Z straight after a correction puts back what was typed.
    type_text(&mut h, "adn ");
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(texts(&h), ["I saw the \u{201c}cat\u{201d} © adn "]);
    // The list can be changed in Settings; removing an entry stops it.
    h.state_mut().view.autocorrect.entries.retain(|(from, _)| from != "teh");
    type_text(&mut h, "teh ");
    assert!(texts(&h)[0].ends_with("adn teh "));
}

#[test]
fn arrows_and_selection() {
    let mut h = harness();
    type_text(&mut h, "abc\ndef");
    press(&mut h, Modifiers::NONE, Key::ArrowUp);
    press(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    press(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    let ed = &h.state().editor;
    assert_eq!(ed.doc.text_in(ed.sel), "bc");
    type_text(&mut h, "X");
    assert_eq!(texts(&h), ["aX", "def"]);
    press(&mut h, Modifiers::COMMAND, Key::A);
    type_text(&mut h, "all gone");
    assert_eq!(texts(&h), ["all gone"]);
}

#[test]
fn bold_toggle_and_heading_shortcut() {
    let mut h = harness();
    type_text(&mut h, "plain ");
    press(&mut h, Modifiers::COMMAND, Key::B);
    type_text(&mut h, "bold");
    let p = h.state().editor.doc.paragraphs().next().unwrap().clone();
    assert_eq!(p.runs.len(), 2);
    assert_eq!(p.runs[1].fmt.bold, Some(true));
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::Num2);
    let p = h.state().editor.doc.paragraphs().next().unwrap().clone();
    assert_eq!(p.style, StyleSheet::heading(2));
    // The toolbar's style picker shows it.
    h.get_by_value("Heading 2");
}

#[test]
fn clicking_places_the_caret() {
    let mut h = harness();
    type_text(&mut h, "first paragraph\nsecond paragraph");
    // Click at the very start of the document (left edge of the first line).
    let o = h.state().view.doc_origin;
    h.event(Event::PointerMoved(o + egui::vec2(1.0, 6.0)));
    h.event(Event::PointerButton {
        pos: o + egui::vec2(1.0, 6.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerButton {
        pos: o + egui::vec2(1.0, 6.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    type_text(&mut h, ">");
    assert_eq!(texts(&h), [">first paragraph", "second paragraph"]);
}

#[test]
fn paste_and_ime() {
    let mut h = harness();
    h.event(Event::Paste("one\ntwo".into()));
    h.run();
    h.event(Event::Ime(egui::ImeEvent::Preedit { text: "にほ".into(), active_range_chars: None }));
    h.run();
    assert_eq!(h.state().view.preedit.as_deref(), Some("にほ"));
    assert_eq!(texts(&h), ["one", "two"], "composition isn't in the document yet");
    h.event(Event::Ime(egui::ImeEvent::Commit("日本".into())));
    h.run();
    assert_eq!(texts(&h), ["one", "two日本"]);
    assert!(h.state().view.preedit.is_none());
}

#[test]
fn save_and_open_round_trip() {
    let dir = std::env::temp_dir().join(format!("igw-gui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.igdoc");
    let mut h = harness();
    type_text(&mut h, "Saved text");
    assert!(h.state_mut().save_to(&path));
    assert!(!h.state().editor.is_dirty());
    let mut h2 = harness();
    h2.state_mut().open_path(&path);
    h2.run();
    assert_eq!(texts(&h2), ["Saved text"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn new_documents_open_in_tabs_and_closing_asks() {
    let mut h = harness();
    type_text(&mut h, "draft");
    // New: a second tab in front; the draft waits in the first.
    press(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(h.state().tabs.len(), 2);
    assert_eq!(texts(&h), [""]);
    type_text(&mut h, "second");
    press(&mut h, Modifiers::COMMAND, Key::Tab);
    assert_eq!(texts(&h), ["draft"]);
    // Closing a tab with unsaved changes asks first.
    press(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Don't save").click();
    h.run();
    assert_eq!(h.state().tabs.len(), 1);
    assert_eq!(texts(&h), ["second"]);
    // A blank, untouched document is replaced rather than kept as a tab.
    press(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Don't save").click();
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(h.state().tabs.len(), 1);
}

fn labels(h: &Harness) -> Vec<String> {
    let d = &h.state().editor.doc;
    let l = xenify_wp_doc::lists::labels(d);
    d.paragraphs().map(|p| l.get(&p.id).cloned().unwrap_or_default()).collect()
}

#[test]
fn lists_from_shortcuts_toolbar_and_tab() {
    let mut h = harness();
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Num7);
    type_text(&mut h, "one\ntwo\nthree");
    assert_eq!(labels(&h), ["1.", "2.", "3."]);
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(labels(&h), ["1.", "2.", "a."]);
    press(&mut h, Modifiers::SHIFT, Key::Tab);
    // Enter twice ends the list.
    press(&mut h, Modifiers::NONE, Key::Enter);
    press(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "after");
    assert_eq!(labels(&h), ["1.", "2.", "3.", ""]);
    // The toolbar button turns the caret paragraph into a bullet.
    h.get_by_label("Bulleted list").click();
    h.run();
    assert_eq!(labels(&h)[3], "•");
}

#[test]
fn copy_paste_keeps_formatting() {
    let mut h = harness();
    type_text(&mut h, "x ");
    press(&mut h, Modifiers::COMMAND, Key::B);
    type_text(&mut h, "bold");
    // Select "bold", copy, move to the end and paste.
    for _ in 0..4 {
        press(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    }
    h.event(Event::Copy);
    h.run();
    press(&mut h, Modifiers::NONE, Key::End);
    h.event(Event::Paste("bold".into()));
    h.run();
    let p = h.state().editor.doc.paragraphs().next().unwrap().clone();
    assert_eq!(p.text, "x boldbold");
    assert!(p.runs.last().unwrap().fmt.bold == Some(true));
}

#[test]
fn markdown_is_the_default_and_warns_about_losses() {
    let dir = std::env::temp_dir().join(format!("igw-gui-md-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness();
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::Num1);
    type_text(&mut h, "Title\nbody text");
    let md = dir.join("doc.md");
    assert!(h.state_mut().request_save(&md));
    let text = std::fs::read_to_string(&md).unwrap();
    assert_eq!(text, "# Title\n\nbody text\n");
    // An outline list can't be stored in Markdown: ask first.
    let _ = xenify_wp_doc::ops::toggle_list(&mut h.state_mut().editor, xenify_wp_doc::ListDef::OUTLINE);
    assert!(!h.state_mut().request_save(&md));
    h.run();
    h.get_by_label("Save as .igdoc instead").click();
    h.run();
    assert!(dir.join("doc.igdoc").exists());
    assert_eq!(h.state().path.as_deref(), Some(dir.join("doc.igdoc").as_path()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn page_breaks_and_pageless_view() {
    let mut h = harness();
    type_text(&mut h, "first page");
    press(&mut h, Modifiers::COMMAND, Key::Enter);
    type_text(&mut h, "second page");
    assert_eq!(h.state().view.layout.pages.len(), 2);
    h.get_by_label("Page 2 of 2");
    h.state_mut().set_pageless(true);
    h.run();
    assert_eq!(h.state().view.layout.pages.len(), 1);
    // Undo removes the page break.
    h.state_mut().set_pageless(false);
    for _ in 0..3 {
        press(&mut h, Modifiers::COMMAND, Key::Z);
    }
    assert_eq!(h.state().view.layout.pages.len(), 1);
}

/// A small PNG made on the fly (4×2 pixels).
fn tiny_png() -> Vec<u8> {
    let mut out = Vec::new();
    let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 30, 30, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

#[test]
fn tables_and_pictures_in_the_app() {
    let dir = std::env::temp_dir().join(format!("igw-gui-tbl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pic = dir.join("pic.png");
    std::fs::write(&pic, tiny_png()).unwrap();
    let mut h = harness();
    type_text(&mut h, "Intro");
    let _ = xenify_wp_doc::tables::insert_table(&mut h.state_mut().editor, 2, 2);
    h.run();
    type_text(&mut h, "a");
    press(&mut h, Modifiers::NONE, Key::Tab);
    type_text(&mut h, "b");
    let doc = &h.state().editor.doc;
    let t = doc.sections[0]
        .blocks
        .iter()
        .find_map(|b| if let xenify_wp_doc::Block::Table(t) = b { Some(t) } else { None })
        .unwrap();
    assert_eq!(doc.para(xenify_wp_doc::tables::first_para(t, 0, 1).unwrap()).unwrap().text, "b");
    // A picture, embedded and saved next to the Markdown file.
    let (key, w, hgt) = xenify_wp_ui::images::embed(&mut h.state_mut().editor.doc, &pic).unwrap();
    assert_eq!((w, hgt), (3.0, 1.5));
    let _ = xenify_wp_doc::tables::insert_image(&mut h.state_mut().editor, &key, w, hgt);
    h.run();
    assert_eq!(h.state().view.layout.images.len(), 1);
    let md = dir.join("out").join("doc.md");
    std::fs::create_dir_all(md.parent().unwrap()).unwrap();
    assert!(h.state_mut().request_save(&md));
    assert!(dir.join("out/media/pic.png").exists());
    let mut h2 = harness();
    h2.state_mut().open_path(&md);
    h2.run();
    assert!(h2.state().editor.doc.assets.contains_key("media/pic.png"));
    assert_eq!(h2.state().view.layout.images.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn export_formats() {
    let dir = std::env::temp_dir().join(format!("igw-gui-exp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness();
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::Num1);
    type_text(&mut h, "Export me\nbody");
    for (name, f) in [
        ("a.pdf", xenify_wp_io::Format::Pdf),
        ("a.docx", xenify_wp_io::Format::Docx),
        ("a.html", xenify_wp_io::Format::Html),
    ] {
        assert!(h.state_mut().export_to(&dir.join(name), f), "{name}");
    }
    assert!(std::fs::read(dir.join("a.pdf")).unwrap().starts_with(b"%PDF"));
    // The document itself is still unsaved and untitled.
    assert!(h.state().path.is_none());
    let mut h2 = harness();
    h2.state_mut().open_path(&dir.join("a.docx"));
    h2.run();
    let texts: Vec<_> = h2.state().editor.doc.paragraphs().map(|p| (p.style.clone(), p.text.clone())).collect();
    assert_eq!(texts, [("heading1".to_string(), "Export me".to_string()), ("normal".into(), "body".into())]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn toolbar_state_after_correction() {
    let dir = std::env::temp_dir().join(format!("igw-gui-tb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = dir.join("p.md");
    std::fs::write(&md, "# Head\n\nThis is an test of the speling checker.\n").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&md);
    h.run();
    let p = h.state().editor.doc.paragraphs().nth(1).unwrap().id;
    let _ = xenify_wp_doc::ops::replace_text(&mut h.state_mut().editor, p, 23..30, "spelling", "Correction");
    h.run();
    let st = h.state();
    let fmt = xenify_wp_doc::ops::common_format(&st.editor);
    eprintln!("sel {:?} fmt {:?} pending {:?}", st.editor.sel, fmt, st.view.pending_fmt);
    assert!(!st.view.is_on(&st.editor, xenify_wp_ui::view::Toggle::Italic));
    std::fs::remove_dir_all(&dir).unwrap();
}

fn fake_ai(h: &mut Harness) {
    use serde_json::json;
    let fake: xenify_wp_ui::assistant::Transport = std::sync::Arc::new(
        |body: &serde_json::Value, on_text: &mut dyn FnMut(&str)| {
            let user =
                body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap_or("").to_string();
            if body["tool_choice"]["function"]["name"] == "report_issues" {
                let label = user.split('[').nth(1).and_then(|r| r.split(']').next()).unwrap_or("p1").to_string();
                let args = json!({"issues": [{"paragraph": label, "quote": "was wrote", "category": "grammar", "severity": "high", "explanation": "Wrong verb form.", "replacement": "was written"}]});
                return Ok(
                    json!({"choices": [{"message": {"tool_calls": [{"id": "1", "type": "function", "function": {"name": "report_issues", "arguments": args.to_string()}}]}}]}),
                );
            }
            if body["tools"].is_array() {
                let args = json!({"quote": "the team", "replacement": "our team", "reason": "More personal."});
                return Ok(
                    json!({"choices": [{"message": {"content": "Try this.", "tool_calls": [{"id": "2", "type": "function", "function": {"name": "propose_edit", "arguments": args.to_string()}}]}}]}),
                );
            }
            if user.contains("Continue the text") {
                on_text("More ");
                on_text("words.");
                return Ok(json!({"choices": [{"message": {"content": "More words."}}]}));
            }
            Ok(json!({"choices": [{"message": {"content": "The report was written by our team."}}]}))
        },
    );
    let st = h.state_mut();
    st.assistant.transport = Some(fake);
    st.assistant.consent = true;
}

fn wait_ai(h: &mut Harness) {
    for _ in 0..200 {
        // One frame at a time: the spinner keeps asking for more while it works.
        h.step();
        if !h.state().assistant.busy() {
            // Let the answer show.
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("AI request didn't finish");
}

#[test]
fn ai_review_rewrite_chat_and_continue() {
    let mut h = harness();
    type_text(&mut h, "The report was wrote by the team.");
    fake_ai(&mut h);
    let ctx = h.ctx.clone();
    // Review: one suggestion, accepted.
    xenify_wp_ui::assistant::review(h.state_mut(), &ctx, true);
    wait_ai(&mut h);
    assert_eq!(h.state().assistant.suggestions.len(), 1, "{:?}", h.state().assistant.status);
    h.state_mut().show_ai = true;
    h.run();
    h.get_by_label("Accept").click();
    h.run();
    assert_eq!(texts(&h), ["The report was written by the team."]);
    // Rewrite the paragraph (caret inside it): a diff, accepted as one undo step.
    xenify_wp_ui::assistant::rewrite(h.state_mut(), &ctx, "Friendlier".into());
    wait_ai(&mut h);
    assert!(h.state().assistant.rewrite.is_some());
    xenify_wp_ui::assistant::accept_rewrite(h.state_mut());
    h.run();
    assert_eq!(texts(&h), ["The report was written by our team."]);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(texts(&h), ["The report was written by the team."]);
    // Continue writing: streamed, then inserted.
    let end = h.state().editor.doc.end();
    h.state_mut().editor.sel = xenify_wp_doc::Selection::caret(end);
    xenify_wp_ui::assistant::continue_writing(h.state_mut(), &ctx);
    wait_ai(&mut h);
    assert_eq!(h.state().assistant.ghost.as_ref().map(|g| g.1.as_str()), Some("More words."));
    // It's further down the Writing tab: scroll there with the mouse wheel.
    h.event(Event::PointerMoved(egui::pos2(930.0, 600.0)));
    for _ in 0..6 {
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        // Scrolling animates: step frames rather than waiting for it to settle.
        h.run_steps(3);
    }
    h.get_by_label("Insert text").click();
    h.run_steps(3);
    assert_eq!(texts(&h), ["The report was written by the team.More words."]);
}

/// A real request to OpenRouter (needs a key; run with `--ignored`).
#[test]
#[ignore]
fn live_openrouter_review() {
    let mut h = harness();
    type_text(&mut h, "Their going to the libary tomorow, and the report were finished by them yesterday.");
    h.state_mut().assistant.consent = true;
    let ctx = h.ctx.clone();
    xenify_wp_ui::assistant::review(h.state_mut(), &ctx, true);
    for _ in 0..1200 {
        h.run();
        if !h.state().assistant.busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let a = &h.state().assistant;
    eprintln!("model: {}\nstatus: {:?}", a.settings.writer_model(), a.status);
    let text = &h.state().editor.doc.paragraphs().next().unwrap().text;
    for s in &a.suggestions {
        eprintln!("  [{}] {:?} -> {:?}: {}", s.category, &text[s.range.clone()], s.replacement, s.explanation);
    }
    assert!(!a.suggestions.is_empty(), "{:?}", a.status);
}

#[test]
fn mail_merge_preview_and_finish() {
    let dir = std::env::temp_dir().join(format!("igw-gui-merge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("people.csv"), "first,last\nAda,Lovelace\nAlan,Turing\n").unwrap();
    std::fs::write(
        dir.join("letter.md"),
        "---\nmerge:\n  source: people.csv\n  file-name: \"{{last}}\"\n---\n\nDear {{first}} {{last}},\n",
    )
    .unwrap();
    let mut h = harness();
    h.state_mut().open_path(&dir.join("letter.md"));
    h.run();
    assert_eq!(h.state().merge.records.len(), 2);
    // Tools › Mail merge… opens the dialog.
    h.state_mut().show_merge = true;
    h.run_steps(2);
    h.get_by_label("Preview results").click();
    h.run();
    let shown: Vec<_> = h.state().merge.preview.as_ref().unwrap().1.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(shown, ["Dear Ada Lovelace,"]);
    // The template itself is unchanged, and typing doesn't edit the preview.
    type_text(&mut h, "x");
    assert_eq!(texts(&h), ["Dear {{first}} {{last}},"]);
    h.get_by_label("▶").click();
    h.run();
    let shown: Vec<_> = h.state().merge.preview.as_ref().unwrap().1.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(shown, ["Dear Alan Turing,"]);
    // Finish & merge to Markdown files.
    let out = dir.join("out");
    let st = h.state_mut();
    st.merge.format = Some(xenify_wp_io::Format::Markdown);
    st.merge.folder = out.display().to_string();
    xenify_wp_ui::merge_panel::start(st);
    for _ in 0..400 {
        h.run();
        if !h.state().merge.busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(std::fs::read_to_string(out.join("Turing.md")).unwrap(), "Dear Alan Turing,\n");
    assert!(out.join("Lovelace.md").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn text_boxes_and_templates() {
    let mut h = harness();
    type_text(&mut h, "Body text");
    h.state_mut().insert_text_box();
    h.run();
    let id = h.state().view.selected_frame.expect("the new box is selected");
    // The caret is in the box: typing fills it.
    h.state_mut().view.selected_frame = None;
    type_text(&mut h, "Boxed");
    assert_eq!(texts(&h), ["Body text", "Boxed"]);
    let x0 = h.state().editor.doc.frame(id).unwrap().x;
    // Selected, arrows nudge it; Delete removes it; undo brings it back.
    h.state_mut().view.selected_frame = Some(id);
    press(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    assert_eq!(h.state().editor.doc.frame(id).unwrap().x, x0 - 10.0);
    press(&mut h, Modifiers::NONE, Key::Delete);
    assert!(h.state().editor.doc.frame(id).is_none());
    assert_eq!(texts(&h), ["Body text"]);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().editor.doc.frame(id).is_some());
    // New from template: the invoice has its boxes and a repeating row.
    h.state_mut().open_template(1);
    h.run();
    // In a new tab: the document with the text box stays open.
    assert_eq!(h.state().tabs.len(), 2);
    let doc = &h.state().editor.doc;
    assert_eq!(doc.sections[0].frames.len(), 4);
    assert!(h.state().path.is_none(), "templates open as new documents");
    assert_eq!(h.state().view.layout.frames.len(), 4);
}

/// Runs frames until the proofing worker has answered (or a few seconds pass).
fn wait_for_proofing(h: &mut Harness, until: impl Fn(&xenify_wp_ui::WriteApp) -> bool) {
    for _ in 0..400 {
        h.step();
        if until(h.state()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn rules_are_grouped_and_searchable() {
    let mut h = harness();
    h.state_mut().show_proofing = true;
    wait_for_proofing(&mut h, |a| !a.proofing.rules.is_empty());
    // In Settings › Spelling and grammar › Advanced.
    h.state_mut().open_settings(xenify_wp_ui::settings::Section::Proofing);
    h.run();
    h.get_by_label("Advanced").click();
    h.run();
    h.get_by_label("Rules (all profiles)").click();
    h.run();
    // Groups with counts, not one long list.
    assert!(h.query_by_label_contains("Sound-alikes and mix-ups (").is_some());
    assert!(h.query_by_label("Accept except").is_none(), "groups start closed");
    // Searching opens the groups that match.
    h.state_mut().proofing.panel.rule_search = "we except credit".into();
    h.run();
    h.get_by_label("Accept except").scroll_to_me();
    h.run();
    h.get_by_label("Accept except").click();
    h.run();
    assert!(h.state().proofing.settings.disabled_rules.contains("AcceptExcept"));
    assert!(h.query_by_label("Turned off (1)").is_some(), "turned-off rules are listed first");

    // Language: a fixed choice, or follow the computer.
    assert!(!h.state().proofing.dialect_auto);
    h.get_by_label("Language").scroll_to_me();
    h.run();
    h.get_by_label("Language").click();
    h.run();
    h.get_by_label_contains("Same as this computer").click();
    h.run();
    assert!(h.state().proofing.dialect_auto);
    assert_eq!(h.state().proofing.settings.dialect, xenify_wp_check::Dialect::from_locale());
}

#[test]
fn writing_profiles_change_what_is_pointed_out() {
    // Keep custom profiles out of the real config folder.
    let dir = std::env::temp_dir().join(format!("xenify-write-gui-profiles-{}", std::process::id()));
    let mut h = harness();
    h.state_mut().proofing.profiles = xenify_wp_check::Profiles::load(Some(dir.clone()));
    h.state_mut().show_proofing = true;
    type_text(&mut h, "idk but its raining, between you and I.");
    let rules = |a: &xenify_wp_ui::WriteApp| a.proofing.issues.iter().map(|i| i.rule.clone()).collect::<Vec<_>>();
    wait_for_proofing(&mut h, |a| rules(a).iter().any(|r| r == "ItsForItIs"));
    let r = rules(h.state());
    assert!(r.contains(&"ItsForItIs".to_string()) && r.contains(&"PronounCaseInPairs".to_string()), "{r:?}");
    let idk = |a: &xenify_wp_ui::WriteApp| {
        a.proofing.issues.iter().find(|i| i.feature == Some(xenify_wp_check::Feature::Texting)).cloned()
    };
    assert!(idk(h.state()).is_some_and(|i| i.strong), "Standard flags texting: {r:?}");

    // Choose a profile for this document from the Spelling tab's list.
    h.get_by_label("Writing profile").click();
    h.run();
    h.get_by_label("Workplace email").click();
    wait_for_proofing(&mut h, |a| a.editor.doc.meta.proofing_profile.is_some());
    assert_eq!(h.state().editor.doc.meta.proofing_profile.as_deref(), Some("workplace-email"));
    // Casual chat (further down the list) allows texting style.
    h.state_mut().editor.doc.meta.proofing_profile = Some("casual".into());
    wait_for_proofing(&mut h, |a| {
        a.proofing.settings.profile.id == "casual" && idk(a).is_none() && !a.proofing.issues.is_empty()
    });
    assert_eq!(h.state().editor.doc.meta.proofing_profile.as_deref(), Some("casual"));
    assert!(idk(h.state()).is_none(), "casual allows texting");
    assert!(rules(h.state()).contains(&"ItsForItIs".to_string()), "real slips are still found");

    // Back to Professional social post: texting is noted, gently.
    h.state_mut().editor.doc.meta.proofing_profile = Some("professional-social".into());
    wait_for_proofing(&mut h, |a| idk(a).is_some());
    let note = idk(h.state()).unwrap();
    assert!(!note.strong && note.message.starts_with("Informal (texting style)"), "{}", note.message);
    assert_eq!(note.suggestions, ["I don't know"]);

    // "Why?" explains it; "Allow" makes a custom copy of the profile that allows it.
    h.state_mut().proofing.why = Some(note.clone());
    h.step();
    h.step();
    assert!(h.query_by_label_contains("Shorthand like u, rn, idk").is_some(), "the Why? window explains the feature");
    xenify_wp_ui::proofing::edit_profile(h.state_mut(), |p| {
        p.features.insert(xenify_wp_check::Feature::Texting, xenify_wp_check::Level::Allow);
    });
    wait_for_proofing(&mut h, |a| {
        a.proofing.settings.profile.name == "Professional social post (mine)" && idk(a).is_none()
    });
    assert!(idk(h.state()).is_none());
    assert!(dir.join("professional-social-post-mine.toml").exists());

    // The profile editor opens from Settings.
    h.state_mut().open_settings(xenify_wp_ui::settings::Section::Proofing);
    h.run();
    h.get_by_label("Edit profiles…").click();
    h.run();
    h.get_by_label("Writing profiles");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mistake_spotter_finds_and_suggests() {
    // Needs a model folder: XENIFY_SPOTTER=target/ml/spotter-v2-xsmall
    let Some(dir) = std::env::var_os("XENIFY_SPOTTER") else { return };
    let mut h = harness();
    h.state_mut().show_proofing = true;
    h.state_mut().proofing.spotter =
        xenify_wp_check::SpotterPrefs { enabled: true, path: dir.to_string_lossy().into(), threshold: 0.9 };
    // A mistake the rules don't catch (they'd otherwise mark it first).
    type_text(&mut h, "I have pet dog at home.");
    let spotted =
        |a: &xenify_wp_ui::WriteApp| a.proofing.issues.iter().find(|i| i.rule.starts_with("Spotter:")).cloned();
    wait_for_proofing(&mut h, |a| spotted(a).is_some());
    let issue = spotted(h.state()).expect("the spotter marks the sentence");
    assert!(issue.suggestions.iter().any(|s| s.contains("a pet")), "{issue:?}");
    assert!(h.state().proofing.spotter_status.starts_with("Loaded"), "{}", h.state().proofing.spotter_status);
    // "Why?" explains the code.
    h.state_mut().proofing.why = Some(issue);
    h.step();
    h.step();
    assert!(h.query_by_label_contains("Most single things in English need a, an or the").is_some());
}

#[test]
fn light_and_dark_appearance() {
    use xenify_wp_ui::theme::Appearance;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().set_appearance(&ctx, Appearance::Dark);
    h.run();
    assert!(h.ctx.global_style().visuals.dark_mode);
    assert_eq!(h.state().appearance(), Appearance::Dark);
    h.state_mut().set_appearance(&ctx, Appearance::Light);
    h.run();
    assert!(!h.ctx.global_style().visuals.dark_mode);
    // The toolbar's icon buttons are found by their names.
    h.get_by_label("Bold");
    h.get_by_label("Align left");
    h.get_by_label("Insert picture");
}

#[test]
fn markdown_shortcuts_while_typing() {
    let mut h = harness();
    type_text(&mut h, "## Plan\n* milk\neggs");
    let styles: Vec<String> = h.state().editor.doc.paragraphs().map(|p| p.style.clone()).collect();
    assert_eq!(texts(&h), ["Plan", "milk", "eggs"]);
    assert_eq!(styles[0], xenify_wp_doc::StyleSheet::heading(2));
    assert_eq!(labels(&h)[1..], ["•", "•"]);
    // Tab moves "eggs" into a sub-list.
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(labels(&h)[2], "◦");
    // Ctrl+Z right after a shortcut gives back the characters.
    press(&mut h, Modifiers::NONE, Key::Enter);
    press(&mut h, Modifiers::NONE, Key::Enter);
    press(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "1. ");
    assert_eq!(labels(&h).last().unwrap(), "1.");
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(texts(&h).last().unwrap(), "1. ");
    // Outside lists, Tab types a tab.
    press(&mut h, Modifiers::COMMAND, Key::A);
    type_text(&mut h, "a");
    press(&mut h, Modifiers::NONE, Key::Tab);
    type_text(&mut h, "b");
    assert_eq!(texts(&h), ["a\tb"]);
}

#[test]
fn pasting_markdown_keeps_structure_and_shift_pastes_plain() {
    let mut h = harness();
    h.event(Event::Paste("# Shopping\n\n* milk\n* eggs\n".into()));
    h.run();
    let d = &h.state().editor.doc;
    let styles: Vec<_> = d.paragraphs().map(|p| p.style.clone()).collect();
    assert_eq!(texts(&h)[..3], ["Shopping", "milk", "eggs"]);
    assert_eq!(styles[0], xenify_wp_doc::StyleSheet::heading(1));
    assert_eq!(labels(&h)[1..3], ["•", "•"]);
    // With Shift held (Ctrl+Shift+V): the text as it is.
    press(&mut h, Modifiers::COMMAND, Key::A);
    h.input_mut().modifiers = Modifiers::SHIFT;
    h.event(Event::Paste("# Not a heading\n* not a list".into()));
    h.run();
    h.input_mut().modifiers = Modifiers::NONE;
    assert_eq!(texts(&h), ["# Not a heading", "* not a list"]);
}

#[test]
fn footnotes_and_citations_from_the_references_menu() {
    use xenify_wp_doc::cite::{Name, Source};
    let mut h = harness();
    type_text(&mut h, "Commas matter");
    // Ctrl+Alt+F: a footnote; typing goes into it.
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::F);
    type_text(&mut h, "Mostly.");
    let d = &h.state().editor.doc;
    let note = &d.sections[0].notes[0];
    assert_eq!(note.kind, xenify_wp_doc::NoteKind::Foot);
    assert!(matches!(&note.blocks[0], xenify_wp_doc::Block::Paragraph(p) if p.text == "Mostly."));
    assert_eq!(d.paragraphs().next().unwrap().text, format!("Commas matter{}", xenify_wp_doc::Field::STR));
    // Back in the text: References › Insert citation… with a source ticked.
    let first = h.state().editor.doc.paragraphs().next().unwrap().id;
    h.state_mut().editor.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(first, "Commas matter".len()));
    h.state_mut().editor.doc.meta.sources = vec![Source {
        key: "smith2020".into(),
        authors: vec![Name { family: "Smith".into(), given: "Jane".into() }],
        title: "Writing well".into(),
        year: "2020".into(),
        ..Default::default()
    }];
    h.get_by_label("References").click();
    h.run();
    h.get_by_label("Insert citation…").click();
    h.run();
    h.get_by_label("Smith (2020) Writing well").click();
    h.run();
    h.get_by_label("(Smith, 2020)");
    h.get_by_label("Cite").click();
    h.run();
    let p = h.state().editor.doc.paragraphs().next().unwrap().clone();
    assert!(
        p.runs.iter().any(
            |r| matches!(&r.fmt.field, Some(xenify_wp_doc::Field::Citation { keys, .. }) if keys == &["smith2020"])
        )
    );
}

#[test]
fn comments_and_tracked_changes() {
    let mut h = harness();
    h.state_mut().user_name = "Sam".into();
    type_text(&mut h, "The cat sat.");
    // Ctrl+Alt+M on the word at the caret opens a new comment in the Review pane.
    let id = h.state().editor.doc.paragraphs().next().unwrap().id;
    h.state_mut().editor.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(id, 5));
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::M);
    h.run();
    h.event(Event::Text("Which cat?".into()));
    h.run();
    h.get_by_label("Comment").click();
    h.run();
    let d = &h.state().editor.doc;
    assert_eq!(d.meta.comments.len(), 1);
    assert_eq!((d.meta.comments[0].author.as_str(), d.meta.comments[0].text.as_str()), ("Sam", "Which cat?"));
    assert_eq!(xenify_wp_doc::review::anchors(d)[0].1.offset, 4);
    // Track changes (Ctrl+Shift+E), then delete a word: it stays, marked deleted.
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::E);
    assert!(h.state().editor.doc.meta.track_changes);
    h.get_by_label("Tracking changes");
    h.state_mut().editor.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(id, 11));
    press(&mut h, Modifiers::COMMAND, Key::Backspace);
    assert_eq!(texts(&h), ["The cat sat."]);
    assert_eq!(xenify_wp_doc::review::changes(&h.state().editor.doc).len(), 1);
    // Review › Show › No markup: a read-only view; typing changes nothing.
    h.state_mut().review.markup = xenify_wp_doc::review::Markup::Final;
    h.run();
    h.get_by_label("Showing no markup (read only)");
    type_text(&mut h, "x");
    assert_eq!(texts(&h), ["The cat sat."]);
    h.get_by_label("Showing no markup (read only)").click();
    h.run();
    // Review › Accept all changes.
    h.get_by_label("Review").click();
    h.run();
    h.get_by_label("Accept all changes").click();
    h.run();
    assert_eq!(texts(&h), ["The cat ."]);
}

/// Clicks at document point (x, y) `n` times in quick succession.
fn click_doc(h: &mut Harness, x: f64, y: f64, n: usize) {
    let (o, s) = (h.state().view.doc_origin, h.state().view.scale());
    let at = egui::pos2(o.x + x as f32 * s, o.y + y as f32 * s);
    for _ in 0..n {
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
}

#[test]
fn double_click_the_margin_to_edit_the_header() {
    let mut h = harness();
    type_text(&mut h, "Body text.");
    h.state_mut().editor.doc.sections[0].page.header =
        xenify_wp_doc::HfLine { left: "Acme".into(), ..Default::default() };
    h.run();
    let area = h.state().view.layout.hf_areas.iter().find(|(p, f, _)| *p == 0 && !*f).unwrap().2;
    click_doc(&mut h, area.center().x, area.center().y, 2);
    assert_eq!(h.state().view.hf_edit, Some(0));
    let d = &h.state().editor.doc;
    assert_eq!(d.sections[0].headers.len(), 1);
    assert!(xenify_wp_doc::hf::in_header(d, h.state().editor.sel.focus.para));
    type_text(&mut h, " Ltd");
    let header_text = |h: &Harness| match &h.state().editor.doc.sections[0].headers[0].blocks[0] {
        xenify_wp_doc::Block::Paragraph(p) => p.text.clone(),
        _ => String::new(),
    };
    assert_eq!(header_text(&h), "Acme Ltd");
    assert_eq!(texts(&h)[0], "Body text.");
    // Esc goes back to the text.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(h.state().view.hf_edit, None);
    assert!(!xenify_wp_doc::hf::in_header(&h.state().editor.doc, h.state().editor.sel.focus.para));
    // Insert › Footer edits the footer the same way.
    h.state_mut().edit_header(true);
    h.run();
    type_text(&mut h, "Confidential");
    let f = h.state().editor.doc.sections[0].headers.iter().find(|p| p.footer).unwrap();
    assert!(matches!(&f.blocks[0], xenify_wp_doc::Block::Paragraph(p) if p.text == "Confidential"));
}

#[test]
fn unsaved_work_left_by_a_crash_is_offered_back() {
    let dir = std::env::temp_dir().join(format!("xenify-gui-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.md");
    std::fs::write(&file, "The cat sat.\n").unwrap();
    // A run that crashed left unsaved work (its autosave says another process).
    let mut crashed = xenify_wp_ui::recovery::Autosave::in_dir(dir.join("rec"));
    crashed.bind(Some(&file));
    std::thread::sleep(std::time::Duration::from_millis(20));
    crashed.write(&xenify_wp_doc::Document::from_plain_text("The dog sat.")).unwrap();
    let info = std::fs::read_dir(dir.join("rec"))
        .unwrap()
        .flatten()
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .unwrap()
        .path();
    let mut meta: xenify_wp_ui::recovery::Info = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
    meta.pid = u32::MAX;
    std::fs::write(&info, serde_json::to_vec(&meta).unwrap()).unwrap();

    let mut h = harness();
    h.state_mut().recovery.off = false;
    h.state_mut().recovery.dir = Some(dir.join("rec"));
    h.state_mut().open_path(&file);
    h.run();
    h.get_by_label("Show as changes").click();
    h.run();
    let d = &h.state().editor.doc;
    let changes = xenify_wp_doc::review::changes(d);
    let words: Vec<&str> = changes.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(words, ["cat", "dog"]);
    assert!(h.state().editor.is_dirty());
    // The file itself is as it was saved.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "The cat sat.\n");
    // Saving clears the autosave: nothing is offered next time.
    let saved = h.state_mut().save_to(&file);
    assert!(saved);
    assert!(h.state_mut().recovery.left_for(&file).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn record_a_change_with_a_suggested_summary() {
    let mut h = harness();
    h.state_mut().user_name = "Sam".into();
    type_text(&mut h, "A new opening paragraph.");
    h.state_mut().open_record_change(false);
    h.run();
    // Not saved yet: everything is new.
    h.get_by_label(&format!("Sam changed this on {}:", xenify_wp_doc::changelog::day(&xenify_wp_doc::review::now())));
    h.get_by_label("Record").click();
    h.run();
    let log = &h.state().editor.doc.meta.changelog;
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].author.as_str(), log[0].summary.as_str()), ("Sam", "Edited “the opening” (4 words added)"));
}

#[test]
fn insert_a_table_from_the_menu() {
    let mut h = harness();
    type_text(&mut h, "Before");
    h.get_by_label("Insert table").click();
    h.run();
    let grid = h.get_by_label("Table size").rect();
    let at = grid.min + egui::vec2(18.0 * 2.5, 18.0 * 1.5);
    h.event(Event::PointerMoved(at));
    h.run();
    h.event(Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run();
    h.event(Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    let tables =
        h.state().editor.doc.sections[0].blocks.iter().filter(|b| matches!(b, xenify_wp_doc::Block::Table(_))).count();
    assert_eq!(tables, 1);
}

#[test]
fn markdown_shortcuts_line_after_line() {
    let mut h = harness();
    type_text(&mut h, "* dfdf\n1. dfds\n# dsfsd");
    let d = &h.state().editor.doc;
    let shape: Vec<(String, String, Option<String>)> =
        d.paragraphs().map(|p| (p.text.clone(), p.style.clone(), p.list.as_ref().map(|l| l.list.clone()))).collect();
    assert_eq!(shape[0], ("dfdf".into(), "normal".into(), Some(xenify_wp_doc::ListDef::BULLETS.into())));
    assert_eq!(shape[1], ("dfds".into(), "normal".into(), Some(xenify_wp_doc::ListDef::NUMBERS.into())));
    assert_eq!(shape[2], ("dsfsd".into(), xenify_wp_doc::StyleSheet::heading(1), None));
}

#[test]
fn right_click_a_word_for_synonyms_and_the_dictionary() {
    let mut h = harness();
    xenify_wp_check::lexicon::lexicon();
    type_text(&mut h, "glad");
    let at = h.state().view.doc_origin + egui::vec2(8.0, 6.0);
    h.event(Event::PointerMoved(at));
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
    h.run_steps(2);
    h.get_by_label("Look up “glad”");
    h.get_by_label("Synonyms").hover();
    h.run_steps(3);
    h.get_by_label("happy").click();
    h.run_steps(2);
    assert_eq!(texts(&h), ["happy"]);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(texts(&h), ["glad"]);

    // Shift+F7: the Dictionary tab on the word at the caret.
    press(&mut h, Modifiers::SHIFT, Key::F7);
    h.run_steps(2);
    assert!(h.state().dictionary.show);
    assert!(h.query_all_by_label("1.").count() > 1);
    h.get_by_label("Open English WordNet (CC BY 4.0), based on Princeton WordNet");
}

#[test]
fn find_and_replace_from_the_bar() {
    let mut h = harness();
    type_text(&mut h, "The cat sat on the cat mat.");
    press(&mut h, Modifiers::COMMAND, Key::H);
    h.get_by_label("Find").click();
    h.run();
    h.event(Event::Text("cat".into()));
    h.run();
    h.get_by_label("2 found");
    press(&mut h, Modifiers::NONE, Key::Enter);
    h.get_by_label("1 of 2");
    h.get_by_label("Replace with").click();
    h.run();
    h.event(Event::Text("dog".into()));
    h.run();
    h.get_by_label("Replace all").click();
    h.run();
    assert_eq!(texts(&h), ["The dog sat on the dog mat."]);
    // Ctrl+Alt+F is still a footnote, not Find.
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::F);
    assert_eq!(h.state().editor.doc.sections[0].notes.len(), 1);
}

#[test]
fn table_properties_change_the_table_as_you_go() {
    let mut h = harness();
    {
        let app = h.state_mut();
        xenify_wp_doc::tables::insert_table(&mut app.editor, 3, 3).unwrap();
        app.open_table_props();
    }
    h.run();
    let table = |h: &Harness| {
        h.state().editor.doc.sections[0]
            .blocks
            .iter()
            .find_map(|b| if let xenify_wp_doc::Block::Table(t) = b { Some(t.clone()) } else { None })
            .unwrap()
    };
    // Changes show in the document straight away (so saving keeps them)…
    h.get_by_label("Bold header style").click();
    h.run();
    assert!(table(&h).header_fill.is_some());
    h.get_by_label("Banded rows").click();
    h.run();
    assert!(table(&h).band.is_some());
    // …as one undo step; Cancel puts the table back.
    h.get_by_label("Cancel").click();
    h.run();
    assert!(table(&h).header_fill.is_none() && table(&h).band.is_none());
    assert!(h.query_by_label("Choose cells").is_none() && h.query_by_label("Banded rows").is_none());
    // Choose cells in the picture: drag from the top-left to the middle cell.
    h.state_mut().open_table_props();
    h.run();
    h.get_by_label("Cells").click();
    h.run();
    let pic = h.get_by_label("Choose cells").rect();
    let at = |fx: f32, fy: f32| egui::pos2(pic.left() + pic.width() * fx, pic.top() + pic.height() * fy);
    h.event(Event::PointerMoved(at(0.1, 0.1)));
    h.event(Event::PointerButton { pos: at(0.1, 0.1), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    for f in [0.2, 0.35, 0.5] {
        h.event(Event::PointerMoved(at(f, f)));
        h.run();
    }
    h.event(Event::PointerButton { pos: at(0.5, 0.5), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    h.get_by_label("Middle").click();
    h.run();
    let t = table(&h);
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            let want = if r < 2 && c < 2 { xenify_wp_doc::VAlign::Middle } else { xenify_wp_doc::VAlign::Top };
            assert_eq!(cell.v_align, want, "cell {r},{c}");
        }
    }
    h.get_by_label("OK").click();
    h.run();
    assert!(h.query_by_label("Choose cells").is_none() && h.query_by_label("Banded rows").is_none());
    assert_eq!(table(&h).rows[0].cells[0].v_align, xenify_wp_doc::VAlign::Middle);
}

#[test]
fn caption_a_picture_from_its_panel() {
    let mut h = harness();
    type_text(&mut h, "Intro");
    let img = {
        let app = h.state_mut();
        xenify_wp_doc::tables::insert_image(&mut app.editor, "media/a.png", 120.0, 90.0).unwrap();
        let id = app.editor.doc.sections[0]
            .blocks
            .iter()
            .find(|b| matches!(b, xenify_wp_doc::Block::Image(_)))
            .unwrap()
            .id();
        app.view.selected_image = Some(id);
        id
    };
    h.run();
    h.get_by_label("Add a caption").click();
    h.run();
    h.run();
    assert!(h.state().view.selected_image.is_none(), "the panel closes");
    type_text(&mut h, "A hill");
    let d = &h.state().editor.doc;
    let body = &d.sections[0].blocks;
    let i = body.iter().position(|b| b.id() == img).unwrap();
    assert!(
        matches!(&body[i + 1], xenify_wp_doc::Block::Paragraph(p) if p.text == "Figure 1: A hill"),
        "typing goes into the caption"
    );
}

#[test]
fn dialogs_close_with_their_buttons_and_esc() {
    let mut h = harness();
    type_text(&mut h, "Some text");
    h.get_by_label("Format").click();
    h.run();
    h.get_by_label("Borders and shading…").click();
    h.run();
    h.run();
    assert!(h.query_by_label("Done").is_some(), "the dialog is open");
    press(&mut h, Modifiers::NONE, Key::Escape);
    h.run();
    assert!(h.query_by_label("Done").is_none(), "Esc closes it");
    h.get_by_label("Format").click();
    h.run();
    h.get_by_label("Borders and shading…").click();
    h.run();
    h.run();
    h.get_by_label("Done").click();
    h.run();
    h.run();
    assert!(h.query_by_label("Done").is_none(), "Done closes it");
}

#[test]
fn alt_on_its_own_gives_the_menu_bar_the_keyboard() {
    let mut h = harness();
    type_text(&mut h, "Hi");
    h.input_mut().modifiers = Modifiers::ALT;
    h.run();
    h.input_mut().modifiers = Modifiers::NONE;
    h.run();
    assert_eq!(h.state().menu_armed(), Some(0), "File is highlighted");
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    assert_eq!(h.state().menu_armed(), Some(1));
    // A letter opens its menu (and isn't typed into the page).
    h.event(Event::Key { key: Key::T, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.event(Event::Text("t".into()));
    h.run();
    h.run();
    assert!(h.query_by_label("Tools").is_some());
    assert!(h.query_by_label("Word count…").is_some(), "Tools is open");
    assert_eq!(h.state().editor.doc.paragraphs().next().unwrap().text, "Hi");
    assert_eq!(h.state().menu_armed(), None);
}

#[test]
fn several_carets_type_together() {
    let mut h = harness();
    type_text(&mut h, "cat one\ncat two\ndog");
    // Ctrl+D on "cat": the word, then the next "cat" too.
    let first = h.state().editor.doc.paragraphs().next().unwrap().id;
    h.state_mut().editor.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(first, 1));
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::D);
    press(&mut h, Modifiers::COMMAND, Key::D);
    assert_eq!(h.state().view.extra.len(), 1);
    type_text(&mut h, "bird");
    let texts: Vec<String> = h.state().editor.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(texts, ["bird one", "bird two", "dog"]);
    // Home at both, then one undo takes the typing back at both.
    press(&mut h, Modifiers::NONE, Key::Home);
    type_text(&mut h, "> ");
    let texts: Vec<String> = h.state().editor.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(texts, ["> bird one", "> bird two", "dog"]);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    let texts: Vec<String> = h.state().editor.doc.paragraphs().map(|p| p.text.clone()).collect();
    assert_eq!(texts, ["bird one", "bird two", "dog"]);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().view.extra.is_empty());
}

#[test]
fn symbols_and_emoji_picker_inserts_at_the_caret() {
    let mut h = harness();
    type_text(&mut h, "Price ");
    press(&mut h, Modifiers::COMMAND, Key::Period);
    assert!(h.state().symbols.open);
    // A new window takes a frame to size itself before it takes clicks.
    h.run_steps(2);
    h.get_by_label("©").click();
    h.run();
    assert_eq!(texts(&h), ["Price ©"]);
    // Emoji: search, then pick; it goes to the recent row too.
    h.state_mut().symbols.tab = xenify_wp_ui::symbols::Tab::Emoji;
    h.state_mut().symbols.query = "hot beverage".into();
    h.run_steps(2);
    h.get_by_label("hot beverage").click();
    h.run();
    assert_eq!(texts(&h), ["Price ©☕"]);
    assert_eq!(h.state().recent_symbols()[0], "☕");
}

#[test]
fn navigation_pane_lists_headings_and_goes_to_them() {
    let mut h = harness();
    type_text(&mut h, "# Introduction\nSome text.\n## Background\nMore text.\n# Results\nThe end.");
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    assert!(h.state().nav.show);
    h.run_steps(2);
    // The caret is under "Results"; clicking "Background" goes there.
    h.get_by_label("Background").click();
    h.run();
    let ed = &h.state().editor;
    assert_eq!(ed.doc.para(ed.sel.focus.para).unwrap().text, "Background");
    assert_eq!(ed.sel.focus.offset, 0);
    // The same keys close it again.
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    assert!(!h.state().nav.show);
}

/// Screenshots of the new panels for a look (needs a GPU; run with --ignored).
#[test]
#[ignore]
fn screenshot_navigation_and_symbols() {
    let mut h = harness();
    type_text(&mut h, "# Introduction\nSome text.\n## Background\nMore text.\n### Detail\n# Results\nThe end.");
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    press(&mut h, Modifiers::COMMAND, Key::Period);
    h.state_mut().symbols.tab = xenify_wp_ui::symbols::Tab::Emoji;
    h.run_steps(4);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-nav-symbols.png")).unwrap();
    h.state_mut().symbols.tab = xenify_wp_ui::symbols::Tab::Symbols;
    h.run_steps(3);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-symbols.png")).unwrap();
    h.state_mut().symbols.open = false;
    press(&mut h, Modifiers::COMMAND, Key::T);
    type_text(&mut h, "Second document");
    h.run_steps(2);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-tabs.png")).unwrap();
    press(&mut h, Modifiers::COMMAND, Key::P);
    h.run_steps(3);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-print.png")).unwrap();
}

#[test]
fn print_screen_opens_and_closes() {
    let mut h = harness();
    type_text(&mut h, "Hello\n");
    press(&mut h, Modifiers::COMMAND, Key::P);
    assert!(h.state().print_preview.is_some());
    h.run_steps(2);
    // In a test there's no printer: it says so and offers a PDF instead.
    h.get_by_label("Export as PDF…");
    h.get_by_label("Page 1 of 1");
    // Typing a range picks "Pages:".
    h.state_mut().print_preview.as_mut().unwrap().range = "1".into();
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().print_preview.is_none());
    // The document wasn't touched.
    assert_eq!(texts(&h), ["Hello", ""]);
}

/// A one-line form: "Colour: " then a drop-down showing its prompt.
fn form_doc() -> (xenify_wp_doc::Document, xenify_wp_doc::NodeId) {
    use xenify_wp_doc::controls::{Control, ControlKind};
    use xenify_wp_doc::{CharFmt, Document, NodeId, Run};
    let mut doc = Document::new();
    let para = doc.paragraphs().next().unwrap().id;
    doc.meta.controls.push(Control {
        id: NodeId(900),
        kind: ControlKind::Dropdown { items: vec!["Red".into(), "Blue".into()] },
        title: "Colour".into(),
        tag: String::new(),
        placeholder: true,
    });
    let p = doc.para_mut(para).unwrap();
    p.text = "Colour: Choose an item.".into();
    p.runs = vec![
        Run { len: 8, fmt: CharFmt::default() },
        Run { len: 15, fmt: CharFmt { control: Some(NodeId(900)), ..Default::default() } },
    ];
    (doc, para)
}

#[test]
fn form_lists_and_prompts_fill_in() {
    use xenify_wp_doc::{Editor, Pos, Selection};
    let mut h = harness();
    let (doc, para) = form_doc();
    h.state_mut().editor = Editor::new(doc);
    h.run();
    // A click in the list opens its choices; picking one fills it in.
    h.state_mut().view.control_click = Some((Pos::new(para, 10), egui::pos2(300.0, 200.0)));
    h.run_steps(2);
    h.get_by_label("Blue").click();
    h.run();
    assert_eq!(texts(&h), ["Colour: Blue"]);
    assert!(h.state().control_menu.is_none());
    // Typing over a prompt replaces the whole prompt.
    let (doc, para) = form_doc();
    h.state_mut().editor = Editor::new(doc);
    h.state_mut().editor.sel = Selection::caret(Pos::new(para, 12));
    let id = h.state().view.id;
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run();
    type_text(&mut h, "Green");
    assert_eq!(texts(&h), ["Colour: Green"]);
}

#[test]
fn spelling_results_stay_with_their_tab() {
    let mut h = harness();
    type_text(&mut h, "A speling mistake.");
    wait_for_proofing(&mut h, |a| a.proofing.issues.iter().any(|i| i.rule == "SpellCheck"));
    press(&mut h, Modifiers::COMMAND, Key::T);
    type_text(&mut h, "All correct here.");
    wait_for_proofing(&mut h, |a| {
        a.proofing.issues.is_empty() && !a.editor.doc.paragraphs().all(|p| p.text.is_empty())
    });
    assert!(h.state().proofing.issues.is_empty(), "{:?}", h.state().proofing.issues);
    // Back to the first: its mistake is still marked.
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
    assert_eq!(texts(&h), ["A speling mistake."]);
    assert!(h.state().proofing.issues.iter().any(|i| i.rule == "SpellCheck"));
}

#[test]
fn split_view_shows_two_documents() {
    let mut h = harness();
    type_text(&mut h, "First document");
    press(&mut h, Modifiers::COMMAND, Key::T);
    type_text(&mut h, "Second document");
    let first = h.state().tabs.order[0];
    press(&mut h, Modifiers::COMMAND, Key::Backslash);
    assert!(h.state().split.is_some());
    // The other pane starts on this document; pick the first one for it.
    h.state_mut().split.as_mut().unwrap().other = first;
    h.run_steps(3);
    assert_eq!(texts(&h), ["Second document"], "still working in the second");
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-split.png")).unwrap();
    press(&mut h, Modifiers::COMMAND, Key::Backslash);
    assert!(h.state().split.is_none());
}

#[test]
fn a_tab_moves_to_a_window_of_its_own() {
    let mut h = harness();
    type_text(&mut h, "Main window text");
    press(&mut h, Modifiers::COMMAND, Key::T);
    type_text(&mut h, "Moving out");
    let moving = h.state().tabs.active;
    h.state_mut().move_tab_to_new_window(moving);
    h.run_steps(3);
    // The main window has one tab again, its own document in front.
    assert_eq!(h.state().windows.len(), 1);
    assert_eq!(h.state().tabs.order.len(), 1);
    assert_eq!(texts(&h), ["Main window text"]);
    // Drawing the other window each frame doesn't disturb this one. (The
    // test draws both windows in one, sharing the keyboard: give it back.)
    let id = h.state().view.id;
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run();
    type_text(&mut h, " and more");
    assert_eq!(texts(&h), ["Main window text and more"]);
    // Quitting gathers the windows' documents to ask about them here.
    h.state_mut().windows.clear();
}

fn equations(h: &Harness) -> Vec<(String, bool)> {
    h.state()
        .editor
        .doc
        .paragraphs()
        .flat_map(|p| p.runs.iter().filter_map(|r| r.fmt.field.clone()))
        .filter_map(|f| match f {
            xenify_wp_doc::Field::Equation { latex, display, .. } => Some((latex, display)),
            _ => None,
        })
        .collect()
}

#[test]
fn equations_are_typed_visually_and_in_latex() {
    let mut h = harness();
    type_text(&mut h, "Area: ");
    press(&mut h, Modifiers::ALT, Key::Equals);
    assert!(h.state().equation.is_some());
    h.run_steps(2);
    let latex = |h: &Harness| h.state().equation.as_ref().unwrap().latex.clone();
    // Typed into the equation itself: / makes a fraction, Tab goes to the next box.
    type_text(&mut h, "1/2");
    assert_eq!(latex(&h), "\\frac{1}{2}");
    press(&mut h, Modifiers::NONE, Key::Tab);
    type_text(&mut h, "+x^2");
    assert_eq!(latex(&h), "\\frac{1}{2}+x^{2}");
    // A command: \ then its name and a space.
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    type_text(&mut h, "\\sqrt y");
    assert_eq!(latex(&h), "\\frac{1}{2}+x^{2}\\sqrt{y}");
    // The palette puts a structure at the caret, with the caret in its first box.
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    h.get_by_label("a/b").click();
    h.run_steps(2);
    type_text(&mut h, "a");
    assert!(latex(&h).ends_with("\\frac{a}{}"), "{}", latex(&h));
    // Clicking in the equation moves the caret there (the start: before the fraction).
    let eq = h.get_by_label("Equation editor").rect();
    h.event(Event::PointerMoved(eq.left_center() + egui::vec2(4.0, 0.0)));
    h.event(Event::PointerButton { pos: eq.left_center() + egui::vec2(4.0, 0.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: eq.left_center() + egui::vec2(4.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    type_text(&mut h, "y=");
    assert!(latex(&h).starts_with("y="), "{}", latex(&h));
    // LaTeX can still be written directly (under Advanced).
    h.state_mut().equation.as_mut().unwrap().latex = "\\pi r^2".into();
    h.run_steps(2);
    h.get_by_label("Done").click();
    h.run();
    assert!(h.state().equation.is_none());
    assert_eq!(equations(&h), [("\\pi r^2".to_string(), false)]);
    // Cancelling a new one takes it out again.
    press(&mut h, Modifiers::ALT, Key::Equals);
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(equations(&h).len(), 1);
}

/// Screenshots of each side panel tab, and all open at once (run with --ignored).
#[test]
#[ignore]
fn screenshot_panels() {
    let mut h = harness();
    type_text(&mut h, "# Heading\nSome text with a speling mistake.\n");
    use xenify_wp_ui::dock::Tab;
    for t in Tab::ALL {
        for other in Tab::ALL {
            xenify_wp_ui::dock::set_open(h.state_mut(), other, false);
        }
        xenify_wp_ui::dock::open(h.state_mut(), t);
        h.run_steps(4);
        let img = h.render().expect("render");
        img.save(format!("{}/../../target/panel-{}.png", env!("CARGO_MANIFEST_DIR"), t.name().replace(' ', "-")))
            .unwrap();
    }
    // The style dialog.
    let normal = h.state().editor.doc.styles.get("normal").cloned().unwrap();
    h.state_mut().styles_panel.dialog = Some(xenify_wp_ui::styles_panel::StyleDialog::Edit(Box::new(normal)));
    h.run_steps(4);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/panel-style-dialog.png")).unwrap();
    h.state_mut().styles_panel.dialog = None;
    for t in Tab::ALL {
        xenify_wp_ui::dock::open(h.state_mut(), t);
    }
    h.state_mut().show_merge = true;
    h.run_steps(4);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/panel-all.png")).unwrap();
}

#[test]
fn writing_tab_checks_one_thing_at_a_time() {
    let mut h = harness();
    type_text(&mut h, "A speling mistake here.");
    h.state_mut().show_proofing = true;
    wait_for_proofing(&mut h, |a| a.proofing.issues.iter().any(|i| i.rule == "SpellCheck"));
    h.run_steps(2);
    // Check goes to the first thing found and shows what to do about it.
    h.get_by_label("Check").click();
    h.run_steps(2);
    let ed = &h.state().editor;
    assert_eq!(ed.doc.text_in(ed.sel), "speling");
    h.get_by_label("spelling").click();
    h.run_steps(2);
    assert_eq!(texts(&h), ["A spelling mistake here."]);
}

/// Drags with the primary button from `a` to `b` in a few steps.
fn drag(h: &mut Harness, a: egui::Pos2, b: egui::Pos2) {
    h.event(Event::PointerMoved(a));
    h.run_steps(2);
    h.event(Event::PointerButton { pos: a, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for k in 1..=6 {
        h.event(Event::PointerMoved(a + (b - a) * (k as f32 / 6.0)));
        h.run_steps(2);
    }
    h.event(Event::PointerButton { pos: b, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn rulers_drag_margins_and_indents() {
    let mut h = harness();
    type_text(&mut h, "Some text");
    let margins = |h: &Harness| h.state().editor.doc.sections[0].page.margins.clone();
    let indent = |h: &Harness| h.state().editor.doc.paragraphs().next().unwrap().props.indent_left;
    let before = margins(&h);
    let (ox, oy, s) = {
        let v = &h.state().view;
        (v.doc_origin.x, v.doc_origin.y, v.scale())
    };
    let page = h.state().view.layout.pages[0].clone();
    let ruler = h.get_by_label("Ruler").rect();
    let left_px = ox + page.body.x0 as f32 * s;
    // The middle of the ruler at the margin's edge moves the margin (the
    // left-indent arrow sits on the same spot)…
    drag(&mut h, egui::pos2(left_px, ruler.center().y), egui::pos2(left_px + 36.0 * s, ruler.center().y));
    assert!((margins(&h).left - (before.left + 36.0)).abs() < 5.0, "{:?} -> {:?}", before, margins(&h));
    assert_eq!(indent(&h), None, "the indent stays");
    // …the bottom arrow moves the left indent.
    let page = h.state().view.layout.pages[0].clone();
    let left_px = ox + page.body.x0 as f32 * s;
    drag(&mut h, egui::pos2(left_px, ruler.bottom() - 5.0), egui::pos2(left_px + 36.0 * s, ruler.bottom() - 5.0));
    assert!(indent(&h).is_some_and(|i| (i - 36.0).abs() < 5.0), "{:?}", indent(&h));
    // The vertical ruler moves the top margin.
    let vr = h.get_by_label("Vertical ruler").rect();
    let top_px = oy + page.body.y0 as f32 * s;
    drag(&mut h, egui::pos2(vr.center().x, top_px), egui::pos2(vr.center().x, top_px + 36.0 * s));
    assert!((margins(&h).top - (before.top + 36.0)).abs() < 5.0, "{:?}", margins(&h));
}

#[test]
fn go_to_page_with_ctrl_g() {
    let mut h = harness();
    type_text(&mut h, "First page");
    press(&mut h, Modifiers::COMMAND, Key::Enter);
    type_text(&mut h, "Second page");
    press(&mut h, Modifiers::COMMAND, Key::Home);
    press(&mut h, Modifiers::COMMAND, Key::G);
    h.get_by_label("Page number").click();
    h.run();
    type_text(&mut h, "2");
    press(&mut h, Modifiers::NONE, Key::Enter);
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.view.caret_page(&app.editor), Some(1), "on the second page");
    assert!(h.query_by_label("Page number").is_none(), "the dialog closed");
    // The page count in the status bar opens it too.
    h.get_by_label("Page 2 of 2").click();
    h.run();
    h.get_by_label("Page number");
}

#[test]
fn regex_replace_and_line_filters_in_the_find_bar() {
    let mut h = harness();
    type_text(&mut h, "Smith, John\nDoe, Jane\nnobody here");
    press(&mut h, Modifiers::COMMAND, Key::H);
    type_text(&mut h, r"(\w+), (\w+)");
    h.get_by_label(".*").click();
    h.run();
    h.get_by_label("Replace with").click();
    h.run();
    type_text(&mut h, "$2 $1");
    h.get_by_label("Replace all").click();
    h.run();
    assert_eq!(texts(&h), ["John Smith", "Jane Doe", "nobody here"]);
    // Keep only the paragraphs containing "J".
    h.get_by_label("Find").click();
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::A);
    type_text(&mut h, "^J");
    h.get_by_label("Filter lines").click();
    h.run();
    h.get_by_label("Keep only them").click();
    h.run();
    assert_eq!(texts(&h), ["John Smith", "Jane Doe"]);
    h.get_by_label("Remove them").click();
    h.run();
    assert_eq!(texts(&h), [""]);
}

/// A look at the window: menus, tabs, the find bar along the bottom and
/// empty generated lists (needs a GPU; run with --ignored).
#[test]
#[ignore]
fn screenshot_find_bar_and_generated_lists() {
    let mut h = harness();
    {
        let app = h.state_mut();
        let _ = xenify_wp_doc::tables::insert_toc(&mut app.editor, 3);
        let _ = xenify_wp_doc::tables::insert_generated(&mut app.editor, xenify_wp_doc::TocKind::ChangeLog);
        let _ = xenify_wp_doc::tables::insert_generated(&mut app.editor, xenify_wp_doc::TocKind::Bibliography);
    }
    press(&mut h, Modifiers::COMMAND, Key::T);
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
    press(&mut h, Modifiers::COMMAND, Key::H);
    h.state_mut().find.filter = true;
    type_text(&mut h, "cat");
    h.run_steps(4);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-find-generated.png")).unwrap();
    press(&mut h, Modifiers::COMMAND, Key::G);
    h.run_steps(3);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-goto.png")).unwrap();
}

/// Screenshots of the visual equation editor and the symbols search (run with --ignored).
#[test]
#[ignore]
fn screenshot_equation_editor_and_symbol_search() {
    let mut h = harness();
    type_text(&mut h, "Energy: ");
    press(&mut h, Modifiers::ALT, Key::Equals);
    h.run_steps(2);
    type_text(&mut h, "E=1/2");
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    type_text(&mut h, "mv^2");
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    type_text(&mut h, "+\\sqrt ");
    h.run_steps(3);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-equation-editor.png")).unwrap();
    h.get_by_label("Done").click();
    h.run_steps(2);
    press(&mut h, Modifiers::COMMAND, Key::Period);
    h.state_mut().symbols.query = ";)".into();
    h.run_steps(3);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-symbol-search.png")).unwrap();
    h.state_mut().symbols.query = "arrow".into();
    h.run_steps(3);
    let img = h.render().expect("render");
    img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/write-symbol-search-arrow.png")).unwrap();
}

#[test]
fn profile_editor_has_the_new_checks() {
    let mut h = harness();
    {
        let app = h.state_mut();
        app.proofing.editor.open = true;
        app.proofing.editor.selected = "standard".into();
    }
    h.run_steps(4);
    for label in ["Consistency", "Word choice", "Sentences that may not make sense"] {
        assert!(h.query_by_label(label).is_some(), "{label} toggle missing");
    }
    // Off by default: no suggested fix to offer.
    assert!(!h.state().proofing.settings.profile.sentence_sense);
}

/// Typing mistakes into the app gets them flagged (rules and, when a model
/// is installed, the spotter). Run by hand: `cargo test -p xenify-wp-ui
/// --test gui flags_typed_mistakes -- --ignored --nocapture`.
#[test]
#[ignore = "needs the installed model; run by hand"]
fn flags_typed_mistakes() {
    let mut h = harness();
    {
        let app = h.state_mut();
        app.proofing.spotter.enabled = true;
        app.proofing.spotter.threshold = 0.97;
        app.proofing.recheck();
    }
    type_text(&mut h, "She go to school every day. I seen him at the shop. We discussed about the problem. There going to the park later. I am agree with you.");
    // The checker runs on its own thread: give it time.
    for _ in 0..600 {
        h.run_steps(5);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let found = h.state().proofing.issues.len();
        if found >= 5 {
            break;
        }
    }
    let issues: Vec<String> = h.state().proofing.issues.iter().map(|i| format!("{} — {}", i.rule, i.message)).collect();
    println!("{} issues:\n{}", issues.len(), issues.join("\n"));
    println!("spotter: {}", h.state().proofing.spotter_status);
    assert!(issues.len() >= 5, "{issues:?}");
}
