//! PDF export: one page per slide (a handout or a deck to send around),
//! optionally with the speaker notes under each slide.
//!
//! Pages are vector graphics: shapes, strokes (with dashes), gradients,
//! images, groups with opacity and blend modes, and text as exact outlines
//! with an invisible text layer on top so it can be selected and searched.
//! Layers PDF can't describe (blur, shadows, glows, masks, mattes, pattern
//! fills) are drawn as sharp pictures of just that layer.

use xenify_anim::{DrawItem, Scene, SceneContent, SceneLayer, TextMeasure};
use xenify_doc::{BlendMode, Color, FillRule, GradientKind, Id, LineCap, LineJoin, Project};
use xenify_render::{Offscreen, RgbaImage, TextEngine};
use kurbo::{Affine, BezPath, PathEl, Rect};
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfOptions {
    /// Resolution of pictures in the PDF (layers drawn as images), relative
    /// to the composition (2× prints sharply).
    pub scale: f64,
    /// Put each slide's speaker notes under it (a portrait handout page).
    pub notes: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self { scale: 2.0, notes: false }
    }
}

/// Every slide as it looks finished (its last frame), or just `comp` at
/// `time` when the project isn't a presentation.
pub fn export_pdf(
    project: &Project,
    comp: Id,
    time: f64,
    options: PdfOptions,
    gpu: &mut Offscreen,
) -> Result<Vec<u8>, String> {
    let shots: Vec<(Id, f64, String)> = if project.slides.is_empty() {
        vec![(comp, time, String::new())]
    } else {
        project
            .slides
            .iter()
            .map(|s| {
                let d = project.comp(s.comp).map_or(0.0, |c| c.settings.duration);
                (s.comp, d, s.notes.clone())
            })
            .collect()
    };
    let mut doc = Doc::new();
    let mut text = TextEngine::new();
    for (comp, time, notes) in shots {
        let scene = xenify_anim::evaluate_project(project, comp, time);
        let notes = options.notes.then_some(notes.as_str());
        doc.page(&scene, notes, options.scale, gpu, &mut text)?;
    }
    Ok(doc.finish())
}

/// Slides are laid out 10 inches wide (72 points per inch), like
/// presentation apps; handout pages (with notes) are US Letter portrait.
const SLIDE_WIDTH: f64 = 720.0;
const HANDOUT: (f64, f64) = (612.0, 792.0);
const MARGIN: f64 = 40.0;
const NOTES_SIZE: f64 = 12.0;

/// Object 1 is the catalog, 2 the page tree, 3 Helvetica; the rest follow.
struct Doc {
    objects: Vec<Vec<u8>>,
    pages: Vec<usize>,
}

/// One page's content stream and the resources it uses.
struct Page<'a> {
    doc: &'a mut Doc,
    out: String,
    /// Scene pixels → page points (y up); shading patterns are placed in it.
    base: Affine,
    ext: Vec<(String, usize)>,
    xobjects: Vec<(String, usize)>,
    patterns: Vec<(String, usize)>,
    alpha_states: HashMap<(u32, u32, u8), String>,
    /// Resolution of layers drawn as pictures.
    raster_scale: f64,
}

impl Doc {
    fn new() -> Self {
        Self { objects: vec![Vec::new(), Vec::new(), helvetica()], pages: Vec::new() }
    }

    /// Adds an object; returns its number.
    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        self.objects.len()
    }

    fn page(
        &mut self,
        scene: &Scene,
        notes: Option<&str>,
        raster_scale: f64,
        gpu: &mut Offscreen,
        text: &mut TextEngine,
    ) -> Result<(), String> {
        let (sw, sh) = (scene.width.max(1) as f64, scene.height.max(1) as f64);
        // Where the slide goes on the page, in points.
        let (pw, ph, frame) = match notes {
            None => {
                let h = SLIDE_WIDTH * sh / sw;
                (SLIDE_WIDTH, h, Rect::new(0.0, 0.0, SLIDE_WIDTH, h))
            }
            Some(_) => {
                let w = HANDOUT.0 - 2.0 * MARGIN;
                let h = (w * sh / sw).min(HANDOUT.1 * 0.5);
                let w = h * sw / sh;
                let x = (HANDOUT.0 - w) / 2.0;
                let top = HANDOUT.1 - MARGIN;
                (HANDOUT.0, HANDOUT.1, Rect::new(x, top - h, x + w, top))
            }
        };
        // Scene pixels (y down) → page points (y up).
        let k = frame.width() / sw;
        let frame_base = Affine::new([k, 0.0, 0.0, -k, frame.x0, frame.y1]);
        // Layers are in composition space, seen through the camera.
        let base = frame_base * scene.camera;
        let mut page = Page {
            doc: self,
            out: String::new(),
            base,
            ext: Vec::new(),
            xobjects: Vec::new(),
            patterns: Vec::new(),
            alpha_states: HashMap::new(),
            raster_scale,
        };
        let _ = writeln!(page.out, "q {} cm", mat(frame_base));
        let bg = scene.background;
        if bg.a > 0.0 {
            page.fill_colour(bg);
            let _ = writeln!(page.out, "0 0 {sw} {sh} re f");
        }
        // Everything is clipped to the frame, as on screen.
        let _ = writeln!(page.out, "0 0 {sw} {sh} re W n");
        if scene.camera != Affine::IDENTITY {
            let _ = writeln!(page.out, "{} cm", mat(scene.camera));
        }
        for layer in &scene.layers {
            page.layer(scene, layer, gpu, text)?;
        }
        page.out.push_str("Q\n");
        if notes.is_some() {
            let _ =
                writeln!(page.out, "0.6 G 0.5 w {} {} {} {} re S", frame.x0, frame.y0, frame.width(), frame.height());
        }
        if let Some(n) = notes.filter(|n| !n.trim().is_empty()) {
            let lines = wrap(n, NOTES_SIZE, pw - 2.0 * MARGIN);
            let top = frame.y0 - 28.0;
            let _ = writeln!(page.out, "0 g BT /F1 {NOTES_SIZE} Tf {:.2} TL {MARGIN:.2} {top:.2} Td", NOTES_SIZE * 1.4);
            for line in lines.iter().take(((top - MARGIN) / (NOTES_SIZE * 1.4)).max(0.0) as usize) {
                let _ = writeln!(page.out, "({}) Tj T*", escape(line));
            }
            page.out.push_str("ET\n");
        }
        let resources = page.resources();
        let content = std::mem::take(&mut page.out).into_bytes();
        let cid = self.add(stream("", &content, true));
        let pid = self.add(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources {resources} /Contents {cid} 0 R >>"
            )
            .into_bytes(),
        );
        self.pages.push(pid);
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        self.objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
        let kids: String = self.pages.iter().map(|id| format!("{id} 0 R ")).collect();
        self.objects[1] = format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", self.pages.len()).into_bytes();
        let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend(body);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.objects.len() + 1)
                .as_bytes(),
        );
        out
    }
}

fn helvetica() -> Vec<u8> {
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec()
}

/// A PDF matrix operand list for `a`.
fn mat(a: Affine) -> String {
    let c = a.as_coeffs();
    format!("{:.5} {:.5} {:.5} {:.5} {:.4} {:.4}", c[0], c[1], c[2], c[3], c[4], c[5])
}

impl Page<'_> {
    fn resources(&self) -> String {
        let dict =
            |items: &[(String, usize)]| items.iter().map(|(n, id)| format!("/{n} {id} 0 R ")).collect::<String>();
        format!(
            "<< /Font << /F1 3 0 R >> /ExtGState << {} >> /XObject << {} >> /Pattern << {} >> >>",
            dict(&self.ext),
            dict(&self.xobjects),
            dict(&self.patterns)
        )
    }

    /// A graphics state setting fill/stroke opacity and blend mode.
    fn gstate(&mut self, fill_alpha: f64, stroke_alpha: f64, blend: BlendMode) -> String {
        let key = ((fill_alpha * 1000.0).round() as u32, (stroke_alpha * 1000.0).round() as u32, blend as u8);
        if let Some(n) = self.alpha_states.get(&key) {
            return n.clone();
        }
        let name = format!("G{}", self.ext.len());
        let id = self.doc.add(
            format!("<< /Type /ExtGState /ca {fill_alpha:.3} /CA {stroke_alpha:.3} /BM /{} >>", pdf_blend(blend))
                .into_bytes(),
        );
        self.ext.push((name.clone(), id));
        self.alpha_states.insert(key, name.clone());
        name
    }

    fn fill_colour(&mut self, c: Color) {
        let _ = writeln!(self.out, "{:.4} {:.4} {:.4} rg", c.r, c.g, c.b);
        if c.a < 1.0 {
            let g = self.gstate(c.a, 1.0, BlendMode::Normal);
            let _ = writeln!(self.out, "/{g} gs");
        }
    }

    /// Whether a layer needs drawing as a picture (PDF has no equivalent).
    fn needs_raster(layer: &SceneLayer) -> bool {
        let uneven_alpha = |g: &xenify_doc::Gradient| g.stops.iter().any(|s| s.color.a < 0.999);
        let items_need = |items: &[DrawItem]| {
            items.iter().any(|i| {
                i.pattern.is_some()
                    || i.gradient.as_ref().is_some_and(uneven_alpha)
                    || i.stroke.as_ref().is_some_and(|s| s.gradient.as_ref().is_some_and(uneven_alpha))
            })
        };
        !layer.effects.is_empty()
            || layer.matte.is_some()
            || !layer.masks.is_empty()
            || match &layer.content {
                SceneContent::Shapes(items) => items_need(items),
                SceneContent::Image(img) => !img.adjust.is_empty(),
                _ => false,
            }
    }

    fn layer(
        &mut self,
        scene: &Scene,
        layer: &SceneLayer,
        gpu: &mut Offscreen,
        text: &mut TextEngine,
    ) -> Result<(), String> {
        if layer.opacity <= 0.0 {
            return Ok(());
        }
        if Self::needs_raster(layer) {
            return self.raster_layer(scene, layer, gpu);
        }
        self.out.push_str("q\n");
        // Group opacity and blend: the layer as one transparency group.
        let grouped = layer.opacity < 1.0 || !layer.blend.is_normal();
        let saved = if grouped { Some(std::mem::take(&mut self.out)) } else { None };
        match &layer.content {
            SceneContent::Shapes(items) => {
                let _ = writeln!(self.out, "{} cm", mat(layer.transform));
                for item in items {
                    self.item(item, layer.transform);
                }
            }
            SceneContent::Text(t) => {
                let _ = writeln!(self.out, "{} cm", mat(layer.transform));
                if t.is_animated() {
                    // Letters animating in: each glyph with its own opacity.
                    for (glyph, opacity) in text.glyph_outlines(t).into_iter().filter(|g| g.1 >= 0.004) {
                        let glyph = layer.distort.map_or(glyph.clone(), |h| h.map_path(&glyph));
                        self.out.push_str("q\n");
                        self.fill_colour(Color { a: t.color.a * opacity, ..t.color });
                        self.path(&glyph);
                        self.out.push_str("f Q\n");
                    }
                } else {
                    let outline = match layer.distort {
                        Some(h) => h.map_path(&text.outline(t)),
                        None => text.outline(t),
                    };
                    self.fill_colour(t.color);
                    self.path(&outline);
                    self.out.push_str("f\n");
                }
                // Invisible text over the outlines: selectable and searchable.
                if t.path.is_none() && layer.distort.is_none() && !t.text.trim().is_empty() {
                    self.invisible_text(t, text);
                }
            }
            SceneContent::Image(img) => {
                let _ = writeln!(self.out, "{} cm", mat(layer.transform));
                if let Some(name) = self.image(img) {
                    let _ = writeln!(
                        self.out,
                        "q {} 0 0 {} 0 {} cm /{name} Do Q",
                        img.width,
                        -(img.height as f64),
                        img.height
                    );
                }
            }
            SceneContent::Precomp { width, height, layers } => {
                // Clip to the precomp's frame (its layers are in scene space).
                let r = kurbo::Shape::to_path(&Rect::new(0.0, 0.0, *width, *height), 0.1);
                self.path(&(layer.transform * r));
                self.out.push_str("W n\n");
                for l in layers {
                    self.layer(scene, l, gpu, text)?;
                }
            }
            SceneContent::Group(layers) => {
                for l in layers {
                    self.layer(scene, l, gpu, text)?;
                }
            }
        }
        if let Some(outer) = saved {
            let inner = std::mem::replace(&mut self.out, outer);
            // The form uses the page's resources (as registered so far).
            let resources = self.resources();
            let form = stream(
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [-100000 -100000 100000 100000] /Group << /S /Transparency /I true >> /Resources {resources}"
                ),
                inner.as_bytes(),
                true,
            );
            let id = self.doc.add(form);
            let name = format!("X{}", self.xobjects.len());
            self.xobjects.push((name.clone(), id));
            let g = self.gstate(layer.opacity, layer.opacity, layer.blend);
            let _ = writeln!(self.out, "/{g} gs /{name} Do");
        }
        self.out.push_str("Q\n");
        Ok(())
    }

    fn path(&mut self, p: &BezPath) {
        let mut last = kurbo::Point::ZERO;
        for el in p.elements() {
            match *el {
                PathEl::MoveTo(a) => {
                    let _ = writeln!(self.out, "{:.3} {:.3} m", a.x, a.y);
                    last = a;
                }
                PathEl::LineTo(a) => {
                    let _ = writeln!(self.out, "{:.3} {:.3} l", a.x, a.y);
                    last = a;
                }
                PathEl::QuadTo(q, a) => {
                    let c1 = last + (q - last) * (2.0 / 3.0);
                    let c2 = a + (q - a) * (2.0 / 3.0);
                    let _ =
                        writeln!(self.out, "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c", c1.x, c1.y, c2.x, c2.y, a.x, a.y);
                    last = a;
                }
                PathEl::CurveTo(c1, c2, a) => {
                    let _ =
                        writeln!(self.out, "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c", c1.x, c1.y, c2.x, c2.y, a.x, a.y);
                    last = a;
                }
                PathEl::ClosePath => self.out.push_str("h\n"),
            }
        }
    }

    /// A gradient as a shading pattern. Patterns live in the page's default
    /// space, so `to_page` places the gradient's own coordinates.
    fn gradient_pattern(&mut self, g: &xenify_doc::Gradient, to_page: Affine) -> String {
        let mut stops = g.sorted_stops();
        if stops.len() == 1 {
            stops.push(stops[0]);
        }
        // Stitched straight blends between consecutive stops.
        let funcs: Vec<String> = stops
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0].color, w[1].color);
                format!(
                    "<< /FunctionType 2 /Domain [0 1] /C0 [{:.4} {:.4} {:.4}] /C1 [{:.4} {:.4} {:.4}] /N 1 >>",
                    a.r, a.g, a.b, b.r, b.g, b.b
                )
            })
            .collect();
        let (first, last) = (stops[0].offset, stops[stops.len() - 1].offset.max(stops[0].offset + 1e-6));
        let function = if funcs.len() == 1 {
            funcs[0].clone()
        } else {
            // Stops mapped onto 0..1 of the drawn axis (which spans first..last).
            let span = last - first;
            let bounds: Vec<String> =
                stops[1..stops.len() - 1].iter().map(|s| format!("{:.5}", (s.offset - first) / span)).collect();
            let encode: Vec<&str> = funcs.iter().map(|_| "0 1").collect();
            format!(
                "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
                funcs.join(" "),
                bounds.join(" "),
                encode.join(" ")
            )
        };
        let shading = match g.kind {
            GradientKind::Linear => {
                let d = g.end - g.start;
                let (p0, p1) = (g.start + d * first, g.start + d * last);
                format!(
                    "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [{:.3} {:.3} {:.3} {:.3}] /Function {function} /Extend [true true] >>",
                    p0.x, p0.y, p1.x, p1.y
                )
            }
            GradientKind::Radial => {
                let r = (g.end - g.start).hypot();
                format!(
                    "<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [{:.3} {:.3} {:.3} {:.3} {:.3} {:.3}] /Function {function} /Extend [true true] >>",
                    g.start.x,
                    g.start.y,
                    r * first,
                    g.start.x,
                    g.start.y,
                    r * last
                )
            }
        };
        let id =
            self.doc.add(format!("<< /PatternType 2 /Matrix [{}] /Shading {shading} >>", mat(to_page)).into_bytes());
        let name = format!("P{}", self.patterns.len());
        self.patterns.push((name.clone(), id));
        name
    }

    fn item(&mut self, item: &DrawItem, layer_to_scene: Affine) {
        let to_page = self.base * layer_to_scene;
        let even_odd = item.fill_rule == FillRule::EvenOdd;
        if let Some(fill) = item.fill {
            self.out.push_str("q\n");
            match &item.gradient {
                Some(g) => {
                    let p = self.gradient_pattern(g, to_page);
                    let _ = writeln!(self.out, "/Pattern cs /{p} scn");
                    let a = g.stops.first().map_or(1.0, |s| s.color.a);
                    if a < 1.0 {
                        let gs = self.gstate(a, 1.0, BlendMode::Normal);
                        let _ = writeln!(self.out, "/{gs} gs");
                    }
                }
                None => self.fill_colour(fill),
            }
            self.path(&item.path);
            self.out.push_str(if even_odd { "f*\nQ\n" } else { "f\nQ\n" });
        }
        let Some(s) = &item.stroke else { return };
        if let Some(marks) = &s.marks {
            // Pen styles are filled shapes.
            for m in marks {
                self.out.push_str("q\n");
                self.fill_colour(Color { a: s.color.a * m.alpha, ..s.color });
                self.path(&m.path);
                self.out.push_str("f\nQ\n");
            }
            return;
        }
        if s.width <= 0.0 {
            return;
        }
        self.out.push_str("q\n");
        match &s.gradient {
            Some(g) => {
                let p = self.gradient_pattern(g, to_page);
                let _ = writeln!(self.out, "/Pattern CS /{p} SCN");
            }
            None => {
                let c = s.color;
                let _ = writeln!(self.out, "{:.4} {:.4} {:.4} RG", c.r, c.g, c.b);
                if c.a < 1.0 {
                    let gs = self.gstate(1.0, c.a, BlendMode::Normal);
                    let _ = writeln!(self.out, "/{gs} gs");
                }
            }
        }
        let cap = match s.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match s.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        let _ = writeln!(self.out, "{:.3} w {cap} J {join} j", s.width);
        if !s.dashes.is_empty() {
            let d: Vec<String> = s.dashes.iter().map(|v| format!("{v:.3}")).collect();
            let _ = writeln!(self.out, "[{}] {:.3} d", d.join(" "), s.dash_offset);
        }
        self.path(&item.path);
        self.out.push_str("S\nQ\n");
    }

    /// Text drawn invisibly (render mode 3) over its outlines, stretched to
    /// the measured box so selecting it highlights the right place.
    fn invisible_text(&mut self, t: &xenify_anim::SceneText, text: &mut TextEngine) {
        let lines = text.text_lines(t);
        let size = t.font_size.max(1.0);
        let clip = t.frame().filter(|f| f.clip).map(|f| f.rect);
        self.out.push_str("BT 3 Tr\n");
        for line in &lines {
            let words = line.text.trim_end();
            // Lines cut off by a box that hides overflow aren't there to find.
            if words.is_empty() || clip.is_some_and(|c| line.baseline < c.y0 || line.baseline > c.y1) {
                continue;
            }
            let w = text_width(words, size).max(1e-6);
            let stretch = ((line.x1 - line.x0) / w * 100.0).clamp(1.0, 1000.0);
            // The text matrix turns it upright.
            let _ = writeln!(
                self.out,
                "/F1 {size:.2} Tf {stretch:.1} Tz 1 0 0 -1 {:.3} {:.3} Tm ({}) Tj",
                line.x0,
                line.baseline,
                escape(words)
            );
        }
        self.out.push_str("ET\n");
    }

    /// An image as an XObject (its transparency as a soft mask).
    fn image(&mut self, img: &xenify_anim::SceneImage) -> Option<String> {
        let (w, h, pixels) = crate::image::scene_pixels(img)?;
        let rgba = RgbaImage { width: w, height: h, pixels };
        Some(self.rgba_xobject(&rgba))
    }

    fn rgba_xobject(&mut self, img: &RgbaImage) -> String {
        let px = img.pixels.as_chunks::<4>().0;
        let rgb: Vec<u8> = px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let alpha: Vec<u8> = px.iter().map(|p| p[3]).collect();
        let smask = if alpha.iter().any(|a| *a < 255) {
            let id = self.doc.add(stream(
                &format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8",
                    img.width, img.height
                ),
                &alpha,
                true,
            ));
            format!(" /SMask {id} 0 R")
        } else {
            String::new()
        };
        let id = self.doc.add(stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8{smask}",
                img.width, img.height
            ),
            &rgb,
            true,
        ));
        let name = format!("X{}", self.xobjects.len());
        self.xobjects.push((name.clone(), id));
        name
    }

    /// Draws one layer as a transparent picture covering the frame.
    fn raster_layer(&mut self, scene: &Scene, layer: &SceneLayer, gpu: &mut Offscreen) -> Result<(), String> {
        let solo = Scene {
            width: scene.width,
            height: scene.height,
            background: Color::rgba(0.0, 0.0, 0.0, 0.0),
            layers: vec![layer.clone()],
            camera: scene.camera,
            time: scene.time,
        };
        let (w, h) = (
            ((scene.width as f64 * self.raster_scale).round() as u32).max(1),
            ((scene.height as f64 * self.raster_scale).round() as u32).max(1),
        );
        let img = gpu.render(&solo, w, h).map_err(|e| e.to_string())?;
        let name = self.rgba_xobject(&img);
        // The picture is already seen through the camera: undo its transform.
        let uncamera = if scene.camera == Affine::IDENTITY {
            String::new()
        } else {
            format!("{} cm ", mat(scene.camera.inverse()))
        };
        let _ = writeln!(
            self.out,
            "q {uncamera}{} 0 0 {} 0 {} cm /{name} Do Q",
            scene.width,
            -(scene.height as f64),
            scene.height
        );
        Ok(())
    }
}

/// PDF's name for a blend mode.
fn pdf_blend(b: BlendMode) -> &'static str {
    match b {
        BlendMode::Normal => "Normal",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::Darken => "Darken",
        BlendMode::Lighten => "Lighten",
        BlendMode::ColorDodge => "ColorDodge",
        BlendMode::ColorBurn => "ColorBurn",
        BlendMode::HardLight => "HardLight",
        BlendMode::SoftLight => "SoftLight",
        BlendMode::Difference => "Difference",
        BlendMode::Exclusion => "Exclusion",
        BlendMode::Hue => "Hue",
        BlendMode::Saturation => "Saturation",
        BlendMode::Color => "Color",
        BlendMode::Luminosity => "Luminosity",
        // PDF has no additive mode; Screen is the closest.
        BlendMode::Add => "Screen",
    }
}

/// A stream object, Flate-compressed when `compress`.
fn stream(dict: &str, data: &[u8], compress: bool) -> Vec<u8> {
    let (data, filter) = if compress {
        (miniz_oxide::deflate::compress_to_vec_zlib(data, 6), " /Filter /FlateDecode")
    } else {
        (data.to_vec(), "")
    };
    let mut out = format!("<< {dict}{filter} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

/// Escapes text for a PDF string in WinAnsi (Latin-1-ish) encoding;
/// characters it can't show become `?`.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '‘' | '’' => out.push('\''),
            '“' | '”' => out.push('"'),
            '–' | '—' => out.push('-'),
            '•' => out.push_str("\\225"),
            c if (c as u32) < 0x20 => {}
            c if (c as u32) < 0x80 => out.push(c),
            c if (c as u32) < 0x100 => {
                let _ = write!(out, "\\{:03o}", c as u32);
            }
            _ => out.push('?'),
        }
    }
    out
}

/// Helvetica advance widths (1/1000 em) for ASCII 32..=126.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556,
    556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334,
    260, 334, 584,
];

fn text_width(s: &str, size: f64) -> f64 {
    s.chars()
        .map(|c| match c as u32 {
            n @ 32..=126 => HELVETICA[(n - 32) as usize] as f64,
            _ => 556.0,
        })
        .sum::<f64>()
        * size
        / 1000.0
}

/// Breaks `text` into lines no wider than `width` points (keeping its own
/// line breaks; very long words are split).
fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&candidate, size) <= width || line.is_empty() && text_width(word, size) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for c in word.chars() {
                if text_width(&format!("{line}{c}"), size) > width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_wrap_to_the_page() {
        let lines = wrap("one two three four five six seven eight nine ten\nnew line", 12.0, 100.0);
        assert!(lines.len() > 2);
        assert!(lines.iter().all(|l| text_width(l, 12.0) <= 100.0), "{lines:?}");
        assert_eq!(lines.last().unwrap(), "new line");
        assert!(wrap(&"x".repeat(100), 12.0, 50.0).len() > 5);
        assert_eq!(escape("(a) café — b"), r"\(a\) caf\351 - b");
    }
}
