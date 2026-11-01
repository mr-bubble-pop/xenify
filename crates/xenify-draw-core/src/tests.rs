use super::*;

fn call(s: &mut Session, name: &str, args: Json) -> Json {
    s.call(name, &args).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn the_poster_lists_its_layers_top_first() {
    let mut s = Session::default();
    let st = call(&mut s, "state", json!({}));
    let names: Vec<&str> = st["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names.first(), Some(&"Invite line"));
    assert!(names.contains(&"Heading"));
    assert_eq!(st["comp"]["width"], 1080);
}

#[test]
fn add_select_move_and_undo() {
    let mut s = Session::new(Project::new());
    let id =
        call(&mut s, "add_shape", json!({"kind": "rect", "x": 100, "y": 100, "w": 200, "h": 100})).as_u64().unwrap();
    assert_eq!(call(&mut s, "hit", json!({"x": 150, "y": 150})), json!(id));
    call(&mut s, "select", json!({"ids": [id]}));
    call(&mut s, "transform_begin", json!({}));
    call(&mut s, "transform", json!({"affine": [1, 0, 0, 1, 10, 0]}));
    call(&mut s, "transform", json!({"affine": [1, 0, 0, 1, 50, 5]}));
    call(&mut s, "transform_end", json!({}));
    let l = call(&mut s, "layer", json!({"id": id}));
    assert_eq!(l["props"]["position"]["value"], json!([250.0, 155.0]));
    // The two drags are one undo step.
    call(&mut s, "undo", json!({}));
    let l = call(&mut s, "layer", json!({"id": id}));
    assert_eq!(l["props"]["position"]["value"], json!([200.0, 150.0]));
    let b = call(&mut s, "bounds", json!({}));
    assert_eq!(b[0]["corners"][0], json!([100.0, 100.0]));
}

#[test]
fn properties_animate_and_texts_say_when_they_show() {
    let mut s = Session::default();
    let st = call(&mut s, "state", json!({}));
    let date = st["layers"].as_array().unwrap().iter().find(|l| l["name"] == "Date and place").unwrap()["id"]
        .as_u64()
        .unwrap();
    let texts = call(&mut s, "texts", json!({}));
    let t = texts.as_array().unwrap().iter().find(|t| t["id"] == date).unwrap();
    assert_eq!(t["from"], json!(1.2));
    call(&mut s, "set_time", json!({"time": 0.0}));
    call(&mut s, "toggle_anim", json!({"id": date, "prop": "rotation"}));
    call(&mut s, "set_time", json!({"time": 2.0}));
    call(&mut s, "set_prop", json!({"id": date, "prop": "rotation", "value": 10.0}));
    let l = call(&mut s, "layer", json!({"id": date}));
    assert_eq!(l["props"]["rotation"]["keys"], json!([0.0, 2.0]));
    call(&mut s, "set_text", json!({"id": date, "text": "Sunday"}));
    assert_eq!(call(&mut s, "layer", json!({"id": date}))["text"]["text"], "Sunday");
}

#[test]
fn saves_and_opens() {
    let mut s = Session::default();
    let saved = call(&mut s, "save", json!({}));
    let mut t = Session::new(Project::new());
    let st = call(&mut t, "open", json!({"json": saved}));
    assert_eq!(st["comp"]["name"], "Autumn bake sale");
}

fn ids(st: &Json) -> Vec<u64> {
    st["layers"].as_array().unwrap().iter().map(|l| l["id"].as_u64().unwrap()).collect()
}

#[test]
fn every_shape_of_the_shapes_button_draws() {
    let mut s = Session::new(Project::new());
    for kind in ["rect", "rounded", "ellipse", "triangle", "polygon", "star", "gear", "heart", "arrow", "bubble"] {
        let id =
            call(&mut s, "add_figure", json!({"kind": kind, "x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000"}))
                .as_u64()
                .unwrap();
        let b = call(&mut s, "bounds", json!({"ids": [id]}));
        let c = b[0]["corners"].as_array().unwrap();
        assert!((c[2][0].as_f64().unwrap() - c[0][0].as_f64().unwrap()) > 50.0, "{kind}");
        assert!(
            !call(&mut s, "points", json!({"id": id}))["shapes"][0]["anchors"].as_array().unwrap().is_empty(),
            "{kind}"
        );
    }
    call(&mut s, "add_line", json!({"a": [0, 0], "b": [100, 50], "stroke": "#000000"}));
    assert_eq!(ids(&call(&mut s, "state", json!({}))).len(), 11);
}

#[test]
fn pathfinder_aligns_and_groups() {
    let mut s = Session::new(Project::new());
    let a = call(&mut s, "add_figure", json!({"kind": "rect", "x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000"}))
        .as_u64()
        .unwrap();
    let b =
        call(&mut s, "add_figure", json!({"kind": "ellipse", "x": 50, "y": 50, "w": 100, "h": 100, "fill": "#00ff00"}))
            .as_u64()
            .unwrap();
    call(&mut s, "select", json!({"ids": [a, b]}));
    call(&mut s, "align", json!({"edge": "left"}));
    assert!(call(&mut s, "bounds", json!({"ids": [b]}))[0]["corners"][0][0].as_f64().unwrap().abs() < 1e-6);
    call(&mut s, "group", json!({}));
    assert_eq!(ids(&call(&mut s, "state", json!({}))).len(), 3);
    call(&mut s, "ungroup", json!({}));
    call(&mut s, "select", json!({"ids": [a, b]}));
    call(&mut s, "path_op", json!({"op": "unite"}));
    assert_eq!(ids(&call(&mut s, "state", json!({}))).len(), 1);
    call(&mut s, "undo", json!({}));
    assert_eq!(ids(&call(&mut s, "state", json!({}))).len(), 2);
}

#[test]
fn deleting_a_group_deletes_what_is_in_it() {
    let mut s = Session::new(Project::new());
    let a = call(&mut s, "add_shape", json!({"kind": "rect", "x": 100, "y": 100, "w": 50, "h": 50})).as_u64().unwrap();
    let b = call(&mut s, "add_shape", json!({"kind": "rect", "x": 300, "y": 100, "w": 50, "h": 50})).as_u64().unwrap();
    call(&mut s, "select", json!({"ids": [a, b]}));
    call(&mut s, "group", json!({}));
    let g = call(&mut s, "state", json!({}))["selection"][0].as_u64().unwrap();
    call(&mut s, "select", json!({"ids": [g]}));
    call(&mut s, "delete", json!({}));
    assert!(ids(&call(&mut s, "state", json!({}))).is_empty(), "nothing is left loose");
    call(&mut s, "undo", json!({}));
    assert_eq!(ids(&call(&mut s, "state", json!({}))).len(), 3, "one undo brings back the group and its layers");
}

#[test]
fn points_move_insert_and_edit() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_figure", json!({"kind": "rect", "x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000"}))
        .as_u64()
        .unwrap();
    let pts = call(&mut s, "points", json!({"id": id}));
    assert_eq!(pts["shapes"][0]["anchors"].as_array().unwrap().len(), 4);
    assert_eq!(pts["shapes"][0]["anchors"][0]["p"], json!([0.0, 0.0]));
    call(&mut s, "points_begin", json!({}));
    call(&mut s, "points_move", json!({"id": id, "points": [[0, 0]], "dx": 5, "dy": 0}));
    call(&mut s, "points_move", json!({"id": id, "points": [[0, 0]], "dx": 10, "dy": 0}));
    let pts = call(&mut s, "points", json!({"id": id}));
    assert_eq!(pts["shapes"][0]["anchors"][0]["p"], json!([10.0, 0.0]));
    let at = call(&mut s, "point_insert", json!({"id": id, "at": [100, 50]}));
    assert!(at.is_array());
    assert_eq!(call(&mut s, "points", json!({"id": id}))["shapes"][0]["anchors"].as_array().unwrap().len(), 5);
    call(&mut s, "points_edit", json!({"id": id, "op": "smooth", "points": [at]}));
    let a = &call(&mut s, "points", json!({"id": id}))["shapes"][0]["anchors"][at[1].as_u64().unwrap() as usize];
    assert!(a["in"].is_array() && a["out"].is_array());
    call(&mut s, "points_edit", json!({"id": id, "op": "delete", "points": [at]}));
    assert_eq!(call(&mut s, "points", json!({"id": id}))["shapes"][0]["anchors"].as_array().unwrap().len(), 4);
}

#[test]
fn copy_paste_and_curved_pen() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "pen_path", json!({"nodes": [{"p": [0, 0]}, {"p": [100, 0], "h": [0, 40]}, {"p": [100, 100]}], "stroke": "#000000", "stroke_width": 3})).as_u64().unwrap();
    let segs = &call(&mut s, "points", json!({"id": id}))["shapes"][0]["segments"];
    assert_eq!(segs[1].as_array().unwrap().len(), 4, "the second piece is a curve");
    call(&mut s, "select", json!({"ids": [id]}));
    let text = call(&mut s, "copy", json!({}));
    call(&mut s, "paste", json!({"text": text}));
    assert_eq!(ids(&call(&mut s, "state", json!({}))).len(), 2);
}

#[test]
fn the_pen_continues_an_open_line() {
    let mut s = Session::new(Project::new());
    let id = call(
        &mut s,
        "pen_path",
        json!({"nodes": [{"p": [0, 0]}, {"p": [100, 0]}], "stroke": "#000000", "stroke_width": 3}),
    )
    .as_u64()
    .unwrap();
    let pts = call(&mut s, "points", json!({"id": id}));
    let end = pts["shapes"][0]["ends"].as_array().unwrap().iter().find(|e| e["from_start"] == false).unwrap().clone();
    assert_eq!(end["p"], json!([100.0, 0.0]));
    call(
        &mut s,
        "pen_path",
        json!({"nodes": [{"p": [100, 100]}], "into": id, "shape": 0, "subpath": end["subpath"], "from_start": false}),
    );
    let pts = call(&mut s, "points", json!({"id": id}));
    assert_eq!(pts["shapes"][0]["anchors"].as_array().unwrap().len(), 3);
    assert_eq!(pts["shapes"][0]["anchors"][2]["p"], json!([100.0, 100.0]));
}

#[test]
fn pencil_recognises_and_knife_cuts() {
    let mut s = Session::new(Project::new());
    let circle: Vec<Json> = (0..=40)
        .map(|i| {
            let a = i as f64 / 40.0 * std::f64::consts::TAU;
            json!([200.0 + 100.0 * a.cos(), 200.0 + 100.0 * a.sin()])
        })
        .collect();
    let id =
        call(&mut s, "freehand", json!({"points": circle, "recognize": true, "fill": "#ff0000"})).as_u64().unwrap();
    assert_eq!(call(&mut s, "layer", json!({"id": id}))["name"], "Circle");
    call(&mut s, "select", json!({"ids": []}));
    let n = call(&mut s, "knife", json!({"a": [50, 200], "b": [350, 200]}));
    assert_eq!(n, json!(2));
    let brush = call(&mut s, "freehand", json!({"points": [[0, 0], [50, 10], [100, 0]], "brush": true, "width": 10}));
    assert!(brush.is_u64());
}

#[test]
fn sculpt_pushes_points() {
    let mut s = Session::new(Project::new());
    let id =
        call(&mut s, "add_figure", json!({"kind": "ellipse", "x": 0, "y": 0, "w": 200, "h": 200, "fill": "#ff0000"}))
            .as_u64()
            .unwrap();
    let before = call(&mut s, "bounds", json!({"ids": [id]}));
    for i in 0..5 {
        call(
            &mut s,
            "sculpt",
            json!({"center": [200.0 + i as f64 * 5.0, 100], "delta": [5, 0], "radius": 40, "stroke": 1}),
        );
    }
    let after = call(&mut s, "bounds", json!({"ids": [id]}));
    assert!(after[0]["corners"][1][0].as_f64().unwrap() > before[0]["corners"][1][0].as_f64().unwrap() + 5.0);
    call(&mut s, "undo", json!({}));
    assert_eq!(call(&mut s, "bounds", json!({"ids": [id]})), before);
}

#[test]
fn effects_modifiers_masks_and_gradients() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_figure", json!({"kind": "star", "x": 0, "y": 0, "w": 200, "h": 200, "fill": "#ff0000", "stroke": "#000000", "stroke_width": 3})).as_u64().unwrap();
    call(&mut s, "add_effect", json!({"kind": "drop_shadow"}));
    call(&mut s, "add_modifier", json!({"preset": "Twist"}));
    call(&mut s, "add_mask", json!({"ellipse": true}));
    call(&mut s, "set_gradient", json!({"id": id, "index": 0, "kind": "radial"}));
    let full = call(&mut s, "layer_full", json!({"id": id}));
    let paths: Vec<&str> = full["props"].as_array().unwrap().iter().map(|p| p["path"].as_str().unwrap()).collect();
    let blur = "effects.0.radius".to_string();
    assert!(paths.contains(&"effects.0.radius"));
    call(&mut s, "set_prop", json!({"id": id, "prop": blur, "value": 20.0}));
    let full = call(&mut s, "layer_full", json!({"id": id}));
    let v = full["props"].as_array().unwrap().iter().find(|p| p["path"] == blur.as_str()).unwrap()["value"].clone();
    assert_eq!(v, json!(20.0));
    assert_eq!(full["layer"]["effects"].as_array().map(|a| a.len()), Some(1));
    let pen = call(&mut s, "pen_preset", json!({"kind": "calligraphy"}));
    assert_eq!(pen["kind"], "calligraphy");
}

#[test]
fn the_mask_tool_draws_and_reshapes_masks() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_figure", json!({"kind": "rect", "x": 0, "y": 0, "w": 200, "h": 200, "fill": "#ff0000"}))
        .as_u64()
        .unwrap();
    assert_eq!(call(&mut s, "add_mask_at", json!({"id": id, "a": [50, 50], "b": [150, 150]})), json!(0));
    let pts = call(&mut s, "points", json!({"id": id, "masks": true}));
    assert_eq!(pts["shapes"][0]["anchors"][0]["p"], json!([50.0, 50.0]));
    call(&mut s, "points_begin", json!({}));
    call(&mut s, "points_move", json!({"id": id, "masks": true, "points": [[0, 0]], "dx": -10, "dy": 0}));
    let pts = call(&mut s, "points", json!({"id": id, "masks": true}));
    assert_eq!(pts["shapes"][0]["anchors"][0]["p"], json!([40.0, 50.0]));
    // The shape itself didn't change.
    assert_eq!(call(&mut s, "points", json!({"id": id}))["shapes"][0]["anchors"][0]["p"], json!([0.0, 0.0]));
}

#[test]
fn easing_presets_wiggle_expressions_and_graph() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_figure", json!({"kind": "rect", "x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000"}))
        .as_u64()
        .unwrap();
    call(&mut s, "apply_preset", json!({"name": "Fade in"}));
    let ks = call(&mut s, "key_easings", json!({"id": id, "prop": "opacity"}));
    assert!(ks.as_array().unwrap().len() >= 2);
    call(&mut s, "set_easing", json!({"id": id, "prop": "opacity", "time": ks[0]["time"], "easing": "ease_in"}));
    assert_eq!(call(&mut s, "key_easings", json!({"id": id, "prop": "opacity"}))[0]["name"], "ease_in");
    let g = call(&mut s, "graph", json!({"id": id, "prop": "opacity"}));
    assert!(g["samples"].as_array().unwrap().len() > 10);
    call(&mut s, "set_key", json!({"id": id, "prop": "opacity", "time": ks[1]["time"], "value": 0.5}));
    assert_eq!(call(&mut s, "graph", json!({"id": id, "prop": "opacity"}))["keys"][1]["value"], json!([0.5]));
    call(&mut s, "set_wiggle", json!({"id": id, "wiggle": {"frequency": 2.0, "position": 20.0, "seed": 3}}));
    call(&mut s, "set_expression", json!({"id": id, "prop": "rotation", "source": "time * 90"}));
    call(&mut s, "set_time", json!({"time": 1.0}));
    assert_eq!(
        call(&mut s, "expr_preview", json!({"id": id, "prop": "rotation", "source": "time * 90"}))["ok"],
        json!(90.0)
    );
    let full = call(&mut s, "layer_full", json!({"id": id}));
    assert!(full["layer"]["transform"]["wiggle"].is_object());
    assert_eq!(full["layer"]["expressions"][0]["source"], "time * 90");
}

#[test]
fn svg_and_lottie_out_and_back_in() {
    let mut s = Session::default();
    let svg = call(&mut s, "export_svg", json!({})).as_str().unwrap().to_string();
    assert!(svg.starts_with("<svg") || svg.contains("<svg"));
    let animated = call(&mut s, "export_svg", json!({"animated": true}));
    assert!(animated.as_str().unwrap().contains("@keyframes") || animated.as_str().unwrap().contains("<animate"));
    let lottie = call(&mut s, "export_lottie", json!({}));
    let text = lottie.as_str().unwrap().to_string();
    assert!(text.contains("\"layers\""));
    let mut t = Session::new(Project::new());
    let got = call(&mut t, "import_svg", json!({"text": svg, "name": "Poster"}));
    assert_eq!(got.as_array().unwrap().len(), 1, "grouped");
    let lot = call(&mut t, "import_lottie", json!({"text": text, "name": "Poster animation"}));
    assert!(lot["id"].is_u64());
}

#[test]
fn a_still_canvas_takes_settings_slices_and_guides() {
    let mut s = Session::new(Project::new());
    call(&mut s, "new", json!({"width": 800, "height": 600, "duration": 0}));
    call(&mut s, "set_comp", json!({"background": "#ff0000"}));
    call(&mut s, "add_slice", json!({"rect": [0, 0, 100, 100]}));
    call(&mut s, "set_guides", json!({"guides": [{"vertical": true, "pos": 50.0}]}));
    assert_eq!(call(&mut s, "slices", json!({})).as_array().unwrap().len(), 1);
    assert_eq!(call(&mut s, "guides", json!({}))["guides"].as_array().unwrap().len(), 1);
}

#[test]
fn the_assistant_runs_tools_and_waits_for_accept() {
    let mut s = Session::new(Project::new());
    let st = call(&mut s, "ai_send", json!({"text": "Add a red circle"}));
    let req = &st["requests"][0];
    assert!(req["body"]["tools"].is_array());
    let id = req["id"].clone();
    // The model asks for a tool…
    let reply = json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{
        "id": "c1", "type": "function",
        "function": {"name": "add_layer", "arguments": "{\"type\":\"ellipse\",\"name\":\"Ball\",\"size\":[100,100],\"fill\":\"#ff0000\"}"}
    }]}, "finish_reason": "tool_calls"}]});
    let st = call(&mut s, "ai_reply", json!({"id": id, "response": reply}));
    assert_eq!(call(&mut s, "state", json!({}))["layers"][0]["name"], "Ball");
    // …then says it's done.
    let id = st["requests"][0]["id"].clone();
    let done = json!({"choices": [{"message": {"role": "assistant", "content": "Added a red ball."}, "finish_reason": "stop"}]});
    let st = call(&mut s, "ai_reply", json!({"id": id, "response": done}));
    assert_eq!(st["reviewing"], true);
    call(&mut s, "ai_reject", json!({}));
    assert!(call(&mut s, "state", json!({}))["layers"].as_array().unwrap().is_empty());
}

#[test]
fn the_assistant_paints_pixels_and_changes_pictures() {
    let mut s = Session::new(Project::new());
    let st = call(&mut s, "ai_send", json!({"text": "Paint a red stroke, then make the top half blue"}));
    let names: Vec<String> = st["requests"][0]["body"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.iter().any(|n| n == "paint_pixels") && names.iter().any(|n| n == "change_picture"));
    let tool = |id: &str, name: &str, args: Json| {
        json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{
        "id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}, "finish_reason": "tool_calls"}]})
    };
    let st = call(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c1", "add_raster_layer", json!({"name": "Paint"}))}),
    );
    let layer = s.selection[0].0;
    let st = call(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c2", "paint_pixels", json!({"layer": layer,
        "strokes": [{"points": [[100, 100], [300, 100]], "color": "#ff0000", "size": 20, "hardness": 1}]}))}),
    );
    let d = call(&mut s, "raster_info", json!({"id": layer}))["px_per_unit"].as_f64().unwrap();
    let at = |v: f64| (v * d) as u32;
    assert_eq!(pixel(&s, layer, at(200.0), at(100.0)), [255, 0, 0, 255]);
    let st = call(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c3", "select_pixels", json!({"layer": layer, "shape": "rect", "rect": [0, 0, 4000, 102]}))}),
    );
    // change_picture: an image request with the layer's pixels and the selection as a mask.
    let st = call(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c4", "change_picture", json!({"layer": layer, "change": "make it blue"}))}),
    );
    let req = &st["requests"][0];
    let body = req["body"].to_string();
    assert!(
        body.contains("make it blue")
            && body.contains("modalities")
            && body.matches("data:image/png;base64").count() == 2,
        "picture + mask: {}",
        &body[..body.len().min(600)]
    );
    // The image model's answer: all blue, with a sentence.
    let (w, h) = (at(1920.0).max(1), at(1080.0).max(1));
    let blue = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 255, 255]));
    let mut png = Vec::new();
    blue.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let url =
        format!("data:image/png;base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png));
    let answer = json!({"choices": [{"message": {"role": "assistant", "content": "I made the top of the stroke blue.", "images": [{"type": "image_url", "image_url": {"url": url}}]}}]});
    let st = call(&mut s, "ai_reply", json!({"id": req["id"], "response": answer}));
    assert_eq!(pixel(&s, layer, at(200.0), at(95.0)), [0, 0, 255, 255], "inside the selection");
    assert_eq!(pixel(&s, layer, at(200.0), at(108.0)), [255, 0, 0, 255], "outside it, as it was");
    assert_eq!(pixel(&s, layer, at(600.0), at(50.0))[3], 0, "see-through parts kept");
    assert!(st["transcript"].to_string().contains("I made the top of the stroke blue."));
    let done = json!({"choices": [{"message": {"role": "assistant", "content": "Done."}, "finish_reason": "stop"}]});
    let st = call(&mut s, "ai_reply", json!({"id": st["requests"][0]["id"], "response": done}));
    assert_eq!(st["reviewing"], true);
    call(&mut s, "ai_reject", json!({}));
    assert!(ids_now(&mut s).is_empty(), "rejecting takes the layer and its pixels away");
}

#[test]
fn the_assistant_selects_with_ai_fills_all_layers_and_uses_references() {
    let mut s = Session::new(Project::new());
    let layer = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let other = call(&mut s, "add_shape", json!({"kind": "rect", "x": 0, "y": 0, "w": 50, "h": 50})).as_u64().unwrap();
    call(&mut s, "set_meta", json!({"id": other, "visible": false}));
    let png_of = |img: &image::RgbaImage| {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    // The screens' renderer, standing in: a black frame with a clear middle (line art on another layer).
    let mut renders = 0;
    let mut png = |_: &xenify_anim::Scene, w: u32, h: u32| -> Result<Vec<u8>, String> {
        renders += 1;
        Ok(png_of(&image::RgbaImage::from_fn(w, h, |x, y| {
            if x < w / 10 || y < h / 10 || x > w * 9 / 10 || y > h * 9 / 10 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        })))
    };
    type Png<'a> = dyn FnMut(&xenify_anim::Scene, u32, u32) -> Result<Vec<u8>, String> + 'a;
    let ai = |s: &mut Session, name: &str, args: Json, png: &mut Png| {
        s.ai_call_with(name, &args, Some(png)).unwrap().unwrap()
    };
    let st = ai(&mut s, "ai_send", json!({"text": "colour it in"}), &mut png);
    let tool = |id: &str, name: &str, args: Json| {
        json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{
        "id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}, "finish_reason": "tool_calls"}]})
    };
    // Fill on all layers: inside the frame drawn by the other layer.
    let st = ai(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c1", "fill_pixels", json!({"layer": layer, "at": [960, 540], "color": "#00ff00", "all_layers": true}))}),
        &mut png,
    );
    let d = call(&mut s, "raster_info", json!({"id": layer}))["px_per_unit"].as_f64().unwrap();
    assert_eq!(pixel(&s, layer, (960.0 * d) as u32, (540.0 * d) as u32), [0, 255, 0, 255]);
    assert_eq!(pixel(&s, layer, (20.0 * d) as u32, (20.0 * d) as u32)[3], 0, "not under the frame's far side");
    // Select with AI: a mask request, then the selection.
    let st = ai(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c2", "select_pixels", json!({"layer": layer, "shape": "ai", "what": "the middle"}))}),
        &mut png,
    );
    let req = st["requests"][0].clone();
    assert!(req["body"].to_string().contains("black-and-white mask") && req["body"].to_string().contains("the middle"));
    let (w, h) = ((1920.0 * d) as u32, (1080.0 * d) as u32);
    let mask = png_of(&image::RgbaImage::from_fn(w / 2, h / 2, |x, _| {
        if x > w / 4 { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([0, 0, 0, 255]) }
    }));
    let answer = |bytes: &[u8]| json!({"choices": [{"message": {"role": "assistant", "content": "", "images": [{"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes))}}]}}]});
    let st = ai(&mut s, "ai_reply", json!({"id": req["id"], "response": answer(&mask)}), &mut png);
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": layer, "x": 1500, "y": 540})), true);
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": layer, "x": 300, "y": 540})), false);
    // change_picture with a (hidden) layer as a reference: picture, mask and reference go to the model.
    let st = ai(
        &mut s,
        "ai_reply",
        json!({"id": st["requests"][0]["id"], "response": tool("c3", "change_picture", json!({"layer": layer, "change": "use this", "references": [{"layer": other, "note": "the box"}]}))}),
        &mut png,
    );
    let body = st["requests"][0]["body"].to_string();
    assert_eq!(body.matches("data:image/png;base64").count(), 3, "picture, mask, reference");
    assert!(body.contains("the box"));
    let red = png_of(&image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255])));
    let st = ai(&mut s, "ai_reply", json!({"id": st["requests"][0]["id"], "response": answer(&red)}), &mut png);
    // The model said nothing: the brain is told to look for itself.
    assert!(st["requests"][0]["body"].to_string().contains("look_at_frame"));
    assert_eq!(pixel(&s, layer, (1500.0 * d) as u32, (540.0 * d) as u32), [255, 0, 0, 255]);
    assert_eq!(pixel(&s, layer, (300.0 * d) as u32, (540.0 * d) as u32), [0, 255, 0, 255]);
    let _ = renders;
}

#[test]
fn pictures_text_boxes_keys_and_pauses() {
    let mut s = Session::new(Project::new());
    // A new picture (a 1×1 PNG), then the same one again: one asset, two layers.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let a = call(
        &mut s,
        "add_picture",
        json!({"base64": png, "name": "dot.png", "mime": "image/png", "width": 1, "height": 1}),
    );
    let b = call(
        &mut s,
        "add_picture",
        json!({"base64": png, "name": "dot.png", "mime": "image/png", "width": 1, "height": 1}),
    );
    assert_ne!(a, b);
    assert_eq!(s.project().assets.len(), 1);
    // A text box from the Text tool, resized.
    let t =
        call(&mut s, "add_text", json!({"x": 10, "y": 10, "text": "Hello there", "box": [300, 120]})).as_u64().unwrap();
    call(&mut s, "set_text_style", json!({"id": t, "box_w": 200, "letter_spacing": 2, "align": "center"}));
    let info = call(&mut s, "layer", json!({"id": t}));
    assert_eq!(info["text"]["frame"]["w"], json!(200.0));
    assert_eq!(info["text"]["align"], json!("center"));
    // Key everything at 1 s, then move the keys and copy them onto another layer.
    call(&mut s, "set_time", json!({"time": 1.0}));
    assert_eq!(call(&mut s, "key_all", json!({"ids": [t], "all": true})), json!(true));
    let keys = call(&mut s, "all_keys", json!({"ids": [t]}));
    assert!(!keys.as_array().unwrap().is_empty());
    call(&mut s, "move_keys", json!({"keys": keys, "delta": 0.5}));
    assert!(
        call(&mut s, "all_keys", json!({"ids": [t]}))
            .as_array()
            .unwrap()
            .iter()
            .all(|k| (k["time"].as_f64().unwrap() - 1.5).abs() < 1e-9)
    );
    // Pauses toggle; trimming keeps out after in.
    assert_eq!(call(&mut s, "toggle_pause", json!({"time": 2.0})), json!(true));
    assert_eq!(call(&mut s, "state", json!({}))["comp"]["pauses"], json!([2.0]));
    assert_eq!(call(&mut s, "toggle_pause", json!({"time": 2.0})), json!(false));
    call(&mut s, "set_in_out", json!({"id": t, "in": 1.0, "out": 0.5}));
    let row = call(&mut s, "state", json!({}))["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == json!(t))
        .cloned()
        .unwrap();
    assert!(row["out"].as_f64().unwrap() > row["in"].as_f64().unwrap());
}

#[test]
fn layers_panel_water_camera_3d_and_emoji_confetti() {
    let mut s = Session::default();
    let st = call(&mut s, "state", json!({}));
    let ids: Vec<u64> = st["layers"].as_array().unwrap().iter().map(|l| l["id"].as_u64().unwrap()).collect();
    // A layer dragged into a new group in the Layers list sits inside it.
    let g = call(&mut s, "add_group", json!({}))
        .as_array()
        .map(|a| a[0].clone())
        .unwrap_or_else(|| call(&mut s, "state", json!({}))["selection"][0].clone())
        .as_u64()
        .unwrap();
    assert_eq!(call(&mut s, "move_layers", json!({"ids": [ids[3]], "target": g, "place": "into"})), json!(true));
    let st = call(&mut s, "state", json!({}));
    let row = st["layers"].as_array().unwrap().iter().find(|l| l["id"] == ids[3]).unwrap();
    assert_eq!(row["depth"], 1);
    // Every visible layer has a box for the screen-reader list.
    assert!(call(&mut s, "layer_boxes", json!({})).as_array().unwrap().len() >= 8);
    // Water filling the screen; a camera made active, shaking.
    let w = call(&mut s, "add_water", json!({"fill": "screen"})).as_u64().unwrap();
    assert_eq!(call(&mut s, "layer", json!({"id": w}))["kind"], "water");
    let c = call(&mut s, "add_camera", json!({})).as_u64().unwrap();
    call(&mut s, "camera_shake", json!({"id": c, "shake": true}));
    assert!(call(&mut s, "layer_full", json!({"id": c}))["layer"]["shake"].is_object());
    assert_eq!(call(&mut s, "camera_info", json!({"id": c}))["active"], true);
    // A 3D torus: its shape and thickness change.
    let solid = call(&mut s, "add_solid", json!({"shape": "Cube"}));
    let sid = solid.as_u64().or_else(|| solid["id"].as_u64()).unwrap();
    call(&mut s, "solid_op", json!({"id": sid, "shape": "Torus"}));
    call(&mut s, "solid_op", json!({"id": sid, "thickness": 0.2}));
    let info = call(&mut s, "solid_info", json!({"id": sid}));
    assert_eq!(info["shape"], "Torus");
    assert!((info["thickness"].as_f64().unwrap() - 0.2).abs() < 1e-9);
    // Confetti whose pieces are an emoji: one undo step takes away both layers.
    let before = call(&mut s, "state", json!({}))["layers"].as_array().unwrap().len();
    call(&mut s, "emoji_confetti", json!({"glyph": "🎉"}));
    assert!(call(&mut s, "state", json!({}))["layers"].as_array().unwrap().len() > before);
    call(&mut s, "undo", json!({}));
    assert_eq!(call(&mut s, "state", json!({}))["layers"].as_array().unwrap().len(), before);
    // How many parent joints a layer bends.
    call(&mut s, "set_meta", json!({"id": ids[0], "ik_chain": 3}));
    assert_eq!(call(&mut s, "layer", json!({"id": ids[0]}))["ik_chain"], 3);
    // A diagram symbol opens for editing.
    let d = call(&mut s, "new_diagram_symbol", json!({}));
    assert_eq!(d["name"], "Diagram 1");
    assert_eq!(call(&mut s, "state", json!({}))["comp"]["kind"], "diagram");
}

#[test]
fn duplicate_copies_groups_and_repeats_the_last_move() {
    let mut s = Session::new(Project::new());
    let a = call(&mut s, "add_shape", json!({"kind": "rect", "x": 0, "y": 0, "w": 50, "h": 50})).as_u64().unwrap();
    let b = call(&mut s, "add_shape", json!({"kind": "rect", "x": 100, "y": 0, "w": 50, "h": 50})).as_u64().unwrap();
    call(&mut s, "select", json!({"ids": [a, b]}));
    call(&mut s, "group", json!({}));
    let n0 = call(&mut s, "state", json!({}))["layers"].as_array().unwrap().len();
    call(&mut s, "duplicate", json!({}));
    // The group comes with both its members.
    assert_eq!(call(&mut s, "state", json!({}))["layers"].as_array().unwrap().len(), n0 * 2);
    // A shape moved after duplicating: Duplicate again moves the next copy as far again.
    let c = call(&mut s, "add_shape", json!({"kind": "rect", "x": 0, "y": 300, "w": 50, "h": 50})).as_u64().unwrap();
    call(&mut s, "select", json!({"ids": [c]}));
    call(&mut s, "duplicate", json!({}));
    let copy = call(&mut s, "state", json!({}))["selection"][0].as_u64().unwrap();
    call(&mut s, "set_prop", json!({"id": copy, "prop": "position", "value": [125.0, 325.0]}));
    let r = call(&mut s, "duplicate", json!({}));
    assert_eq!(r["again"], true);
    let third = call(&mut s, "state", json!({}))["selection"][0].as_u64().unwrap();
    let pos = call(&mut s, "layer", json!({"id": third}))["props"]["position"]["value"].clone();
    assert_eq!(pos, json!([225.0, 325.0]));
}

#[test]
fn shapes_go_into_the_layer_you_are_on() {
    let mut s = Session::default();
    call(&mut s, "new", json!({"width": 800, "height": 600}));
    let st = call(&mut s, "state", json!({}));
    let l1 = st["active_layer"].as_u64().expect("a new canvas starts on Layer 1");
    assert_eq!(st["layers"][0]["name"], "Layer 1");
    assert_eq!(st["layers"][0]["folder"], true);
    let a = call(&mut s, "add_shape", json!({"x": 100, "y": 100, "w": 50, "h": 50})).as_u64().unwrap();
    let b = call(&mut s, "add_shape", json!({"x": 300, "y": 100, "w": 50, "h": 50})).as_u64().unwrap();
    let st = call(&mut s, "state", json!({}));
    assert_eq!(st["layers"][0]["members"], 2);
    // Inside the layer, newest first, and where they were drawn.
    assert_eq!(st["layers"][1]["id"], json!(b));
    assert_eq!(st["layers"][1]["depth"], 1);
    assert_eq!(call(&mut s, "layer", json!({"id": a}))["props"]["position"]["value"], json!([125.0, 125.0]));
    // A click picks the shape, never the whole layer.
    assert_eq!(call(&mut s, "hit", json!({"x": 120, "y": 120, "group": true})), json!(a));
    // A second layer: new shapes go there; picking a shape of Layer 1 goes back to it.
    let l2 = call(&mut s, "add_folder", json!({})).as_u64().unwrap();
    let c = call(&mut s, "add_shape", json!({"x": 500, "y": 300, "w": 50, "h": 50})).as_u64().unwrap();
    assert_eq!(call(&mut s, "layer_full", json!({"id": c}))["layer"]["parent"], json!(l2));
    call(&mut s, "select", json!({"ids": [a]}));
    assert_eq!(call(&mut s, "state", json!({}))["active_layer"], json!(l1));
    // Undoing a drawing that was selected leaves nothing stale selected.
    call(&mut s, "select", json!({"ids": [c]}));
    call(&mut s, "undo", json!({}));
    assert_eq!(call(&mut s, "state", json!({}))["selection"], json!([]));
}

#[test]
fn an_opened_canvas_starts_on_its_top_layer() {
    let mut s = Session::default();
    call(&mut s, "new", json!({"width": 800, "height": 600}));
    let l1 = call(&mut s, "state", json!({}))["active_layer"].clone();
    let saved = call(&mut s, "save", json!({}));
    let mut t = Session::default();
    call(&mut t, "open", json!({"json": saved}));
    assert_eq!(call(&mut t, "state", json!({}))["active_layer"], l1);
    let a = call(&mut t, "add_shape", json!({"x": 100, "y": 100, "w": 50, "h": 50})).as_u64().unwrap();
    assert_eq!(call(&mut t, "layer_full", json!({"id": a}))["layer"]["parent"], l1);
}

#[test]
fn knife_keeps_stacking_and_layer_and_previews_its_cut() {
    let mut s = Session::default();
    call(&mut s, "new", json!({"width": 800, "height": 600}));
    let low = call(&mut s, "add_shape", json!({"x": 100, "y": 100, "w": 200, "h": 100})).as_u64().unwrap();
    let high = call(&mut s, "add_shape", json!({"x": 120, "y": 120, "w": 200, "h": 100})).as_u64().unwrap();
    let names = |s: &mut Session| {
        call(s, "state", json!({}))["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let before = names(&mut s);
    call(&mut s, "select", json!({"ids": []}));
    let prev = call(&mut s, "knife_preview", json!({"a": [200, 50], "b": [200, 300]}));
    assert_eq!(prev.as_array().unwrap().len(), 2);
    let cut = &prev[0]["cuts"][0];
    assert!((cut[0][1].as_f64().unwrap() - 100.0).abs() < 1.0 || (cut[0][1].as_f64().unwrap() - 120.0).abs() < 1.0);
    assert_eq!(call(&mut s, "knife", json!({"a": [200, 50], "b": [200, 300]})), json!(4));
    let after = names(&mut s);
    let piece = |n: &String, of: &String| n.starts_with(&format!("{of} ("));
    let hi = after.iter().position(|n| piece(n, &before[1])).unwrap();
    let lo = after.iter().position(|n| piece(n, &before[2])).unwrap();
    assert!(hi < lo, "the top shape's pieces stay above: {after:?}");
    assert!(piece(&after[hi + 1], &before[1]), "pieces stay together: {after:?}");
    // Still in Layer 1.
    let st = call(&mut s, "state", json!({}));
    assert_eq!(st["layers"][0]["members"], 4);
    let _ = (low, high);
}

#[test]
fn new_canvases_are_see_through_and_exports_can_put_a_colour_behind() {
    let mut s = Session::default();
    call(&mut s, "new", json!({"width": 400, "height": 300}));
    assert!(s.scene().background.a < 0.01, "see-through to begin with");
    call(&mut s, "export_mode", json!({"on": true, "background": "#000000"}));
    let bg = s.scene().background;
    assert!(bg.a > 0.99 && bg.r < 0.01, "black behind it while exporting");
    let copy = call(&mut s, "save_copy", json!({"background": "#ffffff"}));
    assert!(copy.as_str().unwrap().contains("background"));
    call(&mut s, "export_mode", json!({"on": false}));
    assert!(s.scene().background.a < 0.01, "and see-through again after");
    // Slides start white.
    call(&mut s, "new", json!({"width": 1920, "height": 1080, "deck": true}));
    assert!(s.scene().background.a > 0.99);
}

#[test]
fn a_group_in_a_layer_can_be_picked_moved_scaled_and_turned() {
    let mut s = Session::default();
    call(&mut s, "new", json!({"width": 1200, "height": 800}));
    let a = call(&mut s, "add_shape", json!({"x": 100, "y": 100, "w": 200, "h": 150})).as_u64().unwrap();
    let b = call(&mut s, "add_shape", json!({"x": 400, "y": 150, "w": 200, "h": 150})).as_u64().unwrap();
    call(&mut s, "select", json!({"ids": [a, b]}));
    call(&mut s, "group", json!({}));
    let g = call(&mut s, "state", json!({}))["selection"][0].as_u64().unwrap();
    assert_eq!(call(&mut s, "hit", json!({"x": 150, "y": 150, "group": true})), json!(g));
    let box_of = |s: &mut Session| {
        let c = call(s, "bounds", json!({"ids": [g]}))[0]["corners"].clone();
        (c[0][0].as_f64().unwrap(), c[0][1].as_f64().unwrap(), c[2][0].as_f64().unwrap(), c[2][1].as_f64().unwrap())
    };
    assert_eq!(box_of(&mut s), (100.0, 100.0, 600.0, 300.0));
    // Moved by 50, 20.
    call(&mut s, "select", json!({"ids": [g]}));
    call(&mut s, "transform_begin", json!({}));
    call(&mut s, "transform", json!({"affine": [1, 0, 0, 1, 50, 20]}));
    call(&mut s, "transform_end", json!({}));
    let (x0, y0, x1, y1) = box_of(&mut s);
    assert!(
        (x0 - 150.0).abs() < 1e-6
            && (y0 - 120.0).abs() < 1e-6
            && (x1 - 650.0).abs() < 1e-6
            && (y1 - 320.0).abs() < 1e-6
    );
    // Scaled to twice the width about its left edge.
    call(&mut s, "transform_begin", json!({}));
    call(&mut s, "transform", json!({"affine": [2, 0, 0, 1, -150, 0]}));
    call(&mut s, "transform_end", json!({}));
    let (x0, _, x1, _) = box_of(&mut s);
    assert!((x0 - 150.0).abs() < 1e-6 && (x1 - 1150.0).abs() < 1e-6, "{x0} {x1}");
    // Turned a quarter: its box is now taller than wide.
    let (cx, cy) = (650.0, 220.0);
    call(&mut s, "transform_begin", json!({}));
    call(&mut s, "transform", json!({"affine": [0, 1, -1, 0, cx + cy, cy - cx]}));
    call(&mut s, "transform_end", json!({}));
    let (x0, y0, x1, y1) = box_of(&mut s);
    assert!((y1 - y0) > (x1 - x0), "turned: {x0} {y0} {x1} {y1}");
}

#[test]
fn a_picture_the_canvas_size_fills_it_inside_the_layer() {
    let mut s = Session::new(Project::new());
    call(&mut s, "new", json!({"width": 2, "height": 1}));
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGP4z8DwHwQZYAQAZsQH+ZgV3QAAAABJRU5ErkJggg==";
    let id =
        call(&mut s, "add_picture", json!({"base64": png, "name": "x.png", "width": 2, "height": 1})).as_u64().unwrap();
    let st = call(&mut s, "state", json!({}));
    let row = st["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(id)).cloned().unwrap();
    assert_eq!(row["depth"], 1, "in Layer 1");
    let info = call(&mut s, "layer", json!({"id": id}));
    assert_eq!(info["props"]["scale"]["value"], json!([1.0, 1.0]));
}

#[test]
fn typing_on_the_canvas_places_the_caret_and_grows_the_box() {
    let mut s = Session::new(Project::new());
    call(&mut s, "new", json!({"width": 800, "height": 600}));
    let t = call(&mut s, "add_text", json!({"x": 100, "y": 100, "text": "Hello", "box": [200, 60], "size": 24}))
        .as_u64()
        .unwrap();
    // The caret after "He" sits right of the one before it, on the same line.
    let c0 = call(&mut s, "text_caret", json!({"id": t, "anchor": 0, "focus": 0}));
    let c2 = call(&mut s, "text_caret", json!({"id": t, "anchor": 2, "focus": 2}));
    let x = |c: &Json| c["caret"][0][0].as_f64().unwrap();
    assert!(x(&c2) > x(&c0) + 5.0, "{c0} {c2}");
    // A click on that spot picks it back.
    let p = call(
        &mut s,
        "text_pick",
        json!({"id": t, "x": x(&c2) + 0.5, "y": c2["caret"][0][1].as_f64().unwrap() + 5.0, "unit": 1}),
    );
    assert_eq!(p["focus"], 2);
    // A double-click picks the word.
    let w = call(
        &mut s,
        "text_pick",
        json!({"id": t, "x": x(&c2), "y": c2["caret"][0][1].as_f64().unwrap() + 5.0, "unit": 2}),
    );
    assert_eq!((w["anchor"].as_u64(), w["focus"].as_u64()), (Some(0), Some(5)));
    // A selection gives a rectangle.
    assert_eq!(
        call(&mut s, "text_caret", json!({"id": t, "anchor": 0, "focus": 5}))["selection"].as_array().unwrap().len(),
        1
    );
    // Typing more lines than fit: the box grows downwards, its width stays.
    let before = call(&mut s, "text_box", json!({"id": t}))["frame"].clone();
    call(&mut s, "set_text", json!({"id": t, "text": "Hello\nthere\nmore\nlines"}));
    let after = call(&mut s, "text_box", json!({"id": t}))["frame"].clone();
    assert_eq!(before[2], after[2]);
    assert!(after[3].as_f64().unwrap() > before[3].as_f64().unwrap() + 20.0, "{before} {after}");
    // Down a line from the start lands on the second line.
    let d = call(&mut s, "text_step", json!({"id": t, "anchor": 0, "focus": 0, "lines": 1}));
    assert_eq!(d["focus"], 6);
    assert_eq!(call(&mut s, "text_step", json!({"id": t, "anchor": 6, "focus": 6, "edge": 1}))["focus"], 11);
    // A shape's label is typed in the same way, and is checked for spelling.
    let r = call(&mut s, "add_shape", json!({"x": 400, "y": 300, "w": 200, "h": 100})).as_u64().unwrap();
    call(&mut s, "set_label", json!({"id": r, "text": "Clas name"}));
    assert!(call(&mut s, "text_caret", json!({"id": r, "anchor": 4, "focus": 4}))["caret"].is_array());
    assert!(
        call(&mut s, "texts", json!({}))
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == json!(r) && x["label"] == json!(true))
    );
}

/// Pixel (`x`, `y`) of picture layer `id`'s file.
fn pixel(s: &Session, id: u64, x: u32, y: u32) -> [u8; 4] {
    let l = s.project().layer(Id(id)).unwrap();
    let LayerKind::Image { asset, .. } = l.kind else { panic!("not a picture") };
    let a = s.project().asset(asset).unwrap();
    image::load_from_memory(&a.data).unwrap().into_rgba8().get_pixel(x, y).0
}

#[test]
fn raster_layers_paint_erase_and_undo() {
    let mut s = Session::new(Project::new());
    let st = call(&mut s, "state", json!({}));
    let cw = st["comp"]["width"].as_f64().unwrap();
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let info = call(&mut s, "raster_info", json!({"id": id}));
    assert_eq!(info["paintable"], true);
    let d = info["px_per_unit"].as_f64().unwrap();
    assert_eq!(info["width"].as_f64().unwrap(), (cw * d).round());
    let st = call(&mut s, "state", json!({}));
    assert!(st["layers"].as_array().unwrap().iter().any(|l| l["id"] == id && l["raster"] == true));

    // A red stroke across the middle.
    call(&mut s, "raster_begin", json!({"id": id, "color": "#ff0000", "size": 20, "hardness": 1, "pressure": false}));
    call(&mut s, "raster_paint", json!({"points": [[100, 100, 0.5], [300, 100, 0.5]]}));
    call(&mut s, "raster_end", json!({}));
    let at = |v: f64| (v * d) as u32;
    assert_eq!(pixel(&s, id, at(200.0), at(100.0)), [255, 0, 0, 255]);
    assert_eq!(pixel(&s, id, at(200.0), at(140.0))[3], 0);
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Paint");

    // Erasing half of it, then undoing the erase.
    call(&mut s, "raster_begin", json!({"id": id, "erase": true, "size": 40, "hardness": 1, "pressure": false}));
    call(&mut s, "raster_paint", json!({"points": [[100, 100], [200, 100]]}));
    call(&mut s, "raster_end", json!({}));
    assert_eq!(pixel(&s, id, at(150.0), at(100.0))[3], 0);
    assert_eq!(pixel(&s, id, at(290.0), at(100.0))[3], 255);
    call(&mut s, "undo", json!({}));
    assert_eq!(pixel(&s, id, at(150.0), at(100.0)), [255, 0, 0, 255]);
    // The undone pixels are what the next stroke starts from.
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "color": "#0000ff", "opacity": 0.5, "size": 10, "hardness": 1, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[150, 100], [150, 101], [150, 102]]}));
    call(&mut s, "raster_end", json!({}));
    let p = pixel(&s, id, at(150.0), at(100.0));
    assert!(
        (p[0] as i32 - 128).abs() <= 2 && (p[2] as i32 - 128).abs() <= 2 && p[3] == 255,
        "half blue over red, not built up: {p:?}"
    );
}

#[test]
fn raster_fill_wand_and_selection() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let d = call(&mut s, "raster_info", json!({"id": id}))["px_per_unit"].as_f64().unwrap();
    let at = |v: f64| (v * d) as u32;
    // A closed black ring, then the bucket inside it.
    let ring: Vec<Json> = (0..=64)
        .map(|i| {
            let t = i as f64 / 64.0 * std::f64::consts::TAU;
            json!([300.0 + 100.0 * t.cos(), 300.0 + 100.0 * t.sin()])
        })
        .collect();
    call(&mut s, "raster_begin", json!({"id": id, "color": "#000000", "size": 6, "hardness": 1, "pressure": false}));
    call(&mut s, "raster_paint", json!({"points": ring}));
    call(&mut s, "raster_end", json!({}));
    call(&mut s, "raster_fill", json!({"id": id, "x": 300, "y": 300, "color": "#00ff00", "tolerance": 10, "grow": 2}));
    assert_eq!(pixel(&s, id, at(300.0), at(300.0)), [0, 255, 0, 255]);
    assert_eq!(pixel(&s, id, at(500.0), at(300.0))[3], 0, "the fill stays inside the ring");
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Fill");

    // The wand picks the green inside; deleting it leaves the ring.
    let b = call(&mut s, "raster_wand", json!({"id": id, "x": 300, "y": 300, "tolerance": 10}));
    assert!(b.is_array());
    assert!(!call(&mut s, "raster_outline", json!({"id": id})).as_array().unwrap().is_empty());
    call(&mut s, "raster_clear", json!({"id": id}));
    assert_eq!(pixel(&s, id, at(300.0), at(300.0))[3], 0);
    assert_eq!(pixel(&s, id, at(400.0), at(300.0))[3], 255);

    // Painting stays inside a rectangle selection; subtracting all of it leaves nothing.
    call(&mut s, "raster_select", json!({"id": id, "shape": "rect", "rect": [600, 100, 700, 200]}));
    call(&mut s, "raster_begin", json!({"id": id, "color": "#ff0000", "size": 30, "hardness": 1, "pressure": false}));
    call(&mut s, "raster_paint", json!({"points": [[550, 150], [750, 150]]}));
    call(&mut s, "raster_end", json!({}));
    assert_eq!(pixel(&s, id, at(650.0), at(150.0))[3], 255);
    assert_eq!(pixel(&s, id, at(580.0), at(150.0))[3], 0);
    call(&mut s, "raster_invert", json!({"id": id}));
    call(&mut s, "raster_invert", json!({"id": id}));
    let nothing = call(
        &mut s,
        "raster_select",
        json!({"id": id, "shape": "rect", "rect": [0, 0, 4000, 4000], "mode": "subtract"}),
    );
    assert!(nothing.is_null());
    // A new layer from a selection.
    call(&mut s, "raster_select", json!({"id": id, "shape": "ellipse", "rect": [600, 100, 700, 200]}));
    let piece = call(&mut s, "raster_layer_from_selection", json!({"id": id, "cut": true})).as_u64().unwrap();
    assert_eq!(pixel(&s, piece, at(650.0), at(150.0))[3], 255);
    assert_eq!(pixel(&s, id, at(650.0), at(150.0))[3], 0);
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Cut to a new layer");
}

#[test]
fn raster_selection_undo_fits_between_drawing_steps() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let sel = |s: &mut Session| call(s, "raster_info", json!({"id": id}))["selection"].clone();
    call(&mut s, "raster_select", json!({"id": id, "shape": "rect", "rect": [10, 10, 50, 50]}));
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Select");
    call(&mut s, "raster_fill_selection", json!({"id": id, "color": "#ff0000"}));
    call(&mut s, "raster_invert", json!({"id": id}));
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Invert selection");
    // Undo: the inversion, then the fill, then the rectangle.
    assert_eq!(call(&mut s, "undo", json!({})), "Invert selection");
    let rect = sel(&mut s);
    assert!(rect.is_array());
    assert_eq!(call(&mut s, "undo", json!({})), "Fill");
    assert_eq!(sel(&mut s), rect, "undoing the fill keeps the selection");
    assert_eq!(call(&mut s, "undo", json!({})), "Select");
    assert!(sel(&mut s).is_null());
    // Redo in the same order.
    assert_eq!(call(&mut s, "redo", json!({})), "Select");
    assert_eq!(sel(&mut s), rect);
    assert_eq!(call(&mut s, "redo", json!({})), "Fill");
    assert_eq!(call(&mut s, "state", json!({}))["redo"], "Invert selection");
    // A new step drops what there was to redo.
    call(&mut s, "raster_select_none", json!({"id": id}));
    assert!(call(&mut s, "state", json!({}))["redo"].is_null());
    assert_eq!(call(&mut s, "undo", json!({})), "Select none");
    assert_eq!(sel(&mut s), rect);
}

#[test]
fn raster_clone_smudge_and_blur_brushes() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let d = call(&mut s, "raster_info", json!({"id": id}))["px_per_unit"].as_f64().unwrap();
    let at = |v: f64| (v * d) as u32;
    // A red square on the left half of a box, see-through on the right.
    call(&mut s, "raster_select", json!({"id": id, "shape": "rect", "rect": [100, 100, 200, 200]}));
    call(&mut s, "raster_fill_selection", json!({"id": id, "color": "#ff0000"}));
    call(&mut s, "raster_select_none", json!({"id": id}));

    // Clone: paint the square's pixels 300 to the right.
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "brush": "clone", "clone_offset": [-300, 0], "size": 20, "hardness": 1, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[430, 150], [470, 150]]}));
    call(&mut s, "raster_end", json!({}));
    assert_eq!(pixel(&s, id, at(450.0), at(150.0)), [255, 0, 0, 255]);
    assert_eq!(pixel(&s, id, at(450.0), at(180.0))[3], 0, "only under the brush");
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Clone");

    // Smudge: drag red out of the square into the empty part.
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "brush": "smudge", "size": 20, "hardness": 0.5, "opacity": 0.8, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[180, 150], [215, 150]]}));
    call(&mut s, "raster_end", json!({}));
    let p = pixel(&s, id, at(212.0), at(150.0));
    assert!(p[3] > 40 && p[0] > 200, "red dragged outside: {p:?}");
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Smudge");

    // Blur: the square's bottom edge goes soft.
    assert_eq!(pixel(&s, id, at(150.0), at(198.0))[3], 255);
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "brush": "blur", "size": 30, "hardness": 0.5, "opacity": 1, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[130, 200], [170, 200]]}));
    call(&mut s, "raster_end", json!({}));
    let (inside, outside) = (pixel(&s, id, at(150.0), at(198.0))[3], pixel(&s, id, at(150.0), at(202.0))[3]);
    assert!(inside < 255 && outside > 0, "soft edge: {inside} {outside}");
    assert_eq!(pixel(&s, id, at(150.0), at(150.0)), [255, 0, 0, 255], "the middle stays");
    assert_eq!(call(&mut s, "undo", json!({})), "Blur");
    assert_eq!(pixel(&s, id, at(150.0), at(202.0))[3], 0);
}

#[test]
fn raster_move_gradient_adjust_mask_copy_resize_extend() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let d = call(&mut s, "raster_info", json!({"id": id}))["px_per_unit"].as_f64().unwrap();
    let at = |v: f64| (v * d) as u32;
    let png_b64 = |img: &image::RgbaImage| {
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png)
    };
    // A red square, selected, moved 300 to the right and doubled in size.
    call(&mut s, "raster_select", json!({"id": id, "shape": "rect", "rect": [100, 100, 200, 200]}));
    call(&mut s, "raster_fill_selection", json!({"id": id, "color": "#ff0000"}));
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 150, "y": 150})), true);
    call(&mut s, "raster_float_begin", json!({"id": id}));
    call(&mut s, "raster_float_set", json!({"dx": 300, "dy": 0, "sx": 2, "sy": 2}));
    assert!(s.raster_take_dirty().is_some(), "shown live");
    call(&mut s, "raster_live_end", json!({"keep": true}));
    assert_eq!(pixel(&s, id, at(150.0), at(150.0))[3], 0, "lifted from its old place");
    assert_eq!(pixel(&s, id, at(410.0), at(60.0)), [255, 0, 0, 255], "scaled about the centre (450, 150): 350..550");
    assert_eq!(pixel(&s, id, at(345.0), at(150.0))[3], 0);
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Move pixels");
    let moved = call(&mut s, "raster_info", json!({"id": id}))["selection"].clone();
    assert!(moved[0].as_f64().unwrap() > at(340.0) as f64, "the selection moved along: {moved}");
    // Cancelling a move puts everything back.
    call(&mut s, "raster_float_begin", json!({"id": id}));
    call(&mut s, "raster_float_set", json!({"dx": -300, "dy": 300}));
    call(&mut s, "raster_live_end", json!({"keep": false}));
    assert_eq!(pixel(&s, id, at(410.0), at(60.0)), [255, 0, 0, 255]);

    // Colour adjustments: invert inside the selection only.
    call(&mut s, "raster_adjust", json!({"id": id, "invert": true, "preview": true}));
    assert_eq!(
        call(&mut s, "raster_pick", json!({"id": id, "x": 450, "y": 150})).as_str().map(str::to_lowercase).as_deref(),
        Some("#00ffff"),
        "the preview"
    );
    call(&mut s, "raster_live_end", json!({"keep": false}));
    assert_eq!(pixel(&s, id, at(450.0), at(150.0)), [255, 0, 0, 255]);
    call(&mut s, "raster_adjust", json!({"id": id, "hue": 120}));
    assert_eq!(pixel(&s, id, at(450.0), at(150.0)), [0, 255, 0, 255]);
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Adjust colours");

    // A gradient from black to see-through across everything.
    call(&mut s, "raster_select_none", json!({"id": id}));
    call(
        &mut s,
        "raster_gradient",
        json!({"id": id, "from": [0, 900], "to": [1920, 900], "color": "#000000", "color2": "none"}),
    );
    let (l, r) = (pixel(&s, id, at(10.0), at(900.0)), pixel(&s, id, at(1900.0), at(900.0)));
    assert!(l[3] > 240 && r[3] < 15, "{l:?} {r:?}");

    // Select with AI: a mask, white on the right half.
    let (w, h) = (at(1920.0), at(1080.0));
    let mask = image::RgbaImage::from_fn(w / 4, h / 4, |x, _| {
        if x >= w / 8 { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([0, 0, 0, 255]) }
    });
    call(&mut s, "raster_select_mask", json!({"id": id, "base64": png_b64(&mask)}));
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 1500, "y": 500})), true);
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 400, "y": 500})), false);
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Select with AI");

    // Copy: cropped to the selection, with its place on the canvas.
    call(&mut s, "raster_select", json!({"id": id, "shape": "rect", "rect": [350, 50, 550, 250]}));
    let c = call(&mut s, "raster_copy", json!({"id": id}));
    assert!((c["x"].as_f64().unwrap() - 450.0).abs() < 2.0 && c["width"].as_u64().unwrap() >= at(199.0) as u64);

    // Picture size: half the pixels, the same place and size on the canvas.
    let before = call(&mut s, "bounds", json!({"ids": [id]}));
    call(&mut s, "raster_resize", json!({"id": id, "width": w / 2, "height": h / 2}));
    assert_eq!(call(&mut s, "raster_info", json!({"id": id}))["width"], json!(w / 2));
    assert_eq!(call(&mut s, "bounds", json!({"ids": [id]})), before, "still covers the same area");
    call(&mut s, "undo", json!({}));
    assert_eq!(call(&mut s, "raster_info", json!({"id": id}))["width"], json!(w));

    // Extend: 100 more pixels on the left, selected; the old ones stay where they were on the canvas.
    let left_before = pixel(&s, id, at(10.0), at(900.0));
    call(&mut s, "raster_extend", json!({"id": id, "left": 100}));
    assert_eq!(call(&mut s, "raster_info", json!({"id": id}))["width"], json!(w + 100));
    assert_eq!(pixel(&s, id, at(10.0) + 100, at(900.0)), left_before);
    assert_eq!(
        call(&mut s, "raster_sel_at", json!({"id": id, "x": -50.0 / d, "y": 500})),
        true,
        "the new part is selected"
    );
}

#[test]
fn raster_paint_the_selection_and_clone_from_all_layers() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    // The brush paints the selection, the eraser takes from it; one undo step each.
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "target": "selection", "size": 40, "hardness": 1, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[100, 100], [300, 100]]}));
    call(&mut s, "raster_end", json!({}));
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 200, "y": 100})), true);
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "Paint the selection");
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "target": "selection", "erase": true, "size": 40, "hardness": 1, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[200, 100], [201, 100]]}));
    call(&mut s, "raster_end", json!({}));
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 200, "y": 100})), false);
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 120, "y": 100})), true);
    call(&mut s, "undo", json!({}));
    assert_eq!(call(&mut s, "raster_sel_at", json!({"id": id, "x": 200, "y": 100})), true);
    call(&mut s, "raster_select_none", json!({"id": id}));

    // Clone from what the canvas shows (another layer's pixels, given as the sample).
    let info = call(&mut s, "raster_info", json!({"id": id}));
    let (w, h) = (info["width"].as_u64().unwrap() as u32, info["height"].as_u64().unwrap() as u32);
    let mut sample = vec![0u8; (w * h * 4) as usize];
    for p in sample.as_chunks_mut::<4>().0 {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }
    s.raster_set_sample(Id(id), sample);
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "brush": "clone", "clone_offset": [10, 0], "sample": "canvas", "size": 20, "hardness": 1, "pressure": false}),
    );
    call(&mut s, "raster_paint", json!({"points": [[500, 500], [520, 500]]}));
    call(&mut s, "raster_end", json!({}));
    let d = info["px_per_unit"].as_f64().unwrap();
    assert_eq!(pixel(&s, id, (510.0 * d) as u32, (500.0 * d) as u32), [0, 0, 255, 255]);

    // Pen pressure on opacity: a light press paints lighter.
    call(
        &mut s,
        "raster_begin",
        json!({"id": id, "color": "#000000", "size": 20, "hardness": 1, "pressure": false, "pressure_opacity": true}),
    );
    call(&mut s, "raster_paint", json!({"points": [[800, 800, 0.1]]}));
    call(&mut s, "raster_end", json!({}));
    let a = pixel(&s, id, (800.0 * d) as u32, (800.0 * d) as u32)[3];
    assert!(a > 20 && a < 80, "light: {a}");
}

#[test]
fn raster_drawing_frames_and_onion_skin() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    call(&mut s, "raster_select", json!({"id": id, "shape": "rect", "rect": [100, 100, 200, 200]}));
    call(&mut s, "raster_fill_selection", json!({"id": id, "color": "#ff0000"}));
    call(&mut s, "raster_select_none", json!({"id": id}));
    // At the start there's nothing before: no new frame there.
    assert!(s.call("raster_new_frame", &json!({"id": id})).is_err());
    call(&mut s, "set_time", json!({"time": 0.5}));
    let f2 = call(&mut s, "raster_new_frame", json!({"id": id, "copy": true})).as_u64().unwrap();
    let st = call(&mut s, "state", json!({}));
    let row = |id: u64| st["layers"].as_array().unwrap().iter().find(|l| l["id"] == id).cloned().unwrap();
    assert_eq!(row(id)["out"], json!(0.5), "the first frame ends where the new one starts: {}", row(id));
    assert_eq!(row(f2)["in"], json!(0.5));
    assert_eq!(pixel(&s, f2, 150, 150), [255, 0, 0, 255], "a copy of the frame before");
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "New drawing frame");
    // The onion skin shows the frame before under the new one, faint, on screen only.
    call(&mut s, "raster_onion", json!({"on": true}));
    let mut scene = s.scene();
    assert!(scene.layer(Id(id)).is_none(), "the first frame is over at 0.5");
    s.raster_onion_skin(&mut scene);
    let ghost = scene.layer(Id(id)).expect("shown faintly");
    assert!((ghost.opacity - 0.3).abs() < 1e-6);
    call(&mut s, "undo", json!({}));
    assert!(ids_now(&mut s).len() == 1);
}

#[test]
fn raster_swap_and_ai_blend() {
    let mut s = Session::new(Project::new());
    let a =
        call(&mut s, "add_shape", json!({"kind": "rect", "x": 100, "y": 100, "w": 200, "h": 100})).as_u64().unwrap();
    let b =
        call(&mut s, "add_shape", json!({"kind": "ellipse", "x": 400, "y": 100, "w": 100, "h": 100})).as_u64().unwrap();
    let (scene, w, h) = s.raster_scene(&[Id(a), Id(b)]).unwrap();
    assert_eq!(scene.layers.len(), 2);
    // What the GPU would have drawn: half-transparent red everywhere (premultiplied).
    let px: Vec<u8> = (0..w * h).flat_map(|_| [128u8, 0, 0, 128]).collect();
    let r = s.raster_swap(&[Id(a), Id(b)], w, h, px, None).unwrap().as_u64().unwrap();
    let st = call(&mut s, "state", json!({}));
    let ids = ids(&st);
    assert!(!ids.contains(&a) && !ids.contains(&b) && ids.contains(&r));
    assert_eq!(pixel(&s, r, 5, 5), [255, 0, 0, 128]);
    call(&mut s, "undo", json!({}));
    assert!(ids_now(&mut s).contains(&a));
    call(&mut s, "redo", json!({}));

    // An AI answer, put in only inside the selection.
    call(&mut s, "raster_select", json!({"id": r, "shape": "rect", "rect": [0, 0, 100, 100]}));
    let inputs = call(&mut s, "raster_ai_inputs", json!({"id": r, "max": 256}));
    assert!(inputs["mask"].is_string() && inputs["image"].is_string());
    let blue = image::RgbaImage::from_pixel(64, 36, image::Rgba([0, 0, 255, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    blue.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png.into_inner());
    call(&mut s, "raster_ai_apply", json!({"id": r, "base64": b64, "feather": 0}));
    let d = call(&mut s, "raster_info", json!({"id": r}))["px_per_unit"].as_f64().unwrap();
    let p = pixel(&s, r, (50.0 * d) as u32, (50.0 * d) as u32);
    assert_eq!(p, [0, 0, 255, 255]);
    assert_eq!(
        pixel(&s, r, (500.0 * d) as u32, (500.0 * d) as u32),
        [255, 0, 0, 128],
        "outside the selection stays as it was"
    );
    assert_eq!(call(&mut s, "state", json!({}))["undo"], "AI change");
}

fn ids_now(s: &mut Session) -> Vec<u64> {
    ids(&call(s, "state", json!({})))
}

#[test]
fn raster_stroke_goes_on_over_a_change_that_came_in_meanwhile() {
    let mut s = Session::new(Project::new());
    let id = call(&mut s, "add_raster_layer", json!({})).as_u64().unwrap();
    let d = call(&mut s, "raster_info", json!({"id": id}))["px_per_unit"].as_f64().unwrap();
    let at = |v: f64| (v * d) as u32;
    call(&mut s, "raster_begin", json!({"id": id, "color": "#ff0000", "size": 20, "hardness": 1, "pressure": false}));
    call(&mut s, "raster_paint", json!({"points": [[100, 150], [200, 150]]}));
    call(&mut s, "raster_end", json!({}));
    // A stroke starts; the red one is taken back meanwhile (as someone else's change would come in).
    call(&mut s, "raster_begin", json!({"id": id, "color": "#0000ff", "size": 20, "hardness": 1, "pressure": false}));
    call(&mut s, "raster_paint", json!({"points": [[150, 500], [250, 500]]}));
    assert_eq!(call(&mut s, "undo", json!({})), "Paint");
    call(&mut s, "raster_paint", json!({"points": [[350, 500]]}));
    call(&mut s, "raster_end", json!({}));
    assert_eq!(pixel(&s, id, at(150.0), at(150.0))[3], 0, "the change that came in stays");
    assert_eq!(pixel(&s, id, at(180.0), at(500.0)), [0, 0, 255, 255], "the stroke's start is kept");
    assert_eq!(pixel(&s, id, at(330.0), at(500.0)), [0, 0, 255, 255]);

    // The picture's size changing meanwhile stops the stroke (it doesn't paint off its pixels).
    call(&mut s, "raster_extend", json!({"id": id, "left": 300, "bottom": 300}));
    call(&mut s, "undo", json!({}));
    call(&mut s, "raster_begin", json!({"id": id, "size": 30, "pressure": false}));
    call(&mut s, "redo", json!({}));
    assert!(s.call("raster_paint", &json!({"points": [[10, 10], [1900, 1300]]})).is_err());
    assert!(call(&mut s, "raster_end", json!({})).is_null());
}

/// On a server, a picture goes as its name: the change and the saved
/// version carry its hash, and the bytes go up beside them once.
#[test]
fn pictures_on_a_server_go_by_name() {
    use base64::Engine;
    let mut s = Session::new(Project::new());
    call(&mut s, "new", json!({"width": 2, "height": 1}));
    call(&mut s, "remote_open", json!({"server": "s", "node": 5, "name": "Pic", "data": ""}));
    call(&mut s, "live_start", json!({"seq": 0, "ops": [], "conn": "c", "from": 1000, "to": 2000, "role": "editor"}));
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGP4z8DwHwQZYAQAZsQH+ZgV3QAAAABJRU5ErkJggg==";
    call(&mut s, "add_picture", json!({"base64": png, "name": "x.png", "width": 2, "height": 1}));
    let bytes = base64::engine::general_purpose::STANDARD.decode(png).unwrap();
    let hash = xenify_doc::blobs::sha256(&bytes);
    let out = s.live_out().expect("a change to send").to_string();
    assert!(out.contains(&format!("sha256:{hash}")) && !out.contains(&png[..20]), "{out}");
    let saved = call(&mut s, "remote_save", json!({"mine": true}));
    let doc = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(xenify_doc::blobs::refs(&doc), vec![hash.clone()]);
    assert_eq!(xenify_doc::blobs::get(&hash).as_deref(), Some(&bytes[..]));
    // On this computer, a copy still holds the picture itself.
    assert!(call(&mut s, "save", json!({})).as_str().unwrap().contains(&png[..20]));
}

#[test]
fn a_3d_model_is_placed_as_a_picture_that_can_be_turned_again() {
    use base64::Engine as _;
    let mut s = Session::new(Project::new());
    call(&mut s, "new", json!({"width": 800, "height": 600}));
    let stl = "solid t\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 10 0 0\n   vertex 0 10 0\n  endloop\n endfacet\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 0 10 0\n   vertex 0 0 10\n  endloop\n endfacet\nendsolid t\n";
    let staged = call(&mut s, "model_stage", json!({"name": "t.stl", "base64": base64::engine::general_purpose::STANDARD.encode(stl)}));
    assert!(staged["summary"].as_str().unwrap().contains("2 triangles"), "{staged}");
    // A scene of the model, cropped to it, its longer side as asked.
    let scene = s.model_scene(crate::Pose { rot_x: 20.0, rot_y: -30.0, rot_z: 0.0 }, 256).unwrap();
    assert_eq!(scene.width.max(scene.height), 256);
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGP4z8DwHwQZYAQAZsQH+ZgV3QAAAABJRU5ErkJggg==";
    let id = call(&mut s, "model_place", json!({"base64": png, "width": 2, "height": 1, "pose": {"rot_x": 20, "rot_y": -30}})).as_u64().unwrap();
    let row = |s: &mut Session| call(s, "state", json!({}))["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(id)).cloned().unwrap();
    let r = row(&mut s);
    assert_eq!((r["kind"].as_str(), r["model"].as_bool(), r["raster"].as_bool()), (Some("picture"), Some(true), Some(true)));
    // Turned again: the same layer, the pose it was placed with, then one step to undo.
    let again = call(&mut s, "model_stage", json!({"id": id}));
    assert_eq!((again["rot_x"].as_f64(), again["rot_y"].as_f64(), again["side"].as_u64()), (Some(20.0), Some(-30.0), Some(2)));
    call(&mut s, "model_place", json!({"base64": png, "width": 2, "height": 1, "pose": {"rot_x": 0, "rot_y": 90}}));
    assert_eq!(call(&mut s, "state", json!({}))["undo"], json!("Turn the 3D model"));
    assert_eq!(call(&mut s, "model_stage", json!({"id": id}))["rot_y"].as_f64(), Some(90.0));
    assert_eq!(row(&mut s)["model"].as_bool(), Some(true));
}
