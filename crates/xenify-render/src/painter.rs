//! What the scene encoder draws into: Vello's own scene on the desktop
//! (wgpu compute), or Vello Hybrid's in the browser and the HTML apps
//! (WebGL2), behind one set of calls shaped like Vello's.

use kurbo::{Affine, Shape};
use peniko::{BlendMode, BrushRef, Color, Fill, FontData, ImageBrush};

/// A glyph of a font run, positioned relative to the run's transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

pub trait Painter {
    fn fill<'b>(
        &mut self,
        rule: Fill,
        transform: Affine,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    );
    fn stroke<'b>(
        &mut self,
        style: &kurbo::Stroke,
        transform: Affine,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    );
    /// A group composited onto what's below with `blend` and `alpha`, clipped to `clip`.
    fn push_layer(&mut self, rule: Fill, blend: impl Into<BlendMode>, alpha: f32, transform: Affine, clip: &impl Shape);
    fn push_clip_layer(&mut self, rule: Fill, transform: Affine, clip: &impl Shape);
    /// A group whose luminance masks what was drawn before it in the enclosing group.
    fn push_luminance_mask_layer(&mut self, rule: Fill, alpha: f32, transform: Affine, clip: &impl Shape);
    fn pop_layer(&mut self);
    fn draw_image(&mut self, image: &ImageBrush, transform: Affine);
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        font: &FontData,
        size: f32,
        transform: Affine,
        coords: &[i16],
        brush: Color,
        glyphs: &mut dyn Iterator<Item = Glyph>,
    );
}

#[cfg(feature = "gpu")]
impl Painter for vello::Scene {
    fn fill<'b>(
        &mut self,
        rule: Fill,
        transform: Affine,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        vello::Scene::fill(self, rule, transform, brush, brush_transform, shape);
    }

    fn stroke<'b>(
        &mut self,
        style: &kurbo::Stroke,
        transform: Affine,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        vello::Scene::stroke(self, style, transform, brush, brush_transform, shape);
    }

    fn push_layer(
        &mut self,
        rule: Fill,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
    ) {
        vello::Scene::push_layer(self, rule, blend, alpha, transform, clip);
    }

    fn push_clip_layer(&mut self, rule: Fill, transform: Affine, clip: &impl Shape) {
        vello::Scene::push_clip_layer(self, rule, transform, clip);
    }

    fn push_luminance_mask_layer(&mut self, rule: Fill, alpha: f32, transform: Affine, clip: &impl Shape) {
        vello::Scene::push_luminance_mask_layer(self, rule, alpha, transform, clip);
    }

    fn pop_layer(&mut self) {
        vello::Scene::pop_layer(self);
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        vello::Scene::draw_image(self, image, transform);
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
        vello::Scene::draw_glyphs(self, font)
            .font_size(size)
            .transform(transform)
            .normalized_coords(coords)
            .brush(brush)
            .draw(Fill::NonZero, glyphs.map(|g| vello::Glyph { id: g.id, x: g.x, y: g.y }));
    }
}
