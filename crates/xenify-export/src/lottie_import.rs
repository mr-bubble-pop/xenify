//! Lottie import: a Lottie animation (bodymovin JSON) as ordinary, editable
//! layers and keyframes. Covers what the Lottie export writes and what's
//! common in the wild: shape, null, solid and precomp layers; groups with
//! transforms; paths, rectangles, ellipses and stars; fills, gradient fills,
//! strokes and trim paths; keyframes with bezier easing and holds;
//! parenting; masks and track mattes. Anything else is left out and listed
//! in [`LottieImport::skipped`].

use xenify_doc::{
    Animatable, Color, CompSettings, Composition, Easing, Fill, FillRule, Geometry, GradientKind, GradientPaint,
    GradientStop, Id, Keyframe, Layer, LayerKind, LineCap, LineJoin, Mask, MaskMode, Matte, MatteMode, Property,
    ShapeItem, Stroke, Transform, Trim, Vec2,
};
use kurbo::{BezPath, Point, Shape};
use serde_json::Value as Json;
use std::collections::HashMap;

/// A Lottie file, imported.
#[derive(Debug, Default)]
pub struct LottieImport {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Seconds.
    pub duration: f64,
    /// The animation's layers, bottom-most first.
    pub layers: Vec<Layer>,
    /// Precomps it uses (become symbols of the project).
    pub comps: Vec<Composition>,
    /// What couldn't be brought in (for a summary).
    pub skipped: Vec<String>,
    /// The next free id after everything imported.
    pub next_id: u64,
}

impl LottieImport {
    /// The animation as a composition with id `id` (the precomps it uses
    /// alongside). `id` should be [`LottieImport::next_id`].
    pub fn into_compositions(self, id: Id) -> (Composition, Vec<Composition>) {
        let settings = CompSettings {
            name: self.name,
            width: self.width,
            height: self.height,
            fps: self.fps,
            duration: self.duration,
            background: Color::TRANSPARENT,
            ..Default::default()
        };
        (Composition { id, settings, layers: self.layers }, self.comps)
    }

    /// What was left out, as one line (empty when nothing was).
    pub fn summary(&self) -> String {
        if self.skipped.is_empty() { String::new() } else { format!("Left out: {}.", self.skipped.join(", ")) }
    }
}

struct Ctx<'a> {
    next: u64,
    fps: f64,
    assets: HashMap<String, &'a Json>,
    /// Precomps already made, by asset id.
    made: HashMap<String, Id>,
    comps: Vec<Composition>,
    skipped: Vec<String>,
    depth: usize,
    /// The animation's frame size and length.
    size: (f64, f64),
    duration: f64,
}

impl Ctx<'_> {
    fn id(&mut self) -> Id {
        let id = Id(self.next);
        self.next += 1;
        id
    }

    fn skip(&mut self, what: impl Into<String>) {
        let w = what.into();
        if !self.skipped.contains(&w) {
            self.skipped.push(w);
        }
    }
}

/// Imports Lottie JSON, or a dotLottie (`.lottie`, a zip holding it: the
/// first animation is taken); new ids start at `first_id`.
pub fn import_lottie(data: &[u8], first_id: u64) -> Result<LottieImport, String> {
    if data.starts_with(b"PK") {
        return import_lottie(&dot_lottie(data)?, first_id);
    }
    let json: Json = serde_json::from_slice(data).map_err(|e| format!("not a Lottie file: {e}"))?;
    if json.get("layers").and_then(Json::as_array).is_none() {
        return Err("not a Lottie file (no layers)".into());
    }
    let fps = json["fr"].as_f64().unwrap_or(30.0).max(1.0);
    let assets = json["assets"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| Some((x["id"].as_str()?.to_string(), x))).collect())
        .unwrap_or_default();
    let mut ctx = Ctx {
        next: first_id,
        fps,
        assets,
        made: HashMap::new(),
        comps: Vec::new(),
        skipped: Vec::new(),
        depth: 0,
        size: (json["w"].as_f64().unwrap_or(512.0), json["h"].as_f64().unwrap_or(512.0)),
        duration: 0.0,
    };
    let (ip, op) = (json["ip"].as_f64().unwrap_or(0.0), json["op"].as_f64().unwrap_or(fps * 3.0));
    let duration = ((op - ip.min(0.0)) / fps).max(1.0 / fps);
    ctx.duration = duration;
    let layers = layers(&mut ctx, &json["layers"], duration);
    Ok(LottieImport {
        name: json["nm"].as_str().unwrap_or("Lottie").to_string(),
        width: json["w"].as_f64().unwrap_or(512.0).round().max(1.0) as u32,
        height: json["h"].as_f64().unwrap_or(512.0).round().max(1.0) as u32,
        fps,
        duration,
        layers,
        comps: ctx.comps,
        skipped: ctx.skipped,
        next_id: ctx.next,
    })
}

/// The first animation in a dotLottie file.
fn dot_lottie(data: &[u8]) -> Result<Vec<u8>, String> {
    let zip = crate::model3d::zip::Zip::new(data).map_err(|e| format!("not a dotLottie file: {e}"))?;
    let mut names: Vec<String> =
        zip.names().filter(|n| n.starts_with("animations/") && n.ends_with(".json")).map(str::to_string).collect();
    names.sort();
    let name = names.first().ok_or("a dotLottie file without an animation")?;
    zip.read(name).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Values and keyframes

fn nums(v: &Json) -> Vec<f64> {
    match v {
        Json::Number(n) => vec![n.as_f64().unwrap_or(0.0)],
        Json::Array(a) => a.iter().flat_map(nums).collect(),
        _ => Vec::new(),
    }
}

fn first(v: &Json) -> f64 {
    nums(v).first().copied().unwrap_or(0.0)
}

fn easing_of(key: &Json) -> Easing {
    if key["h"].as_i64() == Some(1) || key["h"].as_bool() == Some(true) {
        return Easing::Hold;
    }
    let (o, i) = (&key["o"], &key["i"]);
    if o.is_null() || i.is_null() {
        return Easing::Linear;
    }
    let (x1, y1, x2, y2) = (first(&o["x"]), first(&o["y"]), first(&i["x"]), first(&i["y"]));
    if (x1, y1, x2, y2) == (0.0, 0.0, 1.0, 1.0) || (x1 == y1 && x2 == y2) {
        Easing::Linear
    } else {
        Easing::Bezier { x1: x1.clamp(0.0, 1.0), y1, x2: x2.clamp(0.0, 1.0), y2 }
    }
}

/// A Lottie property (`{"a": 0|1, "k": …}`) as keyframes of `T`, times in
/// seconds shifted by `shift` (the layer's start).
fn prop<T: Animatable>(v: &Json, fps: f64, shift: f64, conv: &dyn Fn(&Json) -> Option<T>, default: T) -> Property<T> {
    let k = &v["k"];
    let animated = v["a"].as_i64() == Some(1)
        || (k.is_array() && k.as_array().is_some_and(|a| a.first().is_some_and(|x| x.get("t").is_some())));
    if !animated {
        return conv(k).unwrap_or(default).into();
    }
    let keys = k.as_array().cloned().unwrap_or_default();
    let mut out: Vec<Keyframe<T>> = Vec::new();
    for (n, key) in keys.iter().enumerate() {
        // Older files put the end value in "e" of the key before instead of "s".
        let value =
            key.get("s").and_then(conv).or_else(|| n.checked_sub(1).and_then(|p| keys[p].get("e")).and_then(conv));
        let Some(value) = value else { continue };
        let time = key["t"].as_f64().unwrap_or(0.0) / fps + shift;
        if out.last().is_some_and(|l: &Keyframe<T>| time <= l.time) {
            continue;
        }
        out.push(Keyframe { time, value, easing: easing_of(key) });
    }
    match out.len() {
        0 => default.into(),
        1 => out.remove(0).value.into(),
        _ => Property::from_keyframes(out).unwrap_or_else(|| default.into()),
    }
}

fn scalar(v: &Json, fps: f64, shift: f64, k: f64, default: f64) -> Property<f64> {
    prop(v, fps, shift, &|x| nums(x).first().map(|n| n * k), default)
}

fn vec2(v: &Json, fps: f64, shift: f64, k: f64, default: Vec2) -> Property<Vec2> {
    // Separated dimensions: x and y animated on their own (keyed from x's keys).
    if v["s"].as_bool() == Some(true) || v.get("x").is_some() && v.get("k").is_none() {
        let x = scalar(&v["x"], fps, shift, k, default.x);
        let y = scalar(&v["y"], fps, shift, k, default.y);
        return match (&x, &y) {
            (Property::Static(a), Property::Static(b)) => Vec2::new(*a, *b).into(),
            _ => {
                let mut times: Vec<f64> = x.keyframes().iter().chain(y.keyframes()).map(|k| k.time).collect();
                times.sort_by(f64::total_cmp);
                times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                let easing = x.keyframes().first().or(y.keyframes().first()).map_or(Easing::Linear, |k| k.easing);
                let keys = times
                    .iter()
                    .map(|&t| Keyframe { time: t, value: Vec2::new(x.value_at(t), y.value_at(t)), easing })
                    .collect();
                Property::from_keyframes(keys).unwrap_or_else(|| default.into())
            }
        };
    }
    prop(
        v,
        fps,
        shift,
        &|x| {
            let n = nums(x);
            Some(Vec2::new(n.first().copied()? * k, n.get(1).copied().unwrap_or(n[0]) * k))
        },
        default,
    )
}

fn color(v: &Json) -> Option<Color> {
    let n = nums(v);
    if n.len() < 3 {
        return None;
    }
    // 0..255 in some files.
    let s = if n.iter().take(3).any(|c| *c > 1.0) { 255.0 } else { 1.0 };
    Some(Color::rgba(n[0] / s, n[1] / s, n[2] / s, n.get(3).copied().map_or(1.0, |a| a / s).clamp(0.0, 1.0)))
}

/// A colour property with a separate opacity (0..100) folded into its alpha.
fn color_prop(c: &Json, o: &Json, fps: f64, shift: f64) -> Property<Color> {
    let c = prop(c, fps, shift, &color, Color::BLACK);
    let o = scalar(o, fps, shift, 0.01, 1.0);
    match (&c, &o) {
        (_, Property::Static(a)) => {
            let a = *a;
            map_prop(c, |col| col.with_alpha(col.a * a))
        }
        _ => {
            let mut times: Vec<f64> =
                c.keyframes().iter().map(|k| k.time).chain(o.keyframes().iter().map(|k| k.time)).collect();
            times.sort_by(f64::total_cmp);
            times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            let easing = o
                .keyframes()
                .first()
                .map(|k| k.easing)
                .or(c.keyframes().first().map(|k| k.easing))
                .unwrap_or(Easing::Linear);
            let keys = times
                .iter()
                .map(|&t| {
                    let col = c.value_at(t);
                    Keyframe { time: t, value: col.with_alpha(col.a * o.value_at(t)), easing }
                })
                .collect();
            Property::from_keyframes(keys).unwrap_or(c)
        }
    }
}

fn map_prop<T: Animatable>(p: Property<T>, f: impl Fn(T) -> T) -> Property<T> {
    match p {
        Property::Static(v) => Property::Static(f(v)),
        Property::Animated(keys) => {
            Property::Animated(keys.into_iter().map(|k| Keyframe { value: f(k.value), ..k }).collect())
        }
    }
}

/// A Lottie path (`{i, o, v, c}`) as a Bézier path.
fn path(v: &Json) -> Option<BezPath> {
    let v = if v.is_array() { v.get(0)? } else { v };
    let pts = |key: &str| -> Vec<Vec2> {
        v[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| {
                        let n = nums(p);
                        Vec2::new(n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (vs, is, os) = (pts("v"), pts("i"), pts("o"));
    if vs.is_empty() {
        return None;
    }
    let closed = v["c"].as_bool().unwrap_or(false);
    let at = |k: usize| vs[k].to_point();
    let mut out = BezPath::new();
    out.move_to(at(0));
    let seg = |out: &mut BezPath, a: usize, b: usize| {
        let c1 = at(a) + os.get(a).copied().unwrap_or_default();
        let c2 = at(b) + is.get(b).copied().unwrap_or_default();
        if c1 == at(a) && c2 == at(b) {
            out.line_to(at(b));
        } else {
            out.curve_to(c1, c2, at(b));
        }
    };
    for k in 1..vs.len() {
        seg(&mut out, k - 1, k);
    }
    if closed && vs.len() > 1 {
        seg(&mut out, vs.len() - 1, 0);
        out.close_path();
    }
    Some(out)
}

fn transform(ks: &Json, fps: f64, shift: f64) -> (Transform, Property<f64>) {
    let mut t = Transform::at(Vec2::ZERO);
    t.anchor = vec2(&ks["a"], fps, shift, 1.0, Vec2::ZERO);
    t.position = vec2(&ks["p"], fps, shift, 1.0, Vec2::ZERO);
    t.scale = vec2(&ks["s"], fps, shift, 0.01, Vec2::new(1.0, 1.0));
    let r = if ks.get("r").is_some() { &ks["r"] } else { &ks["rz"] };
    t.rotation = scalar(r, fps, shift, 1.0, 0.0);
    // Lottie skews the other way round about a mirrored axis (see the export).
    t.skew = scalar(&ks["sk"], fps, shift, -1.0, 0.0);
    t.skew_axis = scalar(&ks["sa"], fps, shift, -1.0, 0.0);
    let opacity = if ks.get("o").is_some() { scalar(&ks["o"], fps, shift, 0.01, 1.0) } else { 1.0.into() };
    (t, opacity)
}

fn is_identity(t: &Transform, o: &Property<f64>) -> bool {
    let s = |p: &Property<Vec2>, v: Vec2| matches!(p, Property::Static(x) if *x == v);
    let f = |p: &Property<f64>, v: f64| matches!(p, Property::Static(x) if *x == v);
    s(&t.anchor, Vec2::ZERO)
        && s(&t.position, Vec2::ZERO)
        && s(&t.scale, Vec2::new(1.0, 1.0))
        && f(&t.rotation, 0.0)
        && f(&t.skew, 0.0)
        && f(o, 1.0)
}

// ---------------------------------------------------------------------------
// Layers

/// One Lottie layer, made: its Xenify Vector layers and how it takes part in mattes.
struct Made {
    layers: Vec<Layer>,
    /// Uses a matte (`tt`), and which layer it is when named (`tp`, Lottie 5.8+).
    uses: Option<(MatteMode, Option<Id>)>,
    /// Is a matte (`td`).
    is_matte: bool,
}

fn layers(ctx: &mut Ctx, list: &Json, duration: f64) -> Vec<Layer> {
    let Some(list) = list.as_array() else { return Vec::new() };
    // Ids for Lottie's `ind`, so parents can be set whatever the order.
    let ids: HashMap<i64, Id> = list.iter().filter_map(|l| l["ind"].as_i64()).map(|i| (i, ctx.id())).collect();
    let by_ind = |v: &Json| v.as_i64().and_then(|i| ids.get(&i).copied());
    let mut made: Vec<Made> = Vec::new();
    for l in list {
        let id = by_ind(&l["ind"]).unwrap_or_else(|| ctx.id());
        let mode = match l["tt"].as_i64() {
            Some(1) => Some(MatteMode::Alpha),
            Some(2) => Some(MatteMode::AlphaInverted),
            Some(3) => Some(MatteMode::Luma),
            Some(4) => Some(MatteMode::LumaInverted),
            _ => None,
        };
        made.push(Made {
            layers: layer(ctx, l, id, by_ind(&l["parent"]), duration),
            uses: mode.map(|m| (m, by_ind(&l["tp"]))),
            is_matte: l["td"].as_i64().unwrap_or(0) != 0,
        });
    }
    // Mattes: the layer named by `tp`, else the one listed just before (flagged `td`).
    for n in 0..made.len() {
        let Some((mode, named)) = made[n].uses else { continue };
        let source = named.or_else(|| {
            n.checked_sub(1).filter(|&p| made[p].is_matte).and_then(|p| made[p].layers.first()).map(|l| l.id)
        });
        if let Some(source) = source
            && let Some(target) = made[n].layers.first_mut()
        {
            target.meta.matte = Some(Matte { layer: source, mode });
        }
    }
    // Lottie lists the top layer first; Xenify Vector paints bottom first.
    made.into_iter().rev().flat_map(|m| m.layers).collect()
}

/// One Lottie layer as Xenify Vector layers (the layer itself first, then any
/// child layers made for its shape groups).
fn layer(ctx: &mut Ctx, l: &Json, id: Id, parent: Option<Id>, duration: f64) -> Vec<Layer> {
    let fps = ctx.fps;
    let shift = l["st"].as_f64().unwrap_or(0.0) / fps;
    let name = l["nm"].as_str().unwrap_or("Layer").to_string();
    let (transform, opacity) = transform(&l["ks"], fps, shift);
    let ty = l["ty"].as_i64().unwrap_or(-1);
    let mut children = Vec::new();
    let kind = match ty {
        4 => {
            let (items, kids) = shape_contents(
                ctx,
                l["shapes"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                id,
                shift,
                &Paint::default(),
            );
            children = kids;
            if items.is_empty() && !children.is_empty() { LayerKind::Group } else { LayerKind::Shape { shapes: items } }
        }
        // A null moves its children but, unlike a group, doesn't fade them.
        3 => LayerKind::Null,
        1 => {
            let c = l["sc"].as_str().and_then(Color::from_hex).unwrap_or(Color::BLACK);
            let (w, h) = (l["sw"].as_f64().unwrap_or(100.0), l["sh"].as_f64().unwrap_or(100.0));
            let mut rect = ShapeItem::rect(Vec2::new(w, h), c);
            rect.name = "Solid".into();
            if let Geometry::Rect { position, .. } = &mut rect.geometry {
                *position = Vec2::new(w / 2.0, h / 2.0).into();
            }
            LayerKind::Shape { shapes: vec![rect] }
        }
        0 => match l["refId"].as_str().and_then(|r| precomp(ctx, r, l["w"].as_f64(), l["h"].as_f64())) {
            Some(comp) => match time_remap(&l["tm"], fps) {
                None => LayerKind::Precomp { comp, start: shift, looped: false },
                Some(Some(start)) => LayerKind::Precomp { comp, start, looped: true },
                Some(None) => {
                    ctx.skip("time remapping (other than looping)");
                    LayerKind::Precomp { comp, start: shift, looped: false }
                }
            },
            None => {
                ctx.skip("a precomp that couldn't be read");
                LayerKind::Group
            }
        },
        2 => {
            ctx.skip("image layers");
            LayerKind::Group
        }
        5 => {
            ctx.skip("text layers (convert text to shapes before exporting to Lottie)");
            LayerKind::Group
        }
        _ => {
            ctx.skip(format!("layers of type {ty}"));
            LayerKind::Group
        }
    };
    let mut layer = Layer::new(id, name, kind);
    layer.transform = transform;
    layer.opacity = opacity;
    layer.meta.parent = parent;
    layer.meta.visible = !l["hd"].as_bool().unwrap_or(false);
    let (ip, op) = (l["ip"].as_f64().unwrap_or(0.0) / fps, l["op"].as_f64().map(|o| o / fps));
    layer.meta.in_point = ip.max(0.0);
    layer.meta.out_point = op.filter(|o| *o < duration - 1e-9);
    if let Some(masks) = l["masksProperties"].as_array() {
        for m in masks {
            let mode = match m["mode"].as_str().unwrap_or("a") {
                "s" => MaskMode::Subtract,
                "i" => MaskMode::Intersect,
                "f" => MaskMode::Difference,
                "n" => continue,
                _ => MaskMode::Add,
            };
            let p = prop(&m["pt"], fps, shift, &path, BezPath::new());
            let mut mask = Mask::new(m["nm"].as_str().unwrap_or("Mask"), p.value_at(0.0));
            mask.path = p;
            mask.mode = mode;
            mask.inverted = m["inv"].as_bool().unwrap_or(false);
            mask.opacity = scalar(&m["o"], fps, shift, 0.01, 1.0);
            layer.meta.masks.push(mask);
        }
    }
    if l.get("ef").and_then(Json::as_array).is_some_and(|e| !e.is_empty()) {
        ctx.skip("layer effects");
    }
    // Layers made for the shape groups live and hide with the layer.
    for c in &mut children {
        c.meta.in_point = layer.meta.in_point;
        c.meta.out_point = layer.meta.out_point;
        c.meta.visible = layer.meta.visible;
    }
    let mut out = vec![layer];
    out.extend(children);
    out
}

/// A precomp's time remapping: `None` without one, `Some(Some(start))` for a
/// loop (0 → end, again and again, as the Lottie export writes it), and
/// `Some(None)` for anything else.
fn time_remap(tm: &Json, fps: f64) -> Option<Option<f64>> {
    let keys = tm.get("k")?.as_array()?;
    let starts: Vec<&Json> = keys.iter().step_by(2).collect();
    let looped = tm["a"].as_i64() == Some(1)
        && keys.len() >= 2
        && starts.iter().all(|k| first(&k["s"]) == 0.0)
        && keys.iter().skip(1).step_by(2).all(|k| first(&k["s"]) > 0.0);
    Some(looped.then(|| keys[0]["t"].as_f64().unwrap_or(0.0) / fps))
}

fn precomp(ctx: &mut Ctx, ref_id: &str, w: Option<f64>, h: Option<f64>) -> Option<Id> {
    if let Some(id) = ctx.made.get(ref_id) {
        return Some(*id);
    }
    if ctx.depth > 8 {
        return None;
    }
    let asset = *ctx.assets.get(ref_id)?;
    asset.get("layers")?;
    let id = ctx.id();
    ctx.made.insert(ref_id.to_string(), id);
    ctx.depth += 1;
    let fps = ctx.fps;
    // The precomp layer's size clips it; the asset's is a fallback.
    let w = w.or(asset["w"].as_f64()).unwrap_or(ctx.size.0).max(1.0) as u32;
    let h = h.or(asset["h"].as_f64()).unwrap_or(ctx.size.1).max(1.0) as u32;
    let duration = 3600.0;
    let layers = layers(ctx, &asset["layers"], duration);
    ctx.depth -= 1;
    // Lottie doesn't store a precomp's length: its layers' latest end.
    let end =
        asset["layers"].as_array().into_iter().flatten().filter_map(|l| l["op"].as_f64()).fold(0.0, f64::max) / fps;
    let settings = CompSettings {
        name: asset["nm"].as_str().unwrap_or(ref_id).to_string(),
        width: w,
        height: h,
        fps,
        duration: if end > 0.0 { end } else { ctx.duration },
        background: Color::TRANSPARENT,
        ..Default::default()
    };
    ctx.comps.push(Composition { id, settings, layers });
    Some(id)
}

// ---------------------------------------------------------------------------
// Shapes

/// Fill, stroke and trim that apply to a group's shapes.
#[derive(Clone, Default)]
struct Paint {
    fill: Option<Fill>,
    rule: FillRule,
    stroke: Option<Stroke>,
    trim: Option<Trim>,
}

/// A group's contents: shape items for this layer (bottom first), and child
/// layers for groups that move on their own. `owner` is the layer they go on.
fn shape_contents(
    ctx: &mut Ctx,
    items: &[Json],
    owner: Id,
    shift: f64,
    inherited: &Paint,
) -> (Vec<ShapeItem>, Vec<Layer>) {
    let fps = ctx.fps;
    let mut paint = inherited.clone();
    let mut own_paint = Paint::default();
    // Paint and trim listed in this group apply to all of its shapes.
    for it in items {
        match it["ty"].as_str().unwrap_or("") {
            "fl" => {
                own_paint.fill =
                    Some(Fill { color: color_prop(&it["c"], &it["o"], fps, shift), gradient: None, pattern: None });
                own_paint.rule = if it["r"].as_i64() == Some(2) { FillRule::EvenOdd } else { FillRule::NonZero };
            }
            "gf" => {
                let g = gradient(it, fps, shift);
                let first = match &g.stops {
                    Property::Static(s) => s.first().map(|s| s.color),
                    Property::Animated(k) => k.first().and_then(|k| k.value.first()).map(|s| s.color),
                }
                .unwrap_or(Color::BLACK);
                own_paint.fill = Some(Fill { color: first.into(), gradient: Some(g), pattern: None });
                own_paint.rule = if it["r"].as_i64() == Some(2) { FillRule::EvenOdd } else { FillRule::NonZero };
            }
            "st" | "gs" => {
                let mut s = Stroke {
                    color: if it["ty"] == "st" {
                        color_prop(&it["c"], &it["o"], fps, shift)
                    } else {
                        Color::BLACK.into()
                    },
                    width: scalar(&it["w"], fps, shift, 1.0, 1.0),
                    ..Default::default()
                };
                if it["ty"] == "gs" {
                    s.gradient = Some(gradient(it, fps, shift));
                }
                s.cap = match it["lc"].as_i64() {
                    Some(2) => LineCap::Round,
                    Some(3) => LineCap::Square,
                    _ => LineCap::Butt,
                };
                s.join = match it["lj"].as_i64() {
                    Some(2) => LineJoin::Round,
                    Some(3) => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                };
                if let Some(d) = it["d"].as_array() {
                    s.dashes = d.iter().filter(|x| x["n"] != "o").map(|x| first(&x["v"]["k"])).collect();
                    if let Some(o) = d.iter().find(|x| x["n"] == "o") {
                        s.dash_offset = scalar(&o["v"], fps, shift, 1.0, 0.0);
                    }
                }
                own_paint.stroke = Some(s);
            }
            "tm" => {
                own_paint.trim = Some(Trim {
                    start: scalar(&it["s"], fps, shift, 0.01, 0.0),
                    end: scalar(&it["e"], fps, shift, 0.01, 1.0),
                    offset: scalar(&it["o"], fps, shift, 1.0 / 360.0, 0.0),
                });
            }
            _ => {}
        }
    }
    if own_paint.fill.is_some() {
        paint.fill = own_paint.fill;
        paint.rule = own_paint.rule;
    }
    if own_paint.stroke.is_some() {
        paint.stroke = own_paint.stroke;
    }
    if own_paint.trim.is_some() {
        paint.trim = own_paint.trim;
    }

    let mut shapes: Vec<ShapeItem> = Vec::new();
    let mut kids: Vec<Layer> = Vec::new();
    // Geometry of this group, gathered into one item (holes and all).
    let mut geometry: Vec<Geometry> = Vec::new();
    for it in items {
        match it["ty"].as_str().unwrap_or("") {
            "sh" => geometry.push(Geometry::Path { path: prop(&it["ks"], fps, shift, &path, BezPath::new()) }),
            "rc" => {
                let size = vec2(&it["s"], fps, shift, 1.0, Vec2::new(100.0, 100.0));
                geometry.push(Geometry::Rect {
                    position: vec2(&it["p"], fps, shift, 1.0, Vec2::ZERO),
                    size,
                    corner_radius: scalar(&it["r"], fps, shift, 1.0, 0.0),
                });
            }
            "el" => geometry.push(Geometry::Ellipse {
                position: vec2(&it["p"], fps, shift, 1.0, Vec2::ZERO),
                size: vec2(&it["s"], fps, shift, 1.0, Vec2::new(100.0, 100.0)),
            }),
            "sr" => geometry.push(Geometry::Path { path: star(it).into() }),
            "gr" => {
                let inner = it["it"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                let tr = inner.iter().find(|x| x["ty"] == "tr").cloned().unwrap_or(Json::Null);
                let (t, o) = transform(&tr, fps, shift);
                let baked = if is_identity(&t, &o) {
                    let (s, k) = shape_contents(ctx, inner, owner, shift, &paint);
                    Some((s, k))
                } else if let Some((a, op)) = static_transform(&t, &o) {
                    // A group that stays put: its transform goes into the shapes.
                    let (mut s, k) = shape_contents(ctx, inner, owner, shift, &paint);
                    (k.is_empty() && s.iter_mut().all(|i| bake(i, a, op))).then_some((s, k))
                } else {
                    None
                };
                if let Some((s, k)) = baked {
                    // Lottie lists the top first: keep that relative order for now (reversed at the end).
                    shapes.extend(s.into_iter().rev());
                    kids.extend(k);
                } else {
                    // A group that moves on its own: a child layer.
                    let id = ctx.id();
                    let (s, k) = shape_contents(ctx, inner, id, shift, &paint);
                    let name = it["nm"].as_str().unwrap_or("Group").to_string();
                    let mut child = Layer::new(
                        id,
                        name,
                        if s.is_empty() { LayerKind::Group } else { LayerKind::Shape { shapes: s } },
                    );
                    child.transform = t;
                    child.opacity = o;
                    child.meta.parent = Some(owner);
                    kids.push(child);
                    kids.extend(k);
                }
            }
            "rp" => ctx.skip("repeaters (use an Array modifier)"),
            "mm" | "rd" | "pb" | "tw" | "zz" | "op" => ctx.skip("shape modifiers other than trim"),
            _ => {}
        }
    }
    if !geometry.is_empty() && (paint.fill.is_some() || paint.stroke.is_some()) {
        let geometry = merge(geometry);
        let name = items.iter().find_map(|x| x["nm"].as_str()).unwrap_or("Shape").to_string();
        shapes.push(ShapeItem {
            name,
            geometry,
            fill_rule: paint.rule,
            fill: paint.fill.clone(),
            stroke: paint.stroke.clone(),
            trim: paint.trim.clone(),
        });
    }
    // Back to bottom-first.
    shapes.reverse();
    (shapes, kids)
}

/// The transform and opacity of a group that doesn't move.
fn static_transform(t: &Transform, o: &Property<f64>) -> Option<(kurbo::Affine, f64)> {
    let s2 = |p: &Property<Vec2>| matches!(p, Property::Static(_));
    let s1 = |p: &Property<f64>| matches!(p, Property::Static(_));
    let still = s2(&t.anchor) && s2(&t.position) && s2(&t.scale) && s1(&t.rotation) && s1(&t.skew) && s1(&t.skew_axis);
    match o {
        Property::Static(op) if still => Some((t.affine_at(0.0), *op)),
        _ => None,
    }
}

/// Applies `a` and `opacity` to an item's geometry and paint; false when it
/// can't be (a rectangle or ellipse whose size animates, turned or skewed).
fn bake(item: &mut ShapeItem, a: kurbo::Affine, opacity: f64) -> bool {
    let c = a.as_coeffs();
    let shift_only =
        (c[0] - 1.0).abs() < 1e-12 && c[1].abs() < 1e-12 && c[2].abs() < 1e-12 && (c[3] - 1.0).abs() < 1e-12;
    let t = Vec2::new(c[4], c[5]);
    match &mut item.geometry {
        Geometry::Rect { position, .. } | Geometry::Ellipse { position, .. } if shift_only => {
            *position = map_prop(position.clone(), |p| p + t);
        }
        Geometry::Path { path } => *path = map_prop(path.clone(), |p| a * p),
        g => {
            let still = match g {
                Geometry::Rect { position, size, corner_radius } => {
                    matches!(position, Property::Static(_))
                        && matches!(size, Property::Static(_))
                        && matches!(corner_radius, Property::Static(_))
                }
                Geometry::Ellipse { position, size } => {
                    matches!(position, Property::Static(_)) && matches!(size, Property::Static(_))
                }
                _ => false,
            };
            if !still {
                return false;
            }
            *g = Geometry::Path { path: (a * xenify_anim::geometry_path(g, 0.0)).into() };
        }
    }
    let at = |v: Vec2| {
        let p = a * v.to_point();
        Vec2::new(p.x, p.y)
    };
    let fade = |col: Color| col.with_alpha(col.a * opacity);
    if let Some(f) = &mut item.fill {
        f.color = map_prop(f.color.clone(), fade);
        if let Some(g) = &mut f.gradient {
            bake_gradient(g, at, opacity);
        }
    }
    if let Some(s) = &mut item.stroke {
        let k = a.determinant().abs().sqrt();
        s.width = map_prop(s.width.clone(), |w| w * k);
        s.color = map_prop(s.color.clone(), fade);
        if let Some(g) = &mut s.gradient {
            bake_gradient(g, at, opacity);
        }
    }
    true
}

fn bake_gradient(g: &mut GradientPaint, at: impl Fn(Vec2) -> Vec2, opacity: f64) {
    g.start = map_prop(g.start.clone(), &at);
    g.end = map_prop(g.end.clone(), &at);
    if opacity < 1.0 {
        g.stops = map_prop(g.stops.clone(), |stops| {
            stops.into_iter().map(|s| GradientStop { color: s.color.with_alpha(s.color.a * opacity), ..s }).collect()
        });
    }
}

/// One geometry for several: a lone one as it is, several as one path.
fn merge(mut g: Vec<Geometry>) -> Geometry {
    if g.len() == 1 {
        return g.remove(0);
    }
    let mut out = BezPath::new();
    for geo in &g {
        out.extend(xenify_anim::geometry_path(geo, 0.0));
    }
    Geometry::Path { path: out.into() }
}

fn gradient(it: &Json, fps: f64, shift: f64) -> GradientPaint {
    let count = it["g"]["p"].as_u64().unwrap_or(2) as usize;
    let stops = move |v: &Json| -> Option<Vec<GradientStop>> {
        let n = nums(v);
        if n.len() < count * 4 {
            return None;
        }
        let alphas = &n[count * 4..];
        Some(
            (0..count)
                .map(|i| {
                    let a = if alphas.len() >= count * 2 { alphas[i * 2 + 1] } else { 1.0 };
                    GradientStop { offset: n[i * 4], color: Color::rgba(n[i * 4 + 1], n[i * 4 + 2], n[i * 4 + 3], a) }
                })
                .collect(),
        )
    };
    GradientPaint {
        kind: if it["t"].as_i64() == Some(2) { GradientKind::Radial } else { GradientKind::Linear },
        start: vec2(&it["s"], fps, shift, 1.0, Vec2::ZERO),
        end: vec2(&it["e"], fps, shift, 1.0, Vec2::new(100.0, 0.0)),
        stops: prop(&it["g"]["k"], fps, shift, &stops, vec![]),
    }
}

/// A Lottie star or polygon (as it is at the start).
fn star(it: &Json) -> BezPath {
    let n = first(&it["pt"]["k"]).round().max(3.0) as usize;
    let outer = first(&it["or"]["k"]);
    let inner = if it["sy"].as_i64() == Some(2) { None } else { Some(first(&it["ir"]["k"])) };
    let c = nums(&it["p"]["k"]);
    let c = Point::new(c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0));
    let rot = first(&it["r"]["k"]).to_radians();
    let mut out = BezPath::new();
    let steps = if inner.is_some() { n * 2 } else { n };
    for k in 0..steps {
        let a = rot - std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::TAU / steps as f64;
        let r = match inner {
            Some(ir) if k % 2 == 1 => ir,
            _ => outer,
        };
        let p = c + Vec2::new(a.cos(), a.sin()) * r;
        if k == 0 { out.move_to(p) } else { out.line_to(p) }
    }
    out.close_path();
    let _ = out.area();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_that_stay_put_are_baked_into_the_shapes() {
        // A matte source whose shape sits in a moved, scaled group: it must
        // stay one layer (a matte is drawn from its own layer only).
        let json = serde_json::json!({
            "fr": 30, "ip": 0, "op": 30, "w": 100, "h": 100,
            "layers": [
                {"ty": 4, "ind": 1, "td": 1, "nm": "Matte", "ks": {}, "shapes": [{"ty": "gr", "it": [
                    {"ty": "rc", "p": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [10, 10]}, "r": {"a": 0, "k": 0}},
                    {"ty": "fl", "c": {"a": 0, "k": [1, 1, 1, 1]}, "o": {"a": 0, "k": 100}},
                    {"ty": "tr", "p": {"a": 0, "k": [50, 50]}, "s": {"a": 0, "k": [200, 200]}, "r": {"a": 0, "k": 45}, "o": {"a": 0, "k": 50}}
                ]}]},
                {"ty": 4, "ind": 2, "tt": 1, "nm": "Art", "ks": {}, "shapes": [
                    {"ty": "rc", "p": {"a": 0, "k": [50, 50]}, "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}},
                    {"ty": "fl", "c": {"a": 0, "k": [1, 0, 0, 1]}, "o": {"a": 0, "k": 100}}
                ]}
            ]
        });
        let imp = import_lottie(json.to_string().as_bytes(), 1).unwrap();
        assert_eq!(imp.layers.len(), 2, "{:#?}", imp.layers);
        let matte = imp.layers.iter().find(|l| l.meta.name == "Matte").unwrap();
        let art = imp.layers.iter().find(|l| l.meta.name == "Art").unwrap();
        assert_eq!(art.meta.matte.map(|m| m.layer), Some(matte.id));
        let item = &matte.shapes()[0];
        let Geometry::Path { path } = &item.geometry else { panic!("turned: a path") };
        let bb = path.value_at(0.0).bounding_box();
        // 20×20 turned 45° about (50, 50): about 28 across.
        assert!((bb.width() - 20.0 * 2f64.sqrt()).abs() < 0.01 && (bb.center().x - 50.0).abs() < 0.01, "{bb:?}");
        assert!((item.fill.as_ref().unwrap().color.value_at(0.0).a - 0.5).abs() < 1e-9, "group opacity folded in");
    }

    #[test]
    fn dot_lottie_files_are_unzipped() {
        let json = br#"{"fr":30,"ip":0,"op":30,"w":10,"h":10,"layers":[]}"#;
        let zip = crate::model3d::zip::tests::make_zip(&[("manifest.json", b"{}"), ("animations/a.json", json)]);
        assert_eq!(import_lottie(&zip, 1).unwrap().width, 10);
    }

    #[test]
    fn keyframes_easing_and_holds_come_across() {
        let v: Json = serde_json::json!({"a": 1, "k": [
            {"t": 0, "s": [0], "o": {"x": [0.4], "y": [0]}, "i": {"x": [0.2], "y": [1]}},
            {"t": 30, "s": [100], "h": 1},
            {"t": 60, "s": [50]}
        ]});
        let p = scalar(&v, 30.0, 0.0, 1.0, 0.0);
        let keys = p.keyframes();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[1].time, 1.0);
        assert_eq!(keys[0].easing, Easing::Bezier { x1: 0.4, y1: 0.0, x2: 0.2, y2: 1.0 });
        assert_eq!(keys[1].easing, Easing::Hold);
        assert_eq!(p.value_at(1.5), 100.0, "held");
    }

    #[test]
    fn a_minimal_file_imports() {
        let json = serde_json::json!({
            "v": "5.7", "fr": 30, "ip": 0, "op": 60, "w": 200, "h": 100, "nm": "Test",
            "layers": [{
                "ty": 4, "ind": 1, "nm": "Ball", "ip": 0, "op": 60, "st": 0,
                "ks": {"p": {"a": 1, "k": [{"t": 0, "s": [0, 50]}, {"t": 60, "s": [200, 50]}]}, "s": {"a": 0, "k": [100, 100]}},
                "shapes": [{"ty": "gr", "it": [
                    {"ty": "el", "p": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [20, 20]}},
                    {"ty": "fl", "c": {"a": 0, "k": [1, 0, 0, 1]}, "o": {"a": 0, "k": 100}},
                    {"ty": "tr", "p": {"a": 0, "k": [0, 0]}, "a": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}}
                ]}]
            }, {"ty": 5, "ind": 2, "nm": "Title", "ks": {}}]
        });
        let imp = import_lottie(json.to_string().as_bytes(), 10).unwrap();
        assert_eq!((imp.width, imp.height, imp.fps), (200, 100, 30.0));
        assert!((imp.duration - 2.0).abs() < 1e-9);
        let ball = imp.layers.iter().find(|l| l.meta.name == "Ball").unwrap();
        assert!(
            matches!(&ball.kind, LayerKind::Shape { shapes } if matches!(shapes[0].geometry, Geometry::Ellipse { .. }))
        );
        assert_eq!(ball.transform.position.value_at(2.0), Vec2::new(200.0, 50.0));
        assert!(imp.skipped.iter().any(|s| s.contains("text")));
    }
}
