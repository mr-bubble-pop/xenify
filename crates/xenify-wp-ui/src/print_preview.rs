//! File › Print (Ctrl+P): the window turns into a print screen. Settings on
//! the left (printer, copies, which pages), the page as it will print on the
//! right; Page Up/Down or the arrows go through the pages; Esc goes back.

use crate::app::WriteApp;
use crate::canvas::Canvas;
use crate::icons::{self, Icon};
use egui::{Color32, Pos2, Rect, Vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pages {
    All,
    Current,
    Range,
}

pub struct PrintPreview {
    /// The page shown (0-based).
    pub page: usize,
    printers: Vec<String>,
    printer: usize,
    copies: u32,
    pub pages: Pages,
    /// Pages to print when [`Pages::Range`]: "1-3, 5".
    pub range: String,
    /// Why printing isn't possible here, if it isn't.
    unavailable: Option<String>,
    canvas: Canvas,
    rendered: Option<(u64, usize, [i32; 4], u32)>,
}

impl PrintPreview {
    pub fn new(page: usize) -> Self {
        let unavailable = crate::print::unavailable();
        let printers = if unavailable.is_none() { crate::print::printers() } else { Vec::new() };
        Self {
            page,
            printers,
            printer: 0,
            copies: 1,
            pages: Pages::All,
            range: String::new(),
            unavailable,
            canvas: Canvas::default(),
            rendered: None,
        }
    }
}

/// `text` as a page list for the printer ("1-3,5"), checked against `count`
/// pages; None when it isn't one.
pub fn page_list(text: &str, count: usize) -> Option<String> {
    let mut parts = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = part.split_once('-').map_or((part, part), |(a, b)| (a.trim(), b.trim()));
        let (a, b): (usize, usize) = (a.parse().ok()?, b.parse().ok()?);
        if a == 0 || b < a || b > count {
            return None;
        }
        parts.push(if a == b { a.to_string() } else { format!("{a}-{b}") });
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

/// The page the caret is on (0-based).
fn caret_page(app: &WriteApp) -> usize {
    let layout = &app.view.layout;
    let y = layout.caret(app.editor.sel.focus, false).map_or(0.0, |r| r.center().y);
    layout.pages.iter().position(|p| p.rect.y0 <= y && y <= p.rect.y1).unwrap_or(0)
}

pub fn open(app: &mut WriteApp) {
    let page = caret_page(app);
    app.print_preview = Some(PrintPreview::new(page));
}

/// Draws the print screen in place of the editor. Returns false when it's closed.
pub fn show(app: &mut WriteApp, ui: &mut egui::Ui, frame: &eframe::Frame) {
    let Some(mut pp) = app.print_preview.take() else { return };
    let count = app.view.layout.pages.len().max(1);
    pp.page = pp.page.min(count - 1);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let mut print = None;
    let mut export = false;
    let mut viewer = false;
    if ui.input(|i| i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::ArrowRight)) {
        pp.page = (pp.page + 1).min(count - 1);
    }
    if ui.input(|i| i.key_pressed(egui::Key::PageUp) || i.key_pressed(egui::Key::ArrowLeft)) {
        pp.page = pp.page.saturating_sub(1);
    }
    egui::Panel::left("print-settings").resizable(false).exact_size(300.0).show(ui, |ui| {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if icons::button(ui, Icon::Close, "Back", "Back to the document (Esc)").clicked() {
                close = true;
            }
            ui.heading("Print");
        });
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 10.0;
        match &pp.unavailable {
            Some(why) => {
                ui.label(format!("Printing isn't available: {why}."));
                ui.label("Export as PDF and print that instead.");
            }
            None if pp.printers.is_empty() => {
                ui.label("No printers are set up.");
            }
            None => {
                egui::ComboBox::from_label("Printer").selected_text(pp.printers[pp.printer].clone()).show_ui(
                    ui,
                    |ui| {
                        for (i, p) in pp.printers.iter().enumerate() {
                            ui.selectable_value(&mut pp.printer, i, p);
                        }
                    },
                );
                ui.add(egui::DragValue::new(&mut pp.copies).range(1..=99).prefix("Copies: "));
            }
        }
        ui.label(egui::RichText::new("Pages").strong());
        ui.radio_value(&mut pp.pages, Pages::All, format!("All pages ({count})"));
        ui.radio_value(&mut pp.pages, Pages::Current, format!("This page ({})", pp.page + 1));
        ui.horizontal(|ui| {
            ui.radio_value(&mut pp.pages, Pages::Range, "Pages:");
            let r = ui.add(egui::TextEdit::singleline(&mut pp.range).hint_text("e.g. 1-3, 5").desired_width(110.0));
            if r.changed() {
                pp.pages = Pages::Range;
            }
        });
        let list = match pp.pages {
            Pages::All => Some(None),
            Pages::Current => Some(Some((pp.page + 1).to_string())),
            Pages::Range => page_list(&pp.range, count).map(Some),
        };
        if list.is_none() {
            ui.colored_label(ui.visuals().error_fg_color, format!("Use page numbers from 1 to {count}, like 1-3, 5."));
        }
        ui.add_space(8.0);
        let can_print = pp.unavailable.is_none() && !pp.printers.is_empty() && list.is_some();
        ui.horizontal(|ui| {
            let b = egui::Button::new(egui::RichText::new("Print").strong()).min_size(Vec2::new(90.0, 30.0));
            if ui.add_enabled(can_print, b).clicked()
                && let Some(pages) = list.clone()
            {
                print = Some((pp.printers[pp.printer].clone(), pp.copies, pages));
            }
            if ui.button("Export as PDF…").clicked() {
                export = true;
            }
        });
        if ui.button("Open in PDF viewer").on_hover_text("Print from there, with its own print settings").clicked() {
            viewer = true;
        }
    });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.add_enabled(pp.page > 0, egui::Button::new("◀")).on_hover_text("Previous page (Page Up)").clicked()
            {
                pp.page -= 1;
            }
            ui.label(format!("Page {} of {count}", pp.page + 1));
            if ui
                .add_enabled(pp.page + 1 < count, egui::Button::new("▶"))
                .on_hover_text("Next page (Page Down)")
                .clicked()
            {
                pp.page += 1;
            }
        });
        let area = ui.available_rect_before_wrap().shrink(16.0);
        paint_page(app, &mut pp, ui, frame, area);
    });
    if let Some((printer, copies, pages)) = print {
        match app.export_pdf_bytes().and_then(|pdf| crate::print::print_pdf(&pdf, &printer, copies, pages.as_deref())) {
            Ok(()) => {
                app.message = Some((format!("Sent to {printer}"), false));
                close = true;
            }
            Err(e) => app.message = Some((format!("Couldn't print: {e}"), true)),
        }
    }
    if export {
        close = true;
        app.open_export_pdf();
    }
    if viewer {
        match app.export_pdf_bytes().and_then(|pdf| crate::print::open_in_viewer(&pdf)) {
            Ok(()) => app.message = Some(("Opened in your PDF viewer".into(), false)),
            Err(e) => app.message = Some((format!("Couldn't open the PDF viewer: {e}"), true)),
        }
    }
    if !close {
        app.print_preview = Some(pp);
    }
}

/// The page, as large as fits in `area`, with a soft shadow.
fn paint_page(app: &mut WriteApp, pp: &mut PrintPreview, ui: &egui::Ui, frame: &eframe::Frame, area: Rect) {
    let Some(page) = app.view.layout.pages.get(pp.page) else { return };
    let (pw, ph) = (page.rect.width() as f32, page.rect.height() as f32);
    let s = (area.width() / pw).min(area.height() / ph).max(0.05);
    let ppp = ui.ctx().pixels_per_point();
    let size = Vec2::new(pw * s, ph * s);
    let min = area.center() - size / 2.0;
    let min = Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
    let rect = Rect::from_min_size(min, size);
    let painter = ui.painter_at(ui.clip_rect());
    painter.rect_filled(rect.translate(Vec2::new(0.0, 3.0)).expand(1.0), 2.0, Color32::from_black_alpha(60));
    let Some(rs) = frame.wgpu_render_state() else {
        painter.rect_filled(rect, 0.0, Color32::WHITE);
        return;
    };
    let px = ((size.x * ppp).round() as u32, (size.y * ppp).round() as u32);
    let key = (
        app.view.layout_revision(),
        pp.page,
        [rect.min.x, rect.min.y, rect.max.x, rect.max.y].map(|v| (v * 4.0).round() as i32),
        ppp.to_bits(),
    );
    let texture = if pp.rendered == Some(key) {
        pp.canvas.texture()
    } else {
        let t = kurbo::Affine::scale((s * ppp) as f64) * kurbo::Affine::translate((-page.rect.x0, -page.rect.y0));
        let visible = page.rect;
        let layout = &app.view.layout;
        let images = &app.view.images;
        match pp.canvas.render(rs, px, vello::peniko::Color::WHITE, |scene| {
            xenify_wp_layout::paint::paint_pages(scene, layout, t, visible);
            xenify_wp_layout::paint::paint_with_images(scene, layout, t, visible, &|src| images.lookup(src));
        }) {
            Ok(id) => {
                pp.rendered = Some(key);
                Some(id)
            }
            Err(e) => {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::default(), Color32::RED);
                None
            }
        }
    };
    if let Some(id) = texture {
        painter.image(id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::page_list;

    #[test]
    fn page_lists() {
        assert_eq!(page_list("1-3, 5", 6).as_deref(), Some("1-3,5"));
        assert_eq!(page_list(" 2 ", 2).as_deref(), Some("2"));
        assert_eq!(page_list("3-3", 4).as_deref(), Some("3"));
        assert_eq!(page_list("0", 4), None);
        assert_eq!(page_list("5", 4), None);
        assert_eq!(page_list("3-1", 4), None);
        assert_eq!(page_list("a", 4), None);
        assert_eq!(page_list("", 4), None);
    }
}
