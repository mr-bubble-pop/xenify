//! The Styles tab: each paragraph style as a tile showing its look. Click
//! one to use it; double-click (or right-click › Edit) changes it in a
//! dialog, where every property is either set or taken from the style it's
//! based on.

use crate::app::WriteApp;
use egui::{Color32, RichText, Sense, Stroke, Vec2};
use xenify_wp_doc::{Align, ParaStyle, Rgba, StyleSheet, ops};

/// The style dialog: editing one, or naming a new one.
pub enum StyleDialog {
    Edit(Box<ParaStyle>),
    New(String),
}

#[derive(Default)]
pub struct StylesPanel {
    pub dialog: Option<StyleDialog>,
}

/// A checkbox "set" plus a value editor for an optional property.
fn opt<T: Clone>(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut Option<T>,
    default: T,
    edit: impl FnOnce(&mut egui::Ui, &mut T),
) {
    ui.horizontal(|ui| {
        let mut set = v.is_some();
        if ui.checkbox(&mut set, label).on_hover_text("Unticked: as in the style it's based on").changed() {
            *v = set.then(|| default.clone());
        }
        if let Some(val) = v.as_mut() {
            edit(ui, val);
        }
    });
}

/// A three-way checkbox: ticked (on), empty (off), or a dash (as in the
/// style it's based on). Each click moves to the next.
fn tri(ui: &mut egui::Ui, label: &str, v: &mut Option<bool>) {
    let mut shown = v.unwrap_or(false);
    let r = ui
        .add(egui::Checkbox::new(&mut shown, label).indeterminate(v.is_none()))
        .on_hover_text("Ticked: on · Empty: off · Dash: as in the style it's based on");
    if r.clicked() {
        *v = match *v {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
    }
}

fn pt(ui: &mut egui::Ui, v: &mut f32, max: f32) {
    ui.add(egui::DragValue::new(v).range(0.0..=max).speed(0.5).suffix(" pt"));
}

pub fn show(app: &mut WriteApp, ui: &mut egui::Ui) {
    let current = app.current_style();
    let styles = app.editor.doc.styles.paragraph.clone();
    ui.horizontal(|ui| {
        if ui.button("New style…").on_hover_text("A new style that looks like the paragraph at the caret").clicked() {
            app.styles_panel.dialog = Some(StyleDialog::New(String::new()));
        }
        if ui.button("Edit…").on_hover_text("Change the style the caret's paragraph has").clicked()
            && let Some(s) = app.editor.doc.styles.get(&current).cloned()
        {
            app.styles_panel.dialog = Some(StyleDialog::Edit(Box::new(s)));
        }
    });
    ui.weak("Click a style to use it; double-click to change it.");
    let gap = 6.0;
    egui::ScrollArea::vertical()
        .id_salt("style-tiles")
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            let w = ((ui.available_width() - gap) / 2.0).floor().max(80.0);
            let mut apply = None;
            let mut edit = None;
            let mut update = None;
            let mut delete = None;
            for pair in styles.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for s in pair {
                        let r = tile(app, ui, s, w, s.id == current);
                        if r.clicked() {
                            apply = Some(s.id.clone());
                        }
                        if r.double_clicked() {
                            edit = Some(s.clone());
                        }
                        let builtin = StyleSheet::builtin().iter().any(|b| b.id == s.id);
                        r.context_menu(|ui| {
                            if ui.button("Use it").clicked() {
                                apply = Some(s.id.clone());
                                ui.close();
                            }
                            if ui.button("Change it…").clicked() {
                                edit = Some(s.clone());
                                ui.close();
                            }
                            if ui.button("Make it look like the paragraph at the caret").clicked() {
                                update = Some(s.id.clone());
                                ui.close();
                            }
                            if !builtin && ui.button("Delete").clicked() {
                                delete = Some(s.id.clone());
                                ui.close();
                            }
                        });
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            if let Some(id) = apply {
                app.editor.seal();
                let _ = ops::set_style(&mut app.editor, &id);
                app.refocus = true;
            }
            if let Some(s) = edit {
                app.styles_panel.dialog = Some(StyleDialog::Edit(Box::new(s)));
            }
            if let Some(id) = update {
                update_to_match(app, &id);
            }
            if let Some(id) = delete {
                let _ = ops::delete_style(&mut app.editor, &id);
            }
        });
}

/// One style as a tile: its name in its own look, on paper.
fn tile(app: &WriteApp, ui: &mut egui::Ui, s: &ParaStyle, w: f32, current: bool) -> egui::Response {
    let r = app.editor.doc.styles.resolve(&s.id);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 54.0), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, current, format!("{} style", s.name))
    });
    let p = ui.painter();
    p.rect_filled(rect, 6.0, Color32::WHITE);
    let accent = ui.visuals().selection.bg_fill;
    let stroke = if current {
        Stroke::new(2.0, accent)
    } else if resp.hovered() {
        Stroke::new(1.5, accent.gamma_multiply(0.7))
    } else {
        Stroke::new(1.0, Color32::from_gray(200))
    };
    p.rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Inside);
    let [cr, cg, cb, _] = r.char.color.0;
    let mut text = RichText::new(&s.name).size(r.char.size.clamp(9.0, 20.0)).color(Color32::from_rgb(cr, cg, cb));
    if r.char.bold {
        text = text.strong();
    }
    if r.char.italic {
        text = text.italics();
    }
    if r.char.underline {
        text = text.underline();
    }
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        w - 16.0,
        egui::TextStyle::Body,
    );
    let pos = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(pos, galley, Color32::BLACK);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &s.name));
    resp.on_hover_text(if current { "The style here" } else { "Click to use it" })
}

/// The style dialog (editing a style, or naming a new one).
pub fn dialog(app: &mut WriteApp, ctx: &egui::Context) {
    let Some(d) = app.styles_panel.dialog.take() else { return };
    match d {
        StyleDialog::New(mut name) => {
            let mut open = true;
            let mut made = None;
            let mut cancel = false;
            crate::dialog::window(ctx, "New style", &mut open).show(ctx, |ui| {
                ui.label("Name it; it starts out looking like the paragraph at the caret.");
                let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text("Style name").desired_width(260.0));
                r.request_focus();
                use crate::dialog::Kind;
                let ok = if name.trim().is_empty() { Kind::Off } else { Kind::Main };
                match crate::dialog::buttons(ui, &[("Create", ok), ("Cancel", Kind::Cancel)]) {
                    Some(0) => made = Some(name.clone()),
                    Some(_) => cancel = true,
                    None => {}
                }
            });
            if let Some(n) = made {
                if let Ok(id) = ops::new_style_from_selection(&mut app.editor, n.trim()) {
                    let _ = ops::set_style(&mut app.editor, &id);
                }
            } else if open && !cancel {
                app.styles_panel.dialog = Some(StyleDialog::New(name));
            }
        }
        StyleDialog::Edit(mut def) => {
            let mut open = true;
            let mut done = None;
            let title = format!("Style: {}", def.name);
            crate::dialog::window(ctx, title, &mut open).resizable(true).default_width(440.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| editor(app, ui, &mut def));
                use crate::dialog::Kind;
                done = crate::dialog::buttons(ui, &[("OK", Kind::Main), ("Cancel", Kind::Cancel)]);
            });
            match done {
                Some(0) => {
                    if app.editor.doc.styles.get(&def.id).is_some_and(|s| *s != *def) {
                        let _ = ops::define_style(&mut app.editor, *def);
                    }
                }
                Some(_) => {}
                None if open => app.styles_panel.dialog = Some(StyleDialog::Edit(def)),
                None => {}
            }
        }
    }
}

/// The style's properties, each set or taken from the style it's based on.
fn editor(app: &mut WriteApp, ui: &mut egui::Ui, def: &mut ParaStyle) {
    let styles = app.editor.doc.styles.paragraph.clone();
    let builtin = StyleSheet::builtin().iter().any(|b| b.id == def.id);
    egui::Grid::new("style-basics").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut def.name);
        ui.end_row();
        let others: Vec<(String, String)> =
            styles.iter().filter(|s| s.id != def.id).map(|s| (s.id.clone(), s.name.clone())).collect();
        let name_of = |id: &Option<String>| {
            id.as_ref()
                .and_then(|i| others.iter().find(|(x, _)| x == i))
                .map_or("(none)".to_string(), |(_, n)| n.clone())
        };
        ui.label("Based on");
        egui::ComboBox::from_id_salt("based-on").selected_text(name_of(&def.based_on)).show_ui(ui, |ui| {
            ui.selectable_value(&mut def.based_on, None, "(none)");
            for (id, name) in &others {
                ui.selectable_value(&mut def.based_on, Some(id.clone()), name);
            }
        });
        ui.end_row();
        ui.label("Next paragraph");
        let shown = if def.next.is_none() { "(same style)".to_string() } else { name_of(&def.next) };
        egui::ComboBox::from_id_salt("next-style").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut def.next, None, "(same style)");
            for (id, name) in &others {
                ui.selectable_value(&mut def.next, Some(id.clone()), name);
            }
        });
        ui.end_row();
        ui.label("Outline level");
        let shown = def.outline_level.map_or("Body text".to_string(), |l| format!("Level {l}"));
        egui::ComboBox::from_id_salt("outline").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut def.outline_level, None, "Body text");
            for l in 1..=6u8 {
                ui.selectable_value(&mut def.outline_level, Some(l), format!("Level {l}"));
            }
        });
        ui.end_row();
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Font").strong());
    let families = app.font_families();
    opt(ui, "Family", &mut def.char.font, "Inter".into(), |ui, f| {
        egui::ComboBox::from_id_salt("style-font").selected_text(f.as_str()).height(300.0).show_ui(ui, |ui| {
            for fam in &families {
                ui.selectable_value(f, fam.clone(), fam);
            }
        });
    });
    opt(ui, "Size", &mut def.char.size, 11.0, |ui, v| {
        ui.add(egui::DragValue::new(v).range(1.0..=400.0).speed(0.25).suffix(" pt"));
    });
    ui.horizontal_wrapped(|ui| {
        tri(ui, "Bold", &mut def.char.bold);
        tri(ui, "Italic", &mut def.char.italic);
        tri(ui, "Underline", &mut def.char.underline);
        tri(ui, "Strikethrough", &mut def.char.strike);
    });
    opt(ui, "Colour", &mut def.char.color, Rgba::BLACK, |ui, c| {
        let [r, g, b, a] = c.0;
        let mut col = Color32::from_rgba_unmultiplied(r, g, b, a);
        if egui::color_picker::color_edit_button_srgba(ui, &mut col, egui::color_picker::Alpha::Opaque).changed() {
            let [r, g, b, _] = col.to_srgba_unmultiplied();
            *c = Rgba([r, g, b, 255]);
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Paragraph").strong());
    opt(ui, "Alignment", &mut def.para.align, Align::Left, |ui, a| {
        for (v, n) in
            [(Align::Left, "Left"), (Align::Center, "Centre"), (Align::Right, "Right"), (Align::Justify, "Justify")]
        {
            ui.selectable_value(a, v, n);
        }
    });
    opt(ui, "Space before", &mut def.para.space_before, 0.0, |ui, v| pt(ui, v, 144.0));
    opt(ui, "Space after", &mut def.para.space_after, 8.0, |ui, v| pt(ui, v, 144.0));
    opt(ui, "Line spacing", &mut def.para.line_height, 1.3, |ui, v| {
        ui.add(egui::DragValue::new(v).range(0.8..=4.0).speed(0.01).suffix("×"));
    });
    opt(ui, "Indent left", &mut def.para.indent_left, 0.0, |ui, v| pt(ui, v, 400.0));
    opt(ui, "Indent right", &mut def.para.indent_right, 0.0, |ui, v| pt(ui, v, 400.0));
    tri(ui, "Keep with the next paragraph", &mut def.para.keep_with_next);
    if builtin {
        ui.add_space(4.0);
        ui.weak("A built-in style: it can be changed but not deleted.");
    }
}

/// Folds the caret paragraph's direct formatting into style `id`.
fn update_to_match(app: &mut WriteApp, id: &str) {
    let ed = &app.editor;
    let Some(p) = ed.doc.para(ed.sel.focus.para) else { return };
    let Some(mut def) = ed.doc.styles.get(id).cloned() else { return };
    def.para = def.para.with(&p.props);
    if let Some(r) = p.runs.first() {
        def.char = def.char.with(&xenify_wp_doc::CharFmt { link: None, ..r.fmt.clone() });
    }
    let _ = ops::define_style(&mut app.editor, def);
}
