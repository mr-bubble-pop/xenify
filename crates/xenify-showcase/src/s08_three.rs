//! Slide 8: 3D + AI: a gear made in code spins and comes apart; an AI prompt
//! types itself and a slide builds (artwork here; the real feature is
//! AI › New Slide).

use crate::kit::*;
use xenify_audio::synth::Sfx;
use xenify_doc::templates::{Content, Layout, THEMES};
use xenify_doc::{Easing, Id, Layer, LayerKind, Solid, SolidShape, TransitionKind, Vec2};
use kurbo::Rect;

fn caption(d: &mut Deck, t: &str, t0: f64, t1: f64) {
    let mut c = text("Caption", t, 40.0, MUTED, v(W / 2.0, 1030.0));
    animate(&mut c, "Fade Up by Word", t0 + 0.2);
    c.opacity = keys(&[(t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    live(&mut c, 0.0, Some(t1 + 0.35));
    d.add(c);
}

fn solid(name: &str, shape: SolidShape, size: f64, depth: f64, color: xenify_doc::Color, at: Vec2) -> Layer {
    let mut s = Solid::new(shape, size, depth, color);
    s.rot_x = 25.0.into();
    s.perspective = 0.5;
    Layer::new(Id(0), name, LayerKind::Solid(Box::new(s))).with_position(at)
}

pub fn build(d: &mut Deck) {
    let card = crate::s05_slides::mini(
        d,
        Layout::Bullets,
        &THEMES[0],
        &Content {
            title: "Our Launch".into(),
            subtitle: String::new(),
            items: vec!["Ships Monday".into(), "Free to start".into(), "Built for teams".into()],
        },
    );
    let pauses = [3.4, 7.6];
    d.slide(
        "3D and AI",
        8.0,
        &pauses,
        tr(TransitionKind::ParticleDissolve, 1.2),
        "3D from code or files; AI that builds editable slides.",
    );
    let mut title = text_left("Title", "3D and AI", 120.0, INK, v(120.0, 200.0));
    animate(&mut title, "Fade Up by Word", 0.0);
    d.add(title);

    // A. A gear spins and comes apart.
    let (t0, t1) = (0.0, 3.5);
    let since = d.next_id();
    let centre = v(W / 2.0, 580.0);
    let spin = |l: &mut Layer, from: f64| {
        if let LayerKind::Solid(s) = &mut l.kind {
            // Comes to rest before the pause.
            s.rot_y = keys(&[(0.0, from), (3.2, from + 300.0)], SMOOTH);
        }
    };
    let gear = xenify_ops::draw::gear_path(14, 0.18, 0.28, Rect::new(-220.0, -220.0, 220.0, 220.0));
    let mut big = solid(
        "Gear",
        SolidShape::Extrude { path: gear, fill_rule: xenify_doc::FillRule::EvenOdd },
        440.0,
        80.0,
        PINK,
        centre,
    );
    spin(&mut big, 0.0);
    big.transform.position = keys(&[(1.8, centre), (2.6, centre + v(-330.0, 0.0))], EXPO);
    d.add(big);
    let small = xenify_ops::draw::gear_path(9, 0.22, 0.3, Rect::new(-120.0, -120.0, 120.0, 120.0));
    let mut sm = solid(
        "Small gear",
        SolidShape::Extrude { path: small, fill_rule: xenify_doc::FillRule::EvenOdd },
        240.0,
        60.0,
        VIOLET,
        centre,
    );
    spin(&mut sm, 30.0);
    sm.transform.position = keys(&[(1.8, centre), (2.6, centre + v(330.0, -40.0))], EXPO);
    sm.opacity = keys(&[(1.8, 0.0), (1.9, 1.0)], Easing::Linear);
    d.add(sm);
    let mut axle = solid("Axle", SolidShape::Cylinder, 70.0, 260.0, CYAN, centre);
    spin(&mut axle, 0.0);
    axle.transform.position = keys(&[(1.8, centre), (2.6, centre + v(0.0, 40.0))], EXPO);
    axle.opacity = keys(&[(1.8, 0.0), (1.9, 1.0)], Easing::Linear);
    d.add(axle);
    d.sfx(Sfx::Whoosh, 1.8, 2.0, 0.6);
    caption(d, "3D: shapes, extrusions and imported models, turned and taken apart", t0, t1);
    d.end_since(since, t1 + 0.35);

    // B. The AI prompt.
    let (t0, t1) = (3.7, 8.0);
    let since = d.next_id();
    let mut box_ = rect("Prompt box", v(1100.0, 110.0), 55.0, PANEL, v(W / 2.0, 330.0));
    if let LayerKind::Shape { shapes } = &mut box_.kind {
        let mut st = stroke(VIOLET, 3.0);
        st.gradient = Some(accent_gradient(1100.0));
        shapes[0].stroke = Some(st);
    }
    slide_in(&mut box_, t0, v(0.0, 40.0), 0.6);
    live(&mut box_, t0, None);
    d.add(box_);
    let mut prompt = text_left("Prompt", "Make a slide about our launch", 44.0, INK, v(480.0, 346.0));
    animate(&mut prompt, "Typewriter", 0.4);
    live(&mut prompt, t0, None);
    d.add(prompt);
    let mut go = logo(56.0, v(1440.0, 330.0));
    pop(&mut go, t0 + 2.2);
    d.add(go);
    let built = t0 + 2.6;
    let id = d.place(card, "Built slide", v(W / 2.0, 690.0), 0.36, built);
    d.edit(id, |l| {
        l.transform.scale = keys(&[(built, v(0.3, 0.3)), (built + 0.6, v(0.36, 0.36))], EXPO);
        l.opacity = keys(&[(built, 0.0), (built + 0.3, 1.0)], Easing::Linear);
        live(l, built, None);
    });
    d.sfx(Sfx::Chime, built, 0.0, 0.6);
    caption(d, "AI › New Slide builds real, editable slides from a sentence", t0, t1);
    let _ = since;
}
