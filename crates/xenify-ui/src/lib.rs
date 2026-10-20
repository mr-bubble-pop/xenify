//! The Xenify Vector editor: viewport, tools, layers, properties and timeline.
//!
//! All edits go through [`xenify_ops::Editor`], so everything is undoable and
//! the future AI assistant can drive exactly the same operations.

mod a11y;
mod actions;
mod ai_create;
mod animated_image_ui;
mod app;
mod assistant;
mod camera_ui;
mod canvas_menu;
mod clipboard;
mod color_picker;
mod colors;
mod control_bar;
mod credits;
mod curve;
mod diagram_ui;
mod dialogs;
mod doc_tabs;
mod dock;
mod documents;
mod effects_gallery;
mod effects_ui;
mod emoji_panel;
mod file_browser;
mod giphy_panel;
mod gpu_canvas;
mod gradient_ui;
mod graph;
mod import;
pub mod keymap;
mod layers_panel;
mod liquid_ui;
mod masks_ui;
mod media_ui;
mod menus;
mod modifiers_ui;
mod new_dialog;
mod panels;
mod particles_ui;
mod path_menu;
mod pattern_ui;
mod pens_panel;
mod platform;
mod position_size;
mod present;
mod presenter_view;
mod session;
pub mod settings;
mod shape_dialog;
mod shape_library;
mod shape_smart;
mod showcase;
mod slices_panel;
mod slides_panel;
mod solids;
mod sound_gen_ui;
mod sound_ui;
mod soundtrack_ui;
mod status_bar;
mod symbols;
mod templates_ui;
mod theme;
mod text_anim_ui;
mod timeline;
mod tool_options;
mod transition_ui;
mod viewport;
mod widgets;

pub use app::XenifyApp;

/// Hooks for headless GUI tests (`tests/`): the app with fake OS services.
#[doc(hidden)]
pub mod testing {
    pub use crate::app::Tool;
    pub use crate::assistant::Script;
    pub use crate::platform::{FakeLog, FakeState};

    /// The app as tests use it: the demo, without the New dialog over it.
    pub fn app(cc: &eframe::CreationContext<'_>, log: FakeLog) -> crate::XenifyApp {
        let mut app = crate::XenifyApp::with_platform(cc, crate::platform::Platform::fake(log));
        app.dialog = None;
        app
    }

    pub fn action_by_id(id: &str) -> crate::keymap::Action {
        crate::keymap::Action::from_id(id).expect("known action")
    }

    /// Brings the dock panel called `label` to the front (as its tab would).
    pub fn show_panel(app: &mut crate::XenifyApp, label: &str) {
        if let Some(p) = crate::dock::Panel::ALL.iter().find(|p| p.label() == label) {
            crate::dock::reveal(app, *p);
        }
    }

    /// Whether the dock panel called `label` (as in View ▸ Panels) is open.
    pub fn panel_open(app: &crate::XenifyApp, label: &str) -> bool {
        crate::dock::Panel::ALL.iter().any(|p| p.label() == label && crate::dock::is_open(app, *p))
    }

    /// Runs the action with id `id` (as a menu item would).
    pub fn run_action(app: &mut crate::XenifyApp, ctx: &egui::Context, id: &str) {
        crate::actions::run(app, ctx, action_by_id(id));
    }

    /// Opens File ▸ Settings on `tab` ("general", "shortcuts" or "ai").
    pub fn open_settings(app: &mut crate::XenifyApp, tab: &str) {
        use crate::settings::Tab;
        app.settings.open(match tab {
            "shortcuts" => Tab::Shortcuts,
            "ai" => Tab::Ai,
            _ => Tab::General,
        });
    }

    /// The canvas size the open New dialog would create.
    /// Opens the New dialog the way it opens at launch (with recent files and examples).
    pub fn open_launch_dialog(app: &mut crate::XenifyApp) {
        let d = crate::new_dialog::NewDialog::at_launch();
        app.dialog = Some(crate::dialogs::Dialog::NewDocument(Box::new(d)));
    }

    /// The Export dialog's exact size choice (None: canvas × scale), if it's open.
    pub fn export_size(app: &crate::XenifyApp) -> Option<Option<(u32, u32)>> {
        match &app.dialog {
            Some(crate::dialogs::Dialog::Export { extra, .. }) => Some(extra.size),
            _ => None,
        }
    }

    /// Opens the Export dialog.
    pub fn open_export(app: &mut crate::XenifyApp) {
        app.dialog = Some(crate::dialogs::Dialog::export(app));
    }

    /// Points the GIFs panel at a fake GIPHY (a key, two results, and a
    /// two-frame GIF for every download) instead of the network.
    pub fn fake_giphy(app: &mut crate::XenifyApp, with_key: bool) {
        fn gif() -> Vec<u8> {
            let mut out = Vec::new();
            {
                let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
                for c in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                    let img = image::RgbaImage::from_pixel(12, 8, image::Rgba(c));
                    let _ = enc.encode_frame(image::Frame::from_parts(
                        img,
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(200, 1),
                    ));
                }
            }
            out
        }
        fn search(key: &str, q: &str, _limit: u32, _offset: u32) -> Result<Vec<xenify_export::giphy::Gif>, String> {
            if key != "test-key" {
                return Err("GIPHY says: Unauthorized".into());
            }
            let topic = if q.is_empty() { "trending" } else { q };
            Ok((1..=2)
                .map(|n| xenify_export::giphy::Gif {
                    id: format!("{topic}-{n}"),
                    title: format!("{topic} {n}"),
                    username: "maker".into(),
                    page: String::new(),
                    preview: format!("https://example.test/{topic}-{n}-preview.gif"),
                    full: format!("https://example.test/{topic}-{n}.gif"),
                    width: 12,
                    height: 8,
                })
                .collect())
        }
        app.giphy_panel.backend = crate::giphy_panel::Backend { search, get: |_| Ok(gif()) };
        app.giphy_panel.key = Some(if with_key { "test-key".into() } else { String::new() });
    }

    /// Keeps saved slide layouts in `dir` instead of the data folder.
    pub fn set_layouts_dir(app: &mut crate::XenifyApp, dir: std::path::PathBuf) {
        app.templates.dir = Some(dir);
    }

    /// Opens Canvas Settings with the duration already changed to `duration`.
    pub fn open_comp_settings(app: &mut crate::XenifyApp, duration: f64) {
        let mut settings = app.comp().settings.clone();
        settings.duration = duration;
        app.dialog = Some(crate::dialogs::Dialog::CompSettings { settings });
    }

    /// The number shown by the pointer during a drag (W × H, Δx/Δy, %, °).
    pub fn live_numbers(app: &crate::XenifyApp) -> Option<String> {
        app.viewport.live_numbers(app)
    }

    /// Whether any dialog is open.
    pub fn dialog_open(app: &crate::XenifyApp) -> bool {
        app.dialog.is_some()
    }

    pub fn new_dialog_size(app: &crate::XenifyApp) -> Option<(u32, u32)> {
        match &app.dialog {
            Some(crate::dialogs::Dialog::NewDocument(d)) => Some((d.settings.width, d.settings.height)),
            _ => None,
        }
    }

    /// Whether closing Settings would ask to save AI changes.
    pub fn ai_settings_unsaved(app: &crate::XenifyApp) -> bool {
        crate::assistant::ai_settings_unsaved(app)
    }

    /// Answers the assistant's requests with `replies` (raw OpenRouter
    /// responses, in order) instead of calling OpenRouter; the returned
    /// script records what was sent.
    pub fn script_ai(
        app: &mut crate::XenifyApp,
        replies: Vec<serde_json::Value>,
    ) -> std::sync::Arc<std::sync::Mutex<crate::assistant::Script>> {
        let script = std::sync::Arc::new(std::sync::Mutex::new(crate::assistant::Script {
            replies: replies.into(),
            sent: Vec::new(),
        }));
        app.assistant.script = Some(script.clone());
        script
    }

    /// As if no OpenRouter key were set (whatever this machine has).
    pub fn pretend_no_ai_key(app: &mut crate::XenifyApp) {
        app.assistant.script = None;
        app.assistant.pretend_no_key();
    }

    /// Accepts the assistant's pending changes (as the panel's button does).
    pub fn accept_ai(app: &mut crate::XenifyApp) {
        crate::assistant::Assistant::accept(app);
    }

    /// Present in this window (not on another monitor) from now on.
    pub fn present_in_this_window(app: &mut crate::XenifyApp) {
        app.present_screen = Some(crate::present::Screen::ThisWindow);
    }

    /// Where a running presentation is waiting (deck time), if it is.
    pub fn presenting_waits_at(app: &crate::XenifyApp) -> Option<f64> {
        let p = app.presenter.as_ref()?;
        p.playback.until.or(Some(p.playback.t))
    }

    pub fn presenting_slide(app: &crate::XenifyApp) -> Option<usize> {
        Some(app.presenter.as_ref()?.playback.slide())
    }

    /// Layer `id`'s box on the canvas (composition pixels) now.
    pub fn world_box(app: &crate::XenifyApp, id: xenify_doc::Id) -> kurbo::Rect {
        let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
        xenify_anim::world_box(app.comp(), &scene, id, &mut xenify_anim::ApproxText).expect("visible layer")
    }

    /// Layer `id`'s own space → composition pixels, now.
    pub fn layer_to_comp(app: &crate::XenifyApp, id: xenify_doc::Id) -> kurbo::Affine {
        let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
        scene.layer(id).expect("visible layer").transform
    }

    /// Moves a layer's transform origin (as Alt+dragging the ⊕ does).
    pub fn set_origin(app: &mut crate::XenifyApp, id: xenify_doc::Id, p: kurbo::Point) -> bool {
        crate::viewport::set_origin(app, id, p)
    }

    pub fn chord(text: &str) -> crate::keymap::Chord {
        crate::keymap::Chord::parse(text).expect("valid chord")
    }
}

/// Opens the editor window and runs until it is closed.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Xenify Vector")
            .with_app_id("xenify")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([900.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    // Files named on the command line: projects open in tabs, anything else
    // (images, SVG, 3D models) is imported into a new document.
    let files: Vec<std::path::PathBuf> =
        std::env::args_os().skip(1).filter(|a| !a.to_string_lossy().starts_with('-')).map(Into::into).collect();
    eframe::run_native(
        "Xenify Vector",
        options,
        Box::new(move |cc| {
            let mut app = XenifyApp::new(cc);
            app.open_from_command_line(files);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}
