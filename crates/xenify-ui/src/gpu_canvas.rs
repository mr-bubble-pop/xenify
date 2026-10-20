//! A Vello-rendered picture shown in egui (used by presenting and slide thumbnails).

use eframe::egui_wgpu::RenderState;
use xenify_render::vello::{self, wgpu};

#[derive(Default)]
pub struct GpuCanvas {
    inner: Option<Inner>,
}

struct Inner {
    renderer: vello::Renderer,
    texture: wgpu::Texture,
    texture_id: egui::TextureId,
    size: (u32, u32),
    scene: vello::Scene,
}

impl GpuCanvas {
    /// Renders what `draw` puts in a Vello scene into a `size` texture and
    /// returns it for painting.
    pub fn render(
        &mut self,
        rs: &RenderState,
        size: (u32, u32),
        draw: impl FnOnce(&mut vello::Scene),
    ) -> Result<egui::TextureId, String> {
        if self.inner.is_none() {
            let renderer =
                vello::Renderer::new(&rs.device, xenify_render::renderer_options()).map_err(|e| e.to_string())?;
            let texture = xenify_render::create_target(&rs.device, size.0, size.1);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let texture_id = rs.renderer.write().register_native_texture(&rs.device, &view, wgpu::FilterMode::Linear);
            self.inner = Some(Inner { renderer, texture, texture_id, size, scene: vello::Scene::new() });
        }
        let g = self.inner.as_mut().expect("just made");
        if g.size != size {
            g.texture = xenify_render::create_target(&rs.device, size.0, size.1);
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
        g.renderer
            .render_to_texture(
                &rs.device,
                &rs.queue,
                &g.scene,
                &target,
                &xenify_render::render_params(size.0, size.1, xenify_doc::Color::BLACK),
            )
            .map_err(|e| e.to_string())?;
        Ok(g.texture_id)
    }
}
