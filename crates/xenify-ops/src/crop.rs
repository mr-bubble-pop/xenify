//! Cropping: resizing the composition to a rectangle, clipping layers to one,
//! and cutting out slices for export.

use crate::{Command, OpError, invalid};
use xenify_doc::{AnyProperty, Id, Project, PropPath, Property, Vec2};
use kurbo::Rect;

/// Resizes `comp` to `rect` (composition pixels, rounded to whole pixels).
/// Top-level layers move so that what was at the rectangle's top-left is now
/// at 0,0 (every position keyframe moves too); slices move with them. The
/// rectangle may extend past the frame to enlarge the canvas.
pub fn crop_comp(project: &Project, comp: Id, rect: Rect) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let r = rect.round();
    if r.width() < 1.0 || r.height() < 1.0 {
        return Err(invalid("the crop area is empty"));
    }
    let offset = Vec2::new(-r.x0, -r.y0);
    let mut cmds = Vec::new();
    if offset != Vec2::ZERO {
        for l in c.layers.iter().filter(|l| l.meta.parent.is_none()) {
            let moved = match l.property(PropPath::Position)? {
                AnyProperty::Vec2(p) => AnyProperty::Vec2(offset_vec2(p, offset)),
                other => other,
            };
            cmds.push(Command::SetProperty { layer: l.id, path: PropPath::Position, property: moved });
        }
    }
    let mut settings = c.settings.clone();
    settings.width = r.width() as u32;
    settings.height = r.height() as u32;
    for s in &mut settings.slices {
        s.rect = s.rect + offset;
    }
    cmds.push(Command::SetCompSettings { comp, settings });
    Ok(Command::batch(cmds))
}

fn offset_vec2(p: Property<Vec2>, d: Vec2) -> Property<Vec2> {
    match p {
        Property::Static(v) => Property::Static(v + d),
        Property::Animated(mut keys) => {
            for k in &mut keys {
                k.value += d;
            }
            Property::Animated(keys)
        }
    }
}

/// The name of the mask Crop adds (and adjusts when cropping again).
pub const CROP_MASK: &str = "Crop";

/// Crops `layers` (layers or groups) to `rect` (composition space) with a
/// mask on each: their "Crop" mask is adjusted if they have one (keyframed
/// when it's animated or `auto_key` is on), otherwise added.
pub fn crop_layers(
    project: &Project,
    comp: Id,
    layers: &[Id],
    rect: Rect,
    time: f64,
    auto_key: bool,
) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    if layers.is_empty() {
        return Err(invalid("select the layers to crop"));
    }
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return Err(invalid("the crop area is empty"));
    }
    let mut cmds = Vec::new();
    for id in layers {
        let layer = project.layer(*id)?;
        let to_local = xenify_anim::world_transform(c, *id, time).inverse();
        let local = to_local * xenify_doc::rect_path(rect);
        match layer.meta.masks.iter().position(|m| m.name == CROP_MASK) {
            Some(index) => {
                let path = PropPath::Mask { index, field: xenify_doc::MaskField::Path };
                cmds.push(Command::edit_value(project, *id, path, time, xenify_doc::Value::Path(local), auto_key)?);
            }
            None => {
                let mut meta = layer.meta.clone();
                meta.masks.push(xenify_doc::Mask::new(CROP_MASK, local));
                cmds.push(Command::SetLayerMeta { layer: *id, meta });
            }
        }
    }
    Ok(Command::batch(cmds))
}

/// A copy of `project` in which `comp` is cropped to `rect` (for exporting a slice).
pub fn cropped_project(project: &Project, comp: Id, rect: Rect) -> Result<Project, OpError> {
    let mut p = project.clone();
    let cmd = crop_comp(&p, comp, rect)?;
    crate::apply(&mut p, cmd)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Color, Keyframe, Layer, ShapeItem};

    #[test]
    fn cropping_the_canvas_moves_content_and_undoes() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(10), "Box", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        l.transform.position = Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: Vec2::new(500.0, 400.0), easing: Default::default() },
            Keyframe { time: 1.0, value: Vec2::new(600.0, 400.0), easing: Default::default() },
        ])
        .unwrap();
        p.comp_mut(comp).unwrap().layers.push(l);
        p.comp_mut(comp)
            .unwrap()
            .settings
            .slices
            .push(xenify_doc::Slice { name: "s".into(), rect: Rect::new(450.0, 350.0, 550.0, 450.0) });
        let before = p.clone();
        let cmd = crop_comp(&p, comp, Rect::new(400.0, 300.0, 1040.0, 660.2)).unwrap();
        let undo = crate::apply(&mut p, cmd).unwrap();
        let c = p.comp(comp).unwrap();
        assert_eq!((c.settings.width, c.settings.height), (640, 360));
        let pos = &c.layers[0].transform.position;
        assert_eq!(pos.value_at(0.0), Vec2::new(100.0, 100.0));
        assert_eq!(pos.value_at(1.0), Vec2::new(200.0, 100.0));
        assert_eq!(c.settings.slices[0].rect, Rect::new(50.0, 50.0, 150.0, 150.0));
        crate::apply(&mut p, undo).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn cropping_layers_masks_them_and_recropping_adjusts_the_mask() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        for id in [1, 2] {
            let l = Layer::shape(Id(id), "s", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE)])
                .with_position(Vec2::new(100.0, 100.0));
            p.comp_mut(comp).unwrap().layers.push(l);
        }
        p.next_id = 3;
        let cmd = crop_layers(&p, comp, &[Id(1)], Rect::new(60.0, 70.0, 110.0, 90.0), 0.0, false).unwrap();
        crate::apply(&mut p, cmd).unwrap();
        let c = p.comp(comp).unwrap();
        assert_eq!(c.layers.len(), 2, "no extra layer");
        let masks = &c.layer(Id(1)).unwrap().meta.masks;
        assert_eq!(masks.len(), 1);
        // In the layer's own space (it sits at 100,100).
        assert_eq!(xenify_doc::as_rect(&masks[0].path.value_at(0.0)), Some(Rect::new(-40.0, -30.0, 10.0, -10.0)));
        assert!(c.layer(Id(2)).unwrap().meta.masks.is_empty());
        // Cropping again with auto-key at 1 s animates the same mask.
        let cmd = crop_layers(&p, comp, &[Id(1)], Rect::new(50.0, 50.0, 150.0, 150.0), 1.0, true).unwrap();
        crate::apply(&mut p, cmd).unwrap();
        let masks = &p.layer(Id(1)).unwrap().meta.masks;
        assert_eq!(masks.len(), 1);
        assert!(masks[0].path.is_animated());
        assert_eq!(xenify_doc::as_rect(&masks[0].path.value_at(1.0)), Some(Rect::new(-50.0, -50.0, 50.0, 50.0)));
    }
}
