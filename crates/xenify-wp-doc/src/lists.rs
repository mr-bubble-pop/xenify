//! Bulleted and numbered lists: list definitions (what each level looks
//! like) and numbering.

use crate::{Document, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A paragraph's membership of a list.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ListItem {
    /// [`ListDef`] id.
    pub list: String,
    /// Nesting level, 0 = outermost.
    #[serde(default)]
    pub level: u8,
    /// Start numbering again at this value (instead of continuing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<u32>,
    /// A checklist item that's ticked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    /// More of the item above (Shift+Enter): lined up with its text, with
    /// no number or bullet of its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub follow: bool,
}

impl ListItem {
    pub fn new(list: &str, level: u8) -> Self {
        Self { list: list.into(), level, restart: None, checked: false, follow: false }
    }
}

/// The labels of checklist items: the layout draws them as a box (ticked or not).
pub const UNCHECKED: &str = "\u{2610}";
pub const CHECKED: &str = "\u{2611}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberKind {
    Bullet,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    /// No label (indent only).
    None,
    /// A tick box (checklist).
    Check,
}

/// How one level of a list looks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListLevel {
    pub kind: NumberKind,
    /// The label: `%1`…`%9` are the numbers of levels 1…9 (e.g. `%1.%2.`);
    /// for bullets, the bullet character.
    pub format: String,
    pub start: u32,
    /// Text indent from the margin, in points.
    pub indent: f32,
    /// How far the label hangs left of the text.
    pub hanging: f32,
}

impl Default for ListLevel {
    fn default() -> Self {
        Self { kind: NumberKind::Bullet, format: "•".into(), start: 1, indent: 22.0, hanging: 16.0 }
    }
}

/// A list definition: one [`ListLevel`] per nesting level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListDef {
    pub id: String,
    pub name: String,
    pub levels: Vec<ListLevel>,
}

impl Default for ListDef {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), levels: vec![ListLevel::default()] }
    }
}

pub const MAX_LEVEL: u8 = 8;

impl ListDef {
    pub const BULLETS: &'static str = "bullets";
    pub const NUMBERS: &'static str = "numbers";
    pub const OUTLINE: &'static str = "outline";
    pub const LETTERS: &'static str = "letters";
    pub const CHECKLIST: &'static str = "checklist";
    /// Numbered headings (1, 1.1, 1.1.1), counted apart from lists in the text.
    pub const HEADINGS: &'static str = "headings";

    /// The built-in list `id`, if it is one.
    pub fn builtin_def(id: &str) -> Option<ListDef> {
        Self::builtin().into_iter().find(|d| d.id == id)
    }

    /// Level `level` (levels past the defined ones repeat the pattern).
    pub fn level(&self, level: u8) -> ListLevel {
        if let Some(own) = self.levels.get(level as usize) {
            return own.clone();
        }
        let n = self.levels.len().max(1);
        let mut l = self.levels.get(level as usize % n).cloned().unwrap_or_default();
        l.indent = 22.0 * (level as f32 + 1.0);
        l
    }

    pub fn builtin() -> Vec<ListDef> {
        let levels = |f: &dyn Fn(u8) -> (NumberKind, String)| -> Vec<ListLevel> {
            (0..=MAX_LEVEL)
                .map(|i| {
                    let (kind, format) = f(i);
                    ListLevel { kind, format, start: 1, indent: 22.0 * (i as f32 + 1.0), hanging: 18.0 }
                })
                .collect()
        };
        vec![
            ListDef {
                id: Self::BULLETS.into(),
                name: "Bullets".into(),
                levels: levels(&|i| (NumberKind::Bullet, ["•", "◦", "▪"][i as usize % 3].into())),
            },
            ListDef {
                id: Self::NUMBERS.into(),
                name: "Numbers".into(),
                levels: levels(&|i| {
                    let n = i + 1;
                    match i % 3 {
                        0 => (NumberKind::Decimal, format!("%{n}.")),
                        1 => (NumberKind::LowerAlpha, format!("%{n}.")),
                        _ => (NumberKind::LowerRoman, format!("%{n}.")),
                    }
                }),
            },
            ListDef {
                id: Self::OUTLINE.into(),
                name: "Outline (1.1.1)".into(),
                levels: levels(&|i| {
                    let f: String = (1..=i + 1).map(|n| format!("%{n}.")).collect();
                    (NumberKind::Decimal, f)
                }),
            },
            ListDef {
                id: Self::HEADINGS.into(),
                name: "Numbered headings (1, 1.1, 1.1.1)".into(),
                // The number starts at the margin and the heading's text after it.
                levels: (0..=MAX_LEVEL)
                    .map(|i| {
                        let format: Vec<String> = (1..=i + 1).map(|n| format!("%{n}")).collect();
                        let indent = 26.0 + 12.0 * i as f32;
                        ListLevel { kind: NumberKind::Decimal, format: format.join("."), start: 1, indent, hanging: indent }
                    })
                    .collect(),
            },
            ListDef {
                id: Self::CHECKLIST.into(),
                name: "Checklist".into(),
                levels: levels(&|_| (NumberKind::Check, UNCHECKED.into())),
            },
            ListDef {
                id: Self::LETTERS.into(),
                name: "Letters (A. B. C.)".into(),
                levels: levels(&|i| {
                    let n = i + 1;
                    match i % 3 {
                        0 => (NumberKind::UpperAlpha, format!("%{n}.")),
                        1 => (NumberKind::Decimal, format!("%{n}.")),
                        _ => (NumberKind::LowerAlpha, format!("%{n})")),
                    }
                }),
            },
        ]
    }
}

/// A number written in style `kind`.
pub fn format_number(n: u32, kind: NumberKind) -> String {
    match kind {
        NumberKind::Decimal => n.to_string(),
        NumberKind::LowerAlpha | NumberKind::UpperAlpha => {
            // 1 → a … 26 → z, 27 → aa …
            let mut s = Vec::new();
            let mut v = n.max(1);
            while v > 0 {
                v -= 1;
                s.push(b'a' + (v % 26) as u8);
                v /= 26;
            }
            s.reverse();
            let s = String::from_utf8(s).expect("ascii");
            if kind == NumberKind::UpperAlpha { s.to_uppercase() } else { s }
        }
        NumberKind::LowerRoman | NumberKind::UpperRoman => {
            let s = roman(n);
            if kind == NumberKind::UpperRoman { s } else { s.to_lowercase() }
        }
        NumberKind::Bullet | NumberKind::None | NumberKind::Check => String::new(),
    }
}

fn roman(mut n: u32) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    let table = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// The label (bullet or number) of every list paragraph.
pub fn labels(doc: &Document) -> HashMap<NodeId, String> {
    let mut out = HashMap::new();
    // Counters per list, per level.
    let mut counters: HashMap<&str, [u32; MAX_LEVEL as usize + 1]> = HashMap::new();
    for p in doc.paragraphs() {
        let Some(item) = &p.list else { continue };
        // More of the item above: no label, and the count doesn't move on.
        if item.follow {
            continue;
        }
        let Some(def) = doc.lists.iter().find(|d| d.id == item.list) else { continue };
        let level = item.level.min(MAX_LEVEL) as usize;
        let c = counters.entry(def.id.as_str()).or_insert([0; MAX_LEVEL as usize + 1]);
        let lv = def.level(level as u8);
        c[level] = match item.restart {
            Some(r) => r,
            None if c[level] == 0 => lv.start,
            None => c[level] + 1,
        };
        for deeper in c.iter_mut().skip(level + 1) {
            *deeper = 0;
        }
        let label = if lv.kind == NumberKind::Check {
            if item.checked { CHECKED } else { UNCHECKED }.to_string()
        } else if lv.kind == NumberKind::Bullet {
            lv.format.clone()
        } else {
            // Replace %N with level N's number in that level's style (from 9 down so %1 doesn't eat %10).
            let mut s = lv.format.clone();
            for n in (1..=level + 1).rev() {
                let k = def.level((n - 1) as u8).kind;
                let v = if c[n - 1] == 0 { def.level((n - 1) as u8).start } else { c[n - 1] };
                s = s.replace(&format!("%{n}"), &format_number(v, k));
            }
            s
        };
        out.insert(p.id, label);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_every_style() {
        assert_eq!(format_number(1, NumberKind::LowerAlpha), "a");
        assert_eq!(format_number(27, NumberKind::UpperAlpha), "AA");
        assert_eq!(format_number(14, NumberKind::LowerRoman), "xiv");
        assert_eq!(format_number(1999, NumberKind::UpperRoman), "MCMXCIX");
    }
}
