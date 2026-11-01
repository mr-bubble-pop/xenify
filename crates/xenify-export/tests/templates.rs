//! Slide templates render: every layout in every theme draws something
//! (checked on the GPU when there is one), and a contact sheet can be written
//! for a look: `TEMPLATES_OUT=dir cargo test -p xenify-export --test templates`.

use xenify_doc::templates::{self, Content, Layout, THEMES};
use xenify_doc::{Id, Project};

fn slide(layout: Layout, theme: usize) -> (Project, Id) {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let built = templates::build(layout, &THEMES[theme], 1920.0, 1080.0, 10, &Content::default());
    let top = built.layers.iter().map(|l| l.id.0).max().unwrap();
    let c = p.comp_mut(comp).unwrap();
    c.settings.width = 1920;
    c.settings.height = 1080;
    c.settings.duration = built.duration;
    c.layers = built.layers;
    p.reserve_id(Id(top));
    (p, comp)
}

#[test]
fn every_layout_draws_in_every_theme() {
    let Ok(mut gpu) = xenify_render::Offscreen::new() else {
        eprintln!("skipping: no GPU");
        return;
    };
    let out = std::env::var_os("TEMPLATES_OUT").map(std::path::PathBuf::from);
    for (ti, theme) in THEMES.iter().enumerate() {
        for layout in Layout::ALL {
            let (p, comp) = slide(layout, ti);
            let t = p.comp(comp).unwrap().settings.duration - 0.01;
            let scene = xenify_anim::evaluate_project(&p, comp, t);
            let img = gpu.render(&scene, 480, 270).unwrap();
            // Not a flat picture: the text and shapes show.
            let first = &img.pixels[0..3];
            let differs = img.pixels.chunks(4).filter(|px| px[0..3] != *first).count();
            assert!(differs > 400, "{} / {}: nearly blank", theme.name, layout.label());
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let name = format!("{ti}-{:02}.png", Layout::ALL.iter().position(|l| *l == layout).unwrap());
                std::fs::write(dir.join(name), img.to_png()).unwrap();
            }
        }
    }
}
