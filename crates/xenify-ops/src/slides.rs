//! Slide operations: add, duplicate, delete and reorder slides. Each returns
//! one command (one undo step).

use crate::{Command, OpError};
use xenify_doc::{Composition, Id, Project, Slide};
use std::collections::HashMap;

/// The slide list, or the main composition as the only slide when the
/// project isn't a presentation yet.
pub fn slides_or_main(project: &Project) -> Vec<Slide> {
    if project.slides.is_empty() {
        project.main_comp().map(|c| vec![Slide::new(c.id)]).unwrap_or_default()
    } else {
        project.slides.clone()
    }
}

/// A new empty slide after slide `after` (at the end if `None`), the same
/// size as the slide it follows. Returns the command and the new composition.
pub fn add_slide(project: &Project, after: Option<usize>) -> Result<(Command, Id), OpError> {
    let mut slides = slides_or_main(project);
    let at = after.map_or(slides.len(), |i| (i + 1).min(slides.len()));
    let template = slides.get(at.saturating_sub(1)).and_then(|s| project.comp(s.comp).ok());
    let mut settings = template.map(|c| c.settings.clone()).unwrap_or_default();
    settings.name = format!("Slide {}", slides.len() + 1);
    settings.slices.clear();
    settings.pauses.clear();
    let id = Id(project.next_id);
    let comp = Composition { id, settings, layers: Vec::new() };
    let transition_in = slides.get(at.saturating_sub(1)).map(|s| s.transition_in).unwrap_or_default();
    slides.insert(at, Slide { transition_in, ..Slide::new(id) });
    Ok((Command::batch(vec![Command::AddComposition { index: None, comp }, Command::SetSlides { slides }]), id))
}

/// A copy of slide `index` right after it (layers get new ids).
pub fn duplicate_slide(project: &Project, index: usize) -> Result<(Command, Id), OpError> {
    let mut slides = slides_or_main(project);
    let slide = slides.get(index).cloned().ok_or_else(|| OpError::Invalid(format!("no slide {}", index + 1)))?;
    let src = project.comp(slide.comp)?;
    let mut next = project.next_id;
    let mut alloc = || {
        next += 1;
        Id(next - 1)
    };
    let id = alloc();
    let layers = copy_layers(&src.layers, &mut alloc);
    let mut settings = src.settings.clone();
    settings.name = format!("{} copy", settings.name);
    let comp = Composition { id, settings, layers };
    slides.insert(index + 1, Slide { comp: id, ..slide });
    Ok((Command::batch(vec![Command::AddComposition { index: None, comp }, Command::SetSlides { slides }]), id))
}

/// Copies of `layers` with new ids from `alloc`, their links (parents,
/// mattes, text paths) pointing at the copies.
pub fn copy_layers(layers: &[xenify_doc::Layer], alloc: &mut dyn FnMut() -> Id) -> Vec<xenify_doc::Layer> {
    let map: HashMap<Id, Id> = layers.iter().map(|l| (l.id, alloc())).collect();
    layers
        .iter()
        .map(|l| {
            let mut l = l.clone();
            l.id = map[&l.id];
            l.meta.parent = l.meta.parent.map(|p| map.get(&p).copied().unwrap_or(p));
            if let Some(m) = &mut l.meta.matte {
                m.layer = map.get(&m.layer).copied().unwrap_or(m.layer);
            }
            if let xenify_doc::LayerKind::Text(t) = &mut l.kind
                && let Some(p) = &mut t.path
            {
                p.layer = map.get(&p.layer).copied().unwrap_or(p.layer);
            }
            l
        })
        .collect()
}

/// Deletes slide `index` and its composition (unless a precomp uses it).
pub fn delete_slide(project: &Project, index: usize) -> Result<Command, OpError> {
    let mut slides = slides_or_main(project);
    if slides.len() <= 1 {
        return Err(OpError::Invalid("a presentation needs at least one slide".into()));
    }
    if index >= slides.len() {
        return Err(OpError::Invalid(format!("no slide {}", index + 1)));
    }
    let gone = slides.remove(index);
    Ok(Command::batch(vec![Command::SetSlides { slides }, Command::RemoveComposition { comp: gone.comp }]))
}

/// Moves slide `from` to position `to`.
pub fn move_slide(project: &Project, from: usize, to: usize) -> Result<Option<Command>, OpError> {
    let mut slides = slides_or_main(project);
    if from >= slides.len() {
        return Err(OpError::Invalid(format!("no slide {}", from + 1)));
    }
    let to = to.min(slides.len() - 1);
    if from == to {
        return Ok(None);
    }
    let s = slides.remove(from);
    slides.insert(to, s);
    Ok(Some(Command::SetSlides { slides }))
}

/// Changes slide `index` (its transitions or notes) with `edit`.
pub fn edit_slide(project: &Project, index: usize, edit: impl FnOnce(&mut Slide)) -> Result<Command, OpError> {
    let mut slides = slides_or_main(project);
    let s = slides.get_mut(index).ok_or_else(|| OpError::Invalid(format!("no slide {}", index + 1)))?;
    edit(s);
    Ok(Command::SetSlides { slides })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Color, Layer, ShapeItem, TransitionKind, Vec2};

    fn undo_round_trip(p: &mut Project, cmd: Command) {
        let before = p.clone();
        let undo = apply(p, cmd).unwrap();
        let mut back = p.clone();
        apply(&mut back, undo).unwrap();
        back.next_id = before.next_id;
        assert_eq!(back, before);
    }

    #[test]
    fn a_plain_project_becomes_a_presentation() {
        let mut p = Project::new();
        let main = p.compositions[0].id;
        let (cmd, second) = add_slide(&p, None).unwrap();
        undo_round_trip(&mut p, cmd);
        let order: Vec<Id> = p.slides.iter().map(|s| s.comp).collect();
        assert_eq!(order, [main, second]);
        assert_eq!(p.comp(second).unwrap().settings.width, 1920);
        assert_eq!(p.comp(second).unwrap().settings.name, "Slide 2");
    }

    #[test]
    fn duplicate_copies_layers_with_new_ids_and_keeps_links() {
        let mut p = Project::new();
        let main = p.compositions[0].id;
        let g = p.alloc_id();
        let a = p.alloc_id();
        let mut member = Layer::shape(a, "a", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        member.meta.parent = Some(g);
        p.comp_mut(main).unwrap().layers = vec![member, Layer::new(g, "G", xenify_doc::LayerKind::Group)];
        let (cmd, copy) = duplicate_slide(&p, 0).unwrap();
        undo_round_trip(&mut p, cmd);
        let c = p.comp(copy).unwrap();
        assert_eq!(c.layers.len(), 2);
        assert!(c.layers.iter().all(|l| l.id != a && l.id != g));
        assert_eq!(c.layers[0].meta.parent, Some(c.layers[1].id), "the copy's member points at the copy's group");
        assert_eq!(p.slides.len(), 2);
    }

    #[test]
    fn delete_move_and_edit() {
        let mut p = Project::new();
        let first = p.compositions[0].id;
        let (cmd, second) = add_slide(&p, None).unwrap();
        apply(&mut p, cmd).unwrap();
        let (cmd, third) = add_slide(&p, Some(0)).unwrap();
        apply(&mut p, cmd).unwrap();
        let order = |p: &Project| p.slides.iter().map(|s| s.comp).collect::<Vec<_>>();
        assert_eq!(order(&p), [first, third, second]);

        let cmd = move_slide(&p, 2, 0).unwrap().unwrap();
        undo_round_trip(&mut p, cmd);
        assert_eq!(order(&p), [second, first, third]);

        let cmd = edit_slide(&p, 1, |s| {
            s.transition_in.kind = TransitionKind::Fade;
            s.notes = "Say hello".into();
        })
        .unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.slides[1].transition_in.kind, TransitionKind::Fade);

        let cmd = delete_slide(&p, 1).unwrap();
        undo_round_trip(&mut p, cmd);
        assert_eq!(order(&p), [second, third]);
        assert!(p.comp(first).is_err());
        // Slides can't be removed as plain compositions, and one slide always stays.
        assert!(apply(&mut p.clone(), Command::RemoveComposition { comp: second }).is_err());
        let cmd = delete_slide(&p, 0).unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(delete_slide(&p, 0).is_err());
    }
}
