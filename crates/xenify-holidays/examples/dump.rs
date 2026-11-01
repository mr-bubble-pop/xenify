//! Prints every working-day holiday for every country and region, 2000–2075,
//! as JSON {"GB/ENG": ["2000-01-03", …]}: scripts/holidays/check.py compares it
//! with python-holidays.

use std::collections::{BTreeMap, BTreeSet};

fn main() {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in xenify_holidays::countries() {
        let regions = std::iter::once(String::new()).chain(c.regions.iter().map(|(r, _)| r.clone()));
        for r in regions {
            let set = out.entry(format!("{}/{}", c.code, r)).or_default();
            for y in 2000..2076 {
                for h in xenify_holidays::holidays(&c.code, &r, y) {
                    if !h.date.is_weekend() {
                        set.insert(h.date.to_string());
                    }
                }
            }
        }
    }
    println!("{}", serde_json::to_string(&out).unwrap());
}
