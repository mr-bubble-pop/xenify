//! Against a real CalDAV server (e.g. a local Radicale), with
//! `XENIFY_TEST_CALDAV=url|user|password`. Run with `--ignored`.

use xenify_cal::caldav::{Dav, DavError};

#[test]
#[ignore]
fn against_a_real_server() {
    let spec = std::env::var("XENIFY_TEST_CALDAV").expect("XENIFY_TEST_CALDAV=url|user|password");
    let mut p = spec.splitn(3, '|');
    let (url, user, pass) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
    assert_eq!(Dav::new(user, "wrong").discover(url).unwrap_err(), DavError::Auth);
    let dav = Dav::new(user, pass);
    let cals = dav.discover(url).unwrap();
    let cal = cals.iter().find(|c| c.name == "Work").unwrap_or(&cals[0]).clone();
    let start = dav.changes(&cal, "").unwrap();
    let uid = format!("xenify-test-{}", std::process::id());
    let ics = |summary: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Xenify//Test//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261016T120000Z\r\nDTSTART:20261023T100000Z\r\nDTEND:20261023T110000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    let item = format!("{}/{uid}.ics", cal.url.trim_end_matches('/'));
    let etag = dav.put(&item, &ics("First"), None).unwrap();
    assert_eq!(dav.put(&item, &ics("Again"), None).unwrap_err(), DavError::Conflict, "create only if new");
    let ch = dav.changes(&cal, &start.token).unwrap();
    assert!(ch.changed.iter().any(|i| i.url == item), "{ch:?}");
    let got = dav.get(&cal, std::slice::from_ref(&item)).unwrap();
    assert!(got[0].2.contains("SUMMARY:First"));
    let etag = etag.unwrap_or_else(|| got[0].1.clone());
    let etag2 = dav.put(&item, &ics("Second"), Some(&etag)).unwrap();
    assert_eq!(dav.put(&item, &ics("Stale"), Some(&etag)).unwrap_err(), DavError::Conflict, "an old ETag is refused");
    let etag2 = etag2.unwrap_or_else(|| dav.get(&cal, std::slice::from_ref(&item)).unwrap()[0].1.clone());
    dav.delete(&item, Some(&etag2)).unwrap();
    let ch2 = dav.changes(&cal, &ch.token).unwrap();
    assert!(ch2.removed.contains(&item) || ch.token.is_empty(), "{ch2:?}");
}

/// Two computers syncing one calendar through the server.
#[test]
#[ignore]
fn two_devices_stay_in_step() {
    use xenify_cal::event::{Event, When};
    use xenify_cal::store::Store;
    let spec = std::env::var("XENIFY_TEST_CALDAV").expect("XENIFY_TEST_CALDAV=url|user|password");
    let mut p = spec.splitn(3, '|');
    let (url, user, pass) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
    let dav = Dav::new(user, pass);
    let remote = dav.discover(url).unwrap().into_iter().find(|c| c.name == "Work").unwrap();
    let device = || {
        let mut s = Store::in_memory().unwrap();
        let id = s.add_calendar("Work", "caldav", &remote.url).unwrap();
        xenify_cal::davsync::sync(&dav, &mut s, id, &remote.url).unwrap();
        (s, id)
    };
    let (mut a, ca) = device();
    let (mut b, cb) = device();
    let uid = format!("two-devices-{}", std::process::id());
    let t = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 24, 9, 0, 0).unwrap();
    a.put(ca, &Event::new(&uid, "Planning", When::Utc(t), Some(When::Utc(t + chrono::Duration::hours(1))))).unwrap();
    assert_eq!(xenify_cal::davsync::sync(&dav, &mut a, ca, &remote.url).unwrap().pushed, 1);
    xenify_cal::davsync::sync(&dav, &mut b, cb, &remote.url).unwrap();
    let mut e = b.find(cb, &uid).unwrap().expect("B got it");
    assert_eq!(e.summary, "Planning");
    e.summary = "Planning (room 2)".into();
    b.put(cb, &e).unwrap();
    xenify_cal::davsync::sync(&dav, &mut b, cb, &remote.url).unwrap();
    xenify_cal::davsync::sync(&dav, &mut a, ca, &remote.url).unwrap();
    assert_eq!(a.find(ca, &uid).unwrap().unwrap().summary, "Planning (room 2)", "A sees B's change");
    // Both change it: the second to sync loses, and is told.
    let mut ea = a.find(ca, &uid).unwrap().unwrap();
    ea.summary = "From A".into();
    a.put(ca, &ea).unwrap();
    let mut eb = b.find(cb, &uid).unwrap().unwrap();
    eb.summary = "From B".into();
    b.put(cb, &eb).unwrap();
    xenify_cal::davsync::sync(&dav, &mut a, ca, &remote.url).unwrap();
    let r = xenify_cal::davsync::sync(&dav, &mut b, cb, &remote.url).unwrap();
    assert_eq!(r.conflicts, ["From B"]);
    assert_eq!(b.find(cb, &uid).unwrap().unwrap().summary, "From A", "the server's version kept");
    // Deleted on A: gone from B.
    a.delete(ca, &uid, None).unwrap();
    assert_eq!(xenify_cal::davsync::sync(&dav, &mut a, ca, &remote.url).unwrap().deleted, 1);
    xenify_cal::davsync::sync(&dav, &mut b, cb, &remote.url).unwrap();
    assert!(b.find(cb, &uid).unwrap().is_none());
}
