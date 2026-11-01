use crate::{Color, Easing};
use kurbo::{BezPath, PathEl, Point, Vec2};
use serde::{Deserialize, Serialize};

/// A value type that can be interpolated between keyframes.
pub trait Animatable: Clone + PartialEq {
    /// Interpolates from `a` to `b`; `t` is eased progress and may leave `0..=1`
    /// for overshooting curves.
    fn lerp(a: &Self, b: &Self, t: f64) -> Self;
}

impl Animatable for f64 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        a + (b - a) * t
    }
}

impl Animatable for Vec2 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        a.lerp(*b, t)
    }
}

impl Animatable for Point {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        a.lerp(*b, t)
    }
}

impl Animatable for Color {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        let l = |x: f64, y: f64| x + (y - x) * t;
        Color::rgba(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), l(a.a, b.a))
    }
}

/// Paths morph point-by-point when both have the same structure; otherwise
/// they switch at the midpoint.
impl Animatable for BezPath {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        let (ea, eb) = (a.elements(), b.elements());
        let same_shape = ea.len() == eb.len()
            && ea.iter().zip(eb).all(|(x, y)| std::mem::discriminant(x) == std::mem::discriminant(y));
        if !same_shape {
            return if t < 0.5 { a.clone() } else { b.clone() };
        }
        let p = |x: Point, y: Point| x.lerp(y, t);
        ea.iter()
            .zip(eb)
            .map(|(x, y)| match (*x, *y) {
                (PathEl::MoveTo(a), PathEl::MoveTo(b)) => PathEl::MoveTo(p(a, b)),
                (PathEl::LineTo(a), PathEl::LineTo(b)) => PathEl::LineTo(p(a, b)),
                (PathEl::QuadTo(a1, a2), PathEl::QuadTo(b1, b2)) => PathEl::QuadTo(p(a1, b1), p(a2, b2)),
                (PathEl::CurveTo(a1, a2, a3), PathEl::CurveTo(b1, b2, b3)) => {
                    PathEl::CurveTo(p(a1, b1), p(a2, b2), p(a3, b3))
                }
                _ => PathEl::ClosePath,
            })
            .collect()
    }
}

/// A keyframe; `easing` controls interpolation from this keyframe to the next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe<T> {
    /// Time in seconds from the start of the composition.
    pub time: f64,
    pub value: T,
    #[serde(default)]
    pub easing: Easing,
}

/// A value that is either constant or animated by keyframes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property<T> {
    Static(T),
    /// Keyframes sorted by time, with no two at the same time. Never empty.
    Animated(Vec<Keyframe<T>>),
}

/// Two keyframe times closer than this are treated as the same keyframe.
pub const TIME_EPSILON: f64 = 1e-6;

impl<T: Animatable> Property<T> {
    pub fn is_animated(&self) -> bool {
        matches!(self, Property::Animated(_))
    }

    pub fn keyframes(&self) -> &[Keyframe<T>] {
        match self {
            Property::Static(_) => &[],
            Property::Animated(k) => k,
        }
    }

    /// Value at `time` (seconds). Before the first / after the last keyframe the
    /// end value is held.
    pub fn value_at(&self, time: f64) -> T {
        let keys = match self {
            Property::Static(v) => return v.clone(),
            Property::Animated(k) => k,
        };
        let Some(first) = keys.first() else {
            unreachable!("animated property with no keyframes");
        };
        if time <= first.time {
            return first.value.clone();
        }
        let last = keys.last().unwrap();
        if time >= last.time {
            return last.value.clone();
        }
        // Index of the first keyframe strictly after `time`.
        let next = keys.partition_point(|k| k.time <= time);
        let (a, b) = (&keys[next - 1], &keys[next]);
        let u = (time - a.time) / (b.time - a.time);
        match a.easing {
            Easing::Hold => a.value.clone(),
            e => T::lerp(&a.value, &b.value, e.apply(u)),
        }
    }

    /// Adds or replaces the keyframe at `time`. A static property becomes
    /// animated. `easing: None` keeps an existing keyframe's easing (or uses the
    /// default for a new one).
    pub fn set_keyframe(&mut self, time: f64, value: T, easing: Option<Easing>) {
        match self {
            Property::Static(_) => {
                *self = Property::Animated(vec![Keyframe { time, value, easing: easing.unwrap_or_default() }]);
            }
            Property::Animated(keys) => {
                if let Some(k) = keys.iter_mut().find(|k| (k.time - time).abs() < TIME_EPSILON) {
                    k.value = value;
                    if let Some(e) = easing {
                        k.easing = e;
                    }
                } else {
                    let at = keys.partition_point(|k| k.time < time);
                    keys.insert(at, Keyframe { time, value, easing: easing.unwrap_or_default() });
                }
            }
        }
    }

    /// Removes the keyframe at `time`, returning it. Removing the last keyframe
    /// turns the property static with that keyframe's value.
    pub fn remove_keyframe(&mut self, time: f64) -> Option<Keyframe<T>> {
        let Property::Animated(keys) = self else {
            return None;
        };
        let i = keys.iter().position(|k| (k.time - time).abs() < TIME_EPSILON)?;
        let removed = keys.remove(i);
        if keys.is_empty() {
            *self = Property::Static(removed.value.clone());
        }
        Some(removed)
    }

    /// Edits the value the way the UI does: animated properties get a keyframe
    /// at `time`, static ones are overwritten.
    pub fn set_value(&mut self, time: f64, value: T) {
        match self {
            Property::Static(v) => *v = value,
            Property::Animated(_) => self.set_keyframe(time, value, None),
        }
    }

    /// Moves the keyframe at `from` to `to`, replacing any keyframe already at
    /// `to`. Returns whether a keyframe was found. The property stays animated
    /// even when it has a single keyframe.
    pub fn move_keyframe(&mut self, from: f64, to: f64) -> bool {
        let Some(k) = self.remove_keyframe(from) else {
            return false;
        };
        self.set_keyframe(to, k.value, Some(k.easing));
        true
    }

    /// Sets the easing of the keyframe at `time`. Returns whether it exists.
    pub fn set_easing(&mut self, time: f64, easing: Easing) -> bool {
        let Property::Animated(keys) = self else {
            return false;
        };
        match keys.iter_mut().find(|k| (k.time - time).abs() < TIME_EPSILON) {
            Some(k) => {
                k.easing = easing;
                true
            }
            None => false,
        }
    }

    /// Replaces the keyframe list, sorting it and dropping duplicate times
    /// (the later entry wins). An empty list is rejected.
    pub fn from_keyframes(mut keys: Vec<Keyframe<T>>) -> Option<Self> {
        if keys.is_empty() {
            return None;
        }
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        let mut out: Vec<Keyframe<T>> = Vec::with_capacity(keys.len());
        for k in keys {
            match out.last_mut() {
                Some(prev) if (prev.time - k.time).abs() < TIME_EPSILON => *prev = k,
                _ => out.push(k),
            }
        }
        Some(Property::Animated(out))
    }
}

impl<T> From<T> for Property<T> {
    fn from(v: T) -> Self {
        Property::Static(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim(keys: &[(f64, f64, Easing)]) -> Property<f64> {
        Property::from_keyframes(keys.iter().map(|&(time, value, easing)| Keyframe { time, value, easing }).collect())
            .unwrap()
    }

    #[test]
    fn static_value() {
        assert_eq!(Property::Static(3.0).value_at(10.0), 3.0);
    }

    #[test]
    fn linear_interpolation_and_clamping() {
        let p = anim(&[(1.0, 0.0, Easing::Linear), (3.0, 100.0, Easing::Linear)]);
        assert_eq!(p.value_at(0.0), 0.0);
        assert_eq!(p.value_at(1.0), 0.0);
        assert_eq!(p.value_at(2.0), 50.0);
        assert_eq!(p.value_at(3.0), 100.0);
        assert_eq!(p.value_at(9.0), 100.0);
    }

    #[test]
    fn hold_and_multi_segment() {
        let p = anim(&[(0.0, 0.0, Easing::Hold), (1.0, 10.0, Easing::Linear), (2.0, 20.0, Easing::Linear)]);
        assert_eq!(p.value_at(0.999), 0.0);
        assert_eq!(p.value_at(1.0), 10.0);
        assert_eq!(p.value_at(1.5), 15.0);
    }

    #[test]
    fn set_and_remove_keyframes() {
        let mut p = Property::Static(5.0);
        p.set_value(0.0, 6.0);
        assert_eq!(p, Property::Static(6.0));

        p.set_keyframe(1.0, 10.0, None);
        p.set_keyframe(0.0, 0.0, Some(Easing::EASE_IN));
        assert_eq!(p.keyframes().len(), 2);
        assert_eq!(p.keyframes()[0].time, 0.0);
        assert_eq!(p.keyframes()[0].easing, Easing::EASE_IN);

        // Overwrite keeps easing.
        p.set_value(0.0, 1.0);
        assert_eq!(p.keyframes()[0].value, 1.0);
        assert_eq!(p.keyframes()[0].easing, Easing::EASE_IN);

        assert!(p.remove_keyframe(0.0).is_some());
        assert!(p.remove_keyframe(0.0).is_none());
        assert!(p.remove_keyframe(1.0).is_some());
        assert_eq!(p, Property::Static(10.0));
    }

    #[test]
    fn move_and_ease_keyframes() {
        let mut p = anim(&[(0.0, 0.0, Easing::Linear), (1.0, 10.0, Easing::EASE_IN)]);
        assert!(p.move_keyframe(1.0, 2.0));
        assert_eq!(p.keyframes()[1].time, 2.0);
        assert_eq!(p.keyframes()[1].easing, Easing::EASE_IN);
        // Moving onto another keyframe replaces it.
        assert!(p.move_keyframe(2.0, 0.0));
        assert_eq!(p.keyframes().len(), 1);
        assert_eq!(p.keyframes()[0].value, 10.0);
        // Moving the only keyframe keeps the property animated.
        assert!(p.move_keyframe(0.0, 5.0));
        assert!(p.is_animated());
        assert_eq!(p.keyframes()[0].time, 5.0);
        assert!(!p.move_keyframe(9.0, 1.0));

        assert!(p.set_easing(5.0, Easing::Hold));
        assert_eq!(p.keyframes()[0].easing, Easing::Hold);
        assert!(!p.set_easing(1.0, Easing::Hold));
    }

    #[test]
    fn from_keyframes_sorts_and_dedups() {
        let p = anim(&[(2.0, 2.0, Easing::Linear), (0.0, 0.0, Easing::Linear), (2.0, 3.0, Easing::Linear)]);
        let times: Vec<_> = p.keyframes().iter().map(|k| (k.time, k.value)).collect();
        assert_eq!(times, vec![(0.0, 0.0), (2.0, 3.0)]);
        assert!(Property::<f64>::from_keyframes(vec![]).is_none());
    }

    #[test]
    fn path_morph() {
        let mut a = BezPath::new();
        a.move_to((0.0, 0.0));
        a.line_to((10.0, 0.0));
        let mut b = BezPath::new();
        b.move_to((0.0, 10.0));
        b.line_to((20.0, 10.0));
        let mid = BezPath::lerp(&a, &b, 0.5);
        assert_eq!(mid.elements(), &[PathEl::MoveTo((0.0, 5.0).into()), PathEl::LineTo((15.0, 5.0).into())]);
    }

    #[test]
    fn color_lerp() {
        let c = Color::lerp(&Color::BLACK, &Color::WHITE, 0.5);
        assert_eq!(c, Color::rgb(0.5, 0.5, 0.5));
    }
}
