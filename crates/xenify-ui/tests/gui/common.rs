//! Shared setup for the headless GUI tests.

use xenify_ui::testing::FakeLog;

/// The app (demo project) in a harness, with fake OS services; autosave goes
/// to a temporary directory.
pub type Harness = egui_kittest::Harness<'static, xenify_ui::XenifyApp>;

pub fn harness(log: FakeLog) -> Harness {
    let dir = std::env::temp_dir().join(format!("xenify-test-{}", std::process::id()));
    // SAFETY: every test sets the same value before any app reads it.
    unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| xenify_ui::testing::app(cc, log));
    h.run_steps(2);
    h
}

#[allow(dead_code)]
pub fn ids(h: &Harness) -> Vec<xenify_doc::Id> {
    h.state().comp().layers.iter().map(|l| l.id).collect()
}
