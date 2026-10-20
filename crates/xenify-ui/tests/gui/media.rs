//! Video and audio: importing a clip and its Properties.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_ui::testing::FakeLog;

#[test]
fn import_a_video_loop_it_fill_the_frame_and_undo() {
    if !xenify_platform::native::FfmpegSink::available() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let dir = std::env::temp_dir().join(format!("xenify-gui-video-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip.mp4");
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "testsrc=size=320x240:rate=10:duration=1", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let mut h = harness(FakeLog::default());
    h.state_mut().import_files(vec![path.clone()], None);
    h.run_steps(3);
    let video = |h: &Harness| {
        h.state().comp().layers.iter().find_map(|l| match &l.kind {
            LayerKind::Video(v) => Some((l.id, (**v).clone(), l.clone())),
            _ => None,
        })
    };
    let (id, v, layer) = video(&h).expect("a video layer");
    assert!(h.state().is_selected(id));
    assert_eq!(layer.meta.out_point, Some(1.0), "as long as the clip");
    assert!(!v.looped);
    assert!(h.query_by_label_contains("Linked:").is_some(), "Properties says where the file is");

    // Scroll Properties down (the timeline is below it).
    h.hover_at(h.get_by_label("Name").rect().center() + egui::vec2(0.0, 60.0));
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -800.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(10);
    h.get_by_label("Loop Clip").click();
    h.run_steps(2);
    assert!(video(&h).unwrap().1.looped);
    h.get_by_label("Fill Frame").click();
    h.run_steps(2);
    // 320×240 covering the canvas.
    let st = &h.state().comp().settings;
    let fill = (st.width as f64 / 320.0).max(st.height as f64 / 240.0);
    let s = video(&h).unwrap().2.transform.scale.value_at(0.0);
    assert!((s.x - fill).abs() < 1e-9, "{s:?} vs {fill}");

    for _ in 0..3 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    }
    assert!(video(&h).is_none());
    assert!(h.state().project().media.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_soundtrack_dialog_lists_clips_and_keeps_playing_through_pauses() {
    let mut h = harness(FakeLog::default());
    let media = xenify_doc::MediaAsset {
        id: xenify_doc::Id(h.state().project().next_id),
        name: "theme".into(),
        kind: xenify_doc::MediaKind::Audio,
        mime: "audio/wav".into(),
        source: xenify_doc::MediaSource::Embedded { data: xenify_audio::wav(&vec![0.0; 9600]).into() },
        duration: 0.1,
        width: 0,
        height: 0,
        fps: 0.0,
        has_audio: true,
        hash: 5,
        recipe: None,
    };
    let cmd = xenify_ops::media::add_to_soundtrack(h.state().project(), media, 0.0);
    h.state_mut().exec("add", cmd);
    h.get_by_label("Slide").click();
    h.run_steps(2);
    h.get_by_label("Soundtrack…").click();
    h.run_steps(10);
    h.get_by_label("theme");
    h.get_by_label("Keep playing while presenting waits at a pause").click();
    h.run_steps(2);
    assert!(h.state().project().soundtrack.keep_playing);
    h.get_by_label("Remove").click();
    h.run_steps(2);
    assert!(h.state().project().soundtrack.clips.is_empty());
    h.get_by_label("Done").click();
    h.run_steps(2);
    assert!(!xenify_ui::testing::dialog_open(h.state()));
}

#[test]
fn make_a_sound_change_its_recipe_and_undo() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Sound ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Pop").click();
    h.run_steps(3);
    let sound = |h: &Harness| {
        h.state().comp().layers.iter().find_map(|l| match &l.kind {
            LayerKind::Audio(a) => Some((l.id, a.asset)),
            _ => None,
        })
    };
    let (id, first) = sound(&h).expect("an audio layer");
    let media = h.state().project().media(first).unwrap().clone();
    assert!(media.is_embedded() && media.recipe.is_some(), "made by the program, with its recipe");
    assert!(xenify_audio::has_sound(h.state().project()));

    // Properties › Audio › Sound: a higher pitch (drag its number right), made again.
    h.set_size(egui::vec2(1600.0, 2600.0));
    h.run_steps(3);
    let field = h
        .get_by(|n| n.role() == egui::accesskit::Role::SpinButton && n.value().as_deref() == Some("0.0 st"))
        .rect()
        .center();
    h.drag_at(field);
    h.run_steps(1);
    for i in 1..=8 {
        h.hover_at(field + egui::vec2(5.0 * i as f32, 0.0));
        h.run_steps(1);
    }
    h.drop_at(field + egui::vec2(40.0, 0.0));
    h.run_steps(3);
    h.get_by_label("Make It Again").click();
    h.run_steps(3);
    let (_, now) = sound(&h).unwrap();
    assert_ne!(now, first, "a new sound");
    let recipe = xenify_audio::synth::recipe_of(h.state().project().media(now).unwrap()).unwrap();
    let xenify_audio::synth::Recipe::Sfx(s) = recipe else { panic!("an effect") };
    assert!(s.pitch > 0.0, "{}", s.pitch);

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(sound(&h).unwrap(), (id, first), "undo brings the first sound back");
}
