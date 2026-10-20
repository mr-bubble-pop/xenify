//! Settings: how Xenify Write behaves for this person, kept in
//! ~/.config/xenify/write-app.json (Linux; the matching folder elsewhere).

use xenify_wp_doc::autocorrect::Autocorrect;
use xenify_wp_doc::ops::Autoformat;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Shown on comments, tracked changes and change-log entries.
    pub name: String,
    /// What Write shows when it opens: "blank" or "reopen" ("new", from when
    /// there was a New document screen, now means "blank").
    pub start: String,
    /// The ruler's units: "cm", "in" or "pt".
    pub units: String,
    /// "system", "light" or "dark".
    pub theme: String,
    pub toolbar: bool,
    pub ruler: bool,
    pub status_bar: bool,
    pub side_panel: bool,
    /// Menus and panels, in percent (the page has its own zoom).
    pub ui_size: u32,
    pub high_contrast: bool,
    pub reduce_motion: bool,
    pub autoformat: Autoformat,
    pub autocorrect: Autocorrect,
    /// Ask what changed each time a saved document is saved again (a change log).
    pub ask_change_note: bool,
    /// Extension new documents are saved as: "md", "docx" or "igdoc".
    pub save_as: String,
    pub proofing: crate::proof::ProofPrefs,
    /// Recently opened files, newest first.
    pub recent: Vec<String>,
    /// Files open when Write last closed (for "reopen").
    pub open_files: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            name: String::new(),
            start: "blank".into(),
            units: "cm".into(),
            theme: "system".into(),
            toolbar: true,
            ruler: true,
            status_bar: true,
            side_panel: true,
            ui_size: 100,
            high_contrast: false,
            reduce_motion: false,
            autoformat: Autoformat::default(),
            autocorrect: Autocorrect::default(),
            save_as: "md".into(),
            ask_change_note: false,
            proofing: Default::default(),
            recent: Vec::new(),
            open_files: Vec::new(),
        }
    }
}

fn path() -> PathBuf {
    xenify_wp_check::config_dir().join("write-app.json")
}

impl Prefs {
    pub fn load() -> Self {
        let mut p: Self = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if p.start == "new" {
            p.start = "blank".into();
        }
        p
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            // Written beside, then moved: a crash mid-write never leaves half a file.
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }

    /// Puts `file` first in the recent list.
    pub fn remember(&mut self, file: &str) {
        self.recent.retain(|f| f != file);
        self.recent.insert(0, file.to_string());
        self.recent.truncate(20);
    }
}
