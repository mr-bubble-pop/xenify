//! References › Manage sources and Insert citation: the document's sources
//! (books, articles, web pages…), imported from BibTeX or RIS or typed in,
//! and citations of them at the caret.

use crate::file_browser::{FileBrowser, Mode};
use xenify_wp_doc::Document;
use xenify_wp_doc::cite::{self, Name, Source, SourceKind};

/// The working copy of the document's sources.
pub struct SourcesDialog {
    pub sources: Vec<Source>,
    selected: Option<usize>,
    /// Authors as typed: "Smith, Jane; Lee, Min".
    authors: String,
    import: Option<FileBrowser>,
    pub message: Option<String>,
}

/// Files sources can be imported from.
pub const SOURCE_EXT: &[&str] = &["bib", "ris", "json"];

fn authors_text(s: &Source) -> String {
    s.authors
        .iter()
        .map(|a| if a.given.is_empty() { a.family.clone() } else { format!("{}, {}", a.family, a.given) })
        .collect::<Vec<_>>()
        .join("; ")
}

fn parse_authors(t: &str) -> Vec<Name> {
    t.split(';')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|a| match a.split_once(',') {
            Some((f, g)) => Name { family: f.trim().into(), given: g.trim().into() },
            None => {
                // "Jane Smith": the last word is the family name.
                let words: Vec<&str> = a.split_whitespace().collect();
                match words.split_last() {
                    Some((f, g)) => Name { family: f.to_string(), given: g.join(" ") },
                    None => Name::default(),
                }
            }
        })
        .collect()
}

/// Sources read from a BibTeX, RIS or CSL-JSON file.
pub fn read_sources_file(path: &std::path::Path) -> Result<Vec<Source>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let found = match ext.as_str() {
        "ris" => cite::read_ris(&text),
        "json" => xenify_wp_io::read_csl_json(&text),
        _ => cite::read_bibtex(&text),
    };
    if found.is_empty() { Err("No sources found in that file.".into()) } else { Ok(found) }
}

impl SourcesDialog {
    pub fn new(doc: &Document) -> Self {
        let sources = doc.meta.sources.clone();
        let selected = (!sources.is_empty()).then_some(0);
        let authors = selected.map(|i| authors_text(&sources[i])).unwrap_or_default();
        Self { sources, selected, authors, import: None, message: None }
    }

    fn select(&mut self, i: Option<usize>) {
        self.commit_authors();
        self.selected = i;
        self.authors = i.and_then(|i| self.sources.get(i)).map(authors_text).unwrap_or_default();
    }

    fn commit_authors(&mut self) {
        if let Some(s) = self.selected.and_then(|i| self.sources.get_mut(i)) {
            s.authors = parse_authors(&self.authors);
        }
    }

    /// Adds `found`, giving any key already used a new one.
    fn add(&mut self, doc: &Document, found: Vec<Source>) -> usize {
        let n = found.len();
        for mut s in found {
            if s.key.is_empty() || self.sources.iter().any(|x| x.key == s.key) {
                let mut probe = doc.clone();
                probe.meta.sources = self.sources.clone();
                s.key = cite::new_key(&probe, &s);
            }
            self.sources.push(s);
        }
        n
    }

    /// The final list (after the authors field is read back).
    pub fn result(&mut self) -> Vec<Source> {
        self.commit_authors();
        self.sources.clone()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, start_dir: &std::path::Path) {
        if let Some(b) = &mut self.import {
            ui.heading("Import sources");
            match b.ui(ui, "Import") {
                Some(Ok(path)) => {
                    self.import = None;
                    match read_sources_file(&path) {
                        Ok(found) => {
                            let n = self.add(doc, found);
                            self.message = Some(format!("Imported {n} source{}.", if n == 1 { "" } else { "s" }));
                            self.select(Some(self.sources.len() - 1));
                        }
                        Err(e) => self.message = Some(e),
                    }
                }
                Some(Err(())) => self.import = None,
                None => {}
            }
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("New source").clicked() {
                let s = Source { key: String::new(), year: String::new(), ..Default::default() };
                let n = self.add(doc, vec![s]);
                let _ = n;
                self.select(Some(self.sources.len() - 1));
            }
            if ui
                .button("Import BibTeX / RIS…")
                .on_hover_text("Reads .bib, .ris or CSL-JSON files from a reference manager")
                .clicked()
            {
                self.import = Some(FileBrowser::new(Mode::Open, start_dir, SOURCE_EXT));
            }
            let can_delete = self.selected.is_some();
            if ui.add_enabled(can_delete, egui::Button::new("Delete")).clicked()
                && let Some(i) = self.selected
            {
                self.sources.remove(i);
                self.selected = None;
                self.select((!self.sources.is_empty()).then(|| i.min(self.sources.len() - 1)));
            }
        });
        if let Some(m) = &self.message {
            ui.weak(m);
        }
        ui.separator();
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("sources-list").max_height(380.0).show(&mut cols[0], |ui| {
                if self.sources.is_empty() {
                    ui.weak("No sources yet. Add one, or import a BibTeX or RIS file.");
                }
                let mut pick = None;
                for (i, s) in self.sources.iter().enumerate() {
                    let who = s.authors.first().map_or("(no author)", |a| a.family.as_str());
                    let label =
                        format!("{who} {} — {}", s.year, if s.title.is_empty() { "(untitled)" } else { &s.title });
                    if ui.selectable_label(self.selected == Some(i), label).clicked() {
                        pick = Some(i);
                    }
                }
                if let Some(i) = pick {
                    self.select(Some(i));
                }
            });
            let ui = &mut cols[1];
            let Some(i) = self.selected.filter(|i| *i < self.sources.len()) else {
                ui.weak("Pick a source to edit it.");
                return;
            };
            let s = &mut self.sources[i];
            egui::Grid::new("source-fields").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Type");
                egui::ComboBox::from_id_salt("source-kind").selected_text(s.kind.name()).show_ui(ui, |ui| {
                    for k in SourceKind::ALL {
                        ui.selectable_value(&mut s.kind, k, k.name());
                    }
                });
                ui.end_row();
                ui.label("Authors");
                ui.add(egui::TextEdit::singleline(&mut self.authors).hint_text("Smith, Jane; Lee, Min"));
                ui.end_row();
                let container = match s.kind {
                    SourceKind::Article => "Journal",
                    SourceKind::Chapter => "Book",
                    SourceKind::Web => "Website",
                    _ => "Series",
                };
                for (label, value, hint) in [
                    ("Title", &mut s.title, ""),
                    ("Year", &mut s.year, "2024"),
                    (container, &mut s.container, ""),
                    ("Publisher", &mut s.publisher, ""),
                ] {
                    ui.label(label);
                    ui.add(egui::TextEdit::singleline(value).hint_text(hint));
                    ui.end_row();
                }
            });
            egui::CollapsingHeader::new("More details").id_salt("source-more").show(ui, |ui| {
                egui::Grid::new("source-more-fields").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    for (label, value, hint) in [
                        ("Volume", &mut s.volume, ""),
                        ("Issue", &mut s.issue, ""),
                        ("Pages", &mut s.pages, "45–67"),
                        ("DOI", &mut s.doi, "10.1000/xyz"),
                        ("Web address", &mut s.url, "https://"),
                        ("Read on", &mut s.accessed, "2024-05-01"),
                        ("Citation key", &mut s.key, "smith2020"),
                    ] {
                        ui.label(label);
                        ui.add(egui::TextEdit::singleline(value).hint_text(hint));
                        ui.end_row();
                    }
                });
            });
            if s.key.trim().is_empty() {
                let mut probe = doc.clone();
                probe.meta.sources = Vec::new();
                s.authors = parse_authors(&self.authors);
                s.key = cite::new_key(&probe, s);
            }
            ui.separator();
            ui.weak("In the bibliography:");
            let mut preview = s.clone();
            preview.authors = parse_authors(&self.authors);
            let runs = cite::entry(&preview, doc.meta.cite_style, i + 1);
            let mut job = egui::text::LayoutJob::default();
            let color = ui.visuals().text_color();
            for (t, italic) in runs {
                job.append(&t, 0.0, egui::TextFormat { italics: italic, color, ..Default::default() });
            }
            job.wrap.max_width = ui.available_width();
            ui.label(job);
        });
    }
}

/// References › Insert citation: pick sources (searchable) and a page.
pub struct CiteDialog {
    search: String,
    picked: Vec<String>,
    locator: String,
    /// Cancel was pressed.
    pub cancelled: bool,
}

impl CiteDialog {
    pub fn new() -> Self {
        Self { search: String::new(), picked: Vec::new(), locator: String::new(), cancelled: false }
    }

    /// Shows the dialog; `Some((keys, locator))` when Insert is pressed,
    /// and whether "Manage sources" was asked for.
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document) -> (Option<(Vec<String>, String)>, bool) {
        let mut manage = false;
        if doc.meta.sources.is_empty() {
            ui.label("This document has no sources yet.");
            use crate::dialog::Kind;
            match crate::dialog::buttons(ui, &[("Add or import sources…", Kind::Main), ("Cancel", Kind::Cancel)]) {
                Some(0) => manage = true,
                Some(_) => self.cancelled = true,
                None => {}
            }
            return (None, manage);
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search author, title or year")
                .desired_width(f32::INFINITY),
        );
        let q = self.search.to_lowercase();
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for s in &doc.meta.sources {
                let hay = format!("{} {} {} {}", authors_text(s), s.title, s.year, s.key).to_lowercase();
                if !q.is_empty() && !q.split_whitespace().all(|w| hay.contains(w)) {
                    continue;
                }
                let mut on = self.picked.contains(&s.key);
                let who = s.authors.first().map_or("(no author)", |a| a.family.as_str());
                if ui.checkbox(&mut on, format!("{who} ({}) {}", s.year, s.title)).changed() {
                    if on {
                        self.picked.push(s.key.clone());
                    } else {
                        self.picked.retain(|k| k != &s.key);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Page");
            ui.add(egui::TextEdit::singleline(&mut self.locator).hint_text("p. 4").desired_width(80.0));
        });
        let preview = if self.picked.is_empty() {
            "Tick the sources to cite.".to_string()
        } else {
            cite::citation(doc, &self.picked, self.locator.trim())
        };
        ui.weak(preview);
        let mut out = None;
        use crate::dialog::Kind;
        let cite = if self.picked.is_empty() { Kind::Off } else { Kind::Main };
        match crate::dialog::buttons(ui, &[("Manage sources…", Kind::Other), ("Cite", cite), ("Cancel", Kind::Cancel)])
        {
            Some(0) => manage = true,
            Some(1) => out = Some((self.picked.clone(), self.locator.trim().to_string())),
            Some(_) => self.cancelled = true,
            None => {}
        }
        (out, manage)
    }
}

impl Default for CiteDialog {
    fn default() -> Self {
        Self::new()
    }
}
