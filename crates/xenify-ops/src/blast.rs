//! Explosions built from ordinary, editable layers: a group of a flash, a
//! shockwave ring, particle layers (fireball, sparks, smoke, stem, rolling
//! cap, dust, debris) and a jolt of camera shake. Every part can be changed
//! or removed afterwards like any other layer.

use crate::{Command, OpError, invalid};
use xenify_doc::generators::{fire_colours, smoke_colours};
use xenify_doc::{
    BlendMode, CameraShake, Color, EmitMode, Emitter, EmitterShape, EndLook, GradientStop, Id, Keyframe, Layer,
    LayerKind, ParticleShape, Project, Property, ShapeItem, Stroke, Vec2,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlastKind {
    /// A burst of fire, sparks and smoke with a shockwave ring.
    Explosion,
    /// A nuclear-style blast: flash, shockwave, fireball, rising stem,
    /// rolling mushroom cap, ground dust and debris, and camera shake.
    BigBlast,
}

impl BlastKind {
    pub fn label(self) -> &'static str {
        match self {
            BlastKind::Explosion => "Explosion",
            BlastKind::BigBlast => "Big Blast",
        }
    }
}

fn keys<T: xenify_doc::Animatable>(k: &[(f64, T)]) -> Property<T> {
    Property::from_keyframes(
        k.iter()
            .map(|(time, value)| Keyframe { time: *time, value: value.clone(), easing: xenify_doc::Easing::EASE_OUT })
            .collect(),
    )
    .unwrap_or_else(|| k[0].1.clone().into())
}

fn particles(id: Id, name: &str, e: Emitter, at: Vec2, group: Id, glow: bool) -> Layer {
    let mut l = Layer::new(id, name, LayerKind::Particles(Box::new(Emitter { seed: id.0, ..e }))).with_position(at);
    l.meta.parent = Some(group);
    if glow {
        l.meta.blend = BlendMode::Add;
    }
    l
}

/// The commands that add a blast at `at` (composition space; for Big
/// Blast, the ground under it) starting at `time`, `scale` times the
/// normal size. The layers take ids from `first` up; returns the group's id.
pub fn add_blast(
    project: &Project,
    comp: Id,
    first: Id,
    kind: BlastKind,
    at: Vec2,
    time: f64,
    scale: f64,
) -> Result<(Command, Id), OpError> {
    let c = project.comp(comp)?;
    if scale.is_nan() || scale <= 0.0 {
        return Err(invalid("the size must be above zero"));
    }
    let (w, h) = (c.settings.width as f64, c.settings.height as f64);
    let k = scale;
    let t0 = time;
    let mut next = first.0;
    let mut id = || {
        let i = Id(next);
        next += 1;
        i
    };
    let group_id = id();
    let mut layers = vec![Layer::new(group_id, kind.label(), LayerKind::Group).with_position(at)];
    let rel = |p: Vec2| p - at;
    let d = Emitter::default();
    let fire = |count: f64, speed: f64, size: f64, life: f64| Emitter {
        shape: EmitterShape::Ellipse { size: Vec2::new(60.0, 60.0) * k },
        count: count.into(),
        start: t0,
        lifetime: life.into(),
        lifetime_variation: 0.3,
        spread: 360.0.into(),
        speed: (speed * k).into(),
        speed_variation: 0.6,
        spin: 30.0.into(),
        gravity: Vec2::new(0.0, -150.0 * k).into(),
        drag: 3.0.into(),
        turbulence: (20.0 * k).into(),
        turbulence_speed: 1.5,
        size: (size * k).into(),
        size_variation: 0.4,
        size_end: 1.6,
        particle: ParticleShape::Soft,
        color_over_life: Some(fire_colours()),
        ..d.clone()
    };

    // The flash: the whole frame lights up and fades.
    let (flash_peak, flash_len) = match kind {
        BlastKind::BigBlast => (1.0, 0.8),
        BlastKind::Explosion => (0.5, 0.35),
    };
    let mut flash = Layer::shape(id(), "Flash", vec![ShapeItem::rect(Vec2::new(w, h), Color::rgb(1.0, 0.95, 0.85))])
        .with_position(rel(Vec2::new(w / 2.0, h / 2.0)));
    flash.opacity = keys(&[(t0, 0.0), (t0 + 0.04, flash_peak), (t0 + flash_len, 0.0)]);
    flash.meta.blend = BlendMode::Add;
    flash.meta.parent = Some(group_id);
    layers.push(flash);

    // The shockwave: a ring racing outwards (flat on the ground for a Big Blast).
    let ring_size = match kind {
        BlastKind::BigBlast => Vec2::new(200.0, 50.0) * k,
        BlastKind::Explosion => Vec2::new(120.0, 120.0) * k,
    };
    let mut ring_item = ShapeItem::ellipse(ring_size, Color::WHITE);
    ring_item.fill = None;
    ring_item.stroke =
        Some(Stroke { color: Color::rgba(1.0, 0.95, 0.85, 0.9).into(), width: (6.0 * k).into(), ..Default::default() });
    let mut ring = Layer::shape(id(), "Shockwave", vec![ring_item]).with_position(Vec2::ZERO);
    let reach = if kind == BlastKind::BigBlast { 14.0 } else { 6.0 };
    ring.transform.scale = keys(&[(t0, Vec2::new(0.1, 0.1)), (t0 + 1.4, Vec2::new(reach, reach))]);
    ring.opacity = keys(&[(t0, 1.0), (t0 + 1.4, 0.0)]);
    ring.meta.blend = BlendMode::Add;
    ring.meta.parent = Some(group_id);
    layers.push(ring);

    match kind {
        BlastKind::Explosion => {
            layers.push(particles(id(), "Fireball", fire(180.0, 380.0, 70.0, 1.2), Vec2::ZERO, group_id, true));
            let sparks = Emitter {
                count: 90.0.into(),
                start: t0,
                lifetime: 1.0.into(),
                spread: 360.0.into(),
                speed: (900.0 * k).into(),
                speed_variation: 0.5,
                gravity: Vec2::new(0.0, 500.0 * k).into(),
                drag: 1.5.into(),
                turbulence: 0.0.into(),
                size: (6.0 * k).into(),
                particle: ParticleShape::Streak,
                colors: vec![Color::rgb(1.0, 0.9, 0.5), Color::rgb(1.0, 0.6, 0.2)],
                ..d.clone()
            };
            layers.push(particles(id(), "Sparks", sparks, Vec2::ZERO, group_id, true));
            let smoke = Emitter {
                count: 40.0.into(),
                start: t0 + 0.1,
                lifetime: 3.0.into(),
                spread: 360.0.into(),
                speed: (160.0 * k).into(),
                drag: 2.0.into(),
                gravity: Vec2::new(0.0, -40.0 * k).into(),
                turbulence: (25.0 * k).into(),
                size: (80.0 * k).into(),
                size_end: 3.5,
                particle: ParticleShape::Soft,
                color_over_life: Some(smoke_colours()),
                ..d.clone()
            };
            layers.push(particles(id(), "Smoke", smoke, Vec2::ZERO, group_id, false));
        }
        BlastKind::BigBlast => {
            let ground = at.y;
            layers.push(particles(
                id(),
                "Fireball",
                fire(260.0, 350.0, 95.0, 1.6),
                Vec2::new(0.0, -60.0 * k),
                group_id,
                true,
            ));
            // The stem: fire turning to smoke, rising from the ground.
            let stem = Emitter {
                shape: EmitterShape::Ellipse { size: Vec2::new(60.0, 20.0) * k },
                mode: EmitMode::Stream,
                rate: 120.0.into(),
                start: t0 + 0.3,
                stop: Some(t0 + 3.0),
                lifetime: 2.2.into(),
                direction: (-90.0).into(),
                spread: 12.0.into(),
                speed: (380.0 * k).into(),
                drag: 0.5.into(),
                gravity: Vec2::new(0.0, -80.0 * k).into(),
                turbulence: (25.0 * k).into(),
                size: (70.0 * k).into(),
                size_end: 2.5,
                particle: ParticleShape::Soft,
                color_over_life: Some(vec![
                    GradientStop { offset: 0.0, color: Color::rgba(1.0, 0.7, 0.3, 0.9) },
                    GradientStop { offset: 0.3, color: Color::rgba(0.6, 0.25, 0.1, 0.7) },
                    GradientStop { offset: 1.0, color: Color::rgba(0.25, 0.22, 0.2, 0.0) },
                ]),
                ..d.clone()
            };
            layers.push(particles(id(), "Stem", stem, Vec2::ZERO, group_id, false));
            // The cap: smoke rolling round a ring that rises and grows.
            let cap_e = Emitter {
                shape: EmitterShape::Ellipse { size: Vec2::new(240.0, 110.0) * k },
                mode: EmitMode::Stream,
                rate: 150.0.into(),
                start: t0 + 0.2,
                lifetime: 3.5.into(),
                speed: 0.0.into(),
                gravity: Vec2::ZERO.into(),
                drag: 0.0.into(),
                turbulence: (25.0 * k).into(),
                vortex: 90.0.into(),
                size: (90.0 * k).into(),
                size_end: 1.8,
                particle: ParticleShape::Soft,
                color_over_life: Some(vec![
                    GradientStop { offset: 0.0, color: Color::rgba(1.0, 0.8, 0.4, 0.0) },
                    GradientStop { offset: 0.1, color: Color::rgba(1.0, 0.6, 0.2, 0.9) },
                    GradientStop { offset: 0.4, color: Color::rgba(0.55, 0.3, 0.2, 0.7) },
                    GradientStop { offset: 1.0, color: Color::rgba(0.3, 0.28, 0.27, 0.0) },
                ]),
                ..d.clone()
            };
            let mut cap = particles(id(), "Mushroom Cap", cap_e, Vec2::ZERO, group_id, false);
            let top = -(ground - h * 0.3).max(200.0 * k);
            cap.transform.position = keys(&[(t0, Vec2::new(0.0, -60.0 * k)), (t0 + 3.5, Vec2::new(0.0, top))]);
            cap.transform.scale = keys(&[(t0, Vec2::new(0.4, 0.4)), (t0 + 3.5, Vec2::new(1.4, 1.4))]);
            layers.push(cap);
            // Dust thrown out along the ground.
            let dust = Emitter {
                shape: EmitterShape::Ellipse { size: Vec2::new(200.0, 20.0) * k },
                count: 140.0.into(),
                start: t0 + 0.1,
                lifetime: 3.0.into(),
                spread: 360.0.into(),
                speed: (500.0 * k).into(),
                drag: 2.5.into(),
                gravity: Vec2::new(0.0, 300.0 * k).into(),
                turbulence: (20.0 * k).into(),
                size: (80.0 * k).into(),
                size_end: 3.0,
                particle: ParticleShape::Soft,
                colors: vec![Color::rgba(0.55, 0.45, 0.35, 0.5)],
                end: EndLook::Fade,
                ..d.clone()
            };
            layers.push(particles(id(), "Dust", dust, Vec2::ZERO, group_id, false));
            // Debris: chunks thrown up that land on the ground.
            let debris = Emitter {
                count: 60.0.into(),
                start: t0,
                lifetime: 3.0.into(),
                direction: (-90.0).into(),
                spread: 120.0.into(),
                speed: (900.0 * k).into(),
                speed_variation: 0.6,
                gravity: Vec2::new(0.0, 900.0 * k).into(),
                drag: 0.4.into(),
                turbulence: 0.0.into(),
                spin: 400.0.into(),
                size: (10.0 * k).into(),
                size_variation: 0.6,
                particle: ParticleShape::Square,
                colors: vec![Color::rgb(0.2, 0.17, 0.15), Color::rgb(0.35, 0.3, 0.25)],
                end: EndLook::Settle { floor: ground },
                ..d.clone()
            };
            layers.push(particles(id(), "Debris", debris, Vec2::ZERO, group_id, false));
        }
    }

    let mut commands: Vec<Command> =
        layers.into_iter().map(|layer| Command::AddLayer { comp, index: None, layer }).collect();
    // Shake the camera (the active one; else a new one).
    let jolt = CameraShake {
        amount: (if kind == BlastKind::BigBlast { 40.0 } else { 16.0 } * k).into(),
        ..CameraShake::impact(t0)
    };
    match xenify_doc::active_camera(&c.layers, t0) {
        Some(cam) => {
            if let LayerKind::Camera(settings) = &cam.kind {
                let mut s = settings.clone();
                s.shake = Some(jolt);
                commands.push(Command::SetCamera { layer: cam.id, camera: Box::new(s) });
            }
        }
        None => {
            let mut cam = xenify_doc::camera_layer(id(), w, h);
            if let LayerKind::Camera(s) = &mut cam.kind {
                s.shake = Some(jolt);
            }
            commands.push(Command::AddLayer { comp, index: None, layer: cam });
        }
    }
    Ok((Command::batch(commands), group_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;

    #[test]
    fn blasts_add_an_editable_group_that_shakes_the_camera() {
        for kind in [BlastKind::Explosion, BlastKind::BigBlast] {
            let mut p = Project::new();
            let comp = p.compositions[0].id;
            let first = Id(p.next_id);
            let (cmd, group) = add_blast(&p, comp, first, kind, Vec2::new(960.0, 900.0), 1.0, 1.0).unwrap();
            let undo = apply(&mut p, cmd).unwrap();
            let c = p.comp(comp).unwrap();
            let members = c.layers.iter().filter(|l| l.meta.parent == Some(group)).count();
            assert!(members >= 5, "{kind:?}: {members} parts");
            let cam = xenify_doc::active_camera(&c.layers, 1.0).expect("a camera");
            assert!(matches!(&cam.kind, LayerKind::Camera(s) if s.shake.is_some()));
            // Something is drawn during the blast, and the camera moves.
            let scene = xenify_anim::evaluate_project(&p, comp, 1.5);
            assert!(!scene.layers.is_empty());
            assert_ne!(scene.camera, kurbo::Affine::IDENTITY);
            apply(&mut p, undo).unwrap();
            assert!(p.comp(comp).unwrap().layers.is_empty(), "one undo removes it all");
        }
    }

    #[test]
    fn the_cap_rolls_round_its_ring() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let (cmd, _) =
            add_blast(&p, comp, Id(p.next_id), BlastKind::BigBlast, Vec2::new(960.0, 900.0), 0.0, 1.0).unwrap();
        apply(&mut p, cmd).unwrap();
        let c = p.comp(comp).unwrap();
        let cap = c.layers.iter().find(|l| l.meta.name == "Mushroom Cap").unwrap();
        let LayerKind::Particles(e) = &cap.kind else { unreachable!() };
        let ps = xenify_anim::particles::particles(c, cap, e, 2.0);
        assert!(!ps.is_empty());
        // All around the cap's centre, not scattered across the frame.
        let centre = xenify_anim::world_transform(c, cap.id, 2.0) * kurbo::Point::ZERO;
        for q in &ps {
            assert!(q.position.distance(centre) < 600.0, "{:?} far from {centre:?}", q.position);
        }
    }
}
