//! The assistant's diagram tools: library shapes, connectors between them,
//! and diagram canvases.

use crate::agent::ToolHost;
use crate::tools::ToolOutcome;
use crate::values;
use serde_json::{Value as Json, json};
use xenify_doc::{ConnectorEnd, Id, JumpStyle, Marker, MarkerKind, PortRef, RouteStyle, Vec2};
use xenify_ops::{Command, diagram, library};

fn tool(name: &str, description: &str, parameters: Json) -> Json {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}

pub fn definitions() -> Vec<Json> {
    let ids: Vec<&str> = library::all().iter().map(|s| s.id).collect();
    let end = json!({"description": "A layer id (glues to that shape) or [x, y] composition pixels (a loose end)."});
    let side = json!({"type": "string", "enum": ["auto", "top", "right", "bottom", "left"],
                      "description": "Which side of the shape the line leaves / arrives at. Default auto (the best side, updated as shapes move)."});
    let markers: Vec<String> = MarkerKind::ALL.iter().map(|k| marker_name(*k).to_string()).collect();
    vec![
        tool(
            "add_diagram_shape",
            "Adds a diagram shape from the library (flowchart symbols, network and cloud/software icons, BPMN events/tasks/gateways/pools, UML classes/actors/use cases/states/lifelines), centred at `position`, with a text label (inside the shape, or under icons). Returns its id. Connect shapes with add_connector. Leave about 80–120 px between shapes so connectors have room.",
            json!({"type": "object", "properties": {
                "shape": {"type": "string", "enum": ids},
                "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y] centre, composition pixels."},
                "size": {"type": "array", "items": {"type": "number"}, "description": "[width, height]. Default: the shape's own size."},
                "label": {"type": "string", "description": "Text shown with the shape. Default: the shape's name; \"\" for none."},
                "fill": {"type": "string", "description": "\"#rrggbb\" fill colour. Default white."},
                "in": {"type": "integer", "description": "A pool or lane (by id) to put it in: it becomes a member and moves with it. Default: the pool/lane under `position`, if any (the default position is then its middle)."}
            }, "required": ["shape"]}),
        ),
        tool(
            "add_connector",
            "Draws a connector: a line glued to shapes at both ends that routes itself around other shapes (elbow, straight or curved) and hops over lines it crosses without joining. It follows its shapes when they move or animate. Returns its id.",
            json!({"type": "object", "properties": {
                "from": end, "to": end,
                "from_side": side, "to_side": side,
                "route": {"type": "string", "enum": ["elbow", "straight", "curved"], "description": "Default: the canvas's default (elbow)."},
                "label": {"type": "string", "description": "Text along the line, e.g. \"yes\" / \"no\" out of a decision."},
                "label_background": {"type": "string", "description": "Colour of a box behind the label (it then sits on the line). Diagram canvases use the canvas colour by default."},
                "arrow": {"type": "string", "enum": markers, "description": "Mark at the `to` end. Default triangle."},
                "start_arrow": {"type": "string", "enum": markers, "description": "Mark at the `from` end. Default none."},
                "relation": {"type": "string", "enum": library::UML_RELATIONS,
                             "description": "A UML relationship: sets both marks and dashes (aggregation/composition put the diamond at `from`, the whole; inheritance/realisation/dependency point at `to`, the parent/supplier). `arrow`, `start_arrow` and `dashed` given as well override it."},
                "dashed": {"type": "boolean"},
                "flow": {"type": "number", "description": "Animate dashes moving along the line at this many px/s (shows data or work flowing)."},
                "color": {"type": "string"}, "width": {"type": "number"}
            }, "required": ["from", "to"]}),
        ),
        tool(
            "new_diagram",
            "Makes a new diagram canvas (a symbol with the diagram tools: shape library and connectors) and switches to it, so the following add_diagram_shape / add_connector calls build it. `place_in_current` also puts a copy of it on the canvas you were on (as a symbol you can move and scale). Use set_composition's `kind` to turn the current canvas into a diagram instead.",
            json!({"type": "object", "properties": {
                "name": {"type": "string"},
                "width": {"type": "integer"}, "height": {"type": "integer"},
                "place_in_current": {"type": "boolean", "description": "Default false."}
            }}),
        ),
    ]
}

fn marker_name(k: MarkerKind) -> &'static str {
    match k {
        MarkerKind::None => "none",
        MarkerKind::Triangle => "triangle",
        MarkerKind::OpenTriangle => "open_triangle",
        MarkerKind::Open => "open",
        MarkerKind::Arrow => "arrow",
        MarkerKind::Diamond => "diamond",
        MarkerKind::OpenDiamond => "open_diamond",
        MarkerKind::Circle => "circle",
        MarkerKind::OpenCircle => "open_circle",
        MarkerKind::Bar => "bar",
        MarkerKind::CrowsFoot => "crows_foot",
    }
}

fn marker(v: &Json) -> Result<Option<MarkerKind>, String> {
    match v.as_str() {
        None => Ok(None),
        Some(s) => MarkerKind::ALL
            .iter()
            .find(|k| marker_name(**k) == s)
            .copied()
            .map(Some)
            .ok_or_else(|| format!("unknown arrow “{s}”")),
    }
}

fn end(host: &dyn ToolHost, v: &Json, side: &Json) -> Result<ConnectorEnd, String> {
    if let Some(n) = v.as_u64() {
        let id = Id(n);
        let layer = host.project().layer(id).map_err(|_| format!("no layer {n}"))?;
        let port = match side.as_str().unwrap_or("auto") {
            "auto" => PortRef::Auto,
            s if layer.meta.ports.is_empty() => {
                let i = ["top", "right", "bottom", "left"].iter().position(|x| *x == s).ok_or("bad side")?;
                PortRef::Port(i as u32)
            }
            _ => PortRef::Auto,
        };
        return Ok(ConnectorEnd::glued(id, port, Vec2::ZERO));
    }
    Ok(ConnectorEnd::free(values::vec2(v)?))
}

pub fn run(name: &str, args: &Json, host: &mut dyn ToolHost) -> Option<Result<ToolOutcome, String>> {
    let r = match name {
        "add_diagram_shape" => add_shape(args, host),
        "add_connector" => add_connector(args, host),
        "new_diagram" => new_diagram(args, host),
        _ => return None,
    };
    Some(r)
}

fn done(result: Json, activity: String) -> ToolOutcome {
    ToolOutcome::Done { result, activity: Some(activity) }
}

fn add_shape(args: &Json, host: &mut dyn ToolHost) -> Result<ToolOutcome, String> {
    let shape = args["shape"].as_str().ok_or("missing `shape`")?;
    let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
    let container = match values::number(&args["in"]) {
        Some(n) => {
            let c = Id(n as u64);
            if !host.project().layer(c).is_ok_and(|l| l.meta.container) {
                return Err(format!("layer {n} isn't a pool or lane"));
            }
            Some(c)
        }
        None => None,
    };
    let at = match args.get("position").filter(|v| !v.is_null()) {
        Some(v) => values::vec2(v)?,
        None => match container {
            Some(c) => {
                let t = xenify_anim::world_transform(comp, c, host.time());
                let p = t * host
                    .project()
                    .layer(c)
                    .map_err(|e| e.to_string())?
                    .transform
                    .anchor
                    .value_at(host.time())
                    .to_point();
                Vec2::new(p.x, p.y)
            }
            None => Vec2::new(comp.settings.width as f64 / 2.0, comp.settings.height as f64 / 2.0),
        },
    };
    let size = match args.get("size").filter(|v| !v.is_null()) {
        Some(v) => Some(values::vec2(v)?),
        None => None,
    };
    let (mut cmd, id) = library::insert(host.project(), host.comp(), shape, at, size, args["label"].as_str())
        .map_err(|e| e.to_string())?;
    if let (Some(fill), Command::AddLayer { layer, .. }) = (args.get("fill").filter(|v| !v.is_null()), &mut cmd) {
        let c = values::color(fill)?;
        if let xenify_doc::LayerKind::Shape { shapes } = &mut layer.kind {
            for s in shapes.iter_mut().filter(|s| s.fill.is_some() && s.stroke.is_some()) {
                s.fill = Some(xenify_doc::Fill::solid(c));
            }
        }
    }
    host.execute("AI: add diagram shape", cmd)?;
    // In a pool or lane: named, or the one it was put in.
    let join = match container {
        Some(c) => {
            Some(diagram::join(host.project(), host.comp(), id, Some(c), host.time()).map_err(|e| e.to_string())?)
        }
        None => diagram::adopt(host.project(), host.comp(), &[id], host.time()).map_err(|e| e.to_string())?,
    };
    if let Some(cmd) = join {
        host.execute("AI: into the lane", cmd)?;
    }
    let name = library::find(shape).map_or(shape, |s| s.name);
    Ok(done(json!({"id": id.0}), format!("Added {name}")))
}

fn add_connector(args: &Json, host: &mut dyn ToolHost) -> Result<ToolOutcome, String> {
    let from = end(host, &args["from"], &args["from_side"])?;
    let to = end(host, &args["to"], &args["to_side"])?;
    let (cmd, id) = diagram::add_connector(host.project(), host.comp(), from, to).map_err(|e| e.to_string())?;
    let Command::AddLayer { comp, index, mut layer } = cmd else { unreachable!("add_connector adds a layer") };
    if let Some(c) = layer.meta.connector.as_deref_mut() {
        if let Some(r) = args["route"].as_str() {
            c.route = match r {
                "straight" => RouteStyle::Straight,
                "curved" => RouteStyle::Curved,
                _ => RouteStyle::Orthogonal,
            };
        }
        if let Some(speed) = args["flow"].as_f64() {
            c.flow = Some(xenify_doc::Flow { speed, kind: Default::default() });
        }
        if c.jump == JumpStyle::None {
            c.jump = JumpStyle::Arc;
        }
    }
    if let Some(text) = args["label"].as_str().filter(|t| !t.is_empty()) {
        let mut l = xenify_doc::NodeLabel::new(text);
        l.font_size = 13.0;
        if let Some(c) = args.get("label_background").filter(|c| !c.is_null()) {
            l.background = Some(crate::values::color(c)?);
        }
        layer.meta.label = Some(l);
    }
    if let xenify_doc::LayerKind::Shape { shapes } = &mut layer.kind
        && let Some(s) = shapes.first_mut().and_then(|s| s.stroke.as_mut())
    {
        if let Some(r) = args["relation"].as_str()
            && !library::style_uml_relation(s, r)
        {
            return Err(format!("unknown relation “{r}”"));
        }
        if let Some(k) = marker(&args["arrow"])? {
            s.end_marker = Marker { kind: k, size: 1.0 };
        }
        if let Some(k) = marker(&args["start_arrow"])? {
            s.start_marker = Marker { kind: k, size: 1.0 };
        }
        match args["dashed"].as_bool() {
            Some(true) => s.dashes = vec![8.0, 6.0],
            Some(false) => s.dashes.clear(),
            None => {}
        }
        if let Some(c) = args.get("color").filter(|v| !v.is_null()) {
            s.color = values::color(c)?.into();
        }
        if let Some(w) = args["width"].as_f64() {
            s.width = w.max(0.25).into();
        }
    }
    host.execute("AI: add connector", Command::AddLayer { comp, index, layer })?;
    Ok(done(json!({"id": id.0}), "Connected shapes".to_string()))
}

fn new_diagram(args: &Json, host: &mut dyn ToolHost) -> Result<ToolOutcome, String> {
    let name = args["name"].as_str().unwrap_or("Diagram");
    let w = args["width"].as_u64().unwrap_or(1600).clamp(64, 16384) as u32;
    let h = args["height"].as_u64().unwrap_or(1000).clamp(64, 16384) as u32;
    let current = host.comp();
    let (cmd, id) = diagram::new_diagram(host.project(), name, w, h);
    host.execute("AI: new diagram", cmd)?;
    if args["place_in_current"].as_bool() == Some(true) {
        let diagram = host.project().comp(id).map_err(|e| e.to_string())?.clone();
        let cur = host.project().comp(current).map_err(|e| e.to_string())?;
        let at = Vec2::new(cur.settings.width as f64 / 2.0, cur.settings.height as f64 / 2.0);
        let layer = xenify_doc::Layer::precomp(Id(host.project().next_id), name, &diagram, at);
        host.execute("AI: place diagram", Command::AddLayer { comp: current, index: None, layer })?;
    }
    host.set_comp(id)?;
    Ok(done(json!({"comp": id.0, "width": w, "height": h}), format!("New diagram “{name}”")))
}
