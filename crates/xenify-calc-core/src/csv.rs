//! CSV and other separated text (tab, semicolon, pipe…): one sheet of
//! values. Opening reads each value the way typing does (numbers, dates,
//! TRUE), except values a number can't hold exactly: leading zeros, a plus
//! sign and more than 28 significant digits stay as they are, as text. Or
//! everything can be read as text. Saving writes numbers in full, every
//! digit (dates as shown).

use crate::book::{Book, CellData};
use crate::ops::{CellEdit, Op};

/// The separator a text most likely uses: tab, pipe, semicolon or comma.
/// The one found the same number of times on every line wins.
pub fn separator(text: &str) -> char {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(20).collect();
    let score = |c: char| -> (bool, usize) {
        let counts: Vec<usize> = lines.iter().map(|l| l.chars().filter(|x| *x == c).count()).collect();
        let steady = !counts.is_empty() && counts[0] > 0 && counts.iter().all(|n| *n == counts[0]);
        (steady, counts.iter().sum())
    };
    ['\t', '|', ';', ',']
        .into_iter()
        .map(|c| (c, score(c)))
        .filter(|(_, (_, n))| *n > 0)
        .max_by_key(|(c, (steady, n))| (*steady, *n, *c == ','))
        .map(|(c, _)| c)
        .unwrap_or(',')
}

/// How values are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Values {
    /// Numbers, dates and TRUE/FALSE as typing reads them; values that a
    /// number would change (0042, 12345678901234567890) stay as text.
    #[default]
    Typed,
    /// Every value as text, exactly as in the file.
    Text,
}

/// Whether a value would change if it became a number: leading zeros, a
/// plus sign, or more digits than a number keeps (28).
pub fn keep_exact(v: &str) -> bool {
    let t = v.trim();
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    let numeric = !digits.is_empty() && t.chars().all(|c| c.is_ascii_digit() || ".,-+ eE%".contains(c));
    if !numeric {
        return false;
    }
    let unsigned = t.trim_start_matches(['-', '+']);
    let leading_zero =
        unsigned.len() > 1 && unsigned.starts_with('0') && !unsigned.starts_with("0.") && !unsigned.starts_with("0,");
    let significant = digits.trim_start_matches('0').trim_end_matches('0').len();
    leading_zero || t.starts_with('+') || significant > 28
}

/// What a value becomes in a cell.
pub fn cell_of(v: &str, values: Values) -> CellData {
    // Text that starts with = is kept as text (a CSV holds values, not formulas).
    if values == Values::Text || v.starts_with('=') || keep_exact(v) {
        CellData::Text(v.to_string())
    } else {
        CellData::Input(v.to_string())
    }
}

/// Rows of fields, with quoted fields ("a, b", "say ""hi""") and line breaks in quotes.
pub fn parse(text: &str, sep: char) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// A book of one sheet (named `name`) from CSV or tab-separated text.
pub fn open(text: &str, name: &str) -> Result<Book, String> {
    open_with(text, name, None, Values::Typed)
}

/// The same with the separator (none: the likeliest) and how values are read.
pub fn open_with(text: &str, name: &str, sep: Option<char>, values: Values) -> Result<Book, String> {
    let mut book = Book::new();
    let sheet = book.id_at(0).ok_or("no sheet")?;
    let clean: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if !clean.trim().is_empty() {
        crate::ops::apply(&mut book, &Op::RenameSheet { sheet, name: clean.trim().to_string() })?;
    }
    let rows = parse(text, sep.unwrap_or_else(|| separator(text)));
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            cells.push(CellEdit { row: r as i32 + 1, col: c as i32 + 1, data: Some(cell_of(v, values)) });
        }
    }
    // Columns wide enough for what's in them (long ids would show ### otherwise).
    let columns = fit_widths(&rows);
    let mut ops = vec![Op::Cells { sheet, cells }];
    if !columns.is_empty() {
        ops.push(Op::Columns { sheet, columns });
    }
    crate::ops::apply_all(&mut book, &ops)?;
    Ok(book)
}

/// Column sizes wide enough for `rows` (as they show; the first 500), from
/// column A: only those wider than normal, at most 320 pixels. Also what
/// query and table editor sheets are sized by when they first load.
pub fn fit_widths(rows: &[Vec<String>]) -> Vec<crate::ops::LineSize> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    (0..width)
        .filter_map(|c| {
            let chars = rows.iter().take(500).filter_map(|r| r.get(c)).map(|v| v.chars().count()).max().unwrap_or(0);
            let px = (chars as f64 * 8.2 + 24.0).min(320.0);
            (px > 90.0).then(|| crate::ops::LineSize { at: c as i32 + 1, size: Some(px.round()), hidden: false })
        })
        .collect()
}

/// Whether a number format shows a date or a time. Quoted text, escaped
/// characters (`\d`, `_d`, `*d`) and brackets (`[Red]`, `[$€-407]`) don't
/// count, except elapsed time (`[h]:mm`).
fn dated(fmt: &str) -> bool {
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                chars.by_ref().find(|c| *c == '"');
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            '[' => {
                let inside: String = chars.by_ref().take_while(|c| *c != ']').collect();
                if !inside.is_empty() && inside.chars().all(|c| "hHmMsS".contains(c)) {
                    return true;
                }
            }
            c if "dmyhsDMYHS".contains(c) => return true,
            _ => {}
        }
    }
    false
}

/// Sheet `sheet` as text separated by `sep`. Numbers are written in full
/// (1234.5678, not 1,234.57), dates and times and words as shown.
pub fn save(book: &Book, sheet: u32, sep: char) -> Result<String, String> {
    let i = book.index(sheet)?;
    let w = &book.model.workbook.worksheets[i as usize];
    if w.sheet_data.is_empty() {
        return Ok(String::new());
    }
    let d = w.dimension();
    let mut out = String::new();
    for r in 1..=d.max_row {
        let mut line = Vec::new();
        for c in 1..=d.max_column {
            let v = match w.cell(r, c).and_then(|cell| cell.number()) {
                Some(n) if !dated(&book.style(sheet, r, c).num_fmt) => n.to_plain_string(),
                _ => book.model.get_formatted_cell_value(i, r, c).unwrap_or_default(),
            };
            let needs = v.contains(sep) || v.contains('"') || v.contains('\n') || v.contains('\r');
            line.push(if needs { format!("\"{}\"", v.replace('"', "\"\"")) } else { v });
        }
        out.push_str(&line.join(&sep.to_string()));
        out.push_str("\r\n");
    }
    Ok(out)
}
