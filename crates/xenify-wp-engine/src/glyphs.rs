//! Fonts the page scenes use, and their glyph outlines as SVG path data.

use parley::FontData;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::collections::HashMap;
use std::fmt::Write;

/// Outlines are drawn at this many units per em: the screen scales a path
/// by `font size / EM`.
pub const EM: f32 = 1000.0;

/// Numbers the fonts (with their variation settings) that scenes refer to,
/// so a glyph is sent to the screen once, not with every page.
#[derive(Default)]
pub struct FontTable {
    ids: HashMap<(u64, u32, Vec<i16>), u32>,
    fonts: Vec<(FontData, Vec<i16>)>,
}

impl FontTable {
    /// The number for `font` at variation `coords`.
    pub fn id(&mut self, font: &FontData, coords: &[i16]) -> u32 {
        let key = (font.data.id(), font.index, coords.to_vec());
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let id = self.fonts.len() as u32;
        self.fonts.push((font.clone(), coords.to_vec()));
        self.ids.insert(key, id);
        id
    }

    /// SVG path data for each glyph of font `font`, y pointing down, at
    /// [`EM`] units per em. A glyph without an outline (a space) is "".
    pub fn outlines(&self, font: u32, glyphs: &[u32]) -> Vec<String> {
        let Some((data, coords)) = self.fonts.get(font as usize) else {
            return vec![String::new(); glyphs.len()];
        };
        let Ok(face) = FontRef::from_index(data.data.data(), data.index) else {
            return vec![String::new(); glyphs.len()];
        };
        let coords: Vec<NormalizedCoord> = coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
        let outlines = face.outline_glyphs();
        glyphs
            .iter()
            .map(|&g| {
                let mut pen = SvgPen(String::new());
                if let Some(glyph) = outlines.get(GlyphId::new(g)) {
                    let settings = DrawSettings::unhinted(Size::new(EM), LocationRef::new(&coords));
                    let _ = glyph.draw(settings, &mut pen);
                }
                pen.0
            })
            .collect()
    }
}

struct SvgPen(String);

fn n(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

impl OutlinePen for SvgPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{} {}", n(x), n(-y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{} {}", n(x), n(-y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let _ = write!(self.0, "Q{} {} {} {}", n(cx), n(-cy), n(x), n(-y));
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let _ = write!(self.0, "C{} {} {} {} {} {}", n(c0x), n(-c0y), n(c1x), n(-c1y), n(x), n(-y));
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}
