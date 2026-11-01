//! STL, binary and ASCII. Z is up.

use super::{ModelError, RawModel, RawPart, Up, cross, dot, invalid, sub};

pub fn read(bytes: &[u8]) -> Result<RawModel, ModelError> {
    let binary_len = |n: usize| 84 + 50 * n;
    let count = bytes.get(80..84).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let exact_binary = count.is_some_and(|n| binary_len(n) == bytes.len());
    let looks_ascii = bytes.trim_ascii_start().starts_with(b"solid") && !exact_binary;
    let parts = if looks_ascii {
        ascii(bytes)?
    } else if let Some(n) = count.filter(|n| binary_len(*n) <= bytes.len()) {
        vec![binary(bytes, n)]
    } else {
        return Err(invalid("this isn't a readable STL file (it's cut short)"));
    };
    Ok(RawModel { parts, up: Up::Z })
}

/// Adds a triangle, turning it round when it disagrees with the file's
/// normal (which exporters usually get right even when the order is off).
fn push(part: &mut RawPart, v: [[f64; 3]; 3], normal: [f64; 3]) {
    let base = part.positions.len() as u32;
    part.positions.extend(v);
    let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
    if dot(n, normal) < 0.0 {
        part.triangles.push([base, base + 2, base + 1]);
    } else {
        part.triangles.push([base, base + 1, base + 2]);
    }
}

fn binary(bytes: &[u8], n: usize) -> RawPart {
    let f = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as f64;
    let v = |o: usize| [f(o), f(o + 4), f(o + 8)];
    let mut part = RawPart::new("", None);
    part.positions.reserve(n * 3);
    part.triangles.reserve(n);
    for t in 0..n {
        let o = 84 + t * 50;
        push(&mut part, [v(o + 12), v(o + 24), v(o + 36)], v(o));
    }
    part
}

fn ascii(bytes: &[u8]) -> Result<Vec<RawPart>, ModelError> {
    let text = String::from_utf8_lossy(bytes);
    let mut parts = Vec::new();
    let mut part = RawPart::new("", None);
    let mut normal = [0.0; 3];
    let mut corners: Vec<[f64; 3]> = Vec::new();
    let num = |s: Option<&str>| -> Result<f64, ModelError> {
        s.and_then(|s| s.parse().ok()).ok_or_else(|| invalid("an STL vertex has a bad number"))
    };
    for line in text.lines() {
        let mut w = line.split_whitespace();
        match w.next() {
            Some("solid") => {
                part = RawPart::new(w.collect::<Vec<_>>().join(" "), None);
            }
            Some("facet") => {
                corners.clear();
                normal = [0.0; 3];
                if w.next() == Some("normal") {
                    normal = [num(w.next())?, num(w.next())?, num(w.next())?];
                }
            }
            Some("vertex") => corners.push([num(w.next())?, num(w.next())?, num(w.next())?]),
            Some("endfacet") => {
                if corners.len() >= 3 {
                    for k in 1..corners.len() - 1 {
                        push(&mut part, [corners[0], corners[k], corners[k + 1]], normal);
                    }
                }
            }
            Some("endsolid") => parts.push(std::mem::take(&mut part)),
            _ => {}
        }
    }
    if !part.triangles.is_empty() {
        parts.push(part);
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model3d::{ModelOptions, import_model, tests::cube_soup};

    fn binary_stl(part: &RawPart) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out.extend((part.triangles.len() as u32).to_le_bytes());
        for t in &part.triangles {
            out.extend([0u8; 12]); // No normal: trust the order.
            for i in t {
                for c in part.positions[*i as usize] {
                    out.extend((c as f32).to_le_bytes());
                }
            }
            out.extend([0u8; 2]);
        }
        out
    }

    #[test]
    fn reads_binary_and_ascii() {
        let cube = cube_soup([10.0, 10.0, 10.0]);
        let bin = binary_stl(&cube);
        // Binary files that happen to start with "solid" are still binary.
        let mut tricky = bin.clone();
        tricky[..5].copy_from_slice(b"solid");
        for bytes in [bin, tricky] {
            let m = import_model("part.stl", &bytes, ModelOptions::default(), &|_| None).unwrap();
            assert_eq!(m.faces, 6);
            assert_eq!(m.size, [10.0, 10.0, 10.0]);
        }
        let ascii = "solid tri\n facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid tri\n";
        let raw = read(ascii.as_bytes()).unwrap();
        assert_eq!(raw.parts.len(), 1);
        assert_eq!(raw.parts[0].name, "tri");
        // Wound counter-clockwise about +z but the normal says -z: turned round.
        assert_eq!(raw.parts[0].triangles, vec![[0, 2, 1]]);
        assert!(read(b"\0\0\0").is_err());
    }
}
