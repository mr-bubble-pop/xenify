//! Copy, cut and paste of layers and keyframes (Ctrl+C / Ctrl+X / Ctrl+V),
//! and pasting images, SVG and files from other apps.
//!
//! Xenify Vector's own clips travel through the system clipboard as tagged JSON,
//! which also lets you paste between Xenify Vector windows. egui only reports a
//! paste when the clipboard has text; when it holds just an image, the
//! Ctrl+V key release is the only sign, so then the clipboard is read
//! directly through the platform.

use crate::app::{XenifyApp, KeySel};
use xenify_doc::{Easing, Id, Layer, PropPath, Value};
use xenify_ops::Command;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const TAG: &str = "xenify-clipboard-v1";

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    clip: Clip,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Clip {
    /// Bottom-most first, with their group members and the images they show.
    Layers {
        layers: Vec<Layer>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        assets: Vec<xenify_doc::ImageAsset>,
    },
    /// Times relative to the earliest copied keyframe.
    Keyframes { keys: Vec<ClipKey> },
}

#[derive(Serialize, Deserialize)]
struct ClipKey {
    path: PropPath,
    time: f64,
    value: Value,
    easing: Easing,
}

/// Handles this frame's copy/cut/paste events (skipped while typing).
pub fn handle(app: &mut XenifyApp, ctx: &egui::Context) {
    if ctx.text_edit_focused() || app.dialog.is_some() {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let shift = ctx.input(|i| i.modifiers.shift);
    let saw_paste = egui::Id::new("xenify-saw-paste");
    for e in events {
        match e {
            // Ctrl+Shift+C is Precompose.
            egui::Event::Copy if !shift => copy(app, ctx, false),
            egui::Event::Cut => copy(app, ctx, true),
            egui::Event::Paste(text) => {
                ctx.data_mut(|d| d.insert_temp(saw_paste, true));
                paste(app, &text);
            }
            // Ctrl+V let go without a text paste: the clipboard holds something else.
            egui::Event::Key { key: egui::Key::V, pressed: false, modifiers, .. }
                if modifiers.command && !ctx.data_mut(|d| d.remove_temp::<bool>(saw_paste)).unwrap_or(false) =>
            {
                paste_os(app);
            }
            _ => {}
        }
    }
}

/// Edit ▸ Copy / Cut.
pub fn copy_selection(app: &mut XenifyApp, ctx: &egui::Context, cut: bool) {
    copy(app, ctx, cut);
}

/// Edit ▸ Duplicate (Ctrl+D): the selected layers again, right above them.
/// Transform Again: when the selection is what the last Duplicate made and
/// it has been moved, turned or scaled since, the new copies get the same
/// change, so pressing it again and again makes a row, an arc or a spiral.
pub fn duplicate(app: &mut XenifyApp) {
    let Some((Clip::Layers { layers, assets }, _)) = copy_layers(app) else {
        app.status = "Select layers to duplicate.".into();
        return;
    };
    let comp = app.comp().clone();
    let by_order = |ids: &mut Vec<Id>| ids.sort_by_key(|id| comp.layer_index(*id));
    // The selected layers that aren't inside another selected group.
    let selected = app.selected_layers();
    let mut sources: Vec<Id> = selected
        .iter()
        .copied()
        .filter(|id| !selected.iter().any(|g| g != id && xenify_anim::descendants(&comp, *g).contains(id)))
        .collect();
    by_order(&mut sources);
    let again = transform_again(app, &sources);
    let (mut cmds, roots) = paste_commands(app, layers, assets);
    let mut label = "Duplicate";
    if let Some(step) = again {
        let mut scratch = app.project().clone();
        let (time, auto_key) = (app.time, app.auto_key);
        if xenify_ops::apply(&mut scratch, Command::batch(cmds.clone())).is_ok() {
            for id in &roots {
                let snap = xenify_ops::transform::Snapshot::new(&scratch, comp.id, *id, time, kurbo::Rect::ZERO);
                if let Ok(c) = snap.and_then(|s| s.affine_commands(&scratch, step, time, auto_key)) {
                    cmds.extend(c);
                }
            }
            label = "Duplicate and transform again";
        }
    }
    let n = roots.len();
    if app.exec(label, Command::batch(cmds)) {
        app.last_duplicate =
            if n == sources.len() { sources.into_iter().zip(roots.clone()).collect() } else { Vec::new() };
        app.select_only(roots.first().copied());
        app.extra_selected = roots.into_iter().skip(1).collect();
        app.status = match again {
            Some(_) => "Duplicated, with the same move again (Transform Again).".into(),
            None => format!(
                "Duplicated {n} layer{}. Move, turn or scale the copy, then Duplicate again to repeat that.",
                if n == 1 { "" } else { "s" }
            ),
        };
    }
}

/// The change (composition space) from the last Duplicate's originals to
/// its copies, when `sources` are those copies and all moved alike.
fn transform_again(app: &XenifyApp, sources: &[Id]) -> Option<kurbo::Affine> {
    let pairs = &app.last_duplicate;
    let mut copies: Vec<Id> = pairs.iter().map(|(_, c)| *c).collect();
    let comp = app.comp();
    copies.sort_by_key(|id| comp.layer_index(*id));
    if copies.is_empty() || copies != sources {
        return None;
    }
    let delta = |(a, b): &(Id, Id)| -> Option<kurbo::Affine> {
        comp.layer(*a)?;
        let (wa, wb) =
            (xenify_anim::world_transform(comp, *a, app.time), xenify_anim::world_transform(comp, *b, app.time));
        (wa.determinant().abs() > 1e-12).then(|| wb * wa.inverse())
    };
    let first = delta(&pairs[0])?;
    let close =
        |m: kurbo::Affine, n: kurbo::Affine| m.as_coeffs().iter().zip(n.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-6);
    let alike = pairs.iter().all(|p| delta(p).is_some_and(|d| close(d, first)));
    (alike && !close(first, kurbo::Affine::IDENTITY)).then_some(first)
}

/// Pastes whatever the system clipboard holds: Xenify Vector layers, an SVG, an
/// image, or image/SVG files copied in a file manager.
pub fn paste_os(app: &mut XenifyApp) {
    use xenify_platform::desktop::ClipContent;
    match app.platform.clipboard.read() {
        Some(ClipContent::Files(paths)) => app.import_files(paths, None),
        Some(ClipContent::Svg(svg)) => app.import_svg_document(svg.as_bytes(), "Pasted SVG"),
        Some(ClipContent::Image { width, height, rgba }) => {
            match xenify_export::image::from_rgba(width, height, rgba, app.prefs.max_image_side) {
                Ok(img) => {
                    app.add_image(&img.data, "Pasted image", None);
                }
                Err(e) => app.fail(format!("Couldn't paste the image: {e}")),
            }
        }
        Some(ClipContent::Encoded { bytes }) => {
            app.add_image(&bytes, "Pasted image", None);
        }
        Some(ClipContent::Text(text)) => paste(app, &text),
        None => app.status = "The clipboard is empty (or holds nothing Xenify Vector can paste).".into(),
    }
}

fn copy(app: &mut XenifyApp, ctx: &egui::Context, cut: bool) {
    let clip = if !app.selected_keys.is_empty() { copy_keys(app) } else { copy_layers(app) };
    let Some((clip, what)) = clip else {
        app.status = "Select layers or keyframes to copy.".into();
        return;
    };
    let json = serde_json::to_string(&Envelope { format: TAG.into(), clip }).expect("clip serialises");
    ctx.copy_text(json);
    if cut {
        if app.selected_keys.is_empty() {
            app.delete_selection();
        } else {
            let cmds = app
                .selected_keys
                .iter()
                .map(|k| Command::RemoveKeyframe { layer: k.layer, path: k.path, time: k.time })
                .collect();
            app.exec("Cut keyframes", Command::batch(cmds));
            app.fix_selection();
        }
        app.status = format!("Cut {what}.");
    } else {
        app.status = format!("Copied {what}.");
    }
}

fn copy_layers(app: &XenifyApp) -> Option<(Clip, String)> {
    let comp = app.comp();
    let mut ids: HashSet<Id> = HashSet::new();
    for id in app.selected_layers() {
        ids.insert(id);
        ids.extend(xenify_anim::descendants(comp, id));
    }
    if ids.is_empty() {
        return None;
    }
    let layers: Vec<Layer> = comp.layers.iter().filter(|l| ids.contains(&l.id)).cloned().collect();
    let assets = layers
        .iter()
        .flat_map(|l| l.asset_refs())
        .filter_map(|asset| app.project().asset(asset).cloned())
        .fold(Vec::<xenify_doc::ImageAsset>::new(), |mut v, a| {
            if !v.iter().any(|x| x.id == a.id) {
                v.push(a);
            }
            v
        });
    let n = app.selected_layers().len();
    Some((Clip::Layers { layers, assets }, if n == 1 { "1 layer".into() } else { format!("{n} layers") }))
}

fn copy_keys(app: &XenifyApp) -> Option<(Clip, String)> {
    let t0 = app.selected_keys.iter().map(|k| k.time).fold(f64::INFINITY, f64::min);
    let keys: Vec<ClipKey> = app
        .selected_keys
        .iter()
        .filter_map(|k: &KeySel| {
            let layer = app.project().layer(k.layer).ok()?;
            let prop = layer.property(k.path).ok()?;
            Some(ClipKey {
                path: k.path,
                time: k.time - t0,
                value: layer.value_at(k.path, k.time).ok()?,
                easing: prop.easing_at(k.time).unwrap_or_default(),
            })
        })
        .collect();
    let n = keys.len();
    (n > 0).then(|| (Clip::Keyframes { keys }, if n == 1 { "1 keyframe".into() } else { format!("{n} keyframes") }))
}

fn paste(app: &mut XenifyApp, text: &str) {
    let trimmed = text.trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && text.contains("<svg")) {
        app.import_svg_document(text.as_bytes(), "Pasted SVG");
        return;
    }
    let files = xenify_platform::desktop::parse_uri_list(text);
    if trimmed.starts_with("file://") && !files.is_empty() {
        app.import_files(files, None);
        return;
    }
    // Not ours (e.g. text copied elsewhere): nothing to do on the canvas.
    let Ok(env) = serde_json::from_str::<Envelope>(text) else { return };
    if env.format != TAG {
        return;
    }
    match env.clip {
        Clip::Layers { layers, assets } => paste_layers(app, layers, assets),
        Clip::Keyframes { keys } => paste_keys(app, keys),
    }
}

fn paste_layers(app: &mut XenifyApp, layers: Vec<Layer>, assets: Vec<xenify_doc::ImageAsset>) {
    let (cmds, roots) = paste_commands(app, layers, assets);
    let n = roots.len();
    if app.exec("Paste layers", Command::batch(cmds)) {
        app.select_only(roots.first().copied());
        app.extra_selected = roots.into_iter().skip(1).collect();
        app.status = format!("Pasted {n} layer{}.", if n == 1 { "" } else { "s" });
    }
}

/// The commands that add `layers` (with new ids) above the selection, and
/// the pasted layers that aren't inside another pasted group.
fn paste_commands(
    app: &XenifyApp,
    mut layers: Vec<Layer>,
    assets: Vec<xenify_doc::ImageAsset>,
) -> (Vec<Command>, Vec<Id>) {
    let comp = app.comp().clone();
    // Images: reuse the project's copy, or add them (from another project).
    let mut project = app.project().clone();
    let mut cmds = Vec::new();
    for a in assets {
        let (add, id) = xenify_ops::assets::add_image(
            &mut project,
            &a.name,
            &a.mime,
            a.width,
            a.height,
            a.data.to_vec(),
            a.frames.clone(),
        );
        cmds.extend(add);
        for l in &mut layers {
            l.remap_asset(a.id, id);
        }
    }
    // New ids for everything; references inside the clip follow them.
    let mut next = project.next_id;
    let map: HashMap<Id, Id> = layers
        .iter()
        .map(|l| {
            let id = Id(next);
            next += 1;
            (l.id, id)
        })
        .collect();
    let exists = |id: Id| comp.layer(id).is_some();
    // Above the top-most selected layer, else on top.
    let top = app.selected_layers().iter().filter_map(|id| comp.layer_index(*id)).max();
    let first = top.map_or(comp.layers.len(), |i| i + 1);
    let mut roots = Vec::new();
    for (index, mut l) in (first..).zip(layers) {
        l.id = map[&l.id];
        l.meta.parent = l.meta.parent.and_then(|p| map.get(&p).copied().or(exists(p).then_some(p)));
        l.meta.matte = l.meta.matte.and_then(|mut m| {
            m.layer = map.get(&m.layer).copied().or(exists(m.layer).then_some(m.layer))?;
            Some(m)
        });
        // Blends and arrays along a path follow a copied partner.
        for m in &mut l.meta.modifiers {
            if let Some(t) = m.kind.target_mut() {
                *t = map.get(t).copied().unwrap_or(*t);
            }
        }
        // Connectors glue to copied shapes; ends on shapes left behind come
        // loose where they were.
        if let Some(c) = l.meta.connector.as_deref_mut() {
            for end in [&mut c.from, &mut c.to] {
                end.attach = end.attach.and_then(|mut a| {
                    a.layer = *map.get(&a.layer)?;
                    Some(a)
                });
            }
        }
        // Emitters follow a copied source, sprite or target layer.
        if let xenify_doc::LayerKind::Particles(e) = &mut l.kind {
            e.remap(&|id| map.get(&id).copied());
        }
        // Text on a copied path follows the copy.
        if let xenify_doc::LayerKind::Text(t) = &mut l.kind {
            t.path = t.path.and_then(|mut p| {
                p.layer = map.get(&p.layer).copied().or(exists(p.layer).then_some(p.layer))?;
                Some(p)
            });
        }
        if !l.meta.parent.is_some_and(|p| map.values().any(|v| *v == p)) {
            roots.push(l.id);
        }
        cmds.push(Command::AddLayer { comp: comp.id, index: Some(index), layer: l });
    }
    (cmds, roots)
}

fn paste_keys(app: &mut XenifyApp, keys: Vec<ClipKey>) {
    let Some(target) = app.selected else {
        app.status = "Select a layer to paste keyframes onto.".into();
        return;
    };
    let Ok(layer) = app.project().layer(target).cloned() else { return };
    let t0 = app.time;
    let mut skipped = 0;
    let cmds: Vec<Command> = keys
        .into_iter()
        .filter_map(|k| {
            // The property must exist on this layer with the same kind of value.
            let fits = layer.property_kind(k.path).is_ok_and(|kind| kind == k.value.kind());
            if !fits {
                skipped += 1;
                return None;
            }
            Some(Command::SetKeyframe {
                layer: target,
                path: k.path,
                time: t0 + k.time,
                value: k.value,
                easing: Some(k.easing),
            })
        })
        .collect();
    let n = cmds.len();
    if n == 0 {
        app.status = "Those keyframes don't fit this layer's properties.".into();
        return;
    }
    let end = app.comp().settings.duration;
    let late = cmds.iter().filter(|c| matches!(c, Command::SetKeyframe { time, .. } if *time > end)).count();
    if app.exec("Paste keyframes", Command::batch(cmds)) {
        app.status =
            format!(
                "Pasted {n} keyframe{} at the playhead{}.",
                if n == 1 { "" } else { "s" },
                if skipped > 0 { format!(" ({skipped} didn't fit this layer)") } else { String::new() }
            ) + &if late > 0 { format!(" {late} landed after the end of the timeline.") } else { String::new() };
    }
}
