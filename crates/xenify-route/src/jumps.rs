//! Line jumps: where a connector crosses one drawn before it (and they
//! don't meet there), it hops over, so crossing lines never look joined.
//! Also turns routed polylines into drawable paths with rounded corners or
//! curves, and finds junction dots where connectors sharing an end split.

use kurbo::{BezPath, Point, Vec2};

/// How a connector shows that it crosses another without joining it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum JumpStyle {
    #[default]
    Arc,
    Gap,
    Square,
    None,
}

/// A hop on one segment of a polyline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    /// Index of the segment (from point `segment` to `segment + 1`).
    pub segment: usize,
    /// Distance from the segment's start to the crossing.
    pub at: f64,
}

/// For each polyline, where it crosses a polyline earlier in the list.
/// Crossings within `size` of either line's ends are left out: lines meeting
/// at a shared port aren't crossing.
pub fn find_hops(polys: &[&[Point]], size: f64) -> Vec<Vec<Hop>> {
    let mut out = vec![Vec::new(); polys.len()];
    for (i, a) in polys.iter().enumerate() {
        for b in &polys[..i] {
            for (si, sa) in a.windows(2).enumerate() {
                for sb in b.windows(2) {
                    let Some((t, u)) = cross(sa[0], sa[1], sb[0], sb[1]) else { continue };
                    let p = sa[0].lerp(sa[1], t);
                    let q = sb[0].lerp(sb[1], u);
                    if near_ends(a, p, size) || near_ends(b, q, size) {
                        continue;
                    }
                    out[i].push(Hop { segment: si, at: t * sa[0].distance(sa[1]) });
                }
            }
        }
        out[i].sort_by(|x, y| x.segment.cmp(&y.segment).then(x.at.total_cmp(&y.at)));
    }
    out
}

fn near_ends(line: &[Point], p: Point, size: f64) -> bool {
    match (line.first(), line.last()) {
        (Some(&s), Some(&e)) => s.distance(p) < size || e.distance(p) < size,
        _ => false,
    }
}

/// Proper crossing of segments ab and cd (not touching, not parallel):
/// the parameters along each.
fn cross(a: Point, b: Point, c: Point, d: Point) -> Option<(f64, f64)> {
    let r = b - a;
    let s = d - c;
    let den = r.cross(s);
    if den.abs() < 1e-9 * r.hypot().max(1.0) * s.hypot().max(1.0) {
        return None;
    }
    let t = (c - a).cross(s) / den;
    let u = (c - a).cross(r) / den;
    let inside = |v: f64| v > 1e-6 && v < 1.0 - 1e-6;
    (inside(t) && inside(u)).then_some((t, u))
}

/// Which way a hop bulges on a segment going `d`: up for horizontal-ish
/// segments, left for vertical ones, so every hop in a drawing matches.
fn bulge(d: Vec2) -> Vec2 {
    let n = Vec2::new(d.y, -d.x);
    if n.y < -1e-9 || (n.y.abs() <= 1e-9 && n.x < 0.0) { n } else { -n }
}

/// Builds the drawn path for a routed polyline: corners rounded by
/// `radius`, hops of half-width `size` drawn in `style`. `curved` instead
/// smooths the whole line through its segments' midpoints (hops are skipped:
/// curves don't jump).
pub fn build_path(poly: &[Point], hops: &[Hop], style: JumpStyle, size: f64, radius: f64, curved: bool) -> BezPath {
    let mut path = BezPath::new();
    let n = poly.len();
    if n == 0 {
        return path;
    }
    path.move_to(poly[0]);
    if n == 1 {
        return path;
    }
    if curved {
        for k in 1..n - 1 {
            let mid = poly[k].midpoint(poly[k + 1]);
            let end = if k == n - 2 { poly[n - 1] } else { mid };
            path.quad_to(poly[k], end);
        }
        if n == 2 {
            path.line_to(poly[1]);
        }
        return path;
    }
    let len = |k: usize| poly[k].distance(poly[k + 1]);
    // How far each corner's rounding reaches along its two segments.
    let cut: Vec<f64> = (0..n)
        .map(|k| if k == 0 || k == n - 1 { 0.0 } else { radius.max(0.0).min(len(k - 1) / 2.0).min(len(k) / 2.0) })
        .collect();
    for k in 0..n - 1 {
        let (a, b) = (poly[k], poly[k + 1]);
        let l = len(k);
        if l < 1e-9 {
            continue;
        }
        let d = (b - a) / l;
        if style != JumpStyle::None && size > 0.0 {
            let mut last_end = f64::NEG_INFINITY;
            for h in hops.iter().filter(|h| h.segment == k) {
                if h.at - size < cut[k] || h.at + size > l - cut[k + 1] || h.at - size < last_end {
                    continue;
                }
                last_end = h.at + size;
                let c = a + d * h.at;
                let p0 = c - d * size;
                let p1 = c + d * size;
                let up = bulge(d) * size;
                path.line_to(p0);
                match style {
                    JumpStyle::Arc => {
                        const K: f64 = 0.552_284_749_8;
                        let top = c + up;
                        path.curve_to(p0 + up * K, top - d * size * K, top);
                        path.curve_to(top + d * size * K, p1 + up * K, p1);
                    }
                    JumpStyle::Gap => path.move_to(p1),
                    JumpStyle::Square => {
                        path.line_to(p0 + up);
                        path.line_to(p1 + up);
                        path.line_to(p1);
                    }
                    JumpStyle::None => {}
                }
            }
        }
        if k + 1 == n - 1 {
            path.line_to(b);
        } else {
            path.line_to(b - d * cut[k + 1]);
            if cut[k + 1] > 0.0 {
                let next = poly[k + 2];
                let dn = (next - b).normalize();
                path.quad_to(b, b + dn * cut[k + 1]);
            }
        }
    }
    path
}

/// Dots where connectors that share an end run together and then split:
/// the point where each such pair stops overlapping.
pub fn junctions(polys: &[&[Point]], eps: f64) -> Vec<Point> {
    let mut dots: Vec<Point> = Vec::new();
    let ends = |p: &[Point]| [(p[0], false), (p[p.len() - 1], true)];
    for (i, a) in polys.iter().enumerate() {
        for b in &polys[i + 1..] {
            if a.len() < 2 || b.len() < 2 {
                continue;
            }
            for (pa, ra) in ends(a) {
                for (pb, rb) in ends(b) {
                    if pa.distance(pb) > eps {
                        continue;
                    }
                    let wa: Vec<Point> = if ra { a.iter().rev().copied().collect() } else { a.to_vec() };
                    let wb: Vec<Point> = if rb { b.iter().rev().copied().collect() } else { b.to_vec() };
                    if let Some(p) = split_point(&wa, &wb, eps)
                        && !dots.iter().any(|d| d.distance(p) < eps)
                    {
                        dots.push(p);
                    }
                }
            }
        }
    }
    dots
}

/// Walks two lines from their shared start while they overlap; the point
/// where they part, if they overlapped for any length and don't simply end
/// together.
fn split_point(a: &[Point], b: &[Point], eps: f64) -> Option<Point> {
    let (mut i, mut j) = (0, 0);
    let mut pos = a[0];
    let mut walked = 0.0;
    while i + 1 < a.len() && j + 1 < b.len() {
        let (ea, eb) = (a[i + 1], b[j + 1]);
        let (la, lb) = (pos.distance(ea), pos.distance(eb));
        if la < eps {
            i += 1;
            continue;
        }
        if lb < eps {
            j += 1;
            continue;
        }
        let (da, db) = ((ea - pos) / la, (eb - pos) / lb);
        if (da - db).hypot() > 1e-6 {
            break;
        }
        let step = la.min(lb);
        pos += da * step;
        walked += step;
    }
    let finished = i + 1 >= a.len() || j + 1 >= b.len();
    (walked > eps && !finished).then_some(pos)
}
