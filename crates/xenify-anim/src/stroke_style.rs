//! Turns styled strokes (pen types, rounded caps) into filled vector marks.
//!
//! Plain strokes are left to the renderer's native stroker ([`marks`] returns
//! `None`). Everything else becomes a list of filled shapes — ribbons, dots,
//! grain — generated along the path. Randomness is keyed to the position along
//! the path (not time), so textures don't flicker from frame to frame.

use crate::SceneStroke;
use xenify_doc::{LineCap, LineJoin, PenKind, PenStyle};
use kurbo::{BezPath, ParamCurveArclen, PathEl, Point, Shape, Vec2};

/// One filled shape of a styled stroke, drawn in the stroke colour × `alpha`.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeMark {
    pub path: BezPath,
    pub alpha: f64,
}

/// Upper bound on generated shapes per stroke (keeps huge paths responsive).
const MAX_MARKS: usize = 6000;

#[derive(Clone, Copy)]
struct Sample {
    p: Point,
    /// Unit tangent.
    t: Vec2,
    /// Distance along the subpath.
    s: f64,
}

impl Sample {
    fn normal(&self) -> Vec2 {
        Vec2::new(-self.t.y, self.t.x)
    }
}

struct Sub {
    pts: Vec<Sample>,
    closed: bool,
    len: f64,
}

/// The marks for `path` drawn with `stroke`, or `None` when the native stroker
/// can draw it directly (a plain line with a standard cap).
pub fn marks(
    path: &BezPath,
    stroke: &SceneStroke,
    style: &PenStyle,
    cap_roundness: f64,
    seed: u64,
) -> Option<Vec<StrokeMark>> {
    let w = stroke.width;
    let rounded_cap = stroke.cap == LineCap::Square && cap_roundness > 0.0;
    if w <= 0.0 || (style.kind == PenKind::Solid && !rounded_cap) {
        return None;
    }
    let seed = seed ^ style.seed.wrapping_mul(0x9E37_79B9);
    let mut out = Vec::new();
    let step = (w / 3.0).clamp(0.75, 6.0);
    let subs = sample(path, step);
    let flow = style.flow.clamp(0.0, 1.0);
    let st = style;
    let taper = |s: f64, len: f64| taper_at(s, len, st.taper_start, st.taper_end);
    let noise = |ch: u64, x: f64| smooth_noise(seed, ch, x);
    use PenKind::*;
    match style.kind {
        Solid => {
            // Only here for rounded caps: native body with flat ends plus the caps.
            let mut body = kurbo::stroke(
                path.iter(),
                &kurbo::Stroke::new(w).with_join(join(stroke.join)).with_caps(kurbo::Cap::Butt),
                &kurbo::StrokeOpts::default(),
                0.05,
            );
            for sub in subs.iter().filter(|s| !s.closed) {
                add_caps(&mut body, sub, w, cap_roundness);
            }
            out.push(StrokeMark { path: body, alpha: 1.0 });
        }
        Dashed | DashDot | Dotted => {
            let on = (style.dash.max(0.0) * w).max(0.0);
            let off = (style.gap.max(0.05) * w).max(0.5);
            let cap = if rounded_cap { None } else { Some(stroke.cap) };
            for sub in &subs {
                let mut s = 0.0;
                let mut dot_next = false;
                while s < sub.len && out.len() < MAX_MARKS {
                    let dot = style.kind == Dotted || (style.kind == DashDot && dot_next);
                    if dot {
                        let smp = at(sub, s);
                        out.push(StrokeMark { path: circle(smp.p, w / 2.0), alpha: 1.0 });
                        s += w + off;
                    } else {
                        let end = (s + on.max(w * 0.1)).min(sub.len);
                        let piece = polyline(&slice(sub, s, end));
                        let mut dash = kurbo::stroke(
                            piece.iter(),
                            &kurbo::Stroke::new(w).with_join(join(stroke.join)).with_caps(match cap {
                                Some(LineCap::Round) => kurbo::Cap::Round,
                                Some(LineCap::Square) => kurbo::Cap::Square,
                                _ => kurbo::Cap::Butt,
                            }),
                            &kurbo::StrokeOpts::default(),
                            0.05,
                        );
                        if cap.is_none() {
                            let part = Sub { pts: slice(sub, s, end), closed: false, len: end - s };
                            add_caps(&mut dash, &part, w, cap_roundness);
                        }
                        out.push(StrokeMark { path: dash, alpha: 1.0 });
                        s = end + off;
                    }
                    dot_next = !dot_next;
                }
            }
        }
        DoubleLine => {
            let n = style.count.clamp(2, 5) as f64;
            let line_w = w * style.thinning.clamp(0.05, 0.9) / n * 2.0;
            for k in 0..n as u32 {
                let off = -w / 2.0 + line_w / 2.0 + (w - line_w) * k as f64 / (n - 1.0);
                for sub in &subs {
                    out.push(ribbon(sub, |_| (off, line_w / 2.0), 1.0));
                }
            }
        }
        Tapered => {
            for sub in &subs {
                out.push(ribbon(sub, |p| (0.0, w / 2.0 * taper(p.s, sub.len)), 1.0));
            }
        }
        Calligraphy => {
            let nib = Vec2::new(style.angle.to_radians().cos(), style.angle.to_radians().sin());
            let thin = style.thinning.clamp(0.02, 1.0);
            for sub in &subs {
                out.push(ribbon(
                    sub,
                    |p| {
                        let across = (p.t.x * nib.y - p.t.y * nib.x).abs();
                        (0.0, w / 2.0 * (thin + (1.0 - thin) * across) * taper(p.s, sub.len))
                    },
                    flow,
                ));
            }
        }
        Brush | Ink | Rough => {
            let (swell, wobble) = match style.kind {
                Brush => (style.thinning, style.roughness * 0.25),
                Ink => (style.thinning * 0.3, style.roughness * 0.15),
                _ => (style.thinning, style.roughness * 0.6),
            };
            let scale = if style.kind == Rough { 1.6 } else { 5.0 };
            for sub in &subs {
                out.push(ribbon(
                    sub,
                    |p| {
                        let x = p.s / (w * scale).max(1.0);
                        let press = 1.0 - swell * 0.5 * (1.0 + noise(1, x));
                        (wobble * w * noise(2, x * 0.7), w / 2.0 * press * taper(p.s, sub.len))
                    },
                    flow,
                ));
            }
        }
        Marker => {
            for sub in &subs {
                out.push(ribbon(
                    sub,
                    |p| {
                        let x = p.s / (w * 4.0).max(1.0);
                        (0.0, w / 2.0 * (1.0 - style.roughness * 0.3 * (0.5 + 0.5 * noise(1, x))))
                    },
                    flow,
                ));
            }
        }
        FeltTip => {
            for (k, scale) in [1.0, 0.82, 0.64].iter().enumerate() {
                for sub in &subs {
                    out.push(ribbon(
                        sub,
                        |p| {
                            let x = p.s / (w * 4.0).max(1.0);
                            (0.0, w / 2.0 * scale * (1.0 - style.roughness * 0.2 * (0.5 + 0.5 * noise(k as u64, x))))
                        },
                        flow * 0.4,
                    ));
                }
            }
        }
        Pencil => {
            // A thin, slightly uneven graphite line with dense grain on top.
            for sub in &subs {
                out.push(ribbon(
                    sub,
                    |p| {
                        let x = p.s / (w * 2.0).max(1.0);
                        (
                            style.roughness * w * 0.15 * noise(6, x),
                            w * 0.3 * taper(p.s, sub.len) * (0.8 + 0.2 * noise(7, x * 3.0)),
                        )
                    },
                    flow * 0.35,
                ));
                speckles(&mut out, sub, w * 0.6, w * 0.05, 0.4 + style.grain, flow * 0.8, seed);
            }
        }
        Charcoal => {
            // A broad smudge with ragged edges, dark grain and loose dust.
            for sub in &subs {
                out.push(ribbon(
                    sub,
                    |p| {
                        let x = p.s / (w * 0.6).max(0.5);
                        let rag = 1.0 - style.roughness * 0.45 * (0.5 + 0.5 * noise(8, x));
                        (style.roughness * w * 0.1 * noise(9, x * 0.3), w / 2.0 * rag * taper(p.s, sub.len))
                    },
                    flow * 0.45,
                ));
                speckles(&mut out, sub, w, w * 0.09, 0.3 + style.grain, flow * 0.75, seed);
                speckles(&mut out, sub, w * 1.5, w * 0.05, style.grain * 0.4, flow * 0.4, seed ^ 0xC4A2);
            }
        }
        Pastel => {
            for sub in &subs {
                out.push(ribbon(sub, |p| (0.0, w / 2.0 * taper(p.s, sub.len)), flow * 0.22));
                speckles(&mut out, sub, w, w * 0.12, style.grain, flow * 0.7, seed);
            }
        }
        Crayon => {
            let streaks = 9u32;
            for sub in &subs {
                out.push(ribbon(sub, |_| (0.0, w / 2.0 * 0.9), flow * 0.3));
            }
            for (si, sub) in subs.iter().enumerate() {
                for k in 0..streaks {
                    let off = (k as f64 + 0.5) / streaks as f64 * w - w / 2.0;
                    let mut s = 0.0;
                    let mut i = 0u64;
                    while s < sub.len && out.len() < MAX_MARKS {
                        i += 1;
                        let h = hash01(seed, (si as u64 * 31 + k as u64) * 100_003 + i, 8);
                        let len = w * (1.5 + 4.0 * h);
                        let end = (s + len).min(sub.len);
                        if h < 0.45 + 0.55 * style.grain {
                            let piece = Sub { pts: slice(sub, s, end), closed: false, len: end - s };
                            out.push(ribbon(&piece, |_| (off, w / streaks as f64 * 0.45), flow * (0.6 + 0.4 * h)));
                        }
                        s = end + w * 0.25 * (1.0 - style.grain) * h;
                    }
                }
            }
        }
        Watercolor => {
            let layers = style.count.clamp(1, 6);
            for k in 0..layers {
                for sub in &subs {
                    out.push(ribbon(
                        sub,
                        |p| {
                            let x = p.s / (w * 3.0).max(1.0) + k as f64 * 13.1;
                            let spread = 1.0 + style.roughness * 0.35 * noise(10 + k as u64, x);
                            (
                                style.roughness * w * 0.15 * noise(20 + k as u64, x),
                                w / 2.0 * spread * taper(p.s, sub.len),
                            )
                        },
                        flow,
                    ));
                }
            }
        }
        Airbrush => {
            for (si, sub) in subs.iter().enumerate() {
                let per_len = 1.5 + 6.0 * style.grain;
                let n = ((sub.len / w.max(0.5)) * per_len * 8.0) as u64;
                for i in 0..n.min(MAX_MARKS as u64) {
                    let h = |c| hash01(seed, si as u64 * 1_000_003 + i, c);
                    let smp = at(sub, h(1) * sub.len);
                    // Roughly gaussian across the line (sum of uniforms).
                    let g = (h(2) + h(3) + h(4) - 1.5) / 1.5;
                    let c = smp.p + smp.normal() * (g * w * 0.8);
                    out.push(StrokeMark {
                        path: circle(c, w * (0.03 + 0.05 * h(5))),
                        alpha: flow * (1.0 - g.abs()).max(0.1),
                    });
                }
            }
        }
        Bristle => {
            let n = style.count.clamp(2, 24);
            for k in 0..n {
                let off = (k as f64 + 0.5) / n as f64 * w - w / 2.0;
                let bw = w / n as f64 * 0.45;
                let lag = hash01(seed, k as u64, 11) * 0.3;
                for sub in &subs {
                    out.push(ribbon(
                        sub,
                        |p| {
                            let x = p.s / (w * 4.0).max(1.0) + k as f64 * 7.3;
                            let t = taper_at(p.s, sub.len, style.taper_start + lag, style.taper_end + lag);
                            (off + style.roughness * w * 0.1 * noise(30 + k as u64, x), bw * t)
                        },
                        flow * (0.7 + 0.3 * hash01(seed, k as u64, 12)),
                    ));
                }
            }
        }
        Glow => {
            let layers = style.count.clamp(1, 8);
            // Many faint layers of slowly growing width read as a soft halo.
            let n = layers * 3;
            for k in (1..=n).rev() {
                let spread = 1.0 + k as f64 * 0.25;
                for sub in &subs {
                    out.push(ribbon(sub, |_| (0.0, w / 2.0 * spread), flow * 0.35 / n as f64 * 2.0));
                }
            }
            for sub in &subs {
                out.push(ribbon(sub, |_| (0.0, w / 2.0), 1.0));
            }
        }
        Sketchy => {
            let passes = style.count.clamp(1, 6);
            for k in 0..passes {
                for sub in &subs {
                    out.push(ribbon(
                        sub,
                        |p| {
                            let x = p.s / (w * 8.0).max(1.0) + k as f64 * 5.7;
                            (style.roughness * w * 0.9 * noise(40 + k as u64, x), w * 0.3 * taper(p.s, sub.len))
                        },
                        flow,
                    ));
                }
            }
        }
    }
    out.retain(|m| !m.path.elements().is_empty() && m.alpha > 0.0);
    Some(out)
}

fn join(j: LineJoin) -> kurbo::Join {
    match j {
        LineJoin::Miter => kurbo::Join::Miter,
        LineJoin::Round => kurbo::Join::Round,
        LineJoin::Bevel => kurbo::Join::Bevel,
    }
}

/// 0 at a tapered end, 1 away from the ends.
fn taper_at(s: f64, len: f64, start: f64, end: f64) -> f64 {
    let ramp = |d: f64, amount: f64| {
        if amount <= 0.0 {
            return 1.0;
        }
        let span = amount.clamp(0.0, 1.0) * len * 0.5;
        let x = (d / span.max(1e-9)).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    (ramp(s, start) * ramp(len - s, end)).max(0.02)
}

/// A filled band along `sub`: `f` gives (centre offset along the normal, half width).
fn ribbon(sub: &Sub, f: impl Fn(&Sample) -> (f64, f64), alpha: f64) -> StrokeMark {
    let mut left = Vec::with_capacity(sub.pts.len());
    let mut right = Vec::with_capacity(sub.pts.len());
    for smp in &sub.pts {
        let (off, half) = f(smp);
        let n = smp.normal();
        let c = smp.p + n * off;
        left.push(c + n * half.max(0.0));
        right.push(c - n * half.max(0.0));
    }
    let mut path = BezPath::new();
    if left.len() < 2 {
        return StrokeMark { path, alpha };
    }
    if sub.closed {
        // Two loops of opposite direction: the band between them fills.
        push_loop(&mut path, left.iter().copied());
        push_loop(&mut path, right.iter().rev().copied());
    } else {
        push_loop(&mut path, left.iter().copied().chain(right.iter().rev().copied()));
    }
    StrokeMark { path, alpha }
}

fn push_loop(path: &mut BezPath, mut pts: impl Iterator<Item = Point>) {
    let Some(first) = pts.next() else { return };
    path.move_to(first);
    for p in pts {
        path.line_to(p);
    }
    path.close_path();
}

/// Small grains scattered within the line's width.
fn speckles(out: &mut Vec<StrokeMark>, sub: &Sub, width: f64, size: f64, density: f64, alpha: f64, seed: u64) {
    let n = ((sub.len / size.max(0.3)) * (0.5 + 3.0 * density)) as u64;
    for i in 0..n {
        if out.len() >= MAX_MARKS {
            return;
        }
        let h = |c| hash01(seed, i, c);
        let smp = at(sub, h(1) * sub.len);
        let edge = taper_at(smp.s, sub.len, 0.15, 0.15);
        let across = (h(2) - 0.5) * width * edge;
        let c = smp.p + smp.normal() * across;
        let r = size * (0.5 + h(3));
        // Irregular grains: squashed, rotated quads read as chalk better than circles.
        let a = h(4) * std::f64::consts::TAU;
        let (u, v) = (Vec2::new(a.cos(), a.sin()) * r, Vec2::new(-a.sin(), a.cos()) * r * (0.4 + 0.6 * h(5)));
        let mut path = BezPath::new();
        push_loop(&mut path, [c + u, c + v, c - u, c - v].into_iter());
        out.push(StrokeMark { path, alpha: alpha * (0.4 + 0.6 * h(6)) });
    }
}

/// Caps with rounded corners at the open ends of `sub` (a flat-ended stroke).
fn add_caps(path: &mut BezPath, sub: &Sub, w: f64, roundness: f64) {
    let (Some(first), Some(last)) = (sub.pts.first(), sub.pts.last()) else { return };
    for (p, dir) in [(first.p, -first.t), (last.p, last.t)] {
        path.extend(cap_shape(p, dir, w, roundness).elements().iter().copied());
    }
}

/// The part of a cap beyond the line's end: half a width long, with its outer
/// corners rounded (`roundness` 0 = square, 1 = semicircle).
pub fn cap_shape(end: Point, dir: Vec2, w: f64, roundness: f64) -> BezPath {
    let h = w / 2.0;
    let r = roundness.clamp(0.0, 1.0) * h;
    let u = dir.normalize();
    let v = Vec2::new(-u.y, u.x);
    let at = |a: f64, b: f64| end + u * a + v * b;
    // Circle-ish corners: cubic with the standard 0.5523 handle factor.
    let k = 0.552_284_75 * r;
    let mut p = BezPath::new();
    p.move_to(at(0.0, -h));
    p.line_to(at(h - r, -h));
    p.curve_to(at(h - r + k, -h), at(h, -h + r - k), at(h, -h + r));
    p.line_to(at(h, h - r));
    p.curve_to(at(h, h - r + k), at(h - r + k, h), at(h - r, h));
    p.line_to(at(0.0, h));
    p.close_path();
    p
}

fn circle(c: Point, r: f64) -> BezPath {
    kurbo::Circle::new(c, r.max(0.05)).to_path(0.05)
}

/// Samples every subpath every ~`step` along its length.
fn sample(path: &BezPath, step: f64) -> Vec<Sub> {
    let mut subs = Vec::new();
    let mut pts: Vec<Point> = Vec::new();
    let mut closed = false;
    let flush = |pts: &mut Vec<Point>, closed: bool, subs: &mut Vec<Sub>| {
        if pts.len() >= 2 {
            subs.push(resample(pts, closed, step));
        }
        pts.clear();
    };
    kurbo::flatten(path.iter(), 0.1, |el| match el {
        PathEl::MoveTo(p) => {
            flush(&mut pts, closed, &mut subs);
            closed = false;
            pts.push(p);
        }
        PathEl::LineTo(p) => {
            if pts.last().is_none_or(|q| (*q - p).hypot() > 1e-9) {
                pts.push(p);
            }
        }
        PathEl::ClosePath => {
            if let (Some(a), Some(b)) = (pts.first().copied(), pts.last().copied())
                && (a - b).hypot() > 1e-9
            {
                pts.push(a);
            }
            closed = true;
            flush(&mut pts, closed, &mut subs);
            closed = false;
        }
        _ => {}
    });
    flush(&mut pts, closed, &mut subs);
    subs
}

fn resample(pts: &[Point], closed: bool, step: f64) -> Sub {
    let mut cum = vec![0.0];
    for w in pts.windows(2) {
        cum.push(cum.last().unwrap() + (w[1] - w[0]).hypot());
    }
    let len = *cum.last().unwrap();
    let n = ((len / step).ceil() as usize).clamp(1, 20_000);
    let mut out = Vec::with_capacity(n + 1);
    let mut j = 0;
    for i in 0..=n {
        let s = len * i as f64 / n as f64;
        while j + 1 < cum.len() - 1 && cum[j + 1] < s {
            j += 1;
        }
        let (a, b) = (pts[j], pts[(j + 1).min(pts.len() - 1)]);
        let seg = (cum[j + 1] - cum[j]).max(1e-9);
        let p = a + (b - a) * ((s - cum[j]) / seg).clamp(0.0, 1.0);
        out.push(Sample { p, t: Vec2::ZERO, s });
    }
    // Tangents by central differences (wrapping for closed paths).
    let m = out.len();
    for i in 0..m {
        let (prev, next) = if closed && m > 2 {
            (out[if i == 0 { m - 2 } else { i - 1 }].p, out[if i == m - 1 { 1 } else { i + 1 }].p)
        } else {
            (out[i.saturating_sub(1)].p, out[(i + 1).min(m - 1)].p)
        };
        let d = next - prev;
        out[i].t = if d.hypot() > 1e-12 { d.normalize() } else { Vec2::new(1.0, 0.0) };
    }
    Sub { pts: out, closed, len }
}

/// The sample at distance `s` (interpolated).
fn at(sub: &Sub, s: f64) -> Sample {
    let s = s.clamp(0.0, sub.len);
    let i = sub.pts.partition_point(|p| p.s < s).clamp(1, sub.pts.len() - 1);
    let (a, b) = (sub.pts[i - 1], sub.pts[i]);
    let k = ((s - a.s) / (b.s - a.s).max(1e-9)).clamp(0.0, 1.0);
    let t = a.t + (b.t - a.t) * k;
    Sample { p: a.p + (b.p - a.p) * k, t: if t.hypot() > 1e-12 { t.normalize() } else { a.t }, s }
}

/// The samples between distances `a` and `b` (with interpolated ends).
fn slice(sub: &Sub, a: f64, b: f64) -> Vec<Sample> {
    let mut out = vec![at(sub, a)];
    out.extend(sub.pts.iter().filter(|p| p.s > a && p.s < b).copied());
    out.push(at(sub, b));
    // Distances relative to the slice start (for ribbons' taper maths).
    for p in &mut out {
        p.s -= a;
    }
    out
}

fn polyline(pts: &[Sample]) -> BezPath {
    let mut p = BezPath::new();
    if let Some(f) = pts.first() {
        p.move_to(f.p);
        for s in &pts[1..] {
            p.line_to(s.p);
        }
    }
    p
}

/// Smooth value noise in -1..1.
fn smooth_noise(seed: u64, channel: u64, x: f64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let h = |k: f64| hash01(seed, channel, k as i64 as u64) * 2.0 - 1.0;
    let s = f * f * (3.0 - 2.0 * f);
    h(i) + (h(i + 1.0) - h(i)) * s
}

fn hash01(seed: u64, i: u64, channel: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ i.wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ channel.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x = x.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^= x >> 32;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Length of `path` (for tests and dash maths).
#[allow(dead_code)]
fn length(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(1e-3)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::Color;

    fn stroke(w: f64, cap: LineCap) -> SceneStroke {
        SceneStroke::solid(Color::WHITE, w, LineJoin::Round, cap)
    }

    fn line() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((100.0, -60.0), (200.0, 60.0), (300.0, 0.0));
        p
    }

    #[test]
    fn plain_strokes_use_the_native_stroker() {
        assert!(marks(&line(), &stroke(10.0, LineCap::Round), &PenStyle::default(), 0.0, 1).is_none());
        // A rounded square cap needs generated geometry.
        let m = marks(&line(), &stroke(10.0, LineCap::Square), &PenStyle::default(), 0.8, 1).unwrap();
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn rounded_caps_go_from_square_to_semicircle() {
        let sq = cap_shape(Point::ZERO, Vec2::new(1.0, 0.0), 10.0, 0.0).bounding_box();
        assert!((sq.x1 - 5.0).abs() < 1e-9 && (sq.y0 + 5.0).abs() < 1e-9);
        let round = cap_shape(Point::ZERO, Vec2::new(1.0, 0.0), 10.0, 1.0);
        // The far corner is cut away: area ≈ half a circle (π r² / 2 ≈ 39.3) vs 50 for square.
        let area = round.area().abs();
        assert!((area - 39.27).abs() < 0.5, "{area}");
    }

    #[test]
    fn every_pen_draws_something_inside_a_sensible_area() {
        for kind in PenKind::ALL.into_iter().filter(|k| *k != PenKind::Solid) {
            let style = PenStyle::preset(kind);
            let m = marks(&line(), &stroke(12.0, LineCap::Butt), &style, 0.0, 42)
                .unwrap_or_else(|| panic!("{kind:?} made nothing"));
            assert!(!m.is_empty(), "{kind:?}");
            let b = m.iter().map(|x| x.path.bounding_box()).reduce(|a, b| a.union(b)).unwrap();
            // Near the line (glow and scatter spill a little further).
            assert!(b.x0 > -60.0 && b.x1 < 360.0 && b.y0 > -110.0 && b.y1 < 110.0, "{kind:?}: {b:?}");
            // Deterministic.
            assert_eq!(m, marks(&line(), &stroke(12.0, LineCap::Butt), &style, 0.0, 42).unwrap(), "{kind:?}");
        }
    }

    #[test]
    fn dashes_repeat_along_the_line() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        let style = PenStyle { dash: 2.0, gap: 1.0, ..PenStyle::preset(PenKind::Dashed) };
        // 10 wide: 20 on, 10 off → dashes at 0, 30, 60, 90.
        let m = marks(&p, &stroke(10.0, LineCap::Butt), &style, 0.0, 0).unwrap();
        assert_eq!(m.len(), 4);
        assert!((length(&line()) - 330.0).abs() < 40.0);
    }
}
