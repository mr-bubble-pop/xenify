//! Xenify Draw's engine in the page: [`xenify_draw_core::Session`] for the
//! calls the screens make, and the canvas drawn with WebGL2 by Vello Hybrid.
//! The desktop app loads this too, so every platform draws the same way.

#[cfg(target_arch = "wasm32")]
mod web {
    use kurbo::{Affine, Rect};
    use std::collections::HashMap;
    use std::sync::Arc;
    use vello_common::paint::{ImageId, ImageSource};
    use vello_hybrid::{RenderSize, Resources, Scene, WebGlRenderer, WebGlTextureBindings};
    use wasm_bindgen::prelude::*;
    use xenify_draw_core::Session;
    use xenify_render::Painter;
    use xenify_render::hybrid::{GlyphCache, HybridPainter, ImageStore};

    /// Uploads each image once, by its data's id (and notes when it was last drawn).
    struct Uploads<'a> {
        renderer: &'a mut WebGlRenderer,
        resources: &'a mut Resources,
        ids: &'a mut HashMap<u64, (ImageId, u64)>,
        /// Where each upload went in the atlas (layer, offset), by its data's id: a raster layer being
        /// painted has its changes written straight there.
        placed: &'a mut HashMap<u64, (u32, [u32; 2])>,
        draw: u64,
    }

    /// Uploads a picture like vello_hybrid does, noting where in the atlas it went.
    struct Placing<'a> {
        pixmap: &'a vello_common::pixmap::Pixmap,
        at: std::cell::Cell<Option<(u32, [u32; 2])>>,
    }

    impl vello_hybrid::WebGlAtlasWriter for Placing<'_> {
        fn width(&self) -> u32 {
            self.pixmap.width() as u32
        }
        fn height(&self) -> u32 {
            self.pixmap.height() as u32
        }
        fn write_to_atlas_layer(
            &self,
            gl: &web_sys::WebGl2RenderingContext,
            atlas: &web_sys::WebGlTexture,
            layer: u32,
            offset: [u32; 2],
            width: u32,
            height: u32,
        ) {
            self.at.set(Some((layer, offset)));
            self.pixmap.write_to_atlas_layer(gl, atlas, layer, offset, width, height);
        }
    }

    impl ImageStore for Uploads<'_> {
        fn image_id(&mut self, image: &peniko::ImageData) -> Option<ImageId> {
            if let Some((id, used)) = self.ids.get_mut(&image.data.id()) {
                *used = self.draw;
                return Some(*id);
            }
            if image.width > u16::MAX as u32 || image.height > u16::MAX as u32 {
                return None;
            }
            let ImageSource::Pixmap(pixmap) = ImageSource::from_peniko_image_data(image) else { return None };
            let placing = Placing { pixmap: &pixmap, at: std::cell::Cell::new(None) };
            let id = self.renderer.upload_image(self.resources, &placing);
            if let Some(at) = placing.at.get() {
                self.placed.insert(image.data.id(), at);
            }
            self.ids.insert(image.data.id(), (id, self.draw));
            Some(id)
        }
    }

    /// Sound for the mix: WAV read here, anything else decoded by the page
    /// (Web Audio) and handed over with `set_pcm`.
    static DECODED: std::sync::Mutex<Option<HashMap<u64, Arc<Vec<f32>>>>> = std::sync::Mutex::new(None);

    /// Video frames, decoded by the page (a <video> element per clip) and
    /// handed over with `set_frame`; the ones asked for and missing are listed
    /// for the page by `frames_wanted`.
    #[derive(Default)]
    struct Frames {
        cache: HashMap<(u64, u64), (xenify_anim::media::Frame, u64)>,
        wanted: Vec<(u64, u64, f64)>,
        tick: u64,
    }

    static FRAMES: std::sync::Mutex<Option<Frames>> = std::sync::Mutex::new(None);

    struct WebFrames;

    impl xenify_anim::media::FrameSource for WebFrames {
        fn frame(&mut self, media: &xenify_anim::media::MediaRef, t: f64) -> Option<xenify_anim::media::Frame> {
            let mut g = FRAMES.lock().ok()?;
            let f = g.get_or_insert_with(Frames::default);
            let index = xenify_anim::media::frame_index(media.fps, t);
            f.tick += 1;
            let tick = f.tick;
            if let Some((frame, used)) = f.cache.get_mut(&(media.asset.0, index)) {
                *used = tick;
                return Some(frame.clone());
            }
            if !f.wanted.iter().any(|w| w.0 == media.asset.0 && w.1 == index) {
                f.wanted.push((media.asset.0, index, t));
            }
            // Not here yet: nothing now (the drawing cache doesn't keep misses), the page fetches it and redraws.
            None
        }
    }

    struct WebPcm;

    impl xenify_audio::PcmSource for WebPcm {
        fn pcm(&mut self, media: &xenify_anim::media::MediaRef) -> Option<Arc<Vec<f32>>> {
            let mut cache = DECODED.lock().ok()?;
            let cache = cache.get_or_insert_with(HashMap::new);
            if let Some(p) = cache.get(&media.hash) {
                return Some(p.clone());
            }
            let pcm = Arc::new(media.data.as_ref().and_then(|d| xenify_audio::read_wav(d))?);
            cache.insert(media.hash, pcm.clone());
            Some(pcm)
        }
    }

    /// The renderer's parts, borrowed apart from the open canvases.
    struct Gpu<'a> {
        renderer: &'a mut WebGlRenderer,
        resources: &'a mut Resources,
        scene: &'a mut Scene,
        glyphs: &'a mut GlyphCache,
        images: &'a mut HashMap<u64, (ImageId, u64)>,
        placed: &'a mut HashMap<u64, (u32, [u32; 2])>,
        bindings: &'a WebGlTextureBindings,
        canvas: &'a web_sys::HtmlCanvasElement,
        draws: &'a mut u64,
    }

    impl Gpu<'_> {
        /// Draws `scene` at `width`×`height` with WebGL and reads the pixels back (top row first).
        fn picture(
            &mut self,
            scene: &xenify_anim::Scene,
            width: u32,
            height: u32,
            text: &mut xenify_render::TextEngine,
        ) -> Result<xenify_export::pdf::Picture, String> {
            let view = Affine::scale_non_uniform(
                width as f64 / scene.width.max(1) as f64,
                height as f64 / scene.height.max(1) as f64,
            );
            self.paint_pixels(width, height, |painter| {
                xenify_render::encode_seen(scene, painter, view, scene.camera, text)
            })
        }

        /// Whatever `paint` draws at `width`×`height`, drawn with WebGL and read back (top row first).
        fn paint_pixels(
            &mut self,
            width: u32,
            height: u32,
            paint: impl FnOnce(&mut HybridPainter<'_>),
        ) -> Result<xenify_export::pdf::Picture, String> {
            use wasm_bindgen::JsCast;
            let (w, h) = (width.clamp(1, 8192), height.clamp(1, 8192));
            let (ow, oh) = (self.canvas.width(), self.canvas.height());
            self.canvas.set_width(w);
            self.canvas.set_height(h);
            *self.scene = Scene::new(w as u16, h as u16);
            {
                *self.draws += 1;
                let mut uploads = Uploads {
                    renderer: self.renderer,
                    resources: self.resources,
                    ids: self.images,
                    placed: self.placed,
                    draw: *self.draws,
                };
                let mut painter = HybridPainter::new(self.scene, &mut uploads, self.glyphs);
                paint(&mut painter);
            }
            let size = RenderSize { width: w, height: h };
            self.renderer.render(self.scene, self.resources, &size, self.bindings).map_err(|x| format!("{x:?}"))?;
            let gl: web_sys::WebGl2RenderingContext =
                self.canvas.get_context("webgl2").ok().flatten().and_then(|c| c.dyn_into().ok()).ok_or("no WebGL2")?;
            let mut px = vec![0u8; (w * h * 4) as usize];
            gl.read_pixels_with_opt_u8_array(
                0,
                0,
                w as i32,
                h as i32,
                web_sys::WebGl2RenderingContext::RGBA,
                web_sys::WebGl2RenderingContext::UNSIGNED_BYTE,
                Some(&mut px),
            )
            .map_err(|x| format!("{x:?}"))?;
            self.canvas.set_width(ow);
            self.canvas.set_height(oh);
            // WebGL rows run bottom to top.
            let row = (w * 4) as usize;
            let mut flipped = Vec::with_capacity(px.len());
            for y in (0..h as usize).rev() {
                flipped.extend_from_slice(&px[y * row..(y + 1) * row]);
            }
            Ok(xenify_export::pdf::Picture { width: w, height: h, pixels: flipped })
        }
    }

    struct WebRaster<'a> {
        gpu: Gpu<'a>,
        text: xenify_render::TextEngine,
    }

    impl xenify_export::pdf::Raster for WebRaster<'_> {
        fn render(
            &mut self,
            scene: &xenify_anim::Scene,
            width: u32,
            height: u32,
        ) -> Result<xenify_export::pdf::Picture, String> {
            self.gpu.picture(scene, width, height, &mut self.text)
        }
    }

    /// Bytes written by an encoder that keeps its writer: read after it's dropped.
    #[derive(Clone, Default)]
    struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl std::io::Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[wasm_bindgen]
    pub struct Engine {
        /// The open canvases (tabs) by id, and the one in front.
        tabs: Vec<(u32, Session)>,
        front: u32,
        next_tab: u32,
        renderer: WebGlRenderer,
        resources: Resources,
        scene: Scene,
        glyphs: GlyphCache,
        images: HashMap<u64, (ImageId, u64)>,
        placed: HashMap<u64, (u32, [u32; 2])>,
        /// Draws so far (pictures not drawn for a while are freed from the GPU).
        draws: u64,
        bindings: WebGlTextureBindings,
        /// Font files added (every canvas and renderer gets them).
        fonts: Vec<Arc<Vec<u8>>>,
        /// The canvas drawn on (PDF pictures are drawn on it, then read back).
        canvas: web_sys::HtmlCanvasElement,
        /// An animated GIF being made, frame by frame.
        gif: Option<(image::codecs::gif::GifEncoder<Shared>, Shared, u32, u32, u32)>,
    }

    /// Panics go to the console with their message (otherwise the page only sees "unreachable").
    #[wasm_bindgen(start)]
    pub fn start() {
        std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&JsValue::from_str(&info.to_string()))));
    }

    fn affine(v: &[f64]) -> Result<Affine, JsValue> {
        if v.len() != 6 {
            return Err(JsValue::from_str("view needs 6 numbers"));
        }
        Ok(Affine::new([v[0], v[1], v[2], v[3], v[4], v[5]]))
    }

    fn xenify_doc_id(id: u64) -> xenify_draw_core::Id {
        xenify_draw_core::Id(id)
    }

    fn now() -> f64 {
        web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now())
    }

    #[wasm_bindgen]
    impl Engine {
        /// Starts on the example poster, drawing into `canvas` (WebGL2).
        #[wasm_bindgen(constructor)]
        pub fn new(canvas: web_sys::HtmlCanvasElement) -> Engine {
            xenify_audio::set_pcm_source(Box::new(WebPcm));
            xenify_anim::media::set_frame_source(Box::new(WebFrames));
            let (renderer, resources) = WebGlRenderer::new(&canvas);
            let canvas = canvas.clone();
            let (w, h) = (canvas.width().clamp(1, 8192) as u16, canvas.height().clamp(1, 8192) as u16);
            Engine {
                tabs: vec![(1, Session::default())],
                front: 1,
                next_tab: 2,
                renderer,
                resources,
                scene: Scene::new(w, h),
                glyphs: GlyphCache::default(),
                images: HashMap::new(),
                placed: HashMap::new(),
                draws: 0,
                bindings: WebGlTextureBindings::new(),
                gif: None,
                fonts: Vec::new(),
                canvas,
            }
        }

        /// A text engine with the added fonts.
        fn text_engine(fonts: &[Arc<Vec<u8>>]) -> xenify_render::TextEngine {
            let mut t = xenify_render::TextEngine::new();
            for f in fonts {
                t.add_font(f.clone());
            }
            t
        }

        /// Adds a font file to every canvas; the families it holds (JSON).
        pub fn add_font(&mut self, bytes: Vec<u8>) -> String {
            let bytes = Arc::new(bytes);
            let mut names = Vec::new();
            for (_, s) in &mut self.tabs {
                names = s.text.add_font(bytes.clone());
            }
            if !names.is_empty() {
                self.fonts.push(bytes);
            }
            serde_json::to_string(&names).unwrap_or_default()
        }

        fn gpu(&mut self) -> Gpu<'_> {
            Gpu {
                renderer: &mut self.renderer,
                resources: &mut self.resources,
                scene: &mut self.scene,
                glyphs: &mut self.glyphs,
                images: &mut self.images,
                placed: &mut self.placed,
                bindings: &self.bindings,
                canvas: &self.canvas,
                draws: &mut self.draws,
            }
        }

        fn session(&mut self) -> &mut Session {
            let front = self.front;
            let i = self.tabs.iter().position(|(id, _)| *id == front).unwrap_or(0);
            &mut self.tabs[i].1
        }

        /// One call from the screens to the canvas in front: JSON arguments in, JSON back.
        pub fn call(&mut self, name: &str, args: &str) -> Result<String, JsValue> {
            let args: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
            // The assistant: frame reviews draw with this renderer.
            if name.starts_with("ai_") {
                let front = self.front;
                let i = self.tabs.iter().position(|(id, _)| *id == front).unwrap_or(0);
                let mut gpu = Gpu {
                    renderer: &mut self.renderer,
                    resources: &mut self.resources,
                    scene: &mut self.scene,
                    glyphs: &mut self.glyphs,
                    images: &mut self.images,
                    placed: &mut self.placed,
                    bindings: &self.bindings,
                    canvas: &self.canvas,
                    draws: &mut self.draws,
                };
                let mut text = Engine::text_engine(&self.fonts);
                let mut png = |scene: &xenify_anim::Scene, w: u32, h: u32| -> Result<Vec<u8>, String> {
                    let pic = gpu.picture(scene, w, h, &mut text)?;
                    let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels).ok_or("bad picture")?;
                    let mut out = std::io::Cursor::new(Vec::new());
                    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
                    Ok(out.into_inner())
                };
                let session = &mut self.tabs[i].1;
                if let Some(r) = session.ai_call_with(name, &args, Some(&mut png)) {
                    return r.map(|v| v.to_string()).map_err(|e| JsValue::from_str(&e));
                }
            }
            // Editing together: calls for a canvas that may not be in front.
            if let Some(tab) = args
                .get("tab")
                .and_then(serde_json::Value::as_u64)
                .filter(|_| name.starts_with("live_") || name.starts_with("remote_"))
            {
                let i = self
                    .tabs
                    .iter()
                    .position(|(id, _)| *id as u64 == tab)
                    .ok_or_else(|| JsValue::from_str("that canvas isn't open"))?;
                return self.tabs[i].1.call(name, &args).map(|v| v.to_string()).map_err(|e| JsValue::from_str(&e));
            }
            self.session().call(name, &args).map(|v| v.to_string()).map_err(|e| JsValue::from_str(&e))
        }

        /// Editing together: changes made on any canvas since last asked, to
        /// send to the server (JSON: [{tab, batch}]).
        pub fn live_out(&mut self) -> String {
            let out: Vec<serde_json::Value> = self
                .tabs
                .iter_mut()
                .filter_map(|(id, s)| s.live_out().map(|b| serde_json::json!({ "id": id, "batch": b })))
                .collect();
            serde_json::Value::from(out).to_string()
        }

        /// Opens an `.xdraw` file in the canvas in front; its state (JSON).
        pub fn open_file(&mut self, bytes: &[u8]) -> Result<String, JsValue> {
            self.session().open_file(bytes).map(|v| v.to_string()).map_err(|e| JsValue::from_str(&e))
        }

        /// The canvas in front as an `.xdraw` file, with a picture of it
        /// and whether something in it moves.
        pub fn save_file(&mut self, preview: Option<Vec<u8>>, animated: bool) -> Result<Vec<u8>, JsValue> {
            self.session().save_file(preview.as_deref(), animated).map_err(|e| JsValue::from_str(&e))
        }

        /// Pictures and clips, kept apart from the canvases that use them
        /// (canvases on a server name them by hash): adds a file's bytes; its hash.
        pub fn media_put(&mut self, bytes: Vec<u8>) -> String {
            xenify_doc::blobs::put(bytes)
        }

        pub fn media_has(&self, hash: &str) -> bool {
            xenify_doc::blobs::has(hash)
        }

        /// A file's bytes by hash (to send to the server), if they're here.
        pub fn media_get(&self, hash: &str) -> Option<Vec<u8>> {
            xenify_doc::blobs::get(hash).map(|a| a.to_vec())
        }

        /// A new, empty canvas in front; its tab id.
        pub fn open_tab(&mut self) -> u32 {
            let id = self.next_tab;
            self.next_tab += 1;
            let mut session = Session::new(xenify_draw_core::empty_project());
            for f in &self.fonts {
                session.text.add_font(f.clone());
            }
            self.tabs.push((id, session));
            self.front = id;
            id
        }

        pub fn switch_tab(&mut self, id: u32) -> bool {
            let found = self.tabs.iter().any(|(t, _)| *t == id);
            if found {
                self.front = id;
            }
            found
        }

        /// Closes a canvas; the one in front afterwards (a new one if that was the last).
        pub fn close_tab(&mut self, id: u32) -> u32 {
            if let Some(i) = self.tabs.iter().position(|(t, _)| *t == id) {
                self.tabs.remove(i);
                if self.front == id {
                    self.front = self.tabs.get(i.saturating_sub(1)).or(self.tabs.first()).map_or(0, |t| t.0);
                }
            }
            xenify_doc::blobs::forget_unused();
            if self.tabs.is_empty() {
                return self.open_tab();
            }
            self.front
        }

        pub fn front_tab(&self) -> u32 {
            self.front
        }

        /// A sound the page decoded (48 kHz stereo, interleaved), by its clip's hash.
        pub fn set_pcm(&mut self, hash: &str, samples: Vec<f32>) {
            let Ok(h) = hash.parse::<u64>() else { return };
            if let Ok(mut c) = DECODED.lock() {
                c.get_or_insert_with(HashMap::new).insert(h, Arc::new(samples));
            }
        }

        pub fn has_pcm(&self, hash: &str) -> bool {
            let Ok(h) = hash.parse::<u64>() else { return false };
            DECODED.lock().map(|c| c.as_ref().is_some_and(|m| m.contains_key(&h))).unwrap_or(false)
        }

        /// The front canvas's sound from `start` to `end` seconds (or the presentation's, with `deck`).
        pub fn mix(&mut self, start: f64, end: f64, deck: bool) -> Vec<f32> {
            self.session().mix(start, end, deck)
        }

        /// Video frames drawn but not here yet: [[clip id, frame number, seconds into the clip], …] (cleared).
        pub fn frames_wanted(&mut self) -> String {
            let Ok(mut g) = FRAMES.lock() else { return "[]".into() };
            let f = g.get_or_insert_with(Frames::default);
            let w = std::mem::take(&mut f.wanted);
            serde_json::to_string(&w).unwrap_or_else(|_| "[]".into())
        }

        /// One decoded video frame (RGBA), for clip `asset`'s frame `index`. Keeps the latest 150.
        pub fn set_frame(&mut self, asset: u64, index: u64, width: u32, height: u32, rgba: Vec<u8>) {
            let Ok(mut g) = FRAMES.lock() else { return };
            let f = g.get_or_insert_with(Frames::default);
            f.tick += 1;
            let frame = xenify_anim::media::Frame { width, height, pixels: Arc::new(rgba) };
            f.cache.insert((asset, index), (frame, f.tick));
            while f.cache.len() > 150 {
                let Some(old) = f.cache.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k) else { break };
                f.cache.remove(&old);
            }
        }

        /// The front canvas at `time` (or every slide, finished) as a PDF:
        /// vectors, with layers that have effects drawn here as pictures.
        pub fn export_pdf(&mut self, time: f64, scale: f64, notes: bool) -> Result<Vec<u8>, JsValue> {
            let front = self.front;
            let (project, comp) = {
                let s = self
                    .tabs
                    .iter()
                    .find(|(id, _)| *id == front)
                    .map(|t| &t.1)
                    .ok_or_else(|| JsValue::from_str("no canvas"))?;
                (s.project().clone(), s.comp)
            };
            let text = Engine::text_engine(&self.fonts);
            let mut raster = WebRaster { gpu: self.gpu(), text };
            let opts = xenify_export::pdf::PdfOptions { scale: scale.max(0.25), notes };
            xenify_export::pdf::export_pdf(&project, comp, time, opts, &mut raster).map_err(|e| JsValue::from_str(&e))
        }

        /// Layer `id` of the front canvas alone, fitted in a `size`×`size` tile, as PNG bytes (Layers panel thumbnails).
        pub fn layer_thumb(&mut self, id: u64, size: u32) -> Result<Vec<u8>, JsValue> {
            let front = self.front;
            let i = self.tabs.iter().position(|(t, _)| *t == front).ok_or_else(|| JsValue::from_str("no canvas"))?;
            let scene = self.tabs[i]
                .1
                .layer_preview(xenify_doc_id(id), size)
                .ok_or_else(|| JsValue::from_str("nothing to show"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let pic = self.gpu().picture(&scene, size, size, &mut text).map_err(|e| JsValue::from_str(&e))?;
            let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
                .ok_or_else(|| JsValue::from_str("bad picture"))?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(out.into_inner())
        }

        /// Insert › 3D model: the model being turned (`pose`: JSON rot_x, rot_y, rot_z in degrees), on a
        /// see-through picture whose longer side is `size` pixels, as PNG bytes.
        pub fn model_view(&mut self, pose: &str, size: u32) -> Result<Vec<u8>, JsValue> {
            let pose: serde_json::Value = serde_json::from_str(pose).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let front = self.front;
            let i = self.tabs.iter().position(|(t, _)| *t == front).ok_or_else(|| JsValue::from_str("no canvas"))?;
            let scene = self.tabs[i]
                .1
                .model_scene(xenify_draw_core::Pose::from_json(&pose), size)
                .ok_or_else(|| JsValue::from_str("nothing to show"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let mut pic = self.gpu().picture(&scene, scene.width, scene.height, &mut text).map_err(|e| JsValue::from_str(&e))?;
            // Its soft edges come back premultiplied.
            for p in pic.pixels.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
                .ok_or_else(|| JsValue::from_str("bad picture"))?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(out.into_inner())
        }

        /// Layer › Convert to raster: layers `ids` (JSON list) drawn here and swapped for a raster layer:
        /// one for all of them (`merge`), else one each. The new layers' ids (JSON).
        pub fn rasterize(&mut self, ids: &str, merge: bool) -> Result<String, JsValue> {
            let ids: Vec<u64> = serde_json::from_str(ids).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let ids: Vec<xenify_draw_core::Id> = ids.into_iter().map(xenify_doc_id).collect();
            let groups: Vec<Vec<xenify_draw_core::Id>> =
                if merge { vec![ids] } else { ids.into_iter().map(|i| vec![i]).collect() };
            let front = self.front;
            let i = self.tabs.iter().position(|(t, _)| *t == front).ok_or_else(|| JsValue::from_str("no canvas"))?;
            let several = groups.len() > 1;
            if several {
                self.tabs[i].1.editor.begin_group("Convert to raster");
            }
            let mut made = Vec::new();
            let mut failed = None;
            for g in groups {
                let Some((scene, w, h)) = self.tabs[i].1.raster_scene(&g) else { continue };
                let mut text = Engine::text_engine(&self.fonts);
                let pic = match self.gpu().picture(&scene, w, h, &mut text) {
                    Ok(p) => p,
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                };
                match self.tabs[i].1.raster_swap(
                    &g,
                    pic.width,
                    pic.height,
                    pic.pixels,
                    (g.len() > 1).then_some("Raster"),
                ) {
                    Ok(id) => made.extend(id.as_u64()),
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            if several {
                self.tabs[i].1.editor.end_group();
            }
            if !made.is_empty() {
                self.tabs[i].1.selection = made.iter().map(|m| xenify_doc_id(*m)).collect();
            }
            match failed {
                Some(e) => Err(JsValue::from_str(&e)),
                None => Ok(serde_json::Value::from(made).to_string()),
            }
        }

        /// What the canvas shows, drawn on raster layer `id`'s pixel grid and kept for its fill, wand and AI
        /// (`sample: "canvas"`, `source: "canvas"`).
        pub fn raster_sample(&mut self, id: u64) -> Result<(), JsValue> {
            let front = self.front;
            let i = self.tabs.iter().position(|(t, _)| *t == front).ok_or_else(|| JsValue::from_str("no canvas"))?;
            let (scene, view, w, h) = self.tabs[i]
                .1
                .raster_sample_view(xenify_doc_id(id))
                .ok_or_else(|| JsValue::from_str("not a raster layer"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let pic = self
                .gpu()
                .paint_pixels(w, h, |p| xenify_render::encode_seen(&scene, p, view, scene.camera, &mut text))
                .map_err(|e| JsValue::from_str(&e))?;
            if pic.width != w || pic.height != h {
                return Err(JsValue::from_str("this layer is too big to look at all at once"));
            }
            self.tabs[i].1.raster_set_sample(xenify_doc_id(id), pic.pixels);
            Ok(())
        }

        /// Frame `frame` (of 12, over 3.3 s) of ready-made effect `name` in its own little scene, as PNG bytes (the effect gallery).
        pub fn effect_preview(&mut self, name: &str, frame: u32, width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
            let scene = xenify_draw_core::effect_preview(name, 0.3 + frame as f64 * 3.3 / 12.0)
                .ok_or_else(|| JsValue::from_str("no preview"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let pic = self.gpu().picture(&scene, width, height, &mut text).map_err(|e| JsValue::from_str(&e))?;
            let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
                .ok_or_else(|| JsValue::from_str("bad picture"))?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(out.into_inner())
        }

        /// A pen (its style, JSON) drawing a sample stroke, as PNG bytes `width`×`height`.
        pub fn pen_thumb(&mut self, style: &str, width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
            let v: serde_json::Value = serde_json::from_str(style).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let scene =
                xenify_draw_core::pen_preview(&v, width, height).ok_or_else(|| JsValue::from_str("not a pen"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let pic = self.gpu().picture(&scene, width, height, &mut text).map_err(|e| JsValue::from_str(&e))?;
            let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
                .ok_or_else(|| JsValue::from_str("bad picture"))?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(out.into_inner())
        }

        /// A slide layout in a theme, finished, as PNG bytes `width`×`height` (gallery thumbnails).
        pub fn layout_thumb(&mut self, theme: &str, layout: &str, width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
            let scene =
                xenify_draw_core::layout_preview(theme, layout).ok_or_else(|| JsValue::from_str("no such layout"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let pic = self.gpu().picture(&scene, width, height, &mut text).map_err(|e| JsValue::from_str(&e))?;
            let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
                .ok_or_else(|| JsValue::from_str("bad picture"))?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(out.into_inner())
        }

        /// Transition `kind` between two sample slides at moment `t` (0: the first slide, 1: the second), as PNG bytes.
        pub fn transition_frame(&mut self, kind: &str, t: f64, width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
            let kind: xenify_doc::TransitionKind =
                serde_json::from_value(serde_json::Value::from(kind)).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let p = xenify_draw_core::transition_sample(kind);
            let deck = xenify_anim::deck::Deck::new(&p);
            // A moment on the first slide, the transition, then a moment on the second.
            let frame = deck.at(0.8 + 1.4 * t.clamp(0.0, 1.0)).ok_or_else(|| JsValue::from_str("no frame"))?;
            let mut text = Engine::text_engine(&self.fonts);
            let pic = self
                .gpu()
                .paint_pixels(width, height, |painter| {
                    xenify_render::encode_deck(&p, &frame, painter, width, height, &mut text)
                })
                .map_err(|e| JsValue::from_str(&e))?;
            let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
                .ok_or_else(|| JsValue::from_str("bad picture"))?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(out.into_inner())
        }

        /// A generated sound (a recipe, as JSON) rendered: 48 kHz stereo, interleaved (for previews).
        pub fn recipe_pcm(&self, recipe: &str) -> Result<Vec<f32>, JsValue> {
            let r: xenify_audio::synth::Recipe =
                serde_json::from_str(recipe).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(r.render())
        }

        /// Starts an animated GIF of `width`×`height` at `fps`, repeating forever when `looping`.
        pub fn gif_start(&mut self, width: u32, height: u32, fps: f64, looping: bool) -> Result<(), JsValue> {
            let out = Shared::default();
            let mut enc = image::codecs::gif::GifEncoder::new_with_speed(out.clone(), 10);
            enc.set_repeat(if looping {
                image::codecs::gif::Repeat::Infinite
            } else {
                image::codecs::gif::Repeat::Finite(0)
            })
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let delay = (100.0 / fps.max(1.0)).round().max(2.0) as u32;
            self.gif = Some((enc, out, width, height, delay));
            Ok(())
        }

        /// Adds one frame (RGBA pixels, top row first).
        pub fn gif_frame(&mut self, rgba: Vec<u8>) -> Result<(), JsValue> {
            let Some((enc, _, w, h, delay)) = self.gif.as_mut() else {
                return Err(JsValue::from_str("no GIF started"));
            };
            let img = image::RgbaImage::from_raw(*w, *h, rgba).ok_or_else(|| JsValue::from_str("wrong frame size"))?;
            let frame = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(*delay * 10, 1));
            enc.encode_frame(frame).map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// The finished GIF file.
        pub fn gif_finish(&mut self) -> Result<Vec<u8>, JsValue> {
            let (enc, out, ..) = self.gif.take().ok_or_else(|| JsValue::from_str("no GIF started"))?;
            drop(enc);
            Ok(out.0.take())
        }

        /// Draws the current frame into the `width`×`height` drawing buffer,
        /// composition space mapped by `view` (a, b, c, d, e, f) over a
        /// `pasteboard` background. Returns how long it took: [encode ms, render ms].
        pub fn render(&mut self, view: &[f64], width: u32, height: u32, pasteboard: &str) -> Result<Vec<f64>, JsValue> {
            let view = affine(view)?;
            self.draw(width, height, pasteboard, |s, p| s.paint(p, view))
        }

        /// Draws the presentation at deck time `t` (presenting), letterboxed in black.
        pub fn render_deck(&mut self, t: f64, width: u32, height: u32) -> Result<Vec<f64>, JsValue> {
            self.draw(width, height, "#000000", |s, p| s.paint_deck(p, t, width, height))
        }

        /// Draws canvas `comp` at `time` mapped by `view` (slide thumbnails).
        pub fn render_comp(
            &mut self,
            comp: u64,
            time: f64,
            view: &[f64],
            width: u32,
            height: u32,
            pasteboard: &str,
        ) -> Result<Vec<f64>, JsValue> {
            let view = affine(view)?;
            self.draw(width, height, pasteboard, |s, p| s.paint_comp(p, xenify_doc_id(comp), time, view))
        }

        /// A raster layer being painted: the pixels changed since the last frame, written into its upload.
        fn write_painted(&mut self) {
            let front = self.front;
            let Some((_, session)) = self.tabs.iter_mut().find(|(id, _)| *id == front) else { return };
            let Some((hash, [x, y, w, h], px)) = session.raster_take_dirty() else { return };
            let Some(&(layer, at)) = xenify_render::images::live_blob(hash).and_then(|b| self.placed.get(&b)) else {
                return;
            };
            let gl = self.renderer.gl_context();
            gl.active_texture(web_sys::WebGl2RenderingContext::TEXTURE0);
            gl.bind_texture(web_sys::WebGl2RenderingContext::TEXTURE_2D_ARRAY, Some(self.renderer.atlas_texture()));
            let _ = gl.tex_sub_image_3d_with_opt_u8_array(
                web_sys::WebGl2RenderingContext::TEXTURE_2D_ARRAY,
                0,
                (at[0] + x) as i32,
                (at[1] + y) as i32,
                layer as i32,
                w as i32,
                h as i32,
                1,
                web_sys::WebGl2RenderingContext::RGBA,
                web_sys::WebGl2RenderingContext::UNSIGNED_BYTE,
                Some(&px),
            );
        }

        fn draw(
            &mut self,
            width: u32,
            height: u32,
            pasteboard: &str,
            paint: impl FnOnce(&mut Session, &mut HybridPainter<'_>),
        ) -> Result<Vec<f64>, JsValue> {
            let (w, h) = (width.clamp(1, 8192) as u16, height.clamp(1, 8192) as u16);
            let t0 = now();
            if self.scene.width() != w || self.scene.height() != h {
                self.scene = Scene::new(w, h);
            } else {
                self.scene.reset();
            }
            let ground = peniko::color::parse_color(pasteboard)
                .map(|c| c.to_alpha_color::<peniko::color::Srgb>())
                .unwrap_or(peniko::Color::WHITE);
            {
                self.draws += 1;
                let mut uploads = Uploads {
                    renderer: &mut self.renderer,
                    resources: &mut self.resources,
                    ids: &mut self.images,
                    placed: &mut self.placed,
                    draw: self.draws,
                };
                let mut painter = HybridPainter::new(&mut self.scene, &mut uploads, &mut self.glyphs);
                painter.fill(
                    peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    ground,
                    None,
                    &Rect::new(0.0, 0.0, w as f64, h as f64),
                );
                let front = self.front;
                if let Some((_, session)) = self.tabs.iter_mut().find(|(id, _)| *id == front) {
                    paint(session, &mut painter);
                }
            }
            self.write_painted();
            let t1 = now();
            let size = RenderSize { width: w as u32, height: h as u32 };
            self.renderer
                .render(&self.scene, &mut self.resources, &size, &self.bindings)
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            // Video frames and pictures not drawn in the last few seconds leave the GPU; while a raster
            // layer is painted (new pixels every frame), whatever wasn't drawn this time goes at once.
            let painting = xenify_render::images::painting();
            if painting || self.draws % 60 == 0 {
                let age = if painting { 0 } else { 240 };
                let old: Vec<u64> =
                    self.images.iter().filter(|(_, (_, used))| self.draws - used > age).map(|(k, _)| *k).collect();
                for k in old {
                    self.placed.remove(&k);
                    if let Some((id, _)) = self.images.remove(&k) {
                        self.renderer.destroy_image(&mut self.resources, id);
                    }
                }
            }
            Ok(vec![t1 - t0, now() - t1])
        }
    }
}
