//! Regression test on a frozen sample of the AI-generated proofing corpus
//! (assets/proofing/test-set.jsonl; see crates/xenify-wp-corpus). Fails if
//! the checker finds fewer mistakes or raises more false alarms than when
//! the sample was frozen. Thresholds sit a little below the measured values
//! so harmless changes don't trip it.

use xenify_grammar::{Category, Checker, Issue, Profiles, Settings};
use serde_json::Value;

fn items() -> Vec<Value> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/proofing/test-set.jsonl");
    std::fs::read_to_string(path).expect("test set").lines().map(|l| serde_json::from_str(l).expect("json")).collect()
}

fn checker(profile: &str) -> Checker {
    Checker::new(Settings { profile: Profiles::default().get(profile), ..Settings::default() }, &[])
}

fn check(c: &mut Checker, text: &str) -> Vec<Issue> {
    c.check(text)
}

fn is_error(i: &Issue) -> bool {
    matches!(i.category, Category::Spelling | Category::Grammar)
}

#[test]
fn standard_profile_finds_mistakes_without_false_alarms() {
    let mut c = checker("standard");
    let (mut n, mut caught, mut clean, mut false_alarms) = (0, 0, 0, 0);
    for it in items() {
        let Some(wrong) = it["mistakes"][0]["wrong"].as_str() else { continue };
        let text = it["text"].as_str().unwrap();
        let Some(start) = text.find(wrong) else { continue };
        let span = start..start + wrong.len();
        n += 1;
        if check(&mut c, text).iter().any(|i| {
            !matches!(i.category, Category::Readability | Category::Register)
                && i.range.start <= span.end + 1
                && span.start <= i.range.end + 1
        }) {
            caught += 1;
        }
        let corrected = it["corrected"].as_str().unwrap();
        clean += 1;
        if check(&mut c, corrected).iter().any(is_error) {
            false_alarms += 1;
        }
    }
    let recall = caught as f64 / n as f64;
    let fa = false_alarms as f64 / clean as f64;
    eprintln!("caught {caught}/{n} ({:.1}%), false alarms {false_alarms}/{clean} ({:.1}%)", 100.0 * recall, 100.0 * fa);
    assert!(recall >= 0.44, "recall fell to {:.1}%", 100.0 * recall);
    assert!(fa <= 0.05, "false alarms rose to {:.1}%", 100.0 * fa);
}

#[test]
fn casual_profile_leaves_informal_writing_alone() {
    let mut c = checker("casual");
    let (mut n, mut flagged) = (0, 0);
    for it in items() {
        if it.get("error_type").is_some() || it["register"] == "standard" {
            continue;
        }
        n += 1;
        let issues = check(&mut c, it["text"].as_str().unwrap());
        assert!(issues.iter().all(|i| i.category != Category::Register), "casual notes nothing: {:?}", it["text"]);
        if issues.iter().any(is_error) {
            flagged += 1;
        }
    }
    let rate = flagged as f64 / n as f64;
    eprintln!("informal passages with an error flag under casual: {flagged}/{n} ({:.0}%)", 100.0 * rate);
    assert!(rate <= 0.25, "{:.0}% of informal passages flagged", 100.0 * rate);
}

#[test]
fn academic_profile_notes_informal_writing() {
    let mut c = checker("academic");
    let (mut n, mut noted) = (0, 0);
    for it in items() {
        if it.get("error_type").is_some() || it["register"] == "standard" || it["register"] == "hedging_filler" {
            continue;
        }
        n += 1;
        if check(&mut c, it["text"].as_str().unwrap()).iter().any(|i| i.category == Category::Register) {
            noted += 1;
        }
    }
    eprintln!("informal passages noted under academic: {noted}/{n}");
    assert!(noted as f64 / n as f64 >= 0.8, "only {noted}/{n} informal passages noted");
}
