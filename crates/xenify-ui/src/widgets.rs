//! Small shared UI helpers.

use egui::Color32;
use xenify_doc::{PropPath, ShapeField};

pub const ACCENT: Color32 = Color32::from_rgb(64, 156, 255);
pub const KEYFRAME: Color32 = Color32::from_rgb(255, 200, 80);
pub const RECORD: Color32 = Color32::from_rgb(230, 60, 60);

/// Human-readable label for a property.
pub fn prop_label(path: PropPath) -> String {
    match path {
        PropPath::Anchor => "Transform Origin".into(),
        PropPath::Position => "Position".into(),
        PropPath::Scale => "Scale".into(),
        PropPath::Rotation => "Rotation".into(),
        PropPath::Opacity => "Opacity".into(),
        PropPath::Skew => "Skew".into(),
        PropPath::SkewAxis => "Skew Direction".into(),
        PropPath::FontSize => "Font Size".into(),
        PropPath::TextColor => "Text Colour".into(),
        PropPath::Distort => "Perspective".into(),
        PropPath::Emitter(f) => f.label().into(),
        PropPath::Liquid(f) => f.label().into(),
        PropPath::Volume => "Volume".into(),
        PropPath::Camera(f) => match f {
            xenify_doc::CameraField::Zoom => "Zoom",
            xenify_doc::CameraField::ShakeAmount => "Shake Amount",
            xenify_doc::CameraField::ShakeRotation => "Shake Turn",
        }
        .into(),
        PropPath::Solid(f) => match f {
            xenify_doc::SolidField::Size => "Size",
            xenify_doc::SolidField::Depth => "Depth",
            xenify_doc::SolidField::RotX => "X Rotation",
            xenify_doc::SolidField::RotY => "Y Rotation",
            xenify_doc::SolidField::RotZ => "Z Rotation",
            xenify_doc::SolidField::Color => "Colour",
        }
        .into(),
        PropPath::Shape { field, .. } => match field {
            ShapeField::Position => "Offset",
            ShapeField::Size => "Size",
            ShapeField::CornerRadius => "Roundness",
            ShapeField::Path => "Path",
            ShapeField::FillColor => "Fill",
            ShapeField::StrokeColor => "Stroke",
            ShapeField::StrokeWidth => "Stroke Width",
            ShapeField::TrimStart => "Trim Start",
            ShapeField::TrimEnd => "Trim End",
            ShapeField::TrimOffset => "Trim Offset",
            ShapeField::GradientStart => "Gradient Start",
            ShapeField::GradientEnd => "Gradient End",
            ShapeField::GradientColors => "Gradient Colours",
            ShapeField::DashOffset => "Dash Offset",
            ShapeField::StrokeGradientStart => "Stroke Gradient Start",
            ShapeField::StrokeGradientEnd => "Stroke Gradient End",
            ShapeField::StrokeGradientColors => "Stroke Gradient Colours",
            ShapeField::Count => "Count",
            ShapeField::Inner => "Inner Radius",
            ShapeField::Hole => "Hole",
        }
        .into(),
        PropPath::SolidPart { index, field } => format!(
            "Part {} {}",
            index + 1,
            match field {
                xenify_doc::PartField::Offset => "Offset",
                xenify_doc::PartField::Depth => "Depth",
                xenify_doc::PartField::RotX => "X Rotation",
                xenify_doc::PartField::RotY => "Y Rotation",
                xenify_doc::PartField::RotZ => "Z Rotation",
            }
        ),
        PropPath::Effect { index, field } => format!(
            "Effect {} {}",
            index + 1,
            match field {
                xenify_doc::EffectField::Color => "Colour",
                xenify_doc::EffectField::Offset => "Offset",
                xenify_doc::EffectField::Radius => "Size",
                xenify_doc::EffectField::Brightness => "Brightness",
                xenify_doc::EffectField::Contrast => "Contrast",
                xenify_doc::EffectField::Saturation => "Saturation",
                xenify_doc::EffectField::Hue => "Hue",
            }
        ),
        PropPath::Modifier { index, field } => format!("Modifier {} {}", index + 1, field.label()),
        PropPath::Mask { index, field } => format!(
            "Mask {} {}",
            index + 1,
            match field {
                xenify_doc::MaskField::Path => "Path",
                xenify_doc::MaskField::Feather => "Feather",
                xenify_doc::MaskField::Opacity => "Opacity",
            }
        ),
    }
}

/// A property's name without its "Mask 2 " / "Effect 1 " / "Modifier 3 "
/// prefix, for rows already under that mask's or effect's heading.
pub fn prop_field_label(path: PropPath) -> String {
    let full = prop_label(path);
    match path {
        PropPath::Mask { .. } | PropPath::Effect { .. } | PropPath::Modifier { .. } | PropPath::SolidPart { .. } => {
            full.splitn(3, ' ').nth(2).map_or(full.clone(), str::to_string)
        }
        _ => full,
    }
}

/// Whether a property is a distance (shown in the document's units).
pub fn is_length(path: PropPath) -> bool {
    use xenify_doc::{EffectField, MaskField, SolidField};
    matches!(
        path,
        PropPath::Position
            | PropPath::Anchor
            | PropPath::Shape {
                field: ShapeField::Position
                    | ShapeField::Size
                    | ShapeField::CornerRadius
                    | ShapeField::StrokeWidth
                    | ShapeField::DashOffset,
                ..
            }
            | PropPath::Solid(SolidField::Size | SolidField::Depth)
            | PropPath::SolidPart { field: xenify_doc::PartField::Offset | xenify_doc::PartField::Depth, .. }
            | PropPath::Mask { field: MaskField::Feather, .. }
            | PropPath::Effect { field: EffectField::Radius | EffectField::Offset, .. }
            | PropPath::Modifier {
                field: xenify_doc::ModifierField::Offset
                    | xenify_doc::ModifierField::Origin
                    | xenify_doc::ModifierField::Amount
                    | xenify_doc::ModifierField::Tolerance,
                ..
            }
    )
}

/// [`prop_display`] in the document's units: distances in mm, cm, in or pt
/// when the composition uses them.
pub fn prop_display_in(path: PropPath, units: xenify_doc::Units) -> (f64, &'static str, f64) {
    if is_length(path) {
        if units.unit != xenify_doc::Unit::Px {
            let per_px = units.from_px(1.0);
            return (per_px, units.unit.suffix_spaced(), per_px);
        }
        let (mul, suffix, speed) = prop_display(path);
        return (mul, if suffix.is_empty() { " px" } else { suffix }, speed);
    }
    prop_display(path)
}

/// The range a property's shown number may take (in shown units), if limited.
pub fn prop_range(path: PropPath) -> Option<std::ops::RangeInclusive<f64>> {
    use xenify_doc::{MaskField, ModifierField};
    match path {
        PropPath::Opacity
        | PropPath::Mask { field: MaskField::Opacity, .. }
        | PropPath::Modifier { field: ModifierField::EndOpacity, .. }
        | PropPath::Shape { field: ShapeField::TrimStart | ShapeField::TrimEnd, .. } => Some(0.0..=100.0),
        PropPath::Volume => Some(0.0..=400.0),
        PropPath::Shape { field: ShapeField::Count, .. } => Some(3.0..=200.0),
        PropPath::Shape { field: ShapeField::Inner, .. } => Some(2.0..=95.0),
        PropPath::Shape { field: ShapeField::Hole, .. } => Some(0.0..=90.0),
        PropPath::FontSize => Some(0.0..=f64::MAX),
        PropPath::Shape { field: ShapeField::StrokeWidth | ShapeField::CornerRadius, .. } => Some(0.0..=f64::MAX),
        _ => None,
    }
}

/// How a property's numbers are shown: `(display multiplier, suffix, drag speed)`.
/// Opacity and scale are stored as factors but shown as percentages.
pub fn prop_display(path: PropPath) -> (f64, &'static str, f64) {
    match path {
        PropPath::Opacity | PropPath::Scale | PropPath::Volume => (100.0, "%", 0.5),
        PropPath::Mask { field: xenify_doc::MaskField::Opacity, .. } => (100.0, "%", 0.5),
        PropPath::Modifier { field, .. } => match field {
            xenify_doc::ModifierField::Scale | xenify_doc::ModifierField::EndOpacity => (100.0, "%", 0.5),
            xenify_doc::ModifierField::Rotation => (1.0, "°", 0.5),
            xenify_doc::ModifierField::Strength => (100.0, "%", 0.5),
            xenify_doc::ModifierField::Start | xenify_doc::ModifierField::End => (100.0, "%", 0.5),
            xenify_doc::ModifierField::Count
            | xenify_doc::ModifierField::Ridges
            | xenify_doc::ModifierField::Levels
            | xenify_doc::ModifierField::Seed => (1.0, "", 0.05),
            xenify_doc::ModifierField::Detail => (1.0, "", 0.1),
            xenify_doc::ModifierField::Offset | xenify_doc::ModifierField::Origin => (1.0, " px", 1.0),
            xenify_doc::ModifierField::Amount | xenify_doc::ModifierField::Tolerance => (1.0, " px", 0.2),
            xenify_doc::ModifierField::Force => (1.0, " px/s", 1.0),
            xenify_doc::ModifierField::Gravity => (1.0, " px/s²", 2.0),
            xenify_doc::ModifierField::Spin => (1.0, "°/s", 1.0),
            xenify_doc::ModifierField::Progress => (1.0, " s", 0.01),
        },
        PropPath::Effect {
            field:
                xenify_doc::EffectField::Brightness
                | xenify_doc::EffectField::Contrast
                | xenify_doc::EffectField::Saturation,
            ..
        } => (100.0, "%", 0.5),
        PropPath::Effect { field: xenify_doc::EffectField::Hue, .. } => (1.0, "°", 0.5),
        PropPath::Effect { field: xenify_doc::EffectField::Radius | xenify_doc::EffectField::Offset, .. } => {
            (1.0, " px", 0.2)
        }
        PropPath::Shape { field: ShapeField::TrimStart | ShapeField::TrimEnd | ShapeField::TrimOffset, .. } => {
            (100.0, "%", 0.5)
        }
        PropPath::Shape { field: ShapeField::Inner | ShapeField::Hole, .. } => (100.0, "%", 0.5),
        PropPath::Shape { field: ShapeField::Count, .. } => (1.0, "", 0.05),
        PropPath::SolidPart {
            field: xenify_doc::PartField::RotX | xenify_doc::PartField::RotY | xenify_doc::PartField::RotZ,
            ..
        } => (1.0, "°", 0.5),
        PropPath::Rotation
        | PropPath::Skew
        | PropPath::SkewAxis
        | PropPath::Solid(xenify_doc::SolidField::RotX | xenify_doc::SolidField::RotY | xenify_doc::SolidField::RotZ) => {
            (1.0, "°", 0.5)
        }
        _ => (1.0, "", 1.0),
    }
}

/// Formats seconds as `SS:FF` (seconds and frames).
/// A time as seconds and frames: "2s 05f" (unambiguous, unlike "02:05").
pub fn timecode(t: f64, fps: f64) -> String {
    let frame = (t * fps).round() as i64;
    let fps_i = fps.round().max(1.0) as i64;
    format!("{}s {:02}f", frame / fps_i, frame % fps_i)
}

/// A ruler label: "2s" on whole seconds, else the frame within the second ("15f").
pub fn ruler_label(t: f64, fps: f64) -> String {
    let frame = (t * fps).round() as i64;
    let fps_i = fps.round().max(1.0) as i64;
    if frame % fps_i == 0 { format!("{}s", frame / fps_i) } else { format!("{}f", frame % fps_i) }
}

/// Icons drawn with the painter, because egui's bundled fonts lack many
/// symbol glyphs (◆, ◯, □, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Select,
    Rect,
    Ellipse,
    Pen,
    Text,
    Node,
    FreeTransform,
    Perspective,
    Line,
    Polygon,
    Pencil,
    Brush,
    Sculpt,
    Eyedropper,
    Gradient,
    Crop,
    Mask,
    Knife,
    Slice,
    /// Two boxes joined by an elbow line.
    Connector,
    Hand,
    Zoom,
    Cube,
    /// Keyframe diamond; `true` = filled.
    Diamond(bool),
    PrevKey,
    NextKey,
}

pub fn paint_diamond(painter: &egui::Painter, c: egui::Pos2, r: f32, fill: Color32, stroke: egui::Stroke) {
    use egui::vec2;
    painter.add(egui::Shape::convex_polygon(
        vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)],
        fill,
        stroke,
    ));
}

pub fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: Color32) {
    use egui::{Pos2, Stroke, StrokeKind, pos2, vec2};
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new(1.6, color);
    let tri = |at: Pos2, dir: f32| {
        egui::Shape::convex_polygon(
            vec![
                at + vec2(-0.35 * s * dir, -0.45 * s),
                at + vec2(0.35 * s * dir, 0.0),
                at + vec2(-0.35 * s * dir, 0.45 * s),
            ],
            color,
            Stroke::NONE,
        )
    };
    match icon {
        Icon::Select => {
            let o = c + vec2(-0.45 * s, -0.6 * s);
            let pts = [
                o,
                o + vec2(0.0, 1.15 * s),
                o + vec2(0.3 * s, 0.85 * s),
                o + vec2(0.55 * s, 1.3 * s),
                o + vec2(0.75 * s, 1.2 * s),
                o + vec2(0.52 * s, 0.78 * s),
                o + vec2(0.9 * s, 0.75 * s),
            ];
            painter.add(egui::Shape::convex_polygon(pts.to_vec(), color, Stroke::NONE));
        }
        Icon::Node => {
            // A curve with its points and a handle: editing points.
            let p0 = c + vec2(-0.7 * s, 0.45 * s);
            let p1 = c + vec2(0.7 * s, -0.35 * s);
            let curve: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let q = c + vec2(-0.1 * s, -0.9 * s);
                    let a = p0 + (q - p0) * t;
                    let b = q + (p1 - q) * t;
                    a + (b - a) * t
                })
                .collect();
            painter.add(egui::Shape::line(curve, Stroke::new(1.3, color)));
            let h = c + vec2(-0.1 * s, -0.9 * s);
            painter.line_segment([p0, h], Stroke::new(0.8, color.gamma_multiply(0.7)));
            painter.circle_filled(h, 0.13 * s, color);
            for p in [p0, p1] {
                painter.rect_filled(egui::Rect::from_center_size(p, vec2(0.34 * s, 0.34 * s)), 0.0, color);
            }
        }
        Icon::Rect => {
            painter.rect_stroke(egui::Rect::from_center_size(c, vec2(1.3 * s, s)), 1.0, stroke, StrokeKind::Middle);
        }
        Icon::Ellipse => {
            painter.circle_stroke(c, 0.62 * s, stroke);
        }
        Icon::Pen => {
            let tip = c + vec2(-0.55 * s, 0.6 * s);
            painter.add(egui::Shape::convex_polygon(
                vec![tip, c + vec2(-0.2 * s, -0.05 * s), c + vec2(0.05 * s, 0.25 * s)],
                color,
                Stroke::NONE,
            ));
            painter.line_segment([c + vec2(-0.1 * s, 0.05 * s), c + vec2(0.55 * s, -0.6 * s)], Stroke::new(3.0, color));
        }
        Icon::Text => {
            painter.text(c, egui::Align2::CENTER_CENTER, "T", egui::FontId::proportional(1.5 * s), color);
        }
        Icon::FreeTransform => {
            // A box with corner handles and a curved rotate arrow.
            let r = egui::Rect::from_center_size(c + vec2(-0.1 * s, 0.1 * s), vec2(1.1 * s, 1.1 * s));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Middle);
            for p in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
                painter.rect_filled(egui::Rect::from_center_size(p, vec2(0.28 * s, 0.28 * s)), 0.0, color);
            }
            let arc: Vec<Pos2> = (0..=8)
                .map(|i| {
                    let a = -1.4 + i as f32 * 0.16;
                    r.right_top() + vec2(a.cos(), a.sin()) * 0.45 * s + vec2(0.0, 0.0)
                })
                .collect();
            painter.add(egui::Shape::line(arc, Stroke::new(1.2, color)));
        }
        Icon::Perspective => {
            let pts = vec![
                c + vec2(-0.35 * s, -0.7 * s),
                c + vec2(0.35 * s, -0.7 * s),
                c + vec2(0.8 * s, 0.6 * s),
                c + vec2(-0.8 * s, 0.6 * s),
            ];
            painter.add(egui::Shape::closed_line(pts.clone(), stroke));
            for p in pts {
                painter.rect_filled(egui::Rect::from_center_size(p, vec2(0.26 * s, 0.26 * s)), 0.0, color);
            }
        }
        Icon::Line => {
            painter.line_segment([c + vec2(-0.7 * s, 0.7 * s), c + vec2(0.7 * s, -0.7 * s)], Stroke::new(2.0, color));
        }
        Icon::Polygon => {
            let pts = (0..5)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / 5.0;
                    c + vec2(a.cos(), a.sin()) * 0.72 * s
                })
                .collect();
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::Pencil => {
            let (a, b) = (c + vec2(-0.6 * s, 0.6 * s), c + vec2(0.55 * s, -0.55 * s));
            painter.line_segment([a + vec2(0.18 * s, -0.18 * s), b], Stroke::new(4.0, color));
            painter.add(egui::Shape::convex_polygon(
                vec![a, a + vec2(0.1 * s, -0.32 * s), a + vec2(0.32 * s, -0.1 * s)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Brush => {
            painter
                .line_segment([c + vec2(-0.05 * s, 0.05 * s), c + vec2(0.65 * s, -0.65 * s)], Stroke::new(2.0, color));
            painter.circle_filled(c + vec2(-0.3 * s, 0.3 * s), 0.3 * s, color);
            painter.add(egui::Shape::convex_polygon(
                vec![c + vec2(-0.55 * s, 0.45 * s), c + vec2(-0.75 * s, 0.8 * s), c + vec2(-0.35 * s, 0.6 * s)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Sculpt => {
            // A brush circle with a dented curve.
            painter.circle_stroke(c + vec2(0.15 * s, -0.2 * s), 0.5 * s, Stroke::new(1.0, color));
            let curve: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let x = -0.8 + 1.6 * t;
                    let bump = (-(x - 0.15).powi(2) * 8.0).exp() * 0.55;
                    c + vec2(x * s, (0.55 - bump) * s)
                })
                .collect();
            painter.add(egui::Shape::line(curve, stroke));
        }
        Icon::Eyedropper => {
            let tip = c + vec2(-0.7 * s, 0.7 * s);
            painter.line_segment([tip, c + vec2(0.15 * s, -0.15 * s)], Stroke::new(2.2, color));
            painter.circle_filled(c + vec2(0.4 * s, -0.4 * s), 0.3 * s, color);
            painter.line_segment([c + vec2(0.0, -0.45 * s), c + vec2(0.45 * s, 0.0)], Stroke::new(2.0, color));
        }
        Icon::Crop => {
            let k = 0.55 * s;
            painter.add(egui::Shape::line(vec![c + vec2(-k, -1.3 * k), c + vec2(-k, k), c + vec2(1.3 * k, k)], stroke));
            painter.add(egui::Shape::line(vec![c + vec2(-1.3 * k, -k), c + vec2(k, -k), c + vec2(k, 1.3 * k)], stroke));
        }
        Icon::Mask => {
            // A photo corner cut by a dashed oval.
            let r = egui::Rect::from_center_size(c, vec2(1.5 * s, 1.2 * s));
            painter.rect_filled(r, 1.0, color.gamma_multiply(0.35));
            let n = 14;
            for k in (0..n).step_by(2) {
                let a0 = k as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = (k + 1) as f32 / n as f32 * std::f32::consts::TAU;
                let p = |a: f32| c + vec2(a.cos() * 0.62 * s, a.sin() * 0.48 * s);
                painter.line_segment([p(a0), p(a1)], stroke);
            }
        }
        Icon::Gradient => {
            // A square shading from light to dark, with the tool's line and two ends.
            let r = egui::Rect::from_center_size(c, vec2(1.4 * s, 1.4 * s));
            let mut mesh = egui::Mesh::default();
            let (light, dark) = (color, color.gamma_multiply(0.15));
            mesh.colored_vertex(r.left_top(), light);
            mesh.colored_vertex(r.right_top(), dark);
            mesh.colored_vertex(r.right_bottom(), dark);
            mesh.colored_vertex(r.left_bottom(), light);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            painter.rect_stroke(r, 1.0, Stroke::new(1.0, color), StrokeKind::Middle);
        }
        Icon::Knife => {
            // A blade: a long thin triangle on a handle.
            let tip = c + vec2(0.75 * s, -0.75 * s);
            let heel = c + vec2(-0.2 * s, 0.2 * s);
            let back = c + vec2(-0.05 * s, 0.35 * s);
            painter.add(egui::Shape::convex_polygon(
                vec![tip, back, heel + vec2(-0.1 * s, -0.1 * s)],
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [c + vec2(-0.2 * s, 0.25 * s), c + vec2(-0.7 * s, 0.75 * s)],
                Stroke::new(0.22 * s, color),
            );
        }
        Icon::Connector => {
            let a = egui::Rect::from_center_size(c + vec2(-0.5 * s, -0.5 * s), vec2(0.6 * s, 0.45 * s));
            let b = egui::Rect::from_center_size(c + vec2(0.5 * s, 0.5 * s), vec2(0.6 * s, 0.45 * s));
            painter.rect_stroke(a, 1.0, stroke, StrokeKind::Middle);
            painter.rect_stroke(b, 1.0, stroke, StrokeKind::Middle);
            let elbow = vec![a.center_bottom(), pos2(a.center().x, c.y), pos2(b.left(), c.y)];
            painter.add(egui::Shape::line(elbow, stroke));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    b.left_center(),
                    b.left_center() + vec2(-0.25 * s, -0.15 * s),
                    b.left_center() + vec2(-0.25 * s, 0.15 * s),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Slice => {
            let r = egui::Rect::from_center_size(c, vec2(1.3 * s, 1.1 * s));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Middle);
            painter.line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, color));
            painter.line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, color));
            painter.line_segment(
                [c + vec2(0.1 * s, 0.1 * s), r.right_bottom() + vec2(0.1 * s, 0.1 * s)],
                Stroke::new(2.2, color),
            );
        }
        Icon::Hand => {
            // Four-way arrows: pan the view.
            let st = Stroke::new(1.4, color);
            let r = 0.75 * s;
            painter.line_segment([c - vec2(r, 0.0), c + vec2(r, 0.0)], st);
            painter.line_segment([c - vec2(0.0, r), c + vec2(0.0, r)], st);
            for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                let tip = c + d * r;
                let side = vec2(-d.y, d.x) * 0.22 * s;
                painter.add(egui::Shape::convex_polygon(
                    vec![tip + d * 0.1 * s, tip - d * 0.25 * s + side, tip - d * 0.25 * s - side],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Zoom => {
            painter.circle_stroke(c + vec2(-0.15 * s, -0.15 * s), 0.45 * s, Stroke::new(1.8, color));
            painter.line_segment([c + vec2(0.18 * s, 0.18 * s), c + vec2(0.7 * s, 0.7 * s)], Stroke::new(2.6, color));
        }
        Icon::Cube => {
            let (f, b) = (0.5 * s, 0.3 * s);
            let front = egui::Rect::from_center_size(c + vec2(-0.12 * s, 0.12 * s), vec2(2.0 * f, 2.0 * f));
            let off = vec2(b, -b);
            painter.rect_stroke(front, 0.0, stroke, StrokeKind::Middle);
            painter.add(egui::Shape::line(
                vec![
                    front.left_top(),
                    front.left_top() + off,
                    front.right_top() + off,
                    front.right_bottom() + off,
                    front.right_bottom(),
                ],
                stroke,
            ));
            painter.line_segment([front.right_top(), front.right_top() + off], stroke);
        }
        Icon::Diamond(filled) => {
            paint_diamond(painter, c, 0.55 * s, if filled { color } else { Color32::TRANSPARENT }, stroke);
        }
        Icon::PrevKey => {
            painter.add(tri(pos2(c.x - 0.45 * s, c.y), -1.0));
            paint_diamond(painter, pos2(c.x + 0.4 * s, c.y), 0.4 * s, KEYFRAME, Stroke::NONE);
        }
        Icon::NextKey => {
            paint_diamond(painter, pos2(c.x - 0.4 * s, c.y), 0.4 * s, KEYFRAME, Stroke::NONE);
            painter.add(tri(pos2(c.x + 0.45 * s, c.y), 1.0));
        }
    }
}

/// A button showing a painted [`Icon`].
impl Icon {
    /// What the icon stands for, for screen readers.
    pub fn name(self) -> &'static str {
        match self {
            Icon::Select => "Select",
            Icon::Rect => "Rectangle",
            Icon::Ellipse => "Ellipse",
            Icon::Pen => "Pen",
            Icon::Text => "Text",
            Icon::Node => "Edit points",
            Icon::FreeTransform => "Free Transform",
            Icon::Perspective => "Perspective",
            Icon::Line => "Line",
            Icon::Polygon => "Polygon",
            Icon::Pencil => "Pencil",
            Icon::Brush => "Brush",
            Icon::Sculpt => "Sculpt",
            Icon::Eyedropper => "Eyedropper",
            Icon::Crop => "Crop",
            Icon::Mask => "Mask",
            Icon::Knife => "Knife",
            Icon::Gradient => "Gradient",
            Icon::Slice => "Slice",
            Icon::Connector => "Connector",
            Icon::Hand => "Hand",
            Icon::Zoom => "Zoom",
            Icon::Cube => "3D",
            Icon::Diamond(true) => "Remove keyframe",
            Icon::Diamond(false) => "Add keyframe",
            Icon::PrevKey => "Previous keyframe",
            Icon::NextKey => "Next keyframe",
        }
    }
}

pub fn icon_button(ui: &mut egui::Ui, icon: Icon, size: f32, selected: bool, frame: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, icon.name()));
    // Keyboard users see where Tab has got to.
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(1.0), 4.0, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if frame || selected || response.hovered() {
            ui.painter().rect(rect, 4.0, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        }
        let color = match icon {
            Icon::Diamond(_) => KEYFRAME,
            _ => visuals.fg_stroke.color,
        };
        paint_icon(ui.painter(), rect.shrink(size * 0.12), icon, color);
    }
    response
}

/// A dialog that reads as one: the window behind is dimmed, and the dialog
/// has an accent border and a title bar with a close button. `width` is the
/// body's width. Returns what `content` returned and whether the dialog asks
/// to close (its × button, Esc, or a click outside it).
pub fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let frame = egui::Frame::window(&ctx.global_style())
        .inner_margin(0.0)
        .stroke(egui::Stroke::new(1.5, ACCENT.gamma_multiply(0.85)))
        .corner_radius(8.0);
    let mut closed = false;
    let r = egui::Modal::new(egui::Id::new(id)).backdrop_color(Color32::from_black_alpha(170)).frame(frame).show(
        ctx,
        |ui| {
            let bar = egui::Frame::new()
                .fill(ACCENT.gamma_multiply(0.25))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .corner_radius(egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 });
            bar.show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).strong().size(16.0).color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let x = egui::Button::new(egui::RichText::new("×").size(18.0)).frame(false);
                        if ui.add(x).on_hover_text("Close (Esc)").clicked() {
                            closed = true;
                        }
                    });
                });
            });
            egui::Frame::new()
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    // A fixed width: without a maximum, wrapping text and "fill
                    // width" fields can feed back into the size and grow it every frame.
                    ui.set_min_width(width);
                    ui.set_max_width(width);
                    content(ui)
                })
                .inner
        },
    );
    let close = closed || r.should_close();
    (r.inner, close)
}

/// The dialog's main button (Save, Create, Export…), in the accent colour.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).strong().color(Color32::WHITE)).fill(ACCENT)
}

/// Strokes `path` (already in screen coordinates) as outlines.
pub fn paint_outline(painter: &egui::Painter, path: &kurbo::BezPath, stroke: egui::Stroke) {
    let mut lines: Vec<(Vec<egui::Pos2>, bool)> = Vec::new();
    let pos = |p: kurbo::Point| egui::pos2(p.x as f32, p.y as f32);
    kurbo::flatten(path, 0.25, |el| match el {
        kurbo::PathEl::MoveTo(p) => lines.push((vec![pos(p)], false)),
        kurbo::PathEl::LineTo(p) => {
            if let Some(l) = lines.last_mut() {
                l.0.push(pos(p));
            }
        }
        kurbo::PathEl::ClosePath => {
            if let Some(l) = lines.last_mut() {
                l.1 = true;
            }
        }
        _ => {}
    });
    for (points, closed) in lines {
        painter.add(if closed { egui::Shape::closed_line(points, stroke) } else { egui::Shape::line(points, stroke) });
    }
}

/// A selectable menu row with a picture of a shape before its label: `shape`
/// builds the outline to fit the box it's given (screen coordinates).
pub fn shape_row(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
    shape: impl FnOnce(kurbo::Rect) -> kurbo::BezPath,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let size = ui.spacing().interact_size.y;
        let (rect, pic) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
        let text: egui::WidgetText = text.into();
        let name = text.text().to_owned();
        pic.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name));
        let label = ui.selectable_label(selected, text);
        let resp = pic | label;
        let color = ui.style().interact_selectable(&resp, selected).fg_stroke.color;
        let r = rect.shrink(3.0);
        let path = shape(kurbo::Rect::new(r.min.x as f64, r.min.y as f64, r.max.x as f64, r.max.y as f64));
        paint_outline(ui.painter(), &path, egui::Stroke::new(1.3, color));
        resp
    })
    .inner
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use xenify_doc::{Unit, Units};

    #[test]
    fn distances_show_in_the_documents_units() {
        let mm = Units { unit: Unit::Mm, dpi: 96.0 };
        let (mul, suffix, _) = prop_display_in(PropPath::Position, mm);
        assert!((96.0 * mul - 25.4).abs() < 1e-9, "96 px is an inch");
        assert_eq!(suffix, " mm");
        // Angles and percentages don't change; pixels say "px".
        assert_eq!(prop_display_in(PropPath::Rotation, mm), prop_display(PropPath::Rotation));
        assert_eq!(prop_display_in(PropPath::Scale, mm), prop_display(PropPath::Scale));
        assert_eq!(prop_display_in(PropPath::Position, Units::default()), (1.0, " px", 1.0));
    }
}
