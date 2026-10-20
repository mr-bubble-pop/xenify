//! Format › Table › Table properties: the table's look, size and cells in tabs, with
//! a picture of the table beside them. Changes show in the document as they
//! are made (one undo step); OK keeps them, Cancel puts the table back.
//! Click or drag in the picture to choose the cells the Cells tab changes.

use xenify_wp_doc::tables::{self, TableStyle};
use xenify_wp_doc::{Border, NodeId, ParaBorders, Rgba, Table, VAlign};
use std::ops::RangeInclusive;

#[derive(Clone, Copy, PartialEq, Default)]
enum Tab {
    #[default]
    Look,
    Size,
    Cells,
}

#[derive(Clone)]
pub struct TableDialog {
    pub id: NodeId,
    /// The table as it will be.
    pub work: Table,
    /// The table as it was when the dialog opened (what Cancel goes back to).
    original: Table,
    /// `work` as last put in the document (or taken from it).
    seen: Table,
    /// Whether the dialog has changed the document.
    changed: bool,
    /// Where a drag over the picture started (row, column).
    drag_from: Option<(usize, usize)>,
    tab: Tab,
    /// The cells the Cells tab changes.
    rows: RangeInclusive<usize>,
    cols: RangeInclusive<usize>,
    custom: egui::Color32,
}

/// What the dialog's buttons asked for.
pub enum Outcome {
    Nothing,
    Done,
    Cancel,
}

/// `look` (its settings and cell formatting) with the text of `now`'s cells,
/// so changing the look never undoes typing done meanwhile.
fn with_content(look: &Table, now: &Table) -> Table {
    let mut t = look.clone();
    for (row, cur) in t.rows.iter_mut().zip(&now.rows) {
        for (cell, c) in row.cells.iter_mut().zip(&cur.cells) {
            cell.blocks = c.blocks.clone();
        }
    }
    t
}

/// Whether `a` and `b` have the same rows and columns (and spans).
fn same_grid(a: &Table, b: &Table) -> bool {
    a.rows.len() == b.rows.len()
        && a.rows.iter().zip(&b.rows).all(|(x, y)| {
            x.cells.len() == y.cells.len()
                && x.cells
                    .iter()
                    .zip(&y.cells)
                    .all(|(p, q)| (p.covered, p.col_span, p.row_span) == (q.covered, q.col_span, q.row_span))
        })
}

/// The row and column at `pos` in a table drawn by [`paint_table`] in `rect`.
fn cell_at(rect: egui::Rect, t: &Table, pos: egui::Pos2, max_rows: usize) -> Option<(usize, usize)> {
    let inner = rect.shrink(6.0);
    let n = t.rows.len().min(max_rows).max(1);
    let cols = t.widths.len().max(1);
    let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
    let tw = inner.width() * t.width.clamp(0.1, 1.0);
    let x0 = match t.align {
        xenify_wp_doc::Align::Center => inner.left() + (inner.width() - tw) / 2.0,
        xenify_wp_doc::Align::Right => inner.right() - tw,
        _ => inner.left(),
    };
    let r = (((pos.y - inner.top()) / (inner.height() / n as f32)).floor().max(0.0) as usize).min(n - 1);
    let mut x = x0;
    for (c, w) in t.widths.iter().enumerate() {
        x += tw * w / total;
        if pos.x < x || c + 1 == cols {
            return Some((r, c));
        }
    }
    None
}

fn c32(c: Rgba) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

/// The cell covering column `c` of row `r` (merged cells cover several).
fn showing(t: &Table, r: usize, c: usize) -> Option<(usize, usize)> {
    let row = t.rows.get(r)?;
    let mut i = c.min(row.cells.len().checked_sub(1)?);
    while i > 0 && row.cells[i].covered {
        i -= 1;
    }
    let mut rr = r;
    while rr > 0 && t.rows[rr].cells.get(i).is_some_and(|x| x.covered) {
        rr -= 1;
    }
    Some((rr, i))
}

/// Draws `t` small in `rect`: lines, header and band colours, cell colours,
/// and grey bars for the text.
pub fn paint_table(painter: &egui::Painter, rect: egui::Rect, t: &Table, max_rows: usize) {
    painter.rect_filled(rect, 3.0, egui::Color32::WHITE);
    let inner = rect.shrink(6.0);
    let cols = t.widths.len().max(1);
    let rows = t.rows.len().min(max_rows).max(1);
    let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
    let tw = inner.width() * t.width.clamp(0.1, 1.0);
    let x0 = match t.align {
        xenify_wp_doc::Align::Center => inner.left() + (inner.width() - tw) / 2.0,
        xenify_wp_doc::Align::Right => inner.right() - tw,
        _ => inner.left(),
    };
    let mut xs = vec![x0];
    for w in &t.widths {
        xs.push(xs.last().copied().unwrap_or(x0) + tw * w / total);
    }
    let rh = inner.height() / rows as f32;
    let head = t.rows.iter().take_while(|r| r.header).count();
    let line = |b: Border| egui::Stroke::new(b.width.clamp(0.5, 2.0), c32(b.color));
    for r in 0..rows {
        for c in 0..cols {
            let Some(cell) = t.rows.get(r).and_then(|row| row.cells.get(c)) else { continue };
            if cell.covered {
                continue;
            }
            let (w, h) = (cell.col_span.max(1) as usize, cell.row_span.max(1) as usize);
            let x1 = xs.get((c + w).min(cols)).copied().unwrap_or(xs[cols]);
            let cr = egui::Rect::from_min_max(
                egui::pos2(xs[c], inner.top() + r as f32 * rh),
                egui::pos2(x1, inner.top() + ((r + h).min(rows)) as f32 * rh),
            );
            // Fill: the cell's own, else header, header column or band.
            let fill = cell
                .shading
                .or_else(|| (r < head).then(|| t.header_fill.unwrap_or(Rgba::rgb(0xf1, 0xf3, 0xf5))))
                .or_else(|| (c == 0 && t.header_col).then(|| t.header_fill.unwrap_or(Rgba::rgb(0xf1, 0xf3, 0xf5))))
                .or_else(|| t.band.filter(|_| r >= head && (r - head) % 2 == 1));
            if let Some(f) = fill {
                painter.rect_filled(cr, 0.0, c32(f));
            }
            // A bar for the text, darker in header cells.
            let bar = egui::Rect::from_min_size(
                cr.left_center() + egui::vec2(4.0, -1.5),
                egui::vec2((cr.width() - 8.0).max(2.0) * 0.6, 3.0),
            );
            let ink = if r < head || (c == 0 && t.header_col) { 0x55 } else { 0xa0 };
            painter.rect_filled(bar, 1.0, egui::Color32::from_gray(ink));
            if let Some(b) = t.border {
                painter.rect_stroke(cr, 0.0, line(b), egui::StrokeKind::Middle);
            }
            if let Some(bs) = cell.borders {
                for (edge, b) in [
                    ((cr.left_top(), cr.right_top()), bs.top),
                    ((cr.left_bottom(), cr.right_bottom()), bs.bottom),
                    ((cr.left_top(), cr.left_bottom()), bs.left),
                    ((cr.right_top(), cr.right_bottom()), bs.right),
                ] {
                    if let Some(b) = b {
                        painter.line_segment([edge.0, edge.1], line(b));
                    }
                }
            }
        }
    }
}

/// Where cells `rows` × `cols` are drawn by [`paint_table`] in `rect`.
fn cells_rect(
    rect: egui::Rect,
    t: &Table,
    rows: RangeInclusive<usize>,
    cols: RangeInclusive<usize>,
    max_rows: usize,
) -> Option<egui::Rect> {
    let inner = rect.shrink(6.0);
    let n = t.rows.len().min(max_rows).max(1);
    if *rows.start() >= n {
        return None;
    }
    let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
    let tw = inner.width() * t.width.clamp(0.1, 1.0);
    let x0 = match t.align {
        xenify_wp_doc::Align::Center => inner.left() + (inner.width() - tw) / 2.0,
        xenify_wp_doc::Align::Right => inner.right() - tw,
        _ => inner.left(),
    };
    let x = |c: usize| x0 + tw * t.widths.iter().take(c).sum::<f32>() / total;
    let rh = inner.height() / n as f32;
    Some(egui::Rect::from_min_max(
        egui::pos2(x(*cols.start()), inner.top() + *rows.start() as f32 * rh),
        egui::pos2(x(cols.end() + 1), inner.top() + ((rows.end() + 1).min(n)) as f32 * rh),
    ))
}

impl TableDialog {
    /// For table `id`, changing the cells `rows` × `cols` on the Cells tab.
    pub fn new(table: Table, rows: RangeInclusive<usize>, cols: RangeInclusive<usize>) -> Self {
        Self {
            id: table.id,
            original: table.clone(),
            seen: table.clone(),
            work: table,
            changed: false,
            drag_from: None,
            tab: Tab::Look,
            rows,
            cols,
            custom: egui::Color32::from_rgb(0xff, 0xf2, 0xcc),
        }
    }

    /// Keeps the dialog and the document in step, once a frame: what the
    /// dialog changed goes into the document (merged into one undo step);
    /// otherwise the dialog follows the document (typing, undo). False when
    /// the table is gone.
    pub fn sync(&mut self, ed: &mut xenify_wp_doc::Editor) -> bool {
        let Some(now) = ed.doc.table(self.id).cloned() else { return false };
        if self.work != self.seen && same_grid(&self.work, &now) {
            let t = with_content(&self.work, &now);
            if t != now {
                let key = xenify_wp_doc::MergeKey::Control(format!("table-props-{}", self.id.0));
                ed.group("Table properties", Some(key), |ed| {
                    let _ =
                        tables::replace_block(ed, self.id, xenify_wp_doc::Block::Table(t.clone()), "Table properties");
                });
                self.changed = true;
            }
            self.work = t;
        } else {
            self.work = now;
        }
        self.seen = self.work.clone();
        let (nr, nc) = (self.work.rows.len().max(1), self.work.widths.len().max(1));
        self.rows = (*self.rows.start()).min(nr - 1)..=(*self.rows.end()).min(nr - 1);
        self.cols = (*self.cols.start()).min(nc - 1)..=(*self.cols.end()).min(nc - 1);
        true
    }

    /// Cancel: the table's look goes back to how it was (its text stays).
    pub fn revert(&mut self, ed: &mut xenify_wp_doc::Editor) {
        if !self.changed {
            return;
        }
        if ed.undo_label() == Some("Table properties") {
            ed.undo();
        } else if let Some(now) = ed.doc.table(self.id).cloned()
            && same_grid(&self.original, &now)
        {
            let t = with_content(&self.original, &now);
            let _ = tables::replace_block(ed, self.id, xenify_wp_doc::Block::Table(t), "Table properties");
        }
    }

    fn each_cell(&mut self, f: impl Fn(&mut xenify_wp_doc::Cell)) {
        let (rows, cols) = (self.rows.clone(), self.cols.clone());
        for r in rows {
            for c in cols.clone() {
                if let Some((rr, cc)) = showing(&self.work, r, c)
                    && let Some(cell) = self.work.rows.get_mut(rr).and_then(|row| row.cells.get_mut(cc))
                {
                    f(cell);
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut out = Outcome::Nothing;
        ui.horizontal(|ui| {
            for (t, name) in [(Tab::Look, "Look"), (Tab::Size, "Size"), (Tab::Cells, "Cells")] {
                ui.selectable_value(&mut self.tab, t, name);
            }
        });
        ui.separator();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(510.0);
                ui.set_min_height(260.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.spacing_mut().slider_width = 140.0;
                match self.tab {
                    Tab::Look => self.look(ui),
                    Tab::Size => self.size(ui),
                    Tab::Cells => self.cells(ui),
                }
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let picking = self.tab != Tab::Look;
                ui.weak(if picking { "Cells" } else { "Preview" });
                let sense = if picking { egui::Sense::click_and_drag() } else { egui::Sense::hover() };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(230.0, 170.0), sense);
                paint_table(ui.painter(), rect, &self.work, 7);
                if picking {
                    // Click a cell, or drag across several, to choose them.
                    let at = resp.interact_pointer_pos().and_then(|p| cell_at(rect, &self.work, p, 7));
                    if resp.drag_started() || resp.clicked() {
                        self.drag_from = at;
                    }
                    if let (Some(a), Some(b)) = (self.drag_from, at)
                        && (resp.dragged() || resp.clicked())
                    {
                        self.rows = a.0.min(b.0)..=a.0.max(b.0);
                        self.cols = a.1.min(b.1)..=a.1.max(b.1);
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Cell);
                    }
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Choose cells"));
                    // The cells the Cells and Size tabs change.
                    if let Some(r) = cells_rect(rect, &self.work, self.rows.clone(), self.cols.clone(), 7) {
                        ui.painter().rect_stroke(
                            r,
                            0.0,
                            egui::Stroke::new(2.0, crate::theme::accent(ui.visuals())),
                            egui::StrokeKind::Inside,
                        );
                    }
                    ui.weak("Click or drag to choose the\ncells that change (outlined).");
                }
            });
        });
        use crate::dialog::Kind;
        match crate::dialog::buttons(ui, &[("OK", Kind::Main), ("Cancel", Kind::Cancel)]) {
            Some(0) => out = Outcome::Done,
            Some(_) => out = Outcome::Cancel,
            None => {}
        }
        out
    }

    fn look(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Style").strong());
        ui.horizontal_wrapped(|ui| {
            for st in TableStyle::ALL {
                let mut sample = self.work.clone();
                tables::style_table(&mut sample, st);
                ui.vertical(|ui| {
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(96.0, 62.0), egui::Sense::click());
                    paint_table(ui.painter(), rect, &sample, 4);
                    let stroke = if r.hovered() {
                        egui::Stroke::new(2.0, crate::theme::accent(ui.visuals()))
                    } else {
                        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
                    };
                    ui.painter().rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Outside);
                    let r = r.on_hover_text("Use this look");
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} style", st.name()))
                    });
                    if r.clicked() {
                        self.work = sample;
                    }
                    ui.weak(st.name());
                });
            }
        });
        ui.add_space(4.0);
        let mut header = self.work.rows.first().is_some_and(|r| r.header);
        if ui.checkbox(&mut header, "Header row (repeats on each page)").changed()
            && let Some(r) = self.work.rows.first_mut()
        {
            r.header = header;
        }
        ui.checkbox(&mut self.work.header_col, "Header column");
        let mut banded = self.work.band.is_some();
        if ui.checkbox(&mut banded, "Banded rows").changed() {
            self.work.band = banded.then_some(Rgba::rgb(0xf4, 0xf7, 0xfb));
        }
        ui.horizontal(|ui| {
            ui.label("Lines");
            let now = self.work.border.map_or(0.0, |b| b.width);
            for (w, name) in [(0.0, "None"), (0.5, "Thin"), (0.75, "Normal"), (1.5, "Thick")] {
                if ui.radio((now - w).abs() < 0.01, name).clicked() {
                    self.work.border = (w > 0.0).then(|| Border { width: w, ..self.work.border.unwrap_or_default() });
                }
            }
        });
    }

    fn size(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("table-size").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Table width");
            let mut pct = self.work.width * 100.0;
            if ui.add(egui::Slider::new(&mut pct, 20.0..=100.0).fixed_decimals(0).suffix("% of the page")).changed() {
                self.work.width = pct / 100.0;
            }
            ui.end_row();
            ui.label("Position");
            ui.horizontal(|ui| {
                for (a, n) in [
                    (xenify_wp_doc::Align::Left, "Left"),
                    (xenify_wp_doc::Align::Center, "Centre"),
                    (xenify_wp_doc::Align::Right, "Right"),
                ] {
                    ui.selectable_value(&mut self.work.align, a, n);
                }
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Columns").strong());
        if ui.button("Make all columns the same width").clicked() {
            let n = self.work.widths.len().max(1);
            self.work.widths = vec![1.0 / n as f32; n];
        }
        let col = *self.cols.start();
        if col < self.work.widths.len() {
            let total: f32 = self.work.widths.iter().sum::<f32>().max(0.001);
            let mut pct = self.work.widths[col] / total * 100.0;
            ui.horizontal(|ui| {
                ui.label("This column is");
                let n = self.work.widths.len() as f32;
                if ui
                    .add(
                        egui::Slider::new(&mut pct, 5.0..=(100.0 - 5.0 * (n - 1.0)).max(5.0))
                            .fixed_decimals(0)
                            .suffix("% of the table"),
                    )
                    .changed()
                {
                    // The others share what's left, keeping their proportions.
                    let rest: f32 = total - self.work.widths[col];
                    let left = 100.0 - pct;
                    for (i, w) in self.work.widths.iter_mut().enumerate() {
                        *w = if i == col {
                            pct
                        } else if rest > 0.0 {
                            *w / rest * left
                        } else {
                            left / (n - 1.0)
                        };
                    }
                }
            });
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Rows").strong());
        let first = *self.rows.start();
        let mut fixed = self.work.rows.get(first).and_then(|r| r.min_height);
        ui.horizontal(|ui| {
            if ui.radio(fixed.is_none(), "As tall as the text").clicked() {
                fixed = None;
            }
            if ui.radio(fixed.is_some(), "At least").clicked() && fixed.is_none() {
                fixed = Some(24.0);
            }
            if let Some(h) = fixed.as_mut() {
                ui.add(egui::DragValue::new(h).range(6.0..=400.0).suffix(" pt"));
            }
        });
        for r in self.rows.clone() {
            if let Some(row) = self.work.rows.get_mut(r) {
                row.min_height = fixed;
            }
        }
        egui::CollapsingHeader::new("Advanced").id_salt("table-size-advanced").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Space inside cells");
                ui.add(egui::DragValue::new(&mut self.work.padding).range(0.0..=36.0).suffix(" pt"));
            });
        });
    }

    fn cells(&mut self, ui: &mut egui::Ui) {
        let (nr, nc) = (self.rows.clone().count(), self.cols.clone().count());
        ui.weak(if nr * nc == 1 {
            "Changes the outlined cell.".to_string()
        } else {
            format!("Changes the {nc} × {nr} outlined cells.")
        });
        ui.label(egui::RichText::new("Colour").strong());
        if let Some(c) = crate::app::swatches(ui, &crate::app::CELL_COLOURS, None) {
            self.each_cell(|cell| cell.shading = Some(c));
        }
        ui.horizontal(|ui| {
            ui.label("Your own:");
            if ui.color_edit_button_srgba(&mut self.custom).on_hover_text("Pick any colour").changed() {
                let c = Rgba::rgb(self.custom.r(), self.custom.g(), self.custom.b());
                self.each_cell(|cell| cell.shading = Some(c));
            }
            if ui.button("No colour").clicked() {
                self.each_cell(|cell| cell.shading = None);
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Text in the cell").strong());
        ui.horizontal(|ui| {
            for (v, n) in [(VAlign::Top, "Top"), (VAlign::Middle, "Middle"), (VAlign::Bottom, "Bottom")] {
                if ui.button(n).clicked() {
                    self.each_cell(|cell| cell.v_align = v);
                }
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Lines around").strong());
        let line = Border { width: 1.0, color: Rgba::rgb(0x40, 0x40, 0x40) };
        let none = ParaBorders::default();
        ui.horizontal_wrapped(|ui| {
            for (name, b) in [
                ("Box", Some(ParaBorders::all(line))),
                ("Line below", Some(ParaBorders { bottom: Some(line), ..none })),
                ("Line above", Some(ParaBorders { top: Some(line), ..none })),
                ("Like the table", None),
            ] {
                if ui.button(name).clicked() {
                    self.each_cell(|cell| cell.borders = b);
                }
            }
        });
    }
}
