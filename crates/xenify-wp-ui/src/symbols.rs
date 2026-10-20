//! Insert › Symbols & emoji (Ctrl+.): a side panel tab that stays open
//! while you type. Search by name, keyword or emoticon (`:)`); matches are
//! listed with what they are and how to type them. Click to insert at the
//! caret. Emoji are drawn from the bundled Fluent set, the same pictures the
//! page uses.

use crate::app::WriteApp;
use egui::{ColorImage, TextureHandle, TextureOptions};
use xenify_wp_layout::emoji;
use std::collections::HashMap;

use xenify_wp_doc::symbols::SYMBOLS;

fn symbols() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    SYMBOLS
        .lines()
        .map(|line| {
            let mut parts = line.split('|');
            let group = parts.next().unwrap_or_default();
            let items = parts.filter_map(|p| p.split_once(' ')).collect();
            (group, items)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Symbols,
    Emoji,
}

#[derive(Default)]
pub struct SymbolPicker {
    pub open: bool,
    pub tab: Tab,
    pub query: String,
    /// Put the keyboard in the search box next frame.
    pub focus: bool,
    /// Emoji pictures as textures, by glyph.
    textures: HashMap<String, TextureHandle>,
}

impl SymbolPicker {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
    }

    fn texture(&mut self, ctx: &egui::Context, e: &emoji::Emoji, px: u32) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(&e.glyph) {
            return Some(t.clone());
        }
        let rgba = emoji::picture(e, px)?;
        let img = ColorImage::from_rgba_premultiplied([px as usize, px as usize], &rgba);
        let t = ctx.load_texture(format!("emoji-{}", e.glyph), img, TextureOptions::LINEAR);
        self.textures.insert(e.glyph.clone(), t.clone());
        Some(t)
    }
}

/// Inserts `text` at the caret and remembers it in the recent list.
fn insert(app: &mut WriteApp, text: &str) {
    let fmt = app.view.pending_fmt.clone();
    let _ = xenify_wp_doc::ops::insert_text(&mut app.editor, text, fmt.as_ref());
    app.prefs.recent_symbols.retain(|s| s != text);
    app.prefs.recent_symbols.insert(0, text.to_string());
    app.prefs.recent_symbols.truncate(24);
}

/// The Symbols & emoji tab of the side panel.
pub fn panel(app: &mut WriteApp, ui: &mut egui::Ui) {
    let mut chosen: Option<String> = None;
    {
        {
            let px = (28.0 * ui.ctx().pixels_per_point()).round() as u32;
            let p = &mut app.symbols;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut p.tab, Tab::Symbols, "Symbols");
                ui.selectable_value(&mut p.tab, Tab::Emoji, "Emoji");
            });
            let search = ui.add(
                egui::TextEdit::singleline(&mut p.query)
                    .hint_text("Search: name, keyword or :)")
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut p.focus) {
                search.request_focus();
            }
            if search.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                p.query.clear();
            }
            let recent = app.prefs.recent_symbols.clone();
            if !recent.is_empty() && p.query.is_empty() {
                ui.weak("Recent");
                ui.horizontal_wrapped(|ui| {
                    for s in &recent {
                        if cell(ui, p, s, px).clicked() {
                            chosen = Some(s.clone());
                        }
                    }
                });
                ui.separator();
            }
            let q = p.query.trim().to_lowercase();
            if !q.is_empty() {
                // Searching: a list, each with its name and how to type it.
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 28.0).show(
                    ui,
                    |ui| {
                        let found = matches(&q);
                        if found.is_empty() {
                            ui.weak("Nothing matches.");
                        }
                        for (glyph, name) in found.into_iter().take(200) {
                            ui.horizontal(|ui| {
                                let r = cell(ui, p, &glyph, px);
                                let typed = xenify_wp_doc::autocorrect::emoticons_for(&glyph);
                                let r2 = ui
                                    .vertical(|ui| {
                                        ui.label(capitalised(&name));
                                        if !typed.is_empty() {
                                            ui.weak(format!("Type {} then a space", typed.join(" or ")));
                                        }
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if r.clicked() || r2.clicked() {
                                    chosen = Some(glyph.clone());
                                }
                            });
                        }
                    },
                );
                ui.separator();
                ui.weak(emoji::CREDIT);
                if let Some(s) = chosen {
                    insert(app, &s);
                    app.refocus = true;
                }
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 28.0).show(
                ui,
                |ui| match p.tab {
                    Tab::Symbols => {
                        for (group, items) in symbols() {
                            let shown: Vec<_> = items
                                .into_iter()
                                .filter(|(c, name)| {
                                    q.is_empty() || name.contains(&q) || *c == q || group.to_lowercase().contains(&q)
                                })
                                .collect();
                            if shown.is_empty() {
                                continue;
                            }
                            ui.label(egui::RichText::new(group).strong());
                            ui.horizontal_wrapped(|ui| {
                                for (c, name) in shown {
                                    if cell(ui, p, c, px).on_hover_text(capitalised(name)).clicked() {
                                        chosen = Some(c.to_string());
                                    }
                                }
                            });
                        }
                    }
                    Tab::Emoji => {
                        let found = emoji::search(&q);
                        if found.is_empty() {
                            ui.weak("No emoji matches.");
                        }
                        let mut group = "";
                        let mut row: Vec<&emoji::Emoji> = Vec::new();
                        let flush = |ui: &mut egui::Ui,
                                     p: &mut SymbolPicker,
                                     row: &mut Vec<&emoji::Emoji>,
                                     chosen: &mut Option<String>| {
                            ui.horizontal_wrapped(|ui| {
                                for e in row.drain(..) {
                                    if cell(ui, p, &e.glyph, px).on_hover_text(hover(&e.glyph, &e.name)).clicked() {
                                        *chosen = Some(e.glyph.clone());
                                    }
                                }
                            });
                        };
                        for e in found {
                            if e.group != group {
                                flush(ui, p, &mut row, &mut chosen);
                                group = &e.group;
                                ui.label(egui::RichText::new(group).strong());
                            }
                            row.push(e);
                        }
                        flush(ui, p, &mut row, &mut chosen);
                    }
                },
            );
            ui.separator();
            ui.weak(emoji::CREDIT);
        }
    }
    if let Some(s) = chosen {
        insert(app, &s);
        // Typing goes on in the document.
        app.refocus = true;
    }
}

/// Symbols and emoji matching `q` (lower case): by name, keyword, group,
/// the character itself or an emoticon that types it. Symbols first.
fn matches(q: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let typed: Vec<&str> = xenify_wp_doc::autocorrect::EMOTICONS
        .iter()
        .filter(|(t, _)| t.eq_ignore_ascii_case(q))
        .map(|(_, e)| *e)
        .collect();
    for (group, items) in symbols() {
        for (c, name) in items {
            if name.contains(q) || c == q || group.to_lowercase().contains(q) {
                out.push((c.to_string(), name.to_string()));
            }
        }
    }
    let key = |s: &str| s.chars().filter(|&c| c != '\u{fe0f}').collect::<String>();
    for e in emoji::library() {
        if typed.iter().any(|t| key(t) == key(&e.glyph)) {
            out.insert(0, (e.glyph.clone(), e.name.clone()));
        }
    }
    for e in emoji::search(q) {
        if !out.iter().any(|(g, _)| *g == e.glyph) {
            out.push((e.glyph.clone(), e.name.clone()));
        }
    }
    out
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// An emoji's name, and the emoticon that types it.
fn hover(glyph: &str, name: &str) -> String {
    let typed = xenify_wp_doc::autocorrect::emoticons_for(glyph);
    if typed.is_empty() { capitalised(name) } else { format!("{} — type {}", capitalised(name), typed.join(" or ")) }
}

/// One clickable square: an emoji picture, or the symbol as text.
fn cell(ui: &mut egui::Ui, p: &mut SymbolPicker, s: &str, px: u32) -> egui::Response {
    let size = egui::vec2(34.0, 34.0);
    if let Some(e) = emoji::find(s)
        && let Some(tex) = p.texture(ui.ctx(), e, px)
    {
        let r = ui.add_sized(
            size,
            egui::Button::image(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(24.0, 24.0))).frame(false),
        );
        // Screen readers (and tests) know it by its name.
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &e.name));
        return r;
    }
    ui.add_sized(size, egui::Button::new(egui::RichText::new(s).size(20.0)).frame(false))
}

impl WriteApp {
    /// Symbols and emoji inserted lately, newest first.
    pub fn recent_symbols(&self) -> &[String] {
        &self.prefs.recent_symbols
    }
}

#[cfg(test)]
mod tests {
    use skrifa::MetadataProvider;

    #[test]
    fn search_finds_by_name_and_by_emoticon() {
        let found = super::matches(";)");
        assert_eq!(found.first().map(|(g, _)| g.as_str()), Some("😉"), "{found:?}");
        assert!(super::matches("euro").iter().any(|(g, _)| g == "€"));
        assert!(super::matches("smiling").iter().any(|(g, _)| g == "🙂"));
    }

    #[test]
    fn every_symbol_has_a_glyph_in_the_bundled_fonts() {
        let fonts: Vec<skrifa::FontRef> = [xenify_wp_layout::INTER_FONT, xenify_wp_layout::SYMBOL_FONT]
            .iter()
            .map(|d| skrifa::FontRef::new(d).unwrap())
            .collect();
        let missing: Vec<&str> = super::symbols()
            .into_iter()
            .flat_map(|(_, items)| items)
            .map(|(c, _)| c)
            .filter(|c| !c.chars().all(|ch| fonts.iter().any(|f| f.charmap().map(ch).is_some())))
            .collect();
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }
}
