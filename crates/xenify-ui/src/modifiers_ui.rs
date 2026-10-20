//! The Modifiers section of Properties and Layer › Modifiers: arrays of
//! copies (in a row, around a centre, in a grid) and outline changes
//! (jitter, zig zag, twist…), applied live, in order.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{Id, Layer, LayerKind, LayerMeta, Modifier, ModifierKind, PropPath, Vec2};
use xenify_ops::Command;
use kurbo::Rect;

/// Whether modifiers can apply to a layer (groups repeat as a whole).
pub fn takes_modifiers(l: &Layer) -> bool {
    !matches!(l.kind, LayerKind::Null)
}

/// The ready-made modifiers, as offered in the menus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Preset {
    Row,
    Radial,
    Grid,
    Jitter,
    ZigZag,
    Wave,
    PuckerBloat,
    Twist,
    Smooth,
    Subdivide,
    Simplify,
    OffsetPath,
    Mirror,
    PathArray,
    Blend,
    ShatterOut,
    ShatterIn,
}

impl Preset {
    /// Copies.
    pub const ARRAYS: [Preset; 5] = [Preset::Row, Preset::Radial, Preset::Grid, Preset::Mirror, Preset::PathArray];
    /// Between two layers.
    pub const PAIRS: [Preset; 1] = [Preset::Blend];
    /// Breaking apart.
    pub const BREAK: [Preset; 2] = [Preset::ShatterOut, Preset::ShatterIn];
    /// Outline changes.
    pub const SHAPES: [Preset; 9] = [
        Preset::Jitter,
        Preset::ZigZag,
        Preset::Wave,
        Preset::PuckerBloat,
        Preset::Twist,
        Preset::Smooth,
        Preset::Subdivide,
        Preset::Simplify,
        Preset::OffsetPath,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Row => "Array",
            Preset::Radial => "Radial Array",
            Preset::Grid => "Grid Array",
            Preset::Jitter => "Jitter",
            Preset::ZigZag => "Zig Zag",
            Preset::Wave => "Wave",
            Preset::PuckerBloat => "Pucker & Bloat",
            Preset::Twist => "Twist",
            Preset::Smooth => "Smooth",
            Preset::Subdivide => "Subdivide",
            Preset::Simplify => "Simplify",
            Preset::OffsetPath => "Offset Path",
            Preset::Mirror => "Mirror",
            Preset::PathArray => "Array Along Path",
            Preset::Blend => "Blend",
            Preset::ShatterOut => "Shatter Out",
            Preset::ShatterIn => "Shatter In",
        }
    }

    /// Whether it needs another layer (a path to follow, a shape to blend into).
    pub fn needs_target(self) -> bool {
        matches!(self, Preset::PathArray | Preset::Blend)
    }

    pub fn tip(self) -> &'static str {
        match self {
            Preset::Row => "Copies in a row, each one step on: set how far they move, turn, grow and fade",
            Preset::Radial => "Copies turned evenly around a centre (set the centre as the array's Origin)",
            Preset::Grid => "Copies in rows and columns (two arrays: one across, one down)",
            Preset::Jitter => "Moves points by random amounts (roughen); animate the seed to make it boil",
            Preset::ZigZag => "Sharp ridges along every edge",
            Preset::Wave => "Smooth waves along every edge",
            Preset::PuckerBloat => "Pucker (below 0) pulls edges in to a star; bloat (above 0) puffs them out",
            Preset::Twist => "Turns the middle more than the edges, like a whirlpool",
            Preset::Smooth => "Rounds every corner",
            Preset::Subdivide => "More points, same look (so later changes have points to move)",
            Preset::Simplify => "Fewer points, nearly the same look",
            Preset::OffsetPath => "Grows the shape outwards (or inwards with a negative distance)",
            Preset::Mirror => "A mirror image across an axis (drag the diamond to move the axis)",
            Preset::PathArray => {
                "Copies spread along another shape's outline (select this layer, then the path: Shift+click)"
            }
            Preset::Blend => "Steps that morph this shape into another (select it, then the other: Shift+click)",
            Preset::ShatterOut => {
                "Breaks into pieces that fly apart, from the playhead (drag the diamond: where it breaks)"
            }
            Preset::ShatterIn => "Pieces fly together into the layer, ending at the playhead",
        }
    }

    /// The modifiers this preset adds, sized to `bounds` (layer space);
    /// `target` is the other layer for those that need one; `time` is the
    /// playhead (where timed ones start or end).
    pub fn modifiers(self, bounds: Rect, target: Option<Id>, time: f64) -> Vec<Modifier> {
        let gap = |d: f64| d + (d * 0.1).max(10.0);
        let centre = bounds.center().to_vec2();
        let small = bounds.width().min(bounds.height()).max(10.0);
        let one = |kind| vec![Modifier::new(kind)];
        match self {
            Preset::Row => vec![Modifier::new(ModifierKind::array(5, Vec2::new(gap(bounds.width()), 0.0), centre))],
            Preset::Radial => {
                let radius = bounds.height().max(bounds.width()).max(20.0);
                vec![Modifier::new(ModifierKind::radial_array(8, centre + Vec2::new(0.0, radius)))]
            }
            Preset::Grid => vec![
                Modifier::new(ModifierKind::array(4, Vec2::new(gap(bounds.width()), 0.0), centre)),
                Modifier::new(ModifierKind::array(3, Vec2::new(0.0, gap(bounds.height())), centre)),
            ],
            Preset::Jitter => one(ModifierKind::Jitter {
                amount: (small * 0.04).max(2.0).into(),
                detail: 5.0.into(),
                seed: 0.0.into(),
                smooth: false,
            }),
            Preset::ZigZag | Preset::Wave => one(ModifierKind::ZigZag {
                amount: (small * 0.05).max(3.0).into(),
                ridges: 4.0.into(),
                smooth: self == Preset::Wave,
            }),
            Preset::PuckerBloat => one(ModifierKind::PuckerBloat { strength: (-0.3).into() }),
            Preset::Twist => one(ModifierKind::Twist { rotation: 60.0.into(), origin: centre.into() }),
            Preset::Smooth => one(ModifierKind::Smooth { strength: 1.0.into() }),
            Preset::Subdivide => one(ModifierKind::Subdivide { levels: 1.0.into() }),
            Preset::Simplify => one(ModifierKind::Simplify { tolerance: 1.0.into() }),
            Preset::OffsetPath => one(ModifierKind::OffsetPath {
                amount: (small * 0.1).max(5.0).into(),
                join: xenify_doc::LineJoin::Round,
            }),
            Preset::Mirror => one(ModifierKind::Mirror {
                origin: Vec2::new(bounds.x1, bounds.center().y).into(),
                rotation: 90.0.into(),
                merge: false,
            }),
            Preset::PathArray => match target {
                Some(path) => one(ModifierKind::PathArray {
                    path,
                    count: 8.0.into(),
                    start: 0.0.into(),
                    end: 1.0.into(),
                    align: true,
                }),
                None => Vec::new(),
            },
            Preset::Blend => match target {
                Some(target) => one(ModifierKind::Blend { target, count: 4.0.into() }),
                None => Vec::new(),
            },
            Preset::ShatterOut | Preset::ShatterIn => {
                one(xenify_ops::effects::shatter(bounds, time, self == Preset::ShatterOut))
            }
        }
    }
}

/// The presets as menu entries: arrays, then outline changes. `pick` gets
/// the one clicked.
fn preset_items(ui: &mut egui::Ui, prefix: &str, pick: &mut Option<Preset>) {
    for (i, group) in
        [&Preset::ARRAYS[..], &Preset::SHAPES[..], &Preset::PAIRS[..], &Preset::BREAK[..]].into_iter().enumerate()
    {
        if i > 0 {
            ui.separator();
        }
        for &p in group {
            if ui.button(format!("{prefix}{}", p.label())).on_hover_text(p.tip()).clicked() {
                *pick = Some(p);
                ui.close();
            }
        }
    }
}

/// `id`'s content box in its own layer space (for a group, around its members).
fn layer_box(app: &XenifyApp, id: Id) -> Rect {
    let comp = app.comp();
    let scene = xenify_anim::evaluate_project(app.project(), comp.id, app.time);
    let mut measure = app.viewport.measurer();
    let own = scene.layer(id).filter(|_| !matches!(comp.layer(id).map(|l| &l.kind), Some(LayerKind::Group)));
    let b = match own {
        Some(l) => l.bounds(&mut measure),
        None => xenify_anim::world_box(comp, &scene, id, &mut measure).and_then(|w| {
            let t = xenify_anim::world_transform(comp, id, app.time);
            (t.determinant().abs() > 1e-12).then(|| t.inverse().transform_rect_bbox(w))
        }),
    };
    b.unwrap_or(Rect::new(-50.0, -50.0, 50.0, 50.0))
}

/// Shape layers `layer` could follow or blend into, by name.
fn other_shapes(app: &XenifyApp, layer: Id) -> Vec<(Id, String)> {
    app.comp()
        .layers
        .iter()
        .rev()
        .filter(|l| l.id != layer && matches!(l.kind, LayerKind::Shape { .. }))
        .map(|l| (l.id, l.meta.name.clone()))
        .collect()
}

/// Adds `preset` to each of `layers`. Those needing another layer go on the
/// first one clicked, with the second as the path / shape to blend into.
pub fn add(app: &mut XenifyApp, layers: &[Layer], preset: Preset) {
    let (layers, target): (Vec<Layer>, Option<Id>) = if preset.needs_target() {
        let order = app.selection_in_click_order();
        let first = order.first().and_then(|id| layers.iter().find(|l| l.id == *id)).or(layers.first());
        let target = order
            .iter()
            .find(|id| Some(**id) != first.map(|l| l.id))
            .copied()
            .or_else(|| first.and_then(|l| other_shapes(app, l.id).first().map(|(id, _)| *id)));
        (first.cloned().into_iter().collect(), target)
    } else {
        (layers.to_vec(), None)
    };
    if preset.needs_target() && target.is_none() {
        app.status = format!("{} needs a second shape layer: select both.", preset.label());
        return;
    }
    let cmds = layers
        .iter()
        .map(|l| {
            let mut meta = l.meta.clone();
            meta.modifiers.extend(preset.modifiers(layer_box(app, l.id), target, app.time));
            Command::SetLayerMeta { layer: l.id, meta }
        })
        .collect();
    if app.exec(preset.label(), Command::batch(cmds)) {
        app.status = format!("{}: change it under Modifiers in Properties.", preset.label());
    }
}

/// Makes `layer`'s first modifier permanent: its copies become layers.
pub fn apply_first(app: &mut XenifyApp, layer: Id) {
    let comp = app.comp().id;
    match xenify_ops::modifiers::apply_first(app.project(), comp, layer, app.time) {
        Ok(cmd) => {
            if app.exec("Apply modifier", cmd) {
                app.status = "Applied: it's part of the layers now.".into();
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if !takes_modifiers(layer) {
        return;
    }
    let n = layer.meta.modifiers.len();
    let title = if n == 0 { "Modifiers".to_string() } else { format!("Modifiers ({n})") };
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(("modifiers", layer.id, n > 0)) // opens when the first one is added
        .default_open(n > 0)
        .show(ui, |ui| editor(app, ui, layer));
}

/// `layer`'s modifiers: on/off, settings, order, apply, add and remove.
fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let mut meta = layer.meta.clone();
    let mut remove = None;
    let mut move_up = None;
    let mut apply = false;
    let others = other_shapes(app, layer.id);
    for (i, m) in meta.modifiers.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut m.enabled, RichText::new(m.kind.label()).strong()).on_hover_text("On / off");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                if i > 0 && ui.small_button("⏶").on_hover_text("Move up (modifiers act in order)").clicked() {
                    move_up = Some(i);
                }
                let tip = if m.kind.is_array() {
                    "Make the copies layers of their own (as they are now)"
                } else {
                    "Make the change part of the shapes (as they are now)"
                };
                if i == 0 && ui.small_button("Apply").on_hover_text(tip).clicked() {
                    apply = true;
                }
            });
        });
        match &mut m.kind {
            ModifierKind::Jitter { smooth, .. } => {
                ui.checkbox(smooth, "Keep curves").on_hover_text("Else straight lines join the moved points");
            }
            ModifierKind::ZigZag { smooth, .. } => {
                ui.checkbox(smooth, "Smooth (wave)").on_hover_text("Rounded waves instead of sharp ridges");
            }
            ModifierKind::Mirror { merge, .. } => {
                ui.checkbox(merge, "Merge halves")
                    .on_hover_text("Join the shape and its mirror image into one outline");
            }
            ModifierKind::PathArray { align, .. } => {
                ui.checkbox(align, "Turn with the path").on_hover_text("Each copy turns to follow the line");
            }
            ModifierKind::OffsetPath { join, .. } => {
                ui.horizontal(|ui| {
                    ui.label("Corners");
                    for (j, name) in [
                        (xenify_doc::LineJoin::Miter, "Sharp"),
                        (xenify_doc::LineJoin::Round, "Round"),
                        (xenify_doc::LineJoin::Bevel, "Cut"),
                    ] {
                        ui.selectable_value(join, j, name);
                    }
                });
            }
            _ => {}
        }
        let what = if matches!(m.kind, ModifierKind::Blend { .. }) { "Into" } else { "Path" };
        if let Some(target) = m.kind.target_mut() {
            ui.horizontal(|ui| {
                ui.label(what);
                let name = others.iter().find(|(id, _)| id == target).map_or("(missing)", |(_, n)| n.as_str());
                egui::ComboBox::from_id_salt(("modifier-target", layer.id, i)).selected_text(name).show_ui(ui, |ui| {
                    for (id, n) in &others {
                        ui.selectable_value(target, *id, n);
                    }
                });
            });
        }
        if !m.kind.is_array() && matches!(layer.kind, LayerKind::Text(_) | LayerKind::Image { .. }) {
            ui.weak("Text and pictures keep their outline: this changes shapes only.");
        }
        egui::Grid::new(("modifier-props", layer.id, i)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            for &field in m.kind.fields() {
                let label = m.kind.field_label(field);
                crate::panels::prop_row_named(app, ui, layer, PropPath::Modifier { index: i, field }, label);
            }
        });
    }
    if let Some(i) = remove {
        meta.modifiers.remove(i);
    }
    if let Some(i) = move_up {
        meta.modifiers.swap(i, i - 1);
    }
    ui.separator();
    let mut preset = None;
    ui.menu_button("+ Add modifier", |ui| preset_items(ui, "", &mut preset));
    if apply {
        apply_first(app, layer.id);
        return;
    }
    if let Some(p) = preset {
        let added = p.modifiers(layer_box(app, layer.id), others.first().map(|(id, _)| *id), app.time);
        if added.is_empty() {
            app.status = format!("{} needs another shape layer to work with.", p.label());
        }
        meta.modifiers.extend(added);
    }
    // Property rows edit the project directly; only structural changes
    // (on/off, order, add/remove) go through here.
    let fresh = app.project().layer(layer.id).map(|l| l.meta.clone()).unwrap_or_else(|_| layer.meta.clone());
    if meta != layer.meta && fresh == layer.meta {
        app.exec("Modifiers", Command::SetLayerMeta { layer: layer.id, meta });
    }
}

/// Layer › Modifiers: add arrays to the selected layers, apply or clear them.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let targets: Vec<Layer> = app
        .selected_layers()
        .iter()
        .filter_map(|id| app.project().layer(*id).ok().cloned())
        .filter(takes_modifiers)
        .collect();
    ui.add_enabled_ui(!targets.is_empty(), |ui| {
        ui.menu_button("Modifiers", |ui| {
            let mut pick = None;
            preset_items(ui, "Add ", &mut pick);
            if let Some(p) = pick {
                add(app, &targets, p);
            }
            ui.separator();
            let with: Vec<Id> = targets.iter().filter(|l| !l.meta.modifiers.is_empty()).map(|l| l.id).collect();
            if ui
                .add_enabled(!with.is_empty(), egui::Button::new("Apply First Modifier"))
                .on_hover_text("Its copies become layers of their own, or its change part of the shapes")
                .clicked()
            {
                for id in with.clone() {
                    apply_first(app, id);
                }
                ui.close();
            }
            if ui.add_enabled(!with.is_empty(), egui::Button::new("Clear Modifiers")).clicked() {
                edit_all(app, &targets, "Clear modifiers", |m| m.modifiers.clear());
                ui.close();
            }
        })
        .response
        .on_disabled_hover_text("Select layers or groups first");
    });
}

fn edit_all(app: &mut XenifyApp, layers: &[Layer], label: &str, f: impl Fn(&mut LayerMeta)) {
    let cmds = layers
        .iter()
        .map(|l| {
            let mut meta = l.meta.clone();
            f(&mut meta);
            Command::SetLayerMeta { layer: l.id, meta }
        })
        .collect();
    app.exec(label, Command::batch(cmds));
}
