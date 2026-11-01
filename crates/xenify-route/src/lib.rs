//! Connector routing for diagrams. Knows nothing about Xenify Draw documents:
//! the caller hands in the two ends, the shapes to keep clear of (as
//! rectangles in one shared space) and the style, and gets back a polyline.
//! [`jumps`] then finds where finished routes cross and draws them with hops.
//!
//! - Orthogonal routes run on a visibility grid built from the obstacles'
//!   margins and the ends, searched with A* (length plus a cost per bend).
//! - Straight routes go directly between the ends, clipped to the shapes.
//! - Curved routes take the orthogonal route and smooth it through the
//!   midpoints of its segments, so they still bend around obstacles.

mod jumps;
mod ortho;

pub use jumps::{Hop, JumpStyle, build_path, find_hops, junctions};

use kurbo::{Point, Rect, Vec2};

/// Which way a port faces, out of its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

    /// Unit vector pointing out of the shape.
    pub fn normal(self) -> Vec2 {
        match self {
            Side::Top => Vec2::new(0.0, -1.0),
            Side::Right => Vec2::new(1.0, 0.0),
            Side::Bottom => Vec2::new(0.0, 1.0),
            Side::Left => Vec2::new(-1.0, 0.0),
        }
    }

    /// The middle of this side of `r`.
    pub fn midpoint(self, r: Rect) -> Point {
        let c = r.center();
        match self {
            Side::Top => Point::new(c.x, r.y0),
            Side::Right => Point::new(r.x1, c.y),
            Side::Bottom => Point::new(c.x, r.y1),
            Side::Left => Point::new(r.x0, c.y),
        }
    }

    /// The side of `r` a point on (or near) its edge belongs to, judged by
    /// which edge is closest.
    pub fn nearest(r: Rect, p: Point) -> Side {
        let d = [(p.y - r.y0).abs(), (p.x - r.x1).abs(), (p.y - r.y1).abs(), (p.x - r.x0).abs()];
        let mut best = 0;
        for i in 1..4 {
            if d[i] < d[best] {
                best = i;
            }
        }
        Side::ALL[best]
    }
}

/// One end of a connector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    /// Not attached to anything.
    Free(Point),
    /// Glued to a fixed port on `shape`, facing `side`.
    Port { point: Point, side: Side, shape: Rect },
    /// Glued to the shape as a whole: the router picks the best side.
    Floating(Rect),
}

impl End {
    /// The shape this end is attached to, if any.
    pub fn shape(&self) -> Option<Rect> {
        match *self {
            End::Free(_) => None,
            End::Port { shape, .. } | End::Floating(shape) => Some(shape),
        }
    }

    /// A representative point (the port, or the shape's centre).
    pub fn anchor(&self) -> Point {
        match *self {
            End::Free(p) | End::Port { point: p, .. } => p,
            End::Floating(r) => r.center(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Style {
    #[default]
    Orthogonal,
    Straight,
    Curved,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub style: Style,
    /// Space kept around every obstacle, and the length of the stub that
    /// leaves a port straight out before the route may turn.
    pub margin: f64,
    /// Extra cost of one bend, in the same units as length.
    pub bend_penalty: f64,
    /// Route around obstacles (false = ignore them).
    pub avoid: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { style: Style::Orthogonal, margin: 12.0, bend_penalty: 40.0, avoid: true }
    }
}

/// Everything needed to route one connector.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub from: End,
    pub to: End,
    /// Points the route must pass through, in order.
    pub waypoints: &'a [Point],
    /// Shapes to keep clear of. The ends' own shapes are added automatically.
    pub obstacles: &'a [Rect],
    /// Routes already laid out; orthogonal routes avoid running along them.
    pub occupied: &'a [Vec<Point>],
    pub options: Options,
}

/// Routes one connector. The result starts at the `from` end and finishes
/// at the `to` end, has no repeated or collinear-middle points, and always
/// has at least two points.
pub fn route(req: &Request) -> Vec<Point> {
    let poly = match req.options.style {
        Style::Straight => straight(req),
        Style::Orthogonal | Style::Curved => ortho::route(req),
    };
    simplify(poly)
}

fn straight(req: &Request) -> Vec<Point> {
    let mut points = vec![req.from.anchor()];
    points.extend_from_slice(req.waypoints);
    points.push(req.to.anchor());
    // Floating ends leave their shape where the line to the neighbour
    // crosses its outline.
    let n = points.len();
    if let End::Floating(r) = req.from {
        points[0] = clip_to_rect(r, points[1]);
    }
    if let End::Floating(r) = req.to {
        points[n - 1] = clip_to_rect(r, points[n - 2]);
    }
    points
}

/// Where the line from `r`'s centre towards `toward` leaves `r`.
pub fn clip_to_rect(r: Rect, toward: Point) -> Point {
    let c = r.center();
    let d = toward - c;
    if d.hypot2() < 1e-12 {
        return c;
    }
    let hw = r.width() / 2.0;
    let hh = r.height() / 2.0;
    let tx = if d.x.abs() > 1e-12 { hw / d.x.abs() } else { f64::INFINITY };
    let ty = if d.y.abs() > 1e-12 { hh / d.y.abs() } else { f64::INFINITY };
    c + d * tx.min(ty).min(1.0)
}

/// Removes repeated points and middle points on a straight run.
pub fn simplify(points: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|q| q.distance(p) < 1e-6) {
            continue;
        }
        while out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let cross = (b - a).cross(p - b);
            let same_way = (b - a).dot(p - b) > 0.0;
            if cross.abs() < 1e-6 * (b - a).hypot().max(1.0) * (p - b).hypot().max(1.0) && same_way {
                out.pop();
            } else {
                break;
            }
        }
        out.push(p);
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

#[cfg(test)]
mod tests;
