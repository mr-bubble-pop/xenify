//! Precompose: move layers into a new composition, replaced by one precomp layer.

use crate::{Command, OpError};
use std::collections::HashSet;
use xenify_doc::{CompSettings, Composition, Id, Layer, Project, Vec2};

/// Moves `layers` (plus their group members and the mattes they use) into a
/// new composition of the same size and timing, and puts a precomp layer of it
/// where the top-most one was. Nothing moves on screen.
///
/// Returns the command, the new composition's id and the precomp layer's id.
pub fn precompose(project: &Project, comp: Id, layers: &[Id], name: &str) -> Result<(Command, Id, Id), OpError> {
    let c = project.comp(comp)?;
    let mut set: HashSet<Id> = layers.iter().copied().filter(|id| c.layer(*id).is_some()).collect();
    if set.is_empty() {
        return Err(OpError::Invalid("select the layers to precompose".into()));
    }
    // Bring along everything the selection needs: children and mattes.
    loop {
        let before = set.len();
        for l in &c.layers {
            if l.meta.parent.is_some_and(|p| set.contains(&p)) {
                set.insert(l.id);
            }
            if set.contains(&l.id)
                && let Some(m) = l.meta.matte
            {
                set.insert(m.layer);
            }
        }
        if set.len() == before {
            break;
        }
    }
    for l in &c.layers {
        let inside = set.contains(&l.id);
        if inside && l.meta.parent.is_some_and(|p| !set.contains(&p)) {
            return Err(OpError::Invalid(format!("“{}” is inside a group that isn't selected", l.meta.name)));
        }
        if !inside && l.meta.matte.is_some_and(|m| set.contains(&m.layer)) {
            return Err(OpError::Invalid(format!("“{}” uses a selected layer as its matte", l.meta.name)));
        }
    }

    let moved: Vec<Layer> = c.layers.iter().filter(|l| set.contains(&l.id)).cloned().collect();
    let top = c.layers.iter().rposition(|l| set.contains(&l.id)).unwrap_or(0);
    // Where the precomp goes once the moved layers are gone.
    let index = c.layers[..top].iter().filter(|l| !set.contains(&l.id)).count();

    let (comp_id, layer_id) = (Id(project.next_id), Id(project.next_id + 1));
    let settings = CompSettings { name: name.to_string(), ..c.settings.clone() };
    let centre = Vec2::new(settings.width as f64 / 2.0, settings.height as f64 / 2.0);
    let new_comp = Composition { id: comp_id, settings, layers: moved.clone() };
    let precomp = Layer::precomp(layer_id, name, &new_comp, centre);

    let mut cmds: Vec<Command> = moved.iter().rev().map(|l| Command::RemoveLayer { layer: l.id }).collect();
    cmds.push(Command::AddComposition { index: None, comp: new_comp });
    cmds.push(Command::AddLayer { comp, index: Some(index), layer: precomp });
    Ok((Command::batch(cmds), comp_id, layer_id))
}

/// Adds `main` (with the compositions it uses, `extras`) to the project and
/// shows it in `comp` as precomp layer `layer`: centred on `at`, starting at
/// `start` (seconds), `size` pixels across (else scaled down to fit when
/// bigger than `comp`), and looping when `looped`.
#[allow(clippy::too_many_arguments)]
pub fn add_precomp(
    project: &Project,
    comp: Id,
    main: Composition,
    extras: Vec<Composition>,
    layer: Id,
    at: Vec2,
    start: f64,
    size: Option<f64>,
    looped: bool,
) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (w, h) = (main.settings.width as f64, main.settings.height as f64);
    let k = match size {
        Some(px) => px / w.max(h).max(1.0),
        None => (c.settings.width as f64 / w).min(c.settings.height as f64 / h).min(1.0),
    };
    let mut l = Layer::precomp(layer, main.settings.name.clone(), &main, at);
    if let xenify_doc::LayerKind::Precomp { start: s, looped: lp, .. } = &mut l.kind {
        *s = start;
        *lp = looped;
    }
    l.meta.in_point = start;
    if k != 1.0 {
        l.transform.scale = Vec2::new(k, k).into();
    }
    let mut cmds: Vec<Command> = extras.into_iter().map(|comp| Command::AddComposition { index: None, comp }).collect();
    cmds.push(Command::AddComposition { index: None, comp: main });
    cmds.push(Command::AddLayer { comp, index: None, layer: l });
    Ok(Command::batch(cmds))
}

/// Compositions that (directly or through other precomps) contain `comp`, plus
/// `comp` itself: placing any of them inside `comp` would make a cycle.
pub fn containing(project: &Project, comp: Id) -> HashSet<Id> {
    let mut out = HashSet::from([comp]);
    loop {
        let before = out.len();
        for c in &project.compositions {
            let uses = c
                .layers
                .iter()
                .any(|l| matches!(l.kind, xenify_doc::LayerKind::Precomp { comp, .. } if out.contains(&comp)));
            if uses {
                out.insert(c.id);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_anim::{SceneContent, evaluate_project};
    use xenify_doc::{Color, ShapeItem};

    #[test]
    fn precompose_keeps_the_picture_and_undoes() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let shape = |id: u64, x: f64| {
            Layer::shape(Id(id), format!("s{id}"), vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)])
                .with_position(Vec2::new(x, 100.0))
        };
        p.comp_mut(comp).unwrap().layers = vec![shape(10, 100.0), shape(11, 300.0), shape(12, 500.0)];
        p.reserve_id(Id(12));
        let before = evaluate_project(&p, comp, 0.0);
        let (cmd, new_comp, layer) = precompose(&p, comp, &[Id(10), Id(11)], "Pre").unwrap();
        let undo = apply(&mut p, cmd).unwrap();

        let c = p.comp(comp).unwrap();
        assert_eq!(c.layers.iter().map(|l| l.id).collect::<Vec<_>>(), vec![layer, Id(12)]);
        assert_eq!(p.comp(new_comp).unwrap().layers.len(), 2);
        let after = evaluate_project(&p, comp, 0.0);
        let SceneContent::Precomp { layers, .. } = &after.layers[0].content else { panic!("not a precomp") };
        for (a, b) in layers.iter().zip(&before.layers) {
            assert!((a.transform.as_coeffs()[4] - b.transform.as_coeffs()[4]).abs() < 1e-9, "same place on screen");
        }
        // A composition can't be placed inside itself or anything containing it.
        assert!(containing(&p, new_comp).contains(&comp));

        apply(&mut p, undo).unwrap();
        assert_eq!(p.compositions.len(), 1);
        assert_eq!(p.comp(comp).unwrap().layers.len(), 3);
    }
}
