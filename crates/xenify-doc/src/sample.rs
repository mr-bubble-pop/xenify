//! A small demo project, used for first launch, tests and the CLI.

use crate::{Color, CompSettings, Easing, Keyframe, Layer, LineCap, LineJoin, Project, Property, ShapeItem, Stroke};
use kurbo::Vec2;

fn keys<T: crate::Animatable>(k: &[(f64, T, Easing)]) -> Property<T> {
    Property::from_keyframes(
        k.iter().map(|(time, value, easing)| Keyframe { time: *time, value: value.clone(), easing: *easing }).collect(),
    )
    .expect("non-empty")
}

/// 1280×720, 3 s: a bouncing ball, a spinning square that changes colour, a
/// ring that fades in and a title that scales up.
pub fn demo_project() -> Project {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let (ball, square, ring, title) = (p.alloc_id(), p.alloc_id(), p.alloc_id(), p.alloc_id());

    let mut ring_layer = Layer::shape(
        ring,
        "Ring",
        vec![ShapeItem {
            stroke: Some(Stroke {
                cap_roundness: 0.0,
                style: Default::default(),
                color: Color::from_hex("#ffd166").unwrap().into(),
                width: 12.0.into(),
                join: LineJoin::Round,
                cap: LineCap::Round,
                ..Default::default()
            }),
            fill: None,
            ..ShapeItem::ellipse(Vec2::new(420.0, 420.0), Color::WHITE)
        }],
    )
    .with_position(Vec2::new(640.0, 360.0));
    ring_layer.opacity = keys(&[(0.0, 0.0, Easing::EASE_OUT), (1.0, 1.0, Easing::Linear)]);

    let mut square_layer = Layer::shape(
        square,
        "Square",
        vec![ShapeItem {
            geometry: crate::Geometry::Rect {
                position: Vec2::ZERO.into(),
                size: Vec2::new(160.0, 160.0).into(),
                corner_radius: 24.0.into(),
            },
            fill: Some(crate::Fill {
                gradient: None,
                color: keys(&[
                    (0.0, Color::from_hex("#ef476f").unwrap(), Easing::EASE_IN_OUT),
                    (3.0, Color::from_hex("#118ab2").unwrap(), Easing::Linear),
                ]),
                pattern: None,
            }),
            ..ShapeItem::rect(Vec2::ZERO, Color::WHITE)
        }],
    )
    .with_position(Vec2::new(640.0, 360.0));
    square_layer.transform.rotation = keys(&[(0.0, 0.0, Easing::EASY_EASE), (3.0, 360.0, Easing::Linear)]);

    let mut ball_layer = Layer::shape(
        ball,
        "Ball",
        vec![ShapeItem::ellipse(Vec2::new(80.0, 80.0), Color::from_hex("#06d6a0").unwrap())],
    );
    ball_layer.transform.position = keys(&[
        (0.0, Vec2::new(160.0, 200.0), Easing::EASE_IN),
        (0.75, Vec2::new(400.0, 620.0), Easing::EASE_OUT),
        (1.5, Vec2::new(640.0, 200.0), Easing::EASE_IN),
        (2.25, Vec2::new(880.0, 620.0), Easing::EASE_OUT),
        (3.0, Vec2::new(1120.0, 200.0), Easing::Linear),
    ]);

    let mut title_layer = Layer::new(
        title,
        "Title",
        crate::LayerKind::Text(crate::TextLayer::new("Xenify", "Inter", 64.0, Color::WHITE).centered()),
    )
    .with_position(Vec2::new(640.0, 660.0));
    title_layer.opacity = keys(&[(0.5, 0.0, Easing::EASE_OUT), (1.5, 1.0, Easing::Linear)]);
    title_layer.transform.scale =
        keys(&[(0.5, Vec2::new(0.6, 0.6), Easing::EASE_OUT), (1.5, Vec2::new(1.0, 1.0), Easing::Linear)]);

    let c = p.comp_mut(comp).unwrap();
    c.settings =
        CompSettings { name: "Demo".into(), width: 1280, height: 720, fps: 30.0, duration: 3.0, ..Default::default() };
    c.layers = vec![ring_layer, square_layer, ball_layer, title_layer];
    p
}

#[cfg(test)]
mod tests {
    #[test]
    fn demo_round_trips() {
        let p = super::demo_project();
        assert_eq!(crate::Project::from_json(&p.to_json()).unwrap(), p);
    }
}
