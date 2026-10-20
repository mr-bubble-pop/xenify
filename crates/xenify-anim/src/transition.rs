//! How slides look during a transition: the outgoing and incoming slide's
//! placement, clip (wipes, iris, clock, blinds, liquid), pieces (shatter),
//! blur, colour wash and specks; and Smart Morph, which blends the two
//! slides' layers into one scene.

use crate::{Scene, SceneContent, SceneLayer};
use xenify_doc::{Color, Id, Project, Transition, TransitionKind};
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use std::collections::HashMap;

/// A piece of a slide (Shatter): shown through `clip` (unit space, the
/// slide's own), placed by `transform`, faded by `opacity`.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub clip: BezPath,
    pub transform: Affine,
    pub opacity: f64,
}

/// How a slide is drawn at a moment of a transition. Everything is in unit
/// frame space: (0, 0) is the frame's top left, (1, 1) its bottom right.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub opacity: f64,
    /// Where the slide goes.
    pub transform: Affine,
    /// Only this part of the frame shows it (fixed to the frame).
    pub clip: Option<BezPath>,
    /// The clip's soft edge, as a fraction of the frame's width.
    pub feather: f64,
    /// Drawn as these pieces instead of whole.
    pub pieces: Vec<Piece>,
    /// Blur radius, as a fraction of the frame's width.
    pub blur: f64,
    /// A colour over it: (colour, how strongly).
    pub wash: Option<(Color, f64)>,
    /// Specks drawn over it in `speck_color`: centre, radius (fraction of
    /// the frame's height) and opacity.
    pub specks: Vec<(Point, f64, f64)>,
    pub speck_color: Color,
}

impl Look {
    pub const PLAIN: Look = Look {
        opacity: 1.0,
        transform: Affine::IDENTITY,
        clip: None,
        feather: 0.0,
        pieces: Vec::new(),
        blur: 0.0,
        wash: None,
        specks: Vec::new(),
        speck_color: Color::WHITE,
    };
    pub const HIDDEN: Look = Look {
        opacity: 0.0,
        transform: Affine::IDENTITY,
        clip: None,
        feather: 0.0,
        pieces: Vec::new(),
        blur: 0.0,
        wash: None,
        specks: Vec::new(),
        speck_color: Color::WHITE,
    };

    fn moved(v: Vec2) -> Look {
        Look { transform: Affine::translate(v), ..Look::PLAIN }
    }

    fn faded(opacity: f64) -> Look {
        Look { opacity, ..Look::PLAIN }
    }

    /// The shift, as a fraction of the frame (for tests and simple uses).
    pub fn offset(&self) -> Vec2 {
        self.transform.translation()
    }
}

/// Turn by `degrees` and scale by `(sx, sy)` about the frame's centre, in
/// true proportions for a frame `aspect` wide per unit high.
fn about_centre(aspect: f64, degrees: f64, sx: f64, sy: f64) -> Affine {
    let to_px = Affine::scale_non_uniform(aspect, 1.0);
    let c = Vec2::new(aspect / 2.0, 0.5);
    let m = Affine::translate(c)
        * Affine::rotate(degrees.to_radians())
        * Affine::scale_non_uniform(sx, sy)
        * Affine::translate(-c);
    to_px.inverse() * m * to_px
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.1)
}

/// A circle of `radius` (frame heights) about the centre, in unit space.
fn circle(aspect: f64, centre: Point, radius: f64) -> BezPath {
    kurbo::Ellipse::new(centre, Vec2::new(radius / aspect, radius), 0.0).to_path(0.05)
}

fn rand(seed: u64, i: usize, c: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// What the outgoing and incoming slides look like `p` (0…1, eased) of the
/// way through `tr`, for frames `aspect` wide per unit high; and whether the
/// outgoing slide is drawn on top.
pub fn looks(tr: &Transition, p: f64, aspect: f64) -> (Look, Look, bool) {
    let p = p.clamp(0.0, 1.0);
    let q = 1.0 - p;
    let d = tr.direction.vector();
    let feather = tr.softness.clamp(0.0, 1.0) * 0.2;
    match tr.kind {
        TransitionKind::Cut => {
            if p < 1.0 {
                (Look::PLAIN, Look::HIDDEN, false)
            } else {
                (Look::HIDDEN, Look::PLAIN, false)
            }
        }
        TransitionKind::Fade | TransitionKind::SmartMorph => (Look::PLAIN, Look::faded(p), false),
        TransitionKind::DipToColor => {
            if p < 0.5 {
                (Look { wash: Some((tr.color, p * 2.0)), ..Look::PLAIN }, Look::HIDDEN, false)
            } else {
                (Look::HIDDEN, Look { wash: Some((tr.color, q * 2.0)), ..Look::PLAIN }, false)
            }
        }
        TransitionKind::Push => (Look::moved(d * p), Look::moved(-d * q), false),
        TransitionKind::Cover => (Look::PLAIN, Look::moved(-d * q), false),
        TransitionKind::Uncover => (Look::moved(d * p), Look::PLAIN, true),
        TransitionKind::Wipe => {
            // The edge moves the way of the direction, showing the new slide behind it.
            let clip = match tr.direction {
                xenify_doc::Direction::Left => rect(1.0 - p, -0.1, 1.1, 1.1),
                xenify_doc::Direction::Right => rect(-0.1, -0.1, p, 1.1),
                xenify_doc::Direction::Up => rect(-0.1, 1.0 - p, 1.1, 1.1),
                xenify_doc::Direction::Down => rect(-0.1, -0.1, 1.1, p),
            };
            (Look::PLAIN, Look { clip: Some(clip), feather, ..Look::PLAIN }, false)
        }
        TransitionKind::ClockWipe => {
            let c = Point::new(0.5, 0.5);
            let mut clip = BezPath::new();
            clip.move_to(c);
            let steps = 64;
            let sweep = p * std::f64::consts::TAU;
            for i in 0..=steps {
                let a = -std::f64::consts::FRAC_PI_2 + sweep * i as f64 / steps as f64;
                clip.line_to(c + Vec2::new(a.cos(), a.sin()) * 1.0);
            }
            clip.close_path();
            (Look::PLAIN, Look { clip: Some(clip), feather, ..Look::PLAIN }, false)
        }
        TransitionKind::Iris => {
            let reach = ((aspect / 2.0).powi(2) + 0.25).sqrt() * (1.0 + feather * 2.0);
            let clip = circle(aspect, Point::new(0.5, 0.5), reach * p);
            (Look::PLAIN, Look { clip: Some(clip), feather, ..Look::PLAIN }, false)
        }
        TransitionKind::Blinds => {
            let n = 8;
            let mut clip = BezPath::new();
            let across = d.x != 0.0;
            for i in 0..n {
                let a = i as f64 / n as f64;
                let b = a + p / n as f64;
                let (a, b) = if d.x > 0.0 || d.y > 0.0 {
                    (a, b)
                } else {
                    ((i + 1) as f64 / n as f64 - p / n as f64, (i + 1) as f64 / n as f64)
                };
                let r = if across { rect(a, -0.1, b, 1.1) } else { rect(-0.1, a, 1.1, b) };
                clip.extend(r);
            }
            (Look::PLAIN, Look { clip: Some(clip), feather: feather * 0.2, ..Look::PLAIN }, false)
        }
        TransitionKind::ZoomIn => (
            Look { opacity: q, transform: about_centre(aspect, 0.0, 1.0 + 0.5 * p, 1.0 + 0.5 * p), ..Look::PLAIN },
            Look { opacity: p, transform: about_centre(aspect, 0.0, 0.6 + 0.4 * p, 0.6 + 0.4 * p), ..Look::PLAIN },
            false,
        ),
        TransitionKind::ZoomOut => (
            Look { opacity: q, transform: about_centre(aspect, 0.0, 1.0 - 0.4 * p, 1.0 - 0.4 * p), ..Look::PLAIN },
            Look { opacity: p, transform: about_centre(aspect, 0.0, 1.4 - 0.4 * p, 1.4 - 0.4 * p), ..Look::PLAIN },
            false,
        ),
        TransitionKind::Spin => (
            Look { opacity: q, transform: about_centre(aspect, 90.0 * p, 1.0 - 0.5 * p, 1.0 - 0.5 * p), ..Look::PLAIN },
            Look {
                opacity: p,
                transform: about_centre(aspect, -90.0 * q, 0.5 + 0.5 * p, 0.5 + 0.5 * p),
                ..Look::PLAIN
            },
            false,
        ),
        TransitionKind::Flip => {
            let turn = (std::f64::consts::PI * p).cos();
            let squash = |k: f64| {
                if d.x != 0.0 { about_centre(aspect, 0.0, k, 1.0) } else { about_centre(aspect, 0.0, 1.0, k) }
            };
            if p < 0.5 {
                (Look { transform: squash(turn.max(0.001)), ..Look::PLAIN }, Look::HIDDEN, false)
            } else {
                (Look::HIDDEN, Look { transform: squash((-turn).max(0.001)), ..Look::PLAIN }, false)
            }
        }
        TransitionKind::BlurDissolve => (
            Look { opacity: 1.0, blur: 0.03 * p, ..Look::PLAIN },
            Look { opacity: p, blur: 0.03 * q, ..Look::PLAIN },
            false,
        ),
        TransitionKind::Liquid => {
            // A wavy surface rising the other way from the direction (Up: from the bottom).
            let amp = 0.04;
            let level = p * (1.0 + 2.0 * amp) - amp;
            let mut clip = BezPath::new();
            let steps = 48;
            let wave = |u: f64| amp * (u * std::f64::consts::TAU * 1.5 + p * 9.0).sin();
            let point = |u: f64, depth: f64| match tr.direction {
                xenify_doc::Direction::Up | xenify_doc::Direction::Left => {
                    if tr.direction == xenify_doc::Direction::Up {
                        Point::new(u, 1.0 - depth)
                    } else {
                        Point::new(1.0 - depth, u)
                    }
                }
                xenify_doc::Direction::Down => Point::new(u, depth),
                xenify_doc::Direction::Right => Point::new(depth, u),
            };
            clip.move_to(point(-0.1, -0.2));
            for i in 0..=steps {
                let u = -0.1 + 1.2 * i as f64 / steps as f64;
                clip.line_to(point(u, level + wave(u)));
            }
            clip.line_to(point(1.1, -0.2));
            clip.close_path();
            (Look::PLAIN, Look { clip: Some(clip), ..Look::PLAIN }, false)
        }
        TransitionKind::Shatter => {
            let bounds = Rect::new(0.0, 0.0, aspect, 1.0);
            let to_unit = Affine::scale_non_uniform(1.0 / aspect, 1.0);
            let origin = Point::new(aspect / 2.0, 0.5);
            let pieces = crate::shatter::shards(bounds, 30, 7)
                .into_iter()
                .enumerate()
                .map(|(i, cell)| {
                    let n = cell.len() as f64;
                    let c = (cell.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / n).to_point();
                    let away = (c - origin).normalize();
                    let v = away * (0.6 + 0.6 * rand(3, i, 1)) + Vec2::new(0.0, -0.4);
                    let t = p * 1.2;
                    let offset = v * t + Vec2::new(0.0, 1.6 * t * t);
                    let turn = (rand(3, i, 2) * 2.0 - 1.0) * 120.0 * t;
                    let m = Affine::translate(offset)
                        * Affine::translate(c.to_vec2())
                        * Affine::rotate(turn.to_radians())
                        * Affine::translate(-c.to_vec2());
                    let mut clip = BezPath::new();
                    clip.move_to(cell[0]);
                    for q in &cell[1..] {
                        clip.line_to(*q);
                    }
                    clip.close_path();
                    Piece {
                        clip: to_unit * clip,
                        transform: to_unit * m * to_unit.inverse(),
                        opacity: (1.0 - (p - 0.6) / 0.4).clamp(0.0, 1.0),
                    }
                })
                .collect();
            (Look { pieces, ..Look::PLAIN }, Look::PLAIN, true)
        }
        TransitionKind::ParticleDissolve => {
            // The new slide shows through dots that grow at random; specks of
            // the old one drift up from them.
            let cols = 28usize;
            let rows = ((cols as f64 / aspect).round() as usize).max(1);
            let cell = 1.0 / rows as f64; // in frame heights
            let mut clip = BezPath::new();
            let mut specks = Vec::new();
            for r in 0..rows {
                for c in 0..cols {
                    let i = r * cols + c;
                    let start = rand(5, i, 1) * 0.6;
                    let k = ((p - start) / 0.4).clamp(0.0, 1.0);
                    if k <= 0.0 {
                        continue;
                    }
                    let centre = Point::new((c as f64 + 0.5) / cols as f64, (r as f64 + 0.5) / rows as f64);
                    clip.extend(circle(aspect, centre, cell * 0.75 * k));
                    if k < 1.0 {
                        let rise = k * 0.08;
                        specks.push((centre + Vec2::new(0.0, -rise), cell * 0.18 * (1.0 - k * 0.5), 1.0 - k));
                    }
                }
            }
            (Look::PLAIN, Look { clip: Some(clip), specks, speck_color: tr.color, ..Look::PLAIN }, false)
        }
    }
}

/// Decomposed placement: position, turn (radians), scale.
fn parts(m: Affine) -> (Vec2, f64, Vec2) {
    let [a, b, c, d, e, f] = m.as_coeffs();
    let sx = a.hypot(b);
    let r = b.atan2(a);
    let sy = if sx > 1e-12 { (a * d - b * c) / sx } else { 0.0 };
    (Vec2::new(e, f), r, Vec2::new(sx, sy))
}

fn lerp_affine(a: Affine, b: Affine, t: f64) -> Affine {
    let (pa, ra, sa) = parts(a);
    let (pb, rb, sb) = parts(b);
    let mut dr = rb - ra;
    while dr > std::f64::consts::PI {
        dr -= std::f64::consts::TAU;
    }
    while dr < -std::f64::consts::PI {
        dr += std::f64::consts::TAU;
    }
    let s = sa.lerp(sb, t);
    Affine::translate(pa.lerp(pb, t)) * Affine::rotate(ra + dr * t) * Affine::scale_non_uniform(s.x, s.y)
}

fn mix(a: Color, b: Color, t: f64) -> Color {
    Color::rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, a.a + (b.a - a.a) * t)
}

/// The pairs Smart Morph makes between two slides: (A's layer, B's layer)
/// with the same name and the same kind of drawing.
pub fn morph_pairs(project: &Project, a: Id, b: Id) -> Vec<(Id, Id, String)> {
    let (Ok(ca), Ok(cb)) = (project.comp(a), project.comp(b)) else { return Vec::new() };
    let mut used = std::collections::HashSet::new();
    let mut out = Vec::new();
    for la in ca.layers.iter().filter(|l| l.meta.parent.is_none()) {
        let kind = std::mem::discriminant(&la.kind);
        if let Some(lb) = cb.layers.iter().find(|lb| {
            lb.meta.parent.is_none()
                && !used.contains(&lb.id)
                && lb.meta.name == la.meta.name
                && std::mem::discriminant(&lb.kind) == kind
        }) {
            used.insert(lb.id);
            out.push((la.id, lb.id, la.meta.name.clone()));
        }
    }
    out
}

/// Smart Morph `t` (0…1) of the way from `a` to `b`: layers paired by
/// [`morph_pairs`] glide and blend; the rest fade out and in.
pub fn morph_scene(project: &Project, a: (Id, f64), b: (Id, f64), t: f64) -> Scene {
    let sa = crate::evaluate_project(project, a.0, a.1);
    let sb = crate::evaluate_project(project, b.0, b.1);
    let pairs: HashMap<Id, Id> = morph_pairs(project, a.0, b.0).into_iter().map(|(x, y, _)| (y, x)).collect();
    let paired_a: std::collections::HashSet<Id> = pairs.values().copied().collect();
    let by_id_a: HashMap<Id, &SceneLayer> = sa.layers.iter().filter(|l| l.copy == 0).map(|l| (l.id, l)).collect();
    let mut layers = Vec::new();
    // What only A has, fading out underneath.
    for l in sa.layers.iter().filter(|l| !paired_a.contains(&l.id)) {
        let mut l = l.clone();
        l.opacity *= 1.0 - t;
        layers.push(l);
    }
    for lb in &sb.layers {
        let Some(la) = pairs.get(&lb.id).and_then(|a| by_id_a.get(a)).filter(|_| lb.copy == 0) else {
            let mut l = lb.clone();
            l.opacity *= t;
            layers.push(l);
            continue;
        };
        let transform = lerp_affine(la.transform, lb.transform, t);
        let opacity = la.opacity + (lb.opacity - la.opacity) * t;
        let blended = match (&la.content, &lb.content) {
            (SceneContent::Shapes(ia), SceneContent::Shapes(ib)) if ia.len() == ib.len() => {
                Some(SceneContent::Shapes(ia.iter().zip(ib).map(|(x, y)| crate::morph::lerp_items(x, y, t)).collect()))
            }
            (SceneContent::Text(ta), SceneContent::Text(tb)) if ta.text == tb.text => {
                let mut text = tb.clone();
                text.font_size = ta.font_size + (tb.font_size - ta.font_size) * t;
                text.color = mix(ta.color, tb.color, t);
                Some(SceneContent::Text(text))
            }
            (SceneContent::Image(ia), SceneContent::Image(ib)) if ia.hash == ib.hash => Some(lb.content.clone()),
            _ => None,
        };
        match blended {
            Some(content) => layers.push(SceneLayer { transform, opacity, content, ..lb.clone() }),
            None => {
                // Different drawings: both ride the same path, crossfading.
                layers.push(SceneLayer { transform, opacity: la.opacity * (1.0 - t), ..(*la).clone() });
                layers.push(SceneLayer { transform, opacity: lb.opacity * t, ..lb.clone() });
            }
        }
    }
    Scene {
        width: sb.width,
        height: sb.height,
        background: mix(sa.background, sb.background, t),
        layers,
        camera: lerp_affine(sa.camera, sb.camera, t),
        time: b.1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Direction, ShapeItem};

    fn tr(kind: TransitionKind) -> Transition {
        Transition::new(kind, 1.0)
    }

    #[test]
    fn every_kind_starts_on_the_old_slide_and_ends_on_the_new() {
        for kind in TransitionKind::ALL {
            let (out0, in0, _) = looks(&tr(kind), 0.0, 16.0 / 9.0);
            let (out1, in1, swap1) = looks(&tr(kind), 1.0, 16.0 / 9.0);
            // At the start the new slide can't be seen.
            let hidden_in = in0.opacity == 0.0
                || in0.clip.as_ref().is_some_and(|c| {
                    c.area().abs() < 1e-6 || c.bounding_box().intersect(Rect::new(0.0, 0.0, 1.0, 1.0)).area() < 1e-6
                })
                || in0.transform.translation().hypot() >= 1.0 - 1e-9;
            assert!(hidden_in || kind == TransitionKind::Uncover || kind == TransitionKind::Shatter, "{kind:?} start");
            assert!(out0.opacity > 0.0, "{kind:?}: the old slide shows at the start");
            // At the end the new slide shows whole (or the old one is gone from over it).
            assert!(in1.opacity > 0.99, "{kind:?} end opacity");
            if !swap1 {
                assert!(in1.transform.translation().hypot() < 1e-9, "{kind:?} end place");
            } else {
                let gone = out1.opacity == 0.0
                    || out1.pieces.iter().all(|p| p.opacity == 0.0)
                    || out1.transform.translation().hypot() >= 1.0 - 1e-9;
                assert!(gone, "{kind:?}: the old slide is out of the way");
            }
        }
    }

    #[test]
    fn wipes_reveal_from_the_side_they_start() {
        let t = Transition { direction: Direction::Right, ..tr(TransitionKind::Wipe) };
        let (_, inn, _) = looks(&t, 0.25, 1.0);
        let b = inn.clip.unwrap().bounding_box();
        assert!((b.x1 - 0.25).abs() < 1e-9, "a quarter from the left: {b:?}");
    }

    #[test]
    fn the_iris_is_round_on_a_wide_frame() {
        let (_, inn, _) = looks(&tr(TransitionKind::Iris), 0.5, 2.0);
        let b = inn.clip.unwrap().bounding_box();
        // Twice as wide as high in unit space would be an ellipse; on a 2:1 frame it's a circle.
        assert!((b.width() * 2.0 - b.height()).abs() < 1e-3, "{b:?}");
    }

    #[test]
    fn smart_morph_glides_layers_with_the_same_name() {
        let mut p = Project::new();
        let a = p.compositions[0].id;
        let b = p.alloc_id();
        let mut cb = p.compositions[0].clone();
        cb.id = b;
        let mk = |x: f64| {
            xenify_doc::Layer::shape(Id(1), "Logo", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE)])
                .with_position(Vec2::new(x, 300.0))
        };
        p.compositions[0].layers = vec![mk(100.0)];
        let mut other = mk(900.0);
        other.id = Id(2);
        cb.layers = vec![other];
        p.compositions.push(cb);
        assert_eq!(morph_pairs(&p, a, b).len(), 1);
        let mid = morph_scene(&p, (a, 0.0), (b, 0.0), 0.5);
        let logo = mid.layers.iter().find(|l| l.id == Id(2)).unwrap();
        assert!((logo.transform.translation().x - 500.0).abs() < 1e-6, "half way along");
        assert_eq!(mid.layers.len(), 1, "nothing to fade");
    }
}
