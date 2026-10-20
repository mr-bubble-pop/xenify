//! Slide templates in the UI: Slide › New Slide From Layout, Apply Theme and
//! Save Slide as Layout, the Templates strip of the New dialog (rendered
//! thumbnails), and each layer's theme role in Properties.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::templates::{self, Content, Layout, Role, THEMES, Theme};
use xenify_doc::{Id, Layer, Project};
use xenify_ops::Command;
use std::collections::HashMap;
use std::path::PathBuf;

/// Rendered thumbnails and the Save as Layout name.
#[derive(Default)]
pub struct Templates {
    gpu: Option<Result<xenify_render::Offscreen, String>>,
    thumbs: HashMap<String, egui::TextureHandle>,
    layout_name: String,
    /// Where saved layouts go instead of the data folder (tests).
    pub(crate) dir: Option<PathBuf>,
}

const THUMB: (u32, u32) = (240, 135);

/// A thumbnail of `layout` in `theme` (rendered once, when first shown).
fn thumb(app: &mut XenifyApp, ctx: &egui::Context, layout: Layout, theme: &Theme) -> Option<egui::TextureHandle> {
    let key = format!("{}/{}", theme.name, layout.label());
    if let Some(t) = app.templates.thumbs.get(&key) {
        return Some(t.clone());
    }
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let built = templates::build(layout, theme, 1920.0, 1080.0, 10, &Content::default());
    let c = p.comp_mut(comp).ok()?;
    c.settings.width = 1920;
    c.settings.height = 1080;
    c.settings.duration = built.duration;
    c.layers = built.layers;
    let scene = xenify_anim::evaluate_project(&p, comp, built.duration - 0.01);
    let gpu = app.templates.gpu.get_or_insert_with(|| xenify_render::Offscreen::new().map_err(|e| e.to_string()));
    let img = gpu.as_mut().ok()?.render(&scene, THUMB.0, THUMB.1).ok()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    let tex = ctx.load_texture(format!("template-{key}"), image, egui::TextureOptions::LINEAR);
    app.templates.thumbs.insert(key, tex.clone());
    Some(tex)
}

fn image(ui: &mut egui::Ui, tex: &egui::TextureHandle, width: f32) {
    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(width, width * 9.0 / 16.0)).corner_radius(3.0));
}

/// Where the current slide is in the deck.
fn here(app: &XenifyApp) -> Option<usize> {
    crate::actions::current_slide(app)
}

/// Adds a `layout` slide after the current one and opens it.
pub fn new_slide(app: &mut XenifyApp, layout: Layout) {
    match xenify_ops::templates::add_layout_slide(app.project(), here(app), layout, None, &Content::default()) {
        Ok((cmd, id)) => {
            if app.exec(&format!("New {} slide", layout.label()), cmd) {
                app.open_comp(id);
                crate::dock::reveal(app, crate::dock::Panel::Slides);
                app.status = format!(
                    "Added a {} slide in {}. Click a text to change it; Slide › Apply Theme restyles the deck.",
                    layout.label(),
                    xenify_ops::templates::current_theme(app.project()).name
                );
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

pub fn apply_theme(app: &mut XenifyApp, theme: &Theme) {
    let cmd = xenify_ops::templates::apply_theme(app.project(), theme);
    if app.exec(&format!("Apply theme {}", theme.name), cmd) {
        app.status = format!("Applied {} to every slide (layers with a theme role).", theme.name);
    }
}

/// The folder saved layouts go in.
pub fn layouts_dir(app: &XenifyApp) -> Option<PathBuf> {
    app.templates.dir.clone().or_else(|| Some(xenify_platform::download::data_dir()?.join("layouts")))
}

/// Saved layouts: (name, file).
pub fn saved_layouts(app: &XenifyApp) -> Vec<(String, PathBuf)> {
    let Some(dir) = layouts_dir(app) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xenify"))
        .map(|p| (p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), p))
        .collect();
    out.sort();
    out
}

/// Saves the open slide as layout `name` (a one-slide project in the data folder).
pub fn save_layout(app: &mut XenifyApp, name: &str) -> Result<PathBuf, String> {
    let clean: String = name.chars().filter(|c| c.is_alphanumeric() || " -_".contains(*c)).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return Err("give the layout a name".into());
    }
    let comp = app.comp().clone();
    let mut p = Project::new();
    let used: std::collections::HashSet<Id> = comp.layers.iter().flat_map(|l| l.asset_refs()).collect();
    p.assets = app.project().assets.iter().filter(|a| used.contains(&a.id)).cloned().collect();
    p.theme = app.project().theme.clone();
    let top = comp.layers.iter().map(|l| l.id.0).chain(p.assets.iter().map(|a| a.id.0)).max().unwrap_or(0);
    p.compositions = vec![comp];
    p.reserve_id(Id(top.max(p.compositions[0].id.0)));
    let dir = layouts_dir(app).ok_or("there's no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{clean}.xenify"));
    std::fs::write(&path, p.to_json()).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn new_slide_from_saved(app: &mut XenifyApp, name: &str, path: &std::path::Path) {
    let saved = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|s| Project::from_json(&s).map_err(|e| e.to_string()));
    let result = saved.and_then(|saved| {
        xenify_ops::templates::add_saved_layout_slide(app.project(), here(app), &saved, name).map_err(|e| e.to_string())
    });
    match result {
        Ok((cmd, id)) => {
            if app.exec(&format!("New slide from {name}"), cmd) {
                app.open_comp(id);
            }
        }
        Err(e) => app.fail(format!("Couldn't use layout {name}: {e}")),
    }
}

/// The Slide menu's template items.
pub fn slide_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let theme = xenify_ops::templates::current_theme(app.project());
    ui.menu_button("New Slide From Layout", |ui| {
        for layout in Layout::ALL {
            let r = ui.button(layout.label());
            let tex = thumb(app, ui.ctx(), layout, theme);
            let r = r.on_hover_ui(|ui| {
                if let Some(t) = &tex {
                    image(ui, t, 240.0);
                }
            });
            if r.clicked() {
                new_slide(app, layout);
                ui.close();
            }
        }
        let saved = saved_layouts(app);
        if !saved.is_empty() {
            ui.separator();
            ui.label(RichText::new("My Layouts").weak());
            for (name, path) in saved {
                if ui.button(&name).clicked() {
                    new_slide_from_saved(app, &name, &path);
                    ui.close();
                }
            }
        }
    });
    ui.menu_button("Apply Theme", |ui| {
        for t in &THEMES {
            let current = app.project().theme.as_deref() == Some(t.name);
            let r = ui.selectable_label(current, t.name);
            let tex = thumb(app, ui.ctx(), Layout::Title, t);
            let r = r.on_hover_ui(|ui| {
                if let Some(tx) = &tex {
                    image(ui, tx, 240.0);
                }
            });
            if r.clicked() {
                apply_theme(app, t);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Recolours every slide's templated layers, and sets the transitions");
    ui.menu_button("Save Slide as Layout", |ui| {
        ui.label("Name");
        let mut name = std::mem::take(&mut app.templates.layout_name);
        if name.is_empty() {
            name = app.comp().settings.name.clone();
        }
        ui.text_edit_singleline(&mut name);
        if ui.button("Save Layout").clicked() {
            match save_layout(app, &name) {
                Ok(path) => {
                    app.status = format!(
                        "Saved layout “{name}” (Slide › New Slide From Layout › My Layouts; {}).",
                        path.display()
                    );
                    name.clear();
                    ui.close();
                }
                Err(e) => app.fail(format!("Couldn't save the layout: {e}")),
            }
        }
        app.templates.layout_name = name;
    });
}

/// The New dialog's Templates strip: a blank deck or a theme's starter deck.
/// Returns whether the choice changed.
pub fn gallery(app: &mut XenifyApp, ui: &mut egui::Ui, chosen: &mut Option<usize>) -> bool {
    let before = *chosen;
    ui.label(RichText::new("Templates").strong());
    egui::ScrollArea::horizontal().id_salt("template-gallery").show(ui, |ui| {
        ui.horizontal(|ui| {
            let blank = ui.add(egui::Button::new("Blank").selected(chosen.is_none()).min_size(egui::vec2(80.0, 72.0)));
            if blank.on_hover_text("An empty first slide").clicked() {
                *chosen = None;
            }
            for (i, t) in THEMES.iter().enumerate() {
                ui.vertical(|ui| {
                    let tex = thumb(app, ui.ctx(), Layout::Title, t);
                    let size = egui::vec2(128.0, 72.0);
                    let r = match &tex {
                        Some(tx) => ui.add(
                            egui::Button::image(egui::Image::new(tx).fit_to_exact_size(size))
                                .selected(*chosen == Some(i)),
                        ),
                        None => ui.add(egui::Button::new(t.name).selected(*chosen == Some(i)).min_size(size)),
                    };
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, *chosen == Some(i), t.name)
                    });
                    if r.on_hover_text(format!("{}: a starter deck (title, bullets, big stat, quote, closing)", t.name))
                        .clicked()
                    {
                        *chosen = Some(i);
                    }
                    ui.small(t.name);
                });
            }
        });
    });
    *chosen != before
}

/// Properties: the layer's theme role (what Apply Theme colours it as).
pub fn role_row(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    if app.project().slides.is_empty() && layer.meta.role.is_none() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label("Theme Role").on_hover_text("How Slide › Apply Theme colours this layer (None: leave it alone)");
        let current = layer.meta.role.map_or("None", Role::label);
        let mut pick = None;
        ui.menu_button(format!("{current} ⏷"), |ui| {
            if ui.selectable_label(layer.meta.role.is_none(), "None").clicked() {
                pick = Some(None);
                ui.close();
            }
            for r in Role::ALL {
                if ui.selectable_label(layer.meta.role == Some(r), r.label()).clicked() {
                    pick = Some(Some(r));
                    ui.close();
                }
            }
        });
        if let Some(role) = pick {
            let mut l = layer.clone();
            l.meta.role = role;
            let theme = xenify_ops::templates::current_theme(app.project());
            templates::restyle(&mut l, theme);
            app.exec("Theme role", Command::ReplaceLayer { layer: Box::new(l) });
        }
    });
}
