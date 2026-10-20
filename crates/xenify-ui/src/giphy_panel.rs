//! The GIFs panel: search GIPHY (or browse what's trending) with the user's
//! own API key, and add a GIF as an animated image layer. Shows "Powered by
//! GIPHY" (their terms) and who made each GIF; the user is responsible for
//! the rights to what they place.

use crate::app::XenifyApp;
use egui::{Color32, RichText};
use xenify_export::giphy::{self, Gif};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};

const TILE: f32 = 96.0;
const PAGE: u32 = 24;

type SearchFn = fn(&str, &str, u32, u32) -> Result<Vec<Gif>, String>;
type Download = Receiver<Result<Vec<u8>, String>>;

/// Where results and files come from (the network; tests swap in fakes).
#[derive(Clone, Copy)]
pub struct Backend {
    pub search: SearchFn,
    pub get: fn(&str) -> Result<Vec<u8>, String>,
}

impl Default for Backend {
    fn default() -> Self {
        Self { search: giphy::search, get: |url| xenify_platform::download::get(url, 16 * 1024 * 1024) }
    }
}

type Results = Result<(Vec<Gif>, bool), String>;

pub struct GiphyPanel {
    pub(crate) backend: Backend,
    /// The API key once known (`None`: not looked up yet; `Some("")`: none saved).
    pub(crate) key: Option<String>,
    key_input: String,
    changing_key: bool,
    query: String,
    shown_query: Option<String>,
    results: Vec<Gif>,
    error: Option<String>,
    searching: Option<Receiver<Results>>,
    previews: HashMap<String, egui::TextureHandle>,
    asked: HashSet<String>,
    preview_tx: Sender<(String, Result<Vec<u8>, String>)>,
    preview_rx: Receiver<(String, Result<Vec<u8>, String>)>,
    adding: Option<(Gif, Download)>,
}

impl Default for GiphyPanel {
    fn default() -> Self {
        let (preview_tx, preview_rx) = channel();
        Self {
            backend: Backend::default(),
            key: None,
            key_input: String::new(),
            changing_key: false,
            query: String::new(),
            shown_query: None,
            results: Vec::new(),
            error: None,
            searching: None,
            previews: HashMap::new(),
            asked: HashSet::new(),
            preview_tx,
            preview_rx,
            adding: None,
        }
    }
}

impl GiphyPanel {
    fn key(&mut self) -> String {
        self.key.get_or_insert_with(|| giphy::saved_key().unwrap_or_default()).clone()
    }

    /// Searches for `query` (trending when empty); `more` adds the next page.
    fn search(&mut self, query: &str, more: bool) {
        let key = self.key();
        let (tx, rx) = channel();
        let (q, offset, search) =
            (query.to_string(), if more { self.results.len() as u32 } else { 0 }, self.backend.search);
        std::thread::spawn(move || {
            let _ = tx.send(search(&key, &q, PAGE, offset).map(|r| (r, more)));
        });
        self.searching = Some(rx);
        self.shown_query = Some(query.to_string());
        self.error = None;
    }

    /// Picks up results, previews and the GIF being added.
    fn poll(&mut self, ctx: &egui::Context) -> Option<(Gif, Result<Vec<u8>, String>)> {
        let mut busy = false;
        if let Some(rx) = &self.searching {
            match rx.try_recv() {
                Ok(Ok((list, more))) => {
                    if !more {
                        self.results.clear();
                    }
                    let known: HashSet<String> = self.results.iter().map(|g| g.id.clone()).collect();
                    self.results.extend(list.into_iter().filter(|g| !known.contains(&g.id)));
                    self.searching = None;
                }
                Ok(Err(e)) => {
                    self.error = Some(e);
                    self.searching = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
                Err(_) => self.searching = None,
            }
        }
        while let Ok((id, bytes)) = self.preview_rx.try_recv() {
            // The first frame is enough for choosing.
            if let Ok(img) = bytes.and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string())) {
                let rgba = img.into_rgba8();
                let (w, h) = rgba.dimensions();
                let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                let tex = ctx.load_texture(format!("giphy-{id}"), image, egui::TextureOptions::LINEAR);
                self.previews.insert(id, tex);
            }
        }
        busy |= self.asked.len() > self.previews.len();
        let mut done = None;
        if let Some((_, rx)) = &self.adding {
            match rx.try_recv() {
                Ok(r) => done = self.adding.take().map(|(g, _)| (g, r)),
                Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
                Err(_) => done = self.adding.take().map(|(g, _)| (g, Err("the download stopped".into()))),
            }
        }
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        done
    }

    fn want_preview(&mut self, g: &Gif) {
        if self.asked.insert(g.id.clone()) {
            let (tx, get, id, url) = (self.preview_tx.clone(), self.backend.get, g.id.clone(), g.preview.clone());
            std::thread::spawn(move || {
                let _ = tx.send((id, get(&url)));
            });
        }
    }

    fn add(&mut self, g: &Gif) {
        let (tx, rx) = channel();
        let (get, url) = (self.backend.get, g.full.clone());
        std::thread::spawn(move || {
            let _ = tx.send(get(&url));
        });
        self.adding = Some((g.clone(), rx));
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let mut panel = std::mem::take(&mut app.giphy_panel);
    let arrived = panel.poll(ui.ctx());
    if panel.key().is_empty() || panel.changing_key {
        key_setup(app, ui, &mut panel);
    } else {
        browse(app, ui, &mut panel);
    }
    app.giphy_panel = panel;
    if let Some((g, bytes)) = arrived {
        match bytes {
            Ok(b) => {
                if app.add_image(&b, &g.credit(), None).is_some() {
                    app.status = format!("Added {}. {} — make sure you may use it.", g.credit(), giphy::ATTRIBUTION);
                }
            }
            Err(e) => app.fail(format!("Couldn't get the GIF: {e}")),
        }
    }
}

fn key_setup(app: &mut XenifyApp, ui: &mut egui::Ui, panel: &mut GiphyPanel) {
    ui.label(RichText::new("Search GIPHY for GIFs").strong());
    ui.label("GIPHY needs an API key of your own (free for personal use). Create an app at:");
    ui.hyperlink(giphy::KEY_URL);
    ui.horizontal(|ui| {
        ui.label("API Key");
        ui.add(egui::TextEdit::singleline(&mut panel.key_input).password(true).desired_width(200.0));
    });
    ui.horizontal(|ui| {
        let key = panel.key_input.trim().to_string();
        if ui.add_enabled(!key.is_empty(), egui::Button::new("Save Key")).clicked() {
            match xenify_platform::secrets::save(giphy::KEY_NAME, &key) {
                Ok(where_) => app.status = format!("Saved your GIPHY key ({}).", where_.describe()),
                Err(e) => app.status = format!("Using the key for this session only (couldn't save it: {e})."),
            }
            panel.key = Some(key);
            panel.key_input.clear();
            panel.changing_key = false;
            panel.search("", false);
        }
        if panel.changing_key && ui.button("Cancel").clicked() {
            panel.changing_key = false;
        }
    });
    ui.add_space(6.0);
    ui.weak(giphy::RIGHTS);
}

fn browse(app: &mut XenifyApp, ui: &mut egui::Ui, panel: &mut GiphyPanel) {
    if panel.shown_query.is_none() && panel.searching.is_none() {
        panel.search("", false);
    }
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut panel.query).hint_text("Search GIPHY…").desired_width(180.0));
        if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button("Search").clicked() {
            let q = panel.query.clone();
            panel.search(&q, false);
        }
        if ui.button("Trending").clicked() {
            panel.query.clear();
            panel.search("", false);
        }
        ui.menu_button("⚙", |ui| {
            if ui.button("Change GIPHY Key…").clicked() {
                panel.changing_key = true;
                ui.close();
            }
            if ui.button("Forget GIPHY Key").clicked() {
                let _ = xenify_platform::secrets::delete(giphy::KEY_NAME);
                panel.key = Some(String::new());
                panel.results.clear();
                panel.shown_query = None;
                ui.close();
            }
        });
    });
    let heading = match panel.shown_query.as_deref() {
        Some("") | None => "Trending".to_string(),
        Some(q) => format!("Results for “{q}”"),
    };
    ui.horizontal(|ui| {
        ui.weak(heading);
        if panel.searching.is_some() || panel.adding.is_some() {
            ui.spinner();
        }
        if let Some((g, _)) = &panel.adding {
            ui.weak(format!("Getting {}…", g.title));
        }
    });
    if let Some(e) = &panel.error {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    let results = panel.results.clone();
    let mut clicked = None;
    let mut more = false;
    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 40.0).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for g in &results {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(TILE, TILE * 0.75), egui::Sense::click());
                let label = g.credit();
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                if ui.is_rect_visible(rect) {
                    panel.want_preview(g);
                    match panel.previews.get(&g.id) {
                        Some(t) => {
                            // Fit, keeping its shape.
                            let [w, h] = t.size().map(|v| v.max(1) as f32);
                            let k = (rect.width() / w).min(rect.height() / h);
                            let r = egui::Rect::from_center_size(rect.center(), egui::vec2(w * k, h * k));
                            ui.painter().image(
                                t.id(),
                                r,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            ui.painter().rect_filled(rect.shrink(4.0), 6.0, Color32::from_gray(45));
                        }
                    }
                    if resp.hovered() {
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(2.0, crate::widgets::ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                if resp.on_hover_text(format!("{label}\nClick to add it as an animated image")).clicked() {
                    clicked = Some(g.clone());
                }
            }
        });
        if !results.is_empty() && panel.searching.is_none() && ui.button("More").clicked() {
            more = true;
        }
    });
    if more {
        let q = panel.shown_query.clone().unwrap_or_default();
        panel.search(&q, true);
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(giphy::ATTRIBUTION).strong());
        ui.weak(giphy::RIGHTS);
    });
    if let Some(g) = clicked {
        if panel.adding.is_some() {
            app.status = "Still getting the last GIF…".into();
        } else {
            panel.add(&g);
        }
    }
}
