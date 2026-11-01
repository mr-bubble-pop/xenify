//! Slide 3: Vector: the pen draws a curve (nodes and handles showing), shapes
//! merge with a boolean union, a gradient sweeps, a radial array blooms into
//! a flower beside a mirrored squiggle, and pens paint strokes. A click
//! between each.

use crate::kit::*;
use kurbo::BezPath;
use xenify_doc::{
    Easing, GradientKind, GradientPaint, GradientStop, Id, LayerKind, Modifier, ModifierKind, PenKind, PenStyle,
    Project, TransitionKind,
};

const STAGE: (f64, f64) = (1100.0, 600.0);

fn caption(d: &mut Deck, t: &str, t0: f64, t1: f64) {
    let mut c = text("Caption", t, 40.0, MUTED, v(STAGE.0, 960.0));
    animate(&mut c, "Fade Up by Word", t0 + 0.2);
    c.opacity = keys(&[(t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    live(&mut c, 0.0, Some(t1 + 0.35));
    d.add(c);
}

/// Shows a layer from `t0`, fading out at `t1`.
fn beat(l: &mut xenify_doc::Layer, t0: f64, t1: f64) {
    live(l, t0, Some(t1 + 0.35));
    let o = l.opacity.clone();
    if matches!(o, xenify_doc::Property::Static(_)) {
        l.opacity = keys(&[(t0, 0.0), (t0 + 0.25, 1.0), (t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
    }
}

pub fn build(d: &mut Deck) {
    let pauses = [2.5, 4.9, 7.1, 9.5, 11.9];
    d.slide("Vector", 12.4, &pauses, tr(TransitionKind::Cover, 0.8), "Pen, booleans, gradients, arrays, pens.");
    let mut title = text_left("Title", "Vector", 120.0, INK, v(120.0, 200.0));
    animate(&mut title, "Fade Up by Word", 0.0);
    d.add(title);
    let s = v(STAGE.0, STAGE.1);

    // A. The pen: a curve with its nodes and handles.
    let pts = [(-420.0, 120.0), (-140.0, -180.0), (160.0, 160.0), (420.0, -100.0)];
    let handles = [
        ((-420.0, 120.0), (-300.0, -120.0)),
        ((-140.0, -180.0), (-20.0, -230.0)),
        ((160.0, 160.0), (40.0, 210.0)),
        ((420.0, -100.0), (330.0, 60.0)),
    ];
    let mut curve = BezPath::new();
    curve.move_to(pts[0]);
    curve.curve_to((-300.0, -120.0), (-260.0, -130.0), pts[1]);
    curve.curve_to((-20.0, -230.0), (40.0, 210.0), pts[2]);
    curve.curve_to((280.0, 110.0), (330.0, 60.0), pts[3]);
    let mut stroke_l = path("Pen curve", curve, None, Some((INK, 8.0)), s);
    draw_on(&mut stroke_l, 0.3, 1.5);
    beat(&mut stroke_l, 0.0, 2.6);
    d.add(stroke_l);
    for (k, (a, h)) in handles.iter().enumerate() {
        let t = 0.3 + k as f64 * 0.37;
        let mut line = BezPath::new();
        line.move_to(*a);
        line.line_to(*h);
        let mut hl = path("Handle", line, None, Some((VIOLET, 3.0)), s);
        beat(&mut hl, t, 2.6);
        d.add(hl);
        let mut knob = ellipse("Handle knob", v(16.0, 16.0), VIOLET, s + v(h.0, h.1));
        pop(&mut knob, t);
        beat(&mut knob, t, 2.6);
        d.add(knob);
        let mut node = rect("Node", v(20.0, 20.0), 3.0, PINK, s + v(a.0, a.1));
        pop(&mut node, t);
        beat(&mut node, t, 2.6);
        d.add(node);
    }
    caption(d, "The pen: curves you shape by their nodes and handles", 0.0, 2.6);

    // B. Boolean union: three shapes slide together and become one.
    let parts = [
        (ellipse("Circle A", v(280.0, 280.0), PINK, s + v(-120.0, -20.0)), v(-300.0, 0.0)),
        (ellipse("Circle B", v(280.0, 280.0), VIOLET, s + v(120.0, -20.0)), v(300.0, 0.0)),
        (rect("Bar", v(420.0, 130.0), 20.0, CYAN, s + v(0.0, 110.0)), v(0.0, 260.0)),
    ];
    let merge_at = 3.9;
    let union = union_of(parts.iter().map(|(l, _)| l.clone()).collect());
    for (mut l, from) in parts {
        slide_in(&mut l, 2.9, from, 0.8);
        l.opacity = keys(&[(2.9, 0.0), (3.2, 0.7)], Easing::Linear);
        live(&mut l, 2.9, Some(merge_at));
        d.add(l);
    }
    let mut u = union;
    if let LayerKind::Shape { shapes } = &mut u.kind {
        for sh in shapes.iter_mut() {
            sh.fill = Some(gradient_fill(560.0));
        }
    }
    u.meta.name = "Union".into();
    u.transform.scale = keys(
        &[(merge_at, xenify_doc::Vec2::new(1.08, 1.08)), (merge_at + 0.5, xenify_doc::Vec2::new(1.0, 1.0))],
        SPRING,
    );
    beat(&mut u, merge_at, 5.0);
    d.add(u);
    caption(d, "Booleans: unite, subtract, intersect, exclude", 2.9, 5.0);

    // C. A gradient sweeps across a card.
    let stops = |k: f64| {
        vec![
            GradientStop { offset: 0.0, color: PINK },
            GradientStop { offset: (0.2 + k).min(0.95), color: VIOLET },
            GradientStop { offset: 1.0, color: CYAN },
        ]
    };
    let mut card = rect("Gradient card", v(760.0, 320.0), 36.0, PINK, s);
    if let LayerKind::Shape { shapes } = &mut card.kind {
        let f = shapes[0].fill.get_or_insert_with(|| xenify_doc::Fill::solid(PINK));
        f.gradient = Some(GradientPaint {
            kind: GradientKind::Linear,
            start: keys(&[(5.2, v(-380.0, -160.0)), (6.8, v(-380.0, 160.0))], SMOOTH),
            end: keys(&[(5.2, v(380.0, 160.0)), (6.8, v(380.0, -160.0))], SMOOTH),
            stops: keys(&[(5.2, stops(0.0)), (6.8, stops(0.6))], SMOOTH),
        });
    }
    pop(&mut card, 5.1);
    beat(&mut card, 5.1, 7.2);
    d.add(card);
    caption(d, "Gradients, with every stop and angle animatable", 5.1, 7.2);

    // D. A radial array blooms into a flower; a mirror doubles a squiggle.
    let mut petal = ellipse("Petal", v(70.0, 210.0), PINK, s + v(-230.0, 0.0));
    if let LayerKind::Shape { shapes } = &mut petal.kind {
        if let xenify_doc::Geometry::Ellipse { position, .. } = &mut shapes[0].geometry {
            *position = v(0.0, -115.0).into();
        }
        shapes[0].fill = Some(gradient_fill(210.0));
    }
    petal.meta.modifiers.push(Modifier {
        enabled: true,
        kind: ModifierKind::Array {
            count: keys(&[(7.5, 1.0), (8.6, 12.0)], SMOOTH),
            offset: v(0.0, 0.0).into(),
            rotation: 30.0.into(),
            scale: v(1.0, 1.0).into(),
            origin: v(0.0, 0.0).into(),
            end_opacity: 0.55.into(),
        },
    });
    // Settles before the click (a pause shouldn't freeze it mid-turn).
    petal.transform.rotation = keys(&[(7.4, 0.0), (9.2, 40.0)], SMOOTH);
    beat(&mut petal, 7.4, 9.6);
    d.add(petal);
    let mut sq = BezPath::new();
    sq.move_to((0.0, -200.0));
    sq.curve_to((150.0, -140.0), (-60.0, -40.0), (110.0, 30.0));
    sq.curve_to((230.0, 80.0), (40.0, 170.0), (0.0, 210.0));
    let mut squiggle = path("Squiggle", sq, None, Some((CYAN, 14.0)), s + v(260.0, 0.0));
    squiggle.meta.modifiers.push(Modifier {
        enabled: true,
        kind: ModifierKind::Mirror { origin: v(0.0, 0.0).into(), rotation: 90.0.into(), merge: false },
    });
    draw_on(&mut squiggle, 7.8, 1.2);
    beat(&mut squiggle, 7.4, 9.6);
    d.add(squiggle);
    caption(d, "Live modifiers: radial arrays, mirrors and more", 7.4, 9.6);

    // E. Pens: the same stroke in brush, calligraphy and marker.
    for (k, (kind, color)) in
        [(PenKind::Brush, PINK), (PenKind::Calligraphy, INK), (PenKind::Marker, CYAN)].into_iter().enumerate()
    {
        let mut p = BezPath::new();
        p.move_to((-380.0, 30.0));
        p.curve_to((-200.0, -90.0), (0.0, 120.0), (160.0, -10.0));
        p.curve_to((260.0, -70.0), (330.0, 30.0), (400.0, -20.0));
        let mut l =
            path(&format!("{kind:?} stroke"), p, None, Some((color, 26.0)), s + v(0.0, -160.0 + k as f64 * 150.0));
        if let LayerKind::Shape { shapes } = &mut l.kind
            && let Some(st) = &mut shapes[0].stroke
        {
            st.style = PenStyle::preset(kind);
        }
        draw_on(&mut l, 9.9 + k as f64 * 0.35, 1.1);
        beat(&mut l, 9.8, 12.4);
        d.add(l);
    }
    caption(d, "Pens: brush, calligraphy, marker, pencil, watercolour…", 9.8, 12.4);
}

/// Three shapes united, as the Boolean tool makes it (worked out on a scratch copy).
fn union_of(layers: Vec<xenify_doc::Layer>) -> xenify_doc::Layer {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let mut ids = Vec::new();
    for mut l in layers {
        l.id = p.alloc_id();
        ids.push(l.id);
        xenify_ops::apply(&mut p, xenify_ops::Command::AddLayer { comp, index: None, layer: l }).unwrap();
    }
    let (cmd, id) =
        xenify_ops::pathfinder::combine(&p, comp, &ids, xenify_ops::pathfinder::PathOp::Unite, 0.0).unwrap();
    xenify_ops::apply(&mut p, cmd).unwrap();
    let mut l = p.layer(id).unwrap().clone();
    l.id = Id(0);
    l
}
