//! Modal dialogs and the background export job.
//!
use crate::app::XenifyApp;
use crate::file_browser::{FileBrowser, Mode};
use xenify_doc::{CompSettings, Composition};
use xenify_export::target::ExportTarget;
use xenify_export::{ExportError, ExportOptions, export_to_file};
use xenify_ops::Command;
use xenify_platform::VideoFormat;
use xenify_render::Offscreen;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

pub enum Dialog {
    Open {
        browser: FileBrowser,
    },
    SaveAs {
        browser: FileBrowser,
    },
    /// Any format: taken from the file name's extension, or picked.
    Export {
        browser: FileBrowser,
        target: ExportTarget,
        scale: f64,
        transparent: bool,
        /// Animated SVG / GIF: repeat forever.
        looping: bool,
        /// Videos and PNGs: every slide of the presentation, not just this composition.
        whole_deck: bool,
        /// PDF: speaker notes under each slide.
        notes: bool,
        /// Size, sound and slide range (videos and PNGs).
        extra: Box<ExportExtra>,
    },
    CompSettings {
        settings: CompSettings,
    },
    NewDocument(Box<crate::new_dialog::NewDialog>),
    PositionSize(Box<crate::position_size::PositionSize>),
    /// Picks the picture for a shape's pattern fill.
    PatternImage {
        browser: FileBrowser,
        layer: xenify_doc::Id,
        index: usize,
    },
    /// Trace a picture layer into vector shapes, placed over it.
    TraceLayer {
        layer: xenify_doc::Id,
        opts: xenify_export::trace::TraceOptions,
    },
    /// Exact size for a new shape (click with a shape tool).
    ShapeSize(crate::shape_dialog::ShapeSize),
    /// AI ▸ New Slide / Modify This Slide.
    AiPrompt(crate::ai_create::AiPrompt),
    /// Print the frame at the playhead. `printers` is empty when printing isn't set up.
    Print {
        printers: Vec<String>,
        printer: usize,
        copies: u32,
        unavailable: Option<String>,
    },
    Import {
        browser: FileBrowser,
    },
    Soundtrack(crate::soundtrack_ui::State),
    MusicBed(Box<crate::sound_gen_ui::MusicState>),
    ImageTrace {
        browser: FileBrowser,
        opts: xenify_export::trace::TraceOptions,
    },
    Illustrate {
        prompt: String,
        colors: u8,
        /// Keep the painting as an image layer instead of tracing it.
        raster: bool,
        job: Option<mpsc::Receiver<Result<Vec<u8>, String>>>,
    },
    ExportSlices {
        browser: FileBrowser,
        kind: SliceKind,
        scale: f64,
        transparent: bool,
    },
}

/// What each slice exports as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliceKind {
    /// The current frame as PNG.
    Png,
    /// The current frame as SVG.
    Svg,
    /// The whole animation.
    Video(VideoFormat),
}

impl SliceKind {
    fn ext(self) -> &'static str {
        match self {
            SliceKind::Png => "png",
            SliceKind::Svg => "svg",
            SliceKind::Video(VideoFormat::Mp4) => "mp4",
            SliceKind::Video(VideoFormat::WebM) => "webm",
            SliceKind::Video(VideoFormat::Gif) => "gif",
            SliceKind::Video(VideoFormat::PngSequence) => "",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SliceKind::Png => "PNG (this frame)",
            SliceKind::Svg => "SVG (this frame)",
            SliceKind::Video(VideoFormat::Mp4) => "MP4 video",
            SliceKind::Video(VideoFormat::WebM) => "WebM video",
            SliceKind::Video(VideoFormat::Gif) => "GIF animation",
            SliceKind::Video(VideoFormat::PngSequence) => "PNG sequence",
        }
    }
}

/// An export waiting for the one before it to finish (slices export one by one).
/// Background work: reports how many steps are done (it stops early when
/// that returns false) and ends with the status line to show.
type Work = Box<dyn FnOnce(&mut dyn FnMut(u64) -> bool) -> Result<String, String> + Send>;

/// An export waiting for the one before it.
pub struct QueuedExport {
    /// "Slice 2 of 5", "Slides", … (shown over the progress bar).
    label: String,
    path: PathBuf,
    /// Steps (frames, slides, pages) in all.
    total: u64,
    work: Work,
}

const PROJECT_EXT: &[&str] = &["xenify"];

/// The Export dialog's size, sound and slide choices.
#[derive(Clone, Debug)]
pub struct ExportExtra {
    /// An exact size (a preset or custom) instead of the canvas × scale.
    pub size: Option<(u32, u32)>,
    /// Which preset `size` came from (None: custom).
    pub preset: Option<usize>,
    pub letterbox: xenify_doc::Color,
    pub with_sound: bool,
    /// First and last slide (counting from 0) of a presentation.
    pub slides: (usize, usize),
}

impl ExportExtra {
    fn new(app: &XenifyApp) -> Self {
        Self {
            size: None,
            preset: None,
            letterbox: xenify_doc::Color::BLACK,
            with_sound: true,
            slides: (0, app.project().slides.len().saturating_sub(1)),
        }
    }

    /// The export settings these choices make.
    fn options(
        &self,
        app: &XenifyApp,
        format: VideoFormat,
        scale: f64,
        transparent: bool,
        deck: bool,
    ) -> ExportOptions {
        let all = (0, app.project().slides.len().saturating_sub(1));
        ExportOptions {
            format,
            scale,
            transparent,
            with_sound: self.with_sound,
            size: self.size,
            letterbox: self.letterbox,
            slides: (deck && self.slides != all).then_some(self.slides),
        }
    }
}

/// Shows the files of `target`'s type (folders only for a PNG sequence).
fn set_export_target(browser: &mut FileBrowser, target: ExportTarget) {
    match target.extension() {
        "" => browser.set_extensions(&[]),
        "png" => browser.set_extensions(&["png"]),
        "svg" => browser.set_extensions(&["svg"]),
        "json" => browser.set_extensions(&["json"]),
        "html" => browser.set_extensions(&["html"]),
        "mp4" => browser.set_extensions(&["mp4"]),
        "webm" => browser.set_extensions(&["webm"]),
        "gif" => browser.set_extensions(&["gif"]),
        other => unreachable!("unknown export extension {other}"),
    }
}

fn export_name(stem: &str, target: ExportTarget) -> String {
    match target.extension() {
        "" => format!("{stem}_frames"),
        ext => format!("{stem}.{ext}"),
    }
}

impl Dialog {
    /// The title bar's text.
    fn title(&self, app: &XenifyApp) -> String {
        match self {
            Dialog::Open { .. } => "Open project".into(),
            Dialog::SaveAs { .. } => "Save project as".into(),
            Dialog::Export { .. } => "Export".into(),
            Dialog::CompSettings { .. } => "Canvas settings".into(),
            Dialog::NewDocument(_) => "New".into(),
            Dialog::PositionSize(_) => "Position & size".into(),
            Dialog::PatternImage { .. } => "Pattern picture".into(),
            Dialog::TraceLayer { .. } => "Trace image to vectors".into(),
            Dialog::ShapeSize(d) => crate::shape_dialog::title_of(d, app),
            Dialog::AiPrompt(d) => crate::ai_create::title_of(d).into(),
            Dialog::Print { .. } => "Print".into(),
            Dialog::Import { .. } => "Import".into(),
            Dialog::Soundtrack(_) => "Soundtrack".into(),
            Dialog::MusicBed(_) => "Music bed".into(),
            Dialog::ImageTrace { .. } => "Import and trace image".into(),
            Dialog::Illustrate { .. } => "Generate illustration".into(),
            Dialog::ExportSlices { .. } => "Export slices".into(),
        }
    }

    /// Where file dialogs start: the current project's folder, else the last
    /// folder used, else the working directory.
    pub(crate) fn start_dir(app: &XenifyApp) -> PathBuf {
        app.file
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .or_else(|| app.last_dir.clone())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(crate::file_browser::home)
    }

    fn stem(app: &XenifyApp) -> String {
        app.file.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy().into_owned())
    }

    pub fn open(app: &XenifyApp) -> Self {
        Dialog::Open { browser: FileBrowser::new(Mode::Open, &Self::start_dir(app), PROJECT_EXT) }
    }

    pub fn save_as(app: &XenifyApp) -> Self {
        let mut browser = FileBrowser::new(Mode::Save, &Self::start_dir(app), PROJECT_EXT);
        browser.name = format!("{}.xenify", Self::stem(app));
        Dialog::SaveAs { browser }
    }

    pub fn export(app: &XenifyApp) -> Self {
        Self::export_as(app, ExportTarget::Video(VideoFormat::Mp4))
    }

    pub fn export_as(app: &XenifyApp, target: ExportTarget) -> Self {
        let mut browser = FileBrowser::new(Mode::Save, &Self::start_dir(app), &[]);
        set_export_target(&mut browser, target);
        browser.name = export_name(&Self::stem(app), target);
        Dialog::Export {
            browser,
            target,
            scale: 1.0,
            transparent: false,
            looping: true,
            // A presentation exports whole by default.
            whole_deck: app.project().slides.len() > 1,
            notes: false,
            extra: Box::new(ExportExtra::new(app)),
        }
    }

    pub fn new_document(app: &mut XenifyApp) -> Self {
        Dialog::NewDocument(Box::new(crate::new_dialog::NewDialog::new(app)))
    }

    /// Exact position and size of the selection (`None` without one).
    pub fn position_size(app: &mut XenifyApp) -> Self {
        match crate::position_size::PositionSize::new(app) {
            Some(d) => Dialog::PositionSize(Box::new(d)),
            None => Dialog::new_document(app),
        }
    }

    pub fn print(app: &XenifyApp) -> Self {
        let unavailable = app.platform.printer.unavailable();
        let printers = if unavailable.is_none() { app.platform.printer.printers() } else { Vec::new() };
        Dialog::Print { printers, printer: 0, copies: 1, unavailable }
    }

    pub fn import(app: &XenifyApp) -> Self {
        Dialog::Import {
            browser: FileBrowser::new(Mode::Open, &Self::start_dir(app), crate::import::IMPORT_EXTENSIONS),
        }
    }

    pub fn image_trace(app: &XenifyApp) -> Self {
        Dialog::ImageTrace {
            browser: FileBrowser::new(Mode::Open, &Self::start_dir(app), &["png", "jpg", "jpeg", "webp"]),
            opts: Default::default(),
        }
    }

    pub fn pattern_image(app: &XenifyApp, layer: xenify_doc::Id, index: usize) -> Self {
        Dialog::PatternImage {
            browser: FileBrowser::new(Mode::Open, &Self::start_dir(app), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]),
            layer,
            index,
        }
    }

    pub fn illustrate() -> Self {
        Dialog::Illustrate { prompt: String::new(), colors: 5, raster: false, job: None }
    }

    pub fn export_slices(app: &XenifyApp) -> Self {
        let mut browser = FileBrowser::new(Mode::Save, &Self::start_dir(app), &["png"]);
        browser.name = format!("{}.png", Self::stem(app));
        Dialog::ExportSlices { browser, kind: SliceKind::Png, scale: 1.0, transparent: false }
    }

    pub fn comp_settings(app: &XenifyApp) -> Self {
        Dialog::CompSettings { settings: app.comp().settings.clone() }
    }
}

pub struct ExportJob {
    done: Arc<AtomicU64>,
    total: u64,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<String, String>>,
    path: PathBuf,
    label: String,
}

impl ExportJob {
    /// Runs the export on a background thread (the window keeps working).
    fn spawn(job: QueuedExport) -> Self {
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (d, c) = (done.clone(), cancel.clone());
        let work = job.work;
        std::thread::Builder::new()
            .name("xenify-export".into())
            .spawn(move || {
                let mut progress = |n: u64| {
                    d.store(n, Ordering::Relaxed);
                    !c.load(Ordering::Relaxed)
                };
                let _ = tx.send(work(&mut progress));
            })
            .expect("spawn export thread");
        Self { done, total: job.total.max(1), cancel, result: rx, path: job.path, label: job.label }
    }
}

/// Starts `job` now, or after the export already running.
fn run_job(app: &mut XenifyApp, job: QueuedExport) {
    if app.export.is_none() {
        app.status = format!("Exporting {}…", job.path.display());
        app.export = Some(ExportJob::spawn(job));
    } else {
        app.status = format!("{} will export after the current one", job.path.display());
        app.export_queue.push(job);
    }
}

/// A video of `comp` (or of the whole presentation when `None`).
fn video_job(
    label: &str,
    project: xenify_doc::Project,
    comp: Option<Composition>,
    path: PathBuf,
    options: ExportOptions,
) -> QueuedExport {
    let total = match &comp {
        Some(c) => c.settings.frame_count(),
        None => {
            let deck = xenify_anim::deck::Deck::new(&project);
            let fps = deck.slide_comp(0).and_then(|c| project.comp(c).ok()).map_or(30.0, |c| c.settings.fps);
            let (from, to) =
                options.slides.map_or((0.0, deck.duration()), |(a, b)| (deck.slide_start(a), deck.slide_end(b)));
            ((to - from) * fps).round() as u64
        }
    };
    let p = path.clone();
    let work: Work = Box::new(move |progress| {
        let mut gpu = Offscreen::new().map_err(|e| ExportError::from(e).to_string())?;
        let progress = &mut |n, _| progress(n);
        match &comp {
            Some(comp) => export_to_file(&project, comp.id, &p, &options, &mut gpu, progress),
            None => xenify_export::export_deck_to_file(&project, &p, &options, &mut gpu, progress),
        }
        .map_err(|e| e.to_string())?;
        Ok(format!("Exported {}", p.display()))
    });
    QueuedExport { label: label.into(), path, total, work }
}

/// A job with one step: `write` makes the file's bytes.
fn file_job(path: &std::path::Path, write: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static) -> QueuedExport {
    let p = path.to_path_buf();
    let work: Work = Box::new(move |progress| {
        let bytes = write()?;
        std::fs::write(&p, bytes).map_err(|e| format!("Couldn't write {}: {e}", p.display()))?;
        progress(1);
        Ok(format!("Exported {}", p.display()))
    });
    QueuedExport { label: String::new(), path: path.to_path_buf(), total: 1, work }
}

/// Checks whether a running export finished.
pub fn poll_export(app: &mut XenifyApp, ctx: &egui::Context) {
    let Some(job) = &app.export else { return };
    let finished = match job.result.try_recv() {
        Ok(r) => Some(r),
        Err(mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            None
        }
        Err(mpsc::TryRecvError::Disconnected) => Some(Err("the export stopped unexpectedly".into())),
    };
    let Some(result) = finished else { return };
    let path = job.path.clone();
    app.export = None;
    match result {
        Ok(message) => {
            app.status = message;
            app.last_export = Some(path);
            if !app.export_queue.is_empty() {
                let next = app.export_queue.remove(0);
                app.export = Some(ExportJob::spawn(next));
            }
        }
        Err(e) => {
            let cancelled = e.to_lowercase().contains("cancel");
            if cancelled {
                app.status = "Export cancelled".into();
            } else {
                app.fail(format!("Export failed: {e}"));
            }
            app.export_queue.clear();
        }
    }
}

pub fn show(app: &mut XenifyApp, ctx: &egui::Context) {
    if let Some(job) = &app.export {
        let mut cancel = false;
        egui::Window::new("Exporting…").collapsible(false).resizable(false).show(ctx, |ui| {
            let n = job.done.load(Ordering::Relaxed);
            if !job.label.is_empty() {
                ui.strong(&job.label);
            }
            ui.label(job.path.display().to_string());
            if job.total > 1 {
                let step = if job.label == "Slides" { "slide" } else { "frame" };
                ui.add(egui::ProgressBar::new(n as f32 / job.total as f32).text(format!("{step} {n} / {}", job.total)));
            } else {
                ui.add(egui::ProgressBar::new(0.0).animate(true).text("Working…"));
            }
            let queued = app.export_queue.len();
            if queued > 0 {
                ui.weak(format!("{queued} more waiting"));
            }
            cancel = ui.button("Cancel").clicked();
        });
        if cancel {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    let Some(mut dialog) = app.dialog.take() else { return };
    let mut close = false;
    // A dialog can hand over to another (e.g. Print → Save as PNG).
    let mut dialog_swap: Option<Dialog> = None;
    let dialog_swap = &mut dialog_swap;
    let title = dialog.title(app);
    // The New dialog shows its sizes as cards beside a preview: wider.
    let width = match &dialog {
        Dialog::NewDocument(d) if d.launch => 840.0 + crate::new_dialog::LAUNCH_COLUMN,
        Dialog::NewDocument(_) => 840.0,
        _ => 640.0,
    };
    let (_, wants_close) = crate::widgets::modal(ctx, "dialog", &title, width, |ui| {
        match &mut dialog {
            Dialog::NewDocument(d) => close = crate::new_dialog::show(app, ui, d),
            Dialog::Soundtrack(s) => close = crate::soundtrack_ui::show(app, ui, s),
            Dialog::MusicBed(s) => close = crate::sound_gen_ui::music_dialog(app, ui, s),
            Dialog::PositionSize(d) => close = crate::position_size::show(app, ui, d),
            Dialog::AiPrompt(d) => close = crate::ai_create::show(app, ui, d),
            Dialog::ShapeSize(d) => close = crate::shape_dialog::show(app, ui, d),
            Dialog::Print { printers, printer, copies, unavailable } => {
                if let Some(why) = unavailable {
                    ui.label(format!("Can't print from here: {why}."));
                    ui.weak("Save the frame as a PNG instead, and print it from your image viewer.");
                    ui.horizontal(|ui| {
                        if ui.button("Save as PNG…").clicked() {
                            *dialog_swap = Some(Dialog::export_as(app, ExportTarget::PngFrame));
                            close = true;
                        }
                        close |= ui.button("Close").clicked();
                    });
                } else if printers.is_empty() {
                    ui.label("No printers are set up. Add one in your system's printer settings.");
                    close = ui.button("Close").clicked();
                } else {
                    ui.weak(format!(
                        "Prints the frame at {}, scaled to fit the page.",
                        crate::widgets::timecode(app.time, app.fps())
                    ));
                    egui::Grid::new("print").num_columns(2).show(ui, |ui| {
                        ui.label("Printer");
                        egui::ComboBox::from_id_salt("printer").selected_text(&printers[*printer]).show_ui(ui, |ui| {
                            for (i, p) in printers.iter().enumerate() {
                                ui.selectable_value(printer, i, p);
                            }
                        });
                        ui.end_row();
                        ui.label("Copies");
                        ui.add(egui::DragValue::new(copies).range(1..=99));
                        ui.end_row();
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.add(crate::widgets::primary("Print")).clicked() {
                            let opts = xenify_platform::desktop::PrintOptions {
                                printer: Some(printers[*printer].clone()),
                                copies: *copies,
                            };
                            app.status = match print_frame(app, &opts) {
                                Ok(()) => format!("Sent to {}", printers[*printer]),
                                Err(e) => format!("Couldn't print: {e}"),
                            };
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
            }
            Dialog::Open { browser } => match browser.ui(ui, "Open") {
                Some(Ok(path)) => {
                    app.last_dir = Some(browser.dir().to_path_buf());
                    app.open(path);
                    close = true;
                }
                Some(Err(())) => close = true,
                None => {}
            },
            Dialog::SaveAs { browser } => match browser.ui(ui, "Save") {
                Some(Ok(path)) => {
                    app.last_dir = Some(browser.dir().to_path_buf());
                    app.save_to(path);
                    close = true;
                }
                Some(Err(())) => close = true,
                None => {}
            },
            Dialog::Import { browser } => {
                ui.weak(
                    "Images (PNG, JPEG, WebP, GIF, BMP) are embedded in the project. SVG shapes come in as editable \
                     paths with their images and named groups; text is converted to outlines. Tip: you can also drag \
                     files onto the window or paste them.",
                );
                match browser.ui(ui, "Import") {
                    Some(Ok(path)) => {
                        app.last_dir = Some(browser.dir().to_path_buf());
                        app.import_files(vec![path], None);
                        close = true;
                    }
                    Some(Err(())) => close = true,
                    None => {}
                }
            }
            Dialog::PatternImage { browser, layer, index } => {
                ui.weak("The picture repeats to fill the shape.");
                match browser.ui(ui, "Use") {
                    Some(Ok(path)) => {
                        app.last_dir = Some(browser.dir().to_path_buf());
                        match std::fs::read(&path) {
                            Ok(data) => {
                                let name =
                                    path.file_stem().map_or("Pattern".into(), |s| s.to_string_lossy().into_owned());
                                crate::pattern_ui::set_from_file(app, *layer, *index, &data, &name);
                            }
                            Err(e) => app.fail(format!("Couldn't read {}: {e}", path.display())),
                        }
                        close = true;
                    }
                    Some(Err(())) => close = true,
                    None => {}
                }
            }
            Dialog::TraceLayer { layer, opts } => {
                ui.weak("Adds editable vector shapes over the picture (the picture stays, underneath). Works best on flat artwork: logos, icons, illustrations.");
                trace_options(ui, opts);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Trace").strong()).clicked() {
                        app.trace_image_layer(*layer, opts);
                        close = true;
                    }
                    close |= ui.button("Cancel").clicked();
                });
            }
            Dialog::ImageTrace { browser, opts } => {
                ui.weak("Turns a picture into editable vector shapes (grouped). Works best on flat artwork: logos, icons, illustrations.");
                trace_options(ui, opts);
                match browser.ui(ui, "Trace") {
                    Some(Ok(path)) => {
                        app.last_dir = Some(browser.dir().to_path_buf());
                        match std::fs::read(&path) {
                            Ok(data) => {
                                let name =
                                    path.file_stem().map_or("Traced".into(), |s| s.to_string_lossy().into_owned());
                                app.trace_image(&data, opts, &name);
                            }
                            Err(e) => app.fail(format!("Couldn't read {}: {e}", path.display())),
                        }
                        close = true;
                    }
                    Some(Err(())) => close = true,
                    None => {}
                }
            }
            Dialog::Illustrate { prompt, colors, raster, job } => {
                let model = app.assistant.image_model().to_string();
                ui.horizontal(|ui| {
                    ui.label("Result");
                    ui.add_enabled_ui(job.is_none(), |ui| {
                        ui.radio_value(raster, false, "Vector shapes")
                            .on_hover_text("Traced into editable, scalable shapes");
                        ui.radio_value(raster, true, "Image").on_hover_text("Kept as a picture (photos, detailed art)");
                    });
                });
                if *raster {
                    ui.weak(format!(
                        "An image model ({model}) paints it; it's added as an image layer. About 10–30 s."
                    ));
                } else {
                    ui.weak(format!(
                        "An image model ({model}) paints it, then Xenify Vector traces it into vector shapes. About 10–30 s."
                    ));
                }
                ui.add_enabled(
                    job.is_none(),
                    egui::TextEdit::multiline(prompt)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("e.g. a cute sitting fox, flat style"),
                );
                if !*raster {
                    ui.horizontal(|ui| {
                        ui.label("Colours");
                        ui.add_enabled(job.is_none(), egui::Slider::new(colors, 2..=8))
                            .on_hover_text("Fewer = simpler, posterised shapes; more = more detail");
                    });
                }
                let done = job.as_ref().and_then(|rx| rx.try_recv().ok());
                match done {
                    Some(Ok(bytes)) => {
                        let opts = xenify_export::trace::TraceOptions { colors: *colors, ..Default::default() };
                        let name: String = prompt
                            .split_whitespace()
                            .take(4)
                            .collect::<Vec<_>>()
                            .join(" ")
                            .trim_end_matches(|c: char| !c.is_alphanumeric())
                            .to_string();
                        let name = if name.is_empty() { "Illustration" } else { &name };
                        if *raster {
                            app.add_image(&bytes, name, None);
                        } else {
                            app.trace_image(&bytes, &opts, name);
                        }
                        close = true;
                    }
                    Some(Err(e)) => {
                        app.fail(format!("Couldn't generate: {e}"));
                        *job = None;
                    }
                    None => {}
                }
                ui.horizontal(|ui| {
                    if job.is_some() {
                        ui.spinner();
                        ui.weak("Painting…");
                        ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    } else if ui.add_enabled(!prompt.trim().is_empty(), crate::widgets::primary("Generate")).clicked() {
                        match app.assistant.start_image(prompt.trim(), !*raster, ctx) {
                            Ok(rx) => *job = Some(rx),
                            Err(e) => app.fail(e),
                        }
                    }
                    let label = if job.is_some() { "Discard" } else { "Cancel" };
                    close |= ui
                        .button(label)
                        .on_hover_text(if job.is_some() {
                            "Close without adding it (the request has already been paid for)"
                        } else {
                            ""
                        })
                        .clicked();
                });
            }
            Dialog::ExportSlices { browser, kind, scale, transparent } => {
                let n = app.comp().settings.slices.len();
                ui.weak(format!(
                    "Each of the {n} slice{} is saved next to the name you choose, as name-slice.ext.",
                    if n == 1 { "" } else { "s" }
                ));
                egui::Grid::new("slices").num_columns(2).show(ui, |ui| {
                    ui.label("Format");
                    let before = *kind;
                    egui::ComboBox::from_id_salt("slice-kind").selected_text(kind.label()).show_ui(ui, |ui| {
                        for k in [
                            SliceKind::Png,
                            SliceKind::Svg,
                            SliceKind::Video(VideoFormat::Mp4),
                            SliceKind::Video(VideoFormat::WebM),
                            SliceKind::Video(VideoFormat::Gif),
                        ] {
                            ui.selectable_value(kind, k, k.label());
                        }
                    });
                    if *kind != before {
                        browser.set_extensions(&[kind.ext()]);
                        let stem = std::path::Path::new(&browser.name)
                            .file_stem()
                            .map_or("slices".into(), |s| s.to_string_lossy().into_owned());
                        browser.name = format!("{stem}.{}", kind.ext());
                    }
                    ui.end_row();
                    if *kind != SliceKind::Svg {
                        ui.label("Scale");
                        ui.add(egui::Slider::new(scale, 0.25..=4.0).suffix("×"));
                        ui.end_row();
                        ui.label("Background");
                        ui.checkbox(transparent, "Transparent");
                        ui.end_row();
                    }
                });
                match browser.ui(ui, "Export") {
                    Some(Ok(path)) => {
                        app.last_dir = Some(browser.dir().to_path_buf());
                        app.status = export_slices(app, &path, *kind, *scale, *transparent);
                        close = true;
                    }
                    Some(Err(())) => close = true,
                    None => {}
                }
            }
            Dialog::Export { browser, target, scale, transparent, looping, whole_deck, notes, extra } => {
                // Typing a name with a known extension picks that format.
                let typed = std::path::Path::new(browser.name.trim())
                    .extension()
                    .is_some()
                    .then(|| ExportTarget::from_path(std::path::Path::new(browser.name.trim())));
                if let Some(Some(t)) = typed
                    && t.extension() != target.extension()
                {
                    *target = t;
                    set_export_target(browser, t);
                }
                egui::Grid::new("export").num_columns(2).show(ui, |ui| {
                    ui.label("Format");
                    let before = *target;
                    egui::ComboBox::from_id_salt("format").selected_text(target.label()).show_ui(ui, |ui| {
                        for t in ExportTarget::ALL {
                            ui.selectable_value(target, t, t.label());
                        }
                    });
                    if *target != before {
                        let stem = std::path::Path::new(browser.name.trim())
                            .file_stem()
                            .map_or("export".into(), |s| s.to_string_lossy().into_owned());
                        let stem = stem.strip_suffix("_frames").unwrap_or(&stem).to_string();
                        browser.name = export_name(&stem, *target);
                        set_export_target(browser, *target);
                    }
                    ui.end_row();
                    if target.is_raster() {
                        ui.label("Background");
                        ui.add_enabled_ui(target.supports_alpha(), |ui| {
                            ui.checkbox(transparent, "Transparent")
                                .on_hover_text("Leave the background out (for overlays and compositing)")
                                .on_disabled_hover_text("MP4 can't store transparency — use WebM, GIF or PNG");
                        });
                        ui.end_row();
                        ui.label("Size");
                        let s = app.comp().settings.clone();
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .selectable_label(extra.size.is_none(), "Canvas Size")
                                    .on_hover_text("The canvas size, times a scale")
                                    .clicked()
                                {
                                    extra.size = None;
                                    extra.preset = None;
                                }
                                let portrait = s.height > s.width;
                                for (i, (name, w, h)) in xenify_export::SIZE_PRESETS.iter().enumerate() {
                                    let wh = if portrait { (*h, *w) } else { (*w, *h) };
                                    if ui
                                        .selectable_label(extra.preset == Some(i), *name)
                                        .on_hover_text(format!("{}×{}", wh.0, wh.1))
                                        .clicked()
                                    {
                                        extra.size = Some(wh);
                                        extra.preset = Some(i);
                                    }
                                }
                                if ui
                                    .selectable_label(extra.size.is_some() && extra.preset.is_none(), "Custom")
                                    .clicked()
                                {
                                    extra.size = Some(extra.size.unwrap_or((s.width, s.height)));
                                    extra.preset = None;
                                }
                            });
                            match extra.size {
                                None => {
                                    ui.horizontal(|ui| {
                                        ui.add(egui::DragValue::new(scale).range(0.1..=8.0).speed(0.01).prefix("× "));
                                        let (w, h) = xenify_export::output_size(app.comp(), *scale);
                                        ui.weak(format!("{w}×{h} @ {} fps", s.fps));
                                    });
                                }
                                Some((mut w, mut h)) => {
                                    ui.horizontal(|ui| {
                                        let custom = extra.preset.is_none();
                                        ui.add_enabled(
                                            custom,
                                            egui::DragValue::new(&mut w).range(16..=16384).suffix(" px"),
                                        );
                                        ui.label("×");
                                        ui.add_enabled(
                                            custom,
                                            egui::DragValue::new(&mut h).range(16..=16384).suffix(" px"),
                                        );
                                        ui.weak(format!("@ {} fps", s.fps));
                                    });
                                    extra.size = Some((w, h));
                                    let (cw, ch) = (s.width as f64, s.height as f64);
                                    if ((w as f64 / h as f64) / (cw / ch) - 1.0).abs() > 0.005 {
                                        ui.horizontal(|ui| {
                                            ui.weak("Another shape than the canvas: bars fill the sides in");
                                            crate::color_picker::button(ui, "letterbox", &mut extra.letterbox);
                                        });
                                    }
                                }
                            }
                        });
                        ui.end_row();
                        if matches!(target, ExportTarget::Video(VideoFormat::Mp4 | VideoFormat::WebM)) {
                            ui.label("Sound");
                            ui.checkbox(&mut extra.with_sound, "Include the sound")
                                .on_hover_text("Clips, generated sounds and the soundtrack, mixed");
                            ui.end_row();
                        }
                    }
                    if *target == ExportTarget::Pdf {
                        ui.label("Pages");
                        ui.checkbox(notes, "Include speaker notes (a handout page per slide)");
                        ui.end_row();
                    }
                    if *target == ExportTarget::SvgAnimated {
                        ui.label("Playback");
                        ui.checkbox(looping, "Loop");
                        ui.end_row();
                    }
                    let slides = app.project().slides.len();
                    if slides > 1 && matches!(target, ExportTarget::Video(_) | ExportTarget::PngFrame) {
                        ui.label("Slides");
                        let what = if *target == ExportTarget::PngFrame {
                            format!("Every slide ({slides} images, each finished)")
                        } else {
                            format!("The whole presentation ({slides} slides with their transitions, no pauses)")
                        };
                        ui.checkbox(whole_deck, what);
                        ui.end_row();
                        if *whole_deck && matches!(target, ExportTarget::Video(_)) {
                            ui.label("From slide");
                            ui.horizontal(|ui| {
                                let (mut a, mut b) = (extra.slides.0 + 1, extra.slides.1 + 1);
                                ui.add(egui::DragValue::new(&mut a).range(1..=slides));
                                ui.label("to");
                                ui.add(egui::DragValue::new(&mut b).range(1..=slides));
                                extra.slides = (a.min(b) - 1, b.max(a) - 1);
                            });
                            ui.end_row();
                        }
                    }
                });
                ui.weak(match target {
                    ExportTarget::Video(VideoFormat::PngSequence) => "Frames are written into a new folder with this name.",
                    ExportTarget::PngFrame | ExportTarget::SvgFrame => "Saves the frame at the playhead.",
                    ExportTarget::SvgAnimated => {
                        "Plays in web browsers: keyframes become CSS animations with their easing (cubic-bezier, steps)."
                    }
                    ExportTarget::LottieJson => "For lottie-web and the iOS/Android Lottie players.",
                    ExportTarget::LottieHtml => "A page that plays the animation (loads the Lottie player from the internet).",
                    ExportTarget::Video(_) => "Tip: the file name's extension picks the format.",
                    ExportTarget::Pdf => "Every slide as it looks when finished, one per page (or this frame if it isn't a presentation).",
                });
                match browser.ui(ui, "Export") {
                    Some(Ok(path)) => {
                        app.last_dir = Some(browser.dir().to_path_buf());
                        if *target == ExportTarget::Pdf {
                            export_pdf(app, &path, *scale, *notes);
                        } else if *whole_deck && app.project().slides.len() > 1 {
                            export_deck(app, &path, *target, *scale, *transparent, extra);
                        } else {
                            run_export(app, &path, *target, *scale, *transparent, *looping, extra);
                        }
                        close = true;
                    }
                    Some(Err(())) => close = true,
                    _ => {}
                }
            }
            Dialog::CompSettings { settings } => {
                egui::Grid::new("comp").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut settings.name);
                    ui.end_row();
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut settings.width).range(1..=16384).suffix(" px"));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut settings.height).range(1..=16384).suffix(" px"));
                        if settings.units.unit != xenify_doc::Unit::Px {
                            let u = settings.units;
                            ui.weak(format!(
                                "= {} × {}",
                                u.format(settings.width as f64),
                                u.format(settings.height as f64)
                            ));
                        }
                    });
                    ui.end_row();
                    kind_row(ui, settings);
                    units_rows(ui, settings);
                    ui.label("Frame rate");
                    ui.add(egui::DragValue::new(&mut settings.fps).range(1.0..=240.0).suffix(" fps"));
                    ui.end_row();
                    ui.label("Duration");
                    ui.add(egui::DragValue::new(&mut settings.duration).range(0.1..=3600.0).speed(0.1).suffix(" s"));
                    ui.end_row();
                    ui.label("Background");
                    crate::color_picker::button(ui, "canvas-background", &mut settings.background);
                    ui.end_row();
                    ui.label("Motion blur");
                    ui.vertical(|ui| {
                        let mut on = settings.motion_blur.is_some();
                        if ui
                            .checkbox(&mut on, "On")
                            .on_hover_text("Blurs fast motion in exported video and AI previews (not the live canvas)")
                            .changed()
                        {
                            settings.motion_blur = on.then(xenify_doc::MotionBlur::default);
                        }
                        if let Some(mb) = &mut settings.motion_blur {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut mb.shutter_angle).range(10.0..=720.0).suffix("° shutter"),
                                );
                                ui.add(egui::DragValue::new(&mut mb.samples).range(2..=32).suffix(" samples"))
                                    .on_hover_text("More samples = smoother blur, slower export");
                            });
                        }
                    });
                    ui.end_row();
                });
                // Keyframes past the new end would be cut off.
                let last_key = app
                    .comp()
                    .layers
                    .iter()
                    .flat_map(|l| {
                        l.property_paths().into_iter().flat_map(move |p| l.keyframe_times(p).unwrap_or_default())
                    })
                    .fold(0.0f64, f64::max);
                if settings.duration + 1e-9 < last_key {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "⚠ Keyframes run to {last_key:.2} s: a {:.2} s canvas cuts off what comes after.",
                            settings.duration
                        ),
                    );
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.add(crate::widgets::primary("OK")).on_hover_text("Enter").clicked() || enter {
                        let cmd = Command::SetCompSettings { comp: app.comp, settings: settings.clone() };
                        if app.exec("Canvas settings", cmd) {
                            app.viewport.request_fit();
                            let t = app.time;
                            app.set_time(t);
                        }
                        close = true;
                    }
                    close |= ui.button("Cancel").clicked();
                });
            }
        }
    });
    // Esc / clicking outside doesn't throw away an illustration being painted
    // (it's paid for): only its button does.
    if wants_close && !matches!(dialog, Dialog::Illustrate { job: Some(_), .. }) {
        close = true;
    }
    if let Some(next) = dialog_swap.take() {
        app.dialog = Some(next);
    } else if !close {
        app.dialog = Some(dialog);
    }
}

/// Units, resolution and grid spacing rows (inside a two-column grid).
/// Animation or diagram canvas (diagram: diagram tools, light page, grid).
pub fn kind_row(ui: &mut egui::Ui, settings: &mut CompSettings) {
    use xenify_doc::CompKind;
    ui.label("Kind").on_hover_text("A diagram canvas shows the diagram tools: connectors and the shape library");
    ui.horizontal(|ui| {
        let mut kind = settings.kind;
        ui.radio_value(&mut kind, CompKind::Animation, "Animation");
        ui.radio_value(&mut kind, CompKind::Diagram, "Diagram")
            .on_hover_text("Flowcharts, network and architecture diagrams: shapes joined by connectors");
        xenify_ops::diagram::set_kind(settings, kind);
    });
    ui.end_row();
}

pub fn units_rows(ui: &mut egui::Ui, settings: &mut CompSettings) {
    let u = &mut settings.units;
    ui.label("Units")
        .on_hover_text("Rulers, guides and Position & Size use these (the picture is still made of pixels)");
    egui::ComboBox::from_id_salt("units").selected_text(u.unit.label()).show_ui(ui, |ui| {
        for unit in xenify_doc::Unit::ALL {
            ui.selectable_value(&mut u.unit, unit, unit.label());
        }
    });
    ui.end_row();
    if u.unit != xenify_doc::Unit::Px {
        ui.label("Resolution").on_hover_text("Pixels per inch: 300 for print, 96 for screens");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut u.dpi).range(10.0..=2400.0).speed(1.0).suffix(" ppi"));
            for dpi in [72.0, 96.0, 150.0, 300.0] {
                if ui.small_button(format!("{dpi}")).clicked() {
                    u.dpi = dpi;
                }
            }
        });
        ui.end_row();
    }
    ui.label("Grid").on_hover_text("Spacing of View › Show Grid");
    let units = settings.units;
    let mut g = units.from_px(settings.grid_spacing());
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::DragValue::new(&mut g).range(0.01..=100_000.0).speed(0.1).suffix(format!(" {}", units.unit.suffix())),
        );
        if r.changed() {
            settings.grid = units.to_px(g);
        }
        if settings.grid > 0.0 && ui.small_button("Auto").clicked() {
            settings.grid = 0.0;
        }
    });
    ui.end_row();
}

fn trace_options(ui: &mut egui::Ui, opts: &mut xenify_export::trace::TraceOptions) {
    egui::Grid::new("trace-opts").num_columns(2).show(ui, |ui| {
        ui.label("Colours");
        ui.add(egui::Slider::new(&mut opts.colors, 1..=8)).on_hover_text("1 = posterised, 8 = detailed");
        ui.end_row();
        ui.label("Ignore specks");
        ui.add(egui::Slider::new(&mut opts.noise, 1..=64).suffix(" px"))
            .on_hover_text("Higher = cleaner, fewer tiny shapes");
        ui.end_row();
        ui.label("Simplify");
        ui.add(egui::Slider::new(&mut opts.simplify, 0.0..=5.0).suffix(" px"));
        ui.end_row();
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut opts.smooth, "Smooth curves");
        ui.checkbox(&mut opts.remove_background, "Remove background");
    });
}

/// Writes the current composition as Lottie JSON, or an HTML player page for `.html`.
/// Renders the frame at the playhead (twice the composition size, for print
/// sharpness) to a temporary PNG and sends it to the printer.
fn print_frame(app: &XenifyApp, opts: &xenify_platform::desktop::PrintOptions) -> Result<(), String> {
    let s = &app.comp().settings;
    let scale = (4000.0 / s.width.max(s.height) as f64).clamp(1.0, 2.0);
    let (w, h) = xenify_export::output_size(app.comp(), scale);
    let mut gpu = Offscreen::new().map_err(|e| e.to_string())?;
    let img = gpu.render_frame(app.project(), app.comp, app.time, w, h, false).map_err(|e| e.to_string())?;
    let path = std::env::temp_dir().join(format!("xenify-print-{}.png", std::process::id()));
    std::fs::write(&path, img.to_png()).map_err(|e| e.to_string())?;
    app.platform.printer.print_file(&path, opts).map_err(|e| e.to_string())
}

/// The whole presentation: one video, or one PNG per slide (`name-1.png`, …).
fn export_deck(
    app: &mut XenifyApp,
    path: &std::path::Path,
    target: ExportTarget,
    scale: f64,
    transparent: bool,
    extra: &ExportExtra,
) {
    let project = app.project().clone();
    let size = extra.size;
    match target {
        ExportTarget::Video(format) => {
            let options = extra.options(app, format, scale, transparent, true);
            run_job(app, video_job("The presentation", project, None, path.to_path_buf(), options));
        }
        _ => {
            let stem = path.file_stem().map_or("slide".into(), |s| s.to_string_lossy().into_owned());
            let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let total = project.slides.len() as u64;
            let work: Work = Box::new(move |progress| {
                let mut gpu = Offscreen::new().map_err(|e| e.to_string())?;
                for (i, s) in project.slides.iter().enumerate() {
                    let c = project.comp(s.comp).map_err(|e| e.to_string())?;
                    let (w, h) = size.unwrap_or_else(|| xenify_export::output_size(c, scale));
                    let img = gpu
                        .render_frame(&project, s.comp, c.settings.duration, w, h, transparent)
                        .map_err(|e| e.to_string())?;
                    std::fs::write(dir.join(format!("{stem}-{}.png", i + 1)), img.to_png())
                        .map_err(|e| e.to_string())?;
                    if !progress(i as u64 + 1) {
                        return Err("cancelled".into());
                    }
                }
                Ok(format!(
                    "Exported {} slides to {}",
                    project.slides.len(),
                    dir.join(format!("{stem}-N.png")).display()
                ))
            });
            run_job(app, QueuedExport { label: "Slides".into(), path: path.to_path_buf(), total, work });
        }
    }
}

/// Every slide (or this frame) as a page of a PDF.
fn export_pdf(app: &mut XenifyApp, path: &std::path::Path, scale: f64, notes: bool) {
    let (project, comp, time) = (app.project().clone(), app.comp, app.time);
    let pages = project.slides.len().max(1);
    let mut job = file_job(path, move || {
        let mut gpu = Offscreen::new().map_err(|e| e.to_string())?;
        let options = xenify_export::pdf::PdfOptions { scale, notes };
        xenify_export::pdf::export_pdf(&project, comp, time, options, &mut gpu)
    });
    let p = path.display().to_string();
    let work = job.work;
    job.work = Box::new(move |progress| {
        work(progress).map(|_| format!("Exported {p} ({pages} page{})", if pages == 1 { "" } else { "s" }))
    });
    run_job(app, job);
}

/// Writes `path` as `target`, in the background.
fn run_export(
    app: &mut XenifyApp,
    path: &std::path::Path,
    target: ExportTarget,
    scale: f64,
    transparent: bool,
    looping: bool,
    extra: &ExportExtra,
) {
    let (project, comp, time) = (app.project().clone(), app.comp, app.time);
    let size = extra.size;
    let job = match target {
        ExportTarget::Video(format) => {
            let options = extra.options(app, format, scale, transparent, false);
            let c = app.comp().clone();
            video_job("", project, Some(c), path.to_path_buf(), options)
        }
        ExportTarget::PngFrame => file_job(path, move || {
            let c = project.comp(comp).map_err(|e| e.to_string())?;
            let (w, h) = size.unwrap_or_else(|| xenify_export::output_size(c, scale));
            let mut gpu = Offscreen::new().map_err(|e| e.to_string())?;
            let img = gpu.render_frame(&project, comp, time, w, h, transparent).map_err(|e| e.to_string())?;
            Ok(img.to_png())
        }),
        ExportTarget::SvgFrame => file_job(path, move || {
            let scene = xenify_anim::evaluate_project(&project, comp, time);
            Ok(xenify_export::svg::export_svg(&scene, &mut xenify_render::TextEngine::new()).into_bytes())
        }),
        ExportTarget::SvgAnimated => file_job(path, move || {
            let options = xenify_export::svg::AnimatedSvgOptions { looping };
            xenify_export::svg::export_animated_svg(&project, comp, &mut xenify_render::TextEngine::new(), options)
                .map(String::into_bytes)
        }),
        ExportTarget::LottieJson | ExportTarget::LottieHtml => {
            let html = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("html"));
            file_job(path, move || lottie_bytes(&project, comp, html))
        }
        ExportTarget::Pdf => {
            export_pdf(app, path, scale, false);
            return;
        }
    };
    run_job(app, job);
}

fn lottie_bytes(project: &xenify_doc::Project, comp: xenify_doc::Id, html: bool) -> Result<Vec<u8>, String> {
    let mut text = xenify_render::TextEngine::new();
    let json = xenify_export::lottie::export_lottie(project, comp, &mut text)?;
    let out = if html {
        let s = &project.comp(comp).map_err(|e| e.to_string())?.settings;
        xenify_export::lottie::export_html(&json, &s.name, s.background)
    } else {
        json.to_string()
    };
    Ok(out.into_bytes())
}

/// Writes every slice of the current composition next to `path`, in the
/// background (videos one after another). Returns a status line.
fn export_slices(
    app: &mut XenifyApp,
    path: &std::path::Path,
    kind: SliceKind,
    scale: f64,
    transparent: bool,
) -> String {
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = path.file_stem().map_or("slices".into(), |s| s.to_string_lossy().into_owned());
    let slices = app.comp().settings.slices.clone();
    let n = slices.len();
    let time = app.time;
    for (i, slice) in slices.iter().enumerate() {
        let safe: String =
            slice.name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        let file = dir.join(format!("{stem}-{safe}.{}", kind.ext()));
        let project = match xenify_ops::crop::cropped_project(app.project(), app.comp, slice.rect) {
            Ok(p) => p,
            Err(e) => return format!("Slice “{}”: {e}", slice.name),
        };
        let Ok(comp) = project.comp(app.comp).cloned() else { continue };
        let label = format!("Slice {} of {n}: {}", i + 1, slice.name);
        let mut job = match kind {
            SliceKind::Svg => file_job(&file, move || {
                let scene = xenify_anim::evaluate_project(&project, comp.id, time);
                Ok(xenify_export::svg::export_svg(&scene, &mut xenify_render::TextEngine::new()).into_bytes())
            }),
            SliceKind::Png => file_job(&file, move || {
                let mut gpu = Offscreen::new().map_err(|e| e.to_string())?;
                let (w, h) = xenify_export::output_size(&comp, scale);
                let img = gpu.render_frame(&project, comp.id, time, w, h, transparent).map_err(|e| e.to_string())?;
                Ok(img.to_png())
            }),
            SliceKind::Video(format) => {
                let options = ExportOptions { format, scale, transparent, ..Default::default() };
                video_job("", project, Some(comp), file.clone(), options)
            }
        };
        job.label = label;
        run_job(app, job);
    }
    format!("Exporting {n} slices to {}…", dir.display())
}
