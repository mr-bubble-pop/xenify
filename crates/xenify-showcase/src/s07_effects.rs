//! Slide 7: Effects: water fills the screen with the title afloat; a word
//! explodes; a title catches fire and burns into smoke; a big blast shakes
//! the camera; emoji confetti; particles assemble into the logo.

use crate::kit::*;
use xenify_audio::synth::Sfx;
use xenify_doc::{Easing, Id, LayerKind, ParticleShape, TransitionKind};
use xenify_ops::effects::Placement;
use kurbo::Rect;

fn caption(d: &mut Deck, t: &str, t0: f64, t1: f64) {
    let mut c = text("Caption", t, 40.0, MUTED, v(W / 2.0, 1030.0));
    animate(&mut c, "Fade Up by Word", t0 + 0.2);
    c.opacity = keys(&[(t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    live(&mut c, 0.0, Some(t1 + 0.35));
    d.add(c);
}

pub fn build(d: &mut Deck) {
    // A reel: effects never sit still (water, fire, smoke, particles), so it
    // plays straight through and stops once the logo has formed.
    d.slide(
        "Effects",
        18.4,
        &[],
        tr(TransitionKind::Iris, 1.0),
        "Every effect is a real layer you can change. Plays through.",
    );

    // A. Water fills the screen; the title floats.
    let (t0, t1) = (0.0, 3.2);
    let since = d.next_id();
    let id = d.next_id();
    let cmd = xenify_ops::liquid::fill_screen(&d.p, d.comp, id, 0.2).expect("water");
    d.exec(cmd);
    let mut title = text("Effects title", "Effects", 200.0, INK, v(W / 2.0, 470.0));
    title.transform.position = keys(
        &[
            (0.0, v(W / 2.0, 470.0)),
            (1.2, v(W / 2.0, 440.0)),
            (2.0, v(W / 2.0, 462.0)),
            (2.8, v(W / 2.0, 444.0)),
            (3.4, v(W / 2.0, 458.0)),
        ],
        SMOOTH,
    );
    title.transform.rotation = keys(&[(0.0, -3.0), (1.4, 2.5), (2.8, -2.0), (3.4, 1.0)], SMOOTH);
    animate(&mut title, "Drop In", 0.6);
    d.add(title);
    d.sfx(Sfx::Splash, 0.3, 0.0, 0.8);
    caption(d, "Water that fills the screen or any shape, with waves and bubbles", t0, t1);
    d.end_since(since, t1 + 0.35);

    // B. A word explodes.
    let (t0, t1) = (3.4, 6.1);
    let since = d.next_id();
    let blow = t0 + 1.0;
    let mut word = text("Boom word", "BOOM", 240.0, GOLD, v(W / 2.0, 500.0));
    animate(&mut word, "Pop by Letter", 0.0);
    live(&mut word, t0, None);
    let word = d.add(word);
    d.effect(
        "Shatter Out",
        Placement {
            time: blow,
            target: Some(word),
            target_local_box: Some(Rect::new(-330.0, -120.0, 330.0, 120.0)),
            ..Default::default()
        },
    );
    d.effect("Explosion", Placement { at: Some(v(W / 2.0, 500.0)), time: blow, ..Default::default() });
    d.sfx(Sfx::Boom, blow, 0.0, 0.9);
    d.sfx(Sfx::Shatter, blow, 0.0, 0.7);
    caption(d, "Explosions and shatter: pick where it starts and how it ends", t0, t1);
    d.end_since(since, t1 + 0.35);

    // C. A title catches fire and burns away into smoke.
    let (t0, t1) = (6.3, 9.0);
    let since = d.next_id();
    let mut hot = text("Hot title", "ON FIRE", 200.0, INK, v(W / 2.0, 560.0));
    hot.opacity = keys(&[(t0 + 1.2, 1.0), (t0 + 2.2, 0.0)], EASE_IN);
    live(&mut hot, t0, None);
    let hot = d.add(hot);
    let fire = d.effect("Burning Shape", Placement { target: Some(hot), time: t0 + 0.3, ..Default::default() });
    d.edit(fire, |l| {
        if let LayerKind::Particles(e) = &mut l.kind {
            e.stop = Some(t0 + 2.0);
        }
    });
    d.effect("Smoke Puffs", Placement { at: Some(v(W / 2.0, 560.0)), time: t0 + 1.4, ..Default::default() });
    d.sfx(Sfx::Crackle, t0 + 0.3, 0.0, 0.6);
    caption(d, "Fire and smoke: set any layer alight", t0, t1);
    d.end_since(since, t1 + 0.35);

    // D. A big blast shakes the camera.
    let (t0, t1) = (9.2, 12.1);
    let since = d.next_id();
    let (cmd, _) = xenify_ops::blast::add_blast(
        &d.p,
        d.comp,
        d.next_id(),
        xenify_ops::blast::BlastKind::BigBlast,
        v(W / 2.0, 900.0),
        t0 + 0.2,
        1.0,
    )
    .expect("blast");
    d.exec(cmd);
    d.sfx(Sfx::Boom, t0 + 0.2, -5.0, 1.0);
    caption(d, "A big blast: flash, shockwave, mushroom cloud and camera shake", t0, t1);
    d.end_since(since, t1 + 0.35);

    // E. Emoji confetti, and an emoji that reacts.
    let (t0, t1) = (12.3, 15.1);
    let since = d.next_id();
    for (k, glyph) in ["🎉", "🚀", "✨"].into_iter().enumerate() {
        let sprite = d.emoji(glyph, v(-1000.0, -1000.0), 120.0);
        d.edit(sprite, |l| l.meta.visible = false);
        d.particles(
            "Confetti Burst",
            Some(v(560.0 + k as f64 * 400.0, 820.0)),
            t0 + 0.2 + k as f64 * 0.25,
            None,
            |e| {
                e.particle = ParticleShape::Layer { layer: sprite };
                e.size = 56.0.into();
                e.spin = 200.0.into();
                e.count = 40.0.into();
            },
        );
        d.sfx(Sfx::Pop, t0 + 0.2 + k as f64 * 0.25, 2.0 * k as f64, 0.7);
    }
    let face = d.emoji("😀", v(W / 2.0, 520.0), 220.0);
    d.edit(face, |l| {
        l.transform.scale = keys_e(&[
            (t0, v(0.0, 0.0), BACK),
            (t0 + 0.5, v(1.0, 1.0), Easing::Hold),
            (t0 + 0.9, v(1.0, 1.0), SPRING),
            (t0 + 1.5, v(1.25, 1.25), SPRING),
        ]);
        l.transform.rotation = keys(&[(t0 + 0.9, 0.0), (t0 + 1.2, -12.0), (t0 + 1.5, 10.0), (t0 + 1.8, 0.0)], SMOOTH);
    });
    let mut credit =
        text("Emoji credit", "Emoji: Fluent Emoji © Microsoft, MIT licence", 24.0, MUTED, v(W - 320.0, 60.0));
    fade_in(&mut credit, t0, 0.4);
    d.add(credit);
    caption(d, "Emoji: 770 of them, as editable shapes, or as confetti", t0, t1);
    d.end_since(since, t1 + 0.35);

    // F. Particles assemble into the logo.
    let (t0, t1) = (15.3, 18.4);
    let since = d.next_id();
    let mut mark = logo(360.0, v(W / 2.0, 500.0));
    mark.opacity = keys(&[(t0 + 2.2, 0.0), (t0 + 2.5, 1.0)], SMOOTH);
    live(&mut mark, t0, None);
    let mark = d.add(mark);
    d.particles("Assemble into Shape", Some(v(W / 2.0, 1200.0)), t0 + 0.2, Some(mark), |e| {
        e.count = 700.0.into();
        e.lifetime = 2.2.into();
        e.lifetime_variation = 0.0;
        e.direction = (-90.0).into();
        e.spread = 120.0.into();
        e.colors = vec![PINK, VIOLET, CYAN];
        e.size = 7.0.into();
    });
    d.sfx(Sfx::Riser, t0 + 0.3, 0.0, 0.5);
    caption(d, "Particles that assemble into any shape", t0, t1);
    let _ = (since, Id(0));
}
