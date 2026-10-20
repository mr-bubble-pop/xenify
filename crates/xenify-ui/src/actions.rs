//! Runs [`Action`]s, whether they come from a key, a menu or the shortcut
//! reference, and says which ones make sense right now.

use crate::app::{XenifyApp, PaintSlot, Tool};
use crate::dialogs::Dialog;
use crate::keymap::{Action, KeyState};
use xenify_doc::{Id, LayerKind};
use xenify_ops::order::Placement;

/// Handles this frame's shortcut keys.
pub fn handle_keys(app: &mut XenifyApp, ctx: &egui::Context) {
    let state = KeyState {
        typing: ctx.text_edit_focused(),
        dialog_open: app.dialog.is_some(),
        brush_tool: matches!(app.tool, Tool::Brush | Tool::Pencil | Tool::Sculpt),
        space_panned: app.viewport.space_panned(),
    };
    // Ctrl+Shift+C arrives as a Copy event with Shift held, not a key press.
    let precompose = ctx.input(|i| i.modifiers.shift && i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
    if precompose && !state.typing && !state.dialog_open {
        run(app, ctx, Action::Precompose);
    }
    // Recording a new shortcut in Settings: keys go there, not to actions.
    if app.settings.recording.is_some() {
        return;
    }
    for action in app.keymap.poll(ctx, state) {
        if enabled(app, action) {
            run(app, ctx, action);
        }
    }
}

/// Whether `action` can run now (menus grey out the rest).
pub fn enabled(app: &XenifyApp, action: Action) -> bool {
    use Action::*;
    let has_sel = app.selected.is_some();
    let n_sel = app.selected_layers().len();
    match action {
        Undo => app.editor.undo_label().is_some(),
        Redo => app.editor.redo_label().is_some(),
        Cut | Copy | Duplicate | Delete | ToFront | Forward | Backward | ToBack | RotateCw | RotateCcw | Rotate180
        | FlipH | FlipV | Precompose => {
            has_sel
                || (action == Delete
                    && (!app.selected_keys.is_empty()
                        || (app.tool == crate::app::Tool::Pen && app.viewport.pen_in_progress())))
        }
        Group => has_sel,
        Ungroup => {
            app.selected.and_then(|id| app.project().layer(id).ok()).is_some_and(|l| matches!(l.kind, LayerKind::Group))
        }
        ClipMask => n_sel >= 2,
        ReleaseClipMask => has_sel,
        SelectAll => !app.comp().layers.is_empty(),
        DuplicateSlide | DeleteSlide => current_slide(app).is_some(),
        PositionSize => has_sel,
        // Not built yet.
        NextTab | PrevTab => app.docs.tabs.len() > 1,
        RenameLayer => has_sel,
        _ => true,
    }
}

pub fn run(app: &mut XenifyApp, ctx: &egui::Context, action: Action) {
    use Action::*;
    match action {
        New => app.dialog = Some(Dialog::new_document(app)),
        Open => crate::session::guard(app, crate::session::Pending::Open),
        Save => app.save(),
        SaveAs => app.dialog = Some(Dialog::save_as(app)),
        Import => app.dialog = Some(Dialog::import(app)),
        Export => app.dialog = Some(Dialog::export(app)),
        Quit => crate::session::guard(app, crate::session::Pending::Quit),
        ShortcutsHelp => app.shortcuts_open = !app.shortcuts_open,
        Settings => app.settings.open(crate::settings::Tab::General),
        CloseTab => crate::session::guard(app, crate::session::Pending::Close(vec![app.docs.active()])),
        NextTab => app.cycle_tab(true),
        PrevTab => app.cycle_tab(false),
        RenameLayer => {
            if let Some(l) = app.selected.and_then(|id| app.project().layer(id).ok()).cloned() {
                crate::layers_panel::start_rename(ctx, &l);
            }
        }
        Print => app.dialog = Some(Dialog::print(app)),
        Undo => app.undo(),
        Redo => app.redo(),
        Cut => crate::clipboard::copy_selection(app, ctx, true),
        Copy => crate::clipboard::copy_selection(app, ctx, false),
        Paste => crate::clipboard::paste_os(app),
        Duplicate => crate::clipboard::duplicate(app),
        Delete => app.delete_selection(),
        SelectAll => select_all(app),
        SelectNext => select_step(app, 1),
        SelectPrev => select_step(app, -1),
        Deselect => {
            let mut vp = std::mem::take(&mut app.viewport);
            let pen = app.tool == crate::app::Tool::Pen && vp.end_pen(app);
            app.viewport = vp;
            if pen {
                // Esc keeps the path drawn so far.
            } else if app.tool == crate::app::Tool::Crop {
                // Esc leaves cropping without changing anything.
                app.viewport.cancel_tool();
                app.set_tool(crate::app::Tool::Select);
            } else if !app.viewport.cancel_tool() {
                // Esc with nothing selected leaves a symbol being edited.
                if app.selected_layers().is_empty() && crate::symbols::is_symbol(app.project(), app.comp) {
                    crate::symbols::finish(app);
                }
                app.selected = None;
                app.extra_selected.clear();
                app.selected_keys.clear();
            }
        }
        Group => app.group_selection(),
        Ungroup => app.ungroup_selection(),
        Precompose => app.precompose_selection(),
        ClipMask => app.clip_selection(true),
        ReleaseClipMask => app.clip_selection(false),
        ToFront => arrange(app, Placement::Front),
        Forward => arrange(app, Placement::Forward),
        Backward => arrange(app, Placement::Backward),
        ToBack => arrange(app, Placement::Back),
        RotateCw => rotate_selection(app, 90.0),
        RotateCcw => rotate_selection(app, -90.0),
        Rotate180 => rotate_selection(app, 180.0),
        FlipH => transform_selection(app, "Flip", kurbo::Affine::scale_non_uniform(-1.0, 1.0)),
        FlipV => transform_selection(app, "Flip", kurbo::Affine::scale_non_uniform(1.0, -1.0)),
        NewSlide => crate::slides_panel::add_after(app, None),
        AiNew => {
            let mut d = crate::new_dialog::NewDialog::new(app);
            d.describe_with_ai();
            app.dialog = Some(Dialog::NewDocument(Box::new(d)));
        }
        AiNewSlide | AiModifySlide => {
            let task = if action == AiNewSlide {
                crate::ai_create::SlideTask::New
            } else {
                crate::ai_create::SlideTask::Modify
            };
            app.dialog = Some(Dialog::AiPrompt(crate::ai_create::AiPrompt { task, text: String::new() }));
        }
        AiImage => app.dialog = Some(Dialog::illustrate()),
        DuplicateSlide => {
            if let Some(i) = current_slide(app) {
                match xenify_ops::slides::duplicate_slide(app.project(), i) {
                    Ok((cmd, id)) => {
                        if app.exec("Duplicate slide", cmd) {
                            app.open_comp(id);
                        }
                    }
                    Err(e) => app.fail(e.to_string()),
                }
            }
        }
        DeleteSlide => {
            if let Some(i) = current_slide(app) {
                crate::slides_panel::delete(app, i);
            }
        }
        AddPause => toggle_pause(app),
        Present => {
            let screen = app.present_screen.clone().unwrap_or_else(crate::present::default_screen);
            crate::present::start(app, 0, screen, ctx);
        }
        PresentCurrent => {
            let i = current_slide(app).unwrap_or(0);
            let screen = app.present_screen.clone().unwrap_or_else(crate::present::default_screen);
            crate::present::start(app, i, screen, ctx);
        }
        ToggleRulers => app.prefs.show_rulers = !app.prefs.show_rulers,
        ToggleGrid => app.prefs.show_grid = !app.prefs.show_grid,
        ToggleGuides => app.prefs.show_guides = !app.prefs.show_guides,
        PositionSize => app.dialog = Some(Dialog::position_size(app)),
        ZoomIn => app.viewport.zoom_by(2.0),
        ZoomOut => app.viewport.zoom_by(0.5),
        ZoomFit => app.viewport.request_fit(),
        Zoom100 => app.viewport.set_zoom_100(),
        PlayPause => app.toggle_play(),
        NudgeLeft => nudge(app, ctx, -1.0, 0.0),
        NudgeRight => nudge(app, ctx, 1.0, 0.0),
        NudgeUp => nudge(app, ctx, 0.0, -1.0),
        NudgeDown => nudge(app, ctx, 0.0, 1.0),
        FrameBack => app.set_time(app.time - 1.0 / app.fps()),
        FrameForward => app.set_time(app.time + 1.0 / app.fps()),
        GoStart => app.set_time(0.0),
        GoEnd => app.set_time(f64::INFINITY),
        PrevKey => app.jump_keyframe(false),
        NextKey => app.jump_keyframe(true),
        Tool(t) => app.set_tool(t),
        CommitTool => {
            let mut vp = std::mem::take(&mut app.viewport);
            vp.commit_tool(app);
            app.viewport = vp;
        }
        ToggleFillStroke => {
            app.paint.active = match app.paint.active {
                PaintSlot::Fill => PaintSlot::Stroke,
                PaintSlot::Stroke => PaintSlot::Fill,
            }
        }
        SwapFillStroke => crate::colors::swap(app),
        ResetColors => crate::colors::reset(app),
        BrushSmaller => crate::tool_options::resize_brush(app, false),
        BrushBigger => crate::tool_options::resize_brush(app, true),
    }
}

/// The slide being edited (0 for a plain project's main composition).
pub fn current_slide(app: &XenifyApp) -> Option<usize> {
    xenify_ops::slides::slides_or_main(app.project()).iter().position(|s| s.comp == app.comp)
}

/// Adds a pause marker at the playhead, or removes the one there.
pub fn toggle_pause(app: &mut XenifyApp) {
    toggle_pause_at(app, app.time);
}

/// Adds a pause marker at `t`, or removes the one there.
pub fn toggle_pause_at(app: &mut XenifyApp, t: f64) {
    let fps = app.fps();
    let t = (t * fps).round() / fps;
    let mut settings = app.comp().settings.clone();
    let near = settings.pauses.iter().position(|p| (p - t).abs() < 0.5 / fps);
    let label = match near {
        Some(i) => {
            settings.pauses.remove(i);
            "Remove pause"
        }
        None => {
            if t <= 0.0 || t >= settings.duration {
                app.status = "Pauses go between the start and the end of the slide: move the playhead.".into();
                return;
            }
            settings.pauses.push(t);
            settings.pauses.sort_by(f64::total_cmp);
            "Add pause"
        }
    };
    app.exec(label, xenify_ops::Command::SetCompSettings { comp: app.comp, settings });
}

/// Ctrl+A: every keyframe when the pointer is over the timeline, otherwise
/// every visible, unlocked top-level object.
fn select_all(app: &mut XenifyApp) {
    if app.timeline.hovered {
        crate::timeline::select_all_keys(app);
        return;
    }
    let comp = app.comp();
    let in_group = |id: Id| {
        comp.layer(id)
            .and_then(|l| l.meta.parent)
            .and_then(|p| comp.layer(p))
            .is_some_and(|p| matches!(p.kind, LayerKind::Group))
    };
    let matte_sources: Vec<Id> = comp.layers.iter().filter_map(|l| l.meta.matte.map(|m| m.layer)).collect();
    let ids: Vec<Id> = comp
        .layers
        .iter()
        .rev()
        .filter(|l| l.meta.visible && !l.meta.locked && !in_group(l.id) && !matte_sources.contains(&l.id))
        .filter(|l| !matches!(l.kind, LayerKind::Null))
        .map(|l| l.id)
        .collect();
    app.select_only(ids.first().copied());
    app.extra_selected = ids.iter().skip(1).copied().collect();
    app.keep_selection();
    app.status = format!("Selected {} objects", ids.len());
}

/// Selects the next (`step` 1) or previous (-1) object in the Layers
/// panel's order, top to bottom, wrapping round: the keyboard's way to pick
/// things on the canvas. Hidden and locked layers are skipped.
pub fn select_step(app: &mut XenifyApp, step: isize) {
    let comp = app.comp();
    let ids: Vec<(Id, String)> = comp
        .layers
        .iter()
        .rev()
        .filter(|l| l.meta.visible && !l.meta.locked)
        .map(|l| (l.id, l.meta.name.clone()))
        .collect();
    if ids.is_empty() {
        app.status = "Nothing to select".into();
        return;
    }
    let n = ids.len() as isize;
    let at = app.selected.and_then(|s| ids.iter().position(|(id, _)| *id == s));
    let next = match at {
        Some(i) => (i as isize + step).rem_euclid(n) as usize,
        None if step > 0 => 0,
        None => ids.len() - 1,
    };
    let (id, name) = ids[next].clone();
    app.select_only(Some(id));
    app.status = format!("Selected {name} ({} of {})", next + 1, ids.len());
}

/// Rotates the selection `degrees` clockwise around its centre.
pub fn rotate_selection(app: &mut XenifyApp, degrees: f64) {
    transform_selection(app, "Rotate", kurbo::Affine::rotate(degrees.to_radians()));
}

fn arrange(app: &mut XenifyApp, placement: Placement) {
    let layers = app.selected_layers();
    match xenify_ops::order::move_layers(app.project(), app.comp, &layers, placement) {
        Ok(Some(cmd)) => {
            app.exec("Arrange", cmd);
        }
        Ok(None) => {}
        Err(e) => app.fail(e.to_string()),
    }
}

/// Applies `m` (a rotation or flip) to the selection around its centre.
/// Arrow keys: moves the selection (or, in Edit Points, its selected points)
/// 1 px, 10 with Shift; repeated presses are one undo step. With nothing
/// selected, left and right step a frame.
fn nudge(app: &mut XenifyApp, ctx: &egui::Context, dx: f64, dy: f64) {
    let step = if ctx.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
    let d = kurbo::Vec2::new(dx * step, dy * step);
    if app.tool == crate::app::Tool::Node {
        let mut vp = std::mem::take(&mut app.viewport);
        let moved = vp.nudge_points(app, d);
        app.viewport = vp;
        if moved {
            return;
        }
    }
    let selected = app.selected_layers();
    let layers: Vec<Id> = selected
        .iter()
        .copied()
        .filter(|id| {
            // Not locked, and not moving already with a selected parent.
            app.project()
                .layer(*id)
                .is_ok_and(|l| !l.meta.locked && l.meta.parent.is_none_or(|p| !selected.contains(&p)))
        })
        .collect();
    if layers.is_empty() {
        if dy == 0.0 {
            app.set_time(app.time + dx / app.fps());
        }
        return;
    }
    let (comp, time, auto_key) = (app.comp, app.time, app.auto_key);
    let mut cmds = Vec::new();
    for id in layers {
        let snap = xenify_ops::transform::Snapshot::new(app.project(), comp, id, time, kurbo::Rect::ZERO);
        match snap.and_then(|s| s.affine_commands(app.project(), kurbo::Affine::translate(d), time, auto_key)) {
            Ok(c) => cmds.extend(c),
            Err(e) => {
                app.fail(e.to_string());
                return;
            }
        }
    }
    app.exec_merged("Nudge", xenify_ops::Command::batch(cmds));
}

fn transform_selection(app: &mut XenifyApp, label: &str, m: kurbo::Affine) {
    let layers = app.selected_layers();
    let (comp, time, auto_key) = (app.comp().clone(), app.time, app.auto_key);
    let scene = xenify_anim::evaluate_project(app.project(), comp.id, time);
    let boxes: Vec<(Id, kurbo::Rect)> = {
        let mut measure = app.viewport.measurer();
        layers.iter().filter_map(|id| Some((*id, xenify_anim::world_box(&comp, &scene, *id, &mut measure)?))).collect()
    };
    let Some(all) = boxes.iter().map(|(_, b)| *b).reduce(|a, b| a.union(b)) else { return };
    let pivot = all.center().to_vec2();
    let x = kurbo::Affine::translate(pivot) * m * kurbo::Affine::translate(-pivot);
    let mut cmds = Vec::new();
    for (id, _) in &boxes {
        let snap = xenify_ops::transform::Snapshot::new(app.project(), comp.id, *id, time, kurbo::Rect::ZERO);
        match snap.and_then(|s| s.affine_commands(app.project(), x, time, auto_key)) {
            Ok(c) => cmds.extend(c),
            Err(e) => {
                app.fail(e.to_string());
                return;
            }
        }
    }
    app.exec(label, xenify_ops::Command::batch(cmds));
}
