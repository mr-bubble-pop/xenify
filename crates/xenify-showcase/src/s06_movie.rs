//! Slide 6: Movie editing: three clips the program made itself (the cold open, an
//! abstract loop, a 3D turntable) on a timeline; one scales up and crops into
//! a split screen; trimming and speed; audio waveforms; a 4K badge.

use crate::kit::*;
use xenify_doc::{Easing, Id, LayerKind, Mask, MediaAsset, MediaKind, MediaSource, SolidShape, TransitionKind};
use kurbo::Rect;

fn window(l: &mut xenify_doc::Layer, t0: f64, t1: f64) {
    live(l, t0, Some(t1 + 0.35));
    if matches!(l.opacity, xenify_doc::Property::Static(_)) {
        l.opacity = keys(&[(t0, 0.0), (t0 + 0.25, 1.0), (t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    }
}

fn caption(d: &mut Deck, t: &str, t0: f64, t1: f64) {
    let mut c = text("Caption", t, 40.0, MUTED, v(W / 2.0, 1030.0));
    animate(&mut c, "Fade Up by Word", t0 + 0.2);
    c.opacity = keys(&[(t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    live(&mut c, 0.0, Some(t1 + 0.35));
    d.add(c);
}

/// An abstract loop: gradient rings turning.
fn abstract_loop(d: &mut Deck) -> Id {
    let comp = d.symbol("Abstract loop", W, H, 4.0, xenify_doc::Color::rgb(0.05, 0.02, 0.12));
    for k in 0..6 {
        let size = 900.0 - k as f64 * 130.0;
        let mut ring = ellipse("Ring", v(size, size * 0.6), PINK, v(W / 2.0, H / 2.0));
        if let LayerKind::Shape { shapes } = &mut ring.kind {
            shapes[0].fill = None;
            let mut st = stroke(if k % 2 == 0 { PINK } else { CYAN }, 18.0);
            st.gradient = Some(accent_gradient(size));
            shapes[0].stroke = Some(st);
        }
        let dir = if k % 2 == 0 { 1.0 } else { -1.0 };
        ring.transform.rotation = keys(&[(0.0, k as f64 * 15.0), (4.0, k as f64 * 15.0 + dir * 180.0)], Easing::Linear);
        d.add_to(comp, ring);
    }
    comp
}

/// A 3D torus turning.
fn turntable(d: &mut Deck) -> Id {
    let comp = d.symbol("3D turntable", W, H, 4.0, xenify_doc::Color::rgb(0.03, 0.05, 0.1));
    let mut s = xenify_doc::Solid::new(SolidShape::Torus { thickness: 0.32 }, 620.0, 200.0, PINK);
    s.rot_x = 60.0.into();
    s.rot_y = keys(&[(0.0, 0.0), (4.0, 360.0)], Easing::Linear);
    let l = xenify_doc::Layer::new(Id(0), "Torus", LayerKind::Solid(Box::new(s))).with_position(v(W / 2.0, H / 2.0));
    d.add_to(comp, l);
    comp
}

/// Renders `comp` to a small MP4 and returns it as an embedded clip.
fn render_clip(p: &xenify_doc::Project, comp: Id, name: &str, id: Id) -> Option<MediaAsset> {
    let dir = std::env::temp_dir().join(format!("showcase-clips-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("{}.mp4", id.0));
    let mut gpu = xenify_render::Offscreen::new().ok()?;
    let opts = xenify_export::ExportOptions { size: Some((640, 360)), with_sound: false, ..Default::default() };
    xenify_export::export_to_file(p, comp, &file, &opts, &mut gpu, &mut |_, _| true).ok()?;
    let info = xenify_platform::media::probe(&file).ok()?;
    let data = std::fs::read(&file).ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    Some(MediaAsset {
        id,
        name: name.into(),
        kind: MediaKind::Video,
        mime: "video/mp4".into(),
        hash: xenify_doc::hash_bytes(&data),
        source: MediaSource::Embedded { data: data.into() },
        duration: info.duration,
        width: info.width,
        height: info.height,
        fps: info.fps,
        has_audio: false,
        recipe: None,
    })
}

/// A clip on the slide: the rendered video, or the symbol itself when
/// clips aren't rendered. Placed at `at`, `width` pixels wide, from `t`.
fn clip(
    d: &mut Deck,
    video: Option<&MediaAsset>,
    comp: Id,
    name: &str,
    at: xenify_doc::Vec2,
    width: f64,
    t: f64,
) -> Id {
    match video {
        Some(m) => {
            let layer = Id(d.p.next_id + 1);
            let media = MediaAsset { id: d.next_id(), ..m.clone() };
            let cmd = xenify_ops::media::add_clip(&d.p, d.comp, media, layer, t).expect("clip");
            d.exec(cmd);
            let k = width / m.width.max(1) as f64;
            d.edit(layer, |l| {
                l.transform.anchor = v(m.width as f64 / 2.0, m.height as f64 / 2.0).into();
                l.transform.position = at.into();
                l.transform.scale = v(k, k).into();
                l.meta.out_point = None;
                if let LayerKind::Video(vl) = &mut l.kind {
                    vl.looped = true;
                }
            });
            layer
        }
        None => d.place(comp, name, at, width / W, t),
    }
}

pub fn build(d: &mut Deck, render: bool) {
    let open = d.p.slides[0].comp;
    let loop_c = abstract_loop(d);
    let turn = turntable(d);
    let sources = [(open, "Cold open"), (loop_c, "Abstract loop"), (turn, "3D turntable")];
    let videos: Vec<Option<MediaAsset>> =
        sources.iter().map(|(c, n)| if render { render_clip(&d.p, *c, n, Id(0)) } else { None }).collect();

    // Clips play, so it plays straight through (a pause would freeze them).
    let pauses: [f64; 0] = [];
    d.slide(
        "Movie editing",
        11.0,
        &pauses,
        tr(TransitionKind::Liquid, 1.2),
        "Clips, crops, trims, speed, sound. All made in here.",
    );
    let mut title = text_left("Title", "Movie editing", 120.0, INK, v(120.0, 200.0));
    animate(&mut title, "Fade Up by Word", 0.0);
    d.add(title);

    // The timeline, all along.
    let mut panel = rect("Timeline", v(1600.0, 230.0), 20.0, PANEL, v(W / 2.0, 840.0));
    fade_in(&mut panel, 0.1, 0.4);
    d.add(panel);
    let colors = [PINK, VIOLET, CYAN];
    let mut bars = Vec::new();
    for (k, (_, name)) in sources.iter().enumerate() {
        let (x, w) = (360.0 + k as f64 * 470.0, 440.0);
        let mut bar = rect(&format!("{name} bar"), v(w, 70.0), 12.0, colors[k], v(x + w / 2.0, 800.0));
        slide_in(&mut bar, 0.2 + k as f64 * 0.15, v(0.0, 80.0), 0.6);
        bars.push(d.add(bar));
        let mut label = text_left("Clip name", name, 28.0, BG, v(x + 20.0, 810.0));
        fade_in(&mut label, 0.5 + k as f64 * 0.15, 0.3);
        d.add(label);
    }
    let mut head = rect("Playhead", v(4.0, 230.0), 2.0, INK, v(330.0, 840.0));
    head.transform.position = keys(&[(0.3, v(330.0, 840.0)), (10.0, v(1740.0, 840.0))], SMOOTH);
    d.add(head);

    // A. Three clips; one scales up and crops into a split screen.
    let (t0, t1) = (0.3, 3.3);
    let mut ids = Vec::new();
    for (k, (comp, name)) in sources.iter().enumerate() {
        let at = v(520.0 + k as f64 * 440.0, 470.0);
        let id = clip(d, videos[k].as_ref(), *comp, name, at, 400.0, 0.0);
        d.edit(id, |l| window(l, t0 + k as f64 * 0.15, t1));
        ids.push(id);
    }
    // The loop and the turntable grow into two halves of the frame, cropped by masks.
    for (k, id) in [ids[1], ids[2]].into_iter().enumerate() {
        let from = d.p.layer(id).unwrap().transform.position.value_at(0.0);
        let s0 = d.p.layer(id).unwrap().transform.scale.value_at(0.0);
        let s1 = s0 * (840.0 / 400.0);
        let to = v(if k == 0 { 540.0 } else { 1380.0 }, 470.0);
        let (lw, lh) = match &d.p.layer(id).unwrap().kind {
            LayerKind::Video(_) => (
                videos[k + 1].as_ref().map_or(W, |m| m.width as f64),
                videos[k + 1].as_ref().map_or(H, |m| m.height as f64),
            ),
            _ => (W, H),
        };
        let crop = |f: f64| xenify_doc::rect_path(Rect::new(lw * f, 0.0, lw * (1.0 - f), lh));
        d.edit(id, |l| {
            l.transform.position = keys(&[(1.6, from), (2.4, to)], EXPO);
            l.transform.scale = keys(&[(1.6, s0), (2.4, s1)], EXPO);
            let mut m = Mask::new("Crop", crop(0.0));
            m.path = keys(&[(1.9, crop(0.0)), (2.6, crop(0.24))], SMOOTH);
            l.meta.masks.push(m);
        });
    }
    d.edit(ids[0], |l| l.opacity = keys(&[(0.3, 0.0), (0.55, 1.0), (1.5, 1.0), (1.8, 0.0)], SMOOTH));
    caption(d, "Clips on a timeline: scale, crop and split the screen", t0, t1);

    // B. Trimming and speed.
    let (t0, t1) = (3.5, 6.1);
    let trim_to = 300.0;
    d.edit(bars[0], |l| {
        l.transform.scale = keys(&[(t0 + 0.3, v(1.0, 1.0)), (t0 + 1.0, v(trim_to / 440.0, 1.0))], EXPO);
        l.transform.anchor = v(-220.0, 0.0).into();
        l.transform.position = v(360.0, 800.0).into();
    });
    let mut trim = text("Badge", "Trim", 36.0, INK, v(360.0 + trim_to / 2.0, 720.0));
    window(&mut trim, t0 + 0.4, t1);
    d.add(trim);
    let mut fast = rect("Speed badge", v(120.0, 60.0), 30.0, GOLD, v(1300.0, 720.0));
    pop(&mut fast, t0 + 1.0);
    window(&mut fast, t0 + 1.0, t1);
    d.add(fast);
    let mut fast_t = text("Speed", "2×", 38.0, BG, v(1300.0, 732.0));
    window(&mut fast_t, t0 + 1.1, t1);
    d.add(fast_t);
    let mut big = text("Speed demo", "Trim, split, slow down, speed up", 72.0, INK, v(W / 2.0, 470.0));
    animate(&mut big, "Slide by Line", 0.2);
    window(&mut big, t0, t1);
    d.add(big);
    caption(d, "Trimming and speed, with the sound kept in step", t0, t1);

    // C. The music's real waveform draws across.
    let (t0, t1) = (6.3, 8.7);
    let music = xenify_audio::synth::Recipe::Music(xenify_audio::synth::MusicRecipe {
        tempo: 112.0,
        key: 2,
        mood: xenify_audio::synth::Mood::Uplifting,
        bars: 8,
        seed: 3,
        volume: 0.6,
        ..Default::default()
    });
    let peaks = xenify_audio::peaks(&music.render(), 160);
    let mut wave = kurbo::BezPath::new();
    for (i, p) in peaks.iter().enumerate() {
        let x = -700.0 + i as f64 * (1400.0 / 160.0);
        let h = (*p as f64 * 300.0).max(3.0);
        wave.extend(xenify_doc::rect_path(Rect::new(x, -h / 2.0, x + 5.0, h / 2.0)));
    }
    let mut w = path("Waveform", wave, Some(CYAN), None, v(W / 2.0, 480.0));
    if let LayerKind::Shape { shapes } = &mut w.kind {
        shapes[0].fill = Some(gradient_fill(1400.0));
    }
    let reveal = |f: f64| xenify_doc::rect_path(Rect::new(-720.0, -200.0, -720.0 + 1440.0 * f, 200.0));
    let mut m = Mask::new("Reveal", reveal(0.0));
    m.path = keys(&[(t0 + 0.1, reveal(0.0)), (t0 + 1.6, reveal(1.0))], SMOOTH);
    w.meta.masks.push(m);
    window(&mut w, t0, t1);
    d.add(w);
    caption(d, "Audio with waveforms: music and sound effects the app makes itself", t0, t1);

    // D. The 4K badge.
    let (t0, t1) = (8.9, 10.9);
    let mut badge = rect("4K badge", v(520.0, 220.0), 40.0, PINK, v(W / 2.0, 470.0));
    if let LayerKind::Shape { shapes } = &mut badge.kind {
        shapes[0].fill = Some(gradient_fill(520.0));
    }
    pop(&mut badge, t0);
    window(&mut badge, t0, t1);
    d.add(badge);
    let mut k4 = text("4K", "4K EXPORT", 84.0, INK, v(W / 2.0, 500.0));
    animate(&mut k4, "Tracking In", 0.2);
    window(&mut k4, t0 + 0.1, t1);
    d.add(k4);
    let sparkle = d.particles("Sparkle", Some(v(W / 2.0, 470.0)), t0 + 0.2, None, |e| {
        // A short burst of sparkle that's gone by the end.
        e.stop = Some(t0 + 0.7);
    });
    d.edit(sparkle, |l| live(l, t0, Some(t1 + 0.35)));
    d.sfx(xenify_audio::synth::Sfx::Chime, t0, 0.0, 0.7);
    caption(d, "Up to 4K at 60 fps, with sound", t0, t1);
}
