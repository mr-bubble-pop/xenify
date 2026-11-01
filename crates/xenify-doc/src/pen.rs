//! Pen styles: how a stroke is drawn along its path (calligraphy, pastel,
//! dashes, …). The look is generated as vector shapes by `xenify_anim`, so it
//! renders and exports the same everywhere.

use serde::{Deserialize, Serialize};

/// The kind of pen. Each has sensible default settings ([`PenStyle::preset`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PenKind {
    /// A plain, even line (the normal stroke).
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
    DoubleLine,
    /// Thin at both ends, full in the middle.
    Tapered,
    /// A flat nib held at an angle: thick and thin by direction.
    Calligraphy,
    /// A soft round brush with pressure-like swelling.
    Brush,
    /// A pointed ink pen: crisp, slightly wobbly, tapered ends.
    Ink,
    /// A broad, slightly see-through marker with square ends.
    Marker,
    /// A felt tip with a soft edge.
    FeltTip,
    /// Fine grainy graphite.
    Pencil,
    /// Broad, broken, smudgy grain.
    Charcoal,
    /// Chalky, speckled and soft.
    Pastel,
    /// Waxy streaks with gaps.
    Crayon,
    /// Soft, layered washes with darker edges.
    Watercolor,
    /// A spray of fine dots that fades at the edges.
    Airbrush,
    /// Several fine parallel bristle lines.
    Bristle,
    /// A bright core with a soft glow around it.
    Glow,
    /// A few loose overlapping hand-drawn passes.
    Sketchy,
    /// A single wobbly, uneven line.
    Rough,
}

impl PenKind {
    pub const ALL: [PenKind; 21] = [
        PenKind::Solid,
        PenKind::Dashed,
        PenKind::Dotted,
        PenKind::DashDot,
        PenKind::DoubleLine,
        PenKind::Tapered,
        PenKind::Calligraphy,
        PenKind::Brush,
        PenKind::Ink,
        PenKind::Marker,
        PenKind::FeltTip,
        PenKind::Pencil,
        PenKind::Charcoal,
        PenKind::Pastel,
        PenKind::Crayon,
        PenKind::Watercolor,
        PenKind::Airbrush,
        PenKind::Bristle,
        PenKind::Glow,
        PenKind::Sketchy,
        PenKind::Rough,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PenKind::Solid => "Solid",
            PenKind::Dashed => "Dashed",
            PenKind::Dotted => "Dotted",
            PenKind::DashDot => "Dash-dot",
            PenKind::DoubleLine => "Double line",
            PenKind::Tapered => "Tapered",
            PenKind::Calligraphy => "Calligraphy",
            PenKind::Brush => "Brush",
            PenKind::Ink => "Ink pen",
            PenKind::Marker => "Marker",
            PenKind::FeltTip => "Felt tip",
            PenKind::Pencil => "Pencil",
            PenKind::Charcoal => "Charcoal",
            PenKind::Pastel => "Pastel",
            PenKind::Crayon => "Crayon",
            PenKind::Watercolor => "Watercolour",
            PenKind::Airbrush => "Airbrush",
            PenKind::Bristle => "Bristle brush",
            PenKind::Glow => "Glow",
            PenKind::Sketchy => "Sketchy",
            PenKind::Rough => "Rough",
        }
    }

    /// Which settings matter for this pen (for showing only relevant sliders).
    pub fn uses(self) -> PenParams {
        use PenKind::*;
        PenParams {
            taper: matches!(
                self,
                Tapered | Brush | Ink | Calligraphy | Pencil | Charcoal | Watercolor | Bristle | Sketchy
            ),
            roughness: matches!(
                self,
                Brush
                    | Ink
                    | Marker
                    | FeltTip
                    | Pencil
                    | Charcoal
                    | Pastel
                    | Crayon
                    | Watercolor
                    | Bristle
                    | Sketchy
                    | Rough
            ),
            grain: matches!(self, Pencil | Charcoal | Pastel | Crayon | Airbrush | Watercolor),
            angle: matches!(self, Calligraphy | Marker),
            thinning: matches!(self, Calligraphy | Brush | Ink | Rough),
            dash: matches!(self, Dashed | Dotted | DashDot),
            count: matches!(self, Bristle | Sketchy | Watercolor | Glow | DoubleLine),
        }
    }
}

/// Which [`PenStyle`] settings a pen kind uses.
#[derive(Clone, Copy, Debug, Default)]
pub struct PenParams {
    pub taper: bool,
    pub roughness: bool,
    pub grain: bool,
    pub angle: bool,
    pub thinning: bool,
    pub dash: bool,
    pub count: bool,
}

/// A pen and its settings. Unused settings are ignored by that pen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PenStyle {
    pub kind: PenKind,
    /// 0..1: how much the start / end thin out.
    pub taper_start: f64,
    pub taper_end: f64,
    /// 0..1: wobble and unevenness.
    pub roughness: f64,
    /// 0..1: texture density (more = denser grain / fuller coverage).
    pub grain: f64,
    /// Nib angle in degrees (calligraphy, marker).
    pub angle: f64,
    /// 0..1: how thin the thinnest part gets (calligraphy hairlines, brush pressure).
    pub thinning: f64,
    /// Dash length and gap, in stroke widths.
    pub dash: f64,
    pub gap: f64,
    /// 0..1: opacity of the marks.
    pub flow: f64,
    /// Bristles, sketch passes, wash layers, glow layers or lines.
    pub count: u32,
    /// Changes the random pattern.
    pub seed: u64,
    /// A name for saved custom pens (empty for the built-in presets).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Default for PenStyle {
    fn default() -> Self {
        Self::preset(PenKind::Solid)
    }
}

impl PenStyle {
    /// The built-in look of `kind`.
    pub fn preset(kind: PenKind) -> Self {
        let base = Self {
            kind,
            taper_start: 0.0,
            taper_end: 0.0,
            roughness: 0.0,
            grain: 0.5,
            angle: 45.0,
            thinning: 0.2,
            dash: 3.0,
            gap: 2.0,
            flow: 1.0,
            count: 1,
            seed: 0,
            name: String::new(),
        };
        use PenKind::*;
        match kind {
            Solid => base,
            Dashed => Self { dash: 3.0, gap: 2.0, ..base },
            Dotted => Self { dash: 0.0, gap: 1.6, ..base },
            DashDot => Self { dash: 4.0, gap: 1.5, ..base },
            DoubleLine => Self { count: 2, thinning: 0.3, ..base },
            Tapered => Self { taper_start: 1.0, taper_end: 1.0, ..base },
            Calligraphy => Self { angle: 40.0, thinning: 0.12, taper_start: 0.2, taper_end: 0.2, ..base },
            Brush => Self { taper_start: 0.6, taper_end: 0.9, roughness: 0.25, thinning: 0.45, ..base },
            Ink => Self { taper_start: 0.8, taper_end: 0.8, roughness: 0.1, thinning: 0.7, ..base },
            Marker => Self { flow: 0.8, roughness: 0.05, angle: 30.0, ..base },
            FeltTip => Self { flow: 0.9, roughness: 0.1, ..base },
            Pencil => Self { flow: 0.75, grain: 0.7, roughness: 0.15, taper_start: 0.3, taper_end: 0.3, ..base },
            Charcoal => Self { flow: 0.85, grain: 0.55, roughness: 0.35, taper_start: 0.2, taper_end: 0.4, ..base },
            Pastel => Self { flow: 0.85, grain: 0.6, roughness: 0.3, ..base },
            Crayon => Self { flow: 0.9, grain: 0.55, roughness: 0.2, ..base },
            Watercolor => {
                Self { flow: 0.35, grain: 0.4, roughness: 0.5, count: 3, taper_start: 0.3, taper_end: 0.5, ..base }
            }
            Airbrush => Self { flow: 0.8, grain: 0.6, ..base },
            Bristle => Self { count: 7, roughness: 0.3, taper_start: 0.3, taper_end: 0.6, ..base },
            Glow => Self { count: 4, flow: 1.0, ..base },
            Sketchy => Self { count: 3, roughness: 0.5, flow: 0.8, taper_start: 0.4, taper_end: 0.4, ..base },
            Rough => Self { roughness: 0.6, thinning: 0.6, ..base },
        }
    }

    pub fn is_solid(&self) -> bool {
        self.kind == PenKind::Solid
    }
}
