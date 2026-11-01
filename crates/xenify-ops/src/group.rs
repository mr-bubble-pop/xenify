//! Grouping: a group is an invisible layer its children are parented to.

use crate::{Command, OpError};
use kurbo::{Affine, Vec2};
use xenify_anim::{TextMeasure, world_box, world_transform};
use xenify_doc::{AnyProperty, Id, Layer, LayerKind, LayerMeta, Project, PropPath, Property};

/// Groups `layers` (2+, or 1) under a new group placed at their centre, just
/// above the topmost of them; the children are moved to sit directly below it.
/// Returns the command and the group's id.
pub fn group(
    project: &Project,
    comp: Id,
    layers: &[Id],
    name: &str,
    time: f64,
    measure: &mut dyn TextMeasure,
) -> Result<(Command, Id), OpError> {
    let c = project.comp(comp)?;
    if layers.is_empty() {
        return Err(OpError::Invalid("select layers to group".into()));
    }
    // Only take the top of each selected hierarchy (a selected child of a selected group moves with it).
    let mut members: Vec<(usize, Id)> = layers
        .iter()
        .filter(|id| {
            let mut p = c.layer(**id).and_then(|l| l.meta.parent);
            while let Some(pp) = p {
                if layers.contains(&pp) {
                    return false;
                }
                p = c.layer(pp).and_then(|l| l.meta.parent);
            }
            true
        })
        .map(|id| c.layer_index(*id).map(|i| (i, *id)).ok_or(OpError::Invalid(format!("no layer {id}"))))
        .collect::<Result<_, _>>()?;
    members.sort();
    // Members must share a parent (usually none) so the group can take their place.
    let parent = c.layer(members[0].1).and_then(|l| l.meta.parent);
    if members.iter().any(|(_, id)| c.layer(*id).and_then(|l| l.meta.parent) != parent) {
        return Err(OpError::Invalid("layers in different groups can't be grouped together".into()));
    }

    let scene = xenify_anim::evaluate_project(project, c.id, time);
    let bounds = members
        .iter()
        .filter_map(|(_, id)| world_box(c, &scene, *id, measure))
        .reduce(|a, b| a.union(b))
        .ok_or_else(|| OpError::Invalid("nothing visible to group at this time".into()))?;
    // Group origin at the centre (in the members' parent space).
    let parent_inv = parent.map_or(Affine::IDENTITY, |p| world_transform(c, p, time).inverse());
    let centre = (parent_inv * bounds.center()).to_vec2();

    let id = Id(project.next_id);
    let mut g = Layer::new(id, name, LayerKind::Group).with_position(centre);
    g.meta.parent = parent;
    let top = members.last().unwrap().0;
    let mut cmds = vec![Command::AddLayer { comp, index: Some(top + 1), layer: g }];
    // Re-parent each member, shifting all its position keys by -centre so
    // nothing moves on screen. Then stack them contiguously under the group.
    for (_, m) in &members {
        let l = project.layer(*m)?;
        let shifted = match &l.transform.position {
            Property::Static(v) => Property::Static(*v - centre),
            Property::Animated(keys) => Property::Animated(
                keys.iter().map(|k| xenify_doc::Keyframe { value: k.value - centre, ..k.clone() }).collect(),
            ),
        };
        cmds.push(Command::SetProperty { layer: *m, path: PropPath::Position, property: AnyProperty::Vec2(shifted) });
        cmds.push(Command::SetLayerMeta { layer: *m, meta: LayerMeta { parent: Some(id), ..l.meta.clone() } });
    }
    // Place members contiguously right below the group, keeping their order.
    let count = members.len();
    for (k, (_, m)) in members.iter().enumerate() {
        // Final layout: [.., m0, m1, .., m_{n-1}, group]; the group ends at top.
        cmds.push(Command::MoveLayer { layer: *m, index: top + 1 - count + k });
    }
    Ok((Command::batch(cmds), id))
}

/// Dissolves a group: its children keep their on-screen placement and become
/// children of the group's parent. The group's own transform must not be animated.
pub fn ungroup(project: &Project, group: Id) -> Result<Command, OpError> {
    let (comp_id, _) = project.locate_layer(group)?;
    let c = project.comp(comp_id)?;
    let g = project.layer(group)?;
    if !matches!(g.kind, LayerKind::Group) {
        return Err(OpError::Invalid("that isn't a group".into()));
    }
    let t = &g.transform;
    if t.position.is_animated() || t.scale.is_animated() || t.rotation.is_animated() || t.anchor.is_animated() {
        return Err(OpError::Invalid(
            "this group is animated — ungrouping would lose its motion. Remove the group's animation first.".into(),
        ));
    }
    let ga = t.affine_at(0.0);
    let (g_rot, g_scale) = (t.rotation.value_at(0.0), t.scale.value_at(0.0));
    let mut cmds = Vec::new();
    for child in c.layers.iter().filter(|l| l.meta.parent == Some(group)) {
        let map = |p: Vec2| (ga * p.to_point()).to_vec2();
        let position = match &child.transform.position {
            Property::Static(v) => Property::Static(map(*v)),
            Property::Animated(keys) => Property::Animated(
                keys.iter().map(|k| xenify_doc::Keyframe { value: map(k.value), ..k.clone() }).collect(),
            ),
        };
        cmds.push(Command::SetProperty {
            layer: child.id,
            path: PropPath::Position,
            property: AnyProperty::Vec2(position),
        });
        if g_rot != 0.0 {
            let rot = match &child.transform.rotation {
                Property::Static(r) => Property::Static(r + g_rot),
                Property::Animated(keys) => Property::Animated(
                    keys.iter().map(|k| xenify_doc::Keyframe { value: k.value + g_rot, ..k.clone() }).collect(),
                ),
            };
            cmds.push(Command::SetProperty {
                layer: child.id,
                path: PropPath::Rotation,
                property: AnyProperty::Scalar(rot),
            });
        }
        if g_scale != Vec2::new(1.0, 1.0) {
            let mul = |v: Vec2| Vec2::new(v.x * g_scale.x, v.y * g_scale.y);
            let scale = match &child.transform.scale {
                Property::Static(v) => Property::Static(mul(*v)),
                Property::Animated(keys) => Property::Animated(
                    keys.iter().map(|k| xenify_doc::Keyframe { value: mul(k.value), ..k.clone() }).collect(),
                ),
            };
            cmds.push(Command::SetProperty {
                layer: child.id,
                path: PropPath::Scale,
                property: AnyProperty::Vec2(scale),
            });
        }
        cmds.push(Command::SetLayerMeta {
            layer: child.id,
            meta: LayerMeta { parent: g.meta.parent, ..child.meta.clone() },
        });
    }
    cmds.push(Command::RemoveLayer { layer: group });
    Ok(Command::batch(cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_anim::ApproxText;
    use xenify_doc::{Color, ShapeItem};

    fn setup() -> (Project, Id, Vec<Id>) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut ids = vec![];
        for (x, y) in [(100.0, 100.0), (300.0, 200.0), (500.0, 500.0)] {
            let id = p.alloc_id();
            ids.push(id);
            let l = Layer::shape(id, "r", vec![ShapeItem::rect(Vec2::new(50.0, 50.0), Color::WHITE)])
                .with_position(Vec2::new(x, y));
            p.comp_mut(comp).unwrap().layers.push(l);
        }
        (p, comp, ids)
    }

    fn screen(p: &Project, id: Id) -> kurbo::Point {
        world_transform(&p.compositions[0], id, 0.0) * kurbo::Point::ZERO
    }

    #[test]
    fn grouping_keeps_everything_in_place_and_ungroup_restores() {
        let (mut p, comp, ids) = setup();
        let before: Vec<_> = ids.iter().map(|i| screen(&p, *i)).collect();
        let (cmd, g) = group(&p, comp, &[ids[0], ids[2]], "Group", 0.0, &mut ApproxText).unwrap();
        apply(&mut p, cmd).unwrap();
        let after: Vec<_> = ids.iter().map(|i| screen(&p, *i)).collect();
        assert_eq!(before, after, "nothing moved on screen");
        let c = &p.compositions[0];
        // Group sits at the centre of its members' box, directly above them.
        assert_eq!(p.layer(g).unwrap().transform.position.value_at(0.0), Vec2::new(300.0, 300.0));
        let order: Vec<Id> = c.layers.iter().map(|l| l.id).collect();
        assert_eq!(order, vec![ids[1], ids[0], ids[2], g]);
        assert_eq!(xenify_anim::outermost_group(c, ids[0]), g);

        // Moving the group moves both members.
        p.layer_mut(g).unwrap().transform.position = Vec2::new(310.0, 300.0).into();
        assert_eq!(screen(&p, ids[0]).x, 110.0);
        p.layer_mut(g).unwrap().transform.rotation = 90.0.into();
        let rotated = screen(&p, ids[0]);

        let cmd = ungroup(&p, g).unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(p.layer(g).is_err());
        let back = screen(&p, ids[0]);
        assert!((back - rotated).hypot() < 1e-9);
        assert_eq!(p.layer(ids[0]).unwrap().transform.rotation.value_at(0.0), 90.0);
    }

    #[test]
    fn animated_groups_refuse_to_ungroup() {
        let (mut p, comp, ids) = setup();
        let (cmd, g) = group(&p, comp, &ids, "G", 0.0, &mut ApproxText).unwrap();
        apply(&mut p, cmd).unwrap();
        p.layer_mut(g).unwrap().transform.rotation.set_keyframe(1.0, 45.0, None);
        assert!(ungroup(&p, g).is_err());
    }
}
