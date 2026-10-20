//! Animated GIF / WebP / APNG images: kept as they are on import, and each
//! frame drawn at its time, looping or once, at the layer's speed.

use xenify_doc::{Color, Id, ImagePlayback, Layer, LayerKind, Project, Vec2};
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, Rgba, RgbaImage};

/// An 8×8 GIF: red for 0.1 s, then green for 0.2 s, then blue for 0.1 s.
pub fn traffic_gif() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for (c, ms) in [([255, 0, 0, 255], 100), ([0, 255, 0, 255], 200), ([0, 0, 255, 255], 100)] {
            let img = RgbaImage::from_pixel(8, 8, Rgba(c));
            enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(ms, 1))).unwrap();
        }
    }
    out
}

fn project(playback: ImagePlayback) -> (Project, Id) {
    let img = xenify_export::image::prepare(&traffic_gif(), 4096).unwrap();
    assert_eq!(img.mime, "image/gif", "kept as a GIF");
    let frames = img.frames.clone().expect("animated");
    assert_eq!(frames.len(), 3);
    assert!((frames[1] - 0.2).abs() < 1e-9);
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let (cmd, asset) = xenify_ops::assets::add_image(&mut p, "lights", img.mime, 8, 8, img.data, img.frames);
    xenify_ops::apply(&mut p, cmd.unwrap()).unwrap();
    let c = p.comp_mut(comp).unwrap();
    c.settings.width = 8;
    c.settings.height = 8;
    c.settings.background = Color::BLACK;
    let mut l = Layer::new(Id(100), "lights", LayerKind::Image { asset, playback });
    l.transform.position = Vec2::ZERO.into();
    c.layers = vec![l];
    p.reserve_id(Id(100));
    (p, comp)
}

/// The colour in the middle of the frame at `t`, as r/g/b initials.
fn colour_at(gpu: &mut xenify_render::Offscreen, p: &Project, comp: Id, t: f64) -> char {
    let scene = xenify_anim::evaluate_project(p, comp, t);
    let img = gpu.render(&scene, 8, 8).unwrap();
    let i = (4 * 8 + 4) * 4;
    let px = &img.pixels[i..i + 3];
    let max = px.iter().copied().max().unwrap();
    ['r', 'g', 'b'][px.iter().position(|v| *v == max).unwrap()]
}

#[test]
fn frames_play_at_their_times() {
    let Ok(mut gpu) = xenify_render::Offscreen::new() else {
        eprintln!("skipping: no GPU");
        return;
    };
    let (p, comp) = project(ImagePlayback::default());
    let seq: String = [0.05, 0.15, 0.29, 0.35, 0.45].iter().map(|t| colour_at(&mut gpu, &p, comp, *t)).collect();
    assert_eq!(seq, "rggbr", "loops after 0.4 s");

    let once = ImagePlayback { looped: false, ..Default::default() };
    let (p, comp) = project(once);
    assert_eq!(colour_at(&mut gpu, &p, comp, 3.0), 'b', "holds the last frame");

    let fast = ImagePlayback { speed: 2.0, offset: 0.1, ..Default::default() };
    let (p, comp) = project(fast);
    // 0.1 + 2 × 0.1 = 0.3 s in: blue.
    assert_eq!(colour_at(&mut gpu, &p, comp, 0.1), 'b');
    // The picture a still export embeds is the current frame.
    let scene = xenify_anim::evaluate_project(&p, comp, 0.1);
    let xenify_anim::SceneContent::Image(img) = &scene.layers[0].content else { panic!() };
    let (_, _, px) = xenify_export::image::scene_pixels(img).unwrap();
    assert_eq!(&px[0..3], &[0, 0, 255]);
}

#[test]
fn still_gifs_become_pngs() {
    let mut out = Vec::new();
    GifEncoder::new(&mut out).encode_frame(Frame::new(RgbaImage::from_pixel(2, 2, Rgba([9, 9, 9, 255])))).unwrap();
    let img = xenify_export::image::prepare(&out, 4096).unwrap();
    assert_eq!((img.mime, img.frames), ("image/png", None));
}
