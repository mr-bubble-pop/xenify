//! Sounds made by the program: sound effects (tick, whoosh, pop, chime,
//! riser, boom, …) and music beds (pad, arpeggio, bass and drums over a
//! chord progression). Everything is generated from a [`Recipe`] and a seed,
//! so there are no recordings and no rights to worry about, and a sound can
//! be made again with other settings.

use crate::RATE;
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// The loudest a generated sound gets (leaves headroom for mixing).
pub const PEAK: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sfx {
    Tick,
    Tock,
    Whoosh,
    Swish,
    Pop,
    Click,
    Chime,
    Riser,
    Boom,
    Crackle,
    Splash,
    Shatter,
}

impl Sfx {
    pub const ALL: [Sfx; 12] = [
        Sfx::Tick,
        Sfx::Tock,
        Sfx::Whoosh,
        Sfx::Swish,
        Sfx::Pop,
        Sfx::Click,
        Sfx::Chime,
        Sfx::Riser,
        Sfx::Boom,
        Sfx::Crackle,
        Sfx::Splash,
        Sfx::Shatter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sfx::Tick => "Tick",
            Sfx::Tock => "Tock",
            Sfx::Whoosh => "Whoosh",
            Sfx::Swish => "Swish",
            Sfx::Pop => "Pop",
            Sfx::Click => "Click",
            Sfx::Chime => "Chime",
            Sfx::Riser => "Riser",
            Sfx::Boom => "Boom",
            Sfx::Crackle => "Crackle",
            Sfx::Splash => "Splash",
            Sfx::Shatter => "Shatter",
        }
    }

    /// Its usual length in seconds.
    pub fn length(self) -> f64 {
        match self {
            Sfx::Tick | Sfx::Tock | Sfx::Click => 0.15,
            Sfx::Pop => 0.25,
            Sfx::Swish => 0.35,
            Sfx::Whoosh => 0.8,
            Sfx::Chime => 2.0,
            Sfx::Riser => 2.5,
            Sfx::Boom => 2.5,
            Sfx::Crackle => 1.5,
            Sfx::Splash => 1.2,
            Sfx::Shatter => 1.5,
        }
    }
}

/// A sound effect's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SfxRecipe {
    pub kind: Sfx,
    /// Semitones up (or down, negative).
    pub pitch: f64,
    /// Seconds.
    pub length: f64,
    /// 0 dull … 1 bright.
    pub brightness: f64,
    /// How long it rings: 0.25 short … 2 long.
    pub decay: f64,
    /// 0 exact … 1 very different each seed.
    pub variation: f64,
    pub seed: u64,
    /// Overall level, 0…1.
    pub volume: f64,
}

impl Default for SfxRecipe {
    fn default() -> Self {
        Self::of(Sfx::Pop)
    }
}

impl SfxRecipe {
    pub fn of(kind: Sfx) -> Self {
        Self {
            kind,
            pitch: 0.0,
            length: kind.length(),
            brightness: 0.5,
            decay: 1.0,
            variation: 0.3,
            seed: 1,
            volume: 0.8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    /// I–V–vi–IV, bright.
    Uplifting,
    /// I–vi–IV–V, gentle.
    Calm,
    /// vi–IV–I–V, big.
    Epic,
    /// i–VI–III–VII, minor.
    Dark,
    /// ii–V–I–vi, jazzy sevenths.
    Smooth,
}

impl Mood {
    pub const ALL: [Mood; 5] = [Mood::Uplifting, Mood::Calm, Mood::Epic, Mood::Dark, Mood::Smooth];

    pub fn label(self) -> &'static str {
        match self {
            Mood::Uplifting => "Uplifting",
            Mood::Calm => "Calm",
            Mood::Epic => "Epic",
            Mood::Dark => "Dark",
            Mood::Smooth => "Smooth",
        }
    }

    /// Chords as (semitones above the key, minor?, seventh?).
    fn chords(self) -> [(i32, bool, bool); 4] {
        match self {
            Mood::Uplifting => [(0, false, false), (7, false, false), (9, true, false), (5, false, false)],
            Mood::Calm => [(0, false, false), (9, true, false), (5, false, false), (7, false, false)],
            Mood::Epic => [(9, true, false), (5, false, false), (0, false, false), (7, false, false)],
            Mood::Dark => [(0, true, false), (8, false, false), (3, false, false), (10, false, false)],
            Mood::Smooth => [(2, true, true), (7, false, true), (0, false, true), (9, true, true)],
        }
    }
}

/// A music bed's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicRecipe {
    /// Beats per minute.
    pub tempo: f64,
    /// Semitones above C (0 = C, 9 = A).
    pub key: i32,
    pub mood: Mood,
    /// Bars of 4 beats (each chord lasts one bar).
    pub bars: u32,
    pub pad: bool,
    pub arp: bool,
    pub bass: bool,
    pub kick: bool,
    pub hat: bool,
    /// Made to loop without a seam.
    pub looped: bool,
    pub seed: u64,
    /// Overall level, 0…1.
    pub volume: f64,
}

impl Default for MusicRecipe {
    fn default() -> Self {
        Self {
            tempo: 110.0,
            key: 0,
            mood: Mood::Uplifting,
            bars: 8,
            pad: true,
            arp: true,
            bass: true,
            kick: true,
            hat: true,
            looped: true,
            seed: 1,
            volume: 0.7,
        }
    }
}

impl MusicRecipe {
    /// Seconds it lasts.
    pub fn length(&self) -> f64 {
        self.bars.max(1) as f64 * 4.0 * 60.0 / self.tempo.clamp(30.0, 300.0)
    }
}

/// How a generated sound was made (stored with it, to make it again).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Recipe {
    Sfx(SfxRecipe),
    Music(MusicRecipe),
}

impl Recipe {
    pub fn name(&self) -> String {
        match self {
            Recipe::Sfx(s) => s.kind.label().to_string(),
            Recipe::Music(m) => format!("Music bed ({})", m.mood.label()),
        }
    }

    /// The sound: 48 kHz stereo, never louder than [`PEAK`].
    pub fn render(&self) -> Vec<f32> {
        match self {
            Recipe::Sfx(s) => sfx(s),
            Recipe::Music(m) => music(m),
        }
    }
}

/// The sound as a clip for the project, `id`: an embedded WAV that keeps
/// its recipe (so it can be made again with other settings).
pub fn media_asset(recipe: &Recipe, id: xenify_doc::Id) -> xenify_doc::MediaAsset {
    let pcm = recipe.render();
    let wav = crate::wav(&pcm);
    xenify_doc::MediaAsset {
        id,
        name: recipe.name(),
        kind: xenify_doc::MediaKind::Audio,
        mime: "audio/wav".into(),
        hash: xenify_doc::hash_bytes(&wav),
        source: xenify_doc::MediaSource::Embedded { data: wav.into() },
        duration: pcm.len() as f64 / 2.0 / RATE as f64,
        width: 0,
        height: 0,
        fps: 0.0,
        has_audio: true,
        recipe: serde_json::to_value(recipe).ok(),
    }
}

/// The recipe a generated clip was made with, if it was.
pub fn recipe_of(media: &xenify_doc::MediaAsset) -> Option<Recipe> {
    serde_json::from_value(media.recipe.clone()?).ok()
}

// ---------------------------------------------------------------------------
// Building blocks

/// Deterministic noise.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    /// −1…1.
    fn noise(&mut self) -> f64 {
        self.next() * 2.0 - 1.0
    }
}

/// A resonant low/band/high-pass filter (Chamberlin state variable).
#[derive(Default)]
struct Svf {
    low: f64,
    band: f64,
}

impl Svf {
    /// Filters `x` at `cutoff` Hz; returns (low, band, high).
    fn tick(&mut self, x: f64, cutoff: f64, q: f64) -> (f64, f64, f64) {
        let f = (2.0 * (std::f64::consts::PI * cutoff.clamp(20.0, 16_000.0) / RATE as f64).sin()).min(1.2);
        let damp = 1.0 / q.max(0.5);
        self.low += f * self.band;
        let high = x - self.low - damp * self.band;
        self.band += f * high;
        (self.low, self.band, high)
    }
}

/// A small room: parallel combs into series allpasses (Schroeder).
struct Room {
    combs: Vec<(Vec<f64>, usize, f64)>,
    alls: Vec<(Vec<f64>, usize)>,
}

impl Room {
    fn new(size: f64) -> Self {
        let ms = |m: f64| ((m * size * RATE as f64 / 1000.0) as usize).max(1);
        Self {
            combs: [29.7, 37.1, 41.1, 43.7].iter().map(|m| (vec![0.0; ms(*m)], 0, 0.78)).collect(),
            alls: [5.0, 1.7].iter().map(|m| (vec![0.0; ms(*m)], 0)).collect(),
        }
    }
    fn tick(&mut self, x: f64) -> f64 {
        let mut y = 0.0;
        for (buf, i, fb) in &mut self.combs {
            let out = buf[*i];
            buf[*i] = x + out * *fb;
            *i = (*i + 1) % buf.len();
            y += out;
        }
        y /= self.combs.len() as f64;
        for (buf, i) in &mut self.alls {
            let d = buf[*i];
            let out = -y * 0.7 + d;
            buf[*i] = y + d * 0.7;
            *i = (*i + 1) % buf.len();
            y = out;
        }
        y
    }
}

fn semis(s: f64) -> f64 {
    2f64.powf(s / 12.0)
}

/// Soft saturation.
fn soft(x: f64) -> f64 {
    x.tanh()
}

/// Scales `pcm` so its loudest sample is `peak` (quiet sounds are raised
/// no more than 4×) and fades the last 5 ms to avoid a click.
fn finish(mut pcm: Vec<f32>, peak: f32, fade_tail: bool) -> Vec<f32> {
    let loudest = pcm.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if loudest > 1e-9 {
        let k = (peak / loudest).min(4.0 * peak);
        for s in &mut pcm {
            *s *= k;
        }
    }
    if fade_tail {
        let n = (0.005 * RATE as f64) as usize;
        let frames = pcm.len() / 2;
        for i in 0..n.min(frames) {
            let g = i as f32 / n as f32;
            let j = frames - 1 - i;
            pcm[j * 2] *= g;
            pcm[j * 2 + 1] *= g;
        }
    }
    pcm
}

// ---------------------------------------------------------------------------
// Sound effects

fn sfx(r: &SfxRecipe) -> Vec<f32> {
    let mut rng = Rng::new(r.seed ^ (r.kind as u64 * 7919));
    let vary = |rng: &mut Rng, amount: f64| 1.0 + (rng.noise() * r.variation.clamp(0.0, 1.0) * amount);
    let pitch = semis(r.pitch) * vary(&mut rng, 0.08);
    let bright = r.brightness.clamp(0.0, 1.0);
    let decay = r.decay.clamp(0.1, 4.0);
    let len = r.length.clamp(0.02, 20.0);
    let n = (len * RATE as f64) as usize;
    let mut out = vec![0.0f32; n * 2];
    let mut filt = Svf::default();
    let mut filt2 = Svf::default();
    let mut room = Room::new(1.0);
    // Crackle / shatter: when each grain starts, and its pitch.
    let grains: Vec<(usize, f64, f64)> = match r.kind {
        Sfx::Crackle | Sfx::Shatter | Sfx::Splash => {
            let count = match r.kind {
                Sfx::Crackle => (len * 60.0) as usize,
                Sfx::Shatter => (len * 90.0) as usize,
                _ => (len * 40.0) as usize,
            };
            (0..count)
                .map(|_| {
                    // Denser at the start.
                    let at = rng.next().powf(if r.kind == Sfx::Crackle { 1.0 } else { 2.2 });
                    ((at * n as f64) as usize, rng.next(), rng.noise())
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let mut phase = 0.0f64;
    let mut phase2 = 0.0f64;
    for i in 0..n {
        let t = i as f64 / RATE as f64;
        let u = t / len;
        let (mut l, mut rr);
        match r.kind {
            Sfx::Tick | Sfx::Tock | Sfx::Click => {
                let f = match r.kind {
                    Sfx::Tick => 2600.0,
                    Sfx::Tock => 1300.0,
                    _ => 4200.0,
                } * pitch;
                let env = (-t / (0.018 * decay)).exp();
                phase += TAU * f / RATE as f64;
                let click = if t < 0.002 { rng.noise() * (1.0 - t / 0.002) } else { 0.0 };
                let body = if r.kind == Sfx::Click { 0.2 } else { 1.0 };
                let (_, band, _) = filt.tick(click, f * (0.8 + bright), 3.0);
                l = (phase.sin() * body * env) + band * 1.5;
                rr = l;
            }
            Sfx::Whoosh | Sfx::Swish => {
                // Noise through a band sweeping up and back, rising and falling, panning across.
                let shape = (u * std::f64::consts::PI).sin().powf(1.5);
                let centre = (300.0 + 3200.0 * (u * std::f64::consts::PI).sin() * (0.5 + bright)) * pitch;
                let (_, band, _) = filt.tick(rng.noise(), centre, 2.2);
                let v = band * shape * 2.5;
                let pan = u;
                l = v * (1.0 - pan * 0.7);
                rr = v * (0.3 + pan * 0.7);
            }
            Sfx::Pop => {
                let f = (200.0 + 800.0 * (-t / 0.02).exp()) * pitch;
                phase += TAU * f / RATE as f64;
                let env = (-t / (0.06 * decay)).exp() * (t / 0.002).min(1.0);
                l = phase.sin() * env + rng.noise() * 0.05 * (-t / 0.004).exp();
                rr = l;
            }
            Sfx::Chime => {
                // A bell: inharmonic partials, the high ones dying first.
                let f = 880.0 * pitch;
                let mut v = 0.0;
                for (k, (ratio, amp)) in
                    [(1.0, 1.0), (2.76, 0.5), (5.4, 0.25 * (0.5 + bright)), (8.93, 0.1 * bright)].iter().enumerate()
                {
                    let d = (1.6 * decay) / (1.0 + k as f64 * 0.8);
                    v += (TAU * f * ratio * t).sin() * amp * (-t / d).exp();
                }
                let env = (t / 0.003).min(1.0);
                let wet = room.tick(v * env);
                l = v * env * 0.8 + wet * 0.5;
                rr = v * env * 0.8 - wet * 0.5;
            }
            Sfx::Riser => {
                // A rising saw and noise, the filter opening, getting louder.
                let f = 110.0 * pitch * semis(24.0 * u);
                phase += f / RATE as f64;
                phase2 += f * 1.007 / RATE as f64;
                let saw = (phase.fract() * 2.0 - 1.0) + (phase2.fract() * 2.0 - 1.0);
                let (low, _, _) =
                    filt.tick(saw * 0.4 + rng.noise() * 0.6, 300.0 + 9000.0 * u.powi(2) * (0.4 + bright), 1.5);
                let env = u.powf(1.8) * (1.0 - ((u - 0.97) / 0.03).clamp(0.0, 1.0));
                l = low * env;
                rr = low * env;
            }
            Sfx::Boom => {
                // A deep thump that drops in pitch, with a rumble of low noise.
                let f = (40.0 + 60.0 * (-t / 0.08).exp()) * pitch;
                phase += TAU * f / RATE as f64;
                let env = (-t / (0.45 * decay)).exp() * (t / 0.004).min(1.0);
                let (low, _, _) = filt.tick(rng.noise(), 180.0 + 600.0 * bright, 0.8);
                let (low2, _, _) = filt2.tick(low, 120.0, 0.7);
                let hit = rng.noise() * (-t / 0.01).exp() * 0.5;
                let v = soft((phase.sin() * 1.4 + low2 * 2.5) * env + hit);
                l = v + room.tick(v) * 0.2;
                rr = v;
            }
            Sfx::Crackle | Sfx::Splash | Sfx::Shatter => {
                l = 0.0;
                rr = 0.0;
                if r.kind == Sfx::Splash {
                    // A wash of water noise under the droplets.
                    let (low, _, _) = filt.tick(rng.noise(), 1200.0 + 2500.0 * bright, 1.0);
                    let wash = low * (-t / (0.35 * decay)).exp() * (t / 0.01).min(1.0);
                    l += wash;
                    rr += wash;
                }
            }
        }
        out[i * 2] = l as f32;
        out[i * 2 + 1] = rr as f32;
    }
    // Grains: short clicks (crackle), droplets (splash), glass tinkles (shatter).
    for &(at, x, pan) in &grains {
        let (f, dur) = match r.kind {
            Sfx::Crackle => (2000.0 + 3000.0 * x, 0.004),
            Sfx::Splash => (400.0 + 900.0 * x, 0.05),
            _ => (2500.0 + 5000.0 * x * (0.5 + bright), 0.15 * decay),
        };
        let f = f * pitch;
        let m = (dur * RATE as f64) as usize;
        let gain = match r.kind {
            Sfx::Crackle => 0.6,
            Sfx::Splash => 0.35,
            _ => 0.3 * (1.0 - at as f64 / n as f64).max(0.1),
        };
        let mut g = Rng::new(at as u64 ^ r.seed);
        for j in 0..m {
            let k = at + j;
            if k >= n {
                break;
            }
            let tt = j as f64 / RATE as f64;
            let v = match r.kind {
                Sfx::Crackle => g.noise() * (-tt / 0.0012).exp(),
                // A droplet: a sine gliding up.
                Sfx::Splash => (TAU * f * tt * (1.0 + tt * 12.0)).sin() * (-tt / 0.015).exp(),
                // Glass: two bright inharmonic tones.
                _ => ((TAU * f * tt).sin() + (TAU * f * 2.41 * tt).sin() * 0.5) * (-tt / (dur * 0.3)).exp(),
            } * gain;
            out[k * 2] += (v * (1.0 - pan.max(0.0) * 0.6)) as f32;
            out[k * 2 + 1] += (v * (1.0 + pan.min(0.0) * 0.6)) as f32;
        }
    }
    finish(out, (PEAK as f64 * r.volume.clamp(0.0, 1.0)) as f32, true)
}

// ---------------------------------------------------------------------------
// Music beds

/// Frequency of MIDI note `n`.
fn note_hz(n: f64) -> f64 {
    440.0 * 2f64.powf((n - 69.0) / 12.0)
}

fn music(m: &MusicRecipe) -> Vec<f32> {
    let beat = 60.0 / m.tempo.clamp(30.0, 300.0);
    let bars = m.bars.clamp(1, 64) as usize;
    let len = m.length();
    // A tail rings on after the last bar; for a loop it wraps round to the start.
    let tail = if m.looped { 2.0 } else { 1.5 };
    let n = (len * RATE as f64) as usize;
    let total = ((len + tail) * RATE as f64) as usize;
    let mut out = vec![0.0f32; total * 2];
    let mut rng = Rng::new(m.seed);
    let root = 48 + m.key.rem_euclid(12); // C3 and up
    let chords = m.mood.chords();
    let chord_notes = |bar: usize| -> Vec<f64> {
        let (deg, minor, seventh) = chords[bar % 4];
        let base = root + deg;
        let mut v = vec![base as f64, (base + if minor { 3 } else { 4 }) as f64, (base + 7) as f64];
        if seventh {
            v.push((base + if minor { 10 } else { 11 }) as f64);
        }
        v
    };
    let mut add = |at: f64, samples: &[f64], pan: f64| {
        let start = (at * RATE as f64) as usize;
        for (j, s) in samples.iter().enumerate() {
            let k = start + j;
            if k >= total {
                break;
            }
            out[k * 2] += (s * (1.0 - pan.max(0.0))) as f32;
            out[k * 2 + 1] += (s * (1.0 + pan.min(0.0))) as f32;
        }
    };
    for bar in 0..bars {
        let t0 = bar as f64 * 4.0 * beat;
        let notes = chord_notes(bar);
        if m.pad {
            // Detuned saws through a soft low-pass, slow in and out.
            let dur = 4.0 * beat + 0.4;
            let count = (dur * RATE as f64) as usize;
            for (v, &note) in notes.iter().enumerate() {
                let f = note_hz(note + 12.0);
                let mut filt = Svf::default();
                let (mut p1, mut p2) = (rng.next(), rng.next());
                let s: Vec<f64> = (0..count)
                    .map(|j| {
                        let t = j as f64 / RATE as f64;
                        p1 += f * 0.997 / RATE as f64;
                        p2 += f * 1.003 / RATE as f64;
                        let saw = (p1.fract() * 2.0 - 1.0) + (p2.fract() * 2.0 - 1.0);
                        let (low, _, _) = filt.tick(saw, 900.0 + 500.0 * (t / dur), 0.8);
                        let env = (t / 0.5).min(1.0) * ((dur - t) / 0.4).clamp(0.0, 1.0);
                        low * env * 0.07
                    })
                    .collect();
                add(t0, &s, if v % 2 == 0 { -0.3 } else { 0.3 });
            }
        }
        if m.arp {
            // Plucked chord notes in sixteenths, up and down.
            let pattern: Vec<f64> = notes.iter().chain(notes.iter().rev().skip(1)).copied().collect();
            for step in 0..16 {
                let note = pattern[step % pattern.len()] + 24.0;
                let f = note_hz(note);
                let dur = beat / 4.0 * 1.8;
                let count = (dur * RATE as f64) as usize;
                let s: Vec<f64> = (0..count)
                    .map(|j| {
                        let t = j as f64 / RATE as f64;
                        let tri = 1.0 - 4.0 * ((f * t).fract() - 0.5).abs();
                        tri * (-t / 0.12).exp() * (t / 0.002).min(1.0) * 0.09
                    })
                    .collect();
                add(t0 + step as f64 * beat / 4.0, &s, ((step % 4) as f64 - 1.5) * 0.25);
            }
        }
        if m.bass {
            // Root notes in eighths, a low sine with a little bite.
            let f = note_hz(notes[0] - 12.0);
            for step in 0..8 {
                let dur = beat / 2.0 * 0.9;
                let count = (dur * RATE as f64) as usize;
                let s: Vec<f64> = (0..count)
                    .map(|j| {
                        let t = j as f64 / RATE as f64;
                        soft((TAU * f * t).sin() * 1.6)
                            * (t / 0.005).min(1.0)
                            * ((dur - t) / 0.02).clamp(0.0, 1.0)
                            * 0.18
                    })
                    .collect();
                add(t0 + step as f64 * beat / 2.0, &s, 0.0);
            }
        }
        for b in 0..4 {
            let at = t0 + b as f64 * beat;
            if m.kick {
                let count = (0.3 * RATE as f64) as usize;
                let mut ph = 0.0;
                let s: Vec<f64> = (0..count)
                    .map(|j| {
                        let t = j as f64 / RATE as f64;
                        ph += TAU * (45.0 + 110.0 * (-t / 0.03).exp()) / RATE as f64;
                        ph.sin() * (-t / 0.12).exp() * 0.35
                    })
                    .collect();
                add(at, &s, 0.0);
            }
            if m.hat {
                let count = (0.05 * RATE as f64) as usize;
                let mut filt = Svf::default();
                let mut g = Rng::new(m.seed ^ (bar * 4 + b) as u64);
                let s: Vec<f64> = (0..count)
                    .map(|j| {
                        let t = j as f64 / RATE as f64;
                        let (_, _, high) = filt.tick(g.noise(), 7000.0, 1.0);
                        high * (-t / 0.015).exp() * 0.06
                    })
                    .collect();
                add(at + beat / 2.0, &s, 0.2);
            }
        }
    }
    // A loop: fold the tail back onto the start, so the end runs into the beginning.
    if m.looped {
        let (head, rest) = out.split_at_mut(n * 2);
        for (i, v) in rest.iter().enumerate().take(head.len()) {
            head[i] += *v;
        }
        out.truncate(n * 2);
    }
    finish(out, (PEAK as f64 * m.volume.clamp(0.0, 1.0)) as f32, !m.looped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn every_effect_sounds_and_never_clips() {
        for kind in Sfx::ALL {
            let pcm = Recipe::Sfx(SfxRecipe::of(kind)).render();
            assert_eq!(pcm.len(), (kind.length() * RATE as f64) as usize * 2, "{kind:?}");
            let p = peak(&pcm);
            assert!(p > 0.1, "{kind:?} is heard: {p}");
            assert!(p <= PEAK + 1e-6, "{kind:?} doesn't clip: {p}");
            assert!(pcm.iter().all(|s| s.is_finite()));
        }
    }

    #[test]
    fn the_same_recipe_makes_the_same_sound_and_seeds_differ() {
        let r = SfxRecipe { variation: 1.0, ..SfxRecipe::of(Sfx::Shatter) };
        assert_eq!(Recipe::Sfx(r.clone()).render(), Recipe::Sfx(r.clone()).render());
        assert_ne!(Recipe::Sfx(r.clone()).render(), Recipe::Sfx(SfxRecipe { seed: 2, ..r }).render());
    }

    #[test]
    fn music_beds_last_whole_bars_and_loop() {
        for mood in Mood::ALL {
            let m = MusicRecipe { mood, bars: 2, tempo: 120.0, ..Default::default() };
            let pcm = Recipe::Music(m.clone()).render();
            assert_eq!(pcm.len(), (4.0 * RATE as f64) as usize * 2, "2 bars at 120 bpm = 4 s");
            let p = peak(&pcm);
            assert!(p > 0.1 && p <= PEAK + 1e-6, "{mood:?}: {p}");
        }
        // Without drums or pad it's quieter but still there.
        let m = MusicRecipe { pad: false, kick: false, hat: false, bass: false, bars: 1, ..Default::default() };
        assert!(peak(&Recipe::Music(m).render()) > 0.1);
    }

    #[test]
    fn recipes_round_trip() {
        for r in [Recipe::Sfx(SfxRecipe::of(Sfx::Boom)), Recipe::Music(MusicRecipe::default())] {
            let json = serde_json::to_value(&r).unwrap();
            assert_eq!(serde_json::from_value::<Recipe>(json).unwrap(), r);
        }
    }
}

/// `XENIFY_DUMP=<dir>`: writes every effect and a music bed as WAV files (to listen to).
#[cfg(test)]
#[test]
fn dump_sounds() {
    let Ok(dir) = std::env::var("XENIFY_DUMP") else { return };
    for kind in Sfx::ALL {
        let pcm = Recipe::Sfx(SfxRecipe::of(kind)).render();
        std::fs::write(format!("{dir}/{}.wav", kind.label()), crate::wav(&pcm)).unwrap();
    }
    for mood in Mood::ALL {
        let pcm = Recipe::Music(MusicRecipe { mood, bars: 4, ..Default::default() }).render();
        std::fs::write(format!("{dir}/music-{}.wav", mood.label()), crate::wav(&pcm)).unwrap();
    }
}
