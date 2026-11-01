//! Modifiers: live changes to what a layer draws, applied in order (like
//! Blender's modifier stack). They never touch the layer's own content, so
//! they can be edited, keyframed, switched off or applied later.

use crate::Property;
use kurbo::{Affine, Point, Vec2};
use serde::{Deserialize, Serialize};

/// More copies than this (across the whole stack) are left out.
pub const MAX_COPIES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modifier {
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: ModifierKind,
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl Modifier {
    pub fn new(kind: ModifierKind) -> Self {
        Self { enabled: true, kind }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "modifier", rename_all = "snake_case")]
pub enum ModifierKind {
    /// Copies of the layer, each one step on from the one before: turned by
    /// `rotation` and scaled by `scale` around `origin`, and moved by
    /// `offset` (Lottie's Repeater, Blender's Array). All in layer space.
    /// As in Lottie, the offset doesn't turn with the copies: for arcs and
    /// rings, turn around an origin away from the layer.
    Array {
        /// How many copies, the layer itself included (rounded).
        count: Property<f64>,
        offset: Property<Vec2>,
        /// Degrees per step.
        rotation: Property<f64>,
        /// Per step; 1 = same size.
        scale: Property<Vec2>,
        /// The point copies turn and scale around, in layer space.
        origin: Property<Vec2>,
        /// Opacity of the last copy (the first is 1); the ones between fade evenly.
        end_opacity: Property<f64>,
    },
    /// Moves points by random amounts (Inkscape's Jitter nodes, Illustrator's
    /// Roughen). Animating the seed makes the outline boil.
    Jitter {
        /// How far points move at most.
        amount: Property<f64>,
        /// Points added per 100 units of outline first (0: only the existing ones).
        detail: Property<f64>,
        seed: Property<f64>,
        /// Keep curves (else straight lines between the points).
        #[serde(default)]
        smooth: bool,
    },
    /// Ridges along every segment (Lottie's and Illustrator's Zig Zag); a
    /// wave when smooth.
    ZigZag {
        /// How far the ridges stand out.
        amount: Property<f64>,
        /// Ridges per segment.
        ridges: Property<f64>,
        #[serde(default)]
        smooth: bool,
    },
    /// Points pulled in and curves pushed out (bloat, > 0) or the other way
    /// round (pucker, < 0), as Lottie's Pucker & Bloat. −1…1.
    PuckerBloat { strength: Property<f64> },
    /// Turns the middle more than the edges, around `origin` (degrees).
    Twist { rotation: Property<f64>, origin: Property<Vec2> },
    /// Makes corners round: 0 keeps them, 1 smooths them fully.
    Smooth { strength: Property<f64> },
    /// Splits every segment in two, `levels` times (Blender's Subdivide).
    Subdivide { levels: Property<f64> },
    /// Fewer points, within `tolerance` units of the original.
    Simplify { tolerance: Property<f64> },
    /// Grows (> 0) or shrinks (< 0) filled outlines by `amount` (Lottie's
    /// and Illustrator's Offset Path).
    OffsetPath {
        amount: Property<f64>,
        #[serde(default)]
        join: crate::LineJoin,
    },
    /// A mirrored copy across the line through `origin` at `rotation`
    /// degrees (90 = upright: left ↔ right), as Blender's Mirror. With
    /// `merge` the two halves become one outline instead.
    Mirror {
        origin: Property<Vec2>,
        rotation: Property<f64>,
        #[serde(default)]
        merge: bool,
    },
    /// `count` steps in between this layer and `target`, morphing shape,
    /// colours, size and place (Illustrator's Blend). Shape layers only.
    Blend { target: crate::Id, count: Property<f64> },
    /// `count` copies along `path`'s outline (another shape layer), from
    /// `start` to `end` of the way along (0…1), each copy's transform origin
    /// on the line; `align` turns them with it. The copies replace the layer
    /// itself on the line: move the path to move them.
    PathArray {
        path: crate::Id,
        count: Property<f64>,
        start: Property<f64>,
        end: Property<f64>,
        #[serde(default = "yes")]
        align: bool,
    },
    /// Breaks the layer into `count` shards that fly apart from `origin`
    /// (layer space): pushed out by `force` (pixels per second) and
    /// `amount` more towards `rotation` degrees, pulled down by `gravity`
    /// and turning by up to `spin` degrees per second. `progress` is the
    /// seconds of flight: keyframe it 0 → 2 to explode, 2 → 0 to assemble.
    Shatter {
        count: Property<f64>,
        origin: Property<Vec2>,
        /// Outward speed.
        force: Property<f64>,
        /// Extra push, and its direction.
        amount: Property<f64>,
        rotation: Property<f64>,
        gravity: Property<f64>,
        spin: Property<f64>,
        progress: Property<f64>,
        seed: Property<f64>,
    },
}

/// One setting of a modifier (for keyframing it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModifierField {
    Count,
    Offset,
    Rotation,
    Scale,
    Origin,
    EndOpacity,
    /// A distance.
    Amount,
    /// A fraction (shown as a percentage).
    Strength,
    Detail,
    Seed,
    Ridges,
    Levels,
    Tolerance,
    /// Where along a path (0…1).
    Start,
    End,
    Gravity,
    Spin,
    Progress,
    Force,
}

impl ModifierField {
    pub const ALL: [ModifierField; 19] = [
        ModifierField::Count,
        ModifierField::Offset,
        ModifierField::Rotation,
        ModifierField::Scale,
        ModifierField::Origin,
        ModifierField::EndOpacity,
        ModifierField::Amount,
        ModifierField::Strength,
        ModifierField::Detail,
        ModifierField::Seed,
        ModifierField::Ridges,
        ModifierField::Levels,
        ModifierField::Tolerance,
        ModifierField::Start,
        ModifierField::End,
        ModifierField::Gravity,
        ModifierField::Spin,
        ModifierField::Progress,
        ModifierField::Force,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ModifierField::Count => "count",
            ModifierField::Offset => "offset",
            ModifierField::Rotation => "rotation",
            ModifierField::Scale => "scale",
            ModifierField::Origin => "origin",
            ModifierField::EndOpacity => "end_opacity",
            ModifierField::Amount => "amount",
            ModifierField::Strength => "strength",
            ModifierField::Detail => "detail",
            ModifierField::Seed => "seed",
            ModifierField::Ridges => "ridges",
            ModifierField::Levels => "levels",
            ModifierField::Tolerance => "tolerance",
            ModifierField::Start => "start",
            ModifierField::End => "end",
            ModifierField::Gravity => "gravity",
            ModifierField::Spin => "spin",
            ModifierField::Progress => "progress",
            ModifierField::Force => "force",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModifierField::Count => "Count",
            ModifierField::Offset => "Offset",
            ModifierField::Rotation => "Rotation",
            ModifierField::Scale => "Scale",
            ModifierField::Origin => "Origin",
            ModifierField::EndOpacity => "End opacity",
            ModifierField::Amount => "Amount",
            ModifierField::Strength => "Strength",
            ModifierField::Detail => "Detail",
            ModifierField::Seed => "Seed",
            ModifierField::Ridges => "Ridges",
            ModifierField::Levels => "Levels",
            ModifierField::Tolerance => "Tolerance",
            ModifierField::Start => "Start",
            ModifierField::End => "End",
            ModifierField::Gravity => "Gravity",
            ModifierField::Spin => "Spin",
            ModifierField::Progress => "Progress",
            ModifierField::Force => "Force",
        }
    }
}

/// An array step at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrayStep {
    pub count: usize,
    pub offset: Vec2,
    pub rotation: f64,
    pub scale: Vec2,
    pub origin: Vec2,
    pub end_opacity: f64,
}

impl ArrayStep {
    /// Copy `i`'s transform in layer space (as lottie-web's repeater: turned,
    /// then scaled, around the origin, then moved by `i` offsets).
    pub fn affine(&self, i: usize) -> Affine {
        let n = i as f64;
        Affine::translate(self.offset * n + self.origin)
            * Affine::scale_non_uniform(self.scale.x.powf(n), self.scale.y.powf(n))
            * Affine::rotate((self.rotation * n).to_radians())
            * Affine::translate(-self.origin)
    }

    /// Each copy's transform (layer space) and opacity, the layer itself first.
    pub fn copies(&self) -> Vec<(Affine, f64)> {
        (0..self.count)
            .map(|i| {
                let t = if self.count > 1 { i as f64 / (self.count - 1) as f64 } else { 0.0 };
                (self.affine(i), 1.0 + (self.end_opacity - 1.0) * t)
            })
            .collect()
    }
}

/// A shape-changing modifier at one moment (see `xenify_anim::deform` for
/// what each does to a path).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deform {
    Jitter {
        amount: f64,
        detail: f64,
        seed: f64,
        smooth: bool,
    },
    ZigZag {
        amount: f64,
        ridges: usize,
        smooth: bool,
    },
    PuckerBloat {
        strength: f64,
    },
    Twist {
        degrees: f64,
        centre: Point,
    },
    Smooth {
        strength: f64,
    },
    Subdivide {
        levels: usize,
    },
    Simplify {
        tolerance: f64,
    },
    OffsetPath {
        amount: f64,
        join: crate::LineJoin,
    },
    /// The outline joined with its mirror image across `axis` (a reflection).
    MirrorMerge {
        axis: Affine,
    },
}

/// A modifier at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Evaluated {
    Array(ArrayStep),
    /// The layer and one mirrored copy (the reflection, layer space).
    Mirror(Affine),
    Deform(Deform),
    /// Depends on other layers (placed by `xenify_anim`), or adds drawings
    /// of its own (a blend): no copies here.
    External,
}

/// The reflection across the line through `origin` at `degrees`.
pub fn reflection(origin: Vec2, degrees: f64) -> Affine {
    Affine::translate(origin)
        * Affine::rotate((2.0 * degrees).to_radians())
        * Affine::scale_non_uniform(1.0, -1.0)
        * Affine::translate(-origin)
}

impl ModifierKind {
    pub fn label(&self) -> &'static str {
        match self {
            ModifierKind::Array { .. } => "Array",
            ModifierKind::Jitter { .. } => "Jitter",
            ModifierKind::ZigZag { smooth: false, .. } => "Zig Zag",
            ModifierKind::ZigZag { smooth: true, .. } => "Wave",
            ModifierKind::PuckerBloat { .. } => "Pucker & Bloat",
            ModifierKind::Twist { .. } => "Twist",
            ModifierKind::Smooth { .. } => "Smooth",
            ModifierKind::Subdivide { .. } => "Subdivide",
            ModifierKind::Simplify { .. } => "Simplify",
            ModifierKind::OffsetPath { .. } => "Offset Path",
            ModifierKind::Mirror { .. } => "Mirror",
            ModifierKind::Blend { .. } => "Blend",
            ModifierKind::PathArray { .. } => "Array Along Path",
            ModifierKind::Shatter { .. } => "Shatter",
        }
    }

    /// Another layer it follows or blends into.
    pub fn target(&self) -> Option<crate::Id> {
        match self {
            ModifierKind::Blend { target, .. } => Some(*target),
            ModifierKind::PathArray { path, .. } => Some(*path),
            _ => None,
        }
    }

    /// Whether it makes copies (rather than changing the outline).
    pub fn is_array(&self) -> bool {
        matches!(
            self,
            ModifierKind::Array { .. }
                | ModifierKind::PathArray { .. }
                | ModifierKind::Mirror { merge: false, .. }
                | ModifierKind::Shatter { .. }
        )
    }

    /// The settings this kind of modifier has, in display order.
    pub fn fields(&self) -> &'static [ModifierField] {
        use ModifierField::*;
        match self {
            ModifierKind::Array { .. } => &[Count, Offset, Rotation, Scale, Origin, EndOpacity],
            ModifierKind::Jitter { .. } => &[Amount, Detail, Seed],
            ModifierKind::ZigZag { .. } => &[Amount, Ridges],
            ModifierKind::PuckerBloat { .. } | ModifierKind::Smooth { .. } => &[Strength],
            ModifierKind::Twist { .. } => &[Rotation, Origin],
            ModifierKind::Subdivide { .. } => &[Levels],
            ModifierKind::Simplify { .. } => &[Tolerance],
            ModifierKind::OffsetPath { .. } => &[Amount],
            ModifierKind::Mirror { .. } => &[Origin, Rotation],
            ModifierKind::Blend { .. } => &[Count],
            ModifierKind::PathArray { .. } => &[Count, Start, End],
            ModifierKind::Shatter { .. } => &[Progress, Count, Origin, Force, Amount, Rotation, Gravity, Spin, Seed],
        }
    }

    /// The settings at `time`.
    pub fn at(&self, time: f64) -> Evaluated {
        let v = |p: &Property<f64>| p.value_at(time);
        let count = |p: &Property<f64>, max: f64| v(p).round().clamp(0.0, max) as usize;
        Evaluated::Deform(match self {
            ModifierKind::Array { count: n, offset, rotation, scale, origin, end_opacity } => {
                return Evaluated::Array(ArrayStep {
                    count: count(n, MAX_COPIES as f64).max(1),
                    offset: offset.value_at(time),
                    rotation: v(rotation),
                    scale: scale.value_at(time),
                    origin: origin.value_at(time),
                    end_opacity: v(end_opacity).clamp(0.0, 1.0),
                });
            }
            ModifierKind::Jitter { amount, detail, seed, smooth } => Deform::Jitter {
                amount: v(amount),
                detail: v(detail).clamp(0.0, 100.0),
                seed: v(seed),
                smooth: *smooth,
            },
            ModifierKind::ZigZag { amount, ridges, smooth } => {
                Deform::ZigZag { amount: v(amount), ridges: count(ridges, 100.0), smooth: *smooth }
            }
            ModifierKind::PuckerBloat { strength } => Deform::PuckerBloat { strength: v(strength).clamp(-1.0, 1.0) },
            ModifierKind::Twist { rotation, origin } => {
                Deform::Twist { degrees: v(rotation), centre: origin.value_at(time).to_point() }
            }
            ModifierKind::Smooth { strength } => Deform::Smooth { strength: v(strength).clamp(0.0, 1.0) },
            ModifierKind::Subdivide { levels } => Deform::Subdivide { levels: count(levels, 6.0) },
            ModifierKind::Simplify { tolerance } => Deform::Simplify { tolerance: v(tolerance).max(0.0) },
            ModifierKind::OffsetPath { amount, join } => Deform::OffsetPath { amount: v(amount), join: *join },
            ModifierKind::Mirror { origin, rotation, merge } => {
                let axis = reflection(origin.value_at(time), v(rotation));
                if !*merge {
                    return Evaluated::Mirror(axis);
                }
                Deform::MirrorMerge { axis }
            }
            ModifierKind::Blend { .. } | ModifierKind::PathArray { .. } | ModifierKind::Shatter { .. } => {
                return Evaluated::External;
            }
        })
    }

    /// A straight row of `count` copies, `offset` apart.
    pub fn array(count: u32, offset: Vec2, origin: Vec2) -> Self {
        ModifierKind::Array {
            count: (count as f64).into(),
            offset: offset.into(),
            rotation: 0.0.into(),
            scale: Vec2::new(1.0, 1.0).into(),
            origin: origin.into(),
            end_opacity: 1.0.into(),
        }
    }

    /// `count` copies spread evenly around `centre` (layer space).
    pub fn radial_array(count: u32, centre: Vec2) -> Self {
        ModifierKind::Array {
            count: (count as f64).into(),
            offset: Vec2::ZERO.into(),
            rotation: (360.0 / count.max(1) as f64).into(),
            scale: Vec2::new(1.0, 1.0).into(),
            origin: centre.into(),
            end_opacity: 1.0.into(),
        }
    }

    /// The point the modifier turns things around (arrays and twists), to
    /// show as a handle.
    pub fn origin(&self) -> Option<&Property<Vec2>> {
        match self {
            ModifierKind::Array { origin, .. }
            | ModifierKind::Twist { origin, .. }
            | ModifierKind::Mirror { origin, .. }
            | ModifierKind::Shatter { origin, .. } => Some(origin),
            _ => None,
        }
    }
}

/// One copy a modifier stack makes: where it goes (layer space), its
/// opacity, and the shape changes to make to it first, in order. Each change
/// works in the space the modifier saw: a path `p` in the copy's own space
/// becomes `a⁻¹ · deform(a · p)`.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyPlan {
    pub transform: Affine,
    pub opacity: f64,
    pub deforms: Vec<(Affine, Deform)>,
    /// Only this part of the layer (its own space, before `transform`)
    /// shows in this copy: a shard.
    pub clip: Option<kurbo::BezPath>,
}

/// What a modifier stack does at `time`: every copy, in paint order
/// (bottom-most first; the layer itself first). Modifiers act in order, on
/// everything the ones before them made: jitter then array gives identical
/// copies; array then twist twists the whole row.
pub fn plan(modifiers: &[Modifier], time: f64) -> Vec<CopyPlan> {
    plan_with(modifiers, time, &|_, _| Vec::new())
}

/// One copy an external modifier makes: layer space transform, opacity and
/// an optional clip (see [`CopyPlan::clip`]).
pub type ExternalCopy = (Affine, f64, Option<kurbo::BezPath>);

/// Gives the copies of a modifier that depends on other layers (see [`plan_with`]).
pub type ExternalCopies<'a> = dyn Fn(usize, &ModifierKind) -> Vec<ExternalCopy> + 'a;

/// [`plan`], with `external(index, modifier)` giving the copies (layer
/// space transform, opacity) of modifiers that depend on other layers; an
/// empty list leaves things as they are.
pub fn plan_with(modifiers: &[Modifier], time: f64, external: &ExternalCopies<'_>) -> Vec<CopyPlan> {
    let mut out = vec![CopyPlan { transform: Affine::IDENTITY, opacity: 1.0, deforms: Vec::new(), clip: None }];
    for (index, m) in modifiers.iter().enumerate().filter(|(_, m)| m.enabled) {
        let steps = match m.kind.at(time) {
            Evaluated::Deform(d) => {
                for c in &mut out {
                    c.deforms.push((c.transform, d));
                }
                continue;
            }
            Evaluated::Array(a) => a.copies().into_iter().map(|(m, o)| (m, o, None)).collect(),
            Evaluated::Mirror(axis) => vec![(Affine::IDENTITY, 1.0, None), (axis, 1.0, None)],
            Evaluated::External => external(index, &m.kind),
        };
        if steps.is_empty() {
            continue;
        }
        let mut next = Vec::with_capacity(out.len() * steps.len());
        'fill: for (step, fade, clip) in steps {
            for c in &out {
                if next.len() >= MAX_COPIES {
                    break 'fill;
                }
                // A shard of a copy: the clip is in the space the copy is in, so
                // it goes back through the copy's own transform.
                let clip = match (&clip, &c.clip) {
                    (Some(k), _) => Some(c.transform.inverse() * k.clone()),
                    (None, own) => own.clone(),
                };
                next.push(CopyPlan { transform: step * c.transform, opacity: c.opacity * fade, clip, ..c.clone() });
            }
        }
        out = next;
    }
    out
}

/// Every copy a modifier stack makes at `time`: its layer-space transform
/// and opacity (see [`plan`]).
pub fn copies(modifiers: &[Modifier], time: f64) -> Vec<(Affine, f64)> {
    plan(modifiers, time).into_iter().map(|c| (c.transform, c.opacity)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    fn near(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn array_steps_build_on_each_other() {
        let row = Modifier::new(ModifierKind::array(3, Vec2::new(10.0, 0.0), Vec2::ZERO));
        let c = copies(std::slice::from_ref(&row), 0.0);
        assert_eq!(c.len(), 3);
        assert!(near(c[2].0 * Point::ZERO, Point::new(20.0, 0.0)));
        assert_eq!(c[0].0, Affine::IDENTITY, "the layer itself comes first, unmoved");

        // Four quarter turns around (0, 50) put a point at the top back where it started.
        let ring = Modifier::new(ModifierKind::radial_array(4, Vec2::new(0.0, 50.0)));
        let c = copies(std::slice::from_ref(&ring), 0.0);
        assert!(near(c[1].0 * Point::ZERO, Point::new(50.0, 50.0)), "{:?}", c[1].0 * Point::ZERO);
        assert!(near(c[2].0 * Point::ZERO, Point::new(0.0, 100.0)));

        // The offset doesn't turn with the copies (as in Lottie).
        let mut arc = row.clone();
        if let ModifierKind::Array { rotation, .. } = &mut arc.kind {
            *rotation = 90.0.into();
        }
        let c = copies(&[arc], 0.0);
        assert!(near(c[2].0 * Point::ZERO, Point::new(20.0, 0.0)), "{:?}", c[2].0 * Point::ZERO);
        assert!(near(c[1].0 * Point::new(1.0, 0.0), Point::new(10.0, 1.0)), "but the copy itself turns");
    }

    #[test]
    fn stacked_arrays_make_a_grid_and_fade() {
        let mut cols = Modifier::new(ModifierKind::array(3, Vec2::new(10.0, 0.0), Vec2::ZERO));
        let rows = Modifier::new(ModifierKind::array(2, Vec2::new(0.0, 20.0), Vec2::ZERO));
        if let ModifierKind::Array { end_opacity, .. } = &mut cols.kind {
            *end_opacity = 0.0.into();
        }
        let c = copies(&[cols.clone(), rows], 0.0);
        assert_eq!(c.len(), 6);
        assert!(near(c[5].0 * Point::ZERO, Point::new(20.0, 20.0)));
        assert_eq!(c.iter().map(|(_, o)| *o).collect::<Vec<_>>(), [1.0, 0.5, 0.0, 1.0, 0.5, 0.0]);
        // Switched off, it makes nothing extra.
        cols.enabled = false;
        assert_eq!(copies(&[cols], 0.0).len(), 1);
    }

    #[test]
    fn shape_changes_act_on_what_came_before() {
        let jitter = Modifier::new(ModifierKind::Jitter {
            amount: 5.0.into(),
            detail: 0.0.into(),
            seed: 1.0.into(),
            smooth: false,
        });
        let row = Modifier::new(ModifierKind::array(3, Vec2::new(10.0, 0.0), Vec2::ZERO));
        // Jitter first: made once, in the layer's own space, then copied.
        let p = plan(&[jitter.clone(), row.clone()], 0.0);
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|c| c.deforms.len() == 1 && c.deforms[0].0 == Affine::IDENTITY));
        // Array first: each copy is jittered where it stands.
        let p = plan(&[row, jitter], 0.0);
        assert_eq!(p[2].deforms[0].0, Affine::translate((20.0, 0.0)));
        assert_eq!(p[2].transform, Affine::translate((20.0, 0.0)));
    }

    #[test]
    fn mirror_reflects_across_its_axis() {
        // An upright axis through x = 100: a point at x = 70 lands at 130.
        let m = Modifier::new(ModifierKind::Mirror {
            origin: Vec2::new(100.0, 0.0).into(),
            rotation: 90.0.into(),
            merge: false,
        });
        let c = copies(&[m], 0.0);
        assert_eq!(c.len(), 2);
        assert!(near(c[1].0 * Point::new(70.0, 5.0), Point::new(130.0, 5.0)), "{:?}", c[1].0 * Point::new(70.0, 5.0));
        // A level axis flips top and bottom.
        assert!(near(reflection(Vec2::ZERO, 0.0) * Point::new(3.0, 4.0), Point::new(3.0, -4.0)));
    }

    #[test]
    fn huge_counts_are_capped_and_files_round_trip() {
        let big = Modifier::new(ModifierKind::array(900, Vec2::new(1.0, 0.0), Vec2::ZERO));
        assert_eq!(copies(&[big.clone(), big.clone()], 0.0).len(), MAX_COPIES);
        let json = serde_json::to_string(&big).unwrap();
        assert!(json.contains(r#""modifier":"array""#), "{json}");
        assert_eq!(serde_json::from_str::<Modifier>(&json).unwrap(), big);
    }
}
