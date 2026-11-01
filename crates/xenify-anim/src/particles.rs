//! Particle emitters: where every particle is at a moment, worked out in
//! closed form from its birth (no stepping), so any frame can be drawn on
//! its own and always looks the same.
//!
//! Motion with gravity `g` and linear drag `k` from `p0`, `v0`:
//! `p(t) = p0 + (v0 - g/k)(1 - e^(-kt))/k + (g/k) t` (and the plain
//! ballistic formula when `k` is 0). Turbulence adds smooth noise on top.

use crate::DrawItem;
use kurbo::{Affine, BezPath, Point, Shape, Vec2};
use std::collections::HashMap;
use xenify_doc::{
    Color, Composition, EmitMode, Emitter, EmitterShape, EndLook, FillRule, Gradient, GradientKind, GradientStop, Id,
    Layer, LayerKind, ParticleShape, Wiggle,
};

/// Drawn at most at once (the oldest are left out beyond this).
pub const MAX_PARTICLES: usize = 5000;
/// Births counted at most (a stream running for a long time).
const MAX_BIRTHS: usize = 50_000;
/// Share of the life at the end over which Fade and Shrink happen.
const FINISH: f64 = 0.3;
/// Form: the share of the life spent flying into place.
const FORM_FROM: f64 = 0.35;

/// One particle at a moment, in composition space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub index: usize,
    pub position: Point,
    pub velocity: Vec2,
    /// Degrees.
    pub rotation: f64,
    pub size: f64,
    pub color: Color,
    pub opacity: f64,
    /// Confetti's paper flip: 1 = face on, 0 = edge on.
    pub flip: f64,
    /// 0 at birth, 1 at the end of its life (and beyond, for settled or formed ones).
    pub age: f64,
}

/// A deterministic value in 0..1 for particle `i`, channel `c`.
fn rand(seed: u64, i: usize, c: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// `-1..1` from `0..1`.
fn signed(r: f64) -> f64 {
    r * 2.0 - 1.0
}

/// Birth times of every particle born up to `time`, in order.
pub fn births(e: &Emitter, time: f64) -> Vec<f64> {
    let mut out = Vec::new();
    match e.mode {
        EmitMode::Burst => {
            for b in 0..=e.repeat {
                let t = e.start + b as f64 * e.interval.max(0.0);
                if t > time {
                    break;
                }
                let n = e.count.value_at(t).round().clamp(0.0, MAX_PARTICLES as f64) as usize;
                out.extend(std::iter::repeat_n(t, n));
            }
        }
        EmitMode::Stream => {
            let end = e.stop.map_or(time, |s| s.min(time));
            let dt = 1.0 / 240.0;
            // Prewarmed: as if it had been running for a lifetime already.
            let warm =
                if e.prewarm { e.lifetime.value_at(e.start) * (1.0 + e.lifetime_variation.max(0.0)) } else { 0.0 };
            let mut t = e.start - warm;
            let mut due = 0.0;
            while t <= end && out.len() < MAX_BIRTHS {
                due += e.rate.value_at(t).max(0.0) * dt;
                while due >= 1.0 && out.len() < MAX_BIRTHS {
                    out.push(t);
                    due -= 1.0;
                }
                t += dt;
            }
        }
    }
    out
}

/// Layer → composition transform of `id` at `time`, through its parents.
pub fn world_at(comp: &Composition, id: Id, time: f64) -> Affine {
    let mut m = Affine::IDENTITY;
    let mut at = comp.layer(id);
    let mut guard = 0;
    while let Some(l) = at {
        m = l.transform.affine_at(time) * m;
        at = l.meta.parent.filter(|p| *p != l.id).and_then(|p| comp.layer(p));
        guard += 1;
        if guard > 64 {
            break;
        }
    }
    m
}

/// A layer's outline in its own space (shapes' paths; a group's members,
/// placed relative to it). Text has none here (no fonts in this crate).
pub fn layer_outline(comp: &Composition, id: Id, time: f64) -> BezPath {
    let Some(layer) = comp.layer(id) else { return BezPath::new() };
    let mut out = BezPath::new();
    match &layer.kind {
        LayerKind::Shape { shapes } => {
            for s in shapes {
                out.extend(crate::draw_item(s, time, 0, None).path);
            }
        }
        LayerKind::Group => {
            let to_group = world_at(comp, id, time).inverse();
            for child in comp.layers.iter().filter(|l| l.meta.parent == Some(id)) {
                let m = to_group * world_at(comp, child.id, time);
                out.extend(m * layer_outline(comp, child.id, time));
            }
        }
        LayerKind::Text(t) => {
            // Its text box (the exact letters need fonts, which this crate hasn't got).
            let mut st =
                crate::SceneText::new(t.text.clone(), t.font_family.clone(), t.font_size.value_at(time), Color::WHITE);
            st.align = t.align;
            st.valign = t.valign;
            st.frame = t.frame;
            let b = crate::TextMeasure::text_bounds(&mut crate::ApproxText, &st);
            out.extend(b.to_path(0.1));
        }
        LayerKind::Solid(_) | LayerKind::Image { .. } | LayerKind::Precomp { .. } => {
            // Their box.
            if let Some(b) = layer_box(layer) {
                out.extend(b.to_path(0.1));
            }
        }
        _ => {}
    }
    out
}

fn layer_box(layer: &Layer) -> Option<kurbo::Rect> {
    match &layer.kind {
        LayerKind::Solid(s) => {
            let r = s.size.value_at(0.0) / 2.0;
            Some(kurbo::Rect::new(-r, -r, r, r))
        }
        _ => None,
    }
}

/// Points along a path by distance.
pub struct Outline {
    points: Vec<Point>,
    dist: Vec<f64>,
}

impl Outline {
    pub fn new(path: &BezPath) -> Option<Self> {
        let mut points: Vec<Point> = Vec::new();
        let mut dist: Vec<f64> = Vec::new();
        let mut start = None;
        kurbo::flatten(path.iter(), 0.25, |el| {
            let p = match el {
                kurbo::PathEl::MoveTo(p) => {
                    start = Some(p);
                    // Jumps between sub-paths don't count as length.
                    points.push(p);
                    dist.push(dist.last().copied().unwrap_or(0.0));
                    return;
                }
                kurbo::PathEl::LineTo(p) => p,
                kurbo::PathEl::ClosePath => match start {
                    Some(s) => s,
                    None => return,
                },
                _ => return,
            };
            let d = dist.last().copied().unwrap_or(0.0) + points.last().map_or(0.0, |q| q.distance(p));
            points.push(p);
            dist.push(d);
        });
        (points.len() >= 2 && dist.last().is_some_and(|d| *d > 0.0)).then_some(Self { points, dist })
    }

    pub fn length(&self) -> f64 {
        *self.dist.last().unwrap()
    }

    /// The point `u` (0..1) of the way round.
    pub fn at(&self, u: f64) -> Point {
        let s = u.rem_euclid(1.0) * self.length();
        let i = self.dist.partition_point(|d| *d < s).clamp(1, self.points.len() - 1);
        let (d0, d1) = (self.dist[i - 1], self.dist[i]);
        let f = if d1 > d0 { (s - d0) / (d1 - d0) } else { 0.0 };
        self.points[i - 1].lerp(self.points[i], f)
    }
}

/// Where particle `i` is born, in the emitter's (or the source layer's) space.
fn spawn_point(shape: &EmitterShape, source: Option<&Outline>, seed: u64, i: usize) -> Point {
    let (a, b) = (rand(seed, i, 10), rand(seed, i, 11));
    match shape {
        EmitterShape::Point => Point::ZERO,
        EmitterShape::Line { length } => Point::new(signed(a) * length / 2.0, 0.0),
        EmitterShape::Rect { size } => Point::new(signed(a) * size.x / 2.0, signed(b) * size.y / 2.0),
        EmitterShape::Ellipse { size } => {
            let (r, t) = (a.sqrt(), b * std::f64::consts::TAU);
            Point::new(r * t.cos() * size.x / 2.0, r * t.sin() * size.y / 2.0)
        }
        EmitterShape::Layer { .. } => source.map_or(Point::ZERO, |o| o.at(a)),
    }
}

/// Position after `t` seconds (see the module docs), and velocity.
fn ballistic(p0: Point, v0: Vec2, g: Vec2, k: f64, t: f64) -> (Point, Vec2) {
    if k.abs() < 1e-6 {
        return (p0 + v0 * t + g * (t * t / 2.0), v0 + g * t);
    }
    let e = (-k * t).exp();
    let terminal = g / k;
    let p = p0 + (v0 - terminal) * ((1.0 - e) / k) + terminal * t;
    let v = (v0 - terminal) * e + terminal;
    (p, v)
}

/// Every particle alive (or settled / formed) at `time`.
pub fn particles(comp: &Composition, layer: &Layer, e: &Emitter, time: f64) -> Vec<Particle> {
    let born = births(e, time);
    let seed = e.seed ^ layer.id.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
    // The source outline (Layer emitters), in the source layer's space.
    let source_id = match e.shape {
        EmitterShape::Layer { layer } => Some(layer),
        _ => None,
    };
    let source = source_id.and_then(|id| Outline::new(&layer_outline(comp, id, time)));
    let spawner = source_id.unwrap_or(layer.id);
    // Form: the target's outline now, in composition space.
    let target = match e.end {
        EndLook::Form { target } => {
            let path = world_at(comp, target, time) * layer_outline(comp, target, time);
            Outline::new(&path)
        }
        _ => None,
    };
    let mut worlds: HashMap<u64, Affine> = HashMap::new();
    let mut out = Vec::new();
    let settle_or_form = matches!(e.end, EndLook::Settle { .. } | EndLook::Form { .. });
    for (i, &tb) in born.iter().enumerate().rev() {
        if out.len() >= MAX_PARTICLES {
            break;
        }
        let age = time - tb;
        let life = (e.lifetime.value_at(tb) * (1.0 + e.lifetime_variation * signed(rand(seed, i, 1)))).max(0.01);
        if age < 0.0 || (age > life && !settle_or_form) {
            continue;
        }
        let world = *worlds.entry(tb.to_bits()).or_insert_with(|| world_at(comp, spawner, tb));
        let p0 = world * spawn_point(&e.shape, source.as_ref(), seed, i);
        // Direction turns with the emitter layer.
        let own = if source_id.is_some() { world_at(comp, layer.id, tb) } else { world };
        let turn = {
            let v = own * Point::new(1.0, 0.0) - own * Point::ZERO;
            v.y.atan2(v.x).to_degrees()
        };
        let dir = (e.direction.value_at(tb) + turn + e.spread.value_at(tb) * (rand(seed, i, 2) - 0.5)).to_radians();
        let speed = e.speed.value_at(tb) * (1.0 + e.speed_variation * signed(rand(seed, i, 3)));
        let v0 = Vec2::new(dir.cos(), dir.sin()) * speed;
        let g = e.gravity.value_at(tb);
        let k = e.drag.value_at(tb).max(0.0);
        let spin = e.spin.value_at(tb) * signed(rand(seed, i, 4));
        let turb = e.turbulence.value_at(tb);
        let wobble =
            Wiggle { frequency: e.turbulence_speed, position: 1.0, rotation: 0.0, scale: 0.0, seed: seed ^ i as u64 };
        let at = |a: f64| {
            let (p, v) = ballistic(p0, v0, g, k, a);
            let ramp = (a * 2.0).min(1.0);
            (p + Vec2::new(wobble.noise(0, a), wobble.noise(1, a)) * turb * ramp, v)
        };
        let mut u = (age / life).min(1.0);
        let (mut position, mut velocity) = at(age);
        // Rolling round a ring (a mushroom cap), carried by the layer as it is now.
        let vortex = e.vortex.value_at(tb);
        if vortex.abs() > 1e-9
            && let EmitterShape::Ellipse { size } = e.shape
        {
            let (big, small) = (size.x / 2.0, size.y / 2.0);
            let theta = rand(seed, i, 12) * std::f64::consts::TAU;
            let phi = rand(seed, i, 13) * std::f64::consts::TAU + (vortex * age).to_radians();
            let local = Point::new((big + small * phi.cos()) * theta.cos(), -small * phi.sin());
            let now = world_at(comp, layer.id, time);
            let before = world_at(comp, layer.id, time - 1.0 / 60.0);
            let ramp = (age * 2.0).min(1.0);
            let swirl = Vec2::new(wobble.noise(0, age), wobble.noise(1, age)) * turb * ramp;
            position = now * local + swirl;
            velocity = (position - (before * local + swirl)) * 60.0;
        }
        let mut rotation = rand(seed, i, 5) * 360.0 + spin * age;
        let flip_rate = 2.0 + rand(seed, i, 6) * 4.0;
        let mut flip = (age * flip_rate + rand(seed, i, 7) * 6.0).cos();
        let base = e.size.value_at(tb) * (1.0 + e.size_variation * signed(rand(seed, i, 8)));
        let mut size = base * (1.0 + (e.size_end - 1.0) * u);
        let mut opacity = 1.0;
        match &e.end {
            EndLook::Fade => opacity = ((1.0 - u) / FINISH).clamp(0.0, 1.0),
            EndLook::Shrink => size *= ((1.0 - u) / FINISH).clamp(0.0, 1.0),
            EndLook::Settle { floor } => {
                // The first moment it reaches the floor (looked for frame by frame, then refined).
                let step = 1.0 / 60.0;
                let mut hit = None;
                let mut a = 0.0;
                while a <= age {
                    if at(a).0.y >= *floor {
                        let (mut lo, mut hi) = ((a - step).max(0.0), a);
                        for _ in 0..20 {
                            let mid = (lo + hi) / 2.0;
                            if at(mid).0.y >= *floor { hi = mid } else { lo = mid }
                        }
                        hit = Some(hi);
                        break;
                    }
                    a += step;
                }
                if let Some(h) = hit {
                    position = Point::new(at(h).0.x, *floor);
                    velocity = Vec2::ZERO;
                    rotation = rand(seed, i, 5) * 360.0 + spin * h;
                    flip = 1.0; // lying flat
                    size = base * (1.0 + (e.size_end - 1.0) * (h / life).min(1.0));
                    u = u.max(1.0);
                } else if age > life {
                    continue; // never landed
                }
            }
            EndLook::Form { .. } => {
                if let Some(t) = &target {
                    // Spread evenly round the outline (golden-ratio steps).
                    let goal = t.at(i as f64 * 0.618_033_988_749_895);
                    let w = ((u - FORM_FROM) / (1.0 - FORM_FROM)).clamp(0.0, 1.0);
                    let w = w * w * (3.0 - 2.0 * w);
                    position = position.lerp(goal, w);
                    rotation *= 1.0 - w;
                    flip = flip + (1.0 - flip) * w;
                    size = base + (size - base) * (1.0 - w);
                    velocity *= 1.0 - w;
                }
            }
        }
        if opacity <= 0.001 || size <= 0.01 {
            continue;
        }
        let color = match &e.color_over_life {
            Some(stops) if !stops.is_empty() => color_at(stops, u),
            _ if e.colors.is_empty() => Color::WHITE,
            _ => e.colors[(rand(seed, i, 9) * e.colors.len() as f64) as usize % e.colors.len()],
        };
        out.push(Particle { index: i, position, velocity, rotation, size, color, opacity, flip, age: u });
    }
    out.reverse();
    out
}

/// A gradient's colour at `u` (0..1).
pub fn color_at(stops: &[GradientStop], u: f64) -> Color {
    let first = stops[0];
    if u <= first.offset {
        return first.color;
    }
    for w in stops.windows(2) {
        if u <= w[1].offset {
            let f = if w[1].offset > w[0].offset { (u - w[0].offset) / (w[1].offset - w[0].offset) } else { 1.0 };
            let (a, b) = (w[0].color, w[1].color);
            return Color::rgba(
                a.r + (b.r - a.r) * f,
                a.g + (b.g - a.g) * f,
                a.b + (b.b - a.b) * f,
                a.a + (b.a - a.a) * f,
            );
        }
    }
    stops[stops.len() - 1].color
}

fn star_path(r: f64) -> BezPath {
    let mut p = BezPath::new();
    for k in 0..10 {
        let a = -std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::PI / 5.0;
        let rr = if k % 2 == 0 { r } else { r * 0.45 };
        let pt = Point::new(rr * a.cos(), rr * a.sin());
        if k == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p.close_path();
    p
}

fn fill_item(path: BezPath, color: Color) -> DrawItem {
    DrawItem { path, fill_rule: FillRule::NonZero, gradient: None, fill: Some(color), stroke: None, pattern: None }
}

/// What layer `id` draws, in its own space: its shapes, or for a group
/// (an emoji, say) its members' shapes placed as they are in it.
fn sprite_items(comp: &Composition, id: Id, time: f64) -> Vec<DrawItem> {
    let Some(layer) = comp.layer(id) else { return Vec::new() };
    match &layer.kind {
        LayerKind::Group => {
            let to_group = world_at(comp, id, time);
            if to_group.determinant().abs() < 1e-12 {
                return Vec::new();
            }
            let to_group = to_group.inverse();
            let mut out = Vec::new();
            for child in comp.layers.iter().filter(|l| l.meta.parent == Some(id)) {
                let m = to_group * world_at(comp, child.id, time);
                for mut item in sprite_items(comp, child.id, time) {
                    item.path = m * &item.path;
                    if let Some(s) = &mut item.stroke {
                        s.width *= m.determinant().abs().sqrt();
                    }
                    out.push(item);
                }
            }
            out
        }
        _ => layer.shapes().iter().map(|s| crate::draw_item(s, time, 0, None)).collect(),
    }
}

/// The emitter's particles drawn at `time`, in composition space.
pub fn particle_items(comp: &Composition, layer: &Layer, e: &Emitter, time: f64) -> Vec<DrawItem> {
    let ps = particles(comp, layer, e, time);
    // A layer used as the sprite: its drawing, centred and scaled to size 1.
    let sprite: Option<Vec<DrawItem>> = match e.particle {
        ParticleShape::Layer { layer: id } => comp.layer(id).map(|_| {
            let items = sprite_items(comp, id, time);
            let b = items.iter().map(|i| i.path.bounding_box()).reduce(|a, b| a.union(b)).unwrap_or_default();
            let k = 1.0 / b.width().max(b.height()).max(1e-6);
            let m = Affine::scale(k) * Affine::translate(-b.center().to_vec2());
            items
                .into_iter()
                .map(|mut i| {
                    i.path = m * &i.path;
                    i.gradient = None;
                    if let Some(s) = &mut i.stroke {
                        s.width *= k;
                    }
                    i
                })
                .collect()
        }),
        _ => None,
    };
    let mut out = Vec::with_capacity(ps.len());
    for p in &ps {
        let s = p.size;
        let place = Affine::translate(p.position.to_vec2()) * Affine::rotate(p.rotation.to_radians());
        let c = p.color.with_alpha(p.color.a * p.opacity);
        match &e.particle {
            ParticleShape::Confetti => {
                let r = kurbo::Rect::new(-s / 2.0, -s * 0.3, s / 2.0, s * 0.3);
                let flip = p.flip.abs().max(0.08);
                // Shade the back of the paper a little darker.
                let shade = if p.flip < 0.0 { 0.75 } else { 1.0 };
                let c = Color::rgba(c.r * shade, c.g * shade, c.b * shade, c.a);
                out.push(fill_item(place * Affine::scale_non_uniform(1.0, flip) * r.to_path(0.1), c));
            }
            ParticleShape::Circle => {
                out.push(fill_item(kurbo::Circle::new(p.position, s / 2.0).to_path(0.1), c));
            }
            ParticleShape::Square => {
                out.push(fill_item(place * kurbo::Rect::new(-s / 2.0, -s / 2.0, s / 2.0, s / 2.0).to_path(0.1), c));
            }
            ParticleShape::Star => out.push(fill_item(place * star_path(s / 2.0), c)),
            ParticleShape::Streak => {
                let v = p.velocity;
                let len = (v.hypot() * 0.04).max(s);
                let dir = if v.hypot() > 1e-6 { v.normalize() } else { Vec2::new(0.0, 1.0) };
                let mut path = BezPath::new();
                path.move_to(p.position - dir * len);
                path.line_to(p.position);
                out.push(DrawItem {
                    path,
                    fill_rule: FillRule::NonZero,
                    gradient: None,
                    fill: None,
                    stroke: Some(crate::SceneStroke::solid(
                        c,
                        (s * 0.5).max(0.5),
                        xenify_doc::LineJoin::Round,
                        xenify_doc::LineCap::Round,
                    )),
                    pattern: None,
                });
            }
            ParticleShape::Soft => {
                let r = s / 2.0;
                // Colours in steps too small to see: the renderer keeps one colour table per
                // distinct gradient, so hundreds of soft particles share a few dozen instead of
                // each building its own every frame (fire and smoke were far too slow in browsers).
                let q = |v: f64, n: f64| (v * n).round() / n;
                let c = Color::rgba(q(c.r, 48.0), q(c.g, 48.0), q(c.b, 48.0), q(c.a, 40.0));
                if c.a <= 0.0 {
                    continue;
                }
                let edge = c.with_alpha(0.0);
                out.push(DrawItem {
                    path: kurbo::Circle::new(p.position, r).to_path(0.1),
                    fill_rule: FillRule::NonZero,
                    gradient: Some(Gradient {
                        kind: GradientKind::Radial,
                        start: p.position.to_vec2(),
                        end: p.position.to_vec2() + Vec2::new(r, 0.0),
                        stops: vec![
                            GradientStop { offset: 0.0, color: c },
                            GradientStop { offset: 0.5, color: c.with_alpha(c.a * 0.5) },
                            GradientStop { offset: 1.0, color: edge },
                        ],
                    }),
                    fill: Some(c),
                    stroke: None,
                    pattern: None,
                });
            }
            ParticleShape::Ring => out.push(DrawItem {
                path: kurbo::Circle::new(p.position, s / 2.0).to_path(0.1),
                fill_rule: FillRule::NonZero,
                gradient: None,
                fill: None,
                stroke: Some(crate::SceneStroke::solid(
                    c,
                    (s * 0.1).max(0.5),
                    xenify_doc::LineJoin::Round,
                    xenify_doc::LineCap::Round,
                )),
                pattern: None,
            }),
            ParticleShape::Layer { .. } => {
                let Some(items) = &sprite else { continue };
                let m = place * Affine::scale(s);
                for it in items {
                    let mut it = it.clone();
                    it.path = m * &it.path;
                    it.fill = it.fill.map(|f| f.with_alpha(f.a * p.opacity));
                    if let Some(st) = &mut it.stroke {
                        st.width *= s;
                        st.color = st.color.with_alpha(st.color.a * p.opacity);
                    }
                    out.push(it);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Project, ShapeItem};

    fn setup(e: Emitter) -> (Composition, Layer) {
        let mut c = Project::new().compositions.remove(0);
        let layer = Layer::new(Id(1), "Confetti", LayerKind::Particles(Box::new(e.clone())))
            .with_position(Vec2::new(500.0, 800.0));
        let logo = Layer::shape(Id(2), "Logo", vec![ShapeItem::ellipse(Vec2::new(200.0, 200.0), Color::WHITE)])
            .with_position(Vec2::new(900.0, 300.0));
        c.layers = vec![logo, layer.clone()];
        (c, layer)
    }

    fn emitter(c: &Composition) -> &Emitter {
        match &c.layers[1].kind {
            LayerKind::Particles(e) => e,
            _ => unreachable!(),
        }
    }

    #[test]
    fn frames_are_deterministic_and_bursts_count_right() {
        let (c, l) = setup(Emitter::default());
        let a = particles(&c, &l, emitter(&c), 0.5);
        let b = particles(&c, &l, emitter(&c), 0.5);
        assert_eq!(a, b);
        assert_eq!(a.len(), 150);
        assert!(particles(&c, &l, emitter(&c), 10.0).is_empty(), "all faded away");
    }

    #[test]
    fn they_leave_in_the_chosen_direction() {
        let e =
            Emitter { spread: 0.0.into(), gravity: Vec2::ZERO.into(), turbulence: 0.0.into(), ..Emitter::default() };
        let (c, l) = setup(e);
        for p in particles(&c, &l, emitter(&c), 0.2) {
            assert!(p.position.y < 800.0 - 50.0, "up");
            assert!((p.position.x - 500.0).abs() < 1e-6, "straight up");
        }
    }

    #[test]
    fn drag_matches_the_closed_form_and_its_limit() {
        let (p, _) = ballistic(Point::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), 0.0, 2.0);
        assert_eq!(p, Point::new(20.0, 20.0));
        let (p, _) = ballistic(Point::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), 1e-9, 2.0);
        assert!((p - Point::new(20.0, 20.0)).hypot() < 1e-3);
        // Terminal velocity under drag: g/k.
        let (_, v) = ballistic(Point::ZERO, Vec2::ZERO, Vec2::new(0.0, 100.0), 2.0, 50.0);
        assert!((v.y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn settled_particles_lie_on_the_floor() {
        let e = Emitter { end: EndLook::Settle { floor: 1000.0 }, turbulence: 0.0.into(), ..Emitter::default() };
        let (c, l) = setup(e);
        let ps = particles(&c, &l, emitter(&c), 20.0);
        assert!(!ps.is_empty());
        for p in &ps {
            assert!((p.position.y - 1000.0).abs() < 1e-6);
        }
    }

    #[test]
    fn form_puts_every_particle_on_the_targets_outline() {
        let e = Emitter { end: EndLook::Form { target: Id(2) }, ..Emitter::default() };
        let (c, l) = setup(e);
        let ps = particles(&c, &l, emitter(&c), 10.0);
        assert_eq!(ps.len(), 150, "formed particles stay");
        for p in &ps {
            let d = p.position.distance(Point::new(900.0, 300.0));
            assert!((d - 100.0).abs() < 1.0, "on the circle: {d}");
        }
    }

    #[test]
    fn prewarmed_streams_are_full_at_the_start() {
        let e = Emitter {
            mode: EmitMode::Stream,
            prewarm: true,
            lifetime: 2.0.into(),
            lifetime_variation: 0.0,
            ..Emitter::default()
        };
        let (c, l) = setup(e);
        let at_start = particles(&c, &l, emitter(&c), 0.0).len();
        let later = particles(&c, &l, emitter(&c), 3.0).len();
        assert!(at_start > 0 && (at_start as f64 - later as f64).abs() < later as f64 * 0.2, "{at_start} vs {later}");
    }

    #[test]
    fn streams_emit_at_their_rate() {
        let e = Emitter { mode: EmitMode::Stream, rate: 100.0.into(), stop: Some(1.0), ..Emitter::default() };
        let n = births(&e, 5.0).len();
        assert!((99..=101).contains(&n), "{n}");
    }

    #[test]
    fn a_layer_outline_is_a_source() {
        let e = Emitter {
            shape: EmitterShape::Layer { layer: Id(2) },
            speed: 0.0.into(),
            gravity: Vec2::ZERO.into(),
            turbulence: 0.0.into(),
            ..Emitter::default()
        };
        let (c, l) = setup(e);
        for p in particles(&c, &l, emitter(&c), 0.01) {
            let d = p.position.distance(Point::new(900.0, 300.0));
            assert!((d - 100.0).abs() < 1.0, "born on the circle: {d}");
        }
    }
}
