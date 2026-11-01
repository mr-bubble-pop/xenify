//! Slide 4: Motion: keyframe diamonds pop onto a timeline while a graph-editor
//! curve draws and a ball rides it; a physics bounce with squash; an IK arm
//! waves; a text animator.

use crate::kit::*;
use kurbo::{BezPath, ParamCurve};
use xenify_doc::{Easing, TransitionKind, Vec2};
use xenify_ops::presets::Preset;

fn caption(d: &mut Deck, t: &str, t0: f64, t1: f64) {
    let mut c = text("Caption", t, 40.0, MUTED, v(W / 2.0, 1000.0));
    animate(&mut c, "Fade Up by Word", t0 + 0.2);
    c.opacity = keys(&[(t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    live(&mut c, 0.0, Some(t1 + 0.35));
    d.add(c);
}

fn window(l: &mut xenify_doc::Layer, t0: f64, t1: f64) {
    live(l, t0, Some(t1 + 0.35));
    if matches!(l.opacity, xenify_doc::Property::Static(_)) {
        l.opacity = keys(&[(t0, 0.0), (t0 + 0.25, 1.0), (t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    }
}

pub fn build(d: &mut Deck) {
    let pauses = [2.7, 5.3, 7.9, 10.6];
    d.slide("Motion", 11.0, &pauses, tr(TransitionKind::ClockWipe, 1.0), "Keyframes, graphs, physics, rigs, text.");
    let mut title = text_left("Title", "Motion", 120.0, INK, v(120.0, 200.0));
    animate(&mut title, "Fade Up by Word", 0.0);
    d.add(title);

    // A. The timeline and the graph editor.
    let end_a = 2.8;
    let mut panel = rect("Timeline", v(1500.0, 150.0), 20.0, PANEL, v(W / 2.0, 850.0));
    window(&mut panel, 0.0, end_a);
    d.add(panel);
    let mut track = rect("Track", v(1380.0, 4.0), 2.0, LINE, v(W / 2.0, 850.0));
    window(&mut track, 0.0, end_a);
    d.add(track);
    for k in 0..7 {
        let t = 0.3 + k as f64 * 0.16;
        let mut key = rect(
            "Keyframe",
            v(26.0, 26.0),
            3.0,
            if k % 3 == 0 { PINK } else { GOLD },
            v(330.0 + k as f64 * 210.0, 850.0),
        );
        key.transform.rotation = 45.0.into();
        pop(&mut key, t);
        key.opacity = keys(&[(t, 0.0), (t + 0.15, 1.0), (end_a, 1.0), (end_a + 0.3, 0.0)], Easing::Linear);
        live(&mut key, 0.0, Some(end_a + 0.35));
        d.add(key);
    }
    let mut head = rect("Playhead", v(4.0, 150.0), 2.0, PINK, v(270.0, 850.0));
    head.transform.position = keys(&[(0.4, v(270.0, 850.0)), (2.6, v(1650.0, 850.0))], Easing::Linear);
    window(&mut head, 0.4, end_a);
    d.add(head);
    // The curve (an ease-in-out) and a ball riding it.
    let (x0, y0, gw, gh) = (560.0, 700.0, 800.0, 380.0);
    let mut curve = BezPath::new();
    curve.move_to((x0, y0));
    curve.curve_to((x0 + gw * 0.55, y0), (x0 + gw * 0.45, y0 - gh), (x0 + gw, y0 - gh));
    let mut axes = BezPath::new();
    axes.move_to((x0, y0 - gh - 20.0));
    axes.line_to((x0, y0));
    axes.line_to((x0 + gw + 20.0, y0));
    let mut ax = path("Graph axes", axes, None, Some((LINE, 4.0)), v(0.0, 0.0));
    window(&mut ax, 0.2, end_a);
    d.add(ax);
    let mut g = path("Graph curve", curve.clone(), None, Some((PINK, 7.0)), v(0.0, 0.0));
    draw_on(&mut g, 0.5, 1.0);
    window(&mut g, 0.2, end_a);
    d.add(g);
    let seg = curve.segments().next().unwrap();
    let ride: Vec<(f64, Vec2)> = (0..=24)
        .map(|i| {
            let u = i as f64 / 24.0;
            let p = seg.eval(u);
            (1.2 + u * 1.3, v(p.x, p.y))
        })
        .collect();
    let mut ball = ellipse("Riding ball", v(36.0, 36.0), GOLD, ride[0].1);
    ball.transform.position = keys(&ride, Easing::Linear);
    window(&mut ball, 1.1, end_a);
    d.add(ball);
    caption(d, "Keyframes, easing and a graph editor for every property", 0.0, end_a);

    // B. A physics bounce with squash and stretch.
    let (t0, t1) = (3.0, 5.4);
    let mut floor = rect("Floor", v(900.0, 6.0), 3.0, LINE, v(W / 2.0, 800.0));
    window(&mut floor, t0, t1);
    d.add(floor);
    let mut b = ellipse("Bouncing ball", v(140.0, 140.0), PINK, v(W / 2.0, 730.0));
    if let xenify_doc::LayerKind::Shape { shapes } = &mut b.kind {
        shapes[0].fill = Some(gradient_fill(140.0));
    }
    b.transform.anchor = v(0.0, 70.0).into();
    b.transform.position = v(W / 2.0, 800.0).into();
    window(&mut b, t0, t1);
    let ball = d.add(b);
    let cmd =
        xenify_ops::presets::apply(&d.p, ball, Preset::Bounce { duration: 1.8, height: 420.0, bounces: 3 }, t0 + 0.1)
            .unwrap();
    d.exec(cmd);
    let cmd = xenify_ops::presets::apply(&d.p, ball, Preset::SquashStretch { strength: 0.3 }, t0).unwrap();
    d.exec(cmd);
    for k in 0..4 {
        d.sfx(
            xenify_audio::synth::Sfx::Pop,
            t0 + 0.1 + [0.62, 1.0, 1.32, 1.6][k],
            -4.0 - k as f64 * 2.0,
            0.5 - k as f64 * 0.1,
        );
    }
    caption(d, "Physics, bounces and squash & stretch in a click", t0, t1);

    // C. An arm on an IK joint chain waves.
    let (t0, t1) = (5.6, 8.0);
    let shoulder = v(820.0, 780.0);
    let seg = |name: &str, len: f64, w: f64, at: Vec2, color| {
        let mut l = rect(name, v(w, len), w / 2.0, color, at);
        if let xenify_doc::LayerKind::Shape { shapes } = &mut l.kind
            && let xenify_doc::Geometry::Rect { position, .. } = &mut shapes[0].geometry
        {
            *position = v(0.0, -len / 2.0 + w / 2.0).into();
        }
        window(&mut l, t0, t1);
        l
    };
    let upper = d.add(seg("Upper arm", 240.0, 60.0, shoulder, VIOLET));
    let fore = d.add(seg("Forearm", 220.0, 50.0, shoulder + v(0.0, -210.0), PINK));
    let hand = d.add(seg("Hand", 90.0, 70.0, shoulder + v(0.0, -410.0), GOLD));
    let cmd = xenify_ops::rig::make_joint_chain(&d.p, d.comp, &[upper, fore, hand], t0).unwrap();
    d.exec(cmd);
    let wave = |a: f64, b: f64| -> Vec<(f64, f64)> {
        (0..7).map(|k| (t0 + 0.2 + k as f64 * 0.32, if k % 2 == 0 { a } else { b })).collect()
    };
    d.edit(upper, |l| l.transform.rotation = keys(&wave(20.0, 32.0), SMOOTH));
    d.edit(fore, |l| l.transform.rotation = keys(&wave(-10.0, 30.0), SMOOTH));
    d.edit(hand, |l| l.transform.rotation = keys(&wave(-20.0, 25.0), SMOOTH));
    let mut shoulder_dot = ellipse("Shoulder", v(70.0, 70.0), INK, shoulder);
    window(&mut shoulder_dot, t0, t1);
    d.add(shoulder_dot);
    caption(d, "Rigs: parented joints with inverse kinematics", t0, t1);

    // D. A text animator.
    let (t0, t1) = (8.2, 10.8);
    let mut a = text("Animated text", "Every letter can move", 110.0, INK, v(W / 2.0, 520.0));
    animate(&mut a, "Pop by Letter", 0.1);
    window(&mut a, t0, t1);
    d.add(a);
    let mut b = text("Animated text 2", "or every word, or every line", 64.0, CYAN, v(W / 2.0, 650.0));
    animate(&mut b, "Fade Up by Word", 1.0);
    window(&mut b, t0, t1);
    d.add(b);
    caption(d, "Text animators: by letter, word or line", t0, t1);
}
