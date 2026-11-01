//! Liquid layers: water filling the screen, or a layer's shape, and
//! changing a liquid's settings.

use crate::{Command, OpError, invalid};
use xenify_doc::{Easing, Id, Keyframe, Layer, LayerKind, LiquidFill, Matte, MatteMode, Project, Property, Vec2};

/// The level rising from empty to `to` over `seconds` from `time`.
fn rising(time: f64, seconds: f64, to: f64) -> Property<f64> {
    Property::from_keyframes(vec![
        Keyframe { time, value: 0.0, easing: Easing::EASE_IN_OUT },
        Keyframe { time: time + seconds, value: to, easing: Easing::EASE_IN_OUT },
    ])
    .unwrap_or_else(|| to.into())
}

/// Water that fills the whole canvas, rising from `time`.
pub fn fill_screen(project: &Project, comp: Id, id: Id, time: f64) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (w, h) = (c.settings.width as f64, c.settings.height as f64);
    let mut water = LiquidFill::water(w, h);
    water.level = rising(time, 3.0, 0.92);
    water.slosh = 3.0;
    let layer = Layer::new(id, "Water", LayerKind::Liquid(Box::new(water))).with_position(Vec2::new(w, h) / 2.0);
    Ok(Command::AddLayer { comp, index: None, layer })
}

/// Water filling `target` (e.g. a title): a liquid layer the size of
/// `bounds` (composition space, measured by the caller) masked by the
/// target's shape (Set Mask: alpha), rising from `time`, over a faint copy
/// of the target. Uses ids `id` (the water) and `id + 1` (the copy).
pub fn fill_layer(
    project: &Project,
    comp: Id,
    id: Id,
    target: Id,
    bounds: kurbo::Rect,
    time: f64,
) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let t = c.layer(target).ok_or_else(|| invalid(format!("no layer {target}")))?;
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Err(invalid("the layer to fill has no size"));
    }
    let b = bounds.inflate(4.0, 4.0);
    let mut water = LiquidFill::water(b.width(), b.height());
    water.level = rising(time, 2.5, 1.0);
    water.bubbles = 6;
    let mut layer = Layer::new(id, format!("Water in {}", t.meta.name), LayerKind::Liquid(Box::new(water)))
        .with_position(b.center().to_vec2());
    layer.meta.matte = Some(Matte { layer: target, mode: MatteMode::Alpha });
    // The target is now only the water's mask: a faint copy under the water
    // shows the empty shape ("glass") filling up.
    let mut glass = t.clone();
    glass.id = Id(id.0 + 1);
    glass.meta.name = format!("{} (glass)", t.meta.name);
    glass.opacity = 0.25.into();
    let index = c.layer_index(target).map(|i| i + 1);
    Ok(Command::batch(vec![
        Command::AddLayer { comp, index, layer: glass },
        Command::AddLayer { comp, index: index.map(|i| i + 1), layer },
    ]))
}

/// Changes a liquid layer's settings.
pub fn edit(project: &Project, layer: Id, f: impl FnOnce(&mut LiquidFill)) -> Result<Command, OpError> {
    let LayerKind::Liquid(w) = &project.layer(layer)?.kind else {
        return Err(invalid(format!("layer {layer} is not a liquid layer")));
    };
    let mut w = w.clone();
    f(&mut w);
    Ok(Command::SetLiquid { layer, liquid: w })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Color, TextLayer};

    #[test]
    fn water_rises_to_fill_the_screen() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let cmd = fill_screen(&p, comp, Id(5), 1.0).unwrap();
        apply(&mut p, cmd).unwrap();
        let area = |t: f64| {
            let scene = xenify_anim::evaluate_project(&p, comp, t);
            match scene.layer(Id(5)).map(|l| &l.content) {
                Some(xenify_anim::SceneContent::Shapes(items)) => {
                    items.first().map_or(0.0, |i| kurbo::Shape::area(&i.path).abs())
                }
                _ => 0.0,
            }
        };
        assert_eq!(area(0.5), 0.0, "empty before it starts");
        assert!(area(2.5) > 0.0 && area(2.5) < area(4.5), "rising");
    }

    #[test]
    fn filling_text_masks_the_water_with_it() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let text = Layer::new(Id(1), "Title", LayerKind::Text(TextLayer::new("Effects", "Inter", 120.0, Color::WHITE)));
        p.comp_mut(comp).unwrap().layers.push(text);
        p.reserve_id(Id(1));
        let cmd = fill_layer(&p, comp, Id(2), Id(1), kurbo::Rect::new(100.0, 100.0, 500.0, 220.0), 0.0).unwrap();
        let undo = apply(&mut p, cmd).unwrap();
        let water = p.layer(Id(2)).unwrap();
        assert_eq!(water.meta.matte.map(|m| m.layer), Some(Id(1)));
        let cmd = edit(&p, Id(2), |w| w.bubbles = 0).unwrap();
        apply(&mut p, cmd).unwrap();
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.layer(Id(2)).unwrap().kind, p.layer(Id(2)).unwrap().kind);
        let _ = undo;
    }
}
