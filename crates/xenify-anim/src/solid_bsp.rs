//! Drawing order for imported 3D models. Painting faces far to near by
//! their centres goes wrong for concave shapes (a face's centre can be
//! nearer than a face it partly covers), so each part gets a BSP tree, built
//! once in the part's own space: walking it with the eye position gives an
//! exactly right back-to-front order from any angle (faces straddling a
//! splitting plane are cut in two). Very dense parts, where centre sorting
//! looks right anyway, skip the tree.

use super::solid::{V3, v3};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use xenify_doc::MeshPart;

/// Parts with more faces than this are sorted by centre depth instead.
const MAX_BSP_FACES: usize = 6000;
/// Give up on the tree if cutting multiplies the faces beyond this.
const MAX_GROWTH: usize = 4;

/// One face in the part's own (unit) space.
#[derive(Clone, Debug)]
pub(crate) struct PFace {
    /// Outline, then holes.
    pub loops: Vec<Vec<V3>>,
    pub normal: V3,
    /// Smooth-shading normals per outline corner.
    pub corners: Option<Vec<V3>>,
}

pub(crate) struct Node {
    n: V3,
    d: f64,
    faces: Vec<PFace>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
}

pub(crate) enum Order {
    Tree(Node),
    /// Too dense (or cutting went wild): sort by depth when drawing.
    Unsorted(Vec<PFace>),
}

thread_local! {
    static CACHE: RefCell<HashMap<(usize, usize, usize), Arc<Order>>> = RefCell::new(HashMap::new());
}

/// The part's faces in unit space, ready to be ordered (cached per part data).
pub(crate) fn order(part: &MeshPart) -> Arc<Order> {
    let key = (part.positions.as_ptr() as usize, part.faces.as_ptr() as usize, part.shading.as_ptr() as usize);
    if let Some(o) = CACHE.with_borrow(|c| c.get(&key).cloned()) {
        return o;
    }
    let faces = unit_faces(part);
    let o = Arc::new(if faces.len() > MAX_BSP_FACES {
        Order::Unsorted(faces)
    } else {
        let limit = faces.len() * MAX_GROWTH + 16;
        let mut count = 0;
        match build(faces.clone(), &mut count, limit) {
            Some(node) => Order::Tree(node),
            None => Order::Unsorted(faces),
        }
    });
    CACHE.with_borrow_mut(|c| {
        // Models are few; forget everything if the cache grows (e.g. many edits).
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, o.clone());
    });
    o
}

fn unit_faces(part: &MeshPart) -> Vec<PFace> {
    let point = |i: &u32| {
        let [x, y, z] = part.vertex(*i);
        v3(x, y, z)
    };
    let mut corner = 0;
    part.faces()
        .map(|face| {
            let outline: Vec<V3> = face.outline.iter().map(point).collect();
            let normal = super::solid::newell(&outline);
            let mut corners = None;
            if MeshPart::smoothable(&face) {
                corners = (0..outline.len())
                    .map(|k| part.corner_normal(corner + k).map(|[x, y, z]| v3(x, y, z).normalize()))
                    .collect::<Option<Vec<V3>>>();
                corner += outline.len();
            }
            let mut loops = vec![outline];
            loops.extend(face.holes().map(|h| h.iter().map(point).collect()));
            PFace { loops, normal, corners }
        })
        .collect()
}

const EPS: f64 = 1e-7;

fn build(mut faces: Vec<PFace>, count: &mut usize, limit: usize) -> Option<Node> {
    if faces.is_empty() {
        return None;
    }
    // Split along the biggest of a few faces (fewer cuts, flatter tree).
    let pick = (0..faces.len().min(8)).max_by(|a, b| area(&faces[*a]).total_cmp(&area(&faces[*b])))?;
    let splitter = faces.swap_remove(pick);
    let n = splitter.normal;
    let d = n.dot(splitter.loops[0][0]);
    let mut node = Node { n, d, faces: vec![splitter], front: None, back: None };
    let (mut front, mut back) = (Vec::new(), Vec::new());
    for f in faces {
        let side = |p: V3| n.dot(p) - d;
        let dists: Vec<f64> = f.loops[0].iter().map(|p| side(*p)).collect();
        let (pos, neg) = (dists.iter().any(|v| *v > EPS), dists.iter().any(|v| *v < -EPS));
        match (pos, neg) {
            (false, false) => node.faces.push(f),
            (true, false) => front.push(f),
            (false, true) => back.push(f),
            (true, true) if f.loops.len() > 1 => {
                // Faces with holes aren't cut: they go by their centre.
                let c = centroid(&f.loops[0]);
                if side(c) >= 0.0 { front.push(f) } else { back.push(f) }
            }
            (true, true) => {
                let (a, b) = split(&f, &dists);
                *count += 1;
                if *count > limit {
                    return None;
                }
                if let Some(a) = a {
                    front.push(a);
                }
                if let Some(b) = b {
                    back.push(b);
                }
            }
        }
    }
    node.front = if front.is_empty() { None } else { Some(Box::new(build(front, count, limit)?)) };
    node.back = if back.is_empty() { None } else { Some(Box::new(build(back, count, limit)?)) };
    Some(node)
}

fn area(f: &PFace) -> f64 {
    let l = &f.loops[0];
    let mut s = v3(0.0, 0.0, 0.0);
    for i in 0..l.len() {
        s = s.add(l[i].cross(l[(i + 1) % l.len()]));
    }
    s.dot(s).sqrt()
}

fn centroid(l: &[V3]) -> V3 {
    l.iter().fold(v3(0.0, 0.0, 0.0), |s, p| s.add(*p)).scale(1.0 / l.len().max(1) as f64)
}

/// Cuts a face (no holes) along the plane; `dists` are its corners'
/// signed distances. Returns the front and back pieces.
fn split(f: &PFace, dists: &[f64]) -> (Option<PFace>, Option<PFace>) {
    let pts = &f.loops[0];
    let n = pts.len();
    let (mut fp, mut bp) = (Vec::new(), Vec::new());
    let (mut fc, mut bc) = (Vec::new(), Vec::new());
    let corner = |i: usize| f.corners.as_ref().map(|c| c[i]);
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (pts[i], pts[j]);
        let (da, db) = (dists[i], dists[j]);
        if da >= -EPS {
            fp.push(a);
            fc.push(corner(i));
        }
        if da <= EPS {
            bp.push(a);
            bc.push(corner(i));
        }
        if (da > EPS && db < -EPS) || (da < -EPS && db > EPS) {
            let t = da / (da - db);
            let p = a.add(b.sub(a).scale(t));
            let c = match (corner(i), corner(j)) {
                (Some(x), Some(y)) => Some(x.add(y.sub(x).scale(t)).normalize()),
                _ => None,
            };
            fp.push(p);
            fc.push(c);
            bp.push(p);
            bc.push(c);
        }
    }
    let piece = |loop_: Vec<V3>, cs: Vec<Option<V3>>| {
        (loop_.len() >= 3).then(|| PFace {
            loops: vec![loop_],
            normal: f.normal,
            corners: if f.corners.is_some() { cs.into_iter().collect() } else { None },
        })
    };
    (piece(fp, fc), piece(bp, bc))
}

/// Faces far to near as seen from `eye` (a point in the part's unit space),
/// or looking along `-dir` from infinitely far (orthographic, `eye` None).
pub(crate) fn back_to_front<'a>(order: &'a Order, eye: Option<V3>, toward_eye: V3, out: &mut Vec<&'a PFace>) {
    match order {
        Order::Unsorted(faces) => out.extend(faces.iter()),
        Order::Tree(node) => walk(node, eye, toward_eye, out),
    }
}

fn walk<'a>(node: &'a Node, eye: Option<V3>, toward_eye: V3, out: &mut Vec<&'a PFace>) {
    let in_front = match eye {
        Some(e) => node.n.dot(e) - node.d > 0.0,
        None => node.n.dot(toward_eye) > 0.0,
    };
    let (far, near) = if in_front { (&node.back, &node.front) } else { (&node.front, &node.back) };
    if let Some(f) = far {
        walk(f, eye, toward_eye, out);
    }
    out.extend(node.faces.iter());
    if let Some(n) = near {
        walk(n, eye, toward_eye, out);
    }
}

/// Whether the order came from a tree (so needs no sorting).
pub(crate) fn is_exact(order: &Order) -> bool {
    matches!(order, Order::Tree(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(pts: [V3; 4]) -> PFace {
        let loops = vec![pts.to_vec()];
        PFace { normal: super::super::solid::newell(&loops[0]), loops, corners: None }
    }

    #[test]
    fn a_wall_on_a_long_floor_is_drawn_after_the_floor() {
        // y is down: the floor is the plane y = 0 facing up (−y), running far back
        // (z to 3); a wall stands on it near the back (z = 2.5), facing the viewer (−z).
        let floor = quad([v3(1.0, 0.0, -1.0), v3(1.0, 0.0, 3.0), v3(-1.0, 0.0, 3.0), v3(-1.0, 0.0, -1.0)]);
        let wall = quad([v3(0.5, 0.0, 2.5), v3(0.5, -1.0, 2.5), v3(-0.5, -1.0, 2.5), v3(-0.5, 0.0, 2.5)]);
        assert!(floor.normal.y < 0.0 && wall.normal.z < 0.0, "{:?} {:?}", floor.normal, wall.normal);
        // By centre the wall is farther from an eye above and in front, so a
        // depth sort would paint the floor over the wall's foot.
        let eye = v3(0.0, -5.0, -5.0);
        let dist = |f: &PFace| {
            let c = centroid(&f.loops[0]).sub(eye);
            c.dot(c)
        };
        assert!(dist(&wall) > dist(&floor));
        for faces in [vec![floor.clone(), wall.clone()], vec![wall.clone(), floor.clone()]] {
            let tree = Order::Tree(build(faces, &mut 0, 100).unwrap());
            let mut out = Vec::new();
            back_to_front(&tree, Some(eye), v3(0.0, 0.0, 0.0), &mut out);
            let pos = |f: &PFace| out.iter().position(|g| g.loops == f.loops).unwrap();
            assert!(pos(&floor) < pos(&wall), "the floor first, then the wall on it");
        }
    }

    #[test]
    fn straddling_faces_are_cut_in_two() {
        // A floor, and a wall passing through it (half above, half below).
        let floor = quad([v3(-2.0, 0.0, -2.0), v3(-2.0, 0.0, 2.0), v3(2.0, 0.0, 2.0), v3(2.0, 0.0, -2.0)]);
        let wall = quad([v3(-1.0, 1.0, 0.0), v3(-1.0, -1.0, 0.0), v3(1.0, -1.0, 0.0), v3(1.0, 1.0, 0.0)]);
        let tree = Order::Tree(build(vec![floor, wall], &mut 0, 100).unwrap());
        let mut out = Vec::new();
        back_to_front(&tree, Some(v3(0.0, -5.0, -5.0)), v3(0.0, 0.0, 0.0), &mut out);
        assert_eq!(out.len(), 3, "the wall split at the floor");
        // Below the floor first, then the floor, then the part above.
        let ys: Vec<f64> = out.iter().map(|f| centroid(&f.loops[0]).y).collect();
        assert!(ys[0] > 0.0 && ys[1] == 0.0 && ys[2] < 0.0, "{ys:?}");
    }
}
