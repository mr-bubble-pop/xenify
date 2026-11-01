//! End-to-end export: GPU render → ffmpeg → probe the file.
//! Skipped (with a message) when there is no GPU or no ffmpeg.

use std::process::Command;
use xenify_doc::demo_project;
use xenify_export::{ExportError, ExportOptions, export_to_file};
use xenify_platform::VideoFormat;
use xenify_platform::native::FfmpegSink;
use xenify_render::Offscreen;

fn setup() -> Option<Offscreen> {
    if !FfmpegSink::available() {
        eprintln!("skipping: ffmpeg not found");
        return None;
    }
    Offscreen::new().map_err(|e| eprintln!("skipping: {e}")).ok()
}

fn probe(path: &std::path::Path) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-show_entries", "stream=codec_name,width,height,nb_read_frames"])
        .args(["-of", "compact=p=0:nk=0"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn exports_mp4_gif_and_pngs() {
    let Some(mut gpu) = setup() else { return };
    let dir = std::env::temp_dir().join(format!("xenify-export-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = demo_project();
    let comp = project.main_comp().unwrap();
    let frames = comp.settings.frame_count();

    let mp4 = dir.join("demo.mp4");
    let mut calls = 0;
    export_to_file(
        &project,
        comp.id,
        &mp4,
        &ExportOptions { format: VideoFormat::Mp4, scale: 0.25, transparent: false, ..Default::default() },
        &mut gpu,
        &mut |_, _| {
            calls += 1;
            true
        },
    )
    .unwrap();
    assert_eq!(calls, frames + 1);
    assert_eq!(probe(&mp4), format!("codec_name=h264|width=320|height=180|nb_read_frames={frames}"));

    let gif = dir.join("demo.gif");
    export_to_file(
        &project,
        comp.id,
        &gif,
        &ExportOptions { format: VideoFormat::Gif, scale: 0.25, transparent: false, ..Default::default() },
        &mut gpu,
        &mut |_, _| true,
    )
    .unwrap();
    assert!(probe(&gif).starts_with("codec_name=gif|width=320|height=180"));

    let pngs = dir.join("pngs");
    export_to_file(
        &project,
        comp.id,
        &pngs,
        &ExportOptions { format: VideoFormat::PngSequence, scale: 0.1, transparent: false, ..Default::default() },
        &mut gpu,
        &mut |_, _| true,
    )
    .unwrap();
    assert_eq!(std::fs::read_dir(&pngs).unwrap().count() as u64, frames);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cancelling_stops_the_export() {
    let Some(mut gpu) = setup() else { return };
    let project = demo_project();
    let out = std::env::temp_dir().join(format!("xenify-cancel-{}.mp4", std::process::id()));
    let err = export_to_file(
        &project,
        project.main_comp().unwrap().id,
        &out,
        &ExportOptions { format: VideoFormat::Mp4, scale: 0.1, transparent: false, ..Default::default() },
        &mut gpu,
        &mut |done, _| done < 5,
    )
    .unwrap_err();
    assert!(matches!(err, ExportError::Cancelled));
    let _ = std::fs::remove_file(out);
}

#[test]
fn transparent_png_and_webm_keep_alpha() {
    let Some(mut gpu) = setup() else { return };
    let project = demo_project();
    let comp = project.main_comp().unwrap();
    let dir = std::env::temp_dir().join(format!("xenify-alpha-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pngs = dir.join("pngs");
    let opts = ExportOptions { format: VideoFormat::PngSequence, scale: 0.1, transparent: true, ..Default::default() };
    export_to_file(&project, comp.id, &pngs, &opts, &mut gpu, &mut |_, _| true).unwrap();
    let first = std::fs::read_dir(&pngs).unwrap().next().unwrap().unwrap().path();
    let img = image::open(&first).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(0, 0)[3], 0, "background left out");
    assert!(img.pixels().any(|p| p[3] == 255), "content stays opaque");
    let webm = dir.join("a.webm");
    let opts = ExportOptions { format: VideoFormat::WebM, scale: 0.1, transparent: true, ..Default::default() };
    export_to_file(&project, comp.id, &webm, &opts, &mut gpu, &mut |_, _| true).unwrap();
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream_tags=alpha_mode", "-of", "csv=p=0"])
        .arg(&webm)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "1", "webm has an alpha channel");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exports_a_whole_deck_as_one_video() {
    use xenify_doc::{CompSettings, Composition, Slide, Transition, TransitionKind};
    let Some(mut gpu) = setup() else { return };
    let mut p = xenify_doc::Project::new();
    let a = p.compositions[0].id;
    let s = &mut p.compositions[0].settings;
    (s.width, s.height, s.fps, s.duration) = (160, 90, 10.0, 1.0);
    s.pauses = vec![0.5]; // ignored when exporting
    let b = p.alloc_id();
    p.compositions.push(Composition {
        id: b,
        settings: CompSettings { width: 160, height: 90, fps: 10.0, duration: 2.0, ..Default::default() },
        layers: vec![],
    });
    let mut second = Slide::new(b);
    second.transition_in = Transition::new(TransitionKind::Fade, 0.5);
    p.slides = vec![Slide::new(a), second];

    let dir = std::env::temp_dir().join(format!("xenify-deck-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("deck.mp4");
    let opts = ExportOptions { format: VideoFormat::Mp4, scale: 1.0, transparent: false, ..Default::default() };
    xenify_export::export_deck_to_file(&p, &out, &opts, &mut gpu, &mut |_, _| true).unwrap();
    // 1 s + 0.5 s fade + 2 s at 10 fps.
    assert_eq!(probe(&out), "codec_name=h264|width=160|height=90|nb_read_frames=35");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_pages_are_vector_with_selectable_text() {
    use xenify_doc::{
        Color, Effect, EffectKind, Fill, Gradient, GradientKind, GradientStop, Id, Layer, LayerKind, ShapeItem,
        TextLayer, Vec2,
    };
    let Some(mut gpu) = setup() else { return };
    let mut p = xenify_doc::Project::new();
    let comp = p.compositions[0].id;
    let mut card = ShapeItem::rect(Vec2::new(400.0, 200.0), Color::WHITE);
    card.fill = Some(Fill {
        gradient: Some(
            Gradient {
                kind: GradientKind::Linear,
                start: Vec2::new(-200.0, 0.0),
                end: Vec2::new(200.0, 0.0),
                stops: vec![
                    GradientStop { offset: 0.0, color: Color::rgb(1.0, 0.0, 0.0) },
                    GradientStop { offset: 0.5, color: Color::rgb(0.0, 1.0, 0.0) },
                    GradientStop { offset: 1.0, color: Color::rgb(0.0, 0.0, 1.0) },
                ],
            }
            .into(),
        ),
        ..Fill::solid(Color::WHITE)
    });
    let card = Layer::shape(Id(10), "card", vec![card]).with_position(Vec2::new(500.0, 400.0));
    let title =
        Layer::new(Id(11), "title", LayerKind::Text(TextLayer::new("Quarterly results", "Inter", 80.0, Color::WHITE)))
            .with_position(Vec2::new(960.0, 200.0));
    let mut blurred = Layer::shape(Id(12), "soft", vec![ShapeItem::ellipse(Vec2::new(100.0, 100.0), Color::WHITE)])
        .with_position(Vec2::new(1400.0, 700.0));
    blurred.meta.effects = vec![Effect::new(EffectKind::Blur { radius: 10.0.into() })];
    p.compositions[0].layers = vec![card, title, blurred];
    let pdf = xenify_export::pdf::export_pdf(&p, comp, 0.0, Default::default(), &mut gpu).unwrap();
    // The page content is compressed: inflate the streams to look inside.
    let mut streams = String::new();
    let mut rest = &pdf[..];
    while let Some(i) = rest.windows(7).position(|w| w == b"stream\n") {
        let start = i + 7;
        let end = rest[start..].windows(10).position(|w| w == b"\nendstream").unwrap() + start;
        if let Ok(data) = miniz_oxide::inflate::decompress_to_vec_zlib(&rest[start..end]) {
            streams.push_str(&String::from_utf8_lossy(&data));
        }
        rest = &rest[end + 10..];
    }
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/ShadingType 2") && text.contains("/FunctionType 3"), "the gradient is a vector shading");
    assert!(streams.contains(" c\n") && streams.contains("/P0 scn"), "vector paths filled with the gradient");
    assert!(streams.contains("3 Tr") && streams.contains("(Quarterly results) Tj"), "invisible, selectable text");
    assert!(text.contains("/SMask"), "the blurred layer is a transparent picture");
    assert!(text.contains("/Count 1"));
}
