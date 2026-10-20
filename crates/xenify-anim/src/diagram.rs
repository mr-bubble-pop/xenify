//! Diagram parts at evaluation time: arrowheads on open lines, labels inside
//! shapes and along connectors, and connectors re-routed between the shapes
//! they're glued to (with hops where they cross), using `xenify-route`.

use crate::{ApproxText, DrawItem, SceneContent, SceneLayer, SceneStroke, SceneText, stroke_style};
use xenify_doc::{
    Composition, ConnectorEnd, FlowKind, Id, Layer, LayerKind, LineCap, LineJoin, Marker, MarkerKind, NodeLabel,
    PortRef, PortSide, RouteStyle, ShapeItem, Stroke,
};
use xenify_route as route;
use kurbo::{Affine, BezPath, ParamCurve, ParamCurveArclen, PathEl, PathSeg, Point, Rect, Shape, Vec2};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Arrowheads

/// Adds the arrowheads of `shapes`' strokes to the drawn `items` (item `i`
/// is shape `i`), shortening each line so it stops under its arrowhead.
/// Closed outlines get none.
pub fn add_markers(items: &mut Vec<DrawItem>, shapes: &[ShapeItem], seed: u64) {
    let mut extra = Vec::new();
    for (i, shape) in shapes.iter().enumerate() {
        let Some(stroke) = &shape.stroke else { continue };
        if stroke.start_marker.is_none() && stroke.end_marker.is_none() {
            continue;
        }
        let Some(item) = items.get_mut(i) else { continue };
        extra.extend(markers_for(item, stroke, seed.wrapping_add(i as u64)));
    }
    items.extend(extra);
}

/// The marker shapes for one drawn line (and shortens its path).
fn markers_for(item: &mut DrawItem, stroke: &Stroke, seed: u64) -> Vec<DrawItem> {
    let Some(st) = item.stroke.clone() else { return Vec::new() };
    let Some(((start, sd), (end, ed))) = open_ends(&item.path) else { return Vec::new() };
    let w = st.width.max(0.5);
    let mut out = Vec::new();
    let mut cut = (0.0, 0.0);
    for (marker, tip, dir, at_start) in [(stroke.start_marker, start, sd, true), (stroke.end_marker, end, ed, false)] {
        if marker.is_none() {
            continue;
        }
        let (shapes, shorten) = marker_shape(marker, tip, dir, w);
        for (path, filled) in shapes {
            out.push(DrawItem {
                path,
                fill_rule: xenify_doc::FillRule::NonZero,
                gradient: None,
                fill: filled.then_some(st.color),
                stroke: (!filled).then(|| SceneStroke::solid(st.color, w, LineJoin::Round, LineCap::Round)),
                pattern: None,
            });
        }
        if at_start {
            cut.0 = shorten;
        } else {
            cut.1 = shorten;
        }
    }
    if cut != (0.0, 0.0) {
        item.path = shorten(&item.path, cut.0, cut.1);
        if let Some(s) = &mut item.stroke
            && s.marks.is_some()
        {
            s.marks = stroke_style::marks(&item.path, s, &stroke.style, stroke.cap_roundness, seed);
        }
    }
    out
}

/// The standard arrowhead length for a line `w` wide.
pub fn marker_length(m: Marker, w: f64) -> f64 {
    m.size.max(0.1) * (3.0 * w + 6.0)
}

/// The shapes of `marker` with its tip at `tip`, pointing along `d` (unit,
/// out of the line), as (path, filled) pairs, and how far to pull the line
/// back under it.
fn marker_shape(marker: Marker, tip: Point, d: Vec2, w: f64) -> (Vec<(BezPath, bool)>, f64) {
    let l = marker_length(marker, w);
    let half = l * 0.4;
    let n = Vec2::new(-d.y, d.x);
    let back = tip - d * l;
    let poly = |pts: &[Point], close: bool| {
        let mut p = BezPath::new();
        p.move_to(pts[0]);
        for q in &pts[1..] {
            p.line_to(*q);
        }
        if close {
            p.close_path();
        }
        p
    };
    match marker.kind {
        MarkerKind::None => (vec![], 0.0),
        MarkerKind::Triangle => (vec![(poly(&[tip, back + n * half, back - n * half], true), true)], l * 0.8),
        MarkerKind::Arrow => {
            (vec![(poly(&[tip, back + n * half, tip - d * (l * 0.7), back - n * half], true), true)], l * 0.6)
        }
        MarkerKind::Open => (vec![(poly(&[back + n * half, tip, back - n * half], false), false)], w * 0.5),
        MarkerKind::Diamond | MarkerKind::OpenDiamond => {
            let mid = tip - d * (l / 2.0);
            let shape = poly(&[tip, mid + n * half, back, mid - n * half], true);
            (vec![(shape, marker.kind == MarkerKind::Diamond)], l)
        }
        MarkerKind::Circle | MarkerKind::OpenCircle => {
            let r = l * 0.35;
            let c = kurbo::Circle::new(tip - d * r, r).to_path(0.1);
            (vec![(c, marker.kind == MarkerKind::Circle)], 2.0 * r)
        }
        MarkerKind::Bar => {
            let at = tip - d * (l * 0.3);
            (vec![(poly(&[at + n * half, at - n * half], false), false)], 0.0)
        }
        MarkerKind::CrowsFoot => {
            let root = tip - d * l;
            let a = poly(&[root, tip + n * half], false);
            let b = poly(&[root, tip - n * half], false);
            (vec![(a, false), (b, false)], 0.0)
        }
    }
}

fn seg_el(seg: PathSeg) -> PathEl {
    match seg {
        PathSeg::Line(l) => PathEl::LineTo(l.p1),
        PathSeg::Quad(q) => PathEl::QuadTo(q.p1, q.p2),
        PathSeg::Cubic(c) => PathEl::CurveTo(c.p1, c.p2, c.p3),
    }
}

fn el_end(el: PathEl) -> Option<Point> {
    match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(p),
        PathEl::ClosePath => None,
    }
}

fn el_seg(from: Point, el: PathEl) -> Option<PathSeg> {
    Some(match el {
        PathEl::LineTo(p) => PathSeg::Line(kurbo::Line::new(from, p)),
        PathEl::QuadTo(a, p) => PathSeg::Quad(kurbo::QuadBez::new(from, a, p)),
        PathEl::CurveTo(a, b, p) => PathSeg::Cubic(kurbo::CubicBez::new(from, a, b, p)),
        _ => return None,
    })
}

/// The start and end of an open path with the directions pointing out of
/// the line there (`None` for closed or empty paths).
fn open_ends(path: &BezPath) -> Option<((Point, Vec2), (Point, Vec2))> {
    if path.elements().iter().any(|e| matches!(e, PathEl::ClosePath)) {
        return None;
    }
    let first = path.segments().next()?;
    let last = path.segments().last()?;
    let away =
        |p: Point, others: &[Point]| others.iter().map(|q| p - *q).find(|v| v.hypot() > 1e-9).map(|v| v.normalize());
    let pts = |s: PathSeg| match s {
        PathSeg::Line(l) => vec![l.p0, l.p1],
        PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
        PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    };
    let f = pts(first);
    let l: Vec<Point> = pts(last).into_iter().rev().collect();
    Some(((f[0], away(f[0], &f[1..])?), (l[0], away(l[0], &l[1..])?)))
}

/// `path` with `a` cut off its first segment's start and `b` off its last
/// segment's end (segments too short to cut are left alone).
fn shorten(path: &BezPath, a: f64, b: f64) -> BezPath {
    let mut els: Vec<PathEl> = path.elements().to_vec();
    let drawing = |e: &PathEl| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath);
    if a > 0.0
        && let Some(k) = els.iter().position(drawing)
        && k > 0
        && let Some(from) = el_end(els[k - 1])
        && let Some(seg) = el_seg(from, els[k])
        && seg.arclen(0.01) > a * 1.2
    {
        let t = seg.inv_arclen(a, 0.01);
        let sub = seg.subsegment(t..1.0);
        els[k - 1] = PathEl::MoveTo(sub.start());
        els[k] = seg_el(sub);
    }
    if b > 0.0
        && let Some(k) = els.iter().rposition(drawing)
        && k > 0
        && let Some(from) = el_end(els[k - 1])
        && let Some(seg) = el_seg(from, els[k])
    {
        let len = seg.arclen(0.01);
        if len > b * 1.2 {
            let t = seg.inv_arclen(len - b, 0.01);
            els[k] = seg_el(seg.subsegment(0.0..t));
        }
    }
    BezPath::from_vec(els)
}

// ---------------------------------------------------------------------------
// Labels

fn text_of(label: &NodeLabel) -> SceneText {
    let mut t = SceneText::new(label.text.clone(), label.font_family.clone(), label.font_size.max(1.0), label.color);
    t.align = label.align;
    t.valign = xenify_doc::TextVAlign::Middle;
    t
}

fn text_layer(id: Id, transform: Affine, text: SceneText) -> SceneLayer {
    SceneLayer {
        id,
        transform,
        opacity: 1.0,
        content: SceneContent::Text(text),
        matte: None,
        distort: None,
        effects: Vec::new(),
        blend: Default::default(),
        masks: Vec::new(),
        copy: 0,
    }
}

/// Draws `text` over `shape`: both become one group, which takes over the
/// shape's opacity, effects, blend mode, masks and matte.
fn with_text(mut shape: SceneLayer, text: SceneLayer) -> SceneLayer {
    let outer = SceneLayer {
        id: shape.id,
        transform: Affine::IDENTITY,
        opacity: shape.opacity,
        content: SceneContent::Group(Vec::new()),
        matte: shape.matte.take(),
        distort: None,
        effects: std::mem::take(&mut shape.effects),
        blend: std::mem::take(&mut shape.blend),
        masks: std::mem::take(&mut shape.masks),
        copy: shape.copy,
    };
    shape.opacity = 1.0;
    SceneLayer { content: SceneContent::Group(vec![shape, text]), ..outer }
}

/// A shape with its label wrapped inside its box.
pub(crate) fn label_node(shape: SceneLayer, label: &NodeLabel) -> SceneLayer {
    if label.text.is_empty() {
        return shape;
    }
    let Some(b) = shape.local_bounds() else { return shape };
    let mut t = text_of(label);
    let frame = if label.below {
        // A box under the shape, at least a few words wide.
        let w = b.width().max(label.font_size * 9.0);
        let top = b.y1 + label.inset.max(0.0);
        t.valign = xenify_doc::TextVAlign::Top;
        Rect::new(b.center().x - w / 2.0, top, b.center().x + w / 2.0, top + label.font_size * 2.6)
    } else {
        let inset = label.inset.max(0.0).min(b.width() / 2.0).min(b.height() / 2.0);
        b.inset(-inset)
    };
    t.frame = Some(xenify_doc::TextFrame::new(frame));
    let text = text_layer(shape.id, shape.transform, t);
    with_text(shape, text)
}

/// The point `f` (0..1) of the way along a polyline, and the direction there.
fn along(poly: &[Point], f: f64) -> (Point, Vec2) {
    let total: f64 = poly.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total * f.clamp(0.0, 1.0);
    let mut last = (poly.first().copied().unwrap_or_default(), Vec2::new(1.0, 0.0));
    for w in poly.windows(2) {
        let l = w[0].distance(w[1]);
        if l < 1e-9 {
            continue;
        }
        let d = (w[1] - w[0]) / l;
        if left <= l {
            return (w[0] + d * left, d);
        }
        left -= l;
        last = (w[1], d);
    }
    last
}

// ---------------------------------------------------------------------------
// Connectors

/// A glue target: its transform and box in its own space.
#[derive(Clone, Copy)]
struct Node {
    transform: Affine,
    local: Rect,
}

impl Node {
    fn world(&self) -> Rect {
        self.transform.transform_rect_bbox(self.local)
    }
}

/// A drawn layer's own transform and box in its own space (what ports are
/// fractions of). A labelled shape is measured by its shape.
pub fn frame_of(l: &SceneLayer) -> Option<(Affine, Rect)> {
    if let SceneContent::Group(inner) = &l.content
        && let [shape, text] = inner.as_slice()
        && shape.id == l.id
        && text.id == l.id
        && matches!(text.content, SceneContent::Text(_))
    {
        return frame_of(shape);
    }
    let b = l.bounds(&mut ApproxText).filter(|b| usable(*b))?;
    Some((l.transform, b))
}

/// The outline of shape layer `id` as drawn (composition space): its first
/// item's path, or `None` for other layers.
fn outline_of(layers: &[SceneLayer], id: Id) -> Option<BezPath> {
    let l = layers.iter().find(|l| l.id == id && l.copy == 0)?;
    let shape = match &l.content {
        SceneContent::Group(inner) => {
            inner.iter().find(|m| m.id == id && matches!(m.content, SceneContent::Shapes(_)))?
        }
        _ => l,
    };
    let SceneContent::Shapes(items) = &shape.content else { return None };
    let item = items.first()?;
    (!item.path.elements().is_empty()).then(|| shape.transform * &item.path)
}

/// Where the line from `centre` towards `toward` last crosses `outline`
/// (the point it leaves the shape), if it does.
fn clip_to_outline(outline: &BezPath, centre: Point, toward: Point) -> Option<Point> {
    let d = toward - centre;
    if d.hypot2() < 1e-12 {
        return None;
    }
    let mut best: Option<f64> = None;
    let mut last = None;
    let mut start = None;
    let mut hit = |a: Point, b: Point| {
        // Segment a–b against the ray centre + t·d, t in 0..=1.
        let e = b - a;
        let den = d.x * e.y - d.y * e.x;
        if den.abs() < 1e-12 {
            return;
        }
        let w = a - centre;
        let t = (w.x * e.y - w.y * e.x) / den;
        let u = (w.x * d.y - w.y * d.x) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) && best.is_none_or(|b| t > b) {
            best = Some(t);
        }
    };
    kurbo::flatten(outline, 0.25, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            last = Some(p);
            start = Some(p);
        }
        kurbo::PathEl::LineTo(p) => {
            if let Some(a) = last {
                hit(a, p);
            }
            last = Some(p);
        }
        kurbo::PathEl::ClosePath => {
            if let (Some(a), Some(s)) = (last, start) {
                hit(a, s);
            }
            last = start;
        }
        _ => {}
    });
    best.map(|t| centre + d * t)
}

/// Where layer `id`'s ports are in the scene (composition space), with the
/// side each faces, as connectors would glue to them. Groups have the
/// default ports of the box around their members.
pub fn port_points(comp: &Composition, scene: &crate::Scene, id: Id) -> Vec<(Point, PortSide)> {
    let Some(layer) = comp.layer(id) else { return Vec::new() };
    let frame = match scene.layer(id).and_then(frame_of) {
        Some(f) => f,
        None => match crate::world_box(comp, scene, id, &mut ApproxText) {
            Some(b) => (Affine::IDENTITY, b),
            None => return Vec::new(),
        },
    };
    let (t, r) = frame;
    layer
        .meta
        .ports()
        .iter()
        .map(|p| (t * Point::new(r.x0 + p.pos.x * r.width(), r.y0 + p.pos.y * r.height()), p.side))
        .collect()
}

fn usable(r: Rect) -> bool {
    r.is_finite() && r.width() >= 0.0 && r.height() >= 0.0
}

fn ancestors(comp: &Composition, id: Id) -> Vec<Id> {
    let mut out = Vec::new();
    let mut p = comp.layer(id).and_then(|l| l.meta.parent);
    while let Some(pp) = p {
        if out.contains(&pp) || out.len() > comp.layers.len() {
            break;
        }
        out.push(pp);
        p = comp.layer(pp).and_then(|l| l.meta.parent);
    }
    out
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 + 1e-6
        && outer.y0 <= inner.y0 + 1e-6
        && outer.x1 >= inner.x1 - 1e-6
        && outer.y1 >= inner.y1 - 1e-6
}

fn side_of(port: PortSide, transform: Affine) -> route::Side {
    let n = match port {
        PortSide::Top => Vec2::new(0.0, -1.0),
        PortSide::Right => Vec2::new(1.0, 0.0),
        PortSide::Bottom => Vec2::new(0.0, 1.0),
        PortSide::Left => Vec2::new(-1.0, 0.0),
    };
    let c = transform.as_coeffs();
    let v = Vec2::new(c[0] * n.x + c[2] * n.y, c[1] * n.x + c[3] * n.y);
    if v.x.abs() >= v.y.abs() {
        if v.x >= 0.0 { route::Side::Right } else { route::Side::Left }
    } else if v.y >= 0.0 {
        route::Side::Bottom
    } else {
        route::Side::Top
    }
}

/// Where a connector end is, for the router.
fn end_of(e: &ConnectorEnd, comp: &Composition, nodes: &HashMap<Id, Node>, outer: Affine) -> route::End {
    let free = route::End::Free(outer * e.point.to_point());
    let Some(a) = e.attach else { return free };
    let Some(node) = nodes.get(&a.layer) else { return free };
    let world = node.world();
    match a.port {
        PortRef::Auto => route::End::Floating(world),
        PortRef::Port(i) => {
            let Some(port) = comp.layer(a.layer).and_then(|l| l.meta.ports().get(i as usize).copied()) else {
                return route::End::Floating(world);
            };
            let r = node.local;
            let local = Point::new(r.x0 + port.pos.x * r.width(), r.y0 + port.pos.y * r.height());
            route::End::Port { point: node.transform * local, side: side_of(port.side, node.transform), shape: world }
        }
    }
}

fn jump_style(j: xenify_doc::JumpStyle) -> route::JumpStyle {
    match j {
        xenify_doc::JumpStyle::Arc => route::JumpStyle::Arc,
        xenify_doc::JumpStyle::Gap => route::JumpStyle::Gap,
        xenify_doc::JumpStyle::Square => route::JumpStyle::Square,
        xenify_doc::JumpStyle::None => route::JumpStyle::None,
    }
}

fn flatten(path: &BezPath) -> Vec<Point> {
    let mut out = Vec::new();
    kurbo::flatten(path, 0.5, |el| {
        if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
            out.push(p);
        }
    });
    out
}

/// A connector's route at one moment, in scene space.
#[derive(Clone, Debug, PartialEq)]
pub struct Routed {
    pub id: Id,
    /// The polyline the router chose (before rounding, curves and hops).
    pub points: Vec<Point>,
    /// The line as drawn.
    pub path: BezPath,
}

/// Lays out every connector of `comp` among the already drawn `layers`
/// (scene space), and writes the result into their drawn layers.
pub(crate) fn resolve_connectors(comp: &Composition, layers: &mut [SceneLayer], outer: Affine, time: f64) {
    let routes = route_all(comp, layers, outer);
    if routes.is_empty() {
        return;
    }
    let by_id: HashMap<Id, &Routed> = routes.iter().map(|r| (r.id, r)).collect();
    for sl in layers.iter_mut() {
        let Some(r) = by_id.get(&sl.id) else { continue };
        let Some(layer) = comp.layer(sl.id) else { continue };
        let det = sl.transform.determinant();
        if det.abs() < 1e-12 {
            continue;
        }
        let local = sl.transform.inverse() * &r.path;
        let local_points: Vec<Point> = r.points.iter().map(|p| sl.transform.inverse() * *p).collect();
        let taken = std::mem::replace(sl, placeholder(sl.id));
        // Labels get a box of the canvas colour on diagram canvases.
        let bg = layer
            .meta
            .label
            .as_ref()
            .and_then(|l| l.background)
            .or_else(|| (comp.settings.kind == xenify_doc::CompKind::Diagram).then_some(comp.settings.background));
        *sl = draw_connector(taken, layer, local, &local_points, time, bg);
    }
}

fn placeholder(id: Id) -> SceneLayer {
    SceneLayer {
        id,
        transform: Affine::IDENTITY,
        opacity: 0.0,
        content: SceneContent::Shapes(Vec::new()),
        matte: None,
        distort: None,
        effects: Vec::new(),
        blend: Default::default(),
        masks: Vec::new(),
        copy: 0,
    }
}

/// Routes every drawn connector in `comp` (in stacking order) among `layers`.
pub fn route_all(comp: &Composition, layers: &[SceneLayer], outer: Affine) -> Vec<Routed> {
    let drawn: HashSet<Id> = layers.iter().map(|l| l.id).collect();
    let connectors: Vec<&Layer> =
        comp.layers.iter().filter(|l| l.meta.connector.is_some() && drawn.contains(&l.id)).collect();
    if connectors.is_empty() {
        return Vec::new();
    }
    let is_connector: HashSet<Id> = connectors.iter().map(|l| l.id).collect();
    let mut nodes: HashMap<Id, Node> = HashMap::new();
    for l in layers.iter().filter(|l| l.copy == 0 && !is_connector.contains(&l.id)) {
        if let Some((transform, local)) = frame_of(l) {
            nodes.entry(l.id).or_insert(Node { transform, local });
        }
    }
    // Groups glue as the box around their members.
    for g in comp.layers.iter().filter(|l| matches!(l.kind, LayerKind::Group | LayerKind::Null)) {
        if nodes.contains_key(&g.id) {
            continue;
        }
        let members = crate::descendants(comp, g.id);
        if let Some(b) = members.iter().filter_map(|m| nodes.get(m).map(|n| n.world())).reduce(|a, b| a.union(b)) {
            nodes.insert(g.id, Node { transform: Affine::IDENTITY, local: b });
        }
    }
    let obstacles_all: Vec<(Id, Rect)> = layers
        .iter()
        .filter(|l| l.copy == 0 && !is_connector.contains(&l.id))
        .filter_map(|l| Some((l.id, nodes.get(&l.id)?.world())))
        .collect();

    let mut polys: Vec<Vec<Point>> = Vec::new();
    for layer in &connectors {
        let c = layer.meta.connector.as_ref().expect("filtered");
        let from = end_of(&c.from, comp, &nodes, outer);
        let to = end_of(&c.to, comp, &nodes, outer);
        let own: Vec<Rect> = [from.shape(), to.shape()].into_iter().flatten().collect();
        let mut skip: HashSet<Id> = HashSet::new();
        for a in c.attached() {
            skip.insert(a);
            skip.extend(ancestors(comp, a));
            skip.extend(crate::descendants(comp, a));
        }
        skip.extend(ancestors(comp, layer.id));
        let obstacles: Vec<Rect> = obstacles_all
            .iter()
            .filter(|(id, r)| !skip.contains(id) && !own.iter().any(|o| contains(*r, *o) || contains(*o, *r)))
            .map(|(_, r)| *r)
            .collect();
        let waypoints: Vec<Point> = c.waypoints.iter().map(|w| outer * w.to_point()).collect();
        let style = match c.route {
            RouteStyle::Orthogonal => route::Style::Orthogonal,
            RouteStyle::Straight => route::Style::Straight,
            RouteStyle::Curved => route::Style::Curved,
        };
        let req = route::Request {
            from,
            to,
            waypoints: &waypoints,
            obstacles: &obstacles,
            occupied: &polys,
            options: route::Options { style, margin: c.margin.max(0.0), avoid: c.avoid, ..route::Options::default() },
        };
        let mut poly = route::route(&req);
        // Straight and curved lines stop at the shape's real outline (a
        // circle, a diamond, a cloud), not its box.
        if style != route::Style::Orthogonal && poly.len() >= 2 {
            let n = poly.len();
            for (end, at, next) in [(&c.from, 0, 1), (&c.to, n - 1, n - 2)] {
                if let Some(a) = end.attach.filter(|a| matches!(a.port, xenify_doc::PortRef::Auto))
                    && let Some(outline) = outline_of(layers, a.layer)
                    && let Some(node) = nodes.get(&a.layer)
                    && let Some(p) = clip_to_outline(&outline, node.world().center(), poly[next])
                {
                    poly[at] = p;
                }
            }
        }
        polys.push(poly);
    }

    // Line jumps against everything drawn earlier (curves by their outline).
    let curved: Vec<bool> =
        connectors.iter().map(|l| l.meta.connector.as_ref().is_some_and(|c| c.route == RouteStyle::Curved)).collect();
    let shapes: Vec<Vec<Point>> =
        polys
            .iter()
            .zip(&curved)
            .map(|(p, &cv)| {
                if cv { flatten(&route::build_path(p, &[], route::JumpStyle::None, 0.0, 0.0, true)) } else { p.clone() }
            })
            .collect();
    let refs: Vec<&[Point]> = shapes.iter().map(|p| p.as_slice()).collect();
    let size = connectors.iter().filter_map(|l| l.meta.connector.as_ref()).map(|c| c.jump_size).fold(0.0, f64::max);
    let hops = route::find_hops(&refs, size.max(1.0));
    connectors
        .iter()
        .zip(polys)
        .enumerate()
        .map(|(k, (layer, points))| {
            let c = layer.meta.connector.as_ref().expect("filtered");
            let path = route::build_path(
                &points,
                &hops[k],
                jump_style(c.jump),
                c.jump_size.max(0.0),
                c.corner_radius.max(0.0),
                curved[k],
            );
            Routed { id: layer.id, points, path }
        })
        .collect()
}

/// The connector's drawn layer with its new line (layer space), arrowheads,
/// flow and label.
fn draw_connector(
    mut sl: SceneLayer,
    layer: &Layer,
    path: BezPath,
    points: &[Point],
    time: f64,
    background: Option<xenify_doc::Color>,
) -> SceneLayer {
    let LayerKind::Shape { shapes } = &layer.kind else { return sl };
    let c = layer.meta.connector.as_ref().expect("a connector");
    let SceneContent::Shapes(items) = &mut sl.content else { return sl };
    let Some(first) = items.first_mut() else { return sl };
    first.path = path;
    first.fill = None;
    first.gradient = None;
    let shape = shapes.first();
    if let (Some(st), Some(stroke)) = (&mut first.stroke, shape.and_then(|s| s.stroke.as_ref())) {
        if let Some(flow) = c.flow {
            let w = st.width.max(0.5);
            match flow.kind {
                FlowKind::Dashes => st.dashes = vec![3.0 * w + 4.0, 2.0 * w + 4.0],
                FlowKind::Dots => {
                    st.dashes = vec![0.01, 2.5 * w + 4.0];
                    st.cap = LineCap::Round;
                }
            }
            st.dash_offset = -flow.speed * time;
        }
        if st.marks.is_some() {
            st.marks = stroke_style::marks(&first.path, st, &stroke.style, stroke.cap_roundness, layer.id.0);
        }
    }
    // Only the line itself: drop old arrowheads and extra shapes.
    items.truncate(1);
    if let Some(shape) = shape {
        add_markers(items, std::slice::from_ref(shape), layer.id.0);
    }
    match &layer.meta.label {
        Some(label) if !label.text.is_empty() && points.len() >= 2 => {
            let (p, d) = along(points, c.label_at);
            let n = Vec2::new(d.y, -d.x);
            let n = if n.y < -1e-9 || (n.y.abs() <= 1e-9 && n.x > 0.0) { n } else { -n };
            let t = text_of(label);
            let Some(bg) = background.filter(|c| c.a > 0.0) else {
                // No box: beside the line, so it doesn't run through the text.
                let at = p + n * (label.font_size * 0.8);
                let text = text_layer(sl.id, sl.transform * Affine::translate(at.to_vec2()), t);
                return with_text(sl, text);
            };
            // A box behind the text, on the line.
            let pad = label.font_size * 0.3;
            let b = crate::TextMeasure::text_bounds(&mut crate::ApproxText, &t).inflate(pad, pad * 0.6);
            let item = crate::DrawItem {
                path: kurbo::Shape::to_path(&kurbo::RoundedRect::from_rect(b, pad), 0.1),
                fill_rule: Default::default(),
                gradient: None,
                fill: Some(bg),
                stroke: None,
                pattern: None,
            };
            let place = sl.transform * Affine::translate(p.to_vec2());
            let mut boxed = text_layer(sl.id, place, t.clone());
            boxed.content = SceneContent::Shapes(vec![item]);
            let text = text_layer(sl.id, place, t);
            let mut group = with_text(sl, text);
            if let SceneContent::Group(members) = &mut group.content {
                members.insert(1, boxed);
            }
            group
        }
        _ => sl,
    }
}

/// For exporters that write the document's layers rather than drawn scenes
/// (Lottie): every label becomes a real text layer (over its shape, parented
/// to it, or where it sits on its connector at the start), and the labels
/// themselves are removed.
pub fn labels_as_layers(project: &mut xenify_doc::Project) {
    let comps: Vec<Id> = project.compositions.iter().map(|c| c.id).collect();
    for cid in comps {
        let scene = crate::evaluate_project(project, cid, 0.0);
        let Ok(comp) = project.comp(cid) else { continue };
        // Connector labels follow their lines: where each is drawn, frame by frame.
        let labelled: Vec<Id> = comp
            .layers
            .iter()
            .filter(|l| l.meta.connector.is_some() && l.meta.label.as_ref().is_some_and(|t| !t.text.is_empty()))
            .map(|l| l.id)
            .collect();
        let mut tracks: HashMap<Id, Vec<(f64, Point)>> = HashMap::new();
        if !labelled.is_empty() {
            let (fps, duration) = (comp.settings.fps.max(1.0), comp.settings.duration.max(0.0));
            let frames = ((duration * fps).ceil() as usize).min(100_000);
            for f in 0..=frames {
                let t = (f as f64 / fps).min(duration);
                let s = crate::evaluate_project(project, cid, t);
                for id in &labelled {
                    let at = s.layer(*id).and_then(|d| match &d.content {
                        SceneContent::Group(inner) => inner
                            .iter()
                            .find(|m| matches!(m.content, SceneContent::Text(_)))
                            .map(|m| m.transform * Point::ZERO),
                        _ => None,
                    });
                    if let Some(p) = at {
                        tracks.entry(*id).or_default().push((t, p));
                    }
                }
            }
        }
        let Ok(comp) = project.comp(cid) else { continue };
        let old = comp.layers.clone();
        let mut layers = Vec::with_capacity(old.len());
        for mut layer in old {
            let label = layer.meta.label.take().filter(|l| !l.text.is_empty());
            let drawn = scene.layer(layer.id).cloned();
            let (id, name, meta) = (layer.id, layer.meta.name.clone(), layer.meta.clone());
            layers.push(layer);
            let Some(label) = label else { continue };
            // The label as drawn: the text member of the layer's group.
            let Some(SceneContent::Group(inner)) = drawn.map(|d| d.content) else { continue };
            let Some(text) = inner.iter().find(|m| matches!(m.content, SceneContent::Text(_))) else { continue };
            let SceneContent::Text(st) = &text.content else { continue };
            let mut t =
                xenify_doc::TextLayer::new(label.text.clone(), label.font_family.clone(), label.font_size, label.color);
            t.align = label.align;
            t.valign = xenify_doc::TextVAlign::Middle;
            t.frame = st.frame;
            let mut tl = Layer::new(project.alloc_id(), format!("{name} label"), LayerKind::Text(t));
            tl.meta.in_point = meta.in_point;
            tl.meta.out_point = meta.out_point;
            if meta.connector.is_some() {
                let track = tracks.get(&id).cloned().unwrap_or_default();
                let moves = track.windows(2).any(|w| w[0].1.distance(w[1].1) > 1e-3);
                tl.transform.position = if moves {
                    let keys = track
                        .iter()
                        .map(|(t, p)| xenify_doc::Keyframe { time: *t, value: p.to_vec2(), easing: Default::default() })
                        .collect();
                    xenify_doc::Property::from_keyframes(keys)
                        .unwrap_or_else(|| (text.transform * Point::ZERO).to_vec2().into())
                } else {
                    (text.transform * Point::ZERO).to_vec2().into()
                };
                // The box behind it (diagram canvases), moving with it.
                if let Some(bx) =
                    inner.iter().find(|m| matches!(m.content, SceneContent::Shapes(_)) && m.transform == text.transform)
                    && let SceneContent::Shapes(items) = &bx.content
                    && let Some(item) = items.first()
                {
                    let mut shape = ShapeItem::path(item.path.clone(), item.fill, None);
                    shape.name = "Label box".into();
                    let mut b = Layer::new(
                        project.alloc_id(),
                        format!("{name} label box"),
                        LayerKind::Shape { shapes: vec![shape] },
                    );
                    b.meta.in_point = meta.in_point;
                    b.meta.out_point = meta.out_point;
                    b.transform.position = tl.transform.position.clone();
                    layers.push(b);
                }
            } else {
                // In the shape's own space, so it moves and turns with it.
                tl.meta.parent = Some(id);
            }
            layers.push(tl);
        }
        if let Ok(c) = project.comp_mut(cid) {
            c.layers = layers;
        }
    }
}

#[cfg(test)]
mod tests;
