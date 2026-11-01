//! Whole-document style checks, switched by the profile:
//!
//! - Consistency: one document written two ways — "organise" and "organize",
//!   "email" and "e-mail", "Internet" and "internet", "3 days" and "three
//!   days". The less used form is noted, with the more used one as the fix.
//! - Word choice: clichés, "very" + a plain word where one stronger word
//!   exists, and several sentences in a row starting with the same word.
//!
//! False alarms matter more than coverage here: capitals and -ise/-ize go
//! by fixed word lists, and checks that couldn't avoid false alarms on
//! well-edited books (the Oxford comma, repeated words) were left out.
//!
//! All are gentle style notes; nothing here is a mistake on its own.

use crate::clarity::sentences;
use crate::{Category, Issue};
use std::collections::HashMap;
use std::ops::Range;

fn note(para: usize, range: Range<usize>, rule: &str, message: String, suggestions: Vec<String>) -> (usize, Issue) {
    (
        para,
        Issue {
            range,
            category: Category::Style,
            rule: rule.into(),
            message,
            suggestions,
            feature: None,
            strong: false,
        },
    )
}

/// Words with byte ranges (letters, digits, apostrophes and hyphens inside words).
fn words(text: &str) -> Vec<(Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut prev = ' ';
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        let inner = (c == '-' || c == '\'' || c == '’') && prev.is_alphanumeric();
        let wordy = c.is_alphanumeric() || (inner && start.is_some());
        match (wordy, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                let w = text[s..i].trim_end_matches(['-', '\'', '’']);
                out.push((s..s + w.len(), w));
                start = None;
            }
            _ => {}
        }
        prev = c;
    }
    out
}

/// Whether byte `pos` starts a sentence (start of text, or after . ! ? : or an opening quote/bracket).
fn sentence_start(text: &str, pos: usize) -> bool {
    // Just after an opening quote: speech starts with a capital ("…he said, “Then we go.”").
    if text[..pos].trim_end_matches([' ', '\t']).ends_with(['“', '"', '‘']) {
        return true;
    }
    let before = text[..pos].trim_end_matches(['"', '“', '‘', '\'', '(', '[', ' ', '\t']);
    before.is_empty() || before.ends_with(['.', '!', '?', ':', '\n'])
}

/// Where a word form is used: (paragraph, byte range).
type Uses = Vec<(usize, Range<usize>)>;

/// A word written one way in some places and another way elsewhere, by key.
/// `forms` maps each spelling to where it's used.
struct Variants {
    forms: HashMap<String, HashMap<String, Uses>>,
}

impl Variants {
    fn new() -> Self {
        Self { forms: HashMap::new() }
    }
    fn add(&mut self, key: String, form: String, para: usize, r: Range<usize>) {
        self.forms.entry(key).or_default().entry(form).or_default().push((para, r));
    }
    /// A note on every use of a less used form, with the most used one as the fix.
    fn notes(self, rule: &str, what: &str, out: &mut Vec<(usize, Issue)>) {
        let mut keys: Vec<_> = self.forms.into_iter().filter(|(_, f)| f.len() > 1).collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, forms) in keys {
            let mut forms: Vec<_> = forms.into_iter().collect();
            // Most used first; ties go to the form used first.
            forms.sort_by(|a, b| {
                b.1.len().cmp(&a.1.len()).then((a.1[0].0, a.1[0].1.start).cmp(&(b.1[0].0, b.1[0].1.start)))
            });
            let main = forms[0].0.clone();
            let n = forms[0].1.len();
            for (form, uses) in &forms[1..] {
                for (para, r) in uses {
                    let msg = format!(
                        "This document also writes {what} as `{main}` ({n} time{}). Pick one and use it throughout.",
                        if n == 1 { "" } else { "s" }
                    );
                    out.push(note(*para, r.clone(), rule, msg, vec![match_case(&main, form)]));
                }
            }
        }
    }
}

/// `word` with the capitals of `like` when `like` starts with one (and `word` is lower case).
fn match_case(word: &str, like: &str) -> String {
    if like.starts_with(char::is_uppercase) && word.starts_with(char::is_lowercase) {
        let mut c = word.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    } else {
        word.to_string()
    }
}

/// -ise/-ize endings (and -yse/-yze), as (UK-style, US-style) pairs.
const ISE: &[(&str, &str)] = &[
    ("ise", "ize"),
    ("ises", "izes"),
    ("ised", "ized"),
    ("ising", "izing"),
    ("isation", "ization"),
    ("isations", "izations"),
    ("yse", "yze"),
    ("ysed", "yzed"),
    ("ysing", "yzing"),
];

/// Common words spelled both -ise and -ize (or -yse and -yze), by stem. A
/// fixed list: guessing from the ending also catches names ("Louise"),
/// -ise-only words ("advise") and words like "size".
const ISE_STEMS: &[&str] = &[
    "organ",
    "real",
    "recogn",
    "apolog",
    "emphas",
    "summar",
    "special",
    "standard",
    "minim",
    "maxim",
    "optim",
    "util",
    "priorit",
    "categor",
    "character",
    "critic",
    "author",
    "central",
    "civil",
    "final",
    "normal",
    "legal",
    "local",
    "visual",
    "capital",
    "modern",
    "memor",
    "symbol",
    "custom",
    "personal",
    "familiar",
    "harmon",
    "global",
    "industrial",
    "material",
    "commercial",
    "hospital",
    "social",
    "general",
    "computer",
    "digit",
    "monet",
    "privat",
    "mobil",
    "stabil",
    "steril",
    "subsid",
    "synchron",
    "theor",
    "colon",
    "critic",
    "item",
    "jeopard",
    "patron",
    "sympath",
    "fertil",
    "neutral",
    "anal",
    "paral",
    "catal",
    "dial",
];

/// For an -ise/-ize word ("organised", "organized"): whether it's the -ise
/// style, and the word in the other style.
fn ise_style(w: &str) -> Option<(bool, String)> {
    for (is, iz) in ISE {
        for (end, other, uk) in [(is, iz, true), (iz, is, false)] {
            if let Some(stem) = w.strip_suffix(end)
                && ISE_STEMS.contains(&stem)
                && (stem != "dial" || end.starts_with('y'))
            {
                return Some((uk, format!("{stem}{other}")));
            }
        }
    }
    None
}

/// Hyphenated and joined words that mean different things ("re-cover" a chair, "recover" from flu).
const HYPHEN_MEANS_MORE: &[&str] = &[
    "recover",
    "resign",
    "reform",
    "recreation",
    "resort",
    "release",
    "remark",
    "repress",
    "resent",
    "reserve",
    "restore",
    "retreat",
    "reprint",
    "coop",
    "unionized",
    "unionised",
    "recount",
    "refuse",
    "relay",
    "reside",
    "represent",
    "research",
    "redress",
    "recollect",
    "recreate",
    "repose",
];

/// Capitalised words that are also everyday words ("May", "Bill", "Turkey"),
/// so a capital and a small letter can both be right.
const NAME_WORDS: &[&str] = &[
    "may",
    "march",
    "august",
    "june",
    "april",
    "bill",
    "mark",
    "will",
    "turkey",
    "china",
    "polish",
    "rose",
    "grace",
    "jack",
    "frank",
    "faith",
    "hope",
    "joy",
    "dawn",
    "summer",
    "autumn",
    "winter",
    "spring",
    "pat",
    "sue",
    "rob",
    "bob",
    "art",
    "max",
    "ray",
    "sky",
    "page",
    "cook",
    "baker",
    "smith",
    "miller",
    "carter",
    "hunter",
    "fisher",
    "young",
    "king",
    "queen",
    "prince",
    "lord",
    "god",
    "church",
    "state",
    "bank",
    "street",
    "road",
    "river",
    "lake",
    "mount",
    "north",
    "south",
    "east",
    "west",
    "western",
    "eastern",
    "northern",
    "southern",
    "new",
    "old",
    "great",
    "white",
    "black",
    "brown",
    "green",
    "gray",
    "grey",
    "long",
    "little",
    "major",
    "general",
    "president",
    "senator",
    "doctor",
    "professor",
    "captain",
    "chapter",
    "section",
    "figure",
    "table",
    "part",
    "book",
    "act",
    "article",
    "university",
    "college",
    "school",
    "company",
    "house",
    "court",
    "city",
    "county",
    "council",
    "party",
    "the",
];

/// Words people really write both with and without a capital in one
/// document. Other capitals often mean something (a title, a name, a
/// personified idea), so they're left alone.
const CAPITAL_TERMS: &[&str] = &[
    "internet",
    "web",
    "website",
    "webpage",
    "email",
    "online",
    "government",
    "parliament",
    "congress",
    "senate",
    "federal",
    "earth",
    "president",
    "chairman",
    "chairwoman",
    "chairperson",
    "director",
    "manager",
    "board",
    "headquarters",
    "department",
    "university",
    "spring",
    "summer",
    "autumn",
    "fall",
    "winter",
    "northern",
    "southern",
    "eastern",
    "western",
    "biblical",
    "bible",
    "covid",
    "wifi",
    "blockchain",
    "cloud",
    "metaverse",
];

/// Spelled-out numbers one to nine, and their digits.
const SMALL: &[(&str, &str)] = &[
    ("one", "1"),
    ("two", "2"),
    ("three", "3"),
    ("four", "4"),
    ("five", "5"),
    ("six", "6"),
    ("seven", "7"),
    ("eight", "8"),
    ("nine", "9"),
];

/// Words after a digit that make the digit right whatever the style (units, times, versions).
const UNITS: &[&str] = &[
    "am", "pm", "a.m", "p.m", "km", "kg", "g", "mg", "m", "cm", "mm", "ml", "l", "gb", "mb", "kb", "tb", "mph", "kph",
    "percent", "per", "x", "px", "pt", "in", "ft", "lb", "lbs", "oz", "s", "ms", "h", "hr", "hrs", "min", "mins",
    "sec", "secs", "k", "m", "bn", "million", "billion", "thousand", "hundred", "degrees", "out", "of", "and", "or",
    "to", "point",
];

/// Consistency notes across the whole document.
pub fn check(paras: &[String]) -> Vec<(usize, Issue)> {
    let mut out = Vec::new();
    // -ise and -ize words: (para, range, word in the other style).
    let mut ise: [Vec<(usize, Range<usize>, String)>; 2] = [Vec::new(), Vec::new()];
    let mut joined = Variants::new();
    let mut caps = Variants::new();
    let mut digits: Vec<(usize, Range<usize>, &str)> = Vec::new();
    let mut spelled: Vec<(usize, Range<usize>, &str)> = Vec::new();
    for (p, text) in paras.iter().enumerate() {
        let ws = words(text);
        for (k, (r, w)) in ws.iter().enumerate() {
            let lower = w.to_lowercase();
            // Names and titles ("Marquise", "Louise") keep their spelling.
            let named = w.starts_with(char::is_uppercase) && !sentence_start(text, r.start);
            if let Some((uk, other)) = ise_style(&lower).filter(|_| !named) {
                ise[uk as usize].push((p, r.clone(), match_case(&other, w)));
            }
            if HYPHEN_MEANS_MORE.contains(&lower.replace('-', "").as_str()) {
                // Both spellings are right, with different meanings.
            } else if lower.contains('-') {
                joined.add(lower.replace('-', ""), lower.clone(), p, r.clone());
            } else if lower.chars().all(char::is_alphabetic) {
                joined.add(lower.clone(), lower.clone(), p, r.clone());
            }
            // Capitals: only mid-sentence, not next to other capitalised words (names like "Mark Twain").
            let alpha = w.chars().all(char::is_alphabetic) && w.chars().count() >= 4;
            let neighbour_cap =
                |j: Option<&(Range<usize>, &str)>| j.is_some_and(|(_, n)| n.starts_with(char::is_uppercase));
            if alpha
                && CAPITAL_TERMS.contains(&lower.as_str())
                && !sentence_start(text, r.start)
                && !NAME_WORDS.contains(&lower.as_str())
                && !w.chars().skip(1).any(char::is_uppercase)
                && !neighbour_cap(k.checked_sub(1).and_then(|j| ws.get(j)))
                && !neighbour_cap(ws.get(k + 1))
            {
                caps.add(lower.clone(), w.to_string(), p, r.clone());
            }
            // Small numbers before a plain word: "3 days" / "three days".
            let next = ws.get(k + 1).map(|(_, n)| n.to_lowercase());
            let plain_next = next
                .as_deref()
                .is_some_and(|n| n.chars().all(|c| c.is_ascii_lowercase()) && !UNITS.contains(&n) && n.len() > 1)
                && ws.get(k + 1).is_some_and(|(nr, _)| &text[r.end..nr.start] == " ");
            let before = text[..r.start].chars().next_back();
            let after = text[r.end..].chars().next();
            let lone = !matches!(before, Some('.' | ':' | '/' | '$' | '£' | '€' | '#' | '-' | '+' | ','))
                && !matches!(after, Some('.' | ':' | '/' | '%' | ')' | '-' | ','));
            if plain_next && lone && !sentence_start(text, r.start) {
                if let Some((_, d)) = SMALL.iter().find(|(_, d)| *d == *w) {
                    digits.push((p, r.clone(), d));
                } else if let Some((s, _)) = SMALL.iter().find(|(s, _)| *s == lower) {
                    spelled.push((p, r.clone(), s));
                }
            }
        }
    }
    // -ise/-ize: the less used style, across all such words.
    if !ise[0].is_empty() && !ise[1].is_empty() {
        let (few, main) = if ise[1].len() < ise[0].len() { (&ise[1], "-ize") } else { (&ise[0], "-ise") };
        for (p, r, other) in few {
            let msg = format!(
                "Elsewhere this document spells words like this with {main}. Pick one style and use it throughout."
            );
            out.push(note(*p, r.clone(), "ConsistentSpelling", msg, vec![other.clone()]));
        }
    }
    // "email" and "e-mail": only words used both ways.
    let mut joined_both = Variants::new();
    for (key, forms) in joined.forms {
        if forms.len() > 1 && forms.keys().any(|f| f.contains('-')) {
            for (form, uses) in forms {
                for (p, r) in uses {
                    joined_both.add(key.clone(), form.clone(), p, r);
                }
            }
        }
    }
    joined_both.notes("ConsistentHyphens", "this word", &mut out);
    caps.notes("ConsistentCapitals", "this word", &mut out);
    // Numbers: the less used style for small numbers.
    if !digits.is_empty() && !spelled.is_empty() {
        let (few, style, fix) =
            if digits.len() < spelled.len() { (&digits, "in words", true) } else { (&spelled, "as figures", false) };
        for (p, r, w) in few {
            let alt = SMALL.iter().find(|(s, d)| *s == *w || *d == *w).map(|(s, d)| if fix { s } else { d });
            out.push(note(
                *p,
                r.clone(),
                "ConsistentNumbers",
                format!("Elsewhere this document writes small numbers {style}. Pick one style and use it throughout."),
                alt.map(|a| vec![match_case(a, &paras[*p][r.clone()])]).unwrap_or_default(),
            ));
        }
    }
    out
}

/// Clichés, with a plainer way to say each (or nothing: just cut it).
const CLICHES: &[(&str, &str)] = &[
    ("at the end of the day", "in the end"),
    ("think outside the box", "think creatively"),
    ("thinking outside the box", "thinking creatively"),
    ("low-hanging fruit", "easy wins"),
    ("low hanging fruit", "easy wins"),
    ("move the needle", "make a difference"),
    ("a perfect storm", "a bad combination"),
    ("touch base", "talk"),
    ("circle back", "come back to it"),
    ("at this point in time", "now"),
    ("in this day and age", "today"),
    ("last but not least", "finally"),
    ("needless to say", ""),
    ("it goes without saying", ""),
    ("avoid it like the plague", "avoid it"),
    ("avoid like the plague", "avoid"),
    ("the tip of the iceberg", "a small part"),
    ("a level playing field", "a fair chance"),
    ("hit the ground running", "start quickly"),
    ("ballpark figure", "estimate"),
    ("game changer", "big change"),
    ("game-changer", "big change"),
    ("paradigm shift", "big change"),
    ("synergy", "working together"),
    ("best of both worlds", "both benefits"),
    ("only time will tell", "we'll see"),
    ("easier said than done", "hard"),
    ("every cloud has a silver lining", "there's a good side"),
    ("a blessing in disguise", "a good thing after all"),
    ("in the nick of time", "just in time"),
    ("better late than never", ""),
    ("think on your feet", "think quickly"),
    ("push the envelope", "go further"),
    ("raise the bar", "set a higher standard"),
    ("on the same page", "in agreement"),
    ("going forward", "from now on"),
    ("moving forward", "from now on"),
    ("take it to the next level", "improve it"),
    ("at the drop of a hat", "at once"),
    ("as old as the hills", "very old"),
    ("crystal clear", "clear"),
    ("few and far between", "rare"),
    ("in a nutshell", "in short"),
    ("the bottom line", "the main point"),
    ("when all is said and done", "in the end"),
    ("all walks of life", "all backgrounds"),
    ("a whole new ballgame", "a different situation"),
    ("par for the course", "normal"),
    ("the elephant in the room", "the obvious problem"),
    ("back to the drawing board", "start again"),
    ("cutting edge", "advanced"),
    ("cutting-edge", "advanced"),
    ("state of the art", "modern"),
    ("state-of-the-art", "modern"),
    ("world-class", "excellent"),
    ("best-in-class", "leading"),
    ("each and every", "every"),
    ("first and foremost", "first"),
    ("tried and true", "proven"),
    ("tried and tested", "proven"),
    ("time and time again", "repeatedly"),
    ("in the final analysis", "in the end"),
    ("leave no stone unturned", "search thoroughly"),
    ("the writing on the wall", "the warning signs"),
    ("a drop in the ocean", "a tiny amount"),
    ("a drop in the bucket", "a tiny amount"),
    ("at the speed of light", "very fast"),
    ("as luck would have it", "luckily"),
];

/// "very" + a plain word → one stronger word.
const WEAK: &[(&str, &str)] = &[
    ("good", "excellent"),
    ("bad", "terrible"),
    ("big", "huge"),
    ("large", "huge"),
    ("small", "tiny"),
    ("tired", "exhausted"),
    ("happy", "delighted"),
    ("sad", "miserable"),
    ("angry", "furious"),
    ("scared", "terrified"),
    ("afraid", "terrified"),
    ("cold", "freezing"),
    ("hot", "scorching"),
    ("hungry", "starving"),
    ("important", "essential"),
    ("difficult", "challenging"),
    ("fast", "rapid"),
    ("quick", "rapid"),
    ("slow", "sluggish"),
    ("clean", "spotless"),
    ("dirty", "filthy"),
    ("funny", "hilarious"),
    ("pretty", "beautiful"),
    ("ugly", "hideous"),
    ("quiet", "silent"),
    ("loud", "deafening"),
    ("sure", "certain"),
    ("surprised", "astonished"),
    ("interesting", "fascinating"),
    ("boring", "tedious"),
    ("bright", "dazzling"),
    ("dark", "pitch-black"),
    ("wet", "soaked"),
    ("full", "packed"),
    ("busy", "swamped"),
    ("worried", "anxious"),
    ("careful", "cautious"),
];

/// Word-choice notes across the whole document.
pub fn check_words(paras: &[String]) -> Vec<(usize, Issue)> {
    let mut out = Vec::new();
    // Sentences in order, as (para, range, first word); headings and list items break a run.
    let mut run: Vec<(usize, Range<usize>, String)> = Vec::new();
    for (p, text) in paras.iter().enumerate() {
        let lower = text.to_lowercase();
        // Clichés.
        for (c, plain) in CLICHES {
            let mut from = 0;
            while let Some(i) = lower[from..].find(c).map(|i| from + i) {
                from = i + c.len();
                let edge_ok = |j: usize| lower[j..].chars().next().is_none_or(|ch| !ch.is_alphanumeric());
                let start_ok = lower[..i].chars().next_back().is_none_or(|ch| !ch.is_alphanumeric());
                if !start_ok || !edge_ok(i + c.len()) {
                    continue;
                }
                let r = i..i + c.len();
                let msg = if plain.is_empty() {
                    "This phrase is a cliché and adds little. Try leaving it out.".to_string()
                } else {
                    format!("This phrase is a cliché. Plainer: `{plain}`.")
                };
                let fix = if plain.is_empty() { Vec::new() } else { vec![match_case(plain, &text[r.clone()])] };
                out.push(note(p, r, "Cliche", msg, fix));
            }
        }
        let ws = words(text);
        // "very good" → "excellent".
        for k in 0..ws.len().saturating_sub(1) {
            let w = ws[k].1.to_lowercase();
            if !matches!(w.as_str(), "very" | "really" | "extremely") {
                continue;
            }
            let next = ws[k + 1].1.to_lowercase();
            if &text[ws[k].0.end..ws[k + 1].0.start] != " " {
                continue;
            }
            // "Very good, sir." / "Very well." as a reply is a set phrase.
            let reply =
                sentence_start(text, ws[k].0.start) && text[ws[k + 1].0.end..].starts_with([',', '.', '!', ';']);
            if let Some((_, strong)) = WEAK.iter().find(|(plain, _)| *plain == next).filter(|_| !reply) {
                let r = ws[k].0.start..ws[k + 1].0.end;
                out.push(note(
                    p,
                    r.clone(),
                    "WeakIntensifier",
                    format!("`{}` could be one stronger word: `{strong}`.", &text[r.clone()]),
                    vec![match_case(strong, &text[r])],
                ));
            }
        }
        // Sentence openers: a heading or list item (no full stop) breaks the run.
        let sents = sentences(text);
        let whole = text.trim_end().ends_with(['.', '!', '?', '"', '”']);
        if !whole || sents.is_empty() {
            note_run(&mut run, paras, &mut out);
            continue;
        }
        for s in sents {
            let first =
                words(&text[s.clone()]).first().map(|(r, w)| (s.start + r.start..s.start + r.end, w.to_lowercase()));
            let Some((r, w)) = first else { continue };
            if run.last().is_some_and(|(_, _, f)| *f != w) {
                note_run(&mut run, paras, &mut out);
            }
            run.push((p, r, w));
        }
    }
    note_run(&mut run, paras, &mut out);
    out
}

/// Three or more sentences in a row starting with the same word: a note on the third and later ones.
fn note_run(run: &mut Vec<(usize, Range<usize>, String)>, paras: &[String], out: &mut Vec<(usize, Issue)>) {
    // Pronouns and "the" start most sentences in stories and reports.
    if run.len() >= 3 && !matches!(run[0].2.as_str(), "i" | "we" | "you" | "he" | "she" | "it" | "they" | "the") {
        for (p, r, _) in &run[2..] {
            let w = &paras[*p][r.clone()];
            out.push(note(
                *p,
                r.clone(),
                "RepeatedOpener",
                format!("Several sentences in a row start with `{w}`. Starting this one differently may read better."),
                Vec::new(),
            ));
        }
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(paras: &[&str], f: fn(&[String]) -> Vec<(usize, Issue)>) -> Vec<(String, String, Vec<String>)> {
        let paras: Vec<String> = paras.iter().map(|s| s.to_string()).collect();
        f(&paras)
            .into_iter()
            .map(|(p, i)| (i.rule.clone(), paras[p][i.range.clone()].to_string(), i.suggestions.clone()))
            .collect()
    }

    #[test]
    fn spelling_hyphens_and_capitals() {
        let r = rules(
            &[
                "We organised the event and recognised the volunteers.",
                "Next year we will organize it again. Send an e-mail to the team.",
                "The email went out on the Internet and the internet was slow. The Internet is busy.",
            ],
            check,
        );
        assert!(r.contains(&("ConsistentSpelling".into(), "organize".into(), vec!["organise".into()])), "{r:?}");
        assert!(r.iter().any(|(rule, w, _)| rule == "ConsistentHyphens" && w.contains("mail")), "{r:?}");
        assert!(r.contains(&("ConsistentCapitals".into(), "internet".into(), vec!["Internet".into()])), "{r:?}");
        // Only-ise words, and words used one way, are fine.
        let r = rules(
            &[
                "She advised them to exercise. He was surprised. We realized it, and we realize it. A large-sized prize was seized.",
            ],
            check,
        );
        assert!(r.is_empty(), "{r:?}");
        // Other capitals often mean something: a title, a name, a personified idea.
        let r = rules(&["We met the Judge today.", "Then the judge left. Love conquers all; love is kind."], check);
        assert!(r.iter().all(|(rule, _, _)| rule != "ConsistentCapitals"), "{r:?}");
        // Speech starts with a capital.
        let r = rules(&["\"Did you say,\" he asked, \"Online is fine?\" We met online."], check);
        assert!(r.is_empty(), "{r:?}");
    }

    #[test]
    fn numbers() {
        let r = rules(&["It took three days and two nights. We waited 3 days more, then four weeks."], check);
        assert_eq!(r, vec![("ConsistentNumbers".into(), "3".into(), vec!["three".into()])]);
        // Units and times are fine as figures.
        let r = rules(&["It took three days. The file is 3 MB and we met at 5 pm."], check);
        assert!(r.is_empty(), "{r:?}");
    }

    #[test]
    fn word_choice() {
        let r = rules(&["At the end of the day, the results were very good."], check_words);
        assert!(r.contains(&("Cliche".into(), "At the end of the day".into(), vec!["In the end".into()])), "{r:?}");
        assert!(r.contains(&("WeakIntensifier".into(), "very good".into(), vec!["excellent".into()])), "{r:?}");
        let r = rules(&["\"Very good, sir,\" he said. Very good."], check_words);
        assert!(r.is_empty(), "a reply: {r:?}");
        let r = rules(&["Then we ate. Then we slept. Then we left."], check_words);
        assert_eq!(r, vec![("RepeatedOpener".into(), "Then".into(), vec![])]);
        let r = rules(&["He ate. He slept. He left."], check_words);
        assert!(r.is_empty(), "pronoun openers are normal: {r:?}");
        // A heading between sentences breaks the run; varied text has no notes.
        let r = rules(&["Then we ate. Then we slept.", "Day two", "Then we left."], check_words);
        assert!(r.is_empty(), "{r:?}");
        let r = rules(&["The morning was calm. We walked to the harbour and watched the boats come in."], check_words);
        assert!(r.is_empty(), "{r:?}");
    }
}
