//! Editing Bézier paths point by point (the "direct selection" model).
//!
//! A path is viewed as a list of anchors. Each anchor may have an incoming and
//! an outgoing handle, which are control points of the adjacent cubic
//! segments. Points are addressed by element index + slot so edits map back
//! onto the underlying [`BezPath`] exactly.

use kurbo::{BezPath, PathEl, Point, Vec2};

/// A control point inside a path: element index and which point of it
/// (`0`/`1`/`2` for a `CurveTo`'s p1/p2/p3; `0` for `MoveTo`/`LineTo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PointRef {
    pub element: usize,
    pub slot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub pos: Point,
    /// Where the anchor itself is stored.
    pub point: PointRef,
    /// Handle controlling the curve arriving at this anchor.
    pub handle_in: Option<(PointRef, Point)>,
    /// Handle controlling the curve leaving this anchor.
    pub handle_out: Option<(PointRef, Point)>,
    /// For a closed path's first anchor: the last element also ends here and
    /// must move with it.
    pub twin: Option<PointRef>,
}

fn get(els: &[PathEl], r: PointRef) -> Option<Point> {
    match (els.get(r.element)?, r.slot) {
        (PathEl::MoveTo(p) | PathEl::LineTo(p), 0) => Some(*p),
        (PathEl::QuadTo(a, _), 0) | (PathEl::CurveTo(a, _, _), 0) => Some(*a),
        (PathEl::QuadTo(_, b), 1) | (PathEl::CurveTo(_, b, _), 1) => Some(*b),
        (PathEl::CurveTo(_, _, c), 2) => Some(*c),
        _ => None,
    }
}

fn set(els: &mut [PathEl], r: PointRef, p: Point) {
    match (&mut els[r.element], r.slot) {
        (PathEl::MoveTo(a) | PathEl::LineTo(a), 0) => *a = p,
        (PathEl::QuadTo(a, _) | PathEl::CurveTo(a, _, _), 0) => *a = p,
        (PathEl::QuadTo(_, b) | PathEl::CurveTo(_, b, _), 1) => *b = p,
        (PathEl::CurveTo(_, _, c), 2) => *c = p,
        _ => {}
    }
}

/// The end point of an element (where an anchor lives).
fn end_slot(el: &PathEl) -> Option<usize> {
    match el {
        PathEl::MoveTo(_) | PathEl::LineTo(_) => Some(0),
        PathEl::QuadTo(..) => Some(1),
        PathEl::CurveTo(..) => Some(2),
        PathEl::ClosePath => None,
    }
}

/// Lists the anchors of `path` in order.
pub fn anchors(path: &BezPath) -> Vec<Anchor> {
    let els = path.elements();
    let mut out: Vec<Anchor> = Vec::new();
    let mut subpath_start: Option<usize> = None; // index into `out`
    for (i, el) in els.iter().enumerate() {
        if matches!(el, PathEl::ClosePath) {
            // If the last segment returned exactly to the start, merge the two anchors.
            if let (Some(start), Some(last)) = (subpath_start, out.len().checked_sub(1))
                && last > start
                && (out[last].pos - out[start].pos).hypot() < 1e-9
            {
                let dup = out.pop().unwrap();
                out[start].twin = Some(dup.point);
                out[start].handle_in = dup.handle_in;
            }
            subpath_start = None;
            continue;
        }
        let slot = end_slot(el).unwrap();
        let point = PointRef { element: i, slot };
        let handle_in = match el {
            PathEl::CurveTo(_, c2, _) => Some((PointRef { element: i, slot: 1 }, *c2)),
            _ => None,
        };
        if matches!(el, PathEl::MoveTo(_)) {
            subpath_start = Some(out.len());
        } else if let (PathEl::CurveTo(c1, _, _), Some(prev)) = (el, out.last_mut()) {
            prev.handle_out = Some((PointRef { element: i, slot: 0 }, *c1));
        }
        out.push(Anchor { pos: get(els, point).unwrap(), point, handle_in, handle_out: None, twin: None });
    }
    // A closed path's closing curve (last element) leaves the first anchor…
    // handled above via `twin`; its outgoing handle belongs to the last anchor,
    // which the loop already recorded.
    out
}

/// Moves anchor `index` to `to`, carrying its handles along.
pub fn move_anchor(path: &BezPath, index: usize, to: Point) -> BezPath {
    let list = anchors(path);
    let Some(a) = list.get(index) else { return path.clone() };
    let delta = to - a.pos;
    let mut els = path.elements().to_vec();
    let mut shift = |r: PointRef| {
        if let Some(p) = get(&els, r) {
            set(&mut els, r, p + delta);
        }
    };
    shift(a.point);
    if let Some(t) = a.twin {
        shift(t);
    }
    if let Some((h, _)) = a.handle_in {
        shift(h);
    }
    if let Some((h, _)) = a.handle_out {
        shift(h);
    }
    BezPath::from_vec(els)
}

/// Moves a handle of anchor `index` (`incoming` or outgoing) to `to`.
///
/// If the anchor was smooth (both handles collinear through it) and `mirror`
/// is set, the opposite handle is rotated to stay collinear, keeping its length.
pub fn move_handle(path: &BezPath, index: usize, incoming: bool, to: Point, mirror: bool) -> BezPath {
    let list = anchors(path);
    let Some(a) = list.get(index) else { return path.clone() };
    let (this, other) = if incoming { (a.handle_in, a.handle_out) } else { (a.handle_out, a.handle_in) };
    let Some((r, old)) = this else { return path.clone() };
    let mut els = path.elements().to_vec();
    set(&mut els, r, to);
    if mirror
        && let Some((or, op)) = other
        && is_smooth(a.pos, old, op)
    {
        let dir = a.pos - to;
        let len = (op - a.pos).hypot();
        if dir.hypot() > 1e-9 {
            set(&mut els, or, a.pos + dir.normalize() * len);
        }
    }
    BezPath::from_vec(els)
}

/// Whether handles `h1` and `h2` of anchor `p` point in opposite directions.
pub fn is_smooth(p: Point, h1: Point, h2: Point) -> bool {
    let (a, b) = (h1 - p, h2 - p);
    if a.hypot() < 1e-9 || b.hypot() < 1e-9 {
        return false;
    }
    let cos = a.dot(b) / (a.hypot() * b.hypot());
    cos < -0.9995
}

/// A point placed with the pen: position plus outgoing handle offset
/// (zero for a corner; the incoming handle mirrors it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenPoint {
    pub pos: Point,
    pub handle: Vec2,
}

/// Builds a path through pen points: straight lines between corners, cubic
/// curves where a point has a handle.
pub fn pen_path(points: &[PenPoint], closed: bool) -> BezPath {
    let mut path = BezPath::new();
    let Some(first) = points.first() else { return path };
    path.move_to(first.pos);
    let segment = |path: &mut BezPath, a: &PenPoint, b: &PenPoint| {
        if a.handle == Vec2::ZERO && b.handle == Vec2::ZERO {
            path.line_to(b.pos);
        } else {
            path.curve_to(a.pos + a.handle, b.pos - b.handle, b.pos);
        }
    };
    for w in points.windows(2) {
        segment(&mut path, &w[0], &w[1]);
    }
    if closed && points.len() > 2 {
        let last = points.last().unwrap();
        if last.handle != Vec2::ZERO || first.handle != Vec2::ZERO {
            segment(&mut path, last, first);
        }
        path.close_path();
    }
    path
}

/// A path point with optional curve handles (absolute positions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub pos: Point,
    pub handle_in: Option<Point>,
    pub handle_out: Option<Point>,
}

impl Node {
    fn corner(pos: Point) -> Self {
        Self { pos, handle_in: None, handle_out: None }
    }
}

/// One continuous run of nodes; `closed` joins the last back to the first.
#[derive(Clone, Debug, PartialEq)]
pub struct SubPath {
    pub nodes: Vec<Node>,
    pub closed: bool,
}

/// The loose ends of a path's open subpaths: (subpath index, is it the start, point).
pub fn open_ends(path: &BezPath) -> Vec<(usize, bool, Point)> {
    to_nodes(path)
        .iter()
        .enumerate()
        .filter(|(_, sp)| !sp.closed && sp.nodes.len() >= 2)
        .flat_map(|(i, sp)| [(i, true, sp.nodes[0].pos), (i, false, sp.nodes[sp.nodes.len() - 1].pos)])
        .collect()
}

/// Continues open subpath `index` of `path` with pen points, from its end (or
/// from its start, which reverses its direction), and optionally closes it.
/// `None` if there is no such open subpath.
pub fn extend_subpath(
    path: &BezPath,
    index: usize,
    from_start: bool,
    points: &[PenPoint],
    close: bool,
) -> Option<BezPath> {
    let mut subs = to_nodes(path);
    let sp = subs.get_mut(index).filter(|sp| !sp.closed)?;
    if from_start {
        sp.nodes.reverse();
        for n in &mut sp.nodes {
            std::mem::swap(&mut n.handle_in, &mut n.handle_out);
        }
    }
    for p in points {
        sp.nodes.push(Node {
            pos: p.pos,
            handle_in: handle(p.pos - p.handle, p.pos),
            handle_out: handle(p.pos + p.handle, p.pos),
        });
    }
    sp.closed = close;
    Some(from_nodes(&subs))
}

/// Handles sitting exactly on their anchor are the same as no handle.
fn handle(h: Point, anchor: Point) -> Option<Point> {
    ((h - anchor).hypot() > 1e-9).then_some(h)
}

/// Converts a path into editable nodes (quadratics become cubics). Node order
/// matches [`anchors`].
pub fn to_nodes(path: &BezPath) -> Vec<SubPath> {
    let mut out: Vec<SubPath> = Vec::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => out.push(SubPath { nodes: vec![Node::corner(p)], closed: false }),
            PathEl::LineTo(p) => {
                if out.is_empty() {
                    out.push(SubPath { nodes: vec![], closed: false });
                }
                out.last_mut().unwrap().nodes.push(Node::corner(p));
            }
            PathEl::QuadTo(q, p) => {
                let Some(sp) = out.last_mut() else { continue };
                let prev = sp.nodes.last().map_or(p, |n| n.pos);
                let c1 = prev + (q - prev) * (2.0 / 3.0);
                let c2 = p + (q - p) * (2.0 / 3.0);
                if let Some(n) = sp.nodes.last_mut() {
                    n.handle_out = handle(c1, n.pos);
                }
                sp.nodes.push(Node { pos: p, handle_in: handle(c2, p), handle_out: None });
            }
            PathEl::CurveTo(c1, c2, p) => {
                let Some(sp) = out.last_mut() else { continue };
                if let Some(n) = sp.nodes.last_mut() {
                    n.handle_out = handle(c1, n.pos);
                }
                sp.nodes.push(Node { pos: p, handle_in: handle(c2, p), handle_out: None });
            }
            PathEl::ClosePath => {
                let Some(sp) = out.last_mut() else { continue };
                sp.closed = true;
                // An explicit segment back to the start: merge the duplicate node.
                if sp.nodes.len() > 1 && (sp.nodes.last().unwrap().pos - sp.nodes[0].pos).hypot() < 1e-9 {
                    let dup = sp.nodes.pop().unwrap();
                    sp.nodes[0].handle_in = dup.handle_in;
                }
            }
        }
    }
    out.retain(|sp| !sp.nodes.is_empty());
    out
}

fn segment(path: &mut BezPath, a: &Node, b: &Node) {
    if a.handle_out.is_none() && b.handle_in.is_none() {
        path.line_to(b.pos);
    } else {
        path.curve_to(a.handle_out.unwrap_or(a.pos), b.handle_in.unwrap_or(b.pos), b.pos);
    }
}

/// Builds a path from nodes (inverse of [`to_nodes`]).
pub fn from_nodes(subpaths: &[SubPath]) -> BezPath {
    let mut path = BezPath::new();
    for sp in subpaths {
        let Some(first) = sp.nodes.first() else { continue };
        path.move_to(first.pos);
        for w in sp.nodes.windows(2) {
            segment(&mut path, &w[0], &w[1]);
        }
        if sp.closed {
            let last = sp.nodes.last().unwrap();
            if sp.nodes.len() > 1 && (last.handle_out.is_some() || first.handle_in.is_some()) {
                segment(&mut path, last, first);
            }
            path.close_path();
        }
    }
    path
}

/// (subpath, node) for a flat node index as used by [`anchors`].
fn locate(subpaths: &[SubPath], index: usize) -> Option<(usize, usize)> {
    let mut i = index;
    for (s, sp) in subpaths.iter().enumerate() {
        if i < sp.nodes.len() {
            return Some((s, i));
        }
        i -= sp.nodes.len();
    }
    None
}

/// Where a point lies on the path: subpath, segment (from node `segment` to
/// the next), curve parameter and distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathHit {
    pub subpath: usize,
    pub segment: usize,
    pub t: f64,
    pub distance: f64,
}

fn seg_curve(a: &Node, b: &Node) -> kurbo::CubicBez {
    kurbo::CubicBez::new(a.pos, a.handle_out.unwrap_or(a.pos), b.handle_in.unwrap_or(b.pos), b.pos)
}

/// The closest point on the path's outline to `p`.
pub fn nearest(path: &BezPath, p: Point) -> Option<PathHit> {
    use kurbo::ParamCurveNearest;
    let subs = to_nodes(path);
    let mut best: Option<PathHit> = None;
    for (s, sp) in subs.iter().enumerate() {
        let n = sp.nodes.len();
        let count = if sp.closed { n } else { n.saturating_sub(1) };
        for k in 0..count {
            let c = seg_curve(&sp.nodes[k], &sp.nodes[(k + 1) % n]);
            let near = c.nearest(p, 1e-6);
            let d = near.distance_sq.sqrt();
            if best.is_none_or(|b| d < b.distance) {
                best = Some(PathHit { subpath: s, segment: k, t: near.t, distance: d });
            }
        }
    }
    best
}

/// Inserts a node at `hit`, splitting the segment without changing its shape.
/// Returns the new path and the new node's flat index.
pub fn insert_node(path: &BezPath, hit: PathHit) -> Option<(BezPath, usize)> {
    use kurbo::ParamCurve;
    let mut subs = to_nodes(path);
    let sp = subs.get_mut(hit.subpath)?;
    let n = sp.nodes.len();
    let (a_i, b_i) = (hit.segment, (hit.segment + 1) % n);
    let (a, b) = (sp.nodes[a_i], sp.nodes[b_i]);
    let t = hit.t.clamp(1e-4, 1.0 - 1e-4);
    let straight = a.handle_out.is_none() && b.handle_in.is_none();
    let node = if straight {
        Node::corner(a.pos.lerp(b.pos, t))
    } else {
        let c = seg_curve(&a, &b);
        let (l, r) = (c.subsegment(0.0..t), c.subsegment(t..1.0));
        sp.nodes[a_i].handle_out = handle(l.p1, a.pos);
        sp.nodes[b_i].handle_in = handle(r.p2, b.pos);
        Node { pos: l.p3, handle_in: handle(l.p2, l.p3), handle_out: handle(r.p1, l.p3) }
    };
    sp.nodes.insert(a_i + 1, node);
    let before: usize = subs[..hit.subpath].iter().map(|s| s.nodes.len()).sum();
    Some((from_nodes(&subs), before + a_i + 1))
}

/// Removes node `index`. Refuses if the subpath would be left with fewer than 2 nodes.
pub fn delete_node(path: &BezPath, index: usize) -> Option<BezPath> {
    let mut subs = to_nodes(path);
    let (s, i) = locate(&subs, index)?;
    if subs[s].nodes.len() <= 2 {
        return None;
    }
    subs[s].nodes.remove(i);
    Some(from_nodes(&subs))
}

/// Toggles node `index` between a corner (no handles) and a smooth point with
/// handles along the direction of its neighbours.
pub fn toggle_smooth(path: &BezPath, index: usize) -> Option<BezPath> {
    let mut subs = to_nodes(path);
    let (s, i) = locate(&subs, index)?;
    let sp = &mut subs[s];
    let n = sp.nodes.len();
    let node = sp.nodes[i];
    if node.handle_in.is_some() || node.handle_out.is_some() {
        sp.nodes[i].handle_in = None;
        sp.nodes[i].handle_out = None;
    } else {
        let prev = if i > 0 {
            Some(sp.nodes[i - 1].pos)
        } else if sp.closed {
            Some(sp.nodes[n - 1].pos)
        } else {
            None
        };
        let next = if i + 1 < n {
            Some(sp.nodes[i + 1].pos)
        } else if sp.closed {
            Some(sp.nodes[0].pos)
        } else {
            None
        };
        let p = node.pos;
        let dir = match (prev, next) {
            (Some(a), Some(b)) => b - a,
            (Some(a), None) => p - a,
            (None, Some(b)) => b - p,
            (None, None) => return None,
        };
        if dir.hypot() < 1e-9 {
            return None;
        }
        let d = dir.normalize();
        if let Some(a) = prev {
            sp.nodes[i].handle_in = Some(p - d * ((p - a).hypot() / 3.0));
        }
        if let Some(b) = next {
            sp.nodes[i].handle_out = Some(p + d * ((b - p).hypot() / 3.0));
        }
    }
    Some(from_nodes(&subs))
}

/// Fewer points, same look: refits smooth runs with as few curves as possible
/// (within `accuracy` units) and keeps sharp corners.
pub fn simplify(path: &BezPath, accuracy: f64) -> BezPath {
    // Turns gentler than ~27° (tan ≈ 0.5) count as smooth, so the many tiny
    // steps of traced/imported outlines are refitted as curves.
    let options = kurbo::simplify::SimplifyOptions::default().angle_thresh(0.5);
    let accuracy = accuracy.max(0.01);
    // kurbo takes a turn straight back (a spike) for a smooth join and then
    // tries to fit one curve through it at ever smaller scales, which can run
    // for minutes; so runs are cut wherever the outline turns back on itself.
    let mut out = BezPath::new();
    let mut run = BezPath::new();
    let mut starts_subpath = true;
    let flush = |run: &mut BezPath, out: &mut BezPath, starts_subpath: bool| {
        if run.elements().len() > 1 {
            out.extend(
                kurbo::simplify::simplify_bezpath(run.iter(), accuracy, &options)
                    .iter()
                    .skip(usize::from(!starts_subpath)),
            );
        } else if starts_subpath && let Some(el) = run.elements().first() {
            out.push(*el);
        }
        run.truncate(0);
    };
    let (mut at, mut start, mut last_tan) = (Point::ZERO, Point::ZERO, None::<Vec2>);
    for el in path.iter() {
        let seg = match el {
            PathEl::MoveTo(p) => {
                flush(&mut run, &mut out, starts_subpath);
                run.move_to(p);
                (at, start, last_tan, starts_subpath) = (p, p, None, true);
                continue;
            }
            PathEl::ClosePath => {
                flush(&mut run, &mut out, starts_subpath);
                out.close_path();
                (at, last_tan, starts_subpath) = (start, None, true);
                continue;
            }
            PathEl::LineTo(p) => vec![at, p],
            PathEl::QuadTo(a, p) => vec![at, a, p],
            PathEl::CurveTo(a, b, p) => vec![at, a, b, p],
        };
        // Direction leaving the start and arriving at the end (first/last distinct points).
        let end = *seg.last().unwrap();
        let t0 = seg.iter().find(|q| **q != at).map_or(Vec2::ZERO, |q| *q - at);
        let t1 = seg.iter().rev().find(|q| **q != end).map_or(Vec2::ZERO, |q| end - *q);
        if last_tan.is_some_and(|t| t.dot(t0) < 0.0) {
            flush(&mut run, &mut out, starts_subpath);
            run.move_to(at);
            starts_subpath = false;
        }
        if run.elements().is_empty() {
            run.move_to(at);
        }
        run.push(el);
        at = end;
        if t1.hypot2() > 0.0 {
            last_tan = Some(t1);
        }
    }
    flush(&mut run, &mut out, starts_subpath);
    out
}

/// Number of editable points (anchors) in a path.
pub fn point_count(path: &BezPath) -> usize {
    anchors(path).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuing_an_open_path_from_either_end() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        let ends = open_ends(&p);
        assert_eq!(ends, vec![(0, true, Point::new(0.0, 0.0)), (0, false, Point::new(10.0, 0.0))]);
        let more = [PenPoint { pos: Point::new(10.0, 10.0), handle: Vec2::ZERO }];
        let q = extend_subpath(&p, 0, false, &more, false).unwrap();
        assert_eq!(
            to_nodes(&q)[0].nodes.iter().map(|n| n.pos).collect::<Vec<_>>(),
            vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)]
        );
        // From the start: reversed, then extended; closing joins the loop.
        let curve = [PenPoint { pos: Point::new(0.0, 10.0), handle: Vec2::new(5.0, 0.0) }];
        let r = extend_subpath(&q, 0, true, &curve, true).unwrap();
        let sp = &to_nodes(&r)[0];
        assert!(sp.closed);
        assert_eq!(sp.nodes.first().unwrap().pos, Point::new(10.0, 10.0));
        assert_eq!(sp.nodes.last().unwrap().handle_out, Some(Point::new(5.0, 10.0)));
        assert!(open_ends(&r).is_empty());
        assert!(extend_subpath(&r, 0, false, &more, false).is_none(), "closed paths can't be continued");
    }

    fn pts(path: &BezPath) -> Vec<Point> {
        anchors(path).iter().map(|a| a.pos).collect()
    }

    #[test]
    fn open_polyline_anchors_and_move() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        assert_eq!(pts(&p), vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)]);
        let q = move_anchor(&p, 1, Point::new(20.0, 5.0));
        assert_eq!(pts(&q)[1], Point::new(20.0, 5.0));
        assert_eq!(pts(&q)[2], Point::new(10.0, 10.0));
    }

    #[test]
    fn curves_expose_handles_that_move_with_anchor() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::new(5.0, 0.0) },
            PenPoint { pos: Point::new(20.0, 0.0), handle: Vec2::new(0.0, 5.0) },
            PenPoint { pos: Point::new(40.0, 0.0), handle: Vec2::ZERO },
        ];
        let p = pen_path(&pen, false);
        let a = anchors(&p);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].handle_out.unwrap().1, Point::new(5.0, 0.0));
        assert_eq!(a[1].handle_in.unwrap().1, Point::new(20.0, -5.0));
        assert_eq!(a[1].handle_out.unwrap().1, Point::new(20.0, 5.0));
        assert!(is_smooth(a[1].pos, a[1].handle_in.unwrap().1, a[1].handle_out.unwrap().1));

        let q = move_anchor(&p, 1, Point::new(20.0, 10.0));
        let b = anchors(&q);
        assert_eq!(b[1].handle_in.unwrap().1, Point::new(20.0, 5.0));
        assert_eq!(b[1].handle_out.unwrap().1, Point::new(20.0, 15.0));
    }

    #[test]
    fn smooth_handles_mirror_unless_disabled() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(20.0, 0.0), handle: Vec2::new(10.0, 0.0) },
            PenPoint { pos: Point::new(40.0, 0.0), handle: Vec2::ZERO },
        ];
        let p = pen_path(&pen, false);
        // Drag the outgoing handle straight up: incoming handle swings down.
        let q = move_handle(&p, 1, false, Point::new(20.0, -10.0), true);
        let a = anchors(&q)[1];
        assert_eq!(a.handle_out.unwrap().1, Point::new(20.0, -10.0));
        let hin = a.handle_in.unwrap().1;
        assert!((hin - Point::new(20.0, 10.0)).hypot() < 1e-9, "{hin:?}");

        let r = move_handle(&p, 1, false, Point::new(20.0, -10.0), false);
        assert_eq!(anchors(&r)[1].handle_in.unwrap().1, Point::new(10.0, 0.0));
    }

    #[test]
    fn closed_curved_path_merges_start_and_end() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::new(0.0, -5.0) },
            PenPoint { pos: Point::new(10.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(5.0, 10.0), handle: Vec2::ZERO },
        ];
        let p = pen_path(&pen, true);
        let a = anchors(&p);
        assert_eq!(a.len(), 3, "{a:?}");
        assert!(a[0].twin.is_some());
        // Moving the first anchor moves the closing segment's end too.
        let q = move_anchor(&p, 0, Point::new(-5.0, 0.0));
        let els = q.elements();
        let end = match els[els.len() - 2] {
            PathEl::CurveTo(_, _, e) => e,
            ref other => panic!("{other:?}"),
        };
        assert_eq!(end, Point::new(-5.0, 0.0));
        assert_eq!(anchors(&q).len(), 3);
    }

    fn same_shape(a: &BezPath, b: &BezPath) {
        use kurbo::Shape;
        // Compare by sampling points along both outlines.
        let (ra, rb) = (a.bounding_box(), b.bounding_box());
        assert!((ra.x0 - rb.x0).abs() < 1e-6 && (ra.y1 - rb.y1).abs() < 1e-6, "{ra:?} vs {rb:?}");
        assert!((a.area() - b.area()).abs() < 1e-6, "{} vs {}", a.area(), b.area());
    }

    #[test]
    fn nodes_round_trip_and_match_anchor_order() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::new(0.0, -5.0) },
            PenPoint { pos: Point::new(10.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(5.0, 10.0), handle: Vec2::new(3.0, 0.0) },
        ];
        for closed in [false, true] {
            let p = pen_path(&pen, closed);
            let back = from_nodes(&to_nodes(&p));
            same_shape(&p, &back);
            let flat: Vec<Point> = to_nodes(&p).iter().flat_map(|s| s.nodes.iter().map(|n| n.pos)).collect();
            assert_eq!(flat, pts(&p));
        }
    }

    #[test]
    fn inserting_a_node_keeps_the_shape() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::new(0.0, -20.0) },
            PenPoint { pos: Point::new(100.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(50.0, 80.0), handle: Vec2::ZERO },
        ];
        let p = pen_path(&pen, true);
        let hit = nearest(&p, Point::new(50.0, -20.0)).unwrap();
        assert_eq!((hit.subpath, hit.segment), (0, 0));
        let (q, idx) = insert_node(&p, hit).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(anchors(&q).len(), 4);
        same_shape(&p, &q);
        // Straight segment (the implicit closing line).
        let hit = nearest(&q, Point::new(20.0, 45.0)).unwrap();
        let (r, _) = insert_node(&q, hit).unwrap();
        assert_eq!(anchors(&r).len(), 5);
        same_shape(&q, &r);
    }

    #[test]
    fn deleting_and_toggling_nodes() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(50.0, -30.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(100.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(50.0, 60.0), handle: Vec2::ZERO },
        ];
        let p = pen_path(&pen, true);
        let smooth = toggle_smooth(&p, 1).unwrap();
        let a = anchors(&smooth)[1];
        let (hi, ho) = (a.handle_in.unwrap().1, a.handle_out.unwrap().1);
        assert!(is_smooth(a.pos, hi, ho));
        // Handles run parallel to the neighbours' direction (0,0)→(100,0).
        assert!((hi.y - a.pos.y).abs() < 1e-9 && hi.x < a.pos.x);
        let corner = toggle_smooth(&smooth, 1).unwrap();
        same_shape(&p, &corner);

        let fewer = delete_node(&p, 1).unwrap();
        assert_eq!(anchors(&fewer).len(), 3);
        let two = delete_node(&fewer, 0).unwrap();
        assert!(delete_node(&two, 0).is_none(), "keeps at least 2 nodes");
    }

    #[test]
    fn simplify_reduces_points_but_keeps_the_shape() {
        use kurbo::Shape;
        // A circle traced as 200 tiny line segments (like a traced bitmap).
        let mut p = BezPath::new();
        for i in 0..200 {
            let a = i as f64 / 200.0 * std::f64::consts::TAU;
            let pt = Point::new(100.0 * a.cos(), 100.0 * a.sin());
            if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
        }
        p.close_path();
        let s = simplify(&p, 0.5);
        assert!(point_count(&s) < 20, "{} points", point_count(&s));
        let (a, b) = (p.area().abs(), s.area().abs());
        assert!((a - b).abs() / a < 0.01, "{a} vs {b}");
    }

    #[test]
    fn simplify_cuts_at_spikes_instead_of_fitting_through_them() {
        // From a traced picture: the last curve turns straight back down the
        // way the one before came up. kurbo alone took minutes on it.
        let p = BezPath::from_svg("M159.09574535124557,-12.289217591285706 C159.09574535124557,-11.453905284404755 159.09574535124557,-10.618592977523804 159.09574535124557,-9.757967591285706 C159.65262344115035,-9.757967591285706 160.20950153105514,-9.757967591285706 160.78324535124557,-9.757967591285706 C161.06168439619796,-10.314842462539673 161.34012344115035,-10.87171733379364 161.62699535124557,-11.445467591285706 C161.62699535124557,-10.888592720031738 161.62699535124557,-10.331717848777771 161.62699535124557,-9.757967591285706").unwrap();
        let s = simplify(&p, 0.75);
        assert_eq!(point_count(&s), 5, "{}", s.to_svg());
    }

    #[test]
    fn simplify_keeps_real_corners() {
        use kurbo::Shape;
        let sq = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
        let s = simplify(&sq, 1.0);
        assert_eq!(point_count(&s), 4);
        assert!((s.area().abs() - 10000.0).abs() < 1.0);
    }

    #[test]
    fn closed_polygon_keeps_close_path() {
        let pen = [
            PenPoint { pos: Point::new(0.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(10.0, 0.0), handle: Vec2::ZERO },
            PenPoint { pos: Point::new(5.0, 10.0), handle: Vec2::ZERO },
        ];
        let p = pen_path(&pen, true);
        assert!(matches!(p.elements().last(), Some(PathEl::ClosePath)));
        assert_eq!(anchors(&p).len(), 3);
    }
}

// ---------------------------------------------------------------- editing several points

/// Flat indices ([`anchors`] order) → (subpath, node).
fn flat_index(subpaths: &[SubPath]) -> Vec<(usize, usize)> {
    subpaths.iter().enumerate().flat_map(|(s, sp)| (0..sp.nodes.len()).map(move |i| (s, i))).collect()
}

/// Splits the segment from `a` to `b` in half (de Casteljau), returning the
/// new middle node and the adjusted handles of `a` and `b`.
fn halve(a: Node, b: Node) -> (Node, Option<Point>, Option<Point>) {
    if a.handle_out.is_none() && b.handle_in.is_none() {
        return (Node::corner(a.pos.midpoint(b.pos)), None, None);
    }
    let (p0, p1, p2, p3) = (a.pos, a.handle_out.unwrap_or(a.pos), b.handle_in.unwrap_or(b.pos), b.pos);
    let (q0, q1, q2) = (p0.midpoint(p1), p1.midpoint(p2), p2.midpoint(p3));
    let (r0, r1) = (q0.midpoint(q1), q1.midpoint(q2));
    let m = r0.midpoint(r1);
    (Node { pos: m, handle_in: handle(r0, m), handle_out: handle(r1, m) }, handle(q0, a.pos), handle(q2, b.pos))
}

/// Adds a point halfway along every segment between two selected points (or
/// along every segment when `selected` is empty), like Blender's Subdivide.
pub fn subdivide(path: &BezPath, selected: &[usize]) -> BezPath {
    let subs = to_nodes(path);
    let idx = flat_index(&subs);
    let chosen = |s: usize, i: usize| {
        selected.is_empty() || idx.iter().position(|x| *x == (s, i)).is_some_and(|f| selected.contains(&f))
    };
    let mut out = Vec::new();
    for (s, sp) in subs.iter().enumerate() {
        let n = sp.nodes.len();
        let mut nodes: Vec<Node> = Vec::new();
        let segs = if sp.closed { n } else { n.saturating_sub(1) };
        let mut cur = sp.nodes.clone();
        let mut mids: Vec<Option<Node>> = vec![None; n];
        for i in 0..segs {
            let j = (i + 1) % n;
            if chosen(s, i) && chosen(s, j) {
                let (m, ha, hb) = halve(cur[i], cur[j]);
                cur[i].handle_out = ha;
                cur[j].handle_in = hb;
                mids[i] = Some(m);
            }
        }
        for i in 0..n {
            nodes.push(cur[i]);
            if let Some(m) = mids[i] {
                nodes.push(m);
            }
        }
        out.push(SubPath { nodes, closed: sp.closed });
    }
    from_nodes(&out)
}

/// Removes several points; runs left with fewer than two points go entirely.
pub fn delete_nodes(path: &BezPath, indices: &[usize]) -> BezPath {
    let subs = to_nodes(path);
    let idx = flat_index(&subs);
    let out: Vec<SubPath> = subs
        .iter()
        .enumerate()
        .map(|(s, sp)| SubPath {
            nodes: sp
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, _)| !idx.iter().position(|x| *x == (s, *i)).is_some_and(|f| indices.contains(&f)))
                .map(|(_, n)| *n)
                .collect(),
            closed: sp.closed,
        })
        .filter(|sp| sp.nodes.len() >= 2)
        .collect();
    from_nodes(&out)
}

/// Moves several points (with their handles) by `delta`.
pub fn move_nodes(path: &BezPath, indices: &[usize], delta: kurbo::Vec2) -> BezPath {
    let mut subs = to_nodes(path);
    let idx = flat_index(&subs);
    for f in indices {
        if let Some(&(s, i)) = idx.get(*f) {
            let n = &mut subs[s].nodes[i];
            n.pos += delta;
            n.handle_in = n.handle_in.map(|h| h + delta);
            n.handle_out = n.handle_out.map(|h| h + delta);
        }
    }
    from_nodes(&subs)
}

/// Breaks the path at point `index`: a closed outline opens there (the
/// point becomes both ends); an open line splits in two.
pub fn break_at(path: &BezPath, index: usize) -> Option<BezPath> {
    let mut subs = to_nodes(path);
    let (s, i) = locate(&subs, index)?;
    let sp = subs[s].clone();
    if sp.closed {
        let mut nodes: Vec<Node> = sp.nodes[i..].iter().chain(sp.nodes[..i].iter()).copied().collect();
        let mut end = nodes[0];
        end.handle_out = None;
        nodes[0].handle_in = None;
        nodes.push(end);
        subs[s] = SubPath { nodes, closed: false };
    } else {
        if i == 0 || i + 1 >= sp.nodes.len() {
            return None; // Already an end.
        }
        let mut first: Vec<Node> = sp.nodes[..=i].to_vec();
        let mut second: Vec<Node> = sp.nodes[i..].to_vec();
        first.last_mut().unwrap().handle_out = None;
        second[0].handle_in = None;
        subs[s] = SubPath { nodes: first, closed: false };
        subs.insert(s + 1, SubPath { nodes: second, closed: false });
    }
    Some(from_nodes(&subs))
}

/// Joins two loose ends with a straight segment: the two ends of one open
/// line close it; ends of two lines merge them into one.
pub fn join(path: &BezPath, a: usize, b: usize) -> Option<BezPath> {
    let mut subs = to_nodes(path);
    let (sa, ia) = locate(&subs, a)?;
    let (sb, ib) = locate(&subs, b)?;
    let is_end = |s: usize, i: usize| !subs[s].closed && (i == 0 || i + 1 == subs[s].nodes.len());
    if !is_end(sa, ia) || !is_end(sb, ib) || (sa, ia) == (sb, ib) {
        return None;
    }
    if sa == sb {
        subs[sa].closed = true;
        return Some(from_nodes(&subs));
    }
    // Orient: `a` at the end of its line, `b` at the start of its own.
    let reverse = |sp: &mut SubPath| {
        sp.nodes.reverse();
        for n in &mut sp.nodes {
            std::mem::swap(&mut n.handle_in, &mut n.handle_out);
        }
    };
    let mut first = subs[sa].clone();
    let mut second = subs[sb].clone();
    if ia == 0 {
        reverse(&mut first);
    }
    if ib != 0 {
        reverse(&mut second);
    }
    first.nodes.extend(second.nodes);
    let (lo, hi) = (sa.min(sb), sa.max(sb));
    subs.remove(hi);
    subs[lo] = first;
    Some(from_nodes(&subs))
}

/// Merges neighbouring points closer than `distance` into one (Blender's
/// Merge by Distance). With `selected` non-empty, only pairs of selected
/// points merge.
pub fn merge_by_distance(path: &BezPath, selected: &[usize], distance: f64) -> BezPath {
    let subs = to_nodes(path);
    let idx = flat_index(&subs);
    let chosen = |s: usize, i: usize| {
        selected.is_empty() || idx.iter().position(|x| *x == (s, i)).is_some_and(|f| selected.contains(&f))
    };
    let out: Vec<SubPath> = subs
        .iter()
        .enumerate()
        .map(|(s, sp)| {
            let mut nodes: Vec<(usize, Node)> = Vec::new();
            for (i, n) in sp.nodes.iter().enumerate() {
                match nodes.last_mut() {
                    Some((j, last)) if chosen(s, *j) && chosen(s, i) && last.pos.distance(n.pos) <= distance => {
                        // One point halfway; the outer handles follow it.
                        let mid = last.pos.midpoint(n.pos);
                        let (da, db) = (mid - last.pos, mid - n.pos);
                        *last = Node {
                            pos: mid,
                            handle_in: last.handle_in.map(|h| h + da),
                            handle_out: n.handle_out.map(|h| h + db),
                        };
                    }
                    _ => nodes.push((i, *n)),
                }
            }
            // A closed run's last point may sit on its first.
            if sp.closed && nodes.len() > 2 {
                let (j, last) = *nodes.last().unwrap();
                let (k, first) = nodes[0];
                if chosen(s, j) && chosen(s, k) && last.pos.distance(first.pos) <= distance {
                    nodes[0].1.handle_in = last.handle_in.map(|h| h + (first.pos - last.pos));
                    nodes.pop();
                }
            }
            SubPath { nodes: nodes.into_iter().map(|(_, n)| n).collect(), closed: sp.closed }
        })
        .filter(|sp| sp.nodes.len() >= 2)
        .collect();
    from_nodes(&out)
}

/// Cuts off the selected corners (every corner when `selected` is empty):
/// each becomes two points `amount` along its edges, joined by a straight
/// edge, or by a curve when `round` (Blender's Bevel).
pub fn bevel(path: &BezPath, selected: &[usize], amount: f64, round: bool) -> BezPath {
    let subs = to_nodes(path);
    let idx = flat_index(&subs);
    let chosen = |s: usize, i: usize| {
        selected.is_empty() || idx.iter().position(|x| *x == (s, i)).is_some_and(|f| selected.contains(&f))
    };
    let out: Vec<SubPath> = subs
        .iter()
        .enumerate()
        .map(|(s, sp)| {
            let n = sp.nodes.len();
            let mut nodes = Vec::new();
            for (i, node) in sp.nodes.iter().enumerate() {
                let inner = sp.closed || (i > 0 && i + 1 < n);
                let corner = node.handle_in.is_none() && node.handle_out.is_none();
                if !(inner && corner && chosen(s, i) && n >= 2) {
                    nodes.push(*node);
                    continue;
                }
                let prev = sp.nodes[(i + n - 1) % n];
                let next = sp.nodes[(i + 1) % n];
                // Towards the neighbours (their handles if curved), at most halfway.
                let toward_prev = prev.handle_out.unwrap_or(prev.pos) - node.pos;
                let toward_next = next.handle_in.unwrap_or(next.pos) - node.pos;
                let d = amount.min(toward_prev.hypot() / 2.0).min(toward_next.hypot() / 2.0);
                if d <= 1e-9 {
                    nodes.push(*node);
                    continue;
                }
                let a = node.pos + toward_prev.normalize() * d;
                let b = node.pos + toward_next.normalize() * d;
                // 0.55 of the way to the corner: close to a circular arc.
                let k = 0.5523;
                let (ha, hb) =
                    if round { (Some(a + (node.pos - a) * k), Some(b + (node.pos - b) * k)) } else { (None, None) };
                nodes.push(Node { pos: a, handle_in: None, handle_out: ha });
                nodes.push(Node { pos: b, handle_in: hb, handle_out: None });
            }
            SubPath { nodes, closed: sp.closed }
        })
        .collect();
    from_nodes(&out)
}

/// Adds a point beyond a loose end, `offset` from it (Blender's Extrude).
/// Returns the path and the new point's index.
pub fn extrude(path: &BezPath, index: usize, offset: kurbo::Vec2) -> Option<(BezPath, usize)> {
    let mut subs = to_nodes(path);
    let (s, i) = locate(&subs, index)?;
    let sp = &mut subs[s];
    if sp.closed || !(i == 0 || i + 1 == sp.nodes.len()) {
        return None;
    }
    let node = Node::corner(sp.nodes[i].pos + offset);
    let new_index = if i == 0 && sp.nodes.len() > 1 {
        sp.nodes.insert(0, node);
        index
    } else {
        sp.nodes.push(node);
        index + 1
    };
    Some((from_nodes(&subs), new_index))
}

#[cfg(test)]
mod point_editing_tests {
    use super::*;

    fn square() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        p.line_to((0.0, 10.0));
        p.close_path();
        p
    }

    #[test]
    fn merge_bevel_and_extrude() {
        // Two points 0.5 apart on a line merge; far ones don't.
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.5, 0.0));
        p.line_to((20.0, 0.0));
        let merged = merge_by_distance(&p, &[], 1.0);
        assert_eq!(anchors(&merged).len(), 3);
        assert_eq!(anchors(&merge_by_distance(&p, &[0, 1], 1.0)).len(), 4, "only selected pairs");
        // Bevelling every corner of a square: 8 points, still closed, corners cut.
        let b = bevel(&square(), &[], 2.0, false);
        assert_eq!(anchors(&b).len(), 8);
        assert!(kurbo::Shape::area(&b).abs() < 100.0 && kurbo::Shape::area(&b).abs() > 90.0);
        let r = bevel(&square(), &[0], 2.0, true);
        assert_eq!(anchors(&r).len(), 5);
        // Extrude from the end of a line; not from the middle or a closed shape.
        let (e, i) = extrude(&p, 3, kurbo::Vec2::new(5.0, 0.0)).unwrap();
        assert_eq!((anchors(&e).len(), i), (5, 4));
        assert_eq!(anchors(&e)[4].pos, Point::new(25.0, 0.0));
        let (e, i) = extrude(&p, 0, kurbo::Vec2::new(-5.0, 0.0)).unwrap();
        assert_eq!((i, anchors(&e)[0].pos), (0, Point::new(-5.0, 0.0)));
        assert!(extrude(&p, 1, kurbo::Vec2::ZERO).is_none());
        assert!(extrude(&square(), 0, kurbo::Vec2::ZERO).is_none());
    }

    #[test]
    fn subdivide_delete_move_break_and_join() {
        // Every side halved: 8 points; only the top edge: 5.
        assert_eq!(anchors(&subdivide(&square(), &[])).len(), 8);
        let top = subdivide(&square(), &[0, 1]);
        assert_eq!(anchors(&top).len(), 5);
        assert_eq!(anchors(&top)[1].pos, Point::new(5.0, 0.0));
        // Curves are halved on the curve.
        let mut c = BezPath::new();
        c.move_to((0.0, 0.0));
        c.curve_to((0.0, 10.0), (10.0, 10.0), (10.0, 0.0));
        let half = subdivide(&c, &[]);
        assert!((anchors(&half)[1].pos - Point::new(5.0, 7.5)).hypot() < 1e-9);
        // Delete two corners of the square: a 2-point line remains closed.
        assert_eq!(anchors(&delete_nodes(&square(), &[0, 2])).len(), 2);
        let moved = move_nodes(&square(), &[1, 2], kurbo::Vec2::new(5.0, 0.0));
        assert_eq!(anchors(&moved)[2].pos, Point::new(15.0, 10.0));
        // Break the square at a corner: an open line with the corner at both ends.
        let open = break_at(&square(), 1).unwrap();
        let subs = to_nodes(&open);
        assert!(!subs[0].closed && subs[0].nodes.len() == 5);
        assert_eq!(subs[0].nodes[0].pos, subs[0].nodes[4].pos);
        // Join its two ends back: closed again.
        let closed = join(&open, 0, 4).unwrap();
        assert!(to_nodes(&closed)[0].closed);
        // Two separate lines merge into one.
        let mut two = BezPath::new();
        two.move_to((0.0, 0.0));
        two.line_to((1.0, 0.0));
        two.move_to((3.0, 0.0));
        two.line_to((2.0, 0.0));
        let one = join(&two, 1, 3).unwrap();
        let subs = to_nodes(&one);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].nodes.iter().map(|n| n.pos.x).collect::<Vec<_>>(), vec![0.0, 1.0, 2.0, 3.0]);
    }
}
