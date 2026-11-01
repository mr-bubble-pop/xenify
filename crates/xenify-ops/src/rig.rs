//! Live rigging while posing on the canvas:
//! - **IK**: dragging a layer whose `rig.ik_chain` is set bends its parent
//!   joints (rotating each about its anchor) so it reaches the pointer.
//!
//! Results are ordinary rotation / position edits (auto-key records poses).

use crate::{Command, OpError};
use kurbo::Point;
use xenify_anim::world_transform;
use xenify_doc::{Composition, Id, LayerMeta, Project, PropPath, Property, Rig, Value, Vec2};

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
    ik_commands_with(project, comp, layer, target, time, auto_key, None)
}

/// Like [`ik_commands`], bending `chain` parent joints (all of them up to 12
/// with `Some(0)`) instead of the layer's own `rig.ik_chain`.
pub fn ik_commands_with(
    project: &Project,
    comp: Id,
    layer: Id,
    target: Point,
    time: f64,
    auto_key: bool,
    chain: Option<u32>,
) -> Result<Vec<Command>, OpError> {
    let c = project.comp(comp)?;
    let joints = match chain {
        None => ik_joints(c, layer),
        Some(n) => {
            let n = if n == 0 { 12 } else { n.min(12) } as usize;
            let mut out = Vec::new();
            let mut cur = c.layer(layer).and_then(|l| l.meta.parent);
            // Up to the first folder or group: those hold the drawing, they aren't joints.
            while let Some(p) = cur {
                let Some(pl) = c.layer(p) else { break };
                if out.len() >= n || out.contains(&p) || matches!(pl.kind, xenify_doc::LayerKind::Group) {
                    break;
                }
                out.push(p);
                cur = pl.meta.parent;
            }
            out
        }
    };
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

/// Commands that put `child` under `parent` (or at the top with `None`)
/// without moving it on screen: its position, rotation and scale become
/// what they need to be under the new parent (as fixed values).
pub fn place_under(
    project: &Project,
    comp: Id,
    child: Id,
    parent: Option<Id>,
    time: f64,
) -> Result<Vec<Command>, OpError> {
    let c = project.comp(comp)?;
    if let Some(parent) = parent
        && (parent == child || xenify_anim::descendants(c, child).contains(&parent))
    {
        return Err(OpError::Invalid("that would make a layer its own ancestor".into()));
    }
    let l = project.layer(child)?;
    let world = world_transform(c, child, time);
    let local = match parent {
        Some(p) => world_transform(c, p, time).inverse() * world,
        None => world,
    };
    let [a, b, cc, d, _, _] = local.as_coeffs();
    let theta = b.atan2(a);
    let (sx, sy) = (a.hypot(b), -cc * theta.sin() + d * theta.cos());
    let anchor = l.transform.anchor.value_at(time).to_point();
    let set = |path, property| Command::SetProperty { layer: child, path, property };
    Ok(vec![
        Command::SetLayerMeta { layer: child, meta: LayerMeta { parent, ..l.meta.clone() } },
        set(PropPath::Position, xenify_doc::AnyProperty::Vec2(Property::Static((local * anchor).to_vec2()))),
        set(PropPath::Rotation, xenify_doc::AnyProperty::Scalar(Property::Static(theta.to_degrees()))),
        set(PropPath::Scale, xenify_doc::AnyProperty::Vec2(Property::Static(Vec2::new(sx, sy)))),
    ])
}

/// Links `layers` (in order) into a joint chain: each is parented to the one
/// before it without moving on screen, and the last one drives the chain by IK.
pub fn make_joint_chain(project: &Project, comp: Id, layers: &[Id], time: f64) -> Result<Command, OpError> {
    if layers.len() < 2 {
        return Err(OpError::Invalid(
            "select 2 or more layers, from the root (e.g. upper arm) to the tip (hand)".into(),
        ));
    }
    let mut cmds = Vec::new();
    for w in layers.windows(2) {
        cmds.extend(place_under(project, comp, w[1], Some(w[0]), time)?);
    }
    // The tip drives the whole chain.
    let tip = project.layer(*layers.last().unwrap())?;
    let mut meta = tip.meta.clone();
    meta.parent = Some(layers[layers.len() - 2]);
    meta.rig = Rig { ik_chain: (layers.len() - 1) as u32 };
    cmds.push(Command::SetLayerMeta { layer: tip.id, meta });
    Ok(Command::batch(cmds))
}

/// Where each layer turns (its anchor, in composition space) and what it
/// hangs from: the bones the Bones tool draws.
pub fn bones(c: &Composition, time: f64) -> Vec<(Id, Point, Option<Id>)> {
    c.layers
        .iter()
        .filter(|l| l.meta.visible)
        .filter_map(|l| Some((l.id, anchor_world(c, l.id, time)?, l.meta.parent)))
        .collect()
}

/// Moves `layer`'s pivot (the point it turns and scales about) to `to`
/// (composition space) without moving its drawing: the anchor changes and
/// the position makes up for it (every keyframe of either, when animated).
pub fn move_pivot(project: &Project, comp: Id, layer: Id, to: Point, time: f64) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let l = project.layer(layer)?;
    let world = world_transform(c, layer, time);
    if world.determinant().abs() < 1e-12 {
        return Err(OpError::Invalid("the layer has no size to turn about".into()));
    }
    let a0 = l.transform.anchor.value_at(time);
    let a1 = (world.inverse() * to).to_vec2();
    let da = a1 - a0;
    // The layer's own turn, skew and scale (no translation): how far the position must move.
    let lin = l.transform.affine_at(time);
    let [m0, m1, m2, m3, _, _] = lin.as_coeffs();
    let dp = Vec2::new(m0 * da.x + m2 * da.y, m1 * da.x + m3 * da.y);
    let shift = |path: PropPath, prop: &Property<Vec2>, by: Vec2| -> Command {
        match prop {
            Property::Static(v) => {
                Command::SetProperty { layer, path, property: xenify_doc::AnyProperty::Vec2(Property::Static(*v + by)) }
            }
            Property::Animated(keys) => Command::batch(
                keys.iter()
                    .map(|k| Command::SetKeyframe {
                        layer,
                        path,
                        time: k.time,
                        value: Value::Vec2(k.value + by),
                        easing: None,
                    })
                    .collect(),
            ),
        }
    };
    Ok(Command::batch(vec![
        shift(PropPath::Anchor, &l.transform.anchor, da),
        shift(PropPath::Position, &l.transform.position, dp),
    ]))
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

/// Keys that send `layer` (its pivot) along the outline of `path` (a line, a
/// circle, any shape: its first outline) from `start` for `seconds`, at an
/// even speed. With `turn` it also turns to face along the path; `reverse`
/// goes the other way round. Its position (and rotation) keys are replaced.
#[allow(clippy::too_many_arguments)]
pub fn follow_path(
    project: &Project,
    comp: Id,
    layer: Id,
    path: Id,
    start: f64,
    seconds: f64,
    turn: bool,
    reverse: bool,
) -> Result<Command, OpError> {
    use kurbo::PathEl;
    use xenify_doc::{AnyProperty, Easing, Keyframe};
    if layer == path {
        return Err(OpError::Invalid("pick another layer as the path".into()));
    }
    let c = project.comp(comp)?;
    let l = project.layer(layer)?;
    let outline = world_transform(c, path, start) * xenify_anim::particles::layer_outline(c, path, start);
    // The first outline as a fine polyline (closed ones end where they began).
    let mut pts: Vec<Point> = Vec::new();
    let mut started = false;
    let mut stop = false;
    kurbo::flatten(outline.iter(), 0.25, |el| {
        if stop {
            return;
        }
        match el {
            PathEl::MoveTo(p) if !started => {
                started = true;
                pts.push(p);
            }
            PathEl::MoveTo(_) => stop = true,
            PathEl::LineTo(p) => pts.push(p),
            PathEl::ClosePath => {
                if let Some(&first) = pts.first() {
                    pts.push(first);
                }
                stop = true;
            }
            _ => {}
        }
    });
    if reverse {
        pts.reverse();
    }
    let mut along = vec![0.0];
    for w in pts.windows(2) {
        along.push(along.last().unwrap() + (w[1] - w[0]).hypot());
    }
    let total = *along.last().unwrap_or(&0.0);
    if pts.len() < 2 || total < 1e-6 {
        return Err(OpError::Invalid("the path layer has no line to follow (draw a line or a shape)".into()));
    }
    let point_at = |d: f64| -> Point {
        let k = along.partition_point(|&x| x < d).clamp(1, pts.len() - 1);
        let (a, b) = (along[k - 1], along[k]);
        let u = if b > a { ((d - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        pts[k - 1].lerp(pts[k], u)
    };
    let seconds = seconds.max(0.1);
    let fps = c.settings.fps.max(1.0);
    let n = ((seconds * fps).ceil() as usize).clamp(2, 1200);
    let parent = l.meta.parent.map_or(kurbo::Affine::IDENTITY, |p| world_transform(c, p, start)).inverse();
    let track: Vec<(f64, Point)> =
        (0..=n).map(|i| (start + seconds * i as f64 / n as f64, point_at(total * i as f64 / n as f64))).collect();
    let pos_keys: Vec<Keyframe<Vec2>> = track
        .iter()
        .map(|&(t, p)| Keyframe { time: t, value: (parent * p).to_vec2(), easing: Easing::Linear })
        .collect();
    let mut cmds = vec![Command::SetProperty {
        layer,
        path: PropPath::Position,
        property: AnyProperty::Vec2(Property::Animated(pos_keys)),
    }];
    if turn {
        // Facing along the path, measured from where it starts (so it keeps its own look).
        // (On a closed outline the ends meet, so the direction there wraps round.)
        let closed = (pts[0] - pts[pts.len() - 1]).hypot() < 1e-6;
        let h = (total / (4.0 * n as f64)).max(1e-3);
        let at = |d: f64| if closed { point_at(d.rem_euclid(total)) } else { point_at(d.clamp(0.0, total)) };
        let dir = |i: usize| {
            let d = total * i as f64 / n as f64;
            let (a, b) = (at(d - h), at(d + h));
            (b.y - a.y).atan2(b.x - a.x).to_degrees()
        };
        let r0 = l.transform.rotation.value_at(start);
        let a0 = dir(0);
        let mut last = a0;
        let mut keys = Vec::new();
        for (i, &(t, _)) in track.iter().enumerate() {
            let mut a = dir(i);
            while a - last > 180.0 {
                a -= 360.0;
            }
            while a - last < -180.0 {
                a += 360.0;
            }
            last = a;
            keys.push(Keyframe { time: t, value: r0 + a - a0, easing: Easing::Linear });
        }
        cmds.push(Command::SetProperty {
            layer,
            path: PropPath::Rotation,
            property: AnyProperty::Scalar(Property::Animated(keys)),
        });
    }
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

    #[test]
    fn moving_the_pivot_keeps_the_drawing_and_turns_about_the_new_point() {
        let (mut p, comp) = arm();
        let corner = |p: &Project| world_transform(p.comp(comp).unwrap(), Id(2), 0.0) * Point::new(10.0, 10.0);
        let before = corner(&p);
        let to = Point::new(230.0, 95.0);
        let cmd = move_pivot(&p, comp, Id(2), to, 0.0).unwrap();
        apply(&mut p, cmd).unwrap();
        assert!((corner(&p) - before).hypot() < 1e-6, "the drawing moved");
        assert!((anchor_world(p.comp(comp).unwrap(), Id(2), 0.0).unwrap() - to).hypot() < 1e-6);
    }

    #[test]
    fn detaching_keeps_the_layer_in_place() {
        let (mut p, comp) = arm();
        let bend = Command::batch(ik_commands(&p, comp, Id(3), Point::new(200.0, 220.0), 0.0, false).unwrap());
        apply(&mut p, bend).unwrap();
        let before = anchor_world(p.comp(comp).unwrap(), Id(3), 0.0).unwrap();
        let off = Command::batch(place_under(&p, comp, Id(3), None, 0.0).unwrap());
        apply(&mut p, off).unwrap();
        let c = p.comp(comp).unwrap();
        assert_eq!(c.layer(Id(3)).unwrap().meta.parent, None);
        assert!((anchor_world(c, Id(3), 0.0).unwrap() - before).hypot() < 1e-6);
        // A layer can't hang from what hangs from it.
        assert!(place_under(&p, comp, Id(1), Some(Id(2)), 0.0).is_err());
    }

    #[test]
    fn shift_drag_bends_every_parent() {
        let (p, comp) = arm();
        // The hand's own chain is 2; asking for all bends the same two here.
        let cmds = ik_commands_with(&p, comp, Id(3), Point::new(200.0, 220.0), 0.0, false, Some(0)).unwrap();
        assert_eq!(cmds.len(), 2);
        // The elbow alone: one joint (the shoulder).
        let cmds = ik_commands_with(&p, comp, Id(2), Point::new(150.0, 180.0), 0.0, false, Some(0)).unwrap();
        assert_eq!(cmds.len(), 1);
    }

    #[test]
    fn following_a_circle_goes_round_it() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let ring = Layer::shape(Id(1), "ring", vec![ShapeItem::ellipse(Vec2::new(200.0, 200.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 300.0));
        let dot = Layer::shape(Id(2), "dot", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)])
            .with_position(Vec2::new(50.0, 50.0));
        p.comp_mut(comp).unwrap().layers = vec![ring, dot];
        p.reserve_id(Id(10));
        let cmd = follow_path(&p, comp, Id(2), Id(1), 0.0, 2.0, true, false).unwrap();
        apply(&mut p, cmd).unwrap();
        let c = p.comp(comp).unwrap();
        for t in [0.0, 0.3, 0.9, 1.4, 2.0] {
            let at = anchor_world(c, Id(2), t).unwrap();
            let r = (at - Point::new(300.0, 300.0)).hypot();
            assert!((r - 100.0).abs() < 1.0, "off the circle at {t}: {r}");
        }
        // Back where it began after one lap.
        assert!((anchor_world(c, Id(2), 0.0).unwrap() - anchor_world(c, Id(2), 2.0).unwrap()).hypot() < 1.0);
        let turned = c.layer(Id(2)).unwrap().transform.rotation.value_at(2.0);
        assert!((turned.abs() - 360.0).abs() < 5.0, "turned {turned}");
    }
}
