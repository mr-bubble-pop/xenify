//! Events (VEVENT): when, what, where, who, how it repeats and reminders.

use crate::ics::{self, Component, Property};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

/// A moment as an invitation states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// A whole day (all-day events).
    Date(NaiveDate),
    /// A fixed instant ("…Z").
    Utc(DateTime<Utc>),
    /// A wall-clock time in a time zone (keeps 9:00 at 9:00 across DST).
    Zoned(NaiveDateTime, Tz),
    /// "Whatever the local time is" (no zone given).
    Floating(NaiveDateTime),
}

impl When {
    /// The instant, with floating and all-day times taken in `local`.
    pub fn instant(&self, local: Tz) -> DateTime<Utc> {
        let resolve = |tz: Tz, t: NaiveDateTime| {
            tz.from_local_datetime(&t)
                .earliest()
                // A time skipped by DST: an hour later is what clocks show.
                .or_else(|| tz.from_local_datetime(&(t + Duration::hours(1))).earliest())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|| Utc.from_utc_datetime(&t))
        };
        match *self {
            When::Date(d) => resolve(local, d.and_time(NaiveTime::MIN)),
            When::Utc(t) => t,
            When::Zoned(t, tz) => resolve(tz, t),
            When::Floating(t) => resolve(local, t),
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(self, When::Date(_))
    }

    /// Reads DTSTART/DTEND/RECURRENCE-ID/EXDATE values.
    pub fn from_prop(p: &Property) -> Option<When> {
        let v = p.value.split(',').next()?.trim();
        if p.param("VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) || (v.len() == 8 && !v.contains('T')) {
            return NaiveDate::parse_from_str(v, "%Y%m%d").ok().map(When::Date);
        }
        let (t, utc) = match v.strip_suffix(['Z', 'z']) {
            Some(t) => (t, true),
            None => (v, false),
        };
        let t = NaiveDateTime::parse_from_str(t, "%Y%m%dT%H%M%S")
            .or_else(|_| NaiveDateTime::parse_from_str(t, "%Y%m%dT%H%M"))
            .ok()?;
        if utc {
            return Some(When::Utc(Utc.from_utc_datetime(&t)));
        }
        match p.param("TZID") {
            Some(id) => Some(match crate::tz::resolve(id) {
                Some(tz) => When::Zoned(t, tz),
                // An unknown zone: read as local time rather than drop it.
                None => When::Floating(t),
            }),
            None => Some(When::Floating(t)),
        }
    }

    pub fn to_prop(&self, name: &str) -> Property {
        match self {
            When::Date(d) => Property::new(name, d.format("%Y%m%d").to_string()).with("VALUE", "DATE"),
            When::Utc(t) => Property::new(name, t.format("%Y%m%dT%H%M%SZ").to_string()),
            When::Zoned(t, tz) => Property::new(name, t.format("%Y%m%dT%H%M%S").to_string()).with("TZID", tz.name()),
            When::Floating(t) => Property::new(name, t.format("%Y%m%dT%H%M%S").to_string()),
        }
    }
}

/// How someone has answered an invitation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Answer {
    #[default]
    NeedsAction,
    Accepted,
    Tentative,
    Declined,
    Delegated,
}

impl Answer {
    pub fn parse(s: &str) -> Answer {
        match s.to_ascii_uppercase().as_str() {
            "ACCEPTED" => Answer::Accepted,
            "TENTATIVE" => Answer::Tentative,
            "DECLINED" => Answer::Declined,
            "DELEGATED" => Answer::Delegated,
            _ => Answer::NeedsAction,
        }
    }
    pub fn ics(self) -> &'static str {
        match self {
            Answer::NeedsAction => "NEEDS-ACTION",
            Answer::Accepted => "ACCEPTED",
            Answer::Tentative => "TENTATIVE",
            Answer::Declined => "DECLINED",
            Answer::Delegated => "DELEGATED",
        }
    }
    /// "Accepted", "Maybe"…
    pub fn label(self) -> &'static str {
        match self {
            Answer::NeedsAction => "No answer yet",
            Answer::Accepted => "Going",
            Answer::Tentative => "Maybe",
            Answer::Declined => "Not going",
            Answer::Delegated => "Passed on",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub email: String,
    pub name: String,
    pub answer: Answer,
    /// Optional rather than required attendance.
    pub optional: bool,
    /// Asked to reply.
    pub rsvp: bool,
}

impl Person {
    fn from_prop(p: &Property) -> Option<Person> {
        let email = p.value.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").trim().to_string();
        if email.is_empty() {
            return None;
        }
        Some(Person {
            name: p.param("CN").unwrap_or("").to_string(),
            answer: p.param("PARTSTAT").map(Answer::parse).unwrap_or_default(),
            optional: p.param("ROLE").is_some_and(|r| r.eq_ignore_ascii_case("OPT-PARTICIPANT")),
            rsvp: p.param("RSVP").is_some_and(|r| r.eq_ignore_ascii_case("TRUE")),
            email,
        })
    }

    fn to_prop(&self, name: &str, attendee: bool) -> Property {
        let mut p = Property::new(name, format!("mailto:{}", self.email));
        if !self.name.is_empty() {
            p = p.with("CN", &self.name);
        }
        if attendee {
            p = p.with("ROLE", if self.optional { "OPT-PARTICIPANT" } else { "REQ-PARTICIPANT" });
            p = p.with("PARTSTAT", self.answer.ics());
            if self.rsvp {
                p = p.with("RSVP", "TRUE");
            }
        }
        p
    }

    /// "Sam Lee <sam@x.org>" or just the address.
    pub fn display(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { format!("{} <{}>", self.name, self.email) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: When,
    /// None: DTEND/DURATION missing (an instant, or one day when all-day).
    pub end: Option<When>,
    /// RRULE, as written.
    pub rrule: Option<String>,
    pub exdates: Vec<When>,
    pub rdates: Vec<When>,
    /// This is a changed copy of one occurrence of a repeating event.
    pub recurrence_id: Option<When>,
    pub sequence: u32,
    /// CONFIRMED, TENTATIVE or CANCELLED.
    pub status: String,
    pub organizer: Option<Person>,
    pub attendees: Vec<Person>,
    /// Reminders, in minutes before the start.
    pub alarms: Vec<i64>,
    /// An online meeting link, when the invitation gives one.
    pub url: String,
    /// Shown as free time (TRANSP:TRANSPARENT).
    pub free: bool,
    /// Last change (DTSTAMP/LAST-MODIFIED), as written.
    pub stamp: String,
}

impl Event {
    pub fn new(uid: impl Into<String>, summary: impl Into<String>, start: When, end: Option<When>) -> Event {
        Event {
            uid: uid.into(),
            summary: summary.into(),
            description: String::new(),
            location: String::new(),
            start,
            end,
            rrule: None,
            exdates: Vec::new(),
            rdates: Vec::new(),
            recurrence_id: None,
            sequence: 0,
            status: "CONFIRMED".into(),
            organizer: None,
            attendees: Vec::new(),
            alarms: Vec::new(),
            url: String::new(),
            free: false,
            stamp: String::new(),
        }
    }

    pub fn all_day(&self) -> bool {
        self.start.is_date()
    }

    pub fn cancelled(&self) -> bool {
        self.status.eq_ignore_ascii_case("CANCELLED")
    }

    /// How long it lasts (one day for all-day events without an end).
    pub fn duration(&self, local: Tz) -> Duration {
        match self.end {
            Some(e) => (e.instant(local) - self.start.instant(local)).max(Duration::zero()),
            None if self.all_day() => Duration::days(1),
            None => Duration::zero(),
        }
    }

    pub fn from_component(c: &Component) -> Option<Event> {
        let start = When::from_prop(c.prop("DTSTART")?)?;
        let end = c.prop("DTEND").or_else(|| c.prop("DUE")).and_then(When::from_prop).or_else(|| {
            let d = parse_duration(&c.prop("DURATION")?.value)?;
            Some(match start {
                When::Date(day) => When::Date(day + d),
                When::Utc(t) => When::Utc(t + d),
                When::Zoned(t, tz) => When::Zoned(t + d, tz),
                When::Floating(t) => When::Floating(t + d),
            })
        });
        let dates = |name: &str| -> Vec<When> {
            c.props_named(name)
                .flat_map(|p| {
                    p.value
                        .split(',')
                        .filter_map(|v| When::from_prop(&Property { value: v.to_string(), ..p.clone() }))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        Some(Event {
            uid: c.text("UID").unwrap_or_default(),
            summary: c.text("SUMMARY").unwrap_or_default(),
            description: c.text("DESCRIPTION").unwrap_or_default(),
            location: c.text("LOCATION").unwrap_or_default(),
            start,
            end,
            rrule: c.prop("RRULE").map(|p| p.value.trim().to_string()),
            exdates: dates("EXDATE"),
            rdates: dates("RDATE"),
            recurrence_id: c.prop("RECURRENCE-ID").and_then(When::from_prop),
            sequence: c.prop("SEQUENCE").and_then(|p| p.value.trim().parse().ok()).unwrap_or(0),
            status: c.text("STATUS").unwrap_or_else(|| "CONFIRMED".into()).to_ascii_uppercase(),
            organizer: c.prop("ORGANIZER").and_then(Person::from_prop),
            attendees: c.props_named("ATTENDEE").filter_map(Person::from_prop).collect(),
            alarms: c
                .children_named("VALARM")
                .filter_map(|a| {
                    let t = a.prop("TRIGGER")?;
                    // Only reminders before the start (the usual kind).
                    let d = parse_duration(&t.value)?;
                    Some((-d.num_minutes()).max(0))
                })
                .collect(),
            url: c.text("URL").unwrap_or_default(),
            free: c.text("TRANSP").is_some_and(|t| t.eq_ignore_ascii_case("TRANSPARENT")),
            stamp: c.text("LAST-MODIFIED").or_else(|| c.text("DTSTAMP")).unwrap_or_default(),
        })
    }

    pub fn to_component(&self) -> Component {
        let mut c = Component::new("VEVENT");
        c.push(Property::new("UID", ics::escape(&self.uid)));
        c.push(Property::new("DTSTAMP", Utc::now().format("%Y%m%dT%H%M%SZ").to_string()));
        c.push(self.start.to_prop("DTSTART"));
        if let Some(e) = &self.end {
            c.push(e.to_prop("DTEND"));
        }
        if let Some(r) = &self.recurrence_id {
            c.push(r.to_prop("RECURRENCE-ID"));
        }
        c.push(Property::new("SUMMARY", ics::escape(&self.summary)));
        for (name, v) in [("DESCRIPTION", &self.description), ("LOCATION", &self.location), ("URL", &self.url)] {
            if !v.is_empty() {
                c.push(Property::new(name, ics::escape(v)));
            }
        }
        if let Some(r) = &self.rrule {
            c.push(Property::new("RRULE", r.clone()));
        }
        for d in &self.exdates {
            c.push(d.to_prop("EXDATE"));
        }
        for d in &self.rdates {
            c.push(d.to_prop("RDATE"));
        }
        c.push(Property::new("SEQUENCE", self.sequence.to_string()));
        c.push(Property::new("STATUS", self.status.clone()));
        if self.free {
            c.push(Property::new("TRANSP", "TRANSPARENT"));
        }
        if let Some(o) = &self.organizer {
            c.push(o.to_prop("ORGANIZER", false));
        }
        for a in &self.attendees {
            c.push(a.to_prop("ATTENDEE", true));
        }
        for m in &self.alarms {
            let mut a = Component::new("VALARM");
            a.push(Property::new("ACTION", "DISPLAY"));
            a.push(Property::new("DESCRIPTION", ics::escape(&self.summary)));
            a.push(Property::new("TRIGGER", format!("-PT{m}M")));
            c.children.push(a);
        }
        c
    }
}

/// `P1D`, `PT1H30M`, `-PT15M`, `P2W` → Duration.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c.to_ascii_uppercase() {
            'T' => in_time = true,
            d if d.is_ascii_digit() => num.push(d),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', _) => Duration::weeks(n),
                    ('D', _) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}

/// Every event in iCalendar text (all VCALENDARs).
pub fn events_in(text: &str) -> Vec<Event> {
    ics::parse(text)
        .map(|cals| cals.iter().flat_map(|c| c.children_named("VEVENT")).filter_map(Event::from_component).collect())
        .unwrap_or_default()
}

/// A calendar file holding `events` (with `method` for invitations).
pub fn calendar(events: &[Event], method: Option<&str>) -> String {
    let mut cal = Component::new("VCALENDAR");
    cal.push(Property::new("VERSION", "2.0"));
    cal.push(Property::new("PRODID", "-//Xenify//Xenify Mail//EN"));
    cal.push(Property::new("CALSCALE", "GREGORIAN"));
    if let Some(m) = method {
        cal.push(Property::new("METHOD", m));
    }
    cal.children.extend(events.iter().map(Event::to_component));
    cal.to_ics()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_duration("P1W"), Some(Duration::days(7)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("1H"), None);
    }

    #[test]
    fn times_in_all_four_forms() {
        let p = |s: &str| {
            When::from_prop(
                &crate::ics::parse(&format!("BEGIN:VCALENDAR\r\n{s}\r\nEND:VCALENDAR")).unwrap()[0].props[0],
            )
            .unwrap()
        };
        let berlin = p("DTSTART;TZID=W. Europe Standard Time:20261020T090000");
        assert_eq!(berlin.instant(Tz::UTC).to_rfc3339(), "2026-10-20T07:00:00+00:00", "summer time: UTC+2");
        let winter = p("DTSTART;TZID=Europe/Berlin:20261120T090000");
        assert_eq!(winter.instant(Tz::UTC).to_rfc3339(), "2026-11-20T08:00:00+00:00", "winter: UTC+1");
        assert_eq!(p("DTSTART:20261020T090000Z").instant(Tz::UTC).to_rfc3339(), "2026-10-20T09:00:00+00:00");
        assert!(p("DTSTART;VALUE=DATE:20261020").is_date());
        assert!(matches!(p("DTSTART:20261020T090000"), When::Floating(_)));
        assert!(matches!(p("DTSTART;TZID=Mars/Olympus:20261020T090000"), When::Floating(_)));
    }
}
