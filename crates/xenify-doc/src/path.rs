//! Addressing individual animatable properties of a layer, plus type-erased
//! values/properties so commands and the AI can edit any property generically.

use crate::{Color, Easing, Geometry, GradientStop, Layer, LayerKind, ProjectError, Property, Quad};
use kurbo::{BezPath, Vec2};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Which property of a layer to address. Serialises as a dotted string such as
/// `"position"`, `"opacity"` or `"shapes.0.fill_color"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum PropPath {
    Anchor,
    Position,
    Scale,
    Rotation,
    Opacity,
    Skew,
    SkewAxis,
    Shape {
        index: usize,
        field: ShapeField,
    },
    FontSize,
    TextColor,
    /// The four corners of the layer's corner pin (see [`crate::Distort`]).
    Distort,
    /// A property of a 3D solid layer.
    Solid(SolidField),
    /// A property of one of the layer's masks.
    Mask {
        index: usize,
        field: MaskField,
    },
    /// How one part of a 3D model moves on its own.
    SolidPart {
        index: usize,
        field: PartField,
    },
    /// A setting of one of the layer's effects.
    Effect {
        index: usize,
        field: crate::EffectField,
    },
    /// A setting of one of the layer's modifiers.
    Modifier {
        index: usize,
        field: crate::ModifierField,
    },
    /// A setting of a camera layer.
    Camera(CameraField),
    /// A setting of a particle emitter.
    Emitter(crate::EmitterField),
    /// A setting of a liquid fill.
    Liquid(crate::LiquidField),
    /// A video or audio layer's loudness (1 = as recorded).
    Volume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CameraField {
    Zoom,
    ShakeAmount,
    ShakeRotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartField {
    Offset,
    Depth,
    RotX,
    RotY,
    RotZ,
}

impl PartField {
    pub const ALL: [PartField; 5] =
        [PartField::Offset, PartField::Depth, PartField::RotX, PartField::RotY, PartField::RotZ];

    fn name(self) -> &'static str {
        match self {
            PartField::Offset => "offset",
            PartField::Depth => "depth",
            PartField::RotX => "rotation_x",
            PartField::RotY => "rotation_y",
            PartField::RotZ => "rotation_z",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaskField {
    Path,
    Feather,
    Opacity,
}

impl MaskField {
    pub const ALL: [MaskField; 3] = [MaskField::Path, MaskField::Feather, MaskField::Opacity];

    fn name(self) -> &'static str {
        match self {
            MaskField::Path => "path",
            MaskField::Feather => "feather",
            MaskField::Opacity => "opacity",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SolidField {
    Size,
    Depth,
    RotX,
    RotY,
    RotZ,
    Color,
}

impl SolidField {
    pub const ALL: [SolidField; 6] =
        [SolidField::Size, SolidField::Depth, SolidField::RotX, SolidField::RotY, SolidField::RotZ, SolidField::Color];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeField {
    Position,
    Size,
    CornerRadius,
    Path,
    FillColor,
    StrokeColor,
    StrokeWidth,
    TrimStart,
    TrimEnd,
    TrimOffset,
    /// Where the fill gradient starts (a radial gradient's centre).
    GradientStart,
    /// Where the fill gradient ends.
    GradientEnd,
    /// The fill gradient's colour stops.
    GradientColors,
    /// Where the stroke's dash pattern starts along the line.
    DashOffset,
    /// The stroke gradient's start, end and colour stops.
    StrokeGradientStart,
    StrokeGradientEnd,
    StrokeGradientColors,
    /// A live figure's sides, points or teeth.
    Count,
    /// A star's inner radius or a gear's tooth depth.
    Inner,
    /// A gear's hole.
    Hole,
}

impl ShapeField {
    const NAMES: &'static [(&'static str, ShapeField)] = &[
        ("position", ShapeField::Position),
        ("size", ShapeField::Size),
        ("corner_radius", ShapeField::CornerRadius),
        ("path", ShapeField::Path),
        ("fill_color", ShapeField::FillColor),
        ("stroke_color", ShapeField::StrokeColor),
        ("stroke_width", ShapeField::StrokeWidth),
        ("trim_start", ShapeField::TrimStart),
        ("trim_end", ShapeField::TrimEnd),
        ("trim_offset", ShapeField::TrimOffset),
        ("gradient_start", ShapeField::GradientStart),
        ("gradient_end", ShapeField::GradientEnd),
        ("gradient_colors", ShapeField::GradientColors),
        ("dash_offset", ShapeField::DashOffset),
        ("stroke_gradient_start", ShapeField::StrokeGradientStart),
        ("stroke_gradient_end", ShapeField::StrokeGradientEnd),
        ("stroke_gradient_colors", ShapeField::StrokeGradientColors),
        ("count", ShapeField::Count),
        ("inner", ShapeField::Inner),
        ("hole", ShapeField::Hole),
        // Older names: the gradient used to be one value; its colours are the
        // closest match.
        ("gradient", ShapeField::GradientColors),
        ("stroke_gradient", ShapeField::StrokeGradientColors),
    ];

    fn name(self) -> &'static str {
        Self::NAMES.iter().find(|(_, f)| *f == self).unwrap().0
    }

    /// One of the stroke gradient's tracks.
    pub fn is_stroke_gradient(self) -> bool {
        matches!(self, Self::StrokeGradientStart | Self::StrokeGradientEnd | Self::StrokeGradientColors)
    }

    /// The fill's or the stroke's gradient tracks: start, end and colours.
    pub fn gradient_tracks(stroke: bool) -> [ShapeField; 3] {
        if stroke {
            [Self::StrokeGradientStart, Self::StrokeGradientEnd, Self::StrokeGradientColors]
        } else {
            [Self::GradientStart, Self::GradientEnd, Self::GradientColors]
        }
    }
}

impl PropPath {
    const SIMPLE: &'static [(&'static str, PropPath)] = &[
        ("anchor", PropPath::Anchor),
        ("position", PropPath::Position),
        ("scale", PropPath::Scale),
        ("rotation", PropPath::Rotation),
        ("opacity", PropPath::Opacity),
        ("skew", PropPath::Skew),
        ("skew_axis", PropPath::SkewAxis),
        ("font_size", PropPath::FontSize),
        ("text_color", PropPath::TextColor),
        ("distort", PropPath::Distort),
        ("solid_size", PropPath::Solid(SolidField::Size)),
        ("solid_depth", PropPath::Solid(SolidField::Depth)),
        ("rotation_x", PropPath::Solid(SolidField::RotX)),
        ("rotation_y", PropPath::Solid(SolidField::RotY)),
        ("rotation_z", PropPath::Solid(SolidField::RotZ)),
        ("solid_color", PropPath::Solid(SolidField::Color)),
        ("camera_zoom", PropPath::Camera(CameraField::Zoom)),
        ("volume", PropPath::Volume),
        ("shake_amount", PropPath::Camera(CameraField::ShakeAmount)),
        ("shake_rotation", PropPath::Camera(CameraField::ShakeRotation)),
    ];

    /// The five transform/opacity properties every layer has.
    pub const LAYER_PROPS: [PropPath; 5] =
        [PropPath::Anchor, PropPath::Position, PropPath::Scale, PropPath::Rotation, PropPath::Opacity];

    pub fn shape(index: usize, field: ShapeField) -> Self {
        PropPath::Shape { index, field }
    }
}

impl fmt::Display for PropPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropPath::Shape { index, field } => write!(f, "shapes.{index}.{}", field.name()),
            PropPath::Mask { index, field } => write!(f, "masks.{index}.{}", field.name()),
            PropPath::SolidPart { index, field } => write!(f, "parts.{index}.{}", field.name()),
            PropPath::Effect { index, field } => write!(f, "effects.{index}.{}", field.name()),
            PropPath::Modifier { index, field } => write!(f, "modifiers.{index}.{}", field.name()),
            PropPath::Emitter(field) => write!(f, "emitter.{}", field.name()),
            PropPath::Liquid(field) => write!(f, "liquid.{}", field.name()),
            p => {
                let name = Self::SIMPLE.iter().find(|(_, q)| q == p).unwrap().0;
                f.write_str(name)
            }
        }
    }
}

impl FromStr for PropPath {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((_, p)) = Self::SIMPLE.iter().find(|(n, _)| *n == s) {
            return Ok(*p);
        }
        let parts: Vec<&str> = s.split('.').collect();
        if let ["shapes", index, field] = parts[..] {
            let index = index.parse().map_err(|_| format!("bad shape index in property path {s:?}"))?;
            let field = ShapeField::NAMES
                .iter()
                .find(|(n, _)| *n == field)
                .map(|(_, f)| *f)
                .ok_or_else(|| format!("unknown shape field in property path {s:?}"))?;
            return Ok(PropPath::Shape { index, field });
        }
        if let ["masks", index, field] = parts[..] {
            let index = index.parse().map_err(|_| format!("bad mask index in property path {s:?}"))?;
            let field = MaskField::ALL
                .into_iter()
                .find(|f| f.name() == field)
                .ok_or_else(|| format!("unknown mask field in property path {s:?}"))?;
            return Ok(PropPath::Mask { index, field });
        }
        if let ["parts", index, field] = parts[..] {
            let index = index.parse().map_err(|_| format!("bad part index in property path {s:?}"))?;
            let field = PartField::ALL
                .into_iter()
                .find(|f| f.name() == field)
                .ok_or_else(|| format!("unknown part setting in property path {s:?}"))?;
            return Ok(PropPath::SolidPart { index, field });
        }
        if let ["effects", index, field] = parts[..] {
            let index = index.parse().map_err(|_| format!("bad effect index in property path {s:?}"))?;
            let field = crate::EffectField::ALL
                .into_iter()
                .find(|f| f.name() == field)
                .ok_or_else(|| format!("unknown effect setting in property path {s:?}"))?;
            return Ok(PropPath::Effect { index, field });
        }
        if let ["liquid", field] = parts[..] {
            let field = crate::LiquidField::ALL
                .into_iter()
                .find(|f| f.name() == field)
                .ok_or_else(|| format!("unknown liquid setting in property path {s:?}"))?;
            return Ok(PropPath::Liquid(field));
        }
        if let ["emitter", field] = parts[..] {
            let field = crate::EmitterField::ALL
                .into_iter()
                .find(|f| f.name() == field)
                .ok_or_else(|| format!("unknown emitter setting in property path {s:?}"))?;
            return Ok(PropPath::Emitter(field));
        }
        if let ["modifiers", index, field] = parts[..] {
            let index = index.parse().map_err(|_| format!("bad modifier index in property path {s:?}"))?;
            let field = crate::ModifierField::ALL
                .into_iter()
                .find(|f| f.name() == field)
                .ok_or_else(|| format!("unknown modifier setting in property path {s:?}"))?;
            return Ok(PropPath::Modifier { index, field });
        }
        Err(format!("unknown property path {s:?}"))
    }
}

impl TryFrom<String> for PropPath {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<PropPath> for String {
    fn from(p: PropPath) -> Self {
        p.to_string()
    }
}

/// The value type stored by a property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Scalar,
    Vec2,
    Color,
    Path,
    /// Gradient colour stops.
    #[serde(alias = "gradient")]
    Stops,
    Quad,
}

/// A single property value of any kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Scalar(f64),
    Vec2(Vec2),
    Color(Color),
    Path(BezPath),
    Stops(Vec<GradientStop>),
    Quad(Quad),
}

impl Value {
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Scalar(_) => ValueKind::Scalar,
            Value::Vec2(_) => ValueKind::Vec2,
            Value::Color(_) => ValueKind::Color,
            Value::Path(_) => ValueKind::Path,
            Value::Stops(_) => ValueKind::Stops,
            Value::Quad(_) => ValueKind::Quad,
        }
    }
}

/// A whole property (static or keyframed) of any kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnyProperty {
    Scalar(Property<f64>),
    Vec2(Property<Vec2>),
    Color(Property<Color>),
    Path(Property<BezPath>),
    Stops(Property<Vec<GradientStop>>),
    Quad(Property<Quad>),
}

macro_rules! any_prop_dispatch {
    ($prop:expr, |$p:ident| $body:expr) => {
        match $prop {
            AnyProperty::Scalar($p) => $body,
            AnyProperty::Vec2($p) => $body,
            AnyProperty::Color($p) => $body,
            AnyProperty::Path($p) => $body,
            AnyProperty::Stops($p) => $body,
            AnyProperty::Quad($p) => $body,
        }
    };
}

impl AnyProperty {
    pub fn kind(&self) -> ValueKind {
        match self {
            AnyProperty::Scalar(_) => ValueKind::Scalar,
            AnyProperty::Vec2(_) => ValueKind::Vec2,
            AnyProperty::Color(_) => ValueKind::Color,
            AnyProperty::Path(_) => ValueKind::Path,
            AnyProperty::Stops(_) => ValueKind::Stops,
            AnyProperty::Quad(_) => ValueKind::Quad,
        }
    }

    pub fn is_animated(&self) -> bool {
        any_prop_dispatch!(self, |p| p.is_animated())
    }

    pub fn keyframe_times(&self) -> Vec<f64> {
        any_prop_dispatch!(self, |p| p.keyframes().iter().map(|k| k.time).collect())
    }

    /// See [`Property::move_keyframe`].
    pub fn move_keyframe(&mut self, from: f64, to: f64) -> bool {
        any_prop_dispatch!(self, |p| p.move_keyframe(from, to))
    }

    /// See [`Property::set_easing`].
    pub fn set_easing(&mut self, time: f64, easing: Easing) -> bool {
        any_prop_dispatch!(self, |p| p.set_easing(time, easing))
    }

    /// Easing of the keyframe at `time`, if there is one.
    pub fn easing_at(&self, time: f64) -> Option<Easing> {
        any_prop_dispatch!(self, |p| p
            .keyframes()
            .iter()
            .find(|k| (k.time - time).abs() < crate::property::TIME_EPSILON)
            .map(|k| k.easing))
    }
}

/// Shared, type-erased borrow of a property inside a layer.
enum PropRef<'a> {
    Scalar(&'a Property<f64>),
    Vec2(&'a Property<Vec2>),
    Color(&'a Property<Color>),
    Path(&'a Property<BezPath>),
    Stops(&'a Property<Vec<GradientStop>>),
    Quad(&'a Property<Quad>),
}

impl PropRef<'_> {
    fn to_owned(&self) -> AnyProperty {
        match self {
            PropRef::Scalar(p) => AnyProperty::Scalar((*p).clone()),
            PropRef::Vec2(p) => AnyProperty::Vec2((*p).clone()),
            PropRef::Color(p) => AnyProperty::Color((*p).clone()),
            PropRef::Path(p) => AnyProperty::Path((*p).clone()),
            PropRef::Stops(p) => AnyProperty::Stops((*p).clone()),
            PropRef::Quad(p) => AnyProperty::Quad((*p).clone()),
        }
    }

    fn value_at(&self, time: f64) -> Value {
        match self {
            PropRef::Scalar(p) => Value::Scalar(p.value_at(time)),
            PropRef::Vec2(p) => Value::Vec2(p.value_at(time)),
            PropRef::Color(p) => Value::Color(p.value_at(time)),
            PropRef::Path(p) => Value::Path(p.value_at(time)),
            PropRef::Stops(p) => Value::Stops(p.value_at(time)),
            PropRef::Quad(p) => Value::Quad(p.value_at(time)),
        }
    }

    fn keyframe_times(&self) -> Vec<f64> {
        fn times<T>(p: &Property<T>) -> Vec<f64> {
            match p {
                Property::Static(_) => Vec::new(),
                Property::Animated(k) => k.iter().map(|k| k.time).collect(),
            }
        }
        match self {
            PropRef::Scalar(p) => times(p),
            PropRef::Vec2(p) => times(p),
            PropRef::Color(p) => times(p),
            PropRef::Path(p) => times(p),
            PropRef::Stops(p) => times(p),
            PropRef::Quad(p) => times(p),
        }
    }
}

/// Mutable, type-erased borrow of a property inside a layer.
enum PropMut<'a> {
    Scalar(&'a mut Property<f64>),
    Vec2(&'a mut Property<Vec2>),
    Color(&'a mut Property<Color>),
    Path(&'a mut Property<BezPath>),
    Stops(&'a mut Property<Vec<GradientStop>>),
    Quad(&'a mut Property<Quad>),
}

/// Runs `$body` with `$p` bound to the typed `&mut Property<T>` and `$variant`
/// aliased to the matching [`Value`] variant (usable as constructor and pattern).
macro_rules! dispatch {
    ($prop:expr, |$p:ident, $variant:ident| $body:expr) => {
        match $prop {
            PropMut::Scalar($p) => {
                #[allow(unused_imports)]
                use Value::Scalar as $variant;
                $body
            }
            PropMut::Vec2($p) => {
                #[allow(unused_imports)]
                use Value::Vec2 as $variant;
                $body
            }
            PropMut::Color($p) => {
                #[allow(unused_imports)]
                use Value::Color as $variant;
                $body
            }
            PropMut::Path($p) => {
                #[allow(unused_imports)]
                use Value::Path as $variant;
                $body
            }
            PropMut::Stops($p) => {
                #[allow(unused_imports)]
                use Value::Stops as $variant;
                $body
            }
            PropMut::Quad($p) => {
                #[allow(unused_imports)]
                use Value::Quad as $variant;
                $body
            }
        }
    };
}

impl PropMut<'_> {
    fn kind(&self) -> ValueKind {
        match self {
            PropMut::Scalar(_) => ValueKind::Scalar,
            PropMut::Vec2(_) => ValueKind::Vec2,
            PropMut::Color(_) => ValueKind::Color,
            PropMut::Path(_) => ValueKind::Path,
            PropMut::Stops(_) => ValueKind::Stops,
            PropMut::Quad(_) => ValueKind::Quad,
        }
    }

    fn replace(self, new: AnyProperty) -> Result<AnyProperty, ValueKind> {
        let expected = self.kind();
        Ok(match (self, new) {
            (PropMut::Scalar(p), AnyProperty::Scalar(n)) => AnyProperty::Scalar(std::mem::replace(p, n)),
            (PropMut::Vec2(p), AnyProperty::Vec2(n)) => AnyProperty::Vec2(std::mem::replace(p, n)),
            (PropMut::Color(p), AnyProperty::Color(n)) => AnyProperty::Color(std::mem::replace(p, n)),
            (PropMut::Path(p), AnyProperty::Path(n)) => AnyProperty::Path(std::mem::replace(p, n)),
            (PropMut::Stops(p), AnyProperty::Stops(n)) => AnyProperty::Stops(std::mem::replace(p, n)),
            (PropMut::Quad(p), AnyProperty::Quad(n)) => AnyProperty::Quad(std::mem::replace(p, n)),
            _ => return Err(expected),
        })
    }
}

impl Layer {
    fn prop_mut(&mut self, path: PropPath) -> Result<PropMut<'_>, ProjectError> {
        let id = self.id;
        let missing = move || ProjectError::NoSuchProperty { layer: id, path };
        Ok(match path {
            PropPath::Anchor => PropMut::Vec2(&mut self.transform.anchor),
            PropPath::Position => PropMut::Vec2(&mut self.transform.position),
            PropPath::Scale => PropMut::Vec2(&mut self.transform.scale),
            PropPath::Rotation => PropMut::Scalar(&mut self.transform.rotation),
            PropPath::Skew => PropMut::Scalar(&mut self.transform.skew),
            PropPath::SkewAxis => PropMut::Scalar(&mut self.transform.skew_axis),
            PropPath::Opacity => PropMut::Scalar(&mut self.opacity),
            PropPath::Distort => PropMut::Quad(&mut self.transform.distort.as_mut().ok_or_else(missing)?.corners),
            PropPath::SolidPart { index, field } => {
                let LayerKind::Solid(s) = &mut self.kind else { return Err(missing()) };
                let p = s.parts.get_mut(index).ok_or_else(missing)?;
                match field {
                    PartField::Offset => PropMut::Vec2(&mut p.offset),
                    PartField::Depth => PropMut::Scalar(&mut p.depth),
                    PartField::RotX => PropMut::Scalar(&mut p.rot_x),
                    PartField::RotY => PropMut::Scalar(&mut p.rot_y),
                    PartField::RotZ => PropMut::Scalar(&mut p.rot_z),
                }
            }
            PropPath::Effect { index, field } => {
                use crate::{EffectField as F, EffectKind as K};
                let e = self.meta.effects.get_mut(index).ok_or_else(missing)?;
                match (&mut e.kind, field) {
                    (K::DropShadow { color, .. } | K::Glow { color, .. }, F::Color) => PropMut::Color(color),
                    (K::DropShadow { offset, .. }, F::Offset) => PropMut::Vec2(offset),
                    (K::DropShadow { blur: r, .. } | K::Glow { radius: r, .. } | K::Blur { radius: r }, F::Radius) => {
                        PropMut::Scalar(r)
                    }
                    (K::AdjustColors { brightness, .. }, F::Brightness) => PropMut::Scalar(brightness),
                    (K::AdjustColors { contrast, .. }, F::Contrast) => PropMut::Scalar(contrast),
                    (K::AdjustColors { saturation, .. }, F::Saturation) => PropMut::Scalar(saturation),
                    (K::AdjustColors { hue, .. }, F::Hue) => PropMut::Scalar(hue),
                    _ => return Err(missing()),
                }
            }
            PropPath::Modifier { index, field } => {
                use crate::{ModifierField as F, ModifierKind as K};
                let m = self.meta.modifiers.get_mut(index).ok_or_else(missing)?;
                match (&mut m.kind, field) {
                    (K::Array { count, .. }, F::Count) => PropMut::Scalar(count),
                    (K::Array { offset, .. }, F::Offset) => PropMut::Vec2(offset),
                    (K::Array { rotation, .. }, F::Rotation) => PropMut::Scalar(rotation),
                    (K::Array { scale, .. }, F::Scale) => PropMut::Vec2(scale),
                    (K::Array { origin, .. }, F::Origin) => PropMut::Vec2(origin),
                    (K::Array { end_opacity, .. }, F::EndOpacity) => PropMut::Scalar(end_opacity),
                    (K::Twist { rotation, .. }, F::Rotation) => PropMut::Scalar(rotation),
                    (K::Twist { origin, .. }, F::Origin) => PropMut::Vec2(origin),
                    (
                        K::Jitter { amount: v, .. } | K::ZigZag { amount: v, .. } | K::OffsetPath { amount: v, .. },
                        F::Amount,
                    ) => PropMut::Scalar(v),
                    (K::PuckerBloat { strength: v } | K::Smooth { strength: v }, F::Strength) => PropMut::Scalar(v),
                    (K::Jitter { detail, .. }, F::Detail) => PropMut::Scalar(detail),
                    (K::Jitter { seed, .. }, F::Seed) => PropMut::Scalar(seed),
                    (K::ZigZag { ridges, .. }, F::Ridges) => PropMut::Scalar(ridges),
                    (K::Subdivide { levels }, F::Levels) => PropMut::Scalar(levels),
                    (K::Simplify { tolerance }, F::Tolerance) => PropMut::Scalar(tolerance),
                    (K::Mirror { origin, .. }, F::Origin) => PropMut::Vec2(origin),
                    (K::Mirror { rotation, .. }, F::Rotation) => PropMut::Scalar(rotation),
                    (K::Blend { count, .. } | K::PathArray { count, .. }, F::Count) => PropMut::Scalar(count),
                    (K::PathArray { start, .. }, F::Start) => PropMut::Scalar(start),
                    (K::PathArray { end, .. }, F::End) => PropMut::Scalar(end),
                    (K::Shatter { count, .. }, F::Count) => PropMut::Scalar(count),
                    (K::Shatter { origin, .. }, F::Origin) => PropMut::Vec2(origin),
                    (K::Shatter { force, .. }, F::Force) => PropMut::Scalar(force),
                    (K::Shatter { amount, .. }, F::Amount) => PropMut::Scalar(amount),
                    (K::Shatter { rotation, .. }, F::Rotation) => PropMut::Scalar(rotation),
                    (K::Shatter { gravity, .. }, F::Gravity) => PropMut::Scalar(gravity),
                    (K::Shatter { spin, .. }, F::Spin) => PropMut::Scalar(spin),
                    (K::Shatter { progress, .. }, F::Progress) => PropMut::Scalar(progress),
                    (K::Shatter { seed, .. }, F::Seed) => PropMut::Scalar(seed),
                    _ => return Err(missing()),
                }
            }
            PropPath::Mask { index, field } => {
                let m = self.meta.masks.get_mut(index).ok_or_else(missing)?;
                match field {
                    MaskField::Path => PropMut::Path(&mut m.path),
                    MaskField::Feather => PropMut::Scalar(&mut m.feather),
                    MaskField::Opacity => PropMut::Scalar(&mut m.opacity),
                }
            }
            PropPath::Volume => match &mut self.kind {
                LayerKind::Video(v) => PropMut::Scalar(&mut v.volume),
                LayerKind::Audio(a) => PropMut::Scalar(&mut a.volume),
                _ => return Err(missing()),
            },
            PropPath::Liquid(field) => {
                use crate::LiquidField as F;
                let LayerKind::Liquid(w) = &mut self.kind else { return Err(missing()) };
                match field {
                    F::Level => PropMut::Scalar(&mut w.level),
                    F::Tilt => PropMut::Scalar(&mut w.tilt),
                    F::WaveHeight => PropMut::Scalar(&mut w.wave_height),
                    F::Color => PropMut::Color(&mut w.color),
                }
            }
            PropPath::Emitter(field) => {
                use crate::EmitterField as F;
                let LayerKind::Particles(e) = &mut self.kind else { return Err(missing()) };
                match field {
                    F::Count => PropMut::Scalar(&mut e.count),
                    F::Rate => PropMut::Scalar(&mut e.rate),
                    F::Lifetime => PropMut::Scalar(&mut e.lifetime),
                    F::Direction => PropMut::Scalar(&mut e.direction),
                    F::Spread => PropMut::Scalar(&mut e.spread),
                    F::Speed => PropMut::Scalar(&mut e.speed),
                    F::Spin => PropMut::Scalar(&mut e.spin),
                    F::Gravity => PropMut::Vec2(&mut e.gravity),
                    F::Drag => PropMut::Scalar(&mut e.drag),
                    F::Turbulence => PropMut::Scalar(&mut e.turbulence),
                    F::Size => PropMut::Scalar(&mut e.size),
                    F::Vortex => PropMut::Scalar(&mut e.vortex),
                }
            }
            PropPath::Camera(field) => {
                let LayerKind::Camera(c) = &mut self.kind else { return Err(missing()) };
                match field {
                    CameraField::Zoom => PropMut::Scalar(&mut c.zoom),
                    CameraField::ShakeAmount => PropMut::Scalar(&mut c.shake.as_mut().ok_or_else(missing)?.amount),
                    CameraField::ShakeRotation => PropMut::Scalar(&mut c.shake.as_mut().ok_or_else(missing)?.rotation),
                }
            }
            PropPath::Solid(field) => {
                let LayerKind::Solid(s) = &mut self.kind else { return Err(missing()) };
                match field {
                    SolidField::Size => PropMut::Scalar(&mut s.size),
                    SolidField::Depth => PropMut::Scalar(&mut s.depth),
                    SolidField::RotX => PropMut::Scalar(&mut s.rot_x),
                    SolidField::RotY => PropMut::Scalar(&mut s.rot_y),
                    SolidField::RotZ => PropMut::Scalar(&mut s.rot_z),
                    SolidField::Color => PropMut::Color(&mut s.color),
                }
            }
            PropPath::FontSize | PropPath::TextColor => {
                let LayerKind::Text(text) = &mut self.kind else {
                    return Err(missing());
                };
                if path == PropPath::FontSize {
                    PropMut::Scalar(&mut text.font_size)
                } else {
                    PropMut::Color(&mut text.color)
                }
            }
            PropPath::Shape { index, field } => {
                let shape = self.shapes_mut().and_then(|s| s.get_mut(index)).ok_or_else(missing)?;
                match (field, &mut shape.geometry) {
                    (
                        ShapeField::Position,
                        Geometry::Rect { position, .. }
                        | Geometry::Ellipse { position, .. }
                        | Geometry::Figure { position, .. },
                    ) => PropMut::Vec2(position),
                    (
                        ShapeField::Size,
                        Geometry::Rect { size, .. } | Geometry::Ellipse { size, .. } | Geometry::Figure { size, .. },
                    ) => PropMut::Vec2(size),
                    (ShapeField::CornerRadius, Geometry::Rect { corner_radius, .. }) => PropMut::Scalar(corner_radius),
                    (ShapeField::Count, Geometry::Figure { count, .. }) => PropMut::Scalar(count),
                    (ShapeField::Inner, Geometry::Figure { inner, .. }) => PropMut::Scalar(inner),
                    (ShapeField::Hole, Geometry::Figure { hole, .. }) => PropMut::Scalar(hole),
                    (ShapeField::Path, Geometry::Path { path }) => PropMut::Path(path),
                    (ShapeField::FillColor, _) => PropMut::Color(&mut shape.fill.as_mut().ok_or_else(missing)?.color),
                    (
                        f @ (ShapeField::GradientStart
                        | ShapeField::GradientEnd
                        | ShapeField::GradientColors
                        | ShapeField::StrokeGradientStart
                        | ShapeField::StrokeGradientEnd
                        | ShapeField::StrokeGradientColors),
                        _,
                    ) => {
                        let g = if f.is_stroke_gradient() {
                            shape.stroke.as_mut().and_then(|s| s.gradient.as_mut())
                        } else {
                            shape.fill.as_mut().and_then(|f| f.gradient.as_mut())
                        }
                        .ok_or_else(missing)?;
                        match f {
                            ShapeField::GradientStart | ShapeField::StrokeGradientStart => PropMut::Vec2(&mut g.start),
                            ShapeField::GradientEnd | ShapeField::StrokeGradientEnd => PropMut::Vec2(&mut g.end),
                            _ => PropMut::Stops(&mut g.stops),
                        }
                    }
                    (ShapeField::StrokeColor, _) => {
                        PropMut::Color(&mut shape.stroke.as_mut().ok_or_else(missing)?.color)
                    }
                    (ShapeField::StrokeWidth, _) => {
                        PropMut::Scalar(&mut shape.stroke.as_mut().ok_or_else(missing)?.width)
                    }
                    (ShapeField::DashOffset, _) => {
                        PropMut::Scalar(&mut shape.stroke.as_mut().ok_or_else(missing)?.dash_offset)
                    }

                    (ShapeField::TrimStart, _) => PropMut::Scalar(&mut shape.trim.as_mut().ok_or_else(missing)?.start),
                    (ShapeField::TrimEnd, _) => PropMut::Scalar(&mut shape.trim.as_mut().ok_or_else(missing)?.end),
                    (ShapeField::TrimOffset, _) => {
                        PropMut::Scalar(&mut shape.trim.as_mut().ok_or_else(missing)?.offset)
                    }
                    _ => return Err(missing()),
                }
            }
        })
    }

    fn prop_ref(&self, path: PropPath) -> Result<PropRef<'_>, ProjectError> {
        let id = self.id;
        let missing = move || ProjectError::NoSuchProperty { layer: id, path };
        Ok(match path {
            PropPath::Anchor => PropRef::Vec2(&self.transform.anchor),
            PropPath::Position => PropRef::Vec2(&self.transform.position),
            PropPath::Scale => PropRef::Vec2(&self.transform.scale),
            PropPath::Rotation => PropRef::Scalar(&self.transform.rotation),
            PropPath::Skew => PropRef::Scalar(&self.transform.skew),
            PropPath::SkewAxis => PropRef::Scalar(&self.transform.skew_axis),
            PropPath::Opacity => PropRef::Scalar(&self.opacity),
            PropPath::Distort => PropRef::Quad(&self.transform.distort.as_ref().ok_or_else(missing)?.corners),
            PropPath::SolidPart { index, field } => {
                let LayerKind::Solid(s) = &self.kind else { return Err(missing()) };
                let p = s.parts.get(index).ok_or_else(missing)?;
                match field {
                    PartField::Offset => PropRef::Vec2(&p.offset),
                    PartField::Depth => PropRef::Scalar(&p.depth),
                    PartField::RotX => PropRef::Scalar(&p.rot_x),
                    PartField::RotY => PropRef::Scalar(&p.rot_y),
                    PartField::RotZ => PropRef::Scalar(&p.rot_z),
                }
            }
            PropPath::Effect { index, field } => {
                use crate::{EffectField as F, EffectKind as K};
                let e = self.meta.effects.get(index).ok_or_else(missing)?;
                match (&e.kind, field) {
                    (K::DropShadow { color, .. } | K::Glow { color, .. }, F::Color) => PropRef::Color(color),
                    (K::DropShadow { offset, .. }, F::Offset) => PropRef::Vec2(offset),
                    (K::DropShadow { blur: r, .. } | K::Glow { radius: r, .. } | K::Blur { radius: r }, F::Radius) => {
                        PropRef::Scalar(r)
                    }
                    (K::AdjustColors { brightness, .. }, F::Brightness) => PropRef::Scalar(brightness),
                    (K::AdjustColors { contrast, .. }, F::Contrast) => PropRef::Scalar(contrast),
                    (K::AdjustColors { saturation, .. }, F::Saturation) => PropRef::Scalar(saturation),
                    (K::AdjustColors { hue, .. }, F::Hue) => PropRef::Scalar(hue),
                    _ => return Err(missing()),
                }
            }
            PropPath::Modifier { index, field } => {
                use crate::{ModifierField as F, ModifierKind as K};
                let m = self.meta.modifiers.get(index).ok_or_else(missing)?;
                match (&m.kind, field) {
                    (K::Array { count, .. }, F::Count) => PropRef::Scalar(count),
                    (K::Array { offset, .. }, F::Offset) => PropRef::Vec2(offset),
                    (K::Array { rotation, .. }, F::Rotation) => PropRef::Scalar(rotation),
                    (K::Array { scale, .. }, F::Scale) => PropRef::Vec2(scale),
                    (K::Array { origin, .. }, F::Origin) => PropRef::Vec2(origin),
                    (K::Array { end_opacity, .. }, F::EndOpacity) => PropRef::Scalar(end_opacity),
                    (K::Twist { rotation, .. }, F::Rotation) => PropRef::Scalar(rotation),
                    (K::Twist { origin, .. }, F::Origin) => PropRef::Vec2(origin),
                    (
                        K::Jitter { amount: v, .. } | K::ZigZag { amount: v, .. } | K::OffsetPath { amount: v, .. },
                        F::Amount,
                    ) => PropRef::Scalar(v),
                    (K::PuckerBloat { strength: v } | K::Smooth { strength: v }, F::Strength) => PropRef::Scalar(v),
                    (K::Jitter { detail, .. }, F::Detail) => PropRef::Scalar(detail),
                    (K::Jitter { seed, .. }, F::Seed) => PropRef::Scalar(seed),
                    (K::ZigZag { ridges, .. }, F::Ridges) => PropRef::Scalar(ridges),
                    (K::Subdivide { levels }, F::Levels) => PropRef::Scalar(levels),
                    (K::Simplify { tolerance }, F::Tolerance) => PropRef::Scalar(tolerance),
                    (K::Mirror { origin, .. }, F::Origin) => PropRef::Vec2(origin),
                    (K::Mirror { rotation, .. }, F::Rotation) => PropRef::Scalar(rotation),
                    (K::Blend { count, .. } | K::PathArray { count, .. }, F::Count) => PropRef::Scalar(count),
                    (K::PathArray { start, .. }, F::Start) => PropRef::Scalar(start),
                    (K::PathArray { end, .. }, F::End) => PropRef::Scalar(end),
                    (K::Shatter { count, .. }, F::Count) => PropRef::Scalar(count),
                    (K::Shatter { origin, .. }, F::Origin) => PropRef::Vec2(origin),
                    (K::Shatter { force, .. }, F::Force) => PropRef::Scalar(force),
                    (K::Shatter { amount, .. }, F::Amount) => PropRef::Scalar(amount),
                    (K::Shatter { rotation, .. }, F::Rotation) => PropRef::Scalar(rotation),
                    (K::Shatter { gravity, .. }, F::Gravity) => PropRef::Scalar(gravity),
                    (K::Shatter { spin, .. }, F::Spin) => PropRef::Scalar(spin),
                    (K::Shatter { progress, .. }, F::Progress) => PropRef::Scalar(progress),
                    (K::Shatter { seed, .. }, F::Seed) => PropRef::Scalar(seed),
                    _ => return Err(missing()),
                }
            }
            PropPath::Mask { index, field } => {
                let m = self.meta.masks.get(index).ok_or_else(missing)?;
                match field {
                    MaskField::Path => PropRef::Path(&m.path),
                    MaskField::Feather => PropRef::Scalar(&m.feather),
                    MaskField::Opacity => PropRef::Scalar(&m.opacity),
                }
            }
            PropPath::Volume => match &self.kind {
                LayerKind::Video(v) => PropRef::Scalar(&v.volume),
                LayerKind::Audio(a) => PropRef::Scalar(&a.volume),
                _ => return Err(missing()),
            },
            PropPath::Liquid(field) => {
                use crate::LiquidField as F;
                let LayerKind::Liquid(w) = &self.kind else { return Err(missing()) };
                match field {
                    F::Level => PropRef::Scalar(&w.level),
                    F::Tilt => PropRef::Scalar(&w.tilt),
                    F::WaveHeight => PropRef::Scalar(&w.wave_height),
                    F::Color => PropRef::Color(&w.color),
                }
            }
            PropPath::Emitter(field) => {
                use crate::EmitterField as F;
                let LayerKind::Particles(e) = &self.kind else { return Err(missing()) };
                match field {
                    F::Count => PropRef::Scalar(&e.count),
                    F::Rate => PropRef::Scalar(&e.rate),
                    F::Lifetime => PropRef::Scalar(&e.lifetime),
                    F::Direction => PropRef::Scalar(&e.direction),
                    F::Spread => PropRef::Scalar(&e.spread),
                    F::Speed => PropRef::Scalar(&e.speed),
                    F::Spin => PropRef::Scalar(&e.spin),
                    F::Gravity => PropRef::Vec2(&e.gravity),
                    F::Drag => PropRef::Scalar(&e.drag),
                    F::Turbulence => PropRef::Scalar(&e.turbulence),
                    F::Size => PropRef::Scalar(&e.size),
                    F::Vortex => PropRef::Scalar(&e.vortex),
                }
            }
            PropPath::Camera(field) => {
                let LayerKind::Camera(c) = &self.kind else { return Err(missing()) };
                match field {
                    CameraField::Zoom => PropRef::Scalar(&c.zoom),
                    CameraField::ShakeAmount => PropRef::Scalar(&c.shake.as_ref().ok_or_else(missing)?.amount),
                    CameraField::ShakeRotation => PropRef::Scalar(&c.shake.as_ref().ok_or_else(missing)?.rotation),
                }
            }
            PropPath::Solid(field) => {
                let LayerKind::Solid(s) = &self.kind else { return Err(missing()) };
                match field {
                    SolidField::Size => PropRef::Scalar(&s.size),
                    SolidField::Depth => PropRef::Scalar(&s.depth),
                    SolidField::RotX => PropRef::Scalar(&s.rot_x),
                    SolidField::RotY => PropRef::Scalar(&s.rot_y),
                    SolidField::RotZ => PropRef::Scalar(&s.rot_z),
                    SolidField::Color => PropRef::Color(&s.color),
                }
            }
            PropPath::FontSize | PropPath::TextColor => {
                let LayerKind::Text(text) = &self.kind else {
                    return Err(missing());
                };
                if path == PropPath::FontSize { PropRef::Scalar(&text.font_size) } else { PropRef::Color(&text.color) }
            }
            PropPath::Shape { index, field } => {
                let shape = self.shapes().get(index).ok_or_else(missing)?;
                match (field, &shape.geometry) {
                    (
                        ShapeField::Position,
                        Geometry::Rect { position, .. }
                        | Geometry::Ellipse { position, .. }
                        | Geometry::Figure { position, .. },
                    ) => PropRef::Vec2(position),
                    (
                        ShapeField::Size,
                        Geometry::Rect { size, .. } | Geometry::Ellipse { size, .. } | Geometry::Figure { size, .. },
                    ) => PropRef::Vec2(size),
                    (ShapeField::CornerRadius, Geometry::Rect { corner_radius, .. }) => PropRef::Scalar(corner_radius),
                    (ShapeField::Count, Geometry::Figure { count, .. }) => PropRef::Scalar(count),
                    (ShapeField::Inner, Geometry::Figure { inner, .. }) => PropRef::Scalar(inner),
                    (ShapeField::Hole, Geometry::Figure { hole, .. }) => PropRef::Scalar(hole),
                    (ShapeField::Path, Geometry::Path { path }) => PropRef::Path(path),
                    (ShapeField::FillColor, _) => PropRef::Color(&shape.fill.as_ref().ok_or_else(missing)?.color),
                    (
                        f @ (ShapeField::GradientStart
                        | ShapeField::GradientEnd
                        | ShapeField::GradientColors
                        | ShapeField::StrokeGradientStart
                        | ShapeField::StrokeGradientEnd
                        | ShapeField::StrokeGradientColors),
                        _,
                    ) => {
                        let g = if f.is_stroke_gradient() {
                            shape.stroke.as_ref().and_then(|s| s.gradient.as_ref())
                        } else {
                            shape.fill.as_ref().and_then(|f| f.gradient.as_ref())
                        }
                        .ok_or_else(missing)?;
                        match f {
                            ShapeField::GradientStart | ShapeField::StrokeGradientStart => PropRef::Vec2(&g.start),
                            ShapeField::GradientEnd | ShapeField::StrokeGradientEnd => PropRef::Vec2(&g.end),
                            _ => PropRef::Stops(&g.stops),
                        }
                    }
                    (ShapeField::StrokeColor, _) => PropRef::Color(&shape.stroke.as_ref().ok_or_else(missing)?.color),
                    (ShapeField::StrokeWidth, _) => PropRef::Scalar(&shape.stroke.as_ref().ok_or_else(missing)?.width),
                    (ShapeField::DashOffset, _) => {
                        PropRef::Scalar(&shape.stroke.as_ref().ok_or_else(missing)?.dash_offset)
                    }

                    (ShapeField::TrimStart, _) => PropRef::Scalar(&shape.trim.as_ref().ok_or_else(missing)?.start),
                    (ShapeField::TrimEnd, _) => PropRef::Scalar(&shape.trim.as_ref().ok_or_else(missing)?.end),
                    (ShapeField::TrimOffset, _) => PropRef::Scalar(&shape.trim.as_ref().ok_or_else(missing)?.offset),
                    _ => return Err(missing()),
                }
            }
        })
    }

    fn mismatch(&self, path: PropPath, expected: ValueKind) -> ProjectError {
        ProjectError::TypeMismatch { layer: self.id, path, expected }
    }

    /// Every property path that exists on this layer.
    pub fn property_paths(&self) -> Vec<PropPath> {
        let mut out = PropPath::LAYER_PROPS.to_vec();
        out.extend([PropPath::Skew, PropPath::SkewAxis]);
        if self.transform.distort.is_some() {
            out.push(PropPath::Distort);
        }
        for index in 0..self.meta.masks.len() {
            out.extend(MaskField::ALL.map(|field| PropPath::Mask { index, field }));
        }
        for (index, e) in self.meta.effects.iter().enumerate() {
            out.extend(e.kind.fields().iter().map(|&field| PropPath::Effect { index, field }));
        }
        for (index, m) in self.meta.modifiers.iter().enumerate() {
            out.extend(m.kind.fields().iter().map(|&field| PropPath::Modifier { index, field }));
        }
        match &self.kind {
            LayerKind::Text(_) => out.extend([PropPath::FontSize, PropPath::TextColor]),
            LayerKind::Shape { shapes } => {
                for (index, s) in shapes.iter().enumerate() {
                    let fields: &[ShapeField] = match s.geometry {
                        Geometry::Rect { .. } => &[ShapeField::Position, ShapeField::Size, ShapeField::CornerRadius],
                        Geometry::Ellipse { .. } => &[ShapeField::Position, ShapeField::Size],
                        Geometry::Path { .. } => &[ShapeField::Path],
                        Geometry::Figure { kind: crate::primitives::FigureKind::Polygon, .. } => {
                            &[ShapeField::Position, ShapeField::Size, ShapeField::Count]
                        }
                        Geometry::Figure { kind: crate::primitives::FigureKind::Star, .. } => {
                            &[ShapeField::Position, ShapeField::Size, ShapeField::Count, ShapeField::Inner]
                        }
                        Geometry::Figure { kind: crate::primitives::FigureKind::Gear, .. } => &[
                            ShapeField::Position,
                            ShapeField::Size,
                            ShapeField::Count,
                            ShapeField::Inner,
                            ShapeField::Hole,
                        ],
                    };
                    out.extend(fields.iter().map(|&field| PropPath::Shape { index, field }));
                    // A gradient replaces the solid colour (which is then unused).
                    let tracks = |stroke| ShapeField::gradient_tracks(stroke).map(|f| PropPath::shape(index, f));
                    match &s.fill {
                        Some(f) if f.gradient.is_some() => out.extend(tracks(false)),
                        Some(_) => out.push(PropPath::shape(index, ShapeField::FillColor)),
                        None => {}
                    }
                    if let Some(st) = &s.stroke {
                        // A gradient replaces the solid colour, as for fills.
                        if st.gradient.is_some() {
                            out.extend(tracks(true));
                        } else {
                            out.push(PropPath::shape(index, ShapeField::StrokeColor));
                        }
                        out.push(PropPath::shape(index, ShapeField::StrokeWidth));
                        if !st.dashes.is_empty() {
                            out.push(PropPath::shape(index, ShapeField::DashOffset));
                        }
                    }
                    if s.trim.is_some() {
                        out.push(PropPath::shape(index, ShapeField::TrimStart));
                        out.push(PropPath::shape(index, ShapeField::TrimEnd));
                        out.push(PropPath::shape(index, ShapeField::TrimOffset));
                    }
                }
            }
            LayerKind::Solid(s) => {
                out.extend(
                    SolidField::ALL
                        .into_iter()
                        .filter(|f| *f != SolidField::Size || s.shape.has_size())
                        .filter(|f| *f != SolidField::Depth || s.shape.has_depth())
                        .map(PropPath::Solid),
                );
                for index in 0..s.parts.len() {
                    out.extend(PartField::ALL.map(|field| PropPath::SolidPart { index, field }));
                }
            }
            LayerKind::Liquid(_) => out.extend(crate::LiquidField::ALL.map(PropPath::Liquid)),
            LayerKind::Video(_) | LayerKind::Audio(_) => out.push(PropPath::Volume),
            LayerKind::Particles(e) => {
                use crate::EmitterField as F;
                out.extend(
                    F::ALL
                        .into_iter()
                        .filter(|f| match f {
                            F::Count => e.mode == crate::EmitMode::Burst,
                            F::Rate => e.mode == crate::EmitMode::Stream,
                            _ => true,
                        })
                        .map(PropPath::Emitter),
                );
            }
            LayerKind::Camera(c) => {
                out.push(PropPath::Camera(CameraField::Zoom));
                if c.shake.is_some() {
                    out.extend([CameraField::ShakeAmount, CameraField::ShakeRotation].map(PropPath::Camera));
                }
            }
            LayerKind::Null | LayerKind::Group | LayerKind::Precomp { .. } | LayerKind::Image { .. } => {}
        }
        out
    }

    pub fn property_kind(&self, path: PropPath) -> Result<ValueKind, ProjectError> {
        Ok(self.prop_ref(path)?.to_owned().kind())
    }

    /// A copy of the whole property at `path`.
    pub fn property(&self, path: PropPath) -> Result<AnyProperty, ProjectError> {
        Ok(self.prop_ref(path)?.to_owned())
    }

    /// Replaces the property at `path`, returning the previous one.
    pub fn replace_property(&mut self, path: PropPath, new: AnyProperty) -> Result<AnyProperty, ProjectError> {
        let prop = self.prop_mut(path)?;
        prop.replace(new).map_err(|expected| self.mismatch(path, expected))
    }

    /// Evaluated value of the property at `time`.
    pub fn value_at(&self, path: PropPath, time: f64) -> Result<Value, ProjectError> {
        Ok(self.prop_ref(path)?.value_at(time))
    }

    /// See [`Property::set_keyframe`].
    pub fn set_keyframe(
        &mut self,
        path: PropPath,
        time: f64,
        value: Value,
        easing: Option<Easing>,
    ) -> Result<(), ProjectError> {
        let prop = self.prop_mut(path)?;
        let expected = prop.kind();
        let ok = dispatch!(prop, |p, V| match value {
            V(v) => {
                p.set_keyframe(time, v, easing);
                true
            }
            _ => false,
        });
        if ok { Ok(()) } else { Err(self.mismatch(path, expected)) }
    }

    /// See [`Property::set_value`].
    pub fn set_value(&mut self, path: PropPath, time: f64, value: Value) -> Result<(), ProjectError> {
        let prop = self.prop_mut(path)?;
        let expected = prop.kind();
        let ok = dispatch!(prop, |p, V| match value {
            V(v) => {
                p.set_value(time, v);
                true
            }
            _ => false,
        });
        if ok { Ok(()) } else { Err(self.mismatch(path, expected)) }
    }

    /// See [`Property::remove_keyframe`]. Returns whether a keyframe was removed.
    pub fn remove_keyframe(&mut self, path: PropPath, time: f64) -> Result<bool, ProjectError> {
        Ok(dispatch!(self.prop_mut(path)?, |p, _V| p.remove_keyframe(time).is_some()))
    }

    /// Keyframe times of the property at `path` (empty when static).
    pub fn keyframe_times(&self, path: PropPath) -> Result<Vec<f64>, ProjectError> {
        Ok(self.prop_ref(path)?.keyframe_times())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Id, ShapeItem};

    fn layer() -> Layer {
        Layer::shape(Id(1), "L", vec![ShapeItem::rect(Vec2::new(10.0, 20.0), Color::WHITE)])
    }

    #[test]
    fn path_strings_round_trip() {
        let l = layer();
        for p in l.property_paths() {
            let s = p.to_string();
            assert_eq!(s.parse::<PropPath>().unwrap(), p, "{s}");
            assert!(l.property(p).is_ok(), "{s}");
        }
        assert_eq!("shapes.0.fill_color".parse::<PropPath>().unwrap(), PropPath::shape(0, ShapeField::FillColor));
        assert!("shapes.x.size".parse::<PropPath>().is_err());
        assert!("nope".parse::<PropPath>().is_err());
    }

    #[test]
    fn generic_editing() {
        let mut l = layer();
        let size = PropPath::shape(0, ShapeField::Size);
        l.set_keyframe(size, 0.0, Value::Vec2(Vec2::new(0.0, 0.0)), None).unwrap();
        l.set_keyframe(size, 1.0, Value::Vec2(Vec2::new(10.0, 20.0)), None).unwrap();
        assert_eq!(l.value_at(size, 0.5).unwrap(), Value::Vec2(Vec2::new(5.0, 10.0)));
        assert_eq!(l.keyframe_times(size).unwrap(), vec![0.0, 1.0]);

        let err = l.set_value(PropPath::Opacity, 0.0, Value::Vec2(Vec2::ZERO)).unwrap_err();
        assert!(matches!(err, ProjectError::TypeMismatch { expected: ValueKind::Scalar, .. }));
        assert!(l.property(PropPath::shape(0, ShapeField::StrokeWidth)).is_err());
        assert!(l.property(PropPath::FontSize).is_err());
        assert!(l.property(PropPath::shape(5, ShapeField::Size)).is_err());
    }

    #[test]
    fn replace_checks_type() {
        let mut l = layer();
        let old = l.replace_property(PropPath::Rotation, AnyProperty::Scalar(45.0.into())).unwrap();
        assert_eq!(old, AnyProperty::Scalar(0.0.into()));
        assert!(l.replace_property(PropPath::Rotation, AnyProperty::Color(Color::BLACK.into())).is_err());
    }

    #[test]
    fn value_json_is_natural() {
        assert_eq!(serde_json::from_str::<Value>("0.5").unwrap(), Value::Scalar(0.5));
        assert_eq!(serde_json::from_str::<Value>(r#"{"x":1,"y":2}"#).unwrap(), Value::Vec2(Vec2::new(1.0, 2.0)));
        assert_eq!(
            serde_json::from_str::<Value>(r#"{"r":1,"g":0,"b":0}"#).unwrap(),
            Value::Color(Color::rgb(1.0, 0.0, 0.0))
        );
    }
}
