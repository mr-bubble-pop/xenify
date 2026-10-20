//! The presenter view (like PowerPoint's): shown in this window while the
//! slides are on another screen, or in place of them when you press V to
//! rehearse. The current slide, the next one, your speaker notes, a timer
//! and the slide count, with buttons for everything the keys do.

use crate::app::XenifyApp;
use crate::present::{handle_keys, paint_frame, tick};
use egui::{Align2, Color32, FontId, Rect, RichText, vec2};

/// `inner` fitted inside `outer` keeping the slide's aspect ratio, centred.
fn fit(outer: Rect, aspect: f32) -> Rect {
    let w = outer.width().min(outer.height() * aspect);
    Rect::from_center_size(outer.center(), vec2(w, w / aspect))
}

/// "12:05" (or "1:02:05").
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Draws the presenter view into `rect`. Returns false to end the show.
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, rect: Rect, rs: Option<&eframe::egui_wgpu::RenderState>) -> bool {
    let project = app.editor.project().clone();
    let Some(p) = app.presenter.as_mut() else { return false };
    let mut alive = handle_keys(p, ui, false, false);
    tick(p, ui);
    ui.painter().rect_filled(rect, 0.0, Color32::from_gray(18));
    let pad = 16.0;
    let inner = rect.shrink(pad);
    let top = Rect::from_min_size(inner.min, vec2(inner.width(), 44.0));
    let body = Rect::from_min_max(egui::pos2(inner.left(), top.bottom() + 8.0), inner.max);
    let left = Rect::from_min_max(body.min, egui::pos2(body.left() + body.width() * 0.62, body.bottom()));
    let right = Rect::from_min_max(egui::pos2(left.right() + pad, body.top()), body.max);

    let deck = &p.playback.deck;
    let n = deck.len();
    let i = p.playback.slide();
    let aspect = deck
        .slide_comp(i)
        .and_then(|c| project.comp(c).ok())
        .map_or(16.0 / 9.0, |c| c.settings.width as f32 / c.settings.height.max(1) as f32);

    // Top bar: timer, slide count, controls.
    let mut ui_top =
        ui.new_child(egui::UiBuilder::new().max_rect(top).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let elapsed = p.timer.elapsed().as_secs();
    ui_top
        .label(RichText::new(clock(elapsed)).monospace().size(28.0).color(Color32::WHITE))
        .on_hover_text("Time since the presentation started");
    if ui_top.small_button("Reset").on_hover_text("Restart the timer").clicked() {
        p.timer = std::time::Instant::now();
    }
    ui_top.add_space(24.0);
    let status = if p.playback.ended {
        "End of presentation".to_string()
    } else if p.playback.until.is_none() {
        format!("Slide {} of {n} — waiting", i + 1)
    } else {
        format!("Slide {} of {n}", i + 1)
    };
    ui_top.label(RichText::new(status).size(18.0).color(Color32::from_gray(210)));
    ui_top.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(RichText::new("End show").size(15.0)).on_hover_text("Esc").clicked() {
            alive = false;
        }
        if p.screen == crate::present::Screen::ThisWindow
            && ui.button("Audience view").on_hover_text("V: what the audience sees").clicked()
        {
            p.presenter_view = false;
        }
        let black = if p.playback.black { "Show slides" } else { "Black screen" };
        if ui.button(black).on_hover_text("B").clicked() {
            p.playback.black = !p.playback.black;
        }
        if ui.button(RichText::new("Next ▶").size(15.0)).on_hover_text("→, Space, Page Down").clicked() {
            alive &= p.playback.next();
        }
        if ui.button(RichText::new("◀ Back").size(15.0)).on_hover_text("←, Page Up").clicked() {
            p.playback.previous();
        }
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));

    // The current slide, as the audience sees it.
    // (At the top left, so the eye doesn't have to hunt for it.)
    let now = fit(Rect::from_min_max(left.min, egui::pos2(left.right(), left.bottom() - 26.0)), aspect);
    let now = Rect::from_min_size(left.min, now.size());
    let painter = ui.painter_at(rect);
    painter.rect_filled(now, 0.0, Color32::BLACK);
    if !p.playback.ended && !p.playback.black {
        let frame = p.playback.frame();
        paint_frame(&mut p.view_canvas, &mut p.text, &project, frame, &painter, now, rs, &format!("Slide {}", i + 1));
    }
    painter.rect_stroke(now, 0.0, egui::Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Outside);
    painter.text(
        egui::pos2(now.left(), now.bottom() + 6.0),
        Align2::LEFT_TOP,
        if p.playback.black { "Current slide (the audience sees black)" } else { "Current slide" },
        FontId::proportional(13.0),
        Color32::from_gray(150),
    );

    // Next slide, settled as it will first wait.
    let next_h = (right.width() / aspect).min(right.height() * 0.4);
    let next = Rect::from_min_size(right.min + vec2(0.0, 18.0), vec2(next_h * aspect, next_h));
    painter.text(right.min, Align2::LEFT_TOP, "Next", FontId::proportional(13.0), Color32::from_gray(150));
    painter.rect_filled(next, 0.0, Color32::BLACK);
    if i + 1 < n {
        let deck = &p.playback.deck;
        let start = deck.slide_start(i + 1);
        let settle = deck.next_stop(start + 1e-6).unwrap_or(deck.slide_end(i + 1)).min(deck.slide_end(i + 1));
        let frame = deck.plain(i + 1, settle - start);
        paint_frame(&mut p.next_canvas, &mut p.text, &project, frame, &painter, next, rs, &format!("Slide {}", i + 2));
    } else {
        painter.text(
            next.center(),
            Align2::CENTER_CENTER,
            "End of the show",
            FontId::proportional(15.0),
            Color32::from_gray(140),
        );
    }
    painter.rect_stroke(next, 0.0, egui::Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Outside);

    // Speaker notes.
    let notes_rect = Rect::from_min_max(egui::pos2(right.left(), next.bottom() + 16.0), right.max);
    let notes = p.playback.deck.notes(i).to_string();
    let mut ui_notes =
        ui.new_child(egui::UiBuilder::new().max_rect(notes_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    ui_notes.label(RichText::new("Notes").size(13.0).color(Color32::from_gray(150)));
    egui::ScrollArea::vertical().id_salt(("notes", i)).show(&mut ui_notes, |ui| {
        if notes.trim().is_empty() {
            ui.label(
                RichText::new("No notes for this slide (add them in the Slides panel).")
                    .italics()
                    .color(Color32::from_gray(120)),
            );
        } else {
            ui.label(RichText::new(notes).size(22.0).color(Color32::WHITE));
        }
    });
    alive
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_reads_naturally() {
        assert_eq!(super::clock(5), "0:05");
        assert_eq!(super::clock(65), "1:05");
        assert_eq!(super::clock(3725), "1:02:05");
    }
}
