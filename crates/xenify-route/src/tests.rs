use super::*;
use kurbo::{PathEl, Rect};

fn req<'a>(from: End, to: End, obstacles: &'a [Rect], style: Style) -> Request<'a> {
    Request { from, to, waypoints: &[], obstacles, occupied: &[], options: Options { style, ..Options::default() } }
}

fn bends(p: &[Point]) -> usize {
    p.len().saturating_sub(2)
}

fn axis_aligned(p: &[Point]) -> bool {
    p.windows(2).all(|w| (w[0].x - w[1].x).abs() < 1e-6 || (w[0].y - w[1].y).abs() < 1e-6)
}

/// True if any segment passes through the inside of `r`.
fn crosses(p: &[Point], r: Rect) -> bool {
    p.windows(2).any(|w| {
        (0..=20).any(|k| {
            let q = w[0].lerp(w[1], k as f64 / 20.0);
            q.x > r.x0 + 1e-6 && q.x < r.x1 - 1e-6 && q.y > r.y0 + 1e-6 && q.y < r.y1 - 1e-6
        })
    })
}

const A: Rect = Rect::new(0.0, 0.0, 100.0, 60.0);
const B: Rect = Rect::new(300.0, 0.0, 400.0, 60.0);

#[test]
fn facing_ports_give_a_straight_line() {
    let p = route(&req(
        End::Port { point: Point::new(100.0, 30.0), side: Side::Right, shape: A },
        End::Port { point: Point::new(300.0, 30.0), side: Side::Left, shape: B },
        &[],
        Style::Orthogonal,
    ));
    assert_eq!(p, vec![Point::new(100.0, 30.0), Point::new(300.0, 30.0)]);
}

#[test]
fn floating_ends_pick_the_facing_sides() {
    let p = route(&req(End::Floating(A), End::Floating(B), &[], Style::Orthogonal));
    assert_eq!(p, vec![Point::new(100.0, 30.0), Point::new(300.0, 30.0)]);
    let below = Rect::new(0.0, 300.0, 100.0, 360.0);
    let p = route(&req(End::Floating(A), End::Floating(below), &[], Style::Orthogonal));
    assert_eq!(p, vec![Point::new(50.0, 60.0), Point::new(50.0, 300.0)]);
}

#[test]
fn l_route_has_one_bend() {
    let c = Rect::new(300.0, 300.0, 400.0, 360.0);
    let p = route(&req(
        End::Port { point: Point::new(100.0, 30.0), side: Side::Right, shape: A },
        End::Port { point: Point::new(350.0, 300.0), side: Side::Top, shape: c },
        &[],
        Style::Orthogonal,
    ));
    assert!(axis_aligned(&p));
    assert_eq!(bends(&p), 1, "{p:?}");
}

#[test]
fn routes_go_around_obstacles() {
    let wall = Rect::new(180.0, -100.0, 220.0, 160.0);
    let p = route(&req(End::Floating(A), End::Floating(B), &[wall], Style::Orthogonal));
    assert!(axis_aligned(&p));
    assert!(!crosses(&p, wall), "{p:?}");
    assert!(!crosses(&p, A) && !crosses(&p, B));
    // Over or under the wall: out, across and back in.
    assert!(bends(&p) <= 4, "{p:?}");
    // Keeps the margin.
    assert!(!crosses(&p, wall.inflate(11.0, 11.0)));
}

#[test]
fn u_route_when_ports_face_away() {
    let p = route(&req(
        End::Port { point: Point::new(50.0, 0.0), side: Side::Top, shape: A },
        End::Port { point: Point::new(350.0, 0.0), side: Side::Top, shape: B },
        &[],
        Style::Orthogonal,
    ));
    assert!(axis_aligned(&p));
    assert_eq!(bends(&p), 2, "{p:?}");
    assert!(p[1].y < -11.0);
}

#[test]
fn waypoints_are_visited_in_order() {
    let w = [Point::new(200.0, 200.0)];
    let mut r = req(End::Floating(A), End::Floating(B), &[], Style::Orthogonal);
    r.waypoints = &w;
    let p = route(&r);
    assert!(axis_aligned(&p));
    assert!(p.windows(2).any(|s| {
        let (a, b) = (s[0], s[1]);
        let on_x = (a.x - 200.0).abs() < 1e-6 && (b.x - 200.0).abs() < 1e-6 && (a.y - 200.0) * (b.y - 200.0) <= 0.0;
        let on_y = (a.y - 200.0).abs() < 1e-6 && (b.y - 200.0).abs() < 1e-6 && (a.x - 200.0) * (b.x - 200.0) <= 0.0;
        on_x || on_y
    }));
}

#[test]
fn free_ends_and_ignoring_obstacles() {
    let wall = [Rect::new(40.0, -50.0, 60.0, 50.0)];
    let mut r = req(End::Free(Point::new(0.0, 0.0)), End::Free(Point::new(100.0, 0.0)), &wall, Style::Orthogonal);
    assert!(!crosses(&route(&r), wall[0]));
    r.options.avoid = false;
    assert_eq!(route(&r), vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)]);
}

#[test]
fn routing_is_deterministic() {
    let obs = [Rect::new(150.0, -40.0, 250.0, 100.0), Rect::new(120.0, 120.0, 200.0, 200.0)];
    let below = Rect::new(200.0, 300.0, 300.0, 360.0);
    let a = route(&req(End::Floating(A), End::Floating(below), &obs, Style::Orthogonal));
    for _ in 0..5 {
        assert_eq!(a, route(&req(End::Floating(A), End::Floating(below), &obs, Style::Orthogonal)));
    }
}

#[test]
fn straight_clips_to_floating_shapes() {
    let p = route(&req(End::Floating(A), End::Floating(Rect::new(300.0, 200.0, 400.0, 260.0)), &[], Style::Straight));
    assert_eq!(p.len(), 2);
    assert!((p[0].x - 100.0).abs() < 1e-6 || (p[0].y - 60.0).abs() < 1e-6);
    assert!((p[1].x - 300.0).abs() < 1e-6 || (p[1].y - 200.0).abs() < 1e-6);
}

#[test]
fn avoids_running_along_an_existing_route() {
    // Two connectors between the same pair of stacked shapes, with a wall
    // between: the second shouldn't sit exactly on the first.
    let top = Rect::new(0.0, 0.0, 100.0, 60.0);
    let bottom = Rect::new(0.0, 300.0, 100.0, 360.0);
    let wall = Rect::new(-50.0, 150.0, 150.0, 200.0);
    let first = route(&req(End::Floating(top), End::Floating(bottom), &[wall], Style::Orthogonal));
    let occupied = [first.clone()];
    let walls = [wall];
    let mut r = req(End::Floating(top), End::Floating(bottom), &walls, Style::Orthogonal);
    r.occupied = &occupied;
    let second = route(&r);
    assert_ne!(first, second);
}

#[test]
fn hops_only_on_real_crossings() {
    let h: &[Point] = &[Point::new(0.0, 50.0), Point::new(100.0, 50.0)];
    let v: &[Point] = &[Point::new(50.0, 0.0), Point::new(50.0, 100.0)];
    // Meets h at its end: not a crossing.
    let t: &[Point] = &[Point::new(100.0, 0.0), Point::new(100.0, 50.0)];
    let hops = find_hops(&[h, v, t], 6.0);
    assert!(hops[0].is_empty());
    assert_eq!(hops[1], vec![Hop { segment: 0, at: 50.0 }]);
    assert!(hops[2].is_empty());
}

#[test]
fn arc_hop_draws_curves_gap_breaks_the_line() {
    let v = [Point::new(50.0, 0.0), Point::new(50.0, 100.0)];
    let hop = [Hop { segment: 0, at: 50.0 }];
    let arc = build_path(&v, &hop, JumpStyle::Arc, 6.0, 0.0, false);
    assert_eq!(arc.elements().iter().filter(|e| matches!(e, PathEl::CurveTo(..))).count(), 2);
    let gap = build_path(&v, &hop, JumpStyle::Gap, 6.0, 0.0, false);
    assert_eq!(gap.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(..))).count(), 2);
    let none = build_path(&v, &hop, JumpStyle::None, 6.0, 0.0, false);
    assert_eq!(none.elements().len(), 2);
}

#[test]
fn rounded_corners_and_curves() {
    let l = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0)];
    let p = build_path(&l, &[], JumpStyle::Arc, 6.0, 10.0, false);
    assert!(p.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..))));
    let c = build_path(&l, &[], JumpStyle::Arc, 6.0, 0.0, true);
    assert!(matches!(c.elements().last(), Some(PathEl::QuadTo(_, e)) if *e == Point::new(100.0, 100.0)));
}

#[test]
fn junction_where_shared_routes_split() {
    let a: &[Point] = &[Point::new(0.0, 0.0), Point::new(0.0, 50.0), Point::new(100.0, 50.0)];
    let b: &[Point] = &[Point::new(0.0, 0.0), Point::new(0.0, 50.0), Point::new(-100.0, 50.0)];
    assert_eq!(junctions(&[a, b], 0.5), vec![Point::new(0.0, 50.0)]);
    // Only sharing the end point itself: no dot.
    let c: &[Point] = &[Point::new(0.0, 0.0), Point::new(80.0, 0.0)];
    assert!(junctions(&[a, c], 0.5).is_empty());
}
