//! Clipping masks (Illustrator's Ctrl+7) built on track mattes.

use crate::{Command, OpError};
use xenify_doc::{Id, LayerKind, LayerMeta, Matte, MatteMode, Project};

/// Makes the top-most of `layers` an alpha matte for all the others, so they
/// only show inside its shape. The mask layer itself is no longer drawn.
pub fn make_clipping_mask(project: &Project, comp: Id, layers: &[Id]) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let mut ordered: Vec<(usize, Id)> = layers.iter().filter_map(|id| c.layer_index(*id).map(|i| (i, *id))).collect();
    ordered.sort();
    ordered.dedup();
    let Some(&(_, mask)) = ordered.last() else {
        return Err(OpError::Invalid("select a shape (on top) and what it should clip".into()));
    };
    if ordered.len() < 2 {
        return Err(OpError::Invalid("select a shape (on top) and what it should clip".into()));
    }
    let mask_layer = project.layer(mask)?;
    if matches!(mask_layer.kind, LayerKind::Group | LayerKind::Null) {
        return Err(OpError::Invalid("the top-most selected layer must be a shape or text to clip with".into()));
    }
    let cmds = ordered[..ordered.len() - 1]
        .iter()
        .map(|&(_, id)| {
            let l = project.layer(id)?;
            let meta = LayerMeta { matte: Some(Matte { layer: mask, mode: MatteMode::Alpha }), ..l.meta.clone() };
            Ok(Command::SetLayerMeta { layer: id, meta })
        })
        .collect::<Result<Vec<_>, OpError>>()?;
    Ok(Command::batch(cmds))
}

/// Removes mattes from `layers` and from anything they are the matte of.
pub fn release_clipping_mask(project: &Project, comp: Id, layers: &[Id]) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let cmds: Vec<Command> = c
        .layers
        .iter()
        .filter(|l| l.meta.matte.is_some_and(|m| layers.contains(&l.id) || layers.contains(&m.layer)))
        .map(|l| Command::SetLayerMeta { layer: l.id, meta: LayerMeta { matte: None, ..l.meta.clone() } })
        .collect();
    if cmds.is_empty() {
        return Err(OpError::Invalid("nothing selected is clipped".into()));
    }
    Ok(Command::batch(cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Color, Layer, ShapeItem, Vec2};

    #[test]
    fn top_layer_clips_the_rest_and_releases() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let shape = |id| Layer::shape(Id(id), "s", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        p.comp_mut(comp).unwrap().layers = vec![shape(1), shape(2), shape(3)];
        let cmd = make_clipping_mask(&p, comp, &[Id(3), Id(1), Id(2)]).unwrap();
        apply(&mut p, cmd).unwrap();
        for id in [1, 2] {
            assert_eq!(p.layer(Id(id)).unwrap().meta.matte.map(|m| m.layer), Some(Id(3)));
        }
        assert!(p.layer(Id(3)).unwrap().meta.matte.is_none());
        // Deleting the mask releases what it clipped; undo restores it.
        let undo = apply(&mut p, Command::RemoveLayer { layer: Id(3) }).unwrap();
        assert!(p.layer(Id(1)).unwrap().meta.matte.is_none());
        apply(&mut p, undo).unwrap();
        assert!(p.layer(Id(1)).unwrap().meta.matte.is_some());
        let cmd = release_clipping_mask(&p, comp, &[Id(3)]).unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(p.layer(Id(2)).unwrap().meta.matte.is_none());
        assert!(make_clipping_mask(&p, comp, &[Id(1)]).is_err());
    }
}
