use crate::{RenderError, create_target, encode, render_params, renderer_options};
use kurbo::Affine;
use vello::wgpu;
use xenify_anim::Scene;
use xenify_doc::Color;

/// Straight-alpha RGBA8 pixels, row-major, no padding.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// Encodes as PNG.
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("PNG header");
            w.write_image_data(&self.pixels).expect("PNG data");
        }
        out
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].try_into().unwrap()
    }
}

/// Headless GPU renderer for export, thumbnails and AI preview frames.
pub struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    adapter_name: String,
    target: Option<(u32, u32, wgpu::Texture, wgpu::Buffer)>,
    vscene: vello::Scene,
    text: crate::TextEngine,
    /// The bars around a frame drawn at another shape (see [`Self::render`]).
    letterbox: Color,
}

impl Offscreen {
    /// Creates its own GPU device (no window needed).
    pub fn new() -> Result<Self, RenderError> {
        // Creating GPU devices on several threads at once can crash some
        // drivers; one at a time is cheap (it's rare).
        static CREATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one_at_a_time = CREATING.lock().unwrap_or_else(|e| e.into_inner());
        // No debug object names on headless devices: the Vulkan loader's
        // naming can race with another thread creating a device (seen as a
        // crash in parallel tests), and nothing here is looked at in a GPU
        // debugger. WGPU_DEBUG=1 turns them back on.
        let debug = std::env::var_os("WGPU_DEBUG").is_some_and(|v| v != "0");
        let mut cx = if debug {
            vello::util::RenderContext::new()
        } else {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                display: None,
                backends: wgpu::Backends::from_env().unwrap_or_default(),
                flags: wgpu::InstanceFlags::empty(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                backend_options: wgpu::BackendOptions::from_env_or_default(),
            });
            vello::util::RenderContext { instance, devices: Vec::new() }
        };
        let id = pollster::block_on(cx.device(None)).ok_or(RenderError::NoAdapter)?;
        let handle = &cx.devices[id];
        let (device, queue) = (handle.device.clone(), handle.queue.clone());
        let adapter_name = handle.adapter().get_info().name;
        let renderer =
            vello::Renderer::new(&device, renderer_options()).map_err(|e| RenderError::Gpu(e.to_string()))?;
        Ok(Self {
            device,
            queue,
            renderer,
            adapter_name,
            target: None,
            vscene: vello::Scene::new(),
            text: crate::TextEngine::new(),
            letterbox: Color::BLACK,
        })
    }

    /// Name of the GPU in use, for diagnostics.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Renders composition `comp` at `time`, with motion blur when the
    /// composition has it: sub-frames across the shutter are averaged (identical
    /// sub-frames are rendered once).
    pub fn render_frame(
        &mut self,
        project: &xenify_doc::Project,
        comp: xenify_doc::Id,
        time: f64,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<RgbaImage, RenderError> {
        let Ok(c) = project.comp(comp) else { return Err(RenderError::Gpu("no such composition".into())) };
        let mut scenes: Vec<(xenify_anim::Scene, u32)> = Vec::new();
        for t in c.settings.shutter_times(time) {
            let mut scene = xenify_anim::evaluate_project(project, comp, t);
            if transparent {
                scene.background = Color::TRANSPARENT;
            }
            match scenes.iter_mut().find(|(s, _)| *s == scene) {
                Some((_, n)) => *n += 1,
                None => scenes.push((scene, 1)),
            }
        }
        if let [(scene, _)] = scenes.as_slice() {
            return self.render(scene, width, height);
        }
        // Average in premultiplied space so transparent edges blend correctly.
        let total: u32 = scenes.iter().map(|s| s.1).sum();
        let mut acc = vec![0.0f32; (width * height * 4) as usize];
        for (scene, n) in &scenes {
            let img = self.render(scene, width, height)?;
            let w = *n as f32;
            for (a, p) in acc.as_chunks_mut::<4>().0.iter_mut().zip(img.pixels.as_chunks::<4>().0) {
                let alpha = p[3] as f32 / 255.0;
                a[0] += p[0] as f32 * alpha * w;
                a[1] += p[1] as f32 * alpha * w;
                a[2] += p[2] as f32 * alpha * w;
                a[3] += alpha * w;
            }
        }
        let pixels = acc
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|a| {
                let alpha = a[3] / total as f32;
                let un = |v: f32| if a[3] > 0.0 { (v / a[3]).round().clamp(0.0, 255.0) as u8 } else { 0 };
                [un(a[0]), un(a[1]), un(a[2]), (alpha * 255.0).round() as u8]
            })
            .collect();
        Ok(RgbaImage { width, height, pixels })
    }

    /// Renders `comp` at `time` to a PNG no larger than `max_side` pixels on its
    /// long side (for AI frame reviews and thumbnails).
    pub fn preview_png(
        &mut self,
        project: &xenify_doc::Project,
        comp: xenify_doc::Id,
        time: f64,
        max_side: u32,
    ) -> Result<Vec<u8>, RenderError> {
        let Ok(comp) = project.comp(comp) else { return Err(RenderError::Gpu("no such composition".into())) };
        let s = &comp.settings;
        let k = (max_side as f64 / s.width.max(s.height) as f64).min(1.0);
        let (w, h) = (((s.width as f64 * k) as u32).max(1), ((s.height as f64 * k) as u32).max(1));
        let id = comp.id;
        Ok(self.render_frame(project, id, time.clamp(0.0, s.duration), w, h, false)?.to_png())
    }

    /// Lays out / measures text with the same fonts the renderer uses.
    pub fn text_engine(&mut self) -> &mut crate::TextEngine {
        &mut self.text
    }

    /// Renders `scene` scaled to `width`×`height` and reads the pixels back.
    pub fn render(&mut self, scene: &Scene, width: u32, height: u32) -> Result<RgbaImage, RenderError> {
        let (sx, sy) = (width as f64 / scene.width.max(1) as f64, height as f64 / scene.height.max(1) as f64);
        self.vscene.reset();
        let view = if (sx / sy - 1.0).abs() < 0.005 {
            Affine::scale_non_uniform(sx, sy)
        } else {
            // Another shape: fitted in the middle, bars around it.
            let k = sx.min(sy);
            let target = kurbo::Rect::new(0.0, 0.0, width as f64, height as f64);
            self.vscene.fill(
                vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                crate::to_peniko(self.letterbox),
                None,
                &target,
            );
            Affine::translate((
                (width as f64 - scene.width as f64 * k) / 2.0,
                (height as f64 - scene.height as f64 * k) / 2.0,
            )) * Affine::scale(k)
        };
        encode(scene, &mut self.vscene, view, &mut self.text);
        self.read_back(width, height)
    }

    /// The colour of the bars when a frame is drawn at another shape (default black).
    pub fn set_letterbox(&mut self, color: Color) {
        self.letterbox = color;
    }

    /// Renders a moment of a presentation (see [`crate::encode_deck`]).
    pub fn render_deck(
        &mut self,
        project: &xenify_doc::Project,
        frame: &xenify_anim::deck::DeckFrame,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, RenderError> {
        self.vscene.reset();
        crate::encode_deck_on(project, frame, &mut self.vscene, width, height, self.letterbox, &mut self.text);
        self.read_back(width, height)
    }

    /// Renders `self.vscene` into a `width`×`height` target and reads the pixels back.
    fn read_back(&mut self, width: u32, height: u32) -> Result<RgbaImage, RenderError> {
        // Rows in a texture→buffer copy must be padded to 256 bytes.
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        if !matches!(&self.target, Some((w, h, ..)) if *w == width && *h == height) {
            let texture = create_target(&self.device, width, height);
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("xenify readback"),
                size: (padded_row * height) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some((width, height, texture, buffer));
        }
        let (_, _, texture, buffer) = self.target.as_ref().unwrap();
        let view_tex = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                &self.vscene,
                &view_tex,
                &render_params(width, height, Color::TRANSPARENT),
            )
            .map_err(|e| RenderError::Gpu(e.to_string()))?;

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|e| RenderError::Gpu(e.to_string()))?;
        rx.recv().map_err(|e| RenderError::Gpu(e.to_string()))?.map_err(|e| RenderError::Gpu(e.to_string()))?;

        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        {
            let data = slice.get_mapped_range();
            for row in data.chunks_exact(padded_row as usize) {
                pixels.extend_from_slice(&row[..(width * 4) as usize]);
            }
        }
        buffer.unmap();
        Ok(RgbaImage { width, height, pixels })
    }
}
