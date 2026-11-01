//! Editing together: a [`Change`] is one undo step's commands, each with
//! what others need to fit their own changes around it, and
//! [`Transform`] for them.
//!
//! Layers are found by id, so most edits never clash. Lists change places:
//! layers in a composition (adding, removing, reordering) and shapes in a
//! layer (and the `shapes.N.…` properties that point into them). Values
//! set as a whole go to whoever's change comes later; replacing a whole
//! layer or property first, then someone's finer edit, keeps that edit on
//! top. Anything aimed at a layer someone removed is dropped.
//!
//! When two changes really can't fit (both reorder the same layers), the
//! app falls back to the server's copy.

use crate::{Command, apply};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
pub use xenify_collab::Transform;
use xenify_doc::{Id, Project};

/// One undo step as sent to the others: applied all together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub cmd: Command,
    #[serde(default, skip_serializing_if = "Ctx::is_empty")]
    pub ctx: Ctx,
}

/// What a command doesn't say but others need.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ctx {
    /// The composition a removed or moved layer is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comp: Option<Id>,
    /// Where a removed or moved layer was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<usize>,
    /// A picture's file before this change set it (kept here only, never sent): two people painting one
    /// picture at once get both their strokes (see [`merge_pictures`]).
    #[serde(skip)]
    pub before: Option<std::sync::Arc<[u8]>>,
}

impl Ctx {
    fn is_empty(&self) -> bool {
        self.comp.is_none() && self.from.is_none()
    }
}

fn flatten(cmd: Command, out: &mut Vec<Command>) {
    match cmd {
        Command::Batch { commands } => commands.into_iter().for_each(|c| flatten(c, out)),
        c => out.push(c),
    }
}

impl Change {
    /// `cmd` applied to a copy of `project`: the change and the project
    /// after it (or the error).
    pub fn make(project: &Project, cmd: Command) -> Result<(Change, Project), crate::OpError> {
        let mut p = project.clone();
        let (change, _) = apply_recorded(&mut p, cmd)?;
        Ok((change, p))
    }

    /// The command to apply.
    pub fn command(&self) -> Command {
        match self.parts.as_slice() {
            [one] => one.cmd.clone(),
            many => Command::Batch { commands: many.iter().map(|p| p.cmd.clone()).collect() },
        }
    }
}

/// Applies `cmd` to `project` part by part, working out what others need
/// to know about each; all or nothing, like a batch. Returns the change and
/// the command that undoes it.
pub fn apply_recorded(project: &mut Project, cmd: Command) -> Result<(Change, Command), crate::OpError> {
    let mut cmds = Vec::new();
    flatten(cmd, &mut cmds);
    let mut parts = Vec::new();
    let mut undo: Vec<Command> = Vec::new();
    for mut c in cmds {
        let before = match &c {
            Command::RemoveLayer { layer } | Command::MoveLayer { layer, .. } => project.locate_layer(*layer).ok(),
            _ => None,
        };
        let picture_before = match &c {
            Command::SetAsset { asset } => project.asset(asset.id).map(|a| a.data.clone()),
            _ => None,
        };
        // "On top" and "at the end" become real places, so two at once keep
        // one order everywhere.
        match &mut c {
            Command::AddLayer { comp, index, .. } if index.is_none() => {
                *index = project.comp(*comp).ok().map(|c| c.layers.len())
            }
            Command::AddShape { layer, index, .. } if index.is_none() => {
                *index = Some(project.layer(*layer).map_or(0, |l| l.shapes().len()))
            }
            _ => {}
        }
        match apply(project, c.clone()) {
            Ok(inv) => undo.push(inv),
            Err(e) => {
                for inv in undo.into_iter().rev() {
                    let _ = apply(project, inv);
                }
                return Err(e);
            }
        }
        let mut ctx = match before {
            Some((comp, from)) => Ctx { comp: Some(comp), from: Some(from), before: None },
            None => Ctx::default(),
        };
        ctx.before = picture_before;
        parts.push(Part { cmd: c, ctx });
    }
    undo.reverse();
    let inverse = if undo.len() == 1 { undo.pop().unwrap() } else { Command::Batch { commands: undo } };
    Ok((Change { parts }, inverse))
}

/// One of my own commands (an undo step) adjusted to come after someone
/// else's change; None when nothing is left of it.
pub fn adjust(mine: Command, over: &Change) -> Option<Command> {
    let mut cmds = Vec::new();
    flatten(mine, &mut cmds);
    let mine = Change { parts: cmds.into_iter().map(|cmd| Part { cmd, ctx: Ctx::default() }).collect() };
    Change::transform(&mine, over, true).first().map(|c| c.command())
}

impl Transform for Change {
    fn transform(a: &Change, b: &Change, later: bool) -> Vec<Change> {
        let parts = if later {
            xenify_collab::transform(&a.parts, &b.parts).0
        } else {
            xenify_collab::transform(&b.parts, &a.parts).1
        };
        if parts.is_empty() { Vec::new() } else { vec![Change { parts }] }
    }
}

// ---- One command against another ----------------------------------------

fn json(c: &Command) -> Json {
    serde_json::to_value(c).unwrap_or(Json::Null)
}

fn id_field(v: &Json, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64())
}

/// The layer a command works on.
fn layer_of(c: &Command) -> Option<Id> {
    match c {
        Command::ReplaceLayer { layer } => Some(layer.id),
        Command::AddLayer { layer, .. } => Some(layer.id),
        _ => id_field(&json(c), "layer").map(Id),
    }
}

/// What a command sets as a whole: two with the same key, the later wins.
/// Keyframes are keyed by layer, property and time.
fn key(c: &Command) -> Option<String> {
    use Command::*;
    let v = json(c);
    let op = v.get("op")?.as_str()?.to_string();
    let l = id_field(&v, "layer").unwrap_or(0);
    let t = |x: f64| format!("{x:.6}");
    Some(match c {
        AddLayer { .. }
        | RemoveLayer { .. }
        | AddShape { .. }
        | RemoveShape { .. }
        | AddComposition { .. }
        | Batch { .. } => {
            return None;
        }
        RemoveComposition { comp } => format!("rmcomp:{comp}"),
        AddAsset { asset } | SetAsset { asset } => format!("asset:{}", asset.id),
        RemoveAsset { asset } => format!("asset:{asset}"),
        AddMedia { media } | SetMediaAsset { media } => format!("media:{}", media.id),
        RemoveMedia { media } => format!("media:{media}"),
        MoveLayer { layer, .. } => format!("move:{layer}"),
        SetShape { layer, index, .. } => format!("shape:{layer}:{index}"),
        SetKeyframe { layer, path, time, .. }
        | RemoveKeyframe { layer, path, time }
        | SetValue { layer, path, time, .. } => {
            format!("kf:{layer}:{}:{}", String::from(*path), t(*time))
        }
        // A keyframe's easing is separate from its value.
        SetEasing { layer, path, time, .. } => format!("ease:{layer}:{}:{}", String::from(*path), t(*time)),
        MoveKeyframe { layer, path, from, .. } => format!("kfmove:{layer}:{}:{}", String::from(*path), t(*from)),
        SetProperty { layer, path, .. } => format!("prop:{layer}:{}", String::from(*path)),
        SetCompSettings { comp, .. } => format!("comp:{comp}"),
        ReplaceLayer { layer } => format!("layer:{}", layer.id),
        _ => format!("{op}:{l}"),
    })
}

/// What a whole-thing command covers, for finer ones inside it: a whole
/// layer, or a whole property.
fn covers(whole: &Command, fine: &Command) -> bool {
    use Command::*;
    match whole {
        ReplaceLayer { layer } => {
            layer_of(fine) == Some(layer.id) && !matches!(fine, ReplaceLayer { .. } | RemoveLayer { .. })
        }
        SetProperty { layer, path, .. } => match fine {
            SetKeyframe { layer: l, path: p, .. }
            | RemoveKeyframe { layer: l, path: p, .. }
            | MoveKeyframe { layer: l, path: p, .. }
            | SetEasing { layer: l, path: p, .. }
            | SetValue { layer: l, path: p, .. } => l == layer && p == path,
            _ => false,
        },
        _ => false,
    }
}

/// One step of a change to a list: an insert or a removal at a place. A
/// move is a removal then an insert (counted without the moved item).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edit {
    Ins(usize),
    Rem(usize),
}

impl Transform for Edit {
    fn transform(a: &Edit, b: &Edit, later: bool) -> Vec<Edit> {
        use Edit::*;
        vec![match (*a, *b) {
            (Ins(i), Ins(j)) => Ins(if i > j || (i == j && later) { i + 1 } else { i }),
            (Ins(i), Rem(j)) => Ins(if i > j { i - 1 } else { i }),
            (Rem(i), Ins(j)) => Rem(if i >= j { i + 1 } else { i }),
            (Rem(i), Rem(j)) if i == j => return Vec::new(),
            (Rem(i), Rem(j)) => Rem(if i > j { i - 1 } else { i }),
        }]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum List {
    Layers(Id),
    Shapes(Id),
}

/// The list a part changes, and how.
fn list_edits(p: &Part) -> Option<(List, Vec<Edit>)> {
    use Command::*;
    Some(match &p.cmd {
        AddLayer { comp, index: Some(i), .. } => (List::Layers(*comp), vec![Edit::Ins(*i)]),
        RemoveLayer { .. } => (List::Layers(p.ctx.comp?), vec![Edit::Rem(p.ctx.from?)]),
        MoveLayer { index, .. } => (List::Layers(p.ctx.comp?), vec![Edit::Rem(p.ctx.from?), Edit::Ins(*index)]),
        AddShape { layer, index: Some(i), .. } => (List::Shapes(*layer), vec![Edit::Ins(*i)]),
        RemoveShape { layer, index } => (List::Shapes(*layer), vec![Edit::Rem(*index)]),
        _ => return None,
    })
}

/// `a`'s list steps after `b`'s.
fn edits_over(a: &[Edit], b: &[Edit], later: bool) -> Vec<Edit> {
    if later { xenify_collab::transform(a, b).0 } else { xenify_collab::transform(b, a).1 }
}

/// An existing item's place after `b`'s steps; None: it was removed.
fn point(i: usize, b: &[Edit]) -> Option<usize> {
    let mut i = i;
    for e in b {
        match *e {
            Edit::Ins(j) if i >= j => i += 1,
            Edit::Rem(j) if i == j => return None,
            Edit::Rem(j) if i > j => i -= 1,
            _ => {}
        }
    }
    Some(i)
}

/// A `shapes.N.…` property path after shape list edit `e`.
fn shape_path(path: xenify_doc::PropPath, e: &[Edit]) -> Option<xenify_doc::PropPath> {
    match path {
        xenify_doc::PropPath::Shape { index, field } => {
            point(index, e).map(|index| xenify_doc::PropPath::Shape { index, field })
        }
        p => Some(p),
    }
}

fn with_path(c: &Command, f: impl Fn(xenify_doc::PropPath) -> Option<xenify_doc::PropPath>) -> Option<Command> {
    use Command::*;
    let mut c = c.clone();
    match &mut c {
        SetProperty { path, .. }
        | SetKeyframe { path, .. }
        | RemoveKeyframe { path, .. }
        | MoveKeyframe { path, .. }
        | SetEasing { path, .. }
        | SetValue { path, .. } => *path = f(*path)?,
        _ => {}
    }
    Some(c)
}

impl Transform for Part {
    fn transform(a: &Part, b: &Part, later: bool) -> Vec<Part> {
        use Command::*;
        let keep = || vec![a.clone()];
        // Aimed at a layer that's gone.
        if let RemoveLayer { layer } = &b.cmd
            && layer_of(&a.cmd) == Some(*layer)
        {
            return Vec::new();
        }
        if let RemoveComposition { comp } = &b.cmd {
            let v = json(&a.cmd);
            if id_field(&v, "comp") == Some(comp.0) {
                return Vec::new();
            }
        }
        // A whole layer or property, and an edit inside it.
        if covers(&a.cmd, &b.cmd) {
            return if later {
                keep()
            } else {
                // Ours came first: theirs goes back on top.
                vec![a.clone(), b.clone()]
            };
        }
        if covers(&b.cmd, &a.cmd) {
            return if later { keep() } else { Vec::new() };
        }
        // The same thing set twice.
        if let (Some(ka), Some(kb)) = (key(&a.cmd), key(&b.cmd))
            && ka == kb
        {
            return if later {
                let mut a = a.clone();
                // A moved layer or keyframe moves from where the other move
                // put it.
                match (&mut a.cmd, &b.cmd) {
                    (MoveLayer { .. }, MoveLayer { index, .. }) => a.ctx.from = Some(*index),
                    (MoveKeyframe { from, .. }, MoveKeyframe { to, .. }) => *from = *to,
                    // Both painted one picture: theirs, with the pixels ours changed.
                    (SetAsset { asset: mine }, SetAsset { asset: theirs }) => {
                        if let Some(before) = &a.ctx.before
                            && let Some(data) = merge_pictures(before, &theirs.data, &mine.data)
                        {
                            *mine = xenify_doc::ImageAsset::new(
                                mine.id,
                                mine.name.clone(),
                                "image/png",
                                theirs.width,
                                theirs.height,
                                data,
                            );
                            a.ctx.before = Some(theirs.data.clone());
                        }
                    }
                    _ => {}
                }
                vec![a]
            } else {
                Vec::new()
            };
        }
        // A keyframe someone moved: follow it; one someone removed: don't move it.
        let mut a = a.clone();
        if let (MoveKeyframe { layer, path, from, .. }, RemoveKeyframe { layer: l, path: p, time }) = (&a.cmd, &b.cmd)
            && l == layer
            && p == path
            && (time - from).abs() < 1e-9
        {
            return Vec::new();
        }
        if let MoveKeyframe { layer, path, from, to } = &b.cmd {
            let same = |l: &Id, p: &xenify_doc::PropPath, t: &f64| l == layer && p == path && (t - from).abs() < 1e-9;
            match &mut a.cmd {
                SetKeyframe { layer: l, path: p, time, .. }
                | RemoveKeyframe { layer: l, path: p, time }
                | SetEasing { layer: l, path: p, time, .. }
                    if same(l, p, time) =>
                {
                    *time = *to
                }
                _ => {}
            }
        }
        // Lists that changed places.
        let Some((list, e)) = list_edits(b) else { return vec![a] };
        match (&mut a.cmd, list) {
            (AddLayer { comp, index: Some(i), .. }, List::Layers(c)) if *comp == c => {
                let [Edit::Ins(n)] = edits_over(&[Edit::Ins(*i)], &e, later)[..] else { return Vec::new() };
                *i = n;
            }
            (MoveLayer { index, .. }, List::Layers(c)) if a.ctx.comp == Some(c) => {
                let Some(from) = a.ctx.from else { return vec![a] };
                let [Edit::Rem(f), Edit::Ins(t)] = edits_over(&[Edit::Rem(from), Edit::Ins(*index)], &e, later)[..]
                else {
                    return Vec::new();
                };
                *index = t;
                a.ctx.from = Some(f);
            }
            (RemoveLayer { layer }, List::Layers(c)) if a.ctx.comp == Some(c) => {
                a.ctx.from = match &b.cmd {
                    // Someone moved the layer we removed: it was there.
                    MoveLayer { layer: moved, index } if moved == layer => Some(*index),
                    _ => a.ctx.from.and_then(|f| point(f, &e)),
                };
            }
            (AddShape { layer, index: Some(i), .. }, List::Shapes(l)) if *layer == l => {
                let [Edit::Ins(n)] = edits_over(&[Edit::Ins(*i)], &e, later)[..] else { return Vec::new() };
                *i = n;
            }
            (RemoveShape { layer, index } | SetShape { layer, index, .. }, List::Shapes(l)) if *layer == l => {
                let Some(i) = point(*index, &e) else { return Vec::new() };
                *index = i;
            }
            (cmd, List::Shapes(l)) if layer_of(cmd) == Some(l) => {
                let Some(c) = with_path(cmd, |p| shape_path(p, &e)) else { return Vec::new() };
                *cmd = c;
            }
            _ => {}
        }
        vec![a]
    }
}

/// Whether two projects hold the same thing, as saved (each window's next
/// new id is its own, so it doesn't count).
pub fn same_content(x: &Project, y: &Project) -> bool {
    let saved = |p: &Project| {
        let mut v = serde_json::to_value(p).ok()?;
        v.as_object_mut()?.remove("next_id");
        Some(v)
    };
    saved(x) == saved(y)
}

#[cfg(test)]
mod tests;

/// Two changes to one picture made from the same `before`: `theirs`, with every pixel `mine` changed taken
/// from `mine`. `None` when they can't be merged (sizes differ, or a file can't be read).
pub fn merge_pictures(before: &[u8], theirs: &[u8], mine: &[u8]) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let read = |d: &[u8]| image::load_from_memory(d).ok().map(|i| i.into_rgba8());
    let (b, t, m) = (read(before)?, read(theirs)?, read(mine)?);
    if b.dimensions() != t.dimensions() || b.dimensions() != m.dimensions() {
        return None;
    }
    let (w, h) = t.dimensions();
    let mut out = t.into_raw();
    for ((o, b), m) in
        out.as_chunks_mut::<4>().0.iter_mut().zip(b.as_raw().as_chunks::<4>().0).zip(m.as_raw().as_chunks::<4>().0)
    {
        if b != m {
            o.copy_from_slice(m);
        }
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Up,
    )
    .write_image(&out, w, h, image::ExtendedColorType::Rgba8)
    .ok()?;
    Some(png)
}
