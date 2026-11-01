//! Dates and times written out in a format like Word's: `d MMMM yyyy` is
//! "19 October 2026", `HH:mm` is "14:05", `h:mm am/pm` is "2:05 pm".
//!
//! | Code | Means |
//! |---|---|
//! | `d` `dd` | day (19, 09) |
//! | `ddd` `dddd` | weekday (Mon, Monday) |
//! | `M` `MM` `MMM` `MMMM` | month (10, 10, Oct, October) |
//! | `yy` `yyyy` | year (26, 2026) |
//! | `H` `HH` | hour, 24-hour clock |
//! | `h` `hh` | hour, 12-hour clock |
//! | `mm` `ss` | minutes, seconds |
//! | `am/pm` `AM/PM` | morning or afternoon |
//!
//! Anything else is written as it is; text in 'quotes' too.

/// A moment, in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateParts {
    pub year: i32,
    /// 1–12.
    pub month: u8,
    /// 1–31.
    pub day: u8,
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// The formats offered in Insert › Date and time (most used first).
pub const PRESETS: &[&str] = &[
    "d MMMM yyyy",
    "dddd d MMMM yyyy",
    "dd/MM/yyyy",
    "MM/dd/yyyy",
    "yyyy-MM-dd",
    "d MMM yyyy",
    "MMMM d, yyyy",
    "HH:mm",
    "h:mm am/pm",
    "d MMMM yyyy, HH:mm",
];

/// `at` written in `format`.
pub fn format(format: &str, at: &DateParts) -> String {
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // Quoted text, as it is.
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        let rest: String = chars[i..].iter().collect();
        for (marker, upper) in [("am/pm", false), ("AM/PM", true)] {
            if rest.starts_with(marker) {
                let s = if at.hour < 12 { "am" } else { "pm" };
                out.push_str(&if upper { s.to_uppercase() } else { s.to_string() });
                i += marker.len();
            }
        }
        if i >= chars.len() || chars[i] != c {
            continue;
        }
        let run = chars[i..].iter().take_while(|&&x| x == c).count();
        let h12 = match at.hour % 12 {
            0 => 12,
            h => h,
        };
        let piece = match (c, run) {
            ('d', 1) => at.day.to_string(),
            ('d', 2) => format!("{:02}", at.day),
            ('d', 3) => DAYS[at.weekday as usize % 7][..3].to_string(),
            ('d', _) => DAYS[at.weekday as usize % 7].to_string(),
            ('M', 1) => at.month.to_string(),
            ('M', 2) => format!("{:02}", at.month),
            ('M', 3) => MONTHS[(at.month as usize + 11) % 12][..3].to_string(),
            ('M', _) => MONTHS[(at.month as usize + 11) % 12].to_string(),
            ('y', 1 | 2) => format!("{:02}", at.year.rem_euclid(100)),
            ('y', _) => at.year.to_string(),
            ('H', 1) => at.hour.to_string(),
            ('H', _) => format!("{:02}", at.hour),
            ('h', 1) => h12.to_string(),
            ('h', _) => format!("{h12:02}"),
            ('m', _) => format!("{:02}", at.minute),
            ('s', _) => format!("{:02}", at.second),
            _ => chars[i..i + run].iter().collect(),
        };
        out.push_str(&piece);
        i += run;
    }
    out
}

/// The local date and time now.
pub fn now() -> DateParts {
    let z = jiff::Zoned::now();
    DateParts {
        year: z.year() as i32,
        month: z.month() as u8,
        day: z.day() as u8,
        weekday: z.weekday().to_monday_zero_offset() as u8,
        hour: z.hour() as u8,
        minute: z.minute() as u8,
        second: z.second() as u8,
    }
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
pub fn civil(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m as u8, d as u8)
}

impl DateParts {
    /// The moment `secs` after 1970 began, `offset` seconds ahead of UTC.
    pub fn from_unix(secs: i64, offset: i64) -> Self {
        let t = secs + offset;
        let days = t.div_euclid(86_400);
        let s = t.rem_euclid(86_400);
        let (year, month, day) = civil(days);
        // 1970-01-01 was a Thursday.
        let weekday = (days + 3).rem_euclid(7) as u8;
        DateParts {
            year,
            month,
            day,
            weekday,
            hour: (s / 3600) as u8,
            minute: (s / 60 % 60) as u8,
            second: (s % 60) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        // 2026-10-19 14:05:09, a Monday.
        let at = DateParts::from_unix(1_792_418_709, 0);
        assert_eq!(at, DateParts { year: 2026, month: 10, day: 19, weekday: 0, hour: 14, minute: 5, second: 9 });
        assert_eq!(format("d MMMM yyyy", &at), "19 October 2026");
        assert_eq!(format("dddd d MMMM yyyy", &at), "Monday 19 October 2026");
        assert_eq!(format("dd/MM/yy", &at), "19/10/26");
        assert_eq!(format("MMMM d, yyyy", &at), "October 19, 2026");
        assert_eq!(format("HH:mm", &at), "14:05");
        assert_eq!(format("h:mm am/pm", &at), "2:05 pm");
        assert_eq!(format("'Updated' d MMM", &at), "Updated 19 Oct");
    }
}
