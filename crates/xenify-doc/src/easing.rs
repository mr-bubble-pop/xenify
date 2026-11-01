use serde::{Deserialize, Serialize};

/// How a keyframe interpolates towards the *next* keyframe.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Easing {
    #[default]
    Linear,
    /// Keep this keyframe's value until the next keyframe (step).
    Hold,
    /// CSS-style `cubic-bezier(x1, y1, x2, y2)`; `x1`/`x2` are clamped to `0..=1`.
    Bezier { x1: f64, y1: f64, x2: f64, y2: f64 },
    /// CSS-style `steps(count, jump)`: jumps in `count` equal steps.
    Steps {
        count: u32,
        #[serde(default)]
        jump: StepJump,
    },
    /// A physical spring settling on the next value (overshooting and
    /// wobbling when lightly damped). Mass 1; the keyframe span is the time
    /// it takes to settle.
    Spring { stiffness: f64, damping: f64 },
    /// CSS `linear()`: straight lines through these (progress, value) points.
    Stops { points: LinearStops },
}

/// The points of a CSS `linear()` easing (at most [`LinearStops::MAX`];
/// stored inline so easings stay small and copyable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearStops {
    n: u8,
    x: [f32; LinearStops::MAX],
    y: [f32; LinearStops::MAX],
}

impl LinearStops {
    pub const MAX: usize = 16;

    /// From `(progress, value)` points with increasing progress (resampled
    /// when there are more than [`Self::MAX`]).
    pub fn new(points: &[(f64, f64)]) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let pts: Vec<(f64, f64)> = if points.len() <= Self::MAX {
            points.to_vec()
        } else {
            (0..Self::MAX)
                .map(|i| {
                    let k = i as f64 * (points.len() - 1) as f64 / (Self::MAX - 1) as f64;
                    points[k.round() as usize]
                })
                .collect()
        };
        let mut s = Self { n: pts.len() as u8, x: [0.0; Self::MAX], y: [0.0; Self::MAX] };
        let mut last = f64::MIN;
        for (i, (x, y)) in pts.into_iter().enumerate() {
            let x = x.max(last);
            last = x;
            s.x[i] = x as f32;
            s.y[i] = y as f32;
        }
        Some(s)
    }

    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        (0..self.n as usize).map(|i| (self.x[i] as f64, self.y[i] as f64))
    }

    fn apply(&self, u: f64) -> f64 {
        let pts: Vec<(f64, f64)> = self.points().collect();
        if u <= pts[0].0 {
            return pts[0].1;
        }
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if u <= x1 {
                return if x1 - x0 < 1e-12 { y1 } else { y0 + (y1 - y0) * (u - x0) / (x1 - x0) };
            }
        }
        pts[pts.len() - 1].1
    }
}

impl Serialize for LinearStops {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v: Vec<[f64; 2]> = self.points().map(|(x, y)| [x, y]).collect();
        v.serialize(s)
    }
}

impl<'de> Deserialize<'de> for LinearStops {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v: Vec<[f64; 2]> = Vec::deserialize(d)?;
        let pts: Vec<(f64, f64)> = v.into_iter().map(|[x, y]| (x, y)).collect();
        Self::new(&pts).ok_or_else(|| serde::de::Error::custom("linear() easing needs at least two points"))
    }
}

/// A damped spring starting at rest at 0 and settling on 1: its value at
/// `u` of the way through the settling time.
fn spring(stiffness: f64, damping: f64, u: f64) -> f64 {
    if u >= 1.0 {
        return 1.0;
    }
    let w = stiffness.max(1e-6).sqrt();
    let zeta = (damping.max(0.0) / (2.0 * w)).max(1e-3);
    if zeta < 1.0 {
        let decay = zeta * w;
        let wd = w * (1.0 - zeta * zeta).sqrt();
        // Settled once the wobble is within 0.1%.
        let t = u * (1000f64).ln() / decay;
        1.0 - (-decay * t).exp() * ((wd * t).cos() + decay / wd * (wd * t).sin())
    } else {
        // Critically (or over-) damped: no overshoot.
        let t = u * 9.23 / w;
        1.0 - (-w * t).exp() * (1.0 + w * t)
    }
}

/// Where a [`Easing::Steps`] easing jumps, as in CSS `steps()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepJump {
    /// Jumps at the start of each step (`jump-start` / `start`).
    Start,
    /// Jumps at the end of each step (`jump-end` / `end`, the CSS default).
    #[default]
    End,
    /// No jump at either end: holds the first and last values (`jump-none`).
    None,
    /// Jumps at both ends (`jump-both`).
    Both,
}

impl StepJump {
    pub const ALL: [StepJump; 4] = [StepJump::Start, StepJump::End, StepJump::None, StepJump::Both];

    /// The CSS keyword.
    pub fn css(self) -> &'static str {
        match self {
            StepJump::Start => "jump-start",
            StepJump::End => "jump-end",
            StepJump::None => "jump-none",
            StepJump::Both => "jump-both",
        }
    }
}

impl Easing {
    pub const EASE: Easing = Easing::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 };
    pub const EASE_IN: Easing = Easing::Bezier { x1: 0.42, y1: 0.0, x2: 1.0, y2: 1.0 };
    pub const EASE_OUT: Easing = Easing::Bezier { x1: 0.0, y1: 0.0, x2: 0.58, y2: 1.0 };
    pub const EASE_IN_OUT: Easing = Easing::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
    /// A gentle ease in and out: both handles a third of the way along.
    pub const EASY_EASE: Easing = Easing::Bezier { x1: 0.333, y1: 0.0, x2: 0.667, y2: 1.0 };

    /// Named presets, useful for UI pickers and for the AI tool schema.
    pub const PRESETS: &'static [(&'static str, Easing)] = &[
        ("linear", Easing::Linear),
        // CSS `step-end`: jump at the end (Blender calls it Constant).
        ("step_end", Easing::Hold),
        ("ease", Easing::EASE),
        ("ease_in", Easing::EASE_IN),
        ("ease_out", Easing::EASE_OUT),
        ("ease_in_out", Easing::EASE_IN_OUT),
        // A gentle ease in and out (both handles at a third).
        ("smooth", Easing::EASY_EASE),
        // The rest are the classic Penner curves (as on easings.net), in CSS form.
        ("ease_in_sine", Easing::Bezier { x1: 0.12, y1: 0.0, x2: 0.39, y2: 0.0 }),
        ("ease_out_sine", Easing::Bezier { x1: 0.61, y1: 1.0, x2: 0.88, y2: 1.0 }),
        ("ease_in_out_sine", Easing::Bezier { x1: 0.37, y1: 0.0, x2: 0.63, y2: 1.0 }),
        ("ease_in_cubic", Easing::Bezier { x1: 0.32, y1: 0.0, x2: 0.67, y2: 0.0 }),
        ("ease_out_cubic", Easing::Bezier { x1: 0.33, y1: 1.0, x2: 0.68, y2: 1.0 }),
        ("ease_in_out_cubic", Easing::Bezier { x1: 0.65, y1: 0.0, x2: 0.35, y2: 1.0 }),
        ("ease_in_expo", Easing::Bezier { x1: 0.7, y1: 0.0, x2: 0.84, y2: 0.0 }),
        ("ease_out_expo", Easing::Bezier { x1: 0.16, y1: 1.0, x2: 0.3, y2: 1.0 }),
        ("ease_in_out_expo", Easing::Bezier { x1: 0.87, y1: 0.0, x2: 0.13, y2: 1.0 }),
        ("ease_in_back", Easing::Bezier { x1: 0.36, y1: 0.0, x2: 0.66, y2: -0.56 }),
        ("ease_out_back", Easing::Bezier { x1: 0.34, y1: 1.56, x2: 0.64, y2: 1.0 }),
        ("ease_in_out_back", Easing::Bezier { x1: 0.68, y1: -0.6, x2: 0.32, y2: 1.6 }),
        // Springs: physical overshoot and settle.
        ("spring", Easing::Spring { stiffness: 100.0, damping: 10.0 }),
        ("spring_gentle", Easing::Spring { stiffness: 100.0, damping: 16.0 }),
        ("spring_bouncy", Easing::Spring { stiffness: 180.0, damping: 6.0 }),
    ];

    pub fn preset(name: &str) -> Option<Easing> {
        // Older names, still accepted.
        let name = match name {
            "hold" => "step_end",
            "easy_ease" => "smooth",
            other => other,
        };
        Self::PRESETS.iter().find(|(n, _)| *n == name).map(|(_, e)| *e)
    }

    /// Parses a CSS `<easing-function>`: a keyword (`ease-in`, `step-end`, …),
    /// `cubic-bezier(x1, y1, x2, y2)` or `steps(n[, jump])`.
    pub fn from_css(css: &str) -> Option<Easing> {
        let css = css.trim().to_ascii_lowercase();
        let Some((func, args)) = css.strip_suffix(')').and_then(|c| c.split_once('(')) else {
            return match css.as_str() {
                "step-end" => Some(Easing::Hold),
                "step-start" => Some(Easing::Steps { count: 1, jump: StepJump::Start }),
                keyword => Self::preset(&keyword.replace('-', "_")),
            };
        };
        let args: Vec<&str> = args.split(',').map(str::trim).collect();
        match (func.trim(), args.as_slice()) {
            ("cubic-bezier", [x1, y1, x2, y2]) => {
                let n = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite());
                Some(Easing::Bezier { x1: n(x1)?, y1: n(y1)?, x2: n(x2)?, y2: n(y2)? })
            }
            ("linear", stops) => parse_linear(stops),
            ("steps", [count, rest @ ..]) if rest.len() <= 1 => {
                let jump = match rest.first().copied() {
                    None | Some("jump-end" | "end") => StepJump::End,
                    Some("jump-start" | "start") => StepJump::Start,
                    Some("jump-none") => StepJump::None,
                    Some("jump-both") => StepJump::Both,
                    Some(_) => return None,
                };
                Some(Easing::Steps { count: count.parse().ok().filter(|c| *c > 0)?, jump })
            }
            _ => None,
        }
    }

    /// Maps linear progress `u` in `0..=1` to eased progress.
    /// `Hold` returns 0 for every `u < 1`.
    pub fn apply(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Easing::Linear => u,
            Easing::Hold => {
                if u >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Easing::Bezier { x1, y1, x2, y2 } => cubic_bezier(x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2, u),
            Easing::Steps { count, jump } => steps(count, jump, u),
            Easing::Spring { stiffness, damping } => spring(stiffness, damping, u),
            Easing::Stops { points } => points.apply(u),
        }
    }

    /// The CSS `<easing-function>` for this easing.
    pub fn css(self) -> String {
        match self {
            Easing::Linear => "linear".into(),
            Easing::Hold => "step-end".into(),
            Easing::Bezier { x1, y1, x2, y2 } => {
                let n = |v: f64| format!("{}", (v * 1000.0).round() / 1000.0);
                format!("cubic-bezier({}, {}, {}, {})", n(x1.clamp(0.0, 1.0)), n(y1), n(x2.clamp(0.0, 1.0)), n(y2))
            }
            Easing::Steps { count, jump } => format!("steps({}, {})", steps_count(count, jump), jump.css()),
            Easing::Spring { .. } => {
                // CSS has no spring: sample it into linear().
                let n = 40;
                let pts: Vec<String> = (0..=n)
                    .map(|i| format!("{}", (self.apply(i as f64 / n as f64) * 1000.0).round() / 1000.0))
                    .collect();
                format!("linear({})", pts.join(", "))
            }
            Easing::Stops { points } => {
                let n = |v: f64| format!("{}", (v * 1000.0).round() / 1000.0);
                let pts: Vec<String> = points.points().map(|(x, y)| format!("{} {}%", n(y), n(x * 100.0))).collect();
                format!("linear({})", pts.join(", "))
            }
        }
    }
}

/// CSS `linear(0, 0.25 40%, 1)`: values with optional positions; missing
/// positions are spread evenly between the known ones.
fn parse_linear(args: &[&str]) -> Option<Easing> {
    let mut pts: Vec<(Option<f64>, f64)> = Vec::new();
    for a in args {
        let mut parts = a.split_whitespace();
        let y: f64 = parts.next()?.parse().ok()?;
        let mut positions: Vec<f64> = Vec::new();
        for p in parts {
            positions.push(p.strip_suffix('%')?.parse::<f64>().ok()? / 100.0);
        }
        match positions[..] {
            [] => pts.push((None, y)),
            [x] => pts.push((Some(x), y)),
            [x0, x1] => {
                pts.push((Some(x0), y));
                pts.push((Some(x1), y));
            }
            _ => return None,
        }
    }
    if pts.len() < 2 {
        return None;
    }
    let last = pts.len() - 1;
    if pts[0].0.is_none() {
        pts[0].0 = Some(0.0);
    }
    if pts[last].0.is_none() {
        pts[last].0 = Some(1.0);
    }
    // Spread runs of missing positions evenly.
    let mut i = 0;
    while i < pts.len() {
        if pts[i].0.is_none() {
            let start = i - 1;
            let end = (i..pts.len()).find(|k| pts[*k].0.is_some())?;
            let (a, b) = (pts[start].0?, pts[end].0?);
            for (k, p) in pts.iter_mut().enumerate().take(end).skip(i) {
                p.0 = Some(a + (b - a) * (k - start) as f64 / (end - start) as f64);
            }
            i = end;
        }
        i += 1;
    }
    let pts: Vec<(f64, f64)> = pts.into_iter().map(|(x, y)| (x.unwrap_or(0.0), y)).collect();
    LinearStops::new(&pts).map(|points| Easing::Stops { points })
}

/// `jump-none` needs at least two steps.
fn steps_count(count: u32, jump: StepJump) -> u32 {
    if jump == StepJump::None { count.max(2) } else { count.max(1) }
}

/// The CSS step easing function (CSS Easing Functions Level 1, §3.3).
fn steps(count: u32, jump: StepJump, u: f64) -> f64 {
    let n = steps_count(count, jump);
    let mut step = (u * n as f64).floor() as i64;
    if matches!(jump, StepJump::Start | StepJump::Both) {
        step += 1;
    }
    let jumps = match jump {
        StepJump::Both => n + 1,
        StepJump::None => n - 1,
        _ => n,
    } as i64;
    step.clamp(0, jumps) as f64 / jumps as f64
}

/// Solves the CSS cubic-bezier timing function for `x`, returning `y`.
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // B(t) = 3(1-t)^2 t p1 + 3(1-t) t^2 p2 + t^3, with p0 = 0 and p3 = 1.
    let bez = |t: f64, p1: f64, p2: f64| {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t
    };
    let dbez = |t: f64, p1: f64, p2: f64| {
        let mt = 1.0 - t;
        3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
    };

    // Newton-Raphson, falling back to bisection when the slope is too flat.
    let mut t = x;
    for _ in 0..8 {
        let err = bez(t, x1, x2) - x;
        if err.abs() < 1e-7 {
            return bez(t, y1, y2);
        }
        let d = dbez(t, x1, x2);
        if d.abs() < 1e-6 {
            break;
        }
        t -= err / d;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    t = x;
    for _ in 0..50 {
        let v = bez(t, x1, x2);
        if (v - x).abs() < 1e-7 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }
    bez(t, y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_fixed() {
        for (_, e) in Easing::PRESETS {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
        }
    }

    #[test]
    fn linear_bezier_is_identity() {
        let e = Easing::Bezier { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };
        for i in 0..=10 {
            let u = i as f64 / 10.0;
            assert!((e.apply(u) - u).abs() < 1e-5, "u={u}");
        }
    }

    #[test]
    fn ease_in_out_is_symmetric_and_monotonic() {
        let e = Easing::EASE_IN_OUT;
        assert!((e.apply(0.5) - 0.5).abs() < 1e-5);
        assert!(e.apply(0.25) < 0.25);
        assert!(e.apply(0.75) > 0.75);
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = e.apply(i as f64 / 100.0);
            assert!(v >= prev);
            prev = v;
        }
    }

    #[test]
    fn back_presets_overshoot() {
        let out = Easing::preset("ease_out_back").unwrap();
        assert!((0..100).any(|i| out.apply(i as f64 / 100.0) > 1.0));
        let inn = Easing::preset("ease_in_back").unwrap();
        assert!((0..100).any(|i| inn.apply(i as f64 / 100.0) < 0.0));
    }

    #[test]
    fn css_steps() {
        let s = |count, jump, u| Easing::Steps { count, jump }.apply(u);
        assert_eq!(s(4, StepJump::End, 0.3), 0.25);
        assert_eq!(s(4, StepJump::Start, 0.3), 0.5);
        assert_eq!(s(4, StepJump::Start, 0.0), 0.25);
        assert_eq!(s(3, StepJump::None, 0.5), 0.5);
        assert_eq!(s(3, StepJump::None, 0.1), 0.0);
        assert_eq!(s(3, StepJump::Both, 0.0), 0.25);
        for jump in StepJump::ALL {
            assert_eq!(s(5, jump, 1.0), 1.0, "{jump:?}");
            assert_eq!(s(0, jump, 1.0), 1.0, "{jump:?} with no steps");
        }
    }

    #[test]
    fn css_strings() {
        assert_eq!(Easing::EASE_IN.css(), "cubic-bezier(0.42, 0, 1, 1)");
        assert_eq!(Easing::Steps { count: 4, jump: StepJump::Both }.css(), "steps(4, jump-both)");
        for e in [Easing::EASE_OUT, Easing::Linear, Easing::Hold, Easing::Steps { count: 2, jump: StepJump::None }] {
            assert_eq!(Easing::from_css(&e.css()), Some(e));
        }
        assert_eq!(Easing::from_css("ease-in-out"), Some(Easing::EASE_IN_OUT));
        assert_eq!(Easing::from_css("steps(3, start)"), Some(Easing::Steps { count: 3, jump: StepJump::Start }));
        assert_eq!(Easing::from_css("steps(0)"), None);
        assert_eq!(Easing::from_css("cubic-bezier(1, 2)"), None);
        let e: Easing = serde_json::from_str(r#"{"type":"steps","count":3}"#).unwrap();
        assert_eq!(e, Easing::Steps { count: 3, jump: StepJump::End });
    }

    #[test]
    fn hold_steps() {
        assert_eq!(Easing::Hold.apply(0.99), 0.0);
    }

    #[test]
    fn springs_and_linear_stops() {
        let s = Easing::preset("spring").unwrap();
        assert_eq!(s.apply(0.0), 0.0);
        assert_eq!(s.apply(1.0), 1.0);
        let peak = (0..100).map(|i| s.apply(i as f64 / 100.0)).fold(0.0, f64::max);
        assert!(peak > 1.1, "a lightly damped spring overshoots: {peak}");
        let gentle = Easing::Spring { stiffness: 100.0, damping: 25.0 };
        assert!((0..=100).all(|i| gentle.apply(i as f64 / 100.0) <= 1.0 + 1e-9), "critically damped: no overshoot");
        assert!(s.css().starts_with("linear(0, "));

        let l = Easing::from_css("linear(0, 0.25 40%, 1)").unwrap();
        assert!((l.apply(0.4) - 0.25).abs() < 1e-6 && (l.apply(0.7) - 0.625).abs() < 1e-6);
        assert_eq!(Easing::from_css(&l.css()), Some(l));
        let even = Easing::from_css("linear(0, 0.5, 0.8, 1)").unwrap();
        assert!((even.apply(1.0 / 3.0) - 0.5).abs() < 1e-6, "positions spread evenly");
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<Easing>(&json).unwrap(), l);
        assert!(Easing::from_css("linear(1)").is_none());
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_string(&Easing::EASE_IN).unwrap();
        assert_eq!(json, r#"{"type":"bezier","x1":0.42,"y1":0.0,"x2":1.0,"y2":1.0}"#);
        let e: Easing = serde_json::from_str(r#"{"type":"hold"}"#).unwrap();
        assert_eq!(e, Easing::Hold);
    }
}
