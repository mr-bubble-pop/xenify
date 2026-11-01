//! Slides: a presentation is a project whose compositions are shown in order,
//! each with a transition in and out and speaker notes. A plain animation
//! has no slide list.

use crate::{Color, Easing, Id};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    /// The composition shown.
    pub comp: Id,
    /// How this slide arrives (over the one before).
    #[serde(default)]
    pub transition_in: Transition,
    /// How this slide leaves (under the one after).
    #[serde(default)]
    pub transition_out: Transition,
    /// Speaker notes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

impl Slide {
    pub fn new(comp: Id) -> Self {
        Self { comp, transition_in: Transition::default(), transition_out: Transition::default(), notes: String::new() }
    }
}

/// How one slide hands over to the next.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "TransitionFile")]
pub struct Transition {
    pub kind: TransitionKind,
    /// Seconds (ignored for [`TransitionKind::Cut`]).
    pub duration: f64,
    /// Which way it moves (pushes, covers, wipes, blinds, flips).
    #[serde(default)]
    pub direction: Direction,
    /// Dip to Colour's colour; the particles of Particle Dissolve.
    #[serde(default = "black")]
    pub color: Color,
    #[serde(default = "ease_in_out")]
    pub easing: Easing,
    /// Soft edges for wipes, the iris and the clock (0 = hard, 1 = very soft).
    #[serde(default)]
    pub softness: f64,
}

fn black() -> Color {
    Color::BLACK
}

fn ease_in_out() -> Easing {
    Easing::EASE_IN_OUT
}

impl Default for Transition {
    fn default() -> Self {
        Self::new(TransitionKind::Cut, 0.5)
    }
}

impl Transition {
    pub fn new(kind: TransitionKind, duration: f64) -> Self {
        Self {
            kind,
            duration,
            direction: Direction::Left,
            color: Color::BLACK,
            easing: Easing::EASE_IN_OUT,
            softness: 0.0,
        }
    }

    /// Seconds it actually takes.
    pub fn length(&self) -> f64 {
        if self.kind == TransitionKind::Cut { 0.0 } else { self.duration.max(0.0) }
    }
}

/// A transition as written in files, old names included: `push_left`,
/// `push_up` and `zoom` became Push with a direction and Zoom In.
#[derive(Deserialize)]
struct TransitionFile {
    kind: KindFile,
    duration: f64,
    #[serde(default)]
    direction: Option<Direction>,
    #[serde(default = "black")]
    color: Color,
    #[serde(default = "ease_in_out")]
    easing: Easing,
    #[serde(default)]
    softness: f64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum KindFile {
    Now(TransitionKind),
    Old(OldKind),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OldKind {
    PushLeft,
    PushUp,
    Zoom,
}

impl From<TransitionFile> for Transition {
    fn from(f: TransitionFile) -> Self {
        let (kind, implied) = match f.kind {
            KindFile::Now(k) => (k, None),
            KindFile::Old(OldKind::PushLeft) => (TransitionKind::Push, Some(Direction::Left)),
            KindFile::Old(OldKind::PushUp) => (TransitionKind::Push, Some(Direction::Up)),
            KindFile::Old(OldKind::Zoom) => (TransitionKind::ZoomIn, None),
        };
        Transition {
            kind,
            duration: f.duration,
            direction: f.direction.or(implied).unwrap_or_default(),
            color: f.color,
            easing: f.easing,
            softness: f.softness,
        }
    }
}

/// Which way a transition moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [Direction::Left, Direction::Right, Direction::Up, Direction::Down];

    pub fn label(self) -> &'static str {
        match self {
            Direction::Left => "Left",
            Direction::Right => "Right",
            Direction::Up => "Up",
            Direction::Down => "Down",
        }
    }

    /// The way things move, one frame's width or height per unit.
    pub fn vector(self) -> kurbo::Vec2 {
        match self {
            Direction::Left => kurbo::Vec2::new(-1.0, 0.0),
            Direction::Right => kurbo::Vec2::new(1.0, 0.0),
            Direction::Up => kurbo::Vec2::new(0.0, -1.0),
            Direction::Down => kurbo::Vec2::new(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// Instant.
    #[default]
    Cut,
    Fade,
    /// Through a colour: out to it, then in from it.
    DipToColor,
    /// The new slide pushes the old one out.
    Push,
    /// The new slide slides in over the old one.
    Cover,
    /// The old slide slides away, showing the new one under it.
    Uncover,
    /// A straight edge sweeps across, showing the new slide.
    Wipe,
    /// A hand sweeps round like a clock, showing the new slide.
    ClockWipe,
    /// A circle opens from the middle.
    Iris,
    /// Stripes open like window blinds.
    Blinds,
    /// The old slide rushes towards you as the new one settles in.
    ZoomIn,
    /// The old slide falls away as the new one comes down.
    ZoomOut,
    /// Turns out and in.
    Spin,
    /// Flips over like a card.
    Flip,
    /// Blurs out and in.
    BlurDissolve,
    /// A wave rises, the new slide under it.
    Liquid,
    /// The old slide breaks into pieces that fall away.
    Shatter,
    /// The old slide breaks into specks that drift away.
    ParticleDissolve,
    /// Layers with the same name glide from where they were to where they
    /// are; the rest fade.
    SmartMorph,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 19] = [
        TransitionKind::Cut,
        TransitionKind::Fade,
        TransitionKind::DipToColor,
        TransitionKind::Push,
        TransitionKind::Cover,
        TransitionKind::Uncover,
        TransitionKind::Wipe,
        TransitionKind::ClockWipe,
        TransitionKind::Iris,
        TransitionKind::Blinds,
        TransitionKind::ZoomIn,
        TransitionKind::ZoomOut,
        TransitionKind::Spin,
        TransitionKind::Flip,
        TransitionKind::BlurDissolve,
        TransitionKind::Liquid,
        TransitionKind::Shatter,
        TransitionKind::ParticleDissolve,
        TransitionKind::SmartMorph,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Cut => "Cut",
            TransitionKind::Fade => "Fade",
            TransitionKind::DipToColor => "Dip to Colour",
            TransitionKind::Push => "Push",
            TransitionKind::Cover => "Cover",
            TransitionKind::Uncover => "Uncover",
            TransitionKind::Wipe => "Wipe",
            TransitionKind::ClockWipe => "Clock Wipe",
            TransitionKind::Iris => "Iris",
            TransitionKind::Blinds => "Blinds",
            TransitionKind::ZoomIn => "Zoom In",
            TransitionKind::ZoomOut => "Zoom Out",
            TransitionKind::Spin => "Spin",
            TransitionKind::Flip => "Flip",
            TransitionKind::BlurDissolve => "Blur Dissolve",
            TransitionKind::Liquid => "Liquid",
            TransitionKind::Shatter => "Shatter",
            TransitionKind::ParticleDissolve => "Particle Dissolve",
            TransitionKind::SmartMorph => "Smart Morph",
        }
    }

    /// Whether its direction setting does anything.
    pub fn has_direction(self) -> bool {
        matches!(
            self,
            TransitionKind::Push
                | TransitionKind::Cover
                | TransitionKind::Uncover
                | TransitionKind::Wipe
                | TransitionKind::Blinds
                | TransitionKind::Flip
                | TransitionKind::Liquid
        )
    }

    /// Whether its colour setting does anything.
    pub fn has_color(self) -> bool {
        matches!(self, TransitionKind::DipToColor | TransitionKind::ParticleDissolve)
    }

    /// Whether its softness setting does anything.
    pub fn has_softness(self) -> bool {
        matches!(self, TransitionKind::Wipe | TransitionKind::ClockWipe | TransitionKind::Iris | TransitionKind::Blinds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_transition_names_still_load() {
        let t: Transition = serde_json::from_str(r#"{"kind":"push_left","duration":0.7}"#).unwrap();
        assert_eq!((t.kind, t.direction, t.duration), (TransitionKind::Push, Direction::Left, 0.7));
        let t: Transition = serde_json::from_str(r#"{"kind":"push_up","duration":0.5}"#).unwrap();
        assert_eq!((t.kind, t.direction), (TransitionKind::Push, Direction::Up));
        let t: Transition = serde_json::from_str(r#"{"kind":"zoom","duration":0.5}"#).unwrap();
        assert_eq!(t.kind, TransitionKind::ZoomIn);
        let t: Transition = serde_json::from_str(r#"{"kind":"fade","duration":0.5}"#).unwrap();
        assert_eq!((t.kind, t.easing), (TransitionKind::Fade, Easing::EASE_IN_OUT));
    }

    #[test]
    fn new_transitions_round_trip() {
        for kind in TransitionKind::ALL {
            let t = Transition {
                direction: Direction::Down,
                softness: 0.3,
                color: Color::WHITE,
                ..Transition::new(kind, 1.2)
            };
            let json = serde_json::to_string(&t).unwrap();
            assert_eq!(serde_json::from_str::<Transition>(&json).unwrap(), t, "{json}");
        }
    }
}
