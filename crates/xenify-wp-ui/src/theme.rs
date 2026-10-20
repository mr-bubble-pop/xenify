//! Write's look: light and dark palettes (our own colours), Inter as the
//! interface font, and the colours the document canvas uses around the
//! pages. Pages stay white in both modes, as printed.

use egui::{Color32, CornerRadius, FontFamily, Stroke, Theme, ThemePreference, Visuals};

/// Light, dark, or whatever the computer uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Appearance::System => "Same as the computer",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Appearance::System => Icon::System,
            Appearance::Light => Icon::Sun,
            Appearance::Dark => Icon::Moon,
        }
    }
}

/// One palette's colours.
struct Tokens {
    panel: Color32,
    window: Color32,
    recessed: Color32,
    faint: Color32,
    button: Color32,
    hover: Color32,
    pressed: Color32,
    border: Color32,
    text: Color32,
    text_strong: Color32,
    text_dim: Color32,
    accent: Color32,
    selection: Color32,
    error: Color32,
    warning: Color32,
}

const LIGHT: Tokens = Tokens {
    panel: Color32::from_rgb(0xf6, 0xf7, 0xf9),
    window: Color32::from_rgb(0xff, 0xff, 0xff),
    recessed: Color32::from_rgb(0xec, 0xee, 0xf2),
    faint: Color32::from_rgb(0xf0, 0xf2, 0xf5),
    button: Color32::from_rgb(0xe9, 0xec, 0xf0),
    hover: Color32::from_rgb(0xe6, 0xe9, 0xee),
    pressed: Color32::from_rgb(0xd8, 0xdd, 0xe5),
    border: Color32::from_rgb(0xd5, 0xd9, 0xe0),
    text: Color32::from_rgb(0x2a, 0x2e, 0x35),
    text_strong: Color32::from_rgb(0x12, 0x15, 0x1a),
    text_dim: Color32::from_rgb(0x5f, 0x66, 0x70),
    accent: Color32::from_rgb(0x24, 0x60, 0xd2),
    selection: Color32::from_rgb(0xcf, 0xdd, 0xfb),
    error: Color32::from_rgb(0xb8, 0x2a, 0x2a),
    warning: Color32::from_rgb(0x94, 0x57, 0x00),
};

const DARK: Tokens = Tokens {
    panel: Color32::from_rgb(0x1f, 0x22, 0x28),
    window: Color32::from_rgb(0x25, 0x29, 0x30),
    recessed: Color32::from_rgb(0x17, 0x19, 0x1e),
    faint: Color32::from_rgb(0x2a, 0x2e, 0x36),
    button: Color32::from_rgb(0x2b, 0x30, 0x38),
    hover: Color32::from_rgb(0x34, 0x3a, 0x44),
    pressed: Color32::from_rgb(0x40, 0x47, 0x53),
    border: Color32::from_rgb(0x3a, 0x40, 0x4a),
    text: Color32::from_rgb(0xd9, 0xdd, 0xe3),
    text_strong: Color32::from_rgb(0xf4, 0xf6, 0xf8),
    text_dim: Color32::from_rgb(0xa3, 0xaa, 0xb5),
    accent: Color32::from_rgb(0x5b, 0x8c, 0xff),
    selection: Color32::from_rgb(0x2c, 0x45, 0x7a),
    error: Color32::from_rgb(0xff, 0x6b, 0x6b),
    warning: Color32::from_rgb(0xff, 0xb3, 0x47),
};

/// High contrast, light: black text and outlines on white.
const LIGHT_HC: Tokens = Tokens {
    panel: Color32::from_rgb(0xff, 0xff, 0xff),
    window: Color32::from_rgb(0xff, 0xff, 0xff),
    recessed: Color32::from_rgb(0xf2, 0xf2, 0xf2),
    faint: Color32::from_rgb(0xf2, 0xf2, 0xf2),
    button: Color32::from_rgb(0xf2, 0xf2, 0xf2),
    hover: Color32::from_rgb(0xdd, 0xe6, 0xff),
    pressed: Color32::from_rgb(0xc4, 0xd4, 0xff),
    border: Color32::from_rgb(0x00, 0x00, 0x00),
    text: Color32::from_rgb(0x00, 0x00, 0x00),
    text_strong: Color32::from_rgb(0x00, 0x00, 0x00),
    text_dim: Color32::from_rgb(0x30, 0x30, 0x30),
    accent: Color32::from_rgb(0x00, 0x3d, 0xb0),
    selection: Color32::from_rgb(0xb3, 0xcb, 0xff),
    error: Color32::from_rgb(0xa0, 0x00, 0x00),
    warning: Color32::from_rgb(0x70, 0x40, 0x00),
};

/// High contrast, dark: white text and bright outlines on black.
const DARK_HC: Tokens = Tokens {
    panel: Color32::from_rgb(0x00, 0x00, 0x00),
    window: Color32::from_rgb(0x0a, 0x0a, 0x0a),
    recessed: Color32::from_rgb(0x00, 0x00, 0x00),
    faint: Color32::from_rgb(0x12, 0x12, 0x12),
    button: Color32::from_rgb(0x18, 0x18, 0x18),
    hover: Color32::from_rgb(0x1c, 0x2c, 0x50),
    pressed: Color32::from_rgb(0x24, 0x3c, 0x70),
    border: Color32::from_rgb(0xd0, 0xd0, 0xd0),
    text: Color32::from_rgb(0xff, 0xff, 0xff),
    text_strong: Color32::from_rgb(0xff, 0xff, 0xff),
    text_dim: Color32::from_rgb(0xe0, 0xe0, 0xe0),
    accent: Color32::from_rgb(0x8c, 0xb8, 0xff),
    selection: Color32::from_rgb(0x1f, 0x4f, 0xa8),
    error: Color32::from_rgb(0xff, 0x8a, 0x8a),
    warning: Color32::from_rgb(0xff, 0xc8, 0x60),
};

/// Settings › Appearance: interface size, contrast and motion.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Display {
    /// Interface size (1.0 = normal); the page has its own zoom.
    pub ui_scale: f32,
    /// Black-on-white (or white-on-black) with strong outlines.
    pub high_contrast: bool,
    /// No blinking caret, no sliding or fading menus, no smooth scrolling.
    pub reduce_motion: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self { ui_scale: 1.0, high_contrast: false, reduce_motion: false }
    }
}

/// Applies the appearance and display settings (cheap when unchanged).
pub fn apply_display(ctx: &egui::Context, appearance: Appearance, d: &Display) {
    let scale = d.ui_scale.clamp(0.5, 3.0);
    if (ctx.zoom_factor() - scale).abs() > 1e-3 {
        ctx.set_zoom_factor(scale);
    }
    let key = egui::Id::new("write-display-applied");
    let now = (appearance, d.high_contrast, d.reduce_motion);
    if ctx.data(|m| m.get_temp::<(Appearance, bool, bool)>(key)) == Some(now) {
        return;
    }
    ctx.data_mut(|m| m.insert_temp(key, now));
    ctx.data_mut(|m| m.insert_temp(egui::Id::new("write-high-contrast"), d.high_contrast));
    apply(ctx, appearance);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |s| {
            if d.high_contrast {
                // Thicker outlines on every control, and a bold focus ring.
                let w = &mut s.visuals.widgets;
                for (st, width) in
                    [(&mut w.inactive, 1.5), (&mut w.hovered, 2.0), (&mut w.active, 2.0), (&mut w.open, 1.5)]
                {
                    st.bg_stroke.width = width;
                    st.bg_stroke.color = s.visuals.window_stroke.color;
                }
                s.visuals.selection.stroke.width = 2.0;
                s.visuals.window_stroke.width = 1.5;
            }
        });
    }
    motion(ctx, d.reduce_motion);
    ctx.data_mut(|m| m.insert_temp(egui::Id::new("write-reduce-motion"), d.reduce_motion));
}

/// Stops the interface's own animation and smooth scrolling (or puts back
/// what was there before).
fn motion(ctx: &egui::Context, reduce: bool) {
    let saved = egui::Id::new("write-motion-before");
    let before = ctx.data(|m| m.get_temp::<(f32, egui::style::ScrollAnimation)>(saved));
    if reduce && before.is_none() {
        let s = ctx.global_style();
        ctx.data_mut(|m| m.insert_temp(saved, (s.animation_time, s.scroll_animation)));
        ctx.all_styles_mut(|s| {
            s.animation_time = 0.0;
            s.scroll_animation = egui::style::ScrollAnimation::none();
        });
    } else if !reduce && let Some((time, scroll)) = before {
        ctx.data_mut(|m| m.remove::<(f32, egui::style::ScrollAnimation)>(saved));
        ctx.all_styles_mut(|s| {
            s.animation_time = time;
            s.scroll_animation = scroll;
        });
    }
}

/// Whether the user asked for less movement (the caret stops blinking).
pub fn reduce_motion(ctx: &egui::Context) -> bool {
    ctx.data(|m| m.get_temp::<bool>(egui::Id::new("write-reduce-motion"))).unwrap_or(false)
}

fn high_contrast(ctx: &egui::Context) -> bool {
    ctx.data(|m| m.get_temp::<bool>(egui::Id::new("write-high-contrast"))).unwrap_or(false)
}

fn visuals(t: &Tokens, dark: bool) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.window;
    v.extreme_bg_color = t.recessed;
    v.text_edit_bg_color = Some(t.window);
    v.faint_bg_color = t.faint;
    v.code_bg_color = t.recessed;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.hyperlink_color = t.accent;
    v.error_fg_color = t.error;
    v.warn_fg_color = t.warning;
    v.selection.bg_fill = t.selection;
    v.selection.stroke = Stroke::new(1.0, t.text_strong);
    v.override_text_color = None;
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow.color = Color32::from_black_alpha(if dark { 110 } else { 40 });
    v.popup_shadow.color = Color32::from_black_alpha(if dark { 100 } else { 30 });
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.panel;
    w.noninteractive.weak_bg_fill = t.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    for (s, fill) in [(&mut w.inactive, t.button), (&mut w.hovered, t.hover), (&mut w.active, t.pressed)] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.corner_radius = CornerRadius::same(5);
    }
    w.inactive.bg_stroke = Stroke::new(1.0, t.border);
    w.inactive.fg_stroke = Stroke::new(1.0, t.text);
    w.hovered.bg_stroke = Stroke::new(1.0, t.border);
    w.hovered.fg_stroke = Stroke::new(1.5, t.text_strong);
    w.active.bg_stroke = Stroke::new(1.0, t.accent);
    w.active.fg_stroke = Stroke::new(1.5, t.text_strong);
    w.open.bg_fill = t.hover;
    w.open.weak_bg_fill = t.hover;
    w.open.fg_stroke = Stroke::new(1.0, t.text_strong);
    v.weak_text_color = Some(t.text_dim);
    v
}

/// Uses `appearance` and installs both palettes.
pub fn apply(ctx: &egui::Context, appearance: Appearance) {
    let (light, dark) = if high_contrast(ctx) { (&LIGHT_HC, &DARK_HC) } else { (&LIGHT, &DARK) };
    ctx.set_visuals_of(Theme::Light, visuals(light, false));
    ctx.set_visuals_of(Theme::Dark, visuals(dark, true));
    ctx.options_mut(|o| {
        o.theme_preference = match appearance {
            Appearance::System => ThemePreference::System,
            Appearance::Light => ThemePreference::Light,
            Appearance::Dark => ThemePreference::Dark,
        }
    });
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |s| {
            s.spacing.button_padding = egui::vec2(6.0, 3.0);
            s.spacing.menu_margin = egui::Margin::same(6);
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.interact_size.y = 22.0;
        });
    }
}

/// Inter for the interface, with egui's own fonts behind it for emoji and
/// symbols Inter lacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), std::sync::Arc::new(egui::FontData::from_static(xenify_wp_layout::INTER_FONT)));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    // Arrows, maths and Greek that neither Inter nor egui's fonts have.
    fonts
        .font_data
        .insert("symbols".into(), std::sync::Arc::new(egui::FontData::from_static(xenify_wp_layout::SYMBOL_FONT)));
    fonts.families.entry(FontFamily::Proportional).or_default().push("symbols".into());
    fonts.families.entry(FontFamily::Monospace).or_default().push("inter".into());
    ctx.set_fonts(fonts);
}

/// Colours of the document canvas around and on the pages.
pub struct Canvas {
    /// Behind the pages.
    pub desk: Color32,
    /// Selected text on the (white) page.
    pub selection: Color32,
    pub caret: Color32,
    /// Selected text boxes and their handles.
    pub accent: Color32,
    /// Outlines of unselected text boxes.
    pub guide: Color32,
    /// Links between text boxes, and their overflow marker.
    pub warning: Color32,
}

pub fn canvas(v: &Visuals) -> Canvas {
    if v.dark_mode {
        Canvas {
            desk: Color32::from_rgb(0x14, 0x16, 0x1a),
            selection: Color32::from_rgba_unmultiplied(66, 133, 244, 72),
            caret: Color32::from_rgb(0x20, 0x21, 0x24),
            accent: Color32::from_rgb(0x2f, 0x6f, 0xed),
            guide: Color32::from_gray(185),
            warning: Color32::from_rgb(0xd9, 0x30, 0x25),
        }
    } else {
        Canvas {
            desk: Color32::from_rgb(0xe7, 0xe9, 0xed),
            selection: Color32::from_rgba_unmultiplied(66, 133, 244, 64),
            caret: Color32::from_rgb(0x20, 0x21, 0x24),
            accent: Color32::from_rgb(0x2f, 0x6f, 0xed),
            guide: Color32::from_gray(185),
            warning: Color32::from_rgb(0xd9, 0x30, 0x25),
        }
    }
}

/// The accent colour (selected things, the document icon).
pub fn accent(v: &Visuals) -> Color32 {
    if v.dark_mode { DARK.accent } else { LIGHT.accent }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast ratio of two colours.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_is_readable_in_both_palettes() {
        for t in [&LIGHT, &DARK, &LIGHT_HC, &DARK_HC] {
            for bg in [t.panel, t.window, t.button, t.hover] {
                assert!(contrast(t.text, bg) >= 7.0, "text on {bg:?}: {}", contrast(t.text, bg));
                assert!(contrast(t.text_dim, bg) >= 4.5, "dim text on {bg:?}: {}", contrast(t.text_dim, bg));
            }
            // WCAG AA for coloured text on the surfaces it's used on.
            for bg in [t.panel, t.window, t.recessed] {
                for (name, c) in [("accent", t.accent), ("warning", t.warning), ("error", t.error)] {
                    assert!(contrast(c, bg) >= 4.5, "{name} on {bg:?}: {}", contrast(c, bg));
                }
            }
            assert!(contrast(t.text_strong, t.selection) >= 4.5);
        }
    }
}
