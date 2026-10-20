//! GPU rendering tests. Skipped (with a message) when no GPU adapter exists.

use xenify_anim::evaluate;
use xenify_doc::{Color, Composition, Id, Layer, Project, ShapeItem, Vec2};
use xenify_render::Offscreen;

fn offscreen() -> Option<Offscreen> {
    match Offscreen::new() {
        Ok(o) => Some(o),
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

fn comp(layers: Vec<Layer>) -> Composition {
    let mut c = Project::new().compositions.remove(0);
    c.settings.width = 100;
    c.settings.height = 50;
    c.settings.background = Color::rgb(0.0, 0.0, 1.0);
    c.layers = layers;
    c
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

#[test]
fn draws_background_shapes_and_opacity() {
    let Some(mut gpu) = offscreen() else { return };
    let red = Layer::shape(Id(10), "red", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::rgb(1.0, 0.0, 0.0))])
        .with_position(Vec2::new(25.0, 25.0));
    let mut half = Layer::shape(Id(11), "half", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::WHITE)])
        .with_position(Vec2::new(75.0, 25.0));
    half.opacity = 0.5.into();

    let img = gpu.render(&evaluate(&comp(vec![red, half]), 0.0), 100, 50).unwrap();
    assert_eq!(img.pixels.len(), 100 * 50 * 4);
    assert!(close(img.pixel(2, 2), [0, 0, 255, 255]), "background: {:?}", img.pixel(2, 2));
    assert!(close(img.pixel(25, 25), [255, 0, 0, 255]), "red rect: {:?}", img.pixel(25, 25));
    // 50% white over blue.
    assert!(close(img.pixel(75, 25), [128, 128, 255, 255]), "half white: {:?}", img.pixel(75, 25));
}

#[test]
fn scales_to_output_size() {
    let Some(mut gpu) = offscreen() else { return };
    let red = Layer::shape(Id(10), "red", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::rgb(1.0, 0.0, 0.0))])
        .with_position(Vec2::new(25.0, 25.0));
    // Composition is 100x50; render at 2x.
    let img = gpu.render(&evaluate(&comp(vec![red]), 0.0), 200, 100).unwrap();
    assert!(close(img.pixel(50, 50), [255, 0, 0, 255]));
    assert!(close(img.pixel(80, 50), [0, 0, 255, 255]));
    eprintln!("rendered on {}", gpu.adapter_name());
}

#[test]
fn draws_text_within_measured_bounds() {
    use xenify_anim::TextMeasure;
    use xenify_doc::{LayerKind, TextLayer};
    let Some(mut gpu) = offscreen() else { return };
    let mut c = comp(vec![
        Layer::new(Id(1), "title", LayerKind::Text(TextLayer::new("Hi", "Inter", 40.0, Color::WHITE)))
            .with_position(Vec2::new(10.0, 40.0)),
    ]);
    c.settings.background = Color::BLACK;
    let scene = evaluate(&c, 0.0);

    let bounds = xenify_render::TextEngine::new().text_bounds(match &scene.layers[0].content {
        xenify_anim::SceneContent::Text(t) => t,
        _ => unreachable!(),
    });
    // Sensible metrics: above the baseline, roughly 40px tall, ~2 glyphs wide.
    assert!(bounds.y0 < -25.0 && bounds.y0 > -45.0, "{bounds:?}");
    assert!(bounds.width() > 20.0 && bounds.width() < 80.0, "{bounds:?}");

    let img = gpu.render(&scene, 100, 50).unwrap();
    let lit = |x0: u32, x1: u32, y0: u32, y1: u32| {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img.pixel(x, y)[0] > 128).count()
    };
    // Glyph pixels exist inside the text box and none far to the right of it.
    let right = (10.0 + bounds.x1).ceil() as u32 + 2;
    assert!(lit(10, right.min(100), 10, 40) > 50, "no text pixels drawn");
    assert_eq!(lit(right.min(99), 100, 0, 50), 0, "pixels outside measured bounds");
}

#[test]
fn centred_text_is_centred_on_the_origin() {
    use xenify_anim::TextMeasure;
    let mut engine = xenify_render::TextEngine::new();
    let mut t = xenify_anim::SceneText {
        align: xenify_doc::TextAlign::Center,
        valign: xenify_doc::TextVAlign::Middle,
        ..xenify_anim::SceneText::new("Centre me\nplease", "Inter", 50.0, Color::WHITE)
    };
    let b = engine.text_bounds(&t);
    assert!(b.center().x.abs() < 0.5 && b.center().y.abs() < 0.5, "{b:?}");
    t.align = xenify_doc::TextAlign::Right;
    t.valign = xenify_doc::TextVAlign::Top;
    let r = engine.text_bounds(&t);
    assert!(r.x1.abs() < 0.5 && r.y0.abs() < 0.5, "{r:?}");
    // Changing the text keeps it centred (no stale anchor).
    t.text = "A much longer line of text".into();
    t.align = xenify_doc::TextAlign::Center;
    t.valign = xenify_doc::TextVAlign::Middle;
    assert!(engine.text_bounds(&t).center().x.abs() < 0.5);
}

#[test]
fn draws_linear_gradients() {
    use xenify_doc::{Gradient, GradientKind, GradientStop};
    let Some(mut gpu) = offscreen() else { return };
    let mut shape = ShapeItem::rect(Vec2::new(100.0, 50.0), Color::WHITE);
    let stops = vec![
        GradientStop { offset: 0.0, color: Color::rgb(1.0, 0.0, 0.0) },
        GradientStop { offset: 1.0, color: Color::rgb(0.0, 0.0, 1.0) },
    ];
    shape.fill.as_mut().unwrap().gradient =
        Some(Gradient::across(kurbo::Rect::new(-50.0, -25.0, 50.0, 25.0), GradientKind::Linear, 0.0, stops).into());
    let layer = Layer::shape(Id(1), "g", vec![shape]).with_position(Vec2::new(50.0, 25.0));
    let img = gpu.render(&evaluate(&comp(vec![layer]), 0.0), 100, 50).unwrap();
    let (l, r, m) = (img.pixel(2, 25), img.pixel(97, 25), img.pixel(50, 25));
    assert!(l[0] > 200 && l[2] < 40, "left is red: {l:?}");
    assert!(r[2] > 200 && r[0] < 40, "right is blue: {r:?}");
    assert!(m[0] > 60 && m[2] > 60, "middle is a blend: {m:?}");
}

#[test]
fn track_mattes_mask_by_alpha_and_luma() {
    use xenify_doc::{Matte, MatteMode};
    let Some(mut gpu) = offscreen() else { return };
    let red = || {
        Layer::shape(Id(10), "red", vec![ShapeItem::rect(Vec2::new(100.0, 50.0), Color::rgb(1.0, 0.0, 0.0))])
            .with_position(Vec2::new(50.0, 25.0))
    };
    // The matte covers the left square only; it is not drawn itself.
    let matte = |color| {
        Layer::shape(Id(11), "matte", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), color)])
            .with_position(Vec2::new(25.0, 25.0))
    };
    let (red_px, blue_px) = ([255, 0, 0, 255], [0, 0, 255, 255]);
    for (mode, color, inside, outside) in [
        (MatteMode::Alpha, Color::WHITE, red_px, blue_px),
        (MatteMode::AlphaInverted, Color::WHITE, blue_px, red_px),
        (MatteMode::Luma, Color::WHITE, red_px, blue_px),
        (MatteMode::Luma, Color::BLACK, blue_px, blue_px),
        (MatteMode::LumaInverted, Color::WHITE, blue_px, red_px),
    ] {
        let mut l = red();
        l.meta.matte = Some(Matte { layer: Id(11), mode });
        let img = gpu.render(&evaluate(&comp(vec![matte(color), l]), 0.0), 100, 50).unwrap();
        assert!(close(img.pixel(25, 25), inside), "{mode:?} inside: {:?}", img.pixel(25, 25));
        assert!(close(img.pixel(75, 25), outside), "{mode:?} outside: {:?}", img.pixel(75, 25));
    }
}

#[test]
fn precomps_render_clipped_to_their_frame() {
    let Some(mut gpu) = offscreen() else { return };
    let mut p = Project::new();
    let main = p.compositions[0].id;
    *p.comp_mut(main).unwrap() = Composition { id: main, ..comp(vec![]) };
    // A 40×40 composition whose red rectangle is much bigger than its frame.
    let mut inner = comp(vec![
        Layer::shape(Id(21), "big", vec![ShapeItem::rect(Vec2::new(400.0, 400.0), Color::rgb(1.0, 0.0, 0.0))])
            .with_position(Vec2::new(20.0, 20.0)),
    ]);
    inner.id = Id(20);
    (inner.settings.width, inner.settings.height) = (40, 40);
    p.compositions.push(inner.clone());
    p.comp_mut(main).unwrap().layers.push(Layer::precomp(Id(22), "pre", &inner, Vec2::new(25.0, 25.0)));
    let img = gpu.render(&xenify_anim::evaluate_project(&p, main, 0.0), 100, 50).unwrap();
    assert!(close(img.pixel(25, 25), [255, 0, 0, 255]), "inside: {:?}", img.pixel(25, 25));
    assert!(close(img.pixel(75, 25), [0, 0, 255, 255]), "clipped: {:?}", img.pixel(75, 25));
}

#[test]
fn text_outlines_match_the_laid_out_text() {
    use kurbo::Shape;
    let mut engine = xenify_render::TextEngine::new();
    let text = xenify_anim::SceneText {
        align: xenify_doc::TextAlign::Center,
        valign: xenify_doc::TextVAlign::Middle,
        ..xenify_anim::SceneText::new("Hello", "Inter", 100.0, Color::WHITE)
    };
    let path = engine.outline(&text);
    let ink = path.bounding_box();
    let layout = engine.layout(&text).bounds;
    assert!(ink.area() > 0.0 && path.elements().len() > 20);
    // Ink sits inside the line box (give or take side bearings) and is centred.
    assert!(layout.inflate(4.0, 4.0).contains_rect(ink), "{ink:?} vs {layout:?}");
    assert!((ink.center().x - layout.center().x).abs() < 6.0, "{ink:?}");
}

#[test]
fn motion_blur_smears_fast_motion() {
    let Some(mut gpu) = offscreen() else { return };
    let mut p = Project::new();
    let main = p.compositions[0].id;
    let mut square = Layer::shape(Id(30), "sq", vec![ShapeItem::rect(Vec2::new(10.0, 50.0), Color::WHITE)]);
    // 30 px per frame at 30 fps.
    square.transform.position = xenify_doc::Property::from_keyframes(vec![
        xenify_doc::Keyframe { time: 0.0, value: Vec2::new(0.0, 25.0), easing: xenify_doc::Easing::Linear },
        xenify_doc::Keyframe { time: 1.0, value: Vec2::new(900.0, 25.0), easing: xenify_doc::Easing::Linear },
    ])
    .unwrap();
    *p.comp_mut(main).unwrap() = Composition { id: main, ..comp(vec![square]) };
    let t = 1.5 / 30.0; // square centred at x = 45
    let sharp = gpu.render_frame(&p, main, t, 100, 50, false).unwrap();
    p.comp_mut(main).unwrap().settings.motion_blur = Some(xenify_doc::MotionBlur::default());
    let blurred = gpu.render_frame(&p, main, t, 100, 50, false).unwrap();
    // Sharp: fully white at the centre, pure background 10 px beside it.
    assert!(close(sharp.pixel(45, 25), [255, 255, 255, 255]));
    assert!(close(sharp.pixel(56, 25), [0, 0, 255, 255]));
    // Blurred over a 15 px shutter: the edge area is a mix of white and blue.
    let edge = blurred.pixel(52, 25);
    assert!(edge[0] > 40 && edge[0] < 215, "partly covered: {edge:?}");
}

#[test]
fn draws_image_layers() {
    let Some(mut gpu) = offscreen() else { return };
    // A 2×1 PNG: red then green, drawn 20× larger (40×20) centred at (50, 25).
    let png = xenify_render::RgbaImage { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 255, 0, 255] }.to_png();
    let mut project = Project::new();
    let asset = xenify_doc::ImageAsset::new(project.alloc_id(), "rg", "image/png", 2, 1, png);
    let id = project.alloc_id();
    let mut layer = Layer::image(id, "rg", &asset, Vec2::new(50.0, 25.0));
    layer.transform.scale = Vec2::new(20.0, 20.0).into();
    project.assets.push(asset);
    let c = project.compositions[0].id;
    let comp = project.comp_mut(c).unwrap();
    comp.settings.width = 100;
    comp.settings.height = 50;
    comp.settings.background = Color::rgb(0.0, 0.0, 1.0);
    comp.layers.push(layer);

    let img = gpu.render_frame(&project, c, 0.0, 100, 50, false).unwrap();
    assert!(close(img.pixel(5, 25), [0, 0, 255, 255]), "background: {:?}", img.pixel(5, 25));
    assert!(close(img.pixel(35, 25), [255, 0, 0, 255]), "left pixel: {:?}", img.pixel(35, 25));
    assert!(close(img.pixel(65, 25), [0, 255, 0, 255]), "right pixel: {:?}", img.pixel(65, 25));

    // Adjust Colours applies to the picture's pixels: fully desaturated, it's grey.
    let layer = &mut project.comp_mut(c).unwrap().layers[0];
    layer.meta.effects = vec![xenify_doc::Effect::new(xenify_doc::EffectKind::AdjustColors {
        brightness: 0.0.into(),
        contrast: 0.0.into(),
        saturation: (-1.0).into(),
        hue: 0.0.into(),
    })];
    let img = gpu.render_frame(&project, c, 0.0, 100, 50, false).unwrap();
    let [r, g, b, _] = img.pixel(35, 25);
    assert!(r.abs_diff(g) <= 2 && g.abs_diff(b) <= 2 && r > 20 && r < 120, "grey red: {:?}", img.pixel(35, 25));
    let [r, g, b, _] = img.pixel(65, 25);
    assert!(r.abs_diff(g) <= 2 && g.abs_diff(b) <= 2 && r > 120, "grey green (brighter): {:?}", img.pixel(65, 25));
}

#[test]
fn deck_transitions_mix_and_move_slides() {
    use xenify_doc::{CompSettings, Slide, Transition, TransitionKind};
    let Some(mut gpu) = offscreen() else { return };
    let mut p = Project::new();
    let a = p.compositions[0].id;
    let s = &mut p.compositions[0].settings;
    (s.width, s.height, s.duration, s.background) = (100, 50, 1.0, Color::rgb(1.0, 0.0, 0.0));
    let b = p.alloc_id();
    p.compositions.push(Composition {
        id: b,
        settings: CompSettings {
            width: 100,
            height: 50,
            duration: 1.0,
            background: Color::rgb(0.0, 0.0, 1.0),
            ..Default::default()
        },
        layers: vec![],
    });
    let mut second = Slide::new(b);
    second.transition_in = Transition::new(TransitionKind::Fade, 1.0);
    p.slides = vec![Slide::new(a), second];

    let deck = xenify_anim::deck::Deck::new(&p);
    let mid = gpu.render_deck(&p, &deck.at(1.5).unwrap(), 100, 50).unwrap();
    let px = mid.pixel(50, 25);
    assert!(px[0].abs_diff(128) < 20 && px[2].abs_diff(128) < 20 && px[1] < 10, "half red, half blue: {px:?}");

    p.slides[1].transition_in = Transition::new(TransitionKind::Push, 1.0);
    let deck = xenify_anim::deck::Deck::new(&p);
    let img = gpu.render_deck(&p, &deck.at(1.5).unwrap(), 100, 50).unwrap();
    assert!(close(img.pixel(20, 25), [255, 0, 0, 255]), "left: the old slide {:?}", img.pixel(20, 25));
    assert!(close(img.pixel(80, 25), [0, 0, 255, 255]), "right: the new slide {:?}", img.pixel(80, 25));

    // A wider target letterboxes the slide on black.
    let wide = gpu.render_deck(&p, &deck.at(0.2).unwrap(), 200, 50).unwrap();
    assert!(close(wide.pixel(10, 25), [0, 0, 0, 255]) && close(wide.pixel(100, 25), [255, 0, 0, 255]));
}

#[test]
fn spacing_widens_text_and_text_follows_a_path() {
    use xenify_anim::TextMeasure;
    use kurbo::Shape;
    let mut engine = xenify_render::TextEngine::new();
    let plain = xenify_anim::SceneText::new("Hello world", "Inter", 40.0, Color::WHITE);
    let w0 = engine.text_bounds(&plain).width();
    let spaced = xenify_anim::SceneText { letter_spacing: 10.0, ..plain.clone() };
    let w1 = engine.text_bounds(&spaced).width();
    assert!((w1 - w0 - 10.0 * 10.0).abs() < 12.0, "10 px between 11 letters: {w0} → {w1}");
    let words = xenify_anim::SceneText { word_spacing: 50.0, ..plain.clone() };
    assert!(engine.text_bounds(&words).width() > w0 + 40.0);

    // Around a circle of radius 200: every glyph's ink stays near the circle.
    let circle = kurbo::Circle::new((0.0, 0.0), 200.0).to_path(0.1);
    let on_path = xenify_anim::SceneText {
        path: Some(xenify_anim::TextOnPath { path: circle, offset: 0.0 }),
        ..xenify_anim::SceneText::new("AROUND AND AROUND AND AROUND", "Inter", 40.0, Color::WHITE)
    };
    let outline = engine.outline(&on_path);
    let mut n = 0;
    for el in outline.elements() {
        if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
            let r = p.to_vec2().hypot();
            assert!((150.0..=260.0).contains(&r), "ink {r:.0} px from the centre");
            n += 1;
        }
    }
    assert!(n > 50);
    let b = engine.text_bounds(&on_path);
    assert!(b.width() > 300.0 && b.height() > 150.0, "wraps around: {b:?}");
    // An open path shorter than the text drops what doesn't fit.
    let short = kurbo::Line::new((0.0, 0.0), (60.0, 0.0)).to_path(0.1);
    let clipped = xenify_anim::SceneText { path: Some(xenify_anim::TextOnPath { path: short, offset: 0.0 }), ..plain };
    assert!(engine.text_bounds(&clipped).width() < 90.0);
    let _ = outline.bounding_box();
}

#[test]
fn effects_shadow_blur_glow_blend_and_colour() {
    use xenify_doc::{BlendMode, Effect, EffectKind};
    let Some(mut gpu) = offscreen() else { return };
    // A white 40×40 square at (50, 25) on blue, 100×50 frame (drawn at 4× for detail).
    let square = |effects: Vec<EffectKind>, blend: BlendMode, fill: Color| {
        let mut l = Layer::shape(Id(10), "sq", vec![ShapeItem::rect(Vec2::new(40.0, 40.0), fill)])
            .with_position(Vec2::new(50.0, 25.0));
        l.meta.effects = effects.into_iter().map(Effect::new).collect();
        l.meta.blend = blend;
        l
    };
    let render = |gpu: &mut xenify_render::Offscreen, layers: Vec<Layer>| {
        gpu.render(&evaluate(&comp(layers), 0.0), 400, 200).unwrap()
    };
    let px = |img: &xenify_render::RgbaImage, x: f64, y: f64| img.pixel((x * 4.0) as u32, (y * 4.0) as u32);

    // Shadow: black, offset (10, 0), no blur: dark just right of the square.
    let shadow =
        EffectKind::DropShadow { color: Color::BLACK.into(), offset: Vec2::new(10.0, 0.0).into(), blur: 0.0.into() };
    let img = render(&mut gpu, vec![square(vec![shadow], BlendMode::Normal, Color::WHITE)]);
    assert!(close(px(&img, 75.0, 25.0), [0, 0, 0, 255]), "shadow right: {:?}", px(&img, 75.0, 25.0));
    assert!(close(px(&img, 50.0, 25.0), [255, 255, 255, 255]), "square on top");
    assert!(close(px(&img, 25.0, 25.0), [0, 0, 255, 255]), "nothing left");

    // Blurred shadow: a soft edge (in between at the square's right edge + offset).
    let soft =
        EffectKind::DropShadow { color: Color::BLACK.into(), offset: Vec2::new(10.0, 0.0).into(), blur: 8.0.into() };
    let img = render(&mut gpu, vec![square(vec![soft], BlendMode::Normal, Color::WHITE)]);
    let edge = px(&img, 80.0, 25.0)[2];
    assert!((60..200).contains(&edge), "half-dark at the shadow's edge: {edge}");

    // Blur on the layer itself: its edge becomes a ramp.
    let img =
        render(&mut gpu, vec![square(vec![EffectKind::Blur { radius: 6.0.into() }], BlendMode::Normal, Color::WHITE)]);
    let r = px(&img, 70.0, 25.0)[0];
    assert!((60..200).contains(&r), "half-white at the edge: {r}");
    assert!(px(&img, 50.0, 25.0)[0] > 240, "solid in the middle");

    // Glow: colour spreads around.
    let glow = EffectKind::Glow { color: Color::rgb(1.0, 0.0, 0.0).into(), radius: 10.0.into() };
    let img = render(&mut gpu, vec![square(vec![glow], BlendMode::Normal, Color::WHITE)]);
    assert!(px(&img, 72.0, 25.0)[0] > 40, "red halo: {:?}", px(&img, 72.0, 25.0));

    // Multiply: yellow × blue = black.
    let img = render(&mut gpu, vec![square(vec![], BlendMode::Multiply, Color::rgb(1.0, 1.0, 0.0))]);
    assert!(close(px(&img, 50.0, 25.0), [0, 0, 0, 255]), "multiply: {:?}", px(&img, 50.0, 25.0));

    // Adjust colours: red turned to green.
    let mut l = square(vec![], BlendMode::Normal, Color::rgb(1.0, 0.0, 0.0));
    l.meta.effects = vec![Effect::new(EffectKind::AdjustColors {
        brightness: 0.0.into(),
        contrast: 0.0.into(),
        saturation: 0.0.into(),
        hue: 120.0.into(),
    })];
    let img = render(&mut gpu, vec![l]);
    let p = px(&img, 50.0, 25.0);
    assert!(p[1] > p[0] && p[1] > p[2], "hue turned: {p:?}");
}

#[test]
fn group_opacity_and_shadow_apply_to_the_group_as_a_whole() {
    use xenify_doc::{Effect, EffectKind, LayerKind};
    let Some(mut gpu) = offscreen() else { return };
    // Two overlapping white squares in a 50% group, on blue.
    let mut group = Layer::new(Id(1), "g", LayerKind::Group);
    group.opacity = 0.5.into();
    let mut a = Layer::shape(Id(2), "a", vec![ShapeItem::rect(Vec2::new(40.0, 40.0), Color::WHITE)])
        .with_position(Vec2::new(40.0, 25.0));
    a.meta.parent = Some(Id(1));
    let mut b = Layer::shape(Id(3), "b", vec![ShapeItem::rect(Vec2::new(40.0, 40.0), Color::WHITE)])
        .with_position(Vec2::new(60.0, 25.0));
    b.meta.parent = Some(Id(1));
    let img = gpu.render(&evaluate(&comp(vec![a.clone(), b.clone(), group.clone()]), 0.0), 100, 50).unwrap();
    // Where they overlap it's no brighter than where there's one: one picture at 50%.
    assert!(close(img.pixel(50, 25), img.pixel(30, 25)), "{:?} vs {:?}", img.pixel(50, 25), img.pixel(30, 25));
    assert!(close(img.pixel(50, 25), [128, 128, 255, 255]));

    // A shadow on the group: one shadow for both squares, 10 px lower.
    group.opacity = 1.0.into();
    group.meta.effects = vec![Effect::new(EffectKind::DropShadow {
        color: Color::BLACK.into(),
        offset: Vec2::new(0.0, 10.0).into(),
        blur: 0.0.into(),
    })];
    let img = gpu.render(&evaluate(&comp(vec![a, b, group]), 0.0), 100, 50).unwrap();
    assert!(close(img.pixel(50, 48), [0, 0, 0, 255]), "shadow below: {:?}", img.pixel(50, 48));
    assert!(close(img.pixel(50, 25), [255, 255, 255, 255]));
}

#[test]
fn masks_crop_combine_invert_and_feather() {
    use xenify_doc::{Mask, MaskMode, kurbo::Rect};
    let Some(mut gpu) = offscreen() else { return };
    // A red rectangle filling the frame; its layer space runs −50..50 × −25..25.
    let red = |masks: Vec<Mask>| {
        let mut l =
            Layer::shape(Id(10), "red", vec![ShapeItem::rect(Vec2::new(100.0, 50.0), Color::rgb(1.0, 0.0, 0.0))])
                .with_position(Vec2::new(50.0, 25.0));
        l.meta.masks = masks;
        l
    };
    let left = Rect::new(-50.0, -25.0, 0.0, 25.0); // the left half
    let middle = Rect::new(-10.0, -10.0, 10.0, 10.0);
    let (red_px, blue_px) = ([255, 0, 0, 255], [0, 0, 255, 255]);
    let render = |gpu: &mut Offscreen, masks| gpu.render(&evaluate(&comp(vec![red(masks)]), 0.0), 100, 50).unwrap();

    // Crop to the left half.
    let img = render(&mut gpu, vec![Mask::rect("crop", left)]);
    assert!(close(img.pixel(25, 25), red_px) && close(img.pixel(75, 25), blue_px));
    // Minus a hole in the middle.
    let hole = Mask { mode: MaskMode::Subtract, ..Mask::rect("hole", middle) };
    let img = render(&mut gpu, vec![Mask::rect("all", Rect::new(-50.0, -25.0, 50.0, 25.0)), hole.clone()]);
    assert!(close(img.pixel(50, 25), blue_px) && close(img.pixel(80, 25), red_px), "{:?}", img.pixel(50, 25));
    // A first subtract mask takes it from the whole layer.
    let img = render(&mut gpu, vec![hole]);
    assert!(close(img.pixel(50, 25), blue_px) && close(img.pixel(80, 25), red_px));
    // Intersect: only where both show.
    let img = render(
        &mut gpu,
        vec![Mask::rect("left", left), Mask { mode: MaskMode::Intersect, ..Mask::rect("mid", middle) }],
    );
    assert!(close(img.pixel(45, 25), red_px) && close(img.pixel(55, 25), blue_px) && close(img.pixel(20, 25), blue_px));
    // Inverted: everything but the middle.
    let img = render(&mut gpu, vec![Mask { inverted: true, ..Mask::rect("not mid", middle) }]);
    assert!(close(img.pixel(50, 25), blue_px) && close(img.pixel(5, 5), red_px));
    // Half strength.
    let img = render(&mut gpu, vec![Mask { opacity: 0.5.into(), ..Mask::rect("half", left) }]);
    assert!(close(img.pixel(25, 25), [128, 0, 128, 255]), "{:?}", img.pixel(25, 25));
    // Feathered: soft across the edge, solid well inside.
    let img = render(&mut gpu, vec![Mask { feather: 8.0.into(), ..Mask::rect("soft", left) }]);
    let edge = img.pixel(50, 25);
    assert!(edge[0] > 60 && edge[0] < 200, "half-way at the edge: {edge:?}");
    assert!(close(img.pixel(20, 25), red_px), "{:?}", img.pixel(20, 25));
    // Disabled masks don't apply.
    let img = render(&mut gpu, vec![Mask { enabled: false, ..Mask::rect("off", left) }]);
    assert!(close(img.pixel(75, 25), red_px));
}

#[test]
fn dashed_and_gradient_strokes() {
    use xenify_doc::{Gradient, GradientKind, GradientStop, Stroke};
    let Some(mut gpu) = offscreen() else { return };
    // A horizontal line across the frame at y = 25, 10 px thick.
    let line = |stroke: Stroke| {
        let mut path = xenify_doc::BezPath::new();
        path.move_to((0.0, 25.0));
        path.line_to((100.0, 25.0));
        let mut s = ShapeItem::path(path, None, None);
        s.stroke = Some(stroke);
        Layer::shape(Id(1), "line", vec![s])
    };
    let red = Stroke { color: Color::rgb(1.0, 0.0, 0.0).into(), width: 10.0.into(), ..Default::default() };
    // Dashes of 10 with gaps of 10: red at x = 5, background at x = 15.
    let dashed = Stroke { dashes: vec![10.0, 10.0], ..red.clone() };
    let img = gpu.render(&evaluate(&comp(vec![line(dashed.clone())]), 0.0), 100, 50).unwrap();
    assert!(
        close(img.pixel(5, 25), [255, 0, 0, 255]) && close(img.pixel(15, 25), [0, 0, 255, 255]),
        "{:?} {:?}",
        img.pixel(5, 25),
        img.pixel(15, 25)
    );
    // Offsetting by 10 swaps them.
    let shifted = Stroke { dash_offset: 10.0.into(), ..dashed };
    let img = gpu.render(&evaluate(&comp(vec![line(shifted)]), 0.0), 100, 50).unwrap();
    assert!(close(img.pixel(5, 25), [0, 0, 255, 255]) && close(img.pixel(15, 25), [255, 0, 0, 255]));
    // A black-to-white gradient along the line.
    let g = Gradient {
        kind: GradientKind::Linear,
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
        stops: vec![
            GradientStop { offset: 0.0, color: Color::BLACK },
            GradientStop { offset: 1.0, color: Color::WHITE },
        ],
    };
    let img =
        gpu.render(&evaluate(&comp(vec![line(Stroke { gradient: Some(g.into()), ..red })]), 0.0), 100, 50).unwrap();
    let (l, r) = (img.pixel(10, 25), img.pixel(90, 25));
    assert!(l[0] < 60 && r[0] > 200 && l[0] == l[1], "dark to light: {l:?} {r:?}");
}

#[test]
fn pattern_fills_repeat_a_picture() {
    use xenify_doc::{Fill, Pattern};
    let Some(mut gpu) = offscreen() else { return };
    // A 2×1 tile: red then green, each pixel 10 units wide once scaled ×10.
    let png = xenify_render::RgbaImage { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 255, 0, 255] }.to_png();
    let mut project = Project::new();
    let asset = xenify_doc::ImageAsset::new(project.alloc_id(), "tile", "image/png", 2, 1, png);
    let id = project.alloc_id();
    let mut shape = ShapeItem::rect(Vec2::new(100.0, 50.0), Color::WHITE);
    shape.fill = Some(Fill {
        pattern: Some(Pattern { scale: 10.0, offset: Vec2::new(-50.0, -25.0), ..Pattern::new(asset.id) }),
        ..Fill::solid(Color::WHITE)
    });
    let layer = Layer::shape(id, "patterned", vec![shape]).with_position(Vec2::new(50.0, 25.0));
    project.assets.push(asset.clone());
    let c = project.compositions[0].id;
    let comp = project.comp_mut(c).unwrap();
    comp.settings.width = 100;
    comp.settings.height = 50;
    comp.layers.push(layer);
    let img = gpu.render_frame(&project, c, 0.0, 100, 50, false).unwrap();
    // Tiles repeat every 20 px: red, green, red, green…
    // (Smooth filtering blends a little across tile edges.)
    for (x, red) in [(5, true), (15, false), (25, true), (95, false)] {
        let [r, g, _, _] = img.pixel(x, 25);
        assert!(if red { r > 200 && g < 50 } else { g > 200 && r < 50 }, "x = {x}: {:?}", img.pixel(x, 25));
    }
    // Saving keeps the picture (it's in use by the pattern).
    let back = Project::from_json(&project.to_json()).unwrap();
    assert_eq!(back.assets.len(), 1);
}

#[test]
fn box_text_wraps_and_aligns_inside_its_box() {
    use xenify_anim::TextMeasure;
    let mut engine = xenify_render::TextEngine::new();
    let rect = kurbo::Rect::new(10.0, 20.0, 310.0, 420.0);
    let words = "The quick brown fox jumps over the lazy dog";
    let boxed = xenify_anim::SceneText {
        frame: Some(xenify_doc::TextFrame::new(rect)),
        valign: xenify_doc::TextVAlign::Top,
        ..xenify_anim::SceneText::new(words, "Inter", 40.0, Color::WHITE)
    };
    let lines = engine.text_lines(&boxed);
    assert!(lines.len() >= 3, "wrapped into lines: {lines:?}");
    assert_eq!(lines.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" "), words);
    for l in &lines {
        assert!(l.x0 >= rect.x0 - 0.5 && l.x1 <= rect.x1 + 0.5, "inside the box: {l:?}");
    }
    assert!((lines[0].x0 - rect.x0).abs() < 0.5, "left aligned to the box edge");
    assert!(lines[0].baseline > rect.y0 && lines[0].baseline < rect.y0 + 45.0, "first line at the top");
    assert_eq!(engine.text_bounds(&boxed), rect, "the box is what's picked");

    // Centred horizontally and at the bottom.
    let centred = xenify_anim::SceneText {
        align: xenify_doc::TextAlign::Center,
        valign: xenify_doc::TextVAlign::Bottom,
        ..boxed.clone()
    };
    for l in engine.text_lines(&centred) {
        assert!(((l.x0 + l.x1) / 2.0 - rect.center().x).abs() < 1.0, "centred: {l:?}");
    }
    let last = engine.text_lines(&centred).last().unwrap().baseline;
    assert!(last < rect.y1 && last > rect.y1 - 30.0, "last line at the bottom: {last}");

    // Without wrapping it's one long line spilling out of the box...
    let mut f = xenify_doc::TextFrame::new(rect);
    f.wrap = false;
    let long = xenify_anim::SceneText { frame: Some(f), ..boxed.clone() };
    assert_eq!(engine.text_lines(&long).len(), 1);
    assert!(engine.text_bounds(&long).x1 > rect.x1);
    // ...unless the overflow is hidden: then outlines stop at the box.
    f.clip = true;
    let cropped = xenify_anim::SceneText { frame: Some(f), ..boxed.clone() };
    assert_eq!(engine.text_bounds(&cropped), rect);
    use kurbo::Shape;
    let ink = engine.outline(&cropped).bounding_box();
    assert!(ink.x1 <= rect.x1 + 30.0, "{ink:?}");
}

#[test]
fn hidden_overflow_crops_box_text_when_drawn() {
    use xenify_doc::{LayerKind, TextLayer};
    let Some(mut gpu) = offscreen() else { return };
    // A long unwrapped line in a narrow box: nothing is drawn right of it.
    let mut f = xenify_doc::TextFrame::new(kurbo::Rect::new(0.0, 0.0, 40.0, 40.0));
    f.wrap = false;
    f.clip = true;
    let mut t = TextLayer::new("WWWWWWWW", "Inter", 30.0, Color::WHITE);
    t.valign = xenify_doc::TextVAlign::Top;
    t.frame = Some(f);
    let mut c = comp(vec![Layer::new(Id(1), "t", LayerKind::Text(t)).with_position(Vec2::new(5.0, 5.0))]);
    c.settings.background = Color::BLACK;
    let img = gpu.render(&evaluate(&c, 0.0), 100, 50).unwrap();
    let lit =
        |x0: u32, x1: u32| (x0..x1).flat_map(|x| (0..50).map(move |y| (x, y))).any(|(x, y)| img.pixel(x, y)[0] > 128);
    assert!(lit(5, 45), "text inside the box");
    assert!(!lit(47, 100), "nothing past the box");
}

#[test]
fn draws_what_the_camera_sees() {
    let Some(mut gpu) = offscreen() else { return };
    // A red square at (25, 25); a camera looking at it, zoomed 2×.
    let red = Layer::shape(Id(10), "red", vec![ShapeItem::rect(Vec2::new(20.0, 20.0), Color::rgb(1.0, 0.0, 0.0))])
        .with_position(Vec2::new(25.0, 25.0));
    let mut cam = xenify_doc::camera_layer(Id(11), 100.0, 50.0).with_position(Vec2::new(25.0, 25.0));
    if let xenify_doc::LayerKind::Camera(c) = &mut cam.kind {
        c.zoom = 2.0.into();
    }
    let img = gpu.render(&evaluate(&comp(vec![red, cam]), 0.0), 100, 50).unwrap();
    // The square is now centred and 40 px wide: 30..70 across.
    assert!(close(img.pixel(50, 25), [255, 0, 0, 255]), "centre: {:?}", img.pixel(50, 25));
    assert!(close(img.pixel(33, 25), [255, 0, 0, 255]), "left part: {:?}", img.pixel(33, 25));
    assert!(close(img.pixel(25, 25), [0, 0, 255, 255]), "outside it: {:?}", img.pixel(25, 25));
    // The background still fills the frame (it isn't moved by the camera).
    assert!(close(img.pixel(1, 1), [0, 0, 255, 255]));
}

#[test]
fn text_animators_bring_letters_in_one_by_one() {
    let Some(mut gpu) = offscreen() else { return };
    let mut text = xenify_doc::TextLayer::new("HHHHHH", "Inter", 30.0, Color::WHITE).centered();
    text.animators
        .push(xenify_doc::TextAnimator { stagger: 1.0, ..xenify_doc::TextAnimator::preset("Typewriter").unwrap() });
    let layer = Layer::new(Id(10), "t", xenify_doc::LayerKind::Text(text)).with_position(Vec2::new(50.0, 25.0));
    let c = comp(vec![layer]);
    let ink = |t: f64, gpu: &mut Offscreen| {
        let img = gpu.render(&evaluate(&c, t), 100, 50).unwrap();
        img.pixels.chunks(4).filter(|p| p[0] > 128).count()
    };
    let (one, three, all) = (ink(0.5, &mut gpu), ink(2.5, &mut gpu), ink(10.0, &mut gpu));
    assert!(one > 0, "the first letter is typed");
    assert!(one * 2 < three && three < all, "then more: {one} < {three} < {all}");
}

/// Every particle preset draws. `XENIFY_DUMP=<dir>` also writes the frames as
/// PNGs to look at.
#[test]
fn particle_presets_draw() {
    let Some(mut gpu) = offscreen() else { return };
    let dump = std::env::var("XENIFY_DUMP").ok();
    for name in xenify_doc::PARTICLE_PRESETS {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let (w, h) = (p.compositions[0].settings.width, p.compositions[0].settings.height);
        p.compositions[0].settings.background = Color::rgb(0.05, 0.05, 0.1);
        let logo = Layer::shape(
            Id(1),
            "Logo",
            vec![ShapeItem::rect(Vec2::new(300.0, 300.0), Color::rgba(1.0, 1.0, 1.0, 0.0))],
        )
        .with_position(Vec2::new(w as f64 / 2.0, h as f64 / 2.0));
        p.compositions[0].layers.push(logo);
        p.reserve_id(Id(1));
        let (mut e, at) = xenify_doc::Emitter::preset(name, w as f64, h as f64).unwrap();
        if *name == "Assemble into Shape" {
            e.end = xenify_doc::EndLook::Form { target: Id(1) };
        }
        let layer = Layer::new(Id(2), *name, xenify_doc::LayerKind::Particles(Box::new(e))).with_position(at);
        p.compositions[0].layers.push(layer);
        for t in [0.4, 1.2, 3.0] {
            let scene = xenify_anim::evaluate_project(&p, comp, t);
            let img = gpu.render(&scene, w / 2, h / 2).unwrap();
            let ink = img.pixels.chunks(4).filter(|px| px[0] > 60 || px[1] > 60).count();
            if t == 1.2 {
                assert!(ink > 0, "{name} draws something at {t}s");
            }
            if let Some(dir) = &dump {
                let file = format!("{dir}/{}-{t}.png", name.replace(' ', "_"));
                std::fs::write(file, img.to_png()).unwrap();
            }
        }
    }
}

/// Two plain slides (red, then blue) with `tr` between them, drawn half way through.
fn half_way(gpu: &mut Offscreen, tr: xenify_doc::Transition) -> xenify_render::RgbaImage {
    use xenify_doc::{CompSettings, Composition, Slide};
    let mut p = Project::new();
    let a = p.compositions[0].id;
    p.compositions[0].settings = CompSettings {
        width: 160,
        height: 90,
        duration: 1.0,
        background: Color::rgb(1.0, 0.0, 0.0),
        ..Default::default()
    };
    let b = p.alloc_id();
    p.compositions.push(Composition {
        id: b,
        settings: CompSettings { background: Color::rgb(0.0, 0.0, 1.0), ..p.compositions[0].settings.clone() },
        layers: vec![],
    });
    let mut second = Slide::new(b);
    second.transition_in = tr;
    p.slides = vec![Slide::new(a), second];
    let deck = xenify_anim::deck::Deck::new(&p);
    gpu.render_deck(&p, &deck.at(1.0 + tr.duration / 2.0).unwrap(), 160, 90).unwrap()
}

#[test]
fn every_transition_draws_half_way() {
    use xenify_doc::{Direction, Easing, Transition, TransitionKind};
    let Some(mut gpu) = offscreen() else { return };
    let linear = |kind| Transition { easing: Easing::Linear, ..Transition::new(kind, 1.0) };
    for kind in TransitionKind::ALL {
        let img = half_way(&mut gpu, linear(kind));
        let px = |x, y| img.pixel(x, y);
        let blue_or_red = |p: [u8; 4]| p[0] > 100 || p[2] > 100;
        // (Dip to Colour is all colour half way; a flip is edge on.)
        let exempt = matches!(kind, TransitionKind::DipToColor | TransitionKind::Flip);
        assert!(blue_or_red(px(80, 45)) || exempt, "{kind:?} shows a slide: {:?}", px(80, 45));
    }
    // A wipe to the right: the new slide on the left half, the old on the right.
    let img = half_way(&mut gpu, Transition { direction: Direction::Right, ..linear(TransitionKind::Wipe) });
    assert!(close(img.pixel(20, 45), [0, 0, 255, 255]) && close(img.pixel(140, 45), [255, 0, 0, 255]));
    // The iris: new in the middle, old in the corners.
    let img = half_way(&mut gpu, linear(TransitionKind::Iris));
    assert!(close(img.pixel(80, 45), [0, 0, 255, 255]) && close(img.pixel(2, 2), [255, 0, 0, 255]));
    // Dip to colour: all white at the middle.
    let white = Transition { color: Color::WHITE, ..linear(TransitionKind::DipToColor) };
    let img = half_way(&mut gpu, white);
    assert!(close(img.pixel(80, 45), [255, 255, 255, 255]), "{:?}", img.pixel(80, 45));
}

/// `XENIFY_DUMP=<dir>`: every transition a quarter, half and three quarters
/// of the way between two slides with things on them (to look at).
#[test]
fn dump_transitions() {
    use xenify_doc::{CompSettings, Composition, Easing, LayerKind, Slide, TextLayer, Transition, TransitionKind};
    let Ok(dir) = std::env::var("XENIFY_DUMP") else { return };
    let Some(mut gpu) = offscreen() else { return };
    let slide = |bg: Color, title: &str, x: f64| {
        let text = TextLayer::new(title, "Inter", 60.0, Color::WHITE).centered();
        let mut t = Layer::new(Id(1), "Title", LayerKind::Text(text)).with_position(Vec2::new(320.0, 90.0));
        t.id = Id(1);
        let logo =
            Layer::shape(Id(2), "Logo", vec![ShapeItem::ellipse(Vec2::new(120.0, 120.0), Color::rgb(1.0, 0.8, 0.2))])
                .with_position(Vec2::new(x, 240.0));
        (bg, vec![t, logo])
    };
    for kind in TransitionKind::ALL {
        let mut p = Project::new();
        let a = p.compositions[0].id;
        let settings = CompSettings { width: 640, height: 360, duration: 1.0, ..Default::default() };
        let (bg, layers) = slide(Color::rgb(0.1, 0.2, 0.5), "Before", 160.0);
        p.compositions[0].settings = CompSettings { background: bg, ..settings.clone() };
        p.compositions[0].layers = layers;
        let b = p.alloc_id();
        let (bg, mut layers) = slide(Color::rgb(0.5, 0.1, 0.3), "After", 480.0);
        for (i, l) in layers.iter_mut().enumerate() {
            l.id = Id(100 + i as u64);
        }
        p.compositions.push(Composition { id: b, settings: CompSettings { background: bg, ..settings }, layers });
        let mut second = Slide::new(b);
        second.transition_in = Transition { easing: Easing::Linear, ..Transition::new(kind, 1.0) };
        p.slides = vec![Slide::new(a), second];
        let deck = xenify_anim::deck::Deck::new(&p);
        for q in [0.25, 0.5, 0.75] {
            let img = gpu.render_deck(&p, &deck.at(1.0 + q).unwrap(), 320, 180).unwrap();
            std::fs::write(
                format!("{dir}/{:02}-{}-{q}.png", kind as usize, kind.label().replace(' ', "_")),
                img.to_png(),
            )
            .unwrap();
        }
    }
}
