//! Corner-pin distortion (perspective): a layer's content is mapped from a
//! source rectangle onto four freely placed corners by a projective transform.

use crate::{Animatable, Property};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

/// Four corners: top-left, top-right, bottom-right, bottom-left.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quad(pub [Vec2; 4]);

impl Quad {
    /// The corners of `r` (clockwise from top-left, y down).
    pub fn of_rect(r: Rect) -> Self {
        Quad([Vec2::new(r.x0, r.y0), Vec2::new(r.x1, r.y0), Vec2::new(r.x1, r.y1), Vec2::new(r.x0, r.y1)])
    }

    pub fn points(&self) -> [Point; 4] {
        self.0.map(|v| v.to_point())
    }

    pub fn map(&self, f: impl Fn(Point) -> Point) -> Self {
        Quad(self.0.map(|v| f(v.to_point()).to_vec2()))
    }
}

impl Animatable for Quad {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        Quad(std::array::from_fn(|i| a.0[i].lerp(b.0[i], t)))
    }
}

/// A layer's corner pin, in layer space: content inside `source` is stretched
/// so the rectangle's corners land on `corners`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Distort {
    pub source: Rect,
    pub corners: Property<Quad>,
}

impl Distort {
    /// No distortion yet: the corners sit on the source rectangle.
    pub fn identity(source: Rect) -> Self {
        Self { source, corners: Quad::of_rect(source).into() }
    }

    /// The layer-space mapping at `time`.
    pub fn homography_at(&self, time: f64) -> Homography {
        Homography::from_quads(&Quad::of_rect(self.source), &self.corners.value_at(time)).unwrap_or_default()
    }
}

/// A 2D projective transform (3×3 matrix, row-major, `m[8]` normalised to 1
/// where possible). Straight lines stay straight; parallel lines may meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [f64; 9]);

impl Default for Homography {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl std::ops::Mul for Homography {
    type Output = Homography;
    fn mul(self, o: Homography) -> Homography {
        let (a, b) = (self.0, o.0);
        Homography(std::array::from_fn(|i| {
            let (r, c) = (i / 3, i % 3);
            (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum()
        }))
    }
}

impl std::ops::Mul<Point> for Homography {
    type Output = Point;
    fn mul(self, p: Point) -> Point {
        let m = self.0;
        let w = m[6] * p.x + m[7] * p.y + m[8];
        let w = if w.abs() < 1e-12 { 1e-12f64.copysign(w) } else { w };
        Point::new((m[0] * p.x + m[1] * p.y + m[2]) / w, (m[3] * p.x + m[4] * p.y + m[5]) / w)
    }
}

impl From<Affine> for Homography {
    fn from(a: Affine) -> Self {
        let [a, b, c, d, e, f] = a.as_coeffs();
        Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])
    }
}

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    /// Whether this is (numerically) the identity.
    pub fn is_identity(&self) -> bool {
        self.0.iter().zip(Self::IDENTITY.0).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    /// Whether this is an affine map (no perspective).
    pub fn is_affine(&self) -> bool {
        self.0[6].abs() < 1e-12 && self.0[7].abs() < 1e-12
    }

    /// The map taking the unit square's corners (0,0) (1,0) (1,1) (0,1) to `q`.
    fn square_to(q: &[Point; 4]) -> Option<Homography> {
        let [p0, p1, p2, p3] = *q;
        let (dx1, dy1) = (p1.x - p2.x, p1.y - p2.y);
        let (dx2, dy2) = (p3.x - p2.x, p3.y - p2.y);
        let (sx, sy) = (p0.x - p1.x + p2.x - p3.x, p0.y - p1.y + p2.y - p3.y);
        let det = dx1 * dy2 - dx2 * dy1;
        if det.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / det;
        let h = (dx1 * sy - sx * dy1) / det;
        Some(Homography([
            p1.x - p0.x + g * p1.x,
            p3.x - p0.x + h * p3.x,
            p0.x,
            p1.y - p0.y + g * p1.y,
            p3.y - p0.y + h * p3.y,
            p0.y,
            g,
            h,
            1.0,
        ]))
    }

    /// The map taking quad `from`'s corners onto `to`'s, if both are proper quads.
    pub fn from_quads(from: &Quad, to: &Quad) -> Option<Homography> {
        let a = Self::square_to(&from.points())?;
        let b = Self::square_to(&to.points())?;
        Some(b * a.inverse()?)
    }

    pub fn inverse(&self) -> Option<Homography> {
        let m = self.0;
        let c = |a: usize, b: usize, c: usize, d: usize| m[a] * m[b] - m[c] * m[d];
        let inv = [
            c(4, 8, 5, 7),
            c(2, 7, 1, 8),
            c(1, 5, 2, 4),
            c(5, 6, 3, 8),
            c(0, 8, 2, 6),
            c(2, 3, 0, 5),
            c(3, 7, 4, 6),
            c(1, 6, 0, 7),
            c(0, 4, 1, 3),
        ];
        let det = m[0] * inv[0] + m[1] * inv[3] + m[2] * inv[6];
        if det.abs() < 1e-15 {
            return None;
        }
        let mut out = inv.map(|v| v / det);
        if out[8].abs() > 1e-12 {
            let s = out[8];
            out = out.map(|v| v / s);
        }
        Some(Homography(out))
    }

    /// Maps a path. Lines stay exact; curves are split so the result stays
    /// within a fraction of a pixel of the true (rational) curve.
    pub fn map_path(&self, path: &BezPath) -> BezPath {
        if self.is_affine() {
            let m = self.0;
            return Affine::new([m[0], m[3], m[1], m[4], m[2], m[5]]) * path;
        }
        let h = *self;
        let mut out = BezPath::new();
        let mut last = Point::ZERO;
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(p) => {
                    out.move_to(h * p);
                    last = p;
                }
                PathEl::LineTo(p) => {
                    out.line_to(h * p);
                    last = p;
                }
                PathEl::QuadTo(a, p) => {
                    let c = kurbo::QuadBez::new(last, a, p).raise();
                    h.push_cubic(&mut out, c);
                    last = p;
                }
                PathEl::CurveTo(a, b, p) => {
                    h.push_cubic(&mut out, kurbo::CubicBez::new(last, a, b, p));
                    last = p;
                }
                PathEl::ClosePath => out.close_path(),
            }
        }
        out
    }

    fn push_cubic(&self, out: &mut BezPath, c: kurbo::CubicBez) {
        use kurbo::ParamCurve;
        // More pieces where the curve is long on screen.
        let len = (*self * c.p0).distance(*self * c.p3)
            + (*self * c.p1).distance(*self * c.p0)
            + (*self * c.p2).distance(*self * c.p3);
        let n = ((len / 40.0).ceil() as usize).clamp(2, 32);
        for i in 0..n {
            let s = c.subsegment(i as f64 / n as f64..(i + 1) as f64 / n as f64);
            out.curve_to(*self * s.p1, *self * s.p2, *self * s.p3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_corners_exactly_and_inverts() {
        let from = Quad::of_rect(Rect::new(-50.0, -20.0, 50.0, 20.0));
        let to = Quad([Vec2::new(-30.0, -40.0), Vec2::new(60.0, -10.0), Vec2::new(70.0, 30.0), Vec2::new(-50.0, 25.0)]);
        let h = Homography::from_quads(&from, &to).unwrap();
        for (a, b) in from.points().iter().zip(to.points()) {
            assert!((h * *a).distance(b) < 1e-9);
        }
        let back = h.inverse().unwrap();
        let p = Point::new(12.0, -7.0);
        assert!((back * (h * p)).distance(p) < 1e-9);
        assert!(Homography::from_quads(&from, &from).unwrap().is_identity());
        // Degenerate target.
        assert!(Homography::from_quads(&from, &Quad([Vec2::ZERO; 4])).is_none());
    }

    #[test]
    fn affine_round_trip_and_path_mapping() {
        let a = Affine::translate((5.0, 7.0)) * Affine::rotate(0.3) * Affine::scale_non_uniform(2.0, 0.5);
        let h = Homography::from(a);
        let p = Point::new(3.0, -4.0);
        assert!((h * p).distance(a * p) < 1e-9);
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.curve_to((10.0, 0.0), (20.0, 10.0), (20.0, 20.0));
        path.close_path();
        let persp = Homography::from_quads(
            &Quad::of_rect(Rect::new(0.0, 0.0, 20.0, 20.0)),
            &Quad([Vec2::new(0.0, 0.0), Vec2::new(40.0, 5.0), Vec2::new(40.0, 35.0), Vec2::new(0.0, 40.0)]),
        )
        .unwrap();
        let mapped = persp.map_path(&path);
        // End points land exactly.
        let last = mapped.elements().iter().rev().find_map(|e| e.end_point()).unwrap();
        assert!(last.distance(persp * Point::new(20.0, 20.0)) < 1e-9);
    }
}
