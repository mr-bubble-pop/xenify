//! Marketing footage for Xenify Draw: three simple slides on white, in the
//! apps' colours. "Draw it.": the pen puts down two points, pulls out their
//! handles and the curve follows; the shape fills. "Animate it.": a ball
//! swings over an arc while the playhead crosses its two keyframes.
//! "Present it.": slides arrive one after another.
//!
//!   cargo run -p xenify-showcase --example drawn -- out.xdraw
//!
//! Everything is ordinary shapes, text and keyframes, as anyone could make
//! in Draw.

use kurbo::{BezPath, Point};
use xenify_doc::{Color, Easing, Layer, LayerKind, Transition, TransitionKind, Vec2};
use xenify_ops::Command;
use xenify_showcase::kit::*;

const PAPER: Color = Color::rgb(1.0, 1.0, 1.0);
const DARK: Color = Color::rgb(0.11, 0.098, 0.09);
const SOFT: Color = Color::rgb(0.34, 0.325, 0.306);
const FAINT: Color = Color::rgb(0.78, 0.76, 0.74);
const EDGE: Color = Color::rgb(0.886, 0.867, 0.843);
const ORANGE: Color = Color::rgb(0.761, 0.255, 0.047);
const NAVY: Color = Color::rgb(0.118, 0.227, 0.373);
const SUN: Color = Color::rgb(0.961, 0.722, 0.29);
const BLUE: Color = Color::rgb(0.31, 0.53, 0.86);

/// A slide on white.
fn slide(d: &mut Deck, name: &str, duration: f64, kind: TransitionKind, t: f64) {
    let tr = Transition { color: PAPER, ..Transition::new(kind, t) };
    let comp = d.slide(name, duration, &[], tr, "");
    let mut s = d.p.comp(comp).unwrap().settings.clone();
    s.background = PAPER;
    d.exec(Command::SetCompSettings { comp, settings: s });
}

/// The words on the left of each slide.
fn heading(d: &mut Deck, title: &str, line: &str) {
    let mut eyebrow = text_left("Eyebrow", "XENIFY DRAW", 28.0, ORANGE, v(170.0, 430.0));
    fade_in(&mut eyebrow, 0.1, 0.5);
    d.add(eyebrow);
    let mut t = text_left("Title", title, 132.0, DARK, v(164.0, 570.0));
    animate(&mut t, "Fade Up by Word", 0.2);
    d.add(t);
    let mut l = text_left("Line", line, 40.0, SOFT, v(170.0, 650.0));
    animate(&mut l, "Fade Up by Word", 0.7);
    d.add(l);
}

fn line(a: Point, b: Point) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

/// A square of `size`, turned to a diamond.
fn diamond(name: &str, size: f64, color: Color, at: Vec2) -> Layer {
    let r = size / 2.0;
    let mut p = BezPath::new();
    p.move_to((0.0, -r));
    p.line_to((r, 0.0));
    p.line_to((0.0, r));
    p.line_to((-r, 0.0));
    p.close_path();
    path(name, p, Some(color), None, at)
}

/// The pen tool's nib, its point at the layer's position.
fn nib(at: Vec2) -> Layer {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((14.0, 30.0));
    p.line_to((8.0, 44.0));
    p.line_to((-8.0, 44.0));
    p.line_to((-14.0, 30.0));
    p.close_path();
    let mut l = path("Pen", p, Some(DARK), Some((PAPER, 3.0)), at);
    l.transform.rotation = (-30.0).into();
    l
}

/// Fades out at `t` over 0.4 s (keeping whatever fade-in it has).
fn fade_out(l: &mut Layer, t: f64) {
    let a = l.opacity.value_at(t - 0.01);
    let mut k: Vec<(f64, f64)> = Vec::new();
    if l.opacity.is_animated() {
        k.push((t - 0.6, 1.0));
    }
    k.push((t, a));
    k.push((t + 0.4, 0.0));
    l.opacity = keys(&k, SMOOTH);
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "drawn.xdraw".into());
    let mut d = Deck::new("Draw it");

    // 1. Draw it: two points, their handles, the curve; then colour.
    slide(&mut d, "Draw it", 6.0, TransitionKind::Cut, 0.0);
    let a = Point::new(1080.0, 760.0);
    let b = Point::new(1620.0, 300.0);
    let (a_out, b_in, b_out, a_in) =
        (Point::new(1080.0, 470.0), Point::new(1330.0, 280.0), Point::new(1640.0, 600.0), Point::new(1380.0, 790.0));
    let mut leaf = BezPath::new();
    leaf.move_to(a);
    leaf.curve_to(a_out, b_in, b);
    leaf.curve_to(b_out, a_in, a);
    leaf.close_path();

    // The colour behind it, after the outline.
    let mut sun = ellipse("Sun", v(250.0, 250.0), SUN, v(1650.0, 330.0));
    pop(&mut sun, 3.3);
    d.add(sun);
    let mut fill = path("Leaf colour", leaf.clone(), Some(ORANGE), None, v(0.0, 0.0));
    fade_in(&mut fill, 2.7, 0.5);
    d.add(fill);
    let mut outline = path("Leaf", leaf, None, Some((DARK, 7.0)), v(0.0, 0.0));
    draw_on(&mut outline, 0.9, 1.7);
    d.add(outline);
    let mut vein = BezPath::new();
    vein.move_to((1130.0, 720.0));
    vein.curve_to((1260.0, 560.0), (1420.0, 430.0), (1570.0, 345.0));
    let mut vein = path("Vein", vein, None, Some((DARK, 5.0)), v(0.0, 0.0));
    draw_on(&mut vein, 3.0, 0.8);
    d.add(vein);
    let mut dot = ellipse("Dot", v(70.0, 70.0), NAVY, v(1170.0, 340.0));
    pop(&mut dot, 3.6);
    d.add(dot);

    // The handles, pulled out as the pen drags, then put away.
    for (name, from, to, t) in [
        ("Handle a", a, a_out, 0.5),
        ("Handle b", b, b_out, 1.6),
        ("Handle b in", b, b_in, 1.6),
        ("Handle a in", a, a_in, 0.5),
    ] {
        let mut h = path(name, line(from, to), None, Some((BLUE, 2.5)), v(0.0, 0.0));
        draw_on(&mut h, t, 0.45);
        fade_out(&mut h, 3.4);
        d.add(h);
        let mut end = ellipse(&format!("{name} end"), v(16.0, 16.0), BLUE, v(to.x, to.y));
        pop(&mut end, t + 0.35);
        fade_out(&mut end, 3.4);
        d.add(end);
    }
    for (name, p, t) in [("Point a", a, 0.35), ("Point b", b, 1.45)] {
        let mut sq = rect(name, v(22.0, 22.0), 3.0, PAPER, v(p.x, p.y));
        if let LayerKind::Shape { shapes } = &mut sq.kind {
            shapes[0].stroke = Some(stroke(BLUE, 3.0));
        }
        pop(&mut sq, t);
        fade_out(&mut sq, 3.4);
        d.add(sq);
    }
    // The pen: clicks a, drags its handle, clicks b, drags, comes back.
    let mut pen = nib(v(a.x, a.y));
    pen.transform.position = keys_e(&[
        (0.0, v(900.0, 900.0), SMOOTH),
        (0.35, v(a.x, a.y), SMOOTH),
        (0.5, v(a.x, a.y), SMOOTH),
        (0.95, v(a_out.x, a_out.y), SMOOTH),
        (1.45, v(b.x, b.y), SMOOTH),
        (1.6, v(b.x, b.y), SMOOTH),
        (2.05, v(b_out.x, b_out.y), SMOOTH),
        (2.6, v(a.x, a.y), SMOOTH),
        (3.2, v(1800.0, 900.0), Easing::Linear),
    ]);
    fade_out(&mut pen, 3.0);
    d.add(pen);
    heading(&mut d, "Draw it.", "Pens, shapes, text and colour.");

    // 2. Animate it: a ball swings over an arc (two rotation keyframes,
    // turning round the arc's middle) as the playhead crosses them.
    slide(&mut d, "Animate it", 6.0, TransitionKind::Push, 0.7);
    let (centre, r) = (Point::new(1410.0, 700.0), 350.0);
    // The arc it follows: half a circle, as two quarter curves.
    let k = 0.5523 * r;
    let mut route = BezPath::new();
    route.move_to((centre.x - r, centre.y));
    route.curve_to((centre.x - r, centre.y - k), (centre.x - k, centre.y - r), (centre.x, centre.y - r));
    route.curve_to((centre.x + k, centre.y - r), (centre.x + r, centre.y - k), (centre.x + r, centre.y));
    let mut dashed = path("Arc", route, None, Some((FAINT, 4.0)), v(0.0, 0.0));
    if let LayerKind::Shape { shapes } = &mut dashed.kind {
        shapes[0].stroke.as_mut().unwrap().dashes = vec![2.0, 16.0];
    }
    draw_on(&mut dashed, 0.2, 0.8);
    d.add(dashed);
    // The timeline under it, with the two keyframes.
    let (tl_y, x0, x1) = (880.0, 1060.0, 1760.0);
    let mut track =
        path("Timeline", line(Point::new(x0, tl_y), Point::new(x1, tl_y)), None, Some((EDGE, 4.0)), v(0.0, 0.0));
    fade_in(&mut track, 0.2, 0.4);
    d.add(track);
    let (go, back) = ((0.6, 2.8), (3.4, 5.6));
    for (i, (x, t)) in [(x0, go.0), (x1, go.1)].iter().enumerate() {
        let mut key = diamond(&format!("Keyframe {}", i + 1), 26.0, NAVY, v(*x, tl_y));
        pop(&mut key, *t);
        d.add(key);
    }
    let mut head =
        path("Playhead", line(Point::new(0.0, -34.0), Point::new(0.0, 34.0)), None, Some((ORANGE, 4.0)), v(x0, tl_y));
    head.transform.position =
        keys(&[(go.0, v(x0, tl_y)), (go.1, v(x1, tl_y)), (back.0, v(x1, tl_y)), (back.1, v(x0, tl_y))], Easing::Linear);
    fade_in(&mut head, 0.3, 0.3);
    d.add(head);
    let mut ball = ellipse("Ball", v(96.0, 96.0), ORANGE, v(centre.x, centre.y));
    // The ball sits r to the left of the point it turns round.
    ball.transform.anchor = v(r, 0.0).into();
    ball.transform.rotation = keys(&[(go.0, 0.0), (go.1, 180.0), (back.0, 180.0), (back.1, 0.0)], SMOOTH);
    fade_in(&mut ball, 0.3, 0.3);
    d.add(ball);
    heading(&mut d, "Animate it.", "Keyframes, easing and timing.");

    // 3. Present it: slides arrive in turn.
    slide(&mut d, "Present it", 5.0, TransitionKind::Push, 0.7);
    let cards: [(f64, f64, f64, Color); 3] =
        [(1180.0, 380.0, -5.0, SUN), (1420.0, 540.0, 0.0, NAVY), (1660.0, 700.0, 5.0, ORANGE)];
    for (i, (x, y, turn, colour)) in cards.into_iter().enumerate() {
        let t = 0.5 + i as f64 * 0.35;
        let at = v(x, y);
        let mut card = rect(&format!("Slide {}", i + 1), v(440.0, 248.0), 22.0, PAPER, at);
        if let LayerKind::Shape { shapes } = &mut card.kind {
            shapes[0].stroke = Some(stroke(EDGE, 3.0));
        }
        let mut blob = ellipse(&format!("Slide {} picture", i + 1), v(110.0, 110.0), colour, at + v(-120.0, 0.0));
        let mut bar1 = rect(&format!("Slide {} title", i + 1), v(170.0, 22.0), 11.0, DARK, at + v(70.0, -26.0));
        let mut bar2 = rect(&format!("Slide {} text", i + 1), v(170.0, 16.0), 8.0, FAINT, at + v(70.0, 22.0));
        for l in [&mut card, &mut blob, &mut bar1, &mut bar2] {
            slide_in(l, t, v(420.0, 60.0), 0.8);
            let p = l.transform.position.value_at(t + 0.8);
            // Each card turns a little as it settles, around the card's middle.
            l.transform.anchor = (at - p).into();
            l.transform.position = keys(&[(t, at + v(420.0, 60.0)), (t + 0.8, at)], EXPO);
            l.transform.rotation = keys(&[(t, turn * 3.0), (t + 0.8, turn)], EXPO);
        }
        for l in [card, blob, bar1, bar2] {
            d.add(l);
        }
    }
    heading(&mut d, "Present it.", "Slides, transitions and video.");

    d.finish_slide();
    std::fs::write(&out, d.p.to_json()).expect("couldn't write the canvas");
    println!("wrote {out}");
}
