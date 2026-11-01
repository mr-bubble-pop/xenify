//! Generator layers: layers whose drawing is made by code from settings.
//!
//! A particle emitter throws particles from a shape (a point, line, box,
//! ellipse or another layer's outline) in a direction, lets gravity, drag,
//! turbulence and spin act on them, and ends them with a final look: fade,
//! shrink, settle on the floor, or fly into the outline of a chosen layer.
//! Everything is seeded, so a frame always looks the same. The maths is in
//! `xenify_anim::particles`.

use crate::{Color, GradientStop, Id, Property};
use kurbo::Vec2;
use serde::{Deserialize, Serialize};

/// Where particles are born, in the layer's own space (centred on its origin).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmitterShape {
    Point,
    /// A horizontal line this long (turn the layer to tilt it).
    Line {
        length: f64,
    },
    Rect {
        size: Vec2,
    },
    Ellipse {
        size: Vec2,
    },
    /// Along another layer's outline (e.g. flames coming off text or a shape).
    Layer {
        layer: Id,
    },
}

impl EmitterShape {
    pub fn label(&self) -> &'static str {
        match self {
            EmitterShape::Point => "Point",
            EmitterShape::Line { .. } => "Line",
            EmitterShape::Rect { .. } => "Box",
            EmitterShape::Ellipse { .. } => "Ellipse",
            EmitterShape::Layer { .. } => "Layer outline",
        }
    }
}

/// What each particle looks like.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParticleShape {
    /// Paper rectangles that flip as they fall.
    Confetti,
    Circle,
    Square,
    Star,
    /// A line along the direction of travel (sparks, rain).
    Streak,
    /// A soft round blob fading to its edge (smoke, fire, glow).
    Soft,
    /// A ring outline (bubbles, shockwaves).
    Ring,
    /// Any layer's drawing, used as the sprite (e.g. an emoji).
    Layer {
        layer: Id,
    },
}

impl ParticleShape {
    pub const SIMPLE: [ParticleShape; 7] = [
        ParticleShape::Confetti,
        ParticleShape::Circle,
        ParticleShape::Square,
        ParticleShape::Star,
        ParticleShape::Streak,
        ParticleShape::Soft,
        ParticleShape::Ring,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ParticleShape::Confetti => "Confetti",
            ParticleShape::Circle => "Circle",
            ParticleShape::Square => "Square",
            ParticleShape::Star => "Star",
            ParticleShape::Streak => "Streak",
            ParticleShape::Soft => "Soft blob",
            ParticleShape::Ring => "Ring",
            ParticleShape::Layer { .. } => "A layer",
        }
    }
}

/// How particles finish.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EndLook {
    /// Fade out over the last part of their life.
    Fade,
    /// Shrink to nothing over the last part of their life.
    Shrink,
    /// Fall to the floor (composition y, pixels) and stay there.
    Settle { floor: f64 },
    /// Fly into the outline of `target` and stay there, forming its shape.
    Form { target: Id },
}

impl EndLook {
    pub fn label(&self) -> &'static str {
        match self {
            EndLook::Fade => "Fade out",
            EndLook::Shrink => "Shrink away",
            EndLook::Settle { .. } => "Settle on the floor",
            EndLook::Form { .. } => "Form a layer's shape",
        }
    }
}

/// How many particles and when.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmitMode {
    /// All at once at `start` (`count` particles), repeated `repeat` times
    /// every `interval` seconds.
    Burst,
    /// A steady stream of `rate` per second from `start` to `stop`.
    Stream,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Emitter {
    pub shape: EmitterShape,
    pub mode: EmitMode,
    /// Particles per burst.
    pub count: Property<f64>,
    /// Particles per second (Stream).
    pub rate: Property<f64>,
    /// When emitting starts (seconds, layer time).
    #[serde(default)]
    pub start: f64,
    /// A stream that's already flowing at `start` (snow already falling)
    /// instead of starting from nothing.
    #[serde(default)]
    pub prewarm: bool,
    /// When a stream stops (`None`: keeps going).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<f64>,
    /// Extra bursts after the first, and the time between them.
    #[serde(default)]
    pub repeat: u32,
    #[serde(default = "one")]
    pub interval: f64,
    /// How long each particle lives (seconds), ± `lifetime_variation` (0..1).
    pub lifetime: Property<f64>,
    #[serde(default)]
    pub lifetime_variation: f64,

    /// Initial direction in degrees (0 = right, -90 = up) and the cone around it.
    pub direction: Property<f64>,
    pub spread: Property<f64>,
    /// Initial speed (pixels per second), ± `speed_variation` (0..1).
    pub speed: Property<f64>,
    #[serde(default)]
    pub speed_variation: f64,
    /// Turning speed (degrees per second), random in ± this.
    pub spin: Property<f64>,

    /// Pull in pixels per second² (positive y falls).
    pub gravity: Property<Vec2>,
    /// Air resistance: how quickly particles lose speed (per second).
    pub drag: Property<f64>,
    /// Random swirling drift, in pixels.
    pub turbulence: Property<f64>,
    /// Rolling smoke (a mushroom cloud's cap): particles turn round a ring
    /// the size of the Ellipse source, in degrees per second, riding along
    /// as the layer moves and grows. 0: off.
    #[serde(default = "zero")]
    pub vortex: Property<f64>,
    /// How fast the swirl changes (per second).
    #[serde(default = "one")]
    pub turbulence_speed: f64,

    /// Particle size (pixels), ± `size_variation` (0..1).
    pub size: Property<f64>,
    #[serde(default)]
    pub size_variation: f64,
    /// Size at the end of its life relative to the start (1 = unchanged,
    /// 3 = grows like smoke).
    #[serde(default = "one")]
    pub size_end: f64,
    pub particle: ParticleShape,
    /// Each particle takes one of these colours at random…
    pub colors: Vec<Color>,
    /// …or, when set, runs through this gradient over its life (fire:
    /// white → yellow → orange → red → smoke).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_over_life: Option<Vec<GradientStop>>,
    pub end: EndLook,
    /// Changes the random pattern.
    #[serde(default)]
    pub seed: u64,
}

fn one() -> f64 {
    1.0
}

fn zero() -> Property<f64> {
    0.0.into()
}

impl Default for Emitter {
    /// A confetti burst going up.
    fn default() -> Self {
        Self {
            shape: EmitterShape::Point,
            mode: EmitMode::Burst,
            count: 150.0.into(),
            rate: 40.0.into(),
            start: 0.0,
            prewarm: false,
            stop: None,
            repeat: 0,
            interval: 1.0,
            lifetime: 3.0.into(),
            lifetime_variation: 0.3,
            direction: (-90.0).into(),
            spread: 60.0.into(),
            speed: 900.0.into(),
            speed_variation: 0.4,
            spin: 360.0.into(),
            gravity: Vec2::new(0.0, 900.0).into(),
            drag: 1.2.into(),
            turbulence: 20.0.into(),
            vortex: 0.0.into(),
            turbulence_speed: 1.0,
            size: 14.0.into(),
            size_variation: 0.3,
            size_end: 1.0,
            particle: ParticleShape::Confetti,
            colors: ["#ef476f", "#ffd166", "#06d6a0", "#118ab2", "#8338ec", "#ffffff"]
                .iter()
                .filter_map(|h| Color::from_hex(h))
                .collect(),
            color_over_life: None,
            end: EndLook::Fade,
            seed: 1,
        }
    }
}

impl Emitter {
    /// Points the emitter at other layers after they were copied with new ids.
    pub fn remap(&mut self, map: &dyn Fn(Id) -> Option<Id>) {
        if let EmitterShape::Layer { layer } = &mut self.shape
            && let Some(n) = map(*layer)
        {
            *layer = n;
        }
        if let ParticleShape::Layer { layer } = &mut self.particle
            && let Some(n) = map(*layer)
        {
            *layer = n;
        }
        if let EndLook::Form { target } = &mut self.end
            && let Some(n) = map(*target)
        {
            *target = n;
        }
    }
}

/// The keyframeable settings of an emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmitterField {
    Count,
    Rate,
    Lifetime,
    Direction,
    Spread,
    Speed,
    Spin,
    Gravity,
    Drag,
    Turbulence,
    Size,
    Vortex,
}

impl EmitterField {
    pub const ALL: [EmitterField; 12] = [
        EmitterField::Count,
        EmitterField::Rate,
        EmitterField::Lifetime,
        EmitterField::Direction,
        EmitterField::Spread,
        EmitterField::Speed,
        EmitterField::Spin,
        EmitterField::Gravity,
        EmitterField::Drag,
        EmitterField::Turbulence,
        EmitterField::Size,
        EmitterField::Vortex,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EmitterField::Count => "count",
            EmitterField::Rate => "rate",
            EmitterField::Lifetime => "lifetime",
            EmitterField::Direction => "direction",
            EmitterField::Spread => "spread",
            EmitterField::Speed => "speed",
            EmitterField::Spin => "spin",
            EmitterField::Gravity => "gravity",
            EmitterField::Drag => "drag",
            EmitterField::Turbulence => "turbulence",
            EmitterField::Size => "size",
            EmitterField::Vortex => "vortex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EmitterField::Count => "Particles",
            EmitterField::Rate => "Per Second",
            EmitterField::Lifetime => "Lifetime",
            EmitterField::Direction => "Direction",
            EmitterField::Spread => "Spread",
            EmitterField::Speed => "Speed",
            EmitterField::Spin => "Spin",
            EmitterField::Gravity => "Gravity",
            EmitterField::Drag => "Drag",
            EmitterField::Turbulence => "Turbulence",
            EmitterField::Size => "Size",
            EmitterField::Vortex => "Vortex",
        }
    }
}

/// Built-in particle looks, for the New menu and the AI.
pub const PARTICLE_PRESETS: &[&str] =
    &["Confetti Burst", "Confetti Rain", "Fireworks", "Sparkle", "Snow", "Rain", "Bubbles", "Assemble into Shape"];

/// Fire and smoke looks (particle layers too).
pub const FIRE_PRESETS: &[&str] = &[
    "Campfire",
    "Torch",
    "Wall of Fire",
    "Burning Shape",
    "Fire Trail",
    "Smoke Puffs",
    "Smoke Trail",
    "Rising Column",
    "Fog",
];

impl Emitter {
    /// Preset `name` for a `width`×`height` canvas, placed (by the caller) at
    /// the returned position. `None` for an unknown name.
    pub fn preset(name: &str, width: f64, height: f64) -> Option<(Emitter, Vec2)> {
        let hex = |h: &str| Color::from_hex(h).unwrap_or(Color::WHITE);
        let d = Emitter::default();
        let centre = Vec2::new(width / 2.0, height / 2.0);
        Some(match name {
            "Confetti Burst" => (d, Vec2::new(width / 2.0, height * 0.85)),
            "Confetti Rain" => (
                Emitter {
                    shape: EmitterShape::Line { length: width * 1.1 },
                    mode: EmitMode::Stream,
                    prewarm: true,
                    rate: 60.0.into(),
                    lifetime: 7.0.into(),
                    direction: 90.0.into(),
                    spread: 20.0.into(),
                    speed: 150.0.into(),
                    gravity: Vec2::new(0.0, 300.0).into(),
                    drag: 1.0.into(),
                    turbulence: 40.0.into(),
                    ..d
                },
                Vec2::new(width / 2.0, -20.0),
            ),
            "Fireworks" => (
                Emitter {
                    count: 120.0.into(),
                    repeat: 3,
                    interval: 0.8,
                    lifetime: 1.6.into(),
                    spread: 360.0.into(),
                    speed: 420.0.into(),
                    speed_variation: 0.6,
                    gravity: Vec2::new(0.0, 160.0).into(),
                    drag: 1.6.into(),
                    turbulence: 4.0.into(),
                    size: 5.0.into(),
                    particle: ParticleShape::Streak,
                    colors: ["#ffd166", "#ef476f", "#8ecae6", "#ffffff"].iter().map(|h| hex(h)).collect(),
                    ..d
                },
                Vec2::new(width / 2.0, height * 0.35),
            ),
            "Sparkle" => (
                Emitter {
                    shape: EmitterShape::Ellipse { size: Vec2::new(width * 0.3, height * 0.3) },
                    mode: EmitMode::Stream,
                    prewarm: true,
                    rate: 30.0.into(),
                    lifetime: 1.2.into(),
                    spread: 360.0.into(),
                    speed: 20.0.into(),
                    gravity: Vec2::ZERO.into(),
                    turbulence: 6.0.into(),
                    spin: 90.0.into(),
                    size: 18.0.into(),
                    particle: ParticleShape::Star,
                    colors: ["#fff3b0", "#ffffff", "#ffd166"].iter().map(|h| hex(h)).collect(),
                    end: EndLook::Shrink,
                    ..d
                },
                centre,
            ),
            "Snow" => (
                Emitter {
                    shape: EmitterShape::Line { length: width * 1.2 },
                    mode: EmitMode::Stream,
                    prewarm: true,
                    rate: 40.0.into(),
                    lifetime: 12.0.into(),
                    direction: 90.0.into(),
                    spread: 10.0.into(),
                    speed: 100.0.into(),
                    gravity: Vec2::new(0.0, 60.0).into(),
                    drag: 0.5.into(),
                    turbulence: 60.0.into(),
                    turbulence_speed: 0.4,
                    spin: 0.0.into(),
                    size: 8.0.into(),
                    size_variation: 0.6,
                    particle: ParticleShape::Soft,
                    colors: vec![Color::WHITE],
                    ..d
                },
                Vec2::new(width / 2.0, -20.0),
            ),
            "Rain" => (
                Emitter {
                    shape: EmitterShape::Line { length: width * 1.3 },
                    mode: EmitMode::Stream,
                    prewarm: true,
                    rate: 160.0.into(),
                    lifetime: 1.4.into(),
                    direction: 100.0.into(),
                    spread: 2.0.into(),
                    speed: 1400.0.into(),
                    speed_variation: 0.1,
                    gravity: Vec2::new(0.0, 400.0).into(),
                    drag: 0.0.into(),
                    turbulence: 0.0.into(),
                    spin: 0.0.into(),
                    size: 3.0.into(),
                    particle: ParticleShape::Streak,
                    colors: vec![hex("#a8c5e6").with_alpha(0.7)],
                    ..d
                },
                Vec2::new(width / 2.0, -40.0),
            ),
            "Bubbles" => (
                Emitter {
                    shape: EmitterShape::Line { length: width * 0.8 },
                    mode: EmitMode::Stream,
                    prewarm: true,
                    rate: 12.0.into(),
                    lifetime: 5.0.into(),
                    direction: (-90.0).into(),
                    spread: 10.0.into(),
                    speed: 180.0.into(),
                    gravity: Vec2::new(0.0, -20.0).into(),
                    drag: 0.3.into(),
                    turbulence: 30.0.into(),
                    spin: 0.0.into(),
                    size: 30.0.into(),
                    size_variation: 0.6,
                    particle: ParticleShape::Ring,
                    colors: vec![hex("#bde0fe"), hex("#ffffff")],
                    ..d
                },
                Vec2::new(width / 2.0, height + 20.0),
            ),
            "Assemble into Shape" => (
                Emitter {
                    count: 400.0.into(),
                    lifetime: 2.5.into(),
                    lifetime_variation: 0.1,
                    spread: 360.0.into(),
                    speed: 700.0.into(),
                    gravity: Vec2::ZERO.into(),
                    size: 8.0.into(),
                    particle: ParticleShape::Circle,
                    colors: vec![hex("#8ecae6"), hex("#ffffff")],
                    // The caller points it at a layer (the selection).
                    end: EndLook::Fade,
                    ..d
                },
                centre,
            ),
            "Campfire" | "Torch" | "Wall of Fire" | "Burning Shape" | "Fire Trail" => {
                let base = Emitter {
                    mode: EmitMode::Stream,
                    prewarm: true,
                    lifetime: 1.3.into(),
                    lifetime_variation: 0.3,
                    direction: (-90.0).into(),
                    spread: 25.0.into(),
                    speed: 80.0.into(),
                    speed_variation: 0.5,
                    spin: 0.0.into(),
                    gravity: Vec2::new(0.0, -650.0).into(),
                    drag: 1.2.into(),
                    turbulence: 18.0.into(),
                    turbulence_speed: 2.5,
                    size: 60.0.into(),
                    size_variation: 0.4,
                    size_end: 0.25,
                    particle: ParticleShape::Soft,
                    colors: vec![hex("#ffb347")],
                    color_over_life: Some(fire_colours()),
                    end: EndLook::Fade,
                    ..d
                };
                match name {
                    "Campfire" => (
                        Emitter {
                            shape: EmitterShape::Ellipse { size: Vec2::new(140.0, 30.0) },
                            rate: 160.0.into(),
                            ..base
                        },
                        Vec2::new(width / 2.0, height * 0.8),
                    ),
                    "Torch" => (
                        Emitter {
                            shape: EmitterShape::Ellipse { size: Vec2::new(40.0, 12.0) },
                            rate: 90.0.into(),
                            size: 34.0.into(),
                            speed: 40.0.into(),
                            gravity: Vec2::new(0.0, -450.0).into(),
                            ..base
                        },
                        centre,
                    ),
                    "Wall of Fire" => (
                        Emitter {
                            shape: EmitterShape::Line { length: width * 0.9 },
                            rate: 600.0.into(),
                            size: 90.0.into(),
                            lifetime: 1.4.into(),
                            gravity: Vec2::new(0.0, -800.0).into(),
                            ..base
                        },
                        Vec2::new(width / 2.0, height * 0.95),
                    ),
                    // The caller points the source at a layer (the selection).
                    "Burning Shape" => (
                        Emitter {
                            rate: 300.0.into(),
                            size: 40.0.into(),
                            lifetime: 0.9.into(),
                            speed: 20.0.into(),
                            spread: 60.0.into(),
                            gravity: Vec2::new(0.0, -350.0).into(),
                            ..base
                        },
                        centre,
                    ),
                    _ => (
                        Emitter {
                            prewarm: false,
                            rate: 120.0.into(),
                            lifetime: 0.7.into(),
                            speed: 20.0.into(),
                            spread: 360.0.into(),
                            gravity: Vec2::new(0.0, -200.0).into(),
                            size: 40.0.into(),
                            ..base
                        },
                        centre,
                    ),
                }
            }
            "Smoke Puffs" | "Smoke Trail" | "Rising Column" | "Fog" => {
                let base = Emitter {
                    lifetime: 2.5.into(),
                    lifetime_variation: 0.3,
                    spread: 360.0.into(),
                    speed: 120.0.into(),
                    speed_variation: 0.5,
                    spin: 20.0.into(),
                    gravity: Vec2::new(0.0, -40.0).into(),
                    drag: 2.0.into(),
                    turbulence: 20.0.into(),
                    turbulence_speed: 0.8,
                    size: 60.0.into(),
                    size_variation: 0.4,
                    size_end: 4.0,
                    particle: ParticleShape::Soft,
                    colors: vec![hex("#808080")],
                    color_over_life: Some(smoke_colours()),
                    end: EndLook::Fade,
                    ..d
                };
                match name {
                    "Smoke Puffs" => (Emitter { count: 25.0.into(), ..base }, centre),
                    "Smoke Trail" => (
                        Emitter {
                            mode: EmitMode::Stream,
                            rate: 40.0.into(),
                            lifetime: 3.0.into(),
                            speed: 10.0.into(),
                            gravity: Vec2::new(0.0, -30.0).into(),
                            size: 30.0.into(),
                            size_end: 5.0,
                            ..base
                        },
                        centre,
                    ),
                    "Rising Column" => (
                        Emitter {
                            shape: EmitterShape::Ellipse { size: Vec2::new(80.0, 20.0) },
                            mode: EmitMode::Stream,
                            prewarm: true,
                            rate: 50.0.into(),
                            lifetime: 5.0.into(),
                            direction: (-90.0).into(),
                            spread: 15.0.into(),
                            speed: 150.0.into(),
                            gravity: Vec2::new(0.0, -60.0).into(),
                            drag: 0.3.into(),
                            turbulence: 40.0.into(),
                            size: 70.0.into(),
                            ..base
                        },
                        Vec2::new(width / 2.0, height * 0.9),
                    ),
                    _ => (
                        Emitter {
                            shape: EmitterShape::Rect { size: Vec2::new(width * 1.2, height * 0.3) },
                            mode: EmitMode::Stream,
                            prewarm: true,
                            rate: 6.0.into(),
                            lifetime: 10.0.into(),
                            speed: 15.0.into(),
                            gravity: Vec2::ZERO.into(),
                            drag: 0.0.into(),
                            turbulence: 60.0.into(),
                            turbulence_speed: 0.2,
                            spin: 0.0.into(),
                            size: 500.0.into(),
                            size_end: 1.3,
                            colors: vec![Color::rgba(0.8, 0.85, 0.9, 0.12)],
                            color_over_life: Some(vec![
                                GradientStop { offset: 0.0, color: Color::rgba(0.8, 0.85, 0.9, 0.0) },
                                GradientStop { offset: 0.3, color: Color::rgba(0.8, 0.85, 0.9, 0.12) },
                                GradientStop { offset: 1.0, color: Color::rgba(0.8, 0.85, 0.9, 0.0) },
                            ]),
                            ..base
                        },
                        Vec2::new(width / 2.0, height * 0.8),
                    ),
                }
            }
            _ => return None,
        })
    }

    /// Whether the preset glows (drawn with the Add blend and a Glow).
    pub fn preset_glows(name: &str) -> bool {
        matches!(name, "Campfire" | "Torch" | "Wall of Fire" | "Burning Shape" | "Fire Trail" | "Sparkle" | "Fireworks")
    }
}

/// White-hot to yellow, orange, red and finally thin smoke.
pub fn fire_colours() -> Vec<GradientStop> {
    [
        (0.0, Color::rgba(1.0, 1.0, 0.9, 1.0)),
        (0.15, Color::rgba(1.0, 0.85, 0.3, 0.9)),
        (0.4, Color::rgba(1.0, 0.45, 0.05, 0.8)),
        (0.7, Color::rgba(0.6, 0.1, 0.05, 0.45)),
        (1.0, Color::rgba(0.2, 0.2, 0.2, 0.0)),
    ]
    .into_iter()
    .map(|(offset, color)| GradientStop { offset, color })
    .collect()
}

/// Grey smoke that thickens, then thins away.
pub fn smoke_colours() -> Vec<GradientStop> {
    [
        (0.0, Color::rgba(0.55, 0.55, 0.55, 0.0)),
        (0.2, Color::rgba(0.5, 0.5, 0.5, 0.5)),
        (0.6, Color::rgba(0.35, 0.35, 0.35, 0.35)),
        (1.0, Color::rgba(0.3, 0.3, 0.3, 0.0)),
    ]
    .into_iter()
    .map(|(offset, color)| GradientStop { offset, color })
    .collect()
}

/// Water (or any liquid) filling a box: its surface rises to `level`,
/// with waves, tilt and slosh, bubbles and a foam line. Use it as the
/// masked layer under text (Set Mask) to fill the text with water.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidFill {
    /// The container, centred on the layer's origin.
    pub size: Vec2,
    /// How full: 0 = empty, 1 = to the brim.
    pub level: Property<f64>,
    /// The way it rises, degrees (-90 = up, the usual).
    #[serde(default = "up")]
    pub direction: f64,
    /// Tilt of the surface, degrees.
    pub tilt: Property<f64>,
    /// Sloshing: extra rocking tilt (degrees) and how often (per second).
    #[serde(default)]
    pub slosh: f64,
    #[serde(default = "one")]
    pub slosh_speed: f64,
    /// Waves: how many layered wave trains, their height (pixels),
    /// length (pixels) and speed (wavelengths per second).
    #[serde(default = "two")]
    pub waves: u32,
    pub wave_height: Property<f64>,
    pub wave_length: f64,
    pub wave_speed: f64,
    /// Water colour at the surface, and in the depths (a gradient).
    pub color: Property<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_color: Option<Color>,
    /// Bubbles rising (how many at once).
    #[serde(default)]
    pub bubbles: u32,
    /// A bright line along the surface, this wide (0: none).
    #[serde(default)]
    pub foam: f64,
    #[serde(default)]
    pub seed: u64,
}

fn up() -> f64 {
    -90.0
}

fn two() -> u32 {
    2
}

impl LiquidFill {
    /// Water filling a `width`×`height` box.
    pub fn water(width: f64, height: f64) -> Self {
        Self {
            size: Vec2::new(width, height),
            level: 0.5.into(),
            direction: -90.0,
            tilt: 0.0.into(),
            slosh: 0.0,
            slosh_speed: 0.6,
            waves: 2,
            wave_height: (height * 0.02).clamp(4.0, 30.0).into(),
            wave_length: (width * 0.35).max(40.0),
            wave_speed: 0.4,
            color: Color::rgba(0.2, 0.6, 0.95, 0.85).into(),
            deep_color: Some(Color::rgba(0.02, 0.2, 0.45, 0.95)),
            bubbles: 12,
            foam: 4.0,
            seed: 1,
        }
    }
}

/// The keyframeable settings of a liquid fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiquidField {
    Level,
    Tilt,
    WaveHeight,
    Color,
}

impl LiquidField {
    pub const ALL: [LiquidField; 4] =
        [LiquidField::Level, LiquidField::Tilt, LiquidField::WaveHeight, LiquidField::Color];

    pub fn name(self) -> &'static str {
        match self {
            LiquidField::Level => "level",
            LiquidField::Tilt => "tilt",
            LiquidField::WaveHeight => "wave_height",
            LiquidField::Color => "color",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LiquidField::Level => "Level",
            LiquidField::Tilt => "Tilt",
            LiquidField::WaveHeight => "Wave Height",
            LiquidField::Color => "Water Colour",
        }
    }
}
