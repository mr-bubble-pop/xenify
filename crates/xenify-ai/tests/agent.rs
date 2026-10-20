//! Agent loop tests with a scripted "model" (no network).

use xenify_ai::{Agent, AiSettings, ChatItem, Next, Purpose, ToolHost};
use xenify_anim::{ApproxText, SceneText, TextMeasure};
use xenify_doc::{Id, LayerKind, Project, PropPath};
use xenify_ops::{Command, Editor};
use serde_json::{Value, json};

struct Host {
    editor: Editor,
    comp: Id,
    renders: usize,
}

impl Host {
    fn new() -> Self {
        let project = Project::new();
        let comp = project.compositions[0].id;
        Self { editor: Editor::new(project), comp, renders: 0 }
    }
}

impl ToolHost for Host {
    fn project(&self) -> &Project {
        self.editor.project()
    }
    fn comp(&self) -> Id {
        self.comp
    }
    fn set_comp(&mut self, comp: Id) -> Result<(), String> {
        self.editor.project().comp(comp).map_err(|e| e.to_string())?;
        self.comp = comp;
        Ok(())
    }
    fn time(&self) -> f64 {
        0.0
    }
    fn execute(&mut self, label: &str, cmd: Command) -> Result<(), String> {
        self.editor.execute(label, cmd).map_err(|e| e.to_string())
    }
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect {
        ApproxText.text_bounds(text)
    }
    fn render_png(&mut self, _time: f64) -> Result<Vec<u8>, String> {
        self.renders += 1;
        Ok(vec![0x89, b'P', b'N', b'G'])
    }
    fn giphy_gif(&mut self, query: &str) -> Result<(xenify_export::giphy::Gif, Vec<u8>), String> {
        let mut gif = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
            for c in [[255, 0, 0, 255], [0, 255, 0, 255]] {
                let img = image::RgbaImage::from_pixel(40, 20, image::Rgba(c));
                enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(100, 1)))
                    .unwrap();
            }
        }
        let g = xenify_export::giphy::Gif {
            id: "x".into(),
            title: format!("{query} GIF"),
            username: "maker".into(),
            page: String::new(),
            preview: String::new(),
            full: String::new(),
            width: 40,
            height: 20,
        };
        Ok((g, gif))
    }
    fn animated_emoji(&mut self, key: &str) -> Result<(xenify_export::animated_emoji::AnimatedEmoji, Vec<u8>), String> {
        // A stand-in for the download: a 100×100 dot that moves for a second.
        let e = xenify_export::animated_emoji::AnimatedEmoji {
            codepoint: "1f389".into(),
            name: "party popper".into(),
            category: "Celebration".into(),
            glyph: "🎉".into(),
            tags: vec!["party popper".into()],
            popularity: 1,
        };
        if key != "🎉" {
            return Err(format!("no animated emoji matches {key:?}"));
        }
        let lottie = json!({"fr": 30, "ip": 0, "op": 30, "w": 100, "h": 100, "layers": [{
            "ty": 4, "ind": 1, "nm": "Dot", "ip": 0, "op": 30, "st": 0,
            "ks": {"p": {"a": 1, "k": [{"t": 0, "s": [20, 50]}, {"t": 30, "s": [80, 50]}]}},
            "shapes": [{"ty": "el", "p": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [20, 20]}},
                       {"ty": "fl", "c": {"a": 0, "k": [1, 0, 0, 1]}, "o": {"a": 0, "k": 100}}]
        }]});
        Ok((e, lottie.to_string().into_bytes()))
    }
}

fn tool_calls(calls: &[(&str, &str, Value)]) -> Value {
    let calls: Vec<Value> = calls
        .iter()
        .map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}))
        .collect();
    json!({"choices": [{"message": {"role": "assistant", "content": "", "tool_calls": calls}}],
           "usage": {"prompt_tokens": 100, "completion_tokens": 20, "cost": 0.0001}})
}

fn text_reply(text: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": text}}], "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
}

fn body(next: &Next) -> &Value {
    match next {
        Next::Request { body, .. } => body,
        other => panic!("expected a request, got {other:?}"),
    }
}

/// Every assistant tool call must be answered by a tool message before the next request.
fn assert_protocol_valid(body: &Value) {
    let msgs = body["messages"].as_array().unwrap();
    let mut open: Vec<String> = Vec::new();
    for m in msgs {
        match m["role"].as_str().unwrap() {
            "assistant" => {
                assert!(open.is_empty(), "unanswered tool calls before next assistant message: {open:?}");
                for c in m["tool_calls"].as_array().into_iter().flatten() {
                    open.push(c["id"].as_str().unwrap().to_string());
                }
            }
            "tool" => {
                let id = m["tool_call_id"].as_str().unwrap();
                let pos = open.iter().position(|o| o == id).expect("tool result for unknown call");
                open.remove(pos);
            }
            _ => {}
        }
    }
    assert!(open.is_empty(), "unanswered tool calls: {open:?}");
}

#[test]
fn builds_an_animation_with_tools() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());

    let next = agent.send("Make a red ball move right", &mut host);
    let first = body(&next);
    assert_eq!(first["model"], "deepseek/deepseek-v4-flash");
    assert!(first["messages"][1]["content"].as_str().unwrap().contains("<current_scene>"));
    assert!(first["tools"].as_array().unwrap().iter().any(|t| t["function"]["name"] == "look_at_frame"));

    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c1", "get_scene", json!({})),
            (
                "c2",
                "add_layer",
                json!({"type": "ellipse", "name": "Ball", "size": [80, 80], "fill": "#ff0000", "position": [100, 540]}),
            ),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    // The model now knows the id from the add_layer result.
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "c3",
            "animate",
            json!({"layer": id, "property": "position", "keyframes": [
                {"time": 0, "value": [100, 540]}, {"time": 2, "value": [1820, 540], "easing": "ease-out"}]}),
        )])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let next = agent.handle_response(Purpose::Brain, Ok(text_reply("Done! A red ball slides across.")), &mut host);
    assert!(matches!(next, Next::Idle));

    let layer = &host.project().compositions[0].layers[0];
    assert_eq!(layer.meta.name, "Ball");
    assert!(layer.transform.position.is_animated());
    assert_eq!(layer.transform.position.value_at(2.0).x, 1820.0);
    assert!(agent.take_changed());
    assert!(
        agent.transcript.iter().any(|i| matches!(i, ChatItem::Activity { text, ok: true } if text.contains("Ball")))
    );
    assert_eq!(agent.transcript.last(), Some(&ChatItem::Assistant("Done! A red ball slides across.".into())));
    assert!(agent.usage.cost > 0.0);
    assert_eq!(agent.usage.prompt_tokens, 210);
}

#[test]
fn ask_user_pauses_and_resumes_remaining_calls() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("Make an intro", &mut host);
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("q1", "ask_user", json!({"question": "Which style?", "options": ["Bold", "Minimal"]})),
            ("c2", "add_layer", json!({"type": "text", "name": "Title", "text": "Hello"})),
        ])),
        &mut host,
    );
    assert!(matches!(next, Next::WaitForUser));
    assert!(agent.is_waiting_for_user());
    assert!(host.project().compositions[0].layers.is_empty(), "later calls wait for the answer");

    let next = agent.send("Minimal", &mut host);
    let b = body(&next);
    assert_protocol_valid(b);
    let answer = b["messages"].as_array().unwrap().iter().find(|m| m["tool_call_id"] == "q1").unwrap();
    assert!(answer["content"].as_str().unwrap().contains("Minimal"));
    let layer = &host.project().compositions[0].layers[0];
    // AI text is centred on its position by alignment (not an anchor offset).
    let LayerKind::Text(t) = &layer.kind else { panic!() };
    assert_eq!((t.align, t.valign), (xenify_doc::TextAlign::Center, xenify_doc::TextVAlign::Middle));
    assert_eq!(layer.transform.anchor.value_at(0.0), kurbo::Vec2::ZERO);
    assert!(agent.transcript.iter().any(|i| matches!(i, ChatItem::Question { answered: true, .. })));
}

#[test]
fn look_at_frame_goes_to_the_reviewer_with_an_image() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("check it", &mut host);
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("r1", "look_at_frame", json!({"time": 1.5, "question": "Is it centred?"}))])),
        &mut host,
    );
    let Next::Request { body, purpose } = next else { panic!() };
    assert_eq!(purpose, Purpose::Review { call_id: "r1".into(), time: 1.5 });
    assert_eq!(body["model"], "qwen/qwen3-vl-30b-a3b-instruct");
    let url = body["messages"][0]["content"][1]["image_url"]["url"].as_str().unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
    // The reviewer is told the real composition size and to use percentages.
    let text = body["messages"][0]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("1920×1080") && text.contains("percentages"), "{text}");
    assert_eq!(host.renders, 1);

    let next = agent.handle_response(purpose, Ok(text_reply("Yes, centred.")), &mut host);
    let b = body_of(&next);
    assert_protocol_valid(b);
    assert!(b["messages"].to_string().contains("Yes, centred."));
}

fn body_of(n: &Next) -> &Value {
    body(n)
}

#[test]
fn bad_calls_become_tool_errors_not_crashes() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    let raw = json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
        {"id": "a", "type": "function", "function": {"name": "add_layer", "arguments": "{not json"}},
        {"id": "b", "type": "function", "function": {"name": "teleport", "arguments": "{}"}},
        {"id": "c", "type": "function", "function": {"name": "animate", "arguments": "{\"layer\": 999, \"property\": \"position\", \"keyframes\": [{\"time\": 0, \"value\": [0,0]}]}"}},
        {"id": "d", "type": "function", "function": {"name": "add_layer", "arguments": "{\"type\": \"path\", \"name\": \"P\", \"path\": \"M 0 0 L 100 0 L 50 80 Z\", \"fill\": \"#00ff00\"}"}}
    ]}}]});
    let next = agent.handle_response(Purpose::Brain, Ok(raw), &mut host);
    let b = body(&next);
    assert_protocol_valid(b);
    let s = b["messages"].to_string();
    assert!(s.contains("not valid JSON") && s.contains("unknown tool") && s.contains("no layer"));
    // The valid call still ran; the path layer is centred on the path.
    let layer = &host.project().compositions[0].layers[0];
    assert_eq!(layer.transform.position.value_at(0.0), kurbo::Vec2::new(50.0, 40.0));
    let path = layer.property(PropPath::shape(0, xenify_doc::ShapeField::Path)).unwrap();
    assert!(!path.is_animated());
}

#[test]
fn network_errors_end_the_turn_and_stop_is_protocol_safe() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("hi", &mut host);
    let next = agent.handle_response(
        Purpose::Brain,
        Err(xenify_ai::AiError::Api { status: 401, message: "bad key".into() }),
        &mut host,
    );
    assert!(matches!(next, Next::Idle));
    assert!(matches!(agent.transcript.last(), Some(ChatItem::Error(e)) if e.contains("401")));

    // Stop while waiting for an answer, then carry on: history stays valid.
    agent.send("again", &mut host);
    agent.handle_response(Purpose::Brain, Ok(tool_calls(&[("q", "ask_user", json!({"question": "?"}))])), &mut host);
    agent.stop();
    let next = agent.send("new request", &mut host);
    assert_protocol_valid(body(&next));
}

#[test]
fn repeated_identical_failures_stop_the_turn() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    let bad = || tool_calls(&[("x", "delete_layer", json!({"layer": 999}))]);
    let mut next = agent.handle_response(Purpose::Brain, Ok(bad()), &mut host);
    assert!(body(&next).to_string().contains("no layer"));
    next = agent.handle_response(Purpose::Brain, Ok(bad()), &mut host);
    assert!(body(&next).to_string().contains("Do not repeat it"));
    next = agent.handle_response(Purpose::Brain, Ok(bad()), &mut host);
    assert!(matches!(next, Next::Request { .. }));
    next = agent.handle_response(Purpose::Brain, Ok(bad()), &mut host);
    assert!(matches!(next, Next::Idle));
    assert!(matches!(agent.transcript.last(), Some(ChatItem::Error(e)) if e.contains("repeating")));
}

#[test]
fn parented_layers_use_composition_coordinates() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "n",
            "add_layer",
            json!({"type": "null", "name": "Group", "position": [500, 500], "scale": 2}),
        )])),
        &mut host,
    );
    assert!(matches!(next, Next::Request { .. }));
    let group = host.project().compositions[0].layers[0].id.0;
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("a", "add_layer", json!({"type": "rect", "name": "Box", "position": [600, 500], "parent": group})),
            (
                "b",
                "add_layer",
                json!({"type": "path", "name": "Tri", "path": "M 500 400 L 600 400 L 550 450 Z", "parent": group}),
            ),
        ])),
        &mut host,
    );
    let comp = &host.project().compositions[0];
    let by_name = |n: &str| comp.layers.iter().find(|l| l.meta.name == n).unwrap().id;
    // Members sit below their group: Box, Tri, Group.
    assert_eq!(comp.layers.iter().map(|l| l.meta.name.as_str()).collect::<Vec<_>>(), ["Box", "Tri", "Group"]);
    assert!(matches!(comp.layers[2].kind, xenify_doc::LayerKind::Group), "\"null\" makes a group");
    // The box lands at (600, 500) on screen even though its parent is scaled 2×.
    let world = xenify_anim::world_transform(comp, by_name("Box"), 0.0);
    let p = world * kurbo::Point::ZERO;
    assert!((p - kurbo::Point::new(600.0, 500.0)).hypot() < 1e-9, "{p:?}");
    // The path's first point renders exactly where the model asked.
    let scene = xenify_anim::evaluate(comp, 0.0);
    let tri = scene.layer(by_name("Tri")).unwrap();
    let xenify_anim::SceneContent::Shapes(items) = &tri.content else { panic!() };
    let first = match items[0].path.elements()[0] {
        kurbo::PathEl::MoveTo(p) => tri.transform * p,
        _ => panic!(),
    };
    assert!((first - kurbo::Point::new(500.0, 400.0)).hypot() < 1e-6, "{first:?}");
    // And the scene summary reports the box position in composition coordinates.
    let summary = xenify_ai::scene_summary(host.project(), host.comp, 0.0, &mut ApproxText).to_string();
    assert!(summary.contains("[600.0,500.0]"), "{summary}");
    // On-screen bounds: the 200×200 box scaled 2× by its parent, centred on (600, 500).
    assert!(summary.contains("[400.0,300.0,800.0,700.0]"), "{summary}");
}

#[test]
fn overlapping_text_gets_a_layout_warning() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            (
                "a",
                "add_layer",
                json!({"type": "text", "name": "Title", "text": "Hello", "font_size": 100, "position": [960, 500]}),
            ),
            (
                "b",
                "add_layer",
                json!({"type": "text", "name": "Sub", "text": "world", "font_size": 40, "position": [960, 520]}),
            ),
            ("c", "add_layer", json!({"type": "rect", "name": "Edge", "size": [300, 100], "position": [1900, 540]})),
            (
                "d",
                "add_layer",
                json!({"type": "text", "name": "Far", "text": "ok", "font_size": 40, "position": [300, 900]}),
            ),
            (
                "e",
                "add_layer",
                json!({"type": "text", "name": "Tiny", "text": "psst", "font_size": 10, "position": [300, 300]}),
            ),
        ])),
        &mut host,
    );
    let s = body(&next)["messages"].to_string();
    assert!(s.contains("overlaps") && s.contains("Title"), "{s}");
    assert!(s.contains("outside the frame"), "{s}");
    assert!(s.contains("too small to read"), "{s}");
    // The well-placed layer gets no warning.
    let msgs = body(&next)["messages"].as_array().unwrap();
    let far = msgs.iter().find(|m| m["tool_call_id"] == "d").unwrap();
    assert!(!far["content"].as_str().unwrap().contains("layout_warnings"));
}

#[test]
fn frame_checks_are_budgeted() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings { max_reviews: 1, ..Default::default() });
    agent.send("check", &mut host);
    let look = |id: &str| tool_calls(&[(id, "look_at_frame", json!({"time": 1, "question": "ok?"}))]);
    let next = agent.handle_response(Purpose::Brain, Ok(look("r1")), &mut host);
    let Next::Request { purpose, .. } = next else { panic!() };
    agent.handle_response(purpose, Ok(text_reply("fine")), &mut host);
    // Second look: over budget, answered without rendering.
    let next = agent.handle_response(Purpose::Brain, Ok(look("r2")), &mut host);
    assert!(matches!(&next, Next::Request { purpose: Purpose::Brain, .. }));
    assert!(body(&next).to_string().contains("budget"));
    assert_eq!(host.renders, 1);
}

#[test]
fn align_tool_centres_layers_on_the_frame() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("a", "add_layer", json!({"type": "rect", "name": "A", "size": [100, 50], "position": [300, 200]})),
            ("b", "add_layer", json!({"type": "rect", "name": "B", "size": [60, 60], "position": [900, 700]})),
        ])),
        &mut host,
    );
    let ids: Vec<u64> = host.project().compositions[0].layers.iter().map(|l| l.id.0).collect();
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("c", "align_layers", json!({"layers": ids, "edge": "h_center"}))])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    for l in &host.project().compositions[0].layers {
        assert_eq!(l.transform.position.value_at(0.0).x, 960.0);
    }
}

#[test]
fn check_layout_measures_offsets_overlaps_and_contrast() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("a", "add_layer", json!({"type": "rect", "name": "Panel", "size": [800, 400], "fill": "#ffffff", "position": [960, 540]})),
            ("b", "add_layer", json!({"type": "text", "name": "Label", "text": "Hi", "fill": "#eeeeee", "font_size": 60, "position": [1000, 540]})),
        ])),
        &mut host,
    );
    let next =
        agent.handle_response(Purpose::Brain, Ok(tool_calls(&[("c", "check_layout", json!({"time": 0}))])), &mut host);
    let msgs = body(&next)["messages"].as_array().unwrap();
    let report: Value =
        serde_json::from_str(msgs.iter().find(|m| m["tool_call_id"] == "c").unwrap()["content"].as_str().unwrap())
            .unwrap();
    let layers = report["layers_bottom_to_top"].as_array().unwrap();
    assert_eq!(layers[0]["centre_offset_from_frame_centre"], json!([0.0, 0.0]));
    assert_eq!(layers[1]["centre_offset_from_frame_centre"][0], json!(40.0));
    // Light grey text on a white panel: poor contrast, detected against the panel (not the background).
    assert_eq!(layers[1]["text"]["behind"], "#ffffff");
    assert_eq!(layers[1]["text"]["contrast_ok"], false);
    assert_eq!(report["overlaps"][0]["involves_text"], true);
}

#[test]
fn presets_via_tool_including_squash_after_bounce() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "a",
            "add_layer",
            json!({"type": "ellipse", "name": "Ball", "size": [100, 100], "position": [960, 900]}),
        )])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("b", "apply_preset", json!({"layers": [id], "preset": "bounce", "height": 400, "bounces": 2})),
            ("c", "apply_preset", json!({"layers": [id], "preset": "squash_stretch", "strength": 0.3})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    assert!(!body(&next).to_string().contains("\"error\""), "{}", body(&next));
    let l = &host.project().compositions[0].layers[0];
    assert!(l.transform.position.is_animated() && l.transform.scale.is_animated());
}

#[test]
fn update_layer_sets_track_mattes_and_draw_on_preset() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("a", "add_layer", json!({"type": "rect", "name": "Title", "size": [600, 100], "position": [960, 540]})),
            ("b", "add_layer", json!({"type": "rect", "name": "Window", "size": [800, 200], "position": [960, 500]})),
        ])),
        &mut host,
    );
    let ids: Vec<u64> = host.project().compositions[0].layers.iter().map(|l| l.id.0).collect();
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c", "update_layer", json!({"layer": ids[0], "matte": ids[1], "matte_mode": "alpha_inverted"})),
            ("d", "apply_preset", json!({"layers": [ids[1]], "preset": "draw_on", "duration": 2})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let layers = &host.project().compositions[0].layers;
    let m = layers[0].meta.matte.expect("matte set");
    assert_eq!((m.layer.0, m.mode), (ids[1], xenify_doc::MatteMode::AlphaInverted));
    assert!(layers[1].shapes()[0].trim.is_some());
    let summary = serde_json::to_string(body(&next)).unwrap();
    assert!(!summary.contains("unknown preset"), "{summary}");
}

#[test]
fn gradients_can_be_keyframed_with_css() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "a",
            "add_layer",
            json!({"type": "rect", "name": "Sky", "size": [400, 200], "fill": "linear-gradient(90deg, #ff0000, #0000ff)"}),
        )])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "b",
            "animate",
            json!({"layer": id, "property": "gradient", "keyframes": [
                {"time": 0, "value": "linear-gradient(90deg, #ff0000, #0000ff)"},
                {"time": 2, "value": "linear-gradient(180deg, #00ff00, #ffffff)"}
            ]}),
        )])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let fill = host.project().compositions[0].layers[0].shapes()[0].fill.clone().unwrap();
    let g = fill.gradient.unwrap();
    assert!(g.is_animated());
    let mid = g.value_at(1.0);
    assert!((mid.stops[0].color.r - 0.5).abs() < 0.01 && (mid.stops[0].color.g - 0.5).abs() < 0.01, "{mid:?}");
    // The scene summary reports it as CSS.
    let comp = host.project().compositions[0].id;
    let summary =
        xenify_ai::scene_summary(&host.project().clone(), comp, 1.0, &mut xenify_anim::ApproxText).to_string();
    assert!(summary.contains("shapes.0.gradient"), "{summary}");
}

#[test]
fn a_matte_hiding_the_settled_layer_is_flagged() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            (
                "a",
                "add_layer",
                json!({"type": "text", "name": "Title", "text": "LINEAR", "font_size": 90, "position": [960, 800]}),
            ),
            // Bottom edge at y = 800: cuts the title in half.
            ("b", "add_layer", json!({"type": "rect", "name": "Mask", "size": [1920, 800], "position": [960, 400]})),
        ])),
        &mut host,
    );
    let ids: Vec<u64> = host.project().compositions[0].layers.iter().map(|l| l.id.0).collect();
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("c", "update_layer", json!({"layer": ids[0], "matte": ids[1]}))])),
        &mut host,
    );
    let text = serde_json::to_string(body(&next)).unwrap();
    assert!(text.contains("hidden by its matte"), "{text}");
    // Growing the matte to cover the title clears it (warnings are checked via the masked layer).
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "d",
            "set_values",
            json!({"layer": ids[1], "values": {"size": [1920, 2000], "position": [960, 0]}}),
        )])),
        &mut host,
    );
    let msgs = body(&next)["messages"].as_array().unwrap();
    let d = msgs.iter().find(|m| m["tool_call_id"] == "d").unwrap()["content"].as_str().unwrap();
    assert!(!d.contains("hidden by its matte"), "{d}");
}

#[test]
fn the_ai_can_precompose_and_edit_inside_precomps() {
    let mut host = Host::new();
    let main = host.comp;
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("a", "add_layer", json!({"type": "rect", "name": "A", "size": [100, 100]})),
            ("b", "add_layer", json!({"type": "ellipse", "name": "B", "size": [50, 50]})),
        ])),
        &mut host,
    );
    let ids: Vec<u64> = host.project().compositions[0].layers.iter().map(|l| l.id.0).collect();
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("c", "precompose", json!({"layers": ids, "name": "Logo"}))])),
        &mut host,
    );
    let msgs = body(&next)["messages"].as_array().unwrap();
    let result: Value =
        serde_json::from_str(msgs.iter().find(|m| m["tool_call_id"] == "c").unwrap()["content"].as_str().unwrap())
            .unwrap();
    let inner = result["comp"].as_u64().unwrap();
    assert_eq!(host.project().compositions[0].layers.len(), 1);
    // Open it, add a layer inside, and come back.
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("d", "open_composition", json!({"comp": inner})),
            ("e", "add_layer", json!({"type": "text", "name": "Inside", "text": "Hi"})),
            ("f", "open_composition", json!({"comp": main.0})),
        ])),
        &mut host,
    );
    assert_eq!(host.comp, main);
    let inner_comp = host.project().comp(xenify_doc::Id(inner)).unwrap();
    assert_eq!(inner_comp.layers.len(), 3);
    assert!(inner_comp.layers.iter().any(|l| l.meta.name == "Inside"));
}

#[test]
fn scene_summary_lists_compositions_and_precomp_targets() {
    let mut host = Host::new();
    let main = host.comp;
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("a", "add_layer", json!({"type": "rect", "name": "A", "size": [100, 100]}))])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    agent.handle_response(Purpose::Brain, Ok(tool_calls(&[("b", "precompose", json!({"layers": [id]}))])), &mut host);
    let s = xenify_ai::scene_summary(&host.project().clone(), main, 0.0, &mut xenify_anim::ApproxText);
    assert_eq!(s["all_compositions"].as_array().unwrap().len(), 2);
    assert!(s["layers_bottom_to_top"][0]["comp"].is_u64());
}

#[test]
fn expressions_are_set_checked_and_reported() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("a", "add_layer", json!({"type": "rect", "name": "Spinner", "size": [100, 100]}))])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("b", "update_layer", json!({"layer": id, "expressions": {"rotation": "time * 90"}})),
            ("c", "update_layer", json!({"layer": id, "expressions": {"opacity": "nonsense("}})),
        ])),
        &mut host,
    );
    let l = &host.project().compositions[0].layers[0];
    assert_eq!(l.meta.expressions.len(), 1);
    assert_eq!(l.meta.expressions[0].source, "time * 90");
    let scene = xenify_anim::evaluate(&host.project().compositions[0], 1.0);
    let t = scene.layers[0].transform.as_coeffs();
    assert!((t[1] - 1.0).abs() < 1e-9, "rotated 90° at 1 s: {t:?}");
    let msgs = body(&next)["messages"].as_array().unwrap();
    let c = msgs.iter().find(|m| m["tool_call_id"] == "c").unwrap()["content"].as_str().unwrap();
    assert!(c.contains("expression for opacity"), "{c}");
}

#[test]
fn pens_and_caps_can_be_set_by_name_or_settings() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "a",
            "add_layer",
            json!({"type": "path", "name": "Line", "path": "M100 100 L500 200", "stroke": "#ffffff", "stroke_width": 12, "fill": "none"}),
        )])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("b", "update_layer", json!({"layer": id, "pen": "pastel", "cap": "rounded", "cap_roundness": 0.8})),
            ("c", "update_layer", json!({"layer": id, "pen": {"kind": "calligraphy", "angle": 10}})),
        ])),
        &mut host,
    );
    let s = host.project().compositions[0].layers[0].shapes()[0].stroke.clone().unwrap();
    assert_eq!(s.style.kind, xenify_doc::PenKind::Calligraphy);
    assert_eq!(s.style.angle, 10.0);
    assert_eq!(s.style.thinning, xenify_doc::PenStyle::preset(xenify_doc::PenKind::Calligraphy).thinning);
    assert_eq!((s.cap, s.cap_roundness), (xenify_doc::LineCap::Square, 0.8));
}

#[test]
fn builds_a_slide_deck() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    let main = host.comp;
    agent.send("Make a two-slide deck", &mut host);
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c1", "add_layer", json!({"type": "text", "name": "Title", "text": "Hello", "position": [960, 540]})),
            (
                "c2",
                "add_slide",
                json!({"name": "Agenda", "notes": "Walk through the plan", "transition": {"kind": "fade", "duration": 0.8}, "duration": 3}),
            ),
            ("c3", "add_layer", json!({"type": "text", "name": "Agenda", "text": "Agenda", "position": [960, 200]})),
            ("c4", "edit_slide", json!({"slide": 1, "notes": "Welcome everyone", "name": "Title"})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let p = host.project();
    assert_eq!(p.slides.len(), 2, "the main composition became slide 1");
    assert_eq!(p.slides[0].comp, main);
    assert_eq!(p.slides[0].notes, "Welcome everyone");
    assert_eq!(p.comp(main).unwrap().settings.name, "Title");
    let second = p.comp(p.slides[1].comp).unwrap();
    assert_eq!(second.settings.name, "Agenda");
    assert_eq!(second.settings.duration, 3.0);
    assert_eq!(second.layers.len(), 1, "later tools edit the new slide");
    assert_eq!(p.slides[1].notes, "Walk through the plan");
    assert_eq!(p.slides[1].transition_in, xenify_doc::Transition::new(xenify_doc::TransitionKind::Fade, 0.8));
    assert_eq!(host.comp, p.slides[1].comp);
    // The scene lists the slides; open_slide goes back.
    let next =
        agent.handle_response(Purpose::Brain, Ok(tool_calls(&[("c5", "open_slide", json!({"slide": 1}))])), &mut host);
    let msgs = body(&next)["messages"].as_array().unwrap();
    let result = msgs.last().unwrap()["content"].as_str().unwrap();
    assert!(result.contains("\"slides\"") && result.contains("\"slide\":1"), "{result}");
    assert_eq!(host.comp, main);
}

#[test]
fn add_mask_crops_in_the_layers_own_space() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("Crop the photo to its left half", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "c1",
            "add_layer",
            json!({"type": "rect", "name": "Photo", "size": [400, 200], "position": [500, 300], "fill": "#ffffff"}),
        )])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c2", "add_mask", json!({"layer": id, "rect": [300, 200, 500, 400]})),
            ("c3", "add_mask", json!({"layer": id, "shape": "ellipse", "rect": [350, 250, 400, 300], "mode": "subtract", "feather": 5})),
            ("c4", "animate", json!({"layer": id, "property": "masks.1.feather", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": 20}]})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let l = &host.project().compositions[0].layers[0];
    assert_eq!(l.meta.masks.len(), 2);
    // The layer sits at 500,300, so the crop is its left half in its own space.
    let r = xenify_doc::as_rect(&l.meta.masks[0].path.value_at(0.0)).unwrap();
    assert!((r.x0 + 200.0).abs() < 1e-6 && r.x1.abs() < 1e-6, "{r:?}");
    assert_eq!(l.meta.masks[1].mode, xenify_doc::MaskMode::Subtract);
    assert!(l.meta.masks[1].feather.is_animated());
}

#[test]
fn add_array_repeats_a_layer_around_a_centre_in_composition_pixels() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("A ring of twelve clock ticks", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "c1",
            "add_layer",
            json!({"type": "rect", "name": "Tick", "size": [10, 40], "position": [500, 200], "fill": "#ffffff"}),
        )])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c2", "add_array", json!({"layer": id, "count": 12, "rotation": 30, "origin": [500, 400]})),
            ("c3", "animate", json!({"layer": id, "property": "modifiers.0.count", "keyframes": [{"time": 0, "value": 1}, {"time": 1, "value": 12}]})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let project = host.project();
    let scene = xenify_anim::evaluate_project(project, project.compositions[0].id, 1.0);
    let ticks: Vec<_> = scene.layers.iter().map(|l| l.transform * kurbo::Point::ZERO).collect();
    assert_eq!(ticks.len(), 12);
    // Half way round the centre at 500,400: straight below it.
    assert!((ticks[6] - kurbo::Point::new(500.0, 600.0)).hypot() < 1e-6, "{:?}", ticks[6]);
}

#[test]
fn add_modifier_roughens_a_shape_and_can_boil() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("A hand-drawn wobbly circle", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "c1",
            "add_layer",
            json!({"type": "ellipse", "name": "Blob", "size": [200, 200], "position": [500, 300], "fill": "#ff8800"}),
        )])),
        &mut host,
    );
    let id = host.project().compositions[0].layers[0].id.0;
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c2", "add_modifier", json!({"layer": id, "type": "jitter", "amount": 8, "smooth": true})),
            ("c3", "add_modifier", json!({"layer": id, "type": "wave", "ridges": 3})),
            ("c4", "animate", json!({"layer": id, "property": "modifiers.0.seed", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": 5}]})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let l = &host.project().compositions[0].layers[0];
    assert_eq!(l.meta.modifiers.len(), 2);
    assert_eq!(l.meta.modifiers[1].kind.label(), "Wave");
    let project = host.project();
    let at = |t| xenify_anim::evaluate_project(project, project.compositions[0].id, t).layers[0].clone();
    assert_ne!(at(0.0).content, at(0.5).content, "the outline boils");
}

#[test]
fn add_modifier_blends_and_follows_paths() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("Beads on a ring, and a square morphing into a circle", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c1", "add_layer", json!({"type": "ellipse", "name": "Ring", "size": [300, 300], "position": [500, 400], "stroke": "#ffffff"})),
            ("c2", "add_layer", json!({"type": "ellipse", "name": "Bead", "size": [20, 20], "position": [0, 0], "fill": "#ff0000"})),
            ("c3", "add_layer", json!({"type": "rect", "name": "Sq", "size": [50, 50], "position": [100, 100], "fill": "#00ff00"})),
            ("c4", "add_layer", json!({"type": "ellipse", "name": "Ci", "size": [80, 80], "position": [900, 100], "fill": "#0000ff"})),
        ])),
        &mut host,
    );
    let id = |n: &str| host.project().compositions[0].layers.iter().find(|l| l.meta.name == n).unwrap().id.0;
    let (ring, bead, sq, ci) = (id("Ring"), id("Bead"), id("Sq"), id("Ci"));
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            ("c5", "add_modifier", json!({"layer": bead, "type": "path_array", "target": ring, "count": 12})),
            ("c6", "add_modifier", json!({"layer": sq, "type": "blend", "target": ci, "count": 5})),
            ("c7", "add_modifier", json!({"layer": ci, "type": "mirror", "centre": [1000, 100]})),
        ])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let project = host.project();
    let scene = xenify_anim::evaluate_project(project, project.compositions[0].id, 0.0);
    let count = |i: u64| scene.layers.iter().filter(|l| l.id.0 == i).count();
    assert_eq!((count(bead), count(sq), count(ci)), (12, 6, 2));
}

#[test]
fn builds_a_diagram_with_shapes_and_connectors() {
    let mut host = Host::new();
    let main = host.comp;
    let mut agent = Agent::new(AiSettings::default());
    let next = agent.send("Draw a login flow", &mut host);
    let tools = body(&next)["tools"].as_array().unwrap().clone();
    for name in ["add_diagram_shape", "add_connector", "new_diagram"] {
        assert!(tools.iter().any(|t| t["function"]["name"] == name), "{name} offered");
    }
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("c1", "new_diagram", json!({"name": "Login", "place_in_current": true}))])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let diagram = host.comp;
    assert_ne!(diagram, main, "switched to the new diagram");
    assert_eq!(host.project().comp(diagram).unwrap().settings.kind, xenify_doc::CompKind::Diagram);
    assert!(
        host.project()
            .comp(main)
            .unwrap()
            .layers
            .iter()
            .any(|l| matches!(l.kind, LayerKind::Precomp { comp, .. } if comp == diagram))
    );

    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[
            (
                "c2",
                "add_diagram_shape",
                json!({"shape": "flowchart.terminator", "position": [200, 100], "label": "Start"}),
            ),
            (
                "c3",
                "add_diagram_shape",
                json!({"shape": "flowchart.decision", "position": [200, 300], "label": "Signed in?"}),
            ),
        ])),
        &mut host,
    );
    let ids: Vec<u64> = host.project().comp(diagram).unwrap().layers.iter().map(|l| l.id.0).collect();
    assert_eq!(ids.len(), 2);
    let next = agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "c4",
            "add_connector",
            json!({"from": ids[0], "to": ids[1], "to_side": "top", "label": "go", "arrow": "open", "flow": 40}),
        )])),
        &mut host,
    );
    assert_protocol_valid(body(&next));
    let c = host.project().comp(diagram).unwrap().layers.last().unwrap().clone();
    let conn = c.meta.connector.as_ref().expect("a connector");
    assert_eq!(conn.to.attach.unwrap().port, xenify_doc::PortRef::Port(0));
    assert_eq!(c.meta.label.as_ref().unwrap().text, "go");
    assert!(conn.flow.is_some());
    // The scene summary shows it as a connector between the two.
    let next = agent.handle_response(Purpose::Brain, Ok(tool_calls(&[("c5", "get_scene", json!({}))])), &mut host);
    let msgs = body(&next)["messages"].as_array().unwrap();
    let scene = msgs.iter().rev().find(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap();
    assert!(scene.contains("\"type\":\"connector\"") && scene.contains("\"kind\":\"diagram\""), "{scene}");
}

#[test]
fn animated_emoji_are_looping_symbols_with_editable_timing() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "e",
            "add_emoji",
            json!({"emoji": "🎉", "animated": true, "size": 300, "position": [400, 300]}),
        )])),
        &mut host,
    );
    let comp = &host.project().compositions[0];
    let layer = comp.layers.last().expect("added");
    let LayerKind::Precomp { comp: symbol, looped, .. } = layer.kind else { panic!("a symbol: {:?}", layer.kind) };
    assert!(looped, "loops by default");
    assert!((layer.transform.scale.value_at(0.0).x - 3.0).abs() < 1e-9, "100 px scaled to 300");
    let inner = host.project().comp(symbol).unwrap();
    assert!(inner.settings.name.contains("party popper"));
    assert_eq!(inner.layers[0].transform.position.keyframes().len(), 2, "keyframes stay editable");
    let id = layer.id.0;
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("t", "set_symbol_timing", json!({"layer": id, "starts_at": 1.5, "looped": false}))])),
        &mut host,
    );
    let layer = host.project().layer(Id(id)).unwrap();
    assert!(matches!(layer.kind, LayerKind::Precomp { start, looped: false, .. } if start == 1.5));
}

#[test]
fn animated_image_playback_is_an_ai_tool() {
    let mut host = Host::new();
    let comp = host.comp;
    let mut asset = xenify_doc::ImageAsset::new(Id(50), "blink", "image/gif", 8, 8, vec![1, 2, 3]);
    asset.frames = Some(vec![0.25, 0.25]);
    let layer = xenify_doc::Layer::image(Id(51), "blink", &asset, xenify_doc::Vec2::ZERO);
    host.editor.execute("add", Command::AddAsset { asset }).unwrap();
    host.editor.execute("add", Command::AddLayer { comp, index: None, layer }).unwrap();
    let summary = xenify_ai::scene_summary(host.project(), comp, 0.0, &mut ApproxText).to_string();
    assert!(summary.contains("\"frames\":2"), "{summary}");
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("p", "set_image_playback", json!({"layer": 51, "speed": 0.5, "looped": false}))])),
        &mut host,
    );
    let LayerKind::Image { playback, .. } = host.project().layer(Id(51)).unwrap().kind else { panic!() };
    assert_eq!((playback.speed, playback.looped), (0.5, false));
}

#[test]
fn gifs_from_giphy_are_animated_image_layers() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("g", "add_gif", json!({"query": "dance", "size": 400}))])),
        &mut host,
    );
    let layer = host.project().compositions[0].layers.last().unwrap();
    assert_eq!(layer.meta.name, "dance GIF by @maker (GIPHY)");
    let LayerKind::Image { asset, .. } = layer.kind else { panic!("{:?}", layer.kind) };
    assert_eq!(host.project().asset(asset).unwrap().frames.as_ref().map(Vec::len), Some(2));
    assert!((layer.transform.scale.value_at(0.0).x - 10.0).abs() < 1e-9, "40 px wide → 400");
}

#[test]
fn templated_slides_and_themes_are_ai_tools() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[(
            "s",
            "add_slide",
            json!({"layout": "bullets", "theme": "Dark Neon", "title": "Why now", "items": ["Cheap", "Fast", "Fun"]}),
        )])),
        &mut host,
    );
    let p = host.project();
    assert_eq!(p.slides.len(), 2);
    assert_eq!(p.theme.as_deref(), Some("Dark Neon"));
    let slide = p.comp(p.slides[1].comp).unwrap();
    assert!(slide.layers.iter().any(|l| matches!(&l.kind, LayerKind::Text(t) if t.text == "Fast")));
    assert_eq!(slide.settings.pauses.len(), 2, "a click before each later bullet");
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("t", "apply_theme", json!({"theme": "Corporate"}))])),
        &mut host,
    );
    assert_eq!(host.project().theme.as_deref(), Some("Corporate"));
}

#[test]
fn diagram_shapes_go_into_lanes() {
    let mut host = Host::new();
    let mut agent = Agent::new(AiSettings::default());
    agent.send("go", &mut host);
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("l", "add_diagram_shape", json!({"shape": "bpmn.lane", "position": [500, 300]}))])),
        &mut host,
    );
    let lane = host.project().compositions[0].layers.iter().find(|l| l.meta.container).unwrap().id;
    agent.handle_response(
        Purpose::Brain,
        Ok(tool_calls(&[("t", "add_diagram_shape", json!({"shape": "bpmn.task", "in": lane.0, "label": "Review"}))])),
        &mut host,
    );
    let task = host.project().compositions[0]
        .layers
        .iter()
        .find(|l| l.meta.label.as_ref().is_some_and(|t| t.text == "Review"))
        .unwrap();
    assert_eq!(task.meta.parent, Some(lane));
}
