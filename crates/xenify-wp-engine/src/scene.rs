//! What is drawn on each page, as display lists the screen paints with a
//! 2D canvas.

use crate::glyphs::FontTable;
use xenify_wp_doc::Rgba;
use xenify_wp_layout::{DocLayout, Fragment, Item};
use kurbo::{Rect, Vec2};
use serde::Serialize;
use std::hash::{Hash, Hasher};

/// A page: where it is in document space, and a signature that changes
/// whenever anything drawn on it does (the screen redraws only those).
#[derive(Clone, Debug, Serialize)]
pub struct PageInfo {
    pub index: usize,
    /// The number printed on it.
    pub number: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// The text area inside the margins (page space), for the ruler.
    pub body: [f64; 4],
    /// Text columns and the space between them (screen-only guides show them).
    pub columns: u8,
    pub column_gap: f64,
    pub sig: String,
}

/// One drawing operation, in page space (points from the page's top-left).
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "t")]
pub enum Op {
    /// A filled rectangle.
    #[serde(rename = "r")]
    Rect { x: f32, y: f32, w: f32, h: f32, c: String },
    /// Glyphs of font `f` (see [`FontTable`]) at size `s`; `g` holds
    /// glyph id, x, y for each glyph; `k` slants them (degrees).
    #[serde(rename = "g")]
    Glyphs {
        f: u32,
        s: f32,
        c: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        k: Option<f32>,
        g: Vec<f32>,
    },
    /// A picture (asset key `src`).
    #[serde(rename = "i")]
    Image { x: f32, y: f32, w: f32, h: f32, src: String, alt: String },
    /// An emoji drawn as a picture.
    #[serde(rename = "e")]
    Emoji { x: f32, y: f32, w: f32, h: f32, e: String },
}

/// Everything drawn on one page.
#[derive(Clone, Debug, Serialize)]
pub struct PageScene {
    pub index: usize,
    pub sig: String,
    /// The paper colour.
    pub paper: String,
    pub ops: Vec<Op>,
}

pub fn css(c: Rgba) -> String {
    let [r, g, b, a] = c.0;
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// The page each fragment is drawn on (by where its middle is).
fn page_of(layout: &DocLayout, r: Rect) -> Option<usize> {
    let y = (r.y0 + r.y1) / 2.0;
    layout
        .pages
        .iter()
        .position(|p| y >= p.rect.y0 && y < p.rect.y1)
        .or_else(|| layout.pages.iter().position(|p| p.rect.overlaps(r)))
}

fn on_page(layout: &DocLayout, page: usize) -> impl Iterator<Item = &Fragment> {
    layout.fragments.iter().chain(&layout.decor).filter(move |f| page_of(layout, f.rect()) == Some(page))
}

pub fn pages(layout: &DocLayout, doc: &xenify_wp_doc::Document) -> Vec<PageInfo> {
    layout
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            p.rect.x0.to_bits().hash(&mut h);
            p.rect.y0.to_bits().hash(&mut h);
            p.rect.width().to_bits().hash(&mut h);
            p.rect.height().to_bits().hash(&mut h);
            p.number.hash(&mut h);
            for f in on_page(layout, i) {
                (std::sync::Arc::as_ptr(&f.para) as usize).hash(&mut h);
                f.origin.x.to_bits().hash(&mut h);
                f.origin.y.to_bits().hash(&mut h);
                f.lines.hash(&mut h);
            }
            for s in layout.shapes.iter().filter(|s| s.rect.overlaps(p.rect)) {
                for v in [s.rect.x0, s.rect.y0, s.rect.x1, s.rect.y1] {
                    v.to_bits().hash(&mut h);
                }
                s.color.0.hash(&mut h);
            }
            for im in layout.images.iter().filter(|im| im.rect.overlaps(p.rect)) {
                im.src.hash(&mut h);
                for v in [im.rect.x0, im.rect.y0, im.rect.x1, im.rect.y1] {
                    v.to_bits().hash(&mut h);
                }
            }
            layout.page_color.map(|c| c.0).hash(&mut h);
            let b = p.body - p.rect.origin().to_vec2();
            PageInfo {
                index: i,
                number: p.number,
                x: p.rect.x0,
                y: p.rect.y0,
                w: p.rect.width(),
                h: p.rect.height(),
                body: [b.x0, b.y0, b.width(), b.height()],
                columns: doc.sections.get(p.section).map_or(1, |s| s.page.columns.max(1)),
                column_gap: doc.sections.get(p.section).map_or(0.0, |s| s.page.column_gap as f64),
                sig: format!("{:x}", h.finish()),
            }
        })
        .collect()
}

fn rect_op(r: Rect, off: Vec2, c: Rgba) -> Op {
    let r = r + off;
    Op::Rect { x: r.x0 as f32, y: r.y0 as f32, w: r.width() as f32, h: r.height() as f32, c: css(c) }
}

/// The display list of page `index`.
pub fn page_scene(layout: &DocLayout, index: usize, fonts: &mut FontTable, sig: String) -> Option<PageScene> {
    let page = layout.pages.get(index)?;
    let to_page = -page.rect.origin().to_vec2();
    let mut ops = Vec::new();
    let paper = layout.page_color.map_or_else(|| "#ffffff".to_string(), css);
    for s in layout.shapes.iter().filter(|s| !s.front && s.rect.overlaps(page.rect)) {
        ops.push(rect_op(s.rect, to_page, s.color));
    }
    for im in layout.images.iter().filter(|im| im.rect.overlaps(page.rect)) {
        let r = im.rect + to_page;
        ops.push(Op::Image {
            x: r.x0 as f32,
            y: r.y0 as f32,
            w: r.width() as f32,
            h: r.height() as f32,
            src: im.src.clone(),
            alt: im.alt.clone(),
        });
    }
    for s in layout.shapes.iter().filter(|s| s.front && s.rect.overlaps(page.rect)) {
        ops.push(rect_op(s.rect, to_page, s.color));
    }
    for f in on_page(layout, index) {
        let off = f.origin.to_vec2() + to_page;
        // Screen-only marks (hints, broken fields) too: this is the screen.
        for item in f.para.items.iter().chain(&f.para.screen) {
            if !f.lines.contains(&item.line()) {
                continue;
            }
            match item {
                Item::Rect { rect, color, .. } => ops.push(rect_op(*rect, off, *color)),
                Item::Emoji { rect, glyph, .. } => {
                    let r = *rect + off;
                    ops.push(Op::Emoji {
                        x: r.x0 as f32,
                        y: r.y0 as f32,
                        w: r.width() as f32,
                        h: r.height() as f32,
                        e: glyph.clone(),
                    });
                }
                Item::Glyphs { font, size, coords, glyphs, color, skew, .. } => {
                    let mut g = Vec::with_capacity(glyphs.len() * 3);
                    for gl in glyphs {
                        g.push(gl.id as f32);
                        g.push(n2(gl.x + off.x as f32));
                        g.push(n2(gl.y + off.y as f32));
                    }
                    ops.push(Op::Glyphs { f: fonts.id(font, coords), s: *size, c: css(*color), k: *skew, g });
                }
            }
        }
    }
    Some(PageScene { index, sig, paper, ops })
}

fn n2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}
