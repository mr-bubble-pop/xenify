//! Typing on the canvas: text layers and diagram labels are edited where
//! they are, with a caret, selection, arrows, Home/End, the clipboard and
//! IME (an egui text field in the text's own font, over the text, which is
//! hidden while it's being edited). One undo step per edit session; Esc or
//! a click elsewhere finishes.

use super::Viewport;
use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Rect, Stroke};
use xenify_anim::{Scene, SceneContent, SceneLayer, SceneText};
use xenify_doc::{Id, LayerKind};
use xenify_ops::Command;
use kurbo::{Affine, Point};

/// What's being typed into.
#[derive(Clone, Debug, PartialEq)]
pub struct InPlace {
    pub layer: Id,
    /// A diagram shape's or connector's label rather than a text layer.
    pub label: bool,
    pub text: String,
    /// The field has had the keyboard (so losing it means "done").
    focused: bool,
    /// Select everything on the first frame (new "Text" placeholder).
    select_all: bool,
}

impl InPlace {
    pub fn text_layer(layer: Id, text: String, select_all: bool) -> Self {
        InPlace { layer, label: false, text, focused: false, select_all }
    }

    pub fn label(layer: Id, text: String) -> Self {
        InPlace { layer, label: true, text, focused: false, select_all: false }
    }
}

/// Starts typing into layer `id` on the canvas: its text, or (on a diagram
/// shape or connector) its label.
pub fn start(app: &mut XenifyApp, id: Id) {
    let Ok(l) = app.project().layer(id) else { return };
    let edit = match &l.kind {
        LayerKind::Text(t) => InPlace::text_layer(id, t.text.clone(), false),
        LayerKind::Shape { .. } => {
            InPlace::label(id, l.meta.label.as_ref().map(|n| n.text.clone()).unwrap_or_default())
        }
        _ => return,
    };
    app.select_only(Some(id));
    app.new_gesture();
    app.in_place = Some(edit);
    app.status = "Type on the canvas · Esc or click elsewhere when done".into();
}

/// The scene with the text being edited left out (the field shows it).
pub fn hide(scene: &Scene, edit: &InPlace) -> Scene {
    fn strip(layers: &mut Vec<SceneLayer>, edit: &InPlace) {
        if !edit.label {
            layers.retain(|l| !(l.id == edit.layer && matches!(l.content, SceneContent::Text(_))));
        }
        for l in layers.iter_mut() {
            match &mut l.content {
                SceneContent::Group(inner) if l.id == edit.layer && edit.label => {
                    inner.retain(|m| !(m.id == edit.layer && matches!(m.content, SceneContent::Text(_))));
                }
                SceneContent::Group(inner) | SceneContent::Precomp { layers: inner, .. } => strip(inner, edit),
                _ => {}
            }
        }
    }
    let mut s = scene.clone();
    strip(&mut s.layers, edit);
    s
}

/// The drawn text of `edit` (layer space) and its transform, if it shows.
fn drawn_text(scene: &Scene, edit: &InPlace) -> Option<(SceneText, Affine)> {
    fn find(layers: &[SceneLayer], edit: &InPlace) -> Option<(SceneText, Affine)> {
        for l in layers {
            match &l.content {
                SceneContent::Text(t) if l.id == edit.layer => return Some((t.clone(), l.transform)),
                SceneContent::Group(inner) | SceneContent::Precomp { layers: inner, .. } => {
                    if let Some(found) = find(inner, edit) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }
    find(&scene.layers, edit)
}

impl Viewport {
    /// The text field over the text being edited (after the canvas is drawn).
    pub(super) fn in_place_editor(&mut self, app: &mut XenifyApp, ui: &mut egui::Ui, rect: Rect, scene: &Scene) {
        let Some(mut edit) = app.in_place.clone() else { return };
        if app.project().layer(edit.layer).is_err() {
            app.in_place = None;
            return;
        }
        // Where and how big: the text as laid out, or (for a new label) the shape's middle.
        // Point text keeps its anchor: the field grows from it as the text is aligned.
        let mut anchor: Option<(Point, xenify_doc::TextAlign)> = None;
        let (font_size, color, family, origin, scale, width, spacing, line_height) = match drawn_text(scene, &edit) {
            Some((t, transform)) => {
                let b = self.measure_text(&t);
                let scale = self.zoom * transform.determinant().abs().sqrt();
                let width = t.frame().map(|f| f.rect.width() * scale);
                if width.is_none() {
                    anchor = Some((transform * Point::new(0.0, b.y0), t.align));
                }
                (
                    t.font_size,
                    t.color,
                    t.font_family.clone(),
                    transform * Point::new(b.x0, b.y0),
                    scale,
                    width,
                    t.letter_spacing,
                    t.line_height,
                )
            }
            None => {
                let label = app.project().layer(edit.layer).ok().and_then(|l| l.meta.label.clone());
                let size = label.as_ref().map_or(14.0, |l| l.font_size);
                let centre = xenify_anim::world_box(app.comp(), scene, edit.layer, &mut *self.text.borrow_mut())
                    .map_or(Point::ZERO, |b| b.center());
                let color = label.as_ref().map_or(xenify_doc::Color::BLACK, |l| l.color);
                (
                    size,
                    color,
                    "Inter".to_string(),
                    centre - kurbo::Vec2::new(size, size * 0.6),
                    self.zoom,
                    None,
                    0.0,
                    0.0,
                )
            }
        };
        let mut at = self.to_screen(rect, origin);
        let px = (font_size * scale).max(4.0) as f32;
        // Inter on the canvas → Inter here (egui's own font otherwise).
        let family = if family.eq_ignore_ascii_case("inter") || family.is_empty() {
            egui::FontFamily::Name("inter-only".into())
        } else {
            egui::FontFamily::Proportional
        };
        let font = egui::FontId::new(px, family);
        let text_color = crate::colors::to_color32(color.with_alpha(color.a.max(0.35)));
        let boxed = width.is_some();
        let wrap = width.map_or(f32::INFINITY, |w| w as f32);
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
            let mut job = egui::text::LayoutJob::default();
            let mut format = egui::TextFormat::simple(font.clone(), text_color);
            format.extra_letter_spacing = (spacing * scale) as f32;
            if line_height > 0.0 {
                format.line_height = Some(px * line_height as f32);
            }
            job.append(text.as_str(), 0.0, format);
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        // How wide the text is now (point text: the field is just that wide).
        let text_w = {
            let mut format = egui::TextFormat::simple(font.clone(), text_color);
            format.extra_letter_spacing = (spacing * scale) as f32;
            let mut job = egui::text::LayoutJob::default();
            job.append(&edit.text, 0.0, format);
            ui.fonts_mut(|f| f.layout_job(job)).size().x.max(px * 0.3)
        };
        if let Some((a, align)) = anchor {
            let x = self.to_screen(rect, a).x;
            at.x = match align {
                xenify_doc::TextAlign::Left => x,
                xenify_doc::TextAlign::Center => x - text_w / 2.0,
                xenify_doc::TextAlign::Right => x - text_w,
            };
        }
        let id = egui::Id::new(("in-place-text", edit.layer));
        let mut done = false;
        let area = egui::Area::new(id.with("area")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
            let mut te = egui::TextEdit::multiline(&mut edit.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_rows(1)
                .layouter(&mut layouter);
            te = if boxed { te.desired_width(wrap) } else { te.desired_width(text_w + px * 0.6).clip_text(false) };
            let out = te.show(ui);
            let r = out.response;
            // Where we type: a thin accent box around the field.
            ui.painter().rect_stroke(r.rect.expand(3.0), 2.0, Stroke::new(1.0, ACCENT), egui::StrokeKind::Outside);
            if !edit.focused {
                r.request_focus();
                if edit.select_all {
                    let mut state = out.state;
                    let all = egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(edit.text.chars().count()),
                    );
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), id);
                }
            }
            if r.has_focus() {
                edit.focused = true;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) || (edit.focused && r.lost_focus()) {
                done = true;
            }
            r.changed()
        });
        let changed = area.inner;
        if changed {
            let cmd = if edit.label {
                xenify_ops::diagram::set_label_text(app.project(), edit.layer, &edit.text).ok()
            } else {
                Some(Command::SetText { layer: edit.layer, text: edit.text.clone() })
            };
            if let Some(cmd) = cmd {
                app.exec_merged(if edit.label { "Label" } else { "Edit text" }, cmd);
            }
        }
        // Another edit may have started this frame (e.g. a click on other text).
        let same =
            |app: &XenifyApp| app.in_place.as_ref().is_some_and(|e| e.layer == edit.layer && e.label == edit.label);
        if !same(app) {
            return;
        }
        if done {
            app.in_place = None;
            // An emptied text layer goes away (in the same undo step).
            if !edit.label && edit.text.trim().is_empty() {
                app.exec_merged("Edit text", Command::RemoveLayer { layer: edit.layer });
                app.select_only(None);
            }
            app.status.clear();
        } else {
            edit.select_all = false;
            app.in_place = Some(edit);
        }
    }
}
