//! The formatting toolbar and the matching Format-menu actions.

use crate::app::WriteApp;
use crate::view::Toggle;
use egui::{Button, Color32};
use xenify_wp_doc::{Align, CharFmt, ListDef, MergeKey, ParaProps, Rgba, ops};

pub const SIZES: [f32; 16] =
    [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 72.0];
pub const LINE_SPACINGS: [(f32, &str); 6] =
    [(1.0, "Single"), (1.15, "1.15"), (1.3, "1.3"), (1.5, "1.5"), (2.0, "Double"), (2.5, "2.5")];
fn c32(c: Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl WriteApp {
    /// Character formatting at the caret / for the selection, with what the
    /// paragraph style gives filled in.
    pub fn effective_char(&self) -> xenify_wp_doc::ResolvedChar {
        let ed = &self.editor;
        let base = ed
            .doc
            .para(ed.sel.focus.para)
            .map(|p| ed.doc.resolve_para(p).char)
            .unwrap_or_else(|| ed.doc.styles.resolve(xenify_wp_doc::StyleSheet::NORMAL).char);
        let direct = match (&self.view.pending_fmt, ed.sel.is_collapsed()) {
            (Some(f), true) => f.clone(),
            _ => ops::common_format(ed),
        };
        base.with(&direct)
    }

    /// Applies character formatting to the selection, or to what is typed next.
    pub fn set_char(&mut self, patch: CharFmt, label: &str, merge: Option<&str>) {
        let ed = &mut self.editor;
        if ed.sel.is_collapsed() {
            let base = self.view.pending_fmt.clone().unwrap_or_else(|| {
                let f = ed.doc.para(ed.sel.focus.para).map(|p| p.fmt_at(ed.sel.focus.offset)).unwrap_or_default();
                CharFmt { field: None, ..f }
            });
            self.view.pending_fmt = Some(ops::normalize_fmt(base.with(&patch)));
        } else {
            if merge.is_none() {
                ed.seal();
            }
            let _ = ops::apply_format_merged(ed, &patch, label, merge.map(|m| MergeKey::Control(m.into())));
        }
    }

    pub fn set_para(&mut self, patch: ParaProps, label: &str) {
        self.editor.seal();
        let _ = ops::apply_para_props(&mut self.editor, &patch, label);
    }

    pub fn current_para_props(&self) -> Option<xenify_wp_doc::ResolvedPara> {
        let ed = &self.editor;
        ed.doc.para(ed.sel.focus.para).map(|p| ed.doc.resolve_para(p))
    }

    /// Indent: list items move a level in; other paragraphs indent by ½ inch.
    pub fn indent(&mut self, delta: i8) {
        match ops::change_list_level(&mut self.editor, delta) {
            Ok(true) => {}
            _ => {
                let now = self.editor.doc.para(self.editor.sel.focus.para).and_then(|p| p.props.indent_left);
                let v = (now.unwrap_or(0.0) + 36.0 * delta as f32).max(0.0);
                self.set_para(ParaProps { indent_left: Some(v), ..Default::default() }, "Indent");
            }
        }
    }

    pub fn focus_document(&self, ctx: &egui::Context) {
        ctx.memory_mut(|m| m.request_focus(self.view.id));
    }

    /// Approximate widths of the toolbar's groups (points), to decide what fits.
    const GROUP_WIDTHS: [f32; 10] = [56.0, 146.0, 222.0, 86.0, 122.0, 145.0, 166.0, 60.0, 30.0, 150.0];

    /// The toolbar: one row; groups that don't fit go behind "…".
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut touched = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let room = ui.available_width() - 40.0;
            // Keep the most-used groups when space is short, in their usual order.
            const KEEP: [usize; 10] = [0, 1, 3, 6, 5, 7, 4, 2, 8, 9];
            let mut used = 0.0;
            let mut shown = [false; 10];
            for g in KEEP {
                let w = Self::GROUP_WIDTHS[g] + 10.0;
                if used + w <= room {
                    shown[g] = true;
                    used += w;
                }
            }
            let hidden: Vec<usize> = (0..Self::GROUP_WIDTHS.len()).filter(|g| !shown[*g]).collect();
            let mut first = true;
            for (g, show) in shown.iter().enumerate() {
                if *show {
                    if !first {
                        ui.separator();
                    }
                    first = false;
                    self.toolbar_group(ui, g, &mut touched);
                }
            }
            if !hidden.is_empty() {
                ui.separator();
                let more = egui::Button::new("…").frame_when_inactive(false).min_size(crate::icons::TOOL_BUTTON);
                let r = menu_with(ui, more, |ui| {
                    for g in &hidden {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            self.toolbar_group(ui, *g, &mut touched);
                        });
                    }
                })
                .response;
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "More tools"));
                r.on_hover_text("More tools");
            }
        });
        if touched {
            self.focus_document(&ctx);
        }
    }

    fn toolbar_group(&mut self, ui: &mut egui::Ui, group: usize, touched: &mut bool) {
        use crate::icons::{self, Icon};
        match group {
            0 => {
                if icons::button_enabled(ui, self.editor.can_undo(), Icon::Undo, "Undo", "Undo (Ctrl+Z)").clicked() {
                    self.editor.undo();
                    self.view.extra.clear();
                    *touched = true;
                }
                if icons::button_enabled(ui, self.editor.can_redo(), Icon::Redo, "Redo", "Redo (Ctrl+Y)").clicked() {
                    self.editor.redo();
                    self.view.extra.clear();
                    *touched = true;
                }
            }
            1 => {
                // Paragraph style.
                let current = self.current_style();
                let name = self.editor.doc.styles.get(&current).map_or(current.clone(), |s| s.name.clone());
                egui::ComboBox::from_id_salt("para-style")
                    .selected_text(name)
                    .width(130.0)
                    .height(560.0)
                    .show_ui(ui, |ui| self.style_list(ui))
                    .response
                    .on_hover_text("Paragraph style");
            }
            2 => {
                // Font and size.
                let eff = self.effective_char();
                let mut font = eff.font.clone();
                egui::ComboBox::from_id_salt("font-family")
                    .selected_text(&font)
                    .width(120.0)
                    .height(400.0)
                    .show_ui(ui, |ui| {
                        for f in self.font_families() {
                            if ui.selectable_label(f == font, &f).clicked() {
                                font = f;
                            }
                        }
                    })
                    .response
                    .on_hover_text("Font");
                if font != eff.font {
                    self.set_char(CharFmt { font: Some(font), ..Default::default() }, "Font", None);
                    *touched = true;
                }
                let mut size = eff.size;
                egui::ComboBox::from_id_salt("font-size")
                    .selected_text(format!("{size}"))
                    .width(52.0)
                    .show_ui(ui, |ui| {
                        for s in SIZES {
                            ui.selectable_value(&mut size, s, format!("{s}"));
                        }
                    })
                    .response
                    .on_hover_text("Font size");
                if size != eff.size {
                    self.set_char(CharFmt { size: Some(size), ..Default::default() }, "Font size", None);
                    *touched = true;
                }
            }
            3 => {
                for (t, icon, name, tip) in [
                    (Toggle::Bold, Icon::Bold, "Bold", "Bold (Ctrl+B)"),
                    (Toggle::Italic, Icon::Italic, "Italic", "Italic (Ctrl+I)"),
                    (Toggle::Underline, Icon::Underline, "Underline", "Underline (Ctrl+U)"),
                ] {
                    let on = self.view.is_on(&self.editor, t);
                    if icons::toggle(ui, icon, name, tip, on).clicked() {
                        self.view.toggle(&mut self.editor, t);
                        *touched = true;
                    }
                }
            }
            4 => {
                // Text colour and highlight: the icon with a bar of the colour under it.
                let eff = self.effective_char();
                if let Some(c) = colour_button(ui, Icon::TextColor, "Text colour", c32(eff.color), false) {
                    let c = c.unwrap_or(Rgba([0, 0, 0, 255]));
                    self.set_char(CharFmt { color: Some(c), ..Default::default() }, "Text colour", None);
                    *touched = true;
                }
                let hl = eff.highlight.filter(|h| h.0[3] > 0);
                if let Some(c) =
                    colour_button(ui, Icon::Highlight, "Highlight", hl.map_or(Color32::TRANSPARENT, c32), true)
                {
                    let c = c.unwrap_or(Rgba([0, 0, 0, 0]));
                    self.set_char(CharFmt { highlight: Some(c), ..Default::default() }, "Highlight", None);
                    *touched = true;
                }
                if icons::button(ui, Icon::Link, "Link", "Link (Ctrl+K)").clicked() {
                    self.open_link_dialog();
                }
                if icons::button(ui, Icon::CommentAdd, "New comment", "New comment (Ctrl+Alt+M)").clicked() {
                    self.review.start_comment(&self.editor);
                }
            }
            5 => {
                // Paragraph alignment and spacing.
                let para = self.current_para_props();
                for (a, icon, name, tip) in [
                    (Align::Left, Icon::AlignLeft, "Align left", "Align left (Ctrl+L)"),
                    (Align::Center, Icon::AlignCenter, "Centre", "Centre (Ctrl+E)"),
                    (Align::Right, Icon::AlignRight, "Align right", "Align right (Ctrl+R)"),
                    (Align::Justify, Icon::AlignJustify, "Justify", "Justify (Ctrl+J)"),
                ] {
                    let on = para.as_ref().is_some_and(|p| p.align == a);
                    if icons::toggle(ui, icon, name, tip, on).clicked() {
                        self.set_para(ParaProps { align: Some(a), ..Default::default() }, "Align");
                        *touched = true;
                    }
                }
                let spacing = egui::Button::image(icons::image(ui, Icon::Spacing, icons::TOOL))
                    .frame_when_inactive(false)
                    .min_size(crate::icons::TOOL_BUTTON);
                menu_with(ui, spacing, |ui| {
                    ui.label("Line spacing");
                    let now = para.as_ref().map_or(1.3, |p| p.line_height);
                    for (v, name) in LINE_SPACINGS {
                        if ui.radio((now - v).abs() < 0.01, name).clicked() {
                            self.set_para(ParaProps { line_height: Some(v), ..Default::default() }, "Line spacing");
                            *touched = true;
                            ui.close();
                        }
                    }
                    ui.separator();
                    let (before, after) = para.as_ref().map_or((0.0, 0.0), |p| (p.space_before, p.space_after));
                    let mut b = before;
                    let mut a = after;
                    ui.horizontal(|ui| {
                        ui.label("Space before");
                        ui.add(egui::DragValue::new(&mut b).range(0.0..=144.0).suffix(" pt"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Space after");
                        ui.add(egui::DragValue::new(&mut a).range(0.0..=144.0).suffix(" pt"));
                    });
                    if b != before || a != after {
                        let _ = ops::apply_para_props(
                            &mut self.editor,
                            &ParaProps { space_before: Some(b), space_after: Some(a), ..Default::default() },
                            "Paragraph spacing",
                        );
                    }
                })
                .response
                .on_hover_text("Line & paragraph spacing");
            }
            6 => {
                // Lists.
                let in_list = self.editor.doc.para(self.editor.sel.focus.para).and_then(|p| p.list.clone());
                for (id, icon, name, tip) in [
                    (
                        ListDef::BULLETS,
                        Icon::Bullets,
                        "Bulleted list",
                        "Bulleted list (Ctrl+Shift+8, or type * and a space)",
                    ),
                    (
                        ListDef::NUMBERS,
                        Icon::Numbers,
                        "Numbered list",
                        "Numbered list (Ctrl+Shift+7, or type 1. and a space)",
                    ),
                    (
                        ListDef::CHECKLIST,
                        Icon::Checklist,
                        "Checklist",
                        "Checklist (or type [ ] and a space); click a box to tick it",
                    ),
                ] {
                    let on = in_list.as_ref().is_some_and(|l| l.list == id);
                    if icons::toggle(ui, icon, name, tip, on).clicked() {
                        let _ = ops::toggle_list(&mut self.editor, id);
                        *touched = true;
                    }
                }
                let more = egui::Button::image(icons::image(ui, Icon::More, 14.0))
                    .frame_when_inactive(false)
                    .min_size(egui::vec2(18.0, icons::TOOL_BUTTON.y));
                menu_with(ui, more, |ui| {
                    ui.label("List style");
                    for def in self.editor.doc.lists.clone() {
                        let on = in_list.as_ref().is_some_and(|l| l.list == def.id);
                        if ui.radio(on, &def.name).clicked() {
                            let _ = ops::toggle_list(&mut self.editor, &def.id);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.add_enabled(in_list.is_some(), Button::new("Restart numbering at 1")).clicked() {
                        let _ = ops::restart_numbering(&mut self.editor, Some(1));
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            in_list.as_ref().is_some_and(|l| l.restart.is_some()),
                            Button::new("Continue numbering"),
                        )
                        .clicked()
                    {
                        let _ = ops::restart_numbering(&mut self.editor, None);
                        ui.close();
                    }
                })
                .response
                .on_hover_text("More list styles");
                if icons::button(ui, Icon::IndentLess, "Decrease indent", "Decrease indent (Shift+Tab)").clicked() {
                    self.indent(-1);
                    *touched = true;
                }
                if icons::button(ui, Icon::IndentMore, "Increase indent", "Increase indent (Tab)").clicked() {
                    self.indent(1);
                    *touched = true;
                }
            }
            7 => {
                if icons::button(ui, Icon::Image, "Insert picture", "Insert a picture").clicked() {
                    self.open_insert_image();
                }
                let table = egui::Button::image(icons::image(ui, Icon::Table, icons::TOOL))
                    .frame_when_inactive(false)
                    .min_size(crate::icons::TOOL_BUTTON);
                let r = menu_with(ui, table, |ui| self.table_picker(ui)).response;
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Insert table"));
                r.on_hover_text("Insert a table");
            }
            8 => {
                if icons::button(ui, Icon::ClearFormat, "Clear formatting", "Clear formatting (Ctrl+\\)").clicked() {
                    self.clear_formatting();
                    *touched = true;
                }
            }
            // Less-used text buttons: the first to move behind "…".
            9 => {
                for (t, icon, name, tip) in [
                    (Toggle::Strike, Icon::Strike, "Strikethrough", "Strikethrough (Alt+Shift+5)"),
                    (Toggle::Superscript, Icon::Superscript, "Superscript", "Superscript (Ctrl+.)"),
                    (Toggle::Subscript, Icon::Subscript, "Subscript", "Subscript (Ctrl+,)"),
                ] {
                    let on = self.view.is_on(&self.editor, t);
                    if icons::toggle(ui, icon, name, tip, on).clicked() {
                        self.view.toggle(&mut self.editor, t);
                        *touched = true;
                    }
                }
                let eff = self.effective_char();
                let mut size = eff.size;
                if ui.small_button("A−").on_hover_text("Smaller text (Ctrl+[)").clicked() {
                    size = SIZES.iter().rev().copied().find(|s| *s < eff.size).unwrap_or(eff.size);
                }
                if ui.small_button("A+").on_hover_text("Bigger text (Ctrl+])").clicked() {
                    size = SIZES.iter().copied().find(|s| *s > eff.size).unwrap_or(eff.size);
                }
                if size != eff.size {
                    self.set_char(CharFmt { size: Some(size), ..Default::default() }, "Font size", None);
                    *touched = true;
                }
            }
            _ => {}
        }
    }

    pub fn clear_formatting(&mut self) {
        self.editor.seal();
        let _ = ops::clear_format(&mut self.editor);
        let _ = ops::clear_para_props(&mut self.editor);
        self.view.pending_fmt = None;
    }

    pub fn font_families(&mut self) -> Vec<String> {
        if self.families.is_empty() {
            let mut v = vec!["Inter".to_string(), "serif".into(), "sans-serif".into(), "monospace".into()];
            v.extend(self.view.engine.fonts.families().into_iter().filter(|f| f != "Inter"));
            self.families = v;
        }
        self.families.clone()
    }
}

/// Text and highlight colours offered in the palette (our own picks).
pub const PALETTE: [[u8; 3]; 20] = [
    [0x00, 0x00, 0x00],
    [0x43, 0x48, 0x52],
    [0x76, 0x7c, 0x88],
    [0xb4, 0xb9, 0xc2],
    [0xff, 0xff, 0xff],
    [0xc6, 0x28, 0x28],
    [0xe6, 0x5c, 0x00],
    [0xf2, 0xb7, 0x05],
    [0x2e, 0x8b, 0x57],
    [0x1f, 0x6f, 0xd1],
    [0x6a, 0x3d, 0xc4],
    [0xb8, 0x2d, 0x84],
    [0x0b, 0x7a, 0x86],
    [0x5d, 0x40, 0x37],
    [0x1b, 0x2a, 0x4a],
    [0xff, 0xeb, 0x3b],
    [0xa5, 0xd6, 0xa7],
    [0x80, 0xde, 0xea],
    [0xf8, 0xbb, 0xd0],
    [0xff, 0xcc, 0x80],
];

/// A toolbar colour button: the icon over a bar of `current`, opening a
/// palette. Returns the chosen colour (`Some(None)`: "none"/automatic).
fn colour_button(
    ui: &mut egui::Ui,
    icon: crate::icons::Icon,
    name: &str,
    current: Color32,
    allow_none: bool,
) -> Option<Option<Rgba>> {
    let img = crate::icons::image(ui, icon, crate::icons::TOOL);
    let mut picked = None;
    let r =
        menu_with(ui, egui::Button::image(img).frame_when_inactive(false).min_size(crate::icons::TOOL_BUTTON), |ui| {
            ui.label(name);
            egui::Grid::new(("palette", name)).spacing([3.0, 3.0]).show(ui, |ui| {
                for (i, [r, g, b]) in PALETTE.iter().enumerate() {
                    let c = Color32::from_rgb(*r, *g, *b);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::ColorButton,
                            true,
                            c == current,
                            format!("#{r:02x}{g:02x}{b:02x}"),
                        )
                    });
                    ui.painter().rect_filled(rect, 3.0, c);
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        egui::Stroke::new(
                            if c == current { 2.0 } else { 1.0 },
                            ui.visuals().widgets.inactive.bg_stroke.color,
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if resp.on_hover_text(format!("#{r:02x}{g:02x}{b:02x}")).clicked() {
                        picked = Some(Some(Rgba([*r, *g, *b, 255])));
                        ui.close();
                    }
                    if i % 5 == 4 {
                        ui.end_row();
                    }
                }
            });
            ui.separator();
            if ui.button(if allow_none { "No highlight" } else { "Automatic (black)" }).clicked() {
                picked = Some(None);
                ui.close();
            }
            let mut c = if current == Color32::TRANSPARENT { Color32::BLACK } else { current };
            ui.horizontal(|ui| {
                ui.label("More colours");
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed()
                {
                    let [r, g, b, _] = c.to_srgba_unmultiplied();
                    picked = Some(Some(Rgba([r, g, b, 255])));
                }
            });
        });
    // The colour bar under the icon.
    let rect = r.response.rect;
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 5.0, rect.max.y - 5.0),
        egui::pos2(rect.max.x - 5.0, rect.max.y - 2.5),
    );
    let shown = if current == Color32::TRANSPARENT { ui.visuals().weak_text_color() } else { current };
    ui.painter().rect_filled(bar, 1.0, shown);
    let enabled = r.response.enabled();
    r.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    r.response.on_hover_text(name);
    picked
}

/// A menu opened by a custom button (an icon); its response.
fn menu_with<R>(ui: &mut egui::Ui, button: egui::Button<'_>, add: impl FnOnce(&mut egui::Ui) -> R) -> MenuShown {
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, add);
    MenuShown { response }
}

struct MenuShown {
    response: egui::Response,
}
