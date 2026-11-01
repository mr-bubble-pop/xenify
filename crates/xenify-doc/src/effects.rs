//! Layer effects (drop shadow, glow, blur, colour adjustment) and blend modes.

use crate::{Color, Property};
use kurbo::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: EffectKind,
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self { enabled: true, kind }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum EffectKind {
    /// A blurred copy of the layer's shape, offset behind it.
    DropShadow {
        #[serde(with = "compat")]
        color: Property<Color>,
        #[serde(with = "compat")]
        offset: Property<Vec2>,
        #[serde(with = "compat")]
        blur: Property<f64>,
    },
    /// A soft halo of colour around the layer.
    Glow {
        #[serde(with = "compat")]
        color: Property<Color>,
        #[serde(with = "compat")]
        radius: Property<f64>,
    },
    /// Blurs the layer itself.
    Blur {
        #[serde(with = "compat")]
        radius: Property<f64>,
    },
    /// Shifts the layer's colours (each −1…1; hue in degrees).
    AdjustColors {
        #[serde(with = "compat")]
        brightness: Property<f64>,
        #[serde(with = "compat")]
        contrast: Property<f64>,
        #[serde(with = "compat")]
        saturation: Property<f64>,
        #[serde(with = "compat")]
        hue: Property<f64>,
    },
}

/// An effect's settings at one moment (what gets drawn).
#[derive(Clone, Debug, PartialEq)]
pub enum Fx {
    DropShadow { color: Color, offset: Vec2, blur: f64 },
    Glow { color: Color, radius: f64 },
    Blur { radius: f64 },
    AdjustColors { brightness: f64, contrast: f64, saturation: f64, hue: f64 },
}

/// One setting of an effect (for keyframing it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectField {
    Color,
    Offset,
    /// A shadow's blur, a glow's size or a blur's amount.
    Radius,
    Brightness,
    Contrast,
    Saturation,
    Hue,
}

impl EffectField {
    pub const ALL: [EffectField; 7] = [
        EffectField::Color,
        EffectField::Offset,
        EffectField::Radius,
        EffectField::Brightness,
        EffectField::Contrast,
        EffectField::Saturation,
        EffectField::Hue,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EffectField::Color => "color",
            EffectField::Offset => "offset",
            EffectField::Radius => "radius",
            EffectField::Brightness => "brightness",
            EffectField::Contrast => "contrast",
            EffectField::Saturation => "saturation",
            EffectField::Hue => "hue",
        }
    }
}

impl EffectKind {
    pub fn label(&self) -> &'static str {
        match self {
            EffectKind::DropShadow { .. } => "Drop shadow",
            EffectKind::Glow { .. } => "Glow",
            EffectKind::Blur { .. } => "Blur",
            EffectKind::AdjustColors { .. } => "Adjust colours",
        }
    }

    /// The settings this kind of effect has, in display order.
    pub fn fields(&self) -> &'static [EffectField] {
        use EffectField::*;
        match self {
            EffectKind::DropShadow { .. } => &[Color, Offset, Radius],
            EffectKind::Glow { .. } => &[Color, Radius],
            EffectKind::Blur { .. } => &[Radius],
            EffectKind::AdjustColors { .. } => &[Brightness, Contrast, Saturation, Hue],
        }
    }

    /// The settings at `time`.
    pub fn at(&self, time: f64) -> Fx {
        match self {
            EffectKind::DropShadow { color, offset, blur } => Fx::DropShadow {
                color: color.value_at(time),
                offset: offset.value_at(time),
                blur: blur.value_at(time).max(0.0),
            },
            EffectKind::Glow { color, radius } => {
                Fx::Glow { color: color.value_at(time), radius: radius.value_at(time).max(0.0) }
            }
            EffectKind::Blur { radius } => Fx::Blur { radius: radius.value_at(time).max(0.0) },
            EffectKind::AdjustColors { brightness, contrast, saturation, hue } => Fx::AdjustColors {
                brightness: brightness.value_at(time),
                contrast: contrast.value_at(time),
                saturation: saturation.value_at(time),
                hue: hue.value_at(time),
            },
        }
    }

    /// New effects of each kind, with sensible settings.
    pub fn presets() -> [EffectKind; 4] {
        [
            EffectKind::DropShadow {
                color: Color::rgba(0.0, 0.0, 0.0, 0.5).into(),
                offset: Vec2::new(8.0, 10.0).into(),
                blur: 12.0.into(),
            },
            EffectKind::Glow { color: Color::rgba(1.0, 0.85, 0.3, 0.9).into(), radius: 16.0.into() },
            EffectKind::Blur { radius: 6.0.into() },
            EffectKind::AdjustColors {
                brightness: 0.0.into(),
                contrast: 0.0.into(),
                saturation: 0.0.into(),
                hue: 0.0.into(),
            },
        ]
    }
}

/// Effect settings are written as properties (so they can be keyframed);
/// files from before that hold plain values, which still load.
mod compat {
    use crate::{Animatable, Property};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(p: &Property<T>, s: S) -> Result<S::Ok, S::Error> {
        p.serialize(s)
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<Property<T>, D::Error>
    where
        T: Deserialize<'de> + Animatable,
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either<T> {
            Prop(Property<T>),
            Plain(T),
        }
        Ok(match Either::deserialize(d)? {
            Either::Prop(p) => p,
            Either::Plain(v) => Property::Static(v),
        })
    }
}

/// How a layer mixes with what's under it (the CSS / SVG `mix-blend-mode`s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// Adds light (CSS `plus-lighter`): glows, fire and sparks brighten
    /// whatever is under them.
    Add,
}

impl BlendMode {
    /// In Lottie's order (the index is the Lottie `bm` value; Add is 16).
    pub const ALL: [BlendMode; 17] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::Add,
    ];

    pub fn is_normal(&self) -> bool {
        *self == BlendMode::Normal
    }

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::ColorDodge => "Colour dodge",
            BlendMode::ColorBurn => "Colour burn",
            BlendMode::HardLight => "Hard light",
            BlendMode::SoftLight => "Soft light",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Colour",
            BlendMode::Luminosity => "Luminosity",
            BlendMode::Add => "Add (glow)",
        }
    }

    /// The CSS `mix-blend-mode` value.
    pub fn css(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColorDodge => "color-dodge",
            BlendMode::ColorBurn => "color-burn",
            BlendMode::HardLight => "hard-light",
            BlendMode::SoftLight => "soft-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
            BlendMode::Add => "plus-lighter",
        }
    }
}

/// `c` with an Adjust Colours effect applied.
pub fn adjust_color(c: Color, brightness: f64, contrast: f64, saturation: f64, hue: f64) -> Color {
    let (mut r, mut g, mut b) = (c.r, c.g, c.b);
    // Hue: rotate around the grey axis (luma-preserving matrix, as SVG's feColorMatrix hueRotate).
    if hue != 0.0 {
        let (s, co) = hue.to_radians().sin_cos();
        let m = [
            [0.213 + co * 0.787 - s * 0.213, 0.715 - co * 0.715 - s * 0.715, 0.072 - co * 0.072 + s * 0.928],
            [0.213 - co * 0.213 + s * 0.143, 0.715 + co * 0.285 + s * 0.140, 0.072 - co * 0.072 - s * 0.283],
            [0.213 - co * 0.213 - s * 0.787, 0.715 - co * 0.715 + s * 0.715, 0.072 + co * 0.928 + s * 0.072],
        ];
        (r, g, b) = (
            m[0][0] * r + m[0][1] * g + m[0][2] * b,
            m[1][0] * r + m[1][1] * g + m[1][2] * b,
            m[2][0] * r + m[2][1] * g + m[2][2] * b,
        );
    }
    // Saturation: towards / away from grey.
    let grey = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let sat = (1.0 + saturation).max(0.0);
    (r, g, b) = (grey + (r - grey) * sat, grey + (g - grey) * sat, grey + (b - grey) * sat);
    // Contrast around mid-grey, then brightness.
    let k = (1.0 + contrast).max(0.0);
    let f = |v: f64| ((v - 0.5) * k + 0.5 + brightness).clamp(0.0, 1.0);
    Color::rgba(f(r), f(g), f(b), c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_adjustments() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        assert_eq!(adjust_color(red, 0.0, 0.0, 0.0, 0.0), red);
        let grey = adjust_color(red, 0.0, 0.0, -1.0, 0.0);
        assert!((grey.r - grey.g).abs() < 1e-9 && (grey.g - grey.b).abs() < 1e-9, "desaturated: {grey:?}");
        let brighter = adjust_color(Color::rgb(0.2, 0.2, 0.2), 0.3, 0.0, 0.0, 0.0);
        assert!((brighter.r - 0.5).abs() < 1e-9);
        let turned = adjust_color(red, 0.0, 0.0, 0.0, 120.0);
        assert!(turned.g > turned.r && turned.g > turned.b, "red turned green: {turned:?}");
        let e = Effect::new(EffectKind::Blur { radius: 3.0.into() });
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Effect>(&json).unwrap(), e);
        // Files from before effects could be animated still load.
        let old: Effect = serde_json::from_str(r#"{"effect":"blur","radius":3.0}"#).unwrap();
        assert_eq!(old, e);
        let old: Effect = serde_json::from_str(
            r#"{"effect":"drop_shadow","color":{"r":0,"g":0,"b":0,"a":0.5},"offset":[8,10],"blur":12}"#,
        )
        .unwrap();
        assert!(matches!(old.kind.at(0.0), Fx::DropShadow { blur, .. } if blur == 12.0));
    }
}
