//! Xenify Write: a word processor built on Xenify's text stack (parley
//! layout, Vello rendering).

mod a11y;
mod app;
pub mod assistant;
mod canvas;
pub mod clipboard;
mod dialog;
mod dictionary;
pub mod dock;
pub mod equation;
pub mod file_browser;
mod find;
mod goto;
pub mod icons;
pub mod images;
pub mod merge_panel;
pub mod nav;
mod page_setup;
mod paragraph_dialog;
mod print;
pub mod print_preview;
pub mod proofing;
pub mod recovery;
mod references;
mod review_panel;
mod ruler;
pub mod settings;
pub mod split;
pub mod styles_panel;
pub mod symbols;
mod table_dialog;
pub mod tabs;
pub mod templates;
pub mod theme;
mod toolbar;
pub mod view;
pub mod windows;
mod writing;

pub use app::WriteApp;

/// Opens the editor window (with `files` opened) and runs until it closes.
pub fn run(files: Vec<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Xenify Write")
            .with_app_id("xenify-write")
            .with_inner_size([1200.0, 900.0])
            .with_min_inner_size([640.0, 480.0])
            .with_icon(icons::app_icon(128)),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Xenify Write",
        options,
        Box::new(move |cc| {
            let mut app = WriteApp::new(cc, xenify_wp_layout::Fonts::new());
            // Files named on the command line, each in a tab; else what was
            // open last time (unless the user turned that off).
            if files.is_empty() {
                app.reopen_session();
            }
            for f in &files {
                app.open_path(f);
            }
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

/// Hooks for the headless GUI tests.
pub mod testing {
    /// The app with bundled fonts only (same layout on every machine), US
    /// English whatever the computer's language, and no mistake spotter
    /// unless a test turns it on.
    pub fn app(cc: &eframe::CreationContext<'_>) -> crate::WriteApp {
        crate::clipboard::set_enabled(false);
        let mut app = crate::WriteApp::new(cc, xenify_wp_layout::Fonts::bundled_only());
        app.proofing.settings.dialect = xenify_wp_check::Dialect::American;
        app.proofing.dialect_auto = false;
        app.proofing.spotter.enabled = false;
        // Tests never write to the user's data folder.
        app.recovery.off = true;
        app
    }
}
