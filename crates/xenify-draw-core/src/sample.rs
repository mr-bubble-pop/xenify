//! The eyedropper: what's at a point on the canvas. The top-most thing's fill
//! and stroke, and the one colour showing right there (a stroke if the point
//! is on it, else the fill with gradients worked out at the point, a
//! picture's pixel, else the background). As the egui app samples.

use xenify_anim::{Scene, SceneContent, SceneLayer, TextMeasure};
use xenify_doc::{Color, Id};

pub struct Sample {
    pub fill: Option<Color>,
    pub stroke: Option<(Color, f64)>,
    pub visible: Color,
}

pub fn sample(scene: &Scene, point: kurbo::Point, tolerance: f64, measure: &mut dyn TextMeasure) -> Sample {
    let background = Sample { fill: None, stroke: None, visible: scene.background };
    let Some(id) = xenify_anim::hit_test(scene, point, tolerance, measure) else { return background };
    let Some(layer) = find(&scene.layers, id) else { return background };
    sample_layer(layer, point, tolerance, measure).unwrap_or(background)
}

fn find(layers: &[SceneLayer], id: Id) -> Option<&SceneLayer> {
    layers.iter().find(|l| l.id == id)
}

fn sample_layer(
    layer: &SceneLayer,
    point: kurbo::Point,
    tolerance: f64,
    measure: &mut dyn TextMeasure,
) -> Option<Sample> {
    let local = layer.transform.inverse() * point;
    let scale = layer.transform.determinant().abs().sqrt().max(1e-9);
    match &layer.content {
        SceneContent::Text(t) => Some(Sample { fill: Some(t.color), stroke: None, visible: t.color }),
        SceneContent::Image(img) => {
            // The pixel under the point (layer space is one unit per image pixel).
            let data = xenify_render::images::decoded(img)?;
            let x = (local.x * data.width as f64 / img.width.max(1) as f64).floor();
            let y = (local.y * data.height as f64 / img.height.max(1) as f64).floor();
            if x < 0.0 || y < 0.0 || x >= data.width as f64 || y >= data.height as f64 {
                return None;
            }
            let i = (y as usize * data.width as usize + x as usize) * 4;
            let px = data.data.data().get(i..i + 4)?;
            let [r, g, b, a] = [px[0], px[1], px[2], px[3]].map(|v| v as f64 / 255.0);
            let c = Color::rgba(r, g, b, a);
            Some(Sample { fill: Some(c), stroke: None, visible: c })
        }
        SceneContent::Precomp { layers, .. } | SceneContent::Group(layers) => {
            let inner = layers.iter().rev().find(|l| xenify_anim::layer_contains(l, point, tolerance, measure))?;
            sample_layer(inner, point, tolerance, measure)
        }
        SceneContent::Shapes(items) => {
            use kurbo::{ParamCurveNearest, Shape};
            // Top-most item first.
            for item in items.iter().rev() {
                let on_stroke = item.stroke.as_ref().is_some_and(|s| {
                    let half = s.width / 2.0 + tolerance / scale;
                    item.path.segments().any(|seg| seg.nearest(local, 0.1).distance_sq <= half * half)
                });
                let inside = item.fill.is_some() && {
                    let w = item.path.winding(local);
                    match item.fill_rule {
                        xenify_doc::FillRule::NonZero => w != 0,
                        xenify_doc::FillRule::EvenOdd => w % 2 != 0,
                    }
                };
                if !(on_stroke || inside) {
                    continue;
                }
                let fill_here = match (&item.gradient, item.fill) {
                    (Some(g), _) => Some(gradient_at(g, local)),
                    (None, f) => f,
                };
                let stroke = item.stroke.as_ref().map(|s| (s.color, s.width));
                let visible = if on_stroke { stroke.map(|s| s.0) } else { fill_here }.unwrap_or(Color::BLACK);
                return Some(Sample { fill: fill_here, stroke, visible });
            }
            None
        }
    }
}

fn gradient_at(g: &xenify_doc::Gradient, p: kurbo::Point) -> Color {
    let d = g.end - g.start;
    let t = match g.kind {
        xenify_doc::GradientKind::Linear => (p.to_vec2() - g.start).dot(d) / d.hypot2().max(1e-9),
        xenify_doc::GradientKind::Radial => (p.to_vec2() - g.start).hypot() / d.hypot().max(1e-9),
    };
    g.color_at(t.clamp(0.0, 1.0))
}
