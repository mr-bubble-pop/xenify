//! Text animators: adding presets, changing and removing them.

use crate::{Command, OpError, invalid};
use xenify_doc::{Id, LayerKind, Project, TextAnimator};

fn animators(project: &Project, layer: Id) -> Result<Vec<TextAnimator>, OpError> {
    match &project.layer(layer)?.kind {
        LayerKind::Text(t) => Ok(t.animators.clone()),
        _ => Err(invalid(format!("layer {layer} is not a text layer"))),
    }
}

/// Adds preset `name` (see [`xenify_doc::text_anim::PRESETS`]) starting at `start` (layer time).
pub fn add_preset(project: &Project, layer: Id, name: &str, start: f64) -> Result<Command, OpError> {
    let preset = TextAnimator::preset(name).ok_or_else(|| invalid(format!("no text animation called {name:?}")))?;
    add(project, layer, TextAnimator { start, ..preset })
}

pub fn add(project: &Project, layer: Id, animator: TextAnimator) -> Result<Command, OpError> {
    let mut list = animators(project, layer)?;
    list.push(animator);
    Ok(Command::SetTextAnimators { layer, animators: list })
}

/// Changes animator `index`.
pub fn edit(project: &Project, layer: Id, index: usize, f: impl FnOnce(&mut TextAnimator)) -> Result<Command, OpError> {
    let mut list = animators(project, layer)?;
    f(list.get_mut(index).ok_or_else(|| invalid(format!("no text animation {index}")))?);
    Ok(Command::SetTextAnimators { layer, animators: list })
}

pub fn remove(project: &Project, layer: Id, index: usize) -> Result<Command, OpError> {
    let mut list = animators(project, layer)?;
    if index >= list.len() {
        return Err(invalid(format!("no text animation {index}")));
    }
    list.remove(index);
    Ok(Command::SetTextAnimators { layer, animators: list })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Color, Layer, TextLayer};

    #[test]
    fn presets_add_edit_remove_and_undo() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let text = Layer::new(Id(1), "Title", LayerKind::Text(TextLayer::new("Hello", "Inter", 40.0, Color::WHITE)));
        p.comp_mut(comp).unwrap().layers = vec![text];
        p.reserve_id(Id(1));
        let list = |p: &Project| match &p.layer(Id(1)).unwrap().kind {
            LayerKind::Text(t) => t.animators.clone(),
            _ => unreachable!(),
        };
        let cmd = add_preset(&p, Id(1), "Pop by Letter", 0.5).unwrap();
        let undo = apply(&mut p, cmd).unwrap();
        assert_eq!(list(&p).len(), 1);
        assert_eq!(list(&p)[0].start, 0.5);
        let cmd = edit(&p, Id(1), 0, |a| a.stagger = 0.2).unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(list(&p)[0].stagger, 0.2);
        assert!(add_preset(&p, Id(1), "Nope", 0.0).is_err());
        let cmd = remove(&p, Id(1), 0).unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(list(&p).is_empty());
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.layer(Id(1)).unwrap().kind, p.layer(Id(1)).unwrap().kind);
        let _ = undo;
    }
}
