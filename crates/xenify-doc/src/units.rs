//! Measurement units. Everything is stored in pixels; a document can show
//! and take sizes in millimetres, centimetres, inches or points instead, at
//! a chosen pixels-per-inch.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Px,
    Mm,
    Cm,
    In,
    Pt,
}

impl Unit {
    pub const ALL: [Unit; 5] = [Unit::Px, Unit::Mm, Unit::Cm, Unit::In, Unit::Pt];

    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Px => "px",
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::In => "in",
            Unit::Pt => "pt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Unit::Px => "Pixels",
            Unit::Mm => "Millimetres",
            Unit::Cm => "Centimetres",
            Unit::In => "Inches",
            Unit::Pt => "Points",
        }
    }

    /// Inches per unit (pixels have none: they depend on the resolution).
    fn inches(self) -> Option<f64> {
        match self {
            Unit::Px => None,
            Unit::Mm => Some(1.0 / 25.4),
            Unit::Cm => Some(1.0 / 2.54),
            Unit::In => Some(1.0),
            Unit::Pt => Some(1.0 / 72.0),
        }
    }
}

/// How a document measures: its unit and resolution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Units {
    pub unit: Unit,
    /// Pixels per inch.
    pub dpi: f64,
}

impl Default for Units {
    fn default() -> Self {
        Self { unit: Unit::Px, dpi: 96.0 }
    }
}

impl Units {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Pixels in one unit.
    pub fn px_per_unit(&self) -> f64 {
        self.unit.inches().map_or(1.0, |i| i * self.dpi.max(1e-6))
    }

    pub fn from_px(&self, px: f64) -> f64 {
        px / self.px_per_unit()
    }

    pub fn to_px(&self, v: f64) -> f64 {
        v * self.px_per_unit()
    }

    /// Sensible decimals for this unit.
    pub fn decimals(&self) -> usize {
        match self.unit {
            Unit::Px | Unit::Pt => 1,
            Unit::Mm => 2,
            Unit::Cm | Unit::In => 3,
        }
    }

    /// `px` written in this unit, e.g. `"25.4 mm"`.
    pub fn format(&self, px: f64) -> String {
        let v = self.from_px(px);
        let s = format!("{v:.*}", self.decimals());
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        format!("{s} {}", self.unit.suffix())
    }
}

/// A guide line: vertical at x = `pos`, or horizontal at y = `pos`
/// (composition pixels).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuideLine {
    pub vertical: bool,
    pub pos: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        let mm = Units { unit: Unit::Mm, dpi: 300.0 };
        assert!((mm.to_px(25.4) - 300.0).abs() < 1e-9);
        assert_eq!(mm.format(300.0), "25.4 mm");
        let pt = Units { unit: Unit::Pt, dpi: 72.0 };
        assert_eq!(pt.from_px(10.0), 10.0);
        assert_eq!(Units::default().format(12.0), "12 px");
        let cm = Units { unit: Unit::Cm, dpi: 96.0 };
        assert_eq!(cm.format(cm.to_px(2.5)), "2.5 cm");
    }
}
