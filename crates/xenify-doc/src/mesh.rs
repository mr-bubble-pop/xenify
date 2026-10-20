//! 3D models imported from STL, OBJ, glTF or 3MF files, embedded in a solid
//! layer.
//!
//! Positions are in Xenify Vector's 3D space (x right, y down, z away from the
//! viewer), centred on the origin and scaled so the model's largest
//! dimension is 1; the layer's size property scales it to pixels.

use crate::Color;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    /// The file it came from (for display).
    pub name: String,
    pub parts: Vec<MeshPart>,
    /// Draw parts in their own colours from the file (otherwise every part
    /// uses the layer's colour).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own_colors: bool,
}

/// One named piece of a model (an assembly's part, a glTF mesh, an OBJ group).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshPart {
    pub name: String,
    /// The part's colour in the file, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// x, y, z per vertex (little-endian f32s, base64 in the file).
    #[serde(with = "f32s")]
    pub positions: Arc<[f32]>,
    /// Flat faces. Each loop is written as its vertex count followed by
    /// that many vertex indices, counter-clockwise seen from outside; a
    /// count with [`HOLE`] set is a hole in the face before it
    /// (little-endian u32s, base64 in the file).
    #[serde(with = "u32s")]
    pub faces: Arc<[u32]>,
    /// Smooth shading for curved surfaces: for each face with 3 or 4
    /// corners and no holes (in order; other faces are flat), the normal at
    /// each corner, as x, y, z bytes scaled by 127. Empty: every face is
    /// shaded flat.
    #[serde(default, with = "i8s", skip_serializing_if = "no_normals")]
    pub shading: Arc<[i8]>,
}

/// Marks a loop in [`MeshPart::faces`] as a hole.
pub const HOLE: u32 = 1 << 31;

fn no_normals(v: &Arc<[i8]>) -> bool {
    v.is_empty()
}

impl PartialEq for MeshPart {
    fn eq(&self, o: &Self) -> bool {
        let same = |a: bool, eq: &dyn Fn() -> bool| a || eq();
        self.name == o.name
            && self.color == o.color
            && same(Arc::ptr_eq(&self.positions, &o.positions), &|| self.positions == o.positions)
            && same(Arc::ptr_eq(&self.faces, &o.faces), &|| self.faces == o.faces)
            && same(Arc::ptr_eq(&self.shading, &o.shading), &|| self.shading == o.shading)
    }
}

impl MeshPart {
    pub fn vertex(&self, i: u32) -> [f64; 3] {
        let k = i as usize * 3;
        [self.positions[k] as f64, self.positions[k + 1] as f64, self.positions[k + 2] as f64]
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// The faces, in order.
    pub fn faces(&self) -> Faces<'_> {
        Faces { faces: &self.faces, vertices: self.vertex_count() }
    }

    /// Each face's outline (holes left out).
    pub fn polygons(&self) -> impl Iterator<Item = &[u32]> {
        self.faces().map(|f| f.outline)
    }

    /// Whether a face gets smooth shading normals (see [`MeshPart::shading`]).
    pub fn smoothable(face: &MeshFace) -> bool {
        matches!(face.outline.len(), 3 | 4) && face.holes.is_empty()
    }

    /// The `k`th stored corner normal, if the part has them.
    pub fn corner_normal(&self, k: usize) -> Option<[f64; 3]> {
        let n = self.shading.get(k * 3..k * 3 + 3)?;
        Some([n[0] as f64 / 127.0, n[1] as f64 / 127.0, n[2] as f64 / 127.0])
    }

    /// Packs a unit normal for [`MeshPart::shading`].
    pub fn pack_normal(n: [f64; 3]) -> [i8; 3] {
        n.map(|v| (v.clamp(-1.0, 1.0) * 127.0).round() as i8)
    }

    /// Encodes faces (each an outline followed by its holes) in the
    /// [`MeshPart::faces`] layout.
    pub fn encode_faces<'a>(faces: impl IntoIterator<Item = &'a [Vec<u32>]>) -> Vec<u32> {
        let mut out = Vec::new();
        for f in faces {
            for (k, l) in f.iter().enumerate() {
                out.push(l.len() as u32 | if k > 0 { HOLE } else { 0 });
                out.extend_from_slice(l);
            }
        }
        out
    }

    /// Encodes plain polygons (no holes).
    pub fn encode_polygons<'a>(polys: impl IntoIterator<Item = &'a [u32]>) -> Vec<u32> {
        let mut out = Vec::new();
        for p in polys {
            out.push(p.len() as u32);
            out.extend_from_slice(p);
        }
        out
    }

    /// The part's bounding box (min, max), `None` when it has no vertices.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut it = self.positions.as_chunks::<3>().0.iter();
        let first = it.next()?;
        let mut lo = [first[0] as f64, first[1] as f64, first[2] as f64];
        let mut hi = lo;
        for p in it {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k] as f64);
                hi[k] = hi[k].max(p[k] as f64);
            }
        }
        Some((lo, hi))
    }
}

/// One face: an outline and any holes in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshFace<'a> {
    pub outline: &'a [u32],
    /// The encoded holes (see [`MeshFace::holes`]).
    holes: &'a [u32],
}

impl<'a> MeshFace<'a> {
    pub fn holes(&self) -> impl Iterator<Item = &'a [u32]> {
        let mut rest = self.holes;
        std::iter::from_fn(move || {
            let (&n, tail) = rest.split_first()?;
            let (h, tail) = tail.split_at((n & !HOLE) as usize);
            rest = tail;
            Some(h)
        })
    }
}

/// Iterates a part's faces, skipping malformed ones (a hand-edited or
/// truncated file must not crash the renderer).
pub struct Faces<'a> {
    faces: &'a [u32],
    vertices: usize,
}

impl<'a> Faces<'a> {
    /// The next loop: (is a hole, indices), or `None` at the end or on damage.
    fn next_loop(&mut self) -> Option<(bool, &'a [u32])> {
        let (&n, rest) = self.faces.split_first()?;
        let len = (n & !HOLE) as usize;
        if len > rest.len() {
            self.faces = &[];
            return None;
        }
        let (l, rest) = rest.split_at(len);
        self.faces = rest;
        Some((n & HOLE != 0, l))
    }

    fn valid(&self, l: &[u32]) -> bool {
        l.len() >= 3 && l.iter().all(|i| (*i as usize) < self.vertices)
    }
}

impl<'a> Iterator for Faces<'a> {
    type Item = MeshFace<'a>;
    fn next(&mut self) -> Option<MeshFace<'a>> {
        loop {
            let (hole, outline) = self.next_loop()?;
            // Its holes follow it; bad ones are dropped with the face.
            let start = self.faces;
            let mut ok = !hole && self.valid(outline);
            while self.faces.first().is_some_and(|n| n & HOLE != 0) {
                let (_, h) = self.next_loop()?;
                ok &= self.valid(h);
            }
            if ok {
                let used = start.len() - self.faces.len();
                return Some(MeshFace { outline, holes: &start[..used] });
            }
        }
    }
}

impl Mesh {
    pub fn polygon_count(&self) -> usize {
        self.parts.iter().map(|p| p.faces().count()).sum()
    }

    pub fn has_colors(&self) -> bool {
        self.parts.iter().any(|p| p.color.is_some())
    }

    /// The box around every part.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        self.parts.iter().filter_map(MeshPart::bounds).reduce(|a, b| {
            (
                [a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), a.0[2].min(b.0[2])],
                [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), a.1[2].max(b.1[2])],
            )
        })
    }
}

fn decode<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 4]>, D::Error> {
    let bytes =
        base64::engine::general_purpose::STANDARD.decode(String::deserialize(d)?).map_err(serde::de::Error::custom)?;
    if bytes.len() % 4 != 0 {
        return Err(serde::de::Error::custom("mesh data has a stray byte"));
    }
    Ok(bytes.as_chunks::<4>().0.to_vec())
}

mod f32s {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Arc<[f32]>, s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<[f32]>, D::Error> {
        Ok(decode(d)?.into_iter().map(f32::from_le_bytes).collect())
    }
}

mod i8s {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Arc<[i8]>, s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = v.iter().map(|b| *b as u8).collect();
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<[i8]>, D::Error> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(String::deserialize(d)?)
            .map_err(serde::de::Error::custom)?;
        Ok(bytes.into_iter().map(|b| b as i8).collect())
    }
}

mod u32s {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Arc<[u32]>, s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<[u32]>, D::Error> {
        Ok(decode(d)?.into_iter().map(u32::from_le_bytes).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_skips_bad_polygons() {
        let faces = MeshPart::encode_polygons([&[0u32, 1, 2][..], &[0, 1][..], &[0, 1, 9][..], &[2, 1, 0][..]]);
        let part = MeshPart {
            name: "tri".into(),
            color: Some(Color::rgb(1.0, 0.0, 0.0)),
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.5, -2.0].into(),
            faces: faces.into(),
            shading: Arc::default(),
        };
        let m = Mesh { name: "m".into(), parts: vec![part], own_colors: true };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Mesh>(&json).unwrap(), m);
        // The two-vertex and out-of-range polygons are skipped.
        assert_eq!(m.parts[0].polygons().collect::<Vec<_>>(), vec![&[0, 1, 2][..], &[2, 1, 0][..]]);
        assert_eq!(m.polygon_count(), 2);
        assert_eq!(m.bounds(), Some(([0.0, 0.0, -2.0], [1.0, 1.5, 0.0])));
        assert_eq!(m.parts[0].vertex(2), [0.0, 1.5, -2.0]);
        // A truncated list ends cleanly.
        let cut = MeshPart { faces: vec![3, 0, 1].into(), ..m.parts[0].clone() };
        assert_eq!(cut.polygons().count(), 0);
        assert!(!json.contains("shading"));
    }

    #[test]
    fn faces_keep_their_holes() {
        let square = vec![0, 1, 2, 3];
        let hole = vec![7, 6, 5, 4];
        let faces = MeshPart::encode_faces([&[square.clone(), hole.clone()][..], &[vec![0, 1, 2]][..]]);
        let part = MeshPart {
            name: String::new(),
            color: None,
            positions: vec![0.0; 24].into(),
            faces: faces.into(),
            shading: [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]
                .into_iter()
                .flat_map(MeshPart::pack_normal)
                .collect(),
        };
        let f: Vec<MeshFace> = part.faces().collect();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].outline, &square[..]);
        assert_eq!(f[0].holes().collect::<Vec<_>>(), vec![&hole[..]]);
        assert_eq!(f[1].holes().count(), 0);
        assert_eq!(part.corner_normal(1), Some([0.0, 1.0, 0.0]));
        assert_eq!(part.corner_normal(3), None);
        assert!(!MeshPart::smoothable(&f[0]) && MeshPart::smoothable(&f[1]));
        let back: MeshPart = serde_json::from_str(&serde_json::to_string(&part).unwrap()).unwrap();
        assert_eq!(back, part);
    }
}
