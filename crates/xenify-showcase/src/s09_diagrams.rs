//! Slide 9: Diagrams: a flowchart builds shape by shape and its connectors glue
//! and route; a shape slides and its lines re-route live; two links cross
//! with a hop; network links run with flow dots; then the diagram (a symbol)
//! zooms out to show it placed on a slide. Built with the same operations
//! as the Shapes panel and the Connector tool.

use crate::kit::*;
use xenify_doc::{ConnectorEnd, Easing, Flow, FlowKind, Id, PortRef, RouteStyle, TransitionKind, Vec2};
use xenify_ops::Command;

const DW: f64 = 1600.0;
const DH: f64 = 900.0;

fn shape(d: &mut Deck, comp: Id, kind: &str, at: Vec2, label: &str, t: f64) -> Id {
    // Twice the library's size: this board is seen from far away.
    let base = xenify_ops::library::find(kind).expect("library shape");
    let size = v(base.size.0 * 2.0, base.size.1 * 2.0);
    let (cmd, id) = xenify_ops::library::insert(&d.p, comp, kind, at, Some(size), Some(label)).expect("library shape");
    d.exec(cmd);
    d.edit(id, |l| pop(l, t));
    id
}

fn link(d: &mut Deck, comp: Id, a: Id, b: Id, t: f64) -> Id {
    let pa = d.p.layer(a).unwrap().transform.position.value_at(0.0);
    let pb = d.p.layer(b).unwrap().transform.position.value_at(0.0);
    let (cmd, id) = xenify_ops::diagram::add_connector(
        &d.p,
        comp,
        ConnectorEnd::glued(a, PortRef::Auto, pa),
        ConnectorEnd::glued(b, PortRef::Auto, pb),
    )
    .expect("connector");
    d.exec(cmd);
    d.edit(id, |l| {
        l.meta.in_point = t;
        draw_on(l, t, 0.5);
    });
    id
}

pub fn build(d: &mut Deck) {
    // The diagram, on a whiteboard of its own.
    let (cmd, dia) = xenify_ops::diagram::new_diagram(&d.p, "System diagram", DW as u32, DH as u32);
    d.exec(cmd);
    let mut settings = d.p.comp(dia).unwrap().settings.clone();
    settings.duration = 10.8;
    settings.fps = FPS;
    d.exec(Command::SetCompSettings { comp: dia, settings });
    let mut board = rect("Whiteboard", v(DW, DH), 0.0, xenify_doc::Color::rgb(0.97, 0.97, 0.99), v(DW / 2.0, DH / 2.0));
    board.id = d.next_id();
    board.meta.locked = true;
    d.exec(Command::AddLayer { comp: dia, index: Some(0), layer: board });

    // The flowchart, shape by shape.
    let start = shape(d, dia, "flowchart.terminator", v(260.0, 110.0), "Idea", 0.2);
    let design = shape(d, dia, "flowchart.process", v(260.0, 320.0), "Design", 0.5);
    let check = shape(d, dia, "flowchart.decision", v(260.0, 560.0), "Looks good?", 0.8);
    let animate_s = shape(d, dia, "flowchart.process", v(700.0, 560.0), "Animate", 1.1);
    let present = shape(d, dia, "flowchart.terminator", v(700.0, 790.0), "Present", 1.4);
    link(d, dia, start, design, 0.7);
    link(d, dia, design, check, 1.0);
    let yes = link(d, dia, check, animate_s, 1.3);
    link(d, dia, animate_s, present, 1.6);
    let no = link(d, dia, check, design, 1.9);
    let cmd = xenify_ops::diagram::set_label_text(&d.p, yes, "yes").unwrap();
    d.exec(cmd);
    let cmd = xenify_ops::diagram::set_label_text(&d.p, no, "no").unwrap();
    d.exec(cmd);
    // Round the left side, not back up the same line.
    let cmd =
        xenify_ops::diagram::edit_connector(&d.p, no, |c| c.waypoints = vec![v(70.0, 560.0), v(70.0, 320.0)]).unwrap();
    d.exec(cmd);

    // The decision slides over; its lines re-route as it goes.
    d.edit(check, |l| {
        let p = v(260.0, 560.0);
        l.transform.position = keys(&[(3.4, p), (4.2, p + v(0.0, 200.0)), (4.9, p)], SMOOTH);
    });

    // A network: two links cross with a hop, and data flows along them.
    let laptop = shape(d, dia, "network.laptop", v(1000.0, 170.0), "Laptop", 5.5);
    let phone = shape(d, dia, "network.phone", v(1000.0, 470.0), "Phone", 5.6);
    let server = shape(d, dia, "network.server", v(1430.0, 470.0), "Server", 5.7);
    let cloud = shape(d, dia, "network.cloud", v(1430.0, 170.0), "Cloud", 5.8);
    for (a, b, t) in [(laptop, server, 6.1), (phone, cloud, 6.3), (server, cloud, 6.5)] {
        let c = link(d, dia, a, b, t);
        let cmd = xenify_ops::diagram::edit_connector(&d.p, c, |c| {
            if a != server {
                c.route = RouteStyle::Straight;
            }
            c.flow = Some(Flow { speed: 80.0, kind: FlowKind::Dots });
        })
        .unwrap();
        d.exec(cmd);
    }

    let pauses = [2.8, 5.2, 7.8, 10.4];
    d.slide(
        "Diagrams",
        10.8,
        &pauses,
        tr(TransitionKind::ZoomIn, 1.0),
        "Flowcharts and network diagrams whose lines stay glued.",
    );
    let placed = d.place(dia, "Diagram", v(W / 2.0, H / 2.0), 1.2, 0.0);
    let (t, k0, k1) = (8.1, 1.2, 0.5);
    // Then it zooms out: it's a symbol on a slide.
    d.edit(placed, |l| {
        l.transform.scale = keys(&[(t, v(k0, k0)), (t + 1.0, v(k1, k1))], SMOOTH);
        l.transform.position = keys(&[(t, v(W / 2.0, H / 2.0)), (t + 1.0, v(W / 2.0, 600.0))], SMOOTH);
    });
    let mut frame = rect("Slide frame", v(DW * k1 + 40.0, DH * k1 + 40.0), 20.0, PANEL, v(W / 2.0, 600.0));
    if let xenify_doc::LayerKind::Shape { shapes } = &mut frame.kind {
        shapes[0].fill = None;
        shapes[0].stroke = Some(stroke(VIOLET, 4.0));
    }
    frame.opacity = keys(&[(t + 0.8, 0.0), (t + 1.2, 1.0)], Easing::Linear);
    d.add(frame);
    let mut heading = text_left("Title", "Diagrams", 120.0, INK, v(120.0, 200.0));
    heading.opacity = keys(&[(t + 0.6, 0.0), (t + 1.0, 1.0)], Easing::Linear);
    d.add(heading);
    let mut cap =
        text("Caption", "Diagrams live on their own canvas and drop into any slide", 40.0, MUTED, v(W / 2.0, 1040.0));
    animate(&mut cap, "Fade Up by Word", t + 0.8);
    d.add(cap);
    d.sfx(xenify_audio::synth::Sfx::Swish, t, 0.0, 0.6);
    for (k, tt) in [0.2, 0.5, 0.8, 1.1, 1.4].into_iter().enumerate() {
        d.sfx(xenify_audio::synth::Sfx::Click, tt, k as f64, 0.5);
    }
}
