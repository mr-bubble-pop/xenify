//! The Slides panel: the presentation's slides in order (click to edit one,
//! drag to reorder, right-click for more), with the current slide's
//! transitions and speaker notes.

use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use xenify_doc::{Id, Slide, TransitionKind};
use xenify_ops::slides;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const THUMB_W: f32 = 132.0;

/// Slide pictures, rendered on the GPU off screen and cached.
#[derive(Default)]
pub struct SlideThumbs {
    gpu: Option<Result<xenify_render::Offscreen, String>>,
    /// Per composition: the texture and the project revision it shows.
    cache: HashMap<Id, (egui::TextureHandle, u64)>,
    doc: Option<crate::documents::DocId>,
    last_render: Option<Instant>,
}

impl SlideThumbs {
    /// The picture of `comp`, re-rendering at most one out-of-date slide
    /// every few frames (the one being edited first).
    fn get(
        &mut self,
        ctx: &egui::Context,
        app: &XenifyApp,
        comp: Id,
        allowed: &mut bool,
    ) -> Option<egui::TextureHandle> {
        let rev = app.editor.revision();
        let cached = self.cache.get(&comp).cloned();
        // Only the slide being edited can have changed since it was drawn.
        let stale = match &cached {
            None => true,
            Some((_, r)) => *r != rev && comp == app.comp,
        };
        let due = self.last_render.is_none_or(|t| t.elapsed() > Duration::from_millis(250));
        if stale && *allowed && due {
            *allowed = false;
            self.last_render = Some(Instant::now());
            if let Some(tex) = self.render(ctx, app, comp) {
                self.cache.insert(comp, (tex.clone(), rev));
                return Some(tex);
            }
        } else if stale {
            ctx.request_repaint_after(Duration::from_millis(260));
        }
        cached.map(|c| c.0)
    }

    fn render(&mut self, ctx: &egui::Context, app: &XenifyApp, comp: Id) -> Option<egui::TextureHandle> {
        let gpu = self.gpu.get_or_insert_with(|| xenify_render::Offscreen::new().map_err(|e| e.to_string()));
        let gpu = gpu.as_mut().ok()?;
        let c = app.project().comp(comp).ok()?;
        let k = (2.0 * THUMB_W as f64 / c.settings.width.max(1) as f64).min(1.0);
        let (w, h) = (((c.settings.width as f64 * k) as u32).max(1), ((c.settings.height as f64 * k) as u32).max(1));
        // Slides are shown finished (at their end).
        let img = gpu.render_frame(app.project(), comp, c.settings.duration, w, h, false).ok()?;
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img.pixels);
        Some(ctx.load_texture(format!("slide-{}", comp.0), image, egui::TextureOptions::LINEAR))
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let active = app.docs.active();
    if app.slide_thumbs.doc != Some(active) {
        app.slide_thumbs = SlideThumbs { gpu: app.slide_thumbs.gpu.take(), doc: Some(active), ..Default::default() };
    }
    let list = slides::slides_or_main(app.project());
    let current = list.iter().position(|s| s.comp == app.comp);
    let ctx = ui.ctx().clone();

    ui.horizontal(|ui| {
        ui.heading("Slides");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let present = crate::keymap::Action::PresentCurrent;
            if ui.button("▶").on_hover_text(format!("Present from this slide ({})", app.keymap.text(present))).clicked()
            {
                crate::actions::run(app, &ctx, present);
            }
            let new = crate::keymap::Action::NewSlide;
            if ui.button("+").on_hover_text(format!("New slide ({})", app.keymap.text(new))).clicked() {
                crate::actions::run(app, &ctx, new);
            }
        });
    });
    if app.project().slides.is_empty() {
        ui.weak("Add a slide (+) to turn this into a presentation.");
    }
    ui.separator();

    let mut drop: Option<(usize, usize)> = None;
    let mut allowed = true;
    let bottom = if current.is_some() { 230.0 } else { 0.0 };
    egui::ScrollArea::vertical().max_height((ui.available_height() - bottom).max(80.0)).auto_shrink(false).show(
        ui,
        |ui| {
            for (i, slide) in list.iter().enumerate() {
                row(app, ui, i, slide, current == Some(i), &mut allowed, &mut drop);
            }
        },
    );
    if let Some((from, to)) = drop {
        match slides::move_slide(app.project(), from, to) {
            Ok(Some(cmd)) => {
                app.exec("Move slide", cmd);
            }
            Ok(None) => {}
            Err(e) => app.fail(e.to_string()),
        }
    }
    if let Some(i) = current.filter(|_| !app.project().slides.is_empty()) {
        ui.separator();
        slide_settings(app, ui, i);
    }
}

fn row(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    i: usize,
    slide: &Slide,
    selected: bool,
    allowed: &mut bool,
    drop: &mut Option<(usize, usize)>,
) {
    let Ok(c) = app.project().comp(slide.comp).cloned() else { return };
    let aspect = c.settings.height as f32 / c.settings.width.max(1) as f32;
    let name = c.settings.name.clone();
    // The thumbnail fills the panel's width; the name goes under it.
    let thumb_w = (ui.available_width() - 32.0).clamp(60.0, 260.0);
    let thumb_h = (thumb_w * aspect).clamp(24.0, 200.0);
    let h = thumb_h + 28.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    let r = ui.interact(rect, egui::Id::new(("slide-row", slide.comp)), Sense::click_and_drag());
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, format!("Slide {} {name}", i + 1))
    });
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 4.0, ACCENT.gamma_multiply(0.25));
    } else if r.hovered() {
        painter.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    painter.text(
        Pos2::new(rect.left() + 6.0, rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        format!("{}", i + 1),
        egui::FontId::proportional(13.0),
        Color32::GRAY,
    );
    let thumb = Rect::from_min_size(Pos2::new(rect.left() + 24.0, rect.top() + 5.0), Vec2::new(thumb_w, thumb_h));
    let ctx = ui.ctx().clone();
    let mut thumbs = std::mem::take(&mut app.slide_thumbs);
    let tex = thumbs.get(&ctx, app, slide.comp, allowed);
    app.slide_thumbs = thumbs;
    match tex {
        Some(t) => {
            painter.image(t.id(), thumb, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(thumb, 2.0, ui.visuals().extreme_bg_color);
        }
    }
    let border = if selected { Stroke::new(2.0, ACCENT) } else { Stroke::new(1.0, Color32::from_gray(70)) };
    painter.rect_stroke(thumb, 2.0, border, egui::StrokeKind::Outside);
    let mut badges = Vec::new();
    if slide.transition_in.kind != TransitionKind::Cut {
        badges.push(format!("⤵ {}", slide.transition_in.kind.label()));
    }
    if !c.settings.pauses.is_empty() {
        badges.push(format!("⏸ {}", c.settings.pauses.len()));
    }
    let label = if badges.is_empty() { name.clone() } else { format!("{name}   {}", badges.join("  ")) };
    let galley = painter.layout(label, egui::FontId::proportional(12.0), ui.visuals().text_color(), thumb_w);
    painter.galley(Pos2::new(thumb.left(), thumb.bottom() + 4.0), galley, ui.visuals().text_color());
    let r = r.on_hover_text(if slide.notes.is_empty() { name.clone() } else { format!("{name}\n\n{}", slide.notes) });

    if r.clicked() {
        app.open_comp(slide.comp);
    }
    if r.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), SlideDrag(i));
    }
    if let Some(p) = ui.ctx().pointer_latest_pos().filter(|p| rect.contains(*p))
        && let Some(from) = egui::DragAndDrop::payload::<SlideDrag>(ui.ctx()).map(|d| d.0)
        && from != i
    {
        let below = p.y > rect.center().y;
        let y = if below { rect.bottom() - 1.0 } else { rect.top() + 1.0 };
        painter.hline(rect.x_range(), y, Stroke::new(2.0, ACCENT));
        if ui.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            let mut to = if below { i + 1 } else { i };
            if from < to {
                to -= 1;
            }
            *drop = Some((from, to));
        }
    }
    r.context_menu(|ui| {
        if ui.button("New Slide After").clicked() {
            add_after(app, Some(i));
            ui.close();
        }
        if ui.button("Duplicate Slide").clicked() {
            match slides::duplicate_slide(app.project(), i) {
                Ok((cmd, id)) => {
                    if app.exec("Duplicate slide", cmd) {
                        app.open_comp(id);
                    }
                }
                Err(e) => app.fail(e.to_string()),
            }
            ui.close();
        }
        if ui.button("Present From Here").clicked() {
            let screen = crate::present::default_screen();
            crate::present::start(app, i, screen, &ctx);
            ui.close();
        }
        ui.separator();
        let n = slides::slides_or_main(app.project()).len();
        if ui.add_enabled(n > 1, egui::Button::new("Delete Slide")).on_disabled_hover_text("The only slide").clicked() {
            delete(app, i);
            ui.close();
        }
    });
}

#[derive(Clone, Copy)]
struct SlideDrag(usize);

/// Adds a slide after `after` (after the current one when `None`) and opens it.
pub fn add_after(app: &mut XenifyApp, after: Option<usize>) {
    let after = after.or_else(|| slides::slides_or_main(app.project()).iter().position(|s| s.comp == app.comp));
    match slides::add_slide(app.project(), after) {
        Ok((cmd, id)) => {
            if app.exec("New slide", cmd) {
                app.open_comp(id);
                if !crate::dock::is_open(app, crate::dock::Panel::Slides) {
                    crate::dock::toggle(app, crate::dock::Panel::Slides);
                }
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

pub fn delete(app: &mut XenifyApp, i: usize) {
    let list = slides::slides_or_main(app.project());
    let showing = list.get(i).map(|s| s.comp) == Some(app.comp);
    let next = list.get(if i + 1 < list.len() { i + 1 } else { i.saturating_sub(1) }).map(|s| s.comp);
    match slides::delete_slide(app.project(), i) {
        Ok(cmd) => {
            // Leave the slide before deleting it.
            if showing && let Some(n) = next {
                app.open_comp(n);
            }
            app.exec("Delete slide", cmd);
        }
        Err(e) => app.fail(e.to_string()),
    }
}

/// Transitions and notes of slide `i`.
fn slide_settings(app: &mut XenifyApp, ui: &mut egui::Ui, i: usize) {
    let Some(slide) = app.project().slides.get(i).cloned() else { return };
    ui.label(RichText::new(format!("Slide {}", i + 1)).strong());
    let mut edited = slide.clone();
    ui.label("Comes in").on_hover_text("How this slide replaces the one before");
    crate::transition_ui::editor(app, ui, "in", &mut edited.transition_in);
    if edited.transition_in.kind == TransitionKind::SmartMorph {
        crate::transition_ui::morph_matches(app, ui, i);
    }
    ui.add_space(4.0);
    ui.label("Goes out").on_hover_text("How this slide leaves, when the next slide doesn't say how it comes in (Cut)");
    crate::transition_ui::editor(app, ui, "out", &mut edited.transition_out);
    ui.add_space(4.0);
    ui.label("Speaker notes");
    ui.add(
        egui::TextEdit::multiline(&mut edited.notes)
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .hint_text("What to say on this slide"),
    );
    if edited != slide
        && let Ok(cmd) = slides::edit_slide(app.project(), i, |s| *s = edited)
    {
        app.exec_merged("Edit slide", cmd);
    }
}
