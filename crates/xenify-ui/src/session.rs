//! Keeping work safe: the "save changes?" prompt, autosave with crash
//! recovery, and the recent-files list. Every open document (tab) is
//! autosaved and recovered on its own.

use crate::app::XenifyApp;
use crate::dialogs::Dialog;
use crate::documents::DocId;
use xenify_doc::Project;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How often unsaved work is written to the autosave file.
pub const RECENT_KEY: &str = "recent_files";

/// Something to do that may first need unsaved changes dealt with.
#[derive(Clone, Debug)]
pub enum Pending {
    New,
    /// Show the Open dialog.
    Open,
    OpenPath(PathBuf),
    OpenDemo,
    Quit,
    /// Close these tabs.
    Close(Vec<DocId>),
}

pub struct Session {
    ctx: egui::Context,
    /// Waiting for the user to answer "save changes?" (or to finish Save As).
    pub pending: Option<Pending>,
    /// Unsaved documents still to ask about before `pending` runs.
    queue: Vec<DocId>,
    prompt_open: bool,
    /// Set once the user agreed to quit, so the close request goes through.
    pub allow_close: bool,
    pub recent: Vec<PathBuf>,
    last_autosave: Option<Instant>,
    /// The revision last autosaved, per document.
    autosaved: HashMap<DocId, u64>,
    /// Autosaves left behind by a session that didn't close properly.
    pub recovery: Option<Vec<Recovery>>,
}

pub struct Recovery {
    path: PathBuf,
    original: Option<PathBuf>,
    when: SystemTime,
}

impl Session {
    pub fn new(storage: Option<&dyn eframe::Storage>, ctx: egui::Context) -> Self {
        let recent: Vec<PathBuf> = storage.and_then(|s| eframe::get_value(s, RECENT_KEY)).unwrap_or_default();
        Self {
            ctx,
            pending: None,
            queue: Vec::new(),
            prompt_open: false,
            allow_close: false,
            recent,
            last_autosave: None,
            autosaved: HashMap::new(),
            recovery: Some(find_recovery()).filter(|r| !r.is_empty()),
        }
    }

    /// Puts `path` first in File ▸ Open Recent, keeping at most `max`.
    pub fn add_recent(&mut self, path: &Path, max: usize) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(max);
    }
}

/// Runs `action`, first asking to save any unsaved document it would close.
pub fn guard(app: &mut XenifyApp, action: Pending) {
    let closing = match &action {
        Pending::Quit => app.docs.ids(),
        Pending::Close(ids) => ids.clone(),
        // These open a tab of their own: nothing is lost.
        _ => Vec::new(),
    };
    let dirty = app.dirty_documents(&closing);
    if dirty.is_empty() {
        perform(app, action);
    } else {
        app.session.pending = Some(action);
        app.session.queue = dirty;
        next_question(app);
    }
}

/// Shows the next unsaved document to ask about, or runs the pending action.
fn next_question(app: &mut XenifyApp) {
    app.session.queue.retain(|id| app.docs.index(*id).is_some());
    match app.session.queue.first() {
        Some(id) => {
            let id = *id;
            app.activate(id);
            app.session.prompt_open = true;
        }
        None => {
            app.session.prompt_open = false;
            if let Some(action) = app.session.pending.take() {
                perform(app, action);
            }
        }
    }
}

fn perform(app: &mut XenifyApp, action: Pending) {
    match action {
        Pending::New => app.new_project(),
        Pending::Open => app.dialog = Some(Dialog::open(app)),
        Pending::OpenPath(p) => app.open(p),
        Pending::OpenDemo => {
            app.open_document(xenify_doc::demo_project(), None);
        }
        Pending::Quit => {
            app.session.allow_close = true;
            clear_all_autosaves();
            app.session.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        Pending::Close(ids) => app.close_documents(&ids),
    }
}

/// After a successful save: continue whatever was waiting on it.
pub fn after_save(app: &mut XenifyApp) {
    let id = app.docs.active();
    clear_autosave(id);
    app.session.autosaved.remove(&id);
    if !app.session.prompt_open && app.session.pending.is_some() {
        app.session.queue.retain(|d| *d != id);
        next_question(app);
    }
}

/// Call every frame: close requests, the prompts, and autosave.
pub fn update(app: &mut XenifyApp, ctx: &egui::Context) {
    // The window's close button.
    if ctx.input(|i| i.viewport().close_requested()) && !app.session.allow_close {
        if !app.dirty_documents(&app.docs.ids()).is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            guard(app, Pending::Quit);
        } else {
            clear_all_autosaves();
        }
    }
    unsaved_prompt(app, ctx);
    recovery_prompt(app, ctx);
    autosave(app, ctx);
}

fn unsaved_prompt(app: &mut XenifyApp, ctx: &egui::Context) {
    if !app.session.prompt_open {
        return;
    }
    let name = app.file.as_ref().and_then(|p| p.file_stem()).map_or("Untitled".into(), |s| s.to_string_lossy());
    let more = app.session.queue.len().saturating_sub(1);
    let mut choice = None;
    let title = format!("Save changes to “{name}”?");
    let (_, closed) = crate::widgets::modal(ctx, "unsaved", &title, 420.0, |ui| {
        ui.label("Your changes will be lost if you don't save them.");
        if more > 0 {
            ui.weak(format!("{more} more unsaved file{} after this one.", if more == 1 { "" } else { "s" }));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::primary("Save")).clicked() {
                choice = Some(0);
            }
            if ui.button("Don't Save").clicked() {
                choice = Some(1);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(2);
            }
        });
    });
    // The title bar's ×, Esc or a click outside: cancel.
    if closed && choice.is_none() {
        choice = Some(2);
    }
    match choice {
        Some(0) => {
            app.session.prompt_open = false;
            // Save (or Save As…); the rest continues once it has saved.
            app.save();
        }
        Some(1) => {
            app.session.queue.remove(0);
            next_question(app);
        }
        Some(_) => {
            app.session.prompt_open = false;
            app.session.pending = None;
            app.session.queue.clear();
        }
        None => {}
    }
}

fn recovery_prompt(app: &mut XenifyApp, ctx: &egui::Context) {
    let Some(items) = &app.session.recovery else { return };
    let newest = items.iter().map(|r| r.when).max().unwrap_or(SystemTime::UNIX_EPOCH);
    let ago = newest.elapsed().map_or("recently".into(), |d| {
        let m = d.as_secs() / 60;
        match m {
            0 => "less than a minute ago".to_string(),
            1..=59 => format!("{m} min ago"),
            _ => format!("{} h ago", m / 60),
        }
    });
    let names: Vec<String> = items
        .iter()
        .map(|r| {
            r.original
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or("an untitled project".into(), |s| format!("“{}”", s.to_string_lossy()))
        })
        .collect();
    let mut choice = None;
    let (_, closed) = crate::widgets::modal(ctx, "recover", "Recover unsaved work?", 460.0, |ui| {
        ui.label(format!(
            "Xenify Vector didn't close properly last time. There are unsaved changes to {}, autosaved {ago}.",
            names.join(", ")
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::primary("Recover")).clicked() {
                choice = Some(true);
            }
            if ui.button("Discard").clicked() {
                choice = Some(false);
            }
        });
        ui.weak("Closing this keeps the autosave: you'll be asked again next time.");
    });
    // Closed without choosing: decide later (nothing is deleted).
    if closed && choice.is_none() {
        app.session.recovery = None;
        app.status = "Kept the autosaved work: you'll be asked again next time Xenify Vector starts.".into();
        return;
    }
    let Some(recover) = choice else { return };
    let items = app.session.recovery.take().unwrap_or_default();
    let mut recovered = 0;
    for r in items {
        if recover {
            match std::fs::read_to_string(&r.path)
                .map_err(|e| e.to_string())
                .and_then(|s| Project::from_json(&s).map_err(|e| e.to_string()))
            {
                Ok(p) => {
                    app.open_document(p, r.original.clone());
                    // Not saved to its file yet.
                    app.editor.mark_unsaved();
                    recovered += 1;
                }
                Err(e) => app.fail(format!("Couldn't recover: {e}")),
            }
        }
        let _ = std::fs::remove_file(&r.path);
        let _ = std::fs::remove_file(r.path.with_extension("json"));
    }
    if recovered > 0 {
        app.status = "Recovered your unsaved work — save it to keep it.".into();
    }
}

fn autosave(app: &mut XenifyApp, ctx: &egui::Context) {
    if app.session.recovery.is_some() {
        return;
    }
    let pending: Vec<(DocId, u64)> = app
        .each_document()
        .into_iter()
        .filter(|(id, e, _)| e.is_dirty() && app.session.autosaved.get(id) != Some(&e.revision()))
        .map(|(id, e, _)| (id, e.revision()))
        .collect();
    if pending.is_empty() {
        return;
    }
    let every = Duration::from_secs(app.prefs.autosave_secs.max(10));
    let due = app.session.last_autosave.is_none_or(|t| t.elapsed() >= every);
    if !due {
        // Come back when it's due, even if nothing else repaints.
        ctx.request_repaint_after(every);
        return;
    }
    app.session.last_autosave = Some(Instant::now());
    for (id, rev) in pending {
        let Some(path) = autosave_path(std::process::id(), id) else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let Some((_, editor, file)) = app.each_document().into_iter().find(|(d, _, _)| *d == id) else { continue };
        let meta = serde_json::json!({ "file": file });
        if std::fs::write(&path, editor.project().to_json()).is_ok() {
            let _ = std::fs::write(path.with_extension("json"), meta.to_string());
            app.session.autosaved.insert(id, rev);
        }
    }
}

fn autosave_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").filter(|_| cfg!(windows)).map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("xenify").join("autosave"))
}

fn autosave_path(pid: u32, doc: DocId) -> Option<PathBuf> {
    Some(autosave_dir()?.join(format!("session-{pid}-{}.xenify", doc.0)))
}

/// Removes a document's autosave (after saving or closing it).
pub fn clear_autosave(doc: DocId) {
    if let Some(p) = autosave_path(std::process::id(), doc) {
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension("json"));
    }
}

/// Removes every autosave of this session (clean exit).
fn clear_all_autosaves() {
    let Some(dir) = autosave_dir() else { return };
    let prefix = format!("session-{}-", std::process::id());
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The Xenify Vector process that wrote autosave `stem` ("session-PID" or "session-PID-DOC").
fn autosave_pid(stem: &str) -> Option<u32> {
    stem.strip_prefix("session-")?.split('-').next()?.parse().ok()
}

/// Autosaves whose Xenify Vector process is gone (it didn't exit cleanly), newest first.
fn find_recovery() -> Vec<Recovery> {
    let Some(dir) = autosave_dir() else { return Vec::new() };
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(pid) = path.file_stem().and_then(|s| s.to_str()).and_then(autosave_pid) else {
            continue;
        };
        if path.extension().is_none_or(|e| e != "xenify") || process_alive(pid) {
            continue;
        }
        let when = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        let original = std::fs::read_to_string(path.with_extension("json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["file"].as_str().map(PathBuf::from));
        found.push(Recovery { path, original, when });
    }
    found.sort_by_key(|r| std::cmp::Reverse(r.when));
    found
}

fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    // Linux/BSD with procfs; elsewhere assume it's gone (worst case: an extra prompt).
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    #[test]
    fn autosave_names() {
        assert_eq!(super::autosave_pid("session-1234"), Some(1234), "files from before tabs");
        assert_eq!(super::autosave_pid("session-1234-7"), Some(1234));
        assert_eq!(super::autosave_pid("other-1"), None);
    }
}
