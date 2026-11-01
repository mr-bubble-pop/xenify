//! Drawing animation frame by frame on raster layers: a new frame at the
//! playhead ends the raster layer showing there and starts another one (blank
//! or a copy), and the onion skin shows the frame before faintly while you
//! paint (on screen only).

use std::sync::Arc;

use serde_json::{Value as Json, json};
use xenify_anim::{Scene, SceneContent, SceneLayer};
use xenify_doc::{Id, ImageAsset, LayerKind};
use xenify_ops::Command;

use super::encode_png;
use crate::{Session, err, id_arg};

impl Session {
    pub(super) fn raster_frames_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "raster_new_frame" => self.raster_new_frame(args),
            "raster_onion" => {
                self.raster.onion = args.get("on").and_then(Json::as_bool).unwrap_or(!self.raster.onion);
                Ok(json!(self.raster.onion))
            }
            _ => return None,
        })
    }

    /// A new drawing frame at the playhead: raster layer `id` (showing now) ends here and a new one starts,
    /// blank or (`copy`) with its pixels, until where `id` ended.
    fn raster_new_frame(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let s = self.comp_settings().ok_or("no canvas")?;
        if s.duration <= 0.0 {
            return Err("A still picture has no frames: give the canvas a timeline first (Animate it).".into());
        }
        let half = 0.5 / s.fps.max(1.0);
        let t = (self.time * s.fps).round() / s.fps;
        let src = self.project().layer(id).map_err(err)?.clone();
        if t < src.meta.in_point + half || src.meta.out_point.is_some_and(|o| t >= o - half) {
            return Err("Move the playhead to a later frame of this layer first (a new frame starts there).".into());
        }
        let b = self.buf(id)?;
        let (w, h) = (b.w, b.h);
        let copy = args.get("copy").and_then(Json::as_bool).unwrap_or(false);
        let px: Arc<Vec<u8>> = if copy { b.px.clone() } else { Arc::new(vec![0; (w * h * 4) as usize]) };
        let png = encode_png(w, h, &px)?;
        // Frames are numbered on from the highest “Frame N”; the first drawing becomes “Frame 1”.
        let number = |name: &str| name.strip_prefix("Frame ").and_then(|n| n.trim().parse::<u32>().ok());
        let n = self
            .project()
            .compositions
            .iter()
            .flat_map(|c| &c.layers)
            .filter_map(|l| number(&l.meta.name))
            .max()
            .unwrap_or(1)
            + 1;
        let name = self.unique_name(&format!("Frame {n}"));
        let mut p = self.project().clone();
        let asset = ImageAsset::new(p.alloc_id(), name.clone(), "image/png", w, h, png);
        let mut layer = src.clone();
        layer.id = p.alloc_id();
        layer.meta.name = name;
        layer.meta.in_point = t;
        layer.kind = LayerKind::Image { asset: asset.id, playback: Default::default() };
        let new = layer.id;
        let mut ended = src.meta.clone();
        ended.out_point = Some(t);
        if number(&ended.name).is_none() {
            ended.name = self.unique_name("Frame 1");
        }
        let comp = self.comp;
        let index = self.project().comp(comp).ok().and_then(|c| c.layer_index(id)).map(|i| i + 1);
        self.exec_all(
            "New drawing frame",
            vec![
                Command::SetLayerMeta { layer: id, meta: ended },
                Command::AddAsset { asset },
                Command::AddLayer { comp, index, layer },
            ],
        )?;
        self.selection = vec![new];
        Ok(json!(new.0))
    }

    /// Onion skin (screen only): the frame before the selected raster frame, faint, just under it.
    pub(crate) fn raster_onion_skin(&self, scene: &mut Scene) {
        if !self.raster.onion || self.selection.len() != 1 {
            return;
        }
        let id = self.selection[0];
        let Ok(c) = self.project().comp(self.comp) else { return };
        let Some(now) = c.layer(id) else { return };
        let fps = c.settings.fps.max(1.0);
        let start = now.meta.in_point;
        let Some(prev) = c.layers.iter().find(|l| {
            l.id != id
                && l.meta.out_point.is_some_and(|o| (o - start).abs() < 0.5 / fps)
                && matches!(l.kind, LayerKind::Image { .. })
        }) else {
            return;
        };
        let before = xenify_anim::evaluate_project(self.project(), self.comp, start - 0.5 / fps);
        let Some(mut ghost) = before.layer(prev.id).cloned() else { return };
        ghost.opacity *= 0.3;
        insert_below(&mut scene.layers, id, ghost);
    }
}

/// Puts `l` just under the layer `target` (wherever it is in groups); false when `target` isn't there.
fn insert_below(layers: &mut Vec<SceneLayer>, target: Id, l: SceneLayer) -> bool {
    if let Some(i) = layers.iter().position(|x| x.id == target && x.copy == 0) {
        layers.insert(i, l);
        return true;
    }
    let mut l = Some(l);
    for x in layers.iter_mut() {
        if let SceneContent::Group(inner) = &mut x.content
            && inner.iter().any(|y| contains(y, target))
        {
            return insert_below(inner, target, l.take().expect("once"));
        }
    }
    false
}

fn contains(l: &SceneLayer, target: Id) -> bool {
    l.id == target || matches!(&l.content, SceneContent::Group(inner) if inner.iter().any(|y| contains(y, target)))
}
