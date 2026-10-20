//! Free transform: rotating, scaling and distorting layers around any point in
//! composition space, written back as ordinary position / rotation / scale
//! and corner-pin values (so it animates like any other edit).

use crate::{Command, OpError};
use xenify_anim::world_transform;
use xenify_doc::{Distort, Homography, Id, Project, PropPath, Property, Quad, Value};
use kurbo::{Affine, Rect, Vec2};

/// A layer's transform when a gesture started; every drag frame is computed
/// from this, so the result never drifts.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: Id,
    anchor: Vec2,
    rotation: f64,
    /// Skew and its axis (degrees) at the start.
    skew: f64,
    skew_axis: f64,
    /// Parent → composition.
    parent: Affine,
    /// Layer → composition.
    world: Affine,
    distort: Option<Distort>,
    /// The layer's content box (layer space, without perspective); the corner
    /// pin's source rectangle when a new one is created.
    source: Rect,
}

impl Snapshot {
    /// `bounds`: the layer's content box in layer space (for a group, the box
    /// around its members in the group's space).
    pub fn new(project: &Project, comp: Id, id: Id, time: f64, bounds: Rect) -> Result<Self, OpError> {
        let c = project.comp(comp)?;
        let l = project.layer(id)?;
        let parent = l.meta.parent.map_or(Affine::IDENTITY, |p| world_transform(c, p, time));
        Ok(Self {
            id,
            anchor: l.transform.anchor.value_at(time),
            rotation: l.transform.rotation.value_at(time),
            skew: l.transform.skew.value_at(time),
            skew_axis: l.transform.skew_axis.value_at(time),
            parent,
            world: world_transform(c, id, time),
            distort: l.transform.distort.clone(),
            source: bounds,
        })
    }

    /// Layer → composition at the start of the gesture.
    pub fn world(&self) -> Affine {
        self.world
    }

    /// The layer's box corners in composition space, including perspective.
    pub fn corners(&self, time: f64) -> Quad {
        let q = self.distort.as_ref().map_or(Quad::of_rect(self.source), |d| d.corners.value_at(time));
        q.map(|p| self.world * p)
    }

    /// Commands that make the layer's on-screen transform `x · (start)`.
    /// A slant that results (e.g. squashing a rotated layer) becomes the
    /// layer's skew; with a skew axis set, the skew is kept as it was.
    pub fn affine_commands(
        &self,
        project: &Project,
        x: Affine,
        time: f64,
        auto_key: bool,
    ) -> Result<Vec<Command>, OpError> {
        let local = self.parent.inverse() * x * self.world;
        // With a skew axis the axis-free decomposition doesn't apply: take the
        // skew out first and keep it.
        let local = if self.skew_axis.abs() > 1e-9 && self.skew != 0.0 {
            let axis = self.skew_axis.to_radians();
            let k = Affine::rotate(axis) * Affine::skew(self.skew.to_radians().tan(), 0.0) * Affine::rotate(-axis);
            let [a, b, c, d, e, f] = local.as_coeffs();
            let lin = Affine::new([a, b, c, d, 0.0, 0.0]);
            // L = R·K·S ≈ (L·S⁻¹·K⁻¹)… approximated by removing K on the rotation side.
            let without = lin * k.inverse();
            let [a, b, c, d, _, _] = without.as_coeffs();
            Affine::new([a, b, c, d, e, f])
        } else {
            local
        };
        let [a, b, c, d, _, _] = local.as_coeffs();
        let theta = b.atan2(a);
        let sx = a.hypot(b);
        let sy = -c * theta.sin() + d * theta.cos();
        // Rotation × (skew × scale) is rotation × an upper-triangular matrix.
        let shear = c * theta.cos() + d * theta.sin();
        let skew_deg = if sy.abs() > 1e-12 { (shear / sy).atan().to_degrees() } else { 0.0 };
        // Keep rotation continuous with where it started (no 359° → 0° jumps).
        let mut deg = theta.to_degrees();
        deg += ((self.rotation - deg) / 360.0).round() * 360.0;
        let position = (local * self.anchor.to_point()).to_vec2();
        let edit = |path, value| Command::edit_value(project, self.id, path, time, value, auto_key);
        let mut cmds = vec![
            edit(PropPath::Position, Value::Vec2(position))?,
            edit(PropPath::Rotation, Value::Scalar(deg))?,
            edit(PropPath::Scale, Value::Vec2(Vec2::new(sx, sy)))?,
        ];
        let axis_free = self.skew_axis.abs() < 1e-9;
        if axis_free && (self.skew != 0.0 || skew_deg.abs() > 0.05) && (skew_deg - self.skew).abs() > 1e-6 {
            cmds.push(edit(PropPath::Skew, Value::Scalar(skew_deg))?);
        }
        Ok(cmds)
    }

    /// The command that adds composition-space perspective `g` on top of the
    /// layer's start state (creating its corner pin if needed).
    pub fn distort_command(&self, g: Homography, time: f64, auto_key: bool) -> Option<Command> {
        let w = Homography::from(self.world);
        let m = w.inverse()? * g * w;
        let (source, mut corners) = match &self.distort {
            Some(d) => (d.source, d.corners.clone()),
            None => (self.source, Property::Static(Quad::of_rect(self.source))),
        };
        if source.width() <= 1e-9 || source.height() <= 1e-9 {
            return None;
        }
        let new = corners.value_at(time).map(|p| m * p);
        if auto_key && !corners.is_animated() {
            let old = corners.value_at(time);
            if time > xenify_doc::TIME_EPSILON {
                corners.set_keyframe(0.0, old, None);
            }
            corners.set_keyframe(time, new, None);
        } else {
            corners.set_value(time, new);
        }
        Some(Command::SetDistort { layer: self.id, distort: Some(Distort { source, corners }) })
    }
}

/// Rotation by `degrees` (clockwise on screen) around `pivot`.
pub fn rotate_about(pivot: kurbo::Point, degrees: f64) -> Affine {
    Affine::translate(pivot.to_vec2()) * Affine::rotate(degrees.to_radians()) * Affine::translate(-pivot.to_vec2())
}

/// Scaling by `sx`/`sy` along axes rotated by `angle` radians, around `pivot`.
pub fn scale_about(pivot: kurbo::Point, angle: f64, sx: f64, sy: f64) -> Affine {
    Affine::translate(pivot.to_vec2())
        * Affine::rotate(angle)
        * Affine::scale_non_uniform(sx, sy)
        * Affine::rotate(-angle)
        * Affine::translate(-pivot.to_vec2())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Color, Layer, ShapeItem};
    use kurbo::Point;

    fn project() -> (Project, Id, Id) {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let id = Id(p.next_id);
        p.next_id += 1;
        let mut l = Layer::shape(id, "Box", vec![ShapeItem::rect(Vec2::new(100.0, 50.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 200.0));
        l.transform.anchor = Vec2::new(10.0, 0.0).into();
        l.transform.rotation = 30.0.into();
        p.compositions[0].layers.push(l);
        (p, comp, id)
    }

    fn apply(p: &mut Project, cmds: Vec<Command>) {
        crate::apply(p, Command::batch(cmds)).unwrap();
    }

    #[test]
    fn rotating_about_a_pivot_moves_the_layer_around_it() {
        let (mut p, comp, id) = project();
        let s = Snapshot::new(&p, comp, id, 0.0, Rect::new(-50.0, -25.0, 50.0, 25.0)).unwrap();
        let pivot = Point::new(100.0, 100.0);
        let x = rotate_about(pivot, 90.0);
        let probe = Point::new(20.0, 5.0);
        let expected = x * (s.world() * probe);
        let cmds = s.affine_commands(&p, x, 0.0, false).unwrap();
        apply(&mut p, cmds);
        let now = world_transform(p.comp(comp).unwrap(), id, 0.0) * probe;
        assert!((now - expected).hypot() < 1e-9, "{now:?} vs {expected:?}");
        assert!((p.layer(id).unwrap().transform.rotation.value_at(0.0) - 120.0).abs() < 1e-9);
    }

    #[test]
    fn scaling_along_the_layer_axes_keeps_the_pivot_fixed() {
        let (mut p, comp, id) = project();
        let s = Snapshot::new(&p, comp, id, 0.0, Rect::new(-50.0, -25.0, 50.0, 25.0)).unwrap();
        let pivot = s.world() * Point::new(-50.0, -25.0);
        let x = scale_about(pivot, 30f64.to_radians(), 2.0, 0.5);
        let cmds = s.affine_commands(&p, x, 0.0, false).unwrap();
        apply(&mut p, cmds);
        let w = world_transform(p.comp(comp).unwrap(), id, 0.0);
        assert!((w * Point::new(-50.0, -25.0) - pivot).hypot() < 1e-9);
        let scale = p.layer(id).unwrap().transform.scale.value_at(0.0);
        assert!((scale - Vec2::new(2.0, 0.5)).hypot() < 1e-9);
    }

    #[test]
    fn perspective_creates_a_corner_pin_that_lands_on_the_new_corners() {
        let (mut p, comp, id) = project();
        let bounds = Rect::new(-50.0, -25.0, 50.0, 25.0);
        let s = Snapshot::new(&p, comp, id, 0.0, bounds).unwrap();
        let before = s.corners(0.0);
        let mut after = before;
        after.0[1].y -= 30.0;
        after.0[2].y += 30.0;
        let g = Homography::from_quads(&before, &after).unwrap();
        let cmd = s.distort_command(g, 1.0, true).unwrap();
        crate::apply(&mut p, cmd).unwrap();
        let d = p.layer(id).unwrap().transform.distort.clone().unwrap();
        assert_eq!(d.corners.keyframes().len(), 2, "auto-key keeps the old corners at 0 s");
        let scene = xenify_anim::evaluate(p.comp(comp).unwrap(), 1.0);
        let xenify_anim::SceneContent::Shapes(items) = &scene.layers[0].content else { panic!() };
        let path_pts: Vec<Point> = items[0].path.elements().iter().filter_map(|e| e.end_point()).collect();
        let on_screen: Vec<Point> = path_pts.iter().map(|q| scene.layers[0].transform * *q).collect();
        for c in after.points() {
            assert!(on_screen.iter().any(|q| (*q - c).hypot() < 1e-6), "corner {c:?} missing");
        }
    }
}

#[cfg(test)]
mod skew_tests {
    use super::*;
    use xenify_doc::{Color, Layer, ShapeItem};

    #[test]
    fn transforming_a_skewed_layer_keeps_its_slant() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let mut l = Layer::shape(Id(5), "s", vec![ShapeItem::rect(Vec2::new(100.0, 50.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 200.0));
        l.transform.skew = 20.0.into();
        l.transform.scale = Vec2::new(1.5, 0.8).into();
        p.compositions[0].layers.push(l);
        let before = world_transform(&p.compositions[0], Id(5), 0.0);
        let snap = Snapshot::new(&p, comp, Id(5), 0.0, Rect::new(-50.0, -25.0, 50.0, 25.0)).unwrap();
        // Rotate 30° about a point and move.
        let x = Affine::translate((40.0, -10.0)) * rotate_about(kurbo::Point::new(250.0, 180.0), 30.0);
        for cmd in snap.affine_commands(&p, x, 0.0, false).unwrap() {
            crate::apply(&mut p, cmd).unwrap();
        }
        let after = world_transform(&p.compositions[0], Id(5), 0.0);
        let want = x * before;
        for (a, b) in after.as_coeffs().iter().zip(want.as_coeffs()) {
            assert!((a - b).abs() < 1e-6, "{after:?} vs {want:?}");
        }
        assert!((p.layer(Id(5)).unwrap().transform.skew.value_at(0.0) - 20.0).abs() < 1e-6);
    }
}
