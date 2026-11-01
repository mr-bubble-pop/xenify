//! Video and audio layers: adding a clip (fitted to the frame and trimmed
//! to its length), Fit and Fill, playback settings, and keeping a clip in
//! the project or linked to its file.

use crate::{Command, OpError, invalid};
use xenify_doc::media::play_length;
use xenify_doc::{AudioLayer, Id, Layer, LayerKind, MediaAsset, MediaKind, Project, PropPath, Value, Vec2, VideoLayer};

/// A clip's picture placed on a `w`×`h` canvas: centred, scaled to fit
/// (`fill`: to cover). Returns (anchor, position, scale).
fn placement(media: &MediaAsset, w: f64, h: f64, fill: bool) -> (Vec2, Vec2, f64) {
    let (mw, mh) = (media.width.max(1) as f64, media.height.max(1) as f64);
    let k = if fill { (w / mw).max(h / mh) } else { (w / mw).min(h / mh) };
    (Vec2::new(mw / 2.0, mh / 2.0), Vec2::new(w / 2.0, h / 2.0), k)
}

/// Adds `media` (unless the project has the same file already) and a layer
/// playing it from `time`: a video fitted to the frame, or an audio layer.
/// The layer is `layer_id`; the clip takes `media.id` when it's new.
pub fn add_clip(project: &Project, comp: Id, media: MediaAsset, layer_id: Id, time: f64) -> Result<Command, OpError> {
    let c = project.comp(comp)?;
    let (w, h) = (c.settings.width as f64, c.settings.height as f64);
    let existing = project.media.iter().find(|m| m.hash == media.hash && m.kind == media.kind).cloned();
    let asset = existing.clone().unwrap_or(media);
    let mut commands = Vec::new();
    if existing.is_none() {
        commands.push(Command::AddMedia { media: asset.clone() });
    }
    let name = asset.name.clone();
    let mut layer = match asset.kind {
        MediaKind::Video => {
            let (anchor, position, k) = placement(&asset, w, h, false);
            let mut l = Layer::new(layer_id, name, LayerKind::Video(Box::new(VideoLayer::new(asset.id))));
            l.transform.anchor = anchor.into();
            l.transform.position = position.into();
            l.transform.scale = Vec2::new(k, k).into();
            l
        }
        MediaKind::Audio => Layer::new(layer_id, name, LayerKind::Audio(Box::new(AudioLayer::new(asset.id)))),
    };
    layer.meta.in_point = time.max(0.0);
    let end = time + play_length(asset.duration, 0.0, 1.0);
    if asset.duration > 0.0 && end < c.settings.duration {
        layer.meta.out_point = Some(end);
    }
    commands.push(Command::AddLayer { comp, index: None, layer });
    Ok(Command::batch(commands))
}

/// Scales and centres a video to fit inside the frame, or to fill it.
pub fn fit(project: &Project, layer: Id, time: f64, fill: bool, auto_key: bool) -> Result<Command, OpError> {
    let (comp, _) = project.locate_layer(layer)?;
    let c = project.comp(comp)?;
    let l = project.layer(layer)?;
    let LayerKind::Video(v) = &l.kind else { return Err(invalid("only videos fit the frame")) };
    let media = project.media(v.asset).ok_or_else(|| invalid("the clip is missing"))?;
    let (anchor, position, k) = placement(media, c.settings.width as f64, c.settings.height as f64, fill);
    let edit = |path, value| Command::edit_value(project, layer, path, time, value, auto_key);
    Ok(Command::batch(vec![
        edit(PropPath::Anchor, Value::Vec2(anchor))?,
        edit(PropPath::Position, Value::Vec2(position))?,
        edit(PropPath::Scale, Value::Vec2(Vec2::new(k, k)))?,
    ]))
}

/// Changes a video layer's playback.
pub fn edit_video(project: &Project, layer: Id, f: impl FnOnce(&mut VideoLayer)) -> Result<Command, OpError> {
    let LayerKind::Video(v) = &project.layer(layer)?.kind else {
        return Err(invalid(format!("layer {layer} is not a video")));
    };
    let mut v = v.clone();
    f(&mut v);
    Ok(Command::SetVideo { layer, video: v })
}

/// Changes an audio layer's playback.
pub fn edit_audio(project: &Project, layer: Id, f: impl FnOnce(&mut AudioLayer)) -> Result<Command, OpError> {
    let LayerKind::Audio(a) = &project.layer(layer)?.kind else {
        return Err(invalid(format!("layer {layer} is not an audio layer")));
    };
    let mut a = a.clone();
    f(&mut a);
    Ok(Command::SetAudio { layer, audio: a })
}

/// Makes the layer as long as its clip plays (from its in point), keeping
/// within the canvas.
pub fn fit_length(project: &Project, layer: Id) -> Result<Command, OpError> {
    let (comp, _) = project.locate_layer(layer)?;
    let c = project.comp(comp)?;
    let l = project.layer(layer)?;
    let (asset, source_in, speed, looped) = match &l.kind {
        LayerKind::Video(v) => (v.asset, v.source_in, v.speed, v.looped),
        LayerKind::Audio(a) => (a.asset, a.source_in, a.speed, a.looped),
        _ => return Err(invalid("only video and audio layers have a clip length")),
    };
    let media = project.media(asset).ok_or_else(|| invalid("the clip is missing"))?;
    let mut meta = l.meta.clone();
    let end = l.meta.in_point + play_length(media.duration, source_in, speed);
    meta.out_point = (!looped && end < c.settings.duration).then_some(end);
    Ok(Command::SetLayerMeta { layer, meta })
}

/// Adds `media` (unless the project has the same sound already) to the
/// presentation's soundtrack, starting `start` seconds in.
pub fn add_to_soundtrack(project: &Project, media: MediaAsset, start: f64) -> Command {
    let existing = project.media.iter().find(|m| m.hash == media.hash).cloned();
    let asset = existing.clone().unwrap_or(media);
    let mut commands = Vec::new();
    if existing.is_none() {
        commands.push(Command::AddMedia { media: asset.clone() });
    }
    let mut soundtrack = project.soundtrack.clone();
    soundtrack.clips.push(xenify_doc::SoundtrackClip { start: start.max(0.0), audio: AudioLayer::new(asset.id) });
    commands.push(Command::SetSoundtrack { soundtrack });
    Command::batch(commands)
}

/// Changes the soundtrack.
pub fn edit_soundtrack(project: &Project, f: impl FnOnce(&mut xenify_doc::Soundtrack)) -> Command {
    let mut s = project.soundtrack.clone();
    f(&mut s);
    Command::SetSoundtrack { soundtrack: s }
}

/// Keeps clip `media` in the project file (`data`: its bytes) or links it
/// to the file at `path` instead.
pub fn set_source(project: &Project, media: Id, source: xenify_doc::MediaSource) -> Result<Command, OpError> {
    let mut m = project.media(media).ok_or_else(|| invalid(format!("no clip {media}")))?.clone();
    m.source = source;
    Ok(Command::SetMediaAsset { media: m })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::MediaSource;

    fn clip(id: u64, kind: MediaKind) -> MediaAsset {
        MediaAsset {
            id: Id(id),
            name: "clip".into(),
            kind,
            mime: "video/mp4".into(),
            source: MediaSource::Linked { path: "/tmp/clip.mp4".into() },
            duration: 2.0,
            width: 960,
            height: 540,
            fps: 25.0,
            has_audio: true,
            hash: 99,
            recipe: None,
        }
    }

    #[test]
    fn a_clip_is_fitted_trimmed_and_shared() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let cmd = add_clip(&p, comp, clip(50, MediaKind::Video), Id(51), 1.0).unwrap();
        let undo = apply(&mut p, cmd).unwrap();
        let l = p.layer(Id(51)).unwrap().clone();
        assert_eq!(l.transform.scale.value_at(0.0), Vec2::new(2.0, 2.0), "960×540 fits 1920×1080 at 200%");
        assert_eq!((l.meta.in_point, l.meta.out_point), (1.0, Some(3.0)));
        // The same file again: one clip, two layers.
        let cmd = add_clip(&p, comp, clip(60, MediaKind::Video), Id(61), 0.0).unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.media.len(), 1);
        // Saved: format 3 with the clip.
        let json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        assert_eq!(json["format_version"], 3);
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.media, p.media);
        apply(&mut p, Command::RemoveLayer { layer: Id(61) }).unwrap();
        apply(&mut p, undo).unwrap();
        assert!(p.media.is_empty(), "undo takes the clip away with its layer");
    }

    #[test]
    fn the_soundtrack_adds_clips_and_undoes() {
        let mut p = Project::new();
        let mut music = clip(70, MediaKind::Audio);
        music.name = "music".into();
        let cmd = add_to_soundtrack(&p, music, 0.0);
        let undo = apply(&mut p, cmd).unwrap();
        assert_eq!(p.soundtrack.clips.len(), 1);
        let cmd = edit_soundtrack(&p, |s| s.keep_playing = true);
        apply(&mut p, cmd).unwrap();
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.soundtrack, p.soundtrack);
        assert_eq!(back.media.len(), 1, "the soundtrack keeps its clip in the file");
        let cmd = edit_soundtrack(&p, |s| s.keep_playing = false);
        apply(&mut p, cmd).unwrap();
        apply(&mut p, undo).unwrap();
        assert!(p.soundtrack.is_empty() && p.media.is_empty());
    }

    #[test]
    fn playback_settings_and_fill_are_undoable() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let cmd = add_clip(&p, comp, clip(50, MediaKind::Video), Id(51), 0.0).unwrap();
        apply(&mut p, cmd).unwrap();
        let cmd = edit_video(&p, Id(51), |v| {
            v.speed = 2.0;
            v.looped = true;
        })
        .unwrap();
        let undo = apply(&mut p, cmd).unwrap();
        let cmd = fit(&p, Id(51), 0.0, true, false).unwrap();
        apply(&mut p, cmd).unwrap();
        let LayerKind::Video(v) = &p.layer(Id(51)).unwrap().kind else { unreachable!() };
        assert_eq!((v.speed, v.looped), (2.0, true));
        apply(&mut p, undo).unwrap();
        let LayerKind::Video(v) = &p.layer(Id(51)).unwrap().kind else { unreachable!() };
        assert_eq!(v.speed, 1.0);
        let cmd = set_source(&p, Id(50), MediaSource::Embedded { data: vec![1, 2].into() }).unwrap();
        apply(&mut p, cmd).unwrap();
        assert!(p.media(Id(50)).unwrap().is_embedded());
    }
}
