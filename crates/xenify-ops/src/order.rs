//! Stacking order: bring to front / send to back / forward / backward, and
//! moving layers above, below or into another layer (Layers panel drags).
//!
//! A group moves with its members, which always sit directly below it in the
//! composition's list (see [`crate::group`]); a layer inside a group stays
//! within its group's block unless it's moved into another one.

use crate::{Command, OpError};
use std::collections::HashMap;
use xenify_doc::{AnyProperty, Composition, Id, Keyframe, LayerKind, LayerMeta, Project, PropPath, Property, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    Front,
    Back,
    Forward,
    Backward,
    /// Just above `Id` (in its group).
    Above(Id),
    /// Just below `Id` (in its group).
    Below(Id),
    /// On top inside group `Id`.
    IntoGroup(Id),
}

/// The command moving `layers` to `placement` in composition `comp`, or
/// `None` if nothing would change.
pub fn move_layers(
    project: &Project,
    comp: Id,
    layers: &[Id],
    placement: Placement,
) -> Result<Option<Command>, OpError> {
    let c = project.comp(comp)?;
    for id in layers {
        if c.layer(*id).is_none() {
            return Err(OpError::Invalid(format!("no layer {id} in this composition")));
        }
    }
    let tree = Tree::new(c);
    // Only the top of each selected hierarchy moves (members come along).
    let mut movers: Vec<Id> =
        layers.iter().copied().filter(|id| !tree.ancestors(*id).any(|a| layers.contains(&a))).collect();
    movers.sort_by_key(|id| c.layer_index(*id));
    movers.dedup();
    if movers.is_empty() {
        return Ok(None);
    }

    let mut scopes = tree.units.clone();
    let mut cmds = Vec::new();
    match placement {
        Placement::Front | Placement::Back => {
            for scope in scopes.values_mut() {
                let (picked, rest): (Vec<Id>, Vec<Id>) = scope.iter().partition(|u| movers.contains(u));
                *scope = if placement == Placement::Front { [rest, picked].concat() } else { [picked, rest].concat() };
            }
        }
        Placement::Forward | Placement::Backward => {
            for scope in scopes.values_mut() {
                let n = scope.len();
                let mut idx: Vec<usize> = (0..n).filter(|i| movers.contains(&scope[*i])).collect();
                if placement == Placement::Forward {
                    idx.reverse();
                }
                for i in idx {
                    let j = if placement == Placement::Forward { i + 1 } else { i.wrapping_sub(1) };
                    // Blocked by the edge or by another moving layer.
                    if j < n && !movers.contains(&scope[j]) {
                        scope.swap(i, j);
                    }
                }
            }
        }
        Placement::Above(target) | Placement::Below(target) | Placement::IntoGroup(target) => {
            if movers.contains(&target) || tree.ancestors(target).any(|a| movers.contains(&a)) {
                return Err(OpError::Invalid("can't move a layer into itself".into()));
            }
            let (new_scope, at) = match placement {
                Placement::IntoGroup(g) => {
                    if !matches!(c.layer(g).map(|l| &l.kind), Some(LayerKind::Group)) {
                        return Err(OpError::Invalid("layers can only be moved into a group".into()));
                    }
                    (Some(g), None)
                }
                _ => (tree.scope[&target], Some(target)),
            };
            for id in &movers {
                let old = tree.scope[id];
                scopes.get_mut(&old).unwrap().retain(|u| u != id);
                if old != new_scope {
                    cmds.extend(reparent(project, c, *id, new_scope)?);
                }
            }
            let list = scopes.entry(new_scope).or_default();
            let pos = match (placement, at) {
                (Placement::Above(_), Some(t)) => list.iter().position(|u| *u == t).map_or(list.len(), |p| p + 1),
                (Placement::Below(_), Some(t)) => list.iter().position(|u| *u == t).unwrap_or(0),
                _ => list.len(),
            };
            for (k, id) in movers.iter().enumerate() {
                list.insert(pos + k, *id);
            }
        }
    }

    let new_order = flatten(&scopes, None, c);
    let old_order: Vec<Id> = c.layers.iter().map(|l| l.id).collect();
    if new_order == old_order && cmds.is_empty() {
        return Ok(None);
    }
    // Build the new order bottom-up: moving each layer to its final index in
    // turn leaves everything below it settled.
    let mut current = old_order;
    for (index, id) in new_order.iter().enumerate() {
        let from = current.iter().position(|x| x == id).expect("same layers");
        if from != index {
            current.remove(from);
            current.insert(index, *id);
            cmds.push(Command::MoveLayer { layer: *id, index });
        }
    }
    Ok(Some(Command::batch(cmds)))
}

/// Turns every null layer (an invisible parent, from older projects) into a
/// group: the layers attached to it become its members, stacked directly
/// below it like any group's. Returns whether anything changed.
pub fn nulls_to_groups(project: &mut Project) -> bool {
    let mut changed = false;
    for c in &mut project.compositions {
        if !c.layers.iter().any(|l| matches!(l.kind, LayerKind::Null)) {
            continue;
        }
        for l in &mut c.layers {
            if matches!(l.kind, LayerKind::Null) {
                l.kind = LayerKind::Group;
            }
        }
        let tree = Tree::new(c);
        let order = flatten(&tree.units, None, c);
        let mut by_id: std::collections::HashMap<Id, xenify_doc::Layer> =
            c.layers.drain(..).map(|l| (l.id, l)).collect();
        c.layers = order.iter().filter_map(|id| by_id.remove(id)).collect();
        // Anything the tree missed (it shouldn't) stays, on top.
        c.layers.extend(by_id.into_values());
        changed = true;
    }
    changed
}

/// Groups and their members, by stacking scope.
struct Tree {
    /// Each layer's group (`None`: top level).
    scope: HashMap<Id, Option<Id>>,
    parent: HashMap<Id, Option<Id>>,
    /// The units (a layer, or a group with its members) of each scope, bottom first.
    units: HashMap<Option<Id>, Vec<Id>>,
}

impl Tree {
    fn new(c: &Composition) -> Self {
        let parent: HashMap<Id, Option<Id>> = c.layers.iter().map(|l| (l.id, l.meta.parent)).collect();
        let is_group = |id: Id| matches!(c.layer(id).map(|l| &l.kind), Some(LayerKind::Group));
        let mut scope = HashMap::new();
        let mut units: HashMap<Option<Id>, Vec<Id>> = HashMap::new();
        for l in &c.layers {
            // The nearest group up the parent chain (parents can be nulls).
            let mut s = l.meta.parent;
            let mut steps = 0;
            while let Some(p) = s {
                if is_group(p) || steps > 64 {
                    break;
                }
                s = parent.get(&p).copied().flatten();
                steps += 1;
            }
            let s = s.filter(|p| is_group(*p));
            scope.insert(l.id, s);
            units.entry(s).or_default().push(l.id);
        }
        Self { scope, parent, units }
    }

    fn ancestors(&self, id: Id) -> impl Iterator<Item = Id> + '_ {
        let mut cur = self.parent.get(&id).copied().flatten();
        let mut steps = 0;
        std::iter::from_fn(move || {
            let p = cur?;
            steps += 1;
            if steps > 64 {
                return None;
            }
            cur = self.parent.get(&p).copied().flatten();
            Some(p)
        })
    }
}

/// Paint order from the scopes: each group's members right below it.
fn flatten(scopes: &HashMap<Option<Id>, Vec<Id>>, scope: Option<Id>, c: &Composition) -> Vec<Id> {
    let mut out = Vec::new();
    for id in scopes.get(&scope).map_or(&[][..], Vec::as_slice) {
        if matches!(c.layer(*id).map(|l| &l.kind), Some(LayerKind::Group)) {
            out.extend(flatten(scopes, Some(*id), c));
        }
        out.push(*id);
    }
    out
}

/// Moves `id` into group `to` (or out to the top level) without it moving on
/// screen: its position keys shift by the difference between the groups'
/// origins (groups are placed by position only).
fn reparent(project: &Project, c: &Composition, id: Id, to: Option<Id>) -> Result<Vec<Command>, OpError> {
    let origin = |g: Option<Id>| -> Vec2 {
        let mut total = Vec2::ZERO;
        let mut cur = g;
        while let Some(p) = cur {
            let Some(l) = c.layer(p) else { break };
            total += l.transform.position.value_at(0.0) - l.transform.anchor.value_at(0.0);
            cur = l.meta.parent;
        }
        total
    };
    let l = project.layer(id)?;
    let old_parent = l.meta.parent;
    let delta = origin(old_parent) - origin(to);
    let shifted = match &l.transform.position {
        Property::Static(v) => Property::Static(*v + delta),
        Property::Animated(keys) => {
            Property::Animated(keys.iter().map(|k| Keyframe { value: k.value + delta, ..k.clone() }).collect())
        }
    };
    Ok(vec![
        Command::SetProperty { layer: id, path: PropPath::Position, property: AnyProperty::Vec2(shifted) },
        Command::SetLayerMeta { layer: id, meta: LayerMeta { parent: to, ..l.meta.clone() } },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Color, Layer, ShapeItem};

    /// Bottom → top: a, [m1, m2, G], b, c
    fn fixture() -> (Project, Id, [Id; 6]) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let ids = [Id(10), Id(11), Id(12), Id(13), Id(14), Id(15)];
        let [a, m1, m2, g, b, c] = ids;
        let shape = |id, name| Layer::shape(id, name, vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        let member = |id, name| {
            let mut l = shape(id, name);
            l.meta.parent = Some(g);
            l
        };
        let layers = vec![
            shape(a, "a"),
            member(m1, "m1"),
            member(m2, "m2"),
            Layer::new(g, "G", LayerKind::Group).with_position(Vec2::new(100.0, 50.0)),
            shape(b, "b"),
            shape(c, "c"),
        ];
        p.comp_mut(comp).unwrap().layers = layers;
        p.next_id = 100;
        (p, comp, ids)
    }

    fn order_after(p: &Project, comp: Id, layers: &[Id], pl: Placement) -> Vec<Id> {
        let mut q = p.clone();
        if let Some(cmd) = move_layers(p, comp, layers, pl).unwrap() {
            let undo = apply(&mut q, cmd).unwrap();
            let mut back = q.clone();
            apply(&mut back, undo).unwrap();
            assert_eq!(back, *p, "undo restores the original");
        }
        q.comp(comp).unwrap().layers.iter().map(|l| l.id).collect()
    }

    #[test]
    fn nulls_become_groups_with_their_children_below() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let shape = |id, name| Layer::shape(id, name, vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        let (a, n, b, c) = (Id(10), Id(11), Id(12), Id(13));
        let mut child = shape(c, "child");
        child.meta.parent = Some(n);
        // Bottom → top: child, a, null, b.
        p.comp_mut(comp).unwrap().layers =
            vec![child, shape(a, "a"), Layer::new(n, "null", LayerKind::Null), shape(b, "b")];
        assert!(nulls_to_groups(&mut p));
        let c0 = p.comp(comp).unwrap();
        let order: Vec<Id> = c0.layers.iter().map(|l| l.id).collect();
        assert_eq!(order, [a, c, n, b], "the child sits right below its group");
        assert!(matches!(c0.layer(n).unwrap().kind, LayerKind::Group));
        assert!(!nulls_to_groups(&mut p), "nothing left to convert");
    }

    #[test]
    fn groups_move_with_their_members() {
        let (p, comp, [a, m1, m2, g, b, c]) = fixture();
        assert_eq!(order_after(&p, comp, &[g], Placement::Front), [a, b, c, m1, m2, g]);
        assert_eq!(order_after(&p, comp, &[g], Placement::Back), [m1, m2, g, a, b, c]);
        assert_eq!(order_after(&p, comp, &[g], Placement::Forward), [a, b, m1, m2, g, c]);
        assert_eq!(order_after(&p, comp, &[b], Placement::Backward), [a, b, m1, m2, g, c]);
        // Selecting a member with its group moves the group once.
        assert_eq!(order_after(&p, comp, &[g, m1], Placement::Back), [m1, m2, g, a, b, c]);
    }

    #[test]
    fn members_stay_inside_their_group() {
        let (p, comp, [a, m1, m2, g, b, c]) = fixture();
        assert_eq!(order_after(&p, comp, &[m1], Placement::Front), [a, m2, m1, g, b, c]);
        assert_eq!(order_after(&p, comp, &[m2], Placement::Forward), [a, m1, m2, g, b, c], "already on top: no change");
        assert!(move_layers(&p, comp, &[c], Placement::Front).unwrap().is_none());
        // b and c can't move up (c is on top, b is blocked by c); a hops over the group.
        assert_eq!(order_after(&p, comp, &[a, b, c], Placement::Forward), [m1, m2, g, a, b, c]);
    }

    #[test]
    fn several_layers_keep_their_relative_order() {
        let (p, comp, [a, m1, m2, g, b, c]) = fixture();
        assert_eq!(order_after(&p, comp, &[a, b], Placement::Front), [m1, m2, g, c, a, b]);
        assert_eq!(order_after(&p, comp, &[c, b], Placement::Back), [b, c, a, m1, m2, g]);
    }

    #[test]
    fn dragging_into_and_out_of_groups_keeps_layers_in_place() {
        let (p, comp, [a, m1, m2, g, b, c]) = fixture();
        let mut q = p.clone();
        apply(&mut q, move_layers(&p, comp, &[c], Placement::IntoGroup(g)).unwrap().unwrap()).unwrap();
        let order: Vec<Id> = q.comp(comp).unwrap().layers.iter().map(|l| l.id).collect();
        assert_eq!(order, [a, m1, m2, c, g, b]);
        let moved = q.layer(c).unwrap();
        assert_eq!(moved.meta.parent, Some(g));
        // The group sits at (100, 50): the member's local position compensates.
        assert_eq!(moved.transform.position.value_at(0.0), Vec2::new(-100.0, -50.0));

        let out = move_layers(&q, comp, &[m1], Placement::Above(b)).unwrap().unwrap();
        apply(&mut q, out).unwrap();
        let order: Vec<Id> = q.comp(comp).unwrap().layers.iter().map(|l| l.id).collect();
        assert_eq!(order, [a, m2, c, g, b, m1]);
        assert_eq!(q.layer(m1).unwrap().meta.parent, None);
        assert_eq!(q.layer(m1).unwrap().transform.position.value_at(0.0), Vec2::new(100.0, 50.0));

        assert!(move_layers(&p, comp, &[g], Placement::IntoGroup(g)).is_err());
        assert!(move_layers(&p, comp, &[a], Placement::IntoGroup(b)).is_err());
        assert_eq!(order_after(&p, comp, &[a], Placement::Below(m2)), [m1, a, m2, g, b, c]);
    }
}
