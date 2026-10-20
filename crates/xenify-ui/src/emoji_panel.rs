//! The Emoji panel: the bundled Fluent Emoji (MIT) to search, browse by
//! group, and add as editable vector groups (click, or drag onto the
//! canvas), or to throw as confetti (the emoji becomes the particles'
//! sprite). The Animated tab offers Noto Emoji Animation (CC BY 4.0),
//! downloaded when first used and added as a looping symbol whose layers
//! and keyframes can be edited.

use crate::app::XenifyApp;
use egui::Color32;
use xenify_doc::{Id, LayerKind, Vec2};
use xenify_export::animated_emoji::{self, AnimatedEmoji};
use xenify_export::emoji::{self, Emoji};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

/// Tile size (points) and new previews drawn per UI frame.
const TILE: f32 = 40.0;
const PER_FRAME: usize = 10;

/// Dragged from the panel onto the canvas.
#[derive(Clone, Copy)]
pub struct EmojiPayload(pub &'static str);

type Download<T> = Receiver<Result<T, String>>;

#[derive(Default)]
pub struct EmojiPanel {
    query: String,
    group: Option<&'static str>,
    gpu: Option<Result<xenify_render::Offscreen, String>>,
    textures: HashMap<&'static str, egui::TextureHandle>,
    /// The Animated tab is showing.
    animated: bool,
    /// Animated emoji play over and over (else once).
    play_once: bool,
    /// The animated emoji list, once downloaded (or why it couldn't be).
    index: Option<Result<Vec<AnimatedEmoji>, String>>,
    index_rx: Option<Download<Vec<AnimatedEmoji>>>,
    /// An animated emoji being downloaded, to add when it arrives.
    pending: Option<(AnimatedEmoji, Download<Vec<u8>>)>,
}

impl EmojiPanel {
    fn preview(&mut self, ctx: &egui::Context, e: &'static Emoji, budget: &mut usize) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(e.glyph.as_str()) {
            return Some(t.clone());
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let px = (TILE * 2.0) as u32;
        let mut comp = xenify_doc::Project::new().compositions.remove(0);
        comp.settings.width = px;
        comp.settings.height = px;
        comp.settings.background = xenify_doc::Color::TRANSPARENT;
        comp.layers = emoji::layers(e, 1, Vec2::new(px as f64 / 2.0, px as f64 / 2.0), px as f64 * 0.9).ok()?;
        let scene = xenify_anim::evaluate(&comp, 0.0);
        let gpu = self.gpu.get_or_insert_with(|| xenify_render::Offscreen::new().map_err(|e| e.to_string()));
        let img = gpu.as_mut().ok()?.render(&scene, px, px).ok()?;
        let image = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &img.pixels);
        let tex = ctx.load_texture(format!("emoji-{}", e.glyph), image, egui::TextureOptions::LINEAR);
        self.textures.insert(&e.glyph, tex.clone());
        Some(tex)
    }
}

/// The panel.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let mut panel = std::mem::take(&mut app.emoji_panel);
    let arrived = panel.poll(ui.ctx());
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut panel.query)
                .hint_text("Search: party, heart, rocket…")
                .desired_width(f32::INFINITY),
        );
    });
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(!panel.animated && panel.group.is_none() && panel.query.is_empty(), "Recent").clicked() {
            panel.group = None;
            panel.animated = false;
            panel.query.clear();
        }
        if ui
            .selectable_label(panel.animated, "Animated")
            .on_hover_text("Emoji that move (Noto Emoji Animation), downloaded when first used")
            .clicked()
        {
            panel.animated = true;
            panel.group = None;
        }
        for g in emoji::groups() {
            if ui.selectable_label(!panel.animated && panel.group == Some(g), g).clicked() {
                panel.group = Some(g);
                panel.animated = false;
                panel.query.clear();
            }
        }
    });
    if panel.animated {
        animated_tab(app, ui, &mut panel);
        app.emoji_panel = panel;
        if let Some((e, bytes)) = arrived {
            finish_animated(app, &e, bytes);
        }
        return;
    }
    let list: Vec<&'static Emoji> = if !panel.query.trim().is_empty() {
        emoji::search(&panel.query)
    } else if let Some(g) = panel.group {
        emoji::library().iter().filter(|e| e.group == g).collect()
    } else {
        let recent: Vec<&Emoji> = app.prefs.recent_emoji.iter().filter_map(|k| emoji::find(k)).collect();
        if recent.is_empty() {
            ["🎉", "🚀", "✨", "❤️", "🔥", "👍", "😀", "⭐"].iter().filter_map(|k| emoji::find(k)).collect()
        } else {
            recent
        }
    };
    let mut budget = PER_FRAME;
    let mut clicked = None;
    let mut burst = None;
    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 24.0).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for e in &list {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(TILE, TILE), egui::Sense::click_and_drag());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &e.name));
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                if ui.is_rect_visible(rect) {
                    match panel.preview(ui.ctx(), e, &mut budget) {
                        Some(t) => ui.painter().image(
                            t.id(),
                            rect.shrink(2.0),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        ),
                        None => ui.painter().rect_filled(rect.shrink(8.0), 8.0, Color32::from_gray(45)),
                    };
                }
                resp.dnd_set_drag_payload(EmojiPayload(&e.glyph));
                let resp = resp.on_hover_text(format!("{} (click to add, drag onto the canvas)", e.name));
                if resp.clicked() {
                    clicked = Some(*e);
                }
                resp.context_menu(|ui| {
                    if ui.button("Add").clicked() {
                        clicked = Some(*e);
                        ui.close();
                    }
                    if ui
                        .button("Confetti Burst of It")
                        .on_hover_text("A burst of particles, each one this emoji")
                        .clicked()
                    {
                        burst = Some(*e);
                        ui.close();
                    }
                });
            }
        });
        if budget == 0 {
            ui.ctx().request_repaint();
        }
    });
    ui.weak(emoji::CREDIT);
    app.emoji_panel = panel;
    if let Some((e, bytes)) = arrived {
        finish_animated(app, &e, bytes);
    }
    if let Some(e) = clicked {
        let s = &app.comp().settings;
        let at = Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0);
        insert(app, e, at);
    }
    if let Some(e) = burst {
        confetti(app, e);
    }
}

/// Adds `e` as a vector group centred on `at`, a quarter of the frame across.
pub fn insert(app: &mut XenifyApp, e: &Emoji, at: Vec2) -> Option<Id> {
    let s = &app.comp().settings;
    let size = (s.width.min(s.height) as f64 / 4.0).round();
    let layers = match emoji::layers(e, app.project().next_id, at, size) {
        Ok(l) => l,
        Err(err) => {
            app.fail(err);
            return None;
        }
    };
    remember(app, &e.glyph);
    app.add_layer_group(layers, &e.name);
    app.selected
}

fn remember(app: &mut XenifyApp, glyph: &str) {
    let r = &mut app.prefs.recent_emoji;
    r.retain(|g| g != glyph);
    r.insert(0, glyph.to_string());
    r.truncate(24);
}

/// A confetti burst whose particles are `e`: the emoji is added (hidden)
/// as the sprite.
pub fn confetti(app: &mut XenifyApp, e: &Emoji) {
    app.editor.begin_group(format!("{} confetti", e.name));
    if let Some(group) = insert(app, e, Vec2::new(-1000.0, -1000.0)) {
        let hide = app.project().layer(group).ok().map(|l| {
            let mut meta = l.meta.clone();
            meta.visible = false;
            meta.name = format!("{} (confetti sprite)", e.name);
            xenify_ops::Command::SetLayerMeta { layer: group, meta }
        });
        if let Some(cmd) = hide {
            app.exec("Hide sprite", cmd);
        }
        let id = Id(app.project().next_id);
        if let Ok(mut cmd) = xenify_ops::particles::add_preset(app.project(), app.comp, id, "Confetti Burst", None) {
            if let xenify_ops::Command::AddLayer { layer, .. } = &mut cmd
                && let LayerKind::Particles(em) = &mut layer.kind
            {
                em.particle = xenify_doc::ParticleShape::Layer { layer: group };
                em.size = 48.0.into();
                em.spin = 180.0.into();
                em.start = app.time;
                layer.meta.name = format!("{} confetti", e.name);
            }
            if app.exec("Emoji confetti", cmd) {
                app.select_only(Some(id));
            }
        }
    }
    app.editor.end_group();
}

impl EmojiPanel {
    /// Picks up finished downloads: the list, and an animated emoji to add.
    fn poll(&mut self, ctx: &egui::Context) -> Option<(AnimatedEmoji, Result<Vec<u8>, String>)> {
        if let Some(rx) = &self.index_rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.index = Some(r);
                    self.index_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                Err(_) => self.index_rx = None,
            }
        }
        let (_, rx) = self.pending.as_ref()?;
        match rx.try_recv() {
            Ok(r) => self.pending.take().map(|(e, _)| (e, r)),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                None
            }
            Err(_) => self.pending.take().map(|(e, _)| (e, Err("the download stopped".into()))),
        }
    }

    /// Starts downloading the list (or reads it from the cache).
    fn load_index(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(animated_emoji::index());
        });
        self.index_rx = Some(rx);
    }

    /// Downloads `e` (or reads it from the cache), to add when it arrives.
    pub fn fetch_animated(&mut self, e: &AnimatedEmoji) {
        let (tx, rx) = std::sync::mpsc::channel();
        let job = e.clone();
        std::thread::spawn(move || {
            let _ = tx.send(animated_emoji::lottie(&job));
        });
        self.pending = Some((e.clone(), rx));
    }
}

/// The Animated tab: the Noto animated emoji, searched by the same box.
fn animated_tab(app: &mut XenifyApp, ui: &mut egui::Ui, panel: &mut EmojiPanel) {
    if panel.index.is_none() && panel.index_rx.is_none() {
        panel.load_index();
    }
    ui.horizontal(|ui| {
        let mut looped = !panel.play_once;
        if ui
            .checkbox(&mut looped, "Loop Animated Emoji")
            .on_hover_text("Added emoji play over and over (else once, from the playhead)")
            .changed()
        {
            panel.play_once = !looped;
        }
        if let Some((e, _)) = &panel.pending {
            ui.spinner();
            ui.weak(format!("Getting {}…", e.name));
        }
    });
    let list = match &panel.index {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Getting the list of animated emoji…");
            });
            ui.weak(animated_emoji::CREDIT);
            return;
        }
        Some(Err(why)) => {
            ui.label(format!("Couldn't get the animated emoji: {why}"));
            if ui.button("Try Again").clicked() {
                panel.index = None;
            }
            ui.weak(animated_emoji::CREDIT);
            return;
        }
        Some(Ok(list)) => list.clone(),
    };
    let shown: Vec<&AnimatedEmoji> =
        if panel.query.trim().is_empty() { list.iter().collect() } else { animated_emoji::search(&list, &panel.query) };
    let mut budget = PER_FRAME;
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 36.0).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for e in &shown {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(TILE, TILE), egui::Sense::click());
                let label = format!("{} (animated)", e.name);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                if ui.is_rect_visible(rect) {
                    // The still Fluent picture when there is one; the glyph otherwise.
                    let still = emoji::find(&e.glyph).and_then(|f| panel.preview(ui.ctx(), f, &mut budget));
                    match still {
                        Some(t) => {
                            ui.painter().image(
                                t.id(),
                                rect.shrink(4.0),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                &e.glyph,
                                egui::FontId::proportional(TILE * 0.6),
                                ui.visuals().text_color(),
                            );
                        }
                    }
                    // A small "plays" mark.
                    let c = rect.right_bottom() + egui::vec2(-7.0, -7.0);
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![c + egui::vec2(-3.0, -4.0), c + egui::vec2(4.0, 0.0), c + egui::vec2(-3.0, 4.0)],
                        crate::widgets::ACCENT,
                        egui::Stroke::NONE,
                    ));
                }
                let cached = if animated_emoji::is_cached(e) { "" } else { " (downloads it)" };
                if resp.on_hover_text(format!("{}: click to add{cached}", e.name)).clicked() {
                    clicked = Some((*e).clone());
                }
            }
        });
        if budget == 0 {
            ui.ctx().request_repaint();
        }
    });
    ui.weak(animated_emoji::CREDIT);
    if let Some(e) = clicked {
        if panel.pending.is_some() {
            app.status = "Still getting the last animated emoji…".into();
        } else {
            panel.fetch_animated(&e);
            app.status = format!("Getting {}…", e.name);
        }
    }
}

/// Adds a downloaded animated emoji (or reports why it couldn't be).
fn finish_animated(app: &mut XenifyApp, e: &AnimatedEmoji, bytes: Result<Vec<u8>, String>) {
    let s = &app.comp().settings;
    let at = Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0);
    let looped = !app.emoji_panel.play_once;
    match bytes.and_then(|b| insert_animated(app, e, &b, at, None, looped)) {
        Ok(_) => {
            app.status = format!(
                "Added {} {} as a looping symbol (double-click to edit its layers). Credit: {}",
                e.glyph,
                e.name,
                animated_emoji::CREDIT
            )
        }
        Err(why) => app.fail(format!("Couldn't add {}: {why}", e.name)),
    }
}

/// Adds animated emoji `e` (its Lottie file `data`) as a symbol centred on
/// `at`, `size` pixels across (a quarter of the frame by default), starting
/// at the playhead. One undo step. Returns the layer.
pub fn insert_animated(
    app: &mut XenifyApp,
    e: &AnimatedEmoji,
    data: &[u8],
    at: Vec2,
    size: Option<f64>,
    looped: bool,
) -> Result<Id, String> {
    let s = &app.comp().settings;
    let size = size.unwrap_or((s.width.min(s.height) as f64 / 4.0).round());
    let import = xenify_export::lottie_import::import_lottie(data, app.project().next_id)?;
    let (comp, layer) = (Id(import.next_id), Id(import.next_id + 1));
    let (mut main, extras) = import.into_compositions(comp);
    main.settings.name = format!("{} {} (animated)", e.glyph, e.name);
    let cmd = xenify_ops::precomp::add_precomp(
        app.project(),
        app.comp,
        main,
        extras,
        layer,
        at,
        app.time,
        Some(size),
        looped,
    )
    .map_err(|e| e.to_string())?;
    if !app.exec(&format!("Add {}", e.name), cmd) {
        return Err("couldn't add it".into());
    }
    remember(app, &e.glyph);
    app.select_only(Some(layer));
    Ok(layer)
}
