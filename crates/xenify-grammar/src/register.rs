//! Tone and register: finds informal features of writing (slang, texting,
//! dialect grammar, contractions…) and applies the writing profile's
//! choice for each — allow, note or flag.
//!
//! Messages are neutral: they say what was noticed and give the standard
//! English equivalent, never that the writing is "wrong".

use crate::{Category, Feature, Level, Local, Profile};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

/// (feature, list file contents)
const LISTS: &[(Feature, &str)] = &[
    (Feature::Slang, include_str!("../rules/lists/slang.txt")),
    (Feature::Texting, include_str!("../rules/lists/texting.txt")),
    (Feature::SpokenSpelling, include_str!("../rules/lists/spoken.txt")),
    (Feature::RegionalDialect, include_str!("../rules/lists/regional.txt")),
    (Feature::HedgingFiller, include_str!("../rules/lists/hedging.txt")),
    (Feature::Profanity, include_str!("../rules/lists/profanity.txt")),
];

/// Rules (Harper's or ours) whose findings are really a register feature.
const RULE_FEATURES: &[(&str, Feature)] = &[
    ("ParticipleAsPast", Feature::NonstandardVerbs),
    ("PronounVerbAgreement", Feature::NonstandardVerbs),
    ("PronounInflectionBe", Feature::NonstandardVerbs),
    ("DoubleNegative", Feature::NonstandardVerbs),
    ("DoubleModal", Feature::RegionalDialect),
    ("AvoidCurses", Feature::Profanity),
    ("OhMyGod", Feature::Texting),
    ("ToBeHonest", Feature::Texting),
    ("SentenceCapitalization", Feature::Texting),
    ("CapitalizePersonalPronouns", Feature::Texting),
    ("FillerWords", Feature::HedgingFiller),
];

/// A feature found in the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub range: Range<usize>,
    pub feature: Feature,
    /// The standard English equivalent, if there's a simple one.
    pub standard: Option<String>,
}

/// Lowercased phrase (words joined by single spaces) → (feature, standard).
fn lexicon() -> &'static HashMap<String, (Feature, Option<String>)> {
    static L: OnceLock<HashMap<String, (Feature, Option<String>)>> = OnceLock::new();
    L.get_or_init(|| {
        let mut m = HashMap::new();
        for (f, src) in LISTS {
            for line in src.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (word, std) = line.split_once('|').unwrap_or((line, ""));
                let std = std.trim();
                let std = (!std.is_empty() && !std.starts_with('(')).then(|| std.to_string());
                m.entry(norm(word.trim())).or_insert((*f, std));
            }
        }
        m
    })
}

/// Whether `pos` starts a sentence (start of text, or after . ! ? or an opening quote).
fn sentence_start(text: &str, pos: usize) -> bool {
    let before = text[..pos].trim_end();
    before.is_empty() || before.ends_with(['.', '!', '?', '"', '\u{201c}'])
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words with byte ranges (letters, digits, apostrophes and `/` inside words).
fn words(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut start = None;
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '\u{2019}' || c == '/';
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, is_word(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                let w = &text[s..i];
                // Keep a leading apostrophe ('bout, 'em) but drop trailing quote marks.
                let trimmed = w.trim_end_matches(['\'', '\u{2019}']);
                let keep_trailing = w.ends_with("in'") || w.ends_with("in\u{2019}");
                let w = if keep_trailing { w } else { trimmed };
                if !w.is_empty() {
                    out.push((s..s + w.len(), norm(w)));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

const SUBJECTS: &[&str] = &["i", "you", "he", "she", "it", "we", "they", "y'all"];
const NEGATIVES: &[&str] = &[
    "don't", "didn't", "doesn't", "can't", "won't", "ain't", "wasn't", "weren't", "isn't", "aren't", "haven't",
    "hasn't", "couldn't", "wouldn't", "never", "nobody", "nothing",
];
const NEGATIVE_OBJECTS: &[&str] =
    &["no", "nothing", "nobody", "none", "nowhere", "never", "no one", "nothin", "nothin'"];
const CONTRACTION_ENDINGS: &[&str] = &["n't", "'re", "'ve", "'ll", "'d", "'m"];
const PRONOUN_IS: &[&str] =
    &["it's", "that's", "what's", "he's", "she's", "there's", "here's", "who's", "where's", "let's", "how's", "when's"];
const FIRST_PERSON: &[&str] = &[
    "i",
    "me",
    "my",
    "mine",
    "myself",
    "we",
    "us",
    "our",
    "ours",
    "ourselves",
    "i'm",
    "i've",
    "i'll",
    "i'd",
    "we're",
    "we've",
    "we'll",
    "we'd",
];

/// Every feature found in `text`.
pub fn find(text: &str) -> Vec<Hit> {
    let ws = words(text);
    let lex = lexicon();
    let mut hits = Vec::new();
    let mut taken = vec![false; ws.len()];
    // Word-list phrases, longest first (up to three words).
    for len in (1..=3).rev() {
        for i in 0..ws.len().saturating_sub(len - 1) {
            if taken[i..i + len].iter().any(|t| *t) {
                continue;
            }
            // Only words separated by spaces form a phrase.
            if (i..i + len - 1).any(|k| !text[ws[k].0.end..ws[k + 1].0.start].chars().all(char::is_whitespace)) {
                continue;
            }
            let phrase = ws[i..i + len].iter().map(|(_, w)| w.as_str()).collect::<Vec<_>>().join(" ");
            if let Some((f, std)) = lex.get(&phrase) {
                // A capitalised single word mid-sentence is probably a name ("Wat Pho").
                if len == 1
                    && !sentence_start(text, ws[i].0.start)
                    && text[ws[i].0.clone()].starts_with(char::is_uppercase)
                {
                    continue;
                }
                hits.push(Hit { range: ws[i].0.start..ws[i + len - 1].0.end, feature: *f, standard: std.clone() });
                taken[i..i + len].iter_mut().for_each(|t| *t = true);
            }
        }
    }
    let w = |k: usize| ws.get(k).map(|(_, w)| w.as_str()).unwrap_or("");
    // Patterns rather than lists: a dropped g ("cookin'"), stretched words ("sooo").
    for (k, (r, word)) in ws.iter().enumerate() {
        if taken[k] {
            continue;
        }
        let bare = word.trim_end_matches('\'');
        if bare.len() > 4
            && bare.ends_with("in")
            && !crate::grammar::known(bare)
            && crate::grammar::known(&format!("{bare}g"))
        {
            hits.push(Hit { range: r.clone(), feature: Feature::SpokenSpelling, standard: Some(format!("{bare}g")) });
            taken[k] = true;
            continue;
        }
        let chars: Vec<char> = bare.chars().collect();
        if chars.windows(3).any(|t| t[0] == t[1] && t[1] == t[2] && t[0].is_alphabetic()) {
            let mut squeezed = String::new();
            for c in &chars {
                if !squeezed.ends_with(*c) || !squeezed.ends_with(&format!("{c}{c}")) {
                    squeezed.push(*c);
                }
            }
            // "sooo" → "so", "lmaooo" → "lmao": keep at most one repeat, then try one.
            let one: String = {
                let mut o = String::new();
                for c in &chars {
                    if !o.ends_with(*c) {
                        o.push(*c);
                    }
                }
                o
            };
            let standard = if crate::grammar::known(&one) || lex.contains_key(&one) { one } else { squeezed };
            hits.push(Hit { range: r.clone(), feature: Feature::Texting, standard: Some(standard) });
            taken[k] = true;
        }
    }
    for i in 0..ws.len() {
        let (r, cur) = (&ws[i].0, ws[i].1.as_str());
        let prev = if i > 0 { w(i - 1) } else { "" };
        // African American English: habitual be ("he be working"), completive done ("I done told you"),
        // and a dropped is/are before an -ing verb ("she working").
        // After "will", "to", "can", "not"… it's ordinary English ("will be working").
        const NOT_SUBJECT: &[&str] = &[
            "will",
            "would",
            "can",
            "could",
            "should",
            "must",
            "may",
            "might",
            "shall",
            "to",
            "not",
            "won't",
            "don't",
            "doesn't",
            "didn't",
            "can't",
            "cannot",
            "never",
            "always",
            "just",
            "also",
            "still",
            "even",
            "let",
            "gonna",
            "wanna",
            "gotta",
            "i'll",
            "you'll",
            "we'll",
            "they'll",
            "he'll",
            "she'll",
            "it'll",
            "that'll",
            "or",
            "and",
            "but",
            "please",
            "sometimes",
            "really",
            "probably",
            "definitely",
            "maybe",
            "only",
            "then",
            "so",
            "ever",
            "often",
            "usually",
            "certainly",
            "actually",
            "rather",
            "better",
            "soon",
            "would've",
            "might've",
            "'ll",
            "finna",
            "tryna",
            "boutta",
            "may",
            "if",
            "whether",
            "that",
            "which",
            "who",
            "what",
            "there",
            "here",
            "it",
            "this",
        ];
        let noun_subject = i > 0
            && !NOT_SUBJECT.contains(&prev)
            && prev.chars().all(|c| c.is_alphabetic())
            && i >= 2
            && ["my", "his", "her", "our", "their", "your", "the", "that", "this", "mama", "momma"].contains(&w(i - 2));
        if cur == "be" && (SUBJECTS.contains(&prev) || noun_subject) && i + 1 < ws.len() {
            let next = w(i + 1);
            if next.ends_with("ing")
                || ["always", "so", "out", "at", "on", "like", "real", "tired", "busy", "late", "up", "talking"]
                    .contains(&next)
            {
                hits.push(Hit {
                    range: ws[i - 1].0.start..ws[i + 1].0.end,
                    feature: Feature::AaeGrammar,
                    standard: Some(
                        format!("{prev} is usually {next}")
                            .replace("i is", "I am")
                            .replace("you is", "you are")
                            .replace("we is", "we are")
                            .replace("they is", "they are"),
                    ),
                });
            }
        }
        if cur == "done" && SUBJECTS.contains(&prev) && i + 1 < ws.len() {
            let next = w(i + 1);
            if next.ends_with("ed")
                || [
                    "told", "said", "gone", "went", "ate", "seen", "got", "made", "had", "left", "forgot", "lost",
                    "broke", "came", "did", "bought", "sent", "took", "gave",
                ]
                .contains(&next)
            {
                hits.push(Hit {
                    range: ws[i - 1].0.start..ws[i + 1].0.end,
                    feature: Feature::AaeGrammar,
                    standard: Some(format!("{prev} already {next}").replace("i already", "I already")),
                });
            }
        }
        if ["he", "she", "we", "they", "you", "y'all"].contains(&prev)
            && cur.len() > 4
            && cur.ends_with("ing")
            && !cur.ends_with("thing")
            && ![
                "during",
                "something",
                "nothing",
                "anything",
                "everything",
                "bring",
                "thing",
                "king",
                "ring",
                "sing",
                "spring",
                "string",
                "swing",
                "wing",
                "being",
            ]
            .contains(&cur)
            && (i < 2
                || ![
                    "is", "are", "was", "were", "be", "been", "am", "'re", "keep", "kept", "start", "started", "stop",
                    "stopped", "see", "saw", "hear", "heard", "watch", "watched", "find", "found", "catch", "caught",
                    "have", "had", "get", "got", "let", "made", "make", "why", "how", "when", "while", "if", "of",
                    "about",
                ]
                .contains(&w(i - 2)))
        {
            hits.push(Hit {
                range: ws[i - 1].0.start..r.end,
                feature: Feature::AaeGrammar,
                standard: Some(format!("{} {} {cur}", prev, if ["he", "she"].contains(&prev) { "is" } else { "are" })),
            });
        }
        // ain't, and double negatives ("don't have no").
        if cur == "ain't" {
            hits.push(Hit {
                range: r.clone(),
                feature: Feature::NonstandardVerbs,
                standard: Some("isn't / aren't / haven't".into()),
            });
        }
        if NEGATIVES.contains(&cur) {
            for j in i + 1..(i + 4).min(ws.len()) {
                if ["say", "said", "says", "answer", "answered", "take", "took", "with", "for", "matter"]
                    .contains(&w(j - 1))
                    && j > i + 1
                {
                    break;
                }
                if NEGATIVE_OBJECTS.contains(&w(j)) {
                    hits.push(Hit {
                        range: r.start..ws[j].0.end,
                        feature: Feature::NonstandardVerbs,
                        standard: Some(format!("{} … any", ws[i].1)),
                    });
                    break;
                }
            }
        }
        // Contractions and first person (formal writing).
        if (CONTRACTION_ENDINGS.iter().any(|e| cur.ends_with(e)) || PRONOUN_IS.contains(&cur))
            && cur != "ain't"
            && !taken[i]
        {
            hits.push(Hit { range: r.clone(), feature: Feature::Contractions, standard: expand(cur) });
        }
        if FIRST_PERSON.contains(&cur) && !(cur == "us" && text[r.clone()].chars().all(char::is_uppercase)) {
            hits.push(Hit { range: r.clone(), feature: Feature::FirstPerson, standard: None });
        }
    }
    // Emoji, smileys, kisses and repeated exclamation marks.
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut k = 0;
    while k < chars.len() {
        let (i, c) = chars[k];
        let emoji = matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x1F000..=0x1F2FF | 0x2B50);
        let bangs = c == '!' && chars.get(k + 1).is_some_and(|(_, d)| *d == '!');
        if emoji || bangs {
            let mut e = k + 1;
            while e < chars.len()
                && (chars[e].1 == c
                    || matches!(chars[e].1 as u32, 0xFE0F | 0x200D | 0x1F3FB..=0x1F3FF)
                    || emoji && matches!(chars[e].1 as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF))
            {
                e += 1;
            }
            let end = chars.get(e).map(|(j, _)| *j).unwrap_or(text.len());
            hits.push(Hit {
                range: i..end,
                feature: Feature::EmojiExclaim,
                standard: if bangs { Some("!".into()) } else { None },
            });
            k = e;
            continue;
        }
        k += 1;
    }
    for (r, w) in &ws {
        if matches!(w.as_str(), "xx" | "xxx" | "xoxo") {
            hits.push(Hit { range: r.clone(), feature: Feature::EmojiExclaim, standard: None });
        }
    }
    for smiley in [":)", ":-)", ":(", ":-(", ":D", ";)", ";-)", ":P", ":p", "<3", ":/"] {
        for (i, _) in text.match_indices(smiley) {
            hits.push(Hit { range: i..i + smiley.len(), feature: Feature::EmojiExclaim, standard: None });
        }
    }
    hits.sort_by_key(|h| (h.range.start, std::cmp::Reverse(h.range.end)));
    hits
}

/// "don't" → "do not".
fn expand(c: &str) -> Option<String> {
    let full = match c {
        "won't" => "will not".into(),
        "can't" => "cannot".into(),
        "shan't" => "shall not".into(),
        "let's" => "let us".into(),
        _ => {
            if let Some(b) = c.strip_suffix("n't") {
                format!("{b} not")
            } else if let Some(b) = c.strip_suffix("'re") {
                format!("{b} are")
            } else if let Some(b) = c.strip_suffix("'ve") {
                format!("{b} have")
            } else if let Some(b) = c.strip_suffix("'ll") {
                format!("{b} will")
            } else if let Some(b) = c.strip_suffix("'m") {
                format!("{b} am")
            } else if let Some(b) = c.strip_suffix("'d") {
                format!("{b} would")
            } else {
                let b = c.strip_suffix("'s")?;
                format!("{b} is")
            }
        }
    };
    Some(
        full.replace("i am", "I am")
            .replace("i have", "I have")
            .replace("i will", "I will")
            .replace("i would", "I would"),
    )
}

/// Whether `pos` is inside quotation marks in `text`.
fn in_quotes(text: &str, pos: usize) -> bool {
    let before = &text[..pos];
    let straight = before.matches('"').count() % 2 == 1;
    let curly = before.matches('\u{201c}').count() > before.matches('\u{201d}').count();
    straight || curly
}

fn message(f: Feature, found: &str, standard: Option<&str>, strong: bool, profile: &Profile) -> String {
    let fname = if f == Feature::AaeGrammar { f.name().to_string() } else { f.name().to_lowercase() };
    let lead =
        if strong { format!("Not usually used in {}", profile.name.to_lowercase()) } else { "Informal".to_string() };
    // A masked word ("****ed") says nothing.
    let standard = standard.filter(|s| !s.contains("**"));
    match (f, standard) {
        (Feature::Contractions, Some(s)) => format!("{lead} ({}): `{found}`. Written out: `{s}`.", fname),
        (Feature::FirstPerson, _) => format!("{lead} ({}): `{found}`. Some formal writing avoids I/we.", fname),
        (_, Some(s)) => format!("{lead} ({}): `{found}`. In standard English: `{s}`.", fname),
        (_, None) => format!("{lead} ({}): `{found}`.", fname),
    }
}

/// Applies `profile` to the issues found in `text`: findings that are really
/// register features are allowed (dropped), noted or flagged; features found
/// directly are added as tone & register issues.
pub(crate) fn apply(text: &str, profile: &Profile, out: &mut Vec<Local>) {
    let hits = find(text);
    let lex = lexicon();
    let quoted = |r: &Range<usize>| profile.ignore_in_quotes && in_quotes(text, r.start);
    // 1. Existing issues that are register features.
    let mut kept = Vec::with_capacity(out.len());
    for mut l in out.drain(..) {
        let word = text.get(l.range.clone()).unwrap_or("");
        // "wot does…": the informal spelling (→ What) says more than the missing capital.
        if l.rule == "SentenceCapitalization"
            && lex.get(&norm(word)).is_some_and(|(f, s)| s.is_some() && profile.level(*f) != Level::Allow)
        {
            continue;
        }
        let by_rule = RULE_FEATURES.iter().find(|(r, _)| *r == l.rule).map(|(_, f)| (*f, None));
        let by_list = lex.get(&norm(word)).cloned().or_else(|| {
            hits.iter()
                .find(|h| {
                    h.range.start <= l.range.start
                        && l.range.end <= h.range.end
                        && !matches!(h.feature, Feature::Contractions | Feature::FirstPerson)
                })
                .map(|h| (h.feature, h.standard.clone()))
        });
        // All-lowercase writing ("i'm", "american", "june") is texting style.
        // (A word on our informal lists is that, even if it's also a name: "arvo" / Arvo.)
        let lowercase_style = !lex.contains_key(&norm(word))
            && (l.category == Category::Spelling || l.rule == "OrthographicConsistency")
            && word.chars().next().is_some_and(char::is_lowercase)
            && {
                let mut c = word.chars();
                let cap: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
                crate::grammar::known(&cap)
            };
        let by_rule = by_rule.or_else(|| lowercase_style.then_some((Feature::Texting, None)));
        let from_rule = by_rule.is_some();
        // Xenify's own typo and grammar rules are never re-labelled ("night shit" is a typo for shift).
        if crate::own_rules().contains(&l.rule.as_str()) && !RULE_FEATURES.iter().any(|(r, _)| *r == l.rule) {
            kept.push(l);
            continue;
        }
        let from_list = by_list.is_some();
        // A rule names the feature; our list may still know the standard word ("pissed" → angry).
        let list_std = by_list.as_ref().and_then(|(_, s)| s.clone());
        let Some((f, std)) = by_rule.map(|(f, s)| (f, s.or(list_std))).or(by_list) else {
            kept.push(l);
            continue;
        };
        match profile.level(f) {
            Level::Allow => {}
            _ if quoted(&l.range) => {}
            // A grammar rule's finding, flagged: keep it as the grammar issue it is.
            Level::Flag if from_rule => kept.push(l),
            level => {
                let strong = level == Level::Flag;
                l.message =
                    message(f, word, std.as_deref().or(l.suggestions.first().map(String::as_str)), strong, profile);
                if let Some(s) = std.filter(|s| !s.contains('/')) {
                    l.suggestions = vec![s];
                } else if from_list || l.category == Category::Spelling {
                    l.suggestions.clear();
                }
                l.category = Category::Register;
                l.feature = Some(f);
                l.strong = strong;
                kept.push(l);
            }
        }
    }
    // 2. Features found directly, where nothing else marks the same words.
    for h in hits {
        let level = profile.level(h.feature);
        if level == Level::Allow || quoted(&h.range) {
            continue;
        }
        if kept.iter().any(|l| l.range.start < h.range.end && h.range.start < l.range.end) {
            continue;
        }
        let found = &text[h.range.clone()];
        let strong = level == Level::Flag;
        let mut h = h;
        if sentence_start(text, h.range.start) {
            h.standard = h.standard.map(|s| {
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            });
        }
        kept.push(Local {
            range: h.range.clone(),
            category: Category::Register,
            rule: format!("Register:{:?}", h.feature),
            message: message(h.feature, found, h.standard.as_deref(), strong, profile),
            suggestions: h.standard.iter().filter(|s| !s.contains('/') && !s.contains('…')).cloned().collect(),
            feature: Some(h.feature),
            strong,
        });
    }
    // One issue per stretch of text: several rules can mark the same informal word.
    let mut seen: Vec<Range<usize>> = Vec::new();
    kept.retain(|l| {
        if l.category != Category::Register {
            return true;
        }
        if seen.contains(&l.range) {
            return false;
        }
        seen.push(l.range.clone());
        true
    });
    *out = kept;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(String, Feature)> {
        find(text).into_iter().map(|h| (text[h.range].to_string(), h.feature)).collect()
    }

    #[test]
    fn finds_features() {
        let f = found(
            "idk tbh, he be working late and she working too. I done told you we gonna win!! 😂 y'all ain't got no chance",
        );
        let has = |w: &str, ft: Feature| f.iter().any(|(x, y)| x == w && *y == ft);
        assert!(has("idk", Feature::Texting), "{f:?}");
        assert!(has("tbh", Feature::Texting));
        assert!(has("he be working", Feature::AaeGrammar), "{f:?}");
        let g = found("My cousin be working doubles. It will be working soon.");
        assert_eq!(g.iter().filter(|(_, f)| *f == Feature::AaeGrammar).count(), 1, "{g:?}");
        assert!(has("she working", Feature::AaeGrammar), "{f:?}");
        assert!(has("I done told", Feature::AaeGrammar), "{f:?}");
        assert!(has("gonna", Feature::SpokenSpelling));
        let g = found("Wot does it feel like? I luv it, but wen is it? We saw Wat Pho.");
        for w in ["Wot", "luv", "wen"] {
            assert!(g.iter().any(|(x, f)| x == w && *f == Feature::SpokenSpelling), "{w}: {g:?}");
        }
        assert!(!g.iter().any(|(x, _)| x == "Wat"), "names mid-sentence are skipped: {g:?}");
        assert!(has("!!", Feature::EmojiExclaim));
        assert!(has("😂", Feature::EmojiExclaim), "{f:?}");
        assert!(has("y'all", Feature::RegionalDialect));
        assert!(has("ain't", Feature::NonstandardVerbs));
        assert!(has("ain't got no", Feature::NonstandardVerbs), "{f:?}");
    }

    #[test]
    fn leaves_standard_english_alone() {
        for text in [
            "She is working late tonight, and they are going home.",
            "Something is wrong; nothing works during the day.",
            "I will be working tomorrow.",
            "He said no to the offer.",
            "The US economy grew.",
            "We saw them running.",
        ] {
            let f: Vec<_> = found(text)
                .into_iter()
                .filter(|(_, f)| !matches!(f, Feature::FirstPerson | Feature::Contractions))
                .collect();
            assert!(f.is_empty(), "{text}: {f:?}");
        }
    }

    #[test]
    fn profile_levels() {
        let text = "idk, it's fine tbh.";
        let run = |p: &Profile| {
            let mut out = Vec::new();
            apply(text, p, &mut out);
            out
        };
        let casual = crate::built_in_profiles().into_iter().find(|p| p.id == "casual").unwrap();
        assert!(run(&casual).is_empty());
        let academic = crate::built_in_profiles().into_iter().find(|p| p.id == "academic").unwrap();
        let a = run(&academic);
        assert!(
            a.iter().any(|l| &text[l.range.clone()] == "idk" && l.strong && l.suggestions[0] == "I don't know"),
            "{:?}",
            a.iter().map(|l| (&text[l.range.clone()], &l.message)).collect::<Vec<_>>()
        );
        assert!(a.iter().any(|l| &text[l.range.clone()] == "it's" && !l.strong && l.suggestions[0] == "it is"));
    }

    #[test]
    fn dialogue_is_left_alone_in_creative_writing() {
        let creative = crate::built_in_profiles().into_iter().find(|p| p.id == "creative").unwrap();
        let mut out = Vec::new();
        apply("\"We gonna be late,\" she said. The gonna was outside.", &creative, &mut out);
        assert_eq!(out.len(), 1, "only the narration is noted");
    }
}
