//! Slide 5: Slides: mini slides (symbols built from the templates) fan out as
//! cards and show off transitions (wipe, iris, liquid, shatter); a
//! presenter view; the templates gallery.

use crate::kit::*;
use xenify_doc::templates::{self, Content, Layout, THEMES, Theme};
use xenify_doc::{Easing, Id, Mask, Modifier, TransitionKind};
use kurbo::Rect;

/// A 1920×1080 symbol holding `layout` in `theme`.
pub fn mini(d: &mut Deck, layout: Layout, theme: &Theme, content: &Content) -> Id {
    let built = templates::build(layout, theme, W, H, 1, content);
    let comp =
        d.symbol(&format!("{} ({})", layout.label(), theme.name), W, H, built.duration, xenify_doc::Color::TRANSPARENT);
    for l in built.layers {
        d.add_to(comp, l);
    }
    comp
}

fn window(l: &mut xenify_doc::Layer, t0: f64, t1: f64) {
    live(l, t0, Some(t1 + 0.35));
    if matches!(l.opacity, xenify_doc::Property::Static(_)) {
        l.opacity = keys(&[(t0, 0.0), (t0 + 0.25, 1.0), (t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    }
}

fn caption(d: &mut Deck, t: &str, t0: f64, t1: f64) {
    let mut c = text("Caption", t, 40.0, MUTED, v(W / 2.0, 1000.0));
    animate(&mut c, "Fade Up by Word", t0 + 0.2);
    c.opacity = keys(&[(t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    live(&mut c, 0.0, Some(t1 + 0.35));
    d.add(c);
}

pub fn build(d: &mut Deck) {
    let neon = &THEMES[0];
    let content = |t: &str, s: &str| Content { title: t.into(), subtitle: s.into(), items: vec![] };
    let cards = [
        (mini(d, Layout::Title, neon, &content("Launch Day", "Everything, in one app")), "Wipe"),
        (mini(d, Layout::BigStat, neon, &content("3×", "faster from idea to screen")), "Iris"),
        (mini(d, Layout::Quote, neon, &content("It just flows.", "— every early user")), "Liquid"),
        (mini(d, Layout::Process, neon, &Content::default()), "Shatter"),
    ];
    let now = mini(d, Layout::Bullets, &THEMES[2], &Content::default());
    let next = mini(d, Layout::Closing, &THEMES[2], &Content::default());
    let gallery: Vec<Id> = THEMES.iter().map(|t| mini(d, Layout::Title, t, &content(t.name, "A theme"))).collect();

    let pauses = [3.3, 6.2, 9.1];
    d.slide(
        "Slides",
        9.5,
        &pauses,
        tr(TransitionKind::Push, 0.8),
        "Decks with real transitions, a presenter view and themes.",
    );
    let mut title = text_left("Title", "Slides", 120.0, INK, v(120.0, 200.0));
    animate(&mut title, "Fade Up by Word", 0.0);
    d.add(title);

    // A. Cards fan out, each showing a transition.
    let (t0, t1) = (0.2, 3.4);
    for (k, (comp, name)) in cards.iter().enumerate() {
        let x = 420.0 + k as f64 * 360.0;
        let t = t0 + k as f64 * 0.15;
        let id = d.place(*comp, &format!("{name} card"), v(W / 2.0, 600.0), 0.17, t + 0.8);
        let reveal = t + 1.1;
        let full = Rect::new(0.0, 0.0, W, H);
        d.edit(id, |l| {
            l.transform.position = keys(&[(t, v(W / 2.0, 620.0)), (t + 0.8, v(x, 580.0))], EXPO);
            l.transform.rotation = keys(&[(t, -12.0 + k as f64 * 8.0), (t + 0.8, 0.0)], EXPO);
            window(l, t, t1);
            match *name {
                "Wipe" => {
                    let mut m = Mask::rect("Wipe", Rect::new(0.0, 0.0, 1.0, H));
                    m.path = keys(
                        &[
                            (reveal, xenify_doc::rect_path(Rect::new(0.0, 0.0, 1.0, H))),
                            (reveal + 0.8, xenify_doc::rect_path(full)),
                        ],
                        SMOOTH,
                    );
                    l.meta.masks.push(m);
                }
                "Iris" => {
                    let c = kurbo::Point::new(W / 2.0, H / 2.0);
                    let circle = |r: f64| kurbo::Shape::to_path(&kurbo::Ellipse::new(c, (r, r), 0.0), 0.1);
                    let mut m = Mask::new("Iris", circle(1.0));
                    m.path = keys(&[(reveal, circle(1.0)), (reveal + 0.9, circle(1200.0))], SMOOTH);
                    l.meta.masks.push(m);
                }
                "Liquid" => {
                    let wave = |top: f64, phase: f64| {
                        let mut p = kurbo::BezPath::new();
                        p.move_to((0.0, H));
                        p.line_to((0.0, top));
                        for i in 0..4 {
                            let x0 = i as f64 * W / 4.0;
                            p.quad_to(
                                (x0 + W / 8.0, top + if (i % 2 == 0) ^ (phase > 0.5) { -60.0 } else { 60.0 }),
                                (x0 + W / 4.0, top),
                            );
                        }
                        p.line_to((W, H));
                        p.close_path();
                        p
                    };
                    let mut m = Mask::new("Liquid", wave(H + 80.0, 0.0));
                    m.path = keys(
                        &[
                            (reveal, wave(H + 80.0, 0.0)),
                            (reveal + 0.5, wave(H * 0.5, 1.0)),
                            (reveal + 1.0, wave(-80.0, 0.0)),
                        ],
                        SMOOTH,
                    );
                    l.meta.masks.push(m);
                }
                _ => {
                    l.meta
                        .modifiers
                        .push(Modifier { enabled: true, kind: xenify_ops::effects::shatter(full, reveal, false) });
                }
            }
        });
        let mut label = text("Card label", name, 34.0, INK, v(x, 730.0));
        window(&mut label, t + 0.6, t1);
        d.add(label);
    }
    caption(d, "Nineteen transitions, from a clean cut to shatter and smart morph", t0, t1);

    // B. The presenter view.
    let (t0, t1) = (3.6, 6.3);
    let mut screen = rect("Presenter view", v(1500.0, 640.0), 24.0, PANEL, v(W / 2.0, 610.0));
    window(&mut screen, t0, t1);
    d.add(screen);
    let now_l = d.place(now, "Current slide", v(640.0, 560.0), 0.42, t0 + 0.2);
    d.edit(now_l, |l| window(l, t0, t1));
    let next_l = d.place(next, "Next slide", v(1450.0, 440.0), 0.2, t0);
    d.edit(next_l, |l| window(l, t0, t1));
    for (k, (t, size, color, at)) in [
        ("Next", 28.0, MUTED, v(1260.0, 310.0)),
        ("Notes: mention the free trial, then ask for questions.", 28.0, INK, v(1260.0, 610.0)),
        ("00:42", 64.0, GOLD, v(1260.0, 760.0)),
        ("Slide 3 of 12", 28.0, MUTED, v(1260.0, 830.0)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut l = text_left("Presenter text", t, size, color, at);
        if let xenify_doc::LayerKind::Text(tx) = &mut l.kind
            && k == 1
        {
            tx.frame =
                Some(xenify_doc::TextFrame { rect: Rect::new(0.0, -30.0, 420.0, 160.0), wrap: true, clip: false });
        }
        window(&mut l, t0 + 0.3 + k as f64 * 0.12, t1);
        d.add(l);
    }
    caption(d, "Present with a presenter view: notes, the next slide and a timer", t0, t1);

    // C. The templates gallery.
    let (t0, t1) = (6.5, 9.3);
    for (k, comp) in gallery.iter().enumerate() {
        let x = 330.0 + k as f64 * 315.0;
        let id = d.place(*comp, "Template", v(x, 560.0), 0.15, t0 + 0.2 + k as f64 * 0.15);
        d.edit(id, |l| {
            pop(l, t0 + k as f64 * 0.15);
            l.opacity = keys(
                &[(t0 + k as f64 * 0.15, 0.0), (t0 + k as f64 * 0.15 + 0.2, 1.0), (t1, 1.0), (t1 + 0.3, 0.0)],
                Easing::Linear,
            );
            live(l, t0, Some(t1 + 0.35));
        });
        let mut label = text("Theme name", THEMES[k].name, 30.0, INK, v(x, 680.0));
        window(&mut label, t0 + 0.4 + k as f64 * 0.15, t1);
        d.add(label);
    }
    caption(d, "Themes and layouts: a finished deck in one click", t0, t1);
}
