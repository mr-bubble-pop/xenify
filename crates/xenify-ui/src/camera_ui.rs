//! Camera layers in the UI: Layer › New › Camera, the Properties section
//! (zoom, follow, shake, which camera is used), the camera frames on the
//! canvas, and View › Look Through Camera.

use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, RichText, Stroke};
use xenify_doc::{CameraField, CameraLayer, CameraShake, Id, Layer, LayerKind, PropPath};
use kurbo::Point;

/// Layer › New › Camera.
pub fn new_camera(app: &mut XenifyApp) {
    let id = Id(app.project().next_id);
    let Ok(cmd) = xenify_ops::camera::add_camera(app.project(), app.comp, id, None) else { return };
    if app.exec("Add camera", cmd) {
        app.select_only(Some(id));
        app.status = "Added a camera framing the canvas. Move, turn or zoom it (keyframes work); View › Look \
                      Through Camera shows what it sees."
            .into();
    }
}

/// Whether the current canvas has a camera.
pub fn has_camera(app: &XenifyApp) -> bool {
    app.comp().layers.iter().any(|l| matches!(l.kind, LayerKind::Camera(_)))
}

/// Looking through the camera (and there is one).
pub fn looking_through(app: &XenifyApp) -> bool {
    app.look_through_camera && has_camera(app)
}

/// View › Look Through Camera.
pub fn view_menu_item(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let enabled = has_camera(app);
    let mut on = app.look_through_camera && enabled;
    if ui
        .add_enabled(enabled, egui::Checkbox::new(&mut on, "Look Through Camera"))
        .on_hover_text("Show the canvas as the active camera sees it (as it plays, presents and exports)")
        .on_disabled_hover_text("Add a camera first (Layer › New › Camera)")
        .clicked()
    {
        app.look_through_camera = on;
        ui.close();
    }
}

/// Properties › Camera.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let LayerKind::Camera(cam) = &layer.kind else { return };
    egui::CollapsingHeader::new(RichText::new("Camera").strong()).default_open(true).show(ui, |ui| {
        let active = xenify_doc::active_camera(&app.comp().layers, app.time).map(|l| l.id);
        ui.horizontal(|ui| {
            if active == Some(layer.id) {
                ui.label(RichText::new("● Active camera").color(ACCENT))
                    .on_hover_text("The topmost visible camera is the one that's used");
            } else {
                ui.weak("Not the active camera");
                if ui.button("Make Active").on_hover_text("Move it above the other cameras and show it").clicked()
                    && let Ok(Some(cmd)) = xenify_ops::camera::make_active(app.project(), layer.id)
                {
                    app.exec("Make camera active", cmd);
                }
            }
        });
        let mut look = app.look_through_camera;
        if ui.checkbox(&mut look, "Look through camera").on_hover_text("View › Look Through Camera").changed() {
            app.look_through_camera = look;
        }
        egui::Grid::new(("camera", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            crate::panels::prop_row(app, ui, layer, PropPath::Camera(CameraField::Zoom));
        });
        ui.weak("Follow a layer: set Parent above to it. Keyframe Position and Rotation to move and turn.");
        shake_editor(app, ui, layer, cam);
    });
}

fn shake_editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, cam: &CameraLayer) {
    let mut on = cam.shake.is_some();
    if ui
        .checkbox(&mut on, "Shake")
        .on_hover_text("Handheld wobble, or a jolt that dies away (explosions add one)")
        .changed()
    {
        let shake = on.then(|| CameraShake { start: app.time, ..CameraShake::default() });
        if let Ok(cmd) = xenify_ops::camera::set_shake(app.project(), layer.id, shake) {
            app.exec(if on { "Add camera shake" } else { "Remove camera shake" }, cmd);
        }
        return;
    }
    let Some(shake) = &cam.shake else { return };
    egui::Grid::new(("camera-shake", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
        crate::panels::prop_row_named(app, ui, layer, PropPath::Camera(CameraField::ShakeAmount), "Amount (px)");
        crate::panels::prop_row_named(app, ui, layer, PropPath::Camera(CameraField::ShakeRotation), "Turn (°)");
    });
    let mut s = shake.clone();
    let mut begin = false;
    let mut changed = false;
    egui::Grid::new(("camera-shake-2", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        let mut row = |ui: &mut egui::Ui,
                       label: &str,
                       tip: &str,
                       v: &mut f64,
                       range: std::ops::RangeInclusive<f64>,
                       suffix: &str| {
            ui.label(label).on_hover_text(tip);
            let r = ui.add(egui::DragValue::new(v).range(range).speed(0.05).suffix(suffix));
            begin |= r.drag_started();
            changed |= r.changed();
            ui.end_row();
        };
        row(ui, "Frequency", "Shakes per second", &mut s.frequency, 0.1..=60.0, " /s");
        row(ui, "Starts at", "When the shake begins", &mut s.start, 0.0..=3600.0, " s");
        row(ui, "Dies away", "How fast it fades: 0 keeps shaking, 3 is a short jolt", &mut s.decay, 0.0..=50.0, "");
        let mut seed = s.seed as f64;
        row(ui, "Variation", "A different random pattern", &mut seed, 0.0..=9999.0, "");
        s.seed = seed.round() as u64;
    });
    if begin {
        app.new_gesture();
    }
    if changed && let Ok(cmd) = xenify_ops::camera::set_shake(app.project(), layer.id, Some(s)) {
        app.exec_merged("Change camera shake", cmd);
    }
}

/// The camera whose frame outline is under `at` (composition space), if any.
pub fn hit(app: &XenifyApp, at: Point, tolerance: f64) -> Option<Id> {
    let comp = app.comp();
    comp.layers.iter().rev().filter(|l| matches!(l.kind, LayerKind::Camera(_)) && !l.meta.locked).find_map(|l| {
        let c = xenify_anim::camera_frame(comp, l.id, app.time)?;
        let near = (0..4).any(|i| {
            let seg = kurbo::Line::new(c[i], c[(i + 1) % 4]);
            let p = kurbo::ParamCurveNearest::nearest(&seg, at, 1e-6);
            p.distance_sq.sqrt() <= tolerance
        });
        near.then_some(l.id)
    })
}

/// Draws every camera's frame (when not looking through one).
pub fn draw_frames(app: &XenifyApp, painter: &egui::Painter, to_screen: impl Fn(Point) -> Pos2) {
    let comp = app.comp();
    let active = xenify_doc::active_camera(&comp.layers, app.time).map(|l| l.id);
    for l in comp.layers.iter().filter(|l| matches!(l.kind, LayerKind::Camera(_)) && l.meta.visible) {
        let Some(c) = xenify_anim::camera_frame(comp, l.id, app.time) else { continue };
        let pts: Vec<Pos2> = c.iter().map(|p| to_screen(*p)).collect();
        let colour = if app.is_selected(l.id) {
            ACCENT
        } else if Some(l.id) == active {
            Color32::from_rgb(120, 200, 255)
        } else {
            Color32::from_gray(140)
        };
        let dash = egui::Shape::dashed_line(&[pts.clone(), vec![pts[0]]].concat(), Stroke::new(1.5, colour), 8.0, 5.0);
        painter.extend(dash);
        let name = if Some(l.id) == active {
            format!("📷 {} (active)", l.meta.name)
        } else {
            format!("📷 {}", l.meta.name)
        };
        painter.text(
            pts[0] + egui::vec2(4.0, 4.0),
            egui::Align2::LEFT_TOP,
            name,
            egui::FontId::proportional(12.0),
            colour,
        );
    }
}

/// A note over the canvas while looking through the camera (editing is off).
pub fn draw_banner(painter: &egui::Painter, rect: egui::Rect) {
    let text = "Looking through the camera. Turn off View › Look Through Camera to edit.";
    let pos = rect.center_top() + egui::vec2(0.0, 10.0);
    let galley = painter.layout_no_wrap(text.into(), egui::FontId::proportional(13.0), Color32::WHITE);
    let r = egui::Rect::from_center_size(
        pos + egui::vec2(0.0, galley.size().y / 2.0 + 4.0),
        galley.size() + egui::vec2(16.0, 8.0),
    );
    painter.rect_filled(r, 4.0, Color32::from_black_alpha(190));
    painter.galley(r.min + egui::vec2(8.0, 4.0), galley, Color32::WHITE);
}
