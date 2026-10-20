//! Outline Stroke: turns strokes into filled shapes of the same look.

use crate::{Command, OpError, invalid};
use xenify_doc::{Fill, FillRule, Geometry, Id, LayerKind, LineCap, LineJoin, Project, ShapeItem};
use kurbo::BezPath;

/// The outline of `shape`'s stroke at `time`, as a filled path (`None` when
/// it has no stroke). Pen styles give their marks; dashes their dashes.
pub fn stroke_outline(shape: &ShapeItem, time: f64, seed: u64, project: &Project) -> Option<BezPath> {
    // The line as drawn: shortened under its arrowheads, which come too.
    let mut items = vec![xenify_anim::draw_item(shape, time, seed, Some(project))];
    xenify_anim::diagram::add_markers(&mut items, std::slice::from_ref(shape), seed);
    let item = items.remove(0);
    let mut out = item_outline(item)?;
    for m in items {
        if m.fill.is_some() {
            out.extend(m.path.iter());
        }
        if m.stroke.is_some()
            && let Some(p) = item_outline(xenify_anim::DrawItem { fill: None, ..m })
        {
            out.extend(p.iter());
        }
    }
    Some(out)
}

/// The filled outline of a drawn item's stroke.
fn item_outline(item: xenify_anim::DrawItem) -> Option<BezPath> {
    let st = item.stroke?;
    if let Some(marks) = &st.marks {
        let mut out = BezPath::new();
        for m in marks.iter().filter(|m| m.alpha > 0.0) {
            out.extend(m.path.iter());
        }
        return Some(out);
    }
    let style = kurbo::Stroke::new(st.width)
        .with_join(match st.join {
            LineJoin::Miter => kurbo::Join::Miter,
            LineJoin::Round => kurbo::Join::Round,
            LineJoin::Bevel => kurbo::Join::Bevel,
        })
        .with_caps(match st.cap {
            LineCap::Butt => kurbo::Cap::Butt,
            LineCap::Round => kurbo::Cap::Round,
            LineCap::Square => kurbo::Cap::Square,
        })
        .with_dashes(st.dash_offset, st.dashes.iter().copied());
    Some(kurbo::stroke(item.path.iter(), &style, &kurbo::StrokeOpts::default(), 0.05))
}

/// Outline Stroke on `layers` (and the shape layers inside groups among
/// them): each stroked shape becomes its fill (if it has one) under a new
/// filled shape in the stroke's paint, as the stroke looks at `time`.
pub fn outline_strokes(project: &Project, comp: Id, layers: &[Id], time: f64) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let mut ids: Vec<Id> = Vec::new();
    for id in layers {
        for x in std::iter::once(*id).chain(xenify_anim::descendants(c, *id)) {
            if !ids.contains(&x) {
                ids.push(x);
            }
        }
    }
    let mut cmds = Vec::new();
    for id in ids {
        let LayerKind::Shape { shapes } = &project.layer(id)?.kind else { continue };
        if shapes.iter().all(|s| s.stroke.is_none()) {
            continue;
        }
        let mut out = Vec::new();
        for (i, s) in shapes.iter().enumerate() {
            let Some(stroke) = &s.stroke else {
                out.push(s.clone());
                continue;
            };
            if s.fill.is_some() {
                out.push(ShapeItem { stroke: None, ..s.clone() });
            }
            let seed = id.0.wrapping_mul(1000) + i as u64;
            let Some(path) = stroke_outline(s, time, seed, project) else { continue };
            out.push(ShapeItem {
                name: format!("{} outline", s.name),
                geometry: Geometry::Path { path: path.into() },
                fill_rule: FillRule::NonZero,
                fill: Some(Fill { color: stroke.color.clone(), gradient: stroke.gradient.clone(), pattern: None }),
                stroke: None,
                trim: None,
            });
        }
        for index in (0..shapes.len()).rev() {
            cmds.push(Command::RemoveShape { layer: id, index });
        }
        for shape in out {
            cmds.push(Command::AddShape { layer: id, index: None, shape });
        }
    }
    if cmds.is_empty() {
        return Err(invalid("select shapes with a stroke to outline"));
    }
    Ok(Command::batch(cmds))
}

#[cfg(test)]
mod tests {
    #[test]
    fn outlined_arrows_keep_their_heads() {
        use xenify_doc::{Color, Marker, MarkerKind};
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((200.0, 0.0));
        for (kind, name) in [(MarkerKind::Triangle, "filled"), (MarkerKind::Open, "open")] {
            let mut s = ShapeItem::path(p.clone(), None, Some((Color::BLACK, 4.0)));
            s.stroke.as_mut().unwrap().end_marker = Marker { kind, size: 1.0 };
            let out = stroke_outline(&s, 0.0, 1, &Project::new()).unwrap();
            let b = kurbo::Shape::bounding_box(&out);
            assert!(b.height() > 10.0, "{name}: the arrowhead is there: {b:?}");
            assert!((b.x1 - 200.0).abs() < 4.0, "{name}: to the tip: {b:?}");
        }
    }

    use super::*;
    use xenify_doc::{Color, Layer, Stroke, Vec2};
    use kurbo::Shape;

    #[test]
    fn a_stroked_square_becomes_its_fill_and_a_ring() {
        let mut project = Project::new();
        let comp = project.compositions[0].id;
        let mut sq = ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE);
        sq.stroke = Some(Stroke { width: 10.0.into(), color: Color::BLACK.into(), ..Default::default() });
        project.comp_mut(comp).unwrap().layers.push(Layer::shape(Id(1), "sq", vec![sq]));
        let cmd = outline_strokes(&project, comp, &[Id(1)], 0.0).unwrap();
        crate::apply(&mut project, cmd).unwrap();
        let LayerKind::Shape { shapes } = &project.layer(Id(1)).unwrap().kind else { panic!() };
        assert_eq!(shapes.len(), 2);
        assert!(shapes.iter().all(|s| s.stroke.is_none()));
        let Geometry::Path { path } = &shapes[1].geometry else { panic!() };
        let ring = path.value_at(0.0);
        assert!((ring.bounding_box().width() - 110.0).abs() < 0.5, "{:?}", ring.bounding_box());
        assert!(ring.contains(kurbo::Point::new(50.0, 0.0)) && !ring.contains(kurbo::Point::ZERO), "a ring");
        assert_eq!(shapes[1].fill.as_ref().unwrap().color.value_at(0.0), Color::BLACK);
        assert!(outline_strokes(&project, comp, &[Id(1)], 0.0).is_err(), "nothing left to outline");
    }
}
