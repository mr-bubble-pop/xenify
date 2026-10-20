//! The assistant's tools: JSON schemas for the model, and their translation
//! into undoable [`Command`]s.
//!
//! Every coordinate the model sees or writes is in composition pixels (origin
//! top-left, y down), like drawing on an SVG canvas the size of the video.

use crate::agent::ToolHost;
use crate::values;
use xenify_anim::{SceneText, world_transform};
use xenify_doc::{
    AnyProperty, Color, CompSettings, Easing, Geometry, Id, Keyframe, Layer, LayerKind, LayerMeta, LineCap, LineJoin,
    PropPath, Property, ShapeField, ShapeItem, Stroke, TextAlign, TextLayer, TextVAlign, Value, ValueKind,
};
use xenify_ops::Command;
use kurbo::{Affine, BezPath, Shape, Vec2};
use serde_json::{Value as Json, json};

/// What running a tool produced.
pub enum ToolOutcome {
    /// Result for the model, plus a short human-readable line for the chat.
    Done { result: Json, activity: Option<String> },
    /// Pause and ask the user; their reply becomes the tool result.
    AskUser { question: String, options: Vec<String> },
    /// Render this frame and ask the reviewer model `question` about it.
    Review { time: f64, question: String },
    /// Generate an image of `prompt`, then trace it into vector layers.
    Illustrate { prompt: String, args: Json },
}

fn done(result: Json, activity: impl Into<String>) -> ToolOutcome {
    ToolOutcome::Done { result, activity: Some(activity.into()) }
}

/// OpenAI-style tool definitions.
pub fn definitions(has_reviewer: bool) -> Vec<Json> {
    let prop_desc = "Property name: position, scale, rotation, opacity, anchor, size, corner_radius, fill, stroke, \
                     stroke_width, path, font_size, text_color, gradient (CSS gradient string, on shapes with a gradient fill — keyframe it to animate colours/direction; gradient_start / gradient_end [x, y] in the shape's own coordinates and gradient_colors animate separately), trim_start, trim_end, trim_offset (0..1, after draw_on) (or a full path like \"shapes.1.fill_color\").";
    let kf = json!({
        "type": "object",
        "properties": {
            "time": {"type": "number", "description": "Seconds from the start."},
            "value": {"description": "Number, [x, y], \"#rrggbb\" colour, or SVG path string (composition coordinates) depending on the property."},
            "easing": {"description": format!("Easing to the NEXT keyframe: one of {}, a CSS steps(n, jump-end), or [x1,y1,x2,y2]. Default ease_in_out.", values::easing_names())}
        },
        "required": ["time", "value"]
    });
    let mut tools = vec![
        tool(
            "get_scene",
            "Returns the composition settings and every layer with its properties and keyframes. Call this first, and again after big changes.",
            json!({"type": "object", "properties": {}}),
        ),
        tool(
            "set_composition",
            "Changes composition settings. Only pass what should change.",
            json!({"type": "object", "properties": {
                "width": {"type": "integer"}, "height": {"type": "integer"},
                "fps": {"type": "number"}, "duration": {"type": "number", "description": "Seconds."},
                "background": {"type": "string", "description": "\"#rrggbb\""},
                "motion_blur": {"type": "boolean", "description": "Film-like blur on fast motion in the rendered video (not shown on the live canvas)."},
                "shutter_angle": {"type": "number", "description": "Motion blur strength in degrees, default 180 (360 = very blurry)."},
                "kind": {"type": "string", "enum": ["animation", "diagram"], "description": "diagram: a flowchart / network / workflow canvas (light page, grid, diagram tools)."}
            }}),
        ),
        tool(
            "add_layer",
            "Creates a layer and returns its id. Shapes are centred on `position`. For type \"path\", give `path` as SVG path data in composition coordinates; the layer's position becomes the path's centre. Text is placed by `align`/`valign` (default: centred on `position`), and stays aligned if its text or font changes.",
            json!({"type": "object", "properties": {
                "type": {"type": "string", "enum": ["rect", "ellipse", "polygon", "star", "gear", "path", "text", "group"]},
                "name": {"type": "string"},
                "count": {"type": "number", "description": "polygon: sides; star: points; gear: teeth (animate with property shape.0.count)."},
                "inner": {"type": "number", "description": "star: inner radius 0..1 (default 0.5); gear: tooth depth 0..0.6 (default 0.2)."},
                "hole": {"type": "number", "description": "gear: hole in the middle, 0..0.9 of the radius (default 0.3)."},
                "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y] centre in composition pixels. Default: composition centre."},
                "size": {"type": "array", "items": {"type": "number"}, "description": "[width, height] for rect/ellipse."},
                "corner_radius": {"type": "number"},
                "path": {"type": "string", "description": "SVG path data (M, L, C, Q, A, Z…), composition coordinates."},
                "text": {"type": "string"}, "font_size": {"type": "number"}, "font_family": {"type": "string", "description": "Default \"Inter\"."},
                "align": {"type": "string", "enum": ["left", "center", "right"], "description": "Text: which point of the text is at `position` horizontally (and how lines align). Default center."},
                "valign": {"type": "string", "enum": ["top", "middle", "baseline", "bottom"], "description": "Text: which point is at `position` vertically. Default middle."},
                "fill": {"type": "string", "description": "\"#rrggbb\", \"none\", or a CSS gradient like \"linear-gradient(135deg, #ff6b6b, #feca57)\" / \"radial-gradient(#fff, #88f)\". Text colour for text layers."},
                "stroke": {"type": "string"}, "stroke_width": {"type": "number"},
                "rotation": {"type": "number", "description": "Degrees clockwise."},
                "scale": {"description": "1 = 100%. Number or [sx, sy]."},
                "opacity": {"type": "number", "description": "0..1"},
                "parent": {"type": "integer", "description": "Id of a layer (e.g. a group) whose transform this layer follows. With a group, the new layer becomes one of its members."},
                "in_point": {"type": "number"}, "out_point": {"type": "number"}
            }, "required": ["type", "name"]}),
        ),
        tool(
            "animate",
            "Replaces a property's animation with the given keyframes (sorted by time). Use for all motion.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "property": {"type": "string", "description": prop_desc},
                "keyframes": {"type": "array", "items": kf, "minItems": 1}
            }, "required": ["layer", "property", "keyframes"]}),
        ),
        tool(
            "set_values",
            "Sets properties to constant values (removing any animation on them).",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "values": {"type": "object", "description": format!("Map of property → value. {prop_desc}")}
            }, "required": ["layer", "values"]}),
        ),
        tool(
            "update_layer",
            "Renames, re-parents, retimes, shows/hides a layer, sets its mask layer, or changes a text layer's text/font.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"}, "name": {"type": "string"},
                "parent": {"description": "Layer id, or null to unparent."},
                "in_point": {"type": "number"}, "out_point": {"description": "Seconds, or null for 'until the end'."},
                "visible": {"type": "boolean"}, "text": {"type": "string"}, "font_family": {"type": "string"},
                "matte": {"description": "Mask layer: id of a shape or text layer; this layer (or group) then only shows inside it (the matte layer itself is no longer drawn). null removes it. Great for reveals: animate the matte shape."},
                "matte_mode": {"type": "string", "enum": ["alpha", "alpha_inverted", "luma", "luma_inverted"], "description": "Default alpha. Inverted = show only OUTSIDE the matte."},
                "expressions": {"type": "object", "description": "Formulas that compute property values every frame, like spreadsheet formulas: map of property → expression (or null to remove). Names: time, value (the keyframed value), frame, fps, duration, index. Functions: wiggle(freq, amount), random(min,max), noise(x), value_at(t), loop_out(), ping_pong(), sin, cos, abs, min, max, clamp, lerp, smoothstep, round. Vectors as [x, y]; value.x/value.y. Examples: {\"rotation\": \"time * 90\"}, {\"position\": \"value + [0, sin(time * 4) * 20]\"}, {\"opacity\": \"loop_out()\"}."},
                "pen": {"description": "Stroke style for the layer's strokes: a name (solid, dashed, dotted, dash_dot, double_line, tapered, calligraphy, brush, ink, marker, felt_tip, pencil, charcoal, pastel, crayon, watercolor, airbrush, bristle, glow, sketchy, rough) or an object {\"kind\": \"calligraphy\", \"angle\": 30, \"thinning\": 0.1, \"taper_start\": 0.5, \"taper_end\": 0.5, \"roughness\": 0.2, \"grain\": 0.5, \"dash\": 3, \"gap\": 2, \"flow\": 1, \"count\": 3}. The layer needs a stroke (stroke colour + width)."},
                "cap": {"type": "string", "enum": ["flat", "square", "round", "rounded"], "description": "Stroke end caps; rounded = rounded rectangle (see cap_roundness)."},
                "cap_roundness": {"type": "number", "description": "0-1 for rounded caps (0.8 ≈ almost round)."},
                "wiggle": {"description": "Live random drift (noise) on top of the animation, e.g. {\"frequency\": 2, \"position\": 15, \"rotation\": 3, \"scale\": 0.05}. position = px, rotation = degrees, scale = factor. null removes it."},
                "align": {"type": "string", "enum": ["left", "center", "right"]},
                "valign": {"type": "string", "enum": ["top", "middle", "baseline", "bottom"]}
            }, "required": ["layer"]}),
        ),
        tool(
            "add_svg",
            "Draws vector artwork by writing SVG markup (paths, shapes, gradients, groups, transforms). It is imported as editable path layers in a new group, scaled down to fit if larger than the composition and centred — then move/animate the group. Best for icons, logos and illustrations that are hard to build from primitives. Use a viewBox; keep it clean (no scripts, no images, no filters).",
            json!({"type": "object", "properties": {
                "svg": {"type": "string", "description": "A complete <svg ...>...</svg> document."},
                "name": {"type": "string"},
                "position": {"type": "array", "items": {"type": "number"}, "description": "Where to centre the group (default: composition centre)."},
                "width": {"type": "number", "description": "Scale the artwork to this width in pixels (keeps proportions)."}
            }, "required": ["svg"]}),
        ),
        tool(
            "generate_illustration",
            "Has an image model paint an illustration, then traces it into editable vector shapes (a group). Use for rich, organic or detailed artwork (animals, characters, scenes, hand-drawn looks) that is hard to write as SVG; use add_svg for simple, geometric icons/logos. Costs a little more and takes ~10-30 s.",
            json!({"type": "object", "properties": {
                "prompt": {"type": "string", "description": "What to draw (subject + style hints). It is automatically asked for a flat, trace-friendly style on a plain background."},
                "name": {"type": "string"},
                "colors": {"type": "integer", "description": "Detail of colours when tracing: 2 (posterised, few shapes) … 8 (detailed). Default 5."},
                "position": {"type": "array", "items": {"type": "number"}},
                "width": {"type": "number", "description": "Scale the artwork to this width in pixels."}
            }, "required": ["prompt"]}),
        ),
        tool(
            "simplify_path",
            "Reduces the number of points in a layer's path shapes while keeping their look (useful after add_svg). tolerance = max deviation in pixels (default 1).",
            json!({"type": "object", "properties": {"layer": {"type": "integer"}, "tolerance": {"type": "number"}}, "required": ["layer"]}),
        ),
        tool(
            "align_layers",
            "Aligns layers by their on-screen boxes (exact, better than computing positions yourself). E.g. centre a title and subtitle on the frame, or line up icons' tops.",
            json!({"type": "object", "properties": {
                "layers": {"type": "array", "items": {"type": "integer"}},
                "edge": {"type": "string", "enum": ["left", "h_center", "right", "top", "v_center", "bottom"]},
                "to": {"description": "\"composition\" (the frame), \"selection\" (the layers' combined box), or a layer id to align to (that layer stays put). Default composition."}
            }, "required": ["layers", "edge"]}),
        ),
        tool(
            "distribute_layers",
            "Spaces 3+ layers so the gaps between their boxes are equal (first and last stay put).",
            json!({"type": "object", "properties": {
                "layers": {"type": "array", "items": {"type": "integer"}, "minItems": 3},
                "axis": {"type": "string", "enum": ["horizontal", "vertical"]}
            }, "required": ["layers", "axis"]}),
        ),
        tool(
            "check_layout",
            "Measures the layout exactly at a time (default: the end, i.e. the settled layout): each visible layer's box, offset from the frame centre, margins to the edges, text size, text contrast against what's behind it, and overlaps. Use this — not look_at_frame — for alignment, spacing and legibility questions.",
            json!({"type": "object", "properties": {"time": {"type": "number"}}}),
        ),
        tool(
            "apply_preset",
            "Applies a ready-made motion preset (writes normal keyframes, replacing the animation of the properties it uses). Much more reliable than hand-made keyframes for these effects. squash_stretch works on an existing bouncing/impacting position animation (e.g. after bounce). draw_on makes shape outlines draw themselves (trim paths; best with a stroke).",
            json!({"type": "object", "properties": {
                "layers": {"type": "array", "items": {"type": "integer"}},
                "preset": {"type": "string", "enum": ["fade_in", "fade_out", "pop_in", "slide_in", "bounce", "squash_stretch", "wiggle", "draw_on"]},
                "start": {"type": "number", "description": "Seconds. Default 0."},
                "stagger": {"type": "number", "description": "Extra delay per layer, seconds (for groups)."},
                "duration": {"type": "number"},
                "overshoot": {"type": "number", "description": "pop_in: 0.15 = 115%."},
                "from": {"type": "string", "enum": ["left", "right", "up", "down"], "description": "slide_in direction it comes FROM."},
                "distance": {"type": "number", "description": "slide_in, pixels."},
                "height": {"type": "number", "description": "bounce drop height, pixels."},
                "bounces": {"type": "integer"},
                "strength": {"type": "number", "description": "squash_stretch 0..0.6 (0.25 is natural)."},
                "amount": {"type": "number", "description": "wiggle pixels."},
                "frequency": {"type": "number", "description": "wiggle moves per second."}
            }, "required": ["layers", "preset"]}),
        ),
        tool(
            "precompose",
            "Converts layers into a symbol: a reusable nested animation (like an SVG <symbol>) shown here as ONE layer. Nothing moves on screen. Use it to animate a finished piece as a unit (e.g. a whole logo lockup), to reuse it, or to give it its own timing. Returns the new layer and composition ids.",
            json!({"type": "object", "properties": {
                "layers": {"type": "array", "items": {"type": "integer"}},
                "name": {"type": "string"}
            }, "required": ["layers"]}),
        ),
        tool(
            "set_symbol_timing",
            "A placed symbol (precomp layer): when its own timeline starts on this canvas (`starts_at`, seconds) and whether it plays over and over (`looped`) or once.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "starts_at": {"type": "number"},
                "looped": {"type": "boolean"}
            }, "required": ["layer"]}),
        ),
        tool(
            "set_image_playback",
            "An animated image layer (GIF, WebP, APNG; the scene lists its `frames` and `animation_seconds`): playback `speed` (1 = as made), `looped` (else it plays once and holds the last frame) and `offset` (seconds into the animation when the layer comes in).",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "speed": {"type": "number"},
                "looped": {"type": "boolean"},
                "offset": {"type": "number"}
            }, "required": ["layer"]}),
        ),
        tool(
            "open_composition",
            "Switches which composition you are editing — e.g. a precomp's `comp` id to change what's inside it, then back to the main one. All other tools act on the open composition. Returns its scene.",
            json!({"type": "object", "properties": {"comp": {"type": "integer"}}, "required": ["comp"]}),
        ),
        tool(
            "add_slide",
            "Presentations: adds a new slide (the same size as the slide before it) and OPENS it — every other tool now edits the new slide. Empty, or with `layout` a finished templated slide (background, text with entrance animations, accents; bullets wait for a click each) in the deck's theme or `theme`, filled from `title`/`subtitle`/`items`. If the project isn't a presentation yet, the current main composition becomes slide 1. Returns the slide number and its scene.",
            json!({"type": "object", "properties": {
                "after": {"type": "integer", "description": "Slide number (1-based) to insert after. Default: at the end."},
                "layout": {"type": "string", "enum": ["title", "section", "bullets", "two_column", "image_caption", "full_bleed_video", "quote", "big_stat", "process", "comparison", "closing"]},
                "theme": {"type": "string", "enum": ["Dark Neon", "Clean Light", "Bold Gradient", "Corporate", "Retro"], "description": "With `layout`: its look (default the deck's theme). Use apply_theme to restyle a whole deck."},
                "title": {"type": "string", "description": "With `layout`: the title (Quote: the quote; Big Stat: the number)."},
                "subtitle": {"type": "string", "description": "With `layout`: subtitle (Quote: who said it; Big Stat: what it means; Section: its number)."},
                "items": {"type": "array", "items": {"type": "string"}, "description": "With `layout`: bullets, process steps, or two columns (first line heading, then lines separated by \n); Closing: a contact line."},
                "name": {"type": "string", "description": "Short title, shown in the slide list."},
                "notes": {"type": "string", "description": "Speaker notes."},
                "transition": transition_schema(),
                "duration": {"type": "number", "description": "Seconds of animation on this slide (it then holds its last frame)."},
                "background": {"type": "string", "description": "\"#rrggbb\""}
            }}),
        ),
        tool(
            "apply_theme",
            "Presentations: restyles every templated layer of every slide (background, titles, body, accents) in a theme and sets the slide transitions to match. Layers you drew yourself keep their colours unless they have a theme role.",
            json!({"type": "object", "properties": {
                "theme": {"type": "string", "enum": ["Dark Neon", "Clean Light", "Bold Gradient", "Corporate", "Retro"]}
            }, "required": ["theme"]}),
        ),
        tool(
            "open_slide",
            "Presentations: opens slide `slide` (1-based) for editing. Returns its scene.",
            json!({"type": "object", "properties": {"slide": {"type": "integer"}}, "required": ["slide"]}),
        ),
        tool(
            "edit_slide",
            "Presentations: changes a slide's name, speaker notes or transitions. Only pass what should change.",
            json!({"type": "object", "properties": {
                "slide": {"type": "integer", "description": "1-based. Default: the open slide."},
                "name": {"type": "string"},
                "notes": {"type": "string"},
                "transition_in": transition_schema(),
                "transition_out": transition_schema()
            }}),
        ),
        tool(
            "add_mask",
            "Adds a mask to a layer (or group): only what's inside the mask shows — the way to crop a picture or reveal part of a layer. Coordinates are composition pixels (they're stored relative to the layer so the mask moves with it). Several masks combine in order by `mode`. Animate a mask with `animate` on masks.<n>.feather or masks.<n>.opacity (n = its index); for a wipe, animate the layer under a fixed mask or use a matte instead.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "shape": {"type": "string", "enum": ["rect", "ellipse", "path"], "description": "Default rect."},
                "rect": {"type": "array", "items": {"type": "number"}, "description": "[x0, y0, x1, y1] for rect / ellipse."},
                "path": {"type": "string", "description": "SVG path data for shape \"path\"."},
                "mode": {"type": "string", "enum": ["add", "subtract", "intersect", "difference"], "description": "Default add."},
                "inverted": {"type": "boolean", "description": "Show what's outside instead."},
                "feather": {"type": "number", "description": "Soft edge, pixels."},
                "opacity": {"type": "number", "description": "0..1"},
                "name": {"type": "string"}
            }, "required": ["layer"]}),
        ),
        tool(
            "animate_text",
            "Makes a text layer's letters, words or lines animate in one after another (After Effects-style text animators), e.g. a title typing on, popping in letter by letter or rising word by word. Start from a `preset`, then override anything. Each unit starts `stagger` seconds after the previous one (in `order`) and takes `duration`, moving from the `from` state to rest. `out` makes them leave instead. Several animators on one layer add up. Prefer this over splitting text into one layer per letter.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "preset": {"type": "string", "enum": xenify_doc::text_anim::PRESETS},
                "start": {"type": "number", "description": "Seconds (layer time) when the first unit starts. Default the current time."},
                "unit": {"type": "string", "enum": ["letter", "word", "line"]},
                "order": {"type": "string", "enum": ["forward", "reverse", "center", "random"]},
                "stagger": {"type": "number", "description": "Seconds between units."},
                "duration": {"type": "number", "description": "Seconds each unit takes."},
                "easing": {"type": "string", "description": "An easing preset name (ease_out, spring, …)."},
                "from": {"type": "object", "description": "Start state: offset [dx, dy] px, scale (1 = full), rotation degrees, opacity 0..1, blur px, tracking px per letter.", "properties": {
                    "offset": {"type": "array", "items": {"type": "number"}},
                    "scale": {"type": "number"}, "rotation": {"type": "number"}, "opacity": {"type": "number"},
                    "blur": {"type": "number"}, "tracking": {"type": "number"}
                }},
                "out": {"type": "boolean"}
            }, "required": ["layer"]}),
        ),
        tool(
            "add_effect",
            "Adds a ready-made effect from the effect gallery at `start`: confetti, fireworks, sparkle, snow, rain, bubbles, fog, fire (campfire, torch, wall, burning shape, trails), smoke, explosions (Explosion, Big Blast with a mushroom cloud and camera shake), water filling the screen or a layer, a layer shattering out or in, or particles assembling into a layer's shape. Effects that work on a layer need `target`. The result is ordinary layers you can change afterwards (add_particles, add_water, add_explosion and add_modifier give finer control).",
            json!({"type": "object", "properties": {
                "effect": {"type": "string", "enum": xenify_ops::effects::names().collect::<Vec<_>>()},
                "target": {"type": "integer", "description": "The layer it works on (Burning Shape, Water Fill Layer, Shatter Out/In, Assemble into Shape)."},
                "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y] where it goes (default: the effect's usual spot)."},
                "start": {"type": "number", "description": "Seconds. Default the current time."}
            }, "required": ["effect"]}),
        ),
        tool(
            "add_particles",
            "Adds a particle effect layer (confetti, fireworks, sparkle, snow, rain, bubbles, or particles that fly in and form a layer's shape). Start from a `preset` and override what you need. Particles are drawn by the program every frame (seeded, so the same each time). Aim with `direction` (degrees: 0 right, -90 up, 90 down) and `speed`; `end` sets their final look: fade, shrink, settle on a floor, or form the outline of layer `target` (a logo assembling from particles). Animate later with animate on emitter.count / .rate / .direction / .spread / .speed / .gravity / .size etc.",
            json!({"type": "object", "properties": {
                "preset": {"type": "string", "enum": xenify_doc::PARTICLE_PRESETS.iter().chain(xenify_doc::FIRE_PRESETS).collect::<Vec<_>>(), "description": "Burning Shape needs `target` (the layer that burns)."},
                "name": {"type": "string"},
                "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y] where they come from."},
                "start": {"type": "number", "description": "Seconds when emitting starts. Default the current time."},
                "count": {"type": "number", "description": "Particles per burst."},
                "rate": {"type": "number", "description": "Particles per second, for streams."},
                "direction": {"type": "number"},
                "spread": {"type": "number", "description": "Degrees of the cone around the direction; 360 = all round."},
                "speed": {"type": "number", "description": "Pixels per second."},
                "lifetime": {"type": "number", "description": "Seconds each particle lives."},
                "size": {"type": "number", "description": "Pixels."},
                "gravity": {"type": "number", "description": "Downward pull in px/s² (negative rises)."},
                "colors": {"type": "array", "items": {"type": "string"}, "description": "CSS colours; each particle takes one."},
                "particle": {"type": "string", "enum": ["confetti", "circle", "square", "star", "streak", "soft", "ring"], "description": "Or give `sprite` instead."},
                "sprite": {"type": "integer", "description": "A layer drawn as every particle (e.g. an emoji or logo)."},
                "end": {"type": "string", "enum": ["fade", "shrink", "settle", "form"]},
                "floor": {"type": "number", "description": "y for end = settle."},
                "target": {"type": "integer", "description": "Layer to form for end = form, or to burn for Burning Shape."}
            }, "required": ["preset"]}),
        ),
        tool(
            "add_explosion",
            "Adds an explosion as a group of ordinary layers you can change afterwards: kind explosion (fire, sparks, smoke, a shockwave ring) or big_blast (a nuclear-style mushroom cloud: flash, ground shockwave, fireball, rising stem, rolling cap, dust and debris). Both shake the active camera (adding a camera if there's none).",
            json!({"type": "object", "properties": {
                "kind": {"type": "string", "enum": ["explosion", "big_blast"]},
                "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y]; for big_blast the ground point under it."},
                "start": {"type": "number", "description": "Seconds. Default the current time."},
                "size": {"type": "number", "description": "Scale, 1 = sized for a 1080-pixel-high canvas."}
            }, "required": ["kind"]}),
        ),
        tool(
            "add_water",
            "Adds water that rises to fill the whole canvas (omit `target`) or the shape of layer `target` (a title or logo: the target becomes the water's mask). It has waves, bubbles and a foam line; the level rises from empty over `seconds` starting at `start`. Change it later with animate on liquid.level (0..1), liquid.tilt, liquid.wave_height, liquid.color.",
            json!({"type": "object", "properties": {
                "target": {"type": "integer"},
                "start": {"type": "number", "description": "Seconds. Default the current time."},
                "seconds": {"type": "number", "description": "How long it takes to fill. Default 3."},
                "level": {"type": "number", "description": "How full it ends, 0..1. Default nearly full."},
                "color": {"type": "string", "description": "CSS colour of the water."}
            }}),
        ),
        tool(
            "add_emoji",
            "Adds an emoji (the bundled Fluent Emoji set, MIT) as an editable vector group, by glyph (🎉) or name (party popper): recolour and animate it like any shapes. With `confetti` true it becomes a burst of particles made of that emoji instead (the emoji is added hidden as their sprite). With `animated` true it's an emoji that moves by itself (Noto Emoji Animation, CC BY 4.0, downloaded once): added as a symbol starting at `start`, looping unless `loop` is false.",
            json!({"type": "object", "properties": {
                "emoji": {"type": "string"},
                "animated": {"type": "boolean"},
                "loop": {"type": "boolean", "description": "Animated emoji: play over and over (default true)."},
                "position": {"type": "array", "items": {"type": "number"}},
                "size": {"type": "number", "description": "Pixels across. Default a quarter of the canvas."},
                "confetti": {"type": "boolean"},
                "start": {"type": "number", "description": "Seconds, for confetti and animated emoji. Default the current time."}
            }, "required": ["emoji"]}),
        ),
        tool(
            "add_gif",
            "Searches GIPHY (with the user's own key) and adds the first GIF for `query` as an animated image layer (it loops; use set_image_playback to change that). Only when the user asks for a GIF; they are responsible for the rights. Returns the layer and the credit (title and maker) to mention.",
            json!({"type": "object", "properties": {
                "query": {"type": "string"},
                "position": {"type": "array", "items": {"type": "number"}},
                "size": {"type": "number", "description": "Pixels across (longest side). Default half the canvas."}
            }, "required": ["query"]}),
        ),
        tool(
            "add_sound",
            "Adds a sound the program makes itself (no recordings, no rights issues): a sound effect (tick, tock, whoosh, swish, pop, click, chime, riser, boom, crackle, splash, shatter) as an audio layer at `start`, or background music (`effect` = music) with a mood, tempo and key, on this canvas or as the presentation's `soundtrack`. Time effects to the animation: a whoosh as something flies past, a boom on an explosion, ticks on a clock.",
            json!({"type": "object", "properties": {
                "effect": {"type": "string", "enum": ["tick", "tock", "whoosh", "swish", "pop", "click", "chime", "riser", "boom", "crackle", "splash", "shatter", "music"]},
                "start": {"type": "number", "description": "Seconds. Default the current time."},
                "pitch": {"type": "number", "description": "Semitones up (or down)."},
                "length": {"type": "number", "description": "Seconds (effects)."},
                "mood": {"type": "string", "enum": ["uplifting", "calm", "epic", "dark", "smooth"]},
                "tempo": {"type": "number", "description": "Beats per minute (music)."},
                "bars": {"type": "integer", "description": "Music length in bars of 4 beats."},
                "soundtrack": {"type": "boolean", "description": "Music: play across the whole presentation instead."},
                "volume": {"type": "number", "description": "0..1"}
            }, "required": ["effect"]}),
        ),
        tool(
            "edit_media",
            "Changes how a video or audio layer plays: where in the clip it starts (`clip_start`, seconds), `speed` (1 = as recorded, 0.5 slow motion), `loop`, `mute`, `fade_in` / `fade_out` (audio, seconds), and `frame`: fit (whole picture inside the canvas), fill (cover it) or trim (make the layer as long as the clip plays). Keyframe loudness with animate on volume (1 = as recorded). Import clips with File › Import; you can't create them.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "clip_start": {"type": "number"},
                "speed": {"type": "number"},
                "loop": {"type": "boolean"},
                "mute": {"type": "boolean"},
                "fade_in": {"type": "number"},
                "fade_out": {"type": "number"},
                "frame": {"type": "string", "enum": ["fit", "fill", "trim"]}
            }, "required": ["layer"]}),
        ),
        tool(
            "set_camera",
            "Adds a camera to the current canvas (or changes camera `layer`). A camera frames what viewers see when it plays, presents and exports: a canvas-sized window centred on its position, turned by its rotation and magnified by `zoom` (2 = twice as close). A new camera frames the canvas exactly, so nothing changes until you move it. Pan / push in / turn by animating position, camera_zoom and rotation on the camera layer with `animate`. `follow` parents it to a layer so it tracks that layer. `shake` adds handheld wobble or an impact jolt that dies away (explosions). The topmost visible camera is the one used.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer", "description": "An existing camera to change; omit to add one."},
                "name": {"type": "string"},
                "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y] the point at the centre of the picture, composition pixels."},
                "zoom": {"type": "number", "description": "Default 1."},
                "rotation": {"type": "number", "description": "Degrees, clockwise."},
                "follow": {"type": "integer", "description": "Layer id to follow (parent the camera to); 0 stops following."},
                "shake": {"type": "string", "enum": ["none", "handheld", "impact"], "description": "impact = a strong jolt at `shake_start` that fades out."},
                "shake_amount": {"type": "number", "description": "Largest drift in pixels."},
                "shake_start": {"type": "number", "description": "Seconds."},
                "shake_decay": {"type": "number", "description": "How fast it dies away; 0 keeps shaking, 3 is a short jolt."}
            }}),
        ),
        tool(
            "add_array",
            "Repeats a layer (or group) as a live array of copies, each one step on from the last: moved by `offset`, turned by `rotation` and scaled by `scale` around `origin`, fading to `end_opacity`. Rows: offset only. Rings / flowers / clock marks: rotation = 360 / count with origin = the ring's centre. Grids: two arrays (one across, one down). Spirals: rotation plus scale < 1. Far better than making many layers by hand. The copies aren't layers (the layer stays one); change them later with set_property / animate on modifiers.<n>.count, .offset, .rotation, .scale, .origin, .end_opacity (n = its index, in layer space), e.g. animate count to make copies appear.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "count": {"type": "integer", "description": "Copies including the layer itself (2..1000)."},
                "offset": {"type": "array", "items": {"type": "number"}, "description": "[dx, dy] composition pixels per copy. Default [0, 0]."},
                "rotation": {"type": "number", "description": "Degrees per copy, clockwise. Default 0."},
                "scale": {"description": "Size of each copy relative to the one before: a number or [sx, sy]. Default 1."},
                "origin": {"type": "array", "items": {"type": "number"}, "description": "[x, y] composition pixels: the point copies turn and scale around. Default the layer's transform origin."},
                "end_opacity": {"type": "number", "description": "0..1, opacity of the last copy (they fade evenly). Default 1."}
            }, "required": ["layer", "count"]}),
        ),
        tool(
            "add_modifier",
            "Changes a shape layer's (or group's) outline live, without touching its points: jitter (hand-drawn / rough / boiling when `seed` is animated), zig_zag (ridges) or wave (smooth ridges), pucker_bloat (strength < 0 star-like pucker, > 0 puffy bloat), twist (whirl around `centre`), smooth (round every corner), subdivide, simplify, offset_path (grow by `amount`, shrink when negative), mirror (a mirror image across the line through `centre` at `angle`, 90 = upright; `merge` joins the halves), path_array (`count` copies spread along the outline of shape layer `target`, e.g. beads on a curve or dots round a circle; the layer's transform origin goes on the line), blend (`count` steps morphing this shape layer into shape layer `target`: shape, colour, size and place). shatter breaks any layer into `count` pieces that fly apart from `centre` (force `amount` px/s) at `start`, or with direction = in fly together to form it; animate modifiers.<n>.progress (seconds of flight) to control it. Modifiers act in order with add_array: jitter then array = identical copies; array then twist = the whole row twisted. Change or animate later with set_property / animate on modifiers.<n>.amount / .strength / .detail / .seed / .ridges / .rotation / .origin / .levels / .tolerance. Text and images keep their outline.",
            json!({"type": "object", "properties": {
                "layer": {"type": "integer"},
                "type": {"type": "string", "enum": ["jitter", "zig_zag", "wave", "pucker_bloat", "twist", "smooth", "subdivide", "simplify", "offset_path", "mirror", "path_array", "blend", "shatter"]},
                "direction": {"type": "string", "enum": ["out", "in"], "description": "shatter: break apart from `start` (out, default), or fly together ending at `start` (in)."},
                "seconds": {"type": "number", "description": "shatter: how long the pieces fly (default 2)."},
                "target": {"type": "integer", "description": "path_array: the shape layer to follow; blend: the shape layer to morph into."},
                "count": {"type": "integer", "description": "path_array copies (default 8) / blend steps (default 4)."},
                "start": {"type": "number", "description": "path_array: where along the path to begin, 0..1 (default 0). shatter: the second it breaks (default the current time)."},
                "end": {"type": "number", "description": "path_array: where to end, 0..1 (default 1)."},
                "align": {"type": "boolean", "description": "path_array: turn copies with the line (default true)."},
                "merge": {"type": "boolean", "description": "mirror: join the halves into one outline (default false)."},
                "amount": {"type": "number", "description": "Pixels: how far jitter moves points (default 4), the ridge size (default 6), or the offset distance (default 10)."},
                "detail": {"type": "number", "description": "Jitter: points added per 100 px (default 5; 0 = only existing points)."},
                "seed": {"type": "number", "description": "Jitter: which random pattern (default 0)."},
                "ridges": {"type": "integer", "description": "Zig zag / wave: ridges per segment (default 4)."},
                "strength": {"type": "number", "description": "pucker_bloat: -1..1 (default -0.3); smooth: 0..1 (default 1)."},
                "angle": {"type": "number", "description": "Twist: degrees at the centre (default 60). Mirror: the axis direction (default 90 = upright)."},
                "centre": {"type": "array", "items": {"type": "number"}, "description": "Twist centre / a point on the mirror axis: [x, y] composition pixels (default the layer's transform origin)."},
                "levels": {"type": "integer", "description": "Subdivide: 1..6 (default 1)."},
                "tolerance": {"type": "number", "description": "Simplify: pixels (default 1)."},
                "smooth": {"type": "boolean", "description": "Jitter: keep curves (default false)."},
                "corners": {"type": "string", "enum": ["miter", "round", "bevel"], "description": "Offset path corners (default round)."}
            }, "required": ["layer", "type"]}),
        ),
        tool(
            "combine_shapes",
            "Boolean operation: merges the filled shapes of 2+ layers into ONE new path layer that replaces them. union = all of them; difference = the bottom-most layer with the others cut out of it (great for holes, crescents, cut-outs); intersection = overlap only; exclusion = overlaps become holes. Returns the new layer id.",
            json!({"type": "object", "properties": {
                "layers": {"type": "array", "items": {"type": "integer"}, "minItems": 2},
                "op": {"type": "string", "enum": ["union", "difference", "intersection", "exclusion"]}
            }, "required": ["layers", "op"]}),
        ),
        tool(
            "group_layers",
            "Groups layers so they move/scale/rotate/animate together (like SVG <g>). Nothing moves on screen. Returns the group id; animate the group's transform to move them all.",
            json!({"type": "object", "properties": {
                "layers": {"type": "array", "items": {"type": "integer"}},
                "name": {"type": "string"}
            }, "required": ["layers"]}),
        ),
        tool(
            "ungroup",
            "Dissolves a group; its layers keep their on-screen placement.",
            json!({"type": "object", "properties": {"group": {"type": "integer"}}, "required": ["group"]}),
        ),
        tool(
            "delete_layer",
            "Deletes a layer.",
            json!({"type": "object", "properties": {"layer": {"type": "integer"}}, "required": ["layer"]}),
        ),
        tool(
            "reorder_layer",
            "Moves a layer in the stacking order: index 0 is the bottom (drawn first).",
            json!({"type": "object", "properties": {"layer": {"type": "integer"}, "index": {"type": "integer"}}, "required": ["layer", "index"]}),
        ),
        tool(
            "ask_user",
            "Asks the user a question and waits for the answer. Use for genuinely ambiguous briefs, or to let them pick between 2-3 concepts (put each concept in `options`).",
            json!({"type": "object", "properties": {
                "question": {"type": "string"},
                "options": {"type": "array", "items": {"type": "string"}, "description": "Suggested answers shown as buttons (the user may also type)."}
            }, "required": ["question"]}),
        ),
    ];
    tools.extend(crate::diagram_tools::definitions());
    if has_reviewer {
        tools.push(tool(
            "look_at_frame",
            "Renders the composition at `time` and has a vision model answer `question` about it. Use to check layout, overlap, legibility and that the result matches the brief.",
            json!({"type": "object", "properties": {
                "time": {"type": "number"},
                "question": {"type": "string", "description": "What to check, e.g. 'Is the title centred and readable?'"}
            }, "required": ["time", "question"]}),
        ));
    }
    tools
}

fn transition_schema() -> Json {
    let kinds: Vec<String> = xenify_doc::TransitionKind::ALL
        .iter()
        .map(|k| serde_json::to_value(k).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default())
        .collect();
    json!({"type": "object", "description": "How the slide arrives. kind: cut (instant), fade, dip_to_color (through `color`), push / cover / uncover / wipe / blinds / flip / liquid (towards `direction`), clock_wipe, iris, zoom_in, zoom_out, spin, blur_dissolve, shatter, particle_dissolve, smart_morph (layers with the same name on both slides glide from their old place to their new one, the rest fade: name matching layers alike). duration in seconds; softness 0..1 for wipes, iris, clock and blinds; easing a preset name.",
           "properties": {"kind": {"type": "string", "enum": kinds},
                          "duration": {"type": "number"},
                          "direction": {"type": "string", "enum": ["left", "right", "up", "down"]},
                          "color": {"type": "string"},
                          "easing": {"type": "string"},
                          "softness": {"type": "number"}}})
}

fn transition(v: &Json, current: xenify_doc::Transition) -> Result<xenify_doc::Transition, String> {
    let mut t = current;
    let kind = v.get("kind").or(Some(v).filter(|v| v.is_string()));
    if let Some(k) = kind.filter(|k| !k.is_null()) {
        // Through the file format, so old names (push_left, zoom…) still work.
        let old = json!({"kind": k, "duration": t.duration});
        let parsed: xenify_doc::Transition =
            serde_json::from_value(old).map_err(|_| format!("unknown transition kind {k}"))?;
        t.kind = parsed.kind;
        if !matches!(k.as_str(), Some(s) if xenify_doc::TransitionKind::ALL.iter().any(|x| serde_json::to_value(x).ok().as_ref().and_then(|v| v.as_str()) == Some(s)))
        {
            t.direction = parsed.direction;
        }
    }
    if let Some(d) = values::number(&v["duration"]) {
        t.duration = d.clamp(0.0, 10.0);
    }
    if let Some(d) = v.get("direction").and_then(Json::as_str) {
        t.direction = serde_json::from_value(json!(d)).map_err(|_| format!("unknown direction {d:?}"))?;
    }
    if let Some(c) = v.get("color").filter(|c| !c.is_null()) {
        t.color = values::color(c)?;
    }
    if let Some(e) = v.get("easing").and_then(Json::as_str) {
        t.easing = xenify_doc::Easing::preset(e).ok_or_else(|| format!("unknown easing {e:?}"))?;
    }
    if let Some(s) = values::number(&v["softness"]) {
        t.softness = s.clamp(0.0, 1.0);
    }
    Ok(t)
}

/// The open slide's index, if the project is a presentation.
fn open_slide_index(host: &dyn ToolHost) -> Option<usize> {
    host.project().slide_index(host.comp())
}

/// Opens slide `index` and describes it.
fn show_slide(host: &mut dyn ToolHost, index: usize, activity: String) -> Result<ToolOutcome, String> {
    let slides = xenify_ops::slides::slides_or_main(host.project());
    let slide = slides.get(index).ok_or_else(|| format!("there are {} slides", slides.len()))?;
    host.set_comp(slide.comp)?;
    let (project, time) = (host.project().clone(), host.time());
    let mut result = scene_summary(&project, slide.comp, time, &mut HostMeasure(host));
    result["slide"] = json!(index + 1);
    Ok(done(result, activity))
}

fn tool(name: &str, description: &str, parameters: Json) -> Json {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}

fn layer_id(args: &Json) -> Result<Id, String> {
    values::number(&args["layer"]).map(|n| Id(n as u64)).ok_or_else(|| "missing integer `layer` id".into())
}

fn opt_f64(args: &Json, key: &str) -> Option<f64> {
    args.get(key).and_then(values::number)
}

/// Resolves friendly property names (`fill`, `size`, …) for `layer`.
pub fn resolve_prop(layer: &Layer, name: &str) -> Result<PropPath, String> {
    if let Ok(p) = name.parse::<PropPath>() {
        return Ok(p);
    }
    let first = |pred: &dyn Fn(&ShapeItem) -> bool| layer.shapes().iter().position(pred);
    let field = match name {
        "fill" | "fill_color" | "color" if matches!(layer.kind, LayerKind::Text(_)) => return Ok(PropPath::TextColor),
        "fill" | "fill_color" | "color" => (first(&|s| s.fill.is_some()), ShapeField::FillColor),
        "stroke" | "stroke_color" => (first(&|s| s.stroke.is_some()), ShapeField::StrokeColor),
        "stroke_width" => (first(&|s| s.stroke.is_some()), ShapeField::StrokeWidth),
        "size" => (first(&|s| !matches!(s.geometry, Geometry::Path { .. })), ShapeField::Size),
        "corner_radius" | "roundness" => {
            (first(&|s| matches!(s.geometry, Geometry::Rect { .. })), ShapeField::CornerRadius)
        }
        "path" => (first(&|s| matches!(s.geometry, Geometry::Path { .. })), ShapeField::Path),
        "offset" => (Some(0), ShapeField::Position),
        "gradient" | "gradient_colors" | "gradient_start" | "gradient_end" => (
            first(&|s| s.fill.as_ref().is_some_and(|f| f.gradient.is_some())),
            match name {
                "gradient_start" => ShapeField::GradientStart,
                "gradient_end" => ShapeField::GradientEnd,
                _ => ShapeField::GradientColors,
            },
        ),
        "trim_start" => (first(&|s| s.trim.is_some()), ShapeField::TrimStart),
        "trim_end" => (first(&|s| s.trim.is_some()), ShapeField::TrimEnd),
        "trim_offset" => (first(&|s| s.trim.is_some()), ShapeField::TrimOffset),
        _ => return Err(format!("unknown property {name:?}")),
    };
    match field {
        (Some(index), field) => Ok(PropPath::Shape { index, field }),
        (None, _) => Err(format!("layer {} has no {name}", layer.id)),
    }
}

/// Composition → parent space for a layer's position (identity when unparented).
fn parent_inverse(comp: &xenify_doc::Composition, layer: &Layer, time: f64) -> Affine {
    layer.meta.parent.map_or(Affine::IDENTITY, |p| world_transform(comp, p, time).inverse())
}

/// Parses a model value for `path` on `layer` at `time`. The model works in
/// composition coordinates: SVG paths are converted into layer space and
/// positions of parented layers into the parent's space.
fn parse_value(host: &dyn ToolHost, layer: &Layer, path: PropPath, v: &Json, time: f64) -> Result<Value, String> {
    let kind = layer.property_kind(path).map_err(|e| e.to_string())?;
    if kind == ValueKind::Stops {
        let css = v.as_str().ok_or("gradient colours must be a CSS linear-gradient()/radial-gradient() string")?;
        let (_, _, stops) = values::css_gradient(css).ok_or("expected a CSS gradient string")??;
        return Ok(Value::Stops(stops));
    }
    let value = values::value(kind, v)?;
    let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
    Ok(match value {
        Value::Path(p) => Value::Path(world_transform(comp, layer.id, time).inverse() * p),
        Value::Vec2(p) if path == PropPath::Position => {
            Value::Vec2((parent_inverse(comp, layer, time) * p.to_point()).to_vec2())
        }
        other => other,
    })
}

fn easing_json(e: Easing) -> Json {
    match Easing::PRESETS.iter().find(|(_, p)| *p == e) {
        Some((name, _)) => json!(name),
        None => match e {
            Easing::Bezier { x1, y1, x2, y2 } => json!([x1, y1, x2, y2]),
            other => json!(other.css()),
        },
    }
}

/// Compact JSON description of the composition for the model, including each
/// visible layer's on-screen bounding box at `time` so it can lay things out.
pub fn scene_summary(
    project: &xenify_doc::Project,
    comp_id: Id,
    time: f64,
    measure: &mut dyn xenify_anim::TextMeasure,
) -> Json {
    let Ok(comp) = project.comp(comp_id) else { return json!({"error": "no composition"}) };
    let s = &comp.settings;
    // On-screen boxes now and at the end (the settled layout after intros).
    let end = (s.duration - 1.0 / s.fps).max(0.0);
    let mut boxes = |t: f64| -> std::collections::HashMap<Id, Json> {
        xenify_anim::evaluate_project(project, comp_id, t)
            .layers
            .iter()
            .filter_map(|l| {
                let r = l.transform.transform_rect_bbox(l.bounds(measure)?);
                Some((l.id, json!([r.x0.round(), r.y0.round(), r.x1.round(), r.y1.round()])))
            })
            .collect()
    };
    let bounds = boxes(time);
    let bounds_end = boxes(end);
    let layers: Vec<Json> = comp
        .layers
        .iter()
        .enumerate()
        .map(|(index, l)| {
            let kind = match &l.kind {
                LayerKind::Shape { shapes } => match shapes.first().map(|s| &s.geometry) {
                    Some(Geometry::Rect { .. }) => "rect",
                    Some(Geometry::Ellipse { .. }) => "ellipse",
                    Some(Geometry::Path { .. }) => "path",
                    Some(Geometry::Figure { kind, .. }) => match kind {
                        xenify_doc::primitives::FigureKind::Polygon => "polygon",
                        xenify_doc::primitives::FigureKind::Star => "star",
                        xenify_doc::primitives::FigureKind::Gear => "gear",
                    },
                    None => "shape",
                },
                LayerKind::Text(_) => "text",
                LayerKind::Null => "null",
                LayerKind::Precomp { .. } => "precomp",
                LayerKind::Group => "group",
                LayerKind::Solid(_) => "solid3d",
                LayerKind::Image { .. } => "image",
                LayerKind::Camera(_) => "camera",
                LayerKind::Particles(_) => "particles",
                LayerKind::Liquid(_) => "liquid",
                LayerKind::Video(_) => "video",
                LayerKind::Audio(_) => "audio",
            };
            let mut props = serde_json::Map::new();
            for path in l.property_paths() {
                let Ok(p) = l.property(path) else { continue };
                let to_comp = |v: Value, t: f64| match v {
                    Value::Path(bp) => Value::Path(world_transform(comp, l.id, t) * bp),
                    Value::Vec2(p) if path == PropPath::Position => {
                        Value::Vec2((parent_inverse(comp, l, t).inverse() * p.to_point()).to_vec2())
                    }
                    other => other,
                };
                let entry = if p.is_animated() {
                    let keys: Vec<Json> = p
                        .keyframe_times()
                        .into_iter()
                        .map(|t| {
                            let v = to_comp(l.value_at(path, t).unwrap(), t);
                            json!([t, values::to_json(&v), easing_json(p.easing_at(t).unwrap_or_default())])
                        })
                        .collect();
                    json!({"keyframes": keys})
                } else {
                    let v = l.value_at(path, time).unwrap();
                    // Skip defaults to keep the summary short.
                    let default = matches!(
                        (path, &v),
                        (PropPath::Anchor, Value::Vec2(a)) if *a == Vec2::ZERO
                    ) || matches!((path, &v), (PropPath::Scale, Value::Vec2(a)) if *a == Vec2::new(1.0, 1.0))
                        || matches!((path, &v), (PropPath::Rotation, Value::Scalar(r)) if *r == 0.0)
                        || matches!((path, &v), (PropPath::Opacity, Value::Scalar(o)) if *o == 1.0)
                        || matches!((path, &v), (PropPath::Shape { field: ShapeField::Position, .. }, Value::Vec2(a)) if *a == Vec2::ZERO);
                    if default {
                        continue;
                    }
                    values::to_json(&to_comp(v, time))
                };
                props.insert(path.to_string(), entry);
            }
            let mut o = json!({"id": l.id.0, "index": index, "name": l.meta.name, "type": kind, "properties": props});
            if let Some(c) = &l.meta.connector {
                // Its line is routed automatically: the ends are what matter.
                let end = |e: &xenify_doc::ConnectorEnd| match e.attach {
                    Some(a) => match a.port {
                        xenify_doc::PortRef::Auto => json!({"layer": a.layer.0}),
                        xenify_doc::PortRef::Port(i) => json!({"layer": a.layer.0, "port": i}),
                    },
                    None => json!([e.point.x.round(), e.point.y.round()]),
                };
                o["type"] = json!("connector");
                o["properties"] = json!({});
                o["from"] = end(&c.from);
                o["to"] = end(&c.to);
                o["route"] = serde_json::to_value(c.route).unwrap();
            }
            if let Some(label) = &l.meta.label {
                o["label"] = json!(label.text);
            }
            if let LayerKind::Text(t) = &l.kind {
                o["text"] = json!(t.text);
                o["font_family"] = json!(t.font_family);
                o["align"] = serde_json::to_value(t.align).unwrap();
                o["valign"] = serde_json::to_value(t.valign).unwrap();
            }
            if let Some(p) = l.meta.parent {
                o["parent"] = json!(p.0);
            }
            if let Some(m) = l.meta.matte {
                o["matte"] = json!({"layer": m.layer.0, "mode": m.mode});
            }
            if let LayerKind::Image { asset, playback } = l.kind
                && let Some(a) = project.asset(asset)
                && let Some(frames) = &a.frames
            {
                o["frames"] = json!(frames.len());
                o["animation_seconds"] = json!(a.animation_length());
                o["playback"] = json!({"speed": playback.speed, "looped": playback.looped, "offset": playback.offset});
            }
            if let LayerKind::Precomp { comp, start, looped } = l.kind {
                o["comp"] = json!(comp.0);
                if start != 0.0 {
                    o["comp_starts_at"] = json!(start);
                }
                if looped {
                    o["looped"] = json!(true);
                }
            }
            if !l.meta.expressions.is_empty() {
                o["expressions"] =
                    l.meta.expressions.iter().map(|e| (e.path.to_string(), json!(e.source))).collect::<serde_json::Map<_, _>>().into();
            }
            if let Some(w) = l.transform.wiggle {
                o["wiggle"] = json!({"frequency": w.frequency, "position": w.position, "rotation": w.rotation, "scale": w.scale});
            }
            if comp.layers.iter().any(|o| o.meta.matte.is_some_and(|m| m.layer == l.id)) {
                o["is_matte"] = json!(true);
            }
            if l.meta.in_point > 0.0 {
                o["in_point"] = json!(l.meta.in_point);
            }
            if let Some(out) = l.meta.out_point {
                o["out_point"] = json!(out);
            }
            if !l.meta.visible {
                o["visible"] = json!(false);
            }
            match bounds.get(&l.id) {
                Some(b) => o["bounds_now"] = b.clone(),
                None if !matches!(l.kind, LayerKind::Null | LayerKind::Group) => o["bounds_now"] = json!("not visible at current_time"),
                None => {}
            }
            if let Some(b) = bounds_end.get(&l.id)
                && bounds.get(&l.id) != Some(b)
            {
                o["bounds_end"] = b.clone();
            }
            o
        })
        .collect();
    let mut out = json!({
        "composition": {"id": comp_id.0, "width": s.width, "height": s.height, "fps": s.fps, "duration": s.duration,
                        "background": s.background.to_hex(), "name": s.name,
                        "kind": serde_json::to_value(s.kind).unwrap()},
        "current_time": time,
        "note": "bounds_now = [x0, y0, x1, y1] on screen at current_time; bounds_end = the same at the end of the \
                 composition (the settled layout), shown when different. Composition pixels.",
        "layers_bottom_to_top": layers,
    });
    if !project.slides.is_empty() {
        out["slides"] = project
            .slides
            .iter()
            .enumerate()
            .map(|(i, sl)| {
                let name = project.comp(sl.comp).map(|c| c.settings.name.clone()).unwrap_or_default();
                json!({"slide": i + 1, "comp": sl.comp.0, "name": name, "open": sl.comp == comp_id})
            })
            .collect();
    }
    if project.compositions.len() > 1 {
        out["all_compositions"] = project
            .compositions
            .iter()
            .map(|c| json!({"id": c.id.0, "name": c.settings.name, "open": c.id == comp_id}))
            .collect();
    }
    out
}

/// Runs one tool call.
pub fn run(name: &str, args: &Json, host: &mut dyn ToolHost) -> Result<ToolOutcome, String> {
    if let Some(r) = crate::diagram_tools::run(name, args, host) {
        return r;
    }
    match name {
        "get_scene" => {
            let (project, comp, time) = (host.project().clone(), host.comp(), host.time());
            let result = scene_summary(&project, comp, time, &mut HostMeasure(host));
            Ok(ToolOutcome::Done { result, activity: None })
        }
        "set_composition" => {
            let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
            let mut s: CompSettings = comp.settings.clone();
            if let Some(w) = opt_f64(args, "width") {
                s.width = w.round().max(1.0) as u32;
            }
            if let Some(h) = opt_f64(args, "height") {
                s.height = h.round().max(1.0) as u32;
            }
            if let Some(f) = opt_f64(args, "fps") {
                s.fps = f;
            }
            if let Some(d) = opt_f64(args, "duration") {
                s.duration = d;
            }
            if let Some(b) = args.get("background").filter(|b| !b.is_null()) {
                s.background = values::color(b)?;
            }
            if let Some(on) = args["motion_blur"].as_bool() {
                s.motion_blur = on.then(|| s.motion_blur.unwrap_or_default());
            }
            if let (Some(a), Some(mb)) = (opt_f64(args, "shutter_angle"), s.motion_blur.as_mut()) {
                mb.shutter_angle = a.clamp(1.0, 720.0);
            }
            match args["kind"].as_str() {
                Some("diagram") => xenify_ops::diagram::set_kind(&mut s, xenify_doc::CompKind::Diagram),
                Some("animation") => xenify_ops::diagram::set_kind(&mut s, xenify_doc::CompKind::Animation),
                _ => {}
            }
            let summary = format!("{}×{}, {} fps, {} s", s.width, s.height, s.fps, s.duration);
            host.execute("AI: composition settings", Command::SetCompSettings { comp: host.comp(), settings: s })?;
            Ok(done(json!({"ok": true}), format!("Composition set to {summary}")))
        }
        "add_layer" => add_layer(args, host),
        "animate" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let pname = args["property"].as_str().ok_or("missing `property`")?;
            let path = resolve_prop(&layer, pname)?;
            let kind = layer.property_kind(path).map_err(|e| e.to_string())?;
            let keys = args["keyframes"]
                .as_array()
                .filter(|k| !k.is_empty())
                .ok_or("`keyframes` must be a non-empty array")?;
            if pname == "gradient" || pname.ends_with(".gradient") || pname.ends_with(".stroke_gradient") {
                return animate_gradient(host, &layer, path, keys);
            }
            let mut parsed = Vec::new();
            for k in keys {
                let time = values::number(&k["time"]).ok_or("each keyframe needs a numeric `time`")?.max(0.0);
                let value = parse_value(host, &layer, path, &k["value"], time)?;
                let easing =
                    if k.get("easing").is_some() { values::easing(&k["easing"])? } else { Easing::EASE_IN_OUT };
                parsed.push((time, value, easing));
            }
            let property = animated_property(kind, parsed)?;
            let n = keys.len();
            host.execute(&format!("AI: animate {pname}"), Command::SetProperty { layer: id, path, property })?;
            Ok(done(
                json!({"ok": true, "keyframes": n}),
                format!("Animated {} of “{}” ({n} keyframes)", pname, layer.meta.name),
            ))
        }
        "set_values" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let map = args["values"].as_object().ok_or("`values` must be an object")?;
            let mut cmds = Vec::new();
            for (pname, v) in map {
                if matches!(pname.as_str(), "fill" | "fill_color" | "color" | "gradient")
                    && let Some(g) = v.as_str().and_then(values::css_gradient)
                {
                    let (kind, angle, stops) = g?;
                    let index = layer.shapes().iter().position(|s| s.fill.is_some()).ok_or("layer has no fill")?;
                    let mut shape = layer.shapes()[index].clone();
                    let b = xenify_anim::geometry_path(&shape.geometry, host.time()).bounding_box();
                    let f = shape.fill.as_mut().unwrap();
                    f.color = stops[0].color.into();
                    f.gradient = Some(xenify_doc::Gradient::across(b, kind, angle, stops).into());
                    cmds.push(Command::SetShape { layer: id, index, shape });
                    continue;
                }
                let path = resolve_prop(&layer, pname)?;
                let value = parse_value(host, &layer, path, v, host.time())?;
                cmds.push(Command::SetProperty { layer: id, path, property: static_property(value) });
            }
            let names: Vec<_> = map.keys().cloned().collect();
            host.execute("AI: set values", Command::batch(cmds))?;
            let result = with_warnings(host, id, json!({"ok": true}));
            Ok(done(result, format!("Set {} on “{}”", names.join(", "), layer.meta.name)))
        }
        "update_layer" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let mut meta: LayerMeta = layer.meta.clone();
            if let Some(n) = args["name"].as_str() {
                meta.name = n.to_string();
            }
            if let Some(p) = args.get("parent") {
                meta.parent = values::number(p).map(|n| Id(n as u64));
            }
            if let Some(t) = opt_f64(args, "in_point") {
                meta.in_point = t.max(0.0);
            }
            if let Some(o) = args.get("out_point") {
                meta.out_point = values::number(o);
            }
            if let Some(v) = args["visible"].as_bool() {
                meta.visible = v;
            }
            if let Some(m) = args.get("matte") {
                meta.matte =
                    values::number(m).map(|n| xenify_doc::Matte { layer: Id(n as u64), mode: Default::default() });
            }
            if let Some(mode) = args.get("matte_mode").filter(|m| !m.is_null()) {
                let mode = serde_json::from_value(mode.clone())
                    .map_err(|_| "matte_mode must be alpha, alpha_inverted, luma or luma_inverted")?;
                meta.matte = meta.matte.map(|m| xenify_doc::Matte { mode, ..m });
            }
            if let Some(map) = args["expressions"].as_object() {
                let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?.clone();
                for (name, source) in map {
                    let path = resolve_prop(&layer, name)?;
                    meta.expressions.retain(|e| e.path != path);
                    let Some(source) = source.as_str().filter(|s| !s.trim().is_empty()) else { continue };
                    // Try it now so mistakes come back as errors, not silent no-ops.
                    xenify_anim::expr::preview(&comp, id, source, path, host.time())
                        .map_err(|e| format!("expression for {name}: {e}"))?;
                    meta.expressions.push(xenify_doc::Expression { path, source: source.to_string() });
                }
            }
            let mut cmds = vec![Command::SetLayerMeta { layer: id, meta }];
            let pen = match args.get("pen").filter(|p| !p.is_null()) {
                Some(Json::String(name)) => {
                    let kind: xenify_doc::PenKind =
                        serde_json::from_value(json!(name.trim().to_lowercase().replace([' ', '-'], "_")))
                            .map_err(|_| format!("unknown pen {name:?}"))?;
                    Some(xenify_doc::PenStyle::preset(kind))
                }
                Some(obj @ Json::Object(o)) => {
                    let kind: xenify_doc::PenKind =
                        serde_json::from_value(o.get("kind").cloned().unwrap_or(json!("solid")))
                            .map_err(|_| "pen.kind must be a pen name")?;
                    // Start from the pen's preset and apply the given settings.
                    let mut base = serde_json::to_value(xenify_doc::PenStyle::preset(kind)).unwrap();
                    for (k, v) in o {
                        base[k] = v.clone();
                    }
                    let _ = obj;
                    Some(
                        serde_json::from_value::<xenify_doc::PenStyle>(base)
                            .map_err(|e| format!("bad pen settings: {e}"))?,
                    )
                }
                Some(_) => return Err("pen must be a name or an object".into()),
                None => None,
            };
            let cap = match args["cap"].as_str() {
                Some("flat") => Some((xenify_doc::LineCap::Butt, 0.0)),
                Some("square") => Some((xenify_doc::LineCap::Square, 0.0)),
                Some("round") => Some((xenify_doc::LineCap::Round, 0.0)),
                Some("rounded") => {
                    Some((xenify_doc::LineCap::Square, opt_f64(args, "cap_roundness").unwrap_or(0.5).clamp(0.0, 1.0)))
                }
                Some(other) => return Err(format!("cap must be flat, square, round or rounded, not {other:?}")),
                None => opt_f64(args, "cap_roundness").map(|r| (xenify_doc::LineCap::Square, r.clamp(0.0, 1.0))),
            };
            if pen.is_some() || cap.is_some() {
                let mut any = false;
                for (index, shape) in layer.shapes().iter().enumerate() {
                    let Some(stroke) = &shape.stroke else { continue };
                    let mut s = shape.clone();
                    let mut k = stroke.clone();
                    if let Some(p) = &pen {
                        k.style = p.clone();
                    }
                    if let Some((c, r)) = cap {
                        (k.cap, k.cap_roundness) = (c, r);
                    }
                    s.stroke = Some(k);
                    cmds.push(Command::SetShape { layer: id, index, shape: s });
                    any = true;
                }
                if !any {
                    return Err("this layer has no stroke — set stroke and stroke_width first (e.g. set_values)".into());
                }
            }
            if let Some(w) = args.get("wiggle") {
                let wiggle = match w {
                    Json::Null => None,
                    Json::Bool(false) => None,
                    w => {
                        let f = |k: &str| w.get(k).and_then(values::number);
                        let base = layer.transform.wiggle.unwrap_or_default();
                        Some(xenify_doc::Wiggle {
                            frequency: f("frequency").unwrap_or(base.frequency).clamp(0.05, 30.0),
                            position: f("position").unwrap_or(if layer.transform.wiggle.is_some() {
                                base.position
                            } else {
                                0.0
                            }),
                            rotation: f("rotation").unwrap_or(if layer.transform.wiggle.is_some() {
                                base.rotation
                            } else {
                                0.0
                            }),
                            scale: f("scale").unwrap_or(base.scale),
                            seed: layer.transform.wiggle.map_or(id.0, |b| b.seed),
                        })
                    }
                };
                cmds.push(Command::SetWiggle { layer: id, wiggle });
            }
            if let Some(t) = args["text"].as_str() {
                cmds.push(Command::SetText { layer: id, text: values::plain_text(t) });
            }
            if let Some(f) = args["font_family"].as_str() {
                cmds.push(Command::SetFontFamily { layer: id, family: f.to_string() });
            }
            if let LayerKind::Text(t) = &layer.kind {
                let align = text_align(&args["align"])?;
                let valign = text_valign(&args["valign"])?;
                if align.is_some() || valign.is_some() {
                    cmds.push(Command::SetTextAlign {
                        layer: id,
                        align: align.unwrap_or(t.align),
                        valign: valign.unwrap_or(t.valign),
                    });
                }
            }
            host.execute("AI: update layer", Command::batch(cmds))?;
            let result = with_warnings(host, id, json!({"ok": true}));
            Ok(done(result, format!("Updated “{}”", layer.meta.name)))
        }
        "align_layers" | "distribute_layers" => {
            use xenify_ops::arrange::{self, AlignEdge, AlignTo, Axis};
            let ids: Vec<Id> = args["layers"]
                .as_array()
                .ok_or("`layers` must be an array of layer ids")?
                .iter()
                .filter_map(values::number)
                .map(|n| Id(n as u64))
                .collect();
            let (project, comp, time) = (host.project().clone(), host.comp(), host.time());
            let cmd = if name == "align_layers" {
                let edge: AlignEdge = serde_json::from_value(args["edge"].clone())
                    .map_err(|_| "edge must be left, h_center, right, top, v_center or bottom")?;
                let to = match &args["to"] {
                    Json::Null => AlignTo::Composition,
                    Json::String(t) if t == "selection" => AlignTo::Selection,
                    Json::String(t) if t == "composition" => AlignTo::Composition,
                    other => AlignTo::Layer(Id(values::number(other).ok_or("bad `to`")? as u64)),
                };
                arrange::align(&project, comp, &ids, edge, to, time, false, &mut HostMeasure(host))
            } else {
                let axis: Axis =
                    serde_json::from_value(args["axis"].clone()).map_err(|_| "axis must be horizontal or vertical")?;
                arrange::distribute(&project, comp, &ids, axis, time, false, &mut HostMeasure(host))
            }
            .map_err(|e| e.to_string())?;
            let what = if name == "align_layers" { "Aligned" } else { "Distributed" };
            match cmd {
                Some(cmd) => {
                    host.execute(&format!("AI: {what} layers"), cmd)?;
                    let mut result = json!({"ok": true});
                    let warnings: Vec<String> = ids.iter().flat_map(|id| layout_warnings(host, *id)).collect();
                    if !warnings.is_empty() {
                        result["layout_warnings"] = json!(warnings);
                    }
                    Ok(done(result, format!("{what} {} layers", ids.len())))
                }
                None => Ok(done(
                    json!({"ok": true, "note": "already aligned"}),
                    format!("{what} {} layers (no change)", ids.len()),
                )),
            }
        }
        "apply_preset" => {
            use xenify_ops::presets::{self, Direction, Preset};
            let ids: Vec<Id> = args["layers"]
                .as_array()
                .ok_or("`layers` must be an array of layer ids")?
                .iter()
                .filter_map(values::number)
                .map(|n| Id(n as u64))
                .collect();
            let name = args["preset"].as_str().ok_or("missing `preset`")?;
            let label = name.replace('_', " ");
            let (_, default) = Preset::DEFAULTS
                .iter()
                .find(|(n, _)| {
                    n.to_lowercase().replace([' ', '&'], "_").replace("__", "_") == name
                        || (name == "squash_stretch" && n.starts_with("Squash"))
                })
                .ok_or_else(|| format!("unknown preset {name:?}"))?;
            let f = |k: &str| opt_f64(args, k);
            let preset = match *default {
                Preset::FadeIn { duration } => Preset::FadeIn { duration: f("duration").unwrap_or(duration) },
                Preset::FadeOut { duration } => Preset::FadeOut { duration: f("duration").unwrap_or(duration) },
                Preset::PopIn { duration, overshoot } => Preset::PopIn {
                    duration: f("duration").unwrap_or(duration),
                    overshoot: f("overshoot").unwrap_or(overshoot),
                },
                Preset::SlideIn { duration, from, distance } => Preset::SlideIn {
                    duration: f("duration").unwrap_or(duration),
                    from: match args["from"].as_str() {
                        Some("left") => Direction::Left,
                        Some("right") => Direction::Right,
                        Some("up") | Some("top") => Direction::Up,
                        Some("down") | Some("bottom") => Direction::Down,
                        _ => from,
                    },
                    distance: f("distance").unwrap_or(distance),
                },
                Preset::Bounce { duration, height, bounces } => Preset::Bounce {
                    duration: f("duration").unwrap_or(duration),
                    height: f("height").unwrap_or(height),
                    bounces: f("bounces").map_or(bounces, |b| b.max(0.0) as u32),
                },
                Preset::SquashStretch { strength } => {
                    Preset::SquashStretch { strength: f("strength").unwrap_or(strength) }
                }
                Preset::DrawOn { duration } => Preset::DrawOn { duration: f("duration").unwrap_or(duration) },
                Preset::Wiggle { duration, amount, frequency } => Preset::Wiggle {
                    duration: f("duration").unwrap_or(duration),
                    amount: f("amount").unwrap_or(amount),
                    frequency: f("frequency").unwrap_or(frequency),
                },
            };
            let start = f("start").unwrap_or(0.0).max(0.0);
            let cmd = presets::apply_many(host.project(), &ids, preset, start, f("stagger").unwrap_or(0.0))
                .map_err(|e| e.to_string())?;
            host.execute(&format!("AI: {label}"), cmd)?;
            Ok(done(json!({"ok": true}), format!("Applied {label} to {} layer(s)", ids.len())))
        }
        "precompose" => {
            let ids: Vec<Id> = args["layers"]
                .as_array()
                .ok_or("`layers` must be an array of layer ids")?
                .iter()
                .filter_map(values::number)
                .map(|n| Id(n as u64))
                .collect();
            let n = host.project().compositions.len() + 1;
            let name = args["name"].as_str().map_or_else(|| format!("Precomp {n}"), str::to_string);
            let (cmd, comp, layer) =
                xenify_ops::precomp::precompose(host.project(), host.comp(), &ids, &name).map_err(|e| e.to_string())?;
            host.execute("AI: precompose", cmd)?;
            Ok(done(json!({"layer": layer.0, "comp": comp.0}), format!("Precomposed {} layers as “{name}”", ids.len())))
        }
        "add_gif" => {
            let query = args["query"].as_str().ok_or("missing `query`")?;
            let (g, data) = host.giphy_gif(query)?;
            let img = xenify_export::image::prepare(&data, 4096)?;
            let comp = host.comp();
            let (w, h) = {
                let c = host.project().comp(comp).map_err(|e| e.to_string())?;
                (c.settings.width as f64, c.settings.height as f64)
            };
            let at = match args.get("position").filter(|p| !p.is_null()) {
                Some(p) => values::vec2(p)?,
                None => Vec2::new(w / 2.0, h / 2.0),
            };
            let size = opt_f64(args, "size").unwrap_or(w.min(h) / 2.0).max(4.0);
            let mut project = host.project().clone();
            let name = g.credit();
            let (add, asset) = xenify_ops::assets::add_image(
                &mut project,
                &name,
                img.mime,
                img.width,
                img.height,
                img.data,
                img.frames,
            );
            let id = Id(project.next_id);
            let mut layer = Layer::new(id, &name, LayerKind::Image { asset, playback: Default::default() });
            layer.transform.position = at.into();
            layer.transform.anchor = Vec2::new(img.width as f64 / 2.0, img.height as f64 / 2.0).into();
            let k = size / img.width.max(img.height).max(1) as f64;
            layer.transform.scale = Vec2::new(k, k).into();
            let cmds: Vec<Command> = add.into_iter().chain([Command::AddLayer { comp, index: None, layer }]).collect();
            host.execute("AI: add GIF", Command::batch(cmds))?;
            Ok(done(
                json!({"ok": true, "layer": id.0, "credit": name, "attribution": xenify_export::giphy::ATTRIBUTION}),
                format!("GIF: {name}"),
            ))
        }
        "set_image_playback" => {
            let layer = Id(values::number(&args["layer"]).ok_or("missing `layer`")? as u64);
            let LayerKind::Image { playback, .. } = host.project().layer(layer).map_err(|e| e.to_string())?.kind else {
                return Err(format!("layer {} isn't an image", layer.0));
            };
            let mut p = playback;
            if let Some(s) = opt_f64(args, "speed") {
                p.speed = s.clamp(0.0, 16.0);
            }
            if let Some(o) = opt_f64(args, "offset") {
                p.offset = o.max(0.0);
            }
            if let Some(l) = args.get("looped").and_then(Json::as_bool) {
                p.looped = l;
            }
            host.execute("AI: image playback", Command::SetImagePlayback { layer, playback: p })?;
            Ok(done(json!({"ok": true}), "Set the animated image's playback".to_string()))
        }
        "set_symbol_timing" => {
            let layer = Id(values::number(&args["layer"]).ok_or("missing `layer`")? as u64);
            let l = host.project().layer(layer).map_err(|e| e.to_string())?;
            if !matches!(l.kind, LayerKind::Precomp { .. }) {
                return Err(format!("layer {} isn't a symbol", layer.0));
            }
            let mut cmds = Vec::new();
            if let Some(s) = opt_f64(args, "starts_at") {
                cmds.push(Command::SetPrecompStart { layer, start: s });
            }
            if let Some(on) = args.get("looped").and_then(Json::as_bool) {
                cmds.push(Command::SetPrecompLoop { layer, looped: on });
            }
            if cmds.is_empty() {
                return Err("give `starts_at` and/or `looped`".into());
            }
            host.execute("AI: symbol timing", Command::batch(cmds))?;
            Ok(done(json!({"ok": true}), "Set the symbol's timing".to_string()))
        }
        "apply_theme" => {
            let name = args["theme"].as_str().ok_or("missing `theme`")?;
            let theme = xenify_doc::templates::theme(name).ok_or_else(|| format!("no theme called {name:?}"))?;
            let cmd = xenify_ops::templates::apply_theme(host.project(), theme);
            host.execute(&format!("AI: theme {}", theme.name), cmd)?;
            Ok(done(json!({"ok": true}), format!("Applied the {} theme", theme.name)))
        }
        "add_slide" => {
            let project = host.project();
            let count = xenify_ops::slides::slides_or_main(project).len();
            let after = values::number(&args["after"]).map(|n| (n.max(1.0) as usize - 1).min(count - 1));
            let layout = match args["layout"].as_str() {
                Some(l) => Some(xenify_doc::Layout::from_name(l).ok_or_else(|| format!("no layout called {l:?}"))?),
                None => None,
            };
            let theme = match args["theme"].as_str() {
                Some(t) => Some(xenify_doc::templates::theme(t).ok_or_else(|| format!("no theme called {t:?}"))?),
                None => None,
            };
            let (cmd, comp) = match layout {
                Some(layout) => {
                    let content = xenify_doc::Content {
                        title: args["title"].as_str().unwrap_or("").to_string(),
                        subtitle: args["subtitle"].as_str().unwrap_or("").to_string(),
                        items: args["items"]
                            .as_array()
                            .map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect())
                            .unwrap_or_default(),
                    };
                    xenify_ops::templates::add_layout_slide(project, after, layout, theme, &content)
                }
                None => xenify_ops::slides::add_slide(project, after),
            }
            .map_err(|e| e.to_string())?;
            host.execute("AI: add slide", cmd)?;
            let mut s = host.project().comp(comp).map_err(|e| e.to_string())?.settings.clone();
            if let Some(name) = args["name"].as_str() {
                s.name = name.to_string();
            }
            if let Some(d) = opt_f64(args, "duration") {
                s.duration = d.max(0.1);
            }
            if let Some(b) = args.get("background").filter(|b| !b.is_null()) {
                s.background = values::color(b)?;
            }
            host.execute("AI: slide settings", Command::SetCompSettings { comp, settings: s })?;
            let index = host.project().slide_index(comp).ok_or("the slide wasn't added")?;
            let (notes, t) = (args["notes"].as_str().map(str::to_string), args.get("transition").cloned());
            let current = host.project().slides[index].transition_in;
            let t = t.filter(|t| !t.is_null()).map(|t| transition(&t, current)).transpose()?;
            let cmd = xenify_ops::slides::edit_slide(host.project(), index, |sl| {
                if let Some(n) = notes {
                    sl.notes = n;
                }
                if let Some(t) = t {
                    sl.transition_in = t;
                }
            })
            .map_err(|e| e.to_string())?;
            host.execute("AI: slide details", cmd)?;
            show_slide(host, index, format!("Added slide {}", index + 1))
        }
        "open_slide" => {
            let n = values::number(&args["slide"]).ok_or("missing `slide` number")?;
            let index = (n.max(1.0) as usize) - 1;
            show_slide(host, index, format!("Opened slide {}", index + 1))
        }
        "edit_slide" => {
            let index = match values::number(&args["slide"]) {
                Some(n) => (n.max(1.0) as usize) - 1,
                None => open_slide_index(host).unwrap_or(0),
            };
            let slides = xenify_ops::slides::slides_or_main(host.project());
            let slide = slides.get(index).ok_or_else(|| format!("there are {} slides", slides.len()))?.clone();
            let t_in = args
                .get("transition_in")
                .filter(|t| !t.is_null())
                .map(|t| transition(t, slide.transition_in))
                .transpose()?;
            let t_out = args
                .get("transition_out")
                .filter(|t| !t.is_null())
                .map(|t| transition(t, slide.transition_out))
                .transpose()?;
            let notes = args["notes"].as_str().map(str::to_string);
            let cmd = xenify_ops::slides::edit_slide(host.project(), index, |sl| {
                if let Some(n) = notes {
                    sl.notes = n;
                }
                if let Some(t) = t_in {
                    sl.transition_in = t;
                }
                if let Some(t) = t_out {
                    sl.transition_out = t;
                }
            })
            .map_err(|e| e.to_string())?;
            host.execute("AI: edit slide", cmd)?;
            if let Some(name) = args["name"].as_str() {
                let mut s = host.project().comp(slide.comp).map_err(|e| e.to_string())?.settings.clone();
                s.name = name.to_string();
                host.execute("AI: rename slide", Command::SetCompSettings { comp: slide.comp, settings: s })?;
            }
            Ok(done(json!({"ok": true}), format!("Edited slide {}", index + 1)))
        }
        "add_mask" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
            let to_local = world_transform(comp, id, host.time()).inverse();
            let shape = args["shape"].as_str().unwrap_or("rect");
            let rect = || -> Result<kurbo::Rect, String> {
                let v: Vec<f64> = args["rect"]
                    .as_array()
                    .ok_or("`rect` must be [x0, y0, x1, y1]")?
                    .iter()
                    .filter_map(values::number)
                    .collect();
                let [x0, y0, x1, y1] = v[..] else { return Err("`rect` must be [x0, y0, x1, y1]".into()) };
                Ok(kurbo::Rect::new(x0, y0, x1, y1).abs())
            };
            let path = match shape {
                "path" => values::path(&args["path"])?,
                "ellipse" => kurbo::Ellipse::from_rect(rect()?).to_path(0.1),
                _ => xenify_doc::rect_path(rect()?),
            };
            let n = layer.meta.masks.len();
            let mut mask = xenify_doc::Mask::new(
                args["name"].as_str().map_or(format!("Mask {}", n + 1), str::to_string),
                to_local * path,
            );
            if let Some(m) = args["mode"].as_str() {
                mask.mode = serde_json::from_value(json!(m))
                    .map_err(|_| "mode must be add, subtract, intersect or difference")?;
            }
            mask.inverted = args["inverted"].as_bool().unwrap_or(false);
            if let Some(f) = opt_f64(args, "feather") {
                mask.feather = f.max(0.0).into();
            }
            if let Some(o) = opt_f64(args, "opacity") {
                mask.opacity = o.clamp(0.0, 1.0).into();
            }
            let mut meta = layer.meta.clone();
            meta.masks.push(mask);
            host.execute("AI: add mask", Command::SetLayerMeta { layer: id, meta })?;
            let result = with_warnings(host, id, json!({"ok": true, "mask_index": n}));
            Ok(done(result, format!("Masked “{}”", layer.meta.name)))
        }
        "animate_text" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            if !matches!(layer.kind, LayerKind::Text(_)) {
                return Err(format!("layer {id} is not a text layer"));
            }
            let preset = args.get("preset").and_then(Json::as_str).unwrap_or("Fade Up by Word");
            let mut a = xenify_doc::TextAnimator::preset(preset).ok_or_else(|| format!("unknown preset {preset:?}"))?;
            a.start = opt_f64(args, "start").unwrap_or(host.time() - layer.meta.in_point).max(0.0);
            if let Some(u) = args.get("unit").and_then(Json::as_str) {
                a.unit = serde_json::from_value(json!(u)).map_err(|_| format!("unknown unit {u:?}"))?;
            }
            if let Some(o) = args.get("order").and_then(Json::as_str) {
                a.order = serde_json::from_value(json!(o)).map_err(|_| format!("unknown order {o:?}"))?;
            }
            if let Some(v) = opt_f64(args, "stagger") {
                a.stagger = v.max(0.0);
            }
            if let Some(v) = opt_f64(args, "duration") {
                a.duration = v.max(0.0);
            }
            if let Some(e) = args.get("easing").and_then(Json::as_str) {
                a.easing = xenify_doc::Easing::preset(e).ok_or_else(|| format!("unknown easing {e:?}"))?;
            }
            if let Some(f) = args.get("from").filter(|f| f.is_object()) {
                if let Some(o) = f.get("offset").filter(|o| !o.is_null()) {
                    a.from.offset = values::vec2(o)?;
                }
                let n = |k: &str| f.get(k).and_then(values::number);
                if let Some(v) = n("scale") {
                    a.from.scale = v.max(0.0);
                }
                if let Some(v) = n("rotation") {
                    a.from.rotation = v;
                }
                if let Some(v) = n("opacity") {
                    a.from.opacity = v.clamp(0.0, 1.0);
                }
                if let Some(v) = n("blur") {
                    a.from.blur = v.max(0.0);
                }
                if let Some(v) = n("tracking") {
                    a.from.tracking = v;
                }
            }
            a.out = args.get("out").and_then(Json::as_bool).unwrap_or(false);
            let cmd = xenify_ops::text_anim::add(host.project(), id, a).map_err(|e| e.to_string())?;
            host.execute("AI: animate text", cmd)?;
            Ok(done(json!({"ok": true}), format!("Animated “{}” ({preset})", layer.meta.name)))
        }
        "add_effect" => {
            let comp = host.comp();
            let name = args["effect"].as_str().ok_or("missing `effect`")?;
            if !xenify_ops::effects::names().any(|n| n == name) {
                return Err(format!("unknown effect {name:?}"));
            }
            let time = opt_f64(args, "start").unwrap_or(host.time()).max(0.0);
            let target = args.get("target").and_then(values::number).map(|n| Id(n as u64));
            let (target_box, target_local_box) = match target {
                Some(t) => {
                    let c = host.project().comp(comp).map_err(|e| e.to_string())?;
                    let scene = xenify_anim::evaluate_project(host.project(), comp, time);
                    let b = xenify_anim::world_box(c, &scene, t, &mut xenify_anim::ApproxText);
                    let to_layer = world_transform(c, t, time);
                    let local = b
                        .filter(|_| to_layer.determinant().abs() > 1e-12)
                        .map(|b| to_layer.inverse().transform_rect_bbox(b));
                    (b, local)
                }
                None => (None, None),
            };
            let at = match args.get("position").filter(|p| !p.is_null()) {
                Some(p) => Some(values::vec2(p)?),
                None => None,
            };
            let placement = xenify_ops::effects::Placement { at, time, target, target_box, target_local_box };
            let first = Id(host.project().next_id);
            let (cmd, select) = xenify_ops::effects::add_effect(host.project(), comp, first, name, placement)
                .map_err(|e| e.to_string())?;
            host.execute(&format!("AI: {name}"), cmd)?;
            Ok(done(json!({"ok": true, "layer": select.0}), format!("{name} at {time:.1}s")))
        }
        "add_particles" => {
            let comp = host.comp();
            let preset = args["preset"].as_str().ok_or("missing `preset`")?;
            let id = Id(host.project().next_id);
            let target = args.get("target").and_then(values::number).map(|n| Id(n as u64));
            let cmd = xenify_ops::particles::add_preset(host.project(), comp, id, preset, target)
                .map_err(|e| e.to_string())?;
            let mut commands = vec![cmd];
            let time = host.time();
            let mut e = match &commands[0] {
                Command::AddLayer { layer, .. } => match &layer.kind {
                    LayerKind::Particles(e) => (**e).clone(),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            e.start = opt_f64(args, "start").unwrap_or(time).max(0.0);
            let set = |p: &mut xenify_doc::Property<f64>, key: &str| {
                if let Some(v) = opt_f64(args, key) {
                    *p = v.into();
                }
            };
            set(&mut e.count, "count");
            set(&mut e.rate, "rate");
            set(&mut e.direction, "direction");
            set(&mut e.spread, "spread");
            set(&mut e.speed, "speed");
            set(&mut e.lifetime, "lifetime");
            set(&mut e.size, "size");
            if let Some(g) = opt_f64(args, "gravity") {
                e.gravity = Vec2::new(0.0, g).into();
            }
            if let Some(cs) = args.get("colors").and_then(Json::as_array) {
                let cs: Result<Vec<_>, _> = cs.iter().map(values::color).collect();
                let cs = cs?;
                if !cs.is_empty() {
                    e.colors = cs;
                    e.color_over_life = None;
                }
            }
            if let Some(p) = args.get("particle").and_then(Json::as_str) {
                e.particle =
                    serde_json::from_value(json!({"type": p})).map_err(|_| format!("unknown particle {p:?}"))?;
            }
            if let Some(n) = args.get("sprite").and_then(values::number) {
                let sprite = Id(n as u64);
                host.project().layer(sprite).map_err(|e| e.to_string())?;
                e.particle = xenify_doc::ParticleShape::Layer { layer: sprite };
            }
            match args.get("end").and_then(Json::as_str) {
                Some("fade") => e.end = xenify_doc::EndLook::Fade,
                Some("shrink") => e.end = xenify_doc::EndLook::Shrink,
                Some("settle") => {
                    let h = host.project().comp(comp).map_err(|e| e.to_string())?.settings.height as f64;
                    e.end = xenify_doc::EndLook::Settle { floor: opt_f64(args, "floor").unwrap_or(h - 20.0) };
                }
                Some("form") => {
                    let t = target.ok_or("end = form needs `target`")?;
                    e.end = xenify_doc::EndLook::Form { target: t };
                }
                _ => {}
            }
            if let Command::AddLayer { layer, .. } = &mut commands[0] {
                layer.kind = LayerKind::Particles(Box::new(e));
                if let Some(name) = args.get("name").and_then(Json::as_str) {
                    layer.meta.name = name.to_string();
                }
                if let Some(p) = args.get("position").filter(|p| !p.is_null()) {
                    layer.transform.position = values::vec2(p)?.into();
                }
            }
            host.execute("AI: add particles", Command::batch(std::mem::take(&mut commands)))?;
            Ok(done(json!({"ok": true, "layer": id.0}), format!("Particles: {preset}")))
        }
        "add_explosion" => {
            use xenify_ops::blast::BlastKind;
            let comp = host.comp();
            let kind = match args["kind"].as_str() {
                Some("big_blast") => BlastKind::BigBlast,
                Some("explosion") => BlastKind::Explosion,
                other => return Err(format!("unknown kind {other:?}")),
            };
            let (w, h) = {
                let c = host.project().comp(comp).map_err(|e| e.to_string())?;
                (c.settings.width as f64, c.settings.height as f64)
            };
            let at = match args.get("position").filter(|p| !p.is_null()) {
                Some(p) => values::vec2(p)?,
                None if kind == BlastKind::BigBlast => Vec2::new(w / 2.0, h * 0.85),
                None => Vec2::new(w / 2.0, h / 2.0),
            };
            let start = opt_f64(args, "start").unwrap_or(host.time()).max(0.0);
            let scale = opt_f64(args, "size").unwrap_or((w.min(h) / 1080.0).max(0.1));
            let first = Id(host.project().next_id);
            let (cmd, group) = xenify_ops::blast::add_blast(host.project(), comp, first, kind, at, start, scale)
                .map_err(|e| e.to_string())?;
            host.execute("AI: add explosion", cmd)?;
            Ok(done(json!({"ok": true, "group": group.0}), format!("{} at {:.1}s", kind.label(), start)))
        }
        "add_water" => {
            let comp = host.comp();
            let id = Id(host.project().next_id);
            let start = opt_f64(args, "start").unwrap_or(host.time()).max(0.0);
            let cmd = match args.get("target").and_then(values::number) {
                Some(n) => {
                    let target = Id(n as u64);
                    let scene = xenify_anim::evaluate_project(host.project(), comp, start);
                    let c = host.project().comp(comp).map_err(|e| e.to_string())?;
                    let bounds = xenify_anim::world_box(c, &scene, target, &mut xenify_anim::ApproxText)
                        .ok_or("that layer isn't visible then, so there's nothing to fill")?;
                    xenify_ops::liquid::fill_layer(host.project(), comp, id, target, bounds, start)
                }
                None => xenify_ops::liquid::fill_screen(host.project(), comp, id, start),
            }
            .map_err(|e| e.to_string())?;
            let mut commands = match cmd {
                Command::Batch { commands } => commands,
                one => vec![one],
            };
            for c in &mut commands {
                let Command::AddLayer { layer, .. } = c else { continue };
                let LayerKind::Liquid(w) = &mut layer.kind else { continue };
                let seconds = opt_f64(args, "seconds").unwrap_or(3.0).max(0.05);
                let to = opt_f64(args, "level").unwrap_or(0.92).clamp(0.0, 1.0);
                let key = |time, value| xenify_doc::Keyframe { time, value, easing: xenify_doc::Easing::EASE_IN_OUT };
                w.level = xenify_doc::Property::from_keyframes(vec![key(start, 0.0), key(start + seconds, to)])
                    .unwrap_or_else(|| to.into());
                if let Some(c) = args.get("color").filter(|c| !c.is_null()) {
                    w.color = values::color(c)?.into();
                }
            }
            host.execute("AI: add water", Command::batch(commands))?;
            Ok(done(json!({"ok": true, "layer": id.0}), "Water".to_string()))
        }
        "add_emoji" => {
            if args.get("animated").and_then(Json::as_bool) == Some(true) {
                return add_animated_emoji(args, host);
            }
            let key = args["emoji"].as_str().ok_or("missing `emoji`")?;
            let e = xenify_export::emoji::find(key)
                .or_else(|| xenify_export::emoji::search(key).into_iter().next())
                .ok_or_else(|| format!("no emoji matches {key:?}"))?;
            let comp = host.comp();
            let (w, h) = {
                let c = host.project().comp(comp).map_err(|e| e.to_string())?;
                (c.settings.width as f64, c.settings.height as f64)
            };
            let confetti = args.get("confetti").and_then(Json::as_bool).unwrap_or(false);
            let at = match args.get("position").filter(|p| !p.is_null()) {
                Some(p) => values::vec2(p)?,
                None if confetti => Vec2::new(-1000.0, -1000.0),
                None => Vec2::new(w / 2.0, h / 2.0),
            };
            let size = opt_f64(args, "size").unwrap_or((w.min(h) / 4.0).round()).max(4.0);
            let layers = xenify_export::emoji::layers(e, host.project().next_id, at, size)?;
            let ids: Vec<Id> = layers.iter().map(|l| l.id).collect();
            let adds = layers.into_iter().map(|layer| Command::AddLayer { comp, index: None, layer }).collect();
            host.execute(&format!("AI: {}", e.name), Command::batch(adds))?;
            let (group_cmd, group) = {
                let p = host.project();
                xenify_ops::group::group(p, comp, &ids, &e.name, host.time(), &mut xenify_anim::ApproxText)
                    .map_err(|e| e.to_string())?
            };
            host.execute("AI: group emoji", group_cmd)?;
            if !confetti {
                return Ok(done(json!({"ok": true, "layer": group.0}), format!("{} {}", e.glyph, e.name)));
            }
            let mut meta = host.project().layer(group).map_err(|e| e.to_string())?.meta.clone();
            meta.visible = false;
            host.execute("AI: hide sprite", Command::SetLayerMeta { layer: group, meta })?;
            let id = Id(host.project().next_id);
            let mut cmd = xenify_ops::particles::add_preset(host.project(), comp, id, "Confetti Burst", None)
                .map_err(|e| e.to_string())?;
            if let Command::AddLayer { layer, .. } = &mut cmd
                && let LayerKind::Particles(em) = &mut layer.kind
            {
                em.particle = xenify_doc::ParticleShape::Layer { layer: group };
                em.size = (size / 5.0).clamp(16.0, 80.0).into();
                em.start = opt_f64(args, "start").unwrap_or(host.time()).max(0.0);
                layer.meta.name = format!("{} confetti", e.name);
            }
            host.execute("AI: emoji confetti", cmd)?;
            Ok(done(json!({"ok": true, "layer": id.0, "sprite": group.0}), format!("{} confetti", e.glyph)))
        }
        "add_sound" => {
            use xenify_audio::synth::{MusicRecipe, Recipe, SfxRecipe};
            let effect = args["effect"].as_str().ok_or("missing `effect`")?;
            let start = opt_f64(args, "start").unwrap_or(host.time()).max(0.0);
            let recipe = if effect == "music" {
                let mut m = MusicRecipe::default();
                if let Some(mood) = args.get("mood").filter(|v| !v.is_null()) {
                    m.mood = serde_json::from_value(mood.clone()).map_err(|_| format!("unknown mood {mood}"))?;
                }
                m.tempo = opt_f64(args, "tempo").unwrap_or(m.tempo).clamp(40.0, 220.0);
                m.bars = opt_f64(args, "bars").map_or(m.bars, |b| b.round().clamp(1.0, 64.0) as u32);
                m.volume = opt_f64(args, "volume").unwrap_or(m.volume).clamp(0.0, 1.0);
                Recipe::Music(m)
            } else {
                let kind = serde_json::from_value(json!(effect)).map_err(|_| format!("unknown sound {effect:?}"))?;
                let mut s = SfxRecipe::of(kind);
                s.pitch = opt_f64(args, "pitch").unwrap_or(0.0).clamp(-24.0, 24.0);
                s.length = opt_f64(args, "length").unwrap_or(s.length).clamp(0.02, 20.0);
                s.volume = opt_f64(args, "volume").unwrap_or(s.volume).clamp(0.0, 1.0);
                Recipe::Sfx(s)
            };
            let media = xenify_audio::synth::media_asset(&recipe, Id(host.project().next_id));
            let name = recipe.name();
            if effect == "music" && args.get("soundtrack").and_then(Json::as_bool).unwrap_or(false) {
                let mut cmd = xenify_ops::media::add_to_soundtrack(host.project(), media, start);
                if let Command::Batch { commands } = &mut cmd
                    && let Some(Command::SetSoundtrack { soundtrack }) = commands.last_mut()
                    && let Some(c) = soundtrack.clips.last_mut()
                {
                    c.audio.looped = true;
                }
                host.execute("AI: music soundtrack", cmd)?;
                return Ok(done(json!({"ok": true}), format!("Soundtrack: {name}")));
            }
            let layer = Id(media.id.0 + 1);
            let cmd = xenify_ops::media::add_clip(host.project(), host.comp(), media, layer, start)
                .map_err(|e| e.to_string())?;
            host.execute(&format!("AI: {name}"), cmd)?;
            Ok(done(json!({"ok": true, "layer": layer.0}), format!("{name} at {start:.1}s")))
        }
        "edit_media" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let b = |k: &str| args.get(k).and_then(Json::as_bool);
            let mut commands = Vec::new();
            match &layer.kind {
                LayerKind::Video(_) => {
                    let cmd = xenify_ops::media::edit_video(host.project(), id, |v| {
                        v.source_in = opt_f64(args, "clip_start").unwrap_or(v.source_in).max(0.0);
                        v.speed = opt_f64(args, "speed").unwrap_or(v.speed).clamp(0.05, 10.0);
                        v.looped = b("loop").unwrap_or(v.looped);
                        v.muted = b("mute").unwrap_or(v.muted);
                    })
                    .map_err(|e| e.to_string())?;
                    commands.push(cmd);
                }
                LayerKind::Audio(_) => {
                    let cmd = xenify_ops::media::edit_audio(host.project(), id, |a| {
                        a.source_in = opt_f64(args, "clip_start").unwrap_or(a.source_in).max(0.0);
                        a.speed = opt_f64(args, "speed").unwrap_or(a.speed).clamp(0.05, 10.0);
                        a.looped = b("loop").unwrap_or(a.looped);
                        a.muted = b("mute").unwrap_or(a.muted);
                        a.fade_in = opt_f64(args, "fade_in").unwrap_or(a.fade_in).max(0.0);
                        a.fade_out = opt_f64(args, "fade_out").unwrap_or(a.fade_out).max(0.0);
                    })
                    .map_err(|e| e.to_string())?;
                    commands.push(cmd);
                }
                _ => return Err(format!("layer {id} is not a video or audio layer")),
            }
            host.execute("AI: edit media", Command::batch(commands))?;
            match args.get("frame").and_then(Json::as_str) {
                Some(f @ ("fit" | "fill")) => {
                    let cmd = xenify_ops::media::fit(host.project(), id, host.time(), f == "fill", false)
                        .map_err(|e| e.to_string())?;
                    host.execute("AI: fit video", cmd)?;
                }
                Some("trim") => {
                    let cmd = xenify_ops::media::fit_length(host.project(), id).map_err(|e| e.to_string())?;
                    host.execute("AI: trim to clip", cmd)?;
                }
                _ => {}
            }
            Ok(done(json!({"ok": true}), format!("Playback of “{}”", layer.meta.name)))
        }
        "set_camera" => {
            let comp = host.comp();
            let time = host.time();
            let id = match args.get("layer").and_then(values::number) {
                Some(n) => Id(n as u64),
                None => {
                    let id = Id(host.project().next_id);
                    let cmd =
                        xenify_ops::camera::add_camera(host.project(), comp, id, None).map_err(|e| e.to_string())?;
                    host.execute("AI: add camera", cmd)?;
                    id
                }
            };
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let LayerKind::Camera(cam) = &layer.kind else { return Err(format!("layer {id} is not a camera")) };
            let mut meta = layer.meta.clone();
            if let Some(name) = args.get("name").and_then(Json::as_str) {
                meta.name = name.to_string();
            }
            match args.get("follow").and_then(values::number) {
                Some(0.0) => meta.parent = None,
                Some(n) => {
                    let target = Id(n as u64);
                    host.project().layer(target).map_err(|e| e.to_string())?;
                    meta.parent = Some(target);
                }
                None => {}
            }
            let mut commands = Vec::new();
            if meta != layer.meta {
                commands.push(Command::SetLayerMeta { layer: id, meta: meta.clone() });
            }
            // Positions are where the picture's centre is, in composition space.
            if let Some(p) = args.get("position").filter(|p| !p.is_null()) {
                let p = values::vec2(p)?;
                let project = host.project();
                let c = project.comp(comp).map_err(|e| e.to_string())?;
                let parent = meta.parent.map_or(kurbo::Affine::IDENTITY, |pa| world_transform(c, pa, time));
                let local = parent.inverse() * p.to_point();
                commands.push(Command::SetValue {
                    layer: id,
                    path: PropPath::Position,
                    time,
                    value: Value::Vec2(local.to_vec2()),
                });
            } else if meta.parent != layer.meta.parent
                && let Some(parent) = meta.parent
            {
                // Keep it where it is when it starts following something.
                let c = host.project().comp(comp).map_err(|e| e.to_string())?;
                let at = world_transform(c, id, time) * kurbo::Point::ZERO;
                let local = world_transform(c, parent, time).inverse() * at;
                commands.push(Command::SetValue {
                    layer: id,
                    path: PropPath::Position,
                    time,
                    value: Value::Vec2(local.to_vec2()),
                });
            }
            if let Some(r) = opt_f64(args, "rotation") {
                commands.push(Command::SetValue { layer: id, path: PropPath::Rotation, time, value: Value::Scalar(r) });
            }
            let mut cam = cam.clone();
            if let Some(z) = opt_f64(args, "zoom") {
                cam.zoom = z.max(0.01).into();
            }
            match args.get("shake").and_then(Json::as_str) {
                Some("none") => cam.shake = None,
                Some("impact") => cam.shake = Some(xenify_doc::CameraShake::impact(time)),
                Some(_) => cam.shake = Some(xenify_doc::CameraShake { start: time, ..Default::default() }),
                None => {}
            }
            if let Some(sh) = &mut cam.shake {
                if let Some(a) = opt_f64(args, "shake_amount") {
                    sh.amount = a.max(0.0).into();
                }
                if let Some(t) = opt_f64(args, "shake_start") {
                    sh.start = t.max(0.0);
                }
                if let Some(d) = opt_f64(args, "shake_decay") {
                    sh.decay = d.max(0.0);
                }
            }
            if !matches!(&layer.kind, LayerKind::Camera(c) if *c == cam) {
                commands.push(Command::SetCamera { layer: id, camera: Box::new(cam) });
            }
            if !commands.is_empty() {
                host.execute("AI: camera", Command::batch(commands))?;
            }
            Ok(done(json!({"ok": true, "layer": id.0}), format!("Camera “{}”", meta.name)))
        }
        "add_array" => {
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
            let world = world_transform(comp, id, host.time());
            if world.determinant().abs() < 1e-12 {
                return Err("the layer is scaled to nothing, so it can't be arrayed".into());
            }
            let to_local = world.inverse();
            let count =
                opt_f64(args, "count").ok_or("missing `count`")?.round().clamp(1.0, xenify_doc::MAX_COPIES as f64);
            let offset = match args.get("offset") {
                Some(v) if !v.is_null() => {
                    let v = values::vec2(v)?;
                    (to_local * v.to_point()).to_vec2() - (to_local * kurbo::Point::ZERO).to_vec2()
                }
                _ => Vec2::ZERO,
            };
            let origin = match args.get("origin") {
                Some(v) if !v.is_null() => (to_local * values::vec2(v)?.to_point()).to_vec2(),
                _ => layer.transform.anchor.value_at(host.time()),
            };
            let scale = match args.get("scale") {
                Some(v) if v.is_array() || v.is_object() => values::vec2(v)?,
                Some(v) if !v.is_null() => {
                    let s = values::number(v).ok_or("`scale` must be a number or [sx, sy]")?;
                    Vec2::new(s, s)
                }
                _ => Vec2::new(1.0, 1.0),
            };
            let kind = xenify_doc::ModifierKind::Array {
                count: count.into(),
                offset: offset.into(),
                rotation: opt_f64(args, "rotation").unwrap_or(0.0).into(),
                scale: scale.into(),
                origin: origin.into(),
                end_opacity: opt_f64(args, "end_opacity").unwrap_or(1.0).clamp(0.0, 1.0).into(),
            };
            let n = layer.meta.modifiers.len();
            let mut meta = layer.meta.clone();
            meta.modifiers.push(xenify_doc::Modifier::new(kind));
            host.execute("AI: add array", Command::SetLayerMeta { layer: id, meta })?;
            let result = with_warnings(host, id, json!({"ok": true, "modifier_index": n}));
            Ok(done(result, format!("Arrayed “{}” ×{count}", layer.meta.name)))
        }
        "add_modifier" => {
            use xenify_doc::ModifierKind as K;
            let id = layer_id(args)?;
            let layer = host.project().layer(id).map_err(|e| e.to_string())?.clone();
            let num = |k: &str, d: f64| opt_f64(args, k).unwrap_or(d);
            let kind = match args["type"].as_str().ok_or("missing `type`")? {
                "jitter" => K::Jitter {
                    amount: num("amount", 4.0).into(),
                    detail: num("detail", 5.0).into(),
                    seed: num("seed", 0.0).into(),
                    smooth: args["smooth"].as_bool().unwrap_or(false),
                },
                t @ ("zig_zag" | "wave") => K::ZigZag {
                    amount: num("amount", 6.0).into(),
                    ridges: num("ridges", 4.0).into(),
                    smooth: t == "wave",
                },
                "pucker_bloat" => K::PuckerBloat { strength: num("strength", -0.3).clamp(-1.0, 1.0).into() },
                t @ ("twist" | "mirror") => {
                    let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
                    let world = world_transform(comp, id, host.time());
                    let centre = match args.get("centre") {
                        Some(v) if !v.is_null() && world.determinant().abs() > 1e-12 => {
                            (world.inverse() * values::vec2(v)?.to_point()).to_vec2()
                        }
                        _ => layer.transform.anchor.value_at(host.time()),
                    };
                    if t == "twist" {
                        K::Twist { rotation: num("angle", 60.0).into(), origin: centre.into() }
                    } else {
                        K::Mirror {
                            origin: centre.into(),
                            rotation: num("angle", 90.0).into(),
                            merge: args["merge"].as_bool().unwrap_or(false),
                        }
                    }
                }
                t @ ("path_array" | "blend") => {
                    let target = Id(values::number(&args["target"]).ok_or("missing `target` layer id")? as u64);
                    let other = host.project().layer(target).map_err(|e| e.to_string())?;
                    if !matches!(other.kind, xenify_doc::LayerKind::Shape { .. }) || target == id {
                        return Err("`target` must be another shape layer".into());
                    }
                    if t == "blend" {
                        K::Blend { target, count: num("count", 4.0).into() }
                    } else {
                        K::PathArray {
                            path: target,
                            count: num("count", 8.0).into(),
                            start: num("start", 0.0).into(),
                            end: num("end", 1.0).into(),
                            align: args["align"].as_bool().unwrap_or(true),
                        }
                    }
                }
                "shatter" => {
                    let t0 = opt_f64(args, "start").unwrap_or(host.time());
                    let flight = num("seconds", 2.0).max(0.05);
                    let key = |time: f64, value: f64| xenify_doc::Keyframe {
                        time,
                        value,
                        easing: xenify_doc::Easing::Linear,
                    };
                    let keys = if args["direction"].as_str() == Some("in") {
                        vec![key((t0 - flight).max(0.0), flight), key(t0.max(flight), 0.0)]
                    } else {
                        vec![key(t0, 0.0), key(t0 + flight, flight)]
                    };
                    let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
                    let world = world_transform(comp, id, host.time());
                    let centre = match args.get("centre") {
                        Some(v) if !v.is_null() && world.determinant().abs() > 1e-12 => {
                            (world.inverse() * values::vec2(v)?.to_point()).to_vec2()
                        }
                        _ => layer.transform.anchor.value_at(host.time()),
                    };
                    K::Shatter {
                        count: num("count", 24.0).into(),
                        origin: centre.into(),
                        force: num("amount", 400.0).into(),
                        amount: 0.0.into(),
                        rotation: (-90.0).into(),
                        gravity: 600.0.into(),
                        spin: 180.0.into(),
                        progress: xenify_doc::Property::from_keyframes(keys).unwrap_or_else(|| 0.0.into()),
                        seed: num("seed", 1.0).into(),
                    }
                }
                "smooth" => K::Smooth { strength: num("strength", 1.0).clamp(0.0, 1.0).into() },
                "subdivide" => K::Subdivide { levels: num("levels", 1.0).into() },
                "simplify" => K::Simplify { tolerance: num("tolerance", 1.0).into() },
                "offset_path" => K::OffsetPath {
                    amount: num("amount", 10.0).into(),
                    join: serde_json::from_value(json!(args["corners"].as_str().unwrap_or("round")))
                        .map_err(|_| "corners must be miter, round or bevel")?,
                },
                other => return Err(format!("unknown modifier type {other:?}")),
            };
            let label = kind.label();
            let n = layer.meta.modifiers.len();
            let mut meta = layer.meta.clone();
            meta.modifiers.push(xenify_doc::Modifier::new(kind));
            host.execute("AI: add modifier", Command::SetLayerMeta { layer: id, meta })?;
            let result = with_warnings(host, id, json!({"ok": true, "modifier_index": n}));
            Ok(done(result, format!("{label} on “{}”", layer.meta.name)))
        }
        "open_composition" => {
            let id = Id(values::number(&args["comp"]).ok_or("missing `comp` id")? as u64);
            host.set_comp(id)?;
            let (project, time) = (host.project().clone(), host.time());
            let name = project.comp(id).map(|c| c.settings.name.clone()).unwrap_or_default();
            let result = scene_summary(&project, id, time, &mut HostMeasure(host));
            Ok(done(result, format!("Opened composition “{name}”")))
        }
        "combine_shapes" => {
            let ids: Vec<Id> = args["layers"]
                .as_array()
                .ok_or("`layers` must be an array of layer ids")?
                .iter()
                .filter_map(values::number)
                .map(|n| Id(n as u64))
                .collect();
            let op: xenify_ops::pathfinder::PathOp = serde_json::from_value(args["op"].clone())
                .map_err(|_| "op must be union, difference, intersection or exclusion")?;
            let (cmd, id) = xenify_ops::pathfinder::combine(host.project(), host.comp(), &ids, op, host.time())
                .map_err(|e| e.to_string())?;
            host.execute("AI: combine shapes", cmd)?;
            let result = with_warnings(host, id, json!({"id": id.0}));
            Ok(done(result, format!("Combined {} shapes ({})", ids.len(), args["op"].as_str().unwrap_or(""))))
        }
        "group_layers" => {
            let ids: Vec<Id> = args["layers"]
                .as_array()
                .ok_or("`layers` must be an array of layer ids")?
                .iter()
                .filter_map(values::number)
                .map(|n| Id(n as u64))
                .collect();
            let name = args["name"].as_str().unwrap_or("Group").to_string();
            let (project, comp, time) = (host.project().clone(), host.comp(), host.time());
            let (cmd, id) = xenify_ops::group::group(&project, comp, &ids, &name, time, &mut HostMeasure(host))
                .map_err(|e| e.to_string())?;
            host.execute("AI: group", cmd)?;
            Ok(done(json!({"id": id.0}), format!("Grouped {} layers as “{name}”", ids.len())))
        }
        "ungroup" => {
            let g = Id(values::number(&args["group"]).ok_or("missing `group` id")? as u64);
            let cmd = xenify_ops::group::ungroup(host.project(), g).map_err(|e| e.to_string())?;
            host.execute("AI: ungroup", cmd)?;
            Ok(done(json!({"ok": true}), "Ungrouped"))
        }
        "generate_illustration" => Ok(ToolOutcome::Illustrate {
            prompt: args["prompt"].as_str().ok_or("missing `prompt`")?.to_string(),
            args: args.clone(),
        }),
        "add_svg" => {
            let svg = args["svg"].as_str().ok_or("missing `svg`")?;
            let layers = host.import_svg(svg.as_bytes())?;
            let (result, n) = add_artwork(host, layers, args, "Artwork")?;
            let name = args["name"].as_str().unwrap_or("Artwork");
            Ok(done(result, format!("Drew “{name}” from SVG ({n} shapes)")))
        }
        "check_layout" => {
            let comp = host.project().comp(host.comp()).map_err(|e| e.to_string())?;
            let s = &comp.settings;
            let end = (s.duration - 1.0 / s.fps).max(0.0);
            let time = values::number(&args["time"]).unwrap_or(end).clamp(0.0, s.duration);
            let report = check_layout(host, time);
            Ok(done(report, format!("Measured the layout at {time:.2}s")))
        }
        "delete_layer" => {
            let id = layer_id(args)?;
            let name = host.project().layer(id).map_err(|e| e.to_string())?.meta.name.clone();
            host.execute("AI: delete layer", Command::RemoveLayer { layer: id })?;
            Ok(done(json!({"ok": true}), format!("Deleted “{name}”")))
        }
        "reorder_layer" => {
            let id = layer_id(args)?;
            let index = opt_f64(args, "index").ok_or("missing `index`")?.max(0.0) as usize;
            host.execute("AI: reorder", Command::MoveLayer { layer: id, index })?;
            Ok(done(json!({"ok": true}), "Reordered layers"))
        }
        "ask_user" => Ok(ToolOutcome::AskUser {
            question: args["question"].as_str().unwrap_or("Could you clarify?").to_string(),
            options: args["options"]
                .as_array()
                .map(|a| a.iter().filter_map(|o| o.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        }),
        "look_at_frame" => Ok(ToolOutcome::Review {
            time: values::number(&args["time"]).unwrap_or(0.0).max(0.0),
            question: args["question"].as_str().unwrap_or("Describe this frame.").to_string(),
        }),
        other => Err(format!("unknown tool {other:?}")),
    }
}

fn text_align(v: &Json) -> Result<Option<TextAlign>, String> {
    match v.as_str().map(str::to_lowercase).as_deref() {
        None => Ok(None),
        Some("left" | "start") => Ok(Some(TextAlign::Left)),
        Some("center" | "centre" | "middle") => Ok(Some(TextAlign::Center)),
        Some("right" | "end") => Ok(Some(TextAlign::Right)),
        Some(o) => Err(format!("align must be left, center or right (got {o:?})")),
    }
}

fn text_valign(v: &Json) -> Result<Option<TextVAlign>, String> {
    match v.as_str().map(str::to_lowercase).as_deref() {
        None => Ok(None),
        Some("top") => Ok(Some(TextVAlign::Top)),
        Some("middle" | "center" | "centre") => Ok(Some(TextVAlign::Middle)),
        Some("baseline") => Ok(Some(TextVAlign::Baseline)),
        Some("bottom") => Ok(Some(TextVAlign::Bottom)),
        Some(o) => Err(format!("valign must be top, middle, baseline or bottom (got {o:?})")),
    }
}

/// Keys a whole CSS gradient (direction and colours): each key lands on the
/// gradient's start, end and colour tracks.
fn animate_gradient(
    host: &mut dyn ToolHost,
    layer: &Layer,
    path: PropPath,
    keys: &[Json],
) -> Result<ToolOutcome, String> {
    let PropPath::Shape { index, field } = path else { return Err("not a shape gradient".into()) };
    let shape = layer.shapes().get(index).ok_or("no such shape")?;
    let (mut starts, mut ends, mut stops) = (Vec::new(), Vec::new(), Vec::new());
    for k in keys {
        let time = values::number(&k["time"]).ok_or("each keyframe needs a numeric `time`")?.max(0.0);
        let css = k["value"].as_str().ok_or("gradient must be a CSS linear-gradient()/radial-gradient() string")?;
        let (g_kind, angle, s) = values::css_gradient(css).ok_or("expected a CSS gradient string")??;
        // CSS gradients span the shape's own bounds.
        let b = xenify_anim::geometry_path(&shape.geometry, time).bounding_box();
        let g = xenify_doc::Gradient::across(b, g_kind, angle, s);
        let easing = if k.get("easing").is_some() { values::easing(&k["easing"])? } else { Easing::EASE_IN_OUT };
        starts.push((time, Value::Vec2(g.start), easing));
        ends.push((time, Value::Vec2(g.end), easing));
        stops.push((time, Value::Stops(g.stops), easing));
    }
    let [start, end, colours] = ShapeField::gradient_tracks(field.is_stroke_gradient());
    let set = |field, keys| -> Result<Command, String> {
        let kind = if field == colours { ValueKind::Stops } else { ValueKind::Vec2 };
        let property = animated_property(kind, keys)?;
        Ok(Command::SetProperty { layer: layer.id, path: PropPath::Shape { index, field }, property })
    };
    let cmds = vec![set(start, starts)?, set(end, ends)?, set(colours, stops)?];
    host.execute("AI: animate gradient", Command::batch(cmds))?;
    let n = keys.len();
    Ok(done(json!({"ok": true, "keyframes": n}), format!("Animated gradient of “{}” ({n} keyframes)", layer.meta.name)))
}

fn static_property(v: Value) -> AnyProperty {
    match v {
        Value::Scalar(x) => AnyProperty::Scalar(Property::Static(x)),
        Value::Vec2(x) => AnyProperty::Vec2(Property::Static(x)),
        Value::Color(x) => AnyProperty::Color(Property::Static(x)),
        Value::Path(x) => AnyProperty::Path(Property::Static(x)),
        Value::Stops(x) => AnyProperty::Stops(Property::Static(x)),
        Value::Quad(x) => AnyProperty::Quad(Property::Static(x)),
    }
}

fn animated_property(kind: ValueKind, keys: Vec<(f64, Value, Easing)>) -> Result<AnyProperty, String> {
    fn build<T: xenify_doc::Animatable>(
        keys: Vec<(f64, Value, Easing)>,
        get: impl Fn(Value) -> Option<T>,
    ) -> Result<Property<T>, String> {
        let ks = keys
            .into_iter()
            .map(|(time, v, easing)| get(v).map(|value| Keyframe { time, value, easing }).ok_or("mixed value types"))
            .collect::<Result<Vec<_>, _>>()?;
        Property::from_keyframes(ks).ok_or_else(|| "no keyframes".to_string())
    }
    Ok(match kind {
        ValueKind::Scalar => {
            AnyProperty::Scalar(build(keys, |v| if let Value::Scalar(x) = v { Some(x) } else { None })?)
        }
        ValueKind::Vec2 => AnyProperty::Vec2(build(keys, |v| if let Value::Vec2(x) = v { Some(x) } else { None })?),
        ValueKind::Color => AnyProperty::Color(build(keys, |v| if let Value::Color(x) = v { Some(x) } else { None })?),
        ValueKind::Path => AnyProperty::Path(build(keys, |v| if let Value::Path(x) = v { Some(x) } else { None })?),
        ValueKind::Stops => AnyProperty::Stops(build(keys, |v| if let Value::Stops(x) = v { Some(x) } else { None })?),
        ValueKind::Quad => AnyProperty::Quad(build(keys, |v| if let Value::Quad(x) = v { Some(x) } else { None })?),
    })
}

fn add_layer(args: &Json, host: &mut dyn ToolHost) -> Result<ToolOutcome, String> {
    let project = host.project();
    let comp = project.comp(host.comp()).map_err(|e| e.to_string())?;
    let (w, h) = (comp.settings.width as f64, comp.settings.height as f64);
    let id = Id(project.next_id);
    let kind_name = args["type"].as_str().ok_or("missing `type`")?;
    let name = args["name"].as_str().unwrap_or(kind_name).to_string();
    let mut position = match args.get("position") {
        Some(p) if !p.is_null() => values::vec2(p)?,
        _ => Vec2::new(w / 2.0, h / 2.0),
    };
    let gradient = match args.get("fill").and_then(Json::as_str).and_then(values::css_gradient) {
        Some(g) => Some(g?),
        None => None,
    };
    let fill = match args.get("fill").and_then(Json::as_str) {
        Some("none") | Some("transparent") => None,
        Some(_) if gradient.is_some() => Some(gradient.as_ref().unwrap().2[0].color),
        Some(_) => Some(values::color(&args["fill"])?),
        None => None,
    };
    let stroke = match args.get("stroke").and_then(Json::as_str) {
        Some("none") | None => None,
        Some(_) => Some(Stroke {
            cap_roundness: 0.0,
            style: Default::default(),
            color: values::color(&args["stroke"])?.into(),
            width: opt_f64(args, "stroke_width").unwrap_or(4.0).into(),
            join: LineJoin::Round,
            cap: LineCap::Round,
            ..Default::default()
        }),
    };
    let size =
        args.get("size").filter(|s| !s.is_null()).map(values::vec2).transpose()?.unwrap_or(Vec2::new(200.0, 200.0));

    let kind = match kind_name {
        "rect" | "ellipse" => {
            let mut shape = if kind_name == "rect" {
                ShapeItem::rect(size, Color::WHITE)
            } else {
                ShapeItem::ellipse(size, Color::WHITE)
            };
            if let Geometry::Rect { corner_radius, .. } = &mut shape.geometry {
                *corner_radius = opt_f64(args, "corner_radius").unwrap_or(0.0).into();
            }
            shape.fill = Some(xenify_doc::Fill::solid(fill.unwrap_or(Color::WHITE)));
            if args.get("fill").and_then(Json::as_str) == Some("none") {
                shape.fill = None;
            }
            shape.stroke = stroke;
            LayerKind::Shape { shapes: vec![shape] }
        }
        "polygon" | "star" | "gear" => {
            use xenify_doc::primitives::FigureKind;
            let (fk, count, inner, hole) = match kind_name {
                "polygon" => (FigureKind::Polygon, 6.0, 0.5, 0.0),
                "star" => (FigureKind::Star, 5.0, 0.5, 0.0),
                _ => (FigureKind::Gear, 12.0, 0.2, 0.3),
            };
            let mut shape = ShapeItem::rect(size, Color::WHITE);
            shape.name = kind_name.to_string();
            shape.geometry = Geometry::Figure {
                position: Vec2::ZERO.into(),
                size: size.into(),
                kind: fk,
                count: opt_f64(args, "count").unwrap_or(count).clamp(3.0, 200.0).into(),
                inner: opt_f64(args, "inner").unwrap_or(inner).clamp(0.02, 0.95).into(),
                hole: opt_f64(args, "hole").unwrap_or(hole).clamp(0.0, 0.9).into(),
            };
            shape.fill = Some(xenify_doc::Fill::solid(fill.unwrap_or(Color::WHITE)));
            if args.get("fill").and_then(Json::as_str) == Some("none") {
                shape.fill = None;
            }
            shape.stroke = stroke;
            LayerKind::Shape { shapes: vec![shape] }
        }
        "path" => {
            let comp_path: BezPath = values::path(args.get("path").ok_or("path layers need `path` (SVG path data)")?)?;
            // Centre the layer on the path so it rotates/scales about its middle.
            let c = comp_path.bounding_box().center().to_vec2();
            position = c;
            let path = Affine::translate(-c) * comp_path;
            let mut shape = ShapeItem::path(path, fill, None);
            shape.stroke = stroke;
            if shape.fill.is_none() && shape.stroke.is_none() {
                shape.fill = Some(xenify_doc::Fill::solid(Color::WHITE));
            }
            LayerKind::Shape { shapes: vec![shape] }
        }
        "text" => {
            let mut text = TextLayer::new(
                values::plain_text(args["text"].as_str().unwrap_or(&name)),
                args["font_family"].as_str().unwrap_or("Inter"),
                opt_f64(args, "font_size").unwrap_or((h / 10.0).round()),
                fill.unwrap_or(Color::WHITE),
            );
            // `position` is the text's centre unless told otherwise.
            text.align = text_align(&args["align"])?.unwrap_or(TextAlign::Center);
            text.valign = text_valign(&args["valign"])?.unwrap_or(TextVAlign::Middle);
            LayerKind::Text(text)
        }
        // An empty group to attach layers to ("null" from older habits).
        "group" | "null" => LayerKind::Group,
        other => return Err(format!("unknown layer type {other:?}")),
    };

    // Positions (and paths) are given in composition space; a parented layer
    // stores them relative to its parent.
    let parent = values::number(&args["parent"]).map(|n| Id(n as u64));
    let parent_inv = match parent {
        Some(p) => world_transform(host.project().comp(host.comp()).map_err(|e| e.to_string())?, p, 0.0).inverse(),
        None => Affine::IDENTITY,
    };
    let kind = match kind {
        LayerKind::Shape { mut shapes } if kind_name == "path" && parent.is_some() => {
            // Re-express the (already centred) path in the parent's orientation/scale.
            let linear = Affine::new({
                let c = parent_inv.as_coeffs();
                [c[0], c[1], c[2], c[3], 0.0, 0.0]
            });
            if let Some(Geometry::Path { path }) = shapes.first_mut().map(|s| &mut s.geometry) {
                *path = (linear * path.value_at(0.0)).into();
            }
            LayerKind::Shape { shapes }
        }
        other => other,
    };
    let position = (parent_inv * position.to_point()).to_vec2();
    let mut layer = Layer::new(id, name.clone(), kind).with_position(position);
    if let Some(r) = opt_f64(args, "rotation") {
        layer.transform.rotation = r.into();
    }
    if let Some(s) = args.get("scale").filter(|s| !s.is_null()) {
        layer.transform.scale = values::vec2(s)?.into();
    }
    if let Some(o) = opt_f64(args, "opacity") {
        layer.opacity = o.clamp(0.0, 1.0).into();
    }
    layer.meta.parent = parent;
    layer.meta.in_point = opt_f64(args, "in_point").unwrap_or(0.0).max(0.0);
    layer.meta.out_point = opt_f64(args, "out_point");

    // Gradient fills span the shape's own bounds.
    if let (Some((kind, angle, stops)), LayerKind::Shape { shapes }) = (&gradient, &mut layer.kind) {
        for s in shapes.iter_mut() {
            let b = xenify_anim::geometry_path(&s.geometry, 0.0).bounding_box();
            if let Some(f) = &mut s.fill {
                f.gradient = Some(xenify_doc::Gradient::across(b, *kind, *angle, stops.clone()).into());
            }
        }
    }
    let comp_id = host.comp();
    // A group's members sit directly below it.
    let index = parent.and_then(|p| {
        let c = host.project().comp(comp_id).ok()?;
        matches!(c.layer(p)?.kind, LayerKind::Group).then(|| c.layer_index(p)).flatten()
    });
    host.execute(&format!("AI: add {name}"), Command::AddLayer { comp: comp_id, index, layer })?;
    let result = with_warnings(host, id, json!({"id": id.0, "name": name, "position": [position.x, position.y]}));
    Ok(done(result, format!("Added {kind_name} “{name}”")))
}

/// Deterministic layout problems for `layer` in its settled state (end of the
/// composition): text overlapping other elements, or anything leaving the
/// frame. Cheap vision models miss these; geometry doesn't.
pub(crate) fn layout_warnings(host: &mut dyn ToolHost, layer: Id) -> Vec<String> {
    let project = host.project().clone();
    let Ok(comp) = project.comp(host.comp()) else { return vec![] };
    // Editing a matte shape: check the layers it masks instead.
    let masked: Vec<Id> =
        comp.layers.iter().filter(|l| l.meta.matte.is_some_and(|m| m.layer == layer)).map(|l| l.id).collect();
    if !masked.is_empty() {
        return masked.into_iter().flat_map(|id| layout_warnings(host, id)).collect();
    }
    let s = &comp.settings;
    let frame = kurbo::Rect::new(0.0, 0.0, s.width as f64, s.height as f64);
    let end = (s.duration - 1.0 / s.fps).max(0.0);
    let scene = xenify_anim::evaluate_project(&project, comp.id, end);
    let mut measure = HostMeasure(host);
    // Fraction of the target layer left visible by its (non-inverted) matte at the end.
    let mut matte_visible = None;
    let boxes: Vec<(Id, bool, kurbo::Rect, &xenify_anim::SceneLayer)> = scene
        .layers
        .iter()
        .filter(|l| l.opacity > 0.05)
        .filter_map(|l| {
            let full = l.transform.transform_rect_bbox(l.bounds(&mut measure)?);
            let mut b = full;
            // A (non-inverted) matte clips what's visible to its own box.
            if let Some(m) = l.matte.as_deref().filter(|m| !m.mode.inverted()) {
                let clip =
                    m.source.as_ref().and_then(|s| Some(s.transform.transform_rect_bbox(s.bounds(&mut measure)?)));
                b = b.intersect(clip.unwrap_or_default());
                if l.id == layer {
                    matte_visible = Some(if full.area() > 0.0 { b.area().max(0.0) / full.area() } else { 1.0 });
                }
                if b.area() <= 0.0 {
                    return None;
                }
            }
            Some((l.id, matches!(l.content, xenify_anim::SceneContent::Text(_)), b, l))
        })
        .collect();
    let mut matte_warning = matte_visible.filter(|v| *v < 0.97).map(|v| {
        format!(
            "“{}” is {}% hidden by its matte at the end. The matte shape must cover where the layer ends up \
             (e.g. the whole area above the reveal line), and only hide where it starts.",
            comp.layer(layer).map_or("?", |l| l.meta.name.as_str()),
            ((1.0 - v) * 100.0).round()
        )
    });
    let name = |id: Id| comp.layer(id).map_or("?".to_string(), |l| l.meta.name.clone());
    let Some(&(_, is_text, mine, mine_layer)) = boxes.iter().find(|b| b.0 == layer) else {
        return matte_warning.into_iter().collect();
    };
    let mut out: Vec<String> = matte_warning.take().into_iter().collect();
    if mine.x0 < -0.5 || mine.y0 < -0.5 || mine.x1 > frame.x1 + 0.5 || mine.y1 > frame.y1 + 0.5 {
        out.push(format!(
            "“{}” extends outside the frame at the end ({:?} vs {}×{}).",
            name(layer),
            [mine.x0.round(), mine.y0.round(), mine.x1.round(), mine.y1.round()],
            s.width,
            s.height
        ));
    }
    if let xenify_anim::SceneContent::Text(t) = &mine_layer.content {
        let size = t.font_size * mine_layer.transform.determinant().abs().sqrt();
        let min = (0.025 * s.height as f64).max(12.0);
        if size < min {
            out.push(format!(
                "“{}” is too small to read ({size:.0}px on screen; use at least {min:.0}px). Make room by moving \
                 elements rather than shrinking text.",
                name(layer)
            ));
        }
    }
    let big = frame.area() * 0.5;
    for &(other, other_text, b, other_layer) in &boxes {
        if other == layer || !(is_text || other_text) || b.area() > big || mine.area() > big {
            continue;
        }
        let i = mine.intersect(b);
        // Only report real contact with drawn content (not empty box space).
        if i.width() > 0.5 && i.height() > 0.5 && xenify_anim::ink_overlap(mine_layer, other_layer, &mut measure) {
            out.push(format!(
                "“{}” overlaps “{}” by {:.0}×{:.0} px at the end — separate them (text must not touch other elements).",
                name(layer),
                name(other),
                i.width(),
                i.height()
            ));
        }
    }
    out
}

/// WCAG relative luminance of an sRGB colour.
fn luminance(c: Color) -> f64 {
    let lin = |v: f64| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio (1..21) between two colours.
pub fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Exact layout measurements at `time` (see the `check_layout` tool).
pub(crate) fn check_layout(host: &mut dyn ToolHost, time: f64) -> Json {
    let project = host.project().clone();
    let Ok(comp) = project.comp(host.comp()) else { return json!({"error": "no composition"}) };
    let s = &comp.settings;
    let (w, h) = (s.width as f64, s.height as f64);
    let scene = xenify_anim::evaluate_project(&project, comp.id, time);
    let mut measure = HostMeasure(host);
    // (index in stack, id, box, scene layer)
    let visible: Vec<(usize, Id, kurbo::Rect, &xenify_anim::SceneLayer)> = scene
        .layers
        .iter()
        .enumerate()
        .filter(|(_, l)| l.opacity > 0.05)
        .filter_map(|(i, l)| Some((i, l.id, l.transform.transform_rect_bbox(l.bounds(&mut measure)?), l)))
        .collect();
    let name = |id: Id| comp.layer(id).map_or("?".to_string(), |l| l.meta.name.clone());
    let r = |v: f64| v.round();
    let mut layers = Vec::new();
    for &(i, id, b, l) in &visible {
        let mut o = json!({
            "id": id.0,
            "name": name(id),
            "box": [r(b.x0), r(b.y0), r(b.x1), r(b.y1)],
            "size": [r(b.width()), r(b.height())],
            "centre_offset_from_frame_centre": [r(b.center().x - w / 2.0), r(b.center().y - h / 2.0)],
            "margins_left_top_right_bottom": [r(b.x0), r(b.y0), r(w - b.x1), r(h - b.y1)],
        });
        if let xenify_anim::SceneContent::Text(t) = &l.content {
            // What's behind the text: the topmost filled shape below it covering
            // the text's centre, else the background.
            let behind = visible
                .iter()
                .rev()
                .filter(|(j, _, bb, _)| *j < i && bb.contains(b.center()))
                .find_map(|(_, _, _, below)| match &below.content {
                    xenify_anim::SceneContent::Shapes(items) => items.iter().rev().find_map(|it| it.fill),
                    _ => None,
                })
                .unwrap_or(s.background);
            let ratio = contrast(t.color, behind);
            let size = t.font_size * l.transform.determinant().abs().sqrt();
            o["text"] = json!({
                "font_size_on_screen": r(size),
                "font_size_percent_of_height": (size / h * 1000.0).round() / 10.0,
                "contrast_ratio": (ratio * 10.0).round() / 10.0,
                "contrast_ok": ratio >= 4.5 || (ratio >= 3.0 && size >= 0.04 * h),
                "behind": behind.to_hex(),
            });
        }
        let out = b.x0 < -0.5 || b.y0 < -0.5 || b.x1 > w + 0.5 || b.y1 > h + 0.5;
        if out {
            o["outside_frame"] = json!(true);
        }
        layers.push(o);
    }
    let mut overlaps = Vec::new();
    for (a_i, a) in visible.iter().enumerate() {
        for b in &visible[a_i + 1..] {
            let i = a.2.intersect(b.2);
            if i.width() > 0.5 && i.height() > 0.5 && xenify_anim::ink_overlap(a.3, b.3, &mut measure) {
                let text = matches!(a.3.content, xenify_anim::SceneContent::Text(_))
                    || matches!(b.3.content, xenify_anim::SceneContent::Text(_));
                overlaps.push(json!({
                    "layers": [name(a.1), name(b.1)],
                    "overlap_px": [r(i.width()), r(i.height())],
                    "involves_text": text,
                }));
            }
        }
    }
    json!({
        "time": time,
        "frame": [s.width, s.height],
        "layers_bottom_to_top": layers,
        "overlaps": overlaps,
        "note": "All values are exact, in composition pixels. Text contrast ≥ 4.5 is good (≥ 3 for large text).",
    })
}

/// Adds layout warnings for `layer` to a tool result.
fn with_warnings(host: &mut dyn ToolHost, layer: Id, mut result: Json) -> Json {
    let w = layout_warnings(host, layer);
    if !w.is_empty() {
        result["layout_warnings"] = json!(w);
    }
    result
}

/// Adds imported/traced `layers` as a named group, optionally sized
/// (`width`) and placed (`position`) from `args`. Returns the tool result and
/// the number of shapes.
pub(crate) fn add_artwork(
    host: &mut dyn ToolHost,
    layers: Vec<Layer>,
    args: &Json,
    default_name: &str,
) -> Result<(Json, usize), String> {
    let name = args["name"].as_str().unwrap_or(default_name).to_string();
    let ids: Vec<Id> = layers.iter().map(|l| l.id).collect();
    let comp = host.comp();
    let adds = layers.into_iter().map(|layer| Command::AddLayer { comp, index: None, layer }).collect();
    host.execute(&format!("AI: add {name}"), Command::batch(adds))?;
    let (project, time) = (host.project().clone(), host.time());
    let (cmd, gid) = xenify_ops::group::group(&project, comp, &ids, &name, time, &mut HostMeasure(host))
        .map_err(|e| e.to_string())?;
    host.execute("AI: group", cmd)?;
    let mut edits = Vec::new();
    if let Some(w) = opt_f64(args, "width") {
        let project = host.project().clone();
        let c = project.comp(comp).map_err(|e| e.to_string())?;
        let scene = xenify_anim::evaluate_project(&project, c.id, time);
        if let Some(b) = xenify_anim::world_box(c, &scene, gid, &mut HostMeasure(host)) {
            let k = (w / b.width().max(1.0)).max(0.01);
            edits.push(Command::SetProperty {
                layer: gid,
                path: PropPath::Scale,
                property: AnyProperty::Vec2(Vec2::new(k, k).into()),
            });
        }
    }
    if let Some(p) = args.get("position").filter(|p| !p.is_null()) {
        edits.push(Command::SetProperty {
            layer: gid,
            path: PropPath::Position,
            property: AnyProperty::Vec2(values::vec2(p)?.into()),
        });
    }
    if !edits.is_empty() {
        host.execute(&format!("AI: place {name}"), Command::batch(edits))?;
    }
    let result =
        with_warnings(host, gid, json!({"group_id": gid.0, "layer_ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
    Ok((result, ids.len()))
}

/// `add_emoji` with `animated`: a Noto animated emoji as a symbol.
fn add_animated_emoji(args: &Json, host: &mut dyn ToolHost) -> Result<ToolOutcome, String> {
    let key = args["emoji"].as_str().ok_or("missing `emoji`")?;
    let (e, data) = host.animated_emoji(key)?;
    let comp = host.comp();
    let (w, h) = {
        let c = host.project().comp(comp).map_err(|e| e.to_string())?;
        (c.settings.width as f64, c.settings.height as f64)
    };
    let at = match args.get("position").filter(|p| !p.is_null()) {
        Some(p) => values::vec2(p)?,
        None => Vec2::new(w / 2.0, h / 2.0),
    };
    let size = opt_f64(args, "size").unwrap_or((w.min(h) / 4.0).round()).max(4.0);
    let looped = args.get("loop").and_then(Json::as_bool).unwrap_or(true);
    let start = opt_f64(args, "start").unwrap_or(host.time());
    let import = xenify_export::lottie_import::import_lottie(&data, host.project().next_id)?;
    let (symbol, layer) = (Id(import.next_id), Id(import.next_id + 1));
    let (mut main, extras) = import.into_compositions(symbol);
    main.settings.name = format!("{} {} (animated)", e.glyph, e.name);
    let cmd =
        xenify_ops::precomp::add_precomp(host.project(), comp, main, extras, layer, at, start, Some(size), looped)
            .map_err(|e| e.to_string())?;
    host.execute(&format!("AI: animated {}", e.name), cmd)?;
    Ok(done(
        json!({"ok": true, "layer": layer.0, "comp": symbol.0, "credit": xenify_export::animated_emoji::CREDIT}),
        format!("{} {} (animated)", e.glyph, e.name),
    ))
}

/// Lets a [`ToolHost`] measure text for [`scene_summary`].
pub(crate) struct HostMeasure<'a>(pub &'a mut dyn ToolHost);

impl xenify_anim::TextMeasure for HostMeasure<'_> {
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect {
        self.0.text_bounds(text)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_handled_tool_is_advertised() {
        let src = include_str!("tools.rs");
        let defs = super::definitions(true);
        let advertised: Vec<&str> = defs.iter().filter_map(|d| d["function"]["name"].as_str()).collect();
        // Tool names are the string arms of the dispatch match.
        let run = &src[src.find("\"get_scene\" => {").unwrap()..src.find("\"look_at_frame\" => Ok").unwrap() + 20];
        for line in run.lines().filter(|l| l.starts_with("        \"") && l.contains("=>")) {
            for name in line.split("=>").next().unwrap().split('|').map(|n| n.trim().trim_matches('"')) {
                assert!(advertised.contains(&name), "tool {name:?} is handled but has no schema");
            }
        }
        assert!(serde_json::to_string(&defs).unwrap().contains("draw_on"));
    }
}
