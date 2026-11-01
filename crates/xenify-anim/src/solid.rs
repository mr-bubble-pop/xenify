//! Turns a [`Solid`] into shaded 2D faces: the object is built as a mesh,
//! rotated, lit, projected (optionally with perspective) and painted back to
//! front. Flat caps (extrusion faces, cylinder ends) keep their exact curves.

use crate::DrawItem;
use kurbo::{BezPath, Point, Rect, Shape, Vec2};
use xenify_doc::{Color, FillRule, Homography, LineCap, LineJoin, Quad, Solid, SolidShape};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct V3 {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

pub(crate) const fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub(crate) fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub(crate) fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    pub(crate) fn scale(self, s: f64) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
    pub(crate) fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub(crate) fn cross(self, o: V3) -> V3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub(crate) fn normalize(self) -> V3 {
        let l = self.dot(self).sqrt();
        if l < 1e-12 { self } else { self.scale(1.0 / l) }
    }
}

/// One face of the mesh, in object space.
enum Face {
    /// A flat polygon, and a point inside the solid behind it (the normal
    /// points away from it).
    Poly(Vec<V3>, V3),
    /// A flat 2D path on the plane `origin + x·u + y·v`, facing `normal`.
    Cap { path: BezPath, origin: V3, u: V3, v: V3, normal: V3 },
    /// A model's face (outline, then holes) with its outward normal, its
    /// part's colour, and for smooth-shaded faces the corner normals.
    Oriented { loops: Vec<Vec<V3>>, normal: V3, corners: Option<Vec<V3>>, color: Option<Color> },
}

const IDENTITY3: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Rotation about X, then Y, then Z (degrees).
fn rotation(rx: f64, ry: f64, rz: f64) -> [[f64; 3]; 3] {
    let (sx, cx) = rx.to_radians().sin_cos();
    let (sy, cy) = ry.to_radians().sin_cos();
    let (sz, cz) = rz.to_radians().sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    let mul = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
        let mut o = [[0.0; 3]; 3];
        for (i, row) in o.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        o
    };
    mul(rz, mul(ry, rx))
}

fn apply3(m: &[[f64; 3]; 3], p: V3) -> V3 {
    v3(
        m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z,
        m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z,
        m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z,
    )
}

/// Rotation (X, then Y, then Z; degrees) and projection.
struct View {
    m: [[f64; 3]; 3],
    /// Camera distance; `None` = orthographic.
    camera: Option<f64>,
}

impl View {
    fn new(rx: f64, ry: f64, rz: f64, perspective: f64, extent: f64) -> Self {
        let (sx, cx) = rx.to_radians().sin_cos();
        let (sy, cy) = ry.to_radians().sin_cos();
        let (sz, cz) = rz.to_radians().sin_cos();
        let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
        let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
        let mul = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
            let mut o = [[0.0; 3]; 3];
            for (i, row) in o.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            o
        };
        let camera = (perspective > 1e-3).then(|| extent.max(1.0) * 2.5 / perspective.min(1.0));
        Self { m: mul(rz, mul(ry, rx)), camera }
    }

    /// View space → model space (the rotation undone).
    fn unrotate(&self, p: V3) -> V3 {
        let m = &self.m;
        v3(
            m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z,
            m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z,
            m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z,
        )
    }

    fn rotate(&self, p: V3) -> V3 {
        let m = &self.m;
        v3(
            m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z,
            m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z,
            m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z,
        )
    }

    /// Rotated point → screen (z grows away from the viewer).
    fn project(&self, p: V3) -> Point {
        match self.camera {
            Some(d) => {
                let f = d / (d + p.z).max(d * 0.05);
                Point::new(p.x * f, p.y * f)
            }
            None => Point::new(p.x, p.y),
        }
    }

    /// Whether a face at (rotated) `center` with (rotated) `normal` faces the viewer.
    fn facing(&self, center: V3, normal: V3) -> bool {
        let to_eye = match self.camera {
            Some(d) => v3(0.0, 0.0, -d).sub(center),
            None => v3(0.0, 0.0, -1.0),
        };
        normal.dot(to_eye) > 1e-9
    }
}

/// Direction towards the main light: upper left, in front.
fn light() -> V3 {
    v3(-0.45, -0.65, -0.6).normalize()
}

/// A softer light from the right and a little below, so faces turned away from the main one keep their shape.
fn fill_light() -> V3 {
    v3(0.6, 0.35, -0.7).normalize()
}

/// A main light, a fill light, more light from above than from below (as outdoors), and a soft highlight.
fn shade(base: Color, normal: V3) -> Color {
    let light = light();
    let diffuse = normal.dot(light).max(0.0);
    let fill = normal.dot(fill_light()).max(0.0);
    // y points down: faces looking up get the sky, faces looking down the darker ground.
    let sky = 0.5 - 0.5 * normal.y;
    let half = light.add(v3(0.0, 0.0, -1.0)).normalize();
    let spec = normal.dot(half).max(0.0).powi(32) * 0.3;
    let k = 0.22 + 0.16 * sky + 0.62 * diffuse + 0.18 * fill;
    let ch = |c: f64| (c * k + spec).clamp(0.0, 1.0);
    Color::rgba(ch(base.r), ch(base.g), ch(base.b), base.a)
}

fn circle(r: f64) -> BezPath {
    kurbo::Circle::new(Point::ZERO, r).to_path(0.05)
}

/// Builds the mesh for `solid` at `time`.
fn mesh(solid: &Solid, time: f64) -> (Vec<Face>, f64) {
    let size = solid.size.value_at(time).max(0.0);
    let depth = solid.depth.value_at(time).max(0.0);
    let (w, d) = (size / 2.0, depth / 2.0);
    let mut faces = Vec::new();
    let tau = std::f64::consts::TAU;
    match &solid.shape {
        SolidShape::Cube => {
            let c = |x: f64, y: f64, z: f64| v3(x * w, y * w, z * d);
            let quads = [
                [c(-1., -1., -1.), c(1., -1., -1.), c(1., 1., -1.), c(-1., 1., -1.)], // front (z-)
                [c(1., -1., 1.), c(-1., -1., 1.), c(-1., 1., 1.), c(1., 1., 1.)],     // back
                [c(-1., -1., 1.), c(1., -1., 1.), c(1., -1., -1.), c(-1., -1., -1.)], // top (y-)
                [c(-1., 1., -1.), c(1., 1., -1.), c(1., 1., 1.), c(-1., 1., 1.)],     // bottom
                [c(1., -1., -1.), c(1., -1., 1.), c(1., 1., 1.), c(1., 1., -1.)],     // right
                [c(-1., -1., 1.), c(-1., -1., -1.), c(-1., 1., -1.), c(-1., 1., 1.)], // left
            ];
            faces.extend(quads.into_iter().map(|q| Face::Poly(q.to_vec(), v3(0.0, 0.0, 0.0))));
        }
        // Drawn as a smoothly shaded disc instead (see `solid_items`).
        SolidShape::Sphere => {}
        SolidShape::Cylinder | SolidShape::Cone => {
            let n = 72;
            let cone = matches!(solid.shape, SolidShape::Cone);
            let rim = |i: usize, y: f64| {
                let a = i as f64 / n as f64 * tau;
                v3(a.cos() * w, y, a.sin() * w)
            };
            for i in 0..n {
                let axis = v3(0.0, 0.0, 0.0);
                if cone {
                    faces.push(Face::Poly(vec![rim(i + 1, d), rim(i, d), v3(0.0, -d, 0.0)], axis));
                } else {
                    faces.push(Face::Poly(vec![rim(i + 1, d), rim(i, d), rim(i, -d), rim(i + 1, -d)], axis));
                }
            }
            let (x, z) = (v3(1.0, 0.0, 0.0), v3(0.0, 0.0, 1.0));
            faces.push(Face::Cap { path: circle(w), origin: v3(0.0, d, 0.0), u: x, v: z, normal: v3(0.0, 1.0, 0.0) });
            if !cone {
                faces.push(Face::Cap {
                    path: circle(w),
                    origin: v3(0.0, -d, 0.0),
                    u: x,
                    v: z,
                    normal: v3(0.0, -1.0, 0.0),
                });
            }
        }
        SolidShape::Torus { thickness } => {
            let r_tube = w * thickness.clamp(0.05, 0.5);
            let r_ring = w - r_tube;
            let (nu, nv) = (48, 20);
            let p = |i: usize, j: usize| {
                let (a, b) = (i as f64 / nu as f64 * tau, j as f64 / nv as f64 * tau);
                let r = r_ring + r_tube * b.cos();
                v3(r * a.cos(), r_tube * b.sin(), r * a.sin())
            };
            for i in 0..nu {
                // The tube's centre line next to this ring of faces.
                let a = (i as f64 + 0.5) / nu as f64 * tau;
                let core = v3(r_ring * a.cos(), 0.0, r_ring * a.sin());
                for j in 0..nv {
                    faces.push(Face::Poly(vec![p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)], core));
                }
            }
        }
        SolidShape::Extrude { path, fill_rule } => {
            let (x, y) = (v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0));
            faces.push(Face::Cap {
                path: path.clone(),
                origin: v3(0.0, 0.0, -d),
                u: x,
                v: y,
                normal: v3(0.0, 0.0, -1.0),
            });
            faces.push(Face::Cap {
                path: path.clone(),
                origin: v3(0.0, 0.0, d),
                u: x,
                v: y,
                normal: v3(0.0, 0.0, 1.0),
            });
            if d > 1e-9 {
                // Inside/outside is judged against the same flattened outline the
                // walls follow (a curve bulges past its chords).
                let polys = polylines(path);
                let mut flat = BezPath::new();
                for poly in &polys {
                    flat.move_to(poly[0]);
                    for p in &poly[1..] {
                        flat.line_to(*p);
                    }
                    flat.close_path();
                }
                let inside = |p: Point| {
                    let wnd = flat.winding(p);
                    match fill_rule {
                        FillRule::NonZero => wnd != 0,
                        FillRule::EvenOdd => wnd % 2 != 0,
                    }
                };
                for poly in polys {
                    for k in 0..poly.len() {
                        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
                        let e = b - a;
                        if e.hypot() < 1e-9 {
                            continue;
                        }
                        // Outward = away from the filled area.
                        let mut n2 = Vec2::new(e.y, -e.x).normalize();
                        if inside(a.midpoint(b) + n2 * 0.25) {
                            n2 = -n2;
                        }
                        let behind = a.midpoint(b) - n2;
                        faces.push(Face::Poly(
                            vec![v3(a.x, a.y, -d), v3(b.x, b.y, -d), v3(b.x, b.y, d), v3(a.x, a.y, d)],
                            v3(behind.x, behind.y, 0.0),
                        ));
                    }
                }
            }
        }
        // Models are built (in drawing order) by `model_faces`.
        SolidShape::Mesh(_) => {}
    }
    let extent = match &solid.shape {
        SolidShape::Extrude { path, .. } => {
            let b = path.bounding_box();
            b.width().max(b.height()).max(depth)
        }
        SolidShape::Mesh(_) => size,
        _ => size.max(depth),
    };
    (faces, extent)
}

/// A polygon's normal (Newell's method: robust for concave and slightly
/// warped polygons), pointing out of a counter-clockwise outline.
pub(crate) fn newell(pts: &[V3]) -> V3 {
    let mut n = v3(0.0, 0.0, 0.0);
    for (i, a) in pts.iter().enumerate() {
        let b = pts[(i + 1) % pts.len()];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n.normalize()
}

/// An imported model's faces in drawing order: parts far to near, each
/// part's faces by its BSP tree (or by depth when too dense for one).
fn model_faces(solid: &Solid, m: &xenify_doc::Mesh, time: f64, view: &View) -> Vec<Face> {
    use crate::solid_bsp;
    let size = solid.size.value_at(time).max(0.0);
    struct PartDraw {
        depth: f64,
        order: std::sync::Arc<solid_bsp::Order>,
        turn: [[f64; 3]; 3],
        centre: V3,
        shift: V3,
        color: Option<Color>,
    }
    let mut parts: Vec<PartDraw> = Vec::new();
    for (k, part) in m.parts.iter().enumerate() {
        let motion = solid.parts.get(k);
        if motion.is_some_and(|mo| !mo.visible) {
            continue;
        }
        let c = part.bounds().map_or(v3(0.0, 0.0, 0.0), |(lo, hi)| {
            v3((lo[0] + hi[0]) / 2.0 * size, (lo[1] + hi[1]) / 2.0 * size, (lo[2] + hi[2]) / 2.0 * size)
        });
        let (turn, shift) = match motion {
            Some(mo) => {
                let o = mo.offset.value_at(time);
                (
                    rotation(mo.rot_x.value_at(time), mo.rot_y.value_at(time), mo.rot_z.value_at(time)),
                    v3(o.x, o.y, mo.depth.value_at(time)),
                )
            }
            None => (IDENTITY3, v3(0.0, 0.0, 0.0)),
        };
        parts.push(PartDraw {
            depth: view.rotate(c.add(shift)).z,
            order: solid_bsp::order(part),
            turn,
            centre: c,
            shift,
            color: if m.own_colors { part.color } else { None },
        });
    }
    parts.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    let mut out = Vec::new();
    for p in &parts {
        let transpose =
            |m: &[[f64; 3]; 3]| [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]];
        let back = transpose(&p.turn);
        // The eye in the part's unit space.
        let (eye, toward) = match view.camera {
            Some(d) => {
                let world = view.unrotate(v3(0.0, 0.0, -d));
                let local = apply3(&back, world.sub(p.centre).sub(p.shift)).add(p.centre).scale(1.0 / size.max(1e-9));
                (Some(local), v3(0.0, 0.0, 0.0))
            }
            None => (None, apply3(&back, view.unrotate(v3(0.0, 0.0, -1.0)))),
        };
        let mut ordered = Vec::new();
        solid_bsp::back_to_front(&p.order, eye, toward, &mut ordered);
        let place = |u: V3| apply3(&p.turn, u.scale(size).sub(p.centre)).add(p.centre).add(p.shift);
        let mut faces: Vec<(f64, Face)> = ordered
            .into_iter()
            .map(|f| {
                let loops: Vec<Vec<V3>> = f.loops.iter().map(|l| l.iter().map(|q| place(*q)).collect()).collect();
                let z = view
                    .rotate(
                        loops[0].iter().fold(v3(0.0, 0.0, 0.0), |s, q| s.add(*q)).scale(1.0 / loops[0].len() as f64),
                    )
                    .z;
                let face = Face::Oriented {
                    normal: apply3(&p.turn, f.normal),
                    corners: f.corners.as_ref().map(|c| c.iter().map(|n| apply3(&p.turn, *n)).collect()),
                    loops,
                    color: p.color,
                };
                (z, face)
            })
            .collect();
        if !solid_bsp::is_exact(&p.order) {
            faces.sort_by(|a, b| b.0.total_cmp(&a.0));
        }
        out.extend(faces.into_iter().map(|(_, f)| f));
    }
    out
}

/// Closed polylines approximating each subpath of `path` (for extrusion walls).
fn polylines(path: &BezPath) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    let tol = (path.bounding_box().size().width.max(path.bounding_box().size().height) * 0.004).clamp(0.2, 2.0);
    kurbo::flatten(path, tol, |el| match el {
        kurbo::PathEl::MoveTo(p) => out.push(vec![p]),
        kurbo::PathEl::LineTo(p) => {
            if let Some(last) = out.last_mut()
                && last.last().is_none_or(|q| q.distance(p) > 1e-9)
            {
                last.push(p);
            }
        }
        _ => {}
    });
    for poly in &mut out {
        if poly.len() > 1 && poly[0].distance(*poly.last().unwrap()) < 1e-9 {
            poly.pop();
        }
    }
    out.retain(|p| p.len() >= 3);
    out
}

/// The solid's shaded faces in layer space, back to front.
pub fn solid_items(solid: &Solid, time: f64) -> Vec<DrawItem> {
    let (mut faces, extent) = mesh(solid, time);
    let view = View::new(
        solid.rot_x.value_at(time),
        solid.rot_y.value_at(time),
        solid.rot_z.value_at(time),
        solid.perspective,
        extent,
    );
    // Models come already in drawing order.
    let presorted = if let SolidShape::Mesh(m) = &solid.shape {
        faces = model_faces(solid, m, time, &view);
        true
    } else {
        false
    };
    let base = solid.color.value_at(time);
    if matches!(solid.shape, SolidShape::Sphere) {
        return vec![sphere(base, solid.size.value_at(time).max(0.0) / 2.0)];
    }
    let mut out: Vec<(f64, DrawItem)> = Vec::new();
    for (k, face) in faces.into_iter().enumerate() {
        match face {
            Face::Poly(pts, behind) => {
                let r: Vec<V3> = pts.iter().map(|p| view.rotate(*p)).collect();
                let center = r.iter().fold(v3(0.0, 0.0, 0.0), |s, p| s.add(*p)).scale(1.0 / r.len() as f64);
                let mut normal = r[1].sub(r[0]).cross(r[2].sub(r[0])).normalize();
                if normal.dot(center.sub(view.rotate(behind))) < 0.0 {
                    normal = normal.scale(-1.0);
                }
                if !view.facing(center, normal) {
                    continue;
                }
                let mut path = BezPath::new();
                for (i, p) in r.iter().enumerate() {
                    let q = view.project(*p);
                    if i == 0 {
                        path.move_to(q);
                    } else {
                        path.line_to(q);
                    }
                }
                path.close_path();
                out.push((center.z, face_item(path, shade(base, normal), FillRule::NonZero)));
            }
            Face::Oriented { loops, normal, corners, color } => {
                let r: Vec<V3> = loops[0].iter().map(|p| view.rotate(*p)).collect();
                let center = r.iter().fold(v3(0.0, 0.0, 0.0), |s, p| s.add(*p)).scale(1.0 / r.len() as f64);
                // Painted in the order given (the sort below keeps it).
                let key = if presorted { -(k as f64) } else { center.z };
                let normal = view.rotate(normal);
                if !view.facing(center, normal) {
                    continue;
                }
                let screen: Vec<Point> = r.iter().map(|p| view.project(*p)).collect();
                let mut path = outline(screen.iter().copied());
                for hole in &loops[1..] {
                    path.extend(outline(hole.iter().map(|p| view.project(view.rotate(*p)))));
                }
                let rule = if loops.len() > 1 { FillRule::EvenOdd } else { FillRule::NonZero };
                let base = color.unwrap_or(base);
                let mut item = face_item(path, shade(base, normal), rule);
                if let Some(c) = corners {
                    let colors: Vec<Color> = c.iter().map(|n| shade(base, view.rotate(*n))).collect();
                    item.gradient = smooth(&screen, &colors);
                    if item.gradient.is_some() {
                        // A flat-coloured hairline would show the mesh: instead
                        // grow the face half a pixel so neighbours overlap.
                        item.path = outline(grow(&screen, 0.5).into_iter());
                        item.stroke = None;
                    }
                }
                out.push((key, item));
            }
            Face::Cap { path, origin, u, v, normal } => {
                let n = view.rotate(normal);
                let b = path.bounding_box();
                if b.width() <= 0.0 || b.height() <= 0.0 {
                    continue;
                }
                let corners = Quad::of_rect(b);
                let world = |p: Point| view.rotate(origin.add(u.scale(p.x)).add(v.scale(p.y)));
                let center = world(b.center());
                if !view.facing(center, n) {
                    continue;
                }
                let screen = corners.map(|p| view.project(world(p)));
                let Some(h) = Homography::from_quads(&corners, &screen) else { continue };
                let fill_rule = match &solid.shape {
                    SolidShape::Extrude { fill_rule, .. } => *fill_rule,
                    _ => FillRule::NonZero,
                };
                out.push((center.z, face_item(h.map_path(&path), shade(base, n), fill_rule)));
            }
        }
    }
    // Painter's algorithm: farthest first.
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().map(|(_, item)| item).collect()
}

/// A sphere looks the same from every side: a disc shaded by a radial
/// gradient centred on the highlight.
fn sphere(base: Color, r: f64) -> DrawItem {
    let l = light();
    let hi = Vec2::new(l.x, l.y) * r * 0.55;
    let stops = [0.0, 0.35, 0.7, 1.0]
        .map(|t: f64| {
            // The normal at that fraction of the way from the highlight to the far rim.
            let a = t * std::f64::consts::FRAC_PI_2 * 1.25;
            let n = l.scale(a.cos()).add(v3(-l.x, -l.y, 0.0).normalize().scale(a.sin())).normalize();
            xenify_doc::GradientStop { offset: t, color: shade(base, n) }
        })
        .to_vec();
    let gradient = xenify_doc::Gradient {
        kind: xenify_doc::GradientKind::Radial,
        start: hi,
        end: hi + Vec2::new(r + hi.hypot(), 0.0),
        stops,
    };
    DrawItem {
        path: circle(r),
        fill_rule: FillRule::NonZero,
        gradient: Some(gradient),
        fill: Some(base),
        stroke: None,
        pattern: None,
    }
}

/// A linear gradient blending a face's corner colours (Gouraud shading
/// along the direction brightness changes fastest), fitted through the
/// darkest corner, the brightest and the one farthest from the line between
/// them. `None` when the corners are the same colour or the face is edge-on.
fn smooth(p: &[Point], c: &[Color]) -> Option<xenify_doc::Gradient> {
    let lum = |c: &Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    let s: Vec<f64> = c.iter().map(lum).collect();
    let min = (0..s.len()).min_by(|a, b| s[*a].total_cmp(&s[*b]))?;
    let max = (0..s.len()).max_by(|a, b| s[*a].total_cmp(&s[*b]))?;
    if s[max] - s[min] < 1.0 / 512.0 {
        return None;
    }
    let area = |k: usize| (p[max] - p[min]).cross(p[k] - p[min]).abs();
    let third = (0..p.len()).filter(|k| *k != min && *k != max).max_by(|a, b| area(*a).total_cmp(&area(*b)))?;
    let [i0, i1, i2] = [min, max, third];
    // Solve for g with g·(p1 − p0) = s1 − s0 and g·(p2 − p0) = s2 − s0.
    let (a, b) = (p[i1] - p[i0], p[i2] - p[i0]);
    let det = a.x * b.y - a.y * b.x;
    if det.abs() < 1e-6 {
        return None;
    }
    let (d1, d2) = (s[i1] - s[i0], s[i2] - s[i0]);
    let g = Vec2::new((d1 * b.y - d2 * a.y) / det, (a.x * d2 - b.x * d1) / det);
    let gg = g.hypot2();
    Some(xenify_doc::Gradient {
        kind: xenify_doc::GradientKind::Linear,
        start: p[min].to_vec2(),
        end: p[min].to_vec2() + g * ((s[max] - s[min]) / gg),
        stops: vec![
            xenify_doc::GradientStop { offset: 0.0, color: quantize(c[min]) },
            xenify_doc::GradientStop { offset: 1.0, color: quantize(c[max]) },
        ],
    })
}

/// Rounds a colour so faces share the renderer's cached colour ramps
/// (thousands of distinct gradients are otherwise slow to draw).
fn quantize(c: Color) -> Color {
    let q = |v: f64| (v * 160.0).round() / 160.0;
    Color::rgba(q(c.r), q(c.g), q(c.b), c.a)
}

/// A convex polygon with every edge moved `d` outwards (miters capped).
fn grow(p: &[Point], d: f64) -> Vec<Point> {
    let n = p.len();
    let area: f64 = (0..n).map(|k| p[k].to_vec2().cross(p[(k + 1) % n].to_vec2())).sum();
    let out = |a: Point, b: Point| {
        let e = (b - a).normalize();
        if area > 0.0 { Vec2::new(e.y, -e.x) } else { Vec2::new(-e.y, e.x) }
    };
    (0..n)
        .map(|k| {
            let (prev, cur, next) = (p[(k + n - 1) % n], p[k], p[(k + 1) % n]);
            let (n1, n2) = (out(prev, cur), out(cur, next));
            if !(n1.is_finite() && n2.is_finite()) {
                return cur;
            }
            let m = (n1 + n2) / (1.0 + n1.dot(n2)).max(0.25);
            cur + m * d
        })
        .collect()
}

fn outline(pts: impl Iterator<Item = Point>) -> BezPath {
    let mut path = BezPath::new();
    for (i, q) in pts.enumerate() {
        if i == 0 {
            path.move_to(q);
        } else {
            path.line_to(q);
        }
    }
    path.close_path();
    path
}

fn face_item(path: BezPath, color: Color, fill_rule: FillRule) -> DrawItem {
    DrawItem {
        pattern: None,
        path,
        fill_rule,
        gradient: None,
        fill: Some(color),
        // A hairline in the same colour hides anti-aliasing seams between faces.
        stroke: Some(crate::SceneStroke::solid(color, 0.8, LineJoin::Round, LineCap::Round)),
    }
}

/// The 2D box a solid covers at `time` (for new layers and selection boxes).
pub fn solid_bounds(solid: &Solid, time: f64) -> Option<Rect> {
    solid_items(solid, time).iter().map(|i| i.path.bounding_box()).reduce(|a, b| a.union(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_seen_at_an_angle_shows_three_shaded_faces() {
        let mut cube = Solid::new(SolidShape::Cube, 100.0, 100.0, Color::rgb(0.2, 0.5, 0.9));
        cube.perspective = 0.0;
        let items = solid_items(&cube, 0.0);
        assert_eq!(items.len(), 3, "front, top and one side are visible");
        let fills: Vec<Color> = items.iter().filter_map(|i| i.fill).collect();
        assert!(fills.windows(2).any(|w| w[0] != w[1]), "faces are shaded differently");
        // Face-on: only the front.
        cube.rot_x = 0.0.into();
        cube.rot_y = 0.0.into();
        let front = solid_items(&cube, 0.0);
        assert_eq!(front.len(), 1);
        let b = front[0].path.bounding_box();
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn extruding_a_shape_with_a_hole_draws_walls_and_caps() {
        let mut path = kurbo::Rect::new(-50.0, -50.0, 50.0, 50.0).to_path(0.1);
        path.extend(kurbo::Rect::new(-20.0, -20.0, 20.0, 20.0).to_path(0.1).reverse_subpaths());
        let s = Solid::new(SolidShape::Extrude { path, fill_rule: FillRule::EvenOdd }, 0.0, 40.0, Color::WHITE);
        let items = solid_items(&s, 0.0);
        // Front cap + outer walls facing us (2) + inner walls facing us (2).
        assert!(items.len() >= 5, "{}", items.len());
        assert!(items.iter().any(|i| i.fill_rule == FillRule::EvenOdd), "the cap keeps its hole");
        // Curved outlines get walls all the way round (none culled by mistake).
        let round = kurbo::Rect::new(-50.0, -50.0, 50.0, 50.0).to_rounded_rect(24.0).to_path(0.05);
        let mut flat =
            Solid::new(SolidShape::Extrude { path: round, fill_rule: FillRule::NonZero }, 0.0, 40.0, Color::WHITE);
        flat.perspective = 0.0;
        flat.rot_x = 0.0.into();
        flat.rot_y = 60.0.into();
        let walls = solid_items(&flat, 0.0).len();
        // Front cap + every wall on the visible half (straight side + two corners' worth of pieces).
        assert!(walls > 6, "{walls}");
        for shape in SolidShape::primitives() {
            let s = Solid::new(shape, 100.0, 80.0, Color::WHITE);
            let b = solid_bounds(&s, 0.0).unwrap();
            assert!(b.width() > 50.0 && b.width() < 200.0, "{b:?}");
        }
    }

    /// A unit cube as a model: corners ±0.5, faces counter-clockwise from outside.
    pub(crate) fn cube_mesh() -> xenify_doc::Mesh {
        let positions: Vec<f32> = (0..8)
            .flat_map(|i| [(i & 1) as f32 - 0.5, ((i >> 1) & 1) as f32 - 0.5, ((i >> 2) & 1) as f32 - 0.5])
            .collect();
        // Right-handed with y down and z away: outward normals via Newell.
        let quads: [[u32; 4]; 6] = [[2, 3, 1, 0], [5, 7, 6, 4], [1, 5, 4, 0], [6, 7, 3, 2], [4, 6, 2, 0], [3, 7, 5, 1]];
        let faces = xenify_doc::MeshPart::encode_polygons(quads.iter().map(|q| &q[..]));
        let part = xenify_doc::MeshPart {
            name: "cube".into(),
            color: Some(Color::rgb(1.0, 0.0, 0.0)),
            positions: positions.into(),
            faces: faces.into(),
            shading: Default::default(),
        };
        xenify_doc::Mesh { name: "cube.stl".into(), parts: vec![part], own_colors: false }
    }

    #[test]
    fn a_model_draws_its_visible_faces() {
        let mesh = cube_mesh();
        for (i, poly) in mesh.parts[0].polygons().enumerate() {
            let pts: Vec<V3> = poly
                .iter()
                .map(|k| {
                    let [x, y, z] = mesh.parts[0].vertex(*k);
                    v3(x, y, z)
                })
                .collect();
            let c = pts.iter().fold(v3(0.0, 0.0, 0.0), |s, p| s.add(*p)).scale(0.25);
            assert!(newell(&pts).dot(c) > 0.0, "face {i} points outwards");
        }
        let mut s = Solid::new(SolidShape::Mesh(Box::new(mesh)), 100.0, 0.0, Color::WHITE);
        s.perspective = 0.0;
        assert_eq!(solid_items(&s, 0.0).len(), 3, "three faces seen from above and to the side");
        s.rot_x = 0.0.into();
        s.rot_y = 0.0.into();
        let front = solid_items(&s, 0.0);
        assert_eq!(front.len(), 1);
        let b = front[0].path.bounding_box();
        assert!((b.width() - 100.0).abs() < 1e-3, "{b:?}");
        assert_ne!(front[0].fill, Some(Color::rgb(1.0, 0.0, 0.0)), "the layer's colour unless own colours");
        let SolidShape::Mesh(m) = &mut s.shape else { unreachable!() };
        m.own_colors = true;
        let red = solid_items(&s, 0.0)[0].fill.unwrap();
        assert!(red.r > 0.3 && red.g < 0.2 && red.b < 0.2, "{red:?}");
    }

    #[test]
    fn parts_move_turn_and_hide_on_their_own() {
        use xenify_doc::PartMotion;
        // Two unit cubes side by side (the second shifted right).
        let mut mesh = cube_mesh();
        let mut second = mesh.parts[0].clone();
        let shifted: Vec<f32> =
            second.positions.iter().enumerate().map(|(i, v)| if i % 3 == 0 { v + 2.0 } else { *v }).collect();
        second.positions = shifted.into();
        mesh.parts.push(second);
        let mut s = Solid::new(SolidShape::Mesh(Box::new(mesh)), 100.0, 0.0, Color::WHITE);
        (s.perspective, s.rot_x, s.rot_y) = (0.0, 0.0.into(), 0.0.into());
        let right_edge =
            |s: &Solid| solid_items(s, 0.0).iter().map(|i| i.path.bounding_box().x1).fold(f64::MIN, f64::max);
        assert!((right_edge(&s) - 250.0).abs() < 1e-6, "{}", right_edge(&s));
        // Move the second part 40 px further right.
        s.parts = vec![
            PartMotion::default(),
            PartMotion { offset: xenify_doc::Vec2::new(40.0, 0.0).into(), ..Default::default() },
        ];
        assert!((right_edge(&s) - 290.0).abs() < 1e-6);
        // Turn it 45° about its own centre: its front face is wider on screen.
        s.parts[1] = PartMotion { rot_y: 45.0.into(), ..Default::default() };
        assert!(right_edge(&s) > 255.0);
        // Hidden: only the first cube is drawn.
        s.parts[1] = PartMotion { visible: false, ..Default::default() };
        assert!((right_edge(&s) - 50.0).abs() < 1e-6);
    }
}
