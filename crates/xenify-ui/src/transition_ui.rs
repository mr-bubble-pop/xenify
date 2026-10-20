//! Choosing and setting up slide transitions: a picker of every kind with
//! moving previews, the settings that kind has (length, direction, colour,
//! easing, softness), Smart Morph's list of matched layers, and
//! Slide › Transition.

use crate::app::XenifyApp;
use egui::{Color32, RichText};
use xenify_doc::{Color, Direction, Id, Project, Slide, Transition, TransitionKind};

/// Preview size (points) and frames in its loop.
const THUMB: egui::Vec2 = egui::vec2(112.0, 63.0);
const FRAMES: usize = 10;
/// New preview frames drawn per UI frame.
const PER_FRAME: usize = 4;

/// The picker's renderer and sample deck (made when first needed).
#[derive(Default)]
pub struct Previews {
    gpu: Option<Result<xenify_render::Offscreen, String>>,
}

/// Two sample slides: a blue one with a title and a yellow disc, then a
/// crimson one with the disc moved (so Smart Morph has something to glide).
fn sample(kind: TransitionKind) -> (Project, f64) {
    use xenify_doc::{CompSettings, Composition, Layer, LayerKind, ShapeItem, TextLayer, Vec2};
    let mut p = Project::new();
    let a = p.compositions[0].id;
    let settings = CompSettings { width: 320, height: 180, duration: 1.0, ..Default::default() };
    let title = |id: u64, s: &str| {
        Layer::new(Id(id), "Title", LayerKind::Text(TextLayer::new(s, "Inter", 34.0, Color::WHITE).centered()))
            .with_position(Vec2::new(160.0, 50.0))
    };
    let disc = |id: u64, x: f64| {
        Layer::shape(Id(id), "Disc", vec![ShapeItem::ellipse(Vec2::new(60.0, 60.0), Color::rgb(1.0, 0.8, 0.2))])
            .with_position(Vec2::new(x, 120.0))
    };
    p.compositions[0].settings = CompSettings { background: Color::rgb(0.12, 0.22, 0.55), ..settings.clone() };
    p.compositions[0].layers = vec![title(1, "One"), disc(2, 80.0)];
    let b = p.alloc_id();
    p.compositions.push(Composition {
        id: b,
        settings: CompSettings { background: Color::rgb(0.55, 0.1, 0.3), ..settings },
        layers: vec![title(101, "Two"), disc(102, 240.0)],
    });
    let mut second = Slide::new(b);
    second.transition_in = Transition { color: Color::WHITE, ..Transition::new(kind, 1.0) };
    p.slides = vec![Slide::new(a), second];
    (p, 1.0)
}

/// Preview frame `f` of `kind`, if ready (drawing it when the budget allows).
fn frame(
    ctx: &egui::Context,
    previews: &mut Previews,
    kind: TransitionKind,
    f: usize,
    budget: &mut usize,
) -> Option<egui::TextureHandle> {
    let key = egui::Id::new(("transition-preview", kind as usize, f));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    let (p, start) = sample(kind);
    let deck = xenify_anim::deck::Deck::new(&p);
    // A moment on the first slide, the transition, then a moment on the second.
    let t = start - 0.2 + 1.4 * f as f64 / (FRAMES - 1) as f64;
    let shot = deck.at(t)?;
    let (w, h) = ((THUMB.x * 2.0) as u32, (THUMB.y * 2.0) as u32);
    let gpu = previews.gpu.get_or_insert_with(|| xenify_render::Offscreen::new().map_err(|e| e.to_string()));
    let img = gpu.as_mut().ok()?.render_deck(&p, &shot, w, h).ok()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img.pixels);
    let tex = ctx.load_texture(format!("transition-{}-{f}", kind.label()), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}

/// A moving preview of `kind` in `rect`.
fn thumb(app: &mut XenifyApp, ui: &egui::Ui, rect: egui::Rect, kind: TransitionKind, budget: &mut usize) {
    let now = ui.input(|i| i.time);
    // Reduce motion: the transition half done, standing still.
    let still = crate::theme::reduce_motion(ui.ctx());
    let f = if still { FRAMES / 2 } else { ((now * FRAMES as f64 / 1.6) as usize) % FRAMES };
    let ctx = ui.ctx().clone();
    let tex = frame(&ctx, &mut app.transition_previews, kind, f, budget)
        .or_else(|| frame(&ctx, &mut app.transition_previews, kind, 0, budget));
    match tex {
        Some(t) => {
            ui.painter().image(
                t.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(30));
        }
    }
    if !still || *budget == 0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }
}

/// The kind picker (a button opening a grid of previews) and the settings
/// that kind has. `id` keeps two editors apart.
pub fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, id: &str, t: &mut Transition) {
    let mut budget = PER_FRAME;
    ui.horizontal(|ui| {
        let r = ui.menu_button(format!("{} ⏷", t.kind.label()), |ui| {
            ui.set_max_width(4.0 * (THUMB.x + 10.0));
            egui::Grid::new(("transition-grid", id)).spacing([6.0, 6.0]).show(ui, |ui| {
                for (n, kind) in TransitionKind::ALL.into_iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(THUMB + egui::vec2(0.0, 16.0), egui::Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, kind.label()));
                    thumb(app, ui, egui::Rect::from_min_size(rect.min, THUMB), kind, &mut budget);
                    let on = t.kind == kind;
                    if on || resp.hovered() {
                        let colour = if on { crate::widgets::ACCENT } else { Color32::from_gray(160) };
                        ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(2.0, colour), egui::StrokeKind::Inside);
                    }
                    ui.painter().text(
                        egui::pos2(rect.min.x + 4.0, rect.max.y - 8.0),
                        egui::Align2::LEFT_CENTER,
                        kind.label(),
                        egui::FontId::proportional(12.0),
                        ui.visuals().text_color(),
                    );
                    if resp.clicked() {
                        t.kind = kind;
                        ui.close();
                    }
                    if n % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
        });
        r.response.on_hover_text("Choose how the slides change over (with moving previews)");
        ui.add_enabled(
            t.kind != TransitionKind::Cut,
            egui::DragValue::new(&mut t.duration).range(0.1..=10.0).speed(0.05).suffix(" s"),
        )
        .on_hover_text("How long it takes");
    });
    if t.kind == TransitionKind::Cut {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        if t.kind.has_direction() {
            ui.label("Towards");
            for d in Direction::ALL {
                ui.selectable_value(&mut t.direction, d, d.label());
            }
        }
        if t.kind.has_color() {
            ui.label("Colour");
            crate::color_picker::button(ui, ("transition-colour", id), &mut t.color);
        }
        if t.kind.has_softness() {
            ui.label("Soft edge");
            let mut soft = t.softness * 100.0;
            if ui.add(egui::DragValue::new(&mut soft).range(0.0..=100.0).speed(0.5).suffix(" %")).changed() {
                t.softness = soft / 100.0;
            }
        }
        ui.label("Easing");
        let current = xenify_doc::Easing::PRESETS.iter().find(|(_, e)| *e == t.easing).map(|(n, _)| *n);
        ui.menu_button(current.map_or("custom".to_string(), crate::curve::preset_label), |ui| {
            if let Some(e) = crate::curve::preset_picker(ui, Some(t.easing)) {
                t.easing = e;
                ui.close();
            }
        });
    });
}

/// Smart Morph's matches into slide `i` from the slide before: which layers
/// glide (same name), and a way to pair the rest by renaming.
pub fn morph_matches(app: &mut XenifyApp, ui: &mut egui::Ui, i: usize) {
    let slides = &app.project().slides;
    let (Some(prev), Some(this)) = (i.checked_sub(1).and_then(|p| slides.get(p)), slides.get(i)) else {
        ui.weak("Smart Morph needs a slide before this one.");
        return;
    };
    let (a, b) = (prev.comp, this.comp);
    let pairs = xenify_anim::transition::morph_pairs(app.project(), a, b);
    let (Ok(ca), Ok(cb)) = (app.project().comp(a), app.project().comp(b)) else { return };
    let paired_b: Vec<Id> = pairs.iter().map(|(_, y, _)| *y).collect();
    let paired_a: Vec<Id> = pairs.iter().map(|(x, _, _)| *x).collect();
    let lone_a: Vec<String> = ca
        .layers
        .iter()
        .filter(|l| l.meta.parent.is_none() && !paired_a.contains(&l.id))
        .map(|l| l.meta.name.clone())
        .collect();
    let lone_b: Vec<(Id, String)> = cb
        .layers
        .iter()
        .filter(|l| l.meta.parent.is_none() && !paired_b.contains(&l.id))
        .map(|l| (l.id, l.meta.name.clone()))
        .collect();
    ui.label(RichText::new(format!("Match ({} gliding)", pairs.len())).strong())
        .on_hover_text("Layers with the same name glide from the slide before; the rest fade");
    for (_, _, name) in &pairs {
        ui.label(format!("↔ {name}"));
    }
    let mut rename = None;
    for (id, name) in &lone_b {
        ui.horizontal(|ui| {
            ui.weak(format!("fades in: {name}"));
            if !lone_a.is_empty() {
                ui.menu_button("Pair with…", |ui| {
                    for other in &lone_a {
                        if ui.button(other).on_hover_text("Rename this slide's layer to match").clicked() {
                            rename = Some((*id, other.clone()));
                            ui.close();
                        }
                    }
                });
            }
        });
    }
    if let Some((id, name)) = rename
        && let Ok(l) = app.project().layer(id)
    {
        let meta = xenify_doc::LayerMeta { name, ..l.meta.clone() };
        app.exec("Pair for Smart Morph", xenify_ops::Command::SetLayerMeta { layer: id, meta });
    }
}

/// Slide › Transition: how the open slide comes in.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let Some(i) = app.project().slides.iter().position(|s| s.comp == app.comp) else {
        ui.weak("Open a slide first");
        return;
    };
    let current = app.project().slides[i].transition_in.kind;
    for kind in TransitionKind::ALL {
        if ui.selectable_label(current == kind, kind.label()).clicked() {
            if let Ok(cmd) = xenify_ops::slides::edit_slide(app.project(), i, |s| s.transition_in.kind = kind) {
                app.exec("Slide transition", cmd);
            }
            ui.close();
        }
    }
}
