//! Bringing outside content in: image files, SVG documents, 3D models and
//! Xenify Vector projects, whether from File ▸ Import, the clipboard or drag and drop.

use crate::app::XenifyApp;
use xenify_doc::{Id, Layer, LayerKind, Vec2};
use xenify_ops::Command;
use std::path::PathBuf;

/// What a file or blob is, judged by its content first and its name second.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// A Xenify Vector project: opened, not imported.
    Project(PathBuf),
    Svg {
        name: String,
        bytes: Vec<u8>,
    },
    Image {
        name: String,
        bytes: Vec<u8>,
    },
    /// A Lottie animation (JSON or dotLottie).
    Lottie {
        name: String,
        bytes: Vec<u8>,
    },
    /// A 3D model (STL, OBJ, glTF, 3MF). `name` keeps its extension; `path`
    /// finds files it refers to (materials, buffers).
    Model {
        name: String,
        path: Option<PathBuf>,
        bytes: Vec<u8>,
    },
    /// A video or audio file (linked by path; not read into memory here).
    Media {
        name: String,
        path: PathBuf,
    },
    Unsupported(String),
}

/// File extensions accepted by File ▸ Import and drag and drop.
pub const IMPORT_EXTENSIONS: &[&str] = &[
    "svg", "png", "jpg", "jpeg", "webp", "gif", "bmp", "stl", "obj", "gltf", "glb", "3mf", "xenify", "mp4", "m4v",
    "json", "lottie", "mov", "webm", "mkv", "avi", "wav", "mp3", "ogg", "oga", "opus", "flac", "m4a", "aac",
];

/// Sorts `bytes` (from `path`, or a clipboard/drop without one) by what they are.
pub fn classify(name: &str, path: Option<PathBuf>, bytes: Vec<u8>) -> Incoming {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s).to_string();
    let is_image = head.starts_with(b"\x89PNG")
        || head.starts_with(b"\xFF\xD8\xFF")
        || head.starts_with(b"GIF8")
        || head.starts_with(b"BM")
        || (head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP");
    if is_image {
        return Incoming::Image { name: stem, bytes };
    }
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let lottie_json =
        trimmed.starts_with('{') && text.contains("\"fr\"") && (text.contains("\"layers\"") || text.contains("\"v\""));
    if (ext == "lottie" && head.starts_with(b"PK")) || lottie_json && !text.contains("\"compositions\"") {
        return Incoming::Lottie { name: stem, bytes };
    }
    if xenify_export::model3d::ModelFormat::detect(&lower, head).is_some() {
        return Incoming::Model { name: name.to_string(), path, bytes };
    }
    if let Some(hint) = xenify_export::model3d::unsupported_hint(ext) {
        return Incoming::Unsupported(format!("{name}: {hint}"));
    }
    if ext == "xenify" || (trimmed.starts_with('{') && text.contains("\"compositions\"")) {
        return match path {
            Some(p) => Incoming::Project(p),
            None => Incoming::Unsupported(format!("{name}: save the project to a file to open it")),
        };
    }
    if ext == "svg" || trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && text.contains("<svg")) {
        return Incoming::Svg { name: stem, bytes };
    }
    Incoming::Unsupported(format!("{name}: not an image, SVG, Lottie, 3D model or Xenify Vector project"))
}

/// A video or audio file as a (linked) clip with id `id`, read with ffprobe.
pub(crate) fn media_asset(path: &std::path::Path, name: &str, id: Id) -> Result<xenify_doc::MediaAsset, String> {
    let info = xenify_platform::media::probe(path).map_err(|e| format!("{name}: {e}"))?;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(xenify_doc::MediaAsset {
        id,
        name: name.to_string(),
        kind: if info.has_video { xenify_doc::MediaKind::Video } else { xenify_doc::MediaKind::Audio },
        mime: xenify_platform::media::mime_for(path).to_string(),
        source: xenify_doc::MediaSource::Linked { path: path.to_string_lossy().into_owned() },
        duration: info.duration,
        width: info.width,
        height: info.height,
        fps: info.fps,
        has_audio: info.has_audio,
        hash: xenify_doc::hash_bytes(format!("{}:{size}", path.display()).as_bytes()),
        recipe: None,
    })
}

/// Drag and drop: highlights the canvas while files are held over the
/// window, and imports them where they're dropped.
pub fn handle_drops(app: &mut XenifyApp, ctx: &egui::Context) {
    let (hovered, dropped, pointer) =
        ctx.input(|i| (i.raw.hovered_files.len(), i.raw.dropped_files.clone(), i.pointer.latest_pos()));
    if hovered > 0 {
        let rect = app.viewport.screen_rect();
        if rect.is_positive() {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            painter.rect_filled(rect, 6.0, crate::widgets::ACCENT.gamma_multiply(0.18));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("Drop to import {hovered} file{} (.xenify files open)", if hovered == 1 { "" } else { "s" }),
                egui::FontId::proportional(20.0),
                egui::Color32::WHITE,
            );
        }
    }
    if dropped.is_empty() {
        return;
    }
    let at = pointer.and_then(|p| app.viewport.screen_to_comp(p));
    let items: Vec<Incoming> = dropped
        .into_iter()
        .filter_map(|f| {
            let name = if f.name.is_empty() {
                f.path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            } else {
                f.name.clone()
            };
            let path = f.path.as_deref().map(crate::platform::local_path);
            let bytes = match (&f.bytes, &path) {
                (Some(b), _) => b.to_vec(),
                (None, Some(p)) if !p.exists() => {
                    app.status = crate::platform::unreachable_reason(f.path.as_deref().unwrap_or(p));
                    return None;
                }
                (None, Some(p)) => match std::fs::read(p) {
                    Ok(b) => b,
                    Err(e) => {
                        app.fail(format!("Couldn't read {}: {e}", p.display()));
                        return None;
                    }
                },
                (None, None) => return None,
            };
            Some(classify(&name, path, bytes))
        })
        .collect();
    app.import_incoming(items, at);
}

impl XenifyApp {
    /// Reads and imports files (images, SVG) in one undo step; `.xenify`
    /// projects are opened instead. Images and SVGs are centred on `at`.
    pub fn import_files(&mut self, paths: Vec<PathBuf>, at: Option<kurbo::Point>) {
        let mut items = Vec::new();
        for path in paths {
            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let local = crate::platform::local_path(&path);
            if !local.exists() {
                self.status = crate::platform::unreachable_reason(&path);
                continue;
            }
            if xenify_platform::media::is_media_file(&local) {
                let stem = local.file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
                items.push(Incoming::Media { name: stem, path: local });
                continue;
            }
            match std::fs::read(&local) {
                Ok(bytes) => items.push(classify(&name, Some(local), bytes)),
                Err(e) => self.fail(format!("Couldn't read {}: {e}", path.display())),
            }
        }
        self.import_incoming(items, at);
    }

    /// Imports classified content (one undo step for everything that isn't a project).
    pub fn import_incoming(&mut self, items: Vec<Incoming>, at: Option<kurbo::Point>) {
        let mut problems = Vec::new();
        let (projects, content): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| matches!(i, Incoming::Project(_)));
        if !content.is_empty() {
            self.editor.begin_group("Import");
            for item in content {
                match item {
                    Incoming::Svg { name, bytes } => self.import_svg_document(&bytes, &name),
                    Incoming::Image { name, bytes } => {
                        self.add_image(&bytes, &name, at);
                    }
                    Incoming::Lottie { name, bytes } => {
                        if let Err(why) = self.import_lottie(&bytes, &name, at) {
                            problems.push(why);
                        }
                    }
                    // Arrives later, as its own undo step.
                    Incoming::Model { name, path, bytes } => self.add_model(bytes, &name, path.as_deref(), at),
                    Incoming::Media { name, path } => {
                        if let Err(why) = self.add_media(&path, &name) {
                            problems.push(why);
                        }
                    }
                    Incoming::Unsupported(why) => problems.push(why),
                    Incoming::Project(_) => unreachable!(),
                }
            }
            self.editor.end_group();
        }
        for p in projects {
            if let Incoming::Project(path) = p {
                crate::session::guard(self, crate::session::Pending::OpenPath(path));
            }
        }
        if !problems.is_empty() {
            self.status = problems.join("; ");
        }
    }

    /// Adds a video or audio file (linked to it) as a layer starting at the
    /// playhead; a video is fitted to the frame. Returns the layer.
    pub fn add_media(&mut self, path: &std::path::Path, name: &str) -> Result<Id, String> {
        let media = media_asset(path, name, Id(self.project().next_id))?;
        let layer = Id(media.id.0 + 1);
        let cmd = xenify_ops::media::add_clip(self.project(), self.comp, media, layer, self.time)
            .map_err(|e| e.to_string())?;
        if self.exec(&format!("Import {name}"), cmd) {
            self.select_only(Some(layer));
            Ok(layer)
        } else {
            Err(format!("{name}: couldn't add it"))
        }
    }

    /// Adds an image file as an image layer centred on `at` (composition
    /// space; the composition's centre when `None`), scaled down to fit if
    /// it's bigger than the composition. One undo step. Returns the layer.
    pub fn add_image(&mut self, bytes: &[u8], name: &str, at: Option<kurbo::Point>) -> Option<Id> {
        let img = match xenify_export::image::prepare(bytes, self.prefs.max_image_side) {
            Ok(img) => img,
            Err(e) => {
                self.fail(format!("{name}: {e}"));
                return None;
            }
        };
        let mut project = self.project().clone();
        let (add, asset) = xenify_ops::assets::add_image(
            &mut project,
            name,
            img.mime,
            img.width,
            img.height,
            img.data,
            img.frames.clone(),
        );
        let s = &self.comp().settings;
        let (w, h) = (s.width as f64, s.height as f64);
        let at = at.map_or(Vec2::new(w / 2.0, h / 2.0), |p| p.to_vec2());
        let fit = (0.8 * w / img.width.max(1) as f64).min(0.8 * h / img.height.max(1) as f64).min(1.0);
        let id = Id(project.next_id);
        let mut layer =
            Layer::new(id, name, LayerKind::Image { asset, playback: Default::default() }).with_position(at);
        layer.transform.anchor = Vec2::new(img.width as f64 / 2.0, img.height as f64 / 2.0).into();
        layer.transform.scale = Vec2::new(fit, fit).into();
        let cmds: Vec<Command> =
            add.into_iter().chain([Command::AddLayer { comp: self.comp, index: None, layer }]).collect();
        if !self.exec(&format!("Add image {name}"), Command::batch(cmds)) {
            return None;
        }
        self.select_only(Some(id));
        self.status = match &img.frames {
            Some(f) => format!(
                "Added animated image {name} ({}×{} px, {} frames, {:.2} s; it loops — see Properties › Animation)",
                img.width,
                img.height,
                f.len(),
                f.iter().sum::<f64>()
            ),
            None => format!("Added image {name} ({}×{} px)", img.width, img.height),
        };
        Some(id)
    }

    /// Starts importing a 3D model file (read and simplified in the
    /// background); it arrives as a 3D layer centred on `at`, half the
    /// frame's size. `path` is where it was read from, for the files it
    /// refers to.
    pub fn add_model(&mut self, bytes: Vec<u8>, name: &str, path: Option<&std::path::Path>, at: Option<kurbo::Point>) {
        let dir = path.and_then(|p| p.parent()).map(|d| d.to_path_buf());
        let max_triangles = match self.prefs.model_triangles {
            0 => usize::MAX,
            n => n,
        };
        let options = xenify_export::model3d::ModelOptions { max_triangles };
        let (tx, rx) = std::sync::mpsc::channel();
        let file = name.to_string();
        std::thread::Builder::new()
            .name("xenify-model-import".into())
            .spawn(move || {
                // Only files beside (or below) the model, never elsewhere on disk.
                let sibling = |rel: &str| -> Option<Vec<u8>> {
                    let rel = std::path::Path::new(rel);
                    let safe = rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
                    std::fs::read(dir.as_ref()?.join(rel)).ok().filter(|_| safe)
                };
                let result = xenify_export::model3d::import_model(&file, &bytes, options, &sibling);
                let _ = tx.send(result.map_err(|e| e.to_string()));
            })
            .expect("spawn model import thread");
        self.status = format!("Importing {name}…");
        self.pending_models.push(PendingModel { name: name.to_string(), at, comp: self.comp, result: rx });
    }

    /// Adds models whose import finished.
    pub fn poll_models(&mut self, ctx: &egui::Context) {
        let mut i = 0;
        while i < self.pending_models.len() {
            match self.pending_models[i].result.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    i += 1;
                }
                done => {
                    let p = self.pending_models.remove(i);
                    match done {
                        Ok(Ok(model)) => {
                            self.finish_model(model, &p.name, p.comp, p.at);
                        }
                        Ok(Err(e)) => self.fail(format!("{}: {e}", p.name)),
                        Err(_) => self.fail(format!("{}: the import stopped unexpectedly", p.name)),
                    }
                }
            }
        }
    }

    /// Adds an imported model as a 3D layer of `comp`. One undo step.
    fn finish_model(
        &mut self,
        model: xenify_export::model3d::ImportedModel,
        name: &str,
        comp: Id,
        at: Option<kurbo::Point>,
    ) -> Option<Id> {
        if self.comp != comp {
            self.open_comp(comp);
        }
        let s = &self.comp().settings;
        let (w, h) = (s.width as f64, s.height as f64);
        let at = at.map_or(Vec2::new(w / 2.0, h / 2.0), |p| p.to_vec2());
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s).to_string();
        let color = xenify_doc::Color::from_hex("#b9c6d8").unwrap_or(xenify_doc::Color::WHITE);
        let solid = xenify_doc::Solid::new(
            xenify_doc::SolidShape::Mesh(Box::new(model.mesh.clone())),
            (w.min(h) * 0.5).round(),
            0.0,
            color,
        );
        let id = Id(self.project().next_id);
        let layer = Layer::new(id, stem.clone(), LayerKind::Solid(Box::new(solid))).with_position(at);
        if !self.exec(&format!("Import {stem}"), Command::AddLayer { comp, index: None, layer }) {
            return None;
        }
        self.select_only(Some(id));
        self.set_tool(crate::app::Tool::Orbit);
        self.status = format!(
            "Imported {name}: {}. Drag to turn it; X/Y/Z rotation and size are keyframeable in Properties.",
            model.summary()
        );
        Some(id)
    }

    /// Adds a Lottie animation as a composition of its own (its layers and
    /// keyframes editable inside), placed as a precomp layer centred on `at`
    /// and starting at the playhead. One undo step. Returns the layer.
    pub fn import_lottie(&mut self, data: &[u8], name: &str, at: Option<kurbo::Point>) -> Result<Id, String> {
        let import = xenify_export::lottie_import::import_lottie(data, self.project().next_id)
            .map_err(|e| format!("{name}: {e}"))?;
        let summary = import.summary();
        let (comp_id, layer) = (Id(import.next_id), Id(import.next_id + 1));
        let (mut main, extras) = import.into_compositions(comp_id);
        if main.settings.name.trim().is_empty() || main.settings.name == "Lottie" {
            main.settings.name = name.to_string();
        }
        let s = &self.comp().settings;
        let at = at.map_or(Vec2::new(s.width as f64 / 2.0, s.height as f64 / 2.0), |p| Vec2::new(p.x, p.y));
        let cmd = xenify_ops::precomp::add_precomp(
            self.project(),
            self.comp,
            main,
            extras,
            layer,
            at,
            self.time,
            None,
            false,
        )
        .map_err(|e| e.to_string())?;
        if !self.exec(&format!("Import {name}"), cmd) {
            return Err(format!("{name}: couldn't add it"));
        }
        self.select_only(Some(layer));
        self.status = format!("Imported {name} as a composition (double-click to edit its layers). {summary}");
        Ok(layer)
    }

    /// Adds an SVG document's shapes, images and named groups, wrapped in a
    /// group called `name` (one undo step).
    pub fn import_svg_document(&mut self, data: &[u8], name: &str) {
        let s = &self.comp().settings;
        let opts =
            xenify_export::svg::SvgImportOptions { max_image_side: self.prefs.max_image_side, ..Default::default() };
        let import =
            match xenify_export::svg::import_svg_document(data, self.project().next_id, s.width, s.height, opts) {
                Ok(i) => i,
                Err(e) => {
                    self.fail(e);
                    return;
                }
            };
        // Images the project already has are shared rather than added again.
        let mut layers = import.layers;
        let mut adds = Vec::new();
        for asset in import.assets {
            match self.project().asset_with_data(&asset.data) {
                Some(existing) => {
                    for l in &mut layers {
                        l.remap_asset(asset.id, existing.id);
                    }
                }
                None => adds.push(Command::AddAsset { asset }),
            }
        }
        self.editor.begin_group(format!("Import {name}"));
        if adds.is_empty() || self.exec("Add images", Command::batch(adds)) {
            self.add_layer_group(layers, name);
        }
        self.editor.end_group();
    }
}

/// A 3D model being read in the background.
pub struct PendingModel {
    pub(crate) name: String,
    at: Option<kurbo::Point>,
    /// The composition it was dropped on.
    comp: Id,
    result: std::sync::mpsc::Receiver<Result<xenify_export::model3d::ImportedModel, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_content_then_name() {
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        assert!(matches!(classify("shot.dat", None, png), Incoming::Image { name, .. } if name == "shot"));
        let svg = b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec();
        assert!(matches!(classify("logo", None, svg), Incoming::Svg { .. }));
        let project = br#"{"format_version":1,"compositions":[]}"#.to_vec();
        let path = PathBuf::from("/tmp/a.xenify");
        assert_eq!(classify("a.xenify", Some(path.clone()), project.clone()), Incoming::Project(path));
        assert!(matches!(classify("a.xenify", None, project), Incoming::Unsupported(_)));
        assert!(matches!(classify("notes.txt", None, b"hello".to_vec()), Incoming::Unsupported(_)));
        assert!(
            matches!(classify("part.STL", None, b"solid x".to_vec()), Incoming::Model { name, .. } if name == "part.STL")
        );
        assert!(matches!(classify("scene.bin", None, b"glTF\x02\0\0\0".to_vec()), Incoming::Model { .. }));
        assert!(
            matches!(classify("bracket.step", None, b"ISO-10303-21;".to_vec()), Incoming::Unsupported(why) if why.contains("STL"))
        );
        let lottie = br#"{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[]}"#.to_vec();
        assert!(matches!(classify("anim.json", None, lottie), Incoming::Lottie { name, .. } if name == "anim"));
        assert!(matches!(classify("data.json", None, br#"{"a":1}"#.to_vec()), Incoming::Unsupported(_)));
        let webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        assert!(matches!(classify("x.webp", None, webp), Incoming::Image { .. }));
    }
}
