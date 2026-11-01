//! Xenify Draw's engine for the HTML app. One [`Session`] is one open
//! canvas: its project and edit history, the selection and the playhead.
//! The screens talk to it with [`Session::call`] (a name and JSON
//! arguments, JSON back), the same on the desktop and in the browser.
//!
//! Drawing is done by the caller: [`Session::paint`] encodes the current
//! frame into any [`xenify_render::Painter`].

mod ai;
mod anim;
mod comps;
mod diagram;
mod files;
mod handles;
mod keys;
mod look;
mod model;
mod raster;
mod remote;
mod sample;
mod samples;
mod shapes;
mod slides;
mod snap;
mod templates;
pub use templates::layout_preview;

/// A pen (its style as stored) drawing a wavy line on a clear `w`×`h` tile, for the Pens panel.
pub fn pen_preview(style: &Json, w: u32, h: u32) -> Option<Scene> {
    let pen: xenify_doc::PenStyle = serde_json::from_value(style.clone()).ok()?;
    let (wf, hf) = (w as f64, h as f64);
    let mut path = kurbo::BezPath::new();
    path.move_to((14.0, hf * 0.65));
    path.curve_to((wf * 0.3, -hf * 0.2), (wf * 0.6, hf * 1.2), (wf - 14.0, hf * 0.35));
    let mut shape = ShapeItem::path(path, None, Some((Color::rgb(0.11, 0.1, 0.09), 7.0)));
    if let Some(s) = shape.stroke.as_mut() {
        s.style = pen;
        s.cap = xenify_doc::LineCap::Round;
    }
    let mut comp = Project::new().compositions.remove(0);
    comp.settings.width = w;
    comp.settings.height = h;
    comp.settings.background = Color::rgba(0.0, 0.0, 0.0, 0.0);
    comp.layers.push(Layer::shape(Id(1), "pen", vec![shape]));
    Some(xenify_anim::evaluate(&comp, 0.0))
}
pub use slides::transition_sample;
mod sound;

use kurbo::{Affine, Point, Rect, Vec2};
use serde_json::{Value as Json, json};
use xenify_anim::{Scene, SceneContent};
pub use xenify_doc::Id;
use xenify_doc::{AnyProperty, Color, Layer, LayerKind, Project, PropPath, Property, ShapeItem, TextLayer, Value};
use xenify_ops::{Command, Editor};
use xenify_render::{Painter, TextEngine};

pub use ai::PngRenderer;
pub use model::Pose;
pub use samples::Sample;

/// A project with one empty canvas (what a new tab holds until it's set up).
pub fn empty_project() -> Project {
    Project::new()
}

pub struct Session {
    /// How new strokes look (a pen, ends, dashes), when one is chosen: shapes drawn from now on take it.
    pub(crate) new_stroke: Option<Json>,
    /// The layer (folder) you're on: what you draw goes into it.
    pub(crate) layer_into: Option<Id>,
    /// Exporting: no checkerboard, and see-through canvases drawn on this colour when there is one.
    pub(crate) exporting: Option<Option<Color>>,
    pub editor: Editor,
    pub comp: Id,
    pub time: f64,
    pub selection: Vec<Id>,
    /// New values on animated properties become keyframes; on static ones
    /// they replace the value.
    pub auto_key: bool,
    pub text: TextEngine,
    /// Where each layer of an ongoing move/resize started.
    gesture: Vec<xenify_ops::transform::Snapshot>,
    gesture_seq: u64,
    /// The text layer being typed into: the screens show the words in an
    /// editing box over it, so it isn't drawn.
    pub editing: Option<Id>,
    /// Edit Points: the outlines a points drag started from (gesture, layer, per shape).
    point_start: (u64, Id, Vec<(usize, kurbo::BezPath)>),
    /// The point calls work on masks (the mask tool) rather than shapes.
    on_masks: bool,
    /// The canvas a symbol was opened from (to go back to).
    symbol_return: Option<Id>,
    /// Draw what the canvas's camera sees (else the canvas as laid out).
    pub look_through: bool,
    /// The last Duplicate's originals and copies (Duplicate again repeats how the copies were moved).
    last_duplicate: Vec<(Id, Id)>,
    assistant: ai::Assistant,
    /// Kept on a Xenify server (see remote.rs).
    pub remote: Option<remote::Remote>,
    /// Raster layers being painted (see raster.rs).
    raster: raster::Raster,
    /// The 3D model being turned in Insert › 3D model (see model.rs).
    model_stage: Option<model::Staged>,
}

impl Default for Session {
    fn default() -> Self {
        Self::sample(Sample::Poster)
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A whole-number argument (0 when missing).
fn u(args: &Json, key: &str) -> usize {
    args.get(key).and_then(Json::as_u64).unwrap_or(0) as usize
}

fn id_arg(args: &Json, key: &str) -> Result<Id, String> {
    args.get(key).and_then(Json::as_u64).map(Id).ok_or_else(|| format!("missing {key}"))
}

fn f(args: &Json, key: &str) -> Option<f64> {
    args.get(key).and_then(Json::as_f64)
}

pub(crate) fn color_json(c: Color) -> Json {
    json!(c.to_hex())
}

fn parse_color(v: &Json) -> Option<Color> {
    Color::from_hex(v.as_str()?)
}

/// The properties the Layer panel shows, by the names the screens use.
pub(crate) fn prop_path(name: &str) -> Option<PropPath> {
    use xenify_doc::ShapeField;
    Some(match name {
        "position" => PropPath::Position,
        "scale" => PropPath::Scale,
        "rotation" => PropPath::Rotation,
        "opacity" => PropPath::Opacity,
        "anchor" => PropPath::Anchor,
        "font_size" => PropPath::FontSize,
        "text_color" => PropPath::TextColor,
        "fill" => PropPath::shape(0, ShapeField::FillColor),
        "stroke" => PropPath::shape(0, ShapeField::StrokeColor),
        "stroke_width" => PropPath::shape(0, ShapeField::StrokeWidth),
        "size" => PropPath::shape(0, ShapeField::Size),
        "corner_radius" => PropPath::shape(0, ShapeField::CornerRadius),
        // Any other property by its path ("shapes.1.stroke_width", "effects.0.blur", …).
        other => return other.parse().ok(),
    })
}

const PROP_NAMES: [&str; 12] = [
    "position",
    "scale",
    "rotation",
    "opacity",
    "anchor",
    "font_size",
    "text_color",
    "fill",
    "stroke",
    "stroke_width",
    "size",
    "corner_radius",
];

/// Every property of `l` by the name the screens use: the short names
/// above for the common ones, else its path ("effects.0.radius").
pub(crate) fn prop_names(l: &Layer) -> Vec<(String, PropPath)> {
    let mut out: Vec<(String, PropPath)> = PROP_NAMES
        .iter()
        .filter_map(|n| Some((n.to_string(), prop_path(n)?)))
        .filter(|(_, p)| l.property(*p).is_ok())
        .collect();
    for p in l.property_paths() {
        if !out.iter().any(|(_, q)| *q == p) {
            out.push((p.to_string(), p));
        }
    }
    out
}

pub(crate) fn value_json(v: &Value) -> Json {
    match v {
        Value::Scalar(s) => json!(s),
        Value::Vec2(p) => json!([p.x, p.y]),
        Value::Color(c) => color_json(*c),
        _ => Json::Null,
    }
}

/// A value from the screens, of the kind `like` is.
pub(crate) fn json_value(v: &Json, like: &Value) -> Option<Value> {
    Some(match like {
        Value::Scalar(_) => Value::Scalar(v.as_f64()?),
        Value::Vec2(_) => {
            let a = v.as_array()?;
            Value::Vec2(Vec2::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?))
        }
        Value::Color(_) => Value::Color(parse_color(v)?),
        Value::Stops(_) => Value::Stops(serde_json::from_value(v.clone()).ok()?),
        _ => return None,
    })
}

/// Canvas settings beyond the size: units and resolution, grid spacing (0: automatic), motion blur and kind.
fn apply_comp_extras(s: &mut xenify_doc::CompSettings, args: &Json) {
    if let Some(u) = args.get("units") {
        if let Some(unit) = u.get("unit").and_then(|v| serde_json::from_value(v.clone()).ok()) {
            s.units.unit = unit;
        }
        if let Some(d) = u.get("dpi").and_then(Json::as_f64) {
            s.units.dpi = d.clamp(10.0, 2400.0);
        }
    }
    if let Some(g) = f(args, "grid") {
        s.grid = g.max(0.0);
    }
    match args.get("motion_blur") {
        Some(Json::Null) => s.motion_blur = None,
        Some(m) => {
            let mut mb = s.motion_blur.unwrap_or_default();
            if let Some(a) = f(m, "shutter_angle") {
                mb.shutter_angle = a.clamp(10.0, 720.0);
            }
            if let Some(n) = m.get("samples").and_then(Json::as_u64) {
                mb.samples = n.clamp(2, 32) as u32;
            }
            s.motion_blur = Some(mb);
        }
        None => {}
    }
    if let Some(k) = args.get("kind").and_then(|v| serde_json::from_value(v.clone()).ok()) {
        xenify_ops::diagram::set_kind(s, k);
    }
    // Defaults for new connectors drawn here.
    if let Some(d) = args.get("diagram").and_then(|v| serde_json::from_value(v.clone()).ok()) {
        s.diagram = Some(d);
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

fn kind_name(l: &Layer) -> &'static str {
    match &l.kind {
        LayerKind::Shape { .. } => "shape",
        LayerKind::Text(_) => "text",
        LayerKind::Null => "null",
        LayerKind::Group => "group",
        LayerKind::Precomp { .. } => "precomp",
        LayerKind::Solid(_) => "3d",
        LayerKind::Image { .. } => "picture",
        LayerKind::Video(_) => "video",
        LayerKind::Audio(_) => "sound",
        LayerKind::Camera(_) => "camera",
        LayerKind::Particles(_) => "particles",
        LayerKind::Liquid(_) => "water",
    }
}

/// Ready-made effect `name` in a little scene of its own at `time` (the effect gallery's previews).
pub fn effect_preview(name: &str, time: f64) -> Option<Scene> {
    let (p, comp) = xenify_ops::effects::preview(name)?;
    Some(xenify_anim::evaluate_project(&p, comp, time))
}

impl Session {
    pub fn new(project: Project) -> Self {
        let comp = project.compositions.first().map(|c| c.id).unwrap_or(Id(0));
        Self {
            editor: Editor::new(project),
            comp,
            time: 0.0,
            selection: Vec::new(),
            auto_key: true,
            text: TextEngine::new(),
            gesture: Vec::new(),
            gesture_seq: 0,
            editing: None,
            point_start: (0, Id(0), Vec::new()),
            on_masks: false,
            symbol_return: None,
            look_through: false,
            last_duplicate: Vec::new(),
            assistant: Default::default(),
            new_stroke: None,
            layer_into: None,
            exporting: None,
            remote: None,
            raster: Default::default(),
            model_stage: None,
        }
    }

    /// An example, shown at a moment when everything in it is on screen.
    pub fn sample(s: Sample) -> Self {
        let mut me = Self::new(s.project());
        me.time = s.showcase_time();
        me
    }

    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    fn comp_settings(&self) -> Option<&xenify_doc::CompSettings> {
        self.project().comp(self.comp).ok().map(|c| &c.settings)
    }

    /// The current frame, evaluated.
    pub fn scene(&self) -> Scene {
        let mut scene = xenify_anim::evaluate_project(self.project(), self.comp, self.time);
        if let Some(Some(bg)) = self.exporting
            && scene.background.a < 0.999
        {
            scene.background = bg;
        }
        scene
    }

    /// Draws the current frame into `out`, composition space mapped through `view`.
    pub fn paint(&mut self, out: &mut impl Painter, view: Affine) {
        // The text being typed in stays drawn: the screens draw the caret and selection over it.
        let mut scene = self.scene();
        self.raster_onion_skin(&mut scene);
        let camera = if self.look_through { scene.camera } else { Affine::IDENTITY };
        // A see-through background shows as a checkerboard (only here; exports leave it see-through or ask).
        if scene.background.a < 0.999 && self.exporting.is_none() {
            checkerboard(out, view, scene.width as f64, scene.height as f64);
        }
        xenify_render::encode_seen(&scene, out, view, camera, &mut self.text);
    }

    fn exec(&mut self, label: &str, cmd: Command) -> Result<(), String> {
        self.editor.execute(label, cmd).map_err(err)
    }

    fn exec_all(&mut self, label: &str, cmds: Vec<Command>) -> Result<(), String> {
        match cmds.len() {
            0 => Ok(()),
            1 => self.exec(label, cmds.into_iter().next().unwrap_or(Command::Batch { commands: Vec::new() })),
            _ => self.exec(label, Command::Batch { commands: cmds }),
        }
    }

    /// Answers one call from the screens.
    pub fn call(&mut self, name: &str, args: &Json) -> Result<Json, String> {
        if let Some(r) = self.remote_call(name, args) {
            return r;
        }
        match name {
            "state" => Ok(self.state()),
            "layer" => self.layer_info(id_arg(args, "id")?),
            "set_time" => {
                let end = self.comp_settings().map_or(0.0, |s| s.duration);
                self.time = f(args, "time").unwrap_or(0.0).clamp(0.0, end);
                Ok(json!(self.time))
            }
            "select" => {
                let ids: Vec<Id> = args
                    .get("ids")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_u64).map(Id).collect())
                    .unwrap_or_default();
                let p = self.project();
                self.selection = ids.into_iter().filter(|id| p.layer(*id).is_ok()).collect();
                // Picking something puts you on its layer (nothing picked: you stay where you were).
                if let Some(&first) = self.selection.first() {
                    self.layer_into = self.folder_of(first);
                }
                Ok(json!(self.selection.iter().map(|i| i.0).collect::<Vec<_>>()))
            }
            // A new layer (a folder for what you draw), on top, and you're on it.
            "add_folder" => {
                let id = self.next_id();
                let n =
                    self.project().comp(self.comp).map_or(0, |c| c.layers.iter().filter(|l| l.meta.folder).count()) + 1;
                let name =
                    args.get("name").and_then(Json::as_str).map(str::to_string).unwrap_or_else(|| format!("Layer {n}"));
                let mut layer = Layer::new(id, name, LayerKind::Group);
                layer.meta.folder = true;
                self.exec("New layer", Command::AddLayer { comp: self.comp, index: None, layer })?;
                self.layer_into = Some(id);
                self.selection.clear();
                Ok(json!(id.0))
            }
            "set_active_layer" => {
                self.layer_into = args.get("id").and_then(Json::as_u64).map(Id).filter(|id| self.is_folder(*id));
                Ok(json!(self.layer_into.map(|i| i.0)))
            }
            "hit" => {
                let at = Point::new(f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
                let tolerance = f(args, "tolerance").unwrap_or(4.0);
                let scene = self.scene();
                // `skip`: layers to pass over (with everything hanging from them).
                let skip: Vec<Id> = args
                    .get("skip")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_u64).map(Id).collect())
                    .unwrap_or_default();
                let hit = if skip.is_empty() {
                    xenify_anim::hit_test(&scene, at, tolerance, &mut self.text)
                } else {
                    let c = self.editor.project().comp(self.comp).map_err(err)?;
                    let mut out: Vec<Id> = skip.clone();
                    for s in &skip {
                        out.extend(xenify_anim::descendants(c, *s));
                    }
                    xenify_anim::hit_test_skipping(&scene, at, tolerance, &mut self.text, &|id| out.contains(&id))
                };
                // Locked layers can't be picked on the canvas.
                let hit = hit.filter(|id| self.project().layer(*id).is_ok_and(|l| !l.meta.locked));
                // `group`: a click picks the whole group a shape is in (Ctrl+click reaches inside).
                let hit = match (
                    hit,
                    args.get("group").and_then(Json::as_bool).unwrap_or(false),
                    self.project().comp(self.comp),
                ) {
                    (Some(h), true, Ok(c)) => Some(xenify_anim::outermost_group(c, h)),
                    (h, _, _) => h,
                };
                Ok(hit.map_or(Json::Null, |i| json!(i.0)))
            }
            "bounds" => {
                let ids: Option<Vec<Id>> = args
                    .get("ids")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_u64).map(Id).collect());
                let ids = ids.unwrap_or_else(|| self.selection.clone());
                Ok(self.bounds_json(&ids))
            }
            // Each layer's box on the canvas now (the screen-reader list of objects).
            "layer_boxes" => {
                let scene = self.scene();
                let c = self.project().comp(self.comp).map_err(err)?.clone();
                let out: Vec<Json> = c
                    .layers
                    .iter()
                    .filter_map(|l| {
                        let b = xenify_anim::world_box(&c, &scene, l.id, &mut self.text)?;
                        Some(json!({ "id": l.id.0, "box": [b.x0, b.y0, b.x1, b.y1] }))
                    })
                    .collect();
                Ok(json!(out))
            }
            "transform_begin" => self.transform_begin(),
            // Everything between these is one undo step (dragging a number, for one).
            "begin_gesture" => {
                let label = args.get("label").and_then(Json::as_str).unwrap_or("Change");
                self.editor.begin_group(label);
                Ok(Json::Null)
            }
            "end_gesture" => {
                self.editor.end_group();
                Ok(Json::Null)
            }
            "transform" => {
                let c: Vec<f64> = args
                    .get("affine")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_f64).collect())
                    .unwrap_or_default();
                if c.len() != 6 {
                    return Err("affine needs 6 numbers".into());
                }
                self.transform(Affine::new([c[0], c[1], c[2], c[3], c[4], c[5]]))?;
                Ok(Json::Null)
            }
            "distort" => {
                // Corner pin: the gesture's layers, taking the quad `from` to `to` (composition space).
                let quad = |k: &str| -> Option<xenify_doc::Quad> {
                    let a = args.get(k)?.as_array()?;
                    let mut q = [Vec2::ZERO; 4];
                    for (i, p) in a.iter().take(4).enumerate() {
                        q[i] = Vec2::new(p.get(0)?.as_f64()?, p.get(1)?.as_f64()?);
                    }
                    Some(xenify_doc::Quad(q))
                };
                let (from, to) = (quad("from").ok_or("missing from")?, quad("to").ok_or("missing to")?);
                let g = xenify_doc::Homography::from_quads(&from, &to).ok_or("those corners don't make a shape")?;
                let cmds: Vec<Command> =
                    self.gesture.iter().filter_map(|s| s.distort_command(g, self.time, self.auto_key)).collect();
                if !cmds.is_empty() {
                    let key = format!("gesture:{}", self.gesture_seq);
                    self.editor.execute_merged("Perspective", key, Command::Batch { commands: cmds }).map_err(err)?;
                }
                Ok(Json::Null)
            }
            "clear_distort" => {
                let cmds = self.selection.iter().map(|&layer| Command::SetDistort { layer, distort: None }).collect();
                self.exec_all("Remove perspective", cmds)?;
                Ok(Json::Null)
            }
            "transform_end" => {
                self.gesture.clear();
                Ok(Json::Null)
            }
            "nudge" => {
                let d = Vec2::new(f(args, "dx").unwrap_or(0.0), f(args, "dy").unwrap_or(0.0));
                self.transform_begin()?;
                self.transform(Affine::translate(d))?;
                self.gesture.clear();
                Ok(Json::Null)
            }
            "add_shape" => self.add_shape(args),
            "add_text" => self.add_text(args),
            "add_path" => self.add_path(args),
            "add_picture" => self.add_picture(args),
            "delete" => {
                // A group goes with everything in it (removing only the group
                // would leave its layers loose at the top of the list).
                let comp = self.project().comp(self.comp).map_err(err)?;
                let mut order: Vec<Id> = Vec::new();
                fn take(c: &xenify_doc::Composition, id: Id, order: &mut Vec<Id>) {
                    if order.contains(&id) {
                        return;
                    }
                    if c.layers.iter().any(|l| l.id == id && matches!(l.kind, xenify_doc::LayerKind::Group)) {
                        for child in c.layers.iter().filter(|l| l.meta.parent == Some(id)) {
                            take(c, child.id, order);
                        }
                    }
                    order.push(id);
                }
                for &id in &self.selection {
                    take(comp, id, &mut order);
                }
                let cmds = order.into_iter().map(|layer| Command::RemoveLayer { layer }).collect();
                self.exec_all("Delete", cmds)?;
                self.selection.clear();
                Ok(Json::Null)
            }
            "duplicate" => self.duplicate(),
            "reorder" => {
                let layer = id_arg(args, "id")?;
                let index = args.get("index").and_then(Json::as_u64).ok_or("missing index")? as usize;
                self.exec("Reorder", Command::MoveLayer { layer, index })?;
                Ok(Json::Null)
            }
            "set_meta" => {
                let id = id_arg(args, "id")?;
                let mut meta = self.project().layer(id).map_err(err)?.meta.clone();
                if let Some(n) = args.get("name").and_then(Json::as_str) {
                    meta.name = n.to_string();
                }
                if let Some(v) = args.get("visible").and_then(Json::as_bool) {
                    meta.visible = v;
                }
                if let Some(v) = args.get("locked").and_then(Json::as_bool) {
                    meta.locked = v;
                }
                // How many parent joints bend when this layer is dragged (a joint chain).
                if let Some(n) = args.get("ik_chain").and_then(Json::as_u64) {
                    meta.rig.ik_chain = n.min(12) as _;
                }
                self.exec("Change layer", Command::SetLayerMeta { layer: id, meta })?;
                Ok(Json::Null)
            }
            "set_prop" => {
                let id = id_arg(args, "id")?;
                let pname = args.get("prop").and_then(Json::as_str).unwrap_or("");
                let path = prop_path(pname).ok_or_else(|| format!("no property {pname}"))?;
                let l = self.project().layer(id).map_err(err)?;
                let like = l.value_at(path, self.time).map_err(err)?;
                let value = json_value(args.get("value").unwrap_or(&Json::Null), &like).ok_or("bad value")?;
                let mut cmds = vec![
                    Command::edit_value(self.project(), id, path, self.time, value.clone(), self.auto_key)
                        .map_err(err)?,
                ];
                // The other selected layers too: the same value, or (when they differ) moved by the same amount.
                let others = self.shared_targets(id, path);
                let mixed = others.iter().any(|o| {
                    self.project()
                        .layer(*o)
                        .ok()
                        .and_then(|ol| ol.value_at(path, self.time).ok())
                        .is_some_and(|ov| ov != like)
                });
                for o in others {
                    let Ok(theirs) = self.project().layer(o).and_then(|ol| ol.value_at(path, self.time)) else {
                        continue;
                    };
                    let target = match (mixed, &like, &value, &theirs) {
                        (true, Value::Scalar(a), Value::Scalar(b), Value::Scalar(c)) => Value::Scalar(c + (b - a)),
                        (true, Value::Vec2(a), Value::Vec2(b), Value::Vec2(c)) => Value::Vec2(*c + (*b - *a)),
                        _ => value.clone(),
                    };
                    cmds.push(
                        Command::edit_value(self.project(), o, path, self.time, target, self.auto_key).map_err(err)?,
                    );
                }
                // Dragging a number merges into one undo step.
                let key = format!("prop:{}:{pname}:{}", id.0, self.selection.len());
                self.editor.execute_merged(format!("Change {pname}"), key, Command::batch(cmds)).map_err(err)?;
                Ok(Json::Null)
            }
            "toggle_anim" => {
                let id = id_arg(args, "id")?;
                let pname = args.get("prop").and_then(Json::as_str).unwrap_or("");
                let path = prop_path(pname).ok_or_else(|| format!("no property {pname}"))?;
                let l = self.project().layer(id).map_err(err)?;
                let (prop, value) = (l.property(path).map_err(err)?, l.value_at(path, self.time).map_err(err)?);
                if prop.is_animated() {
                    self.exec(
                        "Remove animation",
                        Command::SetProperty { layer: id, path, property: static_property(value) },
                    )?;
                } else {
                    let time = self.time;
                    self.exec("Add keyframe", Command::SetKeyframe { layer: id, path, time, value, easing: None })?;
                }
                Ok(Json::Null)
            }
            "move_key" => {
                let id = id_arg(args, "id")?;
                let pname = args.get("prop").and_then(Json::as_str).unwrap_or("");
                let path = prop_path(pname).ok_or_else(|| format!("no property {pname}"))?;
                let (from, to) = (f(args, "from").ok_or("missing from")?, f(args, "to").ok_or("missing to")?);
                self.exec("Move keyframe", Command::MoveKeyframe { layer: id, path, from, to })?;
                Ok(Json::Null)
            }
            "remove_key" => {
                let id = id_arg(args, "id")?;
                let pname = args.get("prop").and_then(Json::as_str).unwrap_or("");
                let path = prop_path(pname).ok_or_else(|| format!("no property {pname}"))?;
                let time = f(args, "time").ok_or("missing time")?;
                self.exec("Remove keyframe", Command::RemoveKeyframe { layer: id, path, time })?;
                Ok(Json::Null)
            }
            "set_text" => {
                let layer = id_arg(args, "id")?;
                let text = args.get("text").and_then(Json::as_str).unwrap_or("").to_string();
                // A text box grows downwards as lines are added, as in most programs (never shrinks while typing).
                let kind = self.project().layer(layer).map_err(err)?.kind.clone();
                let grow = match &kind {
                    LayerKind::Text(t) => t.frame.filter(|f| f.wrap && !f.clip).and_then(|f| {
                        let mut probe = xenify_anim::SceneText::new(
                            text.clone(),
                            t.font_family.clone(),
                            t.font_size.value_at(self.time),
                            t.color.value_at(self.time),
                        );
                        probe.align = t.align;
                        probe.valign = xenify_doc::TextVAlign::Top;
                        probe.letter_spacing = t.letter_spacing;
                        probe.line_height = t.line_height;
                        probe.frame = Some(f);
                        let need = self.text.layout(&probe).bounds.y1;
                        (need > f.rect.y1 + 0.5).then(|| xenify_doc::TextFrame {
                            rect: Rect::new(f.rect.x0, f.rect.y0, f.rect.x1, need.ceil()),
                            ..f
                        })
                    }),
                    _ => None,
                };
                let mut cmds = vec![Command::SetText { layer, text }];
                if let Some(frame) = grow {
                    cmds.push(Command::SetTextFrame { layer, frame: Some(frame) });
                }
                let cmd = if cmds.len() == 1 { cmds.remove(0) } else { Command::Batch { commands: cmds } };
                self.editor.execute_merged("Type", format!("text:{}", layer.0), cmd).map_err(err)?;
                Ok(Json::Null)
            }
            "set_font" => {
                let layer = id_arg(args, "id")?;
                let family = args.get("family").and_then(Json::as_str).unwrap_or("Inter").to_string();
                self.exec("Font", Command::SetFontFamily { layer, family })?;
                Ok(Json::Null)
            }
            "fonts" => Ok(json!(self.text.families())),
            // Layers panel drags: above or below `target`, or into group `target`.
            "move_layers" => {
                let ids: Vec<Id> = args
                    .get("ids")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_u64).map(Id).collect())
                    .unwrap_or_default();
                let target = id_arg(args, "target")?;
                let place = match args.get("place").and_then(Json::as_str).unwrap_or("above") {
                    "below" => xenify_ops::order::Placement::Below(target),
                    "into" => xenify_ops::order::Placement::IntoGroup(target),
                    _ => xenify_ops::order::Placement::Above(target),
                };
                match xenify_ops::order::move_layers(self.project(), self.comp, &ids, place).map_err(err)? {
                    Some(cmd) => self.exec("Move layers", cmd).map(|_| json!(true)),
                    None => Ok(json!(false)),
                }
            }
            "set_new_stroke" => {
                self.new_stroke = args.get("stroke").filter(|v| !v.is_null()).cloned();
                Ok(Json::Null)
            }
            "new_stroke" => Ok(self.new_stroke.clone().unwrap_or(Json::Null)),
            // The eyedropper: the fill and stroke there, and the one colour showing.
            "sample_color" => {
                let at = kurbo::Point::new(f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
                let scene = self.scene();
                let s = sample::sample(&scene, at, f(args, "tolerance").unwrap_or(3.0), &mut self.text);
                Ok(json!({
                    "fill": s.fill.map(color_json), "stroke": s.stroke.map(|(c, _)| color_json(c)),
                    "stroke_width": s.stroke.map(|(_, w)| w), "visible": color_json(s.visible),
                }))
            }
            "image_playback" => {
                let id = id_arg(args, "id")?;
                let l = self.project().layer(id).map_err(err)?;
                let LayerKind::Image { asset, playback } = l.kind else { return Ok(Json::Null) };
                let a = self.project().asset(asset).ok_or("the picture is missing")?;
                Ok(match &a.frames {
                    Some(f) => {
                        json!({ "frames": f.len(), "length": a.animation_length(), "speed": playback.speed, "offset": playback.offset, "looped": playback.looped })
                    }
                    None => Json::Null,
                })
            }
            "set_image_playback" => {
                let layer = id_arg(args, "id")?;
                let LayerKind::Image { playback, .. } = self.project().layer(layer).map_err(err)?.kind else {
                    return Err("not a picture".into());
                };
                let mut p = if args.get("reset").and_then(Json::as_bool).unwrap_or(false) {
                    xenify_doc::ImagePlayback::default()
                } else {
                    playback
                };
                if let Some(v) = f(args, "speed") {
                    p.speed = v.clamp(0.0, 16.0);
                }
                if let Some(v) = f(args, "offset") {
                    p.offset = v.max(0.0);
                }
                if let Some(v) = args.get("looped").and_then(Json::as_bool) {
                    p.looped = v;
                }
                self.exec("Animated image playback", Command::SetImagePlayback { layer, playback: p })?;
                Ok(Json::Null)
            }
            // The moments a frame at `time` averages (just `time` without motion blur).
            "shutter_times" => Ok(json!(
                self.comp_settings().map(|s| s.shutter_times(f(args, "time").unwrap_or(self.time))).unwrap_or_default()
            )),
            "fonts_used" => {
                let mut used: Vec<String> = self
                    .project()
                    .compositions
                    .iter()
                    .flat_map(|c| &c.layers)
                    .filter_map(|l| match &l.kind {
                        LayerKind::Text(t) => Some(t.font_family.clone()),
                        _ => None,
                    })
                    .collect();
                used.sort();
                used.dedup();
                Ok(json!(used))
            }
            "set_text_style" => self.set_text_style(args),
            "set_comp" => {
                let mut s = self.comp_settings().cloned().ok_or("no canvas")?;
                if let Some(n) = args.get("name").and_then(Json::as_str) {
                    s.name = n.to_string();
                }
                if let Some(w) = f(args, "width") {
                    s.width = w.clamp(1.0, 16384.0) as u32;
                }
                if let Some(h) = f(args, "height") {
                    s.height = h.clamp(1.0, 16384.0) as u32;
                }
                if let Some(d) = f(args, "duration") {
                    s.duration = d.clamp(0.0, 3600.0);
                }
                if let Some(fps) = f(args, "fps") {
                    s.fps = fps.clamp(1.0, 240.0);
                }
                if let Some(c) = args.get("background").and_then(parse_color) {
                    s.background = c;
                }
                apply_comp_extras(&mut s, args);
                let comp = self.comp;
                self.exec("Canvas settings", Command::SetCompSettings { comp, settings: s })?;
                Ok(Json::Null)
            }
            "undo" | "redo" => {
                let undo = name == "undo";
                if let Some(label) = self.raster_undo_sel(undo) {
                    return Ok(json!(label));
                }
                let done = if undo { self.editor.undo() } else { self.editor.redo() };
                // What was selected may be gone now (undoing its drawing).
                self.selection.retain(|id| self.editor.project().layer(*id).is_ok());
                Ok(json!(done))
            }
            "new" => {
                let mut p = Project::new();
                if let Some(c) = p.compositions.first_mut() {
                    let s = &mut c.settings;
                    s.name = args.get("name").and_then(Json::as_str).unwrap_or("Canvas").to_string();
                    s.width = f(args, "width").unwrap_or(1080.0).clamp(1.0, 16384.0) as u32;
                    s.height = f(args, "height").unwrap_or(1350.0).clamp(1.0, 16384.0) as u32;
                    s.duration = f(args, "duration").unwrap_or(6.0).clamp(0.0, 3600.0);
                    s.fps = f(args, "fps").unwrap_or(30.0).clamp(1.0, 240.0);
                    // See-through unless asked otherwise: exports keep it, or ask for a colour where the format can't.
                    s.background =
                        args.get("background").and_then(parse_color).unwrap_or(Color::rgba(1.0, 1.0, 1.0, 0.0));
                    apply_comp_extras(s, args);
                }
                // A presentation: in a theme (five starter slides), or one empty slide.
                if let Some(theme) = templates::theme_arg(args.get("template")) {
                    let s = p.compositions[0].settings.clone();
                    p = xenify_ops::templates::new_deck(theme, s.width, s.height, &xenify_ops::templates::STARTER);
                } else if args.get("deck").and_then(Json::as_bool).unwrap_or(false) {
                    p.slides = vec![xenify_doc::Slide::new(p.compositions[0].id)];
                    // Slides are shown on a screen: white unless asked otherwise.
                    if args.get("background").is_none() {
                        p.compositions[0].settings.background = Color::WHITE;
                    }
                }
                // A blank drawing starts with "Layer 1", which what you draw goes into (not diagrams or slides).
                let plain = p.slides.is_empty() && args.get("kind").and_then(Json::as_str) != Some("diagram");
                let first = (plain && args.get("layers").and_then(Json::as_bool).unwrap_or(true)).then(|| {
                    let id = p.alloc_id();
                    let mut l = Layer::new(id, "Layer 1", LayerKind::Group);
                    l.meta.folder = true;
                    p.compositions[0].layers.push(l);
                    id
                });
                self.replace(Self::new(p));
                self.layer_into = first;
                Ok(self.state())
            }
            "sample" => {
                let s = args.get("name").and_then(Json::as_str).and_then(Sample::from_name).ok_or("no such example")?;
                self.replace(Self::sample(s));
                Ok(self.state())
            }
            "open" => {
                let text = args.get("json").and_then(Json::as_str).ok_or("missing json")?;
                let p = Project::from_json(text).map_err(err)?;
                self.replace(Self::new(p));
                Ok(self.state())
            }
            "save" => {
                let text = self.project().to_json();
                self.editor.mark_saved();
                Ok(json!(text))
            }
            // Exports: `on`, with `background` behind a see-through canvas (formats that can't keep see-through).
            "export_mode" => {
                self.exporting = args
                    .get("on")
                    .and_then(Json::as_bool)
                    .unwrap_or(false)
                    .then(|| args.get("background").and_then(parse_color));
                Ok(Json::Null)
            }
            "save_copy" => {
                // With `background`: see-through canvases get that colour (for exports that can't keep see-through).
                match args.get("background").and_then(parse_color) {
                    Some(bg) => {
                        let mut p = self.project().clone();
                        for c in &mut p.compositions {
                            if c.settings.background.a < 0.999 {
                                c.settings.background = bg;
                            }
                        }
                        Ok(json!(p.to_json()))
                    }
                    None => Ok(json!(self.project().to_json())),
                }
            }
            "mark_unsaved" => {
                self.editor.mark_unsaved();
                Ok(Json::Null)
            }
            "texts" => Ok(self.texts()),
            "text_box" => self.text_box(id_arg(args, "id")?),
            "text_rects" => {
                let id = id_arg(args, "id")?;
                let ranges: Vec<(usize, usize)> =
                    serde_json::from_value(args.get("ranges").cloned().unwrap_or_default()).map_err(err)?;
                Ok(self.text_rects(id, &ranges))
            }
            "edit_text" => {
                self.editing = args.get("id").and_then(Json::as_u64).map(Id);
                Ok(Json::Null)
            }
            // Typing on the canvas (text layers and shape labels alike), with byte offsets into the text:
            // the caret and selection as canvas quads; what a click picks; moving a line or to a line's end.
            "text_caret" => {
                let id = id_arg(args, "id")?;
                let (a, b) = (u(args, "anchor"), u(args, "focus"));
                let scene = self.scene();
                let (m, t) = scene_text(&scene, id).ok_or("no text on screen now")?;
                let layout = self.text.layout(&t);
                let quad = |r: Rect| {
                    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
                        .map(|p| {
                            let q = m * p;
                            [q.x, q.y]
                        })
                };
                Ok(
                    json!({ "caret": quad(layout.caret(b)), "selection": layout.selection(a.min(b), a.max(b)).into_iter().map(quad).collect::<Vec<_>>() }),
                )
            }
            "text_pick" => {
                let id = id_arg(args, "id")?;
                let scene = self.scene();
                let (m, t) = scene_text(&scene, id).ok_or("no text on screen now")?;
                if m.determinant().abs() < 1e-12 {
                    return Err("text is squashed flat".into());
                }
                let p = m.inverse() * Point::new(f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
                let (a, b) = self.text.layout(&t).pick(p, u(args, "unit").clamp(1, 3) as u8);
                Ok(json!({ "anchor": a, "focus": b }))
            }
            "text_step" => {
                let id = id_arg(args, "id")?;
                let scene = self.scene();
                let (_, t) = scene_text(&scene, id).ok_or("no text on screen now")?;
                let lines = args.get("lines").and_then(Json::as_i64).unwrap_or(0) as isize;
                let edge = args.get("edge").and_then(Json::as_i64).unwrap_or(0).clamp(-1, 1) as i8;
                let extend = args.get("extend").and_then(Json::as_bool).unwrap_or(false);
                let (a, b) = self.text.layout(&t).step(u(args, "anchor"), u(args, "focus"), lines, edge, extend);
                Ok(json!({ "anchor": a, "focus": b }))
            }
            "samples" => {
                Ok(json!(Sample::ALL.iter().map(|s| json!({"name": s.name(), "title": s.title()})).collect::<Vec<_>>()))
            }
            _ => self
                .shape_call(name, args)
                .or_else(|| self.look_call(name, args))
                .or_else(|| self.anim_call(name, args))
                .or_else(|| self.keys_call(name, args))
                .or_else(|| self.templates_call(name, args))
                .or_else(|| self.files_call(name, args))
                .or_else(|| self.raster_call(name, args))
                .or_else(|| self.comps_call(name, args))
                .or_else(|| self.slides_call(name, args))
                .or_else(|| self.sound_call(name, args))
                .or_else(|| self.model_call(name, args))
                .or_else(|| self.diagram_call(name, args))
                .or_else(|| self.snap_call(name, args))
                .or_else(|| self.ai_call(name, args))
                .or_else(|| self.handles_call(name, args))
                .unwrap_or_else(|| Err(format!("unknown call {name}"))),
        }
    }

    fn state(&self) -> Json {
        let p = self.project();
        let Ok(c) = p.comp(self.comp) else { return json!({}) };
        let s = &c.settings;
        // Children (by parent) follow their group, painted top first like the Layers panel.
        let mut rows = Vec::new();
        fn add(rows: &mut Vec<Json>, layers: &[Layer], parent: Option<Id>, depth: usize) {
            // Layers parented to a layer that isn't a group (a joint chain) stay in the top list.
            let in_group = |l: &Layer| {
                l.meta
                    .parent
                    .and_then(|p| layers.iter().find(|x| x.id == p))
                    .is_some_and(|p| matches!(p.kind, LayerKind::Group | LayerKind::Null))
            };
            for l in
                layers.iter().rev().filter(|l| if parent.is_none() { !in_group(l) } else { l.meta.parent == parent })
            {
                let keys: Vec<f64> = {
                    let mut k: Vec<f64> =
                        l.property_paths().into_iter().filter_map(|p| l.keyframe_times(p).ok()).flatten().collect();
                    k.sort_by(f64::total_cmp);
                    k.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
                    k
                };
                rows.push(json!({
                    "id": l.id.0,
                    "name": l.meta.name,
                    "kind": kind_name(l),
                    "depth": depth,
                    "visible": l.meta.visible,
                    "locked": l.meta.locked,
                    "in": l.meta.in_point,
                    "out": l.meta.out_point,
                    "keys": keys,
                    // For the Layers panel's badges and effect rows.
                    "fx": l.meta.effects.iter().map(|e| json!({ "kind": serde_json::to_value(&e.kind).ok().and_then(|v| v.get("effect").cloned()), "enabled": e.enabled })).collect::<Vec<_>>(),
                    "masks": l.meta.masks.len(),
                    "mods": l.meta.modifiers.len(),
                    "matte": l.meta.matte.is_some(),
                    "group": matches!(l.kind, LayerKind::Group),
                    "folder": l.meta.folder,
                    "members": layers.iter().filter(|x| x.meta.parent == Some(l.id)).count(),
                }));
                if matches!(l.kind, LayerKind::Group | LayerKind::Null) {
                    add(rows, layers, Some(l.id), depth + 1);
                }
            }
        }
        add(&mut rows, &c.layers, None, 0);
        // Pictures of documents on a server, kept up to date (Layer › Keep this picture).
        for row in &mut rows {
            let linked = row["id"].as_u64().and_then(|id| c.layers.iter().find(|l| l.id.0 == id)).is_some_and(|l| {
                matches!(&l.kind, LayerKind::Image { asset, .. } if p.asset(*asset).is_some_and(|a| a.name.starts_with("xen-")))
            });
            if linked {
                row["linked"] = json!(true);
            }
            // A still picture: you can paint on it (the raster tools).
            let raster = row["id"].as_u64().and_then(|id| c.layers.iter().find(|l| l.id.0 == id)).is_some_and(|l| {
                matches!(&l.kind, LayerKind::Image { asset, .. } if p.asset(*asset).is_some_and(|a| a.frames.is_none()))
            });
            if raster {
                row["raster"] = json!(true);
            }
            // Drawn from a 3D model: Turn the 3D model again.
            let model = row["id"].as_u64().and_then(|id| c.layers.iter().find(|l| l.id.0 == id)).is_some_and(|l| {
                matches!(&l.kind, LayerKind::Image { asset, .. } if p.asset(*asset).is_some_and(|a| a.model.is_some()))
            });
            if model {
                row["model"] = json!(true);
            }
        }
        json!({
            "comp": {
                "id": c.id.0, "name": s.name, "width": s.width, "height": s.height,
                "fps": s.fps, "duration": s.duration, "background": color_json(s.background), "pauses": s.pauses,
                "units": { "unit": s.units.unit, "dpi": s.units.dpi, "px_per_unit": s.units.px_per_unit(), "suffix": s.units.unit.suffix(), "decimals": s.units.decimals() },
                "grid": s.grid, "grid_spacing": s.grid_spacing(), "motion_blur": s.motion_blur, "kind": s.kind,
                "diagram": s.diagram.clone().unwrap_or_default(),
                "last_key": c.layers.iter().flat_map(|l| l.property_paths().into_iter().flat_map(move |p| l.keyframe_times(p).unwrap_or_default())).fold(0.0f64, f64::max),
            },
            "symbol": (!xenify_ops::slides::slides_or_main(p).iter().any(|x| x.comp == c.id)).then(|| s.name.clone()),
            "presentation": !p.slides.is_empty(),
            "layers": rows,
            "selection": self.selection.iter().map(|i| i.0).collect::<Vec<_>>(),
            "active_layer": self.active_folder().map(|i| i.0),
            "time": self.time,
            "dirty": self.editor.is_dirty(),
            "revision": self.editor.revision(),
            "undo": self.raster_sel_label(true).or(self.editor.undo_label()),
            "redo": self.raster_sel_label(false).or(self.editor.redo_label()),
        })
    }

    /// Layer `id` alone, as it looks now, fitted in a clear `size`×`size` tile (the Layers panel's thumbnails).
    pub fn layer_preview(&mut self, id: Id, size: u32) -> Option<Scene> {
        // A hidden layer still has a picture of its own (a reference picture kept out of sight, say).
        let hidden = self.project().layer(id).is_ok_and(|l| !l.meta.visible);
        let scene = if hidden {
            let mut p = self.project().clone();
            for c in &mut p.compositions {
                if let Some(l) = c.layers.iter_mut().find(|l| l.id == id) {
                    l.meta.visible = true;
                }
            }
            xenify_anim::evaluate_project(&p, self.comp, self.time)
        } else {
            self.scene()
        };
        let sl = scene.layer(id)?.clone();
        // Its box as drawn (bounds are in its own space).
        let b = sl.transform.transform_rect_bbox(sl.bounds(&mut self.text)?);
        if b.width() < 1e-6 && b.height() < 1e-6 {
            return None;
        }
        let k = (size as f64 * 0.86) / b.width().max(b.height()).max(1e-6);
        let fit = Affine::translate((size as f64 / 2.0, size as f64 / 2.0))
            * Affine::scale(k)
            * Affine::translate(-b.center().to_vec2());
        let mut layer = sl;
        layer.transform = fit * layer.transform;
        Some(Scene {
            width: size,
            height: size,
            background: Color::rgba(0.0, 0.0, 0.0, 0.0),
            layers: vec![layer],
            camera: Affine::IDENTITY,
            time: scene.time,
        })
    }

    /// Becomes `other` (a new or opened canvas), keeping the fonts that were added.
    fn replace(&mut self, mut other: Session) {
        std::mem::swap(&mut other.text, &mut self.text);
        other.new_stroke = self.new_stroke.take();
        *self = other;
        // An opened canvas starts on its top layer, as a new one does on
        // "Layer 1": what you draw goes there, not loose at the top.
        self.layer_into = self.project().comp(self.comp).ok().and_then(|c| {
            c.layers
                .iter()
                .rev()
                .find(|l| {
                    l.meta.folder && l.meta.parent.is_none() && !l.meta.locked && matches!(l.kind, LayerKind::Group)
                })
                .map(|l| l.id)
        });
    }

    /// The other selected layers a change to `path` on `primary` also goes to: those with that property, holding the same kind of value.
    fn shared_targets(&self, primary: Id, path: PropPath) -> Vec<Id> {
        if self.selection.len() < 2 || !self.selection.contains(&primary) {
            return Vec::new();
        }
        let Ok(kind) = self.project().layer(primary).and_then(|l| l.property_kind(path)) else { return Vec::new() };
        self.selection
            .iter()
            .copied()
            .filter(|&o| {
                o != primary && self.project().layer(o).and_then(|l| l.property_kind(path)).is_ok_and(|k| k == kind)
            })
            .collect()
    }

    fn layer_info(&self, id: Id) -> Result<Json, String> {
        let l = self.project().layer(id).map_err(err)?;
        let mut props = serde_json::Map::new();
        for (name, path) in prop_names(l) {
            let (Ok(prop), Ok(v)) = (l.property(path), l.value_at(path, self.time)) else { continue };
            let value = match &v {
                Value::Stops(st) => serde_json::to_value(st).unwrap_or(Json::Null),
                other => value_json(other),
            };
            if value.is_null() {
                continue;
            }
            let others = self.shared_targets(id, path);
            let mixed = others.iter().any(|o| {
                self.project().layer(*o).ok().and_then(|ol| ol.value_at(path, self.time).ok()).is_some_and(|ov| ov != v)
            });
            props.insert(
                name,
                json!({ "value": value, "animated": prop.is_animated(), "keys": prop.keyframe_times(), "shared": others.len(), "mixed": mixed }),
            );
        }
        let text = match &l.kind {
            LayerKind::Text(t) => json!({
                "text": t.text, "family": t.font_family, "align": t.align, "valign": t.valign,
                "letter_spacing": t.letter_spacing, "word_spacing": t.word_spacing, "line_height": t.line_height,
                "path": t.path.map(|p| json!({ "layer": p.layer.0, "offset": p.offset, "flip": p.flip })),
                "frame": t.frame.map(|f| json!({ "w": f.rect.width(), "h": f.rect.height(), "wrap": f.wrap, "clip": f.clip })),
            }),
            _ => Json::Null,
        };
        let connector = l.meta.connector.is_some();
        let ik = self.project().comp(self.comp).map(|c| !xenify_ops::rig::ik_joints(c, id).is_empty()).unwrap_or(false);
        let paint = l
            .shapes()
            .first()
            .map_or(Json::Null, |s| json!({ "fill": s.fill.is_some(), "stroke": s.stroke.is_some() }));
        Ok(json!({
            "id": id.0, "name": l.meta.name, "kind": kind_name(l),
            "visible": l.meta.visible, "locked": l.meta.locked,
            "props": props, "text": text, "paint": paint, "ik": ik, "ik_chain": l.meta.rig.ik_chain, "has_parent": l.meta.parent.is_some(), "connector": connector, "role": l.meta.role,
        }))
    }

    /// Each selected layer's box (layer space) and where its corners are in composition space.
    fn boxes(&mut self, ids: &[Id]) -> Vec<(Id, Rect, [Point; 4])> {
        let scene = self.scene();
        let comp = self.project().comp(self.comp).ok().cloned();
        let mut out = Vec::new();
        for &id in ids {
            // A group draws nothing itself: its box is everything in it, in the group's own space.
            if let Some(c) = &comp
                && c.layer(id).is_some_and(|l| matches!(l.kind, LayerKind::Group | LayerKind::Null))
            {
                let Some(wb) = xenify_anim::world_box(c, &scene, id, &mut self.text) else { continue };
                let t = xenify_anim::world_transform(c, id, self.time);
                let b = if t.determinant().abs() > 1e-12 { t.inverse().transform_rect_bbox(wb) } else { wb };
                let corners = [
                    t * Point::new(b.x0, b.y0),
                    t * Point::new(b.x1, b.y0),
                    t * Point::new(b.x1, b.y1),
                    t * Point::new(b.x0, b.y1),
                ];
                out.push((id, b, corners));
                continue;
            }
            let Some(sl) = scene.layer(id) else { continue };
            let Some(b) = sl.bounds(&mut self.text) else { continue };
            let t = sl.transform;
            let corners = [
                t * Point::new(b.x0, b.y0),
                t * Point::new(b.x1, b.y0),
                t * Point::new(b.x1, b.y1),
                t * Point::new(b.x0, b.y1),
            ];
            out.push((id, b, corners));
        }
        out
    }

    fn bounds_json(&mut self, ids: &[Id]) -> Json {
        let boxes = self.boxes(ids);
        json!(
            boxes
                .iter()
                .map(|(id, _, c)| json!({ "id": id.0, "corners": c.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>() }))
                .collect::<Vec<_>>()
        )
    }

    fn transform_begin(&mut self) -> Result<Json, String> {
        let ids = self.selection.clone();
        let boxes = self.boxes(&ids);
        let mut snaps = Vec::new();
        for (id, b, _) in boxes {
            if self.project().layer(id).is_ok_and(|l| l.meta.locked) {
                continue;
            }
            snaps.push(xenify_ops::transform::Snapshot::new(self.project(), self.comp, id, self.time, b).map_err(err)?);
        }
        self.gesture = snaps;
        self.gesture_seq += 1;
        Ok(Json::Null)
    }

    /// Applies `x` (composition space) on top of where the gesture started.
    fn transform(&mut self, x: Affine) -> Result<(), String> {
        let mut cmds = Vec::new();
        for s in &self.gesture {
            cmds.extend(s.affine_commands(self.project(), x, self.time, self.auto_key).map_err(err)?);
        }
        if cmds.is_empty() {
            return Ok(());
        }
        let key = format!("gesture:{}", self.gesture_seq);
        self.editor.execute_merged("Move", key, Command::Batch { commands: cmds }).map_err(err)
    }

    fn add_layer(&mut self, label: &str, layer: Layer) -> Result<Json, String> {
        let id = layer.id;
        let comp = self.comp;
        let mut layer = Layer { meta: xenify_doc::LayerMeta { in_point: 0.0, ..layer.meta }, ..layer };
        // New strokes in the chosen pen, ends and dashes.
        if let (Some(ns), LayerKind::Shape { shapes }) = (&self.new_stroke, &mut layer.kind) {
            for st in shapes.iter_mut().filter_map(|s| s.stroke.as_mut()) {
                if let Some(pen) = ns.get("style").and_then(|v| serde_json::from_value(v.clone()).ok()) {
                    st.style = pen;
                }
                if let Some(cap) = ns.get("cap").and_then(|v| serde_json::from_value(v.clone()).ok()) {
                    st.cap = cap;
                }
                if let Some(r) = ns.get("cap_roundness").and_then(Json::as_f64) {
                    st.cap_roundness = r.clamp(0.0, 1.0);
                }
                if let Some(d) = ns.get("dashes").and_then(|v| serde_json::from_value(v.clone()).ok()) {
                    st.dashes = d;
                }
            }
        }
        let index = self.place_in_active_folder(&mut layer)?;
        self.exec(label, Command::AddLayer { comp, index, layer })?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    /// Puts a new top-level layer into the layer you're on, at the top of it; the index to add it at.
    fn place_in_active_folder(&self, layer: &mut Layer) -> Result<Option<usize>, String> {
        let comp = self.comp;
        let mut index = None;
        if let Some(g) = self.active_folder().filter(|_| !layer.meta.folder && layer.meta.parent.is_none()) {
            let c = self.project().comp(comp).map_err(err)?;
            let world = xenify_anim::world_transform(c, g, self.time);
            let [a, b, cc, d, tx, ty] = world.as_coeffs();
            // A layer that's moved but not turned or scaled (as layers are): the new one is put in its space.
            if (a - 1.0).abs() < 1e-9 && b.abs() < 1e-9 && cc.abs() < 1e-9 && (d - 1.0).abs() < 1e-9 {
                let off = Vec2::new(tx, ty);
                layer.transform.position =
                    match std::mem::replace(&mut layer.transform.position, Property::Static(Vec2::ZERO)) {
                        Property::Static(v) => Property::Static(v - off),
                        Property::Animated(keys) => Property::Animated(
                            keys.into_iter().map(|k| xenify_doc::Keyframe { value: k.value - off, ..k }).collect(),
                        ),
                    };
                layer.meta.parent = Some(g);
                index = c.layer_index(g);
            }
        }
        Ok(index)
    }

    fn is_folder(&self, id: Id) -> bool {
        self.project()
            .comp(self.comp)
            .ok()
            .and_then(|c| c.layer(id))
            .is_some_and(|l| l.meta.folder && matches!(l.kind, LayerKind::Group))
    }

    /// The folder `id` is in (or `id` itself, if it is one).
    pub(crate) fn folder_of(&self, id: Id) -> Option<Id> {
        let c = self.project().comp(self.comp).ok()?;
        let mut cur = Some(id);
        for _ in 0..=c.layers.len() {
            let l = c.layer(cur?)?;
            if l.meta.folder && matches!(l.kind, LayerKind::Group) {
                return Some(l.id);
            }
            cur = l.meta.parent;
        }
        None
    }

    /// The layer you're on, if it's still there and not locked.
    pub(crate) fn active_folder(&self) -> Option<Id> {
        let id = self.layer_into?;
        let l = self.project().comp(self.comp).ok()?.layer(id)?;
        (self.is_folder(id) && !l.meta.locked).then_some(id)
    }

    fn next_id(&self) -> Id {
        let mut p = self.project().clone();
        p.alloc_id()
    }

    fn add_shape(&mut self, args: &Json) -> Result<Json, String> {
        let (x, y) = (f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
        let (w, h) = (f(args, "w").unwrap_or(100.0).abs().max(1.0), f(args, "h").unwrap_or(100.0).abs().max(1.0));
        let fill = args.get("fill").and_then(parse_color).unwrap_or(Color::from_hex("#C2410C").unwrap_or(Color::BLACK));
        let kind = args.get("kind").and_then(Json::as_str).unwrap_or("rect");
        let size = Vec2::new(w, h);
        let (item, name) = match kind {
            "ellipse" => (ShapeItem::ellipse(size, fill), "Ellipse"),
            _ => (ShapeItem::rect(size, fill), "Rectangle"),
        };
        let id = self.next_id();
        let layer =
            Layer::shape(id, self.unique_name(name), vec![item]).with_position(Vec2::new(x + w / 2.0, y + h / 2.0));
        self.add_layer(&format!("Add {}", name.to_lowercase()), layer)
    }

    fn add_text(&mut self, args: &Json) -> Result<Json, String> {
        let (x, y) = (f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
        let text = args.get("text").and_then(Json::as_str).unwrap_or("Text").to_string();
        let size = f(args, "size").unwrap_or(48.0);
        let color =
            args.get("color").and_then(parse_color).unwrap_or(Color::from_hex("#1C1917").unwrap_or(Color::BLACK));
        let id = self.next_id();
        let mut t = TextLayer::new(text, xenify_render::DEFAULT_FAMILY, size, color);
        // A text box (its top-left at x, y): body text that wraps inside it.
        if let Some([w, h]) =
            args.get("box").and_then(Json::as_array).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]))
        {
            let (w, h) = (w.max(8.0), h.max(8.0));
            t.font_size = size.min((h * 0.8).round()).max(8.0).into();
            t.valign = xenify_doc::TextVAlign::Top;
            t.frame = Some(xenify_doc::TextFrame::new(kurbo::Rect::new(0.0, 0.0, w, h)));
        }
        let layer = Layer::new(id, self.unique_name("Text"), LayerKind::Text(t)).with_position(Vec2::new(x, y));
        self.add_layer("Add text", layer)
    }

    fn add_path(&mut self, args: &Json) -> Result<Json, String> {
        let pts: Vec<Point> = args
            .get("points")
            .and_then(Json::as_array)
            .map(|a| a.iter().filter_map(|p| Some(Point::new(p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect())
            .unwrap_or_default();
        if pts.len() < 2 {
            return Err("a path needs two points".into());
        }
        let closed = args.get("closed").and_then(Json::as_bool).unwrap_or(false);
        let smooth = args.get("smooth").and_then(Json::as_bool).unwrap_or(false);
        let stroke =
            args.get("stroke").and_then(parse_color).unwrap_or(Color::from_hex("#1C1917").unwrap_or(Color::BLACK));
        let width = f(args, "width").unwrap_or(4.0);
        let fill = if closed { args.get("fill").and_then(parse_color) } else { None };
        // Points are in composition space; the layer sits at their centre.
        let b = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
        let c = b.center().to_vec2();
        let local: Vec<Point> = pts.iter().map(|p| *p - c).collect();
        let path = if smooth {
            xenify_ops::draw::smooth_stroke(&local, closed)
        } else {
            let mut bp = kurbo::BezPath::new();
            bp.move_to(local[0]);
            for p in &local[1..] {
                bp.line_to(*p);
            }
            if closed {
                bp.close_path();
            }
            bp
        };
        let id = self.next_id();
        let item = ShapeItem::path(path, fill, Some((stroke, width)));
        let layer = Layer::shape(id, self.unique_name("Path"), vec![item]).with_position(c);
        self.add_layer("Draw path", layer)
    }

    /// A picture (its file's bytes in base64, its size measured by the
    /// screens), centred on the canvas and shrunk to fit inside it.
    fn add_picture(&mut self, args: &Json) -> Result<Json, String> {
        self.add_picture_with(args, None)
    }

    /// `add_picture`, for a picture drawn from 3D `model` (kept with it).
    pub(crate) fn add_picture_with(&mut self, args: &Json, model: Option<xenify_doc::ModelSource>) -> Result<Json, String> {
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing picture")?)
            .map_err(err)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Picture");
        let mime = args.get("mime").and_then(Json::as_str).unwrap_or("image/png");
        let (w, h) = (f(args, "width").unwrap_or(0.0), f(args, "height").unwrap_or(0.0));
        if w < 1.0 || h < 1.0 {
            return Err("couldn't read the picture's size".into());
        }
        let mut p = self.project().clone();
        // Animated GIF, WebP and PNG keep their frames' timing (they play with the timeline).
        let frames = xenify_render::images::frame_delays(&data);
        let (mut add, asset) = xenify_ops::assets::add_image(&mut p, name, mime, w as u32, h as u32, data, frames);
        if let (Some(Command::AddAsset { asset }), Some(m)) = (&mut add, model) {
            asset.model = Some(Box::new(m));
        }
        // A new picture is in the command that adds it; a known one is in the canvas already.
        let a = match &add {
            Some(Command::AddAsset { asset }) => asset.clone(),
            _ => p.asset(asset).cloned().ok_or("the picture didn't load")?,
        };
        let (cw, ch) = self.comp_settings().map_or((w, h), |s| (s.width as f64, s.height as f64));
        // A picture the canvas's size fills it (a canvas made for it); others fit inside with a margin.
        let fills = (w - cw).abs() < 0.5 && (h - ch).abs() < 0.5;
        let k = if fills { 1.0 } else { (cw * 0.8 / w).min(ch * 0.8 / h).min(1.0) };
        // Pixels pasted back: where they were copied from, at their size there.
        let k = f(args, "scale").unwrap_or(k);
        let at = args
            .get("at")
            .and_then(Json::as_array)
            .and_then(|a| Some(Vec2::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
        let mut layer = Layer::image(
            p.alloc_id(),
            self.unique_name(name.rsplit_once('.').map_or(name, |n| n.0)),
            &a,
            at.unwrap_or(Vec2::new(cw / 2.0, ch / 2.0)),
        );
        if (k - 1.0).abs() > 1e-9 {
            layer.transform.scale = Property::Static(Vec2::new(k, k));
        }
        let id = layer.id;
        let comp = self.comp;
        let index = match args.get("above").and_then(Json::as_u64) {
            Some(above) => self.project().comp(comp).ok().and_then(|c| c.layer_index(Id(above))).map(|i| i + 1),
            None => self.place_in_active_folder(&mut layer)?,
        };
        let mut cmds: Vec<Command> = add.into_iter().collect();
        cmds.push(Command::AddLayer { comp, index, layer });
        self.exec_all("Add picture", cmds)?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    /// Copies of the selected layers (with their groups' members). When the selection is the last
    /// Duplicate's copies and they were moved, turned or scaled alike, the new copies get that change
    /// again (step and repeat); otherwise they sit a little down and to the right.
    fn duplicate(&mut self) -> Result<Json, String> {
        let comp = self.project().comp(self.comp).map_err(err)?.clone();
        let selected = self.selection.clone();
        let mut sources: Vec<Id> = selected
            .iter()
            .copied()
            .filter(|id| {
                comp.layer(*id).is_some()
                    && !selected.iter().any(|g| g != id && xenify_anim::descendants(&comp, *g).contains(id))
            })
            .collect();
        sources.sort_by_key(|id| comp.layer_index(*id));
        if sources.is_empty() {
            return Err("Select layers to duplicate.".into());
        }
        let again = self.transform_again(&comp, &sources);
        // The sources and everything inside them, bottom first.
        let mut take: Vec<Id> = sources.clone();
        for id in &sources {
            take.extend(xenify_anim::descendants(&comp, *id));
        }
        let layers: Vec<Layer> = comp.layers.iter().filter(|l| take.contains(&l.id)).cloned().collect();
        let mut p = self.project().clone();
        let mut map = std::collections::HashMap::new();
        for l in &layers {
            map.insert(l.id, p.alloc_id());
        }
        let mut cmds = Vec::new();
        let mut pairs = Vec::new();
        for mut l in layers {
            let old = l.id;
            l.id = map[&old];
            l.meta.parent = l.meta.parent.and_then(|q| map.get(&q).copied().or(Some(q)));
            if sources.contains(&old) {
                pairs.push((old, l.id));
                l.meta.name = format!("{} copy", l.meta.name);
                if again.is_none() {
                    l.transform.position = match l.transform.position {
                        Property::Static(v) => Property::Static(v + Vec2::new(20.0, 20.0)),
                        other => other,
                    };
                }
            }
            cmds.push(Command::AddLayer { comp: self.comp, index: None, layer: l });
        }
        let mut cmds = adopt_after(cmds);
        let mut label = "Duplicate";
        if let Some(step) = again {
            let mut scratch = self.project().clone();
            if xenify_ops::apply(&mut scratch, Command::batch(cmds.clone())).is_ok() {
                for (_, id) in &pairs {
                    let snap = xenify_ops::transform::Snapshot::new(&scratch, self.comp, *id, self.time, Rect::ZERO);
                    if let Ok(c) = snap.and_then(|s| s.affine_commands(&scratch, step, self.time, self.auto_key)) {
                        cmds.extend(c);
                    }
                }
                label = "Duplicate and transform again";
            }
        }
        self.exec(label, Command::batch(cmds))?;
        self.selection = pairs.iter().map(|(_, c)| *c).collect();
        self.last_duplicate = pairs;
        Ok(json!({ "again": again.is_some() }))
    }

    /// How the last Duplicate's copies were changed since (composition space), when `sources` are
    /// exactly those copies and they all changed alike.
    fn transform_again(&self, comp: &xenify_doc::Composition, sources: &[Id]) -> Option<Affine> {
        let pairs = &self.last_duplicate;
        let mut copies: Vec<Id> = pairs.iter().map(|(_, c)| *c).collect();
        copies.sort_by_key(|id| comp.layer_index(*id));
        if copies.is_empty() || copies != sources {
            return None;
        }
        let delta = |(a, b): &(Id, Id)| -> Option<Affine> {
            comp.layer(*a)?;
            let (wa, wb) =
                (xenify_anim::world_transform(comp, *a, self.time), xenify_anim::world_transform(comp, *b, self.time));
            (wa.determinant().abs() > 1e-12).then(|| wb * wa.inverse())
        };
        let first = delta(&pairs[0])?;
        let close = |m: Affine, n: Affine| m.as_coeffs().iter().zip(n.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-6);
        let alike = pairs.iter().all(|p| delta(p).is_some_and(|d| close(d, first)));
        // Only a real change: the plain duplicate's own nudge doesn't count as one to repeat.
        let nudge = Affine::translate((20.0, 20.0));
        (alike && !close(first, Affine::IDENTITY) && !close(first, nudge)).then_some(first)
    }

    fn unique_name(&self, base: &str) -> String {
        let Ok(c) = self.project().comp(self.comp) else { return base.to_string() };
        let taken = |n: &str| c.layers.iter().any(|l| l.meta.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
    }

    /// Where a text layer's words are, for an editing box over them: the
    /// layer's transform (layer → composition), its box in layer space and
    /// how the words are set.
    fn text_box(&mut self, id: Id) -> Result<Json, String> {
        let scene = self.scene();
        let (transform, t) = scene_text(&scene, id).ok_or("no text on screen now")?;
        let t = &t;
        let layout = self.text.layout(t);
        let b = layout.bounds;
        let first = layout.lines.first().map_or(0.0, |l| l.baseline);
        let gap = match layout.lines.as_slice() {
            [a, b, ..] => b.baseline - a.baseline,
            _ => t.font_size * 1.2,
        };
        let [a, bb, c, d, e, f] = transform.as_coeffs();
        Ok(json!({
            "affine": [a, bb, c, d, e, f],
            "box": [b.x0, b.y0, b.x1, b.y1],
            "size": t.font_size,
            "line": gap,
            "first_baseline": first,
            "color": color_json(t.color),
            "align": format!("{:?}", t.align).to_lowercase(),
            "family": t.font_family,
            "frame": t.frame().map(|f| [f.rect.x0, f.rect.y0, f.rect.x1, f.rect.y1]),
        }))
    }

    /// A text layer's alignment, spacing, path and box. `box: true` gives it
    /// a box around the words as they are; `box: false` makes it point text.
    fn set_text_style(&mut self, args: &Json) -> Result<Json, String> {
        let layer = id_arg(args, "id")?;
        let LayerKind::Text(t) = &self.project().layer(layer).map_err(err)?.kind else {
            return Err("not a text layer".into());
        };
        let t = t.clone();
        let mut cmds = Vec::new();
        let (a, v) = (
            args.get("align").and_then(|a| serde_json::from_value(a.clone()).ok()),
            args.get("valign").and_then(|a| serde_json::from_value(a.clone()).ok()),
        );
        if a.is_some() || v.is_some() {
            cmds.push(Command::SetTextAlign { layer, align: a.unwrap_or(t.align), valign: v.unwrap_or(t.valign) });
        }
        let path = match args.get("path") {
            Some(Json::Null) => None,
            Some(p) => Some(xenify_doc::TextPath {
                layer: id_arg(p, "layer").or_else(|_| t.path.map(|x| x.layer).ok_or("missing layer"))?,
                offset: f(p, "offset").or(t.path.map(|x| x.offset)).unwrap_or(0.0),
                flip: p.get("flip").and_then(Json::as_bool).or(t.path.map(|x| x.flip)).unwrap_or(false),
            }),
            None => t.path,
        };
        let (ls, ws, lh) = (
            f(args, "letter_spacing").unwrap_or(t.letter_spacing),
            f(args, "word_spacing").unwrap_or(t.word_spacing),
            f(args, "line_height").unwrap_or(t.line_height),
        );
        if (ls, ws, lh, path) != (t.letter_spacing, t.word_spacing, t.line_height, t.path) {
            cmds.push(Command::SetTextOptions {
                layer,
                letter_spacing: ls,
                word_spacing: ws,
                line_height: lh.max(0.0),
                path,
            });
        }
        let mut frame = t.frame;
        match args.get("box").and_then(Json::as_bool) {
            Some(true) if frame.is_none() => {
                // Start with a box around the words as they are now.
                let scene = self.scene();
                let b = scene.layer(layer).and_then(|sl| match &sl.content {
                    SceneContent::Text(st) => Some(self.text.layout(st).bounds),
                    _ => None,
                });
                frame = Some(xenify_doc::TextFrame::new(
                    b.unwrap_or(kurbo::Rect::new(0.0, 0.0, 400.0, 100.0)).inflate(2.0, 2.0),
                ));
            }
            Some(false) => frame = None,
            _ => {}
        }
        if let Some(fr) = &mut frame {
            if let Some(w) = f(args, "box_w") {
                fr.rect.x1 = fr.rect.x0 + w.max(1.0);
            }
            if let Some(h) = f(args, "box_h") {
                fr.rect.y1 = fr.rect.y0 + h.max(1.0);
            }
            if let Some(r) = args.get("rect").and_then(Json::as_array) {
                let v: Vec<f64> = r.iter().filter_map(Json::as_f64).collect();
                if let [x0, y0, x1, y1] = v[..] {
                    fr.rect = kurbo::Rect::new(x0, y0, x1, y1).abs();
                }
            }
            if let Some(w) = args.get("wrap").and_then(Json::as_bool) {
                fr.wrap = w;
            }
            if let Some(c) = args.get("clip").and_then(Json::as_bool) {
                fr.clip = c;
            }
        }
        if frame != t.frame {
            cmds.push(Command::SetTextFrame { layer, frame });
        }
        if cmds.is_empty() {
            return Ok(Json::Null);
        }
        let key = format!("text_style:{}:{}", layer.0, args.get("merge").and_then(Json::as_str).unwrap_or(""));
        if args.get("merge").is_some() {
            self.editor.execute_merged("Text", key, Command::batch(cmds)).map_err(err)?;
        } else {
            self.exec("Text", Command::batch(cmds))?;
        }
        Ok(Json::Null)
    }

    /// Composition-space boxes (four corners) around byte ranges of a text
    /// layer's words, one per line a range is on: where to underline.
    fn text_rects(&mut self, id: Id, ranges: &[(usize, usize)]) -> Json {
        let scene = self.scene();
        let Some((m, t)) = scene_text(&scene, id) else { return json!([]) };
        let layout = self.text.layout(&t);
        let out: Vec<Json> = ranges
            .iter()
            .map(|&(s, e)| {
                let mut lines: std::collections::BTreeMap<usize, (f64, f64, f64, f64)> = Default::default();
                for run in &layout.runs {
                    for (i, g) in run.glyphs.iter().enumerate() {
                        let at = run.clusters.get(i).copied().unwrap_or(0);
                        if at < s || at >= e {
                            continue;
                        }
                        let (x0, x1) = (g.x as f64, (g.x + run.advances.get(i).copied().unwrap_or(0.0)) as f64);
                        let size = run.font_size as f64;
                        let r = lines.entry(run.line).or_insert((x0, x1, g.y as f64, size));
                        r.0 = r.0.min(x0);
                        r.1 = r.1.max(x1);
                    }
                }
                json!(
                    lines
                        .values()
                        .map(|&(x0, x1, y, size)| {
                            let r = Rect::new(x0, y - 0.8 * size, x1, y + 0.25 * size);
                            [
                                Point::new(r.x0, r.y0),
                                Point::new(r.x1, r.y0),
                                Point::new(r.x1, r.y1),
                                Point::new(r.x0, r.y1),
                            ]
                            .map(|p| {
                                let q = m * p;
                                [q.x, q.y]
                            })
                        })
                        .collect::<Vec<_>>()
                )
            })
            .collect();
        json!(out)
    }

    /// Every text layer's words, for spelling and grammar: its id, name,
    /// text and when it's first on screen.
    fn texts(&self) -> Json {
        let Ok(c) = self.project().comp(self.comp) else { return json!([]) };
        json!(
            c.layers
                .iter()
                .filter_map(|l| match (&l.kind, &l.meta.label) {
                    (LayerKind::Text(t), _) => Some(json!({
                        "id": l.id.0, "name": l.meta.name, "text": t.text,
                        "from": l.meta.in_point.max(first_visible(l)),
                    })),
                    // A shape's or connector's label.
                    (_, Some(label)) if !label.text.trim().is_empty() => Some(json!({
                        "id": l.id.0, "name": l.meta.name, "text": label.text, "label": true,
                        "from": l.meta.in_point.max(first_visible(l)),
                    })),
                    _ => None,
                })
                .collect::<Vec<_>>()
        )
    }
}

/// The text layer `id` draws, or the label on shape or connector `id`, with its layer → canvas transform.
fn scene_text(scene: &Scene, id: Id) -> Option<(Affine, xenify_anim::SceneText)> {
    fn find(l: &xenify_anim::SceneLayer, id: Id, outer: Affine) -> Option<(Affine, xenify_anim::SceneText)> {
        let m = outer * l.transform;
        match &l.content {
            SceneContent::Text(t) if l.id == id => Some((m, t.clone())),
            SceneContent::Group(inner) if l.id == id => inner.iter().find_map(|x| find(x, id, m)),
            _ => None,
        }
    }
    find(scene.layer(id)?, id, Affine::IDENTITY)
}

/// When a layer that fades in starts to show (the keyframe its fade starts
/// from), else 0.
fn first_visible(l: &Layer) -> f64 {
    let times = AnyProperty::Scalar(l.opacity.clone()).keyframe_times();
    match times.iter().position(|t| l.opacity.value_at(*t) > 0.0) {
        Some(0) | None => 0.0,
        Some(i) => times[i - 1],
    }
}

#[cfg(test)]
mod tests;

/// Layers added bottom first sit below their group, so they come before it: each is added without its
/// parent, then attached once everything is there.
pub(crate) fn adopt_after(cmds: Vec<Command>) -> Vec<Command> {
    let mut adds = Vec::new();
    let mut links = Vec::new();
    for c in cmds {
        match c {
            Command::AddLayer { comp, index, mut layer } if layer.meta.parent.is_some() => {
                links.push(Command::SetLayerMeta { layer: layer.id, meta: layer.meta.clone() });
                layer.meta.parent = None;
                adds.push(Command::AddLayer { comp, index, layer });
            }
            other => adds.push(other),
        }
    }
    adds.extend(links);
    adds
}

/// Light and lighter grey squares about 8 screen pixels across over the canvas `w` × `h` (the see-through background).
fn checkerboard(out: &mut impl Painter, view: Affine, w: f64, h: f64) {
    use xenify_render::peniko::{Color as PColor, Fill};
    let frame = kurbo::Rect::new(0.0, 0.0, w, h);
    out.fill(Fill::NonZero, view, PColor::from_rgba8(255, 255, 255, 255), None, &frame);
    let scale = view.as_coeffs()[0].abs().max(1e-6);
    let mut side = 8.0 / scale;
    // Never more than about 40 000 squares (big canvases zoomed in get bigger ones).
    while (w / side) * (h / side) > 80_000.0 {
        side *= 2.0;
    }
    let mut path = kurbo::BezPath::new();
    let (cols, rows) = ((w / side).ceil() as usize, (h / side).ceil() as usize);
    for r in 0..rows {
        for c in (r % 2..cols).step_by(2) {
            let (x0, y0) = (c as f64 * side, r as f64 * side);
            let sq = kurbo::Rect::new(x0, y0, (x0 + side).min(w), (y0 + side).min(h));
            path.extend(kurbo::Shape::path_elements(&sq, 0.1));
        }
    }
    out.fill(Fill::NonZero, view, PColor::from_rgba8(226, 226, 226, 255), None, &path);
}
