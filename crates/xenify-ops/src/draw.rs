//! Geometry for the drawing tools: regular polygons and stars, freehand
//! strokes (smoothed, or recognised as clean shapes) and the sculpt brush.

use kurbo::{BezPath, Point, Rect, Vec2};

pub use xenify_doc::primitives::{closed_polyline, gear_path, polygon_path};

/// What the polygon tool draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Figure {
    #[default]
    Polygon,
    Star,
    Gear,
    Heart,
    Arrow,
    /// A speech bubble: a rounded box with a tail.
    Bubble,
}

impl Figure {
    pub const ALL: [Figure; 6] =
        [Figure::Polygon, Figure::Star, Figure::Gear, Figure::Heart, Figure::Arrow, Figure::Bubble];

    pub fn name(self) -> &'static str {
        match self {
            Figure::Polygon => "Polygon",
            Figure::Star => "Star",
            Figure::Gear => "Gear",
            Figure::Heart => "Heart",
            Figure::Arrow => "Arrow",
            Figure::Bubble => "Speech Bubble",
        }
    }
}

/// The polygon tool's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureParams {
    pub sides: usize,
    /// Star inner radius, as a fraction of the outer one.
    pub star_inner: f64,
    pub teeth: usize,
    /// How deep gear teeth are, as a fraction of the radius.
    pub tooth_depth: f64,
    /// The gear's hole, as a fraction of the radius (0 = none).
    pub hole: f64,
}

impl Default for FigureParams {
    fn default() -> Self {
        Self { sides: 5, star_inner: 0.5, teeth: 12, tooth_depth: 0.2, hole: 0.3 }
    }
}

/// `figure` filling `bounds`.
pub fn figure_path(figure: Figure, params: &FigureParams, bounds: Rect) -> BezPath {
    match figure {
        Figure::Polygon => polygon_path(params.sides, None, bounds),
        Figure::Star => polygon_path(params.sides.max(3), Some(params.star_inner), bounds),
        Figure::Gear => gear_path(params.teeth, params.tooth_depth, params.hole, bounds),
        Figure::Heart => fit(heart_unit(), bounds),
        Figure::Arrow => fit(
            closed_polyline(&[
                Point::new(0.0, 0.3),
                Point::new(0.6, 0.3),
                Point::new(0.6, 0.0),
                Point::new(1.0, 0.5),
                Point::new(0.6, 1.0),
                Point::new(0.6, 0.7),
                Point::new(0.0, 0.7),
            ]),
            bounds,
        ),
        Figure::Bubble => bubble_path(bounds),
    }
}

/// `figure` as a live geometry `size` across, centred on the layer origin,
/// for the kinds that stay editable (polygons, stars, gears).
pub fn live_figure(figure: Figure, params: &FigureParams, size: kurbo::Size) -> Option<xenify_doc::Geometry> {
    use xenify_doc::primitives::FigureKind;
    let (kind, count, inner, hole) = match figure {
        Figure::Polygon => (FigureKind::Polygon, params.sides, 0.5, 0.0),
        Figure::Star => (FigureKind::Star, params.sides.max(3), params.star_inner, 0.0),
        Figure::Gear => (FigureKind::Gear, params.teeth, params.tooth_depth, params.hole),
        _ => return None,
    };
    Some(xenify_doc::Geometry::Figure {
        position: xenify_doc::Vec2::ZERO.into(),
        size: xenify_doc::Vec2::new(size.width, size.height).into(),
        kind,
        count: (count as f64).into(),
        inner: inner.into(),
        hole: hole.into(),
    })
}

/// Stretches `path` so its bounding box fills `bounds`.
fn fit(path: BezPath, bounds: Rect) -> BezPath {
    use kurbo::Shape;
    let unit = path.bounding_box();
    let sx = bounds.width() / unit.width().max(1e-9);
    let sy = bounds.height() / unit.height().max(1e-9);
    kurbo::Affine::new([sx, 0.0, 0.0, sy, bounds.x0 - unit.x0 * sx, bounds.y0 - unit.y0 * sy]) * path
}

fn heart_unit() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.5, 0.28));
    p.curve_to((0.5, 0.12), (0.37, 0.0), (0.24, 0.0));
    p.curve_to((0.1, 0.0), (0.0, 0.11), (0.0, 0.28));
    p.curve_to((0.0, 0.56), (0.3, 0.74), (0.5, 1.0));
    p.curve_to((0.7, 0.74), (1.0, 0.56), (1.0, 0.28));
    p.curve_to((1.0, 0.11), (0.9, 0.0), (0.76, 0.0));
    p.curve_to((0.63, 0.0), (0.5, 0.12), (0.5, 0.28));
    p.close_path();
    p
}

/// Appends a circular arc (one curve, for arcs up to a quarter turn) from
/// the current point at `a0` to `a1` around `c`.
fn arc_to(p: &mut BezPath, c: Point, r: f64, a0: f64, a1: f64) {
    let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan() * r;
    let at = |a: f64| c + Vec2::new(a.cos(), a.sin()) * r;
    let tangent = |a: f64| Vec2::new(-a.sin(), a.cos());
    p.curve_to(at(a0) + tangent(a0) * k, at(a1) - tangent(a1) * k, at(a1));
}

/// A rounded box over the top four fifths of `bounds`, with a tail down to
/// the bottom-left.
fn bubble_path(bounds: Rect) -> BezPath {
    let b = bounds.abs();
    let (w, h) = (b.width(), b.height());
    let body = b.y0 + h * 0.78;
    let r = 0.15 * w.min(body - b.y0);
    let mut p = BezPath::new();
    let q = std::f64::consts::FRAC_PI_2;
    p.move_to((b.x0 + r, b.y0));
    p.line_to((b.x1 - r, b.y0));
    arc_to(&mut p, Point::new(b.x1 - r, b.y0 + r), r, -q, 0.0);
    p.line_to((b.x1, body - r));
    arc_to(&mut p, Point::new(b.x1 - r, body - r), r, 0.0, q);
    p.line_to((b.x0 + 0.38 * w, body));
    p.line_to((b.x0 + 0.15 * w, b.y1));
    p.line_to((b.x0 + 0.22 * w, body));
    p.line_to((b.x0 + r, body));
    arc_to(&mut p, Point::new(b.x0 + r, body - r), r, q, 2.0 * q);
    p.line_to((b.x0, b.y0 + r));
    arc_to(&mut p, Point::new(b.x0 + r, b.y0 + r), r, 2.0 * q, 3.0 * q);
    p.close_path();
    p
}

/// What a freehand stroke was recognised as (composition space).
#[derive(Clone, Debug, PartialEq)]
pub enum Recognized {
    Line(Point, Point),
    /// `angle` in radians.
    Ellipse {
        center: Point,
        radii: Vec2,
        angle: f64,
    },
    Rect {
        center: Point,
        size: Vec2,
        angle: f64,
    },
    /// Triangle, pentagon, …: the corner points.
    Polygon(Vec<Point>),
    /// Nothing clean: the smoothed stroke.
    Free {
        path: BezPath,
        closed: bool,
    },
}

/// Drops points closer than `min` to the previous one and lightly smooths jitter.
fn clean(points: &[Point], min: f64) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_none_or(|q: &Point| q.distance(*p) >= min) {
            out.push(*p);
        }
    }
    if out.len() < 5 {
        return out;
    }
    let mut smooth = out.clone();
    for i in 1..out.len() - 1 {
        smooth[i] = Point::new(
            (out[i - 1].x + 2.0 * out[i].x + out[i + 1].x) / 4.0,
            (out[i - 1].y + 2.0 * out[i].y + out[i + 1].y) / 4.0,
        );
    }
    smooth
}

fn length(pts: &[Point]) -> f64 {
    pts.windows(2).map(|w| w[0].distance(w[1])).sum()
}

fn bbox(pts: &[Point]) -> Rect {
    pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

/// A smooth Bézier path through a freehand stroke.
pub fn smooth_stroke(points: &[Point], closed: bool) -> BezPath {
    let pts = clean(points, 1.5);
    let mut path = BezPath::new();
    let Some(first) = pts.first() else { return path };
    path.move_to(*first);
    for p in &pts[1..] {
        path.line_to(*p);
    }
    if closed {
        path.close_path();
    }
    let size = if pts.len() > 1 { bbox(&pts).size() } else { kurbo::Size::ZERO };
    let accuracy = (size.width.max(size.height) * 0.004).clamp(0.3, 2.5);
    xenify_doc::bezier::simplify(&path, accuracy)
}

/// Recognises a freehand stroke as a line, ellipse, rectangle or polygon when
/// it's close enough to one; otherwise returns the smoothed stroke.
pub fn recognize(points: &[Point]) -> Recognized {
    let pts = clean(points, 1.0);
    if pts.len() < 3 {
        let (a, b) = (points.first().copied().unwrap_or_default(), points.last().copied().unwrap_or_default());
        return Recognized::Line(a, b);
    }
    let len = length(&pts);
    let b = bbox(&pts);
    let diag = b.width().hypot(b.height()).max(1e-9);
    let (first, last) = (pts[0], *pts.last().unwrap());
    let closed = first.distance(last) < 0.2 * diag && len > 1.8 * diag;
    if !closed {
        let chord = kurbo::Line::new(first, last);
        let dev = pts.iter().map(|p| dist_to_segment(*p, chord)).fold(0.0, f64::max);
        let straight = first.distance(last);
        if dev < 0.06 * straight.max(1.0) && len < 1.2 * straight {
            return Recognized::Line(first, last);
        }
        return Recognized::Free { path: smooth_stroke(points, false), closed: false };
    }
    let ring = resample_closed(&pts, 96);
    let (ellipse, e_err) = fit_ellipse(&ring);
    let corners = polygon_corners(&ring, 0.07 * diag);
    let p_err = if corners.len() >= 3 {
        ring.iter().map(|p| dist_to_polygon(*p, &corners)).sum::<f64>() / ring.len() as f64
    } else {
        f64::INFINITY
    };
    // Straight sides fit much better than an ellipse when there are corners.
    let best = e_err.min(p_err);
    if best > 0.05 * diag {
        return Recognized::Free { path: smooth_stroke(points, true), closed: true };
    }
    if p_err < e_err * 0.8 && corners.len() <= 8 {
        if corners.len() == 4
            && let Some(r) = as_rect(&corners)
        {
            return r;
        }
        return Recognized::Polygon(corners);
    }
    ellipse
}

fn dist_to_segment(p: Point, l: kurbo::Line) -> f64 {
    use kurbo::ParamCurveNearest;
    l.nearest(p, 1e-6).distance_sq.sqrt()
}

fn dist_to_polygon(p: Point, poly: &[Point]) -> f64 {
    (0..poly.len())
        .map(|i| dist_to_segment(p, kurbo::Line::new(poly[i], poly[(i + 1) % poly.len()])))
        .fold(f64::INFINITY, f64::min)
}

/// `n` points evenly spaced along the closed polyline.
fn resample_closed(pts: &[Point], n: usize) -> Vec<Point> {
    let mut ring = pts.to_vec();
    ring.push(pts[0]);
    let total = length(&ring);
    let step = total / n as f64;
    let mut out = Vec::with_capacity(n);
    let (mut seg, mut acc) = (0usize, 0.0);
    for i in 0..n {
        let target = i as f64 * step;
        while seg + 1 < ring.len() - 1 && acc + ring[seg].distance(ring[seg + 1]) < target {
            acc += ring[seg].distance(ring[seg + 1]);
            seg += 1;
        }
        let l = ring[seg].distance(ring[seg + 1]).max(1e-9);
        out.push(ring[seg].lerp(ring[seg + 1], ((target - acc) / l).clamp(0.0, 1.0)));
    }
    out
}

/// Principal-axes ellipse through evenly spaced points, and its mean error in pixels.
fn fit_ellipse(ring: &[Point]) -> (Recognized, f64) {
    let n = ring.len() as f64;
    let c = ring.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / n;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for p in ring {
        let d = p.to_vec2() - c;
        sxx += d.x * d.x;
        syy += d.y * d.y;
        sxy += d.x * d.y;
    }
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (cos, sin) = (angle.cos(), angle.sin());
    // Radii from the extents along the axes (robust for uneven drawing speed).
    let (mut a, mut b) = (0.0f64, 0.0f64);
    for p in ring {
        let d = p.to_vec2() - c;
        a = a.max((d.x * cos + d.y * sin).abs());
        b = b.max((-d.x * sin + d.y * cos).abs());
    }
    let (a, b) = (a.max(1e-9), b.max(1e-9));
    let err = ring
        .iter()
        .map(|p| {
            let d = p.to_vec2() - c;
            let (u, v) = (d.x * cos + d.y * sin, -d.x * sin + d.y * cos);
            let r = ((u / a).powi(2) + (v / b).powi(2)).sqrt();
            (r - 1.0).abs() * (a + b) / 2.0
        })
        .sum::<f64>()
        / n;
    let mut radii = Vec2::new(a, b);
    let mut angle = angle;
    // Nearly round → circle; nearly axis-aligned → straight.
    if (a - b).abs() < 0.12 * a.max(b) {
        let r = (a + b) / 2.0;
        radii = Vec2::new(r, r);
        angle = 0.0;
    }
    angle = snap_to_axis(angle);
    (Recognized::Ellipse { center: c.to_point(), radii, angle }, err)
}

/// Rounds angles within 8° of horizontal/vertical to exactly that.
fn snap_to_axis(angle: f64) -> f64 {
    let q = std::f64::consts::FRAC_PI_2;
    let k = (angle / q).round();
    if (angle - k * q).abs() < 8f64.to_radians() {
        // A quarter turn swaps the axes; callers use it as rotation, so keep 0 when possible.
        if (k as i64).rem_euclid(2) == 0 { 0.0 } else { q }
    } else {
        angle
    }
}

/// Corners of a closed ring by Douglas–Peucker, with almost-straight corners removed.
fn polygon_corners(ring: &[Point], eps: f64) -> Vec<Point> {
    // Split the ring at its two farthest-apart points and simplify each half.
    let a = 0;
    let b = (0..ring.len()).max_by(|&i, &j| ring[a].distance(ring[i]).total_cmp(&ring[a].distance(ring[j]))).unwrap();
    let a = (0..ring.len()).max_by(|&i, &j| ring[b].distance(ring[i]).total_cmp(&ring[b].distance(ring[j]))).unwrap();
    let (lo, hi) = (a.min(b), a.max(b));
    let first: Vec<Point> = ring[lo..=hi].to_vec();
    let mut second: Vec<Point> = ring[hi..].to_vec();
    second.extend_from_slice(&ring[..=lo]);
    let mut corners = douglas_peucker(&first, eps);
    corners.pop();
    let mut rest = douglas_peucker(&second, eps);
    rest.pop();
    corners.extend(rest);
    // Drop corners that barely turn.
    loop {
        let n = corners.len();
        if n <= 3 {
            break;
        }
        let flattest = (0..n)
            .map(|i| {
                let (p, c, q) = (corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
                let (u, v) = (p - c, q - c);
                (i, (u.dot(v) / (u.hypot() * v.hypot()).max(1e-9)).clamp(-1.0, 1.0).acos())
            })
            .max_by(|x, y| x.1.total_cmp(&y.1))
            .unwrap();
        if flattest.1 > 155f64.to_radians() {
            corners.remove(flattest.0);
        } else {
            break;
        }
    }
    corners
}

fn douglas_peucker(pts: &[Point], eps: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let line = kurbo::Line::new(pts[0], *pts.last().unwrap());
    let (idx, d) = pts[1..pts.len() - 1]
        .iter()
        .enumerate()
        .map(|(i, p)| (i + 1, dist_to_segment(*p, line)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    if d <= eps {
        return vec![pts[0], *pts.last().unwrap()];
    }
    let mut left = douglas_peucker(&pts[..=idx], eps);
    left.pop();
    left.extend(douglas_peucker(&pts[idx..], eps));
    left
}

/// Four corners with roughly right angles → a (possibly rotated) rectangle.
fn as_rect(c: &[Point]) -> Option<Recognized> {
    for i in 0..4 {
        let (p, q, r) = (c[(i + 3) % 4], c[i], c[(i + 1) % 4]);
        let (u, v) = (p - q, r - q);
        let cos = u.dot(v) / (u.hypot() * v.hypot()).max(1e-9);
        if cos.abs() > 20f64.to_radians().sin() {
            return None;
        }
    }
    // Orientation from the longest side; size from the averaged opposite sides.
    let longest = (0..4).max_by(|&i, &j| c[i].distance(c[(i + 1) % 4]).total_cmp(&c[j].distance(c[(j + 1) % 4])))?;
    let d = c[(longest + 1) % 4] - c[longest];
    let mut angle = d.y.atan2(d.x);
    // Express as the smallest rotation (|angle| ≤ 45°), swapping width/height.
    let q = std::f64::consts::FRAC_PI_2;
    angle -= (angle / q).round() * q;
    let angle = if angle.abs() < 8f64.to_radians() { 0.0 } else { angle };
    let center = c.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / 4.0;
    let (cos, sin) = (angle.cos(), angle.sin());
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for p in c {
        let d = p.to_vec2() - center;
        w = w.max((d.x * cos + d.y * sin).abs() * 2.0);
        h = h.max((-d.x * sin + d.y * cos).abs() * 2.0);
    }
    Some(Recognized::Rect { center: center.to_point(), size: Vec2::new(w, h), angle })
}

/// Sculpt brush modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SculptMode {
    /// Drag points along with the brush (like pushing with a finger).
    #[default]
    Push,
    /// Pull points towards the brush centre (magnet).
    Attract,
    /// Push points away from the brush centre.
    Repel,
}

/// Smooth brush falloff: 1 at the centre, 0 at the rim and beyond.
pub fn falloff(distance: f64, radius: f64) -> f64 {
    if distance >= radius || radius <= 0.0 {
        return 0.0;
    }
    let t = distance / radius;
    (1.0 - t * t).powi(2)
}

/// Moves every control point (anchors and handles) of `path` by `field(point)`.
pub fn deform(path: &BezPath, field: impl Fn(Point) -> Vec2) -> BezPath {
    use kurbo::PathEl;
    let f = |p: Point| p + field(p);
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(f(p)),
            PathEl::LineTo(p) => PathEl::LineTo(f(p)),
            PathEl::QuadTo(a, p) => PathEl::QuadTo(f(a), f(p)),
            PathEl::CurveTo(a, b, p) => PathEl::CurveTo(f(a), f(b), f(p)),
            PathEl::ClosePath => PathEl::ClosePath,
        })
        .collect()
}

/// One brush step in the path's own space: `center`/`radius` of the brush,
/// `delta` how far it moved (Push), `amount` pixels of pull/push this step.
pub fn sculpt(path: &BezPath, mode: SculptMode, center: Point, radius: f64, delta: Vec2, amount: f64) -> BezPath {
    let dense = densify(path, center, radius, (radius / 4.0).max(2.0));
    deform(&dense, |p| {
        let w = falloff(p.distance(center), radius);
        if w == 0.0 {
            return Vec2::ZERO;
        }
        let to_center = center - p;
        let dist = to_center.hypot();
        match mode {
            SculptMode::Push => delta * w,
            SculptMode::Attract if dist > 1e-6 => to_center / dist * (amount * w).min(dist),
            SculptMode::Repel if dist > 1e-6 => -to_center / dist * amount * w,
            _ => Vec2::ZERO,
        }
    })
}

/// Splits segments that touch the brush circle until they're shorter than
/// `max_len`, so a brush has points to move (like subdividing a mesh).
/// Splitting at parameter midpoints leaves the shape unchanged.
pub fn densify(path: &BezPath, center: Point, radius: f64, max_len: f64) -> BezPath {
    use kurbo::{ParamCurve, ParamCurveArclen, PathEl, PathSeg, Shape};
    let mut out = BezPath::new();
    let mut last = Point::ZERO;
    let mut start = Point::ZERO;
    let near = |seg: &PathSeg| {
        let b = seg.bounding_box();
        let dx = (b.x0 - center.x).max(0.0).max(center.x - b.x1);
        let dy = (b.y0 - center.y).max(0.0).max(center.y - b.y1);
        dx.hypot(dy) < radius
    };
    let push = |out: &mut BezPath, seg: PathSeg| {
        let n = if near(&seg) { (seg.arclen(0.1) / max_len).ceil().clamp(1.0, 64.0) as usize } else { 1 };
        for i in 0..n {
            let piece = seg.subsegment(i as f64 / n as f64..(i + 1) as f64 / n as f64);
            match piece {
                // Straight sides become curves so they can bend smoothly.
                PathSeg::Line(l) if n > 1 => out.curve_to(l.p0.lerp(l.p1, 1.0 / 3.0), l.p0.lerp(l.p1, 2.0 / 3.0), l.p1),
                other => out.push(other.as_path_el()),
            }
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(p);
                last = p;
                start = p;
            }
            PathEl::LineTo(p) => {
                push(&mut out, PathSeg::Line(kurbo::Line::new(last, p)));
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                push(&mut out, PathSeg::Quad(kurbo::QuadBez::new(last, a, p)));
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                push(&mut out, PathSeg::Cubic(kurbo::CubicBez::new(last, a, b, p)));
                last = p;
            }
            PathEl::ClosePath => {
                // Make the closing edge explicit so it can be split and bent too.
                if last.distance(start) > 1e-9 {
                    push(&mut out, PathSeg::Line(kurbo::Line::new(last, start)));
                }
                out.close_path();
                last = start;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn figures_fill_their_bounds() {
        use kurbo::Shape;
        let b = Rect::new(10.0, 20.0, 110.0, 80.0);
        for f in Figure::ALL {
            let p = figure_path(f, &FigureParams::default(), b);
            let bb = p.bounding_box();
            assert!((bb.x0 - b.x0).abs() < 0.5 && (bb.x1 - b.x1).abs() < 0.5, "{f:?} {bb:?}");
            assert!((bb.y0 - b.y0).abs() < 0.5 && (bb.y1 - b.y1).abs() < 0.5, "{f:?} {bb:?}");
            assert!(p.area().abs() > 0.2 * b.area(), "{f:?} has an inside");
        }
    }

    #[test]
    fn gear_hole_is_empty() {
        use kurbo::Shape;
        let b = Rect::new(-50.0, -50.0, 50.0, 50.0);
        let p = gear_path(10, 0.2, 0.3, b);
        assert_eq!(p.winding(Point::ZERO), 0, "the middle is a hole");
        assert_ne!(p.winding(Point::new(0.0, -30.0)), 0, "the body is filled");
        let solid = gear_path(10, 0.2, 0.0, b);
        assert_ne!(solid.winding(Point::ZERO), 0);
    }

    use super::*;

    fn circle(c: Point, r: f64, n: usize, wobble: f64) -> Vec<Point> {
        (0..=n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                let rr = r + wobble * (a * 7.0).sin();
                c + Vec2::new(a.cos() * rr, a.sin() * rr)
            })
            .collect()
    }

    fn polyline(corners: &[Point], per_side: usize, jitter: f64) -> Vec<Point> {
        let mut out = Vec::new();
        for i in 0..corners.len() {
            let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
            for k in 0..per_side {
                let t = k as f64 / per_side as f64;
                let n = Vec2::new(-(b - a).y, (b - a).x).normalize() * jitter * ((k * 13 % 7) as f64 / 7.0 - 0.5);
                out.push(a.lerp(b, t) + n);
            }
        }
        out.push(corners[0]);
        out
    }

    #[test]
    fn recognises_lines_circles_ellipses() {
        let line: Vec<Point> = (0..30).map(|i| Point::new(i as f64 * 5.0, 100.0 + (i % 3) as f64)).collect();
        assert!(matches!(recognize(&line), Recognized::Line(..)));

        match recognize(&circle(Point::new(200.0, 200.0), 80.0, 80, 2.0)) {
            Recognized::Ellipse { center, radii, .. } => {
                assert!(center.distance(Point::new(200.0, 200.0)) < 5.0);
                assert!((radii.x - radii.y).abs() < 1e-9 && (radii.x - 80.0).abs() < 6.0, "{radii:?}");
            }
            other => panic!("{other:?}"),
        }
        let oval: Vec<Point> = (0..=80)
            .map(|i| i as f64 / 80.0 * std::f64::consts::TAU)
            .map(|a| Point::new(a.cos() * 150.0, a.sin() * 60.0))
            .collect();
        match recognize(&oval) {
            Recognized::Ellipse { radii, angle, .. } => {
                assert!((radii.x - 150.0).abs() < 5.0 && (radii.y - 60.0).abs() < 5.0 && angle == 0.0, "{radii:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn recognises_rectangles_and_triangles() {
        let rect = polyline(
            &[Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 100.0), Point::new(0.0, 100.0)],
            20,
            3.0,
        );
        match recognize(&rect) {
            Recognized::Rect { center, size, angle } => {
                assert!(center.distance(Point::new(100.0, 50.0)) < 5.0, "{center:?}");
                assert!((size.x - 200.0).abs() < 10.0 && (size.y - 100.0).abs() < 10.0, "{size:?}");
                assert_eq!(angle, 0.0);
            }
            other => panic!("{other:?}"),
        }
        let tri = polyline(&[Point::new(100.0, 0.0), Point::new(200.0, 170.0), Point::new(0.0, 170.0)], 25, 3.0);
        match recognize(&tri) {
            Recognized::Polygon(c) => assert_eq!(c.len(), 3),
            other => panic!("{other:?}"),
        }
        // A scribble stays freehand.
        let scribble: Vec<Point> = (0..100)
            .map(|i| Point::new(i as f64 * 3.0, ((i as f64) * 0.4).sin() * 40.0 + (i as f64 * 1.3).cos() * 20.0))
            .collect();
        let r = recognize(&scribble);
        assert!(matches!(r, Recognized::Free { closed: false, .. }), "{r:?}");
    }

    #[test]
    fn polygons_fill_their_box() {
        let tri = polygon_path(3, None, Rect::new(0.0, 0.0, 100.0, 80.0));
        let b = kurbo::Shape::bounding_box(&tri);
        assert!((b.width() - 100.0).abs() < 1e-9 && (b.height() - 80.0).abs() < 1e-9);
        let star = polygon_path(5, Some(0.5), Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(xenify_doc::bezier::point_count(&star), 10);
    }

    #[test]
    fn sculpt_moves_only_what_is_under_the_brush() {
        let square = closed_polyline(&[
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 100.0),
        ]);
        let pushed = sculpt(&square, SculptMode::Push, Point::new(50.0, 0.0), 20.0, Vec2::new(0.0, -10.0), 0.0);
        let b = kurbo::Shape::bounding_box(&pushed);
        assert!(b.y0 < -5.0, "the top edge bulges up: {b:?}");
        assert!((b.y1 - 100.0).abs() < 1e-9, "the bottom edge doesn't move");
        let repelled = sculpt(&square, SculptMode::Repel, Point::new(50.0, 10.0), 20.0, Vec2::ZERO, 5.0);
        assert!(kurbo::Shape::bounding_box(&repelled).y0 < 0.0);
        // Densifying alone keeps the outline.
        let dense = densify(&square, Point::new(50.0, 0.0), 20.0, 5.0);
        assert!(xenify_doc::bezier::point_count(&dense) > 4);
        assert!((kurbo::Shape::area(&dense) - kurbo::Shape::area(&square)).abs() < 1e-6);
    }
}
