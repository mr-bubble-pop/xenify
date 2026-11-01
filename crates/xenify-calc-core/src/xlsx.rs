//! Excel files. IronCalc reads and writes the cells, formulas, styles,
//! merged cells, sizes, frozen panes, names, conditional formatting and
//! links; Calc adds tables (their parts, totals functions and filters) and
//! its own extras (kinds of sheet, column notes, definitions, charts) in a
//! part of its own, and keeps everything else it can't edit as it was:
//! pictures, drawings and charts, comments, pivot tables, macros, external
//! links, data validation, page setup, sparklines. Kept parts stay with
//! their sheet (by its id), so renaming or moving sheets doesn't lose them.

use crate::book::{Book, SheetInfo};
use base64::Engine as _;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Write};

/// The Excel file a book was opened from, for the parts Calc keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    pub original: Vec<u8>,
    /// Each sheet's part in `original`, by sheet id.
    pub sheets: BTreeMap<u32, String>,
    /// Chart parts that became Calc charts (written anew from Calc's charts).
    pub imported: Vec<String>,
}

/// Calc's extras, in `xenify/calc.json` inside the Excel file.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Extras {
    format: String,
    version: u32,
    sheets: BTreeMap<u32, SheetInfo>,
    notes: Vec<crate::book::ColumnNote>,
    definitions: Vec<crate::book::Definition>,
    filters: BTreeMap<String, Vec<crate::edit::ColumnFilter>>,
    charts: Vec<crate::charts::Chart>,
    summaries: BTreeMap<u32, crate::summary::Def>,
    reports: BTreeMap<u32, crate::report::Def>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    queries: BTreeMap<u32, crate::query::Def>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    editors: BTreeMap<u32, crate::editor::Def>,
    /// Calc's pictures (their bytes are the media parts named by `part`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pictures: Vec<crate::pictures::Picture>,
}

impl Extras {
    /// Part by part: something this version can't read (from a newer Calc,
    /// or changed by hand) loses only that part, not every extra.
    fn read(json: &[u8]) -> Option<Extras> {
        let v: serde_json::Value = serde_json::from_slice(json).ok()?;
        let part = |k: &str| v.get(k).cloned().unwrap_or(serde_json::Value::Null);
        fn get<T: Default + serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
            if v.is_null() { T::default() } else { serde_json::from_value(v).unwrap_or_default() }
        }
        // Sheets one by one too: one sheet of a new kind leaves the others.
        let sheets = part("sheets")
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(id, info)| Some((id.parse().ok()?, serde_json::from_value(info.clone()).ok()?)))
                    .collect()
            })
            .unwrap_or_default();
        Some(Extras {
            format: get(part("format")),
            version: get(part("version")),
            sheets,
            notes: get(part("notes")),
            definitions: get(part("definitions")),
            filters: get(part("filters")),
            charts: get(part("charts")),
            summaries: get(part("summaries")),
            reports: get(part("reports")),
            queries: get(part("queries")),
            editors: get(part("editors")),
            pictures: get(part("pictures")),
        })
    }
}

const EXTRAS_PART: &str = "xenify/calc.json";
const EXTRAS_REL: &str = "https://xenify.app/calc/extras";
const REL_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const REL_HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const CT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const CT_DRAWING: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
const REL_DRAWING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const CT_MAIN: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const CT_MAIN_MACROS: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";
/// Conditional formats in an extension: IronCalc writes these itself.
const EXT_CF: &str = "{78C0D931-6437-407d-A8EE-F0AAD7539E65}";

/// Worksheet children in the order the format requires.
const SHEET_ORDER: &[&str] = &[
    "sheetPr",
    "dimension",
    "sheetViews",
    "sheetFormatPr",
    "cols",
    "sheetData",
    "sheetCalcPr",
    "sheetProtection",
    "protectedRanges",
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];
/// The ones Calc keeps from the original (IronCalc writes the rest).
const SHEET_KEPT: &[&str] = &[
    "sheetPr",
    "sheetFormatPr",
    "sheetCalcPr",
    "sheetProtection",
    "protectedRanges",
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "phoneticPr",
    "dataValidations",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
];
const BOOK_ORDER: &[&str] = &[
    "fileVersion",
    "fileSharing",
    "workbookPr",
    "workbookProtection",
    "bookViews",
    "sheets",
    "functionGroups",
    "externalReferences",
    "definedNames",
    "calcPr",
    "oleSize",
    "customWorkbookViews",
    "pivotCaches",
    "smartTagPr",
    "smartTagTypes",
    "webPublishing",
    "fileRecoveryPr",
    "webPublishObjects",
    "extLst",
];
const BOOK_KEPT: &[&str] =
    &["fileVersion", "workbookPr", "workbookProtection", "externalReferences", "pivotCaches", "extLst"];
/// Workbook relationships IronCalc writes itself (or Excel rebuilds).
const BOOK_RELS_OWN: &[&str] = &["/worksheet", "/styles", "/sharedStrings", "/theme", "/sheetMetadata", "/calcChain"];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ----- Reading the package -----

/// Whether XML `b` nests elements deeper than 100. IronCalc's XML reader
/// goes into nested elements by recursion: a made-up file nesting many
/// thousands would run out of stack and crash the app. Real Excel parts nest
/// a dozen or so. It errs on the deep side (what it doesn't follow counts as
/// opening).
fn too_deep(b: &[u8]) -> bool {
    const MAX: usize = 100;
    let find = |from: usize, what: &[u8]| {
        b.get(from..).and_then(|r| r.windows(what.len()).position(|w| w == what)).map_or(b.len(), |p| from + p + what.len())
    };
    let (mut i, mut d) = (0, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        if rest.starts_with(b"<!--") {
            i = find(i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = find(i + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = find(i + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            i = find(i + 2, b">");
        } else if rest.starts_with(b"</") {
            d = d.saturating_sub(1);
            i = find(i + 2, b">");
        } else {
            let mut j = i + 1;
            let mut quote = None;
            let mut closed = false;
            while j < b.len() {
                match (quote, b[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, c @ (b'"' | b'\'')) => quote = Some(c),
                    (None, b'>') => {
                        closed = b[j - 1] == b'/';
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if !closed {
                d += 1;
                if d > MAX {
                    return true;
                }
            }
            i = j + 1;
        }
    }
    false
}

/// The parts of a zip, in order.
struct Package {
    parts: Vec<(String, Vec<u8>)>,
}

impl Package {
    fn read(bytes: &[u8]) -> Result<Package, String> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "This isn't an .xlsx file".to_string())?;
        let mut parts = Vec::new();
        // Read with a limit (see file::Budget), each part's bytes its own:
        // a small made-up file can't fill the memory, here or in IronCalc's
        // reader after this (which believes the sizes the zip gives).
        let mut budget = crate::file::Budget::for_file(bytes.len());
        let mut starts = std::collections::HashSet::new();
        for k in 0..zip.len() {
            let f = zip.by_index(k).map_err(err)?;
            if f.is_dir() {
                continue;
            }
            if !starts.insert(f.header_start()) {
                return Err(crate::file::TOO_BIG.into());
            }
            let name = f.name().map_err(err)?.to_string();
            let b = budget.read(f.size(), f)?;
            parts.push((name, b));
        }
        Ok(Package { parts })
    }

    fn get(&self, name: &str) -> Option<&[u8]> {
        self.parts.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice())
    }

    fn text(&self, name: &str) -> Option<String> {
        self.get(name).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    fn put(&mut self, name: &str, bytes: Vec<u8>) {
        match self.parts.iter_mut().find(|(n, _)| n == name) {
            Some(p) => p.1 = bytes,
            None => self.parts.push((name.to_string(), bytes)),
        }
    }

    fn write(&self) -> Result<Vec<u8>, String> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        // The content types first, as Excel expects.
        let mut order: Vec<&(String, Vec<u8>)> = self.parts.iter().collect();
        order.sort_by_key(|(n, _)| n != "[Content_Types].xml");
        for (name, bytes) in order {
            zip.start_file(name.as_str(), opts).map_err(err)?;
            zip.write_all(bytes).map_err(err)?;
        }
        Ok(zip.finish().map_err(err)?.into_inner())
    }
}

/// A relationship: id, type, target, and whether the target is outside the file.
#[derive(Clone, Debug)]
struct Rel {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

fn attrs(e: &quick_xml::events::BytesStart) -> HashMap<String, String> {
    e.attributes()
        .flatten()
        .map(|a| {
            let key = a.key.as_ref().to_string();
            let value =
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default();
            (key, value)
        })
        .collect()
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

fn read_rels(xml: &str) -> Vec<Rel> {
    let mut out = Vec::new();
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local(e.name().as_ref()) == "Relationship" => {
                let a = attrs(&e);
                out.push(Rel {
                    id: a.get("Id").cloned().unwrap_or_default(),
                    kind: a.get("Type").cloned().unwrap_or_default(),
                    target: a.get("Target").cloned().unwrap_or_default(),
                    external: a.get("TargetMode").is_some_and(|m| m == "External"),
                });
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn write_rels(rels: &[Rel]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for r in rels {
        s.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
            esc(&r.id),
            esc(&r.kind),
            esc(&r.target),
            if r.external { " TargetMode=\"External\"" } else { "" }
        ));
    }
    s.push_str("</Relationships>");
    s
}

use xenify_util::esc;

/// The rels part for part `p` (xl/worksheets/sheet1.xml → xl/worksheets/_rels/sheet1.xml.rels).
fn rels_path(p: &str) -> String {
    match p.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{p}.rels"),
    }
}

/// `target` (relative to part `from`) as a part name.
fn resolve(from: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The root's start tag and its top-level children: (local name, raw XML).
fn children(xml: &str) -> (String, Vec<(String, String)>) {
    let mut r = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut root = String::new();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut name = String::new();
    loop {
        let before = r.buffer_position() as usize;
        let ev = r.read_event();
        let after = r.buffer_position() as usize;
        match ev {
            Ok(Event::Start(e)) => {
                if depth == 0 {
                    root = xml[before..after].to_string();
                } else if depth == 1 {
                    start = before;
                    name = local(e.name().as_ref());
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) => {
                if depth == 1 {
                    out.push((local(e.name().as_ref()), xml[before..after].to_string()));
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    out.push((name.clone(), xml[start..after].to_string()));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (root, out)
}

/// The `<ext>` elements of an `<extLst>`: (uri, raw XML).
fn exts(ext_lst: &str) -> Vec<(String, String)> {
    let (_, kids) = children(ext_lst);
    kids.into_iter()
        .filter(|(n, _)| n == "ext")
        .map(|(_, raw)| {
            let uri = raw.split("uri=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("").to_string();
            (uri, raw)
        })
        .collect()
}

/// Every `xmlns…` declaration (and `mc:Ignorable`) on a start tag.
fn namespaces(start: &str) -> Vec<(String, String)> {
    let mut r = Reader::from_str(start);
    if let Ok(Event::Start(e)) | Ok(Event::Empty(e)) = r.read_event() {
        return attrs(&e).into_iter().filter(|(k, _)| k.starts_with("xmlns") || k == "mc:Ignorable").collect();
    }
    Vec::new()
}

/// `start` with the namespace declarations in `more` it lacks.
fn with_namespaces(start: &str, more: &[(String, String)]) -> String {
    let have = namespaces(start);
    let mut add = String::new();
    for (k, v) in more {
        if !have.iter().any(|(hk, _)| hk == k) {
            add.push_str(&format!(" {k}=\"{}\"", esc(v)));
        }
    }
    match start.rfind('>') {
        Some(k) => format!("{}{add}{}", &start[..k], &start[k..]),
        None => start.to_string(),
    }
}

/// The content types: defaults by extension and overrides by part.
struct Types {
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
}

impl Types {
    fn read(xml: &str) -> Types {
        let mut t = Types { defaults: Vec::new(), overrides: Vec::new() };
        let mut r = Reader::from_str(xml);
        loop {
            match r.read_event() {
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                    let a = attrs(&e);
                    match local(e.name().as_ref()).as_str() {
                        "Default" => t.defaults.push((
                            a.get("Extension").cloned().unwrap_or_default(),
                            a.get("ContentType").cloned().unwrap_or_default(),
                        )),
                        "Override" => t.overrides.push((
                            a.get("PartName").cloned().unwrap_or_default(),
                            a.get("ContentType").cloned().unwrap_or_default(),
                        )),
                        _ => {}
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        t
    }

    fn of(&self, part: &str) -> Option<String> {
        let name = format!("/{part}");
        if let Some((_, ct)) = self.overrides.iter().find(|(p, _)| p.eq_ignore_ascii_case(&name)) {
            return Some(ct.clone());
        }
        let ext = part.rsplit('.').next()?.to_lowercase();
        self.defaults.iter().find(|(e, _)| e.to_lowercase() == ext).map(|(_, ct)| ct.clone())
    }

    fn set_override(&mut self, part: &str, ct: &str) {
        let name = format!("/{part}");
        match self.overrides.iter_mut().find(|(p, _)| *p == name) {
            Some(o) => o.1 = ct.to_string(),
            None => self.overrides.push((name, ct.to_string())),
        }
    }

    fn ensure_default(&mut self, ext: &str, ct: &str) {
        if !self.defaults.iter().any(|(e, _)| e.eq_ignore_ascii_case(ext)) {
            self.defaults.push((ext.to_string(), ct.to_string()));
        }
    }

    fn write(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        );
        for (e, ct) in &self.defaults {
            s.push_str(&format!("<Default Extension=\"{}\" ContentType=\"{}\"/>", esc(e), esc(ct)));
        }
        for (p, ct) in &self.overrides {
            s.push_str(&format!("<Override PartName=\"{}\" ContentType=\"{}\"/>", esc(p), esc(ct)));
        }
        s.push_str("</Types>");
        s
    }
}

// ----- Opening -----

/// What IronCalc doesn't read from table parts: each column's totals
/// function or label, and the values each filter keeps.
fn read_table_extras(pkg: &Package, book: &mut Book) {
    for (name, bytes) in pkg.parts.iter().filter(|(n, _)| n.starts_with("xl/tables/") && n.ends_with(".xml")) {
        let _ = name;
        let xml = String::from_utf8_lossy(bytes);
        let mut r = Reader::from_str(&xml);
        let mut table = String::new();
        let mut columns: Vec<(Option<String>, Option<String>, Option<String>)> = Vec::new();
        let mut filters: Vec<(usize, Vec<String>)> = Vec::new();
        let mut in_formula = false;
        let mut filter_col: Option<usize> = None;
        loop {
            match r.read_event() {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let a = attrs(&e);
                    match local(e.name().as_ref()).as_str() {
                        "table" => table = a.get("name").or_else(|| a.get("displayName")).cloned().unwrap_or_default(),
                        "tableColumn" => {
                            columns.push((a.get("totalsRowFunction").cloned(), a.get("totalsRowLabel").cloned(), None))
                        }
                        "totalsRowFormula" => in_formula = true,
                        "filterColumn" => {
                            filter_col = a.get("colId").and_then(|v| v.parse().ok());
                            if let Some(k) = filter_col {
                                filters.push((k, Vec::new()));
                            }
                        }
                        "filter" => {
                            if let (Some(_), Some(last)) = (filter_col, filters.last_mut()) {
                                last.1.push(a.get("val").cloned().unwrap_or_default());
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(t)) if in_formula => {
                    if let Some(c) = columns.last_mut() {
                        c.2 = Some(t.as_ref().to_string());
                    }
                }
                Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                    "totalsRowFormula" => in_formula = false,
                    "filterColumn" => filter_col = None,
                    _ => {}
                },
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        let Some(t) = book.model.workbook.tables.values_mut().find(|t| t.name.eq_ignore_ascii_case(&table)) else {
            continue;
        };
        for (k, (func, label, _)) in columns.iter().enumerate() {
            if let Some(c) = t.columns.get_mut(k) {
                c.totals_row_function = func.clone();
                c.totals_row_label = label.clone();
            }
        }
        let names: Vec<String> = t.columns.iter().map(|c| c.name.clone()).collect();
        let tname = t.name.clone();
        let kept: Vec<crate::edit::ColumnFilter> = filters
            .into_iter()
            .filter_map(|(k, keep)| names.get(k).map(|n| crate::edit::ColumnFilter { column: n.clone(), keep }))
            .collect();
        if !kept.is_empty() {
            book.filters.insert(tname, kept);
        }
    }
}

/// The drawing part of a sheet part (if it has one) and its anchors.
fn sheet_anchors(pkg: &Package, sheet_part: &str) -> Vec<(String, crate::drawings::Anchor)> {
    let rels = read_rels(&pkg.text(&rels_path(sheet_part)).unwrap_or_default());
    let mut out = Vec::new();
    for r in rels.iter().filter(|r| r.kind.ends_with("/drawing") && !r.external) {
        let drawing = resolve(sheet_part, &r.target);
        if let Some(xml) = pkg.text(&drawing) {
            for a in crate::drawings::anchors(&xml) {
                out.push((drawing.clone(), a));
            }
        }
    }
    out
}

/// The pictures on a sheet (from its Excel file), for the screens.
pub fn pictures(book: &Book, sheet: u32) -> Vec<serde_json::Value> {
    let Some(k) = &book.kept else { return Vec::new() };
    let Some(part) = k.sheets.get(&sheet) else { return Vec::new() };
    let Ok(index) = book.index(sheet) else { return Vec::new() };
    let Ok(pkg) = Package::read(&k.original) else { return Vec::new() };
    let mut out = Vec::new();
    for (drawing, a) in sheet_anchors(&pkg, part) {
        let Some(rid) = &a.picture else { continue };
        let rels = read_rels(&pkg.text(&rels_path(&drawing)).unwrap_or_default());
        let Some(rel) = rels.iter().find(|r| r.id == *rid && !r.external) else { continue };
        let media = resolve(&drawing, &rel.target);
        // Calc's own pictures are Calc's (book.pictures), not kept ones.
        if k.imported.contains(&media) {
            continue;
        }
        if let Some(bytes) = pkg.get(&media) {
            out.push(crate::drawings::picture_json(book, index, &a, bytes, &media));
        }
    }
    out
}

/// Each sheet's part, by sheet id, from a workbook's parts.
fn sheet_parts(pkg: &Package) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let Some(wb) = pkg.text("xl/workbook.xml") else { return out };
    let rels = read_rels(&pkg.text("xl/_rels/workbook.xml.rels").unwrap_or_default());
    let mut r = Reader::from_str(&wb);
    loop {
        match r.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local(e.name().as_ref()) == "sheet" => {
                let a = attrs(&e);
                let id: u32 = a.get("sheetId").and_then(|v| v.parse().ok()).unwrap_or(0);
                let rid = a.iter().find(|(k, _)| k.ends_with(":id")).map(|(_, v)| v.clone()).unwrap_or_default();
                if let Some(rel) = rels.iter().find(|x| x.id == rid && x.kind.ends_with("/worksheet")) {
                    out.insert(id, resolve("xl/workbook.xml", &rel.target));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// A book from the bytes of an Excel file.
pub fn open(bytes: &[u8], name: &str) -> Result<Book, String> {
    // Read with limits first: IronCalc's reader below has none.
    let pkg = Package::read(bytes);
    match &pkg {
        Err(e) if e == crate::file::TOO_BIG => return Err(e.clone()),
        Ok(p) if p.parts.iter().any(|(n, b)| (n.ends_with(".xml") || n.ends_with(".rels")) && too_deep(b)) => {
            return Err("Couldn’t read this .xlsx file: its parts nest too deeply".into());
        }
        _ => {}
    }
    let wb = ironcalc::import::load_from_xlsx_bytes(bytes, name, "en", "UTC")
        .map_err(|e| format!("Couldn’t read this .xlsx file: {e}"))?;
    let model =
        ironcalc_base::Model::from_workbook(wb, "en").map_err(|e| format!("Couldn’t read this .xlsx file: {e}"))?;
    let mut book = Book::from_model(model);
    let pkg = pkg?;
    // Hidden sheets are working sheets.
    let ids: Vec<(u32, bool)> = book
        .model
        .workbook
        .worksheets
        .iter()
        .map(|w| (w.sheet_id, !matches!(w.state, ironcalc_base::types::SheetState::Visible)))
        .collect();
    for (id, hidden) in ids {
        book.info.insert(id, SheetInfo { working: hidden, ..Default::default() });
    }
    let mut calc_pictures: Vec<String> = Vec::new();
    if let Some(json) = pkg.get(EXTRAS_PART)
        && let Some(x) = Extras::read(json)
    {
        for (id, info) in x.sheets {
            if book.index(id).is_ok() {
                book.info.insert(id, info);
            }
        }
        book.notes = x.notes;
        book.definitions = x.definitions.into_iter().map(|d| (d.name.clone(), d)).collect();
        book.filters = x.filters;
        book.charts = x.charts;
        book.summaries = x.summaries;
        book.reports = x.reports;
        book.queries = x.queries;
        book.editors = x.editors;
        // Calc's pictures: their bytes from the media parts (written again on save).
        for mut p in x.pictures {
            if let Some(bytes) = pkg.get(&p.part) {
                p.data = base64::engine::general_purpose::STANDARD.encode(bytes);
                calc_pictures.push(p.part.clone());
                p.part.clear();
                book.pictures.push(p);
            }
        }
    }
    read_table_extras(&pkg, &mut book);
    // Tables with filters on: their hidden rows are filtered ones.
    for (name, f) in book.filters.clone() {
        if let Some(t) = book.model.workbook.tables.values_mut().find(|t| t.name == name) {
            t.has_filters = !f.is_empty();
        }
    }
    let sheets = sheet_parts(&pkg);
    for (id, part) in &sheets {
        if let Some(xml) = pkg.text(part) {
            let found = crate::sparklines::read(&book, *id, &xml);
            book.sparklines.extend(found);
        }
    }
    // Excel's charts become Calc charts (unless Calc's own were saved in the file).
    let mut imported = std::mem::take(&mut calc_pictures);
    let had_calc_charts = pkg.get(EXTRAS_PART).is_some();
    for (id, part) in &sheets {
        let Ok(index) = book.index(*id) else { continue };
        for (drawing, a) in sheet_anchors(&pkg, part) {
            let Some(rid) = &a.chart else { continue };
            let rels = read_rels(&pkg.text(&rels_path(&drawing)).unwrap_or_default());
            let Some(rel) = rels.iter().find(|r| r.id == *rid) else { continue };
            let chart_part = resolve(&drawing, &rel.target);
            if had_calc_charts {
                imported.push(chart_part);
                continue;
            }
            let Some(xml) = pkg.text(&chart_part) else { continue };
            let next = book.charts.iter().map(|c| c.id).max().unwrap_or(0) + 1;
            let at = crate::drawings::place(&book, index, &a);
            if let Some(c) = crate::drawings::import_chart(&book, &xml, next, *id, at) {
                book.charts.push(c);
                imported.push(chart_part);
            }
        }
    }
    book.kept = Some(Kept { original: bytes.to_vec(), sheets, imported });
    book.model.refresh_tables();
    Ok(book)
}

// ----- Saving -----

fn table_xml(book: &Book, t: &crate::tables::TableDef, id: usize) -> String {
    let col = crate::tables::column_letters;
    let reference = format!("{}{}:{}{}", col(t.left), t.top, col(t.right), t.bottom);
    let filter_ref = format!("{}{}:{}{}", col(t.left), t.top, col(t.right), t.bottom - t.totals as i32);
    let filters = book.filters.get(&t.name).cloned().unwrap_or_default();
    let mut filter_cols = String::new();
    for f in &filters {
        if let Some(k) = t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(&f.column)) {
            let vals: String = f.keep.iter().map(|v| format!("<filter val=\"{}\"/>", esc(v))).collect();
            let blank = if f.keep.iter().any(|v| v.is_empty()) { " blank=\"1\"" } else { "" };
            filter_cols
                .push_str(&format!("<filterColumn colId=\"{k}\"><filters{blank}>{vals}</filters></filterColumn>"));
        }
    }
    let auto_filter = if t.header {
        if filter_cols.is_empty() {
            format!("<autoFilter ref=\"{filter_ref}\"/>")
        } else {
            format!("<autoFilter ref=\"{filter_ref}\">{filter_cols}</autoFilter>")
        }
    } else {
        String::new()
    };
    let mut cols = String::new();
    for (k, c) in t.columns.iter().enumerate() {
        let total = if t.totals {
            let cell = book.cell_data(t.sheet, t.bottom, t.left + k as i32);
            match (c.total, cell) {
                (Some(crate::tables::Total::Median), Some(crate::book::CellData::Formula(f))) => format!(
                    " totalsRowFunction=\"custom\"><totalsRowFormula>{}</totalsRowFormula></tableColumn",
                    esc(f.trim_start_matches('='))
                ),
                (Some(f), _) => format!(" totalsRowFunction=\"{}\"", total_name(f)),
                (None, Some(crate::book::CellData::Formula(f))) => format!(
                    " totalsRowFunction=\"custom\"><totalsRowFormula>{}</totalsRowFormula></tableColumn",
                    esc(f.trim_start_matches('='))
                ),
                (None, Some(crate::book::CellData::Text(s))) => format!(" totalsRowLabel=\"{}\"", esc(&s)),
                _ => String::new(),
            }
        } else {
            String::new()
        };
        if total.ends_with("</tableColumn") {
            cols.push_str(&format!("<tableColumn id=\"{}\" name=\"{}\"{total}>", k + 1, esc(&c.name)));
        } else {
            cols.push_str(&format!("<tableColumn id=\"{}\" name=\"{}\"{total}/>", k + 1, esc(&c.name)));
        }
    }
    let totals = if t.totals { " totalsRowCount=\"1\"" } else { " totalsRowShown=\"0\"" };
    let header = if t.header { "" } else { " headerRowCount=\"0\"" };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{id}\" name=\"{n}\" displayName=\"{n}\" ref=\"{reference}\"{header}{totals}>{auto_filter}<tableColumns count=\"{}\">{cols}</tableColumns><tableStyleInfo name=\"TableStyleLight1\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"{}\" showColumnStripes=\"0\"/></table>",
        t.columns.len(),
        if t.stripes { 1 } else { 0 },
        n = esc(&t.name),
    )
}

fn total_name(t: crate::tables::Total) -> &'static str {
    use crate::tables::Total::*;
    match t {
        Sum => "sum",
        Average => "average",
        Count => "count",
        Min => "min",
        Max => "max",
        Median => "custom",
    }
}

/// Copies part `name` from `orig` (and every part its relationships lead
/// to) into `out`, once; their content types too.
fn copy_part(
    orig: &Package,
    orig_types: &Types,
    out: &mut Package,
    types: &mut Types,
    name: &str,
    done: &mut HashSet<String>,
) {
    if !done.insert(name.to_string()) {
        return;
    }
    let Some(bytes) = orig.get(name) else { return };
    out.put(name, bytes.to_vec());
    if let Some(ct) = orig_types.of(name) {
        let ext = name.rsplit('.').next().unwrap_or("");
        if orig_types.overrides.iter().any(|(p, _)| p.eq_ignore_ascii_case(&format!("/{name}"))) {
            types.set_override(name, &ct);
        } else {
            types.ensure_default(ext, &ct);
        }
    }
    let rp = rels_path(name);
    if let Some(rels) = orig.text(&rp) {
        out.put(&rp, rels.clone().into_bytes());
        for r in read_rels(&rels) {
            if !r.external {
                copy_part(orig, orig_types, out, types, &resolve(name, &r.target), done);
            }
        }
    }
}

/// `raw` with its `r:id="…"` (any prefix) values changed by `map`.
fn remap_ids(raw: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(k) = rest.find(":id=\"") {
        let (head, tail) = rest.split_at(k + 5);
        out.push_str(head);
        let end = tail.find('"').unwrap_or(0);
        let id = &tail[..end];
        out.push_str(map.get(id).map(String::as_str).unwrap_or(id));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn order_of(order: &[&str], name: &str) -> usize {
    order.iter().position(|n| *n == name).unwrap_or(order.len() - 1)
}

/// Reassembles a root element from `children`, in `order`, with extensions merged.
fn assemble(
    root_start: &str,
    root_name: &str,
    mut kids: Vec<(String, String)>,
    order: &[&str],
    exts_list: Vec<(String, String)>,
) -> String {
    kids.retain(|(n, _)| n != "extLst");
    if !exts_list.is_empty() {
        let inner: String = exts_list.into_iter().map(|(_, raw)| raw).collect();
        kids.push(("extLst".to_string(), format!("<extLst>{inner}</extLst>")));
    }
    kids.sort_by_key(|(n, _)| order_of(order, n));
    let body: String = kids.into_iter().map(|(_, raw)| raw).collect();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{root_start}{body}</{root_name}>")
}

/// The bytes of an Excel file of the book.
pub fn save(book: &Book) -> Result<Vec<u8>, String> {
    let base = ironcalc::export::save_xlsx_to_writer(&book.model, Cursor::new(Vec::new()))
        .map_err(|e| format!("Couldn’t write the .xlsx file: {e}"))?;
    let mut pkg = Package::read(&base.into_inner())?;
    let mut types = Types::read(&pkg.text("[Content_Types].xml").unwrap_or_default());

    let kept = match &book.kept {
        Some(k) => Some((Package::read(&k.original)?, k)),
        None => None,
    };
    let orig_types = kept.as_ref().map(|(p, _)| Types::read(&p.text("[Content_Types].xml").unwrap_or_default()));
    let mut copied: HashSet<String> = HashSet::new();
    // Original sheet and table parts are never copied: they're written anew.
    if let Some((orig, k)) = &kept {
        for p in k.sheets.values() {
            copied.insert(p.clone());
        }
        for (n, _) in &orig.parts {
            if n.starts_with("xl/tables/") || n.starts_with("xl/worksheets/") {
                copied.insert(n.clone());
            }
        }
    }

    let all_tables = crate::tables::all(book);
    let mut table_no = 0usize;
    let mut chart_no = 0usize;
    let mut picture_no = 0usize;
    let mut saved_pictures: Vec<crate::pictures::Picture> = Vec::new();
    for (k, w) in book.model.workbook.worksheets.iter().enumerate() {
        let part = format!("xl/worksheets/sheet{}.xml", k + 1);
        let rp = rels_path(&part);
        let mut rels = read_rels(&pkg.text(&rp).unwrap_or_default());
        let base_xml = pkg.text(&part).unwrap_or_default();
        let (mut root, mut kids) = children(&base_xml);
        let mut ext_list: Vec<(String, String)> =
            kids.iter().filter(|(n, _)| n == "extLst").flat_map(|(_, raw)| exts(raw)).collect();
        let mut n = 0usize;
        let mut fresh = |rels: &[Rel]| loop {
            n += 1;
            let id = format!("rIdX{n}");
            if !rels.iter().any(|r| r.id == id) {
                return id;
            }
        };
        // Kept from the original sheet.
        if let (Some((orig, kp)), Some(ot)) = (&kept, &orig_types)
            && let Some(op) = kp.sheets.get(&w.sheet_id)
        {
            let oxml = orig.text(op).unwrap_or_default();
            let (oroot, okids) = children(&oxml);
            root = with_namespaces(&root, &namespaces(&oroot));
            let orels = read_rels(&orig.text(&rels_path(op)).unwrap_or_default());
            let mut map = HashMap::new();
            for r in &orels {
                if r.kind == REL_HYPERLINK || r.kind == REL_TABLE {
                    continue;
                }
                let id = fresh(&rels);
                map.insert(r.id.clone(), id.clone());
                let target = if r.external {
                    r.target.clone()
                } else {
                    let full = resolve(op, &r.target);
                    copy_part(orig, ot, &mut pkg, &mut types, &full, &mut copied);
                    // Relative to the new sheet part (same folder as the original's, usually).
                    if op.rsplit_once('/').map(|x| x.0) == Some("xl/worksheets") {
                        r.target.clone()
                    } else {
                        format!("/{full}")
                    }
                };
                rels.push(Rel { id, kind: r.kind.clone(), target, external: r.external });
            }
            for (name, raw) in okids {
                if name == "extLst" {
                    for (uri, e) in exts(&raw) {
                        // Sparklines are written from Calc's own (below).
                        if uri != EXT_CF
                            && uri != crate::sparklines::EXT_URI
                            && !ext_list.iter().any(|(u, _)| *u == uri)
                        {
                            ext_list.push((uri, remap_ids(&e, &map)));
                        }
                    }
                } else if SHEET_KEPT.contains(&name.as_str()) && !kids.iter().any(|(n, _)| *n == name) {
                    kids.push((name, remap_ids(&raw, &map)));
                }
            }
        }
        // Charts: Calc's, written as Excel charts in the sheet's drawing layer
        // (beside what else it holds; the Excel charts they came from go).
        let charts: Vec<&crate::charts::Chart> = book.charts.iter().filter(|c| c.sheet == w.sheet_id).collect();
        let imported: Vec<String> = book.kept.as_ref().map(|k| k.imported.clone()).unwrap_or_default();
        let drawing_rid = kids
            .iter()
            .find(|(n, _)| n == "drawing")
            .and_then(|(_, raw)| raw.split(":id=\"").nth(1).and_then(|x| x.split('"').next()).map(str::to_string));
        let existing =
            drawing_rid.as_ref().and_then(|rid| rels.iter().find(|r| r.id == *rid)).map(|r| resolve(&part, &r.target));
        let pictures: Vec<&crate::pictures::Picture> = book.pictures.iter().filter(|p| p.sheet == w.sheet_id).collect();
        if !charts.is_empty() || !pictures.is_empty() || (existing.is_some() && !imported.is_empty()) {
            let (drawing, start, mut anchors_xml, mut drels) = match &existing {
                Some(d) => {
                    let xml = pkg.text(d).unwrap_or_default();
                    let (root_start, _) = children(&xml);
                    let drels = read_rels(&pkg.text(&rels_path(d)).unwrap_or_default());
                    // Keep everything but the charts that are now Calc's.
                    let mut kept_xml = String::new();
                    let mut keep_rels = drels.clone();
                    for a in crate::drawings::anchors(&xml) {
                        // Charts that became Calc's, and pictures Calc wrote (they're written again).
                        let gone = a
                            .chart
                            .as_ref()
                            .or(a.picture.as_ref())
                            .and_then(|rid| drels.iter().find(|r| r.id == *rid))
                            .map(|r| resolve(d, &r.target))
                            .filter(|p| imported.contains(p));
                        match gone {
                            Some(chart_part) => {
                                keep_rels.retain(|r| resolve(d, &r.target) != chart_part);
                                pkg.parts.retain(|(n, _)| *n != chart_part && *n != rels_path(&chart_part));
                                types.overrides.retain(|(p, _)| *p != format!("/{chart_part}"));
                            }
                            None => kept_xml.push_str(&a.raw),
                        }
                    }
                    (d.clone(), root_start, kept_xml, keep_rels)
                }
                None => (
                    format!("xl/drawings/calcdrawing{}.xml", k + 1),
                    crate::drawings::DRAWING_START.to_string(),
                    String::new(),
                    Vec::new(),
                ),
            };
            for (n, c) in charts.iter().enumerate() {
                let Some(xml) = crate::drawings::chart_xml(book, c) else { continue };
                chart_no += 1;
                let cp = format!("xl/charts/calcchart{chart_no}.xml");
                pkg.put(&cp, xml.into_bytes());
                types.set_override(&cp, CT_CHART);
                let rid = format!("rIdC{chart_no}");
                drels.push(Rel {
                    id: rid.clone(),
                    kind: REL_CHART.to_string(),
                    target: format!("../charts/calcchart{chart_no}.xml"),
                    external: false,
                });
                if let Some(a) = crate::drawings::chart_anchor(book, c, &rid, n) {
                    anchors_xml.push_str(&a);
                }
            }
            for (n, p) in pictures.iter().enumerate() {
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&p.data) else { continue };
                picture_no += 1;
                let mp = format!("xl/media/calcpicture{picture_no}.{}", p.extension());
                pkg.put(&mp, bytes);
                types.set_override(&mp, &p.mime);
                let rid = format!("rIdP{picture_no}");
                drels.push(Rel {
                    id: rid.clone(),
                    kind: REL_IMAGE.to_string(),
                    target: format!("../media/calcpicture{picture_no}.{}", p.extension()),
                    external: false,
                });
                if let Some(a) = crate::drawings::picture_anchor(book, p, &rid, n) {
                    anchors_xml.push_str(&a);
                }
                saved_pictures.push(crate::pictures::Picture { data: String::new(), part: mp, ..(*p).clone() });
            }
            if anchors_xml.is_empty() && existing.is_none() {
                // Nothing to draw after all.
            } else {
                let root_name =
                    start.trim_start_matches('<').split([' ', '>']).next().unwrap_or("xdr:wsDr").to_string();
                let start = with_namespaces(&start, &namespaces(crate::drawings::DRAWING_START));
                pkg.put(&drawing, format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{start}{anchors_xml}</{root_name}>").into_bytes());
                pkg.put(&rels_path(&drawing), write_rels(&drels).into_bytes());
                types.set_override(&drawing, CT_DRAWING);
                if existing.is_none() {
                    let id = fresh(&rels);
                    rels.push(Rel {
                        id: id.clone(),
                        kind: REL_DRAWING.to_string(),
                        target: format!("../drawings/calcdrawing{}.xml", k + 1),
                        external: false,
                    });
                    kids.push(("drawing".to_string(), format!("<drawing r:id=\"{id}\"/>")));
                }
            }
        }
        if let Some(x) = crate::sparklines::ext_xml(book, w.sheet_id) {
            ext_list.push((crate::sparklines::EXT_URI.to_string(), x));
        }
        // Tables.
        let mine: Vec<&crate::tables::TableDef> = all_tables.iter().filter(|t| t.sheet == w.sheet_id).collect();
        if !mine.is_empty() {
            let mut parts = String::new();
            for t in &mine {
                table_no += 1;
                let tp = format!("xl/tables/table{table_no}.xml");
                pkg.put(&tp, table_xml(book, t, table_no).into_bytes());
                types.set_override(&tp, CT_TABLE);
                let id = fresh(&rels);
                rels.push(Rel {
                    id: id.clone(),
                    kind: REL_TABLE.to_string(),
                    target: format!("../tables/table{table_no}.xml"),
                    external: false,
                });
                parts.push_str(&format!("<tablePart r:id=\"{id}\"/>"));
            }
            kids.push(("tableParts".to_string(), format!("<tableParts count=\"{}\">{parts}</tableParts>", mine.len())));
        }
        if !rels.is_empty() {
            pkg.put(&rp, write_rels(&rels).into_bytes());
        }
        pkg.put(&part, assemble(&root, "worksheet", kids, SHEET_ORDER, ext_list).into_bytes());
    }

    // The workbook's own kept parts: macros, pivot caches, external links, people (threaded comments)…
    if let (Some((orig, _)), Some(ot)) = (&kept, &orig_types) {
        let orels = read_rels(&orig.text("xl/_rels/workbook.xml.rels").unwrap_or_default());
        let mut rels = read_rels(&pkg.text("xl/_rels/workbook.xml.rels").unwrap_or_default());
        let mut map = HashMap::new();
        let mut macros = false;
        for (k, r) in orels.iter().enumerate() {
            if BOOK_RELS_OWN.iter().any(|t| r.kind.ends_with(t)) {
                continue;
            }
            let id = format!("rIdK{}", k + 1);
            map.insert(r.id.clone(), id.clone());
            if !r.external {
                copy_part(orig, ot, &mut pkg, &mut types, &resolve("xl/workbook.xml", &r.target), &mut copied);
            }
            macros |= r.kind.ends_with("/vbaProject");
            rels.push(Rel { id, kind: r.kind.clone(), target: r.target.clone(), external: r.external });
        }
        pkg.put("xl/_rels/workbook.xml.rels", write_rels(&rels).into_bytes());
        let owb = orig.text("xl/workbook.xml").unwrap_or_default();
        let (oroot, okids) = children(&owb);
        let (root, mut kids) = children(&pkg.text("xl/workbook.xml").unwrap_or_default());
        // Chart sheets (a chart as a whole sheet) stay as sheets.
        let chart_sheets: Vec<String> = okids
            .iter()
            .filter(|(n, _)| n == "sheets")
            .flat_map(|(_, raw)| children(raw).1)
            .filter(|(_, raw)| {
                let rid = raw.split(":id=\"").nth(1).and_then(|x| x.split('"').next()).unwrap_or("");
                orels.iter().any(|r| r.id == rid && r.kind.ends_with("/chartsheet"))
            })
            .map(|(_, raw)| remap_ids(&raw, &map))
            .collect();
        if !chart_sheets.is_empty()
            && let Some(k) = kids.iter().position(|(n, _)| n == "sheets")
        {
            let raw = kids[k].1.replace("</sheets>", &format!("{}</sheets>", chart_sheets.join("")));
            kids[k].1 = raw;
        }
        let root = with_namespaces(&root, &namespaces(&oroot));
        let mut ext_list = Vec::new();
        for (name, raw) in okids {
            if name == "extLst" {
                ext_list.extend(exts(&raw).into_iter().map(|(u, e)| (u, remap_ids(&e, &map))));
            } else if BOOK_KEPT.contains(&name.as_str()) && !kids.iter().any(|(n, _)| *n == name) {
                kids.push((name, remap_ids(&raw, &map)));
            }
        }
        pkg.put("xl/workbook.xml", assemble(&root, "workbook", kids, BOOK_ORDER, ext_list).into_bytes());
        if macros {
            types.set_override("xl/workbook.xml", CT_MAIN_MACROS);
        }
        // Custom document properties.
        let pkg_rels = read_rels(&orig.text("_rels/.rels").unwrap_or_default());
        let mut root_rels = read_rels(&pkg.text("_rels/.rels").unwrap_or_default());
        for r in pkg_rels {
            if r.kind.ends_with("/custom-properties") && !r.external {
                copy_part(orig, ot, &mut pkg, &mut types, &resolve("", &r.target), &mut copied);
                root_rels.push(Rel { id: "rIdK0".to_string(), ..r });
            }
        }
        pkg.put("_rels/.rels", write_rels(&root_rels).into_bytes());
    } else {
        types.set_override("xl/workbook.xml", CT_MAIN);
    }

    // Calc's extras.
    let extras = Extras {
        format: "xenify-calc-extras".to_string(),
        version: 1,
        // Linked CSV sheets: an Excel file keeps only their values.
        sheets: book
            .info
            .iter()
            .map(|(k, v)| {
                let mut v = v.clone();
                if v.kind == crate::book::SheetKind::LinkedCsv {
                    v.kind = crate::book::SheetKind::Normal;
                }
                (*k, v)
            })
            .filter(|(_, i)| *i != SheetInfo::default())
            .collect(),
        notes: book.notes.clone(),
        definitions: book.definitions.values().cloned().collect(),
        filters: book.filters.clone(),
        charts: book.charts.clone(),
        summaries: book.summaries.clone(),
        reports: book.reports.clone(),
        queries: book.queries.clone(),
        editors: book.editors.clone(),
        pictures: saved_pictures,
    };
    let has_extras = !extras.sheets.is_empty()
        || !extras.notes.is_empty()
        || !extras.definitions.is_empty()
        || !extras.filters.is_empty()
        || !extras.charts.is_empty()
        || !extras.summaries.is_empty()
        || !extras.reports.is_empty()
        || !extras.queries.is_empty()
        || !extras.editors.is_empty()
        || !extras.pictures.is_empty();
    if has_extras {
        pkg.put(EXTRAS_PART, serde_json::to_vec(&extras).map_err(err)?);
        types.ensure_default("json", "application/json");
        let mut root_rels = read_rels(&pkg.text("_rels/.rels").unwrap_or_default());
        root_rels.retain(|r| r.kind != EXTRAS_REL);
        root_rels.push(Rel {
            id: "rIdXenify".to_string(),
            kind: EXTRAS_REL.to_string(),
            target: EXTRAS_PART.to_string(),
            external: false,
        });
        pkg.put("_rels/.rels", write_rels(&root_rels).into_bytes());
    }
    pkg.put("[Content_Types].xml", types.write().into_bytes());
    pkg.write()
}

/// Cells with a list of choices (data validation kept from the Excel file):
/// [{ top, left, bottom, right, values }].
pub fn choices(book: &Book, sheet: u32) -> Vec<serde_json::Value> {
    let Some(k) = &book.kept else { return Vec::new() };
    let Some(part) = k.sheets.get(&sheet) else { return Vec::new() };
    let Ok(pkg) = Package::read(&k.original) else { return Vec::new() };
    let Some(xml) = pkg.text(part) else { return Vec::new() };
    let mut out = Vec::new();
    let mut r = Reader::from_str(&xml);
    let mut current: Option<(String, String)> = None; // (sqref, type)
    let mut in_formula = false;
    let mut formula = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()).as_str() {
                "dataValidation" => {
                    let a = attrs(&e);
                    current =
                        Some((a.get("sqref").cloned().unwrap_or_default(), a.get("type").cloned().unwrap_or_default()));
                    formula.clear();
                }
                "formula1" => in_formula = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_formula => formula.push_str(t.as_ref()),
            Ok(Event::GeneralRef(g)) if in_formula => {
                let name: &str = g.as_ref();
                formula.push_str(match name {
                    "quot" => "\"",
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "apos" => "'",
                    _ => "",
                });
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                "formula1" => in_formula = false,
                "dataValidation" => {
                    if let Some((sqref, kind)) = current.take()
                        && kind == "list"
                    {
                        let values = list_values(book, sheet, &formula);
                        for part in sqref.split_whitespace() {
                            let (a, b) = part.split_once(':').unwrap_or((part, part));
                            if let (Some((r1, c1)), Some((r2, c2))) =
                                (crate::tables::parse_cell_pub(a), crate::tables::parse_cell_pub(b))
                            {
                                out.push(serde_json::json!({ "top": r1, "left": c1, "bottom": r2, "right": c2, "values": values }));
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// A list's choices: "Yes,No" written out, or the values of a range.
fn list_values(book: &Book, sheet: u32, f: &str) -> Vec<String> {
    let f = f.trim().trim_start_matches('=');
    if let Some(inner) = f.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return inner.split(',').map(|v| v.trim().to_string()).collect();
    }
    let (sheet_name, range) = match f.rsplit_once('!') {
        Some((s, r)) => (Some(s.trim_matches('\'').replace("''", "'")), r),
        None => (None, f),
    };
    let id = match sheet_name {
        Some(n) => match book.model.workbook.worksheets.iter().find(|w| w.get_name().eq_ignore_ascii_case(&n)) {
            Some(w) => w.sheet_id,
            None => return Vec::new(),
        },
        None => sheet,
    };
    let Ok(i) = book.index(id) else { return Vec::new() };
    let (a, b) = range.split_once(':').unwrap_or((range, range));
    let (Some((r1, c1)), Some((r2, c2))) = (crate::tables::parse_cell_pub(a), crate::tables::parse_cell_pub(b)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for r in r1..=r2.min(r1 + 500) {
        for c in c1..=c2.min(c1 + 20) {
            let v = book.model.get_formatted_cell_value(i, r, c).unwrap_or_default();
            if !v.is_empty() && !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

/// Whether the book keeps macros (so it should be saved as .xlsm).
pub fn has_macros(book: &Book) -> bool {
    book.kept.as_ref().is_some_and(|k| {
        Package::read(&k.original).ok().is_some_and(|p| p.parts.iter().any(|(n, _)| n.ends_with("vbaProject.bin")))
    })
}

/// What the book keeps from its Excel file without editing it, in words.
pub fn kept_summary(book: &Book) -> Vec<String> {
    let Some(k) = &book.kept else { return Vec::new() };
    let Ok(p) = Package::read(&k.original) else { return Vec::new() };
    let mut out = Vec::new();
    let count = |pre: &str| p.parts.iter().filter(|(n, _)| n.starts_with(pre) && !n.contains("/_rels/")).count();
    let add = |out: &mut Vec<String>, n: usize, one: &str, many: &str| {
        if n == 1 {
            out.push(one.to_string());
        } else if n > 1 {
            out.push(format!("{n} {many}"));
        }
    };
    add(&mut out, count("xl/charts/chart"), "A chart", "charts");
    add(&mut out, count("xl/media/"), "A picture", "pictures");
    add(&mut out, count("xl/drawings/drawing"), "A drawing layer", "drawing layers");
    add(&mut out, count("xl/comments") + count("xl/threadedComments/"), "Comments", "comment parts");
    add(&mut out, count("xl/pivotTables/"), "A pivot table", "pivot tables");
    add(
        &mut out,
        count("xl/externalLinks/"),
        "A link to another file (not refreshed)",
        "links to other files (not refreshed)",
    );
    if p.parts.iter().any(|(n, _)| n.ends_with("vbaProject.bin")) {
        out.push("Macros (kept, never run)".to_string());
    }
    let sheets_with = |tag: &str| k.sheets.values().filter(|sp| p.text(sp).is_some_and(|x| x.contains(tag))).count();
    add(&mut out, sheets_with("<dataValidations"), "Choices for cells on 1 sheet", "sheets with choices for cells");
    add(&mut out, sheets_with("sparklineGroups"), "Sparklines on 1 sheet", "sheets with sparklines");
    out
}

#[cfg(test)]
mod hostile_tests {
    use super::*;

    fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in parts {
            z.start_file(*n, o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn a_squeezed_bomb_is_refused() {
        let file = zip_of(&[("xl/workbook.xml", &vec![b' '; 80 << 20])]);
        assert!(file.len() < 1 << 20);
        assert_eq!(open(&file, "bomb.xlsx").err().as_deref(), Some(crate::file::TOO_BIG));
        let calc = zip_of(&[("calc.json", &vec![b' '; 80 << 20])]);
        assert_eq!(crate::file::open(&calc).err().as_deref(), Some(crate::file::TOO_BIG));
    }

    #[test]
    fn deeply_nested_parts_are_refused_without_a_crash() {
        let deep = format!("<?xml version=\"1.0\"?>{}", "<a>".repeat(200_000));
        let file = zip_of(&[("xl/workbook.xml", deep.as_bytes())]);
        assert!(open(&file, "deep.xlsx").err().unwrap().contains("nest too deeply"));
        assert!(!too_deep(br#"<?xml version="1.0"?><!-- <a><a> --><a x="/>"><b/><c>t</c></a>"#));
    }
}
