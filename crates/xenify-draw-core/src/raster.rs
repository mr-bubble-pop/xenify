//! Raster layers: pictures you paint on. A raster layer is a picture layer
//! (`LayerKind::Image`) whose file is a PNG; any still picture can be painted.
//! The pixels being edited are kept decoded here (straight-alpha RGBA, one
//! per layer). While a stroke goes on the renderer is handed those pixels
//! directly ([`xenify_render::images::set_live`]); the stroke's end saves a
//! PNG with `SetAsset`, one undo step.
//!
//! A selection is a mask (0–255 per pixel) on one raster layer: painting,
//! filling, deleting and AI changes stay inside it.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use kurbo::{Affine, Point};
use serde_json::{Value as Json, json};
use xenify_doc::{Color, Id, ImageAsset, Layer, LayerKind, Property, Vec2};
use xenify_ops::Command;

use crate::{Session, err, f, id_arg};

mod frames;
mod ops;

/// Raster layers being worked on, in one open canvas.
#[derive(Default)]
pub struct Raster {
    bufs: HashMap<Id, Buf>,
    stroke: Option<Stroke>,
    /// What the canvas shows, drawn on a layer's pixel grid (fill and wand on "all layers").
    sample: Option<(Id, Vec<u8>)>,
    /// The part of the stroke's layer changed since the screens' renderer last took it (x0, y0, x1, y1).
    dirty: Option<(i64, i64, i64, i64)>,
    /// Selection changes to undo and redo; they fit in between the drawing's steps.
    sel_undo: Vec<SelStep>,
    sel_redo: Vec<SelStep>,
    /// A change shown live before it's kept (moving pixels, colour adjustments).
    edit: Option<ops::Edit>,
    /// Onion skin: the frame before shows faintly under the selected frame (screen only).
    onion: bool,
}

/// One selection change: the selection to go back to, and how many drawing steps there were to undo then.
struct SelStep {
    layer: Id,
    label: &'static str,
    sel: Option<Vec<u8>>,
    at: usize,
}

/// Selection changes kept for undo (each can be a full-size mask).
const SEL_HISTORY: usize = 40;

struct Buf {
    asset: Id,
    /// The hash of the file these pixels came from (or were saved as).
    hash: u64,
    w: u32,
    h: u32,
    px: Arc<Vec<u8>>,
    sel: Option<Vec<u8>>,
    /// The selection's edges in composition space (for the marching ants), until it changes.
    outline: Option<Json>,
}

struct Stroke {
    layer: Id,
    base: Vec<u8>,
    cover: Vec<u8>,
    color: [f32; 3],
    opacity: f32,
    erase: bool,
    brush: Brush,
    keep_alpha: bool,
    /// Painting the selection (quick mask) instead of pixels: the selection when the stroke began, and its mask
    /// to paint over (all 0 when there was none).
    sel_base: Option<(Option<Vec<u8>>, Vec<u8>)>,
    /// Pen pressure makes the paint lighter as well as thinner.
    pressure_opacity: bool,
    /// Clone from what the canvas shows (all layers) instead of the layer's own pixels.
    clone_src: Option<Vec<u8>>,
    hardness: f64,
    /// Diameter in canvas pixels, and layer pixels per canvas pixel.
    size: f64,
    px_per_unit: f64,
    pressure: bool,
    to_px: Affine,
    last: Option<(Point, f64)>,
    /// Distance walked since the last dab.
    carry: f64,
}

/// What a stroke does where it goes.
enum Brush {
    /// The paint colour (or rubbing out), through the stroke's coverage.
    Paint,
    /// The layer's own pixels from (dx, dy) layer pixels away, as they were when the stroke began.
    Clone(f64, f64),
    /// Drags the colours along: the pixels under the brush (premultiplied), carried from dab to dab,
    /// in a square `half` pixels each side of the centre. Empty until the first dab.
    Smudge { carry: Vec<f32>, half: i64 },
    /// Softens the pixels under the brush.
    Blur,
}

/// How a new selection combines with the one there is.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Replace,
    Add,
    Subtract,
    Intersect,
}

fn mode_arg(args: &Json) -> Mode {
    match args.get("mode").and_then(Json::as_str) {
        Some("add") => Mode::Add,
        Some("subtract") => Mode::Subtract,
        Some("intersect") => Mode::Intersect,
        _ => Mode::Replace,
    }
}

fn encode_png(w: u32, h: u32, px: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Up,
    )
    .write_image(px, w, h, image::ExtendedColorType::Rgba8)
    .map_err(err)?;
    Ok(out)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Premultiplied pixels (as the GPU reads them back) → straight alpha.
pub(crate) fn premultiply(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u16;
        for c in &mut p[..3] {
            *c = (*c as u16 * a / 255) as u8;
        }
    }
}

/// A mask drawn by a model, tidied: specks smaller than a few pixels go, edges stay soft.
fn clean_mask(m: Vec<u8>, w: u32, h: u32) -> Vec<u8> {
    let soft = blur(&m, w, h, 1);
    soft.into_iter()
        .map(|v| {
            if v < 24 {
                0
            } else if v > 232 {
                255
            } else {
                v
            }
        })
        .collect()
}

pub fn unpremultiply(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a == 0 {
            p[..3].fill(0);
        } else if a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

/// The most pixels a picture painted on may have (16384 × 16384, the
/// largest canvas: 1 GiB of RGBA). Pictures read from files are kept under
/// that by the image reader's own limit (512 MiB).
pub const MAX_PIXELS: u64 = 16384 * 16384;

/// Bytes of RGBA for a `w` × `h` picture (worked out without wrapping
/// around), or why there can't be one.
pub fn rgba_len(w: u32, h: u32) -> Result<usize, String> {
    let px = w as u64 * h as u64;
    if px > MAX_PIXELS {
        return Err("This canvas is too big to paint on.".into());
    }
    usize::try_from(px * 4).map_err(|_| "This canvas is too big to paint on.".to_string())
}

impl Session {
    pub(crate) fn raster_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "add_raster_layer" => self.add_raster_layer(args),
            "raster_info" => self.raster_info(id_arg(args, "id").ok()?),
            "raster_begin" => self.raster_begin(args),
            "raster_paint" => self.raster_paint(args),
            "raster_end" => self.raster_end(),
            "raster_cancel" => {
                self.raster_cancel();
                Ok(Json::Null)
            }
            "raster_fill" => self.raster_fill(args),
            "raster_wand" => self.raster_wand(args),
            "raster_select" => self.raster_select(args),
            "raster_select_all" => self.with_sel(args, "Select all", |_, w, h| Some(vec![255; (w * h) as usize])),
            "raster_select_none" => self.with_sel(args, "Select none", |_, _, _| None),
            "raster_invert" => self.with_sel(args, "Invert selection", |s, w, h| {
                Some(s.map_or_else(|| vec![255; (w * h) as usize], |s| s.iter().map(|v| 255 - v).collect()))
            }),
            "raster_grow" => {
                let by = f(args, "by").unwrap_or(4.0);
                let label = if by < 0.0 { "Shrink selection" } else { "Grow selection" };
                self.with_sel(args, label, move |s, w, h| s.map(|s| grow(s, w, h, by)))
            }
            "raster_feather" => {
                let r = f(args, "by").unwrap_or(4.0).clamp(0.0, 200.0) as usize;
                self.with_sel(args, "Soften selection edges", move |s, w, h| s.map(|s| blur(s, w, h, r)))
            }
            "raster_outline" => self.raster_outline(id_arg(args, "id").ok()?),
            "raster_clear" => self.raster_edit(args, "Delete pixels", |px, sel, _| {
                for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let m = sel.map_or(255, |s| s[i]) as u32;
                    p[3] = (p[3] as u32 * (255 - m) / 255) as u8;
                }
            }),
            "raster_fill_selection" => {
                let c = args.get("color").and_then(crate::parse_color).unwrap_or(Color::BLACK);
                let opacity = f(args, "opacity").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
                self.raster_edit(args, "Fill", move |px, sel, _| {
                    let all = vec![255u8; px.len() / 4];
                    paint_mask(px, sel.unwrap_or(&all), None, rgb(c), opacity * c.a as f32, false);
                })
            }
            "raster_layer_from_selection" => self.raster_layer_from_selection(args),
            "raster_pick" => self.raster_pick(args),
            "raster_ai_inputs" => self.raster_ai_inputs(args),
            "raster_ai_apply" => self.raster_ai_apply(args),
            _ => return self.raster_ops_call(name, args).or_else(|| self.raster_frames_call(name, args)),
        })
    }

    /// Layer `id`'s pixels → canvas, at the current time.
    fn to_canvas(&self, id: Id) -> Result<Affine, String> {
        let c = self.project().comp(self.comp).map_err(err)?;
        Ok(xenify_anim::world_transform(c, id, self.time))
    }

    /// Why layer `id` can't be painted, if it can't.
    fn not_paintable(&self, id: Id) -> Option<&'static str> {
        let Ok(l) = self.project().layer(id) else { return Some("That layer is gone.") };
        let LayerKind::Image { asset, .. } = &l.kind else {
            return Some("Painting works on raster layers: Layer › New raster layer, or Layer › Convert to raster.");
        };
        let Some(a) = self.project().asset(*asset) else { return Some("The picture is missing.") };
        if a.frames.is_some() {
            return Some("This picture is animated (a GIF or animated WebP): it can't be painted on.");
        }
        if a.name.starts_with("xen-") {
            return Some(
                "This picture is kept up to date from the server, so painting on it would be lost: Layer › Stop updating this picture first.",
            );
        }
        if l.meta.locked {
            return Some("This layer is locked: unlock it in the Layers panel to paint on it.");
        }
        None
    }

    /// The working pixels of raster layer `id`, read from its file when they're not here or the file changed (undo, someone else's change).
    fn buf(&mut self, id: Id) -> Result<&mut Buf, String> {
        if let Some(why) = self.not_paintable(id) {
            return Err(why.into());
        }
        let l = self.project().layer(id).map_err(err)?;
        let LayerKind::Image { asset, .. } = l.kind else { unreachable!() };
        let a = self.project().asset(asset).ok_or("the picture is missing")?;
        let fresh = self.raster.bufs.get(&id).is_some_and(|b| b.asset == asset && b.hash == a.hash);
        if !fresh {
            let (hash, data) = (a.hash, a.data.clone());
            let img =
                image::load_from_memory(&data).map_err(|e| format!("couldn't read the picture: {e}"))?.into_rgba8();
            let (w, h) = img.dimensions();
            let sel = self.raster.bufs.remove(&id).and_then(|b| b.sel.filter(|_| b.w == w && b.h == h));
            self.raster.bufs.insert(id, Buf { asset, hash, w, h, px: Arc::new(img.into_raw()), sel, outline: None });
        }
        Ok(self.raster.bufs.get_mut(&id).expect("just put there"))
    }

    /// Saves layer `id`'s working pixels as its picture (one undo step). A picture other layers show too gets a copy of its own.
    fn commit(&mut self, id: Id, label: &str) -> Result<(), String> {
        let (asset, old_hash, w, h, px) = {
            let b = self.raster.bufs.get(&id).ok_or("nothing painted")?;
            (b.asset, b.hash, b.w, b.h, b.px.clone())
        };
        let png = encode_png(w, h, &px)?;
        let a = self.project().asset(asset).ok_or("the picture is missing")?.clone();
        let shared = self
            .project()
            .compositions
            .iter()
            .flat_map(|c| &c.layers)
            .filter(|l| l.id != id && matches!(l.kind, LayerKind::Image { asset: x, .. } if x == asset))
            .count()
            > 0;
        let new_id = if shared { self.next_id() } else { asset };
        let mut new = ImageAsset::new(new_id, a.name.clone(), "image/png", w, h, png);
        // Drawn from a 3D model: it can still be turned again (which repaints it).
        new.model = a.model.clone();
        let new_hash = new.hash;
        let cmds = if shared {
            let mut l = self.project().layer(id).map_err(err)?.clone();
            if let LayerKind::Image { asset, .. } = &mut l.kind {
                *asset = new_id;
            }
            vec![Command::AddAsset { asset: new }, Command::ReplaceLayer { layer: Box::new(l) }]
        } else {
            vec![Command::SetAsset { asset: new }]
        };
        self.exec_all(label, cmds)?;
        xenify_render::images::clear_live(old_hash);
        xenify_render::images::prime(new_hash, w, h, px);
        if let Some(b) = self.raster.bufs.get_mut(&id) {
            b.asset = new_id;
            b.hash = new_hash;
        }
        Ok(())
    }

    /// Layer › New raster layer: an empty, see-through picture the canvas's size.
    fn add_raster_layer(&mut self, args: &Json) -> Result<Json, String> {
        let s = self.comp_settings().ok_or("no canvas")?;
        let (cw, ch) = (s.width as f64, s.height as f64);
        let (w, h) = (cw.round().max(1.0) as u32, ch.round().max(1.0) as u32);
        let png = encode_png(w, h, &vec![0; rgba_len(w, h)?])?;
        let name = self.unique_name(args.get("name").and_then(Json::as_str).unwrap_or("Raster"));
        let mut p = self.project().clone();
        let asset = ImageAsset::new(p.alloc_id(), name.clone(), "image/png", w, h, png);
        let mut layer = Layer::image(p.alloc_id(), name, &asset, Vec2::new(cw / 2.0, ch / 2.0));
        let id = layer.id;
        let comp = self.comp;
        let index = match args.get("above").and_then(Json::as_u64) {
            Some(above) => self.project().comp(comp).ok().and_then(|c| c.layer_index(Id(above))).map(|i| i + 1),
            None => self.place_in_active_folder(&mut layer)?,
        };
        self.exec_all("New raster layer", vec![Command::AddAsset { asset }, Command::AddLayer { comp, index, layer }])?;
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    fn raster_info(&mut self, id: Id) -> Result<Json, String> {
        if let Some(why) = self.not_paintable(id) {
            return Ok(json!({ "paintable": false, "why": why }));
        }
        let b = self.buf(id)?;
        let see_through = b.px.as_chunks::<4>().0.iter().any(|p| p[3] < 250);
        let (w, h, sel) = (b.w, b.h, b.sel.as_ref().map(|s| bounds(s, b.w, b.h)));
        let m = self.to_canvas(id)?;
        let unit = (m.determinant().abs()).sqrt();
        // The selection's box on the canvas (for the handles that move and scale it).
        let sel = sel.flatten();
        let sel_box = sel.map(|[x0, y0, x1, y1]| {
            let pts = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| m * Point::new(x as f64, y as f64));
            let (xs, ys) = (pts.map(|p| p.x), pts.map(|p| p.y));
            [
                xs.iter().copied().fold(f64::MAX, f64::min),
                ys.iter().copied().fold(f64::MAX, f64::min),
                xs.iter().copied().fold(f64::MIN, f64::max),
                ys.iter().copied().fold(f64::MIN, f64::max),
            ]
        });
        Ok(json!({
            "paintable": true, "width": w, "height": h, "px_per_unit": if unit > 0.0 { 1.0 / unit } else { 1.0 },
            "selection": sel, "selection_box": sel_box, "see_through": see_through,
        }))
    }

    // ----- Brush and eraser -----

    fn raster_begin(&mut self, args: &Json) -> Result<Json, String> {
        self.raster_cancel();
        self.raster_live_end(true)?;
        let id = id_arg(args, "id")?;
        let clone_src = (args.get("sample").and_then(Json::as_str) == Some("canvas"))
            .then(|| self.raster.sample.as_ref().filter(|(s, _)| *s == id).map(|(_, px)| px.clone()))
            .flatten();
        let to_selection = args.get("target").and_then(Json::as_str) == Some("selection");
        let to_px = self.to_canvas(id)?.inverse();
        let c = args.get("color").and_then(crate::parse_color).unwrap_or(Color::BLACK);
        let b = self.buf(id)?;
        let n = (b.w * b.h) as usize;
        let base = b.px.as_ref().clone();
        let sel_base = to_selection.then(|| (b.sel.clone(), b.sel.clone().unwrap_or_else(|| vec![0; n])));
        let px_per_unit = 1.0 / to_px.inverse().determinant().abs().sqrt().max(1e-9);
        let size = f(args, "size").unwrap_or(12.0).clamp(0.5, 2000.0);
        let erase = args.get("erase").and_then(Json::as_bool).unwrap_or(false);
        let brush = match args.get("brush").and_then(Json::as_str).unwrap_or("paint") {
            "smudge" => Brush::Smudge { carry: Vec::new(), half: (size * px_per_unit / 2.0).ceil() as i64 + 1 },
            "blur" => Brush::Blur,
            "clone" => {
                let o = args.get("clone_offset").and_then(Json::as_array).ok_or("clone needs clone_offset")?;
                let o = Vec2::new(
                    o.first().and_then(Json::as_f64).unwrap_or(0.0),
                    o.get(1).and_then(Json::as_f64).unwrap_or(0.0),
                );
                let d = to_px * Point::new(o.x, o.y) - to_px * Point::ZERO;
                Brush::Clone(d.x, d.y)
            }
            _ => Brush::Paint,
        };
        self.raster.stroke = Some(Stroke {
            layer: id,
            base,
            cover: vec![0; n],
            color: rgb(c),
            opacity: (f(args, "opacity").unwrap_or(1.0).clamp(0.0, 1.0) * c.a) as f32,
            erase,
            brush,
            sel_base,
            pressure_opacity: args.get("pressure_opacity").and_then(Json::as_bool).unwrap_or(false),
            clone_src,
            keep_alpha: args.get("keep_alpha").and_then(Json::as_bool).unwrap_or(false),
            hardness: f(args, "hardness").unwrap_or(0.8).clamp(0.0, 1.0),
            size,
            px_per_unit,
            pressure: args.get("pressure").and_then(Json::as_bool).unwrap_or(true),
            to_px,
            last: None,
            carry: 0.0,
        });
        // The layer shows the working pixels until the stroke ends (the renderer gets each change as it's made).
        let b = self.raster.bufs.get(&id).expect("loaded above");
        if !to_selection {
            xenify_render::images::set_live(b.hash, b.w, b.h, b.px.clone());
        }
        self.raster.dirty = None;
        Ok(Json::Null)
    }

    /// More of the stroke: `points` [[x, y, pressure], …] in canvas space.
    fn raster_paint(&mut self, args: &Json) -> Result<Json, String> {
        let Some(mut st) = self.raster.stroke.take() else { return Err("no stroke going".into()) };
        let pts: Vec<(Point, f64)> = args
            .get("points")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let p = p.as_array()?;
                        let pr = p.get(2).and_then(Json::as_f64).filter(|v| *v > 0.0).unwrap_or(0.5);
                        Some((st.to_px * Point::new(p.first()?.as_f64()?, p.get(1)?.as_f64()?), pr))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let id = st.layer;
        let was = self.raster.bufs.get(&id).map(|b| b.hash);
        let b = self.buf(id)?;
        let (w, h, hash) = (b.w, b.h, b.hash);
        let b = self.raster.bufs.get_mut(&id).expect("loaded above");
        if was != Some(hash) {
            // Someone else's change to the picture came in during the stroke.
            if let Some(old) = was {
                xenify_render::images::clear_live(old);
            }
            if st.cover.len() != (w * h) as usize {
                // Its size changed: the stroke stops here.
                return Err("The picture changed size while you painted (someone else changed it): paint again.".into());
            }
            if st.sel_base.is_none() {
                // The stroke goes on over their pixels (not over the ones it started from).
                st.base = b.px.as_ref().clone();
                if matches!(st.brush, Brush::Paint | Brush::Clone(..)) {
                    let px = Arc::make_mut(&mut b.px);
                    composite(px, &st.base, &st.cover, b.sel.as_deref(), w, h, (0, 0, w as i64, h as i64), &st);
                }
                xenify_render::images::set_live(hash, w, h, b.px.clone());
                grow_dirty(&mut self.raster.dirty, Some((0, 0, w as i64, h as i64)));
            }
        }
        let mut dirty: Option<(i64, i64, i64, i64)> = None;
        let radius = |st: &Stroke, pr: f64| {
            let k = if st.pressure { (pr * 2.0).clamp(0.15, 1.0) } else { 1.0 };
            (st.size * st.px_per_unit * k / 2.0).max(0.4)
        };
        for (p, pr) in pts {
            match st.last {
                None => {
                    let r = radius(&st, pr);
                    grow_dirty(&mut dirty, st.dab(b, p, r, pr));
                }
                Some((q, qpr)) => {
                    let len = (p - q).hypot();
                    let mut t = 0.0;
                    let mut walked = st.carry;
                    loop {
                        let r = radius(&st, qpr + (pr - qpr) * (t / len.max(1e-9)));
                        let step = (r * 0.18).max(0.5);
                        if walked + (len - t) < step {
                            st.carry = walked + (len - t);
                            break;
                        }
                        t += step - walked;
                        walked = 0.0;
                        let at = q + (p - q) * (t / len.max(1e-9));
                        let here = qpr + (pr - qpr) * (t / len.max(1e-9));
                        let r = radius(&st, here);
                        grow_dirty(&mut dirty, st.dab(b, at, r, here));
                    }
                }
            }
            st.last = Some((p, pr));
        }
        if let Some((x0, y0, x1, y1)) = dirty
            && let Some((_, base)) = &st.sel_base
        {
            // Painting the selection: the brush adds to it (the eraser takes away).
            let sel = b.sel.get_or_insert_with(|| base.clone());
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y as u32 * w + x as u32) as usize;
                    let c = st.cover[i] as f32 / 255.0 * st.opacity;
                    let o = base[i] as f32;
                    sel[i] = if st.erase { o * (1.0 - c) } else { o + (255.0 - o) * c }.round() as u8;
                }
            }
            b.outline = None;
        } else if let Some((x0, y0, x1, y1)) = dirty {
            if matches!(st.brush, Brush::Paint | Brush::Clone(..)) {
                let px = Arc::make_mut(&mut b.px);
                composite(px, &st.base, &st.cover, b.sel.as_deref(), w, h, (x0, y0, x1, y1), &st);
            }
            grow_dirty(&mut self.raster.dirty, dirty);
        }
        self.raster.stroke = Some(st);
        Ok(Json::Null)
    }

    /// The pixels painted since last asked, for a renderer that writes them into its upload of the layer:
    /// the picture's hash, the box (x, y, width, height) and its pixels, premultiplied.
    pub fn raster_take_dirty(&mut self) -> Option<(u64, [u32; 4], Vec<u8>)> {
        let layer = self.raster.stroke.as_ref().map(|s| s.layer).or(self.raster.edit.as_ref().map(|e| e.layer))?;
        let b = self.raster.bufs.get(&layer)?;
        let (x0, y0, x1, y1) = self.raster.dirty.take()?;
        let (x0, y0, x1, y1) = (x0.max(0) as u32, y0.max(0) as u32, (x1 as u32).min(b.w), (y1 as u32).min(b.h));
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
        for y in y0..y1 {
            for p in b.px[((y * b.w + x0) * 4) as usize..((y * b.w + x1) * 4) as usize].as_chunks::<4>().0 {
                let a = p[3] as u16;
                out.extend([
                    (p[0] as u16 * a / 255) as u8,
                    (p[1] as u16 * a / 255) as u8,
                    (p[2] as u16 * a / 255) as u8,
                    p[3],
                ]);
            }
        }
        Some((b.hash, [x0, y0, x1 - x0, y1 - y0], out))
    }

    fn raster_end(&mut self) -> Result<Json, String> {
        let Some(st) = self.raster.stroke.take() else { return Ok(Json::Null) };
        if let Some((was, _)) = st.sel_base {
            // One undo step for the painted selection.
            let Some(b) = self.raster.bufs.get_mut(&st.layer) else { return Ok(Json::Null) };
            let now = std::mem::replace(&mut b.sel, was);
            return Ok(self.set_sel(st.layer, "Paint the selection", now));
        }
        if st.cover.iter().all(|c| *c == 0) {
            return Ok(Json::Null);
        }
        let label = match st.brush {
            Brush::Paint if st.erase => "Erase",
            Brush::Paint => "Paint",
            Brush::Clone(..) => "Clone",
            Brush::Smudge { .. } => "Smudge",
            Brush::Blur => "Blur",
        };
        self.commit(st.layer, label)?;
        Ok(Json::Null)
    }

    fn raster_cancel(&mut self) {
        if let Some(st) = self.raster.stroke.take()
            && let Some(b) = self.raster.bufs.get_mut(&st.layer)
        {
            if let Some((was, _)) = st.sel_base {
                b.sel = was;
                b.outline = None;
                return;
            }
            xenify_render::images::clear_live(b.hash);
            b.px = Arc::new(st.base);
        }
    }

    // ----- Fill and wand -----

    /// The pixel under canvas point (`x`, `y`) on layer `id`'s grid, if it's on the layer.
    fn pixel_at(&self, id: Id, args: &Json, w: u32, h: u32) -> Result<Option<(u32, u32)>, String> {
        let p = self.to_canvas(id)?.inverse() * Point::new(f(args, "x").unwrap_or(0.0), f(args, "y").unwrap_or(0.0));
        Ok((p.x >= 0.0 && p.y >= 0.0 && p.x < w as f64 && p.y < h as f64).then_some((p.x as u32, p.y as u32)))
    }

    /// The pixels fill and wand look at: the layer's own, or (`sample: "canvas"`) what the canvas shows there.
    fn source(&mut self, id: Id, args: &Json) -> Result<Arc<Vec<u8>>, String> {
        let b = self.buf(id)?;
        let (w, h, px) = (b.w, b.h, b.px.clone());
        if args.get("sample").and_then(Json::as_str) == Some("canvas")
            && let Some((sid, s)) = &self.raster.sample
            && *sid == id
            && rgba_len(w, h).ok() == Some(s.len())
        {
            return Ok(Arc::new(s.clone()));
        }
        Ok(px)
    }

    fn region_at(&mut self, args: &Json) -> Result<Option<(Id, Vec<u8>)>, String> {
        let id = id_arg(args, "id")?;
        let src = self.source(id, args)?;
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let Some((x, y)) = self.pixel_at(id, args, w, h)? else { return Ok(None) };
        let tol = (f(args, "tolerance").unwrap_or(32.0).clamp(0.0, 100.0) / 100.0) as f32;
        let contiguous = args.get("contiguous").and_then(Json::as_bool).unwrap_or(true);
        let mut r = region(&src, w, h, x, y, tol, contiguous);
        let by = f(args, "grow").unwrap_or(0.0);
        if by != 0.0 {
            r = grow(&r, w, h, by);
        }
        Ok(Some((id, r)))
    }

    /// The paint bucket: fills the area of like colour (within `tolerance`) where you click.
    fn raster_fill(&mut self, args: &Json) -> Result<Json, String> {
        let Some((id, mut r)) = self.region_at(args)? else {
            return Err("Click on the layer's picture to fill.".into());
        };
        let c = args.get("color").and_then(crate::parse_color).unwrap_or(Color::BLACK);
        let opacity = f(args, "opacity").unwrap_or(1.0).clamp(0.0, 1.0) as f32 * c.a as f32;
        let b = self.raster.bufs.get_mut(&id).expect("loaded");
        if let Some(s) = &b.sel {
            for (v, m) in r.iter_mut().zip(s) {
                *v = ((*v as u32 * *m as u32) / 255) as u8;
            }
        }
        if r.iter().all(|v| *v == 0) {
            return Err("There's nothing to fill there (it's outside the selection).".into());
        }
        let keep_alpha = args.get("keep_alpha").and_then(Json::as_bool).unwrap_or(false);
        paint_mask(Arc::make_mut(&mut b.px).as_mut_slice(), &r, None, rgb(c), opacity, keep_alpha);
        self.commit(id, "Fill")?;
        Ok(json!(true))
    }

    /// The magic wand: selects the area of like colour where you click.
    fn raster_wand(&mut self, args: &Json) -> Result<Json, String> {
        let Some((id, r)) = self.region_at(args)? else {
            if mode_arg(args) == Mode::Replace {
                return self.with_sel(args, "Select none", |_, _, _| None);
            }
            return Ok(Json::Null);
        };
        let mode = mode_arg(args);
        let b = self.raster.bufs.get_mut(&id).expect("loaded");
        let new = combine(b.sel.clone(), r, mode);
        Ok(self.set_sel(id, "Select", new))
    }

    /// Rectangle, ellipse or lasso selection: `shape` ("rect" with `rect`, "ellipse" with `rect`, "lasso" with `points`), canvas space.
    fn raster_select(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let to_px = self.to_canvas(id)?.inverse();
        let poly: Vec<Point> = match args.get("shape").and_then(Json::as_str).unwrap_or("rect") {
            "lasso" => args
                .get("points")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(|p| Some(Point::new(p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect())
                .unwrap_or_default(),
            shape => {
                let r: Vec<f64> = args
                    .get("rect")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_f64).collect())
                    .unwrap_or_default();
                if r.len() != 4 {
                    return Err("rect needs 4 numbers".into());
                }
                let (x0, y0, x1, y1) = (r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3]));
                if shape == "ellipse" {
                    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
                    (0..96)
                        .map(|i| {
                            let t = i as f64 / 96.0 * std::f64::consts::TAU;
                            Point::new(cx + rx * t.cos(), cy + ry * t.sin())
                        })
                        .collect()
                } else {
                    vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
                }
            }
        };
        let poly: Vec<Point> = poly.into_iter().map(|p| to_px * p).collect();
        let mode = mode_arg(args);
        let b = self.buf(id)?;
        let r = fill_polygon(&poly, b.w, b.h);
        let new = combine(b.sel.clone(), r, mode);
        Ok(self.set_sel(id, "Select", new))
    }

    /// Changes layer `id`'s selection with `f(selection, w, h)`.
    fn with_sel(
        &mut self,
        args: &Json,
        label: &'static str,
        f: impl FnOnce(Option<&[u8]>, u32, u32) -> Option<Vec<u8>>,
    ) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let b = self.buf(id)?;
        let new = f(b.sel.as_deref(), b.w, b.h);
        Ok(self.set_sel(id, label, new))
    }

    /// Sets layer `id`'s (loaded) selection, as a step you can undo; returns its bounds.
    fn set_sel(&mut self, id: Id, label: &'static str, new: Option<Vec<u8>>) -> Json {
        let at = self.editor.undo_len();
        let b = self.raster.bufs.get_mut(&id).expect("loaded");
        let new = new.filter(|s| s.iter().any(|v| *v > 0));
        let old = std::mem::replace(&mut b.sel, new);
        b.outline = None;
        let out = json!(b.sel.as_ref().and_then(|s| bounds(s, b.w, b.h)));
        if old != b.sel {
            let r = &mut self.raster;
            r.sel_undo.push(SelStep { layer: id, label, sel: old, at });
            if r.sel_undo.len() > SEL_HISTORY {
                r.sel_undo.remove(0);
            }
            r.sel_redo.clear();
        }
        out
    }

    /// Undo or redo: a selection change made since the drawing's last step
    /// goes first. Returns its label, or `None` when the drawing's step is next.
    pub(crate) fn raster_undo_sel(&mut self, undo: bool) -> Option<String> {
        loop {
            let at = self.editor.undo_len();
            let r = &mut self.raster;
            let from = if undo { &mut r.sel_undo } else { &mut r.sel_redo };
            let top = from.last()?;
            if top.at != at {
                // Drawing steps came after it: they go first (undo), or it can't come back (redo, after a new step).
                if if undo { top.at > at } else { top.at < at || self.editor.redo_label().is_none() } {
                    from.clear();
                }
                return None;
            }
            let step = from.pop().expect("there");
            let Some(b) = self.raster.bufs.get_mut(&step.layer) else { continue };
            if step.sel.as_ref().is_some_and(|s| s.len() != (b.w * b.h) as usize) {
                continue;
            }
            let now = std::mem::replace(&mut b.sel, step.sel);
            b.outline = None;
            let to = if undo { &mut self.raster.sel_redo } else { &mut self.raster.sel_undo };
            to.push(SelStep { layer: step.layer, label: step.label, sel: now, at });
            return Some(step.label.to_string());
        }
    }

    /// The label of the selection change undo (or redo) would do next.
    pub(crate) fn raster_sel_label(&self, undo: bool) -> Option<&'static str> {
        let r = &self.raster;
        let top = if undo { r.sel_undo.last() } else { r.sel_redo.last() }?;
        (top.at == self.editor.undo_len()).then_some(top.label)
    }

    /// The selection's edges as segments [x0, y0, x1, y1, …] in canvas space (empty: nothing selected).
    fn raster_outline(&mut self, id: Id) -> Result<Json, String> {
        if self.not_paintable(id).is_some() {
            return Ok(json!([]));
        }
        let m = self.to_canvas(id)?;
        let b = self.buf(id)?;
        if let Some(o) = &b.outline {
            return Ok(o.clone());
        }
        let segs = b.sel.as_ref().map(|s| outline(s, b.w, b.h)).unwrap_or_default();
        let out: Vec<f64> = segs
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|s| {
                let (a, c) = (m * Point::new(s[0] as f64, s[1] as f64), m * Point::new(s[2] as f64, s[3] as f64));
                [a.x, a.y, c.x, c.y]
            })
            .collect();
        let j = json!(out);
        b.outline = Some(j.clone());
        Ok(j)
    }

    /// Changes layer `id`'s pixels with `edit(pixels, selection, size)` and saves them as `label`.
    fn raster_edit(
        &mut self,
        args: &Json,
        label: &str,
        edit: impl FnOnce(&mut [u8], Option<&[u8]>, (u32, u32)),
    ) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let b = self.buf(id)?;
        let size = (b.w, b.h);
        let sel = b.sel.clone();
        edit(Arc::make_mut(&mut b.px).as_mut_slice(), sel.as_deref(), size);
        self.commit(id, label)?;
        Ok(json!(true))
    }

    /// Select › New layer from the selection: the selected pixels on a raster layer of their own, above (`cut`: taken out of this one).
    fn raster_layer_from_selection(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let cut = args.get("cut").and_then(Json::as_bool).unwrap_or(false);
        let b = self.buf(id)?;
        let sel =
            b.sel.clone().ok_or("Select part of the picture first (the wand, or a rectangle, ellipse or lasso).")?;
        let (w, h) = (b.w, b.h);
        let mut piece = b.px.as_ref().clone();
        for (p, m) in piece.as_chunks_mut::<4>().0.iter_mut().zip(&sel) {
            p[3] = (p[3] as u32 * *m as u32 / 255) as u8;
        }
        let png = encode_png(w, h, &piece)?;
        let src = self.project().layer(id).map_err(err)?.clone();
        let name = self.unique_name(&format!("{} piece", src.meta.name));
        let mut p = self.project().clone();
        let asset = ImageAsset::new(p.alloc_id(), name.clone(), "image/png", w, h, png);
        let mut layer = src.clone();
        layer.id = p.alloc_id();
        layer.meta.name = name;
        layer.kind = LayerKind::Image { asset: asset.id, playback: Default::default() };
        let new = layer.id;
        let comp = self.comp;
        let index = self.project().comp(comp).ok().and_then(|c| c.layer_index(id)).map(|i| i + 1);
        self.editor.begin_group(if cut { "Cut to a new layer" } else { "Copy to a new layer" });
        let mut r =
            self.exec_all("New layer", vec![Command::AddAsset { asset }, Command::AddLayer { comp, index, layer }]);
        if r.is_ok() && cut {
            r = self
                .raster_edit(&json!({ "id": id.0 }), "Cut", |px, sel, _| {
                    for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let m = sel.map_or(255, |s| s[i]) as u32;
                        p[3] = (p[3] as u32 * (255 - m) / 255) as u8;
                    }
                })
                .map(|_| ());
        }
        self.editor.end_group();
        r?;
        self.selection = vec![new];
        Ok(json!(new.0))
    }

    /// The colour of the pixel under (`x`, `y`) on layer `id` (null where it's see-through).
    fn raster_pick(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let b = self.buf(id)?;
        let (w, h, px) = (b.w, b.h, b.px.clone());
        let Some((x, y)) = self.pixel_at(id, args, w, h)? else { return Ok(Json::Null) };
        let i = ((y * w + x) * 4) as usize;
        if px[i + 3] < 8 {
            return Ok(Json::Null);
        }
        Ok(json!(format!("#{:02x}{:02x}{:02x}", px[i], px[i + 1], px[i + 2])))
    }

    // ----- Rasterize (the web engine draws, this swaps) -----

    /// Layers `ids` alone (with what's inside groups), on a see-through canvas: what Convert to raster draws.
    pub fn raster_scene(&self, ids: &[Id]) -> Option<(xenify_anim::Scene, u32, u32)> {
        fn keep(layers: Vec<xenify_anim::SceneLayer>, ids: &[Id]) -> Vec<xenify_anim::SceneLayer> {
            layers
                .into_iter()
                .filter_map(|mut l| {
                    if ids.contains(&l.id) {
                        return Some(l);
                    }
                    if let xenify_anim::SceneContent::Group(inner) = l.content {
                        let inner = keep(inner, ids);
                        if inner.is_empty() {
                            return None;
                        }
                        l.content = xenify_anim::SceneContent::Group(inner);
                        return Some(l);
                    }
                    None
                })
                .collect()
        }
        let mut scene = self.scene();
        scene.layers = keep(std::mem::take(&mut scene.layers), ids);
        if scene.layers.is_empty() {
            return None;
        }
        scene.background = Color::rgba(0.0, 0.0, 0.0, 0.0);
        let (w, h) = (scene.width, scene.height);
        Some((scene, w, h))
    }

    /// Swaps layers `ids` (and what's in them) for one raster layer showing `rgba` (premultiplied, the canvas's size).
    pub fn raster_swap(
        &mut self,
        ids: &[Id],
        w: u32,
        h: u32,
        mut rgba: Vec<u8>,
        name: Option<&str>,
    ) -> Result<Json, String> {
        if rgba_len(w, h).ok() != Some(rgba.len()) {
            return Err("the drawing came back the wrong size".into());
        }
        unpremultiply(&mut rgba);
        let c = self.project().comp(self.comp).map_err(err)?.clone();
        let mut gone: Vec<Id> = Vec::new();
        fn with_inside(c: &xenify_doc::Composition, id: Id, out: &mut Vec<Id>) {
            if out.contains(&id) {
                return;
            }
            for l in c.layers.iter().filter(|l| l.meta.parent == Some(id)) {
                with_inside(c, l.id, out);
            }
            out.push(id);
        }
        for id in ids {
            with_inside(&c, *id, &mut gone);
        }
        let top = gone.iter().filter_map(|id| c.layer_index(*id)).max().ok_or("Select the layers to convert.")?;
        let first = ids.iter().copied().max_by_key(|id| c.layer_index(*id).unwrap_or(0)).ok_or("nothing selected")?;
        let base =
            name.map(str::to_string).unwrap_or_else(|| c.layer(first).map_or("Raster".into(), |l| l.meta.name.clone()));
        let name = self.unique_name(&base);
        let (cw, ch) = (c.settings.width as f64, c.settings.height as f64);
        let d = w as f64 / cw.max(1.0);
        let png = encode_png(w, h, &rgba)?;
        let mut p = self.project().clone();
        let asset = ImageAsset::new(p.alloc_id(), name.clone(), "image/png", w, h, png);
        let mut layer = Layer::image(p.alloc_id(), name, &asset, Vec2::new(cw / 2.0, ch / 2.0));
        layer.transform.scale = Property::Static(Vec2::new(1.0 / d, 1.0 / d));
        // Inside the same folder, when that folder is only moved.
        if let Some(parent) = c.layer(first).and_then(|l| l.meta.parent).filter(|g| !gone.contains(g)) {
            let [a, b, cc, dd, tx, ty] = xenify_anim::world_transform(&c, parent, self.time).as_coeffs();
            if (a - 1.0).abs() < 1e-9 && b.abs() < 1e-9 && cc.abs() < 1e-9 && (dd - 1.0).abs() < 1e-9 {
                layer.transform.position = Property::Static(Vec2::new(cw / 2.0 - tx, ch / 2.0 - ty));
                layer.meta.parent = Some(parent);
            }
        }
        let id = layer.id;
        let (hash, rgba) = (asset.hash, Arc::new(rgba));
        let mut cmds =
            vec![Command::AddAsset { asset }, Command::AddLayer { comp: self.comp, index: Some(top + 1), layer }];
        cmds.extend(gone.iter().map(|l| Command::RemoveLayer { layer: *l }));
        self.exec_all("Convert to raster", cmds)?;
        xenify_render::images::prime(hash, w, h, rgba);
        self.selection = vec![id];
        Ok(json!(id.0))
    }

    /// For sampling the whole canvas on layer `id`'s pixel grid: the scene, the view (canvas → pixels) and the size.
    pub fn raster_sample_view(&mut self, id: Id) -> Option<(xenify_anim::Scene, Affine, u32, u32)> {
        let m = self.to_canvas(id).ok()?.inverse();
        let b = self.buf(id).ok()?;
        let (w, h) = (b.w, b.h);
        Some((self.scene(), m, w, h))
    }

    /// What the canvas shows on layer `id`'s grid (premultiplied), for fill and wand on "all layers" and the AI.
    pub fn raster_set_sample(&mut self, id: Id, mut rgba: Vec<u8>) {
        unpremultiply(&mut rgba);
        self.raster.sample = Some((id, rgba));
    }

    // ----- AI changes -----

    /// What an image model is shown: the picture (`source`: "layer", over white, or "canvas", what you see),
    /// the selection as a white-on-black mask, at most `max` pixels on the longer side; PNGs in base64.
    fn raster_ai_inputs(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let canvas = args.get("source").and_then(Json::as_str) == Some("canvas");
        let src = self.source(id, &json!({ "sample": if canvas { "canvas" } else { "layer" } }))?;
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let max = f(args, "max").unwrap_or(1024.0).clamp(64.0, 4096.0);
        let k = (max / w.max(h) as f64).min(1.0);
        let (tw, th) = (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1));
        let mut flat = src.as_ref().clone();
        for p in flat.as_chunks_mut::<4>().0 {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
        let img = image::RgbaImage::from_raw(w, h, flat).ok_or("bad picture")?;
        let small = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle);
        let mask = match &b.sel {
            Some(s) => {
                let m: Vec<u8> = s.iter().flat_map(|v| [*v, *v, *v, 255]).collect();
                let m = image::RgbaImage::from_raw(w, h, m).ok_or("bad mask")?;
                let m = image::imageops::resize(&m, tw, th, image::imageops::FilterType::Triangle);
                Json::from(b64(&encode_png(tw, th, m.as_raw())?))
            }
            None => Json::Null,
        };
        Ok(json!({ "image": b64(&encode_png(tw, th, small.as_raw())?), "mask": mask, "width": tw, "height": th }))
    }

    /// Puts an image model's answer (PNG/JPEG/WebP in base64) on layer `id`: inside the selection only
    /// (softened by `feather` pixels) when `only_selection`, keeping the see-through parts when `keep_alpha`.
    fn raster_ai_apply(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing picture")?)
            .map_err(err)?;
        let got =
            image::load_from_memory(&data).map_err(|e| format!("the AI's picture couldn't be read: {e}"))?.into_rgba8();
        let only_sel = args.get("only_selection").and_then(Json::as_bool).unwrap_or(true);
        let keep_alpha = args.get("keep_alpha").and_then(Json::as_bool).unwrap_or(false);
        let feather = f(args, "feather").unwrap_or(3.0).clamp(0.0, 100.0) as usize;
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let fitted = fit_cover(&got, w, h);
        let mask = match (&b.sel, only_sel) {
            (Some(s), true) => Some(if feather > 0 { blur(s, w, h, feather) } else { s.clone() }),
            _ => None,
        };
        let px = Arc::make_mut(&mut b.px);
        let new = fitted.as_raw();
        for i in 0..(w * h) as usize {
            let m = mask.as_ref().map_or(255, |m| m[i]) as u32;
            if m == 0 {
                continue;
            }
            let (o, n) = (&mut px[i * 4..i * 4 + 4], &new[i * 4..i * 4 + 4]);
            let a = if keep_alpha { o[3] } else { 255 };
            for c in 0..3 {
                o[c] = ((o[c] as u32 * (255 - m) + n[c] as u32 * m) / 255) as u8;
            }
            o[3] = ((o[3] as u32 * (255 - m) + a as u32 * m) / 255) as u8;
        }
        self.commit(id, "AI change")?;
        Ok(json!(true))
    }
}

// ----- Pixels -----

fn rgb(c: Color) -> [f32; 3] {
    [c.r as f32 * 255.0, c.g as f32 * 255.0, c.b as f32 * 255.0]
}

/// One round dab of radius `r` (pixels) at `at` into the stroke's coverage. Returns the box it touched.
fn dab(
    cover: &mut [u8],
    w: u32,
    h: u32,
    at: Point,
    r: f64,
    hardness: f64,
    amount: f64,
) -> Option<(i64, i64, i64, i64)> {
    let x0 = ((at.x - r - 1.0).floor() as i64).max(0);
    let y0 = ((at.y - r - 1.0).floor() as i64).max(0);
    let x1 = ((at.x + r + 1.0).ceil() as i64).min(w as i64);
    let y1 = ((at.y + r + 1.0).ceil() as i64).min(h as i64);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let a = (falloff(x, y, at, r, hardness) * amount * 255.0).round() as u8;
            if a == 0 {
                continue;
            }
            let i = (y as u32 * w + x as u32) as usize;
            cover[i] = cover[i].max(a);
        }
    }
    Some((x0, y0, x1, y1))
}

/// How much of a round dab of radius `r` at `at` covers pixel (`x`, `y`): 1 inside `hardness`, easing to 0 at the edge.
fn falloff(x: i64, y: i64, at: Point, r: f64, hardness: f64) -> f64 {
    let d = ((x as f64 + 0.5 - at.x).powi(2) + (y as f64 + 0.5 - at.y).powi(2)).sqrt();
    let edge = (r + 0.5 - d).clamp(0.0, 1.0);
    if edge <= 0.0 {
        return 0.0;
    }
    let t = d / r.max(1e-6);
    let soft = if t <= hardness {
        1.0
    } else {
        let u = ((t - hardness) / (1.0 - hardness).max(1e-6)).clamp(0.0, 1.0);
        1.0 - u * u * (3.0 - 2.0 * u)
    };
    edge.min(soft)
}

impl Stroke {
    /// One dab of the brush at `at` (layer pixels), radius `r`; returns the box it touched.
    fn dab(&mut self, b: &mut Buf, at: Point, r: f64, pressure: f64) -> Option<(i64, i64, i64, i64)> {
        let (w, h) = (b.w, b.h);
        let amount = if self.pressure_opacity { (pressure * 2.0).clamp(0.05, 1.0) } else { 1.0 };
        let touched = dab(&mut self.cover, w, h, at, r, self.hardness, amount);
        let (x0, y0, x1, y1) = touched?;
        let sel = b.sel.as_deref();
        let strength = self.opacity;
        let hardness = self.hardness;
        match &mut self.brush {
            Brush::Paint | Brush::Clone(..) => {}
            Brush::Smudge { carry, half } => {
                let half = *half;
                let side = (2 * half + 1) as usize;
                let (cx, cy) = (at.x.floor() as i64, at.y.floor() as i64);
                let px = Arc::make_mut(&mut b.px);
                let first = carry.is_empty();
                if first {
                    carry.resize(side * side * 4, 0.0);
                }
                for j in 0..side * side {
                    let (x, y) = (cx - half + (j % side) as i64, cy - half + (j / side) as i64);
                    if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                        continue;
                    }
                    let i = (y as u32 * w + x as u32) as usize;
                    let cur = premul(&px[i * 4..i * 4 + 4]);
                    let c = &mut carry[j * 4..j * 4 + 4];
                    if first {
                        c.copy_from_slice(&cur);
                        continue;
                    }
                    let m =
                        (falloff(x, y, at, r, hardness) as f32) * strength * sel.map_or(1.0, |s| s[i] as f32 / 255.0);
                    if m <= 0.0 {
                        continue;
                    }
                    let new: [f32; 4] = std::array::from_fn(|k| cur[k] + (c[k] - cur[k]) * m);
                    unpremul_into(&new, &mut px[i * 4..i * 4 + 4]);
                    // The brush picks up some of what it went over.
                    for k in 0..4 {
                        c[k] += (new[k] - c[k]) * (1.0 - strength * 0.9);
                    }
                }
            }
            Brush::Blur => {
                let k = ((r / 3.0).round() as i64).clamp(1, 16);
                let (bx0, by0) = ((x0 - k).max(0), (y0 - k).max(0));
                let (bx1, by1) = ((x1 + k).min(w as i64), (y1 + k).min(h as i64));
                let (bw, bh) = ((bx1 - bx0) as usize, (by1 - by0) as usize);
                let px = Arc::make_mut(&mut b.px);
                // A box blur (across, then down) of the box around the dab, premultiplied.
                let mut a: Vec<[f32; 4]> = (0..bw * bh)
                    .map(|j| {
                        let i = ((by0 + (j / bw) as i64) as u32 * w + (bx0 + (j % bw) as i64) as u32) as usize;
                        premul(&px[i * 4..i * 4 + 4])
                    })
                    .collect();
                for pass in 0..2 {
                    let (n, lines) = if pass == 0 { (bw, bh) } else { (bh, bw) };
                    let at = |line: usize, t: usize| if pass == 0 { line * bw + t } else { t * bw + line };
                    let mut out = a.clone();
                    for line in 0..lines {
                        for t in 0..n {
                            let (lo, hi) = ((t as i64 - k).max(0) as usize, ((t as i64 + k) as usize).min(n - 1));
                            let mut sum = [0.0f32; 4];
                            for u in lo..=hi {
                                let v = a[at(line, u)];
                                for c in 0..4 {
                                    sum[c] += v[c];
                                }
                            }
                            let cnt = (hi - lo + 1) as f32;
                            out[at(line, t)] = sum.map(|v| v / cnt);
                        }
                    }
                    a = out;
                }
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = (y as u32 * w + x as u32) as usize;
                        let m = (falloff(x, y, at, r, hardness) as f32)
                            * strength
                            * sel.map_or(1.0, |s| s[i] as f32 / 255.0);
                        if m <= 0.0 {
                            continue;
                        }
                        let blurred = a[(y - by0) as usize * bw + (x - bx0) as usize];
                        let cur = premul(&px[i * 4..i * 4 + 4]);
                        let new: [f32; 4] = std::array::from_fn(|c| cur[c] + (blurred[c] - cur[c]) * m);
                        unpremul_into(&new, &mut px[i * 4..i * 4 + 4]);
                    }
                }
            }
        }
        touched
    }
}

/// A straight-alpha RGBA pixel, premultiplied (0–255).
fn premul(p: &[u8]) -> [f32; 4] {
    let a = p[3] as f32 / 255.0;
    [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, p[3] as f32]
}

fn unpremul_into(v: &[f32; 4], o: &mut [u8]) {
    let a = v[3].clamp(0.0, 255.0);
    o[3] = a.round() as u8;
    if o[3] == 0 {
        o[..3].copy_from_slice(&[0, 0, 0]);
        return;
    }
    for c in 0..3 {
        o[c] = (v[c] * 255.0 / a).round().clamp(0.0, 255.0) as u8;
    }
}

fn grow_dirty(d: &mut Option<(i64, i64, i64, i64)>, r: Option<(i64, i64, i64, i64)>) {
    let Some(r) = r else { return };
    *d = Some(match *d {
        None => r,
        Some(o) => (o.0.min(r.0), o.1.min(r.1), o.2.max(r.2), o.3.max(r.3)),
    });
}

/// The stroke-start pixels with the stroke's coverage painted (or erased) over them, in box `r`.
#[allow(clippy::too_many_arguments)]
fn composite(
    px: &mut [u8],
    base: &[u8],
    cover: &[u8],
    sel: Option<&[u8]>,
    w: u32,
    h: u32,
    r: (i64, i64, i64, i64),
    st: &Stroke,
) {
    for y in r.1..r.3 {
        for x in r.0..r.2 {
            let i = (y as u32 * w + x as u32) as usize;
            let mut m = cover[i] as f32 / 255.0 * st.opacity * sel.map_or(1.0, |s| s[i] as f32 / 255.0);
            let mut color = st.color;
            if let Brush::Clone(dx, dy) = st.brush {
                let (sx, sy) = ((x as f64 + dx).round() as i64, (y as f64 + dy).round() as i64);
                if sx < 0 || sy < 0 || sx >= w as i64 || sy >= h as i64 {
                    m = 0.0;
                } else {
                    let j = (sy as u32 * w + sx as u32) as usize * 4;
                    let src = st.clone_src.as_deref().unwrap_or(base);
                    color = [src[j] as f32, src[j + 1] as f32, src[j + 2] as f32];
                    m *= src[j + 3] as f32 / 255.0;
                }
            }
            let (o, b) = (&mut px[i * 4..i * 4 + 4], &base[i * 4..i * 4 + 4]);
            blend_px(o, b, m, color, st.erase, st.keep_alpha);
        }
    }
}

/// Pixel `b` with `color` laid over it at coverage `m` (or erased by `m`), into `o`.
fn blend_px(o: &mut [u8], b: &[u8], m: f32, color: [f32; 3], erase: bool, keep_alpha: bool) {
    let ab = b[3] as f32 / 255.0;
    if erase {
        o[..3].copy_from_slice(&b[..3]);
        o[3] = (ab * (1.0 - m) * 255.0).round() as u8;
        return;
    }
    if keep_alpha {
        for c in 0..3 {
            o[c] = (b[c] as f32 * (1.0 - m) + color[c] * m).round() as u8;
        }
        o[3] = b[3];
        return;
    }
    let ao = m + ab * (1.0 - m);
    if ao <= 1e-6 {
        o.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for c in 0..3 {
        o[c] = ((color[c] * m + b[c] as f32 * ab * (1.0 - m)) / ao).round().clamp(0.0, 255.0) as u8;
    }
    o[3] = (ao * 255.0).round() as u8;
}

/// `color` laid over `px` through `mask` (0–255) at `opacity`.
fn paint_mask(px: &mut [u8], mask: &[u8], sel: Option<&[u8]>, color: [f32; 3], opacity: f32, keep_alpha: bool) {
    for (i, m) in mask.iter().enumerate() {
        if *m == 0 {
            continue;
        }
        let m = *m as f32 / 255.0 * opacity * sel.map_or(1.0, |s| s[i] as f32 / 255.0);
        let b: [u8; 4] = px[i * 4..i * 4 + 4].try_into().expect("4 bytes");
        blend_px(&mut px[i * 4..i * 4 + 4], &b, m, color, false, keep_alpha);
    }
}

/// The pixels like the one at (`x`, `y`) — every channel within `tol` (0–1), compared premultiplied
/// so see-through pixels match whatever their colour — joined to it (`contiguous`) or anywhere. 255 = in.
fn region(src: &[u8], w: u32, h: u32, x: u32, y: u32, tol: f32, contiguous: bool) -> Vec<u8> {
    let pm = |i: usize| {
        let a = src[i * 4 + 3] as f32;
        [src[i * 4] as f32 * a / 255.0, src[i * 4 + 1] as f32 * a / 255.0, src[i * 4 + 2] as f32 * a / 255.0, a]
    };
    let seed = pm((y * w + x) as usize);
    let t = tol * 255.0 + 0.5;
    let like = |i: usize| {
        let p = pm(i);
        (0..4).all(|c| (p[c] - seed[c]).abs() <= t)
    };
    let n = (w * h) as usize;
    let mut out = vec![0u8; n];
    if !contiguous {
        for (i, o) in out.iter_mut().enumerate() {
            if like(i) {
                *o = 255;
            }
        }
        return out;
    }
    // Scanline flood fill.
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        let row = (y * w) as usize;
        if out[row + x as usize] != 0 || !like(row + x as usize) {
            continue;
        }
        let mut l = x;
        while l > 0 && out[row + l as usize - 1] == 0 && like(row + l as usize - 1) {
            l -= 1;
        }
        let mut r = x;
        while r + 1 < w && out[row + r as usize + 1] == 0 && like(row + r as usize + 1) {
            r += 1;
        }
        for i in l..=r {
            out[row + i as usize] = 255;
        }
        for ny in [y.wrapping_sub(1), y + 1] {
            if ny >= h {
                continue;
            }
            let nrow = (ny * w) as usize;
            let mut i = l;
            while i <= r {
                if out[nrow + i as usize] == 0 && like(nrow + i as usize) {
                    stack.push((i, ny));
                    while i <= r && like(nrow + i as usize) {
                        i += 1;
                    }
                }
                i += 1;
            }
        }
    }
    out
}

fn combine(old: Option<Vec<u8>>, new: Vec<u8>, mode: Mode) -> Option<Vec<u8>> {
    let out = match (mode, old) {
        (Mode::Replace, _) | (Mode::Add, None) => new,
        (Mode::Subtract | Mode::Intersect, None) => return None,
        (Mode::Add, Some(o)) => o.iter().zip(&new).map(|(a, b)| *a.max(b)).collect(),
        (Mode::Subtract, Some(o)) => {
            o.iter().zip(&new).map(|(a, b)| ((*a as u32 * (255 - *b as u32)) / 255) as u8).collect()
        }
        (Mode::Intersect, Some(o)) => o.iter().zip(&new).map(|(a, b)| *a.min(b)).collect(),
    };
    out.iter().any(|v| *v > 0).then_some(out)
}

/// The selection's box [x0, y0, x1, y1] in pixels.
fn bounds(s: &[u8], w: u32, h: u32) -> Option<[u32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if s[(y * w + x) as usize] >= 128 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0).then_some([x0, y0, x1, y1])
}

/// The pixels inside polygon `poly` (pixel space), with soft edges (4 rows a pixel, exact across).
fn fill_polygon(poly: &[Point], w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h) as usize];
    if poly.len() < 3 {
        return out;
    }
    let (ymin, ymax) = poly.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let mut row = vec![0f32; w as usize];
    let mut xs: Vec<f64> = Vec::new();
    for y in (ymin.floor().max(0.0) as u32)..(ymax.ceil().min(h as f64) as u32) {
        row.iter_mut().for_each(|v| *v = 0.0);
        for sub in 0..4 {
            let sy = y as f64 + (sub as f64 + 0.5) / 4.0;
            xs.clear();
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if (a.y <= sy) != (b.y <= sy) {
                    xs.push(a.x + (sy - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.as_chunks::<2>().0 {
                let (xa, xb) = (pair[0].clamp(0.0, w as f64), pair[1].clamp(0.0, w as f64));
                if xb <= xa {
                    continue;
                }
                let (ia, ib) = (xa.floor() as usize, (xb.ceil() as usize).min(w as usize));
                for (i, v) in row.iter_mut().enumerate().take(ib).skip(ia) {
                    let cover = ((i + 1) as f64).min(xb) - (i as f64).max(xa);
                    *v += cover.max(0.0) as f32 / 4.0;
                }
            }
        }
        for (x, v) in row.iter().enumerate() {
            out[(y * w) as usize + x] = (v.min(1.0) * 255.0).round() as u8;
        }
    }
    out
}

/// Squared distance from each pixel to the nearest pixel with `inside` set (Felzenszwalb & Huttenlocher).
fn distance2(inside: impl Fn(usize) -> bool, w: u32, h: u32) -> Vec<f32> {
    const INF: f32 = 1e20;
    let (w, h) = (w as usize, h as usize);
    let mut d: Vec<f32> = (0..w * h).map(|i| if inside(i) { 0.0 } else { INF }).collect();
    fn pass(f: &mut [f32], n: usize, v: &mut [usize], z: &mut [f32], out: &mut [f32]) {
        let mut k = 0;
        v[0] = 0;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            let mut s;
            loop {
                let p = v[k];
                s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
                if s <= z[k] && k > 0 {
                    k -= 1;
                } else {
                    break;
                }
            }
            if s <= z[k] {
                // k == 0 and the new parabola is lower everywhere.
                v[0] = q;
                z[0] = -INF;
                z[1] = INF;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = INF;
        }
        k = 0;
        for (q, o) in out.iter_mut().enumerate().take(n) {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let p = v[k];
            *o = (q as f32 - p as f32).powi(2) + f[p];
        }
    }
    let n = w.max(h);
    let (mut v, mut z, mut f, mut out) = (vec![0usize; n], vec![0f32; n + 1], vec![0f32; n], vec![0f32; n]);
    for x in 0..w {
        for y in 0..h {
            f[y] = d[y * w + x];
        }
        pass(&mut f, h, &mut v, &mut z, &mut out);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&d[y * w..y * w + w]);
        pass(&mut f, w, &mut v, &mut z, &mut out);
        d[y * w..y * w + w].copy_from_slice(&out[..w]);
    }
    d
}

/// The selection grown by `by` pixels (shrunk when negative), round at the corners.
fn grow(s: &[u8], w: u32, h: u32, by: f64) -> Vec<u8> {
    if by >= 0.0 {
        let d = distance2(|i| s[i] >= 128, w, h);
        d.iter().zip(s).map(|(d, v)| if *d <= (by * by) as f32 { 255 } else { *v }).collect()
    } else {
        let d = distance2(|i| s[i] < 128, w, h);
        d.iter().zip(s).map(|(d, v)| if *d <= (by * by) as f32 { 0 } else { *v }).collect()
    }
}

/// Soft edges: three box blurs of radius `r` (close to a Gaussian).
fn blur(s: &[u8], w: u32, h: u32, r: usize) -> Vec<u8> {
    if r == 0 {
        return s.to_vec();
    }
    let (w, h) = (w as usize, h as usize);
    let mut a: Vec<f32> = s.iter().map(|v| *v as f32).collect();
    let mut b = vec![0f32; a.len()];
    let r = (r / 2).max(1);
    let line = |src: &[f32], dst: &mut [f32], n: usize, at: &dyn Fn(usize) -> usize| {
        let mut sum = 0.0;
        let k = (2 * r + 1) as f32;
        let get = |i: isize| src[at(i.clamp(0, n as isize - 1) as usize)];
        for i in -(r as isize)..=(r as isize) {
            sum += get(i);
        }
        for i in 0..n {
            dst[at(i)] = sum / k;
            sum += get(i as isize + r as isize + 1) - get(i as isize - r as isize);
        }
    };
    for _ in 0..3 {
        for y in 0..h {
            line(&a, &mut b, w, &|x| y * w + x);
        }
        for x in 0..w {
            line(&b, &mut a, h, &|y| y * w + x);
        }
    }
    a.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()
}

/// The edges between selected (≥ 128) and unselected pixels, joined into runs: [x0, y0, x1, y1, …].
fn outline(s: &[u8], w: u32, h: u32) -> Vec<u32> {
    const MAX: usize = 60_000 * 4;
    let on = |x: i64, y: i64| {
        x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && s[(y as u32 * w + x as u32) as usize] >= 128
    };
    let mut out = Vec::new();
    for y in 0..=h as i64 {
        let mut run: Option<i64> = None;
        for x in 0..=w as i64 {
            let edge = x < w as i64 && on(x, y) != on(x, y - 1);
            match (edge, run) {
                (true, None) => run = Some(x),
                (false, Some(x0)) => {
                    out.extend([x0 as u32, y as u32, x as u32, y as u32]);
                    run = None;
                }
                _ => {}
            }
        }
        if out.len() > MAX {
            return out;
        }
    }
    for x in 0..=w as i64 {
        let mut run: Option<i64> = None;
        for y in 0..=h as i64 {
            let edge = y < h as i64 && on(x, y) != on(x - 1, y);
            match (edge, run) {
                (true, None) => run = Some(y),
                (false, Some(y0)) => {
                    out.extend([x as u32, y0 as u32, x as u32, y as u32]);
                    run = None;
                }
                _ => {}
            }
        }
        if out.len() > MAX * 2 {
            return out;
        }
    }
    out
}

/// `img` scaled to cover `w`×`h` (centred; the edges cut off when the shapes differ, never stretched more than 3%).
fn fit_cover(img: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    let (iw, ih) = img.dimensions();
    let (want, have) = (w as f64 / h as f64, iw as f64 / ih as f64);
    let cut = if (want / have - 1.0).abs() < 0.03 {
        image::imageops::crop_imm(img, 0, 0, iw, ih).to_image()
    } else if have > want {
        let cw = (ih as f64 * want).round() as u32;
        image::imageops::crop_imm(img, (iw - cw) / 2, 0, cw, ih).to_image()
    } else {
        let ch = (iw as f64 / want).round() as u32;
        image::imageops::crop_imm(img, 0, (ih - ch) / 2, iw, ch).to_image()
    };
    image::imageops::resize(&cut, w, h, image::imageops::FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_sizes_never_wrap_around() {
        assert_eq!(rgba_len(16384, 16384), Ok(1 << 30));
        // 65536 × 65536 × 4 is 0 in 32 bits.
        assert!(rgba_len(65536, 65536).is_err());
        assert!(rgba_len(16385, 16384).is_err());
        assert!(rgba_len(u32::MAX, u32::MAX).is_err());
    }

    #[test]
    fn grow_and_shrink_round() {
        let (w, h) = (21, 21);
        let mut s = vec![0u8; 441];
        s[10 * 21 + 10] = 255;
        let g = grow(&s, w, h, 5.0);
        assert_eq!(g[10 * 21 + 15], 255);
        assert_eq!(g[15 * 21 + 15], 0, "the corner is round, not square");
        let back = grow(&g, w, h, -4.0);
        assert!(back.iter().filter(|v| **v > 0).count() < 10);
    }

    #[test]
    fn region_matches_see_through_whatever_colour() {
        let px = vec![255, 0, 0, 0, 0, 255, 0, 0, 9, 9, 9, 255];
        let r = region(&px, 3, 1, 0, 0, 0.1, true);
        assert_eq!(r, vec![255, 255, 0]);
    }

    #[test]
    fn polygon_covers_its_area() {
        let r = fill_polygon(
            &[Point::new(2.0, 2.0), Point::new(8.0, 2.0), Point::new(8.0, 6.0), Point::new(2.0, 6.0)],
            10,
            10,
        );
        let total: u32 = r.iter().map(|v| *v as u32).sum();
        assert_eq!(total, 24 * 255);
    }
}
