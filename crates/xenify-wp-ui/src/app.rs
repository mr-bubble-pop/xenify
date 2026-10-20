use crate::file_browser::{FileBrowser, Mode};
use crate::view::{DocView, Toggle};
use egui::{Button, Color32, RichText};
use xenify_wp_doc::{Document, Editor, ops};
use xenify_wp_io::Format;
use std::path::{Path, PathBuf};

/// Extensions the Open dialog lists.
const OPEN_EXT: &[&str] = &["md", "markdown", "docx", "igdoc", "html", "htm", "txt"];
const SAVE_EXT: &[&str] = &["md", "docx", "igdoc", "txt"];

pub(crate) enum Dialog {
    Open(FileBrowser),
    SaveAs(FileBrowser),
    /// Unsaved changes: what to do before `then`.
    Unsaved(Then),
    About,
    /// Add or edit the link on the selection.
    Link(String),
    /// Saving to `path` would lose these things.
    Losses {
        path: PathBuf,
        losses: Vec<String>,
    },
    PageSetup(Box<crate::page_setup::PageSetupDialog>),
    Paragraph(crate::paragraph_dialog::ParagraphDialog),
    Sources(Box<crate::references::SourcesDialog>),
    Cite(crate::references::CiteDialog),
    InsertImage(FileBrowser),
    Export(FileBrowser, Format),
    /// A list of things to tell the user (import report).
    Notice(String, Vec<String>),
    /// Format › Table › Table properties.
    TableProps(Box<crate::table_dialog::TableDialog>),
    /// Tools › Word count.
    WordCount,
    /// Help › Keyboard shortcuts.
    Shortcuts,
    /// File › Record a change: the summary being written, and whether to save after.
    RecordChange(String, bool),
    /// Unsaved work an earlier run left: (what, the work, untitled key).
    Recover(Box<(crate::recovery::Info, Document, Option<String>)>),
    /// Edit › Go to page: what's typed.
    GoTo(String),
    /// File › Document properties: the title and author being edited.
    Properties(String, String),
}

/// The top-level menus' ids (to open them from the keyboard), and whether
/// a menu just opened that way should focus its first entry.
#[derive(Default)]
pub(crate) struct MenuKeys {
    pub(crate) ids: Vec<egui::Id>,
    pub(crate) focus_first: bool,
    /// Alt is down and nothing else has been pressed with it.
    alt_alone: bool,
    alt_was: bool,
    /// Alt was tapped on its own: the menu bar has the keyboard, with this menu highlighted.
    pub(crate) armed: Option<usize>,
}

#[derive(Clone)]
pub(crate) enum Then {
    New,
    Template(usize),
    Open,
    OpenPath(PathBuf),
    /// Close the tab in front.
    CloseTab,
    Quit,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct Prefs {
    pub(crate) ai_consent: bool,
    pub(crate) show_ai: bool,
    pub(crate) proofing: xenify_wp_check::Settings,
    pub(crate) spotter: xenify_wp_check::SpotterPrefs,
    pub(crate) show_proofing: bool,
    pub(crate) recent: Vec<PathBuf>,
    pub(crate) zoom: Option<f32>,
    pub(crate) show_styles: bool,
    pub(crate) unit: crate::page_setup::Unit,
    pub(crate) appearance: crate::theme::Appearance,
    pub(crate) hide_toolbar: bool,
    pub(crate) hide_ruler: bool,
    pub(crate) autoformat: Option<ops::Autoformat>,
    pub(crate) autocorrect: Option<xenify_wp_doc::autocorrect::Autocorrect>,
    /// Symbols and emoji inserted lately (Insert › Symbols & emoji).
    pub(crate) recent_symbols: Vec<String>,
    pub(crate) show_nav: bool,
    /// Dictionary: look up words as they're selected.
    pub(crate) dictionary_follow: bool,
    /// Documents open when Xenify Write last closed, and the one in front.
    pub(crate) open_files: Vec<PathBuf>,
    pub(crate) open_front: usize,
    /// Start with a blank document instead of reopening those.
    pub(crate) start_blank: bool,
    /// The user picked a language; otherwise follow the computer's.
    pub(crate) dialect_chosen: bool,
    /// The name on comments and tracked changes.
    pub(crate) user_name: String,
    /// Ask for a change-log note on every save.
    pub(crate) ask_change_note: bool,
    /// The side dock on the left (else the right).
    pub(crate) dock_left: bool,
    /// Dialogs inside the app's window (else windows of their own).
    pub(crate) dialogs_inside: bool,
    /// Interface size, high contrast and reduce motion.
    pub(crate) display: crate::theme::Display,
}

pub struct WriteApp {
    pub editor: Editor,
    /// Comments and tracked changes pane.
    pub review: crate::review_panel::ReviewState,
    /// Autosaves for recovering unsaved work after a crash.
    pub recovery: crate::recovery::Autosave,
    /// File › Settings.
    pub settings: crate::settings::SettingsState,
    /// The side dock's tabs.
    pub dock: crate::dock::Dock,
    /// Keyboard access to the menus.
    pub(crate) menu_keys: MenuKeys,
    /// Dictionary and thesaurus (a dock tab).
    pub dictionary: crate::dictionary::Dictionary,
    /// Edit › Find and Replace.
    pub find: crate::find::FindState,
    pub symbols: crate::symbols::SymbolPicker,
    /// File › Print: the print screen, while it's showing.
    pub print_preview: Option<crate::print_preview::PrintPreview>,
    /// A form's drop-down list, open: where in the text, and where on screen.
    pub control_menu: Option<(xenify_wp_doc::Pos, egui::Pos2)>,
    pub nav: crate::nav::Nav,
    /// The open documents (the one in front is `editor`, `view`, `path`…).
    pub tabs: crate::tabs::Tabs,
    /// View › Split: a second pane.
    pub split: Option<crate::split::Split>,
    /// The equation editor, while it's open.
    pub equation: Option<crate::equation::EquationEditor>,
    /// Windows other than this one.
    pub windows: Vec<crate::windows::Window>,
    pub(crate) window_count: u64,
    /// Quitting: tabs whose unsaved changes the user has answered for.
    quit_ok: std::collections::HashSet<u64>,
    /// Where the document's right-click menu was opened.
    pub(crate) menu_pos: Option<xenify_wp_doc::Pos>,
    /// The generated list (contents, bibliography…) right-clicked.
    pub(crate) menu_generated: Option<xenify_wp_doc::NodeId>,
    recovery_checked: bool,
    /// The name on comments and tracked changes (Review pane, "Your name").
    pub user_name: String,
    pub view: DocView,
    /// Where the document lives, and in which format.
    pub path: Option<PathBuf>,
    pub format: Format,
    pub(crate) dialog: Option<Dialog>,
    /// A message for the status bar (text, is error).
    pub message: Option<(String, bool)>,
    pub(crate) prefs: Prefs,
    pub(crate) title: String,
    /// The user confirmed quitting with unsaved changes.
    quitting: bool,
    /// What to do once Save as (from the unsaved-changes prompt) succeeds.
    pending_after_save: Option<Then>,
    /// Font family names (filled on first use).
    pub(crate) families: Vec<String>,
    pub show_styles: bool,
    pub proofing: crate::proofing::Proofing,
    pub show_proofing: bool,
    pub assistant: crate::assistant::Assistant,
    pub show_ai: bool,
    pub merge: crate::merge_panel::MergeState,
    pub show_merge: bool,
    /// The proofing issue whose menu is open, and where.
    menu_issue: Option<(xenify_wp_check::Issue, egui::Pos2)>,
    /// Put the keyboard back in the page next frame (after a panel closes:
    /// this frame its button still has the pointer).
    pub(crate) refocus: bool,
    /// Losses the user agreed to when saving to this path (don't ask again).
    pub(crate) accepted_losses: Option<(PathBuf, Vec<String>)>,
    pub styles_panel: crate::styles_panel::StylesPanel,
    pub(crate) ruler: crate::ruler::Ruler,
    /// The Borders and shading window is open.
    show_borders: bool,
}

impl WriteApp {
    pub fn new(cc: &eframe::CreationContext<'_>, fonts: xenify_wp_layout::Fonts) -> Self {
        let saved: Option<Prefs> = cc.storage.and_then(|s| eframe::get_value(s, "prefs"));
        let mut prefs = saved.unwrap_or_default();
        crate::theme::install_fonts(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx, prefs.appearance);
        if !prefs.dialect_chosen {
            // The computer's variety of English (en_AU → Australian…).
            prefs.proofing.dialect = xenify_wp_check::Dialect::from_locale();
        }
        let mut view = DocView::new(fonts);
        view.autoformat = prefs.autoformat.clone().unwrap_or_default();
        view.autocorrect = prefs.autocorrect.clone().unwrap_or_default();
        view.zoom = prefs.zoom.unwrap_or(1.0).clamp(0.25, 4.0);
        cc.egui_ctx.memory_mut(|m| m.request_focus(view.id));
        let show_styles = prefs.show_styles;
        let mut proofing = crate::proofing::Proofing::new(prefs.proofing.clone(), prefs.spotter.clone());
        proofing.dialect_auto = !prefs.dialect_chosen;
        let show_proofing = prefs.show_proofing;
        let mut assistant = crate::assistant::Assistant::default();
        assistant.consent = prefs.ai_consent;
        let show_ai = prefs.show_ai;
        let user_name = if prefs.user_name.trim().is_empty() { default_user_name() } else { prefs.user_name.clone() };
        Self {
            review: Default::default(),
            recovery: Default::default(),
            settings: Default::default(),
            dock: Default::default(),
            menu_keys: Default::default(),
            dictionary: crate::dictionary::Dictionary::following(prefs.dictionary_follow),
            menu_pos: None,
            menu_generated: None,
            find: Default::default(),
            symbols: Default::default(),
            print_preview: None,
            control_menu: None,
            nav: crate::nav::Nav::new(prefs.show_nav),
            tabs: crate::tabs::Tabs::new(),
            split: None,
            equation: None,
            windows: Vec::new(),
            window_count: 0,
            quit_ok: Default::default(),
            recovery_checked: false,
            user_name,
            merge: Default::default(),
            show_merge: false,
            assistant,
            show_ai,
            proofing,
            show_proofing,
            menu_issue: None,
            refocus: false,
            editor: Editor::new(Document::new()),
            view,
            path: None,
            format: Format::Markdown,
            dialog: None,
            message: None,
            prefs,
            title: String::new(),
            quitting: false,
            pending_after_save: None,
            families: Vec::new(),
            show_styles,
            styles_panel: Default::default(),
            ruler: Default::default(),
            show_borders: false,
            accepted_losses: None,
        }
    }

    fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    fn open_export(&mut self, format: Format) {
        let mut b = FileBrowser::new(Mode::Save, &self.start_dir(), &[format.extension()]);
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned());
        b.name = format!("{stem}.{}", format.extension());
        self.dialog = Some(Dialog::Export(b, format));
    }

    /// Bytes of `doc` in `format`, laid out with the editor's fonts.
    fn export_bytes(&mut self, format: Format) -> Result<Vec<u8>, String> {
        let doc = &self.editor.doc;
        let base = self.view.base_dir.clone();
        let assets = doc.assets.clone();
        let read = move |src: &str| crate::images::bytes_for_parts(src, &assets, base.as_deref());
        xenify_wp_io::save_with(doc, format, &mut self.view.engine, &read).map_err(|e| e.to_string())
    }

    /// Exports a copy (the document keeps its own file and format).
    pub fn export_to(&mut self, path: &Path, format: Format) -> bool {
        let r = self.export_bytes(format).and_then(|b| write_atomically(path, &b).map_err(|e| e.to_string()));
        // Markdown exports carry their pictures along.
        let r = r.and_then(|()| {
            if format != Format::Markdown {
                return Ok(());
            }
            let dir = path.parent().unwrap_or(Path::new("."));
            for src in xenify_wp_io::image_sources(&self.editor.doc) {
                if let Some(bytes) = self.editor.doc.assets.get(&src) {
                    let t = dir.join(&src);
                    if let Some(p) = t.parent() {
                        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&t, bytes.as_slice()).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        });
        match r {
            Ok(()) => {
                self.message = Some((format!("Exported {}", path.display()), false));
                true
            }
            Err(e) => {
                self.message = Some((format!("Couldn't export {}: {e}", path.display()), true));
                false
            }
        }
    }

    /// A form's drop-down list: its choices under the click; one picks it.
    /// (`fresh`: it opened this frame, on the click that's still in progress.)
    fn control_list(&mut self, ctx: &egui::Context, fresh: bool) {
        use xenify_wp_doc::controls::{self, ControlKind};
        let Some((pos, at)) = self.control_menu else { return };
        let Some((c, _)) = controls::control_at(&self.editor, pos) else {
            self.control_menu = None;
            return;
        };
        let items = match &c.kind {
            ControlKind::Dropdown { items } | ControlKind::ComboBox { items } => items.clone(),
            _ => Vec::new(),
        };
        let mut chosen = None;
        let r = egui::Area::new(egui::Id::new("control-list"))
            .order(egui::Order::Foreground)
            .fixed_pos(at + egui::vec2(0.0, 14.0))
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    if !c.title.is_empty() {
                        ui.weak(&c.title);
                    }
                    for item in &items {
                        if ui.add(egui::Button::new(item).frame(false)).clicked() {
                            chosen = Some(item.clone());
                        }
                    }
                    if items.is_empty() {
                        ui.weak("No choices");
                    }
                });
            });
        let away = ctx.input(|i| i.key_pressed(egui::Key::Escape))
            || (!fresh && ctx.input(|i| i.pointer.any_pressed()) && !r.response.contains_pointer() && chosen.is_none());
        if let Some(v) = chosen {
            let _ = controls::set_value(&mut self.editor, pos, &v);
            self.control_menu = None;
            // Back to typing in the document.
            self.refocus = true;
        } else if away {
            self.control_menu = None;
        }
    }

    fn open_print(&mut self) {
        crate::print_preview::open(self);
    }

    /// The document as PDF (for printing).
    pub(crate) fn export_pdf_bytes(&mut self) -> Result<Vec<u8>, String> {
        self.export_bytes(Format::Pdf)
    }

    /// File › Export as PDF: asks where to save it.
    pub(crate) fn open_export_pdf(&mut self) {
        self.open_export(Format::Pdf);
    }

    fn caret_section(&self) -> usize {
        self.editor.doc.section_of(self.editor.sel.focus.para).unwrap_or(0)
    }

    pub fn open_page_setup(&mut self) {
        let si = self.caret_section();
        let s = &self.editor.doc.sections[si];
        self.dialog =
            Some(Dialog::PageSetup(Box::new(crate::page_setup::PageSetupDialog::new(si, s.start, s.page.clone()))));
    }

    fn apply_page_setup(&mut self, d: &crate::page_setup::PageSetupDialog) {
        self.editor.seal();
        let targets: Vec<usize> =
            if d.whole_document { (0..self.editor.doc.sections.len()).collect() } else { vec![d.section] };
        for si in targets {
            let start = if si == d.section { d.start } else { self.editor.doc.sections[si].start };
            let _ = ops::set_section(&mut self.editor, si, start, d.page.clone());
        }
        self.editor.seal();
    }

    /// An 8×8 grid: hover to choose the size, click to insert.
    pub(crate) fn table_picker(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::new("table-picker");
        let (mut rows, mut cols): (usize, usize) = ui.data(|d| d.get_temp(id)).unwrap_or((2, 3));
        ui.label(format!("{cols} × {rows} table"));
        let cell = 18.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell * 8.0, cell * 8.0), egui::Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Table size"));
        if let Some(p) = resp.hover_pos() {
            let d = (p - rect.min) / cell;
            (cols, rows) = ((d.x as usize + 1).min(8), (d.y as usize + 1).min(8));
            ui.data_mut(|dd| dd.insert_temp(id, (rows, cols)));
        }
        for r in 0..8 {
            for c in 0..8 {
                let cr = egui::Rect::from_min_size(
                    rect.min + egui::vec2(c as f32, r as f32) * cell,
                    egui::vec2(cell - 3.0, cell - 3.0),
                );
                let on = r < rows && c < cols;
                ui.painter().rect_filled(
                    cr,
                    2.0,
                    if on { egui::Color32::from_rgb(0x1a, 0x73, 0xe8) } else { ui.visuals().faint_bg_color },
                );
                ui.painter().rect_stroke(
                    cr,
                    2.0,
                    ui.visuals().widgets.noninteractive.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        if resp.clicked() {
            if let Err(e) = xenify_wp_doc::tables::insert_table(&mut self.editor, rows, cols) {
                self.message = Some((format!("Couldn't insert a table here ({e})"), true));
            }
            self.view.reveal();
            ui.close();
        }
    }

    pub(crate) fn table_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        use xenify_wp_doc::tables as t;
        let here = t::cell_at(&self.editor, self.editor.sel.focus.para);
        let Some((table_id, _, col)) = here else {
            ui.weak("Put the caret in a table to change it.");
            icons::submenu(ui, Icon::Table, "Insert table", |ui| self.table_picker(ui));
            return;
        };
        let first_record = self.merge.records.first().cloned();
        let sortable = t::can_sort(&self.editor.doc, table_id);
        let mut props = false;
        {
            let ed = &mut self.editor;
            let mut act = |ui: &mut egui::Ui, icon: Icon, label: &str, f: &mut dyn FnMut(&mut Editor)| {
                if icons::item(ui, icon, label, "").clicked() {
                    f(&mut *ed);
                    ui.close();
                }
            };
            act(ui, Icon::RowAdd, "Insert row above", &mut |e| drop(t::insert_row(e, false)));
            act(ui, Icon::RowAdd, "Insert row below", &mut |e| drop(t::insert_row(e, true)));
            act(ui, Icon::ColumnAdd, "Insert column left", &mut |e| drop(t::insert_column(e, false)));
            act(ui, Icon::ColumnAdd, "Insert column right", &mut |e| drop(t::insert_column(e, true)));
            ui.separator();
            act(ui, Icon::DeleteRow, "Delete row", &mut |e| drop(t::delete_rows(e)));
            act(ui, Icon::DeleteColumn, "Delete column", &mut |e| drop(t::delete_columns(e)));
            act(ui, Icon::Delete, "Delete table", &mut |e| drop(t::delete_table(e)));
            ui.separator();
            act(ui, Icon::Merge, "Merge cells", &mut |e| drop(t::merge_cells(e)));
            act(ui, Icon::Split, "Split cell", &mut |e| drop(t::split_cell(e)));
            // Sorting moves whole rows: only offered when no cell spans rows.
            if sortable {
                act(ui, Icon::SortAz, "Sort rows by this column, A to Z", &mut |e| {
                    drop(t::sort_rows(e, table_id, col, false))
                });
                act(ui, Icon::SortZa, "Sort rows by this column, Z to A", &mut |e| {
                    drop(t::sort_rows(e, table_id, col, true))
                });
            }
            act(ui, Icon::Columns3, "Make all columns the same width", &mut |e| {
                drop(t::distribute_columns(e, table_id))
            });
            ui.separator();
        }
        if icons::item(ui, Icon::TableProperties, "Table properties…", "")
            .on_hover_text("Look, size and cell colours, with a preview")
            .clicked()
        {
            props = true;
            ui.close();
        }
        let ed = &mut self.editor;
        icons::submenu(ui, Icon::Settings, "Advanced", |ui| {
            icons::submenu(ui, Icon::Mail, "Repeat this row (mail merge)", |ui| {
                ui.weak("For mail merge: one copy of this row per item in a list, e.g. the lines of an invoice.");
                let mut lists: Vec<String> = vec!["rows".into()];
                if let Some(serde_json::Value::Object(o)) = &first_record {
                    for (k, v) in o {
                        if v.is_array() && !lists.contains(k) {
                            lists.push(k.clone());
                        }
                    }
                }
                for l in lists {
                    if ui.button(&l).clicked() {
                        let _ = t::set_row_repeat(ed, Some(l));
                        ui.close();
                    }
                }
                if ui.button("Don't repeat").clicked() {
                    let _ = t::set_row_repeat(ed, None);
                    ui.close();
                }
            });
        });
        if props {
            self.open_table_props();
        }
    }

    /// The menu highlighted after tapping Alt on its own (the menu bar has the keyboard).
    pub fn menu_armed(&self) -> Option<usize> {
        self.menu_keys.armed
    }

    /// Whether the caret is in a tracked change.
    fn on_change(&self) -> bool {
        let at = self.editor.sel.focus;
        xenify_wp_doc::review::changes(&self.editor.doc)
            .iter()
            .any(|c| c.para == at.para && c.range.start <= at.offset && at.offset <= c.range.end)
    }

    /// Format › Table › Table properties: the dialog, for the selected cells (or the caret's).
    pub fn open_table_props(&mut self) {
        use xenify_wp_doc::tables as t;
        let (id, rows, cols) = match t::selected_cells(&self.editor) {
            Some(x) => x,
            None => match t::cell_at(&self.editor, self.editor.sel.focus.para) {
                Some((id, r, c)) => (id, r..=r, c..=c),
                None => return,
            },
        };
        if let Some(table) = self.editor.doc.table(id).cloned() {
            self.dialog = Some(Dialog::TableProps(Box::new(crate::table_dialog::TableDialog::new(table, rows, cols))));
        }
    }

    /// Starts a new document from template `i` (after asking about unsaved changes).
    pub fn open_template(&mut self, i: usize) {
        self.run(Then::Template(i));
    }

    /// Adds a text box on the page the caret is on, near the caret.
    pub fn insert_text_box(&mut self) {
        let si = self.caret_section();
        let pages = &self.view.layout.pages;
        let caret_page = self.view.caret_page(&self.editor).unwrap_or(0);
        let first = pages.iter().position(|p| p.section == si).unwrap_or(0);
        let page = caret_page.saturating_sub(first) as u32;
        let ps = self.editor.doc.sections[si].page.clone();
        let top = pages.get(caret_page).map_or(0.0, |p| p.rect.y0);
        let caret_y =
            self.view.layout.caret(self.editor.sel.focus, false).map_or(ps.margins.top as f64, |c| c.y0 - top) as f32;
        let (w, h) = (200.0, 100.0);
        let frame = xenify_wp_doc::Frame {
            page,
            x: (ps.width - ps.margins.right - w).max(0.0),
            y: caret_y.clamp(ps.margins.top, (ps.height - ps.margins.bottom - h).max(ps.margins.top)),
            width: w,
            height: h,
            border: Some(xenify_wp_doc::Border::default()),
            ..Default::default()
        };
        if let Ok(id) = xenify_wp_doc::frames::insert_frame(&mut self.editor, si, frame) {
            self.view.selected_frame = Some(id);
        }
    }

    /// Properties of the selected box: a picture placed on the page, or a text box.
    fn frame_panel(&mut self, ctx: &egui::Context) {
        use crate::dialog::{self, Kind, Placement};
        let Some(id) = self.view.selected_frame else { return };
        let Some(orig) = self.editor.doc.frame(id).cloned() else {
            self.view.selected_frame = None;
            return;
        };
        let mut f = orig.clone();
        let mut open = true;
        let mut delete = false;
        let mut done = false;
        let mut inline_picture = false;
        let mut caption = None;
        let picture = orig.blocks.iter().find_map(|b| match b {
            xenify_wp_doc::Block::Image(i) => Some(i.id),
            _ => None,
        });
        let placed = self.view.layout.frames.iter().find(|p| p.id == id).cloned();
        let right = ctx.content_rect().right();
        let what = if picture.is_some() { "Picture" } else { "Text box" };
        egui::Window::new(what)
            .id(egui::Id::new("frame-panel"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos([right - 20.0, 70.0])
            .pivot(egui::Align2::RIGHT_TOP)
            .show(ctx, |ui| {
                ui.set_width(330.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.weak(format!("Drag the {} to move it, or a corner to resize it.", what.to_lowercase()));
                dialog::heading(ui, "Text around it");
                ui.horizontal_wrapped(|ui| {
                    for how in Placement::ALL {
                        // Only pictures can go back into the text.
                        if how == Placement::InLine && picture.is_none() {
                            continue;
                        }
                        let on = how == Placement::Wrap(f.wrap);
                        if dialog::picture_choice(ui, how.name(), how.hint(), on, egui::vec2(58.0, 50.0), |p, r| {
                            dialog::wrap_drawing(p, r, how)
                        }) {
                            match how {
                                Placement::InLine => inline_picture = true,
                                Placement::Wrap(w) => f.wrap = w,
                            }
                        }
                    }
                });
                if let Some(img) = picture {
                    dialog::heading(ui, "Caption");
                    if ui.button("Add a caption").on_hover_text("“Figure 1: …” under the picture").clicked() {
                        caption = Some(img);
                    }
                } else {
                    dialog::heading(ui, "Look");
                    ui.horizontal(|ui| {
                        let mut on = f.border.is_some();
                        if ui.checkbox(&mut on, "Line around").changed() {
                            f.border = on.then(xenify_wp_doc::Border::default);
                        }
                        if let Some(b) = f.border.as_mut() {
                            let mut c = Color32::from_rgb(b.color.0[0], b.color.0[1], b.color.0[2]);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                b.color = xenify_wp_doc::Rgba::rgb(c.r(), c.g(), c.b());
                            }
                        }
                        let mut on = f.fill.is_some();
                        if ui.checkbox(&mut on, "Background").changed() {
                            f.fill = on.then(|| xenify_wp_doc::Rgba::rgb(0xf1, 0xf3, 0xf4));
                        }
                        if let Some(fill) = f.fill.as_mut() {
                            let mut c = Color32::from_rgb(fill.0[0], fill.0[1], fill.0[2]);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                *fill = xenify_wp_doc::Rgba::rgb(c.r(), c.g(), c.b());
                            }
                        }
                    });
                    if placed.as_ref().is_some_and(|p| p.overflow) {
                        ui.colored_label(
                            Color32::from_rgb(0xd9, 0x30, 0x25),
                            "The text doesn't fit: make the box bigger, or let it continue in another box (Advanced).",
                        );
                    }
                }
                egui::CollapsingHeader::new("Advanced").id_salt("frame-advanced").show(ui, |ui| {
                    let pt = |ui: &mut egui::Ui, v: &mut f32, max: f32| {
                        ui.add(egui::DragValue::new(v).range(0.0..=max).speed(1.0).suffix(" pt"));
                    };
                    egui::Grid::new("frame-props").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("Width × height");
                        ui.horizontal(|ui| {
                            pt(ui, &mut f.width, 3000.0);
                            ui.label("×");
                            pt(ui, &mut f.height, 3000.0);
                        });
                        ui.end_row();
                        ui.label("From the page's left edge");
                        pt(ui, &mut f.x, 3000.0);
                        ui.end_row();
                        ui.label("From the page's top edge");
                        pt(ui, &mut f.y, 3000.0);
                        ui.end_row();
                        ui.label("On page (of its section)");
                        let mut page = f.page + 1;
                        ui.add(egui::DragValue::new(&mut page).range(1..=999));
                        f.page = page - 1;
                        ui.end_row();
                        ui.label("Space between it and the text");
                        pt(ui, &mut f.gap, 144.0);
                        ui.end_row();
                        if picture.is_none() {
                            ui.label("Space inside the box");
                            pt(ui, &mut f.padding, 144.0);
                            ui.end_row();
                            if let Some(b) = f.border.as_mut() {
                                ui.label("Line thickness");
                                ui.add(egui::DragValue::new(&mut b.width).range(0.25..=12.0).speed(0.05).suffix(" pt"));
                                ui.end_row();
                            }
                        }
                    });
                    if picture.is_none() {
                        ui.horizontal(|ui| {
                            if ui
                                .button("Continue the text in another box…")
                                .on_hover_text("Then click the box the text should continue in")
                                .clicked()
                            {
                                self.view.linking = Some(id);
                            }
                            if f.next.is_some() && ui.button("Stop continuing").clicked() {
                                f.next = None;
                            }
                        });
                        if self.view.linking == Some(id) {
                            ui.label("Click the box the text should continue in (Esc cancels).");
                        }
                    }
                });
                let delete_label = if picture.is_some() { "Delete picture" } else { "Delete text box" };
                match dialog::buttons(ui, &[(delete_label, Kind::Other), ("Done", Kind::Main)]) {
                    Some(0) => delete = true,
                    Some(_) => done = true,
                    None => {}
                }
            });
        if delete {
            let _ = xenify_wp_doc::frames::delete_frame(&mut self.editor, id);
            self.view.selected_frame = None;
            return;
        }
        if inline_picture {
            // Back before the paragraph the caret is in (when that's in the body).
            let before = Some(self.editor.sel.focus.para);
            if xenify_wp_doc::frames::inline_image(&mut self.editor, id, before).is_ok() {
                self.view.selected_frame = None;
                return;
            }
        }
        if f != orig {
            let _ = xenify_wp_doc::frames::update_frame(&mut self.editor, id, what, |x| *x = f.clone());
        }
        if let Some(img) = caption {
            if xenify_wp_doc::ops::add_caption(&mut self.editor, img).is_ok() {
                self.view.reveal();
            }
            done = true;
        }
        if !open || done {
            self.view.selected_frame = None;
            self.view.linking = None;
            self.refocus = true;
        }
    }

    /// Properties of the selected picture (in line with the text).
    fn image_panel(&mut self, ctx: &egui::Context) {
        use crate::dialog::{self, Kind, Placement};
        let Some(id) = self.view.selected_image else { return };
        let Some(xenify_wp_doc::Block::Image(img)) = self.editor.doc.block(id).cloned() else {
            self.view.selected_image = None;
            return;
        };
        let mut edit = img.clone();
        let mut open = true;
        let mut done = false;
        let mut delete = false;
        let mut caption = false;
        let mut float: Option<xenify_wp_doc::Wrap> = None;
        // Its size now (0 means "its own size", so ask the layout).
        let shown = self.view.layout.images.iter().find(|i| i.id == id).map(|i| i.rect);
        if edit.width <= 0.0
            && let Some(r) = shown
        {
            edit.width = r.width() as f32;
            edit.height = r.height() as f32;
        }
        let column = self.editor.doc.sections.get(self.caret_section()).map_or(450.0, |s| s.page.column_width());
        let right = ctx.content_rect().right();
        egui::Window::new("Picture")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos([right - 20.0, 70.0])
            .pivot(egui::Align2::RIGHT_TOP)
            .show(ctx, |ui| {
                ui.set_width(330.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.spacing_mut().slider_width = 170.0;
                let ratio = if edit.height > 0.0 { edit.width / edit.height } else { 1.0 };
                dialog::heading(ui, "Size");
                ui.weak("Drag a corner of the picture to resize it, or:");
                let mut pct = (edit.width / column * 100.0).round();
                if ui
                    .add(egui::Slider::new(&mut pct, 5.0..=100.0).fixed_decimals(0).suffix("% of the text width"))
                    .changed()
                {
                    edit.width = column * pct / 100.0;
                    edit.height = edit.width / ratio;
                }
                dialog::heading(ui, "Position");
                ui.horizontal(|ui| {
                    for (a, n) in [
                        (xenify_wp_doc::Align::Left, "Left"),
                        (xenify_wp_doc::Align::Center, "Centre"),
                        (xenify_wp_doc::Align::Right, "Right"),
                    ] {
                        if dialog::picture_choice(ui, n, "", edit.align == a, egui::vec2(58.0, 50.0), |p, r| {
                            dialog::align_drawing(p, r, a)
                        }) {
                            edit.align = a;
                        }
                    }
                });
                dialog::heading(ui, "Text around it");
                ui.horizontal_wrapped(|ui| {
                    for how in Placement::ALL {
                        let on = how == Placement::InLine;
                        if dialog::picture_choice(ui, how.name(), how.hint(), on, egui::vec2(58.0, 50.0), |p, r| {
                            dialog::wrap_drawing(p, r, how)
                        }) && let Placement::Wrap(w) = how
                        {
                            float = Some(w);
                        }
                    }
                });
                dialog::heading(ui, "Caption");
                if ui.button("Add a caption").on_hover_text("“Figure 1: …” under the picture").clicked() {
                    caption = true;
                }
                dialog::heading(ui, "Description");
                ui.add(
                    egui::TextEdit::singleline(&mut edit.alt)
                        .hint_text("What it shows, for people who can't see it")
                        .desired_width(f32::INFINITY),
                );
                egui::CollapsingHeader::new("Advanced").id_salt("picture-advanced").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Width");
                        if ui.add(egui::DragValue::new(&mut edit.width).range(8.0..=2000.0).suffix(" pt")).changed() {
                            edit.height = edit.width / ratio;
                        }
                        ui.label("Height");
                        if ui.add(egui::DragValue::new(&mut edit.height).range(8.0..=2000.0).suffix(" pt")).changed() {
                            edit.width = edit.height * ratio;
                        }
                    });
                    if ui.button("Back to its own size").clicked() {
                        edit.width = 0.0;
                        edit.height = 0.0;
                    }
                    ui.horizontal(|ui| {
                        let mut on = edit.border.is_some();
                        if ui.checkbox(&mut on, "Line around it").changed() {
                            edit.border = on.then(|| xenify_wp_doc::Border {
                                width: 1.0,
                                color: xenify_wp_doc::Rgba::rgb(0x40, 0x40, 0x40),
                            });
                        }
                        if let Some(b) = edit.border.as_mut() {
                            ui.add(egui::DragValue::new(&mut b.width).range(0.25..=12.0).speed(0.05).suffix(" pt"));
                            let mut c = Color32::from_rgb(b.color.0[0], b.color.0[1], b.color.0[2]);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                b.color = xenify_wp_doc::Rgba::rgb(c.r(), c.g(), c.b());
                            }
                        }
                    });
                    ui.weak(format!("File: {}", img.src));
                });
                match dialog::buttons(ui, &[("Delete picture", Kind::Other), ("Done", Kind::Main)]) {
                    Some(0) => delete = true,
                    Some(_) => done = true,
                    None => {}
                }
            });
        if delete {
            let _ = xenify_wp_doc::tables::delete_block(&mut self.editor, id);
            self.view.selected_image = None;
            return;
        }
        if let Some(wrap) = float {
            self.float_picture(id, wrap);
            return;
        }
        // Unchanged size: keep "its own size" (0) rather than writing numbers.
        let size_same = img.width <= 0.0
            && shown.is_some_and(|r| {
                (r.width() as f32 - edit.width).abs() < 0.5 && (r.height() as f32 - edit.height).abs() < 0.5
            });
        if size_same {
            edit.width = img.width;
            edit.height = img.height;
        }
        if edit != img {
            let _ = xenify_wp_doc::tables::replace_block(
                &mut self.editor,
                id,
                xenify_wp_doc::Block::Image(edit),
                "Picture",
            );
        }
        if caption {
            let _ = xenify_wp_doc::ops::add_caption(&mut self.editor, id);
            self.view.reveal();
            done = true;
        }
        if !open || done {
            self.view.selected_image = None;
            self.refocus = true;
        }
    }

    /// Moves picture `id` out of the text into a box at the same place on
    /// its page, with text wrapping as `wrap` says.
    fn float_picture(&mut self, id: xenify_wp_doc::NodeId, wrap: xenify_wp_doc::Wrap) {
        let layout = &self.view.layout;
        let Some(placed) = layout.images.iter().find(|i| i.id == id) else { return };
        let Some(pi) = layout.pages.iter().position(|p| p.rect.contains(placed.rect.center())) else { return };
        let page = &layout.pages[pi];
        let si = page.section;
        let first = layout.pages.iter().position(|p| p.section == si).unwrap_or(pi);
        let at = ((placed.rect.x0 - page.rect.x0) as f32, (placed.rect.y0 - page.rect.y0) as f32);
        if let Ok(frame) = xenify_wp_doc::frames::float_image(&mut self.editor, id, si, (pi - first) as u32, at, wrap) {
            self.view.selected_image = None;
            self.view.selected_frame = Some(frame);
        }
    }

    pub fn set_pageless(&mut self, on: bool) {
        if self.editor.doc.meta.pageless != on {
            self.editor.doc.meta.pageless = on;
            self.editor.doc.revision += 1;
        }
    }

    pub(crate) fn open_link_dialog(&mut self) {
        let ed = &self.editor;
        if ed.sel.is_collapsed() {
            // Select the word at the caret so there's something to link.
            self.editor.sel = ops::word_at(&self.editor, self.editor.sel.focus);
        }
        let url = ops::common_format(&self.editor).link.unwrap_or_default();
        self.dialog = Some(Dialog::Link(url));
    }

    fn start_dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .or_else(|| self.prefs.recent.first().and_then(|p| p.parent()).map(Path::to_path_buf))
            .unwrap_or_else(crate::file_browser::home)
    }

    fn remember(&mut self, path: &Path) {
        self.prefs.recent.retain(|p| p != path);
        self.prefs.recent.insert(0, path.to_path_buf());
        self.prefs.recent.truncate(10);
    }

    /// Runs `then` now, or asks first when there are unsaved changes.
    fn guard(&mut self, then: Then) {
        if self.editor.is_dirty() {
            self.dialog = Some(Dialog::Unsaved(then));
        } else {
            self.run(then);
        }
    }

    fn run(&mut self, then: Then) {
        match then {
            Then::New => {
                if !self.front_is_blank() {
                    self.open_in_new_tab(Document::new(), None, Format::Markdown);
                }
            }
            Then::Open => self.dialog = Some(Dialog::Open(FileBrowser::new(Mode::Open, &self.start_dir(), OPEN_EXT))),
            Then::OpenPath(p) => self.open_path(&p),
            Then::Template(i) => {
                if let Some(doc) = crate::templates::document(i) {
                    self.place_document(doc, None, Format::Markdown);
                }
            }
            Then::CloseTab => self.close_front_tab(),
            Then::Quit => {
                // This document is answered for (saved, or let go); ask about the next.
                self.quit_ok.insert(self.tabs.active);
                self.ask_quit_next();
            }
        }
    }

    /// Notes the open documents (with files) for next time.
    fn remember_session(&mut self) {
        let mut files = Vec::new();
        let mut front = 0;
        // Every window's documents (they all reopen in one window).
        let keys: Vec<u64> =
            self.tabs.order.iter().copied().chain(self.windows.iter().flat_map(|w| w.keys())).collect();
        for &k in &keys {
            let path = if k == self.tabs.active {
                self.path.clone()
            } else {
                self.tabs.others.iter().find(|t| t.key == k).and_then(|t| t.path.clone())
            };
            if let Some(p) = path {
                if k == self.tabs.active {
                    front = files.len();
                }
                files.push(p);
            }
        }
        self.prefs.open_files = files;
        self.prefs.open_front = front;
    }

    /// At start: opens again the documents open last time (those still there).
    pub fn reopen_session(&mut self) {
        if self.prefs.start_blank {
            return;
        }
        let files: Vec<PathBuf> = self.prefs.open_files.iter().filter(|p| p.exists()).cloned().collect();
        let front = files.get(self.prefs.open_front).cloned();
        for f in &files {
            self.open_path(f);
        }
        if let Some(f) = front {
            self.open_path(&f);
        }
    }

    /// File › Quit: asks about each document with unsaved changes in turn.
    fn ask_quit(&mut self) {
        self.quit_ok.clear();
        self.gather_windows();
        self.ask_quit_next();
    }

    fn ask_quit_next(&mut self) {
        let next = self.unsaved_tabs().into_iter().find(|k| !self.quit_ok.contains(k));
        match next {
            Some(k) => {
                self.switch_to(k);
                self.dialog = Some(Dialog::Unsaved(Then::Quit));
            }
            None => {
                self.remember_session();
                self.quitting = true;
            }
        }
    }

    /// File › Close (Ctrl+W): asks first when it has unsaved changes.
    pub fn close_tab_asking(&mut self) {
        self.guard(Then::CloseTab);
    }

    /// A document just opened or made: in a new tab, or in place of an
    /// untouched blank one.
    fn place_document(&mut self, doc: Document, path: Option<PathBuf>, format: Format) {
        if self.front_is_blank() {
            self.set_document(doc, path, format);
        } else {
            self.open_in_new_tab(doc, path, format);
        }
    }

    /// Edit › Find / Replace: the bar, searching for a short selection.
    pub fn open_find(&mut self, replace: bool) {
        let (a, b) = self.editor.doc.ordered(self.editor.sel);
        let selected = (a.para == b.para && a != b)
            .then(|| self.editor.doc.para(a.para).and_then(|p| p.text.get(a.offset..b.offset)).map(str::to_string))
            .flatten();
        self.find.show(replace, selected);
    }

    /// Edit › Go to page (not in pageless view, which has no pages).
    pub fn open_go_to(&mut self) {
        if !self.editor.doc.meta.pageless {
            self.dialog = Some(Dialog::GoTo(String::new()));
        }
    }

    /// File › Settings, on `section`.
    pub fn open_settings(&mut self, section: crate::settings::Section) {
        self.settings.open = true;
        self.settings.section = section;
    }

    /// File › Record a change: with a summary of what changed since the
    /// file was last saved (`then_save`: save once it's recorded).
    pub fn open_record_change(&mut self, then_save: bool) {
        let saved = self
            .path
            .as_ref()
            .and_then(|p| {
                let format = Format::from_name(&p.to_string_lossy())?;
                xenify_wp_io::load(&std::fs::read(p).ok()?, format).ok()
            })
            .map(|l| l.doc)
            .unwrap_or_else(|| Document::from_plain_text(""));
        let summary = xenify_wp_doc::changelog::summarize(&saved, &self.editor.doc);
        self.dialog = Some(Dialog::RecordChange(summary, then_save));
    }

    /// Takes back unsaved work an earlier run left (`as_changes`: shown as
    /// tracked changes against the saved file, to accept or reject).
    fn recover(&mut self, info: &crate::recovery::Info, doc: Document, untitled: Option<String>, as_changes: bool) {
        let when = crate::recovery::when(info.time);
        match (&info.path, untitled) {
            (Some(path), _) => {
                let content = if as_changes {
                    let mut c =
                        xenify_wp_doc::compare::compare(&self.editor.doc, &doc, &format!("Unsaved work from {when}"));
                    c.assets.extend(doc.assets.clone());
                    c
                } else {
                    doc
                };
                let _ = xenify_wp_doc::restore(&mut self.editor, content);
                self.recovery.forget(Some(path), None);
                self.recovery.bind(Some(path));
                let _ = self.recovery.write(&self.editor.doc);
                self.message = Some((
                    format!(
                        "Recovered the unsaved work from {when}. Save to keep it (Ctrl+Z goes back to the saved file)"
                    ),
                    false,
                ));
            }
            (None, Some(key)) => {
                self.set_document(doc, None, Format::Markdown);
                self.editor.doc.revision += 1;
                self.recovery.forget(None, Some(&key));
                let _ = self.recovery.write(&self.editor.doc);
                self.message = Some((format!("Recovered an unsaved document from {when}"), false));
            }
            (None, None) => {}
        }
    }

    /// Offers back unsaved work left behind for the document just opened
    /// (or, at start, for documents never saved).
    fn offer_recovery(&mut self) {
        let left = match self.path.clone() {
            Some(p) => self.recovery.left_for(&p).map(|(i, d)| (i, d, None)),
            None if self.editor.doc.paragraphs().all(|p| p.text.is_empty()) => {
                self.recovery.orphans().into_iter().next().map(|(i, d, k)| (i, d, Some(k)))
            }
            None => None,
        };
        if let Some((info, doc, key)) = left {
            self.dialog = Some(Dialog::Recover(Box::new((info, doc, key))));
        }
    }

    pub(crate) fn set_document(&mut self, doc: Document, path: Option<PathBuf>, format: Format) {
        // Open comments or tracked changes: show them.
        let review = doc.meta.comments.iter().any(|c| !c.resolved) || !xenify_wp_doc::review::changes(&doc).is_empty();
        self.editor = Editor::new(doc);
        self.review.show |= review;
        self.path = path;
        self.format = format;
        self.accepted_losses = None;
        self.view.reset();
        self.view.base_dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
        self.merge.preview = None;
        crate::merge_panel::load(self);
        // Unsaved work an earlier run left for this document?
        let path = self.path.clone();
        self.recovery.bind(path.as_deref());
        self.offer_recovery();
    }

    pub fn open_path(&mut self, path: &Path) {
        // Already open: its tab comes to the front.
        let same = |p: &Path| p == path || (p.canonicalize().ok().is_some_and(|c| path.canonicalize().ok() == Some(c)));
        if self.path.as_deref().is_some_and(same) {
            return;
        }
        if let Some(k) = self.tabs.others.iter().find(|t| t.path.as_deref().is_some_and(same)).map(|t| t.key) {
            self.switch_to(k);
            return;
        }
        let Some(format) = Format::from_name(&path.to_string_lossy()) else {
            self.message = Some((format!("{} isn't a file type Xenify Write can open", path.display()), true));
            return;
        };
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| xenify_wp_io::load(&b, format).map_err(|e| e.to_string()));
        match result {
            Ok(mut loaded) => {
                // Pictures a Markdown file refers to are read in, so they travel with the document.
                if format == Format::Markdown {
                    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
                    xenify_wp_io::attach_images(&mut loaded.doc, &|src| std::fs::read(dir.join(src)).ok());
                }
                if loaded.warnings.len() > 1 {
                    self.dialog = Some(Dialog::Notice("Opened with changes".into(), loaded.warnings.clone()));
                }
                self.place_document(loaded.doc, Some(path.to_path_buf()), format);
                self.remember(path);
                self.message = loaded.warnings.first().map(|w| (w.clone(), false));
            }
            Err(e) => self.message = Some((format!("Couldn't open {}: {e}", path.display()), true)),
        }
    }

    /// Saves to `path` in the format its extension names.
    pub fn save_to(&mut self, path: &Path) -> bool {
        let format = Format::from_name(&path.to_string_lossy()).unwrap_or(Format::Igdoc);
        let result = xenify_wp_io::save(&self.editor.doc, format)
            .map_err(|e| e.to_string())
            .and_then(|bytes| write_atomically(path, &bytes).map_err(|e| e.to_string()))
            .and_then(|()| {
                // Markdown keeps pictures as files next to the document.
                if format != Format::Markdown {
                    return Ok(());
                }
                let dir = path.parent().unwrap_or(Path::new("."));
                for src in xenify_wp_io::image_sources(&self.editor.doc) {
                    let Some(bytes) = self.editor.doc.assets.get(&src) else { continue };
                    let target = dir.join(&src);
                    if std::fs::read(&target).is_ok_and(|b| b == **bytes) {
                        continue;
                    }
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&target, bytes.as_slice()).map_err(|e| format!("{}: {e}", target.display()))?;
                }
                Ok(())
            });
        match result {
            Ok(()) => {
                self.editor.mark_saved();
                // Saved: the autosave of the work goes (and follows a new name).
                self.recovery.clear();
                self.recovery.bind(Some(path));
                self.recovery.clear();
                self.path = Some(path.to_path_buf());
                self.view.base_dir = path.parent().map(Path::to_path_buf);
                self.format = format;
                self.remember(path);
                self.message = Some((format!("Saved {}", path.display()), false));
                true
            }
            Err(e) => {
                self.message = Some((format!("Couldn't save {}: {e}", path.display()), true));
                false
            }
        }
    }

    /// Saves to `path`, first asking when its format can't hold everything
    /// in the document. Returns true when saved now.
    pub fn request_save(&mut self, path: &Path) -> bool {
        let format = Format::from_name(&path.to_string_lossy()).unwrap_or(Format::Markdown);
        let losses = xenify_wp_io::losses(&self.editor.doc, format);
        if !losses.is_empty() && self.accepted_losses != Some((path.to_path_buf(), losses.clone())) {
            self.dialog = Some(Dialog::Losses { path: path.to_path_buf(), losses });
            return false;
        }
        self.save_to(path)
    }

    /// File › Save (asks for a change-log note first when that's turned on).
    pub fn save(&mut self) {
        if self.prefs.ask_change_note && self.path.is_some() && self.editor.is_dirty() {
            self.open_record_change(true);
            return;
        }
        self.save_now();
    }

    /// Saves without asking for a note.
    pub fn save_now(&mut self) {
        match self.path.clone() {
            Some(p) => {
                self.request_save(&p);
            }
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let mut b = FileBrowser::new(Mode::Save, &self.start_dir(), SAVE_EXT);
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned());
        b.name = format!("{stem}.{}", self.format.extension());
        self.dialog = Some(Dialog::SaveAs(b));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let sc = |m, k| KeyboardShortcut::new(m, k);
        // First: Ctrl+Shift+E would otherwise match Ctrl+E (centre).
        let alt = ctx.input(|i| i.modifiers.alt);
        if !alt && ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::F))) {
            self.open_find(false);
        }
        if !alt && ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::H))) {
            self.open_find(true);
        }
        if !alt && ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::G))) {
            self.open_go_to();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::SHIFT, Key::F7))) {
            crate::dictionary::open_at_caret(self);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Period))) {
            self.symbols.show();
        }
        // Review: new comment, track changes.
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::ALT, Key::M))) {
            self.review.start_comment(&self.editor);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::E))) {
            let on = !self.editor.doc.meta.track_changes;
            let _ = xenify_wp_doc::review::set_tracking(&mut self.editor, on);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::S))) {
            self.save_as();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::S))) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::O))) {
            self.run(Then::Open);
        }
        // Tabs: new, close, next and previous (Shift first: Ctrl+Tab would match it).
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Backslash))) {
            if self.split.is_some() { self.close_split() } else { self.open_split(false) }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::ALT, Key::Equals))) {
            self.insert_equation();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::T))) {
            self.open_in_new_tab(Document::new(), None, Format::Markdown);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::W))) {
            self.close_tab_asking();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab))) {
            self.cycle_tab(false);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Tab))) {
            self.cycle_tab(true);
        }
        // As in a web browser: Ctrl+PageUp/PageDown, and Ctrl+1…8 for that tab (9 for the last).
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::PageUp))) {
            self.cycle_tab(false);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::PageDown))) {
            self.cycle_tab(true);
        }
        let digits =
            [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        // (Not with Shift: Ctrl+Shift+8 is bullets.)
        let shift = ctx.input(|i| i.modifiers.shift);
        for (n, key) in digits.into_iter().enumerate() {
            if !alt && !shift && ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, key))) {
                let order = &self.tabs.order;
                let pick = if n == 8 { order.last() } else { order.get(n) };
                if let Some(&k) = pick {
                    self.switch_to(k);
                }
            }
        }
        // Ctrl+Shift+N before Ctrl+N (which would also match it).
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::N))) {
            self.nav.show = !self.nav.show;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::N))) {
            self.run(Then::New);
        }
        // Undo/redo work everywhere except in text fields (which have their own).
        let focus = ctx.memory(|m| m.focused());
        if focus.is_none() || focus == Some(self.view.id) || !ctx.egui_wants_keyboard_input() {
            if ctx.input_mut(|i| {
                i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
                    || i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Y))
            }) {
                self.editor.redo();
                self.view.extra.clear();
            }
            if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Z))) {
                self.editor.undo();
                self.view.extra.clear();
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter)))
            && !self.assistant.busy()
        {
            self.show_ai = true;
            crate::assistant::continue_writing(self, ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::P))) {
            self.open_print();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::K))) {
            self.open_link_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Backslash))) {
            self.clear_formatting();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Num8))) {
            let _ = ops::toggle_list(&mut self.editor, xenify_wp_doc::ListDef::BULLETS);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Num7))) {
            let _ = ops::toggle_list(&mut self.editor, xenify_wp_doc::ListDef::NUMBERS);
        }
        for (key, a) in [
            (Key::L, xenify_wp_doc::Align::Left),
            (Key::E, xenify_wp_doc::Align::Center),
            (Key::R, xenify_wp_doc::Align::Right),
            (Key::J, xenify_wp_doc::Align::Justify),
        ] {
            if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, key))) {
                self.set_para(xenify_wp_doc::ParaProps { align: Some(a), ..Default::default() }, "Align");
            }
        }
        for (key, bigger) in [(Key::OpenBracket, false), (Key::CloseBracket, true)] {
            if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, key))) {
                let now = self.effective_char().size;
                let sizes = crate::toolbar::SIZES;
                let size = if bigger {
                    sizes.iter().copied().find(|s| *s > now)
                } else {
                    sizes.iter().rev().copied().find(|s| *s < now)
                };
                if let Some(size) = size {
                    self.set_char(xenify_wp_doc::CharFmt { size: Some(size), ..Default::default() }, "Font size", None);
                }
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Q))) {
            self.ask_quit();
        }
        // Superscript (Ctrl+.) and subscript (Ctrl+,), as in Google Docs.
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Period))) {
            self.view.toggle(&mut self.editor, crate::view::Toggle::Superscript);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Comma))) {
            self.view.toggle(&mut self.editor, crate::view::Toggle::Subscript);
        }
        // Footnote and endnote.
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::ALT, Key::F))) {
            self.insert_note(xenify_wp_doc::NoteKind::Foot);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::ALT, Key::D))) {
            self.insert_note(xenify_wp_doc::NoteKind::End);
        }
        // Paragraph styles: Ctrl+Alt+0 Normal, Ctrl+Alt+1…6 headings.
        let digits = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6];
        for (n, key) in digits.into_iter().enumerate() {
            if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::ALT, key))) {
                let style = if n == 0 {
                    xenify_wp_doc::StyleSheet::NORMAL.to_string()
                } else {
                    xenify_wp_doc::StyleSheet::heading(n as u8)
                };
                self.editor.seal();
                let _ = ops::set_style(&mut self.editor, &style);
            }
        }
        let zoom = |z: f32| (z * 100.0).round() / 100.0;
        if ctx.input_mut(|i| {
            i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Plus))
                || i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Equals))
        }) {
            self.view.zoom = zoom((self.view.zoom * 1.1).min(4.0));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Minus))) {
            self.view.zoom = zoom((self.view.zoom / 1.1).max(0.25));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Num0))) {
            self.view.zoom = 1.0;
        }
    }

    /// Format › Columns: how many text columns the caret's section has, the
    /// gap between them and whether a line runs down each gap.
    fn columns_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        let si = self.caret_section();
        let Some(sec) = self.editor.doc.sections.get(si) else { return };
        let (start, mut page) = (sec.start, sec.page.clone());
        let before = page.clone();
        for (n, icon, name) in [(1u8, Icon::Document, "One"), (2, Icon::Columns2, "Two"), (3, Icon::Columns3, "Three")]
        {
            if icons::check_item(ui, icon, name, "", page.columns == n).clicked() {
                page.columns = n;
            }
        }
        ui.separator();
        if icons::check_item(ui, Icon::Rule, "Line between columns", "", page.column_line).clicked() {
            page.column_line = !page.column_line;
        }
        if page != before {
            let _ = ops::set_section(&mut self.editor, si, start, page);
        }
        if icons::item(ui, Icon::Settings, "More columns…", "")
            .on_hover_text("Number of columns and the space between them")
            .clicked()
        {
            self.open_page_setup();
            if let Some(Dialog::PageSetup(d)) = &mut self.dialog {
                d.show_layout();
            }
            ui.close();
        }
    }

    /// A file dropped on the window at `at` (screen position): a picture is
    /// inserted there; a document opens.
    pub fn drop_file(&mut self, path: &Path, at: Option<egui::Pos2>) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if crate::images::IMAGE_EXT.contains(&ext.as_str()) {
            // Put the caret where the picture was dropped.
            if let Some(p) = at {
                let d = (p - self.view.doc_origin) / self.view.scale();
                if let Some(pos) = self.view.layout.hit(kurbo::Point::new(d.x as f64, d.y as f64)) {
                    self.editor.sel = xenify_wp_doc::Selection::caret(pos);
                }
            }
            match crate::images::embed(&mut self.editor.doc, path) {
                Ok((key, w, h)) => {
                    let _ = xenify_wp_doc::tables::insert_image(&mut self.editor, &key, w, h);
                }
                Err(e) => self.message = Some((e, true)),
            }
        } else if Format::from_name(&path.to_string_lossy()).is_some_and(|f| f.can_open()) {
            self.run(Then::OpenPath(path.to_path_buf()));
        } else {
            self.message = Some((format!("Xenify Write can't use {}", path.display()), true));
        }
    }

    /// Format › Borders and shading: lines around and colour behind the
    /// selected paragraphs. Changes apply straight away.
    fn borders_window(&mut self, ctx: &egui::Context) {
        if !self.show_borders {
            return;
        }
        let Some(r) = self.current_para_props() else { return };
        let mut open = true;
        let mut patch: Option<xenify_wp_doc::ParaProps> = None;
        let mut done = false;
        crate::dialog::window(ctx, "Borders and shading", &mut open).show(ctx, |ui| {
            let b = r.borders;
            let line = [b.top, b.bottom, b.left, b.right]
                .into_iter()
                .flatten()
                .next()
                .unwrap_or(xenify_wp_doc::Border { width: 0.75, color: xenify_wp_doc::Rgba::rgb(0x40, 0x40, 0x40) });
            crate::dialog::heading(ui, "Lines around the paragraph");
            ui.horizontal_wrapped(|ui| {
                let none = xenify_wp_doc::ParaBorders::default();
                let presets = [
                    ("None", none),
                    ("Box", xenify_wp_doc::ParaBorders::all(line)),
                    ("Top", xenify_wp_doc::ParaBorders { top: Some(line), ..none }),
                    ("Bottom", xenify_wp_doc::ParaBorders { bottom: Some(line), ..none }),
                    ("Top and bottom", xenify_wp_doc::ParaBorders { top: Some(line), bottom: Some(line), ..none }),
                    (
                        "Left bar",
                        xenify_wp_doc::ParaBorders {
                            left: Some(xenify_wp_doc::Border { width: line.width.max(3.0), ..line }),
                            ..none
                        },
                    ),
                ];
                for (name, preset) in presets {
                    let draw = |p: &egui::Painter, r: egui::Rect| {
                        // A paragraph (grey lines) with the preset's lines around it.
                        let text = r.shrink2(egui::vec2(8.0, 9.0));
                        crate::dialog::text_lines(p, text, None);
                        let ink = egui::Stroke::new(1.5, Color32::from_gray(0x30));
                        let box_ = r.shrink2(egui::vec2(4.0, 5.0));
                        if preset.top.is_some() {
                            p.line_segment([box_.left_top(), box_.right_top()], ink);
                        }
                        if preset.bottom.is_some() {
                            p.line_segment([box_.left_bottom(), box_.right_bottom()], ink);
                        }
                        if let Some(l) = preset.left {
                            p.line_segment(
                                [box_.left_top(), box_.left_bottom()],
                                egui::Stroke::new(l.width.min(4.0), Color32::from_gray(0x30)),
                            );
                        }
                        if preset.right.is_some() {
                            p.line_segment([box_.right_top(), box_.right_bottom()], ink);
                        }
                    };
                    if crate::dialog::picture_choice(ui, name, "", b == preset, egui::vec2(64.0, 48.0), draw) {
                        patch = Some(xenify_wp_doc::ParaProps { borders: Some(preset), ..Default::default() });
                    }
                }
            });
            if !b.is_empty() {
                let mut l = line;
                ui.horizontal(|ui| {
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut l.width).range(0.25..=12.0).speed(0.05).suffix(" pt"));
                    ui.label("Colour");
                    let mut c = Color32::from_rgb(l.color.0[0], l.color.0[1], l.color.0[2]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        l.color = xenify_wp_doc::Rgba::rgb(c.r(), c.g(), c.b());
                    }
                });
                if l != line {
                    let set = |s: Option<xenify_wp_doc::Border>| s.map(|_| l);
                    let nb = xenify_wp_doc::ParaBorders {
                        top: set(b.top),
                        bottom: set(b.bottom),
                        left: set(b.left),
                        right: set(b.right),
                    };
                    patch = Some(xenify_wp_doc::ParaProps { borders: Some(nb), ..Default::default() });
                }
            }
            crate::dialog::heading(ui, "Shading behind it");
            if let Some(c) = swatches(ui, &PAGE_COLOURS, r.shading) {
                patch = Some(xenify_wp_doc::ParaProps { shading: Some(c), ..Default::default() });
            }
            if ui.button("No shading").clicked() {
                patch = Some(xenify_wp_doc::ParaProps {
                    shading: Some(xenify_wp_doc::Rgba([0, 0, 0, 0])),
                    ..Default::default()
                });
            }
            done = crate::dialog::buttons(ui, &[("Done", crate::dialog::Kind::Main)]).is_some()
                || crate::dialog::escape(ui.ctx());
        });
        open &= !done;
        if let Some(p) = patch {
            let _ = ops::apply_para_props_merged(
                &mut self.editor,
                &p,
                "Borders and shading",
                Some(xenify_wp_doc::MergeKey::Control("borders".into())),
            );
        }
        if !open {
            self.show_borders = false;
            self.editor.seal();
        }
    }

    /// A small toolbar above the table the caret is in.
    fn table_toolbar(&mut self, ctx: &egui::Context) {
        use crate::icons::{self, Icon};
        use xenify_wp_doc::tables as t;
        if self.dialog.is_some() || self.view.read_only {
            return;
        }
        let Some((id, _, col)) = t::cell_at(&self.editor, self.editor.sel.focus.para) else { return };
        let Some(caret) = self.view.layout.caret(self.editor.sel.focus, false) else { return };
        let Some(page) = self.view.layout.pages.iter().find(|p| p.rect.contains(caret.center())).map(|p| p.rect) else {
            return;
        };
        let cells =
            self.view.layout.cells.iter().filter(|c| c.table == id && !c.repeat && page.contains(c.rect.center()));
        let Some(bbox) = cells.map(|c| c.rect).reduce(|a, b| a.union(b)) else { return };
        let s = self.view.scale();
        let o = self.view.doc_origin;
        let vis = self.view.visible;
        if !vis.is_positive() || o.y + bbox.y1 as f32 * s < vis.top() || o.y + (bbox.y0 as f32) * s > vis.bottom() {
            return;
        }
        // Beside the page when there's room (it never covers the text);
        // otherwise a compact bar above the table.
        let page_right = o.x + page.x1 as f32 * s;
        let beside = vis.right() - page_right >= 176.0;
        let pos = if beside {
            egui::pos2(
                page_right + 12.0,
                (o.y + bbox.y0 as f32 * s).clamp(vis.top() + 4.0, (vis.bottom() - 360.0).max(vis.top() + 4.0)),
            )
        } else {
            egui::pos2(
                (o.x + bbox.x0 as f32 * s).max(vis.left() + 4.0),
                (o.y + bbox.y0 as f32 * s - 52.0).clamp(vis.top() + 4.0, (vis.bottom() - 40.0).max(vis.top() + 4.0)),
            )
        };
        let mut open_props = false;
        let sortable = t::can_sort(&self.editor.doc, id);
        egui::Area::new(egui::Id::new("table-toolbar")).fixed_pos(pos).order(egui::Order::Middle).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                let layout = if beside {
                    egui::Layout::top_down(egui::Align::Min)
                } else {
                    egui::Layout::left_to_right(egui::Align::Center)
                };
                ui.with_layout(layout, |ui| {
                    ui.spacing_mut().item_spacing = if beside { egui::vec2(0.0, 2.0) } else { egui::vec2(1.0, 0.0) };
                    if beside {
                        ui.set_width(150.0);
                        ui.label(egui::RichText::new("Table").strong());
                    }
                    let ed = &mut self.editor;
                    // In the palette: icon and words; in the bar: the icon (words on hover).
                    let mut b = |ui: &mut egui::Ui, icon: Icon, name: &str, f: &mut dyn FnMut(&mut Editor)| {
                        let clicked = if beside {
                            let w = ui.available_width().max(140.0);
                            ui.add(
                                egui::Button::image_and_text(icons::image(ui, icon, icons::MENU), name)
                                    .frame_when_inactive(false)
                                    .min_size(egui::vec2(w, 26.0)),
                            )
                            .clicked()
                        } else {
                            icons::button(ui, icon, name, name).clicked()
                        };
                        if clicked {
                            f(&mut *ed);
                        }
                    };
                    b(ui, Icon::RowAdd, "Row below", &mut |e| drop(t::insert_row(e, true)));
                    b(ui, Icon::ColumnAdd, "Column right", &mut |e| drop(t::insert_column(e, true)));
                    b(ui, Icon::DeleteRow, "Delete row", &mut |e| drop(t::delete_rows(e)));
                    b(ui, Icon::DeleteColumn, "Delete column", &mut |e| drop(t::delete_columns(e)));
                    if beside {
                        ui.separator();
                    }
                    b(ui, Icon::Merge, "Merge cells", &mut |e| drop(t::merge_cells(e)));
                    b(ui, Icon::Split, "Split cell", &mut |e| drop(t::split_cell(e)));
                    // Sorting moves whole rows: only when no cell spans rows.
                    if sortable {
                        b(ui, Icon::SortAz, "Sort A to Z", &mut |e| drop(t::sort_rows(e, id, col, false)));
                        b(ui, Icon::SortZa, "Sort Z to A", &mut |e| drop(t::sort_rows(e, id, col, true)));
                    }
                    if beside {
                        ui.separator();
                    }
                    let fill = if beside {
                        egui::Button::image_and_text(icons::image(ui, Icon::Fill, icons::MENU), "Cell colour")
                            .frame_when_inactive(false)
                            .min_size(egui::vec2(150.0, 26.0))
                    } else {
                        egui::Button::image(icons::image(ui, Icon::Fill, icons::TOOL)).frame_when_inactive(false)
                    };
                    egui::containers::menu::MenuButton::from_button(fill)
                        .ui(ui, |ui| {
                            if let Some(c) = swatches(ui, &CELL_COLOURS, None) {
                                let _ = t::set_shading(ed, Some(c));
                                ui.close();
                            }
                            // Any colour, with the picker.
                            let key = egui::Id::new("cell-custom-colour");
                            let mut custom: egui::Color32 =
                                ui.data(|d| d.get_temp(key)).unwrap_or(egui::Color32::from_rgb(0xff, 0xf2, 0xcc));
                            ui.horizontal(|ui| {
                                ui.label("Your own:");
                                if ui.color_edit_button_srgba(&mut custom).changed() {
                                    ui.data_mut(|d| d.insert_temp(key, custom));
                                }
                                if ui.button("Use it").clicked() {
                                    let _ = t::set_shading(
                                        ed,
                                        Some(xenify_wp_doc::Rgba::rgb(custom.r(), custom.g(), custom.b())),
                                    );
                                    ui.close();
                                }
                            });
                            if ui.button("No colour").clicked() {
                                let _ = t::set_shading(ed, None);
                                ui.close();
                            }
                        })
                        .0
                        .on_hover_text("Cell colour");
                    let style = if beside {
                        egui::Button::image_and_text(icons::image(ui, Icon::Styles, icons::MENU), "Table style")
                            .frame_when_inactive(false)
                            .min_size(egui::vec2(150.0, 26.0))
                    } else {
                        egui::Button::image(icons::image(ui, Icon::Styles, icons::TOOL)).frame_when_inactive(false)
                    };
                    egui::containers::menu::MenuButton::from_button(style)
                        .ui(ui, |ui| {
                            for st in t::TableStyle::ALL {
                                if ui.button(st.name()).clicked() {
                                    let _ = t::apply_style(ed, id, st);
                                    ui.close();
                                }
                            }
                        })
                        .0
                        .on_hover_text("Table style");
                    let props = if beside {
                        egui::Button::image_and_text(
                            icons::image(ui, Icon::TableProperties, icons::MENU),
                            "Properties…",
                        )
                        .frame_when_inactive(false)
                        .min_size(egui::vec2(150.0, 26.0))
                    } else {
                        egui::Button::image(icons::image(ui, Icon::TableProperties, icons::TOOL))
                            .frame_when_inactive(false)
                    };
                    if ui.add(props).on_hover_text("Table properties: look, size, cells").clicked() {
                        open_props = true;
                    }
                    let more = if beside {
                        egui::Button::image_and_text(icons::image(ui, Icon::More, icons::MENU), "More…")
                            .frame_when_inactive(false)
                            .min_size(egui::vec2(150.0, 26.0))
                    } else {
                        egui::Button::image(icons::image(ui, Icon::More, 14.0)).frame_when_inactive(false)
                    };
                    egui::containers::menu::MenuButton::from_button(more)
                        .ui(ui, |ui| self.table_menu(ui))
                        .0
                        .on_hover_text("All table actions");
                });
            });
        });
        if open_props {
            self.open_table_props();
        }
    }

    /// The right-click menu: table actions in a table, editing elsewhere.
    fn document_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        // Synonyms and look-up for the word right-clicked.
        if let Some(pos) = self.menu_pos {
            crate::dictionary::context_items(self, ui, pos);
        }
        let has_sel = !self.editor.sel.is_collapsed();
        if icons::item_enabled(ui, has_sel, Icon::Cut, "Cut", "Ctrl+X").clicked() {
            self.view.copy(ui.ctx(), &mut self.editor, true);
            ui.close();
        }
        if icons::item_enabled(ui, has_sel, Icon::Copy, "Copy", "Ctrl+C").clicked() {
            self.view.copy(ui.ctx(), &mut self.editor, false);
            ui.close();
        }
        if icons::item(ui, Icon::Paste, "Paste", "Ctrl+V").clicked() {
            self.view.paste_from_clipboard(&mut self.editor, false);
            ui.close();
        }
        if icons::item(ui, Icon::PastePlain, "Paste as plain text", "Ctrl+Shift+V").clicked() {
            self.view.paste_from_clipboard(&mut self.editor, true);
            ui.close();
        }
        ui.separator();
        if icons::item(ui, Icon::CommentAdd, "New comment", "Ctrl+Alt+M").clicked() {
            self.review.start_comment(&self.editor);
            ui.close();
        }
        // On a tracked change: accept or reject it.
        if self.on_change() {
            if icons::item(ui, Icon::Check, "Accept change", "").clicked() {
                let _ = xenify_wp_doc::review::decide(&mut self.editor, true, false);
                ui.close();
            }
            if icons::item(ui, Icon::Close, "Reject change", "").clicked() {
                let _ = xenify_wp_doc::review::decide(&mut self.editor, false, false);
                ui.close();
            }
        }
        ui.separator();
        if xenify_wp_doc::tables::cell_at(&self.editor, self.editor.sel.focus.para).is_some() {
            self.table_menu(ui);
            return;
        }
        if icons::item(ui, Icon::Link, "Link…", "Ctrl+K").clicked() {
            self.open_link_dialog();
            ui.close();
        }
        if icons::item(ui, Icon::Pilcrow, "Paragraph…", "").clicked() {
            self.open_paragraph_dialog();
            ui.close();
        }
        if icons::item(ui, Icon::ClearFormat, "Clear formatting", "Ctrl+\\").clicked() {
            self.clear_formatting();
            ui.close();
        }
    }

    pub fn open_paragraph_dialog(&mut self) {
        if let Some(r) = self.current_para_props() {
            self.dialog = Some(Dialog::Paragraph(crate::paragraph_dialog::ParagraphDialog::new(&r)));
        }
    }

    /// Insert › Header / Footer: edits it on the caret's page.
    pub fn edit_header(&mut self, footer: bool) {
        if self.editor.doc.meta.pageless {
            self.message = Some(("Headers and footers show on pages: turn off View › Pageless.".into(), false));
            return;
        }
        let page = self.view.caret_page(&self.editor).unwrap_or(0);
        let Some(pg) = self.view.layout.pages.get(page).cloned() else { return };
        if xenify_wp_doc::hf::edit_on_page(&mut self.editor, pg.section, pg.number, pg.first_of_section, footer).is_ok()
        {
            self.view.hf_edit = Some(page);
            self.view.reveal();
        }
    }

    /// References › Insert footnote / endnote: the caret moves into the new note.
    pub fn insert_note(&mut self, kind: xenify_wp_doc::NoteKind) {
        if xenify_wp_doc::notes::insert_note(&mut self.editor, kind).is_err() {
            self.message = Some(("Notes can't go inside another note.".into(), true));
        }
    }

    pub fn open_sources(&mut self) {
        self.dialog = Some(Dialog::Sources(Box::new(crate::references::SourcesDialog::new(&self.editor.doc))));
    }

    pub fn open_cite(&mut self) {
        self.dialog = Some(Dialog::Cite(crate::references::CiteDialog::new()));
    }

    fn review_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        use xenify_wp_doc::review;
        if icons::item(ui, Icon::CommentAdd, "New comment", "Ctrl+Alt+M").clicked() {
            self.review.start_comment(&self.editor);
            ui.close();
        }
        if icons::check_item(ui, Icon::Comments, "Review pane", "", self.review.show).clicked() {
            self.review.show = !self.review.show;
        }
        ui.separator();
        icons::submenu(ui, Icon::Eye, "Show", |ui| crate::review_panel::markup_choice(ui, &mut self.review.markup));
        let tracking = self.editor.doc.meta.track_changes;
        if icons::check_item(ui, Icon::TrackChanges, "Track changes", "Ctrl+Shift+E", tracking).clicked() {
            let _ = review::set_tracking(&mut self.editor, !tracking);
        }
        let any = !review::changes(&self.editor.doc).is_empty();
        // These two act on the change the caret is in.
        let here = self.on_change();
        if icons::item_enabled(ui, here, Icon::Check, "Accept change", "")
            .on_disabled_hover_text("Put the caret in a tracked change")
            .clicked()
        {
            let _ = review::decide(&mut self.editor, true, false);
        }
        if icons::item_enabled(ui, here, Icon::Close, "Reject change", "")
            .on_disabled_hover_text("Put the caret in a tracked change")
            .clicked()
        {
            let _ = review::decide(&mut self.editor, false, false);
        }
        if icons::item_enabled(ui, any, Icon::Check, "Accept all changes", "").clicked() {
            let _ = review::decide(&mut self.editor, true, true);
        }
        if icons::item_enabled(ui, any, Icon::Close, "Reject all changes", "").clicked() {
            let _ = review::decide(&mut self.editor, false, true);
        }
        for (forward, label) in [(false, "Previous change"), (true, "Next change")] {
            if icons::item_enabled(ui, any, Icon::Blank, label, "").clicked()
                && let Some(sel) = review::next_change(&self.editor.doc, self.editor.sel.focus, forward)
            {
                self.editor.sel = sel;
                self.view.reveal();
            }
        }
        ui.separator();
        if icons::item(ui, Icon::Edit, "Record a change…", "")
            .on_hover_text("Add “who changed what, when” to the document's change log")
            .clicked()
        {
            self.open_record_change(false);
            ui.close();
        }
        if icons::item(ui, Icon::Recent, "Insert change log", "")
            .on_hover_text("A table of the recorded changes, kept up to date")
            .clicked()
        {
            let _ = xenify_wp_doc::tables::insert_generated(&mut self.editor, xenify_wp_doc::TocKind::ChangeLog);
        }
    }

    /// References › the contents tables, lists of figures and
    /// bibliographies already in the document: their settings.
    fn generated_lists_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        use xenify_wp_doc::Block;
        let found: Vec<xenify_wp_doc::Toc> = self
            .editor
            .doc
            .sections
            .iter()
            .flat_map(|s| &s.blocks)
            .filter_map(|b| if let Block::Toc(t) = b { Some(t.clone()) } else { None })
            .collect();
        if found.is_empty() {
            return;
        }
        icons::submenu(ui, Icon::Settings, "In this document", |ui| {
            for (i, t) in found.iter().enumerate() {
                let name = generated_name(t.kind);
                ui.menu_button(if found.len() > 1 { format!("{name} {}", i + 1) } else { name.to_string() }, |ui| {
                    self.generated_settings(ui, t, true);
                });
            }
        });
    }

    /// The generated list (contents, bibliography…) with id `id`, if it is one.
    pub(crate) fn generated_block(&self, id: xenify_wp_doc::NodeId) -> Option<xenify_wp_doc::Toc> {
        let (c, i) = self.editor.doc.locate(id)?;
        match self.editor.doc.blocks(&c)?.get(i)? {
            xenify_wp_doc::Block::Toc(t) => Some(t.clone()),
            _ => None,
        }
    }

    /// The settings of generated list `t`: in References › In this document,
    /// and its right-click menu. `go_to` offers to scroll to it.
    pub(crate) fn generated_settings(&mut self, ui: &mut egui::Ui, t: &xenify_wp_doc::Toc, go_to: bool) {
        use crate::icons::{self, Icon};
        use xenify_wp_doc::{Block, TocKind};
        let name = generated_name(t.kind);
        let mut edit = t.clone();
        match t.kind {
            TocKind::Contents => {
                icons::submenu(ui, Icon::Heading, "Headings shown", |ui| {
                    for n in 1u8..=6 {
                        let label = if n == 1 { "Heading 1 only".to_string() } else { format!("Headings 1–{n}") };
                        if ui.radio(edit.levels == n, label).clicked() {
                            edit.levels = n;
                        }
                    }
                });
            }
            TocKind::Bibliography => {
                icons::submenu(ui, Icon::Styles, "Citation style", |ui| {
                    for style in xenify_wp_doc::cite::CiteStyle::ALL {
                        let on = self.editor.doc.meta.cite_style == style;
                        if ui.radio(on, style.name()).clicked() {
                            let _ = xenify_wp_doc::cite::set_style(&mut self.editor, style);
                            ui.close();
                        }
                    }
                });
                if icons::item(ui, Icon::Sources, "Manage sources…", "").clicked() {
                    self.open_sources();
                    ui.close();
                }
            }
            TocKind::ChangeLog => {
                if icons::item(ui, Icon::Edit, "Record a change…", "").clicked() {
                    self.open_record_change(false);
                    ui.close();
                }
            }
            TocKind::Figures => {}
        }
        if matches!(t.kind, TocKind::Contents | TocKind::Figures) {
            let on = edit.page_numbers;
            if icons::check_item(ui, Icon::Number, "Page numbers", "", on).clicked() {
                edit.page_numbers = !on;
            }
        }
        if go_to && icons::item(ui, Icon::Eye, "Go to it", "").clicked() {
            if let Some(next) = xenify_wp_doc::ops::para_after_block(&self.editor.doc, t.id) {
                self.editor.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(next, 0));
                self.view.reveal();
            }
            ui.close();
        }
        if icons::item(ui, Icon::Delete, &format!("Delete {}", name.to_lowercase()), "").clicked() {
            let _ = xenify_wp_doc::tables::delete_block(&mut self.editor, t.id);
            ui.close();
        } else if edit != *t {
            let _ = xenify_wp_doc::tables::replace_block(&mut self.editor, t.id, Block::Toc(edit), name);
        }
    }

    fn field_menu(&mut self, ui: &mut egui::Ui) {
        use xenify_wp_doc::Field;
        for (f, name) in [
            (Field::Page, "Page number"),
            (Field::Pages, "Number of pages"),
            (Field::Date, "Date"),
            (Field::Title, "Document title"),
            (Field::Author, "Author"),
        ] {
            if ui.button(name).clicked() {
                let _ = ops::insert_field(&mut self.editor, f, "Insert field");
                ui.close();
            }
        }
    }

    pub fn open_insert_image(&mut self) {
        self.dialog =
            Some(Dialog::InsertImage(FileBrowser::new(Mode::Open, &self.start_dir(), crate::images::IMAGE_EXT)));
    }

    /// The unit the user measures in (rulers, page setup).
    pub fn unit(&self) -> crate::page_setup::Unit {
        self.prefs.unit
    }

    pub fn appearance(&self) -> crate::theme::Appearance {
        self.prefs.appearance
    }

    pub fn set_appearance(&mut self, ctx: &egui::Context, a: crate::theme::Appearance) {
        self.prefs.appearance = a;
        crate::theme::apply(ctx, a);
    }

    /// The menus' titles and access letters (Alt+letter), in order.
    pub(crate) const MENUS: [(&'static str, char); 9] = [
        ("File", 'f'),
        ("Edit", 'e'),
        ("View", 'v'),
        ("Insert", 'i'),
        ("Format", 'o'),
        ("References", 's'),
        ("Review", 'r'),
        ("Tools", 't'),
        ("Help", 'h'),
    ];

    /// One top-level menu: its title (access letter underlined while Alt is
    /// held), with keyboard focus on its first entry when opened from the
    /// keyboard.
    fn top_menu(&mut self, ui: &mut egui::Ui, index: usize, add: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let (name, key) = Self::MENUS[index];
        let alt = ui.input(|i| i.modifiers.alt) || self.menu_keys.armed.is_some();
        let at = name.to_lowercase().find(key).unwrap_or(0);
        let title = crate::icons::menu_title(ui, name, at, alt);
        let r = ui.menu_button(title, |ui| {
            let first = ui.next_auto_id();
            add(self, ui);
            if self.menu_keys.focus_first {
                self.menu_keys.focus_first = false;
                ui.memory_mut(|m| m.request_focus(first));
            }
        });
        r.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        if self.menu_keys.armed == Some(index) {
            let accent = crate::theme::accent(ui.visuals());
            ui.painter().rect_filled(r.response.rect, 4.0, accent.gamma_multiply(0.25));
            ui.painter().rect_stroke(r.response.rect, 4.0, egui::Stroke::new(1.0, accent), egui::StrokeKind::Inside);
        }
        if self.menu_keys.ids.len() <= index {
            self.menu_keys.ids.resize(index + 1, egui::Id::NULL);
        }
        self.menu_keys.ids[index] = r.response.id;
        // Like other programs: with a menu open, pointing at another menu's
        // title opens that one instead.
        let me = r.response.id.with("popup");
        if r.response.hovered() && !egui::Popup::is_id_open(ui.ctx(), me) {
            let other = self.menu_keys.ids.iter().enumerate().any(|(i, id)| {
                i != index && *id != egui::Id::NULL && egui::Popup::is_id_open(ui.ctx(), id.with("popup"))
            });
            if other {
                egui::Popup::close_all(ui.ctx());
                egui::Popup::open_id(ui.ctx(), me);
            }
        }
    }

    /// Alt+letter (or F10) opens a menu; arrows move through it and to the
    /// menus beside it; Enter chooses; Esc closes.
    fn menu_keyboard(&mut self, ctx: &egui::Context) {
        let ids = self.menu_keys.ids.clone();
        if ids.len() != Self::MENUS.len() {
            return;
        }
        let popup = |i: usize| ids[i].with("popup");
        let open = (0..ids.len()).find(|i| egui::Popup::is_id_open(ctx, popup(*i)));
        let mut go = None;
        // Alt tapped on its own (pressed and let go with nothing else) gives
        // the menu bar the keyboard, like other programs.
        let (alt, other) = ctx.input(|i| {
            let other = i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key, pressed: true, .. } if !matches!(key, egui::Key::Escape) || i.modifiers.alt)
                    || matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Text(_))
            });
            (i.modifiers.alt, other)
        });
        let keys = &mut self.menu_keys;
        if alt && !keys.alt_was {
            keys.alt_alone = true;
        }
        if alt && other {
            keys.alt_alone = false;
        }
        if !alt && keys.alt_was && std::mem::take(&mut keys.alt_alone) {
            keys.armed = if keys.armed.is_some() || open.is_some() { None } else { Some(0) };
            if open.is_some() {
                egui::Popup::close_all(ctx);
            }
        }
        keys.alt_was = alt;
        if let Some(a) = keys.armed {
            let n = ids.len();
            ctx.input_mut(|inp| {
                if inp.pointer.any_pressed() || inp.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    keys.armed = None;
                } else if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                    keys.armed = Some((a + 1) % n);
                } else if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                    keys.armed = Some((a + n - 1) % n);
                } else if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || inp.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                {
                    go = Some(a);
                } else {
                    for (i, (_, key)) in Self::MENUS.iter().enumerate() {
                        let k = egui::Key::from_name(&key.to_string()).expect("letter");
                        if inp.consume_key(egui::Modifiers::NONE, k) || inp.consume_key(egui::Modifiers::SHIFT, k) {
                            go = Some(i);
                        }
                    }
                    // Anything else typed while the menu bar has the keyboard isn't text for the page.
                    inp.events.retain(|e| !matches!(e, egui::Event::Text(_) | egui::Event::Key { .. }));
                }
            });
            if go.is_some() {
                self.menu_keys.armed = None;
            }
        }
        ctx.input_mut(|inp| {
            if go.is_some() {
                return;
            }
            if open.is_none() {
                for (i, (_, key)) in Self::MENUS.iter().enumerate() {
                    let k = egui::Key::from_name(&key.to_string()).expect("letter");
                    if inp.consume_key(egui::Modifiers::ALT, k) {
                        go = Some(i);
                    }
                }
                if inp.consume_key(egui::Modifiers::NONE, egui::Key::F10) {
                    go = Some(0);
                }
            } else if let Some(o) = open {
                if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                    go = Some((o + 1) % ids.len());
                } else if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                    go = Some((o + ids.len() - 1) % ids.len());
                } else if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::Down));
                } else if inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::Up));
                }
            }
        });
        if let Some(i) = go {
            egui::Popup::close_all(ctx);
            egui::Popup::open_id(ctx, popup(i));
            self.menu_keys.focus_first = true;
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        egui::MenuBar::new().ui(ui, |ui| {
            ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
            self.top_menu(ui, 0, |this, ui| this.file_menu(ui));
            self.top_menu(ui, 1, |this, ui| this.edit_menu(ui));
            self.top_menu(ui, 2, |this, ui| this.view_menu(ui));
            self.top_menu(ui, 3, |this, ui| this.insert_menu(ui));
            self.top_menu(ui, 4, |this, ui| this.format_menu(ui));
            self.top_menu(ui, 5, |this, ui| this.references_menu(ui));
            self.top_menu(ui, 6, |this, ui| this.review_menu(ui));
            self.top_menu(ui, 7, |this, ui| this.tools_menu(ui));
            self.top_menu(ui, 8, |this, ui| {
                if icons::item(ui, Icon::Help, "Keyboard shortcuts", "").clicked() {
                    this.dialog = Some(Dialog::Shortcuts);
                    ui.close();
                }
                if icons::item(ui, Icon::Info, "About Xenify Write", "").clicked() {
                    this.dialog = Some(Dialog::About);
                    ui.close();
                }
            });
        });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        if icons::item(ui, Icon::New, "New", "Ctrl+N").clicked() {
            self.run(Then::New);
        }
        if icons::item(ui, Icon::Panel, "New window", "").clicked() {
            self.new_window();
            ui.close();
        }
        icons::submenu(ui, Icon::Template, "New from template", |ui| {
            for (i, (name, about, _)) in crate::templates::TEMPLATES.iter().enumerate() {
                if ui.button(*name).on_hover_text(*about).clicked() {
                    self.run(Then::Template(i));
                    ui.close();
                }
            }
        });
        if icons::item(ui, Icon::Open, "Open…", "Ctrl+O").clicked() {
            self.run(Then::Open);
        }
        icons::submenu(ui, Icon::Recent, "Open recent", |ui| {
            if self.prefs.recent.is_empty() {
                ui.weak("Nothing yet");
            }
            for p in self.prefs.recent.clone() {
                if ui.button(p.display().to_string()).clicked() {
                    self.run(Then::OpenPath(p));
                }
            }
        });
        ui.separator();
        if icons::item(ui, Icon::Save, "Save", "Ctrl+S").clicked() {
            self.save();
        }
        if icons::item(ui, Icon::SaveAs, "Save as…", "Ctrl+Shift+S").clicked() {
            self.save_as();
        }
        icons::submenu(ui, Icon::Export, "Export as", |ui| {
            for f in [Format::Pdf, Format::Docx, Format::Html, Format::Markdown, Format::Text] {
                if ui.button(f.name()).clicked() {
                    self.open_export(f);
                    ui.close();
                }
            }
        });
        ui.separator();
        if icons::item(ui, Icon::Info, "Document properties…", "")
            .on_hover_text("The title and author (shown by Insert › Field and saved with the file)")
            .clicked()
        {
            let m = &self.editor.doc.meta;
            self.dialog = Some(Dialog::Properties(m.title.clone(), m.author.clone()));
            ui.close();
        }
        if icons::item(ui, Icon::Print, "Print…", "Ctrl+P").clicked() {
            self.open_print();
        }
        if icons::item(ui, Icon::Settings, "Settings…", "").clicked() {
            self.open_settings(crate::settings::Section::General);
            ui.close();
        }
        if icons::item(ui, Icon::Close, "Close", "Ctrl+W").clicked() {
            self.close_tab_asking();
            ui.close();
        }
        ui.separator();
        if icons::item(ui, Icon::Quit, "Quit", "Ctrl+Q").clicked() {
            self.ask_quit();
        }
    }

    fn edit_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        let undo = self.editor.undo_label().map(|l| format!("Undo {l}"));
        if icons::item_enabled(ui, undo.is_some(), Icon::Undo, &undo.unwrap_or("Undo".into()), "Ctrl+Z").clicked() {
            self.editor.undo();
            self.view.extra.clear();
        }
        let redo = self.editor.redo_label().map(|l| format!("Redo {l}"));
        if icons::item_enabled(ui, redo.is_some(), Icon::Redo, &redo.unwrap_or("Redo".into()), "Ctrl+Y").clicked() {
            self.editor.redo();
            self.view.extra.clear();
        }
        ui.separator();
        let has_sel = !self.editor.sel.is_collapsed();
        if icons::item_enabled(ui, has_sel, Icon::Cut, "Cut", "Ctrl+X").clicked() {
            self.view.copy(ui.ctx(), &mut self.editor, true);
        }
        if icons::item_enabled(ui, has_sel, Icon::Copy, "Copy", "Ctrl+C").clicked() {
            self.view.copy(ui.ctx(), &mut self.editor, false);
        }
        if icons::item(ui, Icon::Paste, "Paste", "Ctrl+V").clicked()
            && !self.view.paste_from_clipboard(&mut self.editor, false)
        {
            self.message = Some(("The clipboard is empty".into(), false));
        }
        if icons::item(ui, Icon::PastePlain, "Paste as plain text", "Ctrl+Shift+V").clicked()
            && !self.view.paste_from_clipboard(&mut self.editor, true)
        {
            self.message = Some(("The clipboard has no text".into(), false));
        }
        ui.separator();
        if icons::item(ui, Icon::Blank, "Select all", "Ctrl+A").clicked() {
            let (s, e) = (self.editor.doc.start(), self.editor.doc.end());
            self.editor.sel = xenify_wp_doc::Selection { anchor: s, focus: e };
        }
        ui.separator();
        if icons::item(ui, Icon::Find, "Find…", "Ctrl+F").clicked() {
            self.open_find(false);
            ui.close();
        }
        if icons::item(ui, Icon::Replace, "Replace…", "Ctrl+H").clicked() {
            self.open_find(true);
            ui.close();
        }
        if icons::item(ui, Icon::Rows, "Filter lines…", "")
            .on_hover_text("Remove the paragraphs that contain something, or keep only those")
            .clicked()
        {
            self.open_find(false);
            self.find.filter = true;
            ui.close();
        }
        ui.separator();
        if icons::item_enabled(ui, !self.editor.doc.meta.pageless, Icon::Document, "Go to page…", "Ctrl+G").clicked()
        {
            self.open_go_to();
            ui.close();
        }
    }

    fn view_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        let pageless = self.editor.doc.meta.pageless;
        if icons::check_item(ui, Icon::Document, "Print layout (pages)", "", !pageless).clicked() {
            self.set_pageless(false);
        }
        if icons::check_item(ui, Icon::Rows, "Pageless", "", pageless).clicked() {
            self.set_pageless(true);
        }
        ui.separator();
        if icons::check_item(ui, Icon::Pilcrow, "Toolbar", "", !self.prefs.hide_toolbar).clicked() {
            self.prefs.hide_toolbar = !self.prefs.hide_toolbar;
        }
        if icons::check_item(ui, Icon::Ruler, "Ruler", "", !self.prefs.hide_ruler).clicked() {
            self.prefs.hide_ruler = !self.prefs.hide_ruler;
        }
        if icons::check_item(ui, Icon::Contents, "Navigation", "Ctrl+Shift+N", self.nav.show).clicked() {
            self.nav.show = !self.nav.show;
        }
        if self.split.is_none() {
            if icons::item(ui, Icon::Columns2, "Split side by side", "Ctrl+\\").clicked() {
                self.open_split(false);
                ui.close();
            }
            if icons::item(ui, Icon::Rows, "Split top and bottom", "").clicked() {
                self.open_split(true);
                ui.close();
            }
        } else if icons::item(ui, Icon::Close, "Close split", "Ctrl+\\").clicked() {
            self.close_split();
            ui.close();
        }
        icons::submenu(ui, Icon::Panel, "Side panel", |ui| {
            for t in crate::dock::Tab::ALL {
                let on = crate::dock::is_open(self, t);
                if ui.checkbox(&mut on.clone(), t.name()).clicked() {
                    if on {
                        crate::dock::set_open(self, t, false);
                    } else {
                        crate::dock::open(self, t);
                    }
                }
            }
        });
        ui.separator();
        if icons::item(ui, Icon::ZoomIn, "Zoom in", "Ctrl+=").clicked() {
            self.view.zoom = ((self.view.zoom * 1.1).min(4.0) * 100.0).round() / 100.0;
        }
        if icons::item(ui, Icon::ZoomOut, "Zoom out", "Ctrl+-").clicked() {
            self.view.zoom = ((self.view.zoom / 1.1).max(0.25) * 100.0).round() / 100.0;
        }
        if icons::item(ui, Icon::Blank, "Actual size (100%)", "Ctrl+0").clicked() {
            self.view.zoom = 1.0;
        }
        icons::submenu(ui, Icon::Blank, "Zoom", |ui| {
            if ui.button("Fit width").clicked() {
                self.view.fit(false);
            }
            if ui.button("Fit page").clicked() {
                self.view.fit(true);
            }
            ui.separator();
            for z in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                if ui.radio(self.view.zoom == z, format!("{:.0}%", z * 100.0)).clicked() {
                    self.view.zoom = z;
                }
            }
        });
        icons::submenu(ui, self.prefs.appearance.icon(), "Appearance", |ui| {
            for a in crate::theme::Appearance::ALL {
                if icons::check_item(ui, a.icon(), a.name(), "", self.prefs.appearance == a).clicked() {
                    self.set_appearance(ui.ctx(), a);
                }
            }
        });
    }

    fn insert_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        icons::submenu(ui, Icon::Table, "Table", |ui| self.table_picker(ui));
        if icons::item(ui, Icon::Image, "Picture…", "").clicked() {
            self.open_insert_image();
            ui.close();
        }
        if icons::item(ui, Icon::TextBox, "Text box", "")
            .on_hover_text("A box you can place anywhere on the page; text wraps around it")
            .clicked()
        {
            self.insert_text_box();
            ui.close();
        }
        if icons::item(ui, Icon::Link, "Link…", "Ctrl+K").clicked() {
            self.open_link_dialog();
        }
        if icons::item(ui, Icon::Symbol, "Equation", "Alt+=").clicked() {
            self.insert_equation();
            ui.close();
        }
        if icons::item(ui, Icon::Emoji, "Symbols & emoji…", "Ctrl+.").clicked() {
            self.symbols.show();
            ui.close();
        }
        ui.separator();
        if icons::item(ui, Icon::Break, "Page break", "Ctrl+Enter").clicked() {
            let _ = ops::insert_page_break(&mut self.editor);
        }
        if icons::item(ui, Icon::Document, "Title page", "")
            .on_hover_text("A first page with the title, subtitle and author, centred top to bottom")
            .clicked()
        {
            let _ = ops::insert_title_page(&mut self.editor);
            self.refocus = true;
            ui.close();
        }
        if icons::item(ui, Icon::Header, "Header", "")
            .on_hover_text("Edit the header on this page (or double-click the top margin)")
            .clicked()
        {
            self.edit_header(false);
        }
        if icons::item(ui, Icon::Footer, "Footer", "")
            .on_hover_text("Edit the footer on this page (or double-click the bottom margin)")
            .clicked()
        {
            self.edit_header(true);
        }
        icons::submenu(ui, Icon::Field, "Field", |ui| self.field_menu(ui));
        ui.separator();
        if icons::item(ui, Icon::Footnote, "Footnote", "Ctrl+Alt+F").clicked() {
            self.insert_note(xenify_wp_doc::NoteKind::Foot);
        }
        if icons::item(ui, Icon::CommentAdd, "Comment", "Ctrl+Alt+M").clicked() {
            self.review.start_comment(&self.editor);
            ui.close();
        }
    }

    fn format_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        icons::submenu(ui, Icon::Bold, "Text", |ui| {
            for (t, icon, key) in [
                (Toggle::Bold, Icon::Bold, "Ctrl+B"),
                (Toggle::Italic, Icon::Italic, "Ctrl+I"),
                (Toggle::Underline, Icon::Underline, "Ctrl+U"),
                (Toggle::Strike, Icon::Strike, ""),
                (Toggle::Superscript, Icon::Superscript, "Ctrl+."),
                (Toggle::Subscript, Icon::Subscript, "Ctrl+,"),
            ] {
                let on = self.view.is_on(&self.editor, t);
                if icons::check_item(ui, icon, t.label(), key, on).clicked() {
                    self.view.toggle(&mut self.editor, t);
                }
            }
        });
        icons::submenu(ui, Icon::Heading, "Styles", |ui| self.style_list(ui));
        if icons::item(ui, Icon::Pilcrow, "Paragraph…", "").clicked() {
            self.open_paragraph_dialog();
        }
        icons::submenu(ui, Icon::AlignLeft, "Align", |ui| {
            for (a, icon, name, key) in [
                (xenify_wp_doc::Align::Left, Icon::AlignLeft, "Left", "Ctrl+L"),
                (xenify_wp_doc::Align::Center, Icon::AlignCenter, "Centre", "Ctrl+E"),
                (xenify_wp_doc::Align::Right, Icon::AlignRight, "Right", "Ctrl+R"),
                (xenify_wp_doc::Align::Justify, Icon::AlignJustify, "Justify", "Ctrl+J"),
            ] {
                if icons::item(ui, icon, name, key).clicked() {
                    self.set_para(xenify_wp_doc::ParaProps { align: Some(a), ..Default::default() }, "Align");
                }
            }
        });
        icons::submenu(ui, Icon::Bullets, "Lists", |ui| {
            for def in self.editor.doc.lists.clone() {
                if ui.button(&def.name).clicked() {
                    let _ = ops::toggle_list(&mut self.editor, &def.id);
                }
            }
            ui.separator();
            if icons::item(ui, Icon::IndentMore, "Increase indent", "Tab").clicked() {
                self.indent(1);
            }
            if icons::item(ui, Icon::IndentLess, "Decrease indent", "Shift+Tab").clicked() {
                self.indent(-1);
            }
        });
        ui.separator();
        if icons::item(ui, Icon::Borders, "Borders and shading…", "").clicked() {
            self.show_borders = true;
        }
        if icons::item(ui, Icon::ClearFormat, "Clear formatting", "Ctrl+\\").clicked() {
            self.clear_formatting();
        }
        ui.separator();
        // Tables and the page (once menus of their own).
        icons::submenu(ui, Icon::Table, "Table", |ui| self.table_menu(ui));
        self.layout_menu(ui);
    }

    /// Format › the page: setup, columns, colour and breaks.
    fn layout_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        if icons::item(ui, Icon::PageSetup, "Page setup…", "").clicked() {
            self.open_page_setup();
        }
        icons::submenu(ui, Icon::Columns2, "Columns", |ui| self.columns_menu(ui));
        icons::submenu(ui, Icon::Fill, "Page colour", |ui| {
            let now = self.editor.doc.meta.page_color;
            let picked = swatches(ui, &PAGE_COLOURS, now);
            if ui.button("No colour (white)").clicked() || picked.is_some() {
                let mut meta = self.editor.doc.meta.clone();
                meta.page_color = picked;
                self.editor.seal();
                let _ = ops::set_meta(&mut self.editor, meta, "Page colour");
                self.editor.seal();
                ui.close();
            }
        });
        icons::submenu(ui, Icon::Break, "Breaks", |ui| {
            if icons::item(ui, Icon::Break, "Page break", "Ctrl+Enter").clicked() {
                let _ = ops::insert_page_break(&mut self.editor);
            }
            if icons::item(ui, Icon::Columns2, "Column break", "").clicked() {
                let _ = ops::insert_column_break(&mut self.editor);
            }
            if icons::item(ui, Icon::Break, "Section break (next page)", "").clicked() {
                let _ = ops::insert_section_break(&mut self.editor, xenify_wp_doc::SectionStart::NewPage);
            }
            if icons::item(ui, Icon::Break, "Section break (continuous)", "").clicked() {
                let _ = ops::insert_section_break(&mut self.editor, xenify_wp_doc::SectionStart::Continuous);
            }
            let si = self.caret_section();
            if icons::item_enabled(ui, si > 0, Icon::Delete, "Remove section break", "")
                .on_hover_text("Join this section to the one before")
                .clicked()
            {
                let _ = ops::remove_section_break(&mut self.editor, si);
            }
        });
    }

    fn references_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        icons::submenu(ui, Icon::Contents, "Table of contents", |ui| {
            for levels in 1u8..=6 {
                let label = if levels == 1 { "Heading 1 only".to_string() } else { format!("Headings 1–{levels}") };
                if ui.button(label).clicked() {
                    let _ = xenify_wp_doc::tables::insert_toc(&mut self.editor, levels);
                    ui.close();
                }
            }
        });
        if icons::item(ui, Icon::Image, "List of figures", "")
            .on_hover_text("Lists the paragraphs in the Caption style, with their pages")
            .clicked()
        {
            let _ = xenify_wp_doc::tables::insert_generated(&mut self.editor, xenify_wp_doc::TocKind::Figures);
        }
        self.generated_lists_menu(ui);
        ui.separator();
        if icons::item(ui, Icon::Footnote, "Footnote", "Ctrl+Alt+F").clicked() {
            self.insert_note(xenify_wp_doc::NoteKind::Foot);
        }
        if icons::item(ui, Icon::Footnote, "Endnote", "Ctrl+Alt+D").clicked() {
            self.insert_note(xenify_wp_doc::NoteKind::End);
        }
        ui.separator();
        if icons::item(ui, Icon::Quote, "Insert citation…", "").clicked() {
            self.open_cite();
        }
        if icons::item(ui, Icon::Sources, "Manage sources…", "").clicked() {
            self.open_sources();
        }
        icons::submenu(ui, Icon::Styles, "Citation style", |ui| {
            for style in xenify_wp_doc::cite::CiteStyle::ALL {
                let on = self.editor.doc.meta.cite_style == style;
                if ui.radio(on, style.name()).clicked() {
                    let _ = xenify_wp_doc::cite::set_style(&mut self.editor, style);
                    ui.close();
                }
            }
        });
        if icons::item(ui, Icon::Library, "Bibliography", "")
            .on_hover_text("A list of the sources cited, kept up to date")
            .clicked()
        {
            let _ = xenify_wp_doc::tables::insert_generated(&mut self.editor, xenify_wp_doc::TocKind::Bibliography);
        }
        ui.separator();
        icons::submenu(ui, Icon::Bookmark, "Cross-reference", |ui| {
            let headings = xenify_wp_layout::toc_entries(&self.editor.doc, 6);
            if headings.is_empty() {
                ui.weak("Add headings to refer to them");
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (id, level, text) in headings {
                    let label = format!("{}{}", "   ".repeat(level.saturating_sub(1) as usize), text);
                    if ui
                        .button(label)
                        .on_hover_text("Inserts the heading's text; it follows the heading when that changes")
                        .clicked()
                    {
                        let _ = ops::insert_field(
                            &mut self.editor,
                            xenify_wp_doc::Field::Ref { target: id },
                            "Insert cross-reference",
                        );
                        ui.close();
                    }
                }
            });
        });
    }

    fn tools_menu(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{self, Icon};
        if icons::item(ui, Icon::Spelling, "Spelling and grammar", "").clicked() {
            crate::dock::open(self, crate::dock::Tab::Writing);
            ui.close();
        }
        if icons::item(ui, Icon::Sources, "Dictionary & thesaurus", "Shift+F7").clicked() {
            crate::dictionary::open_at_caret(self);
            ui.close();
        }
        if icons::item(ui, Icon::Number, "Word count…", "").clicked() {
            self.dialog = Some(Dialog::WordCount);
            ui.close();
        }
        ui.separator();
        icons::submenu(ui, Icon::Ai, "AI assistant", |ui| {
            let ctx = ui.ctx().clone();
            if ui.button("Open the assistant").clicked() {
                crate::dock::open(self, crate::dock::Tab::Writing);
                ui.close();
            }
            ui.separator();
            if ui.button("Review selection").clicked() {
                crate::dock::open(self, crate::dock::Tab::Writing);
                crate::assistant::review(self, &ctx, false);
                ui.close();
            }
            if ui.button("Review document").clicked() {
                crate::dock::open(self, crate::dock::Tab::Writing);
                crate::assistant::review(self, &ctx, true);
                ui.close();
            }
            icons::submenu(ui, Icon::Edit, "Rewrite selection", |ui| {
                for (name, instruction) in xenify_wp_ai::REWRITES {
                    if ui.button(name).clicked() {
                        crate::dock::open(self, crate::dock::Tab::Writing);
                        crate::assistant::rewrite(self, &ctx, instruction.to_string());
                        ui.close();
                    }
                }
            });
            if ui.add(Button::new("Continue writing").shortcut_text("Ctrl+Shift+Enter")).clicked() {
                crate::dock::open(self, crate::dock::Tab::Writing);
                crate::assistant::continue_writing(self, &ctx);
                ui.close();
            }
        });
        if icons::item(ui, Icon::Mail, "Mail merge…", "").clicked() {
            self.show_merge = true;
            ui.close();
        }
    }

    /// The paragraph styles, each shown in its own look (size, weight,
    /// slant, colour). Right-click one to update it from the selection.
    pub(crate) fn style_list(&mut self, ui: &mut egui::Ui) {
        let current = self.current_style();
        let mut update = None;
        let mut panel = false;
        // Styles for the text itself (contents entries, headers, notes… are
        // applied by Write; they're in the Styles panel).
        let internal = |id: &str| {
            id.starts_with("toc") || matches!(id, "header" | "footer") || id == xenify_wp_doc::StyleSheet::NOTE
        };
        egui::ScrollArea::vertical().id_salt("style-list").max_height(300.0).show(ui, |ui| {
            for s in
                self.editor.doc.styles.paragraph.clone().into_iter().filter(|s| !internal(&s.id) || s.id == current)
            {
                let look = self.editor.doc.styles.resolve(&s.id).char;
                let [r8, g8, b8, _] = look.color.0;
                // The style's own colour when it has one; near-black text uses the menu's.
                let grey = r8.abs_diff(g8) < 24 && g8.abs_diff(b8) < 24;
                let color = if grey { ui.visuals().text_color() } else { egui::Color32::from_rgb(r8, g8, b8) };
                let mut text = egui::RichText::new(&s.name).size(look.size.clamp(10.0, 22.0)).color(color);
                if look.bold {
                    text = text.strong();
                }
                if look.italic {
                    text = text.italics();
                }
                if look.underline {
                    text = text.underline();
                }
                ui.set_min_width(240.0);
                let h = look.size.clamp(10.0, 22.0) * 1.3 + 10.0;
                let r = ui.add(
                    egui::Button::new(text)
                        .selected(current == s.id)
                        .frame_when_inactive(current == s.id)
                        .min_size(egui::vec2(ui.available_width(), h)),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, current == s.id, &s.name)
                });
                if r.clicked() {
                    self.editor.seal();
                    let _ = ops::set_style(&mut self.editor, &s.id);
                    ui.close();
                }
                r.context_menu(|ui| {
                    if ui.button(format!("Update “{}” to match the selection", s.name)).clicked() {
                        update = Some(s.id.clone());
                        ui.close();
                    }
                    if ui.button("Styles panel…").clicked() {
                        panel = true;
                        ui.close();
                    }
                });
            }
        });
        ui.separator();
        let cur_name = self.editor.doc.styles.get(&current).map(|s| s.name.clone()).unwrap_or_default();
        let w = ui.available_width();
        let entry = |ui: &mut egui::Ui, text: String| {
            ui.add(
                egui::Button::new(text).frame_when_inactive(false).min_size(egui::vec2(w, crate::icons::ITEM_HEIGHT)),
            )
            .clicked()
        };
        if !cur_name.is_empty() && entry(ui, format!("Update “{cur_name}” to match the selection")) {
            update = Some(current.clone());
            ui.close();
        }
        if entry(ui, "Styles panel…".into()) {
            panel = true;
            ui.close();
        }
        if let Some(id) = update {
            let _ = ops::update_style_from_selection(&mut self.editor, &id);
        }
        if panel {
            crate::dock::open(self, crate::dock::Tab::Styles);
        }
    }

    pub(crate) fn current_style(&self) -> String {
        self.editor.doc.para(self.editor.sel.focus.para).map(|p| p.style.clone()).unwrap_or_default()
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let mut go_to = false;
        ui.horizontal(|ui| {
            let d = &self.editor.doc;
            if !d.meta.pageless {
                let pages = self.view.layout.pages.len();
                if let Some(i) = self.view.caret_page(&self.editor) {
                    if ui
                        .add(egui::Label::new(format!("Page {} of {pages}", i + 1)).sense(egui::Sense::click()))
                        .on_hover_text("Go to page… (Ctrl+G)")
                        .clicked()
                    {
                        go_to = true;
                    }
                    ui.separator();
                }
            }
            // With a selection: how much of the document it is.
            let (words, chars) = (d.word_count(), d.char_count());
            match ops::selection_counts(&self.editor) {
                Some((sw, sc)) => ui.label(format!("{sw} of {words} words · {sc} of {chars} characters")),
                None => ui.label(format!("{words} words · {chars} characters")),
            };
            if !self.view.extra.is_empty() {
                ui.separator();
                ui.label(format!("{} carets · Esc for one", self.view.extra.len() + 1));
            }
            if self.review.markup != xenify_wp_doc::review::Markup::All {
                ui.separator();
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!("Showing {} (read only)", self.review.markup.name().to_lowercase()))
                                .strong(),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Click to show all markup and edit again")
                    .clicked()
                {
                    self.review.markup = xenify_wp_doc::review::Markup::All;
                }
            }
            if d.meta.track_changes {
                ui.separator();
                let c = xenify_wp_layout::author_color(&self.user_name).0;
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new("Tracking changes").color(egui::Color32::from_rgb(c[0], c[1], c[2])),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Click to open the Review pane")
                    .clicked()
                {
                    self.review.show = true;
                }
            }
            if let Some((msg, err)) = &self.message {
                ui.separator();
                let t = RichText::new(msg);
                let r = ui.label(if *err { t.color(ui.visuals().error_fg_color) } else { t });
                // Screen readers say it when it appears.
                let live = if *err { egui::accesskit::Live::Assertive } else { egui::accesskit::Live::Polite };
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_live(live));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut pct = self.view.zoom * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 25.0..=400.0).suffix("%").integer()).changed() {
                    self.view.zoom = pct / 100.0;
                }
                ui.label("Zoom");
            });
        });
        if go_to {
            self.open_go_to();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let mut then = None;
        // A dialog that opens another in its place.
        let mut next: Option<Dialog> = None;
        let mut dialog = dialog;
        let open = matches!(dialog, Dialog::Open(_));
        match &mut dialog {
            Dialog::Open(b) | Dialog::SaveAs(b) => {
                let title = if open { "Open" } else { "Save as" };
                let mut window_open = true;
                crate::dialog::window(ctx, title, &mut window_open).resizable(true).default_size([640.0, 440.0]).show(
                    ctx,
                    |ui| match b.ui(ui, if open { "Open" } else { "Save" }) {
                        Some(Ok(path)) => {
                            keep = false;
                            if open {
                                then = Some(Then::OpenPath(path));
                            } else if self.request_save(&path) {
                                then = self.pending_after_save.take();
                            }
                        }
                        Some(Err(())) => {
                            keep = false;
                            self.pending_after_save = None;
                        }
                        None => {}
                    },
                );
                keep &= window_open;
            }
            Dialog::Unsaved(next) => {
                let next = next.clone();
                egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
                    ui.heading(format!("Save changes to {}?", self.file_name()));
                    ui.label("Your changes will be lost if you don't save them.");
                    use crate::dialog::Kind;
                    match crate::dialog::buttons(
                        ui,
                        &[("Save", Kind::Main), ("Don't save", Kind::Other), ("Cancel", Kind::Cancel)],
                    ) {
                        Some(0) => {
                            keep = false;
                            self.pending_after_save = Some(next.clone());
                            if let Some(path) = self.path.clone() {
                                if self.request_save(&path) {
                                    then = self.pending_after_save.take();
                                }
                            } else {
                                self.save_as();
                            }
                        }
                        Some(1) => {
                            keep = false;
                            then = Some(next.clone());
                            // Chosen not to keep: no recovering it later.
                            self.recovery.clear();
                        }
                        Some(_) => keep = false,
                        None => {}
                    }
                });
            }
            Dialog::Link(url) => {
                let mut open = true;
                let had_link = !ops::common_format(&self.editor).link.unwrap_or_default().is_empty();
                crate::dialog::window(ctx, "Link", &mut open).show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.weak(format!("Linking “{}”", self.editor.doc.text_in(self.editor.sel)));
                    ui.label("Web address");
                    let r = ui.add(egui::TextEdit::singleline(url).hint_text("https://…").desired_width(f32::INFINITY));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    use crate::dialog::Kind;
                    let mut list = vec![
                        ("OK", if url.trim().is_empty() { Kind::Off } else { Kind::Main }),
                        ("Cancel", Kind::Cancel),
                    ];
                    if had_link {
                        list.insert(0, ("Remove link", Kind::Other));
                    }
                    let chosen = crate::dialog::buttons(ui, &list).map(|i| list[i].0);
                    let esc = crate::dialog::escape(ui.ctx());
                    {
                        if chosen == Some("OK") || (enter && !url.trim().is_empty()) {
                            let link = Some(url.trim().to_string());
                            self.editor.seal();
                            let _ = ops::apply_format(
                                &mut self.editor,
                                &xenify_wp_doc::CharFmt { link, ..Default::default() },
                                "Link",
                            );
                            keep = false;
                        }
                        if chosen == Some("Remove link") {
                            self.editor.seal();
                            let _ = ops::apply_format(
                                &mut self.editor,
                                &xenify_wp_doc::CharFmt { link: Some(String::new()), ..Default::default() },
                                "Remove link",
                            );
                            keep = false;
                        }
                        if chosen == Some("Cancel") || esc {
                            keep = false;
                        }
                    }
                });
                keep &= open;
                if !keep {
                    self.focus_document(ctx);
                }
            }
            Dialog::Losses { path, losses } => {
                let path = path.clone();
                let losses = losses.clone();
                egui::Modal::new(egui::Id::new("losses")).show(ctx, |ui| {
                    let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    ui.heading("Some things can't be saved in this format");
                    ui.label(format!("Saving {name} would lose:"));
                    for l in losses.iter().take(5) {
                        ui.label(format!("  • {l}"));
                    }
                    if losses.len() > 5 {
                        ui.collapsing(format!("and {} more", losses.len() - 5), |ui| {
                            for l in losses.iter().skip(5) {
                                ui.label(format!("  • {l}"));
                            }
                        });
                    }
                    ui.add_space(6.0);
                    ui.label("A Xenify Write document (.igdoc) keeps everything.");
                    use crate::dialog::Kind;
                    match crate::dialog::buttons(
                        ui,
                        &[
                            ("Save as .igdoc instead", Kind::Main),
                            ("Save anyway", Kind::Other),
                            ("Cancel", Kind::Cancel),
                        ],
                    ) {
                        Some(0) => {
                            keep = false;
                            if self.save_to(&path.with_extension("igdoc")) {
                                then = self.pending_after_save.take();
                            }
                        }
                        Some(1) => {
                            keep = false;
                            self.accepted_losses = Some((path.clone(), losses.clone()));
                            if self.save_to(&path) {
                                then = self.pending_after_save.take();
                            }
                        }
                        Some(_) => {
                            keep = false;
                            self.pending_after_save = None;
                        }
                        None => {}
                    }
                });
            }
            Dialog::InsertImage(b) => {
                let mut open = true;
                crate::dialog::window(ctx, "Insert picture", &mut open)
                    .resizable(true)
                    .default_size([640.0, 440.0])
                    .show(ctx, |ui| match b.ui(ui, "Insert") {
                        Some(Ok(path)) => {
                            keep = false;
                            match crate::images::embed(&mut self.editor.doc, &path) {
                                Ok((key, w, h)) => {
                                    let _ = xenify_wp_doc::tables::insert_image(&mut self.editor, &key, w, h);
                                }
                                Err(e) => self.message = Some((e, true)),
                            }
                        }
                        Some(Err(())) => keep = false,
                        None => {}
                    });
                keep &= open;
            }
            Dialog::Export(b, format) => {
                let format = *format;
                let mut open = true;
                crate::dialog::window(ctx, format!("Export as {}", format.name()), &mut open)
                    .resizable(true)
                    .default_size([640.0, 440.0])
                    .show(ctx, |ui| match b.ui(ui, "Export") {
                        Some(Ok(path)) => {
                            keep = false;
                            self.export_to(&path, format);
                        }
                        Some(Err(())) => keep = false,
                        None => {}
                    });
                keep &= open;
            }
            Dialog::RecordChange(summary, then_save) => {
                let mut open = true;
                let mut record = None;
                let mut remove = None;
                let can_save = self.path.is_some();
                let log = self.editor.doc.meta.changelog.clone();
                crate::dialog::window(ctx, "Record a change", &mut open).default_width(460.0).show(ctx, |ui| {
                    ui.label(format!(
                        "{} changed this on {}:",
                        self.user_name,
                        xenify_wp_doc::changelog::day(&xenify_wp_doc::review::now())
                    ));
                    ui.add(egui::TextEdit::multiline(summary).desired_rows(2).desired_width(f32::INFINITY));
                    ui.weak("Suggested from what changed since the last save: edit it to say why.");
                    if !log.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(format!("Earlier changes ({})", log.len())).show(ui, |ui| {
                            egui::Grid::new("changelog").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                                for (i, e) in log.iter().enumerate() {
                                    ui.label(xenify_wp_doc::changelog::day(&e.date));
                                    ui.label(&e.author);
                                    ui.label(&e.summary);
                                    if ui.small_button("✕").on_hover_text("Remove this entry").clicked() {
                                        remove = Some(i);
                                    }
                                    ui.end_row();
                                }
                            });
                        });
                        ui.weak("Review › Insert change log puts it in the document as a table.");
                    }
                    use crate::dialog::Kind;
                    let ok = if summary.trim().is_empty() { Kind::Off } else { Kind::Main };
                    let mut list = vec![("Record", ok)];
                    if can_save && !*then_save {
                        list.push(("Record and save", if ok == Kind::Main { Kind::Other } else { Kind::Off }));
                    }
                    list.push((if *then_save { "Save without a note" } else { "Cancel" }, Kind::Cancel));
                    match crate::dialog::buttons(ui, &list).map(|i| list[i].0) {
                        Some("Record") => record = Some(false),
                        Some("Record and save") => record = Some(true),
                        Some(_) => {
                            keep = false;
                            if *then_save {
                                self.save_now();
                            }
                        }
                        None => {}
                    }
                });
                keep &= open;
                if let Some(i) = remove {
                    let _ = xenify_wp_doc::changelog::remove(&mut self.editor, i);
                }
                if let Some(save) = record {
                    let _ = xenify_wp_doc::changelog::record(&mut self.editor, summary);
                    keep = false;
                    if save || *then_save {
                        self.dialog = None;
                        self.save_now();
                    }
                }
            }
            Dialog::Recover(left) => {
                let (info, _, untitled) = &**left;
                let name = info
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| {
                        if info.title.is_empty() { "an untitled document".into() } else { info.title.clone() }
                    });
                let has_file = info.path.is_some();
                let mut choice = None;
                let mut open = true;
                crate::dialog::window(ctx, "Recover unsaved work", &mut open).show(ctx, |ui| {
                    ui.label(format!(
                        "Xenify Write closed before {name} was saved. Its unsaved work from {} was kept.",
                        crate::recovery::when(info.time)
                    ));
                    if has_file {
                        ui.weak("The saved file hasn't been changed.");
                    }
                    if has_file {
                        ui.weak("“Show as changes” marks what's different from the saved file, to accept or reject.");
                    }
                    use crate::dialog::Kind;
                    let mut list = vec![("Discard", Kind::Other)];
                    if has_file {
                        list.push(("Show as changes", Kind::Other));
                    }
                    list.push(("Recover", Kind::Main));
                    match crate::dialog::buttons(ui, &list).map(|i| list[i].0) {
                        Some("Recover") => choice = Some(false),
                        Some("Show as changes") => choice = Some(true),
                        Some(_) => {
                            let path = info.path.clone();
                            self.recovery.forget(path.as_deref(), untitled.as_deref());
                            keep = false;
                        }
                        None => {}
                    }
                });
                keep &= open;
                if let Some(as_changes) = choice {
                    keep = false;
                    let (info, doc, untitled) = (**left).clone();
                    self.recover(&info, doc, untitled, as_changes);
                }
            }
            Dialog::Notice(title, lines) => {
                let mut open = true;
                crate::dialog::window(ctx, title.as_str(), &mut open).show(ctx, |ui| {
                    for l in lines.iter() {
                        ui.label(format!("• {l}"));
                    }
                    if crate::dialog::buttons(ui, &[("OK", crate::dialog::Kind::Main)]).is_some()
                        || crate::dialog::escape(ui.ctx())
                    {
                        keep = false;
                    }
                });
                keep &= open;
            }
            Dialog::Sources(d) => {
                let mut open = true;
                let mut save = false;
                let start = self.start_dir();
                let doc = &self.editor.doc;
                crate::dialog::window(ctx, "Sources", &mut open).default_width(720.0).show(ctx, |ui| {
                    d.ui(ui, doc, &start);
                    use crate::dialog::Kind;
                    match crate::dialog::buttons(ui, &[("OK", Kind::Main), ("Cancel", Kind::Cancel)]) {
                        Some(0) => {
                            save = true;
                            keep = false;
                        }
                        Some(_) => keep = false,
                        None => {}
                    }
                });
                keep &= open;
                if save {
                    let sources = d.result();
                    let _ = xenify_wp_doc::cite::set_sources(&mut self.editor, sources);
                }
            }
            Dialog::Cite(d) => {
                let mut open = true;
                let mut result = (None, false);
                let doc = &self.editor.doc;
                crate::dialog::window(ctx, "Insert citation", &mut open).default_width(420.0).show(ctx, |ui| {
                    result = d.ui(ui, doc);
                });
                keep &= open && !d.cancelled;
                if let Some((keys, locator)) = result.0 {
                    let _ = xenify_wp_doc::cite::insert_citation(&mut self.editor, keys, locator);
                    keep = false;
                }
                if result.1 {
                    next = Some(Dialog::Sources(Box::new(crate::references::SourcesDialog::new(&self.editor.doc))));
                    keep = false;
                }
            }
            Dialog::Paragraph(d) => {
                let mut open = true;
                let mut apply = false;
                let unit = self.prefs.unit;
                crate::dialog::window(ctx, "Paragraph", &mut open).default_width(480.0).show(ctx, |ui| {
                    egui::ScrollArea::vertical().max_height(620.0).show(ui, |ui| d.ui(ui, unit));
                    use crate::dialog::Kind;
                    match crate::dialog::buttons(
                        ui,
                        &[("OK", Kind::Main), ("Apply", Kind::Other), ("Cancel", Kind::Cancel)],
                    ) {
                        Some(0) => {
                            apply = true;
                            keep = false;
                        }
                        Some(1) => apply = true,
                        Some(_) => keep = false,
                        None => {}
                    }
                });
                keep &= open;
                if apply {
                    let props = d.props();
                    self.editor.seal();
                    let _ = ops::apply_para_props(&mut self.editor, &props, "Paragraph");
                    self.editor.seal();
                }
            }
            Dialog::PageSetup(d) => {
                let mut open = true;
                let mut apply = false;
                let sections = self.editor.doc.sections.len();
                let rich: Vec<(bool, xenify_wp_doc::HfSlot)> = self
                    .editor
                    .doc
                    .sections
                    .get(d.section)
                    .map(|s| s.headers.iter().map(|h| (h.footer, h.slot)).collect())
                    .unwrap_or_default();
                let mut plain = None;
                crate::dialog::window(ctx, "Page setup", &mut open).default_width(460.0).show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(620.0)
                        .show(ui, |ui| d.ui(ui, &mut self.prefs.unit, sections));
                    // Rich headers replace the lines above: say so.
                    for (footer, slot) in &rich {
                        ui.horizontal(|ui| {
                            let which = match slot {
                                xenify_wp_doc::HfSlot::Default => "",
                                xenify_wp_doc::HfSlot::First => "first page ",
                                xenify_wp_doc::HfSlot::Even => "even page ",
                            };
                            let kind = if *footer { "footer" } else { "header" };
                            ui.weak(format!(
                                "The {which}{kind} has its own content: double-click it on the page to edit."
                            ));
                            if ui.small_button("Make plain").on_hover_text("Use the line above instead").clicked() {
                                plain = Some((*footer, *slot));
                            }
                        });
                    }
                    use crate::dialog::Kind;
                    match crate::dialog::buttons(
                        ui,
                        &[("OK", Kind::Main), ("Apply", Kind::Other), ("Cancel", Kind::Cancel)],
                    ) {
                        Some(0) => {
                            apply = true;
                            keep = false;
                        }
                        Some(1) => apply = true,
                        Some(_) => keep = false,
                        None => {}
                    }
                });
                keep &= open;
                if let Some((footer, slot)) = plain {
                    let _ = xenify_wp_doc::hf::make_plain(&mut self.editor, d.section, footer, slot);
                }
                if apply {
                    let d = d.clone();
                    self.apply_page_setup(&d);
                }
            }
            Dialog::TableProps(d) => {
                // Changes show in the document as they're made; closing the window keeps them.
                let mut open = true;
                let mut outcome = crate::table_dialog::Outcome::Nothing;
                crate::dialog::window(ctx, "Table properties", &mut open).show(ctx, |ui| {
                    outcome = d.ui(ui);
                });
                keep &= open && d.sync(&mut self.editor);
                match outcome {
                    crate::table_dialog::Outcome::Done => keep = false,
                    crate::table_dialog::Outcome::Cancel => {
                        d.revert(&mut self.editor);
                        keep = false;
                    }
                    crate::table_dialog::Outcome::Nothing => {}
                }
            }
            Dialog::GoTo(text) => keep = crate::goto::dialog(self, ctx, text),
            Dialog::Properties(title, author) => {
                let mut open = true;
                let mut ok = false;
                crate::dialog::window(ctx, "Document properties", &mut open).default_width(380.0).show(ctx, |ui| {
                    egui::Grid::new("doc-props").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Title");
                        let r = ui.add(egui::TextEdit::singleline(title).desired_width(260.0));
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Title"));
                        ui.end_row();
                        ui.label("Author");
                        let r = ui.add(egui::TextEdit::singleline(author).desired_width(260.0));
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Author"));
                        ui.end_row();
                    });
                    ui.weak("Insert › Field › Document title and Author show these in the text.");
                    use crate::dialog::Kind;
                    match crate::dialog::buttons(ui, &[("OK", Kind::Main), ("Cancel", Kind::Cancel)]) {
                        Some(0) => ok = true,
                        Some(_) => keep = false,
                        None => {}
                    }
                });
                keep &= open;
                if ok {
                    let mut meta = self.editor.doc.meta.clone();
                    if (meta.title.as_str(), meta.author.as_str()) != (title.as_str(), author.as_str()) {
                        meta.title = title.trim().to_string();
                        meta.author = author.trim().to_string();
                        let _ = ops::set_meta(&mut self.editor, meta, "Document properties");
                    }
                    keep = false;
                }
            }
            Dialog::WordCount => {
                let mut open = true;
                let d = &self.editor.doc;
                let (words, chars) = (d.word_count(), d.char_count());
                let no_spaces: usize =
                    d.text_paragraphs().map(|p| p.text.chars().filter(|c| !c.is_whitespace()).count()).sum();
                let paras = d.text_paragraphs().filter(|p| !p.text.trim().is_empty()).count();
                let pages = self.view.layout.pages.len();
                let sel = ops::selection_counts(&self.editor);
                let texts: Vec<&str> = d.text_paragraphs().map(|p| p.text.as_str()).collect();
                let r = xenify_wp_check::readability(&texts);
                let mut close = false;
                crate::dialog::window(ctx, "Word count", &mut open).show(ctx, |ui| {
                    egui::Grid::new("word-count").num_columns(3).spacing([24.0, 8.0]).show(ui, |ui| {
                        ui.label("");
                        ui.label(egui::RichText::new("Document").strong());
                        ui.label(egui::RichText::new(if sel.is_some() { "Selection" } else { "" }).strong());
                        ui.end_row();
                        ui.label("Words");
                        ui.label(words.to_string());
                        ui.label(sel.map_or(String::new(), |s| s.0.to_string()));
                        ui.end_row();
                        ui.label("Characters");
                        ui.label(chars.to_string());
                        ui.label(sel.map_or(String::new(), |s| s.1.to_string()));
                        ui.end_row();
                        ui.label("Characters (no spaces)");
                        ui.label(no_spaces.to_string());
                        ui.end_row();
                        ui.label("Paragraphs");
                        ui.label(paras.to_string());
                        ui.end_row();
                        if !self.editor.doc.meta.pageless {
                            ui.label("Pages");
                            ui.label(pages.to_string());
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new("Readability").show(ui, |ui| {
                        ui.label(format!(
                            "School grade level {:.1} · reading ease {:.0} (higher is easier)",
                            r.grade, r.ease
                        ));
                        if r.sentences > 0 {
                            ui.label(format!(
                                "{} sentences, {:.1} words each on average",
                                r.sentences,
                                r.words as f32 / r.sentences as f32
                            ));
                        }
                    });
                    close = crate::dialog::buttons(ui, &[("Close", crate::dialog::Kind::Cancel)]).is_some();
                });
                keep = open && !close;
            }
            Dialog::Shortcuts => {
                let mut open = true;
                let mut close = false;
                crate::dialog::window(ctx, "Keyboard shortcuts", &mut open).default_size([520.0, 520.0]).show(
                    ctx,
                    |ui| {
                        egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                            for (group, keys) in SHORTCUTS {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(*group).strong());
                                egui::Grid::new(*group).num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                                    for (k, what) in *keys {
                                        ui.monospace(*k);
                                        ui.label(*what);
                                        ui.end_row();
                                    }
                                });
                            }
                        });
                        close = crate::dialog::buttons(ui, &[("Close", crate::dialog::Kind::Cancel)]).is_some();
                    },
                );
                keep = open && !close;
            }
            Dialog::About => {
                let mut open = true;
                let mut close = false;
                crate::dialog::window(ctx, "About Xenify Write", &mut open).show(ctx, |ui| {
                    ui.heading("Xenify Write");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.label("A word processor built on parley and Vello. MIT licensed.");
                    ui.label("Fonts: Inter (SIL Open Font License).");
                    close = crate::dialog::buttons(ui, &[("Close", crate::dialog::Kind::Cancel)]).is_some();
                });
                keep = open && !close;
            }
        }
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
        if let Some(d) = next {
            self.dialog = Some(d);
        }
        if let Some(t) = then {
            self.run(t);
        }
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

impl WriteApp {
    /// One window's contents: the main window (`main`), or another one
    /// (its documents swapped in by [`crate::windows`]).
    pub(crate) fn window_ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, main: bool) {
        let ctx = &ui.ctx().clone();
        let title = format!("{}{} — Xenify Write", if self.editor.is_dirty() { "• " } else { "" }, self.file_name());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if std::mem::take(&mut self.refocus) {
            self.focus_document(ctx);
        }
        crate::dialog::set_own_windows(ctx, !self.prefs.dialogs_inside);
        if main && ctx.input(|i| i.viewport().close_requested()) && !self.quitting && !self.unsaved_tabs().is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ask_quit();
        }
        if main && self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.dialog.is_none() && !self.settings.open && self.print_preview.is_none() {
            self.menu_keyboard(ctx);
            self.shortcuts(ctx);
        }
        // Files dropped on the window: pictures go in where they land, documents open.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        let drop_at = ctx.input(|i| i.pointer.hover_pos());
        for path in dropped {
            self.drop_file(&path, drop_at);
        }
        // Menus, then tabs, as in a web browser: both on the strip's colour.
        let strip = egui::Frame::NONE.fill(crate::tabs::strip_fill(ui.visuals()));
        egui::Panel::top("menu")
            .frame(strip.inner_margin(egui::Margin { left: 6, right: 6, top: 2, bottom: 0 }))
            .show_separator_line(false)
            .show(ui, |ui| self.menu_bar(ui));
        if self.print_preview.is_some() {
            // The print screen takes the window until Esc or Back.
            crate::print_preview::show(self, ui, frame);
            self.dialogs(ctx);
            return;
        }
        egui::Panel::top("tabs").frame(strip).show_separator_line(false).show(ui, |ui| crate::tabs::strip(self, ui));
        if !self.prefs.hide_toolbar {
            egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        }
        let rulers = !self.prefs.hide_ruler && self.merge.preview.is_none();
        if rulers {
            egui::Panel::top("ruler")
                .exact_size(crate::ruler::SIZE)
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
                .show(ui, |ui| crate::ruler::horizontal(self, ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        // Find sits just above the status bar, as in a web browser.
        if self.find.open {
            egui::Panel::bottom("find").show(ui, |ui| crate::find::bar(self, ui));
            let marks = self.find.matches(&self.editor.doc).to_vec();
            self.view.find_marks = marks;
            self.view.find_current = Some(self.find.current);
        } else if !self.view.find_marks.is_empty() {
            self.view.find_marks.clear();
        }
        // The panels: navigation on the left, then one dock, as tabs.
        crate::nav::panel(self, ui);
        crate::dock::show(self, ui);
        if main {
            crate::assistant::poll(self);
        }
        self.editor.doc.user.clone_from(&self.user_name);
        // At start: unsaved work a crashed run left for a never-saved document.
        if main && !self.recovery_checked {
            self.recovery_checked = true;
            if self.path.is_none() && self.dialog.is_none() {
                self.offer_recovery();
            }
        }
        let dirty = self.editor.is_dirty();
        self.recovery.tick(&self.editor.doc, dirty);
        if main {
            self.tick_background_autosaves();
        }
        self.proofing.poll(ctx, &self.editor.doc);
        if main {
            crate::settings::window(self, ctx);
            crate::proofing::why_window(self, ctx);
            crate::proofing::profile_window(self, ctx);
        }
        crate::equation::window(self, ctx, frame);
        if let Some(at) = self.view.equation_click.take() {
            self.edit_equation(at, false);
        }
        if rulers && !self.editor.doc.meta.pageless {
            egui::Panel::left("vruler")
                .exact_size(crate::ruler::SIZE)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
                .show(ui, |ui| crate::ruler::vertical(self, ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            crate::split::show(self, ui, frame, |app, ui| {
                let ai = crate::assistant::marked(app);
                match &mut app.merge.preview {
                    Some((_, preview)) => {
                        app.view.read_only = true;
                        app.view.ui(ui, preview, Some(frame), &[], &[]);
                    }
                    None => match app.review.shown_doc(&app.editor.doc) {
                        // No markup / Original: a read-only copy.
                        Some(shown) => {
                            app.view.read_only = true;
                            app.view.ui(ui, shown, Some(frame), &[], &[]);
                        }
                        None => {
                            app.view.read_only = false;
                            app.view.ui(ui, &mut app.editor, Some(frame), &app.proofing.issues, &ai);
                        }
                    },
                }
            });
        });
        // Right-click on a squiggle: its menu; elsewhere the document's menu.
        if let Some((pos, at)) = self.view.context_click.take() {
            self.menu_pos = Some(pos);
            self.menu_generated = self.view.layout.generated_at(self.view.context_doc);
            self.menu_issue =
                crate::proofing::issue_at(&self.proofing.issues, pos.para, pos.offset).cloned().map(|i| (i, at));
            // Outside the selection, the caret goes where you right-clicked
            // (so the menu acts on that word, change or cell).
            let in_table_sel = xenify_wp_doc::tables::cell_at(&self.editor, pos.para).is_some_and(|(t, r, c)| {
                xenify_wp_doc::tables::selected_cells(&self.editor)
                    .is_some_and(|(st, rows, cols)| st == t && rows.contains(&r) && cols.contains(&c))
            });
            let (a, b) = self.editor.doc.ordered(self.editor.sel);
            let in_text_sel = !self.editor.sel.is_collapsed()
                && self.editor.doc.ordered(xenify_wp_doc::Selection { anchor: a, focus: pos }).0 == a
                && self.editor.doc.ordered(xenify_wp_doc::Selection { anchor: pos, focus: b }).0 == pos;
            if !in_table_sel && !in_text_sel {
                self.editor.sel = xenify_wp_doc::Selection::caret(pos);
            }
        }
        if let Some(r) = self.view.response.clone() {
            r.context_menu(|ui| {
                // A squiggle's suggestions first, then the usual menu.
                if let Some((issue, _)) = self.menu_issue.clone()
                    && self.proofing.issues.contains(&issue)
                {
                    if crate::proofing::issue_menu(self, ui, &issue) {
                        self.menu_issue = None;
                        ui.close();
                        return;
                    }
                    ui.separator();
                }
                // A contents table, bibliography… : its settings first.
                if let Some(t) = self.menu_generated.and_then(|id| self.generated_block(id)) {
                    ui.label(egui::RichText::new(generated_name(t.kind)).strong());
                    self.generated_settings(ui, &t, false);
                    ui.separator();
                }
                self.document_menu(ui);
            });
        }
        let fresh = self.view.control_click.is_some();
        if let Some(click) = self.view.control_click.take() {
            self.control_menu = Some(click);
        }
        self.control_list(ctx, fresh);
        self.dialogs(ctx);
        self.borders_window(ctx);
        self.table_toolbar(ctx);
        self.image_panel(ctx);
        self.frame_panel(ctx);
        if main {
            crate::assistant::consent_dialog(self, ctx);
            crate::merge_panel::windows(self, ctx);
            crate::merge_panel::dialog(self, ctx);
            crate::styles_panel::dialog(self, ctx);
        }
    }
}

impl eframe::App for WriteApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::theme::apply_display(ui.ctx(), self.prefs.appearance, &self.prefs.display);
        self.window_ui(ui, frame, true);
        crate::windows::show_others(self, &ui.ctx().clone(), frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.zoom = Some(self.view.zoom);
        self.prefs.user_name = self.user_name.clone();
        self.prefs.autoformat = Some(self.view.autoformat.clone());
        self.prefs.autocorrect = Some(self.view.autocorrect.clone());
        self.prefs.show_styles = self.show_styles;
        self.prefs.show_nav = self.nav.show;
        self.prefs.dictionary_follow = self.dictionary.follow;
        self.remember_session();
        self.prefs.show_proofing = self.show_proofing;
        self.prefs.show_ai = self.show_ai;
        self.prefs.ai_consent = self.assistant.consent;
        self.prefs.proofing = self.proofing.settings.clone();
        self.prefs.spotter = self.proofing.spotter.clone();
        self.prefs.dialect_chosen = !self.proofing.dialect_auto;
        eframe::set_value(storage, "prefs", &self.prefs);
    }
}

/// What a generated list is called in menus.
pub(crate) fn generated_name(kind: xenify_wp_doc::TocKind) -> &'static str {
    use xenify_wp_doc::TocKind;
    match kind {
        TocKind::Contents => "Table of contents",
        TocKind::Figures => "List of figures",
        TocKind::Bibliography => "Bibliography",
        TocKind::ChangeLog => "Change log",
    }
}

/// Cell colours (our own picks): soft tints and a few strong ones.
pub(crate) const CELL_COLOURS: [[u8; 3]; 12] = [
    [0xf1, 0xf3, 0xf5],
    [0xdd, 0xe6, 0xf3],
    [0xe3, 0xf2, 0xe6],
    [0xfd, 0xf0, 0xd5],
    [0xfb, 0xe1, 0xe1],
    [0xec, 0xe4, 0xf7],
    [0xcf, 0xdd, 0xfb],
    [0xc8, 0xe6, 0xc9],
    [0xff, 0xe0, 0x82],
    [0xf8, 0xbb, 0xd0],
    [0x43, 0x48, 0x52],
    [0x1f, 0x6f, 0xd1],
];

/// Soft colours for page backgrounds and paragraph shading (our own picks).
const PAGE_COLOURS: [[u8; 3]; 10] = [
    [0xfd, 0xf6, 0xe3],
    [0xf4, 0xf1, 0xea],
    [0xee, 0xf4, 0xfb],
    [0xec, 0xf6, 0xee],
    [0xfb, 0xee, 0xf1],
    [0xf2, 0xee, 0xfa],
    [0xff, 0xf8, 0xd6],
    [0xe8, 0xea, 0xed],
    [0xd6, 0xe4, 0xf5],
    [0x2b, 0x2f, 0x36],
];

/// A row of colour swatches; the one clicked.
pub(crate) fn swatches(
    ui: &mut egui::Ui,
    colours: &[[u8; 3]],
    current: Option<xenify_wp_doc::Rgba>,
) -> Option<xenify_wp_doc::Rgba> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for [r, g, b] in colours {
            let c = Color32::from_rgb(*r, *g, *b);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 4.0, c);
            let on = current.is_some_and(|x| x.0[..3] == [*r, *g, *b]);
            let stroke = egui::Stroke::new(if on { 2.0 } else { 1.0 }, ui.visuals().widgets.inactive.bg_stroke.color);
            ui.painter().rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
            if resp.on_hover_text(format!("#{r:02x}{g:02x}{b:02x}")).clicked() {
                picked = Some(xenify_wp_doc::Rgba::rgb(*r, *g, *b));
            }
        }
    });
    picked
}

/// The computer's user name, capitalised ("sam" → "Sam"), for comments.
fn default_user_name() -> String {
    let raw = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    let mut c = raw.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => "Author".into(),
    }
}

/// Help › Keyboard shortcuts.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Menus",
        &[
            ("Alt+F, Alt+E, Alt+V…", "Open a menu (the underlined letter)"),
            ("F10", "Open the File menu"),
            ("↑ ↓ ← →", "Move through the menus"),
            ("Enter", "Choose"),
            ("Esc", "Close the menu"),
        ],
    ),
    (
        "Files",
        &[
            ("Ctrl+N", "New document"),
            ("Ctrl+O", "Open"),
            ("Ctrl+S", "Save"),
            ("Ctrl+Shift+S", "Save as"),
            ("Ctrl+P", "Print"),
            ("Ctrl+T", "New tab"),
            ("Ctrl+W", "Close the tab"),
            ("Ctrl+Tab", "Next tab (Ctrl+Shift+Tab: the one before)"),
            ("Ctrl+PageDown / PageUp", "Next / previous tab"),
            ("Ctrl+1…8, Ctrl+9", "That tab, the last tab"),
            ("Ctrl+.", "Symbols & emoji"),
            ("Ctrl+Shift+N", "Navigation"),
            ("Ctrl+Q", "Quit"),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
            ("Ctrl+X, Ctrl+C, Ctrl+V", "Cut, copy, paste"),
            ("Ctrl+Shift+V", "Paste as plain text"),
            ("Ctrl+A", "Select all"),
            ("Ctrl+Backspace", "Delete the word before"),
            ("Ctrl+F / Ctrl+H", "Find / replace (the bar along the bottom)"),
            ("Enter / Shift+Enter", "In find: next / previous match"),
            ("Ctrl+G", "Go to page"),
        ],
    ),
    (
        "Text",
        &[
            ("Ctrl+B, Ctrl+I, Ctrl+U", "Bold, italic, underline"),
            ("Ctrl+. / Ctrl+,", "Superscript / subscript"),
            ("Ctrl+\\", "Clear formatting"),
            ("Ctrl+K", "Link"),
            ("Ctrl+Alt+0…6", "Normal text, headings 1–6"),
            ("Ctrl+L, Ctrl+E, Ctrl+R, Ctrl+J", "Align left, centre, right, justify"),
            ("Ctrl+Shift+8 / 7", "Bulleted / numbered list"),
            ("Tab / Shift+Tab", "Indent / outdent a list item"),
        ],
    ),
    (
        "Typing shortcuts (at the start of a line)",
        &[
            ("* or -", "Bulleted list"),
            ("1. or a.", "Numbered list"),
            ("[ ]", "Checklist"),
            ("# … ######", "Headings 1–6"),
            (">", "Quote"),
        ],
    ),
    (
        "Document",
        &[
            ("Ctrl+Enter", "Page break"),
            ("Ctrl+Alt+F / Ctrl+Alt+D", "Footnote / endnote"),
            ("Ctrl+Alt+M", "New comment"),
            ("Ctrl+Shift+E", "Track changes on or off"),
            ("Ctrl+Shift+Enter", "AI: continue writing"),
            ("Ctrl+= / Ctrl+- / Ctrl+0", "Zoom in / out / back to 100%"),
        ],
    ),
];
