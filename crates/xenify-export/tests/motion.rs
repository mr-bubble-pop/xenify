//! Cameras and text animators in every export: the picture is what the camera
//! sees, and animated letters are written glyph by glyph.

use xenify_doc::{CameraField, Color, Id, Keyframe, Layer, LayerKind, Project, PropPath, Property, ShapeItem, Vec2};
use xenify_export::svg::{AnimatedSvgOptions, export_animated_svg, export_svg};

/// A red square at (100, 100) and a camera looking at it, zooming 1 → 2 over a second.
fn project() -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    c.settings.width = 400;
    c.settings.height = 200;
    c.settings.duration = 1.0;
    c.settings.fps = 10.0;
    let square = Layer::shape(Id(1), "Square", vec![ShapeItem::rect(Vec2::new(40.0, 40.0), Color::rgb(1.0, 0.0, 0.0))])
        .with_position(Vec2::new(100.0, 100.0));
    let mut cam = xenify_doc::camera_layer(Id(2), 400.0, 200.0).with_position(Vec2::new(100.0, 100.0));
    if let LayerKind::Camera(k) = &mut cam.kind {
        let key = |time, value| Keyframe { time, value, easing: Default::default() };
        k.zoom = Property::from_keyframes(vec![key(0.0, 1.0), key(1.0, 2.0)]).unwrap();
    }
    c.layers = vec![square, cam];
    p.reserve_id(Id(2));
    (p, comp)
}

#[test]
fn still_svg_wraps_the_layers_in_the_camera_view() {
    let (p, comp) = project();
    let scene = xenify_anim::evaluate_project(&p, comp, 1.0);
    let svg = export_svg(&scene, &mut xenify_anim::ApproxText);
    // Zoom 2 about (100, 100), which lands in the middle of the 400×200 frame.
    assert!(svg.contains(r#"transform="matrix(2 0 0 2 0 -100)""#), "{svg}");
}

#[test]
fn animated_svg_moves_the_camera_group() {
    let (p, comp) = project();
    let svg = export_animated_svg(&p, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
    // CSS keyframes on the camera group: the first frame's view and the last one's (t = 0.9, zoom 1.9).
    assert!(svg.contains("transform: matrix(1, 0, 0, 1, 100, 0)"), "{svg}");
    assert!(svg.contains("transform: matrix(1.9, 0, 0, 1.9, 10, -90)"), "{svg}");
}

#[test]
fn lottie_hangs_everything_off_a_camera_null() {
    let (p, comp) = project();
    let json = xenify_export::lottie::export_lottie(&p, comp, &mut xenify_render::TextEngine::new()).unwrap();
    let layers = json["layers"].as_array().unwrap();
    let view = layers.iter().find(|l| l["nm"] == "Camera view").expect("a camera null");
    let ind = view["ind"].as_u64().unwrap();
    let square = layers.iter().find(|l| l["nm"] == "Square").unwrap();
    assert_eq!(square["parent"].as_u64(), Some(ind));
    // First and last keys: scale 100% → 200%, position (100, 0) → (0, -100).
    let s = view["ks"]["s"]["k"].as_array().unwrap();
    assert_eq!(s.first().unwrap()["s"][0].as_f64(), Some(100.0));
    assert!((s.last().unwrap()["s"][0].as_f64().unwrap() - 200.0).abs() < 1e-6);
    let pos = view["ks"]["p"]["k"].as_array().unwrap();
    let last = &pos.last().unwrap()["s"];
    assert!((last[0].as_f64().unwrap()).abs() < 1e-6 && (last[1].as_f64().unwrap() + 100.0).abs() < 1e-6);
}

#[test]
fn zoom_is_a_keyframeable_property() {
    let (p, _) = project();
    let cam = p.layer(Id(2)).unwrap();
    assert!(cam.property_paths().contains(&PropPath::Camera(CameraField::Zoom)));
    assert_eq!(cam.keyframe_times(PropPath::Camera(CameraField::Zoom)).unwrap(), vec![0.0, 1.0]);
    assert_eq!("camera_zoom".parse::<PropPath>(), Ok(PropPath::Camera(CameraField::Zoom)));
}

#[test]
fn pdf_pages_show_the_camera_view() {
    let Ok(mut gpu) = xenify_render::Offscreen::new() else {
        eprintln!("skipping: no GPU");
        return;
    };
    let (p, comp) = project();
    let pdf = xenify_export::pdf::export_pdf(&p, comp, 1.0, Default::default(), &mut gpu).unwrap();
    let text = String::from_utf8_lossy(&pdf).to_string();
    assert!(text.starts_with("%PDF"));
    // Content streams are compressed; a working export with the camera is the check here.
    assert!(pdf.len() > 200);
}

/// Text animators are exported glyph by glyph.
fn typed() -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    c.settings.duration = 1.0;
    c.settings.fps = 10.0;
    let mut text = xenify_doc::TextLayer::new("Hi there", "Inter", 40.0, Color::WHITE);
    text.animators.push(xenify_doc::TextAnimator::preset("Pop by Letter").unwrap());
    c.layers = vec![Layer::new(Id(1), "Title", LayerKind::Text(text)).with_position(Vec2::new(100.0, 100.0))];
    p.reserve_id(Id(1));
    (p, comp)
}

#[test]
fn svg_writes_animated_text_as_glyph_outlines() {
    let (p, comp) = typed();
    let mut fonts = xenify_render::TextEngine::new();
    let early = export_svg(&xenify_anim::evaluate_project(&p, comp, 0.05), &mut fonts);
    // 7 letters (the space has no glyph outline worth a letter, but may have a path).
    assert!(early.matches("<path").count() >= 7, "{early}");
    assert!(early.contains(r#"fill-opacity="0.000""#), "later letters are still hidden");
    let late = export_svg(&xenify_anim::evaluate_project(&p, comp, 1.0), &mut fonts);
    assert!(!late.contains(r#"fill-opacity="0.000""#));
    assert!(!late.contains("<text"), "outlines, not live text");
}

#[test]
fn lottie_bakes_animated_text_per_frame() {
    let (p, comp) = typed();
    let json = xenify_export::lottie::export_lottie(&p, comp, &mut xenify_render::TextEngine::new()).unwrap();
    let title = json["layers"].as_array().unwrap().iter().find(|l| l["nm"] == "Title").unwrap();
    assert_eq!(title["ty"], 0, "a precomp of frames");
    let asset = json["assets"].as_array().unwrap().iter().find(|a| a["id"] == title["refId"]).unwrap();
    assert_eq!(asset["layers"].as_array().unwrap().len(), 10);
}

/// A second of confetti.
fn confetti() -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    c.settings.duration = 1.0;
    c.settings.fps = 10.0;
    let (e, at) = xenify_doc::Emitter::preset("Confetti Burst", 1920.0, 1080.0).unwrap();
    c.layers = vec![Layer::new(Id(1), "Confetti", LayerKind::Particles(Box::new(e))).with_position(at)];
    p.reserve_id(Id(1));
    (p, comp)
}

#[test]
fn particles_export_to_svg_and_bake_into_lottie() {
    let (p, comp) = confetti();
    let still = export_svg(&xenify_anim::evaluate_project(&p, comp, 0.5), &mut xenify_anim::ApproxText);
    assert!(still.matches("<path").count() >= 100, "each particle is drawn");
    let animated = export_animated_svg(&p, comp, &mut xenify_anim::ApproxText, AnimatedSvgOptions::default()).unwrap();
    assert!(animated.contains("<animate") || animated.contains("@keyframes"), "the particles move");
    let json = xenify_export::lottie::export_lottie(&p, comp, &mut xenify_render::TextEngine::new()).unwrap();
    let layer = json["layers"].as_array().unwrap().iter().find(|l| l["nm"] == "Confetti").unwrap();
    assert_eq!(layer["ty"], 0, "a precomp of frames");
    let asset = json["assets"].as_array().unwrap().iter().find(|a| a["id"] == layer["refId"]).unwrap();
    assert_eq!(asset["layers"].as_array().unwrap().len(), 10);
}

/// `XENIFY_DUMP=<dir>`: writes frames of the fire, smoke and blast looks
/// to check by eye (skipped otherwise).
#[test]
fn dump_fire_and_blast_frames() {
    let Ok(dir) = std::env::var("XENIFY_DUMP") else { return };
    let Ok(mut gpu) = xenify_render::Offscreen::new() else { return };
    let mut shots: Vec<(String, Project, Id)> = Vec::new();
    for name in xenify_doc::FIRE_PRESETS.iter().filter(|n| **n != "Burning Shape") {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        p.compositions[0].settings.background = Color::rgb(0.03, 0.03, 0.06);
        let cmd = xenify_ops::particles::add_preset(&p, comp, Id(p.next_id), name, None).unwrap();
        xenify_ops::apply(&mut p, cmd).unwrap();
        shots.push((name.to_string(), p, comp));
    }
    for kind in [xenify_ops::blast::BlastKind::Explosion, xenify_ops::blast::BlastKind::BigBlast] {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        p.compositions[0].settings.background = Color::rgb(0.25, 0.35, 0.5);
        p.compositions[0].settings.duration = 6.0;
        let (cmd, _) =
            xenify_ops::blast::add_blast(&p, comp, Id(p.next_id), kind, Vec2::new(960.0, 900.0), 0.5, 1.0).unwrap();
        xenify_ops::apply(&mut p, cmd).unwrap();
        shots.push((kind.label().to_string(), p, comp));
    }
    {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        p.compositions[0].settings.background = Color::rgb(0.95, 0.95, 0.97);
        let cmd = xenify_ops::liquid::fill_screen(&p, comp, Id(p.next_id), 0.0).unwrap();
        xenify_ops::apply(&mut p, cmd).unwrap();
        shots.push(("Water Screen".into(), p, comp));
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        p.compositions[0].settings.background = Color::rgb(0.1, 0.1, 0.15);
        let title = Layer::new(
            Id(1),
            "Title",
            LayerKind::Text(xenify_doc::TextLayer::new("WATER", "Inter", 300.0, Color::WHITE).centered()),
        )
        .with_position(Vec2::new(960.0, 540.0));
        p.compositions[0].layers.push(title);
        p.reserve_id(Id(1));
        let scene = xenify_anim::evaluate_project(&p, comp, 0.0);
        let mut fonts = xenify_render::TextEngine::new();
        let b = xenify_anim::world_box(&p.compositions[0], &scene, Id(1), &mut fonts).unwrap();
        let cmd = xenify_ops::liquid::fill_layer(&p, comp, Id(p.next_id), Id(1), b, 0.0).unwrap();
        xenify_ops::apply(&mut p, cmd).unwrap();
        shots.push(("Water Text".into(), p, comp));
    }
    shots.push(("Shatter".into(), shattered().0, shattered().1));
    for (name, p, comp) in shots {
        for t in [0.6, 1.5, 3.0, 4.5] {
            let scene = xenify_anim::evaluate_project(&p, comp, t);
            let img = gpu.render(&scene, 960, 540).unwrap();
            std::fs::write(format!("{dir}/{}-{t}.png", name.replace(' ', "_")), img.to_png()).unwrap();
        }
    }
}

/// A title and a square breaking apart from 1 s.
fn shattered() -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    p.compositions[0].settings.background = Color::rgb(0.08, 0.08, 0.12);
    let key = |time, value| Keyframe { time, value, easing: Default::default() };
    let shatter = |origin: Vec2| {
        xenify_doc::Modifier::new(xenify_doc::ModifierKind::Shatter {
            count: 30.0.into(),
            origin: origin.into(),
            force: 500.0.into(),
            amount: 0.0.into(),
            rotation: (-90.0).into(),
            gravity: 700.0.into(),
            spin: 200.0.into(),
            progress: Property::from_keyframes(vec![key(1.0, 0.0), key(3.0, 2.0)]).unwrap(),
            seed: 1.0.into(),
        })
    };
    let mut title = Layer::new(
        Id(1),
        "Title",
        LayerKind::Text(xenify_doc::TextLayer::new("SHATTER", "Inter", 200.0, Color::WHITE).centered()),
    )
    .with_position(Vec2::new(960.0, 350.0));
    title.meta.modifiers.push(shatter(Vec2::ZERO));
    let mut square =
        Layer::shape(Id(2), "Square", vec![ShapeItem::rect(Vec2::new(300.0, 300.0), Color::rgb(0.2, 0.6, 1.0))])
            .with_position(Vec2::new(960.0, 780.0));
    square.meta.modifiers.push(shatter(Vec2::new(-150.0, 150.0)));
    p.compositions[0].layers = vec![title, square];
    p.reserve_id(Id(2));
    (p, comp)
}

#[test]
fn shattered_layers_export_as_clipped_pieces() {
    let (p, comp) = shattered();
    let svg = export_svg(&xenify_anim::evaluate_project(&p, comp, 1.5), &mut xenify_anim::ApproxText);
    assert!(svg.matches("<mask").count() >= 30, "each piece is masked to its shard");
    let whole = export_svg(&xenify_anim::evaluate_project(&p, comp, 0.5), &mut xenify_anim::ApproxText);
    assert_eq!(whole.matches("<mask").count(), 0, "in one piece before it breaks");
    let json = xenify_export::lottie::export_lottie(&p, comp, &mut xenify_render::TextEngine::new()).unwrap();
    let square = json["layers"].as_array().unwrap().iter().find(|l| l["nm"] == "Square").unwrap();
    assert_eq!(square["ty"], 0, "baked frame by frame");
}

#[test]
fn pdf_pages_bake_particles_and_shards() {
    let Ok(mut gpu) = xenify_render::Offscreen::new() else { return };
    for (mut p, comp) in [confetti(), shattered()] {
        // Small: shards fall back to pictures in PDF.
        let c = p.comp_mut(comp).unwrap();
        (c.settings.width, c.settings.height) = (480, 270);
        let pdf = xenify_export::pdf::export_pdf(&p, comp, 1.5, Default::default(), &mut gpu).unwrap();
        assert!(pdf.starts_with(b"%PDF") && pdf.len() > 1000);
    }
}
