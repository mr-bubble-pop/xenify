//! Hit-testing and caret/selection geometry in document space.

use crate::{DocLayout, Fragment};
use xenify_wp_doc::{Document, Pos, Selection};
use kurbo::{Point, Rect};

impl DocLayout {
    /// The fragment showing the caret at `pos`.
    fn fragment_at(&self, pos: Pos, upstream: bool) -> Option<&Fragment> {
        let frags = self.by_para.get(&pos.para)?;
        let first = &self.fragments[*frags.first()?];
        let line = first.para.line_of(pos.offset, upstream);
        frags.iter().map(|&i| &self.fragments[i]).find(|f| f.lines.contains(&line)).or(Some(first))
    }

    /// The position nearest to document-space point `p`.
    pub fn hit(&self, p: Point) -> Option<Pos> {
        // Inside a text box: only its own text (a click below its last line goes to
        // that line, not to the page text the box sits over).
        let frame = self.frames.iter().find(|f| f.rect.contains(p)).map(|f| f.rect.inflate(1.0, 1.0));
        // (Wholly inside: body text flowing above and below the box is one piece around it.)
        let inside = |r: kurbo::Rect| {
            frame.is_none_or(|fr| fr.contains(Point::new(r.x0, r.y0)) && fr.contains(Point::new(r.x1, r.y1)))
        };
        let f = self
            .fragments
            .iter()
            .filter(|f| f.para.id != xenify_wp_doc::NodeId(0) && inside(f.rect()))
            .min_by(|a, b| distance(a.rect(), p).total_cmp(&distance(b.rect(), p)))
            .or_else(|| {
                self.fragments
                    .iter()
                    .filter(|f| f.para.id != xenify_wp_doc::NodeId(0))
                    .min_by(|a, b| distance(a.rect(), p).total_cmp(&distance(b.rect(), p)))
            })?;
        let r = f.rect();
        // Clamp into the fragment so clicks above/below it pick its first/last line.
        let y = p.y.clamp(r.y0 + 0.5, (r.y1 - 0.5).max(r.y0 + 0.5)) - f.origin.y;
        let x = p.x - f.origin.x;
        Some(Pos::new(f.para.id, f.para.hit(x as f32, y as f32)))
    }

    /// The caret at `pos`, in document space.
    pub fn caret(&self, pos: Pos, upstream: bool) -> Option<Rect> {
        let f = self.fragment_at(pos, upstream)?;
        Some(f.para.caret(pos.offset, upstream) + f.origin.to_vec2())
    }

    /// Boxes covering `range` of paragraph `para` (spelling squiggles, AI
    /// suggestions, search hits), in document space.
    pub fn range_rects(&self, para: xenify_wp_doc::NodeId, range: std::ops::Range<usize>) -> Vec<Rect> {
        let Some(frags) = self.by_para.get(&para) else { return Vec::new() };
        let mut out = Vec::new();
        for &i in frags {
            let f = &self.fragments[i];
            let clip = f.rect();
            for r in f.para.range_rects(range.clone()) {
                let r = r + f.origin.to_vec2();
                if r.center().y >= clip.y0 && r.center().y <= clip.y1 {
                    out.push(r);
                }
            }
        }
        out
    }

    /// Boxes covering the selection, in document space. Whole paragraphs in
    /// the middle get a sliver past their end to show the paragraph break.
    pub fn selection_rects(&self, doc: &Document, sel: Selection) -> Vec<Rect> {
        if sel.is_collapsed() {
            return Vec::new();
        }
        let (a, b) = doc.ordered(sel);
        let mut out = Vec::new();
        let mut inside = false;
        for p in doc.paragraphs() {
            inside |= p.id == a.para;
            if !inside {
                continue;
            }
            let start = if p.id == a.para { a.offset } else { 0 };
            let end = if p.id == b.para { b.offset } else { p.text.len() };
            let mut rects = self.range_rects(p.id, start..end);
            if p.id != b.para {
                // Show the selected paragraph break.
                if let Some(c) = self.caret(Pos::new(p.id, p.text.len()), true) {
                    rects.push(Rect::new(c.x0, c.y0, c.x0 + 5.0, c.y1));
                }
            }
            out.extend(rects);
            if p.id == b.para {
                break;
            }
        }
        out
    }

    /// Moves `delta` lines up (negative) or down from `pos`, aiming for
    /// document-space x `goal_x`. Returns `None` past the first/last line.
    pub fn line_move(&self, pos: Pos, upstream: bool, delta: isize, goal_x: f64) -> Option<Pos> {
        let caret = self.caret(pos, upstream)?;
        // Every line in document order, as (fragment, line index).
        let lines: Vec<(usize, usize)> =
            self.fragments.iter().enumerate().flat_map(|(fi, f)| f.lines.clone().map(move |l| (fi, l))).collect();
        let here = lines.iter().position(|&(fi, l)| {
            let f = &self.fragments[fi];
            f.para.id == pos.para && {
                let lb = f.para.lines[l];
                let y = caret.center().y - f.origin.y;
                y >= lb.top as f64 && y < lb.bottom as f64
            }
        })?;
        let target = here.checked_add_signed(delta).filter(|&t| t < lines.len())?;
        let (fi, l) = lines[target];
        let f = &self.fragments[fi];
        let lb = f.para.lines[l];
        let y = (lb.top + lb.bottom) / 2.0;
        Some(Pos::new(f.para.id, f.para.hit((goal_x - f.origin.x) as f32, y)))
    }

    /// The start (or end) of the visual line holding `pos`.
    pub fn line_edge(&self, pos: Pos, upstream: bool, end: bool) -> Pos {
        match self.by_para.get(&pos.para).and_then(|v| v.first()) {
            Some(&i) => Pos::new(pos.para, self.fragments[i].para.line_edge(pos.offset, upstream, end)),
            None => pos,
        }
    }
}

fn distance(r: Rect, p: Point) -> f64 {
    let dy = if p.y < r.y0 {
        r.y0 - p.y
    } else if p.y > r.y1 {
        p.y - r.y1
    } else {
        0.0
    };
    // Vertical distance decides; horizontal only breaks ties (side by side columns).
    let dx = if p.x < r.x0 {
        r.x0 - p.x
    } else if p.x > r.x1 {
        p.x - r.x1
    } else {
        0.0
    };
    dy * 1000.0 + dx
}
