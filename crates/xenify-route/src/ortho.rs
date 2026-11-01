//! Orthogonal routing: A* over a sparse visibility grid. The grid's lines
//! are the obstacles' edges pushed out by the margin plus the lines through
//! every end and waypoint, so any shortest, fewest-bends route between
//! axis-aligned boxes lies on it. An edge between neighbouring grid points is
//! blocked when it runs through the inside of an (inflated) obstacle; since
//! the obstacle's own edges are grid lines, each edge is either wholly inside
//! or wholly outside.

use super::{End, Request, Side};
use kurbo::{Point, Rect};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// Grids bigger than this aren't searched; the route falls back to a
/// simple elbow.
const MAX_NODES: usize = 250_000;
/// Running along an existing route costs this much extra per unit length.
const OVERLAP_COST: f64 = 0.5;
const EPS: f64 = 1e-6;

/// Directions of travel: +x, +y, -x, -y. 4 = not moving yet.
type Dir = u8;
const NONE: Dir = 4;

fn side_dir(s: Side) -> Dir {
    match s {
        Side::Right => 0,
        Side::Bottom => 1,
        Side::Left => 2,
        Side::Top => 3,
    }
}

fn opposite(d: Dir) -> Dir {
    (d + 2) % 4
}

/// Where a leg may start: the grid point, the direction already travelling,
/// the cost so far and the points before it (the port, before its stub).
#[derive(Clone, Debug)]
struct Source {
    at: Point,
    dir: Dir,
    cost: f64,
    before: Vec<Point>,
}

/// Where a leg may finish: the grid point, the direction the route has to
/// arrive in (NONE = any) and the points after it.
#[derive(Clone, Debug)]
struct Target {
    at: Point,
    dir: Dir,
    after: Vec<Point>,
}

/// Where a port's stub ends: `margin` out from the port, and at least clear of its shape's box (a port on a
/// turned shape is inside the box around it, and a route can't start inside an obstacle).
fn stub_end(point: Point, side: Side, shape: Option<Rect>, margin: f64) -> Point {
    let mut at = point + side.normal() * margin;
    if let Some(r) = shape.map(|r| r.abs()) {
        match side {
            Side::Right => at.x = at.x.max(r.x1 + margin),
            Side::Left => at.x = at.x.min(r.x0 - margin),
            Side::Bottom => at.y = at.y.max(r.y1 + margin),
            Side::Top => at.y = at.y.min(r.y0 - margin),
        }
    }
    at
}

fn sources(end: End, margin: f64) -> Vec<Source> {
    let stub = |point: Point, side: Side| Source {
        at: stub_end(point, side, end.shape(), margin),
        dir: side_dir(side),
        cost: margin,
        before: vec![point],
    };
    match end {
        End::Free(p) => vec![Source { at: p, dir: NONE, cost: 0.0, before: vec![] }],
        End::Port { point, side, .. } => vec![stub(point, side)],
        End::Floating(r) => Side::ALL.iter().map(|&s| stub(s.midpoint(r), s)).collect(),
    }
}

fn targets(end: End, margin: f64) -> Vec<Target> {
    let stub = |point: Point, side: Side| Target {
        at: stub_end(point, side, end.shape(), margin),
        dir: opposite(side_dir(side)),
        after: vec![point],
    };
    match end {
        End::Free(p) => vec![Target { at: p, dir: NONE, after: vec![] }],
        End::Port { point, side, .. } => vec![stub(point, side)],
        End::Floating(r) => Side::ALL.iter().map(|&s| stub(s.midpoint(r), s)).collect(),
    }
}

pub(crate) fn route(req: &Request) -> Vec<Point> {
    let margin = req.options.margin.max(0.0);
    let mut obstacles: Vec<Rect> = if req.options.avoid { req.obstacles.to_vec() } else { vec![] };
    obstacles.extend(req.from.shape());
    obstacles.extend(req.to.shape());
    let obstacles: Vec<Rect> = obstacles.iter().map(|r| r.abs().inflate(margin, margin)).collect();

    let from = sources(req.from, margin);
    let to = targets(req.to, margin);

    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for r in &obstacles {
        xs.extend([r.x0, r.x1]);
        ys.extend([r.y0, r.y1]);
    }
    let mut add = |p: Point| {
        xs.push(p.x);
        ys.push(p.y);
    };
    from.iter().for_each(|s| add(s.at));
    to.iter().for_each(|t| add(t.at));
    req.waypoints.iter().for_each(|&p| add(p));
    let grid = Grid::new(xs, ys, &obstacles, req.occupied);

    // One leg per stretch between waypoints; each leg starts in the
    // direction the last one arrived in.
    let mut out = Vec::new();
    let mut legs_from = from;
    for (i, &w) in req.waypoints.iter().chain(std::iter::once(&Point::ZERO)).enumerate() {
        let last = i == req.waypoints.len();
        let leg_to = if last { to.clone() } else { vec![Target { at: w, dir: NONE, after: vec![] }] };
        let (points, arrived) = match &grid {
            Some(g) => g.search(&legs_from, &leg_to, req.options.bend_penalty),
            None => None,
        }
        .unwrap_or_else(|| elbow(&legs_from[0], &leg_to[0]));
        // Waypoint legs overlap at the waypoint itself.
        if !out.is_empty() && !points.is_empty() {
            out.pop();
        }
        out.extend(points);
        if !last {
            legs_from = vec![Source { at: w, dir: arrived, cost: 0.0, before: vec![] }];
        }
    }
    out
}

/// The plain route used when the grid is too big or has no way through:
/// out along the first stub, one turn, and in along the last.
fn elbow(s: &Source, t: &Target) -> (Vec<Point>, Dir) {
    let mut out = s.before.clone();
    out.push(s.at);
    let horizontal_first = match s.dir {
        0 | 2 => true,
        1 | 3 => false,
        _ => !matches!(t.dir, 1 | 3),
    };
    let corner = if horizontal_first { Point::new(t.at.x, s.at.y) } else { Point::new(s.at.x, t.at.y) };
    out.push(corner);
    out.push(t.at);
    out.extend(t.after.iter().copied());
    let d = t.at - corner;
    let arrived = if d.x.abs() > d.y.abs() {
        if d.x > 0.0 { 0 } else { 2 }
    } else if d.y.abs() > EPS {
        if d.y > 0.0 { 1 } else { 3 }
    } else {
        s.dir
    };
    (out, arrived)
}

struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// Blocked edges from node (i, j) to (i + 1, j).
    h_blocked: Vec<bool>,
    /// Blocked edges from node (i, j) to (i, j + 1).
    v_blocked: Vec<bool>,
    /// Existing routes' horizontal runs keyed by their y, and vertical ones
    /// by their x.
    busy_h: HashMap<i64, Vec<(f64, f64)>>,
    busy_v: HashMap<i64, Vec<(f64, f64)>>,
}

fn sorted_unique(mut v: Vec<f64>) -> Vec<f64> {
    v.retain(|x| x.is_finite());
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < EPS);
    v
}

fn index_of(v: &[f64], x: f64) -> Option<usize> {
    let i = v.partition_point(|&a| a < x - EPS);
    (i < v.len() && (v[i] - x).abs() < EPS).then_some(i)
}

fn key(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

impl Grid {
    fn new(xs: Vec<f64>, ys: Vec<f64>, obstacles: &[Rect], occupied: &[Vec<Point>]) -> Option<Grid> {
        let xs = sorted_unique(xs);
        let ys = sorted_unique(ys);
        let (nx, ny) = (xs.len(), ys.len());
        if nx == 0 || ny == 0 || nx * ny > MAX_NODES {
            return None;
        }
        let mut h_blocked = vec![false; nx * ny];
        let mut v_blocked = vec![false; nx * ny];
        for r in obstacles {
            let (Some(i0), Some(i1), Some(j0), Some(j1)) =
                (index_of(&xs, r.x0), index_of(&xs, r.x1), index_of(&ys, r.y0), index_of(&ys, r.y1))
            else {
                continue;
            };
            for j in j0 + 1..j1 {
                for i in i0..i1 {
                    h_blocked[j * nx + i] = true;
                }
            }
            for j in j0..j1 {
                for i in i0 + 1..i1 {
                    v_blocked[j * nx + i] = true;
                }
            }
        }
        let mut busy_h: HashMap<i64, Vec<(f64, f64)>> = HashMap::new();
        let mut busy_v: HashMap<i64, Vec<(f64, f64)>> = HashMap::new();
        for line in occupied {
            for w in line.windows(2) {
                let (a, b) = (w[0], w[1]);
                if (a.y - b.y).abs() < EPS {
                    busy_h.entry(key(a.y)).or_default().push((a.x.min(b.x), a.x.max(b.x)));
                } else if (a.x - b.x).abs() < EPS {
                    busy_v.entry(key(a.x)).or_default().push((a.y.min(b.y), a.y.max(b.y)));
                }
            }
        }
        Some(Grid { xs, ys, h_blocked, v_blocked, busy_h, busy_v })
    }

    fn node(&self, p: Point) -> Option<usize> {
        Some(index_of(&self.ys, p.y)? * self.xs.len() + index_of(&self.xs, p.x)?)
    }

    fn point(&self, n: usize) -> Point {
        let nx = self.xs.len();
        Point::new(self.xs[n % nx], self.ys[n / nx])
    }

    /// The neighbour of `n` going `d`, if the edge there is open.
    fn step(&self, n: usize, d: Dir) -> Option<usize> {
        let nx = self.xs.len();
        let (i, j) = (n % nx, n / nx);
        match d {
            0 if i + 1 < nx && !self.h_blocked[n] => Some(n + 1),
            2 if i > 0 && !self.h_blocked[n - 1] => Some(n - 1),
            1 if j + 1 < self.ys.len() && !self.v_blocked[n] => Some(n + nx),
            3 if j > 0 && !self.v_blocked[n - nx] => Some(n - nx),
            _ => None,
        }
    }

    fn overlap(&self, a: Point, b: Point, d: Dir) -> bool {
        let (runs, along) = if d.is_multiple_of(2) {
            (self.busy_h.get(&key(a.y)), (a.x + b.x) / 2.0)
        } else {
            (self.busy_v.get(&key(a.x)), (a.y + b.y) / 2.0)
        };
        runs.is_some_and(|runs| runs.iter().any(|&(lo, hi)| along > lo && along < hi))
    }

    /// Cheapest route from any source to any target. Returns the points
    /// (including the sources' and targets' extra points) and the direction
    /// it arrived in.
    fn search(&self, from: &[Source], to: &[Target], bend: f64) -> Option<(Vec<Point>, Dir)> {
        let states = self.xs.len() * self.ys.len() * 5;
        let mut best = vec![f64::INFINITY; states];
        let mut parent = vec![u32::MAX; states];
        let mut start_of = HashMap::new();
        let mut heap = BinaryHeap::new();
        let goals: Vec<(usize, &Target)> = to.iter().filter_map(|t| Some((self.node(t.at)?, t))).collect();
        if goals.is_empty() {
            return None;
        }
        let h = |n: usize| {
            let p = self.point(n);
            goals.iter().map(|(_, t)| (t.at.x - p.x).abs() + (t.at.y - p.y).abs()).fold(f64::INFINITY, f64::min)
        };
        for (k, s) in from.iter().enumerate() {
            let Some(n) = self.node(s.at) else { continue };
            let st = n * 5 + s.dir as usize;
            if s.cost < best[st] {
                best[st] = s.cost;
                start_of.insert(st, k);
                heap.push(Item { f: s.cost + h(n), g: s.cost, state: st, goal: None });
            }
        }
        while let Some(Item { g, state, goal, .. }) = heap.pop() {
            if let Some(gi) = goal {
                let target = goals[gi].1;
                let mut nodes = vec![state];
                let mut s = state;
                while parent[s] != u32::MAX {
                    s = parent[s] as usize;
                    nodes.push(s);
                }
                nodes.reverse();
                let src = &from[start_of[&s]];
                let mut out = src.before.clone();
                out.extend(nodes.iter().map(|&s| self.point(s / 5)));
                out.extend(target.after.iter().copied());
                let arrived = (state % 5) as Dir;
                return Some((out, if target.dir == NONE { arrived } else { target.dir }));
            }
            if g > best[state] {
                continue;
            }
            let n = state / 5;
            let dir = (state % 5) as Dir;
            for (gi, (gn, t)) in goals.iter().enumerate() {
                if *gn == n {
                    let extra = turn_cost(dir, t.dir, bend);
                    heap.push(Item { f: g + extra, g: g + extra, state, goal: Some(gi) });
                }
            }
            for d in 0..4u8 {
                if dir != NONE && d == opposite(dir) {
                    continue;
                }
                let Some(m) = self.step(n, d) else { continue };
                let (a, b) = (self.point(n), self.point(m));
                let len = a.distance(b);
                let mut cost = g + len + if dir != NONE && d != dir { bend } else { 0.0 };
                if self.overlap(a, b, d) {
                    cost += len * OVERLAP_COST;
                }
                let st = m * 5 + d as usize;
                if cost < best[st] - 1e-9 {
                    best[st] = cost;
                    parent[st] = state as u32;
                    heap.push(Item { f: cost + h(m), g: cost, state: st, goal: None });
                }
            }
        }
        None
    }
}

/// Bends needed to go from travelling `from` to travelling `to`.
fn turn_cost(from: Dir, to: Dir, bend: f64) -> f64 {
    if from == NONE || to == NONE || from == to {
        0.0
    } else if from == opposite(to) {
        2.0 * bend
    } else {
        bend
    }
}

struct Item {
    f: f64,
    g: f64,
    state: usize,
    goal: Option<usize>,
}

impl PartialEq for Item {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    /// Smallest f first (BinaryHeap is a max-heap); on ties, finished goals
    /// first, then the deeper state, then the lower state number, so
    /// results never depend on hash order.
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f)
            .then_with(|| self.goal.is_some().cmp(&o.goal.is_some()))
            .then_with(|| self.g.total_cmp(&o.g))
            .then_with(|| o.state.cmp(&self.state))
    }
}
