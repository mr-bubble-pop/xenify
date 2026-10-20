//! A small built-in file browser.
//!
//! Native pickers (xdg-desktop-portal / GTK) are often unavailable inside
//! containers and sandboxes, so Xenify Vector ships its own that works everywhere.

use egui::{Color32, RichText};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
}

struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
}

pub struct FileBrowser {
    mode: Mode,
    dir: PathBuf,
    dir_text: String,
    /// File name being opened / saved.
    pub name: String,
    /// Extensions (lower case, no dot) to show; empty shows everything.
    extensions: Vec<&'static str>,
    entries: Vec<Entry>,
    error: Option<String>,
    show_hidden: bool,
    /// List folders only (e.g. choosing where a PNG sequence goes).
    folders_only: bool,
    confirm_overwrite: bool,
}

impl FileBrowser {
    /// `start` may be a file (its folder is shown and its name pre-filled) or a folder.
    pub fn new(mode: Mode, start: &Path, extensions: &[&'static str]) -> Self {
        let (dir, name) = if start.is_dir() {
            (start.to_path_buf(), String::new())
        } else {
            let dir = start.parent().filter(|p| p.is_dir()).map_or_else(home, Path::to_path_buf);
            let name = start.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            (dir, name)
        };
        let mut b = Self {
            mode,
            dir_text: String::new(),
            dir,
            name,
            extensions: extensions.to_vec(),
            entries: Vec::new(),
            error: None,
            show_hidden: false,
            folders_only: false,
            confirm_overwrite: false,
        };
        b.refresh();
        b
    }

    /// Shows files with these extensions; an empty list means "folders only".
    pub fn set_extensions(&mut self, extensions: &[&'static str]) {
        self.extensions = extensions.to_vec();
        self.folders_only = extensions.is_empty();
        self.refresh();
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn navigate(&mut self, dir: PathBuf) {
        match std::fs::read_dir(&dir) {
            Ok(_) => {
                self.dir = dir;
                self.refresh();
            }
            Err(e) => self.error = Some(format!("{}: {e}", dir.display())),
        }
    }

    fn refresh(&mut self) {
        self.dir_text = self.dir.display().to_string();
        self.error = None;
        self.entries.clear();
        let read = match std::fs::read_dir(&self.dir) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !self.show_hidden {
                continue;
            }
            // Follow symlinks so linked folders open like folders.
            let meta = std::fs::metadata(entry.path());
            let is_dir = meta.as_ref().is_ok_and(|m| m.is_dir());
            if !is_dir && (self.folders_only || !self.matches(&name)) {
                continue;
            }
            self.entries.push(Entry { name, is_dir, size: meta.map_or(0, |m| m.len()) });
        }
        self.entries
            .sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    }

    fn matches(&self, name: &str) -> bool {
        self.extensions.is_empty()
            || Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| self.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }

    /// The path that would be chosen right now, with the first extension
    /// added when saving a name without one.
    pub fn target(&self) -> Option<PathBuf> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let mut p = if Path::new(name).is_absolute() { PathBuf::from(name) } else { self.dir.join(name) };
        if self.mode == Mode::Save
            && p.extension().is_none()
            && let Some(ext) = self.extensions.first()
        {
            p.set_extension(ext);
        }
        Some(p)
    }

    /// Draws the browser. Returns the chosen path once the user confirms, and
    /// `Err(())` if they cancelled.
    pub fn ui(&mut self, ui: &mut egui::Ui, confirm_label: &str) -> Option<Result<PathBuf, ()>> {
        let mut result = None;
        ui.horizontal(|ui| {
            if ui.button("⬆").on_hover_text("Parent folder").clicked()
                && let Some(parent) = self.dir.parent()
            {
                self.navigate(parent.to_path_buf());
            }
            if ui.button("🏠").on_hover_text("Home").clicked() {
                self.navigate(home());
            }
            let r = ui.add(egui::TextEdit::singleline(&mut self.dir_text).desired_width(f32::INFINITY));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.navigate(PathBuf::from(self.dir_text.trim()));
            }
        });

        let mut open_dir = None;
        let mut confirm_now = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(320.0).min_scrolled_height(320.0).auto_shrink(false).show(
                ui,
                |ui| {
                    if self.entries.is_empty() {
                        ui.weak(if self.error.is_some() { "Can't read this folder." } else { "Empty folder." });
                    }
                    for e in &self.entries {
                        let selected = !e.is_dir && e.name == self.name;
                        let label = if e.is_dir {
                            RichText::new(format!("{}/", e.name)).strong()
                        } else {
                            RichText::new(e.name.as_str())
                        };
                        let r = ui
                            .horizontal(|ui| {
                                let r = ui.selectable_label(selected, label);
                                if !e.is_dir {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.weak(human_size(e.size));
                                    });
                                }
                                r
                            })
                            .inner;
                        if e.is_dir && r.clicked() {
                            open_dir = Some(self.dir.join(&e.name));
                        } else if !e.is_dir && r.clicked() {
                            self.name = e.name.clone();
                            self.confirm_overwrite = false;
                        }
                        if !e.is_dir && r.double_clicked() {
                            self.name = e.name.clone();
                            confirm_now = true;
                        }
                    }
                },
            );
        });
        if let Some(d) = open_dir {
            self.navigate(d);
        }
        if let Some(e) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }

        ui.horizontal(|ui| {
            ui.label(if self.mode == Mode::Save { "Save as" } else { "File" });
            let r = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width() - 120.0));
            if r.changed() {
                self.confirm_overwrite = false;
            }
            confirm_now |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !self.extensions.is_empty() {
                ui.weak(self.extensions.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(" "));
            }
        });
        if ui.checkbox(&mut self.show_hidden, "Show hidden files").changed() {
            self.refresh();
        }

        ui.horizontal(|ui| {
            let target = self.target();
            let ok = match (self.mode, &target) {
                (_, None) => false,
                (Mode::Open, Some(p)) => p.is_file(),
                (Mode::Save, Some(p)) => p.parent().is_some_and(Path::is_dir),
            };
            let label = if self.confirm_overwrite { "Replace" } else { confirm_label };
            if (ui.add_enabled(ok, crate::widgets::primary(label)).clicked() || (confirm_now && ok))
                && let Some(p) = target
            {
                if self.mode == Mode::Save && p.exists() && !self.confirm_overwrite {
                    self.confirm_overwrite = true;
                } else {
                    result = Some(Ok(p));
                }
            }
            if ui.button("Cancel").clicked() {
                result = Some(Err(()));
            }
            if self.confirm_overwrite {
                ui.colored_label(Color32::from_rgb(255, 200, 80), "That file exists — click Replace to overwrite it.");
            }
        });
        result
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_adds_extension_when_saving() {
        let dir = std::env::temp_dir();
        let mut b = FileBrowser::new(Mode::Save, &dir, &["xenify"]);
        b.name = "clip".into();
        assert_eq!(b.target().unwrap(), dir.join("clip.xenify"));
        b.name = "clip.json".into();
        assert_eq!(b.target().unwrap(), dir.join("clip.json"));
        b.name = "  ".into();
        assert!(b.target().is_none());
    }

    #[test]
    fn filters_by_extension_and_lists_folders_first() {
        let dir = std::env::temp_dir().join(format!("xenify-fb-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.xenify"), "{}").unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        let b = FileBrowser::new(Mode::Open, &dir, &["xenify"]);
        let names: Vec<_> = b.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["sub", "a.xenify"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
    }
}
