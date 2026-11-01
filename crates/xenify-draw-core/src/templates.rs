//! Slide templates: the themes and layouts, new slides from a layout (built
//! in, or one saved from a slide), applying a theme to a whole deck, the
//! theme role of a layer, and new decks.

use serde_json::{Value as Json, json};
use xenify_doc::templates::{Content, Layout, Role, THEMES, Theme};
use xenify_doc::{Id, Project};
use xenify_ops::Command;

use crate::{Session, color_json, err, id_arg};

pub(crate) fn theme_arg(v: Option<&Json>) -> Option<&'static Theme> {
    v.and_then(Json::as_str).and_then(xenify_doc::templates::theme)
}

pub(crate) fn layout_arg(v: Option<&Json>) -> Option<Layout> {
    let name = v.and_then(Json::as_str)?;
    Layout::ALL.into_iter().find(|l| l.label().eq_ignore_ascii_case(name))
}

/// A slide laid out as `layout` in `theme`, as it looks when its animations are done (for thumbnails).
pub fn layout_preview(theme: &str, layout: &str) -> Option<xenify_anim::Scene> {
    let theme = xenify_doc::templates::theme(theme)?;
    let layout = Layout::ALL.into_iter().find(|l| l.label().eq_ignore_ascii_case(layout))?;
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let built = xenify_doc::templates::build(layout, theme, 1920.0, 1080.0, 10, &Content::default());
    let c = p.comp_mut(comp).ok()?;
    c.settings.width = 1920;
    c.settings.height = 1080;
    c.settings.background = theme.background.0;
    c.settings.duration = built.duration;
    c.layers = built.layers;
    Some(xenify_anim::evaluate_project(&p, comp, (built.duration - 0.01).max(0.0)))
}

impl Session {
    pub(crate) fn templates_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "themes" => Ok(json!({
                "current": xenify_ops::templates::current_theme(self.project()).name,
                "set": self.project().theme,
                "list": THEMES.iter().map(|t| json!({
                    "name": t.name, "background": [color_json(t.background.0), color_json(t.background.1)],
                    "title": color_json(t.title), "body": color_json(t.body), "accent": color_json(t.accent), "accent2": color_json(t.accent2), "font": t.font,
                })).collect::<Vec<_>>(),
            })),
            "layouts" => Ok(json!(Layout::ALL.iter().map(|l| l.label()).collect::<Vec<_>>())),
            "roles" => Ok(json!(Role::ALL.iter().map(|r| json!({ "id": r, "label": r.label() })).collect::<Vec<_>>())),
            "add_layout_slide" => self.add_layout_slide(args),
            "add_saved_layout" => self.add_saved_layout(args),
            "save_layout" => Ok(self.layout_of_slide()),
            "apply_theme" => {
                let Some(theme) = theme_arg(args.get("name")) else { return Some(Err("no such theme".into())) };
                let cmd = xenify_ops::templates::apply_theme(self.project(), theme);
                self.exec("Apply theme", cmd).map(|_| Json::Null)
            }
            "set_role" => self.set_role(args),
            _ => return None,
        })
    }

    /// Where a new slide goes: after the one open.
    fn here(&self) -> Option<usize> {
        xenify_ops::slides::slides_or_main(self.project()).iter().position(|s| s.comp == self.comp)
    }

    fn add_layout_slide(&mut self, args: &Json) -> Result<Json, String> {
        let layout = layout_arg(args.get("layout")).ok_or("no such layout")?;
        let theme = theme_arg(args.get("theme"));
        let (cmd, comp) =
            xenify_ops::templates::add_layout_slide(self.project(), self.here(), layout, theme, &Content::default())
                .map_err(err)?;
        self.exec(&format!("New {} slide", layout.label()), cmd)?;
        self.open_comp(comp)?;
        Ok(self.state())
    }

    /// A new slide copying a saved layout (a one-slide canvas file, as `save_layout` made it).
    fn add_saved_layout(&mut self, args: &Json) -> Result<Json, String> {
        let text = args.get("json").and_then(Json::as_str).ok_or("missing json")?;
        let name = args.get("name").and_then(Json::as_str).unwrap_or("Layout");
        let saved = Project::from_json(text).map_err(err)?;
        let (cmd, comp) =
            xenify_ops::templates::add_saved_layout_slide(self.project(), self.here(), &saved, name).map_err(err)?;
        self.exec(&format!("New “{name}” slide"), cmd)?;
        self.open_comp(comp)?;
        Ok(self.state())
    }

    /// The open slide as a layout to keep: a one-slide project with the pictures it uses.
    fn layout_of_slide(&self) -> Json {
        let Ok(comp) = self.project().comp(self.comp).cloned() else { return Json::Null };
        let mut p = Project::new();
        let used: std::collections::HashSet<Id> = comp.layers.iter().flat_map(|l| l.asset_refs()).collect();
        p.assets = self.project().assets.iter().filter(|a| used.contains(&a.id)).cloned().collect();
        p.theme = self.project().theme.clone();
        let top = comp.layers.iter().map(|l| l.id.0).chain(p.assets.iter().map(|a| a.id.0)).max().unwrap_or(0);
        p.compositions = vec![comp];
        p.reserve_id(Id(top.max(p.compositions[0].id.0)));
        json!(p.to_json())
    }

    /// What Apply Theme colours layer `id` as (`role`: null leaves it alone); it takes the deck's theme now.
    fn set_role(&mut self, args: &Json) -> Result<Json, String> {
        let id = id_arg(args, "id")?;
        let role: Option<Role> = match args.get("role") {
            None | Some(Json::Null) => None,
            Some(v) => Some(serde_json::from_value(v.clone()).map_err(err)?),
        };
        let mut l = self.project().layer(id).map_err(err)?.clone();
        l.meta.role = role;
        let theme = xenify_ops::templates::current_theme(self.project());
        xenify_doc::templates::restyle(&mut l, theme);
        self.exec("Theme role", Command::ReplaceLayer { layer: Box::new(l) }).map(|_| Json::Null)
    }
}
