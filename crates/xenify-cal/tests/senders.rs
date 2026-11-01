//! Invitations shaped like the ones Outlook, Google Calendar, Apple
//! Calendar and Zoom send (hand-written, no real data).

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use xenify_cal::event::{Answer, events_in};
use xenify_cal::expand::occurrences;

fn read(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn outlook_invitation() {
    let ev = &events_in(&read("outlook-request.ics"))[0];
    assert_eq!(ev.summary, "Q4 planning");
    assert_eq!(ev.location, "Room 4 / Online");
    assert_eq!(ev.description, "Quarterly planning.\n\nAgenda to follow.\n");
    assert!(ev.uid.starts_with("040000008200E00074C5B7101A82E008"), "folded UID joined");
    // "GMT Standard Time" is London: 14:00 BST = 13:00 UTC.
    assert_eq!(ev.start.instant(Tz::UTC), utc("2026-10-22T13:00:00Z"));
    assert_eq!(ev.duration(Tz::UTC), Duration::hours(1));
    assert_eq!(ev.organizer.as_ref().unwrap().email, "pat@example.com");
    assert_eq!(ev.attendees.len(), 2);
    assert!(ev.attendees[1].optional);
    assert_eq!(ev.attendees[0].answer, Answer::NeedsAction);
    assert_eq!(ev.alarms, [15]);
}

#[test]
fn google_weekly_until_end_of_november() {
    let events = events_in(&read("google-weekly.ics"));
    let occ = occurrences(&events, utc("2026-10-01T00:00:00Z"), utc("2027-01-01T00:00:00Z"), Tz::UTC);
    let starts: Vec<String> = occ.iter().map(|o| o.start.to_rfc3339()).collect();
    assert_eq!(
        starts,
        [
            "2026-10-27T14:00:00+00:00", // EDT
            "2026-11-03T15:00:00+00:00", // EST after 1 Nov: still 10:00 in New York
            "2026-11-10T15:00:00+00:00",
            "2026-11-17T15:00:00+00:00",
            // 24 Nov 10:00 EST is 15:00 UTC, after UNTIL (04:59:59 UTC): not included.
        ]
    );
    assert_eq!(occ[0].event.attendees[1].email, "sam@example.org");
}

#[test]
fn apple_two_day_all_day_event() {
    let ev = &events_in(&read("apple-allday.ics"))[0];
    assert!(ev.all_day());
    assert!(ev.free, "shown as free");
    let local = Tz::Europe__Berlin;
    assert_eq!(ev.duration(local), Duration::days(2));
    let occ = occurrences(std::slice::from_ref(ev), utc("2026-11-02T12:00:00Z"), utc("2026-11-02T13:00:00Z"), local);
    assert_eq!(occ.len(), 1, "still on during its second day");
    assert_eq!(occ[0].start.with_timezone(&local).date_naive(), NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
}

#[test]
fn zoom_utc_times_and_link() {
    let ev = &events_in(&read("zoom-utc.ics"))[0];
    assert_eq!(ev.start.instant(Tz::Europe__Paris), utc("2026-10-21T16:00:00Z"));
    assert!(ev.location.starts_with("https://"));
    assert_eq!(ev.alarms, [10]);
}

#[test]
fn written_events_read_back_the_same() {
    for f in ["outlook-request.ics", "google-weekly.ics", "apple-allday.ics", "zoom-utc.ics"] {
        let events = events_in(&read(f));
        let again = events_in(&xenify_cal::event::calendar(&events, None));
        assert_eq!(events.len(), again.len(), "{f}");
        for (a, b) in events.iter().zip(&again) {
            let mut b = b.clone();
            b.stamp = a.stamp.clone();
            assert_eq!(*a, b, "{f}");
        }
    }
}
