//! Camera layers: Layer › New › Camera, its Properties, and looking through it.

use super::common::*;
use egui_kittest::kittest::Queryable;
use xenify_doc::{LayerKind, Project};
use xenify_ui::testing::FakeLog;

fn camera(h: &Harness) -> Option<xenify_doc::Layer> {
    h.state().comp().layers.iter().find(|l| matches!(l.kind, LayerKind::Camera(_))).cloned()
}

#[test]
fn add_a_camera_shake_it_look_through_it_and_undo() {
    let mut h = harness(FakeLog::default());
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("New ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Camera").click();
    h.run_steps(3);
    let cam = camera(&h).expect("a camera was added");
    assert!(h.state().is_selected(cam.id), "and selected");
    // A new camera frames the canvas exactly.
    assert_eq!(xenify_anim::camera_view(h.state().comp(), 0.0), kurbo::Affine::IDENTITY);
    h.get_by_label("● Active camera");

    // Shake, from Properties.
    h.get_by_label("Shake").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Shake").click();
    h.run_steps(3);
    let LayerKind::Camera(c) = camera(&h).unwrap().kind else { unreachable!() };
    assert!(c.shake.is_some());
    h.get_by_label("Dies away"); // its settings are shown

    // Look through it: the canvas shows the camera's view and editing pauses.
    h.get_by_label("Look through camera").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Look through camera").click();
    h.run_steps(3);
    assert!(h.state().look_through_camera);
    h.get_by_label("View").click();
    h.run_steps(2);
    assert!(h.query_by_label("Look Through Camera").is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    // Saved with the project.
    let back = Project::from_json(&h.state().project().to_json()).unwrap();
    assert_eq!(back.layer(cam.id).unwrap().kind, camera(&h).unwrap().kind);

    // Undo the shake, then the camera.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    let LayerKind::Camera(c) = camera(&h).unwrap().kind else { unreachable!() };
    assert!(c.shake.is_none());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(camera(&h).is_none());
}

#[test]
fn clicking_a_camera_frame_selects_it() {
    let mut h = harness(FakeLog::default());
    let id = xenify_doc::Id(h.state().project().next_id);
    let comp = h.state().comp;
    let cmd = xenify_ops::camera::add_camera(h.state().project(), comp, id, None).unwrap();
    // Zoomed in, so its frame sits inside the canvas away from other things' edges.
    h.state_mut().editor.execute("cam", cmd).unwrap();
    let cmd = xenify_ops::camera::edit(h.state().project(), id, |c| c.zoom = 2.0.into()).unwrap();
    h.state_mut().editor.execute("zoom", cmd).unwrap();
    h.state_mut().select_only(None);
    h.run_steps(2);
    let c = xenify_anim::camera_frame(h.state().comp(), id, 0.0).unwrap();
    let mid_top = kurbo::Point::new((c[0].x + c[1].x) / 2.0, c[0].y);
    let at = h.state().viewport.comp_to_screen(mid_top);
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.state().is_selected(id));
}
