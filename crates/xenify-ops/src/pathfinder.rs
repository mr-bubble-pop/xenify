//! Pathfinder: combine the filled shapes of several layers into one path
//! (unite, minus front, intersect, exclude), like Illustrator.

use crate::{Command, OpError};
use kurbo::{Affine, BezPath, Shape};
use linesweeper::{BinaryOp, FillRule as LsRule};
use serde::{Deserialize, Serialize};
use xenify_anim::SceneContent;
use xenify_doc::{FillRule, Id, Layer, LayerKind, Project, ShapeItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathOp {
    /// Everything covered by any shape.
    #[serde(rename = "union", alias = "unite")]
    Unite,
    /// The bottom shape with everything above it cut away.
    #[serde(rename = "difference", alias = "minus_front")]
    MinusFront,
    /// Only where all shapes overlap.
    #[serde(rename = "intersection", alias = "intersect")]
    Intersect,
    /// Where an odd number of shapes overlap (overlaps become holes).
    #[serde(rename = "exclusion", alias = "exclude")]
    Exclude,
}

impl PathOp {
    pub const ALL: [(PathOp, &'static str); 4] = [
        (PathOp::Unite, "Union"),
        (PathOp::MinusFront, "Difference"),
        (PathOp::Intersect, "Intersection"),
        (PathOp::Exclude, "Exclusion"),
    ];
}

/// The filled area of a layer at `time`, in composition space.
fn filled_area(project: &Project, comp: Id, layer: Id, time: f64) -> Result<BezPath, OpError> {
    let c = project.comp(comp)?;
    let scene = xenify_anim::evaluate_project(project, c.id, time);
    let l = scene
        .layers
        .iter()
        .find(|l| l.id == layer)
        .ok_or_else(|| OpError::Invalid(format!("layer {layer} isn't visible at this time")))?;
    let SceneContent::Shapes(items) = &l.content else {
        return Err(OpError::Invalid("only shape layers can be combined (not text)".into()));
    };
    let mut out = BezPath::new();
    for item in items.iter().filter(|i| i.fill.is_some()) {
        // Normalise each shape on its own so its fill rule is honoured.
        let rule = match item.fill_rule {
            FillRule::NonZero => LsRule::NonZero,
            FillRule::EvenOdd => LsRule::EvenOdd,
        };
        let world = l.transform * item.path.clone();
        let closed = close_all(&world);
        out.extend(contours(linesweeper::binary_op(&closed, &BezPath::new(), rule, BinaryOp::Union))?);
    }
    Ok(out)
}

/// linesweeper wants closed subpaths; close any open ones.
fn close_all(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in path.elements() {
        match el {
            kurbo::PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
                out.push(*el);
            }
            kurbo::PathEl::ClosePath => {
                open = false;
                out.push(*el);
            }
            _ => out.push(*el),
        }
    }
    if open {
        out.close_path();
    }
    out
}

fn contours(r: Result<linesweeper::topology::Contours, linesweeper::Error>) -> Result<BezPath, OpError> {
    let c = r.map_err(|e| OpError::Invalid(format!("path operation failed: {e}")))?;
    let mut out = BezPath::new();
    for contour in c.contours() {
        out.extend(contour.path.iter());
    }
    Ok(out)
}

/// Combines the (2+) layers into a new path layer that replaces them.
/// Returns the command and the new layer's id.
pub fn combine(project: &Project, comp: Id, layers: &[Id], op: PathOp, time: f64) -> Result<(Command, Id), OpError> {
    if layers.len() < 2 {
        return Err(OpError::Invalid("select at least two shapes to combine".into()));
    }
    let c = project.comp(comp)?;
    // Stacking order: bottom first.
    let mut ordered: Vec<(usize, Id)> = layers
        .iter()
        .map(|id| c.layer_index(*id).map(|i| (i, *id)).ok_or(OpError::Invalid(format!("no layer {id}"))))
        .collect::<Result<_, _>>()?;
    ordered.sort();
    let areas: Vec<BezPath> =
        ordered.iter().map(|(_, id)| filled_area(project, comp, *id, time)).collect::<Result<_, _>>()?;

    let ls_op = match op {
        PathOp::Unite => BinaryOp::Union,
        PathOp::MinusFront => BinaryOp::Difference,
        PathOp::Intersect => BinaryOp::Intersection,
        PathOp::Exclude => BinaryOp::Xor,
    };
    let mut acc = areas[0].clone();
    for next in &areas[1..] {
        // Normalised contours are simple and nested: even-odd is exact.
        acc = contours(linesweeper::binary_op(&acc, next, LsRule::EvenOdd, ls_op))?;
    }
    if acc.elements().is_empty() {
        return Err(OpError::Invalid(match op {
            PathOp::Intersect => "the shapes don't overlap".into(),
            _ => "the result is empty".into(),
        }));
    }

    // Style: the bottom shape for Minus Front (it's what remains), else the top.
    let style_layer = if op == PathOp::MinusFront { ordered[0].1 } else { ordered.last().unwrap().1 };
    let style = project.layer(style_layer)?;
    let template = style.shapes().iter().find(|s| s.fill.is_some()).cloned();
    let centre = acc.bounding_box().center().to_vec2();
    let local = Affine::translate(-centre) * acc;
    let mut shape = ShapeItem::path(local, None, None);
    if let Some(t) = template {
        shape.fill = t.fill.map(|f| xenify_doc::Fill {
            color: f.color.value_at(time).into(),
            gradient: f.gradient,
            pattern: f.pattern,
        });
        shape.stroke = t.stroke;
    }
    shape.fill_rule = FillRule::EvenOdd;
    shape.name = op_name(op).into();
    let top_index = ordered.last().unwrap().0;
    let new_id = Id(project.next_id);
    let mut layer =
        Layer::shape(new_id, format!("{} {}", op_name(op), project.layer(style_layer)?.meta.name), vec![shape])
            .with_position(centre);
    layer.meta.in_point = style.meta.in_point;
    layer.meta.out_point = style.meta.out_point;

    let mut cmds = vec![Command::AddLayer { comp, index: Some(top_index + 1), layer }];
    cmds.extend(ordered.iter().map(|(_, id)| Command::RemoveLayer { layer: *id }));
    Ok((Command::batch(cmds), new_id))
}

fn op_name(op: PathOp) -> &'static str {
    PathOp::ALL.iter().find(|(o, _)| *o == op).unwrap().1
}

/// Whether a layer can take part in a path operation.
pub fn can_combine(layer: &Layer) -> bool {
    matches!(&layer.kind, LayerKind::Shape { shapes } if shapes.iter().any(|s| s.fill.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use kurbo::Point;
    use xenify_doc::{Color, Geometry, Vec2};

    /// Two overlapping 100×100 squares: A at (100,100), B at (150,100).
    fn setup() -> (Project, Id, Id, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let a = p.alloc_id();
        let b = p.alloc_id();
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let la = Layer::shape(a, "A", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), red)])
            .with_position(Vec2::new(100.0, 100.0));
        let lb = Layer::shape(b, "B", vec![ShapeItem::ellipse(Vec2::new(100.0, 100.0), blue)])
            .with_position(Vec2::new(150.0, 100.0));
        p.comp_mut(comp).unwrap().layers.extend([la, lb]);
        (p, comp, a, b)
    }

    fn result_area(p: &Project, id: Id) -> (f64, kurbo::Rect) {
        let l = p.layer(id).unwrap();
        let path = match &l.shapes()[0].geometry {
            Geometry::Path { path } => path.value_at(0.0),
            _ => panic!(),
        };
        let world = Affine::translate(l.transform.position.value_at(0.0)) * path;
        // Signed areas of nested contours; sum of absolute values is fine here
        // because none of these results has holes except Exclude.
        (world.area().abs(), world.bounding_box())
    }

    #[test]
    fn unite_minus_intersect_exclude() {
        let circle = std::f64::consts::PI * 50.0 * 50.0;
        // (operation, expected x extent of the result)
        for (op, (x0, x1)) in
            [(PathOp::Unite, (50.0, 200.0)), (PathOp::MinusFront, (50.0, 150.0)), (PathOp::Intersect, (100.0, 150.0))]
        {
            let (mut p, comp, a, b) = setup();
            let (cmd, id) = combine(&p, comp, &[a, b], op, 0.0).unwrap();
            let before = p.clone();
            let inverse = apply(&mut p, cmd).unwrap();
            // Sources replaced by one layer.
            let layers = &p.compositions[0].layers;
            assert_eq!(layers.len(), 1, "{op:?}");
            let (_, bounds) = result_area(&p, id);
            assert!((bounds.x0 - x0).abs() < 0.5 && (bounds.x1 - x1).abs() < 0.5, "{op:?}: {bounds:?}");
            if op == PathOp::Unite {
                // Overlap of square [50,150]² and circle centred (150,100) r=50 is half the circle.
                let expected = 10000.0 + circle / 2.0;
                let (area, _) = result_area(&p, id);
                assert!((area - expected).abs() / expected < 0.01, "{area} vs {expected}");
            }
            // One undo restores the originals.
            apply(&mut p, inverse).unwrap();
            let mut restored = p.clone();
            restored.next_id = before.next_id;
            assert_eq!(restored, before);
        }
        // Exclude creates a hole where they overlap: the overlap point isn't filled.
        let (mut p, comp, a, b) = setup();
        let (cmd, _) = combine(&p, comp, &[a, b], PathOp::Exclude, 0.0).unwrap();
        apply(&mut p, cmd).unwrap();
        let scene = xenify_anim::evaluate(&p.compositions[0], 0.0);
        let hit = |x: f64| {
            xenify_anim::layer_contains(&scene.layers[0], Point::new(x, 100.0), 0.0, &mut xenify_anim::ApproxText)
        };
        assert!(hit(80.0) && hit(180.0) && !hit(125.0));
    }

    #[test]
    fn needs_two_shape_layers_that_overlap_for_intersect() {
        let (mut p, comp, a, b) = setup();
        assert!(combine(&p, comp, &[a], PathOp::Unite, 0.0).is_err());
        p.layer_mut(b).unwrap().transform.position = Vec2::new(900.0, 900.0).into();
        assert!(combine(&p, comp, &[a, b], PathOp::Intersect, 0.0).is_err());
    }
}

/// Knife: cuts each of `layers` whose filled shape the line from `a` to `b`
/// (composition space) crosses right through into two layers, one on each
/// side of the line. Returns the command and the new layers' ids.
pub fn knife(
    project: &Project,
    comp: Id,
    layers: &[Id],
    a: kurbo::Point,
    b: kurbo::Point,
    time: f64,
) -> Result<(Command, Vec<Id>), OpError> {
    let d = b - a;
    if d.hypot() < 1e-6 {
        return Err(OpError::Invalid("drag across the shape to cut it".into()));
    }
    let dir = d.normalize();
    let normal = kurbo::Vec2::new(-dir.y, dir.x);
    // Two huge half-planes on either side of the line.
    let far = 1.0e6;
    let half = |side: f64| {
        let mut p = BezPath::new();
        let (p0, p1) = (a - dir * far, a + dir * far);
        p.move_to(p0);
        p.line_to(p1);
        p.line_to(p1 + normal * (far * side));
        p.line_to(p0 + normal * (far * side));
        p.close_path();
        p
    };
    let (left, right) = (half(1.0), half(-1.0));
    let segment = kurbo::Line::new(a, b);
    let c = project.comp(comp)?;
    let mut cmds = Vec::new();
    let mut new_ids = Vec::new();
    let mut next = project.next_id;
    // From the top of the stack down, so the pieces put in above one layer don't move the ones below it.
    let mut order: Vec<Id> = layers.to_vec();
    order.sort_by_key(|id| std::cmp::Reverse(c.layer_index(*id).unwrap_or(0)));
    order.dedup();
    for id in order {
        let Ok(layer) = project.layer(id) else { continue };
        if !can_combine(layer) || layer.meta.locked {
            continue;
        }
        let Ok(area) = filled_area(project, comp, id, time) else { continue };
        // Only shapes the stroke of the knife actually goes through.
        let crossings: usize = area.segments().map(|s| s.intersect_line(segment).len()).sum();
        if crossings < 2 {
            continue;
        }
        let pieces: Vec<BezPath> = [&left, &right]
            .into_iter()
            .map(|h| contours(linesweeper::binary_op(&area, h, LsRule::EvenOdd, BinaryOp::Intersection)))
            .collect::<Result<_, _>>()?;
        if pieces.iter().any(|p| p.elements().is_empty()) {
            continue;
        }
        let template = layer.shapes().iter().find(|s| s.fill.is_some()).cloned();
        let index = c.layer_index(id).unwrap_or(c.layers.len());
        // The pieces stay in the layer's group, drawn in the group's own space.
        let parent = layer.meta.parent.filter(|p| c.layer_index(*p).is_some());
        let into = parent.map(|p| xenify_anim::world_transform(c, p, time).inverse()).unwrap_or(Affine::IDENTITY);
        for (k, piece) in pieces.into_iter().enumerate() {
            let piece = into * piece;
            let centre = piece.bounding_box().center().to_vec2();
            let mut shape = ShapeItem::path(Affine::translate(-centre) * piece, None, None);
            if let Some(t) = &template {
                shape.fill = t.fill.clone().map(|f| xenify_doc::Fill { color: f.color.value_at(time).into(), ..f });
                shape.stroke = t.stroke.clone();
                shape.name = t.name.clone();
            }
            shape.fill_rule = FillRule::EvenOdd;
            let new_id = Id(next);
            next += 1;
            let mut piece_layer =
                Layer::shape(new_id, format!("{} ({})", layer.meta.name, k + 1), vec![shape]).with_position(centre);
            piece_layer.meta.in_point = layer.meta.in_point;
            piece_layer.meta.out_point = layer.meta.out_point;
            piece_layer.meta.parent = parent;
            cmds.push(Command::AddLayer { comp, index: Some(index + 1 + k), layer: piece_layer });
            new_ids.push(new_id);
        }
        cmds.push(Command::RemoveLayer { layer: id });
    }
    if cmds.is_empty() {
        return Err(OpError::Invalid("drag the knife right across a filled shape to cut it".into()));
    }
    Ok((Command::batch(cmds), new_ids))
}

/// Where a knife from `a` to `b` would cut: for each of `layers` it goes right
/// through, the stretches of the line inside the shape (canvas space).
pub fn knife_cuts(
    project: &Project,
    comp: Id,
    layers: &[Id],
    a: kurbo::Point,
    b: kurbo::Point,
    time: f64,
) -> Vec<(Id, Vec<(kurbo::Point, kurbo::Point)>)> {
    let segment = kurbo::Line::new(a, b);
    let mut out = Vec::new();
    if (b - a).hypot() < 1e-6 {
        return out;
    }
    for &id in layers {
        let Ok(layer) = project.layer(id) else { continue };
        if !can_combine(layer) || layer.meta.locked {
            continue;
        }
        let Ok(area) = filled_area(project, comp, id, time) else { continue };
        let mut ts: Vec<f64> = area.segments().flat_map(|s| s.intersect_line(segment)).map(|x| x.line_t).collect();
        if ts.len() < 2 {
            continue;
        }
        ts.sort_by(f64::total_cmp);
        let at = |t: f64| a.lerp(b, t);
        let cuts: Vec<_> = ts
            .windows(2)
            .filter(|w| w[1] - w[0] > 1e-9 && area.winding(at((w[0] + w[1]) / 2.0)) % 2 != 0)
            .map(|w| (at(w[0]), at(w[1])))
            .collect();
        if !cuts.is_empty() {
            out.push((id, cuts));
        }
    }
    out
}

#[cfg(test)]
mod knife_tests {
    use super::*;
    use xenify_doc::{Color, Vec2};

    #[test]
    fn the_knife_splits_a_shape_in_two() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let sq = Layer::shape(Id(1), "sq", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(200.0, 200.0));
        p.comp_mut(comp).unwrap().layers.push(sq);
        p.next_id = 2;
        // A vertical cut through the middle.
        let (cmd, ids) = knife(&p, comp, &[Id(1)], (200.0, 100.0).into(), (200.0, 300.0).into(), 0.0).unwrap();
        crate::apply(&mut p, cmd).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(p.layer(Id(1)).is_err(), "the original is replaced");
        let boxes: Vec<kurbo::Rect> = ids
            .iter()
            .map(|id| {
                xenify_anim::world_box(
                    p.comp(comp).unwrap(),
                    &xenify_anim::evaluate_project(&p, comp, 0.0),
                    *id,
                    &mut xenify_anim::ApproxText,
                )
                .unwrap()
            })
            .collect();
        let mut xs: Vec<(f64, f64)> = boxes.iter().map(|b| (b.x0.round(), b.x1.round())).collect();
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(xs, vec![(150.0, 200.0), (200.0, 250.0)]);
        // A line that misses does nothing.
        assert!(knife(&p, comp, &ids, (0.0, 0.0).into(), (10.0, 0.0).into(), 0.0).is_err());
    }
}
