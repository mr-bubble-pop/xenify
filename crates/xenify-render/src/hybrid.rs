//! [`Painter`] for Vello Hybrid's scene, which the HTML apps render with
//! WebGL2 (and which works where WebGPU doesn't, such as WebKitGTK).
//!
//! Differences from the Vello painter:
//! - Glyphs are filled as outlines (cached per glyph), so no glyph atlas is needed.
//! - Images are uploaded once by the caller ([`ImageStore`]) and drawn by id.
//! - Luminance masks aren't supported by Hybrid yet: what a luma matte would
//!   cut is drawn unmasked, and the matte itself isn't drawn.

use crate::{Glyph, Painter};
use kurbo::{Affine, BezPath, Shape};
use peniko::{BlendMode, Brush, BrushRef, Color, Fill, FontData, ImageBrush, ImageData};
use std::collections::HashMap;
use vello_common::paint::{ImageId, ImageSource, PaintType};

/// Gives the id of an image already uploaded to the renderer (uploading it
/// the first time it's asked for), or `None` if it can't be.
pub trait ImageStore {
    fn image_id(&mut self, image: &ImageData) -> Option<ImageId>;
}

/// Glyph outlines at one unit per em, so one cached outline serves every size.
#[derive(Default)]
pub struct GlyphCache {
    outlines: HashMap<(u64, u32, u32, Vec<i16>), BezPath>,
}

impl GlyphCache {
    fn outline(&mut self, font: &FontData, glyph: u32, coords: &[i16]) -> Option<&BezPath> {
        use skrifa::MetadataProvider;
        let key = (font.data.id(), font.index, glyph, coords.to_vec());
        if !self.outlines.contains_key(&key) {
            let f = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
            let g = f.outline_glyphs().get(skrifa::GlyphId::new(glyph))?;
            let coords: Vec<skrifa::instance::NormalizedCoord> =
                coords.iter().map(|c| skrifa::raw::types::F2Dot14::from_bits(*c)).collect();
            let location = skrifa::instance::LocationRef::new(&coords);
            let mut pen = UnitPen(BezPath::new());
            let settings = skrifa::outline::DrawSettings::unhinted(skrifa::instance::Size::new(1.0), location);
            g.draw(settings, &mut pen).ok()?;
            if self.outlines.len() > 20_000 {
                self.outlines.clear();
            }
            self.outlines.insert(key.clone(), pen.0);
        }
        self.outlines.get(&key)
    }
}

/// Collects a glyph outline with y pointing down.
struct UnitPen(BezPath);

impl skrifa::outline::OutlinePen for UnitPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, -y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, -y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((cx0 as f64, -cy0 as f64), (x as f64, -y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((cx0 as f64, -cy0 as f64), (cx1 as f64, -cy1 as f64), (x as f64, -y as f64));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// Draws into a Vello Hybrid scene.
pub struct HybridPainter<'a> {
    pub scene: &'a mut vello_hybrid::Scene,
    pub images: &'a mut dyn ImageStore,
    pub glyphs: &'a mut GlyphCache,
    /// Layers pushed so far, and whether each was a real Hybrid layer.
    layers: Vec<bool>,
    /// Inside a luminance mask (unsupported): draw nothing.
    muted: usize,
}

impl<'a> HybridPainter<'a> {
    pub fn new(scene: &'a mut vello_hybrid::Scene, images: &'a mut dyn ImageStore, glyphs: &'a mut GlyphCache) -> Self {
        Self { scene, images, glyphs, layers: Vec::new(), muted: 0 }
    }

    fn paint(&mut self, brush: BrushRef<'_>) -> Option<PaintType> {
        Some(match brush {
            Brush::Solid(c) => PaintType::Solid(c),
            Brush::Gradient(g) => PaintType::Gradient(g.clone()),
            Brush::Image(r) => {
                let id = self.images.image_id(r.image)?;
                PaintType::Image(ImageBrush { image: ImageSource::opaque_id(id), sampler: r.sampler })
            }
        })
    }

    fn set(&mut self, transform: Affine, brush: BrushRef<'_>, brush_transform: Option<Affine>) -> bool {
        let Some(paint) = self.paint(brush) else { return false };
        self.scene.set_transform(transform);
        self.scene.set_paint(paint);
        match brush_transform {
            Some(t) => self.scene.set_paint_transform(t),
            None => self.scene.reset_paint_transform(),
        }
        true
    }
}

/// Curves flattened finely enough for any zoom the editor allows.
fn path_of(shape: &impl Shape) -> BezPath {
    shape.to_path(0.01)
}

impl Painter for HybridPainter<'_> {
    fn fill<'b>(
        &mut self,
        rule: Fill,
        transform: Affine,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        if self.muted > 0 || !self.set(transform, brush.into(), brush_transform) {
            return;
        }
        self.scene.set_fill_rule(rule);
        self.scene.fill_path(&path_of(shape));
    }

    fn stroke<'b>(
        &mut self,
        style: &kurbo::Stroke,
        transform: Affine,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        if self.muted > 0 || !self.set(transform, brush.into(), brush_transform) {
            return;
        }
        self.scene.set_stroke(style.clone());
        self.scene.stroke_path(&path_of(shape));
    }

    fn push_layer(
        &mut self,
        rule: Fill,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
    ) {
        if self.muted > 0 {
            self.muted += 1;
            self.layers.push(false);
            return;
        }
        self.scene.set_transform(transform);
        self.scene.set_fill_rule(rule);
        self.scene.push_layer(Some(&path_of(clip)), Some(blend.into()), Some(alpha), None, None);
        self.layers.push(true);
    }

    fn push_clip_layer(&mut self, rule: Fill, transform: Affine, clip: &impl Shape) {
        self.push_layer(rule, peniko::Mix::Normal, 1.0, transform, clip);
    }

    fn push_luminance_mask_layer(&mut self, _rule: Fill, _alpha: f32, _transform: Affine, _clip: &impl Shape) {
        self.muted += 1;
        self.layers.push(false);
    }

    fn pop_layer(&mut self) {
        match self.layers.pop() {
            Some(true) => self.scene.pop_layer(),
            Some(false) => self.muted = self.muted.saturating_sub(1),
            None => {}
        }
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        let rect = kurbo::Rect::new(0.0, 0.0, image.image.width as f64, image.image.height as f64);
        self.fill(Fill::NonZero, transform, image, None, &rect);
    }

    fn draw_glyphs(
        &mut self,
        font: &FontData,
        size: f32,
        transform: Affine,
        coords: &[i16],
        brush: Color,
        glyphs: &mut dyn Iterator<Item = Glyph>,
    ) {
        if self.muted > 0 {
            return;
        }
        let mut all = BezPath::new();
        for g in glyphs {
            if let Some(outline) = self.glyphs.outline(font, g.id, coords) {
                let place = Affine::translate((g.x as f64, g.y as f64)) * Affine::scale(size as f64);
                all.extend((place * outline.clone()).iter());
            }
        }
        if all.is_empty() {
            return;
        }
        self.scene.set_transform(transform);
        self.scene.set_paint(PaintType::Solid(brush));
        self.scene.reset_paint_transform();
        self.scene.set_fill_rule(Fill::NonZero);
        self.scene.fill_path(&all);
    }
}
