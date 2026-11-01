//! Writes a project showing every pen style on the same curve:
//! `cargo run -p xenify-export --example pen_gallery out.xenify`.

use kurbo::BezPath;
use xenify_doc::{
    Color, Id, Layer, LayerKind, LineCap, LineJoin, PenKind, PenStyle, Project, ShapeItem, Stroke, TextLayer, Vec2,
};

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let mut p = Project::new();
    let main = p.compositions[0].id;
    let mut layers = Vec::new();
    let mut id = 10;
    let cols = 4;
    for (i, kind) in PenKind::ALL.into_iter().enumerate() {
        let (cx, cy) = ((i % cols) as f64 * 470.0 + 250.0, (i / cols) as f64 * 175.0 + 90.0);
        let mut path = BezPath::new();
        path.move_to((-170.0, 20.0));
        path.curve_to((-80.0, -70.0), (40.0, 70.0), (170.0, -25.0));
        let mut shape = ShapeItem::path(path, None, Some((Color::from_hex("#f4d35e").unwrap(), 16.0)));
        let s = shape.stroke.as_mut().unwrap();
        s.style = PenStyle::preset(kind);
        s.cap = LineCap::Round;
        s.join = LineJoin::Round;
        id += 1;
        layers.push(Layer::shape(Id(id), kind.label(), vec![shape]).with_position(Vec2::new(cx, cy)));
        id += 1;
        layers.push(
            Layer::new(
                Id(id),
                "label",
                LayerKind::Text(TextLayer::new(kind.label(), "Inter", 22.0, Color::WHITE).centered()),
            )
            .with_position(Vec2::new(cx, cy + 62.0)),
        );
    }
    // Caps: flat, square, round, and rounded squares at 40% / 80%.
    for (i, (cap, r)) in [
        (LineCap::Butt, 0.0),
        (LineCap::Square, 0.0),
        (LineCap::Round, 0.0),
        (LineCap::Square, 0.4),
        (LineCap::Square, 0.8),
    ]
    .into_iter()
    .enumerate()
    {
        let mut path = BezPath::new();
        path.move_to((0.0, -40.0));
        path.line_to((0.0, 40.0));
        let mut shape = ShapeItem::path(path, None, Some((Color::from_hex("#ee6c4d").unwrap(), 40.0)));
        let s: &mut Stroke = shape.stroke.as_mut().unwrap();
        s.cap = cap;
        s.cap_roundness = r;
        id += 1;
        layers.push(Layer::shape(Id(id), "cap", vec![shape]).with_position(Vec2::new(1450.0 + i as f64 * 90.0, 990.0)));
    }
    let c = p.comp_mut(main).unwrap();
    c.settings.width = 1920;
    c.settings.height = 1080;
    c.layers = layers;
    p.reserve_id(Id(id + 1));
    std::fs::write(out, p.to_json()).unwrap();
}
