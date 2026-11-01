//! Lottie import: our own Lottie exports come back as layers that draw the
//! same picture at every moment.

use kurbo::{Affine, Rect, Shape};
use xenify_anim::{Scene, SceneContent, SceneLayer};
use xenify_doc::{Color, Easing, Id, Keyframe, Layer, Mask, Project, Property, ShapeItem, Trim, Vec2};
use xenify_export::lottie::export_lottie;
use xenify_export::lottie_import::import_lottie;

fn keys<T: xenify_doc::Animatable>(k: &[(f64, T)], easing: Easing) -> Property<T> {
    Property::from_keyframes(k.iter().cloned().map(|(time, value)| Keyframe { time, value, easing }).collect()).unwrap()
}

/// A drawn thing: its box in the frame, fill, and stroke (colour, width).
type Drawn = (Rect, Option<Color>, Option<(Color, f64)>);

/// Each drawn thing, flattened.
fn picture(scene: &Scene) -> Vec<Drawn> {
    fn walk(l: &SceneLayer, parent_op: f64, cam: Affine, out: &mut Vec<Drawn>) {
        let op = parent_op * l.opacity;
        match &l.content {
            SceneContent::Shapes(items) => {
                for it in items {
                    if it.path.elements().is_empty() {
                        continue;
                    }
                    let bb = (cam * l.transform * it.path.clone()).bounding_box();
                    let fill = it.fill.map(|c| c.with_alpha(c.a * op)).filter(|c| c.a > 0.001);
                    let stroke = it.stroke.as_ref().map(|s| (s.color.with_alpha(s.color.a * op), s.width));
                    if fill.is_some() || stroke.is_some() {
                        out.push((bb, fill, stroke));
                    }
                }
            }
            SceneContent::Group(ls) | SceneContent::Precomp { layers: ls, .. } => {
                for c in ls {
                    walk(c, op, cam, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for l in &scene.layers {
        walk(l, 1.0, scene.camera, &mut out);
    }
    out.sort_by(|a, b| a.0.x0.total_cmp(&b.0.x0).then(a.0.y0.total_cmp(&b.0.y0)));
    out
}

fn same_picture(a: &Project, ca: Id, b: &Project, cb: Id, times: &[f64]) {
    for &t in times {
        let (pa, pb) =
            (picture(&xenify_anim::evaluate_project(a, ca, t)), picture(&xenify_anim::evaluate_project(b, cb, t)));
        assert!(!pa.is_empty(), "nothing drawn at {t}");
        assert_eq!(pa.len(), pb.len(), "at {t}: {pa:#?}\nvs\n{pb:#?}");
        for (x, y) in pa.iter().zip(&pb) {
            let close = |p: f64, q: f64| (p - q).abs() < 0.75;
            let (r, s) = (x.0, y.0);
            assert!(
                close(r.x0, s.x0) && close(r.y0, s.y0) && close(r.x1, s.x1) && close(r.y1, s.y1),
                "at {t}: box {r:?} vs {s:?}"
            );
            let col = |p: Option<Color>, q: Option<Color>| match (p, q) {
                (Some(p), Some(q)) => {
                    (p.r - q.r).abs() + (p.g - q.g).abs() + (p.b - q.b).abs() + (p.a - q.a).abs() < 0.02
                }
                (None, None) => true,
                _ => false,
            };
            assert!(col(x.1, y.1), "at {t}: fill {:?} vs {:?}", x.1, y.1);
            assert!(col(x.2.map(|s| s.0), y.2.map(|s| s.0)), "at {t}: stroke {:?} vs {:?}", x.2, y.2);
        }
    }
}

/// Exports `comp` to Lottie and imports it into a project of its own.
fn round_trip(p: &Project, comp: Id) -> (Project, Id, Vec<String>) {
    let json = export_lottie(p, comp, &mut xenify_render::TextEngine::new()).unwrap();
    let imp = import_lottie(json.to_string().as_bytes(), 1).unwrap();
    let mut q = Project::new();
    let id = q.compositions[0].id;
    let src = &p.comp(comp).unwrap().settings;
    let c = q.comp_mut(id).unwrap();
    assert_eq!((imp.width, imp.height), (src.width, src.height));
    assert!((imp.duration - src.duration).abs() < 1.0 / src.fps + 1e-9, "{} vs {}", imp.duration, src.duration);
    c.settings.width = imp.width;
    c.settings.height = imp.height;
    c.settings.fps = imp.fps;
    c.settings.duration = imp.duration;
    c.settings.background = src.background;
    c.layers = imp.layers;
    q.compositions.extend(imp.comps);
    q.reserve_id(Id(imp.next_id));
    (q, id, imp.skipped)
}

fn project(layers: Vec<Layer>) -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    c.settings.width = 400;
    c.settings.height = 300;
    c.settings.duration = 2.0;
    c.settings.fps = 30.0;
    let top = layers.iter().map(|l| l.id.0).max().unwrap_or(1);
    c.layers = layers;
    p.reserve_id(Id(top + 50));
    (p, comp)
}

#[test]
fn moving_eased_and_parented_shapes_round_trip() {
    let ease = Easing::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
    let mut ball =
        Layer::shape(Id(1), "Ball", vec![ShapeItem::ellipse(Vec2::new(40.0, 40.0), Color::rgb(1.0, 0.2, 0.1))]);
    ball.transform.position = keys(&[(0.0, Vec2::new(50.0, 150.0)), (1.5, Vec2::new(350.0, 80.0))], ease);
    ball.opacity = keys(&[(0.0, 1.0), (2.0, 0.3)], Easing::Linear);
    let mut boxy = ShapeItem::rect(Vec2::new(60.0, 30.0), Color::rgb(0.1, 0.5, 0.9));
    boxy.stroke =
        Some(xenify_doc::Stroke { color: Color::rgb(0.0, 0.0, 0.0).into(), width: 4.0.into(), ..Default::default() });
    let mut spinner = Layer::shape(Id(2), "Spinner", vec![boxy]).with_position(Vec2::new(200.0, 150.0));
    spinner.transform.rotation = keys(&[(0.0, 0.0), (1.0, 90.0)], Easing::Hold);
    spinner.transform.scale = keys(&[(0.0, Vec2::new(1.0, 1.0)), (2.0, Vec2::new(2.0, 0.5))], Easing::Linear);
    // Rides on the spinner.
    let mut moon =
        Layer::shape(Id(3), "Moon", vec![ShapeItem::ellipse(Vec2::new(10.0, 10.0), Color::rgb(0.9, 0.9, 0.2))])
            .with_position(Vec2::new(50.0, 0.0));
    moon.meta.parent = Some(Id(2));
    let (p, comp) = project(vec![ball, spinner, moon]);
    let (q, qc, skipped) = round_trip(&p, comp);
    assert!(skipped.is_empty(), "{skipped:?}");
    same_picture(&p, comp, &q, qc, &[0.0, 0.4, 0.99, 1.0, 1.3, 1.99]);
    // Keyframes stay keyframes (editable), easing and all.
    let ball = q.compositions[0].layers.iter().find(|l| l.meta.name == "Ball").unwrap();
    let k = ball.transform.position.keyframes();
    assert_eq!(k.len(), 2);
    assert_eq!(k[0].easing, ease);
}

#[test]
fn paths_trim_masks_and_in_out_points_round_trip() {
    let mut wave = kurbo::BezPath::new();
    wave.move_to((0.0, 0.0));
    wave.curve_to((40.0, -60.0), (80.0, 60.0), (120.0, 0.0));
    let mut line = ShapeItem::path(wave, None, Some((Color::rgb(0.2, 0.8, 0.3), 6.0)));
    line.trim =
        Some(Trim { start: 0.0.into(), end: keys(&[(0.0, 0.0), (1.0, 1.0)], Easing::Linear), offset: 0.0.into() });
    let mut l1 = Layer::shape(Id(1), "Wave", vec![line]).with_position(Vec2::new(100.0, 100.0));
    l1.meta.in_point = 0.5;
    l1.meta.out_point = Some(1.5);
    let mut l2 =
        Layer::shape(Id(2), "Masked", vec![ShapeItem::rect(Vec2::new(200.0, 200.0), Color::rgb(0.5, 0.0, 0.5))])
            .with_position(Vec2::new(200.0, 150.0));
    l2.meta.masks.push(Mask::rect("Window", Rect::new(-50.0, -50.0, 50.0, 50.0)));
    let (p, comp) = project(vec![l2, l1]);
    let (q, qc, _) = round_trip(&p, comp);
    same_picture(&p, comp, &q, qc, &[0.0, 0.6, 1.0, 1.49, 1.6]);
    let masked = q.compositions[0].layers.iter().find(|l| l.meta.name == "Masked").unwrap();
    assert_eq!(masked.meta.masks.len(), 1);
    // The mask's outline survives (a 100×100 square).
    let bb = masked.meta.masks[0].path.value_at(0.0).bounding_box();
    assert!((bb.width() - 100.0).abs() < 0.5, "{bb:?}");
}

#[test]
fn an_emoji_round_trips() {
    let e = xenify_export::emoji::find("🎉").unwrap();
    let layers = xenify_export::emoji::layers(e, 1, Vec2::new(200.0, 150.0), 120.0).unwrap();
    let (p, comp) = project(layers);
    let (q, qc, _) = round_trip(&p, comp);
    same_picture(&p, comp, &q, qc, &[0.0]);
    let n = |p: &Project, c| picture(&xenify_anim::evaluate_project(p, c, 0.0)).len();
    assert!(n(&q, qc) > 5, "an emoji is many shapes");
}

#[test]
fn a_looping_symbol_round_trips() {
    // A 0.5 s symbol: a dot moving right, looped from 0.25 s.
    let mut p = Project::new();
    let main = p.compositions[0].id;
    let mut dot =
        Layer::shape(Id(10), "Dot", vec![ShapeItem::ellipse(Vec2::new(10.0, 10.0), Color::rgb(0.0, 0.0, 1.0))]);
    dot.transform.position = keys(&[(0.0, Vec2::new(10.0, 50.0)), (0.5, Vec2::new(90.0, 50.0))], Easing::Linear);
    let mut inner =
        xenify_doc::Composition { id: Id(20), settings: p.compositions[0].settings.clone(), layers: vec![dot] };
    inner.settings.name = "Dot loop".into();
    inner.settings.width = 100;
    inner.settings.height = 100;
    inner.settings.duration = 0.5;
    let mut pre = Layer::precomp(Id(21), "Loop", &inner, Vec2::new(200.0, 150.0));
    pre.kind = xenify_doc::LayerKind::Precomp { comp: Id(20), start: 0.25, looped: true };
    p.compositions.push(inner);
    let c = p.comp_mut(main).unwrap();
    c.settings.width = 400;
    c.settings.height = 300;
    c.settings.duration = 2.0;
    c.settings.fps = 30.0;
    c.layers = vec![pre];
    p.reserve_id(Id(60));
    let (q, qc, skipped) = round_trip(&p, main);
    assert!(skipped.is_empty(), "{skipped:?}");
    let l = &q.compositions[0].layers[0];
    assert!(
        matches!(l.kind, xenify_doc::LayerKind::Precomp { looped: true, start, .. } if (start - 0.25).abs() < 1e-9)
    );
    // Mid-pass on the 1st, 2nd and 4th time round.
    same_picture(&p, main, &q, qc, &[0.4, 0.9, 1.9]);
    let x = |t| picture(&xenify_anim::evaluate_project(&p, main, t))[0].0.center().x;
    assert!((x(0.5) - x(1.0)).abs() < 1e-6, "loops every 0.5 s");
}

#[test]
fn not_lottie_is_an_error() {
    assert!(import_lottie(b"{\"hello\": 1}", 1).is_err());
    assert!(import_lottie(b"garbage", 1).is_err());
}

/// Imports every `*.json` in `$LOTTIE_DIR` and renders a strip of frames of
/// each to `$LOTTIE_DIR/out/` (a manual check against real-world files).
#[test]
#[ignore]
fn render_lottie_dir() {
    let Some(dir) = std::env::var_os("LOTTIE_DIR").map(std::path::PathBuf::from) else { return };
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let mut gpu = xenify_render::Offscreen::new().unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == "json")) {
        let imp = import_lottie(&std::fs::read(f).unwrap(), 1).unwrap();
        println!(
            "{}: {} layers, {} comps, {:.2}s, skipped {:?}",
            f.display(),
            imp.layers.len(),
            imp.comps.len(),
            imp.duration,
            imp.skipped
        );
        let mut q = Project::new();
        let id = q.compositions[0].id;
        let d = imp.duration;
        let next = imp.next_id;
        let (mut main, extras) = imp.into_compositions(id);
        main.settings.background = Color::WHITE;
        q.compositions = vec![main];
        q.compositions.extend(extras);
        q.reserve_id(Id(next));
        for k in 0..4 {
            let t = d * k as f64 / 4.0;
            let scene = xenify_anim::evaluate_project(&q, id, t);
            let img = gpu.render(&scene, 256, 256).unwrap();
            let name = format!("{}-{k}.png", f.file_stem().unwrap().to_string_lossy());
            std::fs::write(out.join(name), img.to_png()).unwrap();
        }
    }
}

#[test]
fn live_figures_export_as_their_outlines() {
    use xenify_doc::{Geometry, primitives::FigureKind};
    let figure = |kind, count: Property<f64>, at: Vec2| {
        let mut s = ShapeItem::rect(Vec2::new(1.0, 1.0), Color::rgb(0.9, 0.6, 0.1));
        s.geometry = Geometry::Figure {
            position: Vec2::ZERO.into(),
            size: Vec2::new(120.0, 120.0).into(),
            kind,
            count,
            inner: 0.45.into(),
            hole: 0.3.into(),
        };
        Layer::shape(xenify_doc::Id(0), "Figure", vec![s]).with_position(at)
    };
    let mut star = figure(FigureKind::Star, 5.0.into(), Vec2::new(100.0, 150.0));
    star.id = xenify_doc::Id(1);
    let mut gear = figure(FigureKind::Gear, 12.0.into(), Vec2::new(250.0, 150.0));
    gear.id = xenify_doc::Id(2);
    // A polygon whose sides change 4 → 4 (animated size, same corners).
    let mut poly = figure(FigureKind::Polygon, 6.0.into(), Vec2::new(350.0, 150.0));
    poly.id = xenify_doc::Id(3);
    if let xenify_doc::LayerKind::Shape { shapes } = &mut poly.kind
        && let Geometry::Figure { size, .. } = &mut shapes[0].geometry
    {
        *size = keys(&[(0.0, Vec2::new(60.0, 60.0)), (2.0, Vec2::new(120.0, 120.0))], Easing::Linear);
    }
    let (p, comp) = project(vec![star, gear, poly]);
    let (q, qc, _) = round_trip(&p, comp);
    same_picture(&p, comp, &q, qc, &[0.0, 1.0, 1.9]);
}
