//! The colour panel: fill and stroke swatches (Illustrator style), a colour
//! picker, swatches, and the eyedroppers.

use crate::app::{XenifyApp, PaintSlot};
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};
use xenify_anim::{Scene, SceneContent, SceneLayer};
use xenify_doc::{Color, Fill, Id, LayerKind, PropPath, ShapeField, ShapeItem, Value};
use xenify_ops::Command;

const SWATCHES: [&str; 24] = [
    "#000000", "#3a3a3a", "#7a7a7a", "#b5b5b5", "#e6e6e6", "#ffffff", "#ef476f", "#f78c6b", "#ffd166", "#06d6a0",
    "#118ab2", "#073b4c", "#8338ec", "#3a86ff", "#ff006e", "#fb5607", "#ffbe0b", "#8ac926", "#1982c4", "#6a4c93",
    "#e63946", "#f1faee", "#a8dadc", "#457b9d",
];

pub fn to_color32(c: Color) -> Color32 {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(b(c.r), b(c.g), b(c.b), b(c.a))
}

/// Layers a colour change reaches: the selection, with groups expanded.
pub(crate) fn targets(app: &XenifyApp) -> Vec<Id> {
    let mut out = Vec::new();
    for id in app.selected_layers() {
        out.push(id);
        out.extend(xenify_anim::descendants(app.comp(), id));
    }
    out.dedup();
    out
}

/// Sets the fill or stroke colour (`None` = no fill / no stroke) and applies it
/// to the selection, merged into the current gesture's undo step.
pub fn set_color(app: &mut XenifyApp, slot: PaintSlot, color: Option<Color>) {
    app.paint.set(slot, color);
    let mut cmds = Vec::new();
    for id in targets(app) {
        let Ok(layer) = app.project().layer(id) else { continue };
        if layer.meta.locked {
            continue;
        }
        match &layer.kind {
            LayerKind::Text(_) if slot == PaintSlot::Fill => {
                if let Some(c) = color
                    && let Ok(cmd) = Command::edit_value(
                        app.project(),
                        id,
                        PropPath::TextColor,
                        app.time,
                        Value::Color(c),
                        app.auto_key,
                    )
                {
                    cmds.push(cmd);
                }
            }
            LayerKind::Shape { shapes } => {
                for (index, shape) in shapes.iter().enumerate() {
                    cmds.extend(shape_color_command(app, id, index, shape, slot, color));
                }
            }
            _ => {}
        }
    }
    if !cmds.is_empty() {
        let label = if slot == PaintSlot::Fill { "Change fill" } else { "Change stroke" };
        app.exec_merged(label, Command::batch(cmds));
    }
}

fn shape_color_command(
    app: &XenifyApp,
    layer: Id,
    index: usize,
    shape: &ShapeItem,
    slot: PaintSlot,
    color: Option<Color>,
) -> Option<Command> {
    let set_shape = |new: ShapeItem| Some(Command::SetShape { layer, index, shape: new });
    let edit = |field, c| {
        Command::edit_value(
            app.project(),
            layer,
            PropPath::shape(index, field),
            app.time,
            Value::Color(c),
            app.auto_key,
        )
        .ok()
    };
    match (slot, color) {
        (PaintSlot::Fill, Some(c)) => match &shape.fill {
            Some(f) if f.gradient.is_none() => edit(ShapeField::FillColor, c),
            _ => set_shape(ShapeItem { fill: Some(Fill::solid(c)), ..shape.clone() }),
        },
        (PaintSlot::Fill, None) => {
            shape.fill.is_some().then(|| ShapeItem { fill: None, ..shape.clone() }).and_then(set_shape)
        }
        (PaintSlot::Stroke, Some(c)) => match &shape.stroke {
            Some(_) => edit(ShapeField::StrokeColor, c),
            None => {
                let styled = app.paint.apply_to(shape.clone());
                set_shape(ShapeItem {
                    stroke: styled.stroke.map(|mut s| {
                        s.color = c.into();
                        s
                    }),
                    ..shape.clone()
                })
            }
        },
        (PaintSlot::Stroke, None) => {
            shape.stroke.is_some().then(|| ShapeItem { stroke: None, ..shape.clone() }).and_then(set_shape)
        }
    }
}

/// Sets the stroke width of the selection's strokes.
fn set_stroke_width(app: &mut XenifyApp, width: f64) {
    app.paint.stroke_width = width;
    let mut cmds = Vec::new();
    for id in targets(app) {
        let Ok(layer) = app.project().layer(id) else { continue };
        for (index, shape) in layer.shapes().iter().enumerate() {
            if shape.stroke.is_some()
                && let Ok(cmd) = Command::edit_value(
                    app.project(),
                    id,
                    PropPath::shape(index, ShapeField::StrokeWidth),
                    app.time,
                    Value::Scalar(width),
                    app.auto_key,
                )
            {
                cmds.push(cmd);
            }
        }
    }
    if !cmds.is_empty() {
        app.exec_merged("Change stroke width", Command::batch(cmds));
    }
}

/// Shift+X: swaps the fill and stroke colours.
pub fn swap(app: &mut XenifyApp) {
    let (fill, stroke) = (app.paint.fill, app.paint.stroke);
    app.new_gesture();
    set_color(app, PaintSlot::Fill, stroke);
    set_color(app, PaintSlot::Stroke, fill);
}

/// D: white fill, black stroke.
pub fn reset(app: &mut XenifyApp) {
    app.new_gesture();
    set_color(app, PaintSlot::Fill, Some(Color::WHITE));
    set_color(app, PaintSlot::Stroke, Some(Color::BLACK));
}

/// When the selection changes, the swatches show the selected layer's colours
/// (so new shapes continue in the same style, like Illustrator).
pub fn sync_from_selection(app: &mut XenifyApp, last: &mut Option<Id>) {
    if app.selected == *last {
        return;
    }
    *last = app.selected;
    let Some(layer) = app.selected.and_then(|id| app.project().layer(id).ok()).cloned() else { return };
    let t = app.time;
    match &layer.kind {
        LayerKind::Text(text) => app.paint.fill = Some(text.color.value_at(t)),
        LayerKind::Shape { shapes } => {
            if let Some(s) = shapes.first() {
                app.paint.fill = s.fill.as_ref().map(|f| f.color.value_at(t));
                app.paint.stroke = s.stroke.as_ref().map(|s| s.color.value_at(t));
                if let Some(st) = &s.stroke {
                    app.paint.stroke_width = st.width.value_at(t);
                }
            }
        }
        _ => {}
    }
}

/// The fill/stroke swatch pair. Click one to make it active; the small arrow swaps them.
pub fn swatch_pair(app: &mut XenifyApp, ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let box_size = size * 0.62;
    let fill_rect = Rect::from_min_size(rect.min, vec2(box_size, box_size));
    let stroke_rect = Rect::from_min_size(rect.max - vec2(box_size, box_size), vec2(box_size, box_size));
    let fill_resp = ui.interact(fill_rect, ui.id().with("fill-swatch"), Sense::click());
    let stroke_resp = ui.interact(stroke_rect, ui.id().with("stroke-swatch"), Sense::click());
    let painter = ui.painter();
    let draw_fill = |painter: &egui::Painter| {
        paint_swatch(painter, fill_rect, app.paint.fill);
        let active = app.paint.active == PaintSlot::Fill;
        painter.rect_stroke(
            fill_rect,
            2.0,
            Stroke::new(if active { 2.0 } else { 1.0 }, if active { Color32::WHITE } else { Color32::from_gray(90) }),
            StrokeKind::Outside,
        );
    };
    let draw_stroke = |painter: &egui::Painter| {
        paint_swatch(painter, stroke_rect, app.paint.stroke);
        // Hollow middle: it's an outline.
        let inner = stroke_rect.shrink(box_size * 0.28);
        painter.rect_filled(inner, 1.0, ui.visuals().panel_fill);
        painter.rect_stroke(inner, 1.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Outside);
        let active = app.paint.active == PaintSlot::Stroke;
        painter.rect_stroke(
            stroke_rect,
            2.0,
            Stroke::new(if active { 2.0 } else { 1.0 }, if active { Color32::WHITE } else { Color32::from_gray(90) }),
            StrokeKind::Outside,
        );
    };
    // The active one is drawn on top.
    if app.paint.active == PaintSlot::Fill {
        draw_stroke(painter);
        draw_fill(painter);
    } else {
        draw_fill(painter);
        draw_stroke(painter);
    }
    // Swap arrow in the top-right corner.
    let swap_rect =
        Rect::from_min_size(Pos2::new(rect.max.x - size * 0.34, rect.min.y), vec2(size * 0.34, size * 0.34));
    let swap_resp = ui.interact(swap_rect, ui.id().with("swap-swatch"), Sense::click());
    // Names for screen readers (these are painted, not labelled widgets).
    fill_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Fill"));
    stroke_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Stroke"));
    swap_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Swap fill and stroke"));
    let col = if swap_resp.hovered() { Color32::WHITE } else { Color32::from_gray(170) };
    // A curved arrow with a head at each end: one pointing at the fill
    // (left), one at the stroke (down).
    let inner = swap_rect.shrink(swap_rect.width() * 0.18);
    let (centre, r) = (inner.left_bottom(), inner.width().min(inner.height()));
    let arc: Vec<Pos2> = (0..=10)
        .map(|i| {
            let t = std::f32::consts::FRAC_PI_2 * i as f32 / 10.0;
            centre + vec2(r * t.sin(), -r * t.cos())
        })
        .collect();
    let (start, end) = (arc[0], arc[10]);
    painter.add(egui::Shape::line(arc, Stroke::new(1.4, col)));
    let head = (r * 0.42).max(3.0);
    for (tip, back) in [(start, vec2(1.0, 0.0)), (end, vec2(0.0, -1.0))] {
        let side = vec2(-back.y, back.x);
        painter.add(egui::Shape::convex_polygon(
            vec![tip - back * 0.5, tip + back * head + side * head * 0.6, tip + back * head - side * head * 0.6],
            col,
            Stroke::NONE,
        ));
    }
    let fill_resp =
        fill_resp.on_hover_text("Fill: click to make it active, click again to choose its colour (X switches)");
    let stroke_resp = stroke_resp
        .on_hover_text("Stroke / outline: click to make it active, click again to choose its colour (X switches)");
    // Clicking the swatch that's already active opens its picker.
    let reopen = (fill_resp.clicked() && app.paint.active == PaintSlot::Fill)
        || (stroke_resp.clicked() && app.paint.active == PaintSlot::Stroke);
    let popup_id = ui.id().with("swatch-picker");
    let swap_tip = app.keymap.tip(crate::keymap::Action::SwapFillStroke, "Swap fill and stroke");
    if swap_resp.on_hover_text(swap_tip).clicked() {
        swap(app);
    } else if fill_resp.clicked() {
        app.paint.active = PaintSlot::Fill;
    } else if stroke_resp.clicked() {
        app.paint.active = PaintSlot::Stroke;
    }
    let double = fill_resp.double_clicked() || stroke_resp.double_clicked();
    if reopen && !double && egui::Popup::is_id_open(ui.ctx(), popup_id) {
        egui::Popup::close_id(ui.ctx(), popup_id);
    } else if reopen || double {
        egui::Popup::open_id(ui.ctx(), popup_id);
        app.new_gesture();
    }
    let anchor = if app.paint.active == PaintSlot::Fill { &fill_resp } else { &stroke_resp };
    egui::Popup::from_response(anchor)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let slot = app.paint.active;
            ui.label(
                egui::RichText::new(if slot == PaintSlot::Fill {
                    "Choose the fill colour"
                } else {
                    "Choose the stroke colour"
                })
                .strong(),
            );
            if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
                app.new_gesture();
            }
            picker(app, ui, slot);
            swatch_grid(app, ui, &SWATCHES.iter().filter_map(|h| Color::from_hex(h)).collect::<Vec<_>>());
        });
    // A double-click's second click lands outside the popup its first click
    // opened: keep it open.
    if double {
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
}

/// The colour picker for `slot`'s solid colour.
fn picker(app: &mut XenifyApp, ui: &mut egui::Ui, slot: PaintSlot) {
    let mut c = app.paint.get(slot).unwrap_or(Color::WHITE);
    let picked = crate::color_picker::picker(ui, slot == PaintSlot::Fill, &mut c);
    if picked.changed {
        set_color(app, slot, Some(c));
    }
    if picked.released && app.paint.get(slot).is_some() {
        app.paint.remember(c);
    }
}

fn paint_swatch(painter: &egui::Painter, rect: Rect, color: Option<Color>) {
    match color {
        Some(c) => {
            // Checkerboard under translucent colours.
            if c.a < 1.0 {
                let n = 4;
                let cell = rect.width() / n as f32;
                for y in 0..n {
                    for x in 0..n {
                        let g = if (x + y) % 2 == 0 { 200 } else { 140 };
                        let r =
                            Rect::from_min_size(rect.min + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell));
                        painter.rect_filled(r, 0.0, Color32::from_gray(g));
                    }
                }
            }
            painter.rect_filled(rect, 2.0, to_color32(c));
        }
        None => {
            painter.rect_filled(rect, 2.0, Color32::WHITE);
            painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(2.0, Color32::RED));
        }
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    // One undo step per click/drag in the panel.
    if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
        app.new_gesture();
    }
    let slot = app.paint.active;
    let current = app.paint.get(slot);
    ui.horizontal(|ui| {
        swatch_pair(app, ui, 56.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(if slot == PaintSlot::Fill { "Fill" } else { "Stroke" }).strong());
            ui.horizontal(|ui| {
                if ui.selectable_label(current.is_none(), "None").on_hover_text("No fill / no stroke").clicked() {
                    set_color(app, slot, None);
                }
                let armed = app.pick_color;
                if crate::widgets::icon_button(ui, crate::widgets::Icon::Eyedropper, 22.0, armed, true)
                    .on_hover_text(format!(
                        "Pick a colour from the canvas into the {} only",
                        if slot == PaintSlot::Fill { "fill" } else { "stroke" }
                    ))
                    .clicked()
                {
                    app.pick_color = !armed;
                }
            });
        });
    });
    ui.add_space(6.0);

    // Solid / linear / radial first; the picker below is for solid colours
    // (the gradient editor has its own, for the chosen stop).
    ui.separator();
    let gradient = crate::gradient_ui::section(app, ui, slot == PaintSlot::Stroke);
    ui.add_space(6.0);
    if !gradient {
        picker(app, ui, slot);
        ui.add_space(6.0);
    }
    swatch_grid(app, ui, &SWATCHES.iter().filter_map(|h| Color::from_hex(h)).collect::<Vec<_>>());
    if !app.paint.recent.is_empty() {
        ui.label(egui::RichText::new("Recent").small().weak());
        let recent = app.paint.recent.clone();
        swatch_grid(app, ui, &recent);
    }
}

fn swatch_grid(app: &mut XenifyApp, ui: &mut egui::Ui, colors: &[Color]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for c in colors {
            let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, c.to_hex()));
            paint_swatch(ui.painter(), rect, Some(*c));
            if resp.hovered() {
                ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
            }
            if resp.on_hover_text(c.to_hex()).clicked() {
                app.new_gesture();
                set_color(app, app.paint.active, Some(*c));
                app.paint.remember(*c);
            }
        }
    });
}

/// What the eyedropper finds at a point.
pub struct Sample {
    /// The clicked layer and shape, when a shape was hit.
    pub fill: Option<Color>,
    pub stroke: Option<(Color, f64)>,
    /// The single colour visible right under the pointer.
    pub visible: Color,
}

/// Samples the top-most thing at `point` (composition space): the layer's
/// fill and stroke, and the one colour showing there (a stroke if the point is
/// on it, else the fill — gradients evaluated at the point — else the background).
pub fn sample(
    scene: &Scene,
    point: kurbo::Point,
    tolerance: f64,
    measure: &mut dyn xenify_anim::TextMeasure,
) -> Sample {
    let background = Sample { fill: None, stroke: None, visible: scene.background };
    let Some(id) = xenify_anim::hit_test(scene, point, tolerance, measure) else { return background };
    let Some(layer) = find(&scene.layers, id) else { return background };
    sample_layer(layer, point, tolerance, measure).unwrap_or(background)
}

fn find(layers: &[SceneLayer], id: Id) -> Option<&SceneLayer> {
    layers.iter().find(|l| l.id == id)
}

fn sample_layer(
    layer: &SceneLayer,
    point: kurbo::Point,
    tolerance: f64,
    measure: &mut dyn xenify_anim::TextMeasure,
) -> Option<Sample> {
    let local = layer.transform.inverse() * point;
    let scale = layer.transform.determinant().abs().sqrt().max(1e-9);
    match &layer.content {
        SceneContent::Text(t) => Some(Sample { fill: Some(t.color), stroke: None, visible: t.color }),
        SceneContent::Image(img) => {
            // The pixel under the point (layer space is one unit per image pixel).
            let data = xenify_render::images::decoded(img)?;
            let x = (local.x * data.width as f64 / img.width.max(1) as f64).floor();
            let y = (local.y * data.height as f64 / img.height.max(1) as f64).floor();
            if x < 0.0 || y < 0.0 || x >= data.width as f64 || y >= data.height as f64 {
                return None;
            }
            let i = (y as usize * data.width as usize + x as usize) * 4;
            let px = data.data.data().get(i..i + 4)?;
            let [r, g, b, a] = [px[0], px[1], px[2], px[3]].map(|v| v as f64 / 255.0);
            let c = Color::rgba(r, g, b, a);
            Some(Sample { fill: Some(c), stroke: None, visible: c })
        }
        SceneContent::Precomp { layers, .. } | SceneContent::Group(layers) => {
            let inner = layers.iter().rev().find(|l| xenify_anim::layer_contains(l, point, tolerance, measure))?;
            sample_layer(inner, point, tolerance, measure)
        }
        SceneContent::Shapes(items) => {
            use kurbo::{ParamCurveNearest, Shape};
            // Top-most item first.
            for item in items.iter().rev() {
                let on_stroke = item.stroke.as_ref().is_some_and(|s| {
                    let half = s.width / 2.0 + tolerance / scale;
                    item.path.segments().any(|seg| seg.nearest(local, 0.1).distance_sq <= half * half)
                });
                let inside = item.fill.is_some() && {
                    let w = item.path.winding(local);
                    match item.fill_rule {
                        xenify_doc::FillRule::NonZero => w != 0,
                        xenify_doc::FillRule::EvenOdd => w % 2 != 0,
                    }
                };
                if !(on_stroke || inside) {
                    continue;
                }
                let fill_here = match (&item.gradient, item.fill) {
                    (Some(g), _) => Some(gradient_at(g, local)),
                    (None, f) => f,
                };
                let stroke = item.stroke.as_ref().map(|s| (s.color, s.width));
                let visible = if on_stroke { stroke.map(|s| s.0) } else { fill_here }.unwrap_or(Color::BLACK);
                return Some(Sample { fill: fill_here, stroke, visible });
            }
            None
        }
    }
}

fn gradient_at(g: &xenify_doc::Gradient, p: kurbo::Point) -> Color {
    let d = g.end - g.start;
    let t = match g.kind {
        xenify_doc::GradientKind::Linear => (p.to_vec2() - g.start).dot(d) / d.hypot2().max(1e-9),
        xenify_doc::GradientKind::Radial => (p.to_vec2() - g.start).hypot() / d.hypot().max(1e-9),
    };
    g.color_at(t.clamp(0.0, 1.0))
}

/// The eyedropper tool: takes fill and stroke from the clicked shape into the
/// panel and applies both to the selection (if anything else is selected).
pub fn eyedropper_tool(app: &mut XenifyApp, s: Sample) {
    app.new_gesture();
    app.paint.fill = s.fill;
    if let Some((c, w)) = s.stroke {
        app.paint.stroke = Some(c);
        app.paint.stroke_width = w;
    } else {
        app.paint.stroke = None;
    }
    if !app.selected_layers().is_empty() {
        set_color(app, PaintSlot::Fill, s.fill);
        set_color(app, PaintSlot::Stroke, s.stroke.map(|s| s.0));
        if let Some((_, w)) = s.stroke {
            set_stroke_width(app, w);
        }
    }
    if let Some(c) = s.fill {
        app.paint.remember(c);
    }
    app.status = format!(
        "Picked fill {} and stroke {}.",
        s.fill.map_or("none".into(), |c| c.to_hex()),
        s.stroke.map_or("none".into(), |c| c.0.to_hex())
    );
}

/// The panel's eyedropper: only the active slot takes the colour under the pointer.
pub fn pick_into_active(app: &mut XenifyApp, s: Sample) {
    app.new_gesture();
    let slot = app.paint.active;
    set_color(app, slot, Some(s.visible));
    app.paint.remember(s.visible);
    app.pick_color = false;
    app.status =
        format!("Picked {} into the {}.", s.visible.to_hex(), if slot == PaintSlot::Fill { "fill" } else { "stroke" });
}
