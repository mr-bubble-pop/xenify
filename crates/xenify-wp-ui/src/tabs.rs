//! Several documents in one window, as tabs (as in a web browser). The document in
//! front lives in [`WriteApp`]'s own fields (`editor`, `view`, `path`…), so
//! everything else works on it as before; the others wait here as
//! [`DocTab`]s and are swapped in when their tab is chosen.

use crate::app::WriteApp;
use crate::recovery::Autosave;
use crate::review_panel::ReviewState;
use crate::view::DocView;
use xenify_wp_doc::{Document, Editor};
use xenify_wp_io::Format;
use std::path::{Path, PathBuf};

/// A document that's open but not in front.
pub struct DocTab {
    /// Names it for as long as it's open (stays the same across swaps).
    pub key: u64,
    pub editor: Editor,
    pub view: DocView,
    pub path: Option<PathBuf>,
    pub format: Format,
    pub recovery: Autosave,
    pub review: ReviewState,
    pub checks: crate::proofing::DocChecks,
    pub accepted_losses: Option<(PathBuf, Vec<String>)>,
}

/// The tabs' order and which is in front.
#[derive(Default)]
pub struct Tabs {
    /// Tab keys, left to right (the one in front included).
    pub order: Vec<u64>,
    /// The tab in front.
    pub active: u64,
    /// The documents not in front.
    pub others: Vec<DocTab>,
    next: u64,
}

impl Tabs {
    pub fn new() -> Self {
        Self { order: vec![0], active: 0, others: Vec::new(), next: 1 }
    }

    fn fresh_key(&mut self) -> u64 {
        let k = self.next;
        self.next += 1;
        k
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Never: a window always has a document.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The tab `key`'s document, whether in front or not.
    pub fn doc<'a>(&'a self, app: &'a WriteApp, key: u64) -> Option<(&'a Document, Option<&'a Path>)> {
        if key == self.active {
            return Some((&app.editor.doc, app.path.as_deref()));
        }
        self.others.iter().find(|t| t.key == key).map(|t| (&t.editor.doc, t.path.as_deref()))
    }
}

/// A file's name for its tab and the title bar.
pub fn name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy().into_owned())
}

impl WriteApp {
    /// Brings tab `key` to the front.
    pub fn switch_to(&mut self, key: u64) {
        if key == self.tabs.active {
            return;
        }
        if self.swap_front(key) {
            self.after_switch();
        }
    }

    /// Puts document `key` in the front slot (the fields everything works
    /// on), keeping the one there with the others. False when there's no
    /// such document.
    pub(crate) fn swap_front(&mut self, key: u64) -> bool {
        if key == self.tabs.active {
            return true;
        }
        let Some(i) = self.tabs.others.iter().position(|t| t.key == key) else { return false };
        let mut t = self.tabs.others.remove(i);
        let show_review = self.review.show;
        std::mem::swap(&mut self.editor, &mut t.editor);
        std::mem::swap(&mut self.view, &mut t.view);
        std::mem::swap(&mut self.path, &mut t.path);
        std::mem::swap(&mut self.format, &mut t.format);
        std::mem::swap(&mut self.recovery, &mut t.recovery);
        std::mem::swap(&mut self.review, &mut t.review);
        std::mem::swap(&mut self.accepted_losses, &mut t.accepted_losses);
        // The checker's state for this document, and the old one's kept.
        self.proofing.swap_doc(key, &mut t.checks);
        // The comments pane stays open or shut as it was.
        self.review.show = show_review;
        t.key = self.tabs.active;
        self.tabs.others.push(t);
        self.tabs.active = key;
        true
    }

    /// What belongs to the document in front only: closed when it changes.
    fn after_switch(&mut self) {
        self.print_preview = None;
        self.control_menu = None;
        self.menu_pos = None;
        self.find.forget();
        self.merge.preview = None;
        crate::merge_panel::load(self);
        self.refocus = true;
    }

    /// Opens `doc` in a new tab in front (after the current one).
    pub fn open_in_new_tab(&mut self, doc: Document, path: Option<PathBuf>, format: Format) {
        let key = self.tabs.fresh_key();
        let mut recovery = self.recovery.sibling(key);
        recovery.bind(path.as_deref());
        let t = DocTab {
            key,
            editor: Editor::new(Document::new()),
            view: DocView::new(self.view.engine.fonts.clone()),
            path: None,
            format: Format::Markdown,
            recovery,
            review: ReviewState::default(),
            checks: crate::proofing::DocChecks::new(key),
            accepted_losses: None,
        };
        let at = self.tabs.order.iter().position(|k| *k == self.tabs.active).map_or(self.tabs.order.len(), |i| i + 1);
        self.tabs.order.insert(at, key);
        self.tabs.others.push(t);
        self.switch_to(key);
        self.view.zoom = self.prefs.zoom.unwrap_or(1.0).clamp(0.25, 4.0);
        self.view.autoformat = self.prefs.autoformat.clone().unwrap_or_default();
        self.view.autocorrect = self.prefs.autocorrect.clone().unwrap_or_default();
        self.set_document(doc, path, format);
    }

    /// Whether the document in front is an untouched new one (opening a file
    /// can take its place instead of adding a tab).
    pub fn front_is_blank(&self) -> bool {
        self.path.is_none() && !self.editor.is_dirty() && self.editor.doc.paragraphs().all(|p| p.text.is_empty())
    }

    /// Closes the tab in front (asking first is the caller's job). The last
    /// tab isn't closed: it becomes a new blank document.
    pub fn close_front_tab(&mut self) {
        let closing = self.tabs.active;
        self.recovery.clear();
        if self.tabs.order.len() <= 1 {
            self.set_document(Document::new(), None, Format::Markdown);
            return;
        }
        let i = self.tabs.order.iter().position(|k| *k == closing).unwrap_or(0);
        let next = self.tabs.order[if i + 1 < self.tabs.order.len() { i + 1 } else { i - 1 }];
        self.switch_to(next);
        self.tabs.order.retain(|k| *k != closing);
        self.tabs.others.retain(|t| t.key != closing);
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab: the next or previous tab.
    pub fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.order.len();
        if n < 2 {
            return;
        }
        let i = self.tabs.order.iter().position(|k| *k == self.tabs.active).unwrap_or(0);
        let j = if forward { (i + 1) % n } else { (i + n - 1) % n };
        self.switch_to(self.tabs.order[j]);
    }

    /// Keys of open tabs with unsaved changes.
    pub fn unsaved_tabs(&self) -> Vec<u64> {
        let mut out: Vec<u64> = self.tabs.others.iter().filter(|t| t.editor.is_dirty()).map(|t| t.key).collect();
        if self.editor.is_dirty() {
            out.insert(0, self.tabs.active);
        }
        out
    }

    /// Autosaves of the documents not in front (the one in front is done each frame).
    pub(crate) fn tick_background_autosaves(&mut self) {
        for t in &mut self.tabs.others {
            let dirty = t.editor.is_dirty();
            t.recovery.tick(&t.editor.doc, dirty);
        }
    }
}

/// The strip of tabs under the menus, as in a web browser: fixed-width
/// tabs (narrower when there are many) whose front one joins the toolbar
/// below; click to switch, drag to reorder, × or middle-click to close,
/// + or a double-click on the empty strip for a new document.
pub fn strip(app: &mut WriteApp, ui: &mut egui::Ui) {
    let mut switch = None;
    let mut close = None;
    let mut new = false;
    let mut to_window = None;
    let mut moved: Option<(u64, f32)> = None;
    let height = 32.0;
    let visuals = ui.visuals().clone();
    let front_fill = visuals.panel_fill;
    let accent = crate::theme::accent(&visuals);
    let order = app.tabs.order.clone();
    // Each tab as wide as fits, between these.
    let room = ui.available_width() - 40.0;
    let w = (room / order.len().max(1) as f32).clamp(90.0, 220.0);
    ui.spacing_mut().item_spacing.x = 0.0;
    let mut rects = Vec::new();
    egui::ScrollArea::horizontal()
        .id_salt("tab-strip")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(6.0);
                let mut untitled = 0;
                for (i, &key) in order.iter().enumerate() {
                    let Some((doc_dirty, path)) = (if key == app.tabs.active {
                        Some((app.editor.is_dirty(), app.path.clone()))
                    } else {
                        app.tabs.others.iter().find(|t| t.key == key).map(|t| (t.editor.is_dirty(), t.path.clone()))
                    }) else {
                        continue;
                    };
                    // Untitled documents are told apart by number.
                    let title = match &path {
                        Some(_) => name(path.as_deref()),
                        None => {
                            untitled += 1;
                            if untitled == 1 { "Untitled".to_string() } else { format!("Untitled {untitled}") }
                        }
                    };
                    let front = key == app.tabs.active;
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, height), egui::Sense::click_and_drag());
                    rects.push((key, rect));
                    let tip = path.as_ref().map_or("Not saved yet".to_string(), |p| p.display().to_string());
                    let r = r.on_hover_text(tip);
                    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, front, &title));
                    let close_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 16.0, rect.center().y + 2.0),
                        egui::vec2(18.0, 18.0),
                    );
                    let x = ui
                        .interact(close_rect, r.id.with("close"), egui::Sense::click())
                        .on_hover_text("Close (Ctrl+W)");
                    x.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {title}"))
                    });
                    // The tab: rounded at the top, the front one the toolbar's colour.
                    let body =
                        egui::Rect::from_min_max(rect.min + egui::vec2(1.0, 4.0), rect.max - egui::vec2(1.0, 0.0));
                    let corners = egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 };
                    let p = ui.painter();
                    if front {
                        p.rect_filled(body, corners, front_fill);
                        p.line_segment(
                            [body.left_top() + egui::vec2(8.0, 0.0), body.right_top() - egui::vec2(8.0, 0.0)],
                            egui::Stroke::new(2.0, accent),
                        );
                    } else if r.hovered() || x.hovered() {
                        p.rect_filled(body.shrink2(egui::vec2(2.0, 2.0)), 6, visuals.widgets.hovered.weak_bg_fill);
                    }
                    // A thin line between tabs that aren't in front.
                    let next_front = order.get(i + 1) == Some(&app.tabs.active);
                    if !front && !next_front && i + 1 < order.len() {
                        let c = visuals.widgets.noninteractive.bg_stroke.color;
                        p.line_segment(
                            [
                                egui::pos2(rect.right(), rect.top() + 10.0),
                                egui::pos2(rect.right(), rect.bottom() - 6.0),
                            ],
                            egui::Stroke::new(1.0, c),
                        );
                    }
                    let text_color = if front { visuals.strong_text_color() } else { visuals.text_color() };
                    let label = format!("{}{title}", if doc_dirty { "• " } else { "" });
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let max = w - 44.0;
                    let galley = ui.fonts_mut(|f| {
                        let mut job = egui::text::LayoutJob::simple_singleline(label, font, text_color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(max);
                        f.layout_job(job)
                    });
                    let at = egui::pos2(rect.left() + 14.0, rect.center().y + 2.0 - galley.size().y / 2.0);
                    ui.painter().galley(at, galley, text_color);
                    // ×: always on the front tab, on the others when pointed at.
                    if front || r.hovered() || x.hovered() {
                        if x.hovered() {
                            ui.painter().rect_filled(close_rect, 4, visuals.widgets.active.weak_bg_fill);
                        }
                        let c = close_rect.center();
                        let d = 4.0;
                        let stroke = egui::Stroke::new(1.4, text_color);
                        ui.painter().line_segment([c - egui::vec2(d, d), c + egui::vec2(d, d)], stroke);
                        ui.painter().line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], stroke);
                    }
                    if x.clicked() || r.middle_clicked() {
                        close = Some(key);
                    } else if r.clicked() || r.drag_started() {
                        switch = Some(key);
                    }
                    // Dragging a tab along the strip moves it.
                    if r.dragged()
                        && let Some(pos) = ui.ctx().pointer_interact_pos()
                    {
                        moved = Some((key, pos.x));
                    }
                    r.context_menu(|ui| {
                        if ui.button("New tab").clicked() {
                            new = true;
                            ui.close();
                        }
                        if ui.button("Move to new window").clicked() {
                            to_window = Some(key);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Close").clicked() {
                            close = Some(key);
                            ui.close();
                        }
                    });
                }
                ui.add_space(4.0);
                let (plus, r) = ui.allocate_exact_size(egui::vec2(30.0, height), egui::Sense::click());
                let r = r.on_hover_text("New tab (Ctrl+T)");
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New tab"));
                let c = plus.center() + egui::vec2(0.0, 2.0);
                if r.hovered() {
                    ui.painter().rect_filled(
                        egui::Rect::from_center_size(c, egui::vec2(24.0, 24.0)),
                        6,
                        visuals.widgets.hovered.weak_bg_fill,
                    );
                }
                let stroke = egui::Stroke::new(1.5, visuals.text_color());
                ui.painter().line_segment([c - egui::vec2(6.0, 0.0), c + egui::vec2(6.0, 0.0)], stroke);
                ui.painter().line_segment([c - egui::vec2(0.0, 6.0), c + egui::vec2(0.0, 6.0)], stroke);
                if r.clicked() {
                    new = true;
                }
                // The empty rest of the strip: a double-click opens a new tab.
                let rest = ui.available_size_before_wrap();
                let (_, r) = ui.allocate_exact_size(egui::vec2(rest.x.max(0.0), height), egui::Sense::click());
                if r.double_clicked() || r.middle_clicked() {
                    new = true;
                }
            });
        });
    if let Some((key, x)) = moved {
        // Its place: after every other tab whose middle is left of the pointer.
        let to = rects.iter().filter(|(k, r)| *k != key && r.center().x < x).count();
        if let Some(from) = app.tabs.order.iter().position(|k| *k == key)
            && from != to
        {
            let k = app.tabs.order.remove(from);
            app.tabs.order.insert(to.min(app.tabs.order.len()), k);
        }
    }
    if let Some(k) = switch {
        app.switch_to(k);
    }
    if let Some(k) = close {
        app.switch_to(k);
        app.close_tab_asking();
    }
    if new {
        app.open_in_new_tab(Document::new(), None, Format::Markdown);
    }
    if let Some(k) = to_window {
        app.move_tab_to_new_window(k);
    }
}

/// The colour behind the menus and tabs: a little darker than the
/// toolbar, so the tab in front (the toolbar's colour) joins it.
pub fn strip_fill(v: &egui::Visuals) -> egui::Color32 {
    v.panel_fill.lerp_to_gamma(egui::Color32::BLACK, if v.dark_mode { 0.55 } else { 0.09 })
}
