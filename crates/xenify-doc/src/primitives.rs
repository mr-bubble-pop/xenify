//! Rectangles (with any corner rounding) and ellipses as bezier outlines, and
//! recognising them again in any path: every shape can be edited as points,
//! and a path that is still a rectangle keeps its size and corner controls.

use crate::layer::Geometry;
use crate::property::{Keyframe, Property};
use kurbo::{BezPath, CubicBez, ParamCurve, PathSeg, Point, Rect, Shape, Vec2};

/// How far along a quarter circle's tangents its bezier handles reach.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

/// A shape recognised in (or made into) a path, in the path's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    /// Corner radii: top-left, top-right, bottom-right, bottom-left (y down).
    Rect {
        rect: Rect,
        radii: [f64; 4],
    },
    Ellipse {
        rect: Rect,
    },
}

impl Primitive {
    pub fn rect(&self) -> Rect {
        match *self {
            Primitive::Rect { rect, .. } | Primitive::Ellipse { rect } => rect,
        }
    }

    /// The same kind of shape in `rect` (radii kept, as far as they fit).
    pub fn with_rect(self, rect: Rect) -> Self {
        match self {
            Primitive::Rect { radii, .. } => Primitive::Rect { rect, radii: clamp_radii(rect, radii) },
            Primitive::Ellipse { .. } => Primitive::Ellipse { rect },
        }
    }

    pub fn to_path(&self) -> BezPath {
        match *self {
            Primitive::Rect { rect, radii } => rect_path(rect, radii),
            Primitive::Ellipse { rect } => ellipse_path(rect),
        }
    }
}

/// Shrinks corner radii that don't fit (as CSS does): no radius below zero,
/// and the two on any edge together at most as long as the edge.
pub fn clamp_radii(rect: Rect, radii: [f64; 4]) -> [f64; 4] {
    let r = radii.map(|r| if r.is_finite() { r.max(0.0) } else { 0.0 });
    let (w, h) = (rect.width().abs(), rect.height().abs());
    let mut f: f64 = 1.0;
    for (len, a, b) in [(w, r[0], r[1]), (h, r[1], r[2]), (w, r[2], r[3]), (h, r[3], r[0])] {
        if a + b > len {
            f = f.min(len / (a + b));
        }
    }
    r.map(|r| r * f)
}

/// A rectangle with each corner rounded by its own radius (top-left,
/// top-right, bottom-right, bottom-left), clockwise from the top-left corner.
/// One point per sharp corner, two (joined by a quarter circle) per rounded one.
pub fn rect_path(rect: Rect, radii: [f64; 4]) -> BezPath {
    let rect = rect.abs();
    let [tl, tr, br, bl] = clamp_radii(rect, radii);
    let (x0, y0, x1, y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    let mut p = BezPath::new();
    let start = Point::new(x0 + tl, y0);
    p.move_to(start);
    let mut at = start;
    let line = |p: &mut BezPath, at: &mut Point, to: Point| {
        if (to - *at).hypot() > 1e-9 && (to - start).hypot() > 1e-9 {
            p.line_to(to);
        }
        *at = to;
    };
    // Quarter circle from `a` to `b` around corner `c`.
    let arc = |p: &mut BezPath, at: &mut Point, a: Point, c: Point, b: Point| {
        p.curve_to(a + (c - a) * KAPPA, b + (c - b) * KAPPA, b);
        *at = b;
    };
    line(&mut p, &mut at, Point::new(x1 - tr, y0));
    if tr > 0.0 {
        arc(&mut p, &mut at, Point::new(x1 - tr, y0), Point::new(x1, y0), Point::new(x1, y0 + tr));
    }
    line(&mut p, &mut at, Point::new(x1, y1 - br));
    if br > 0.0 {
        arc(&mut p, &mut at, Point::new(x1, y1 - br), Point::new(x1, y1), Point::new(x1 - br, y1));
    }
    line(&mut p, &mut at, Point::new(x0 + bl, y1));
    if bl > 0.0 {
        arc(&mut p, &mut at, Point::new(x0 + bl, y1), Point::new(x0, y1), Point::new(x0, y1 - bl));
    }
    line(&mut p, &mut at, Point::new(x0, y0 + tl));
    if tl > 0.0 {
        arc(&mut p, &mut at, Point::new(x0, y0 + tl), Point::new(x0, y0), start);
    }
    p.close_path();
    p
}

/// An ellipse as four quarter curves, clockwise from the top.
pub fn ellipse_path(rect: Rect) -> BezPath {
    let rect = rect.abs();
    let c = rect.center();
    let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
    let pts =
        [Point::new(c.x, c.y - ry), Point::new(c.x + rx, c.y), Point::new(c.x, c.y + ry), Point::new(c.x - rx, c.y)];
    let mut p = BezPath::new();
    p.move_to(pts[0]);
    for i in 0..4 {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        // The corner of the bounding box between the two quarter points.
        let corner = Point::new(if a.x == c.x { b.x } else { a.x }, if a.y == c.y { b.y } else { a.y });
        p.curve_to(a + (corner - a) * KAPPA, b + (corner - b) * KAPPA, b);
    }
    p.close_path();
    p
}

/// The outline of a rectangle or ellipse geometry at `time` (the same
/// points [`recognize`] finds again); `None` for paths.
pub fn live_path(geometry: &Geometry, time: f64) -> Option<BezPath> {
    match geometry {
        Geometry::Rect { position, size, corner_radius } => {
            let rect = Rect::from_center_size(position.value_at(time).to_point(), size.value_at(time).to_size());
            let r = corner_radius.value_at(time);
            Some(rect_path(rect, [r; 4]))
        }
        Geometry::Ellipse { position, size } => Some(ellipse_path(Rect::from_center_size(
            position.value_at(time).to_point(),
            size.value_at(time).to_size(),
        ))),
        Geometry::Path { .. } => None,
        Geometry::Figure { position, size, kind, count, inner, hole } => Some(figure_outline(
            *kind,
            count.value_at(time),
            inner.value_at(time),
            hole.value_at(time),
            Rect::from_center_size(position.value_at(time).to_point(), size.value_at(time).to_size()),
        )),
    }
}

/// A rectangle or ellipse geometry as a path geometry, animation included:
/// a path keyframe wherever any of its values had one.
pub fn to_path_geometry(geometry: &Geometry) -> Geometry {
    let keys: Vec<(f64, crate::Easing)> = match geometry {
        Geometry::Rect { position, size, corner_radius } => {
            let mut k: Vec<(f64, crate::Easing)> = position.keyframes().iter().map(|k| (k.time, k.easing)).collect();
            k.extend(size.keyframes().iter().map(|k| (k.time, k.easing)));
            k.extend(corner_radius.keyframes().iter().map(|k| (k.time, k.easing)));
            k
        }
        Geometry::Ellipse { position, size } => {
            let mut k: Vec<(f64, crate::Easing)> = position.keyframes().iter().map(|k| (k.time, k.easing)).collect();
            k.extend(size.keyframes().iter().map(|k| (k.time, k.easing)));
            k
        }
        Geometry::Path { .. } => return geometry.clone(),
        Geometry::Figure { position, size, count, inner, hole, .. } => {
            let mut k: Vec<(f64, crate::Easing)> = position.keyframes().iter().map(|k| (k.time, k.easing)).collect();
            k.extend(size.keyframes().iter().map(|k| (k.time, k.easing)));
            for p in [count, inner, hole] {
                k.extend(p.keyframes().iter().map(|k| (k.time, k.easing)));
            }
            k
        }
    };
    let mut keys = keys;
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    keys.dedup_by(|b, a| (a.0 - b.0).abs() < crate::TIME_EPSILON);
    let path = if keys.is_empty() {
        Property::Static(live_path(geometry, 0.0).unwrap_or_default())
    } else {
        Property::Animated(
            keys.iter()
                .map(|(time, easing)| Keyframe {
                    time: *time,
                    value: live_path(geometry, *time).unwrap_or_default(),
                    easing: *easing,
                })
                .collect(),
        )
    };
    Geometry::Path { path }
}

/// What `path` is, if it's a single closed outline that is an axis-aligned
/// rectangle (sharp or with circular corners, each its own radius) or an
/// ellipse. Works for any start point and direction, lines split in pieces
/// and corners drawn as several curves.
pub fn recognize(path: &BezPath) -> Option<Primitive> {
    let moves = path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
    // Closed, or ending where it started (kurbo's ellipses do).
    let first = match path.elements().first() {
        Some(kurbo::PathEl::MoveTo(p)) => *p,
        _ => return None,
    };
    let closed = match path.elements().last() {
        Some(kurbo::PathEl::ClosePath) => true,
        Some(e) => e.end_point().is_some_and(|p| (p - first).hypot() < 1e-6 * (1.0 + first.to_vec2().hypot())),
        None => false,
    };
    if moves != 1 || !closed {
        return None;
    }
    let bb = path.bounding_box();
    let size = bb.width().max(bb.height());
    if bb.width() < 1e-6 || bb.height() < 1e-6 || !size.is_finite() {
        return None;
    }
    let eps = 1e-4 * size.max(1.0);
    let segs: Vec<PathSeg> = path.segments().filter(|s| seg_len(s) > eps).collect();
    if segs.len() < 2 {
        return None;
    }
    if let Some(e) = as_ellipse(&segs, bb) {
        return Some(e);
    }
    as_rect(path, &segs, bb, eps)
}

fn seg_len(s: &PathSeg) -> f64 {
    let (a, b) = (s.start(), s.end());
    match s {
        PathSeg::Line(_) => (b - a).hypot(),
        PathSeg::Quad(q) => (q.p1 - a).hypot() + (b - q.p1).hypot(),
        PathSeg::Cubic(c) => (c.p1 - a).hypot() + (c.p2 - c.p1).hypot() + (b - c.p2).hypot(),
    }
}

fn samples(s: &PathSeg) -> impl Iterator<Item = Point> + '_ {
    [0.0, 0.25, 0.5, 0.75, 1.0].into_iter().map(move |t| s.eval(t))
}

fn as_ellipse(segs: &[PathSeg], bb: Rect) -> Option<Primitive> {
    if segs.len() < 4 || segs.iter().any(|s| !matches!(s, PathSeg::Cubic(_))) {
        return None;
    }
    let c = bb.center();
    let (rx, ry) = (bb.width() / 2.0, bb.height() / 2.0);
    let on = |p: Point| {
        let d = ((p.x - c.x) / rx).powi(2) + ((p.y - c.y) / ry).powi(2);
        (d.sqrt() - 1.0).abs() < 2e-3
    };
    if !segs.iter().all(|s| samples(s).all(on)) {
        return None;
    }
    // Once around, not back and forth.
    let mut turn = 0.0;
    for s in segs {
        let (a, b) = (s.start() - c, s.end() - c);
        let mut d = b.atan2() - a.atan2();
        while d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        }
        while d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        turn += d;
    }
    ((turn.abs() - std::f64::consts::TAU).abs() < 0.05).then_some(Primitive::Ellipse { rect: bb })
}

/// Which edges of `bb` point `p` is on: (top/bottom, left/right).
fn edges(p: Point, bb: Rect, eps: f64) -> (Option<usize>, Option<usize>) {
    let h = if (p.y - bb.y0).abs() < eps {
        Some(0)
    } else if (p.y - bb.y1).abs() < eps {
        Some(1)
    } else {
        None
    };
    let v = if (p.x - bb.x0).abs() < eps {
        Some(0)
    } else if (p.x - bb.x1).abs() < eps {
        Some(1)
    } else {
        None
    };
    (h, v)
}

/// Corner index (top-left, top-right, bottom-right, bottom-left) from the
/// horizontal edge (0 top, 1 bottom) and vertical edge (0 left, 1 right).
fn corner_index(h: usize, v: usize) -> usize {
    match (h, v) {
        (0, 0) => 0,
        (0, 1) => 1,
        (1, 1) => 2,
        _ => 3,
    }
}

fn as_rect(path: &BezPath, segs: &[PathSeg], bb: Rect, eps: f64) -> Option<Primitive> {
    let mut radii = [0.0; 4];
    let mut rounded = [false; 4];
    // A corner may be drawn as several curves in a row.
    let mut run: Vec<CubicBez> = Vec::new();
    let on_edge_line = |a: Point, b: Point| {
        let (ha, va) = edges(a, bb, eps);
        let (hb, vb) = edges(b, bb, eps);
        (ha.is_some() && ha == hb) || (va.is_some() && va == vb)
    };
    // Start where a line or a finished corner begins, so no corner is split
    // across the end of the list.
    let start = segs.iter().position(|s| {
        let (h, v) = edges(s.start(), bb, eps);
        h.is_some() && v.is_some() || matches!(s, PathSeg::Line(_))
    })?;
    for k in 0..segs.len() {
        let s = segs[(start + k) % segs.len()];
        let cubic = match s {
            PathSeg::Line(l) => {
                if !run.is_empty() || !on_edge_line(l.p0, l.p1) {
                    return None;
                }
                continue;
            }
            PathSeg::Quad(q) => q.raise(),
            PathSeg::Cubic(c) => c,
        };
        // A "curve" lying flat along an edge is a line.
        if run.is_empty()
            && on_edge_line(cubic.p0, cubic.p3)
            && [cubic.p1, cubic.p2].iter().all(|p| on_edge_line(cubic.p0, *p))
        {
            continue;
        }
        run.push(cubic);
        let (a, b) = (run[0].p0, cubic.p3);
        let (ha, va) = edges(a, bb, eps);
        let (hb, vb) = edges(b, bb, eps);
        // The run ends when it reaches the other edge of its corner.
        let (h, v, on_h, on_v) = match (ha, va, hb, vb) {
            (Some(h), None, None, Some(v)) => (h, v, a, b),
            (None, Some(v), Some(h), None) => (h, v, b, a),
            _ => continue,
        };
        let corner = Point::new(if v == 0 { bb.x0 } else { bb.x1 }, if h == 0 { bb.y0 } else { bb.y1 });
        let (rx, ry) = ((on_h.x - corner.x).abs(), (on_v.y - corner.y).abs());
        let r = (rx + ry) / 2.0;
        if (rx - ry).abs() > eps.max(0.02 * r) {
            return None;
        }
        let inward = Vec2::new(if v == 0 { r } else { -r }, if h == 0 { r } else { -r });
        let centre = corner + inward;
        let tol = eps.max(0.01 * r);
        if !run.iter().all(|c| samples(&PathSeg::Cubic(*c)).all(|p| ((p - centre).hypot() - r).abs() < tol)) {
            return None;
        }
        let i = corner_index(h, v);
        if rounded[i] {
            return None;
        }
        rounded[i] = true;
        radii[i] = r;
        run.clear();
    }
    if !run.is_empty() {
        return None;
    }
    // Every edge walked once, and nothing doubling back: the area matches.
    let expected = bb.area() - radii.iter().map(|r| (1.0 - std::f64::consts::FRAC_PI_4) * r * r).sum::<f64>();
    let area = path.area().abs();
    ((area - expected).abs() <= 1e-3 * expected.max(1.0))
        .then_some(Primitive::Rect { rect: bb, radii: clamp_radii(bb, radii) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn rect_paths_round_trip() {
        let rect = Rect::new(-50.0, -30.0, 70.0, 40.0);
        for radii in [[0.0; 4], [10.0; 4], [0.0, 20.0, 5.0, 0.0], [35.0, 35.0, 0.0, 0.0]] {
            let p = rect_path(rect, radii);
            match recognize(&p) {
                Some(Primitive::Rect { rect: r, radii: got }) => {
                    assert!(close(r.x0, rect.x0) && close(r.y1, rect.y1), "{r:?}");
                    for (a, b) in got.iter().zip(radii) {
                        assert!((a - b).abs() < 1e-3, "{got:?} vs {radii:?}");
                    }
                }
                other => panic!("{radii:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn sharp_rect_has_one_point_per_corner() {
        let p = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
        assert_eq!(crate::bezier::point_count(&p), 4);
        let p = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0), [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(crate::bezier::point_count(&p), 5);
    }

    #[test]
    fn kurbo_and_reversed_rects_are_recognised() {
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        // kurbo draws rounded corners as arcs (maybe several curves each).
        let k = rect.to_rounded_rect(30.0).to_path(0.01);
        assert!(matches!(recognize(&k), Some(Primitive::Rect { radii: [r, ..], .. }) if (r - 30.0).abs() < 0.5));
        // Counter-clockwise, starting mid-edge, with a split edge.
        let mut p = BezPath::new();
        p.move_to((100.0, 0.0));
        p.line_to((0.0, 0.0));
        p.line_to((0.0, 100.0));
        p.line_to((120.0, 100.0));
        p.line_to((200.0, 100.0));
        p.line_to((200.0, 0.0));
        p.close_path();
        assert!(matches!(recognize(&p), Some(Primitive::Rect { radii: [0.0, 0.0, 0.0, 0.0], .. })));
    }

    #[test]
    fn ellipses_are_recognised() {
        let r = Rect::new(0.0, 0.0, 80.0, 40.0);
        assert_eq!(recognize(&ellipse_path(r)), Some(Primitive::Ellipse { rect: r }));
        let k = kurbo::Ellipse::from_rect(r).to_path(0.01);
        assert!(matches!(recognize(&k), Some(Primitive::Ellipse { .. })));
    }

    #[test]
    fn other_shapes_are_not() {
        let mut tri = BezPath::new();
        tri.move_to((0.0, 0.0));
        tri.line_to((10.0, 0.0));
        tri.line_to((0.0, 10.0));
        tri.close_path();
        assert_eq!(recognize(&tri), None);
        // A rectangle with one point dragged off its corner.
        let p =
            crate::bezier::move_anchor(&rect_path(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]), 1, (14.0, 2.0).into());
        assert_eq!(recognize(&p), None);
        // Open.
        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 0.0));
        open.line_to((10.0, 10.0));
        open.line_to((0.0, 10.0));
        assert_eq!(recognize(&open), None);
    }

    #[test]
    fn radii_that_dont_fit_shrink() {
        let r = clamp_radii(Rect::new(0.0, 0.0, 10.0, 100.0), [10.0, 10.0, 0.0, 0.0]);
        assert!(close(r[0], 5.0) && close(r[1], 5.0));
    }

    #[test]
    fn animated_rects_bake_to_path_keys() {
        let mut size = Property::Static(Vec2::new(10.0, 10.0));
        size.set_keyframe(0.0, Vec2::new(10.0, 10.0), None);
        size.set_keyframe(1.0, Vec2::new(40.0, 10.0), None);
        let g = Geometry::Rect { position: Vec2::ZERO.into(), size, corner_radius: 0.0.into() };
        let Geometry::Path { path } = to_path_geometry(&g) else { panic!() };
        assert_eq!(path.keyframes().len(), 2);
        assert!(close(path.value_at(1.0).bounding_box().width(), 40.0));
    }
}

// ---------------------------------------------------------------------------
// Polygons, stars and gears

/// A regular polygon (or star when `inner` is set: inner radius as a fraction
/// of the outer one) with a vertex pointing up, stretched to fill `bounds`.
pub fn polygon_path(sides: usize, inner: Option<f64>, bounds: Rect) -> BezPath {
    let sides = sides.max(3);
    let n = if inner.is_some() { sides * 2 } else { sides };
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / n as f64;
            let r = if inner.is_some() && i % 2 == 1 { inner.unwrap_or(1.0).clamp(0.05, 1.0) } else { 1.0 };
            Point::new(a.cos() * r, a.sin() * r)
        })
        .collect();
    // Fit the unit shape's own box to `bounds` (a triangle fills the box, not a circle's worth).
    let unit = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
    let map = |p: Point| {
        Point::new(
            bounds.x0 + (p.x - unit.x0) / unit.width().max(1e-9) * bounds.width(),
            bounds.y0 + (p.y - unit.y0) / unit.height().max(1e-9) * bounds.height(),
        )
    };
    closed_polyline(&pts.into_iter().map(map).collect::<Vec<_>>())
}

/// Appends a circular arc (one curve, for arcs up to a quarter turn) from
/// the current point at `a0` to `a1` around `c`.
fn arc_to(p: &mut BezPath, c: Point, r: f64, a0: f64, a1: f64) {
    let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan() * r;
    let at = |a: f64| c + Vec2::new(a.cos(), a.sin()) * r;
    let tangent = |a: f64| Vec2::new(-a.sin(), a.cos());
    p.curve_to(at(a0) + tangent(a0) * k, at(a1) - tangent(a1) * k, at(a1));
}
/// A gear: `teeth` flat-topped teeth `depth` deep (fraction of the radius)
/// around a circle, with a round hole of `hole` × the radius in the middle.
pub fn gear_path(teeth: usize, depth: f64, hole: f64, bounds: Rect) -> BezPath {
    let n = teeth.clamp(3, 200);
    let depth = depth.clamp(0.02, 0.6);
    let (outer, root) = (1.0, 1.0 - depth);
    let step = std::f64::consts::TAU / n as f64;
    let c = Point::ZERO;
    let at = |r: f64, a: f64| Point::new(a.cos() * r, a.sin() * r);
    let start = -std::f64::consts::FRAC_PI_2 - step * 0.2;
    let mut p = BezPath::new();
    for i in 0..n {
        let a = start + i as f64 * step;
        // Tip from 0.0 to 0.3 of the step, flanks 0.1 wide, root the rest.
        let (tip0, tip1, down, next) = (a, a + step * 0.3, a + step * 0.4, a + step * 0.9);
        if i == 0 {
            p.move_to(at(outer, tip0));
        } else {
            p.line_to(at(outer, tip0));
        }
        arc_to(&mut p, c, outer, tip0, tip1);
        p.line_to(at(root, down));
        arc_to(&mut p, c, root, down, next);
    }
    p.close_path();
    let hole = hole.clamp(0.0, root - 0.05);
    if hole > 0.01 {
        // The other way round, so non-zero filling leaves it empty.
        let q = std::f64::consts::FRAC_PI_2;
        p.move_to(at(hole, 0.0));
        for k in 0..4 {
            let a = -(k as f64) * q;
            arc_to(&mut p, c, hole, a, a - q);
        }
        p.close_path();
    }
    let unit = Rect::new(-1.0, -1.0, 1.0, 1.0);
    let (sx, sy) = (bounds.width() / unit.width(), bounds.height() / unit.height());
    kurbo::Affine::new([sx, 0.0, 0.0, sy, bounds.x0 + sx, bounds.y0 + sy]) * p
}

pub fn closed_polyline(pts: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            path.move_to(*p);
        } else {
            path.line_to(*p);
        }
    }
    path.close_path();
    path
}

/// What a live figure geometry draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureKind {
    #[default]
    Polygon,
    Star,
    Gear,
}

impl FigureKind {
    pub fn label(self) -> &'static str {
        match self {
            FigureKind::Polygon => "Polygon",
            FigureKind::Star => "Star",
            FigureKind::Gear => "Gear",
        }
    }
}

/// A live figure's outline: `count` sides, points or teeth; `inner` the
/// star's inner radius or the gear's tooth depth (fractions of the radius);
/// `hole` the gear's hole; fitted to `bounds`.
pub fn figure_outline(kind: FigureKind, count: f64, inner: f64, hole: f64, bounds: Rect) -> BezPath {
    let n = count.round().max(3.0) as usize;
    match kind {
        FigureKind::Polygon => polygon_path(n, None, bounds),
        FigureKind::Star => polygon_path(n, Some(inner), bounds),
        FigureKind::Gear => gear_path(n, inner, hole, bounds),
    }
}

#[cfg(test)]
mod figure_tests {
    use super::*;
    use crate::{Geometry, Keyframe, Layer, PropPath, ShapeField, ShapeItem};

    #[test]
    fn live_stars_keep_their_points_editable() {
        let star = Geometry::Figure {
            position: Vec2::ZERO.into(),
            size: Vec2::new(100.0, 100.0).into(),
            kind: FigureKind::Star,
            count: Property::from_keyframes(vec![
                Keyframe { time: 0.0, value: 5.0, easing: Default::default() },
                Keyframe { time: 1.0, value: 8.0, easing: Default::default() },
            ])
            .unwrap(),
            inner: 0.4.into(),
            hole: 0.0.into(),
        };
        let corners = |t: f64| live_path(&star, t).unwrap().elements().len();
        assert_eq!(corners(0.0), 10 + 1, "5 points: 10 corners (a move and 9 lines), then close");
        assert_eq!(corners(1.0), 16 + 1, "8 points by the end");
        let mut shape = ShapeItem::rect(Vec2::new(1.0, 1.0), crate::Color::WHITE);
        shape.geometry = star.clone();
        let layer = Layer::shape(crate::Id(1), "Star", vec![shape]);
        let paths = layer.property_paths();
        assert!(paths.contains(&PropPath::shape(0, ShapeField::Count)));
        assert!(paths.contains(&PropPath::shape(0, ShapeField::Inner)));
        assert!(!paths.contains(&PropPath::shape(0, ShapeField::Hole)), "only gears have a hole");
        // As a path (for editing points), keyed where it changed.
        let Geometry::Path { path } = to_path_geometry(&star) else { panic!() };
        assert_eq!(path.keyframes().len(), 2);
    }
}
