//! The interface's colours, font and spacing: one set of named colours
//! ([`Tokens`]) every panel reads, instead of each widget picking its own greys.

use crate::widgets::ACCENT;
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// The colours the interface is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Panels and the dock.
    pub panel: Color32,
    /// Windows, menus and popups (a step lighter, so they lift off the panels).
    pub raised: Color32,
    /// Tab strips and other recessed bars.
    pub recessed: Color32,
    /// Text fields and other places values are typed into.
    pub input: Color32,
    /// Buttons at rest, hovered and pressed.
    pub button: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// Outer edges of windows and fields.
    pub border: Color32,
    /// Lines between rows and sections.
    pub divider: Color32,
    pub text: Color32,
    /// Titles and values that should stand out.
    pub text_strong: Color32,
    /// Hints, units and secondary labels.
    pub text_dim: Color32,
    /// Hidden or unavailable things.
    pub text_disabled: Color32,
    pub accent: Color32,
    pub warning: Color32,
    pub error: Color32,
}

impl Tokens {
    /// A cool slate dark: faintly blue greys, so the neutral artwork on the
    /// canvas reads as the brightest, truest-coloured thing on screen.
    pub const DARK: Tokens = Tokens {
        panel: Color32::from_rgb(0x1f, 0x22, 0x28),
        raised: Color32::from_rgb(0x27, 0x2b, 0x32),
        recessed: Color32::from_rgb(0x18, 0x1a, 0x1f),
        input: Color32::from_rgb(0x14, 0x16, 0x1a),
        button: Color32::from_rgb(0x36, 0x3c, 0x46),
        hover: Color32::from_rgb(0x43, 0x4a, 0x56),
        pressed: Color32::from_rgb(0x24, 0x28, 0x2f),
        border: Color32::from_rgb(0x0f, 0x11, 0x14),
        divider: Color32::from_rgb(0x33, 0x38, 0x41),
        text: Color32::from_rgb(0xd5, 0xd9, 0xe0),
        text_strong: Color32::from_rgb(0xf4, 0xf6, 0xf9),
        text_dim: Color32::from_rgb(0xb4, 0xba, 0xc4),
        text_disabled: Color32::from_rgb(0x5f, 0x66, 0x72),
        accent: ACCENT,
        warning: Color32::from_rgb(0xf0, 0xaa, 0x50),
        error: Color32::from_rgb(0xff, 0x6e, 0x64),
    };

    /// Settings ▸ High contrast: near-black surfaces, white text, bright outlines.
    pub const HIGH_CONTRAST: Tokens = Tokens {
        panel: Color32::from_gray(8),
        raised: Color32::from_gray(14),
        recessed: Color32::from_gray(0),
        input: Color32::BLACK,
        button: Color32::from_gray(24),
        hover: Color32::from_gray(48),
        pressed: Color32::from_gray(0),
        border: Color32::from_gray(200),
        divider: Color32::from_gray(150),
        text: Color32::WHITE,
        text_strong: Color32::WHITE,
        text_dim: Color32::from_gray(215),
        text_disabled: Color32::from_gray(150),
        accent: ACCENT,
        warning: Color32::from_rgb(255, 190, 70),
        error: Color32::from_rgb(255, 120, 110),
    };

    /// The tokens in use (set by [`apply`]).
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::NULL)).unwrap_or(Tokens::DARK)
    }
}

/// Small corners on controls, rounder on windows.
const WIDGET_RADIUS: u8 = 4;
const WINDOW_RADIUS: u8 = 8;

/// Inter for the interface, with egui's own fonts behind it for symbols and
/// emoji Inter lacks. `inter-only` keeps Inter ahead of everything (typing on
/// the canvas looks like the text it edits).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("inter".into(), std::sync::Arc::new(egui::FontData::from_static(xenify_render::INTER_FONT)));
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.retain(|f| f != "inter");
    prop.insert(0, "inter".into());
    let prop = prop.clone();
    // Monospace keeps its fixed-width font first; Inter fills in symbols.
    fonts.families.entry(FontFamily::Monospace).or_default().push("inter".into());
    fonts.families.insert(FontFamily::Name("inter-only".into()), prop);
    ctx.set_fonts(fonts);
}

/// Applies `t` to egui's visuals, text sizes and spacing.
pub fn apply(ctx: &egui::Context, t: Tokens) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
    let high_contrast = t == Tokens::HIGH_CONTRAST;
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(t.text);
    v.weak_text_color = Some(t.text_dim);
    v.panel_fill = t.panel;
    v.window_fill = t.raised;
    v.faint_bg_color = t.recessed;
    v.extreme_bg_color = t.input;
    v.code_bg_color = t.recessed;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.error;
    v.hyperlink_color = t.accent;
    v.window_stroke = Stroke::new(1.0, if high_contrast { t.border } else { t.divider });
    v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow =
        egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(130) };
    v.popup_shadow =
        egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(110) };
    v.selection.bg_fill = t.accent.gamma_multiply(if high_contrast { 0.6 } else { 0.45 });
    v.selection.stroke =
        if high_contrast { Stroke::new(2.0, Color32::from_rgb(255, 220, 0)) } else { Stroke::new(1.0, t.accent) };
    let w = &mut v.widgets;
    for (state, fill) in [
        (&mut w.noninteractive, t.panel),
        (&mut w.inactive, t.button),
        (&mut w.hovered, t.hover),
        (&mut w.active, t.pressed),
        (&mut w.open, t.hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(WIDGET_RADIUS);
        state.fg_stroke = Stroke::new(1.0, t.text);
        state.expansion = 0.0;
    }
    w.hovered.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.active.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
    if high_contrast {
        w.inactive.bg_stroke = Stroke::new(1.5, t.border);
        w.hovered.bg_stroke = Stroke::new(2.0, Color32::WHITE);
        w.active.bg_stroke = Stroke::new(2.0, Color32::WHITE);
    } else {
        // No resting edge: egui counts strokes in a widget's size.
        w.inactive.bg_stroke = Stroke::NONE;
        w.hovered.bg_stroke = Stroke::new(1.0, t.hover.gamma_multiply(1.6));
        w.active.bg_stroke = Stroke::new(1.0, t.accent);
    }
    w.open.bg_stroke = w.hovered.bg_stroke;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(10.5)),
            (TextStyle::Body, FontId::proportional(12.5)),
            (TextStyle::Button, FontId::proportional(12.5)),
            (TextStyle::Heading, FontId::proportional(17.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(6.0, 3.0);
        s.spacing.button_padding = egui::vec2(5.0, 2.0);
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.window_margin = egui::Margin::same(10);
        s.spacing.icon_spacing = 6.0;
        s.animation_time = 0.1;
    });
}

/// Turns the interface's own movement off (or back on): animated opening
/// and closing, smooth scrolling. Previews check [`reduce_motion`].
pub fn set_reduce_motion(ctx: &egui::Context, on: bool) {
    // What was there before is put back when it's turned off.
    let saved = egui::Id::new("motion-before");
    let before = ctx.data(|d| d.get_temp::<egui::style::ScrollAnimation>(saved));
    if on {
        if before.is_none() {
            let scroll = ctx.global_style().scroll_animation;
            ctx.data_mut(|d| d.insert_temp(saved, scroll));
        }
        ctx.all_styles_mut(|s| {
            s.animation_time = 0.0;
            s.scroll_animation = egui::style::ScrollAnimation::none();
        });
    } else if let Some(scroll) = before {
        // `apply` has just set the usual animation time.
        ctx.data_mut(|d| d.remove::<egui::style::ScrollAnimation>(saved));
        ctx.all_styles_mut(|s| s.scroll_animation = scroll);
    }
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("reduce-motion"), on));
}

/// Whether the user asked for less movement (Settings › Motion).
pub fn reduce_motion(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new("reduce-motion"))).unwrap_or(false)
}

/// The dock's tab strips and separators, from the tokens in use.
pub fn dock_style(ui: &egui::Ui) -> egui_dock::Style {
    let t = Tokens::get(ui.ctx());
    let mut s = egui_dock::Style::from_egui(ui.style());
    s.separator.width = 2.0;
    s.separator.color_idle = t.border;
    s.separator.color_hovered = t.accent.gamma_multiply(0.6);
    s.separator.color_dragged = t.accent;
    s.tab_bar.bg_fill = t.recessed;
    s.tab_bar.hline_color = t.border;
    s.tab_bar.height = 26.0;
    s.tab.tab_body.bg_fill = t.panel;
    s.tab.tab_body.stroke = Stroke::NONE;
    // Square surfaces: rounded ones leave specks at the corners where panels meet.
    s.tab.tab_body.corner_radius = CornerRadius::ZERO;
    s.tab_bar.corner_radius = CornerRadius::ZERO;
    s.main_surface_border_stroke = Stroke::NONE;
    s.main_surface_border_rounding = CornerRadius::ZERO;
    let radius = CornerRadius { nw: WIDGET_RADIUS, ne: WIDGET_RADIUS, sw: 0, se: 0 };
    for (tab, bg, text) in [
        (&mut s.tab.active, t.panel, t.text_strong),
        (&mut s.tab.focused, t.panel, t.text_strong),
        (&mut s.tab.inactive, t.recessed, t.text_dim),
        (&mut s.tab.hovered, t.button, t.text),
    ] {
        tab.bg_fill = bg;
        tab.text_color = text;
        tab.outline_color = Color32::TRANSPARENT;
        tab.corner_radius = radius;
    }
    s.buttons.close_tab_color = t.text_dim;
    s.buttons.close_tab_active_color = t.text_strong;
    s.buttons.close_tab_bg_fill = t.hover;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text must stay readable on every surface it sits on (WCAG AA: 4.5:1,
    /// hints and units too); accent, warning and error text on panels and
    /// windows. Disabled text is exempt (WCAG: inactive controls).
    #[test]
    fn text_contrast() {
        fn lum(c: Color32) -> f64 {
            let ch = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        }
        let ratio = |a: Color32, b: Color32| {
            let (a, b) = (lum(a), lum(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for t in [Tokens::DARK, Tokens::HIGH_CONTRAST] {
            for bg in [t.panel, t.raised, t.recessed, t.input, t.button, t.hover] {
                assert!(ratio(t.text, bg) >= 4.5, "text on {bg:?}: {}", ratio(t.text, bg));
                assert!(ratio(t.text_dim, bg) >= 4.5, "dim text on {bg:?}: {}", ratio(t.text_dim, bg));
            }
            for bg in [t.panel, t.raised, t.recessed, t.input] {
                for (name, c) in [("accent", t.accent), ("warning", t.warning), ("error", t.error)] {
                    assert!(ratio(c, bg) >= 4.5, "{name} on {bg:?}: {}", ratio(c, bg));
                }
            }
        }
    }
}
