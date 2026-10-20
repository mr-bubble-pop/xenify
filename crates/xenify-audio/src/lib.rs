//! Sound for Xenify Vector: mixing a composition's or a presentation's audio (clip
//! timing, speed, volume keyframes, fades, symbols, slides, the deck's
//! soundtrack), peaks for waveforms, WAV files, and synthesized sounds.
//!
//! Sound is 48 kHz stereo, as interleaved `f32` samples (left, right, …).
//! Decoding files is left to a [`PcmSource`] (ffmpeg on the desktop), so
//! this crate stays pure Rust.

pub mod synth;

use xenify_anim::media::MediaRef;
use xenify_doc::media::source_time;
use xenify_doc::{AudioLayer, Composition, LayerKind, Project, Property};
use std::sync::{Arc, Mutex};

/// Samples per second.
pub const RATE: u32 = 48_000;
/// Nested symbols deeper than this are silent.
const MAX_DEPTH: usize = 8;

/// Decodes a clip's sound to 48 kHz stereo.
pub trait PcmSource: Send {
    fn pcm(&mut self, media: &MediaRef) -> Option<Arc<Vec<f32>>>;
}

static SOURCE: Mutex<Option<Box<dyn PcmSource>>> = Mutex::new(None);

/// Sets how clips are decoded (the app does this at start).
pub fn set_pcm_source(source: Box<dyn PcmSource>) {
    *SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
}

pub fn has_pcm_source() -> bool {
    SOURCE.lock().map(|s| s.is_some()).unwrap_or(false)
}

/// Clip `media`'s sound, if a decoder is installed and it can.
pub fn pcm(media: &MediaRef) -> Option<Arc<Vec<f32>>> {
    // Sound made by the program is stored with its recipe: no decoder needed.
    SOURCE.lock().ok()?.as_mut()?.pcm(media)
}

/// How one clip plays: timing and loudness.
#[derive(Clone, Debug)]
pub struct Play<'a> {
    /// When (in the mix's own time) the clip's layer starts and stops.
    pub in_point: f64,
    pub out_point: Option<f64>,
    pub source_in: f64,
    pub speed: f64,
    pub looped: bool,
    pub volume: &'a Property<f64>,
    pub fade_in: f64,
    pub fade_out: f64,
}

/// Adds `pcm` (clip `duration` seconds long) into `out`, whose first sample
/// is at time `t0` (the mix's own time).
pub fn add_clip(out: &mut [f32], t0: f64, pcm: &[f32], duration: f64, play: &Play) {
    let frames = out.len() / 2;
    let src_frames = pcm.len() / 2;
    if src_frames == 0 {
        return;
    }
    let end = play.out_point.unwrap_or(f64::INFINITY);
    let span = end - play.in_point;
    // Volume keyframes are looked up every 64 samples (about 1 ms).
    let mut gain = 0.0;
    for i in 0..frames {
        let t = t0 + i as f64 / RATE as f64;
        if t < play.in_point || t >= end {
            continue;
        }
        let local = t - play.in_point;
        let Some(src) = source_time(play.source_in, play.speed, play.looped, duration, local) else { continue };
        if i % 64 == 0 || gain == 0.0 {
            let mut g = play.volume.value_at(t).max(0.0);
            if play.fade_in > 0.0 {
                g *= (local / play.fade_in).min(1.0);
            }
            if play.fade_out > 0.0 && span.is_finite() {
                g *= ((span - local) / play.fade_out).clamp(0.0, 1.0);
            }
            gain = g;
        }
        if gain <= 0.0 {
            continue;
        }
        // Between two samples of the clip (speed changes the pitch, like tape).
        let x = src * RATE as f64;
        let k = x.floor() as usize;
        if k >= src_frames {
            continue;
        }
        let f = (x - k as f64) as f32;
        let next = (k + 1).min(src_frames - 1);
        for ch in 0..2 {
            let a = pcm[k * 2 + ch];
            let b = pcm[next * 2 + ch];
            out[i * 2 + ch] += (a + (b - a) * f) * gain as f32;
        }
    }
}

/// The sound `comp` makes from `start` to `end` seconds.
pub fn mix_comp(project: &Project, comp: xenify_doc::Id, start: f64, end: f64) -> Vec<f32> {
    let frames = ((end - start).max(0.0) * RATE as f64).round() as usize;
    let mut out = vec![0.0f32; frames * 2];
    if let Ok(c) = project.comp(comp) {
        add_comp(&mut out, project, c, start, 0);
    }
    out
}

/// Adds `comp`'s sound into `out`, whose first sample is at comp time `t0`.
fn add_comp(out: &mut [f32], project: &Project, comp: &Composition, t0: f64, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for layer in comp.layers.iter().filter(|l| l.meta.visible) {
        let (asset, play) = match &layer.kind {
            LayerKind::Audio(a) if !a.muted => (a.asset, play_of(layer, a)),
            LayerKind::Video(v) if !v.muted => (
                v.asset,
                Play {
                    in_point: layer.meta.in_point,
                    out_point: layer.meta.out_point,
                    source_in: v.source_in,
                    speed: v.speed,
                    looped: v.looped,
                    volume: &v.volume,
                    fade_in: 0.0,
                    fade_out: 0.0,
                },
            ),
            LayerKind::Precomp { comp: child, start, looped } => {
                if let Ok(c) = project.comp(*child) {
                    // Only while the symbol's layer is on: mix into a scratch buffer, then keep that part.
                    let mut inner = vec![0.0f32; out.len()];
                    if *looped && c.settings.duration > 0.0 {
                        // Each pass, mixed where it plays (the buffer's own passes only).
                        let d = c.settings.duration;
                        let span = out.len() as f64 / 2.0 / RATE as f64;
                        let first = ((t0 - start) / d).floor().max(0.0);
                        let mut k = first;
                        while start + k * d < t0 + span && k - first < 10_000.0 {
                            let mut pass = vec![0.0f32; out.len()];
                            add_comp(&mut pass, project, c, t0 - start - k * d, depth + 1);
                            for (i, s) in pass.iter().enumerate() {
                                let local = t0 + (i / 2) as f64 / RATE as f64 - start - k * d;
                                if (0.0..d).contains(&local) {
                                    inner[i] += s;
                                }
                            }
                            k += 1.0;
                        }
                    } else {
                        add_comp(&mut inner, project, c, t0 - start, depth + 1);
                    }
                    for (i, pair) in inner.chunks(2).enumerate() {
                        let t = t0 + i as f64 / RATE as f64;
                        if layer.is_active_at(t) {
                            out[i * 2] += pair[0];
                            out[i * 2 + 1] += pair[1];
                        }
                    }
                }
                continue;
            }
            _ => continue,
        };
        let Some(media) = project.media(asset) else { continue };
        if !media.has_audio && media.is_video() {
            continue;
        }
        if let Some(pcm) = pcm(&MediaRef::of(media)) {
            add_clip(out, t0, &pcm, media.duration, &play);
        }
    }
}

fn play_of<'a>(layer: &xenify_doc::Layer, a: &'a AudioLayer) -> Play<'a> {
    Play {
        in_point: layer.meta.in_point,
        out_point: layer.meta.out_point,
        source_in: a.source_in,
        speed: a.speed,
        looped: a.looped,
        volume: &a.volume,
        fade_in: a.fade_in,
        fade_out: a.fade_out,
    }
}

/// The sound of the whole presentation (each slide while it plays, and the
/// soundtrack) from `start` to `end` seconds of deck time.
pub fn mix_deck(project: &Project, start: f64, end: f64) -> Vec<f32> {
    let frames = ((end - start).max(0.0) * RATE as f64).round() as usize;
    let mut out = vec![0.0f32; frames * 2];
    let deck = xenify_anim::deck::Deck::new(project);
    for i in 0..deck.len() {
        let (s0, s1) = (deck.slide_start(i), deck.slide_end(i));
        if s1 <= start || s0 >= end {
            continue;
        }
        let Some(comp) = deck.slide_comp(i).and_then(|c| project.comp(c).ok()) else { continue };
        // Only the part of the mix while this slide plays.
        let a = (((s0.max(start) - start) * RATE as f64).round() as usize).min(frames);
        let b = (((s1.min(end) - start) * RATE as f64).round() as usize).min(frames);
        if b > a {
            let t0 = start + a as f64 / RATE as f64 - s0;
            add_comp(&mut out[a * 2..b * 2], project, comp, t0, 0);
        }
    }
    add_soundtrack(&mut out, project, start);
    out
}

/// Adds the presentation's soundtrack into `out`, whose first sample is at deck time `t0`.
pub fn add_soundtrack(out: &mut [f32], project: &Project, t0: f64) {
    for clip in &project.soundtrack.clips {
        if clip.audio.muted {
            continue;
        }
        let Some(media) = project.media(clip.audio.asset) else { continue };
        let Some(pcm) = pcm(&MediaRef::of(media)) else { continue };
        let play = Play {
            in_point: clip.start,
            out_point: None,
            source_in: clip.audio.source_in,
            speed: clip.audio.speed,
            looped: clip.audio.looped,
            volume: &clip.audio.volume,
            fade_in: clip.audio.fade_in,
            fade_out: 0.0,
        };
        add_clip(out, t0, &pcm, media.duration, &play);
    }
}

/// Whether anything in the project makes a sound (so exports add a track).
pub fn has_sound(project: &Project) -> bool {
    !project.soundtrack.is_empty()
        || project.compositions.iter().flat_map(|c| &c.layers).any(|l| match &l.kind {
            LayerKind::Audio(a) => !a.muted,
            LayerKind::Video(v) => !v.muted && project.media(v.asset).is_some_and(|m| m.has_audio),
            _ => false,
        })
}

/// The loudest sample (either channel) in each of `bins` equal parts of
/// `pcm`: a waveform's outline.
pub fn peaks(pcm: &[f32], bins: usize) -> Vec<f32> {
    let frames = pcm.len() / 2;
    if bins == 0 || frames == 0 {
        return vec![0.0; bins];
    }
    (0..bins)
        .map(|b| {
            let (s, e) = (b * frames / bins, ((b + 1) * frames / bins).max(b * frames / bins + 1).min(frames));
            pcm[s * 2..e * 2].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        })
        .collect()
}

/// `pcm` as a 16-bit stereo WAV file (loud parts are clipped at full scale).
pub fn wav(pcm: &[f32]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// A WAV file's samples as 48 kHz stereo `f32` (16-bit PCM or 32-bit float,
/// mono or stereo at 48 kHz; others: `None`).
pub fn read_wav(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..(at + 8 + len).min(bytes.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            format = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            if rate != RATE || !(1..=2).contains(&channels) {
                return None;
            }
            let samples: Vec<f32> = match (format, bits) {
                (1, 16) => body.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect(),
                (3, 32) => body.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
                _ => return None,
            };
            return Some(if channels == 2 { samples } else { samples.iter().flat_map(|s| [*s, *s]).collect() });
        }
        at += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Id, Keyframe, Layer, MediaAsset, MediaKind, MediaSource};

    /// A constant-level clip: `seconds` long at `level` in both channels.
    fn tone(seconds: f64, level: f32) -> Vec<f32> {
        vec![level; (seconds * RATE as f64) as usize * 2]
    }

    struct Fixed(Arc<Vec<f32>>);
    impl PcmSource for Fixed {
        fn pcm(&mut self, _: &MediaRef) -> Option<Arc<Vec<f32>>> {
            Some(self.0.clone())
        }
    }

    #[test]
    fn clips_start_at_their_in_point_and_follow_volume_keys() {
        let pcm = tone(2.0, 0.5);
        let volume: Property<f64> = Property::from_keyframes(vec![
            Keyframe { time: 1.0, value: 1.0, easing: xenify_doc::Easing::Linear },
            Keyframe { time: 2.0, value: 0.0, easing: xenify_doc::Easing::Linear },
        ])
        .unwrap();
        let play = Play {
            in_point: 1.0,
            out_point: None,
            source_in: 0.0,
            speed: 1.0,
            looped: false,
            volume: &volume,
            fade_in: 0.0,
            fade_out: 0.0,
        };
        let mut out = vec![0.0f32; 3 * RATE as usize * 2];
        add_clip(&mut out, 0.0, &pcm, 2.0, &play);
        let at = |t: f64| out[(t * RATE as f64) as usize * 2];
        assert_eq!(at(0.5), 0.0, "silent before the in point");
        assert!((at(1.0) - 0.5).abs() < 1e-3, "exactly from 1 s: {}", at(1.0));
        assert!((at(1.5) - 0.25).abs() < 0.01, "half volume half way: {}", at(1.5));
        assert!(at(2.99).abs() < 0.01);
        // Sample-accurate start.
        let first = out.iter().position(|v| *v != 0.0).unwrap() / 2;
        assert_eq!(first, RATE as usize);
    }

    #[test]
    fn a_deck_plays_each_slide_after_the_one_before() {
        set_pcm_source(Box::new(Fixed(Arc::new(tone(10.0, 0.25)))));
        let mut p = Project::new();
        let a = p.compositions[0].id;
        p.compositions[0].settings.duration = 1.0;
        let media = MediaAsset {
            id: Id(50),
            name: "tone".into(),
            kind: MediaKind::Audio,
            mime: "audio/wav".into(),
            source: MediaSource::Linked { path: "tone.wav".into() },
            duration: 10.0,
            width: 0,
            height: 0,
            fps: 0.0,
            has_audio: true,
            hash: 1,
            recipe: None,
        };
        p.media.push(media);
        let b = p.alloc_id();
        let mut second = p.compositions[0].clone();
        second.id = b;
        second.layers = vec![Layer::new(Id(60), "sound", LayerKind::Audio(Box::new(AudioLayer::new(Id(50)))))];
        p.compositions.push(second);
        p.slides = vec![xenify_doc::Slide::new(a), xenify_doc::Slide::new(b)];
        p.reserve_id(Id(60));
        let mix = mix_deck(&p, 0.0, 2.0);
        assert_eq!(mix[(0.5 * RATE as f64) as usize * 2], 0.0, "slide 1 is silent");
        assert!((mix[(1.5 * RATE as f64) as usize * 2] - 0.25).abs() < 1e-3, "slide 2 plays its sound");
        assert!(has_sound(&p));
    }

    #[test]
    fn wav_files_round_trip_and_peaks_find_the_loud_parts() {
        let mut pcm = tone(0.1, 0.0);
        pcm[1000] = 0.8;
        let back = read_wav(&wav(&pcm)).unwrap();
        assert_eq!(back.len(), pcm.len());
        assert!((back[1000] - 0.8).abs() < 1e-3);
        let p = peaks(&pcm, 10);
        assert!(p.iter().any(|v| (*v - 0.8).abs() < 1e-6));
        assert_eq!(p.iter().filter(|v| **v > 0.0).count(), 1);
    }
}
