//! Applying modifiers: turning what they make into ordinary layers.

use crate::transform::Snapshot;
use crate::{Command, OpError, invalid};
use kurbo::{Affine, Rect};
use std::collections::HashMap;
use xenify_doc::{Geometry, Id, LayerKind, Modifier, ModifierKind, Project, Property, Vec2};

/// The ready-made modifiers, as offered in the menus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Preset {
    Row,
    Radial,
    Grid,
    Jitter,
    ZigZag,
    Wave,
    PuckerBloat,
    Twist,
    Smooth,
    Subdivide,
    Simplify,
    OffsetPath,
    Mirror,
    PathArray,
    Blend,
    ShatterOut,
    ShatterIn,
}

impl Preset {
    /// Between two layers.
    pub const PAIRS: [Preset; 1] = [Preset::Blend];
    /// Outline changes.
    pub const SHAPES: [Preset; 9] = [
        Preset::Jitter,
        Preset::ZigZag,
        Preset::Wave,
        Preset::PuckerBloat,
        Preset::Twist,
        Preset::Smooth,
        Preset::Subdivide,
        Preset::Simplify,
        Preset::OffsetPath,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Row => "Array",
            Preset::Radial => "Radial Array",
            Preset::Grid => "Grid Array",
            Preset::Jitter => "Jitter",
            Preset::ZigZag => "Zig Zag",
            Preset::Wave => "Wave",
            Preset::PuckerBloat => "Pucker & Bloat",
            Preset::Twist => "Twist",
            Preset::Smooth => "Smooth",
            Preset::Subdivide => "Subdivide",
            Preset::Simplify => "Simplify",
            Preset::OffsetPath => "Offset Path",
            Preset::Mirror => "Mirror",
            Preset::PathArray => "Array Along Path",
            Preset::Blend => "Blend",
            Preset::ShatterOut => "Shatter Out",
            Preset::ShatterIn => "Shatter In",
        }
    }

    /// Whether it needs another layer (a path to follow, a shape to blend into).
    pub fn needs_target(self) -> bool {
        matches!(self, Preset::PathArray | Preset::Blend)
    }

    pub fn tip(self) -> &'static str {
        match self {
            Preset::Row => "Copies in a row, each one step on: set how far they move, turn, grow and fade",
            Preset::Radial => "Copies turned evenly around a centre (set the centre as the array's Origin)",
            Preset::Grid => "Copies in rows and columns (two arrays: one across, one down)",
            Preset::Jitter => "Moves points by random amounts (roughen); animate the seed to make it boil",
            Preset::ZigZag => "Sharp ridges along every edge",
            Preset::Wave => "Smooth waves along every edge",
            Preset::PuckerBloat => "Pucker (below 0) pulls edges in to a star; bloat (above 0) puffs them out",
            Preset::Twist => "Turns the middle more than the edges, like a whirlpool",
            Preset::Smooth => "Rounds every corner",
            Preset::Subdivide => "More points, same look (so later changes have points to move)",
            Preset::Simplify => "Fewer points, nearly the same look",
            Preset::OffsetPath => "Grows the shape outwards (or inwards with a negative distance)",
            Preset::Mirror => "A mirror image across an axis (drag the diamond to move the axis)",
            Preset::PathArray => {
                "Copies spread along another shape's outline (select this layer, then the path: Shift+click)"
            }
            Preset::Blend => "Steps that morph this shape into another (select it, then the other: Shift+click)",
            Preset::ShatterOut => {
                "Breaks into pieces that fly apart, from the playhead (drag the diamond: where it breaks)"
            }
            Preset::ShatterIn => "Pieces fly together into the layer, ending at the playhead",
        }
    }

    /// The modifiers this preset adds, sized to `bounds` (layer space);
    /// `target` is the other layer for those that need one; `time` is the
    /// playhead (where timed ones start or end).
    pub fn modifiers(self, bounds: Rect, target: Option<Id>, time: f64) -> Vec<Modifier> {
        let gap = |d: f64| d + (d * 0.1).max(10.0);
        let centre = bounds.center().to_vec2();
        let small = bounds.width().min(bounds.height()).max(10.0);
        let one = |kind| vec![Modifier::new(kind)];
        match self {
            Preset::Row => vec![Modifier::new(ModifierKind::array(5, Vec2::new(gap(bounds.width()), 0.0), centre))],
            Preset::Radial => {
                let radius = bounds.height().max(bounds.width()).max(20.0);
                vec![Modifier::new(ModifierKind::radial_array(8, centre + Vec2::new(0.0, radius)))]
            }
            Preset::Grid => vec![
                Modifier::new(ModifierKind::array(4, Vec2::new(gap(bounds.width()), 0.0), centre)),
                Modifier::new(ModifierKind::array(3, Vec2::new(0.0, gap(bounds.height())), centre)),
            ],
            Preset::Jitter => one(ModifierKind::Jitter {
                amount: (small * 0.04).max(2.0).into(),
                detail: 5.0.into(),
                seed: 0.0.into(),
                smooth: false,
            }),
            Preset::ZigZag | Preset::Wave => one(ModifierKind::ZigZag {
                amount: (small * 0.05).max(3.0).into(),
                ridges: 4.0.into(),
                smooth: self == Preset::Wave,
            }),
            Preset::PuckerBloat => one(ModifierKind::PuckerBloat { strength: (-0.3).into() }),
            Preset::Twist => one(ModifierKind::Twist { rotation: 60.0.into(), origin: centre.into() }),
            Preset::Smooth => one(ModifierKind::Smooth { strength: 1.0.into() }),
            Preset::Subdivide => one(ModifierKind::Subdivide { levels: 1.0.into() }),
            Preset::Simplify => one(ModifierKind::Simplify { tolerance: 1.0.into() }),
            Preset::OffsetPath => one(ModifierKind::OffsetPath {
                amount: (small * 0.1).max(5.0).into(),
                join: xenify_doc::LineJoin::Round,
            }),
            Preset::Mirror => one(ModifierKind::Mirror {
                origin: Vec2::new(bounds.x1, bounds.center().y).into(),
                rotation: 90.0.into(),
                merge: false,
            }),
            Preset::PathArray => match target {
                Some(path) => one(ModifierKind::PathArray {
                    path,
                    count: 8.0.into(),
                    start: 0.0.into(),
                    end: 1.0.into(),
                    align: true,
                }),
                None => Vec::new(),
            },
            Preset::Blend => match target {
                Some(target) => one(ModifierKind::Blend { target, count: 4.0.into() }),
                None => Vec::new(),
            },
            Preset::ShatterOut | Preset::ShatterIn => {
                one(crate::effects::shatter(bounds, time, self == Preset::ShatterOut))
            }
        }
    }
}

/// Makes `layer`'s first modifier permanent, as it is at `time`, and removes
/// it. An Array's copies become layers of their own (a group's copies bring
/// copies of its members), placed above the layer; a shape change is written
/// into the outlines of its shape layers (text and images keep theirs). Any
/// modifiers after it stay on the layer and on every copy. Only the first
/// one can be applied, because the later ones act on its result.
pub fn apply_first(project: &Project, comp: Id, layer: Id, time: f64) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let source = project.layer(layer)?;
    let modifier = source.meta.modifiers.first().ok_or_else(|| invalid("the layer has no modifiers to apply"))?;
    let mut meta = source.meta.clone();
    meta.modifiers.remove(0);
    let mut cmds = vec![Command::SetLayerMeta { layer, meta }];
    if !modifier.enabled {
        // Switched off, it makes nothing: applying it just removes it.
        return Ok(Command::batch(cmds));
    }
    if matches!(modifier.kind, xenify_doc::ModifierKind::Shatter { .. }) {
        return Err(invalid(
            "a shatter can't be applied: its pieces are cut from what the layer draws. Keyframe its progress instead",
        ));
    }
    // The layer and its members, in paint order.
    let mut members = xenify_anim::descendants(c, layer);
    members.push(layer);
    members.sort_by_key(|id| c.layer_index(*id));
    let top = members.iter().filter_map(|id| c.layer_index(*id)).max().unwrap_or(0);
    let world = xenify_anim::world_transform(c, layer, time);
    if world.determinant().abs() < 1e-12 {
        return Err(invalid("the layer is squashed flat, so its copies can't be placed"));
    }
    if let xenify_doc::Evaluated::Deform(d) = modifier.kind.at(time) {
        let plan = xenify_doc::CopyPlan {
            transform: Affine::IDENTITY,
            opacity: 1.0,
            deforms: vec![(Affine::IDENTITY, d)],
            clip: None,
        };
        let mut changed = false;
        for id in &members {
            let l = project.layer(*id)?;
            let LayerKind::Shape { shapes } = &l.kind else { continue };
            let to_space = world.inverse() * xenify_anim::world_transform(c, *id, time);
            for (index, s) in shapes.iter().enumerate() {
                let path = xenify_anim::geometry_path(&s.geometry, time);
                let path = xenify_anim::deform::deform_copy(&plan, &path, to_space);
                let shape = xenify_doc::ShapeItem { geometry: Geometry::Path { path: path.into() }, ..s.clone() };
                cmds.push(Command::SetShape { layer: *id, index, shape });
                changed = true;
            }
        }
        if !changed {
            return Err(invalid("only shape layers' outlines can take this change (text and images can't)"));
        }
        return Ok(Command::batch(cmds));
    }
    if let ModifierKind::Blend { .. } = &modifier.kind {
        // The steps in between become shape layers of their own, above it.
        let mut only = c.clone();
        if let Some(l) = only.layers.iter_mut().find(|l| l.id == layer) {
            l.meta.modifiers = vec![modifier.clone()];
        }
        let steps = xenify_anim::blend_drawings(&only, layer, time);
        if steps.is_empty() {
            return Err(invalid("a blend needs two shape layers"));
        }
        for (k, step) in steps.iter().enumerate() {
            let xenify_anim::SceneContent::Shapes(items) = &step.content else { continue };
            let shapes = items.iter().map(|i| xenify_anim::shape_from_item(i, "Blend")).collect();
            let mut l = xenify_doc::Layer::shape(
                Id(project.next_id + k as u64),
                format!("{} blend {}", source.meta.name, k + 1),
                shapes,
            );
            l.opacity = step.opacity.into();
            l.meta.blend = step.blend;
            cmds.push(Command::AddLayer { comp, index: Some(top + 1 + k), layer: l });
        }
        return Ok(Command::batch(cmds));
    }
    let plans = xenify_anim::modifiers_plan(c, source, std::slice::from_ref(modifier), time);
    // Build the copies in a scratch project, so each can be placed with the
    // same maths as dragging it on the canvas.
    let mut scratch = project.clone();
    crate::apply(&mut scratch, cmds[0].clone())?;
    // Copies on a path replace the layer itself: it moves to the first place.
    if let Some(first) = plans.first().filter(|p| p.transform != Affine::IDENTITY) {
        let place = Snapshot::new(&scratch, comp, layer, time, Rect::ZERO)?.affine_commands(
            &scratch,
            world * first.transform * world.inverse(),
            time,
            false,
        )?;
        for cmd in place {
            crate::apply(&mut scratch, cmd.clone())?;
            cmds.push(cmd);
        }
    }
    // Groups go in before their members (which need them as parents), each
    // at its place in paint order among what's in so far.
    let depth = |id: &Id| {
        let mut d = 0;
        let mut at = *id;
        while at != layer && d < members.len() {
            at = c.layer(at).and_then(|l| l.meta.parent).unwrap_or(layer);
            d += 1;
        }
        d
    };
    let mut adding = members.clone();
    adding.sort_by_key(depth);
    let mut next = project.next_id;
    let mut base = top + 1;
    let mut added = Vec::new();
    // (Copies are made from the layer as placed now, i.e. after the first step.)
    let first_step = plans.first().map_or(Affine::IDENTITY, |p| p.transform);
    for (step, fade) in plans.iter().skip(1).map(|p| (p.transform, p.opacity)) {
        let map: HashMap<Id, Id> = members
            .iter()
            .map(|id| {
                next += 1;
                (*id, Id(next - 1))
            })
            .collect();
        let inside = |id: Id| map.get(&id).copied().unwrap_or(id);
        let mut placed = Vec::new();
        for id in &adding {
            let mut l = scratch.layer(*id)?.clone();
            l.id = map[id];
            l.meta.parent = l.meta.parent.map(inside);
            if let Some(m) = &mut l.meta.matte {
                m.layer = inside(m.layer);
            }
            if let LayerKind::Text(t) = &mut l.kind
                && let Some(p) = &mut t.path
            {
                p.layer = inside(p.layer);
            }
            if *id == layer {
                // The later modifiers now act in the copy's own space: express
                // their offsets and origins there, so everything stays put.
                // (Exact unless a copy is turned and a later array scales
                // unevenly.)
                let back = step.inverse();
                let linear = |v: Vec2| {
                    let [a, b, c, d, _, _] = back.as_coeffs();
                    Vec2::new(a * v.x + c * v.y, b * v.x + d * v.y)
                };
                for m in &mut l.meta.modifiers {
                    if let ModifierKind::Array { offset, .. } = &mut m.kind {
                        map_values(offset, linear);
                    }
                    if let ModifierKind::Array { origin, .. }
                    | ModifierKind::Twist { origin, .. }
                    | ModifierKind::Mirror { origin, .. } = &mut m.kind
                    {
                        map_values(origin, |p| (back * p.to_point()).to_vec2());
                    }
                }
                match &mut l.opacity {
                    Property::Static(o) => *o *= fade,
                    Property::Animated(keys) => keys.iter_mut().for_each(|k| k.value *= fade),
                }
            }
            let order = |m: &Id| members.iter().position(|x| x == m);
            let index = base + placed.iter().filter(|p| order(p) < order(id)).count();
            crate::apply(&mut scratch, Command::AddLayer { comp, index: Some(index), layer: l })?;
            placed.push(*id);
            added.push((map[id], index));
        }
        base += members.len();
        let root = map[&layer];
        let place = Snapshot::new(&scratch, comp, root, time, Rect::ZERO)?.affine_commands(
            &scratch,
            world * step * first_step.inverse() * world.inverse(),
            time,
            false,
        )?;
        for cmd in place {
            crate::apply(&mut scratch, cmd)?;
        }
    }
    // The same inserts, with the copies as placed.
    for (id, index) in added {
        cmds.push(Command::AddLayer { comp, index: Some(index), layer: scratch.layer(id)?.clone() });
    }
    Ok(Command::batch(cmds))
}

fn map_values<T>(p: &mut Property<T>, f: impl Fn(T) -> T)
where
    T: Copy,
{
    match p {
        Property::Static(v) => *v = f(*v),
        Property::Animated(keys) => keys.iter_mut().for_each(|k| k.value = f(k.value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;
    use xenify_doc::{Color, Layer, Modifier, ShapeItem};

    #[test]
    fn applying_an_array_makes_real_layers_where_the_copies_were() {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let mut group = Layer::new(Id(10), "flower", LayerKind::Group).with_position(Vec2::new(300.0, 300.0));
        let mut arr = ModifierKind::radial_array(4, Vec2::new(0.0, 50.0));
        if let ModifierKind::Array { end_opacity, .. } = &mut arr {
            *end_opacity = 0.4.into();
        }
        group.meta.modifiers.push(Modifier::new(arr));
        group.meta.modifiers.push(Modifier::new(ModifierKind::array(2, Vec2::new(500.0, 0.0), Vec2::ZERO)));
        let mut petal = Layer::shape(Id(11), "petal", vec![ShapeItem::ellipse(Vec2::new(10.0, 30.0), Color::WHITE)]);
        petal.meta.parent = Some(Id(10));
        let other = Layer::shape(Id(12), "other", vec![ShapeItem::rect(Vec2::new(5.0, 5.0), Color::WHITE)]);
        {
            let c = project.comp_mut(comp).unwrap();
            c.layers = vec![petal, group, other];
        }
        project.next_id = 13;
        let before = xenify_anim::evaluate_project(&project, comp, 0.0);
        let cmd = apply_first(&project, comp, Id(10), 0.0).unwrap();
        crate::apply(&mut project, cmd).unwrap();
        let c = project.comp(comp).unwrap();
        assert_eq!(c.layers.len(), 9, "three more groups with a petal each");
        assert_eq!(c.layers.last().unwrap().id, Id(12), "copies go right above the original");
        assert_eq!(project.layer(Id(10)).unwrap().meta.modifiers.len(), 1, "the second array stays");
        // Paint order: each copy's petal, then its group.
        let (petal, group) = (&c.layers[2], &c.layers[3]);
        assert_eq!((petal.meta.parent, &group.kind), (Some(group.id), &LayerKind::Group));
        assert_eq!(petal.meta.modifiers.len(), 0, "the petal has none of its own");
        assert_eq!(group.meta.modifiers.len(), 1, "each copy keeps the later modifiers");
        // Everything is drawn where it was.
        let after = xenify_anim::evaluate_project(&project, comp, 0.0);
        let centres = |s: &xenify_anim::Scene| {
            let mut v: Vec<(i64, i64)> = Vec::new();
            fn walk(ls: &[xenify_anim::SceneLayer], v: &mut Vec<(i64, i64)>) {
                for l in ls {
                    match &l.content {
                        xenify_anim::SceneContent::Group(inner) => walk(inner, v),
                        _ => {
                            let p = l.transform * Point::ZERO;
                            v.push((p.x.round() as i64, p.y.round() as i64));
                        }
                    }
                }
            }
            walk(&s.layers, &mut v);
            v.sort();
            v
        };
        assert_eq!(centres(&before), centres(&after));
        let last_group = c.layers.iter().rev().find(|l| matches!(l.kind, LayerKind::Group)).unwrap();
        assert!((last_group.opacity.value_at(0.0) - 0.4).abs() < 1e-9);
    }

    #[test]
    fn applying_a_shape_change_rewrites_the_outlines() {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let mut l = Layer::shape(Id(1), "a", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE)]);
        l.meta.modifiers.push(Modifier::new(ModifierKind::ZigZag {
            amount: 5.0.into(),
            ridges: 2.0.into(),
            smooth: false,
        }));
        l.meta.modifiers.push(Modifier::new(ModifierKind::array(2, Vec2::new(150.0, 0.0), Vec2::ZERO)));
        project.comp_mut(comp).unwrap().layers.push(l);
        let before = xenify_anim::evaluate_project(&project, comp, 0.0);
        let cmd = apply_first(&project, comp, Id(1), 0.0).unwrap();
        crate::apply(&mut project, cmd).unwrap();
        let l = project.layer(Id(1)).unwrap();
        assert!(
            matches!(l.kind, LayerKind::Shape { ref shapes } if matches!(shapes[0].geometry, Geometry::Path { .. }))
        );
        assert_eq!(l.meta.modifiers.len(), 1, "the array stays");
        assert_eq!(xenify_anim::evaluate_project(&project, comp, 0.0), before, "looks the same");
    }

    #[test]
    fn applying_a_path_array_or_a_blend_keeps_the_picture() {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let line = Layer::shape(
            Id(1),
            "line",
            vec![ShapeItem::path(
                kurbo::Shape::to_path(&kurbo::Line::new((0.0, 0.0), (300.0, 100.0)), 0.1),
                None,
                Some((Color::WHITE, 1.0)),
            )],
        )
        .with_position(Vec2::new(100.0, 100.0));
        let mut dot = Layer::shape(Id(2), "dot", vec![ShapeItem::rect(Vec2::new(10.0, 6.0), Color::WHITE)]);
        dot.meta.modifiers.push(Modifier::new(ModifierKind::PathArray {
            path: Id(1),
            count: 3.0.into(),
            start: 0.0.into(),
            end: 1.0.into(),
            align: true,
        }));
        let a = Layer::shape(Id(3), "a", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)])
            .with_position(Vec2::new(0.0, 500.0));
        let b = Layer::shape(Id(4), "b", vec![ShapeItem::ellipse(Vec2::new(40.0, 40.0), Color::BLACK)])
            .with_position(Vec2::new(400.0, 500.0));
        let mut a = a;
        a.meta.modifiers.push(Modifier::new(ModifierKind::Blend { target: Id(4), count: 4.0.into() }));
        project.comp_mut(comp).unwrap().layers = vec![line, dot, a, b];
        project.next_id = 10;
        let centres = |p: &Project| {
            let scene = xenify_anim::evaluate_project(p, comp, 0.0);
            let mut measure = xenify_anim::ApproxText;
            let mut v: Vec<(i64, i64)> = scene
                .layers
                .iter()
                .filter_map(|l| l.bounds(&mut measure).map(|b| l.transform.transform_rect_bbox(b).center()))
                .map(|c| (c.x.round() as i64, c.y.round() as i64))
                .collect();
            v.sort();
            v
        };
        let before = centres(&project);
        for id in [Id(2), Id(3)] {
            let cmd = apply_first(&project, comp, id, 0.0).unwrap();
            crate::apply(&mut project, cmd).unwrap();
        }
        let c = project.comp(comp).unwrap();
        assert_eq!(c.layers.len(), 4 + 2 + 4, "two more dots, four blend steps");
        assert_eq!(centres(&project), before, "everything is drawn where it was");
    }

    #[test]
    fn only_a_layer_with_modifiers_can_apply_one() {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let l = Layer::shape(Id(1), "a", vec![ShapeItem::rect(Vec2::new(5.0, 5.0), Color::WHITE)]);
        project.comp_mut(comp).unwrap().layers.push(l);
        assert!(apply_first(&project, comp, Id(1), 0.0).is_err());
    }
}
