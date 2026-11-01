use crate::{CharFmt, Rgba};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// Where text after a tab character lines up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    /// Text starts at the stop.
    #[default]
    Left,
    /// Text is centred on the stop.
    Center,
    /// Text ends at the stop.
    Right,
    /// The decimal point sits on the stop (numbers in a column).
    Decimal,
}

/// What fills the gap before a tab stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabLeader {
    #[default]
    None,
    /// ……… (tables of contents, menus).
    Dots,
    /// - - - -
    Dashes,
    /// ____
    Line,
}

/// A tab stop, in points from the left margin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TabStop {
    pub pos: f32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: TabKind,
    #[serde(default, skip_serializing_if = "is_default")]
    pub leader: TabLeader,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Lines around a paragraph (each side optional).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParaBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<crate::Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<crate::Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<crate::Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<crate::Border>,
}

impl ParaBorders {
    /// The same line on all four sides.
    pub fn all(b: crate::Border) -> Self {
        Self { top: Some(b), bottom: Some(b), left: Some(b), right: Some(b) }
    }

    pub fn is_empty(&self) -> bool {
        [self.top, self.bottom, self.left, self.right].iter().all(|b| b.is_none_or(|b| b.width <= 0.0))
    }
}

/// Default tab stops after the last set one: every half inch.
pub const DEFAULT_TAB: f32 = 36.0;

/// Paragraph properties. `None` means "inherit" (from `based_on`, then the defaults).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParaProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// Points above / below the paragraph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after: Option<f32>,
    /// Line height as a multiple of the font size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f32>,
    /// Line height exactly this many points (instead of a multiple).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_exact: Option<f32>,
    /// Points from the left / right margin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_right: Option<f32>,
    /// Points the first line starts right of the others (negative: it hangs
    /// left of them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_line: Option<f32>,
    /// Tab stops (sorted by position).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<TabStop>>,
    /// Keep on the same page as the next paragraph (headings).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_with_next: Option<bool>,
    /// Keep all lines of the paragraph on one page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_together: Option<bool>,
    /// Start on a new page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<bool>,
    /// Start at the top of the next column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_break_before: Option<bool>,
    /// Background colour behind the paragraph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<Rgba>,
    /// Lines around the paragraph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<ParaBorders>,
    /// Widow and orphan control (don't leave one line alone at a page end or start).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widow_control: Option<bool>,
    /// Track changes: the break after this paragraph was inserted or
    /// deleted (and not yet accepted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_change: Option<crate::review::Change>,
}

impl ParaProps {
    pub fn is_empty(&self) -> bool {
        *self == ParaProps::default()
    }

    pub fn with(&self, p: &ParaProps) -> ParaProps {
        ParaProps {
            align: p.align.or(self.align),
            space_before: p.space_before.or(self.space_before),
            space_after: p.space_after.or(self.space_after),
            line_height: p.line_height.or(self.line_height),
            line_exact: if p.line_height.is_some() && p.line_exact.is_none() {
                None
            } else {
                p.line_exact.or(self.line_exact)
            },
            indent_left: p.indent_left.or(self.indent_left),
            indent_right: p.indent_right.or(self.indent_right),
            first_line: p.first_line.or(self.first_line),
            tabs: p.tabs.clone().or_else(|| self.tabs.clone()),
            keep_with_next: p.keep_with_next.or(self.keep_with_next),
            keep_together: p.keep_together.or(self.keep_together),
            page_break_before: p.page_break_before.or(self.page_break_before),
            column_break_before: p.column_break_before.or(self.column_break_before),
            shading: p.shading.or(self.shading),
            borders: p.borders.or(self.borders),
            widow_control: p.widow_control.or(self.widow_control),
            break_change: p.break_change.clone().or_else(|| self.break_change.clone()),
        }
    }
}

/// A named, editable paragraph style.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParaStyle {
    /// Stable id used by paragraphs ("normal", "heading1", or a user id).
    pub id: String,
    /// Name shown in the UI.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    /// Style for the paragraph Enter makes after this one (`None`: the same).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// Outline level (1 = Heading 1) for the table of contents and navigation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u8>,
    /// A list paragraphs of this style are numbered in ([`crate::ListDef`] id):
    /// every paragraph given the style joins it (headings numbered 1, 1.1…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    pub para: ParaProps,
    pub char: CharFmt,
}

/// Fully resolved paragraph properties.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPara {
    pub align: Align,
    pub space_before: f32,
    pub space_after: f32,
    pub line_height: f32,
    /// Exact line height in points (overrides `line_height`).
    pub line_exact: Option<f32>,
    pub indent_left: f32,
    pub indent_right: f32,
    /// First-line indent (negative: hanging).
    pub first_line: f32,
    pub tabs: Vec<TabStop>,
    pub keep_with_next: bool,
    pub keep_together: bool,
    pub page_break_before: bool,
    pub column_break_before: bool,
    pub shading: Option<Rgba>,
    pub borders: ParaBorders,
    pub widow_control: bool,
    pub outline_level: Option<u8>,
    /// How far a list label hangs left of the text (0 outside lists).
    pub hanging: f32,
    pub char: ResolvedChar,
}

impl ResolvedPara {
    /// Direct paragraph formatting on top.
    pub fn apply(&mut self, p: &ParaProps) {
        if let Some(v) = p.align {
            self.align = v;
        }
        if let Some(v) = p.space_before {
            self.space_before = v;
        }
        if let Some(v) = p.space_after {
            self.space_after = v;
        }
        if let Some(v) = p.line_height {
            self.line_height = v;
            self.line_exact = None;
        }
        if let Some(v) = p.line_exact {
            self.line_exact = Some(v);
        }
        if let Some(v) = p.indent_left {
            self.indent_left = v;
        }
        if let Some(v) = p.indent_right {
            self.indent_right = v;
        }
        if let Some(v) = p.first_line {
            self.first_line = v;
        }
        if let Some(v) = &p.tabs {
            self.tabs = v.clone();
        }
        if let Some(v) = p.keep_with_next {
            self.keep_with_next = v;
        }
        if let Some(v) = p.keep_together {
            self.keep_together = v;
        }
        if let Some(v) = p.page_break_before {
            self.page_break_before = v;
        }
        if let Some(v) = p.column_break_before {
            self.column_break_before = v;
        }
        if let Some(v) = p.shading {
            // A fully transparent colour clears inherited shading.
            self.shading = (v.0[3] > 0).then_some(v);
        }
        if let Some(v) = p.borders {
            self.borders = v;
        }
        if let Some(v) = p.widow_control {
            self.widow_control = v;
        }
    }
}

/// Fully resolved character formatting.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedChar {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Rgba,
    pub highlight: Option<Rgba>,
    pub link: Option<String>,
}

impl ResolvedChar {
    pub fn with(&self, f: &CharFmt) -> ResolvedChar {
        ResolvedChar {
            font: f.font.clone().unwrap_or_else(|| self.font.clone()),
            size: f.size.unwrap_or(self.size),
            bold: f.bold.unwrap_or(self.bold),
            italic: f.italic.unwrap_or(self.italic),
            underline: f.underline.unwrap_or(self.underline),
            strike: f.strike.unwrap_or(self.strike),
            color: f.color.unwrap_or(self.color),
            highlight: f.highlight.or(self.highlight).filter(|h| h.0[3] > 0),
            link: f.link.clone().or_else(|| self.link.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleSheet {
    /// Body font used when a style doesn't name one.
    pub default_font: String,
    pub default_size: f32,
    pub paragraph: Vec<ParaStyle>,
}

impl StyleSheet {
    pub const NORMAL: &'static str = "normal";
    pub const TITLE: &'static str = "title";
    pub const SUBTITLE: &'static str = "subtitle";
    pub const QUOTE: &'static str = "quote";
    pub const CODE: &'static str = "code";
    pub const CAPTION: &'static str = "caption";
    /// Footnote and endnote text.
    pub const NOTE: &'static str = "footnote-text";
    pub const HEADER: &'static str = "header";
    pub const FOOTER: &'static str = "footer";

    /// The id of the built-in heading style for `level` (1–6).
    pub fn heading(level: u8) -> String {
        format!("heading{}", level.clamp(1, 6))
    }

    pub fn get(&self, id: &str) -> Option<&ParaStyle> {
        self.paragraph.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ParaStyle> {
        self.paragraph.iter_mut().find(|s| s.id == id)
    }

    /// The style a new paragraph after one in `id` gets.
    pub fn next_style(&self, id: &str) -> String {
        self.get(id).and_then(|s| s.next.clone()).unwrap_or_else(|| id.to_string())
    }

    /// Resolves style `id` through its `based_on` chain (unknown ids resolve as Normal).
    pub fn resolve(&self, id: &str) -> ResolvedPara {
        // Collect the chain from the style up to the root (with a cycle guard).
        let mut chain = Vec::new();
        let mut cur = self.get(id).or_else(|| self.get(Self::NORMAL));
        while let Some(s) = cur {
            if chain.iter().any(|c: &&ParaStyle| c.id == s.id) || chain.len() > 16 {
                break;
            }
            chain.push(s);
            cur = s.based_on.as_deref().and_then(|b| self.get(b));
        }
        let mut para = ParaProps::default();
        let mut chr = CharFmt::default();
        for s in chain.iter().rev() {
            para = para.with(&s.para);
            chr = chr.with(&s.char);
        }
        let base = ResolvedChar {
            font: self.default_font.clone(),
            size: self.default_size,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: Rgba::BLACK,
            highlight: None,
            link: None,
        };
        ResolvedPara {
            align: para.align.unwrap_or_default(),
            space_before: para.space_before.unwrap_or(0.0),
            space_after: para.space_after.unwrap_or(0.0),
            line_height: para.line_height.unwrap_or(1.2),
            line_exact: para.line_exact,
            indent_left: para.indent_left.unwrap_or(0.0),
            indent_right: para.indent_right.unwrap_or(0.0),
            first_line: para.first_line.unwrap_or(0.0),
            tabs: para.tabs.clone().unwrap_or_default(),
            keep_with_next: para.keep_with_next.unwrap_or(false),
            keep_together: para.keep_together.unwrap_or(false),
            page_break_before: para.page_break_before.unwrap_or(false),
            column_break_before: para.column_break_before.unwrap_or(false),
            shading: para.shading.filter(|c| c.0[3] > 0),
            borders: para.borders.unwrap_or_default(),
            widow_control: para.widow_control.unwrap_or(true),
            outline_level: chain.first().and_then(|s| s.outline_level),
            hanging: 0.0,
            char: base.with(&chr),
        }
    }

    /// The built-in styles (all editable).
    pub fn builtin() -> Vec<ParaStyle> {
        let gray = Rgba::rgb(0x59, 0x59, 0x59);
        let style = |id: &str, name: &str, para: ParaProps, char: CharFmt| ParaStyle {
            id: id.into(),
            name: name.into(),
            based_on: Some(Self::NORMAL.into()),
            para,
            char,
            ..Default::default()
        };
        let mut v = vec![
            ParaStyle {
                based_on: None,
                ..style(
                    Self::NORMAL,
                    "Normal",
                    ParaProps { space_after: Some(8.0), line_height: Some(1.3), ..Default::default() },
                    CharFmt::default(),
                )
            },
            ParaStyle {
                next: Some(Self::NORMAL.into()),
                ..style(
                    Self::TITLE,
                    "Title",
                    ParaProps { space_after: Some(6.0), line_height: Some(1.1), ..Default::default() },
                    CharFmt { size: Some(26.0), bold: Some(true), ..Default::default() },
                )
            },
            ParaStyle {
                next: Some(Self::NORMAL.into()),
                ..style(
                    Self::SUBTITLE,
                    "Subtitle",
                    ParaProps { space_after: Some(14.0), ..Default::default() },
                    CharFmt { size: Some(15.0), color: Some(gray), ..Default::default() },
                )
            },
        ];
        let headings: [(f32, f32, bool, bool); 6] = [
            (20.0, 20.0, true, false),
            (16.0, 16.0, true, false),
            (13.5, 14.0, true, false),
            (12.0, 12.0, true, true),
            (11.0, 10.0, true, false),
            (11.0, 10.0, false, true),
        ];
        for (i, (size, before, bold, italic)) in headings.into_iter().enumerate() {
            let level = i as u8 + 1;
            v.push(ParaStyle {
                next: Some(Self::NORMAL.into()),
                outline_level: Some(level),
                ..style(
                    &Self::heading(level),
                    &format!("Heading {level}"),
                    ParaProps {
                        space_before: Some(before),
                        space_after: Some(6.0),
                        line_height: Some(1.15),
                        keep_with_next: Some(true),
                        ..Default::default()
                    },
                    CharFmt {
                        size: Some(size),
                        bold: Some(bold),
                        italic: italic.then_some(true),
                        color: (level >= 5).then_some(gray),
                        ..Default::default()
                    },
                )
            });
        }
        v.push(style(
            Self::QUOTE,
            "Quote",
            ParaProps { indent_left: Some(28.0), indent_right: Some(28.0), ..Default::default() },
            CharFmt { italic: Some(true), color: Some(gray), ..Default::default() },
        ));
        v.push(style(
            Self::CODE,
            "Code",
            ParaProps {
                space_before: Some(6.0),
                space_after: Some(6.0),
                line_height: Some(1.25),
                ..Default::default()
            },
            CharFmt { font: Some("monospace".into()), size: Some(10.0), ..Default::default() },
        ));
        for level in 1..=3u8 {
            v.push(ParaStyle {
                next: Some(Self::NORMAL.into()),
                ..style(
                    &format!("toc{level}"),
                    &format!("Contents {level}"),
                    ParaProps {
                        space_after: Some(if level == 1 { 4.0 } else { 2.0 }),
                        indent_left: Some(14.0 * (level as f32 - 1.0)),
                        ..Default::default()
                    },
                    CharFmt { bold: (level == 1).then_some(true), ..Default::default() },
                )
            });
        }
        for (id, name) in [(Self::HEADER, "Header"), (Self::FOOTER, "Footer")] {
            v.push(style(
                id,
                name,
                ParaProps { space_after: Some(0.0), line_height: Some(1.2), ..Default::default() },
                CharFmt { size: Some(9.0), color: Some(gray), ..Default::default() },
            ));
        }
        v.push(ParaStyle {
            next: Some(Self::NORMAL.into()),
            ..style(
                Self::CAPTION,
                "Caption",
                ParaProps { space_after: Some(10.0), ..Default::default() },
                CharFmt { size: Some(9.0), italic: Some(true), color: Some(gray), ..Default::default() },
            )
        });
        v.push(style(
            Self::NOTE,
            "Note",
            ParaProps { space_after: Some(2.0), line_height: Some(1.25), ..Default::default() },
            CharFmt { size: Some(9.0), ..Default::default() },
        ));
        v
    }
}

impl Default for StyleSheet {
    fn default() -> Self {
        Self { default_font: "Inter".into(), default_size: 11.0, paragraph: Self::builtin() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_inherit_from_normal() {
        let mut s = StyleSheet::default();
        let h1 = s.resolve(&StyleSheet::heading(1));
        assert_eq!(h1.char.size, 20.0);
        assert!(h1.char.bold);
        assert_eq!(h1.outline_level, Some(1));
        assert_eq!(h1.char.font, "Inter");
        // Editing Normal changes everything based on it.
        s.get_mut(StyleSheet::NORMAL).unwrap().char.font = Some("Serif".into());
        assert_eq!(s.resolve(&StyleSheet::heading(1)).char.font, "Serif");
        // Unknown styles resolve as Normal; cycles don't hang.
        assert_eq!(s.resolve("nope").char.size, 11.0);
        s.get_mut(StyleSheet::NORMAL).unwrap().based_on = Some(StyleSheet::QUOTE.into());
        let _ = s.resolve(StyleSheet::QUOTE);
    }
}
