//! Format › Paragraph…: indents (first line, hanging), spacing, alignment,
//! tab stops and the keep/break options of the selected paragraphs.

use crate::page_setup::{Unit, length};
use xenify_wp_doc::{Align, ParaProps, ResolvedPara, TabKind, TabLeader, TabStop};

#[derive(Clone, Copy, PartialEq)]
enum Special {
    None,
    FirstLine,
    Hanging,
}

/// The dialog's working copy (values as the caret paragraph shows them).
#[derive(Clone)]
pub struct ParagraphDialog {
    align: Align,
    left: f32,
    right: f32,
    special: Special,
    by: f32,
    before: f32,
    after: f32,
    line: f32,
    /// "Exactly" this many points (instead of a multiple).
    exact: Option<f32>,
    tabs: Vec<TabStop>,
    new_tab: TabStop,
    keep_with_next: bool,
    keep_together: bool,
    page_break_before: bool,
    widow_control: bool,
    tab: u8,
}

impl ParagraphDialog {
    pub fn new(r: &ResolvedPara) -> Self {
        let (special, by) = if r.first_line > 0.0 {
            (Special::FirstLine, r.first_line)
        } else if r.first_line < 0.0 {
            (Special::Hanging, -r.first_line)
        } else {
            (Special::None, 36.0)
        };
        Self {
            align: r.align,
            left: r.indent_left,
            right: r.indent_right,
            special,
            by,
            before: r.space_before,
            after: r.space_after,
            line: r.line_height,
            exact: r.line_exact,
            tabs: r.tabs.clone(),
            new_tab: TabStop { pos: 72.0, ..Default::default() },
            keep_with_next: r.keep_with_next,
            keep_together: r.keep_together,
            page_break_before: r.page_break_before,
            widow_control: r.widow_control,
            tab: 0,
        }
    }

    /// What applying the dialog sets.
    pub fn props(&self) -> ParaProps {
        ParaProps {
            align: Some(self.align),
            indent_left: Some(self.left),
            indent_right: Some(self.right),
            first_line: Some(match self.special {
                Special::None => 0.0,
                Special::FirstLine => self.by,
                Special::Hanging => -self.by,
            }),
            space_before: Some(self.before),
            space_after: Some(self.after),
            line_height: self.exact.is_none().then_some(self.line),
            line_exact: self.exact,
            tabs: Some(self.tabs.clone()),
            keep_with_next: Some(self.keep_with_next),
            keep_together: Some(self.keep_together),
            page_break_before: Some(self.page_break_before),
            widow_control: Some(self.widow_control),
            ..Default::default()
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, unit: Unit) {
        ui.horizontal(|ui| {
            for (i, name) in ["Indents and spacing", "Tab stops", "Line and page breaks"].into_iter().enumerate() {
                ui.selectable_value(&mut self.tab, i as u8, name);
            }
        });
        ui.separator();
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        match self.tab {
            1 => self.tabs_ui(ui, unit),
            2 => self.breaks_ui(ui),
            _ => self.indents_ui(ui, unit),
        }
    }

    fn indents_ui(&mut self, ui: &mut egui::Ui, unit: Unit) {
        ui.label(egui::RichText::new("Alignment and indents").strong());
        egui::Grid::new("para-indents").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Alignment");
            ui.horizontal(|ui| {
                use crate::icons::{self, Icon};
                for (a, icon, name) in [
                    (Align::Left, Icon::AlignLeft, "Left"),
                    (Align::Center, Icon::AlignCenter, "Centre"),
                    (Align::Right, Icon::AlignRight, "Right"),
                    (Align::Justify, Icon::AlignJustify, "Justify"),
                ] {
                    let b = egui::Button::image_and_text(icons::image(ui, icon, icons::MENU), name)
                        .selected(self.align == a)
                        .frame_when_inactive(self.align == a);
                    if ui.add(b).clicked() {
                        self.align = a;
                    }
                }
            });
            ui.end_row();
            ui.label("Left indent");
            length(ui, &mut self.left, unit, 400.0);
            ui.end_row();
            ui.label("Right indent");
            length(ui, &mut self.right, unit, 400.0);
            ui.end_row();
            ui.label("First line");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("para-special")
                    .selected_text(match self.special {
                        Special::None => "Normal",
                        Special::FirstLine => "Indented by",
                        Special::Hanging => "Hanging by",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.special, Special::None, "Normal");
                        ui.selectable_value(&mut self.special, Special::FirstLine, "Indented by");
                        ui.selectable_value(&mut self.special, Special::Hanging, "Hanging by");
                    });
                if self.special != Special::None {
                    length(ui, &mut self.by, unit, 200.0);
                }
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Spacing").strong());
        egui::Grid::new("para-spacing").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Before");
            ui.add(egui::DragValue::new(&mut self.before).range(0.0..=144.0).suffix(" pt"));
            ui.end_row();
            ui.label("After");
            ui.add(egui::DragValue::new(&mut self.after).range(0.0..=144.0).suffix(" pt"));
            ui.end_row();
            ui.label("Line spacing");
            ui.horizontal(|ui| {
                for (v, name) in crate::toolbar::LINE_SPACINGS {
                    let on = self.exact.is_none() && (self.line - v).abs() < 0.001;
                    if ui.selectable_label(on, name).clicked() {
                        self.line = v;
                        self.exact = None;
                    }
                }
            });
            ui.end_row();
        });
        egui::CollapsingHeader::new("More spacing options").id_salt("para-spacing-more").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.radio(self.exact.is_none(), "Lines of").clicked() {
                    self.exact = None;
                }
                ui.add_enabled(
                    self.exact.is_none(),
                    egui::DragValue::new(&mut self.line).range(0.5..=5.0).speed(0.01).suffix(" × the text size"),
                );
            });
            ui.horizontal(|ui| {
                if ui.radio(self.exact.is_some(), "Exactly").clicked() && self.exact.is_none() {
                    self.exact = Some(14.0);
                }
                let mut pt = self.exact.unwrap_or(14.0);
                if ui
                    .add_enabled(
                        self.exact.is_some(),
                        egui::DragValue::new(&mut pt).range(1.0..=200.0).speed(0.25).suffix(" pt"),
                    )
                    .changed()
                {
                    self.exact = Some(pt);
                }
                ui.weak("(every line the same height)");
            });
        });
    }

    fn tabs_ui(&mut self, ui: &mut egui::Ui, unit: Unit) {
        ui.weak("Measured from the left margin. Text after a tab lines up at the next stop.");
        let mut remove = None;
        egui::Grid::new("para-tabs").num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
            for (i, t) in self.tabs.iter_mut().enumerate() {
                length(ui, &mut t.pos, unit, 1000.0);
                kind_combo(ui, ("tab-kind", i), &mut t.kind);
                leader_combo(ui, ("tab-leader", i), &mut t.leader);
                if ui.button("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
            length(ui, &mut self.new_tab.pos, unit, 1000.0);
            kind_combo(ui, "new-tab-kind", &mut self.new_tab.kind);
            leader_combo(ui, "new-tab-leader", &mut self.new_tab.leader);
            if ui.button("Add").clicked() {
                self.tabs.retain(|t| (t.pos - self.new_tab.pos).abs() > 0.5);
                self.tabs.push(self.new_tab);
                self.new_tab.pos += 72.0;
            }
            ui.end_row();
        });
        if let Some(i) = remove {
            self.tabs.remove(i);
        }
        self.tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        if !self.tabs.is_empty() && ui.button("Clear all tab stops").clicked() {
            self.tabs.clear();
        }
    }

    fn breaks_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.keep_with_next, "Keep with the next paragraph");
        ui.checkbox(&mut self.keep_together, "Keep lines together");
        ui.checkbox(&mut self.page_break_before, "Start on a new page");
        ui.checkbox(&mut self.widow_control, "Don't leave a single line at the top or bottom of a page");
    }
}

fn kind_combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, k: &mut TabKind) {
    egui::ComboBox::from_id_salt(id)
        .width(80.0)
        .selected_text(match k {
            TabKind::Left => "Left",
            TabKind::Center => "Centre",
            TabKind::Right => "Right",
            TabKind::Decimal => "Decimal",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(k, TabKind::Left, "Left");
            ui.selectable_value(k, TabKind::Center, "Centre");
            ui.selectable_value(k, TabKind::Right, "Right");
            ui.selectable_value(k, TabKind::Decimal, "Decimal");
        });
}

fn leader_combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, l: &mut TabLeader) {
    egui::ComboBox::from_id_salt(id)
        .width(90.0)
        .selected_text(match l {
            TabLeader::None => "No leader",
            TabLeader::Dots => "Dots ……",
            TabLeader::Dashes => "Dashes - -",
            TabLeader::Line => "Line ___",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(l, TabLeader::None, "No leader");
            ui.selectable_value(l, TabLeader::Dots, "Dots ……");
            ui.selectable_value(l, TabLeader::Dashes, "Dashes - -");
            ui.selectable_value(l, TabLeader::Line, "Line ___");
        });
}
