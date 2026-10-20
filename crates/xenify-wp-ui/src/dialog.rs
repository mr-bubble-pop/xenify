//! What every dialog shares: a centred window, a row of buttons along the
//! bottom (the main one coloured; Enter chooses it, Esc cancels), and
//! picture choices (a small drawing of each option above its name).

use egui::{Color32, Painter, Rect, RichText, Stroke, Ui};

/// Whether dialogs open as windows of their own (where the system allows);
/// otherwise inside the app's window. Set each frame from the settings.
pub fn set_own_windows(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("dialogs-own-windows"), on));
}

/// A dialog titled `title`: a window of its own centred over the app, or
/// (in a browser, in tests, or when turned off in Settings) a window inside
/// the app. `open` goes false when its close button is pressed.
pub fn window<'a>(_ctx: &egui::Context, title: impl Into<String>, open: &'a mut bool) -> Dialog<'a> {
    Dialog { title: title.into(), open, resizable: false, size: None, width: None }
}

pub struct Dialog<'a> {
    title: String,
    open: &'a mut bool,
    resizable: bool,
    size: Option<egui::Vec2>,
    width: Option<f32>,
}

impl Dialog<'_> {
    pub fn resizable(mut self, on: bool) -> Self {
        self.resizable = on;
        self
    }

    pub fn default_size(mut self, size: impl Into<egui::Vec2>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn default_width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn show<R>(self, ctx: &egui::Context, add: impl FnMut(&mut Ui) -> R) -> Option<R> {
        let own = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("dialogs-own-windows"))).unwrap_or(false);
        if own && !ctx.embed_viewports() {
            return Some(self.show_own(ctx, add));
        }
        let mut w = egui::Window::new(self.title.as_str())
            .open(self.open)
            .collapsible(false)
            .resizable(self.resizable)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center());
        if let Some(s) = self.size {
            w = w.default_size(s);
        }
        if let Some(x) = self.width {
            w = w.default_width(x);
        }
        w.show(ctx, add).and_then(|r| r.inner)
    }

    /// As a window of its own: centred over the app when it first opens,
    /// and (unless resizable) as big as what's in it.
    fn show_own<R>(self, ctx: &egui::Context, mut add: impl FnMut(&mut Ui) -> R) -> R {
        const MARGIN: f32 = 14.0;
        let id = egui::ViewportId::from_hash_of(("dialog", &self.title));
        let size = self.size.unwrap_or(egui::vec2(self.width.unwrap_or(420.0), 200.0));
        // Where it first appeared (kept, so moving it isn't undone next frame).
        let key = egui::Id::new(("dialog-pos", &self.title));
        let pos = ctx.data(|d| d.get_temp::<egui::Pos2>(key)).or_else(|| {
            let parent = ctx.input(|i| i.viewport().outer_rect)?;
            let p = parent.center() - size / 2.0;
            ctx.data_mut(|d| d.insert_temp(key, p));
            Some(p)
        });
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.title.as_str())
            .with_inner_size(size)
            .with_resizable(self.resizable)
            .with_minimize_button(false)
            .with_maximize_button(self.resizable);
        if let Some(p) = pos {
            builder = builder.with_position(p);
        }
        let open = self.open;
        let resizable = self.resizable;
        let out = ctx.show_viewport_immediate(id, builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                *open = false;
            }
            let inner = egui::CentralPanel::default()
                .frame(egui::Frame::central_panel(ui.style()).inner_margin(MARGIN))
                .show(ui, |ui| {
                    let r = add(ui);
                    (r, ui.min_rect().size())
                })
                .inner;
            // Fit the window to what's in it.
            let want = inner.1 + egui::vec2(2.0 * MARGIN, 2.0 * MARGIN);
            let now = ui.input(|i| i.viewport().inner_rect.map(|r| r.size()));
            if !resizable && now.is_some_and(|n| (n - want).length() > 1.5) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(want));
            }
            inner.0
        });
        if !*open {
            ctx.data_mut(|d| d.remove_temp::<egui::Pos2>(key));
        }
        out
    }
}

/// How a footer button looks and what keys choose it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The main action: coloured; Enter chooses it.
    Main,
    Other,
    /// Cancel or Close: Esc chooses it.
    Cancel,
    /// Shown greyed out.
    Off,
}

/// The row of buttons along the bottom of a dialog, right-aligned and in
/// the order given; returns the index of the one chosen (by click, Enter
/// for the main one when no text box is being typed in, Esc for Cancel).
pub fn buttons(ui: &mut Ui, list: &[(&str, Kind)]) -> Option<usize> {
    let mut chosen = None;
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(2.0);
    // One row high (a right-to-left layout would take all the height left).
    let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 10.0);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().button_padding = egui::vec2(14.0, 5.0);
        // Right to left: the last one given is drawn first.
        for (i, (text, kind)) in list.iter().enumerate().rev() {
            let b = match kind {
                Kind::Main => egui::Button::new(RichText::new(*text).color(Color32::WHITE))
                    .fill(crate::theme::accent(ui.visuals())),
                _ => egui::Button::new(*text),
            };
            let b = b.min_size(egui::vec2(76.0, 0.0));
            if ui.add_enabled(*kind != Kind::Off, b).clicked() {
                chosen = Some(i);
            }
        }
    });
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if chosen.is_none() && enter && !typing {
        chosen = list.iter().position(|(_, k)| *k == Kind::Main);
    }
    if chosen.is_none() && esc {
        chosen = list.iter().position(|(_, k)| *k == Kind::Cancel);
    }
    chosen
}

/// Whether Esc was pressed (for dialogs whose cancel isn't in [`buttons`]).
pub fn escape(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.key_pressed(egui::Key::Escape))
}

/// A heading for a group of options inside a dialog.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).strong());
}

/// A choice shown as a drawing with its name under it; `paint` draws the
/// option into the given rect (white, like paper). Returns true when clicked.
pub fn picture_choice(
    ui: &mut Ui,
    name: &str,
    hint: &str,
    selected: bool,
    size: egui::Vec2,
    paint: impl FnOnce(&Painter, Rect),
) -> bool {
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(size.x);
        let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter().rect_filled(rect, 3.0, Color32::WHITE);
        paint(ui.painter(), rect.shrink(5.0));
        let accent = crate::theme::accent(ui.visuals());
        let stroke = if selected {
            Stroke::new(2.5, accent)
        } else if r.hovered() {
            Stroke::new(1.5, accent)
        } else {
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        ui.painter().rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Outside);
        let r = if hint.is_empty() { r } else { r.on_hover_text(hint) };
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, name));
        clicked = r.clicked();
        let text = RichText::new(name).small();
        ui.label(if selected { text.strong() } else { text });
    });
    clicked
}

/// Grey lines of text filling `r` (for the drawings), skipping `gap`.
pub fn text_lines(p: &Painter, r: Rect, gap: Option<Rect>) {
    let ink = Color32::from_gray(0xb8);
    let mut y = r.top() + 2.0;
    while y + 2.0 <= r.bottom() {
        let line = Rect::from_min_max(egui::pos2(r.left(), y), egui::pos2(r.right(), y + 2.0));
        match gap.filter(|g| g.expand2(egui::vec2(0.0, 2.0)).intersects(line)) {
            // Text beside the gap, on whichever sides have room.
            Some(g) => {
                if g.left() - r.left() > 6.0 {
                    p.rect_filled(Rect::from_min_max(line.min, egui::pos2(g.left() - 3.0, line.max.y)), 1.0, ink);
                }
                if r.right() - g.right() > 6.0 {
                    p.rect_filled(Rect::from_min_max(egui::pos2(g.right() + 3.0, line.min.y), line.max), 1.0, ink);
                }
            }
            None => {
                p.rect_filled(line, 1.0, ink);
            }
        }
        y += 6.0;
    }
}

/// The picture in the drawings: a blue square with a hill.
pub fn picture(p: &Painter, r: Rect) {
    p.rect_filled(r, 2.0, Color32::from_rgb(0x9c, 0xc3, 0xf5));
    let hill = vec![
        egui::pos2(r.left(), r.bottom()),
        egui::pos2(r.left() + r.width() * 0.4, r.top() + r.height() * 0.45),
        egui::pos2(r.right(), r.bottom()),
    ];
    p.add(egui::Shape::convex_polygon(hill, Color32::from_rgb(0x3b, 0x82, 0xf6), Stroke::NONE));
}

/// How text sits around a picture or box, as drawn in [`wrap_drawing`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// In the text like a big letter (moves with it).
    InLine,
    Wrap(xenify_wp_doc::Wrap),
}

impl Placement {
    pub const ALL: [Placement; 5] = [
        Placement::InLine,
        Placement::Wrap(xenify_wp_doc::Wrap::Around),
        Placement::Wrap(xenify_wp_doc::Wrap::TopBottom),
        Placement::Wrap(xenify_wp_doc::Wrap::InFront),
        Placement::Wrap(xenify_wp_doc::Wrap::Behind),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Placement::InLine => "In line with text",
            Placement::Wrap(xenify_wp_doc::Wrap::Around) => "Text around it",
            Placement::Wrap(xenify_wp_doc::Wrap::TopBottom) => "Text above and below",
            Placement::Wrap(xenify_wp_doc::Wrap::InFront) => "In front of text",
            Placement::Wrap(xenify_wp_doc::Wrap::Behind) => "Behind text",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Placement::InLine => "Moves with the text, like a big letter",
            Placement::Wrap(xenify_wp_doc::Wrap::Around) => "Stays where you drag it; text flows beside it",
            Placement::Wrap(xenify_wp_doc::Wrap::TopBottom) => {
                "Stays where you drag it; text stops above and carries on below"
            }
            Placement::Wrap(xenify_wp_doc::Wrap::InFront) => "Stays where you drag it, covering the text",
            Placement::Wrap(xenify_wp_doc::Wrap::Behind) => {
                "Stays where you drag it, under the text (like a watermark)"
            }
        }
    }
}

/// A small page showing `how` text sits around a picture.
pub fn wrap_drawing(p: &Painter, r: Rect, how: Placement) {
    use xenify_wp_doc::Wrap;
    let pic = |w: f32, h: f32, x: f32| {
        Rect::from_min_size(egui::pos2(r.left() + (r.width() - w) * x, r.top() + r.height() * 0.3), egui::vec2(w, h))
    };
    match how {
        Placement::InLine => {
            let pr = pic(r.width() * 0.35, r.height() * 0.35, 0.0);
            let row = Rect::from_min_max(egui::pos2(r.left(), pr.top()), egui::pos2(r.right(), pr.bottom()));
            text_lines(p, r, Some(row));
            picture(p, pr);
            // The rest of its line, beside the picture's foot.
            p.rect_filled(
                Rect::from_min_max(egui::pos2(pr.right() + 3.0, pr.bottom() - 2.0), egui::pos2(r.right(), pr.bottom())),
                1.0,
                Color32::from_gray(0xb8),
            );
        }
        Placement::Wrap(Wrap::Around) => {
            let pr = pic(r.width() * 0.45, r.height() * 0.4, 0.0);
            text_lines(p, r, Some(pr));
            picture(p, pr);
        }
        Placement::Wrap(Wrap::TopBottom) => {
            let pr = pic(r.width() * 0.45, r.height() * 0.4, 0.5);
            let row = Rect::from_min_max(egui::pos2(r.left(), pr.top()), egui::pos2(r.right(), pr.bottom()));
            text_lines(p, r, Some(row));
            picture(p, pr);
        }
        Placement::Wrap(Wrap::InFront) => {
            text_lines(p, r, None);
            picture(p, pic(r.width() * 0.5, r.height() * 0.45, 0.5));
        }
        Placement::Wrap(Wrap::Behind) => {
            let pr = pic(r.width() * 0.6, r.height() * 0.5, 0.5);
            p.rect_filled(pr, 2.0, Color32::from_rgb(0xd6, 0xe6, 0xfb));
            text_lines(p, r, None);
        }
    }
}

/// A small page with a picture on its own line, aligned `a`.
pub fn align_drawing(p: &Painter, r: Rect, a: xenify_wp_doc::Align) {
    let w = r.width() * 0.4;
    let x = match a {
        xenify_wp_doc::Align::Center => r.left() + (r.width() - w) / 2.0,
        xenify_wp_doc::Align::Right => r.right() - w,
        _ => r.left(),
    };
    let pr = Rect::from_min_size(egui::pos2(x, r.top() + r.height() * 0.3), egui::vec2(w, r.height() * 0.4));
    let row = Rect::from_min_max(egui::pos2(r.left(), pr.top()), egui::pos2(r.right(), pr.bottom()));
    text_lines(p, r, Some(row));
    picture(p, pr);
}
