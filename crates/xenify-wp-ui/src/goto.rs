//! Edit › Go to page (Ctrl+G, or click "Page 3 of 9" in the status bar):
//! type a page number (or +2 / -1 to move from this page) and the caret
//! goes to the top of that page.

use crate::app::WriteApp;
use xenify_wp_doc::Selection;

/// What was typed: the page (1-based, counted from the first sheet) it
/// means, when it means one. `here` is the caret's page (1-based).
pub fn parse(text: &str, here: usize, pages: usize) -> Option<usize> {
    let t = text.trim();
    let n = if let Some(d) = t.strip_prefix('+') {
        here.checked_add(d.trim().parse().ok()?)?
    } else if let Some(d) = t.strip_prefix('-') {
        here.checked_sub(d.trim().parse().ok()?)?
    } else {
        t.parse().ok()?
    };
    (1..=pages).contains(&n).then_some(n)
}

/// Puts the caret at the top of page `n` (1-based) and scrolls to it.
pub fn go(app: &mut WriteApp, n: usize) {
    let Some(page) = app.view.layout.pages.get(n.saturating_sub(1)) else { return };
    let top = kurbo::Point::new(page.body.x0 + 1.0, page.body.y0 + 1.0);
    if let Some(pos) = app.view.layout.hit(top) {
        app.editor.sel = Selection::caret(pos);
        app.view.reveal_at_top();
    }
}

/// The dialog. `text` is what's typed; false once it closes.
pub fn dialog(app: &mut WriteApp, ctx: &egui::Context, text: &mut String) -> bool {
    let pages = app.view.layout.pages.len();
    let here = app.view.caret_page(&app.editor).map_or(1, |i| i + 1);
    let mut open = true;
    let mut keep = true;
    let mut chosen = None;
    crate::dialog::window(ctx, "Go to page", &mut open).default_width(300.0).show(ctx, |ui| {
        ui.label(format!("You're on page {here} of {pages}."));
        ui.horizontal(|ui| {
            ui.label("Page");
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(80.0).hint_text(here.to_string()));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Page number"));
            // The keyboard goes to the box when the dialog opens.
            let opened = egui::Id::new("go-to-page-opened");
            if !ui.data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
                r.request_focus();
                ui.data_mut(|d| d.insert_temp(opened, true));
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                chosen = parse(text, here, pages);
            }
        });
        ui.weak("Type +2 or -1 to move from this page.");
        let target = parse(text, here, pages);
        if !text.trim().is_empty() && target.is_none() {
            ui.colored_label(ui.visuals().error_fg_color, format!("There's no such page (1 to {pages})."));
        }
        use crate::dialog::Kind;
        let list = [("Go", if target.is_some() { Kind::Main } else { Kind::Off }), ("Close", Kind::Cancel)];
        match crate::dialog::buttons(ui, &list) {
            Some(0) => chosen = target,
            Some(_) => keep = false,
            None => {}
        }
    });
    if !(keep && open) || chosen.is_some() {
        ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("go-to-page-opened")));
    }
    if let Some(n) = chosen {
        go(app, n);
        keep = false;
        app.focus_document(ctx);
    }
    keep && open
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn pages_and_moves() {
        assert_eq!(parse("3", 1, 9), Some(3));
        assert_eq!(parse(" +2 ", 4, 9), Some(6));
        assert_eq!(parse("-1", 4, 9), Some(3));
        assert_eq!(parse("-5", 4, 9), None);
        assert_eq!(parse("10", 1, 9), None);
        assert_eq!(parse("0", 1, 9), None);
        assert_eq!(parse("abc", 1, 9), None);
    }
}
