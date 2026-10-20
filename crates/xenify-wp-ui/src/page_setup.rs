//! File › Page setup: page size, orientation, margins, columns, headers and
//! footers (with fields) and numbering, for the section at the caret.

use xenify_wp_doc::{HfLine, PageSetup, SectionStart};

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Unit {
    #[default]
    Cm,
    Mm,
    In,
    Pt,
}

impl Unit {
    pub const ALL: [Unit; 4] = [Unit::Cm, Unit::Mm, Unit::In, Unit::Pt];

    pub fn name(self) -> &'static str {
        match self {
            Unit::Cm => "cm",
            Unit::Mm => "mm",
            Unit::In => "in",
            Unit::Pt => "pt",
        }
    }

    /// Points per unit.
    pub fn pt(self) -> f32 {
        match self {
            Unit::Cm => 72.0 / 2.54,
            Unit::Mm => 72.0 / 25.4,
            Unit::In => 72.0,
            Unit::Pt => 1.0,
        }
    }
}

pub const SIZES: [(&str, (f32, f32)); 6] = [
    ("A4", PageSetup::A4),
    ("Letter", PageSetup::LETTER),
    ("Legal", (612.0, 1008.0)),
    ("A5", (419.53, 595.28)),
    ("A3", (841.89, 1190.55)),
    ("Tabloid", (792.0, 1224.0)),
];

/// The dialog's working copy.
#[derive(Clone)]
pub struct PageSetupDialog {
    pub section: usize,
    pub start: SectionStart,
    pub page: PageSetup,
    pub whole_document: bool,
    tab: Tab,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    #[default]
    Page,
    Headers,
    Layout,
}

/// A length edited in `unit`, stored in points.
pub(crate) fn length(ui: &mut egui::Ui, v: &mut f32, unit: Unit, max_pt: f32) -> bool {
    let mut shown = *v / unit.pt();
    let speed = if unit == Unit::Pt { 0.5 } else { 0.01 };
    let r = ui.add(
        egui::DragValue::new(&mut shown)
            .range(0.0..=max_pt / unit.pt())
            .speed(speed)
            .max_decimals(2)
            .suffix(format!(" {}", unit.name())),
    );
    if r.changed() {
        *v = shown * unit.pt();
    }
    r.changed()
}

fn hf_line(ui: &mut egui::Ui, id: &str, line: &mut HfLine) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        for (label, text) in [("Left", &mut line.left), ("Centre", &mut line.center), ("Right", &mut line.right)] {
            ui.label(label);
            ui.add(egui::TextEdit::singleline(text).desired_width(260.0).hint_text("e.g. Page {page} of {pages}"));
            ui.end_row();
        }
    });
}

impl PageSetupDialog {
    /// Opens on the Layout tab (columns).
    pub fn show_layout(&mut self) {
        self.tab = Tab::Layout;
    }

    pub fn new(section: usize, start: SectionStart, page: PageSetup) -> Self {
        Self { section, start, page, whole_document: false, tab: Tab::Page }
    }

    /// Shows the dialog body: Page, Headers and footers, Layout.
    pub fn ui(&mut self, ui: &mut egui::Ui, unit: &mut Unit, sections: usize) {
        let unit = *unit;
        ui.horizontal(|ui| {
            for (t, name) in [(Tab::Page, "Page"), (Tab::Headers, "Headers and footers"), (Tab::Layout, "Layout")] {
                ui.selectable_value(&mut self.tab, t, name);
            }
        });
        ui.separator();
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        match self.tab {
            Tab::Page => self.page_tab(ui, unit),
            Tab::Headers => self.headers_tab(ui),
            Tab::Layout => self.layout_tab(ui, unit),
        }
        if sections > 1 {
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Apply to");
                ui.radio_value(
                    &mut self.whole_document,
                    false,
                    format!("this section ({} of {sections})", self.section + 1),
                );
                ui.radio_value(&mut self.whole_document, true, "the whole document");
            });
        }
    }

    fn page_tab(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let p = &mut self.page;
        egui::Grid::new("page-size").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Paper size");
            let landscape = p.width > p.height;
            let current = SIZES
                .iter()
                .find(|(_, (w, h))| {
                    let (a, b) = if landscape { (*h, *w) } else { (*w, *h) };
                    (a - p.width).abs() < 0.5 && (b - p.height).abs() < 0.5
                })
                .map_or("Custom", |(n, _)| n);
            egui::ComboBox::from_id_salt("paper").selected_text(current).show_ui(ui, |ui| {
                for (name, (w, h)) in SIZES {
                    if ui.selectable_label(current == name, name).clicked() {
                        (p.width, p.height) = if landscape { (h, w) } else { (w, h) };
                    }
                }
            });
            ui.end_row();
            ui.label("Orientation");
            ui.horizontal(|ui| {
                let landscape = p.width > p.height;
                if ui.selectable_label(!landscape, "Portrait").clicked() && landscape
                    || ui.selectable_label(landscape, "Landscape").clicked() && !landscape
                {
                    std::mem::swap(&mut p.width, &mut p.height);
                }
            });
            ui.end_row();
            ui.label("Margins");
            egui::Grid::new("margins").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Top");
                length(ui, &mut p.margins.top, unit, 1000.0);
                ui.label("Bottom");
                length(ui, &mut p.margins.bottom, unit, 1000.0);
                ui.end_row();
                ui.label("Left");
                length(ui, &mut p.margins.left, unit, 1000.0);
                ui.label("Right");
                length(ui, &mut p.margins.right, unit, 1000.0);
                ui.end_row();
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Advanced").id_salt("page-advanced").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Custom size");
                length(ui, &mut p.width, unit, 5000.0);
                ui.label("×");
                length(ui, &mut p.height, unit, 5000.0);
            });
            ui.weak("Lengths are shown in the unit chosen in Settings › Appearance.");
        });
    }

    fn headers_tab(&mut self, ui: &mut egui::Ui) {
        let p = &mut self.page;
        ui.weak("Double-click the top or bottom of a page to write its header or footer.");
        if self.section > 0 {
            ui.checkbox(&mut p.link_to_previous, "Same as the previous section");
        }
        ui.add_enabled_ui(self.section == 0 || !p.link_to_previous, |ui| {
            ui.checkbox(&mut p.different_first, "Different first page");
            ui.checkbox(&mut p.different_odd_even, "Different odd and even pages")
                .on_hover_text("Even pages get their own header and footer, e.g. page numbers on the outside edge");
        });
        egui::Grid::new("page-numbers").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Page numbers");
            egui::ComboBox::from_id_salt("page-number-format").selected_text(p.number_format.name()).show_ui(
                ui,
                |ui| {
                    for f in xenify_wp_doc::PageNumberFormat::ALL {
                        ui.selectable_value(&mut p.number_format, f, f.name());
                    }
                },
            );
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                let mut restart = p.start_number.is_some();
                if ui.checkbox(&mut restart, "Start at").changed() {
                    p.start_number = restart.then_some(1);
                }
                if let Some(n) = p.start_number.as_mut() {
                    ui.add(egui::DragValue::new(n).range(0..=99_999));
                }
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Advanced: simple header and footer text").id_salt("hf-advanced").show(ui, |ui| {
            ui.add_enabled_ui(self.section == 0 || !p.link_to_previous, |ui| {
                ui.weak(
                    "A line of text with left, centre and right parts. Fields: {page} {pages} {date} {title} {author}",
                );
                ui.label(egui::RichText::new("Header").strong());
                hf_line(ui, "hf-header", &mut p.header);
                ui.label(egui::RichText::new("Footer").strong());
                hf_line(ui, "hf-footer", &mut p.footer);
                if p.different_first {
                    ui.label(egui::RichText::new("First page header").strong());
                    hf_line(ui, "hf-first-header", &mut p.first_header);
                    ui.label(egui::RichText::new("First page footer").strong());
                    hf_line(ui, "hf-first-footer", &mut p.first_footer);
                }
                if p.different_odd_even {
                    ui.label(egui::RichText::new("Even page header").strong());
                    hf_line(ui, "hf-even-header", &mut p.even_header);
                    ui.label(egui::RichText::new("Even page footer").strong());
                    hf_line(ui, "hf-even-footer", &mut p.even_footer);
                }
            });
        });
    }

    fn layout_tab(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let p = &mut self.page;
        egui::Grid::new("layout").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Text columns");
            ui.horizontal(|ui| {
                for n in 1..=4u8 {
                    ui.selectable_value(&mut p.columns, n, n.to_string());
                }
            });
            ui.end_row();
            if p.columns > 1 {
                ui.label("Space between");
                length(ui, &mut p.column_gap, unit, 200.0);
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut p.column_line, "Line between columns");
                ui.end_row();
            }
            ui.label("Text on the page");
            ui.horizontal(|ui| {
                use xenify_wp_doc::PageVAlign as V;
                for (v, name, tip) in [
                    (V::Top, "At the top", "As usual"),
                    (V::Center, "Centred", "In the middle, top to bottom: a title page"),
                    (V::Bottom, "At the bottom", "Down against the bottom margin"),
                ] {
                    ui.radio_value(&mut p.v_align, v, name).on_hover_text(tip);
                }
            });
            ui.end_row();
            if self.section > 0 {
                ui.label("This section starts");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.start, SectionStart::NewPage, "on a new page");
                    ui.radio_value(&mut self.start, SectionStart::Continuous, "straight after the last");
                });
                ui.end_row();
            }
        });
    }
}
