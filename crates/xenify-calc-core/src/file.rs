//! `.xcalc` files: a zip holding `calc.json` (Calc's own description of the
//! book, versioned and independent of IronCalc's internal formats) and,
//! later, pictures under `media/`.
//!
//! Cells are kept exactly (numbers as numbers, formulas in English), each
//! with an index into one list of looks.

use crate::book::{Book, CellData, ColumnNote, Definition, SheetInfo};
use crate::ops::LineSize;
use crate::tables::TableDef;
use base64::Engine as _;
use ironcalc_base::types::{DefinedName, Style};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

pub const FORMAT: &str = "xenify-calc";
/// The format level this Calc writes and understands. A file's `version` is
/// the lowest level a reader needs to open it correctly (worked out from what
/// the book uses, so plain books stay readable by older Calcs); `saved_with`
/// is the level of the Calc that wrote it. Raise this when older Calcs would
/// misread something new (a sheet kind, a cell kind, a changed meaning), and
/// raise `version` in `needs` for books that use it. See docs/formats.md.
///
/// 1: the first. 2: linked CSV sheets.
pub const VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
pub struct FileBook {
    pub format: String,
    pub version: u32,
    /// The level of the Calc that wrote it (0: from before this was kept).
    #[serde(default)]
    pub saved_with: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub timezone: String,
    pub sheets: Vec<FileSheet>,
    /// Every look a cell has; cells point into it.
    #[serde(default)]
    pub styles: Vec<Style>,
    #[serde(default)]
    pub tables: Vec<TableDef>,
    #[serde(default)]
    pub names: Vec<FileName>,
    #[serde(default)]
    pub notes: Vec<ColumnNote>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub filters: BTreeMap<String, Vec<crate::edit::ColumnFilter>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charts: Vec<crate::charts::Chart>,
    /// Pictures (their bytes are files of their own: `part`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<crate::pictures::Picture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sparklines: Vec<crate::sparklines::Sparkline>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub summaries: BTreeMap<u32, crate::summary::Def>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reports: BTreeMap<u32, crate::report::Def>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub queries: BTreeMap<u32, crate::query::Def>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub editors: BTreeMap<u32, crate::editor::Def>,
    /// Linked CSV sheets' files.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub csv_links: BTreeMap<u32, crate::csvlink::Def>,
    /// When the book came from an Excel file: each sheet's part in it (that
    /// file is kept as kept.xlsx beside this, for the parts Calc keeps).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kept_sheets: BTreeMap<u32, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept_imported: Vec<String>,
    #[serde(default)]
    pub definitions: Vec<Definition>,
}

const NEWER: &str = "This file was made by a newer Calc: update Calc to open it";

/// The lowest format level that reads `book` correctly.
fn needs(book: &Book) -> u32 {
    let linked = !book.csv_links.is_empty() || book.info.values().any(|i| i.kind == crate::book::SheetKind::LinkedCsv);
    if linked { 2 } else { 1 }
}

#[derive(Serialize, Deserialize)]
pub struct FileSheet {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub info: SheetInfo,
    #[serde(default)]
    pub frozen: (i32, i32),
    #[serde(default = "yes")]
    pub grid: bool,
    #[serde(default)]
    pub rows: Vec<LineSize>,
    #[serde(default)]
    pub columns: Vec<LineSize>,
    pub cells: Vec<FileCell>,
    #[serde(default)]
    pub merged: Vec<ironcalc_base::types::MergedCell>,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
pub struct FileCell {
    pub r: i32,
    pub c: i32,
    #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
    pub data: Option<CellData>,
    /// Index into `styles`; 0 is the plain look.
    #[serde(default, rename = "st", skip_serializing_if = "is_zero")]
    pub s: usize,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

#[derive(Serialize, Deserialize)]
pub struct FileName {
    pub name: String,
    pub formula: String,
    /// The sheet id it belongs to, when not the whole book.
    #[serde(default)]
    pub sheet: Option<u32>,
}

impl FileBook {
    pub fn from_book(book: &Book) -> FileBook {
        let mut styles: Vec<Style> = vec![Style::default()];
        let mut style_index = |s: Style| -> usize {
            match styles.iter().position(|x| *x == s) {
                Some(k) => k,
                None => {
                    styles.push(s);
                    styles.len() - 1
                }
            }
        };
        let mut sheets = Vec::new();
        for w in &book.model.workbook.worksheets {
            let id = w.sheet_id;
            let cells = book
                .cells_in(id, None, None)
                .into_iter()
                .map(|(r, c, data, style)| FileCell { r, c, data, s: style_index(style) })
                .collect();
            let rows = w.rows.iter().filter(|r| r.custom_height || r.hidden).map(crate::ops::line_of).collect();
            let mut columns = Vec::new();
            for c in w.cols.iter().filter(|c| c.custom_width || c.hidden) {
                for at in c.min..=c.max.min(16384) {
                    columns.push(LineSize {
                        at,
                        size: c.custom_width.then(|| w.get_actual_column_width(at).ok()).flatten(),
                        hidden: c.hidden,
                    });
                }
            }
            sheets.push(FileSheet {
                id,
                name: w.get_name(),
                info: book.info(id),
                frozen: (w.frozen_rows, w.frozen_columns),
                grid: w.show_grid_lines,
                rows,
                columns,
                cells,
                merged: w.merged_cells.clone(),
            });
        }
        let names = book
            .model
            .workbook
            .defined_names
            .iter()
            .map(|d| FileName { name: d.name.clone(), formula: d.formula.clone(), sheet: d.sheet_id })
            .collect();
        FileBook {
            format: FORMAT.to_string(),
            version: needs(book),
            saved_with: VERSION,
            name: book.model.workbook.name.clone(),
            timezone: book.model.get_timezone(),
            sheets,
            styles,
            tables: crate::tables::all(book),
            names,
            notes: book.notes.clone(),
            filters: book.filters.clone(),
            charts: book.charts.clone(),
            pictures: book
                .pictures
                .iter()
                .map(|p| crate::pictures::Picture {
                    data: String::new(),
                    part: format!("pictures/{}.{}", p.id, p.extension()),
                    ..p.clone()
                })
                .collect(),
            sparklines: book.sparklines.clone(),
            summaries: book.summaries.clone(),
            reports: book.reports.clone(),
            queries: book.queries.clone(),
            editors: book.editors.clone(),
            csv_links: book.csv_links.clone(),
            kept_sheets: book.kept.as_ref().map(|k| k.sheets.clone()).unwrap_or_default(),
            kept_imported: book.kept.as_ref().map(|k| k.imported.clone()).unwrap_or_default(),
            definitions: book.definitions.values().cloned().collect(),
        }
    }

    /// The book from calc.json: the format and levels checked first, so a
    /// newer file whose new parts this version can't read says so instead of
    /// "damaged".
    pub fn read(json: &[u8]) -> Result<FileBook, String> {
        let raw: serde_json::Value =
            serde_json::from_slice(json).map_err(|e| format!("This Calc file is damaged: {e}"))?;
        if raw["format"].as_str() != Some(FORMAT) {
            return Err("This isn't a Calc file".to_string());
        }
        let level = |k: &str| raw[k].as_u64().unwrap_or(0);
        if level("version") > VERSION as u64 {
            return Err(NEWER.to_string());
        }
        let newer = level("saved_with") > VERSION as u64;
        serde_json::from_value(raw).map_err(|e| if newer { NEWER.to_string() } else { format!("This Calc file is damaged: {e}") })
    }

    pub fn into_book(self) -> Result<Book, String> {
        if self.format != FORMAT {
            return Err("This isn't a Calc file".to_string());
        }
        if self.version > VERSION {
            return Err(NEWER.to_string());
        }
        if self.sheets.is_empty() {
            return Err("This file has no sheets".to_string());
        }
        let mut book = Book::new();
        if !self.timezone.is_empty() {
            let _ = book.model.set_timezone(&self.timezone);
        }
        if !self.name.is_empty() {
            book.model.workbook.name = self.name.clone();
        }
        // Sheets first (formulas name them), then names and tables (formulas
        // use them), then the cells.
        let first = book.model.workbook.worksheets[0].sheet_id;
        for (k, s) in self.sheets.iter().enumerate() {
            book.model.insert_sheet(&format!("__new{k}"), k as u32 + 1, Some(s.id + 100_000))?;
        }
        book.model.delete_sheet(0)?;
        book.info.remove(&first);
        for (k, s) in self.sheets.iter().enumerate() {
            let w = &mut book.model.workbook.worksheets[k];
            w.sheet_id = s.id;
            w.set_name(&s.name);
            book.info.insert(s.id, s.info.clone());
        }
        book.model.workbook.defined_names = self
            .names
            .iter()
            .map(|n| DefinedName { name: n.name.clone(), formula: n.formula.clone(), sheet_id: n.sheet })
            .collect();
        for t in &self.tables {
            crate::tables::set(&mut book, &t.name, Some(t))?;
        }
        book.model.refresh_tables();
        for (k, s) in self.sheets.iter().enumerate() {
            let i = k as u32;
            for c in &s.cells {
                if c.s != 0
                    && let Some(style) = self.styles.get(c.s)
                {
                    book.model.set_cell_style(i, c.r, c.c, style)?;
                }
            }
            for c in &s.cells {
                if let Some(d) = &c.data {
                    book.put(s.id, c.r, c.c, Some(d)).map_err(|e| format!("{}!R{}C{}: {e}", s.name, c.r, c.c))?;
                }
            }
            for r in &s.rows {
                if let Some(h) = r.size {
                    book.model.set_row_height(i, r.at, h)?;
                }
                if r.hidden {
                    book.model.set_row_hidden(i, r.at, true)?;
                }
            }
            for c in &s.columns {
                if let Some(w) = c.size {
                    book.model.set_column_width(i, c.at, w)?;
                }
                if c.hidden {
                    book.model.set_column_hidden(i, c.at, true)?;
                }
            }
            book.model.set_frozen_rows(i, s.frozen.0)?;
            book.model.set_frozen_columns(i, s.frozen.1)?;
            book.model.set_show_grid_lines(i, s.grid)?;
            book.model.workbook.worksheets[k].merged_cells = s.merged.clone();
            book.sync_state(s.id);
        }
        book.notes = self.notes;
        book.filters = self.filters;
        book.charts = self.charts;
        book.pictures = self.pictures;
        book.sparklines = self.sparklines;
        book.summaries = self.summaries;
        book.reports = self.reports;
        book.queries = self.queries;
        book.editors = self.editors;
        book.csv_links = self.csv_links;
        let _ = self.kept_sheets;
        book.definitions = self.definitions.into_iter().map(|d| (d.name.clone(), d)).collect::<BTreeMap<_, _>>();
        book.model.refresh_tables();
        Ok(book)
    }
}

/// The bytes of a `.xcalc` file.
pub fn save(book: &Book) -> Result<Vec<u8>, String> {
    save_with(book, None)
}

/// The same, with `link` (what Write shows of the book: its tables,
/// summaries, sheets as shown and charts as pictures) as `link.json`, so a
/// Write document can link to the file and follow it as it changes.
pub fn save_with(book: &Book, link: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(&FileBook::from_book(book)).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // First and stored, so the kind of file shows at the start.
    zip.start_file(
        "mimetype",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .map_err(|e| e.to_string())?;
    zip.write_all(b"application/vnd.xenify.calc").map_err(|e| e.to_string())?;
    zip.start_file("calc.json", deflate).map_err(|e| e.to_string())?;
    zip.write_all(&json).map_err(|e| e.to_string())?;
    // Pictures, each a file of its own (already compressed: stored).
    for p in &book.pictures {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&p.data) else { continue };
        zip.start_file(
            format!("pictures/{}.{}", p.id, p.extension()),
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    if let Some(link) = link {
        zip.start_file("link.json", deflate).map_err(|e| e.to_string())?;
        zip.write_all(link).map_err(|e| e.to_string())?;
    }
    if let Some(k) = &book.kept {
        zip.start_file(
            "kept.xlsx",
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .map_err(|e| e.to_string())?;
        zip.write_all(&k.original).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

pub const TOO_BIG: &str = "This file is too big to open.";

/// What a zip may unpack to, all its entries together. Pictures are kept as
/// they are (not squeezed), so a real file unpacks to about its own size,
/// its sheets to some tens of times less than they take in the zip; a
/// made-up one squeezes far more (up to a thousand times), to fill the
/// memory. So: 64 times the file, and at least 64 MB.
pub(crate) struct Budget(u64);

impl Budget {
    pub(crate) fn for_file(len: usize) -> Budget {
        Budget((len as u64).saturating_mul(64).max(64 << 20))
    }

    /// An entry's bytes, read with what's left. `claimed`, what the zip says
    /// it holds, only decides refusing early; nothing is set aside on its
    /// word.
    pub(crate) fn read(&mut self, claimed: u64, r: impl Read) -> Result<Vec<u8>, String> {
        if claimed > self.0 {
            return Err(TOO_BIG.into());
        }
        let mut out = Vec::new();
        r.take(self.0 + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
        if out.len() as u64 > self.0 {
            return Err(TOO_BIG.into());
        }
        self.0 -= out.len() as u64;
        Ok(out)
    }
}

/// A book from the bytes of a `.xcalc` file.
pub fn open(bytes: &[u8]) -> Result<Book, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "This isn't a Calc file".to_string())?;
    let mut budget = Budget::for_file(bytes.len());
    let json = {
        let f = zip.by_name("calc.json").map_err(|_| "This Calc file is missing its sheets".to_string())?;
        budget.read(f.size(), f)?
    };
    let file = FileBook::read(&json)?;
    let kept_sheets = file.kept_sheets.clone();
    let kept_imported = file.kept_imported.clone();
    let mut book = file.into_book()?;
    let mut read_part = |name: &str| -> Result<Option<Vec<u8>>, String> {
        match zip.by_name(name) {
            Ok(f) => budget.read(f.size(), f).map(Some),
            Err(_) => Ok(None),
        }
    };
    // Each part once (many pictures naming one part would read it many times).
    let mut parts: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    for p in &mut book.pictures {
        if !parts.contains_key(&p.part) {
            let data = read_part(&p.part)?.map(|b| base64::engine::general_purpose::STANDARD.encode(&b));
            parts.insert(p.part.clone(), data);
        }
        if let Some(Some(data)) = parts.get(&p.part) {
            p.data = data.clone();
        }
        p.part.clear();
    }
    book.pictures.retain(|p| !p.data.is_empty());
    if let Some(original) = read_part("kept.xlsx")?
        && !original.is_empty()
    {
        book.kept = Some(crate::xlsx::Kept { original, sheets: kept_sheets, imported: kept_imported });
    }
    Ok(book)
}
