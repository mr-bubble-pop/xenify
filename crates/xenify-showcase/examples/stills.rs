//! Lists where presenting stops on a moving picture: for each pause (and a
//! slide's end), whether the scene still changes just after it, and when it
//! next stops changing.

fn main() {
    let (p, _) = xenify_showcase::build_with(&xenify_showcase::Options { clips: false, ..Default::default() });
    let gpu = std::cell::RefCell::new(xenify_render::Offscreen::new().expect("a GPU"));
    for (i, s) in p.slides.iter().enumerate() {
        let c = p.comp(s.comp).unwrap();
        let d = c.settings.duration;
        // Pixels that change by more than a little between two moments 1/10 s apart.
        let still = |t: f64| {
            let mut g = gpu.borrow_mut();
            // The last tenth of a second before stopping at `t`.
            let a = g.render(&xenify_anim::evaluate_project(&p, s.comp, (t - 0.1).max(0.0)), 320, 180).unwrap();
            let b = g.render(&xenify_anim::evaluate_project(&p, s.comp, t), 320, 180).unwrap();
            let moving = a
                .pixels
                .chunks(4)
                .zip(b.pixels.chunks(4))
                .filter(|(x, y)| x.iter().zip(y.iter()).any(|(u, v)| u.abs_diff(*v) > 24))
                .count();
            // (Under 0.5 %: small ambient motion, like connector flow dots.)
            moving < 288
        };
        let stops: Vec<f64> = c.settings.pauses.iter().copied().chain([d]).collect();
        for &t in &stops {
            if !still(t - 0.001) {
                // When does it settle?
                println!("slide {} ({}): stops at {t:.2} while moving (end {d:.2})", i + 1, c.settings.name);
            }
        }
    }
}
