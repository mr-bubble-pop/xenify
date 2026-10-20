//! The GIFs panel: GIPHY search with the user's key, adding a GIF as an
//! animated image, and the attribution.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::LayerKind;
use xenify_ui::testing::FakeLog;

fn wait_for(h: &mut Harness, label: &str) {
    for _ in 0..200 {
        h.run_steps(1);
        if h.query_by_label(label).is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("{label:?} never showed");
}

#[test]
fn search_giphy_and_add_a_gif() {
    let mut h = harness(FakeLog::default());
    xenify_ui::testing::fake_giphy(h.state_mut(), true);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("GIF (GIPHY)…").click();
    h.run_steps(3);
    assert!(xenify_ui::testing::panel_open(h.state(), "GIFs"));
    // Trending first, with GIPHY's attribution.
    wait_for(&mut h, "trending 1 by @maker (GIPHY)");
    assert!(h.query_by_label("Powered by GIPHY").is_some());
    // Search.
    h.get_by(|n| n.role() == egui::accesskit::Role::TextInput).focus();
    h.run_steps(1);
    h.get_by(|n| n.role() == egui::accesskit::Role::TextInput).type_text("cats");
    h.run_steps(1);
    h.get_by_label("Search").click();
    wait_for(&mut h, "cats 2 by @maker (GIPHY)");
    h.get_by_label("cats 2 by @maker (GIPHY)").click();
    let mut added = None;
    for _ in 0..200 {
        h.run_steps(1);
        added = h.state().comp().layers.iter().find(|l| l.meta.name.contains("cats 2")).cloned();
        if added.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let layer = added.expect("the GIF was added");
    let LayerKind::Image { asset, .. } = layer.kind else { panic!("an image layer") };
    let a = h.state().project().asset(asset).unwrap();
    assert_eq!(a.frames.as_ref().map(Vec::len), Some(2), "animated");
    assert!(a.name.contains("@maker"), "credits the maker: {}", a.name);
    assert!(h.state().status.contains("Powered by GIPHY"), "{}", h.state().status);
}

#[test]
fn without_a_key_it_asks_for_one() {
    let mut h = harness(FakeLog::default());
    xenify_ui::testing::fake_giphy(h.state_mut(), false);
    xenify_ui::testing::show_panel(h.state_mut(), "GIFs");
    h.run_steps(3);
    assert!(h.query_by_label("Save Key").is_some());
    assert!(h.query_by_label("https://developers.giphy.com/dashboard/").is_some());
}
