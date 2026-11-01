//! Edits. Every change to a book is a list of [`Op`]s; applying one gives
//! back the ops that undo it, so undo and redo are the same thing, and the
//! same ops go to the server when editing together.
//!
//! Sheets are named by their id, which stays put when sheets move.
//! Deleting rows, columns or a sheet remembers what was there, and every
//! formula elsewhere that the deletion broke, so undo brings all of it back.

use crate::book::{Book, CellData, ColumnNote, Definition, NoteTarget, SheetInfo};
use crate::tables::TableDef;
use ironcalc_base::types::Style;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One cell's new content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellEdit {
    pub row: i32,
    pub col: i32,
    /// None empties the cell.
    pub data: Option<CellData>,
}

/// One cell's new look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleEdit {
    pub row: i32,
    pub col: i32,
    pub style: Style,
}

/// A row's or column's size (in IronCalc's pixels) and whether it's hidden.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineSize {
    pub at: i32,
    /// None: the sheet's normal size.
    pub size: Option<f64>,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Cells {
        sheet: u32,
        cells: Vec<CellEdit>,
    },
    Styles {
        sheet: u32,
        cells: Vec<StyleEdit>,
    },
    InsertRows {
        sheet: u32,
        at: i32,
        count: i32,
    },
    DeleteRows {
        sheet: u32,
        at: i32,
        count: i32,
    },
    InsertColumns {
        sheet: u32,
        at: i32,
        count: i32,
    },
    DeleteColumns {
        sheet: u32,
        at: i32,
        count: i32,
    },
    Rows {
        sheet: u32,
        rows: Vec<LineSize>,
    },
    Columns {
        sheet: u32,
        columns: Vec<LineSize>,
    },
    Freeze {
        sheet: u32,
        rows: i32,
        columns: i32,
    },
    /// The sheet's merged cells, all of them.
    Merged {
        sheet: u32,
        cells: Vec<ironcalc_base::types::MergedCell>,
    },
    AddSheet {
        sheet: u32,
        name: String,
        at: u32,
        #[serde(default)]
        info: SheetInfo,
    },
    RemoveSheet {
        sheet: u32,
    },
    RenameSheet {
        sheet: u32,
        name: String,
    },
    MoveSheet {
        sheet: u32,
        to: u32,
    },
    SheetInfo {
        sheet: u32,
        info: SheetInfo,
    },
    /// Adds, changes (none: removes) the table called `name`.
    Table {
        name: String,
        table: Option<TableDef>,
    },
    /// A Report sheet's page (none: takes it away).
    Report {
        sheet: u32,
        def: Option<crate::report::Def>,
    },
    /// A Summary sheet's summary (none: takes it away).
    Summary {
        sheet: u32,
        def: Option<crate::summary::Def>,
    },
    /// A Table editor sheet's table (none: takes it away).
    Editor {
        sheet: u32,
        def: Option<crate::editor::Def>,
    },
    /// A Query sheet's query (none: takes it away).
    Query {
        sheet: u32,
        def: Option<crate::query::Def>,
    },
    /// A linked CSV sheet's file (none: takes it away).
    CsvLink {
        sheet: u32,
        def: Option<crate::csvlink::Def>,
    },
    /// The sparkline in a cell (none: takes it away).
    Sparkline {
        sheet: u32,
        row: i32,
        col: i32,
        spark: Option<crate::sparklines::Sparkline>,
    },
    /// Adds, changes (none: removes) chart `id`.
    Chart {
        id: u32,
        chart: Option<crate::charts::Chart>,
    },
    /// Adds, changes (none: removes) picture `id`.
    Picture {
        id: u32,
        picture: Option<crate::pictures::Picture>,
    },
    /// A table's filters (rows are hidden by `Rows` ops with it).
    Filter {
        table: String,
        filters: Vec<crate::edit::ColumnFilter>,
    },
    Note {
        target: NoteTarget,
        note: Option<ColumnNote>,
    },
    Definition {
        name: String,
        definition: Option<Definition>,
    },
}

impl Op {
    /// Whether formulas elsewhere need working out again after it.
    fn changes_tables(&self) -> bool {
        matches!(self, Op::Table { .. })
    }
}

/// Applies `op`; returns the ops that undo it (in the order to apply them).
pub fn apply(book: &mut Book, op: &Op) -> Result<Vec<Op>, String> {
    let undo = match op {
        Op::Cells { sheet, cells } => {
            let mut old = Vec::with_capacity(cells.len());
            for c in cells {
                old.push(CellEdit { row: c.row, col: c.col, data: book.cell_data(*sheet, c.row, c.col) });
                book.put(*sheet, c.row, c.col, c.data.as_ref())?;
            }
            old.reverse();
            vec![Op::Cells { sheet: *sheet, cells: old }]
        }
        Op::Styles { sheet, cells } => {
            let i = book.index(*sheet)?;
            let mut old = Vec::with_capacity(cells.len());
            for c in cells {
                old.push(StyleEdit { row: c.row, col: c.col, style: book.style(*sheet, c.row, c.col) });
                book.model.set_cell_style(i, c.row, c.col, &c.style)?;
            }
            old.reverse();
            vec![Op::Styles { sheet: *sheet, cells: old }]
        }
        Op::InsertRows { sheet, at, count } => {
            let i = book.index(*sheet)?;
            book.model.insert_rows(i, *at, *count)?;
            shift_notes(book, *sheet, None);
            for c in &mut book.charts {
                c.shift(*sheet, *at, *count, true);
            }
            for p in &mut book.pictures {
                p.shift(*sheet, *at, *count, true);
            }
            book.sparklines = book.sparklines.iter().filter_map(|s| s.shift(*sheet, *at, *count, true)).collect();
            vec![Op::DeleteRows { sheet: *sheet, at: *at, count: *count }]
        }
        Op::InsertColumns { sheet, at, count } => {
            let i = book.index(*sheet)?;
            book.model.insert_columns(i, *at, *count)?;
            shift_notes(book, *sheet, Some((*at, *count)));
            for c in &mut book.charts {
                c.shift(*sheet, *at, *count, false);
            }
            for p in &mut book.pictures {
                p.shift(*sheet, *at, *count, false);
            }
            book.sparklines = book.sparklines.iter().filter_map(|s| s.shift(*sheet, *at, *count, false)).collect();
            vec![Op::DeleteColumns { sheet: *sheet, at: *at, count: *count }]
        }
        Op::DeleteRows { sheet, at, count } => delete_lines(book, *sheet, *at, *count, true)?,
        Op::DeleteColumns { sheet, at, count } => delete_lines(book, *sheet, *at, *count, false)?,
        Op::Rows { sheet, rows } => {
            let old = rows.iter().map(|r| row_size(book, *sheet, r.at)).collect();
            for r in rows {
                set_row_size(book, *sheet, r)?;
            }
            vec![Op::Rows { sheet: *sheet, rows: old }]
        }
        Op::Columns { sheet, columns } => {
            let old = columns.iter().map(|c| column_size(book, *sheet, c.at)).collect();
            for c in columns {
                set_column_size(book, *sheet, c)?;
            }
            vec![Op::Columns { sheet: *sheet, columns: old }]
        }
        Op::Freeze { sheet, rows, columns } => {
            let i = book.index(*sheet)?;
            let old = Op::Freeze {
                sheet: *sheet,
                rows: book.model.get_frozen_rows_count(i)?,
                columns: book.model.get_frozen_columns_count(i)?,
            };
            book.model.set_frozen_rows(i, *rows)?;
            book.model.set_frozen_columns(i, *columns)?;
            vec![old]
        }
        Op::Merged { sheet, cells } => {
            let i = book.index(*sheet)?;
            let w = &mut book.model.workbook.worksheets[i as usize];
            let old = std::mem::replace(&mut w.merged_cells, cells.clone());
            vec![Op::Merged { sheet: *sheet, cells: old }]
        }
        Op::AddSheet { sheet, name, at, info } => {
            // At most at the end (editing together, sheets may have gone meanwhile).
            let at = (*at).min(book.model.workbook.worksheets.len() as u32);
            book.model.insert_sheet(name, at, Some(*sheet))?;
            book.info.insert(*sheet, info.clone());
            book.sync_state(*sheet);
            vec![Op::RemoveSheet { sheet: *sheet }]
        }
        Op::RemoveSheet { sheet } => remove_sheet(book, *sheet)?,
        Op::RenameSheet { sheet, name } => {
            let i = book.index(*sheet)?;
            let old = book.sheet_name(*sheet)?;
            book.model.rename_sheet_by_index(i, name)?;
            vec![Op::RenameSheet { sheet: *sheet, name: old }]
        }
        Op::MoveSheet { sheet, to } => {
            let i = book.index(*sheet)?;
            let to = (*to).min(book.model.workbook.worksheets.len().saturating_sub(1) as u32);
            book.model.move_sheet(i, to)?;
            vec![Op::MoveSheet { sheet: *sheet, to: i }]
        }
        Op::SheetInfo { sheet, info } => {
            book.index(*sheet)?;
            let old = book.info.insert(*sheet, info.clone()).unwrap_or_default();
            book.sync_state(*sheet);
            vec![Op::SheetInfo { sheet: *sheet, info: old }]
        }
        Op::Table { name, table } => {
            let old = crate::tables::get(book, name);
            if let (Some(o), Some(t)) = (&old, table) {
                rename_table(book, o, t)?;
            }
            crate::tables::set(book, name, table.as_ref())?;
            vec![Op::Table { name: table.as_ref().map_or_else(|| name.clone(), |t| t.name.clone()), table: old }]
        }
        Op::Report { sheet, def } => {
            let old = match def {
                Some(d) => book.reports.insert(*sheet, d.clone()),
                None => book.reports.remove(sheet),
            };
            vec![Op::Report { sheet: *sheet, def: old }]
        }
        Op::Summary { sheet, def } => {
            let old = match def {
                Some(d) => book.summaries.insert(*sheet, d.clone()),
                None => book.summaries.remove(sheet),
            };
            vec![Op::Summary { sheet: *sheet, def: old }]
        }
        Op::Editor { sheet, def } => {
            let old = match def {
                Some(d) => book.editors.insert(*sheet, d.clone()),
                None => book.editors.remove(sheet),
            };
            vec![Op::Editor { sheet: *sheet, def: old }]
        }
        Op::Query { sheet, def } => {
            let old = match def {
                Some(d) => book.queries.insert(*sheet, d.clone()),
                None => book.queries.remove(sheet),
            };
            vec![Op::Query { sheet: *sheet, def: old }]
        }
        Op::CsvLink { sheet, def } => {
            let old = match def {
                Some(d) => book.csv_links.insert(*sheet, d.clone()),
                None => book.csv_links.remove(sheet),
            };
            vec![Op::CsvLink { sheet: *sheet, def: old }]
        }
        Op::Sparkline { sheet, row, col, spark } => {
            let at = book.sparklines.iter().position(|s| s.sheet == *sheet && s.row == *row && s.col == *col);
            let old = at.map(|k| book.sparklines.remove(k));
            if let Some(s) = spark {
                book.sparklines.push(crate::sparklines::Sparkline { sheet: *sheet, row: *row, col: *col, ..s.clone() });
            }
            vec![Op::Sparkline { sheet: *sheet, row: *row, col: *col, spark: old }]
        }
        Op::Picture { id, picture } => {
            let at = book.pictures.iter().position(|p| p.id == *id);
            let old = at.map(|k| book.pictures.remove(k));
            if let Some(p) = picture {
                book.pictures.push(crate::pictures::Picture { id: *id, ..p.clone() });
            }
            vec![Op::Picture { id: *id, picture: old }]
        }
        Op::Chart { id, chart } => {
            let at = book.charts.iter().position(|c| c.id == *id);
            let old = at.map(|k| book.charts.remove(k));
            if let Some(c) = chart {
                book.charts.push(crate::charts::Chart { id: *id, ..c.clone() });
            }
            vec![Op::Chart { id: *id, chart: old }]
        }
        Op::Filter { table, filters } => {
            let old = if filters.is_empty() {
                book.filters.remove(table)
            } else {
                book.filters.insert(table.clone(), filters.clone())
            };
            if let Some(t) = book.model.workbook.tables.values_mut().find(|t| t.name.eq_ignore_ascii_case(table)) {
                t.has_filters = !filters.is_empty();
            }
            vec![Op::Filter { table: table.clone(), filters: old.unwrap_or_default() }]
        }
        Op::Note { target, note } => {
            let at = book.notes.iter().position(|n| &n.target == target);
            let old = at.map(|k| book.notes.remove(k));
            if let Some(n) = note {
                let mut n = n.clone();
                n.target = target.clone();
                book.notes.push(n);
            }
            vec![Op::Note { target: target.clone(), note: old }]
        }
        Op::Definition { name, definition } => {
            let old = match definition {
                Some(d) => book.definitions.insert(name.clone(), Definition { name: name.clone(), ..d.clone() }),
                None => book.definitions.remove(name),
            };
            vec![Op::Definition { name: name.clone(), definition: old }]
        }
    };
    if op.changes_tables() {
        book.model.refresh_tables();
    }
    Ok(undo)
}

/// Applies `ops` in order; returns the ops that undo all of them. Stops at
/// the first that can't be applied, after undoing the ones before it.
pub fn apply_all(book: &mut Book, ops: &[Op]) -> Result<Vec<Op>, String> {
    let mut undo: Vec<Vec<Op>> = Vec::new();
    for op in ops {
        match apply(book, op) {
            Ok(u) => undo.push(u),
            Err(e) => {
                for u in undo.iter().rev() {
                    for o in u {
                        let _ = apply(book, o);
                    }
                }
                book.model.evaluate();
                return Err(e);
            }
        }
    }
    book.model.evaluate();
    // Summary sheets follow what they summarise; reports their numbers.
    crate::summary::refresh_all(book);
    crate::report::refresh_all(book);
    Ok(undo.into_iter().rev().flatten().collect())
}

/// A table or its columns getting new names: formulas and column notes
/// follow.
fn rename_table(book: &mut Book, old: &TableDef, new: &TableDef) -> Result<(), String> {
    if old.name != new.name {
        if crate::tables::get(book, &new.name).is_some() && !old.name.eq_ignore_ascii_case(&new.name) {
            return Err(format!("There's already a table called {}", new.name));
        }
        book.model.rename_table(&old.name, &new.name)?;
        follow_table(book, &old.name, &new.name);
    }
    if old.columns.len() == new.columns.len() {
        for (a, b) in old.columns.iter().zip(&new.columns) {
            if a.name != b.name {
                book.model.rename_table_column(&new.name, &a.name, &b.name)?;
                follow_column(book, &new.name, &a.name, &b.name);
            }
        }
    }
    Ok(())
}

/// What names table `old` follows it to `new`: column notes, filters,
/// summaries, charts, reports, query and table editor sheets.
fn follow_table(book: &mut Book, old: &str, new: &str) {
    let rename = |t: &mut String| {
        if t.eq_ignore_ascii_case(old) {
            *t = new.to_string();
        }
    };
    for n in &mut book.notes {
        if let NoteTarget::Table { table, .. } = &mut n.target {
            rename(table);
        }
    }
    if let Some(key) = book.filters.keys().find(|k| k.eq_ignore_ascii_case(old)).cloned()
        && let Some(f) = book.filters.remove(&key)
    {
        book.filters.insert(new.to_string(), f);
    }
    for d in book.summaries.values_mut() {
        d.from.table.iter_mut().for_each(rename);
    }
    for c in &mut book.charts {
        c.source.table.iter_mut().for_each(rename);
    }
    for d in book.reports.values_mut() {
        for b in &mut d.blocks {
            if let crate::report::Block::Cells { table: Some(t), .. } = b {
                rename(t);
            }
        }
    }
    for d in book.queries.values_mut() {
        rename(&mut d.table);
    }
    for d in book.editors.values_mut() {
        rename(&mut d.calc_table);
    }
}

/// What names column `old` of table `table` follows it to `new`: column
/// notes, filters, summaries and charts of the table.
fn follow_column(book: &mut Book, table: &str, old: &str, new: &str) {
    let rename = |c: &mut String| {
        if c.eq_ignore_ascii_case(old) {
            *c = new.to_string();
        }
    };
    for n in &mut book.notes {
        if let NoteTarget::Table { table: t, column } = &mut n.target
            && t.eq_ignore_ascii_case(table)
        {
            rename(column);
        }
    }
    for (t, filters) in &mut book.filters {
        if t.eq_ignore_ascii_case(table) {
            filters.iter_mut().for_each(|f| rename(&mut f.column));
        }
    }
    let ours = |t: &Option<String>| t.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(table));
    for d in book.summaries.values_mut().filter(|d| ours(&d.from.table)) {
        d.group.iter_mut().for_each(rename);
        d.show.iter_mut().for_each(|s| rename(&mut s.column));
        d.only.iter_mut().for_each(|c| rename(&mut c.column));
    }
    for c in book.charts.iter_mut().filter(|c| ours(&c.source.table)) {
        let s = &mut c.source;
        s.x.iter_mut().for_each(rename);
        s.series.iter_mut().for_each(rename);
        s.split.iter_mut().for_each(rename);
        s.values.iter_mut().for_each(|v| rename(&mut v.column));
        s.only.iter_mut().for_each(|o| rename(&mut o.column));
    }
}

/// A row's size as set (its height even when it's hidden: IronCalc then gives 0).
pub(crate) fn row_size(book: &Book, sheet: u32, row: i32) -> LineSize {
    let w = book.index(sheet).ok().map(|i| &book.model.workbook.worksheets[i as usize]);
    match w.and_then(|w| w.rows.iter().find(|r| r.r == row)) {
        Some(r) => line_of(r),
        None => LineSize { at: row, size: None, hidden: false },
    }
}

/// An IronCalc row's size.
pub(crate) fn line_of(r: &ironcalc_base::types::Row) -> LineSize {
    LineSize { at: r.r, size: r.custom_height.then_some(r.height * ironcalc_base::ROW_HEIGHT_FACTOR), hidden: r.hidden }
}

fn set_row_size(book: &mut Book, sheet: u32, r: &LineSize) -> Result<(), String> {
    let i = book.index(sheet)?;
    match r.size {
        Some(h) => book.model.set_row_height(i, r.at, h)?,
        None => {
            let w = &mut book.model.workbook.worksheets[i as usize];
            if let Some(row) = w.rows.iter_mut().find(|x| x.r == r.at) {
                row.custom_height = false;
                // IronCalc's normal height, in its pixels
                row.height = 25.0 / ironcalc_base::ROW_HEIGHT_FACTOR;
            }
        }
    }
    book.model.set_row_hidden(i, r.at, r.hidden)
}

pub(crate) fn column_size(book: &Book, sheet: u32, column: i32) -> LineSize {
    let w = book.index(sheet).ok().map(|i| &book.model.workbook.worksheets[i as usize]);
    match w.and_then(|w| w.cols.iter().find(|c| c.min <= column && column <= c.max)) {
        Some(c) => LineSize {
            at: column,
            size: c.custom_width.then(|| w.and_then(|w| w.get_actual_column_width(column).ok())).flatten(),
            hidden: c.hidden,
        },
        None => LineSize { at: column, size: None, hidden: false },
    }
}

fn set_column_size(book: &mut Book, sheet: u32, c: &LineSize) -> Result<(), String> {
    let i = book.index(sheet)?;
    // IronCalc's normal width, in its pixels
    book.model.set_column_width(i, c.at, c.size.unwrap_or(90.0))?;
    if c.size.is_none() {
        let w = &mut book.model.workbook.worksheets[i as usize];
        for col in w.cols.iter_mut().filter(|x| x.min <= c.at && c.at <= x.max) {
            col.custom_width = false;
        }
    }
    book.model.set_column_hidden(i, c.at, c.hidden)
}

/// Column notes keep to their column when columns are inserted before it.
fn shift_notes(book: &mut Book, sheet: u32, inserted: Option<(i32, i32)>) {
    let Some((at, count)) = inserted else { return };
    for n in &mut book.notes {
        if let NoteTarget::Column { sheet: s, column } = &mut n.target
            && *s == sheet
            && *column >= at
        {
            *column += count;
        }
    }
}

/// Formulas whose text changed, at their place before the change, given
/// where each place went (`moved`: none when it was deleted).
fn broken(
    before: &HashMap<(u32, i32, i32), String>,
    after: &HashMap<(u32, i32, i32), String>,
    moved: impl Fn(u32, i32, i32) -> Option<(i32, i32)>,
) -> HashMap<u32, Vec<CellEdit>> {
    let mut out: HashMap<u32, Vec<CellEdit>> = HashMap::new();
    for ((sheet, row, col), f) in before {
        let Some((r, c)) = moved(*sheet, *row, *col) else { continue };
        if after.get(&(*sheet, r, c)) != Some(f) {
            out.entry(*sheet).or_default().push(CellEdit {
                row: *row,
                col: *col,
                data: Some(CellData::Formula(f.clone())),
            });
        }
    }
    for cells in out.values_mut() {
        cells.sort_by_key(|c| (c.row, c.col));
    }
    out
}

fn delete_lines(book: &mut Book, sheet: u32, at: i32, count: i32, rows: bool) -> Result<Vec<Op>, String> {
    let i = book.index(sheet)?;
    let span = Some((at, at + count - 1));
    let (cells, sizes) = if rows {
        (book.cells_in(sheet, span, None), (at..at + count).map(|r| row_size(book, sheet, r)).collect::<Vec<_>>())
    } else {
        (book.cells_in(sheet, None, span), (at..at + count).map(|c| column_size(book, sheet, c)).collect::<Vec<_>>())
    };
    let tables = crate::tables::all(book);
    let formulas = book.formulas();
    let notes = book.notes.clone();
    let charts = book.charts.clone();
    let pictures = book.pictures.clone();
    let sparks = book.sparklines.clone();
    let merged = book.model.workbook.worksheets[i as usize].merged_cells.clone();
    for c in &mut book.charts {
        c.shift(sheet, at, -count, rows);
    }
    for p in &mut book.pictures {
        p.shift(sheet, at, -count, rows);
    }
    book.sparklines = book.sparklines.iter().filter_map(|s| s.shift(sheet, at, -count, rows)).collect();
    if rows {
        book.model.delete_rows(i, at, count)?;
    } else {
        book.model.delete_columns(i, at, count)?;
        book.notes.retain(|n| !matches!(n.target, NoteTarget::Column { sheet: s, column } if s == sheet && column >= at && column < at + count));
        for n in &mut book.notes {
            if let NoteTarget::Column { sheet: s, column } = &mut n.target
                && *s == sheet
                && *column >= at + count
            {
                *column -= count;
            }
        }
    }
    let after = book.formulas();
    let fixes = broken(&formulas, &after, |s, r, c| {
        let v = if rows { r } else { c };
        if s != sheet || v < at {
            Some((r, c))
        } else if v < at + count {
            None
        } else if rows {
            Some((r - count, c))
        } else {
            Some((r, c - count))
        }
    });

    let mut undo =
        vec![if rows { Op::InsertRows { sheet, at, count } } else { Op::InsertColumns { sheet, at, count } }];
    undo.extend(restore_tables(book, &tables));
    undo.push(if rows { Op::Rows { sheet, rows: sizes } } else { Op::Columns { sheet, columns: sizes } });
    undo.extend(restore_cells(sheet, cells));
    for (s, cells) in fixes {
        undo.push(Op::Cells { sheet: s, cells });
    }
    if book.notes != notes {
        undo.extend(restore_notes(book, &notes));
    }
    undo.extend(restore_charts(book, &charts));
    undo.extend(restore_pictures(book, &pictures));
    // Last: the merges and sparklines as they were (inserting the rows back grew or moved them).
    undo.push(Op::Merged { sheet, cells: merged });
    undo.extend(restore_sparks(book, &sparks));
    Ok(undo)
}

/// Ops that put the sparklines back as they were in `old`.
fn restore_sparks(book: &Book, old: &[crate::sparklines::Sparkline]) -> Vec<Op> {
    let mut ops = Vec::new();
    for s in &book.sparklines {
        if !old.contains(s) {
            ops.push(Op::Sparkline { sheet: s.sheet, row: s.row, col: s.col, spark: None });
        }
    }
    for s in old {
        if !book.sparklines.contains(s) {
            ops.push(Op::Sparkline { sheet: s.sheet, row: s.row, col: s.col, spark: Some(s.clone()) });
        }
    }
    ops
}

/// Ops that put the pictures back as they were in `old`.
fn restore_pictures(book: &Book, old: &[crate::pictures::Picture]) -> Vec<Op> {
    old.iter()
        .filter(|p| !book.pictures.contains(p))
        .map(|p| Op::Picture { id: p.id, picture: Some(p.clone()) })
        .collect()
}

/// Ops that put the charts back as they were in `old`.
fn restore_charts(book: &Book, old: &[crate::charts::Chart]) -> Vec<Op> {
    let mut ops = Vec::new();
    for c in old {
        if !book.charts.contains(c) {
            ops.push(Op::Chart { id: c.id, chart: Some(c.clone()) });
        }
    }
    ops
}

fn remove_sheet(book: &mut Book, sheet: u32) -> Result<Vec<Op>, String> {
    let i = book.index(sheet)?;
    let name = book.sheet_name(sheet)?;
    let info = book.info(sheet);
    let cells = book.cells_in(sheet, None, None);
    let w = &book.model.workbook.worksheets[i as usize];
    let rows: Vec<LineSize> =
        w.rows.iter().map(|r| r.r).collect::<Vec<_>>().into_iter().map(|r| row_size(book, sheet, r)).collect();
    let mut column_numbers = Vec::new();
    for c in &w.cols {
        column_numbers.extend(c.min..=c.max.min(c.min + 1000));
    }
    let columns: Vec<LineSize> = column_numbers.into_iter().map(|c| column_size(book, sheet, c)).collect();
    let freeze = Op::Freeze {
        sheet,
        rows: book.model.get_frozen_rows_count(i)?,
        columns: book.model.get_frozen_columns_count(i)?,
    };
    let merged = book.model.workbook.worksheets[i as usize].merged_cells.clone();
    let tables = crate::tables::all(book);
    let formulas = book.formulas();
    let notes = book.notes.clone();
    let charts = book.charts.clone();
    let pictures = book.pictures.clone();
    let sparks = book.sparklines.clone();

    book.model.delete_sheet(i)?;
    book.charts.retain(|c| c.sheet != sheet && c.source.sheet != sheet);
    book.pictures.retain(|p| p.sheet != sheet);
    book.sparklines.retain(|s| s.sheet != sheet && s.source_sheet != sheet);
    book.info.remove(&sheet);
    let summary = book.summaries.remove(&sheet);
    let query = book.queries.remove(&sheet);
    let editor = book.editors.remove(&sheet);
    let csv_link = book.csv_links.remove(&sheet);
    let report = book.reports.remove(&sheet);
    book.notes.retain(|n| !matches!(n.target, NoteTarget::Column { sheet: s, .. } if s == sheet));

    let after = book.formulas();
    let fixes = broken(&formulas, &after, |s, r, c| (s != sheet).then_some((r, c)));
    let mut undo = vec![Op::AddSheet { sheet, name, at: i, info }];
    if let Some(d) = summary {
        undo.push(Op::Summary { sheet, def: Some(d) });
    }
    if let Some(d) = query {
        undo.push(Op::Query { sheet, def: Some(d) });
    }
    if let Some(d) = editor {
        undo.push(Op::Editor { sheet, def: Some(d) });
    }
    if let Some(d) = report {
        undo.push(Op::Report { sheet, def: Some(d) });
    }
    if let Some(d) = csv_link {
        undo.push(Op::CsvLink { sheet, def: Some(d) });
    }
    undo.extend(restore_tables(book, &tables));
    if !rows.is_empty() {
        undo.push(Op::Rows { sheet, rows });
    }
    if !columns.is_empty() {
        undo.push(Op::Columns { sheet, columns });
    }
    undo.push(freeze);
    undo.extend(restore_cells(sheet, cells));
    if !merged.is_empty() {
        undo.push(Op::Merged { sheet, cells: merged });
    }
    undo.extend(restore_sparks(book, &sparks));
    for (s, cells) in fixes {
        undo.push(Op::Cells { sheet: s, cells });
    }
    if book.notes != notes {
        undo.extend(restore_notes(book, &notes));
    }
    undo.extend(restore_charts(book, &charts));
    undo.extend(restore_pictures(book, &pictures));
    Ok(undo)
}

/// Ops that put the tables back as they were in `old`.
fn restore_tables(book: &Book, old: &[TableDef]) -> Vec<Op> {
    let now = crate::tables::all(book);
    let mut ops = Vec::new();
    for t in old {
        if !now.contains(t) {
            ops.push(Op::Table { name: t.name.clone(), table: Some(t.clone()) });
        }
    }
    ops
}

fn restore_notes(book: &Book, old: &[ColumnNote]) -> Vec<Op> {
    let mut ops = Vec::new();
    for n in &book.notes {
        if !old.iter().any(|o| o.target == n.target) {
            ops.push(Op::Note { target: n.target.clone(), note: None });
        }
    }
    for n in old {
        if !book.notes.contains(n) {
            ops.push(Op::Note { target: n.target.clone(), note: Some(n.clone()) });
        }
    }
    ops
}

fn restore_cells(sheet: u32, cells: Vec<(i32, i32, Option<CellData>, Style)>) -> Vec<Op> {
    let default = Style::default();
    let styles: Vec<StyleEdit> = cells
        .iter()
        .filter(|c| c.3 != default)
        .map(|(row, col, _, style)| StyleEdit { row: *row, col: *col, style: style.clone() })
        .collect();
    let data: Vec<CellEdit> =
        cells.into_iter().filter(|c| c.2.is_some()).map(|(row, col, data, _)| CellEdit { row, col, data }).collect();
    let mut ops = Vec::new();
    if !styles.is_empty() {
        ops.push(Op::Styles { sheet, cells: styles });
    }
    if !data.is_empty() {
        ops.push(Op::Cells { sheet, cells: data });
    }
    ops
}
