//! Menu and toolbar icons: Lucide (ISC, `assets/icons/lucide.json`, fetched
//! by `scripts/fetch-lucide.sh`). Each icon's strokes are drawn once, at the
//! screen's resolution, into a small white texture that egui tints with the
//! theme's text colour, so the same icon works in light and dark mode and in
//! egui's own image buttons and menu items.

use egui::{Color32, ColorImage, Response, TextureHandle, TextureOptions, Ui, Vec2, Widget};
use kurbo::{BezPath, PathEl, Point, Shape};
use std::collections::HashMap;
use std::sync::OnceLock;

const LUCIDE: &str = include_str!("../../../assets/icons/lucide.json");

macro_rules! icons {
    ($($v:ident = $name:literal),* $(,)?) => {
        /// An icon (its Lucide name in [`Icon::lucide`]).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon { $($v),*, /// Nothing drawn: keeps menu text lined up.
            Blank }
        impl Icon {
            pub fn lucide(self) -> &'static str {
                match self { $(Icon::$v => $name),*, Icon::Blank => "" }
            }
            #[cfg(test)]
            pub const ALL: &'static [Icon] = &[$(Icon::$v),*];
        }
    };
}

icons! {
    Undo = "undo-2", Redo = "redo-2", Bold = "bold", Italic = "italic", Underline = "underline",
    Strike = "strikethrough", Superscript = "superscript", Subscript = "subscript", TextColor = "baseline",
    Highlight = "highlighter", AlignLeft = "text-align-start", AlignCenter = "text-align-center",
    AlignRight = "text-align-end", AlignJustify = "text-align-justify", Bullets = "list",
    Numbers = "list-ordered", Checklist = "list-checks", IndentMore = "list-indent-increase",
    IndentLess = "list-indent-decrease", Image = "image", Table = "table", Link = "link",
    CommentAdd = "message-square-plus", Comments = "message-square", Reply = "message-square-reply",
    Resolve = "circle-check", New = "file-plus", Open = "folder-open", Save = "save", SaveAs = "save-all",
    Print = "printer", Export = "file-down", PageSetup = "file-cog", Recent = "clock", Quit = "log-out",
    Cut = "scissors", Copy = "copy", Paste = "clipboard-paste", PastePlain = "clipboard-type",
    Find = "search", Replace = "replace", Sun = "sun", Moon = "moon", System = "monitor", Ruler = "ruler",
    Columns2 = "columns-2", Columns3 = "columns-3", Header = "panel-top", Footer = "panel-bottom",
    Panel = "panel-right", Heading = "heading", Pilcrow = "pilcrow", Quote = "quote", Rule = "minus",
    ClearFormat = "remove-formatting", ZoomIn = "zoom-in", ZoomOut = "zoom-out", Document = "file-text",
    Contents = "list-tree", Footnote = "footprints", Library = "library", Sources = "book-open",
    Bookmark = "bookmark", Spelling = "spell-check", Ai = "sparkles", Mail = "mail", Settings = "settings",
    Help = "circle-question-mark", Info = "info", Close = "x", Check = "check", Plus = "plus",
    Delete = "trash", Merge = "table-cells-merge", Split = "table-cells-split",
    RowAdd = "between-horizontal-start", ColumnAdd = "between-vertical-start", Fill = "paint-bucket",
    Borders = "square-dashed", Styles = "brush", Wrap = "text-wrap", Crop = "crop", Edit = "pen-line",
    TrackChanges = "file-diff", Eye = "eye", User = "user", Template = "layout-template",
    Break = "separator-horizontal", TextBox = "type", FontSize = "a-large-small",
    Spacing = "chevrons-up-down", SortUp = "arrow-up-narrow-wide", SortDown = "arrow-down-wide-narrow",
    Number = "hash", CheckBox = "square-check", Rows = "rows-2", Field = "text-cursor-input",
    More = "chevron-down", DeleteRow = "panel-bottom-close", DeleteColumn = "panel-right-close",
    SortAz = "arrow-down-a-z", SortZa = "arrow-down-z-a", TableProperties = "table-properties", Pipette = "pipette",
    Up = "arrow-up", Home = "house", Emoji = "face-slightly-smiling", Symbol = "omega", Folded = "chevron-right",
}

/// Each icon's strokes, flattened to polylines in Lucide's 24×24 units, and
/// whether each closes on itself. Parsed once.
type Outline = Vec<(Vec<Point>, bool)>;
/// An icon's strokes and its filled areas.
type Shapes = (Outline, Vec<Vec<Point>>);

fn outlines() -> &'static HashMap<String, Shapes> {
    static OUT: OnceLock<HashMap<String, Shapes>> = OnceLock::new();
    OUT.get_or_init(|| {
        let v: serde_json::Value = serde_json::from_str(LUCIDE).expect("lucide.json is valid");
        let mut out = HashMap::new();
        for (name, els) in v.as_object().into_iter().flatten() {
            let mut strokes = Vec::new();
            let mut fills = Vec::new();
            for el in els.as_array().into_iter().flatten() {
                let (Some(kind), Some(a)) = (el.get(0).and_then(|k| k.as_str()), el.get(1)) else { continue };
                let f = |k: &str| a.get(k).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok());
                let n = |k: &str| f(k).unwrap_or(0.0);
                let path: BezPath = match kind {
                    "path" => a.get("d").and_then(|d| d.as_str()).and_then(|d| BezPath::from_svg(d).ok()),
                    "line" => Some(kurbo::Line::new((n("x1"), n("y1")), (n("x2"), n("y2"))).to_path(0.01)),
                    "circle" => Some(kurbo::Circle::new((n("cx"), n("cy")), n("r")).to_path(0.01)),
                    "ellipse" => Some(kurbo::Ellipse::new((n("cx"), n("cy")), (n("rx"), n("ry")), 0.0).to_path(0.01)),
                    "rect" => {
                        let r = kurbo::Rect::new(n("x"), n("y"), n("x") + n("width"), n("y") + n("height"));
                        let rx = f("rx").or(f("ry")).unwrap_or(0.0);
                        Some(kurbo::RoundedRect::from_rect(r, rx).to_path(0.01))
                    }
                    "polyline" | "polygon" => {
                        let pts: Vec<f64> = a
                            .get("points")
                            .and_then(|p| p.as_str())
                            .unwrap_or("")
                            .split(|c: char| c == ',' || c.is_whitespace())
                            .filter_map(|t| t.parse().ok())
                            .collect();
                        let mut p = BezPath::new();
                        for (i, xy) in pts.as_chunks::<2>().0.iter().enumerate() {
                            if i == 0 { p.move_to((xy[0], xy[1])) } else { p.line_to((xy[0], xy[1])) }
                        }
                        if kind == "polygon" {
                            p.close_path();
                        }
                        Some(p)
                    }
                    _ => None,
                }
                .unwrap_or_default();
                let filled = a.get("fill").and_then(|v| v.as_str()).is_some_and(|f| f != "none");
                for (poly, closed) in flatten(&path) {
                    if filled && closed {
                        fills.push(poly.clone());
                    }
                    strokes.push((poly, closed));
                }
            }
            out.insert(name.clone(), (strokes, fills));
        }
        out
    })
}

fn flatten(path: &BezPath) -> Vec<(Vec<Point>, bool)> {
    let mut out: Vec<(Vec<Point>, bool)> = Vec::new();
    kurbo::flatten(path, 0.02, |el| match el {
        PathEl::MoveTo(p) => out.push((vec![p], false)),
        PathEl::LineTo(p) => {
            if let Some(last) = out.last_mut() {
                last.0.push(p);
            }
        }
        PathEl::ClosePath => {
            if let Some(last) = out.last_mut() {
                last.1 = true;
            }
        }
        _ => {}
    });
    out.retain(|(p, _)| !p.is_empty());
    out
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 == 0.0 { 0.0 } else { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) };
    (p - (a + ab * t)).hypot()
}

fn inside(p: Point, poly: &[Point]) -> bool {
    let mut odd = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            odd = !odd;
        }
    }
    odd
}

/// Draws `icon` into a `px`×`px` coverage mask (Lucide's 2-unit round strokes).
pub fn rasterize(icon: Icon, px: usize) -> Vec<f32> {
    let mut mask = vec![0.0f32; px * px];
    let Some((strokes, fills)) = outlines().get(icon.lucide()) else { return mask };
    let s = px as f64 / 24.0;
    // Lucide strokes are 2 units wide; keep at least a pixel at small sizes.
    let half = (s).max(0.55);
    let segs: Vec<(Point, Point)> = strokes
        .iter()
        .flat_map(|(p, closed)| {
            let n = p.len();
            let extra = if *closed && n > 2 { Some((p[n - 1], p[0])) } else { None };
            p.windows(2)
                .map(|w| (w[0], w[1]))
                .chain(extra)
                .chain((n == 1).then(|| (p[0], p[0])))
                .map(|(a, b)| (Point::new(a.x * s, a.y * s), Point::new(b.x * s, b.y * s)))
                .collect::<Vec<_>>()
        })
        .collect();
    let fills: Vec<Vec<Point>> =
        fills.iter().map(|f| f.iter().map(|p| Point::new(p.x * s, p.y * s)).collect()).collect();
    for y in 0..px {
        for x in 0..px {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let d = segs.iter().map(|&(a, b)| segment_distance(p, a, b)).fold(f64::INFINITY, f64::min);
            let mut c = (half + 0.5 - d).clamp(0.0, 1.0);
            if fills.iter().any(|f| inside(p, f)) {
                c = 1.0;
            }
            mask[y * px + x] = c as f32;
        }
    }
    mask
}

/// The cached texture for `icon` at `size` points on this screen.
fn texture(ctx: &egui::Context, icon: Icon, size: f32) -> TextureHandle {
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as usize;
    let id = egui::Id::new(("lucide-icon", icon, px));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let mask = rasterize(icon, px);
    let pixels = mask.iter().map(|&a| Color32::from_white_alpha((a * 255.0).round() as u8)).collect();
    let image = ColorImage { size: [px, px], pixels, source_size: Vec2::splat(px as f32) };
    let t = ctx.load_texture(format!("icon-{}-{px}", icon.lucide()), image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// `icon` as an egui image `size` points square, in the text colour.
pub fn image(ui: &Ui, icon: Icon, size: f32) -> egui::Image<'static> {
    let t = texture(ui.ctx(), icon, size);
    egui::Image::from_texture((t.id(), Vec2::splat(size))).tint(ui.visuals().text_color())
}

/// The same, in `color`.
pub fn image_colored(ui: &Ui, icon: Icon, size: f32, color: Color32) -> egui::Image<'static> {
    image(ui, icon, size).tint(color)
}

pub const TOOL: f32 = 18.0;
/// A toolbar button's size: the same whether it's hovered or not (egui
/// pads a framed button more, which would shift everything after it).
pub const TOOL_BUTTON: egui::Vec2 = egui::vec2(TOOL + 14.0, TOOL + 12.0);

/// The size a frameless button with `text` (and an icon) takes when
/// hovered, so it can always take that much and not shift what follows.
pub fn steady_size(ui: &Ui, text: &str, icon: bool) -> egui::Vec2 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let w = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE).size().x);
    let pad = ui.spacing().button_padding;
    let icon_w = if icon { MENU + ui.spacing().icon_spacing } else { 0.0 };
    egui::vec2(w + icon_w + 2.0 * pad.x + 4.0, font.size + 2.0 * pad.y + 6.0)
}
/// Height of a menu entry.
pub const ITEM_HEIGHT: f32 = 26.0;

/// A top-level menu's title, its access letter (`key`, the char index)
/// underlined while Alt is held.
pub fn menu_title(ui: &Ui, name: &str, key: usize, underline: bool) -> egui::WidgetText {
    if !underline {
        return name.into();
    }
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    for (i, ch) in name.chars().enumerate() {
        let mut f = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
        if i == key {
            f.underline = egui::Stroke::new(1.0, color);
        }
        job.append(&ch.to_string(), 0.0, f);
    }
    job.into()
}
pub const MENU: f32 = 16.0;

/// A toolbar button showing `icon`, named `name` for screen readers and
/// tests, with `tip` (what it does and its shortcut) on hover.
pub fn button(ui: &mut Ui, icon: Icon, name: &str, tip: &str) -> Response {
    toggle(ui, icon, name, tip, false)
}

/// A toolbar button that shows when it's on (bold, an alignment…).
pub fn toggle(ui: &mut Ui, icon: Icon, name: &str, tip: &str, on: bool) -> Response {
    let img = image(ui, icon, TOOL);
    let r = egui::Button::image(img).selected(on).frame_when_inactive(on).min_size(TOOL_BUTTON).ui(ui);
    let enabled = r.enabled();
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, name));
    r.on_hover_text(tip)
}

/// The same, disabled when `enabled` is false.
pub fn button_enabled(ui: &mut Ui, enabled: bool, icon: Icon, name: &str, tip: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| button(ui, icon, name, tip)).inner
}

/// A menu entry: icon, text and (right-aligned) shortcut.
pub fn item(ui: &mut Ui, icon: Icon, text: &str, shortcut: &str) -> Response {
    let img = image(ui, icon, MENU);
    // Menu entries never wrap: the menu grows to fit them.
    let mut b = egui::Button::image_and_text(img, text)
        .wrap_mode(egui::TextWrapMode::Extend)
        .min_size(egui::vec2(0.0, ITEM_HEIGHT));
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    b.ui(ui)
}

/// A menu entry that can be disabled.
pub fn item_enabled(ui: &mut Ui, enabled: bool, icon: Icon, text: &str, shortcut: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| item(ui, icon, text, shortcut)).inner
}

/// A menu entry with a tick when `on` (a setting or a toggle).
pub fn check_item(ui: &mut Ui, icon: Icon, text: &str, shortcut: &str, on: bool) -> Response {
    let img = image(ui, if on { Icon::Check } else { icon }, MENU);
    let mut b = egui::Button::image_and_text(img, text)
        .selected(on)
        .wrap_mode(egui::TextWrapMode::Extend)
        .min_size(egui::vec2(0.0, ITEM_HEIGHT));
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    b.ui(ui)
}

/// A submenu with an icon.
pub fn submenu<R>(ui: &mut Ui, icon: Icon, text: &str, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let img = image(ui, icon, MENU);
    // The ▸ says it opens a submenu.
    let b = egui::Button::image_and_text(img, text)
        .right_text(egui::containers::menu::SubMenuButton::RIGHT_ARROW)
        .wrap_mode(egui::TextWrapMode::Extend)
        .min_size(egui::vec2(0.0, ITEM_HEIGHT));
    let (r, inner) = egui::containers::menu::SubMenuButton::from_button(b).ui(ui, add);
    // Named by its words (not the arrow) for screen readers and tests.
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    inner.map(|r| r.inner)
}

/// The app's own icon: Lucide's document in white on a rounded blue square.
pub fn app_icon(px: usize) -> egui::IconData {
    let mask = rasterize(Icon::Document, (px as f32 * 0.66) as usize);
    let inner = (px as f32 * 0.66) as usize;
    let off = (px - inner) / 2;
    let r = px as f32 * 0.2;
    let mut rgba = Vec::with_capacity(px * px * 4);
    for y in 0..px {
        for x in 0..px {
            // Rounded square coverage.
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, px as f32 - r);
            let cy = fy.clamp(r, px as f32 - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let bg = (r + 0.5 - d).clamp(0.0, 1.0);
            let fg = if (off..off + inner).contains(&x) && (off..off + inner).contains(&y) {
                mask[(y - off) * inner + (x - off)]
            } else {
                0.0
            };
            let (br, bgc, bb) = (47.0, 111.0, 237.0);
            let mix = |b: f32| b + (255.0 - b) * fg;
            rgba.extend_from_slice(&[mix(br) as u8, mix(bgc) as u8, mix(bb) as u8, (bg * 255.0) as u8]);
        }
    }
    egui::IconData { rgba, width: px as u32, height: px as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_in_the_set_and_draws_something() {
        for &icon in Icon::ALL {
            assert!(outlines().contains_key(icon.lucide()), "{icon:?} ({}) isn't in lucide.json", icon.lucide());
            let mask = rasterize(icon, 24);
            let ink: f32 = mask.iter().sum();
            assert!(ink > 10.0, "{icon:?} draws almost nothing ({ink})");
            assert!(ink < 24.0 * 24.0 * 0.8, "{icon:?} is nearly solid ({ink})");
        }
    }

    #[test]
    fn app_icon_is_square_and_opaque_in_the_middle() {
        let i = app_icon(64);
        assert_eq!(i.rgba.len(), 64 * 64 * 4);
        let mid = (32 * 64 + 32) * 4;
        assert_eq!(i.rgba[mid + 3], 255);
    }
}
