//! Masks: add one from the menu and drag it in to crop, draw one with the
//! Mask tool, and Crop making a mask instead of an extra layer.

use super::common::*;
use egui::Key;
use egui_kittest::kittest::Queryable;
use xenify_doc::{Color, Id, Layer, ShapeItem, Vec2, as_rect};
use xenify_ui::testing::FakeLog;

fn drag(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) {
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * (i as f32 / 8.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(2);
}

/// A 400×300 white rectangle centred at 960,540, selected.
fn photo(h: &mut Harness) -> Id {
    let app = h.state_mut();
    let id = Id(app.project().next_id);
    let layer = Layer::shape(id, "Photo", vec![ShapeItem::rect(Vec2::new(400.0, 300.0), Color::WHITE)])
        .with_position(Vec2::new(960.0, 540.0));
    app.exec("add", xenify_ops::Command::AddLayer { comp: app.comp, index: None, layer });
    app.select_only(Some(id));
    h.run_steps(2);
    id
}

fn mask_rect(h: &Harness, id: Id) -> Option<xenify_doc::kurbo::Rect> {
    let l = h.state().project().layer(id).unwrap();
    as_rect(&l.meta.masks.last()?.path.value_at(0.0))
}

#[test]
fn a_rectangle_mask_from_the_menu_crops_by_dragging_its_edge() {
    let mut h = harness(FakeLog::default());
    let id = photo(&mut h);
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Mask ⏵").hover();
    h.run_steps(2);
    h.get_by_label("Add Rectangle Mask").click();
    h.run_steps(3);
    assert_eq!(mask_rect(&h, id), Some(xenify_doc::kurbo::Rect::new(-200.0, -150.0, 200.0, 150.0)));
    assert_eq!(format!("{:?}", h.state().tool), "Mask");

    // Drag the right edge's handle 100 px (composition) to the left.
    let vp = &h.state().viewport;
    let from = vp.comp_to_screen(xenify_doc::kurbo::Point::new(1160.0, 540.0));
    let to = vp.comp_to_screen(xenify_doc::kurbo::Point::new(1060.0, 540.0));
    drag(&mut h, from, to);
    let r = mask_rect(&h, id).unwrap();
    assert!((r.x1 - 100.0).abs() < 2.0 && r.x0 == -200.0 && r.y0 == -150.0, "{r:?}");
    // The selection box follows the crop.
    let b = xenify_ui::testing::world_box(h.state(), id);
    assert!((b.x1 - 1060.0).abs() < 2.0, "{b:?}");
    // One undo puts the edge back.
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(mask_rect(&h, id).unwrap().x1, 200.0);
}

#[test]
fn the_mask_tool_draws_a_mask_on_the_selected_layer() {
    let mut h = harness(FakeLog::default());
    let id = photo(&mut h);
    h.key_press(Key::U);
    h.run_steps(2);
    let vp = &h.state().viewport;
    let from = vp.comp_to_screen(xenify_doc::kurbo::Point::new(860.0, 440.0));
    let to = vp.comp_to_screen(xenify_doc::kurbo::Point::new(1060.0, 640.0));
    drag(&mut h, from, to);
    let r = mask_rect(&h, id).expect("a rectangle mask");
    assert!((r.x0 + 100.0).abs() < 3.0 && (r.x1 - 100.0).abs() < 3.0, "{r:?}");
    assert_eq!(h.state().project().layer(id).unwrap().meta.masks.len(), 1);
}

#[test]
fn crop_on_a_selected_layer_adds_a_mask_not_a_layer() {
    let mut h = harness(FakeLog::default());
    let id = photo(&mut h);
    let layers = h.state().comp().layers.len();
    // Crop is an action now (Layer › Crop Selection…), not a tool.
    h.get_by_label("Layer").click();
    h.run_steps(2);
    h.get_by_label("Crop Selection…").click();
    h.run_steps(2);
    assert_eq!(format!("{:?}", h.state().tool), "Crop");
    // The crop box starts as the layer's box; pull the left edge in by 150.
    let vp = &h.state().viewport;
    let from = vp.comp_to_screen(xenify_doc::kurbo::Point::new(760.0, 540.0));
    let to = vp.comp_to_screen(xenify_doc::kurbo::Point::new(910.0, 540.0));
    drag(&mut h, from, to);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().comp().layers.len(), layers, "no clipping layer added");
    let l = h.state().project().layer(id).unwrap();
    assert_eq!(l.meta.masks.len(), 1);
    assert_eq!(l.meta.masks[0].name, "Crop");
    let r = mask_rect(&h, id).unwrap();
    assert!((r.x0 + 50.0).abs() < 2.0, "{r:?}");
}
