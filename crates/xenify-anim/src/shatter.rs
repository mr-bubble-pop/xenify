//! The Shatter modifier: a layer's box cut into Voronoi shards that fly
//! apart from an origin. Each shard is a copy of the layer clipped to its
//! cell, so any layer (shapes, text, pictures, groups) can shatter.

use kurbo::{Affine, BezPath, Point, Rect, Vec2};

fn rand(seed: u64, i: usize, c: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Shard centres: a jittered grid over `bounds` (evener than pure random).
fn sites(bounds: Rect, n: usize, seed: u64) -> Vec<Point> {
    let n = n.max(1);
    let aspect = (bounds.width() / bounds.height().max(1e-9)).clamp(0.05, 20.0);
    let cols = ((n as f64 * aspect).sqrt().round() as usize).max(1);
    let rows = n.div_ceil(cols).max(1);
    let (cw, ch) = (bounds.width() / cols as f64, bounds.height() / rows as f64);
    (0..n)
        .map(|i| {
            let (c, r) = (i % cols, i / cols);
            Point::new(
                bounds.x0 + (c as f64 + 0.15 + 0.7 * rand(seed, i, 1)) * cw,
                bounds.y0 + (r as f64 + 0.15 + 0.7 * rand(seed, i, 2)) * ch,
            )
        })
        .collect()
}

/// Keeps the part of `poly` on the side of the line through `m` with normal `n` where `(p - m)·n <= 0`.
fn cut(poly: &[Point], m: Point, n: Vec2) -> Vec<Point> {
    let side = |p: Point| (p - m).dot(n);
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (sa, sb) = (side(a), side(b));
        if sa <= 0.0 {
            out.push(a);
        }
        if (sa <= 0.0) != (sb <= 0.0) {
            let t = sa / (sa - sb);
            out.push(a.lerp(b, t));
        }
    }
    out
}

/// The Voronoi cells of `n` shards over `bounds` (each a convex polygon).
pub fn shards(bounds: Rect, n: usize, seed: u64) -> Vec<Vec<Point>> {
    let s = sites(bounds, n, seed);
    let rect = vec![
        bounds.origin(),
        Point::new(bounds.x1, bounds.y0),
        Point::new(bounds.x1, bounds.y1),
        Point::new(bounds.x0, bounds.y1),
    ];
    s.iter()
        .enumerate()
        .map(|(i, &si)| {
            let mut cell = rect.clone();
            for (j, &sj) in s.iter().enumerate() {
                if i != j && !cell.is_empty() {
                    cell = cut(&cell, si.midpoint(sj), sj - si);
                }
            }
            cell
        })
        .filter(|c| c.len() >= 3)
        .collect()
}

pub fn area(poly: &[Point]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i].to_vec2().cross(poly[(i + 1) % n].to_vec2())).sum::<f64>().abs() / 2.0
}

fn centroid(poly: &[Point]) -> Point {
    let n = poly.len() as f64;
    let sum = poly.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2());
    (sum / n).to_point()
}

/// How the shards fly (all in layer space).
#[derive(Clone, Copy, Debug)]
pub struct Flight {
    pub pieces: usize,
    pub origin: Point,
    /// Outward speed, pixels per second.
    pub force: f64,
    /// Extra push (pixels per second) towards `direction` degrees.
    pub push: f64,
    pub direction: f64,
    pub gravity: f64,
    /// Largest turn, degrees per second.
    pub spin: f64,
    /// Seconds of flight (0: together).
    pub progress: f64,
    pub seed: u64,
}

/// Each shard's placement (layer space), opacity and clip, for the modifier
/// stack. Nothing at progress 0: the layer shows whole, without seams.
pub fn copies(bounds: Rect, f: &Flight) -> Vec<(Affine, f64, Option<BezPath>)> {
    if f.progress.abs() < 1e-6 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Vec::new();
    }
    let t = f.progress.max(0.0);
    let push = Vec2::from_angle(f.direction.to_radians()) * f.push;
    shards(bounds, f.pieces.clamp(1, 400), f.seed)
        .into_iter()
        .enumerate()
        .map(|(i, cell)| {
            let c = centroid(&cell);
            let away = c - f.origin;
            let dir = if away.hypot() > 1e-9 {
                away.normalize()
            } else {
                Vec2::from_angle(rand(f.seed, i, 3) * std::f64::consts::TAU)
            };
            // Nearer pieces fly faster, like a real blast.
            let near = 1.0 / (1.0 + away.hypot() / bounds.width().max(bounds.height()).max(1.0));
            let v = dir * f.force * (0.5 + near) * (0.7 + 0.6 * rand(f.seed, i, 4)) + push;
            let offset = v * t + Vec2::new(0.0, 0.5 * f.gravity * t * t);
            let turn = f.spin * (rand(f.seed, i, 5) * 2.0 - 1.0) * t;
            let m = Affine::translate(offset)
                * Affine::translate(c.to_vec2())
                * Affine::rotate(turn.to_radians())
                * Affine::translate(-c.to_vec2());
            let mut path = BezPath::new();
            path.move_to(cell[0]);
            for p in &cell[1..] {
                path.line_to(*p);
            }
            path.close_path();
            (m, 1.0, Some(path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shards_tile_the_box_exactly() {
        let b = Rect::new(-100.0, -50.0, 300.0, 150.0);
        for n in [1, 7, 30, 100] {
            let cells = shards(b, n, 3);
            assert_eq!(cells.len(), n);
            let total: f64 = cells.iter().map(|c| area(c)).sum();
            assert!((total - b.area()).abs() < 1e-6 * b.area(), "{n}: {total} vs {}", b.area());
        }
    }

    #[test]
    fn pieces_fly_away_from_the_origin_and_come_back_at_zero() {
        let b = Rect::new(0.0, 0.0, 200.0, 100.0);
        let f = Flight {
            pieces: 12,
            origin: Point::new(100.0, 50.0),
            force: 300.0,
            push: 0.0,
            direction: 0.0,
            gravity: 0.0,
            spin: 0.0,
            progress: 1.0,
            seed: 1,
        };
        for (m, _, clip) in copies(b, &f) {
            let c = centroid(&clip.unwrap().elements().iter().filter_map(|e| e.end_point()).collect::<Vec<_>>());
            let moved = m * c;
            assert!(moved.distance(f.origin) > c.distance(f.origin), "away from the origin");
        }
        assert!(copies(b, &Flight { progress: 0.0, ..f }).is_empty(), "whole again at 0");
    }
}
