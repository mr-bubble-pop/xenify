//! 3MF (3D printing / CAD): zipped XML with objects, components, colours
//! and a build plate. Z is up.

use super::zip::Zip;
use super::{ModelError, RawModel, RawPart, Up, invalid};
use std::collections::HashMap;
use xenify_doc::Color;

/// A 3MF transform: `x' = x·m00 + y·m10 + z·m20 + m30`, …
type Matrix = [f64; 12];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

fn apply(m: &Matrix, p: [f64; 3]) -> [f64; 3] {
    [
        p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
        p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
        p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
    ]
}

/// `a` then `b`.
fn then(a: &Matrix, b: &Matrix) -> Matrix {
    let r = |i: usize, j: usize| (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum::<f64>();
    let t = apply(b, [a[9], a[10], a[11]]);
    [r(0, 0), r(0, 1), r(0, 2), r(1, 0), r(1, 1), r(1, 2), r(2, 0), r(2, 1), r(2, 2), t[0], t[1], t[2]]
}

fn parse_matrix(s: Option<&str>) -> Matrix {
    let v: Vec<f64> = s.unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
    v.try_into().unwrap_or(IDENTITY)
}

struct Object {
    name: String,
    vertices: Vec<[f64; 3]>,
    /// Triangle and its colour.
    triangles: Vec<([u32; 3], Option<Color>)>,
    components: Vec<(String, Matrix)>,
}

pub fn read(bytes: &[u8]) -> Result<RawModel, ModelError> {
    let zip = Zip::new(bytes)?;
    let model = zip
        .names()
        .find(|n| n.eq_ignore_ascii_case("3D/3dmodel.model"))
        .or_else(|| zip.names().find(|n| n.to_ascii_lowercase().ends_with(".model")))
        .map(str::to_string)
        .ok_or_else(|| invalid("the 3MF file has no model in it"))?;
    let xml = String::from_utf8_lossy(&zip.read(&model)?).into_owned();
    read_model(&xml)
}

fn read_model(xml: &str) -> Result<RawModel, ModelError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| invalid(format!("the 3MF model isn't valid XML: {e}")))?;
    let tag = |n: &roxmltree::Node, name: &str| n.is_element() && n.tag_name().name() == name;
    // Colour groups: id → colours.
    let mut palettes: HashMap<String, Vec<Color>> = HashMap::new();
    for n in doc.descendants() {
        let key = if tag(&n, "basematerials") {
            ("base", "displaycolor")
        } else if tag(&n, "colorgroup") {
            ("color", "color")
        } else {
            continue;
        };
        let colors = n
            .children()
            .filter(|c| tag(c, key.0))
            .map(|c| c.attribute(key.1).and_then(Color::from_hex).unwrap_or(Color::WHITE))
            .collect();
        if let Some(id) = n.attribute("id") {
            palettes.insert(id.to_string(), colors);
        }
    }
    let color = |pid: Option<&str>, index: Option<&str>| -> Option<Color> {
        let i: usize = index?.parse().ok()?;
        palettes.get(pid?)?.get(i).copied()
    };
    let mut objects: HashMap<String, Object> = HashMap::new();
    for o in doc.descendants().filter(|n| tag(n, "object")) {
        let Some(id) = o.attribute("id") else { continue };
        let default = color(o.attribute("pid"), o.attribute("pindex"));
        let mut obj = Object {
            name: o.attribute("name").unwrap_or("").to_string(),
            vertices: Vec::new(),
            triangles: Vec::new(),
            components: Vec::new(),
        };
        for n in o.descendants() {
            if tag(&n, "vertex") {
                let c = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                obj.vertices.push([c("x"), c("y"), c("z")]);
            } else if tag(&n, "triangle") {
                let v = |a: &str| n.attribute(a).and_then(|v| v.parse::<u32>().ok());
                let (Some(a), Some(b), Some(c)) = (v("v1"), v("v2"), v("v3")) else { continue };
                let col = color(n.attribute("pid").or(o.attribute("pid")), n.attribute("p1")).or(default);
                obj.triangles.push(([a, b, c], col));
            } else if tag(&n, "component")
                && let Some(to) = n.attribute("objectid")
            {
                obj.components.push((to.to_string(), parse_matrix(n.attribute("transform"))));
            }
        }
        objects.insert(id.to_string(), obj);
    }
    let mut items: Vec<(String, Matrix)> = doc
        .descendants()
        .filter(|n| tag(n, "item"))
        .filter_map(|n| Some((n.attribute("objectid")?.to_string(), parse_matrix(n.attribute("transform")))))
        .collect();
    if items.is_empty() {
        items = objects.keys().map(|k| (k.clone(), IDENTITY)).collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let unit = match doc.root_element().attribute("unit").unwrap_or("millimeter") {
        "micron" => 0.001,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        _ => 1.0,
    };
    let scale = [unit, 0.0, 0.0, 0.0, unit, 0.0, 0.0, 0.0, unit, 0.0, 0.0, 0.0];
    let mut parts = Vec::new();
    for (id, m) in items {
        place(&objects, &id, &then(&m, &scale), "", &mut parts, 0);
    }
    Ok(RawModel { parts, up: Up::Z })
}

/// Adds object `id` (and its components) placed by `m`, one part per colour.
fn place(objects: &HashMap<String, Object>, id: &str, m: &Matrix, parent: &str, out: &mut Vec<RawPart>, depth: usize) {
    let Some(o) = objects.get(id) else { return };
    if depth > 32 {
        return; // A component cycle.
    }
    let name = if o.name.is_empty() { parent.to_string() } else { o.name.clone() };
    let mut by_color: Vec<(Option<Color>, RawPart)> = Vec::new();
    for (t, c) in &o.triangles {
        if t.iter().any(|i| *i as usize >= o.vertices.len()) {
            continue;
        }
        let at = match by_color.iter().position(|(k, _)| k == c) {
            Some(i) => i,
            None => {
                let mut p = RawPart::new(name.clone(), *c);
                p.positions = o.vertices.iter().map(|v| apply(m, *v)).collect();
                by_color.push((*c, p));
                by_color.len() - 1
            }
        };
        by_color[at].1.triangles.push(*t);
    }
    // A mirroring transform turns the triangles inside out.
    let det =
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    for (_, mut p) in by_color {
        if det < 0.0 {
            p.triangles.iter_mut().for_each(|t| t.swap(1, 2));
        }
        out.push(p);
    }
    for (child, cm) in &o.components {
        place(objects, child, &then(cm, m), &name, out, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model3d::{ModelOptions, import_model, zip::tests::make_zip};

    const MODEL: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<model unit="centimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
       xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02">
  <resources>
    <basematerials id="1"><base name="Red" displaycolor="#FF0000"/><base name="Blue" displaycolor="#0000FFFF"/></basematerials>
    <object id="2" type="model" name="Tet" pid="1" pindex="0">
      <mesh>
        <vertices>
          <vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/>
        </vertices>
        <triangles>
          <triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/>
          <triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3" pid="1" p1="1"/>
        </triangles>
      </mesh>
    </object>
    <object id="3" type="model"><components><component objectid="2" transform="1 0 0 0 1 0 0 0 1 5 0 0"/></components></object>
  </resources>
  <build><item objectid="2"/><item objectid="3" transform="2 0 0 0 2 0 0 0 2 0 0 0"/></build>
</model>"##;

    #[test]
    fn reads_objects_components_and_colours() {
        let raw = read_model(MODEL).unwrap();
        // Two placements of the tetrahedron, each split into red and blue.
        assert_eq!(raw.parts.len(), 4);
        assert_eq!(raw.parts[0].name, "Tet");
        assert_eq!(raw.parts[0].color, Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(raw.parts[0].triangles.len(), 3);
        assert_eq!(raw.parts[1].color, Some(Color::rgba(0.0, 0.0, 1.0, 1.0)));
        // Centimetres → millimetres; the component's offset is scaled by the item.
        assert_eq!(raw.parts[0].positions[1], [10.0, 0.0, 0.0]);
        assert_eq!(raw.parts[2].positions[0], [100.0, 0.0, 0.0]);
        let zip = make_zip(&[("3D/3dmodel.model", MODEL.as_bytes())]);
        let m = import_model("t.3mf", &zip, ModelOptions::default(), &|_| None).unwrap();
        assert_eq!(m.mesh.parts.len(), 4);
        assert!(m.mesh.own_colors);
        assert_eq!(m.size, [120.0, 20.0, 20.0]);
    }
}
