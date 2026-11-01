//! Wavefront OBJ, with colours from its MTL materials. Y is up.

use super::{ModelError, RawModel, RawPart, Up, invalid};
use std::collections::HashMap;
use xenify_doc::Color;

pub fn read(bytes: &[u8], sibling: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<RawModel, ModelError> {
    let text = String::from_utf8_lossy(bytes);
    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut materials: HashMap<String, Color> = HashMap::new();
    // One part per (object or group, material).
    let mut parts: Vec<(String, Option<String>, Vec<Vec<u32>>)> = Vec::new();
    let mut index: HashMap<(String, Option<String>), usize> = HashMap::new();
    let (mut group, mut material) = (String::new(), None::<String>);
    for (line_no, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("");
        let mut w = line.split_whitespace();
        let bad = || invalid(format!("OBJ line {} isn't valid", line_no + 1));
        match w.next() {
            Some("v") => {
                let mut n = || w.next().and_then(|s| s.parse::<f64>().ok()).ok_or_else(bad);
                positions.push([n()?, n()?, n()?]);
            }
            Some("o" | "g") => group = w.collect::<Vec<_>>().join(" "),
            Some("usemtl") => material = w.next().map(str::to_string),
            Some("mtllib") => {
                for file in w {
                    if let Some(mtl) = sibling(file) {
                        materials.extend(read_mtl(&String::from_utf8_lossy(&mtl)));
                    }
                }
            }
            Some("f") => {
                let mut face = Vec::new();
                for v in w {
                    let i: i64 = v.split('/').next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
                    // 1-based; negative counts back from the latest vertex.
                    let i = if i < 0 { positions.len() as i64 + i } else { i - 1 };
                    if i < 0 || i >= positions.len() as i64 {
                        return Err(bad());
                    }
                    face.push(i as u32);
                }
                let key = (group.clone(), material.clone());
                let at = *index.entry(key.clone()).or_insert_with(|| {
                    parts.push((key.0, key.1, Vec::new()));
                    parts.len() - 1
                });
                parts[at].2.push(face);
            }
            _ => {}
        }
    }
    let parts = parts
        .into_iter()
        .map(|(group, mat, faces)| {
            let name = match (&group, &mat) {
                (g, _) if !g.is_empty() => g.clone(),
                (_, Some(m)) => m.clone(),
                _ => String::new(),
            };
            let mut part = RawPart::new(name, mat.as_ref().and_then(|m| materials.get(m).copied()));
            // Only the vertices this part uses.
            let mut local: HashMap<u32, u32> = HashMap::new();
            for f in faces {
                let idx: Vec<u32> = f
                    .iter()
                    .map(|i| {
                        *local.entry(*i).or_insert_with(|| {
                            part.positions.push(positions[*i as usize]);
                            part.positions.len() as u32 - 1
                        })
                    })
                    .collect();
                part.polygon(&idx);
            }
            part
        })
        .collect();
    Ok(RawModel { parts, up: Up::Y })
}

/// Material name → diffuse colour (`Kd`, with `d` as opacity).
fn read_mtl(text: &str) -> HashMap<String, Color> {
    let mut out = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let mut w = line.split_whitespace();
        match (w.next(), &current) {
            (Some("newmtl"), _) => current = w.next().map(str::to_string),
            (Some("Kd"), Some(name)) => {
                let c: Vec<f64> = w.filter_map(|s| s.parse().ok()).collect();
                if let [r, g, b, ..] = c[..] {
                    let a = out.get(name).map_or(1.0, |c: &Color| c.a);
                    out.insert(name.clone(), Color::rgba(r, g, b, a));
                }
            }
            (Some("d"), Some(name)) => {
                if let Some(a) = w.next().and_then(|s| s.parse::<f64>().ok()) {
                    let c = out.entry(name.clone()).or_insert(Color::WHITE);
                    c.a = a.clamp(0.0, 1.0);
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_groups_materials_and_polygons() {
        let obj = "mtllib parts.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\n\
                   o body\nusemtl red\nf 1/1/1 2/2/2 3/3/3 4/4/4\nusemtl blue\nf -5 -1 -4\n\
                   o lid\nf 1 2 5\n";
        let mtl = b"newmtl red\nKd 1 0 0\nnewmtl blue\nKd 0 0 1\nd 0.5\n".to_vec();
        let raw = read(obj.as_bytes(), &|f| (f == "parts.mtl").then(|| mtl.clone())).unwrap();
        assert_eq!(raw.up, Up::Y);
        let names: Vec<&str> = raw.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["body", "body", "lid"]);
        assert_eq!(raw.parts[0].color, Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(raw.parts[0].triangles.len(), 2, "the quad is split into two triangles");
        assert_eq!(raw.parts[0].positions.len(), 4, "only the part's own vertices");
        assert_eq!(raw.parts[1].color, Some(Color::rgba(0.0, 0.0, 1.0, 0.5)));
        assert_eq!(raw.parts[1].positions, vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]]);
        // The lid has the last-used material.
        assert_eq!(raw.parts[2].color, raw.parts[1].color);
        assert!(read(b"v 0 0 0\nf 1 2 3\n", &|_| None).is_err(), "out-of-range index");
    }
}
