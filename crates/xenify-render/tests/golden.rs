//! Golden-image tests: known scenes are rendered and compared with reference
//! PNGs in `tests/golden/`, so any change to how things look is noticed.
//!
//! - Missing references are written (and the test passes, saying so).
//! - `XENIFY_UPDATE_GOLDEN=1 cargo test -p xenify-render --test golden`
//!   rewrites them after an intended visual change — check the new images!
//! - Small anti-aliasing differences between GPUs/drivers are tolerated.
//! - Skipped (with a message) when no GPU adapter exists.

use std::path::PathBuf;
use xenify_anim::evaluate_project;
use xenify_doc::{
    Color, Composition, Easing, Expression, Fill, Gradient, GradientKind, GradientStop, Id, Keyframe, Layer, LayerKind,
    Matte, MatteMode, Project, PropPath, Property, ShapeItem, Stroke, TextLayer, Trim, Vec2,
};
use xenify_render::{Offscreen, RgbaImage};

const W: u32 = 320;
const H: u32 = 180;
/// A pixel "differs" when any channel is off by more than this…
const CHANNEL_TOLERANCE: u8 = 24;
/// …and the image fails when more than this fraction of pixels differ.
const MAX_DIFFERENT: f64 = 0.004;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn decode(bytes: &[u8]) -> RgbaImage {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().expect("valid png");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        other => panic!("unexpected golden colour type {other:?}"),
    };
    RgbaImage { width: info.width, height: info.height, pixels: rgba }
}

/// Compares `img` with the reference `name`.png (writing it if missing/updating).
fn check(name: &str, img: &RgbaImage) {
    let path = golden_dir().join(format!("{name}.png"));
    let update = std::env::var("XENIFY_UPDATE_GOLDEN").is_ok_and(|v| v == "1");
    if update || !path.exists() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, img.to_png()).unwrap();
        eprintln!("wrote golden image {}", path.display());
        return;
    }
    let want = decode(&std::fs::read(&path).unwrap());
    assert_eq!((want.width, want.height), (img.width, img.height), "{name}: size changed");
    let different = img
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want.pixels.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE))
        .count();
    let fraction = different as f64 / (img.width * img.height) as f64;
    if fraction > MAX_DIFFERENT {
        let actual = std::env::temp_dir().join(format!("xenify-golden-{name}.png"));
        std::fs::write(&actual, img.to_png()).unwrap();
        panic!(
            "{name}: {:.2}% of pixels differ from {} (limit {:.2}%). This render: {}. If the change is intended, \
             rerun with XENIFY_UPDATE_GOLDEN=1.",
            fraction * 100.0,
            path.display(),
            MAX_DIFFERENT * 100.0,
            actual.display()
        );
    }
}

fn gpu() -> Option<Offscreen> {
    match Offscreen::new() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping golden tests: {e}");
            None
        }
    }
}

fn keys<T: xenify_doc::Animatable>(k: Vec<(f64, T)>) -> Property<T> {
    Property::from_keyframes(
        k.into_iter().map(|(time, value)| Keyframe { time, value, easing: Easing::EASE_IN_OUT }).collect(),
    )
    .unwrap()
}

/// Gradients, trim paths, a track matte, text, a precomp and an expression.
fn features() -> (Project, Id) {
    let mut p = Project::new();
    let main = p.compositions[0].id;
    let mut layers = Vec::new();

    let mut ring = ShapeItem::ellipse(Vec2::new(300.0, 300.0), Color::WHITE);
    ring.fill = None;
    ring.stroke = Some(Stroke {
        cap_roundness: 0.0,
        style: Default::default(),
        color: Color::rgb(1.0, 0.6, 0.1).into(),
        width: 28.0.into(),
        join: Default::default(),
        cap: xenify_doc::LineCap::Round,
        ..Default::default()
    });
    ring.trim = Some(Trim { end: keys(vec![(0.0, 0.0), (1.0, 1.0)]), ..Default::default() });
    layers.push(Layer::shape(Id(101), "Ring", vec![ring]).with_position(Vec2::new(360.0, 540.0)));

    let mut card = ShapeItem::rect(Vec2::new(420.0, 260.0), Color::WHITE);
    let b = kurbo::Rect::new(-210.0, -130.0, 210.0, 130.0);
    card.fill = Some(Fill {
        color: Color::WHITE.into(),
        gradient: Some(
            Gradient::across(
                b,
                GradientKind::Linear,
                30.0,
                vec![
                    GradientStop { offset: 0.0, color: Color::rgb(1.0, 0.2, 0.4) },
                    GradientStop { offset: 1.0, color: Color::rgb(0.2, 0.3, 1.0) },
                ],
            )
            .into(),
        ),
        pattern: None,
    });
    let mut card_l = Layer::shape(Id(102), "Card", vec![card]).with_position(Vec2::new(960.0, 360.0));
    card_l.meta.expressions.push(Expression { path: PropPath::Rotation, source: "time * 20".into() });
    layers.push(card_l);

    // Text revealed by an alpha matte.
    layers.push(
        Layer::shape(Id(103), "Mask", vec![ShapeItem::rect(Vec2::new(900.0, 200.0), Color::WHITE)])
            .with_position(Vec2::new(1300.0, 800.0)),
    );
    let mut title = Layer::new(
        Id(104),
        "Title",
        LayerKind::Text(TextLayer::new("Golden", "Inter", 150.0, Color::WHITE).centered()),
    );
    title.transform.position = keys(vec![(0.0, Vec2::new(1300.0, 1000.0)), (1.0, Vec2::new(1300.0, 800.0))]);
    title.meta.matte = Some(Matte { layer: Id(103), mode: MatteMode::Alpha });
    layers.push(title);

    // A precomp of a radial-gradient ball, scaled down.
    let mut ball = ShapeItem::ellipse(Vec2::new(600.0, 600.0), Color::WHITE);
    ball.fill.as_mut().unwrap().gradient = Some(
        Gradient::across(
            kurbo::Rect::new(-300.0, -300.0, 300.0, 300.0),
            GradientKind::Radial,
            0.0,
            vec![
                GradientStop { offset: 0.0, color: Color::WHITE },
                GradientStop { offset: 1.0, color: Color::rgb(0.1, 0.5, 0.3) },
            ],
        )
        .into(),
    );
    let inner = Composition {
        id: Id(200),
        settings: xenify_doc::CompSettings {
            name: "Inner".into(),
            background: Color::TRANSPARENT,
            ..Default::default()
        },
        layers: vec![Layer::shape(Id(201), "Ball", vec![ball]).with_position(Vec2::new(960.0, 540.0))],
    };
    let mut pre = Layer::precomp(Id(105), "Inner", &inner, Vec2::new(1600.0, 250.0));
    pre.transform.scale = Vec2::new(0.25, 0.25).into();
    layers.push(pre);
    p.compositions.push(inner);

    let c = p.comp_mut(main).unwrap();
    c.layers = layers;
    c.settings.duration = 2.0;
    p.reserve_id(Id(300));
    (p, main)
}

#[test]
fn demo_frames_match_their_golden_images() {
    let Some(mut gpu) = gpu() else { return };
    let p = xenify_doc::demo_project();
    let main = p.compositions[0].id;
    for (name, t) in [("demo_0s", 0.0), ("demo_1s", 1.0), ("demo_2_5s", 2.5)] {
        let img = gpu.render(&evaluate_project(&p, main, t), W, H).unwrap();
        check(name, &img);
    }
}

#[test]
fn feature_frames_match_their_golden_images() {
    let Some(mut gpu) = gpu() else { return };
    let (p, main) = features();
    for (name, t) in [("features_0_5s", 0.5), ("features_1_5s", 1.5)] {
        let img = gpu.render(&evaluate_project(&p, main, t), W, H).unwrap();
        check(name, &img);
    }
}

#[test]
fn the_comparison_catches_real_changes() {
    let a = RgbaImage { width: 10, height: 10, pixels: vec![100; 400] };
    let mut b = a.clone();
    // A few pixels a tiny bit off: fine. Many pixels far off: detected.
    for px in b.pixels.as_chunks_mut::<4>().0.iter_mut().take(3) {
        px[0] = 110;
    }
    let off = |x: &RgbaImage, y: &RgbaImage| {
        x.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(y.pixels.as_chunks::<4>().0.iter())
            .filter(|(p, q)| p.iter().zip(*q).any(|(u, v)| u.abs_diff(*v) > CHANNEL_TOLERANCE))
            .count()
    };
    assert_eq!(off(&a, &b), 0);
    for px in b.pixels.as_chunks_mut::<4>().0.iter_mut().take(20) {
        px[1] = 250;
    }
    assert_eq!(off(&a, &b), 20);
}
