//! Slide 11: Finale: confetti cannons and fireworks, the logo and name, and the
//! call to action as the music resolves.

use crate::kit::*;
use xenify_audio::synth::Sfx;
use xenify_doc::{Easing, TransitionKind};
use xenify_ops::effects::Placement;

pub fn build(d: &mut Deck) {
    d.slide("Finale", 5.5, &[5.0], tr(TransitionKind::DipToColor, 1.0), "Thank you.");
    d.effect("Fireworks", Placement { at: Some(v(W / 2.0, 300.0)), time: 0.4, ..Default::default() });
    for (k, (x, dir)) in [(80.0, -60.0), (W - 80.0, -120.0)].into_iter().enumerate() {
        d.particles("Confetti Burst", Some(v(x, H + 20.0)), 0.3 + k as f64 * 0.1, None, |e| {
            e.direction = dir.into();
            e.spread = 30.0.into();
            e.speed = 1500.0.into();
            e.count = 160.0.into();
            e.colors = vec![PINK, VIOLET, CYAN, GOLD, INK];
        });
        d.sfx(Sfx::Pop, 0.3 + k as f64 * 0.1, -3.0, 0.8);
    }
    let mut mark = logo(220.0, v(W / 2.0, 360.0));
    mark.transform.scale = keys_e(&[(0.6, v(0.0, 0.0), SPRING), (1.4, v(1.0, 1.0), Easing::Linear)]);
    mark.transform.rotation = keys(&[(0.6, -90.0), (1.4, 0.0)], EXPO);
    d.add(mark);
    let mut name = text("Product name", &d.name.clone(), 150.0, INK, v(W / 2.0, 610.0));
    animate(&mut name, "Tracking In", 1.0);
    d.add(name);
    let mut cta = text("Call to action", "Create. Animate. Present.", 64.0, INK, v(W / 2.0, 760.0));
    animate(&mut cta, "Fade Up by Word", 1.8);
    d.add(cta);
    let mut line = rect("Underline", v(700.0, 6.0), 3.0, PINK, v(W / 2.0, 810.0));
    if let xenify_doc::LayerKind::Shape { shapes } = &mut line.kind {
        shapes[0].fill = Some(gradient_fill(700.0));
    }
    line.transform.scale = keys(&[(2.6, v(0.0, 1.0)), (3.4, v(1.0, 1.0))], EXPO);
    d.add(line);
    // The music resolves: a rising chord of chimes.
    for (k, pitch) in [0.0, 4.0, 7.0, 12.0].into_iter().enumerate() {
        d.sfx(Sfx::Chime, 1.8 + k as f64 * 0.18, pitch, 0.5);
    }
}
