//! Snapping and guides: what a moving selection lines up with (the frame's
//! edges and middle, other layers' edges and middles, guides, the grid),
//! and the guides dragged out of the rulers.

use serde_json::{Value as Json, json};
use xenify_doc::{GuideLine, Id};
use xenify_ops::Command;

use crate::{Session, err, f};

impl Session {
    pub(crate) fn snap_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "snap_targets" => Ok(self.snap_targets(args)),
            "doc_colors" => Ok(self.doc_colors()),
            "guides" => Ok(json!({
                "guides": self.comp_settings().map(|s| s.guides.clone()).unwrap_or_default(),
                "grid": self.comp_settings().map_or(50.0, |s| s.grid_spacing()),
            })),
            "set_guides" => serde_json::from_value::<Vec<GuideLine>>(args.get("guides").cloned().unwrap_or(json!([])))
                .map_err(err)
                .and_then(|guides| {
                    let mut s = self.comp_settings().cloned().ok_or("no canvas")?;
                    s.guides = guides;
                    if let Some(g) = f(args, "grid") {
                        s.grid = g.max(0.0);
                    }
                    let comp = self.comp;
                    self.editor
                        .execute_merged("Guides", "guides", Command::SetCompSettings { comp, settings: s })
                        .map_err(err)
                        .map(|_| Json::Null)
                }),
            _ => return None,
        })
    }

    /// The colours used on this canvas (fills, strokes, text, gradient stops), most used first.
    fn doc_colors(&self) -> Json {
        let mut count: std::collections::HashMap<String, usize> = Default::default();
        let mut add = |c: xenify_doc::Color| *count.entry(c.to_hex()).or_default() += 1;
        if let Ok(c) = self.project().comp(self.comp) {
            for l in &c.layers {
                for s in l.shapes() {
                    if let Some(fl) = &s.fill {
                        add(fl.color.value_at(self.time));
                        if let Some(g) = &fl.gradient {
                            g.stops.value_at(self.time).iter().for_each(|st| add(st.color));
                        }
                    }
                    if let Some(st) = &s.stroke {
                        add(st.color.value_at(self.time));
                    }
                }
                if let xenify_doc::LayerKind::Text(t) = &l.kind {
                    add(t.color.value_at(self.time));
                }
            }
        }
        let mut v: Vec<(String, usize)> = count.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        json!(v.into_iter().take(32).map(|(c, _)| c).collect::<Vec<_>>())
    }

    /// The box of the layers `ids` (default: the selection) and the lines
    /// to snap to: `objects` (frame and other layers), `guides`, `grid`,
    /// each `margin` pixels away too.
    fn snap_targets(&mut self, args: &Json) -> Json {
        let on = |k: &str| args.get(k).and_then(Json::as_bool).unwrap_or(true);
        let ids: Vec<Id> = match args.get("ids").and_then(Json::as_array) {
            Some(a) => a.iter().filter_map(Json::as_u64).map(Id).collect(),
            None => self.selection.clone(),
        };
        let Some(s) = self.comp_settings().cloned() else { return Json::Null };
        let Ok(c) = self.project().comp(self.comp).cloned() else { return Json::Null };
        let moving: Vec<Id> =
            ids.iter().flat_map(|id| std::iter::once(*id).chain(xenify_anim::descendants(&c, *id))).collect();
        let scene = self.scene();
        let (w, h) = (s.width as f64, s.height as f64);
        let m = f(args, "margin").unwrap_or(0.0).max(0.0);
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        let mut start: Option<kurbo::Rect> = None;
        if on("objects") {
            xs.extend([0.0, w / 2.0, w]);
            ys.extend([0.0, h / 2.0, h]);
            if m > 0.0 {
                xs.extend([m, w - m]);
                ys.extend([m, h - m]);
            }
        }
        for l in scene.layers.iter().filter(|l| l.opacity > 0.02) {
            let Some(b) = l.bounds(&mut self.text).map(|b| l.transform.transform_rect_bbox(b)) else { continue };
            if moving.contains(&l.id) {
                start = Some(start.map_or(b, |s| s.union(b)));
            } else if on("objects") && (b.width() < w * 0.95 || b.height() < h * 0.95) {
                xs.extend([b.x0, b.center().x, b.x1]);
                ys.extend([b.y0, b.center().y, b.y1]);
                if m > 0.0 {
                    xs.extend([b.x0 - m, b.x1 + m]);
                    ys.extend([b.y0 - m, b.y1 + m]);
                }
            }
        }
        if on("guides") {
            for g in &s.guides {
                if g.vertical { xs.push(g.pos) } else { ys.push(g.pos) }
            }
        }
        if args.get("grid").and_then(Json::as_bool).unwrap_or(false) {
            let step = s.grid_spacing();
            if step > 0.0 {
                xs.extend((0..=(w / step) as usize).map(|i| i as f64 * step));
                ys.extend((0..=(h / step) as usize).map(|i| i as f64 * step));
            }
        }
        json!({ "box": start.map(|b| [b.x0, b.y0, b.x1, b.y1]), "xs": xs, "ys": ys })
    }
}
