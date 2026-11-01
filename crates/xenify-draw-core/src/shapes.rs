//! Drawing and shape commands: the shapes the toolbar draws, Pathfinder,
//! align and distribute, grouping, arrange, the Path menu, copy and paste,
//! and editing a shape's points (Edit Points).

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use serde_json::{Value as Json, json};
use xenify_doc::primitives;
use xenify_doc::{Color, Geometry, Id, Layer, LayerKind, PropPath, ShapeField, ShapeItem, Value, bezier};
use xenify_ops::Command;
use xenify_ops::arrange::{AlignEdge, AlignTo, Axis};
use xenify_ops::draw::{Figure, FigureParams};
use xenify_ops::order::Placement;
use xenify_ops::pathfinder::PathOp;

use crate::{Session, err, f, id_arg, parse_color};

/// A point from the screens: `[x, y]`.
fn pt(v: &Json) -> Option<Point> {
    Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))
}

fn pj(p: Point) -> Json {
    json!([p.x, p.y])
}

/// Selected points: `[[shape, index], …]`.
fn point_list(args: &Json) -> Vec<(usize, usize)> {
    args.get("points")
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_u64()? as usize, p.get(1)?.as_u64()? as usize))).collect())
        .unwrap_or_default()
}

impl Session {
    pub(crate) fn shape_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        // The point calls edit a layer's masks instead of its shapes with `masks: true`.
        self.on_masks = args.get("masks").and_then(Json::as_bool).unwrap_or(false);
        Some(match name {
            "add_mask_at" => self.add_mask_at(args),
            "add_figure" => self.add_figure(args),
            "add_line" => self.add_line(args),
            "path_op" => self.path_op(args),
            "align" => self.align(args),
            "distribute" => self.distribute(args),
            "group" => self.group(),
            "ungroup" => self.ungroup(),
            "clip" => self.clip(args.get("make").and_then(Json::as_bool).unwrap_or(true)),
            "precompose" => self.precompose(),
            "arrange" => self.arrange(args),
            "rotate_by" => self.turn(args),
            "flip" => self.flip(args),
            "convert_to_path" => self.convert_to_path(),
            "simplify" => self.each_path("Simplify", |p| bezier::simplify(p, 1.0)),
            "subdivide" => self.each_path("Subdivide", |p| bezier::subdivide(p, &[])),
            "outline_stroke" => self.outline_stroke(),
            "copy" => self.copy(),
            "paste" => self.paste(args),
            "points" => self.points(id_arg(args, "id").ok().or(self.selection.first().copied())),
            "points_begin" => {
                self.gesture_seq += 1;
                Ok(Json::Null)
            }
            "points_move" => self.points_move(args),
            "handle_move" => self.handle_move(args),
            "point_insert" => self.point_insert(args),
            "points_edit" => self.points_edit(args),
            "corners" => Ok(self.corners(id_arg(args, "id").ok().or(self.selection.first().copied()))),
            "smart_shape" => self.smart_shape(args),
            "set_smart_shape" => self.set_smart_shape(args),
            "corner_round" => self.corner_round(args),
            "pen_path" => self.pen_path(args),
            "freehand" => self.freehand(args),
            "set_paint" => self.set_paint(args),
            "knife" => self.knife(args),
            "knife_preview" => self.knife_preview(args),
            "sculpt" => self.sculpt(args),
            _ => return None,
        })
    }

    // ----- Drawing shapes -----

    fn styled(&self, mut item: ShapeItem, args: &Json) -> ShapeItem {
        let fill = args.get("fill").and_then(parse_color);
        let stroke = args.get("stroke").and_then(parse_color);
        let width = f(args, "stroke_width").unwrap_or(0.0);
        item.fill = fill.map(|c| xenify_doc::Fill { color: c.into(), gradient: None, pattern: None });
        item.stroke = match stroke {
            Some(c) if width > 0.0 => {
                let mut s = ShapeItem::path(BezPath::new(), None, Some((c, width))).stroke;
                if let Some(s) = &mut s {
                    s.join = xenify_doc::LineJoin::Round;
                }
                s
            }
            _ => None,
        };
        item
    }

    /// Any shape of the shapes button, filling the box `x, y, w, h`:
    /// rect, rounded, ellipse, triangle, polygon, star, gear, heart, arrow, bubble.
    /// Polygons, stars and gears stay live (their sides, points and teeth can change).
    fn add_figure(&mut self, args: &Json) -> Result<Json, String> {
        let (x, y) = (f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
        let (w, h) = (f(args, "w").unwrap_or(100.0).abs().max(1.0), f(args, "h").unwrap_or(100.0).abs().max(1.0));
        let kind = args.get("kind").and_then(Json::as_str).unwrap_or("rect");
        let size = Vec2::new(w, h);
        let mut params = FigureParams::default();
        if let Some(n) = f(args, "sides") {
            params.sides = n.clamp(3.0, 64.0) as usize;
        }
        if let Some(n) = f(args, "inner") {
            params.star_inner = n.clamp(0.05, 0.95);
        }
        if let Some(n) = f(args, "teeth") {
            params.teeth = n.clamp(3.0, 96.0) as usize;
        }
        if let Some(n) = f(args, "depth") {
            params.tooth_depth = n.clamp(0.02, 0.8);
        }
        if let Some(n) = f(args, "hole") {
            params.hole = n.clamp(0.0, 0.9);
        }
        let figure = |fig: Figure, params: &FigureParams| {
            let half = Rect::from_center_size(Point::ZERO, size.to_size());
            let mut item = ShapeItem::path(xenify_ops::draw::figure_path(fig, params, half), None, None);
            if let Some(g) = xenify_ops::draw::live_figure(fig, params, size.to_size()) {
                item.geometry = g;
            }
            item.name = fig.name().into();
            item
        };
        let (item, name) = match kind {
            "ellipse" => (ShapeItem::ellipse(size, Color::WHITE), "Ellipse"),
            "rounded" => {
                let mut item = ShapeItem::rect(size, Color::WHITE);
                let r = f(args, "radius").unwrap_or(w.min(h) * 0.15);
                if let Geometry::Rect { corner_radius, .. } = &mut item.geometry {
                    *corner_radius = r.into();
                }
                item.name = "Rounded Rectangle".into();
                (item, "Rounded Rectangle")
            }
            "triangle" => {
                let p = FigureParams { sides: 3, ..params };
                let mut item = figure(Figure::Polygon, &p);
                item.name = "Triangle".into();
                (item, "Triangle")
            }
            "polygon" => (figure(Figure::Polygon, &params), "Polygon"),
            "star" => (figure(Figure::Star, &params), "Star"),
            "gear" => (figure(Figure::Gear, &params), "Gear"),
            "heart" => (figure(Figure::Heart, &params), "Heart"),
            "arrow" => (figure(Figure::Arrow, &params), "Arrow"),
            "bubble" => (figure(Figure::Bubble, &params), "Speech Bubble"),
            _ => (ShapeItem::rect(size, Color::WHITE), "Rectangle"),
        };
        let item = self.styled(item, args);
        let id = self.next_id();
        let layer =
            Layer::shape(id, self.unique_name(name), vec![item]).with_position(Vec2::new(x + w / 2.0, y + h / 2.0));
        self.add_layer(&format!("Add {}", name.to_lowercase()), layer)
    }

    /// A straight line from `a` to `b` in the stroke colour, with round ends.
    fn add_line(&mut self, args: &Json) -> Result<Json, String> {
        let a = args.get("a").and_then(pt).ok_or("missing a")?;
        let b = args.get("b").and_then(pt).ok_or("missing b")?;
        let c = a.midpoint(b).to_vec2();
        let mut path = BezPath::new();
        path.move_to(a - c);
        path.line_to(b - c);
        let color = args.get("stroke").and_then(parse_color).unwrap_or(Color::BLACK);
        let mut item = ShapeItem::path(path, None, Some((color, f(args, "stroke_width").unwrap_or(4.0).max(0.5))));
        if let Some(s) = &mut item.stroke {
            s.cap = xenify_doc::LineCap::Round;
        }
        let id = self.next_id();
        let layer = Layer::shape(id, self.unique_name("Line"), vec![item]).with_position(c);
        self.add_layer("Add line", layer)
    }

    /// A path through pen points `{p: [x, y], h: [dx, dy]}` (composition
    /// space; `h` is the outgoing handle, mirrored on the way in).
    fn pen_path(&mut self, args: &Json) -> Result<Json, String> {
        let pts: Vec<bezier::PenPoint> = args
            .get("nodes")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|n| {
                        let pos = n.get("p").and_then(pt)?;
                        let h = n.get("h").and_then(pt).map_or(Vec2::ZERO, |h| h.to_vec2());
                        Some(bezier::PenPoint { pos, handle: h })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let into = args.get("into").and_then(Json::as_u64).map(Id);
        if pts.len() < if into.is_some() { 1 } else { 2 } {
            return Err("a path needs two points".into());
        }
        let closed = args.get("closed").and_then(Json::as_bool).unwrap_or(false);
        // Continuing an open line from one of its ends (`into`: layer, shape, subpath, from_start).
        if let Some(target) = into {
            let shape = f(args, "shape").unwrap_or(0.0) as usize;
            let subpath = f(args, "subpath").unwrap_or(0.0) as usize;
            let from_start = args.get("from_start").and_then(Json::as_bool).unwrap_or(false);
            let inv = self.world(target).inverse();
            let lin = Affine::new({
                let c = inv.as_coeffs();
                [c[0], c[1], c[2], c[3], 0.0, 0.0]
            });
            let local: Vec<bezier::PenPoint> = pts
                .iter()
                .map(|p| bezier::PenPoint { pos: inv * p.pos, handle: (lin * p.handle.to_point()).to_vec2() })
                .collect();
            let path = self.current_path(target, shape).ok_or("no such shape")?;
            let new =
                bezier::extend_subpath(&path, subpath, from_start, &local, closed).ok_or("that line is closed")?;
            self.gesture_seq += 1;
            self.set_shape_path(target, shape, new, if closed { "Close path" } else { "Continue path" })?;
            self.selection = vec![target];
            return Ok(json!(target.0));
        }
        let b = pts.iter().fold(Rect::from_points(pts[0].pos, pts[0].pos), |r, p| r.union_pt(p.pos));
        let c = b.center().to_vec2();
        let local: Vec<bezier::PenPoint> =
            pts.iter().map(|p| bezier::PenPoint { pos: p.pos - c, handle: p.handle }).collect();
        let path = bezier::pen_path(&local, closed);
        let fill = if closed { args.get("fill").and_then(parse_color) } else { None };
        let stroke = args.get("stroke").and_then(parse_color);
        let width = f(args, "stroke_width").unwrap_or(4.0);
        let stroke = stroke.filter(|_| width > 0.0).map(|c| (c, width));
        let item =
            ShapeItem::path(path, fill, stroke.or(if fill.is_none() { Some((Color::BLACK, 2.0)) } else { None }));
        let id = self.next_id();
        let layer = Layer::shape(id, self.unique_name("Path"), vec![item]).with_position(c);
        self.add_layer("Draw path", layer)
    }

    /// A pencil or brush stroke through `points` (composition space).
    /// With `recognize`, a stroke that looks like a line, ellipse,
    /// rectangle or polygon becomes that clean shape; the brush paints the
    /// stroke's outline as a filled shape.
    fn freehand(&mut self, args: &Json) -> Result<Json, String> {
        use xenify_ops::draw::Recognized;
        let points: Vec<Point> =
            args.get("points").and_then(Json::as_array).map(|a| a.iter().filter_map(pt).collect()).unwrap_or_default();
        if points.len() < 2 {
            return Err("draw a longer stroke".into());
        }
        let brush = args.get("brush").and_then(Json::as_bool).unwrap_or(false);
        let width = f(args, "width").unwrap_or(4.0).max(0.5);
        let smoothing = f(args, "smoothing").unwrap_or(3.0).clamp(1.0, 10.0);
        let fill = args.get("fill").and_then(parse_color).unwrap_or(Color::BLACK);
        let stroke = args.get("stroke").and_then(parse_color).unwrap_or(Color::BLACK);
        let mut result = if args.get("recognize").and_then(Json::as_bool).unwrap_or(false) {
            xenify_ops::draw::recognize(&points)
        } else {
            Recognized::Free { path: xenify_ops::draw::smooth_stroke(&points, false), closed: false }
        };
        if let Recognized::Free { path, .. } = &mut result {
            let b = kurbo::Shape::bounding_box(&*path);
            *path = bezier::simplify(path, (b.width().max(b.height()) * 0.004).clamp(0.3, 2.5) * smoothing);
        }
        let line = |path: BezPath| {
            let mut item = ShapeItem::path(path, None, Some((stroke, width)));
            if let Some(s) = &mut item.stroke {
                s.cap = xenify_doc::LineCap::Round;
                s.join = xenify_doc::LineJoin::Round;
            }
            item
        };
        let painted = |mut item: ShapeItem| {
            item.fill = Some(xenify_doc::Fill { color: fill.into(), gradient: None, pattern: None });
            item.stroke = None;
            item
        };
        let (name, item, centre, angle) = match result {
            Recognized::Line(a, b) => {
                let mut p = BezPath::new();
                p.move_to(a);
                p.line_to(b);
                ("Line", line(p), None, 0.0)
            }
            Recognized::Ellipse { center, radii, angle } => {
                let name = if (radii.x - radii.y).abs() < 1e-9 { "Circle" } else { "Ellipse" };
                (name, painted(ShapeItem::ellipse(Vec2::new(radii.x * 2.0, radii.y * 2.0), fill)), Some(center), angle)
            }
            Recognized::Rect { center, size, angle } => {
                let square = (size.x - size.y).abs() < 0.08 * size.x.max(size.y);
                let size = if square { Vec2::new(size.x.max(size.y), size.x.max(size.y)) } else { size };
                (if square { "Square" } else { "Rectangle" }, painted(ShapeItem::rect(size, fill)), Some(center), angle)
            }
            Recognized::Polygon(corners) => {
                let name = if corners.len() == 3 { "Triangle" } else { "Polygon" };
                (name, painted(ShapeItem::path(xenify_ops::draw::closed_polyline(&corners), None, None)), None, 0.0)
            }
            Recognized::Free { path, closed } if brush => {
                let _ = closed;
                let outline = kurbo::stroke(
                    path.iter(),
                    &kurbo::Stroke::new(width).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round),
                    &kurbo::StrokeOpts::default(),
                    0.1,
                );
                ("Brush stroke", ShapeItem::path(bezier::simplify(&outline, 0.3), Some(fill), None), None, 0.0)
            }
            Recognized::Free { mut path, closed } => {
                if closed {
                    path.close_path();
                }
                ("Pencil", line(path), None, 0.0)
            }
        };
        let mut item = item;
        // Paths are drawn in composition space: centre them on their layer.
        let centre = match centre {
            Some(c) => c.to_vec2(),
            None => {
                let Geometry::Path { path } = &mut item.geometry else { return Err("no path".into()) };
                let p = path.value_at(0.0);
                let c = kurbo::Shape::bounding_box(&p).center().to_vec2();
                *path = (Affine::translate(-c) * p).into();
                c
            }
        };
        let id = self.next_id();
        let mut layer = Layer::shape(id, self.unique_name(name), vec![item]).with_position(centre);
        if angle != 0.0 {
            layer.transform.rotation = angle.to_degrees().into();
        }
        self.add_layer(&format!("Draw {}", name.to_lowercase()), layer)
    }

    /// Cuts the selected shapes (or every shape the line crosses) in two along `a`–`b`.
    fn knife(&mut self, args: &Json) -> Result<Json, String> {
        let a = args.get("a").and_then(pt).ok_or("missing a")?;
        let b = args.get("b").and_then(pt).ok_or("missing b")?;
        let all: Vec<Id> = self
            .project()
            .comp(self.comp)
            .map(|c| c.layers.iter().filter(|l| !l.meta.locked).map(|l| l.id).collect())
            .unwrap_or_default();
        // The selected shapes, or (when the cut misses them all) any shape it crosses.
        let cut = xenify_ops::pathfinder::knife(self.project(), self.comp, &self.selection, a, b, self.time);
        let (cmd, ids) = match cut {
            Ok(r) if !r.1.is_empty() => r,
            _ => xenify_ops::pathfinder::knife(self.project(), self.comp, &all, a, b, self.time).map_err(err)?,
        };
        self.exec("Knife", cmd)?;
        let n = ids.len();
        self.selection = ids;
        Ok(json!(n))
    }

    /// What the knife from `a` to `b` would cut (the same shapes `knife` would): `[{id, cuts: [[a, b]…]}]`.
    fn knife_preview(&mut self, args: &Json) -> Result<Json, String> {
        let a = args.get("a").and_then(pt).ok_or("missing a")?;
        let b = args.get("b").and_then(pt).ok_or("missing b")?;
        let mut cuts = xenify_ops::pathfinder::knife_cuts(self.project(), self.comp, &self.selection, a, b, self.time);
        if cuts.is_empty() {
            let all: Vec<Id> =
                self.project().comp(self.comp).map(|c| c.layers.iter().map(|l| l.id).collect()).unwrap_or_default();
            cuts = xenify_ops::pathfinder::knife_cuts(self.project(), self.comp, &all, a, b, self.time);
        }
        Ok(Json::Array(
            cuts.into_iter()
                .map(|(id, c)| json!({"id": id.0, "cuts": c.iter().map(|(p, q)| json!([[p.x, p.y], [q.x, q.y]])).collect::<Vec<_>>()}))
                .collect(),
        ))
    }

    /// Turns fill or stroke on (`"#rrggbb"`) or off (`false`) on every shape
    /// of the selected layers (or of `ids`); `stroke_width` sets the width
    /// of strokes turned on.
    fn set_paint(&mut self, args: &Json) -> Result<Json, String> {
        let ids: Vec<Id> = match args.get("ids").and_then(Json::as_array) {
            Some(a) => a.iter().filter_map(Json::as_u64).map(Id).collect(),
            None => self.selection.clone(),
        };
        let width = f(args, "stroke_width").unwrap_or(4.0).max(0.5);
        let mut cmds = Vec::new();
        for id in ids {
            let Ok(l) = self.project().layer(id) else { continue };
            for (i, item) in l.shapes().iter().enumerate() {
                let mut new = item.clone();
                match args.get("fill") {
                    Some(Json::Bool(false)) => new.fill = None,
                    Some(v) => {
                        if let Some(c) = parse_color(v) {
                            match &mut new.fill {
                                Some(fl) => fl.color = c.into(),
                                None => {
                                    new.fill = Some(xenify_doc::Fill { color: c.into(), gradient: None, pattern: None })
                                }
                            }
                        }
                    }
                    None => {}
                }
                match args.get("stroke") {
                    Some(Json::Bool(false)) => new.stroke = None,
                    Some(v) => {
                        if let Some(c) = parse_color(v) {
                            match &mut new.stroke {
                                Some(st) => st.color = c.into(),
                                None => new.stroke = ShapeItem::path(BezPath::new(), None, Some((c, width))).stroke,
                            }
                        }
                    }
                    None => {}
                }
                if new != *item {
                    cmds.push(Command::SetShape { layer: id, index: i, shape: new });
                }
            }
        }
        self.exec_all("Change colours", cmds)?;
        Ok(Json::Null)
    }

    /// One step of the sculpt brush over the selected shapes (groups'
    /// members too): `mode` push, attract or repel, at `center` moved by
    /// `delta` (composition space), `radius` and `amount` in canvas pixels.
    /// Steps of one drag (same `stroke`) are one undo step.
    fn sculpt(&mut self, args: &Json) -> Result<Json, String> {
        use xenify_ops::draw::SculptMode;
        let mode = match args.get("mode").and_then(Json::as_str) {
            Some("attract") => SculptMode::Attract,
            Some("repel") => SculptMode::Repel,
            _ => SculptMode::Push,
        };
        let center = args.get("center").and_then(pt).ok_or("missing center")?;
        let delta = args.get("delta").and_then(pt).map_or(Vec2::ZERO, |p| p.to_vec2());
        let radius = f(args, "radius").unwrap_or(60.0).max(1.0);
        let amount = f(args, "amount").unwrap_or(4.0);
        let Ok(c) = self.project().comp(self.comp) else { return Ok(Json::Null) };
        let mut layers = Vec::new();
        for &id in &self.selection {
            for l in std::iter::once(id).chain(xenify_anim::descendants(c, id)) {
                if c.layer(l).is_some_and(|x| matches!(x.kind, LayerKind::Shape { .. }) && !x.meta.locked)
                    && !layers.contains(&l)
                {
                    layers.push(l);
                }
            }
        }
        if layers.is_empty() {
            return Err("Select the shapes to sculpt first.".into());
        }
        let key = format!("sculpt:{}", f(args, "stroke").unwrap_or(0.0));
        let mut cmds = Vec::new();
        for id in layers {
            let w = self.world(id);
            if w.determinant().abs() < 1e-12 {
                continue;
            }
            let inv = w.inverse();
            let scale = w.determinant().abs().sqrt();
            let k = inv.as_coeffs();
            let local_delta = Vec2::new(k[0] * delta.x + k[2] * delta.y, k[1] * delta.x + k[3] * delta.y);
            let local_center = inv * center;
            let Ok(layer) = self.project().layer(id).cloned() else { continue };
            for (index, shape) in layer.shapes().iter().enumerate() {
                let Some(before) = shape_path(shape, self.time) else { continue };
                let b = kurbo::Shape::bounding_box(&before).inflate(radius / scale, radius / scale);
                if !b.contains(local_center) {
                    continue;
                }
                let after =
                    xenify_ops::draw::sculpt(&before, mode, local_center, radius / scale, local_delta, amount / scale);
                if after == before {
                    continue;
                }
                if !matches!(shape.geometry, Geometry::Path { .. }) {
                    let baked = ShapeItem { geometry: Geometry::Path { path: after.into() }, ..shape.clone() };
                    cmds.push(Command::SetShape { layer: id, index, shape: baked });
                    continue;
                }
                let prop = PropPath::shape(index, ShapeField::Path);
                cmds.push(
                    Command::edit_value(self.project(), id, prop, self.time, Value::Path(after), self.auto_key)
                        .map_err(err)?,
                );
            }
        }
        if !cmds.is_empty() {
            self.editor.execute_merged("Sculpt", key, Command::batch(cmds)).map_err(err)?;
        }
        Ok(Json::Null)
    }

    // ----- Shape commands -----

    fn path_op(&mut self, args: &Json) -> Result<Json, String> {
        let op = match args.get("op").and_then(Json::as_str).unwrap_or("") {
            "unite" => PathOp::Unite,
            "minus_front" => PathOp::MinusFront,
            "intersect" => PathOp::Intersect,
            "exclude" => PathOp::Exclude,
            o => return Err(format!("no path operation {o}")),
        };
        let (cmd, id) =
            xenify_ops::pathfinder::combine(self.project(), self.comp, &self.selection, op, self.time).map_err(err)?;
        let label = PathOp::ALL.iter().find(|(o, _)| *o == op).map_or("Combine", |(_, l)| l);
        self.exec(label, cmd)?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    fn align(&mut self, args: &Json) -> Result<Json, String> {
        let edge = match args.get("edge").and_then(Json::as_str).unwrap_or("") {
            "left" => AlignEdge::Left,
            "center" => AlignEdge::HCenter,
            "right" => AlignEdge::Right,
            "top" => AlignEdge::Top,
            "middle" => AlignEdge::VCenter,
            "bottom" => AlignEdge::Bottom,
            e => return Err(format!("no edge {e}")),
        };
        let to = if args.get("to").and_then(Json::as_str) == Some("canvas") {
            AlignTo::Composition
        } else {
            AlignTo::Selection
        };
        let (p, comp, sel, time, key) =
            (self.project().clone(), self.comp, self.selection.clone(), self.time, self.auto_key);
        let cmd = xenify_ops::arrange::align(&p, comp, &sel, edge, to, time, key, &mut self.text).map_err(err)?;
        match cmd {
            Some(c) => self.exec("Align", c).map(|_| json!(true)),
            None => Ok(json!(false)),
        }
    }

    fn distribute(&mut self, args: &Json) -> Result<Json, String> {
        let axis =
            if args.get("axis").and_then(Json::as_str) == Some("vertical") { Axis::Vertical } else { Axis::Horizontal };
        let (p, comp, sel, time, key) =
            (self.project().clone(), self.comp, self.selection.clone(), self.time, self.auto_key);
        let cmd = xenify_ops::arrange::distribute(&p, comp, &sel, axis, time, key, &mut self.text).map_err(err)?;
        match cmd {
            Some(c) => self.exec("Distribute", c).map(|_| json!(true)),
            None => Ok(json!(false)),
        }
    }

    fn group(&mut self) -> Result<Json, String> {
        let n = self
            .project()
            .comp(self.comp)
            .map_or(0, |c| c.layers.iter().filter(|l| matches!(l.kind, LayerKind::Group)).count())
            + 1;
        let (p, comp, sel, time) = (self.project().clone(), self.comp, self.selection.clone(), self.time);
        let (cmd, id) =
            xenify_ops::group::group(&p, comp, &sel, &format!("Group {n}"), time, &mut self.text).map_err(err)?;
        self.exec("Group", cmd)?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    fn ungroup(&mut self) -> Result<Json, String> {
        let Ok(c) = self.project().comp(self.comp) else { return Err("no canvas".into()) };
        let groups: Vec<Id> = self
            .selection
            .iter()
            .copied()
            .filter(|id| c.layer(*id).is_some_and(|l| matches!(l.kind, LayerKind::Group)))
            .collect();
        if groups.is_empty() {
            return Err("Select a group to ungroup.".into());
        }
        let children: Vec<Id> = groups
            .iter()
            .flat_map(|g| c.layers.iter().filter(move |l| l.meta.parent == Some(*g)).map(|l| l.id))
            .collect();
        let cmds: Result<Vec<Command>, _> =
            groups.iter().map(|g| xenify_ops::group::ungroup(self.project(), *g)).collect();
        self.exec("Ungroup", Command::batch(cmds.map_err(err)?))?;
        self.selection = children;
        Ok(Json::Null)
    }

    fn clip(&mut self, make: bool) -> Result<Json, String> {
        let cmd = if make {
            xenify_ops::matte::make_clipping_mask(self.project(), self.comp, &self.selection)
        } else {
            xenify_ops::matte::release_clipping_mask(self.project(), self.comp, &self.selection)
        }
        .map_err(err)?;
        self.exec(if make { "Clipping mask" } else { "Release clipping mask" }, cmd)?;
        Ok(Json::Null)
    }

    fn precompose(&mut self) -> Result<Json, String> {
        let n = self.project().compositions.len();
        let (cmd, _, layer) =
            xenify_ops::precomp::precompose(self.project(), self.comp, &self.selection, &format!("Symbol {n}"))
                .map_err(err)?;
        self.exec("Convert to symbol", cmd)?;
        self.selection = vec![layer];
        Ok(json!(layer.0))
    }

    fn arrange(&mut self, args: &Json) -> Result<Json, String> {
        let placement = match args.get("to").and_then(Json::as_str).unwrap_or("") {
            "front" => Placement::Front,
            "back" => Placement::Back,
            "forward" => Placement::Forward,
            "backward" => Placement::Backward,
            "above" => Placement::Above(id_arg(args, "target")?),
            "below" => Placement::Below(id_arg(args, "target")?),
            "into" => Placement::IntoGroup(id_arg(args, "target")?),
            t => return Err(format!("no placement {t}")),
        };
        let ids: Vec<Id> = match args.get("ids").and_then(Json::as_array) {
            Some(a) => a.iter().filter_map(Json::as_u64).map(Id).collect(),
            None => self.selection.clone(),
        };
        if let Some(cmd) = xenify_ops::order::move_layers(self.project(), self.comp, &ids, placement).map_err(err)? {
            self.exec("Arrange", cmd)?;
        }
        Ok(Json::Null)
    }

    /// Applies `m` around the selection's centre (rotate, flip).
    fn about_centre(&mut self, label: &str, m: Affine) -> Result<Json, String> {
        let boxes = self.boxes(&self.selection.clone());
        let mut all: Option<Rect> = None;
        for (_, _, c) in &boxes {
            for p in c {
                all = Some(all.map_or(Rect::from_points(*p, *p), |r| r.union_pt(*p)));
            }
        }
        let Some(all) = all else { return Ok(Json::Null) };
        let pivot = all.center().to_vec2();
        let x = Affine::translate(pivot) * m * Affine::translate(-pivot);
        let mut cmds = Vec::new();
        for (id, b, _) in boxes {
            let snap =
                xenify_ops::transform::Snapshot::new(self.project(), self.comp, id, self.time, b).map_err(err)?;
            cmds.extend(snap.affine_commands(self.project(), x, self.time, self.auto_key).map_err(err)?);
        }
        self.exec_all(label, cmds)?;
        Ok(Json::Null)
    }

    fn turn(&mut self, args: &Json) -> Result<Json, String> {
        let d = f(args, "degrees").unwrap_or(90.0);
        self.about_centre("Rotate", Affine::rotate(d.to_radians()))
    }

    fn flip(&mut self, args: &Json) -> Result<Json, String> {
        let v = args.get("axis").and_then(Json::as_str) == Some("vertical");
        self.about_centre(
            if v { "Flip vertically" } else { "Flip horizontally" },
            if v { Affine::FLIP_Y } else { Affine::FLIP_X },
        )
    }

    /// Rectangles, ellipses, polygons and stars of the selection become paths.
    fn convert_to_path(&mut self) -> Result<Json, String> {
        let mut cmds = Vec::new();
        for &id in &self.selection {
            let Ok(l) = self.project().layer(id) else { continue };
            for (i, s) in l.shapes().iter().enumerate() {
                if matches!(s.geometry, Geometry::Path { .. }) {
                    continue;
                }
                let baked = ShapeItem { geometry: primitives::to_path_geometry(&s.geometry), ..s.clone() };
                cmds.push(Command::SetShape { layer: id, index: i, shape: baked });
            }
        }
        if cmds.is_empty() {
            return Err("Nothing to convert: select rectangles, ellipses or polygons.".into());
        }
        self.exec_all("Convert to path", cmds)?;
        Ok(Json::Null)
    }

    /// Applies `f` to every path of the selected layers (live shapes are
    /// converted first), one undo step. Answers the point counts before and after.
    fn each_path(&mut self, label: &str, op: impl Fn(&BezPath) -> BezPath) -> Result<Json, String> {
        let (mut before, mut after) = (0, 0);
        let mut cmds = Vec::new();
        for id in self.selection.clone() {
            let Ok(layer) = self.project().layer(id).cloned() else { continue };
            for (i, s) in layer.shapes().iter().enumerate() {
                let Some(p) = shape_path(s, self.time) else { continue };
                let new = op(&p);
                before += bezier::point_count(&p);
                after += bezier::point_count(&new);
                if new == p {
                    continue;
                }
                if !matches!(s.geometry, Geometry::Path { .. }) {
                    let baked = ShapeItem { geometry: Geometry::Path { path: new.into() }, ..s.clone() };
                    cmds.push(Command::SetShape { layer: id, index: i, shape: baked });
                    continue;
                }
                let prop = PropPath::shape(i, ShapeField::Path);
                cmds.push(
                    Command::edit_value(self.project(), id, prop, self.time, Value::Path(new), self.auto_key)
                        .map_err(err)?,
                );
            }
        }
        if before == 0 {
            return Err("Select shapes to change their points.".into());
        }
        self.exec_all(label, cmds)?;
        Ok(json!({ "before": before, "after": after }))
    }

    fn outline_stroke(&mut self) -> Result<Json, String> {
        let cmd =
            xenify_ops::outline::outline_strokes(self.project(), self.comp, &self.selection, self.time).map_err(err)?;
        self.exec("Outline stroke", cmd)?;
        Ok(Json::Null)
    }

    /// The selected layers as text for the clipboard (their groups' members too).
    fn copy(&self) -> Result<Json, String> {
        let Ok(c) = self.project().comp(self.comp) else { return Ok(Json::Null) };
        let mut take: Vec<Id> = self.selection.clone();
        let mut i = 0;
        while i < take.len() {
            let parent = take[i];
            for l in &c.layers {
                if l.meta.parent == Some(parent) && !take.contains(&l.id) {
                    take.push(l.id);
                }
            }
            i += 1;
        }
        let layers: Vec<&Layer> = c.layers.iter().filter(|l| take.contains(&l.id)).collect();
        if layers.is_empty() {
            return Ok(Json::Null);
        }
        let assets: Vec<&xenify_doc::ImageAsset> =
            self.project().assets.iter().filter(|a| layers.iter().any(|l| l.asset_refs().contains(&a.id))).collect();
        Ok(json!(serde_json::to_string(&json!({ "xenify_draw_layers": layers, "assets": assets })).map_err(err)?))
    }

    /// Layers copied with [`Self::copy`] (from this canvas or another), with new ids, nudged when pasted over themselves.
    fn paste(&mut self, args: &Json) -> Result<Json, String> {
        let text = args.get("text").and_then(Json::as_str).ok_or("nothing to paste")?;
        let v: Json = serde_json::from_str(text).map_err(|_| "nothing to paste")?;
        let layers: Vec<Layer> =
            serde_json::from_value(v.get("xenify_draw_layers").cloned().ok_or("nothing to paste")?).map_err(err)?;
        let assets: Vec<xenify_doc::ImageAsset> =
            serde_json::from_value(v.get("assets").cloned().unwrap_or(json!([]))).unwrap_or_default();
        let mut p = self.project().clone();
        let mut cmds = Vec::new();
        for a in assets {
            if p.asset(a.id).is_none() {
                cmds.push(Command::AddAsset { asset: a });
            }
        }
        let mut map = std::collections::HashMap::new();
        for l in &layers {
            map.insert(l.id, p.alloc_id());
        }
        let here =
            self.project().comp(self.comp).map(|c| layers.iter().any(|l| c.layer(l.id).is_some())).unwrap_or(false);
        let mut tops = Vec::new();
        for mut l in layers {
            let old = l.id;
            l.id = map[&old];
            l.meta.parent = l.meta.parent.and_then(|p| map.get(&p).copied());
            if l.meta.parent.is_none() {
                tops.push(l.id);
                if here {
                    l.transform.position = match l.transform.position {
                        xenify_doc::Property::Static(v) => xenify_doc::Property::Static(v + Vec2::new(20.0, 20.0)),
                        other => other,
                    };
                }
            }
            cmds.push(Command::AddLayer { comp: self.comp, index: None, layer: l });
        }
        self.exec_all("Paste", crate::adopt_after(cmds))?;
        self.selection = tops;
        Ok(Json::Null)
    }

    /// The mask tool: a rectangle or ellipse mask on layer `id` filling the
    /// box between `a` and `b` (composition space). Answers the mask's index.
    fn add_mask_at(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let a = args.get("a").and_then(pt).ok_or("missing a")?;
        let b = args.get("b").and_then(pt).ok_or("missing b")?;
        let inv = self.world(id).inverse();
        let corners =
            [Point::new(a.x, a.y), Point::new(b.x, a.y), Point::new(b.x, b.y), Point::new(a.x, b.y)].map(|p| inv * p);
        let r = corners.iter().fold(Rect::from_points(corners[0], corners[0]), |r, p| r.union_pt(*p));
        let mut meta = self.project().layer(id).map_err(err)?.meta.clone();
        let n = meta.masks.len();
        let ellipse = args.get("ellipse").and_then(Json::as_bool).unwrap_or(false);
        let name = format!("Mask {}", n + 1);
        meta.masks.push(if ellipse { xenify_doc::Mask::ellipse(name, r) } else { xenify_doc::Mask::rect(name, r) });
        self.exec(
            if ellipse { "Draw ellipse mask" } else { "Draw rectangle mask" },
            Command::SetLayerMeta { layer: id, meta },
        )?;
        Ok(json!(n))
    }

    // ----- Edit Points -----

    fn world(&self, id: Id) -> Affine {
        self.project().comp(self.comp).map_or(Affine::IDENTITY, |c| xenify_anim::world_transform(c, id, self.time))
    }

    /// A layer's shapes' points in composition space: per shape, whether it's
    /// closed, and each point with its handles.
    fn points(&self, id: Option<Id>) -> Result<Json, String> {
        let id = id.ok_or("select a shape")?;
        let l = self.project().layer(id).map_err(err)?;
        if !self.on_masks && !matches!(l.kind, LayerKind::Shape { .. }) {
            return Err(match l.kind {
                LayerKind::Text(_) => "Words have no points to edit: use Select to move or size them.".into(),
                _ => "Only shapes have points to edit.".into(),
            });
        }
        let w = self.world(id);
        let shapes: Vec<Json> = self
            .outlines(id)
            .into_iter()
            .map(|(i, p, live)| {
                let anchors: Vec<Json> = bezier::anchors(&p)
                    .iter()
                    .map(|a| {
                        let hin = a.handle_in.map(|h| pj(w * h.1));
                        let hout = a.handle_out.map(|h| pj(w * h.1));
                        json!({ "p": pj(w * a.pos), "in": hin, "out": hout })
                    })
                    .collect();
                // The outline itself, for drawing it and hitting edges: one list of segments.
                let segs: Vec<Json> = p
                    .segments()
                    .map(|seg| match seg {
                        kurbo::PathSeg::Line(l) => json!([pj(w * l.p0), pj(w * l.p1)]),
                        kurbo::PathSeg::Quad(q) => {
                            let c = q.raise();
                            json!([pj(w * c.p0), pj(w * c.p1), pj(w * c.p2), pj(w * c.p3)])
                        }
                        kurbo::PathSeg::Cubic(c) => json!([pj(w * c.p0), pj(w * c.p1), pj(w * c.p2), pj(w * c.p3)]),
                    })
                    .collect();
                let ends: Vec<Json> = bezier::open_ends(&p)
                    .iter()
                    .map(|(sub, start, at)| json!({ "subpath": sub, "from_start": start, "p": pj(w * *at) }))
                    .collect();
                json!({ "shape": i, "anchors": anchors, "segments": segs, "ends": ends, "live": live })
            })
            .collect();
        Ok(json!({ "id": id.0, "shapes": shapes }))
    }

    /// The outlines the point tools edit: each shape's (live ones as paths), or with `on_masks` each mask's.
    fn outlines(&self, id: Id) -> Vec<(usize, BezPath, bool)> {
        let Ok(l) = self.project().layer(id) else { return Vec::new() };
        if self.on_masks {
            return l.meta.masks.iter().enumerate().map(|(i, m)| (i, m.path.value_at(self.time), false)).collect();
        }
        l.shapes()
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i, shape_path(s, self.time)?, !matches!(s.geometry, Geometry::Path { .. }))))
            .collect()
    }

    fn current_path(&self, id: Id, shape: usize) -> Option<BezPath> {
        let l = self.project().layer(id).ok()?;
        if self.on_masks {
            return Some(l.meta.masks.get(shape)?.path.value_at(self.time));
        }
        shape_path(l.shapes().get(shape)?, self.time)
    }

    /// Replaces a shape's outline now (a live rectangle or ellipse becomes a
    /// path first); merged into the gesture's undo step.
    fn set_shape_path(&mut self, id: Id, shape: usize, new: BezPath, label: &str) -> Result<(), String> {
        let key = format!("points:{}", self.gesture_seq);
        if self.on_masks {
            let path = PropPath::Mask { index: shape, field: xenify_doc::MaskField::Path };
            let cmd = Command::edit_value(self.project(), id, path, self.time, Value::Path(new), self.auto_key)
                .map_err(err)?;
            return self.editor.execute_merged(label, key, cmd).map_err(err);
        }
        let live = self
            .project()
            .layer(id)
            .ok()
            .and_then(|l| l.shapes().get(shape).filter(|s| !matches!(s.geometry, Geometry::Path { .. })).cloned());
        if let Some(item) = live {
            let baked = ShapeItem { geometry: primitives::to_path_geometry(&item.geometry), ..item };
            self.editor
                .execute_merged(label, key.clone(), Command::SetShape { layer: id, index: shape, shape: baked })
                .map_err(err)?;
        }
        let cmd = Command::edit_value(
            self.project(),
            id,
            PropPath::shape(shape, ShapeField::Path),
            self.time,
            Value::Path(new),
            self.auto_key,
        )
        .map_err(err)?;
        self.editor.execute_merged(label, key, cmd).map_err(err)
    }

    /// Moves points by a composition-space offset from where they were at
    /// `points_begin` (`start` is the shapes' outlines then, kept by the screens as the call's first answer).
    fn points_move(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let d = Vec2::new(f(args, "dx").unwrap_or(0.0), f(args, "dy").unwrap_or(0.0));
        let sel = point_list(args);
        let inv = self.world(id).inverse();
        let local = inv * d.to_point() - inv * Point::ZERO;
        let starts = self.gesture_paths(id, &sel);
        for (shape, path) in starts {
            let idx: Vec<usize> = sel.iter().filter(|(s, _)| *s == shape).map(|(_, i)| *i).collect();
            self.set_shape_path(id, shape, bezier::move_nodes(&path, &idx, local), "Move points")?;
        }
        Ok(Json::Null)
    }

    /// The outlines of `sel`'s shapes as they were when this gesture began.
    fn gesture_paths(&mut self, id: Id, sel: &[(usize, usize)]) -> Vec<(usize, BezPath)> {
        if self.point_start.0 != self.gesture_seq || self.point_start.1 != id {
            let mut shapes: Vec<usize> = sel.iter().map(|(s, _)| *s).collect();
            shapes.sort_unstable();
            shapes.dedup();
            let paths = shapes.into_iter().filter_map(|s| Some((s, self.current_path(id, s)?))).collect();
            self.point_start = (self.gesture_seq, id, paths);
        }
        self.point_start.2.clone()
    }

    fn handle_move(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let shape = f(args, "shape").unwrap_or(0.0) as usize;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        let incoming = args.get("incoming").and_then(Json::as_bool).unwrap_or(false);
        let mirror = args.get("mirror").and_then(Json::as_bool).unwrap_or(true);
        let to = self.world(id).inverse() * args.get("to").and_then(pt).ok_or("missing to")?;
        let p = self.current_path(id, shape).ok_or("no such shape")?;
        self.set_shape_path(id, shape, bezier::move_handle(&p, index, incoming, to, mirror), "Move handle")?;
        Ok(Json::Null)
    }

    /// Adds a point on the outline nearest `at` (within `tolerance`): answers `[shape, index]`, or null.
    fn point_insert(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let w = self.world(id);
        let at = w.inverse() * args.get("at").and_then(pt).ok_or("missing at")?;
        let tol = f(args, "tolerance").unwrap_or(6.0) / w.determinant().abs().sqrt().max(1e-9);
        let best = self
            .outlines(id)
            .into_iter()
            .filter_map(|(i, p, _)| {
                let hit = bezier::nearest(&p, at)?;
                Some((i, p, hit))
            })
            .min_by(|a, b| a.2.distance.total_cmp(&b.2.distance));
        let Some((shape, path, hit)) = best.filter(|b| b.2.distance <= tol) else { return Ok(Json::Null) };
        let Some((new, index)) = bezier::insert_node(&path, hit) else { return Ok(Json::Null) };
        self.gesture_seq += 1;
        self.set_shape_path(id, shape, new, "Add point")?;
        Ok(json!([shape, index]))
    }

    /// Shape `index` of `id` as the Layer panel's shape controls see it: a rectangle (its box and four corner
    /// radii; a live one or a path still shaped like one), an ellipse (its box), or a path (how many points).
    fn smart_shape(&self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        let l = self.project().layer(id).map_err(err)?;
        let item = l.shapes().get(index).ok_or("no such shape")?;
        let t = self.time;
        let r = |r: kurbo::Rect| [r.x0, r.y0, r.x1, r.y1];
        Ok(match &item.geometry {
            Geometry::Rect { position, size, corner_radius } => {
                let rect =
                    kurbo::Rect::from_center_size(position.value_at(t).to_point(), size.value_at(t).to_size()).abs();
                let c = corner_radius.value_at(t);
                json!({ "kind": "rect", "rect": r(rect), "radii": [c, c, c, c], "live": true })
            }
            Geometry::Path { path } => {
                let p = path.value_at(t);
                match primitives::recognize(&p) {
                    Some(primitives::Primitive::Rect { rect, radii }) => {
                        json!({ "kind": "rect", "rect": r(rect), "radii": radii, "animated": path.is_animated() })
                    }
                    Some(primitives::Primitive::Ellipse { rect }) => {
                        json!({ "kind": "ellipse", "rect": r(rect), "animated": path.is_animated() })
                    }
                    None => {
                        json!({ "kind": "path", "points": bezier::point_count(&p), "animated": path.is_animated() })
                    }
                }
            }
            _ => Json::Null,
        })
    }

    /// Sets a rectangle's corner radii (`radii`, four) or size (`w`, `h`), or simplifies a path (`simplify`: how far
    /// it may stray, from the outline as the gesture began).
    fn set_smart_shape(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let index = f(args, "index").unwrap_or(0.0) as usize;
        if let Some(tol) = f(args, "simplify") {
            let start = self
                .gesture_paths(id, &[(index, 0)])
                .into_iter()
                .find(|(s, _)| *s == index)
                .map(|(_, p)| p)
                .ok_or("no such shape")?;
            let simple = bezier::simplify(&start, tol.max(0.01));
            let n = bezier::point_count(&simple);
            self.set_shape_path(id, index, simple, "Simplify path")?;
            return Ok(json!(n));
        }
        let info = self.smart_shape(args)?;
        let rect = info["rect"]
            .as_array()
            .map(|a| a.iter().filter_map(Json::as_f64).collect::<Vec<_>>())
            .filter(|v| v.len() == 4)
            .ok_or("not a rectangle or ellipse")?;
        let mut rect = kurbo::Rect::new(rect[0], rect[1], rect[2], rect[3]);
        if let (Some(w), Some(h)) = (f(args, "w"), f(args, "h")) {
            rect = kurbo::Rect::from_center_size(rect.center(), (w.max(0.01), h.max(0.01)));
        }
        let path = if info["kind"] == "ellipse" {
            primitives::ellipse_path(rect)
        } else {
            let old: Vec<f64> =
                info["radii"].as_array().map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
            let radii = args
                .get("radii")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(Json::as_f64).collect::<Vec<_>>())
                .unwrap_or(old);
            let r = |i: usize| radii.get(i).copied().unwrap_or(0.0).max(0.0);
            primitives::rect_path(rect, [r(0), r(1), r(2), r(3)])
        };
        self.gesture_seq += 1;
        self.set_shape_path(id, index, path, "Shape")?;
        Ok(Json::Null)
    }

    /// The sharp corners of `id`'s outlines, to round by dragging: shape, point index, where it is
    /// and the way into the corner (both on the canvas), and the shorter of its two edges.
    fn corners(&self, id: Option<Id>) -> Json {
        let Some(id) = id else { return json!([]) };
        let w = self.world(id);
        let lin = Affine::new({
            let c = w.as_coeffs();
            [c[0], c[1], c[2], c[3], 0.0, 0.0]
        });
        let mut out = Vec::new();
        for (shape, path, _) in self.outlines(id) {
            let mut flat = 0;
            for sp in bezier::to_nodes(&path) {
                let n = sp.nodes.len();
                for (k, node) in sp.nodes.iter().enumerate() {
                    let index = flat + k;
                    let inner = sp.closed || (k > 0 && k + 1 < n);
                    if !inner || n < 3 || node.handle_in.is_some() || node.handle_out.is_some() {
                        continue;
                    }
                    let prev = sp.nodes[(k + n - 1) % n];
                    let next = sp.nodes[(k + 1) % n];
                    let (a, b) = (
                        (lin * (prev.handle_out.unwrap_or(prev.pos) - node.pos).to_point()).to_vec2(),
                        (lin * (next.handle_in.unwrap_or(next.pos) - node.pos).to_point()).to_vec2(),
                    );
                    let edge = a.hypot().min(b.hypot());
                    let (a, b) = (a.normalize(), b.normalize());
                    let cos = a.dot(b).clamp(-1.0, 1.0);
                    // Nearly straight, or a spike: nothing to round.
                    if !(-0.996..=0.996).contains(&cos) {
                        continue;
                    }
                    let dir = (a + b).normalize();
                    let at = w * node.pos;
                    out.push(json!({ "shape": shape, "index": index, "at": [at.x, at.y], "dir": [dir.x, dir.y], "edge": edge, "cos_half": ((1.0 + cos) / 2.0).sqrt() }));
                }
                flat += n;
            }
        }
        json!(out)
    }

    /// Rounds corner `index` of shape `shape` (or, without `index`, every corner) by `radius`,
    /// from the outline as it was when the drag began.
    fn corner_round(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let shape = f(args, "shape").unwrap_or(0.0) as usize;
        let radius = f(args, "radius").unwrap_or(0.0).max(0.0);
        let start = self
            .gesture_paths(id, &[(shape, 0)])
            .into_iter()
            .find(|(s, _)| *s == shape)
            .map(|(_, p)| p)
            .ok_or("no such shape")?;
        let idx: Vec<usize> = match args.get("index").and_then(Json::as_u64) {
            Some(i) => vec![i as usize],
            None => self
                .corners(Some(id))
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|c| c["shape"] == json!(shape))
                        .filter_map(|c| c["index"].as_u64().map(|i| i as usize))
                        .collect()
                })
                .unwrap_or_default(),
        };
        let new = if radius < 1e-6 { start } else { bezier::bevel(&start, &idx, radius, true) };
        self.set_shape_path(id, shape, new, "Round corners")?;
        Ok(Json::Null)
    }

    /// The point tools: smooth (toggle), delete, break, join, subdivide,
    /// extrude, bevel, round, merge, simplify. Answers the points to select after.
    fn points_edit(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let op = args.get("op").and_then(Json::as_str).unwrap_or("");
        let sel = point_list(args);
        let amount = f(args, "amount").unwrap_or(10.0);
        let n = self.outlines(id).iter().map(|o| o.0 + 1).max().unwrap_or(0);
        self.gesture_seq += 1;
        let mut after: Vec<(usize, usize)> = Vec::new();
        let mut changed = false;
        for shape in 0..n {
            let Some(path) = self.current_path(id, shape) else { continue };
            let idx: Vec<usize> = sel.iter().filter(|(s, _)| *s == shape).map(|(_, i)| *i).collect();
            let new = match op {
                "smooth" => {
                    let mut out = path.clone();
                    for i in &idx {
                        out = bezier::toggle_smooth(&out, *i).unwrap_or(out);
                    }
                    after.extend(idx.iter().map(|i| (shape, *i)));
                    Some(out)
                }
                "delete" if !idx.is_empty() => Some(bezier::delete_nodes(&path, &idx)),
                "break" => idx.first().and_then(|i| bezier::break_at(&path, *i)),
                "join" => match idx.as_slice() {
                    [a, b] => bezier::join(&path, *a, *b),
                    _ => None,
                },
                "subdivide" => Some(bezier::subdivide(&path, &idx)),
                "bevel" | "round" => Some(bezier::bevel(&path, &idx, amount, op == "round")),
                "merge" => Some(bezier::merge_by_distance(&path, &idx, amount)),
                "simplify" => Some(bezier::simplify(&path, amount)),
                "extrude" => match idx.as_slice() {
                    [i] => {
                        let a = bezier::anchors(&path);
                        let here = a.get(*i).map_or(Point::ZERO, |a| a.pos);
                        let nb = if *i == 0 { a.get(1) } else { a.get(i - 1) }.map_or(here, |a| a.pos);
                        let dir = (here - nb).normalize();
                        let dir = if dir.is_finite() { dir } else { Vec2::new(1.0, 0.0) };
                        bezier::extrude(&path, *i, dir * amount).map(|(p, j)| {
                            after.push((shape, j));
                            p
                        })
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(new) = new.filter(|p| *p != path) {
                self.set_shape_path(id, shape, new, "Edit points")?;
                changed = true;
            }
        }
        if !changed {
            return Err(match op {
                "join" => "Select the two loose ends of a line to join them.".into(),
                "break" => "Select one point to break the outline there.".into(),
                "extrude" => "Select one end point of an open line.".into(),
                "delete" => "Select points to delete.".into(),
                _ => "Nothing changed.".into(),
            });
        }
        Ok(json!(after.iter().map(|(s, i)| [*s, *i]).collect::<Vec<_>>()))
    }
}

/// A shape's outline now (a rectangle's, ellipse's or polygon's as a path).
pub(crate) fn shape_path(s: &ShapeItem, time: f64) -> Option<BezPath> {
    match &s.geometry {
        Geometry::Path { path } => Some(path.value_at(time)),
        live => primitives::live_path(live, time),
    }
}
