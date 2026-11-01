//! Slide 1: Cold open: a spark streaks in, bursts, and its specks form the logo;
//! the name tightens in; the four verbs follow. Riser, boom, then music.

use crate::kit::*;
use xenify_audio::synth::{Mood, MusicRecipe, Sfx};
use xenify_doc::{Easing, EndLook, ParticleShape};

pub fn build(d: &mut Deck) {
    d.slide("Cold open", 6.0, &[4.8], tr(xenify_doc::TransitionKind::Cut, 0.0), "Let the spark land, then click.");
    let (from, to) = (v(-60.0, 760.0), v(W / 2.0, 430.0));
    let streak_ease = xenify_doc::Easing::Bezier { x1: 0.45, y1: 0.0, x2: 0.9, y2: 0.55 };
    let angle = (to.y - from.y).atan2(to.x - from.x).to_degrees();
    // The streak and its bright head.
    let mut streak = rect("Streak", v(260.0, 6.0), 3.0, PINK, from);
    if let xenify_doc::LayerKind::Shape { shapes } = &mut streak.kind {
        shapes[0].fill = Some(gradient_fill(260.0));
        if let xenify_doc::Geometry::Rect { position, .. } = &mut shapes[0].geometry {
            *position = v(-130.0, 0.0).into();
        }
    }
    streak.transform.rotation = angle.into();
    streak.transform.position = keys(&[(0.15, from), (1.1, to)], streak_ease);
    live(&mut streak, 0.0, Some(1.12));
    d.add(streak);
    let mut head = ellipse("Spark head", v(22.0, 22.0), INK, from);
    head.transform.position = keys(&[(0.15, from), (1.1, to)], streak_ease);
    live(&mut head, 0.0, Some(1.12));
    d.add(head);
    // The flash of the burst.
    let mut flash = ellipse("Flash", v(80.0, 80.0), INK, to);
    flash.transform.scale = keys(&[(1.1, v(0.2, 0.2)), (1.5, v(6.0, 6.0))], EXPO);
    flash.opacity = keys(&[(1.1, 1.0), (1.5, 0.0)], Easing::Linear);
    flash.meta.blend = xenify_doc::BlendMode::Add;
    live(&mut flash, 1.1, Some(1.6));
    d.add(flash);
    // The logo the specks form; it shows as they land.
    let mut mark = logo(250.0, to);
    mark.opacity = keys(&[(2.35, 0.0), (2.6, 1.0)], SMOOTH);
    mark.transform.scale = keys(&[(2.35, v(0.92, 0.92)), (2.9, v(1.0, 1.0))], SPRING);
    let mark = d.add(mark);
    d.particles("Assemble into Shape", Some(to), 1.1, Some(mark), |e| {
        e.count = 520.0.into();
        e.lifetime = 1.4.into();
        e.lifetime_variation = 0.0;
        e.speed = 900.0.into();
        e.size = 7.0.into();
        e.particle = ParticleShape::Circle;
        e.colors = vec![PINK, VIOLET, INK];
        e.end = EndLook::Form { target: mark };
    });
    // The name, letters tightening in; the tagline word by word.
    let mut name = text("Product name", &d.name.clone(), 132.0, INK, v(W / 2.0, 700.0));
    animate(&mut name, "Tracking In", 2.5);
    d.add(name);
    let mut tag = text("Tagline", "Design. Animate. Present. Edit.", 44.0, MUTED, v(W / 2.0, 800.0));
    animate(&mut tag, "Fade Up by Word", 3.3);
    d.add(tag);
    d.sfx(Sfx::Riser, 0.0, 0.0, 0.7);
    d.sfx(Sfx::Boom, 1.1, -2.0, 0.9);
    d.music(
        MusicRecipe {
            tempo: 112.0,
            key: 2,
            mood: Mood::Uplifting,
            bars: 8,
            seed: 3,
            volume: 0.6,
            ..Default::default()
        },
        1.1,
    );
}
