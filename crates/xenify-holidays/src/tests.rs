use super::*;

fn d(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn days(c: &str, r: &str, y: i32) -> Vec<String> {
    holidays(c, r, y).into_iter().map(|h| h.date.to_string()).collect()
}

#[test]
fn dates_round_trip() {
    for z in -800_000..800_000 {
        let (y, m, dd) = Date(z).ymd();
        assert_eq!(Date::new(y, m, dd), Some(Date(z)));
    }
    assert_eq!(d("2026-11-16").weekday(), 0);
    assert_eq!(d("2026-12-25").weekday(), 4);
    assert_eq!(Date::new(2026, 2, 29), None);
}

#[test]
fn easters() {
    assert_eq!(easter(2026), d("2026-04-05"));
    assert_eq!(easter(2027), d("2027-03-28"));
    assert_eq!(orthodox_easter(2026), d("2026-04-12"));
    assert_eq!(orthodox_easter(2027), d("2027-05-02"));
}

#[test]
fn england_2026() {
    assert_eq!(
        days("GB", "ENG", 2026),
        [
            "2026-01-01",
            "2026-04-03",
            "2026-04-06",
            "2026-05-04",
            "2026-05-25",
            "2026-08-31",
            "2026-12-25",
            "2026-12-26",
            "2026-12-28"
        ]
    );
    let boxing = holidays("GB", "ENG", 2026).pop().unwrap();
    assert_eq!(boxing.moved_from, Some(d("2026-12-26")));
}

#[test]
fn scotland_has_its_own() {
    let s = days("GB", "SCT", 2026);
    assert!(s.contains(&"2026-11-30".to_string()), "{s:?}");
    assert!(s.contains(&"2026-01-02".to_string()));
    assert!(!s.contains(&"2026-04-06".to_string()), "no Easter Monday in Scotland");
}

#[test]
fn christmas_pair_on_a_weekend() {
    // 2021: Christmas Saturday, Boxing Day Sunday → Monday 27 and Tuesday 28.
    let s = days("GB", "ENG", 2021);
    assert!(s.contains(&"2021-12-27".to_string()) && s.contains(&"2021-12-28".to_string()), "{s:?}");
}

#[test]
fn us_nearest_weekday() {
    let s = days("US", "", 2026);
    assert!(s.contains(&"2026-07-03".to_string()), "Independence Day on Saturday → Friday: {s:?}");
    assert!(s.contains(&"2026-11-26".to_string()), "Thanksgiving");
    // New Year 2022 was a Saturday: observed Friday 31 Dec 2021.
    assert!(days("US", "", 2021).contains(&"2021-12-31".to_string()));
}

#[test]
fn every_file_parses_and_has_days() {
    for c in countries() {
        assert!(!holidays(&c.code, "", 2027).is_empty() || !c.regions.is_empty(), "{}", c.code);
        for (r, _) in &c.regions {
            holidays(&c.code, r, 2027);
        }
    }
    assert!(countries().len() >= 27);
}

#[test]
fn cached_lookup() {
    let days = Days::new("gb", "ENG");
    assert_eq!(days.name(d("2026-12-25")).as_deref(), Some("Christmas Day"));
    assert!(days.is_holiday(d("2026-12-28")));
    assert!(!days.is_holiday(d("2026-12-29")));
}
