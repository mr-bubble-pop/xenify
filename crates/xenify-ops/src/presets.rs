//! Motion presets: generate ordinary (editable) keyframes for common motion.
//!
//! Each preset replaces the animation of the properties it touches and
//! returns one command, so applying it is a single undo step.

use crate::{Command, OpError};
use xenify_doc::{AnyProperty, Easing, Id, Keyframe, Layer, Project, PropPath, Property, ShapeItem, Trim};
use kurbo::{Affine, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "preset", rename_all = "snake_case")]
pub enum Preset {
    FadeIn {
        duration: f64,
    },
    FadeOut {
        duration: f64,
    },
    /// Scale from 0 with an overshoot of `overshoot` (e.g. 0.15 = 115%).
    PopIn {
        duration: f64,
        overshoot: f64,
    },
    /// Arrive from `from`, travelling `distance` pixels.
    SlideIn {
        duration: f64,
        from: Direction,
        distance: f64,
    },
    /// Drop from `height` pixels above and bounce `bounces` times.
    Bounce {
        duration: f64,
        height: f64,
        bounces: u32,
    },
    /// Squash at impacts in the existing position animation (strength 0..1).
    SquashStretch {
        strength: f64,
    },
    /// Random drift of up to `amount` pixels, `frequency` times per second.
    Wiggle {
        duration: f64,
        amount: f64,
        frequency: f64,
    },
    /// Outlines draw themselves (trim paths: end 0 → 100%).
    DrawOn {
        duration: f64,
    },
}

impl Preset {
    /// Presets with sensible defaults, for menus.
    pub const DEFAULTS: [(&'static str, Preset); 8] = [
        ("Fade in", Preset::FadeIn { duration: 0.5 }),
        ("Fade out", Preset::FadeOut { duration: 0.5 }),
        ("Pop in", Preset::PopIn { duration: 0.5, overshoot: 0.15 }),
        ("Slide in", Preset::SlideIn { duration: 0.6, from: Direction::Down, distance: 120.0 }),
        ("Bounce", Preset::Bounce { duration: 1.4, height: 300.0, bounces: 3 }),
        ("Squash & stretch", Preset::SquashStretch { strength: 0.25 }),
        ("Wiggle", Preset::Wiggle { duration: 2.0, amount: 12.0, frequency: 4.0 }),
        ("Draw on", Preset::DrawOn { duration: 1.5 }),
    ];
}

fn key<T>(time: f64, value: T, easing: Easing) -> Keyframe<T> {
    Keyframe { time: time.max(0.0), value, easing }
}

fn scalar(keys: Vec<Keyframe<f64>>) -> AnyProperty {
    AnyProperty::Scalar(Property::from_keyframes(keys).expect("keys"))
}

fn vec2(keys: Vec<Keyframe<Vec2>>) -> AnyProperty {
    AnyProperty::Vec2(Property::from_keyframes(keys).expect("keys"))
}

fn set(layer: Id, path: PropPath, property: AnyProperty) -> Command {
    Command::SetProperty { layer, path, property }
}

/// Deterministic pseudo-random numbers in -1..1 (same layer ⇒ same wiggle).
fn noise(seed: u64, i: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ i.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Builds the command applying `preset` to `layer`, starting at `start` seconds.
pub fn apply(project: &Project, layer: Id, preset: Preset, start: f64) -> Result<Command, OpError> {
    let l = project.layer(layer)?;
    let pos = l.transform.position.value_at(start);
    let scale = l.transform.scale.value_at(start);
    // The opacity to fade to/from: the layer's highest opacity (1 if it's never visible).
    let opacity = match &l.opacity {
        Property::Static(v) => *v,
        Property::Animated(keys) => keys.iter().map(|k| k.value).fold(0.0, f64::max),
    };
    let opacity = if opacity <= 0.0 { 1.0 } else { opacity.min(1.0) };
    let ease_out = Easing::EASE_OUT;
    let fade = |dur: f64| {
        set(
            layer,
            PropPath::Opacity,
            scalar(vec![key(start, 0.0, ease_out), key(start + dur, opacity, Easing::Linear)]),
        )
    };
    Ok(match preset {
        Preset::FadeIn { duration } => fade(duration.max(0.01)),
        Preset::FadeOut { duration } => set(
            layer,
            PropPath::Opacity,
            scalar(vec![key(start, opacity, Easing::EASE_IN), key(start + duration.max(0.01), 0.0, Easing::Linear)]),
        ),
        Preset::PopIn { duration, overshoot } => {
            let d = duration.max(0.05);
            Command::batch(vec![
                set(
                    layer,
                    PropPath::Scale,
                    vec2(vec![
                        key(start, Vec2::ZERO, ease_out),
                        key(start + d * 0.65, scale * (1.0 + overshoot), Easing::EASE_IN_OUT),
                        key(start + d, scale, Easing::Linear),
                    ]),
                ),
                fade(d * 0.4),
            ])
        }
        Preset::SlideIn { duration, from, distance } => {
            let off = match from {
                Direction::Left => Vec2::new(-distance, 0.0),
                Direction::Right => Vec2::new(distance, 0.0),
                Direction::Up => Vec2::new(0.0, -distance),
                Direction::Down => Vec2::new(0.0, distance),
            };
            let d = duration.max(0.05);
            Command::batch(vec![
                set(
                    layer,
                    PropPath::Position,
                    vec2(vec![
                        key(start, pos + off, Easing::Bezier { x1: 0.16, y1: 1.0, x2: 0.3, y2: 1.0 }),
                        key(start + d, pos, Easing::Linear),
                    ]),
                ),
                fade(d * 0.6),
            ])
        }
        Preset::Bounce { duration, height, bounces } => {
            // Gravity: fall time ∝ √height; each bounce keeps `r` of the height.
            let r: f64 = 0.4;
            let n = bounces.min(8) as i32;
            // Durations: first fall 1 unit, then each bounce up+down = 2·√(r^k).
            let units: f64 = 1.0 + (1..=n).map(|k| 2.0 * r.powi(k).sqrt()).sum::<f64>();
            let unit = duration.max(0.1) / units;
            let mut keys =
                vec![key(start, pos - Vec2::new(0.0, height), Easing::Bezier { x1: 0.55, y1: 0.0, x2: 1.0, y2: 0.45 })];
            let mut t = start + unit;
            keys.push(key(t, pos, Easing::Bezier { x1: 0.0, y1: 0.55, x2: 0.45, y2: 1.0 }));
            for k in 1..=n {
                let half = unit * r.powi(k).sqrt();
                keys.push(key(
                    t + half,
                    pos - Vec2::new(0.0, height * r.powi(k)),
                    Easing::Bezier { x1: 0.55, y1: 0.0, x2: 1.0, y2: 0.45 },
                ));
                t += 2.0 * half;
                let easing =
                    if k == n { Easing::Linear } else { Easing::Bezier { x1: 0.0, y1: 0.55, x2: 0.45, y2: 1.0 } };
                keys.push(key(t, pos, easing));
            }
            set(layer, PropPath::Position, vec2(keys))
        }
        Preset::SquashStretch { strength } => squash_stretch(project, l, strength.clamp(0.0, 0.8))?,
        Preset::DrawOn { duration } => {
            if l.shapes().is_empty() {
                return Err(OpError::Invalid("Draw on needs a shape layer".into()));
            }
            let end = Property::from_keyframes(vec![
                key(start, 0.0, Easing::EASE_IN_OUT),
                key(start + duration.max(0.05), 1.0, Easing::Linear),
            ])
            .expect("keys");
            Command::batch(
                l.shapes()
                    .iter()
                    .enumerate()
                    .map(|(index, s)| {
                        let trim = Trim { end: end.clone(), ..s.trim.clone().unwrap_or_default() };
                        Command::SetShape { layer, index, shape: ShapeItem { trim: Some(trim), ..s.clone() } }
                    })
                    .collect(),
            )
        }
        Preset::Wiggle { duration, amount, frequency } => {
            let step = 1.0 / frequency.clamp(0.2, 30.0);
            let n = (duration.max(step) / step).round() as u64;
            let mut keys = vec![key(start, pos, Easing::EASE_IN_OUT)];
            for i in 1..n {
                let jitter = Vec2::new(noise(layer.0, 2 * i), noise(layer.0, 2 * i + 1)) * amount;
                keys.push(key(start + i as f64 * step, pos + jitter, Easing::EASE_IN_OUT));
            }
            keys.push(key(start + n as f64 * step, pos, Easing::Linear));
            set(layer, PropPath::Position, vec2(keys))
        }
    })
}

/// Squash at each impact of the position animation (where motion along the
/// main axis reverses), with the pivot moved to the contact edge.
fn squash_stretch(project: &Project, l: &Layer, s: f64) -> Result<Command, OpError> {
    let keys = l.transform.position.keyframes();
    if keys.len() < 3 {
        return Err(OpError::Invalid(
            "squash & stretch needs a position animation with direction changes (e.g. apply Bounce first)".into(),
        ));
    }
    // Main axis: the one with more travel.
    let span = |f: fn(Vec2) -> f64| {
        let v: Vec<f64> = keys.iter().map(|k| f(k.value)).collect();
        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
    };
    let vertical = span(|v| v.y) >= span(|v| v.x);
    let axis = |v: Vec2| if vertical { v.y } else { v.x };
    // Impacts: interior keyframes that are a local extreme along the axis, on
    // the side the motion keeps returning to (the "ground").
    let ground_is_max = {
        let last = axis(keys.last().unwrap().value);
        let first = axis(keys[0].value);
        last >= first
    };
    let mut impacts: Vec<(f64, f64)> = Vec::new(); // (time, min gap to neighbours)
    for i in 1..keys.len() {
        let here = axis(keys[i].value);
        let before = axis(keys[i - 1].value);
        let after = keys.get(i + 1).map(|k| axis(k.value));
        let is_extreme = match after {
            Some(a) if ground_is_max => here > before && here > a,
            Some(a) => here < before && here < a,
            None => false,
        };
        if is_extreme {
            let gap =
                (keys[i].time - keys[i - 1].time).min(keys.get(i + 1).map_or(f64::MAX, |k| k.time - keys[i].time));
            impacts.push((keys[i].time, gap));
        }
    }
    if impacts.is_empty() {
        return Err(OpError::Invalid("no impacts found in the position animation".into()));
    }

    let base = l.transform.scale.value_at(keys[0].time);
    // (squash across, squash along) as factors on the main axis.
    let shape = |along: f64, across: f64| {
        if vertical { Vec2::new(base.x * across, base.y * along) } else { Vec2::new(base.x * along, base.y * across) }
    };
    let mut scale_keys = vec![key(keys[0].time, base, Easing::EASE_IN_OUT)];
    for (t, gap) in &impacts {
        let d = (gap / 3.0).clamp(1.0 / 60.0, 0.08);
        scale_keys.push(key(t - d, shape(1.0 + s * 0.5, 1.0 - s * 0.3), Easing::EASE_OUT));
        scale_keys.push(key(*t, shape(1.0 - s, 1.0 + s * 0.7), Easing::EASE_OUT));
        scale_keys.push(key(t + d, shape(1.0 + s * 0.3, 1.0 - s * 0.2), Easing::EASE_IN_OUT));
        scale_keys.push(key(t + 2.0 * d, base, Easing::EASE_IN_OUT));
    }

    // Move the pivot to the contact edge (bottom for a floor, etc.) and shift
    // the position the same amount so nothing jumps.
    let mut cmds = vec![Command::SetProperty { layer: l.id, path: PropPath::Scale, property: vec2(scale_keys) }];
    let comp = project.locate_layer(l.id).and_then(|(c, _)| project.comp(c))?;
    let scene = xenify_anim::evaluate_project(project, comp.id, keys[0].time);
    let local = scene.layer(l.id).and_then(xenify_anim::SceneLayer::local_bounds);
    if let Some(b) = local {
        let anchor_old = l.transform.anchor.value_at(0.0);
        let contact = match (vertical, ground_is_max) {
            (true, true) => Vec2::new(b.center().x, b.y1),
            (true, false) => Vec2::new(b.center().x, b.y0),
            (false, true) => Vec2::new(b.x1, b.center().y),
            (false, false) => Vec2::new(b.x0, b.center().y),
        };
        let delta = contact - anchor_old;
        if delta.hypot() > 1e-9 && !l.transform.anchor.is_animated() {
            let shifted: Vec<Keyframe<Vec2>> = keys
                .iter()
                .map(|k| {
                    let lin = Affine::rotate(l.transform.rotation.value_at(k.time).to_radians())
                        * Affine::scale_non_uniform(base.x, base.y);
                    let d = lin * delta.to_point();
                    Keyframe { time: k.time, value: k.value + d.to_vec2(), easing: k.easing }
                })
                .collect();
            cmds.push(Command::SetProperty {
                layer: l.id,
                path: PropPath::Anchor,
                property: AnyProperty::Vec2(contact.into()),
            });
            cmds.push(Command::SetProperty { layer: l.id, path: PropPath::Position, property: vec2(shifted) });
        }
    }
    Ok(Command::batch(cmds))
}

/// Applies `preset` to several layers, each starting `stagger` seconds after
/// the previous one.
pub fn apply_many(
    project: &Project,
    layers: &[Id],
    preset: Preset,
    start: f64,
    stagger: f64,
) -> Result<Command, OpError> {
    let cmds = layers
        .iter()
        .enumerate()
        .map(|(i, id)| apply(project, *id, preset, start + stagger * i as f64))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Command::batch(cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply as run;
    use xenify_doc::Color;

    fn go(p: &mut Project, id: Id, preset: Preset, start: f64) {
        let c = apply(p, id, preset, start).unwrap();
        run(p, c).unwrap();
    }

    fn project() -> (Project, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let id = p.alloc_id();
        let l = Layer::shape(id, "ball", vec![ShapeItem::ellipse(Vec2::new(100.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(500.0, 800.0));
        p.comp_mut(comp).unwrap().layers.push(l);
        (p, id)
    }

    #[test]
    fn draw_on_trims_every_shape() {
        let (mut p, id) = project();
        go(&mut p, id, Preset::DrawOn { duration: 1.0 }, 0.5);
        let t = p.layer(id).unwrap().shapes()[0].trim.clone().unwrap();
        assert_eq!(t.end.value_at(0.5), 0.0);
        assert_eq!(t.end.value_at(1.5), 1.0);
        assert_eq!(t.start.value_at(1.0), 0.0);
    }

    #[test]
    fn pop_in_overshoots_and_settles() {
        let (mut p, id) = project();
        go(&mut p, id, Preset::PopIn { duration: 0.5, overshoot: 0.2 }, 1.0);
        let s = &p.layer(id).unwrap().transform.scale;
        assert_eq!(s.value_at(1.0), Vec2::ZERO);
        assert!(s.value_at(1.0 + 0.5 * 0.65).x > 1.19);
        assert_eq!(s.value_at(2.0), Vec2::new(1.0, 1.0));
        assert_eq!(p.layer(id).unwrap().opacity.value_at(5.0), 1.0);
    }

    #[test]
    fn slide_in_ends_where_it_was() {
        let (mut p, id) = project();
        let preset = Preset::SlideIn { duration: 0.6, from: Direction::Left, distance: 200.0 };
        go(&mut p, id, preset, 0.0);
        let pos = &p.layer(id).unwrap().transform.position;
        assert_eq!(pos.value_at(0.0), Vec2::new(300.0, 800.0));
        assert_eq!(pos.value_at(1.0), Vec2::new(500.0, 800.0));
    }

    #[test]
    fn bounce_lands_repeatedly_with_decreasing_height_then_squashes_on_the_ground() {
        let (mut p, id) = project();
        go(&mut p, id, Preset::Bounce { duration: 1.5, height: 300.0, bounces: 3 }, 0.0);
        let keys = p.layer(id).unwrap().transform.position.keyframes().to_vec();
        assert_eq!(keys.len(), 2 + 2 * 3);
        assert_eq!(keys[0].value.y, 500.0);
        assert!((keys.last().unwrap().time - 1.5).abs() < 1e-9);
        let apexes: Vec<f64> = keys.iter().skip(2).step_by(2).map(|k| 800.0 - k.value.y).collect();
        assert!(apexes.windows(2).all(|w| w[1] < w[0]), "{apexes:?}");

        // Squash & stretch: squashed vertically at each impact, pivot on the floor.
        let before = xenify_anim::evaluate(&p.compositions[0], 0.9);
        go(&mut p, id, Preset::SquashStretch { strength: 0.3 }, 0.0);
        let l = p.layer(id).unwrap();
        let impact = keys[1].time;
        let sq = l.transform.scale.value_at(impact);
        assert!(sq.y < 0.75 && sq.x > 1.15, "{sq:?}");
        assert!((l.transform.anchor.value_at(0.0) - Vec2::new(0.0, 50.0)).hypot() < 1e-9);
        // Moving the pivot didn't move the ball (compare where scale is 1 again).
        let after = xenify_anim::evaluate(&p.compositions[0], 0.9);
        let centre = |s: &xenify_anim::Scene| s.layers[0].transform * s.layers[0].local_bounds().unwrap().center();
        let (a, b) = (centre(&before), centre(&after));
        let still = l.transform.scale.value_at(0.9) == Vec2::new(1.0, 1.0);
        if still {
            assert!((a - b).hypot() < 1e-6, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn squash_needs_motion_and_wiggle_is_repeatable() {
        let (p, id) = project();
        assert!(apply(&p, id, Preset::SquashStretch { strength: 0.3 }, 0.0).is_err());
        let w = Preset::Wiggle { duration: 1.0, amount: 10.0, frequency: 5.0 };
        assert_eq!(apply(&p, id, w, 0.0).unwrap(), apply(&p, id, w, 0.0).unwrap());
    }

    #[test]
    fn stagger_offsets_start_times() {
        let (mut p, id) = project();
        let comp = p.compositions[0].id;
        let id2 = p.alloc_id();
        let l2 = Layer::shape(id2, "b", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        p.comp_mut(comp).unwrap().layers.push(l2);
        {
            let c = apply_many(&p, &[id, id2], Preset::FadeIn { duration: 0.5 }, 1.0, 0.25).unwrap();
            run(&mut p, c).unwrap();
        }
        assert_eq!(p.layer(id).unwrap().opacity.keyframes()[0].time, 1.0);
        assert_eq!(p.layer(id2).unwrap().opacity.keyframes()[0].time, 1.25);
    }
}
