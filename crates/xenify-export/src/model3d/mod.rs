//! 3D model import: STL, OBJ, glTF / GLB and 3MF files become a
//! [`Mesh`] for a solid layer.
//!
//! Every format is read into [`RawModel`] triangles; [`process`] then welds
//! duplicate vertices, fixes inside-out parts, thins out very dense meshes,
//! merges flat regions into single polygons (CAD models are mostly flat
//! faces, and fewer faces draw faster and cleaner) and scales the model to a
//! unit box in Xenify Vector's axes.

mod gltf;
mod obj;
mod qem;
mod stl;
mod threemf;
pub(crate) mod zip;

use xenify_doc::{Color, Mesh, MeshPart};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0} files aren't supported")]
    Unsupported(String),
    #[error("the file has no triangles")]
    Empty,
}

pub(crate) fn invalid(msg: impl Into<String>) -> ModelError {
    ModelError::Invalid(msg.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFormat {
    Stl,
    Obj,
    Gltf,
    Glb,
    ThreeMf,
}

/// File extensions that [`import_model`] reads.
pub const MODEL_EXTENSIONS: [&str; 5] = ["stl", "obj", "gltf", "glb", "3mf"];

/// CAD formats people often have, with what to do instead.
pub fn unsupported_hint(ext: &str) -> Option<&'static str> {
    match ext {
        "step" | "stp" | "iges" | "igs" | "sldprt" | "sldasm" | "f3d" | "ipt" | "catpart" | "x_t" | "prt" => Some(
            "CAD files have to be turned into a mesh first: export STL, 3MF or glTF from your CAD program, then import that.",
        ),
        "fbx" | "blend" | "dae" | "3ds" | "ply" | "usdz" | "usd" => {
            Some("Export this model as glTF (.glb), OBJ or STL from your 3D program, then import that.")
        }
        _ => None,
    }
}

impl ModelFormat {
    /// Recognises a model by its contents, then by its extension.
    pub fn detect(name: &str, bytes: &[u8]) -> Option<Self> {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if bytes.starts_with(b"glTF") {
            return Some(ModelFormat::Glb);
        }
        match ext.as_str() {
            "stl" => Some(ModelFormat::Stl),
            "obj" => Some(ModelFormat::Obj),
            "gltf" => Some(ModelFormat::Gltf),
            "glb" => Some(ModelFormat::Glb),
            "3mf" => Some(ModelFormat::ThreeMf),
            _ => None,
        }
    }
}

/// Which way is up in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Up {
    /// glTF, and OBJ from most 3D programs.
    Y,
    /// STL and 3MF (CAD and 3D printing).
    Z,
}

/// Triangles as read from a file, before clean-up.
#[derive(Clone, Debug, Default)]
pub struct RawPart {
    pub name: String,
    pub color: Option<Color>,
    pub positions: Vec<[f64; 3]>,
    /// Counter-clockwise seen from outside (right-handed).
    pub triangles: Vec<[u32; 3]>,
}

impl RawPart {
    pub(crate) fn new(name: impl Into<String>, color: Option<Color>) -> Self {
        Self { name: name.into(), color, ..Default::default() }
    }

    /// Adds a polygon as a fan of triangles.
    pub(crate) fn polygon(&mut self, idx: &[u32]) {
        for k in 1..idx.len().saturating_sub(1) {
            self.triangles.push([idx[0], idx[k], idx[k + 1]]);
        }
    }
}

#[derive(Debug)]
pub struct RawModel {
    pub parts: Vec<RawPart>,
    pub up: Up,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelOptions {
    /// Denser models are simplified to about this many triangles.
    pub max_triangles: usize,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self { max_triangles: 30_000 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedModel {
    pub mesh: Mesh,
    /// Triangles in the file, and after simplifying.
    pub triangles_in: usize,
    pub triangles_kept: usize,
    /// Faces drawn (flat regions merged).
    pub faces: usize,
    /// The model's width, height and depth in the file's units (usually mm).
    pub size: [f64; 3],
}

impl ImportedModel {
    /// A one-line summary for the status bar.
    pub fn summary(&self) -> String {
        let [w, h, d] = self.size.map(|v| format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_string());
        let parts = match self.mesh.parts.len() {
            1 => String::new(),
            n => format!("{n} parts, "),
        };
        let simplified = if self.triangles_kept < self.triangles_in {
            format!(" (simplified from {})", self.triangles_in)
        } else {
            String::new()
        };
        format!("{parts}{} triangles{simplified}, {w} × {h} × {d} units", self.triangles_kept)
    }
}

/// Reads a model file. `sibling` loads files it refers to (an OBJ's
/// materials, a glTF's buffers) by relative path.
pub fn import_model(
    name: &str,
    bytes: &[u8],
    options: ModelOptions,
    sibling: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<ImportedModel, ModelError> {
    let format = ModelFormat::detect(name, bytes).ok_or_else(|| ModelError::Unsupported(name.to_string()))?;
    let raw = match format {
        ModelFormat::Stl => stl::read(bytes)?,
        ModelFormat::Obj => obj::read(bytes, sibling)?,
        ModelFormat::Gltf | ModelFormat::Glb => gltf::read(bytes, sibling)?,
        ModelFormat::ThreeMf => threemf::read(bytes)?,
    };
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    process(stem, raw, options)
}

/// Cleans up and normalises a model (see the module docs).
pub fn process(name: &str, raw: RawModel, options: ModelOptions) -> Result<ImportedModel, ModelError> {
    let mut parts: Vec<RawPart> = raw.parts.into_iter().filter(|p| !p.triangles.is_empty()).collect();
    let (lo, hi) = bounds(parts.iter().flat_map(|p| &p.positions)).ok_or(ModelError::Empty)?;
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    if !extent.is_finite() || extent <= 0.0 {
        return Err(invalid("the model has no size"));
    }
    for p in &mut parts {
        weld(p, extent * 1e-6);
        orient(p);
    }
    let triangles_in: usize = parts.iter().map(|p| p.triangles.len()).sum();
    if triangles_in == 0 {
        return Err(ModelError::Empty);
    }
    if triangles_in > options.max_triangles.max(100) {
        simplify(&mut parts, lo, extent, options.max_triangles.max(100));
    }
    parts.retain(|p| !p.triangles.is_empty());
    let triangles_kept = parts.iter().map(|p| p.triangles.len()).sum();

    // Xenify Vector's axes: x right, y down, z away from the viewer.
    let axes = |p: [f64; 3]| match raw.up {
        Up::Z => [p[0], -p[2], p[1]],
        Up::Y => [p[0], -p[1], -p[2]],
    };
    let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let size = axes([hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]).map(f64::abs);
    let mut faces = 0;
    let mesh_parts = parts
        .iter()
        .map(|p| {
            let polys = merge_flat(p, extent);
            faces += polys.len();
            let shading: Vec<i8> = corner_normals(p, &polys)
                .map(|ns| ns.into_iter().flat_map(|n| MeshPart::pack_normal(axes(n))).collect())
                .unwrap_or_default();
            let positions: Vec<f32> = p
                .positions
                .iter()
                .flat_map(|v| {
                    let c = [(v[0] - center[0]) / extent, (v[1] - center[1]) / extent, (v[2] - center[2]) / extent];
                    axes(c).map(|x| x as f32)
                })
                .collect();
            MeshPart {
                name: p.name.clone(),
                color: p.color,
                positions: positions.into(),
                faces: MeshPart::encode_faces(polys.iter().map(|(l, _)| l.as_slice())).into(),
                shading: shading.into(),
            }
        })
        .collect::<Vec<_>>();
    let own_colors = mesh_parts.iter().any(|p| p.color.is_some());
    Ok(ImportedModel {
        mesh: Mesh { name: name.to_string(), parts: mesh_parts, own_colors },
        triangles_in,
        triangles_kept,
        faces,
        size,
    })
}

fn bounds<'a>(pts: impl Iterator<Item = &'a [f64; 3]>) -> Option<([f64; 3], [f64; 3])> {
    pts.filter(|p| p.iter().all(|v| v.is_finite())).fold(None, |acc, p| {
        Some(match acc {
            None => (*p, *p),
            Some((lo, hi)) => ([0, 1, 2].map(|k| lo[k].min(p[k])), [0, 1, 2].map(|k| hi[k].max(p[k]))),
        })
    })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The triangle's normal scaled by twice its area.
fn area_normal(p: &RawPart, t: [u32; 3]) -> [f64; 3] {
    let [a, b, c] = t.map(|i| p.positions[i as usize]);
    cross(sub(b, a), sub(c, a))
}

/// Joins vertices closer than `tol` (STL repeats every corner per triangle),
/// drops unused ones and triangles that collapse.
fn weld(p: &mut RawPart, tol: f64) {
    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut remap = vec![u32::MAX; p.positions.len()];
    let key = |v: [f64; 3]| v.map(|x| (x / tol).round() as i64);
    let mut triangles = Vec::with_capacity(p.triangles.len());
    for t in &p.triangles {
        let mut out = [0u32; 3];
        let mut ok = true;
        for (k, i) in t.iter().enumerate() {
            let Some(v) = p.positions.get(*i as usize).copied().filter(|v| v.iter().all(|x| x.is_finite())) else {
                ok = false;
                break;
            };
            if remap[*i as usize] == u32::MAX {
                remap[*i as usize] = *map.entry(key(v)).or_insert_with(|| {
                    positions.push(v);
                    positions.len() as u32 - 1
                });
            }
            out[k] = remap[*i as usize];
        }
        if ok && out[0] != out[1] && out[1] != out[2] && out[0] != out[2] {
            triangles.push(out);
        }
    }
    p.positions = positions;
    p.triangles = triangles;
}

/// Turns a part the right way out when its triangles are wound inside-out
/// (its volume comes out negative).
fn orient(p: &mut RawPart) {
    let Some((lo, hi)) = bounds(p.positions.iter()) else { return };
    let o = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
    let (mut vol, mut abs) = (0.0, 0.0);
    for t in &p.triangles {
        let [a, b, c] = t.map(|i| sub(p.positions[i as usize], o));
        let v = dot(a, cross(b, c));
        vol += v;
        abs += v.abs();
    }
    if vol < 0.0 && -vol > abs * 0.05 {
        for t in &mut p.triangles {
            t.swap(1, 2);
        }
    }
}

/// Thins dense parts out to about `budget` triangles in all, each part in
/// proportion to its share (quadric error edge collapses: detail goes where
/// the surface is flattest first).
fn simplify(parts: &mut [RawPart], _lo: [f64; 3], _extent: f64, budget: usize) {
    let total: usize = parts.iter().map(|p| p.triangles.len()).sum();
    for p in parts.iter_mut() {
        let share = (p.triangles.len() as f64 / total.max(1) as f64 * budget as f64).round() as usize;
        qem::simplify(p, share.max(4));
    }
}

/// A flat face: its outline, then any holes.
type Loops = Vec<Vec<u32>>;

/// Groups each connected flat region of triangles into one face (with
/// holes). Regions with odd topology stay as triangles. Returns each face
/// and its unit normal.
fn merge_flat(p: &RawPart, extent: f64) -> Vec<(Loops, [f64; 3])> {
    let n = p.triangles.len();
    let normals: Vec<Option<[f64; 3]>> = p
        .triangles
        .iter()
        .map(|t| {
            let a = area_normal(p, *t);
            let l = dot(a, a).sqrt();
            (l > extent * extent * 1e-14).then(|| a.map(|x| x / l))
        })
        .collect();
    let mut edges: HashMap<(u32, u32), u32> = HashMap::with_capacity(n * 3);
    for (i, t) in p.triangles.iter().enumerate() {
        for k in 0..3 {
            edges.insert((t[k], t[(k + 1) % 3]), i as u32);
        }
    }
    let mut done = vec![false; n];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    let mut members = Vec::new();
    for seed in 0..n {
        if done[seed] {
            continue;
        }
        done[seed] = true;
        let Some(sn) = normals[seed] else {
            continue; // A zero-area sliver: nothing to draw.
        };
        let plane = dot(sn, p.positions[p.triangles[seed][0] as usize]);
        members.clear();
        stack.push(seed);
        while let Some(i) = stack.pop() {
            members.push(i);
            let t = p.triangles[i];
            for k in 0..3 {
                let Some(&j) = edges.get(&(t[(k + 1) % 3], t[k])) else { continue };
                let j = j as usize;
                if done[j] {
                    continue;
                }
                let Some(nj) = normals[j] else { continue };
                let off = dot(sn, p.positions[p.triangles[j][0] as usize]) - plane;
                if dot(sn, nj) > 1.0 - 1e-6 && off.abs() < extent * 1e-5 {
                    done[j] = true;
                    stack.push(j);
                }
            }
        }
        match (members.len() > 1).then(|| boundary(p, &members, sn)).flatten() {
            Some(loops) => out.push((loops, sn)),
            None => out.extend(members.iter().filter_map(|i| Some((vec![p.triangles[*i].to_vec()], normals[*i]?)))),
        }
    }
    out
}

/// A flat region's outline and holes, without points in the middle of
/// straight edges; `None` if its edges pinch or it has two outlines.
fn boundary(p: &RawPart, members: &[usize], normal: [f64; 3]) -> Option<Loops> {
    let inside: std::collections::HashSet<(u32, u32)> = members
        .iter()
        .flat_map(|i| {
            let t = p.triangles[*i];
            [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]
        })
        .collect();
    let mut next: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &inside {
        if !inside.contains(&(b, a)) && next.insert(a, b).is_some() {
            return None;
        }
    }
    let pos = |i: u32| p.positions[i as usize];
    let mut loops: Vec<(f64, Vec<u32>)> = Vec::new();
    let mut left = next.clone();
    while let Some(&start) = left.keys().next() {
        let mut l = vec![start];
        let mut at = left.remove(&start)?;
        while at != start {
            l.push(at);
            at = left.remove(&at)?;
        }
        // Drop points in the middle of straight edges.
        let len = l.len();
        let keep: Vec<u32> = (0..len)
            .filter(|&k| {
                let (a, b, c) = (pos(l[(k + len - 1) % len]), pos(l[k]), pos(l[(k + 1) % len]));
                let (u, v) = (sub(b, a), sub(c, b));
                let x = cross(u, v);
                dot(x, x) > 1e-12 * dot(u, u) * dot(v, v) || dot(u, v) < 0.0
            })
            .map(|k| l[k])
            .collect();
        if keep.len() < 3 {
            return None;
        }
        // Signed area about the normal: the outline is positive, holes negative.
        let area =
            (0..keep.len()).map(|k| dot(normal, cross(pos(keep[k]), pos(keep[(k + 1) % keep.len()])))).sum::<f64>();
        loops.push((area, keep));
    }
    let outlines = loops.iter().filter(|(a, _)| *a > 0.0).count();
    if outlines != 1 {
        return None;
    }
    loops.sort_by(|a, b| b.0.total_cmp(&a.0));
    Some(loops.into_iter().map(|(_, l)| l).collect())
}

/// Smooth shading: for each face with 3 or 4 corners and no holes, the
/// normal at each corner is the average of the faces meeting there that are
/// within ~35° of it (weighted by area), so curved surfaces look smooth and
/// sharp edges stay sharp. Returns the corner normals in order, or `None`
/// when every face would be shaded flat anyway.
fn corner_normals(p: &RawPart, faces: &[(Loops, [f64; 3])]) -> Option<Vec<[f64; 3]>> {
    const COS: f64 = 0.82;
    let pos = |i: u32| p.positions[i as usize];
    let weights: Vec<f64> = faces
        .iter()
        .map(|(loops, n)| {
            let l = &loops[0];
            (0..l.len()).map(|k| dot(*n, cross(pos(l[k]), pos(l[(k + 1) % l.len()])))).sum::<f64>().abs() / 2.0
        })
        .collect();
    let mut at_vertex: Vec<Vec<u32>> = vec![Vec::new(); p.positions.len()];
    for (f, (loops, _)) in faces.iter().enumerate() {
        for i in loops.iter().flatten() {
            at_vertex[*i as usize].push(f as u32);
        }
    }
    let mut changed = false;
    let mut out = Vec::new();
    for (loops, n) in faces {
        if loops.len() != 1 || !matches!(loops[0].len(), 3 | 4) {
            continue;
        }
        for i in &loops[0] {
            let mut sum = [0.0; 3];
            for &g in &at_vertex[*i as usize] {
                let m = faces[g as usize].1;
                if dot(*n, m) > COS {
                    let w = weights[g as usize];
                    sum = [sum[0] + m[0] * w, sum[1] + m[1] * w, sum[2] + m[2] * w];
                }
            }
            let l = dot(sum, sum).sqrt();
            let c = if l > 0.0 { sum.map(|x| x / l) } else { *n };
            changed |= dot(c, *n) < 0.9998;
            out.push(c);
        }
    }
    changed.then_some(out)
}

/// Linear light (glTF) to sRGB.
pub(crate) fn linear_to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2×2×2 cube made of 12 triangles, each corner repeated (like STL).
    pub fn cube_soup(z_up_size: [f64; 3]) -> RawPart {
        let c = |i: usize| [0, 1, 2].map(|k| if (i >> k) & 1 == 1 { z_up_size[k] } else { 0.0 });
        // Quads counter-clockwise seen from outside (right-handed).
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut p = RawPart::new("cube", None);
        for q in quads {
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                let base = p.positions.len() as u32;
                p.positions.extend(t.map(c));
                p.triangles.push([base, base + 1, base + 2]);
            }
        }
        p
    }

    #[test]
    fn a_triangle_soup_cube_becomes_six_square_faces() {
        let raw = RawModel { parts: vec![cube_soup([2.0, 4.0, 6.0])], up: Up::Z };
        let m = process("cube.stl", raw, ModelOptions::default()).unwrap();
        assert_eq!(m.triangles_in, 12);
        assert_eq!(m.faces, 6, "each side's two triangles merge");
        let part = &m.mesh.parts[0];
        assert_eq!(part.vertex_count(), 8, "corners are welded");
        assert!(part.polygons().all(|p| p.len() == 4));
        // Z-up height 6 becomes Xenify Vector's y (the tallest side, scaled to 1).
        let (lo, hi) = m.mesh.bounds().unwrap();
        assert!((hi[1] - lo[1] - 1.0).abs() < 1e-6 && (hi[0] - lo[0] - 1.0 / 3.0).abs() < 1e-6);
        assert!((lo[1] + 0.5).abs() < 1e-6, "centred");
        assert_eq!(m.size, [2.0, 6.0, 4.0]);
        // Outward normals: every face's centre is away from the middle along its normal.
        for poly in part.polygons() {
            let pts: Vec<[f64; 3]> = poly.iter().map(|i| part.vertex(*i)).collect();
            let n = cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]));
            let c = pts.iter().fold([0.0; 3], |s, v| [s[0] + v[0], s[1] + v[1], s[2] + v[2]]);
            assert!(dot(n, c) > 0.0, "{poly:?} faces outwards");
        }
        assert!(m.summary().contains("12 triangles"), "{}", m.summary());
    }

    #[test]
    fn a_plate_with_a_hole_is_one_face_per_side_and_curves_shade_smoothly() {
        // A square ring: outer 0..3, inner 1..2, as 8 triangles on top (z = 1, normal +z).
        let mut p = RawPart::new("plate", None);
        let ring = |v: f64| [[-v, -v], [v, -v], [v, v], [-v, v]];
        for [x, y] in ring(3.0).into_iter().chain(ring(1.0)) {
            p.positions.push([x, y, 1.0]);
        }
        for k in 0..4u32 {
            let (o0, o1, i0, i1) = (k, (k + 1) % 4, 4 + k, 4 + (k + 1) % 4);
            p.triangles.push([o0, o1, i1]);
            p.triangles.push([o0, i1, i0]);
        }
        let faces = merge_flat(&p, 6.0);
        assert_eq!(faces.len(), 1, "one face with a hole");
        let (loops, n) = &faces[0];
        assert_eq!(loops.len(), 2);
        assert_eq!(loops[0].len(), 4);
        assert!(loops[0].iter().all(|i| *i < 4), "the outline comes first");
        assert_eq!(*n, [0.0, 0.0, 1.0]);
        assert!(corner_normals(&p, &faces).is_none(), "flat faces are shaded flat");

        // A cylinder: its sides are smoothed into each other, but not into the cap.
        let mut cyl = RawPart::new("cyl", None);
        let segs = 36u32;
        for s in 0..segs {
            let a = std::f64::consts::TAU * s as f64 / segs as f64;
            cyl.positions.push([a.cos(), a.sin(), 0.0]);
            cyl.positions.push([a.cos(), a.sin(), 1.0]);
        }
        for s in 0..segs {
            let (b0, t0, b1, t1) = (2 * s, 2 * s + 1, 2 * ((s + 1) % segs), 2 * ((s + 1) % segs) + 1);
            cyl.triangles.push([b0, b1, t1]);
            cyl.triangles.push([b0, t1, t0]);
        }
        let top: Vec<u32> = (0..segs).map(|s| 2 * s + 1).collect();
        cyl.polygon(&top);
        let faces = merge_flat(&cyl, 2.0);
        // Each side quad's two triangles are coplanar and merge; so does the cap.
        assert_eq!(faces.len(), segs as usize + 1);
        let corners = corner_normals(&cyl, &faces).expect("sides are curved");
        assert_eq!(corners.len(), segs as usize * 4, "four corners per side quad; the cap has more");
        for (k, c) in corners.iter().enumerate() {
            assert!(c[2].abs() < 1e-9, "corner {k} isn't pulled towards the cap: {c:?}");
            let quad = &faces.iter().filter(|(l, _)| l[0].len() == 4).nth(k / 4).unwrap().0[0];
            let v = cyl.positions[quad[k % 4] as usize];
            assert!(dot(*c, [v[0], v[1], 0.0]) > 0.999, "corner {k} points straight out from the axis");
        }
    }

    #[test]
    fn inside_out_parts_are_flipped_and_dense_ones_simplified() {
        let mut inverted = cube_soup([1.0, 1.0, 1.0]);
        for t in &mut inverted.triangles {
            t.swap(1, 2);
        }
        let m = process("x", RawModel { parts: vec![inverted], up: Up::Y }, ModelOptions::default()).unwrap();
        let part = &m.mesh.parts[0];
        for poly in part.polygons() {
            let pts: Vec<[f64; 3]> = poly.iter().map(|i| part.vertex(*i)).collect();
            let n = cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]));
            let c = pts.iter().fold([0.0; 3], |s, v| [s[0] + v[0], s[1] + v[1], s[2] + v[2]]);
            assert!(dot(n, c) > 0.0);
        }
        // A finely divided sphere is thinned out to the budget.
        let mut sphere = RawPart::new("ball", Some(Color::rgb(1.0, 0.0, 0.0)));
        let (rings, segs) = (120u32, 240u32);
        for r in 0..=rings {
            let th = std::f64::consts::PI * r as f64 / rings as f64;
            for s in 0..segs {
                let ph = std::f64::consts::TAU * s as f64 / segs as f64;
                sphere.positions.push([th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()]);
            }
        }
        for r in 0..rings {
            for s in 0..segs {
                let (a, b) = (r * segs + s, r * segs + (s + 1) % segs);
                let (c, d) = (a + segs, b + segs);
                sphere.triangles.push([a, c, d]);
                sphere.triangles.push([a, d, b]);
            }
        }
        let options = ModelOptions { max_triangles: 5_000 };
        let m = process("ball.stl", RawModel { parts: vec![sphere], up: Up::Z }, options).unwrap();
        assert!(m.triangles_in > 50_000, "{}", m.triangles_in);
        assert!(m.triangles_kept <= 5_000 && m.triangles_kept > 1_000, "{}", m.triangles_kept);
        assert!(m.mesh.own_colors);
        assert!(m.summary().contains("simplified"));
    }

    #[test]
    fn cad_formats_get_advice() {
        assert!(unsupported_hint("step").unwrap().contains("STL"));
        assert_eq!(ModelFormat::detect("a.GLB", b"xx"), Some(ModelFormat::Glb));
        assert_eq!(ModelFormat::detect("a.bin", b"glTF\x02\0\0\0"), Some(ModelFormat::Glb));
        assert_eq!(ModelFormat::detect("a.png", b"\x89PNG"), None);
    }
}
