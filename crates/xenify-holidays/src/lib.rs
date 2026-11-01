//! Public holidays for any year, offline.
//!
//! Each country is a small TOML file of rules in `data/` (a fixed day, the
//! nth weekday of a month, a weekday on or after a day, days from Easter),
//! with the years a holiday moved or didn't happen, and lists of dates for the
//! few that follow no rule (lunar calendars, one-offs). The rules were worked
//! out from python-holidays (MIT) by `scripts/holidays/infer.py` and are
//! checked against it by `scripts/holidays/check.py`.
//!
//! Regions are ISO 3166-2 codes without the country ("ENG", "BY", "VIC");
//! the region "" is the country with no region chosen.

mod date;

pub use date::{Date, days_in_month, easter, orthodox_easter};

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

/// The data files, by country code.
const FILES: &[(&str, &str)] = &[
    ("AT", include_str!("../data/at.toml")),
    ("AU", include_str!("../data/au.toml")),
    ("BE", include_str!("../data/be.toml")),
    ("BR", include_str!("../data/br.toml")),
    ("CA", include_str!("../data/ca.toml")),
    ("CH", include_str!("../data/ch.toml")),
    ("CZ", include_str!("../data/cz.toml")),
    ("DE", include_str!("../data/de.toml")),
    ("DK", include_str!("../data/dk.toml")),
    ("ES", include_str!("../data/es.toml")),
    ("FI", include_str!("../data/fi.toml")),
    ("FR", include_str!("../data/fr.toml")),
    ("GB", include_str!("../data/gb.toml")),
    ("IE", include_str!("../data/ie.toml")),
    ("IN", include_str!("../data/in.toml")),
    ("IT", include_str!("../data/it.toml")),
    ("JP", include_str!("../data/jp.toml")),
    ("MX", include_str!("../data/mx.toml")),
    ("NL", include_str!("../data/nl.toml")),
    ("NO", include_str!("../data/no.toml")),
    ("NZ", include_str!("../data/nz.toml")),
    ("PL", include_str!("../data/pl.toml")),
    ("PT", include_str!("../data/pt.toml")),
    ("SE", include_str!("../data/se.toml")),
    ("SG", include_str!("../data/sg.toml")),
    ("US", include_str!("../data/us.toml")),
    ("ZA", include_str!("../data/za.toml")),
];

/// A country and its regions, for pickers.
#[derive(Clone, Debug, Serialize)]
pub struct Country {
    pub code: String,
    pub name: String,
    /// (code, name), in the file's order.
    pub regions: Vec<(String, String)>,
}

/// One day off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holiday {
    pub date: Date,
    pub name: String,
    /// Set when this is the working day a weekend holiday moved to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<Date>,
}

#[derive(Deserialize)]
struct File {
    country: String,
    name: String,
    #[serde(default)]
    regions: toml::Table,
    #[serde(default, rename = "holiday")]
    holidays: Vec<Rule>,
}

#[derive(Deserialize, Default)]
struct Rule {
    name: String,
    date: Option<String>,
    weekday: Option<String>,
    month: Option<u32>,
    nth: Option<i32>,
    on_or_after: Option<String>,
    on_or_before: Option<String>,
    easter: Option<i32>,
    orthodox_easter: Option<i32>,
    from: Option<i32>,
    until: Option<i32>,
    #[serde(default)]
    except: Vec<i32>,
    substitute: Option<String>,
    /// The first year weekend days move (none before it).
    substitute_from: Option<i32>,
    regions: Option<Vec<String>>,
    #[serde(default)]
    moved: BTreeMap<String, String>,
    #[serde(default)]
    dates: BTreeMap<String, String>,
}

struct Parsed {
    country: Country,
    rules: Vec<Rule>,
}

fn parsed() -> &'static HashMap<&'static str, Parsed> {
    static ALL: OnceLock<HashMap<&'static str, Parsed>> = OnceLock::new();
    ALL.get_or_init(|| {
        FILES
            .iter()
            .map(|(code, text)| {
                let file: File = toml::from_str(text).unwrap_or_else(|e| panic!("holidays/{code}: {e}"));
                let regions =
                    file.regions.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or(k).to_string())).collect();
                let country = Country { code: file.country, name: file.name, regions };
                (*code, Parsed { country, rules: file.holidays })
            })
            .collect()
    })
}

/// Every country with built-in holidays, by name.
pub fn countries() -> Vec<Country> {
    let mut v: Vec<Country> = parsed().values().map(|p| p.country.clone()).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn country(code: &str) -> Option<Country> {
    parsed().get(code.to_ascii_uppercase().as_str()).map(|p| p.country.clone())
}

fn weekday_of(s: &str) -> Option<u32> {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter().position(|w| *w == s).map(|i| i as u32)
}

fn month_day(y: i32, s: &str) -> Option<Date> {
    let (m, d) = s.split_once('-')?;
    Date::new(y, m.parse().ok()?, d.parse().ok()?)
}

fn nth_weekday(y: i32, m: u32, wd: u32, n: i32) -> Option<Date> {
    if n > 0 {
        let first = Date::new(y, m, 1)?;
        let d = first.add(((wd + 7 - first.weekday()) % 7) as i32 + 7 * (n - 1));
        (d.ymd().1 == m).then_some(d)
    } else {
        let last = Date::new(y, m, days_in_month(y, m))?;
        Some(last.add(-(((last.weekday() + 7 - wd) % 7) as i32)))
    }
}

impl Rule {
    fn applies(&self, region: &str) -> bool {
        self.regions.as_ref().is_none_or(|r| r.iter().any(|x| x == region))
    }

    /// The day in year `y`, before any weekend substitute.
    fn day(&self, y: i32) -> Option<Date> {
        let key = y.to_string();
        if !self.dates.is_empty() {
            return self.dates.get(&key).and_then(|s| month_day(y, s));
        }
        if self.from.is_some_and(|f| y < f) || self.until.is_some_and(|u| y > u) || self.except.contains(&y) {
            return None;
        }
        if let Some(s) = self.moved.get(&key) {
            return month_day(y, s);
        }
        if let Some(s) = &self.date {
            return month_day(y, s);
        }
        if let Some(off) = self.easter {
            return Some(easter(y).add(off));
        }
        if let Some(off) = self.orthodox_easter {
            return Some(orthodox_easter(y).add(off));
        }
        let wd = weekday_of(self.weekday.as_deref()?)?;
        if let (Some(m), Some(n)) = (self.month, self.nth) {
            return nth_weekday(y, m, wd, n);
        }
        if let Some(s) = &self.on_or_after {
            let d = month_day(y, s)?;
            return Some(d.add(((wd + 7 - d.weekday()) % 7) as i32));
        }
        if let Some(s) = &self.on_or_before {
            let d = month_day(y, s)?;
            return Some(d.add(-(((d.weekday() + 7 - wd) % 7) as i32)));
        }
        None
    }
}

/// The holidays of `country` (and `region`, "" for none) in `year`, by date.
/// A weekend holiday that moves keeps its own day and adds the working day
/// it moved to (`moved_from` set). Unknown countries have none.
pub fn holidays(country: &str, region: &str, year: i32) -> Vec<Holiday> {
    let Some(p) = parsed().get(country.to_ascii_uppercase().as_str()) else { return Vec::new() };
    // Look a year either side: a substitute can cross New Year.
    let mut raw: Vec<(Date, &Rule)> = Vec::new();
    for y in year - 1..=year + 1 {
        for r in p.rules.iter().filter(|r| r.applies(region)) {
            if let Some(d) = r.day(y) {
                raw.push((d, r));
            }
        }
    }
    raw.sort_by_key(|(d, _)| *d);
    let mut taken: BTreeSet<Date> = raw.iter().map(|(d, _)| *d).collect();
    let mut out: Vec<Holiday> = Vec::new();
    for (d, r) in &raw {
        out.push(Holiday { date: *d, name: r.name.clone(), moved_from: None });
        if !d.is_weekend() {
            continue;
        }
        let policy = r.substitute.as_deref().filter(|_| r.substitute_from.is_none_or(|f| d.year() >= f));
        let to = match policy {
            Some("nearest") => Some(if d.weekday() == 5 { d.add(-1) } else { d.add(1) }),
            Some("sunday") if d.weekday() == 5 => None,
            Some("before") => {
                let mut x = d.add(-1);
                while x.is_weekend() || taken.contains(&x) {
                    x = x.add(-1);
                }
                Some(x)
            }
            Some(_) => {
                let mut x = d.add(1);
                while x.is_weekend() || taken.contains(&x) {
                    x = x.add(1);
                }
                Some(x)
            }
            None => None,
        };
        if let Some(to) = to {
            taken.insert(to);
            out.push(Holiday { date: to, name: r.name.clone(), moved_from: Some(*d) });
        }
    }
    out.retain(|h| h.date.year() == year);
    out.sort_by(|a, b| a.date.cmp(&b.date).then(a.moved_from.is_some().cmp(&b.moved_from.is_some())));
    out
}

/// Working days off (holidays that aren't on a weekend), with names, for
/// one country and region; years are worked out once and kept.
pub struct Days {
    country: String,
    region: String,
    years: Mutex<HashMap<i32, BTreeMap<Date, String>>>,
}

impl Days {
    pub fn new(country: &str, region: &str) -> Days {
        Days { country: country.to_ascii_uppercase(), region: region.to_string(), years: Mutex::new(HashMap::new()) }
    }

    /// The holiday's name when `d` is a holiday (weekends included).
    pub fn name(&self, d: Date) -> Option<String> {
        let y = d.year();
        let mut years = self.years.lock().unwrap_or_else(|e| e.into_inner());
        let map = years.entry(y).or_insert_with(|| {
            let mut m = BTreeMap::new();
            for h in holidays(&self.country, &self.region, y) {
                m.entry(h.date).or_insert(h.name);
            }
            m
        });
        map.get(&d).cloned()
    }

    pub fn is_holiday(&self, d: Date) -> bool {
        self.name(d).is_some()
    }
}

#[cfg(test)]
mod tests;
