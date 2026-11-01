use xenify_wp_doc::{Block, Document, Editor, Frame, Wrap, frames, ops};
use xenify_wp_layout::{DocLayout, Engine, Fonts};

const PARA: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut \\
                    labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco.";

fn layout(doc: &Document) -> DocLayout {
    Engine::new(Fonts::bundled_only()).layout(doc)
}

fn body_rects(doc: &Document, l: &DocLayout) -> Vec<kurbo::Rect> {
    let body: Vec<_> = doc.sections[0].blocks.iter().map(Block::id).collect();
    let mut v = Vec::new();
    for id in body {
        let Some(frags) = l.by_para.get(&id) else { continue };
        for &i in frags {
            let f = &l.fragments[i];
            // Each line's own box (its glyph extent), in document space.
            for li in f.lines.clone() {
                let line = &f.para.layout.lines().nth(li).unwrap();
                let m = line.metrics();
                let x0 = f.origin.x + (f.para.indent + m.offset + m.inline_min_coord) as f64;
                let x1 = x0 + (m.advance - m.trailing_whitespace) as f64;
                let y0 = f.origin.y + m.block_min_coord as f64;
                let y1 = f.origin.y + m.block_max_coord as f64;
                v.push(kurbo::Rect::new(x0, y0, x1, y1));
            }
        }
    }
    v
}

fn with_frame(wrap: Wrap) -> (Editor, xenify_wp_doc::NodeId) {
    let mut ed = Editor::new(Document::from_plain_text(&[PARA; 6].join("\\n")));
    let id = frames::insert_frame(
        &mut ed,
        0,
        Frame { x: 300.0, y: 100.0, width: 200.0, height: 150.0, wrap, ..Default::default() },
    )
    .unwrap();
    ops::insert_text(&mut ed, "In the box", None).unwrap();
    (ed, id)
}

#[test]
fn text_wraps_around_a_frame() {
    let (ed, id) = with_frame(Wrap::Around);
    let l = layout(&ed.doc);
    let frame = l.frames.iter().find(|f| f.id == id).unwrap().rect;
    let keep_out = frame.inflate(7.9, 7.9);
    let lines = body_rects(&ed.doc, &l);
    for r in &lines {
        assert!(r.intersect(keep_out).area() <= 0.01, "line {r:?} runs into the frame {frame:?}");
    }
    // Some lines sit beside the frame (shorter), not only above and below it.
    assert!(lines.iter().any(|r| r.y0 < frame.y1 && r.y1 > frame.y0), "text flows beside the frame");
    // The frame's own text is inside it.
    let p = ed.doc.frame(id).unwrap().blocks[0].id();
    let fr = l.fragments[l.by_para[&p][0]].rect();
    assert!(frame.contains(fr.origin()), "{fr:?} in {frame:?}");
}

#[test]
fn top_bottom_frames_push_text_below() {
    let (ed, id) = with_frame(Wrap::TopBottom);
    let l = layout(&ed.doc);
    let frame = l.frames.iter().find(|f| f.id == id).unwrap().rect;
    for r in body_rects(&ed.doc, &l) {
        assert!(r.y1 <= frame.y0 - 7.9 || r.y0 >= frame.y1 + 7.9, "line {r:?} beside a top/bottom frame {frame:?}");
    }
    // In front: text ignores it.
    let (ed, id) = with_frame(Wrap::InFront);
    let l = layout(&ed.doc);
    let frame = l.frames.iter().find(|f| f.id == id).unwrap().rect;
    assert!(body_rects(&ed.doc, &l).iter().any(|r| r.intersect(frame).area() > 1.0));
}

#[test]
fn overflow_and_threads() {
    let mut ed = Editor::new(Document::from_plain_text("Body"));
    let a = frames::insert_frame(
        &mut ed,
        0,
        Frame { x: 72.0, y: 300.0, width: 150.0, height: 60.0, wrap: Wrap::InFront, ..Default::default() },
    )
    .unwrap();
    ops::insert_text(&mut ed, &[PARA; 2].join(" "), None).unwrap();
    let l = layout(&ed.doc);
    assert!(l.frames[0].overflow, "too much text for the box");
    let b = frames::insert_frame(
        &mut ed,
        0,
        Frame { x: 300.0, y: 500.0, width: 200.0, height: 300.0, wrap: Wrap::InFront, ..Default::default() },
    )
    .unwrap();
    frames::update_frame(&mut ed, a, "Link", |f| f.next = Some(b)).unwrap();
    let l = layout(&ed.doc);
    let fa = l.frames.iter().find(|f| f.id == a).unwrap();
    let fb = l.frames.iter().find(|f| f.id == b).unwrap();
    assert!(!fa.overflow && !fb.overflow && fb.threaded_in);
    // The story's paragraph continues in the second frame.
    let p = ed.doc.frame(a).unwrap().blocks[0].id();
    let pieces: Vec<_> = l.by_para[&p].iter().map(|&i| l.fragments[i].rect()).collect();
    assert_eq!(pieces.len(), 2);
    assert!(fa.rect.contains(pieces[0].center()) && fb.rect.contains(pieces[1].center()));
}

#[test]
fn first_line_pushed_below_a_frame_stays_below() {
    // A full-width banner at the very top of the text area.
    let mut ed = Editor::new(Document::from_plain_text("Body under the banner"));
    let ps = ed.doc.sections[0].page.clone();
    let id = frames::insert_frame(
        &mut ed,
        0,
        Frame {
            x: ps.margins.left,
            y: ps.margins.top,
            width: ps.content_width(),
            height: 80.0,
            wrap: Wrap::TopBottom,
            ..Default::default()
        },
    )
    .unwrap();
    let l = layout(&ed.doc);
    let frame = l.frames.iter().find(|f| f.id == id).unwrap().rect;
    for r in body_rects(&ed.doc, &l) {
        assert!(r.y0 >= frame.y1, "line {r:?} should be below the banner {frame:?}");
    }
}

#[test]
fn a_click_anywhere_in_a_text_box_goes_into_it() {
    let (mut ed, id) = with_frame(Wrap::TopBottom);
    ops::insert_text(&mut ed, "Boxed", None).unwrap();
    let l = layout(&ed.doc);
    let fr = l.frames.iter().find(|f| f.id == id).unwrap().rect;
    let inside = ed.doc.frame(id).unwrap().blocks[0].id();
    // Below its one line, near the bottom of the box.
    let pos = l.hit(kurbo::Point::new(fr.center().x, fr.y1 - 4.0)).unwrap();
    assert_eq!(pos.para, inside, "frame {fr:?}");
}
