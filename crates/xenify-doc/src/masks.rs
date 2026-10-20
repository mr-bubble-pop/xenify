//! Layer masks (like After Effects): shapes drawn on a layer that decide
//! which parts of it show. A rectangle mask crops; several masks combine in
//! order (add, subtract, intersect, difference); each can be feathered,
//! faded, inverted and animated.

use crate::Property;
use kurbo::{BezPath, Rect, Shape};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    #[serde(default)]
    pub name: String,
    /// The outline, in the layer's own coordinates (it moves with the layer).
    pub path: Property<BezPath>,
    #[serde(default)]
    pub mode: MaskMode,
    /// Show what's outside the shape instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inverted: bool,
    /// Softness of the edge, in pixels.
    #[serde(default = "zero")]
    pub feather: Property<f64>,
    /// 0..1: how strongly the mask applies.
    #[serde(default = "one")]
    pub opacity: Property<f64>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

fn zero() -> Property<f64> {
    0.0.into()
}

fn one() -> Property<f64> {
    1.0.into()
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

/// How a mask combines with the masks above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    /// Adds its area to what shows.
    #[default]
    Add,
    /// Cuts its area away.
    Subtract,
    /// Shows only where it overlaps what the masks above show.
    Intersect,
    /// Shows where exactly one of it and the masks above show.
    Difference,
}

impl MaskMode {
    pub const ALL: [MaskMode; 4] = [MaskMode::Add, MaskMode::Subtract, MaskMode::Intersect, MaskMode::Difference];

    pub fn label(self) -> &'static str {
        match self {
            MaskMode::Add => "Add",
            MaskMode::Subtract => "Subtract",
            MaskMode::Intersect => "Intersect",
            MaskMode::Difference => "Difference",
        }
    }
}

impl Mask {
    pub fn new(name: impl Into<String>, path: BezPath) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            mode: MaskMode::Add,
            inverted: false,
            feather: zero(),
            opacity: one(),
            enabled: true,
        }
    }

    /// A rectangle mask (for cropping).
    pub fn rect(name: impl Into<String>, r: Rect) -> Self {
        Self::new(name, rect_path(r))
    }

    /// An ellipse mask filling `r`.
    pub fn ellipse(name: impl Into<String>, r: Rect) -> Self {
        Self::new(name, kurbo::Ellipse::from_rect(r).to_path(0.1))
    }
}

/// A closed rectangle, corners clockwise from the top-left (so rectangle
/// masks can be recognised and resized like a crop).
pub fn rect_path(r: Rect) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((r.x0, r.y0));
    p.line_to((r.x1, r.y0));
    p.line_to((r.x1, r.y1));
    p.line_to((r.x0, r.y1));
    p.close_path();
    p
}

/// The rectangle `p` outlines, if it's an axis-aligned rectangle made by
/// [`rect_path`] (or edited while staying one).
pub fn as_rect(p: &BezPath) -> Option<Rect> {
    use kurbo::PathEl::*;
    let els = p.elements();
    let pts: Vec<kurbo::Point> = match els {
        [MoveTo(a), LineTo(b), LineTo(c), LineTo(d), ClosePath] => vec![*a, *b, *c, *d],
        [MoveTo(a), LineTo(b), LineTo(c), LineTo(d), LineTo(e), ClosePath] if e.distance(*a) < 1e-9 => {
            vec![*a, *b, *c, *d]
        }
        _ => return None,
    };
    let eq = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let ok = (eq(pts[0].y, pts[1].y) && eq(pts[1].x, pts[2].x) && eq(pts[2].y, pts[3].y) && eq(pts[3].x, pts[0].x))
        || (eq(pts[0].x, pts[1].x) && eq(pts[1].y, pts[2].y) && eq(pts[2].x, pts[3].x) && eq(pts[3].y, pts[0].y));
    ok.then(|| Rect::from_points(pts[0], pts[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_are_recognised_and_masks_round_trip() {
        let r = Rect::new(10.0, 20.0, 110.0, 70.0);
        assert_eq!(as_rect(&rect_path(r)), Some(r));
        let e = Mask::ellipse("oval", r);
        assert_eq!(as_rect(&e.path.value_at(0.0)), None);
        let m = Mask { mode: MaskMode::Subtract, inverted: true, ..Mask::rect("crop", r) };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Mask>(&json).unwrap(), m);
        // Old or minimal masks: defaults fill in.
        let mut v = serde_json::to_value(Mask::rect("r", r)).unwrap();
        for k in ["feather", "opacity", "mode", "name"] {
            v.as_object_mut().unwrap().remove(k);
        }
        let minimal: Mask = serde_json::from_value(v).unwrap();
        assert!(minimal.enabled && minimal.opacity.value_at(0.0) == 1.0 && minimal.mode == MaskMode::Add);
    }
}
