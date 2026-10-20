//! What shape-changing modifiers do to a path: jitter, zig zag / wave,
//! pucker & bloat, twist, smooth, subdivide, simplify and offset path.

use xenify_doc::bezier::{self, Node, SubPath};
use xenify_doc::{Deform, LineJoin};
use kurbo::{
    Affine, BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathEl, PathSeg, Point, Vec2,
};

/// Pieces per segment are capped at this, and whole paths at [`MAX_POINTS`].
const MAX_PIECES: usize = 200;
const MAX_POINTS: usize = 20_000;

/// `path` with `deform` applied (all in one space).
pub fn apply(deform: &Deform, path: &BezPath) -> BezPath {
    if path.elements().is_empty() {
        return path.clone();
    }
    match *deform {
        Deform::Jitter { amount, detail, seed, smooth } => jitter(path, amount, detail, seed, smooth),
        Deform::ZigZag { amount, ridges, smooth } => zig_zag(path, amount, ridges, smooth),
        Deform::PuckerBloat { strength } => pucker_bloat(path, strength),
        Deform::Twist { degrees, centre } => twist(path, degrees, centre),
        Deform::Smooth { strength } => smooth(path, strength),
        Deform::Subdivide { levels } => (0..levels).fold(path.clone(), |p, _| {
            if bezier::point_count(&p) * 2 > MAX_POINTS { p } else { bezier::subdivide(&p, &[]) }
        }),
        Deform::Simplify { tolerance } if tolerance > 0.0 => bezier::simplify(path, tolerance),
        Deform::Simplify { .. } => path.clone(),
        Deform::OffsetPath { amount, join } => offset_path(path, amount, join),
        Deform::MirrorMerge { axis } => union(&closed(path), &closed(&(axis * path))).unwrap_or_else(|| path.clone()),
    }
}

/// `path` in the copy's own space, changed by the copy's deforms (each in the
/// space its modifier saw). `to_space` takes the path into the layer's space
/// first (for the members of a group).
pub fn deform_copy(copy: &xenify_doc::CopyPlan, path: &BezPath, to_space: Affine) -> BezPath {
    if copy.deforms.is_empty() || to_space.determinant().abs() < 1e-12 {
        return path.clone();
    }
    let mut p = to_space * path;
    for (a, d) in &copy.deforms {
        if a.determinant().abs() < 1e-12 {
            continue;
        }
        p = a.inverse() * apply(d, &(*a * &p));
    }
    to_space.inverse() * &p
}

/// Every segment of every subpath, with the closing line made explicit.
fn subpaths(path: &BezPath) -> Vec<(Vec<PathSeg>, bool)> {
    let mut out: Vec<(Vec<PathSeg>, bool)> = Vec::new();
    let (mut start, mut cur) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.push((Vec::new(), false));
                (start, cur) = (p, p);
            }
            PathEl::ClosePath => {
                if let Some((segs, closed)) = out.last_mut() {
                    if (cur - start).hypot() > 1e-9 {
                        segs.push(PathSeg::Line(Line::new(cur, start)));
                    }
                    *closed = true;
                }
                cur = start;
            }
            _ => {
                if out.is_empty() {
                    out.push((Vec::new(), false));
                }
                let seg = match *el {
                    PathEl::LineTo(p) => PathSeg::Line(Line::new(cur, p)),
                    PathEl::QuadTo(a, b) => PathSeg::Quad(kurbo::QuadBez::new(cur, a, b)),
                    PathEl::CurveTo(a, b, c) => PathSeg::Cubic(CubicBez::new(cur, a, b, c)),
                    _ => unreachable!(),
                };
                cur = seg.end();
                out.last_mut().unwrap().0.push(seg);
            }
        }
    }
    out.retain(|(s, _)| !s.is_empty());
    out
}

/// The parameters splitting `seg` into `n` pieces of equal length.
fn even_params(seg: &PathSeg, n: usize) -> Vec<f64> {
    let len = seg.arclen(0.01);
    (0..=n)
        .map(|k| match seg {
            PathSeg::Line(_) => k as f64 / n as f64,
            _ if k == 0 => 0.0,
            _ if k == n => 1.0,
            _ => seg.inv_arclen(len * k as f64 / n as f64, 0.01),
        })
        .collect()
}

fn to_cubic(seg: PathSeg) -> CubicBez {
    match seg {
        PathSeg::Line(l) => CubicBez::new(l.p0, l.p0, l.p1, l.p1),
        PathSeg::Quad(q) => q.raise(),
        PathSeg::Cubic(c) => c,
    }
}

/// The path with each segment split into `pieces(seg)` pieces of equal length
/// (as nodes; straight pieces have no handles).
fn resample(path: &BezPath, pieces: impl Fn(&PathSeg) -> usize) -> Vec<SubPath> {
    let mut budget = MAX_POINTS;
    subpaths(path)
        .into_iter()
        .map(|(segs, closed)| {
            let mut nodes: Vec<Node> = vec![Node { pos: segs[0].start(), handle_in: None, handle_out: None }];
            for seg in &segs {
                let n = pieces(seg).clamp(1, MAX_PIECES).min(budget.max(1));
                budget = budget.saturating_sub(n);
                let ts = even_params(seg, n);
                for w in ts.windows(2) {
                    let piece = seg.subsegment(w[0]..w[1]);
                    let (h_out, h_in) = match piece {
                        PathSeg::Line(_) => (None, None),
                        p => {
                            let c = to_cubic(p);
                            (Some(c.p1), Some(c.p2))
                        }
                    };
                    nodes.last_mut().unwrap().handle_out = h_out;
                    nodes.push(Node { pos: piece.end(), handle_in: h_in, handle_out: None });
                }
            }
            if closed && nodes.len() > 1 {
                // The last node is the first one again.
                let last = nodes.pop().unwrap();
                nodes[0].handle_in = last.handle_in;
            }
            SubPath { nodes, closed }
        })
        .collect()
}

/// A repeatable pseudo-random number in −1…1 for a lattice point.
fn hash(x: i64, y: i64, s: i64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (s as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Smooth value noise in −1…1.
fn noise(p: Point, s: i64) -> f64 {
    let (x0, y0) = (p.x.floor(), p.y.floor());
    let (fx, fy) = (p.x - x0, p.y - y0);
    let ease = |t: f64| t * t * (3.0 - 2.0 * t);
    let (u, v) = (ease(fx), ease(fy));
    let (ix, iy) = (x0 as i64, y0 as i64);
    let a = hash(ix, iy, s) + (hash(ix + 1, iy, s) - hash(ix, iy, s)) * u;
    let b = hash(ix, iy + 1, s) + (hash(ix + 1, iy + 1, s) - hash(ix, iy + 1, s)) * u;
    a + (b - a) * v
}

/// A random offset of up to `amount` for the point at `p`; nearby points
/// (within `scale`) move alike. Between whole seeds it blends smoothly.
fn wobble(p: Point, amount: f64, scale: f64, seed: f64) -> Vec2 {
    let q = (p.to_vec2() / scale.max(1e-3)).to_point();
    let at = |s: i64| Vec2::new(noise(q, 2 * s), noise(q + Vec2::new(31.7, 17.3), 2 * s + 1));
    let s0 = seed.floor();
    let f = seed - s0;
    let f = f * f * (3.0 - 2.0 * f);
    let (a, b) = (at(s0 as i64), at(s0 as i64 + 1));
    (a + (b - a) * f) * amount
}

fn jitter(path: &BezPath, amount: f64, detail: f64, seed: f64, smooth: bool) -> BezPath {
    let spacing = if detail > 0.0 { 100.0 / detail } else { f64::INFINITY };
    let mut subs = resample(path, |seg| (seg.arclen(0.1) / spacing).ceil() as usize);
    // Noise features about as far apart as the points, so neighbours differ.
    let scale = if spacing.is_finite() { spacing * 0.8 } else { 40.0 };
    for sp in &mut subs {
        for n in &mut sp.nodes {
            let d = wobble(n.pos, amount, scale, seed);
            n.pos += d;
            if smooth {
                n.handle_in = n.handle_in.map(|h| h + d);
                n.handle_out = n.handle_out.map(|h| h + d);
            } else {
                (n.handle_in, n.handle_out) = (None, None);
            }
        }
    }
    bezier::from_nodes(&subs)
}

/// The unit normal (left of the direction of travel) at `t`.
fn normal(seg: &PathSeg, t: f64) -> Vec2 {
    let d = to_cubic(*seg).deriv().eval(t).to_vec2();
    let d = if d.hypot() < 1e-9 { seg.end() - seg.start() } else { d };
    if d.hypot() < 1e-12 { Vec2::ZERO } else { Vec2::new(d.y, -d.x).normalize() }
}

/// A smooth curve through `points` (Catmull-Rom).
fn through(points: &[Point], closed: bool) -> SubPath {
    let n = points.len();
    let at = |i: isize| -> Point {
        if closed { points[i.rem_euclid(n as isize) as usize] } else { points[i.clamp(0, n as isize - 1) as usize] }
    };
    let nodes = (0..n as isize)
        .map(|i| {
            let tangent = (at(i + 1) - at(i - 1)) / 6.0;
            let (first, last) = (!closed && i == 0, !closed && i == n as isize - 1);
            Node {
                pos: at(i),
                handle_in: (!first).then(|| at(i) - tangent),
                handle_out: (!last).then(|| at(i) + tangent),
            }
        })
        .collect();
    SubPath { nodes, closed }
}

fn zig_zag(path: &BezPath, amount: f64, ridges: usize, smooth: bool) -> BezPath {
    if ridges == 0 || amount == 0.0 {
        return path.clone();
    }
    let n = (2 * ridges).min(MAX_PIECES);
    let subs = subpaths(path)
        .into_iter()
        .map(|(segs, closed)| {
            let mut points = vec![segs[0].start()];
            for seg in &segs {
                for (k, t) in even_params(seg, n).into_iter().enumerate().skip(1) {
                    // Corners stay put; the points between go out and in by turns.
                    let side = if k == n {
                        0.0
                    } else if k % 2 == 1 {
                        1.0
                    } else {
                        -1.0
                    };
                    points.push(seg.eval(t) + normal(seg, t) * amount * side);
                }
            }
            if closed {
                points.pop();
            }
            if smooth {
                through(&points, closed)
            } else {
                let nodes = points.into_iter().map(|pos| Node { pos, handle_in: None, handle_out: None }).collect();
                SubPath { nodes, closed }
            }
        })
        .collect::<Vec<_>>();
    bezier::from_nodes(&subs)
}

/// As lottie-web: points move towards the middle of their subpath by
/// `strength` and handles away from it by the same amount.
fn pucker_bloat(path: &BezPath, strength: f64) -> BezPath {
    let mut subs = bezier::to_nodes(path);
    for sp in &mut subs {
        let n = sp.nodes.len().max(1) as f64;
        let c = sp.nodes.iter().fold(Vec2::ZERO, |a, x| a + x.pos.to_vec2()) / n;
        let c = c.to_point();
        let pull = |p: Point, k: f64| p + (c - p) * k;
        for node in &mut sp.nodes {
            let pos = node.pos;
            node.handle_in = Some(pull(node.handle_in.unwrap_or(pos), -strength));
            node.handle_out = Some(pull(node.handle_out.unwrap_or(pos), -strength));
            node.pos = pull(pos, strength);
        }
        if !sp.closed {
            // Open ends have no outer segment to bend.
            if let Some(f) = sp.nodes.first_mut() {
                f.handle_in = None;
            }
            if let Some(l) = sp.nodes.last_mut() {
                l.handle_out = None;
            }
        }
    }
    bezier::from_nodes(&subs)
}

fn twist(path: &BezPath, degrees: f64, centre: Point) -> BezPath {
    if degrees == 0.0 {
        return path.clone();
    }
    let reach = bezier::anchors(path).iter().map(|a| (a.pos - centre).hypot()).fold(0.0, f64::max);
    if reach < 1e-9 {
        return path.clone();
    }
    // Short pieces, so the curves can bend.
    let step = reach / 16.0;
    let mut subs = resample(path, |seg| (seg.arclen(0.1) / step).ceil() as usize);
    let warp = |p: Point| {
        let t = 1.0 - ((p - centre).hypot() / reach).min(1.0);
        let c = centre.to_vec2();
        Affine::translate(c) * Affine::rotate((degrees * t).to_radians()) * Affine::translate(-c) * p
    };
    for sp in &mut subs {
        // Straight pieces get handles a third of the way along, so they can bend.
        let count = sp.nodes.len();
        let pos: Vec<Point> = sp.nodes.iter().map(|n| n.pos).collect();
        for (i, n) in sp.nodes.iter_mut().enumerate() {
            let next = if i + 1 < count { Some(pos[i + 1]) } else { sp.closed.then(|| pos[0]) };
            let prev = if i > 0 { Some(pos[i - 1]) } else { sp.closed.then(|| pos[count - 1]) };
            if n.handle_out.is_none() {
                n.handle_out = next.map(|q| n.pos.lerp(q, 1.0 / 3.0));
            }
            if n.handle_in.is_none() {
                n.handle_in = prev.map(|q| n.pos.lerp(q, 1.0 / 3.0));
            }
        }
        for n in &mut sp.nodes {
            n.pos = warp(n.pos);
            n.handle_in = n.handle_in.map(warp);
            n.handle_out = n.handle_out.map(warp);
        }
    }
    bezier::from_nodes(&subs)
}

fn smooth(path: &BezPath, strength: f64) -> BezPath {
    let mut subs = bezier::to_nodes(path);
    for sp in &mut subs {
        let n = sp.nodes.len();
        if n < 3 && !(sp.closed && n == 2) {
            continue;
        }
        let pos: Vec<Point> = sp.nodes.iter().map(|x| x.pos).collect();
        let round = through(&pos, sp.closed);
        for (node, r) in sp.nodes.iter_mut().zip(round.nodes) {
            let mix = |own: Option<Point>, target: Option<Point>| match target {
                Some(t) => Some(own.unwrap_or(node.pos).lerp(t, strength)),
                None => own,
            };
            node.handle_in = mix(node.handle_in, r.handle_in);
            node.handle_out = mix(node.handle_out, r.handle_out);
        }
    }
    bezier::from_nodes(&subs)
}

/// Grows (> 0) or shrinks (< 0) the filled area by `amount`: the outline
/// joined with (or cut by) a stroke twice as wide along it.
fn offset_path(path: &BezPath, amount: f64, join: LineJoin) -> BezPath {
    if amount.abs() < 1e-9 {
        return path.clone();
    }
    let join = match join {
        LineJoin::Miter => kurbo::Join::Miter,
        LineJoin::Round => kurbo::Join::Round,
        LineJoin::Bevel => kurbo::Join::Bevel,
    };
    let style = kurbo::Stroke::new(amount.abs() * 2.0).with_join(join).with_miter_limit(4.0);
    let band = kurbo::stroke(path.iter(), &style, &kurbo::StrokeOpts::default(), 0.05);
    let op = if amount > 0.0 { linesweeper::BinaryOp::Union } else { linesweeper::BinaryOp::Difference };
    boolean(&closed(path), &band, op).unwrap_or_else(|| path.clone())
}

/// `path` with every subpath closed (booleans need closed outlines).
fn closed(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    for (segs, _) in subpaths(path) {
        out.move_to(segs[0].start());
        for s in segs {
            match s {
                PathSeg::Line(l) => out.line_to(l.p1),
                PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
            }
        }
        out.close_path();
    }
    out
}

fn boolean(a: &BezPath, b: &BezPath, op: linesweeper::BinaryOp) -> Option<BezPath> {
    let contours = linesweeper::binary_op(a, b, linesweeper::FillRule::NonZero, op).ok()?;
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(c.path.iter());
    }
    Some(out)
}

/// The part of filled outline `a` inside `b`.
pub fn intersect(a: &BezPath, b: &BezPath) -> Option<BezPath> {
    boolean(a, b, linesweeper::BinaryOp::Intersection)
}

fn union(a: &BezPath, b: &BezPath) -> Option<BezPath> {
    boolean(a, b, linesweeper::BinaryOp::Union)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Rect, Shape};

    fn square() -> BezPath {
        Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1)
    }

    fn area(p: &BezPath) -> f64 {
        p.area().abs()
    }

    #[test]
    fn jitter_moves_points_but_repeats_itself() {
        let d = Deform::Jitter { amount: 5.0, detail: 10.0, seed: 3.0, smooth: false };
        let a = apply(&d, &square());
        assert_eq!(bezier::point_count(&a), 40, "10 points per 100 units on each side");
        assert_eq!(a, apply(&d, &square()), "the same seed gives the same result");
        let moved = bezier::anchors(&a).iter().filter(|x| x.pos.x.fract().abs() > 1e-6).count();
        assert!(moved > 30, "most points moved");
        assert!(bezier::anchors(&a).iter().all(|x| x.pos.x > -6.0 && x.pos.x < 106.0));
        let other = apply(&Deform::Jitter { amount: 5.0, detail: 10.0, seed: 4.0, smooth: false }, &square());
        assert_ne!(a, other);
    }

    #[test]
    fn zig_zag_pucker_twist_and_smooth() {
        let z = apply(&Deform::ZigZag { amount: 10.0, ridges: 3, smooth: false }, &square());
        assert_eq!(bezier::point_count(&z), 24, "six pieces per side");
        let b = z.bounding_box();
        assert!(b.x0 < -9.0 && b.x1 > 109.0, "ridges stand out both ways: {b:?}");

        let bloat = apply(&Deform::PuckerBloat { strength: 0.5 }, &square());
        let pucker = apply(&Deform::PuckerBloat { strength: -0.5 }, &square());
        // Puckered: the corners go out to a star, the sides curve in.
        let corners = bezier::anchors(&pucker);
        assert!((corners[0].pos - Point::new(-25.0, -25.0)).hypot() < 1e-9, "{:?}", corners[0].pos);
        assert!(pucker.contains(Point::new(50.0, 50.0)) && !pucker.contains(Point::new(50.0, 2.0)));
        assert!(bloat.bounding_box().width() > 100.0, "bloated curves bulge out");

        let t = apply(&Deform::Twist { degrees: 90.0, centre: Point::new(50.0, 50.0) }, &square());
        assert!((area(&t) - area(&square())).abs() < 800.0, "twisting keeps roughly the area: {}", area(&t));
        let corners = bezier::anchors(&t);
        assert!(corners.iter().any(|a| (a.pos - Point::new(0.0, 0.0)).hypot() < 1e-6), "the far corners stay put");

        let round = apply(&Deform::Smooth { strength: 1.0 }, &square());
        // Through the same corners, rounded: it bulges out towards a circle.
        assert!(area(&round) > area(&square()) && area(&round) < 15_708.0, "{}", area(&round));
        assert!(bezier::anchors(&round).iter().all(|a| a.handle_in.is_some() && a.handle_out.is_some()));
        assert_eq!(bezier::point_count(&round), 4);
    }

    #[test]
    fn offset_grows_and_shrinks_and_subdivide_simplify() {
        let out = apply(&Deform::OffsetPath { amount: 10.0, join: LineJoin::Miter }, &square());
        assert!((out.bounding_box().width() - 120.0).abs() < 0.5, "{:?}", out.bounding_box());
        let inset = apply(&Deform::OffsetPath { amount: -10.0, join: LineJoin::Round }, &square());
        assert!((area(&inset) - 6400.0).abs() < 5.0, "{}", area(&inset));

        // Mirrored across x = 100 and merged: one outline twice as wide.
        let axis = xenify_doc::reflection(Vec2::new(100.0, 0.0), 90.0);
        let both = apply(&Deform::MirrorMerge { axis }, &square());
        assert!((both.bounding_box().width() - 200.0).abs() < 1e-6 && (area(&both) - 20_000.0).abs() < 1.0);

        let fine = apply(&Deform::Subdivide { levels: 2 }, &square());
        assert_eq!(bezier::point_count(&fine), 16);
        let circle = apply(&Deform::Subdivide { levels: 3 }, &kurbo::Circle::new((0.0, 0.0), 50.0).to_path(0.1));
        let back = apply(&Deform::Simplify { tolerance: 0.5 }, &circle);
        assert!(bezier::point_count(&back) < bezier::point_count(&circle) / 2, "{}", bezier::point_count(&back));
    }

    #[test]
    fn copies_deform_in_the_space_their_modifier_saw() {
        let d = Deform::Twist { degrees: 90.0, centre: Point::new(50.0, 50.0) };
        // A copy moved 200 to the right, twisted around the layer's (50, 50):
        // it's at the edge of nothing, so it turns as a whole far from it.
        let copy = xenify_doc::CopyPlan {
            transform: Affine::translate((200.0, 0.0)),
            opacity: 1.0,
            deforms: vec![(Affine::translate((200.0, 0.0)), d)],
            clip: None,
        };
        let p = deform_copy(&copy, &square(), Affine::IDENTITY);
        let q = apply(&Deform::Twist { degrees: 90.0, centre: Point::new(-150.0, 50.0) }, &square());
        let (a, b) = (bezier::anchors(&p), bezier::anchors(&q));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| (x.pos - y.pos).hypot() < 1e-6));
    }
}
