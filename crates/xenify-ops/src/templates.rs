//! Slide templates as edits: new slides from a layout (built-in or saved),
//! applying a theme to a whole deck, and new decks.

use crate::slides::{copy_layers, slides_or_main};
use crate::{Command, OpError};
use xenify_doc::templates::{self, Content, Layout, THEMES, Theme};
use xenify_doc::{CompSettings, Composition, Id, Project, Slide};

/// The deck's theme (the first built-in one when none is set).
pub fn current_theme(project: &Project) -> &'static Theme {
    project.theme.as_deref().and_then(templates::theme).unwrap_or(&THEMES[1])
}

/// A new slide after slide `after` (at the end if `None`) laid out as
/// `layout` in `theme` (the deck's own when `None`), with `content` (the
/// layout's sample words where empty). One command; returns its composition.
pub fn add_layout_slide(
    project: &Project,
    after: Option<usize>,
    layout: Layout,
    theme: Option<&Theme>,
    content: &Content,
) -> Result<(Command, Id), OpError> {
    let theme = theme.unwrap_or_else(|| current_theme(project));
    let mut slides = slides_or_main(project);
    let at = after.map_or(slides.len(), |i| (i + 1).min(slides.len()));
    let like = slides.get(at.saturating_sub(1)).and_then(|s| project.comp(s.comp).ok());
    let mut settings = like.map(|c| c.settings.clone()).unwrap_or_default();
    let id = Id(project.next_id);
    let built =
        templates::build(layout, theme, settings.width as f64, settings.height as f64, project.next_id + 1, content);
    settings.name = format!("{} {}", layout.label(), slides.len() + 1);
    settings.slices.clear();
    settings.pauses = built.pauses;
    settings.duration = built.duration;
    settings.background = theme.background.0;
    let comp = Composition { id, settings, layers: built.layers };
    slides.insert(at, Slide { transition_in: built.transition, ..Slide::new(id) });
    let mut cmds = vec![Command::AddComposition { index: None, comp }, Command::SetSlides { slides }];
    if project.theme.is_none() {
        cmds.push(Command::SetTheme { theme: Some(theme.name.to_string()) });
    }
    Ok((Command::batch(cmds), id))
}

/// A new slide after slide `after` that copies `layout` (a saved slide:
/// its layers, timing and background; images it uses come along).
pub fn add_saved_layout_slide(
    project: &Project,
    after: Option<usize>,
    saved: &Project,
    name: &str,
) -> Result<(Command, Id), OpError> {
    let src = saved.compositions.first().ok_or_else(|| OpError::Invalid("the layout is empty".into()))?;
    let mut slides = slides_or_main(project);
    let at = after.map_or(slides.len(), |i| (i + 1).min(slides.len()));
    let mut scratch = project.clone();
    let mut cmds = Vec::new();
    // Images: shared with the deck when it has them already.
    let mut remap = Vec::new();
    for a in &saved.assets {
        let (add, id) = crate::assets::add_image(
            &mut scratch,
            &a.name,
            &a.mime,
            a.width,
            a.height,
            a.data.to_vec(),
            a.frames.clone(),
        );
        if let Some(Command::AddAsset { asset }) = add {
            scratch.assets.push(asset.clone());
            cmds.push(Command::AddAsset { asset });
        }
        remap.push((a.id, id));
    }
    let mut next = scratch.next_id;
    let mut alloc = || {
        next += 1;
        Id(next - 1)
    };
    let id = alloc();
    let mut layers = copy_layers(&src.layers, &mut alloc);
    for l in &mut layers {
        for (from, to) in &remap {
            l.remap_asset(*from, *to);
        }
    }
    let like = slides.get(at.saturating_sub(1)).and_then(|s| project.comp(s.comp).ok());
    let mut settings = CompSettings { name: format!("{name} {}", slides.len() + 1), ..src.settings.clone() };
    if let Some(l) = like
        && (l.settings.width, l.settings.height) != (settings.width, settings.height)
    {
        // Laid out for another size: scaled to fit this deck.
        let k =
            (l.settings.width as f64 / settings.width as f64).min(l.settings.height as f64 / settings.height as f64);
        let (dx, dy) = (
            (l.settings.width as f64 - settings.width as f64 * k) / 2.0,
            (l.settings.height as f64 - settings.height as f64 * k) / 2.0,
        );
        for layer in layers.iter_mut().filter(|l| l.meta.parent.is_none()) {
            let p = layer.transform.position.value_at(0.0);
            layer.transform.position = xenify_doc::Vec2::new(p.x * k + dx, p.y * k + dy).into();
            let s = layer.transform.scale.value_at(0.0);
            layer.transform.scale = xenify_doc::Vec2::new(s.x * k, s.y * k).into();
        }
        settings.width = l.settings.width;
        settings.height = l.settings.height;
    }
    let comp = Composition { id, settings, layers };
    let transition_in = slides.get(at.saturating_sub(1)).map(|s| s.transition_in).unwrap_or_default();
    slides.insert(at, Slide { transition_in, ..Slide::new(id) });
    cmds.push(Command::AddComposition { index: None, comp });
    cmds.push(Command::SetSlides { slides });
    Ok((Command::batch(cmds), id))
}

/// Restyles every templated layer of every slide (and symbol) in `theme`,
/// sets its transitions and slide backgrounds, and makes it the deck's
/// theme. One command.
pub fn apply_theme(project: &Project, theme: &Theme) -> Command {
    let mut cmds = Vec::new();
    for c in &project.compositions {
        for l in &c.layers {
            let mut styled = l.clone();
            if templates::restyle(&mut styled, theme) {
                cmds.push(Command::ReplaceLayer { layer: Box::new(styled) });
            }
        }
        if c.layers.iter().any(|l| l.meta.role.is_some()) && c.settings.background != theme.background.0 {
            let settings = CompSettings { background: theme.background.0, ..c.settings.clone() };
            cmds.push(Command::SetCompSettings { comp: c.id, settings });
        }
    }
    if !project.slides.is_empty() {
        let mut slides = project.slides.clone();
        for s in slides.iter_mut().skip(1) {
            s.transition_in = theme.transition();
        }
        cmds.push(Command::SetSlides { slides });
    }
    cmds.push(Command::SetTheme { theme: Some(theme.name.to_string()) });
    Command::batch(cmds)
}

/// A new presentation in `theme`: a `w`×`h` slide per layout in `layouts`
/// (each with its sample words).
pub fn new_deck(theme: &Theme, w: u32, h: u32, layouts: &[Layout]) -> Project {
    let mut p = Project::new();
    let first = p.compositions[0].id;
    p.comp_mut(first).expect("the first composition").settings =
        CompSettings { width: w, height: h, background: theme.background.0, ..Default::default() };
    p.theme = Some(theme.name.to_string());
    for (k, layout) in layouts.iter().enumerate() {
        let (cmd, _) = add_layout_slide(&p, None, *layout, Some(theme), &Content::default()).expect("a slide");
        crate::apply(&mut p, cmd).expect("a slide applies");
        if k == 0 {
            // The deck starts with the built slide, not the empty first one.
            let empty = p.slides.remove(0);
            p.compositions.retain(|c| c.id != empty.comp);
        }
    }
    if let Some(s) = p.slides.first_mut() {
        s.transition_in = Default::default();
    }
    p
}

/// The starter slides of a new deck from the Templates gallery.
pub const STARTER: [Layout; 5] = [Layout::Title, Layout::Bullets, Layout::BigStat, Layout::Quote, Layout::Closing];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use xenify_doc::{LayerKind, Role};

    #[test]
    fn layout_slides_apply_themes_and_undo() {
        let mut p = new_deck(&THEMES[0], 1920, 1080, &STARTER);
        assert_eq!(p.slides.len(), 5);
        assert_eq!(p.compositions.len(), 5, "no empty leftover slide");
        assert_eq!(p.theme.as_deref(), Some("Dark Neon"));
        // A new slide uses the deck's theme.
        let (cmd, comp) = add_layout_slide(&p, Some(0), Layout::Process, None, &Content::default()).unwrap();
        apply(&mut p, cmd).unwrap();
        assert_eq!(p.slides[1].comp, comp);
        assert_eq!(p.comp(comp).unwrap().settings.background, THEMES[0].background.0);

        let before = p.clone();
        let light = templates::theme("Clean Light").unwrap();
        let cmd = apply_theme(&p, light);
        let undo = apply(&mut p, cmd).unwrap();
        let title = p.compositions.iter().flat_map(|c| &c.layers).find(|l| l.meta.role == Some(Role::Title)).unwrap();
        let LayerKind::Text(t) = &title.kind else { panic!() };
        assert_eq!(t.color.value_at(0.0), light.title);
        assert_eq!(p.theme.as_deref(), Some("Clean Light"));
        assert_eq!(p.slides[1].transition_in.kind, light.transition);
        apply(&mut p, undo).unwrap();
        assert_eq!(p, before, "one undo puts it all back");
    }

    #[test]
    fn saved_layouts_copy_in_with_new_ids_and_scale_to_the_deck() {
        let saved = new_deck(&THEMES[2], 1280, 720, &[Layout::Title]);
        let mut p = new_deck(&THEMES[1], 1920, 1080, &[Layout::Title]);
        let (cmd, comp) = add_saved_layout_slide(&p, None, &saved, "Mine").unwrap();
        apply(&mut p, cmd).unwrap();
        let c = p.comp(comp).unwrap();
        assert_eq!((c.settings.width, c.settings.height), (1920, 1080));
        let ids: std::collections::HashSet<Id> =
            p.compositions.iter().flat_map(|c| c.layers.iter().map(|l| l.id)).collect();
        let total: usize = p.compositions.iter().map(|c| c.layers.len()).sum();
        assert_eq!(ids.len(), total, "ids stay unique");
        let bg = c.layers.iter().find(|l| l.meta.role == Some(Role::Background)).unwrap();
        assert!((bg.transform.scale.value_at(0.0).x - 1.5).abs() < 1e-9);
    }
}
