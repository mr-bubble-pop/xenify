//! Diagram parts: connectors that stay glued to shapes, the ports they glue
//! to, text labels on shapes and connectors, arrowheads, and the settings of
//! a diagram canvas.
//!
//! A connector is a shape layer with [`LayerMeta::connector`] set: its
//! first shape's outline is re-routed from the two ends every time the scene
//! is evaluated (`xenify-anim`), so moving or animating a shape drags its
//! connectors along. The stored outline is the last route, kept so the file
//! still draws something sensible where routing isn't available.
//!
//! [`LayerMeta::connector`]: crate::LayerMeta::connector

use crate::{Color, Id, TextAlign, Vec2};
use serde::{Deserialize, Serialize};

/// How a connector's route is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStyle {
    /// Horizontal and vertical runs with right-angle bends (elbow).
    #[default]
    Orthogonal,
    Straight,
    /// A smooth curve that still bends around shapes in the way.
    Curved,
}

impl RouteStyle {
    pub const ALL: [RouteStyle; 3] = [RouteStyle::Orthogonal, RouteStyle::Straight, RouteStyle::Curved];

    pub fn label(self) -> &'static str {
        match self {
            RouteStyle::Orthogonal => "Elbow",
            RouteStyle::Straight => "Straight",
            RouteStyle::Curved => "Curved",
        }
    }
}

/// How a connector shows it crosses another without joining it (line jumps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JumpStyle {
    #[default]
    Arc,
    Gap,
    Square,
    None,
}

impl JumpStyle {
    pub const ALL: [JumpStyle; 4] = [JumpStyle::Arc, JumpStyle::Gap, JumpStyle::Square, JumpStyle::None];

    pub fn label(self) -> &'static str {
        match self {
            JumpStyle::Arc => "Arc",
            JumpStyle::Gap => "Gap",
            JumpStyle::Square => "Square",
            JumpStyle::None => "None",
        }
    }
}

/// The side of a shape a port faces out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// A place on a shape where connectors glue.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Port {
    /// Position as a fraction of the layer's own bounding box (0,0 = top
    /// left, 1,1 = bottom right), so ports follow when the shape is resized.
    pub pos: Vec2,
    /// Which way connectors leave the port.
    pub side: PortSide,
}

impl Port {
    pub const fn new(x: f64, y: f64, side: PortSide) -> Self {
        Port { pos: Vec2::new(x, y), side }
    }
}

/// The ports of a shape without its own: the middle of each side.
pub const DEFAULT_PORTS: [Port; 4] = [
    Port::new(0.5, 0.0, PortSide::Top),
    Port::new(1.0, 0.5, PortSide::Right),
    Port::new(0.5, 1.0, PortSide::Bottom),
    Port::new(0.0, 0.5, PortSide::Left),
];

/// Which part of a shape a connector end is glued to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortRef {
    /// The whole shape: the route leaves from whichever side is best.
    #[default]
    Auto,
    /// One port (an index into the shape's ports).
    Port(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attach {
    pub layer: Id,
    #[serde(default)]
    pub port: PortRef,
}

/// One end of a connector.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectorEnd {
    /// The shape it's glued to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach: Option<Attach>,
    /// Where the end is when not glued (composition space). Also kept up to
    /// date as the last glued position, so detaching leaves it in place.
    pub point: Vec2,
}

impl ConnectorEnd {
    pub fn free(point: Vec2) -> Self {
        ConnectorEnd { attach: None, point }
    }

    pub fn glued(layer: Id, port: PortRef, point: Vec2) -> Self {
        ConnectorEnd { attach: Some(Attach { layer, port }), point }
    }
}

/// Moving marks along a connector, for showing data or work flowing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    /// Pixels per second along the line (negative runs backwards).
    pub speed: f64,
    #[serde(default)]
    pub kind: FlowKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    #[default]
    Dashes,
    Dots,
}

impl FlowKind {
    pub const ALL: [FlowKind; 2] = [FlowKind::Dashes, FlowKind::Dots];

    pub fn label(self) -> &'static str {
        match self {
            FlowKind::Dashes => "Dashes",
            FlowKind::Dots => "Dots",
        }
    }
}

/// A line that stays glued to shapes and finds its own way between them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connector {
    pub from: ConnectorEnd,
    pub to: ConnectorEnd,
    #[serde(default)]
    pub route: RouteStyle,
    /// Points the route must pass through (composition space), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<Vec2>,
    #[serde(default)]
    pub jump: JumpStyle,
    /// Half the width of a jump, in pixels.
    #[serde(default = "default_jump_size")]
    pub jump_size: f64,
    /// Rounds elbow bends (0 = sharp).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub corner_radius: f64,
    /// Find a way around other shapes (off: go straight through them).
    #[serde(default = "yes")]
    pub avoid: bool,
    /// Space kept around shapes, and how far a route leaves a port before
    /// turning.
    #[serde(default = "default_margin")]
    pub margin: f64,
    /// Where the label sits along the line, 0 = start, 1 = end.
    #[serde(default = "half")]
    pub label_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Flow>,
}

fn default_jump_size() -> f64 {
    5.0
}
fn default_margin() -> f64 {
    16.0
}
fn half() -> f64 {
    0.5
}
fn yes() -> bool {
    true
}
fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

impl Connector {
    /// A connector between two ends, with default styling.
    pub fn new(from: ConnectorEnd, to: ConnectorEnd) -> Self {
        Connector {
            from,
            to,
            route: RouteStyle::default(),
            waypoints: Vec::new(),
            jump: JumpStyle::default(),
            jump_size: default_jump_size(),
            corner_radius: 0.0,
            avoid: true,
            margin: default_margin(),
            label_at: 0.5,
            flow: None,
        }
    }

    /// Takes the diagram's defaults for route, jumps, margin and corners.
    pub fn styled(self, d: &DiagramSettings) -> Self {
        Connector {
            route: d.route,
            jump: d.jump,
            jump_size: d.jump_size,
            margin: d.margin,
            corner_radius: d.corner_radius,
            ..self
        }
    }

    /// The layers the connector is glued to.
    pub fn attached(&self) -> impl Iterator<Item = Id> + '_ {
        [self.from.attach, self.to.attach].into_iter().flatten().map(|a| a.layer)
    }

    /// Each glued end's layer, for remapping (paste, duplicate).
    pub fn attached_mut(&mut self) -> impl Iterator<Item = &mut Id> + '_ {
        [&mut self.from.attach, &mut self.to.attach].into_iter().flatten().map(|a| &mut a.layer)
    }
}

/// Text drawn centred in a shape (or along a connector), wrapping inside the
/// shape's box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeLabel {
    pub text: String,
    #[serde(default = "default_font")]
    pub font_family: String,
    #[serde(default = "default_size")]
    pub font_size: f64,
    #[serde(default = "default_color")]
    pub color: Color,
    #[serde(default = "center")]
    pub align: TextAlign,
    /// Space between the shape's edge and the text box.
    #[serde(default = "default_inset")]
    pub inset: f64,
    /// Under the shape instead of inside it (for icons).
    #[serde(default, skip_serializing_if = "is_false")]
    pub below: bool,
    /// On a connector: a box of this colour behind the text, which then sits
    /// on the line (default: the canvas colour on diagram canvases).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_font() -> String {
    "Inter".into()
}
fn default_size() -> f64 {
    16.0
}
fn default_color() -> Color {
    Color::rgb(0.1, 0.1, 0.12)
}
fn center() -> TextAlign {
    TextAlign::Center
}
fn default_inset() -> f64 {
    6.0
}

impl NodeLabel {
    pub fn new(text: impl Into<String>) -> Self {
        NodeLabel {
            text: text.into(),
            font_family: default_font(),
            font_size: default_size(),
            color: default_color(),
            align: TextAlign::Center,
            inset: default_inset(),
            below: false,
            background: None,
        }
    }
}

/// The settings of a diagram canvas (a composition with
/// [`crate::CompSettings::diagram`] set): defaults for new connectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagramSettings {
    pub route: RouteStyle,
    pub jump: JumpStyle,
    pub jump_size: f64,
    pub margin: f64,
    pub corner_radius: f64,
    /// Arrowhead for new connectors' ends.
    pub end_marker: Marker,
    /// Default line colour and width of new connectors.
    pub line_color: Color,
    pub line_width: f64,
}

impl Default for DiagramSettings {
    fn default() -> Self {
        DiagramSettings {
            route: RouteStyle::Orthogonal,
            jump: JumpStyle::Arc,
            jump_size: default_jump_size(),
            margin: default_margin(),
            corner_radius: 4.0,
            end_marker: Marker { kind: MarkerKind::Triangle, size: 1.0 },
            line_color: Color::rgb(0.2, 0.22, 0.26),
            line_width: 2.0,
        }
    }
}

/// An arrowhead or other mark at the end of an open line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub kind: MarkerKind,
    /// Scale: 1 = the standard size for the line's width.
    #[serde(default = "one")]
    pub size: f64,
}

fn one() -> f64 {
    1.0
}

impl Marker {
    pub fn is_none(&self) -> bool {
        self.kind == MarkerKind::None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    #[default]
    None,
    /// Filled triangle (SVG/CSS-style arrowhead).
    Triangle,
    /// Two strokes, like "→".
    Open,
    /// Filled arrow with a notched back.
    Arrow,
    Diamond,
    OpenDiamond,
    Circle,
    OpenCircle,
    /// A short bar across the line.
    Bar,
    /// Crow's foot (many, from ER diagrams).
    CrowsFoot,
}

impl MarkerKind {
    pub const ALL: [MarkerKind; 10] = [
        MarkerKind::None,
        MarkerKind::Triangle,
        MarkerKind::Open,
        MarkerKind::Arrow,
        MarkerKind::Diamond,
        MarkerKind::OpenDiamond,
        MarkerKind::Circle,
        MarkerKind::OpenCircle,
        MarkerKind::Bar,
        MarkerKind::CrowsFoot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MarkerKind::None => "None",
            MarkerKind::Triangle => "Triangle",
            MarkerKind::Open => "Open arrow",
            MarkerKind::Arrow => "Arrow",
            MarkerKind::Diamond => "Diamond",
            MarkerKind::OpenDiamond => "Open diamond",
            MarkerKind::Circle => "Circle",
            MarkerKind::OpenCircle => "Open circle",
            MarkerKind::Bar => "Bar",
            MarkerKind::CrowsFoot => "Crow's foot",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_round_trips_and_old_minimal_json_loads() {
        let c = Connector::new(
            ConnectorEnd::glued(Id(3), PortRef::Port(2), Vec2::new(1.0, 2.0)),
            ConnectorEnd::free(Vec2::new(5.0, 6.0)),
        );
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Connector>(&json).unwrap(), c);
        let min: Connector =
            serde_json::from_str(r#"{"from":{"point":{"x":0,"y":0}},"to":{"point":{"x":1,"y":1}}}"#).unwrap();
        assert!(min.avoid);
        assert_eq!(min.margin, 16.0);
        assert_eq!(min.label_at, 0.5);
    }
}
