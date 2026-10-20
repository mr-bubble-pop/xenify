//! Blending one drawing into another (the Blend modifier): outlines are
//! matched point for point, then everything is mixed.

use crate::{DrawItem, SceneStroke};
use xenify_doc::bezier::{Node, SubPath};
use xenify_doc::{Animatable, Color, Gradient, bezier};
use kurbo::{BezPath, ParamCurve, ParamCurveArclen, PathSeg, Point, Shape};

/// Points per outline when two outlines don't already match.
const POINTS: usize = 64;

/// `a` turned `t` of the way (0…1) into `b`.
pub fn lerp_paths(a: &BezPath, b: &BezPath, t: f64) -> BezPath {
    let (sa, sb) = (bezier::to_nodes(a), bezier::to_nodes(b));
    let same = sa.len() == sb.len()
        && sa.iter().zip(&sb).all(|(x, y)| {
            x.closed == y.closed
                && x.nodes.len() == y.nodes.len()
                && x.nodes.iter().zip(&y.nodes).all(|(p, q)| {
                    p.handle_in.is_some() == q.handle_in.is_some() && p.handle_out.is_some() == q.handle_out.is_some()
                })
        });
    let (sa, sb) = if same { (sa, sb) } else { matched(a, b) };
    let mix = |p: Point, q: Point| p.lerp(q, t);
    let mixo = |p: Option<Point>, q: Option<Point>, pa: Point, pb: Point| match (p, q) {
        (None, None) => None,
        (p, q) => Some(mix(p.unwrap_or(pa), q.unwrap_or(pb))),
    };
    let out: Vec<SubPath> = sa
        .iter()
        .zip(&sb)
        .map(|(x, y)| SubPath {
            closed: x.closed,
            nodes: x
                .nodes
                .iter()
                .zip(&y.nodes)
                .map(|(p, q)| Node {
                    pos: mix(p.pos, q.pos),
                    handle_in: mixo(p.handle_in, q.handle_in, p.pos, q.pos),
                    handle_out: mixo(p.handle_out, q.handle_out, p.pos, q.pos),
                })
                .collect(),
        })
        .collect();
    bezier::from_nodes(&out)
}

/// Both outlines as the same number of subpaths with the same number of
/// points (their own curves, split evenly), started near each other and
/// running the same way.
fn matched(a: &BezPath, b: &BezPath) -> (Vec<SubPath>, Vec<SubPath>) {
    let (mut pa, mut pb) = (pieces(a), pieces(b));
    // Missing subpaths shrink to (or grow from) a point in the middle.
    let pad = |v: &mut Vec<(Vec<PathSeg>, bool)>, n: usize, at: Point| {
        while v.len() < n {
            v.push((vec![PathSeg::Line(kurbo::Line::new(at, at))], true));
        }
    };
    let n = pa.len().max(pb.len());
    let (ca, cb) = (a.bounding_box().center(), b.bounding_box().center());
    pad(&mut pa, n, ca);
    pad(&mut pb, n, cb);
    let mut out_a = Vec::new();
    let mut out_b = Vec::new();
    for ((sa, ca_), (sb, cb_)) in pa.iter().zip(&pb) {
        let closed = *ca_ && *cb_;
        let count = POINTS.max(sa.len()).max(sb.len());
        let na = split(sa, count, closed);
        let mut nb = split(sb, count, closed);
        if closed {
            // Same direction of travel, then start at the point nearest a's start.
            let area = |v: &[Node]| {
                v.iter().zip(v.iter().cycle().skip(1)).map(|(p, q)| p.pos.x * q.pos.y - q.pos.x * p.pos.y).sum::<f64>()
            };
            if area(&na).signum() != area(&nb).signum() {
                nb.reverse();
                for node in &mut nb {
                    std::mem::swap(&mut node.handle_in, &mut node.handle_out);
                }
            }
            let first = na[0].pos;
            let k = (0..nb.len())
                .min_by(|&i, &j| (nb[i].pos - first).hypot2().total_cmp(&(nb[j].pos - first).hypot2()))
                .unwrap_or(0);
            nb.rotate_left(k);
        }
        out_a.push(SubPath { nodes: na, closed });
        out_b.push(SubPath { nodes: nb, closed });
    }
    (out_a, out_b)
}

fn pieces(path: &BezPath) -> Vec<(Vec<PathSeg>, bool)> {
    let mut out: Vec<(Vec<PathSeg>, bool)> = Vec::new();
    for seg in path.segments() {
        match out.last_mut() {
            Some((segs, _)) if segs.last().is_some_and(|l: &PathSeg| (l.end() - seg.start()).hypot() < 1e-9) => {
                segs.push(seg)
            }
            _ => out.push((vec![seg], false)),
        }
    }
    // Closed when it ends where it began.
    for (segs, closed) in &mut out {
        *closed = (segs[0].start() - segs.last().unwrap().end()).hypot() < 1e-6;
    }
    out
}

/// `segs` split into `count` curve pieces in all (longer segments get more),
/// as nodes; straight pieces get handles a third of the way along so they
/// can bend into curves.
fn split(segs: &[PathSeg], count: usize, closed: bool) -> Vec<Node> {
    let lens: Vec<f64> = segs.iter().map(|s| s.arclen(0.05)).collect();
    let total: f64 = lens.iter().sum::<f64>().max(1e-9);
    let mut per: Vec<usize> = lens.iter().map(|l| ((l / total) * count as f64).round().max(1.0) as usize).collect();
    // Make the pieces add up to `count` exactly.
    while per.iter().sum::<usize>() > count {
        let i = (0..per.len()).filter(|&i| per[i] > 1).max_by(|&i, &j| per[i].cmp(&per[j])).unwrap();
        per[i] -= 1;
    }
    while per.iter().sum::<usize>() < count {
        let i =
            (0..per.len()).max_by(|&i, &j| (lens[i] / per[i] as f64).total_cmp(&(lens[j] / per[j] as f64))).unwrap();
        per[i] += 1;
    }
    let mut nodes = vec![Node { pos: segs[0].start(), handle_in: None, handle_out: None }];
    for (s, &k) in segs.iter().zip(&per) {
        for j in 0..k {
            let piece = s.subsegment(j as f64 / k as f64..(j + 1) as f64 / k as f64);
            let c = match piece {
                PathSeg::Line(l) => {
                    kurbo::CubicBez::new(l.p0, l.p0.lerp(l.p1, 1.0 / 3.0), l.p0.lerp(l.p1, 2.0 / 3.0), l.p1)
                }
                PathSeg::Quad(q) => q.raise(),
                PathSeg::Cubic(c) => c,
            };
            nodes.last_mut().unwrap().handle_out = Some(c.p1);
            nodes.push(Node { pos: c.p3, handle_in: Some(c.p2), handle_out: None });
        }
    }
    if closed {
        let last = nodes.pop().unwrap();
        nodes[0].handle_in = last.handle_in;
    }
    nodes
}

fn clear(c: Color) -> Color {
    Color { a: 0.0, ..c }
}

fn lerp_gradient(a: &Gradient, b: &Gradient, t: f64) -> Gradient {
    if a.kind != b.kind || a.stops.len() != b.stops.len() {
        return if t < 0.5 { a.clone() } else { b.clone() };
    }
    Gradient {
        kind: a.kind,
        start: a.start.lerp(b.start, t),
        end: a.end.lerp(b.end, t),
        stops: a
            .stops
            .iter()
            .zip(&b.stops)
            .map(|(x, y)| xenify_doc::GradientStop {
                offset: x.offset + (y.offset - x.offset) * t,
                color: Color::lerp(&x.color, &y.color, t),
            })
            .collect(),
    }
}

/// Item `a` turned `t` of the way into item `b`, both in the same space.
pub fn lerp_items(a: &DrawItem, b: &DrawItem, t: f64) -> DrawItem {
    let color = |x: Option<Color>, y: Option<Color>| match (x, y) {
        (None, None) => None,
        (x, y) => {
            let (x, y) = (x.unwrap_or_else(|| clear(y.unwrap())), y.unwrap_or_else(|| clear(x.unwrap())));
            Some(Color::lerp(&x, &y, t))
        }
    };
    let gradient = match (&a.gradient, &b.gradient) {
        (Some(x), Some(y)) => Some(lerp_gradient(x, y, t)),
        (Some(x), None) if t < 0.5 => Some(x.clone()),
        (None, Some(y)) if t >= 0.5 => Some(y.clone()),
        _ => None,
    };
    let stroke = match (&a.stroke, &b.stroke) {
        (None, None) => None,
        (x, y) => {
            let base = x.as_ref().or(y.as_ref()).unwrap();
            let (cx, cy) = (x.as_ref().map(|s| s.color), y.as_ref().map(|s| s.color));
            let (wx, wy) = (x.as_ref().map_or(0.0, |s| s.width), y.as_ref().map_or(0.0, |s| s.width));
            let pick = if t < 0.5 { x.as_ref().unwrap_or(base) } else { y.as_ref().unwrap_or(base) };
            Some(SceneStroke {
                color: color(cx, cy).unwrap(),
                width: wx + (wy - wx) * t,
                marks: None,
                gradient: match (
                    &x.as_ref().and_then(|s| s.gradient.clone()),
                    &y.as_ref().and_then(|s| s.gradient.clone()),
                ) {
                    (Some(p), Some(q)) => Some(lerp_gradient(p, q, t)),
                    _ => pick.gradient.clone(),
                },
                ..pick.clone()
            })
        }
    };
    DrawItem {
        path: lerp_paths(&a.path, &b.path, t),
        fill_rule: if t < 0.5 { a.fill_rule } else { b.fill_rule },
        gradient,
        fill: color(a.fill, b.fill),
        stroke,
        pattern: if t < 0.5 { a.pattern.clone() } else { b.pattern.clone() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Circle, Rect};

    #[test]
    fn a_square_turns_into_a_circle() {
        let sq = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
        let circle = Circle::new((250.0, 50.0), 50.0).to_path(0.1);
        let mid = lerp_paths(&sq, &circle, 0.5);
        let b = mid.bounding_box();
        assert!((b.center().x - 150.0).abs() < 2.0 && (b.width() - 100.0).abs() < 4.0, "{b:?}");
        assert!(mid.area().abs() > 7500.0 && mid.area().abs() < 10000.0, "between the two: {}", mid.area());
        // Matching outlines blend point for point.
        let moved = Rect::new(200.0, 0.0, 300.0, 100.0).to_path(0.1);
        assert_eq!(lerp_paths(&sq, &moved, 0.5), Rect::new(100.0, 0.0, 200.0, 100.0).to_path(0.1));
        let start = lerp_paths(&sq, &circle, 0.0);
        assert!((start.bounding_box().width() - 100.0).abs() < 1e-6 && (start.area().abs() - 10_000.0).abs() < 1e-3);
    }
}
