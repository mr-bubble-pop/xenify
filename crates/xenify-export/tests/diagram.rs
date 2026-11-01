//! Diagrams (connectors, arrowheads, labels, line jumps) in SVG and Lottie.

use xenify_doc::{
    BezPath, Color, Connector, ConnectorEnd, Id, Layer, LayerKind, Marker, MarkerKind, NodeLabel, PortRef, Project,
    ShapeItem, Vec2,
};
use xenify_export::{lottie::export_lottie, svg::export_svg};
use xenify_render::TextEngine;

fn node(id: u64, name: &str, pos: Vec2) -> Layer {
    let mut l =
        Layer::shape(Id(id), name, vec![ShapeItem::rect(Vec2::new(120.0, 60.0), Color::WHITE)]).with_position(pos);
    l.meta.label = Some(NodeLabel::new(name));
    l
}

fn connector(id: u64, from: u64, to: u64) -> Layer {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((1.0, 0.0));
    let mut shape = ShapeItem::path(p, None, Some((Color::BLACK, 2.0)));
    shape.stroke.as_mut().unwrap().end_marker = Marker { kind: MarkerKind::Triangle, size: 1.0 };
    let mut l = Layer::shape(Id(id), "Connector", vec![shape]);
    l.meta.connector = Some(Box::new(Connector::new(
        ConnectorEnd::glued(Id(from), PortRef::Auto, Vec2::ZERO),
        ConnectorEnd::glued(Id(to), PortRef::Auto, Vec2::ZERO),
    )));
    l
}

fn diagram() -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let mut edge = connector(3, 1, 2);
    edge.meta.label = Some(NodeLabel::new("yes"));
    p.comp_mut(comp).unwrap().layers = vec![
        node(1, "Start", Vec2::new(200.0, 300.0)),
        node(2, "Finish", Vec2::new(900.0, 300.0)),
        node(4, "Top", Vec2::new(550.0, 80.0)),
        node(5, "Bottom", Vec2::new(550.0, 600.0)),
        edge,
        connector(6, 4, 5),
    ];
    p.reserve_id(Id(10));
    (p, comp)
}

#[test]
fn svg_has_connectors_arrowheads_labels_and_a_hop() {
    let (p, comp) = diagram();
    let scene = xenify_anim::evaluate_project(&p, comp, 0.0);
    let svg = export_svg(&scene, &mut TextEngine::new());
    for word in ["Start", "Finish", "yes"] {
        assert!(svg.contains(word), "label {word} missing");
    }
    // The vertical connector hops the horizontal one: cubic segments.
    assert!(svg.contains(" C") || svg.contains("C "), "no hop curve in {svg}");
}

#[test]
fn lottie_bakes_connectors_and_turns_labels_into_text_layers() {
    let (p, comp) = diagram();
    let json = export_lottie(&p, comp, &mut TextEngine::new()).unwrap();
    let layers = json["layers"].as_array().unwrap();
    let names: Vec<&str> = layers.iter().filter_map(|l| l["nm"].as_str()).collect();
    for n in ["Start label", "Finish label", "Connector label"] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    let edge = layers.iter().find(|l| l["nm"] == "Connector").unwrap();
    // Baked from the scene: the line plus its arrowhead.
    let text = edge["shapes"].to_string();
    assert!(text.contains("\"sh\""), "{text}");
    // The source project keeps its labels.
    let LayerKind::Shape { .. } = &p.comp(comp).unwrap().layers[0].kind else { panic!() };
    assert!(p.comp(comp).unwrap().layers[0].meta.label.is_some());
}
