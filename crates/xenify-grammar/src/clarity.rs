//! Clarity checks for work writing (emails, technical documents): long
//! noun stacks, a long lead-in before the main point, sentences doing too
//! much, a request buried at the end of an email, and acronyms used before
//! they're spelled out. All are gentle style notes, switched by the profile.

use crate::{Category, Issue, Local, Profile};
use std::collections::HashSet;

fn note(range: std::ops::Range<usize>, rule: &str, message: String, suggestions: Vec<String>) -> Local {
    Local { range, category: Category::Style, rule: rule.into(), message, suggestions, feature: None, strong: false }
}

/// Sentences of `text` as byte ranges.
pub fn sentences(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = text.as_bytes();
    for (i, c) in text.char_indices() {
        if matches!(c, '.' | '!' | '?')
            && (i + 1 == text.len() || b.get(i + 1).is_some_and(|n| n.is_ascii_whitespace()))
        {
            out.push(start..i + 1);
            start = i + 1;
        }
    }
    if text[start..].trim().len() > 1 {
        out.push(start..text.len());
    }
    out.into_iter()
        .map(|r| {
            let lead = text[r.clone()].len() - text[r.clone()].trim_start().len();
            r.start + lead..r.end
        })
        .filter(|r| !r.is_empty())
        .collect()
}

const LEAD_INS: &[&str] = &[
    "because",
    "due",
    "as",
    "since",
    "although",
    "though",
    "when",
    "if",
    "while",
    "whilst",
    "given",
    "following",
    "after",
    "before",
    "in",
    "with",
    "considering",
    "regarding",
    "despite",
    "once",
    "until",
    "unless",
    "whereas",
    "based",
    "according",
];

/// Paragraph checks.
pub(crate) fn check(text: &str, p: &Profile) -> Vec<Local> {
    let mut out = Vec::new();
    if !p.clarity {
        return out;
    }
    fillers(text, &mut out);
    list_after_comma(text, &mut out);
    vague_it(text, &mut out);
    let toks = crate::sound_alike::tagged(text);
    // Noun stacks: four or more nouns in a row ("database migration rollback strategy document").
    let mut k = 0;
    while k < toks.len() {
        // Nouns, with -ing modifiers between them ("artifact caching layer").
        let mut e = k;
        let mut nouns = 0;
        while e < toks.len()
            && (toks[e].2 == "NOUN"
                || toks[e].1.ends_with("ing") && nouns > 0 && toks.get(e + 1).is_some_and(|n| n.2 == "NOUN"))
        {
            nouns += usize::from(toks[e].2 == "NOUN");
            e += 1;
        }
        if e - k >= 4 && nouns >= 3 {
            let r = toks[k].0.start..toks[e - 1].0.end;
            let head = &toks[e - 1].1;
            out.push(note(
                r,
                "NounStack",
                format!("{} nouns in a row are hard to unpack. Try leading with the main noun (`{head}`) and adding the rest with of/for/that.", e - k),
                Vec::new(),
            ));
        }
        k = e.max(k + 1);
    }
    for s in sentences(text) {
        let sent = &text[s.clone()];
        let words: Vec<&str> = sent.split_whitespace().collect();
        // A long lead-in before the main point.
        let first = words.first().map(|w| w.to_lowercase()).unwrap_or_default();
        if LEAD_INS.contains(&first.trim_matches(|c: char| !c.is_alphanumeric())) {
            let before_comma = sent.find(',').map(|i| sent[..i].split_whitespace().count());
            if let Some(n) = before_comma.filter(|n| *n >= 15) {
                let end = s.start + sent.find(',').unwrap_or(0);
                out.push(note(
                    s.start..end,
                    "LongLeadIn",
                    format!("The main point waits until after a {n}-word lead-in. Try starting with the main point and putting the reason or condition after it."),
                    Vec::new(),
                ));
            }
        }
        // Too many ideas in one sentence: many clause links in a longish sentence.
        let links = sent.matches(',').count()
            + sent.matches(';').count()
            + sent.matches('(').count()
            + words
                .iter()
                .filter(|w| {
                    matches!(
                        w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()),
                        "which" | "and" | "but" | "whereas" | "while" | "because" | "so"
                    )
                })
                .count();
        if words.len() >= 25 && links >= 6 {
            out.push(note(
                s.clone(),
                "TooManyIdeas",
                format!("This sentence links {links} ideas. Readers follow one idea per sentence more easily: try splitting it into two or three."),
                Vec::new(),
            ));
        }
    }
    out
}

/// Filler words that rarely add meaning in work writing, with a note.
const FILLERS: &[(&str, &str)] = &[
    ("really", "usually adds nothing"),
    ("actually", "usually adds nothing"),
    ("basically", "usually adds nothing"),
    ("simply", "usually adds nothing"),
    ("totally", "usually adds nothing"),
    ("literally", "usually adds nothing"),
    ("very", "try a stronger word instead (very big → huge)"),
    ("quite", "usually adds nothing"),
];

/// Habit and filler notes: "I just want" → "I want", a "So" opener, filler words.
fn fillers(text: &str, out: &mut Vec<Local>) {
    let words: Vec<(usize, &str)> = text.unicode_word_indices().collect();
    for (k, &(i, w)) in words.iter().enumerate() {
        let lw = w.to_lowercase();
        let next = words.get(k + 1).map(|(_, n)| n.to_lowercase()).unwrap_or_default();
        let prev = if k > 0 { words[k - 1].1.to_lowercase() } else { String::new() };
        // "I just want", "just wanted to check", "just a quick note".
        if lw == "just"
            && matches!(
                next.as_str(),
                "want"
                    | "wanted"
                    | "need"
                    | "needed"
                    | "thought"
                    | "think"
                    | "checking"
                    | "wondering"
                    | "following"
                    | "wanting"
                    | "a"
                    | "to"
                    | "letting"
            )
        {
            let end = i + w.len();
            let take_space = text[end..].starts_with(' ');
            let start = if text[..i].ends_with(' ') && !take_space { i - 1 } else { i };
            let r = start..if take_space { end + 1 } else { end };
            out.push(note(r, "Filler", "`Just` makes the request sound smaller and less sure. Try leaving it out: `I want…`, `I wanted to check…`.".into(), vec![String::new()]));
            continue;
        }
        // "So I want…", "So, the plan is…" at the start of a sentence.
        if lw == "so" && w.starts_with('S') {
            let before = text[..i].trim_end();
            let at_start = before.is_empty() || before.ends_with(['.', '!', '?', '\n']);
            let filler = matches!(next.as_str(), "i" | "we" | "the" | "basically" | "yeah" | "anyway" | "um")
                || text[i + w.len()..].starts_with(',');
            if at_start && filler {
                let mut end = i + w.len();
                if text[end..].starts_with(',') {
                    end += 1;
                }
                if text[end..].starts_with(' ') {
                    end += 1;
                }
                let rest = &text[end..];
                let cap: String = rest
                    .chars()
                    .take(1)
                    .flat_map(char::to_uppercase)
                    .chain(rest.chars().skip(1).take_while(|c| c.is_alphanumeric()))
                    .collect();
                let first: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                let r = i..end + first.len();
                out.push(note(
                    r,
                    "Filler",
                    "Starting with `So` is a spoken habit; in writing, start with the point.".into(),
                    vec![cap],
                ));
                continue;
            }
        }
        if let Some((_, why)) = FILLERS.iter().find(|(f, _)| *f == lw) {
            // Keep "not really", "very well", "quite a few" (they change meaning).
            if prev == "not"
                || matches!(
                    next.as_str(),
                    "well"
                        | "much"
                        | "many"
                        | "few"
                        | "a"
                        | "an"
                        | "the"
                        | "same"
                        | "first"
                        | "last"
                        | "best"
                        | "least"
                        | "end"
                )
            {
                continue;
            }
            let end = i + w.len();
            let r = if text[end..].starts_with(' ') { i..end + 1 } else { i..end };
            out.push(note(r, "Filler", format!("`{w}` {why}."), vec![String::new()]));
        }
    }
}

const NUMBER_WORDS: &[&str] = &[
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "several",
    "many",
    "a few",
    "the following",
];

/// "There are 5 systems, ABC, CDE, EFG." → a colon introduces the list.
fn list_after_comma(text: &str, out: &mut Vec<Local>) {
    let lower = text.to_lowercase();
    for lead in
        ["there are ", "there were ", "we have ", "it has ", "they are ", "these are ", "it includes ", "includes "]
    {
        for (i, _) in lower.match_indices(lead) {
            let after = &text[i + lead.len()..];
            // A count, then a plural noun, then a comma.
            let mut words = after.split_whitespace();
            let count = words.next().unwrap_or("");
            let noun = words.next().unwrap_or("");
            let counted = count.chars().all(|c| c.is_ascii_digit()) && !count.is_empty()
                || NUMBER_WORDS.contains(&count.to_lowercase().as_str());
            if !counted || !noun.ends_with("s,") {
                continue;
            }
            let comma = i + lead.len() + after.find(noun).unwrap_or(0) + noun.len() - 1;
            // The list itself follows: as many items as the count ("5 systems, ABC, CDE, EFG, GHI, IJK"),
            // not more of an ordinary list ("three locomotives, eight cars, and a station").
            let rest = &text[comma + 1..];
            let end = rest.find(['.', '!', '?', ';']).unwrap_or(rest.len());
            let items: Vec<&str> = rest[..end]
                .split(',')
                .flat_map(|x| x.split(" and "))
                .map(|x| x.trim().trim_start_matches("and ").trim())
                .filter(|x| !x.is_empty())
                .collect();
            let n = count.parse::<usize>().ok().or_else(|| {
                ["two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"]
                    .iter()
                    .position(|w| *w == count.to_lowercase())
                    .map(|i| i + 2)
            });
            let first = rest[..end].split_whitespace().next().unwrap_or("").to_lowercase();
            let starts_list = !matches!(first.as_str(), "and" | "but" | "or" | "a" | "an" | "the" | "one")
                && !first.chars().next().is_some_and(|c| c.is_ascii_digit())
                && !NUMBER_WORDS.contains(&first.as_str());
            if items.len() < 2 || !starts_list || n.is_some_and(|n| items.len() != n) {
                continue;
            }
            out.push(note(
                comma..comma + 1,
                "ListColon",
                "Use a colon to introduce a list: `There are five systems: ABC, CDE and EFG.`".into(),
                vec![":".into()],
            ));
        }
    }
}

/// An "it"/"this" that could mean several named things: "ABC ingests data,
/// CDE appends data, so it needs to be complete…".
fn vague_it(text: &str, out: &mut Vec<Local>) {
    for s in sentences(text) {
        let sent = &text[s.clone()];
        let words: Vec<(usize, &str)> = sent.unicode_word_indices().collect();
        for (k, &(i, w)) in words.iter().enumerate() {
            if !matches!(w, "it" | "It" | "this" | "This") {
                continue;
            }
            // Used as a subject: a verb follows.
            let next = words.get(k + 1).map(|(_, n)| n.to_lowercase()).unwrap_or_default();
            if !matches!(
                next.as_str(),
                "is" | "was"
                    | "needs"
                    | "has"
                    | "will"
                    | "should"
                    | "must"
                    | "can"
                    | "requires"
                    | "depends"
                    | "runs"
                    | "takes"
                    | "means"
                    | "would"
                    | "could"
            ) {
                continue;
            }
            // Two or more named things (acronyms or capitalised names) before it in this sentence.
            let named: Vec<&str> = words[..k]
                .iter()
                .map(|(_, w)| *w)
                .filter(|w| {
                    w.len() >= 2
                        && w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                        && w.chars().any(|c| c.is_ascii_uppercase())
                })
                .collect();
            let mut distinct: Vec<&str> = named.clone();
            distinct.sort();
            distinct.dedup();
            if distinct.len() >= 2 {
                let names = distinct.iter().take(3).copied().collect::<Vec<_>>().join(", ");
                let r = s.start + i..s.start + i + w.len();
                out.push(note(
                    r,
                    "VagueReference",
                    format!(
                        "It's not clear what `{w}` refers to here ({names}…?). Name it, e.g. `{} needs to be…`.",
                        distinct[distinct.len() - 1]
                    ),
                    Vec::new(),
                ));
            }
        }
    }
}

/// Words that mark a request or a question in an email.
const ASKS: &[&str] = &[
    "could you",
    "can you",
    "would you",
    "will you",
    "please",
    "let me know",
    "i need you",
    "we need you",
    "would it be possible",
    "are you able",
    "do you mind",
];

/// Acronyms almost everyone knows.
const KNOWN: &[&str] = &[
    "OK", "US", "USA", "UK", "EU", "UN", "PDF", "URL", "CEO", "CFO", "CTO", "ID", "PM", "AM", "TV", "FAQ", "USB",
    "CPU", "GPU", "RAM", "HTTP", "HTTPS", "HTML", "CSS", "IT", "HR", "AI", "PC", "ASAP", "FYI", "PS", "BBC", "NASA",
    "FBI", "NHS", "GDP", "DIY", "ETA", "Q1", "Q2", "Q3", "Q4", "I", "A", "UTC", "GMT", "AM", "SMS", "GPS", "DVD", "CD",
    "MP3", "JSON", "XML", "SQL", "API", "UI", "UX", "VPN", "WIFI",
];

/// Document checks: a request buried at the end of an email; acronyms used before being spelled out.
pub fn check_document(paras: &[String], p: &Profile) -> Vec<(usize, Issue)> {
    let mut out = Vec::new();
    let issue = |para: usize, range: std::ops::Range<usize>, rule: &str, message: String| {
        (
            para,
            Issue {
                range,
                category: Category::Style,
                rule: rule.into(),
                message,
                suggestions: Vec::new(),
                feature: None,
                strong: false,
            },
        )
    };
    if p.main_point_first {
        // Sentences in order, skipping a greeting line.
        let mut count = 0;
        let mut words = 0;
        'outer: for (id, text) in paras.iter().enumerate() {
            let t = text.trim();
            if t.is_empty() || (count == 0 && t.split_whitespace().count() <= 4 && t.ends_with(',')) {
                continue;
            }
            for s in sentences(text) {
                let sent = &text[s.clone()];
                let lower = sent.to_lowercase();
                let ask = sent.trim_end().ends_with('?') || ASKS.iter().any(|a| lower.contains(a));
                if ask {
                    if count >= 3 && words >= 30 {
                        out.push(issue(
                            id,
                            s.clone(),
                            "MainPointLate",
                            format!("Your request comes after {words} words. Busy readers decide from the first lines: try putting what you need first, then the background."),
                        ));
                    }
                    break 'outer;
                }
                count += 1;
                words += sent.split_whitespace().count();
            }
        }
    }
    if p.define_acronyms {
        let mut seen: HashSet<String> = HashSet::new();
        for (id, text) in paras.iter().enumerate() {
            for (r, word) in acronyms(text) {
                if seen.contains(&word) || KNOWN.contains(&word.as_str()) {
                    continue;
                }
                seen.insert(word.clone());
                // Defined here: "SLA (service level agreement)" or "service level agreement (SLA)".
                let after = text[r.end..].trim_start();
                let before = text[..r.start].trim_end();
                if after.starts_with('(') || before.ends_with('(') {
                    continue;
                }
                out.push(issue(id, r, "UndefinedAcronym", format!("`{word}` isn't spelled out before this. On first use, write it in full with the short form in brackets, e.g. `service level agreement (SLA)`.")));
            }
        }
    }
    out
}

/// All-capital words of 2–6 letters (an optional plural `s`).
fn acronyms(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    for (i, w) in text.split_word_bound_indices() {
        let core = w.strip_suffix('s').filter(|c| c.len() >= 2).unwrap_or(w);
        if (2..=6).contains(&core.len())
            && core.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && core.chars().any(|c| c.is_ascii_uppercase())
            && core.chars().filter(|c| c.is_ascii_uppercase()).count() >= 2
        {
            // A whole shouted sentence isn't acronyms.
            let shouting = text.chars().filter(|c| c.is_alphabetic()).filter(|c| c.is_uppercase()).count() * 2
                > text.chars().filter(|c| c.is_alphabetic()).count();
            if !shouting {
                out.push((i..i + w.len(), core.to_string()));
            }
        }
    }
    out
}

use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str, p: &Profile) -> Vec<String> {
        check(text, p).into_iter().map(|l| l.rule).collect()
    }

    #[test]
    fn paragraph_checks() {
        let p = Profile { clarity: true, ..Profile::default() };
        assert!(
            rules("Please review the database migration rollback strategy document today.", &p)
                .contains(&"NounStack".to_string())
        );
        assert!(
            rules("The build artifact caching layer invalidation logic is wrong.", &p)
                .contains(&"NounStack".to_string())
        );
        assert!(rules("Please review the strategy for rolling back the migration.", &p).is_empty());
        let lead = "Because the vendor changed the API contract last week without telling anyone on our side of the integration team, we missed the deadline.";
        assert!(rules(lead, &p).contains(&"LongLeadIn".to_string()));
        assert!(rules("Because the vendor changed the API, we missed the deadline.", &p).is_empty());
        let busy = "The service, which runs on the cluster, reads the queue, and it writes results to the store, but it retries on failure, and it logs everything, which helps debugging.";
        assert!(rules(busy, &p).contains(&"TooManyIdeas".to_string()));
        let off = Profile { clarity: false, ..Profile::default() };
        assert!(rules(lead, &off).is_empty());
    }

    #[test]
    fn habits_and_fillers() {
        let p = Profile { clarity: true, ..Profile::default() };
        let fix = |t: &str| {
            let mut found = check(t, &p);
            found.retain(|l| l.rule == "Filler");
            found.sort_by_key(|l| std::cmp::Reverse(l.range.start));
            let mut s = t.to_string();
            for l in found {
                s.replace_range(l.range, &l.suggestions[0]);
            }
            s
        };
        assert_eq!(fix("I just want the report by Friday."), "I want the report by Friday.");
        assert_eq!(fix("So I want to move the meeting."), "I want to move the meeting.");
        assert_eq!(fix("So, the plan is simple."), "The plan is simple.");
        assert_eq!(fix("It is really important."), "It is important.");
        assert_eq!(fix("It is not really important."), "It is not really important.");
        assert_eq!(fix("We did very well."), "We did very well.");
        assert_eq!(fix("It was just right."), "It was just right.");
        assert_eq!(fix("Quite a few people came."), "Quite a few people came.");
        assert_eq!(fix("We did it so it works."), "We did it so it works.");
    }

    #[test]
    fn the_five_systems_example() {
        let p = Profile { clarity: true, ..Profile::default() };
        let t = "There are 5 systems, ABC, CDE, EFG, GHI, IJK. ABC ingests data, CDE appends data, so it needs to be complete within 5 weeks.";
        let rules: Vec<(String, String)> = check(t, &p).into_iter().map(|l| (l.rule, t[l.range].to_string())).collect();
        assert!(rules.contains(&("ListColon".into(), ",".into())), "{rules:?}");
        let t2 = "We have three teams, Sales, Support and Ops.";
        assert!(check(t2, &p).iter().any(|l| l.rule == "ListColon"), "{t2}");
        assert!(rules.contains(&("VagueReference".into(), "it".into())), "{rules:?}");
        // Fine as written.
        for ok in [
            "There are two options, but both are slow.",
            "ABC ingests data. It needs to be complete by Friday.",
            "There are 5 systems: ABC, CDE and EFG.",
            "The new set includes three locomotives, eight passenger cars, and a model station.",
            "There were several staircases, and one marked out from the others by its steps.",
        ] {
            let r: Vec<String> = check(ok, &p)
                .into_iter()
                .filter(|l| matches!(l.rule.as_str(), "ListColon" | "VagueReference"))
                .map(|l| l.rule)
                .collect();
            assert!(r.is_empty(), "{ok}: {r:?}");
        }
    }

    #[test]
    fn document_checks() {
        let p = Profile { main_point_first: true, define_acronyms: true, ..Profile::default() };
        let paras: Vec<String> = [
            "Hi Sam,",
            "Last week we moved the billing service to the new cluster. The move went well overall. Some jobs ran slowly at first because the cache was cold. We tuned the pool sizes and things settled down by Thursday.",
            "Could you approve the SLA change by Friday?",
            "The service level agreement (SLA) covers uptime. The API stays the same. Our TCO dropped.",
        ]
        .iter()
        .map(|t| t.to_string())
        .collect();
        let found = check_document(&paras, &p);
        let rules: Vec<(&str, usize)> = found.iter().map(|(p, i)| (i.rule.as_str(), *p)).collect();
        assert!(rules.contains(&("MainPointLate", 2)), "{rules:?}");
        assert!(rules.contains(&("UndefinedAcronym", 2)), "SLA used before it's defined: {rules:?}");
        assert!(rules.contains(&("UndefinedAcronym", 3)), "TCO: {rules:?}");
        assert!(!found.iter().any(|(p, i)| paras[*p][i.range.clone()].starts_with("API")), "API is well known");
        // Asking first is fine.
        let early: Vec<String> = vec!["Could you approve the change by Friday? Background: the move went well, the jobs were slow, and we tuned them.".into()];
        assert!(check_document(&early, &p).iter().all(|(_, i)| i.rule != "MainPointLate"));
    }
}
