//! Camera layers: what the viewer sees of a composition.
//!
//! A camera frames a composition-sized window centred on its position, turned
//! with its rotation and magnified by its zoom. The topmost visible camera
//! that is active at the current time is the one that's used; the others are
//! ignored, so cutting between cameras is a matter of in and out points.
//! Parenting the camera to a layer makes it follow that layer.

use crate::{Id, Layer, LayerKind, Property, Wiggle};
use kurbo::{Affine, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraLayer {
    /// Magnification: 2 shows half as much, twice as big.
    pub zoom: Property<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shake: Option<CameraShake>,
}

impl Default for CameraLayer {
    fn default() -> Self {
        Self { zoom: 1.0.into(), shake: None }
    }
}

/// Handheld or impact shake: smooth random motion that starts at `start`
/// and dies away at `decay`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraShake {
    /// Largest drift, in pixels.
    pub amount: Property<f64>,
    /// Largest turn, in degrees.
    #[serde(default = "zero")]
    pub rotation: Property<f64>,
    /// Shakes per second.
    pub frequency: f64,
    /// When the shake starts (seconds).
    #[serde(default)]
    pub start: f64,
    /// How fast it dies away: the amount halves every `ln 2 / decay`
    /// seconds; 0 keeps shaking.
    #[serde(default)]
    pub decay: f64,
    #[serde(default)]
    pub seed: u64,
}

fn zero() -> Property<f64> {
    0.0.into()
}

impl Default for CameraShake {
    fn default() -> Self {
        Self { amount: 12.0.into(), rotation: 0.5.into(), frequency: 8.0, start: 0.0, decay: 0.0, seed: 0 }
    }
}

impl CameraShake {
    /// A short, strong jolt that fades out (what an explosion does to the camera).
    pub fn impact(start: f64) -> Self {
        Self { amount: 40.0.into(), rotation: 1.5.into(), frequency: 14.0, start, decay: 3.0, seed: 1 }
    }

    /// `(offset, turn in degrees)` at `time`.
    pub fn at(&self, time: f64) -> (Vec2, f64) {
        if time < self.start {
            return (Vec2::ZERO, 0.0);
        }
        let envelope = (-self.decay.max(0.0) * (time - self.start)).exp();
        let w = Wiggle { frequency: self.frequency, position: 1.0, rotation: 1.0, scale: 0.0, seed: self.seed };
        let amount = self.amount.value_at(time) * envelope;
        let turn = self.rotation.value_at(time) * envelope;
        (Vec2::new(w.noise(0, time), w.noise(1, time)) * amount, w.noise(2, time) * turn)
    }
}

impl CameraLayer {
    /// Composition → view transform for a camera whose world transform is
    /// `world` (layer → composition), for a `width`×`height` frame.
    pub fn view(&self, world: Affine, width: f64, height: f64, time: f64) -> Affine {
        let zoom = self.zoom.value_at(time).max(1e-3);
        let (offset, turn) = self.shake.as_ref().map_or((Vec2::ZERO, 0.0), |s| s.at(time));
        let camera = world * Affine::translate(offset) * Affine::rotate(turn.to_radians());
        if camera.determinant().abs() < 1e-12 {
            return Affine::IDENTITY;
        }
        Affine::translate((width / 2.0, height / 2.0)) * Affine::scale(zoom) * camera.inverse()
    }
}

/// The camera layer in use at `time`: the topmost visible one that's active.
pub fn active_camera(layers: &[Layer], time: f64) -> Option<&Layer> {
    layers.iter().rev().find(|l| matches!(l.kind, LayerKind::Camera(_)) && l.meta.visible && l.is_active_at(time))
}

/// A new camera framing the whole composition exactly as it is.
pub fn camera_layer(id: Id, width: f64, height: f64) -> Layer {
    Layer::new(id, "Camera", LayerKind::Camera(CameraLayer::default())).with_position(Vec2::new(width, height) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn a_centred_camera_shows_the_composition_unchanged() {
        let cam = CameraLayer::default();
        let v = cam.view(Affine::translate((960.0, 540.0)), 1920.0, 1080.0, 0.0);
        assert!((v * Point::new(10.0, 20.0) - Point::new(10.0, 20.0)).hypot() < 1e-9);
    }

    #[test]
    fn zooming_magnifies_about_the_camera_centre() {
        let cam = CameraLayer { zoom: 2.0.into(), shake: None };
        // Camera looking at (100, 100): that point lands in the frame's centre.
        let v = cam.view(Affine::translate((100.0, 100.0)), 200.0, 100.0, 0.0);
        assert!((v * Point::new(100.0, 100.0) - Point::new(100.0, 50.0)).hypot() < 1e-9);
        assert!((v * Point::new(110.0, 100.0) - Point::new(120.0, 50.0)).hypot() < 1e-9);
    }

    #[test]
    fn turning_the_camera_turns_the_picture_the_other_way() {
        let cam = CameraLayer::default();
        let world = Affine::translate((0.0, 0.0)) * Affine::rotate(90f64.to_radians());
        let v = cam.view(world, 0.0, 0.0, 0.0);
        // A point to the camera's right (world +y after a 90° turn) appears to the right.
        assert!((v * Point::new(0.0, 10.0) - Point::new(10.0, 0.0)).hypot() < 1e-9);
    }

    #[test]
    fn shake_waits_for_its_start_and_dies_away() {
        let s = CameraShake { decay: 5.0, start: 1.0, ..CameraShake::default() };
        assert_eq!(s.at(0.5), (Vec2::ZERO, 0.0));
        let early: f64 = (1.0..1.2).step_by_f64(0.01).into_iter().map(|t| s.at(t).0.hypot()).fold(0.0, f64::max);
        let late: f64 = (3.0..3.2).step_by_f64(0.01).into_iter().map(|t| s.at(t).0.hypot()).fold(0.0, f64::max);
        assert!(early > 1.0 && late < early / 100.0, "early {early}, late {late}");
    }

    #[test]
    fn the_topmost_visible_active_camera_wins() {
        let mut a = camera_layer(Id(1), 100.0, 100.0);
        let mut b = camera_layer(Id(2), 100.0, 100.0);
        assert_eq!(active_camera(&[a.clone(), b.clone()], 0.0).unwrap().id, Id(2));
        b.meta.visible = false;
        assert_eq!(active_camera(&[a.clone(), b.clone()], 0.0).unwrap().id, Id(1));
        a.meta.out_point = Some(1.0);
        assert!(active_camera(&[a, b], 2.0).is_none());
    }

    trait StepBy {
        fn step_by_f64(self, step: f64) -> Vec<f64>;
    }
    impl StepBy for std::ops::Range<f64> {
        fn step_by_f64(self, step: f64) -> Vec<f64> {
            let n = ((self.end - self.start) / step) as usize;
            (0..n).map(|i| self.start + i as f64 * step).collect()
        }
    }
}
