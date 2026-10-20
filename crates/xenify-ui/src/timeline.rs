//! The timeline: transport controls, time ruler, layer bars and keyframes.
//!
//! - Ctrl+scroll zooms around the pointer, Shift+scroll (or horizontal scroll) pans.
//! - Click a keyframe to select it, Shift+click to add/remove, drag to move all
//!   selected keyframes, drag on empty track space (or Shift+drag anywhere,
//!   bars included) to box-select.
//! - Drag a layer bar to slide the layer in time, or its ends to trim it.
//! - Ctrl+A (pointer over the timeline) selects every keyframe; the Keys menu
//!   also keys all properties at the playhead and sets the easing of the
//!   whole selection at once.

use crate::app::{XenifyApp, KeySel};
use crate::widgets::{ACCENT, Icon, KEYFRAME, RECORD, icon_button, paint_diamond, prop_label, ruler_label, timecode};
use egui::{Color32, CursorIcon, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use xenify_doc::{Easing, Id, Layer, LayerMeta, PropPath};
use xenify_ops::Command;
use std::collections::HashSet;

const NAME_W: f32 = 220.0;
const ROW_H: f32 = 22.0;
const RULER_H: f32 = 24.0;
const KEY_R: f32 = 5.5;
const PAD: f32 = 12.0;
/// Width of the grab zone at each end of a layer bar.
const TRIM_ZONE: f32 = 6.0;

#[derive(Default)]
pub struct TimelineState {
    expanded: HashSet<Id>,
    /// Expanded layers list only their animated properties.
    animated_only: bool,
    /// Time at the left edge of the track area.
    view_start: f64,
    /// Zoom; `None` fits the whole composition.
    px_per_sec: Option<f32>,
    drag: Option<Drag>,
    /// Screen positions of the keyframes drawn this frame (for box selection).
    key_positions: Vec<(KeySel, Pos2)>,
    /// The collapsed rows' keyframe dots drawn this frame: layer, time, where.
    summary_positions: Vec<(Id, f64, Pos2)>,
    /// The easing curve editor window is open.
    pub curve_open: bool,
    /// The pointer is over the timeline (Select All then selects keyframes).
    pub hovered: bool,
    /// Where the ruler was right-clicked (its menu acts there, not at the playhead).
    menu_time: Option<f64>,
    /// Last frame's ruler: its vertical middle, and the time scale (tests aim with it).
    ruler: Option<(f32, TimeScale)>,
}

impl TimelineState {
    /// The point on the ruler showing time `t` (as last drawn).
    pub fn ruler_point(&self, t: f64) -> Option<Pos2> {
        self.ruler.map(|(y, s)| Pos2::new(s.x(t), y))
    }

    /// Where the dot for `layer`'s keyframes at `t` was drawn on its collapsed row.
    pub fn summary_point(&self, layer: Id, t: f64) -> Option<Pos2> {
        self.summary_positions.iter().find(|(l, time, _)| *l == layer && (time - t).abs() < 1e-6).map(|x| x.2)
    }
}

#[derive(Clone, Copy)]
enum Drag {
    /// Dragging the selected keyframes; `applied` is the offset moved so far.
    Keys {
        start: f64,
        applied: f64,
    },
    Layer {
        layer: Id,
        start: f64,
        applied: f64,
    },
    TrimIn {
        layer: Id,
    },
    TrimOut {
        layer: Id,
    },
    Box {
        start: Pos2,
        additive: bool,
    },
}

/// Maps time ↔ x within the track area.
#[derive(Clone, Copy)]
struct TimeScale {
    x0: f32,
    view_start: f64,
    px_per_sec: f32,
    duration: f64,
}

impl TimeScale {
    fn x(&self, t: f64) -> f32 {
        self.x0 + ((t - self.view_start) as f32) * self.px_per_sec
    }
    /// Unclamped time at `x`.
    fn t_raw(&self, x: f32) -> f64 {
        self.view_start + ((x - self.x0) / self.px_per_sec) as f64
    }
    fn t(&self, x: f32) -> f64 {
        self.t_raw(x).clamp(0.0, self.duration)
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    controls(app, ui);
    ui.separator();

    let full = ui.available_rect_before_wrap();
    // Edit ▸ Select All (Ctrl+A) picks keyframes while the pointer is here.
    app.timeline.hovered = ui.rect_contains_pointer(full);
    let track_left = full.left() + NAME_W;
    let track_w = (full.right() - track_left - 2.0 * PAD).max(10.0);
    let duration = app.comp().settings.duration;
    let fit = track_w / duration as f32;
    let tl = &mut app.timeline;
    let px_per_sec = tl.px_per_sec.unwrap_or(fit).max(fit.min(1.0));
    // Keep the view inside the composition (with the fit zoom this pins it to 0).
    let visible = (track_w / px_per_sec) as f64;
    tl.view_start = tl.view_start.clamp(0.0, (duration - visible).max(0.0));
    let scale = TimeScale { x0: track_left + PAD, view_start: tl.view_start, px_per_sec, duration };

    let track_rect = Rect::from_min_max(Pos2::new(track_left, full.top()), full.max);
    zoom_and_pan(app, ui, track_rect, scale, fit);

    // Ruler (scrubbing).
    let ruler = Rect::from_min_size(full.min, Vec2::new(full.width(), RULER_H));
    ui.allocate_rect(ruler, Sense::hover());
    let track_ruler = Rect::from_min_max(Pos2::new(track_left, ruler.top()), ruler.max);
    app.timeline.ruler = Some((track_ruler.center().y, scale));
    let r = ui.interact(track_ruler, ui.id().with("ruler"), Sense::click_and_drag());
    let now = app.time;
    r.widget_info(|| egui::WidgetInfo::slider(true, now, "Time (seconds): Home, End, comma and full stop move it"));
    if (r.clicked() || r.dragged())
        && let Some(p) = r.interact_pointer_pos()
    {
        app.playing = false;
        app.set_time(scale.t(p.x));
    }
    if r.secondary_clicked()
        && let Some(p) = r.interact_pointer_pos()
    {
        // Snap to a pause marker within a few pixels, else to the frame.
        let near = app.comp().settings.pauses.iter().copied().find(|t| (scale.x(*t) - p.x).abs() <= 5.0);
        let fps = app.fps();
        app.timeline.menu_time = Some(near.unwrap_or_else(|| (scale.t(p.x) * fps).round() / fps));
    }
    r.context_menu(|ui| {
        let t = app.timeline.menu_time.unwrap_or(app.time);
        let on_pause = app.comp().settings.pauses.iter().any(|p| (p - t).abs() < 0.5 / app.fps());
        let when = timecode(t, app.fps());
        let label = if on_pause { format!("Remove pause at {when}") } else { format!("Add pause at {when}") };
        if ui.button(label).on_hover_text("When presenting, the slide stops here until you continue").clicked() {
            crate::actions::toggle_pause_at(app, t);
            ui.close();
        }
        if ui.button(format!("Move playhead to {when}")).clicked() {
            app.playing = false;
            app.set_time(t);
            ui.close();
        }
    });

    // Background of the track area: box selection / deselect. Registered before
    // the rows so keyframes and bars (added later) take priority.
    let rows_rect = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), full.max);
    let rows_track = Rect::from_min_max(Pos2::new(track_left, ruler.bottom()), full.max);
    let bg = ui.interact(rows_track, ui.id().with("track-bg"), Sense::click_and_drag());
    bg.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Keyframe area: drag to box-select keys")
    });
    if bg.clicked() && !ui.input(|i| i.modifiers.shift) {
        app.selected_keys.clear();
    }
    if bg.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        app.timeline.drag = Some(Drag::Box { start: p, additive: ui.input(|i| i.modifiers.shift) });
    }

    app.timeline.key_positions.clear();
    app.timeline.summary_positions.clear();
    let mut rows_ui = ui.new_child(egui::UiBuilder::new().max_rect(rows_rect));
    rows_ui.set_clip_rect(rows_rect);
    egui::ScrollArea::vertical().auto_shrink(false).show(&mut rows_ui, |ui| {
        let comp = app.comp().clone();
        // Group members sit (indented) under their group and only show when
        // the group is expanded.
        let in_group = |l: &Layer| {
            l.meta.parent.and_then(|p| comp.layer(p)).is_some_and(|p| matches!(p.kind, xenify_doc::LayerKind::Group))
        };
        for layer in comp.layers.iter().rev().filter(|l| !in_group(l)) {
            layer_tree(app, ui, &comp, layer, scale, track_left, 0);
        }
        if comp.layers.is_empty() {
            ui.add_space(8.0);
            ui.weak("   No layers yet.");
        }
    });

    box_select(app, ui, &bg, rows_track);
    draw_ruler(ui, track_ruler, scale, app.fps());
    draw_pauses(ui, track_ruler, full.bottom(), scale, &app.comp().settings.pauses);

    // Playhead over ruler + rows.
    let x = scale.x(app.time);
    if x >= track_left {
        let painter = ui.painter_at(track_rect);
        painter.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, full.bottom())], Stroke::new(1.5, ACCENT));
        painter.add(egui::Shape::convex_polygon(
            vec![Pos2::new(x - 6.0, ruler.top()), Pos2::new(x + 6.0, ruler.top()), Pos2::new(x, ruler.top() + 8.0)],
            ACCENT,
            Stroke::NONE,
        ));
    }
    ui.allocate_rect(full, Sense::hover());
}

/// Ctrl+scroll zooms around the pointer; Shift+scroll / horizontal scroll pans.
fn zoom_and_pan(app: &mut XenifyApp, ui: &egui::Ui, track: Rect, scale: TimeScale, fit: f32) {
    let Some(hover) = ui.input(|i| i.pointer.hover_pos()).filter(|p| track.contains(*p)) else { return };
    let (scroll, zoom, mods) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
    let tl = &mut app.timeline;
    let mut factor = zoom;
    if mods.command {
        factor *= (scroll.y / 200.0).exp();
    }
    if (factor - 1.0).abs() > 1e-4 {
        let anchor = scale.t_raw(hover.x);
        let max = 2000.0_f32.max(fit);
        let new = (scale.px_per_sec * factor).clamp(fit, max);
        tl.px_per_sec = if (new - fit).abs() < 0.5 { None } else { Some(new) };
        tl.view_start = anchor - ((hover.x - scale.x0) / new) as f64;
        ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    } else if !mods.command {
        let dx = if mods.shift { scroll.y + scroll.x } else { scroll.x };
        if dx != 0.0 {
            tl.view_start -= (dx / scale.px_per_sec) as f64;
        }
    }
}

fn box_select(app: &mut XenifyApp, ui: &egui::Ui, bg: &egui::Response, area: Rect) {
    let Some(Drag::Box { start, additive }) = app.timeline.drag else { return };
    let Some(now) = ui.input(|i| i.pointer.latest_pos()) else { return };
    let rect = Rect::from_two_pos(start, now).intersect(area);
    let painter = ui.painter_at(area);
    painter.rect(rect, 0.0, ACCENT.gamma_multiply(0.12), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
    if bg.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
        if !additive {
            app.selected_keys.clear();
        }
        for (k, p) in &app.timeline.key_positions {
            if rect.contains(*p) && !app.selected_keys.iter().any(|s| s.is(k.layer, k.path, k.time)) {
                app.selected_keys.push(*k);
            }
        }
        if let Some(k) = app.selected_keys.first() {
            app.selected = Some(k.layer);
        }
        app.timeline.drag = None;
    }
}

/// Every keyframe of `layer`.
fn layer_keys(layer: &Layer) -> impl Iterator<Item = KeySel> + '_ {
    layer.property_paths().into_iter().flat_map(move |path| {
        let times = layer.keyframe_times(path).unwrap_or_default();
        times.into_iter().map(move |time| KeySel { layer: layer.id, path, time })
    })
}

/// Every keyframe of the composition (Ctrl+A over the timeline).
pub fn select_all_keys(app: &mut XenifyApp) {
    select_keys(app, None);
}

/// Selects every keyframe of `layers`, or of the whole composition.
fn select_keys(app: &mut XenifyApp, layers: Option<&[Id]>) {
    let comp = app.comp();
    let keys: Vec<KeySel> =
        comp.layers.iter().filter(|l| layers.is_none_or(|ids| ids.contains(&l.id))).flat_map(layer_keys).collect();
    app.status = format!("Selected {} keyframes", keys.len());
    app.selected_keys = keys;
}

/// Keys the properties of `layers` at the playhead (all of them, or only the
/// animated ones).
fn key_all(app: &mut XenifyApp, layers: &[Id], static_too: bool) {
    match Command::key_all(app.project(), layers, app.time, static_too) {
        Ok(Some(cmd)) => {
            let label = if static_too { "Key all properties" } else { "Key animated properties" };
            app.exec(label, cmd);
        }
        Ok(None) => app.status = "Nothing to key: none of the selected layers' properties are animated".into(),
        Err(e) => app.fail(e.to_string()),
    }
}

/// Sets `easing` on every key in `keys` as one undo step.
fn set_easing(app: &mut XenifyApp, keys: &[KeySel], easing: Easing) {
    let cmds = keys.iter().map(|k| Command::SetEasing { layer: k.layer, path: k.path, time: k.time, easing });
    app.exec("Set easing", Command::batch(cmds.collect()));
}

/// The Keys menu: selection, keying all properties, easing for the selection.
fn keys_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let layers = app.selected_layers();
    let has_layers = !layers.is_empty();
    let all = app.keymap.text(crate::keymap::Action::SelectAll);
    if ui.add(egui::Button::new("Select all keyframes").shortcut_text(all)).clicked() {
        select_keys(app, None);
        ui.close();
    }
    if ui.add_enabled(has_layers, egui::Button::new("Select keyframes of selected layers")).clicked() {
        select_keys(app, Some(&layers));
        ui.close();
    }
    ui.separator();
    let at = timecode(app.time, app.fps());
    let all = ui
        .add_enabled(has_layers, egui::Button::new(format!("Key all properties at {at}")))
        .on_hover_text("Adds a keyframe to every property of the selected layers, holding their current values")
        .on_disabled_hover_text("Select layers first");
    if all.clicked() {
        key_all(app, &layers, true);
        ui.close();
    }
    let animated = ui
        .add_enabled(has_layers, egui::Button::new(format!("Key animated properties at {at}")))
        .on_hover_text("Adds a keyframe to the selected layers' already-animated properties only")
        .on_disabled_hover_text("Select layers first");
    if animated.clicked() {
        key_all(app, &layers, false);
        ui.close();
    }
    ui.separator();
    let keys = app.selected_keys.clone();
    if keys.is_empty() {
        ui.weak("Select keyframes to set their easing");
        return;
    }
    ui.label(RichText::new(format!("Easing for {} keyframes", keys.len())).weak());
    // Highlight the preset only if every selected keyframe shares it.
    let easing_of = |k: &KeySel| app.project().layer(k.layer).ok()?.property(k.path).ok()?.easing_at(k.time);
    let first = keys.first().and_then(easing_of);
    let current = first.filter(|e| keys.iter().all(|k| easing_of(k) == Some(*e)));
    if let Some(easing) = crate::curve::preset_picker(ui, current) {
        set_easing(app, &keys, easing);
        ui.close();
    }
}

fn controls(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        use crate::keymap::Action;
        let tip = |app: &XenifyApp, a: Action, text: &str| app.keymap.tip(a, text);
        if ui.button("⏮").on_hover_text(tip(app, Action::GoStart, "Go to start")).clicked() {
            app.set_time(0.0);
        }
        if icon_button(ui, Icon::PrevKey, 22.0, false, true)
            .on_hover_text(tip(app, Action::PrevKey, "Previous keyframe"))
            .clicked()
        {
            app.jump_keyframe(false);
        }
        let play = if app.playing { "⏸" } else { "▶" };
        if ui
            .add(egui::Button::new(RichText::new(play).size(16.0)))
            .on_hover_text(tip(app, Action::PlayPause, "Play / pause"))
            .clicked()
        {
            app.toggle_play();
        }
        if icon_button(ui, Icon::NextKey, 22.0, false, true)
            .on_hover_text(tip(app, Action::NextKey, "Next keyframe"))
            .clicked()
        {
            app.jump_keyframe(true);
        }
        if ui.button("⏭").on_hover_text(tip(app, Action::GoEnd, "Go to end")).clicked() {
            app.set_time(f64::INFINITY);
        }
        ui.toggle_value(&mut app.looping, "🔁").on_hover_text("Loop playback");
        let rec = RichText::new("Auto Keying").color(if app.auto_key { RECORD } else { ui.visuals().text_color() });
        ui.toggle_value(&mut app.auto_key, rec).on_hover_text(
            "Auto Keying: when on, moving, scaling, rotating or editing any property adds a keyframe at the \
             current time. Remembered; also in Settings › General. A layer's ⏱ tracks all its properties",
        );
        ui.separator();
        let s = app.comp().settings.clone();
        ui.label(RichText::new(timecode(app.time, s.fps)).monospace().size(15.0).color(ACCENT));
        ui.weak(format!(
            "frame {} / {}   ·   {} fps   ·   {} long",
            (app.time * s.fps).round() as u64,
            s.frame_count().saturating_sub(1),
            s.fps,
            timecode(s.duration, s.fps)
        ))
        .on_hover_text("Times are seconds and frames: 2s 05f is 2 seconds and 5 frames");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut app.timeline.animated_only, "Only animated")
                .on_hover_text("Expanded layers list only the properties that have keyframes");
            ui.separator();
            if ui.small_button("Fit").on_hover_text("Show the whole timeline (Ctrl+scroll to zoom)").clicked() {
                app.timeline.px_per_sec = None;
                app.timeline.view_start = 0.0;
            }
            // Zoom: pixels per second (Fit when at the left end).
            let mut zoom = app.timeline.px_per_sec.unwrap_or(0.0);
            let r = ui
                .add(
                    egui::Slider::new(&mut zoom, 0.0..=2000.0)
                        .logarithmic(true)
                        .show_value(false)
                        .smallest_positive(20.0),
                )
                .on_hover_text("Zoom the timeline (or Ctrl+scroll over it); all the way left fits it");
            if r.changed() {
                app.timeline.px_per_sec = (zoom >= 20.0).then_some(zoom);
            }
            ui.weak("Zoom");
            let n = app.selected_keys.len();
            ui.menu_button("Keyframes", |ui| keys_menu(app, ui))
                .response
                .on_hover_text("Select all keyframes, key all properties at the playhead, set easing in bulk");
            let curve = ui
                .add_enabled(n > 0, egui::Button::selectable(app.timeline.curve_open, "〰 Easing"))
                .on_hover_text("Edit the easing curve of the selected keyframes")
                .on_disabled_hover_text("Select keyframes to edit their easing curve");
            if curve.clicked() {
                app.timeline.curve_open = !app.timeline.curve_open;
            }
            if n > 1 {
                ui.weak(format!("{n} keyframes selected"));
            }
        });
    });
}

/// Pause markers: where a presentation waits for the presenter.
fn draw_pauses(ui: &egui::Ui, ruler: Rect, bottom: f32, scale: TimeScale, pauses: &[f64]) {
    let painter = ui.painter_at(Rect::from_min_max(ruler.min, Pos2::new(ruler.right(), bottom)));
    let colour = Color32::from_rgb(255, 170, 60);
    for &t in pauses {
        let x = scale.x(t);
        painter.line_segment(
            [Pos2::new(x, ruler.bottom()), Pos2::new(x, bottom)],
            Stroke::new(1.0, colour.gamma_multiply(0.5)),
        );
        let flag = Rect::from_center_size(Pos2::new(x, ruler.top() + 8.0), Vec2::new(12.0, 12.0));
        painter.rect_filled(flag, 2.0, colour);
        painter.text(flag.center(), egui::Align2::CENTER_CENTER, "⏸", egui::FontId::proportional(9.0), Color32::BLACK);
    }
}

fn draw_ruler(ui: &egui::Ui, rect: Rect, scale: TimeScale, fps: f64) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
    let color = Color32::from_gray(120);
    // Pick a label step that keeps labels ≥ 60px apart.
    let steps = [1.0 / fps, 2.0 / fps, 5.0 / fps, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0];
    let step = steps.into_iter().find(|s| (*s as f32) * scale.px_per_sec >= 60.0).unwrap_or(600.0);
    let t_end = scale.t(rect.right());
    if scale.px_per_sec / fps as f32 >= 6.0 {
        let first = (scale.view_start * fps).floor().max(0.0) as u64;
        for f in first..=(t_end * fps).ceil() as u64 {
            let x = scale.x(f as f64 / fps);
            painter.line_segment(
                [Pos2::new(x, rect.bottom() - 4.0), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, color),
            );
        }
    }
    let mut t = (scale.view_start / step).floor().max(0.0) * step;
    while t <= t_end + 1e-9 {
        let x = scale.x(t);
        painter.line_segment([Pos2::new(x, rect.bottom() - 9.0), Pos2::new(x, rect.bottom())], Stroke::new(1.0, color));
        painter.text(
            Pos2::new(x + 3.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            ruler_label(t, fps),
            egui::FontId::monospace(10.0),
            color,
        );
        t += step;
    }
}

fn row(ui: &mut egui::Ui) -> Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    rect
}

/// How many of the layer's properties are animated, of how many.
fn tracked(layer: &Layer) -> (usize, usize) {
    let paths = layer.property_paths();
    let animated = paths.iter().filter(|p| layer.property(**p).is_ok_and(|p| p.is_animated())).count();
    (animated, paths.len())
}

/// A layer row's ⏱: lit when every property is animated, dim when some are.
/// Click to track them all (a keyframe on each at the playhead), or, when
/// all are, to stop animating the layer.
fn layer_stopwatch(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, rect: Rect) {
    let (animated, total) = tracked(layer);
    let all = total > 0 && animated == total;
    let color = if all {
        ACCENT
    } else if animated > 0 {
        ACCENT.gamma_multiply(0.45)
    } else {
        Color32::from_gray(110)
    };
    let r = ui.interact(rect, egui::Id::new(("layer-watch", layer.id)), Sense::click());
    ui.painter_at(rect).text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "⏱",
        egui::FontId::proportional(13.0),
        if r.hovered() { ui.visuals().strong_text_color() } else { color },
    );
    let tip = if all {
        "Every property is animated. Click to stop animating this layer (removes all its keyframes)".to_string()
    } else {
        format!(
            "Track everything: animate all of this layer's properties ({animated} of {total} are now) with a \
             keyframe at the playhead, so any change becomes animation"
        )
    };
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Track all of {}", layer.meta.name))
    });
    if !r.on_hover_text(tip).clicked() {
        return;
    }
    let time = app.time;
    let cmds: Vec<Command> = layer
        .property_paths()
        .into_iter()
        .filter_map(|path| {
            let prop = layer.property(path).ok()?;
            let value = layer.value_at(path, time).ok()?;
            match (all, prop.is_animated()) {
                (true, _) => Some(Command::SetProperty {
                    layer: layer.id,
                    path,
                    property: crate::panels::static_property(value),
                }),
                (false, false) => Some(Command::SetKeyframe { layer: layer.id, path, time, value, easing: None }),
                (false, true) => None,
            }
        })
        .collect();
    let n = cmds.len();
    let label = if all { "Stop animating layer" } else { "Track all properties" };
    if n > 0 && app.exec(label, Command::batch(cmds)) {
        app.status = if all {
            format!(
                "“{}” isn't animated any more ({} to undo)",
                layer.meta.name,
                app.keymap.text(crate::keymap::Action::Undo)
            )
        } else {
            format!("Tracking every property of “{}”: {n} more now have a keyframe", layer.meta.name)
        };
    }
}

fn layer_tree(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    comp: &xenify_doc::Composition,
    layer: &Layer,
    scale: TimeScale,
    track_left: f32,
    depth: usize,
) {
    layer_rows(app, ui, layer, scale, track_left, depth);
    if matches!(layer.kind, xenify_doc::LayerKind::Group) && app.timeline.expanded.contains(&layer.id) {
        for child in comp.layers.iter().rev().filter(|l| l.meta.parent == Some(layer.id)) {
            layer_tree(app, ui, comp, child, scale, track_left, depth + 1);
        }
    }
}

fn layer_rows(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, scale: TimeScale, track_left: f32, depth: usize) {
    let rect = row(ui);
    let selected = app.selected == Some(layer.id);
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 0.0, ACCENT.gamma_multiply(0.15));
    }

    // Expand arrow + name.
    let expanded = app.timeline.expanded.contains(&layer.id);
    let indent = depth as f32 * 14.0;
    let arrow = Rect::from_min_size(rect.min + Vec2::new(indent, 0.0), Vec2::new(20.0, ROW_H));
    let exp = ui.interact(arrow, ui.id().with(("exp", layer.id)), Sense::click());
    exp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            true,
            expanded,
            format!("{} properties", layer.meta.name),
        )
    });
    if exp.clicked() {
        if expanded {
            app.timeline.expanded.remove(&layer.id);
        } else {
            app.timeline.expanded.insert(layer.id);
        }
    }
    painter.text(
        arrow.center(),
        egui::Align2::CENTER_CENTER,
        if expanded { "⏷" } else { "⏵" },
        egui::FontId::proportional(12.0),
        Color32::GRAY,
    );
    // Track everything: one ⏱ for all the layer's properties.
    let watch = Rect::from_min_size(Pos2::new(arrow.right(), rect.top()), Vec2::new(18.0, ROW_H));
    layer_stopwatch(app, ui, layer, watch);
    let name_rect =
        Rect::from_min_max(Pos2::new(watch.right() + 2.0, rect.top()), Pos2::new(track_left, rect.bottom()));
    let name = ui.interact(name_rect, ui.id().with(("name", layer.id)), Sense::click());
    let is_sel = app.is_selected(layer.id);
    // Says where it is: the Layers panel lists the same names.
    name.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            is_sel,
            format!("Timeline: {}", layer.meta.name),
        )
    });
    if name.clicked() {
        app.selected = Some(layer.id);
        app.selected_keys.clear();
    }
    name.context_menu(|ui| {
        if ui.button("Select this layer's keyframes").clicked() {
            select_keys(app, Some(&[layer.id]));
            ui.close();
        }
        let at = timecode(app.time, app.fps());
        if ui.button(format!("Key all properties at {at}")).clicked() {
            key_all(app, &[layer.id], true);
            ui.close();
        }
        if ui.button(format!("Key animated properties at {at}")).clicked() {
            key_all(app, &[layer.id], false);
            ui.close();
        }
    });
    painter.text(
        Pos2::new(name_rect.left(), name_rect.center().y),
        egui::Align2::LEFT_CENTER,
        &layer.meta.name,
        egui::FontId::proportional(13.0),
        if layer.meta.visible { ui.visuals().text_color() } else { Color32::from_gray(100) },
    );

    let track_painter = ui.painter_at(Rect::from_min_max(Pos2::new(track_left, rect.top()), rect.max));
    layer_bar(app, ui, &track_painter, layer, rect, scale, selected);

    // Summary of all keyframe times on the collapsed row.
    if !expanded {
        let mut times: Vec<f64> =
            layer.property_paths().into_iter().flat_map(|p| layer.keyframe_times(p).unwrap_or_default()).collect();
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        for (i, t) in times.into_iter().enumerate() {
            summary_key(app, ui, &track_painter, layer, t, i, Pos2::new(scale.x(t), rect.center().y), scale);
        }
        return;
    }

    for path in layer.property_paths() {
        let Ok(prop) = layer.property(path) else { continue };
        if !prop.is_animated() && app.timeline.animated_only {
            continue;
        }
        let rect = row(ui);
        let watch = Rect::from_center_size(Pos2::new(rect.left() + indent + 26.0, rect.center().y), Vec2::splat(18.0));
        let r = ui.put(watch, crate::panels::stopwatch(prop.is_animated()));
        if r.on_hover_text(crate::panels::stopwatch_tip(prop.is_animated())).clicked() {
            crate::panels::toggle_animation(app, layer, path);
        }
        ui.painter_at(rect).text(
            Pos2::new(rect.left() + indent + 40.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            prop_label(path),
            egui::FontId::proportional(12.0),
            if prop.is_animated() { ui.visuals().text_color() } else { Color32::from_gray(110) },
        );
        let track = Rect::from_min_max(Pos2::new(track_left, rect.top()), rect.max);
        let painter = ui.painter_at(track);
        painter.line_segment(
            [Pos2::new(scale.x(0.0), rect.center().y), Pos2::new(scale.x(scale.duration), rect.center().y)],
            Stroke::new(1.0, Color32::from_gray(45)),
        );
        for (i, t) in prop.keyframe_times().into_iter().enumerate() {
            let x = scale.x(t);
            if x < track.left() - KEY_R || x > track.right() + KEY_R {
                continue;
            }
            keyframe(app, ui, &painter, layer.id, path, t, prop.easing_at(t).unwrap_or_default(), i, rect, scale);
        }
    }
}

/// Moves the selected keyframes while `r` (a keyframe being dragged) follows
/// the pointer, snapped to frames.
fn drag_selected_keys(app: &mut XenifyApp, r: &egui::Response, scale: TimeScale) {
    if r.dragged()
        && let (Some(Drag::Keys { start, applied }), Some(p)) = (app.timeline.drag, r.interact_pointer_pos())
    {
        let fps = app.fps();
        let want = ((scale.t_raw(p.x) - start) * fps).round() / fps;
        if (want - applied).abs() > 1e-9 {
            let sel: Vec<_> = app.selected_keys.iter().map(|k| (k.layer, k.path, k.time)).collect();
            let min = sel.iter().map(|k| k.2).fold(f64::INFINITY, f64::min);
            let delta = (want - applied).max(-min);
            if let Some(cmd) = Command::move_keyframes(&sel, delta)
                && app.exec_merged("Move keyframes", cmd)
            {
                for k in &mut app.selected_keys {
                    k.time += delta;
                }
                app.timeline.drag = Some(Drag::Keys { start, applied: applied + delta });
            }
        }
    }
    if r.drag_stopped() {
        app.timeline.drag = None;
    }
}

/// A dot on a collapsed layer row standing for every keyframe of the layer at
/// `time`: click selects them (Shift adds), drag moves them, right-click sets
/// their easing.
#[allow(clippy::too_many_arguments)]
fn summary_key(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    layer: &Layer,
    time: f64,
    index: usize,
    at: Pos2,
    scale: TimeScale,
) {
    let keys: Vec<KeySel> = layer_keys(layer).filter(|k| (k.time - time).abs() < 1e-6).collect();
    app.timeline.summary_positions.push((layer.id, time, at));
    let hit = Rect::from_center_size(at, Vec2::splat(KEY_R * 2.2));
    // Keyed by position in the row, not time: the time changes while dragged.
    let r = ui.interact(hit, ui.id().with(("summary", layer.id, index)), Sense::click_and_drag());
    r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{}: {} keyframes at {time:.2} s", layer.meta.name, keys.len()),
        )
    });
    let all_selected = keys.iter().all(|k| app.selected_keys.iter().any(|s| s.is(k.layer, k.path, k.time)));
    if r.clicked() || (r.drag_started() && !all_selected) {
        app.selected = Some(layer.id);
        if !ui.input(|i| i.modifiers.shift) {
            app.selected_keys.clear();
            if r.clicked() {
                app.set_time(time);
            }
        }
        for k in &keys {
            if !app.selected_keys.iter().any(|s| s.is(k.layer, k.path, k.time)) {
                app.selected_keys.push(*k);
            }
        }
    }
    if r.drag_started() {
        app.new_gesture();
        app.timeline.drag = Some(Drag::Keys { start: time, applied: 0.0 });
    }
    drag_selected_keys(app, &r, scale);
    r.context_menu(|ui| {
        let n = keys.len();
        ui.label(RichText::new(format!("Easing for {n} keyframe{} here", if n == 1 { "" } else { "s" })).weak());
        let easing_of = |k: &KeySel| layer.property(k.path).ok()?.easing_at(k.time);
        let first = keys.first().and_then(easing_of);
        let current = first.filter(|e| keys.iter().all(|k| easing_of(k) == Some(*e)));
        if let Some(easing) = crate::curve::preset_picker(ui, current) {
            set_easing(app, &keys, easing);
            ui.close();
        }
        if ui.button("Edit curve…").clicked() {
            app.selected = Some(layer.id);
            app.selected_keys = keys.clone();
            app.timeline.curve_open = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Show properties").on_hover_text("Expand the layer to see each keyframe").clicked() {
            app.timeline.expanded.insert(layer.id);
            ui.close();
        }
    });
    let hovered = r.hovered();
    r.on_hover_text(format!(
        "{} keyframe{} @ {} — click to select, right-click for easing",
        keys.len(),
        if keys.len() == 1 { "" } else { "s" },
        timecode(time, app.fps())
    ));
    let (radius, color) = if all_selected {
        (4.5, Color32::WHITE)
    } else if hovered {
        (4.5, KEYFRAME)
    } else {
        (3.5, KEYFRAME.gamma_multiply(0.85))
    };
    painter.circle_filled(at, radius, color);
}

/// The in→out bar: drag the middle to slide the layer, the ends to trim.
fn layer_bar(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    layer: &Layer,
    row: Rect,
    scale: TimeScale,
    selected: bool,
) {
    let out = layer.meta.out_point.unwrap_or(scale.duration).min(scale.duration);
    let bar = Rect::from_min_max(
        Pos2::new(scale.x(layer.meta.in_point), row.top() + 4.0),
        Pos2::new(scale.x(out), row.bottom() - 4.0),
    );
    let fill = if selected { ACCENT.gamma_multiply(0.45) } else { Color32::from_gray(60) };
    painter.rect_filled(bar, 3.0, fill);
    // Sound: its waveform along the bar.
    crate::sound_ui::waveform(app, painter, layer, bar, |t| scale.x(t));

    let r = ui.interact(bar.expand2(Vec2::new(2.0, 0.0)), ui.id().with(("bar", layer.id)), Sense::click_and_drag());
    r.widget_info(|| {
        let out = layer.meta.out_point.map_or("the end".to_string(), |t| format!("{t:.2} s"));
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            format!("{}: shown from {:.2} s to {out}", layer.meta.name, layer.meta.in_point),
        )
    });
    let pointer = r.hover_pos().or(r.interact_pointer_pos());
    let near_in = pointer.is_some_and(|p| (p.x - bar.left()).abs() <= TRIM_ZONE);
    let near_out = pointer.is_some_and(|p| (p.x - bar.right()).abs() <= TRIM_ZONE);
    if r.hovered() || r.dragged() {
        ui.ctx().set_cursor_icon(if near_in || near_out { CursorIcon::ResizeHorizontal } else { CursorIcon::Grab });
    }
    if r.clicked() {
        app.selected = Some(layer.id);
        app.selected_keys.clear();
    }
    // Shift+drag on a bar selects keyframes in a box instead of moving it.
    if r.drag_started()
        && ui.input(|i| i.modifiers.shift)
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        app.timeline.drag = Some(Drag::Box { start: p, additive: true });
    } else if r.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        app.selected = Some(layer.id);
        app.new_gesture();
        app.timeline.drag = Some(if (p.x - bar.left()).abs() <= TRIM_ZONE {
            Drag::TrimIn { layer: layer.id }
        } else if (p.x - bar.right()).abs() <= TRIM_ZONE {
            Drag::TrimOut { layer: layer.id }
        } else {
            Drag::Layer { layer: layer.id, start: scale.t_raw(p.x), applied: 0.0 }
        });
    }
    if r.dragged()
        && let Some(p) = r.interact_pointer_pos()
    {
        let fps = app.fps();
        let frame = |t: f64| (t * fps).round() / fps;
        match app.timeline.drag {
            Some(Drag::TrimIn { layer: id }) if id == layer.id => {
                let limit = layer.meta.out_point.unwrap_or(scale.duration) - 1.0 / fps;
                let t = frame(scale.t(p.x)).min(limit).max(0.0);
                if (t - layer.meta.in_point).abs() > 1e-9 {
                    let meta = LayerMeta { in_point: t, ..layer.meta.clone() };
                    app.exec_merged("Trim layer", Command::SetLayerMeta { layer: id, meta });
                }
            }
            Some(Drag::TrimOut { layer: id }) if id == layer.id => {
                let t = frame(scale.t(p.x)).max(layer.meta.in_point + 1.0 / fps);
                // Dragging to the very end means "until the end".
                let out = if t >= scale.duration - 1e-9 { None } else { Some(t) };
                if out != layer.meta.out_point {
                    let meta = LayerMeta { out_point: out, ..layer.meta.clone() };
                    app.exec_merged("Trim layer", Command::SetLayerMeta { layer: id, meta });
                }
            }
            Some(Drag::Layer { layer: id, start, applied }) if id == layer.id => {
                let want = frame(scale.t_raw(p.x) - start);
                if (want - applied).abs() > 1e-9 {
                    match Command::shift_layer(app.project(), id, want - applied) {
                        Ok(Some(cmd)) => {
                            if app.exec_merged("Move layer in time", cmd) {
                                // The actual shift may have been clamped at 0.
                                let moved =
                                    app.project().layer(id).map_or(0.0, |l| l.meta.in_point - layer.meta.in_point);
                                app.timeline.drag = Some(Drag::Layer { layer: id, start, applied: applied + moved });
                                app.selected_keys.clear();
                            }
                        }
                        Ok(None) => {}
                        Err(e) => app.fail(e.to_string()),
                    }
                }
            }
            _ => {}
        }
    }
    if r.drag_stopped() {
        app.timeline.drag = None;
    }
    r.on_hover_text(format!(
        "{} — drag to move in time, drag the ends to trim ({} → {})",
        layer.meta.name,
        timecode(layer.meta.in_point, app.fps()),
        timecode(out, app.fps())
    ));
}

#[allow(clippy::too_many_arguments)]
fn keyframe(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    layer: Id,
    path: PropPath,
    time: f64,
    easing: Easing,
    index: usize,
    row: Rect,
    scale: TimeScale,
) {
    let centre = Pos2::new(scale.x(time), row.center().y);
    let key = KeySel { layer, path, time };
    app.timeline.key_positions.push((key, centre));
    let hit = Rect::from_center_size(centre, Vec2::splat(KEY_R * 2.6));
    let r = ui.interact(hit, ui.id().with(("key", layer, path.to_string(), index)), Sense::click_and_drag());
    let is_selected = |app: &XenifyApp| app.selected_keys.iter().any(|k| k.is(layer, path, time));
    let sel_now = is_selected(app);
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, sel_now, format!("Keyframe {path} at {time:.2} s"))
    });
    let shift = ui.input(|i| i.modifiers.shift);

    if r.clicked() {
        app.selected = Some(layer);
        if shift {
            if is_selected(app) {
                app.selected_keys.retain(|k| !k.is(layer, path, time));
            } else {
                app.selected_keys.push(key);
            }
        } else {
            app.selected_keys = vec![key];
            app.set_time(time);
        }
    }
    if r.drag_started() {
        app.selected = Some(layer);
        if !is_selected(app) {
            if shift {
                app.selected_keys.push(key);
            } else {
                app.selected_keys = vec![key];
            }
        }
        app.new_gesture();
        app.timeline.drag = Some(Drag::Keys { start: time, applied: 0.0 });
    }
    drag_selected_keys(app, &r, scale);
    r.context_menu(|ui| {
        // Acts on the whole selection if this keyframe is part of it.
        let targets: Vec<KeySel> = if is_selected(app) { app.selected_keys.clone() } else { vec![key] };
        let n = targets.len();
        ui.label(
            RichText::new(if n > 1 { format!("Easing for {n} keyframes") } else { "Easing (to next keyframe)".into() })
                .weak(),
        );
        if let Some(preset) = crate::curve::preset_picker(ui, Some(easing)) {
            set_easing(app, &targets, preset);
            ui.close();
        }
        if ui.button("Edit curve…").clicked() {
            if !is_selected(app) {
                app.selected_keys = vec![key];
            }
            app.timeline.curve_open = true;
            ui.close();
        }
        ui.separator();
        if ui.button(if n > 1 { format!("Delete {n} keyframes") } else { "Delete keyframe".into() }).clicked() {
            let cmds = targets.iter().map(|k| Command::RemoveKeyframe { layer: k.layer, path: k.path, time: k.time });
            app.exec("Delete keyframes", Command::batch(cmds.collect()));
            app.fix_selection();
            ui.close();
        }
    });
    let hovered = r.hovered();
    r.on_hover_text(format!("{} @ {}", prop_label(path), timecode(time, app.fps())));

    let selected = is_selected(app);
    let fill = if selected {
        Color32::WHITE
    } else if hovered {
        KEYFRAME.gamma_multiply(1.2)
    } else {
        KEYFRAME
    };
    let stroke = Stroke::new(1.0, if selected { ACCENT } else { Color32::from_black_alpha(160) });
    match easing {
        // Hold/steps: square; eased: circle; linear: diamond (like After Effects).
        Easing::Hold | Easing::Steps { .. } => {
            painter.rect(
                Rect::from_center_size(centre, Vec2::splat(KEY_R * 1.6)),
                0.0,
                fill,
                stroke,
                StrokeKind::Middle,
            );
        }
        Easing::Bezier { .. } | Easing::Spring { .. } | Easing::Stops { .. } => {
            painter.circle(centre, KEY_R, fill, stroke);
        }
        Easing::Linear => paint_diamond(painter, centre, KEY_R + 1.0, fill, stroke),
    }
}

/// The floating easing editor for the selected keyframes (edits them all).
pub fn curve_window(app: &mut XenifyApp, ctx: &egui::Context) {
    if !app.timeline.curve_open {
        return;
    }
    let keys = app.selected_keys.clone();
    // The first selected keyframe that still exists and isn't the last one of its property.
    let current = keys.iter().find_map(|k| {
        let prop = app.project().layer(k.layer).ok()?.property(k.path).ok()?;
        let last = prop.keyframe_times().last().copied()?;
        ((k.time - last).abs() > 1e-6).then(|| prop.easing_at(k.time)).flatten()
    });
    let mut open = true;
    egui::Window::new("Easing curve").open(&mut open).resizable(false).collapsible(false).default_width(240.0).show(
        ctx,
        |ui| {
            let Some(easing) = current else {
                ui.weak("Select a keyframe that has a keyframe after it — the curve shapes the motion between them.");
                return;
            };
            if keys.len() > 1 {
                ui.weak(format!("Editing {} keyframes", keys.len()));
            }
            let edit = crate::curve::easing_editor(ui, ui.id().with("curve"), easing);
            if edit.began {
                app.new_gesture();
            }
            if let Some(easing) = edit.changed {
                let cmds =
                    keys.iter().map(|k| Command::SetEasing { layer: k.layer, path: k.path, time: k.time, easing });
                app.exec_merged("Edit easing", Command::batch(cmds.collect()));
            }
        },
    );
    if !open {
        app.timeline.curve_open = false;
    }
}
