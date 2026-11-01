//! Slide templates: themes (background, a palette by role, type and a
//! default transition) and layouts (Title, Bullets, Big Stat, …) that build
//! a slide's layers with entrance animations. Layers remember their role
//! ([`crate::LayerMeta::role`]), so a theme can be applied to a finished deck
//! and recolour it.

use crate::{
    Color, Easing, Fill, GradientKind, GradientPaint, GradientStop, Id, Keyframe, Layer, LayerKind, Property,
    ShapeItem, TextAnimator, TextFrame, TextLayer, Transition, TransitionKind, Vec2,
};
use serde::{Deserialize, Serialize};

/// What a layer is for on a slide, so themes know how to colour it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The full-frame background.
    Background,
    Title,
    Subtitle,
    Body,
    /// Highlights: bars, numbers, big stats.
    Accent,
    /// Secondary highlights (lines, second series).
    Accent2,
    /// Placeholder and card panels.
    Panel,
    /// Text drawn on an accent shape.
    OnAccent,
    /// Quiet text (placeholder hints, captions).
    Muted,
}

impl Role {
    pub const ALL: [Role; 9] = [
        Role::Background,
        Role::Title,
        Role::Subtitle,
        Role::Body,
        Role::Accent,
        Role::Accent2,
        Role::Panel,
        Role::OnAccent,
        Role::Muted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Role::Background => "Background",
            Role::Title => "Title",
            Role::Subtitle => "Subtitle",
            Role::Body => "Body",
            Role::Accent => "Accent",
            Role::Accent2 => "Accent 2",
            Role::Panel => "Panel",
            Role::OnAccent => "On Accent",
            Role::Muted => "Muted",
        }
    }
}

/// A look for a deck.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    /// Top and bottom of the background (the same colour for a flat one).
    pub background: (Color, Color),
    pub title: Color,
    pub subtitle: Color,
    pub body: Color,
    pub accent: Color,
    pub accent2: Color,
    pub panel: Color,
    pub muted: Color,
    pub font: &'static str,
    /// Letter spacing of titles, as a fraction of their size.
    pub title_tracking: f64,
    /// Corner radius of panels and bars, as a fraction of the slide height.
    pub radius: f64,
    /// How slides arrive.
    pub transition: TransitionKind,
    pub transition_seconds: f64,
}

const fn hex(v: u32) -> Color {
    Color::rgb(((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0)
}

/// The built-in themes.
pub const THEMES: [Theme; 5] = [
    Theme {
        name: "Dark Neon",
        background: (hex(0x0B0B1A), hex(0x1A0B2E)),
        title: hex(0xFFFFFF),
        subtitle: hex(0x9EF0FF),
        body: hex(0xD8D8F0),
        accent: hex(0xFF2E97),
        accent2: hex(0x00E5FF),
        panel: hex(0x22223A),
        muted: hex(0x7A7A9A),
        font: "Inter",
        title_tracking: -0.02,
        radius: 0.02,
        transition: TransitionKind::Push,
        transition_seconds: 0.7,
    },
    Theme {
        name: "Clean Light",
        background: (hex(0xFFFFFF), hex(0xF3F5F8)),
        title: hex(0x14171F),
        subtitle: hex(0x4A5568),
        body: hex(0x2D3748),
        accent: hex(0x2F6FEB),
        accent2: hex(0x16A34A),
        panel: hex(0xE8ECF2),
        muted: hex(0x8A94A6),
        font: "Inter",
        title_tracking: -0.01,
        radius: 0.015,
        transition: TransitionKind::Fade,
        transition_seconds: 0.6,
    },
    Theme {
        name: "Bold Gradient",
        background: (hex(0x6A11CB), hex(0x2575FC)),
        title: hex(0xFFFFFF),
        subtitle: hex(0xFFE3F7),
        body: hex(0xF4F0FF),
        accent: hex(0xFFD23F),
        accent2: hex(0xFF6B9D),
        panel: Color::rgba(1.0, 1.0, 1.0, 0.16),
        muted: Color::rgba(1.0, 1.0, 1.0, 0.7),
        font: "Inter",
        title_tracking: -0.03,
        radius: 0.03,
        transition: TransitionKind::ZoomIn,
        transition_seconds: 0.8,
    },
    Theme {
        name: "Corporate",
        background: (hex(0xF7F8FA), hex(0xF7F8FA)),
        title: hex(0x0A2540),
        subtitle: hex(0x425466),
        body: hex(0x1F2D3D),
        accent: hex(0x0A6CFF),
        accent2: hex(0x00B894),
        panel: hex(0xE3E8EE),
        muted: hex(0x8792A2),
        font: "Inter",
        title_tracking: 0.0,
        radius: 0.006,
        transition: TransitionKind::Wipe,
        transition_seconds: 0.6,
    },
    Theme {
        name: "Retro",
        background: (hex(0xF4E4C1), hex(0xEAD2A0)),
        title: hex(0x3B1F0E),
        subtitle: hex(0x8C3B1E),
        body: hex(0x4A2C17),
        accent: hex(0xE85D24),
        accent2: hex(0x2A9D8F),
        panel: hex(0xE6C88F),
        muted: hex(0x9C7A55),
        font: "Inter",
        title_tracking: 0.04,
        radius: 0.04,
        transition: TransitionKind::Iris,
        transition_seconds: 0.8,
    },
];

/// A built-in theme by name (any case).
pub fn theme(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.name.eq_ignore_ascii_case(name.trim()))
}

impl Theme {
    pub fn transition(&self) -> Transition {
        Transition { color: self.background.0, ..Transition::new(self.transition, self.transition_seconds) }
    }

    /// The colour a role uses.
    pub fn color(&self, role: Role) -> Color {
        match role {
            Role::Background => self.background.0,
            Role::Title => self.title,
            Role::Subtitle => self.subtitle,
            Role::Body => self.body,
            Role::Accent => self.accent,
            Role::Accent2 => self.accent2,
            Role::Panel => self.panel,
            Role::OnAccent => self.background.0,
            Role::Muted => self.muted,
        }
    }
}

/// The built-in slide layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Title,
    Section,
    Bullets,
    TwoColumn,
    ImageCaption,
    FullBleedVideo,
    Quote,
    BigStat,
    Process,
    Comparison,
    Closing,
}

/// What goes on a slide. Each layout reads what it needs:
/// - Title, Closing: `title`, `subtitle`, and `items[0]` (a contact line, Closing).
/// - Section: `title`, `subtitle` (the section number, e.g. "02").
/// - Bullets: `title`, `items` (one bullet each).
/// - Two Column, Comparison: `title`, `items[0]` and `items[1]` (first line
///   the heading, the rest its lines).
/// - Image + Caption, Full-bleed Video: `title` (caption), `subtitle`.
/// - Quote: `title` (the quote), `subtitle` (who said it).
/// - Big Stat: `title` (the number), `subtitle` (what it means).
/// - Process: `title`, `items` (the steps).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Content {
    pub title: String,
    pub subtitle: String,
    pub items: Vec<String>,
}

impl Layout {
    pub const ALL: [Layout; 11] = [
        Layout::Title,
        Layout::Section,
        Layout::Bullets,
        Layout::TwoColumn,
        Layout::ImageCaption,
        Layout::FullBleedVideo,
        Layout::Quote,
        Layout::BigStat,
        Layout::Process,
        Layout::Comparison,
        Layout::Closing,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Layout::Title => "Title",
            Layout::Section => "Section",
            Layout::Bullets => "Bullets",
            Layout::TwoColumn => "Two Column",
            Layout::ImageCaption => "Image + Caption",
            Layout::FullBleedVideo => "Full-bleed Video",
            Layout::Quote => "Quote",
            Layout::BigStat => "Big Stat",
            Layout::Process => "Process",
            Layout::Comparison => "Comparison",
            Layout::Closing => "Closing",
        }
    }

    /// By label or id (`"two column"`, `"two_column"`, `"Big Stat"`).
    pub fn from_name(name: &str) -> Option<Layout> {
        let key = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let k = key(name);
        Layout::ALL.into_iter().find(|l| key(l.label()) == k || (k == "imagecaption" && *l == Layout::ImageCaption))
    }

    /// Example words, used when a slide is made without content.
    pub fn sample(self) -> Content {
        let c = |title: &str, subtitle: &str, items: &[&str]| Content {
            title: title.into(),
            subtitle: subtitle.into(),
            items: items.iter().map(|s| s.to_string()).collect(),
        };
        match self {
            Layout::Title => c("Your Big Idea", "A subtitle that sets the scene", &[]),
            Layout::Section => c("The Next Chapter", "02", &[]),
            Layout::Bullets => {
                c("Three Things to Know", "", &["It starts simple", "It builds on itself", "It ends with a bang"])
            }
            Layout::TwoColumn => c(
                "Two Sides",
                "",
                &[
                    "What we have\nA working prototype\nHappy early users",
                    "What we need\nA bigger team\nSix more months",
                ],
            ),
            Layout::ImageCaption => c("A picture of it working", "Replace the panel with your image", &[]),
            Layout::FullBleedVideo => c("See it in motion", "Replace the panel with your video", &[]),
            Layout::Quote => c("Simplicity is the ultimate sophistication.", "— Leonardo da Vinci", &[]),
            Layout::BigStat => c("87%", "of people remember a story, not a statistic", &[]),
            Layout::Process => c("How It Works", "", &["Plan", "Make", "Share", "Grow"]),
            Layout::Comparison => {
                c("Before and After", "", &["Before\nSlow\nManual\nError-prone", "After\nInstant\nAutomatic\nReliable"])
            }
            Layout::Closing => c("Thank You", "Questions?", &["hello@example.com"]),
        }
    }
}

/// A slide built from a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltSlide {
    /// Bottom-most first.
    pub layers: Vec<Layer>,
    /// Where the presentation waits for a click (e.g. before each bullet).
    pub pauses: Vec<f64>,
    /// How long the slide's animation runs (seconds).
    pub duration: f64,
    pub transition: Transition,
}

/// Builds `layout` for a `w`×`h` slide in `theme`, with `content` (the
/// layout's sample words where empty). Ids start at `first_id`.
pub fn build(layout: Layout, theme: &Theme, w: f64, h: f64, first_id: u64, content: &Content) -> BuiltSlide {
    let sample = layout.sample();
    let pick = |a: &str, b: &str| if a.trim().is_empty() { b.to_string() } else { a.to_string() };
    let title = pick(&content.title, &sample.title);
    let subtitle = if content.title.trim().is_empty() && content.subtitle.trim().is_empty() {
        sample.subtitle.clone()
    } else {
        content.subtitle.clone()
    };
    let items =
        if content.items.iter().all(|i| i.trim().is_empty()) { sample.items.clone() } else { content.items.clone() };
    let mut b = Builder { theme, w, h, s: h / 1080.0, next: first_id, layers: Vec::new() };
    b.background();
    let mut pauses = Vec::new();
    let mut end = 1.5;
    let x0 = w * 0.08;
    match layout {
        Layout::Title => {
            b.text(
                Role::Title,
                "Title",
                &title,
                112.0,
                Vec2::new(w / 2.0, h * 0.44),
                Anchor::Center,
                0.0,
                "Fade Up by Word",
            );
            b.bar(Role::Accent, Vec2::new(w / 2.0, h * 0.56), Vec2::new(w * 0.08, h * 0.008), 0.5);
            if !subtitle.is_empty() {
                b.text(
                    Role::Subtitle,
                    "Subtitle",
                    &subtitle,
                    44.0,
                    Vec2::new(w / 2.0, h * 0.66),
                    Anchor::Center,
                    0.6,
                    "Blur In",
                );
            }
        }
        Layout::Section => {
            b.bar(Role::Accent, Vec2::new(w * 0.07, h * 0.5), Vec2::new(h * 0.012, h * 0.3), 0.0);
            if !subtitle.is_empty() {
                b.text(
                    Role::Accent,
                    "Number",
                    &subtitle,
                    64.0,
                    Vec2::new(w * 0.1, h * 0.43),
                    Anchor::Left,
                    0.2,
                    "Tracking In",
                );
            }
            b.text(
                Role::Title,
                "Title",
                &title,
                104.0,
                Vec2::new(w * 0.1, h * 0.6),
                Anchor::Left,
                0.4,
                "Slide by Line",
            );
        }
        Layout::Bullets => {
            b.heading(&title);
            for (k, item) in items.iter().enumerate() {
                let t = 0.8 + k as f64;
                let y = h * 0.4 + k as f64 * h * 0.14;
                b.bar(Role::Accent, Vec2::new(x0 + 10.0 * b.s, y - 18.0 * b.s), Vec2::new(18.0 * b.s, 18.0 * b.s), t);
                let id = b.text(
                    Role::Body,
                    &format!("Bullet {}", k + 1),
                    item,
                    58.0,
                    Vec2::new(x0 + 48.0 * b.s, y),
                    Anchor::Left,
                    0.0,
                    "Fade Up by Word",
                );
                b.starts_at(id, t);
                if k > 0 {
                    // Wait for a click before each bullet after the first.
                    pauses.push(t);
                }
                end = t + 1.2;
            }
        }
        Layout::TwoColumn | Layout::Comparison => {
            b.heading(&title);
            let comparison = layout == Layout::Comparison;
            for (k, col) in items.iter().take(2).enumerate() {
                let x = if k == 0 { x0 } else { w * 0.53 };
                let cw = w * 0.39;
                let mut lines = col.lines();
                let head = lines.next().unwrap_or("");
                let rest: Vec<&str> = lines.collect();
                let t = 0.4 + k as f64 * 0.5;
                if comparison {
                    let panel =
                        b.panel(Role::Panel, "Card", Vec2::new(x + cw / 2.0, h * 0.61), Vec2::new(cw, h * 0.5), t);
                    let _ = panel;
                }
                let pad = if comparison { 36.0 * b.s } else { 0.0 };
                let hid = b.text(
                    Role::Accent,
                    &format!("Heading {}", k + 1),
                    head,
                    56.0,
                    Vec2::new(x + pad, h * 0.44),
                    Anchor::Left,
                    t,
                    "Fade Up by Word",
                );
                let _ = hid;
                let body = rest
                    .iter()
                    .map(|l| if comparison { format!("• {l}") } else { l.to_string() })
                    .collect::<Vec<_>>()
                    .join("\n");
                let id = b.boxed_text(
                    Role::Body,
                    &format!("Column {}", k + 1),
                    &body,
                    44.0,
                    kurbo::Rect::new(x + pad, h * 0.5, x + cw - pad, h * 0.84),
                    t + 0.3,
                    "Slide by Line",
                );
                let _ = id;
            }
            end = 2.4;
        }
        Layout::ImageCaption => {
            b.panel(Role::Panel, "Image placeholder", Vec2::new(w * 0.34, h * 0.5), Vec2::new(w * 0.56, h * 0.74), 0.0);
            b.text(
                Role::Muted,
                "Placeholder hint",
                "Drop an image here",
                40.0,
                Vec2::new(w * 0.34, h * 0.5),
                Anchor::Center,
                0.0,
                "",
            );
            b.boxed_text(
                Role::Title,
                "Caption",
                &title,
                60.0,
                kurbo::Rect::new(w * 0.66, h * 0.32, w * 0.94, h * 0.62),
                0.3,
                "Fade Up by Word",
            );
            if !subtitle.is_empty() {
                b.boxed_text(
                    Role::Subtitle,
                    "Subtitle",
                    &subtitle,
                    40.0,
                    kurbo::Rect::new(w * 0.66, h * 0.64, w * 0.94, h * 0.84),
                    0.7,
                    "Blur In",
                );
            }
        }
        Layout::FullBleedVideo => {
            b.panel(Role::Panel, "Video placeholder", Vec2::new(w / 2.0, h / 2.0), Vec2::new(w, h), 0.0);
            b.text(
                Role::Muted,
                "Placeholder hint",
                "Drop a video here",
                48.0,
                Vec2::new(w / 2.0, h * 0.45),
                Anchor::Center,
                0.0,
                "",
            );
            b.text(Role::Title, "Caption", &title, 64.0, Vec2::new(x0, h * 0.84), Anchor::Left, 0.5, "Slide by Line");
            if !subtitle.is_empty() {
                b.text(
                    Role::Subtitle,
                    "Subtitle",
                    &subtitle,
                    34.0,
                    Vec2::new(x0, h * 0.91),
                    Anchor::Left,
                    0.8,
                    "Blur In",
                );
            }
        }
        Layout::Quote => {
            b.text(
                Role::Accent,
                "Quote mark",
                "\u{201C}",
                300.0,
                Vec2::new(w * 0.12, h * 0.42),
                Anchor::Left,
                0.0,
                "Drop In",
            );
            b.boxed_text(
                Role::Title,
                "Quote",
                &title,
                68.0,
                kurbo::Rect::new(w * 0.2, h * 0.3, w * 0.86, h * 0.66),
                0.3,
                "Fade Up by Word",
            );
            if !subtitle.is_empty() {
                b.text(
                    Role::Subtitle,
                    "Attribution",
                    &subtitle,
                    38.0,
                    Vec2::new(w * 0.2, h * 0.76),
                    Anchor::Left,
                    1.4,
                    "Blur In",
                );
            }
            end = 2.5;
        }
        Layout::BigStat => {
            b.text(
                Role::Accent,
                "Stat",
                &title,
                280.0,
                Vec2::new(w / 2.0, h * 0.46),
                Anchor::Center,
                0.0,
                "Pop by Letter",
            );
            if !subtitle.is_empty() {
                b.text(
                    Role::Subtitle,
                    "Label",
                    &subtitle,
                    48.0,
                    Vec2::new(w / 2.0, h * 0.72),
                    Anchor::Center,
                    0.7,
                    "Fade Up by Word",
                );
            }
        }
        Layout::Process => {
            b.heading(&title);
            let n = items.len().clamp(1, 6);
            let span = w * 0.76;
            let y = h * 0.56;
            let r = (h * 0.075).min(span / n as f64 * 0.3);
            b.line(Role::Accent2, Vec2::new(w * 0.12, y), Vec2::new(w * 0.88, y), 0.2);
            for (k, step) in items.iter().take(n).enumerate() {
                let x = if n == 1 { w / 2.0 } else { w * 0.12 + span * k as f64 / (n - 1) as f64 };
                let t = 0.4 + k as f64 * 0.35;
                b.dot(Role::Accent, &format!("Step {} dot", k + 1), Vec2::new(x, y), r, t);
                b.text(
                    Role::OnAccent,
                    &format!("Step {} number", k + 1),
                    &format!("{}", k + 1),
                    48.0,
                    Vec2::new(x, y),
                    Anchor::Center,
                    t,
                    "",
                );
                b.text(
                    Role::Body,
                    &format!("Step {}", k + 1),
                    step,
                    46.0,
                    Vec2::new(x, y + r + 70.0 * b.s),
                    Anchor::Center,
                    t + 0.15,
                    "Fade Up by Word",
                );
                end = t + 1.2;
            }
        }
        Layout::Closing => {
            b.text(
                Role::Title,
                "Title",
                &title,
                128.0,
                Vec2::new(w / 2.0, h * 0.42),
                Anchor::Center,
                0.0,
                "Pop by Letter",
            );
            if !subtitle.is_empty() {
                b.text(
                    Role::Subtitle,
                    "Call to action",
                    &subtitle,
                    52.0,
                    Vec2::new(w / 2.0, h * 0.58),
                    Anchor::Center,
                    0.6,
                    "Fade Up by Word",
                );
            }
            if let Some(contact) = items.first().filter(|c| !c.trim().is_empty()) {
                b.text(
                    Role::Accent,
                    "Contact",
                    contact,
                    36.0,
                    Vec2::new(w / 2.0, h * 0.7),
                    Anchor::Center,
                    1.0,
                    "Typewriter",
                );
            }
            end = 2.2;
        }
    }
    BuiltSlide { layers: b.layers, pauses, duration: end.max(3.0), transition: theme.transition() }
}

#[derive(Clone, Copy, PartialEq)]
enum Anchor {
    Left,
    Center,
}

struct Builder<'a> {
    theme: &'a Theme,
    w: f64,
    h: f64,
    /// Scale from a 1080-tall slide.
    s: f64,
    next: u64,
    layers: Vec<Layer>,
}

impl Builder<'_> {
    fn push(&mut self, mut layer: Layer, role: Role) -> Id {
        layer.meta.role = Some(role);
        restyle(&mut layer, self.theme);
        let id = layer.id;
        self.layers.push(layer);
        id
    }

    fn id(&mut self) -> Id {
        let id = Id(self.next);
        self.next += 1;
        id
    }

    fn background(&mut self) {
        let id = self.id();
        let mut l = Layer::shape(id, "Background", vec![ShapeItem::rect(Vec2::new(self.w, self.h), Color::BLACK)])
            .with_position(Vec2::new(self.w / 2.0, self.h / 2.0));
        l.meta.locked = true;
        self.push(l, Role::Background);
    }

    /// A slide heading at the top left, with a short bar under it.
    fn heading(&mut self, title: &str) {
        let (x, y) = (self.w * 0.08, self.h * 0.17);
        self.text(Role::Title, "Title", title, 88.0, Vec2::new(x, y), Anchor::Left, 0.0, "Fade Up by Word");
        self.bar(
            Role::Accent,
            Vec2::new(x + self.w * 0.03, y + 48.0 * self.s),
            Vec2::new(self.w * 0.06, self.h * 0.007),
            0.3,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        role: Role,
        name: &str,
        text: &str,
        size: f64,
        at: Vec2,
        anchor: Anchor,
        start: f64,
        anim: &str,
    ) -> Id {
        let mut t = TextLayer::new(text, self.theme.font, (size * self.s).round(), Color::WHITE);
        if anchor == Anchor::Center {
            t = t.centered();
        }
        if let Some(mut a) = TextAnimator::preset(anim) {
            a.start += start;
            t.animators.push(a);
        }
        let id = self.id();
        self.push(Layer::new(id, name, LayerKind::Text(t)).with_position(at), role)
    }

    #[allow(clippy::too_many_arguments)]
    fn boxed_text(
        &mut self,
        role: Role,
        name: &str,
        text: &str,
        size: f64,
        rect: kurbo::Rect,
        start: f64,
        anim: &str,
    ) -> Id {
        let mut t = TextLayer::new(text, self.theme.font, (size * self.s).round(), Color::WHITE);
        t.valign = crate::TextVAlign::Top;
        t.frame =
            Some(TextFrame { rect: kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height()), wrap: true, clip: false });
        t.line_height = 1.3;
        if let Some(mut a) = TextAnimator::preset(anim) {
            a.start += start;
            t.animators.push(a);
        }
        let id = self.id();
        self.push(Layer::new(id, name, LayerKind::Text(t)).with_position(Vec2::new(rect.x0, rect.y0)), role)
    }

    /// A rounded bar that fades and grows in at `start`.
    fn bar(&mut self, role: Role, at: Vec2, size: Vec2, start: f64) -> Id {
        let id = self.id();
        let mut l = Layer::shape(id, "Accent bar", vec![ShapeItem::rect(size, Color::WHITE)]).with_position(at);
        grow_in(&mut l, start);
        self.push(l, role)
    }

    fn panel(&mut self, role: Role, name: &str, at: Vec2, size: Vec2, start: f64) -> Id {
        let id = self.id();
        let mut l = Layer::shape(id, name, vec![ShapeItem::rect(size, Color::WHITE)]).with_position(at);
        if start > 0.0 {
            fade_in(&mut l, start);
        }
        self.push(l, role)
    }

    fn dot(&mut self, role: Role, name: &str, at: Vec2, r: f64, start: f64) -> Id {
        let id = self.id();
        let mut l = Layer::shape(id, name, vec![ShapeItem::ellipse(Vec2::new(r * 2.0, r * 2.0), Color::WHITE)])
            .with_position(at);
        grow_in(&mut l, start);
        self.push(l, role)
    }

    fn line(&mut self, role: Role, a: Vec2, b: Vec2, start: f64) -> Id {
        let mut p = kurbo::BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((b.x - a.x, b.y - a.y));
        let mut item = ShapeItem::path(p, None, Some((Color::WHITE, (6.0 * self.s).max(1.0))));
        item.trim =
            Some(crate::Trim { start: 0.0.into(), end: keys(&[(start, 0.0), (start + 1.2, 1.0)]), offset: 0.0.into() });
        let id = self.id();
        self.push(Layer::shape(id, "Line", vec![item]).with_position(a), role)
    }

    fn starts_at(&mut self, id: Id, t: f64) {
        if let Some(l) = self.layers.iter_mut().find(|l| l.id == id) {
            l.meta.in_point = t;
        }
    }
}

fn keys<T: crate::Animatable>(k: &[(f64, T)]) -> Property<T> {
    let ease = Easing::Bezier { x1: 0.2, y1: 0.0, x2: 0.0, y2: 1.0 };
    Property::from_keyframes(k.iter().cloned().map(|(time, value)| Keyframe { time, value, easing: ease }).collect())
        .unwrap_or_else(|| k[0].1.clone().into())
}

fn fade_in(l: &mut Layer, start: f64) {
    l.opacity = keys(&[(start, 0.0), (start + 0.5, 1.0)]);
}

fn grow_in(l: &mut Layer, start: f64) {
    fade_in(l, start);
    l.transform.scale = keys(&[(start, Vec2::new(0.0, 1.0)), (start + 0.6, Vec2::new(1.0, 1.0))]);
}

/// Colours and sets the type of `layer` for its role in `theme` (layers
/// without a role are left alone). Returns whether anything changed.
pub fn restyle(layer: &mut Layer, theme: &Theme) -> bool {
    let Some(role) = layer.meta.role else { return false };
    let before = layer.clone();
    let color = theme.color(role);
    match &mut layer.kind {
        LayerKind::Text(t) => {
            t.color = color.into();
            t.font_family = theme.font.to_string();
            if role == Role::Title {
                t.letter_spacing = (theme.title_tracking * t.font_size.value_at(0.0) * 10.0).round() / 10.0;
            }
        }
        LayerKind::Shape { shapes } => {
            for item in shapes {
                if role == Role::Background {
                    let (top, bottom) = theme.background;
                    let size = match &item.geometry {
                        crate::Geometry::Rect { size, .. } => size.value_at(0.0),
                        _ => Vec2::new(0.0, 1080.0),
                    };
                    let gradient = (top != bottom).then(|| GradientPaint {
                        kind: GradientKind::Linear,
                        start: Vec2::new(0.0, -size.y / 2.0).into(),
                        end: Vec2::new(0.0, size.y / 2.0).into(),
                        stops: vec![
                            GradientStop { offset: 0.0, color: top },
                            GradientStop { offset: 1.0, color: bottom },
                        ]
                        .into(),
                    });
                    item.fill = Some(Fill { color: top.into(), gradient, pattern: None });
                    continue;
                }
                if let Some(s) = &mut item.stroke {
                    s.color = color.into();
                }
                if let Some(f) = &mut item.fill {
                    *f = Fill::solid(color);
                }
                if matches!(role, Role::Panel | Role::Accent)
                    && let crate::Geometry::Rect { size, corner_radius, .. } = &mut item.geometry
                {
                    let sz = size.value_at(0.0);
                    // Bars are pills; panels get the theme's rounding.
                    let r = if role == Role::Accent { sz.x.min(sz.y) / 2.0 } else { theme.radius * 1080.0 };
                    *corner_radius = r.min(sz.x.min(sz.y) / 2.0).into();
                }
            }
        }
        _ => {}
    }
    *layer != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layout_builds_in_every_theme() {
        for t in &THEMES {
            for l in Layout::ALL {
                let s = build(l, t, 1920.0, 1080.0, 1, &Content::default());
                assert!(s.layers.len() >= 3, "{} {}", t.name, l.label());
                assert_eq!(s.layers[0].meta.role, Some(Role::Background));
                assert!(s.layers.iter().all(|x| x.meta.role.is_some()), "every layer has a role");
                let ids: std::collections::HashSet<_> = s.layers.iter().map(|x| x.id).collect();
                assert_eq!(ids.len(), s.layers.len(), "unique ids");
                assert!(s.duration >= 3.0);
            }
        }
        assert_eq!(Layout::from_name("big stat"), Some(Layout::BigStat));
        assert_eq!(Layout::from_name("two_column"), Some(Layout::TwoColumn));
        assert_eq!(Layout::from_name("Image + Caption"), Some(Layout::ImageCaption));
        assert!(theme("retro").is_some());
    }

    #[test]
    fn bullets_wait_for_a_click_each() {
        let content =
            Content { title: "Plan".into(), items: vec!["a".into(), "b".into(), "c".into()], ..Default::default() };
        let s = build(Layout::Bullets, &THEMES[0], 1920.0, 1080.0, 1, &content);
        assert_eq!(s.pauses.len(), 2, "before the second and third bullets");
        let bullets: Vec<&Layer> = s.layers.iter().filter(|l| l.meta.name.starts_with("Bullet")).collect();
        assert_eq!(bullets.len(), 3);
        assert!(bullets[2].meta.in_point > bullets[1].meta.in_point);
    }

    #[test]
    fn restyle_recolours_by_role() {
        let s = build(Layout::Title, &THEMES[0], 1920.0, 1080.0, 1, &Content::default());
        let mut title = s.layers.iter().find(|l| l.meta.name == "Title").unwrap().clone();
        let light = theme("Clean Light").unwrap();
        assert!(restyle(&mut title, light));
        let LayerKind::Text(t) = &title.kind else { panic!() };
        assert_eq!(t.color.value_at(0.0), light.title);
        assert!(!restyle(&mut title, light), "already styled");
    }
}
