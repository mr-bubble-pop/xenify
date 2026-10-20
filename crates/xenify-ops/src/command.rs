use crate::{OpError, invalid};
use xenify_doc::{
    AnyProperty, CompSettings, Easing, Id, Layer, LayerKind, LayerMeta, Project, PropPath, ShapeItem, Value,
};
use serde::{Deserialize, Serialize};

/// A single, invertible edit. Serialises as `{"op": "set_keyframe", ...}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Inserts a layer at `index` (paint order; `None` = on top).
    AddLayer {
        comp: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        layer: Layer,
    },
    /// Removes a layer; layers parented to it are unparented.
    RemoveLayer {
        layer: Id,
    },
    /// Moves a layer to a new paint-order index within its composition.
    MoveLayer {
        layer: Id,
        index: usize,
    },
    SetLayerMeta {
        layer: Id,
        meta: LayerMeta,
    },
    /// Replaces a whole property (static value or keyframe list).
    SetProperty {
        layer: Id,
        path: PropPath,
        property: AnyProperty,
    },
    /// Adds or replaces a keyframe (turns a static property into an animated one).
    SetKeyframe {
        layer: Id,
        path: PropPath,
        time: f64,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        easing: Option<Easing>,
    },
    RemoveKeyframe {
        layer: Id,
        path: PropPath,
        time: f64,
    },
    MoveKeyframe {
        layer: Id,
        path: PropPath,
        from: f64,
        to: f64,
    },
    SetEasing {
        layer: Id,
        path: PropPath,
        time: f64,
        easing: Easing,
    },
    /// Edits a value the way a user does in the UI: keyframes it at `time` if
    /// the property is animated, otherwise changes the static value.
    SetValue {
        layer: Id,
        path: PropPath,
        time: f64,
        value: Value,
    },
    AddShape {
        layer: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        shape: ShapeItem,
    },
    RemoveShape {
        layer: Id,
        index: usize,
    },
    SetText {
        layer: Id,
        text: String,
    },
    /// Replaces a whole shape (geometry, fill incl. gradient, stroke, fill rule).
    SetShape {
        layer: Id,
        index: usize,
        shape: ShapeItem,
    },
    /// Font family of a text layer (e.g. `"Inter"`); missing fonts fall back to Inter.
    SetFontFamily {
        layer: Id,
        family: String,
    },
    /// Which point of a text layer sits on its position.
    SetTextAlign {
        layer: Id,
        align: xenify_doc::TextAlign,
        valign: xenify_doc::TextVAlign,
    },
    /// Letter / word spacing, line height and the path of a text layer.
    SetTextOptions {
        layer: Id,
        letter_spacing: f64,
        word_spacing: f64,
        line_height: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<xenify_doc::TextPath>,
    },
    /// Gives a text layer a box to wrap and align in (`None`: point text).
    SetTextFrame {
        layer: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<xenify_doc::TextFrame>,
    },
    /// Replaces a text layer's animators (letters/words/lines animating in).
    SetTextAnimators {
        layer: Id,
        animators: Vec<xenify_doc::TextAnimator>,
    },
    SetCompSettings {
        comp: Id,
        settings: CompSettings,
    },
    /// Adds a composition (with its layers) at `index` in the project's list.
    AddComposition {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        comp: xenify_doc::Composition,
    },
    /// Removes a composition that no precomp layer uses.
    RemoveComposition {
        comp: Id,
    },
    /// Adds an embedded image to the project (see [`crate::assets::add_image`]).
    AddAsset {
        asset: xenify_doc::ImageAsset,
    },
    /// Removes an image no layer shows.
    RemoveAsset {
        asset: Id,
    },
    /// Replaces the presentation's slide list (order, transitions, notes).
    SetSlides {
        slides: Vec<xenify_doc::Slide>,
    },
    /// Sets (or removes) a layer's live wiggle.
    SetWiggle {
        layer: Id,
        wiggle: Option<xenify_doc::Wiggle>,
    },
    /// Replaces a 3D solid layer's settings (shape, perspective and all its properties).
    SetSolid {
        layer: Id,
        solid: Box<xenify_doc::Solid>,
    },
    /// Adds a video or audio file to the project.
    AddMedia {
        media: xenify_doc::MediaAsset,
    },
    /// Removes a video or audio file no layer plays.
    RemoveMedia {
        media: Id,
    },
    /// Replaces the presentation's soundtrack.
    SetSoundtrack {
        soundtrack: xenify_doc::Soundtrack,
    },
    /// Replaces a video layer's playback settings.
    SetVideo {
        layer: Id,
        video: Box<xenify_doc::VideoLayer>,
    },
    /// Replaces an audio layer's playback settings.
    SetAudio {
        layer: Id,
        audio: Box<xenify_doc::AudioLayer>,
    },
    /// Changes how a media file is kept (embedded or linked) or its name.
    SetMediaAsset {
        media: xenify_doc::MediaAsset,
    },
    /// Replaces a liquid layer's settings.
    SetLiquid {
        layer: Id,
        liquid: Box<xenify_doc::LiquidFill>,
    },
    /// Replaces a particle layer's emitter settings.
    SetEmitter {
        layer: Id,
        emitter: Box<xenify_doc::Emitter>,
    },
    /// Replaces a camera layer's settings (zoom, shake).
    SetCamera {
        layer: Id,
        camera: Box<xenify_doc::CameraLayer>,
    },
    /// Sets (or removes) a layer's corner pin / perspective.
    SetDistort {
        layer: Id,
        distort: Option<xenify_doc::Distort>,
    },
    /// When a precomp layer's composition starts, in the parent's time.
    SetPrecompStart {
        layer: Id,
        start: f64,
    },
    /// Replaces a whole layer (same id, same place in its composition).
    ReplaceLayer {
        layer: Box<Layer>,
    },
    /// The deck's theme name (used for new slides).
    SetTheme {
        theme: Option<String>,
    },
    /// How an image layer plays an animated image.
    SetImagePlayback {
        layer: Id,
        playback: xenify_doc::ImagePlayback,
    },
    /// Whether a precomp layer plays its composition over and over.
    SetPrecompLoop {
        layer: Id,
        looped: bool,
    },
    /// Applies all commands in order, atomically: if one fails, the earlier
    /// ones are rolled back.
    Batch {
        commands: Vec<Command>,
    },
}

impl Command {
    pub fn batch(commands: Vec<Command>) -> Self {
        Command::Batch { commands }
    }

    /// The command for an interactive edit of `path` to `value` at `time`.
    ///
    /// With `auto_key` off this is a plain [`Command::SetValue`] (keyframes
    /// only if the property is already animated). With it on, a static
    /// property starts animating: its current value is keyed at 0 s and the new
    /// value at `time`, so the change becomes motion from the old state.
    pub fn edit_value(
        project: &Project,
        layer: Id,
        path: PropPath,
        time: f64,
        value: Value,
        auto_key: bool,
    ) -> Result<Command, OpError> {
        let l = project.layer(layer)?;
        if !auto_key || l.property(path)?.is_animated() {
            return Ok(Command::SetValue { layer, path, time, value });
        }
        let old = l.value_at(path, 0.0)?;
        let mut commands = Vec::new();
        if time > xenify_doc::TIME_EPSILON {
            commands.push(Command::SetKeyframe { layer, path, time: 0.0, value: old, easing: None });
        }
        commands.push(Command::SetKeyframe { layer, path, time, value, easing: None });
        Ok(Command::batch(commands))
    }

    /// Keys the properties of `layers` at `time` with their current values as
    /// one command (After Effects' "add keyframe to all properties"). With
    /// `static_too` off only already-animated properties get a keyframe.
    /// Keyframes already at `time` keep their easing. Returns `None` if there
    /// is nothing to key.
    pub fn key_all(project: &Project, layers: &[Id], time: f64, static_too: bool) -> Result<Option<Command>, OpError> {
        let mut commands = Vec::new();
        for &layer in layers {
            let l = project.layer(layer)?;
            for path in l.property_paths() {
                if !static_too && !l.property(path)?.is_animated() {
                    continue;
                }
                let value = l.value_at(path, time)?;
                commands.push(Command::SetKeyframe { layer, path, time, value, easing: None });
            }
        }
        Ok((!commands.is_empty()).then(|| Command::batch(commands)))
    }

    /// Moves several keyframes by `delta` seconds as one command.
    ///
    /// Moves are ordered so keyframes in the set never land on each other
    /// mid-way, and `delta` is clamped so no keyframe goes before 0 s.
    /// Returns `None` if there is nothing to do.
    pub fn move_keyframes(keys: &[(Id, PropPath, f64)], delta: f64) -> Option<Command> {
        let min = keys.iter().map(|k| k.2).fold(f64::INFINITY, f64::min);
        let delta = delta.max(-min);
        if keys.is_empty() || delta.abs() < xenify_doc::TIME_EPSILON {
            return None;
        }
        let mut sorted = keys.to_vec();
        // Moving right: move the right-most first (and vice versa).
        sorted.sort_by(|a, b| if delta > 0.0 { b.2.total_cmp(&a.2) } else { a.2.total_cmp(&b.2) });
        Some(Command::batch(
            sorted
                .into_iter()
                .map(|(layer, path, from)| Command::MoveKeyframe { layer, path, from, to: from + delta })
                .collect(),
        ))
    }

    /// Slides a layer in time by `delta` seconds: its in/out points and every
    /// keyframe move together (clamped so nothing goes before 0 s).
    pub fn shift_layer(project: &Project, layer: Id, delta: f64) -> Result<Option<Command>, OpError> {
        let l = project.layer(layer)?;
        let keys: Vec<(Id, PropPath, f64)> = l
            .property_paths()
            .into_iter()
            .flat_map(|p| l.keyframe_times(p).unwrap_or_default().into_iter().map(move |t| (layer, p, t)))
            .collect();
        let earliest = keys.iter().map(|k| k.2).fold(l.meta.in_point, f64::min);
        let delta = delta.max(-earliest);
        if delta.abs() < xenify_doc::TIME_EPSILON {
            return Ok(None);
        }
        let meta = LayerMeta {
            in_point: l.meta.in_point + delta,
            out_point: l.meta.out_point.map(|o| o + delta),
            ..l.meta.clone()
        };
        let mut commands = vec![Command::SetLayerMeta { layer, meta }];
        if let LayerKind::Precomp { start, .. } = l.kind {
            commands.push(Command::SetPrecompStart { layer, start: start + delta });
        }
        commands.extend(Command::move_keyframes(&keys, delta));
        Ok(Some(Command::batch(commands)))
    }
}

/// Applies `cmd` to `project`, returning the command that undoes it.
/// On error the project is left unchanged.
pub fn apply(project: &mut Project, cmd: Command) -> Result<Command, OpError> {
    use Command::*;
    Ok(match cmd {
        AddLayer { comp, index, layer } => {
            if project.id_in_use(layer.id) {
                return Err(xenify_doc::ProjectError::DuplicateId(layer.id).into());
            }
            let comp_ref = project.comp(comp)?;
            if let Some(parent) = layer.meta.parent
                && comp_ref.layer(parent).is_none()
            {
                return Err(invalid(format!("parent {parent} is not in composition {comp}")));
            }
            if let Some(asset) =
                layer.asset_refs().into_iter().find(|a| project.asset(*a).is_none() && project.media(*a).is_none())
            {
                return Err(invalid(format!("image or media {asset} is not in the project")));
            }
            let len = comp_ref.layers.len();
            let index = index.unwrap_or(len);
            if index > len {
                return Err(invalid(format!("layer index {index} out of range (0..={len})")));
            }
            let id = layer.id;
            project.reserve_id(id);
            project.comp_mut(comp)?.layers.insert(index, layer);
            RemoveLayer { layer: id }
        }
        RemoveLayer { layer } => {
            let (comp_id, index) = project.locate_layer(layer)?;
            // Connector ends glued to it stay where it was.
            let glued = project
                .comp(comp_id)?
                .layers
                .iter()
                .any(|l| l.meta.connector.as_ref().is_some_and(|c| c.attached().any(|a| a == layer)));
            let centre = if glued { crate::diagram::centre_of(project, comp_id, layer) } else { None };
            let comp = project.comp_mut(comp_id)?;
            let removed = comp.layers.remove(index);
            let mut undo = vec![AddLayer { comp: comp_id, index: Some(index), layer: removed }];
            // Children are unparented and layers it masked lose their matte.
            for other in comp
                .layers
                .iter_mut()
                .filter(|l| l.meta.parent == Some(layer) || l.meta.matte.is_some_and(|m| m.layer == layer))
            {
                undo.push(SetLayerMeta { layer: other.id, meta: other.meta.clone() });
                if other.meta.parent == Some(layer) {
                    other.meta.parent = None;
                }
                if other.meta.matte.is_some_and(|m| m.layer == layer) {
                    other.meta.matte = None;
                }
            }
            if glued {
                undo.extend(crate::diagram::detach_from(comp, layer, centre));
            }
            if undo.len() == 1 { undo.pop().unwrap() } else { Batch { commands: undo } }
        }
        MoveLayer { layer, index } => {
            let (comp_id, from) = project.locate_layer(layer)?;
            let comp = project.comp_mut(comp_id)?;
            if index >= comp.layers.len() {
                return Err(invalid(format!("layer index {index} out of range")));
            }
            let l = comp.layers.remove(from);
            comp.layers.insert(index, l);
            MoveLayer { layer, index: from }
        }
        SetLayerMeta { layer, meta } => {
            validate_parent(project, layer, meta.parent)?;
            if let Some(m) = meta.matte {
                let (comp, _) = project.locate_layer(layer)?;
                if m.layer == layer || project.comp(comp)?.layer(m.layer).is_none() {
                    return Err(invalid(format!("matte layer {} is not another layer in this composition", m.layer)));
                }
            }
            if meta.out_point.is_some_and(|out| out <= meta.in_point) {
                return Err(invalid("out point must be after in point"));
            }
            if let Some(c) = &meta.connector {
                crate::diagram::validate(project, layer, c)?;
            }
            let old = std::mem::replace(&mut project.layer_mut(layer)?.meta, meta);
            SetLayerMeta { layer, meta: old }
        }
        SetProperty { layer, path, property } => {
            let old = project.layer_mut(layer)?.replace_property(path, property)?;
            SetProperty { layer, path, property: old }
        }
        SetKeyframe { layer, path, time, value, easing } => {
            check_time(time)?;
            edit_property(project, layer, path, |l| l.set_keyframe(path, time, value, easing).map_err(Into::into))?
        }
        RemoveKeyframe { layer, path, time } => edit_property(project, layer, path, |l| {
            if l.remove_keyframe(path, time)? {
                Ok(())
            } else {
                Err(invalid(format!("no keyframe on {path} at {time}s")))
            }
        })?,
        MoveKeyframe { layer, path, from, to } => {
            check_time(to)?;
            edit_any_property(project, layer, path, |p| {
                p.move_keyframe(from, to)
                    .then_some(())
                    .ok_or_else(|| invalid(format!("no keyframe on {path} at {from}s")))
            })?
        }
        SetEasing { layer, path, time, easing } => edit_any_property(project, layer, path, |p| {
            p.set_easing(time, easing).then_some(()).ok_or_else(|| invalid(format!("no keyframe on {path} at {time}s")))
        })?,
        SetValue { layer, path, time, value } => {
            check_time(time)?;
            edit_property(project, layer, path, |l| l.set_value(path, time, value).map_err(Into::into))?
        }
        AddShape { layer, index, shape } => {
            let shapes = shapes_mut(project, layer)?;
            let index = index.unwrap_or(shapes.len());
            if index > shapes.len() {
                return Err(invalid(format!("shape index {index} out of range")));
            }
            shapes.insert(index, shape);
            RemoveShape { layer, index }
        }
        RemoveShape { layer, index } => {
            let shapes = shapes_mut(project, layer)?;
            if index >= shapes.len() {
                return Err(invalid(format!("shape index {index} out of range")));
            }
            let shape = shapes.remove(index);
            AddShape { layer, index: Some(index), shape }
        }
        SetShape { layer, index, shape } => {
            let shapes = shapes_mut(project, layer)?;
            let slot = shapes.get_mut(index).ok_or_else(|| invalid(format!("shape index {index} out of range")))?;
            let old = std::mem::replace(slot, shape);
            SetShape { layer, index, shape: old }
        }
        SetText { layer, text } => {
            let LayerKind::Text(t) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a text layer")));
            };
            let old = std::mem::replace(&mut t.text, text);
            SetText { layer, text: old }
        }
        SetTextAlign { layer, align, valign } => {
            let LayerKind::Text(t) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a text layer")));
            };
            let old = SetTextAlign { layer, align: t.align, valign: t.valign };
            t.align = align;
            t.valign = valign;
            old
        }
        SetTextOptions { layer, letter_spacing, word_spacing, line_height, path } => {
            if let Some(p) = path
                && !matches!(project.layer(p.layer).map(|l| &l.kind), Ok(LayerKind::Shape { .. }))
            {
                return Err(invalid("text can only follow a shape layer's outline"));
            }
            let LayerKind::Text(t) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a text layer")));
            };
            let old = SetTextOptions {
                layer,
                letter_spacing: t.letter_spacing,
                word_spacing: t.word_spacing,
                line_height: t.line_height,
                path: t.path,
            };
            (t.letter_spacing, t.word_spacing, t.line_height, t.path) =
                (letter_spacing, word_spacing, line_height, path);
            old
        }
        SetTextAnimators { layer, animators } => {
            let LayerKind::Text(t) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a text layer")));
            };
            let old = std::mem::replace(&mut t.animators, animators);
            SetTextAnimators { layer, animators: old }
        }
        SetTextFrame { layer, frame } => {
            if let Some(f) = frame
                && !(f.rect.width() > 0.0 && f.rect.height() > 0.0 && f.rect.is_finite())
            {
                return Err(invalid("a text box needs a width and height above zero"));
            }
            let LayerKind::Text(t) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a text layer")));
            };
            let old = std::mem::replace(&mut t.frame, frame);
            SetTextFrame { layer, frame: old }
        }
        SetFontFamily { layer, family } => {
            let LayerKind::Text(t) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a text layer")));
            };
            let old = std::mem::replace(&mut t.font_family, family);
            SetFontFamily { layer, family: old }
        }
        SetCompSettings { comp, settings } => {
            if settings.width == 0 || settings.height == 0 || settings.width > 16384 || settings.height > 16384 {
                return Err(invalid("composition size must be between 1 and 16384 pixels"));
            }
            if !(settings.fps > 0.0 && settings.fps <= 240.0) {
                return Err(invalid("fps must be in (0, 240]"));
            }
            if !(settings.duration > 0.0 && settings.duration.is_finite()) {
                return Err(invalid("duration must be positive"));
            }
            let old = std::mem::replace(&mut project.comp_mut(comp)?.settings, settings);
            SetCompSettings { comp, settings: old }
        }
        AddComposition { index, comp } => {
            if let Some(id) =
                std::iter::once(comp.id).chain(comp.layers.iter().map(|l| l.id)).find(|id| project.id_in_use(*id))
            {
                return Err(xenify_doc::ProjectError::DuplicateId(id).into());
            }
            let len = project.compositions.len();
            let index = index.unwrap_or(len);
            if index > len {
                return Err(invalid(format!("composition index {index} out of range (0..={len})")));
            }
            let id = comp.id;
            project.reserve_id(id);
            for l in &comp.layers {
                project.reserve_id(l.id);
            }
            project.compositions.insert(index, comp);
            RemoveComposition { comp: id }
        }
        SetSlides { slides } => {
            if let Some(s) = slides.iter().find(|s| project.comp(s.comp).is_err()) {
                return Err(xenify_doc::ProjectError::NoSuchComposition(s.comp).into());
            }
            SetSlides { slides: std::mem::replace(&mut project.slides, slides) }
        }
        AddAsset { asset } => {
            if project.id_in_use(asset.id) {
                return Err(xenify_doc::ProjectError::DuplicateId(asset.id).into());
            }
            let id = asset.id;
            project.reserve_id(id);
            project.assets.push(asset);
            RemoveAsset { asset: id }
        }
        RemoveAsset { asset } => {
            if project.asset_in_use(asset) {
                return Err(invalid("that image is used by a layer"));
            }
            let index = project
                .assets
                .iter()
                .position(|a| a.id == asset)
                .ok_or_else(|| invalid(format!("no image {asset}")))?;
            AddAsset { asset: project.assets.remove(index) }
        }
        SetWiggle { layer, wiggle } => {
            let old = std::mem::replace(&mut project.layer_mut(layer)?.transform.wiggle, wiggle);
            SetWiggle { layer, wiggle: old }
        }
        SetSolid { layer, solid } => {
            let LayerKind::Solid(current) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a 3D solid")));
            };
            let old = std::mem::replace(current, solid);
            SetSolid { layer, solid: old }
        }
        AddMedia { media } => {
            if project.id_in_use(media.id) {
                return Err(xenify_doc::ProjectError::DuplicateId(media.id).into());
            }
            let id = media.id;
            project.reserve_id(id);
            project.media.push(media);
            RemoveMedia { media: id }
        }
        RemoveMedia { media } => {
            if project.asset_in_use(media) {
                return Err(invalid("that clip is used by a layer"));
            }
            let index =
                project.media.iter().position(|m| m.id == media).ok_or_else(|| invalid(format!("no clip {media}")))?;
            AddMedia { media: project.media.remove(index) }
        }
        SetMediaAsset { media } => {
            let current = project
                .media
                .iter_mut()
                .find(|m| m.id == media.id)
                .ok_or_else(|| invalid(format!("no clip {}", media.id)))?;
            SetMediaAsset { media: std::mem::replace(current, media) }
        }
        SetSoundtrack { soundtrack } => {
            if let Some(c) = soundtrack.clips.iter().find(|c| project.media(c.audio.asset).is_none()) {
                return Err(invalid(format!("clip {} is not in the project", c.audio.asset)));
            }
            SetSoundtrack { soundtrack: std::mem::replace(&mut project.soundtrack, soundtrack) }
        }
        SetVideo { layer, video } => {
            let LayerKind::Video(current) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a video")));
            };
            let old = std::mem::replace(current, video);
            SetVideo { layer, video: old }
        }
        SetAudio { layer, audio } => {
            let LayerKind::Audio(current) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not an audio layer")));
            };
            let old = std::mem::replace(current, audio);
            SetAudio { layer, audio: old }
        }
        SetLiquid { layer, liquid } => {
            let LayerKind::Liquid(current) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a liquid layer")));
            };
            let old = std::mem::replace(current, liquid);
            SetLiquid { layer, liquid: old }
        }
        SetEmitter { layer, emitter } => {
            let LayerKind::Particles(current) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a particle layer")));
            };
            let old = std::mem::replace(current, emitter);
            SetEmitter { layer, emitter: old }
        }
        SetCamera { layer, camera } => {
            let LayerKind::Camera(current) = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a camera")));
            };
            let old = Box::new(std::mem::replace(current, *camera));
            SetCamera { layer, camera: old }
        }
        SetDistort { layer, distort } => {
            let old = std::mem::replace(&mut project.layer_mut(layer)?.transform.distort, distort);
            SetDistort { layer, distort: old }
        }
        SetPrecompStart { layer, start } => {
            let LayerKind::Precomp { start: current, .. } = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a precomp")));
            };
            let old = std::mem::replace(current, start);
            SetPrecompStart { layer, start: old }
        }
        ReplaceLayer { layer } => {
            let current = project.layer_mut(layer.id)?;
            let old = std::mem::replace(current, *layer);
            ReplaceLayer { layer: Box::new(old) }
        }
        SetTheme { theme } => {
            let old = std::mem::replace(&mut project.theme, theme);
            SetTheme { theme: old }
        }
        SetImagePlayback { layer, playback } => {
            let LayerKind::Image { playback: current, .. } = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not an image")));
            };
            let old = std::mem::replace(current, playback);
            SetImagePlayback { layer, playback: old }
        }
        SetPrecompLoop { layer, looped } => {
            let LayerKind::Precomp { looped: current, .. } = &mut project.layer_mut(layer)?.kind else {
                return Err(invalid(format!("layer {layer} is not a precomp")));
            };
            let old = std::mem::replace(current, looped);
            SetPrecompLoop { layer, looped: old }
        }
        RemoveComposition { comp } => {
            let used = project
                .compositions
                .iter()
                .flat_map(|c| &c.layers)
                .any(|l| matches!(l.kind, LayerKind::Precomp { comp: c, .. } if c == comp));
            if used {
                return Err(invalid("that composition is used by a precomp layer"));
            }
            if project.slide_index(comp).is_some() {
                return Err(invalid("that composition is a slide; delete the slide instead"));
            }
            if project.compositions.len() == 1 {
                return Err(invalid("a project needs at least one composition"));
            }
            let index = project
                .compositions
                .iter()
                .position(|c| c.id == comp)
                .ok_or(xenify_doc::ProjectError::NoSuchComposition(comp))?;
            let removed = project.compositions.remove(index);
            AddComposition { index: Some(index), comp: removed }
        }
        Batch { commands } => {
            let mut undo = Vec::with_capacity(commands.len());
            for c in commands {
                match apply(project, c) {
                    Ok(inv) => undo.push(inv),
                    Err(e) => {
                        for inv in undo.into_iter().rev() {
                            apply(project, inv).expect("inverse of an applied command must apply");
                        }
                        return Err(e);
                    }
                }
            }
            undo.reverse();
            Batch { commands: undo }
        }
    })
}

fn check_time(time: f64) -> Result<(), OpError> {
    if time.is_finite() && time >= 0.0 {
        Ok(())
    } else {
        Err(invalid(format!("time must be a non-negative number of seconds, got {time}")))
    }
}

/// Runs `edit` on a layer and returns a `SetProperty` restoring the previous
/// state of `path`. Rolls back if `edit` fails.
fn edit_property(
    project: &mut Project,
    layer: Id,
    path: PropPath,
    edit: impl FnOnce(&mut Layer) -> Result<(), OpError>,
) -> Result<Command, OpError> {
    let l = project.layer_mut(layer)?;
    let old = l.property(path)?;
    if let Err(e) = edit(l) {
        l.replace_property(path, old)?;
        return Err(e);
    }
    Ok(Command::SetProperty { layer, path, property: old })
}

fn edit_any_property(
    project: &mut Project,
    layer: Id,
    path: PropPath,
    edit: impl FnOnce(&mut AnyProperty) -> Result<(), OpError>,
) -> Result<Command, OpError> {
    let l = project.layer_mut(layer)?;
    let old = l.property(path)?;
    let mut new = old.clone();
    edit(&mut new)?;
    l.replace_property(path, new)?;
    Ok(Command::SetProperty { layer, path, property: old })
}

fn shapes_mut(project: &mut Project, layer: Id) -> Result<&mut Vec<ShapeItem>, OpError> {
    project.layer_mut(layer)?.shapes_mut().ok_or_else(|| invalid(format!("layer {layer} is not a shape layer")))
}

/// A parent must be another layer in the same composition and must not create
/// a cycle.
fn validate_parent(project: &Project, layer: Id, parent: Option<Id>) -> Result<(), OpError> {
    let Some(parent) = parent else { return Ok(()) };
    let (comp_id, _) = project.locate_layer(layer)?;
    let comp = project.comp(comp_id)?;
    let mut cursor = Some(parent);
    let mut steps = 0;
    while let Some(id) = cursor {
        if id == layer {
            return Err(invalid(format!("parenting {layer} to {parent} would create a cycle")));
        }
        let l = comp.layer(id).ok_or_else(|| invalid(format!("parent {id} is not in the same composition")))?;
        cursor = l.meta.parent;
        steps += 1;
        if steps > comp.layers.len() {
            return Err(invalid("existing parent chain contains a cycle"));
        }
    }
    Ok(())
}
