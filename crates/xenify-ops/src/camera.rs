//! Camera layers: adding one, following a layer, shake, and choosing which
//! camera is used.

use crate::{Command, OpError, invalid};
use kurbo::Vec2;
use xenify_doc::{CameraLayer, CameraShake, Id, LayerKind, Project, camera_layer};

/// Adds a camera on top of `comp` that frames it exactly as it is (so
/// nothing changes until it's moved, turned or zoomed). With `follow`, the
/// camera is parented to that layer and moves with it.
pub fn add_camera(project: &Project, comp: Id, id: Id, follow: Option<Id>) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (w, h) = (c.settings.width as f64, c.settings.height as f64);
    let mut layer = camera_layer(id, w, h);
    let n = c.layers.iter().filter(|l| matches!(l.kind, LayerKind::Camera(_))).count();
    if n > 0 {
        layer.meta.name = format!("Camera {}", n + 1);
    }
    if let Some(target) = follow {
        let t = c.layer(target).ok_or_else(|| invalid(format!("no layer {target} to follow")))?;
        // Keep the camera where it is: its position becomes relative to the target.
        let world = xenify_anim::world_transform(c, target, 0.0);
        let local = world.inverse() * kurbo::Point::new(w / 2.0, h / 2.0);
        layer.transform.position = Vec2::new(local.x, local.y).into();
        layer.meta.parent = Some(t.id);
    }
    Ok(Command::AddLayer { comp, index: None, layer })
}

/// Changes a camera's settings.
pub fn edit(project: &Project, layer: Id, f: impl FnOnce(&mut CameraLayer)) -> Result<Command, OpError> {
    let LayerKind::Camera(c) = &project.layer(layer)?.kind else {
        return Err(invalid(format!("layer {layer} is not a camera")));
    };
    let mut c = c.clone();
    f(&mut c);
    Ok(Command::SetCamera { layer, camera: Box::new(c) })
}

/// Turns shake on (with `shake`) or off.
pub fn set_shake(project: &Project, layer: Id, shake: Option<CameraShake>) -> Result<Command, OpError> {
    edit(project, layer, |c| c.shake = shake)
}

/// Makes `layer` the camera that's used: it moves above the other cameras
/// (the topmost visible camera wins) and is shown.
pub fn make_active(project: &Project, layer: Id) -> Result<Option<Command>, OpError> {
    let (comp, index) = project.locate_layer(layer)?;
    let c = project.comp(comp)?;
    let mut commands = Vec::new();
    if !c.layers[index].meta.visible {
        let mut meta = c.layers[index].meta.clone();
        meta.visible = true;
        commands.push(Command::SetLayerMeta { layer, meta });
    }
    let top = c.layers.iter().rposition(|l| matches!(l.kind, LayerKind::Camera(_)) && l.meta.visible);
    if let Some(top) = top.filter(|&t| t > index) {
        commands.push(Command::MoveLayer { layer, index: top });
    }
    Ok((!commands.is_empty()).then(|| Command::batch(commands)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use kurbo::{Affine, Point};
    use xenify_doc::{Color, Layer, ShapeItem};

    fn setup() -> (Project, Id, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let ball = Layer::shape(Id(1), "Ball", vec![ShapeItem::ellipse(Vec2::new(50.0, 50.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 200.0));
        p.comp_mut(comp).unwrap().layers = vec![ball];
        p.reserve_id(Id(1));
        (p, comp, Id(1))
    }

    #[test]
    fn a_new_camera_changes_nothing_until_moved() {
        let (mut p, comp, _) = setup();
        let before = xenify_anim::evaluate_project(&p, comp, 0.0);
        let cmd = add_camera(&p, comp, Id(10), None).unwrap();
        apply(&mut p, cmd).unwrap();
        let after = xenify_anim::evaluate_project(&p, comp, 0.0);
        assert_eq!(before.layers, after.layers, "the camera itself isn't drawn");
        let c = after.camera;
        assert!((c * Point::new(12.0, 34.0) - Point::new(12.0, 34.0)).hypot() < 1e-9);
    }

    #[test]
    fn a_following_camera_keeps_the_target_in_view() {
        let (mut p, comp, ball) = setup();
        let cmd = add_camera(&p, comp, Id(10), Some(ball)).unwrap();
        apply(&mut p, cmd).unwrap();
        let before = xenify_anim::evaluate_project(&p, comp, 0.0).camera * Point::new(300.0, 200.0);
        // Move the ball: it stays at the same place in the picture.
        let cmd = Command::SetValue {
            layer: ball,
            path: xenify_doc::PropPath::Position,
            time: 0.0,
            value: xenify_doc::Value::Vec2(Vec2::new(900.0, 700.0)),
        };
        apply(&mut p, cmd).unwrap();
        let after = xenify_anim::evaluate_project(&p, comp, 0.0).camera * Point::new(900.0, 700.0);
        assert!((before - after).hypot() < 1e-9, "{before:?} vs {after:?}");
    }

    #[test]
    fn zoom_and_shake_are_undoable_settings() {
        let (mut p, comp, _) = setup();
        let cmd = add_camera(&p, comp, Id(10), None).unwrap();
        apply(&mut p, cmd).unwrap();
        let cmd = edit(&p, Id(10), |c| c.zoom = 2.0.into()).unwrap();
        let undo = apply(&mut p, cmd).unwrap();
        assert_ne!(xenify_anim::evaluate_project(&p, comp, 0.0).camera, Affine::IDENTITY);
        apply(&mut p, undo).unwrap();
        assert_eq!(xenify_anim::evaluate_project(&p, comp, 0.0).camera, Affine::IDENTITY);
        let cmd = set_shake(&p, Id(10), Some(CameraShake::impact(0.0))).unwrap();
        apply(&mut p, cmd).unwrap();
        assert_ne!(xenify_anim::evaluate_project(&p, comp, 0.2).camera, Affine::IDENTITY);
        // Saved and loaded.
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.layer(Id(10)).unwrap().kind, p.layer(Id(10)).unwrap().kind);
    }

    #[test]
    fn make_active_moves_a_camera_above_the_others() {
        let (mut p, comp, _) = setup();
        let cmd = add_camera(&p, comp, Id(10), None).unwrap();
        apply(&mut p, cmd).unwrap();
        let cmd = add_camera(&p, comp, Id(11), None).unwrap();
        apply(&mut p, cmd).unwrap();
        let active = |p: &Project| xenify_doc::active_camera(&p.comp(comp).unwrap().layers, 0.0).unwrap().id;
        assert_eq!(active(&p), Id(11));
        let cmd = make_active(&p, Id(10)).unwrap().unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(active(&p), Id(10));
        assert!(make_active(&p, Id(10)).unwrap().is_none());
    }
}
