//! The diagram shape library: flowchart symbols, network and cloud/tech
//! icons, and BPMN elements.
//!
//! Every shape here is original work, drawn in code from plain geometry
//! (rectangles, ellipses, arcs and straight lines) for Xenify Vector and released
//! under the project's MIT licence. No outside icon sets, logos or
//! trademarks are used: vendor icons (cloud providers, network equipment
//! makers) are deliberately left out. See `assets/SOURCES.toml`.
//!
//! A shape is drawn centred on the layer's origin at its default size; each
//! part is either the body (filled and outlined), a line (outline only), or
//! a solid mark (filled with the line colour).

use crate::{Command, OpError, invalid};
use xenify_doc::{
    BezPath, Color, Fill, Id, Layer, LineJoin, NodeLabel, Port, PortSide, Project, ShapeItem, Stroke, Vec2,
};
use kurbo::{Arc, Circle, Ellipse, Point, Rect, RoundedRect, Shape};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Flowchart,
    Network,
    Cloud,
    Bpmn,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Flowchart, Category::Network, Category::Cloud, Category::Bpmn];

    pub fn label(self) -> &'static str {
        match self {
            Category::Flowchart => "Flowchart",
            Category::Network => "Network",
            Category::Cloud => "Cloud & software",
            Category::Bpmn => "BPMN & swimlanes",
        }
    }
}

/// How a part of a shape is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Filled with the shape colour and outlined.
    Body,
    /// Outlined only.
    Line,
    /// Filled with the line colour.
    Solid,
    /// Invisible, but clickable (for shapes that are mostly open lines).
    Hit,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub path: BezPath,
    pub role: Role,
}

/// One entry of the library.
pub struct LibShape {
    /// Stable id, e.g. `flowchart.decision` (used by the AI and in files).
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    /// Default size in pixels.
    pub size: (f64, f64),
    /// The label goes under the shape (icons) rather than inside it.
    pub label_below: bool,
    /// Line width multiplier (BPMN end events are drawn heavy).
    pub weight: f64,
    draw: fn(f64, f64) -> Vec<Part>,
    /// Ports other than the middle of each side.
    ports: Option<fn() -> Vec<Port>>,
}

impl LibShape {
    /// The parts at `w` × `h`, centred on the origin.
    pub fn parts(&self, w: f64, h: f64) -> Vec<Part> {
        (self.draw)(w.abs().max(1.0), h.abs().max(1.0))
    }

    /// The shape's ports (empty: the default four).
    pub fn ports(&self) -> Vec<Port> {
        self.ports.map(|f| f()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers (all centred on the origin).

fn body(path: BezPath) -> Part {
    Part { path, role: Role::Body }
}
fn line(path: BezPath) -> Part {
    Part { path, role: Role::Line }
}
fn solid(path: BezPath) -> Part {
    Part { path, role: Role::Solid }
}

fn poly(points: &[(f64, f64)], closed: bool) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(points[0]);
    for q in &points[1..] {
        p.line_to(*q);
    }
    if closed {
        p.close_path();
    }
    p
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.1)
}

fn rrect(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> BezPath {
    RoundedRect::new(x0, y0, x1, y1, r).to_path(0.1)
}

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> BezPath {
    Ellipse::new((cx, cy), (rx, ry), 0.0).to_path(0.1)
}

fn circle(cx: f64, cy: f64, r: f64) -> BezPath {
    Circle::new((cx, cy), r).to_path(0.1)
}

fn seg(a: (f64, f64), b: (f64, f64)) -> BezPath {
    poly(&[a, b], false)
}

/// Adds an elliptical arc (angles in degrees, clockwise on screen) to `p`,
/// joined to where `p` is.
fn arc_to(p: &mut BezPath, c: (f64, f64), rx: f64, ry: f64, start: f64, sweep: f64) {
    let arc = Arc::new(c, (rx, ry), start.to_radians(), sweep.to_radians(), 0.0);
    let mut first = true;
    for el in arc.append_iter(0.1) {
        if first {
            first = false;
            if p.elements().is_empty() {
                p.move_to(arc.center + kurbo::Vec2::new(rx * start.to_radians().cos(), ry * start.to_radians().sin()));
            }
        }
        p.push(el);
    }
}

/// An arrow line from a to b with a small open head at b.
fn arrow(a: (f64, f64), b: (f64, f64), head: f64) -> BezPath {
    let (ax, ay) = a;
    let (bx, by) = b;
    let d = kurbo::Vec2::new(bx - ax, by - ay);
    let len = d.hypot().max(1e-9);
    let u = d / len;
    let n = kurbo::Vec2::new(-u.y, u.x);
    let tip = Point::new(bx, by);
    let back = tip - u * head;
    let mut p = seg(a, b);
    p.move_to(back + n * head * 0.6);
    p.line_to(tip);
    p.line_to(back - n * head * 0.6);
    p
}

/// A cylinder (database) w × h with ellipse height `e`: body and the front
/// rim of the top.
fn cylinder(w: f64, h: f64, bands: usize) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let e = (h * 0.12).min(w * 0.25);
    let mut b = BezPath::new();
    b.move_to((-hw, -hh + e));
    arc_to(&mut b, (0.0, -hh + e), hw, e, 180.0, 180.0);
    b.line_to((hw, hh - e));
    arc_to(&mut b, (0.0, hh - e), hw, e, 0.0, 180.0);
    b.close_path();
    let mut out = vec![body(b)];
    let mut rim = BezPath::new();
    rim.move_to((-hw, -hh + e));
    arc_to(&mut rim, (0.0, -hh + e), hw, e, 180.0, -180.0);
    out.push(line(rim));
    for k in 1..=bands {
        let y = -hh + e + (h - 2.0 * e) * k as f64 / (bands + 1) as f64;
        let mut band = BezPath::new();
        band.move_to((-hw, y));
        arc_to(&mut band, (0.0, y), hw, e, 180.0, -180.0);
        out.push(line(band));
    }
    out
}

/// A page with a wavy bottom edge.
fn document_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    let w = x1 - x0;
    let wave = (y1 - y0) * 0.14;
    let mut p = BezPath::new();
    p.move_to((x0, y0));
    p.line_to((x1, y0));
    p.line_to((x1, y1 - wave));
    p.curve_to((x1 - w * 0.25, y1 - wave * 3.0), (x0 + w * 0.5, y1 + wave * 1.5), (x0, y1 - wave * 0.4));
    p.close_path();
    p
}

fn diamond(w: f64, h: f64) -> BezPath {
    let (hw, hh) = (w / 2.0, h / 2.0);
    poly(&[(0.0, -hh), (hw, 0.0), (0.0, hh), (-hw, 0.0)], true)
}

/// A person: head and shoulders, fitting w × h, centred at (cx, cy).
fn person(cx: f64, cy: f64, w: f64, h: f64) -> Vec<Part> {
    let r = (w * 0.22).min(h * 0.2);
    let head = circle(cx, cy - h / 2.0 + r, r);
    let top = cy - h / 2.0 + 2.2 * r;
    let mut sh = BezPath::new();
    sh.move_to((cx - w / 2.0, cy + h / 2.0));
    sh.line_to((cx - w / 2.0, top + (cy + h / 2.0 - top) * 0.45));
    arc_to(&mut sh, (cx, top + (cy + h / 2.0 - top) * 0.45), w / 2.0, (cy + h / 2.0 - top) * 0.45, 180.0, 180.0);
    sh.line_to((cx + w / 2.0, cy + h / 2.0));
    sh.close_path();
    vec![body(sh), body(head)]
}

// ---------------------------------------------------------------------------
// Flowchart

fn process(w: f64, h: f64) -> Vec<Part> {
    vec![body(rect(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0))]
}
fn decision(w: f64, h: f64) -> Vec<Part> {
    vec![body(diamond(w, h))]
}
fn terminator(w: f64, h: f64) -> Vec<Part> {
    vec![body(rrect(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0, h.min(w) / 2.0))]
}
fn data(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh, k) = (w / 2.0, h / 2.0, (w * 0.18).min(h * 0.5));
    vec![body(poly(&[(-hw + k, -hh), (hw, -hh), (hw - k, hh), (-hw, hh)], true))]
}
fn document(w: f64, h: f64) -> Vec<Part> {
    vec![body(document_path(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0))]
}
fn multi_document(w: f64, h: f64) -> Vec<Part> {
    let o = (w.min(h) * 0.08).max(3.0);
    let (x0, y0, x1, y1) = (-w / 2.0, -h / 2.0, w / 2.0, h / 2.0);
    vec![
        body(document_path(x0 + 2.0 * o, y0, x1, y1 - 2.0 * o)),
        body(document_path(x0 + o, y0 + o, x1 - o, y1 - o)),
        body(document_path(x0, y0 + 2.0 * o, x1 - 2.0 * o, y1)),
    ]
}
fn predefined(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let k = (w * 0.1).min(14.0);
    vec![body(rect(-hw, -hh, hw, hh)), line(seg((-hw + k, -hh), (-hw + k, hh))), line(seg((hw - k, -hh), (hw - k, hh)))]
}
fn manual_input(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![body(poly(&[(-hw, -hh + h * 0.3), (hw, -hh), (hw, hh), (-hw, hh)], true))]
}
fn manual_operation(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh, k) = (w / 2.0, h / 2.0, w * 0.14);
    vec![body(poly(&[(-hw, -hh), (hw, -hh), (hw - k, hh), (-hw + k, hh)], true))]
}
fn preparation(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh, k) = (w / 2.0, h / 2.0, (w * 0.18).min(h * 0.5));
    vec![body(poly(&[(-hw, 0.0), (-hw + k, -hh), (hw - k, -hh), (hw, 0.0), (hw - k, hh), (-hw + k, hh)], true))]
}
fn database(w: f64, h: f64) -> Vec<Part> {
    cylinder(w, h, 0)
}
fn delay(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let r = hh.min(hw);
    let mut p = BezPath::new();
    p.move_to((-hw, -hh));
    p.line_to((hw - r, -hh));
    arc_to(&mut p, (hw - r, 0.0), r, hh, -90.0, 180.0);
    p.line_to((-hw, hh));
    p.close_path();
    vec![body(p)]
}
fn off_page(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![body(poly(&[(-hw, -hh), (hw, -hh), (hw, hh * 0.3), (0.0, hh), (-hw, hh * 0.3)], true))]
}
fn on_page(w: f64, h: f64) -> Vec<Part> {
    vec![body(ellipse(0.0, 0.0, w / 2.0, h / 2.0))]
}
fn annotation(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let k = (w * 0.12).min(14.0);
    vec![
        Part { path: rect(-hw, -hh, hw, hh), role: Role::Hit },
        line(poly(&[(-hw + k, -hh), (-hw, -hh), (-hw, hh), (-hw + k, hh)], false)),
    ]
}
fn stored_data(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let k = (w * 0.12).min(hh);
    let mut p = BezPath::new();
    p.move_to((-hw + k, -hh));
    p.line_to((hw, -hh));
    arc_to(&mut p, (hw, 0.0), k, hh, -90.0, -180.0);
    p.line_to((-hw + k, hh));
    arc_to(&mut p, (-hw + k, 0.0), k, hh, 90.0, 180.0);
    p.close_path();
    vec![body(p)]
}

// ---------------------------------------------------------------------------
// Network

fn server(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let mut out = vec![body(rrect(-hw, -hh, hw, hh, w * 0.08))];
    for k in 1..3 {
        let y = -hh + h * k as f64 / 3.0;
        out.push(line(seg((-hw, y), (hw, y))));
    }
    for k in 0..3 {
        let y = -hh + h * (k as f64 + 0.5) / 3.0;
        out.push(solid(circle(hw - w * 0.2, y, w * 0.05)));
        out.push(line(seg((-hw + w * 0.15, y), (hw - w * 0.4, y))));
    }
    out
}
fn desktop(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let screen_bottom = -hh + h * 0.72;
    vec![
        body(rrect(-hw, -hh, hw, screen_bottom, w * 0.05)),
        line(rect(-hw + w * 0.07, -hh + h * 0.08, hw - w * 0.07, screen_bottom - h * 0.08)),
        body(rect(-w * 0.06, screen_bottom, w * 0.06, hh - h * 0.08)),
        body(rrect(-w * 0.25, hh - h * 0.08, w * 0.25, hh, 2.0)),
    ]
}
fn laptop(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let base = hh - h * 0.2;
    vec![
        body(rrect(-hw * 0.78, -hh, hw * 0.78, base, w * 0.04)),
        line(rect(-hw * 0.66, -hh + h * 0.1, hw * 0.66, base - h * 0.1)),
        body(poly(&[(-hw * 0.9, base), (hw * 0.9, base), (hw, hh), (-hw, hh)], true)),
    ]
}
fn phone(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![
        body(rrect(-hw, -hh, hw, hh, w * 0.2)),
        line(rect(-hw + w * 0.12, -hh + h * 0.12, hw - w * 0.12, hh - h * 0.18)),
        solid(circle(0.0, hh - h * 0.09, w * 0.07)),
    ]
}
fn router(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let a = hw * 0.55;
    vec![
        body(ellipse(0.0, 0.0, hw, hh)),
        line(arrow((-a, -hh * 0.3), (a, -hh * 0.3), hh * 0.25)),
        line(arrow((a, hh * 0.3), (-a, hh * 0.3), hh * 0.25)),
    ]
}
fn switch(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let a = hw * 0.6;
    vec![
        body(rrect(-hw, -hh, hw, hh, h * 0.15)),
        line(arrow((-a, -hh * 0.35), (a, -hh * 0.35), hh * 0.3)),
        line(arrow((a, hh * 0.35), (-a, hh * 0.35), hh * 0.3)),
    ]
}
fn firewall(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let mut out = vec![body(rect(-hw, -hh, hw, hh))];
    let rows = 4;
    for r in 0..rows {
        let y0 = -hh + h * r as f64 / rows as f64;
        let y1 = -hh + h * (r + 1) as f64 / rows as f64;
        if r > 0 {
            out.push(line(seg((-hw, y0), (hw, y0))));
        }
        let off = if r % 2 == 0 { 0.25 } else { 0.0 };
        for k in 0..3 {
            let x = -hw + w * (off + k as f64 * 0.5);
            if x > -hw + 1.0 && x < hw - 1.0 {
                out.push(line(seg((x, y0), (x, y1))));
            }
        }
    }
    out
}
fn cloud(w: f64, h: f64) -> Vec<Part> {
    // Bumps drawn as arcs around a flat bottom, in a 120 × 70 design box.
    let (sx, sy) = (w / 120.0, h / 70.0);
    let mut p = BezPath::new();
    let s = |x: f64, y: f64| (x * sx, y * sy);
    p.move_to(s(-38.0, 30.0));
    arc_to(&mut p, s(-38.0, 12.0), 18.0 * sx, 18.0 * sy, 90.0, 180.0);
    arc_to(&mut p, s(-12.0, -8.0), 24.0 * sx, 24.0 * sy, 200.0, 125.0);
    arc_to(&mut p, s(18.0, -10.0), 22.0 * sx, 22.0 * sy, 225.0, 110.0);
    arc_to(&mut p, s(40.0, 12.0), 18.0 * sx, 18.0 * sy, 280.0, 170.0);
    p.close_path();
    vec![body(p)]
}
fn globe(w: f64, h: f64) -> Vec<Part> {
    let (rx, ry) = (w / 2.0, h / 2.0);
    vec![
        body(ellipse(0.0, 0.0, rx, ry)),
        line(ellipse(0.0, 0.0, rx * 0.42, ry)),
        line(seg((-rx, 0.0), (rx, 0.0))),
        line(seg((0.0, -ry), (0.0, ry))),
        line(seg((-rx * 0.87, -ry * 0.5), (rx * 0.87, -ry * 0.5))),
        line(seg((-rx * 0.87, ry * 0.5), (rx * 0.87, ry * 0.5))),
    ]
}
fn load_balancer(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let mut out = vec![body(rrect(-hw, -hh, hw, hh, h * 0.2))];
    let from = (-hw * 0.6, 0.0);
    out.push(line(seg(from, (-hw * 0.15, 0.0))));
    for y in [-hh * 0.5, 0.0, hh * 0.5] {
        out.push(line(arrow((-hw * 0.15, 0.0), (hw * 0.65, y), hh * 0.22)));
    }
    out
}
fn wireless(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let base_top = hh - h * 0.3;
    let mut out = vec![body(rrect(-hw * 0.7, base_top, hw * 0.7, hh, h * 0.08))];
    let c = (0.0, base_top - h * 0.05);
    out.push(solid(circle(c.0, c.1, w * 0.05)));
    for k in 1..=3 {
        let r = (base_top + hh) * 0.28 * k as f64;
        let mut a = BezPath::new();
        a.move_to((c.0 + r * (225f64).to_radians().cos(), c.1 + r * (225f64).to_radians().sin()));
        arc_to(&mut a, c, r, r, 225.0, 90.0);
        out.push(line(a));
    }
    out
}
fn storage(w: f64, h: f64) -> Vec<Part> {
    cylinder(w, h, 2)
}
fn printer(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![
        body(rect(-hw * 0.6, -hh, hw * 0.6, -hh + h * 0.4)),
        body(rrect(-hw, -hh + h * 0.3, hw, hh - h * 0.2, h * 0.08)),
        body(rect(-hw * 0.6, hh - h * 0.32, hw * 0.6, hh)),
        solid(circle(hw * 0.75, -hh + h * 0.45, w * 0.04)),
    ]
}

// ---------------------------------------------------------------------------
// Cloud & software

fn service(w: f64, h: f64) -> Vec<Part> {
    vec![body(rrect(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0, h.min(w) * 0.16))]
}
fn api(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let k = w * 0.25;
    vec![body(poly(&[(-hw, 0.0), (-hw + k, -hh), (hw - k, -hh), (hw, 0.0), (hw - k, hh), (-hw + k, hh)], true))]
}
fn queue(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let mut out = vec![body(rrect(-hw, -hh, hw, hh, h * 0.15))];
    for k in 1..4 {
        let x = -hw + w * k as f64 / 4.0;
        out.push(line(seg((x, -hh), (x, hh))));
    }
    out
}
fn topic(w: f64, h: f64) -> Vec<Part> {
    let r = w.min(h) * 0.28;
    let c = (-w / 2.0 + r + 2.0, 0.0);
    let mut out = vec![body(circle(c.0, c.1, r))];
    for y in [-h * 0.35, 0.0, h * 0.35] {
        out.push(line(arrow((c.0 + r, c.1), (w / 2.0, y), h * 0.12)));
    }
    out
}
fn container(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let q = hh * 0.5;
    vec![
        body(poly(&[(0.0, -hh), (hw, -hh + q), (hw, hh - q), (0.0, hh), (-hw, hh - q), (-hw, -hh + q)], true)),
        line(poly(&[(-hw, -hh + q), (0.0, -hh + 2.0 * q), (hw, -hh + q)], false)),
        line(seg((0.0, -hh + 2.0 * q), (0.0, hh))),
    ]
}
fn function(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let s = w.min(h) * 0.3;
    vec![
        body(rrect(-hw, -hh, hw, hh, w.min(h) * 0.15)),
        // λ
        line(poly(&[(-s * 0.8, -s), (-s * 0.3, -s), (s * 0.8, s)], false)),
        line(seg((0.1 * s, -0.15 * s), (-s * 0.8, s))),
    ]
}
fn bucket(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let e = h * 0.12;
    let mut b = BezPath::new();
    b.move_to((-hw, -hh + e));
    b.line_to((-hw * 0.72, hh - e * 0.6));
    arc_to(&mut b, (0.0, hh - e * 0.6), hw * 0.72, e * 0.6, 180.0, -180.0);
    b.line_to((hw, -hh + e));
    arc_to(&mut b, (0.0, -hh + e), hw, e, 0.0, -180.0);
    b.close_path();
    vec![body(b), line(ellipse(0.0, -hh + e, hw, e))]
}
fn cache(w: f64, h: f64) -> Vec<Part> {
    let mut out = cylinder(w, h, 0);
    let s = w.min(h) * 0.22;
    out.push(solid(poly(
        &[
            (s * 0.2, -s * 1.1),
            (-s * 0.6, s * 0.2),
            (0.0, s * 0.2),
            (-s * 0.2, s * 1.3),
            (s * 0.6, -s * 0.1),
            (0.0, -s * 0.1),
        ],
        true,
    )));
    out
}
fn user(w: f64, h: f64) -> Vec<Part> {
    person(0.0, 0.0, w, h)
}
fn users(w: f64, h: f64) -> Vec<Part> {
    let mut out = person(w * 0.18, -h * 0.05, w * 0.6, h * 0.9);
    out.extend(person(-w * 0.18, h * 0.05, w * 0.6, h * 0.9));
    out
}
fn browser(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let bar = -hh + h * 0.2;
    let mut out = vec![body(rrect(-hw, -hh, hw, hh, 4.0)), line(seg((-hw, bar), (hw, bar)))];
    for k in 0..3 {
        out.push(solid(circle(-hw + w * (0.07 + 0.07 * k as f64), -hh + h * 0.1, h * 0.035)));
    }
    out
}
fn gateway(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![
        body(rrect(-hw, -hh, hw, hh, h * 0.12)),
        line(seg((-hw * 0.25, -hh), (-hw * 0.25, hh))),
        line(seg((hw * 0.25, -hh), (hw * 0.25, hh))),
        line(arrow((-hw * 0.8, 0.0), (hw * 0.8, 0.0), hh * 0.3)),
    ]
}
fn key(w: f64, h: f64) -> Vec<Part> {
    let r = h * 0.3;
    let c = (-w / 2.0 + r + 1.0, 0.0);
    let x1 = w / 2.0;
    let t = h * 0.07;
    vec![
        body(circle(c.0, c.1, r)),
        line(circle(c.0, c.1, r * 0.35)),
        body(poly(
            &[
                (c.0 + r, -t),
                (x1, -t),
                (x1, t + h * 0.18),
                (x1 - w * 0.1, t + h * 0.18),
                (x1 - w * 0.1, t),
                (x1 - w * 0.2, t),
                (x1 - w * 0.2, t + h * 0.12),
                (x1 - w * 0.3, t + h * 0.12),
                (x1 - w * 0.3, t),
                (c.0 + r, t),
            ],
            true,
        )),
    ]
}

// ---------------------------------------------------------------------------
// BPMN

fn event(w: f64, h: f64) -> Vec<Part> {
    vec![body(ellipse(0.0, 0.0, w / 2.0, h / 2.0))]
}
fn intermediate(w: f64, h: f64) -> Vec<Part> {
    vec![body(ellipse(0.0, 0.0, w / 2.0, h / 2.0)), line(ellipse(0.0, 0.0, w / 2.0 - 3.5, h / 2.0 - 3.5))]
}
fn task(w: f64, h: f64) -> Vec<Part> {
    vec![body(rrect(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0, h.min(w) * 0.14))]
}
fn subprocess(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let s = 7.0;
    let y = hh - s - 3.0;
    vec![
        body(rrect(-hw, -hh, hw, hh, h.min(w) * 0.14)),
        line(rect(-s, y - s, s, y + s)),
        line(seg((-s * 0.6, y), (s * 0.6, y))),
        line(seg((0.0, y - s * 0.6), (0.0, y + s * 0.6))),
    ]
}
fn gateway_x(w: f64, h: f64) -> Vec<Part> {
    let k = w.min(h) * 0.18;
    vec![body(diamond(w, h)), line(seg((-k, -k), (k, k))), line(seg((-k, k), (k, -k)))]
}
fn gateway_plus(w: f64, h: f64) -> Vec<Part> {
    let k = w.min(h) * 0.24;
    vec![body(diamond(w, h)), line(seg((-k, 0.0), (k, 0.0))), line(seg((0.0, -k), (0.0, k)))]
}
fn gateway_o(w: f64, h: f64) -> Vec<Part> {
    vec![body(diamond(w, h)), line(ellipse(0.0, 0.0, w * 0.2, h * 0.2))]
}
fn data_object(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let f = w.min(h) * 0.3;
    vec![
        body(poly(&[(-hw, -hh), (hw - f, -hh), (hw, -hh + f), (hw, hh), (-hw, hh)], true)),
        line(poly(&[(hw - f, -hh), (hw - f, -hh + f), (hw, -hh + f)], false)),
    ]
}
fn data_store(w: f64, h: f64) -> Vec<Part> {
    cylinder(w, h, 2)
}
fn pool(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![body(rect(-hw, -hh, hw, hh)), line(seg((-hw + 30.0, -hh), (-hw + 30.0, hh)))]
}
fn lane(w: f64, h: f64) -> Vec<Part> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![body(rect(-hw, -hh, hw, hh)), line(seg((-hw + 24.0, -hh), (-hw + 24.0, hh)))]
}

fn corner_ports() -> Vec<Port> {
    // A diamond's four points are its side midpoints already.
    Vec::new()
}

// ---------------------------------------------------------------------------

macro_rules! shape {
    ($id:literal, $name:literal, $cat:ident, $w:expr, $h:expr, $draw:ident) => {
        LibShape {
            id: $id,
            name: $name,
            category: Category::$cat,
            size: ($w, $h),
            label_below: false,
            weight: 1.0,
            draw: $draw,
            ports: None,
        }
    };
    ($id:literal, $name:literal, $cat:ident, $w:expr, $h:expr, $draw:ident, below) => {
        LibShape { label_below: true, ..shape!($id, $name, $cat, $w, $h, $draw) }
    };
}

static SHAPES: &[LibShape] = &[
    shape!("flowchart.process", "Process", Flowchart, 120.0, 60.0, process),
    shape!("flowchart.decision", "Decision", Flowchart, 120.0, 80.0, decision),
    shape!("flowchart.terminator", "Start / end", Flowchart, 120.0, 50.0, terminator),
    shape!("flowchart.data", "Data (input/output)", Flowchart, 120.0, 60.0, data),
    shape!("flowchart.document", "Document", Flowchart, 120.0, 70.0, document),
    shape!("flowchart.documents", "Documents", Flowchart, 120.0, 76.0, multi_document),
    shape!("flowchart.predefined", "Predefined process", Flowchart, 120.0, 60.0, predefined),
    shape!("flowchart.manual_input", "Manual input", Flowchart, 120.0, 60.0, manual_input),
    shape!("flowchart.manual_operation", "Manual operation", Flowchart, 120.0, 60.0, manual_operation),
    shape!("flowchart.preparation", "Preparation", Flowchart, 120.0, 60.0, preparation),
    shape!("flowchart.database", "Database", Flowchart, 80.0, 90.0, database),
    shape!("flowchart.stored_data", "Stored data", Flowchart, 120.0, 60.0, stored_data),
    shape!("flowchart.delay", "Delay", Flowchart, 110.0, 60.0, delay),
    shape!("flowchart.off_page", "Off-page link", Flowchart, 50.0, 50.0, off_page, below),
    shape!("flowchart.on_page", "On-page link", Flowchart, 40.0, 40.0, on_page, below),
    shape!("flowchart.note", "Note", Flowchart, 110.0, 60.0, annotation),
    shape!("network.server", "Server", Network, 56.0, 76.0, server, below),
    shape!("network.desktop", "Desktop", Network, 80.0, 70.0, desktop, below),
    shape!("network.laptop", "Laptop", Network, 90.0, 60.0, laptop, below),
    shape!("network.phone", "Phone", Network, 38.0, 68.0, phone, below),
    shape!("network.router", "Router", Network, 80.0, 44.0, router, below),
    shape!("network.switch", "Switch", Network, 90.0, 36.0, switch, below),
    shape!("network.firewall", "Firewall", Network, 80.0, 56.0, firewall, below),
    shape!("network.cloud", "Cloud", Network, 120.0, 70.0, cloud),
    shape!("network.internet", "Internet", Network, 64.0, 64.0, globe, below),
    shape!("network.load_balancer", "Load balancer", Network, 80.0, 48.0, load_balancer, below),
    shape!("network.wireless", "Wireless access point", Network, 70.0, 60.0, wireless, below),
    shape!("network.database", "Database", Network, 64.0, 76.0, database, below),
    shape!("network.storage", "Storage", Network, 64.0, 76.0, storage, below),
    shape!("network.printer", "Printer", Network, 80.0, 64.0, printer, below),
    shape!("cloud.service", "Service", Cloud, 120.0, 60.0, service),
    shape!("cloud.api", "API", Cloud, 110.0, 56.0, api),
    shape!("cloud.queue", "Queue", Cloud, 110.0, 40.0, queue, below),
    shape!("cloud.topic", "Topic (publish/subscribe)", Cloud, 80.0, 56.0, topic, below),
    shape!("cloud.container", "Container", Cloud, 60.0, 64.0, container, below),
    shape!("cloud.function", "Function", Cloud, 60.0, 60.0, function, below),
    shape!("cloud.bucket", "Object storage", Cloud, 64.0, 64.0, bucket, below),
    shape!("cloud.cache", "Cache", Cloud, 64.0, 70.0, cache, below),
    shape!("cloud.user", "User", Cloud, 44.0, 60.0, user, below),
    shape!("cloud.users", "Users", Cloud, 64.0, 60.0, users, below),
    shape!("cloud.mobile_app", "Mobile app", Cloud, 38.0, 68.0, phone, below),
    shape!("cloud.browser", "Web browser", Cloud, 90.0, 64.0, browser, below),
    shape!("cloud.gateway", "Gateway", Cloud, 80.0, 50.0, gateway, below),
    shape!("cloud.secret", "Key / secret", Cloud, 80.0, 40.0, key, below),
    shape!("bpmn.start", "Start event", Bpmn, 40.0, 40.0, event, below),
    shape!("bpmn.intermediate", "Intermediate event", Bpmn, 40.0, 40.0, intermediate, below),
    LibShape { weight: 2.5, ..shape!("bpmn.end", "End event", Bpmn, 40.0, 40.0, event, below) },
    shape!("bpmn.task", "Task", Bpmn, 120.0, 72.0, task),
    shape!("bpmn.subprocess", "Sub-process", Bpmn, 120.0, 72.0, subprocess),
    LibShape {
        ports: Some(corner_ports),
        ..shape!("bpmn.exclusive", "Exclusive gateway", Bpmn, 50.0, 50.0, gateway_x, below)
    },
    shape!("bpmn.parallel", "Parallel gateway", Bpmn, 50.0, 50.0, gateway_plus, below),
    shape!("bpmn.inclusive", "Inclusive gateway", Bpmn, 50.0, 50.0, gateway_o, below),
    shape!("bpmn.data_object", "Data object", Bpmn, 40.0, 52.0, data_object, below),
    shape!("bpmn.data_store", "Data store", Bpmn, 56.0, 56.0, data_store, below),
    shape!("bpmn.pool", "Pool", Bpmn, 720.0, 240.0, pool),
    shape!("bpmn.lane", "Lane", Bpmn, 720.0, 160.0, lane),
];

/// Every shape in the library, in display order.
pub fn all() -> &'static [LibShape] {
    SHAPES
}

pub fn find(id: &str) -> Option<&'static LibShape> {
    SHAPES.iter().find(|s| s.id == id)
}

/// Colours of a new shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fill: Color,
    pub line: Color,
    pub width: f64,
}

impl Default for Style {
    fn default() -> Self {
        Style { fill: Color::WHITE, line: Color::rgb(0.2, 0.22, 0.26), width: 2.0 }
    }
}

/// A layer drawing `shape` at `size` (default size when `None`), centred at
/// `at` (composition space), labelled `label`.
pub fn layer(shape: &LibShape, id: Id, at: Vec2, size: Option<Vec2>, style: Style, label: &str) -> Layer {
    let (w, h) = size.map_or(shape.size, |s| (s.x, s.y));
    let width = style.width * shape.weight;
    let items: Vec<ShapeItem> = shape
        .parts(w, h)
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let mut item = ShapeItem::path(part.path, None, None);
            item.name = match part.role {
                Role::Body => format!("{} {}", shape.name, i + 1),
                Role::Line => format!("Line {}", i + 1),
                Role::Solid => format!("Mark {}", i + 1),
                Role::Hit => "Hit area".into(),
            };
            let outline = || Stroke {
                color: style.line.into(),
                width: width.into(),
                join: LineJoin::Round,
                cap: xenify_doc::LineCap::Round,
                ..Default::default()
            };
            match part.role {
                Role::Body => {
                    item.fill = Some(Fill::solid(style.fill));
                    item.stroke = Some(outline());
                }
                Role::Line => item.stroke = Some(outline()),
                Role::Solid => item.fill = Some(Fill::solid(style.line)),
                Role::Hit => item.fill = Some(Fill::solid(Color::TRANSPARENT)),
            }
            item
        })
        .collect();
    let mut l = Layer::shape(id, shape.name, items).with_position(at);
    l.meta.ports = shape.ports();
    l.meta.container = matches!(shape.id, "bpmn.pool" | "bpmn.lane");
    if !label.is_empty() {
        let mut nl = NodeLabel::new(label);
        nl.below = shape.label_below;
        if shape.label_below {
            nl.font_size = 13.0;
            nl.inset = 4.0;
        }
        l.meta.label = Some(nl);
    }
    l
}

/// Adds library shape `shape_id` to `comp`, centred at `at`, on top (or at
/// the bottom for pools and lanes, so they stay behind what's in them).
/// Returns the command and the new layer's id.
pub fn insert(
    project: &Project,
    comp: Id,
    shape_id: &str,
    at: Vec2,
    size: Option<Vec2>,
    label: Option<&str>,
) -> Result<(Command, Id), OpError> {
    project.comp(comp)?;
    let shape = find(shape_id).ok_or_else(|| invalid(format!("no library shape called “{shape_id}”")))?;
    let id = Id(project.next_id);
    let layer = layer(shape, id, at, size, Style::default(), label.unwrap_or(shape.name));
    // Pools at the very back; lanes just above the pools (drawn on them).
    let pools = project.comp(comp)?.layers.iter().take_while(|l| l.meta.container && l.meta.name == "Pool").count();
    let index = match shape_id {
        "bpmn.pool" => Some(0),
        "bpmn.lane" => Some(pools),
        _ => None,
    };
    Ok((Command::AddLayer { comp, index, layer }, id))
}

/// The side a port on a shape's box at `pos` (fractions) faces.
pub fn side_for(pos: Vec2) -> PortSide {
    let d = [pos.y, 1.0 - pos.x, 1.0 - pos.y, pos.x];
    let i = (0..4).min_by(|a, b| d[*a].total_cmp(&d[*b])).unwrap_or(0);
    [PortSide::Top, PortSide::Right, PortSide::Bottom, PortSide::Left][i]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;

    #[test]
    fn ids_are_unique_and_every_shape_draws_inside_its_box() {
        let mut seen = std::collections::HashSet::new();
        for s in all() {
            assert!(seen.insert(s.id), "duplicate id {}", s.id);
            assert!(s.id.contains('.') && !s.name.is_empty());
            let (w, h) = s.size;
            let parts = s.parts(w, h);
            assert!(parts.iter().any(|p| p.role != Role::Line), "{} has nothing to click", s.id);
            for p in &parts {
                let b = p.path.bounding_box();
                assert!(b.is_finite(), "{}", s.id);
                let slack = 1.5;
                assert!(
                    b.x0 >= -w / 2.0 - slack
                        && b.x1 <= w / 2.0 + slack
                        && b.y0 >= -h / 2.0 - slack
                        && b.y1 <= h / 2.0 + slack,
                    "{} draws outside its {w}×{h} box: {b:?}",
                    s.id
                );
            }
            // Resizing keeps it inside the new box too.
            for p in s.parts(w * 2.0, h * 0.5) {
                let b = p.path.bounding_box();
                assert!(b.x1 <= w + 3.0 && b.y1 <= h / 4.0 + 3.0, "{} at 2×, ½: {b:?}", s.id);
            }
        }
        for c in Category::ALL {
            assert!(all().iter().filter(|s| s.category == c).count() >= 12, "{c:?} has few shapes");
        }
    }

    #[test]
    fn inserting_places_a_labelled_shape() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let (cmd, id) = insert(&p, comp, "flowchart.decision", Vec2::new(200.0, 100.0), None, Some("OK?")).unwrap();
        apply(&mut p, cmd).unwrap();
        let l = p.layer(id).unwrap();
        assert_eq!(l.meta.label.as_ref().unwrap().text, "OK?");
        assert_eq!(l.transform.position.value_at(0.0), Vec2::new(200.0, 100.0));
        assert!(insert(&p, comp, "nope", Vec2::ZERO, None, None).is_err());
        let (cmd, pool) = insert(&p, comp, "bpmn.pool", Vec2::ZERO, None, None).unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.comp(comp).unwrap().layers[0].id, pool, "pools go to the back");
    }
}
