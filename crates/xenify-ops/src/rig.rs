//! Live rigging while posing on the canvas:
//! - **IK**: dragging a layer whose `rig.ik_chain` is set bends its parent
//!   joints (rotating each about its anchor) so it reaches the pointer.
//!
//! Results are ordinary rotation / position edits (auto-key records poses).

use crate::{Command, OpError};
use xenify_anim::world_transform;
use xenify_doc::{Composition, Id, LayerMeta, Project, PropPath, Property, Rig, Value, Vec2};
use kurbo::Point;

/// Iterations of cyclic coordinate descent per drag update.
const IK_ITERATIONS: usize = 12;

fn anchor_world(c: &Composition, id: Id, time: f64) -> Option<Point> {
    let l = c.layer(id)?;
    Some(world_transform(c, id, time) * l.transform.anchor.value_at(time).to_point())
}

/// The joints IK bends for `layer`: its parents, nearest first, up to its chain length.
pub fn ik_joints(c: &Composition, layer: Id) -> Vec<Id> {
    let Some(l) = c.layer(layer) else { return vec![] };
    let mut out = Vec::new();
    let mut cur = l.meta.parent;
    while let Some(p) = cur {
        if out.len() >= l.meta.rig.ik_chain as usize || out.contains(&p) {
            break;
        }
        out.push(p);
        cur = c.layer(p).and_then(|x| x.meta.parent);
    }
    out
}

/// Rotation edits that bend `layer`'s IK chain so its anchor reaches `target`
/// (composition space). Empty if it has no chain.
pub fn ik_commands(
    project: &Project,
    comp: Id,
    layer: Id,
    target: Point,
    time: f64,
    auto_key: bool,
) -> Result<Vec<Command>, OpError> {
    let c = project.comp(comp)?;
    let joints = ik_joints(c, layer);
    if joints.is_empty() {
        return Ok(vec![]);
    }
    // Solve on a scratch copy whose joint rotations are plain numbers.
    let mut work = c.clone();
    let mut angle: Vec<f64> =
        joints.iter().map(|j| c.layer(*j).map_or(0.0, |l| l.transform.rotation.value_at(time))).collect();
    let set = |work: &mut Composition, j: Id, deg: f64| {
        if let Some(l) = work.layers.iter_mut().find(|l| l.id == j) {
            l.transform.rotation = Property::Static(deg);
        }
    };
    for (j, a) in joints.iter().zip(&angle) {
        set(&mut work, *j, *a);
    }
    for _ in 0..IK_ITERATIONS {
        for (k, j) in joints.iter().enumerate() {
            let (Some(pivot), Some(end)) = (anchor_world(&work, *j, time), anchor_world(&work, layer, time)) else {
                continue;
            };
            let (a, b) = (end - pivot, target - pivot);
            if a.hypot() < 1e-6 || b.hypot() < 1e-6 {
                continue;
            }
            let delta = (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y).to_degrees();
            angle[k] += delta;
            set(&mut work, *j, angle[k]);
        }
        if anchor_world(&work, layer, time).is_some_and(|e| (e - target).hypot() < 0.25) {
            break;
        }
    }
    joints
        .iter()
        .zip(angle)
        .map(|(j, deg)| Command::edit_value(project, *j, PropPath::Rotation, time, Value::Scalar(deg), auto_key))
        .collect()
}

/// Links `layers` (in order) into a joint chain: each is parented to the one
/// before it without moving on screen, and the last one drives the chain by IK.
pub fn make_joint_chain(project: &Project, comp: Id, layers: &[Id], time: f64) -> Result<Command, OpError> {
    if layers.len() < 2 {
        return Err(OpError::Invalid(
            "select 2 or more layers, from the root (e.g. upper arm) to the tip (hand)".into(),
        ));
    }
    let c = project.comp(comp)?;
    let mut cmds = Vec::new();
    for w in layers.windows(2) {
        let (parent, child) = (w[0], w[1]);
        if xenify_anim::descendants(c, child).contains(&parent) {
            return Err(OpError::Invalid("that would make a layer its own ancestor".into()));
        }
        let l = project.layer(child)?;
        // Keep the child where it is on screen under its new parent.
        let world = world_transform(c, child, time);
        let local = world_transform(c, parent, time).inverse() * world;
        let [a, b, cc, d, _, _] = local.as_coeffs();
        let theta = b.atan2(a);
        let (sx, sy) = (a.hypot(b), -cc * theta.sin() + d * theta.cos());
        let anchor = l.transform.anchor.value_at(time).to_point();
        let set = |path, property| Command::SetProperty { layer: child, path, property };
        cmds.push(Command::SetLayerMeta { layer: child, meta: LayerMeta { parent: Some(parent), ..l.meta.clone() } });
        cmds.push(set(PropPath::Position, xenify_doc::AnyProperty::Vec2(Property::Static((local * anchor).to_vec2()))));
        cmds.push(set(PropPath::Rotation, xenify_doc::AnyProperty::Scalar(Property::Static(theta.to_degrees()))));
        cmds.push(set(PropPath::Scale, xenify_doc::AnyProperty::Vec2(Property::Static(Vec2::new(sx, sy)))));
    }
    // The tip drives the whole chain.
    let tip = project.layer(*layers.last().unwrap())?;
    let mut meta = tip.meta.clone();
    meta.parent = Some(layers[layers.len() - 2]);
    meta.rig = Rig { ik_chain: (layers.len() - 1) as u32 };
    cmds.push(Command::SetLayerMeta { layer: tip.id, meta });
    Ok(Command::batch(cmds))
}

/// Removes IK from `layers` (parenting is kept).
pub fn unlink(project: &Project, layers: &[Id]) -> Result<Command, OpError> {
    let cmds = layers
        .iter()
        .map(|id| {
            let l = project.layer(*id)?;
            Ok(Command::SetLayerMeta { layer: *id, meta: LayerMeta { rig: Rig::default(), ..l.meta.clone() } })
        })
        .collect::<Result<Vec<_>, OpError>>()?;
    Ok(Command::batch(cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Color, Layer, ShapeItem};

    /// A two-bone arm along +x: shoulder at (100,100), elbow 100 px out, hand 100 px further.
    fn arm() -> (Project, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let bone = |id: u64, pos: Vec2| {
            Layer::shape(Id(id), format!("b{id}"), vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)])
                .with_position(pos)
        };
        let upper = bone(1, Vec2::new(100.0, 100.0));
        let mut lower = bone(2, Vec2::new(100.0, 0.0));
        lower.meta.parent = Some(Id(1));
        let mut hand = bone(3, Vec2::new(100.0, 0.0));
        hand.meta.parent = Some(Id(2));
        hand.meta.rig.ik_chain = 2;
        p.comp_mut(comp).unwrap().layers = vec![upper, lower, hand];
        p.reserve_id(Id(10));
        (p, comp)
    }

    #[test]
    fn ik_bends_the_chain_to_reach_the_target() {
        let (mut p, comp) = arm();
        // Reachable: 200 px arm, target 150 px from the shoulder.
        let target = Point::new(200.0, 220.0);
        let cmds = ik_commands(&p, comp, Id(3), target, 0.0, false).unwrap();
        assert_eq!(cmds.len(), 2);
        apply(&mut p, Command::batch(cmds)).unwrap();
        let hand = anchor_world(p.comp(comp).unwrap(), Id(3), 0.0).unwrap();
        assert!((hand - target).hypot() < 1.0, "hand at {hand:?}");
        // The shoulder stays put.
        assert_eq!(anchor_world(p.comp(comp).unwrap(), Id(1), 0.0).unwrap(), Point::new(100.0, 100.0));
    }

    #[test]
    fn joint_chains_keep_layers_in_place() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let box_at = |id: u64, x: f64| {
            Layer::shape(Id(id), "b", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)])
                .with_position(Vec2::new(x, 50.0))
        };
        let mut a = box_at(1, 100.0);
        a.transform.rotation = 30.0.into();
        p.comp_mut(comp).unwrap().layers = vec![a, box_at(2, 200.0), box_at(3, 300.0)];
        p.reserve_id(Id(10));
        let before: Vec<Point> = (1..=3).map(|i| anchor_world(p.comp(comp).unwrap(), Id(i), 0.0).unwrap()).collect();
        let cmd = make_joint_chain(&p, comp, &[Id(1), Id(2), Id(3)], 0.0).unwrap();
        apply(&mut p, cmd).unwrap();
        let c = p.comp(comp).unwrap();
        for (i, b) in (1..=3).zip(before) {
            assert!((anchor_world(c, Id(i), 0.0).unwrap() - b).hypot() < 1e-6);
        }
        assert_eq!(c.layer(Id(3)).unwrap().meta.parent, Some(Id(2)));
        assert_eq!(c.layer(Id(3)).unwrap().meta.rig.ik_chain, 2);
    }
}
