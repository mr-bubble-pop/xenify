//! Hearing the project: while it plays (on the canvas or presenting) the
//! mix from the playhead goes to the speakers, restarted when the playhead
//! jumps; and waveforms for the timeline.

use crate::app::XenifyApp;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Mixed ahead each time sound starts (it restarts when this runs out).
const CHUNK: f64 = 30.0;
/// How far the playhead may drift from the sound before it restarts.
const DRIFT: f64 = 0.3;

#[derive(Default)]
pub struct Sound {
    player: Option<xenify_platform::media::Player>,
    /// Playhead time when the sound started, and when that was.
    started: Option<(f64, Instant)>,
    /// No way to play sound here (said once).
    unavailable: bool,
    /// Waveform outlines by clip.
    peaks: HashMap<u64, Arc<Vec<f32>>>,
    /// A sound being tried out (Properties › Sound › Preview).
    pub preview: Option<xenify_platform::media::Player>,
}

/// What's playing: a canvas, or a presentation (deck time).
#[derive(Clone, Copy, PartialEq)]
pub enum Source {
    Comp(xenify_doc::Id, f64),
    Deck(f64),
}

impl Sound {
    /// Keeps the speakers in step with the playhead at `time` (`playing` or not).
    pub fn sync(
        &mut self,
        project: &xenify_doc::Project,
        source: Source,
        time: f64,
        playing: bool,
        enabled: bool,
    ) -> Option<String> {
        if !playing || !enabled || !xenify_audio::has_sound(project) {
            self.player = None;
            self.started = None;
            return None;
        }
        if let Some((t0, at)) = self.started {
            let expected = t0 + at.elapsed().as_secs_f64();
            let near_end = at.elapsed().as_secs_f64() > CHUNK - 1.0;
            if (expected - time).abs() < DRIFT && !near_end && self.player.is_some() {
                return None;
            }
        }
        if self.unavailable {
            return None;
        }
        let (end, pcm) = match source {
            Source::Comp(comp, duration) => {
                let end = (time + CHUNK).min(duration);
                (end, xenify_audio::mix_comp(project, comp, time, end))
            }
            Source::Deck(duration) => {
                let end = (time + CHUNK).min(duration);
                (end, xenify_audio::mix_deck(project, time, end))
            }
        };
        if end <= time {
            return None;
        }
        self.player = xenify_platform::media::Player::start(pcm);
        self.started = Some((time, Instant::now()));
        if self.player.is_none() {
            self.unavailable = true;
            return Some("No sound device found, so playback is silent (exports still include the sound).".into());
        }
        None
    }

    /// Whether sound is playing now.
    pub fn is_playing(&self) -> bool {
        self.player.is_some()
    }

    /// The waveform outline of `media` (4000 points), decoded the first time.
    pub fn peaks(&mut self, media: &xenify_doc::MediaAsset) -> Option<Arc<Vec<f32>>> {
        if let Some(p) = self.peaks.get(&media.hash) {
            return Some(p.clone());
        }
        let pcm = xenify_audio::pcm(&xenify_anim::media::MediaRef::of(media))?;
        let p = Arc::new(xenify_audio::peaks(&pcm, 4000));
        self.peaks.insert(media.hash, p.clone());
        Some(p)
    }
}

/// Draws a clip's waveform across its bar in the timeline (`x_of` maps canvas time to screen x).
pub fn waveform(
    app: &mut XenifyApp,
    painter: &egui::Painter,
    layer: &xenify_doc::Layer,
    bar: egui::Rect,
    x_of: impl Fn(f64) -> f32,
) {
    let (asset, source_in, speed, looped) = match &layer.kind {
        xenify_doc::LayerKind::Audio(a) => (a.asset, a.source_in, a.speed, a.looped),
        xenify_doc::LayerKind::Video(v) => (v.asset, v.source_in, v.speed, v.looped),
        _ => return,
    };
    let Some(media) = app.project().media(asset).cloned() else { return };
    if media.is_video() && !media.has_audio {
        return;
    }
    let Some(peaks) = app.sound.peaks(&media) else { return };
    if peaks.is_empty() || media.duration <= 0.0 {
        return;
    }
    let colour = egui::Color32::from_rgba_unmultiplied(160, 220, 255, 150);
    let mid = bar.center().y;
    let half = bar.height() / 2.0 - 1.0;
    let (x0, x1) = (bar.left().max(painter.clip_rect().left()), bar.right().min(painter.clip_rect().right()));
    let t_at = |x: f32| {
        // Screen x back to canvas time (the bar is linear in time).
        let (a, b) = (x_of(layer.meta.in_point), x_of(layer.meta.in_point + 1.0));
        layer.meta.in_point + ((x - a) / (b - a).max(1e-6)) as f64
    };
    let mut x = x0;
    while x < x1 {
        let t = t_at(x);
        if let Some(src) =
            xenify_doc::media::source_time(source_in, speed, looped, media.duration, t - layer.meta.in_point)
        {
            let i = ((src / media.duration) * peaks.len() as f64) as usize;
            let v = peaks.get(i.min(peaks.len() - 1)).copied().unwrap_or(0.0);
            let h = (v.min(1.0) * half).max(0.5);
            painter.line_segment([egui::pos2(x, mid - h), egui::pos2(x, mid + h)], egui::Stroke::new(1.0, colour));
        }
        x += 2.0;
    }
}
