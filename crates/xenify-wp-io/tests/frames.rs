use xenify_wp_doc::{Border, Document, Editor, Frame, Rgba, Wrap, frames, ops};
use xenify_wp_io::{Format, load, save};

fn doc_with_frames() -> Document {
    let mut ed = Editor::new(Document::from_plain_text("Body text\nMore body"));
    let a = frames::insert_frame(
        &mut ed,
        0,
        Frame {
            x: 360.0,
            y: 60.0,
            width: 180.0,
            height: 90.0,
            wrap: Wrap::Around,
            border: Some(Border { width: 1.0, color: Rgba::rgb(0x33, 0x33, 0x33) }),
            fill: Some(Rgba::rgb(0xf1, 0xf3, 0xf4)),
            ..Default::default()
        },
    )
    .unwrap();
    ops::insert_text(&mut ed, "ACME Ltd\n1 High Street", None).unwrap();
    let b = frames::insert_frame(
        &mut ed,
        0,
        Frame { x: 72.0, y: 600.0, width: 200.0, height: 60.0, wrap: Wrap::TopBottom, ..Default::default() },
    )
    .unwrap();
    ops::insert_text(&mut ed, "Footnote box", None).unwrap();
    let _ = (a, b);
    ed.doc
}

fn shape(d: &Document) -> Vec<String> {
    d.sections[0]
        .frames
        .iter()
        .map(|f| {
            let texts: Vec<String> = f
                .blocks
                .iter()
                .filter_map(|b| if let xenify_wp_doc::Block::Paragraph(p) = b { Some(p.text.clone()) } else { None })
                .collect();
            format!(
                "{:.1},{:.1} {:.1}x{:.1} {:?} border={:?} fill={:?} pad={} [{}]",
                f.x,
                f.y,
                f.width,
                f.height,
                f.wrap,
                f.border.map(|b| b.width),
                f.fill,
                f.padding,
                texts.join("|")
            )
        })
        .collect()
}

#[test]
fn frames_round_trip_markdown_and_docx() {
    let doc = doc_with_frames();
    for fmt in [Format::Markdown, Format::Docx, Format::Igdoc] {
        let bytes = save(&doc, fmt).unwrap();
        let back = load(&bytes, fmt).unwrap().doc;
        assert_eq!(
            shape(&back),
            shape(&doc),
            "{fmt:?}\n{}",
            String::from_utf8_lossy(&bytes).chars().take(1500).collect::<String>()
        );
        let body: Vec<_> = back.sections[0].blocks.iter().map(|b| b.id()).collect();
        assert_eq!(body.len(), 2, "{fmt:?}: body stays out of the frames");
    }
}

#[test]
fn markdown_threads() {
    let md = "::: {#a .frame x=72 y=72 width=100 height=40 next=b}\nStory text\n:::\n\n::: {#b .frame x=300 y=72 width=100 height=200}\n:::\n\nBody\n";
    let doc = load(md.as_bytes(), Format::Markdown).unwrap().doc;
    let f = &doc.sections[0].frames;
    assert_eq!(f.len(), 2);
    assert_eq!(f[0].next, Some(f[1].id));
    let again = String::from_utf8(save(&doc, Format::Markdown).unwrap()).unwrap();
    assert!(again.contains("next=frame2"), "{again}");
}
