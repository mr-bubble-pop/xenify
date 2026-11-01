//! Sound: audio files (kept inside the canvas file), sound effects and
//! music made by the program (stored with their recipe, so they can be made
//! again with other settings), the presentation's soundtrack, and the
//! mix the screens play.

use base64::Engine as _;
use serde_json::{Value as Json, json};
use xenify_audio::synth::{MusicRecipe, Recipe, Sfx, SfxRecipe};
use xenify_doc::{Id, LayerKind, MediaAsset, MediaKind, MediaSource};
use xenify_ops::Command;

use crate::{Session, err, f, id_arg};

/// A clip's file: its bytes inside the canvas (`base64`), or (desktop) linked by `path` (`size` bytes).
fn source_of(args: &Json) -> Result<(MediaSource, u64), String> {
    if let Some(path) = args.get("path").and_then(Json::as_str).filter(|p| !p.is_empty()) {
        let size = f(args, "size").unwrap_or(0.0) as u64;
        return Ok((
            MediaSource::Linked { path: path.to_string() },
            xenify_doc::hash_bytes(format!("{path}:{size}").as_bytes()),
        ));
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(args.get("base64").and_then(Json::as_str).ok_or("missing file")?)
        .map_err(err)?;
    let hash = xenify_doc::hash_bytes(&data);
    Ok((MediaSource::Embedded { data: data.into() }, hash))
}

impl Session {
    pub(crate) fn sound_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "add_sound" => self.add_sound(args, false),
            "add_video" => self.add_video(args),
            "add_soundtrack_clip" => self.add_sound(args, true),
            "sfx_kinds" => Ok(json!(Sfx::ALL.iter().map(|k| json!({ "kind": k, "label": k.label() })).collect::<Vec<_>>())),
            "music_default" => serde_json::to_value(Recipe::Music(MusicRecipe::default())).map_err(err),
            "add_generated" => self.add_generated(args),
            "regenerate" => self.regenerate(args),
            "recipe" => self.recipe(args),
            "soundtrack" => serde_json::to_value(&self.project().soundtrack).map_err(err).map(|mut v| {
                let names: serde_json::Map<String, Json> =
                    self.project().media.iter().map(|m| (m.id.0.to_string(), json!(m.name))).collect();
                v["names"] = Json::Object(names);
                v
            }),
            "set_soundtrack" => serde_json::from_value(args.get("soundtrack").cloned().unwrap_or_default())
                .map_err(err)
                .and_then(|soundtrack| self.exec("Soundtrack", Command::SetSoundtrack { soundtrack }).map(|_| Json::Null)),
            "sounds_to_decode" => Ok(self.sounds_to_decode()),
            "media_bytes" => self.media_bytes(args),
            "media_list" => Ok(json!(self.project().media.iter().map(|m| json!({
                "id": m.id.0, "name": m.name, "kind": m.kind, "mime": m.mime, "hash": m.hash.to_string(),
                "fps": m.fps, "width": m.width, "height": m.height, "duration": m.duration,
                "path": match &m.source { MediaSource::Linked { path } => Some(path.clone()), _ => None },
                "has_audio": m.has_audio,
            })).collect::<Vec<_>>())),
            "embed_media" => self.embed_media(args),
            "media_fit" => id_arg(args, "id").and_then(|id| {
                let fill = args.get("fill").and_then(Json::as_bool).unwrap_or(false);
                let cmd = xenify_ops::media::fit(self.project(), id, self.time, fill, self.auto_key).map_err(err)?;
                self.exec(if fill { "Fill frame" } else { "Fit frame" }, cmd).map(|_| Json::Null)
            }),
            "media_trim" => id_arg(args, "id").and_then(|id| {
                let cmd = xenify_ops::media::fit_length(self.project(), id).map_err(err)?;
                self.exec("Trim to clip", cmd).map(|_| Json::Null)
            }),
            "media_info" => id_arg(args, "id").and_then(|id| {
                let l = self.project().layer(id).map_err(err)?;
                let asset = match &l.kind {
                    xenify_doc::LayerKind::Audio(a) => a.asset,
                    xenify_doc::LayerKind::Video(v) => v.asset,
                    _ => return Ok(Json::Null),
                };
                let m = self.project().media(asset).ok_or("the clip is missing")?;
                Ok(json!({
                    "asset": m.id.0, "name": m.name, "video": m.is_video(), "width": m.width, "height": m.height, "fps": m.fps, "duration": m.duration, "has_audio": m.has_audio,
                    "linked": match &m.source { MediaSource::Linked { path } => Some(path.clone()), _ => None },
                    "size": match &m.source { MediaSource::Embedded { data } => data.len(), _ => 0 },
                }))
            }),
            // A generated sound (music bed or effect) for the whole presentation.
            "recipe_to_soundtrack" => {
                let recipe: Recipe = match serde_json::from_value(args.get("recipe").cloned().unwrap_or_default()) {
                    Ok(r) => r,
                    Err(e) => return Some(Err(e.to_string())),
                };
                let looped = matches!(&recipe, Recipe::Music(m) if m.looped);
                let media = xenify_audio::synth::media_asset(&recipe, Id(self.project().next_id));
                let mut cmd = xenify_ops::media::add_to_soundtrack(self.project(), media, 0.0);
                if looped
                    && let Command::Batch { commands } = &mut cmd
                    && let Some(Command::SetSoundtrack { soundtrack }) = commands.last_mut()
                    && let Some(c) = soundtrack.clips.last_mut()
                {
                    c.audio.looped = true;
                }
                self.exec("Soundtrack", cmd).map(|_| Json::Null)
            }
            "has_sound" => Ok(json!(xenify_audio::has_sound(self.project()))),
            "set_layer_json" => self.set_layer_json(args),
            _ => return None,
        })
    }

    /// An audio file (base64; its `duration` measured by the screens) as a
    /// sound layer at the playhead, or with `soundtrack` a clip of the
    /// presentation's soundtrack starting at `start`.
    fn add_sound(&mut self, args: &Json, soundtrack: bool) -> Result<Json, String> {
        let (source, hash) = source_of(args)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Sound").to_string();
        let mime = args.get("mime").and_then(Json::as_str).unwrap_or("audio/mpeg").to_string();
        let duration = f(args, "duration").unwrap_or(0.0).max(0.01);
        let id = Id(self.project().next_id);
        let media = MediaAsset {
            id,
            name: name.clone(),
            kind: MediaKind::Audio,
            mime,
            hash,
            source,
            duration,
            width: 0,
            height: 0,
            fps: 0.0,
            has_audio: true,
            recipe: None,
        };
        if soundtrack {
            let cmd = xenify_ops::media::add_to_soundtrack(self.project(), media, f(args, "start").unwrap_or(0.0));
            self.exec("Add to the soundtrack", cmd)?;
            return Ok(Json::Null);
        }
        let layer = Id(id.0 + 1);
        let cmd = xenify_ops::media::add_clip(self.project(), self.comp, media, layer, self.time).map_err(err)?;
        self.exec(&format!("Add {name}"), cmd)?;
        self.selection = vec![layer];
        Ok(json!(layer.0))
    }

    /// A video file (base64; its size, length and frame rate measured by the screens) as a layer at the playhead.
    fn add_video(&mut self, args: &Json) -> Result<Json, String> {
        let (source, hash) = source_of(args)?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Video").to_string();
        let id = Id(self.project().next_id);
        let media = MediaAsset {
            id,
            name: name.clone(),
            kind: MediaKind::Video,
            mime: args.get("mime").and_then(Json::as_str).unwrap_or("video/mp4").to_string(),
            hash,
            source,
            duration: f(args, "duration").unwrap_or(1.0).max(0.04),
            width: f(args, "width").unwrap_or(1280.0) as u32,
            height: f(args, "height").unwrap_or(720.0) as u32,
            fps: f(args, "fps").unwrap_or(30.0).clamp(1.0, 240.0),
            has_audio: args.get("has_audio").and_then(Json::as_bool).unwrap_or(true),
            recipe: None,
        };
        let layer = Id(id.0 + 1);
        let cmd = xenify_ops::media::add_clip(self.project(), self.comp, media, layer, self.time).map_err(err)?;
        self.exec(&format!("Add {name}"), cmd)?;
        self.selection = vec![layer];
        Ok(json!(layer.0))
    }

    /// A sound made by the program at the playhead: `recipe` as stored, or a sound effect `kind`.
    fn add_generated(&mut self, args: &Json) -> Result<Json, String> {
        let recipe: Recipe = match args.get("recipe") {
            Some(r) => serde_json::from_value(r.clone()).map_err(err)?,
            None => Recipe::Sfx(SfxRecipe::of(
                serde_json::from_value(args.get("kind").cloned().unwrap_or(json!("pop"))).map_err(err)?,
            )),
        };
        let media = xenify_audio::synth::media_asset(&recipe, Id(self.project().next_id));
        let layer = Id(media.id.0 + 1);
        let cmd = xenify_ops::media::add_clip(self.project(), self.comp, media, layer, self.time).map_err(err)?;
        self.exec(&format!("Add {}", recipe.name()), cmd)?;
        self.selection = vec![layer];
        Ok(json!(layer.0))
    }

    fn audio_asset(&self, layer: Id) -> Result<&MediaAsset, String> {
        let l = self.project().layer(layer).map_err(err)?;
        let LayerKind::Audio(a) = &l.kind else { return Err("not a sound layer".into()) };
        self.project().media(a.asset).ok_or_else(|| "the sound is missing".into())
    }

    /// The recipe a sound layer's sound was made with (null for recorded sound).
    fn recipe(&self, args: &Json) -> Result<Json, String> {
        let m = self.audio_asset(id_arg(args, "id")?)?;
        Ok(xenify_audio::synth::recipe_of(m).and_then(|r| serde_json::to_value(r).ok()).unwrap_or(Json::Null))
    }

    /// Makes a generated sound again with a changed recipe (one undo step).
    fn regenerate(&mut self, args: &Json) -> Result<Json, String> {
        let layer = id_arg(args, "id")?;
        let recipe: Recipe =
            serde_json::from_value(args.get("recipe").cloned().ok_or("missing recipe")?).map_err(err)?;
        let old = self.audio_asset(layer)?.clone();
        let mut media = xenify_audio::synth::media_asset(&recipe, old.id);
        media.name = old.name.clone();
        self.editor
            .execute_merged("Change sound", format!("sound:{}", layer.0), Command::SetMediaAsset { media })
            .map_err(err)?;
        Ok(Json::Null)
    }

    /// Sounds the screens must decode for the mix (everything but WAV, which the engine reads itself).
    fn sounds_to_decode(&self) -> Json {
        json!(self
            .project()
            .media
            .iter()
            .filter(|m| m.has_audio && !(m.mime.contains("wav") && matches!(m.source, MediaSource::Embedded { .. })))
            .map(|m| json!({ "id": m.id.0, "hash": m.hash.to_string(), "mime": m.mime, "path": match &m.source { MediaSource::Linked { path } => Some(path.clone()), _ => None } }))
            .collect::<Vec<_>>())
    }

    fn media_bytes(&self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let m = self.project().media(id).ok_or("no such sound")?;
        match &m.source {
            MediaSource::Embedded { data } => Ok(json!(base64::engine::general_purpose::STANDARD.encode(data))),
            MediaSource::Linked { path } => Ok(json!({ "path": path })),
        }
    }

    /// Keeps a linked clip inside the canvas file from now on (`base64`: its bytes, read by the screens).
    fn embed_media(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(args.get("base64").and_then(Json::as_str).ok_or("missing file")?)
            .map_err(err)?;
        let cmd = xenify_ops::media::set_source(self.project(), id, MediaSource::Embedded { data: data.into() })
            .map_err(err)?;
        self.exec("Keep the clip in the file", cmd)?;
        Ok(Json::Null)
    }

    /// Replaces a layer as stored (for settings only the layer's own kind has: a sound's fades, speed, loop…).
    fn set_layer_json(&mut self, args: &Json) -> Result<Json, String> {
        let layer: xenify_doc::Layer =
            serde_json::from_value(args.get("layer").cloned().ok_or("missing layer")?).map_err(err)?;
        let key = format!("layer:{}", layer.id.0);
        let label = args.get("label").and_then(Json::as_str).unwrap_or("Change layer").to_string();
        self.editor.execute_merged(label, key, Command::ReplaceLayer { layer: Box::new(layer) }).map_err(err)?;
        Ok(Json::Null)
    }

    /// The sound from `start` to `end` seconds: 48 kHz stereo, interleaved (presentations: the deck's).
    pub fn mix(&self, start: f64, end: f64, deck: bool) -> Vec<f32> {
        if deck {
            xenify_audio::mix_deck(self.project(), start, end)
        } else {
            xenify_audio::mix_comp(self.project(), self.comp, start, end)
        }
    }
}
