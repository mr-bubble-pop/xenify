//! Draws display lists with Vello.

use crate::{DocLayout, Item};
use xenify_wp_doc::Rgba;
use kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

pub fn color(c: Rgba) -> Color {
    let [r, g, b, a] = c.0;
    Color::from_rgba8(r, g, b, a)
}

/// Draws the fragments of `layout` that meet `visible` (document space),
/// mapping document space to the target with `transform`.
pub fn paint(scene: &mut vello::Scene, layout: &DocLayout, transform: Affine, visible: Rect) {
    paint_with_images(scene, layout, transform, visible, &|_| None);
}

/// Like [`paint`], drawing pictures with `image` (asset key → decoded image);
/// pictures it can't provide show as a grey box.
pub fn paint_with_images(
    scene: &mut vello::Scene,
    layout: &DocLayout,
    transform: Affine,
    visible: Rect,
    image: &dyn Fn(&str) -> Option<vello::peniko::ImageBrush>,
) {
    for s in layout.shapes.iter().filter(|s| !s.front && s.rect.overlaps(visible)) {
        scene.fill(Fill::NonZero, transform, color(s.color), None, &s.rect);
    }
    for i in layout.images.iter().filter(|i| i.rect.overlaps(visible)) {
        match image(&i.src) {
            Some(brush) => {
                let (w, h) = (brush.image.width as f64, brush.image.height as f64);
                let t = transform
                    * Affine::translate(i.rect.origin().to_vec2())
                    * Affine::scale_non_uniform(i.rect.width() / w.max(1.0), i.rect.height() / h.max(1.0));
                scene.draw_image(&brush, t);
            }
            None => {
                scene.fill(Fill::NonZero, transform, Color::from_rgba8(0xe0, 0xe0, 0xe0, 255), None, &i.rect);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    transform,
                    Color::from_rgba8(0x9e, 0x9e, 0x9e, 255),
                    None,
                    &i.rect,
                );
            }
        }
    }
    for s in layout.shapes.iter().filter(|s| s.front && s.rect.overlaps(visible)) {
        scene.fill(Fill::NonZero, transform, color(s.color), None, &s.rect);
    }
    for f in layout.fragments.iter().chain(&layout.decor) {
        if !f.rect().overlaps(visible) {
            continue;
        }
        let t = transform * Affine::translate(f.origin.to_vec2());
        // Screen-only marks (hints, broken fields) too: this is the screen.
        for item in f.para.items.iter().chain(&f.para.screen) {
            if !f.lines.contains(&item.line()) {
                continue;
            }
            paint_item(scene, item, t);
        }
    }
}

pub fn paint_item(scene: &mut vello::Scene, item: &Item, t: Affine) {
    match item {
        Item::Emoji { rect, glyph, .. } => {
            // A picture sharp at this zoom: the size on screen, rounded up to a power of two.
            let scale = (t.determinant().abs().sqrt() * rect.height()).max(1.0);
            let px = (scale.ceil() as u32).next_power_of_two().clamp(16, 256);
            if let Some(brush) = emoji_brush(glyph, px) {
                let k = rect.width() / px as f64;
                scene.draw_image(&brush, t * Affine::translate(rect.origin().to_vec2()) * Affine::scale(k));
            }
        }
        Item::Rect { rect, color: c, .. } => scene.fill(Fill::NonZero, t, color(*c), None, rect),
        Item::Glyphs { font, size, coords, glyphs, color: c, skew, .. } => {
            let skew = skew.map(|deg| Affine::skew((deg.to_radians() as f64).tan(), 0.0));
            scene
                .draw_glyphs(font)
                .font_size(*size)
                .transform(t)
                .glyph_transform(skew)
                .normalized_coords(coords)
                .brush(color(*c))
                .draw(Fill::NonZero, glyphs.iter().map(|g| vello::Glyph { id: g.id, x: g.x, y: g.y }));
        }
    }
}

/// An emoji picture as an image brush (made once per emoji and size).
fn emoji_brush(glyph: &str, px: u32) -> Option<vello::peniko::ImageBrush> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use vello::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    static CACHE: OnceLock<Mutex<HashMap<(String, u32), ImageData>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut c = cache.lock().ok()?;
    if let Some(img) = c.get(&(glyph.to_string(), px)) {
        return Some(img.clone().into());
    }
    let e = crate::emoji::find(glyph)?;
    let rgba = crate::emoji::picture(e, px)?;
    let img = ImageData {
        data: Blob::new(std::sync::Arc::new(rgba.as_ref().clone())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: px,
        height: px,
    };
    if c.len() > 2000 {
        c.clear();
    }
    c.insert((glyph.to_string(), px), img.clone());
    Some(img.into())
}

/// Draws the page sheets (white, with a soft edge) that meet `visible`.
pub fn paint_pages(scene: &mut vello::Scene, layout: &DocLayout, transform: Affine, visible: Rect) {
    for p in &layout.pages {
        if !p.rect.overlaps(visible) {
            continue;
        }
        let shadow = p.rect + kurbo::Vec2::new(0.0, 1.0);
        scene.fill(Fill::NonZero, transform, Color::from_rgba8(0, 0, 0, 40), None, &shadow.inflate(0.6, 0.6));
        let paper = layout.page_color.map_or(Color::WHITE, |c| {
            let [r, g, b, a] = c.0;
            Color::from_rgba8(r, g, b, a)
        });
        scene.fill(Fill::NonZero, transform, paper, None, &p.rect);
    }
}
