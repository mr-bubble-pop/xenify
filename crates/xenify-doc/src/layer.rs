use crate::{Animatable, Color, Id, Keyframe, Property};
use kurbo::{Affine, BezPath, Vec2};
use serde::{Deserialize, Serialize};

/// A layer's 2D transform, After Effects style:
/// `translate(position) · rotate(rotation) · scale(scale) · translate(-anchor)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Point in layer space that `position` refers to and that the layer
    /// rotates/scales around.
    pub anchor: Property<Vec2>,
    /// Anchor position in parent space (composition pixels for unparented layers).
    pub position: Property<Vec2>,
    /// Scale factor per axis; `1.0` is 100%.
    pub scale: Property<Vec2>,
    /// Clockwise rotation in degrees (y points down).
    pub rotation: Property<f64>,
    /// Slant in degrees (like CSS `skewX`): positive leans the top left.
    #[serde(default = "zero_prop", skip_serializing_if = "is_static_zero")]
    pub skew: Property<f64>,
    /// The direction the slant runs along, in degrees (0 = horizontal).
    #[serde(default = "zero_prop", skip_serializing_if = "is_static_zero")]
    pub skew_axis: Property<f64>,
    /// Live random drift added on top of the animation (After Effects' `wiggle()`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiggle: Option<Wiggle>,
    /// Corner pin / perspective applied to the layer's content (and, for
    /// groups, everything inside), in layer space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distort: Option<crate::Distort>,
}

/// Smooth, deterministic random motion added to a layer's transform.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wiggle {
    /// Wiggles per second.
    pub frequency: f64,
    /// Maximum drift in pixels.
    #[serde(default)]
    pub position: f64,
    /// Maximum rotation in degrees.
    #[serde(default)]
    pub rotation: f64,
    /// Maximum scale change as a factor (0.1 = ±10%).
    #[serde(default)]
    pub scale: f64,
    /// Changes the pattern; layers with different seeds move differently.
    #[serde(default)]
    pub seed: u64,
}

impl Default for Wiggle {
    fn default() -> Self {
        Self { frequency: 2.0, position: 20.0, rotation: 0.0, scale: 0.0, seed: 0 }
    }
}

impl Wiggle {
    /// Smooth noise in -1..1 for channel `channel` at `time` (two octaves).
    pub fn noise(&self, channel: u64, time: f64) -> f64 {
        let x = time * self.frequency.max(0.0);
        let octave = |x: f64, salt: u64| {
            let i = x.floor();
            let f = x - i;
            let h = |k: f64| hash(self.seed, channel * 2 + salt, k as i64);
            let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
            h(i) + (h(i + 1.0) - h(i)) * s
        };
        (octave(x, 0) + 0.5 * octave(x * 2.0 + 17.3, 1)) / 1.5
    }

    /// `(position offset, rotation offset, scale factor)` at `time`.
    pub fn at(&self, time: f64) -> (Vec2, f64, f64) {
        (
            Vec2::new(self.noise(0, time), self.noise(1, time)) * self.position,
            self.noise(2, time) * self.rotation,
            1.0 + self.noise(3, time) * self.scale,
        )
    }
}

/// Deterministic value in -1..1 for integer lattice point `i`.
fn hash(seed: u64, channel: u64, i: i64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ channel.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

impl Default for Transform {
    fn default() -> Self {
        Self::at(Vec2::ZERO)
    }
}

impl Transform {
    pub fn at(position: Vec2) -> Self {
        Self {
            anchor: Vec2::ZERO.into(),
            position: position.into(),
            scale: Vec2::new(1.0, 1.0).into(),
            rotation: 0.0.into(),
            skew: 0.0.into(),
            skew_axis: 0.0.into(),
            wiggle: None,
            distort: None,
        }
    }

    /// The skew matrix at `time` (identity when there's no skew).
    pub fn skew_at(&self, time: f64) -> Affine {
        let skew = self.skew.value_at(time).clamp(-85.0, 85.0);
        if skew == 0.0 {
            return Affine::IDENTITY;
        }
        let axis = self.skew_axis.value_at(time).to_radians();
        Affine::rotate(axis) * Affine::skew(skew.to_radians().tan(), 0.0) * Affine::rotate(-axis)
    }

    /// Position at `time`, including any wiggle.
    pub fn position_at(&self, time: f64) -> Vec2 {
        self.position.value_at(time) + self.wiggle.map_or(Vec2::ZERO, |w| w.at(time).0)
    }

    /// Rotation at `time`, including any wiggle.
    pub fn rotation_at(&self, time: f64) -> f64 {
        self.rotation.value_at(time) + self.wiggle.map_or(0.0, |w| w.at(time).1)
    }

    /// Scale at `time`, including any wiggle.
    pub fn scale_at(&self, time: f64) -> Vec2 {
        self.scale.value_at(time) * self.wiggle.map_or(1.0, |w| w.at(time).2)
    }

    /// Layer-to-parent matrix at `time`.
    pub fn affine_at(&self, time: f64) -> Affine {
        let anchor = self.anchor.value_at(time);
        let position = self.position_at(time);
        let scale = self.scale_at(time);
        let rotation = self.rotation_at(time);
        Affine::translate(position)
            * Affine::rotate(rotation.to_radians())
            * self.skew_at(time)
            * Affine::scale_non_uniform(scale.x, scale.y)
            * Affine::translate(-anchor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    /// Solid colour (also the gradient's "representative" colour, e.g. for
    /// contrast checks).
    pub color: Property<Color>,
    /// When set, the fill is this gradient instead of the solid colour.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "gradient_compat")]
    pub gradient: Option<GradientPaint>,
    /// When set, the fill is this repeating picture (over any gradient or colour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<Pattern>,
}

/// A fill that repeats an embedded image, like SVG's `<pattern>`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    /// The image asset tiled.
    pub asset: Id,
    /// Tile size relative to the image's pixels (1 = actual size).
    #[serde(default = "one")]
    pub scale: f64,
    /// Degrees, clockwise.
    #[serde(default)]
    pub rotation: f64,
    /// Where a tile's top-left corner falls (shape coordinates).
    #[serde(default)]
    pub offset: Vec2,
}

fn one() -> f64 {
    1.0
}

impl Pattern {
    pub fn new(asset: Id) -> Self {
        Self { asset, scale: 1.0, rotation: 0.0, offset: Vec2::ZERO }
    }

    /// Image pixels → shape coordinates.
    pub fn affine(&self) -> Affine {
        Affine::translate(self.offset)
            * Affine::rotate(self.rotation.to_radians())
            * Affine::scale(self.scale.max(1e-6))
    }
}

/// Reads the current form and older files, where the gradient was one
/// keyframed value (or before that a plain value).
fn gradient_compat<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<GradientPaint>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Paint(GradientPaint),
        Property(Property<Gradient>),
        Plain(Gradient),
    }
    Ok(Option::<Either>::deserialize(d)?.map(|e| match e {
        Either::Paint(p) => p,
        Either::Property(p) => p.into(),
        Either::Plain(g) => g.into(),
    }))
}

impl Fill {
    pub fn solid(color: Color) -> Self {
        Self { color: color.into(), gradient: None, pattern: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    #[default]
    Linear,
    /// Centred on `start`, reaching `end`.
    Radial,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// 0..1 along the gradient.
    pub offset: f64,
    pub color: Color,
}

/// A gradient in layer space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub start: Vec2,
    pub end: Vec2,
    pub stops: Vec<GradientStop>,
}

impl Gradient {
    /// A gradient spanning `bounds`: linear along `angle_deg` (0 = left→right,
    /// 90 = top→bottom), or radial from the centre to the farthest side.
    pub fn across(bounds: kurbo::Rect, kind: GradientKind, angle_deg: f64, stops: Vec<GradientStop>) -> Self {
        let c = bounds.center().to_vec2();
        match kind {
            GradientKind::Linear => {
                let a = angle_deg.to_radians();
                let dir = Vec2::new(a.cos(), a.sin());
                let half = (bounds.width() / 2.0 * a.cos()).abs() + (bounds.height() / 2.0 * a.sin()).abs();
                Self { kind, start: c - dir * half, end: c + dir * half, stops }
            }
            GradientKind::Radial => {
                let r = bounds.width().max(bounds.height()) / 2.0;
                Self { kind, start: c, end: c + Vec2::new(r, 0.0), stops }
            }
        }
    }

    /// Direction of a linear gradient in degrees (0 = left→right).
    pub fn angle_deg(&self) -> f64 {
        let d = self.end - self.start;
        d.y.atan2(d.x).to_degrees()
    }

    /// Stops sorted by offset.
    pub fn sorted_stops(&self) -> Vec<GradientStop> {
        let mut s = self.stops.clone();
        s.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        s
    }

    /// The colour at `offset` (0..1), interpolating between stops.
    pub fn color_at(&self, offset: f64) -> Color {
        let stops = self.sorted_stops();
        let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return Color::BLACK };
        if offset <= first.offset {
            return first.color;
        }
        for w in stops.windows(2) {
            if offset <= w[1].offset {
                let span = (w[1].offset - w[0].offset).max(1e-9);
                return Color::lerp(&w[0].color, &w[1].color, (offset - w[0].offset) / span);
            }
        }
        last.color
    }
}

/// Same number of stops: each stop moves and changes colour. Otherwise both
/// sets are resampled at every stop offset of either, then blended.
impl crate::Animatable for Vec<GradientStop> {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        let sort = |s: &Self| {
            let mut s = s.clone();
            s.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            s
        };
        let (sa, sb) = (sort(a), sort(b));
        if sa.len() == sb.len() {
            return sa
                .iter()
                .zip(&sb)
                .map(|(x, y)| GradientStop {
                    offset: f64::lerp(&x.offset, &y.offset, t).clamp(0.0, 1.0),
                    color: Color::lerp(&x.color, &y.color, t),
                })
                .collect();
        }
        let at = |stops: &Self, o: f64| {
            Gradient { kind: GradientKind::Linear, start: Vec2::ZERO, end: Vec2::ZERO, stops: stops.clone() }
                .color_at(o)
        };
        let mut offsets: Vec<f64> = sa.iter().chain(&sb).map(|s| s.offset).collect();
        offsets.sort_by(f64::total_cmp);
        offsets.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
        offsets
            .into_iter()
            .map(|o| GradientStop { offset: o, color: Color::lerp(&at(&sa, o), &at(&sb, o), t) })
            .collect()
    }
}

impl crate::Animatable for Gradient {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        Gradient {
            kind: if t < 0.5 { a.kind } else { b.kind },
            start: <Vec2 as Animatable>::lerp(&a.start, &b.start, t),
            end: <Vec2 as Animatable>::lerp(&a.end, &b.end, t),
            stops: Vec::<GradientStop>::lerp(&a.stops, &b.stops, t),
        }
    }
}

/// A fill's or stroke's gradient. Where it starts, where it ends and its
/// colours are separate properties, so each keyframes on its own track (like
/// After Effects' Start Point, End Point and Colors). Linear or radial is not
/// animated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientPaint {
    pub kind: GradientKind,
    /// Where the gradient starts (a radial gradient's centre), in layer space.
    pub start: Property<Vec2>,
    /// Where it ends (a radial gradient's radius reaches here).
    pub end: Property<Vec2>,
    /// The colour stops.
    pub stops: Property<Vec<GradientStop>>,
}

impl GradientPaint {
    pub fn value_at(&self, time: f64) -> Gradient {
        Gradient {
            kind: self.kind,
            start: self.start.value_at(time),
            end: self.end.value_at(time),
            stops: self.stops.value_at(time),
        }
    }

    pub fn is_animated(&self) -> bool {
        self.start.is_animated() || self.end.is_animated() || self.stops.is_animated()
    }
}

impl From<Gradient> for GradientPaint {
    fn from(g: Gradient) -> Self {
        Self { kind: g.kind, start: g.start.into(), end: g.end.into(), stops: g.stops.into() }
    }
}

/// Splits a gradient keyframed as one value: every key lands on each track.
impl From<Property<Gradient>> for GradientPaint {
    fn from(p: Property<Gradient>) -> Self {
        let keys = match p {
            Property::Static(g) => return g.into(),
            Property::Animated(keys) => keys,
        };
        fn track<T: Animatable>(keys: &[Keyframe<Gradient>], f: impl Fn(&Gradient) -> T) -> Property<T> {
            let ks = keys.iter().map(|k| Keyframe { time: k.time, value: f(&k.value), easing: k.easing }).collect();
            Property::from_keyframes(ks).expect("keyframes are never empty")
        }
        Self {
            kind: keys[0].value.kind,
            start: track(&keys, |g| g.start),
            end: track(&keys, |g| g.end),
            stops: track(&keys, |g| g.stops.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Property<Color>,
    pub width: Property<f64>,
    #[serde(default)]
    pub join: LineJoin,
    #[serde(default)]
    pub cap: LineCap,
    /// Rounds the corners of `Square` caps: 0 = square, 1 = fully round
    /// (0.8 ≈ "almost round").
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cap_roundness: f64,
    /// How the line is drawn (calligraphy, pastel, dashes…).
    #[serde(default, skip_serializing_if = "crate::PenStyle::is_solid")]
    pub style: crate::PenStyle,
    /// Dash pattern: alternating dash and gap lengths in pixels (empty: solid),
    /// like SVG's `stroke-dasharray`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashes: Vec<f64>,
    /// Shifts the dash pattern along the line (animate it for marching ants).
    #[serde(default = "zero_prop", skip_serializing_if = "is_static_zero")]
    pub dash_offset: Property<f64>,
    /// Paints the line with a gradient instead of `color`.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "gradient_compat")]
    pub gradient: Option<GradientPaint>,
    /// Arrowheads (or other marks) at the start and end of an open line.
    #[serde(default, skip_serializing_if = "crate::Marker::is_none")]
    pub start_marker: crate::Marker,
    #[serde(default, skip_serializing_if = "crate::Marker::is_none")]
    pub end_marker: crate::Marker,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            color: Color::WHITE.into(),
            width: 1.0.into(),
            join: LineJoin::default(),
            cap: LineCap::default(),
            cap_roundness: 0.0,
            style: crate::PenStyle::default(),
            dashes: Vec::new(),
            dash_offset: zero_prop(),
            gradient: None,
            start_marker: Default::default(),
            end_marker: Default::default(),
        }
    }
}

fn zero_prop() -> Property<f64> {
    0.0.into()
}

fn is_static_zero(p: &Property<f64>) -> bool {
    matches!(p, Property::Static(v) if *v == 0.0)
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

/// Which areas of a self-overlapping path count as "inside".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    #[default]
    NonZero,
    /// Alternating inside/outside: nested outlines become holes.
    EvenOdd,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Geometry of a shape, in layer space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Geometry {
    Rect {
        /// Centre of the rectangle.
        position: Property<Vec2>,
        size: Property<Vec2>,
        corner_radius: Property<f64>,
    },
    Ellipse {
        /// Centre of the ellipse.
        position: Property<Vec2>,
        size: Property<Vec2>,
    },
    Path {
        path: Property<BezPath>,
    },
    /// A regular polygon, star or gear that stays live: its sides, points
    /// or teeth, inner radius / tooth depth and hole stay editable.
    Figure {
        /// Centre of the box it fills.
        position: Property<Vec2>,
        size: Property<Vec2>,
        kind: crate::primitives::FigureKind,
        /// Sides, points or teeth (rounded).
        count: Property<f64>,
        /// Star: inner radius; gear: tooth depth (fractions of the radius).
        #[serde(default = "half")]
        inner: Property<f64>,
        /// Gear: the hole in the middle (fraction of the radius, 0 = none).
        #[serde(default = "zero_prop")]
        hole: Property<f64>,
    },
}

fn half() -> Property<f64> {
    0.5.into()
}

/// One drawable shape inside a shape layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeItem {
    pub name: String,
    pub geometry: Geometry,
    /// How overlapping parts / holes of the path are filled.
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill_rule: FillRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Trim paths: only part of the outline is drawn (the "draw on" effect).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<Trim>,
}

/// Which part of a shape's outline is drawn, as fractions of its length.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    /// 0..1
    pub start: Property<f64>,
    /// 0..1
    pub end: Property<f64>,
    /// Shifts the drawn part along the outline (0..1 = one full turn).
    pub offset: Property<f64>,
}

impl Default for Trim {
    fn default() -> Self {
        Self { start: 0.0.into(), end: 1.0.into(), offset: 0.0.into() }
    }
}

impl ShapeItem {
    pub fn rect(size: Vec2, fill: Color) -> Self {
        Self {
            name: "Rectangle".into(),
            fill_rule: FillRule::NonZero,
            geometry: Geometry::Rect { position: Vec2::ZERO.into(), size: size.into(), corner_radius: 0.0.into() },
            fill: Some(Fill::solid(fill)),
            stroke: None,
            trim: None,
        }
    }

    pub fn ellipse(size: Vec2, fill: Color) -> Self {
        Self {
            name: "Ellipse".into(),
            fill_rule: FillRule::NonZero,
            geometry: Geometry::Ellipse { position: Vec2::ZERO.into(), size: size.into() },
            fill: Some(Fill::solid(fill)),
            stroke: None,
            trim: None,
        }
    }

    pub fn path(path: BezPath, fill: Option<Color>, stroke: Option<(Color, f64)>) -> Self {
        Self {
            name: "Path".into(),
            fill_rule: FillRule::NonZero,
            geometry: Geometry::Path { path: path.into() },
            fill: fill.map(Fill::solid),
            stroke: stroke.map(|(c, w)| Stroke {
                cap_roundness: 0.0,
                style: Default::default(),
                color: c.into(),
                width: w.into(),
                join: LineJoin::default(),
                cap: LineCap::default(),
                ..Default::default()
            }),
            trim: None,
        }
    }
}

/// Which point of the text sits on the layer's origin horizontally; lines of
/// multi-line text are aligned the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Which point of the text sits on the layer's origin vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextVAlign {
    /// The first line's baseline (like After Effects point text).
    #[default]
    Baseline,
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextLayer {
    pub text: String,
    pub font_family: String,
    pub font_size: Property<f64>,
    pub color: Property<Color>,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub valign: TextVAlign,
    /// Extra space between letters, in pixels (negative tightens).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub letter_spacing: f64,
    /// Extra space between words, in pixels.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub word_spacing: f64,
    /// Distance between lines as a multiple of the font size (0 = the font's own).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub line_height: f64,
    /// Set along another layer's outline instead of in straight lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<TextPath>,
    /// A text box: lines wrap and align inside it (point text without one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<TextFrame>,
    /// Letters, words or lines animating in (see [`crate::TextAnimator`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animators: Vec<crate::TextAnimator>,
}

/// The box of box text (CSS: a sized block with `overflow`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextFrame {
    /// The box in the layer's own space.
    pub rect: kurbo::Rect,
    /// Break lines that are wider than the box.
    #[serde(default = "yes")]
    pub wrap: bool,
    /// Hide text outside the box (CSS `overflow: hidden`), else it spills out.
    #[serde(default)]
    pub clip: bool,
}

impl TextFrame {
    /// A wrapping, unclipped box.
    pub fn new(rect: kurbo::Rect) -> Self {
        Self { rect, wrap: true, clip: false }
    }
}

/// Text following the outline of a shape layer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextPath {
    /// The shape layer whose first shape's outline the text follows.
    pub layer: Id,
    /// Distance along the outline before the text starts (the margin on the
    /// curve), in pixels; with centred text, from the middle of the outline.
    #[serde(default)]
    pub offset: f64,
    /// Follow the outline the other way round (e.g. text inside a circle).
    #[serde(default)]
    pub flip: bool,
}

impl TextLayer {
    /// Left/baseline-aligned text.
    pub fn new(text: impl Into<String>, font_family: impl Into<String>, font_size: f64, color: Color) -> Self {
        Self {
            text: text.into(),
            font_family: font_family.into(),
            font_size: font_size.into(),
            color: color.into(),
            align: TextAlign::default(),
            valign: TextVAlign::default(),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            line_height: 0.0,
            path: None,
            frame: None,
            animators: Vec::new(),
        }
    }

    /// Centred horizontally and vertically on the layer's position.
    pub fn centered(self) -> Self {
        Self { align: TextAlign::Center, valign: TextVAlign::Middle, ..self }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerKind {
    Shape {
        shapes: Vec<ShapeItem>,
    },
    Text(TextLayer),
    /// Invisible layer used as a parent to group transforms.
    Null,
    /// A group: invisible; its children (layers parented to it) move, scale
    /// and animate with it, and it is selected as one object.
    Group,
    /// A nested composition (precomp), drawn as one layer and clipped to its
    /// frame. Its local time is the parent's time minus `start`; `looped`
    /// plays it over and over from then on (else it shows once).
    Precomp {
        comp: Id,
        #[serde(default)]
        start: f64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        looped: bool,
    },
    /// A shaded 3D object (primitive or extruded outline).
    Solid(Box<crate::Solid>),
    /// A raster image (an [`crate::ImageAsset`] of the project), drawn with its
    /// top-left corner at the layer origin, one unit per pixel. Animated
    /// images (GIF, WebP, APNG) play as set by `playback`.
    Image {
        asset: Id,
        #[serde(default, skip_serializing_if = "is_default")]
        playback: crate::ImagePlayback,
    },
    /// A camera: frames what the viewer sees (see [`crate::CameraLayer`]).
    /// Not drawn itself.
    Camera(crate::CameraLayer),
    /// A particle emitter (see [`crate::Emitter`]).
    Particles(Box<crate::Emitter>),
    /// Water filling a box (see [`crate::LiquidFill`]).
    Liquid(Box<crate::LiquidFill>),
    /// A video (a [`crate::MediaAsset`] of the project), drawn with its top
    /// left at the layer origin, one unit per pixel.
    Video(Box<crate::VideoLayer>),
    /// Sound on the timeline (nothing drawn).
    Audio(Box<crate::AudioLayer>),
}

/// The non-animated settings of a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerMeta {
    pub name: String,
    /// Time (seconds) at which the layer becomes visible.
    #[serde(default)]
    pub in_point: f64,
    /// Time (seconds) at which the layer stops being visible (exclusive);
    /// `None` means it stays until the end of the composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_point: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// Track matte: another layer whose shape (alpha) or brightness (luma)
    /// masks this layer. The matte layer itself is then not drawn. On a group
    /// it masks every member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matte: Option<Matte>,
    /// Expressions computing property values (see `xenify_anim::expr`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<Expression>,
    /// Live rigging used while posing on the canvas (IK joint chains).
    #[serde(default, skip_serializing_if = "Rig::is_off")]
    pub rig: Rig,
    /// Drop shadows, glows, blur and colour adjustments, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<crate::Effect>,
    /// How the layer mixes with what's under it.
    #[serde(default, skip_serializing_if = "crate::BlendMode::is_normal")]
    pub blend: crate::BlendMode,
    /// Shapes that decide which parts of the layer show (crop), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<crate::Mask>,
    /// Live changes to what the layer draws (arrays of copies), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<crate::Modifier>,
    /// Makes this (shape) layer a connector: its line is routed between two
    /// shapes whenever they move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<Box<crate::Connector>>,
    /// Where connectors glue to this layer (empty: the middle of each side).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<crate::Port>,
    /// Text shown inside the shape (or along a connector).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<crate::NodeLabel>,
    /// A diagram container (a pool or lane): shapes dropped inside it become
    /// its members (parented to it) and move with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub container: bool,
    /// What it is on a templated slide (title, body, accent…), so Apply
    /// Theme can restyle it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<crate::Role>,
}

/// How a layer behaves when layers are dragged on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rig {
    /// IK: dragging this layer bends up to this many parent joints (rotating
    /// them about their anchors) so it reaches the pointer. 0 = off.
    pub ik_chain: u32,
}

impl Rig {
    pub fn is_off(&self) -> bool {
        self.ik_chain == 0
    }
}

/// An expression driving one property, e.g. `value + wiggle(2, 20)` on position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expression {
    pub path: crate::PropPath,
    pub source: String,
}

/// A track matte reference, see [`LayerMeta::matte`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Matte {
    pub layer: Id,
    #[serde(default)]
    pub mode: MatteMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatteMode {
    /// Visible only inside the matte's shape.
    #[default]
    Alpha,
    /// Visible only outside the matte's shape.
    AlphaInverted,
    /// Visible where the matte is bright.
    Luma,
    /// Visible where the matte is dark (or absent).
    LumaInverted,
}

impl MatteMode {
    pub const ALL: [MatteMode; 4] =
        [MatteMode::Alpha, MatteMode::AlphaInverted, MatteMode::Luma, MatteMode::LumaInverted];

    pub fn label(self) -> &'static str {
        match self {
            MatteMode::Alpha => "Alpha",
            MatteMode::AlphaInverted => "Alpha inverted",
            MatteMode::Luma => "Luminance",
            MatteMode::LumaInverted => "Luminance inverted",
        }
    }

    pub fn inverted(self) -> bool {
        matches!(self, MatteMode::AlphaInverted | MatteMode::LumaInverted)
    }
}

fn yes() -> bool {
    true
}

impl LayerMeta {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            in_point: 0.0,
            out_point: None,
            parent: None,
            visible: true,
            locked: false,
            matte: None,
            expressions: Vec::new(),
            rig: Rig::default(),
            effects: Vec::new(),
            blend: Default::default(),
            masks: Vec::new(),
            modifiers: Vec::new(),
            connector: None,
            ports: Vec::new(),
            label: None,
            container: false,
            role: None,
        }
    }

    /// The layer's ports, or the default four.
    pub fn ports(&self) -> &[crate::Port] {
        if self.ports.is_empty() { &crate::DEFAULT_PORTS } else { &self.ports }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: Id,
    #[serde(flatten)]
    pub meta: LayerMeta,
    pub transform: Transform,
    /// `0.0..=1.0`, applied to the layer as a group.
    pub opacity: Property<f64>,
    #[serde(flatten)]
    pub kind: LayerKind,
}

impl Layer {
    /// The embedded images this layer shows (as an image layer, or as
    /// pattern fills).
    pub fn asset_refs(&self) -> Vec<Id> {
        let mut out = Vec::new();
        match &self.kind {
            LayerKind::Image { asset, .. } => out.push(*asset),
            LayerKind::Video(v) => out.push(v.asset),
            LayerKind::Audio(a) => out.push(a.asset),
            _ => {}
        }
        for s in self.shapes() {
            if let Some(p) = s.fill.as_ref().and_then(|f| f.pattern) {
                out.push(p.asset);
            }
        }
        out
    }

    /// Points every image reference `from` at `to` instead.
    pub fn remap_asset(&mut self, from: Id, to: Id) {
        if let LayerKind::Image { asset, .. } = &mut self.kind
            && *asset == from
        {
            *asset = to;
        }
        if let Some(shapes) = self.shapes_mut() {
            for s in shapes {
                if let Some(p) = s.fill.as_mut().and_then(|f| f.pattern.as_mut())
                    && p.asset == from
                {
                    p.asset = to;
                }
            }
        }
    }

    pub fn new(id: Id, name: impl Into<String>, kind: LayerKind) -> Self {
        Self { id, meta: LayerMeta::named(name), transform: Transform::default(), opacity: 1.0.into(), kind }
    }

    pub fn shape(id: Id, name: impl Into<String>, shapes: Vec<ShapeItem>) -> Self {
        Self::new(id, name, LayerKind::Shape { shapes })
    }

    /// A layer showing composition `comp`, anchored at its centre and placed at `position`.
    pub fn precomp(id: Id, name: impl Into<String>, comp: &crate::Composition, position: Vec2) -> Self {
        let mut l = Self::new(id, name, LayerKind::Precomp { comp: comp.id, start: 0.0, looped: false })
            .with_position(position);
        l.transform.anchor = Vec2::new(comp.settings.width as f64 / 2.0, comp.settings.height as f64 / 2.0).into();
        l
    }

    /// An image layer showing `asset` at its natural size, centred on `position`.
    pub fn image(id: Id, name: impl Into<String>, asset: &crate::ImageAsset, position: Vec2) -> Self {
        let mut l = Self::new(id, name, LayerKind::Image { asset: asset.id, playback: Default::default() })
            .with_position(position);
        l.transform.anchor = Vec2::new(asset.width as f64 / 2.0, asset.height as f64 / 2.0).into();
        l
    }

    pub fn with_position(mut self, position: Vec2) -> Self {
        self.transform.position = position.into();
        self
    }

    /// Whether the layer is shown at `time` (visibility flag and in/out points).
    pub fn is_active_at(&self, time: f64) -> bool {
        self.meta.visible && time >= self.meta.in_point && self.meta.out_point.is_none_or(|out| time < out)
    }

    pub fn shapes(&self) -> &[ShapeItem] {
        match &self.kind {
            LayerKind::Shape { shapes } => shapes,
            _ => &[],
        }
    }

    pub fn shapes_mut(&mut self) -> Option<&mut Vec<ShapeItem>> {
        match &mut self.kind {
            LayerKind::Shape { shapes } => Some(shapes),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn gradients_interpolate_and_old_files_still_load() {
        let stop = |offset, color| GradientStop { offset, color };
        let a = Gradient {
            kind: GradientKind::Linear,
            start: Vec2::ZERO,
            end: Vec2::new(100.0, 0.0),
            stops: vec![stop(0.0, Color::BLACK), stop(1.0, Color::WHITE)],
        };
        let b = Gradient {
            end: Vec2::new(0.0, 100.0),
            stops: vec![stop(0.0, Color::WHITE), stop(1.0, Color::BLACK)],
            ..a.clone()
        };
        let mid = Gradient::lerp(&a, &b, 0.5);
        assert_eq!(mid.end, Vec2::new(50.0, 50.0));
        assert!((mid.stops[0].color.r - 0.5).abs() < 1e-9);
        // Different stop counts resample both.
        let three = Gradient {
            stops: vec![stop(0.0, Color::BLACK), stop(0.5, Color::WHITE), stop(1.0, Color::BLACK)],
            ..a.clone()
        };
        assert_eq!(Gradient::lerp(&a, &three, 1.0).color_at(0.5), Color::WHITE);
        assert_eq!(Gradient::lerp(&a, &three, 0.0).stops.len(), 3);
        // A plain gradient (the pre-animation file format) loads as a static property.
        let old = serde_json::json!({"color": {"static": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}}, "gradient": a});
        let fill: Fill = serde_json::from_value(old).unwrap();
        assert_eq!(fill.gradient, Some(a.clone().into()));
        let round: Fill = serde_json::from_str(&serde_json::to_string(&fill).unwrap()).unwrap();
        assert_eq!(round, fill);
        // A gradient keyframed as one value splits into start, end and colour tracks.
        let keys = Property::from_keyframes(vec![
            Keyframe { time: 0.0, value: a.clone(), easing: crate::Easing::Linear },
            Keyframe { time: 2.0, value: b.clone(), easing: crate::Easing::Linear },
        ])
        .unwrap();
        let old = serde_json::json!({"color": {"static": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}}, "gradient": keys});
        let fill: Fill = serde_json::from_value(old).unwrap();
        let paint = fill.gradient.unwrap();
        assert_eq!(paint.start.keyframes().len(), 2);
        assert_eq!(paint.end.keyframes().len(), 2);
        assert_eq!(paint.stops.keyframes().len(), 2);
        assert_eq!(paint.value_at(1.0), Gradient::lerp(&a, &b, 0.5));
    }

    #[test]
    fn wiggle_is_smooth_bounded_and_deterministic() {
        let w = Wiggle { frequency: 3.0, position: 50.0, rotation: 10.0, scale: 0.2, seed: 7 };
        let mut prev = w.at(0.0);
        let mut moved = 0.0;
        for i in 1..=300 {
            let now = w.at(i as f64 / 100.0);
            assert!(now.0.x.abs() <= 50.0 && now.1.abs() <= 10.0 && (now.2 - 1.0).abs() <= 0.2);
            assert!((now.0 - prev.0).hypot() < 5.0, "no jumps");
            moved += (now.0 - prev.0).hypot();
            prev = now;
        }
        assert!(moved > 50.0, "it actually moves");
        assert_eq!(w.at(1.234), w.at(1.234));
        assert_ne!(w.at(1.0).0, Wiggle { seed: 8, ..w }.at(1.0).0);
        let mut t = Transform::at(Vec2::new(100.0, 100.0));
        t.wiggle = Some(w);
        assert_eq!(t.affine_at(0.5) * kurbo::Point::ORIGIN, (Vec2::new(100.0, 100.0) + w.at(0.5).0).to_point());
    }

    #[test]
    fn transform_order_matches_after_effects() {
        let mut t = Transform::at(Vec2::new(100.0, 100.0));
        t.anchor = Vec2::new(10.0, 0.0).into();
        t.scale = Vec2::new(2.0, 2.0).into();
        t.rotation = 90.0.into();
        let a = t.affine_at(0.0);
        // The anchor lands exactly on `position`.
        let p = a * Point::new(10.0, 0.0);
        assert!((p - Point::new(100.0, 100.0)).hypot() < 1e-9);
        // A point 1px right of the anchor: scaled to 2px, rotated 90° clockwise → down.
        let q = a * Point::new(11.0, 0.0);
        assert!((q - Point::new(100.0, 102.0)).hypot() < 1e-9);
    }

    #[test]
    fn layer_json_is_flat() {
        let layer = Layer::shape(Id(3), "Box", vec![ShapeItem::rect(Vec2::new(10.0, 10.0), Color::WHITE)]);
        let v = serde_json::to_value(&layer).unwrap();
        assert_eq!(v["id"], 3);
        assert_eq!(v["name"], "Box");
        assert_eq!(v["type"], "shape");
        assert_eq!(v["shapes"][0]["geometry"]["type"], "rect");
        let back: Layer = serde_json::from_value(v).unwrap();
        assert_eq!(back, layer);
    }
}
