//! Particle layers: adding one from a preset and changing its emitter.

use crate::{Command, OpError, invalid};
use xenify_doc::{Emitter, EmitterShape, EndLook, Id, Layer, LayerKind, Project};

/// A new particle layer with preset `name`, on top of `comp`. `target` is
/// the layer "Assemble into Shape" forms (it needs one).
pub fn add_preset(project: &Project, comp: Id, id: Id, name: &str, target: Option<Id>) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (w, h) = (c.settings.width as f64, c.settings.height as f64);
    let (mut e, at) = Emitter::preset(name, w, h).ok_or_else(|| invalid(format!("no particle preset {name:?}")))?;
    let mut at = at;
    match name {
        "Assemble into Shape" => {
            let target = target.ok_or_else(|| invalid("select the layer the particles should form first"))?;
            c.layer(target).ok_or_else(|| invalid(format!("no layer {target}")))?;
            e.end = EndLook::Form { target };
        }
        "Burning Shape" => {
            let source = target.ok_or_else(|| invalid("select the layer that should burn first"))?;
            c.layer(source).ok_or_else(|| invalid(format!("no layer {source}")))?;
            e.shape = EmitterShape::Layer { layer: source };
            // The emitter sits on the burning layer (its marker, and the flames' origin).
            at = xenify_anim::world_transform(c, source, 0.0).translation();
        }
        _ => {}
    }
    e.seed = id.0;
    let mut layer = Layer::new(id, name, LayerKind::Particles(Box::new(e))).with_position(at);
    if Emitter::preset_glows(name) {
        layer.meta.blend = xenify_doc::BlendMode::Add;
    }
    Ok(Command::AddLayer { comp, index: None, layer })
}

/// Changes a particle layer's emitter.
pub fn edit(project: &Project, layer: Id, f: impl FnOnce(&mut Emitter)) -> Result<Command, OpError> {
    let LayerKind::Particles(e) = &project.layer(layer)?.kind else {
        return Err(invalid(format!("layer {layer} is not a particle layer")));
    };
    let mut e = e.clone();
    f(&mut e);
    Ok(Command::SetEmitter { layer, emitter: e })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;

    #[test]
    fn every_preset_adds_a_layer_that_draws_and_undoes() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let logo = Layer::shape(
            Id(1),
            "Logo",
            vec![xenify_doc::ShapeItem::rect(kurbo::Vec2::new(80.0, 80.0), xenify_doc::Color::WHITE)],
        );
        p.comp_mut(comp).unwrap().layers.push(logo);
        p.reserve_id(Id(1));
        for (n, name) in xenify_doc::PARTICLE_PRESETS.iter().chain(xenify_doc::FIRE_PRESETS).enumerate() {
            let id = Id(10 + n as u64);
            let cmd = add_preset(&p, comp, id, name, Some(Id(1))).unwrap();
            let undo = apply(&mut p, cmd).unwrap();
            let scene = xenify_anim::evaluate_project(&p, comp, 1.0);
            let drawn = scene.layer(id).map_or(0, |l| match &l.content {
                xenify_anim::SceneContent::Shapes(items) => items.len(),
                _ => 0,
            });
            assert!(drawn > 0, "{name} draws particles");
            apply(&mut p, undo).unwrap();
        }
        assert!(add_preset(&p, comp, Id(99), "Assemble into Shape", None).is_err(), "needs a target");
    }

    #[test]
    fn edits_are_undoable_and_saved() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let cmd = add_preset(&p, comp, Id(5), "Snow", None).unwrap();
        apply(&mut p, cmd).unwrap();
        let cmd = edit(&p, Id(5), |e| e.size = 30.0.into()).unwrap();
        let undo = apply(&mut p, cmd).unwrap();
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.layer(Id(5)).unwrap().kind, p.layer(Id(5)).unwrap().kind);
        apply(&mut p, undo).unwrap();
        let LayerKind::Particles(e) = &p.layer(Id(5)).unwrap().kind else { unreachable!() };
        assert_eq!(e.size.value_at(0.0), 8.0);
    }
}
