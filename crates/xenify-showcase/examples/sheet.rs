//! Renders a contact sheet of the showcase: `cargo run -p xenify-showcase
//! --example sheet -- out_dir [slide numbers…]` writes one PNG strip per slide
//! (frames at even times and at each pause).

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("output folder"));
    let args: Vec<String> = args.collect();
    let only: Vec<usize> = args.iter().filter_map(|a| a.parse().ok()).collect();
    std::fs::create_dir_all(&out).unwrap();
    let (p, _) = xenify_showcase::build_with(&xenify_showcase::Options { clips: false, ..Default::default() });
    let mut gpu = xenify_render::Offscreen::new().expect("a GPU");
    // "slide@seconds": one big frame.
    for a in &args {
        if let Some((s, t)) = a.split_once('@') {
            let (s, t): (usize, f64) = (s.parse().unwrap(), t.parse().unwrap());
            let scene = xenify_anim::evaluate_project(&p, p.slides[s - 1].comp, t);
            let img = gpu.render(&scene, 960, 540).unwrap();
            std::fs::write(out.join(format!("frame-{s}-{t}.png")), img.to_png()).unwrap();
        }
    }
    if only.is_empty() && args.iter().any(|a| a.contains('@')) {
        return;
    }
    for (i, slide) in p.slides.iter().enumerate() {
        if !only.is_empty() && !only.contains(&(i + 1)) {
            continue;
        }
        let c = p.comp(slide.comp).unwrap();
        let d = c.settings.duration;
        let mut times: Vec<f64> = (0..8).map(|k| d * k as f64 / 7.0).collect();
        times.extend(c.settings.pauses.iter().map(|t| t - 0.02));
        times.sort_by(f64::total_cmp);
        let (w, h) = (384u32, 216u32);
        let mut strip = vec![0u8; (w as usize * times.len()) * h as usize * 4];
        for (k, t) in times.iter().enumerate() {
            let scene = xenify_anim::evaluate_project(&p, slide.comp, t.clamp(0.0, d - 0.001));
            let img = gpu.render(&scene, w, h).unwrap();
            for y in 0..h as usize {
                let row = &img.pixels[y * w as usize * 4..(y + 1) * w as usize * 4];
                let at = (y * w as usize * times.len() + k * w as usize) * 4;
                strip[at..at + row.len()].copy_from_slice(row);
            }
        }
        let img = xenify_render::RgbaImage { width: w * times.len() as u32, height: h, pixels: strip };
        std::fs::write(out.join(format!("slide{:02}.png", i + 1)), img.to_png()).unwrap();
        println!(
            "slide {} ({}): {:.1}s, times {:?}",
            i + 1,
            c.settings.name,
            d,
            times.iter().map(|t| (t * 10.0).round() / 10.0).collect::<Vec<_>>()
        );
    }
}
