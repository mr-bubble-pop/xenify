//! Xenify Write layout.
//!
//! Each paragraph is laid out with parley (rich text: the paragraph style
//! plus direct formatting runs) into a [`ParaLayout`], which carries a
//! display list of glyph runs and rectangles in paragraph space. A
//! [`DocLayout`] places paragraph fragments in document space; the screen
//! painter, PDF export and golden tests all draw from the same display
//! lists, so what prints matches what is on screen.
//!
//! Units are points (1/72 inch).

mod blocks;
pub mod emoji;
pub mod equation;
mod fonts;
mod geometry;
#[cfg(feature = "offscreen")]
pub mod offscreen;
mod paginate;
#[cfg(feature = "vello")]
pub mod paint;
mod para;

pub use blocks::toc_entries;
pub use paginate::now;
pub use paginate::{PAGE_GAP, fill_fields};

pub use fonts::{
    Fonts, INTER_FONT, SERIF_FAMILY, SERIF_FONTS, SYMBOL_FONT, provide_late_fonts, serif_fonts, symbol_font,
};
pub use para::{COMMENT_HIGHLIGHT, Glyph, Item, LineBox, ParaLayout, author_color};

use xenify_wp_doc::{Document, NodeId};
use kurbo::{Point, Rect};
use std::collections::HashMap;
use std::sync::Arc;

/// A run of lines of one paragraph placed in document space.
#[derive(Clone)]
pub struct Fragment {
    pub para: Arc<ParaLayout>,
    /// Where the paragraph's (0, 0) is, in document space.
    pub origin: Point,
    /// The lines of the paragraph shown here.
    pub lines: std::ops::Range<usize>,
}

impl Fragment {
    /// The fragment's box in document space (the lines it shows).
    pub fn rect(&self) -> Rect {
        let first = &self.para.lines[self.lines.start];
        let last = &self.para.lines[self.lines.end - 1];
        Rect::new(0.0, first.top as f64, self.para.width as f64, last.bottom as f64) + self.origin.to_vec2()
    }
}

/// A filled rectangle: cell shading (behind text) or a table line (in front).
#[derive(Clone, Debug)]
pub struct Shape {
    pub rect: Rect,
    pub color: xenify_wp_doc::Rgba,
    pub front: bool,
}

/// A picture placed in document space.
#[derive(Clone, Debug)]
pub struct PlacedImage {
    pub id: NodeId,
    pub src: String,
    pub alt: String,
    pub rect: Rect,
}

/// A frame placed on a page.
#[derive(Clone, Debug)]
pub struct PlacedFrame {
    pub id: NodeId,
    pub section: usize,
    pub page: usize,
    /// Its box in document space.
    pub rect: Rect,
    /// Its text doesn't fit (and there's no next frame to continue in).
    pub overflow: bool,
    /// Text comes in from a previous frame.
    pub threaded_in: bool,
}

/// A page in document space.
#[derive(Clone, Debug)]
pub struct Page {
    /// The number shown on it (sections can restart numbering).
    pub number: u32,
    /// Index of the section whose page setup it uses.
    pub section: usize,
    /// The sheet.
    pub rect: Rect,
    /// The text area inside the margins.
    pub body: Rect,
    /// The first page of its section.
    pub first_of_section: bool,
}

/// A laid-out document.
#[derive(Clone, Default)]
pub struct DocLayout {
    pub fragments: Vec<Fragment>,
    /// Fragment indexes of each paragraph (in order).
    pub by_para: HashMap<NodeId, Vec<usize>>,
    /// Pages, top to bottom (one tall page in pageless view).
    pub pages: Vec<Page>,
    /// Headers and footers (drawn, not editable in place).
    pub decor: Vec<Fragment>,
    /// Cell shading and table lines.
    pub shapes: Vec<Shape>,
    pub images: Vec<PlacedImage>,
    pub frames: Vec<PlacedFrame>,
    /// The width of the text column.
    pub width: f32,
    pub height: f32,
    /// The pages' colour (Format › Page colour), when not white.
    pub page_color: Option<xenify_wp_doc::Rgba>,
    /// Table cells as placed (for selecting, handles and resizing).
    pub cells: Vec<PlacedCell>,
    /// Rich headers and footers being edited: (page, part, area).
    pub header_rects: Vec<(usize, xenify_wp_doc::NodeId, Rect)>,
    /// Header and footer areas of every page: (page, footer?, area) — where a double-click edits them.
    pub hf_areas: Vec<(usize, bool, Rect)>,
}

/// A table cell placed on a page.
#[derive(Clone, Debug)]
pub struct PlacedCell {
    pub table: NodeId,
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub rect: Rect,
    /// A copy of a header row repeated on a later page.
    pub repeat: bool,
}

impl DocLayout {
    fn push(&mut self, f: Fragment) {
        // Generated text (contents entries) has id 0 and isn't editable.
        if f.para.id != NodeId(0) {
            self.by_para.entry(f.para.id).or_default().push(self.fragments.len());
        }
        self.fragments.push(f);
    }

    /// The frame whose edge is near document point `p` (within `slop`).
    pub fn frame_edge_at(&self, p: Point, slop: f64) -> Option<&PlacedFrame> {
        self.frames.iter().rev().find(|f| {
            let outer = f.rect.inflate(slop, slop);
            let inner = f.rect.inflate(-slop, -slop);
            outer.contains(p) && !(inner.width() > 0.0 && inner.height() > 0.0 && inner.contains(p))
        })
    }

    pub fn frame_at(&self, p: Point) -> Option<&PlacedFrame> {
        self.frames.iter().rev().find(|f| f.rect.contains(p))
    }

    /// The picture at document point `p`.
    pub fn image_at(&self, p: Point) -> Option<&PlacedImage> {
        self.images.iter().find(|i| i.rect.contains(p))
    }

    /// The generated list (contents, bibliography, change log…) at
    /// document point `p`, or the table there (the change log is one).
    pub fn generated_at(&self, p: Point) -> Option<NodeId> {
        self.fragments
            .iter()
            .find(|f| f.para.generated.is_some() && f.rect().contains(p))
            .and_then(|f| f.para.generated)
            .or_else(|| self.cells.iter().find(|c| c.rect.contains(p)).map(|c| c.table))
    }

    /// Where a click at `p` on generated text (a contents entry) leads.
    pub fn link_at(&self, p: Point) -> Option<NodeId> {
        self.fragments.iter().find(|f| f.para.link.is_some() && f.rect().contains(p)).and_then(|f| f.para.link)
    }
}

/// Space between two consecutive paragraphs: the larger of the first's
/// space after and the second's space before (like CSS margins collapse),
/// and only a little between items of the same list.
pub(crate) fn gap_between(prev: Option<&ParaLayout>, next: &ParaLayout) -> f32 {
    let Some(prev) = prev else { return next.space_before };
    // (Numbered headings keep their spacing.)
    if prev.list.is_some() && prev.list == next.list && prev.list.as_deref() != Some(xenify_wp_doc::ListDef::HEADINGS) {
        return 3.0;
    }
    if prev.style == xenify_wp_doc::StyleSheet::CODE && next.style == prev.style {
        return 0.0;
    }
    prev.space_after.max(next.space_before)
}

/// Lays documents out, caching paragraph layouts between edits.
pub struct Engine {
    pub fonts: Fonts,
    cache: HashMap<NodeId, (u64, Arc<ParaLayout>)>,
    decor_cache: HashMap<u64, Arc<ParaLayout>>,
    /// Heading page numbers from the last layout (for contents tables).
    pub(crate) toc_pages: HashMap<NodeId, u32>,
    /// Pages in the last layout (for number-of-pages fields).
    pub(crate) page_count: u32,
    /// The page whose headers and footers are being edited (their text is live there).
    pub header_edit: Option<usize>,
}

impl Engine {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts,
            cache: HashMap::new(),
            decor_cache: HashMap::new(),
            toc_pages: HashMap::new(),
            page_count: 0,
            header_edit: None,
        }
    }

    /// Forgets every laid-out paragraph (fonts were added, so text may
    /// shape differently now).
    pub fn forget_layouts(&mut self) {
        self.cache.clear();
        self.decor_cache.clear();
    }

    /// The page (1-based) each heading was on in the last layout.
    pub fn heading_pages(&self) -> &HashMap<NodeId, u32> {
        &self.toc_pages
    }

    /// Paragraph `p` laid out `width` wide (cached).
    pub(crate) fn para(
        &mut self,
        doc: &Document,
        p: &xenify_wp_doc::Paragraph,
        width: f32,
        label: Option<&str>,
    ) -> Arc<ParaLayout> {
        let key = para::cache_key(doc, p, width, label);
        match self.cache.get(&p.id) {
            Some((k, l)) if *k == key => l.clone(),
            _ => {
                let l = Arc::new(para::layout_paragraph(&mut self.fonts, doc, p, width, label));
                self.cache.insert(p.id, (key, l.clone()));
                l
            }
        }
    }

    /// Header/footer text laid out (cached).
    pub(crate) fn decor(
        &mut self,
        doc: &Document,
        text: &str,
        style: &str,
        align: xenify_wp_doc::Align,
        width: f32,
    ) -> Arc<ParaLayout> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (text, style, align as u8, width.to_bits()).hash(&mut h);
        format!("{:?}", doc.styles.resolve(style)).hash(&mut h);
        let key = h.finish();
        if self.decor_cache.len() > 4096 {
            self.decor_cache.clear();
        }
        self.decor_cache
            .entry(key)
            .or_insert_with(|| Arc::new(paginate::layout_decor(&mut self.fonts, doc, text, style, align, width)))
            .clone()
    }

    /// Lays the document out on pages, or as one long page when it is set
    /// to pageless view.
    pub fn layout(&mut self, doc: &Document) -> DocLayout {
        let mut out = paginate::paginate(self, doc, doc.meta.pageless);
        let live: std::collections::HashSet<NodeId> = doc.paragraphs().map(|p| p.id).collect();
        self.cache.retain(|id, _| live.contains(id));
        out.width = out.pages.iter().map(|p| p.rect.width() as f32).fold(0.0, f32::max);
        out
    }

    /// One continuous column as wide as the first section's text area, on
    /// one tall page ("pageless" view).
    pub fn layout_pageless(&mut self, doc: &Document) -> DocLayout {
        paginate::paginate(self, doc, true)
    }
}
