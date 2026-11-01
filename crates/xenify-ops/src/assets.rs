//! Embedded images: adding them without duplicates.

use crate::Command;
use xenify_doc::{Id, ImageAsset, Project};

/// The command adding an image (already decoded enough to know its size and
/// type) and the asset id to use in image layers. Identical bytes reuse the
/// project's existing asset: the command is then `None`. `frames` are an
/// animated image's frame timings.
pub fn add_image(
    project: &mut Project,
    name: &str,
    mime: &str,
    width: u32,
    height: u32,
    data: Vec<u8>,
    frames: Option<Vec<f64>>,
) -> (Option<Command>, Id) {
    if let Some(a) = project.asset_with_data(&data) {
        return (None, a.id);
    }
    let mut asset = ImageAsset::new(project.alloc_id(), name, mime, width, height, data);
    asset.frames = frames;
    let id = asset.id;
    (Some(Command::AddAsset { asset }), id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{Layer, Vec2};

    #[test]
    fn identical_images_are_stored_once_and_undo_cleanly() {
        let mut p = Project::new();
        let (cmd, a) = add_image(&mut p, "one", "image/png", 2, 2, vec![1, 2, 3], None);
        let undo = apply(&mut p, cmd.unwrap()).unwrap();
        let (again, b) = add_image(&mut p, "two", "image/png", 2, 2, vec![1, 2, 3], None);
        assert!(again.is_none());
        assert_eq!(a, b);
        assert_eq!(p.assets.len(), 1);

        // Used by a layer: can't be removed; the layer needs the asset to exist.
        let comp = p.compositions[0].id;
        let id = p.alloc_id();
        let layer = Layer::image(id, "img", p.asset(a).unwrap(), Vec2::ZERO);
        apply(&mut p, Command::AddLayer { comp, index: None, layer: layer.clone() }).unwrap();
        assert!(apply(&mut p, undo.clone()).is_err());
        apply(&mut p, Command::RemoveLayer { layer: id }).unwrap();
        let redo = apply(&mut p, undo).unwrap();
        assert!(p.assets.is_empty());
        apply(&mut p, redo).unwrap();
        assert_eq!(p.assets.len(), 1);

        let missing =
            Layer::image(p.alloc_id(), "x", &ImageAsset::new(Id(999), "x", "image/png", 1, 1, vec![]), Vec2::ZERO);
        assert!(apply(&mut p, Command::AddLayer { comp, index: None, layer: missing }).is_err());
    }
}
