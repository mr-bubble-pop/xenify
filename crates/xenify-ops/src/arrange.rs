//! Align and distribute layers by their on-screen bounding boxes.

use crate::{Command, OpError};
use kurbo::{Rect, Vec2};
use serde::{Deserialize, Serialize};
use xenify_anim::{TextMeasure, world_transform};
use xenify_doc::{Id, Project, PropPath, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignEdge {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignTo {
    /// The combined box of the layers (needs 2+ layers).
    Selection,
    /// The composition frame.
    Composition,
    /// A specific layer's box (it doesn't move).
    Layer(Id),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// On-screen boxes of `layers` at `time` (layers that aren't visible are skipped).
pub fn world_boxes(
    project: &Project,
    comp: Id,
    layers: &[Id],
    time: f64,
    measure: &mut dyn TextMeasure,
) -> Vec<(Id, Rect)> {
    let Ok(c) = project.comp(comp) else { return vec![] };
    let scene = xenify_anim::evaluate_project(project, c.id, time);
    layers.iter().filter_map(|id| Some((*id, xenify_anim::world_box(c, &scene, *id, measure)?))).collect()
}

/// Moves each layer by a screen-space `delta` (converted into its parent's space).
fn moves(
    project: &Project,
    comp: Id,
    deltas: &[(Id, Vec2)],
    time: f64,
    auto_key: bool,
) -> Result<Option<Command>, OpError> {
    let c = project.comp(comp)?;
    let mut cmds = Vec::new();
    for &(id, delta) in deltas {
        if delta.hypot() < 1e-9 {
            continue;
        }
        let layer = project.layer(id)?;
        let parent_inv = layer.meta.parent.map_or(kurbo::Affine::IDENTITY, |p| world_transform(c, p, time).inverse());
        let k = parent_inv.as_coeffs();
        let local = Vec2::new(k[0] * delta.x + k[2] * delta.y, k[1] * delta.x + k[3] * delta.y);
        let pos = layer.transform.position.value_at(time) + local;
        cmds.push(Command::edit_value(project, id, PropPath::Position, time, Value::Vec2(pos), auto_key)?);
    }
    Ok((!cmds.is_empty()).then(|| Command::batch(cmds)))
}

#[allow(clippy::too_many_arguments)]
/// Aligns the layers' edges or centres. With [`AlignTo::Selection`] and a
/// single layer, aligns to the composition instead.
pub fn align(
    project: &Project,
    comp: Id,
    layers: &[Id],
    edge: AlignEdge,
    to: AlignTo,
    time: f64,
    auto_key: bool,
    measure: &mut dyn TextMeasure,
) -> Result<Option<Command>, OpError> {
    let c = project.comp(comp)?;
    let boxes = world_boxes(project, comp, layers, time, measure);
    let frame = Rect::new(0.0, 0.0, c.settings.width as f64, c.settings.height as f64);
    let target = match to {
        AlignTo::Composition => frame,
        AlignTo::Selection if boxes.len() < 2 => frame,
        AlignTo::Selection => boxes.iter().map(|b| b.1).reduce(|a, b| a.union(b)).unwrap(),
        AlignTo::Layer(id) => world_boxes(project, comp, &[id], time, measure)
            .first()
            .map(|b| b.1)
            .ok_or_else(|| OpError::Invalid(format!("layer {id} isn't visible at this time")))?,
    };
    let deltas: Vec<(Id, Vec2)> = boxes
        .iter()
        .filter(|(id, _)| !matches!(to, AlignTo::Layer(t) if t == *id))
        .map(|&(id, b)| {
            let d = match edge {
                AlignEdge::Left => Vec2::new(target.x0 - b.x0, 0.0),
                AlignEdge::HCenter => Vec2::new(target.center().x - b.center().x, 0.0),
                AlignEdge::Right => Vec2::new(target.x1 - b.x1, 0.0),
                AlignEdge::Top => Vec2::new(0.0, target.y0 - b.y0),
                AlignEdge::VCenter => Vec2::new(0.0, target.center().y - b.center().y),
                AlignEdge::Bottom => Vec2::new(0.0, target.y1 - b.y1),
            };
            (id, d)
        })
        .collect();
    moves(project, comp, &deltas, time, auto_key)
}

/// Spaces 3+ layers so the gaps between their boxes are equal, keeping the
/// first and last in place.
pub fn distribute(
    project: &Project,
    comp: Id,
    layers: &[Id],
    axis: Axis,
    time: f64,
    auto_key: bool,
    measure: &mut dyn TextMeasure,
) -> Result<Option<Command>, OpError> {
    let mut boxes = world_boxes(project, comp, layers, time, measure);
    if boxes.len() < 3 {
        return Err(OpError::Invalid("distribute needs at least 3 visible layers".into()));
    }
    let span = |r: &Rect| match axis {
        Axis::Horizontal => (r.x0, r.x1),
        Axis::Vertical => (r.y0, r.y1),
    };
    boxes.sort_by(|a, b| span(&a.1).0.total_cmp(&span(&b.1).0));
    let start = span(&boxes[0].1).0;
    let end = span(&boxes.last().unwrap().1).1;
    let total: f64 = boxes.iter().map(|b| span(&b.1).1 - span(&b.1).0).sum();
    let gap = (end - start - total) / (boxes.len() - 1) as f64;
    let mut cursor = start;
    let deltas: Vec<(Id, Vec2)> = boxes
        .iter()
        .map(|&(id, b)| {
            let (a0, a1) = span(&b);
            let d = cursor - a0;
            cursor += (a1 - a0) + gap;
            let v = match axis {
                Axis::Horizontal => Vec2::new(d, 0.0),
                Axis::Vertical => Vec2::new(0.0, d),
            };
            (id, v)
        })
        .collect();
    moves(project, comp, &deltas, time, auto_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_anim::ApproxText;
    use xenify_doc::{Color, Layer, LayerKind, ShapeItem};

    fn setup() -> (Project, Id, Vec<Id>) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut ids = Vec::new();
        for (x, y, w) in [(100.0, 100.0, 50.0), (400.0, 300.0, 100.0), (1000.0, 200.0, 20.0)] {
            let id = p.alloc_id();
            ids.push(id);
            let l = Layer::shape(id, "r", vec![ShapeItem::rect(Vec2::new(w, w), Color::WHITE)])
                .with_position(Vec2::new(x, y));
            p.comp_mut(comp).unwrap().layers.push(l);
        }
        (p, comp, ids)
    }

    fn boxes(p: &Project, comp: Id, ids: &[Id]) -> Vec<Rect> {
        world_boxes(p, comp, ids, 0.0, &mut ApproxText).into_iter().map(|b| b.1).collect()
    }

    #[test]
    fn align_left_and_centre() {
        let (mut p, comp, ids) = setup();
        let cmd =
            align(&p, comp, &ids, AlignEdge::Left, AlignTo::Selection, 0.0, false, &mut ApproxText).unwrap().unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(boxes(&p, comp, &ids).iter().all(|b| (b.x0 - 75.0).abs() < 1e-9));

        let cmd = align(&p, comp, &ids, AlignEdge::HCenter, AlignTo::Composition, 0.0, false, &mut ApproxText)
            .unwrap()
            .unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(boxes(&p, comp, &ids).iter().all(|b| (b.center().x - 960.0).abs() < 1e-9));
        // Vertical positions untouched.
        assert_eq!(p.layer(ids[1]).unwrap().transform.position.value_at(0.0).y, 300.0);
    }

    #[test]
    fn align_to_a_key_layer_keeps_it_still() {
        let (mut p, comp, ids) = setup();
        let cmd = align(&p, comp, &ids, AlignEdge::Top, AlignTo::Layer(ids[1]), 0.0, false, &mut ApproxText)
            .unwrap()
            .unwrap();
        apply(&mut p, cmd).unwrap();
        let b = boxes(&p, comp, &ids);
        assert!(b.iter().all(|r| (r.y0 - 250.0).abs() < 1e-9), "{b:?}");
    }

    #[test]
    fn distribute_equal_gaps() {
        let (mut p, comp, ids) = setup();
        let cmd = distribute(&p, comp, &ids, Axis::Horizontal, 0.0, false, &mut ApproxText).unwrap().unwrap();
        apply(&mut p, cmd).unwrap();
        let mut b = boxes(&p, comp, &ids);
        b.sort_by(|a, c| a.x0.total_cmp(&c.x0));
        let g1 = b[1].x0 - b[0].x1;
        let g2 = b[2].x0 - b[1].x1;
        assert!((g1 - g2).abs() < 1e-9, "{g1} vs {g2}");
        assert!((b[0].x0 - 75.0).abs() < 1e-9 && (b[2].x1 - 1010.0).abs() < 1e-9);
        assert!(distribute(&p, comp, &ids[..2], Axis::Vertical, 0.0, false, &mut ApproxText).is_err());
    }

    #[test]
    fn parented_layers_move_in_screen_space() {
        let (mut p, comp, ids) = setup();
        let null = p.alloc_id();
        let mut n = Layer::new(null, "n", LayerKind::Null).with_position(Vec2::new(0.0, 0.0));
        n.transform.scale = Vec2::new(2.0, 2.0).into();
        p.comp_mut(comp).unwrap().layers.insert(0, n);
        p.layer_mut(ids[0]).unwrap().meta.parent = Some(null);
        let cmd = align(&p, comp, &[ids[0]], AlignEdge::Left, AlignTo::Composition, 0.0, false, &mut ApproxText)
            .unwrap()
            .unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(boxes(&p, comp, &[ids[0]])[0].x0.abs() < 1e-9);
    }
}
