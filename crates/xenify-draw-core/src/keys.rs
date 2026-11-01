//! Keyframes in bulk: every key of some layers, keying all properties at the
//! playhead, easing and moving many keys at once, copying and pasting keys,
//! sliding and trimming layers in time, and pause markers.

use serde_json::{Value as Json, json};
use xenify_doc::{Easing, Id, PropPath, Value};
use xenify_ops::Command;

use crate::{Session, err, f, id_arg, prop_names, prop_path, static_property};

/// A keyframe named by the page: layer, property name, time.
struct Key {
    layer: Id,
    path: PropPath,
    time: f64,
}

fn keys_arg(args: &Json) -> Vec<Key> {
    args.get("keys")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|k| {
                    Some(Key {
                        layer: Id(k.get("id")?.as_u64()?),
                        path: prop_path(k.get("prop")?.as_str()?)?,
                        time: k.get("time")?.as_f64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn ids_arg(args: &Json) -> Option<Vec<Id>> {
    args.get("ids").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_u64).map(Id).collect())
}

impl Session {
    pub(crate) fn keys_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "all_keys" => Ok(self.all_keys(ids_arg(args))),
            "key_all" => self.key_all(args),
            "set_easings" => self.set_easings(args),
            "move_keys" => self.move_keys(args),
            "remove_keys" => {
                let cmds = keys_arg(args)
                    .into_iter()
                    .map(|k| Command::RemoveKeyframe { layer: k.layer, path: k.path, time: k.time })
                    .collect();
                self.exec_all("Remove keyframes", cmds).map(|_| Json::Null)
            }
            "copy_keys" => Ok(self.copy_keys(args)),
            "paste_keys" => self.paste_keys(args),
            "track_all" => self.track_all(id_arg(args, "id").ok()),
            "shift_layer" => {
                let layer = match id_arg(args, "id") {
                    Ok(l) => l,
                    Err(e) => return Some(Err(e)),
                };
                let delta = f(args, "delta").unwrap_or(0.0);
                match Command::shift_layer(self.project(), layer, delta).map_err(err) {
                    Ok(Some(cmd)) => self.merge_or_exec("Move layer in time", args, &format!("shift:{}", layer.0), cmd),
                    Ok(None) => Ok(Json::Null),
                    Err(e) => Err(e),
                }
            }
            "set_in_out" => self.set_in_out(args),
            "toggle_pause" => self.toggle_pause(args),
            "wave" => self.wave(args),
            "key_here" => self.key_here(args),
            _ => return None,
        })
    }

    fn merge_or_exec(&mut self, label: &str, args: &Json, key: &str, cmd: Command) -> Result<Json, String> {
        match args.get("drag").and_then(Json::as_u64) {
            Some(n) => self.editor.execute_merged(label, format!("{key}:{n}"), cmd).map_err(err)?,
            None => self.exec(label, cmd)?,
        }
        Ok(Json::Null)
    }

    /// Every keyframe of `ids` (or of every layer on this canvas): { id, prop, time }.
    fn all_keys(&self, ids: Option<Vec<Id>>) -> Json {
        let Ok(c) = self.project().comp(self.comp) else { return json!([]) };
        let mut out = Vec::new();
        for l in c.layers.iter().filter(|l| ids.as_ref().is_none_or(|ids| ids.contains(&l.id))) {
            for (name, path) in prop_names(l) {
                for t in l.keyframe_times(path).unwrap_or_default() {
                    out.push(json!({ "id": l.id.0, "prop": name, "time": t }));
                }
            }
        }
        json!(out)
    }

    /// Keys the properties of `ids` at the playhead: all of them (`all`), or only the animated ones.
    fn key_all(&mut self, args: &Json) -> Result<Json, String> {
        let ids = ids_arg(args).unwrap_or_else(|| self.selection.clone());
        let all = args.get("all").and_then(Json::as_bool).unwrap_or(true);
        match Command::key_all(self.project(), &ids, self.time, all).map_err(err)? {
            Some(cmd) => {
                self.exec(if all { "Key all properties" } else { "Key animated properties" }, cmd)?;
                Ok(json!(true))
            }
            None => Ok(json!(false)),
        }
    }

    /// One easing (a preset name or as stored) on every key in `keys`.
    fn set_easings(&mut self, args: &Json) -> Result<Json, String> {
        let easing = crate::anim::easing_arg(args.get("easing").ok_or("missing easing")?)?;
        let cmds: Vec<Command> = keys_arg(args)
            .into_iter()
            .map(|k| Command::SetEasing { layer: k.layer, path: k.path, time: k.time, easing })
            .collect();
        if let Some(n) = args.get("drag").and_then(Json::as_u64) {
            self.editor.execute_merged("Set easing", format!("easings:{n}"), Command::batch(cmds)).map_err(err)?;
        } else {
            self.exec_all("Set easing", cmds)?;
        }
        Ok(Json::Null)
    }

    /// Moves every key in `keys` by `delta` seconds (none before 0 s).
    fn move_keys(&mut self, args: &Json) -> Result<Json, String> {
        let keys = keys_arg(args);
        let earliest = keys.iter().map(|k| k.time).fold(f64::INFINITY, f64::min);
        let delta = f(args, "delta").unwrap_or(0.0).max(-earliest);
        let list: Vec<(Id, PropPath, f64)> = keys.iter().map(|k| (k.layer, k.path, k.time)).collect();
        if let Some(cmd) = Command::move_keyframes(&list, delta) {
            self.exec("Move keyframes", cmd)?;
        }
        Ok(json!(delta))
    }

    /// The keys as a clip: each one's property, time from the first, value and easing.
    fn copy_keys(&self, args: &Json) -> Json {
        let keys = keys_arg(args);
        let t0 = keys.iter().map(|k| k.time).fold(f64::INFINITY, f64::min);
        let out: Vec<Json> = keys
            .iter()
            .filter_map(|k| {
                let l = self.project().layer(k.layer).ok()?;
                let prop = l.property(k.path).ok()?;
                let name = prop_names(l).into_iter().find(|(_, p)| *p == k.path)?.0;
                Some(json!({
                    "prop": name, "time": k.time - t0,
                    "value": l.value_at(k.path, k.time).ok()?,
                    "easing": prop.easing_at(k.time).unwrap_or_default(),
                }))
            })
            .collect();
        json!(out)
    }

    /// Pastes a clip of keys onto layer `id` (or the selected one) from the playhead.
    fn paste_keys(&mut self, args: &Json) -> Result<Json, String> {
        let target = id_arg(args, "id")
            .ok()
            .or_else(|| self.selection.first().copied())
            .ok_or("Select a layer to paste keyframes onto.")?;
        let layer = self.project().layer(target).map_err(err)?.clone();
        let t0 = self.time;
        let (mut cmds, mut skipped) = (Vec::new(), 0);
        for k in args.get("clip").and_then(Json::as_array).cloned().unwrap_or_default() {
            let path = k.get("prop").and_then(Json::as_str).and_then(prop_path);
            let value: Option<Value> = k.get("value").and_then(|v| serde_json::from_value(v.clone()).ok());
            let easing: Easing =
                k.get("easing").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
            match (path, value) {
                // The property must exist on this layer with the same kind of value.
                (Some(path), Some(value)) if layer.property_kind(path).is_ok_and(|kind| kind == value.kind()) => {
                    let time = t0 + k.get("time").and_then(Json::as_f64).unwrap_or(0.0);
                    cmds.push(Command::SetKeyframe { layer: target, path, time, value, easing: Some(easing) });
                }
                _ => skipped += 1,
            }
        }
        let n = cmds.len();
        if n == 0 {
            return Err("Those keyframes don’t fit this layer’s properties.".into());
        }
        self.exec("Paste keyframes", Command::batch(cmds))?;
        Ok(json!({ "pasted": n, "skipped": skipped }))
    }

    /// The layer row's stopwatch: animate every property (a key at the
    /// playhead on each that isn't), or, when all are, stop animating it.
    fn track_all(&mut self, id: Option<Id>) -> Result<Json, String> {
        let id = id.or_else(|| self.selection.first().copied()).ok_or("no layer")?;
        let l = self.project().layer(id).map_err(err)?;
        let paths = l.property_paths();
        let all = !paths.is_empty() && paths.iter().all(|p| l.property(*p).is_ok_and(|p| p.is_animated()));
        let time = self.time;
        let cmds: Vec<Command> = paths
            .into_iter()
            .filter_map(|path| {
                let prop = l.property(path).ok()?;
                let value = l.value_at(path, time).ok()?;
                match (all, prop.is_animated()) {
                    (true, _) => Some(Command::SetProperty { layer: id, path, property: static_property(value) }),
                    (false, false) => Some(Command::SetKeyframe { layer: id, path, time, value, easing: None }),
                    (false, true) => None,
                }
            })
            .collect();
        self.exec_all(if all { "Stop animating layer" } else { "Track all properties" }, cmds)?;
        Ok(json!(!all))
    }

    /// Trims a layer: when it starts (`in`) and ends (`out`; null: the canvas's end).
    fn set_in_out(&mut self, args: &Json) -> Result<Json, String> {
        let layer = id_arg(args, "id")?;
        let mut meta = self.project().layer(layer).map_err(err)?.meta.clone();
        let fps = self.comp_settings().map_or(30.0, |s| s.fps);
        let frame = 1.0 / fps.max(1.0);
        if let Some(t) = f(args, "in") {
            meta.in_point = t.max(0.0).min(meta.out_point.map_or(f64::MAX, |o| o - frame));
        }
        match args.get("out") {
            Some(Json::Null) => meta.out_point = None,
            Some(v) => {
                if let Some(t) = v.as_f64() {
                    meta.out_point = Some(t.max(meta.in_point + frame));
                }
            }
            None => {}
        }
        self.merge_or_exec("Trim layer", args, &format!("trim:{}", layer.0), Command::SetLayerMeta { layer, meta })
    }

    /// A key on property `prop` at the playhead holding its value now, or (`remove`) none there.
    fn key_here(&mut self, args: &Json) -> Result<Json, String> {
        let layer = id_arg(args, "id")?;
        let name = args.get("prop").and_then(Json::as_str).unwrap_or("");
        let path = prop_path(name).ok_or_else(|| format!("no property {name}"))?;
        let time = self.time;
        let l = self.project().layer(layer).map_err(err)?;
        let here = l.keyframe_times(path).unwrap_or_default().into_iter().find(|t| (t - time).abs() < 1e-6);
        match here {
            Some(t) => {
                let only = l.keyframe_times(path).unwrap_or_default().len() == 1;
                let value = l.value_at(path, time).map_err(err)?;
                // The last key going: the property keeps its value, not animated any more.
                let cmd = if only {
                    Command::SetProperty { layer, path, property: static_property(value) }
                } else {
                    Command::RemoveKeyframe { layer, path, time: t }
                };
                self.exec("Remove keyframe", cmd)?;
                Ok(json!(false))
            }
            None => {
                let value = l.value_at(path, time).map_err(err)?;
                self.exec("Add keyframe", Command::SetKeyframe { layer, path, time, value, easing: None })?;
                Ok(json!(true))
            }
        }
    }

    /// A sound or video layer's waveform: `bins` peaks over its clip, and how the clip plays.
    fn wave(&self, args: &Json) -> Result<Json, String> {
        let l = self.project().layer(id_arg(args, "id")?).map_err(err)?;
        let (asset, source_in, speed, looped) = match &l.kind {
            xenify_doc::LayerKind::Audio(a) => (a.asset, a.source_in, a.speed, a.looped),
            xenify_doc::LayerKind::Video(v) => (v.asset, v.source_in, v.speed, v.looped),
            _ => return Ok(Json::Null),
        };
        let Some(media) = self.project().media(asset) else { return Ok(Json::Null) };
        if media.is_video() && !media.has_audio {
            return Ok(Json::Null);
        }
        let Some(pcm) = xenify_audio::pcm(&xenify_anim::media::MediaRef::of(media)) else { return Ok(Json::Null) };
        let bins = args.get("bins").and_then(Json::as_u64).unwrap_or(600).clamp(16, 4000) as usize;
        Ok(
            json!({ "peaks": xenify_audio::peaks(&pcm, bins), "duration": media.duration, "source_in": source_in, "speed": speed, "looped": looped }),
        )
    }

    /// Adds a pause marker at `time` (the playhead), or removes the one there.
    fn toggle_pause(&mut self, args: &Json) -> Result<Json, String> {
        let mut s = self.comp_settings().cloned().ok_or("no canvas")?;
        let fps = s.fps.max(1.0);
        let t = (f(args, "time").unwrap_or(self.time) * fps).round() / fps;
        match s.pauses.iter().position(|p| (p - t).abs() < 0.5 / fps) {
            Some(i) => {
                s.pauses.remove(i);
                self.exec("Remove pause", Command::SetCompSettings { comp: self.comp, settings: s })?;
                Ok(json!(false))
            }
            None => {
                if t <= 0.0 || t >= s.duration {
                    return Err("Pauses go between the start and the end: move the playhead.".into());
                }
                s.pauses.push(t);
                s.pauses.sort_by(f64::total_cmp);
                self.exec("Add pause", Command::SetCompSettings { comp: self.comp, settings: s })?;
                Ok(json!(true))
            }
        }
    }
}
