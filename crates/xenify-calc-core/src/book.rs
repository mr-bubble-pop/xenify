//! The workbook: IronCalc's model holds the cells, styles and tables; Calc
//! adds what each sheet is (a plain sheet, a query, a table editor…),
//! which sheets are working sheets, column notes and definitions.

use ironcalc_base::Model;
use ironcalc_base::types::{Cell, SheetState, Style};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// What a sheet is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetKind {
    /// Cells, formulas, formats and tables.
    #[default]
    Normal,
    /// The read-only result of a query through a server data source.
    Query,
    /// Rows of one database table; changes are sent on Save.
    TableEditor,
    /// A Summarise result (groups and totals) of a table or sheet.
    Summary,
    /// A printable page of titles, notes, summaries and charts.
    Report,
    /// The values of a CSV file, kept up to date with it.
    LinkedCsv,
}

impl SheetKind {
    /// People can't type in these: their cells come from elsewhere.
    pub fn locked(self) -> bool {
        matches!(self, SheetKind::Query | SheetKind::Summary | SheetKind::LinkedCsv)
    }
}

/// Calc's own facts about a sheet, by IronCalc sheet id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetInfo {
    pub kind: SheetKind,
    /// Off the tab strip, seen on the All sheets page (hidden in xlsx).
    pub working: bool,
}

/// What a cell holds, exactly (not as shown).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CellData {
    #[serde(rename = "n")]
    Number(ironcalc_base::num::Num),
    #[serde(rename = "s")]
    Text(String),
    #[serde(rename = "b")]
    Bool(bool),
    /// An error typed as a value (`#N/A`).
    #[serde(rename = "e")]
    Error(String),
    /// A formula in English, with its `=`.
    #[serde(rename = "f")]
    Formula(String),
    /// An array formula entered over `w`×`h` cells.
    #[serde(rename = "a")]
    Array { f: String, w: i32, h: i32 },
    /// What someone typed, read the way typing is (numbers, dates, `=`…).
    #[serde(rename = "i")]
    Input(String),
}

/// A note about one column: what it means and how it's worked out. Asked
/// for by the AI before it guesses, and shown under About this column.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnNote {
    pub target: NoteTarget,
    /// What the column is ("Money taken, before refunds").
    pub meaning: String,
    /// Unit or currency ("GBP", "kg").
    pub unit: String,
    /// How it's worked out ("Price × Quantity, after discount").
    pub how: String,
    /// Includes VAT/sales tax: yes, no, or not known (None).
    pub with_tax: Option<bool>,
    /// Rows that identify something (an order number, a customer id).
    pub key: bool,
    /// Rows to leave out of totals ("Status = Cancelled").
    pub leave_out: String,
    /// Where it comes from ("Shop database, orders.total").
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "on")]
pub enum NoteTarget {
    /// A table's column, by names (they stay put as rows come and go).
    Table { table: String, column: String },
    /// A column of a sheet, by sheet id and column number.
    Column { sheet: u32, column: i32 },
}

impl Default for NoteTarget {
    fn default() -> Self {
        NoteTarget::Column { sheet: 1, column: 1 }
    }
}

/// A named measure everyone in the file uses the same way ("Profit").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Definition {
    pub name: String,
    /// The formula, with table columns: `=Sales[Revenue]-Sales[Costs]`.
    pub formula: String,
    /// In words, for people and the AI.
    pub about: String,
}

pub struct Book {
    pub model: Model<'static>,
    pub info: BTreeMap<u32, SheetInfo>,
    pub notes: Vec<ColumnNote>,
    pub definitions: BTreeMap<String, Definition>,
    /// Each table's filters, by table name.
    pub filters: BTreeMap<String, Vec<crate::edit::ColumnFilter>>,
    pub charts: Vec<crate::charts::Chart>,
    /// Pictures put on sheets in Calc (Insert › Picture).
    pub pictures: Vec<crate::pictures::Picture>,
    pub sparklines: Vec<crate::sparklines::Sparkline>,
    /// Each Summary sheet's summary, by sheet id.
    pub summaries: BTreeMap<u32, crate::summary::Def>,
    /// Each Report sheet's page, by sheet id.
    pub reports: BTreeMap<u32, crate::report::Def>,
    /// Each Query sheet's query, by sheet id.
    pub queries: BTreeMap<u32, crate::query::Def>,
    /// Each Table editor sheet's table, by sheet id.
    pub editors: BTreeMap<u32, crate::editor::Def>,
    /// Each linked CSV sheet's file, by sheet id.
    pub csv_links: BTreeMap<u32, crate::csvlink::Def>,
    /// The Excel file it came from, for the parts Calc keeps but can't edit.
    pub kept: Option<crate::xlsx::Kept>,
    /// Editing together: the ids this window may give new sheets and charts.
    pub lease: Option<(u32, u32)>,
}

impl Book {
    /// A new book with one sheet.
    pub fn new() -> Book {
        Book::from_model(Model::new_empty("Book", "en", "UTC", "en").expect("en and UTC exist"))
    }

    /// A book around a workbook IronCalc read (from an Excel file).
    pub fn from_model(model: Model<'static>) -> Book {
        let mut info = BTreeMap::new();
        for w in &model.workbook.worksheets {
            info.insert(w.sheet_id, SheetInfo::default());
        }
        Book {
            model,
            info,
            notes: Vec::new(),
            definitions: BTreeMap::new(),
            filters: BTreeMap::new(),
            charts: Vec::new(),
            pictures: Vec::new(),
            sparklines: Vec::new(),
            summaries: BTreeMap::new(),
            reports: BTreeMap::new(),
            queries: BTreeMap::new(),
            editors: BTreeMap::new(),
            csv_links: BTreeMap::new(),
            kept: None,
            lease: None,
        }
    }

    /// The position of sheet `id` (IronCalc works by position).
    pub fn index(&self, id: u32) -> Result<u32, String> {
        self.model
            .workbook
            .worksheets
            .iter()
            .position(|w| w.sheet_id == id)
            .map(|i| i as u32)
            .ok_or_else(|| format!("no sheet {id}"))
    }

    pub fn id_at(&self, index: u32) -> Option<u32> {
        self.model.workbook.worksheets.get(index as usize).map(|w| w.sheet_id)
    }

    pub fn sheet_name(&self, id: u32) -> Result<String, String> {
        let i = self.index(id)?;
        Ok(self.model.workbook.worksheets[i as usize].get_name())
    }

    pub fn info(&self, id: u32) -> SheetInfo {
        self.info.get(&id).cloned().unwrap_or_default()
    }

    /// Keeps IronCalc's sheet state (what xlsx files carry) in step with
    /// working sheets.
    pub(crate) fn sync_state(&mut self, id: u32) {
        let working = self.info(id).working;
        if let Ok(i) = self.index(id) {
            let w = &mut self.model.workbook.worksheets[i as usize];
            w.state = match (working, &w.state) {
                (true, SheetState::Visible) => SheetState::Hidden,
                (true, s) => s.clone(),
                (false, _) => SheetState::Visible,
            };
        }
    }

    /// A free sheet id.
    pub fn next_sheet_id(&self) -> u32 {
        match self.lease {
            Some((from, _)) => self.next_id(from),
            None => self.model.workbook.worksheets.iter().map(|w| w.sheet_id).max().unwrap_or(0) + 1,
        }
    }

    pub fn next_chart_id(&self) -> u32 {
        match self.lease {
            Some((from, _)) => self.next_id(from),
            None => self.charts.iter().map(|c| c.id).max().unwrap_or(0) + 1,
        }
    }

    pub fn next_picture_id(&self) -> u32 {
        match self.lease {
            Some((from, _)) => self.next_id(from),
            None => self.pictures.iter().map(|p| p.id).max().unwrap_or(0) + 1,
        }
    }

    /// Editing together: the first id from `from` no sheet, chart or picture has.
    pub fn next_id(&self, from: u32) -> u32 {
        let used: std::collections::HashSet<u32> = self
            .model
            .workbook
            .worksheets
            .iter()
            .map(|w| w.sheet_id)
            .chain(self.charts.iter().map(|c| c.id))
            .chain(self.pictures.iter().map(|p| p.id))
            .collect();
        (from..).find(|k| !used.contains(k)).unwrap_or(from)
    }

    /// What cell (`row`, `column`) of sheet `id` holds, if anything.
    pub fn cell_data(&self, id: u32, row: i32, column: i32) -> Option<CellData> {
        let i = self.index(id).ok()?;
        let w = &self.model.workbook.worksheets[i as usize];
        let cell = w.cell(row, column)?;
        Some(match cell.as_ref() {
            Cell::EmptyCell { .. } | Cell::SpillCell { .. } => return None,
            Cell::BooleanCell { v, .. } => CellData::Bool(*v),
            Cell::NumberCell { v, .. } => CellData::Number(*v),
            Cell::ErrorCell { ei, .. } => CellData::Error(ei.to_string()),
            Cell::SharedString { si, .. } => {
                CellData::Text(self.model.workbook.shared_strings.get(*si as usize).cloned().unwrap_or_default())
            }
            Cell::CellFormula { .. } => CellData::Formula(self.formula(i, row, column)?),
            Cell::ArrayFormula { r, kind, .. } => {
                let f = self.formula(i, row, column)?;
                if matches!(kind, ironcalc_base::types::ArrayKind::Dynamic) {
                    CellData::Formula(f)
                } else {
                    CellData::Array { f, w: r.0, h: r.1 }
                }
            }
        })
    }

    fn formula(&self, index: u32, row: i32, column: i32) -> Option<String> {
        self.model.get_cell_formula(index, row, column).ok().flatten()
    }

    /// Puts `data` in a cell (none: empties it, keeping its look).
    pub(crate) fn put(&mut self, id: u32, row: i32, column: i32, data: Option<&CellData>) -> Result<(), String> {
        let i = self.index(id)?;
        let m = &mut self.model;
        match data {
            None => {
                let area = ironcalc_base::expressions::types::Area { sheet: i, row, column, width: 1, height: 1 };
                m.range_clear_contents(&area)
            }
            Some(CellData::Number(v)) => m.update_cell_with_number(i, row, column, *v),
            Some(CellData::Text(s)) => m.update_cell_with_text(i, row, column, s),
            Some(CellData::Bool(b)) => m.update_cell_with_bool(i, row, column, *b),
            Some(CellData::Error(e)) => m.set_user_input(i, row, column, e.clone()),
            Some(CellData::Formula(f)) => m.update_cell_with_formula(i, row, column, f.clone()),
            Some(CellData::Array { f, w, h }) => m.set_user_array_formula(i, row, column, *w, *h, f),
            Some(CellData::Input(s)) => m.set_user_input(i, row, column, s.clone()),
        }
    }

    pub fn style(&self, id: u32, row: i32, column: i32) -> Style {
        self.index(id).ok().and_then(|i| self.model.get_style_for_cell(i, row, column).ok()).unwrap_or_default()
    }

    /// Every formula in the book, by (sheet id, row, column).
    pub(crate) fn formulas(&self) -> HashMap<(u32, i32, i32), String> {
        let mut out = HashMap::new();
        for (i, w) in self.model.workbook.worksheets.iter().enumerate() {
            for (row, column, cell) in w.sheet_data.cells() {
                if matches!(cell.as_ref(), Cell::CellFormula { .. } | Cell::ArrayFormula { .. })
                    && let Some(f) = self.formula(i as u32, row, column)
                {
                    out.insert((w.sheet_id, row, column), f);
                }
            }
        }
        out
    }

    /// The cells with something in them or a look of their own, in `rows`
    /// (or all rows) and `columns` (or all columns) of sheet `id`.
    pub(crate) fn cells_in(
        &self,
        id: u32,
        rows: Option<(i32, i32)>,
        columns: Option<(i32, i32)>,
    ) -> Vec<(i32, i32, Option<CellData>, Style)> {
        let Ok(i) = self.index(id) else { return Vec::new() };
        let w = &self.model.workbook.worksheets[i as usize];
        let inside = |v: i32, span: Option<(i32, i32)>| span.is_none_or(|(a, b)| v >= a && v <= b);
        let mut out: Vec<_> = w
            .sheet_data
            .cells()
            .filter(|(r, c, _)| inside(*r, rows) && inside(*c, columns))
            .map(|(r, c, _)| (r, c))
            .collect();
        out.sort();
        out.into_iter().map(|(r, c)| (r, c, self.cell_data(id, r, c), self.style(id, r, c))).collect()
    }
}

impl Default for Book {
    fn default() -> Self {
        Book::new()
    }
}
