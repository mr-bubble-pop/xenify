//! The colour picker: a saturation/brightness square with a hue strip, then
//! the colour as RGB, HSV, HSL, CMYK or grey sliders (like Inkscape's Fill and
//! Stroke tabs), alpha, and a hex field. All models work on the sRGB values,
//! as design tools do. CMYK is the plain device formula (no print profile).

use egui::{Color32, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use xenify_doc::Color;

/// How the colour's sliders are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Model {
    #[default]
    Rgb,
    Hsv,
    Hsl,
    Cmyk,
    Grey,
}

impl Model {
    const ALL: [(Model, &'static str, &'static str); 5] = [
        (Model::Rgb, "RGB", "Red, green and blue (0–255)"),
        (Model::Hsv, "HSV", "Hue, saturation and value (brightness), also called HSB"),
        (Model::Hsl, "HSL", "Hue, saturation and lightness (as in CSS hsl())"),
        (
            Model::Cmyk,
            "CMYK",
            "Cyan, magenta, yellow and black ink. The simple screen formula: printers' profiles will differ",
        ),
        (Model::Grey, "Grey", "A single brightness: sets an equal mix of red, green and blue"),
    ];
}

/// What happened in the picker this frame.
#[derive(Default)]
pub struct Picked {
    /// The colour was changed.
    pub changed: bool,
    /// The pointer was released on the picker (a good moment to add the
    /// colour to the recent list).
    pub released: bool,
}

/// Hue and saturation remembered while the colour is grey or black (where
/// they can't be read back from RGB).
#[derive(Clone, Copy)]
struct Memory {
    color: Color,
    h: f64,
    s: f64,
}

fn hsv_of(c: Color, memory: Option<Memory>) -> [f64; 3] {
    let [h, s, v] = rgb_to_hsv(c.r, c.g, c.b);
    match memory {
        Some(m) if m.color == c => [m.h, m.s, v],
        Some(m) if s <= 1e-9 || v <= 1e-9 => [m.h, if v <= 1e-9 { m.s } else { s }, v],
        _ => [h, s, v],
    }
}

/// Shows the picker for `color`. `id_salt` keeps separate pickers'
/// remembered hues apart.
pub fn picker(ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, color: &mut Color) -> Picked {
    let id = ui.id().with(("colour-picker", id_salt));
    let memory: Option<Memory> = ui.data(|d| d.get_temp(id.with("hue")));
    let model_id = egui::Id::new("colour-picker-model");
    let mut model: Model = ui.data_mut(|d| d.get_persisted(model_id)).unwrap_or_default();
    let mut out = Picked::default();
    let [mut h, mut s, mut v] = hsv_of(*color, memory);
    let width = ui.available_width().clamp(160.0, 320.0);

    // The square (saturation →, value ↑) and the hue strip beside it.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let strip = 16.0;
        let side = (width - strip - 6.0).min(200.0);
        let (sq, sq_resp) = ui.allocate_exact_size(vec2(side, side * 0.75), Sense::click_and_drag());
        paint_sv_square(ui.painter(), sq, h);
        if let Some(p) = pointer_on(&sq_resp) {
            s = ((p.x - sq.left()) / sq.width()).clamp(0.0, 1.0) as f64;
            v = 1.0 - ((p.y - sq.top()) / sq.height()).clamp(0.0, 1.0) as f64;
            out.changed = true;
        }
        let at = pos2(sq.left() + s as f32 * sq.width(), sq.top() + (1.0 - v as f32) * sq.height());
        ui.painter().circle_stroke(at, 5.0, Stroke::new(2.5, Color32::BLACK));
        ui.painter().circle_stroke(at, 5.0, Stroke::new(1.5, Color32::WHITE));
        sq_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Saturation and value"));

        let (hr, hue_resp) = ui.allocate_exact_size(vec2(strip, sq.height()), Sense::click_and_drag());
        paint_track(ui.painter(), hr, true, |t| {
            let [r, g, b] = hsv_to_rgb(t * 360.0, 1.0, 1.0);
            Color::rgb(r, g, b)
        });
        if let Some(p) = pointer_on(&hue_resp) {
            h = ((p.y - hr.top()) / hr.height()).clamp(0.0, 1.0) as f64 * 360.0;
            out.changed = true;
        }
        marker(ui.painter(), hr, (h / 360.0) as f32, true);
        hue_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Hue"));
    });
    if out.changed {
        let [r, g, b] = hsv_to_rgb(h, s, v);
        *color = Color::rgba(r, g, b, color.a);
    }

    // Colour model tabs.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (m, label, tip) in Model::ALL {
            if ui.selectable_label(model == m, label).on_hover_text(tip).clicked() {
                model = m;
                ui.data_mut(|d| d.insert_persisted(model_id, model));
            }
        }
    });

    let c = *color;
    let mut edited: Option<Color> = None;
    let slider_w = (width - 62.0).max(80.0);
    let row = |ui: &mut egui::Ui, label: &str, value: f64, max: f64, suffix: &str, at: &dyn Fn(f64) -> Color| {
        let mut shown = value * max;
        let mut set = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_sized(vec2(12.0, 16.0), egui::Label::new(egui::RichText::new(label).monospace()));
            let (r, resp) = ui.allocate_exact_size(vec2(slider_w, 14.0), Sense::click_and_drag());
            paint_track(ui.painter(), r, false, at);
            marker(ui.painter(), r, value as f32, false);
            if let Some(p) = pointer_on(&resp) {
                set = Some(((p.x - r.left()) / r.width()).clamp(0.0, 1.0) as f64);
            }
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, channel_name(label)));
            let decimals = if max > 100.0 && suffix.is_empty() { 0 } else { 1 };
            let field = egui::DragValue::new(&mut shown).range(0.0..=max).speed(max / 200.0).max_decimals(decimals);
            if ui.add_sized(vec2(44.0, 18.0), field.suffix(suffix)).changed() {
                set = Some(shown / max);
            }
        });
        set
    };

    let mut h_new = h;
    let mut s_new = s;
    match model {
        Model::Rgb => {
            let chans = [("R", c.r), ("G", c.g), ("B", c.b)];
            for (i, (label, value)) in chans.into_iter().enumerate() {
                let with = move |t: f64| {
                    let mut x = [c.r, c.g, c.b];
                    x[i] = t;
                    Color::rgb(x[0], x[1], x[2])
                };
                if let Some(t) = row(ui, label, value, 255.0, "", &with) {
                    edited = Some(Color { a: c.a, ..with(t) });
                }
            }
        }
        Model::Hsv => {
            let rgb = |h: f64, s: f64, v: f64| {
                let [r, g, b] = hsv_to_rgb(h, s, v);
                Color::rgb(r, g, b)
            };
            if let Some(t) = row(ui, "H", h / 360.0, 360.0, "°", &|t| rgb(t * 360.0, s.max(0.5), v.max(0.5))) {
                h_new = t * 360.0;
                edited = Some(Color { a: c.a, ..rgb(h_new, s, v) });
            }
            if let Some(t) = row(ui, "S", s, 100.0, "%", &|t| rgb(h, t, v)) {
                s_new = t;
                edited = Some(Color { a: c.a, ..rgb(h, t, v) });
            }
            if let Some(t) = row(ui, "V", v, 100.0, "%", &|t| rgb(h, s, t)) {
                edited = Some(Color { a: c.a, ..rgb(h, s, t) });
            }
        }
        Model::Hsl => {
            let [_, sl, l] = rgb_to_hsl(c.r, c.g, c.b);
            // Keep the remembered saturation where lightness hides it.
            let sl = if l <= 1e-9 || l >= 1.0 - 1e-9 { s } else { sl };
            let rgb = |h: f64, s: f64, l: f64| {
                let [r, g, b] = hsl_to_rgb(h, s, l);
                Color::rgb(r, g, b)
            };
            if let Some(t) = row(ui, "H", h / 360.0, 360.0, "°", &|t| rgb(t * 360.0, sl.max(0.5), 0.5)) {
                h_new = t * 360.0;
                edited = Some(Color { a: c.a, ..rgb(h_new, sl, l) });
            }
            if let Some(t) = row(ui, "S", sl, 100.0, "%", &|t| rgb(h, t, l)) {
                edited = Some(Color { a: c.a, ..rgb(h, t, l) });
            }
            if let Some(t) = row(ui, "L", l, 100.0, "%", &|t| rgb(h, sl, t)) {
                edited = Some(Color { a: c.a, ..rgb(h, sl, t) });
            }
        }
        Model::Cmyk => {
            let cmyk = rgb_to_cmyk(c.r, c.g, c.b);
            for (i, label) in ["C", "M", "Y", "K"].into_iter().enumerate() {
                let with = move |t: f64| {
                    let mut x = cmyk;
                    x[i] = t;
                    let [r, g, b] = cmyk_to_rgb(x);
                    Color::rgb(r, g, b)
                };
                if let Some(t) = row(ui, label, cmyk[i], 100.0, "%", &with) {
                    edited = Some(Color { a: c.a, ..with(t) });
                }
            }
        }
        Model::Grey => {
            let grey = luma(c);
            if let Some(t) = row(ui, "L", grey, 100.0, "%", &|t| Color::rgb(t, t, t)) {
                edited = Some(Color::rgba(t, t, t, c.a));
            }
        }
    }
    let with_alpha = |t: f64| Color { a: t, ..c };
    if let Some(t) = row(ui, "A", c.a, 100.0, "%", &with_alpha) {
        edited = Some(with_alpha(t));
    }
    if let Some(e) = edited {
        *color = e;
        out.changed = true;
        h = h_new;
        s = s_new;
    }

    // Hex (CSS #rrggbb or #rrggbbaa).
    ui.horizontal(|ui| {
        ui.label("Hex");
        let hex_id = id.with("hex");
        let mut hex: String = ui
            .data(|d| d.get_temp::<(Color, String)>(hex_id))
            .filter(|(k, _)| *k == *color)
            .map_or_else(|| color.to_hex(), |(_, text)| text);
        let r = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(90.0).font(egui::TextStyle::Monospace));
        if r.changed()
            && let Some(parsed) = Color::from_hex(hex.trim())
        {
            *color = parsed;
            out.changed = true;
            out.released = true;
        }
        ui.data_mut(|d| d.insert_temp(hex_id, (*color, hex)));
        r.on_hover_text("Type a CSS colour such as #ff8800 or #ff880080 (with alpha)");
    });

    if out.changed {
        // Keep the hue and saturation the user chose, even through grey.
        let [_, s_now, v_now] = rgb_to_hsv(color.r, color.g, color.b);
        let s_keep = if s_now > 1e-9 && v_now > 1e-9 { s_now } else { s };
        ui.data_mut(|d| d.insert_temp(id.with("hue"), Memory { color: *color, h, s: s_keep }));
    }
    out.released = out.released || ui.input(|i| i.pointer.any_released()) && ui.rect_contains_pointer(ui.min_rect());
    out
}

/// A small swatch that opens the picker in a popup when clicked.
pub fn button(ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug + Copy, color: &mut Color) -> Picked {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 18.0), Sense::click());
    checker(ui.painter(), rect);
    ui.painter().rect_filled(rect, 2.0, crate::colors::to_color32(*color));
    let edge = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(90) };
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, edge), StrokeKind::Outside);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, color.to_hex()));
    let resp = resp.on_hover_text(format!("{} — click to change", color.to_hex()));
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(260.0);
            picker(ui, id_salt, color)
        })
        .map(|r| r.inner)
        .unwrap_or_default()
}

/// The full name of a slider's one-letter label, for screen readers.
fn channel_name(label: &str) -> &str {
    match label {
        "R" => "Red",
        "G" => "Green",
        "B" => "Blue",
        "H" => "Hue",
        "S" => "Saturation",
        "V" => "Value",
        "L" => "Lightness",
        "C" => "Cyan",
        "M" => "Magenta",
        "Y" => "Yellow",
        "K" => "Black",
        "A" => "Alpha",
        other => other,
    }
}

/// Where the pointer is while it presses or drags on `resp`.
fn pointer_on(resp: &egui::Response) -> Option<Pos2> {
    (resp.dragged() || resp.is_pointer_button_down_on() || resp.clicked())
        .then(|| resp.interact_pointer_pos())
        .flatten()
}

/// The saturation/value square for hue `h`, as a grid of shaded quads.
fn paint_sv_square(painter: &egui::Painter, rect: Rect, h: f64) {
    const N: u32 = 16;
    let mut mesh = Mesh::default();
    for y in 0..=N {
        for x in 0..=N {
            let (s, v) = (x as f64 / N as f64, 1.0 - y as f64 / N as f64);
            let [r, g, b] = hsv_to_rgb(h, s, v);
            let p = pos2(
                rect.left() + rect.width() * x as f32 / N as f32,
                rect.top() + rect.height() * y as f32 / N as f32,
            );
            mesh.colored_vertex(p, crate::colors::to_color32(Color::rgb(r, g, b)));
        }
    }
    for y in 0..N {
        for x in 0..N {
            let i = y * (N + 1) + x;
            mesh.add_triangle(i, i + 1, i + N + 1);
            mesh.add_triangle(i + 1, i + N + 2, i + N + 1);
        }
    }
    painter.add(mesh);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Outside);
}

/// A slider track shaded by `at(t)` for t in 0..1, over a checkerboard (so
/// alpha shows). `vertical` runs top to bottom.
pub fn paint_track(painter: &egui::Painter, rect: Rect, vertical: bool, at: impl Fn(f64) -> Color) {
    checker(painter, rect);
    const N: u32 = 24;
    let mut mesh = Mesh::default();
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let c = crate::colors::to_color32(at(t as f64));
        let (a, b) = if vertical {
            let y = rect.top() + rect.height() * t;
            (pos2(rect.left(), y), pos2(rect.right(), y))
        } else {
            let x = rect.left() + rect.width() * t;
            (pos2(x, rect.top()), pos2(x, rect.bottom()))
        };
        mesh.colored_vertex(a, c);
        mesh.colored_vertex(b, c);
        if i > 0 {
            let k = i * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(mesh);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Outside);
}

/// A grey checkerboard, shown under translucent colours.
pub fn checker(painter: &egui::Painter, rect: Rect) {
    let cell = 5.0;
    let painter = painter.with_clip_rect(rect);
    let (cols, rows) = ((rect.width() / cell).ceil() as i32, (rect.height() / cell).ceil() as i32);
    for y in 0..rows {
        for x in 0..cols {
            let g = if (x + y) % 2 == 0 { 200 } else { 140 };
            let r = Rect::from_min_size(rect.min + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell));
            painter.rect_filled(r, 0.0, Color32::from_gray(g));
        }
    }
}

/// The position marker on a track: a black and white line across it.
fn marker(painter: &egui::Painter, rect: Rect, t: f32, vertical: bool) {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = if vertical {
        let y = rect.top() + rect.height() * t;
        (pos2(rect.left() - 2.0, y), pos2(rect.right() + 2.0, y))
    } else {
        let x = rect.left() + rect.width() * t;
        (pos2(x, rect.top() - 2.0), pos2(x, rect.bottom() + 2.0))
    };
    painter.line_segment([a, b], Stroke::new(4.0, Color32::BLACK));
    painter.line_segment([a, b], Stroke::new(2.0, Color32::WHITE));
}

/// Relative luminance weights (Rec. 709) on the sRGB values.
fn luma(c: Color) -> f64 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// sRGB (0–1) → hue (0–360), saturation, value (0–1).
pub fn rgb_to_hsv(r: f64, g: f64, b: f64) -> [f64; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    [hue(r, g, b, max, d), s, max]
}

pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let f = |n: f64| {
        let k = (n + h.rem_euclid(360.0) / 60.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)]
}

pub fn rgb_to_hsl(r: f64, g: f64, b: f64) -> [f64; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let l = (max + min) / 2.0;
    let s = if d <= 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    [hue(r, g, b, max, d), s.clamp(0.0, 1.0), l]
}

pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let a = s * l.min(1.0 - l);
    let f = |n: f64| {
        let k = (n + h.rem_euclid(360.0) / 30.0) % 12.0;
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn hue(r: f64, g: f64, b: f64, max: f64, d: f64) -> f64 {
    if d <= 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h * 60.0
}

pub fn rgb_to_cmyk(r: f64, g: f64, b: f64) -> [f64; 4] {
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let f = |x: f64| (1.0 - x - k) / (1.0 - k);
    [f(r), f(g), f(b), k]
}

pub fn cmyk_to_rgb([c, m, y, k]: [f64; 4]) -> [f64; 3] {
    [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn models_round_trip() {
        for rgb in
            [[1.0, 0.5, 0.0], [0.2, 0.4, 0.9], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.3, 0.3, 0.3], [0.9, 0.1, 0.6]]
        {
            let [h, s, v] = rgb_to_hsv(rgb[0], rgb[1], rgb[2]);
            assert!(close(hsv_to_rgb(h, s, v), rgb), "hsv {rgb:?}");
            let [h, s, l] = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
            assert!(close(hsl_to_rgb(h, s, l), rgb), "hsl {rgb:?}");
            assert!(close(cmyk_to_rgb(rgb_to_cmyk(rgb[0], rgb[1], rgb[2])), rgb), "cmyk {rgb:?}");
        }
    }

    #[test]
    fn known_values() {
        assert!(close(hsv_to_rgb(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]));
        assert!(close(hsl_to_rgb(240.0, 1.0, 0.5), [0.0, 0.0, 1.0]));
        let [h, s, l] = rgb_to_hsl(1.0, 0.5, 0.0);
        assert!((h - 30.0).abs() < 1e-9 && (s - 1.0).abs() < 1e-9 && (l - 0.5).abs() < 1e-9);
        assert_eq!(rgb_to_cmyk(1.0, 0.0, 0.0), [0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn grey_keeps_the_remembered_hue() {
        let grey = Color::rgb(0.5, 0.5, 0.5);
        let m = Memory { color: Color::rgb(0.1, 0.1, 0.1), h: 200.0, s: 0.7 };
        let [h, s, v] = hsv_of(grey, Some(m));
        assert_eq!((h, s), (200.0, 0.0));
        assert!((v - 0.5).abs() < 1e-9);
    }
}
