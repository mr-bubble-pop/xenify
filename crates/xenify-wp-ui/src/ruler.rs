//! The rulers above and beside the page. The horizontal one shows the page
//! margins (drag to change), the caret paragraph's first-line, left and
//! right indents (drag the markers) and its tab stops (click to add one of
//! the kind chosen at the left end, drag to move, drag off to remove). The
//! vertical one shows the top and bottom margins.

use crate::app::WriteApp;
use crate::page_setup::Unit;
use egui::{Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use xenify_wp_doc::{MergeKey, ParaProps, TabKind, TabStop, ops};

pub const SIZE: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    MarginLeft,
    MarginRight,
    MarginTop,
    MarginBottom,
    FirstLine,
    LeftIndent,
    RightIndent,
    /// A tab stop (index in the paragraph's tabs); `off` once dragged off the ruler.
    Tab {
        index: usize,
        off: bool,
    },
}

/// What the rulers remember between frames.
#[derive(Default)]
pub struct Ruler {
    drag: Option<Drag>,
    /// The kind of tab stop a click adds.
    pub tab_kind: TabKind,
}

/// Where things are, in document points.
struct Geometry {
    page: kurbo::Rect,
    body: kurbo::Rect,
    section: usize,
    /// The caret paragraph's column: its left edge and width.
    col_left: f32,
    col_width: f32,
    para: Option<xenify_wp_doc::ResolvedPara>,
}

fn geometry(app: &WriteApp) -> Option<Geometry> {
    let layout = &app.view.layout;
    let page_i = app.view.caret_page(&app.editor).or(if layout.pages.is_empty() { None } else { Some(0) })?;
    let page = layout.pages.get(page_i)?;
    let focus = app.editor.sel.focus.para;
    let frag = layout
        .fragments
        .iter()
        .find(|f| f.para.id == focus && page.rect.contains(f.rect().center()))
        .or_else(|| layout.fragments.iter().find(|f| f.para.id == focus));
    let (col_left, col_width) =
        frag.map_or((page.body.x0 as f32, page.body.width() as f32), |f| (f.origin.x as f32, f.para.width));
    let para = app.editor.doc.para(focus).map(|p| app.editor.doc.resolve_para(p));
    Some(Geometry { page: page.rect, body: page.body, section: page.section, col_left, col_width, para })
}

/// Tick spacing for `unit`: (major step, minor steps per major), in points.
fn ticks(unit: Unit) -> (f32, usize) {
    match unit {
        Unit::In => (72.0, 8),
        Unit::Pt => (72.0, 6),
        Unit::Cm | Unit::Mm => (Unit::Cm.pt(), 4),
    }
}

/// Draws the symbol of a tab stop of `kind` with its foot at `at`.
fn tab_symbol(p: &egui::Painter, at: Pos2, kind: TabKind, color: Color32) {
    let s = Stroke::new(1.6, color);
    let top = at + Vec2::new(0.0, -7.0);
    p.line_segment([top, at], s);
    match kind {
        TabKind::Left => {
            p.line_segment([at, at + Vec2::new(6.0, 0.0)], s);
        }
        TabKind::Right => {
            p.line_segment([at + Vec2::new(-6.0, 0.0), at], s);
        }
        TabKind::Center => {
            p.line_segment([at + Vec2::new(-5.0, 0.0), at + Vec2::new(5.0, 0.0)], s);
        }
        TabKind::Decimal => {
            p.line_segment([at + Vec2::new(-5.0, 0.0), at + Vec2::new(5.0, 0.0)], s);
            p.circle_filled(at + Vec2::new(3.0, -4.0), 1.2, color);
        }
    }
}

fn kind_name(k: TabKind) -> &'static str {
    match k {
        TabKind::Left => "Left tab",
        TabKind::Center => "Centre tab",
        TabKind::Right => "Right tab",
        TabKind::Decimal => "Decimal tab",
    }
}

/// A triangle marker pointing down (`down`) or up, its tip at `tip`.
fn marker(p: &egui::Painter, tip: Pos2, down: bool, fill: Color32, stroke: Color32) {
    let d = if down { -1.0 } else { 1.0 };
    let pts = vec![tip, tip + Vec2::new(-5.0, 6.0 * d), tip + Vec2::new(5.0, 6.0 * d)];
    p.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, stroke)));
}

/// The horizontal ruler, as wide as `ui`.
pub fn horizontal(app: &mut WriteApp, ui: &mut Ui) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SIZE), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Ruler"));
    let Some(g) = geometry(app) else { return };
    let s = app.view.scale();
    let ox = app.view.doc_origin.x;
    let sx = |x: f32| ox + x * s;
    let dx = |px: f32| (px - ox) / s;
    let v = ui.visuals().clone();
    let p = ui.painter_at(rect);
    let (paper, shade, ink) = if v.dark_mode {
        (Color32::from_rgb(0x2e, 0x33, 0x3b), Color32::from_rgb(0x22, 0x26, 0x2c), v.text_color())
    } else {
        (Color32::WHITE, Color32::from_rgb(0xdd, 0xe1, 0xe7), Color32::from_rgb(0x4a, 0x50, 0x5a))
    };
    let band = Rect::from_min_max(
        Pos2::new(sx(g.page.x0 as f32), rect.top() + 3.0),
        Pos2::new(sx(g.page.x1 as f32), rect.bottom() - 3.0),
    );
    p.rect_filled(band, 2.0, shade);
    let text_area =
        Rect::from_min_max(Pos2::new(sx(g.body.x0 as f32), band.top()), Pos2::new(sx(g.body.x1 as f32), band.bottom()));
    p.rect_filled(text_area, 0.0, paper);
    // Gaps between columns are shaded like margins.
    let ps = app.editor.doc.sections.get(g.section).map(|s| s.page.clone()).unwrap_or_default();
    if ps.columns > 1 && !app.editor.doc.meta.pageless {
        let cw = ps.column_width();
        for k in 1..ps.columns {
            let x0 = g.body.x0 as f32 + k as f32 * (cw + ps.column_gap) - ps.column_gap;
            let gap =
                Rect::from_min_max(Pos2::new(sx(x0), band.top()), Pos2::new(sx(x0 + ps.column_gap), band.bottom()));
            p.rect_filled(gap, 0.0, shade);
        }
    }
    // Ticks from the left margin.
    let (major, minor) = ticks(app.unit());
    let step = major / minor as f32;
    let zero = g.body.x0 as f32;
    let font = egui::FontId::proportional(9.0);
    let mut i = -((zero - g.page.x0 as f32) / step).floor() as i32;
    loop {
        let x = zero + i as f32 * step;
        if x > g.page.x1 as f32 {
            break;
        }
        let px = sx(x);
        let is_major = i.rem_euclid(minor as i32) == 0;
        if is_major && i != 0 {
            let n = i.unsigned_abs() as usize / minor;
            p.text(Pos2::new(px, band.center().y), egui::Align2::CENTER_CENTER, n.to_string(), font.clone(), ink);
        } else if !is_major {
            let h = if i.rem_euclid(minor as i32) == (minor as i32) / 2 { 4.0 } else { 2.0 };
            p.line_segment(
                [Pos2::new(px, band.center().y - h), Pos2::new(px, band.center().y + h)],
                Stroke::new(1.0, ink.gamma_multiply(0.6)),
            );
        }
        i += 1;
    }

    // The tab-kind chooser at the left end.
    let chooser = Rect::from_min_size(rect.min + Vec2::new(4.0, 2.0), Vec2::splat(SIZE - 4.0));
    let chooser_resp = ui.interact(chooser, ui.id().with("tab-kind"), Sense::click());
    p.rect_filled(
        chooser,
        3.0,
        if chooser_resp.hovered() { v.widgets.hovered.bg_fill } else { v.widgets.inactive.bg_fill },
    );
    tab_symbol(&p, chooser.center() + Vec2::new(0.0, 4.0), app.ruler.tab_kind, ink);
    let chooser_resp = chooser_resp.on_hover_text(format!(
        "Clicking the ruler adds a {} stop (click here to change)",
        kind_name(app.ruler.tab_kind).to_lowercase()
    ));
    if chooser_resp.clicked() {
        app.ruler.tab_kind = match app.ruler.tab_kind {
            TabKind::Left => TabKind::Center,
            TabKind::Center => TabKind::Right,
            TabKind::Right => TabKind::Decimal,
            TabKind::Decimal => TabKind::Left,
        };
    }

    // The paragraph's indents and tab stops.
    let Some(r) = g.para.clone() else { return };
    let left = g.col_left + r.indent_left;
    let first = left + r.first_line;
    let right = g.col_left + g.col_width - r.indent_right;
    let accent = crate::theme::accent(&v);
    marker(&p, Pos2::new(sx(first), band.top() + 6.0), true, paper, ink);
    marker(&p, Pos2::new(sx(left), band.bottom() - 6.0), false, paper, ink);
    marker(&p, Pos2::new(sx(right), band.bottom() - 6.0), false, paper, ink);
    for t in &r.tabs {
        tab_symbol(&p, Pos2::new(sx(g.col_left + t.pos), band.bottom() - 1.0), t.kind, accent);
    }
    // Default stops after the last one: small marks under the ruler.
    let last = r.tabs.last().map_or(0.0, |t| t.pos);
    let mut d = ((last / xenify_wp_doc::DEFAULT_TAB).floor() + 1.0) * xenify_wp_doc::DEFAULT_TAB;
    while g.col_left + d < right {
        let px = sx(g.col_left + d);
        p.line_segment(
            [Pos2::new(px, band.bottom() - 2.0), Pos2::new(px, band.bottom())],
            Stroke::new(1.0, ink.gamma_multiply(0.5)),
        );
        d += xenify_wp_doc::DEFAULT_TAB;
    }

    // Hover cursors and tooltips. The markers take the top and bottom
    // strips; the middle of a margin's edge moves the margin (they often
    // share a place, as when nothing is indented).
    let near = |px: f32, x: f32| (px - sx(x)).abs() <= 5.0;
    let strip = band.height() / 3.0;
    let hit = |pos: Pos2| -> Option<Drag> {
        let (top, bottom) = (pos.y < band.top() + strip, pos.y > band.bottom() - strip);
        if top && near(pos.x, first) {
            return Some(Drag::FirstLine);
        }
        if bottom && near(pos.x, left) {
            return Some(Drag::LeftIndent);
        }
        if bottom && near(pos.x, right) {
            return Some(Drag::RightIndent);
        }
        if let Some(i) = r.tabs.iter().position(|t| near(pos.x, g.col_left + t.pos)) {
            return Some(Drag::Tab { index: i, off: false });
        }
        if near(pos.x, g.body.x0 as f32) {
            return Some(Drag::MarginLeft);
        }
        if near(pos.x, g.body.x1 as f32) {
            return Some(Drag::MarginRight);
        }
        None
    };
    if let Some(h) = resp.hover_pos().and_then(hit) {
        let tip = match h {
            Drag::FirstLine => "First-line indent",
            Drag::LeftIndent => "Left indent (the first line keeps its place)",
            Drag::RightIndent => "Right indent",
            Drag::Tab { .. } => "Tab stop: drag to move, drag off the ruler to remove",
            Drag::MarginLeft => "Left margin: drag to change",
            Drag::MarginRight => "Right margin: drag to change",
            _ => "",
        };
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        resp.clone().on_hover_text(tip);
    }
    // The margin edge being pointed at or dragged.
    let lit = app.ruler.drag.or_else(|| resp.hover_pos().and_then(hit));
    for (d, x) in [(Drag::MarginLeft, g.body.x0), (Drag::MarginRight, g.body.x1)] {
        if lit == Some(d) {
            let px = sx(x as f32);
            p.line_segment([Pos2::new(px, band.top()), Pos2::new(px, band.bottom())], Stroke::new(2.0, accent));
        }
    }

    // Where the button went down (the pointer has moved on by the time a drag starts).
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        app.editor.seal();
        app.ruler.drag = hit(pos);
    }
    if let (Some(drag), Some(pos)) = (app.ruler.drag, resp.interact_pointer_pos())
        && resp.dragged()
    {
        let x = dx(pos.x);
        let snap = |v: f32| (v / (step / 2.0)).round() * (step / 2.0);
        let merge = Some(MergeKey::Control("ruler".into()));
        let mut props = ParaProps::default();
        match drag {
            Drag::FirstLine => props.first_line = Some(snap(x - left)),
            Drag::LeftIndent => {
                // Out into the margin too (a negative indent), as far as the page edge.
                let new_left = snap(x - g.col_left).clamp(g.page.x0 as f32 - g.col_left, g.col_width - 36.0);
                props.indent_left = Some(new_left);
                props.first_line = Some(first - (g.col_left + new_left));
            }
            Drag::RightIndent => {
                props.indent_right = Some(
                    snap(g.col_left + g.col_width - x)
                        .clamp(g.col_left + g.col_width - g.page.x1 as f32, g.col_width - 36.0),
                )
            }
            Drag::Tab { index, .. } => {
                let off = (pos.y - rect.center().y).abs() > SIZE * 1.5;
                app.ruler.drag = Some(Drag::Tab { index, off });
                let mut tabs = r.tabs.clone();
                if let Some(t) = tabs.get_mut(index) {
                    t.pos = snap(x - g.col_left).max(0.0);
                }
                tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
                let moved =
                    tabs.iter().position(|t| (t.pos - snap(x - g.col_left).max(0.0)).abs() < 0.01).unwrap_or(index);
                app.ruler.drag = Some(Drag::Tab { index: moved, off });
                props.tabs = Some(tabs);
            }
            Drag::MarginLeft | Drag::MarginRight => {
                if let Some(sec) = app.editor.doc.sections.get(g.section) {
                    let (start, mut page) = (sec.start, sec.page.clone());
                    if drag == Drag::MarginLeft {
                        page.margins.left =
                            snap(x - g.page.x0 as f32).clamp(0.0, page.width - page.margins.right - 72.0);
                    } else {
                        page.margins.right =
                            snap(g.page.x1 as f32 - x).clamp(0.0, page.width - page.margins.left - 72.0);
                    }
                    let _ = ops::set_section(&mut app.editor, g.section, start, page);
                }
            }
            Drag::MarginTop | Drag::MarginBottom => {}
        }
        if props != ParaProps::default() {
            let _ = ops::apply_para_props_merged(&mut app.editor, &props, "Ruler", merge);
        }
    }
    if resp.drag_stopped() {
        if let Some(Drag::Tab { index, off: true }) = app.ruler.drag {
            let mut tabs = r.tabs.clone();
            if index < tabs.len() {
                tabs.remove(index);
                let p = ParaProps { tabs: Some(tabs), ..Default::default() };
                let _ =
                    ops::apply_para_props_merged(&mut app.editor, &p, "Ruler", Some(MergeKey::Control("ruler".into())));
            }
        }
        app.ruler.drag = None;
        app.editor.seal();
    }
    // A click on empty ruler inside the column: a new tab stop.
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && hit(pos).is_none()
        && !chooser.contains(pos)
    {
        let x = dx(pos.x);
        if x > g.col_left && x < g.col_left + g.col_width {
            let snap = (x - g.col_left) / (step / 2.0);
            let at = snap.round() * (step / 2.0);
            let mut tabs = r.tabs.clone();
            tabs.retain(|t| (t.pos - at).abs() > 0.5);
            tabs.push(TabStop { pos: at, kind: app.ruler.tab_kind, ..Default::default() });
            tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
            app.editor.seal();
            let _ = ops::apply_para_props(
                &mut app.editor,
                &ParaProps { tabs: Some(tabs), ..Default::default() },
                "Add tab stop",
            );
            app.editor.seal();
        }
    }
}

/// The vertical ruler beside the page: top and bottom margins.
pub fn vertical(app: &mut WriteApp, ui: &mut Ui) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(SIZE, ui.available_height()), Sense::drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Vertical ruler"));
    let Some(g) = geometry(app) else { return };
    let s = app.view.scale();
    let oy = app.view.doc_origin.y;
    let sy = |y: f32| oy + y * s;
    let v = ui.visuals().clone();
    let p = ui.painter_at(rect);
    let (paper, shade, ink) = if v.dark_mode {
        (Color32::from_rgb(0x2e, 0x33, 0x3b), Color32::from_rgb(0x22, 0x26, 0x2c), v.text_color())
    } else {
        (Color32::WHITE, Color32::from_rgb(0xdd, 0xe1, 0xe7), Color32::from_rgb(0x4a, 0x50, 0x5a))
    };
    let band = Rect::from_min_max(
        Pos2::new(rect.left() + 3.0, sy(g.page.y0 as f32)),
        Pos2::new(rect.right() - 3.0, sy(g.page.y1 as f32)),
    );
    p.rect_filled(band, 2.0, shade);
    p.rect_filled(
        Rect::from_min_max(Pos2::new(band.left(), sy(g.body.y0 as f32)), Pos2::new(band.right(), sy(g.body.y1 as f32))),
        0.0,
        paper,
    );
    let (major, minor) = ticks(app.unit());
    let step = major / minor as f32;
    let font = egui::FontId::proportional(9.0);
    let mut i = 1;
    loop {
        let y = g.body.y0 as f32 + i as f32 * step;
        if y >= g.body.y1 as f32 {
            break;
        }
        let py = sy(y);
        if i % minor == 0 {
            p.text(
                Pos2::new(band.center().x, py),
                egui::Align2::CENTER_CENTER,
                (i / minor).to_string(),
                font.clone(),
                ink,
            );
        } else {
            let w = if i % minor == minor / 2 { 4.0 } else { 2.0 };
            p.line_segment(
                [Pos2::new(band.center().x - w, py), Pos2::new(band.center().x + w, py)],
                Stroke::new(1.0, ink.gamma_multiply(0.6)),
            );
        }
        i += 1;
    }
    let near = |py: f32, y: f64| (py - sy(y as f32)).abs() <= 5.0;
    let hit = |pos: Pos2| {
        if near(pos.y, g.body.y0) {
            Some(Drag::MarginTop)
        } else if near(pos.y, g.body.y1) {
            Some(Drag::MarginBottom)
        } else {
            None
        }
    };
    if let Some(h) = resp.hover_pos().and_then(hit) {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        resp.clone().on_hover_text(if h == Drag::MarginTop {
            "Top margin: drag to change"
        } else {
            "Bottom margin: drag to change"
        });
    }
    let lit = app.ruler.drag.or_else(|| resp.hover_pos().and_then(hit));
    for (d, y) in [(Drag::MarginTop, g.body.y0), (Drag::MarginBottom, g.body.y1)] {
        if lit == Some(d) {
            let py = sy(y as f32);
            p.line_segment(
                [Pos2::new(band.left(), py), Pos2::new(band.right(), py)],
                Stroke::new(2.0, crate::theme::accent(&v)),
            );
        }
    }
    // Where the button went down (the pointer has moved on by the time a drag starts).
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        app.editor.seal();
        app.ruler.drag = hit(pos);
    }
    if let (Some(drag @ (Drag::MarginTop | Drag::MarginBottom)), Some(pos)) =
        (app.ruler.drag, resp.interact_pointer_pos())
        && resp.dragged()
        && let Some(sec) = app.editor.doc.sections.get(g.section)
    {
        let y = (pos.y - oy) / s;
        let snap = |v: f32| (v / (step / 2.0)).round() * (step / 2.0);
        let (start, mut page) = (sec.start, sec.page.clone());
        if drag == Drag::MarginTop {
            page.margins.top = snap(y - g.page.y0 as f32).clamp(0.0, page.height - page.margins.bottom - 72.0);
        } else {
            page.margins.bottom = snap(g.page.y1 as f32 - y).clamp(0.0, page.height - page.margins.top - 72.0);
        }
        let _ = ops::set_section(&mut app.editor, g.section, start, page);
    }
    if resp.drag_stopped() {
        app.ruler.drag = None;
        app.editor.seal();
    }
}
