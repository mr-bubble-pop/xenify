//! Turning events (some repeating) into the occurrences shown on a calendar.

use crate::event::{Event, When};
use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;

/// One appearance of an event.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence<'a> {
    pub event: &'a Event,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Most occurrences of one event listed per request (a daily event over
/// many years would otherwise be huge).
const LIMIT: u16 = 2000;

/// The occurrences overlapping `from..to`, in start order. `local` is the
/// zone for all-day and floating times. Cancelled events and occurrences
/// are left out; a changed occurrence (same UID with RECURRENCE-ID)
/// replaces the one it changes.
pub fn occurrences<'a>(events: &'a [Event], from: DateTime<Utc>, to: DateTime<Utc>, local: Tz) -> Vec<Occurrence<'a>> {
    let mut out = Vec::new();
    // Changed occurrences, by UID and the start they replace.
    let mut changed: HashMap<&str, Vec<DateTime<Utc>>> = HashMap::new();
    for e in events {
        if let Some(r) = &e.recurrence_id {
            changed.entry(e.uid.as_str()).or_default().push(r.instant(local));
        }
    }
    for e in events {
        let dur = e.duration(local);
        let overlaps = |s: DateTime<Utc>| s < to && s + dur.max(Duration::seconds(1)) > from;
        if e.recurrence_id.is_some() {
            let s = e.start.instant(local);
            if !e.cancelled() && overlaps(s) {
                out.push(Occurrence { event: e, start: s, end: s + dur });
            }
            continue;
        }
        if e.cancelled() {
            continue;
        }
        let skip = changed.get(e.uid.as_str()).cloned().unwrap_or_default();
        let excluded: Vec<DateTime<Utc>> = e.exdates.iter().map(|d| d.instant(local)).collect();
        for s in starts(e, from - dur, to, local) {
            if !skip.contains(&s) && !excluded.contains(&s) && overlaps(s) {
                out.push(Occurrence { event: e, start: s, end: s + dur });
            }
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.event.summary.cmp(&b.event.summary)));
    out
}

/// Start instants of `e` between `from` and `to` (before exclusions).
fn starts(e: &Event, from: DateTime<Utc>, to: DateTime<Utc>, local: Tz) -> Vec<DateTime<Utc>> {
    let first = e.start.instant(local);
    let mut v: Vec<DateTime<Utc>> = match &e.rrule {
        None => vec![first],
        Some(rule) => match expand_rule(e, rule, from, to, local) {
            Some(list) => list,
            // A rule we can't read: show the first occurrence rather than none.
            None => vec![first],
        },
    };
    v.extend(e.rdates.iter().map(|d| d.instant(local)));
    v.sort();
    v.dedup();
    v
}

fn expand_rule(e: &Event, rule: &str, from: DateTime<Utc>, to: DateTime<Utc>, local: Tz) -> Option<Vec<DateTime<Utc>>> {
    // The rule runs in the event's own zone, so 9:00 stays 9:00 across DST.
    let (zone, naive) = match e.start {
        When::Zoned(t, tz) => (rrule::Tz::Tz(tz), t),
        When::Utc(t) => (rrule::Tz::UTC, t.naive_utc()),
        When::Floating(t) => (rrule::Tz::Tz(local), t),
        When::Date(d) => (rrule::Tz::Tz(local), d.and_time(chrono::NaiveTime::MIN)),
    };
    let start = zone.from_local_datetime(&naive).earliest()?;
    let rule = fix_until(rule);
    let parsed: rrule::RRule<rrule::Unvalidated> = rule.parse().ok()?;
    let set = parsed.build(start).ok()?;
    let to_zone = |t: DateTime<Utc>| t.with_timezone(&zone);
    let result = set.after(to_zone(from - Duration::seconds(1))).before(to_zone(to)).all(LIMIT);
    Some(result.dates.into_iter().map(|d| d.with_timezone(&Utc)).collect())
}

/// The rule is expanded from a date-and-time start (all-day events start at
/// midnight), so UNTIL must be a UTC date-time: a date becomes the end of
/// that day; a time without a zone is taken as UTC. (Senders differ here.)
fn fix_until(rule: &str) -> String {
    rule.split(';')
        .map(|part| {
            let Some(v) = part.strip_prefix("UNTIL=") else { return part.to_string() };
            let v = v.trim();
            match v.len() {
                8 => format!("UNTIL={v}T235959Z"),
                _ if !v.ends_with('Z') => format!("UNTIL={v}Z"),
                _ => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Midnight at the start of `day` in `zone`, as UTC (for day/week/month ranges).
pub fn day_start(day: chrono::NaiveDate, zone: Tz) -> DateTime<Utc> {
    zone.from_local_datetime(&day.and_time(chrono::NaiveTime::MIN))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&day.and_time(chrono::NaiveTime::MIN)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::events_in;
    use chrono::NaiveDate;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn cal(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    #[test]
    fn weekly_meeting_keeps_its_time_across_dst_and_honours_changes() {
        let text = cal(
            "BEGIN:VEVENT\r\nUID:standup\r\nSUMMARY:Stand-up\r\nDTSTART;TZID=Europe/London:20261019T093000\r\nDTEND;TZID=Europe/London:20261019T094500\r\nRRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4\r\nEXDATE;TZID=Europe/London:20261102T093000\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:standup\r\nRECURRENCE-ID;TZID=Europe/London:20261026T093000\r\nSUMMARY:Stand-up (moved)\r\nDTSTART;TZID=Europe/London:20261026T110000\r\nDTEND;TZID=Europe/London:20261026T111500\r\nEND:VEVENT\r\n",
        );
        let events = events_in(&text);
        let occ = occurrences(&events, utc("2026-10-01T00:00:00Z"), utc("2026-12-01T00:00:00Z"), Tz::UTC);
        let got: Vec<(String, String)> = occ.iter().map(|o| (o.start.to_rfc3339(), o.event.summary.clone())).collect();
        assert_eq!(
            got,
            [
                ("2026-10-19T08:30:00+00:00".to_string(), "Stand-up".to_string()), // BST: UTC+1
                ("2026-10-26T11:00:00+00:00".into(), "Stand-up (moved)".into()),
                // 2 Nov excluded; 9 Nov is after the clocks change: still 9:30 in London.
                ("2026-11-09T09:30:00+00:00".into(), "Stand-up".into()),
            ]
        );
        assert_eq!(occ[0].end - occ[0].start, Duration::minutes(15));
    }

    #[test]
    fn all_day_monthly_and_outlook_until() {
        let text = cal(
            "BEGIN:VEVENT\r\nUID:pay\r\nSUMMARY:Payday\r\nDTSTART;VALUE=DATE:20260131\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-1;UNTIL=20260430T000000\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Cancelled thing\r\nSTATUS:CANCELLED\r\nDTSTART:20260201T100000Z\r\nEND:VEVENT\r\n",
        );
        let events = events_in(&text);
        let local = Tz::Australia__Sydney;
        let occ = occurrences(&events, utc("2026-01-01T00:00:00Z"), utc("2026-12-31T00:00:00Z"), local);
        let days: Vec<NaiveDate> = occ.iter().map(|o| o.start.with_timezone(&local).date_naive()).collect();
        assert_eq!(
            days,
            [
                NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 30).unwrap(),
            ],
            "last day of each month until 30 April (UNTIL includes it), cancelled event left out"
        );
        assert_eq!(occ[0].end - occ[0].start, Duration::days(1));
    }

    #[test]
    fn a_rule_we_cannot_read_still_shows_the_first_time() {
        let text = cal(
            "BEGIN:VEVENT\r\nUID:odd\r\nSUMMARY:Odd\r\nDTSTART:20260301T100000Z\r\nRRULE:FREQ=SOMETIMES\r\nEND:VEVENT\r\n",
        );
        let events = events_in(&text);
        let occ = occurrences(&events, utc("2026-01-01T00:00:00Z"), utc("2026-12-31T00:00:00Z"), Tz::UTC);
        assert_eq!(occ.len(), 1);
    }
}
