//! The assistant's raster tools: pixels on picture layers. A new raster
//! layer, brush strokes, the paint bucket, pixel selections, and a change to
//! the picture painted by an image model. Only hosts that edit pixels
//! ([`ToolHost::raster`]) offer them.

use crate::agent::ToolHost;
use crate::tools::ToolOutcome;
use crate::values;
use serde_json::{Value as Json, json};

fn tool(name: &str, description: &str, parameters: Json) -> Json {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}

pub fn definitions() -> Vec<Json> {
    let layer = json!({"type": "integer", "description": "A raster layer: any still picture layer (kind image), e.g. one from add_raster_layer."});
    let point = json!({"type": "array", "items": {"type": "number"}, "description": "[x, y] composition pixels."});
    vec![
        tool(
            "add_raster_layer",
            "Adds an empty, see-through raster layer the size of the canvas (above the selected layer) to paint pixels on with paint_pixels and fill_pixels. Returns its id.",
            json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        ),
        tool(
            "paint_pixels",
            "Paints brush strokes on a raster layer, like a person with a brush: each stroke is a line through `points` (composition pixels) with a round brush. Good for hand-drawn marks, shading, highlights, scribbles and rubbing out (erase). Inside a pixel selection, painting stays inside it. Each call is one undo step per stroke.",
            json!({"type": "object", "properties": {
                "layer": layer,
                "strokes": {"type": "array", "items": {"type": "object", "properties": {
                    "points": {"type": "array", "items": point, "description": "The stroke's path, 1 or more points; space points ≤ the brush size apart for curves."},
                    "color": {"type": "string", "description": "\"#rrggbb\" (default black)."},
                    "size": {"type": "number", "description": "Brush diameter in pixels (default 12)."},
                    "hardness": {"type": "number", "description": "0 (soft airbrush edge) … 1 (crisp edge). Default 0.8."},
                    "opacity": {"type": "number", "description": "0–1 (default 1)."},
                    "erase": {"type": "boolean", "description": "Rub out instead of painting."},
                    "brush": {"type": "string", "enum": ["paint", "smudge", "blur"], "description": "paint (default), smudge (drag the colours along) or blur (soften)."}
                }, "required": ["points"]}}
            }, "required": ["layer", "strokes"]}),
        ),
        tool(
            "fill_pixels",
            "The paint bucket: fills the area of like colour around a point on a raster layer (inside the pixel selection, if any). `all_layers` looks at what the canvas shows, so it fills inside line art kept on other layers (and a little under the lines).",
            json!({"type": "object", "properties": {
                "layer": layer,
                "at": point,
                "color": {"type": "string", "description": "\"#rrggbb\"."},
                "tolerance": {"type": "number", "description": "0–100: how alike colours must be (default 20)."},
                "all_layers": {"type": "boolean"}
            }, "required": ["layer", "at", "color"]}),
        ),
        tool(
            "select_pixels",
            "Selects part of a raster layer: painting, fill_pixels, delete and change_picture then only touch the selected pixels. shape: rect / ellipse (with `rect` [x0, y0, x1, y1]), lasso (with `points`), wand (the area of like colour at `at`), ai (what `what` describes, found by an image model: best for objects, clothing, people, sky), all, none, invert. mode: replace (default), add, subtract, intersect. Returns the selection's bounds (null: nothing selected).",
            json!({"type": "object", "properties": {
                "layer": layer,
                "shape": {"type": "string", "enum": ["rect", "ellipse", "lasso", "wand", "ai", "all", "none", "invert"]},
                "what": {"type": "string", "description": "ai: what to select, in words (\"the jacket\", \"the sky\"); an image model finds it (~10–30 s)."},
                "rect": {"type": "array", "items": {"type": "number"}},
                "points": {"type": "array", "items": point},
                "at": point,
                "tolerance": {"type": "number", "description": "wand: 0–100 (default 20)."},
                "mode": {"type": "string", "enum": ["replace", "add", "subtract", "intersect"]},
                "feather": {"type": "number", "description": "Soften the selection's edges by this many pixels."}
            }, "required": ["layer", "shape"]}),
        ),
        tool(
            "delete_pixels",
            "Deletes (makes see-through) the selected pixels of a raster layer, or all of them when nothing is selected.",
            json!({"type": "object", "properties": {"layer": layer}, "required": ["layer"]}),
        ),
        tool(
            "change_picture",
            "Has an image model repaint a picture or raster layer from a description of the change (\"make it night\", \"give her a red jacket\", \"remove the car\"). With a pixel selection and only_selection (default true), only the selected part changes and the rest stays exactly as it was: select the area first (select_pixels) for local changes. Photo-real or painterly results; takes ~10–40 s and costs a little. The result says what the model changed.",
            json!({"type": "object", "properties": {
                "layer": layer,
                "change": {"type": "string", "description": "What should change, in plain words."},
                "only_selection": {"type": "boolean"},
                "references": {"type": "array", "items": {"type": "object", "properties": {
                    "layer": {"type": "integer", "description": "A layer showing what you mean (it can be hidden)."},
                    "note": {"type": "string", "description": "What it shows, e.g. \"the jacket\"."}
                }, "required": ["layer"]}, "description": "Up to 4 reference pictures: a jacket to put on, a style to copy, a face to keep."},
                "keep_see_through": {"type": "boolean", "description": "Keep the layer's see-through parts see-through (default true: only the colours of what's there change). Set false to let the change paint into empty parts, e.g. colouring in a sketch on a see-through layer."}
            }, "required": ["layer", "change"]}),
        ),
    ]
}

fn layer(args: &Json) -> Result<u64, String> {
    values::number(&args["layer"]).map(|n| n as u64).ok_or_else(|| "missing integer `layer` id".into())
}

fn pt(v: &Json) -> Option<[f64; 2]> {
    Some([values::number(v.get(0)?)?, values::number(v.get(1)?)?])
}

const NAMES: [&str; 6] =
    ["add_raster_layer", "paint_pixels", "fill_pixels", "select_pixels", "delete_pixels", "change_picture"];

/// Runs a raster tool; `None` when `name` isn't one (or the host has no raster layers).
pub fn run(name: &str, args: &Json, host: &mut dyn ToolHost) -> Option<Result<ToolOutcome, String>> {
    if !NAMES.contains(&name) || !host.paints_pixels() {
        return None;
    }
    let done = |result: Json, activity: String| Ok(ToolOutcome::Done { result, activity: Some(activity) });
    Some((|| match name {
        "add_raster_layer" => {
            let id = host.raster("add_raster_layer", &json!({"name": args["name"]}))?;
            done(
                json!({"id": id}),
                format!("Added a raster layer{}", args["name"].as_str().map(|n| format!(" “{n}”")).unwrap_or_default()),
            )
        }
        "paint_pixels" => {
            let id = layer(args)?;
            let strokes = args["strokes"].as_array().ok_or("missing `strokes`")?;
            let mut n = 0;
            for s in strokes {
                let pts: Vec<Json> = s["points"]
                    .as_array()
                    .map(|a| a.iter().filter_map(pt).map(|[x, y]| json!([x, y, 0.5])).collect())
                    .unwrap_or_default();
                if pts.is_empty() {
                    continue;
                }
                host.raster("raster_begin", &json!({
                    "id": id, "color": s["color"].as_str().unwrap_or("#000000"), "size": values::number(&s["size"]).unwrap_or(12.0),
                    "hardness": values::number(&s["hardness"]).unwrap_or(0.8), "opacity": values::number(&s["opacity"]).unwrap_or(1.0),
                    "erase": s["erase"].as_bool().unwrap_or(false), "brush": s["brush"].as_str().unwrap_or("paint"), "pressure": false,
                }))?;
                let painted = host
                    .raster("raster_paint", &json!({"points": pts}))
                    .and_then(|_| host.raster("raster_end", &json!({})));
                if let Err(e) = painted {
                    let _ = host.raster("raster_cancel", &json!({}));
                    return Err(e);
                }
                n += 1;
            }
            done(json!({"ok": true, "strokes": n}), format!("Painted {n} stroke{}", if n == 1 { "" } else { "s" }))
        }
        "fill_pixels" => {
            let id = layer(args)?;
            let [x, y] = pt(&args["at"]).ok_or("missing `at` [x, y]")?;
            let all = args["all_layers"].as_bool().unwrap_or(false);
            if all {
                host.raster("raster_sample", &json!({"id": id}))?;
            }
            host.raster(
                "raster_fill",
                &json!({
                    "id": id, "x": x, "y": y, "color": args["color"].as_str().unwrap_or("#000000"),
                    "tolerance": values::number(&args["tolerance"]).unwrap_or(20.0),
                    "sample": if all { "canvas" } else { "layer" }, "grow": if all { 3 } else { 1 },
                }),
            )?;
            done(json!({"ok": true}), "Filled an area".into())
        }
        "select_pixels" => {
            let id = layer(args)?;
            let mode = args["mode"].as_str().unwrap_or("replace");
            if args["shape"].as_str() == Some("ai") {
                let what = args["what"].as_str().filter(|w| !w.trim().is_empty()).ok_or("shape ai needs `what`")?;
                let inputs = host.raster("raster_ai_inputs", &json!({"id": id, "max": 1024, "source": "layer"}))?;
                return Ok(ToolOutcome::SelectWithAi {
                    what: what.to_string(),
                    image: format!(
                        "data:image/png;base64,{}",
                        inputs["image"].as_str().ok_or("couldn't read the picture")?
                    ),
                    args: json!({"id": id, "mode": mode}),
                });
            }
            let bounds = match args["shape"].as_str().unwrap_or("rect") {
                "all" => host.raster("raster_select_all", &json!({"id": id}))?,
                "none" => host.raster("raster_select_none", &json!({"id": id}))?,
                "invert" => host.raster("raster_invert", &json!({"id": id}))?,
                "wand" => {
                    let [x, y] = pt(&args["at"]).ok_or("the wand needs `at` [x, y]")?;
                    host.raster("raster_wand", &json!({"id": id, "x": x, "y": y, "tolerance": values::number(&args["tolerance"]).unwrap_or(20.0), "mode": mode}))?
                }
                shape => host.raster(
                    "raster_select",
                    &json!({"id": id, "shape": shape, "rect": args["rect"], "points": args["points"], "mode": mode}),
                )?,
            };
            let bounds = match values::number(&args["feather"]).filter(|v| *v > 0.0) {
                Some(by) if !bounds.is_null() => host.raster("raster_feather", &json!({"id": id, "by": by}))?,
                _ => bounds,
            };
            let what = if bounds.is_null() { "Selected nothing".to_string() } else { "Selected pixels".to_string() };
            done(json!({"bounds_in_layer_pixels": bounds}), what)
        }
        "delete_pixels" => {
            host.raster("raster_clear", &json!({"id": layer(args)?}))?;
            done(json!({"ok": true}), "Deleted pixels".into())
        }
        "change_picture" => {
            let id = layer(args)?;
            let change = args["change"].as_str().filter(|c| !c.trim().is_empty()).ok_or("missing `change`")?;
            let info = host.raster("raster_info", &json!({"id": id}))?;
            if info["paintable"] == false {
                return Err(info["why"].as_str().unwrap_or("this layer can't be changed").to_string());
            }
            let only = args["only_selection"].as_bool().unwrap_or(true) && !info["selection"].is_null();
            let keep_alpha = args["keep_see_through"].as_bool().unwrap_or(true) && info["see_through"] == true;
            let inputs = host.raster("raster_ai_inputs", &json!({"id": id, "max": 1024, "source": "layer"}))?;
            let mut refs = Vec::new();
            for r in args["references"].as_array().into_iter().flatten().take(4) {
                let l = values::number(&r["layer"]).ok_or("each reference needs a `layer`")? as u64;
                let png = host.raster("layer_png", &json!({"id": l}))?;
                refs.push((
                    format!("data:image/png;base64,{}", png.as_str().unwrap_or_default()),
                    r["note"].as_str().unwrap_or("").to_string(),
                ));
            }
            Ok(ToolOutcome::EditPicture {
                change: change.to_string(),
                image: format!(
                    "data:image/png;base64,{}",
                    inputs["image"].as_str().ok_or("couldn't read the picture")?
                ),
                mask: if only { inputs["mask"].as_str().map(|m| format!("data:image/png;base64,{m}")) } else { None },
                refs,
                args: json!({"id": id, "only_selection": only, "keep_alpha": keep_alpha}),
            })
        }
        _ => unreachable!("one of NAMES"),
    })())
}
