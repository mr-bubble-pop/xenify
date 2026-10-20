//! The Mail merge panel: data source, fields, preview and "Finish & merge".

use crate::app::WriteApp;
use crate::file_browser::{FileBrowser, Mode};
use egui::RichText;
use xenify_wp_doc::{Editor, merge};
use xenify_wp_io::Format;
use xenify_wp_io::merge_data::{self, MergeJob};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

pub enum Progress {
    Step(usize, usize),
    Done(Result<Vec<PathBuf>, String>),
}

#[derive(Default)]
pub struct MergeState {
    /// Records loaded from the data source.
    pub records: Vec<Value>,
    pub columns: Vec<String>,
    pub error: Option<String>,
    /// Showing record `n` (0-based) instead of the template.
    pub preview: Option<(usize, Editor)>,
    browser: Option<FileBrowser>,
    /// "Finish & merge" choices.
    pub format: Option<Format>,
    pub combined: bool,
    pub range: String,
    pub folder: String,
    pub finishing: bool,
    job: Option<Receiver<Progress>>,
    pub progress: Option<(usize, usize)>,
    pub last: Option<String>,
}

impl MergeState {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

fn source_path(app: &WriteApp) -> Option<PathBuf> {
    let src = app.editor.doc.meta.merge.as_ref()?.source.clone();
    if src.is_empty() {
        return None;
    }
    let p = PathBuf::from(&src);
    Some(if p.is_absolute() { p } else { app.view.base_dir.clone().unwrap_or_default().join(p) })
}

/// (Re)reads the data source.
pub fn load(app: &mut WriteApp) {
    let m = &mut app.merge;
    m.error = None;
    m.records.clear();
    m.columns.clear();
    let Some(path) = source_path(app) else { return };
    let group = app.editor.doc.meta.merge.as_ref().map(|c| c.group_by.clone()).unwrap_or_default();
    match std::fs::read(&path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|b| merge_data::records(&b, &path.to_string_lossy()))
    {
        Ok(mut recs) => {
            // Columns from the first record, before grouping.
            if let Some(Value::Object(o)) = recs.first() {
                app.merge.columns = o.keys().cloned().collect();
            }
            if !group.is_empty() {
                recs = merge_data::group(recs, &group);
            }
            app.merge.records = recs;
        }
        Err(e) => app.merge.error = Some(e),
    }
}

fn set_preview(app: &mut WriteApp, n: Option<usize>) {
    app.merge.preview = n.and_then(|i| {
        let rec = app.merge.records.get(i)?;
        let mut doc = merge::instantiate(&app.editor.doc, rec, i + 1);
        doc.revision = app.editor.doc.revision.wrapping_add(1_000_000 + i as u64);
        Some((i, Editor::new(doc)))
    });
    app.view.reset();
    app.view.base_dir = app.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
}

/// Turns the preview off (e.g. before editing).
pub fn stop_preview(app: &mut WriteApp) {
    if app.merge.preview.is_some() {
        set_preview(app, None);
    }
}

/// The panel.
pub fn panel(app: &mut WriteApp, ui: &mut egui::Ui) {
    ui.weak("Make one copy of this document per person (or row) in a spreadsheet or data file.");
    let step = |ui: &mut egui::Ui, n: u8, title: &str| {
        ui.add_space(8.0);
        ui.label(RichText::new(format!("{n}  {title}")).strong());
    };
    step(ui, 1, "Choose the data");
    let source = app.editor.doc.meta.merge.as_ref().map(|m| m.source.clone()).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(if source.is_empty() { RichText::new("No data chosen").weak() } else { RichText::new(&source) });
        if ui.button("Choose…").on_hover_text("A CSV, TSV or JSON file").clicked() {
            let start = app.view.base_dir.clone().unwrap_or_else(crate::file_browser::home);
            app.merge.browser = Some(FileBrowser::new(Mode::Open, &start, &["csv", "json", "tsv", "txt"]));
        }
        if !source.is_empty() && ui.button("Reload").clicked() {
            load(app);
        }
    });
    if let Some(e) = &app.merge.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    if !source.is_empty() {
        ui.weak(format!("{} records", app.merge.records.len()));
    }

    step(ui, 2, "Put fields in the text");
    let mut cols = app.merge.columns.clone();
    for f in merge::fields(&app.editor.doc) {
        if !cols.contains(&f) {
            cols.push(f);
        }
    }
    if cols.is_empty() {
        ui.weak("Choose the data to see its fields.");
    }
    ui.horizontal_wrapped(|ui| {
        for c in &cols {
            if ui.button(c).on_hover_text(format!("Inserts {{{{{c}}}}} at the caret")).clicked() {
                stop_preview(app);
                app.editor.seal();
                let _ = xenify_wp_doc::ops::insert_text(&mut app.editor, &format!("{{{{{c}}}}}"), None);
                app.focus_document(ui.ctx());
            }
        }
        if ui.button("Record number").on_hover_text("1 for the first copy, 2 for the second…").clicked() {
            let _ = xenify_wp_doc::ops::insert_text(&mut app.editor, "{{#}}", None);
        }
    });
    if !source.is_empty() {
        egui::CollapsingHeader::new("Advanced").id_salt("merge-advanced").show(ui, |ui| {
            let mut group = app.editor.doc.meta.merge.as_ref().map(|m| m.group_by.clone()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("One copy per");
                egui::ComboBox::from_id_salt("merge-group")
                    .selected_text(if group.is_empty() { "row".to_string() } else { format!("different {group}") })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut group, String::new(), "row");
                        for c in app.merge.columns.clone() {
                            ui.selectable_value(&mut group, c.clone(), format!("different {c} (its rows listed together)"));
                        }
                    });
            });
            if app.editor.doc.meta.merge.as_ref().is_some_and(|m| m.group_by != group) {
                if let Some(m) = app.editor.doc.meta.merge.as_mut() {
                    m.group_by = group;
                }
                app.editor.doc.revision += 1;
                load(app);
            }
            ui.weak("If a field is empty, show something else: {{name|Customer}}. Fields inside fields (JSON): {{address.city}}.");
        });
    }

    step(ui, 3, "Check and finish");
    let n = app.merge.records.len();
    ui.horizontal(|ui| {
        let mut on = app.merge.preview.is_some();
        if ui.add_enabled(n > 0, egui::Checkbox::new(&mut on, "Preview results")).changed() {
            set_preview(app, on.then_some(0));
        }
        if let Some((i, _)) = app.merge.preview.as_ref().map(|(i, _)| (*i, ())) {
            if ui.add_enabled(i > 0, egui::Button::new("◀")).clicked() {
                set_preview(app, Some(i - 1));
            }
            ui.label(format!("{} / {n}", i + 1));
            if ui.add_enabled(i + 1 < n, egui::Button::new("▶")).clicked() {
                set_preview(app, Some(i + 1));
            }
        }
    });
    if app.merge.preview.is_some() {
        ui.weak("Previewing: the document can't be edited until the preview is off.");
    }
    ui.separator();

    // Finish.
    ui.add_enabled_ui(n > 0 && !app.merge.busy(), |ui| {
        if ui.button(RichText::new("Finish & merge…").strong()).clicked() {
            app.merge.finishing = true;
            if app.merge.folder.is_empty() {
                let base = app.view.base_dir.clone().unwrap_or_else(crate::file_browser::home);
                app.merge.folder = base.join("merged").display().to_string();
            }
        }
    });
    if let Some((d, t)) = app.merge.progress {
        ui.add(egui::ProgressBar::new(d as f32 / t.max(1) as f32).text(format!("{d} / {t}")));
    }
    if let Some(msg) = &app.merge.last {
        ui.label(msg);
    }
}

/// The "Finish & merge" window, the data-file picker, and progress.
pub fn windows(app: &mut WriteApp, ctx: &egui::Context) {
    if let Some(b) = &mut app.merge.browser {
        let mut open = true;
        let mut chosen = None;
        let mut close = false;
        crate::dialog::window(ctx, "Choose merge data", &mut open).resizable(true).default_size([640.0, 440.0]).show(
            ctx,
            |ui| match b.ui(ui, "Use") {
                Some(Ok(p)) => chosen = Some(p),
                Some(Err(())) => close = true,
                None => {}
            },
        );
        if let Some(p) = chosen {
            // Relative to the document when it's next to it.
            let rel =
                app.view.base_dir.as_ref().and_then(|d| p.strip_prefix(d).ok()).map(Path::to_path_buf).unwrap_or(p);
            let cfg = app.editor.doc.meta.merge.get_or_insert_with(Default::default);
            cfg.source = rel.display().to_string();
            app.editor.doc.revision += 1;
            app.merge.browser = None;
            load(app);
        } else if close || !open {
            app.merge.browser = None;
        }
    }
    if app.merge.finishing {
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        crate::dialog::window(ctx, "Finish & merge", &mut open).show(ctx, |ui| {
            let m = &mut app.merge;
            let mut f = m.format.unwrap_or(Format::Pdf);
            egui::ComboBox::from_label("Format").selected_text(f.name()).show_ui(ui, |ui| {
                for x in [Format::Pdf, Format::Docx, Format::Markdown, Format::Html, Format::Igdoc, Format::Text] {
                    ui.selectable_value(&mut f, x, x.name());
                }
            });
            m.format = Some(f);
            ui.radio_value(&mut m.combined, false, "One file per record");
            ui.radio_value(&mut m.combined, true, "One file with every record");
            if !m.combined {
                let cfg = app.editor.doc.meta.merge.get_or_insert_with(Default::default);
                ui.horizontal(|ui| {
                    ui.label("File names");
                    ui.add(egui::TextEdit::singleline(&mut cfg.file_name).hint_text("e.g. {{last}}-{{first}}"));
                });
            }
            let m = &mut app.merge;
            ui.horizontal(|ui| {
                ui.label("Records");
                ui.add(egui::TextEdit::singleline(&mut m.range).hint_text("all, or e.g. 1-10,15").desired_width(120.0));
            });
            ui.horizontal(|ui| {
                ui.label("Folder");
                ui.add(egui::TextEdit::singleline(&mut m.folder).desired_width(280.0));
            });
            use crate::dialog::Kind;
            match crate::dialog::buttons(ui, &[("Merge", Kind::Main), ("Cancel", Kind::Cancel)]) {
                Some(0) => go = true,
                Some(_) => cancel = true,
                None => {}
            }
        });
        if go {
            start(app);
            app.merge.finishing = false;
        }
        if !open || cancel {
            app.merge.finishing = false;
        }
    }
    // Progress from the worker.
    if let Some(rx) = &app.merge.job {
        while let Ok(p) = rx.try_recv() {
            match p {
                Progress::Step(d, t) => app.merge.progress = Some((d, t)),
                Progress::Done(r) => {
                    app.merge.job = None;
                    app.merge.progress = None;
                    app.merge.last = Some(match r {
                        Ok(files) => format!("Wrote {} file(s) to {}", files.len(), app.merge.folder),
                        Err(e) => format!("Merge failed: {e}"),
                    });
                    break;
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Starts the merge on a worker thread.
pub fn start(app: &mut WriteApp) {
    let doc = app.editor.doc.clone();
    let records = app.merge.records.clone();
    let job = MergeJob {
        format: app.merge.format.unwrap_or(Format::Pdf),
        combined: app.merge.combined,
        name_template: doc.meta.merge.as_ref().map(|m| m.file_name.clone()).unwrap_or_default(),
        range: app.merge.range.clone(),
    };
    let folder = PathBuf::from(&app.merge.folder);
    let base = app.view.base_dir.clone();
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("xenify-write-merge".into())
        .spawn(move || {
            let mut engine = xenify_wp_layout::Engine::new(xenify_wp_layout::Fonts::new());
            let assets = doc.assets.clone();
            let image = move |src: &str| crate::images::bytes_for_parts(src, &assets, base.as_deref());
            let tx2 = tx.clone();
            let r = merge_data::run(&doc, &records, &job, &mut engine, &image, &mut |d, t| {
                let _ = tx2.send(Progress::Step(d, t));
            })
            .map_err(|e| e.to_string())
            .and_then(|files| {
                std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
                let mut written = Vec::new();
                for (name, bytes) in files {
                    let p = folder.join(name);
                    std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
                    written.push(p);
                }
                Ok(written)
            });
            let _ = tx.send(Progress::Done(r));
        })
        .expect("thread");
    app.merge.job = Some(rx);
    app.merge.progress = Some((0, records_len(app)));
}

fn records_len(app: &WriteApp) -> usize {
    merge_data::select(&app.merge.range, app.merge.records.len()).len()
}

/// Tools › Mail merge: the steps in a dialog (a one-off task, not a panel).
pub fn dialog(app: &mut WriteApp, ctx: &egui::Context) {
    if !app.show_merge {
        return;
    }
    let mut open = true;
    crate::dialog::window(ctx, "Mail merge", &mut open).resizable(true).default_width(420.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| panel(app, ui));
    });
    if !open {
        app.show_merge = false;
        stop_preview(app);
    }
}
