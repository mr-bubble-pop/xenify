//! Slide 10: Export anywhere: the formats fly in as badges.

use crate::kit::*;
use xenify_audio::synth::Sfx;
use xenify_doc::{LayerKind, TransitionKind};

const FORMATS: [&str; 7] = ["MP4 4K", "WebM", "GIF", "Animated SVG", "Lottie", "PDF", "PNG"];

pub fn build(d: &mut Deck) {
    d.slide("Export anywhere", 4.0, &[3.6], tr(TransitionKind::Wipe, 0.8), "Every format from one file.");
    let mut title = text("Title", "Export anywhere", 130.0, INK, v(W / 2.0, 250.0));
    animate(&mut title, "Tracking In", 0.0);
    d.add(title);
    for (k, f) in FORMATS.iter().enumerate() {
        let row = if k < 4 { 0.0 } else { 1.0 };
        let (n, i) = if k < 4 { (4.0, k as f64) } else { (3.0, (k - 4) as f64) };
        let at = v(W / 2.0 + (i - (n - 1.0) / 2.0) * 400.0, 520.0 + row * 200.0);
        let t = 0.5 + k as f64 * 0.14;
        let from = v(((k * 37) % 7) as f64 * 160.0 - 480.0, if k % 2 == 0 { -700.0 } else { 700.0 });
        let mut badge = rect(&format!("{f} badge"), v(340.0, 130.0), 65.0, PANEL, at);
        if let LayerKind::Shape { shapes } = &mut badge.kind {
            let mut st = stroke(PINK, 5.0);
            st.gradient = Some(accent_gradient(340.0));
            shapes[0].stroke = Some(st);
        }
        badge.transform.position = keys(&[(t, at + from), (t + 0.7, at)], EXPO);
        badge.transform.rotation = keys(&[(t, if k % 2 == 0 { -25.0 } else { 25.0 }), (t + 0.9, 0.0)], SPRING);
        let badge = d.add(badge);
        let size = if f.len() > 8 { 36.0 } else { 46.0 };
        let mut label = text(&format!("{f} label"), f, size, INK, v(0.0, 14.0));
        label.meta.parent = Some(badge);
        d.add(label);
        d.sfx(Sfx::Swish, t, k as f64, 0.45);
    }
}
