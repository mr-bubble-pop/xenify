//! What screen readers see of the document: the page is a multi-line text
//! field, each laid-out line a text run (its text, where each character
//! sits on screen and where words start), and the caret or selection a
//! range across those runs. Orca, NVDA and VoiceOver read lines, words and
//! characters as the caret moves, and "say all" reads on from it.
//!
//! Only built while a screen reader (or a test) asks for the tree. Lines on
//! screen are sent, plus the caret's and the selection anchor's.

use egui::accesskit;
use xenify_wp_doc::{Document, NodeId, Pos, Selection};
use xenify_wp_layout::DocLayout;
use std::collections::HashMap;
use std::sync::Arc;

/// AccessKit stores word starts as `u8`, so a run holds at most this many characters.
const MAX_RUN: usize = 255;

/// One line's text and character geometry, in document points from the
/// line's left edge.
struct Line {
    /// Byte range of the paragraph's text on this line.
    bytes: (usize, usize),
    text: String,
    lengths: Vec<u8>,
    xs: Vec<f32>,
    widths: Vec<f32>,
    word_starts: Vec<usize>,
    /// The line's box in document space.
    rect: kurbo::Rect,
}

/// Lines worked out for one layout (cleared when the document changes).
#[derive(Default)]
pub struct Cache {
    revision: Option<u64>,
    lines: HashMap<(NodeId, usize), Arc<Line>>,
}

/// Where everything goes on screen: document space to screen points.
pub struct Placement {
    pub origin: egui::Pos2,
    pub scale: f32,
    /// The part of the document on screen, in document space.
    pub visible: kurbo::Rect,
}

impl Placement {
    fn rect(&self, r: kurbo::Rect) -> accesskit::Rect {
        let s = self.scale as f64;
        let (ox, oy) = (self.origin.x as f64, self.origin.y as f64);
        accesskit::Rect { x0: ox + r.x0 * s, y0: oy + r.y0 * s, x1: ox + r.x1 * s, y1: oy + r.y1 * s }
    }
}

/// Describes the document for screen readers. `id` is the document area's
/// widget (it must already have a node: call after its `widget_info`).
#[allow(clippy::too_many_arguments)]
pub fn publish(
    ui: &mut egui::Ui,
    id: egui::Id,
    cache: &mut Cache,
    revision: u64,
    layout: &DocLayout,
    doc: &Document,
    sel: Selection,
    upstream: bool,
    read_only: bool,
    at: &Placement,
) {
    let ctx = ui.ctx().clone();
    let on = ctx
        .accesskit_node_builder(id, |n| {
            n.set_role(accesskit::Role::MultilineTextInput);
            n.set_label("Document");
            if read_only {
                n.set_read_only();
            }
        })
        .is_some();
    if !on {
        return;
    }
    if cache.revision != Some(revision) {
        cache.revision = Some(revision);
        cache.lines.clear();
    }
    let wanted = |p: Pos, up: bool| {
        let frags = layout.by_para.get(&p.para)?;
        let para = &layout.fragments[*frags.first()?].para;
        Some((p.para, para.line_of(p.offset, up)))
    };
    let focus_line = wanted(sel.focus, upstream);
    let anchor_line = wanted(sel.anchor, false);

    // Runs in reading order: the document's fragments, top to bottom.
    for f in &layout.fragments {
        let pid = f.para.id;
        if pid == NodeId(0) {
            continue;
        }
        let show_all = f.rect().overlaps(at.visible.inflate(0.0, at.visible.height()));
        for li in f.lines.clone() {
            let key = (pid, li);
            if !show_all && Some(key) != focus_line && Some(key) != anchor_line {
                continue;
            }
            let line = cache.lines.entry(key).or_insert_with(|| Arc::new(measure(f, li, doc))).clone();
            emit(ui, id, key, &line, at);
        }
    }

    let pos = |p: Pos, up: bool| -> Option<accesskit::TextPosition> {
        let key = wanted(p, up)?;
        let line = cache.lines.get(&key)?;
        let text = &line.text;
        let upto = p.offset.clamp(line.bytes.0, line.bytes.1) - line.bytes.0;
        let chars = text.get(..upto.min(text.len())).map_or(0, |s| s.chars().count());
        // A position on a run boundary belongs to the end of the earlier run.
        let chunk = if chars > 0 && chars.is_multiple_of(MAX_RUN) { chars / MAX_RUN - 1 } else { chars / MAX_RUN };
        Some(accesskit::TextPosition {
            node: run_id(id, key, chunk).accesskit_id(),
            character_index: chars - chunk * MAX_RUN,
        })
    };
    if let (Some(anchor), Some(focus)) = (pos(sel.anchor, false), pos(sel.focus, upstream)) {
        ctx.accesskit_node_builder(id, |n| n.set_text_selection(accesskit::TextSelection { anchor, focus }));
    }
}

fn run_id(doc: egui::Id, (para, line): (NodeId, usize), chunk: usize) -> egui::Id {
    doc.with(("line", para.0, line, chunk))
}

/// Works out line `li` of fragment `f`: its text and where each character is.
fn measure(f: &xenify_wp_layout::Fragment, li: usize, doc: &Document) -> Line {
    let para = &f.para;
    let lb = para.lines[li];
    let full = doc.para(para.id).map(|p| p.text.as_str()).unwrap_or("");
    let end = lb.text.1.min(full.len()).min(para.text_len);
    let start = lb.text.0.min(end);
    // Parley's line ranges are byte ranges of the same text; stay on char boundaries.
    let (start, end) = (floor_char(full, start), floor_char(full, end));
    let text_part = &full[start..end];
    let x_at = |off: usize| para.caret(off, false).x0 as f32;
    let left = x_at(start);
    let mut text = String::with_capacity(text_part.len() + 1);
    let (mut lengths, mut xs, mut widths, mut word_starts) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut after_word = li > 0 && start > 0;
    for (i, ch) in text_part.char_indices() {
        let off = start + i;
        let x0 = x_at(off);
        let x1 = para.caret(off + ch.len_utf8(), true).x0 as f32;
        let word = ch.is_alphanumeric() || ch == '_';
        if word && after_word {
            word_starts.push(lengths.len());
        }
        after_word = !word;
        text.push(ch);
        lengths.push(ch.len_utf8() as u8);
        xs.push(x0 - left);
        widths.push((x1 - x0).max(0.0));
    }
    let last_line = li + 1 == para.lines.len();
    let right = left + xs.last().copied().unwrap_or(0.0) + widths.last().copied().unwrap_or(0.0);
    if last_line {
        // The paragraph break, so line and paragraph moves read as they should.
        text.push('\n');
        lengths.push(1);
        xs.push(right - left);
        widths.push(0.0);
    }
    let o = f.origin.to_vec2();
    let rect =
        kurbo::Rect::new(left as f64, lb.top as f64, (right as f64).max(left as f64 + 1.0), lb.bottom as f64) + o;
    Line { bytes: (start, end), text, lengths, xs, widths, word_starts, rect }
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Sends one line as text runs (more than one when it's very long).
fn emit(ui: &mut egui::Ui, doc: egui::Id, key: (NodeId, usize), line: &Line, at: &Placement) {
    let n = line.lengths.len();
    let chunks = n.div_ceil(MAX_RUN).max(1);
    let mut byte = 0;
    for c in 0..chunks {
        let (a, b) = (c * MAX_RUN, ((c + 1) * MAX_RUN).min(n));
        let len: usize = line.lengths[a..b].iter().map(|&l| l as usize).sum();
        let value = line.text[byte..byte + len].to_owned();
        byte += len;
        let rid = run_id(doc, key, c);
        let x0 = line.xs.get(a).copied().unwrap_or(0.0);
        let x1 = if b > a { line.xs[b - 1] + line.widths[b - 1] } else { x0 };
        let r = kurbo::Rect::new(line.rect.x0 + x0 as f64, line.rect.y0, line.rect.x0 + x1 as f64, line.rect.y1);
        let bounds = at.rect(r);
        // A child Ui is how egui makes a node a child of another one.
        let corner = egui::pos2(bounds.x0 as f32, bounds.y0 as f32);
        let _ = ui.new_child(
            egui::UiBuilder::new()
                .id(rid)
                .accessibility_parent(doc)
                .max_rect(egui::Rect::from_min_size(corner, egui::Vec2::ZERO)),
        );
        ui.ctx().accesskit_node_builder(rid, |node| {
            node.set_role(accesskit::Role::TextRun);
            node.set_text_direction(accesskit::TextDirection::LeftToRight);
            node.set_bounds(bounds);
            node.set_value(value);
            node.set_character_lengths(line.lengths[a..b].to_vec());
            let s = at.scale;
            node.set_character_positions(line.xs[a..b].iter().map(|x| (x - x0) * s).collect::<Vec<_>>());
            node.set_character_widths(line.widths[a..b].iter().map(|w| w * s).collect::<Vec<_>>());
            node.set_word_starts(
                line.word_starts.iter().filter(|&&w| w >= a && w < b).map(|&w| (w - a) as u8).collect::<Vec<_>>(),
            );
            if c > 0 {
                node.set_previous_on_line(run_id(doc, key, c - 1).accesskit_id());
            }
            if c + 1 < chunks {
                node.set_next_on_line(run_id(doc, key, c + 1).accesskit_id());
            }
        });
    }
}
