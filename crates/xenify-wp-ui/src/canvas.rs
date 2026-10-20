//! A Vello-rendered picture shown in egui: renders into a wgpu texture that
//! egui draws. (Same approach as Xenify's `gpu_canvas`; shared once the
//! writer is merged into Xenify.)

use eframe::egui_wgpu::RenderState;
use vello::wgpu;

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Default)]
pub struct Canvas {
    inner: Option<Inner>,
}

struct Inner {
    renderer: vello::Renderer,
    texture: wgpu::Texture,
    texture_id: egui::TextureId,
    size: (u32, u32),
    scene: vello::Scene,
}

fn create_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("xenify-write page"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Canvas {
    /// The last rendered texture, if any.
    pub fn texture(&self) -> Option<egui::TextureId> {
        self.inner.as_ref().map(|g| g.texture_id)
    }

    /// Renders what `draw` puts in a Vello scene into a `size` texture
    /// (physical pixels) over `base`, and returns it for painting.
    pub fn render(
        &mut self,
        rs: &RenderState,
        size: (u32, u32),
        base: vello::peniko::Color,
        draw: impl FnOnce(&mut vello::Scene),
    ) -> Result<egui::TextureId, String> {
        let size = (size.0.clamp(1, 16384), size.1.clamp(1, 16384));
        if self.inner.is_none() {
            let options =
                vello::RendererOptions { antialiasing_support: vello::AaSupport::area_only(), ..Default::default() };
            let renderer = vello::Renderer::new(&rs.device, options).map_err(|e| e.to_string())?;
            let texture = create_target(&rs.device, size.0, size.1);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let texture_id = rs.renderer.write().register_native_texture(&rs.device, &view, wgpu::FilterMode::Linear);
            self.inner = Some(Inner { renderer, texture, texture_id, size, scene: vello::Scene::new() });
        }
        let g = self.inner.as_mut().expect("just made");
        if g.size != size {
            g.texture = create_target(&rs.device, size.0, size.1);
            let view = g.texture.create_view(&wgpu::TextureViewDescriptor::default());
            rs.renderer.write().update_egui_texture_from_wgpu_texture(
                &rs.device,
                &view,
                wgpu::FilterMode::Linear,
                g.texture_id,
            );
            g.size = size;
        }
        g.scene.reset();
        draw(&mut g.scene);
        let target = g.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let params = vello::RenderParams {
            base_color: base,
            width: size.0,
            height: size.1,
            antialiasing_method: vello::AaConfig::Area,
        };
        g.renderer.render_to_texture(&rs.device, &rs.queue, &g.scene, &target, &params).map_err(|e| e.to_string())?;
        Ok(g.texture_id)
    }
}
