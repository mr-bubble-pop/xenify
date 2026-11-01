use crate::{SceneContent, SceneLayer, evaluate};
use kurbo::{PathEl, Point, Rect, Shape};
use xenify_doc::{
    BezPath, Color, Composition, Connector, ConnectorEnd, Easing, Id, Keyframe, Layer, Marker, MarkerKind, NodeLabel,
    PortRef, Project, Property, ShapeItem, Vec2,
};

fn comp_with(layers: Vec<Layer>) -> Composition {
    let mut p = Project::new();
    let mut c = p.compositions.remove(0);
    c.layers = layers;
    c
}

fn node(id: u64, pos: Vec2) -> Layer {
    Layer::shape(Id(id), "Box", vec![ShapeItem::rect(Vec2::new(100.0, 60.0), Color::WHITE)]).with_position(pos)
}

fn line_shape() -> ShapeItem {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((1.0, 0.0));
    ShapeItem::path(p, None, Some((Color::BLACK, 2.0)))
}

fn connector(id: u64, from: u64, to: u64) -> Layer {
    let mut l = Layer::shape(Id(id), "Connector", vec![line_shape()]);
    l.meta.connector = Some(Box::new(Connector::new(
        ConnectorEnd::glued(Id(from), PortRef::Auto, Vec2::ZERO),
        ConnectorEnd::glued(Id(to), PortRef::Auto, Vec2::ZERO),
    )));
    l
}

/// The connector's line in scene space.
fn line_of(l: &SceneLayer) -> BezPath {
    let SceneContent::Shapes(items) = &l.content else { panic!("not shapes: {:?}", l.content) };
    l.transform * &items[0].path
}

fn ends(p: &BezPath) -> (Point, Point) {
    let pts: Vec<Point> = p
        .elements()
        .iter()
        .filter_map(|e| match e {
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(*p),
            _ => None,
        })
        .collect();
    (pts[0], *pts.last().unwrap())
}

fn crosses(p: &BezPath, r: Rect) -> bool {
    let mut hit = false;
    kurbo::flatten(p, 0.5, |el| {
        if let PathEl::LineTo(q) | PathEl::MoveTo(q) = el {
            hit |= q.x > r.x0 + 1e-6 && q.x < r.x1 - 1e-6 && q.y > r.y0 + 1e-6 && q.y < r.y1 - 1e-6;
        }
    });
    // Also sample along segments.
    for seg in p.segments() {
        for k in 0..=40 {
            let q = kurbo::ParamCurve::eval(&seg, k as f64 / 40.0);
            hit |= q.x > r.x0 + 1e-6 && q.x < r.x1 - 1e-6 && q.y > r.y0 + 1e-6 && q.y < r.y1 - 1e-6;
        }
    }
    hit
}

#[test]
fn connector_follows_its_shapes() {
    let mut b = node(2, Vec2::new(500.0, 100.0));
    b.transform.position = Property::from_keyframes(vec![
        Keyframe { time: 0.0, value: Vec2::new(500.0, 100.0), easing: Easing::Linear },
        Keyframe { time: 1.0, value: Vec2::new(500.0, 400.0), easing: Easing::Linear },
    ])
    .unwrap();
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), b, connector(3, 1, 2)]);
    let (s, e) = ends(&line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap()));
    assert_eq!(s, Point::new(150.0, 100.0));
    // Stops short of the shape by the arrowhead? No arrowhead: exactly at it.
    assert_eq!(e, Point::new(450.0, 100.0));
    let (s, e) = ends(&line_of(evaluate(&comp, 1.0).layer(Id(3)).unwrap()));
    // B is now below and to the right: leaves A and reaches B on facing sides.
    assert!(s.x == 150.0 || s.y == 130.0, "{s:?}");
    assert!(e.x == 450.0 || e.y == 370.0, "{e:?}");
}

#[test]
fn connector_goes_around_shapes_in_the_way() {
    let wall = Layer::shape(Id(9), "Wall", vec![ShapeItem::rect(Vec2::new(60.0, 300.0), Color::WHITE)])
        .with_position(Vec2::new(300.0, 100.0));
    let comp =
        comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), wall, connector(3, 1, 2)]);
    let line = line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap());
    assert!(!crosses(&line, Rect::new(270.0, -50.0, 330.0, 250.0)), "{line:?}");
}

#[test]
fn missing_target_leaves_a_free_end() {
    let mut c = connector(3, 1, 42);
    c.meta.connector.as_mut().unwrap().to.point = Vec2::new(400.0, 300.0);
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), c]);
    let (_, e) = ends(&line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap()));
    assert_eq!(e, Point::new(400.0, 300.0));
}

#[test]
fn crossing_connectors_hop() {
    // A→B horizontal, C→D vertical through its middle.
    let comp = comp_with(vec![
        node(1, Vec2::new(100.0, 300.0)),
        node(2, Vec2::new(700.0, 300.0)),
        node(4, Vec2::new(400.0, 50.0)),
        node(5, Vec2::new(400.0, 550.0)),
        connector(3, 1, 2),
        connector(6, 4, 5),
    ]);
    let scene = evaluate(&comp, 0.0);
    let curves = |id| {
        line_of(scene.layer(Id(id)).unwrap()).elements().iter().filter(|e| matches!(e, PathEl::CurveTo(..))).count()
    };
    assert_eq!(curves(3), 0, "the first one drawn doesn't hop");
    assert_eq!(curves(6), 2, "one arc (two quarter curves)");
}

#[test]
fn arrowheads_are_drawn_and_the_line_stops_under_them() {
    let mut c = connector(3, 1, 2);
    if let xenify_doc::LayerKind::Shape { shapes } = &mut c.kind {
        shapes[0].stroke.as_mut().unwrap().end_marker = Marker { kind: MarkerKind::Triangle, size: 1.0 };
    }
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), c]);
    let scene = evaluate(&comp, 0.0);
    let l = scene.layer(Id(3)).unwrap();
    let SceneContent::Shapes(items) = &l.content else { panic!() };
    assert_eq!(items.len(), 2);
    let head = &items[1];
    assert!(head.fill.is_some() && head.stroke.is_none());
    let tip = head.path.bounding_box();
    assert!((tip.x1 - 450.0).abs() < 1e-6, "tip at the shape: {tip:?}");
    let (_, e) = ends(&line_of(l));
    assert!(e.x < 450.0 - 5.0, "line pulled back under the head: {e:?}");
}

#[test]
fn plain_lines_get_arrowheads_too() {
    let mut s = line_shape();
    s.stroke.as_mut().unwrap().start_marker = Marker { kind: MarkerKind::Circle, size: 1.0 };
    s.geometry = xenify_doc::Geometry::Path {
        path: {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((100.0, 0.0));
            p
        }
        .into(),
    };
    let comp = comp_with(vec![Layer::shape(Id(1), "Line", vec![s])]);
    let scene = evaluate(&comp, 0.0);
    let SceneContent::Shapes(items) = &scene.layer(Id(1)).unwrap().content else { panic!() };
    assert_eq!(items.len(), 2);
    let (s, _) = ends(&items[0].path);
    assert!(s.x > 5.0);
}

#[test]
fn labels_draw_inside_the_shape() {
    let mut n = node(1, Vec2::new(100.0, 100.0));
    n.meta.label = Some(NodeLabel::new("Start"));
    let comp = comp_with(vec![n]);
    let scene = evaluate(&comp, 0.0);
    let l = scene.layer(Id(1)).unwrap();
    let SceneContent::Group(inner) = &l.content else { panic!("{:?}", l.content) };
    let SceneContent::Text(t) = &inner[1].content else { panic!() };
    assert_eq!(t.text, "Start");
    let frame = t.frame.unwrap().rect;
    assert_eq!(inner[1].transform * frame.center(), Point::new(100.0, 100.0));
    // The labelled node still glues connectors.
    let comp = comp_with(vec![
        {
            let mut n = node(1, Vec2::new(100.0, 100.0));
            n.meta.label = Some(NodeLabel::new("A"));
            n
        },
        node(2, Vec2::new(500.0, 100.0)),
        connector(3, 1, 2),
    ]);
    let (s, _) = ends(&line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap()));
    assert_eq!(s, Point::new(150.0, 100.0));
}

#[test]
fn fixed_ports_are_used() {
    let mut c = connector(3, 1, 2);
    let conn = c.meta.connector.as_mut().unwrap();
    conn.from.attach.as_mut().unwrap().port = PortRef::Port(2); // bottom
    conn.to.attach.as_mut().unwrap().port = PortRef::Port(2);
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), c]);
    let (s, e) = ends(&line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap()));
    assert_eq!(s, Point::new(100.0, 130.0));
    assert_eq!(e, Point::new(500.0, 130.0));
}

#[test]
fn connector_labels_sit_on_a_box_on_diagram_canvases() {
    let mut c = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), connector(3, 1, 2)]);
    c.layers[2].meta.label = Some(NodeLabel::new("yes"));
    // An animation canvas: beside the line, no box.
    let scene = evaluate(&c, 0.0);
    let SceneContent::Group(members) = &scene.layer(Id(3)).unwrap().content else { panic!("a group") };
    assert_eq!(members.len(), 2, "line and text");
    // A diagram canvas: a box of the canvas colour behind the text, on the line.
    xenify_ops_free_set_diagram(&mut c);
    let scene = evaluate(&c, 0.0);
    let SceneContent::Group(members) = &scene.layer(Id(3)).unwrap().content else { panic!("a group") };
    assert_eq!(members.len(), 3, "line, box, text");
    let SceneContent::Shapes(items) = &members[1].content else { panic!("the box") };
    assert_eq!(items[0].fill, Some(c.settings.background));
    let on = members[2].transform.translation();
    assert!((on.y - 100.0).abs() < 1.0, "on the line: {on:?}");
}

/// Makes `c` a diagram canvas (as `ops::diagram::set_kind` does).
fn xenify_ops_free_set_diagram(c: &mut Composition) {
    c.settings.kind = xenify_doc::CompKind::Diagram;
}

#[test]
fn straight_lines_stop_at_the_real_outline() {
    let circle = |id: u64, pos: Vec2| {
        Layer::shape(Id(id), "Circle", vec![ShapeItem::ellipse(Vec2::new(100.0, 100.0), Color::WHITE)])
            .with_position(pos)
    };
    let mut line = connector(3, 1, 2);
    if let Some(c) = line.meta.connector.as_deref_mut() {
        c.route = xenify_doc::RouteStyle::Straight;
    }
    let c = comp_with(vec![circle(1, Vec2::new(100.0, 100.0)), circle(2, Vec2::new(400.0, 400.0)), line]);
    let scene = evaluate(&c, 0.0);
    let (a, b) = ends(&line_of(scene.layer(Id(3)).unwrap()));
    // On the circles (radius 50), not the corners of their boxes (≈ 70.7 out).
    assert!((a.distance(Point::new(100.0, 100.0)) - 50.0).abs() < 1.0, "{a:?}");
    assert!((b.distance(Point::new(400.0, 400.0)) - 50.0).abs() < 1.0, "{b:?}");
}

#[test]
fn exported_connector_labels_follow_moving_shapes() {
    let mut p = Project::new();
    let cid = p.compositions[0].id;
    let mut moving = node(2, Vec2::new(500.0, 100.0));
    let key = |time, value| Keyframe { time, value, easing: Easing::Linear };
    moving.transform.position =
        Property::from_keyframes(vec![key(0.0, Vec2::new(500.0, 100.0)), key(1.0, Vec2::new(500.0, 400.0))]).unwrap();
    let mut line = connector(3, 1, 2);
    line.meta.label = Some(NodeLabel::new("yes"));
    {
        let c = p.comp_mut(cid).unwrap();
        c.settings.duration = 1.0;
        c.settings.fps = 10.0;
        c.settings.kind = xenify_doc::CompKind::Diagram;
        c.layers = vec![node(1, Vec2::new(100.0, 100.0)), moving, line];
    }
    p.reserve_id(Id(10));
    crate::diagram::labels_as_layers(&mut p);
    let c = p.comp(cid).unwrap();
    let label = c.layers.iter().find(|l| l.meta.name.ends_with("label")).expect("a label layer");
    assert!(label.transform.position.is_animated(), "it follows the line");
    let (a, b) = (label.transform.position.value_at(0.0), label.transform.position.value_at(1.0));
    assert!((b - a).hypot() > 50.0, "it moves with the line: {a:?} → {b:?}");
    assert!(c.layers.iter().any(|l| l.meta.name.ends_with("label box")), "its box comes too");
}

#[test]
fn hollow_triangles_are_outlined_and_the_line_stops_at_their_base() {
    let mut c = connector(3, 1, 2);
    if let xenify_doc::LayerKind::Shape { shapes } = &mut c.kind {
        shapes[0].stroke.as_mut().unwrap().end_marker = Marker { kind: MarkerKind::OpenTriangle, size: 1.0 };
    }
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), c]);
    let scene = evaluate(&comp, 0.0);
    let l = scene.layer(Id(3)).unwrap();
    let SceneContent::Shapes(items) = &l.content else { panic!() };
    assert_eq!(items.len(), 2);
    let head = &items[1];
    assert!(head.fill.is_none() && head.stroke.is_some(), "outlined, not filled");
    assert!(head.path.elements().iter().any(|e| matches!(e, PathEl::ClosePath)));
    let b = head.path.bounding_box();
    assert!((b.x1 - 450.0).abs() < 1e-6, "tip at the shape: {b:?}");
    let (_, e) = ends(&line_of(l));
    assert!((e.x - b.x0).abs() < 1e-6, "line ends at the base: {e:?} vs {b:?}");
}

#[test]
fn dashed_connectors_stay_dashed_with_solid_arrowheads() {
    let mut c = connector(3, 1, 2);
    if let xenify_doc::LayerKind::Shape { shapes } = &mut c.kind {
        let s = shapes[0].stroke.as_mut().unwrap();
        s.dashes = vec![10.0, 8.0];
        s.end_marker = Marker { kind: MarkerKind::Open, size: 1.0 };
    }
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), c]);
    let scene = evaluate(&comp, 0.0);
    let SceneContent::Shapes(items) = &scene.layer(Id(3)).unwrap().content else { panic!() };
    assert_eq!(items[0].stroke.as_ref().unwrap().dashes, vec![10.0, 8.0]);
    assert!(items[1].stroke.as_ref().unwrap().dashes.is_empty());
}

#[test]
fn top_labels_sit_at_the_top() {
    let mut n = node(1, Vec2::new(100.0, 100.0));
    let mut label = NodeLabel::new("Order");
    label.top = true;
    n.meta.label = Some(label);
    let scene = evaluate(&comp_with(vec![n]), 0.0);
    let SceneContent::Group(inner) = &scene.layer(Id(1)).unwrap().content else { panic!() };
    let SceneContent::Text(t) = &inner[1].content else { panic!() };
    assert_eq!(t.valign, xenify_doc::TextVAlign::Top);
}

/// Distance from `p` to the outline of a `w`×`h` box centred on `c`, turned by `deg`.
fn off_turned_box(p: Point, c: Point, w: f64, h: f64, deg: f64) -> f64 {
    let a = -deg.to_radians();
    let d = p - c;
    let l = Point::new(d.x * a.cos() - d.y * a.sin(), d.x * a.sin() + d.y * a.cos());
    let (dx, dy) = (l.x.abs() - w / 2.0, l.y.abs() - h / 2.0);
    if dx > 0.0 || dy > 0.0 { dx.max(0.0).hypot(dy.max(0.0)) } else { -dx.max(dy) }
}

#[test]
fn elbow_lines_meet_a_turned_shape_and_spots_turn_with_it() {
    let mut a = node(1, Vec2::new(100.0, 100.0));
    a.transform.rotation = Property::Static(30.0);
    let comp = comp_with(vec![a.clone(), node(2, Vec2::new(500.0, 100.0)), connector(3, 1, 2)]);
    let (s, _) = ends(&line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap()));
    assert!(
        off_turned_box(s, Point::new(100.0, 100.0), 100.0, 60.0, 30.0).abs() < 0.5,
        "on the turned box's edge, not its bounding box: {s:?}"
    );
    // A spot at the top-right corner of its own box stays on that corner when it's turned.
    let mut c = connector(3, 1, 2);
    c.meta.connector.as_mut().unwrap().from.attach.as_mut().unwrap().port = PortRef::Spot { x: 1.0, y: 0.0 };
    let comp = comp_with(vec![a, node(2, Vec2::new(500.0, 100.0)), c]);
    let (s, _) = ends(&line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap()));
    let (r, k) = (30f64.to_radians(), Vec2::new(50.0, -30.0));
    let corner = Point::new(100.0 + k.x * r.cos() - k.y * r.sin(), 100.0 + k.x * r.sin() + k.y * r.cos());
    assert!((s - corner).hypot() < 0.5, "{s:?} vs {corner:?}");
    // edge_spot finds that corner from a point near it.
    let scene = evaluate(&comp, 0.0);
    let (q, f) = super::edge_spot(&comp, &scene, Id(1), corner + Vec2::new(3.0, 0.0), 8.0).unwrap();
    assert!((q - corner).hypot() < 4.0 && (f.x - 1.0).abs() < 0.08 && f.y < 0.08, "{q:?} {f:?}");
}

#[test]
fn a_bottom_port_is_reached_from_below() {
    let mut c = connector(3, 1, 2);
    c.meta.connector.as_mut().unwrap().to.attach.as_mut().unwrap().port = PortRef::Port(2); // bottom
    let comp = comp_with(vec![node(1, Vec2::new(100.0, 100.0)), node(2, Vec2::new(500.0, 100.0)), c]);
    let line = line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap());
    assert!(!crosses(&line, Rect::new(450.0, 70.0, 550.0, 130.0)), "through its own shape: {:?}", line.elements());
}

#[test]
fn a_spot_on_the_bottom_is_reached_from_below() {
    for (fx, rot) in [(0.5, 0.0), (0.625, 0.0), (0.625, 80.0)] {
        let mut c = connector(3, 1, 2);
        let conn = c.meta.connector.as_mut().unwrap();
        conn.to.attach.as_mut().unwrap().port = PortRef::Spot { x: fx, y: 1.0 };
        conn.from.attach.as_mut().unwrap().port = PortRef::Spot { x: 0.387, y: 0.0 };
        let mut a = node(1, Vec2::new(100.0, 100.0));
        a.transform.rotation = Property::Static(rot);
        let comp = comp_with(vec![a, node(2, Vec2::new(500.0, 100.0)), c]);
        let line = line_of(evaluate(&comp, 0.0).layer(Id(3)).unwrap());
        assert!(
            !crosses(&line, Rect::new(450.0, 70.0, 550.0, 130.0)),
            "{fx} {rot}: through its own shape: {:?}",
            line.elements()
        );
    }
}
