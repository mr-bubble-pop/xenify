//! Xenify: tables follow the sheet. Inserting rows or columns inside a table
//! makes it bigger, deleting them makes it smaller, and rows or columns
//! before it move it. Structured references (`Sales[Revenue]`) are then
//! worked out again, so totals keep covering the whole column.

use crate::expressions::lexer::LexerMode;
use crate::expressions::parser::stringify::to_rc_format;
use crate::expressions::parser::{parse_range, Node};
use crate::expressions::token::TableReference;
use crate::expressions::types::CellReferenceRC;
use crate::expressions::utils::number_to_column;
use crate::model::Model;
use crate::types::TableColumn;

/// Rows or columns inserted (`delta` > 0) or deleted (`delta` < 0) at `at`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TableShift {
    Rows { at: i32, delta: i32 },
    Columns { at: i32, delta: i32 },
}

/// The first and last of `lo..=hi` after inserting or deleting at `at`, or
/// none when all of it is deleted. Inserting after `lo`, up to and
/// including `hi`, makes the span longer.
fn shift_span(lo: i32, hi: i32, at: i32, delta: i32) -> Option<(i32, i32)> {
    if delta > 0 {
        if at <= lo {
            Some((lo + delta, hi + delta))
        } else if at <= hi {
            Some((lo, hi + delta))
        } else {
            Some((lo, hi))
        }
    } else {
        let (first, last) = (at, at - delta - 1);
        let before = (last.min(lo - 1) - first + 1).max(0);
        let inside = (last.min(hi) - first.max(lo) + 1).max(0);
        if inside == hi - lo + 1 {
            return None;
        }
        Some((lo - before, hi - before - inside))
    }
}

pub(crate) fn range_text(c1: i32, r1: i32, c2: i32, r2: i32) -> String {
    let col = |c: i32| number_to_column(c).unwrap_or_default();
    format!("{}{}:{}{}", col(c1), r1, col(c2), r2)
}

impl Model<'_> {
    /// Moves, grows or shrinks the tables on `sheet`; true when one changed.
    pub(crate) fn displace_tables(&mut self, sheet: u32, shift: TableShift) -> bool {
        let Ok(sheet_name) = self.workbook.worksheet(sheet).map(|w| w.get_name()) else {
            return false;
        };
        let mut changed = false;
        let mut gone = Vec::new();
        for (key, table) in self.workbook.tables.iter_mut() {
            if table.sheet_name != sheet_name {
                continue;
            }
            let Some((c1, r1, c2, r2)) = parse_range(&table.reference).ok() else {
                continue;
            };
            let header = table.header_row_count as i32;
            let totals = table.totals_row_count as i32;
            let new = match shift {
                TableShift::Rows { at, delta } => {
                    // Deleting the header row takes the table away.
                    if delta < 0 && header > 0 && at <= r1 && r1 <= at - delta - 1 {
                        None
                    } else {
                        shift_span(r1, r2, at, delta).map(|(a, b)| {
                            // At least one row of data stays.
                            (c1, a, c2, b.max(a + header + totals))
                        })
                    }
                }
                TableShift::Columns { at, delta } => match shift_span(c1, c2, at, delta) {
                    None => None,
                    Some((a, b)) => {
                        if delta > 0 && at > c1 && at <= c2 {
                            let index = (at - c1) as usize;
                            let mut next_id = table.columns.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                            for k in 0..delta {
                                let mut n = table.columns.len() + 1;
                                let mut name = format!("Column{n}");
                                while table.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
                                    n += 1;
                                    name = format!("Column{n}");
                                }
                                let column = TableColumn { id: next_id, name, ..Default::default() };
                                next_id += 1;
                                table.columns.insert(index + k as usize, column);
                            }
                        } else if delta < 0 {
                            let (first, last) = (at, at - delta - 1);
                            let mut k = 0;
                            table.columns.retain(|_| {
                                let c = c1 + k;
                                k += 1;
                                c < first || c > last
                            });
                        }
                        Some((a, r1, b, r2))
                    }
                },
            };
            match new {
                None => gone.push(key.clone()),
                Some((a, b, c, d)) => {
                    let reference = range_text(a, b, c, d);
                    if reference != table.reference {
                        table.reference = reference;
                        changed = true;
                    }
                }
            }
        }
        for key in &gone {
            self.workbook.tables.remove(key);
        }
        changed || !gone.is_empty()
    }

    /// Call after changing `workbook.tables`: structured references are
    /// worked out again from the tables as they are now, and the workbook is
    /// evaluated.
    pub fn refresh_tables(&mut self) {
        self.reset_parsed_structures();
    }
}

/// Calls `f` with the table name and columns of every structured reference in `node`.
fn for_each_table_ref(node: &mut Node, f: &mut dyn FnMut(&mut String, &mut Option<TableReference>)) {
    match node {
        Node::TableRefKind { table_name, table_reference, .. } => f(table_name, table_reference),
        Node::OpRangeKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            for_each_table_ref(left, f);
            for_each_table_ref(right, f);
        }
        Node::FunctionKind { args, .. } | Node::NamedFunctionKind { args, .. } => {
            for arg in args {
                for_each_table_ref(arg, f);
            }
        }
        Node::UnaryKind { right, .. } => for_each_table_ref(right, f),
        Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => for_each_table_ref(child, f),
        Node::LambdaDefKind { body, .. } => for_each_table_ref(body, f),
        Node::LambdaCallKind { lambda, args } => {
            for_each_table_ref(lambda, f);
            for arg in args {
                for_each_table_ref(arg, f);
            }
        }
        Node::BooleanKind(_)
        | Node::NumberKind(_)
        | Node::StringKind(_)
        | Node::ErrorKind(_)
        | Node::ParseErrorKind { .. }
        | Node::ArrayKind(_)
        | Node::DefinedNameKind(_)
        | Node::TableNameKind(_)
        | Node::NamedVariableKind { .. }
        | Node::EmptyArgKind
        | Node::ReferenceKind { .. }
        | Node::RangeKind { .. }
        | Node::WrongReferenceKind { .. }
        | Node::WrongRangeKind { .. } => {}
    }
}

impl Model<'_> {
    /// Rewrites every formula's structured references with `change`, then the
    /// caller changes `workbook.tables` and calls [`Model::refresh_tables`].
    fn rewrite_table_refs(&mut self, change: &mut dyn FnMut(&mut String, &mut Option<TableReference>)) {
        self.parser.set_lexer_mode(LexerMode::R1C1);
        for worksheet in &mut self.workbook.worksheets {
            let cell_reference = &CellReferenceRC { sheet: worksheet.get_name(), row: 1, column: 1 };
            let mut formulas = Vec::new();
            for formula in &worksheet.shared_formulas {
                let mut t = self.parser.parse(formula, cell_reference);
                if let Node::ParseErrorKind { .. } = t {
                    formulas.push(formula.clone());
                    continue;
                }
                for_each_table_ref(&mut t, change);
                formulas.push(to_rc_format(&t));
            }
            worksheet.shared_formulas = formulas;
        }
        self.parser.set_lexer_mode(LexerMode::A1);
    }

    /// Renames table `old` to `new` in every formula and in the workbook.
    pub fn rename_table(&mut self, old: &str, new: &str) -> Result<(), String> {
        let key = self.workbook.tables.keys().find(|k| k.eq_ignore_ascii_case(old)).cloned();
        let Some(key) = key else { return Err(format!("no table {old}")) };
        self.rewrite_table_refs(&mut |name, _| {
            if name.eq_ignore_ascii_case(old) {
                *name = new.to_string();
            }
        });
        if let Some(mut t) = self.workbook.tables.remove(&key) {
            t.name = new.to_string();
            t.display_name = new.to_string();
            self.workbook.tables.insert(new.to_string(), t);
        }
        self.reset_parsed_structures();
        Ok(())
    }

    /// Renames column `old` of table `table` to `new` in every formula and
    /// in the table.
    pub fn rename_table_column(&mut self, table: &str, old: &str, new: &str) -> Result<(), String> {
        let key = self.workbook.tables.keys().find(|k| k.eq_ignore_ascii_case(table)).cloned();
        let Some(key) = key else { return Err(format!("no table {table}")) };
        let rename = |c: &mut String| {
            if c.eq_ignore_ascii_case(old) {
                *c = new.to_string();
            }
        };
        self.rewrite_table_refs(&mut |name, columns| {
            if name.eq_ignore_ascii_case(table) {
                match columns {
                    Some(TableReference::ColumnReference(c)) => rename(c),
                    Some(TableReference::RangeReference((a, b))) => {
                        rename(a);
                        rename(b);
                    }
                    None => {}
                }
            }
        });
        if let Some(t) = self.workbook.tables.get_mut(&key) {
            for c in &mut t.columns {
                rename(&mut c.name);
            }
        }
        self.reset_parsed_structures();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::shift_span;

    #[test]
    fn spans() {
        // inserting before, inside, after
        assert_eq!(shift_span(5, 10, 2, 3), Some((8, 13)));
        assert_eq!(shift_span(5, 10, 5, 3), Some((8, 13)));
        assert_eq!(shift_span(5, 10, 6, 3), Some((5, 13)));
        assert_eq!(shift_span(5, 10, 10, 1), Some((5, 11)));
        assert_eq!(shift_span(5, 10, 11, 1), Some((5, 10)));
        // deleting before, across the start, inside, all
        assert_eq!(shift_span(5, 10, 1, -2), Some((3, 8)));
        assert_eq!(shift_span(5, 10, 4, -3), Some((4, 7)));
        assert_eq!(shift_span(5, 10, 7, -2), Some((5, 8)));
        assert_eq!(shift_span(5, 10, 4, -8), None);
    }
}

impl Model<'_> {
    /// Xenify: `formula`, typed at (`sheet` index, `row`, `column`), as it
    /// reads once rows or columns move by `displace` (editing together: a
    /// formula made in one window while another inserted rows).
    pub fn displace_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: &str,
        displace: &crate::expressions::parser::stringify::DisplaceData,
    ) -> String {
        let Some(ws) = self.workbook.worksheets.get(sheet as usize) else {
            return formula.to_string();
        };
        let context = CellReferenceRC { sheet: ws.get_name(), row, column };
        let body = formula.strip_prefix('=').unwrap_or(formula);
        let node = self.parser.parse(body, &context);
        let out = crate::expressions::parser::stringify::to_string_displaced(
            &node,
            &context,
            displace,
            self.locale,
            self.language,
        );
        format!("={out}")
    }
}
