//! The menu bar. Items run [`Action`]s, so their shortcut text and enabled
//! state come from the keymap and [`crate::actions::enabled`].

use crate::actions;
use crate::app::{XenifyApp, Tool};
use crate::dialogs::Dialog;
use crate::dock::Panel;
use crate::keymap::Action;

/// A menu entry for `action` (label, shortcut, greyed out when it can't run).
pub(crate) fn item(app: &mut XenifyApp, ui: &mut egui::Ui, action: Action) -> egui::Response {
    item_labelled(app, ui, action, &action.label())
}

fn item_labelled(app: &mut XenifyApp, ui: &mut egui::Ui, action: Action, label: &str) -> egui::Response {
    let button = egui::Button::new(label).shortcut_text(app.keymap.text(action));
    let r = ui.add_enabled(actions::enabled(app, action), button);
    if r.clicked() {
        let ctx = ui.ctx().clone();
        actions::run(app, &ctx, action);
        ui.close();
    }
    r
}

pub fn menu_bar(app: &mut XenifyApp, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        let mut tops = Vec::new();
        tops.push(ui.menu_button("File", |ui| file_menu(app, ui)).response);
        tops.push(ui.menu_button("Edit", |ui| edit_menu(app, ui)).response);
        tops.push(ui.menu_button("Layer", |ui| layer_menu(app, ui)).response);
        tops.push(
            ui.menu_button("Canvas", |ui| crate::panels::composition_menu(app, ui))
                .response
                .on_hover_text("The canvas you're working on (its size, length and frame rate), and symbols"),
        );
        tops.push(ui.menu_button("Slide", |ui| slide_menu(app, ui)).response);
        tops.push(ui.menu_button("AI", |ui| ai_menu(app, ui)).response);
        tops.push(ui.menu_button("View", |ui| view_menu(app, ui)).response);
        tops.push(
            ui.menu_button("Help", |ui| {
                item(app, ui, Action::ShortcutsHelp);
                if ui.button("Showcase").on_hover_text("A presentation of what the app does, made in the app").clicked()
                {
                    crate::showcase::open(app);
                    ui.close();
                }
                if ui
                    .button("Credits & Licences")
                    .on_hover_text("Where the fonts and artwork came from, and their licences")
                    .clicked()
                {
                    crate::credits::open(ui.ctx());
                    ui.close();
                }
            })
            .response,
        );
        switch_on_hover(ui.ctx(), &tops);
    });
    crate::credits::window(ui.ctx());
    crate::effects_gallery::window(app, ui.ctx());
}

/// Like desktop menu bars: while one menu is open, pointing at another
/// title opens that one instead.
fn switch_on_hover(ctx: &egui::Context, tops: &[egui::Response]) {
    let ids: Vec<egui::Id> = tops.iter().map(egui::Popup::default_response_id).collect();
    let Some(open) = ids.iter().position(|id| egui::Popup::is_id_open(ctx, *id)) else { return };
    if let Some(hovered) = tops.iter().position(|r| r.hovered())
        && hovered != open
    {
        egui::Popup::close_id(ctx, ids[open]);
        egui::Popup::open_id(ctx, ids[hovered]);
    }
}

/// Layer: new layers, grouping, paths, tracing, cropping, masks, effects.
fn layer_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.menu_button("New", |ui| crate::panels::new_layer_items(app, ui));
    if ui
        .button("New Effect…")
        .on_hover_text("Confetti, fire, smoke, explosions, water, shatter… with moving previews")
        .clicked()
    {
        crate::effects_gallery::open(ui.ctx());
        ui.close();
    }
    ui.separator();
    item(app, ui, Action::Group).on_hover_text("Put the selected layers in a group (select several with Shift+click)");
    item(app, ui, Action::Ungroup).on_hover_text("Take the layers out of the selected group");
    item(app, ui, Action::ClipMask).on_hover_text("The top-most selected shape clips the others");
    item(app, ui, Action::ReleaseClipMask);
    ui.separator();
    ui.add_enabled_ui(crate::path_menu::has_shapes(app), |ui| {
        path_menu(app, ui).on_disabled_hover_text("Select a shape first");
    });
    let image = app
        .selected
        .and_then(|id| app.project().layer(id).ok())
        .filter(|l| matches!(l.kind, xenify_doc::LayerKind::Image { .. }))
        .map(|l| l.id);
    if ui
        .add_enabled(image.is_some(), egui::Button::new("Trace Image to Vectors…"))
        .on_hover_text("Turn the selected picture into editable shapes (no AI needed)")
        .on_disabled_hover_text("Select a picture layer (or use File › Import and Trace Image for a picture file)")
        .clicked()
    {
        app.dialog = Some(crate::dialogs::Dialog::TraceLayer { layer: image.unwrap(), opts: Default::default() });
        ui.close();
    }
    if ui
        .add_enabled(!app.selected_layers().is_empty(), egui::Button::new("Crop Selection…"))
        .on_hover_text("Drag the edges in to crop the selected layers or group, then Apply")
        .clicked()
    {
        crate::viewport::start_crop(app, true);
        ui.close();
    }
    ui.separator();
    crate::masks_ui::menu(app, ui);
    crate::effects_ui::menu(app, ui);
    crate::modifiers_ui::menu(app, ui);
}

/// Layer › Path (also in the canvas's right-click menu).
pub(crate) fn path_menu(app: &mut XenifyApp, ui: &mut egui::Ui) -> egui::Response {
    ui.menu_button("Path", |ui| {
        if ui.button("Convert to Path").on_hover_text("Rectangles and ellipses become editable points").clicked() {
            crate::path_menu::convert_selected(app);
            ui.close();
        }
        if ui
            .button("Simplify")
            .on_hover_text("Fewer points, same look (great after drawing with the pencil)")
            .clicked()
        {
            crate::path_menu::simplify_selected(app);
            ui.close();
        }
        if ui.button("Subdivide").on_hover_text("A new point halfway along every edge").clicked() {
            crate::path_menu::subdivide_selected(app);
            ui.close();
        }
        if ui.button("Outline Stroke").on_hover_text("The stroke becomes a filled shape of the same look").clicked() {
            crate::path_menu::outline_selected(app);
            ui.close();
        }
        if ui
            .button("Offset Path")
            .on_hover_text("Grow or shrink shapes by a distance (a modifier: change or apply it in Properties)")
            .clicked()
        {
            crate::path_menu::offset_selected(app);
            ui.close();
        }
        ui.separator();
        if ui.button("Edit Points (A)").clicked() {
            app.set_tool(crate::app::Tool::Node);
            ui.close();
        }
        if ui.button("Knife (C)").on_hover_text("Drag across shapes to cut them in two").clicked() {
            app.set_tool(crate::app::Tool::Knife);
            ui.close();
        }
    })
    .response
}

fn file_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    use crate::session::{Pending, guard};
    item(app, ui, Action::New);
    item(app, ui, Action::Open);
    let recent = app.session.recent.clone();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button("Open Recent", |ui| {
            for path in recent {
                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into());
                if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                    guard(app, Pending::OpenPath(path));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Clear list").clicked() {
                app.session.recent.clear();
                ui.close();
            }
        });
    });
    if ui
        .button("New Diagram")
        .on_hover_text("A new document for a flowchart, network or architecture diagram, with the diagram tools")
        .clicked()
    {
        let settings = xenify_ops::diagram::diagram_settings("Diagram", 1600, 1000);
        crate::new_dialog::create(app, settings, crate::new_dialog::Source::Blank);
        crate::diagram_ui::entered(app);
        ui.close();
    }
    if ui.button("Open Demo").clicked() {
        guard(app, Pending::OpenDemo);
        ui.close();
    }
    ui.separator();
    item(app, ui, Action::Save);
    item(app, ui, Action::SaveAs);
    item(app, ui, Action::CloseTab);
    ui.separator();
    item(app, ui, Action::Import);
    if ui
        .button("Import and Trace Image…")
        .on_hover_text(
            "Turn a PNG/JPG file into vector shapes (Layer › Trace Image to Vectors does a picture already here)",
        )
        .clicked()
    {
        app.dialog = Some(Dialog::image_trace(app));
        ui.close();
    }
    ui.separator();
    item(app, ui, Action::Export);
    let slices = app.comp().settings.slices.len();
    if ui
        .add_enabled(slices > 0, egui::Button::new("Export Slices…"))
        .on_hover_text("Each slice (Slice tool, Shift+C) as its own image or video")
        .on_disabled_hover_text("Draw slices with the Slice tool (Shift+C) first")
        .clicked()
    {
        app.dialog = Some(Dialog::export_slices(app));
        ui.close();
    }
    ui.separator();
    item(app, ui, Action::Settings);
    item(app, ui, Action::Print);
    ui.separator();
    item(app, ui, Action::Quit);
}

fn edit_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    let undo = app.editor.undo_label().map_or("Undo".into(), |l| format!("Undo {l}"));
    item_labelled(app, ui, Action::Undo, &undo);
    let redo = app.editor.redo_label().map_or("Redo".into(), |l| format!("Redo {l}"));
    item_labelled(app, ui, Action::Redo, &redo);
    ui.separator();
    for a in [Action::Cut, Action::Copy, Action::Paste, Action::Duplicate, Action::Delete, Action::SelectAll] {
        item(app, ui, a);
    }
    item(app, ui, Action::Deselect);
    item(app, ui, Action::SelectNext);
    item(app, ui, Action::SelectPrev);
    item(app, ui, Action::RenameLayer);
    ui.separator();
    ui.menu_button("Tools", |ui| {
        for (tool, _, _, _) in Tool::ALL {
            let action = Action::Tool(tool);
            let current = app.tool == tool;
            let label = if current { format!("● {}", action.label()) } else { action.label() };
            item_labelled(app, ui, action, &label);
        }
    });
    ui.menu_button("Rotate", |ui| {
        item(app, ui, Action::RotateCw);
        item(app, ui, Action::RotateCcw);
        item(app, ui, Action::Rotate180);
        let id = egui::Id::new("rotate-other");
        let mut deg: f64 = ui.data(|d| d.get_temp(id)).unwrap_or(15.0);
        ui.add_enabled_ui(actions::enabled(app, Action::RotateCw), |ui| {
            ui.horizontal(|ui| {
                ui.label("Other");
                ui.add(egui::DragValue::new(&mut deg).range(-360.0..=360.0).suffix("°").speed(1.0));
                if ui.button("Rotate").clicked() {
                    crate::actions::rotate_selection(app, deg);
                    ui.close();
                }
            });
        });
        ui.data_mut(|d| d.insert_temp(id, deg));
        ui.separator();
        item(app, ui, Action::FlipH);
        item(app, ui, Action::FlipV);
    });
    item(app, ui, Action::PositionSize);
    ui.menu_button("Arrange", |ui| {
        for a in [Action::ToFront, Action::Forward, Action::Backward, Action::ToBack] {
            item(app, ui, a);
        }
    });
}

fn slide_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    for a in [Action::NewSlide, Action::DuplicateSlide, Action::DeleteSlide] {
        item(app, ui, a);
    }
    crate::templates_ui::slide_menu(app, ui);
    ui.separator();
    if ui.button("Soundtrack…").on_hover_text("Music or sound playing across the whole presentation").clicked() {
        app.dialog = Some(crate::dialogs::Dialog::Soundtrack(crate::soundtrack_ui::State::new()));
        ui.close();
    }
    ui.menu_button("Transition", |ui| crate::transition_ui::menu(app, ui))
        .response
        .on_hover_text("How this slide comes in (more settings in the Slides panel)");
    ui.separator();
    item(app, ui, Action::AddPause)
        .on_hover_text("Presenting stops here until you press → or click (shown as ⏸ on the timeline)");
    ui.separator();
    item(app, ui, Action::Present);
    item(app, ui, Action::PresentCurrent);
    if ui
        .button("Rehearse With Presenter View")
        .on_hover_text("Your notes, the next slide and a timer, in this window (V switches to the slides)")
        .clicked()
    {
        let ctx = ui.ctx().clone();
        crate::present::start(app, 0, crate::present::Screen::ThisWindow, &ctx);
        if let Some(p) = app.presenter.as_mut() {
            p.presenter_view = true;
        }
        ui.close();
    }
    ui.menu_button("Present On", |ui| {
        let chosen = app.present_screen.clone().unwrap_or_else(crate::present::default_screen);
        for screen in crate::present::screens() {
            let label = crate::present::screen_label(&screen);
            if ui.selectable_label(screen == chosen, label).clicked() {
                app.present_screen = Some(screen.clone());
                let ctx = ui.ctx().clone();
                let i = crate::actions::current_slide(app).unwrap_or(0);
                crate::present::start(app, i, screen, &ctx);
                ui.close();
            }
        }
    });
}

fn ai_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    item(app, ui, Action::AiNew).on_hover_text("Describe an animation or a whole slide deck; it opens in a new tab");
    item(app, ui, Action::AiNewSlide).on_hover_text("Adds a slide after this one and builds it from your description");
    item(app, ui, Action::AiModifySlide).on_hover_text("Describe what to change on the open slide");
    item(app, ui, Action::AiImage).on_hover_text("Paint a picture, kept as an image or traced into vectors");
    ui.separator();
    if ui.button("Assistant Panel").clicked() {
        crate::dock::reveal(app, crate::dock::Panel::Assistant);
        ui.close();
    }
    if ui.button("AI Settings…").on_hover_text("Your OpenRouter key and which models to use").clicked() {
        app.settings.open(crate::settings::Tab::Ai);
        ui.close();
    }
}

fn view_menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    ui.menu_button("Panels", |ui| {
        for panel in Panel::ALL {
            let mut open = crate::dock::is_open(app, panel);
            let enabled = panel != Panel::Canvas;
            if ui.add_enabled(enabled, egui::Checkbox::new(&mut open, panel.label())).clicked() {
                crate::dock::toggle(app, panel);
            }
        }
        ui.separator();
        if ui.button("Reset Layout").on_hover_text("Put all panels back where they started").clicked() {
            app.dock = Some(crate::dock::default_layout());
            app.viewport.request_fit();
            ui.close();
        }
    });
    ui.separator();
    item(app, ui, Action::ZoomFit);
    item(app, ui, Action::Zoom100);
    item(app, ui, Action::ZoomIn);
    item(app, ui, Action::ZoomOut);
    ui.menu_button("Zoom To", |ui| {
        for pct in [25, 50, 200, 400] {
            if ui.button(format!("{pct}%")).clicked() {
                app.viewport.set_zoom(pct as f64 / 100.0);
                ui.close();
            }
        }
        ui.separator();
        ui.weak(format!("Now {:.0}%", app.viewport.zoom() * 100.0));
    });
    ui.separator();
    crate::camera_ui::view_menu_item(app, ui);
    ui.separator();
    toggle(app, ui, Action::ToggleRulers, |p| &mut p.show_rulers);
    toggle(app, ui, Action::ToggleGrid, |p| &mut p.show_grid);
    toggle(app, ui, Action::ToggleGuides, |p| &mut p.show_guides);
    if ui
        .checkbox(&mut app.prefs.show_slices, "Show Slices")
        .on_hover_text("Outline the export slices on the canvas (edit them in View › Panels › Slices)")
        .clicked()
    {
        ui.close();
    }
    let has_guides = !app.comp().settings.guides.is_empty();
    if ui.add_enabled(has_guides, egui::Button::new("Clear Guides")).clicked() {
        let mut settings = app.comp().settings.clone();
        settings.guides.clear();
        app.exec("Clear guides", xenify_ops::Command::SetCompSettings { comp: app.comp, settings });
        ui.close();
    }
    ui.separator();
    ui.menu_button("Snap", |ui| {
        let p = &mut app.prefs;
        ui.checkbox(&mut p.snap_objects, "To objects and the frame");
        ui.checkbox(&mut p.snap_guides, "To guides");
        ui.checkbox(&mut p.snap_grid, "To the grid");
        ui.separator();
        let units = app.comp().settings.units;
        let mut v = units.from_px(app.prefs.snap_margin);
        ui.horizontal(|ui| {
            ui.label("Margin").on_hover_text(
                "Also stop this far from the frame's and other objects' edges (0 = off), so things line up with a gap",
            );
            let r = ui.add(
                egui::DragValue::new(&mut v)
                    .range(0.0..=10_000.0)
                    .speed(0.5)
                    .suffix(format!(" {}", units.unit.suffix())),
            );
            if r.changed() {
                app.prefs.snap_margin = units.to_px(v);
            }
        });
        ui.weak("Hold Ctrl while dragging to turn snapping off.");
    });
}

/// A View-menu checkbox bound to a preference, with its shortcut.
fn toggle(app: &mut XenifyApp, ui: &mut egui::Ui, action: Action, flag: fn(&mut crate::settings::Prefs) -> &mut bool) {
    let on = *flag(&mut app.prefs);
    let label = if on { format!("✔ {}", action.label()) } else { format!("   {}", action.label()) };
    if ui.add(egui::Button::new(label).shortcut_text(app.keymap.text(action))).clicked() {
        *flag(&mut app.prefs) = !on;
        ui.close();
    }
}
