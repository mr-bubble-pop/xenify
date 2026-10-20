use xenify_wp_doc::Rgba;
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{FontContext, LayoutContext};
use std::sync::Arc;

/// The bundled Inter font (SIL Open Font License; see assets/SOURCES.toml).
pub const INTER_FONT: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");

/// The bundled STIX Two Math font (SIL Open Font License): symbols Inter
/// lacks (arrows, maths, Greek), on every system and in the browser.
/// Empty with the `late-fonts` feature: see [`provide_late_fonts`].
#[cfg(not(feature = "late-fonts"))]
pub const SYMBOL_FONT: &[u8] = include_bytes!("../../../assets/fonts/STIXTwoMath-Regular.ttf");
#[cfg(feature = "late-fonts")]
pub const SYMBOL_FONT: &[u8] = &[];

/// The bundled CMU Serif fonts (Computer Modern Unicode, SIL Open Font
/// License): the classic LaTeX look, the same on every system. Empty with
/// the `late-fonts` feature.
#[cfg(not(feature = "late-fonts"))]
pub const SERIF_FONTS: [&[u8]; 4] = [
    include_bytes!("../../../assets/fonts/cmunrm.otf"),
    include_bytes!("../../../assets/fonts/cmunti.otf"),
    include_bytes!("../../../assets/fonts/cmunbx.otf"),
    include_bytes!("../../../assets/fonts/cmunbi.otf"),
];
#[cfg(feature = "late-fonts")]
pub const SERIF_FONTS: [&[u8]; 4] = [&[]; 4];

/// The symbol, serif and emoji files handed over at run time (the browser
/// downloads them after the first page is shown, see the `late-fonts` feature).
struct Late {
    symbol: &'static [u8],
    serif: Vec<&'static [u8]>,
    #[cfg_attr(not(feature = "late-fonts"), allow(dead_code))]
    emoji: &'static [u8],
}

static LATE: std::sync::OnceLock<Late> = std::sync::OnceLock::new();

/// Hands over the files left out by the `late-fonts` feature: STIX Two
/// Math, the CMU Serif files and the packed emoji set. Only the first call
/// counts. Fonts made earlier get them through [`Fonts::add_late_fonts`].
pub fn provide_late_fonts(symbol: Vec<u8>, serif: Vec<Vec<u8>>, emoji: Vec<u8>) {
    let leak = |v: Vec<u8>| -> &'static [u8] { Box::leak(v.into_boxed_slice()) };
    let _ = LATE.set(Late { symbol: leak(symbol), serif: serif.into_iter().map(leak).collect(), emoji: leak(emoji) });
}

/// STIX Two Math: built in, or handed over later (empty until then).
pub fn symbol_font() -> &'static [u8] {
    if SYMBOL_FONT.is_empty() { LATE.get().map_or(&[], |l| l.symbol) } else { SYMBOL_FONT }
}

/// The CMU Serif files: built in, or handed over later (none until then).
pub fn serif_fonts() -> Vec<&'static [u8]> {
    if SERIF_FONTS[0].is_empty() { LATE.get().map(|l| l.serif.clone()).unwrap_or_default() } else { SERIF_FONTS.to_vec() }
}

/// The packed emoji set: built in, or handed over later (empty until then).
pub(crate) fn emoji_packed() -> &'static [u8] {
    #[cfg(not(feature = "late-fonts"))]
    return include_bytes!("../../../assets/emoji/fluent-emoji.json.z");
    #[cfg(feature = "late-fonts")]
    return LATE.get().map_or(&[], |l| l.emoji);
}

/// The family name of [`SERIF_FONTS`].
pub const SERIF_FAMILY: &str = "CMU Serif";

/// Font collection plus parley's layout scratch space.
pub struct Fonts {
    pub(crate) cx: FontContext,
    pub(crate) lcx: LayoutContext<Rgba>,
    /// The symbol and serif fonts are in the collection.
    symbols: bool,
}

impl Fonts {
    /// Bundled fonts plus the fonts installed on the system.
    pub fn new() -> Self {
        Self::with_system_fonts(true)
    }

    /// Bundled fonts only: identical results on every machine (tests, CLI
    /// exports that must be reproducible).
    pub fn bundled_only() -> Self {
        Self::with_system_fonts(false)
    }

    fn with_system_fonts(system_fonts: bool) -> Self {
        // Shared: every clone (one per open document) sees fonts added later.
        let collection = Collection::new(CollectionOptions { system_fonts, shared: true });
        let mut cx = FontContext { collection, source_cache: SourceCache::default() };
        cx.collection.register_fonts(Blob::new(Arc::new(INTER_FONT)), None);
        let mut fonts = Self { cx, lcx: LayoutContext::new(), symbols: false };
        fonts.add_late_fonts();
        fonts
    }

    /// Adds the symbol and serif fonts to this collection (and every clone
    /// of it) once they're available; true when they were added.
    pub fn add_late_fonts(&mut self) -> bool {
        let symbol = symbol_font();
        if symbol.is_empty() || self.symbols {
            return false;
        }
        self.symbols = true;
        self.cx.collection.register_fonts(Blob::new(Arc::new(symbol)), None);
        for data in serif_fonts() {
            self.cx.collection.register_fonts(Blob::new(Arc::new(data)), None);
        }
        true
    }

    /// Adds a font file (TTF/OTF/collection); returns the family names in it.
    pub fn register(&mut self, data: Vec<u8>) -> Vec<String> {
        let families = self.cx.collection.register_fonts(Blob::new(Arc::new(data)), None);
        families.iter().filter_map(|(id, _)| self.cx.collection.family_name(*id).map(str::to_string)).collect()
    }

    /// Every family name available, sorted.
    pub fn families(&mut self) -> Vec<String> {
        let mut v: Vec<String> = self.cx.collection.family_names().map(str::to_string).collect();
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup();
        v
    }
}

/// Another handle on the same fonts (for another open document): cheap,
/// nothing is loaded again.
impl Clone for Fonts {
    fn clone(&self) -> Self {
        Self { cx: self.cx.clone(), lcx: LayoutContext::new(), symbols: self.symbols }
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}
