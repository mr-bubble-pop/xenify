//! The showcase is ordinary editing: replaying its commands on a new project
//! gives the same file; the product name is a parameter only; every slide
//! draws something at its pauses.

use xenify_doc::{LayerKind, Project};

fn build(name: &str) -> (Project, Vec<xenify_ops::Command>) {
    xenify_showcase::build_with(&xenify_showcase::Options { name: name.into(), clips: false })
}

#[test]
fn replaying_the_commands_rebuilds_it() {
    let (p, log) = build("Spark");
    let mut q = Project::new();
    for cmd in log {
        xenify_ops::apply(&mut q, cmd).expect("each step applies");
    }
    q.theme = p.theme.clone();
    assert_eq!(q, p);
    assert_eq!(p.slides.len(), 11);
    // It survives saving and loading.
    assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
}

#[test]
fn a_slide_never_needs_two_clicks_at_its_end() {
    let (p, _) = build("Spark");
    for (i, s) in p.slides.iter().enumerate() {
        let c = p.comp(s.comp).unwrap();
        if let Some(last) = c.settings.pauses.last() {
            assert!(c.settings.duration - last > 1.3, "slide {}: a pause at {last} just before its end", i + 1);
        }
    }
}

#[test]
fn the_name_is_a_parameter() {
    let (p, _) = build("Zephyr");
    let texts: Vec<(&str, &str)> = p
        .compositions
        .iter()
        .flat_map(|c| &c.layers)
        .filter_map(|l| match &l.kind {
            LayerKind::Text(t) => Some((l.meta.name.as_str(), t.text.as_str())),
            _ => None,
        })
        .collect();
    let named: Vec<_> = texts.iter().filter(|(_, t)| t.contains("Zephyr")).collect();
    assert_eq!(named.len(), 2, "the title and the finale");
    assert!(named.iter().all(|(n, _)| *n == "Product name"));
    let json = p.to_json().to_lowercase();
    assert!(!json.contains("xenify"), "the program's own name never appears");
}

#[test]
fn every_slide_draws_where_presenting_stops() {
    let Ok(mut gpu) = xenify_render::Offscreen::new() else {
        eprintln!("skipping: no GPU");
        return;
    };
    let (p, _) = build("Spark");
    for (i, s) in p.slides.iter().enumerate() {
        let c = p.comp(s.comp).unwrap();
        // Where presenting stops: each pause, and the slide's end.
        let stops: Vec<f64> = c.settings.pauses.iter().copied().chain([c.settings.duration]).collect();
        for t in &stops {
            let img = gpu.render(&xenify_anim::evaluate_project(&p, s.comp, *t - 0.02), 192, 108).unwrap();
            let bg = &img.pixels[0..3];
            let lit =
                img.pixels.chunks(4).filter(|px| px[0..3].iter().zip(bg).any(|(a, b)| a.abs_diff(*b) > 30)).count();
            assert!(lit > 60, "slide {} ({}) at {t}: nearly empty", i + 1, c.settings.name);
        }
    }
}

#[test]
fn presenting_never_stops_on_something_mid_motion() {
    // A stop freezes the picture: explosions, water and clips must have
    // settled (small ambient motion, like connector flow dots, is fine).
    let Ok(mut gpu) = xenify_render::Offscreen::new() else {
        eprintln!("skipping: no GPU");
        return;
    };
    let (p, _) = build("Spark");
    for (i, s) in p.slides.iter().enumerate() {
        let c = p.comp(s.comp).unwrap();
        let d = c.settings.duration;
        for t in c.settings.pauses.iter().copied().chain([d]) {
            let t = t - 0.001;
            let a = gpu.render(&xenify_anim::evaluate_project(&p, s.comp, (t - 0.1).max(0.0)), 320, 180).unwrap();
            let b = gpu.render(&xenify_anim::evaluate_project(&p, s.comp, t), 320, 180).unwrap();
            let moving = a
                .pixels
                .chunks(4)
                .zip(b.pixels.chunks(4))
                .filter(|(x, y)| x.iter().zip(y.iter()).any(|(u, v)| u.abs_diff(*v) > 24))
                .count();
            assert!(
                moving < 288,
                "slide {} ({}): stops at {t:.2} with {moving} pixels still moving",
                i + 1,
                c.settings.name
            );
        }
    }
}
