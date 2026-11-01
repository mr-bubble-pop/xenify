//! 3D solids: simple shaded objects (primitives, or a flat shape extruded)
//! that rotate in 3D and are drawn as vector faces.

use crate::{Color, Property};
use kurbo::BezPath;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SolidShape {
    Cube,
    Sphere,
    Cylinder,
    Cone,
    /// A ring; `thickness` is the tube's radius as a fraction of the outer radius.
    Torus {
        thickness: f64,
    },
    /// A flat outline (layer space, centred on the origin) pushed out into depth.
    Extrude {
        path: BezPath,
        #[serde(default)]
        fill_rule: crate::FillRule,
    },
    /// An imported 3D model.
    Mesh(Box<crate::Mesh>),
}

impl SolidShape {
    pub fn label(&self) -> &'static str {
        match self {
            SolidShape::Cube => "Cube",
            SolidShape::Sphere => "Sphere",
            SolidShape::Cylinder => "Cylinder",
            SolidShape::Cone => "Cone",
            SolidShape::Torus { .. } => "Torus",
            SolidShape::Extrude { .. } => "Extrusion",
            SolidShape::Mesh(_) => "3D model",
        }
    }

    /// Whether the size property applies (extrusions use their outline's size).
    pub fn has_size(&self) -> bool {
        !matches!(self, SolidShape::Extrude { .. })
    }

    /// Whether the depth property applies (models keep their own proportions).
    pub fn has_depth(&self) -> bool {
        !matches!(self, SolidShape::Mesh(_))
    }

    /// The built-in primitives (everything but extrusions).
    pub fn primitives() -> [SolidShape; 5] {
        [
            SolidShape::Cube,
            SolidShape::Sphere,
            SolidShape::Cylinder,
            SolidShape::Cone,
            SolidShape::Torus { thickness: 0.3 },
        ]
    }
}

/// A 3D object layer. Rotations are in degrees: X tilts it forward/back, Y
/// turns it left/right, Z spins it in the screen plane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    pub shape: SolidShape,
    /// Width of primitives in pixels, or a model's largest dimension
    /// (extrusions use their outline's size).
    pub size: Property<f64>,
    /// Depth in pixels (cube/cylinder/cone height, extrusion depth).
    pub depth: Property<f64>,
    pub rot_x: Property<f64>,
    pub rot_y: Property<f64>,
    pub rot_z: Property<f64>,
    pub color: Property<Color>,
    /// Perspective strength: 0 = flat (orthographic), 1 = strong.
    #[serde(default = "default_perspective")]
    pub perspective: f64,
    /// For imported models with several parts: how each part moves on its
    /// own (by part index; parts without an entry stay put).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartMotion>,
}

/// One part of a model moved, turned or hidden on its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartMotion {
    #[serde(default = "yes")]
    pub visible: bool,
    /// Pixels across and down (before the model's own rotation).
    #[serde(default = "zero_vec")]
    pub offset: Property<kurbo::Vec2>,
    /// Pixels away from the viewer (before the model's own rotation).
    #[serde(default = "zero")]
    pub depth: Property<f64>,
    /// Degrees, about the part's own centre.
    #[serde(default = "zero")]
    pub rot_x: Property<f64>,
    #[serde(default = "zero")]
    pub rot_y: Property<f64>,
    #[serde(default = "zero")]
    pub rot_z: Property<f64>,
}

fn yes() -> bool {
    true
}

fn zero() -> Property<f64> {
    0.0.into()
}

fn zero_vec() -> Property<kurbo::Vec2> {
    kurbo::Vec2::ZERO.into()
}

impl Default for PartMotion {
    fn default() -> Self {
        Self { visible: true, offset: zero_vec(), depth: zero(), rot_x: zero(), rot_y: zero(), rot_z: zero() }
    }
}

fn default_perspective() -> f64 {
    0.3
}

impl Solid {
    pub fn new(shape: SolidShape, size: f64, depth: f64, color: Color) -> Self {
        Self {
            shape,
            size: size.into(),
            depth: depth.into(),
            // Seen from a little above and to the left.
            rot_x: 20.0.into(),
            rot_y: (-30.0).into(),
            rot_z: 0.0.into(),
            color: color.into(),
            perspective: default_perspective(),
            parts: Vec::new(),
        }
    }
}
