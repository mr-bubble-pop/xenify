use serde::{Deserialize, Serialize};

/// Straight-alpha sRGB colour, each channel in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    #[serde(default = "one")]
    pub a: f64,
}

fn one() -> f64 {
    1.0
}

impl Color {
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    pub const fn rgb(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let f = |v: u8| v as f64 / 255.0;
        match hex.len() {
            3 => Some(Self::rgb(f(digit(0)? * 17), f(digit(1)? * 17), f(digit(2)? * 17))),
            6 => Some(Self::rgb(f(byte(0)?), f(byte(2)?), f(byte(4)?))),
            8 => Some(Self::rgba(f(byte(0)?), f(byte(2)?), f(byte(4)?), f(byte(6)?))),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a >= 1.0 {
            format!("#{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b))
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b), b(self.a))
        }
    }

    pub fn with_alpha(self, a: f64) -> Self {
        Self { a, ..self }
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [b(self.r), b(self.g), b(self.b), b(self.a)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(Color::from_hex("#ff8000").unwrap().to_hex(), "#ff8000");
        assert_eq!(Color::from_hex("fff").unwrap(), Color::WHITE);
        assert_eq!(Color::from_hex("#00000080").unwrap().to_hex(), "#00000080");
        assert!(Color::from_hex("#12").is_none());
        assert!(Color::from_hex("#gggggg").is_none());
    }
}
