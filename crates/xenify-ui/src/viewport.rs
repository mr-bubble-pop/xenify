//! The canvas: renders the composition with Vello into a texture that egui
//! displays, and handles the drawing / transform tools on top of it.

use crate::app::{XenifyApp, Tool};
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2 as EVec2};
use xenify_anim::{Scene, hit_test, world_transform};
use xenify_doc::{Color, Id, Layer, PropPath, ShapeItem, Value, Vec2};
use xenify_ops::Command;
use xenify_render::TextEngine;
use xenify_render::vello::{self, wgpu};
use kurbo::{Affine, Point};
use std::cell::RefCell;

mod array_handle;
mod connector_tool;
mod crop;
mod emitter_handle;
mod freehand;
mod gradient_tool;
mod in_place;
pub use in_place::{InPlace, start as start_in_place};
mod knife;
mod mask_tool;
mod node_edit;
mod path_tools;
mod rulers;
mod text_box;
mod xform;
pub use crop::{crop_options, start_crop};
pub use mask_tool::{mask_colour, mask_options};
use node_edit::NodeDrag;
pub use path_tools::convert_to_path_command;
pub use xform::{clear_distort, recentre, set_origin};

const PASTEBOARD: Color = Color::rgb(0.13, 0.13, 0.15);
const HANDLE: f32 = 7.0;
const ROTATE_OFFSET: f32 = 28.0;

struct Gpu {
    renderer: vello::Renderer,
    texture: wgpu::Texture,
    texture_id: egui::TextureId,
    size: (u32, u32),
    vscene: vello::Scene,
    /// What was last rendered; skip re-rendering when nothing changed.
    last: Option<(u64, u64, [u64; 6])>,
}

/// Composition-space lines a moving box can snap to.
#[derive(Clone, Default)]
struct SnapTargets {
    xs: Vec<f64>,
    ys: Vec<f64>,
}

/// A guide line to draw while snapping (composition space).
#[derive(Clone, Copy)]
enum Guide {
    V(f64),
    H(f64),
}

/// Snap distance in screen points.
const SNAP_PX: f64 = 6.0;

#[derive(Clone)]
enum Drag {
    Pan,
    /// Box (rubber-band) selection from empty canvas; Shift adds.
    Marquee {
        start: Point,
        end: Point,
        additive: bool,
    },
    /// Moving one or more layers: (id, start position, comp→parent transform),
    /// plus the selection's box at the start and what it can snap to.
    Move {
        layers: Vec<(Id, Vec2, Affine)>,
        start_mouse: Point,
        start_box: Option<kurbo::Rect>,
        snap: SnapTargets,
        /// Connectors being dragged, as they were: their loose ends and
        /// bends move (glued ends stay glued).
        connectors: Vec<(Id, xenify_doc::Connector)>,
    },
    Scale {
        layer: Id,
        start_scale: Vec2,
        anchor: Pos2,
        start_dist: f32,
    },
    Rotate {
        layer: Id,
        start_rot: f64,
        anchor: Pos2,
        start_angle: f32,
    },
    Create {
        start: Point,
        snap: SnapTargets,
    },
    /// Posing an IK chain: the dragged layer's anchor follows the pointer
    /// (plus `offset`), bending its parent joints.
    Ik {
        layer: Id,
        offset: Vec2,
    },
    /// Dragging a gradient's start (`end == false`) or end handle.
    Gradient {
        layer: Id,
        index: usize,
        /// The stroke's gradient (else the fill's).
        stroke: bool,
        end: bool,
        /// Composition → layer space.
        inv: Affine,
    },
    /// The Gradient tool: dragging out a new gradient line (composition space).
    GradientLine {
        from: Point,
    },
    /// The Connector tool: from where the drag started (composition space).
    Connect {
        from: xenify_doc::ConnectorEnd,
        start: Point,
    },
    /// Moving the selected connector's end (`to`: its end, else its start).
    ConnectorEnd {
        layer: Id,
        to: bool,
    },
    /// Moving a middle segment of an elbow connector: its route when the
    /// drag started, and where the drag started (composition space).
    ConnectorSegment {
        layer: Id,
        index: usize,
        points: Vec<Point>,
        start: Point,
    },
    /// Dragging a liquid's surface (its level); composition → layer space.
    LiquidLevel {
        layer: Id,
        inv: Affine,
    },
    /// Dragging a particle emitter's direction arrow (composition-space
    /// origin, and the layer's turn in degrees).
    EmitterArrow {
        layer: Id,
        origin: Point,
        turn: f64,
    },
    /// Dragging an array's origin (the point its copies turn around).
    ArrayOrigin {
        layer: Id,
        index: usize,
        /// Composition → layer space.
        inv: Affine,
    },
}

/// A document's zoom and scroll position.
#[derive(Clone, Copy, Debug)]
pub struct ViewState {
    zoom: f64,
    pan: EVec2,
    fit_pending: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self { zoom: 1.0, pan: EVec2::ZERO, fit_pending: true }
    }
}

pub struct Viewport {
    /// What a click would select right now (outlined under the pointer).
    pub hover_pick: std::cell::Cell<Option<Id>>,
    /// Screen points per composition pixel.
    zoom: f64,
    /// Offset of the composition origin from the viewport's top-left, in points.
    pan: EVec2,
    fit_pending: bool,
    gpu: Option<Gpu>,
    drag: Option<Drag>,
    create_end: Option<Point>,
    create_start: Option<Point>,
    pen: Vec<xenify_doc::bezier::PenPoint>,
    /// While dragging out a pen point: its anchor, and whether it closes the path.
    pen_drag: Option<(Point, bool)>,
    /// Continuing an existing open path with the pen (instead of a new layer).
    pen_continue: Option<path_tools::PenContinue>,
    node_drag: Option<NodeDrag>,
    /// Lines a dragged path point can snap to.
    node_snap: Option<SnapTargets>,
    /// Selected path point in Edit Points mode: (layer, shape index, node index).
    /// Edit Points: the selected points (shape, index) of `nodes_layer`.
    selected_nodes: Vec<(usize, usize)>,
    nodes_layer: Option<Id>,
    /// Guides shown while snapping.
    guides: Vec<Guide>,
    /// Free transform: a custom centre, the drag in progress, and a centre being dragged.
    pivot: xform::Pivot,
    xdrag: Option<xform::XDrag>,
    pending_pivot: Option<Point>,
    /// Pencil/brush points being drawn (composition space).
    stroke: Vec<Point>,
    /// Sculpt brush position last frame.
    sculpt_last: Option<Point>,
    crop: crop::CropState,
    /// Knife: the cut being dragged (composition space).
    knife: Option<(Point, Point)>,
    mask: mask_tool::MaskState,
    slice: crop::SliceState,
    slice_preview: Option<kurbo::Rect>,
    /// Zoom tool marquee start (screen).
    zoom_box: Option<Pos2>,
    /// The canvas area last frame (for keyboard zoom).
    last_rect: Rect,
    /// Orbit tool: the solid being turned, its start rotation (x, y) and where the drag began.
    orbit: Option<(Id, (f64, f64), Point)>,
    error: Option<String>,
    /// Fonts and text layout cache, shared by drawing, hit testing and selection.
    text: RefCell<TextEngine>,
    families: Option<Vec<String>>,
    /// Space has been held to pan since it was pressed (so letting go doesn't play).
    space_panned: bool,
    /// A guide being dragged from a ruler or moved.
    guide_drag: Option<rulers::GuideDrag>,
    /// Text tool: a box being drawn or resized.
    text_drag: Option<text_box::TextDrag>,
    /// Where the canvas was last right-clicked (composition space).
    menu_at: Option<Point>,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            hover_pick: Default::default(),
            space_panned: false,
            guide_drag: None,
            zoom: 1.0,
            pan: EVec2::ZERO,
            fit_pending: true,
            gpu: None,
            drag: None,
            create_end: None,
            create_start: None,
            pen: Vec::new(),
            pen_drag: None,
            pen_continue: None,
            node_drag: None,
            node_snap: None,
            selected_nodes: Vec::new(),
            nodes_layer: None,
            guides: Vec::new(),
            pivot: Default::default(),
            xdrag: None,
            pending_pivot: None,
            stroke: Vec::new(),
            sculpt_last: None,
            crop: Default::default(),
            knife: None,
            mask: Default::default(),
            slice: Default::default(),
            slice_preview: None,
            text_drag: None,
            menu_at: None,
            zoom_box: None,
            last_rect: Rect::NOTHING,
            orbit: None,
            error: None,
            text: RefCell::new(TextEngine::new()),
            families: None,
        }
    }
}

impl Viewport {
    pub fn space_panned(&self) -> bool {
        self.space_panned
    }

    pub fn request_fit(&mut self) {
        self.fit_pending = true;
    }

    pub fn set_zoom_100(&mut self) {
        self.zoom_about(self.last_rect.center(), 1.0 / self.zoom);
        self.fit_pending = false;
    }

    /// Zoom and scroll position (kept per open document).
    pub fn view_state(&self) -> ViewState {
        ViewState { zoom: self.zoom, pan: self.pan, fit_pending: self.fit_pending }
    }

    /// Shows another document's view; the next frame redraws.
    pub fn set_view_state(&mut self, v: ViewState) {
        self.zoom = v.zoom;
        self.pan = v.pan;
        self.fit_pending = v.fit_pending;
        if let Some(gpu) = &mut self.gpu {
            gpu.last = None;
        }
        self.cancel_tool();
        self.selected_nodes.clear();
    }

    /// Sets the zoom (1.0 = 100%), keeping the middle of the canvas in place.
    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom_about(self.last_rect.center(), zoom / self.zoom);
        self.fit_pending = false;
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// Zooms by `factor` around the middle of the canvas.
    pub fn zoom_by(&mut self, factor: f64) {
        self.zoom_about(self.last_rect.center(), factor);
    }

    /// Zooms by `factor`, keeping the point under screen position `at` fixed.
    fn zoom_about(&mut self, at: Pos2, factor: f64) {
        if !self.last_rect.is_positive() {
            self.zoom = (self.zoom * factor).clamp(0.02, 32.0);
            return;
        }
        let anchor = self.to_comp(self.last_rect, at);
        self.zoom = (self.zoom * factor).clamp(0.02, 32.0);
        let now = self.to_screen(self.last_rect, anchor);
        self.pan += at - now;
    }

    /// Enter: finishes what the current tool is doing (crop, pen path).
    pub fn commit_tool(&mut self, app: &mut XenifyApp) {
        match app.tool {
            Tool::Crop => self.commit_crop(app),
            Tool::Pen if !self.pen.is_empty() || self.pen_continue.is_some() => self.finish_pen(app, false),
            _ => {}
        }
    }

    /// Text bounds with the same fonts the canvas uses.
    pub fn measure_text(&self, text: &xenify_anim::SceneText) -> kurbo::Rect {
        use xenify_anim::TextMeasure;
        self.text.borrow_mut().text_bounds(text)
    }

    /// A text layer's box in its own space now (as laid out on the canvas).
    pub fn layer_text_bounds(&self, app: &XenifyApp, id: Id) -> Option<kurbo::Rect> {
        let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
        match &scene.layer(id)?.content {
            xenify_anim::SceneContent::Text(t) => {
                Some(self.measure_text(t)).filter(|b| b.width() > 0.0 && b.height() > 0.0)
            }
            _ => None,
        }
    }

    /// Where the canvas's right-click menu was opened (composition space).
    pub fn menu_at(&self) -> Option<Point> {
        self.menu_at
    }

    /// A [`xenify_anim::TextMeasure`] using the canvas fonts.
    pub fn measurer(&self) -> impl xenify_anim::TextMeasure + '_ {
        struct M<'a>(&'a Viewport);
        impl xenify_anim::TextMeasure for M<'_> {
            fn text_bounds(&mut self, text: &xenify_anim::SceneText) -> kurbo::Rect {
                self.0.measure_text(text)
            }
        }
        M(self)
    }

    /// Installed + bundled font family names (computed once).
    pub fn font_families(&mut self) -> &[String] {
        let text = &self.text;
        self.families.get_or_insert_with(|| text.borrow_mut().families())
    }

    /// Cancels an in-progress tool action. Returns whether there was one.
    pub fn cancel_tool(&mut self) -> bool {
        let had = !self.pen.is_empty()
            || self.drag.is_some()
            || self.pen_continue.is_some()
            || self.crop.rect.is_some()
            || self.xdrag.is_some()
            || !self.stroke.is_empty();
        self.pen.clear();
        self.pen_continue = None;
        self.pen_drag = None;
        self.node_drag = None;
        self.text_drag = None;
        self.drag = None;
        self.create_end = None;
        self.xdrag = None;
        self.pending_pivot = None;
        self.stroke.clear();
        self.sculpt_last = None;
        self.crop = Default::default();
        self.slice.drag = None;
        self.slice_preview = None;
        self.zoom_box = None;
        had
    }

    /// Where composition point `p` is on screen.
    pub fn comp_to_screen(&self, p: Point) -> Pos2 {
        self.to_screen(self.last_rect, p)
    }

    /// The composition point under screen position `pos`, if it's over the canvas.
    pub fn screen_to_comp(&self, pos: Pos2) -> Option<Point> {
        self.last_rect.contains(pos).then(|| self.to_comp(self.last_rect, pos))
    }

    /// The canvas area on screen (last frame).
    pub fn screen_rect(&self) -> Rect {
        self.last_rect
    }

    fn to_screen(&self, rect: Rect, p: Point) -> Pos2 {
        rect.min + self.pan + EVec2::new((p.x * self.zoom) as f32, (p.y * self.zoom) as f32)
    }

    fn to_comp(&self, rect: Rect, p: Pos2) -> Point {
        let v = p - rect.min - self.pan;
        Point::new(v.x as f64 / self.zoom, v.y as f64 / self.zoom)
    }

    fn fit(&mut self, rect: Rect, w: u32, h: u32) {
        let margin = 40.0;
        let zx = (rect.width() as f64 - margin) / w as f64;
        let zy = (rect.height() as f64 - margin) / h as f64;
        self.zoom = zx.min(zy).clamp(0.02, 32.0);
        self.pan = (rect.size() - EVec2::new((w as f64 * self.zoom) as f32, (h as f64 * self.zoom) as f32)) / 2.0;
    }

    /// Draws the canvas and runs the current tool. Returns the canvas's
    /// response (for its right-click menu).
    pub fn show(
        &mut self,
        app: &mut XenifyApp,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        scene: &Scene,
    ) -> egui::Response {
        let rect = ui.available_rect_before_wrap();
        self.last_rect = rect;
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if self.fit_pending && rect.width() > 10.0 {
            self.fit(rect, scene.width, scene.height);
            self.fit_pending = false;
        }
        self.handle_zoom(ui, rect, &response);

        // The text being typed into is shown by its field, not drawn twice.
        let hidden = app.in_place.as_ref().map(|e| in_place::hide(scene, e));
        match frame.wgpu_render_state() {
            Some(rs) => self.render(rs, ui, rect, app, hidden.as_ref().unwrap_or(scene)),
            None => self.error = Some("wgpu is not available".into()),
        }
        let painter = ui.painter_at(rect);
        if let Some(gpu) = &self.gpu {
            painter.image(gpu.texture_id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        if let Some(err) = &self.error {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                err,
                egui::FontId::proportional(16.0),
                Color32::RED,
            );
        }

        self.draw_grid_and_guides(app, &painter, rect);
        if !self.rulers(app, ui, rect, &response) {
            self.handle_tools(app, ui, rect, &response, scene);
        }
        // A shape dragged out of the Shapes panel.
        if let Some(p) = response.dnd_release_payload::<crate::emoji_panel::EmojiPayload>()
            && let Some(at) = response.hover_pos()
            && let Some(e) = xenify_export::emoji::find(p.0)
        {
            crate::emoji_panel::insert(app, e, self.to_comp(rect, at).to_vec2());
        }
        if let Some(p) = response.dnd_release_payload::<crate::shape_library::LibPayload>()
            && let Some(at) = response.hover_pos()
        {
            crate::shape_library::add(app, p.0, self.to_comp(rect, at).to_vec2());
        }
        self.node_context_menu(app, &response);
        self.right_click_select(app, rect, &response, scene);
        self.draw_overlay(app, &painter, rect, scene, response.hover_pos().or(response.interact_pointer_pos()));
        self.in_place_editor(app, ui, rect, scene);
        if let Some(p) = egui::DragAndDrop::payload::<crate::shape_library::LibPayload>(ui.ctx())
            && let Some(at) = response.hover_pos()
            && let Some(s) = xenify_ops::library::find(p.0)
        {
            // A preview of the shape being dragged in, at its real size.
            let (w, h) = s.size;
            let r = Rect::from_center_size(at, egui::vec2((w * self.zoom) as f32, (h * self.zoom) as f32));
            crate::shape_library::paint(&painter, r, s, ACCENT);
        }
        self.draw_rulers(app, &painter, rect, ui.input(|i| i.pointer.latest_pos()));

        self.zoom_control(ui, rect);
        {
            let text = &self.text;
            let boxes = |l: &xenify_anim::SceneLayer| {
                (l.opacity > 0.02).then(|| Some(l.transform.transform_rect_bbox(l.bounds(&mut *text.borrow_mut())?)))?
            };
            crate::a11y::canvas(app, ui, &response, scene, boxes, |p| self.to_screen(rect, p));
        }
        response
    }

    /// A right-click selects what's under it (keeping a selection it is part
    /// of, and the selection when it is on empty canvas) for the menu.
    fn right_click_select(&mut self, app: &mut XenifyApp, rect: Rect, resp: &egui::Response, scene: &Scene) {
        if !resp.secondary_clicked() || matches!(app.tool, Tool::Node | Tool::Zoom) {
            return;
        }
        let Some(pos) = resp.interact_pointer_pos() else { return };
        let at = self.to_comp(rect, pos);
        self.menu_at = Some(at);
        let hit = hit_test(scene, at, 4.0 / self.zoom, &mut *self.text.borrow_mut())
            .or_else(|| crate::camera_ui::hit(app, at, 6.0 / self.zoom))
            .or_else(|| emitter_handle::hit(app, at, 8.0 / self.zoom))
            .map(|h| xenify_anim::outermost_group(app.comp(), h))
            .filter(|id| app.project().layer(*id).is_ok_and(|l| !l.meta.locked));
        if let Some(id) = hit
            && !app.is_selected(id)
        {
            app.select_only(Some(id));
        }
    }

    /// The zoom level in the canvas corner, with − / + and a menu of sizes.
    fn zoom_control(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let area = Rect::from_min_size(rect.right_bottom() - EVec2::new(150.0, 30.0), EVec2::new(144.0, 24.0));
        let mut child = ui
            .new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::right_to_left(egui::Align::Center)));
        egui::Frame::new()
            .fill(Color32::from_black_alpha(170))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(4, 1))
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.small_button("+").on_hover_text("Zoom in (Ctrl+=, or Ctrl+scroll)").clicked() {
                    self.set_zoom(self.zoom * 1.25);
                }
                ui.menu_button(format!("{:.0}%", self.zoom * 100.0), |ui| {
                    if ui.button("Fit to window").clicked() {
                        self.request_fit();
                        ui.close();
                    }
                    for pct in [25.0, 50.0, 100.0, 200.0, 400.0] {
                        if ui.button(format!("{pct:.0}%")).clicked() {
                            self.set_zoom(pct / 100.0);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Zoom level — click for sizes");
                if ui.small_button("−").on_hover_text("Zoom out (Ctrl+−, or Ctrl+scroll)").clicked() {
                    self.set_zoom(self.zoom / 1.25);
                }
            });
    }

    fn handle_zoom(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        let Some(hover) = response.hover_pos() else { return };
        let (scroll, zoom_delta, mods) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
        let mut factor = zoom_delta as f64;
        if mods.command || mods.alt {
            factor *= (scroll.y as f64 / 200.0).exp();
        } else if scroll != EVec2::ZERO {
            self.pan += scroll; // two-finger/scroll wheel pans
        }
        if (factor - 1.0).abs() > 1e-6 {
            let anchor = self.to_comp(rect, hover);
            self.zoom = (self.zoom * factor).clamp(0.02, 32.0);
            // Keep the point under the cursor fixed.
            let now = self.to_screen(rect, anchor);
            self.pan += hover - now;
        }
    }

    fn render(&mut self, rs: &egui_wgpu_state::RenderState, ui: &egui::Ui, rect: Rect, app: &XenifyApp, scene: &Scene) {
        let ppp = ui.ctx().pixels_per_point();
        let size = (((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1));

        if self.gpu.is_none() {
            match vello::Renderer::new(&rs.device, xenify_render::renderer_options()) {
                Ok(renderer) => {
                    let texture = xenify_render::create_target(&rs.device, size.0, size.1);
                    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let texture_id =
                        rs.renderer.write().register_native_texture(&rs.device, &view, wgpu::FilterMode::Linear);
                    self.gpu =
                        Some(Gpu { renderer, texture, texture_id, size, vscene: vello::Scene::new(), last: None });
                }
                Err(e) => {
                    self.error = Some(format!("Could not start the Vello renderer: {e}"));
                    return;
                }
            }
        }
        let gpu = self.gpu.as_mut().unwrap();
        if gpu.size != size {
            gpu.texture = xenify_render::create_target(&rs.device, size.0, size.1);
            let view = gpu.texture.create_view(&wgpu::TextureViewDescriptor::default());
            rs.renderer.write().update_egui_texture_from_wgpu_texture(
                &rs.device,
                &view,
                wgpu::FilterMode::Linear,
                gpu.texture_id,
            );
            gpu.size = size;
            gpu.last = None;
        }

        let look = crate::camera_ui::looking_through(app);
        let key = (
            app.editor.revision(),
            app.time.to_bits(),
            [
                app.comp.0 * 2 + look as u64,
                self.zoom.to_bits(),
                self.pan.x.to_bits() as u64,
                self.pan.y.to_bits() as u64,
                size.0 as u64,
                size.1 as u64,
            ],
        );
        if gpu.last == Some(key) {
            return;
        }
        gpu.last = Some(key);

        let view = Affine::scale(ppp as f64)
            * Affine::translate((self.pan.x as f64, self.pan.y as f64))
            * Affine::scale(self.zoom);
        gpu.vscene.reset();
        // Editing shows the canvas itself; the camera only when looking through it.
        let camera = if look { scene.camera } else { Affine::IDENTITY };
        xenify_render::encode_seen(scene, &mut gpu.vscene, view, camera, &mut self.text.borrow_mut());
        let target = gpu.texture.create_view(&wgpu::TextureViewDescriptor::default());
        if let Err(e) = gpu.renderer.render_to_texture(
            &rs.device,
            &rs.queue,
            &gpu.vscene,
            &target,
            &xenify_render::render_params(size.0, size.1, PASTEBOARD),
        ) {
            self.error = Some(format!("Render error: {e}"));
        }
    }

    fn handle_tools(&mut self, app: &mut XenifyApp, ui: &egui::Ui, rect: Rect, resp: &egui::Response, scene: &Scene) {
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let middle = ui.input(|i| i.pointer.middle_down());

        // Holding Space turns any tool into the hand (Space tapped alone plays; see keymap).
        let space = ui.input(|i| i.key_down(egui::Key::Space)) && !ui.ctx().text_edit_focused();
        if !space {
            self.space_panned = false;
        }
        let hand_tool = app.tool == Tool::Hand || space;
        let hand = hand_tool && resp.dragged_by(egui::PointerButton::Primary);
        if space && hand {
            self.space_panned = true;
        }
        if hand_tool && resp.hovered() {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        // Panning: middle mouse anywhere, any tool (and the hand tool).
        if (middle || hand) && resp.dragged() || matches!(self.drag, Some(Drag::Pan)) {
            if resp.drag_started() || self.drag.is_none() {
                self.drag = Some(Drag::Pan);
            }
            self.pan += resp.drag_delta();
            if resp.drag_stopped() || !resp.dragged() {
                self.drag = None;
            }
            return;
        }
        let Some(pos) = pointer else { return };
        let comp_pos = self.to_comp(rect, pos);
        // Seen through the camera, the canvas isn't where the tools think it is.
        if crate::camera_ui::looking_through(app) {
            return;
        }

        // The colour panel's eyedropper, or the eyedropper tool.
        if app.pick_color || app.tool == Tool::Eyedropper {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if resp.clicked() {
                let sample = crate::colors::sample(scene, comp_pos, 3.0 / self.zoom, &mut *self.text.borrow_mut());
                if app.pick_color {
                    crate::colors::pick_into_active(app, sample);
                } else {
                    crate::colors::eyedropper_tool(app, sample);
                }
            }
            return;
        }

        // The selected connector's ends and bends come before everything else.
        let connector_handle = app.tool == Tool::Select
            && (matches!(self.drag, Some(Drag::ConnectorEnd { .. } | Drag::ConnectorSegment { .. }))
                || (self.drag.is_none()
                    && self.xdrag.is_none()
                    && resp.drag_started_by(egui::PointerButton::Primary)
                    && self.start_connector_handle(app, rect, resp, pos)));
        let quick = !connector_handle
            && app.tool == Tool::Select
            && resp.clicked()
            && self.quick_connect(app, rect, pos, scene);
        match app.tool {
            Tool::Select if quick => {}
            Tool::Select if connector_handle => {
                let ctrl = ui.input(|i| i.modifiers.command);
                self.connector_handle_drag(app, rect, resp, comp_pos, scene, ctrl);
            }
            Tool::Select => {
                // A rectangle's corner circles round it; the selection's handles
                // (corners, edges, pivot, rotate zone) work like Free Transform;
                // everything else selects and moves.
                if self.drag.is_none() && self.xdrag.is_none() && self.on_corner_circle(app, rect, resp) {
                    self.corner_tool(app, rect, resp, pos, comp_pos);
                } else if self.xdrag.is_some()
                    || (self.drag.is_none() && self.on_transform_handles(app, rect, resp, pos, scene))
                {
                    self.xform_tool(app, rect, resp, pos, comp_pos, scene);
                } else {
                    self.select_tool(app, rect, resp, pos, comp_pos, scene);
                }
            }
            Tool::Rect | Tool::Ellipse | Tool::Line | Tool::Polygon => {
                self.shape_tool(app, rect, resp, comp_pos, ui.input(|i| i.modifiers.shift), scene)
            }
            Tool::Pen => self.pen_tool(app, rect, resp, pos, comp_pos),
            Tool::Node => self.node_tool(app, rect, resp, pos, comp_pos, scene),
            Tool::Text => self.text_tool(app, rect, resp, pos, comp_pos, scene),
            Tool::FreeTransform | Tool::Perspective => self.xform_tool(app, rect, resp, pos, comp_pos, scene),
            Tool::Pencil | Tool::Brush => self.freehand_tool(app, rect, resp, comp_pos),
            Tool::Sculpt => self.sculpt_tool(app, rect, resp, comp_pos, scene),
            Tool::Crop => self.crop_tool(app, rect, resp, pos, comp_pos, scene),
            Tool::Mask => self.mask_tool(app, rect, resp, pos),
            Tool::Knife => self.knife_tool(app, rect, resp, comp_pos, ui.input(|i| i.modifiers.shift)),
            Tool::Gradient => {
                self.gradient_tool(app, rect, resp, pos, comp_pos, scene, ui.input(|i| i.modifiers.shift))
            }
            Tool::Slice => self.slice_tool(app, rect, resp, pos, comp_pos),
            Tool::Connector => {
                let ctrl = ui.input(|i| i.modifiers.command);
                self.connector_tool(app, rect, resp, pos, comp_pos, scene, ctrl)
            }
            Tool::Zoom => self.zoom_tool(rect, resp, pos),
            Tool::Orbit => self.orbit_tool(app, resp, comp_pos, scene),
            Tool::Eyedropper | Tool::Hand => {}
        }
    }

    fn select_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        pos: Pos2,
        comp_pos: Point,
        scene: &Scene,
    ) {
        if resp.drag_started_by(egui::PointerButton::Primary) {
            // egui reports a drag only after a small movement; start from where
            // the button actually went down.
            let pos = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let comp_pos = self.to_comp(rect, pos);
            // Gradient handles, then the selected layer's transform handles, take priority.
            if let Some(drag) = self
                .gradient_handle_at(app, rect, scene, pos)
                .or_else(|| self.array_handle_at(app, rect, pos))
                .or_else(|| self.emitter_handle_at(app, rect, pos))
                .or_else(|| self.liquid_handle_at(app, rect, pos))
            {
                app.new_gesture();
                self.drag = Some(drag);
                return;
            }
            let handle = self.selection_geometry(app, rect, scene).and_then(|g| g.handle_at(pos));
            if let (Some(h), Some(layer)) = (handle, app.selected.and_then(|id| app.project().layer(id).ok()).cloned())
            {
                let t = app.time;
                let anchor = self.to_screen(
                    rect,
                    world_transform(app.comp(), layer.id, t) * layer.transform.anchor.value_at(t).to_point(),
                );
                app.new_gesture();
                self.drag = Some(match h {
                    Handle::Rotate => Drag::Rotate {
                        layer: layer.id,
                        start_rot: layer.transform.rotation.value_at(t),
                        anchor,
                        start_angle: (pos - anchor).angle(),
                    },
                    Handle::Corner => Drag::Scale {
                        layer: layer.id,
                        start_scale: layer.transform.scale.value_at(t),
                        anchor,
                        start_dist: (pos - anchor).length().max(1.0),
                    },
                });
                return;
            }
            let hit = hit_test(scene, comp_pos, 4.0 / self.zoom, &mut *self.text.borrow_mut())
                .or_else(|| crate::camera_ui::hit(app, comp_pos, 6.0 / self.zoom))
                .or_else(|| emitter_handle::hit(app, comp_pos, 8.0 / self.zoom));
            // A click selects the whole group; Ctrl+click reaches inside it.
            let direct = resp.ctx.input(|i| i.modifiers.command);
            let hit = hit.map(|h| if direct { h } else { xenify_anim::outermost_group(app.comp(), h) });
            let shift = resp.ctx.input(|i| i.modifiers.shift);
            // Locked layers (e.g. a background) can't be grabbed: drag a box there.
            let hit = hit.filter(|id| app.project().layer(*id).is_ok_and(|l| !l.meta.locked));
            match hit {
                // Shift+drag always draws a box, adding to the selection.
                _ if shift => {
                    self.drag = Some(Drag::Marquee { start: comp_pos, end: comp_pos, additive: true });
                    return;
                }
                None => {
                    // Empty canvas: drag a box to select what it touches.
                    if !shift {
                        app.select_only(None);
                    }
                    self.drag = Some(Drag::Marquee { start: comp_pos, end: comp_pos, additive: shift });
                    return;
                }
                // Dragging an already-selected layer moves the whole selection.
                Some(id) if app.is_selected(id) => app.selected_keys.clear(),
                other => app.select_only(other),
            }
            let t = app.time;
            let layers: Vec<(Id, Vec2, Affine)> = app
                .selected_layers()
                .into_iter()
                .filter_map(|id| app.project().layer(id).ok())
                .filter(|l| !l.meta.locked)
                // A child moves with its selected parent; don't move it twice.
                .filter(|l| l.meta.parent.is_none_or(|p| !app.is_selected(p)))
                .map(|l| {
                    let parent_inv =
                        l.meta.parent.map_or(Affine::IDENTITY, |p| world_transform(app.comp(), p, t).inverse());
                    (l.id, l.transform.position.value_at(t), parent_inv)
                })
                .collect();
            // A single IK-driven layer bends its chain instead of moving.
            if let [one] = app.selected_layers().as_slice()
                && let Ok(l) = app.project().layer(*one)
                && l.meta.rig.ik_chain > 0
                && l.meta.parent.is_some()
                && !l.meta.locked
            {
                let (id, anchor) =
                    (l.id, world_transform(app.comp(), l.id, t) * l.transform.anchor.value_at(t).to_point());
                app.new_gesture();
                self.drag = Some(Drag::Ik { layer: id, offset: anchor - comp_pos });
                return;
            }
            if hit.is_some() && !layers.is_empty() {
                app.new_gesture();
                let ids: Vec<Id> = layers.iter().map(|l| l.0).collect();
                let (start_box, snap) = self.snap_setup(app, scene, &ids);
                let connectors = ids
                    .iter()
                    .filter_map(|id| Some((*id, (**app.project().layer(*id).ok()?.meta.connector.as_ref()?).clone())))
                    .collect();
                self.drag = Some(Drag::Move { layers, start_mouse: comp_pos, start_box, snap, connectors });
            }
        } else if resp.double_clicked() || resp.triple_clicked() {
            // Double-click a symbol to open it, a shape to edit its points.
            let precomp = app.selected.and_then(|id| app.project().layer(id).ok()).and_then(|l| match l.kind {
                xenify_doc::LayerKind::Precomp { comp, .. } => Some(comp),
                _ => None,
            });
            // Connectors, and shapes on a diagram: type their label.
            let labelled = app.selected.and_then(|id| app.project().layer(id).ok()).is_some_and(|l| {
                l.meta.connector.is_some()
                    || (!app.comp().settings.kind.is_animation()
                        && matches!(l.kind, xenify_doc::LayerKind::Shape { .. }))
            });
            if let Some(comp) = precomp {
                crate::symbols::edit(app, comp);
            } else if labelled {
                // Type the label right there.
                if let Some(id) = app.selected {
                    start_in_place(app, id);
                }
            } else if app
                .selected
                .and_then(|id| app.project().layer(id).ok())
                .is_some_and(|l| matches!(l.kind, xenify_doc::LayerKind::Shape { .. }))
            {
                // Every shape is points: rectangles and ellipses too.
                app.tool = Tool::Node;
                app.status = "Editing points — drag points, edges or handles; the circles inside corners round \
                              them. V to go back."
                    .into();
            } else if app
                .selected
                .and_then(|id| app.project().layer(id).ok())
                .is_some_and(|l| matches!(l.kind, xenify_doc::LayerKind::Text(_)))
            {
                // Text: type into it where it is.
                if let Some(id) = app.selected {
                    start_in_place(app, id);
                }
            } else if app.selected.is_some() {
                // Other shapes and images: type exact values.
                app.dialog = Some(crate::dialogs::Dialog::position_size(app));
            }
        } else if resp.clicked() {
            let hit = hit_test(scene, comp_pos, 4.0 / self.zoom, &mut *self.text.borrow_mut())
                .or_else(|| crate::camera_ui::hit(app, comp_pos, 6.0 / self.zoom))
                .or_else(|| emitter_handle::hit(app, comp_pos, 8.0 / self.zoom));
            let direct = resp.ctx.input(|i| i.modifiers.command);
            let hit = hit.map(|h| if direct { h } else { xenify_anim::outermost_group(app.comp(), h) });
            match hit {
                Some(id) if resp.ctx.input(|i| i.modifiers.shift) => app.toggle_selection(id),
                other => app.select_only(other),
            }
        }

        if resp.dragged()
            && let Some(drag) = self.drag.clone()
        {
            match drag {
                Drag::Move { layers, start_mouse, start_box, snap, connectors } => {
                    let mut delta = comp_pos - start_mouse;
                    self.guides.clear();
                    // Shift keeps to one axis; Ctrl turns snapping off (as everywhere).
                    let mods = resp.ctx.input(|i| i.modifiers);
                    if mods.shift {
                        if delta.x.abs() > delta.y.abs() {
                            delta.y = 0.0;
                        } else {
                            delta.x = 0.0;
                        }
                    }
                    if let Some(b) = start_box
                        && !mods.command
                    {
                        let moved = b + delta;
                        let tol = SNAP_PX / self.zoom;
                        if let Some((d, x)) = best_snap(&[moved.x0, moved.center().x, moved.x1], &snap.xs, tol) {
                            delta.x += d;
                            self.guides.push(Guide::V(x));
                        }
                        if let Some((d, y)) = best_snap(&[moved.y0, moved.center().y, moved.y1], &snap.ys, tol) {
                            delta.y += d;
                            self.guides.push(Guide::H(y));
                        }
                    }
                    let mut cmds: Vec<Command> = layers
                        .iter()
                        .filter(|(id, _, _)| !connectors.iter().any(|(c, _)| c == id))
                        .filter_map(|&(id, start, parent_inv)| {
                            // Linear part only: the delta in the parent's space.
                            let c = parent_inv.as_coeffs();
                            let d = Vec2::new(c[0] * delta.x + c[2] * delta.y, c[1] * delta.x + c[3] * delta.y);
                            let value = Value::Vec2(start + d);
                            Command::edit_value(app.project(), id, PropPath::Position, app.time, value, app.auto_key)
                                .ok()
                        })
                        .collect();
                    // Connectors: loose ends and bends move by the drag (composition space).
                    let d = Vec2::new(delta.x, delta.y);
                    for (id, start) in &connectors {
                        let moved = |e: &xenify_doc::ConnectorEnd| {
                            if e.attach.is_some() { *e } else { xenify_doc::ConnectorEnd::free(e.point + d) }
                        };
                        if let Ok(cmd) = xenify_ops::diagram::edit_connector(app.project(), *id, |c| {
                            c.from = moved(&start.from);
                            c.to = moved(&start.to);
                            c.waypoints = start.waypoints.iter().map(|w| *w + d).collect();
                        }) {
                            cmds.push(cmd);
                        }
                    }
                    let label = if cmds.len() == 1 { "Move layer" } else { "Move layers" };
                    app.exec_merged(label, Command::batch(cmds));
                }
                Drag::Scale { layer, start_scale, anchor, start_dist } => {
                    let f = ((pos - anchor).length() / start_dist) as f64;
                    app.edit_value(layer, PropPath::Scale, Value::Vec2(start_scale * f));
                }
                Drag::Rotate { layer, start_rot, anchor, start_angle } => {
                    let mut deg = start_rot + ((pos - anchor).angle() - start_angle).to_degrees() as f64;
                    if resp.ctx.input(|i| i.modifiers.shift) {
                        deg = (deg / 15.0).round() * 15.0;
                    }
                    app.edit_value(layer, PropPath::Rotation, Value::Scalar(deg));
                }
                Drag::Gradient { layer, index, stroke, end, inv } => {
                    gradient_tool::move_gradient_end(app, layer, index, stroke, end, inv * comp_pos);
                }
                Drag::ArrayOrigin { layer, index, inv } => {
                    array_handle::move_array_origin(app, layer, index, inv * comp_pos);
                }
                Drag::LiquidLevel { layer, inv } => {
                    emitter_handle::drag_level(app, layer, inv, comp_pos);
                }
                Drag::EmitterArrow { layer, origin, turn } => {
                    let snap = resp.ctx.input(|i| i.modifiers.shift);
                    emitter_handle::drag_arrow(app, layer, origin, turn, comp_pos, snap);
                }
                Drag::Ik { layer, offset } => {
                    let target = comp_pos + offset;
                    match xenify_ops::rig::ik_commands(app.project(), app.comp, layer, target, app.time, app.auto_key) {
                        Ok(cmds) if !cmds.is_empty() => {
                            app.exec_merged("Pose", Command::batch(cmds));
                        }
                        Ok(_) => {}
                        Err(e) => app.fail(e.to_string()),
                    }
                }
                Drag::Marquee { start, additive, .. } => {
                    self.drag = Some(Drag::Marquee { start, end: comp_pos, additive });
                }
                Drag::Pan
                | Drag::Create { .. }
                | Drag::GradientLine { .. }
                | Drag::Connect { .. }
                | Drag::ConnectorEnd { .. }
                | Drag::ConnectorSegment { .. } => {}
            }
        }
        if resp.drag_stopped() {
            if let Some(Drag::Marquee { start, end, additive }) = self.drag.clone() {
                self.marquee_select(app, scene, kurbo::Rect::from_points(start, end), additive);
            }
            // Dropped into (or out of) a pool or lane: membership changes, in the same undo step.
            if let Some(Drag::Move { layers, .. }) = &self.drag {
                let ids: Vec<Id> = layers.iter().map(|l| l.0).collect();
                if let Ok(Some(cmd)) = xenify_ops::diagram::adopt(app.project(), app.comp, &ids, app.time) {
                    app.exec_merged("Move layers", cmd);
                }
            }
            self.drag = None;
            self.guides.clear();
        }
    }

    /// Selects every visible, unlocked object the box touches (whole groups).
    fn marquee_select(&mut self, app: &mut XenifyApp, scene: &Scene, area: kurbo::Rect, additive: bool) {
        let mut found: Vec<Id> = Vec::new();
        let mut text = self.text.borrow_mut();
        for l in &scene.layers {
            let Some(b) = l.bounds(&mut *text) else { continue };
            let b = l.transform.transform_rect_bbox(b);
            if l.opacity <= 0.02 || b.intersect(area).area() <= 0.0 && !area.contains(b.center()) {
                continue;
            }
            let id = xenify_anim::outermost_group(app.comp(), l.id);
            let usable = app.project().layer(id).is_ok_and(|x| x.meta.visible && !x.meta.locked);
            if usable && !found.contains(&id) {
                found.push(id);
            }
        }
        drop(text);
        if !additive {
            app.select_only(None);
        }
        for id in found {
            if !app.is_selected(id) {
                app.toggle_selection(id);
            }
        }
    }

    /// The selected layer's first gradient — the stroke's when the stroke
    /// swatch is active, else the fill's: (layer, shape index, whether it's
    /// the stroke's, gradient now, layer → composition transform). Only with
    /// one selected.
    #[allow(clippy::type_complexity)]
    fn gradient_target(
        &self,
        app: &XenifyApp,
        scene: &Scene,
    ) -> Option<(Id, usize, bool, xenify_doc::Gradient, Affine)> {
        if !matches!(app.tool, Tool::Select | Tool::Gradient) || !app.extra_selected.is_empty() {
            return None;
        }
        let id = app.selected?;
        let layer = app.project().layer(id).ok()?;
        let t = app.time;
        let fill =
            |(i, s): (usize, &xenify_doc::ShapeItem)| Some((i, false, s.fill.as_ref()?.gradient.as_ref()?.value_at(t)));
        let stroke = |(i, s): (usize, &xenify_doc::ShapeItem)| {
            Some((i, true, s.stroke.as_ref()?.gradient.as_ref()?.value_at(t)))
        };
        let shapes = layer.shapes();
        let find = |f: &dyn Fn((usize, &xenify_doc::ShapeItem)) -> Option<(usize, bool, xenify_doc::Gradient)>| {
            shapes.iter().enumerate().find_map(f)
        };
        let (index, stroke, g) = if app.paint.active == crate::app::PaintSlot::Stroke {
            find(&stroke).or_else(|| find(&fill))
        } else {
            find(&fill).or_else(|| find(&stroke))
        }?;
        Some((id, index, stroke, g, scene.layer(id)?.transform))
    }

    /// The moving layers' combined box, and the frame/other-layer lines to snap to.
    fn snap_setup(&self, app: &XenifyApp, scene: &Scene, moving: &[Id]) -> (Option<kurbo::Rect>, SnapTargets) {
        let mut text = self.text.borrow_mut();
        // Invisible layers (e.g. before a fade-in) aren't snap targets.
        let moving: Vec<Id> = moving
            .iter()
            .flat_map(|id| std::iter::once(*id).chain(xenify_anim::descendants(app.comp(), *id)))
            .collect();
        let moving = moving.as_slice();
        let mut boxes = scene.layers.iter().filter(|l| l.opacity > 0.02).filter_map(|l| {
            let b = l.transform.transform_rect_bbox(l.bounds(&mut *text)?);
            Some((l.id, b))
        });
        let s = &app.comp().settings;
        let p = &app.prefs;
        let (w, h) = (s.width as f64, s.height as f64);
        let mut snap = SnapTargets::default();
        let mut start_box: Option<kurbo::Rect> = None;
        let m = p.snap_margin.max(0.0);
        if p.snap_objects {
            snap.xs.extend([0.0, w / 2.0, w]);
            snap.ys.extend([0.0, h / 2.0, h]);
            // A margin: also stop that far inside the frame.
            if m > 0.0 {
                snap.xs.extend([m, w - m]);
                snap.ys.extend([m, h - m]);
            }
        }
        for (id, b) in boxes.by_ref() {
            if moving.contains(&id) {
                start_box = Some(start_box.map_or(b, |s| s.union(b)));
            } else if p.snap_objects && (b.width() < w * 0.95 || b.height() < h * 0.95) {
                // (Full-frame backgrounds would only duplicate the frame lines.)
                snap.xs.extend([b.x0, b.center().x, b.x1]);
                snap.ys.extend([b.y0, b.center().y, b.y1]);
                // …and a margin away from other objects' edges.
                if m > 0.0 {
                    snap.xs.extend([b.x0 - m, b.x1 + m]);
                    snap.ys.extend([b.y0 - m, b.y1 + m]);
                }
            }
        }
        if p.snap_guides && p.show_guides {
            for g in &s.guides {
                let lines = if m > 0.0 { vec![g.pos, g.pos - m, g.pos + m] } else { vec![g.pos] };
                if g.vertical { snap.xs.extend(lines) } else { snap.ys.extend(lines) }
            }
        }
        if p.snap_grid {
            let step = s.grid_spacing();
            if step > 0.0 {
                snap.xs.extend((0..=(w / step) as usize).map(|i| i as f64 * step));
                snap.ys.extend((0..=(h / step) as usize).map(|i| i as f64 * step));
            }
        }
        (start_box, snap)
    }

    fn shape_tool(
        &mut self,
        app: &mut XenifyApp,
        rect: Rect,
        resp: &egui::Response,
        comp_pos: Point,
        square: bool,
        scene: &Scene,
    ) {
        // Alt draws from the centre; Ctrl turns snapping off.
        let (from_centre, free) = resp.ctx.input(|i| (i.modifiers.alt, i.modifiers.command));
        if resp.hovered() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let mut start = origin.map_or(comp_pos, |p| self.to_comp(rect, p));
            let (_, snap) = self.snap_setup(app, scene, &[]);
            if !free {
                start = self.snap_point(start, &snap);
            }
            self.drag = Some(Drag::Create { start, snap });
        }
        if resp.clicked_by(egui::PointerButton::Primary) && self.drag.is_none() {
            // A click (no drag): type the exact size.
            app.dialog = Some(crate::dialogs::Dialog::ShapeSize(crate::shape_dialog::ShapeSize::new(app, comp_pos)));
            return;
        }
        let Some(Drag::Create { start, snap }) = self.drag.clone() else { return };
        // Corners snap to the frame and other layers' edges and centres (Ctrl: off).
        self.guides.clear();
        let mut end = if free { comp_pos } else { self.snap_point(comp_pos, &snap) };
        let line = app.tool == Tool::Line;
        if square {
            if line {
                end = xenify_ops::draw::snap_angle(start, end);
            } else {
                let d = end - start;
                let s = d.x.abs().max(d.y.abs());
                end = start + Vec2::new(s.copysign(d.x), s.copysign(d.y));
            }
        }
        let (a, b) = if from_centre && !line { (start - (end - start), end) } else { (start, end) };
        self.create_end = Some(b);
        self.create_start = Some(a);
        if resp.drag_stopped() {
            self.drag = None;
            self.create_end = None;
            self.create_start = None;
            self.guides.clear();
            let size = Vec2::new((b.x - a.x).abs(), (b.y - a.y).abs());
            let centre = a.midpoint(b).to_vec2();
            if line {
                if a.distance(b) < 2.0 {
                    return;
                }
                let d = b - a;
                create_line(app, centre, d);
                return;
            }
            if size.x < 2.0 || size.y < 2.0 {
                return;
            }
            create_shape(app, app.tool, centre, size);
        }
    }

    /// Snaps a point to the nearest target lines, adding guides for them.
    fn snap_point(&mut self, p: Point, snap: &SnapTargets) -> Point {
        let tol = SNAP_PX / self.zoom;
        let mut q = p;
        if let Some((d, x)) = best_snap(&[p.x], &snap.xs, tol) {
            q.x += d;
            self.guides.push(Guide::V(x));
        }
        if let Some((d, y)) = best_snap(&[p.y], &snap.ys, tol) {
            q.y += d;
            self.guides.push(Guide::H(y));
        }
        q
    }

    fn selection_geometry(&self, app: &XenifyApp, rect: Rect, scene: &Scene) -> Option<SelectionGeometry> {
        let id = app.selected?;
        let camera = xenify_anim::camera_frame(app.comp(), id, app.time);
        let corners = match scene.layer(id) {
            // A camera: its (turned) frame.
            _ if camera.is_some() => camera.unwrap().map(|p| self.to_screen(rect, p)),
            Some(layer) => {
                let b = layer.bounds(&mut *self.text.borrow_mut())?;
                [b.origin(), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]
                    .map(|p| self.to_screen(rect, layer.transform * p))
            }
            // Groups (and nulls): the box around everything inside them.
            None => {
                let b = xenify_anim::world_box(app.comp(), scene, id, &mut *self.text.borrow_mut())?;
                [b.origin(), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]
                    .map(|p| self.to_screen(rect, p))
            }
        };
        let top_mid = corners[0] + (corners[1] - corners[0]) / 2.0;
        let centre = corners[0] + (corners[2] - corners[0]) / 2.0;
        let up = (top_mid - centre).normalized();
        let up = if up.is_finite() { up } else { -EVec2::Y };
        Some(SelectionGeometry { corners, rotate: top_mid + up * ROTATE_OFFSET, top_mid })
    }

    /// What the current drag is doing, as a number by the pointer: W × H
    /// while drawing, Δx/Δy while moving, % while scaling, ° while rotating.
    pub(crate) fn live_numbers(&self, app: &XenifyApp) -> Option<String> {
        let t = app.time;
        let u = app.comp().settings.units;
        let len = |v: f64| u.format(v);
        match self.drag.as_ref()? {
            Drag::Create { .. } => {
                let (a, b) = (self.create_start?, self.create_end?);
                Some(format!("{} × {}", len((b.x - a.x).abs()), len((b.y - a.y).abs())))
            }
            Drag::Move { layers, .. } => {
                let (id, start, to_parent) = layers.first()?;
                let now = app.project().layer(*id).ok()?.transform.position.value_at(t);
                // In composition space, whatever the parent does.
                let d = to_parent.inverse() * now.to_point() - to_parent.inverse() * start.to_point();
                let sign = |v: f64| if v >= 0.0 { format!("+{}", len(v)) } else { format!("−{}", len(-v)) };
                Some(format!("Δx {}   Δy {}", sign(d.x), sign(d.y)))
            }
            Drag::Scale { layer, .. } => {
                let s = app.project().layer(*layer).ok()?.transform.scale.value_at(t);
                Some(if (s.x - s.y).abs() < 1e-6 {
                    format!("{:.0}%", s.x * 100.0)
                } else {
                    format!("{:.0}% × {:.0}%", s.x * 100.0, s.y * 100.0)
                })
            }
            Drag::Rotate { layer, .. } => {
                let r = app.project().layer(*layer).ok()?.transform.rotation.value_at(t);
                Some(format!("{r:.1}°"))
            }
            _ => None,
        }
    }

    fn draw_live_numbers(&self, app: &XenifyApp, painter: &egui::Painter, hover: Option<Pos2>) {
        let (Some(text), Some(at)) = (self.live_numbers(app), hover) else { return };
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), Color32::WHITE);
        let pos = at + EVec2::new(16.0, 18.0);
        let bg = Rect::from_min_size(pos, galley.size()).expand(4.0);
        painter.rect_filled(bg, 4.0, Color32::from_black_alpha(190));
        painter.galley(pos, galley, Color32::WHITE);
    }

    fn draw_overlay(&self, app: &XenifyApp, painter: &egui::Painter, rect: Rect, scene: &Scene, hover: Option<Pos2>) {
        let stroke = Stroke::new(1.5, ACCENT);
        // Composition frame outline.
        let frame = Rect::from_min_max(
            self.to_screen(rect, Point::ZERO),
            self.to_screen(rect, Point::new(scene.width as f64, scene.height as f64)),
        );
        // Auto Keying is on by default, so its reminder is a quiet one.
        let frame_stroke = if app.auto_key {
            Stroke::new(1.0, crate::widgets::RECORD.gamma_multiply(0.6))
        } else {
            Stroke::new(1.0, Color32::from_gray(70))
        };
        painter.rect_stroke(frame, 0.0, frame_stroke, StrokeKind::Outside);
        if crate::camera_ui::looking_through(app) {
            crate::camera_ui::draw_banner(painter, rect);
            return;
        }
        crate::camera_ui::draw_frames(app, painter, |p| self.to_screen(rect, p));
        if let Some(Drag::Marquee { start, end, .. }) = &self.drag {
            let r = Rect::from_two_pos(self.to_screen(rect, *start), self.to_screen(rect, *end));
            painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }

        for id in &app.extra_selected {
            if let Some(b) = xenify_anim::world_box(app.comp(), scene, *id, &mut *self.text.borrow_mut()) {
                let r =
                    Rect::from_min_max(self.to_screen(rect, b.origin()), self.to_screen(rect, Point::new(b.x1, b.y1)));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), StrokeKind::Middle);
            }
        }
        let own_box = matches!(
            app.tool,
            Tool::Select
                | Tool::Node
                | Tool::FreeTransform
                | Tool::Perspective
                | Tool::Crop
                | Tool::Mask
                | Tool::Slice
                | Tool::Zoom
                | Tool::Knife
        ) || (app.tool == Tool::Text && self.selected_text_box(app, scene).is_some());
        if !own_box && let Some(g) = self.selection_geometry(app, rect, scene) {
            painter.add(egui::Shape::closed_line(g.corners.to_vec(), stroke));
            painter.line_segment([g.top_mid, g.rotate], stroke);
            painter.circle(g.rotate, HANDLE * 0.7, Color32::WHITE, stroke);
            for c in g.corners {
                painter.rect(
                    Rect::from_center_size(c, EVec2::splat(HANDLE)),
                    1.0,
                    Color32::WHITE,
                    stroke,
                    StrokeKind::Middle,
                );
            }
            if let Some(layer) = app.selected.and_then(|id| app.project().layer(id).ok()) {
                let t = app.time;
                let a = self.to_screen(
                    rect,
                    world_transform(app.comp(), layer.id, t) * layer.transform.anchor.value_at(t).to_point(),
                );
                painter.circle_stroke(a, 4.0, stroke);
                painter.line_segment([a - EVec2::X * 7.0, a + EVec2::X * 7.0], stroke);
                painter.line_segment([a - EVec2::Y * 7.0, a + EVec2::Y * 7.0], stroke);
            }
        }

        self.draw_array_handle(app, painter, rect);
        self.draw_emitters(app, painter, rect);
        self.draw_liquid(app, painter, rect);

        // Gradient handles: a line from start to end, dots in the end colours.
        if let Some((_, _, _, g, to_comp)) = self.gradient_target(app, scene) {
            let (a, b) =
                (self.to_screen(rect, to_comp * g.start.to_point()), self.to_screen(rect, to_comp * g.end.to_point()));
            let dark = Stroke::new(3.0, Color32::from_black_alpha(140));
            painter.line_segment([a, b], dark);
            painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
            if g.kind == xenify_doc::GradientKind::Radial {
                painter.circle_stroke(a, (b - a).length(), Stroke::new(1.0, Color32::from_white_alpha(120)));
            }
            let stops = g.sorted_stops();
            // The colours in between, as small dots along the line.
            for st in stops.iter().skip(1).take(stops.len().saturating_sub(2)) {
                let c = crate::colors::to_color32(st.color.with_alpha(1.0));
                painter.circle(a + (b - a) * st.offset as f32, HANDLE * 0.45, c, Stroke::new(1.0, Color32::WHITE));
            }
            for (p, c) in [(a, stops.first()), (b, stops.last())] {
                let c = c.map_or(Color::WHITE, |s| s.color);
                let fill = Color32::from_rgb((c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8);
                painter.circle(p, HANDLE * 0.8, fill, Stroke::new(1.5, Color32::WHITE));
            }
        }

        if let (Some(Drag::Create { .. }), Some(start), Some(end)) =
            (self.drag.clone(), self.create_start, self.create_end)
        {
            let r = Rect::from_two_pos(self.to_screen(rect, start), self.to_screen(rect, end));
            if app.tool == Tool::Line {
                painter.line_segment([self.to_screen(rect, start), self.to_screen(rect, end)], stroke);
            } else if app.tool == Tool::Polygon {
                let o = &app.tool_options;
                let path =
                    xenify_ops::draw::figure_path(o.figure, &o.figure_params(), kurbo::Rect::from_points(start, end));
                path_tools::draw_bezpath(painter, &path, |p| self.to_screen(rect, p), stroke);
            } else if app.tool == Tool::Ellipse {
                let pts: Vec<Pos2> = (0..64)
                    .map(|i| {
                        let a = i as f32 / 64.0 * std::f32::consts::TAU;
                        r.center() + EVec2::new(a.cos() * r.width() / 2.0, a.sin() * r.height() / 2.0)
                    })
                    .collect();
                painter.add(egui::Shape::closed_line(pts, stroke));
            } else {
                painter.rect_stroke(r, 0.0, stroke, StrokeKind::Middle);
            }
        }

        // Smart guides (magenta, across the whole view).
        let guide = Stroke::new(1.0, Color32::from_rgb(255, 64, 160));
        for g in &self.guides {
            match *g {
                Guide::V(x) => {
                    let sx = self.to_screen(rect, Point::new(x, 0.0)).x;
                    painter.line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], guide);
                }
                Guide::H(y) => {
                    let sy = self.to_screen(rect, Point::new(0.0, y)).y;
                    painter.line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], guide);
                }
            }
        }
        self.draw_live_numbers(app, painter, hover);
        // Hover: outline what a click would pick (the same rule as the click).
        self.hover_pick.set(None);
        if app.tool == Tool::Select
            && self.drag.is_none()
            && let Some(hp) = hover.filter(|p| rect.contains(*p))
        {
            let cp = self.to_comp(rect, hp);
            let direct = painter.ctx().input(|i| i.modifiers.command);
            let hit = hit_test(scene, cp, 4.0 / self.zoom, &mut *self.text.borrow_mut())
                .map(|h| if direct { h } else { xenify_anim::outermost_group(app.comp(), h) })
                .filter(|id| !app.is_selected(*id) && app.project().layer(*id).is_ok_and(|l| !l.meta.locked));
            if let Some(id) = hit
                && let Some(b) = xenify_anim::world_box(app.comp(), scene, id, &mut *self.text.borrow_mut())
            {
                let r = Rect::from_two_pos(
                    self.to_screen(rect, Point::new(b.x0, b.y0)),
                    self.to_screen(rect, Point::new(b.x1, b.y1)),
                );
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.85)), StrokeKind::Outside);
                self.hover_pick.set(Some(id));
            }
        }
        match app.tool {
            Tool::Select | Tool::FreeTransform | Tool::Perspective => self.draw_xform(app, painter, rect, scene),
            Tool::Pencil | Tool::Brush => {
                self.draw_freehand(app, painter, rect);
                self.draw_brush_ring(app, painter, rect, hover);
            }
            Tool::Sculpt => self.draw_sculpt_brush(app, painter, rect, hover),
            Tool::Crop => self.draw_crop(app, painter, rect),
            Tool::Mask => self.draw_masks(app, painter, rect),
            Tool::Knife => self.draw_knife(painter, rect),
            _ => {}
        }
        self.draw_text_box(app, painter, rect, scene);
        self.draw_connectors(app, painter, rect, scene, hover);
        if app.prefs.show_slices {
            self.draw_slices(app, painter, rect);
        }
        if let (Some(a), Some(b)) = (self.zoom_box, hover) {
            painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
        }
        self.draw_pen(painter, rect);
        if app.tool == Tool::Pen {
            self.draw_open_ends(app, painter, rect);
        }
        if app.tool == Tool::Node {
            self.draw_nodes(app, painter, rect);
        } else if app.tool == Tool::Select {
            self.draw_corner_widgets(app, painter, rect);
        }
    }
}

#[derive(Clone, Copy)]
enum Handle {
    Corner,
    Rotate,
}

struct SelectionGeometry {
    corners: [Pos2; 4],
    rotate: Pos2,
    top_mid: Pos2,
}

impl SelectionGeometry {
    fn handle_at(&self, p: Pos2) -> Option<Handle> {
        let near = |q: Pos2| (q - p).length() <= HANDLE + 2.0;
        if near(self.rotate) {
            Some(Handle::Rotate)
        } else if self.corners.iter().any(|c| near(*c)) {
            Some(Handle::Corner)
        } else {
            None
        }
    }
}

/// The smallest correction that makes one of `edges` land on one of
/// `targets` within `tol`: returns (correction, target line).
fn best_snap(edges: &[f64], targets: &[f64], tol: f64) -> Option<(f64, f64)> {
    edges
        .iter()
        .flat_map(|e| targets.iter().map(move |t| (t - e, *t)))
        .filter(|(d, _)| d.abs() <= tol)
        .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

#[cfg(test)]
mod snap_tests {
    #[test]
    fn best_snap_picks_the_closest_line() {
        assert_eq!(super::best_snap(&[10.0, 50.0, 90.0], &[0.0, 52.0, 200.0], 5.0), Some((2.0, 52.0)));
        assert_eq!(super::best_snap(&[10.0], &[20.0], 5.0), None);
    }
}

/// `shape` in the colour panel's fill and stroke (a white outline if both are off).
fn styled(app: &XenifyApp, shape: ShapeItem) -> ShapeItem {
    let s = app.paint.apply_to(shape);
    if s.fill.is_none() && s.stroke.is_none() {
        return ShapeItem {
            stroke: app.paint.apply_to(s.clone()).stroke.or(Some(xenify_doc::Stroke {
                cap_roundness: 0.0,
                style: Default::default(),
                color: Color::WHITE.into(),
                width: app.paint.stroke_width.max(1.0).into(),
                join: xenify_doc::LineJoin::Round,
                cap: xenify_doc::LineCap::Round,
                ..Default::default()
            })),
            ..s
        };
    }
    s
}

/// Adds a new shape layer on top, selects it and switches back to the select tool.
pub fn add_layer(app: &mut XenifyApp, name: &str, shapes: Vec<ShapeItem>, position: Vec2) {
    add_layer_rotated(app, name, shapes, position, 0.0);
}

/// [`add_layer`] with the layer rotated by `degrees`.
pub fn add_layer_rotated(app: &mut XenifyApp, name: &str, shapes: Vec<ShapeItem>, position: Vec2, degrees: f64) {
    let id = Id(app.project().next_id);
    let n = app.comp().layers.iter().filter(|l| l.meta.name.starts_with(name)).count() + 1;
    let mut layer = Layer::shape(id, format!("{name} {n}"), shapes).with_position(position);
    layer.transform.rotation = degrees.into();
    if app.exec(&format!("Add {name}"), Command::AddLayer { comp: app.comp, index: None, layer }) {
        app.selected = Some(id);
        app.selected_keys.clear();
        app.tool = Tool::Select;
    }
}

/// Adds a text layer with its baseline starting at `position` and puts the
/// cursor in its text field.
pub fn add_text_layer(app: &mut XenifyApp, position: Vec2) {
    add_text(app, position, None);
}

/// A new text layer at `position`: point text, or text wrapping in a box of
/// `size` (whose top-left is `position`). Selected, ready to type into.
pub(crate) fn add_text(app: &mut XenifyApp, position: Vec2, size: Option<kurbo::Size>) {
    let id = Id(app.project().next_id);
    let n = app.comp().layers.iter().filter(|l| matches!(l.kind, xenify_doc::LayerKind::Text(_))).count() + 1;
    let mut font = (app.comp().settings.height as f64 / 10.0).round().max(8.0);
    let mut text = xenify_doc::TextLayer::new("Text", xenify_render::DEFAULT_FAMILY, font, Color::WHITE);
    if let Some(size) = size {
        // Body text in a box (half the size of a heading), smaller for small
        // boxes so the first line shows.
        font = (font / 2.0).round().min((size.height * 0.8).round()).max(8.0);
        text.font_size = font.into();
        text.valign = xenify_doc::TextVAlign::Top;
        text.frame = Some(xenify_doc::TextFrame::new(kurbo::Rect::from_origin_size(Point::ZERO, size)));
    }
    let layer = Layer::new(id, format!("Text {n}"), xenify_doc::LayerKind::Text(text)).with_position(position);
    if app.exec("Add Text", Command::AddLayer { comp: app.comp, index: None, layer }) {
        app.selected = Some(id);
        app.selected_keys.clear();
        // Straight into typing (the placeholder selected); the Text tool stays on.
        app.new_gesture();
        app.in_place = Some(InPlace::text_layer(id, "Text".into(), true));
        app.status = "Type on the canvas · Esc or click elsewhere when done".into();
    }
}

impl Viewport {
    fn orbit_tool(&mut self, app: &mut XenifyApp, resp: &egui::Response, comp_pos: Point, scene: &Scene) {
        let solid_of = |app: &XenifyApp, id: Id| {
            app.project().layer(id).ok().and_then(|l| match &l.kind {
                xenify_doc::LayerKind::Solid(s) => Some((s.rot_x.value_at(app.time), s.rot_y.value_at(app.time))),
                _ => None,
            })
        };
        if resp.hovered() {
            resp.ctx.set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let hit = xenify_anim::hit_test(scene, comp_pos, 4.0 / self.zoom, &mut *self.text.borrow_mut());
            // The clicked 3D object, else the selected one.
            let target = hit
                .filter(|id| solid_of(app, *id).is_some())
                .or(app.selected.filter(|id| solid_of(app, *id).is_some()));
            match target.and_then(|id| Some((id, solid_of(app, id)?))) {
                Some((id, start)) => {
                    app.select_only(Some(id));
                    app.new_gesture();
                    self.orbit = Some((id, start, comp_pos));
                }
                None => app.status = "Drag a 3D object to turn it (Layer › 3D adds one, or extrudes a shape).".into(),
            }
        }
        if resp.dragged()
            && let Some((id, start, from)) = self.orbit
        {
            let shift = resp.ctx.input(|i| i.modifiers.shift);
            Self::orbit_drag(app, id, start, (comp_pos - from) * self.zoom, shift);
        }
        if resp.drag_stopped() {
            self.orbit = None;
        }
    }

    /// Zoom tool: click zooms in (Alt: out) around the pointer, dragging a box zooms to fit it.
    fn zoom_tool(&mut self, rect: Rect, resp: &egui::Response, pos: Pos2) {
        // Any modifier zooms out (window managers often take Alt+click for themselves).
        let out = resp.ctx.input(|i| i.modifiers.alt || i.modifiers.shift || i.modifiers.command);
        if resp.hovered() {
            resp.ctx.set_cursor_icon(if out { egui::CursorIcon::ZoomOut } else { egui::CursorIcon::ZoomIn });
        }
        if resp.clicked() {
            self.zoom_about(pos, if out { 0.5 } else { 2.0 });
        }
        if resp.secondary_clicked() {
            self.zoom_about(pos, 0.5);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            self.zoom_box = resp.ctx.input(|i| i.pointer.press_origin());
        }
        if resp.drag_stopped()
            && let Some(a) = self.zoom_box.take()
        {
            let r = Rect::from_two_pos(a, pos);
            if r.width() > 4.0 && r.height() > 4.0 {
                let (c0, c1) = (self.to_comp(rect, r.min), self.to_comp(rect, r.max));
                let zx = rect.width() as f64 / (c1.x - c0.x);
                let zy = rect.height() as f64 / (c1.y - c0.y);
                self.zoom = zx.min(zy).clamp(0.02, 32.0);
                let centre = c0.midpoint(c1);
                self.pan = rect.size() / 2.0 - EVec2::new((centre.x * self.zoom) as f32, (centre.y * self.zoom) as f32);
            }
        }
    }
}

/// Adds a line from `centre − d/2` to `centre + d/2` in the stroke colour.
pub fn create_line(app: &mut XenifyApp, centre: Vec2, d: Vec2) {
    let mut path = kurbo::BezPath::new();
    path.move_to((-d / 2.0).to_point());
    path.line_to((d / 2.0).to_point());
    let color = app.paint.stroke.or(app.paint.fill).unwrap_or(Color::WHITE);
    let mut shape = ShapeItem::path(path, None, Some((color, app.paint.stroke_width.max(1.0))));
    if let Some(s) = &mut shape.stroke {
        s.cap = xenify_doc::LineCap::Round;
    }
    add_layer(app, "Line", vec![shape], centre);
    app.tool = Tool::Line;
}

/// Adds a rectangle, ellipse or polygon/star (as set in the tool options) of
/// `size`, centred on `centre`, in the current colours; keeps the tool.
pub fn create_shape(app: &mut XenifyApp, tool: Tool, centre: Vec2, size: Vec2) {
    let (name, shape) = match tool {
        Tool::Rect => {
            let mut shape = ShapeItem::rect(size, Color::WHITE);
            let r = app.tool_options.corner_radius;
            if r > 0.0
                && let xenify_doc::Geometry::Rect { corner_radius, .. } = &mut shape.geometry
            {
                *corner_radius = r.into();
                shape.name = "Rounded Rectangle".into();
            }
            (if r > 0.0 { "Rounded Rectangle" } else { "Rectangle" }, shape)
        }
        Tool::Polygon => {
            let o = &app.tool_options;
            let half = kurbo::Rect::from_center_size(Point::ZERO, size.to_size());
            // Polygons, stars and gears stay live (sides, points and teeth editable).
            let mut shape =
                ShapeItem::path(xenify_ops::draw::figure_path(o.figure, &o.figure_params(), half), None, None);
            if let Some(g) = xenify_ops::draw::live_figure(o.figure, &o.figure_params(), size.to_size()) {
                shape.geometry = g;
            }
            shape.name = o.figure_name().into();
            (o.figure_name(), shape)
        }
        _ => ("Ellipse", ShapeItem::ellipse(size, Color::WHITE)),
    };
    let shape = styled(app, shape);
    add_layer(app, name, vec![shape], centre);
    app.tool = tool;
}

/// Re-exported so this module can name the wgpu render state type.
mod egui_wgpu_state {
    pub use eframe::egui_wgpu::RenderState;
}
