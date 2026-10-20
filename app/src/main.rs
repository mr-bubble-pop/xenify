//! Xenify Vector desktop entry point.

fn main() -> eframe_result::Result {
    xenify_ui::run()
}

mod eframe_result {
    pub type Result = std::result::Result<(), Box<dyn std::error::Error>>;
}
