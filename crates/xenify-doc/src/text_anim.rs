//! Text animators: letters, words or lines of a text layer animate in one
//! after another from a "from" state (moved, scaled, turned, faded, blurred,
//! spread out) to where they belong.
//!
//! Each unit starts `stagger` seconds after the one before (in `order`),
//! takes `duration` seconds and follows `easing`. Several animators on one
//! layer add up.

use crate::Easing;
use kurbo::{Affine, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextUnit {
    #[default]
    Letter,
    Word,
    Line,
}

impl TextUnit {
    pub const ALL: [TextUnit; 3] = [TextUnit::Letter, TextUnit::Word, TextUnit::Line];
    pub fn label(self) -> &'static str {
        match self {
            TextUnit::Letter => "Letter",
            TextUnit::Word => "Word",
            TextUnit::Line => "Line",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOrder {
    #[default]
    Forward,
    Reverse,
    /// From the middle out.
    Center,
    /// Shuffled (by `seed`).
    Random,
}

impl TextOrder {
    pub const ALL: [TextOrder; 4] = [TextOrder::Forward, TextOrder::Reverse, TextOrder::Center, TextOrder::Random];
    pub fn label(self) -> &'static str {
        match self {
            TextOrder::Forward => "First to last",
            TextOrder::Reverse => "Last to first",
            TextOrder::Center => "From the middle",
            TextOrder::Random => "Random",
        }
    }
}

/// Where a unit starts from (it animates to rest: no offset, 100%, 0°,
/// opaque, sharp, normal spacing).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlyphState {
    /// Pixels, in the layer's space.
    pub offset: Vec2,
    /// 1 = full size.
    pub scale: f64,
    /// Degrees.
    pub rotation: f64,
    /// 0..1.
    pub opacity: f64,
    /// Blur radius, pixels.
    pub blur: f64,
    /// Extra space between letters, pixels (spreads the text apart).
    pub tracking: f64,
}

impl Default for GlyphState {
    /// At rest.
    fn default() -> Self {
        Self { offset: Vec2::ZERO, scale: 1.0, rotation: 0.0, opacity: 1.0, blur: 0.0, tracking: 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextAnimator {
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub unit: TextUnit,
    #[serde(default)]
    pub order: TextOrder,
    /// When the first unit starts (seconds, layer time).
    #[serde(default)]
    pub start: f64,
    /// Seconds between one unit starting and the next.
    pub stagger: f64,
    /// Seconds each unit takes.
    pub duration: f64,
    #[serde(default)]
    pub easing: Easing,
    pub from: GlyphState,
    /// Changes the Random order.
    #[serde(default)]
    pub seed: u64,
    /// Animate out instead of in: units leave for the "from" state.
    #[serde(default)]
    pub out: bool,
}

fn yes() -> bool {
    true
}

/// How one glyph looks at a moment, combined over all animators.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphLook {
    /// Glyph space (layer) → where it's drawn, about its centre.
    pub transform: Affine,
    pub opacity: f64,
    pub blur: f64,
}

impl GlyphLook {
    pub const REST: GlyphLook = GlyphLook { transform: Affine::IDENTITY, opacity: 1.0, blur: 0.0 };
}

/// Where a glyph sits in its text: which letter, word and line it belongs
/// to (letters and words count only visible characters; spaces have none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphPlace {
    pub letter: Option<usize>,
    pub word: Option<usize>,
    pub line: usize,
}

/// Units in `text`: `(letters, words)` with the unit of each char (by byte
/// offset), so a renderer can find a glyph's place from its cluster start.
pub fn places(text: &str) -> Vec<(usize, Option<usize>, Option<usize>)> {
    let mut out = Vec::new();
    let (mut letter, mut word) = (0usize, 0usize);
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if in_word {
                word += 1;
                in_word = false;
            }
            out.push((i, None, None));
        } else {
            in_word = true;
            out.push((i, Some(letter), Some(word)));
            letter += 1;
        }
    }
    out
}

/// The place of the character starting at byte `offset` of `text`, on `line`.
pub fn place_at(places: &[(usize, Option<usize>, Option<usize>)], offset: usize, line: usize) -> GlyphPlace {
    let i = places.partition_point(|p| p.0 <= offset).saturating_sub(1);
    let (_, letter, word) = places.get(i).copied().unwrap_or((0, None, None));
    GlyphPlace { letter, word, line }
}

impl TextAnimator {
    /// A preset by name (see [`PRESETS`]).
    pub fn preset(name: &str) -> Option<Self> {
        let base = |unit, stagger, duration, from: GlyphState| TextAnimator {
            name: name.to_string(),
            enabled: true,
            unit,
            order: TextOrder::Forward,
            start: 0.0,
            stagger,
            duration,
            easing: Easing::EASE_OUT,
            from,
            seed: 0,
            out: false,
        };
        let rest = GlyphState::default();
        Some(match name {
            "Typewriter" => base(TextUnit::Letter, 0.06, 0.0, GlyphState { opacity: 0.0, ..rest }),
            "Fade Up by Word" => {
                base(TextUnit::Word, 0.12, 0.6, GlyphState { offset: Vec2::new(0.0, 30.0), opacity: 0.0, ..rest })
            }
            "Pop by Letter" => TextAnimator {
                easing: Easing::Spring { stiffness: 100.0, damping: 10.0 },
                ..base(TextUnit::Letter, 0.04, 0.5, GlyphState { scale: 0.0, opacity: 0.0, ..rest })
            },
            "Slide by Line" => {
                base(TextUnit::Line, 0.2, 0.7, GlyphState { offset: Vec2::new(-80.0, 0.0), opacity: 0.0, ..rest })
            }
            "Tracking In" => {
                base(TextUnit::Letter, 0.0, 1.2, GlyphState { tracking: 40.0, opacity: 0.0, blur: 6.0, ..rest })
            }
            "Blur In" => base(TextUnit::Letter, 0.03, 0.5, GlyphState { blur: 12.0, opacity: 0.0, ..rest }),
            "Drop In" => TextAnimator {
                easing: Easing::Spring { stiffness: 100.0, damping: 10.0 },
                ..base(TextUnit::Letter, 0.05, 0.6, GlyphState { offset: Vec2::new(0.0, -120.0), opacity: 0.0, ..rest })
            },
            "Spin In" => {
                base(TextUnit::Letter, 0.05, 0.6, GlyphState { rotation: -180.0, scale: 0.3, opacity: 0.0, ..rest })
            }
            _ => return None,
        })
    }

    /// When the last unit (of `count`) has finished.
    pub fn end(&self, count: usize) -> f64 {
        self.start + self.stagger.max(0.0) * count.saturating_sub(1) as f64 + self.duration.max(0.0)
    }

    /// Progress (0 = the "from" state, 1 = at rest) of unit `index` of `count` at `time`.
    pub fn progress(&self, index: usize, count: usize, time: f64) -> f64 {
        let n = count.max(1);
        let k = match self.order {
            TextOrder::Forward => index as f64,
            TextOrder::Reverse => (n - 1 - index.min(n - 1)) as f64,
            TextOrder::Center => (index as f64 - (n - 1) as f64 / 2.0).abs(),
            TextOrder::Random => shuffled(index, n, self.seed) as f64,
        };
        let begin = self.start + k * self.stagger.max(0.0);
        let t = if self.duration <= 1e-9 {
            if time >= begin { 1.0 } else { 0.0 }
        } else {
            ((time - begin) / self.duration).clamp(0.0, 1.0)
        };
        let p = self.easing.apply(t);
        if self.out { 1.0 - p } else { p }
    }

    /// The unit index of a glyph for this animator.
    pub fn unit_of(&self, place: GlyphPlace) -> Option<usize> {
        match self.unit {
            TextUnit::Letter => place.letter,
            TextUnit::Word => place.word,
            TextUnit::Line => Some(place.line),
        }
    }
}

/// The preset names, for menus.
pub const PRESETS: &[&str] = &[
    "Typewriter",
    "Fade Up by Word",
    "Pop by Letter",
    "Slide by Line",
    "Tracking In",
    "Blur In",
    "Drop In",
    "Spin In",
];

/// Counts of letters, words and lines, for staggering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitCounts {
    pub letters: usize,
    pub words: usize,
    pub lines: usize,
}

/// How a glyph looks under `animators` at `time`. `centre` is the glyph's
/// middle (layer space) and `spread` its distance from the text's middle
/// in letters (for tracking).
pub fn glyph_look(
    animators: &[TextAnimator],
    place: GlyphPlace,
    counts: UnitCounts,
    centre: kurbo::Point,
    spread: f64,
    time: f64,
) -> GlyphLook {
    let mut look = GlyphLook::REST;
    for a in animators.iter().filter(|a| a.enabled) {
        let Some(unit) = a.unit_of(place) else { continue };
        let count = match a.unit {
            TextUnit::Letter => counts.letters,
            TextUnit::Word => counts.words,
            TextUnit::Line => counts.lines,
        };
        let p = a.progress(unit, count, time);
        let q = 1.0 - p;
        let f = a.from;
        let offset = f.offset * q + Vec2::new(f.tracking * q * spread, 0.0);
        let scale = 1.0 + (f.scale - 1.0) * q;
        let rotation = f.rotation * q;
        let m = Affine::translate(offset)
            * Affine::translate(centre.to_vec2())
            * Affine::rotate(rotation.to_radians())
            * Affine::scale(scale)
            * Affine::translate(-centre.to_vec2());
        look.transform = m * look.transform;
        look.opacity *= (f.opacity + (1.0 - f.opacity) * p).clamp(0.0, 1.0);
        look.blur += f.blur.max(0.0) * q;
    }
    look
}

/// Position of `index` in a seeded shuffle of `0..n`.
fn shuffled(index: usize, n: usize, seed: u64) -> usize {
    let key = |i: usize| {
        let mut x = (i as u64 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 31;
        x.wrapping_mul(0x94D0_49BB_1331_11EB)
    };
    let mine = key(index);
    (0..n).filter(|&j| key(j) < mine || (key(j) == mine && j < index)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fade(stagger: f64) -> TextAnimator {
        TextAnimator {
            easing: Easing::Linear,
            ..TextAnimator::preset("Fade Up by Word").map(|a| TextAnimator { stagger, duration: 1.0, ..a }).unwrap()
        }
    }

    #[test]
    fn units_start_one_stagger_apart() {
        let a = fade(0.5);
        assert_eq!(a.progress(0, 3, 0.5), 0.5);
        assert_eq!(a.progress(1, 3, 0.5), 0.0);
        assert_eq!(a.progress(1, 3, 1.0), 0.5);
        assert_eq!(a.progress(2, 3, 2.0), 1.0);
        assert_eq!(a.end(3), 2.0);
    }

    #[test]
    fn orders() {
        let a = TextAnimator { order: TextOrder::Reverse, ..fade(1.0) };
        assert_eq!(a.progress(2, 3, 0.5), 0.5, "the last goes first");
        let c = TextAnimator { order: TextOrder::Center, ..fade(1.0) };
        assert_eq!(c.progress(1, 3, 0.5), 0.5, "the middle goes first");
        assert_eq!(c.progress(0, 3, 0.5), c.progress(2, 3, 0.5));
        // Random is a shuffle: every unit gets a distinct start.
        let r = TextAnimator { order: TextOrder::Random, seed: 3, ..fade(1.0) };
        let mut starts: Vec<usize> = (0..10).map(|i| shuffled(i, 10, r.seed)).collect();
        starts.sort();
        assert_eq!(starts, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn letters_and_words_skip_spaces() {
        let p = places("ab  cd");
        assert_eq!(place_at(&p, 0, 0), GlyphPlace { letter: Some(0), word: Some(0), line: 0 });
        assert_eq!(place_at(&p, 2, 0), GlyphPlace { letter: None, word: None, line: 0 });
        assert_eq!(place_at(&p, 4, 1), GlyphPlace { letter: Some(2), word: Some(1), line: 1 });
    }

    #[test]
    fn looks_blend_from_the_start_state_to_rest() {
        let a = fade(0.0);
        let counts = UnitCounts { letters: 1, words: 1, lines: 1 };
        let place = GlyphPlace { letter: Some(0), word: Some(0), line: 0 };
        let at = |t| glyph_look(std::slice::from_ref(&a), place, counts, kurbo::Point::ZERO, 0.0, t);
        assert_eq!(at(0.0).opacity, 0.0);
        assert_eq!(at(0.0).transform, Affine::translate((0.0, 30.0)));
        assert_eq!(at(1.0), GlyphLook::REST);
        assert!((at(0.5).opacity - 0.5).abs() < 1e-9);
        // Out: leaves instead.
        let out = TextAnimator { out: true, ..a };
        assert_eq!(glyph_look(&[out], place, counts, kurbo::Point::ZERO, 0.0, 1.0).opacity, 0.0);
    }

    #[test]
    fn every_preset_exists_and_round_trips() {
        for name in PRESETS {
            let a = TextAnimator::preset(name).unwrap();
            let json = serde_json::to_string(&a).unwrap();
            assert_eq!(serde_json::from_str::<TextAnimator>(&json).unwrap(), a);
        }
    }
}
