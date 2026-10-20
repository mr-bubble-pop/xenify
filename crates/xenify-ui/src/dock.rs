//! Dockable workspace: the canvas, layers, properties and timeline are tabs
//! that can be dragged into any arrangement (View ▸ Reset Layout restores it).

use crate::app::XenifyApp;
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Panel {
    Canvas,
    Layers,
    Properties,
    Timeline,
    Assistant,
    Graph,
    Color,
    Slides,
    Pens,
    Slices,
    Symbols,
    /// The diagram shape library.
    Library,
    Emoji,
    /// GIPHY search.
    Gifs,
}

/// v2: the roomier default (canvas ¾ wide; properties, colour and the
/// assistant as tabs), given to everyone once.
pub const LAYOUT_KEY: &str = "dock_layout_v2";
/// Set once layouts may have panels closed on purpose (View ▸ Panels).
pub const CLOSABLE_KEY: &str = "dock_closable_v1";

impl Panel {
    pub const ALL: [Panel; 14] = [
        Panel::Canvas,
        Panel::Slides,
        Panel::Layers,
        Panel::Properties,
        Panel::Color,
        Panel::Timeline,
        Panel::Graph,
        Panel::Assistant,
        Panel::Pens,
        Panel::Slices,
        Panel::Symbols,
        Panel::Library,
        Panel::Emoji,
        Panel::Gifs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Panel::Canvas => "Canvas",
            Panel::Layers => "Layers",
            Panel::Properties => "Properties",
            Panel::Timeline => "Timeline",
            Panel::Assistant => "Assistant",
            Panel::Color => "Colour",
            Panel::Graph => "Graph Editor",
            Panel::Slides => "Slides",
            Panel::Pens => "Pens",
            Panel::Slices => "Slices",
            Panel::Symbols => "Symbols",
            Panel::Library => "Shapes",
            Panel::Emoji => "Emoji",
            Panel::Gifs => "GIFs",
        }
    }

    /// Panels that usually share a place with this one (where it reopens).
    fn neighbours(self) -> &'static [Panel] {
        match self {
            Panel::Timeline | Panel::Graph => &[Panel::Timeline, Panel::Graph],
            Panel::Layers | Panel::Properties => &[Panel::Layers, Panel::Properties, Panel::Color],
            Panel::Color | Panel::Assistant | Panel::Pens => &[Panel::Color, Panel::Assistant, Panel::Pens],
            Panel::Slices | Panel::Symbols | Panel::Library | Panel::Emoji | Panel::Gifs => {
                &[Panel::Layers, Panel::Properties]
            }
            Panel::Canvas | Panel::Slides => &[],
        }
    }
}

/// Whether `panel` is open anywhere in the layout.
pub fn is_open(app: &XenifyApp, panel: Panel) -> bool {
    app.dock.as_ref().is_some_and(|d| d.find_tab(&panel).is_some())
}

/// Opens `panel` if it's closed and brings it to the front of its tab group.
pub fn reveal(app: &mut XenifyApp, panel: Panel) {
    if !is_open(app, panel) {
        toggle(app, panel);
    }
    if let Some(dock) = app.dock.as_mut()
        && let Some(path) = dock.find_tab(&panel)
    {
        let _ = dock.set_active_tab(path);
    }
}

/// View ▸ Panels: closes `panel`, or reopens it beside the panels it's usually with.
pub fn toggle(app: &mut XenifyApp, panel: Panel) {
    let Some(dock) = app.dock.as_mut() else { return };
    if let Some(path) = dock.find_tab(&panel) {
        if panel != Panel::Canvas {
            dock.remove_tab(path);
        }
        return;
    }
    // Slides open as a column left of the canvas, like a slide sorter.
    if panel == Panel::Slides
        && let Some(c) = dock.find_tab(&Panel::Canvas)
    {
        let path = egui_dock::NodePath { surface: c.surface, node: c.node };
        dock.split(path, egui_dock::Split::Left, 0.16, egui_dock::Node::leaf(Panel::Slides));
        return;
    }
    match panel.neighbours().iter().find_map(|n| dock.find_tab(n)) {
        Some(t) => {
            dock.set_focused_node_and_surface(egui_dock::NodePath { surface: t.surface, node: t.node });
            dock.push_to_focused_leaf(panel);
        }
        None => dock.push_to_first_leaf(panel),
    }
}

/// The layout saved last session; else the default.
pub fn saved_layout(storage: Option<&dyn eframe::Storage>) -> DockState<Panel> {
    let saved: Option<DockState<Panel>> = storage.and_then(|s| eframe::get_value(s, LAYOUT_KEY));
    let closable: bool = storage.and_then(|s| eframe::get_value(s, CLOSABLE_KEY)).unwrap_or(false);
    if closable && let Some(d) = saved.clone().filter(|d| d.find_tab(&Panel::Canvas).is_some()) {
        return d;
    }
    let complete = |d: &DockState<Panel>| {
        [Panel::Canvas, Panel::Layers, Panel::Properties, Panel::Timeline, Panel::Assistant]
            .iter()
            .all(|p| d.find_tab(p).is_some())
    };
    let mut dock = saved.filter(complete).unwrap_or_else(default_layout);
    // Layouts saved before the graph editor existed: put it beside the timeline.
    if dock.find_tab(&Panel::Graph).is_none()
        && let Some(t) = dock.find_tab(&Panel::Timeline)
    {
        dock.set_focused_node_and_surface(egui_dock::NodePath { surface: t.surface, node: t.node });
        dock.push_to_focused_leaf(Panel::Graph);
        if let Some(t) = dock.find_tab(&Panel::Timeline) {
            let _ = dock.set_active_tab(t);
        }
    }
    // Layouts saved before the colour panel existed: put it beside the layers.
    if dock.find_tab(&Panel::Color).is_none()
        && let Some(l) = dock.find_tab(&Panel::Layers)
    {
        dock.set_focused_node_and_surface(egui_dock::NodePath { surface: l.surface, node: l.node });
        dock.push_to_focused_leaf(Panel::Color);
        if let Some(l) = dock.find_tab(&Panel::Layers) {
            let _ = dock.set_active_tab(l);
        }
    }
    dock
}

/// The canvas gets most of the window: layers top right, then properties
/// with colour and the assistant as tabs behind it; timeline along the bottom.
pub fn default_layout() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Canvas]);
    let tree = dock.main_surface_mut();
    let [top, _timeline] = tree.split_below(NodeIndex::root(), 0.7, vec![Panel::Timeline, Panel::Graph]);
    let [_canvas, right] = tree.split_right(top, 0.75, vec![Panel::Layers]);
    tree.split_below(right, 0.34, vec![Panel::Properties, Panel::Color, Panel::Assistant]);
    dock
}

struct Tabs<'a> {
    app: &'a mut XenifyApp,
    frame: &'a eframe::Frame,
}

impl TabViewer for Tabs<'_> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        // The same names as View › Panels.
        match tab {
            Panel::Canvas => format!("Canvas — {}", self.app.comp().settings.name).into(),
            other => other.label().into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        let app = &mut *self.app;
        match tab {
            Panel::Canvas => {
                crate::symbols::editing_banner(app, ui);
                // Evaluated here, at the moment of drawing, so the render cache key
                // (revision, time) always matches the scene it renders.
                let scene = xenify_anim::evaluate_project(app.project(), app.comp, app.time);
                let mut viewport = std::mem::take(&mut app.viewport);
                let response = viewport.show(app, ui, self.frame, &scene);
                app.viewport = viewport;
                crate::canvas_menu::show(app, &response);
            }
            Panel::Layers => crate::panels::layers(app, ui),
            Panel::Properties => {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| crate::panels::properties(app, ui));
            }
            Panel::Timeline => crate::timeline::show(app, ui),
            Panel::Assistant => crate::assistant::show(app, ui),
            Panel::Color => {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| crate::colors::show(app, ui));
            }
            Panel::Graph => crate::graph::show(app, ui),
            Panel::Slides => crate::slides_panel::show(app, ui),
            Panel::Pens => crate::pens_panel::show(app, ui),
            Panel::Slices => crate::slices_panel::show(app, ui),
            Panel::Symbols => crate::symbols::show(app, ui),
            Panel::Library => crate::shape_library::show(app, ui),
            Panel::Emoji => crate::emoji_panel::show(app, ui),
            Panel::Gifs => crate::giphy_panel::show(app, ui),
        }
    }

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(("xenify-panel", *tab))
    }

    fn is_closeable(&self, tab: &Panel) -> bool {
        *tab != Panel::Canvas
    }

    fn scroll_bars(&self, _tab: &Panel) -> [bool; 2] {
        // Every panel manages its own scrolling.
        [false, false]
    }

    fn tab_style_override(&self, tab: &Panel, global: &egui_dock::TabStyle) -> Option<egui_dock::TabStyle> {
        (*tab == Panel::Canvas).then(|| {
            let mut style = global.clone();
            style.tab_body.inner_margin = egui::Margin::ZERO;
            style
        })
    }

    fn allowed_in_windows(&self, _tab: &mut Panel) -> bool {
        false
    }
}

pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, frame: &eframe::Frame) {
    let mut dock = app.dock.take().unwrap_or_else(default_layout);
    let style = crate::theme::dock_style(ui);
    DockArea::new(&mut dock)
        .style(style)
        // Panels close from their tab (View › Panels brings them back); not the canvas.
        .show_close_buttons(true)
        .show_add_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, &mut Tabs { app, frame });
    app.dock = Some(dock);
    // Floating, so it shows whichever tab is in front (it's opened from the timeline and properties).
    crate::timeline::curve_window(app, ui.ctx());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mem(HashMap<String, String>);

    impl eframe::Storage for Mem {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    fn json(d: &DockState<Panel>) -> String {
        serde_json::to_string(d).unwrap()
    }

    #[test]
    fn layout_survives_a_restart_and_bad_data_falls_back() {
        let mut storage = Mem::default();
        assert_eq!(json(&saved_layout(Some(&storage))), json(&default_layout()));
        // Every panel as tabs of one area: clearly not the default.
        let dock = DockState::new(vec![
            Panel::Canvas,
            Panel::Layers,
            Panel::Properties,
            Panel::Timeline,
            Panel::Assistant,
            Panel::Graph,
            Panel::Color,
        ]);
        eframe::set_value(&mut storage, LAYOUT_KEY, &dock);
        assert_eq!(json(&saved_layout(Some(&storage))), json(&dock));
        // A layout missing a panel (or unreadable) falls back to the default.
        eframe::set_value(&mut storage, LAYOUT_KEY, &DockState::new(vec![Panel::Canvas]));
        assert_eq!(json(&saved_layout(Some(&storage))), json(&default_layout()));
        // An older layout without the graph editor gets it added, beside the timeline.
        let old =
            DockState::new(vec![Panel::Canvas, Panel::Layers, Panel::Properties, Panel::Timeline, Panel::Assistant]);
        eframe::set_value(&mut storage, LAYOUT_KEY, &old);
        let upgraded = saved_layout(Some(&storage));
        let (g, t) = (upgraded.find_tab(&Panel::Graph).unwrap(), upgraded.find_tab(&Panel::Timeline).unwrap());
        assert_eq!((g.surface, g.node), (t.surface, t.node));
        // …and the colour panel beside the layers.
        let (c, l) = (upgraded.find_tab(&Panel::Color).unwrap(), upgraded.find_tab(&Panel::Layers).unwrap());
        assert_eq!((c.surface, c.node), (l.surface, l.node));
        eframe::Storage::set_string(&mut storage, LAYOUT_KEY, "garbage".into());
        assert_eq!(json(&saved_layout(Some(&storage))), json(&default_layout()));
    }
}
