//! File ▸ New: pick a canvas size (or start from what's on the clipboard) and
//! open it in a new tab.

use crate::app::XenifyApp;
use xenify_doc::{CompSettings, Project};
use xenify_platform::desktop::ClipContent;

/// What a canvas size is for (the dialog's tabs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Video,
    Social,
    Slides,
    /// Diagram canvases (flowcharts, networks): the diagram tools come with them.
    Diagram,
    Print,
    Other,
}

impl Family {
    pub const ALL: [Family; 6] =
        [Family::Video, Family::Social, Family::Slides, Family::Diagram, Family::Print, Family::Other];

    pub fn label(self) -> &'static str {
        match self {
            Family::Video => "Video",
            Family::Social => "Social",
            Family::Slides => "Slides",
            Family::Diagram => "Diagram",
            Family::Print => "Print",
            Family::Other => "Screens & icons",
        }
    }
}

/// A preset's size: pixels, or paper in millimetres / inches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    Px(u32, u32),
    Mm(f64, f64),
    In(f64, f64),
}

impl Dim {
    /// Pixels at `dpi` (paper sizes).
    pub fn pixels(self, dpi: f64) -> (u32, u32) {
        let px = |v: f64| (v * dpi).round().max(1.0) as u32;
        match self {
            Dim::Px(w, h) => (w, h),
            Dim::Mm(w, h) => (px(w / 25.4), px(h / 25.4)),
            Dim::In(w, h) => (px(w), px(h)),
        }
    }

    /// Width and height in one comparable measure (mm for paper, px otherwise).
    fn measure(self) -> (f64, f64) {
        match self {
            Dim::Px(w, h) => (w as f64, h as f64),
            Dim::Mm(w, h) => (w, h),
            Dim::In(w, h) => (w * 25.4, h * 25.4),
        }
    }

    fn turned(self, landscape: bool) -> Dim {
        let (w, h) = self.measure();
        if (w > h) == landscape || w == h {
            return self;
        }
        match self {
            Dim::Px(w, h) => Dim::Px(h, w),
            Dim::Mm(w, h) => Dim::Mm(h, w),
            Dim::In(w, h) => Dim::In(h, w),
        }
    }

    fn label(self) -> String {
        let n = |v: f64| if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v}") };
        match self {
            Dim::Px(w, h) => format!("{w} × {h}"),
            Dim::Mm(w, h) => format!("{} × {} mm", n(w), n(h)),
            Dim::In(w, h) => format!("{} × {} in", n(w), n(h)),
        }
    }
}

pub struct CanvasPreset {
    pub name: &'static str,
    pub family: Family,
    pub dim: Dim,
    /// What it's usually for.
    pub note: &'static str,
}

const fn p(family: Family, name: &'static str, dim: Dim, note: &'static str) -> CanvasPreset {
    CanvasPreset { name, family, dim, note }
}

use Dim::{In, Mm, Px};
use Family::{Diagram, Other, Print, Slides, Social, Video};

/// Common sizes, by what they're for.
pub const CANVAS_PRESETS: &[CanvasPreset] = &[
    p(Video, "1080p Full HD", Px(1920, 1080), "The usual video size: YouTube, web, most screens"),
    p(Video, "720p HD", Px(1280, 720), "Smaller files; web and email"),
    p(Video, "1440p QHD", Px(2560, 1440), "Sharper, for big monitors"),
    p(Video, "4K UHD", Px(3840, 2160), "TVs and big screens (heavy to render)"),
    p(Social, "Square post", Px(1080, 1080), "Instagram and Facebook feeds"),
    p(Social, "Portrait post", Px(1080, 1350), "Instagram 4:5: takes more of the feed"),
    p(Social, "Story / reel", Px(1080, 1920), "Stories, Reels, TikTok, Shorts (full-screen phone)"),
    p(Social, "Wide post", Px(1600, 900), "X, LinkedIn and link previews"),
    p(Social, "Banner", Px(1500, 500), "Profile headers"),
    p(Slides, "Slide 16:9", Px(1920, 1080), "Modern screens and projectors"),
    p(Slides, "Slide 16:10", Px(1920, 1200), "Laptops and many monitors"),
    p(Slides, "Slide 4:3", Px(1024, 768), "Older projectors"),
    p(Diagram, "Whiteboard", Px(2400, 1600), "Room to sketch a system: flowcharts, networks, architectures"),
    p(Diagram, "A4 landscape", Px(1123, 794), "A diagram that prints on A4 (96 ppi)"),
    p(Diagram, "Letter landscape", Px(1056, 816), "A diagram that prints on US Letter (96 ppi)"),
    p(Diagram, "Wide", Px(3200, 1800), "Big maps and timelines, for screens"),
    p(Print, "A5", Mm(148.0, 210.0), "Flyers, booklets, notebooks"),
    p(Print, "A4", Mm(210.0, 297.0), "Letters, reports: the standard sheet"),
    p(Print, "A3", Mm(297.0, 420.0), "Posters and drawings: two A4s side by side"),
    p(Print, "US Letter", In(8.5, 11.0), "The standard sheet in the US and Canada"),
    p(Print, "US Legal", In(8.5, 14.0), "Longer sheet for forms"),
    p(Print, "Tabloid", In(11.0, 17.0), "Posters: two Letters side by side"),
    p(Print, "Business card", Mm(85.0, 55.0), "Cards and badges"),
    p(Other, "Icon", Px(512, 512), "App icons and avatars"),
    p(Other, "Desktop wallpaper", Px(2560, 1440), "A 1440p desktop"),
    p(Other, "Phone wallpaper", Px(1170, 2532), "A modern phone screen"),
    p(Other, "Web banner", Px(728, 90), "Leaderboard ad"),
];

/// Where the new document's content comes from.
pub enum Source {
    Blank,
    /// The clipboard's image or SVG, placed on a canvas its size.
    Clipboard(ClipContent),
    /// A one-slide presentation, with the Slides panel open.
    Deck,
    /// A starter deck in theme `THEMES[i]`.
    Template(usize),
}

pub struct NewDialog {
    pub settings: CompSettings,
    preset: Option<usize>,
    /// The tab of sizes shown.
    family: Family,
    /// Paper sizes: landscape rather than portrait.
    landscape: bool,
    /// Paper sizes: pixels per inch (96 for screens, 300 for print).
    print_dpi: f64,
    /// What the clipboard held when the dialog opened (image or SVG only).
    clip: Option<(ClipContent, u32, u32)>,
    use_clip: bool,
    /// Have the assistant build it from `description`.
    ai: bool,
    kind: crate::ai_create::DocKind,
    description: String,
    /// Put the cursor in the description box on the next frame.
    focus_description: bool,
    /// Opened at launch: also offers recent files and the examples.
    pub launch: bool,
    /// A template chosen in the Slides tab (an index into `THEMES`).
    template: Option<usize>,
}

impl NewDialog {
    pub fn new(app: &mut XenifyApp) -> Self {
        let clip = app.platform.clipboard.read().and_then(|c| {
            let (w, h) = clip_size(&c)?;
            Some((c, w, h))
        });
        Self::with_clip(clip)
    }

    /// The dialog Xenify Vector opens with. It doesn't offer the clipboard: what was
    /// copied before launch is rarely what the first document is for.
    pub fn at_launch() -> Self {
        Self { launch: true, ..Self::with_clip(None) }
    }

    fn with_clip(clip: Option<(ClipContent, u32, u32)>) -> Self {
        let mut settings = CompSettings::default();
        let use_clip = clip.is_some();
        if let Some((_, w, h)) = &clip {
            (settings.width, settings.height) = (*w, *h);
        }
        Self {
            settings,
            preset: (!use_clip).then_some(0),
            family: Family::Video,
            landscape: false,
            print_dpi: 300.0,
            clip,
            use_clip,
            ai: false,
            kind: crate::ai_create::DocKind::Animation,
            description: String::new(),
            focus_description: false,
            launch: false,
            template: None,
        }
    }

    /// AI ▸ New From a Description: starts with the description box open
    /// (a blank canvas, not the clipboard).
    pub fn describe_with_ai(&mut self) {
        self.ai = true;
        self.use_clip = false;
        self.focus_description = true;
    }

    /// Picks preset `i` (paper sizes in the chosen orientation and resolution).
    fn choose(&mut self, i: usize) {
        let Some(preset) = CANVAS_PRESETS.get(i) else { return };
        self.preset = Some(i);
        self.use_clip = false;
        let paper = !matches!(preset.dim, Dim::Px(..));
        let dim = if paper { preset.dim.turned(self.landscape) } else { preset.dim };
        (self.settings.width, self.settings.height) = dim.pixels(if paper { self.print_dpi } else { 96.0 });
        // Diagram sizes make a diagram canvas (the diagram tools come with it).
        let kind = if preset.family == Family::Diagram {
            xenify_doc::CompKind::Diagram
        } else {
            xenify_doc::CompKind::Animation
        };
        xenify_ops::diagram::set_kind(&mut self.settings, kind);
        // Paper measures in mm / inches at its resolution; the rest in pixels.
        self.settings.units = match dim {
            Dim::Mm(..) => xenify_doc::Units { unit: xenify_doc::Unit::Mm, dpi: self.print_dpi },
            Dim::In(..) => xenify_doc::Units { unit: xenify_doc::Unit::In, dpi: self.print_dpi },
            Dim::Px(..) => xenify_doc::Units::default(),
        };
    }

    /// Whether the dialog offers (and chose) the clipboard's content.
    pub fn uses_clipboard(&self) -> bool {
        self.use_clip && self.clip.is_some()
    }
}

/// The pixel size of clipboard content that can start a document.
fn clip_size(c: &ClipContent) -> Option<(u32, u32)> {
    match c {
        ClipContent::Image { width, height, .. } => Some((*width, *height)),
        ClipContent::Encoded { bytes } => image_size(bytes),
        ClipContent::Svg(svg) => xenify_export::svg::svg_size(svg.as_bytes()),
        ClipContent::Files(files) => {
            let first = files.first()?;
            image_size(&std::fs::read(first).ok()?)
        }
        ClipContent::Text(_) => None,
    }
}

fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    xenify_export::image::prepare(bytes, u32::MAX).ok().map(|p| (p.width, p.height))
}

/// The tabs of sizes and the cards of the chosen tab.
fn preset_picker(ui: &mut egui::Ui, d: &mut NewDialog) {
    ui.horizontal(|ui| {
        for f in Family::ALL {
            ui.selectable_value(&mut d.family, f, f.label());
        }
    });
    if d.family == Family::Print {
        // Paper: which way up, and for screen or printing.
        let before = (d.landscape, d.print_dpi);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.landscape, false, "Portrait");
            ui.selectable_value(&mut d.landscape, true, "Landscape");
            ui.separator();
            ui.selectable_value(&mut d.print_dpi, 300.0, "For printing (300 ppi)")
                .on_hover_text("Sharp on paper; a big canvas");
            ui.selectable_value(&mut d.print_dpi, 96.0, "For screens (96 ppi)")
                .on_hover_text("PDFs read on screen; a light canvas");
        });
        if before != (d.landscape, d.print_dpi)
            && let Some(i) = d.preset.filter(|i| CANVAS_PRESETS[*i].family == Family::Print)
        {
            d.choose(i);
        }
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        let family = d.family;
        for (i, preset) in CANVAS_PRESETS.iter().enumerate().filter(|(_, p)| p.family == family) {
            let dim = if matches!(preset.dim, Dim::Px(..)) { preset.dim } else { preset.dim.turned(d.landscape) };
            if card(ui, preset, dim, d.preset == Some(i)).on_hover_text(preset.note).clicked() {
                d.choose(i);
            }
        }
    });
}

/// One size as a card: its shape at its real proportions, name and size.
fn card(ui: &mut egui::Ui, preset: &CanvasPreset, dim: Dim, selected: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(104.0, 112.0), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, preset.name));
    let painter = ui.painter_at(rect);
    let v = ui.visuals();
    let bg = if selected {
        crate::widgets::ACCENT.gamma_multiply(0.3)
    } else if r.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        v.widgets.inactive.weak_bg_fill
    };
    painter.rect_filled(rect, 6.0, bg);
    if selected {
        painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.5, crate::widgets::ACCENT), egui::StrokeKind::Inside);
    }
    let area = egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 8.0), egui::vec2(80.0, 62.0));
    thumbnail(&painter, area, preset.family, dim.measure());
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 80.0),
        egui::Align2::CENTER_CENTER,
        preset.name,
        egui::FontId::proportional(12.5),
        v.strong_text_color(),
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 97.0),
        egui::Align2::CENTER_CENTER,
        dim.label(),
        egui::FontId::proportional(11.0),
        v.weak_text_color(),
    );
    r
}

/// The canvas shape (`size`'s proportions, fitted in `area`) with a hint of
/// what goes on it: a play button, a slide's title, a page's text…
fn thumbnail(painter: &egui::Painter, area: egui::Rect, family: Family, size: (f64, f64)) {
    use egui::{Color32, Stroke, pos2, vec2};
    let (w, h) = (size.0.max(1.0) as f32, size.1.max(1.0) as f32);
    let k = (area.width() / w).min(area.height() / h);
    let r = egui::Rect::from_center_size(area.center(), vec2(w * k, h * k));
    let paper = family == Family::Print;
    let fill = if paper { Color32::from_gray(235) } else { Color32::from_gray(38) };
    let ink = if paper { Color32::from_gray(150) } else { Color32::from_gray(120) };
    let corner = if family == Family::Other && (w - h).abs() < 1.0 { r.width() * 0.2 } else { 2.0 };
    painter.rect(r, corner, fill, Stroke::new(1.0, Color32::from_gray(140)), egui::StrokeKind::Middle);
    let line = |y: f32, from: f32, to: f32| {
        painter.line_segment(
            [
                pos2(r.left() + r.width() * from, r.top() + r.height() * y),
                pos2(r.left() + r.width() * to, r.top() + r.height() * y),
            ],
            Stroke::new(1.5, ink),
        )
    };
    match family {
        Family::Video => {
            let c = r.center();
            let s = r.height().min(r.width()) * 0.18;
            painter.add(egui::Shape::convex_polygon(
                vec![c + vec2(-s * 0.7, -s), c + vec2(-s * 0.7, s), c + vec2(s, 0.0)],
                crate::widgets::ACCENT,
                Stroke::NONE,
            ));
        }
        Family::Social => {
            painter.circle_filled(
                r.left_top() + vec2(r.width() * 0.18, r.height() * 0.15),
                r.width().min(r.height()) * 0.07,
                ink,
            );
            line(0.72, 0.12, 0.8);
            line(0.84, 0.12, 0.55);
        }
        Family::Slides => {
            line(0.2, 0.12, 0.6);
            for y in [0.45, 0.6, 0.75] {
                painter.circle_filled(pos2(r.left() + r.width() * 0.15, r.top() + r.height() * y), 1.5, ink);
                line(y, 0.22, 0.7);
            }
        }
        Family::Diagram => {
            // Two boxes joined by an elbow line.
            let (w, h) = (r.width(), r.height());
            let a = egui::Rect::from_min_size(r.left_top() + vec2(w * 0.12, h * 0.2), vec2(w * 0.3, h * 0.22));
            let b = egui::Rect::from_min_size(r.left_top() + vec2(w * 0.58, h * 0.58), vec2(w * 0.3, h * 0.22));
            for bx in [a, b] {
                painter.rect_stroke(bx, 2.0, Stroke::new(1.5, ink), egui::StrokeKind::Middle);
            }
            let mid = pos2(b.center().x, a.center().y);
            painter.line_segment([pos2(a.right(), a.center().y), mid], Stroke::new(1.5, crate::widgets::ACCENT));
            painter.line_segment([mid, pos2(b.center().x, b.top())], Stroke::new(1.5, crate::widgets::ACCENT));
        }
        Family::Print => {
            // A folded corner and lines of text.
            let f = r.width().min(r.height()) * 0.18;
            painter.add(egui::Shape::convex_polygon(
                vec![r.right_top() + vec2(-f, 0.0), r.right_top() + vec2(0.0, f), r.right_top() + vec2(-f, f)],
                Color32::from_gray(190),
                Stroke::NONE,
            ));
            let rows = ((r.height() / 7.0) as usize).clamp(2, 7);
            for i in 0..rows {
                let y = 0.22 + 0.62 * i as f32 / rows.max(1) as f32;
                line(y, 0.14, if i + 1 == rows { 0.5 } else { 0.84 });
            }
        }
        Family::Other => {}
    }
}

/// The chosen size drawn to scale among the rest of its family (A5 in A4 in
/// A3; 720p in 1080p in 4K…), and what it is.
fn preview(ui: &mut egui::Ui, d: &NewDialog) {
    let preset = d.preset.map(|i| &CANVAS_PRESETS[i]);
    let paper = preset.is_some_and(|p| !matches!(p.dim, Dim::Px(..)));
    let chosen = match preset {
        Some(p) if paper => p.dim.turned(d.landscape),
        Some(p) => p.dim,
        None => Dim::Px(d.settings.width, d.settings.height),
    };
    let title = match preset {
        Some(p) => p.name.to_string(),
        None if d.uses_clipboard() => "From the clipboard".into(),
        None => "Custom size".into(),
    };
    ui.label(egui::RichText::new(title).strong().size(16.0));
    if let Some(p) = preset {
        ui.weak(p.note);
    }
    ui.add_space(4.0);
    // Everyone of the family, at one scale, standing on the same corner.
    let family = preset.map_or(Family::Video, |p| p.family);
    let mut others: Vec<(&str, Dim)> = CANVAS_PRESETS
        .iter()
        .filter(|p| p.family == family && Some(p.name) != preset.map(|c| c.name))
        .filter(|p| paper || family != Family::Social || p.dim.measure().1 >= 500.0)
        .map(|p| (p.name, if paper { p.dim.turned(d.landscape) } else { p.dim }))
        .collect();
    if family == Family::Print {
        // Compare within a series: A5 / A4 / A3 (and cards against them), or
        // US Letter / Legal / Tabloid. Mixing them only clutters the picture.
        let us = |n: &str| n.starts_with("US") || n == "Tabloid";
        let chosen_us = preset.is_some_and(|p| us(p.name));
        others.retain(|(n, _)| us(n) == chosen_us && *n != "Business card");
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(250.0, 190.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let all: Vec<(f64, f64)> = others.iter().map(|o| o.1.measure()).chain([chosen.measure()]).collect();
    let (mw, mh) = all.iter().fold((1.0f64, 1.0f64), |(a, b), (w, h)| (a.max(*w), b.max(*h)));
    let inner = rect.shrink(12.0);
    let k = (inner.width() as f64 / mw).min(inner.height() as f64 / mh);
    let place = |dim: Dim| {
        let (w, h) = dim.measure();
        egui::Rect::from_min_size(
            egui::pos2(inner.left(), inner.bottom() - (h * k) as f32),
            egui::vec2((w * k) as f32, (h * k) as f32),
        )
    };
    let grey = egui::Color32::from_gray(110);
    // Biggest first, so smaller outlines sit on top.
    others.sort_by(|a, b| {
        let (x, y) = (a.1.measure(), b.1.measure());
        (y.0 * y.1).total_cmp(&(x.0 * x.1))
    });
    for (name, dim) in &others {
        let r = place(*dim);
        painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, grey), egui::StrokeKind::Middle);
        painter.text(
            r.right_top() + egui::vec2(-3.0, 3.0),
            egui::Align2::RIGHT_TOP,
            *name,
            egui::FontId::proportional(10.0),
            grey,
        );
    }
    let r = place(chosen);
    painter.rect(
        r,
        1.0,
        crate::widgets::ACCENT.gamma_multiply(0.35),
        egui::Stroke::new(2.0, crate::widgets::ACCENT),
        egui::StrokeKind::Middle,
    );
    // Its own name, just above it (inside when it reaches the top).
    let label = preset.map_or("This canvas", |p| p.name);
    let (at, align) = if r.top() - 14.0 > rect.top() {
        (r.left_top() + egui::vec2(2.0, -2.0), egui::Align2::LEFT_BOTTOM)
    } else {
        (r.left_top() + egui::vec2(4.0, 3.0), egui::Align2::LEFT_TOP)
    };
    painter.text(at, align, label, egui::FontId::proportional(11.0), egui::Color32::WHITE);
    ui.add_space(6.0);
    // The facts.
    let (w, h) = (d.settings.width, d.settings.height);
    egui::Grid::new("new-facts").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
        if paper {
            ui.weak("Paper");
            ui.label(chosen.label());
            ui.end_row();
        }
        ui.weak("Pixels");
        ui.label(if paper { format!("{w} × {h} at {} ppi", d.print_dpi) } else { format!("{w} × {h}") });
        ui.end_row();
        ui.weak("Shape");
        ui.label(aspect(w, h));
        ui.end_row();
        if !paper {
            let (mp, sec) = (w as f64 * h as f64 / 1e6, d.settings.duration);
            ui.weak("Length");
            ui.label(format!("{sec} s at {} fps ({mp:.1} MP a frame)", d.settings.fps));
            ui.end_row();
        }
    });
}

/// "16:9 landscape", "1 : 1.41 portrait", "square".
fn aspect(w: u32, h: u32) -> String {
    if w == h {
        return "Square".into();
    }
    let g = gcd(w, h).max(1);
    let (a, b) = (w / g, h / g);
    let ratio = if a <= 32 && b <= 32 {
        format!("{a}:{b}")
    } else {
        let r = w.max(h) as f64 / w.min(h) as f64;
        if w > h { format!("{r:.2} : 1") } else { format!("1 : {r:.2}") }
    };
    format!("{ratio} {}", if w > h { "landscape" } else { "portrait" })
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Draws the dialog; returns true when it should close.
/// Width of the launch column (Open, Recent, Examples).
pub const LAUNCH_COLUMN: f32 = 190.0;

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, d: &mut NewDialog) -> bool {
    if !d.launch {
        return new_document(app, ui, d);
    }
    let mut close = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LAUNCH_COLUMN - 14.0);
            close |= launch_column(app, ui);
        });
        ui.add_space(14.0);
        ui.vertical(|ui| close |= new_document(app, ui, d));
    });
    close
}

/// At launch, beside the new-document choices: open something that
/// already exists, or look at an example. Returns whether the dialog is done.
fn launch_column(app: &mut XenifyApp, ui: &mut egui::Ui) -> bool {
    use crate::session::{Pending, guard};
    let mut close = false;
    ui.label(egui::RichText::new("Open").strong());
    if ui
        .button("Open a File…")
        .on_hover_text(
            app.keymap.tip(crate::keymap::Action::Open, "Open a project, or import a picture, SVG or 3D model"),
        )
        .clicked()
    {
        let ctx = ui.ctx().clone();
        crate::actions::run(app, &ctx, crate::keymap::Action::Open);
        close = true;
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Recent").strong());
    let recent: Vec<_> = app.session.recent.iter().take(8).cloned().collect();
    if recent.is_empty() {
        ui.weak("Files you open or save show up here.");
    }
    for path in recent {
        let name = path.file_stem().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into());
        let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        if ui.add(egui::Button::new(name).truncate()).on_hover_text(folder).clicked() {
            guard(app, Pending::OpenPath(path));
            close = true;
        }
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Examples").strong());
    if ui
        .button("Showcase")
        .on_hover_text("A presentation of everything the app does, made in the app (F5 presents it)")
        .clicked()
    {
        crate::showcase::open(app);
        close = true;
    }
    if ui
        .button("Bouncing ball")
        .on_hover_text("A short animation to look around in: keyframes, easing and a text layer")
        .clicked()
    {
        app.open_document(xenify_doc::demo_project(), None);
        close = true;
    }
    close
}

fn new_document(app: &mut XenifyApp, ui: &mut egui::Ui, d: &mut NewDialog) -> bool {
    if let Some((_, w, h)) = &d.clip {
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.use_clip, true, format!("From clipboard ({w}×{h})"))
                .on_hover_text("A canvas the size of what you copied, with it placed on it");
            ui.radio_value(&mut d.use_clip, false, "Blank");
        });
        if d.use_clip {
            (d.settings.width, d.settings.height) = (*w, *h);
            d.preset = None;
        }
        ui.add_space(4.0);
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(560.0);
            ui.add_enabled_ui(!d.uses_clipboard(), |ui| preset_picker(ui, d));
            if d.family == Family::Slides && !d.uses_clipboard() {
                ui.add_space(6.0);
                if crate::templates_ui::gallery(app, ui, &mut d.template)
                    && d.preset.is_none_or(|i| CANVAS_PRESETS[i].family != Family::Slides)
                    && let Some(i) = CANVAS_PRESETS.iter().position(|p| p.family == Family::Slides)
                {
                    d.choose(i);
                }
            }
        });
        // Space, not a separator: a vertical separator takes the whole height
        // on offer, which would grow the dialog every frame.
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.set_width(250.0);
            preview(ui, d);
        });
    });
    ui.add_space(6.0);
    egui::Grid::new("new-doc").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut d.settings.name);
        ui.end_row();
        ui.label("Size");
        ui.horizontal(|ui| {
            let w = ui.add(egui::DragValue::new(&mut d.settings.width).range(1..=16384).suffix(" px"));
            ui.label("×");
            let h = ui.add(egui::DragValue::new(&mut d.settings.height).range(1..=16384).suffix(" px"));
            if w.changed() || h.changed() {
                d.preset = None;
                d.use_clip = false;
            }
            if ui.small_button("Turn").on_hover_text("Swap width and height (portrait ↔ landscape)").clicked() {
                let s = &mut d.settings;
                (s.width, s.height) = (s.height, s.width);
                d.landscape = s.width > s.height;
                if d.preset.is_some_and(|i| matches!(CANVAS_PRESETS[i].dim, Dim::Px(..))) {
                    d.preset = None;
                }
            }
        });
        ui.end_row();
        crate::dialogs::kind_row(ui, &mut d.settings);
        crate::dialogs::units_rows(ui, &mut d.settings);
        ui.label("Frame rate");
        ui.add(egui::DragValue::new(&mut d.settings.fps).range(1.0..=240.0).suffix(" fps"));
        ui.end_row();
        ui.label("Duration");
        ui.add(egui::DragValue::new(&mut d.settings.duration).range(0.1..=3600.0).speed(0.1).suffix(" s"));
        ui.end_row();
        ui.label("Background");
        crate::color_picker::button(ui, "new-background", &mut d.settings.background);
        ui.end_row();
    });
    ui.add_space(4.0);
    if ui
        .checkbox(&mut d.ai, "Build it with AI from a description")
        .on_hover_text("The assistant creates the animation or slide deck in the new tab")
        .changed()
        && d.ai
    {
        d.use_clip = false;
        d.focus_description = true;
    }
    if d.ai {
        use crate::ai_create::DocKind;
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.kind, DocKind::Animation, "Animation / scene");
            if ui.radio_value(&mut d.kind, DocKind::Deck, "Slide deck").clicked() && d.settings.name.is_empty() {
                d.settings.name = "Slide 1".into();
            }
        });
        let hint = match d.kind {
            DocKind::Animation => "e.g. “A 6-second logo reveal for a coffee shop called Ember, warm colours”",
            DocKind::Deck => "e.g. “A 6-slide pitch for a community garden: problem, plan, budget, timeline, ask”",
        };
        let r = ui.add(
            egui::TextEdit::multiline(&mut d.description).hint_text(hint).desired_rows(4).desired_width(f32::INFINITY),
        );
        // The cursor goes to the description whenever nothing else has it.
        if d.focus_description && ui.ctx().memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if d.uses_clipboard() {
            ui.weak("The AI builds on a blank canvas: the clipboard isn't used.");
        }
    }
    ui.separator();
    let mut close = false;
    ui.horizontal(|ui| {
        // In the description box Enter starts a new line; Ctrl+Enter creates.
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && (!d.ai || i.modifiers.command));
        // With "Build it with AI" ticked, there must be something to build.
        let ready = !d.ai || !d.description.trim().is_empty();
        let create_button = ui
            .add_enabled(ready, crate::widgets::primary("Create"))
            .on_disabled_hover_text("Describe what to build first (or untick “Build it with AI”)");
        if create_button.clicked() || (enter && ready) {
            let ai = d.ai && !d.description.trim().is_empty();
            // No key: stay here (Settings › AI opens) rather than make an empty document.
            if ai && !crate::assistant::ai_ready(app) {
                return;
            }
            let slides = d.preset.is_some_and(|i| CANVAS_PRESETS[i].family == Family::Slides);
            let source = if d.uses_clipboard() && !ai {
                Source::Clipboard(d.clip.take().expect("checked").0)
            } else if slides
                && !ai
                && let Some(t) = d.template
            {
                Source::Template(t)
            } else if slides && !ai {
                Source::Deck
            } else {
                Source::Blank
            };
            create(app, d.settings.clone(), source);
            if ai {
                let ctx = ui.ctx().clone();
                crate::ai_create::start_new_document(app, &ctx, d.kind, &d.description);
            }
            close = true;
        }
        close |= ui.button("Cancel").clicked();
    });
    close
}

/// Opens a new document with `settings`, filled from `source`. The hook for
/// "New from a description" later: it fills the returned tab asynchronously.
pub fn create(app: &mut XenifyApp, settings: CompSettings, source: Source) -> crate::documents::DocId {
    let mut project = match source {
        Source::Template(t) => {
            let theme = &xenify_doc::THEMES[t.min(xenify_doc::THEMES.len() - 1)];
            xenify_ops::templates::new_deck(theme, settings.width, settings.height, &xenify_ops::templates::STARTER)
        }
        _ => {
            let mut p = Project::new();
            p.compositions[0].settings = settings;
            p
        }
    };
    if let Source::Template(t) = source {
        let id = app.open_document(project, None);
        crate::dock::reveal(app, crate::dock::Panel::Slides);
        app.status = format!(
            "New {} presentation: five starter slides. Click a text to change it; Slide › New Slide From Layout adds more.",
            xenify_doc::THEMES[t.min(xenify_doc::THEMES.len() - 1)].name
        );
        return id;
    }
    if matches!(source, Source::Deck) {
        project.slides = vec![xenify_doc::Slide::new(project.compositions[0].id)];
    }
    let id = app.open_document(project, None);
    if matches!(source, Source::Deck) {
        crate::dock::reveal(app, crate::dock::Panel::Slides);
        let key = app.keymap.text(crate::keymap::Action::NewSlide);
        app.status = format!("New presentation. {key} adds a slide; F5 presents.");
        return id;
    }
    if let Source::Clipboard(content) = source {
        let (w, h) = (app.comp().settings.width as f64, app.comp().settings.height as f64);
        let centre = Some(kurbo::Point::new(w / 2.0, h / 2.0));
        match content {
            ClipContent::Svg(svg) => app.import_svg_document(svg.as_bytes(), "Pasted SVG"),
            ClipContent::Image { width, height, rgba } => {
                if let Ok(img) = xenify_export::image::from_rgba(width, height, rgba, u32::MAX) {
                    app.add_image(&img.data, "Pasted image", centre);
                }
            }
            ClipContent::Encoded { bytes } => {
                app.add_image(&bytes, "Pasted image", centre);
            }
            ClipContent::Files(files) => app.import_files(files.into_iter().take(1).collect(), centre),
            ClipContent::Text(_) => {}
        }
        // Starting content isn't an unsaved edit to undo.
        app.editor.mark_saved();
    }
    app.status = "New document".into();
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_sensible() {
        for p in CANVAS_PRESETS {
            let (w, h) = p.dim.pixels(300.0);
            assert!(w >= 16 && h >= 16 && w <= 16384 && h <= 16384, "{}", p.name);
        }
        // A4 at 300 ppi is the familiar 2480 × 3508; turned, it's landscape.
        let a4 = Dim::Mm(210.0, 297.0);
        assert_eq!(a4.pixels(300.0), (2480, 3508));
        assert_eq!(a4.turned(true).pixels(96.0), (1123, 794));
        assert_eq!(aspect(1920, 1080), "16:9 landscape");
        assert_eq!(aspect(2480, 3508), "1 : 1.41 portrait");
        assert_eq!(clip_size(&ClipContent::Image { width: 300, height: 200, rgba: vec![] }), Some((300, 200)));
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40"/>"#;
        assert_eq!(clip_size(&ClipContent::Svg(svg.into())), Some((120, 40)));
        assert_eq!(clip_size(&ClipContent::Text("hi".into())), None);
    }
}
