//! One open document and what the keyboard and mouse do to it.

use crate::glyphs::FontTable;
use crate::scene::{self, PageInfo, PageScene};
use kurbo::{Point, Rect};
use serde::Serialize;
use xenify_wp_doc::autocorrect::Autocorrect;
use xenify_wp_doc::ops::{self, Autoformat};
use xenify_wp_doc::{Align, CharFmt, Document, Editor, ListDef, ParaProps, Pos, Selection, VertAlign};
use xenify_wp_layout::{DocLayout, Engine, Fonts};

/// What the toolbar and status bar show for the caret or selection.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ToolState {
    pub style: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    /// "left", "center", "right" or "justify".
    pub align: String,
    /// The list the caret's paragraph is in ("bullets", "numbers"…).
    pub list: Option<String>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub words: usize,
    /// Words in the selection (when there is one).
    pub selected_words: Option<usize>,
    pub dirty: bool,
    /// The caret is in a table cell.
    pub in_table: bool,
    /// How many carets there are (more than one after Alt+click, Ctrl+D…).
    pub carets: usize,
}

/// Where everything is: pages, the caret, the selection.
#[derive(Clone, Debug, Serialize)]
pub struct View {
    /// Changes whenever the layout does.
    pub rev: u64,
    pub width: f64,
    pub height: f64,
    pub pages: Vec<PageInfo>,
    /// The caret (x, y, w, h) in document space, when nothing is selected.
    pub caret: Option<[f64; 4]>,
    /// The page the caret is on.
    pub caret_page: usize,
    pub selection: Vec<[f64; 4]>,
    /// The other carets (Alt+click, Ctrl+Alt+Up/Down…); their selections are in `selection`.
    pub carets: Vec<[f64; 4]>,
    /// The selected text, so copying needs no round trip (None: nothing selected).
    pub selected_text: Option<String>,
    /// For screen readers: the caret's paragraph and where the caret and
    /// selection are in it (UTF-16 units, as the browser counts).
    pub para_text: String,
    pub para_sel: [usize; 2],
    /// The caret paragraph's indents (left, right, first line), in points, for the ruler.
    pub indents: [f32; 3],
    /// Text boxes (id, x, y, w, h in document space), outlined on screen.
    pub frames: Vec<(u64, [f64; 4])>,
    pub state: ToolState,
}

/// Bold, italic and the other on/off formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
}

impl Toggle {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bold" => Toggle::Bold,
            "italic" => Toggle::Italic,
            "underline" => Toggle::Underline,
            "strike" => Toggle::Strike,
            "superscript" => Toggle::Superscript,
            "subscript" => Toggle::Subscript,
            _ => return None,
        })
    }

    fn patch(self, on: bool) -> CharFmt {
        let v = Some(on);
        match self {
            Toggle::Bold => CharFmt { bold: v, ..Default::default() },
            Toggle::Italic => CharFmt { italic: v, ..Default::default() },
            Toggle::Underline => CharFmt { underline: v, ..Default::default() },
            Toggle::Strike => CharFmt { strike: v, ..Default::default() },
            Toggle::Superscript | Toggle::Subscript => {
                let raised = if self == Toggle::Superscript { VertAlign::Superscript } else { VertAlign::Subscript };
                CharFmt { vert: Some(if on { raised } else { VertAlign::Baseline }), ..Default::default() }
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Toggle::Bold => "Bold",
            Toggle::Italic => "Italic",
            Toggle::Underline => "Underline",
            Toggle::Strike => "Strikethrough",
            Toggle::Superscript => "Superscript",
            Toggle::Subscript => "Subscript",
        }
    }
}

/// Modifier keys held with a key or click.
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
pub struct Mods {
    #[serde(default)]
    pub shift: bool,
    /// Ctrl (Cmd on a Mac).
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub alt: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum DragBy {
    Char,
    Word,
    Paragraph,
    /// Shift+Alt+drag: a caret or selection on each line, between the
    /// press and the pointer (from the press point).
    Column(Point),
}

/// One open document.
pub struct Session {
    pub ed: Editor,
    /// Reading view: laid out as one long column (see [`Session::set_reading`]).
    reading: bool,
    engine: Engine,
    layout: DocLayout,
    laid_out: Option<u64>,
    rev: u64,
    /// The caret sits at the end of a wrapped line rather than the start of the next.
    upstream: bool,
    /// Where Up/Down aim for, across short lines.
    goal_x: Option<f64>,
    /// Bold etc. turned on with nothing selected: for the next text typed.
    pending_fmt: Option<CharFmt>,
    drag: Option<(DragBy, Selection)>,
    /// More carets or selections besides the editor's (Alt+click, Ctrl+D,
    /// Ctrl+Alt+Up/Down): typing, deleting and moving happen at each.
    pub extra: Vec<Selection>,
    pub autoformat: Autoformat,
    pub autocorrect: Autocorrect,
    /// A link Ctrl+clicked (for the screens to open), taken by whoever reads it.
    pub link_clicked: Option<String>,
    /// The last copy: its text and the copied part of the document (pasting
    /// that same text back keeps all its formatting).
    clip: Option<(String, Document)>,
}

impl Session {
    pub fn new(doc: Document, fonts: Fonts) -> Self {
        Self {
            ed: Editor::new(doc),
            engine: Engine::new(fonts),
            layout: DocLayout::default(),
            laid_out: None,
            rev: 0,
            upstream: false,
            goal_x: None,
            pending_fmt: None,
            drag: None,
            extra: Vec::new(),
            autoformat: Autoformat::default(),
            autocorrect: Autocorrect::default(),
            link_clicked: None,
            clip: None,
            reading: false,
        }
    }

    /// Reading view on or off: the text as one long column instead of pages.
    pub fn set_reading(&mut self, on: bool) {
        if self.reading != on {
            self.reading = on;
            self.laid_out = None;
        }
    }

    /// Fonts were added (the browser's late fonts): lay everything out
    /// again, as text may now use them.
    pub fn fonts_changed(&mut self) {
        self.engine.forget_layouts();
        self.laid_out = None;
    }

    /// Lays the document out again if it changed.
    fn relayout(&mut self) {
        if self.laid_out != Some(self.ed.doc.revision) {
            self.layout = if self.reading {
                // Reading view: one long column, like a web page (the document isn't changed).
                let mut l = self.engine.layout_pageless(&self.ed.doc);
                l.width = l.pages.iter().map(|p| p.rect.width() as f32).fold(0.0, f32::max);
                l
            } else {
                self.engine.layout(&self.ed.doc)
            };
            self.laid_out = Some(self.ed.doc.revision);
            self.rev += 1;
        }
    }

    pub fn layout(&mut self) -> &DocLayout {
        self.relayout();
        &self.layout
    }

    /// Where someone else's caret and selection are on the pages (people
    /// editing together): the caret when nothing is selected, and the
    /// selected stretches. Positions that no longer exist give nothing.
    pub fn rects_of(&mut self, sel: Selection) -> (Option<[f64; 4]>, Vec<[f64; 4]>) {
        self.relayout();
        let ok = |p: Pos| {
            self.ed.doc.para(p.para).is_some_and(|q| p.offset <= q.text.len() && q.text.is_char_boundary(p.offset))
        };
        if !ok(sel.anchor) || !ok(sel.focus) {
            return (None, Vec::new());
        }
        let r4 = |r: Rect| [r.x0, r.y0, r.width(), r.height()];
        let caret = if sel.is_collapsed() { self.layout.caret(sel.focus, false).map(r4) } else { None };
        (caret, self.layout.selection_rects(&self.ed.doc, sel).into_iter().map(r4).collect())
    }

    pub fn view(&mut self) -> View {
        self.relayout();
        self.prune_extra();
        let ed = &self.ed;
        let caret = if ed.sel.is_collapsed() { self.layout.caret(ed.sel.focus, self.upstream) } else { None };
        let r4 = |r: Rect| [r.x0, r.y0, r.width(), r.height()];
        let caret_y = caret.map_or(0.0, |c| c.y0);
        let caret_page = self.layout.pages.iter().position(|p| caret_y < p.rect.y1).unwrap_or(0);
        let width = self.layout.pages.iter().map(|p| p.rect.x1).fold(0.0, f64::max);
        View {
            rev: self.rev,
            width,
            height: self.layout.pages.last().map_or(0.0, |p| p.rect.y1),
            pages: scene::pages(&self.layout, &self.ed.doc),
            caret: caret.map(r4),
            caret_page,
            selection: std::iter::once(ed.sel)
                .chain(self.extra.iter().copied())
                .flat_map(|s| self.layout.selection_rects(&ed.doc, s))
                .map(r4)
                .collect(),
            carets: self
                .extra
                .iter()
                .filter(|s| s.is_collapsed())
                .filter_map(|s| self.layout.caret(s.focus, false))
                .map(r4)
                .collect(),
            selected_text: (!ed.sel.is_collapsed()).then(|| self.selected_text()),
            para_text: ed.doc.para(ed.sel.focus.para).map(|p| p.text.clone()).unwrap_or_default(),
            frames: self.layout.frames.iter().map(|f| (f.id.0, r4(f.rect))).collect(),
            indents: ed.doc.para(ed.sel.focus.para).map_or([0.0; 3], |p| {
                let r = ed.doc.resolve_para(p);
                [r.indent_left, r.indent_right, r.first_line]
            }),
            para_sel: {
                let text = ed.doc.para(ed.sel.focus.para).map(|p| p.text.as_str()).unwrap_or("");
                let u16 = |b: usize| text.get(..b.min(text.len())).map_or(0, |s| s.encode_utf16().count());
                let f = u16(ed.sel.focus.offset);
                let a = if ed.sel.anchor.para == ed.sel.focus.para {
                    u16(ed.sel.anchor.offset)
                } else if ed.doc.ordered(ed.sel).0 == ed.sel.focus {
                    text.encode_utf16().count()
                } else {
                    0
                };
                [a.min(f), a.max(f)]
            },
            state: self.state(),
        }
    }

    /// Display lists of the pages asked for.
    pub fn scenes(&mut self, pages: &[usize], fonts: &mut FontTable) -> Vec<PageScene> {
        self.relayout();
        let infos = scene::pages(&self.layout, &self.ed.doc);
        pages
            .iter()
            .filter_map(|&i| {
                let sig = infos.get(i)?.sig.clone();
                scene::page_scene(&self.layout, i, fonts, sig)
            })
            .collect()
    }

    pub fn state(&self) -> ToolState {
        let ed = &self.ed;
        let para = ed.doc.para(ed.sel.focus.para);
        let style_id = para.map(|p| p.style.clone()).unwrap_or_default();
        let resolved = ed.doc.styles.resolve(&style_id);
        let fmt = match (&self.pending_fmt, ed.sel.is_collapsed()) {
            (Some(f), true) => f.clone(),
            _ => ops::common_format(ed),
        };
        let mut props = resolved.clone();
        if let Some(p) = para {
            props.apply(&p.props);
        }
        let c = &resolved.char;
        ToolState {
            style: style_id,
            font: fmt.font.clone().unwrap_or_else(|| c.font.clone()),
            size: fmt.size.unwrap_or(c.size),
            bold: fmt.bold.unwrap_or(c.bold),
            italic: fmt.italic.unwrap_or(c.italic),
            underline: fmt.underline.unwrap_or(c.underline),
            strike: fmt.strike.unwrap_or(c.strike),
            superscript: fmt.vert == Some(VertAlign::Superscript),
            subscript: fmt.vert == Some(VertAlign::Subscript),
            align: match props.align {
                Align::Left => "left",
                Align::Center => "center",
                Align::Right => "right",
                Align::Justify => "justify",
            }
            .into(),
            list: para.and_then(|p| p.list.as_ref()).map(|l| l.list.clone()),
            can_undo: ed.can_undo(),
            can_redo: ed.can_redo(),
            undo_label: ed.undo_label().map(str::to_string),
            redo_label: ed.redo_label().map(str::to_string),
            words: ed.doc.word_count(),
            selected_words: ops::selection_counts(ed).map(|(w, _)| w),
            dirty: ed.is_dirty(),
            in_table: xenify_wp_doc::tables::cell_at(ed, ed.sel.focus.para).is_some(),
            carets: self.extra.len() + 1,
        }
    }

    // ----- Moving the caret -----

    fn go(&mut self, pos: Pos, extend: bool) {
        let ed = &mut self.ed;
        ed.sel = if extend { Selection { anchor: ed.sel.anchor, focus: pos } } else { Selection::caret(pos) };
        ed.seal();
        self.pending_fmt = None;
    }

    fn after_edit(&mut self) {
        self.goal_x = None;
        self.upstream = false;
    }

    /// A key press (names as in the browser's `KeyboardEvent.key`). Returns
    /// whether it did something (so the screen can stop the browser's own
    /// handling).
    pub fn key(&mut self, key: &str, m: Mods) -> bool {
        self.relayout();
        self.prune_extra();
        // More carets: Ctrl+D selects the next match too; Ctrl+Alt+Up/Down
        // (Shift+Alt+Up/Down, as some desktops keep Ctrl+Alt+arrows) add a
        // caret on the line above/below.
        if m.command && !m.alt && !m.shift && key.eq_ignore_ascii_case("d") {
            self.add_next_match();
            return true;
        }
        if matches!(key, "ArrowUp" | "ArrowDown") && m.alt && (m.command || m.shift) {
            self.add_caret_line(if key == "ArrowUp" { -1 } else { 1 });
            return true;
        }
        if !self.extra.is_empty()
            && let Some(done) = self.multi_key(key, m)
        {
            return done;
        }
        let word = m.command || m.alt;
        let shift = m.shift;
        let (start, end) = self.ed.doc.ordered(self.ed.sel);
        let vertical = matches!(key, "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown");
        if !vertical {
            self.goal_x = None;
        }
        let lower = key.to_lowercase();
        match key {
            "ArrowLeft" | "ArrowRight" => {
                let fwd = key == "ArrowRight";
                if !shift && !self.ed.sel.is_collapsed() && !word {
                    self.go(if fwd { end } else { start }, false);
                } else {
                    let pos = if word {
                        ops::word_move(&self.ed, self.ed.sel.focus, fwd)
                    } else {
                        ops::char_move(&self.ed, self.ed.sel.focus, fwd)
                    };
                    self.upstream = false;
                    self.go(pos, shift);
                }
            }
            "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown" => {
                let caret = self.layout.caret(self.ed.sel.focus, self.upstream);
                let goal = *self.goal_x.get_or_insert(caret.map_or(0.0, |c| c.x0));
                let lines: isize = match key {
                    "ArrowUp" => -1,
                    "ArrowDown" => 1,
                    "PageUp" => -30,
                    _ => 30,
                };
                let mut pos = None;
                // Page moves go as far as they can.
                let mut n = lines;
                while n != 0 && pos.is_none() {
                    pos = self.layout.line_move(self.ed.sel.focus, self.upstream, n, goal);
                    if lines.abs() == 1 {
                        break;
                    }
                    n -= n.signum();
                }
                // Lines with no paragraph of the document's (a contents' entries): on past them.
                let mut more = lines;
                while pos.is_some_and(|p| self.ed.doc.para(p.para).is_none()) && more.abs() < 2000 {
                    more += lines.signum();
                    pos = self.layout.line_move(self.ed.sel.focus, self.upstream, more, goal);
                }
                let pos = pos.filter(|p| self.ed.doc.para(p.para).is_some());
                // Up from under a contents (or picture, or table) at the very top: room to type above it.
                if pos.is_none()
                    && key == "ArrowUp"
                    && !shift
                    && xenify_wp_doc::tables::open_above_top(&mut self.ed).unwrap_or(false)
                {
                    self.upstream = false;
                    self.pending_fmt = None;
                    self.after_edit();
                    return true;
                }
                let pos = pos.unwrap_or(if lines < 0 { self.ed.doc.start() } else { self.ed.doc.end() });
                self.upstream = false;
                self.go(pos, shift);
                self.goal_x = Some(goal);
            }
            "Home" | "End" => {
                let to_end = key == "End";
                let pos = if m.command {
                    if to_end { self.ed.doc.end() } else { self.ed.doc.start() }
                } else {
                    self.layout.line_edge(self.ed.sel.focus, self.upstream, to_end)
                };
                self.go(pos, shift);
                // At the end of a wrapped line the caret stays on that line.
                self.upstream = to_end && !m.command;
            }
            "Backspace" => {
                let _ = ops::backspace(&mut self.ed, word);
                self.after_edit();
            }
            "Delete" => {
                let _ = ops::delete_forward(&mut self.ed, word);
                self.after_edit();
            }
            "Enter" if m.command => {
                let _ = ops::insert_page_break(&mut self.ed);
                self.after_edit();
            }
            "Enter" if shift => {
                // In a list: more of the same item; elsewhere a new paragraph.
                if !matches!(ops::continue_item(&mut self.ed), Ok(true)) {
                    let _ = ops::split_paragraph(&mut self.ed);
                }
                self.after_edit();
            }
            "Enter" => {
                let _ = ops::split_paragraph(&mut self.ed);
                self.after_edit();
            }
            "Tab" if !m.command => {
                // Tab goes to the next table cell, moves list items in and
                // out, and elsewhere types a tab (to the next tab stop).
                if !matches!(xenify_wp_doc::tables::move_cell(&mut self.ed, !shift), Ok(true))
                    && !matches!(ops::change_list_level(&mut self.ed, if shift { -1 } else { 1 }), Ok(true))
                    && !shift
                {
                    self.type_text("\t");
                }
            }
            _ if m.command && lower == "a" => {
                // The body text (text boxes and notes have their own Select all, inside them).
                self.ed.sel = Selection { anchor: self.ed.doc.start(), focus: self.ed.doc.body_end() };
            }
            _ if m.command && lower == "z" && !shift => self.undo(),
            _ if m.command && (lower == "z" && shift || lower == "y") => self.redo(),
            _ if m.command && lower == "b" => self.toggle(Toggle::Bold),
            _ if m.command && lower == "i" => self.toggle(Toggle::Italic),
            _ if m.command && lower == "u" => self.toggle(Toggle::Underline),
            _ => return false,
        }
        true
    }

    /// Text typed (or committed by an input method).
    pub fn type_text(&mut self, text: &str) {
        if !self.extra.is_empty() {
            let fmt = self.pending_fmt.clone();
            let typing = Some(xenify_wp_doc::MergeKey::Control("multi-typing".into()));
            self.each("Typing", typing, &mut |ed| drop(ops::insert_text(ed, text, fmt.as_ref())));
            return;
        }
        let ed = &mut self.ed;
        // Typing into a form's prompt ("Click to enter text") replaces it.
        if ed.sel.is_collapsed()
            && let Some((c, _)) = xenify_wp_doc::controls::control_at(ed, ed.sel.focus)
            && c.placeholder
        {
            let _ = xenify_wp_doc::controls::set_value(ed, ed.sel.focus, text);
            self.after_edit();
            return;
        }
        let fmt = self.pending_fmt.clone();
        let _ = ops::insert_text(ed, text, fmt.as_ref());
        // "* ", "1. ", "# " at the start of a paragraph: a list or heading.
        let formatted = text == " " && ops::autoformat(ed, &self.autoformat).unwrap_or(false);
        // "**bold**" and the like: formatting, and what's typed next isn't.
        if matches!(text, "*" | "_" | "~")
            && let Ok(Some(outer)) = ops::inline_markdown(ed, &self.autoformat)
        {
            self.pending_fmt = Some(outer);
            self.after_edit();
            return;
        }
        if !formatted {
            let _ = xenify_wp_doc::autocorrect::autocorrect(ed, text, &self.autocorrect);
        }
        self.after_edit();
    }

    /// Puts `text` in at the caret as a link to `url` (a page picked to link
    /// to with nothing selected): what's typed after it isn't linked.
    pub fn insert_link(&mut self, text: &str, url: &str) {
        let ed = &mut self.ed;
        let base = self.pending_fmt.clone().unwrap_or_else(|| {
            ed.doc.para(ed.sel.focus.para).map(|p| p.fmt_at(ed.sel.focus.offset)).unwrap_or_default()
        });
        let fmt = ops::normalize_fmt(base.with(&CharFmt { link: Some(url.to_string()), ..Default::default() }));
        ed.seal();
        let _ = ops::insert_text(ed, text, Some(&fmt));
        ed.seal();
        self.pending_fmt = Some(base);
        self.after_edit();
    }

    /// Pastes plain text (line breaks become paragraphs).
    pub fn paste_text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n");
        if !self.extra.is_empty() {
            // As many lines as carets: one each; otherwise all of it at each.
            let lines: Vec<&str> = text.lines().collect();
            let n = self.extra.len() + 1;
            let mut k = n;
            self.each("Paste", None, &mut |ed| {
                k -= 1;
                let piece = if lines.len() == n { lines[k] } else { text.as_str() };
                let _ = ops::insert_text(ed, piece, None);
            });
            return;
        }
        self.ed.group("Paste", None, |ed| {
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    let _ = ops::split_paragraph(ed);
                }
                if !line.is_empty() {
                    let _ = ops::insert_text(ed, line, None);
                }
            }
        });
        self.after_edit();
    }

    /// Remembers the selection with its formatting, for pasting in Write.
    pub fn copy(&mut self) {
        if !self.ed.sel.is_collapsed() && self.extra.is_empty() {
            let frag = xenify_wp_doc::clip::copy_fragment(&self.ed);
            self.clip = Some((self.selected_text(), frag));
        }
    }

    /// A paste: Write's own copy with its formatting, else `html` from the
    /// clipboard, else spreadsheet cells or Markdown structure, else the
    /// text. `plain`: just the text.
    pub fn paste(&mut self, text: &str, html: &str, plain: bool) {
        let text = text.replace("\r\n", "\n");
        let frag = if plain || !self.extra.is_empty() {
            None
        } else {
            match &self.clip {
                Some((t, frag)) if *t == text && !text.is_empty() => Some(frag.clone()),
                _ if !html.trim().is_empty() => Some(xenify_wp_io::html_read::read(html)),
                _ => xenify_wp_doc::clip::tsv_fragment(&text).or_else(|| {
                    xenify_wp_io::looks_like_markdown(&text)
                        .then(|| xenify_wp_io::load(text.as_bytes(), xenify_wp_io::Format::Markdown).ok())
                        .flatten()
                        .map(|l| l.doc)
                }),
            }
        };
        match frag {
            Some(f) => {
                self.ed.seal();
                let _ = xenify_wp_doc::clip::paste_fragment(&mut self.ed, &f, "Paste");
                self.ed.seal();
                self.after_edit();
            }
            None => self.paste_text(&text),
        }
    }

    /// The selected text, for copying.
    pub fn selected_text(&self) -> String {
        if !self.extra.is_empty() {
            // Each selection on a line of its own (a paste back gives one each).
            let sels = xenify_wp_doc::multi::normalize(&self.ed.doc, &self.all_sels());
            return sels.iter().map(|s| self.ed.doc.text_in(*s)).collect::<Vec<_>>().join("\n");
        }
        let spans = ops::copy_spans(&self.ed);
        spans.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    pub fn cut(&mut self) -> String {
        if !self.extra.is_empty() {
            let text = self.selected_text();
            self.each("Cut", None, &mut |ed| drop(ops::delete_selection(ed)));
            return text;
        }
        self.copy();
        let text = self.selected_text();
        let _ = ops::delete_selection(&mut self.ed);
        self.after_edit();
        text
    }

    // ----- Several carets -----

    /// The editor's selection and the extra ones.
    fn all_sels(&self) -> Vec<Selection> {
        let mut v = self.extra.clone();
        v.push(self.ed.sel);
        v
    }

    /// Drops extra carets the document no longer has room for (after an
    /// undo, someone else's edit…) and joins ones that now overlap.
    fn prune_extra(&mut self) {
        if self.extra.is_empty() {
            return;
        }
        let doc = &self.ed.doc;
        self.extra.retain(|s| doc.para(s.anchor.para).is_some() && doc.para(s.focus.para).is_some());
        for s in &mut self.extra {
            *s = Selection { anchor: doc.clamp(s.anchor), focus: doc.clamp(s.focus) };
        }
        let main = self.ed.sel;
        let all = xenify_wp_doc::multi::normalize(doc, &self.all_sels());
        // The editor's selection stays the main one (or the one it joined).
        let (ma, mb) = doc.ordered(main);
        let keep = all.iter().position(|s| {
            let (a, b) = doc.ordered(*s);
            doc.cmp_pos(a, ma).is_le() && doc.cmp_pos(mb, b).is_le()
        });
        let keep = keep.unwrap_or(all.len().saturating_sub(1));
        self.ed.sel = all.get(keep).copied().unwrap_or(main);
        self.extra = all.into_iter().enumerate().filter(|(i, _)| *i != keep).map(|(_, s)| s).collect();
    }

    /// Runs `edit` at every caret, as one undo step.
    fn each(&mut self, label: &str, merge: Option<xenify_wp_doc::MergeKey>, edit: &mut dyn FnMut(&mut Editor)) {
        let sels = self.all_sels();
        let done = xenify_wp_doc::multi::edit_each(&mut self.ed, &sels, label, merge, edit);
        self.extra = done[..done.len().saturating_sub(1)].to_vec();
        self.pending_fmt = None;
        self.after_edit();
    }

    /// A key while there are several carets: edits happen at each, moves
    /// move each. None when it's for the usual handling (undo, shortcuts…).
    fn multi_key(&mut self, key: &str, m: Mods) -> Option<bool> {
        let word = m.command || m.alt;
        let typing = Some(xenify_wp_doc::MergeKey::Control("multi-typing".into()));
        match key {
            "Escape" => self.extra.clear(),
            "Backspace" => self.each("Delete", None, &mut |ed| drop(ops::backspace(ed, word))),
            "Delete" => self.each("Delete", None, &mut |ed| drop(ops::delete_forward(ed, word))),
            "Enter" if !m.command => self.each("New paragraph", None, &mut |ed| drop(ops::split_paragraph(ed))),
            "Tab" if !m.command && !m.shift => {
                self.each("Typing", typing, &mut |ed| drop(ops::insert_text(ed, "\t", None)));
            }
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End" if !m.command || !m.alt => {
                // Ctrl+Home/End go to one place: back to one caret there.
                if m.command && matches!(key, "Home" | "End") {
                    self.extra.clear();
                    return None;
                }
                // Move each (the usual way, one at a time).
                let mut moved = Vec::new();
                for s in self.all_sels() {
                    self.ed.sel = s;
                    self.goal_x = None;
                    self.extra_off(|me| me.key(key, m));
                    moved.push(self.ed.sel);
                }
                let main = self.ed.sel;
                self.extra = moved[..moved.len() - 1].to_vec();
                self.ed.sel = main;
                self.prune_extra();
            }
            // Undo, select all and the rest: back to one caret.
            _ => {
                if m.command
                    && !key.eq_ignore_ascii_case("b")
                    && !key.eq_ignore_ascii_case("i")
                    && !key.eq_ignore_ascii_case("u")
                {
                    self.extra.clear();
                }
                return None;
            }
        }
        Some(true)
    }

    /// Runs `f` as if there were one caret (the extra ones put back after).
    fn extra_off(&mut self, f: impl FnOnce(&mut Self) -> bool) -> bool {
        let extra = std::mem::take(&mut self.extra);
        let out = f(self);
        self.extra = extra;
        out
    }

    /// Ctrl+D: the word at the caret, then each time the next place the
    /// selected text occurs, selected as well.
    fn add_next_match(&mut self) {
        if self.ed.sel.is_collapsed() {
            self.ed.sel = ops::word_at(&self.ed, self.ed.sel.focus);
            return;
        }
        let text = self.ed.doc.text_in(self.ed.sel);
        let after = self.ed.doc.ordered(self.ed.sel).1;
        if let Some(next) = xenify_wp_doc::multi::next_match(&self.ed.doc, &text, after, &self.all_sels()) {
            self.extra.push(self.ed.sel);
            self.ed.sel = next;
            self.ed.seal();
        }
    }

    /// One more caret, on the line above (-1) or below (1) the caret.
    fn add_caret_line(&mut self, dir: isize) {
        let goal = *self
            .goal_x
            .get_or_insert_with(|| self.layout.caret(self.ed.sel.focus, self.upstream).map_or(0.0, |c| c.x0));
        if let Some(pos) = self.layout.line_move(self.ed.sel.focus, self.upstream, dir, goal) {
            self.extra.push(self.ed.sel);
            self.ed.sel = Selection::caret(pos);
            self.upstream = false;
            self.ed.seal();
            self.prune_extra();
        }
    }

    /// Shift+Alt+drag from `from` to `to`: on each line between them, the
    /// text between the two across (a caret where they are the same).
    fn column(&mut self, from: Point, to: Point) {
        let Some(p0) = self.layout.hit(from) else { return };
        let dir: isize = if to.y < from.y { -1 } else { 1 };
        let mut sels = Vec::new();
        let mut n: isize = 0;
        while n.unsigned_abs() < 10_000 {
            let a = if n == 0 { Some(p0) } else { self.layout.line_move(p0, false, n, from.x) };
            let Some(a) = a else { break };
            let Some(c) = self.layout.caret(a, false) else { break };
            if n != 0 && (if dir > 0 { c.y0 > to.y } else { c.y1 < to.y }) {
                break;
            }
            let b = self.layout.hit(Point::new(to.x, c.center().y)).filter(|b| b.para == a.para).unwrap_or(a);
            sels.push(Selection { anchor: a, focus: b });
            n += dir;
        }
        if dir < 0 {
            sels.reverse();
        }
        let Some(last) = sels.pop() else { return };
        self.ed.sel = last;
        self.extra = sels;
    }

    // ----- The mouse -----

    /// A press at document point (x, y); `clicks` is 2 for a double click
    /// (selects a word), 3 for a triple (the paragraph).
    pub fn pointer_down(&mut self, x: f64, y: f64, clicks: u32, m: Mods) {
        self.relayout();
        let Some(pos) = self.layout.hit(Point::new(x, y)) else { return };
        let dp = Point::new(x, y);
        // Ctrl+click on a link follows it (a plain click edits the text).
        if m.command && clicks == 1 {
            let link = self.ed.doc.para(pos.para).and_then(|p| {
                let at = pos.offset.min(p.text.len().saturating_sub(1));
                p.fmt_at(at + 1).link.or_else(|| p.fmt_at(pos.offset).link)
            });
            if let Some(url) = link {
                self.link_clicked = Some(url);
                return;
            }
        }
        // A checklist's box, or a form's checkbox: the click ticks it.
        if !m.shift && clicks == 1 {
            if self.on_checklist_box(pos, dp) {
                let _ = ops::toggle_checked(&mut self.ed, pos.para);
                return;
            }
            if let Some(at) = self.form_check_at(pos, dp) {
                let _ = xenify_wp_doc::controls::toggle_check(&mut self.ed, at);
                return;
            }
        }
        self.goal_x = None;
        self.upstream = false;
        // Shift+Alt+drag: a column of carets. Alt+click: one more caret
        // (or one fewer, on a caret already there).
        if m.alt && m.shift && clicks == 1 {
            self.extra.clear();
            self.ed.sel = Selection::caret(pos);
            self.ed.seal();
            self.drag = Some((DragBy::Column(dp), self.ed.sel));
            return;
        }
        if m.alt && !m.shift && !m.command && clicks == 1 {
            let here = Selection::caret(pos);
            if let Some(i) = self.extra.iter().position(|s| *s == here) {
                self.extra.remove(i);
                self.drag = None;
                return;
            }
            if self.ed.sel == here && !self.extra.is_empty() {
                self.ed.sel = self.extra.pop().unwrap_or(here);
                self.drag = None;
                return;
            }
            self.extra.push(self.ed.sel);
        } else {
            self.extra.clear();
        }
        let by = match clicks {
            2 => DragBy::Word,
            n if n >= 3 => DragBy::Paragraph,
            _ => DragBy::Char,
        };
        let unit = self.unit_at(pos, by);
        if m.shift && by == DragBy::Char {
            self.go(pos, true);
            self.drag = Some((by, Selection::caret(self.ed.sel.anchor)));
            return;
        }
        self.ed.sel = unit;
        self.ed.seal();
        self.pending_fmt = None;
        self.drag = Some((by, unit));
    }

    pub fn pointer_move(&mut self, x: f64, y: f64) {
        let Some((by, first)) = self.drag else { return };
        self.relayout();
        if let DragBy::Column(from) = by {
            self.column(from, Point::new(x, y));
            return;
        }
        let Some(pos) = self.layout.hit(Point::new(x, y)) else { return };
        let unit = self.unit_at(pos, by);
        let (a0, a1) = self.ed.doc.ordered(first);
        let (b0, b1) = self.ed.doc.ordered(unit);
        // Dragging back before where it started selects backwards.
        self.ed.sel = if self.ed.doc.ordered(Selection { anchor: a0, focus: b0 }) == (b0, a0) && b0 != a0 {
            Selection { anchor: a1, focus: b0 }
        } else {
            Selection { anchor: a0, focus: b1 }
        };
    }

    pub fn pointer_up(&mut self) {
        self.drag = None;
    }

    /// Whether `dp` is on the box of a checklist item (left of its first line).
    fn on_checklist_box(&self, pos: Pos, dp: Point) -> bool {
        let ed = &self.ed;
        let Some(p) = ed.doc.para(pos.para) else { return false };
        let Some(item) = &p.list else { return false };
        let is_check =
            ed.doc.list_def(&item.list).is_some_and(|d| d.level(item.level).kind == xenify_wp_doc::NumberKind::Check);
        if !is_check {
            return false;
        }
        let Some(first) = self.layout.caret(Pos::new(pos.para, 0), false) else { return false };
        let hanging = ed.doc.resolve_para(p).hanging as f64;
        dp.y >= first.y0 && dp.y <= first.y1 && dp.x < first.x0 - 1.0 && dp.x > first.x0 - hanging - 4.0
    }

    /// The form checkbox (as the position before it) that `dp` is on.
    fn form_check_at(&self, pos: Pos, dp: Point) -> Option<Pos> {
        use xenify_wp_doc::Field;
        let p = self.ed.doc.para(pos.para)?;
        let w = Field::CHAR.len_utf8();
        [Some(pos.offset), pos.offset.checked_sub(w)].into_iter().flatten().find_map(|at| {
            let is_box = p.text.get(at..at + w) == Some(Field::STR)
                && p.fmt_at(at + w).field.as_ref().is_some_and(|f| matches!(f, Field::Check { .. }));
            let a = self.layout.caret(Pos::new(pos.para, at), false)?;
            let b = self.layout.caret(Pos::new(pos.para, at + w), true)?;
            let on = dp.x >= a.x0.min(b.x0) - 1.0 && dp.x <= a.x0.max(b.x0) + 1.0 && dp.y >= a.y0 && dp.y <= a.y1;
            (is_box && on).then_some(Pos::new(pos.para, at))
        })
    }

    fn unit_at(&self, pos: Pos, by: DragBy) -> Selection {
        match by {
            DragBy::Char | DragBy::Column(_) => Selection::caret(pos),
            DragBy::Word => ops::word_at(&self.ed, pos),
            DragBy::Paragraph => {
                let len = self.ed.doc.para(pos.para).map_or(0, |p| p.text.len());
                Selection { anchor: Pos::new(pos.para, 0), focus: Pos::new(pos.para, len) }
            }
        }
    }

    // ----- Formatting -----

    pub fn undo(&mut self) {
        self.extra.clear();
        self.pending_fmt = None;
        self.ed.undo();
        self.after_edit();
    }

    pub fn redo(&mut self) {
        self.extra.clear();
        self.ed.redo();
        self.after_edit();
    }

    /// Bold / italic / underline… on the selection, or for the next typed
    /// text when nothing is selected.
    pub fn toggle(&mut self, what: Toggle) {
        let st = self.state();
        let now = match what {
            Toggle::Bold => st.bold,
            Toggle::Italic => st.italic,
            Toggle::Underline => st.underline,
            Toggle::Strike => st.strike,
            Toggle::Superscript => st.superscript,
            Toggle::Subscript => st.subscript,
        };
        let patch = what.patch(!now);
        self.format(&patch, what.label());
    }

    /// Applies `patch` to the selection, or to what is typed next.
    pub fn format(&mut self, patch: &CharFmt, label: &str) {
        if !self.extra.is_empty() && self.all_sels().iter().any(|s| !s.is_collapsed()) {
            self.each(label, None, &mut |ed| drop(ops::apply_format(ed, patch, label)));
            return;
        }
        let ed = &mut self.ed;
        if ed.sel.is_collapsed() {
            let base = self.pending_fmt.clone().unwrap_or_else(|| {
                ed.doc.para(ed.sel.focus.para).map(|p| p.fmt_at(ed.sel.focus.offset)).unwrap_or_default()
            });
            self.pending_fmt = Some(ops::normalize_fmt(base.with(patch)));
        } else {
            ed.seal();
            let _ = ops::apply_format(ed, patch, label);
            ed.seal();
        }
    }

    pub fn set_style(&mut self, id: &str) {
        let _ = ops::set_style(&mut self.ed, id);
        self.after_edit();
    }

    pub fn set_align(&mut self, align: &str) {
        let align = match align {
            "center" => Align::Center,
            "right" => Align::Right,
            "justify" => Align::Justify,
            _ => Align::Left,
        };
        let _ = ops::apply_para_props(&mut self.ed, &ParaProps { align: Some(align), ..Default::default() }, "Align");
    }

    /// Turns the selected paragraphs into a list ("bullets", "numbers",
    /// "checklist"), or takes them out of it.
    pub fn toggle_list(&mut self, list: &str) {
        let id = match list {
            "numbers" => ListDef::NUMBERS,
            "checklist" => ListDef::CHECKLIST,
            _ => ListDef::BULLETS,
        };
        let _ = ops::toggle_list(&mut self.ed, id);
    }

    pub fn clear_format(&mut self) {
        let _ = ops::clear_format(&mut self.ed);
        self.pending_fmt = None;
    }

    /// The paragraph styles: (id, name, outline level).
    pub fn styles(&self) -> Vec<(String, String, Option<u8>)> {
        self.ed.doc.styles.paragraph.iter().map(|s| (s.id.clone(), s.name.clone(), s.outline_level)).collect()
    }

    pub fn font_families(&mut self) -> Vec<String> {
        self.engine.fonts.families()
    }
}
