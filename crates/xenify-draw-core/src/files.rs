//! Files in and out that don't need the GPU, so they work in the browser
//! too: SVG (a frame, or animated), Lottie (JSON or a web page), and
//! importing SVG, Lottie and pictures traced into shapes.

use base64::Engine as _;
use serde_json::{Value as Json, json};
use xenify_doc::{Id, Layer, Vec2};
use xenify_ops::Command;

use crate::{Session, err, f};

/// `rect: [x0, y0, x1, y1]` (composition pixels).
fn rect_arg(args: &Json) -> Result<kurbo::Rect, String> {
    let a = args.get("rect").and_then(Json::as_array).ok_or("missing rect")?;
    let n: Vec<f64> = a.iter().filter_map(Json::as_f64).collect();
    if n.len() != 4 {
        return Err("rect needs 4 numbers".into());
    }
    let r = kurbo::Rect::new(n[0], n[1], n[2], n[3]).abs();
    if r.width() < 1.0 || r.height() < 1.0 {
        return Err("Drag a bigger rectangle.".into());
    }
    Ok(r)
}

fn bytes_arg(args: &Json) -> Result<Vec<u8>, String> {
    if let Some(t) = args.get("text").and_then(Json::as_str) {
        return Ok(t.as_bytes().to_vec());
    }
    base64::engine::general_purpose::STANDARD
        .decode(args.get("base64").and_then(Json::as_str).ok_or("missing file")?)
        .map_err(err)
}

impl Session {
    pub(crate) fn files_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "export_svg" => self.export_svg(args),
            "export_lottie" => self.export_lottie(args),
            "import_svg" => self.import_svg(args),
            "import_lottie" => self.import_lottie(args),
            "trace" => self.trace(args),
            "trace_layer" => self.trace_layer(args),
            "crop_canvas" => self.crop_canvas(args),
            "emoji_groups" => Ok(json!(xenify_export::emoji::groups())),
            "emoji_search" => {
                let q = args.get("query").and_then(Json::as_str).unwrap_or("").trim().to_string();
                let group = args.get("group").and_then(Json::as_str);
                let list: Vec<&xenify_export::emoji::Emoji> = if q.is_empty() {
                    xenify_export::emoji::library().iter().filter(|e| group.is_none_or(|g| e.group == g)).collect()
                } else {
                    xenify_export::emoji::search(&q)
                };
                let limit = f(args, "limit").unwrap_or(200.0) as usize;
                Ok(json!(list.iter().take(limit).map(|e| json!({ "glyph": e.glyph, "name": e.name, "group": e.group, "svg": e.svg })).collect::<Vec<_>>()))
            }
            // Emoji by glyph (recent ones, stills for the animated ones); null where there's none.
            "emoji_find" => Ok(json!(args.get("glyphs").and_then(Json::as_array).map(|a| a.iter().map(|g| {
                g.as_str().and_then(xenify_export::emoji::find).map_or(Json::Null, |e| json!({ "glyph": e.glyph, "name": e.name, "group": e.group, "svg": e.svg }))
            }).collect::<Vec<_>>()).unwrap_or_default())),
            "add_emoji" => self.add_emoji(args),
            "emoji_confetti" => self.emoji_confetti(args),
            "animated_emoji_list" => xenify_export::animated_emoji::parse_index(args.get("text").and_then(Json::as_str).unwrap_or("").as_bytes())
                .map(|list| json!(list.iter().map(|e| json!({ "codepoint": e.codepoint, "name": e.name, "glyph": e.glyph, "tags": e.tags, "url": e.lottie_url() })).collect::<Vec<_>>())),
            "giphy_url" => Ok(json!(xenify_export::giphy::url(
                args.get("key").and_then(Json::as_str).unwrap_or(""),
                args.get("query").and_then(Json::as_str).unwrap_or(""),
                f(args, "limit").unwrap_or(24.0) as u32,
                f(args, "offset").unwrap_or(0.0) as u32,
            ))),
            "giphy_parse" => xenify_export::giphy::parse(args.get("text").and_then(Json::as_str).unwrap_or("").as_bytes()).map(|list| {
                json!(list.iter().map(|g| json!({ "id": g.id, "title": g.title, "credit": g.credit(), "preview": g.preview, "full": g.full, "width": g.width, "height": g.height, "page": g.page })).collect::<Vec<_>>())
            }),
            "crop_layers" => self.crop_layers(args),
            "slices" => Ok(json!(self.comp_settings().map(|s| s.slices.iter().map(|x| json!({ "name": x.name, "rect": [x.rect.x0, x.rect.y0, x.rect.x1, x.rect.y1] })).collect::<Vec<_>>()).unwrap_or_default())),
            "add_slice" => self.edit_slices(args, "add"),
            "set_slice" => self.edit_slices(args, "set"),
            "remove_slice" => self.edit_slices(args, "remove"),
            "cropped_project" => self.export_project(args).map(|p| Json::String(p.to_json())),
            _ => return None,
        })
    }

    /// Opens an `.xdraw` file (its pictures and clips go in the store beside
    /// the canvas, see xenify_doc::blobs); the state.
    pub fn open_file(&mut self, bytes: &[u8]) -> Result<Json, String> {
        let p = xenify_doc::file::unpack(bytes)?;
        self.replace(Session::new(p));
        Ok(self.state())
    }

    /// The canvas as an `.xdraw` file, with `preview` (PNG) and whether
    /// it's `animated` for documents that show it; it counts as saved.
    pub fn save_file(&mut self, preview: Option<&[u8]>, animated: bool) -> Result<Vec<u8>, String> {
        let bytes = xenify_doc::file::pack(self.project(), preview, animated)?;
        self.editor.mark_saved();
        Ok(bytes)
    }

    /// The current frame, or (with `animated`) the whole animation, as SVG text.
    /// The project to export: as it is, or cropped to `rect` (a slice).
    fn export_project(&self, args: &Json) -> Result<xenify_doc::Project, String> {
        let mut p = match args.get("rect") {
            Some(_) => xenify_ops::crop::cropped_project(self.project(), self.comp, rect_arg(args)?).map_err(err)?,
            None => self.project().clone(),
        };
        // Exporting see-through canvases to a format that can't keep it: the colour chosen to go behind.
        if let Some(Some(bg)) = self.exporting {
            for c in &mut p.compositions {
                if c.settings.background.a < 0.999 {
                    c.settings.background = bg;
                }
            }
        }
        Ok(p)
    }

    fn export_svg(&mut self, args: &Json) -> Result<Json, String> {
        let p = self.export_project(args)?;
        if args.get("animated").and_then(Json::as_bool).unwrap_or(false) {
            let opts = xenify_export::svg::AnimatedSvgOptions {
                looping: args.get("loop").and_then(Json::as_bool).unwrap_or(true),
            };
            return xenify_export::svg::export_animated_svg(&p, self.comp, &mut self.text, opts).map(Json::String);
        }
        let scene = xenify_anim::evaluate_project(&p, self.comp, self.time);
        Ok(Json::String(xenify_export::svg::export_svg(&scene, &mut self.text)))
    }

    /// The animation as Lottie JSON, or (with `html`) a web page that plays it.
    fn export_lottie(&mut self, args: &Json) -> Result<Json, String> {
        let p = self.export_project(args)?;
        let lottie = xenify_export::lottie::export_lottie(&p, self.comp, &mut self.text)?;
        if args.get("html").and_then(Json::as_bool).unwrap_or(false) {
            let c = p.comp(self.comp).map_err(err)?;
            return Ok(Json::String(xenify_export::lottie::export_html(
                &lottie,
                &c.settings.name,
                c.settings.background,
            )));
        }
        Ok(Json::String(lottie.to_string()))
    }

    /// Adds layers (and their pictures) as one undo step, grouped under `name` when there are several.
    fn add_imported(
        &mut self,
        name: &str,
        layers: Vec<Layer>,
        assets: Vec<xenify_doc::ImageAsset>,
    ) -> Result<Json, String> {
        if layers.is_empty() {
            return Err(format!("{name} has nothing Draw can show."));
        }
        let ids: Vec<Id> = layers.iter().filter(|l| l.meta.parent.is_none()).map(|l| l.id).collect();
        let mut cmds: Vec<Command> = assets.into_iter().map(|asset| Command::AddAsset { asset }).collect();
        cmds.extend(layers.into_iter().map(|layer| Command::AddLayer { comp: self.comp, index: None, layer }));
        self.editor.begin_group(format!("Import {name}"));
        let done = self.exec("Add shapes", Command::batch(cmds));
        let mut select = ids.clone();
        if done.is_ok() && ids.len() > 1 {
            let (p, comp, time) = (self.project().clone(), self.comp, self.time);
            if let Ok((cmd, gid)) = xenify_ops::group::group(&p, comp, &ids, name, time, &mut self.text)
                && self.exec("Group", cmd).is_ok()
            {
                select = vec![gid];
            }
        }
        self.editor.end_group();
        done?;
        self.selection = select;
        Ok(json!(self.selection.iter().map(|i| i.0).collect::<Vec<_>>()))
    }

    fn import_svg(&mut self, args: &Json) -> Result<Json, String> {
        let data = bytes_arg(args)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("SVG").to_string();
        let (w, h) = self.comp_settings().map_or((1080, 1080), |s| (s.width, s.height));
        let import = xenify_export::svg::import_svg_document(&data, self.project().next_id, w, h, Default::default())?;
        self.add_imported(&name, import.layers, import.assets)
    }

    /// A Lottie animation, as a symbol placed in the middle of the canvas.
    fn import_lottie(&mut self, args: &Json) -> Result<Json, String> {
        let data = bytes_arg(args)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Lottie").to_string();
        let import = xenify_export::lottie_import::import_lottie(&data, self.project().next_id)?;
        let summary = import.summary();
        let (comp_id, layer) = (Id(import.next_id), Id(import.next_id + 1));
        let (mut main, extras) = import.into_compositions(comp_id);
        if main.settings.name.trim().is_empty()
            || main.settings.name == "Lottie"
            || args.get("rename").and_then(Json::as_bool).unwrap_or(false)
        {
            main.settings.name = name.clone();
        }
        let (w, h) = self.comp_settings().map_or((1080.0, 1080.0), |s| (s.width as f64, s.height as f64));
        // Animated emoji: dropped at a point, a quarter of the frame across, playing over and over (or once).
        let at = args
            .get("at")
            .and_then(|v| Some(Vec2::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
            .unwrap_or(Vec2::new(w / 2.0, h / 2.0));
        let size = f(args, "size");
        let looped = args.get("looped").and_then(Json::as_bool).unwrap_or(false);
        let cmd = xenify_ops::precomp::add_precomp(
            self.project(),
            self.comp,
            main,
            extras,
            layer,
            at,
            self.time,
            size,
            looped,
        )
        .map_err(err)?;
        self.exec(&format!("Import {name}"), cmd)?;
        self.selection = vec![layer];
        Ok(json!({ "id": layer.0, "summary": summary }))
    }

    /// An emoji as editable shapes (grouped), in the middle of the canvas.
    fn add_emoji(&mut self, args: &Json) -> Result<Json, String> {
        let key = args.get("glyph").and_then(Json::as_str).ok_or("missing emoji")?;
        let e = xenify_export::emoji::find(key).ok_or("no such emoji")?;
        let (w, h) = self.comp_settings().map_or((1080.0, 1080.0), |s| (s.width as f64, s.height as f64));
        let size = f(args, "size").unwrap_or(w.min(h) * 0.3);
        // Where it was dropped, else the middle.
        let at = args
            .get("at")
            .and_then(|v| Some(Vec2::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
            .unwrap_or(Vec2::new(w / 2.0, h / 2.0));
        let layers = xenify_export::emoji::layers(e, self.project().next_id, at, size)?;
        let name = e.name.clone();
        let mut name_cap = name.clone();
        if let Some(c) = name_cap.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        self.add_imported(&name_cap, layers, Vec::new())
    }

    /// A confetti burst whose pieces are an emoji: the emoji is added (hidden) as the particles' sprite. One undo step.
    fn emoji_confetti(&mut self, args: &Json) -> Result<Json, String> {
        let key = args.get("glyph").and_then(Json::as_str).ok_or("missing emoji")?;
        let e = xenify_export::emoji::find(key).ok_or("no such emoji")?;
        self.editor.begin_group(format!("{} confetti", e.name));
        let r = (|| -> Result<Json, String> {
            let layers = xenify_export::emoji::layers(e, self.project().next_id, Vec2::new(-1000.0, -1000.0), 120.0)?;
            let added = self.add_imported(&format!("{} (confetti sprite)", e.name), layers, Vec::new())?;
            let sprite = Id(added.as_array().and_then(|a| a.first()).and_then(Json::as_u64).ok_or("no sprite")?);
            let mut meta = self.project().layer(sprite).map_err(err)?.meta.clone();
            meta.visible = false;
            meta.name = format!("{} (confetti sprite)", e.name);
            self.exec("Hide sprite", Command::SetLayerMeta { layer: sprite, meta })?;
            let id = Id(self.project().next_id);
            let mut cmd = xenify_ops::particles::add_preset(self.project(), self.comp, id, "Confetti Burst", None)
                .map_err(err)?;
            if let Command::AddLayer { layer, .. } = &mut cmd
                && let xenify_doc::LayerKind::Particles(em) = &mut layer.kind
            {
                em.particle = xenify_doc::ParticleShape::Layer { layer: sprite };
                em.size = 48.0.into();
                em.spin = 180.0.into();
                em.start = self.time;
                layer.meta.name = format!("{} confetti", e.name);
            }
            self.exec("Emoji confetti", cmd)?;
            self.selection = vec![id];
            Ok(json!(id.0))
        })();
        self.editor.end_group();
        r
    }

    fn crop_canvas(&mut self, args: &Json) -> Result<Json, String> {
        let cmd = xenify_ops::crop::crop_comp(self.project(), self.comp, rect_arg(args)?).map_err(err)?;
        self.exec("Crop canvas", cmd)?;
        Ok(Json::Null)
    }

    fn crop_layers(&mut self, args: &Json) -> Result<Json, String> {
        if self.selection.is_empty() {
            return Err("Select the layers to crop first.".into());
        }
        let cmd = xenify_ops::crop::crop_layers(
            self.project(),
            self.comp,
            &self.selection,
            rect_arg(args)?,
            self.time,
            self.auto_key,
        )
        .map_err(err)?;
        self.exec("Crop layers", cmd)?;
        Ok(Json::Null)
    }

    /// Named export regions: `add` (rect, name), `set` (index, name and/or rect), `remove` (index).
    fn edit_slices(&mut self, args: &Json, how: &str) -> Result<Json, String> {
        let mut s = self.comp_settings().cloned().ok_or("no canvas")?;
        let index = f(args, "index").map(|i| i as usize);
        match how {
            "add" => {
                let name = args
                    .get("name")
                    .and_then(Json::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Slice {}", s.slices.len() + 1));
                s.slices.push(xenify_doc::Slice { name, rect: rect_arg(args)? });
            }
            "set" => {
                let i = index.filter(|i| *i < s.slices.len()).ok_or("no such slice")?;
                if let Some(n) = args.get("name").and_then(Json::as_str) {
                    s.slices[i].name = n.to_string();
                }
                if args.get("rect").is_some() {
                    s.slices[i].rect = rect_arg(args)?;
                }
            }
            _ => {
                let i = index.filter(|i| *i < s.slices.len()).ok_or("no such slice")?;
                s.slices.remove(i);
            }
        }
        let comp = self.comp;
        self.exec("Slices", Command::SetCompSettings { comp, settings: s })?;
        Ok(Json::Null)
    }

    /// A picture traced into shapes: `colors` 1–8, `noise` 1–64, `smooth`, `remove_background`.
    fn trace(&mut self, args: &Json) -> Result<Json, String> {
        let data = bytes_arg(args)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Traced picture").to_string();
        let o = trace_options(args);
        let (w, h) = self.comp_settings().map_or((1080, 1080), |s| (s.width, s.height));
        let layers = xenify_export::trace::trace_to_layers(&data, &o, self.project().next_id, w, h)?;
        self.add_imported(&name, layers, Vec::new())
    }

    /// Vector shapes over picture layer `id`, where it is (the picture stays, underneath).
    fn trace_layer(&mut self, args: &Json) -> Result<Json, String> {
        let id = crate::id_arg(args, "id")?;
        let layer = self.project().layer(id).map_err(err)?.clone();
        let xenify_doc::LayerKind::Image { asset, .. } = layer.kind else {
            return Err("Select a picture to trace.".into());
        };
        let data = self.project().asset(asset).map(|a| a.data.clone()).ok_or("the picture is missing")?;
        let c = self.project().comp(self.comp).map_err(err)?;
        let place = xenify_anim::world_transform(c, id, self.time);
        let layers =
            xenify_export::trace::trace_to_layers_at(&data, &trace_options(args), self.project().next_id, place)?;
        self.add_imported(&format!("{} (traced)", layer.meta.name), layers, Vec::new())
    }
}

/// Trace settings from a call's arguments.
fn trace_options(args: &Json) -> xenify_export::trace::TraceOptions {
    {
        let mut o = xenify_export::trace::TraceOptions::default();
        if let Some(c) = f(args, "colors") {
            o.colors = c.clamp(1.0, 8.0) as u8;
        }
        if let Some(n) = f(args, "noise") {
            o.noise = n.clamp(1.0, 64.0) as usize;
        }
        if let Some(s) = f(args, "simplify") {
            o.simplify = s.clamp(0.0, 10.0);
        }
        o.smooth = args.get("smooth").and_then(Json::as_bool).unwrap_or(o.smooth);
        o.remove_background = args.get("remove_background").and_then(Json::as_bool).unwrap_or(o.remove_background);
        o
    }
}
