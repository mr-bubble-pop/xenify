//! Bringing images and SVG in: clipboard and drag and drop.

use super::common::*;
use egui::{Key, Modifiers};
use xenify_doc::LayerKind;
use xenify_platform::desktop::ClipContent;
use xenify_ui::testing::FakeLog;

fn png(w: u32, h: u32) -> Vec<u8> {
    xenify_render::RgbaImage { width: w, height: h, pixels: vec![200; (w * h * 4) as usize] }.to_png()
}

fn image_layers(h: &Harness) -> Vec<xenify_doc::Layer> {
    h.state().comp().layers.iter().filter(|l| matches!(l.kind, LayerKind::Image { .. })).cloned().collect()
}

#[test]
fn ctrl_v_pastes_an_image_from_the_clipboard() {
    let log = FakeLog::default();
    log.with(|s| s.clipboard = Some(ClipContent::Image { width: 3, height: 2, rgba: vec![255; 24] }));
    let mut h = harness(log.clone());
    // No text on the clipboard: the system sends no paste event, just the keys.
    h.key_press_modifiers(Modifiers::COMMAND, Key::V);
    h.run_steps(2);
    assert_eq!(log.with(|s| s.clipboard_reads), 1);
    let imgs = image_layers(&h);
    assert_eq!(imgs.len(), 1);
    let LayerKind::Image { asset, .. } = imgs[0].kind else { unreachable!() };
    let a = h.state().project().asset(asset).unwrap();
    assert_eq!((a.width, a.height, a.mime.as_str()), (3, 2, "image/png"));
    assert_eq!(h.state().selected, Some(imgs[0].id));

    // One undo removes it again.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert!(image_layers(&h).is_empty());
}

#[test]
fn a_text_paste_does_not_also_read_the_clipboard() {
    let log = FakeLog::default();
    let mut h = harness(log.clone());
    h.event(egui::Event::Paste("just some words".into()));
    h.key_press_modifiers(Modifiers::COMMAND, Key::V);
    h.run_steps(2);
    assert_eq!(log.with(|s| s.clipboard_reads), 0);
}

#[test]
fn pasted_svg_text_becomes_a_group() {
    let mut h = harness(FakeLog::default());
    let before = h.state().comp().layers.len();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="40" height="40" fill="#f00"/><circle cx="70" cy="70" r="20" fill="#00f"/></svg>"##;
    h.event(egui::Event::Paste(svg.into()));
    h.run_steps(2);
    let layers = &h.state().comp().layers;
    assert_eq!(layers.len(), before + 3, "two shapes and their group");
    let group = layers.last().unwrap();
    assert!(matches!(group.kind, LayerKind::Group));
    assert_eq!(h.state().selected, Some(group.id));
}

#[test]
fn dropped_image_lands_where_it_is_dropped() {
    let mut h = harness(FakeLog::default());
    let at = egui::pos2(300.0, 300.0);
    let expected = h.state().viewport.screen_to_comp(at).expect("over the canvas");
    h.hover_at(at);
    h.input_mut().dropped_files.push(egui::DroppedFile {
        name: "photo.png".into(),
        bytes: Some(png(10, 6).into()),
        ..Default::default()
    });
    h.run_steps(2);
    let imgs = image_layers(&h);
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].meta.name, "photo");
    let pos = imgs[0].transform.position.value_at(0.0);
    assert!((pos.x - expected.x).abs() < 1e-6 && (pos.y - expected.y).abs() < 1e-6, "{pos:?} vs {expected:?}");
}

#[test]
fn edit_paste_imports_copied_files() {
    let dir = std::env::temp_dir().join(format!("xenify-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("logo.png");
    std::fs::write(&file, png(4, 4)).unwrap();
    let log = FakeLog::default();
    log.with(|s| s.clipboard = Some(ClipContent::Files(vec![file.clone(), dir.join("notes.txt")])));
    std::fs::write(dir.join("notes.txt"), "hello").unwrap();
    let mut h = harness(log);
    {
        let app = h.state_mut();
        let ctx = egui::Context::default();
        xenify_ui::testing::run_action(app, &ctx, "edit.paste");
    }
    h.run_steps(2);
    assert_eq!(image_layers(&h).len(), 1);
    assert!(h.state().status.contains("notes.txt"), "{}", h.state().status);
}

/// Dropped files arrive with the path the desktop sees; inside a dev
/// container, paths in the shared project folder are translated.
#[test]
fn dropping_a_host_path_from_the_shared_folder_works() {
    let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let shared = xenify_platform::desktop::shared_folders(&info);
    let Some((host, local)) = shared.first().cloned().filter(|_| xenify_platform::desktop::in_container()) else {
        eprintln!("skipping: not in a container with a shared folder");
        return;
    };
    let dir = local.join("target").join("drop-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("dropped.png"), png(5, 5)).unwrap();
    let host_path = host.join("target/drop-test/dropped.png");

    let mut h = harness(FakeLog::default());
    h.hover_at(egui::pos2(300.0, 300.0));
    h.input_mut().dropped_files.push(egui::DroppedFile { path: Some(host_path), ..Default::default() });
    h.run_steps(2);
    assert_eq!(image_layers(&h).len(), 1, "{}", h.state().status);

    // Outside the shared folder: nothing to read, and the status says why.
    h.input_mut()
        .dropped_files
        .push(egui::DroppedFile { path: Some("/home/nobody/Downloads/photo.png".into()), ..Default::default() });
    h.run_steps(2);
    assert_eq!(image_layers(&h).len(), 1);
    assert!(h.state().status.contains("outside the folders"), "{}", h.state().status);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dropped_3d_model_becomes_a_turnable_3d_layer() {
    // An OBJ cube whose colour comes from the MTL file beside it.
    let dir = std::env::temp_dir().join(format!("xenify-model-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut obj = String::from("mtllib box.mtl\nusemtl orange\n");
    for i in 0..8 {
        obj += &format!("v {} {} {}\n", i & 1, (i >> 1) & 1, (i >> 2) & 1);
    }
    // Counter-clockwise from outside, 1-based.
    for q in [[1, 3, 4, 2], [5, 6, 8, 7], [1, 2, 6, 5], [3, 7, 8, 4], [1, 5, 7, 3], [2, 4, 8, 6]] {
        obj += &format!("f {} {} {} {}\n", q[0], q[1], q[2], q[3]);
    }
    std::fs::write(dir.join("box.obj"), obj).unwrap();
    std::fs::write(dir.join("box.mtl"), "newmtl orange\nKd 1 0.5 0\n").unwrap();

    let mut h = harness(FakeLog::default());
    h.input_mut().dropped_files.push(egui::DroppedFile { path: Some(dir.join("box.obj")), ..Default::default() });
    h.run_steps(2);
    // Read in the background: the window keeps working meanwhile.
    for _ in 0..500 {
        if h.state().pending_models.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let layer = h.state().comp().layers.last().unwrap().clone();
    assert_eq!(layer.meta.name, "box");
    let LayerKind::Solid(solid) = &layer.kind else { panic!("not a 3D layer: {:?}", layer.kind) };
    let xenify_doc::SolidShape::Mesh(mesh) = &solid.shape else { panic!("not a model") };
    assert_eq!(mesh.polygon_count(), 6);
    assert!(mesh.own_colors);
    assert_eq!(mesh.parts[0].color, Some(xenify_doc::Color::rgb(1.0, 0.5, 0.0)));
    assert_eq!(h.state().selected, Some(layer.id));
    assert_eq!(format!("{:?}", h.state().tool), "Orbit");
    assert!(h.state().status.contains("12 triangles"), "{}", h.state().status);

    // It draws three shaded, orange faces.
    let scene = xenify_anim::evaluate_project(h.state().project(), h.state().comp, 0.0);
    let xenify_anim::SceneContent::Shapes(items) = &scene.layer(layer.id).unwrap().content else { panic!() };
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.fill.is_some_and(|c| c.r > c.g && c.g > c.b)));
    // The project file keeps it.
    let back = xenify_doc::Project::from_json(&h.state().project().to_json()).unwrap();
    assert_eq!(back.layer(layer.id).unwrap().kind, layer.kind);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn animated_gifs_play_and_their_playback_is_editable() {
    use image::codecs::gif::GifEncoder;
    let mut gif = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut gif);
        for c in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            let img = image::RgbaImage::from_pixel(8, 8, image::Rgba(c));
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(250, 1))).unwrap();
        }
    }
    use egui_kittest::kittest::Queryable;
    let file = std::env::temp_dir().join(format!("xenify-gui-blink-{}.gif", std::process::id()));
    std::fs::write(&file, gif).unwrap();
    let mut h = harness(FakeLog::default());
    h.state_mut().import_files(vec![file.clone()], None);
    let _ = std::fs::remove_file(&file);
    h.run_steps(3);
    assert!(h.state().status.contains("animated image"), "{}", h.state().status);
    let id = h.state().selected.expect("selected");
    let LayerKind::Image { asset, .. } = h.state().project().layer(id).unwrap().kind else { panic!() };
    assert_eq!(h.state().project().asset(asset).unwrap().frames.as_ref().map(Vec::len), Some(2));
    h.get_by_label("Loop Animation").click();
    h.run_steps(2);
    let looped = |a: &xenify_ui::XenifyApp| matches!(a.project().layer(id).unwrap().kind, LayerKind::Image { playback, .. } if playback.looped);
    assert!(!looped(h.state()));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(looped(h.state()));
}
