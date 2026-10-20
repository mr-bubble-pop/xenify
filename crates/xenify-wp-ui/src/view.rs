//! The document view: lays the document out, paints it with Vello and
//! turns keyboard, mouse, IME and clipboard events into edits.

use crate::canvas::Canvas;
use egui::{Color32, CursorIcon, Event, EventFilter, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};
use xenify_wp_doc::{CharFmt, Editor, Pos, Selection, ops};
use xenify_wp_layout::{DocLayout, Engine, Fonts};
use std::time::{Duration, Instant};

/// Screen points per document point at 100% zoom (96 dpi screens).
const PT: f32 = 96.0 / 72.0;
/// Grey around the page.
const BLINK: Duration = Duration::from_millis(530);
/// Space above and below the sheet, in screen points.
const GAP: f32 = 24.0;

/// Dragging a frame: which handle (`None`: the whole frame), where the
/// drag started (document space) and the frame's box then.
#[derive(Clone, Copy)]
struct FrameDrag {
    id: xenify_wp_doc::NodeId,
    handle: Option<(f64, f64)>,
    start: kurbo::Point,
    orig: (f32, f32, f32, f32),
}

/// Resizing a picture by a corner: the picture, which corner (x, y as 0 or
/// 1), where the drag began, its size then, and the size now (applied on release).
#[derive(Clone, Copy)]
struct ImageDrag {
    id: xenify_wp_doc::NodeId,
    corner: (f64, f64),
    start: kurbo::Point,
    orig: (f32, f32),
    size: (f32, f32),
}

/// Dragging the line between columns `col - 1` and `col` of a table: the
/// columns' edges (doc x) and relative widths when the drag began.
#[derive(Clone)]
struct ColDrag {
    table: xenify_wp_doc::NodeId,
    col: usize,
    edges: Vec<f64>,
    widths: Vec<f32>,
}

/// One table's placed geometry on one page: column edges and row tops (doc space).
struct TableGrid {
    id: xenify_wp_doc::NodeId,
    cols: Vec<f64>,
    rows: Vec<(usize, f64, f64)>,
    bbox: kurbo::Rect,
}

/// The 8 resize handles of a box, as fractions of its width and height.
const HANDLES: [(f64, f64); 8] =
    [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (1.0, 0.5), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)];

#[derive(Clone, Copy, PartialEq)]
enum DragBy {
    Char,
    Word(Selection),
    Paragraph(Selection),
}

/// What the cached page texture was drawn for: revision, visible rect and
/// page origin, zoom, pixels per point and desk colour.
type RenderKey = (u64, [i32; 6], u32, u32, [u8; 4]);

pub struct DocView {
    pub engine: Engine,
    pub layout: DocLayout,
    laid_out_revision: Option<u64>,
    canvas: Canvas,
    rendered: Option<RenderKey>,
    pub zoom: f32,
    /// The caret sits at the end of a wrapped line rather than the start of the next.
    upstream: bool,
    /// x the caret aims for when moving up/down, in document points.
    goal_x: Option<f64>,
    /// IME composition in progress (shown, not yet in the document).
    pub preedit: Option<String>,
    drag: Option<DragBy>,
    last_press: Option<(Instant, Pos2, u32)>,
    /// The page whose header and footer are being edited (double-click in the margin).
    pub hf_edit: Option<usize>,
    /// Find: every match (highlighted), and which is the current one.
    pub find_marks: Vec<(xenify_wp_doc::NodeId, std::ops::Range<usize>)>,
    pub find_current: Option<usize>,
    /// More carets or selections besides the editor's (Alt+click, Ctrl+D):
    /// typing and deleting happen at each.
    pub extra: Vec<Selection>,
    /// Format for the next typed text (Ctrl+B with nothing selected).
    pub pending_fmt: Option<CharFmt>,
    /// Which typed shortcuts (`* `, `1. `, `# `…) become formatting.
    pub autoformat: ops::Autoformat,
    /// Small fixes as you type (`teh` → `the`, curly quotes).
    pub autocorrect: xenify_wp_doc::autocorrect::Autocorrect,
    blink_from: Instant,
    reveal_caret: bool,
    /// Scroll the caret's line to the top of the view (Navigation), not just into view.
    reveal_top: bool,
    pub id: egui::Id,
    /// The document's top-left in screen space last frame (for tests).
    pub doc_origin: Pos2,
    /// What was last copied here, with formatting, keyed by its plain text:
    /// pasting that text again pastes the formatted version.
    /// What Write last copied: its plain text and the fragment itself.
    clip: Option<(String, xenify_wp_doc::Document)>,
    /// The visible area's size last frame (for fit-to-width zoom).
    pub view_size: Vec2,
    /// A picture selected by clicking it.
    pub selected_image: Option<xenify_wp_doc::NodeId>,
    /// A frame selected by clicking its edge.
    pub selected_frame: Option<xenify_wp_doc::NodeId>,
    frame_drag: Option<FrameDrag>,
    image_drag: Option<ImageDrag>,
    col_drag: Option<ColDrag>,
    /// Linking a thread: the next frame clicked continues this one's text.
    pub linking: Option<xenify_wp_doc::NodeId>,
    /// Right-clicked here: (position in the text, screen point).
    pub context_click: Option<(Pos, Pos2)>,
    /// Where it was, in document space.
    pub context_doc: kurbo::Point,
    /// A click in a form's drop-down list (the app opens its choices there).
    pub control_click: Option<(Pos, Pos2)>,
    /// A double-click on an equation (the app opens the editor on it).
    pub equation_click: Option<Pos>,
    /// The document area's response last frame (its context menu).
    pub response: Option<egui::Response>,
    /// The visible part of the document area on screen, last frame.
    pub visible: Rect,
    pub images: crate::images::ImageCache,
    /// The document's folder (pictures kept next to it).
    pub base_dir: Option<std::path::PathBuf>,
    /// Showing something that can't be edited (a mail merge preview).
    pub read_only: bool,
    /// Space around the page, in screen points (0 for a text box that fills
    /// its area, as in Xenify Mail).
    pub gap: f32,
    /// Links are handed to the app instead of opening (Xenify Mail checks
    /// them first): a click on one (no Ctrl needed when read-only) sets
    /// `clicked_link`, and `hovered_link` is the link under the pointer.
    pub intercept_links: bool,
    pub clicked_link: Option<String>,
    pub hovered_link: Option<String>,
    /// Lines as screen readers get them.
    a11y: crate::a11y::Cache,
}

impl DocView {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            engine: Engine::new(fonts),
            layout: DocLayout::default(),
            laid_out_revision: None,
            canvas: Canvas::default(),
            rendered: None,
            zoom: 1.0,
            upstream: false,
            goal_x: None,
            preedit: None,
            drag: None,
            last_press: None,
            hf_edit: None,
            find_marks: Vec::new(),
            find_current: None,
            extra: Vec::new(),
            pending_fmt: None,
            autoformat: Default::default(),
            autocorrect: Default::default(),
            blink_from: Instant::now(),
            reveal_caret: true,
            reveal_top: false,
            // Each view its own (split panes and windows show several at once).
            id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                egui::Id::new(("xenify-write-document", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
            },
            doc_origin: Pos2::ZERO,
            clip: None,
            view_size: Vec2::new(800.0, 600.0),
            selected_image: None,
            context_click: None,
            context_doc: kurbo::Point::ZERO,
            control_click: None,
            equation_click: None,
            response: None,
            visible: Rect::NOTHING,
            selected_frame: None,
            frame_drag: None,
            image_drag: None,
            col_drag: None,
            linking: None,
            images: Default::default(),
            base_dir: None,
            read_only: false,
            gap: GAP,
            intercept_links: false,
            clicked_link: None,
            hovered_link: None,
            a11y: Default::default(),
        }
    }

    /// The pointer shape over document point `p` (frame edges and handles).
    fn frame_cursor(&self, p: kurbo::Point, s: f32) -> CursorIcon {
        let slop = 5.0 / s as f64;
        if let Some(f) = self.selected_frame.and_then(|id| self.layout.frames.iter().find(|f| f.id == id)) {
            for (hx, hy) in HANDLES {
                let c = kurbo::Point::new(f.rect.x0 + f.rect.width() * hx, f.rect.y0 + f.rect.height() * hy);
                if (c - p).hypot() <= slop * 1.6 {
                    return match (hx, hy) {
                        (0.5, _) => CursorIcon::ResizeVertical,
                        (_, 0.5) => CursorIcon::ResizeHorizontal,
                        (a, b) if a == b => CursorIcon::ResizeNwSe,
                        _ => CursorIcon::ResizeNeSw,
                    };
                }
            }
        }
        if self.layout.frame_edge_at(p, slop).is_some() { CursorIcon::Move } else { CursorIcon::Text }
    }

    /// Zoom that fits the widest page (or a whole page) in the window.
    pub fn fit(&mut self, whole_page: bool) {
        let Some(page) = self.layout.pages.first() else { return };
        let w = (self.view_size.x - 2.0 * self.gap) / (self.layout.width * PT);
        let h = (self.view_size.y - 2.0 * self.gap) / (page.rect.height() as f32 * PT);
        let z = if whole_page { w.min(h) } else { w };
        self.zoom = (z * 100.0).floor().clamp(25.0, 400.0) / 100.0;
    }

    /// The page the caret is on (index into the layout's pages).
    pub fn caret_page(&self, ed: &Editor) -> Option<usize> {
        let c = self.layout.caret(ed.sel.focus, self.upstream)?;
        self.layout.pages.iter().position(|p| p.rect.contains(c.center()))
    }

    /// Forgets everything about the previous document (after New / Open).
    pub fn reset(&mut self) {
        self.laid_out_revision = None;
        self.rendered = None;
        self.goal_x = None;
        self.preedit = None;
        self.pending_fmt = None;
        self.reveal_caret = true;
        self.selected_image = None;
        self.hf_edit = None;
        self.images.clear();
    }

    pub fn scale(&self) -> f32 {
        self.zoom * PT
    }

    /// Lays the document out again if it changed.
    pub fn relayout(&mut self, ed: &Editor) {
        if self.engine.header_edit != self.hf_edit {
            self.engine.header_edit = self.hf_edit;
            self.laid_out_revision = None;
        }
        if self.laid_out_revision != Some(ed.doc.revision) {
            self.layout = self.engine.layout(&ed.doc);
            self.images.prepare(&self.layout, &ed.doc, self.base_dir.as_deref());
            self.laid_out_revision = Some(ed.doc.revision);
            self.rendered = None;
        }
    }

    /// Stops editing headers and footers; the caret goes back to the text.
    pub fn leave_header(&mut self, ed: &mut Editor) {
        if self.hf_edit.take().is_some() {
            if xenify_wp_doc::hf::in_header(&ed.doc, ed.sel.focus.para)
                && let Some(p) = ed.doc.sections.first().and_then(|s| {
                    s.blocks.iter().find_map(|b| match b {
                        xenify_wp_doc::Block::Paragraph(p) => Some(p.id),
                        _ => None,
                    })
                })
            {
                ed.sel = Selection::caret(Pos::new(p, 0));
            }
            self.touched();
        }
    }

    /// The caret moved or the text changed: show the caret and keep it on screen.
    fn touched(&mut self) {
        self.blink_from = Instant::now();
        self.reveal_caret = true;
    }

    /// The document version last laid out.
    pub fn layout_revision(&self) -> u64 {
        self.laid_out_revision.unwrap_or(0)
    }

    /// Makes the view scroll to the caret next frame.
    pub fn reveal(&mut self) {
        self.reveal_caret = true;
        self.touched();
    }

    /// Makes the view scroll the caret's line to the top next frame.
    pub fn reveal_at_top(&mut self) {
        self.reveal_top = true;
        self.reveal();
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ed: &mut Editor,
        frame: Option<&eframe::Frame>,
        issues: &[xenify_wp_check::Issue],
        ai: &[(xenify_wp_doc::NodeId, std::ops::Range<usize>)],
    ) {
        self.relayout(ed);
        let s = self.scale();
        // Document space: the pages stacked top to bottom.
        let sheet = Vec2::new(self.layout.width * s, self.layout.height * s);
        let avail = ui.available_size();
        self.view_size = avail;
        let gap = self.gap;
        let content = Vec2::new(avail.x.max(sheet.x + 2.0 * gap), avail.y.max(sheet.y + 2.0 * gap));

        egui::ScrollArea::both().auto_shrink([false, false]).id_salt("doc-scroll").show_viewport(ui, |ui, _vp| {
            let (rect, _) = ui.allocate_exact_size(content, Sense::hover());
            let resp = ui.interact(rect, self.id, Sense::click_and_drag());
            self.response = Some(resp.clone());
            let sheet_rect = Rect::from_min_size(
                Pos2::new((rect.center().x - sheet.x / 2.0).round(), (rect.min.y + gap).round()),
                sheet,
            );
            let origin = sheet_rect.min;
            self.doc_origin = origin;
            let to_screen = |x: f64, y: f64| origin + Vec2::new(x as f32, y as f32) * s;
            let to_doc = |p: Pos2| {
                let d = (p - origin) / s;
                kurbo::Point::new(d.x as f64, d.y as f64)
            };

            // Input first, so this frame shows its result.
            let hover_icon = resp.hover_pos().map(|p| self.frame_cursor(to_doc(p), s)).unwrap_or(CursorIcon::Text);
            if self.intercept_links {
                self.hovered_link =
                    resp.hover_pos().and_then(|p| self.layout.hit(to_doc(p))).and_then(|pos| link_at(&ed.doc, pos));
            }
            let hover_icon = if self.hovered_link.is_some() && self.read_only { CursorIcon::PointingHand } else { hover_icon };
            let resp = resp.on_hover_cursor(if self.linking.is_some() { CursorIcon::Alias } else { hover_icon });
            if !self.read_only && self.table_input(ui, &resp, ed, origin, s) {
                // A table handle took the click.
            } else {
                self.pointer(ui, &resp, ed, to_doc);
            }
            if resp.has_focus() {
                self.keyboard(ui, ed);
                self.relayout(ed);
            }

            let clip = ui.clip_rect().intersect(rect);
            self.visible = clip;
            self.paint(ui, frame, clip, sheet_rect, origin);

            // Selection, caret and IME composition on top of the page.
            let painter = ui.painter_at(clip);
            let to_rect = |r: kurbo::Rect| Rect::from_min_max(to_screen(r.x0, r.y0), to_screen(r.x1, r.y1));
            // Squiggles under proofing issues.
            let visible_doc = kurbo::Rect::new(
                ((clip.min.x - origin.x) / s) as f64,
                ((clip.min.y - origin.y) / s) as f64,
                ((clip.max.x - origin.x) / s) as f64,
                ((clip.max.y - origin.y) / s) as f64,
            );
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Document"));
            let revision = self.layout_revision();
            crate::a11y::publish(
                ui,
                self.id,
                &mut self.a11y,
                revision,
                &self.layout,
                &ed.doc,
                ed.sel,
                self.upstream,
                self.read_only,
                &crate::a11y::Placement { origin, scale: s, visible: visible_doc },
            );
            let cv = crate::theme::canvas(ui.visuals());
            let marks = issues
                .iter()
                .map(|i| (i.para, i.range.clone(), crate::proofing::color(i.category)))
                .chain(ai.iter().map(|(p, r)| (*p, r.clone(), crate::assistant::AI_COLOR)));
            for (para, range, color) in marks {
                for r in self.layout.range_rects(para, range) {
                    if !r.overlaps(visible_doc) {
                        continue;
                    }
                    let r = to_rect(r);
                    squiggle(&painter, r.left_bottom().x, r.right_bottom().x, r.bottom() - 1.0, color);
                }
            }
            // Find: every match lit, the current one more strongly.
            for (i, (para, range)) in self.find_marks.iter().enumerate() {
                let color = if Some(i) == self.find_current {
                    Color32::from_rgba_unmultiplied(255, 150, 0, 140)
                } else {
                    Color32::from_rgba_unmultiplied(255, 220, 0, 110)
                };
                for r in self.layout.range_rects(*para, range.clone()) {
                    if r.overlaps(visible_doc) {
                        painter.rect_filled(to_rect(r), 1.0, color);
                    }
                }
            }
            for s in self.extra.iter().chain([&ed.sel]) {
                for r in self.layout.selection_rects(&ed.doc, *s) {
                    painter.rect_filled(to_rect(r), 0.0, cv.selection);
                }
            }
            // Frames: a faint outline (not printed); the selected one with handles.
            let selected_chain = self.selected_frame.map(|id| chain_of(&ed.doc, id)).unwrap_or_default();
            for f in &self.layout.frames {
                let r = to_rect(f.rect);
                let bordered = ed.doc.frame(f.id).is_some_and(|x| x.border.is_some());
                if Some(f.id) == self.selected_frame {
                    painter.rect_stroke(r, 0.0, Stroke::new(1.5, cv.accent), egui::StrokeKind::Outside);
                    for (hx, hy) in HANDLES {
                        let c = Pos2::new(r.min.x + r.width() * hx as f32, r.min.y + r.height() * hy as f32);
                        painter.rect_filled(Rect::from_center_size(c, Vec2::splat(7.0)), 1.0, Color32::WHITE);
                        painter.rect_stroke(
                            Rect::from_center_size(c, Vec2::splat(7.0)),
                            1.0,
                            Stroke::new(1.0, cv.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                } else if !bordered {
                    dashed_rect(&painter, r, Stroke::new(1.0, cv.guide));
                }
                if f.overflow {
                    // Too much text: a red + in the corner (link a frame to continue it).
                    let m = Rect::from_center_size(r.right_bottom(), Vec2::splat(11.0));
                    painter.rect_filled(m, 1.0, Color32::WHITE);
                    painter.rect_stroke(m, 1.0, Stroke::new(1.0, cv.warning), egui::StrokeKind::Inside);
                    painter.text(
                        m.center(),
                        egui::Align2::CENTER_CENTER,
                        "+",
                        egui::FontId::proportional(11.0),
                        cv.warning,
                    );
                }
            }
            // The selected frame's thread: lines from each frame to the next.
            for w in selected_chain.windows(2) {
                let (a, b) = (
                    self.layout.frames.iter().find(|f| f.id == w[0]),
                    self.layout.frames.iter().find(|f| f.id == w[1]),
                );
                if let (Some(a), Some(b)) = (a, b) {
                    let (p0, p1) = (to_rect(a.rect).right_bottom(), to_rect(b.rect).left_top());
                    painter.line_segment([p0, p1], Stroke::new(1.5, cv.accent));
                    painter.circle_filled(p1, 3.5, cv.accent);
                }
            }
            if let Some(idr) = self.image_drag
                && let Some(img) = self.layout.images.iter().find(|i| i.id == idr.id)
            {
                // The new size, anchored at the opposite corner.
                let r = img.rect;
                let (w, h) = (idr.size.0 as f64, idr.size.1 as f64);
                let x0 = if idr.corner.0 == 1.0 { r.x0 } else { r.x1 - w };
                let y0 = if idr.corner.1 == 1.0 { r.y0 } else { r.y1 - h };
                dashed_rect(&painter, to_rect(kurbo::Rect::new(x0, y0, x0 + w, y0 + h)), Stroke::new(1.5, cv.accent));
            }
            if let Some(img) = self.selected_image.and_then(|id| self.layout.images.iter().find(|i| i.id == id)) {
                let r = to_rect(img.rect);
                painter.rect_stroke(r, 0.0, Stroke::new(2.0, cv.accent), egui::StrokeKind::Outside);
                for corner in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                    painter.rect_filled(Rect::from_center_size(corner, Vec2::splat(7.0)), 1.0, cv.accent);
                }
            }
            if !self.read_only {
                self.table_paint(ui, &painter, ed, origin, s);
            }
            // Editing a header or footer: the text fades; the margins are labelled.
            if let Some(pi) = self.hf_edit {
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.leave_header(ed);
                } else {
                    for page in &self.layout.pages {
                        painter.rect_filled(to_rect(page.body), 0.0, egui::Color32::from_white_alpha(150));
                    }
                    if let Some(page) = self.layout.pages.get(pi) {
                        let accent = crate::theme::accent(ui.visuals());
                        for (y, label, below) in [(page.body.y0, "Header", false), (page.body.y1, "Footer", true)] {
                            let a = to_screen(page.body.x0, y);
                            let b = to_screen(page.body.x1, y);
                            let mut x = a.x;
                            while x < b.x {
                                painter.line_segment(
                                    [Pos2::new(x, a.y), Pos2::new((x + 5.0).min(b.x), a.y)],
                                    Stroke::new(1.0, accent),
                                );
                                x += 9.0;
                            }
                            let at = if below { Pos2::new(a.x, a.y + 2.0) } else { Pos2::new(a.x, a.y - 2.0) };
                            let align = if below { egui::Align2::LEFT_TOP } else { egui::Align2::LEFT_BOTTOM };
                            let galley = painter.layout_no_wrap(
                                label.into(),
                                egui::FontId::proportional(11.0),
                                egui::Color32::WHITE,
                            );
                            let r = align.anchor_size(at, galley.size() + egui::vec2(10.0, 4.0));
                            painter.rect_filled(r, 3.0, accent);
                            painter.galley(r.min + egui::vec2(5.0, 2.0), galley, egui::Color32::WHITE);
                        }
                    }
                }
            }
            let caret = self.layout.caret(ed.sel.focus, self.upstream).map(to_rect);
            if let Some(c) = caret {
                let mut c = Rect::from_min_max(c.min, Pos2::new(c.min.x + 1.5 * self.zoom.max(0.75), c.max.y));
                if let Some(pre) = &self.preedit {
                    let galley = painter.layout_no_wrap(
                        pre.clone(),
                        egui::FontId::proportional(c.height() * 0.7),
                        Color32::from_rgb(0x1a, 0x5f, 0xd0),
                    );
                    let at = Pos2::new(c.min.x, c.center().y - galley.size().y / 2.0);
                    let r = Rect::from_min_size(at, galley.size());
                    painter.rect_filled(r.expand(1.0), 2.0, Color32::WHITE);
                    painter.galley(at, galley, Color32::BLACK);
                    painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, Color32::BLACK));
                    c = c.translate(Vec2::new(r.width(), 0.0));
                }
                let focused = resp.has_focus();
                // Reduce motion: a steady caret.
                let steady = crate::theme::reduce_motion(ui.ctx());
                let on = steady || (self.blink_from.elapsed().as_millis() / BLINK.as_millis()).is_multiple_of(2);
                if focused && (on || self.preedit.is_some()) {
                    painter.rect_filled(c, 0.0, cv.caret);
                    for s in &self.extra {
                        if let Some(e) = self.layout.caret(s.focus, false).map(to_rect) {
                            painter.rect_filled(
                                Rect::from_min_size(e.min, Vec2::new(c.width(), e.height())),
                                0.0,
                                cv.caret,
                            );
                        }
                    }
                }
                if focused {
                    if !steady {
                        let left = BLINK
                            - Duration::from_millis((self.blink_from.elapsed().as_millis() % BLINK.as_millis()) as u64);
                        ui.ctx().request_repaint_after(left);
                    }
                    ui.output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            rect: clip,
                            cursor_rect: c,
                            should_interrupt_composition: false,
                        })
                    });
                }
                if self.reveal_caret {
                    self.reveal_caret = false;
                    if std::mem::take(&mut self.reveal_top) {
                        let r = Rect::from_min_max(c.min - Vec2::new(24.0, 48.0), c.max + Vec2::new(24.0, 0.0));
                        ui.scroll_to_rect(r, Some(egui::Align::TOP));
                    } else {
                        ui.scroll_to_rect(c.expand2(Vec2::new(24.0, 36.0)), None);
                    }
                }
            }
        });
    }

    fn paint(&mut self, ui: &egui::Ui, frame: Option<&eframe::Frame>, clip: Rect, sheet: Rect, origin: Pos2) {
        let painter = ui.painter_at(clip);
        let desk = crate::theme::canvas(ui.visuals()).desk;
        painter.rect_filled(clip, 0.0, desk);
        let Some(rs) = frame.and_then(|f| f.wgpu_render_state()) else {
            // No GPU (headless tests): just the sheets.
            let s = self.scale();
            for p in &self.layout.pages {
                let r = Rect::from_min_max(
                    origin + Vec2::new(p.rect.x0 as f32, p.rect.y0 as f32) * s,
                    origin + Vec2::new(p.rect.x1 as f32, p.rect.y1 as f32) * s,
                );
                painter.rect_filled(r, 0.0, Color32::WHITE);
            }
            return;
        };
        let ppp = ui.ctx().pixels_per_point();
        // Whole physical pixels, so the texture isn't resampled (blurry text).
        let snap = |p: Pos2| Pos2::new((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp);
        let vis = sheet.intersect(clip);
        let vis = Rect::from_min_max(snap(vis.min), snap(vis.max));
        if !vis.is_positive() {
            return;
        }
        let px = ((vis.width() * ppp).round() as u32, (vis.height() * ppp).round() as u32);
        // The whole rect: when only its size changes (window resized, dock
        // opened) the old texture would be stretched over the new one.
        let key_rect =
            [vis.min.x, vis.min.y, vis.max.x, vis.max.y, origin.x, origin.y].map(|v| (v * 4.0).round() as i32);
        let key = (self.laid_out_revision.unwrap_or(0), key_rect, self.zoom.to_bits(), ppp.to_bits(), desk.to_array());
        let texture = if self.rendered == Some(key) {
            self.canvas.texture()
        } else {
            let s = self.scale() as f64;
            let t = kurbo::Affine::scale(ppp as f64)
                * kurbo::Affine::translate(((origin.x - vis.min.x) as f64, (origin.y - vis.min.y) as f64))
                * kurbo::Affine::scale(s);
            let visible = kurbo::Rect::new(
                ((vis.min.x - origin.x) / s as f32) as f64,
                ((vis.min.y - origin.y) / s as f32) as f64,
                ((vis.max.x - origin.x) / s as f32) as f64,
                ((vis.max.y - origin.y) / s as f32) as f64,
            );
            let layout = &self.layout;
            let images = &self.images;
            let [r, g, b, _] = desk.to_array();
            match self.canvas.render(rs, px, vello::peniko::Color::from_rgb8(r, g, b), |scene| {
                xenify_wp_layout::paint::paint_pages(scene, layout, t, visible);
                xenify_wp_layout::paint::paint_with_images(scene, layout, t, visible, &|src| images.lookup(src));
            }) {
                Ok(id) => {
                    self.rendered = Some(key);
                    Some(id)
                }
                Err(e) => {
                    painter.text(vis.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::default(), Color32::RED);
                    None
                }
            }
        };
        if let Some(id) = texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(id, vis, uv, Color32::WHITE);
        }
    }

    /// The table under doc point `at` (or the caret's), as placed on that page.
    fn table_grid(&self, ed: &Editor, at: Option<kurbo::Point>) -> Option<TableGrid> {
        let hovered = at
            .and_then(|p| self.layout.cells.iter().rev().find(|c| !c.repeat && c.rect.inflate(18.0, 18.0).contains(p)));
        let (id, page_y) = match hovered {
            Some(c) => (c.table, c.rect.center().y),
            None => {
                let (id, _, _) = xenify_wp_doc::tables::cell_at(ed, ed.sel.focus.para)?;
                let caret = self.layout.caret(ed.sel.focus, false)?;
                (id, caret.center().y)
            }
        };
        let page = self.layout.pages.iter().find(|p| p.rect.y0 <= page_y && page_y <= p.rect.y1)?.rect;
        let cells: Vec<&xenify_wp_layout::PlacedCell> =
            self.layout.cells.iter().filter(|c| c.table == id && !c.repeat && page.contains(c.rect.center())).collect();
        if cells.is_empty() {
            return None;
        }
        let ncols = cells.iter().map(|c| c.col + c.col_span).max().unwrap_or(1);
        let mut cols = vec![f64::NAN; ncols + 1];
        for c in &cells {
            if cols[c.col].is_nan() || c.rect.x0 < cols[c.col] {
                cols[c.col] = c.rect.x0;
            }
            let e = c.col + c.col_span;
            if cols[e].is_nan() || c.rect.x1 > cols[e] {
                cols[e] = c.rect.x1;
            }
        }
        if cols.iter().any(|x| x.is_nan()) {
            return None;
        }
        let mut rows: Vec<(usize, f64, f64)> = Vec::new();
        for c in &cells {
            match rows.iter_mut().find(|r| r.0 == c.row) {
                Some(r) => {
                    r.1 = r.1.min(c.rect.y0);
                    if c.row_span == 1 {
                        r.2 = r.2.max(c.rect.y1);
                    }
                }
                None => rows.push((c.row, c.rect.y0, if c.row_span == 1 { c.rect.y1 } else { c.rect.y0 })),
            }
        }
        rows.sort_by_key(|r| r.0);
        // Rows whose every cell spans further: end where the next row starts.
        for k in 0..rows.len() {
            if rows[k].2 <= rows[k].1 {
                rows[k].2 = rows.get(k + 1).map_or(rows[k].1 + 12.0, |n| n.1);
            }
        }
        let bbox = cells.iter().map(|c| c.rect).reduce(|a, b| a.union(b))?;
        Some(TableGrid { id, cols, rows, bbox })
    }

    /// Handles, insert buttons and column resizing for the table under the
    /// pointer. True when it used the click.
    fn table_input(&mut self, ui: &egui::Ui, resp: &egui::Response, ed: &mut Editor, origin: Pos2, s: f32) -> bool {
        use xenify_wp_doc::tables;
        let to_doc = |p: Pos2| kurbo::Point::new(((p.x - origin.x) / s) as f64, ((p.y - origin.y) / s) as f64);
        let (pressed, down, pointer) = ui.input(|i| {
            (i.pointer.button_pressed(egui::PointerButton::Primary), i.pointer.primary_down(), i.pointer.hover_pos())
        });
        // An ongoing column resize.
        if let Some(cd) = self.col_drag.clone() {
            if down && let Some(p) = pointer {
                let x = to_doc(p).x;
                let (lo, hi) = (cd.edges[cd.col - 1] + 12.0, cd.edges[cd.col + 1] - 12.0);
                let x = x.clamp(lo, hi.max(lo));
                let pt: Vec<f64> = cd.edges.windows(2).map(|w| w[1] - w[0]).collect();
                let rel_total: f32 = cd.widths.iter().sum();
                let pt_total: f64 = pt.iter().sum();
                let mut new_pt = pt.clone();
                new_pt[cd.col - 1] = x - cd.edges[cd.col - 1];
                new_pt[cd.col] = cd.edges[cd.col + 1] - x;
                let widths: Vec<f32> = new_pt.iter().map(|w| (*w / pt_total) as f32 * rel_total).collect();
                let _ = tables::set_widths(ed, cd.table, widths);
                ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
                self.touched();
            } else {
                self.col_drag = None;
                ed.seal();
            }
            return true;
        }
        let Some(p) = pointer.filter(|p| resp.rect.contains(*p)) else { return false };
        let dp = to_doc(p);
        let Some(g) = self.table_grid(ed, Some(dp)) else { return false };
        let px = 1.0 / s as f64;
        let top = g.bbox.y0;
        let left = g.bbox.x0;
        // Column edges inside the table: drag to resize.
        if dp.y >= top && dp.y <= g.bbox.y1 {
            for c in 1..g.cols.len() - 1 {
                if (dp.x - g.cols[c]).abs() <= 3.0 * px {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
                    if pressed && let Some(widths) = ed.doc.table(g.id).map(|t| t.widths.clone()) {
                        ed.seal();
                        self.col_drag = Some(ColDrag { table: g.id, col: c, edges: g.cols.clone(), widths });
                        return true;
                    }
                }
            }
        }
        // The handle strips above and left of the table.
        let in_col_strip =
            dp.y >= top - 16.0 * px && dp.y < top - 2.0 * px && dp.x >= left - 4.0 * px && dp.x <= g.bbox.x1;
        let in_row_strip = dp.x >= left - 16.0 * px && dp.x < left - 2.0 * px && dp.y >= top && dp.y <= g.bbox.y1;
        if in_col_strip {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            if pressed {
                // Near an edge: insert a column there; else select the column.
                if let Some(c) = (0..g.cols.len()).find(|&c| (dp.x - g.cols[c]).abs() <= 6.0 * px) {
                    let _ = tables::insert_column_at(ed, g.id, c);
                } else if let Some(c) = (0..g.cols.len() - 1).find(|&c| dp.x > g.cols[c] && dp.x < g.cols[c + 1]) {
                    tables::select_column(ed, g.id, c);
                }
                self.touched();
                resp.request_focus();
                return true;
            }
        }
        if in_row_strip {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            if pressed {
                let edge = g.rows.iter().enumerate().find_map(|(k, r)| {
                    if (dp.y - r.1).abs() <= 6.0 * px {
                        Some(r.0)
                    } else if k + 1 == g.rows.len() && (dp.y - r.2).abs() <= 6.0 * px {
                        Some(r.0 + 1)
                    } else {
                        None
                    }
                });
                if let Some(at) = edge {
                    let _ = tables::insert_row_at(ed, g.id, at);
                } else if let Some(r) = g.rows.iter().find(|r| dp.y > r.1 && dp.y < r.2) {
                    tables::select_row(ed, g.id, r.0);
                }
                self.touched();
                resp.request_focus();
                return true;
            }
        }
        false
    }

    /// Draws the table handles, the "+" insert buttons under the pointer and
    /// the selected cells.
    fn table_paint(&self, ui: &egui::Ui, painter: &egui::Painter, ed: &Editor, origin: Pos2, s: f32) {
        let to_screen = |x: f64, y: f64| origin + Vec2::new(x as f32, y as f32) * s;
        let cv = crate::theme::canvas(ui.visuals());
        // Selected cells (when the selection crosses cells).
        if let Some((id, rows, cols)) = xenify_wp_doc::tables::selected_cells(ed)
            && (rows.start() != rows.end() || cols.start() != cols.end())
        {
            for c in
                self.layout.cells.iter().filter(|c| c.table == id && rows.contains(&c.row) && cols.contains(&c.col))
            {
                let r = Rect::from_min_max(to_screen(c.rect.x0, c.rect.y0), to_screen(c.rect.x1, c.rect.y1));
                painter.rect_filled(r, 0.0, cv.selection);
            }
        }
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let at = pointer.map(|p| kurbo::Point::new(((p.x - origin.x) / s) as f64, ((p.y - origin.y) / s) as f64));
        let caret_in = xenify_wp_doc::tables::cell_at(ed, ed.sel.focus.para).map(|c| c.0);
        let Some(g) = self.table_grid(ed, at) else { return };
        let near = at.is_some_and(|p| g.bbox.inflate(24.0, 24.0).contains(p));
        if !near && caret_in != Some(g.id) && self.col_drag.is_none() {
            return;
        }
        let selected = xenify_wp_doc::tables::selected_cells(ed).filter(|s| s.0 == g.id);
        let handle = |on: bool, hover: bool| {
            if on {
                cv.accent
            } else if hover {
                cv.accent.gamma_multiply(0.6)
            } else {
                Color32::from_gray(if ui.visuals().dark_mode { 120 } else { 200 })
            }
        };
        let (top, left) = (g.bbox.y0, g.bbox.x0);
        let hp = |p: Rect| pointer.is_some_and(|q| p.contains(q));
        for c in 0..g.cols.len() - 1 {
            let r = Rect::from_min_max(to_screen(g.cols[c], top), to_screen(g.cols[c + 1], top));
            let r = Rect::from_min_max(
                Pos2::new(r.left() + 2.0, r.top() - 11.0),
                Pos2::new(r.right() - 2.0, r.top() - 5.0),
            );
            let on = selected.as_ref().is_some_and(|(_, rows, cols)| {
                cols.contains(&c) && *rows.start() == 0 && *rows.end() + 1 >= g.rows.len()
            });
            painter.rect_filled(r, 3.0, handle(on, hp(r.expand(3.0))));
        }
        for (row, y0, y1) in &g.rows {
            let a = to_screen(left, *y0);
            let b = to_screen(left, *y1);
            let r = Rect::from_min_max(Pos2::new(a.x - 11.0, a.y + 2.0), Pos2::new(a.x - 5.0, b.y - 2.0));
            let on = selected.as_ref().is_some_and(|(_, rows, cols)| {
                rows.contains(row) && *cols.start() == 0 && *cols.end() + 1 >= g.cols.len() - 1
            });
            painter.rect_filled(r, 3.0, handle(on, hp(r.expand(3.0))));
        }
        // "+" where the pointer is near an edge in a strip.
        let Some(p) = pointer else { return };
        let plus = |at: Pos2| {
            painter.circle_filled(at, 7.0, cv.accent);
            painter
                .line_segment([at - Vec2::new(3.5, 0.0), at + Vec2::new(3.5, 0.0)], Stroke::new(1.6, Color32::WHITE));
            painter
                .line_segment([at - Vec2::new(0.0, 3.5), at + Vec2::new(0.0, 3.5)], Stroke::new(1.6, Color32::WHITE));
        };
        let t = to_screen(left, top);
        if p.y >= t.y - 18.0 && p.y < t.y - 2.0 {
            for &x in &g.cols {
                let at = to_screen(x, top);
                if (p.x - at.x).abs() <= 7.0 {
                    plus(Pos2::new(at.x, at.y - 8.0));
                    painter.line_segment([at, to_screen(x, g.bbox.y1)], Stroke::new(1.5, cv.accent));
                }
            }
        }
        if p.x >= t.x - 18.0 && p.x < t.x - 2.0 {
            let mut ys: Vec<f64> = g.rows.iter().map(|r| r.1).collect();
            ys.extend(g.rows.last().map(|r| r.2));
            for y in ys {
                let at = to_screen(left, y);
                if (p.y - at.y).abs() <= 7.0 {
                    plus(Pos2::new(at.x - 8.0, at.y));
                    painter.line_segment([at, to_screen(g.bbox.x1, y)], Stroke::new(1.5, cv.accent));
                }
            }
        }
    }

    fn pointer(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        ed: &mut Editor,
        to_doc: impl Fn(Pos2) -> kurbo::Point,
    ) {
        let (pressed, down, pointer, shift, command) = ui.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
                i.modifiers.command,
            )
        });
        let Some(p) = pointer else { return };
        if resp.secondary_clicked()
            && let Some(pos) = self.layout.hit(to_doc(p))
        {
            self.context_click = Some((pos, p));
            self.context_doc = to_doc(p);
        }
        if pressed && resp.hovered() {
            resp.request_focus();
            let dp = to_doc(p);
            let slop = 5.0 / self.scale() as f64;
            // Linking a thread: the frame clicked continues the text.
            if let Some(src) = self.linking.take() {
                if let Some(target) = self.layout.frame_at(dp).map(|f| f.id).filter(|t| *t != src) {
                    link_frames(ed, src, target);
                }
                self.touched();
                return;
            }
            // The selected frame's handles, or any frame's edge: move/resize it.
            let handle = self.selected_frame.and_then(|id| {
                let f = self.layout.frames.iter().find(|f| f.id == id)?;
                HANDLES
                    .iter()
                    .find(|(hx, hy)| {
                        let c = kurbo::Point::new(f.rect.x0 + f.rect.width() * hx, f.rect.y0 + f.rect.height() * hy);
                        (c - dp).hypot() <= slop * 1.6
                    })
                    .map(|h| (id, Some(*h)))
            });
            let grab = handle.or_else(|| self.layout.frame_edge_at(dp, slop).map(|f| (f.id, None)));
            if let Some((id, handle)) = grab
                && let Some(orig) = ed.doc.frame(id).map(|fr| (fr.x, fr.y, fr.width, fr.height))
            {
                ed.seal();
                self.selected_frame = Some(id);
                self.selected_image = None;
                self.frame_drag = Some(FrameDrag { id, handle, start: dp, orig });
                self.drag = None;
                self.touched();
                return;
            }
            self.selected_frame = None;
            // The selected picture's corners: resize it.
            if let Some(img) = self.selected_image.and_then(|id| self.layout.images.iter().find(|i| i.id == id)) {
                let r = img.rect;
                let corner = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].into_iter().find(|(cx, cy)| {
                    (kurbo::Point::new(r.x0 + r.width() * cx, r.y0 + r.height() * cy) - dp).hypot() <= slop * 1.8
                });
                if let Some(corner) = corner {
                    let orig = (r.width() as f32, r.height() as f32);
                    self.image_drag = Some(ImageDrag { id: img.id, corner, start: dp, orig, size: orig });
                    self.drag = None;
                    return;
                }
            }
            // A picture: select it.
            if let Some(img) = self.layout.image_at(dp) {
                self.selected_image = Some(img.id);
                self.drag = None;
                self.touched();
                return;
            }
            self.selected_image = None;
            // A contents entry: go to its heading.
            if let Some(heading) = self.layout.link_at(dp) {
                ed.sel = Selection::caret(Pos::new(heading, 0));
                self.drag = None;
                self.touched();
                return;
            }
            // Headers and footers: a double-click in the top or bottom margin edits
            // them; a click in the text goes back to it.
            let quick = matches!(self.last_press, Some((t, at, _)) if t.elapsed() < Duration::from_millis(450) && at.distance(p) < 6.0);
            let margin = self.layout.hf_areas.iter().find(|(_, _, r)| r.contains(dp)).copied();
            match margin {
                Some((page, footer, _)) if quick && self.hf_edit != Some(page) && !self.read_only => {
                    let pg = self.layout.pages[page].clone();
                    if xenify_wp_doc::hf::edit_on_page(ed, pg.section, pg.number, pg.first_of_section, footer).is_ok() {
                        self.hf_edit = Some(page);
                        self.relayout(ed);
                    }
                    self.last_press = None;
                    self.drag = None;
                    self.touched();
                    return;
                }
                None if self.hf_edit.is_some() => self.leave_header(ed),
                _ => {}
            }
            let Some(pos) = self.layout.hit(dp) else { return };
            // Ctrl+click on a link: a web link opens in the browser; a link
            // inside the document (#p12) goes there.
            if (command || (self.read_only && self.intercept_links))
                && let Some(url) = link_at(&ed.doc, pos)
            {
                match url.strip_prefix("#p").and_then(|n| n.parse::<u64>().ok()) {
                    _ if self.intercept_links && !url.starts_with('#') => self.clicked_link = Some(url),
                    Some(id) if ed.doc.para(xenify_wp_doc::NodeId(id)).is_some() => {
                        ed.sel = Selection::caret(Pos::new(xenify_wp_doc::NodeId(id), 0));
                        self.reveal();
                    }
                    _ if !url.starts_with('#') => ui.ctx().open_url(egui::OpenUrl::new_tab(url)),
                    _ => {}
                }
                self.drag = None;
                self.touched();
                return;
            }
            // A checklist box: tick or untick it.
            if self.on_checkbox(ed, pos, dp) {
                let _ = ops::toggle_checked(ed, pos.para);
                self.drag = None;
                self.touched();
                return;
            }
            // A form's checkbox: the same.
            if !shift && let Some(at) = self.form_check_at(ed, pos, dp) {
                let _ = xenify_wp_doc::controls::toggle_check(ed, at);
                self.drag = None;
                self.touched();
                return;
            }
            // A form's list: its choices open.
            if !shift
                && let Some((c, _)) = xenify_wp_doc::controls::control_at(ed, pos)
                && matches!(
                    c.kind,
                    xenify_wp_doc::controls::ControlKind::Dropdown { .. }
                        | xenify_wp_doc::controls::ControlKind::ComboBox { .. }
                )
            {
                self.control_click = Some((pos, p));
            }
            // Count clicks in quick succession at about the same place.
            let count = match self.last_press {
                Some((t, at, n)) if t.elapsed() < Duration::from_millis(450) && at.distance(p) < 6.0 => n % 3 + 1,
                _ => 1,
            };
            self.last_press = Some((Instant::now(), p, count));

            ed.seal();
            self.goal_x = None;
            self.upstream = false;
            self.pending_fmt = None;
            self.preedit = None;
            // Alt+click: another caret; any other click: back to one.
            let alt = ui.input(|i| i.modifiers.alt);
            if count == 1 && alt && !shift {
                self.extra.push(ed.sel);
                ed.sel = Selection::caret(pos);
                self.extra = xenify_wp_doc::multi::normalize(&ed.doc, &self.extra);
                self.extra.retain(|s| *s != ed.sel);
                self.drag = None;
                self.touched();
                return;
            }
            self.extra.clear();
            match count {
                1 if shift => ed.sel.focus = pos,
                1 => ed.sel = Selection::caret(pos),
                2 => {
                    ed.sel = ops::word_at(ed, pos);
                    // Double-click on an equation (it selects just it): its editor opens.
                    let (a, b) = ed.doc.ordered(ed.sel);
                    if a.para == b.para
                        && b.offset == a.offset + xenify_wp_doc::Field::CHAR.len_utf8()
                        && ops::equation_at(&ed.doc, a).is_some()
                    {
                        self.equation_click = Some(a);
                    }
                }
                _ => {
                    let len = ed.doc.para(pos.para).map_or(0, |q| q.text.len());
                    ed.sel = Selection { anchor: Pos::new(pos.para, 0), focus: Pos::new(pos.para, len) };
                }
            }
            self.drag = Some(match count {
                1 => DragBy::Char,
                2 => DragBy::Word(ed.sel),
                _ => DragBy::Paragraph(ed.sel),
            });
            self.touched();
        } else if down && let Some(mut idr) = self.image_drag {
            // Keep the picture's proportions; the corner follows the pointer.
            let d = to_doc(p) - idr.start;
            let dx = if idr.corner.0 == 1.0 { d.x } else { -d.x } as f32;
            let (w, h) = idr.orig;
            let nw = (w + dx).clamp(12.0, 2000.0);
            idr.size = (nw, nw * h / w.max(1.0));
            self.image_drag = Some(idr);
            self.touched();
        } else if down && let Some(fd) = self.frame_drag {
            let d = to_doc(p) - fd.start;
            let (dx, dy) = (d.x as f32, d.y as f32);
            let (x, y, w, h) = fd.orig;
            let (nx, ny, nw, nh) = match fd.handle {
                None => (x + dx, y + dy, w, h),
                Some((hx, hy)) => {
                    let (mut nx, mut nw) = (x, w);
                    let (mut ny, mut nh) = (y, h);
                    if hx == 0.0 {
                        nx = x + dx.min(w - 12.0);
                        nw = w - dx.min(w - 12.0);
                    } else if hx == 1.0 {
                        nw = (w + dx).max(12.0);
                    }
                    if hy == 0.0 {
                        ny = y + dy.min(h - 12.0);
                        nh = h - dy.min(h - 12.0);
                    } else if hy == 1.0 {
                        nh = (h + dy).max(12.0);
                    }
                    (nx, ny, nw, nh)
                }
            };
            let label = if fd.handle.is_none() { "Move text box" } else { "Resize text box" };
            let _ = xenify_wp_doc::frames::update_frame(ed, fd.id, label, |f| {
                f.x = nx.round();
                f.y = ny.round();
                f.width = nw.round();
                f.height = nh.round();
            });
            self.touched();
        } else if down && let Some(by) = self.drag {
            let Some(pos) = self.layout.hit(to_doc(p)) else { return };
            match by {
                DragBy::Char => ed.sel.focus = pos,
                DragBy::Word(start) | DragBy::Paragraph(start) => {
                    let unit = if matches!(by, DragBy::Word(_)) {
                        ops::word_at(ed, pos)
                    } else {
                        let len = ed.doc.para(pos.para).map_or(0, |q| q.text.len());
                        Selection { anchor: Pos::new(pos.para, 0), focus: Pos::new(pos.para, len) }
                    };
                    let (s0, s1) = ed.doc.ordered(start);
                    let (u0, u1) = ed.doc.ordered(unit);
                    ed.sel = if ed.doc.cmp_pos(u0, s0).is_lt() {
                        Selection { anchor: s1, focus: u0 }
                    } else {
                        Selection { anchor: s0, focus: u1 }
                    };
                }
            }
            self.touched();
            // Scroll while dragging past the edges.
            let clip = ui.clip_rect();
            if !clip.contains(p) {
                ui.scroll_to_rect(Rect::from_center_size(p, Vec2::splat(4.0)), None);
            }
        }
        if !down {
            self.drag = None;
            if self.frame_drag.take().is_some() {
                ed.seal();
            }
            if let Some(idr) = self.image_drag.take()
                && idr.size != idr.orig
                && let Some(xenify_wp_doc::Block::Image(mut img)) = ed.doc.block(idr.id).cloned()
            {
                (img.width, img.height) = (idr.size.0.round(), idr.size.1.round());
                ed.seal();
                let _ = xenify_wp_doc::tables::replace_block(
                    ed,
                    idr.id,
                    xenify_wp_doc::Block::Image(img),
                    "Resize picture",
                );
                ed.seal();
            }
        }
    }

    fn keyboard(&mut self, ui: &egui::Ui, ed: &mut Editor) {
        // Extra carets the document no longer has room for (after an undo…).
        self.extra.retain(|s| ed.doc.para(s.anchor.para).is_some() && ed.doc.para(s.focus.para).is_some());
        for s in &mut self.extra {
            *s = Selection { anchor: ed.doc.clamp(s.anchor), focus: ed.doc.clamp(s.focus) };
        }
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                self.id,
                // While editing a header, Esc goes back to the text (and keeps the keyboard).
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    // (and with several carets, Esc goes back to one).
                    escape: self.hf_edit.is_some() || !self.extra.is_empty(),
                },
            )
        });
        let events = ui.input(|i| i.events.clone());
        for event in events {
            if self.read_only
                && matches!(
                    event,
                    Event::Text(_)
                        | Event::Paste(_)
                        | Event::Cut
                        | Event::Ime(_)
                        | Event::Key {
                            key: Key::Backspace
                                | Key::Delete
                                | Key::Enter
                                | Key::Tab
                                | Key::Z
                                | Key::Y
                                | Key::B
                                | Key::I
                                | Key::U,
                            ..
                        }
                )
            {
                continue;
            }
            if !self.extra.is_empty() && self.multi_event(ui, ed, &event) {
                continue;
            }
            match event {
                Event::Text(t) if !t.is_empty() && !t.chars().any(char::is_control) => {
                    self.preedit = None;
                    self.type_text(ed, &t);
                }
                Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    self.preedit = (!text.is_empty()).then_some(text);
                    self.touched();
                }
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    self.preedit = None;
                    if !text.is_empty() {
                        self.type_text(ed, &text);
                    }
                }
                Event::Copy => self.copy(ui.ctx(), ed, false),
                Event::Cut => self.copy(ui.ctx(), ed, true),
                Event::Paste(text) => {
                    let plain_only = ui.input(|i| i.modifiers.shift);
                    self.paste(ed, &text, plain_only);
                }
                Event::Key { key, pressed: true, modifiers, .. } => self.key(ed, key, modifiers),
                _ => {}
            }
        }
    }

    /// Whether document point `dp` (which hit `pos`) is on the box of a
    /// checklist item, left of its first line.
    fn on_checkbox(&self, ed: &Editor, pos: Pos, dp: kurbo::Point) -> bool {
        let Some(p) = ed.doc.para(pos.para) else { return false };
        let Some(item) = &p.list else { return false };
        let is_check =
            ed.doc.list_def(&item.list).is_some_and(|d| d.level(item.level).kind == xenify_wp_doc::NumberKind::Check);
        if !is_check {
            return false;
        }
        let Some(first) = self.layout.caret(Pos::new(pos.para, 0), false) else { return false };
        let hanging = ed.doc.resolve_para(p).hanging as f64;
        dp.y >= first.y0 && dp.y <= first.y1 && dp.x < first.x0 - 1.0 && dp.x > first.x0 - hanging - 4.0
    }

    /// The form checkbox (as the position before it) that document point `dp` is on.
    fn form_check_at(&self, ed: &Editor, pos: Pos, dp: kurbo::Point) -> Option<Pos> {
        self.field_at(ed, pos, dp, |f| matches!(f, xenify_wp_doc::Field::Check { .. }))
    }

    /// The field matching `which` (as the position before it) that document
    /// point `dp` (which hit `pos`) is on.
    fn field_at(
        &self,
        ed: &Editor,
        pos: Pos,
        dp: kurbo::Point,
        which: impl Fn(&xenify_wp_doc::Field) -> bool,
    ) -> Option<Pos> {
        use xenify_wp_doc::Field;
        let p = ed.doc.para(pos.para)?;
        let w = Field::CHAR.len_utf8();
        [Some(pos.offset), pos.offset.checked_sub(w)].into_iter().flatten().find_map(|at| {
            let is_box =
                p.text.get(at..at + w) == Some(Field::STR) && p.fmt_at(at + w).field.as_ref().is_some_and(&which);
            let a = self.layout.caret(Pos::new(pos.para, at), false)?;
            let b = self.layout.caret(Pos::new(pos.para, at + w), true)?;
            let on = dp.x >= a.x0.min(b.x0) - 1.0 && dp.x <= a.x0.max(b.x0) + 1.0 && dp.y >= a.y0 && dp.y <= a.y1;
            (is_box && on).then_some(Pos::new(pos.para, at))
        })
    }

    fn type_text(&mut self, ed: &mut Editor, text: &str) {
        self.selected_image = None;
        // Typing into a form's prompt ("Click to enter text") replaces it.
        if ed.sel.is_collapsed()
            && let Some((c, _)) = xenify_wp_doc::controls::control_at(ed, ed.sel.focus)
            && c.placeholder
        {
            let _ = xenify_wp_doc::controls::set_value(ed, ed.sel.focus, text);
            self.after_edit();
            return;
        }
        let fmt = self.pending_fmt.clone();
        let _ = ops::insert_text(ed, text, fmt.as_ref());
        // "* ", "1. ", "# " at the start of a paragraph: a list or heading.
        let formatted = text == " " && ops::autoformat(ed, &self.autoformat).unwrap_or(false);
        if !formatted {
            let _ = xenify_wp_doc::autocorrect::autocorrect(ed, text, &self.autocorrect);
        }
        self.after_edit();
    }

    /// Ctrl+D: selects the word at the caret, or else the next place the
    /// selected text occurs too.
    fn add_next_match(&mut self, ed: &mut Editor) {
        if ed.sel.is_collapsed() {
            ed.sel = ops::word_at(ed, ed.sel.focus);
            self.touched();
            return;
        }
        let text = ed.doc.text_in(ed.sel);
        let mut all = self.extra.clone();
        all.push(ed.sel);
        let after = ed.doc.ordered(ed.sel).1;
        if let Some(next) = xenify_wp_doc::multi::next_match(&ed.doc, &text, after, &all) {
            self.extra.push(ed.sel);
            ed.sel = next;
            self.reveal();
        }
        self.touched();
    }

    /// An event while there are several carets: edits happen at each, moves
    /// move each. False when it's for the usual handling (undo, shortcuts…).
    fn multi_event(&mut self, ui: &egui::Ui, ed: &mut Editor, event: &Event) -> bool {
        use xenify_wp_doc::{MergeKey, multi};
        let mut sels = self.extra.clone();
        sels.push(ed.sel);
        let each =
            |this: &mut Self, ed: &mut Editor, label: &str, merge: Option<MergeKey>, f: &mut dyn FnMut(&mut Editor)| {
                let done = multi::edit_each(ed, &sels, label, merge, f);
                this.extra = done[..done.len().saturating_sub(1)].to_vec();
                this.after_edit();
            };
        let typing = Some(MergeKey::Control("multi-typing".into()));
        match event {
            Event::Text(t) if !t.is_empty() && !t.chars().any(char::is_control) => {
                let fmt = self.pending_fmt.clone();
                each(self, ed, "Typing", typing, &mut |ed| drop(ops::insert_text(ed, t, fmt.as_ref())));
            }
            Event::Paste(text) => {
                // As many lines as carets: one each; otherwise all of it at each.
                let text = text.replace("\r\n", "\n");
                let lines: Vec<&str> = text.lines().collect();
                let n = sels.len();
                let mut k = n;
                each(self, ed, "Paste", None, &mut |ed| {
                    k -= 1;
                    let piece = if lines.len() == n { lines[k] } else { text.as_str() };
                    let _ = ops::insert_text(ed, piece, None);
                });
            }
            Event::Copy | Event::Cut => {
                let sels = multi::normalize(&ed.doc, &sels);
                let text: Vec<String> = sels.iter().map(|s| ed.doc.text_in(*s)).collect();
                ui.ctx().copy_text(text.join("\n"));
                if matches!(event, Event::Cut) {
                    each(self, ed, "Cut", None, &mut |ed| drop(ops::delete_selection(ed)));
                }
            }
            Event::Key { key, pressed: true, modifiers: m, .. } => {
                let word = m.command || m.alt;
                match key {
                    Key::Escape => {
                        self.extra.clear();
                        self.touched();
                    }
                    Key::Backspace => each(self, ed, "Delete", None, &mut |ed| drop(ops::backspace(ed, word))),
                    Key::Delete => each(self, ed, "Delete", None, &mut |ed| drop(ops::delete_forward(ed, word))),
                    Key::Enter if !m.command => {
                        each(self, ed, "New paragraph", None, &mut |ed| drop(ops::split_paragraph(ed)))
                    }
                    Key::Tab if !m.command && !m.shift => {
                        each(self, ed, "Typing", typing, &mut |ed| drop(ops::insert_text(ed, "\t", None)));
                    }
                    Key::B | Key::I | Key::U if m.command && !m.alt => {
                        let what = match key {
                            Key::B => Toggle::Bold,
                            Key::I => Toggle::Italic,
                            _ => Toggle::Underline,
                        };
                        let patch = what.patch(!self.is_on(ed, what));
                        each(self, ed, what.label(), None, &mut |ed| drop(ops::apply_format(ed, &patch, what.label())));
                    }
                    Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End
                        if !(m.command && m.alt) =>
                    {
                        // Move each (the usual way, one at a time).
                        let mut moved = Vec::new();
                        for s in &sels {
                            ed.sel = *s;
                            self.goal_x = None;
                            self.key(ed, *key, *m);
                            moved.push(ed.sel);
                        }
                        let main = ed.sel;
                        self.extra = multi::normalize(&ed.doc, &moved);
                        self.extra.retain(|s| *s != main);
                    }
                    // Undo, select all and the rest: back to one caret.
                    _ => {
                        if m.command && !matches!(key, Key::D) {
                            self.extra.clear();
                        }
                        return false;
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn after_edit(&mut self) {
        self.goal_x = None;
        self.upstream = false;
        self.touched();
    }

    /// Copies (or cuts) the selection: Write keeps the fragment for its
    /// own pastes, other programs get HTML and plain text.
    pub fn copy(&mut self, ctx: &egui::Context, ed: &mut Editor, cut: bool) {
        if ed.sel.is_collapsed() {
            return;
        }
        let plain = ed.doc.text_in(ed.sel);
        let frag = xenify_wp_doc::clip::copy_fragment(ed);
        let html = xenify_wp_io::save(&frag, xenify_wp_io::Format::Html)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        if !crate::clipboard::write(&plain, &html) {
            ctx.copy_text(plain.clone());
        }
        self.clip = Some((plain, frag));
        if cut {
            let _ = ops::delete_selection(ed);
            self.after_edit();
        }
    }

    /// Pastes `text` (what the system clipboard holds as text): Write's own
    /// copy with all its formatting, else the clipboard's HTML, else Markdown
    /// structure, else plain text. `plain_only`: just the text.
    pub fn paste(&mut self, ed: &mut Editor, text: &str, plain_only: bool) {
        let text = text.replace("\r\n", "\n");
        ed.seal();
        let frag = if plain_only {
            None
        } else {
            match &self.clip {
                Some((plain, frag)) if *plain == text => Some(frag.clone()),
                _ => crate::clipboard::html()
                    .map(|h| xenify_wp_io::html_read::read(&h))
                    .or_else(|| xenify_wp_doc::clip::tsv_fragment(&text))
                    .or_else(|| {
                        crate::clipboard::looks_like_markdown(&text)
                            .then(|| xenify_wp_io::load(text.as_bytes(), xenify_wp_io::Format::Markdown).ok())
                            .flatten()
                            .map(|l| l.doc)
                    }),
            }
        };
        match frag {
            Some(f) => {
                let _ = xenify_wp_doc::clip::paste_fragment(ed, &f, "Paste");
            }
            None => {
                let _ = ops::insert_text(ed, &text, None);
            }
        }
        ed.seal();
        self.after_edit();
    }

    /// Edit › Paste: the clipboard read directly (also pictures).
    pub fn paste_from_clipboard(&mut self, ed: &mut Editor, plain_only: bool) -> bool {
        if let Some(t) = crate::clipboard::text() {
            self.paste(ed, &t, plain_only);
            return true;
        }
        if !plain_only && let Some((png, w, h)) = crate::clipboard::image_png() {
            let key = format!("media/pasted-{}.png", ed.doc.next_id);
            ed.doc.assets.insert(key.clone(), std::sync::Arc::new(png));
            let _ = xenify_wp_doc::tables::insert_image(ed, &key, w as f32 * 0.75, h as f32 * 0.75);
            self.after_edit();
            return true;
        }
        false
    }

    /// Moves the caret to `pos` (extending the selection with Shift).
    fn go(&mut self, ed: &mut Editor, pos: Pos, extend: bool) {
        ed.sel = if extend { Selection { anchor: ed.sel.anchor, focus: pos } } else { Selection::caret(pos) };
        ed.seal();
        self.pending_fmt = None;
        self.touched();
    }

    fn key(&mut self, ed: &mut Editor, key: Key, m: Modifiers) {
        // A selected frame: Delete removes it, arrows nudge it, Escape lets go.
        if let Some(id) = self.selected_frame {
            let step = if m.shift { 10.0 } else { 1.0 };
            let nudge = match key {
                Key::ArrowLeft => Some((-step, 0.0)),
                Key::ArrowRight => Some((step, 0.0)),
                Key::ArrowUp => Some((0.0, -step)),
                Key::ArrowDown => Some((0.0, step)),
                _ => None,
            };
            match key {
                Key::Delete | Key::Backspace => {
                    self.selected_frame = None;
                    let _ = xenify_wp_doc::frames::delete_frame(ed, id);
                    self.after_edit();
                    return;
                }
                Key::Escape => {
                    self.selected_frame = None;
                    self.linking = None;
                    return;
                }
                _ if nudge.is_some() => {
                    let (dx, dy) = nudge.expect("checked");
                    let _ = xenify_wp_doc::frames::update_frame(ed, id, "Move text box", |f| {
                        f.x += dx;
                        f.y += dy;
                    });
                    self.touched();
                    return;
                }
                _ => self.selected_frame = None,
            }
        }
        // A selected picture: Delete removes it; anything else deselects it.
        if let Some(id) = self.selected_image.take()
            && matches!(key, Key::Delete | Key::Backspace)
        {
            let _ = xenify_wp_doc::tables::delete_block(ed, id);
            self.after_edit();
            return;
        }
        // More carets: Ctrl+D selects the next match too; Ctrl+Alt+Up/Down
        // add a caret on the line above/below.
        if key == Key::D && m.command && !m.alt && !m.shift {
            self.add_next_match(ed);
            return;
        }
        if matches!(key, Key::ArrowUp | Key::ArrowDown) && m.command && m.alt {
            let goal = self.layout.caret(ed.sel.focus, self.upstream).map_or(0.0, |c| c.x0);
            let n = if key == Key::ArrowUp { -1 } else { 1 };
            if let Some(pos) = self.layout.line_move(ed.sel.focus, self.upstream, n, goal) {
                self.extra.push(ed.sel);
                ed.sel = Selection::caret(pos);
                self.touched();
            }
            return;
        }
        let word = m.command || m.alt;
        let shift = m.shift;
        // Collapsing a selection with Left/Right goes to its start/end.
        let (start, end) = ed.doc.ordered(ed.sel);
        let vertical = matches!(key, Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown);
        if !vertical {
            self.goal_x = None;
        }
        match key {
            Key::ArrowLeft | Key::ArrowRight => {
                let fwd = key == Key::ArrowRight;
                if !shift && !ed.sel.is_collapsed() && !word {
                    self.go(ed, if fwd { end } else { start }, false);
                } else {
                    let pos = if word {
                        ops::word_move(ed, ed.sel.focus, fwd)
                    } else {
                        ops::char_move(ed, ed.sel.focus, fwd)
                    };
                    self.upstream = false;
                    self.go(ed, pos, shift);
                }
            }
            Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                let caret = self.layout.caret(ed.sel.focus, self.upstream);
                let goal = *self.goal_x.get_or_insert(caret.map_or(0.0, |c| c.x0));
                let lines: isize = match key {
                    Key::ArrowUp => -1,
                    Key::ArrowDown => 1,
                    Key::PageUp => -30,
                    _ => 30,
                };
                let mut pos = None;
                // Page moves go as far as they can.
                let mut n = lines;
                while n != 0 && pos.is_none() {
                    pos = self.layout.line_move(ed.sel.focus, self.upstream, n, goal);
                    if lines.abs() == 1 {
                        break;
                    }
                    n -= n.signum();
                }
                let pos = pos.unwrap_or(if lines < 0 { ed.doc.start() } else { ed.doc.end() });
                self.upstream = false;
                self.go(ed, pos, shift);
                self.goal_x = Some(goal);
            }
            Key::Home | Key::End => {
                let to_end = key == Key::End;
                let pos = if m.command {
                    if to_end { ed.doc.end() } else { ed.doc.start() }
                } else {
                    self.layout.line_edge(ed.sel.focus, self.upstream, to_end)
                };
                self.go(ed, pos, shift);
                // At the end of a wrapped line the caret stays on that line.
                self.upstream = to_end && !m.command;
            }
            Key::Backspace => {
                let _ = ops::backspace(ed, word);
                self.after_edit();
            }
            Key::Delete => {
                let _ = ops::delete_forward(ed, word);
                self.after_edit();
            }
            Key::Enter if m.command => {
                let _ = ops::insert_page_break(ed);
                self.after_edit();
            }
            Key::Enter if !m.command => {
                let _ = ops::split_paragraph(ed);
                self.after_edit();
            }
            Key::Tab if !m.command => {
                // Tab goes to the next table cell, moves list items in and
                // out, and elsewhere types a tab (to the next tab stop).
                if !matches!(xenify_wp_doc::tables::move_cell(ed, !shift), Ok(true))
                    && !matches!(ops::change_list_level(ed, if shift { -1 } else { 1 }), Ok(true))
                    && !shift
                {
                    self.type_text(ed, "\t");
                }
                self.touched();
            }
            Key::A if m.command => {
                ed.sel = Selection { anchor: ed.doc.start(), focus: ed.doc.end() };
                self.touched();
            }
            Key::Z if m.command && !shift => {
                ed.undo();
                self.after_edit();
            }
            Key::Z if m.command && shift => {
                ed.redo();
                self.after_edit();
            }
            Key::Y if m.command => {
                ed.redo();
                self.after_edit();
            }
            Key::B if m.command => self.toggle(ed, Toggle::Bold),
            Key::I if m.command => self.toggle(ed, Toggle::Italic),
            Key::U if m.command => self.toggle(ed, Toggle::Underline),
            _ => {}
        }
    }

    /// Bold / italic / underline / strikethrough on the selection, or for
    /// the next typed text when nothing is selected.
    pub fn toggle(&mut self, ed: &mut Editor, what: Toggle) {
        let now = self.is_on(ed, what);
        let patch = what.patch(!now);
        if ed.sel.is_collapsed() {
            let base = self.pending_fmt.clone().unwrap_or_else(|| {
                ed.doc.para(ed.sel.focus.para).map(|p| p.fmt_at(ed.sel.focus.offset)).unwrap_or_default()
            });
            self.pending_fmt = Some(ops::normalize_fmt(base.with(&patch)));
        } else {
            ed.seal();
            let _ = ops::apply_format(ed, &patch, what.label());
            ed.seal();
        }
        self.touched();
    }

    /// Whether `what` is on for the whole selection (or at the caret).
    pub fn is_on(&self, ed: &Editor, what: Toggle) -> bool {
        let style = ed.doc.para(ed.sel.focus.para).map(|p| ed.doc.styles.resolve(&p.style).char);
        let fmt = match (&self.pending_fmt, ed.sel.is_collapsed()) {
            (Some(f), true) => f.clone(),
            _ => ops::common_format(ed),
        };
        let (direct, styled) = match what {
            Toggle::Bold => (fmt.bold, style.as_ref().map(|s| s.bold)),
            Toggle::Italic => (fmt.italic, style.as_ref().map(|s| s.italic)),
            Toggle::Underline => (fmt.underline, style.as_ref().map(|s| s.underline)),
            Toggle::Strike => (fmt.strike, style.as_ref().map(|s| s.strike)),
            Toggle::Superscript => (Some(fmt.vert == Some(xenify_wp_doc::VertAlign::Superscript)), None),
            Toggle::Subscript => (Some(fmt.vert == Some(xenify_wp_doc::VertAlign::Subscript)), None),
        };
        direct.or(styled).unwrap_or(false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
}

impl Toggle {
    fn patch(self, on: bool) -> CharFmt {
        let v = Some(on);
        match self {
            Toggle::Bold => CharFmt { bold: v, ..Default::default() },
            Toggle::Italic => CharFmt { italic: v, ..Default::default() },
            Toggle::Underline => CharFmt { underline: v, ..Default::default() },
            Toggle::Strike => CharFmt { strike: v, ..Default::default() },
            Toggle::Superscript | Toggle::Subscript => {
                let raised = if self == Toggle::Superscript {
                    xenify_wp_doc::VertAlign::Superscript
                } else {
                    xenify_wp_doc::VertAlign::Subscript
                };
                CharFmt {
                    vert: Some(if on { raised } else { xenify_wp_doc::VertAlign::Baseline }),
                    ..Default::default()
                }
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Toggle::Bold => "Bold",
            Toggle::Italic => "Italic",
            Toggle::Underline => "Underline",
            Toggle::Strike => "Strikethrough",
            Toggle::Superscript => "Superscript",
            Toggle::Subscript => "Subscript",
        }
    }
}

/// A wavy underline from `x0` to `x1` at `y`.
fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut pts = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x <= x1 {
        pts.push(Pos2::new(x, if up { y - 1.5 } else { y + 0.5 }));
        x += 2.0;
        up = !up;
    }
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, Stroke::new(1.2, color)));
    }
}

/// The frames of the thread holding frame `id`, first to last.
/// The link on the character at `pos`, if any.
fn link_at(doc: &xenify_wp_doc::Document, pos: Pos) -> Option<String> {
    doc.para(pos.para).and_then(|p| {
        p.run_ranges()
            .find(|(r, _)| r.start <= pos.offset && pos.offset < r.end.max(r.start + 1))
            .and_then(|(_, f)| f.link.clone())
    })
}

pub fn chain_of(doc: &xenify_wp_doc::Document, id: xenify_wp_doc::NodeId) -> Vec<xenify_wp_doc::NodeId> {
    let frames: Vec<&xenify_wp_doc::Frame> = doc.sections.iter().flat_map(|s| &s.frames).collect();
    // Back to the head…
    let mut head = id;
    let mut seen = vec![id];
    while let Some(prev) = frames.iter().find(|f| f.next == Some(head)).map(|f| f.id) {
        if seen.contains(&prev) {
            break;
        }
        seen.push(prev);
        head = prev;
    }
    // …then forward.
    let mut out = vec![head];
    let mut cur = head;
    while let Some(n) = frames.iter().find(|f| f.id == cur).and_then(|f| f.next) {
        if out.contains(&n) {
            break;
        }
        out.push(n);
        cur = n;
    }
    if out.len() > 1 { out } else { Vec::new() }
}

/// Makes `target` continue `src`'s text. Text already in `target` moves to
/// the end of the story, so nothing is lost.
pub fn link_frames(ed: &mut Editor, src: xenify_wp_doc::NodeId, target: xenify_wp_doc::NodeId) {
    use xenify_wp_doc::frames;
    // No loops: the target can't already lead to the source.
    if chain_of(&ed.doc, target).iter().skip_while(|f| **f != target).any(|f| *f == src) {
        return;
    }
    // The target's own text joins the story (unless it's just an empty paragraph).
    let moved: Vec<xenify_wp_doc::Block> = ed
        .doc
        .frame(target)
        .map(|f| {
            f.blocks
                .iter()
                .filter(|b| !matches!(b, xenify_wp_doc::Block::Paragraph(p) if p.text.is_empty()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let head = chain_of(&ed.doc, src).first().copied().unwrap_or(src);
    ed.seal();
    let _ = frames::update_frame(ed, src, "Link text boxes", |f| f.next = Some(target));
    if !moved.is_empty() {
        let _ = frames::update_frame(ed, head, "Link text boxes", |f| f.blocks.extend(moved.clone()));
        let _ = frames::update_frame(ed, target, "Link text boxes", |f| {
            f.blocks.retain(|b| !moved.iter().any(|m| m.id() == b.id()))
        });
    }
    ed.seal();
}

/// A dashed rectangle outline.
fn dashed_rect(painter: &egui::Painter, r: Rect, stroke: Stroke) {
    for (a, b) in [
        (r.left_top(), r.right_top()),
        (r.right_top(), r.right_bottom()),
        (r.right_bottom(), r.left_bottom()),
        (r.left_bottom(), r.left_top()),
    ] {
        painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
    }
}
