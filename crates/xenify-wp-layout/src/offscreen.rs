//! Renders Vello scenes to pixels without a window (golden tests, page
//! thumbnails, PNG export).

use vello::wgpu;

#[derive(Debug)]
pub struct NoGpu(pub String);

pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA, row by row.
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].try_into().expect("4 bytes")
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&self.pixels).expect("png data");
        w.finish().expect("png end");
        out
    }
}

pub struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
}

impl Offscreen {
    pub fn new() -> Result<Self, NoGpu> {
        // Creating GPU devices on several threads at once can crash some drivers.
        static CREATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = CREATING.lock().unwrap_or_else(|e| e.into_inner());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            display: None,
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            flags: wgpu::InstanceFlags::empty(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
        });
        let mut cx = vello::util::RenderContext { instance, devices: Vec::new() };
        let id = pollster::block_on(cx.device(None)).ok_or_else(|| NoGpu("no GPU adapter".into()))?;
        let handle = &cx.devices[id];
        let (device, queue) = (handle.device.clone(), handle.queue.clone());
        let options =
            vello::RendererOptions { antialiasing_support: vello::AaSupport::area_only(), ..Default::default() };
        let renderer = vello::Renderer::new(&device, options).map_err(|e| NoGpu(e.to_string()))?;
        Ok(Self { device, queue, renderer })
    }

    /// Renders `scene` on white into a `width`×`height` image.
    pub fn render(&mut self, scene: &vello::Scene, width: u32, height: u32) -> Result<Image, NoGpu> {
        let gpu = |e: &dyn std::fmt::Display| NoGpu(e.to_string());
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xenify-write offscreen"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xenify-write readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::WHITE,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        self.renderer.render_to_texture(&self.device, &self.queue, scene, &view, &params).map_err(|e| gpu(&e))?;
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).map_err(|e| gpu(&e))?;
        rx.recv().map_err(|e| gpu(&e))?.map_err(|e| gpu(&e))?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in slice.get_mapped_range().chunks_exact(padded as usize) {
            pixels.extend_from_slice(&row[..(width * 4) as usize]);
        }
        buffer.unmap();
        Ok(Image { width, height, pixels })
    }
}
