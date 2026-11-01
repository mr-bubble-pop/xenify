//! More pixel edits on raster layers: moving and scaling the selected pixels
//! (with a live preview), gradients, colour adjustments, a selection from an
//! AI-drawn mask, copying pixels out, and changing the picture's size or
//! extending it past its edges.

use std::sync::Arc;

use base64::Engine as _;
use kurbo::{Point, Vec2 as KVec};
use serde_json::{Value as Json, json};
use xenify_doc::{Color, Id, Property, Vec2};
use xenify_ops::Command;

use super::{Buf, b64, blend_px, bounds, encode_png, rgb};
use crate::{Session, err, f, id_arg};

/// A change shown live before it's kept: the pixels and selection it started from.
pub(super) struct Edit {
    pub layer: Id,
    base: Vec<u8>,
    base_sel: Option<Vec<u8>>,
    kind: EditKind,
}

enum EditKind {
    /// The selected pixels lifted off (premultiplied, with their selection), from box `from` (x0, y0, x1, y1).
    /// `lifted` and `mask` cover only box `from` (x0, y0, x1, y1); `under`: what's left where they were lifted from.
    Float {
        lifted: Vec<[f32; 4]>,
        mask: Vec<u8>,
        under: Vec<u8>,
        from: [f64; 4],
        shown: Option<(i64, i64, i64, i64)>,
    },
    Adjust,
}

impl Session {
    pub(super) fn raster_ops_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "raster_float_begin" => self.raster_float_begin(args),
            "raster_float_set" => self.raster_float_set(args),
            "raster_adjust" => self.raster_adjust(args),
            "raster_live_end" => self.raster_live_end(args.get("keep").and_then(Json::as_bool).unwrap_or(true)),
            "raster_gradient" => self.raster_gradient(args),
            "raster_select_mask" => self.raster_select_mask(args),
            "raster_copy" => self.raster_copy(args),
            "raster_resize" => self.raster_resize(args),
            "raster_extend" => self.raster_extend(args),
            "raster_sel_at" => self.raster_sel_at(args),
            _ => return None,
        })
    }

    /// Starts a live change on layer `id`: keeps its pixels and selection to go back to.
    fn live_begin(&mut self, id: Id, kind: EditKind) -> Result<(), String> {
        self.raster_live_end(false)?;
        let b = self.buf(id)?;
        let (base, base_sel) = (b.px.as_ref().clone(), b.sel.clone());
        xenify_render::images::set_live(b.hash, b.w, b.h, b.px.clone());
        self.raster.edit = Some(Edit { layer: id, base, base_sel, kind });
        self.raster.dirty = None;
        Ok(())
    }

    /// Ends the live change: keeps it (one undo step) or puts things back.
    pub(super) fn raster_live_end(&mut self, keep: bool) -> Result<Json, String> {
        let Some(e) = self.raster.edit.take() else { return Ok(Json::Null) };
        let Some(b) = self.raster.bufs.get_mut(&e.layer) else { return Ok(Json::Null) };
        let changed = b.px.as_slice() != e.base.as_slice();
        if b.sel.as_ref().is_some_and(|s| s.iter().all(|v| *v == 0)) {
            b.sel = None;
        }
        if keep && changed {
            let label = match e.kind {
                EditKind::Float { .. } => "Move pixels",
                EditKind::Adjust => "Adjust colours",
            };
            self.commit(e.layer, label)?;
            return Ok(json!(true));
        }
        xenify_render::images::clear_live(b.hash);
        b.px = Arc::new(e.base);
        b.sel = e.base_sel;
        b.outline = None;
        Ok(json!(false))
    }

    /// Whether canvas point (`x`, `y`) is inside layer `id`'s selection.
    fn raster_sel_at(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let p = self.to_canvas(id)?.inverse() * Point::new(f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
        let b = self.buf(id)?;
        let (x, y) = (p.x.floor(), p.y.floor());
        if x < 0.0 || y < 0.0 || x >= b.w as f64 || y >= b.h as f64 {
            return Ok(json!(false));
        }
        Ok(json!(b.sel.as_ref().is_some_and(|s| s[(y as u32 * b.w + x as u32) as usize] > 127)))
    }

    // ----- Moving and scaling the selected pixels -----

    /// Lifts the selected pixels off layer `id` (they move with raster_float_set until raster_live_end).
    fn raster_float_begin(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let b = self.buf(id)?;
        let sel = b.sel.clone().ok_or("Select the pixels to move first.")?;
        let [x0, y0, x1, y1] = bounds(&sel, b.w, b.h).ok_or("Nothing is selected.")?;
        let w = b.w;
        // Only the selection's box is lifted.
        let (bw, bh) = (x1 - x0, y1 - y0);
        let mut lifted = Vec::with_capacity((bw * bh) as usize);
        let mut mask = Vec::with_capacity((bw * bh) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w + x) as usize;
                lifted.push(premul_m(&b.px[i * 4..i * 4 + 4], sel[i] as f32 / 255.0));
                mask.push(sel[i]);
            }
        }
        // What's left behind: the selected pixels' place, see-through.
        let mut under = b.px.as_ref().clone();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w + x) as usize;
                under[i * 4 + 3] = (under[i * 4 + 3] as u32 * (255 - sel[i] as u32) / 255) as u8;
            }
        }
        let from = [x0 as f64, y0 as f64, x1 as f64, y1 as f64];
        self.live_begin(
            id,
            EditKind::Float { lifted, mask, under, from, shown: Some((x0 as i64, y0 as i64, x1 as i64, y1 as i64)) },
        )?;
        self.raster.bufs.get_mut(&id).expect("loaded").sel = None;
        self.raster_float_set(&json!({"dx": 0, "dy": 0, "sx": 1, "sy": 1}))?;
        Ok(json!([x0, y0, x1, y1]))
    }

    /// Where the lifted pixels are now: moved by (`dx`, `dy`) canvas units and scaled by `sx`, `sy` about their box's centre.
    fn raster_float_set(&mut self, args: &Json) -> Result<Json, String> {
        let layer = self
            .raster
            .edit
            .as_ref()
            .filter(|e| matches!(e.kind, EditKind::Float { .. }))
            .ok_or("nothing lifted")?
            .layer;
        let to_px = self.to_canvas(layer)?.inverse();
        let d = to_px * Point::new(f(args, "dx").unwrap_or(0.0), f(args, "dy").unwrap_or(0.0)) - to_px * Point::ZERO;
        let (sx, sy) = (f(args, "sx").unwrap_or(1.0), f(args, "sy").unwrap_or(1.0));
        let (sx, sy) = (if sx.abs() < 1e-3 { 1e-3 } else { sx }, if sy.abs() < 1e-3 { 1e-3 } else { sy });
        // Just moved: whole pixels, copied (no resampling, and quicker).
        let plain = (sx - 1.0).abs() < 1e-9 && (sy - 1.0).abs() < 1e-9;
        let d = if plain { KVec::new(d.x.round(), d.y.round()) } else { d };
        let e = self.raster.edit.as_mut().expect("checked");
        let b = self.raster.bufs.get_mut(&layer).ok_or("the layer went away")?;
        let (w, h) = (b.w, b.h);
        let EditKind::Float { lifted, mask, under, from, shown } = &mut e.kind else { unreachable!() };
        let c = KVec::new((from[0] + from[2]) / 2.0, (from[1] + from[3]) / 2.0);
        // Pixel q shows lifted pixel c + (q − d − c) / s.
        let back = |q: Point| Point::new(c.x + (q.x - d.x - c.x) / sx, c.y + (q.y - d.y - c.y) / sy);
        let corners = [(from[0], from[1]), (from[2], from[1]), (from[0], from[3]), (from[2], from[3])]
            .map(|(x, y)| Point::new(c.x + (x - c.x) * sx + d.x, c.y + (y - c.y) * sy + d.y));
        let clampx = |v: f64| (v as i64).clamp(0, w as i64);
        let clampy = |v: f64| (v as i64).clamp(0, h as i64);
        let now = (
            clampx(corners.iter().map(|p| p.x).fold(f64::MAX, f64::min).floor() - 1.0),
            clampy(corners.iter().map(|p| p.y).fold(f64::MAX, f64::min).floor() - 1.0),
            clampx(corners.iter().map(|p| p.x).fold(f64::MIN, f64::max).ceil() + 1.0),
            clampy(corners.iter().map(|p| p.y).fold(f64::MIN, f64::max).ceil() + 1.0),
        );
        let area = match *shown {
            Some(o) => (o.0.min(now.0), o.1.min(now.1), o.2.max(now.2), o.3.max(now.3)),
            None => (0, 0, w as i64, h as i64),
        };
        let (fx, fy) = (from[0], from[1]);
        let (fw, fh) = ((from[2] - from[0]) as u32, (from[3] - from[1]) as u32);
        *shown = Some(now);
        let px = Arc::make_mut(&mut b.px);
        let mut sel = b.sel.take().unwrap_or_else(|| vec![0; (w * h) as usize]);
        for y in area.1..area.3 {
            for x in area.0..area.2 {
                let i = (y as u32 * w + x as u32) as usize;
                px[i * 4..i * 4 + 4].copy_from_slice(&under[i * 4..i * 4 + 4]);
                sel[i] = 0;
                if x < now.0 || x >= now.2 || y < now.1 || y >= now.3 {
                    continue;
                }
                let (v, m) = if plain {
                    let (lx, ly) = (x - d.x as i64 - fx as i64, y - d.y as i64 - fy as i64);
                    if lx < 0 || ly < 0 || lx >= fw as i64 || ly >= fh as i64 {
                        continue;
                    }
                    let j = (ly as u32 * fw + lx as u32) as usize;
                    (lifted[j], mask[j])
                } else {
                    let s = back(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                    bilinear(lifted, mask, fw, fh, s.x - 0.5 - fx, s.y - 0.5 - fy)
                };
                sel[i] = m;
                if v[3] <= 0.0 {
                    continue;
                }
                // The lifted pixel over what's left.
                let o = &mut px[i * 4..i * 4 + 4];
                let under = premul_m(o, 1.0);
                let a = v[3] / 255.0;
                let out: [f32; 4] = std::array::from_fn(|k| v[k] + under[k] * (1.0 - a));
                unpremul(&out, o);
            }
        }
        b.sel = Some(sel);
        b.outline = None;
        super::grow_dirty(&mut self.raster.dirty, Some(area));
        Ok(Json::Null)
    }

    // ----- Gradient -----

    /// A gradient from canvas point `from` (colour `color`) to `to` (`color2`, "none" = see-through), linear or radial.
    fn raster_gradient(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let to_px = self.to_canvas(id)?.inverse();
        let pt = |k: &str| -> Result<Point, String> {
            let a = args.get(k).and_then(Json::as_array).ok_or(format!("missing {k}"))?;
            Ok(to_px
                * Point::new(
                    a.first().and_then(Json::as_f64).unwrap_or(0.0),
                    a.get(1).and_then(Json::as_f64).unwrap_or(0.0),
                ))
        };
        let (p0, p1) = (pt("from")?, pt("to")?);
        let col = |k: &str, dflt: Color| match args.get(k).and_then(Json::as_str) {
            Some("none") => Color::rgba(0.0, 0.0, 0.0, 0.0),
            _ => args.get(k).and_then(crate::parse_color).unwrap_or(dflt),
        };
        let (c0, c1) = (col("color", Color::BLACK), col("color2", Color::WHITE));
        let radial = args.get("kind").and_then(Json::as_str) == Some("radial");
        let opacity = f(args, "opacity").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
        let keep_alpha = args.get("keep_alpha").and_then(Json::as_bool).unwrap_or(false);
        let (a0, a1) = (c0.a as f32, c1.a as f32);
        let (r0, r1) = (rgb(c0).map(|v| v * a0), rgb(c1).map(|v| v * a1));
        let d = p1 - p0;
        let len2 = d.hypot2().max(1e-9);
        self.raster_edit(args, "Gradient", move |px, sel, (w, _)| {
            for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let q = Point::new((i as u32 % w) as f64 + 0.5, (i as u32 / w) as f64 + 0.5);
                let t = if radial { ((q - p0).hypot2() / len2).sqrt() } else { (q - p0).dot(d) / len2 }.clamp(0.0, 1.0)
                    as f32;
                let a = a0 + (a1 - a0) * t;
                let m = a * opacity * sel.map_or(1.0, |s| s[i] as f32 / 255.0);
                if m <= 0.0 {
                    continue;
                }
                let c: [f32; 3] = std::array::from_fn(|k| (r0[k] + (r1[k] - r0[k]) * t) / a.max(1e-6));
                let b = *p;
                blend_px(p, &b, m, c, false, keep_alpha);
            }
        })
    }

    // ----- Colour adjustments -----

    /// Brightness, contrast, hue, saturation and invert on the selection (or the whole layer). `preview`: shown live
    /// until raster_live_end; otherwise kept at once.
    fn raster_adjust(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        if self.raster.edit.as_ref().is_none_or(|e| e.layer != id || !matches!(e.kind, EditKind::Adjust)) {
            self.live_begin(id, EditKind::Adjust)?;
        }
        let bright = f(args, "brightness").unwrap_or(0.0).clamp(-1.0, 1.0) as f32;
        let contrast = f(args, "contrast").unwrap_or(0.0).clamp(-1.0, 1.0) as f32;
        let hue = f(args, "hue").unwrap_or(0.0) as f32;
        let sat = f(args, "saturation").unwrap_or(0.0).clamp(-1.0, 1.0) as f32;
        let invert = args.get("invert").and_then(Json::as_bool).unwrap_or(false);
        let e = self.raster.edit.as_ref().expect("begun");
        let b = self.raster.bufs.get_mut(&id).expect("loaded");
        let (w, h) = (b.w, b.h);
        let sel = e.base_sel.as_deref();
        let px = Arc::make_mut(&mut b.px);
        let k = if contrast >= 0.0 { 1.0 / (1.0 - contrast * 0.99) } else { 1.0 + contrast };
        // Only the selection's box changes.
        let [bx0, by0, bx1, by1] = sel.and_then(|s| bounds(s, w, h)).unwrap_or([0, 0, w, h]);
        for (i, (o, s)) in px.as_chunks_mut::<4>().0.iter_mut().zip(e.base.as_chunks::<4>().0).enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            if x < bx0 || x >= bx1 || y < by0 || y >= by1 {
                continue;
            }
            let m = sel.map_or(1.0, |s| s[i] as f32 / 255.0);
            if m <= 0.0 {
                o.copy_from_slice(s);
                continue;
            }
            let mut c = [s[0] as f32 / 255.0, s[1] as f32 / 255.0, s[2] as f32 / 255.0];
            if invert {
                c = c.map(|v| 1.0 - v);
            }
            if hue != 0.0 || sat != 0.0 {
                let (hh, ss, ll) = to_hsl(c);
                c = from_hsl((hh + hue / 360.0).rem_euclid(1.0), (ss * (1.0 + sat)).clamp(0.0, 1.0), ll);
            }
            c = c.map(|v| ((v - 0.5) * k + 0.5 + bright * 0.5).clamp(0.0, 1.0));
            for j in 0..3 {
                o[j] = ((s[j] as f32 * (1.0 - m) + c[j] * 255.0 * m).round()) as u8;
            }
            o[3] = s[3];
        }
        let r = sel
            .and_then(|s| bounds(s, w, h))
            .map_or((0, 0, w as i64, h as i64), |[a, b, c, d]| (a as i64, b as i64, c as i64, d as i64));
        super::grow_dirty(&mut self.raster.dirty, Some(r));
        if args.get("preview").and_then(Json::as_bool).unwrap_or(false) {
            return Ok(Json::Null);
        }
        self.raster_live_end(true)
    }

    // ----- Selections from a mask -----

    /// Selects what an AI-drawn mask (white = in) shows, fitted to layer `id`.
    fn raster_select_mask(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing mask")?)
            .map_err(err)?;
        let img = image::load_from_memory(&data).map_err(|e| format!("the mask couldn't be read: {e}"))?.into_rgba8();
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let fitted = super::fit_cover(&img, w, h);
        let mask = super::clean_mask(
            fitted
                .pixels()
                .map(|p| (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) / 256)
                .map(|l| ((l as i32 - 64) * 2).clamp(0, 255) as u8)
                .collect(),
            w,
            h,
        );
        let old = b.sel.clone();
        let new = super::combine(old, mask, super::mode_arg(args));
        Ok(self.set_sel(id, "Select with AI", new))
    }

    // ----- Copying out -----

    /// The selected pixels (or all) as a PNG cropped to them, and where they are on the canvas (centre, size).
    fn raster_copy(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let m = self.to_canvas(id)?;
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let [x0, y0, x1, y1] = match &b.sel {
            Some(s) => bounds(s, w, h).ok_or("Nothing is selected.")?,
            None => [0, 0, w, h],
        };
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut out = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w + x) as usize;
                let a = b.sel.as_ref().map_or(255, |s| s[i] as u32);
                let p = &b.px[i * 4..i * 4 + 4];
                out.extend([p[0], p[1], p[2], (p[3] as u32 * a / 255) as u8]);
            }
        }
        let c = m * Point::new((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
        let k = m.determinant().abs().sqrt();
        Ok(
            json!({ "base64": b64(&encode_png(cw, ch, &out)?), "x": c.x, "y": c.y, "width": cw, "height": ch, "scale": k }),
        )
    }

    // ----- The picture's size -----

    /// Resamples layer `id` to `width` × `height` pixels; it stays the same size on the canvas.
    fn raster_resize(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let nw = f(args, "width").ok_or("missing width")?.round().clamp(1.0, 16384.0) as u32;
        let nh = f(args, "height").ok_or("missing height")?.round().clamp(1.0, 16384.0) as u32;
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        if (nw, nh) == (w, h) {
            return Ok(json!(false));
        }
        let mut px = b.px.as_ref().clone();
        super::premultiply(&mut px);
        let img = image::RgbaImage::from_raw(w, h, px).ok_or("bad picture")?;
        let mut out = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3).into_raw();
        super::unpremultiply(&mut out);
        let (kx, ky) = (nw as f64 / w as f64, nh as f64 / h as f64);
        self.swap_pixels(
            id,
            "Picture size",
            nw,
            nh,
            out,
            None,
            |a| Vec2::new(a.x * kx, a.y * ky),
            |s| Vec2::new(s.x / kx, s.y / ky),
        )
    }

    /// Adds see-through pixels around layer `id` (`left`, `top`, `right`, `bottom`, or `to_canvas`: as far as the
    /// canvas goes); the new part is selected, ready for an AI change to fill it.
    fn raster_extend(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let m = self.to_canvas(id)?;
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let [mut l, mut t, mut r, mut bm] =
            ["left", "top", "right", "bottom"].map(|k| f(args, k).unwrap_or(0.0).max(0.0).round() as i64);
        if args.get("to_canvas").and_then(Json::as_bool).unwrap_or(false) {
            let s = self.comp_settings().ok_or("no canvas")?;
            let inv = m.inverse();
            let pts = [(0.0, 0.0), (s.width as f64, 0.0), (0.0, s.height as f64), (s.width as f64, s.height as f64)]
                .map(|(x, y)| inv * Point::new(x, y));
            l = (-pts.iter().map(|p| p.x).fold(f64::MAX, f64::min).floor()).max(0.0) as i64;
            t = (-pts.iter().map(|p| p.y).fold(f64::MAX, f64::min).floor()).max(0.0) as i64;
            r = (pts.iter().map(|p| p.x).fold(f64::MIN, f64::max).ceil() - w as f64).max(0.0) as i64;
            bm = (pts.iter().map(|p| p.y).fold(f64::MIN, f64::max).ceil() - h as f64).max(0.0) as i64;
        }
        let (nw, nh) = (w as i64 + l + r, h as i64 + t + bm);
        if (l, t, r, bm) == (0, 0, 0, 0) {
            return Err("The picture already covers that.".into());
        }
        if nw > 16384 || nh > 16384 {
            return Err("That would make the picture bigger than 16384 pixels.".into());
        }
        let b = self.raster.bufs.get(&id).expect("loaded");
        let (nw, nh) = (nw as u32, nh as u32);
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        let mut sel = vec![255u8; (nw * nh) as usize];
        for y in 0..h {
            let (o, s) = ((((y as i64 + t) as u32 * nw + l as u32) * 4) as usize, (y * w * 4) as usize);
            out[o..o + (w * 4) as usize].copy_from_slice(&b.px[s..s + (w * 4) as usize]);
            let o = o / 4;
            sel[o..o + w as usize].fill(0);
        }
        let (dl, dt) = (l as f64, t as f64);
        self.swap_pixels(id, "Extend the picture", nw, nh, out, Some(sel), |a| Vec2::new(a.x + dl, a.y + dt), |s| s)
    }

    /// Puts `px` (`w` × `h`) on layer `id` as a new picture, moving its anchor and scale so the picture stays where it was.
    #[allow(clippy::too_many_arguments)]
    fn swap_pixels(
        &mut self,
        id: Id,
        label: &str,
        w: u32,
        h: u32,
        px: Vec<u8>,
        sel: Option<Vec<u8>>,
        anchor: impl Fn(Vec2) -> Vec2,
        scale: impl Fn(Vec2) -> Vec2,
    ) -> Result<Json, String> {
        self.raster_live_end(false)?;
        {
            let b: &mut Buf = self.raster.bufs.get_mut(&id).ok_or("not loaded")?;
            b.w = w;
            b.h = h;
            b.px = Arc::new(px);
            b.sel = None;
            b.outline = None;
        }
        self.editor.begin_group(label);
        let mut r = self.commit(id, label);
        if r.is_ok() {
            let mut l = self.project().layer(id).map_err(err)?.clone();
            map_prop(&mut l.transform.anchor, &anchor);
            map_prop(&mut l.transform.scale, &scale);
            r = self.exec(label, Command::ReplaceLayer { layer: Box::new(l) });
        }
        self.editor.end_group();
        r?;
        if let Some(b) = self.raster.bufs.get_mut(&id) {
            b.sel = sel.filter(|s| s.iter().any(|v| *v > 0));
        }
        Ok(json!(true))
    }
}

fn map_prop(p: &mut Property<Vec2>, f: &impl Fn(Vec2) -> Vec2) {
    match p {
        Property::Static(v) => *v = f(*v),
        Property::Animated(keys) => keys.iter_mut().for_each(|k| k.value = f(k.value)),
    }
}

/// Straight RGBA, times `m`, premultiplied.
fn premul_m(p: &[u8], m: f32) -> [f32; 4] {
    let a = p[3] as f32 * m;
    [p[0] as f32 * a / 255.0, p[1] as f32 * a / 255.0, p[2] as f32 * a / 255.0, a]
}

fn unpremul(v: &[f32; 4], o: &mut [u8]) {
    let a = v[3].clamp(0.0, 255.0);
    o[3] = a.round() as u8;
    for k in 0..3 {
        o[k] = if a < 0.5 { 0 } else { (v[k] * 255.0 / a).round().clamp(0.0, 255.0) as u8 };
    }
}

/// Bilinear sample at pixel-centre coordinates (x, y) of premultiplied `px` and of `mask`.
fn bilinear(px: &[[f32; 4]], mask: &[u8], w: u32, h: u32, x: f64, y: f64) -> ([f32; 4], u8) {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let mut v = [0.0f32; 4];
    let mut m = 0.0f32;
    for (dx, dy, wt) in
        [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
    {
        let (sx, sy) = (x0 as i64 + dx, y0 as i64 + dy);
        if sx < 0 || sy < 0 || sx >= w as i64 || sy >= h as i64 || wt <= 0.0 {
            continue;
        }
        let i = (sy as u32 * w + sx as u32) as usize;
        for k in 0..4 {
            v[k] += px[i][k] * wt;
        }
        m += mask[i] as f32 * wt;
    }
    (v, m.round().clamp(0.0, 255.0) as u8)
}

fn to_hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let l = (mx + mn) / 2.0;
    if mx - mn < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == c[0] {
        (c[1] - c[2]) / d + if c[1] < c[2] { 6.0 } else { 0.0 }
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 1e-6 {
        return [l; 3];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}
