//! Liquid fills: the water's body (surface with waves, tilt and slosh, down
//! to the bottom of its box), foam along the surface and rising bubbles,
//! all in the layer's own space. The box clips it (as a mask).

use crate::{DrawItem, SceneStroke};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use xenify_doc::{Color, FillRule, Gradient, GradientKind, GradientStop, LiquidFill};

/// Samples along the surface.
const STEPS: usize = 96;

fn rand(seed: u64, i: usize, c: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// The container box, in layer space.
pub fn container(w: &LiquidFill) -> Rect {
    Rect::from_center_size(Point::ZERO, w.size.to_size())
}

/// Layer space → a frame where the liquid rises towards -y.
fn frame(w: &LiquidFill) -> Affine {
    // The rise direction turns to -y (pointing up).
    Affine::rotate(-(w.direction + 90.0).to_radians())
}

/// The surface's height (frame y, where -y is up) at frame x and `time`.
fn surface_at(w: &LiquidFill, base: f64, x: f64, time: f64) -> f64 {
    let tilt = w.tilt.value_at(time) + w.slosh * (std::f64::consts::TAU * w.slosh_speed * time).sin();
    let mut y = base + x * tilt.to_radians().tan();
    // Waves calm down as the container empties (none at all when it's empty).
    let height = w.wave_height.value_at(time) * (w.level.value_at(time).clamp(0.0, 1.0) * 20.0).min(1.0);
    for k in 0..w.waves.min(6) {
        // Each wave train shorter, lower and a little faster than the last, drifting either way.
        let f = 1.0 + k as f64 * 0.73;
        let len = (w.wave_length / f).max(1.0);
        let dir = if k % 2 == 0 { 1.0 } else { -1.0 };
        let phase = rand(w.seed, k as usize, 1) * std::f64::consts::TAU;
        y += height / f * (std::f64::consts::TAU * (x / len - dir * w.wave_speed * f * time) + phase).sin();
    }
    y
}

/// The water at `time`, in layer space: body, foam and bubbles. Empty when there's no water.
pub fn liquid_items(w: &LiquidFill, time: f64) -> Vec<DrawItem> {
    let level = w.level.value_at(time).clamp(0.0, 1.0);
    let bx = container(w);
    let to_frame = frame(w);
    let from_frame = to_frame.inverse();
    // The box's extent in the frame.
    let corners = [bx.origin(), Point::new(bx.x1, bx.y0), Point::new(bx.x1, bx.y1), Point::new(bx.x0, bx.y1)];
    let fr = corners
        .iter()
        .fold(Rect::from_points(to_frame * corners[0], to_frame * corners[0]), |r, p| r.union_pt(to_frame * *p));
    if level <= 0.0 {
        return Vec::new();
    }
    let reach = w.wave_height.value_at(time).abs() * 2.0 + 2.0;
    let base = fr.y1 - level * fr.height();
    let (x0, x1) = (fr.x0 - reach, fr.x1 + reach);
    let bottom = fr.y1 + reach;
    let surface: Vec<Point> = (0..=STEPS)
        .map(|i| {
            let x = x0 + (x1 - x0) * i as f64 / STEPS as f64;
            // At level 0 the waves sink below the bottom; at 1 they rise above the brim.
            Point::new(x, surface_at(w, base, x, time).min(bottom))
        })
        .collect();
    // The body, cut to the box (so every export shows it contained).
    let mut outline: Vec<Point> = surface.iter().map(|p| from_frame * *p).collect();
    outline.push(from_frame * Point::new(x1, bottom));
    outline.push(from_frame * Point::new(x0, bottom));
    let clipped = clip_polygon(&outline, bx);
    if clipped.len() < 3 {
        return Vec::new();
    }
    let mut body = BezPath::new();
    body.move_to(clipped[0]);
    for p in &clipped[1..] {
        body.line_to(*p);
    }
    body.close_path();

    let colour = w.color.value_at(time);
    let gradient = w.deep_color.map(|deep| Gradient {
        kind: GradientKind::Linear,
        start: (from_frame * Point::new(0.0, base)).to_vec2(),
        end: (from_frame * Point::new(0.0, fr.y1)).to_vec2(),
        stops: vec![GradientStop { offset: 0.0, color: colour }, GradientStop { offset: 1.0, color: deep }],
    });
    let mut out = vec![DrawItem {
        path: body,
        fill_rule: FillRule::NonZero,
        gradient,
        fill: Some(colour),
        stroke: None,
        pattern: None,
    }];

    // Bubbles: rising from the bottom, wobbling, popping at the surface.
    let depth = (fr.y1 - base).max(0.0);
    if depth > 1.0 {
        for i in 0..w.bubbles.min(500) as usize {
            let speed = 60.0 + rand(w.seed, i, 2) * 120.0;
            let travel = depth + 20.0;
            let y = fr.y1 - ((time * speed + rand(w.seed, i, 3) * travel) % travel);
            let x = fr.x0 + rand(w.seed, i, 4) * fr.width() + (time * 3.0 + i as f64).sin() * 6.0;
            let r = 2.0 + rand(w.seed, i, 5) * 6.0;
            if y - r < surface_at(w, base, x, time) + 1.0 || !bx.inset(-r).contains(from_frame * Point::new(x, y)) {
                continue;
            }
            out.push(DrawItem {
                path: from_frame * kurbo::Circle::new((x, y), r).to_path(0.1),
                fill_rule: FillRule::NonZero,
                gradient: None,
                fill: Some(Color::rgba(1.0, 1.0, 1.0, 0.15)),
                stroke: Some(SceneStroke::solid(
                    Color::rgba(1.0, 1.0, 1.0, 0.55),
                    1.2,
                    xenify_doc::LineJoin::Round,
                    xenify_doc::LineCap::Round,
                )),
                pattern: None,
            });
        }
    }

    // Foam along the surface.
    if w.foam > 0.0 && level > 0.0 && level < 1.0 {
        let mut line = BezPath::new();
        let mut open = false;
        for pair in surface.windows(2) {
            let (a, b) = (from_frame * pair[0], from_frame * pair[1]);
            match clip_segment(a, b, bx) {
                Some((c, d)) => {
                    if !open {
                        line.move_to(c);
                        open = true;
                    }
                    line.line_to(d);
                }
                None => open = false,
            }
        }
        out.push(DrawItem {
            path: line,
            fill_rule: FillRule::NonZero,
            gradient: None,
            fill: None,
            stroke: Some(SceneStroke::solid(
                Color::rgba(1.0, 1.0, 1.0, 0.75),
                w.foam,
                xenify_doc::LineJoin::Round,
                xenify_doc::LineCap::Round,
            )),
            pattern: None,
        });
    }
    out
}

/// `poly` cut to `r` (Sutherland–Hodgman).
fn clip_polygon(poly: &[Point], r: Rect) -> Vec<Point> {
    type Inside = fn(Point, Rect) -> bool;
    type Cut = fn(Point, Point, Rect) -> Point;
    let edges: [(Inside, Cut); 4] = [
        (|p, r| p.x >= r.x0, |a, b, r| lerp_x(a, b, r.x0)),
        (|p, r| p.x <= r.x1, |a, b, r| lerp_x(a, b, r.x1)),
        (|p, r| p.y >= r.y0, |a, b, r| lerp_y(a, b, r.y0)),
        (|p, r| p.y <= r.y1, |a, b, r| lerp_y(a, b, r.y1)),
    ];
    let mut out = poly.to_vec();
    for (inside, cut) in edges {
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else { break };
        let mut prev = last;
        for p in input {
            match (inside(p, r), inside(prev, r)) {
                (true, true) => out.push(p),
                (true, false) => {
                    out.push(cut(prev, p, r));
                    out.push(p);
                }
                (false, true) => out.push(cut(prev, p, r)),
                (false, false) => {}
            }
            prev = p;
        }
    }
    out
}

fn lerp_x(a: Point, b: Point, x: f64) -> Point {
    let t = if (b.x - a.x).abs() > 1e-12 { (x - a.x) / (b.x - a.x) } else { 0.0 };
    Point::new(x, a.y + (b.y - a.y) * t)
}

fn lerp_y(a: Point, b: Point, y: f64) -> Point {
    let t = if (b.y - a.y).abs() > 1e-12 { (y - a.y) / (b.y - a.y) } else { 0.0 };
    Point::new(a.x + (b.x - a.x) * t, y)
}

/// The part of segment `a`–`b` inside `r` (Liang–Barsky).
fn clip_segment(a: Point, b: Point, r: Rect) -> Option<(Point, Point)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-d.x, a.x - r.x0), (d.x, r.x1 - a.x), (-d.y, a.y - r.y0), (d.y, r.y1 - a.y)] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 <= t1).then(|| (a + d * t0, a + d * t1))
}

/// The level (0..1) that puts the surface through `p` (layer space).
pub fn level_for(w: &LiquidFill, p: Point) -> f64 {
    let to_frame = frame(w);
    let fr = to_frame.transform_rect_bbox(container(w));
    ((fr.y1 - (to_frame * p).y) / fr.height().max(1e-9)).clamp(0.0, 1.0)
}

/// Where the surface is (layer space), for handles: its middle point.
pub fn surface_middle(w: &LiquidFill, time: f64) -> Point {
    let to_frame = frame(w);
    let bx = container(w);
    let fr = to_frame.transform_rect_bbox(bx);
    let base = fr.y1 - w.level.value_at(time).clamp(0.0, 1.0) * fr.height();
    to_frame.inverse() * Point::new(fr.center().x, surface_at(w, base, fr.center().x, time))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm(level: f64) -> LiquidFill {
        LiquidFill {
            level: level.into(),
            wave_height: 0.0.into(),
            bubbles: 0,
            foam: 0.0,
            ..LiquidFill::water(200.0, 100.0)
        }
    }

    #[test]
    fn the_level_sets_how_high_the_water_is() {
        for (level, top) in [(0.25, 25.0), (0.5, 0.0), (1.0, -50.0)] {
            let items = liquid_items(&calm(level), 0.0);
            let b = items[0].path.bounding_box().intersect(container(&calm(level)));
            assert!((b.y0 - top).abs() < 1e-6, "level {level}: top {}", b.y0);
            assert!((b.height() - 100.0 * level).abs() < 1e-6);
        }
        assert!(liquid_items(&calm(0.0), 0.0).is_empty());
    }

    #[test]
    fn it_rises_the_way_it_is_told() {
        // Rising to the right: the water is on the left.
        let w = LiquidFill { direction: 0.0, ..calm(0.5) };
        let b = liquid_items(&w, 0.0)[0].path.bounding_box().intersect(container(&w));
        assert!((b.x1 - 0.0).abs() < 1e-6 && (b.x0 + 100.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn waves_move_the_surface_over_time() {
        let w = LiquidFill { wave_height: 10.0.into(), ..calm(0.5) };
        let a = liquid_items(&w, 0.0)[0].path.clone();
        let b = liquid_items(&w, 0.5)[0].path.clone();
        assert_ne!(a, b);
        let s = surface_middle(&w, 0.3);
        assert!(s.y.abs() <= 10.0 * 2.0 + 1e-9, "near the middle level: {s:?}");
    }

    #[test]
    fn the_level_follows_a_dragged_point() {
        let w = calm(0.3);
        assert!((level_for(&w, Point::new(40.0, 0.0)) - 0.5).abs() < 1e-9);
        assert_eq!(level_for(&w, Point::new(0.0, -500.0)), 1.0);
        let s = surface_middle(&w, 0.0);
        assert!((level_for(&w, s) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn bubbles_stay_under_the_surface() {
        let w = LiquidFill { bubbles: 50, ..calm(0.5) };
        for t in [0.0, 0.7, 3.1] {
            for item in &liquid_items(&w, t)[1..] {
                assert!(item.path.bounding_box().y0 > -1.0, "bubble above the water at {t}");
            }
        }
    }
}
