//! Renders a sample document on the GPU (skipped without one). The PNG is
//! written to target/xenify-write-sample.png for a look.

use xenify_wp_doc::{Align, CharFmt, Document, Editor, ListDef, ParaProps, Pos, Rgba, Selection, StyleSheet, ops};
use xenify_wp_layout::offscreen::Offscreen;
use xenify_wp_layout::{Engine, Fonts, paint};

#[test]
fn renders_styled_text() {
    let mut gpu = match Offscreen::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: {e:?}");
            return;
        }
    };
    let mut ed = Editor::new(Document::from_plain_text(
        "Xenify Write\nA word processor on parley and Vello\nIntroduction\nThis paragraph has bold, italic, underlined, struck and highlighted words, and it is long enough to wrap onto a second line in the column.\nA quotation, set in the Quote style, indented from both sides.\nFirst bullet\nSecond bullet with a nested list\nNested one\nNested two\nStep one\nStep two\nCentred text with a link",
    ));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    for (i, style) in [(0, StyleSheet::TITLE), (1, StyleSheet::SUBTITLE), (2, "heading1"), (4, StyleSheet::QUOTE)] {
        ed.sel = Selection::caret(Pos::new(ids[i], 0));
        ops::set_style(&mut ed, style).unwrap();
    }
    let text = ed.doc.para(ids[3]).unwrap().text.clone();
    let mark = |ed: &mut Editor, word: &str, f: CharFmt| {
        let at = text.find(word).unwrap();
        ed.sel = Selection { anchor: Pos::new(ids[3], at), focus: Pos::new(ids[3], at + word.len()) };
        ops::apply_format(ed, &f, "fmt").unwrap();
    };
    mark(&mut ed, "bold", CharFmt { bold: Some(true), ..Default::default() });
    mark(&mut ed, "italic", CharFmt { italic: Some(true), ..Default::default() });
    mark(&mut ed, "underlined", CharFmt { underline: Some(true), ..Default::default() });
    mark(&mut ed, "struck", CharFmt { strike: Some(true), ..Default::default() });
    mark(&mut ed, "highlighted", CharFmt { highlight: Some(Rgba::rgb(0xff, 0xeb, 0x3b)), ..Default::default() });

    let sel = |ed: &mut Editor, a: usize, b: usize| {
        let len = ed.doc.para(ids[b]).unwrap().text.len();
        ed.sel = Selection { anchor: Pos::new(ids[a], 0), focus: Pos::new(ids[b], len) };
    };
    sel(&mut ed, 5, 8);
    ops::toggle_list(&mut ed, ListDef::BULLETS).unwrap();
    sel(&mut ed, 7, 8);
    ops::change_list_level(&mut ed, 1).unwrap();
    sel(&mut ed, 9, 10);
    ops::toggle_list(&mut ed, ListDef::NUMBERS).unwrap();
    sel(&mut ed, 11, 11);
    ops::apply_para_props(&mut ed, &ParaProps { align: Some(Align::Center), ..Default::default() }, "c").unwrap();
    let t = ed.doc.para(ids[11]).unwrap().text.clone();
    let at = t.find("a link").unwrap();
    ed.sel = Selection { anchor: Pos::new(ids[11], at + 2), focus: Pos::new(ids[11], at + 6) };
    ops::apply_format(&mut ed, &CharFmt { link: Some("https://example.com".into()), ..Default::default() }, "Link")
        .unwrap();

    let mut engine = Engine::new(Fonts::bundled_only());
    let layout = engine.layout_pageless(&ed.doc);
    let scale = 2.0;
    let (w, h) = (((layout.width + 72.0) * scale) as u32, ((layout.height + 72.0) * scale) as u32);
    let mut scene = vello::Scene::new();
    let t = kurbo::Affine::scale(scale as f64) * kurbo::Affine::translate((36.0, 36.0));
    paint::paint(&mut scene, &layout, t, kurbo::Rect::new(-1e9, -1e9, 1e9, 1e9));
    let img = gpu.render(&scene, w, h).unwrap();
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xenify-write-sample.png");
    std::fs::write(&out, img.to_png()).unwrap();
    // Something dark was drawn (text), and the highlight is yellow somewhere.
    let dark = img.pixels.chunks(4).filter(|p| p[0] < 80 && p[1] < 80 && p[2] < 80).count();
    assert!(dark > 1000, "text drawn: {dark}");
    let yellow = img.pixels.chunks(4).filter(|p| p[0] > 240 && p[1] > 220 && p[2] < 90).count();
    assert!(yellow > 200, "highlight drawn: {yellow}");
}

#[test]
fn draws_emoji_as_pictures_in_the_line() {
    let mut gpu = match Offscreen::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: {e:?}");
            return;
        }
    };
    let doc = Document::from_plain_text(
        "Coffee ☕ and cake 🍰 at 3pm, a waving hand 👋 here.\nText symbols stay text: © ™ → ★",
    );
    let mut engine = Engine::new(Fonts::bundled_only());
    let layout = engine.layout_pageless(&doc);
    let emoji: Vec<_> = layout
        .fragments
        .iter()
        .flat_map(|f| f.para.items.iter())
        .filter_map(|i| match i {
            xenify_wp_layout::Item::Emoji { glyph, .. } => Some(glyph.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(emoji, ["☕", "🍰", "👋"]);
    let scale = 2.0;
    let (w, h) = (((layout.width + 72.0) * scale) as u32, ((layout.height + 72.0) * scale) as u32);
    let mut scene = vello::Scene::new();
    let t = kurbo::Affine::scale(scale as f64) * kurbo::Affine::translate((36.0, 36.0));
    paint::paint(&mut scene, &layout, t, kurbo::Rect::new(-1e9, -1e9, 1e9, 1e9));
    let img = gpu.render(&scene, w, h).unwrap();
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xenify-write-emoji.png");
    std::fs::write(&out, img.to_png()).unwrap();
    // Colour where the emoji are (text is black on white).
    let colourful = img.pixels.chunks(4).filter(|p| p[0].abs_diff(p[1]) > 60 || p[1].abs_diff(p[2]) > 60).count();
    assert!(colourful > 300, "emoji drawn in colour: {colourful}");
}

#[test]
fn typesets_equations() {
    use xenify_wp_doc::{CharFmt, Field, Paragraph, Run};
    let mut gpu = match Offscreen::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: {e:?}");
            return;
        }
    };
    let eqs = [
        ("Inline: ", "\\frac{a}{b} + x^2 - \\sqrt{y_1^3}", false),
        ("", "x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}", true),
        ("", "\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}", true),
        ("", "\\int_0^\\infty e^{-x^2} \\, dx = \\frac{\\sqrt{\\pi}}{2}", true),
        ("", "\\left( \\begin{matrix} 1 & 0 \\\\ 0 & 1 \\end{matrix} \\right) \\hat{x} \\vec{v} \\overline{AB}", true),
        (
            "Greek and functions: ",
            "\\alpha + \\beta \\le \\sin \\theta, \\quad \\lim_{n \\to \\infty} \\frac{1}{n} = 0",
            false,
        ),
        ("", "f(x) = \\begin{cases} x & x \\ge 0 \\\\ -x & x < 0 \\end{cases}", true),
        ("Unknown: ", "\\nosuch{x}", false),
    ];
    let mut doc = Document::from_plain_text(&vec!["x"; eqs.len()].join("\n"));
    let ids: Vec<_> = doc.paragraphs().map(|p| p.id).collect();
    for (id, (lead, latex, display)) in ids.iter().zip(eqs) {
        let p: &mut Paragraph = doc.para_mut(*id).unwrap();
        p.text = format!("{lead}{}", Field::STR);
        let field = CharFmt {
            field: Some(Field::Equation { latex: latex.into(), display, omml: String::new() }),
            ..Default::default()
        };
        p.runs = if lead.is_empty() {
            vec![Run { len: 3, fmt: field }]
        } else {
            vec![Run { len: lead.len(), fmt: CharFmt::default() }, Run { len: 3, fmt: field }]
        };
    }
    let mut engine = Engine::new(Fonts::bundled_only());
    let layout = engine.layout_pageless(&doc);
    let scale = 2.0;
    let (w, h) = (((layout.width + 72.0) * scale) as u32, ((layout.height + 72.0) * scale) as u32);
    let mut scene = vello::Scene::new();
    let t = kurbo::Affine::scale(scale as f64) * kurbo::Affine::translate((36.0, 36.0));
    paint::paint(&mut scene, &layout, t, kurbo::Rect::new(-1e9, -1e9, 1e9, 1e9));
    let img = gpu.render(&scene, w, h).unwrap();
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xenify-write-equations.png");
    std::fs::write(&out, img.to_png()).unwrap();
    let dark = img.pixels.chunks(4).filter(|p| p[0] < 80 && p[1] < 80 && p[2] < 80).count();
    assert!(dark > 2000, "maths drawn: {dark}");
}

/// The visual equation editor's drawing: glyphs, empty boxes and caret places
/// (target/equation-edit.png; run with --ignored).
#[test]
#[ignore]
fn renders_the_equation_editor_view() {
    let mut gpu = Offscreen::new().expect("a GPU");
    let mut ed = xenify_math::edit::MathEdit::new("");
    for c in "E=1/2".chars() {
        ed.type_char(c);
    }
    ed.right();
    for c in "mv^2".chars() {
        ed.type_char(c);
    }
    ed.right();
    ed.type_char('+');
    ed.command("sqrt");
    let (t, b) = xenify_wp_layout::equation::typeset_edit(&ed, false, 40.0, Rgba::rgb(0, 0, 0)).unwrap();
    let (w, h) = (t.width + 40.0, t.ascent + t.descent + 40.0);
    let mut scene = vello::Scene::new();
    let tr = kurbo::Affine::translate((20.0, 20.0 + t.ascent as f64));
    scene.fill(
        vello::peniko::Fill::NonZero,
        kurbo::Affine::IDENTITY,
        vello::peniko::Color::WHITE,
        None,
        &kurbo::Rect::new(0.0, 0.0, w as f64, h as f64),
    );
    for it in &t.items {
        paint::paint_item(&mut scene, it, tr);
    }
    for p in &b.pieces {
        let (r, c) = match *p {
            xenify_math::Piece::Slot { x, y, w, h } => (
                kurbo::Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64),
                vello::peniko::Color::from_rgb8(0, 120, 255),
            ),
            xenify_math::Piece::Mark { x, y, h, .. } => (
                kurbo::Rect::new(x as f64, y as f64, x as f64 + 0.6, (y + h) as f64),
                vello::peniko::Color::from_rgb8(255, 80, 80),
            ),
            _ => continue,
        };
        scene.stroke(&kurbo::Stroke::new(0.8), tr, c, None, &r);
    }
    let img = gpu.render(&scene, w as u32, h as u32).unwrap();
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/equation-edit.png"), img.to_png()).unwrap();
    assert_eq!(ed.latex(), "E=\\frac{1}{2}mv^{2}+\\sqrt{}");
}
