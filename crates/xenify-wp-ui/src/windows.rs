//! More windows (File › New window, a tab's "Move to new window"): each has
//! its own tabs, split and dialogs; all documents, the checker, fonts and
//! the dictionary are shared by the one program. Before a window is drawn
//! its state and its front document are swapped into [`WriteApp`]'s own
//! fields (where the main window's are otherwise), then swapped back.

use crate::app::{Dialog, WriteApp};
use crate::find::FindState;
use crate::print_preview::PrintPreview;
use crate::split::Split;
use xenify_wp_doc::Document;
use xenify_wp_io::Format;

/// A window other than the main one.
pub struct Window {
    viewport: egui::ViewportId,
    /// Its tabs, left to right.
    order: Vec<u64>,
    /// Its front document while it isn't drawn; the main window's while it is.
    active: u64,
    split: Option<Split>,
    find: FindState,
    print_preview: Option<PrintPreview>,
    control_menu: Option<(xenify_wp_doc::Pos, egui::Pos2)>,
    dialog: Option<Dialog>,
    title: String,
    refocus: bool,
}

impl Window {
    /// Its documents' tab keys.
    pub fn keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.order.iter().copied()
    }

    fn new(n: u64, key: u64) -> Self {
        Self {
            viewport: egui::ViewportId::from_hash_of(("write-window", n)),
            order: vec![key],
            active: key,
            split: None,
            find: FindState::default(),
            print_preview: None,
            control_menu: None,
            dialog: None,
            title: String::new(),
            refocus: true,
        }
    }
}

impl WriteApp {
    /// Swaps window `i`'s state with the main window's (both ways: the same
    /// call puts things back).
    fn swap_window(&mut self, i: usize) {
        let w = &mut self.windows[i];
        std::mem::swap(&mut self.tabs.order, &mut w.order);
        std::mem::swap(&mut self.split, &mut w.split);
        std::mem::swap(&mut self.find, &mut w.find);
        std::mem::swap(&mut self.print_preview, &mut w.print_preview);
        std::mem::swap(&mut self.control_menu, &mut w.control_menu);
        std::mem::swap(&mut self.dialog, &mut w.dialog);
        std::mem::swap(&mut self.title, &mut w.title);
        std::mem::swap(&mut self.refocus, &mut w.refocus);
        // The front document: the other window's in, this one's noted there.
        let front = self.tabs.active;
        let want = w.active;
        self.swap_front(want);
        self.windows[i].active = front;
    }

    /// Moves tab `key` from this window into a new window of its own.
    pub fn move_tab_to_new_window(&mut self, key: u64) {
        if self.tabs.order.len() <= 1 {
            // This window keeps a blank document.
            self.open_in_new_tab(Document::new(), None, Format::Markdown);
        }
        if self.tabs.active == key {
            self.cycle_tab(true);
        }
        self.tabs.order.retain(|k| *k != key);
        self.window_count += 1;
        self.windows.push(Window::new(self.window_count, key));
    }

    /// File › New window: a blank document in a window of its own.
    pub fn new_window(&mut self) {
        self.open_in_new_tab(Document::new(), None, Format::Markdown);
        let key = self.tabs.active;
        self.move_tab_to_new_window(key);
    }

    /// Closes window `i`: documents with unsaved changes move to the main
    /// window (nothing is lost); the others close.
    fn close_window(&mut self, i: usize) {
        let w = self.windows.remove(i);
        let mut kept = 0;
        for k in w.order {
            let dirty = self.tabs.others.iter().find(|t| t.key == k).is_some_and(|t| t.editor.is_dirty());
            if dirty {
                self.tabs.order.push(k);
                kept += 1;
            } else if k != self.tabs.active {
                self.tabs.others.retain(|t| t.key != k);
            }
        }
        if kept > 0 {
            self.message = Some((
                format!(
                    "{kept} document{} with unsaved changes moved to this window",
                    if kept == 1 { "" } else { "s" }
                ),
                false,
            ));
        }
    }

    /// Quitting: every window's documents come to the main one, so each
    /// unsaved one can be asked about.
    pub(crate) fn gather_windows(&mut self) {
        while let Some(w) = self.windows.pop() {
            self.tabs.order.extend(w.order);
        }
    }
}

/// Draws the other windows (each as its own OS window).
pub fn show_others(app: &mut WriteApp, ctx: &egui::Context, frame: &eframe::Frame) {
    let mut i = 0;
    while i < app.windows.len() {
        let id = app.windows[i].viewport;
        app.swap_window(i);
        let title = app.title.clone();
        let builder = egui::ViewportBuilder::default()
            .with_title(if title.is_empty() { "Xenify Write".to_string() } else { title })
            .with_app_id("xenify-write")
            .with_inner_size([1100.0, 850.0])
            .with_min_inner_size([640.0, 480.0]);
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |ui, _| {
            if ui.input(|inp| inp.viewport().close_requested()) {
                close = true;
            }
            app.window_ui(ui, frame, false);
        });
        app.swap_window(i);
        if close {
            app.close_window(i);
        } else {
            i += 1;
        }
    }
}
