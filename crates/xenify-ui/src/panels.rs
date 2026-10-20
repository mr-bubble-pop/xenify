//! Menu bar, toolbar, and the inspector (layers + properties).

use crate::app::{XenifyApp, Tool};
use crate::dialogs::Dialog;
use crate::widgets::{ACCENT, Icon, icon_button, prop_label};
use egui::{Color32, RichText};
use xenify_doc::{
    AnyProperty, Color, Id, Layer, LayerKind, LayerMeta, PropPath, Property, ShapeField, ShapeItem, Value, Vec2,
};
use xenify_ops::Command;

pub fn menu_bar(app: &mut XenifyApp, ui: &mut egui::Ui) {
    crate::menus::menu_bar(app, ui);
}

/// Mouse gestures (keys are listed from the keymap).
const MOUSE: &[(&str, &str)] = &[
    ("Scroll / Ctrl+scroll", "Pan / zoom the canvas"),
    ("Space+drag, middle-drag", "Pan the canvas"),
    ("Shift+click", "Add to the selection"),
    ("Ctrl+click", "Select inside a group"),
    ("Ctrl (while dragging)", "Turn snapping off (layers, new shapes, path points)"),
    (
        "Shift (while dragging)",
        "Constrain: one axis when moving, 15° steps when rotating or drawing lines, squares and circles",
    ),
    ("Alt (while drawing)", "Draw from the centre"),
    ("Alt (dragging a handle)", "Break a smooth point's handles apart"),
    ("Perspective tool (W)", "Drag a corner to distort in perspective; Ctrl+corner moves just that corner"),
    ("Double-click a symbol", "Edit it"),
    ("Ctrl+scroll / Shift+scroll (timeline)", "Zoom / pan time"),
    ("Right-click a keyframe", "Easing, curve editor, delete"),
    ("Right-click a layer name (timeline)", "Select its keyframes, key all properties"),
];

/// The shortcut reference window.
pub fn shortcuts_window(app: &mut XenifyApp, ctx: &egui::Context) {
    use crate::keymap::{Action, Category};
    let mut open = app.shortcuts_open;
    egui::Window::new("Keyboard shortcuts").open(&mut open).collapsible(false).default_width(560.0).show(ctx, |ui| {
        ui.weak("Change any of these in Settings › Shortcuts.");
        egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
            let all = Action::all();
            for cat in Category::ALL {
                ui.add_space(4.0);
                ui.label(RichText::new(cat.label()).strong().color(ACCENT));
                egui::Grid::new(cat.label()).num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
                    for a in all.iter().filter(|a| a.category() == cat) {
                        let keys = app.keymap.all_text(*a);
                        if keys.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(keys).monospace());
                        ui.label(a.label());
                        ui.end_row();
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Mouse").strong().color(ACCENT));
            egui::Grid::new("mouse").num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
                for (keys, what) in MOUSE {
                    ui.label(RichText::new(*keys).monospace());
                    ui.label(*what);
                    ui.end_row();
                }
            });
        });
    });
    app.shortcuts_open = open;
}

/// The shape tools, grouped under one toolbar button.
const SHAPES: [Tool; 4] = [Tool::Rect, Tool::Ellipse, Tool::Polygon, Tool::Line];

/// What the toolbar shows, top to bottom (`None` = a divider).
const TOOLBAR: &[Option<Tool>] = &[
    Some(Tool::Select),
    Some(Tool::Node),
    Some(Tool::Perspective),
    None,
    Some(Tool::Rect), // the shapes group
    Some(Tool::Pen),
    Some(Tool::Pencil),
    Some(Tool::Brush),
    Some(Tool::Sculpt),
    Some(Tool::Knife),
    None,
    Some(Tool::Text),
    Some(Tool::Connector),
    Some(Tool::Eyedropper),
    Some(Tool::Gradient),
    Some(Tool::Mask),
    Some(Tool::Slice),
    Some(Tool::Orbit),
    None,
    Some(Tool::Hand),
    Some(Tool::Zoom),
];

/// The toolbar on a diagram canvas: drawing, connecting and labelling.
const DIAGRAM_TOOLBAR: &[Option<Tool>] = &[
    Some(Tool::Select),
    Some(Tool::Node),
    None,
    Some(Tool::Rect), // the shapes group
    Some(Tool::Connector),
    Some(Tool::Pen),
    Some(Tool::Text),
    None,
    Some(Tool::Eyedropper),
    Some(Tool::Gradient),
    None,
    Some(Tool::Hand),
    Some(Tool::Zoom),
];

pub fn toolbar(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let tools = if app.comp().settings.kind.is_animation() { TOOLBAR } else { DIAGRAM_TOOLBAR };
    egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(
        ui,
        |ui| {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for entry in tools {
                    match entry {
                        None => {
                            ui.add_space(3.0);
                            ui.separator();
                            ui.add_space(1.0);
                        }
                        Some(Tool::Rect) => shapes_button(app, ui),
                        Some(tool) => {
                            let Some((_, icon, tip, _)) = Tool::ALL.iter().find(|t| t.0 == *tool) else { continue };
                            let tip = tool_tip(app, *tool, tip);
                            if icon_button(ui, *icon, 30.0, app.tool == *tool, false).on_hover_text(tip).clicked() {
                                app.set_tool(*tool);
                            }
                            // Crop sits with the transform tools (it's a mode, not a tool).
                            if *tool == Tool::Perspective {
                                let selection = !app.selected_layers().is_empty();
                                let tip = if selection {
                                    "Crop the selection: drag the edges in, then Apply"
                                } else {
                                    "Crop the canvas: drag the edges in, then Apply"
                                };
                                if icon_button(ui, crate::widgets::Icon::Crop, 30.0, app.tool == Tool::Crop, false)
                                    .on_hover_text(tip)
                                    .clicked()
                                {
                                    crate::viewport::start_crop(app, selection);
                                }
                            }
                        }
                    }
                }
                ui.add_space(6.0);
                crate::colors::swatch_pair(app, ui, 34.0);
            });
        },
    );
}

/// The shapes group: shows the last shape tool used; clicking picks it, and
/// clicking it again (or right-clicking) opens the list of shapes.
fn shapes_button(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let key = egui::Id::new("last-shape-tool");
    let last = if SHAPES.contains(&app.tool) {
        ui.ctx().data_mut(|d| d.insert_temp(key, app.tool));
        app.tool
    } else {
        ui.ctx().data(|d| d.get_temp::<Tool>(key)).unwrap_or(Tool::Rect)
    };
    let (_, icon, tip, _) = Tool::ALL.iter().find(|t| t.0 == last).copied().unwrap_or(Tool::ALL[0]);
    let active = SHAPES.contains(&app.tool);
    let r = icon_button(ui, icon, 30.0, active, false);
    // A small corner triangle: there's more in this group.
    let corner = r.rect.right_bottom() - egui::vec2(3.0, 3.0);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![corner, corner - egui::vec2(6.0, 0.0), corner - egui::vec2(0.0, 6.0)],
        Color32::from_gray(170),
        egui::Stroke::NONE,
    ));
    let r = r.on_hover_text(format!(
        "{}\n\nShapes: click again (or right-click) for rectangles, ellipses, polygons, stars, gears, hearts, \
         arrows, speech bubbles and lines. \
         Click on the canvas without dragging to type an exact size",
        tool_tip(app, last, tip)
    ));
    let popup_id = egui::Popup::default_response_id(&r);
    if r.clicked() {
        if active {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        } else {
            app.set_tool(last);
        }
    }
    if r.secondary_clicked() {
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
    egui::Popup::menu(&r).open_memory(None).show(|ui| {
        ui.set_min_width(170.0);
        use xenify_ops::draw::Figure;
        let o = app.tool_options.clone();
        let sides = o.polygon_sides;
        let rounded = if o.corner_radius > 0.0 { o.corner_radius } else { 20.0 };
        /// The polygon tool's figure and sides, or the rectangle's corner radius.
        #[derive(Clone, Copy, PartialEq)]
        enum Entry {
            Plain,
            Corners(f64),
            Figure(Figure, usize),
        }
        let entries: [(&str, Tool, Entry); 11] = [
            ("Rectangle", Tool::Rect, Entry::Corners(0.0)),
            ("Rounded Rectangle", Tool::Rect, Entry::Corners(rounded)),
            ("Ellipse", Tool::Ellipse, Entry::Plain),
            ("Triangle", Tool::Polygon, Entry::Figure(Figure::Polygon, 3)),
            ("Polygon", Tool::Polygon, Entry::Figure(Figure::Polygon, sides.clamp(4, 64))),
            ("Star", Tool::Polygon, Entry::Figure(Figure::Star, sides.max(5))),
            ("Gear", Tool::Polygon, Entry::Figure(Figure::Gear, sides)),
            ("Heart", Tool::Polygon, Entry::Figure(Figure::Heart, sides)),
            ("Arrow", Tool::Polygon, Entry::Figure(Figure::Arrow, sides)),
            ("Speech Bubble", Tool::Polygon, Entry::Figure(Figure::Bubble, sides)),
            ("Line", Tool::Line, Entry::Plain),
        ];
        for (label, tool, entry) in entries {
            let on = app.tool == tool
                && match entry {
                    Entry::Plain => true,
                    Entry::Corners(r) => (r > 0.0) == (o.corner_radius > 0.0),
                    Entry::Figure(Figure::Polygon, n) => o.figure == Figure::Polygon && (n == 3) == (sides == 3),
                    Entry::Figure(f, _) => o.figure == f,
                };
            let keys = app.keymap.all_text(crate::keymap::Action::Tool(tool));
            // The shortcut picks the tool with its last settings: shown on the plain entries.
            let plain = matches!(entry, Entry::Plain | Entry::Corners(0.0) | Entry::Figure(Figure::Polygon, 4..));
            let text = if keys.is_empty() || !plain { label.to_string() } else { format!("{label}  ({keys})") };
            let picture = |b: kurbo::Rect| match entry {
                Entry::Corners(r) => kurbo::Shape::to_path(&b.to_rounded_rect(if r > 0.0 { 2.5 } else { 0.0 }), 0.1),
                Entry::Figure(f, n) => xenify_ops::draw::figure_path(
                    f,
                    &xenify_ops::draw::FigureParams { sides: n, ..o.figure_params() },
                    b,
                ),
                Entry::Plain if tool == Tool::Ellipse => kurbo::Shape::to_path(&kurbo::Ellipse::from_rect(b), 0.1),
                Entry::Plain => kurbo::Shape::to_path(&kurbo::Line::new((b.x0, b.y1), (b.x1, b.y0)), 0.1),
            };
            if crate::widgets::shape_row(ui, on, text, picture).clicked() {
                app.set_tool(tool);
                match entry {
                    Entry::Plain => {}
                    Entry::Corners(r) => app.tool_options.corner_radius = r,
                    Entry::Figure(f, n) => {
                        app.tool_options.figure = f;
                        app.tool_options.polygon_sides = n;
                    }
                }
                ui.close();
            }
        }
        ui.separator();
        ui.weak("Sides, points, teeth, corners: in the bar above the canvas");
    });
}

/// A tool's tooltip with its current shortcut: "Pen (P) — …" shows whatever
/// key Pen is bound to now.
fn tool_tip(app: &XenifyApp, tool: Tool, tip: &str) -> String {
    let keys = app.keymap.all_text(crate::keymap::Action::Tool(tool));
    // The descriptions name their default key in the first "(…)"; replace it.
    let (name, rest) = match (tip.find(" ("), tip.find(')')) {
        (Some(open), Some(close)) if close > open && close - open <= 12 => (&tip[..open], &tip[close + 1..]),
        _ => (tip.split(" —").next().unwrap_or(tip), tip.find(" —").map_or("", |i| &tip[i..])),
    };
    if keys.is_empty() { format!("{name}{rest}") } else { format!("{name} ({keys}){rest}") }
}

/// Layer › New.
pub fn new_layer_items(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let s = &app.comp().settings;
    let centre = Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0);
    let side = (s.width.min(s.height) as f64 / 4.0).round();
    let color = Color::from_hex("#118ab2").unwrap();
    if ui.button("Rectangle").clicked() {
        crate::viewport::add_layer(app, "Rectangle", vec![ShapeItem::rect(Vec2::new(side, side), color)], centre);
        ui.close();
    }
    if ui.button("Ellipse").clicked() {
        crate::viewport::add_layer(app, "Ellipse", vec![ShapeItem::ellipse(Vec2::new(side, side), color)], centre);
        ui.close();
    }
    if ui.button("Text").clicked() {
        crate::viewport::add_text_layer(app, centre);
        ui.close();
    }
    if ui.button("Empty Group").on_hover_text("Drag layers onto it in the Layers panel").clicked() {
        let id = Id(app.project().next_id);
        let n = app.comp().layers.iter().filter(|l| matches!(l.kind, LayerKind::Group)).count() + 1;
        let layer = Layer::new(id, format!("Group {n}"), LayerKind::Group).with_position(centre);
        if app.exec("New group", Command::AddLayer { comp: app.comp, index: None, layer }) {
            app.select_only(Some(id));
        }
        ui.close();
    }
    if ui
        .button("Camera")
        .on_hover_text("Frames what the viewer sees: move, turn, zoom, follow a layer, shake")
        .clicked()
    {
        crate::camera_ui::new_camera(app);
        ui.close();
    }
    ui.menu_button("Particles", |ui| crate::particles_ui::menu(app, ui));
    ui.menu_button("Water", |ui| crate::liquid_ui::menu(app, ui));
    ui.menu_button("Sound", |ui| crate::sound_gen_ui::menu(app, ui));
    if ui.button("Emoji…").on_hover_text("Search the emoji and add them as editable shapes").clicked() {
        crate::dock::reveal(app, crate::dock::Panel::Emoji);
        ui.close();
    }
    if ui
        .button("GIF (GIPHY)…")
        .on_hover_text("Search GIPHY with your own key; GIFs come in as animated images")
        .clicked()
    {
        crate::dock::reveal(app, crate::dock::Panel::Gifs);
        ui.close();
    }
    ui.menu_button("3D", |ui| crate::solids::menu(app, ui));
    if ui.button("Illustration (AI)…").on_hover_text("An image model paints it; kept as a picture or traced").clicked()
    {
        app.dialog = Some(Dialog::illustrate());
        ui.close();
    }
}

pub fn add_layer_buttons(app: &mut XenifyApp, ui: &mut egui::Ui) {
    if let Some(id) = app.selected
        && let Some(cmd) = crate::viewport::convert_to_path_command(app, id)
    {
        if ui
            .button("Convert to Path")
            .on_hover_text("Turn the selected rectangle/ellipse into an editable path")
            .clicked()
        {
            app.exec("Convert to path", cmd);
            app.tool = Tool::Node;
            ui.close();
        }
        ui.separator();
    }
    let s = &app.comp().settings;
    let centre = Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0);
    let side = (s.width.min(s.height) as f64 / 4.0).round();
    let color = Color::from_hex("#118ab2").unwrap();
    if ui.button("New Rectangle").clicked() {
        crate::viewport::add_layer(app, "Rectangle", vec![ShapeItem::rect(Vec2::new(side, side), color)], centre);
        ui.close();
    }
    if ui.button("New Ellipse").clicked() {
        crate::viewport::add_layer(app, "Ellipse", vec![ShapeItem::ellipse(Vec2::new(side, side), color)], centre);
        ui.close();
    }
    if ui.button("New Text").clicked() {
        crate::viewport::add_text_layer(app, centre);
        ui.close();
    }
    let selected = app.selected_layers();
    if !selected.is_empty()
        && ui.add(egui::Button::new("Group").shortcut_text(app.keymap.text(crate::keymap::Action::Group))).clicked()
    {
        app.group_selection();
        ui.close();
    }
    if selected.iter().any(|id| app.project().layer(*id).is_ok_and(|l| matches!(l.kind, LayerKind::Group)))
        && ui.add(egui::Button::new("Ungroup").shortcut_text(app.keymap.text(crate::keymap::Action::Ungroup))).clicked()
    {
        app.ungroup_selection();
        ui.close();
    }
    if selected.len() > 1
        && ui
            .add(egui::Button::new("Set Mask").shortcut_text(app.keymap.text(crate::keymap::Action::ClipMask)))
            .on_hover_text("The top-most selected shape becomes the mask: the others show only inside it")
            .clicked()
    {
        app.clip_selection(true);
        ui.close();
    }
    let clipped = app
        .comp()
        .layers
        .iter()
        .any(|l| l.meta.matte.is_some_and(|m| selected.contains(&l.id) || selected.contains(&m.layer)));
    if clipped
        && ui
            .add(
                egui::Button::new("Release Mask")
                    .shortcut_text(app.keymap.text(crate::keymap::Action::ReleaseClipMask)),
            )
            .clicked()
    {
        app.clip_selection(false);
        ui.close();
    }
    if !selected.is_empty() {
        ui.separator();
    }
    if ui
        .button("Generate Illustration…")
        .on_hover_text("An image model paints it; Xenify Vector traces it into vectors")
        .clicked()
    {
        app.dialog = Some(Dialog::illustrate());
        ui.close();
    }
    ui.menu_button("3D", |ui| crate::solids::menu(app, ui));
    if ui
        .button("New Empty Group")
        .on_hover_text(
            "A group to fill by dragging layers onto it in the Layers panel (or select layers and press Ctrl+G)",
        )
        .clicked()
    {
        let id = Id(app.project().next_id);
        let n = app.comp().layers.iter().filter(|l| matches!(l.kind, LayerKind::Group)).count() + 1;
        let layer = Layer::new(id, format!("Group {n}"), LayerKind::Group).with_position(centre);
        if app.exec("New group", Command::AddLayer { comp: app.comp, index: None, layer }) {
            app.select_only(Some(id));
        }
        ui.close();
    }
}

pub fn layers(app: &mut XenifyApp, ui: &mut egui::Ui) {
    crate::layers_panel::show(app, ui);
}

/// Align & distribute buttons for the selected layers.
fn arrange(app: &mut XenifyApp, ui: &mut egui::Ui) {
    use xenify_ops::arrange::{self, AlignEdge, AlignTo, Axis};
    let layers = app.selected_layers();
    let many = layers.len() > 1;
    egui::CollapsingHeader::new(
        RichText::new(if many { format!("Arrange {} layers", layers.len()) } else { "Arrange".into() }).strong(),
    )
    .default_open(many)
    .show(ui, |ui| {
        let mut to_comp =
            ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new("align-to-comp"))).unwrap_or(false) || !many;
        let mut action: Option<Result<Option<Command>, xenify_ops::OpError>> = None;
        let (project, comp, time, auto_key) = (app.project().clone(), app.comp, app.time, app.auto_key);
        let mut measure = app.viewport.measurer();
        let to = if to_comp { AlignTo::Composition } else { AlignTo::Selection };
        ui.horizontal_wrapped(|ui| {
            for (edge, label, tip) in [
                (AlignEdge::Left, "⇤ Left", "Align left edges"),
                (AlignEdge::HCenter, "↔ Centre", "Align horizontal centres"),
                (AlignEdge::Right, "Right ⇥", "Align right edges"),
                (AlignEdge::Top, "↑ Top", "Align top edges"),
                (AlignEdge::VCenter, "↕ Middle", "Align vertical centres"),
                (AlignEdge::Bottom, "↓ Bottom", "Align bottom edges"),
            ] {
                if ui.small_button(label).on_hover_text(tip).clicked() {
                    action = Some(arrange::align(&project, comp, &layers, edge, to, time, auto_key, &mut measure));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(many, |ui| {
                ui.radio_value(&mut to_comp, false, "to selection");
            });
            ui.radio_value(&mut to_comp, true, "to canvas");
        });
        if layers.len() >= 3 {
            ui.horizontal(|ui| {
                ui.label("Distribute");
                if ui.small_button("Horizontally").on_hover_text("Equal gaps left to right").clicked() {
                    action = Some(arrange::distribute(
                        &project,
                        comp,
                        &layers,
                        Axis::Horizontal,
                        time,
                        auto_key,
                        &mut measure,
                    ));
                }
                if ui.small_button("Vertically").on_hover_text("Equal gaps top to bottom").clicked() {
                    action = Some(arrange::distribute(
                        &project,
                        comp,
                        &layers,
                        Axis::Vertical,
                        time,
                        auto_key,
                        &mut measure,
                    ));
                }
            });
        } else if many {
            ui.weak("Select 3+ layers (Shift+click) to distribute them.");
        } else {
            ui.weak("Shift+click more layers to align them to each other.");
        }
        let mut combine = None;
        if many {
            use xenify_ops::pathfinder::PathOp;
            ui.horizontal_wrapped(|ui| {
                ui.label("Boolean");
                for (op, name) in PathOp::ALL {
                    let tip = match op {
                        PathOp::Unite => "Merge into one shape",
                        PathOp::MinusFront => "Cut the shapes above out of the bottom one",
                        PathOp::Intersect => "Keep only where they all overlap",
                        PathOp::Exclude => "Remove where they overlap",
                    };
                    if ui.small_button(name).on_hover_text(tip).clicked() {
                        combine = Some(op);
                    }
                }
            });
        }
        drop(measure);
        if let Some(op) = combine {
            match xenify_ops::pathfinder::combine(&project, comp, &layers, op, time) {
                Ok((cmd, id)) => {
                    if app.exec("Boolean", cmd) {
                        app.select_only(Some(id));
                        app.status = "Combined into one path — press A (Edit Points) to edit its points.".into();
                    }
                }
                Err(e) => app.fail(e.to_string()),
            }
            return;
        }
        if many {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("align-to-comp"), to_comp));
        }
        match action {
            Some(Ok(Some(cmd))) => {
                app.exec("Arrange", cmd);
            }
            Some(Ok(None)) => app.status = "Already aligned.".into(),
            Some(Err(e)) => app.fail(e.to_string()),
            None => {}
        }
    });
}

/// Motion presets: pick one, tweak it, apply at the playhead.
fn presets(app: &mut XenifyApp, ui: &mut egui::Ui) {
    use xenify_ops::presets::{self, Direction, Preset};
    let layers = app.selected_layers();
    egui::CollapsingHeader::new(RichText::new("Motion presets").strong()).default_open(false).show(ui, |ui| {
        let (mut index, mut preset, mut stagger) = app.preset_ui;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("preset").selected_text(Preset::DEFAULTS[index].0).show_ui(ui, |ui| {
                for (i, (name, default)) in Preset::DEFAULTS.iter().enumerate() {
                    if ui.selectable_label(i == index, *name).clicked() {
                        index = i;
                        preset = *default;
                    }
                }
            });
        });
        let slider =
            |ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str| {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let speed = (range.end() - range.start()) / 300.0;
                    ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix).max_decimals(2));
                });
            };
        match &mut preset {
            Preset::FadeIn { duration } | Preset::FadeOut { duration } => {
                slider(ui, "Duration", duration, 0.05..=5.0, " s")
            }
            Preset::PopIn { duration, overshoot } => {
                slider(ui, "Duration", duration, 0.1..=3.0, " s");
                slider(ui, "Overshoot", overshoot, 0.0..=0.5, "");
            }
            Preset::SlideIn { duration, from, distance } => {
                slider(ui, "Duration", duration, 0.1..=3.0, " s");
                slider(ui, "Distance", distance, 10.0..=2000.0, " px");
                ui.horizontal(|ui| {
                    ui.label("From");
                    for (d, n) in [
                        (Direction::Left, "left"),
                        (Direction::Right, "right"),
                        (Direction::Up, "above"),
                        (Direction::Down, "below"),
                    ] {
                        ui.selectable_value(from, d, n);
                    }
                });
            }
            Preset::Bounce { duration, height, bounces } => {
                slider(ui, "Duration", duration, 0.3..=5.0, " s");
                slider(ui, "Drop height", height, 20.0..=1500.0, " px");
                ui.horizontal(|ui| {
                    ui.label("Bounces");
                    ui.add(egui::DragValue::new(bounces).range(0..=6).speed(0.05));
                });
            }
            Preset::SquashStretch { strength } => {
                slider(ui, "Strength", strength, 0.05..=0.6, "");
                ui.weak("Works on existing motion with impacts — e.g. apply Bounce first.");
            }
            Preset::DrawOn { duration } => {
                slider(ui, "Duration", duration, 0.1..=10.0, " s");
                ui.weak("Animates Trim End 0 → 100%. Looks best with a stroke.");
            }
            Preset::Wiggle { duration, amount, frequency } => {
                slider(ui, "Duration", duration, 0.2..=10.0, " s");
                slider(ui, "Amount", amount, 1.0..=200.0, " px");
                slider(ui, "Frequency", frequency, 0.5..=15.0, " /s");
            }
        }
        if layers.len() > 1 {
            slider(ui, "Stagger", &mut stagger, 0.0..=1.0, " s");
        }
        let label = if layers.len() > 1 {
            format!("Apply to {} layers at playhead", layers.len())
        } else {
            "Apply at playhead".into()
        };
        if ui.button(label).on_hover_text("Replaces the animation of the properties this preset uses").clicked() {
            match presets::apply_many(app.project(), &layers, preset, app.time, stagger) {
                Ok(cmd) => {
                    app.exec(Preset::DEFAULTS[index].0, cmd);
                }
                Err(e) => app.fail(e.to_string()),
            }
        }
        app.preset_ui = (index, preset, stagger);
    });
}

pub fn properties(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let Some(layer) = app.selected.and_then(|id| app.project().layer(id).ok()).cloned() else {
        canvas_summary(app, ui);
        return;
    };
    let n = app.selected_layers().len();
    if n > 1 {
        egui::Frame::new().fill(ACCENT.gamma_multiply(0.18)).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("{n} layers selected")).strong()).on_hover_text(
                "Values with a ⏱ (position, size, colours…) change on all of them; the rest is for the \
                 first one clicked",
            );
            ui.weak(format!("Values with a ⏱ change on all {n}. Other settings: “{}” only.", layer.meta.name));
        });
    }

    // Name + meta.
    let mut name = layer.meta.name.clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        let r = ui.text_edit_singleline(&mut name);
        if r.lost_focus() && name != layer.meta.name && !name.trim().is_empty() {
            let meta = LayerMeta { name: name.trim().to_string(), ..layer.meta.clone() };
            app.exec("Rename layer", Command::SetLayerMeta { layer: layer.id, meta });
        }
    });
    parent_picker(app, ui, &layer);
    matte_picker(app, ui, &layer);
    ui.separator();

    egui::CollapsingHeader::new(RichText::new("Transform").strong()).default_open(true).show(ui, |ui| {
        egui::Grid::new("transform").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            for path in PropPath::LAYER_PROPS {
                prop_row(app, ui, &layer, path);
            }
            blend_row(app, ui, &layer);
            if layer.transform.distort.is_some() {
                prop_row(app, ui, &layer, PropPath::Distort);
            }
        });
        // The rarely used ones, folded away.
        let used = layer.transform.skew.is_animated()
            || layer.transform.skew.value_at(0.0) != 0.0
            || layer.transform.wiggle.is_some();
        egui::CollapsingHeader::new("More").id_salt(("transform-more", layer.id)).default_open(used).show(ui, |ui| {
            egui::Grid::new("transform-more").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                prop_row(app, ui, &layer, PropPath::Skew);
                prop_row(app, ui, &layer, PropPath::SkewAxis);
            });
            wiggle_editor(app, ui, &layer);
        });
    });

    if let LayerKind::Text(text) = &layer.kind {
        egui::CollapsingHeader::new(RichText::new("Text").strong()).default_open(true).show(ui, |ui| {
            text_editor(app, ui, &layer, text);
            egui::Grid::new("text-props").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                prop_row(app, ui, &layer, PropPath::FontSize);
                prop_row(app, ui, &layer, PropPath::TextColor);
            });
            crate::text_anim_ui::section(app, ui, &layer, text);
        });
    }

    if let LayerKind::Solid(solid) = &layer.kind {
        egui::CollapsingHeader::new(RichText::new("3D").strong()).default_open(true).show(ui, |ui| {
            crate::solids::editor(app, ui, &layer, solid);
            egui::Grid::new("solid-props").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                for path in layer.property_paths() {
                    if matches!(path, PropPath::Solid(_)) {
                        prop_row(app, ui, &layer, path);
                    }
                }
            });
        });
    }

    if let LayerKind::Shape { shapes } = &layer.kind {
        for (index, shape) in shapes.iter().enumerate() {
            egui::CollapsingHeader::new(RichText::new(&shape.name).strong())
                .id_salt(("shape", layer.id, index))
                .default_open(true)
                .show(ui, |ui| {
                    egui::Grid::new(("shape-grid", index)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                        for path in layer.property_paths() {
                            if matches!(path, PropPath::Shape { index: i, .. } if i == index) {
                                match (path, &shape.geometry) {
                                    // A live figure's own names for its numbers.
                                    (PropPath::Shape { field, .. }, xenify_doc::Geometry::Figure { kind, .. })
                                        if matches!(field, ShapeField::Count | ShapeField::Inner) =>
                                    {
                                        use xenify_doc::primitives::FigureKind;
                                        let name = match (field, kind) {
                                            (ShapeField::Count, FigureKind::Polygon) => "Sides",
                                            (ShapeField::Count, FigureKind::Star) => "Points",
                                            (ShapeField::Count, FigureKind::Gear) => "Teeth",
                                            (_, FigureKind::Gear) => "Tooth Depth",
                                            _ => "Inner Radius",
                                        };
                                        prop_row_named(app, ui, &layer, path, name);
                                    }
                                    _ => prop_row(app, ui, &layer, path),
                                }
                            }
                        }
                    });
                    crate::shape_smart::editor(app, ui, &layer, index, shape);
                    fill_stroke_toggles(app, ui, &layer, index, shape);
                });
        }
    }

    crate::templates_ui::role_row(app, ui, &layer);
    crate::symbols::section(app, ui, &layer);
    crate::animated_image_ui::section(app, ui, &layer);
    crate::camera_ui::section(app, ui, &layer);
    crate::particles_ui::section(app, ui, &layer);
    crate::liquid_ui::section(app, ui, &layer);
    crate::media_ui::section(app, ui, &layer);
    // What it looks like first (above), then what changes it, then tools.
    crate::diagram_ui::section(app, ui, &layer);
    crate::masks_ui::section(app, ui, &layer);
    crate::effects_ui::section(app, ui, &layer);
    crate::modifiers_ui::section(app, ui, &layer);
    arrange(app, ui);
    presets(app, ui);
}

/// Blend mode, beside Opacity.
fn blend_row(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    ui.label("Blend").on_hover_text("How this layer mixes with what's under it");
    let mut pick = None;
    ui.menu_button(format!("{} ⏷", layer.meta.blend.label()), |ui| {
        for b in xenify_doc::BlendMode::ALL {
            if ui.selectable_label(layer.meta.blend == b, b.label()).clicked() {
                pick = Some(b);
                ui.close();
            }
        }
    });
    ui.label("");
    ui.end_row();
    if let Some(blend) = pick.filter(|b| *b != layer.meta.blend) {
        let meta = LayerMeta { blend, ..layer.meta.clone() };
        app.exec("Blend mode", Command::SetLayerMeta { layer: layer.id, meta });
    }
}

/// Properties with nothing selected: the canvas itself.
fn canvas_summary(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let s = app.comp().settings.clone();
    ui.label(RichText::new(format!("Canvas: {}", s.name)).strong());
    let all = app.keymap.text(crate::keymap::Action::SelectAll);
    ui.weak(format!("Nothing is selected. Click a layer (or press {all}) to edit it."));
    ui.add_space(4.0);
    egui::Grid::new("canvas-summary").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        ui.label("Size");
        ui.label(format!("{} × {} px", s.width, s.height));
        ui.end_row();
        ui.label("Frame rate");
        ui.label(format!("{} fps", s.fps));
        ui.end_row();
        ui.label("Length");
        ui.label(format!("{} ({} frames)", crate::widgets::timecode(s.duration, s.fps), s.frame_count()));
        ui.end_row();
        ui.label("Layers");
        ui.label(app.comp().layers.len().to_string());
        ui.end_row();
    });
    ui.add_space(4.0);
    if ui.button("Canvas Settings…").on_hover_text("Size, length, frame rate, background, units").clicked() {
        app.dialog = Some(crate::dialogs::Dialog::comp_settings(app));
    }
}

fn parent_picker(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let comp = app.comp().clone();
    let current = layer.meta.parent.and_then(|p| comp.layer(p)).map_or("None".to_string(), |p| p.meta.name.clone());
    ui.horizontal(|ui| {
        ui.label("Parent");
        egui::ComboBox::from_id_salt(("parent", layer.id)).selected_text(current).show_ui(ui, |ui| {
            let mut choice = None;
            if ui.selectable_label(layer.meta.parent.is_none(), "None").clicked() {
                choice = Some(None);
            }
            for other in comp.layers.iter().filter(|l| l.id != layer.id) {
                if ui.selectable_label(layer.meta.parent == Some(other.id), &other.meta.name).clicked() {
                    choice = Some(Some(other.id));
                }
            }
            if let Some(parent) = choice {
                let meta = LayerMeta { parent, ..layer.meta.clone() };
                app.exec("Set parent", Command::SetLayerMeta { layer: layer.id, meta });
            }
        });
    });
}

/// Canvas menu: the current document's (or slide's, or symbol's) canvas.
pub fn composition_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    if ui.button("Canvas Settings…").on_hover_text("Size, units, length, frame rate and background").clicked() {
        app.dialog = Some(Dialog::comp_settings(app));
        ui.close();
    }
    if ui
        .button("Resize Canvas…")
        .on_hover_text("Drag the frame's edges to cut the canvas down (or out, to enlarge it), then Apply")
        .clicked()
    {
        crate::viewport::start_crop(app, false);
        ui.close();
    }
    ui.separator();
    if ui
        .add_enabled(
            !app.selected_layers().is_empty(),
            egui::Button::new("Convert to Symbol").shortcut_text(app.keymap.text(crate::keymap::Action::Precompose)),
        )
        .on_hover_text("Turn the selected layers into a reusable symbol, leaving one copy in their place")
        .clicked()
    {
        app.precompose_selection();
        ui.close();
    }
    if ui.button("Symbols…").on_hover_text("Place, edit, rename and delete symbols").clicked() {
        crate::dock::reveal(app, crate::dock::Panel::Symbols);
        ui.close();
    }
    if ui
        .button("New Diagram Symbol")
        .on_hover_text(
            "A diagram (flowchart, network, workflow) you can place on this canvas: it opens with the diagram \
             tools. Double-click it to edit",
        )
        .clicked()
    {
        crate::symbols::new_diagram(app);
        ui.close();
    }
}

/// Noise: random drift on top of the layer's animation.
fn wiggle_editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let current = layer.transform.wiggle;
    let mut on = current.is_some();
    let mut w = current.unwrap_or_default();
    let mut begin = false;
    let toggled = ui
        .checkbox(&mut on, "Noise")
        .on_hover_text("Smooth random drift on top of the animation (like Blender's Noise modifier)")
        .changed();
    if let (true, Some(_)) = (on, current) {
        egui::Grid::new(("wiggle", layer.id)).num_columns(2).show(ui, |ui| {
            let mut row = |ui: &mut egui::Ui, label: &str, v: &mut f64, range, suffix: &str, mul: f64| {
                ui.label(label);
                let mut shown = *v * mul;
                let range: std::ops::RangeInclusive<f64> = range;
                let speed = (range.end() - range.start()) / 300.0;
                let r =
                    ui.add(egui::DragValue::new(&mut shown).range(range).speed(speed).suffix(suffix).max_decimals(2));
                begin |= r.drag_started();
                if r.changed() {
                    *v = shown / mul;
                }
                ui.end_row();
            };
            row(ui, "Frequency", &mut w.frequency, 0.1..=20.0, " /s", 1.0);
            row(ui, "Position", &mut w.position, 0.0..=300.0, " px", 1.0);
            row(ui, "Rotation", &mut w.rotation, 0.0..=90.0, "°", 1.0);
            row(ui, "Scale", &mut w.scale, 0.0..=50.0, "%", 100.0);
            ui.label("Seed");
            let r = ui.add(egui::DragValue::new(&mut w.seed));
            begin |= r.drag_started() || r.gained_focus();
            ui.end_row();
        });
    }
    let new = on.then_some(w);
    if toggled {
        // A new wiggle gets a seed from the layer id so layers differ.
        let new = new.map(|w| xenify_doc::Wiggle { seed: layer.id.0, ..w });
        app.exec("Noise", Command::SetWiggle { layer: layer.id, wiggle: new });
    } else if new != current {
        if begin {
            app.new_gesture();
        }
        app.exec_merged("Noise", Command::SetWiggle { layer: layer.id, wiggle: new });
    }
}

/// Mask layer: which layer masks this one, and how (like CSS `mask-mode`).
fn matte_picker(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let comp = app.comp().clone();
    let used_by: Vec<&str> = comp
        .layers
        .iter()
        .filter(|l| l.meta.matte.is_some_and(|m| m.layer == layer.id))
        .map(|l| l.meta.name.as_str())
        .collect();
    if !used_by.is_empty() {
        ui.weak(format!("Mask for {} (not drawn itself)", used_by.join(", ")));
    }
    let matte = layer.meta.matte;
    let current = matte.and_then(|m| comp.layer(m.layer)).map_or("None".to_string(), |l| l.meta.name.clone());
    let mut choice = None;
    ui.horizontal(|ui| {
        ui.label("Mask layer").on_hover_text(
            "Show this layer only inside (or outside) another layer. Alpha uses its shape, luminance its brightness",
        );
        egui::ComboBox::from_id_salt(("matte", layer.id)).selected_text(current).show_ui(ui, |ui| {
            if ui.selectable_label(matte.is_none(), "None").clicked() {
                choice = Some(None);
            }
            let candidates = comp
                .layers
                .iter()
                .filter(|l| l.id != layer.id && matches!(l.kind, LayerKind::Shape { .. } | LayerKind::Text(_)));
            for other in candidates {
                if ui.selectable_label(matte.is_some_and(|m| m.layer == other.id), &other.meta.name).clicked() {
                    let mode = matte.map_or(xenify_doc::MatteMode::Alpha, |m| m.mode);
                    choice = Some(Some(xenify_doc::Matte { layer: other.id, mode }));
                }
            }
        });
        if let Some(m) = matte {
            egui::ComboBox::from_id_salt(("matte-mode", layer.id)).selected_text(m.mode.label()).show_ui(ui, |ui| {
                for mode in xenify_doc::MatteMode::ALL {
                    if ui.selectable_label(m.mode == mode, mode.label()).clicked() {
                        choice = Some(Some(xenify_doc::Matte { mode, ..m }));
                    }
                }
            });
        }
    });
    if let Some(matte) = choice {
        let meta = LayerMeta { matte, ..layer.meta.clone() };
        app.exec("Set mask layer", Command::SetLayerMeta { layer: layer.id, meta });
    }
}

/// The stopwatch: lit when the property is animated.
pub(crate) fn stopwatch(animated: bool) -> egui::Button<'static> {
    egui::Button::new(RichText::new("⏱").color(if animated { ACCENT } else { Color32::from_gray(110) })).frame(false)
}

pub(crate) fn stopwatch_tip(animated: bool) -> &'static str {
    if animated { "Stop animating (removes all keyframes)" } else { "Animate: add a keyframe here" }
}

/// What clicking the stopwatch does: starts animating with a keyframe at the
/// playhead, or stops animating (keeping the value at the playhead).
pub(crate) fn toggle_animation(app: &mut XenifyApp, layer: &Layer, path: PropPath) {
    let (Ok(prop), Ok(value)) = (layer.property(path), layer.value_at(path, app.time)) else { return };
    if prop.is_animated() {
        let property = static_property(value);
        app.exec("Remove animation", Command::SetProperty { layer: layer.id, path, property });
    } else {
        let time = app.time;
        app.exec("Add keyframe", Command::SetKeyframe { layer: layer.id, path, time, value, easing: None });
    }
}

/// One property row: stopwatch, label, value editor, keyframe navigator.
pub(crate) fn prop_row(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, path: PropPath) {
    prop_row_named(app, ui, layer, path, &prop_label(path));
}

/// [`prop_row`] with a shorter label, for rows inside a titled section.
pub(crate) fn prop_row_named(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, path: PropPath, label: &str) {
    let Ok(prop) = layer.property(path) else { return };
    let Ok(value) = layer.value_at(path, app.time) else { return };
    let animated = prop.is_animated();
    let t = app.time;

    if ui.add(stopwatch(animated)).on_hover_text(stopwatch_tip(animated)).clicked() {
        toggle_animation(app, layer, path);
    }

    let mixed = app.shared_value_mixed(layer.id, path);
    let shown = if mixed { RichText::new(format!("{label} (mixed)")).italics() } else { RichText::new(label) };
    let tip = if mixed {
        format!("{}: the selected layers differ. Numbers change each of them by the same amount", prop_label(path))
    } else {
        prop_label(path)
    };
    ui.label(shown).on_hover_text(tip);

    ui.horizontal(|ui| {
        if let Some(new) = value_editor(app, ui, path, &value) {
            app.edit_value_all(layer.id, path, new);
        }
        if animated {
            let times = prop.keyframe_times();
            let eps = 0.5 / app.fps();
            let on_key = times.iter().any(|k| (k - t).abs() < eps);
            if icon_button(ui, Icon::PrevKey, 18.0, false, false).on_hover_text("Previous keyframe").clicked()
                && let Some(p) = times.iter().rev().find(|k| **k < t - eps)
            {
                app.set_time(*p);
            }
            let tip = if on_key { "Remove keyframe" } else { "Add keyframe" };
            if icon_button(ui, Icon::Diamond(on_key), 18.0, false, false).on_hover_text(tip).clicked() {
                if on_key {
                    let at = *times.iter().find(|k| (**k - t).abs() < eps).unwrap();
                    app.exec("Remove keyframe", Command::RemoveKeyframe { layer: layer.id, path, time: at });
                } else {
                    app.exec(
                        "Add keyframe",
                        Command::SetKeyframe { layer: layer.id, path, time: t, value, easing: None },
                    );
                }
            }
            if icon_button(ui, Icon::NextKey, 18.0, false, false).on_hover_text("Next keyframe").clicked()
                && let Some(n) = times.iter().find(|k| **k > t + eps)
            {
                app.set_time(*n);
            }
            easing_menu(app, ui, layer.id, path, &prop, &times, t + eps);
        }
        // Expression toggle.
        let has_expr = layer.meta.expressions.iter().any(|e| e.path == path);
        if !has_expr && xenify_anim::expr::supports(prop.kind()) {
            let b = egui::Button::new(RichText::new("=").monospace().color(Color32::from_gray(120))).frame(false);
            if ui
                .add(b)
                .on_hover_text("Add an expression: a formula for this value, e.g. value + wiggle(2, 20) or time * 90")
                .clicked()
            {
                let mut meta = layer.meta.clone();
                meta.expressions.push(xenify_doc::Expression { path, source: "value".into() });
                app.exec("Add expression", Command::SetLayerMeta { layer: layer.id, meta });
            }
        }
    });
    ui.end_row();
    if let Some(i) = layer.meta.expressions.iter().position(|e| e.path == path) {
        expression_row(app, ui, layer, i);
    }
}

/// Easing of the keyframe the playhead is on or just after (the motion
/// towards the next keyframe), as a dropdown of the presets.
fn easing_menu(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    layer: Id,
    path: PropPath,
    prop: &xenify_doc::AnyProperty,
    times: &[f64],
    before: f64,
) {
    // The last keyframe has nothing to ease towards.
    let Some(&key) = times[..times.len().saturating_sub(1)].iter().rev().find(|k| **k <= before) else { return };
    let Some(easing) = prop.easing_at(key) else { return };
    let name = xenify_doc::Easing::PRESETS.iter().find(|(_, e)| *e == easing).map_or_else(
        || if matches!(easing, xenify_doc::Easing::Steps { .. }) { "steps".into() } else { "custom".into() },
        |(n, _)| crate::curve::preset_label(n),
    );
    let tip = format!(
        "Easing from the keyframe at {} to the next one (ease in, ease out, …)",
        crate::widgets::timecode(key, app.fps())
    );
    ui.menu_button(RichText::new(format!("〰 {name}")).small(), |ui| {
        if let Some(e) = crate::curve::preset_picker(ui, Some(easing)) {
            app.exec("Set easing", Command::SetEasing { layer, path, time: key, easing: e });
            ui.close();
        }
        if ui.button("Edit curve…").clicked() {
            app.selected_keys = vec![crate::app::KeySel { layer, path, time: key }];
            app.timeline.curve_open = true;
            ui.close();
        }
    })
    .response
    .on_hover_text(tip);
}

/// The expression editor under a property row, with the live result or error.
fn expression_row(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, i: usize) {
    let expr = &layer.meta.expressions[i];
    let path = expr.path;
    ui.label("");
    ui.label(RichText::new("  = expr").color(Color32::from_rgb(235, 110, 100))).on_hover_text(
        "Names: time, value, frame, fps, duration, index, pi. Functions: wiggle(freq, amount), \
                        random(), noise(x), value_at(t), loop_out(), loop_in(), ping_pong(), sin, cos, abs, \
                        min, max, clamp, lerp, smoothstep, round, floor, … Vectors: [x, y], value.x.",
    );
    ui.vertical(|ui| {
        let mut source = expr.source.clone();
        let r = ui.add(
            egui::TextEdit::singleline(&mut source)
                .code_editor()
                .desired_width(220.0)
                .hint_text("value + wiggle(2, 20)"),
        );
        if r.gained_focus() {
            app.new_gesture();
        }
        let comp = app.comp().clone();
        let result = xenify_anim::expr::preview(&comp, layer.id, &source, path, app.time);
        ui.horizontal(|ui| {
            match &result {
                Ok(v) => ui.label(RichText::new(format!("→ {}", short_value(v))).small().weak()),
                Err(e) => ui.label(RichText::new(e).small().color(crate::widgets::RECORD)),
            };
            if ui.small_button("Remove").on_hover_text("Remove the expression").clicked() {
                let mut meta = layer.meta.clone();
                meta.expressions.remove(i);
                app.exec("Remove expression", Command::SetLayerMeta { layer: layer.id, meta });
            }
        });
        if r.changed() {
            let mut meta = layer.meta.clone();
            meta.expressions[i].source = source;
            app.exec_merged("Edit expression", Command::SetLayerMeta { layer: layer.id, meta });
        }
    });
    ui.end_row();
}

fn short_value(v: &Value) -> String {
    match v {
        Value::Scalar(x) => format!("{x:.2}"),
        Value::Vec2(p) => format!("[{:.1}, {:.1}]", p.x, p.y),
        Value::Color(c) => c.to_hex(),
        _ => "…".into(),
    }
}

pub(crate) fn static_property(value: Value) -> AnyProperty {
    match value {
        Value::Scalar(v) => AnyProperty::Scalar(Property::Static(v)),
        Value::Vec2(v) => AnyProperty::Vec2(Property::Static(v)),
        Value::Color(v) => AnyProperty::Color(Property::Static(v)),
        Value::Path(v) => AnyProperty::Path(Property::Static(v)),
        Value::Stops(v) => AnyProperty::Stops(Property::Static(v)),
        Value::Quad(v) => AnyProperty::Quad(Property::Static(v)),
    }
}

/// Shows an editor for `value`; returns the new value when the user changed it.
fn value_editor(app: &mut XenifyApp, ui: &mut egui::Ui, path: PropPath, value: &Value) -> Option<Value> {
    let (mul, suffix, speed) = crate::widgets::prop_display_in(path, app.comp().settings.units);
    let mut begin = false;
    let result = match value {
        Value::Scalar(v) => {
            let mut shown = v * mul;
            let mut dv = egui::DragValue::new(&mut shown).speed(speed).suffix(suffix).max_decimals(3);
            if let Some(range) = crate::widgets::prop_range(path) {
                dv = dv.range(range);
            }
            let r = ui.add(dv);
            begin |= r.drag_started() || r.gained_focus();
            r.changed().then(|| Value::Scalar(shown / mul))
        }
        Value::Vec2(v) => {
            let (mut x, mut y) = (v.x * mul, v.y * mul);
            let rx = ui.add(egui::DragValue::new(&mut x).speed(speed).suffix(suffix).max_decimals(3));
            let ry = ui.add(egui::DragValue::new(&mut y).speed(speed).suffix(suffix).max_decimals(3));
            begin |= rx.drag_started() || rx.gained_focus() || ry.drag_started() || ry.gained_focus();
            (rx.changed() || ry.changed()).then(|| Value::Vec2(Vec2::new(x / mul, y / mul)))
        }
        Value::Color(c) => {
            // The app's own picker, as everywhere else.
            let mut color = *c;
            let salt = egui::Id::new(format!("prop-colour-{path:?}"));
            let picked = crate::color_picker::button(ui, salt, &mut color);
            if picked.released {
                app.new_gesture();
            }
            picked.changed.then_some(Value::Color(color))
        }
        Value::Path(_) => {
            ui.weak("Bézier path");
            None
        }
        Value::Stops(stops) => {
            let g = xenify_doc::Gradient {
                kind: xenify_doc::GradientKind::Linear,
                start: Default::default(),
                end: Default::default(),
                stops: stops.clone(),
            };
            gradient_swatch(ui, &g);
            None
        }
        Value::Quad(_) => {
            ui.weak("4 corners (drag them on the canvas)");
            None
        }
    };
    if begin {
        app.new_gesture();
    }
    result
}

fn fill_stroke_toggles(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, index: usize, shape: &ShapeItem) {
    ui.horizontal(|ui| {
        let mut has_fill = shape.fill.is_some();
        let mut has_stroke = shape.stroke.is_some();
        let fill_changed = ui.checkbox(&mut has_fill, "Fill").changed();
        let stroke_changed = ui.checkbox(&mut has_stroke, "Stroke").changed();
        let mut has_trim = shape.trim.is_some();
        if ui
            .checkbox(&mut has_trim, "Trim")
            .on_hover_text("Trim paths: draw only part of the outline — animate Trim End for a draw-on effect")
            .changed()
        {
            let new = ShapeItem { trim: has_trim.then(xenify_doc::Trim::default), ..shape.clone() };
            app.exec("Trim paths", Command::SetShape { layer: layer.id, index, shape: new });
        }
        if fill_changed || stroke_changed {
            let mut new = shape.clone();
            new.fill = has_fill.then(|| shape.fill.clone().unwrap_or(xenify_doc::Fill::solid(Color::WHITE)));
            new.stroke = has_stroke.then(|| {
                shape.stroke.clone().unwrap_or(xenify_doc::Stroke {
                    cap_roundness: 0.0,
                    style: Default::default(),
                    color: Color::WHITE.into(),
                    width: 4.0.into(),
                    join: Default::default(),
                    cap: Default::default(),
                    ..Default::default()
                })
            });
            app.exec("Change fill/stroke", Command::SetShape { layer: layer.id, index, shape: new });
        }
    });
    if shape.fill.is_some() {
        crate::gradient_ui::editor(app, ui, layer, index, shape, false, true);
        crate::pattern_ui::editor(app, ui, layer, index, shape);
    }
    if let Some(stroke) = &shape.stroke {
        crate::gradient_ui::editor(app, ui, layer, index, shape, true, true);
        dash_editor(app, ui, layer, index, shape, stroke);
    }
    if let xenify_doc::Geometry::Path { path } = &shape.geometry {
        simplify_row(app, ui, layer, index, shape, path);
    }
}

/// Point count + "Simplify" for path shapes.
fn simplify_row(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    layer: &Layer,
    index: usize,
    shape: &ShapeItem,
    path: &xenify_doc::Property<xenify_doc::BezPath>,
) {
    use xenify_doc::bezier;
    let current = path.value_at(app.time);
    ui.horizontal(|ui| {
        ui.weak(format!("{} points", bezier::point_count(&current)));
        let key = egui::Id::new("simplify-tolerance");
        let mut tol = ui.ctx().data(|d| d.get_temp::<f64>(key)).unwrap_or(1.0);
        ui.add(egui::Slider::new(&mut tol, 0.1..=20.0).logarithmic(true).suffix(" px"))
            .on_hover_text("How far the simplified outline may deviate from the original");
        ui.ctx().data_mut(|d| d.insert_temp(key, tol));
        let animated = path.is_animated();
        let r = ui
            .add_enabled(!animated, egui::Button::new("Simplify"))
            .on_hover_text("Fewer points, same look (smooth runs are refitted; corners are kept)")
            .on_disabled_hover_text("Simplifying an animated path would break its morph — remove its keyframes first");
        if r.clicked() {
            let simple = bezier::simplify(&current, tol);
            let (before, after) = (bezier::point_count(&current), bezier::point_count(&simple));
            let mut new = shape.clone();
            new.geometry = xenify_doc::Geometry::Path { path: simple.into() };
            if app.exec("Simplify path", Command::SetShape { layer: layer.id, index, shape: new }) {
                app.status = format!("Simplified: {before} → {after} points.");
            }
        }
    });
}

/// A stroke's dash pattern: presets, or dash and gap lengths typed in.
fn dash_editor(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    layer: &Layer,
    index: usize,
    shape: &ShapeItem,
    stroke: &xenify_doc::Stroke,
) {
    let w = stroke.width.value_at(app.time).max(1.0);
    let text_key = egui::Id::new(("dash-text", layer.id, index));
    let current: String =
        stroke.dashes.iter().map(|d| format!("{}", (d * 10.0).round() / 10.0)).collect::<Vec<_>>().join(" ");
    let mut text = ui.ctx().data(|d| d.get_temp::<String>(text_key)).unwrap_or_else(|| current.clone());
    let mut set: Option<(Vec<f64>, Option<xenify_doc::LineCap>)> = None;
    ui.horizontal(|ui| {
        ui.label("Dashes").on_hover_text("Dash and gap lengths in pixels, like SVG stroke-dasharray (e.g. 12 6)");
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0).hint_text("solid"));
        if r.changed() {
            ui.ctx().data_mut(|d| d.insert_temp(text_key, text.clone()));
        }
        if r.lost_focus() {
            let v: Vec<f64> =
                text.split([' ', ',']).filter_map(|t| t.trim().parse::<f64>().ok()).filter(|v| *v >= 0.0).collect();
            ui.ctx().data_mut(|d| d.remove::<String>(text_key));
            if v != stroke.dashes {
                set = Some((v, None));
            }
        } else if !r.has_focus() && text != current {
            ui.ctx().data_mut(|d| d.remove::<String>(text_key));
        }
        for (label, dashes, cap) in [
            ("Solid", vec![], None),
            ("Dashed", vec![w * 3.0, w * 2.0], Some(xenify_doc::LineCap::Butt)),
            ("Dotted", vec![0.01, w * 2.0], Some(xenify_doc::LineCap::Round)),
        ] {
            if ui.small_button(label).clicked() {
                set = Some((dashes, cap));
            }
        }
    });
    if let Some((dashes, cap)) = set {
        let mut new = shape.clone();
        let st = new.stroke.as_mut().unwrap();
        st.dashes = dashes;
        if let Some(c) = cap {
            st.cap = c;
        }
        app.exec("Dashes", Command::SetShape { layer: layer.id, index, shape: new });
    }
}

/// A small preview strip of a gradient's colours.
fn gradient_swatch(ui: &mut egui::Ui, g: &xenify_doc::Gradient) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, 16.0), egui::Sense::hover());
    let n = 24;
    let w = rect.width() / n as f32;
    for i in 0..n {
        let c = g.color_at((i as f64 + 0.5) / n as f64);
        let x = rect.left() + i as f32 * w;
        let color = Color32::from_rgba_unmultiplied(
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
            (c.a * 255.0) as u8,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w + 0.5, 16.0)),
            0.0,
            color,
        );
    }
    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
}

fn text_editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, text: &xenify_doc::TextLayer) {
    let mut content = text.text.clone();
    let r = ui.add(
        egui::TextEdit::multiline(&mut content).desired_rows(2).desired_width(f32::INFINITY).hint_text("Type here…"),
    );
    if std::mem::take(&mut app.focus_text_edit) {
        r.request_focus();
        // Select the placeholder so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(content.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.gained_focus() {
        app.new_gesture();
    }
    if r.changed() {
        // One undo step per editing session (merged by gesture).
        app.exec_merged("Edit text", Command::SetText { layer: layer.id, text: content });
    }

    text_box_controls(app, ui, layer, text);
    ui.horizontal(|ui| {
        use xenify_doc::{TextAlign, TextVAlign};
        let boxed = text.frame.is_some() && text.path.is_none();
        ui.label("Align").on_hover_text(if boxed {
            "Where the lines sit inside the text box"
        } else {
            "Which point of the text sits on its position (give it a text box to align inside one)"
        });
        let mut align = text.align;
        let mut valign = text.valign;
        let (left, centre, right) = if boxed {
            ("Lines start at the box's left edge", "Lines are centred in the box", "Lines end at the box's right edge")
        } else {
            ("Position is the left edge", "Position is the centre", "Position is the right edge")
        };
        for (a, label, tip) in
            [(TextAlign::Left, "Left", left), (TextAlign::Center, "Centre", centre), (TextAlign::Right, "Right", right)]
        {
            ui.selectable_value(&mut align, a, label).on_hover_text(tip);
        }
        ui.separator();
        // In a box the text starts at the top edge: no separate baseline.
        let options: &[(TextVAlign, &str)] = if boxed {
            &[(TextVAlign::Top, "Top"), (TextVAlign::Middle, "Middle"), (TextVAlign::Bottom, "Bottom")]
        } else {
            &[
                (TextVAlign::Top, "Top"),
                (TextVAlign::Middle, "Middle"),
                (TextVAlign::Baseline, "Baseline"),
                (TextVAlign::Bottom, "Bottom"),
            ]
        };
        let shown = if boxed && valign == TextVAlign::Baseline { TextVAlign::Top } else { valign };
        egui::ComboBox::from_id_salt(("valign", layer.id))
            .width(80.0)
            .selected_text(options.iter().find(|o| o.0 == shown).map_or("", |o| o.1))
            .show_ui(ui, |ui| {
                for (v, label) in options {
                    ui.selectable_value(&mut valign, *v, *label);
                }
            })
            .response
            .on_hover_text(if boxed {
                "Where the lines sit inside the box vertically"
            } else {
                "Which part of the text sits on its position vertically"
            });
        if (align, valign) != (text.align, text.valign) {
            app.exec("Align text", Command::SetTextAlign { layer: layer.id, align, valign });
        }
    });
    ui.horizontal(|ui| {
        ui.label("Font");
        let families = app.viewport.font_families().to_vec();
        egui::ComboBox::from_id_salt(("font", layer.id)).selected_text(&text.font_family).height(400.0).show_ui(
            ui,
            |ui| {
                for family in &families {
                    if ui.selectable_label(*family == text.font_family, family).clicked() {
                        app.exec("Change font", Command::SetFontFamily { layer: layer.id, family: family.clone() });
                    }
                }
            },
        );
    });
    text_options(app, ui, layer, text);
}

/// Point text or a text box: its size, wrapping and overflow.
fn text_box_controls(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, text: &xenify_doc::TextLayer) {
    if text.path.is_some() {
        return;
    }
    let mut frame = text.frame;
    let mut boxed = frame.is_some();
    ui.horizontal(|ui| {
        ui.checkbox(&mut boxed, "Text box").on_hover_text(
            "Wrap and align the text inside a box (drag its edges with the Text tool, T). Off: point text that grows as you type",
        );
        if boxed != frame.is_some() {
            frame = if boxed {
                // Start with a box around the text as it is now.
                let b = app.viewport.layer_text_bounds(app, layer.id).unwrap_or(kurbo::Rect::new(0.0, 0.0, 400.0, 100.0));
                Some(xenify_doc::TextFrame::new(b.inflate(2.0, 2.0)))
            } else {
                None
            };
        }
    });
    let mut began = false;
    if let Some(f) = &mut frame {
        let u = app.comp().settings.units;
        let (mut w, mut h) = (u.from_px(f.rect.width()), u.from_px(f.rect.height()));
        ui.horizontal(|ui| {
            ui.label("Box");
            let r = ui.add(
                egui::DragValue::new(&mut w)
                    .speed(u.from_px(1.0))
                    .range(u.from_px(1.0)..=f64::MAX)
                    .max_decimals(u.decimals()),
            );
            began |= r.drag_started();
            ui.label("×");
            let r = ui.add(
                egui::DragValue::new(&mut h)
                    .speed(u.from_px(1.0))
                    .range(u.from_px(1.0)..=f64::MAX)
                    .max_decimals(u.decimals()),
            );
            began |= r.drag_started();
            ui.weak(u.unit.suffix());
        });
        let (w, h) = (u.to_px(w), u.to_px(h));
        if (w - f.rect.width()).abs() > 1e-9 || (h - f.rect.height()).abs() > 1e-9 {
            f.rect = kurbo::Rect::from_origin_size(f.rect.origin(), (w, h));
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.wrap, "Wrap lines").on_hover_text("Break lines that are wider than the box");
            ui.checkbox(&mut f.clip, "Hide overflow")
                .on_hover_text("Crop text that doesn't fit in the box (CSS overflow: hidden); off lets it spill out");
        });
    }
    if began {
        app.new_gesture();
    }
    if frame != text.frame {
        app.exec_merged("Text box", Command::SetTextFrame { layer: layer.id, frame });
    }
}

/// Spacing, line height and text on a path.
fn text_options(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, text: &xenify_doc::TextLayer) {
    let (mut letter, mut word, mut line, mut path) =
        (text.letter_spacing, text.word_spacing, text.line_height, text.path);
    let mut began = false;
    egui::Grid::new(("text-opts", layer.id)).num_columns(2).show(ui, |ui| {
        ui.label("Letter spacing").on_hover_text("Extra space between letters (negative tightens)");
        let r = ui.add(egui::DragValue::new(&mut letter).speed(0.1).range(-50.0..=500.0).suffix(" px"));
        began |= r.drag_started();
        ui.end_row();
        ui.label("Word spacing");
        let r = ui.add(egui::DragValue::new(&mut word).speed(0.2).range(-50.0..=1000.0).suffix(" px"));
        began |= r.drag_started();
        ui.end_row();
        ui.label("Line height")
            .on_hover_text("Distance between lines, as a multiple of the font size (0 = the font's own)");
        let r = ui.add(egui::DragValue::new(&mut line).speed(0.01).range(0.0..=5.0).suffix("×"));
        began |= r.drag_started();
        ui.end_row();

        ui.label("On path").on_hover_text("Set the text along the outline of a shape layer");
        let shapes: Vec<(Id, String)> = app
            .comp()
            .layers
            .iter()
            .filter(|l| matches!(l.kind, LayerKind::Shape { .. }))
            .map(|l| (l.id, l.meta.name.clone()))
            .collect();
        let current =
            path.and_then(|p| shapes.iter().find(|(id, _)| *id == p.layer)).map_or("None".into(), |s| s.1.clone());
        egui::ComboBox::from_id_salt(("text-path", layer.id)).selected_text(current).show_ui(ui, |ui| {
            if ui.selectable_label(path.is_none(), "None (straight lines)").clicked() {
                path = None;
            }
            for (id, name) in &shapes {
                if ui.selectable_label(path.is_some_and(|p| p.layer == *id), name).clicked() {
                    path = Some(xenify_doc::TextPath {
                        layer: *id,
                        ..path.unwrap_or(xenify_doc::TextPath { layer: *id, offset: 0.0, flip: false })
                    });
                }
            }
        });
        ui.end_row();
        if let Some(p) = &mut path {
            ui.label("Start")
                .on_hover_text("Distance along the outline before the text (from the middle for centred text)");
            let r = ui.add(egui::DragValue::new(&mut p.offset).speed(1.0).suffix(" px"));
            began |= r.drag_started();
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut p.flip, "Other way round")
                .on_hover_text("Run the text the other direction (e.g. inside a circle)");
            ui.end_row();
        }
    });
    if began {
        app.new_gesture();
    }
    if (letter, word, line, path) != (text.letter_spacing, text.word_spacing, text.line_height, text.path) {
        let cmd = Command::SetTextOptions {
            layer: layer.id,
            letter_spacing: letter,
            word_spacing: word,
            line_height: line,
            path,
        };
        app.exec_merged("Text spacing", cmd);
    }
}
