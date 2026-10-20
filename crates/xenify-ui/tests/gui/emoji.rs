//! The Emoji panel: search, add as a vector group, emoji confetti.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_ui::testing::FakeLog;

#[test]
fn search_an_emoji_add_it_as_shapes_and_throw_it_as_confetti() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Emoji…").click();
    h.run_steps(3);
    assert!(xenify_ui::testing::panel_open(h.state(), "Emoji"));
    let field = h.get_by(|n| n.role() == egui::accesskit::Role::TextInput);
    field.focus();
    h.run_steps(1);
    h.get_by(|n| n.role() == egui::accesskit::Role::TextInput).type_text("rocket");
    h.run_steps(3);
    let before = h.state().comp().layers.len();
    h.get_by_label("rocket").click();
    h.run_steps(3);
    let comp = h.state().comp();
    let group = comp
        .layers
        .iter()
        .find(|l| l.meta.name == "rocket" && matches!(l.kind, LayerKind::Group))
        .expect("a rocket group");
    let parts = comp.layers.iter().filter(|l| l.meta.parent == Some(group.id)).count();
    assert!(parts > 0 && comp.layers.len() > before, "made of shapes");
    assert_eq!(h.state().prefs.recent_emoji.first().map(String::as_str), Some("🚀"));

    // Right-click: a confetti burst of it.
    h.get_by_label("rocket").click_secondary();
    h.run_steps(2);
    h.get_by_label("Confetti Burst of It").click();
    h.run_steps(3);
    let comp = h.state().comp();
    let burst = comp.layers.iter().find(|l| l.meta.name == "rocket confetti").expect("confetti");
    let LayerKind::Particles(e) = &burst.kind else { panic!() };
    let xenify_doc::ParticleShape::Layer { layer: sprite } = e.particle else { panic!("an emoji sprite") };
    assert!(!comp.layer(sprite).unwrap().meta.visible, "the sprite itself is hidden");
    // One undo takes the confetti (and its sprite) away.
    let n = comp.layers.len();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(h.state().comp().layers.len() < n);
    assert!(h.state().comp().layers.iter().all(|l| l.meta.name != "rocket confetti"));
}

/// A small Lottie file: a red dot moving right for a second.
fn dot_lottie() -> String {
    serde_json::json!({"v": "5.7.4", "fr": 30, "ip": 0, "op": 30, "w": 100, "h": 100, "nm": "Dot", "layers": [{
        "ty": 4, "ind": 1, "nm": "Dot", "ip": 0, "op": 30, "st": 0,
        "ks": {"p": {"a": 1, "k": [{"t": 0, "s": [20, 50]}, {"t": 30, "s": [80, 50]}]}},
        "shapes": [{"ty": "el", "p": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [20, 20]}},
                   {"ty": "fl", "c": {"a": 0, "k": [1, 0, 0, 1]}, "o": {"a": 0, "k": 100}}]
    }]})
    .to_string()
}

#[test]
fn animated_emoji_come_in_as_looping_symbols() {
    // Everything "downloaded" already: the list and the animation are in the cache.
    let data = std::env::temp_dir().join(format!("xenify-gui-animated-emoji-{}", std::process::id()));
    let cache = data.join("downloads/noto-emoji-animation");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("api.json"),
        r#"{"icons":[{"codepoint":"1f389","popularity":5,"categories":["Celebration"],"tags":[":party-popper:"]}]}"#,
    )
    .unwrap();
    std::fs::write(cache.join("1f389.json"), dot_lottie()).unwrap();
    // SAFETY: only this test reads XENIFY_DATA_DIR.
    unsafe { std::env::set_var("XENIFY_DATA_DIR", &data) };

    let mut h = harness(FakeLog::default());
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Emoji…").click();
    h.run_steps(3);
    h.get_by_label("Animated").click();
    let wait = |h: &mut egui_kittest::Harness<'_, xenify_ui::XenifyApp>,
                done: &dyn Fn(&xenify_ui::XenifyApp) -> bool| {
        for _ in 0..200 {
            h.run_steps(1);
            if done(h.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out");
    };
    for _ in 0..200 {
        h.run_steps(1);
        if h.query_by_label("party popper (animated)").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.get_by_label("party popper (animated)").click();
    let is_symbol =
        |a: &xenify_ui::XenifyApp| a.comp().layers.iter().any(|l| matches!(l.kind, LayerKind::Precomp { .. }));
    wait(&mut h, &is_symbol);
    h.run_steps(3);
    let layer = h.state().comp().layers.iter().find(|l| matches!(l.kind, LayerKind::Precomp { .. })).unwrap().clone();
    let LayerKind::Precomp { comp, looped, .. } = layer.kind else { unreachable!() };
    assert!(looped, "loops by default");
    let inner = h.state().project().comp(comp).unwrap();
    assert!(inner.settings.name.contains("party popper"), "{}", inner.settings.name);
    assert_eq!(inner.layers[0].transform.position.keyframes().len(), 2, "its keyframes are editable");
    assert!(h.state().status.contains("CC BY 4.0"), "credited: {}", h.state().status);

    // Properties › Symbol: stop it looping; undo brings the loop back.
    h.get_by_label("Loop Symbol").click();
    h.run_steps(2);
    let looped = |a: &xenify_ui::XenifyApp| {
        matches!(a.project().layer(layer.id).unwrap().kind, LayerKind::Precomp { looped: true, .. })
    };
    assert!(!looped(h.state()));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(looped(h.state()));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn lottie_files_import_as_symbols() {
    let dir = std::env::temp_dir().join(format!("xenify-gui-lottie-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bouncy.json");
    std::fs::write(&file, dot_lottie()).unwrap();
    let mut h = harness(FakeLog::default());
    h.state_mut().import_files(vec![file], None);
    h.run_steps(3);
    let comp = h.state().comp();
    let layer = comp.layers.iter().find(|l| matches!(l.kind, LayerKind::Precomp { .. })).expect("a symbol");
    assert_eq!(h.state().selected, Some(layer.id));
    let LayerKind::Precomp { comp: inner, .. } = layer.kind else { unreachable!() };
    assert_eq!(h.state().project().comp(inner).unwrap().layers[0].meta.name, "Dot");
    assert!(h.state().status.contains("Imported"), "{}", h.state().status);
    let _ = std::fs::remove_dir_all(&dir);
}
