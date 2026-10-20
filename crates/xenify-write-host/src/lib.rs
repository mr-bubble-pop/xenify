//! Xenify Write's open documents and every call the HTML screens make on
//! them (`Docs::call`). The desktop app answers the calls through Tauri;
//! the browser build through WebAssembly. Files, printing and crash
//! recovery only exist on the desktop.

pub mod docs;
pub mod prefs;
pub mod proof;

// Printing through CUPS and crash recovery, shared with the egui Write
// (they have no toolkit code).
#[cfg(not(target_arch = "wasm32"))]
#[path = "../../xenify-wp-ui/src/print.rs"]
pub mod print;
#[cfg(not(target_arch = "wasm32"))]
#[path = "../../xenify-wp-ui/src/recovery.rs"]
pub mod recovery;

/// In the browser: no autosaves to files (a later version keeps them in the
/// browser), and dates shown as written.
#[cfg(target_arch = "wasm32")]
pub mod recovery {
    use xenify_wp_doc::Document;
    use std::path::{Path, PathBuf};

    #[derive(Clone, Debug, Default)]
    pub struct Info {
        pub path: Option<PathBuf>,
        pub title: String,
        pub time: u64,
        pub pid: u32,
    }

    #[derive(Default)]
    pub struct Autosave {
        pub session: u64,
    }

    impl Autosave {
        pub fn sibling(&self, _n: u64) -> Self {
            Self::default()
        }
        pub fn bind(&mut self, _path: Option<&Path>) {}
        pub fn tick(&mut self, _doc: &Document, _dirty: bool) {}
        pub fn clear(&mut self) {}
        pub fn left_for(&mut self, _path: &Path) -> Option<(Info, Document)> {
            None
        }
        pub fn orphans(&mut self) -> Vec<(Info, Document, String)> {
            Vec::new()
        }
        pub fn forget(&mut self, _path: Option<&Path>, _key: Option<&str>) {}
    }

    pub fn now_ms() -> u64 {
        0
    }

    /// Dates are kept as ISO 8601 text; the browser shows them as they are.
    pub fn parse_iso(s: &str) -> Option<u64> {
        (!s.is_empty()).then_some(1)
    }

    pub fn when(_ms: u64) -> String {
        String::new()
    }
}
