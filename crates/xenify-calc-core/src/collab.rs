//! Editing a book together: Calc's edits adjusted to come after someone
//! else's made at the same time (xenify-collab's `Transform`).
//!
//! - Rows or columns inserted or deleted move every cell, size, merge,
//!   table, sparkline and chart after them, and the references in formulas
//!   typed in the other window (`=A5` becomes `=A6`); edits to rows,
//!   columns or sheets the other window deleted are dropped.
//! - Two deletes that overlap delete each row once; a delete with rows
//!   inserted inside it goes round them.
//! - Two windows setting the same thing (a cell, a sheet's name, a table):
//!   the later one wins. Two new sheets with one name: the later is renamed.
//!
//! Whatever slips through still ends the same everywhere: each window keeps
//! the server's copy beside its own and takes it when they differ (remote.rs).

use crate::book::{CellData, NoteTarget};
use crate::ops::{CellEdit, LineSize, Op, StyleEdit};
use ironcalc_base::expressions::lexer::{Lexer, LexerMode};
use ironcalc_base::expressions::token::TokenType;
use ironcalc_base::expressions::utils::number_to_column;
use serde::{Deserialize, Serialize};
use xenify_collab::Transform;

const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

/// An edit as it goes to the others: the op, and the name its sheet had
/// (formulas name other sheets by name).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub op: Op,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// Rows or columns inserted (`delta` > 0) or deleted (`delta` < 0) at `at`
/// on sheet `sheet` (named `name`).
#[derive(Clone, Copy, Debug)]
struct Shift<'a> {
    sheet: u32,
    name: &'a str,
    rows: bool,
    at: i32,
    delta: i32,
}

impl Shift<'_> {
    /// Where line `p` goes; none when it was deleted.
    fn line(&self, p: i32) -> Option<i32> {
        if self.delta > 0 {
            Some(if p >= self.at { p + self.delta } else { p })
        } else {
            let end = self.at - self.delta;
            if p < self.at {
                Some(p)
            } else if p < end {
                None
            } else {
                Some(p + self.delta)
            }
        }
    }

    /// A span of lines after the shift (inserting inside it makes it
    /// longer; deleting part of it shorter); none when all of it went.
    fn span(&self, lo: i32, hi: i32) -> Option<(i32, i32)> {
        if self.delta > 0 {
            if self.at <= lo {
                Some((lo + self.delta, hi + self.delta))
            } else if self.at <= hi {
                Some((lo, hi + self.delta))
            } else {
                Some((lo, hi))
            }
        } else {
            let (d1, d2) = (self.at, self.at - self.delta - 1);
            let gone = |p: i32| p >= d1 && p <= d2;
            let first = (lo..=hi).find(|p| !gone(*p))?;
            let last = (lo..=hi).rev().find(|p| !gone(*p))?;
            Some((self.line(first)?, self.line(last)?))
        }
    }

    /// (row, col) after the shift, for a cell on `sheet`.
    fn cell(&self, sheet: u32, row: i32, col: i32) -> Option<(i32, i32)> {
        if sheet != self.sheet {
            return Some((row, col));
        }
        if self.rows { Some((self.line(row)?, col)) } else { Some((row, self.line(col)?)) }
    }
}

/// `formula` (typed on sheet `own`) with the references the shift moves
/// moved: on its own sheet when unqualified, or naming the shifted sheet.
fn shift_formula(formula: &str, own: &str, s: &Shift) -> String {
    let Some(body) = formula.strip_prefix('=') else { return formula.to_string() };
    let locale = ironcalc_base::locale::get_default_locale();
    let language = ironcalc_base::language::get_default_language();
    let mut lexer = Lexer::new(body, LexerMode::A1, locale, language);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut last = 0usize;
    let ours = |sheet: &Option<String>| match sheet {
        None => own.eq_ignore_ascii_case(s.name),
        Some(n) => n.eq_ignore_ascii_case(s.name),
    };
    let reference = |row: i32, col: i32, abs_r: bool, abs_c: bool| -> String {
        format!(
            "{}{}{}{}",
            if abs_c { "$" } else { "" },
            number_to_column(col).unwrap_or_default(),
            if abs_r { "$" } else { "" },
            row
        )
    };
    loop {
        let start = lexer.get_position() as usize;
        let tok = lexer.next_token();
        let end = lexer.get_position() as usize;
        // The token itself starts after any spaces.
        let tstart = (start..end).find(|k| !chars[*k].is_whitespace()).unwrap_or(start);
        let replace = match &tok {
            TokenType::EOF | TokenType::Illegal(_) => break,
            TokenType::Reference { sheet, row, column, absolute_column, absolute_row } if ours(sheet) => {
                let moved =
                    if s.rows { s.line(*row).map(|r| (r, *column)) } else { s.line(*column).map(|c| (*row, c)) };
                let text = match moved {
                    Some((r, c)) => reference(r, c, *absolute_row, *absolute_column),
                    None => "#REF!".to_string(),
                };
                Some(match sheet {
                    Some(_) => format!("{}!{text}", prefix(&chars[tstart..end])),
                    None => text,
                })
            }
            TokenType::Range { sheet, left, right } if ours(sheet) => {
                // Whole columns (A:A) don't move with rows, nor whole rows with columns.
                let whole = if s.rows {
                    left.row == 1 && right.row == LAST_ROW
                } else {
                    left.column == 1 && right.column == LAST_COLUMN
                };
                if whole {
                    None
                } else {
                    let span = if s.rows { s.span(left.row, right.row) } else { s.span(left.column, right.column) };
                    let text = match span {
                        Some((a, b)) => {
                            let (l, r) = if s.rows {
                                ((a, left.column), (b, right.column))
                            } else {
                                ((left.row, a), (right.row, b))
                            };
                            // Written as they were: whole columns (B:D) and whole rows (3:5) stay so.
                            let written: String = chars[tstart..end].iter().collect();
                            let cells = written.rsplit('!').next().unwrap_or_default();
                            let dollar = |abs: bool| if abs { "$" } else { "" };
                            if !cells.contains(|c: char| c.is_ascii_digit()) {
                                let col = |c: i32, abs: bool| {
                                    format!("{}{}", dollar(abs), number_to_column(c).unwrap_or_default())
                                };
                                format!("{}:{}", col(l.1, left.absolute_column), col(r.1, right.absolute_column))
                            } else if !cells.contains(|c: char| c.is_ascii_alphabetic()) {
                                format!("{}{}:{}{}", dollar(left.absolute_row), l.0, dollar(right.absolute_row), r.0)
                            } else {
                                format!(
                                    "{}:{}",
                                    reference(l.0, l.1, left.absolute_row, left.absolute_column),
                                    reference(r.0, r.1, right.absolute_row, right.absolute_column)
                                )
                            }
                        }
                        None => "#REF!".to_string(),
                    };
                    Some(match sheet {
                        Some(_) => format!("{}!{text}", prefix(&chars[tstart..end])),
                        None => text,
                    })
                }
            }
            _ => None,
        };
        if let Some(text) = replace {
            out.extend(&chars[last..tstart]);
            out.push_str(&text);
            last = end;
        }
        if end <= start {
            break;
        }
    }
    out.extend(&chars[last.min(chars.len())..]);
    format!("={out}")
}

/// The sheet part of a reference as written (`'My sheet'` or `Data`).
fn prefix(token: &[char]) -> String {
    let s: String = token.iter().collect();
    match s.rfind('!') {
        Some(k) => s[..k].to_string(),
        None => String::new(),
    }
}

fn shift_data(data: &CellData, own: &str, s: &Shift) -> CellData {
    match data {
        CellData::Formula(f) => CellData::Formula(shift_formula(f, own, s)),
        CellData::Input(t) if t.starts_with('=') => CellData::Input(shift_formula(t, own, s)),
        other => other.clone(),
    }
}

fn lines(list: &[LineSize], s: &Shift) -> Vec<LineSize> {
    list.iter().filter_map(|l| s.line(l.at).map(|at| LineSize { at, ..l.clone() })).collect()
}

/// `a` (on sheet named `name`) adjusted to come after rows or columns moved.
fn shift_op(a: &Op, name: &str, s: &Shift, later: bool) -> Vec<Op> {
    let same = |sheet: u32| sheet == s.sheet;
    match a {
        Op::Cells { sheet, cells } => {
            let cells: Vec<CellEdit> = cells
                .iter()
                .filter_map(|c| {
                    s.cell(*sheet, c.row, c.col).map(|(row, col)| CellEdit {
                        row,
                        col,
                        data: c.data.as_ref().map(|d| shift_data(d, name, s)),
                    })
                })
                .collect();
            if cells.is_empty() { vec![] } else { vec![Op::Cells { sheet: *sheet, cells }] }
        }
        Op::Styles { sheet, cells } => {
            let cells: Vec<StyleEdit> = cells
                .iter()
                .filter_map(|c| {
                    s.cell(*sheet, c.row, c.col).map(|(row, col)| StyleEdit { row, col, style: c.style.clone() })
                })
                .collect();
            if cells.is_empty() { vec![] } else { vec![Op::Styles { sheet: *sheet, cells }] }
        }
        Op::Rows { sheet, rows } if same(*sheet) && s.rows => vec![Op::Rows { sheet: *sheet, rows: lines(rows, s) }],
        Op::Columns { sheet, columns } if same(*sheet) && !s.rows => {
            vec![Op::Columns { sheet: *sheet, columns: lines(columns, s) }]
        }
        Op::InsertRows { sheet, at, count } if same(*sheet) && s.rows => {
            vec![Op::InsertRows { sheet: *sheet, at: insert_at(*at, s, later), count: *count }]
        }
        Op::InsertColumns { sheet, at, count } if same(*sheet) && !s.rows => {
            vec![Op::InsertColumns { sheet: *sheet, at: insert_at(*at, s, later), count: *count }]
        }
        Op::DeleteRows { sheet, at, count } if same(*sheet) && s.rows => delete_after(*at, *count, s)
            .into_iter()
            .map(|(at, count)| Op::DeleteRows { sheet: *sheet, at, count })
            .collect(),
        Op::DeleteColumns { sheet, at, count } if same(*sheet) && !s.rows => delete_after(*at, *count, s)
            .into_iter()
            .map(|(at, count)| Op::DeleteColumns { sheet: *sheet, at, count })
            .collect(),
        Op::Merged { sheet, cells } if same(*sheet) => {
            let cells = cells
                .iter()
                .filter_map(|m| {
                    let (lo, hi) =
                        if s.rows { (m.row, m.row + m.height - 1) } else { (m.column, m.column + m.width - 1) };
                    let (a, b) = s.span(lo, hi)?;
                    let mut m = *m;
                    if s.rows {
                        m.row = a;
                        m.height = b - a + 1;
                    } else {
                        m.column = a;
                        m.width = b - a + 1;
                    }
                    (m.width > 1 || m.height > 1).then_some(m)
                })
                .collect();
            vec![Op::Merged { sheet: *sheet, cells }]
        }
        // As IronCalc moves the tables itself (patches/ironcalc_base/src/xenify_tables.rs).
        Op::Table { name: tname, table: Some(t) } if same(t.sheet) => {
            // Deleting the header row takes the table away.
            if s.rows && s.delta < 0 && t.header && s.line(t.top).is_none() {
                return vec![];
            }
            let span = if s.rows { s.span(t.top, t.bottom) } else { s.span(t.left, t.right) };
            match span {
                Some((lo, hi)) => {
                    let mut t = t.clone();
                    if s.rows {
                        t.top = lo;
                        // At least one row of data stays.
                        t.bottom = hi.max(lo + t.header as i32 + t.totals as i32);
                    } else if s.delta > 0 {
                        // Columns inserted inside it are new columns of it, named as IronCalc names them.
                        if s.at > t.left && s.at <= t.right {
                            let index = (s.at - t.left) as usize;
                            for k in 0..s.delta as usize {
                                let mut n = t.columns.len() + 1;
                                while t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&format!("Column{n}"))) {
                                    n += 1;
                                }
                                let col = crate::tables::TableCol { name: format!("Column{n}"), total: None };
                                t.columns.insert((index + k).min(t.columns.len()), col);
                            }
                        }
                        t.left = lo;
                        t.right = hi;
                    } else {
                        // Columns of the table that were deleted go from its list.
                        let left = t.left;
                        let mut k = 0;
                        t.columns.retain(|_| {
                            k += 1;
                            s.line(left + k - 1).is_some()
                        });
                        t.left = lo;
                        t.right = hi;
                    }
                    vec![Op::Table { name: tname.clone(), table: Some(t) }]
                }
                None => vec![],
            }
        }
        Op::Sparkline { sheet, row, col, spark } => {
            let Some((r, c)) = s.cell(*sheet, *row, *col) else { return vec![] };
            let spark = spark.as_ref().map(|sp| sp.shift(s.sheet, s.at, s.delta, s.rows).unwrap_or_else(|| sp.clone()));
            vec![Op::Sparkline { sheet: *sheet, row: r, col: c, spark }]
        }
        Op::Chart { id, chart: Some(ch) } => {
            let mut ch = ch.clone();
            ch.shift(s.sheet, s.at, s.delta, s.rows);
            vec![Op::Chart { id: *id, chart: Some(ch) }]
        }
        Op::Picture { id, picture: Some(p) } => {
            let mut p = p.clone();
            p.shift(s.sheet, s.at, s.delta, s.rows);
            vec![Op::Picture { id: *id, picture: Some(p) }]
        }
        Op::Note { target: NoteTarget::Column { sheet, column }, note } if same(*sheet) && !s.rows => match s
            .line(*column)
        {
            Some(c) => vec![Op::Note { target: NoteTarget::Column { sheet: *sheet, column: c }, note: note.clone() }],
            None => vec![],
        },
        other => vec![other.clone()],
    }
}

/// Where an insert at `at` goes after the shift (two inserts at one place:
/// the earlier one first).
fn insert_at(at: i32, s: &Shift, later: bool) -> i32 {
    if s.delta > 0 {
        if at > s.at || (at == s.at && later) { at + s.delta } else { at }
    } else {
        let end = s.at - s.delta;
        if at >= end {
            at + s.delta
        } else if at > s.at {
            s.at
        } else {
            at
        }
    }
}

/// A delete of `count` lines at `at`, after the shift: what's left of it.
fn delete_after(at: i32, count: i32, s: &Shift) -> Vec<(i32, i32)> {
    let end = at + count; // one past the last
    if s.delta > 0 {
        if s.at >= end {
            vec![(at, count)]
        } else if s.at <= at {
            vec![(at + s.delta, count)]
        } else {
            // Inserted inside it: delete round the new lines (the first part, then the rest, now after them).
            let first = s.at - at;
            vec![(at, first), (at + s.delta, count - first)]
        }
    } else {
        let (d1, d2) = (s.at, s.at - s.delta);
        let left = (at..end).filter(|p| *p < d1 || *p >= d2).count() as i32;
        if left == 0 {
            return vec![];
        }
        let first = (at..end).find(|p| *p < d1 || *p >= d2).unwrap_or(at);
        vec![(s.line(first).unwrap_or(at), left)]
    }
}

/// The sheet an op is on (none: the whole book).
pub(crate) fn sheet_of(op: &Op) -> Option<u32> {
    match op {
        Op::Cells { sheet, .. }
        | Op::Styles { sheet, .. }
        | Op::InsertRows { sheet, .. }
        | Op::DeleteRows { sheet, .. }
        | Op::InsertColumns { sheet, .. }
        | Op::DeleteColumns { sheet, .. }
        | Op::Rows { sheet, .. }
        | Op::Columns { sheet, .. }
        | Op::Freeze { sheet, .. }
        | Op::Merged { sheet, .. }
        | Op::AddSheet { sheet, .. }
        | Op::RemoveSheet { sheet }
        | Op::RenameSheet { sheet, .. }
        | Op::MoveSheet { sheet, .. }
        | Op::SheetInfo { sheet, .. }
        | Op::Report { sheet, .. }
        | Op::Summary { sheet, .. }
        | Op::Query { sheet, .. }
        | Op::Editor { sheet, .. }
        | Op::CsvLink { sheet, .. }
        | Op::Sparkline { sheet, .. } => Some(*sheet),
        Op::Table { table: Some(t), .. } => Some(t.sheet),
        Op::Chart { chart: Some(c), .. } => Some(c.sheet),
        Op::Picture { picture: Some(p), .. } => Some(p.sheet),
        Op::Note { target: NoteTarget::Column { sheet, .. }, .. } => Some(*sheet),
        _ => None,
    }
}

/// Whether `a` and `b` set the same thing (the later wins).
fn same_target(a: &Op, b: &Op) -> bool {
    use Op::*;
    match (a, b) {
        (Freeze { sheet: x, .. }, Freeze { sheet: y, .. })
        | (Merged { sheet: x, .. }, Merged { sheet: y, .. })
        | (RenameSheet { sheet: x, .. }, RenameSheet { sheet: y, .. })
        | (MoveSheet { sheet: x, .. }, MoveSheet { sheet: y, .. })
        | (SheetInfo { sheet: x, .. }, SheetInfo { sheet: y, .. })
        | (Report { sheet: x, .. }, Report { sheet: y, .. })
        | (Summary { sheet: x, .. }, Summary { sheet: y, .. })
        | (Query { sheet: x, .. }, Query { sheet: y, .. })
        | (Editor { sheet: x, .. }, Editor { sheet: y, .. })
        | (CsvLink { sheet: x, .. }, CsvLink { sheet: y, .. }) => x == y,
        (Table { name: x, .. }, Table { name: y, .. }) | (Filter { table: x, .. }, Filter { table: y, .. }) => {
            x.eq_ignore_ascii_case(y)
        }
        (Definition { name: x, .. }, Definition { name: y, .. }) => x.eq_ignore_ascii_case(y),
        (Chart { id: x, .. }, Chart { id: y, .. }) | (Picture { id: x, .. }, Picture { id: y, .. }) => x == y,
        (Note { target: x, .. }, Note { target: y, .. }) => x == y,
        (Sparkline { sheet: s1, row: r1, col: c1, .. }, Sparkline { sheet: s2, row: r2, col: c2, .. }) => {
            (s1, r1, c1) == (s2, r2, c2)
        }
        _ => false,
    }
}

impl Transform for Change {
    fn transform(a: &Change, b: &Change, later: bool) -> Vec<Change> {
        let wrap = |ops: Vec<Op>| ops.into_iter().map(|op| Change { op, name: a.name.clone() }).collect::<Vec<_>>();
        // Rows or columns moved.
        let shift = match &b.op {
            Op::InsertRows { sheet, at, count } => {
                Some(Shift { sheet: *sheet, name: &b.name, rows: true, at: *at, delta: *count })
            }
            Op::DeleteRows { sheet, at, count } => {
                Some(Shift { sheet: *sheet, name: &b.name, rows: true, at: *at, delta: -*count })
            }
            Op::InsertColumns { sheet, at, count } => {
                Some(Shift { sheet: *sheet, name: &b.name, rows: false, at: *at, delta: *count })
            }
            Op::DeleteColumns { sheet, at, count } => {
                Some(Shift { sheet: *sheet, name: &b.name, rows: false, at: *at, delta: -*count })
            }
            _ => None,
        };
        if let Some(s) = shift {
            return wrap(shift_op(&a.op, &a.name, &s, later));
        }
        match (&a.op, &b.op) {
            // A sheet the other window removed: nothing of it is left to change.
            (op, Op::RemoveSheet { sheet }) if sheet_of(op) == Some(*sheet) && !matches!(op, Op::AddSheet { .. }) => {
                vec![]
            }
            // The same cells: the later wins.
            (Op::Cells { sheet: s1, cells }, Op::Cells { sheet: s2, cells: theirs }) if s1 == s2 && !later => {
                let taken: std::collections::HashSet<(i32, i32)> = theirs.iter().map(|c| (c.row, c.col)).collect();
                let cells: Vec<CellEdit> = cells.iter().filter(|c| !taken.contains(&(c.row, c.col))).cloned().collect();
                if cells.is_empty() { vec![] } else { wrap(vec![Op::Cells { sheet: *s1, cells }]) }
            }
            (Op::Styles { sheet: s1, cells }, Op::Styles { sheet: s2, cells: theirs }) if s1 == s2 && !later => {
                let taken: std::collections::HashSet<(i32, i32)> = theirs.iter().map(|c| (c.row, c.col)).collect();
                let cells: Vec<StyleEdit> =
                    cells.iter().filter(|c| !taken.contains(&(c.row, c.col))).cloned().collect();
                if cells.is_empty() { vec![] } else { wrap(vec![Op::Styles { sheet: *s1, cells }]) }
            }
            // Two new sheets with one name: the later one takes another; the
            // earlier window's own sheet is renamed first when it comes back.
            // New sheets at one place: the earlier one first.
            (Op::AddSheet { sheet, name, at, info }, Op::AddSheet { sheet: other, name: theirs, at: their_at, .. })
                if sheet != other =>
            {
                let at = if *at > *their_at || (*at == *their_at && later) { at + 1 } else { *at };
                if !name.eq_ignore_ascii_case(theirs) {
                    wrap(vec![Op::AddSheet { sheet: *sheet, name: name.clone(), at, info: info.clone() }])
                } else if later {
                    wrap(vec![Op::AddSheet { sheet: *sheet, name: format!("{name} 2"), at, info: info.clone() }])
                } else {
                    vec![
                        Change {
                            op: Op::RenameSheet { sheet: *other, name: format!("{theirs} 2") },
                            name: format!("{theirs} 2"),
                        },
                        Change {
                            op: Op::AddSheet { sheet: *sheet, name: name.clone(), at, info: info.clone() },
                            name: a.name.clone(),
                        },
                    ]
                }
            }
            (x, y) if same_target(x, y) && !later => vec![],
            _ => vec![a.clone()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_collab::transform;

    fn ch(op: Op) -> Change {
        Change { op, name: "Sales".into() }
    }

    fn cells(sheet: u32, list: &[(i32, i32, &str)]) -> Op {
        Op::Cells {
            sheet,
            cells: list
                .iter()
                .map(|(r, c, v)| CellEdit { row: *r, col: *c, data: Some(CellData::Input(v.to_string())) })
                .collect(),
        }
    }

    #[test]
    fn formulas_follow_rows_and_columns() {
        let ins = Shift { sheet: 1, name: "Sales", rows: true, at: 3, delta: 2 };
        assert_eq!(shift_formula("=SUM(A1:A5)+B2+$C$7", "Sales", &ins), "=SUM(A1:A7)+B2+$C$9");
        assert_eq!(shift_formula("=Sales!A4+'Other'!A4+\"A4\"", "Notes", &ins), "=Sales!A6+'Other'!A4+\"A4\"");
        assert_eq!(shift_formula("=SUM(A:A)", "Sales", &ins), "=SUM(A:A)");
        let del = Shift { sheet: 1, name: "Sales", rows: true, at: 3, delta: -2 };
        assert_eq!(shift_formula("=A3+A6+SUM(A1:A5)", "Sales", &del), "=#REF!+A4+SUM(A1:A3)");
        let cols = Shift { sheet: 1, name: "Sales", rows: false, at: 2, delta: 1 };
        assert_eq!(shift_formula("=A1+B1+ SUM( B2:C3 )", "Sales", &cols), "=A1+C1+ SUM( C2:D3 )");
        assert_eq!(shift_formula("=SUM(B:B)+SUM($A:C)", "Sales", &cols), "=SUM(C:C)+SUM($A:D)");
        assert_eq!(shift_formula("=SUM(2:5)+SUM($4:$4)", "Sales", &ins), "=SUM(2:7)+SUM($6:$6)");
    }

    #[test]
    fn cells_move_and_deleted_ones_go() {
        let a = [ch(cells(1, &[(2, 1, "x"), (5, 1, "=A2"), (5, 2, "y")]))];
        let b = [ch(Op::InsertRows { sheet: 1, at: 3, count: 1 })];
        let (a2, _) = transform(&a, &b);
        assert_eq!(a2[0].op, cells(1, &[(2, 1, "x"), (6, 1, "=A2"), (6, 2, "y")]));
        let b = [ch(Op::DeleteRows { sheet: 1, at: 5, count: 1 })];
        let (a2, _) = transform(&a, &b);
        assert_eq!(a2[0].op, cells(1, &[(2, 1, "x")]));
    }

    #[test]
    fn deletes_meet_inserts_and_each_other() {
        let s = Shift { sheet: 1, name: "", rows: true, at: 5, delta: 2 };
        assert_eq!(delete_after(3, 4, &s), vec![(3, 2), (5, 2)]);
        let s = Shift { sheet: 1, name: "", rows: true, at: 4, delta: -3 }; // 4..6 gone
        assert_eq!(delete_after(2, 4, &s), vec![(2, 2)]); // 2..5 → 2,3 left
        assert_eq!(delete_after(5, 4, &s), vec![(4, 2)]); // 5..8 → 7,8 left, now at 4
        assert_eq!(delete_after(4, 3, &s), vec![]);
    }

    #[test]
    fn tables_move_as_ironcalc_moves_them() {
        let col = |n: &str| crate::tables::TableCol { name: n.into(), total: None };
        let t = crate::tables::TableDef {
            name: "Shop".into(),
            sheet: 1,
            top: 1,
            left: 1,
            bottom: 4,
            right: 2,
            header: true,
            totals: false,
            columns: vec![col("Item"), col("Price")],
            stripes: true,
        };
        let a = [ch(Op::Table { name: "Shop".into(), table: Some(t.clone()) })];
        // A column inserted inside the table is a new column of it: as many names as columns.
        let (a2, _) = transform(&a, &[ch(Op::InsertColumns { sheet: 1, at: 2, count: 1 })]);
        let Op::Table { table: Some(got), .. } = &a2[0].op else { panic!("{a2:?}") };
        assert_eq!((got.left, got.right), (1, 3));
        let names: Vec<&str> = got.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Item", "Column3", "Price"]);
        // Its header row deleted: the table went.
        let (a2, _) = transform(&a, &[ch(Op::DeleteRows { sheet: 1, at: 1, count: 1 })]);
        assert!(a2.is_empty(), "{a2:?}");
        // All its data rows deleted: one stays.
        let (a2, _) = transform(&a, &[ch(Op::DeleteRows { sheet: 1, at: 2, count: 3 })]);
        let Op::Table { table: Some(got), .. } = &a2[0].op else { panic!("{a2:?}") };
        assert_eq!((got.top, got.bottom), (1, 2));
    }

    #[test]
    fn the_later_wins_and_names_stay_apart() {
        let a = [ch(cells(1, &[(1, 1, "mine"), (1, 2, "only mine")]))];
        let b = [ch(cells(1, &[(1, 1, "theirs")]))];
        let (a2, b2) = transform(&a, &b);
        assert_eq!(a2[0].op, a[0].op, "a is later: it keeps both");
        assert_eq!(b2.len(), 0, "b loses the cell a set after it");
        let a = [ch(Op::AddSheet { sheet: 10, name: "Sheet2".into(), at: 1, info: Default::default() })];
        let b = [ch(Op::AddSheet { sheet: 20, name: "Sheet2".into(), at: 1, info: Default::default() })];
        let (a2, b2) = transform(&a, &b);
        assert!(matches!(&a2[0].op, Op::AddSheet { name, .. } if name == "Sheet2 2"));
        assert!(matches!(&b2[0].op, Op::RenameSheet { sheet: 10, name } if name == "Sheet2 2"));
        assert!(matches!(&b2[1].op, Op::AddSheet { sheet: 20, .. }));
    }
}
