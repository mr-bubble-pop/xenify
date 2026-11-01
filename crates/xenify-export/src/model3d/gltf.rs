//! glTF 2.0: `.gltf` (JSON, with embedded or neighbouring buffers) and
//! `.glb` (binary). Y is up.

use super::{ModelError, RawModel, RawPart, Up, invalid, linear_to_srgb};
use base64::Engine;
use serde_json::Value;
use xenify_doc::Color;

/// Column-major 4×4.
type Mat4 = [f64; 16];

const IDENTITY: Mat4 = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    o
}

fn transform(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

fn numbers(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default()
}

/// A node's local transform (a matrix, or translation · rotation · scale).
fn local(node: &Value) -> Mat4 {
    if let Ok(m) = <[f64; 16]>::try_from(numbers(&node["matrix"])) {
        return m;
    }
    let t = <[f64; 3]>::try_from(numbers(&node["translation"])).unwrap_or([0.0; 3]);
    let [x, y, z, w] = <[f64; 4]>::try_from(numbers(&node["rotation"])).unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let s = <[f64; 3]>::try_from(numbers(&node["scale"])).unwrap_or([1.0; 3]);
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    [
        r[0] * s[0],
        r[1] * s[0],
        r[2] * s[0],
        0.0,
        r[3] * s[1],
        r[4] * s[1],
        r[5] * s[1],
        0.0,
        r[6] * s[2],
        r[7] * s[2],
        r[8] * s[2],
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

struct Gltf {
    json: Value,
    buffers: Vec<Vec<u8>>,
}

pub fn read(bytes: &[u8], sibling: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<RawModel, ModelError> {
    let (json, bin) = if bytes.starts_with(b"glTF") { split_glb(bytes)? } else { (bytes.to_vec(), None) };
    let json: Value =
        serde_json::from_slice(&json).map_err(|e| invalid(format!("the glTF file isn't valid JSON: {e}")))?;
    for ext in numbers_or_strings(&json["extensionsRequired"]) {
        if ext.contains("draco") || ext.contains("meshopt") {
            return Err(invalid(
                "this glTF file is compressed (Draco / meshopt); export it again without mesh compression",
            ));
        }
    }
    let mut buffers = Vec::new();
    for (i, b) in json["buffers"].as_array().into_iter().flatten().enumerate() {
        let data = match b["uri"].as_str() {
            None if i == 0 => bin.clone().ok_or_else(|| invalid("the glTF file's binary data is missing"))?,
            None => Vec::new(),
            Some(uri) if uri.starts_with("data:") => {
                let b64 = uri.split_once(',').map(|x| x.1).unwrap_or("");
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|_| invalid("the glTF file's embedded data is damaged"))?
            }
            Some(uri) => {
                let file = percent_decode(uri);
                sibling(&file).ok_or_else(|| invalid(format!("the glTF file needs {file} next to it")))?
            }
        };
        buffers.push(data);
    }
    let g = Gltf { json, buffers };
    let mut parts = Vec::new();
    let scene = g.json["scene"].as_u64().unwrap_or(0) as usize;
    let roots: Vec<usize> = match g.json["scenes"].get(scene) {
        Some(s) => s["nodes"].as_array().into_iter().flatten().filter_map(|v| v.as_u64()).map(|v| v as usize).collect(),
        // No scene: every node that isn't someone's child.
        None => {
            let nodes = g.json["nodes"].as_array().map_or(0, Vec::len);
            let children: Vec<usize> = g.json["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|n| n["children"].as_array().into_iter().flatten().filter_map(Value::as_u64))
                .map(|v| v as usize)
                .collect();
            (0..nodes).filter(|i| !children.contains(i)).collect()
        }
    };
    for r in roots {
        g.node(r, &IDENTITY, &mut parts, 0)?;
    }
    if parts.is_empty()
        && let Some(meshes) = g.json["meshes"].as_array()
    {
        // Meshes that no node places.
        for m in 0..meshes.len() {
            g.mesh(m, &IDENTITY, "", &mut parts)?;
        }
    }
    Ok(RawModel { parts, up: Up::Y })
}

fn numbers_or_strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn split_glb(bytes: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), ModelError> {
    let bad = || invalid("this isn't a readable GLB file");
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let (mut json, mut bin) = (None, None);
    let mut at = 12;
    while let (Some(len), Some(kind)) = (u32_at(at), u32_at(at + 4)) {
        let data = bytes.get(at + 8..at + 8 + len).ok_or_else(bad)?;
        match kind {
            0x4E4F_534A => json = Some(data.to_vec()),
            0x004E_4942 => bin = Some(data.to_vec()),
            _ => {}
        }
        at += 8 + len;
    }
    Ok((json.ok_or_else(bad)?, bin))
}

impl Gltf {
    fn node(&self, i: usize, parent: &Mat4, out: &mut Vec<RawPart>, depth: usize) -> Result<(), ModelError> {
        let Some(node) = self.json["nodes"].get(i) else { return Ok(()) };
        if depth > 64 {
            return Ok(());
        }
        let m = mul(parent, &local(node));
        if let Some(mesh) = node["mesh"].as_u64() {
            self.mesh(mesh as usize, &m, node["name"].as_str().unwrap_or(""), out)?;
        }
        for c in node["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            self.node(c as usize, &m, out, depth + 1)?;
        }
        Ok(())
    }

    fn mesh(&self, i: usize, m: &Mat4, node_name: &str, out: &mut Vec<RawPart>) -> Result<(), ModelError> {
        let Some(mesh) = self.json["meshes"].get(i) else { return Ok(()) };
        let name = if node_name.is_empty() { mesh["name"].as_str().unwrap_or("") } else { node_name };
        let det = m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2])
            + m[8] * (m[1] * m[6] - m[5] * m[2]);
        for prim in mesh["primitives"].as_array().into_iter().flatten() {
            let mode = prim["mode"].as_u64().unwrap_or(4);
            if !(4..=6).contains(&mode) {
                continue; // Points and lines.
            }
            let Some(pos) = prim["attributes"]["POSITION"].as_u64() else { continue };
            let positions = self.floats(pos as usize, 3)?;
            let color = prim["material"].as_u64().and_then(|mi| {
                let c = numbers(&self.json["materials"][mi as usize]["pbrMetallicRoughness"]["baseColorFactor"]);
                let [r, g, b, a] = <[f64; 4]>::try_from(c).ok()?;
                Some(Color::rgba(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b), a))
            });
            let mut part = RawPart::new(name, color);
            part.positions = positions.as_chunks::<3>().0.iter().map(|p| transform(m, *p)).collect();
            let n = part.positions.len() as u32;
            let idx: Vec<u32> = match prim["indices"].as_u64() {
                Some(a) => self.ints(a as usize)?,
                None => (0..n).collect(),
            };
            match mode {
                4 => part.triangles.extend(idx.as_chunks::<3>().0.iter().copied()),
                5 => {
                    for k in 0..idx.len().saturating_sub(2) {
                        let t = [idx[k], idx[k + 1], idx[k + 2]];
                        part.triangles.push(if k % 2 == 0 { t } else { [t[1], t[0], t[2]] });
                    }
                }
                _ => part.polygon(&idx),
            }
            part.triangles.retain(|t| t.iter().all(|i| *i < n));
            if det < 0.0 {
                part.triangles.iter_mut().for_each(|t| t.swap(1, 2));
            }
            out.push(part);
        }
        Ok(())
    }

    /// An accessor's bytes, with its element size and stride.
    fn view(&self, a: &Value, elem: usize) -> Result<(&[u8], usize, usize), ModelError> {
        let bad = || invalid("the glTF file's data is damaged");
        let count = a["count"].as_u64().ok_or_else(bad)? as usize;
        let Some(v) = a["bufferView"].as_u64() else { return Ok((&[], 0, 0)) };
        let view = &self.json["bufferViews"][v as usize];
        let buf = self.buffers.get(view["buffer"].as_u64().ok_or_else(bad)? as usize).ok_or_else(bad)?;
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = view["byteStride"].as_u64().map_or(elem, |s| s as usize).max(elem);
        let end = if count == 0 { start } else { start + stride * (count - 1) + elem };
        Ok((buf.get(start..end).ok_or_else(bad)?, count, stride))
    }

    fn floats(&self, accessor: usize, width: usize) -> Result<Vec<f64>, ModelError> {
        let a = &self.json["accessors"][accessor];
        let ctype = a["componentType"].as_u64().unwrap_or(0);
        let size = match ctype {
            5126 => 4,
            _ => return Err(invalid("this glTF file stores positions in a packed form that isn't supported")),
        };
        let (data, count, stride) = self.view(a, size * width)?;
        let mut out = Vec::with_capacity(count * width);
        for e in 0..count {
            for c in 0..width {
                let o = e * stride + c * size;
                out.push(f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as f64);
            }
        }
        Ok(out)
    }

    fn ints(&self, accessor: usize) -> Result<Vec<u32>, ModelError> {
        let a = &self.json["accessors"][accessor];
        let size = match a["componentType"].as_u64() {
            Some(5121) => 1,
            Some(5123) => 2,
            Some(5125) => 4,
            _ => return Err(invalid("the glTF file's triangle indices are damaged")),
        };
        let (data, count, stride) = self.view(a, size)?;
        Ok((0..count)
            .map(|e| {
                let o = e * stride;
                match size {
                    1 => data[o] as u32,
                    2 => u16::from_le_bytes([data[o], data[o + 1]]) as u32,
                    _ => u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model3d::{ModelOptions, import_model};

    /// One triangle, indexed, under a node moved 10 along x and scaled ×2.
    fn triangle_bin() -> Vec<u8> {
        let mut bin: Vec<u8> =
            [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0].iter().flat_map(|f| f.to_le_bytes()).collect();
        bin.extend([0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
        bin.extend([0, 0]); // pad to 4
        bin
    }

    fn triangle_json(uri: Option<String>) -> String {
        let buffer = match uri {
            Some(u) => format!(r#"{{"byteLength": 44, "uri": "{u}"}}"#),
            None => r#"{"byteLength": 44}"#.to_string(),
        };
        format!(
            r#"{{"asset": {{"version": "2.0"}}, "scene": 0, "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"name": "Base", "children": [1], "translation": [10, 0, 0]}}, {{"mesh": 0, "scale": [2, 2, 2]}}],
            "meshes": [{{"name": "tri", "primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}]}}],
            "materials": [{{"pbrMetallicRoughness": {{"baseColorFactor": [1, 0.2140, 0, 1]}}}}],
            "buffers": [{buffer}],
            "bufferViews": [{{"buffer": 0, "byteLength": 36}}, {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}],
            "accessors": [{{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                          {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}]}}"#
        )
    }

    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut j = json.as_bytes().to_vec();
        while !j.len().is_multiple_of(4) {
            j.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + j.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((j.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(&j);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(bin);
        out
    }

    #[test]
    fn reads_gltf_glb_and_neighbouring_buffers() {
        let bin = triangle_bin();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);
        let files = [
            ("a.glb", glb(&triangle_json(None), &bin)),
            ("a.gltf", triangle_json(Some(format!("data:application/octet-stream;base64,{b64}"))).into_bytes()),
            ("b.gltf", triangle_json(Some("my%20data.bin".into())).into_bytes()),
        ];
        for (name, bytes) in files {
            let raw = read(&bytes, &|f| (f == "my data.bin").then(|| bin.clone())).unwrap();
            assert_eq!(raw.parts.len(), 1, "{name}");
            let p = &raw.parts[0];
            assert_eq!(p.name, "tri", "the mesh node has no name of its own, so the mesh's");
            assert_eq!(p.positions, vec![[10.0, 0.0, 0.0], [12.0, 0.0, 0.0], [10.0, 2.0, 0.0]], "{name}");
            assert_eq!(p.triangles, vec![[0, 1, 2]]);
            let c = p.color.unwrap();
            assert!((c.g - 0.5).abs() < 0.01, "linear 0.214 is sRGB ½: {c:?}");
        }
        assert!(
            read(triangle_json(Some("gone.bin".into())).as_bytes(), &|_| None)
                .unwrap_err()
                .to_string()
                .contains("gone.bin")
        );
        let m = import_model("a.glb", &glb(&triangle_json(None), &bin), ModelOptions::default(), &|_| None).unwrap();
        assert_eq!(m.triangles_kept, 1);
    }
}
