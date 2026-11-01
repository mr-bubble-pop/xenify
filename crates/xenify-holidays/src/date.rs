//! A calendar day with no time or zone: what holidays and plans count in.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A day, stored as days since 1970-01-01 so adding and comparing is cheap.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Date(pub i32);

impl Date {
    pub fn new(y: i32, m: u32, d: u32) -> Option<Date> {
        if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        Some(Date(days_from_civil(y, m, d)))
    }

    /// Year, month (1–12), day (1–31).
    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0)
    }

    pub fn year(self) -> i32 {
        self.ymd().0
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday.
        (self.0 + 3).rem_euclid(7) as u32
    }

    pub fn is_weekend(self) -> bool {
        self.weekday() >= 5
    }

    #[allow(clippy::should_implement_trait)]
    pub fn add(self, days: i32) -> Date {
        Date(self.0 + days)
    }

    /// "2026-11-16".
    pub fn parse(s: &str) -> Option<Date> {
        let mut it = s.trim().splitn(3, '-');
        let y = it.next()?.parse().ok()?;
        let m = it.next()?.parse().ok()?;
        let d = it.next()?.parse().ok()?;
        Date::new(y, m, d)
    }
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant's civil-date algorithms.
fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i32 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Date, D::Error> {
        let s = String::deserialize(d)?;
        Date::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a date: {s}")))
    }
}

/// Western (Gregorian) Easter Sunday.
pub fn easter(y: i32) -> Date {
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    Date::new(y, month as u32, day as u32).expect("easter")
}

/// Orthodox Easter Sunday, as a Gregorian date.
pub fn orthodox_easter(y: i32) -> Date {
    let a = y % 4;
    let b = y % 7;
    let c = y % 19;
    let d = (19 * c + 15) % 30;
    let e = (2 * a + 4 * b - d + 34) % 7;
    let month = (d + e + 114) / 31;
    let day = (d + e + 114) % 31 + 1;
    // Julian → Gregorian: 13 days from 1900 to 2099.
    let julian = Date::new(y, month as u32, day as u32).expect("easter");
    julian.add(if y < 2100 { 13 } else { 14 })
}
