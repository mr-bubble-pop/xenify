use crate::{Color, Id, ImageAsset, Layer, PropPath, ValueKind};
use serde::{Deserialize, Serialize};

/// The file format level this version of Draw writes and understands.
///
/// A file stores two numbers: `format_version`, the lowest level a reader
/// needs to open it correctly (worked out from what the project uses, so
/// plain projects stay readable by older Draws), and `saved_with`, the level
/// of the Draw that wrote it. Bump this whenever a change would make older
/// Draws misread a file (a new layer, effect, modifier or other tagged kind,
/// or a changed meaning) and raise `format_version` in `to_json` for projects
/// that use it. New optional fields with `#[serde(default)]` need no bump.
/// See docs/formats.md.
///
/// 2: embedded image assets. Files without images are still written as 1 so
/// older versions of Xenify Draw keep opening them.
/// 3: video and audio (written only when the project has some).
pub const FORMAT_VERSION: u32 = 3;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProjectError {
    #[error("no composition {0}")]
    NoSuchComposition(Id),
    #[error("no layer {0}")]
    NoSuchLayer(Id),
    #[error("layer {layer} has no property {path}")]
    NoSuchProperty { layer: Id, path: PropPath },
    #[error("property {path} of layer {layer} expects a {expected:?} value")]
    TypeMismatch { layer: Id, path: PropPath, expected: ValueKind },
    #[error("id {0} is already in use")]
    DuplicateId(Id),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompSettings {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Length in seconds.
    pub duration: f64,
    pub background: Color,
    /// Motion blur for rendered output (export, previews), like After Effects'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_blur: Option<MotionBlur>,
    /// Named regions exported as separate images/videos (File ▸ Export Slices).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<Slice>,
    /// Times (seconds) where a presentation stops until the presenter
    /// continues.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pauses: Vec<f64>,
    /// The unit sizes are shown and typed in.
    #[serde(default, skip_serializing_if = "crate::Units::is_default")]
    pub units: crate::Units,
    /// Guide lines dragged out of the rulers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<crate::GuideLine>,
    /// Grid spacing in pixels (0: one unit, or 50 px for pixel documents).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub grid: f64,
    /// What the canvas is for: a diagram canvas shows the diagram tools and
    /// panels while it's open.
    #[serde(default, skip_serializing_if = "CompKind::is_animation")]
    pub kind: CompKind,
    /// Defaults for new connectors drawn here (the built-in ones when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram: Option<crate::DiagramSettings>,
}

/// What a composition is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompKind {
    #[default]
    Animation,
    /// A flowchart, network or other diagram: shapes joined by connectors.
    Diagram,
}

impl CompKind {
    pub fn is_animation(&self) -> bool {
        *self == CompKind::Animation
    }
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl CompSettings {
    /// The grid's spacing in pixels.
    pub fn grid_spacing(&self) -> f64 {
        if self.grid > 0.0 {
            self.grid
        } else if self.units.unit == crate::Unit::Px {
            50.0
        } else {
            // Ten per centimetre / inch, one per mm or point.
            let per = match self.units.unit {
                crate::Unit::Cm | crate::Unit::In => 0.1,
                _ => 1.0,
            };
            let px = self.units.to_px(per);
            // Keep it visible: at least 8 px apart.
            px * (8.0 / px).max(1.0).ceil()
        }
    }
}

/// A rectangle of the composition that exports as its own file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slice {
    pub name: String,
    /// Composition pixels.
    pub rect: kurbo::Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionBlur {
    /// Degrees of a frame the shutter is open (180 = half a frame, film-like).
    pub shutter_angle: f64,
    /// Sub-frame renders averaged per frame.
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self { shutter_angle: 180.0, samples: 12 }
    }
}

impl Default for CompSettings {
    fn default() -> Self {
        Self {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 5.0,
            background: Color::rgb(0.08, 0.08, 0.1),
            motion_blur: None,
            slices: Vec::new(),
            pauses: Vec::new(),
            units: Default::default(),
            guides: Vec::new(),
            grid: 0.0,
            kind: CompKind::Animation,
            diagram: None,
        }
    }
}

impl CompSettings {
    pub fn frame_count(&self) -> u64 {
        (self.duration * self.fps).round().max(0.0) as u64
    }

    pub fn frame_time(&self, frame: u64) -> f64 {
        frame as f64 / self.fps
    }

    /// The times to average for the frame at `time`: just `time` without motion
    /// blur, else samples spread over the open shutter, centred on `time`.
    pub fn shutter_times(&self, time: f64) -> Vec<f64> {
        match self.motion_blur {
            Some(mb) if mb.samples > 1 && mb.shutter_angle > 0.0 => {
                let span = mb.shutter_angle.min(720.0) / 360.0 / self.fps;
                let n = mb.samples.min(64);
                (0..n)
                    .map(|i| (time - span / 2.0 + span * (i as f64 + 0.5) / n as f64).clamp(0.0, self.duration))
                    .collect()
            }
            _ => vec![time],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub id: Id,
    #[serde(flatten)]
    pub settings: CompSettings,
    /// Paint order: index 0 is drawn first (bottom-most).
    pub layers: Vec<Layer>,
}

impl Composition {
    pub fn layer(&self, id: Id) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_index(&self, id: Id) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub format_version: u32,
    pub compositions: Vec<Composition>,
    /// Next free id. Only ever increases.
    pub next_id: u64,
    /// Embedded raster images, referenced by image layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ImageAsset>,
    /// Video and audio files, played by video and audio layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<crate::MediaAsset>,
    /// Sound playing across the whole presentation.
    #[serde(default, skip_serializing_if = "crate::Soundtrack::is_empty")]
    pub soundtrack: crate::Soundtrack,
    /// The presentation's slides in order (empty: not a presentation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slides: Vec<crate::Slide>,
    /// The deck's theme (see [`crate::THEMES`]), used for new slides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    /// A project with a single empty 1080p composition.
    pub fn new() -> Self {
        let mut p = Self {
            format_version: FORMAT_VERSION,
            compositions: Vec::new(),
            next_id: 1,
            assets: Vec::new(),
            media: Vec::new(),
            soundtrack: Default::default(),
            slides: Vec::new(),
            theme: None,
        };
        let id = p.alloc_id();
        p.compositions.push(Composition { id, settings: CompSettings::default(), layers: Vec::new() });
        p
    }

    pub fn alloc_id(&mut self) -> Id {
        let id = Id(self.next_id);
        self.next_id += 1;
        id
    }

    /// Makes sure `id` will never be handed out by [`Self::alloc_id`].
    pub fn reserve_id(&mut self, id: Id) {
        self.next_id = self.next_id.max(id.0 + 1);
    }

    pub fn id_in_use(&self, id: Id) -> bool {
        self.compositions.iter().any(|c| c.id == id || c.layers.iter().any(|l| l.id == id))
            || self.assets.iter().any(|a| a.id == id)
            || self.media.iter().any(|m| m.id == id)
    }

    pub fn media(&self, id: Id) -> Option<&crate::MediaAsset> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Index of the slide showing composition `comp`.
    pub fn slide_index(&self, comp: Id) -> Option<usize> {
        self.slides.iter().position(|s| s.comp == comp)
    }

    pub fn asset(&self, id: Id) -> Option<&ImageAsset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// The asset holding exactly `data`, if the project already has it.
    pub fn asset_with_data(&self, data: &[u8]) -> Option<&ImageAsset> {
        self.assets.iter().find(|a| a.is(data))
    }

    /// Whether any layer shows asset `id`.
    pub fn asset_in_use(&self, id: Id) -> bool {
        self.compositions.iter().flat_map(|c| &c.layers).any(|l| l.asset_refs().contains(&id))
            || self.soundtrack.clips.iter().any(|c| c.audio.asset == id)
    }

    pub fn comp(&self, id: Id) -> Result<&Composition, ProjectError> {
        self.compositions.iter().find(|c| c.id == id).ok_or(ProjectError::NoSuchComposition(id))
    }

    pub fn comp_mut(&mut self, id: Id) -> Result<&mut Composition, ProjectError> {
        self.compositions.iter_mut().find(|c| c.id == id).ok_or(ProjectError::NoSuchComposition(id))
    }

    /// The first composition (projects always have at least one when created
    /// through [`Project::new`]).
    pub fn main_comp(&self) -> Option<&Composition> {
        self.compositions.first()
    }

    /// Finds a layer anywhere in the project, returning `(composition id, index)`.
    pub fn locate_layer(&self, id: Id) -> Result<(Id, usize), ProjectError> {
        self.compositions.iter().find_map(|c| c.layer_index(id).map(|i| (c.id, i))).ok_or(ProjectError::NoSuchLayer(id))
    }

    pub fn layer(&self, id: Id) -> Result<&Layer, ProjectError> {
        self.compositions.iter().find_map(|c| c.layer(id)).ok_or(ProjectError::NoSuchLayer(id))
    }

    pub fn layer_mut(&mut self, id: Id) -> Result<&mut Layer, ProjectError> {
        self.compositions
            .iter_mut()
            .find_map(|c| c.layers.iter_mut().find(|l| l.id == id))
            .ok_or(ProjectError::NoSuchLayer(id))
    }

    /// The project file. Assets no layer uses are left out (they stay in
    /// memory for undo).
    pub fn to_json(&self) -> String {
        let unused =
            self.assets.iter().any(|a| !self.asset_in_use(a.id)) || self.media.iter().any(|m| !self.asset_in_use(m.id));
        let pruned;
        let p = if unused {
            let mut p = self.clone();
            p.assets.retain(|a| self.asset_in_use(a.id));
            p.media.retain(|m| self.asset_in_use(m.id));
            pruned = p;
            &pruned
        } else {
            self
        };
        let mut json = serde_json::to_value(p).expect("project serialisation cannot fail");
        let version = if !p.media.is_empty() {
            FORMAT_VERSION
        } else if !p.assets.is_empty() {
            2
        } else {
            1
        };
        json["format_version"] = version.into();
        json["saved_with"] = FORMAT_VERSION.into();
        serde_json::to_string_pretty(&json).expect("project serialisation cannot fail")
    }

    pub fn from_json(json: &str) -> Result<Self, ProjectError> {
        // Check the levels before the typed read: a newer file may use kinds
        // this version doesn't know, which would otherwise read as "invalid".
        let raw: serde_json::Value =
            serde_json::from_str(json).map_err(|e| ProjectError::Invalid(format!("invalid project file: {e}")))?;
        let level = |key: &str| raw.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        let newer = || {
            ProjectError::Invalid(
                "This project was made with a newer version of Xenify Draw. Update Draw to open it.".into(),
            )
        };
        if level("format_version") > FORMAT_VERSION as u64 {
            return Err(newer());
        }
        let mut p: Project = serde_json::from_value(raw.clone()).map_err(|e| {
            if level("saved_with") > FORMAT_VERSION as u64 {
                newer()
            } else {
                ProjectError::Invalid(format!("invalid project file: {e}"))
            }
        })?;
        p.format_version = FORMAT_VERSION;
        // Guard against hand-edited files with a stale counter.
        let max = p
            .compositions
            .iter()
            .flat_map(|c| std::iter::once(c.id).chain(c.layers.iter().map(|l| l.id)))
            .chain(p.assets.iter().map(|a| a.id))
            .chain(p.media.iter().map(|m| m.id))
            .max();
        if let Some(max) = max {
            p.reserve_id(max);
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Easing, ShapeItem, Value};
    use kurbo::Vec2;

    #[test]
    fn json_round_trip() {
        let mut p = Project::new();
        let comp = p.compositions[0].id;
        let id = p.alloc_id();
        let mut layer = Layer::shape(id, "Ball", vec![ShapeItem::ellipse(Vec2::new(50.0, 50.0), Color::WHITE)]);
        layer.set_keyframe(PropPath::Position, 0.0, Value::Vec2(Vec2::new(0.0, 0.0)), Some(Easing::EASE_OUT)).unwrap();
        layer.set_keyframe(PropPath::Position, 1.0, Value::Vec2(Vec2::new(100.0, 0.0)), None).unwrap();
        p.comp_mut(comp).unwrap().layers.push(layer);

        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.layer(id).unwrap().meta.name, "Ball");
        assert_eq!(back.locate_layer(id).unwrap(), (comp, 0));
    }

    #[test]
    fn stale_next_id_is_repaired() {
        let mut p = Project::new();
        let id = Id(40);
        p.compositions[0].layers.push(Layer::new(id, "N", crate::LayerKind::Null));
        p.next_id = 1;
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.next_id, 41);
    }

    #[test]
    fn rejects_future_format() {
        let mut json: serde_json::Value = serde_json::from_str(&Project::new().to_json()).unwrap();
        json["format_version"] = (FORMAT_VERSION + 1).into();
        let err = Project::from_json(&json.to_string()).unwrap_err().to_string();
        assert!(err.contains("newer version"), "{err}");
    }

    #[test]
    fn unknown_kinds_from_a_newer_draw_say_update() {
        let mut json: serde_json::Value = serde_json::from_str(&Project::new().to_json()).unwrap();
        assert_eq!(json["saved_with"], FORMAT_VERSION);
        json["compositions"][0]["layers"] = serde_json::json!([{"id": 9, "name": "x", "kind": {"Hologram": {}}}]);
        // Saved by a newer Draw: it's ours to update, not damaged.
        json["saved_with"] = (FORMAT_VERSION + 1).into();
        let err = Project::from_json(&json.to_string()).unwrap_err().to_string();
        assert!(err.contains("newer version"), "{err}");
        // The same file from our own level really is invalid.
        json["saved_with"] = FORMAT_VERSION.into();
        let err = Project::from_json(&json.to_string()).unwrap_err().to_string();
        assert!(err.contains("invalid project file"), "{err}");
    }

    #[test]
    fn images_bump_the_format_and_unused_assets_are_dropped() {
        let mut p = Project::new();
        let json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        assert_eq!(json["format_version"], 1, "no images: readable by older versions");
        assert!(json.get("assets").is_none());

        let used = ImageAsset::new(p.alloc_id(), "a", "image/png", 4, 2, vec![1, 2, 3]);
        let unused = ImageAsset::new(p.alloc_id(), "b", "image/png", 1, 1, vec![9]);
        let id = p.alloc_id();
        p.compositions[0].layers.push(Layer::image(id, "A", &used, kurbo::Vec2::new(10.0, 10.0)));
        p.assets = vec![used.clone(), unused];
        let text = p.to_json();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["format_version"], 2);
        let back = Project::from_json(&text).unwrap();
        assert_eq!(back.assets, vec![used.clone()]);
        assert_eq!(back.layer(id).unwrap().transform.anchor.value_at(0.0), kurbo::Vec2::new(2.0, 1.0));
        assert_eq!(back.asset_with_data(&[1, 2, 3]).map(|a| a.id), Some(used.id));
        assert!(back.next_id > id.0);
    }

    #[test]
    fn version_1_files_still_load() {
        let json = r#"{"format_version":1,"compositions":[],"next_id":5}"#;
        let p = Project::from_json(json).unwrap();
        assert!(p.assets.is_empty());
        assert_eq!(p.next_id, 5);
    }

    #[test]
    fn frame_math() {
        let s = CompSettings { fps: 24.0, duration: 2.0, ..Default::default() };
        assert_eq!(s.frame_count(), 48);
        assert_eq!(s.frame_time(12), 0.5);
    }
}
