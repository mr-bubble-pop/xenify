//! Diagram edits: connectors glued between shapes, their ends, labels and
//! ports, and diagram canvases (as symbols that can be placed anywhere).

use crate::{Command, OpError, invalid};
use xenify_doc::{
    BezPath, CompKind, CompSettings, Composition, Connector, ConnectorEnd, DiagramSettings, Id, Layer, LayerKind,
    LayerMeta, LineJoin, NodeLabel, Port, Project, ShapeItem, Vec2,
};

/// A new connector layer (not yet added) from `from` to `to`, styled with
/// the diagram's defaults.
pub fn connector_layer(id: Id, from: ConnectorEnd, to: ConnectorEnd, d: &DiagramSettings) -> Layer {
    let mut path = BezPath::new();
    path.move_to(from.point.to_point());
    path.line_to(to.point.to_point());
    let mut shape = ShapeItem::path(path, None, Some((d.line_color, d.line_width)));
    shape.name = "Line".into();
    if let Some(s) = &mut shape.stroke {
        s.join = LineJoin::Round;
        s.end_marker = d.end_marker;
    }
    let mut layer = Layer::shape(id, "Connector", vec![shape]);
    layer.meta.connector = Some(Box::new(Connector::new(from, to).styled(d)));
    layer
}

/// The diagram settings in force in `comp` (its own, or the defaults).
pub fn settings_of(project: &Project, comp: Id) -> DiagramSettings {
    project.comp(comp).ok().and_then(|c| c.settings.diagram.clone()).unwrap_or_default()
}

/// Adds a connector on top of `comp`. Returns the command and the new
/// layer's id.
pub fn add_connector(
    project: &Project,
    comp: Id,
    from: ConnectorEnd,
    to: ConnectorEnd,
) -> Result<(Command, Id), OpError> {
    let c = project.comp(comp)?;
    for a in [from.attach, to.attach].into_iter().flatten() {
        if c.layer(a.layer).is_none() {
            return Err(invalid(format!("layer {} is not in this composition", a.layer)));
        }
    }
    let id = Id(project.next_id);
    let layer = connector_layer(id, from, to, &settings_of(project, comp));
    Ok((Command::AddLayer { comp, index: None, layer }, id))
}

/// Pools and lanes hold what's dropped in them: each of `layers` whose
/// middle is inside a container (the smallest one; lanes and other
/// containers go only into bigger containers) becomes its member, parented
/// to it without moving on screen; one dragged out of its container leaves
/// it. `None` when nothing changes.
pub fn adopt(project: &Project, comp: Id, layers: &[Id], time: f64) -> Result<Option<Command>, OpError> {
    let c = project.comp(comp)?;
    let scene = xenify_anim::evaluate_project(project, comp, time);
    let world_box = |id: Id| xenify_anim::world_box(c, &scene, id, &mut xenify_anim::ApproxText);
    let containers: Vec<(Id, kurbo::Rect)> =
        c.layers.iter().filter(|l| l.meta.container).filter_map(|l| Some((l.id, world_box(l.id)?))).collect();
    let mut cmds = Vec::new();
    for &id in layers {
        let l = project.layer(id)?;
        if l.meta.connector.is_some() {
            continue;
        }
        // Only layers that are loose or already in a container move between containers.
        let current = l.meta.parent;
        if current.is_some_and(|p| c.layer(p).is_none_or(|p| !p.meta.container)) {
            continue;
        }
        let Some(b) = world_box(id) else { continue };
        let area = b.area();
        let inside = xenify_anim::descendants(c, id);
        let target = containers
            .iter()
            .filter(|(cid, r)| {
                *cid != id && !inside.contains(cid) && r.contains(b.center()) && (!l.meta.container || r.area() > area)
            })
            .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .map(|(cid, _)| *cid);
        if target == current {
            continue;
        }
        cmds.push(join(project, comp, id, target, time)?);
    }
    Ok((!cmds.is_empty()).then(|| Command::batch(cmds)))
}

/// Makes `layer` a member of `container` (or of nothing), keeping it where
/// it is on screen: its position (every key) moves by the change.
pub fn join(project: &Project, comp: Id, layer: Id, container: Option<Id>, time: f64) -> Result<Command, OpError> {
    use xenify_anim::world_transform;
    let c = project.comp(comp)?;
    let l = project.layer(layer)?;
    if container.is_some_and(|p| p == layer || xenify_anim::descendants(c, layer).contains(&p)) {
        return Err(invalid("a layer can't go inside itself"));
    }
    let anchor = l.transform.anchor.value_at(time).to_point();
    let world = world_transform(c, layer, time) * anchor;
    let parent_world = container.map_or(kurbo::Affine::IDENTITY, |p| world_transform(c, p, time));
    let now = l.transform.position.value_at(time);
    let local = parent_world.inverse() * world;
    let shift = Vec2::new(local.x - now.x, local.y - now.y);
    let mut position = l.transform.position.clone();
    match &mut position {
        xenify_doc::Property::Static(v) => *v += shift,
        xenify_doc::Property::Animated(keys) => keys.iter_mut().for_each(|k| k.value += shift),
    }
    let mut meta = l.meta.clone();
    meta.parent = container;
    Ok(Command::batch(vec![
        Command::SetLayerMeta { layer, meta },
        Command::SetProperty {
            layer,
            path: xenify_doc::PropPath::Position,
            property: xenify_doc::AnyProperty::Vec2(position),
        },
    ]))
}

/// Changes a layer's meta through `f` as one undoable command.
pub fn edit_meta(project: &Project, layer: Id, f: impl FnOnce(&mut LayerMeta)) -> Result<Command, OpError> {
    let mut meta = project.layer(layer)?.meta.clone();
    f(&mut meta);
    Ok(Command::SetLayerMeta { layer, meta })
}

/// Changes a connector's settings through `f`.
pub fn edit_connector(project: &Project, layer: Id, f: impl FnOnce(&mut Connector)) -> Result<Command, OpError> {
    let l = project.layer(layer)?;
    if l.meta.connector.is_none() {
        return Err(invalid(format!("“{}” is not a connector", l.meta.name)));
    }
    edit_meta(project, layer, |m| {
        if let Some(c) = m.connector.as_deref_mut() {
            f(c);
        }
    })
}

/// Sets (or with empty text, removes) a layer's label, keeping its styling.
pub fn set_label_text(project: &Project, layer: Id, text: &str) -> Result<Command, OpError> {
    edit_meta(project, layer, |m| match (&mut m.label, text.is_empty()) {
        (_, true) => m.label = None,
        (Some(l), false) => l.text = text.to_string(),
        (None, false) => m.label = Some(NodeLabel::new(text)),
    })
}

pub fn set_ports(project: &Project, layer: Id, ports: Vec<Port>) -> Result<Command, OpError> {
    edit_meta(project, layer, |m| m.ports = ports)
}

/// Checks a connector's settings before they're stored on `layer`.
pub(crate) fn validate(project: &Project, layer: Id, c: &Connector) -> Result<(), OpError> {
    let l = project.layer(layer)?;
    if !matches!(l.kind, LayerKind::Shape { ref shapes } if !shapes.is_empty()) {
        return Err(invalid("only a shape layer with a line can be a connector"));
    }
    if c.attached().any(|a| a == layer) {
        return Err(invalid("a connector can't be glued to itself"));
    }
    Ok(())
}

/// Where layer `id` of `comp` is (its box's centre) at the start, for
/// leaving connector ends in place when it's deleted.
pub(crate) fn centre_of(project: &Project, comp: Id, id: Id) -> Option<Vec2> {
    let c = project.comp(comp).ok()?;
    let scene = xenify_anim::evaluate_project(project, comp, 0.0);
    let b = xenify_anim::world_box(c, &scene, id, &mut xenify_anim::ApproxText)?;
    Some(b.center().to_vec2())
}

/// The connector ends in `comp` glued to `removed`, freed where the shape
/// was. Returns the undo commands (old metas) and applies the change.
pub(crate) fn detach_from(comp: &mut Composition, removed: Id, at: Option<Vec2>) -> Vec<Command> {
    let mut undo = Vec::new();
    for l in comp.layers.iter_mut() {
        let Some(c) = l.meta.connector.as_deref() else { continue };
        if !c.attached().any(|a| a == removed) {
            continue;
        }
        undo.push(Command::SetLayerMeta { layer: l.id, meta: l.meta.clone() });
        let c = l.meta.connector.as_deref_mut().expect("checked");
        for end in [&mut c.from, &mut c.to] {
            if end.attach.is_some_and(|a| a.layer == removed) {
                end.attach = None;
                if let Some(p) = at {
                    end.point = p;
                }
            }
        }
    }
    undo
}

/// Makes `settings` a diagram canvas (or back into an animation one). A new
/// diagram gets a light page, a grid and connector defaults unless it
/// already has its own.
pub fn set_kind(settings: &mut CompSettings, kind: CompKind) {
    if settings.kind == kind {
        return;
    }
    settings.kind = kind;
    if kind == CompKind::Diagram {
        if settings.background == CompSettings::default().background {
            settings.background = xenify_doc::Color::rgb(0.98, 0.98, 0.97);
        }
        if settings.grid == 0.0 {
            settings.grid = 20.0;
        }
        settings.diagram.get_or_insert_with(DiagramSettings::default);
    }
}

/// The settings of a new, empty diagram canvas.
pub fn diagram_settings(name: &str, width: u32, height: u32) -> CompSettings {
    let mut s = CompSettings { name: name.to_string(), width, height, ..CompSettings::default() };
    set_kind(&mut s, CompKind::Diagram);
    s
}

/// A new, empty diagram canvas in the project. It's a symbol, so it can be
/// placed in any other canvas. Returns the command and its id.
pub fn new_diagram(project: &Project, name: &str, width: u32, height: u32) -> (Command, Id) {
    let id = Id(project.next_id);
    let settings = diagram_settings(name, width, height);
    (Command::AddComposition { index: None, comp: Composition { id, settings, layers: Vec::new() } }, id)
}

/// Whether `comp` is a diagram canvas.
pub fn is_diagram(project: &Project, comp: Id) -> bool {
    project.comp(comp).is_ok_and(|c| c.settings.kind == CompKind::Diagram)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lanes_hold_what_is_dropped_in_them() {
        use crate::library::insert;
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let add = |p: &mut Project, id: &str, at: Vec2| {
            let (cmd, l) = insert(p, comp, id, at, None, None).unwrap();
            apply(p, cmd).unwrap();
            l
        };
        let pool = add(&mut p, "bpmn.pool", Vec2::new(500.0, 400.0));
        let lane = add(&mut p, "bpmn.lane", Vec2::new(500.0, 380.0));
        let task = add(&mut p, "flowchart.process", Vec2::new(400.0, 380.0));
        let cmd = adopt(&p, comp, &[lane, task], 0.0).unwrap().expect("changes");
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.layer(lane).unwrap().meta.parent, Some(pool), "the lane goes into the pool");
        assert_eq!(p.layer(task).unwrap().meta.parent, Some(lane), "the task into the (smaller) lane");
        let world = |p: &Project, id| xenify_anim::world_transform(p.comp(comp).unwrap(), id, 0.0).translation();
        assert!((world(&p, task) - kurbo::Vec2::new(400.0, 380.0)).hypot() < 1e-6, "nothing moved on screen");
        // Moving the pool carries both.
        let mut l = p.layer(pool).unwrap().clone();
        l.transform.position = Vec2::new(600.0, 400.0).into();
        apply(&mut p, Command::ReplaceLayer { layer: Box::new(l) }).unwrap();
        assert!((world(&p, task) - kurbo::Vec2::new(500.0, 380.0)).hypot() < 1e-6);
        // Dragged out, it leaves.
        let mut l = p.layer(task).unwrap().clone();
        l.transform.position = Vec2::new(2000.0, 0.0).into();
        apply(&mut p, Command::ReplaceLayer { layer: Box::new(l) }).unwrap();
        let cmd = adopt(&p, comp, &[task], 0.0).unwrap().expect("changes");
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.layer(task).unwrap().meta.parent, None);
        assert!(adopt(&p, comp, &[task], 0.0).unwrap().is_none(), "nothing more to do");
    }

    use super::*;
    use crate::apply;
    use xenify_doc::{Color, PortRef};

    fn setup() -> (Project, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let a = Layer::shape(Id(1), "A", vec![ShapeItem::rect(Vec2::new(100.0, 60.0), Color::WHITE)])
            .with_position(Vec2::new(100.0, 100.0));
        let b = Layer::shape(Id(2), "B", vec![ShapeItem::rect(Vec2::new(100.0, 60.0), Color::WHITE)])
            .with_position(Vec2::new(500.0, 300.0));
        p.comp_mut(comp).unwrap().layers = vec![a, b];
        p.reserve_id(Id(2));
        (p, comp)
    }

    #[test]
    fn deleting_a_shape_frees_its_connector_ends_and_undo_reglues() {
        let (mut p, comp) = setup();
        let (cmd, id) = add_connector(
            &p,
            comp,
            ConnectorEnd::glued(Id(1), PortRef::Auto, Vec2::ZERO),
            ConnectorEnd::glued(Id(2), PortRef::Auto, Vec2::ZERO),
        )
        .unwrap();
        apply(&mut p, cmd).unwrap();
        let undo = apply(&mut p, Command::RemoveLayer { layer: Id(2) }).unwrap();
        let c = p.layer(id).unwrap().meta.connector.clone().unwrap();
        assert!(c.to.attach.is_none());
        assert_eq!(c.to.point, Vec2::new(500.0, 300.0));
        assert!(c.from.attach.is_some());
        apply(&mut p, undo).unwrap();
        let c = p.layer(id).unwrap().meta.connector.clone().unwrap();
        assert_eq!(c.to.attach.unwrap().layer, Id(2));
    }

    #[test]
    fn connectors_must_be_lines_and_not_glued_to_themselves() {
        let (mut p, comp) = setup();
        let (cmd, id) = add_connector(
            &p,
            comp,
            ConnectorEnd::glued(Id(1), PortRef::Auto, Vec2::ZERO),
            ConnectorEnd::free(Vec2::new(9.0, 9.0)),
        )
        .unwrap();
        apply(&mut p, cmd).unwrap();
        let bad = edit_connector(&p, id, |c| c.to = ConnectorEnd::glued(id, PortRef::Auto, Vec2::ZERO)).unwrap();
        assert!(apply(&mut p, bad).is_err());
        assert!(edit_connector(&p, Id(1), |_| {}).is_err(), "A isn't a connector");
        assert!(
            add_connector(
                &p,
                comp,
                ConnectorEnd::glued(Id(77), PortRef::Auto, Vec2::ZERO),
                ConnectorEnd::free(Vec2::ZERO)
            )
            .is_err()
        );
    }

    #[test]
    fn labels_and_new_diagrams() {
        let (mut p, _) = setup();
        let cmd = set_label_text(&p, Id(1), "Start").unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.layer(Id(1)).unwrap().meta.label.as_ref().unwrap().text, "Start");
        let cmd = set_label_text(&p, Id(1), "").unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(p.layer(Id(1)).unwrap().meta.label.is_none());
        let (cmd, d) = new_diagram(&p, "Flow", 1600, 900);
        apply(&mut p, cmd).unwrap();
        assert!(is_diagram(&p, d));
    }
}
