//! Writes a project with baked physics (bouncing balls squishing onto a shelf,
//! and an orbit): `cargo run -p xenify-export --example physics_demo out.xenify`.

use xenify_doc::{Color, Easing, Id, Keyframe, Layer, Project, Property, ShapeItem, Vec2};
use xenify_ops::physics::{OrbitSettings, PhysicsSettings, orbit, simulate};

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let ball = |id: u64, x: f64, y: f64, c: &str, d: f64| {
        Layer::shape(
            Id(id),
            format!("Ball {id}"),
            vec![ShapeItem::ellipse(Vec2::new(d, d), Color::from_hex(c).unwrap())],
        )
        .with_position(Vec2::new(x, y))
    };
    let mut thrown = ball(2, 300.0, 250.0, "#ee6c4d", 110.0);
    thrown.transform.position = Property::from_keyframes(vec![
        Keyframe { time: 0.0, value: Vec2::new(300.0, 250.0), easing: Easing::Linear },
        Keyframe { time: 1.0, value: Vec2::new(900.0, 150.0), easing: Easing::Linear },
    ])
    .unwrap();
    let shelf = Layer::shape(
        Id(3),
        "Pillow",
        vec![ShapeItem::rect(Vec2::new(520.0, 90.0), Color::from_hex("#98c1d9").unwrap())],
    )
    .with_position(Vec2::new(760.0, 700.0));
    let sun = ball(4, 1500.0, 350.0, "#f4d35e", 140.0);
    let planet = ball(5, 1750.0, 350.0, "#3d5a80", 50.0);
    let moon = ball(6, 1650.0, 350.0, "#e0fbfc", 30.0);
    let c = p.comp_mut(comp).unwrap();
    c.settings.duration = 4.0;
    c.layers = vec![shelf, ball(1, 700.0, 100.0, "#f4d35e", 140.0), thrown, sun, planet, moon];
    p.reserve_id(Id(100));
    let settings = PhysicsSettings { squish: 0.9, squish_hit: true, ..Default::default() };
    let cmd = simulate(&p, comp, &[Id(1), Id(2)], 0.0, &settings, &mut xenify_anim::ApproxText).unwrap();
    xenify_ops::apply(&mut p, cmd).unwrap();
    let o = OrbitSettings { speed: 0.4, flatten: 0.45, kepler: true, ..Default::default() };
    let cmd = orbit(&p, comp, Id(4), &[Id(5), Id(6)], 0.0, &o).unwrap();
    xenify_ops::apply(&mut p, cmd).unwrap();
    std::fs::write(out, p.to_json()).unwrap();
}
