//! Baked physics: simulate from a start time and write the motion as ordinary
//! keyframes (so it plays back, exports and can be edited like any animation).
//!
//! - [`simulate`]: gravity, bouncing off the frame and other layers, friction,
//!   and squish on impact — the moving object, what it hits, or both squash at
//!   the point of contact and spring back.
//! - [`orbit`]: layers circle a centre layer (which may itself move).

use crate::{Command, OpError};
use xenify_anim::{TextMeasure, evaluate_project, world_box};
use xenify_doc::{
    AnyProperty, Composition, Easing, Geometry, Id, Keyframe, LayerKind, Project, PropPath, Property, Vec2,
};
use kurbo::{Affine, Point, Rect};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsSettings {
    /// Downward acceleration in px/s².
    pub gravity: f64,
    /// 0 = dead stop, 1 = perfectly elastic.
    pub bounce: f64,
    /// 0..1: how much sliding speed is lost on each contact.
    pub friction: f64,
    /// Collide with the frame's bottom / sides / top.
    pub floor: bool,
    pub walls: bool,
    pub ceiling: bool,
    /// Other visible layers act as (possibly moving) obstacles.
    pub obstacles: bool,
    /// 0..1: how strongly impacts squash things.
    pub squish: f64,
    /// Who squishes: the simulated (moving) layers and/or what they hit.
    pub squish_moving: bool,
    pub squish_hit: bool,
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            gravity: 2400.0,
            bounce: 0.6,
            friction: 0.15,
            floor: true,
            walls: true,
            ceiling: false,
            obstacles: true,
            squish: 0.5,
            squish_moving: true,
            squish_hit: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Circle(f64),
    /// Half extents.
    Box(Vec2),
}

impl Shape {
    fn half(&self) -> Vec2 {
        match *self {
            Shape::Circle(r) => Vec2::new(r, r),
            Shape::Box(h) => h,
        }
    }
}

/// An impact that squashes a layer: when, which way (unit, pointing from the
/// layer's centre to the contact) and how hard.
#[derive(Clone, Copy, Debug)]
struct Impact {
    time: f64,
    dir: Vec2,
    amount: f64,
}

/// How long a squash takes to spring back.
const SQUISH_TIME: f64 = 0.28;
/// Impacts slower than this (px/s) don't bounce or squish (resting contact).
const REST_SPEED: f64 = 40.0;

/// Squash envelope 0..1 over the time since an impact: quick squash, then a
/// damped spring back through a small stretch.
fn envelope(dt: f64) -> f64 {
    if !(0.0..SQUISH_TIME).contains(&dt) {
        return 0.0;
    }
    let u = dt / SQUISH_TIME;
    let attack = 0.12;
    if u < attack {
        let x = u / attack;
        x * x * (3.0 - 2.0 * x)
    } else {
        let v = (u - attack) / (1.0 - attack);
        (-4.0 * v).exp() * (v * std::f64::consts::PI * 2.2).cos() * (1.0 - v)
    }
}

/// The squash of a layer at `time`: (unit direction to the contact, amount;
/// negative = stretching back).
fn squash_at(impacts: &[Impact], time: f64) -> Option<(Vec2, f64)> {
    impacts
        .iter()
        .map(|i| (i.dir, i.amount * envelope(time - i.time)))
        .filter(|(_, a)| a.abs() > 1e-4)
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
}

struct Body {
    id: Id,
    shape: Shape,
    /// Centre in composition space.
    pos: Point,
    vel: Vec2,
    mass: f64,
}

/// Contact between two shapes: (normal from `a` to `b`, penetration depth).
fn contact(a: Point, sa: Shape, b: Point, sb: Shape) -> Option<(Vec2, f64)> {
    match (sa, sb) {
        (Shape::Circle(ra), Shape::Circle(rb)) => {
            let d = b - a;
            let dist = d.hypot();
            let pen = ra + rb - dist;
            (pen > 0.0).then(|| (if dist > 1e-9 { d / dist } else { Vec2::new(0.0, 1.0) }, pen))
        }
        (Shape::Circle(r), Shape::Box(h)) => circle_box(a, r, b, h),
        (Shape::Box(h), Shape::Circle(r)) => circle_box(b, r, a, h).map(|(n, p)| (-n, p)),
        (Shape::Box(ha), Shape::Box(hb)) => {
            let d = b - a;
            let (ox, oy) = (ha.x + hb.x - d.x.abs(), ha.y + hb.y - d.y.abs());
            if ox <= 0.0 || oy <= 0.0 {
                return None;
            }
            Some(if ox < oy {
                (Vec2::new(d.x.signum(), 0.0), ox)
            } else {
                (Vec2::new(0.0, if d.y == 0.0 { 1.0 } else { d.y.signum() }), oy)
            })
        }
    }
}

/// Circle at `c` against a box centred at `b`: normal from the circle to the box.
fn circle_box(c: Point, r: f64, b: Point, h: Vec2) -> Option<(Vec2, f64)> {
    let d = c - b;
    let closest = Vec2::new(d.x.clamp(-h.x, h.x), d.y.clamp(-h.y, h.y));
    let inside = d.x.abs() < h.x && d.y.abs() < h.y;
    if inside {
        // Centre inside the box: push out along the shallowest side.
        let (px, py) = (h.x - d.x.abs(), h.y - d.y.abs());
        return Some(if px < py {
            (Vec2::new(-d.x.signum(), 0.0), px + r)
        } else {
            (Vec2::new(0.0, -d.y.signum()), py + r)
        });
    }
    let off = d - closest;
    let dist = off.hypot();
    (dist < r).then(|| (if dist > 1e-9 { -off / dist } else { Vec2::new(0.0, 1.0) }, r - dist))
}

/// The collision shape of a layer from its on-screen box: round things (single
/// ellipses that are about as wide as tall) are circles, everything else boxes.
fn shape_of(project: &Project, id: Id, b: Rect) -> Shape {
    let h = Vec2::new(b.width() / 2.0, b.height() / 2.0);
    let round = project.layer(id).ok().is_some_and(|l| match &l.kind {
        LayerKind::Shape { shapes } => shapes.len() == 1 && matches!(shapes[0].geometry, Geometry::Ellipse { .. }),
        _ => false,
    });
    if round && (h.x / h.y.max(1e-9) - 1.0).abs() < 0.25 { Shape::Circle(h.x.min(h.y)) } else { Shape::Box(h) }
}

/// Simulates `layers` from `start` to the end of the composition and returns
/// the command that writes their motion (and squish) as keyframes. Existing
/// animation before `start` is kept; a layer already moving at `start` keeps
/// its speed (so you can throw things).
pub fn simulate(
    project: &Project,
    comp: Id,
    layers: &[Id],
    start: f64,
    s: &PhysicsSettings,
    measure: &mut dyn TextMeasure,
) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (fps, duration) = (c.settings.fps, c.settings.duration);
    let (w, h) = (c.settings.width as f64, c.settings.height as f64);
    if start >= duration {
        return Err(OpError::Invalid("move the playhead before the end of the composition".into()));
    }
    let scene0 = evaluate_project(project, comp, start);
    let dt_frame = 1.0 / fps;
    let scene1 = evaluate_project(project, comp, (start + dt_frame).min(duration));
    let mut bodies: Vec<Body> = Vec::new();
    for &id in layers {
        let (Some(b0), Some(b1)) = (world_box(c, &scene0, id, measure), world_box(c, &scene1, id, measure)) else {
            continue;
        };
        let shape = shape_of(project, id, b0);
        let hh = shape.half();
        bodies.push(Body {
            id,
            shape,
            pos: b0.center(),
            vel: (b1.center() - b0.center()) / dt_frame,
            mass: (4.0 * hh.x * hh.y).max(1.0),
        });
    }
    if bodies.is_empty() {
        return Err(OpError::Invalid("select the layers to simulate".into()));
    }
    // Obstacles: other visible drawable layers (not in or around the selection).
    let mut excluded: Vec<Id> = layers.to_vec();
    for &id in layers {
        excluded.extend(xenify_anim::descendants(c, id));
        let mut p = c.layer(id).and_then(|l| l.meta.parent);
        while let Some(pid) = p {
            excluded.push(pid);
            p = c.layer(pid).and_then(|l| l.meta.parent);
        }
    }
    let obstacle_ids: Vec<Id> = if s.obstacles {
        c.layers
            .iter()
            .filter(|l| !excluded.contains(&l.id) && matches!(l.kind, LayerKind::Shape { .. } | LayerKind::Text(_)))
            .filter(|l| {
                l.meta.matte.is_none() && !c.layers.iter().any(|o| o.meta.matte.is_some_and(|m| m.layer == l.id))
            })
            .map(|l| l.id)
            .collect()
    } else {
        vec![]
    };

    let frames = ((duration - start) * fps).floor() as usize;
    let substeps = 8;
    let dt = dt_frame / substeps as f64;
    let g = Vec2::new(0.0, s.gravity);
    let mut track: Vec<Vec<(f64, Point)>> = bodies.iter().map(|b| vec![(start, b.pos)]).collect();
    let mut impacts: HashMap<Id, Vec<Impact>> = HashMap::new();
    let mut last_hit: HashMap<(usize, u64), f64> = HashMap::new();
    let record = |impacts: &mut HashMap<Id, Vec<Impact>>, id: Id, time: f64, dir: Vec2, speed: f64| {
        let amount = (s.squish * speed / 2500.0).clamp(0.0, 0.6);
        if amount > 0.01 {
            impacts.entry(id).or_default().push(Impact { time, dir, amount });
        }
    };
    let mut boxes_cache: Vec<(Id, Point, Shape)> = Vec::new();
    for f in 1..=frames {
        let t0 = start + (f - 1) as f64 * dt_frame;
        // Obstacles where they are this frame (they may be animated).
        if !obstacle_ids.is_empty() {
            let scene = evaluate_project(project, comp, t0);
            boxes_cache = obstacle_ids
                .iter()
                .filter(|id| scene.layer(**id).is_some_and(|l| l.opacity > 0.02))
                .filter_map(|id| {
                    world_box(c, &scene, *id, measure).map(|b| (*id, b.center(), shape_of(project, *id, b)))
                })
                .collect();
        }
        for k in 0..substeps {
            let now = t0 + (k + 1) as f64 * dt;
            for b in &mut bodies {
                b.vel += g * dt;
                b.pos += b.vel * dt;
            }
            // The frame.
            for (i, b) in bodies.iter_mut().enumerate() {
                let hh = b.shape.half();
                let mut walls: Vec<(Vec2, f64)> = Vec::new(); // (normal into the wall, penetration)
                if s.floor {
                    walls.push((Vec2::new(0.0, 1.0), b.pos.y + hh.y - h));
                }
                if s.ceiling {
                    walls.push((Vec2::new(0.0, -1.0), hh.y - b.pos.y));
                }
                if s.walls {
                    walls.push((Vec2::new(-1.0, 0.0), hh.x - b.pos.x));
                    walls.push((Vec2::new(1.0, 0.0), b.pos.x + hh.x - w));
                }
                for (wi, (n, pen)) in walls.into_iter().enumerate() {
                    if pen <= 0.0 {
                        continue;
                    }
                    b.pos -= n * pen;
                    let vn = b.vel.dot(n);
                    if vn > 0.0 {
                        let tangent = b.vel - n * vn;
                        let bounce = if vn > REST_SPEED { s.bounce } else { 0.0 };
                        b.vel = tangent * (1.0 - s.friction) - n * vn * bounce;
                        let key = (i, 1_000_000 + wi as u64);
                        if vn > REST_SPEED && last_hit.get(&key).is_none_or(|t| now - t > 0.1) && s.squish_moving {
                            record(&mut impacts, b.id, now, n, vn);
                        }
                        last_hit.insert(key, now);
                    }
                }
            }
            // Obstacles.
            for (i, b) in bodies.iter_mut().enumerate() {
                for (oi, &(oid, opos, oshape)) in boxes_cache.iter().enumerate() {
                    let Some((n, pen)) = contact(b.pos, b.shape, opos, oshape) else { continue };
                    b.pos -= n * pen;
                    let vn = b.vel.dot(n);
                    if vn > 0.0 {
                        let tangent = b.vel - n * vn;
                        let bounce = if vn > REST_SPEED { s.bounce } else { 0.0 };
                        b.vel = tangent * (1.0 - s.friction) - n * vn * bounce;
                        let key = (i, oi as u64);
                        if vn > REST_SPEED && last_hit.get(&key).is_none_or(|t| now - t > 0.1) {
                            if s.squish_moving {
                                record(&mut impacts, b.id, now, n, vn);
                            }
                            if s.squish_hit {
                                record(&mut impacts, oid, now, -n, vn);
                            }
                        }
                        last_hit.insert(key, now);
                    }
                }
            }
            // Each other.
            for i in 0..bodies.len() {
                for j in i + 1..bodies.len() {
                    let (a, b) = (&bodies[i], &bodies[j]);
                    let Some((n, pen)) = contact(a.pos, a.shape, b.pos, b.shape) else { continue };
                    let (ma, mb) = (a.mass, b.mass);
                    let total = ma + mb;
                    let rel = (b.vel - a.vel).dot(n);
                    let (pa, pb) = (bodies[i].pos - n * pen * mb / total, bodies[j].pos + n * pen * ma / total);
                    bodies[i].pos = pa;
                    bodies[j].pos = pb;
                    if rel < 0.0 {
                        let e = if -rel > REST_SPEED { s.bounce } else { 0.0 };
                        let imp = -(1.0 + e) * rel / (1.0 / ma + 1.0 / mb);
                        bodies[i].vel -= n * (imp / ma);
                        bodies[j].vel += n * (imp / mb);
                        let key = (i, 2_000_000 + j as u64);
                        if -rel > REST_SPEED && last_hit.get(&key).is_none_or(|t| now - t > 0.1) {
                            // Both are "moving" objects, and each is what the other hit.
                            if s.squish_moving || s.squish_hit {
                                let (ia, ib) = (bodies[i].id, bodies[j].id);
                                record(&mut impacts, ia, now, n, -rel);
                                record(&mut impacts, ib, now, -n, -rel);
                            }
                        }
                        last_hit.insert(key, now);
                    }
                }
            }
        }
        let t = start + f as f64 * dt_frame;
        for (i, b) in bodies.iter().enumerate() {
            track[i].push((t, b.pos));
        }
    }

    // Write keyframes: positions of simulated layers (plus squish), and squish
    // of obstacles that got hit.
    let mut cmds = Vec::new();
    let t_frames: Vec<f64> = (0..=frames).map(|f| start + f as f64 * dt_frame).collect();
    for (i, body) in bodies.iter().enumerate() {
        let hits = impacts.remove(&body.id).unwrap_or_default();
        let hh = body.shape.half();
        let positions: Vec<(f64, Point)> = track[i].clone();
        cmds.extend(write_motion(project, comp, body.id, start, &positions, &hits, hh, true)?);
    }
    for (id, hits) in impacts {
        let Some(&(_, pos, shape)) = boxes_cache.iter().find(|b| b.0 == id) else { continue };
        let positions: Vec<(f64, Point)> = t_frames.iter().map(|t| (*t, pos)).collect();
        // An obstacle that moves keeps its own motion; it only squashes.
        let still = project.layer(id).is_ok_and(|l| !l.transform.position.is_animated());
        cmds.extend(write_motion(project, comp, id, start, &positions, &hits, shape.half(), still)?);
    }
    Ok(Command::batch(cmds))
}

/// Keyframes a layer following `positions` (composition-space centres), with
/// squash-and-stretch from `hits` that keeps the contact side in place.
#[allow(clippy::too_many_arguments)]
fn write_motion(
    project: &Project,
    comp: Id,
    id: Id,
    start: f64,
    positions: &[(f64, Point)],
    hits: &[Impact],
    half: Vec2,
    write_position: bool,
) -> Result<Vec<Command>, OpError> {
    let c = project.comp(comp)?;
    let l = project.layer(id)?;
    let (Some(&(_, first)), true) = (positions.first(), !positions.is_empty()) else { return Ok(vec![]) };
    // World displacements → the layer's parent space (linear part only).
    let parent = l.meta.parent.map_or(Affine::IDENTITY, |p| xenify_anim::world_transform(c, p, start));
    let inv = parent.inverse().as_coeffs();
    let to_parent = |v: Vec2| Vec2::new(inv[0] * v.x + inv[2] * v.y, inv[1] * v.x + inv[3] * v.y);
    let base_pos = l.transform.position.value_at(start);
    let base_scale = l.transform.scale.value_at(start);
    let rot = (l.transform.rotation.value_at(start)).to_radians();
    let moving = positions.iter().any(|(_, p)| (*p - first).hypot() > 0.01);
    let mut pos_keys = Vec::new();
    let mut scale_keys = Vec::new();
    for &(t, p) in positions {
        let mut world = p;
        let mut scale = base_scale;
        if let Some((dir, amount)) = squash_at(hits, t) {
            // Squash along the contact direction, in the layer's own axes.
            let local = Vec2::new(dir.x * rot.cos() + dir.y * rot.sin(), -dir.x * rot.sin() + dir.y * rot.cos());
            let along_x = local.x.abs() > local.y.abs();
            let (sa, sp) = (1.0 - amount, 1.0 + amount * 0.5);
            scale = if along_x {
                Vec2::new(base_scale.x * sa, base_scale.y * sp)
            } else {
                Vec2::new(base_scale.x * sp, base_scale.y * sa)
            };
            // Keep the contact side where it was.
            let extent = if along_x { half.x } else { half.y };
            world += dir * (extent * amount);
        }
        pos_keys.push(Keyframe { time: t, value: base_pos + to_parent(world - first), easing: Easing::Linear });
        scale_keys.push(Keyframe { time: t, value: scale, easing: Easing::Linear });
    }
    let mut cmds = Vec::new();
    if write_position && (moving || !hits.is_empty()) {
        let keys = merge(&l.transform.position, reduce(pos_keys), start);
        cmds.push(Command::SetProperty { layer: id, path: PropPath::Position, property: AnyProperty::Vec2(keys) });
    }
    if !hits.is_empty() {
        let keys = merge(&l.transform.scale, reduce(scale_keys), start);
        cmds.push(Command::SetProperty { layer: id, path: PropPath::Scale, property: AnyProperty::Vec2(keys) });
    }
    Ok(cmds)
}

/// Drops keys that lie on the straight line between their neighbours.
fn reduce(keys: Vec<Keyframe<Vec2>>) -> Vec<Keyframe<Vec2>> {
    if keys.len() < 3 {
        return keys;
    }
    let mut out = vec![keys[0].clone()];
    for i in 1..keys.len() - 1 {
        let (a, b, c) = (out.last().unwrap(), &keys[i], &keys[i + 1]);
        let u = (b.time - a.time) / (c.time - a.time).max(1e-12);
        let predicted = a.value + (c.value - a.value) * u;
        if (predicted - b.value).hypot() > 0.05 {
            out.push(b.clone());
        }
    }
    out.push(keys.last().unwrap().clone());
    out
}

/// Keeps `old`'s keyframes before `start` and continues with `new`.
fn merge(old: &Property<Vec2>, new: Vec<Keyframe<Vec2>>, start: f64) -> Property<Vec2> {
    let mut keys: Vec<Keyframe<Vec2>> =
        old.keyframes().iter().filter(|k| k.time < start - xenify_doc::TIME_EPSILON).cloned().collect();
    keys.extend(new);
    Property::from_keyframes(keys).unwrap_or_else(|| old.clone())
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrbitSettings {
    /// Turns per second (for the innermost orbit with `kepler`).
    pub speed: f64,
    pub clockwise: bool,
    /// 1 = circle; smaller = a flattened ellipse (a tilted, 3D-looking orbit).
    pub flatten: f64,
    /// Outer orbits turn slower, like planets.
    pub kepler: bool,
}

impl Default for OrbitSettings {
    fn default() -> Self {
        Self { speed: 0.25, clockwise: true, flatten: 1.0, kepler: false }
    }
}

/// Makes `orbiters` circle `center` (which may be animated) from `start` to the
/// end of the composition, starting from where they are.
pub fn orbit(
    project: &Project,
    comp: Id,
    center: Id,
    orbiters: &[Id],
    start: f64,
    s: &OrbitSettings,
) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (fps, duration) = (c.settings.fps, c.settings.duration);
    if orbiters.is_empty() {
        return Err(OpError::Invalid("select the centre first, then the layers that orbit it".into()));
    }
    let anchor = |c: &Composition, id: Id, t: f64| -> Option<Point> {
        let l = c.layer(id)?;
        Some(xenify_anim::world_transform(c, id, t) * l.transform.anchor.value_at(t).to_point())
    };
    let c0 = anchor(c, center, start).ok_or_else(|| OpError::Invalid("no centre layer".into()))?;
    let flat = s.flatten.clamp(0.05, 1.0);
    let radii: Vec<f64> = orbiters
        .iter()
        .filter_map(|id| anchor(c, *id, start))
        .map(|p| {
            let d = p - c0;
            Vec2::new(d.x, d.y / flat).hypot()
        })
        .collect();
    let inner = radii.iter().copied().fold(f64::INFINITY, f64::min).max(1.0);
    let frames = ((duration - start) * fps).floor() as usize;
    let dir = if s.clockwise { 1.0 } else { -1.0 };
    let mut cmds = Vec::new();
    for &id in orbiters {
        let Some(p0) = anchor(c, id, start) else { continue };
        let d0 = p0 - c0;
        let r = Vec2::new(d0.x, d0.y / flat).hypot();
        if r < 1e-6 {
            continue;
        }
        let theta0 = (d0.y / flat).atan2(d0.x);
        let speed = if s.kepler { s.speed * (inner / r).powf(1.5) } else { s.speed };
        let track: Vec<(f64, Point)> = (0..=frames)
            .filter_map(|f| {
                let t = start + f as f64 / fps;
                let centre = anchor(c, center, t)?;
                let a = theta0 + dir * std::f64::consts::TAU * speed * (t - start);
                Some((t, centre + Vec2::new(r * a.cos(), r * flat * a.sin())))
            })
            .collect();
        cmds.extend(write_motion(project, comp, id, start, &track, &[], Vec2::ZERO, true)?);
    }
    Ok(Command::batch(cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_anim::ApproxText;
    use xenify_doc::{Color, Layer, ShapeItem};

    fn scene(layers: Vec<Layer>) -> (Project, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let c = p.comp_mut(comp).unwrap();
        c.settings.duration = 3.0;
        c.layers = layers;
        p.reserve_id(Id(100));
        (p, comp)
    }

    fn ball(id: u64, x: f64, y: f64) -> Layer {
        Layer::shape(Id(id), "ball", vec![ShapeItem::ellipse(Vec2::new(100.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(x, y))
    }

    #[test]
    fn a_ball_falls_bounces_and_settles_on_the_floor() {
        let (mut p, comp) = scene(vec![ball(1, 960.0, 200.0)]);
        let s = PhysicsSettings { obstacles: false, ..Default::default() };
        let cmd = simulate(&p, comp, &[Id(1)], 0.0, &s, &mut ApproxText).unwrap();
        apply(&mut p, cmd).unwrap();
        let pos = |t| p.layer(Id(1)).unwrap().transform.position.value_at(t);
        // Falls…
        assert!(pos(0.3).y > 300.0);
        // …never sinks through the floor (1080 - radius 50)…
        for i in 0..=90 {
            assert!(pos(i as f64 / 30.0).y <= 1030.0 + 1.0, "{:?}", pos(i as f64 / 30.0));
        }
        // …bounces back up at some point, and ends resting on the floor.
        let ys: Vec<f64> = (0..=90).map(|i| pos(i as f64 / 30.0).y).collect();
        let lowest = ys.iter().position(|y| *y > 1020.0).unwrap();
        assert!(ys[lowest..].iter().any(|y| *y < 900.0), "it bounces");
        assert!((pos(3.0).y - 1030.0).abs() < 5.0, "rests on the floor: {}", pos(3.0).y);
        // It squashed on impact (scale keys exist and dip below 100%).
        let scale = &p.layer(Id(1)).unwrap().transform.scale;
        assert!(scale.is_animated());
        assert!((0..=90).any(|i| scale.value_at(i as f64 / 30.0).y < 0.9));
    }

    #[test]
    fn falling_onto_an_obstacle_can_squish_it() {
        let shelf = Layer::shape(Id(2), "shelf", vec![ShapeItem::rect(Vec2::new(400.0, 60.0), Color::WHITE)])
            .with_position(Vec2::new(960.0, 700.0));
        let (mut p, comp) = scene(vec![ball(1, 960.0, 200.0), shelf]);
        let s = PhysicsSettings { squish_moving: false, squish_hit: true, ..Default::default() };
        let cmd = simulate(&p, comp, &[Id(1)], 0.0, &s, &mut ApproxText).unwrap();
        apply(&mut p, cmd).unwrap();
        // The ball lands on the shelf (top at 670), not the floor.
        let y = p.layer(Id(1)).unwrap().transform.position.value_at(3.0).y;
        assert!((y - 620.0).abs() < 6.0, "{y}");
        // The shelf got squashed (vertically) where it was hit; the ball didn't.
        let shelf_scale = &p.layer(Id(2)).unwrap().transform.scale;
        assert!((0..=90).any(|i| shelf_scale.value_at(i as f64 / 30.0).y < 0.97));
        assert!(!p.layer(Id(1)).unwrap().transform.scale.is_animated());
    }

    #[test]
    fn two_balls_collide_and_both_squish() {
        let mut a = ball(1, 400.0, 540.0);
        let mut b = ball(2, 1500.0, 540.0);
        // Thrown at each other.
        a.transform.position = Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: Vec2::new(400.0, 540.0), easing: Easing::Linear },
            Keyframe { time: 1.0, value: Vec2::new(1400.0, 540.0), easing: Easing::Linear },
        ])
        .unwrap();
        b.transform.position = Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: Vec2::new(1500.0, 540.0), easing: Easing::Linear },
            Keyframe { time: 1.0, value: Vec2::new(500.0, 540.0), easing: Easing::Linear },
        ])
        .unwrap();
        let (mut p, comp) = scene(vec![a, b]);
        let s = PhysicsSettings { gravity: 0.0, floor: false, ..Default::default() };
        let cmd = simulate(&p, comp, &[Id(1), Id(2)], 0.0, &s, &mut ApproxText).unwrap();
        apply(&mut p, cmd).unwrap();
        let x = |id, t| p.layer(Id(id)).unwrap().transform.position.value_at(t).x;
        // They never pass through each other.
        for i in 0..=90 {
            let t = i as f64 / 30.0;
            assert!(x(2, t) - x(1, t) >= 95.0, "t={t}: {} {}", x(1, t), x(2, t));
        }
        for id in [1, 2] {
            let sc = &p.layer(Id(id)).unwrap().transform.scale;
            assert!((0..=90).any(|i| sc.value_at(i as f64 / 30.0).x < 0.9), "layer {id} squishes sideways");
        }
    }

    #[test]
    fn orbits_circle_the_centre_at_the_set_speed() {
        let (mut p, comp) = scene(vec![ball(1, 960.0, 540.0), ball(2, 1260.0, 540.0)]);
        let s = OrbitSettings { speed: 0.5, ..Default::default() };
        let cmd = orbit(&p, comp, Id(1), &[Id(2)], 0.0, &s).unwrap();
        apply(&mut p, cmd).unwrap();
        let pos = |t| p.layer(Id(2)).unwrap().transform.position.value_at(t);
        assert!((pos(1.0) - Vec2::new(660.0, 540.0)).hypot() < 1.0, "half a turn in 1 s: {:?}", pos(1.0));
        assert!((pos(0.5) - Vec2::new(960.0, 840.0)).hypot() < 1.0, "clockwise (down first): {:?}", pos(0.5));
        assert!((pos(2.0) - Vec2::new(1260.0, 540.0)).hypot() < 1.0);
    }
}
