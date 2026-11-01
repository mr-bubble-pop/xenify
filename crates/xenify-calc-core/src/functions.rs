//! The function dictionary: every function formulas can use, what each of
//! its arguments is, what it does in plain words, and an example (cells
//! of example data, a formula over them and what it gives). Typing a
//! formula shows the names and arguments from here; Help › Functions shows
//! the whole entry. A test works out every example, so they stay true.

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::sync::OnceLock;

/// The catalogue files (functions/*.json) and the group each is shown under.
const FILES: &[(&str, &str, &str)] = &[
    ("Logic", "logic", include_str!("../functions/logic.json")),
    ("Maths", "maths", include_str!("../functions/maths.json")),
    ("About cells", "information", include_str!("../functions/information.json")),
    ("Look up and lists", "lookup", include_str!("../functions/lookup.json")),
    ("Text", "text", include_str!("../functions/text.json")),
    ("Statistics", "statistics", include_str!("../functions/statistics.json")),
    ("Statistics", "statistics-2", include_str!("../functions/statistics-2.json")),
    ("Dates and times", "dates", include_str!("../functions/dates.json")),
    ("Money", "money", include_str!("../functions/money.json")),
    ("Engineering", "engineering", include_str!("../functions/engineering.json")),
    ("Database tables", "database", include_str!("../functions/database.json")),
    ("Older names", "older", include_str!("../functions/older.json")),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arg {
    pub name: String,
    /// What to give it, in a few words.
    pub about: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Example {
    /// Example data from A1, typed as in cells ("12", "Apple", "2026-01-31", "=A1*2").
    #[serde(default)]
    pub cells: Vec<Vec<String>>,
    pub formula: String,
    /// What the formula shows (none: it changes, like NOW or RAND).
    pub result: Option<String>,
    /// Why it gives that, in a sentence.
    #[serde(default)]
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    #[serde(default)]
    pub group: String,
    pub args: Vec<Arg>,
    /// Whether the last argument can be given again and again (number1, number2…).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
    /// What it does, in a sentence.
    pub about: String,
    pub example: Example,
    /// Anything else worth knowing (when it gives an error, what to use instead).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// The catalogue file it's in (functions/<file>.json).
    #[serde(skip)]
    pub file: &'static str,
}

impl Entry {
    /// SUMIF(range, criteria, [sum_range])
    pub fn syntax(&self) -> String {
        let mut args: Vec<String> =
            self.args.iter().map(|a| if a.optional { format!("[{}]", a.name) } else { a.name.clone() }).collect();
        if self.more {
            args.push("…".to_string());
        }
        format!("{}({})", self.name, args.join(", "))
    }
}

/// Every entry, by group and then name.
pub fn all() -> &'static [Entry] {
    static ALL: OnceLock<Vec<Entry>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out = Vec::new();
        for (group, file, text) in FILES {
            let list: Vec<Entry> = serde_json::from_str(text).unwrap_or_else(|e| panic!("functions/{file}.json: {e}"));
            out.extend(list.into_iter().map(|mut e| {
                e.group = group.to_string();
                e.file = file;
                e
            }));
        }
        out
    })
}

pub fn find(name: &str) -> Option<&'static Entry> {
    let name = name.trim().trim_start_matches('=').to_uppercase();
    all().iter().find(|e| e.name == name)
}

/// The names, groups and one-line descriptions (for the list as a formula is typed).
pub fn list() -> Json {
    json!(
        all()
            .iter()
            .map(|e| json!({ "name": e.name, "group": e.group, "about": e.about, "syntax": e.syntax() }))
            .collect::<Vec<_>>()
    )
}

/// One whole entry, with its syntax line.
pub fn entry(name: &str) -> Option<Json> {
    find(name).map(|e| {
        let mut j = json!(e);
        j["syntax"] = json!(e.syntax());
        j
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    /// Every function the engine knows has an entry, and nothing else does.
    #[test]
    fn every_function_is_explained() {
        let known: Vec<&str> = include_str!("../functions/all.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| l.split('\t').nth(1))
            .collect();
        let have: Vec<&str> = all().iter().map(|e| e.name.as_str()).collect();
        let missing: Vec<&&str> = known.iter().filter(|n| !have.contains(n)).collect();
        let extra: Vec<&&str> = have.iter().filter(|n| !known.contains(n)).collect();
        let mut seen = std::collections::HashSet::new();
        let twice: Vec<&&str> = have.iter().filter(|n| !seen.insert(**n)).collect();
        assert!(missing.is_empty(), "{} functions without an entry: {missing:?}", missing.len());
        assert!(extra.is_empty(), "entries for unknown functions: {extra:?}");
        assert!(twice.is_empty(), "entries twice: {twice:?}");
    }

    /// Each example, worked out: the formula over its cells gives what the entry says.
    /// `XENIFY_FUNCTIONS=text,logic` checks only those files (functions/text.json…).
    #[test]
    fn every_example_works() {
        let only: Option<Vec<String>> =
            std::env::var("XENIFY_FUNCTIONS").ok().map(|v| v.split(',').map(|s| s.trim().to_lowercase()).collect());
        let mut wrong = Vec::new();
        for e in all() {
            if let Some(only) = &only
                && !only.iter().any(|f| f == e.file)
            {
                continue;
            }
            if e.about.trim().is_empty() || e.example.why.trim().is_empty() || e.args.iter().any(|a| a.about.is_empty())
            {
                wrong.push(format!("{}: description, example reason or an argument's description is empty", e.name));
            }
            if !e.example.formula.to_uppercase().contains(&format!("{}(", e.name)) {
                wrong.push(format!("{}: the example doesn't use it: {}", e.name, e.example.formula));
            }
            match run(&e.example) {
                Ok(got) => {
                    if let Some(want) = &e.example.result
                        && got != *want
                    {
                        wrong.push(format!("{}: {} gives {got:?}, the entry says {want:?}", e.name, e.example.formula));
                    }
                    if got.starts_with('#') && e.example.result.as_deref() != Some(got.as_str()) {
                        wrong.push(format!("{}: {} gives {got}", e.name, e.example.formula));
                    }
                }
                Err(why) => wrong.push(format!("{}: {why}", e.name)),
            }
        }
        assert!(wrong.is_empty(), "{} examples are wrong:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// What the example's formula shows, with its cells from A1.
    fn run(ex: &Example) -> Result<String, String> {
        let mut s = Session::new();
        let sheet = s.book.id_at(0).ok_or("no sheet")?;
        for (r, row) in ex.cells.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if !v.is_empty() {
                    s.type_in(sheet, r as i32 + 1, c as i32 + 1, v)?;
                }
            }
        }
        // The formula goes two columns right of the data.
        let col = ex.cells.iter().map(Vec::len).max().unwrap_or(0) as i32 + 2;
        s.type_in(sheet, 1, col, &ex.formula)?;
        let i = s.book.index(sheet)?;
        s.book.model.get_formatted_cell_value(i, 1, col).map_err(|e| e.to_string())
    }
}
