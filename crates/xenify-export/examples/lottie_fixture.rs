//! Writes a project exercising every Lottie-mapped feature, for comparing the
//! Xenify Vector render against lottie-web: `cargo run -p xenify-export --example lottie_fixture out.xenify`.

use xenify_doc::{
    Color, Composition, Easing, Fill, FillRule, Gradient, GradientKind, GradientStop, Id, Keyframe, Layer, LayerKind,
    Matte, MatteMode, Project, Property, ShapeItem, Stroke, TextLayer, Trim, Vec2,
};
use kurbo::BezPath;

fn keys<T: xenify_doc::Animatable>(k: Vec<(f64, T, Easing)>) -> Property<T> {
    Property::from_keyframes(k.into_iter().map(|(time, value, easing)| Keyframe { time, value, easing }).collect())
        .unwrap()
}

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let mut p = Project::new();
    let main = p.compositions[0].id;
    let mut id = 100;
    let mut next = || {
        id += 1;
        Id(id)
    };
    let mut layers = Vec::new();

    // Draw-on stroked circle (trimmed ellipse).
    let mut ring = ShapeItem::ellipse(Vec2::new(300.0, 300.0), Color::WHITE);
    ring.fill = None;
    ring.stroke = Some(Stroke {
        cap_roundness: 0.0,
        style: Default::default(),
        color: Color::rgb(1.0, 0.6, 0.1).into(),
        width: 24.0.into(),
        join: Default::default(),
        cap: xenify_doc::LineCap::Round,
        ..Default::default()
    });
    ring.trim = Some(Trim {
        end: keys(vec![(0.0, 0.0, Easing::EASE_IN_OUT), (1.5, 1.0, Easing::Linear)]),
        ..Default::default()
    });
    layers.push(Layer::shape(next(), "Ring", vec![ring]).with_position(Vec2::new(400.0, 300.0)));

    // Animated linear gradient on a rounded rect, rotating, with hold opacity.
    let mut card = ShapeItem::rect(Vec2::new(360.0, 220.0), Color::WHITE);
    if let xenify_doc::Geometry::Rect { corner_radius, .. } = &mut card.geometry {
        *corner_radius = 30.0.into();
    }
    let b = kurbo::Rect::new(-180.0, -110.0, 180.0, 110.0);
    let g0 = Gradient::across(
        b,
        GradientKind::Linear,
        0.0,
        vec![
            GradientStop { offset: 0.0, color: Color::rgb(1.0, 0.2, 0.4) },
            GradientStop { offset: 1.0, color: Color::rgb(0.2, 0.3, 1.0) },
        ],
    );
    let g1 = Gradient::across(
        b,
        GradientKind::Linear,
        90.0,
        vec![
            GradientStop { offset: 0.0, color: Color::rgb(0.1, 0.9, 0.5) },
            GradientStop { offset: 0.5, color: Color::rgb(1.0, 1.0, 0.2) },
            GradientStop { offset: 1.0, color: Color::rgb(0.9, 0.1, 0.9) },
        ],
    );
    card.fill = Some(Fill {
        color: Color::WHITE.into(),
        gradient: Some(keys(vec![(0.0, g0, Easing::EASE), (2.0, g1, Easing::Linear)]).into()),
        pattern: None,
    });
    let mut card_l = Layer::shape(next(), "Card", vec![card]).with_position(Vec2::new(1000.0, 280.0));
    card_l.transform.rotation =
        keys(vec![(0.0, -10.0, Easing::Bezier { x1: 0.3, y1: 1.6, x2: 0.6, y2: 1.0 }), (1.5, 10.0, Easing::Linear)]);
    card_l.opacity = keys(vec![(0.0, 0.4, Easing::Hold), (1.0, 1.0, Easing::Linear)]);
    layers.push(card_l);

    // Radial gradient ball parented to a moving null.
    let null = next();
    let mut n = Layer::new(null, "Mover", LayerKind::Null).with_position(Vec2::new(1500.0, 300.0));
    n.transform.position = keys(vec![
        (0.0, Vec2::new(1450.0, 250.0), Easing::EASE_IN_OUT),
        (2.5, Vec2::new(1600.0, 420.0), Easing::Linear),
    ]);
    layers.push(n);
    let mut ball = ShapeItem::ellipse(Vec2::new(180.0, 180.0), Color::WHITE);
    ball.fill.as_mut().unwrap().gradient = Some(
        Gradient::across(
            kurbo::Rect::new(-90.0, -90.0, 90.0, 90.0),
            GradientKind::Radial,
            0.0,
            vec![
                GradientStop { offset: 0.0, color: Color::WHITE },
                GradientStop { offset: 1.0, color: Color::rgb(0.1, 0.4, 0.9) },
            ],
        )
        .into(),
    );
    let mut ball_l = Layer::shape(next(), "Ball", vec![ball]);
    ball_l.meta.parent = Some(null);
    ball_l.transform.scale =
        keys(vec![(0.0, Vec2::new(0.5, 0.5), Easing::EASE_OUT), (1.0, Vec2::new(1.0, 1.0), Easing::Linear)]);
    layers.push(ball_l);

    // Even-odd star-in-square path with curves.
    let mut path = BezPath::new();
    path.move_to((-120.0, -120.0));
    path.line_to((120.0, -120.0));
    path.line_to((120.0, 120.0));
    path.line_to((-120.0, 120.0));
    path.close_path();
    path.move_to((0.0, -80.0));
    path.curve_to((60.0, -80.0), (80.0, -20.0), (60.0, 40.0));
    path.quad_to((0.0, 100.0), (-60.0, 40.0));
    path.close_path();
    let mut holes = ShapeItem::path(path, Some(Color::rgb(0.95, 0.95, 0.95)), None);
    holes.fill_rule = FillRule::EvenOdd;
    layers.push(Layer::shape(next(), "Holes", vec![holes]).with_position(Vec2::new(400.0, 800.0)));

    // Title revealed by an alpha matte (text slides up behind a line).
    let mask = next();
    let mut m = ShapeItem::rect(Vec2::new(700.0, 300.0), Color::WHITE);
    m.stroke = None;
    layers.push(Layer::shape(mask, "Reveal mask", vec![m]).with_position(Vec2::new(1000.0, 700.0)));
    let mut title = Layer::new(
        next(),
        "Title",
        LayerKind::Text(TextLayer::new("Lottie!", "Inter", 140.0, Color::WHITE).centered()),
    );
    title.transform.position =
        keys(vec![(0.2, Vec2::new(1000.0, 1000.0), Easing::EASE_OUT), (1.2, Vec2::new(1000.0, 720.0), Easing::Linear)]);
    title.meta.matte = Some(Matte { layer: mask, mode: MatteMode::Alpha });
    layers.push(title);

    // A group of dots with an inverted matte (a bar cuts through them).
    let group = next();
    layers.push(Layer::new(group, "Dots", LayerKind::Group).with_position(Vec2::new(1500.0, 800.0)));
    for i in 0..3 {
        let mut d = Layer::shape(
            next(),
            format!("Dot {i}"),
            vec![ShapeItem::ellipse(Vec2::new(120.0, 120.0), Color::rgb(0.2, 0.9, 0.7))],
        )
        .with_position(Vec2::new(-150.0 + 150.0 * i as f64, 0.0));
        d.meta.parent = Some(group);
        layers.push(d);
    }
    let bar = next();
    let mut bar_l = Layer::shape(bar, "Bar", vec![ShapeItem::rect(Vec2::new(40.0, 400.0), Color::WHITE)]);
    bar_l.transform.position = keys(vec![
        (0.0, Vec2::new(1300.0, 800.0), Easing::EASE_IN_OUT),
        (3.0, Vec2::new(1700.0, 800.0), Easing::Linear),
    ]);
    layers.push(bar_l);
    let gi = layers.iter().position(|l| l.id == group).unwrap();
    layers[gi].meta.matte = Some(Matte { layer: bar, mode: MatteMode::AlphaInverted });

    // A precomp starting at 1 s, scaled down in the corner.
    let inner_id = next();
    let mut spin =
        Layer::shape(next(), "Spinner", vec![ShapeItem::rect(Vec2::new(400.0, 400.0), Color::rgb(1.0, 0.8, 0.0))])
            .with_position(Vec2::new(960.0, 540.0));
    spin.transform.rotation = keys(vec![(0.0, 0.0, Easing::Linear), (2.0, 180.0, Easing::Linear)]);
    let inner = Composition {
        id: inner_id,
        settings: xenify_doc::CompSettings { name: "Inner".into(), ..Default::default() },
        layers: vec![spin],
    };
    let mut pre = Layer::precomp(next(), "Inner", &inner, Vec2::new(250.0, 1000.0));
    pre.transform.scale = Vec2::new(0.2, 0.2).into();
    pre.kind = LayerKind::Precomp { comp: inner_id, start: 1.0, looped: false };
    layers.push(pre);
    p.compositions.push(inner);

    let c = p.comp_mut(main).unwrap();
    c.layers = layers;
    c.settings.name = "Lottie fixture".into();
    p.reserve_id(Id(id + 1));
    std::fs::write(out, p.to_json()).unwrap();
}
