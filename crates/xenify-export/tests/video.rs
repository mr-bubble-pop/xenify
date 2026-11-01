//! Video layers: frames decoded with ffmpeg, drawn, exported.
//! Skipped (with a message) without ffmpeg or a GPU.

use std::path::{Path, PathBuf};
use std::process::Command;
use xenify_doc::{Id, MediaAsset, MediaKind, MediaSource, Project, Vec2};

/// A 2 s, 10 fps test pattern (ffmpeg's testsrc, with a frame counter).
fn clip(dir: &Path) -> Option<PathBuf> {
    let path = dir.join("pattern.mp4");
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc=size=160x120:rate=10:duration=2", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .status()
        .ok()?
        .success()
        .then_some(path)
}

fn project_with(path: &Path) -> (Project, Id, Id) {
    let info = xenify_platform::media::probe(path).unwrap();
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    (c.settings.width, c.settings.height, c.settings.duration, c.settings.fps) = (320, 240, 2.0, 10.0);
    let media = MediaAsset {
        id: Id(p.next_id),
        name: "pattern".into(),
        kind: MediaKind::Video,
        mime: "video/mp4".into(),
        source: MediaSource::Linked { path: path.to_string_lossy().into() },
        duration: info.duration,
        width: info.width,
        height: info.height,
        fps: info.fps,
        has_audio: false,
        hash: 1234,
        recipe: None,
    };
    let layer = Id(media.id.0 + 1);
    let cmd = xenify_ops::media::add_clip(&p, comp, media, layer, 0.0).unwrap();
    xenify_ops::apply(&mut p, cmd).unwrap();
    (p, comp, layer)
}

#[test]
fn video_frames_are_drawn_and_exported() {
    if !xenify_platform::native::FfmpegSink::available() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    xenify_platform::media::install();
    let dir = std::env::temp_dir().join(format!("xenify-video-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = clip(&dir).unwrap();
    let (p, comp, layer) = project_with(&path);
    assert_eq!(p.layer(layer).unwrap().transform.scale.value_at(0.0), Vec2::new(2.0, 2.0), "fitted to the canvas");

    // The picture changes as the clip plays.
    if let Ok(mut gpu) = xenify_render::Offscreen::new() {
        let at = |gpu: &mut xenify_render::Offscreen, t: f64| {
            gpu.render(&xenify_anim::evaluate_project(&p, comp, t), 320, 240).unwrap().pixels
        };
        let (a, b, a2) = (at(&mut gpu, 0.0), at(&mut gpu, 1.0), at(&mut gpu, 0.0));
        assert_ne!(a, b, "a second later, a different frame");
        assert_eq!(a, a2, "the same moment, the same picture");
        assert!(a.chunks(4).any(|px| px != [0, 0, 0, 255]), "not just the background");
    }
    // SVG: the frame as a picture.
    let svg =
        xenify_export::svg::export_svg(&xenify_anim::evaluate_project(&p, comp, 0.5), &mut xenify_anim::ApproxText);
    assert!(svg.contains("data:image/png;base64,"), "the frame is embedded");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exported_video_carries_the_sound() {
    if !xenify_platform::native::FfmpegSink::available() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let Ok(mut gpu) = xenify_render::Offscreen::new() else { return };
    xenify_platform::media::install();
    let dir = std::env::temp_dir().join(format!("xenify-sound-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tone = dir.join("tone.wav");
    assert!(
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .arg(&tone)
            .status()
            .unwrap()
            .success()
    );
    let info = xenify_platform::media::probe(&tone).unwrap();
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    (c.settings.width, c.settings.height, c.settings.duration, c.settings.fps) = (64, 64, 1.0, 10.0);
    let media = MediaAsset {
        id: Id(p.next_id),
        name: "tone".into(),
        kind: MediaKind::Audio,
        mime: "audio/wav".into(),
        source: MediaSource::Linked { path: tone.to_string_lossy().into() },
        duration: info.duration,
        width: 0,
        height: 0,
        fps: 0.0,
        has_audio: true,
        hash: 777,
        recipe: None,
    };
    let cmd = xenify_ops::media::add_clip(&p, comp, media, Id(p.next_id + 1), 0.0).unwrap();
    xenify_ops::apply(&mut p, cmd).unwrap();
    // The mix is the tone, as loud as the file.
    let m = xenify_anim::media::MediaRef::of(p.media.first().unwrap());
    let source = xenify_platform::media::decode_pcm(&m).unwrap();
    let mix = xenify_audio::mix_comp(&p, comp, 0.0, 1.0);
    let peak = |v: &[f32]| v.iter().fold(0.0f32, |a, x| a.max(x.abs()));
    assert!(peak(&mix) > 0.05 && (peak(&mix) - peak(&source)).abs() < 1e-4);

    let out = dir.join("with_sound.mp4");
    xenify_export::export_to_file(&p, comp, &out, &Default::default(), &mut gpu, &mut |_, _| true).unwrap();
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name", "-of", "compact=p=0:nk=1"])
        .arg(&out)
        .output()
        .unwrap();
    let streams = String::from_utf8_lossy(&probe.stdout).to_string();
    assert!(streams.contains("aac|audio"), "an AAC sound track: {streams}");
    assert!(streams.contains("h264|video"), "{streams}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_at_4k_and_letterboxes_other_shapes() {
    if !xenify_platform::native::FfmpegSink::available() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let Ok(mut gpu) = xenify_render::Offscreen::new() else { return };
    let dir = std::env::temp_dir().join(format!("xenify-size-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A square red canvas, 0.2 s.
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let c = p.comp_mut(comp).unwrap();
    (c.settings.width, c.settings.height, c.settings.duration, c.settings.fps) = (100, 100, 0.2, 10.0);
    c.settings.background = xenify_doc::Color::rgb(1.0, 0.0, 0.0);
    let size = xenify_export::parse_size("4k", false).unwrap();
    let options = xenify_export::ExportOptions {
        size: Some(size),
        letterbox: xenify_doc::Color::rgb(0.0, 1.0, 0.0),
        ..Default::default()
    };
    let out = dir.join("uhd.mp4");
    xenify_export::export_to_file(&p, comp, &out, &options, &mut gpu, &mut |_, _| true).unwrap();
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "3840,2160");
    // The frame itself: red in the middle, green bars at the sides.
    gpu.set_letterbox(xenify_doc::Color::rgb(0.0, 1.0, 0.0));
    let img = gpu.render(&xenify_anim::evaluate_project(&p, comp, 0.0), 384, 216).unwrap();
    assert_eq!(img.pixel(192, 108)[..3], [255, 0, 0]);
    assert_eq!(img.pixel(10, 108)[..3], [0, 255, 0]);
    let _ = std::fs::remove_dir_all(&dir);
}
