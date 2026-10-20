//! Slide 2: Save time: one app instead of five. A card per job pops in with
//! a tick (a click each), each with its own little drawing; the last is the
//! logo: all in one app.

use crate::kit::*;
use xenify_audio::synth::Sfx;
use xenify_doc::{Easing, LayerKind, TransitionKind, Vec2};
use kurbo::BezPath;

const ITEMS: [&str; 5] = ["Vector art", "Slideshows", "Motion graphics", "Movie editing", "All in one app"];
const CARD: (f64, f64) = (300.0, 340.0);

/// A small line drawing for card `k`, about 140 px across, centred on 0.
fn icon(k: usize) -> Vec<(BezPath, bool)> {
    let mut out = Vec::new();
    let line = |pts: &[(f64, f64)]| {
        let mut p = BezPath::new();
        p.move_to(pts[0]);
        for q in &pts[1..] {
            p.line_to(*q);
        }
        p
    };
    match k {
        // A pen curve with two nodes and a handle.
        0 => {
            let mut c = BezPath::new();
            c.move_to((-70.0, 40.0));
            c.curve_to((-40.0, -70.0), (40.0, 70.0), (70.0, -40.0));
            out.push((c, false));
            out.push((line(&[(-70.0, 40.0), (-40.0, -70.0)]), false));
            out.push((kurbo::Shape::to_path(&kurbo::Circle::new((-40.0, -70.0), 8.0), 0.1), true));
        }
        // A slide: a frame, a title bar and two lines.
        1 => {
            out.push((kurbo::Shape::to_path(&kurbo::RoundedRect::new(-75.0, -50.0, 75.0, 50.0, 8.0), 0.1), false));
            out.push((line(&[(-50.0, -22.0), (20.0, -22.0)]), false));
            out.push((line(&[(-50.0, 5.0), (50.0, 5.0)]), false));
            out.push((line(&[(-50.0, 25.0), (30.0, 25.0)]), false));
        }
        // A ball with motion lines.
        2 => {
            out.push((kurbo::Shape::to_path(&kurbo::Circle::new((30.0, 0.0), 32.0), 0.1), true));
            for (y, len) in [(-22.0, 60.0), (0.0, 80.0), (22.0, 60.0)] {
                out.push((line(&[(-20.0, y), (-20.0 - len, y)]), false));
            }
        }
        // A film strip with a play mark.
        3 => {
            out.push((kurbo::Shape::to_path(&kurbo::RoundedRect::new(-75.0, -45.0, 75.0, 45.0, 6.0), 0.1), false));
            for x in [-55.0, -25.0, 5.0, 35.0] {
                out.push((kurbo::Shape::to_path(&kurbo::Rect::new(x, -38.0, x + 14.0, -28.0), 0.1), true));
                out.push((kurbo::Shape::to_path(&kurbo::Rect::new(x, 28.0, x + 14.0, 38.0), 0.1), true));
            }
            let mut play = BezPath::new();
            play.move_to((-10.0, -15.0));
            play.line_to((18.0, 0.0));
            play.line_to((-10.0, 15.0));
            play.close_path();
            out.push((play, true));
        }
        _ => {}
    }
    out
}

pub fn build(d: &mut Deck) {
    let t_k = |k: usize| 0.6 + k as f64;
    // A click before each card after the first.
    let pauses: Vec<f64> = (1..ITEMS.len()).map(|k| t_k(k) - 0.05).collect();
    let end = t_k(ITEMS.len() - 1) + 2.2;
    d.slide(
        "Save time",
        end,
        &pauses,
        tr(TransitionKind::Shatter, 1.2),
        "One app instead of five. A click brings in each.",
    );
    let mut title = text_left("Title", "Save time", 120.0, INK, v(120.0, 200.0));
    animate(&mut title, "Fade Up by Word", 0.0);
    d.add(title);

    let gap = 50.0;
    let row = ITEMS.len() as f64 * CARD.0 + (ITEMS.len() - 1) as f64 * gap;
    for (k, item) in ITEMS.iter().enumerate() {
        let t = t_k(k);
        let last = k + 1 == ITEMS.len();
        let at = v(W / 2.0 - row / 2.0 + CARD.0 / 2.0 + k as f64 * (CARD.0 + gap), 560.0);
        // A group per card (its opacity fades everything in it together).
        let mut group = xenify_doc::Layer::new(xenify_doc::Id(0), *item, LayerKind::Group).with_position(at);
        group.transform.position = keys(&[(t, at + v(0.0, 60.0)), (t + 0.7, at)], EXPO);
        group.transform.scale = keys(&[(t, v(0.85, 0.85)), (t + 0.6, v(1.0, 1.0))], BACK);
        group.opacity = keys(&[(t, 0.0), (t + 0.2, 1.0)], Easing::Linear);
        let group = d.add(group);
        // The card: a panel with the accent outline (filled for the last one).
        let mut card = rect(&format!("{item} card"), v(CARD.0, CARD.1), 28.0, PANEL, v(0.0, 0.0));
        card.meta.parent = Some(group);
        if let LayerKind::Shape { shapes } = &mut card.kind {
            let mut st = stroke(PINK, if last { 0.0 } else { 3.0 });
            st.gradient = Some(accent_gradient(CARD.0));
            shapes[0].stroke = Some(st);
            if last {
                shapes[0].fill = Some(gradient_fill(CARD.0));
            }
        }
        d.add(card);
        let card = group;
        // Its drawing and its name, riding on it.
        let parts = icon(k);
        if last {
            let mut mark = logo(150.0, v(0.0, -40.0));
            mark.meta.parent = Some(card);
            mark.transform.rotation = keys(&[(t + 0.2, -90.0), (t + 1.0, 0.0)], SPRING);
            if let LayerKind::Shape { shapes } = &mut mark.kind {
                shapes[0].fill = Some(xenify_doc::Fill::solid(INK));
            }
            d.add(mark);
        }
        for (i, (p, solid)) in parts.into_iter().enumerate() {
            let color = if i == 0 { INK } else { CYAN };
            let mut l = path(
                &format!("{item} drawing"),
                p,
                solid.then_some(color),
                (!solid).then_some((color, 6.0)),
                v(0.0, -40.0),
            );
            l.meta.parent = Some(card);
            if !solid {
                draw_on(&mut l, t + 0.2, 0.7);
            }
            d.add(l);
        }
        let mut label = text(&format!("{item} label"), item, 34.0, INK, v(0.0, 110.0));
        label.meta.parent = Some(card);
        d.add(label);
        d.sfx(if k % 2 == 0 { Sfx::Tick } else { Sfx::Tock }, t, 0.0, 0.8);
    }
    let mut caption = text("Caption", "One app instead of five", 48.0, MUTED, v(W / 2.0, 900.0));
    animate(&mut caption, "Fade Up by Word", t_k(ITEMS.len() - 1) + 0.6);
    d.add(caption);
    d.sfx(Sfx::Chime, t_k(ITEMS.len() - 1) + 0.3, 0.0, 0.6);
    let _ = Vec2::ZERO;
}
