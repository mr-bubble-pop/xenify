use crate::dialogs::{Dialog, ExportJob};
use crate::timeline::TimelineState;
use crate::viewport::Viewport;
use crate::widgets::Icon;
use egui::Key;
use xenify_doc::{Composition, Id, Project, PropPath};
use xenify_ops::{Command, Editor};
use xenify_platform::FileStore;
use xenify_platform::native::NativeFs;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    /// Direct selection: edit the points of paths.
    Node,
    /// Rotate / scale around a movable centre; Shift+corner for perspective.
    FreeTransform,
    /// Drag corners to change the perspective (corner pin).
    Perspective,
    Rect,
    Ellipse,
    Line,
    /// Triangles, polygons and stars.
    Polygon,
    Pen,
    /// Freehand stroke, recognised as a clean shape when it looks like one.
    Pencil,
    /// Freehand filled brush stroke, also recognised as shapes.
    Brush,
    /// Push / attract / repel path points with a soft brush.
    Sculpt,
    Text,
    Eyedropper,
    /// Drag across shapes to set where their fill's or stroke's gradient
    /// starts and ends.
    Gradient,
    /// Resize the canvas or crop layers to a rectangle.
    Crop,
    /// Draw and reshape masks on the selected layer.
    Mask,
    /// Cut shapes in two along a dragged line.
    Knife,
    /// Named export regions.
    Slice,
    /// Lines glued between shapes that route themselves around others.
    Connector,
    Hand,
    Zoom,
    /// Drag to turn 3D objects.
    Orbit,
}

impl Tool {
    pub const ALL: [(Tool, Icon, &'static str, Option<Key>); 23] = [
        (
            Tool::Select,
            Icon::Select,
            "Select (V) — click to select, drag to move, drag on empty canvas (or Shift+drag) to box-select; corners and edges resize, just outside a corner \
             rotates, the ⊕ is its transform origin, the point it turns around (Alt+drag to move it). Shift keeps a move to one axis and rotation to 15°; Ctrl turns snapping off",
            Some(Key::V),
        ),
        (
            Tool::Node,
            Icon::Node,
            "Edit points (A) — click a shape to edit its points (rectangles and ellipses become paths). \
             Drag points, edges or handles; Shift+click or drag a box to select several. Right-click: \
             subdivide, delete, break, join, simplify. Double-click adds a point or makes one smooth/sharp",
            Some(Key::A),
        ),
        (
            Tool::FreeTransform,
            Icon::FreeTransform,
            "Free transform (Q) — drag inside to move, corners/edges to scale, outside a corner to rotate, \
             around the ⊕ centre (drag it to move it; double-click it to re-centre). The Perspective tool (W) distorts corners. \
             Alt: scale from the opposite side",
            Some(Key::Q),
        ),
        (
            Tool::Perspective,
            Icon::Perspective,
            "Perspective (W) — drag a corner: its neighbour moves the opposite way (a true perspective tilt). \
             Ctrl+drag moves one corner freely. The corners are keyframeable (Perspective in Properties)",
            Some(Key::W),
        ),
        (
            Tool::Rect,
            Icon::Rect,
            "Rectangle (R) — drag to draw, Shift for a square, Alt from the centre. Set Corners in the tool \
             options for rounded ones",
            Some(Key::R),
        ),
        (
            Tool::Ellipse,
            Icon::Ellipse,
            "Ellipse (E) — drag to draw, Shift for a circle, Alt from the centre",
            Some(Key::E),
        ),
        (
            Tool::Line,
            Icon::Line,
            "Line (L) — drag to draw, Shift snaps to 15° steps, Ctrl turns snapping off",
            Some(Key::L),
        ),
        (
            Tool::Polygon,
            Icon::Polygon,
            "Polygons, stars, gears, hearts, arrows and speech bubbles (Y) — pick one in the tool options, \
             drag to draw. Shift keeps it regular",
            Some(Key::Y),
        ),
        (
            Tool::Pen,
            Icon::Pen,
            "Pen (P) — click for corners, drag for curves; click the first point to close. Double-click, Enter or \
             Esc finishes an open path; Backspace takes back the last point",
            Some(Key::P),
        ),
        (
            Tool::Pencil,
            Icon::Pencil,
            "Pencil (N) — draw freely; lines, circles, rectangles and triangles snap into clean shapes \
             (turn off in the tool options, or hold Alt)",
            Some(Key::N),
        ),
        (
            Tool::Brush,
            Icon::Brush,
            "Brush (B) — paint a filled stroke; recognised shapes become clean shapes (hold Alt to keep the stroke). \
             [ and ] change the size",
            Some(Key::B),
        ),
        (
            Tool::Sculpt,
            Icon::Sculpt,
            "Sculpt (M) — push, attract (magnet) or repel the points of paths under the brush. \
             Alt flips attract/repel; [ and ] change the size",
            Some(Key::M),
        ),
        (
            Tool::Orbit,
            Icon::Cube,
            "3D orbit (O) — drag a 3D object to turn it (Shift: one axis at a time). Add 3D objects from \
             Layer › 3D, or extrude a shape there",
            Some(Key::O),
        ),
        (
            Tool::Text,
            Icon::Text,
            "Text (T) — click for a line of text or drag out a text box, then type right there; click text to edit it (Esc or click elsewhere when done)",
            Some(Key::T),
        ),
        (
            Tool::Eyedropper,
            Icon::Eyedropper,
            "Eyedropper (I) — click a shape to pick up its fill and stroke (and apply them to the selection)",
            Some(Key::I),
        ),
        (
            Tool::Gradient,
            Icon::Gradient,
            "Gradient (G) — drag across the selected shapes (or one under the pointer) to set where the gradient \
             starts and ends: the fill's, or the stroke's when the stroke swatch is active. Drag the ends to adjust. \
             Shift: 15° steps",
            Some(Key::G),
        ),
        (
            Tool::Crop,
            Icon::Crop,
            "Crop — drag the edges, then Enter or Apply: resizes the canvas, or with layers selected crops \
             them (with a mask you can adjust later). Esc cancels. Start it from Canvas › Crop Canvas or Layer › \
             Crop",
            None,
        ),
        (
            Tool::Knife,
            Icon::Knife,
            "Knife (C) — drag a line right across shapes to cut them in two (the selected ones, or any it \
             crosses). Shift: straight or 45°",
            Some(Key::C),
        ),
        (
            Tool::Mask,
            Icon::Mask,
            "Mask (U) — hide the parts of a layer outside a shape (like cropping): drag on the selected layer \
             to draw a rectangle or ellipse mask. \
             Drag corners, edges or points to reshape it, the inside to move it. Keyframeable; feather and invert \
             in Properties › Masks",
            Some(Key::U),
        ),
        (
            Tool::Slice,
            Icon::Slice,
            "Slice (Shift+C) — drag out regions to export as separate files (File › Export Slices). \
             Drag a slice to move it, Delete removes it",
            None,
        ),
        (
            Tool::Connector,
            Icon::Connector,
            "Connector (F) — drag from one shape to another for a line that stays glued to both and finds its \
             own way around other shapes. Start or end on a port (the circles) to use that side; Ctrl leaves an \
             end unglued. Select a connector to drag its ends or bends",
            Some(Key::F),
        ),
        (Tool::Hand, Icon::Hand, "Hand (H) — drag to pan (also: middle mouse, or scroll)", Some(Key::H)),
        (
            Tool::Zoom,
            Icon::Zoom,
            "Zoom (Z) — click to zoom in; right-click (or Shift/Ctrl/Alt+click) to zoom out; drag a box to zoom \
             to it. Scroll with Ctrl to zoom anywhere",
            Some(Key::Z),
        ),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Node => "Edit Points",
            Tool::FreeTransform => "Free Transform",
            Tool::Perspective => "Perspective",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Line => "Line",
            Tool::Polygon => "Shapes",
            Tool::Pen => "Pen",
            Tool::Pencil => "Pencil",
            Tool::Brush => "Brush",
            Tool::Sculpt => "Sculpt",
            Tool::Text => "Text",
            Tool::Eyedropper => "Eyedropper",
            Tool::Gradient => "Gradient",
            Tool::Crop => "Crop",
            Tool::Mask => "Mask",
            Tool::Knife => "Knife",
            Tool::Slice => "Slice",
            Tool::Connector => "Connector",
            Tool::Hand => "Hand",
            Tool::Zoom => "Zoom",
            Tool::Orbit => "3D Orbit",
        }
    }
}

/// Which colour the colour panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PaintSlot {
    #[default]
    Fill,
    Stroke,
}

/// The current fill and stroke: used by new shapes and edited in the colour panel.
#[derive(Clone, Debug)]
pub struct Paint {
    pub fill: Option<xenify_doc::Color>,
    pub stroke: Option<xenify_doc::Color>,
    pub stroke_width: f64,
    pub active: PaintSlot,
    /// Recently used colours, newest first.
    pub recent: Vec<xenify_doc::Color>,
    /// Pen, caps and joins for new strokes (the control bar's Stroke group).
    pub pen: xenify_doc::PenStyle,
    pub cap: xenify_doc::LineCap,
    pub cap_roundness: f64,
    pub join: xenify_doc::LineJoin,
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            fill: xenify_doc::Color::from_hex("#118ab2"),
            stroke: None,
            stroke_width: 4.0,
            active: PaintSlot::Fill,
            recent: Vec::new(),
            pen: xenify_doc::PenStyle::default(),
            cap: xenify_doc::LineCap::Round,
            cap_roundness: 0.0,
            join: xenify_doc::LineJoin::Round,
        }
    }
}

impl Paint {
    pub fn get(&self, slot: PaintSlot) -> Option<xenify_doc::Color> {
        match slot {
            PaintSlot::Fill => self.fill,
            PaintSlot::Stroke => self.stroke,
        }
    }

    pub fn set(&mut self, slot: PaintSlot, color: Option<xenify_doc::Color>) {
        match slot {
            PaintSlot::Fill => self.fill = color,
            PaintSlot::Stroke => self.stroke = color,
        }
    }

    /// `shape` with the current fill and stroke.
    pub fn apply_to(&self, mut shape: xenify_doc::ShapeItem) -> xenify_doc::ShapeItem {
        shape.fill = self.fill.map(xenify_doc::Fill::solid);
        shape.stroke = self.stroke.map(|c| xenify_doc::Stroke {
            cap_roundness: self.cap_roundness,
            style: self.pen.clone(),
            color: c.into(),
            width: self.stroke_width.into(),
            join: self.join,
            cap: self.cap,
            ..Default::default()
        });
        shape
    }

    /// Remembers `color` in the recent list.
    pub fn remember(&mut self, color: xenify_doc::Color) {
        self.recent.retain(|c| *c != color);
        self.recent.insert(0, color);
        self.recent.truncate(16);
    }
}

/// Settings for the current tool, shown in the control bar.
#[derive(Clone, Debug)]
pub struct ToolOptions {
    pub polygon_sides: usize,
    /// What the polygon tool draws: polygon, star, gear, heart…
    pub figure: xenify_ops::draw::Figure,
    /// Inner radius of stars, as a fraction of the outer one.
    pub star_inner: f64,
    pub gear_teeth: usize,
    /// Tooth depth, as a fraction of the gear's radius.
    pub gear_depth: f64,
    /// The gear's hole, as a fraction of its radius (0 = none).
    pub gear_hole: f64,
    /// Corner radius of new rectangles (0 = sharp).
    pub corner_radius: f64,
    /// Turn freehand strokes into clean shapes when they look like one.
    pub recognize: bool,
    pub pencil_width: f64,
    /// How much freehand strokes are smoothed (1 = follow closely, 10 = very
    /// smooth, few points).
    pub smoothing: f64,
    pub brush_size: f64,
    pub sculpt_mode: xenify_ops::draw::SculptMode,
    /// Brush radius in screen points.
    pub sculpt_radius: f64,
    /// 0..1
    pub sculpt_strength: f64,
    /// What the Gradient tool draws.
    pub gradient_kind: xenify_doc::GradientKind,
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            polygon_sides: 3,
            figure: xenify_ops::draw::Figure::Polygon,
            star_inner: 0.5,
            gear_teeth: 12,
            gear_depth: 0.2,
            gear_hole: 0.3,
            corner_radius: 0.0,
            recognize: true,
            pencil_width: 4.0,
            smoothing: 3.0,
            brush_size: 16.0,
            sculpt_mode: xenify_ops::draw::SculptMode::Push,
            sculpt_radius: 60.0,
            sculpt_strength: 0.5,
            gradient_kind: xenify_doc::GradientKind::Linear,
        }
    }
}

impl ToolOptions {
    pub fn figure_params(&self) -> xenify_ops::draw::FigureParams {
        xenify_ops::draw::FigureParams {
            sides: self.polygon_sides,
            star_inner: self.star_inner,
            teeth: self.gear_teeth,
            tooth_depth: self.gear_depth,
            hole: self.gear_hole,
        }
    }

    /// What the polygon tool draws now ("Triangle", "Star", "Gear"…).
    pub fn figure_name(&self) -> &'static str {
        use xenify_ops::draw::Figure;
        match self.figure {
            Figure::Polygon if self.polygon_sides == 3 => "Triangle",
            f => f.name(),
        }
    }
}

/// A selected keyframe in the timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeySel {
    pub layer: Id,
    pub path: PropPath,
    pub time: f64,
}

impl KeySel {
    pub fn is(&self, layer: Id, path: PropPath, time: f64) -> bool {
        self.layer == layer && self.path == path && (self.time - time).abs() < 1e-6
    }
}

pub struct XenifyApp {
    pub editor: Editor,
    pub comp: Id,
    pub selected: Option<Id>,
    /// Further selected layers (Shift+click); `selected` is the primary one.
    pub extra_selected: Vec<Id>,
    pub(crate) keep_extras: bool,
    /// The last Duplicate: (original, copy) pairs, for Transform Again.
    pub(crate) last_duplicate: Vec<(Id, Id)>,
    pub selected_keys: Vec<KeySel>,
    /// Current time in seconds.
    pub time: f64,
    pub playing: bool,
    pub looping: bool,
    /// When on, any value change keyframes the property (Blender-style auto-key).
    pub auto_key: bool,
    /// View › Look Through Camera: the canvas shows what the active camera sees.
    pub look_through_camera: bool,
    /// The effect gallery's preview renderer.
    pub(crate) effect_previews: crate::effects_gallery::Previews,
    /// Sound playing along, and waveforms.
    pub(crate) sound: crate::sound_ui::Sound,
    pub(crate) emoji_panel: crate::emoji_panel::EmojiPanel,
    pub(crate) giphy_panel: crate::giphy_panel::GiphyPanel,
    pub(crate) templates: crate::templates_ui::Templates,
    /// Properties already explained as "isn't animated" this session.
    pub(crate) hinted: std::collections::HashSet<String>,
    /// Text or a label being typed on the canvas.
    pub in_place: Option<crate::viewport::InPlace>,
    /// The transition picker's preview renderer.
    pub(crate) transition_previews: crate::transition_ui::Previews,
    last_tick: Option<Instant>,
    pub tool: Tool,
    pub paint: Paint,
    /// Custom pens the user saved (kept between sessions).
    pub my_pens: Vec<xenify_doc::PenStyle>,
    /// The pen deleted last (and where it was), until restored.
    pub(crate) deleted_pen: Option<(usize, xenify_doc::PenStyle)>,
    /// Physics menu settings.
    pub physics: xenify_ops::physics::PhysicsSettings,
    pub orbit: xenify_ops::physics::OrbitSettings,
    pub tool_options: ToolOptions,
    /// The colour panel's eyedropper is armed: the next canvas click picks
    /// a colour into the active slot only.
    pub pick_color: bool,
    /// Where "Done" goes back to after editing a symbol.
    pub symbol_return: Option<Id>,
    /// The layer whose colours the colour panel last loaded.
    pub(crate) paint_synced: Option<Id>,
    pub viewport: Viewport,
    pub timeline: TimelineState,
    pub file: Option<PathBuf>,
    /// Folder last used in a file dialog.
    pub last_dir: Option<PathBuf>,
    pub status: String,
    /// The last thing that went wrong: shown in red until dismissed.
    pub error: Option<String>,
    pub dialog: Option<Dialog>,
    /// Panel arrangement (taken out while the dock is drawn).
    pub dock: Option<egui_dock::DockState<crate::dock::Panel>>,
    pub export: Option<ExportJob>,
    /// Exports waiting to run after the current one (e.g. one per slice).
    pub export_queue: Vec<crate::dialogs::QueuedExport>,
    /// 3D models being read in the background.
    pub pending_models: Vec<crate::import::PendingModel>,
    /// The last file exported (for "Show in folder").
    pub last_export: Option<PathBuf>,
    pub assistant: crate::assistant::Assistant,
    /// The motion preset being configured in the Properties panel.
    pub preset_ui: (usize, xenify_ops::presets::Preset, f64),
    window_title: String,
    /// Unsaved-changes prompt, autosave/recovery and recent files.
    pub session: crate::session::Session,
    /// The keyboard shortcut reference is showing.
    pub shortcuts_open: bool,
    pub graph: crate::graph::GraphState,
    /// Clipboard, file manager and printing (fakes in tests).
    pub platform: crate::platform::Platform,
    /// Keyboard shortcuts (defaults plus the user's changes).
    pub keymap: crate::keymap::Keymap,
    pub prefs: crate::settings::Prefs,
    /// The Settings window.
    pub settings: crate::settings::SettingsUi,
    /// Open files (tabs); the shown one's state is in the fields above.
    pub docs: crate::documents::Documents,
    /// A presentation in progress.
    pub presenter: Option<crate::present::Presenter>,
    pub slide_thumbs: crate::slides_panel::SlideThumbs,
    /// The screen chosen to present on (`None`: pick another monitor if any).
    pub present_screen: Option<crate::present::Screen>,
    /// Set when a new text layer is created so the properties panel focuses its text box.
    pub focus_text_edit: bool,
    /// Increments per user gesture; used as undo merge key so one drag = one undo step.
    gesture: u64,
    /// The widget with keyboard focus last frame (a new one starts a new undo step).
    last_focus: Option<egui::Id>,
}

impl XenifyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::with_platform(cc, crate::platform::Platform::native());
        // The showcase waits behind the New dialog (tests keep the plain demo).
        crate::showcase::open(&mut app);
        app.status.clear();
        app
    }

    pub fn with_platform(cc: &eframe::CreationContext<'_>, platform: crate::platform::Platform) -> Self {
        // Video layers get their frames from ffmpeg (once per process).
        xenify_platform::media::install();
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        crate::theme::install_fonts(&cc.egui_ctx);
        let project = xenify_doc::demo_project();
        let comp = project.compositions[0].id;
        let mut app = Self {
            editor: Editor::new(project),
            comp,
            selected: None,
            extra_selected: Vec::new(),
            last_duplicate: Vec::new(),
            keep_extras: false,
            selected_keys: Vec::new(),
            time: 0.0,
            playing: false,
            looping: true,
            auto_key: false,
            look_through_camera: false,
            effect_previews: Default::default(),
            sound: Default::default(),
            emoji_panel: Default::default(),
            giphy_panel: Default::default(),
            templates: Default::default(),
            hinted: Default::default(),
            in_place: None,
            transition_previews: Default::default(),
            last_tick: None,
            tool: Tool::Select,
            paint: Paint::default(),
            physics: Default::default(),
            orbit: Default::default(),
            deleted_pen: None,
            my_pens: cc.storage.and_then(|s| eframe::get_value(s, crate::control_bar::MY_PENS_KEY)).unwrap_or_default(),
            tool_options: ToolOptions::default(),
            pick_color: false,
            symbol_return: None,
            paint_synced: None,
            viewport: Viewport::default(),
            timeline: TimelineState::default(),
            file: None,
            last_dir: cc.storage.and_then(|s| eframe::get_value(s, "last_dir")),
            status: "Welcome to Xenify Vector — this is a demo project. Press Space to play.".into(),
            error: None,
            dialog: None,
            dock: Some(crate::dock::saved_layout(cc.storage)),
            export: None,
            export_queue: Vec::new(),
            last_export: None,
            pending_models: Vec::new(),
            assistant: Default::default(),
            preset_ui: (0, xenify_ops::presets::Preset::DEFAULTS[0].1, 0.1),
            gesture: 0,
            last_focus: None,
            focus_text_edit: false,
            window_title: String::new(),
            session: crate::session::Session::new(cc.storage, cc.egui_ctx.clone()),
            shortcuts_open: false,
            graph: Default::default(),
            platform,
            keymap: crate::keymap::Keymap::load(cc.storage),
            prefs: crate::settings::Prefs::load(cc.storage),
            settings: Default::default(),
            docs: Default::default(),
            presenter: None,
            slide_thumbs: Default::default(),
            present_screen: None,
        };
        app.editor.mark_saved();
        app.auto_key = app.prefs.auto_key;
        // Reopen last session's files (they replace the demo).
        if app.prefs.reopen_tabs {
            let paths: Vec<PathBuf> = cc.storage.and_then(|s| eframe::get_value(s, OPEN_TABS_KEY)).unwrap_or_default();
            for path in paths.into_iter().filter(|p| p.exists()) {
                app.open(path);
            }
        }
        // Nothing reopened: ask what to make (the demo waits behind it).
        if app.prefs.show_start && app.docs.tabs.len() <= 1 && app.file.is_none() {
            let d = crate::new_dialog::NewDialog::at_launch();
            app.dialog = Some(crate::dialogs::Dialog::NewDocument(Box::new(d)));
        }
        app
    }

    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    pub fn comp(&self) -> &Composition {
        self.editor.project().comp(self.comp).expect("current composition exists")
    }

    pub fn fps(&self) -> f64 {
        self.comp().settings.fps
    }

    /// Snaps a time to the nearest frame, clamped to the composition.
    pub fn snap(&self, t: f64) -> f64 {
        let s = &self.comp().settings;
        let last = (s.frame_count().max(1) - 1) as f64 / s.fps;
        ((t * s.fps).round() / s.fps).clamp(0.0, last)
    }

    pub fn set_time(&mut self, t: f64) {
        self.time = self.snap(t);
    }

    /// Starts a new undo-merge gesture and returns its key.
    pub fn new_gesture(&mut self) -> String {
        self.gesture += 1;
        self.gesture_key()
    }

    pub fn gesture_key(&self) -> String {
        format!("g{}", self.gesture)
    }

    /// A manual edit while AI changes await review accepts them first, so the
    /// user's edit doesn't get folded into the AI's undo step.
    pub(crate) fn settle_ai(&mut self) {
        if self.assistant.reviewing {
            crate::assistant::Assistant::accept(self);
        }
    }

    /// Runs a command as its own undo step, reporting failures in the status bar.
    pub fn exec(&mut self, label: &str, cmd: Command) -> bool {
        self.settle_ai();
        match self.editor.execute(label, cmd) {
            Ok(()) => true,
            Err(e) => {
                self.fail(format!("{label}: {e}"));
                false
            }
        }
    }

    /// Runs a command merged into the current gesture's undo step.
    pub fn exec_merged(&mut self, label: &str, cmd: Command) -> bool {
        self.settle_ai();
        let key = self.gesture_key();
        match self.editor.execute_merged(label, key, cmd) {
            Ok(()) => true,
            Err(e) => {
                self.fail(format!("{label}: {e}"));
                false
            }
        }
    }

    /// Applies an interactive value change (drag, property field) at the
    /// current time, honouring auto-key, merged into the current gesture.
    pub fn edit_value(&mut self, layer: Id, path: PropPath, value: xenify_doc::Value) {
        self.settle_ai();
        let label = format!("Change {}", crate::widgets::prop_label(path));
        let cmd = match Command::edit_value(self.project(), layer, path, self.time, value, self.auto_key) {
            Ok(c) => c,
            Err(e) => {
                self.fail(format!("{label}: {e}"));
                return;
            }
        };
        let was_animated =
            self.project().layer(layer).ok().and_then(|l| l.property(path).ok()).is_some_and(|p| p.is_animated());
        if self.exec_merged(&label, cmd) && !was_animated && !self.auto_key {
            self.not_animated_hint(layer, path);
        }
    }

    /// Explains (once per property per session) that a change to a
    /// property without keyframes applies to the whole clip.
    pub(crate) fn not_animated_hint(&mut self, layer: Id, path: xenify_doc::PropPath) {
        if self.hinted.insert(format!("{}:{path:?}", layer.0)) {
            self.status = format!(
                "{} isn't animated, so this changed it for the whole clip. Click its ⏱, or turn on Auto Keying in the timeline, to keyframe changes.",
                crate::widgets::prop_label(path)
            );
        }
    }

    /// Reports a failure: in the status bar in red, staying until dismissed.
    pub fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.error = Some(message);
    }

    /// The other selected layers a Properties edit of `path` on `primary`
    /// also applies to: transform, shape and text values, on layers that have
    /// the same property.
    pub fn shared_edit_targets(&self, primary: Id, path: PropPath) -> Vec<Id> {
        let shared = matches!(
            path,
            PropPath::Anchor
                | PropPath::Position
                | PropPath::Scale
                | PropPath::Rotation
                | PropPath::Opacity
                | PropPath::Skew
                | PropPath::SkewAxis
                | PropPath::Shape { .. }
                | PropPath::FontSize
                | PropPath::TextColor
        );
        if !shared {
            return Vec::new();
        }
        let kind = self.project().layer(primary).ok().and_then(|l| l.property(path).ok()).map(|p| p.kind());
        self.selected_layers()
            .into_iter()
            .filter(|id| *id != primary)
            .filter(|id| self.project().layer(*id).ok().and_then(|l| l.property(path).ok()).map(|p| p.kind()) == kind)
            .collect()
    }

    /// Whether `path` differs between `primary` and the other layers an edit
    /// would reach.
    pub fn shared_value_mixed(&self, primary: Id, path: PropPath) -> bool {
        let value = |id: Id| self.project().layer(id).ok().and_then(|l| l.value_at(path, self.time).ok());
        let first = value(primary);
        self.shared_edit_targets(primary, path).into_iter().any(|id| value(id) != first)
    }

    /// A Properties edit: `primary` gets `value`; the other selected layers
    /// with the same property get it too, or, where their numbers differed,
    /// the same change (so moving several spread-out layers keeps them apart).
    pub fn edit_value_all(&mut self, primary: Id, path: PropPath, value: xenify_doc::Value) {
        use xenify_doc::Value;
        let others = self.shared_edit_targets(primary, path);
        let mixed = self.shared_value_mixed(primary, path);
        let old = self.project().layer(primary).ok().and_then(|l| l.value_at(path, self.time).ok());
        self.edit_value(primary, path, value.clone());
        for id in others {
            let theirs = self.project().layer(id).ok().and_then(|l| l.value_at(path, self.time).ok());
            let target = match (mixed, &old, &value, theirs) {
                (true, Some(Value::Scalar(a)), Value::Scalar(b), Some(Value::Scalar(c))) => Value::Scalar(c + (b - a)),
                (true, Some(Value::Vec2(a)), Value::Vec2(b), Some(Value::Vec2(c))) => Value::Vec2(c + (*b - *a)),
                _ => value.clone(),
            };
            self.edit_value(id, path, target);
        }
    }

    pub fn undo(&mut self) {
        // Undo closes any open AI group; there is nothing left to review.
        self.assistant.reviewing = false;
        match self.editor.undo() {
            Some(l) => self.status = format!("Undid: {l}"),
            None => self.status = "Nothing to undo".into(),
        }
        self.fix_selection();
    }

    pub fn redo(&mut self) {
        match self.editor.redo() {
            Some(l) => self.status = format!("Redid: {l}"),
            None => self.status = "Nothing to redo".into(),
        }
        self.fix_selection();
    }

    pub fn is_selected(&self, id: Id) -> bool {
        self.selected == Some(id) || self.extra_selected.contains(&id)
    }

    /// All selected layers, primary first.
    pub fn selected_layers(&self) -> Vec<Id> {
        self.selected.into_iter().chain(self.extra_selected.iter().copied()).collect()
    }

    /// Keeps the extra selection this frame (it's normally dropped when the
    /// primary selection changes without Shift).
    pub fn keep_selection(&mut self) {
        self.keep_extras = true;
    }

    /// Selects only `id` (or nothing).
    pub fn select_only(&mut self, id: Option<Id>) {
        self.selected = id;
        self.extra_selected.clear();
        self.selected_keys.clear();
    }

    /// Shift+click: adds `id` to the selection (making it primary) or removes it.
    /// The selected layers in the order they were clicked (Shift+click adds
    /// the newest as primary, pushing earlier ones into `extra_selected`).
    pub fn selection_in_click_order(&self) -> Vec<Id> {
        self.extra_selected.iter().copied().chain(self.selected).collect()
    }

    pub fn toggle_selection(&mut self, id: Id) {
        if self.selected == Some(id) {
            self.selected = self.extra_selected.pop();
        } else if let Some(i) = self.extra_selected.iter().position(|e| *e == id) {
            self.extra_selected.remove(i);
        } else {
            if let Some(p) = self.selected {
                self.extra_selected.push(p);
            }
            self.selected = Some(id);
        }
        self.keep_extras = true;
    }

    /// Drops selections that no longer exist (after undo, delete, open, …).
    pub fn fix_selection(&mut self) {
        // Undo may remove the composition being shown (e.g. undoing a precompose).
        if self.project().comp(self.comp).is_err() {
            self.comp = self.project().compositions[0].id;
        }
        let in_comp = |app: &Self, id: Id| app.comp().layer(id).is_some();
        if self.selected.is_some_and(|id| !in_comp(self, id)) {
            self.selected = None;
        }
        let keep: Vec<Id> = self.extra_selected.iter().copied().filter(|id| in_comp(self, *id)).collect();
        self.extra_selected = keep;
        if let Some(id) = self.selected
            && self.project().layer(id).is_err()
        {
            self.selected = None;
        }
        let (project, primary) = (self.editor.project(), self.selected);
        self.extra_selected.retain(|id| project.layer(*id).is_ok() && Some(*id) != primary);
        if self.selected.is_none() {
            self.selected = self.extra_selected.pop();
        }
        let project = self.editor.project();
        self.selected_keys.retain(|k| {
            project
                .layer(k.layer)
                .ok()
                .and_then(|l| l.keyframe_times(k.path).ok())
                .is_some_and(|ts| ts.iter().any(|t| (t - k.time).abs() < 1e-6))
        });
        if self.project().comp(self.comp).is_err() {
            self.comp = self.project().compositions[0].id;
        }
    }

    /// Traces an image into a group of vector layers (one undo step).
    pub fn trace_image(&mut self, data: &[u8], opts: &xenify_export::trace::TraceOptions, name: &str) {
        let s = &self.comp().settings;
        match xenify_export::trace::trace_to_layers(data, opts, self.project().next_id, s.width, s.height) {
            Ok(layers) => self.add_layer_group(layers, name),
            Err(e) => self.fail(e),
        }
    }

    /// Traces an image layer into vector shapes grouped just above it,
    /// exactly over the picture (one undo step).
    pub fn trace_image_layer(&mut self, id: Id, opts: &xenify_export::trace::TraceOptions) {
        let Ok(layer) = self.project().layer(id).cloned() else { return };
        let xenify_doc::LayerKind::Image { asset, .. } = layer.kind else {
            self.status = "Select a picture to trace.".into();
            return;
        };
        let Some(data) = self.project().asset(asset).map(|a| a.data.clone()) else { return };
        let place = xenify_anim::world_transform(self.comp(), id, self.time);
        match xenify_export::trace::trace_to_layers_at(&data, opts, self.project().next_id, place) {
            Ok(layers) => self.add_layer_group(layers, &format!("{} (traced)", layer.meta.name)),
            Err(e) => self.fail(e),
        }
    }

    /// Adds an SVG document (shapes, images, named groups) as one group (one undo step).
    pub fn import_svg(&mut self, data: &[u8], name: &str) {
        self.import_svg_document(data, name);
    }

    /// Adds `layers` and groups them as `name` (one undo step), selecting the group.
    pub(crate) fn add_layer_group(&mut self, layers: Vec<xenify_doc::Layer>, name: &str) {
        let ids: Vec<Id> = layers.iter().map(|l| l.id).collect();
        let n = ids.len();
        self.editor.begin_group(format!("Add {name}"));
        let adds = layers.into_iter().map(|layer| Command::AddLayer { comp: self.comp, index: None, layer }).collect();
        if self.exec("Add shapes", Command::batch(adds)) {
            let (project, comp, time) = (self.project().clone(), self.comp, self.time);
            let grouped = {
                let mut m = self.viewport.measurer();
                xenify_ops::group::group(&project, comp, &ids, name, time, &mut m)
            };
            if let Ok((cmd, gid)) = grouped
                && self.exec("Group", cmd)
            {
                self.select_only(Some(gid));
            }
            self.status = format!("Added {n} vector shapes ({name}).");
        }
        self.editor.end_group();
    }

    /// Ctrl+G: groups the selected layers.
    pub fn group_selection(&mut self) {
        let layers = self.selected_layers();
        let n = self.comp().layers.iter().filter(|l| matches!(l.kind, xenify_doc::LayerKind::Group)).count() + 1;
        let (project, comp, time) = (self.project().clone(), self.comp, self.time);
        let result = {
            let mut m = self.viewport.measurer();
            xenify_ops::group::group(&project, comp, &layers, &format!("Group {n}"), time, &mut m)
        };
        match result {
            Ok((cmd, id)) => {
                if self.exec("Group", cmd) {
                    self.select_only(Some(id));
                    self.status = "Grouped. Click selects the group; Ctrl+click selects inside it.".into();
                }
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// Shows composition `id` in the canvas and timeline.
    pub fn open_comp(&mut self, id: Id) {
        if self.project().comp(id).is_err() || id == self.comp {
            return;
        }
        self.comp = id;
        self.select_only(None);
        self.time = self.time.min(self.comp().settings.duration);
        let name = self.comp().settings.name.clone();
        self.status = match self.project().slide_index(id) {
            Some(i) => format!("Slide {} of {}: {name}", i + 1, self.project().slides.len()),
            None => format!("Editing the symbol “{name}”: press Done (or Esc) to go back."),
        };
    }

    /// A new, empty symbol with the current canvas's settings, opened.
    pub fn new_comp(&mut self) {
        let id = Id(self.project().next_id);
        let name = crate::symbols::next_name(self.project());
        let mut settings = xenify_doc::CompSettings { name, ..self.comp().settings.clone() };
        settings.slices.clear();
        settings.pauses.clear();
        let comp = xenify_doc::Composition { id, settings, layers: Vec::new() };
        if self.exec("New symbol", Command::AddComposition { index: None, comp }) {
            crate::symbols::edit(self, id);
        }
    }

    /// Places composition `id` in the current one as a layer.
    pub fn add_comp_layer(&mut self, id: Id) {
        let Ok(child) = self.project().comp(id).cloned() else { return };
        let s = &self.comp().settings;
        let centre = xenify_doc::Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0);
        let layer_id = Id(self.project().next_id);
        let layer = xenify_doc::Layer::precomp(layer_id, child.settings.name.clone(), &child, centre);
        if self.exec("Place symbol", Command::AddLayer { comp: self.comp, index: None, layer }) {
            self.select_only(Some(layer_id));
        }
    }

    /// Deletes the current composition (if nothing uses it) and opens another.
    pub fn delete_comp(&mut self) {
        let id = self.comp;
        let Some(other) = self.project().compositions.iter().find(|c| c.id != id).map(|c| c.id) else { return };
        let cmd = Command::RemoveComposition { comp: id };
        // Validate first so a failure leaves us where we were.
        if let Err(e) = xenify_ops::apply(&mut self.project().clone(), cmd.clone()) {
            self.fail(format!("Can't delete: {e}"));
            return;
        }
        self.comp = other;
        self.select_only(None);
        self.exec("Delete symbol", cmd);
    }

    /// Ctrl+Shift+C: turns the selected layers into a symbol (one copy left in place).
    pub fn precompose_selection(&mut self) {
        let layers = self.selected_layers();
        let name = crate::symbols::next_name(self.project());
        match xenify_ops::precomp::precompose(self.project(), self.comp, &layers, &name) {
            Ok((cmd, _, layer)) => {
                if self.exec("Convert to symbol", cmd) {
                    self.select_only(Some(layer));
                    self.status = format!(
                        "Made the symbol “{name}”. Double-click it to edit; place more copies from the Symbols panel."
                    );
                }
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// Ctrl+7 / Ctrl+Alt+7: sets (or releases) a mask layer from the selection.
    pub fn clip_selection(&mut self, make: bool) {
        let layers = self.selected_layers();
        let result = if make {
            xenify_ops::matte::make_clipping_mask(self.project(), self.comp, &layers)
        } else {
            xenify_ops::matte::release_clipping_mask(self.project(), self.comp, &layers)
        };
        match result {
            Ok(cmd) => {
                let label = if make { "Set mask" } else { "Release mask" };
                self.exec(label, cmd);
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// Ctrl+Shift+G: dissolves the selected groups.
    pub fn ungroup_selection(&mut self) {
        let groups: Vec<Id> = self
            .selected_layers()
            .into_iter()
            .filter(|id| self.project().layer(*id).is_ok_and(|l| matches!(l.kind, xenify_doc::LayerKind::Group)))
            .collect();
        if groups.is_empty() {
            self.status = "Select a group to ungroup.".into();
            return;
        }
        let children: Vec<Id> = groups
            .iter()
            .flat_map(|g| self.comp().layers.iter().filter(move |l| l.meta.parent == Some(*g)).map(|l| l.id))
            .collect();
        let cmds: Result<Vec<Command>, _> =
            groups.iter().map(|g| xenify_ops::group::ungroup(self.project(), *g)).collect();
        match cmds {
            Ok(cmds) => {
                if self.exec("Ungroup", Command::batch(cmds)) {
                    self.select_only(children.first().copied());
                    self.extra_selected = children.into_iter().skip(1).collect();
                    self.keep_extras = true;
                }
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    pub fn delete_selection(&mut self) {
        if self.tool == Tool::Slice {
            let mut vp = std::mem::take(&mut self.viewport);
            let deleted = vp.delete_selected_slice(self);
            self.viewport = vp;
            if deleted {
                return;
            }
        }
        // With a path in progress, the pen's last point.
        if self.tool == Tool::Pen && self.viewport.pen_remove_last() {
            return;
        }
        if self.tool == Tool::Node {
            let mut vp = std::mem::take(&mut self.viewport);
            let deleted = vp.delete_selected_node(self);
            self.viewport = vp;
            if deleted {
                return;
            }
        }
        if !self.selected_keys.is_empty() {
            let keys = std::mem::take(&mut self.selected_keys);
            let label = if keys.len() == 1 { "Delete keyframe" } else { "Delete keyframes" };
            let cmds = keys.iter().map(|k| Command::RemoveKeyframe { layer: k.layer, path: k.path, time: k.time });
            self.exec(label, Command::batch(cmds.collect()));
        } else if !self.selected_layers().is_empty() {
            // Every selected layer (a deleted group's other members stay).
            let cmds: Vec<Command> =
                self.selected_layers().into_iter().map(|id| Command::RemoveLayer { layer: id }).collect();
            let label = if cmds.len() == 1 { "Delete layer" } else { "Delete layers" };
            self.select_only(None);
            self.exec(label, Command::batch(cmds));
        }
    }

    pub fn new_project(&mut self) {
        self.open_document(Project::new(), None);
        self.status = "New project".into();
    }

    pub fn load(&mut self, mut project: Project, file: Option<PathBuf>) {
        // Older projects' null layers become groups.
        xenify_ops::order::nulls_to_groups(&mut project);
        self.comp = project.compositions[0].id;
        self.editor.replace_project(project);
        self.file = file;
        self.selected = None;
        self.selected_keys.clear();
        self.time = 0.0;
        self.playing = false;
        self.viewport.request_fit();
    }

    pub fn open(&mut self, path: PathBuf) {
        match NativeFs
            .read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| Project::from_json(&s).map_err(|e| e.to_string()))
        {
            Ok(p) => {
                self.status = format!("Opened {}", path.display());
                self.session.add_recent(&path, self.prefs.recent_max);
                self.open_document(p, Some(path));
            }
            Err(e) => self.fail(format!("Could not open {}: {e}", path.display())),
        }
    }

    /// Opens the files the app was started with (`xenify a.xenify logo.svg`).
    pub fn open_from_command_line(&mut self, files: Vec<PathBuf>) {
        let (projects, other): (Vec<PathBuf>, Vec<PathBuf>) =
            files.into_iter().partition(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xenify")));
        for p in projects {
            let p = std::path::absolute(&p).unwrap_or(p);
            if p.exists() {
                self.open(p);
            } else {
                self.status = format!("{} doesn't exist", p.display());
            }
        }
        if !other.is_empty() {
            crate::new_dialog::create(self, xenify_doc::CompSettings::default(), crate::new_dialog::Source::Blank);
            self.import_files(other, None);
            self.editor.mark_saved();
        }
    }

    pub fn save_to(&mut self, mut path: PathBuf) {
        if path.extension().is_none() {
            path.set_extension("xenify");
        }
        match NativeFs.write(&path, self.project().to_json().as_bytes()) {
            Ok(()) => {
                self.editor.mark_saved();
                self.status = format!("Saved {}", path.display());
                self.session.add_recent(&path, self.prefs.recent_max);
                self.file = Some(path);
                crate::session::after_save(self);
            }
            Err(e) => self.fail(format!("Could not save {}: {e}", path.display())),
        }
    }

    pub fn save(&mut self) {
        match self.file.clone() {
            Some(p) => self.save_to(p),
            None => self.dialog = Some(Dialog::save_as(self)),
        }
    }

    fn tick_playback(&mut self, ctx: &egui::Context) {
        // Presenting plays its own sound (see present.rs).
        if self.presenter.is_none() {
            let source = crate::sound_ui::Source::Comp(self.comp, self.comp().settings.duration);
            let enabled = self.prefs.play_sound;
            if let Some(msg) = self.sound.sync(self.editor.project(), source, self.time, self.playing, enabled) {
                self.status = msg;
            }
        }
        if !self.playing {
            self.last_tick = None;
            return;
        }
        let now = Instant::now();
        let dt = self.last_tick.map_or(0.0, |t| (now - t).as_secs_f64());
        self.last_tick = Some(now);
        let duration = self.comp().settings.duration;
        let mut t = self.time + dt;
        if t >= duration {
            if self.looping {
                t %= duration;
            } else {
                t = duration;
                self.playing = false;
            }
        }
        // Keep sub-frame time while playing for smooth motion; snap when stopped.
        self.time = t.min(duration);
        if !self.playing {
            self.set_time(self.time);
        }
        ctx.request_repaint();
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        if !self.playing {
            self.set_time(self.time);
        } else if self.time >= self.comp().settings.duration - 1.0 / self.fps() {
            self.time = 0.0;
        }
    }

    /// Jumps to the previous/next keyframe of the selected layer.
    pub fn jump_keyframe(&mut self, forward: bool) {
        let Some(layer) = self.selected.and_then(|id| self.project().layer(id).ok()) else { return };
        let mut times: Vec<f64> =
            layer.property_paths().into_iter().flat_map(|p| layer.keyframe_times(p).unwrap_or_default()).collect();
        times.sort_by(f64::total_cmp);
        let eps = 0.5 / self.fps();
        let target = if forward {
            times.into_iter().find(|t| *t > self.time + eps)
        } else {
            times.into_iter().rev().find(|t| *t < self.time - eps)
        };
        if let Some(t) = target {
            self.set_time(t);
        }
    }

    /// The Select tool (for tests and the Esc path).
    pub fn set_tool_select(&mut self) {
        self.set_tool(Tool::Select);
    }

    /// Switches tool, cancelling whatever the old one was doing.
    pub fn set_tool(&mut self, tool: Tool) {
        // Free Transform is part of Select now (Q still picks it).
        let tool = if tool == Tool::FreeTransform { Tool::Select } else { tool };
        if self.tool != tool {
            // A path being drawn with the pen is kept, not thrown away.
            if self.tool == Tool::Pen {
                let mut vp = std::mem::take(&mut self.viewport);
                vp.end_pen(self);
                self.viewport = vp;
            }
            self.tool = tool;
            self.viewport.cancel_tool();
            // Drawing slices needs to see them (View › Show Slices hides them again).
            if tool == Tool::Slice {
                self.prefs.show_slices = true;
            }
        }
        self.pick_color = false;
    }

    fn title(&self) -> String {
        let name = self.file.as_ref().and_then(|p| p.file_stem()).map_or("Untitled".into(), |s| s.to_string_lossy());
        format!("{name}{} — Xenify Vector", if self.editor.is_dirty() { " •" } else { "" })
    }
}

/// eframe storage key: files open when the app last closed.
const OPEN_TABS_KEY: &str = "open_tabs";

impl eframe::App for XenifyApp {
    /// Remembers the panel layout (eframe also keeps the window size/position).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(dock) = &self.dock {
            eframe::set_value(storage, crate::dock::LAYOUT_KEY, dock);
            eframe::set_value(storage, crate::dock::CLOSABLE_KEY, &true);
        }
        if let Some(dir) = &self.last_dir {
            eframe::set_value(storage, "last_dir", dir);
        }
        eframe::set_value(storage, crate::session::RECENT_KEY, &self.session.recent);
        eframe::set_value(storage, crate::control_bar::MY_PENS_KEY, &self.my_pens);
        self.keymap.save(storage);
        self.prefs.auto_key = self.auto_key;
        eframe::set_value(storage, crate::settings::PREFS_KEY, &self.prefs);
        let open: Vec<PathBuf> = self.tab_info().into_iter().filter_map(|t| t.path).collect();
        eframe::set_value(storage, OPEN_TABS_KEY, &open);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::settings::apply_display(&ctx, &self.prefs);
        // Each press, and each field taking the keyboard, starts its own undo
        // step: typing notes right after a drag can't fold into the drag's.
        let focus = ctx.memory(|m| m.focused());
        if ctx.input(|i| i.pointer.any_pressed()) || focus != self.last_focus {
            self.new_gesture();
        }
        self.last_focus = focus;
        // Presenting in this window: the slides take it over.
        if crate::present::show(self, ui, frame) {
            return;
        }
        crate::actions::handle_keys(self, &ctx);
        self.tick_playback(&ctx);
        crate::dialogs::poll_export(self, &ctx);
        self.poll_models(&ctx);
        crate::assistant::Assistant::poll(self, &ctx);
        crate::session::update(self, &ctx);
        crate::clipboard::handle(self, &ctx);
        crate::panels::shortcuts_window(self, &ctx);
        crate::settings::window(self, &ctx);
        let mut synced = self.paint_synced;
        crate::colors::sync_from_selection(self, &mut synced);
        self.paint_synced = synced;
        // Only when it changes: setting the title makes the window redraw,
        // so sending it every frame would repaint forever.
        let title = self.title();
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        egui::Panel::top("menu").show(ui, |ui| crate::panels::menu_bar(self, ui));
        egui::Panel::top("doc_tabs").show(ui, |ui| crate::doc_tabs::strip(self, ui));
        egui::Panel::top("control").show(ui, |ui| crate::control_bar::bar(self, ui));
        egui::Panel::bottom("status").show(ui, |ui| crate::status_bar::bar(self, ui));
        egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| crate::panels::toolbar(self, ui));
        let before = (self.editor.revision(), self.time.to_bits());
        let selected_before = self.selected;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::dock::show(self, ui, frame));
        // A panel drawn after the canvas may have changed the time or the
        // project this frame; draw once more so the canvas catches up.
        if (self.editor.revision(), self.time.to_bits()) != before {
            ctx.request_repaint();
        }
        // Selecting something new (without Shift) drops the extra selection.
        if self.selected != selected_before && !self.keep_extras {
            self.extra_selected.clear();
        }
        self.keep_extras = false;

        crate::dialogs::show(self, &ctx);
        crate::import::handle_drops(self, &ctx);
    }
}
