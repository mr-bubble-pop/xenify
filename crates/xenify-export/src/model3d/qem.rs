//! Mesh simplification by quadric error metrics (Garland & Heckbert):
//! repeatedly collapse the edge whose removal changes the surface least,
//! keeping open edges in place and never folding triangles over.

use super::{RawPart, cross, dot, sub};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// A symmetric 4×4 matrix (10 numbers) measuring squared distance to planes.
#[derive(Clone, Copy, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    /// Distance² to the plane `n·x + d = 0` (n unit), times `w`.
    fn plane(n: [f64; 3], d: f64, w: f64) -> Self {
        let [a, b, c] = n;
        Self([
            a * a * w,
            a * b * w,
            a * c * w,
            a * d * w,
            b * b * w,
            b * c * w,
            b * d * w,
            c * c * w,
            c * d * w,
            d * d * w,
        ])
    }

    fn add(&mut self, o: &Quadric) {
        for (x, y) in self.0.iter_mut().zip(o.0) {
            *x += y;
        }
    }

    fn error(&self, p: [f64; 3]) -> f64 {
        let q = &self.0;
        let [x, y, z] = p;
        q[0] * x * x
            + 2.0 * q[1] * x * y
            + 2.0 * q[2] * x * z
            + 2.0 * q[3] * x
            + q[4] * y * y
            + 2.0 * q[5] * y * z
            + 2.0 * q[6] * y
            + q[7] * z * z
            + 2.0 * q[8] * z
            + q[9]
    }

    /// The point with the least error, if the quadric pins one down.
    fn optimum(&self) -> Option<[f64; 3]> {
        let q = &self.0;
        let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
        let rhs = [-q[3], -q[6], -q[8]];
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let scale = m.iter().flatten().fold(0.0f64, |a, v| a.max(v.abs())).max(1e-300);
        if det.abs() < 1e-9 * scale * scale * scale {
            return None;
        }
        let solve = |col: usize| {
            let mut mm = m;
            for r in 0..3 {
                mm[r][col] = rhs[r];
            }
            (mm[0][0] * (mm[1][1] * mm[2][2] - mm[1][2] * mm[2][1])
                - mm[0][1] * (mm[1][0] * mm[2][2] - mm[1][2] * mm[2][0])
                + mm[0][2] * (mm[1][0] * mm[2][1] - mm[1][1] * mm[2][0]))
                / det
        };
        Some([solve(0), solve(1), solve(2)])
    }
}

/// A candidate collapse, cheapest first in the heap.
struct Candidate {
    cost: f64,
    a: u32,
    b: u32,
    /// The vertices' versions when this was computed (stale entries are skipped).
    va: u32,
    vb: u32,
    target: [f64; 3],
}

impl PartialEq for Candidate {
    fn eq(&self, o: &Self) -> bool {
        self.cost == o.cost
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

/// Simplifies `p` (welded) to about `target` triangles.
pub fn simplify(p: &mut RawPart, target: usize) {
    let n = p.positions.len();
    if p.triangles.len() <= target || n < 4 {
        return;
    }
    let mut pos = p.positions.clone();
    let mut tris: Vec<[u32; 3]> = p.triangles.clone();
    let mut alive = vec![true; tris.len()];
    let mut vfaces: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (f, t) in tris.iter().enumerate() {
        for v in t {
            vfaces[*v as usize].push(f as u32);
        }
    }
    // Plane quadrics of the faces around each vertex (area-weighted).
    let mut q = vec![Quadric::default(); n];
    for t in &tris {
        let [a, b, c] = t.map(|i| pos[i as usize]);
        let nrm = cross(sub(b, a), sub(c, a));
        let len = dot(nrm, nrm).sqrt();
        if len < 1e-300 {
            continue;
        }
        let unit = nrm.map(|x| x / len);
        let plane = Quadric::plane(unit, -dot(unit, a), len / 2.0);
        for v in t {
            q[*v as usize].add(&plane);
        }
    }
    // Open edges (used by one triangle) get a steep wall so they stay put.
    let mut edge_use: HashMap<(u32, u32), u32> = HashMap::new();
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]));
            *edge_use.entry((a, b)).or_default() += 1;
        }
    }
    for t in &tris {
        let [a, b, c] = t.map(|i| pos[i as usize]);
        let fnorm = cross(sub(b, a), sub(c, a));
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            if edge_use[&(i.min(j), i.max(j))] != 1 {
                continue;
            }
            let (pi, pj) = (pos[i as usize], pos[j as usize]);
            let e = sub(pj, pi);
            let wall = cross(e, fnorm);
            let len = dot(wall, wall).sqrt();
            if len < 1e-300 {
                continue;
            }
            let unit = wall.map(|x| x / len);
            let w = dot(e, e) * 1000.0;
            let plane = Quadric::plane(unit, -dot(unit, pi), w);
            q[i as usize].add(&plane);
            q[j as usize].add(&plane);
        }
    }

    let mut version = vec![0u32; n];
    let mut dead = vec![false; n];
    let candidate = |a: u32, b: u32, q: &[Quadric], pos: &[[f64; 3]], version: &[u32]| {
        let mut qq = q[a as usize];
        qq.add(&q[b as usize]);
        let (pa, pb) = (pos[a as usize], pos[b as usize]);
        let mid = [0, 1, 2].map(|k| (pa[k] + pb[k]) / 2.0);
        let mut best = (qq.error(mid), mid);
        for p in [qq.optimum(), Some(pa), Some(pb)].into_iter().flatten() {
            let e = qq.error(p);
            if e < best.0 {
                best = (e, p);
            }
        }
        Candidate { cost: best.0, a, b, va: version[a as usize], vb: version[b as usize], target: best.1 }
    };
    let mut heap = BinaryHeap::new();
    for &(a, b) in edge_use.keys() {
        heap.push(candidate(a, b, &q, &pos, &version));
    }
    let mut live = tris.len();
    while live > target {
        let Some(c) = heap.pop() else { break };
        let (a, b) = (c.a as usize, c.b as usize);
        if dead[a] || dead[b] || version[a] != c.va || version[b] != c.vb {
            continue;
        }
        // Refuse collapses that would turn a neighbouring triangle over.
        let flips = |v: usize, other: u32, tris: &[[u32; 3]], alive: &[bool]| {
            vfaces[v].iter().any(|&f| {
                let f = f as usize;
                if !alive[f] || tris[f].contains(&other) {
                    return false;
                }
                let t = tris[f];
                let before = t.map(|i| pos[i as usize]);
                let after = t.map(|i| if i as usize == v { c.target } else { pos[i as usize] });
                let n0 = cross(sub(before[1], before[0]), sub(before[2], before[0]));
                let n1 = cross(sub(after[1], after[0]), sub(after[2], after[0]));
                dot(n0, n1) <= 0.0
            })
        };
        if flips(a, c.b, &tris, &alive) || flips(b, c.a, &tris, &alive) {
            continue;
        }
        // Collapse b into a.
        pos[a] = c.target;
        let qb = q[b];
        q[a].add(&qb);
        dead[b] = true;
        version[a] += 1;
        let bfaces = std::mem::take(&mut vfaces[b]);
        for f in bfaces {
            let fi = f as usize;
            if !alive[fi] {
                continue;
            }
            if tris[fi].contains(&c.a) {
                alive[fi] = false;
                live -= 1;
                continue;
            }
            for v in &mut tris[fi] {
                if *v == c.b {
                    *v = c.a;
                }
            }
            vfaces[a].push(f);
        }
        vfaces[a].retain(|f| alive[*f as usize]);
        // New candidates around a.
        let mut neighbours: Vec<u32> = vfaces[a].iter().flat_map(|f| tris[*f as usize]).filter(|v| *v != c.a).collect();
        neighbours.sort_unstable();
        neighbours.dedup();
        for nb in neighbours {
            heap.push(candidate(c.a, nb, &q, &pos, &version));
        }
    }
    // Compact.
    let mut remap = vec![u32::MAX; n];
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for (f, t) in tris.iter().enumerate() {
        if !alive[f] {
            continue;
        }
        let mut out = [0u32; 3];
        for (k, v) in t.iter().enumerate() {
            if remap[*v as usize] == u32::MAX {
                remap[*v as usize] = positions.len() as u32;
                positions.push(pos[*v as usize]);
            }
            out[k] = remap[*v as usize];
        }
        if out[0] != out[1] && out[1] != out[2] && out[0] != out[2] {
            triangles.push(out);
        }
    }
    p.positions = positions;
    p.triangles = triangles;
}
